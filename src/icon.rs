// The SDA shield, drawn in code: one geometry for the tray icon, the window icon
// and the .exe icon.
//
// This module deliberately sticks to bare std and knows nothing about the rest of the
// project, because build.rs pulls it in with `include!` to bake the .ico from the same
// formulas the app draws with. Keeping one source stops the file and the code drifting.

/// The shield outline in a 16x19 box around the origin, same as `title_bar` draws.
const SHIELD: [(f32, f32); 5] = [(-8.0, -9.0), (8.0, -9.0), (8.0, 1.0), (0.0, 10.0), (-8.0, 1.0)];

/// The outline uses the app's accent colour.
const STROKE: (f32, f32, f32) = (118.0, 158.0, 196.0);
/// The fill is dark and opaque so the icon reads both against Explorer's light
/// background and against a dark taskbar.
const FILL_TOP: (f32, f32, f32) = (32.0, 41.0, 54.0);
const FILL_BOTTOM: (f32, f32, f32) = (16.0, 19.0, 25.0);

/// An `n` by `n` shield as straight (not premultiplied) RGBA8, top row first.
pub fn shield_rgba(n: u32) -> Vec<u8> {
    const SS: i32 = 4; // subsamples per pixel along each axis
    let nf = n as f32;
    let margin = nf * 0.06;
    // The shield is taller than it is wide (19 against 16), so height drives the fit.
    let scale = (nf - 2.0 * margin) / 19.0;
    let (cx, cy) = (nf / 2.0, nf / 2.0 - 0.5 * scale);
    let stroke_px = (nf / 18.0).max(1.0);

    let to_px = |(x, y): (f32, f32), shrink: f32| {
        // Shrinking towards the shield's centre yields the inner outline.
        let (dx, dy) = (x, y - 0.5);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let k = (len - shrink) / len;
        (cx + dx * k * scale, cy + 0.5 * scale + dy * k * scale)
    };
    let outer: Vec<(f32, f32)> = SHIELD.iter().map(|&p| to_px(p, 0.0)).collect();
    let inner: Vec<(f32, f32)> = SHIELD.iter().map(|&p| to_px(p, stroke_px / scale)).collect();
    let (top, bottom) = (cy - 9.5 * scale, cy + 10.5 * scale);

    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (mut out_hits, mut in_hits) = (0, 0);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if inside(&outer, px, py) {
                        out_hits += 1;
                        if inside(&inner, px, py) {
                            in_hits += 1;
                        }
                    }
                }
            }
            let total = (SS * SS) as f32;
            let cover = out_hits as f32 / total;
            if cover <= 0.0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let fill = in_hits as f32 / total;
            let edge = cover - fill;
            // A gentle vertical gradient keeps the fill from reading as a flat blob.
            let t = ((y as f32 + 0.5 - top) / (bottom - top)).clamp(0.0, 1.0);
            let body = (
                FILL_TOP.0 + (FILL_BOTTOM.0 - FILL_TOP.0) * t,
                FILL_TOP.1 + (FILL_BOTTOM.1 - FILL_TOP.1) * t,
                FILL_TOP.2 + (FILL_BOTTOM.2 - FILL_TOP.2) * t,
            );
            // Straight colour: outline and fill contributions divided by total coverage.
            let mix = |s: f32, b: f32| ((s * edge + b * fill) / cover).round().clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[
                mix(STROKE.0, body.0),
                mix(STROKE.1, body.1),
                mix(STROKE.2, body.2),
                (cover * 255.0).round().clamp(0.0, 255.0) as u8,
            ]);
        }
    }
    rgba
}

/// Even-odd point-in-polygon test.
fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut hit = false;
    for i in 0..poly.len() {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % poly.len()];
        if (y1 > y) != (y2 > y) && x < (x2 - x1) * (y - y1) / (y2 - y1) + x1 {
            hit = !hit;
        }
    }
    hit
}

/// A multi-size .ico. Frames use the BMP variant (BITMAPINFOHEADER, bottom-up BGRA,
/// then a mask) because it needs no compression, which keeps this module dependency-free.
#[allow(dead_code)] // build.rs only; never reached from the binary
pub fn ico_bytes(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = sizes.iter().map(|&n| (n, bmp_image(n))).collect();

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    // Pixel data starts right after the directory of 16-byte entries.
    let mut offset = 6 + 16 * images.len() as u32;
    for (n, data) in &images {
        // 256 is encoded as zero, since the field is a single byte.
        let dim = if *n >= 256 { 0u8 } else { *n as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

#[allow(dead_code)]
fn bmp_image(n: u32) -> Vec<u8> {
    let rgba = shield_rgba(n);
    // Transparency mask: one bit per pixel, rows padded to four bytes.
    let mask_stride = n.div_ceil(32) * 4;
    let mut out = Vec::new();

    out.extend_from_slice(&40u32.to_le_bytes()); // sizeof BITMAPINFOHEADER
    out.extend_from_slice(&(n as i32).to_le_bytes());
    // Doubled height: the header covers the image and its mask as one bitmap.
    out.extend_from_slice(&(2 * n as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB, uncompressed
    out.extend_from_slice(&(n * n * 4 + n * mask_stride).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // resolution and palette fields, all zero

    // BGRA, bottom row first.
    for y in (0..n).rev() {
        for x in 0..n {
            let i = ((y * n + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    // Modern Windows reads transparency from the alpha channel, but the format still
    // requires the mask to be present.
    for _ in 0..n {
        out.extend(std::iter::repeat_n(0u8, mask_stride as usize));
    }
    out
}
