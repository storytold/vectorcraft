//! Bounds-checked little-endian reads. Coordinates remain inches until assembly.

pub(crate) struct Reader<'a> {
    pub bytes: &'a [u8],
    pub pos: usize,
}
impl<'a> Reader<'a> {
    /// Start a cursor at the beginning of a borrowed record.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    /// Consume exactly `n` bytes, rejecting truncation or offset overflow.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("CDR offset overflow")?;
        let out = self.bytes.get(self.pos..end).ok_or("truncated CDR record")?;
        self.pos = end;
        Ok(out)
    }
    /// Advance by `n` bytes using the same bounds checks as a read.
    pub fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
    /// Create an independent cursor at an absolute offset within this record.
    pub fn at(&self, pos: usize) -> Result<Self, String> {
        if pos > self.bytes.len() {
            return Err("CDR offset outside record".into());
        }
        Ok(Self { bytes: self.bytes, pos })
    }
    /// Consume one unsigned byte.
    pub fn u8(&mut self) -> Result<u8, String> {
        self.take(1)?.first().copied().ok_or_else(|| "truncated CDR byte".into())
    }
    /// Consume an unsigned little-endian 16-bit integer.
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| "truncated CDR integer")?))
    }
    /// Consume an unsigned little-endian 32-bit integer.
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| "truncated CDR integer")?))
    }
    /// Consume a little-endian double, rejecting infinities and NaNs.
    pub fn f64(&mut self) -> Result<f64, String> {
        let v = f64::from_le_bytes(self.take(8)?.try_into().map_err(|_| "truncated CDR number")?);
        if !v.is_finite() {
            return Err("non-finite CDR number".into());
        }
        Ok(v)
    }
    /// Decode a signed CDR coordinate into inches (254,000 units per inch).
    pub fn coord(&mut self) -> Result<f64, String> {
        Ok(self.u32()? as i32 as f64 / 254000.0)
    }
}
