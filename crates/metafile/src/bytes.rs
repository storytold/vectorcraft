//! Little-endian values for hand-built import fixtures.

/// Bytes being written.
#[derive(Default)]
pub(crate) struct Out(pub Vec<u8>);

impl Out {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    /// Zeros up to the next multiple of `n` bytes.
    pub fn pad(&mut self, n: usize) {
        while !self.0.len().is_multiple_of(n) {
            self.0.push(0);
        }
    }
}
