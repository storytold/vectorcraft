//! Encapsulated PostScript: a hand-written EPS writer, PostScript Level 2 and 3.
//!
//! [`export`] writes the visible art of a document as one EPS file: a DSC header with the
//! `%%BoundingBox` and `%%HiResBoundingBox` of [`EpsOptions::region`] (y up, from
//! [`EpsOptions::origin`]), paths filled and stroked with their caps, joins and dashes (strokes
//! a plain line can't draw as their filled outlines, brushed strokes as their brush art), clipping
//! groups, linear and radial gradients (smooth shadings at Level 3, stepped fills at Level 2 or
//! with compatible gradient printing), pattern fills as their tiles, freeform gradients and placed
//! images as images, type as glyph outlines, spot colours as Separation colour spaces and
//! overprints when preserved. RGB colours are written as CMYK in CMYK documents and, with
//! [`EpsOptions::cmyk`], in RGB ones.
//!
//! PostScript has no transparency: callers flatten it first. What is left (opacity, blending
//! modes, opacity masks, raster effects) is written opaque, and each such loss comes back as a
//! warning.
//!
//! The file can carry a TIFF preview ([`Preview`]) behind a binary header, a PNG thumbnail and the
//! native document, both in comments that PostScript interpreters skip ([`native`],
//! [`thumbnail`] read them back).
//!
//! [`import`] reads EPS and PostScript files back: the native document when the file carries one
//! is [`native`]'s job; other files are run through a small PostScript interpreter, or come in as
//! their TIFF preview.
//!
//! - `ps`: numbers, strings, paths and the data encodings (ASCII85, run-length, Flate).
//! - `scene`: the document walk that writes the page.
//! - `tiff`: the TIFF preview.
//! - `import`: the PostScript interpreter.
//! - `print`: print jobs as PostScript files ([`print()`]), their pages drawn by `scene`.

mod import;
mod print;
mod ps;
mod scene;
mod tiff;

pub use import::{Imported, ai_alone, family_style, import, import_with, is_loss, layered_ai};
pub use print::{PrintJob, PrintPage, print};

use vectorcraft_doc::Document;
use vectorcraft_geom::{Point, Rect};

/// The PostScript language level the file needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Gradients as stepped fills, images run-length encoded, image transparency written white.
    Two,
    /// Smooth shadings, Flate-compressed images, transparent image pixels masked out.
    #[default]
    Three,
}

impl Level {
    pub const ALL: [Self; 2] = [Self::Two, Self::Three];

    /// The `level` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::Two => "2",
            Self::Three => "3",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Two => "Language Level 2",
            Self::Three => "Language Level 3",
        }
    }

    /// A level by id (`2`, `3`, `level2`, `LanguageLevel 3`), any case.
    pub fn from_id(s: &str) -> Option<Self> {
        let digits: String = s.chars().filter(char::is_ascii_digit).collect();
        Self::ALL.into_iter().find(|l| l.id() == digits)
    }
}

/// The preview other apps show for the file: none, or a TIFF image behind a binary header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preview {
    None,
    /// One bit per pixel: black where the art is darker than mid-grey.
    TiffBlackWhite,
    /// 8 bits per channel, on white or with transparency ([`EpsOptions::transparent_preview`]).
    #[default]
    TiffColor,
}

impl Preview {
    pub const ALL: [Self; 3] = [Self::None, Self::TiffBlackWhite, Self::TiffColor];

    /// The `preview` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::TiffBlackWhite => "tiffBw",
            Self::TiffColor => "tiffColor",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::TiffBlackWhite => "TIFF (Black & White)",
            Self::TiffColor => "TIFF (8-bit Color)",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| s.trim().eq_ignore_ascii_case(p.id()))
    }
}

/// What happens to overprinting fills and strokes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overprint {
    /// They overprint on press (`setoverprint`).
    #[default]
    Preserve,
    /// They knock out like any other art.
    Discard,
}

impl Overprint {
    pub const ALL: [Self; 2] = [Self::Preserve, Self::Discard];

    /// The `overprints` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Discard => "discard",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Preserve => "Preserve",
            Self::Discard => "Discard",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| s.trim().eq_ignore_ascii_case(o.id()))
    }
}

/// An image the caller rendered: straight (not premultiplied) RGBA, row by row from the top.
#[derive(Clone, Debug, Default)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The EPS export settings.
#[derive(Clone, Debug, Default)]
pub struct EpsOptions {
    pub level: Level,
    pub preview: Preview,
    /// A colour preview keeps the art's transparency (else it is on white).
    pub transparent_preview: bool,
    pub overprint: Overprint,
    /// Write RGB colours as CMYK (CMYK documents always are).
    pub cmyk: bool,
    /// Gradients as stepped fills at Level 3 too (for devices without smooth shading).
    pub compatible_gradients: bool,
    /// The area of the document the bounding box covers (document space, y down).
    pub region: Rect,
    /// The document point PostScript's origin is at (y up from it).
    pub origin: Point,
    /// `%%Title`.
    pub title: String,
    /// `%%CreationDate` as Unix seconds (UTC); `None`: left out.
    pub created: Option<i64>,
    /// The native document the file carries, for reopening it as it was.
    pub native: Option<Vec<u8>>,
}

/// A written file.
#[derive(Clone, Debug, Default)]
pub struct EpsOutput {
    pub bytes: Vec<u8>,
    /// What was approximated or left out.
    pub warnings: Vec<String>,
}

/// The `%%BoundingBox` of `region` with PostScript's origin at document point `origin`: whole
/// points enclosing it, y up (`[llx, lly, urx, ury]`).
pub fn bounding_box(region: Rect, origin: Point) -> [i64; 4] {
    let [x0, y0, x1, y1] = hires_box(region, origin);
    [x0.floor() as i64, y0.floor() as i64, x1.ceil() as i64, y1.ceil() as i64]
}

/// The `%%HiResBoundingBox` of `region` (see [`bounding_box`]).
fn hires_box(region: Rect, origin: Point) -> [f64; 4] {
    let r = region.abs();
    [r.x0 - origin.x, origin.y - r.y1, r.x1 - origin.x, origin.y - r.y0]
}

/// The document rect a preview must show: the whole-point [`bounding_box`], which it covers.
pub fn preview_rect(region: Rect, origin: Point) -> Rect {
    let [x0, y0, x1, y1] = bounding_box(region, origin).map(|v| v as f64);
    Rect::new(origin.x + x0, origin.y - y1, origin.x + x1, origin.y - y0)
}

/// Write `doc` as EPS with `opts`. `preview` is the art of [`preview_rect`] when
/// [`EpsOptions::preview`] asks for one, and `thumbnail` a small image of it to embed. Live
/// geometry effects are applied first; template layers, guides and hidden objects are left out.
pub fn export(doc: &Document, opts: &EpsOptions, preview: Option<&Raster>, thumbnail: Option<&Raster>) -> Result<EpsOutput, String> {
    let r = opts.region;
    if ![r.x0, r.y0, r.x1, r.y1, opts.origin.x, opts.origin.y].iter().all(|v| v.is_finite()) {
        return Err("the export region is not finite".into());
    }
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    let page = scene::Scene::new(doc, opts).run();
    let mut warnings = page.warnings;
    let thumb = thumbnail.and_then(|t| match tiff::png(t) {
        Ok(png) => Some(png),
        Err(e) => {
            warnings.push(format!("the thumbnail was left out: {e}"));
            None
        }
    });
    let ps = document(opts, &page.setup, &page.body, &page.custom, thumb.as_deref());
    let bytes = match (opts.preview, preview) {
        (Preview::None, _) => ps.into_bytes(),
        (_, None) => {
            warnings.push("the preview was left out: nothing was rendered for it".into());
            ps.into_bytes()
        }
        (kind, Some(img)) => {
            let tiff = tiff::encode(img, kind == Preview::TiffBlackWhite, opts.transparent_preview && kind == Preview::TiffColor)?;
            dos_eps(ps.as_bytes(), &tiff)?
        }
    };
    Ok(EpsOutput { bytes, warnings })
}

/// Comment lines that start a data section the file carries (`%VectorCraft_BeginData: <name>`)
/// and end it; between them, ASCII85 lines after `% `.
const BEGIN_DATA: &str = "%VectorCraft_BeginData: ";
const END_DATA: &str = "%VectorCraft_EndData";
/// The data sections.
const NATIVE: &str = "native";
const THUMBNAIL: &str = "thumbnail";
/// Largest native document [`native`] inflates (bytes).
const MAX_NATIVE: u64 = 1 << 30;

/// The DSC comments every file of ours has after its version line: creator, title, date,
/// language level, data, and the spot colours `custom` with their CMYK equivalents.
pub(crate) fn comments(s: &mut String, title: &str, created: Option<i64>, level: Level, custom: &[(String, [f32; 4])]) {
    use std::fmt::Write;
    let _ = writeln!(s, "%%Creator: VectorCraft {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "%%Title: {}", ps::string(title));
    if let Some(t) = created {
        let [y, mo, d, h, mi, se] = vectorcraft_doc::metadata::civil(t);
        let _ = writeln!(s, "%%CreationDate: ({y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{se:02} UTC)");
    }
    let _ = writeln!(s, "%%LanguageLevel: {}", level.id());
    s.push_str("%%DocumentData: Clean7Bit\n");
    for (i, (name, cmyk)) in custom.iter().enumerate() {
        let _ = writeln!(s, "{}{}", if i == 0 { "%%DocumentCustomColors: " } else { "%%+ " }, ps::string(name));
        let [c, m, y, k] = cmyk.map(|v| ps::num(f64::from(v)));
        let _ = writeln!(s, "%%CMYKCustomColor: {c} {m} {y} {k} {}", ps::string(name));
    }
}

/// The PostScript program: header comments, thumbnail, prolog, setup, the page and the trailer
/// with the native document.
fn document(o: &EpsOptions, setup: &str, body: &str, custom: &[(String, [f32; 4])], thumb: Option<&[u8]>) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(body.len() + setup.len() + 4096);
    let bb = bounding_box(o.region, o.origin);
    let hi = hires_box(o.region, o.origin);
    s.push_str("%!PS-Adobe-3.0 EPSF-3.0\n"); // brand-ok: the DSC version line every EPS file starts with
    comments(&mut s, &o.title, o.created, o.level, custom);
    let _ = writeln!(s, "%%BoundingBox: {} {} {} {}", bb[0], bb[1], bb[2], bb[3]);
    let _ = writeln!(s, "%%HiResBoundingBox: {} {} {} {}", ps::num(hi[0]), ps::num(hi[1]), ps::num(hi[2]), ps::num(hi[3]));
    s.push_str("%%Pages: 1\n%%EndComments\n");
    if let Some(png) = thumb {
        data_section(&mut s, THUMBNAIL, png);
    }
    s.push_str("%%BeginProlog\n");
    s.push_str(ps::PROLOG);
    s.push_str("%%EndProlog\n%%BeginSetup\n");
    if !setup.is_empty() {
        s.push_str("VCdict begin\n");
        s.push_str(setup);
        s.push_str("end\n");
    }
    s.push_str("%%EndSetup\n%%Page: 1 1\nVCdict begin\nq\n");
    // Document space (y down) onto the page (y up from the origin).
    let _ = writeln!(s, "[1 0 0 -1 {} {}] cm", ps::num(-o.origin.x), ps::num(o.origin.y));
    s.push_str(body);
    s.push_str("Q\nend\nshowpage\n%%PageTrailer\n%%Trailer\n");
    if let Some(native) = o.native.as_deref() {
        data_section(&mut s, NATIVE, &ps::deflate(native));
    }
    s.push_str("%%EOF\n");
    s
}

/// A data section of comment lines carrying `bytes` (see [`BEGIN_DATA`]).
fn data_section(s: &mut String, name: &str, bytes: &[u8]) {
    s.push_str(BEGIN_DATA);
    s.push_str(name);
    s.push('\n');
    for line in ps::ascii85(bytes).lines() {
        s.push_str("% ");
        s.push_str(line);
        s.push('\n');
    }
    s.push_str(END_DATA);
    s.push('\n');
}

/// The bytes of data section `name` in the PostScript `ps`, when it has one that decodes.
fn read_data(ps: &[u8], name: &str) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(ps);
    let mut lines = text.lines();
    lines.find(|l| l.strip_prefix(BEGIN_DATA).is_some_and(|n| n.trim() == name))?;
    let mut encoded = String::new();
    for l in lines {
        if l.starts_with(END_DATA) {
            return ps::ascii85_decode(&encoded);
        }
        encoded.push_str(l.strip_prefix('%').unwrap_or(l));
    }
    None
}

/// The magic number of an EPS file with a binary header (a preview).
const DOS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];
/// The binary header's size.
const DOS_HEADER: usize = 30;

/// `ps` behind a binary header, followed by the TIFF preview `tiff`.
fn dos_eps(ps: &[u8], tiff: &[u8]) -> Result<Vec<u8>, String> {
    let big = || "the file is too large for an EPS preview header (4 GB): choose no preview".to_string();
    let ps_len = u32::try_from(ps.len()).map_err(|_| big())?;
    let tiff_at = u32::try_from(DOS_HEADER + ps.len()).map_err(|_| big())?;
    let tiff_len = u32::try_from(tiff.len()).map_err(|_| big())?;
    tiff_at.checked_add(tiff_len).ok_or_else(big)?;
    let mut out = Vec::with_capacity(DOS_HEADER + ps.len() + tiff.len());
    out.extend(DOS_MAGIC);
    out.extend((DOS_HEADER as u32).to_le_bytes());
    out.extend(ps_len.to_le_bytes());
    // No metafile preview.
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(tiff_at.to_le_bytes());
    out.extend(tiff_len.to_le_bytes());
    // No checksum.
    out.extend(0xFFFFu16.to_le_bytes());
    out.extend(ps);
    out.extend(tiff);
    Ok(out)
}

/// The sections of an EPS file: its PostScript and its TIFF preview (binary header), or the whole
/// file as PostScript. `None` when a binary header points outside the file.
pub fn sections(bytes: &[u8]) -> Option<(&[u8], Option<&[u8]>)> {
    if !bytes.starts_with(&DOS_MAGIC) {
        return Some((bytes, None));
    }
    let word = |at: usize| bytes.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u32::from_le_bytes(b) as usize);
    let part = |at: usize| {
        let (start, len) = (word(at)?, word(at + 4)?);
        bytes.get(start..start.checked_add(len)?)
    };
    let ps = part(4)?;
    let tiff = match word(24) {
        Some(0) | None => None,
        Some(_) => Some(part(20)?),
    };
    Some((ps, tiff))
}

/// The Windows metafile preview behind an EPS file's binary header, if it has one.
pub fn metafile_preview(bytes: &[u8]) -> Option<&[u8]> {
    if !bytes.starts_with(&DOS_MAGIC) {
        return None;
    }
    let word = |at: usize| bytes.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u32::from_le_bytes(b) as usize);
    let (start, len) = (word(12)?, word(16)?);
    if start == 0 || len == 0 {
        return None;
    }
    bytes.get(start..start.checked_add(len)?)
}

/// The native document an EPS file written by [`export`] carries, if any.
pub fn native(bytes: &[u8]) -> Option<Vec<u8>> {
    let (ps, _) = sections(bytes)?;
    ps::inflate(&read_data(ps, NATIVE)?, MAX_NATIVE)
}

/// Does the EPS file carry a native document (readable or not)?
pub fn has_native(bytes: &[u8]) -> bool {
    let marker = format!("{BEGIN_DATA}{NATIVE}");
    sections(bytes).is_some_and(|(ps, _)| ps.windows(marker.len()).any(|w| w == marker.as_bytes()))
}

/// The PNG thumbnail an EPS file written by [`export`] carries, if any.
pub fn thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    read_data(sections(bytes)?.0, THUMBNAIL)
}

#[cfg(test)]
mod tests;
