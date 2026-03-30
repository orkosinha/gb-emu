//! Binary codec for emulator save-states.
//!
//! ## Format (version 1)
//!
//! ```text
//! "GBSNAP1"  magic (7 bytes)
//! 0x01       version (u8)
//! <CPU>      23 bytes
//! <Timer>     6 bytes
//! <PPU>       9 bytes  (frame buffer regenerated from VRAM on next tick)
//! <APU>     ~104 bytes (audio ring buffer preserved across restore)
//! <Joypad>   10 bytes
//! <Memory>  ~49.7 KB fixed + mbc_len(u16) + mbc_data + cart_ram_len(u32) + cart_ram
//! <Counters> 20 bytes
//! ```
//!
//! Typical sizes: ~50 KB (DMG, no cart RAM) · ~82 KB (DMG + 32 KB LSDJ cart RAM).

pub(crate) trait Snapshot {
    fn snapshot(&self, w: &mut SnapWriter);
    fn restore_from(&mut self, r: &mut SnapReader) -> Result<(), &'static str>;
}

pub(crate) struct SnapWriter(Vec<u8>);

impl SnapWriter {
    pub fn new() -> Self { SnapWriter(Vec::new()) }

    #[inline] pub fn u8(&mut self, v: u8)     { self.0.push(v); }
    #[inline] pub fn u16(&mut self, v: u16)   { self.0.extend_from_slice(&v.to_le_bytes()); }
    #[inline] pub fn u32(&mut self, v: u32)   { self.0.extend_from_slice(&v.to_le_bytes()); }
    #[inline] pub fn u64(&mut self, v: u64)   { self.0.extend_from_slice(&v.to_le_bytes()); }
    #[inline] pub fn f32(&mut self, v: f32)   { self.u32(v.to_bits()); }
    #[inline] pub fn f64(&mut self, v: f64)   { self.u64(v.to_bits()); }
    #[inline] pub fn bool(&mut self, v: bool) { self.u8(v as u8); }
    #[inline] pub fn bytes(&mut self, v: &[u8]) { self.0.extend_from_slice(v); }

    pub fn into_vec(self) -> Vec<u8> { self.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_primitives() {
        let mut w = SnapWriter::new();
        w.u8(0xFF);
        w.u16(0x1234);
        w.u32(0xDEAD_BEEF);
        w.u64(u64::MAX);
        w.f32(1.5_f32);
        w.f64(std::f64::consts::PI);
        w.bool(true);
        w.bool(false);

        let data = w.into_vec();
        let mut r = SnapReader::new(&data);

        assert_eq!(r.u8().unwrap(), 0xFF);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.u64().unwrap(), u64::MAX);
        assert_eq!(r.f32().unwrap(), 1.5_f32);
        assert!((r.f64().unwrap() - std::f64::consts::PI).abs() < 1e-15);
        assert!(r.bool().unwrap());
        assert!(!r.bool().unwrap());
    }

    #[test]
    fn roundtrip_bytes() {
        let src = [1u8, 2, 3, 255, 0];
        let mut w = SnapWriter::new();
        w.bytes(&src);
        let data = w.into_vec();
        let mut r = SnapReader::new(&data);
        assert_eq!(r.bytes(src.len()).unwrap(), &src);
    }

    #[test]
    fn reader_u8_truncated() {
        let data = [];
        let mut r = SnapReader::new(&data);
        assert!(r.u8().is_err());
    }

    #[test]
    fn reader_u16_truncated() {
        let data = [0u8]; // only 1 byte, need 2
        let mut r = SnapReader::new(&data);
        assert!(r.u16().is_err());
    }

    #[test]
    fn reader_bytes_truncated() {
        let data = [0u8; 3];
        let mut r = SnapReader::new(&data);
        assert!(r.bytes(4).is_err());
    }

    #[test]
    fn reader_bytes_exact() {
        let data = [1u8, 2, 3];
        let mut r = SnapReader::new(&data);
        assert_eq!(r.bytes(3).unwrap(), &[1, 2, 3]);
        assert!(r.u8().is_err()); // exhausted
    }

    #[test]
    fn little_endian_u16() {
        let mut w = SnapWriter::new();
        w.u16(0x0102);
        assert_eq!(w.into_vec(), [0x02, 0x01]);
    }

    #[test]
    fn little_endian_u32() {
        let mut w = SnapWriter::new();
        w.u32(0x01020304);
        assert_eq!(w.into_vec(), [0x04, 0x03, 0x02, 0x01]);
    }
}

pub(crate) struct SnapReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SnapReader<'a> {
    pub fn new(data: &'a [u8]) -> Self { SnapReader { data, pos: 0 } }

    #[inline]
    pub fn u8(&mut self) -> Result<u8, &'static str> {
        if self.pos >= self.data.len() { return Err("snapshot truncated"); }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    #[inline]
    pub fn u16(&mut self) -> Result<u16, &'static str> {
        Ok(self.u8()? as u16 | ((self.u8()? as u16) << 8))
    }

    #[inline]
    pub fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(self.u8()? as u32
            | ((self.u8()? as u32) << 8)
            | ((self.u8()? as u32) << 16)
            | ((self.u8()? as u32) << 24))
    }

    #[inline]
    pub fn u64(&mut self) -> Result<u64, &'static str> {
        Ok(self.u32()? as u64 | ((self.u32()? as u64) << 32))
    }

    #[inline] pub fn f32(&mut self) -> Result<f32, &'static str>  { Ok(f32::from_bits(self.u32()?)) }
    #[inline] pub fn f64(&mut self) -> Result<f64, &'static str>  { Ok(f64::from_bits(self.u64()?)) }
    #[inline] pub fn bool(&mut self) -> Result<bool, &'static str> { Ok(self.u8()? != 0) }

    pub fn bytes(&mut self, n: usize) -> Result<&[u8], &'static str> {
        let end = self.pos + n;
        if end > self.data.len() { return Err("snapshot truncated"); }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
}
