//! PDF/X files (print exchange): what each standard asks of a file, and the entries that say which
//! one it is.
//!
//! - PDF/X-1a:2001: every colour is CMYK, grey or a spot ink with a CMYK alternate (colours and
//!   images are converted to a CMYK destination and written untagged, see [`crate::output`]), no
//!   transparency (the app flattens it before writing), no layers, PDF 1.3.
//! - PDF/X-3:2002: colours are ICC-based (always tagged), no transparency, no layers, PDF 1.3.
//! - PDF/X-4:2010: transparency and layers are kept, colours are tagged, PDF 1.6 at most, and the
//!   XMP metadata names the standard too.
//! - All of them: a `/GTS_PDFX` output intent (blank, the CMYK profile in effect, embedded), a
//!   TrimBox on every page (the artboard), `/Trapped` True or False, `/GTS_PDFXVersion` in the
//!   document information, a title and dates, embedded fonts (or text as outlines), no editing
//!   data and no encryption (both refused by the settings checks).
//!
//! PDF/X-1a and PDF/X-3 files are PDF 1.3 files ([`crate::PdfSettings::pdf13`]): flat, written with
//! the PDF 1.4 settings, their header and metadata saying 1.3. The written file is checked ([`verify`]): what the
//! standard forbids that is still there refuses the export, rather than a file claiming a
//! standard it breaks.

use vectorcraft_color::cms::{self, Cms, ProfileKind};

use crate::lab_spot::find;
use crate::patch::{Patch, Xref};
use crate::{PdfError, PdfSettings, Standard};

impl Standard {
    /// A PDF/X standard.
    pub fn is_pdfx(self) -> bool {
        matches!(self, Self::PdfX1a | Self::PdfX3 | Self::PdfX4)
    }

    /// Files of this standard have no transparency (PDF/X-1a and PDF/X-3): it is flattened.
    pub fn flattens(self) -> bool {
        matches!(self, Self::PdfX1a | Self::PdfX3)
    }

    /// Every colour is CMYK, grey or a spot ink (PDF/X-1a).
    pub fn cmyk_only(self) -> bool {
        self == Self::PdfX1a
    }

    /// Files of this standard may have PDF layers (PDF/X-1a and PDF/X-3 are PDF 1.3, which has
    /// none).
    pub fn allows_layers(self) -> bool {
        !self.flattens()
    }

    /// `/GTS_PDFXVersion` and `/GTS_PDFXConformance` (when the standard has one).
    fn pdfx_ids(self) -> Option<(&'static str, Option<&'static str>)> {
        match self {
            Self::PdfX1a => Some(("PDF/X-1:2001", Some("PDF/X-1a:2001"))),
            Self::PdfX3 => Some(("PDF/X-3:2002", None)),
            Self::PdfX4 => Some(("PDF/X-4", None)),
            Self::None | Self::PdfA2b => None,
        }
    }
}

/// Refuse a PDF/X output intent the file can't have: a grey profile, an RGB one in PDF/X-1a, or a
/// name that isn't a profile, unless PDF/X-1a and PDF/X-3 name a registered printing condition
/// (with a registry) without embedding a profile; PDF/X-4 always embeds one.
pub(crate) fn check(set: &PdfSettings) -> Result<(), PdfError> {
    let o = &set.output;
    let name = o.output_intent.trim();
    if !set.standard.is_pdfx() || name.is_empty() {
        return Ok(());
    }
    let std = set.standard.label();
    match cms::profile(name).map(|p| p.kind) {
        Some(ProfileKind::Cmyk) => Ok(()),
        Some(ProfileKind::Rgb) if !set.standard.cmyk_only() => Ok(()),
        Some(ProfileKind::Rgb) => {
            Err(PdfError::BadSetting(format!("output.outputIntent: {std} files print in CMYK, and “{name}” is an RGB profile")))
        }
        Some(ProfileKind::Gray) => {
            Err(PdfError::BadSetting(format!("output.outputIntent: “{name}” is a grey profile, which can't be a {std} output intent")))
        }
        None if !o.registry.trim().is_empty() && set.standard != Standard::PdfX4 => Ok(()),
        None if set.standard == Standard::PdfX4 => {
            Err(PdfError::BadSetting(format!("output.outputIntent: no profile is called “{name}”, and a {std} output intent embeds its profile")))
        }
        None => Err(PdfError::BadSetting(format!(
            "output.outputIntent: no profile is called “{name}”: a {std} output intent embeds a profile, or names a registered printing condition (outputConditionId and registry)"
        ))),
    }
}

/// The CMYK profile PDF/X-1a colours are converted to when no destination is named: the output
/// intent's profile when it is a CMYK one, else the colour settings' CMYK profile.
pub(crate) fn cmyk_destination(set: &PdfSettings, source: &Cms) -> String {
    match cms::profile(set.output.output_intent.trim()) {
        Some(p) if p.kind == ProfileKind::Cmyk => p.name,
        _ => source.settings().cmyk.clone(),
    }
}

/// The title a file of `standard` gets: PDF/X files need one.
pub(crate) fn title(standard: Standard, title: String) -> String {
    if standard.is_pdfx() && title.trim().is_empty() { "Untitled".into() } else { title }
}

/// Refuse a PDF/X file without a creation date (its dates are required).
pub(crate) fn require_date(standard: Standard, created: Option<i64>) -> Result<(), PdfError> {
    match created {
        None if standard.is_pdfx() => {
            Err(PdfError::Unsupported(format!("{} files need a creation date, and this platform has no clock to give one", standard.label())))
        }
        _ => Ok(()),
    }
}

/// The document information entries naming `standard` (empty for other files).
pub(crate) fn info_entries(standard: Standard) -> String {
    let Some((version, conformance)) = standard.pdfx_ids() else { return String::new() };
    let mut out = format!("/GTS_PDFXVersion({version})");
    if let Some(c) = conformance {
        out.push_str(&format!("/GTS_PDFXConformance({c})"));
    }
    out
}

/// Edit the XMP metadata of a PDF/X file (the catalog's `/Metadata` stream; `root` is the
/// catalog dictionary's contents) into `patch`: PDF/X-4 adds the standard, Trapped and a version
/// id (PDF 1.3 files say so in [`finish`]). Files without metadata are left as they are.
pub(crate) fn xmp(pdf: &[u8], xref: &Xref, root: (usize, usize), patch: &mut Patch, standard: Standard, trapped: bool) -> Result<(), PdfError> {
    if !standard.is_pdfx() {
        return Ok(());
    }
    let bad = || PdfError::Write("the written PDF's metadata can't be edited".into());
    let Some(at) = pdf.get(root.0..root.1).and_then(|d| find(d, b"/Metadata", 0)) else { return Ok(()) };
    let n = crate::patch::number(pdf, root.0 + at + b"/Metadata".len()).and_then(|(n, _)| u32::try_from(n).ok()).ok_or_else(bad)?;
    let (start, end) = xref.dict(pdf, n).ok_or_else(bad)?;
    let obj = pdf.get(start..end).ok_or_else(bad)?;
    let length_at = find(obj, b"/Length", 0).ok_or_else(bad)? + b"/Length".len();
    let (length, length_end) = crate::patch::number(obj, length_at).ok_or_else(bad)?;
    let data = find(obj, b"stream", length_end).map(|i| i + b"stream".len()).ok_or_else(bad)?;
    let data = data + obj.get(data..).map_or(0, |r| r.iter().take(2).take_while(|b| matches!(b, b'\r' | b'\n')).count());
    let xml = data.checked_add(length).and_then(|e| obj.get(data..e)).ok_or_else(bad)?;
    let mut edits: Vec<(usize, usize, Vec<u8>)> = vec![];
    if let Some((version, _)) = standard.pdfx_ids().filter(|_| standard == Standard::PdfX4) {
        // Into the writer's description, whose namespaces declare the `pdf` and `xmpMM` prefixes.
        let close = crate::lab_spot::rfind(xml, b"</rdf:Description>").ok_or_else(bad)?;
        let trapped = if trapped { "True" } else { "False" };
        let props = format!(
            "<pdfxid:GTS_PDFXVersion xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\">{version}</pdfxid:GTS_PDFXVersion><pdf:Trapped>{trapped}</pdf:Trapped><xmpMM:VersionID>1</xmpMM:VersionID>"
        );
        edits.push((close, close, props.into_bytes()));
    }
    // Every edit grows the stream (or keeps its length).
    let grown: usize = edits.iter().map(|(s, e, new)| new.len().saturating_sub(e - s)).sum();
    if grown > 0 {
        patch.replace(start + length_at, start + length_end, format!(" {}", length + grown).into_bytes());
    }
    for (s, e, new) in edits {
        patch.replace(start + data + s, start + data + e, new);
    }
    Ok(())
}

/// The finished file of `standard`: a PDF 1.3 file (`pdf13`: PDF/X-1a, PDF/X-3, or asked for) says
/// so ([`crate::post::declare_pdf13`]) and must be flat, then the file is checked against its
/// standard ([`verify`]).
pub(crate) fn finish(mut pdf: Vec<u8>, standard: Standard, pdf13: bool) -> Result<Vec<u8>, PdfError> {
    if pdf13 {
        crate::post::declare_pdf13(&mut pdf);
        if !standard.flattens() {
            flat(&without_streams(&pdf), "PDF 1.3")?;
        }
    }
    verify(&pdf, standard)?;
    Ok(pdf)
}

/// Refuse a file (`objects`, without its stream data) with transparency: `kind` files have none.
fn flat(objects: &[u8], kind: &str) -> Result<(), PdfError> {
    match transparency(objects) {
        Some(what) => Err(PdfError::Unsupported(format!(
            "{kind} files have no transparency, and the file would have {what}: flatten it first (object.flattenTransparency)"
        ))),
        None => Ok(()),
    }
}

/// Refuse a written file that breaks `standard`: transparency in PDF/X-1a and PDF/X-3 (soft
/// masks, transparency groups, constant opacity, blend modes), colour spaces other than CMYK,
/// grey and spot inks in PDF/X-1a, and fonts that aren't embedded.
pub(crate) fn verify(pdf: &[u8], standard: Standard) -> Result<(), PdfError> {
    if !standard.is_pdfx() {
        return Ok(());
    }
    let objects = without_streams(pdf);
    let std = standard.label();
    if standard.flattens() {
        flat(&objects, std)?;
    }
    if standard.cmyk_only()
        && let Some(space) =
            ["/DeviceRGB", "/CalRGB", "/CalGray", "/Lab", "/ICCBased"].into_iter().find(|n| !names(&objects, n.as_bytes()).is_empty())
    {
        return Err(PdfError::Unsupported(format!("{std} files are CMYK, and the file would have colours in {}", &space[1..])));
    }
    let mut from = 0;
    while let Some(at) = find(&objects, b"/Type/FontDescriptor", from) {
        let end = find(&objects, b"endobj", at).unwrap_or(objects.len());
        let dict = objects.get(at..end).unwrap_or_default();
        if find(dict, b"/FontFile", 0).is_none() {
            return Err(PdfError::Unsupported(format!("{std} files embed their fonts, and the file would have one that isn't")));
        }
        from = end;
    }
    Ok(())
}

/// `pdf` without its streams' data, the text of its literal strings and its comments (dictionaries
/// and the rest of the syntax), so that what is looked for there isn't found in image or font
/// bytes, or in a string such as the file identifier, whose base64 hash can hold `/BM`.
fn without_streams(pdf: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pdf.len().min(1 << 20));
    let mut at = 0;
    while let Some(&b) = pdf.get(at) {
        match b {
            // A literal string: parentheses nest, and a backslash escapes the next byte.
            b'(' => {
                out.extend_from_slice(b"()");
                let mut depth = 0usize;
                at += 1;
                while let Some(&c) = pdf.get(at) {
                    at += 1;
                    match c {
                        b'\\' => at += 1,
                        b'(' => depth += 1,
                        b')' if depth == 0 => break,
                        b')' => depth -= 1,
                        _ => {}
                    }
                }
            }
            // A comment, to the end of its line.
            b'%' => at += pdf.get(at..).unwrap_or_default().iter().take_while(|c| !matches!(c, b'\r' | b'\n')).count(),
            // A stream keyword (not `endstream`, and followed by its end of line): its data is left out.
            b's' if pdf.get(at..at + 6) == Some(b"stream")
                && !(at >= 3 && pdf.get(at - 3..at) == Some(b"end"))
                && pdf.get(at + 6).is_some_and(|c| matches!(c, b'\r' | b'\n')) =>
            {
                out.extend_from_slice(b"stream");
                at = find(pdf, b"endstream", at + 6).unwrap_or(pdf.len());
            }
            _ => {
                out.push(b);
                at += 1;
            }
        }
    }
    out
}

/// Where name `name` (`/SMask`) occurs in `objects` as a whole name: the index just after it.
fn names(objects: &[u8], name: &[u8]) -> Vec<usize> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(at) = find(objects, name, from) {
        let end = at + name.len();
        if !objects.get(end).is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'#')) {
            out.push(end);
        }
        from = end;
    }
    out
}

/// The token after `at` in `objects` (after white space): a name with its slash, or a number.
fn token(objects: &[u8], at: usize) -> &[u8] {
    let rest = objects.get(at..).unwrap_or_default();
    let rest = rest.get(rest.iter().take_while(|b| b.is_ascii_whitespace()).count()..).unwrap_or_default();
    let len = rest.iter().enumerate().take_while(|(i, b)| *i == 0 || !(b.is_ascii_whitespace() || b"/[]<>()".contains(b))).count();
    rest.get(..len).unwrap_or_default()
}

/// The first kind of transparency `objects` has, if any.
fn transparency(objects: &[u8]) -> Option<&'static str> {
    if names(objects, b"/SMask").into_iter().any(|at| token(objects, at) != b"/None") {
        return Some("soft masks");
    }
    if !names(objects, b"/Transparency").is_empty() {
        return Some("transparency groups");
    }
    let below_one = |t: &[u8]| std::str::from_utf8(t).ok().and_then(|s| s.parse::<f64>().ok()).is_some_and(|v| v < 1.0);
    if names(objects, b"/CA").into_iter().chain(names(objects, b"/ca")).any(|at| below_one(token(objects, at))) {
        return Some("opacity");
    }
    if names(objects, b"/BM").into_iter().any(|at| !matches!(token(objects, at), b"/Normal" | b"/Compatible")) {
        return Some("blend modes");
    }
    None
}
