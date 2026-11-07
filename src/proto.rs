//! Just enough protobuf for Steam's services: varints, fixed64, length-delimited
//! strings and bytes, and nested messages. No schema, no codegen.

#[derive(Default)]
pub struct Enc(Vec<u8>);

fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

impl Enc {
    pub fn new() -> Self {
        Self::default()
    }
    fn key(&mut self, field: u32, wire: u8) {
        varint(&mut self.0, ((field as u64) << 3) | wire as u64);
    }
    pub fn uint(mut self, field: u32, v: u64) -> Self {
        self.key(field, 0);
        varint(&mut self.0, v);
        self
    }
    /// int32 or enum. Negative values take the full 10-byte varint, as protobuf requires.
    pub fn int(self, field: u32, v: i64) -> Self {
        self.uint(field, v as u64)
    }
    pub fn bool(self, field: u32, v: bool) -> Self {
        self.uint(field, v as u64)
    }
    pub fn fixed64(mut self, field: u32, v: u64) -> Self {
        self.key(field, 1);
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn bytes(mut self, field: u32, v: &[u8]) -> Self {
        self.key(field, 2);
        varint(&mut self.0, v.len() as u64);
        self.0.extend_from_slice(v);
        self
    }
    pub fn str(self, field: u32, v: &str) -> Self {
        self.bytes(field, v.as_bytes())
    }
    pub fn msg(self, field: u32, m: Enc) -> Self {
        self.bytes(field, &m.0)
    }
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[derive(Clone, Debug)]
pub enum Val {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    Bytes(Vec<u8>),
}

/// A parsed message: (field number, value) pairs in the order they appeared.
#[derive(Clone, Debug, Default)]
pub struct Msg(pub Vec<(u32, Val)>);

impl Msg {
    pub fn parse(mut b: &[u8]) -> Result<Msg, String> {
        fn read_varint(b: &mut &[u8]) -> Result<u64, String> {
            let mut v = 0u64;
            for shift in (0..64).step_by(7) {
                let (&byte, rest) = b.split_first().ok_or("protobuf: обрыв varint")?;
                *b = rest;
                v |= ((byte & 0x7f) as u64) << shift;
                if byte & 0x80 == 0 {
                    return Ok(v);
                }
            }
            Err("protobuf: varint is too long".into())
        }
        let mut out = Vec::new();
        while !b.is_empty() {
            let key = read_varint(&mut b)?;
            let field = (key >> 3) as u32;
            let val = match key & 7 {
                0 => Val::Varint(read_varint(&mut b)?),
                1 => {
                    let (x, rest) = b.split_at_checked(8).ok_or("protobuf: truncated fixed64")?;
                    b = rest;
                    Val::Fixed64(u64::from_le_bytes(x.try_into().unwrap()))
                }
                2 => {
                    let len = read_varint(&mut b)? as usize;
                    let (x, rest) = b.split_at_checked(len).ok_or("protobuf: truncated length-delimited field")?;
                    b = rest;
                    Val::Bytes(x.to_vec())
                }
                5 => {
                    let (x, rest) = b.split_at_checked(4).ok_or("protobuf: truncated fixed32")?;
                    b = rest;
                    Val::Fixed32(u32::from_le_bytes(x.try_into().unwrap()))
                }
                w => return Err(format!("protobuf: unknown wire type {w}")),
            };
            out.push((field, val));
        }
        Ok(Msg(out))
    }

    fn get(&self, field: u32) -> Option<&Val> {
        self.0.iter().find(|(f, _)| *f == field).map(|(_, v)| v)
    }
    pub fn u64(&self, field: u32) -> u64 {
        match self.get(field) {
            Some(Val::Varint(v) | Val::Fixed64(v)) => *v,
            Some(Val::Fixed32(v)) => *v as u64,
            _ => 0,
        }
    }
    pub fn i64(&self, field: u32) -> i64 {
        self.u64(field) as i64
    }
    pub fn bool(&self, field: u32) -> bool {
        self.u64(field) != 0
    }
    pub fn f32(&self, field: u32) -> f32 {
        match self.get(field) {
            Some(Val::Fixed32(v)) => f32::from_bits(*v),
            _ => 0.0,
        }
    }
    pub fn bytes(&self, field: u32) -> Vec<u8> {
        match self.get(field) {
            Some(Val::Bytes(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    pub fn str(&self, field: u32) -> String {
        String::from_utf8_lossy(&self.bytes(field)).into_owned()
    }
    /// Every nested message carrying this field number, i.e. a repeated field.
    pub fn msgs(&self, field: u32) -> Vec<Msg> {
        self.0
            .iter()
            .filter(|(f, _)| *f == field)
            .filter_map(|(_, v)| match v {
                Val::Bytes(b) => Msg::parse(b).ok(),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let inner = Enc::new().str(1, "hi").int(3, -500);
        let bytes = Enc::new().uint(1, 300).fixed64(2, 76561198000000001).str(3, "hello").bool(5, true).msg(9, inner).finish();
        let m = Msg::parse(&bytes).unwrap();
        assert_eq!(m.u64(1), 300);
        assert_eq!(m.u64(2), 76561198000000001);
        assert_eq!(m.str(3), "hello");
        assert!(m.bool(5));
        let sub = &m.msgs(9)[0];
        assert_eq!(sub.str(1), "hi");
        assert_eq!(sub.i64(3) as i32, -500);
    }

    #[test]
    fn varint_300_is_ac02() {
        // The example from the protobuf docs: 300 encodes as 0xAC 0x02.
        assert_eq!(Enc::new().uint(1, 300).finish(), vec![0x08, 0xAC, 0x02]);
    }
}
