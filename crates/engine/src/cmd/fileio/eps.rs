//! EPS export for every EPS path (`document.export {format: eps}`, `document.exportEps`, Export As
//! → EPS Options): the options parsed from the params, transparency flattened with a flattener
//! preset, one file of the art's bounds or one per artboard, the preview and thumbnail rendered,
//! and the native document embedded for reopening.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_eps::{EpsOptions, Level, Overprint, Preview, Raster};
use vectorcraft_geom::{Point, Rect};

use super::super::flatten::{FlattenOptions, flatten_document};
use super::super::*;
use super::{ArtboardPick, Encoded, FormatOption};

const C: &str = "document.export";

/// Most pixels along a side of a preview.
const MAX_PREVIEW: f64 = 2048.0;
/// Most pixels along a side of a thumbnail.
const THUMBNAIL: f64 = 128.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "document.exportEps",
        "Export EPS",
        [],
        None,
        "{path?, level?: 2|3 (default; PostScript language level: 3 writes smooth gradients and masks transparent image pixels out, 2 steps gradients and writes those pixels white), previewFormat?: none|tiffBw|tiffColor (default: a TIFF preview other apps show), transparentPreview?: true (else on white), overprints?: preserve (default)|discard, flattenerPreset?: \"medium\" (default; high, low or a saved preset: flattener.presets.list), flattener?: {the options of object.flattenTransparency over the preset}, embedFonts?: true (type is always written as outlines, so no fonts are needed), includeLinkedFiles?: false (the embedded document keeps linked images whole, not just their previews), thumbnails?: true (a PNG thumbnail in the file), cmykPostScript?: true (RGB documents written in CMYK), compatibleGradients?: false (stepped gradients at level 3 too), selectedOnly?: false, useArtboards?: true (one file per chosen artboard, artboards?/range?, {stem}_{artboard}.eps, the bounding box the artboard) | false (default: one file, the bounding box the visible art, y up from the first artboard's bottom-left corner)} → {path, bytes, warnings, files?}; no path → {dataBase64, …}. Transparency is flattened with the preset (EPS has none); the native document rides along in comments, for reopening the file as it was. Same as document.export {format: eps}",
        has_doc,
        export_eps
    )]
}

/// The EPS options `document.formats` lists.
pub(super) const OPTIONS: &[FormatOption] = &[
    super::ARTBOARD,
    super::ARTBOARDS,
    super::RANGE,
    FormatOption {
        name: "useArtboards",
        ty: "boolean",
        default: "null",
        description: "true: one file per chosen artboard (default all), <file>_<artboard>.eps; false or absent: one file of the visible art (naming artboards also uses them)",
    },
    FormatOption { name: "level", ty: "integer", default: "3", description: "PostScript language level, 2 or 3" },
    FormatOption {
        name: "previewFormat",
        ty: "string",
        default: "\"tiffColor\"",
        description: "none, tiffBw (1 bit) or tiffColor: a TIFF preview other apps show",
    },
    FormatOption {
        name: "transparentPreview",
        ty: "boolean",
        default: "true",
        description: "the colour preview keeps transparency (false: on white)",
    },
    FormatOption { name: "overprints", ty: "string", default: "\"preserve\"", description: "preserve or discard overprinting fills and strokes" },
    FormatOption {
        name: "flattenerPreset",
        ty: "string",
        default: "\"medium\"",
        description: "the flattener preset transparency is flattened with: high, medium, low or a saved one",
    },
    FormatOption {
        name: "flattener",
        ty: "object",
        default: "null",
        description: "flattener options over the preset (as object.flattenTransparency)",
    },
    FormatOption {
        name: "embedFonts",
        ty: "boolean",
        default: "true",
        description: "type is written as outlines, so the file needs no fonts either way",
    },
    FormatOption {
        name: "includeLinkedFiles",
        ty: "boolean",
        default: "false",
        description: "the embedded document keeps linked images whole (else their previews)",
    },
    FormatOption { name: "thumbnails", ty: "boolean", default: "true", description: "a small PNG thumbnail in the file" },
    FormatOption {
        name: "cmykPostScript",
        ty: "boolean",
        default: "true",
        description: "RGB documents are written in CMYK, for devices without RGB",
    },
    FormatOption { name: "compatibleGradients", ty: "boolean", default: "false", description: "gradients as stepped fills at level 3 too" },
    FormatOption { name: "selectedOnly", ty: "boolean", default: "false", description: "only the selected objects, in their layers" },
];

/// The EPS params, as they come (each checked when read).
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Params {
    #[serde(flatten)]
    boards: ArtboardPick,
    use_artboards: Option<bool>,
    level: Option<Value>,
    preview_format: Option<String>,
    transparent_preview: Option<bool>,
    overprints: Option<String>,
    flattener_preset: Option<String>,
    flattener: Option<Value>,
    include_linked_files: Option<bool>,
    thumbnails: Option<bool>,
    cmyk_post_script: Option<bool>,
    compatible_gradients: Option<bool>,
}

/// The settings of one EPS export.
struct Settings {
    /// The writer's options (the region, origin and native document are set per file).
    eps: EpsOptions,
    flatten: FlattenOptions,
    include_linked: bool,
    thumbnails: bool,
    boards: ArtboardPick,
    use_artboards: Option<bool>,
}

/// An option read by `read`, or an error listing what it may be.
fn choice<T>(field: &str, v: Option<String>, read: impl Fn(&str) -> Option<T>, default: T, allowed: &str) -> Result<T> {
    match v {
        None => Ok(default),
        Some(s) => read(&s).ok_or_else(|| bad(C, format!("EPS {field} `{s}`: {allowed}"))),
    }
}

/// The flattener options `p` asks for (`flattenerPreset` and `flattener` over it), built-in
/// presets and `saved` ones; discarded overprints are dropped from the flattened art too.
fn flattener(p: &Params, saved: &[super::super::FlattenerPreset]) -> Result<FlattenOptions> {
    FlattenOptions::for_export(p.flattener_preset.as_deref(), p.flattener.as_ref(), saved).map_err(|e| bad(C, format!("EPS flattener: {e}")))
}

fn params(p: &Value) -> Result<Params> {
    if p.is_object() { Params::deserialize(p).map_err(|e| bad(C, format!("EPS options: {e}"))) } else { Ok(Params::default()) }
}

fn parse(p: &Value) -> Result<Settings> {
    let q = params(p)?;
    let d = EpsOptions::default();
    let overprint = choice("overprints", q.overprints.clone(), Overprint::from_id, d.overprint, "preserve or discard")?;
    let mut flatten = flattener(&q, &[])?;
    flatten.preserve_overprints &= overprint == Overprint::Preserve;
    let level = q.level.as_ref().map(super::dxf::text);
    let eps = EpsOptions {
        level: choice("level", level, Level::from_id, d.level, "2 or 3")?,
        preview: choice("previewFormat", q.preview_format.clone(), Preview::from_id, d.preview, "none, tiffBw or tiffColor")?,
        transparent_preview: q.transparent_preview.unwrap_or(true),
        overprint,
        cmyk: q.cmyk_post_script.unwrap_or(true),
        compatible_gradients: q.compatible_gradients.unwrap_or(false),
        ..d
    };
    Ok(Settings {
        eps,
        flatten,
        include_linked: q.include_linked_files.unwrap_or(false),
        thumbnails: q.thumbnails.unwrap_or(true),
        boards: q.boards,
        use_artboards: q.use_artboards,
    })
}

/// `p` checked (dialogs keep themselves open on an error) and with a saved flattener preset of
/// `saved` it names written out, as the encoder needs it.
pub fn resolve(p: &Value, saved: &[super::super::FlattenerPreset]) -> std::result::Result<Value, String> {
    let run = || -> Result<Value> {
        let q = expanded(C, p, saved)?.unwrap_or_else(|| p.clone());
        parse(&q)?;
        Ok(q)
    };
    run().map_err(|e| e.to_string())
}

/// `p` with a saved flattener preset it names written out as `flattener` options, for the
/// encoder, which knows only the built-in presets; `None` when `p` names none.
fn expanded(cmd: &str, p: &Value, saved: &[super::super::FlattenerPreset]) -> Result<Option<Value>> {
    match str_param(p, "flattenerPreset") {
        Some(name) if !name.trim().is_empty() && FlattenOptions::preset(name).is_none() => {
            let o = FlattenOptions::for_export(Some(name), p.get("flattener"), saved).map_err(|e| bad(cmd, format!("flattener: {e}")))?;
            let mut q = p.clone();
            q["flattener"] = serde_json::to_value(o).map_err(|e| EngineError::Other(e.to_string()))?;
            if let Some(m) = q.as_object_mut() {
                m.remove("flattenerPreset");
            }
            Ok(Some(q))
        }
        _ => Ok(None),
    }
}

/// The files to write: `(artboard, region, origin)` each — every chosen artboard with its corner
/// as the origin, or the visible art's bounds with the first artboard's corner.
fn pages(doc: &Document, set: &Settings) -> Result<Vec<(Option<usize>, Rect, Point)>> {
    let n = doc.artboards.len();
    let named = set.boards.resolve(n).map_err(|e| bad(C, e))?;
    let boards = match (set.use_artboards, named) {
        (Some(false), _) => None,
        (_, Some(b)) => Some(b),
        (Some(true), None) => Some((0..n).collect()),
        (None, None) => None,
    };
    let corner = |r: Rect| Point::new(r.x0, r.y1);
    match boards {
        Some(boards) => boards
            .into_iter()
            .map(|b| doc.artboards.get(b).map(|a| (Some(b), a.rect, corner(a.rect))).ok_or_else(|| bad(C, format!("no artboard {}", b + 1))))
            .collect(),
        None => {
            let art = vectorcraft_render::encode::art_bounds(doc).ok_or_else(|| bad(C, "nothing to export: the document has no visible art"))?;
            Ok(vec![(None, art, doc.artboards.first().map_or(corner(art), |a| corner(a.rect)))])
        }
    }
}

/// `rect` of `doc` drawn at a pixel a point (fewer when larger than `max` a side), straight RGBA,
/// transparent or on white.
fn render(r: &mut vectorcraft_render::Renderer, doc: &Document, rect: Rect, max: f64, transparent: bool) -> Result<Raster> {
    let long = rect.width().max(rect.height());
    let scale = if long > max { max / long } else { 1.0 };
    vectorcraft_render::raster_size(rect, scale).map_err(|e| bad(C, e))?;
    let img = r.render_region(doc, rect, scale, !transparent);
    Ok(Raster { width: img.width, height: img.height, rgba: img.to_straight() })
}

/// What the file carries to reopen: `doc` (with only artboard `board` when the file is one of
/// several artboards), with linked images whole when `include_linked`.
fn native(doc: &Document, board: Option<usize>, include_linked: bool) -> Vec<u8> {
    let several = board.filter(|_| doc.artboards.len() > 1);
    if several.is_none() && !include_linked {
        return vectorcraft_format::save(doc, false);
    }
    let mut d = doc.clone();
    if let Some(a) = several.and_then(|b| doc.artboards.get(b)) {
        d.artboards = vec![a.clone()];
    }
    if include_linked {
        // A blob keeps its preview only while its file hasn't been read.
        for blob in d.images.values_mut().filter(|b| !b.is_proxy()) {
            blob.proxy = None;
        }
    }
    vectorcraft_format::save(&d, false)
}

/// Encode `doc` as EPS: one file of the visible art, or with Use Artboards one per chosen
/// artboard. Transparency is flattened first (once for all of them); the previews show `doc`
/// itself.
pub(super) fn encode(doc: &Document, p: &Value) -> Result<Encoded> {
    let set = parse(p)?;
    let original = doc;
    let (full, warnings) = crate::cmd::place::document::full_documents(doc);
    let doc = &*full;
    // Include available bytes while retaining missing, relinkable objects in the attachment.
    let editing = if set.include_linked {
        let mut d = original.clone();
        d.images = doc.images.clone();
        Cow::Owned(d)
    } else {
        Cow::Borrowed(original)
    };
    let pages = pages(doc, &set)?;
    if pages.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    let transparent = doc.layers.iter().any(|l| l.shows_transparency());
    let flat = flatten_document(doc, &set.flatten)?;
    let drawn = flat.as_ref().unwrap_or(doc);
    let mut enc = Encoded { warnings, joiner: Some("_"), ..Encoded::default() };
    if transparent && flat.is_some() {
        enc.warnings.push("transparency is flattened into opaque art and images (EPS has none): see the flattener preset".into());
    }
    let created = vectorcraft_doc::metadata::now_unix();
    let mut renderer = vectorcraft_render::Renderer::new();
    for (board, region, origin) in pages {
        let rect = vectorcraft_eps::preview_rect(region, origin);
        let preview = match set.eps.preview {
            Preview::None => None,
            kind => Some(render(&mut renderer, doc, rect, MAX_PREVIEW, set.eps.transparent_preview && kind == Preview::TiffColor)?),
        };
        let thumbnail = set.thumbnails.then(|| render(&mut renderer, doc, rect, THUMBNAIL, false)).transpose()?;
        let o = EpsOptions {
            region,
            origin,
            title: doc.title.clone(),
            created,
            native: Some(native(&editing, board, set.include_linked)),
            ..set.eps.clone()
        };
        let out = vectorcraft_eps::export(drawn, &o, preview.as_ref(), thumbnail.as_ref()).map_err(|e| bad(C, e))?;
        enc.files.push((board, out.bytes));
        for w in out.warnings {
            if !enc.warnings.contains(&w) {
                enc.warnings.push(w);
            }
        }
    }
    Ok(enc)
}

fn export_eps(s: &mut Session, p: &Value) -> Result<Value> {
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    q["format"] = json!("eps");
    super::export::export(s, &q)
}

/// `p` with a saved flattener preset of the session's written out (see [`expanded`]).
pub(super) fn with_flattener<'a>(s: &Session, cmd: &str, p: Cow<'a, Value>) -> Result<Cow<'a, Value>> {
    Ok(match expanded(cmd, &p, &s.prefs.flattener_presets)? {
        Some(q) => Cow::Owned(q),
        None => p,
    })
}
