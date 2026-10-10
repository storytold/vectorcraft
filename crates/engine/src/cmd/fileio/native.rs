//! The native format's options: compression, the version to write (older apps read older
//! versions), an embedded preview, and the save options native and `.ai` files share: each
//! artboard to a separate file, Include Linked Files, Embed ICC Profiles and Create
//! PDF-Compatible File.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_color::cms;
use vectorcraft_doc::Document;
use vectorcraft_format::{PREVIEW_MAX, SaveOptions};

use super::super::*;
use super::{Format, FormatOption};

/// The options `document.save` and the native encoder read.
pub const OPTIONS: &[FormatOption] = &[
    FormatOption {
        name: "compress",
        ty: "boolean",
        default: "false",
        description: "gzip the file: smaller, not readable as text (saves default to Preferences → File Handling → Use Compression)",
    },
    FormatOption {
        name: "version",
        ty: "integer",
        default: "4",
        description: "the format version to write: 4 (current), or 3, 2 or 1 for older VectorCraft versions (1 and 2 are never compressed; newer features they don't know are lost there)",
    },
    FormatOption {
        name: "preview",
        ty: "boolean",
        default: "false",
        description: "embed a PNG preview of the first artboard (at most 256 pixels on its longer side) for file browsers",
    },
    SEPARATE_ARTBOARDS,
    SEPARATE_RANGE,
    INCLUDE_LINKED,
    EMBED_PROFILES,
    FormatOption {
        name: "pdfCompatible",
        ty: "boolean",
        default: "false",
        description: "also carry a PDF of every artboard, for apps that read PDF (a larger file)",
    },
];

const SEPARATE_ARTBOARDS: FormatOption = FormatOption {
    name: "separateArtboards",
    ty: "boolean",
    default: "false",
    description: "also save each artboard (of range) to a file of its own beside this one, <name>-<artboard>.<ext>, holding that artboard and the art on it",
};
const SEPARATE_RANGE: FormatOption = FormatOption {
    name: "range",
    ty: "string",
    default: "null",
    description: "with separateArtboards: the artboards saved separately, 1-based such as \"1-3, 5\", or \"all\" (default)",
};
const INCLUDE_LINKED: FormatOption = FormatOption {
    name: "includeLinked",
    ty: "boolean",
    default: "false",
    description: "keep the linked files' own pixels in the file (they stay linked), not just a preview: the document shows them in full where the files can't be found",
};
const EMBED_PROFILES: FormatOption = FormatOption {
    name: "embedProfiles",
    ty: "boolean",
    default: "true",
    description: "carry the ICC profiles the document is tagged with that were loaded from files (Edit → Assign Profile), so the document keeps them where they aren't installed",
};

/// The save and export options of a `.ai` file (a PDF carrying the native document; its PDF
/// options come on top, see document.exportPdf). Saves always retain every artboard.
pub const AI_OPTIONS: &[FormatOption] = &[
    super::ARTBOARD,
    super::ARTBOARDS,
    super::USE_ARTBOARDS,
    FormatOption {
        name: "range",
        ty: "string",
        default: "null",
        description: "1-based artboards such as \"1-3, 5\", or \"all\": the PDF pages to export, or with separateArtboards the files to save separately",
    },
    SEPARATE_ARTBOARDS,
    INCLUDE_LINKED,
    EMBED_PROFILES,
    FormatOption {
        name: "pdfCompatible",
        ty: "boolean",
        default: "true",
        description: "write the artwork as PDF pages that other apps show; false keeps only the native document (smaller, faster; other apps show empty pages)",
    },
    FormatOption {
        name: "compress",
        ty: "boolean",
        default: "true",
        description: "compress the PDF's content and the native document it carries (the PDF option compression.compressText)",
    },
];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct NativeOptions {
    compress: Option<bool>,
    version: Option<u32>,
    preview: bool,
    include_linked: bool,
    embed_profiles: Option<bool>,
    pdf_compatible: bool,
}

/// What [`save_options`] leaves to [`encode`] (rendering and encoding take time).
pub(super) struct Extras {
    preview: bool,
    pdf: bool,
}

/// The options in `p` for saving `doc`, checked, with the profiles to embed (the preview and PDF
/// are left to [`encode`]).
pub(super) fn save_options(cmd: &str, f: &Format, doc: &Document, p: &Value) -> Result<(SaveOptions, Extras)> {
    let o: NativeOptions = super::encode::options(f, p)?;
    let mut so = SaveOptions::for_doc(doc);
    so.compress = o.compress.unwrap_or(false);
    so.version = o.version.unwrap_or(vectorcraft_format::VERSION);
    so.include_linked = o.include_linked;
    if o.embed_profiles.unwrap_or(true) {
        so.profiles = embedded_profiles(doc);
    }
    so.check().map_err(|e| bad(cmd, e.to_string()))?;
    Ok((so, Extras { preview: o.preview, pdf: o.pdf_compatible }))
}

/// The `.vectorcraft` file of `doc` with the options in `p`.
pub(super) fn encode(cmd: &str, f: &Format, doc: &Document, p: &Value) -> Result<Vec<u8>> {
    let (mut so, extras) = save_options(cmd, f, doc, p)?;
    // Include Linked Files keeps placed documents' files; older versions get their art.
    let full = if so.include_linked || so.version < vectorcraft_format::VERSION {
        crate::cmd::place::document::full_documents(doc).0
    } else {
        std::borrow::Cow::Borrowed(doc)
    };
    if extras.preview {
        so.preview = preview_png(&full)?;
    }
    if extras.pdf {
        // Every artboard as PDF pages (no editing data: the file is the native document).
        so.pdf = Some(super::pdf::encode(cmd, &full, &json!({ "range": "all", "preserveEditing": false }))?.0);
    }
    let native = match full {
        Cow::Owned(full) if so.include_linked && so.version >= vectorcraft_format::VERSION => {
            // Include available bytes without replacing missing, relinkable placed objects.
            let mut original = doc.clone();
            original.images = full.images;
            Cow::Owned(original)
        }
        other => other,
    };
    vectorcraft_format::save_with(&native, &so).map_err(|e| bad(cmd, e.to_string()))
}

/// The ICC files of the profiles `doc` is tagged with that were loaded from files (the built-in
/// ones are there wherever VectorCraft runs).
fn embedded_profiles(doc: &Document) -> BTreeMap<String, Vec<u8>> {
    [&doc.color_profiles.rgb, &doc.color_profiles.cmyk]
        .into_iter()
        .flatten()
        .filter_map(|name| cms::profile(name).filter(|p| !p.builtin))
        .filter_map(|p| Some((p.name.clone(), cms::icc_bytes(&p.name).ok()?.to_vec())))
        .collect()
}

/// Most profiles read from one file.
const MAX_PROFILES: usize = 8;

/// Make the profiles a file carries available (those not installed here), named as in the file →
/// a warning per profile that can't be used.
pub(super) fn install_profiles(profiles: BTreeMap<String, Vec<u8>>) -> Vec<String> {
    // A document is tagged with an RGB and a CMYK profile at most: more is junk.
    profiles
        .into_iter()
        .take(MAX_PROFILES)
        .filter(|(name, _)| cms::profile(name).is_none())
        .filter_map(|(name, icc)| {
            cms::register_icc(&icc, Some(name.clone())).err().map(|e| format!("the colour profile “{name}” the file carries can't be used: {e}"))
        })
        .collect()
}

/// Check AI options even when chosen PDF pages omit the native editing attachment.
pub(super) fn ai_options(cmd: &str, f: &Format, doc: &Document, p: &Value) -> Result<SaveOptions> {
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        // AI carries the current native format without a separate preview. Its compression
        // and PDF-content flags are still validated as booleans.
        o.retain(|k, _| !matches!(k.as_str(), "version" | "preview"));
        if o.get("pdfCompatible").is_some_and(Value::is_null) {
            o.remove("pdfCompatible");
        }
    }
    let (mut so, _) = save_options(cmd, f, doc, &q)?;
    so.pretty = false;
    so.compress = false; // The PDF compresses its editing attachment.
    Ok(so)
}

/// The original native document carried by an AI file, with validated save options.
pub(super) fn ai_native(cmd: &str, doc: &Document, so: &SaveOptions) -> Result<Vec<u8>> {
    let native = if so.include_linked {
        match crate::cmd::place::document::full_documents(doc).0 {
            Cow::Borrowed(d) => Cow::Borrowed(d),
            Cow::Owned(full) => {
                // Include available files, retaining placed objects even when their files
                // are missing. Only drawn output substitutes unlinked preview images.
                let mut original = doc.clone();
                original.images = full.images;
                Cow::Owned(original)
            }
        }
    } else {
        Cow::Borrowed(doc)
    };
    vectorcraft_format::save_with(&native, so).map_err(|e| bad(cmd, e.to_string()))
}

/// The first artboard (else the art) as a PNG fitted into [`PREVIEW_MAX`] pixels (`None`: nothing
/// to show).
pub(crate) fn preview_png(doc: &Document) -> Result<Option<Vec<u8>>> {
    let Some(r) = doc.artboards.first().map(|a| a.rect).or_else(|| vectorcraft_render::encode::art_bounds(doc)) else { return Ok(None) };
    let scale = f64::from(PREVIEW_MAX) / r.width().max(r.height());
    if vectorcraft_render::raster_size(r, scale).is_err() {
        return Ok(None);
    }
    vectorcraft_render::Renderer::new().render_region(doc, r, scale, false).to_png().map(Some).map_err(EngineError::Other)
}

/// `p` with `compress` from the Use Compression preference when it doesn't say (saves only;
/// exports and serializing write what they're told).
pub fn with_compression_pref(prefs: &crate::Prefs, p: &Value) -> Value {
    let mut q = if p.is_object() { p.clone() } else { Value::Object(Default::default()) };
    if let Some(o) = q.as_object_mut()
        && o.get("compress").is_none_or(Value::is_null)
        // Older versions can't read compressed files: those saves stay plain.
        && o.get("version").and_then(Value::as_u64).is_none_or(|v| v >= u64::from(vectorcraft_format::COMPRESSED_SINCE))
    {
        o.insert("compress".into(), Value::Bool(prefs.use_compression));
    }
    q
}
