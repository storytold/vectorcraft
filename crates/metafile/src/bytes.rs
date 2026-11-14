//! Little-endian values: the buffer both writers build records in.

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
    pub fn f32(&mut self, v: f32) {
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
    /// Overwrite the `u32` at `at` (a size or count known only later).
    pub fn set_u32(&mut self, at: usize, v: u32) {
        if let Some(b) = self.0.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
    pub fn set_i32(&mut self, at: usize, v: i32) {
        if let Some(b) = self.0.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
}
