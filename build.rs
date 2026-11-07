//! Embeds the application icon into the .exe. Rust has no "pick an image in the IDE":
//! an executable's icon is a Win32 resource, compiled by rc.exe and linked in.
//!
//! The .ico is not checked in as artwork. It is drawn by the same code as the tray
//! icon: `src/icon.rs` is pulled in with `include!` so the two cannot drift apart.

include!("src/icon.rs");

fn main() {
    // Listing rerun-if-changed explicitly turns off cargo's default "watch the whole
    // package" behaviour. Without that, writing assets/sda.ico below would make cargo
    // re-run this script on every build.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // The sizes Windows actually asks for: file lists, the desktop, Alt+Tab, the
    // taskbar, and Explorer's extra-large tiles.
    let ico = ico_bytes(&[16, 24, 32, 48, 64, 128, 256]);
    let path = std::path::Path::new("assets/sda.ico");
    // Written into the working tree so the file is available for shortcuts and
    // installers. Only rewritten when it actually changed, to keep mtimes stable.
    if std::fs::read(path).map(|old| old != ico).unwrap_or(true) {
        std::fs::write(path, &ico).expect("could not write assets/sda.ico");
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/sda.ico");
    if let Err(e) = res.compile() {
        // Without the Windows SDK there is no rc.exe. An icon is not worth failing over.
        println!("cargo:warning=could not build the .exe icon: {e}");
    }
}
