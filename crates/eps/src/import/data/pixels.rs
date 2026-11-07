use super::ops::Source;
use super::{Affine, Obj, Res, Space, ps_err};

/// An image to decode.
pub(super) struct Spec {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) bpc: u8,
    pub(super) space: Space,
    pub(super) decode: Vec<f64>,
    /// User space → image space.
    pub(super) matrix: Affine,
    /// One source, or one per component.
    pub(super) sources: Vec<Source>,
    /// `MaskColor` (ImageType 4): samples that are transparent.
    pub(super) mask_color: Option<Vec<u32>>,
    /// An image mask: whether its 1 bits paint.
    pub(super) stencil: Option<bool>,
}

impl Spec {
    /// Bytes a row of `n` components takes.
    pub(super) fn row_bytes(&self, n: usize) -> usize {
        (self.w * n * usize::from(self.bpc)).div_ceil(8)
    }
}

/// Data of an ImageType 3 interleaved by row, split into the mask's and the image's: the one
/// with fewer rows gives one row before each run of the other's (`(row bytes, rows)` each).
pub(super) fn by_rows(raw: &[u8], (mrow, mh): (usize, usize), (drow, dh): (usize, usize)) -> (Vec<u8>, Vec<u8>) {
    let room = |row: usize, rows: usize| row.saturating_mul(rows).min(raw.len());
    let (mut mask, mut data) = (Vec::with_capacity(room(mrow, mh)), Vec::with_capacity(room(drow, dh)));
    let mut at = 0;
    let mut take = |into: &mut Vec<u8>, n: usize| {
        into.extend_from_slice(raw.get(at..at + n).unwrap_or_default());
        at += n;
    };
    let (blocks, masks, rows) = if mh >= dh { (dh, mh / dh.max(1), 1) } else { (mh, 1, dh / mh.max(1)) };
    for _ in 0..blocks {
        for _ in 0..masks {
            take(&mut mask, mrow);
        }
        for _ in 0..rows {
            take(&mut data, drow);
        }
    }
    (mask, data)
}

/// Data of an ImageType 3 interleaved by sample (the mask's value first, then `n` colour
/// components, `bpc` bits each, rows from whole bytes), split into the mask's and the image's.
pub(super) fn by_samples(raw: &[u8], w: usize, h: usize, n: usize, bpc: u8) -> (Vec<u8>, Vec<u8>) {
    let bpc = usize::from(bpc);
    let bit = |at: usize| raw.get(at / 8).is_some_and(|b| b & (0x80 >> (at % 8)) != 0);
    let (mrow, drow, row) = ((w * bpc).div_ceil(8), (w * n * bpc).div_ceil(8), (w * (n + 1) * bpc).div_ceil(8));
    let (mut mask, mut data) = (vec![0u8; mrow * h], vec![0u8; drow * h]);
    let put = |into: &mut [u8], at: usize| {
        if let Some(b) = into.get_mut(at / 8) {
            *b |= 0x80 >> (at % 8);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let from = y * row * 8 + x * (n + 1) * bpc;
            for k in 0..bpc {
                if bit(from + k) {
                    put(&mut mask, y * mrow * 8 + x * bpc + k);
                }
            }
            for k in 0..n * bpc {
                if bit(from + bpc + k) {
                    put(&mut data, y * drow * 8 + x * n * bpc + k);
                }
            }
        }
    }
    (mask, data)
}

/// Decoded pixels.
pub(super) struct Pixels {
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) rgba: Vec<u8>,
    /// A CMYK image's ink amounts: C, M, Y and K bytes a pixel, 0 is no ink.
    pub(super) inks: Option<Vec<u8>>,
    pub(super) matrix: Affine,
}

pub(super) fn source(o: &Obj) -> Res<Source> {
    Ok(match o {
        Obj::File { stream: f, .. } => Source::File(f.clone()),
        Obj::Str(s) => Source::Str(s.to_vec()),
        Obj::Array { items, .. } => Source::Proc(Obj::Array { items: items.clone(), exec: true }),
        _ => return ps_err("typecheck", "DataSource"),
    })
}

/// Do samples `raw` match `MaskColor` `m` (one value per component, or a range)?
pub(super) fn masked(m: &[u32], raw: &[u32]) -> bool {
    if m.len() == raw.len() {
        return m.iter().zip(raw).all(|(a, b)| a == b);
    }
    m.len() == raw.len() * 2
        && raw.iter().enumerate().all(|(i, v)| (m.get(2 * i).copied().unwrap_or(0)..=m.get(2 * i + 1).copied().unwrap_or(0)).contains(v))
}
