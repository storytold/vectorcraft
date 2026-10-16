//! What the export does to the file once it is written, in this order: the pages' thumbnails
//! (General › Embed page thumbnails), the seal of the editing data, fast web view (a linearised
//! file, [`crate::linearize`]) and encryption ([`crate::encrypt`]), which a linearised file goes
//! through between its two steps so that it stays linearised. Also here: what makes a file say it
//! is PDF 1.3 ([`declare_pdf13`]), for PDF 1.3 and PDF/X-1a and PDF/X-3 files, and the pages'
//! user units ([`user_units`]), which the export writes before the thumbnails.

use std::borrow::Cow;

use crate::encrypt::Obj;
use crate::linearize::{self, Parsed};
use crate::patch::Patch;
use crate::{PdfError, PdfOptions, PdfSettings};

/// Longest side of a page thumbnail, in pixels.
pub const THUMBNAIL_SIZE: u32 = 106;

/// A page thumbnail: 8-bit RGB pixels, row by row, at most [`THUMBNAIL_SIZE`] pixels a side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Thumbnail {
    /// The image XObject of a page's `/Thumb` entry; `gray`: in DeviceGray (for PDF/X files, whose
    /// device colours must suit the printing condition, an RGB thumbnail wouldn't).
    fn object(&self, gray: bool) -> Result<Vec<u8>, PdfError> {
        let (w, h) = (self.width, self.height);
        let sized = (1..=THUMBNAIL_SIZE).contains(&w) && (1..=THUMBNAIL_SIZE).contains(&h);
        if !sized || self.rgb.len() as u64 != u64::from(w) * u64::from(h) * 3 {
            return Err(PdfError::BadSetting(format!("a page thumbnail must be RGB pixels, 1 to {THUMBNAIL_SIZE} a side (got {w} × {h})")));
        }
        let (space, pixels) = if gray {
            // Rec. 601 luma.
            let luma = |p: &[u8; 3]| ((299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2]) + 500) / 1000) as u8;
            ("DeviceGray", Cow::Owned(self.rgb.as_chunks::<3>().0.iter().map(luma).collect()))
        } else {
            ("DeviceRGB", Cow::Borrowed(self.rgb.as_slice()))
        };
        let data = crate::output::deflate(&pixels).map_err(|e| PdfError::Write(format!("page thumbnail: {e}")))?;
        let mut out = format!("<</Width {w}/Height {h}/ColorSpace/{space}/BitsPerComponent 8/Filter/FlateDecode/Length {}>>\nstream\n", data.len())
            .into_bytes();
        out.extend_from_slice(&data);
        out.extend_from_slice(b"\nendstream");
        Ok(out)
    }
}

/// `pdf` with `thumbs` as the thumbnails of its pages, in page order (in grey when `gray`).
fn thumbnails(pdf: Vec<u8>, thumbs: &[Thumbnail], gray: bool) -> Result<Vec<u8>, PdfError> {
    let failed = |why: &str| PdfError::Write(format!("can't add page thumbnails: {why}"));
    let Parsed { xref, pages, .. } = linearize::parse(&pdf).map_err(failed)?;
    let mut patch = Patch::new(&xref);
    for (page, thumb) in pages.iter().zip(thumbs) {
        let (dict, _) = xref.dict(&pdf, *page).ok_or_else(|| failed("a page can't be read"))?;
        let n = patch.add_object(thumb.object(gray)?);
        patch.replace(dict, dict, format!("/Thumb {n} 0 R").into_bytes());
    }
    patch.apply(&pdf, &xref)
}

/// `pdf` with `units` as the `UserUnit` entries of its pages, in page order (a unit of 1 writes
/// none); unchanged when every unit is 1.
pub(crate) fn user_units(pdf: Vec<u8>, units: &[f64]) -> Result<Vec<u8>, PdfError> {
    if units.iter().all(|u| *u == 1.0) {
        return Ok(pdf);
    }
    let failed = |why: &str| PdfError::Write(format!("can't write the pages' user units: {why}"));
    let Parsed { xref, pages, .. } = linearize::parse(&pdf).map_err(failed)?;
    let mut patch = Patch::new(&xref);
    for (page, unit) in pages.iter().zip(units).filter(|(_, u)| **u != 1.0) {
        let (dict, _) = xref.dict(&pdf, *page).ok_or_else(|| failed("a page can't be read"))?;
        patch.replace(dict, dict, format!("/UserUnit {unit}").into_bytes());
    }
    patch.apply(&pdf, &xref)
}

/// Linearise `pdf`, encrypting it on the way as `set` says.
fn linearised(pdf: &[u8], set: &PdfSettings) -> Result<Vec<u8>, PdfError> {
    let (prepared, plan) = linearize::prepare(pdf)?;
    let (encrypted, mut cipher) = crate::encrypt::protect_keeping(prepared, set)?;
    linearize::write(&encrypted, &plan, cipher.as_mut())
}

/// The file the export wrote (`pdf`), finished as `opts` say; `editing`: it carries the editing
/// data, to seal once the pages are final. What can't be done comes back in `warnings`.
pub(crate) fn finish(pdf: Vec<u8>, opts: &PdfOptions, editing: bool, warnings: &mut Vec<String>) -> Result<Vec<u8>, PdfError> {
    let set = &opts.settings;
    let pdf = match (set.thumbnails, opts.thumbnails.is_empty()) {
        (true, false) => thumbnails(pdf, &opts.thumbnails, set.standard.is_pdfx())?,
        (true, true) => {
            warnings.push("page thumbnails need the pages drawn, which weren't given: none are embedded".into());
            pdf
        }
        (false, _) => pdf,
    };
    let pdf = if editing { crate::editing::seal(pdf)? } else { pdf };
    if !set.fast_web_view {
        return crate::encrypt::protect(pdf, set);
    }
    match linearised(&pdf, set) {
        Ok(out) => Ok(out),
        Err(e) => {
            warnings.push(format!("the file isn't optimized for fast web view: {e}"));
            crate::encrypt::protect(pdf, set)
        }
    }
}

/// Make a file written with the PDF 1.4 settings say it is PDF 1.3: its header and the version
/// its XMP metadata gives, rewritten in place.
pub(crate) fn declare_pdf13(pdf: &mut [u8]) {
    if let Some(v) = pdf.get_mut(..8).filter(|h| h.starts_with(b"%PDF-1.4")) {
        v.copy_from_slice(b"%PDF-1.3");
    }
    if let Some(digit) = metadata_version(pdf).and_then(|at| pdf.get_mut(at)) {
        *digit = b'3';
    }
}

/// Where the minor version digit is in the `1.4` of the XMP metadata stream of `pdf`'s catalog.
fn metadata_version(pdf: &[u8]) -> Option<usize> {
    const VERSION: &[u8] = b"<pdf:PDFVersion>1.4<";
    let p = linearize::parse(pdf).ok()?;
    let Some(&Obj::Ref(n, _)) = p.objects.get(&p.root)?.value.get(b"Metadata") else { return None };
    let o = p.objects.get(&n)?;
    let at = crate::lab_spot::find(pdf.get(..o.endobj)?, VERSION, o.value_span.1)?;
    Some(at + VERSION.len() - 2)
}
