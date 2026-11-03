//! Tiling patterns whose cells overlap their neighbours a little (#2823).
//!
//! A viewer paints every cell of a tiling pattern (ISO 32000-1, 8.7.3) clipped to the cell's
//! `BBox` with anti-aliased edges. When the box is exactly one step, two cells only touch: the
//! pixels along the shared edge are each partly covered by both, the paper shows through and the
//! tiling reads as a grid of hairlines. A box a little larger than the step, with the same
//! `XStep` and `YStep`, makes neighbouring cells overlap: the repeat is the same, the edge
//! pixels are covered in full, and the art in the overlap is what the neighbour draws there, so
//! opaque art shows no trace of it.
//!
//! The PDF writer only writes boxes of exactly one step, so the export draws every cell wider
//! than the box it will have ([`max_overlap`]) and [`tile_overlap`] widens the boxes of the
//! written file by [`overlap`]. The import takes it off again ([`written_overlap`]).

use crate::PdfError;
use crate::encrypt::{Lexer, Obj};
use crate::lab_spot::find;
use crate::patch::{Patch, Xref};

/// How far a cell reaches past its step, in units of the space the pattern's matrix maps to: a
/// point at a page's own scale, about a pixel at 72 ppi.
const OVERLAP: f64 = 1.0;

/// The greatest overlap, as a share of the smaller step: small tiles overlap less, so that the
/// overlap stays a seam's width and never a tile's.
const MAX_SHARE: f64 = 0.25;

/// The numbers of the overlap are written to this many decimals.
const DECIMALS: f64 = 1e4;

/// The greatest [`overlap`] of a cell of `step`: what the art of a cell is drawn past its repeat.
pub(crate) fn max_overlap(step: (f64, f64)) -> f64 {
    MAX_SHARE * step.0.min(step.1)
}

/// How far each side of a cell of `step` (pattern space) reaches past it, for a pattern placed by
/// `matrix`: about [`OVERLAP`] in the page's units, never more than [`max_overlap`]. Rounded to
/// what the file writes; 0 for a degenerate step or placement.
pub(crate) fn overlap(step: (f64, f64), matrix: [f64; 6]) -> f64 {
    let scale = (matrix[0] * matrix[3] - matrix[1] * matrix[2]).abs().sqrt();
    if !(scale.is_finite() && scale > 1e-9 && step.0.is_finite() && step.1.is_finite() && step.0 > 0.0 && step.1 > 0.0) {
        return 0.0;
    }
    ((OVERLAP / scale).min(max_overlap(step)) * DECIMALS).round() / DECIMALS
}

/// Is `bbox` (`[x0, y0, x1, y1]`) a cell of `step` placed by `matrix` that [`tile_overlap`]
/// widened: the step's box grown by [`overlap`] on every side? The overlap isn't art: a reader
/// takes it off.
pub(crate) fn written_overlap(bbox: [f64; 4], step: (f64, f64), matrix: [f64; 6]) -> bool {
    let pad = overlap(step, matrix);
    let tol = 2.0 / DECIMALS;
    let want = [-pad, -pad, step.0 + pad, step.1 + pad];
    pad > 0.0 && bbox.iter().zip(want).all(|(a, b)| (a - b).abs() <= tol)
}

/// The numbers of the array that `key` (`/BBox`) names in dictionary text `dict`, and where the
/// array (`[` to `]`) is.
fn array(dict: &[u8], key: &[u8]) -> Option<(Vec<f64>, usize, usize)> {
    let mut at = find(dict, key, 0)? + key.len();
    while dict.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    if dict.get(at) != Some(&b'[') {
        return None;
    }
    let end = find(dict, b"]", at)? + 1;
    let numbers =
        std::str::from_utf8(dict.get(at + 1..end - 1)?).ok()?.split_ascii_whitespace().map(|n| n.parse().ok()).collect::<Option<Vec<f64>>>()?;
    Some((numbers, at, end))
}

/// The number that `key` (`/XStep`) names in dictionary text `dict`.
fn number(dict: &[u8], key: &[u8]) -> Option<f64> {
    let mut at = find(dict, key, 0)? + key.len();
    while dict.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    let len = dict.get(at..)?.iter().take_while(|b| !b.is_ascii_whitespace() && !matches!(b, b'/' | b'[' | b']' | b'<' | b'>')).count();
    std::str::from_utf8(dict.get(at..at + len)?).ok()?.parse().ok()
}

/// A number as the file writes it (no exponent; whole numbers without a point).
fn write(v: f64) -> String {
    format!("{}", (v * DECIMALS).round() / DECIMALS)
}

/// `pdf` (as the export writes it: uncompressed objects, one cross-reference table) with the
/// `BBox` of every tiling pattern grown by [`overlap`] on each side, its `XStep` and `YStep`
/// untouched. A box that isn't exactly one step is left as it is.
pub(crate) fn tile_overlap(pdf: Vec<u8>) -> Result<Vec<u8>, PdfError> {
    if find(&pdf, b"/PatternType 1", 0).is_none() {
        return Ok(pdf);
    }
    let failed = |why: &str| PdfError::Write(format!("can't write the tiling patterns' overlap: {why}"));
    let xref = Xref::read(&pdf).ok_or_else(|| failed("the written PDF has no cross-reference table"))?;
    let mut patch = Patch::new(&xref);
    let mut any = false;
    for n in 1..u32::try_from(xref.count()).unwrap_or(u32::MAX) {
        let Some(off) = xref.offset(&pdf, n) else { continue };
        let mut lexer = Lexer::at(&pdf, off);
        if lexer.header().map(|h| h.0) != Some(n) {
            continue;
        }
        let Some(dict @ Obj::Dict { start, end, .. }) = lexer.object(0) else { continue };
        if dict.int(b"PatternType") != Some(1) {
            continue;
        }
        let Some(text) = pdf.get(start..end) else { continue };
        let (Some((bbox, from, to)), Some(x), Some(y), Some((matrix, ..))) =
            (array(text, b"/BBox"), number(text, b"/XStep"), number(text, b"/YStep"), array(text, b"/Matrix"))
        else {
            continue;
        };
        let (Ok(bbox), Ok(matrix)) = (<[f64; 4]>::try_from(bbox), <[f64; 6]>::try_from(matrix)) else { continue };
        let pad = overlap((x, y), matrix);
        if pad <= 0.0 || bbox != [0.0, 0.0, x, y] {
            continue;
        }
        let grown = format!("[{} {} {} {}]", write(-pad), write(-pad), write(x + pad), write(y + pad));
        patch.replace(start + from, start + to, grown.into_bytes());
        any = true;
    }
    if any { patch.apply(&pdf, &xref) } else { Ok(pdf) }
}
