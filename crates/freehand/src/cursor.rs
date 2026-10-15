//! Big-endian cursor over an untrusted byte slice. Every read fails at the end of the slice
//! instead of panicking. The failure sentence depends on which part of the file is being read.

use crate::Error;

/// Which structural region the cursor is walking, so a short read names that region.
#[derive(Clone, Copy)]
pub(crate) enum Phase {
    Header,
    Dict,
    List,
    Record,
}

pub(crate) struct Cur<'a> {
    data: &'a [u8],
    pub pos: usize,
    phase: Phase,
    /// Escaped record ids above this (1-based count) are refused. `usize::MAX` skips the check.
    max_id: usize,
}

impl<'a> Cur<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self::phased(data, Phase::Record)
    }

    pub(crate) fn phased(data: &'a [u8], phase: Phase) -> Self {
        Self { data, pos: 0, phase, max_id: usize::MAX }
    }

    pub(crate) fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }

    pub(crate) fn set_max_id(&mut self, max_id: usize) {
        self.max_id = max_id;
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// The user-facing sentence for a read that ran off the end of this region.
    pub(crate) fn fail(&self) -> Error {
        match self.need(self.remaining().saturating_add(1)) {
            Err(e) => e,
            Ok(()) => Error::new("the FreeHand file is cut off in a record"),
        }
    }

    fn need(&self, n: usize) -> Result<(), Error> {
        if self.remaining() >= n {
            return Ok(());
        }
        match self.phase {
            Phase::Header => Err("the FreeHand file is cut off in the header".into()),
            Phase::Dict => Err("the FreeHand file is cut off in the dictionary".into()),
            Phase::List => Err("the FreeHand file is cut off in the record list".into()),
            Phase::Record => Err("the FreeHand file is cut off in a record".into()),
        }
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Error> {
        self.need(1)?;
        let b = *self.data.get(self.pos).ok_or_else(|| self.fail())?;
        self.pos += 1;
        Ok(b)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, Error> {
        let b = self.bytes(2)?;
        let arr: [u8; 2] = b.try_into().map_err(|_| self.fail())?;
        Ok(u16::from_be_bytes(arr))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, Error> {
        let b = self.bytes(4)?;
        let arr: [u8; 4] = b.try_into().map_err(|_| self.fail())?;
        Ok(u32::from_be_bytes(arr))
    }

    /// Signed 16.16 fixed point, in points (or a plain ratio, for a matrix scale).
    pub(crate) fn fixed(&mut self) -> Result<f64, Error> {
        Ok(f64::from(self.u32()? as i32) / 65536.0)
    }

    /// A record id: 0 is none, `0xFFFF` then a u16 is `0x1FF00` minus that u16.
    /// An escaped id past the record count is refused. A plain u16 is not: block slots can
    /// hold numbers that are not record ids.
    pub(crate) fn id(&mut self) -> Result<u32, Error> {
        let v = self.u16()?;
        if v != 0xFFFF {
            return Ok(u32::from(v));
        }
        let n = u32::from(self.u16()?);
        let id = 0x1_FF00u32.wrapping_sub(n);
        if self.max_id != usize::MAX && id != 0 && usize::try_from(id).unwrap_or(usize::MAX) > self.max_id {
            return Err("a FreeHand record id points past the end of the file".into());
        }
        Ok(id)
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<(), Error> {
        self.need(n)?;
        self.pos += n;
        Ok(())
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        self.need(n)?;
        let start = self.pos;
        self.pos += n;
        self.data.get(start..self.pos).ok_or_else(|| self.fail())
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }

    pub(crate) fn peek_at(&self, abs: usize) -> Result<u8, Error> {
        self.data.get(abs).copied().ok_or_else(|| self.fail())
    }

    /// NUL-terminated bytes, the NUL consumed, capped so a missing NUL cannot run away.
    pub(crate) fn cstr(&mut self, cap: usize) -> Result<&'a [u8], Error> {
        let start = self.pos;
        let end = (start + cap).min(self.data.len());
        let window = self.data.get(start..end).ok_or_else(|| self.fail())?;
        let Some(rel) = window.iter().position(|b| *b == 0) else {
            return Err(self.fail());
        };
        self.pos = start + rel + 1;
        self.data.get(start..start + rel).ok_or_else(|| self.fail())
    }
}

/// `count * size` bytes, rejected when the product overflows or passes `limit`.
pub(crate) fn mul_size(count: u32, size: usize, limit: usize) -> Result<usize, Error> {
    let n = usize::try_from(count).map_err(|_| crate::entries())?;
    let bytes = n.checked_mul(size).ok_or_else(crate::entries)?;
    if bytes > limit { Err(crate::entries()) } else { Ok(bytes) }
}
