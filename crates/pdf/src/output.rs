//! Save PDF › Output: colours converted to a destination profile, the ICC profiles that tag them,
//! and the output intent and Trapped entries.
//!
//! A document's colours are in its own colour spaces ([`document_cms`]): the profiles assigned to
//! it (Edit › Assign Profile), which it keeps when saved and opened again, else the working spaces.
//!
//! - Conversion ([`ColorConversion`]): `destination` converts every colour to the destination
//!   profile's model (CMYK or RGB) with the colour settings' intent, also colours already in that
//!   model when their profile differs from the destination; `preserveNumbers` converts only the
//!   colours of the other model (and Lab ones), keeping the numbers of those already in it.
//!   Grey stays grey. Images (and freeform gradients' images) are converted the same way, pixel
//!   by pixel (CMYK images too: they keep their numbers unless converted, see [`CmykPixels`]);
//!   printer's marks are not. The destination is a profile name; empty, the document's
//!   profile for its colour mode.
//! - Profiles ([`ProfileInclusion`]): with `all`, with `destination`, and with `taggedSource` on a
//!   document that was assigned profiles, colours are written in ICC-based colour spaces: CMYK
//!   with the CMYK profile in effect (the destination when converting to CMYK, else the
//!   document's), RGB with sRGB (colours of another RGB space as their sRGB equivalents) and grey
//!   with the sRGB tone curve. Otherwise they are device colours. PDF/A, PDF/X-3 and PDF/X-4
//!   always tag them, whatever `profiles` says; PDF/X-1a never does, and always converts to a
//!   CMYK destination (see [`crate::pdfx`]).
//! - Output intent: the profile `outputIntent` names, embedded as the `/GTS_PDFX` output intent
//!   of the catalog with the condition, identifier and registry; Trapped goes in the document
//!   information (`/False` when an output intent is written untrapped). PDF/A files carry their
//!   own output intent: neither is written there. PDF/X files always have one: blank, the CMYK
//!   profile in effect. A profile that also tags colours is embedded once, shared by both.

use std::sync::{Arc, OnceLock};

use craft_pdf::write::backend as krilla;
use krilla::image::ImageColorspace;
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, Cms, CmykLut, Intent, Model, ProfileKind, ProofLut};
use vectorcraft_doc::{ColorMode, Document};

use crate::patch::{Patch, Xref};
use crate::{ColorConversion, PdfError, PdfSettings, ProfileInclusion, Standard};

/// Most characters of an output intent's text entries written.
const MAX_TEXT: usize = 255;

/// The destination colours are converted to.
struct Dest {
    cms: Cms,
    model: Model,
    /// Colours already in the destination's model keep their numbers (Preserve Numbers, or the
    /// destination is their own profile).
    keep: bool,
    /// The destination is sRGB (what images are).
    srgb: bool,
}

/// How an export writes colours (see the module documentation).
pub(crate) struct ColorOut {
    /// The colour settings colours come from: the document's colour spaces ([`document_cms`]).
    source: Arc<Cms>,
    /// Why colours aren't in a profile assigned to the document (warnings).
    pub profile_notes: Vec<String>,
    dest: Option<Dest>,
    /// Colours are written in ICC-based colour spaces.
    pub tagged: bool,
    /// The CMYK profile that tags CMYK colours.
    cmyk_profile: String,
    /// Tagged RGB colours are written as their sRGB equivalents: why (a warning), when their RGB
    /// space isn't sRGB.
    pub srgb_note: Option<String>,
    /// The image transform, built when the first image needs it.
    pixels: OnceLock<Option<Pixels>>,
    /// The CMYK image transform, built when the first CMYK image needs it.
    cmyk_pixels: OnceLock<Option<CmykPixels>>,
    intent: Intent,
    /// The output intent and Trapped entries to write.
    catalog: Option<Catalog>,
    /// The standard the file conforms to.
    pub standard: Standard,
}

/// An output intent dictionary's entries.
struct OutputIntent {
    /// `/OutputConditionIdentifier`.
    id: String,
    /// `/OutputCondition`, `/RegistryName` and `/Info` (left out when empty).
    condition: String,
    registry: String,
    info: String,
    /// The profile to embed (`/DestOutputProfile`): its name and number of components.
    profile: Option<(String, u8)>,
}

/// The output intent and Trapped entries of the catalog and document information.
struct Catalog {
    intent: Option<OutputIntent>,
    /// `/Trapped`, when it is written.
    trapped: Option<bool>,
}

impl Default for ColorOut {
    /// Colours as they are, device colours, nothing in the catalog.
    fn default() -> Self {
        let source = cms::active();
        let cmyk_profile = source.settings().cmyk.clone();
        let intent = source.settings().intent;
        Self {
            source,
            profile_notes: vec![],
            dest: None,
            tagged: false,
            cmyk_profile,
            srgb_note: None,
            pixels: OnceLock::new(),
            cmyk_pixels: OnceLock::new(),
            intent,
            catalog: None,
            standard: Standard::None,
        }
    }
}

/// The colour settings `doc`'s colours are in: the active ones, with the profiles assigned to the
/// document (Edit › Assign Profile) as its RGB and CMYK spaces. The document keeps them when it
/// is saved and opened again, while the working spaces are the app's (a new session starts with
/// the defaults), so the two can differ. An assigned profile that isn't available leaves its
/// working space, with a warning in the second value.
pub(crate) fn document_cms(doc: &Document) -> (Arc<Cms>, Vec<String>) {
    let active = cms::active();
    let mut settings = active.settings().clone();
    let mut notes = vec![];
    let assigned = [(&doc.color_profiles.rgb, ProfileKind::Rgb, "RGB"), (&doc.color_profiles.cmyk, ProfileKind::Cmyk, "CMYK")];
    for (name, kind, label) in assigned {
        let Some(name) = name else { continue };
        let space = if kind == ProfileKind::Rgb { &mut settings.rgb } else { &mut settings.cmyk };
        match cms::profile(name).filter(|p| p.kind == kind) {
            Some(p) => *space = p.name,
            None => notes.push(format!(
                "the document's {label} profile “{name}” isn't available: its {label} colours are written in the working {label} space, {space}"
            )),
        }
    }
    if settings == *active.settings() {
        return (active, notes);
    }
    match Cms::new(&settings) {
        Ok(own) => (Arc::new(own), notes),
        Err(e) => {
            notes.push(format!("the document's colour profiles can't be used ({e}): its colours are written in the working spaces"));
            (active, notes)
        }
    }
}

/// The profile of `set`'s destination and its colour model; `None` without conversion. A blank
/// destination is the profile of a document in colour mode `mode`. PDF/X-1a files always convert,
/// to a CMYK destination: blank, their output intent's profile ([`crate::pdfx::cmyk_destination`]).
fn destination(set: &PdfSettings, mode: ColorMode, source: &Cms) -> Result<Option<(String, Model)>, PdfError> {
    let o = &set.output;
    let cmyk_only = set.standard.cmyk_only();
    if o.conversion == ColorConversion::None && !cmyk_only {
        return Ok(None);
    }
    let name = o.destination.trim();
    let fallback = if cmyk_only { crate::pdfx::cmyk_destination(set, source) } else { String::new() };
    let name = match (name.is_empty(), mode) {
        // Document CMYK: the CMYK profile in effect, in an RGB document too (#2950).
        (false, _) if name.eq_ignore_ascii_case(crate::DOCUMENT_CMYK) => source.settings().cmyk.as_str(),
        (false, _) => name,
        (true, _) if cmyk_only => fallback.as_str(),
        (true, ColorMode::Cmyk) => source.settings().cmyk.as_str(),
        (true, ColorMode::Rgb) => source.settings().rgb.as_str(),
    };
    let unknown = || PdfError::BadSetting(format!("output.destination: no RGB or CMYK profile is called “{name}”"));
    let info = cms::profile(name).ok_or_else(unknown)?;
    let model = match info.kind {
        ProfileKind::Rgb => Model::Rgb,
        ProfileKind::Cmyk => Model::Cmyk,
        ProfileKind::Gray => return Err(unknown()),
    };
    if cmyk_only && model != Model::Cmyk {
        return Err(PdfError::BadSetting(format!("output.destination: {} files are CMYK, and “{name}” is an RGB profile", set.standard.label())));
    }
    Ok(Some((info.name, model)))
}

/// Refuse a destination profile that isn't there (when converting).
pub(crate) fn check(set: &PdfSettings) -> Result<(), PdfError> {
    // A blank destination is the document's own profile, which is always there.
    if !set.output.destination.trim().is_empty() {
        destination(set, ColorMode::Rgb, &cms::active())?;
    }
    Ok(())
}

/// What `set`'s Output section leaves out, one warning each.
pub(crate) fn warnings(set: &PdfSettings) -> Vec<String> {
    let o = &set.output;
    let asks = !o.output_intent.trim().is_empty() || !o.output_condition_id.trim().is_empty();
    let mut out = vec![];
    if set.standard == Standard::PdfA2b {
        if asks || o.trapped {
            out.push("PDF/A files carry their own output intent: the output intent and Trapped entries are left out".into());
        }
        return out;
    }
    let name = o.output_intent.trim();
    if !name.is_empty() && cms::profile(name).is_none() {
        out.push(format!("no profile is called “{name}”: the output intent names it without embedding it"));
    }
    // PDF/X files always have an output intent.
    if !asks && !set.standard.is_pdfx() && (!o.output_condition.trim().is_empty() || !o.registry.trim().is_empty()) {
        out.push("an output intent needs a profile or a condition identifier: the output condition and registry are left out".into());
    }
    out
}

impl ColorOut {
    /// How `set` writes the colours of `doc` (its colour spaces, [`document_cms`], are the source).
    pub(crate) fn new(doc: &Document, set: &PdfSettings) -> Result<Self, PdfError> {
        let (source, profile_notes) = document_cms(doc);
        let cmyk_profile = source.settings().cmyk.clone();
        let mut out = Self { source, profile_notes, cmyk_profile, ..Self::default() };
        let source = out.source.clone();
        let st = source.settings();
        if let Some((name, model)) = destination(set, doc.color_mode, &source)? {
            let mut settings = st.clone();
            let own = match model {
                Model::Cmyk => std::mem::replace(&mut settings.cmyk, name.clone()),
                _ => std::mem::replace(&mut settings.rgb, name.clone()),
            };
            let cms = Cms::new(&settings).map_err(|e| PdfError::BadSetting(format!("output.destination: {e}")))?;
            // PDF/X-1a converts without being asked: as Preserve Numbers does.
            let keep = set.output.conversion != ColorConversion::Destination || cms::canonical_name(&own) == name;
            if model == Model::Cmyk {
                out.cmyk_profile = name.clone();
            }
            out.dest = Some(Dest { srgb: name == cms::SRGB, cms, model, keep });
        }
        out.tagged = matches!(set.standard, Standard::PdfA2b | Standard::PdfX3 | Standard::PdfX4)
            || (!set.standard.cmyk_only()
                && match set.output.profiles {
                    ProfileInclusion::None => false,
                    ProfileInclusion::All | ProfileInclusion::Destination => true,
                    ProfileInclusion::TaggedSource => !doc.color_profiles.is_empty(),
                });
        let rgb = match &out.dest {
            Some(d) if d.model == Model::Rgb => (d.srgb, d.cms.settings().rgb.clone()),
            _ => (source.rgb_is_srgb(), st.rgb.clone()),
        };
        if out.tagged && !rgb.0 {
            out.srgb_note = Some(format!("RGB colours are tagged with the sRGB profile: those of {} are written as their sRGB equivalents", rgb.1));
        }
        out.catalog = catalog(set, &out.cmyk_profile);
        out.standard = set.standard;
        Ok(out)
    }

    /// Transparency blends in CMYK: converting to CMYK, or (without conversion) in a CMYK
    /// document (`cmyk`).
    pub(crate) fn blends_cmyk(&self, cmyk: bool) -> bool {
        self.dest.as_ref().map_or(cmyk, |d| d.model == Model::Cmyk)
    }

    /// The ICC profile that tags CMYK colours (when they are tagged).
    pub(crate) fn cmyk_icc(&self) -> Option<krilla::icc::ICCProfile<4>> {
        krilla::icc::ICCProfile::new(&cms::icc_bytes(&self.cmyk_profile).ok()?)
    }

    /// The name of that profile.
    pub(crate) fn cmyk_profile(&self) -> &str {
        &self.cmyk_profile
    }

    /// Colour `c` as it is written with `intent`: converted to the destination (with `convert`;
    /// printer's marks aren't), else RGB and Lab colours of a CMYK document (`cmyk_doc`)
    /// separated into CMYK; tagged RGB colours as their sRGB equivalents.
    pub(crate) fn color(&self, c: &Color, intent: Intent, cmyk_doc: bool, convert: bool) -> Color {
        let s = &self.source;
        let c = match &self.dest {
            Some(d) if convert => d.convert(s, c, intent),
            _ if cmyk_doc && matches!(c, Color::Rgb { .. } | Color::Lab { .. }) => {
                let [c, m, y, k] = s.to_cmyk(c, intent);
                Color::Cmyk { c, m, y, k }
            }
            _ => *c,
        };
        match c {
            Color::Rgb { r, g, b } if self.srgb_note.is_some() => {
                let space = self.dest.as_ref().filter(|d| convert && d.model == Model::Rgb).map_or(&**s, |d| &d.cms);
                let [r, g, b] = space.rgb_to_srgb([r, g, b]);
                Color::Rgb { r, g, b }
            }
            c => c,
        }
    }

    /// Ink values of `c` in the CMYK space of the output (a spot colour's alternate).
    pub(crate) fn cmyk(&self, c: &Color, intent: Intent) -> [f32; 4] {
        match &self.dest {
            Some(d) if d.model == Model::Cmyk => d.cms.to_cmyk(&d.convert(&self.source, c, intent), intent),
            _ => self.source.to_cmyk(c, intent),
        }
    }

    /// How colour images' pixels are converted; `None` when they are written as they are.
    pub(crate) fn pixels(&self) -> Option<&Pixels> {
        self.pixels
            .get_or_init(|| {
                let d = self.dest.as_ref()?;
                let intent = self.intent;
                match d.model {
                    Model::Cmyk => {
                        let plane = |k: bool| {
                            ProofLut::build(|rgb| {
                                let v = d.cms.srgb_to_cmyk(rgb, intent);
                                if k { [v[3], 0.0, 0.0] } else { [v[0], v[1], v[2]] }
                            })
                        };
                        Some(Pixels::Cmyk(Box::new([plane(false), plane(true)])))
                    }
                    // Images are sRGB: converted unless the destination is sRGB, they keep their
                    // numbers or RGB is tagged as sRGB.
                    _ if d.srgb || d.keep || self.tagged => None,
                    _ => Some(Pixels::Rgb(Box::new(ProofLut::build(|rgb| d.cms.srgb_to_rgb(rgb))))),
                }
            })
            .as_ref()
    }

    /// How CMYK images' ink amounts are converted; `None` when they keep their numbers (no
    /// conversion, or to a CMYK destination that keeps them).
    pub(crate) fn cmyk_pixels(&self) -> Option<&CmykPixels> {
        self.cmyk_pixels
            .get_or_init(|| {
                let d = self.dest.as_ref().filter(|d| !(d.keep && d.model == Model::Cmyk))?;
                let cmyk = d.model == Model::Cmyk;
                // Each ink combination as a CMYK colour is written.
                let lut = CmykLut::build(|[c, m, y, k]| {
                    let out = self.color(&Color::Cmyk { c, m, y, k }, self.intent, false, true);
                    if cmyk {
                        return d.cms.to_cmyk(&out, self.intent);
                    }
                    let [r, g, b] = match out {
                        Color::Rgb { r, g, b } => [r, g, b],
                        other => d.cms.srgb_to_rgb(d.cms.display_rgb(&other)),
                    };
                    [r, g, b, 0.0]
                });
                Some(CmykPixels { lut, cmyk })
            })
            .as_ref()
    }

    /// `pdf` with the output intent and Trapped entries, and a PDF/X file's identification
    /// ([`crate::pdfx`]) → (the file, warnings).
    pub(crate) fn write_catalog(&self, pdf: Vec<u8>) -> Result<(Vec<u8>, Vec<String>), PdfError> {
        let Some(cat) = &self.catalog else { return Ok((pdf, vec![])) };
        let xref = Xref::read(&pdf).ok_or_else(|| PdfError::Write("the written PDF has no cross-reference table".into()))?;
        let mut patch = Patch::new(&xref);
        let mut warnings = vec![];
        let root = xref.trailer_ref(&pdf, b"/Root").and_then(|n| xref.dict(&pdf, n));
        let Some(root) = root else { return Err(PdfError::Write("the written PDF has no catalog".into())) };
        if let Some(OutputIntent { id, condition, registry, info, profile }) = &cat.intent {
            if crate::lab_spot::find(pdf.get(root.0..root.1).unwrap_or_default(), b"/OutputIntents", 0).is_some() {
                warnings.push("the file has an output intent already: the one asked for is left out".into());
            } else {
                let mut dict = format!("<</Type/OutputIntent/S/GTS_PDFX/OutputConditionIdentifier{}", text_string(id));
                for (key, v) in [("OutputCondition", condition), ("RegistryName", registry), ("Info", info)] {
                    if !v.is_empty() {
                        dict.push_str(&format!("/{key}{}", text_string(v)));
                    }
                }
                if let Some((name, n)) = profile {
                    let icc = cms::icc_bytes(name).map_err(|e| PdfError::Write(format!("output intent profile: {e}")))?;
                    // The profile tagging the colours is often this one: both share its stream.
                    let shared = if self.tagged { tagging_stream(&pdf, &xref, &icc) } else { None };
                    let stream = match shared {
                        Some(s) => s,
                        None => {
                            let data = deflate(&icc).map_err(|e| PdfError::Write(format!("output intent profile: {e}")))?;
                            let mut stream = format!("<</N {n}/Length {}/Filter/FlateDecode>>\nstream\n", data.len()).into_bytes();
                            stream.extend_from_slice(&data);
                            stream.extend_from_slice(b"\nendstream");
                            patch.add_object(stream)
                        }
                    };
                    dict.push_str(&format!("/DestOutputProfile {stream} 0 R"));
                }
                dict.push_str(">>");
                let oi = patch.add_object(dict.into_bytes());
                patch.replace(root.0, root.0, format!("/OutputIntents[{oi} 0 R]").into_bytes());
            }
        }
        let info = xref.trailer_ref(&pdf, b"/Info").and_then(|n| xref.dict(&pdf, n));
        let mut entries = String::new();
        if let Some(trapped) = cat.trapped
            && info.is_none_or(|(s, e)| crate::lab_spot::find(pdf.get(s..e).unwrap_or_default(), b"/Trapped", 0).is_none())
        {
            entries.push_str(if trapped { "/Trapped/True" } else { "/Trapped/False" });
        }
        entries.push_str(&crate::pdfx::info_entries(self.standard));
        match info {
            _ if entries.is_empty() => {}
            Some((at, _)) => patch.replace(at, at, entries.into_bytes()),
            None => {
                let n = patch.add_object(format!("<<{entries}>>").into_bytes());
                patch.trailer_entry(format!("/Info {n} 0 R").as_bytes());
            }
        }
        crate::pdfx::xmp(&pdf, &xref, root, &mut patch, self.standard, cat.trapped == Some(true))?;
        Ok((patch.apply(&pdf, &xref)?, warnings))
    }
}

impl Dest {
    /// `c` converted from `source` (see the module documentation).
    fn convert(&self, source: &Cms, c: &Color, intent: Intent) -> Color {
        match (self.model, c) {
            (_, Color::Gray { .. }) | (Model::Cmyk, Color::Cmyk { .. }) | (Model::Rgb, Color::Rgb { .. })
                if self.keep || matches!(c, Color::Gray { .. }) =>
            {
                *c
            }
            (Model::Cmyk, _) => {
                // Through Lab (CMYK of another profile, Lab) or the display colour (RGB).
                let [c, m, y, k] = match c {
                    Color::Rgb { .. } => self.cms.srgb_to_cmyk(source.display_rgb(c), intent),
                    _ => self.cms.lab_to_cmyk(source.lab(c), intent),
                };
                Color::Cmyk { c, m, y, k }
            }
            // CMYK straight into the destination RGB (the destination shares the source's CMYK).
            (_, Color::Cmyk { c, m, y, k }) => {
                let [r, g, b] = self.cms.cmyk_to_rgb([*c, *m, *y, *k], intent);
                Color::Rgb { r, g, b }
            }
            _ => {
                let [r, g, b] = self.cms.srgb_to_rgb(source.display_rgb(c));
                Color::Rgb { r, g, b }
            }
        }
    }
}

/// The catalog entries `set` asks for (none in PDF/A files, see [`warnings`]); a PDF/X file's
/// output intent profile is `cmyk_profile` (the CMYK profile in effect) unless `set` names one.
fn catalog(set: &PdfSettings, cmyk_profile: &str) -> Option<Catalog> {
    let o = &set.output;
    if set.standard == Standard::PdfA2b {
        return None;
    }
    let (mut name, id) = (o.output_intent.trim(), o.output_condition_id.trim());
    if name.is_empty() && set.standard.is_pdfx() {
        name = cmyk_profile;
    }
    let intent = (!name.is_empty() || !id.is_empty()).then(|| {
        let profile = cms::profile(name).and_then(|p| match p.kind {
            ProfileKind::Rgb => Some((p.name, 3)),
            ProfileKind::Cmyk => Some((p.name, 4)),
            ProfileKind::Gray => None,
        });
        let text = |s: &str| s.trim().chars().take(MAX_TEXT).collect::<String>();
        OutputIntent {
            id: text(if id.is_empty() { name } else { id }),
            condition: text(&o.output_condition),
            registry: text(&o.registry),
            info: text(name),
            profile,
        }
    });
    let trapped = (o.trapped || intent.is_some()).then_some(o.trapped);
    (intent.is_some() || trapped.is_some()).then_some(Catalog { intent, trapped })
}

/// The object number of the stream of an ICC-based colour space of `pdf` that holds profile `icc`
/// (its data as it is, or Flate-compressed); `None` when no colour space uses that profile.
fn tagging_stream(pdf: &[u8], xref: &Xref, icc: &[u8]) -> Option<u32> {
    use std::io::Read;
    const ICC: &[u8] = b"[/ICCBased ";
    let mut from = 0;
    while let Some(at) = crate::lab_spot::find(pdf, ICC, from) {
        from = at + ICC.len();
        let Some(n) = crate::patch::number(pdf, from).and_then(|(n, _)| u32::try_from(n).ok()) else { continue };
        let Some(obj) = xref.dict(pdf, n).and_then(|(s, e)| pdf.get(s..e)) else { continue };
        let Some(length) = crate::lab_spot::find(obj, b"/Length", 0).and_then(|i| crate::patch::number(obj, i + b"/Length".len())) else {
            continue;
        };
        let Some(data) = crate::lab_spot::find(obj, b"stream", length.1).map(|i| i + b"stream".len()) else { continue };
        let data = data + obj.get(data..).map_or(0, |r| r.iter().take(2).take_while(|b| matches!(b, b'\r' | b'\n')).count());
        let Some(raw) = data.checked_add(length.0).and_then(|e| obj.get(data..e)) else { continue };
        let same = if crate::lab_spot::find(obj.get(..data).unwrap_or_default(), b"/FlateDecode", 0).is_some() {
            let mut out = vec![];
            // One byte more than the profile is enough to tell a longer one apart.
            flate2::read::ZlibDecoder::new(raw).take(icc.len() as u64 + 1).read_to_end(&mut out).is_ok() && out == icc
        } else {
            raw == icc
        };
        if same {
            return Some(n);
        }
    }
    None
}

/// `s` as a PDF text string: a literal string when it is printable ASCII, else UTF-16BE (with its
/// byte order mark) in hex.
pub(crate) fn text_string(s: &str) -> String {
    if s.bytes().all(|b| (b' '..=b'~').contains(&b)) {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('(');
        for c in s.chars() {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        out
    } else {
        let mut out = String::from("<FEFF");
        for u in s.encode_utf16() {
            out.push_str(&format!("{u:04X}"));
        }
        out.push('>');
        out
    }
}

pub(crate) fn deflate(data: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data)?;
    e.finish()
}

/// How colour images' pixels are converted to the destination.
pub(crate) enum Pixels {
    /// To another RGB space.
    Rgb(Box<ProofLut>),
    /// To CMYK: the cyan, magenta and yellow plane, and the black one.
    Cmyk(Box<[ProofLut; 2]>),
}

impl Pixels {
    /// Convert the RGB samples of straight RGBA pixels `rgba` in place (RGB destinations).
    pub(crate) fn rgb_in_place(&self, rgba: &mut [u8]) {
        if let Self::Rgb(lut) = self {
            for p in rgba.as_chunks_mut::<4>().0 {
                let [r, g, b] = lut.apply8([p[0], p[1], p[2]]);
                (p[0], p[1], p[2]) = (r, g, b);
            }
        }
    }

    /// The CMYK samples of straight RGBA pixels `rgba` (CMYK destinations).
    pub(crate) fn cmyk(&self, rgba: &[u8]) -> Option<Vec<u8>> {
        let Self::Cmyk(luts) = self else { return None };
        let [cmy, k] = &**luts;
        Some(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    let rgb = [p[0], p[1], p[2]];
                    let [c, m, y] = cmy.apply8(rgb);
                    [c, m, y, k.apply8(rgb)[0]]
                })
                .collect(),
        )
    }
}

/// How CMYK images' ink amounts are converted to the destination: a lookup table to its RGB
/// values, or to its CMYK ones (another profile).
pub(crate) struct CmykPixels {
    lut: CmykLut,
    /// The destination is CMYK.
    cmyk: bool,
}

impl CmykPixels {
    /// Ink amounts `inks` (four bytes a pixel) converted → (the samples, their colour space).
    pub(crate) fn convert(&self, inks: &[u8]) -> (Vec<u8>, ImageColorspace) {
        let n = if self.cmyk { 4 } else { 3 };
        let mut out = Vec::with_capacity(inks.len() / 4 * n);
        // Runs of one colour convert once.
        let mut last: Option<([u8; 4], [u8; 4])> = None;
        for p in inks.as_chunks::<4>().0 {
            let v = match last {
                Some((k, v)) if k == *p => v,
                _ => self.lut.apply8(*p),
            };
            last = Some((*p, v));
            out.extend_from_slice(v.get(..n).unwrap_or_default());
        }
        (out, if self.cmyk { ImageColorspace::Cmyk } else { ImageColorspace::Rgb })
    }
}
