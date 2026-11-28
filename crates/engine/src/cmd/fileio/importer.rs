//! `document.open` and `file.place` of the files importer plug-ins read
//! ([`vectorcraft_plugins::import`]).
//!
//! A file opens with an importer when `importer` names one (its format id, `plugin.<id>`), which
//! must list the file's extension, or when no built-in format reads the file, by its content or by
//! its extension, and an importer lists its extension (the one whose id sorts first, when several
//! do). A built-in format that can't read a damaged file reports its own error: the file is not
//! handed to an importer instead. File › Place uses an importer by the same rules, among the
//! importers whose manifests set `place` ([`named_for_place`], [`for_place`]).
//!
//! The importer is given the file's bytes, at most [`import::MAX_INPUT_BYTES`] of them: a file
//! named by `path` that only an importer can read is read no further than that. Its objects become
//! a new document whose artboard is their bounds (or, to place, the art of one group), and nothing
//! opens or is placed when the importer fails, declines, is cancelled or returns objects that don't
//! pass the checks a filter's output does.
//! An importer at plug-in API level 4 may also return point type, embedded images, top-level
//! layers and artboards (its manifest's `outputKinds`): the artboards it returns replace the one
//! around the art of a new document, and File › Place leaves them out.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::{CharStyle, Document, ImageObject, LAYER_COLORS, LayerColor, MAX_COORD, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_plugins::import::{ImportObject, Mode};
use vectorcraft_plugins::{Manifest, Plugin, export, import, objects};

use super::super::{Result, bad, str_param};
use super::load::{Loaded, Source, detect, file_name};
use super::{Format, err, extension, file_stem, format};
use crate::EngineError;

const C: &str = "document.open";

/// The importer command `cmd`'s params `p` name (`importer`), if any; a value that isn't a string
/// is an error.
fn named<'a>(p: &'a Value, cmd: &str) -> Result<Option<&'a str>> {
    match p.get("importer").filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| bad(cmd, "`importer` must be a plug-in format id (see document.formats)")),
    }
}

/// Checks that importer `m` lists the extension of the file `name`: else command `cmd`'s error,
/// which names the extensions it lists.
fn check_extension(cmd: &str, m: &Manifest, name: &str) -> Result<()> {
    if m.extensions.contains(&extension(name)) {
        return Ok(());
    }
    let shown = vectorcraft_plugins::permissions::display_text(&m.name, vectorcraft_plugins::permissions::MAX_SHOWN_NAME);
    let exts: Vec<String> = m.extensions.iter().map(|e| format!(".{e}")).collect();
    Err(bad(cmd, format!("“{shown}” opens {} files, not `{}`", exts.join(", "), file_name(name))))
}

/// The importer that opens the file `name` holding `bytes` with `document.open`'s params `p`: the
/// one `importer` names, which must list the file's extension; else, when no built-in format reads
/// the file, the first installed importer that lists its extension; else `None`.
pub(super) fn pick(name: &str, bytes: &[u8], p: &Value) -> Result<Option<Arc<Plugin>>> {
    if let Some(id) = named(p, C)? {
        let plugin = import::named(id).map_err(|e| bad(C, e.to_string()))?;
        check_extension(C, plugin.manifest(), name)?;
        return Ok(Some(plugin));
    }
    if detect(name, bytes).is_some() {
        return Ok(None);
    }
    Ok(import::for_extension(&extension(name)).into_iter().next())
}

/// The importer `file.place`'s params `p` name (`importer`) for the file `name`, which must set
/// `place` in its manifest and list the file's extension (errors of command `cmd`); `None` when `p`
/// names none.
pub(crate) fn named_for_place(name: &str, p: &Value, cmd: &str) -> Result<Option<Arc<Plugin>>> {
    let Some(id) = named(p, cmd)? else { return Ok(None) };
    let plugin = import::named(id).map_err(|e| bad(cmd, e.to_string()))?;
    let m = plugin.manifest();
    if !m.place {
        return Err(match import::not_placing(m) {
            vectorcraft_plugins::Error::Params(why) => bad(cmd, why),
            e => bad(cmd, e.to_string()),
        });
    }
    check_extension(cmd, m, name)?;
    Ok(Some(plugin))
}

/// The importer File › Place uses for the file `name` when no built-in format reads it: the first
/// installed importer, by plug-in id, whose manifest sets `place` and that lists its extension.
pub(crate) fn for_place(name: &str) -> Option<Arc<Plugin>> {
    import::placing(&extension(name)).into_iter().next()
}

/// Whether an importer may read a file of extension `ext` for `mode` when none is named: no
/// built-in format reads the extension (for Place, as text, RTF or Word either) and an importer
/// that reads for `mode` lists it.
fn importer_reads(ext: &str, mode: Mode) -> bool {
    super::readable_format(ext).is_none()
        && match mode {
            Mode::Import => !import::for_extension(ext).is_empty(),
            Mode::Place => !super::PLACE_EXTS.contains(&ext) && !import::placing(ext).is_empty(),
        }
}

/// The error of the file `name`, past [`import::MAX_INPUT_BYTES`], which an importer would read
/// for `mode`.
fn too_large(name: &str, mode: Mode) -> EngineError {
    match mode {
        Mode::Import => err(format!("can't open `{}`: {}", file_name(name), import::too_large())),
        Mode::Place => failed(Mode::Place, name, import::too_large()),
    }
}

/// The error of the file `name`, which importer `e` couldn't read for `mode`.
fn failed(mode: Mode, name: &str, e: impl std::fmt::Display) -> EngineError {
    match mode {
        Mode::Import => err(format!("can't open `{}`: {e}", file_name(name))),
        Mode::Place => err(format!("can't place `{}`: {e}", file_name(name))),
    }
}

/// How much of a file [`detect`] is given: more than its content checks read.
const SNIFF_BYTES: usize = 64 << 10;

/// Refuses `document.open`'s `dataBase64` in `p` before it is decoded when it holds a file past
/// [`import::MAX_INPUT_BYTES`] that only an importer reads, by the rule of `read_path`.
pub(super) fn check_data(p: &Value) -> Result<()> {
    check_data_for(p, Mode::Import, C)
}

/// [`check_data`] for `mode` and command `cmd`: `file.place`'s params with [`Mode::Place`].
fn check_data_for(p: &Value, mode: Mode, cmd: &str) -> Result<()> {
    let Some(b64) = str_param(p, "dataBase64") else {
        return Ok(());
    };
    let name = str_param(p, "name").unwrap_or("Untitled");
    let explicit = named(p, cmd)?.is_some();
    if (!explicit && !importer_reads(&extension(name), mode)) || vectorcraft_format::base64_decoded_len(b64) <= import::MAX_INPUT_BYTES as u64 {
        return Ok(());
    }
    if !explicit {
        let start: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).take(SNIFF_BYTES / 3 * 4).collect();
        let head = std::str::from_utf8(&start).ok().and_then(vectorcraft_format::base64_decode);
        if head.is_some_and(|head| detect(name, &head).is_some()) {
            return Ok(());
        }
    }
    Err(too_large(name, mode))
}

/// The file at `path` for `document.open`'s params `p`. A file that only an importer reads (one
/// `importer` names, or an extension no built-in format reads that an importer lists) is read up
/// to [`import::MAX_INPUT_BYTES`]: past that, it is an error, unless the start of the file shows a
/// built-in format and no importer was named, which is read whole as any other file is. A file past
/// the limit that isn't read whole is reported from its size: with `importer` named, before any of
/// it is read, and otherwise after the start of it is read to look for a built-in format.
pub(super) fn read_path(path: &str, p: &Value) -> Result<Vec<u8>> {
    read_path_for(path, p, Mode::Import, C)
}

/// [`read_path`] for `mode` and command `cmd`: `file.place`'s params with [`Mode::Place`].
#[cfg(not(target_arch = "wasm32"))]
fn read_path_for(path: &str, p: &Value, mode: Mode, cmd: &str) -> Result<Vec<u8>> {
    let explicit = named(p, cmd)?.is_some();
    if explicit {
        return read_for_importer(path, mode);
    }
    if !importer_reads(&extension(path), mode) {
        return super::read_file(path);
    }
    let head = super::read_file_head(path, SNIFF_BYTES)?;
    if head.len() < SNIFF_BYTES {
        return Ok(head);
    }
    if detect(path, &head).is_some() {
        return super::read_file(path);
    }
    read_for_importer(path, mode)
}

/// The file at `path` for an importer: refused from its size, before any of it is read, when it is
/// past [`import::MAX_INPUT_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
fn read_for_importer(path: &str, mode: Mode) -> Result<Vec<u8>> {
    super::may_read(path)?;
    let meta = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if meta.is_file() && meta.len() > import::MAX_INPUT_BYTES as u64 {
        return Err(too_large(path, mode));
    }
    // The file can grow after the size is read; anything but a regular file is refused here.
    let file = super::read_file_head(path, import::MAX_INPUT_BYTES + 1)?;
    if file.len() > import::MAX_INPUT_BYTES {
        return Err(too_large(path, mode));
    }
    Ok(file)
}

#[cfg(target_arch = "wasm32")]
fn read_path_for(path: &str, _: &Value, _: Mode, _: &str) -> Result<Vec<u8>> {
    super::read_file(path)
}

/// The file `file.place`'s params `p` name, as [`super::source`] reads it for command `cmd`, except
/// that a file an importer would place (one `importer` names, or an extension no built-in format
/// reads that an importer which places its files lists) is read, or decoded, only when it is at
/// most [`import::MAX_INPUT_BYTES`] long.
pub(crate) fn place_source<'a>(p: &'a Value, cmd: &str) -> Result<Source<'a>> {
    match str_param(p, "path") {
        Some(path) => Ok(Source { name: path, bytes: read_path_for(path, p, Mode::Place, cmd)?, path: Some(path) }),
        None => {
            check_data_for(p, Mode::Place, cmd)?;
            super::source(p, cmd)
        }
    }
}

/// What [`read`] gives: the importer's format, the new document, its objects' ids, the artboards
/// the importer returned and its warnings.
type Read = (&'static Format, Document, Vec<NodeId>, Vec<import::OutArtboard>, Vec<String>);

/// The file `name` holding `bytes` read by importer `plugin` for `mode` with command `cmd`'s params
/// `p` (`params`: the importer's own parameters) → the importer's format, a new document holding its
/// objects (on its first layer, or in the layers the importer returned), their ids, the artboards
/// the importer returned, and its warnings.
fn read(name: &str, bytes: &[u8], plugin: &Plugin, p: &Value, cmd: &str, mode: Mode) -> Result<Read> {
    let m = plugin.manifest();
    let format = format(&export::format_id(&m.id)).filter(|f| f.read).ok_or_else(|| err(vectorcraft_plugins::Error::NotFound(m.id.clone())))?;
    let (ext, stem) = (extension(name), file_stem(name));
    let source = import::Source { extension: &ext, name: &stem };
    let params = p.get("params").cloned().unwrap_or(Value::Null);
    let imported = import::run_as(plugin, bytes, &params, source, mode).map_err(|e| match e {
        vectorcraft_plugins::Error::Cancelled => EngineError::Cancelled,
        vectorcraft_plugins::Error::Params(why) => bad(cmd, why),
        e => failed(mode, name, e),
    })?;
    let mut doc = Document::new(1.0, 1.0);
    let layer = doc.layers.first().map(|l| l.id);
    let admit = |fx: &[vectorcraft_doc::Effect], n: &vectorcraft_doc::Node| crate::cmd::plugin::admit_effects_for(cmd, fx, n);
    let mut warnings = Vec::new();
    let (ids, artboards) = if imported.paths_only() {
        let paths = imported.objects.into_iter().filter_map(|o| if let ImportObject::Path(o) = o { Some(o) } else { None }).collect();
        (objects::apply_output(&mut doc, &[], paths, layer, &admit).map_err(|e| failed(mode, name, e))?, Vec::new())
    } else {
        let ids = build(&mut doc, imported.objects, imported.layers, &admit, &mut warnings).map_err(|e| failed(mode, name, e))?;
        (ids, imported.artboards)
    };
    warnings.splice(0..0, imported.warnings);
    Ok((format, doc, ids, artboards, warnings))
}

/// The file `name` holding `bytes` read by importer `plugin` to place it, with `file.place`'s params
/// `p` (errors of command `cmd`) → the importer's format, the document holding its objects in its
/// layers, and its warnings; artboards the importer returned are left out. Importer `plugin`'s
/// manifest must set `place`.
pub(crate) fn read_for_place(name: &str, bytes: &[u8], plugin: &Plugin, p: &Value, cmd: &str) -> Result<(&'static Format, Document, Vec<String>)> {
    let (format, doc, _, _, warnings) = read(name, bytes, plugin, p, cmd, Mode::Place)?;
    Ok((format, doc, warnings))
}

/// The file `name` holding `bytes` read by importer `plugin` with `document.open`'s params `p`
/// (`params`: the importer's own parameters) into a new document, its artboard the bounds of what
/// the importer returned.
pub(super) fn load(name: &str, bytes: &[u8], plugin: &Plugin, p: &Value) -> Result<Loaded> {
    let (format, mut doc, ids, artboards, warnings) = read(name, bytes, plugin, p, C, Mode::Import)?;
    if artboards.is_empty() {
        if let (Some(bounds), Some(board)) = (doc.bounds_of(&ids, true), doc.artboards.first_mut()) {
            board.rect = artboard_rect(bounds);
        }
    } else {
        set_artboards(&mut doc, artboards);
    }
    doc.title = file_name(name);
    Ok(Loaded { doc, format, warnings, restored: false, converted: false, preview_only: false, editing_data: false, native_pdf_compatible: None })
}

/// Puts `objects`, what an importer at plug-in API level 4 returned, into `doc`, a new document of
/// one layer: in `layers`, bottom first, each holding the objects it lists in their order, or all in
/// that one layer when there are none. Paths are made as a filter's new ones are (their effects let
/// in by `admit`), type is laid out, and each image is stored in the document as a PNG of its
/// pixels. A warning for type whose font isn't there, or doesn't have its style, goes to `warnings`.
/// An object that reaches more than [`MAX_COORD`] from the origin is an error. Returns the objects'
/// ids in order.
fn build(
    doc: &mut Document,
    objects: Vec<ImportObject>,
    layers: Vec<import::OutLayer>,
    admit: objects::AdmitEffects,
    warnings: &mut Vec<String>,
) -> std::result::Result<Vec<NodeId>, String> {
    let count = objects.len();
    let first = doc.layers.first().map(|l| (**l).clone());
    let first = first.unwrap_or_else(|| Node::layer(doc.alloc_id(), "Layer 1", LayerColor::Preset(0)));
    let mut owner = vec![0usize; count];
    for (li, l) in layers.iter().enumerate() {
        for &k in &l.objects {
            if let Some(o) = owner.get_mut(k) {
                *o = li;
            }
        }
    }
    let mut members: Vec<Vec<Arc<Node>>> = vec![Vec::new(); layers.len().max(1)];
    let mut ids = Vec::with_capacity(count);
    let mut fonts = std::collections::BTreeSet::new();
    for (i, o) in objects.into_iter().enumerate() {
        let at = format!("objects[{i}]");
        let node = match o {
            ImportObject::Path(o) => objects::new_object(doc, o, &at, admit).map_err(|e| e.to_string())?,
            ImportObject::Type(t) => {
                let node = type_node(doc, t);
                if let NodeKind::Text(t) = &node.kind {
                    fonts.extend(t.runs.iter().flat_map(crate::cmd::fonts::run_fonts));
                }
                node
            }
            ImportObject::Image(im) => image_node(doc, im).map_err(|e| e.to_string())?,
        };
        if node.geometric_bounds().is_some_and(|b| ![b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)) {
            return Err(format!("plug-in output: {at}: it reaches more than {MAX_COORD} pt from the origin"));
        }
        ids.push(node.id);
        if let Some(list) = owner.get(i).and_then(|&li| members.get_mut(li)) {
            list.push(Arc::new(node));
        }
    }
    let built: Vec<Arc<Node>> = if layers.is_empty() {
        let mut layer = first;
        if let Some(children) = layer.children_mut() {
            *children = members.into_iter().next().unwrap_or_default();
        }
        vec![Arc::new(layer)]
    } else {
        layers
            .into_iter()
            .zip(members)
            .enumerate()
            .map(|(li, (l, children))| {
                let mut layer = Node::layer(doc.alloc_id(), &l.name, LayerColor::Preset((li % LAYER_COLORS.len()) as u8));
                layer.visible = l.visible;
                layer.locked = l.locked;
                if let Some(c) = layer.children_mut() {
                    *c = children;
                }
                Arc::new(layer)
            })
            .collect()
    };
    doc.layers = built;
    warnings.extend(crate::cmd::fonts::set_font_warning(fonts));
    Ok(ids)
}

/// Point type `t` as a new node of `doc`, in [Normal Character Style] with the face, size and fill
/// `t` gives, laid out.
fn type_node(doc: &mut Document, t: import::OutType) -> Node {
    let mut style = crate::cmd::textstyles::normal_char_style(doc);
    style.font_version = None;
    style.font_axes.clear();
    style.font_family = t.font_family;
    match t.font_style {
        Some(s) => style.font_style = s,
        None => {
            style.font_style = CharStyle::default().font_style;
            crate::cmd::textedit::fit_font_style(&mut style);
        }
    }
    style.size = t.font_size;
    if let Some(fill) = t.fill {
        style.fill = fill;
    }
    let mut text = TextObject::point(Point::ZERO, &t.text, style);
    text.xf = t.transform;
    text.para = crate::cmd::create::new_type_para();
    text.system_layout = doc.setup.system_layout;
    crate::cmd::typecmd::refresh_bounds(&mut text);
    let mut n = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(text)));
    n.name = t.name;
    n
}

/// Image `im` as a new node of `doc`, its pixels stored in `doc` as a PNG.
fn image_node(doc: &mut Document, im: import::OutImage) -> Result<Node> {
    let pixels = image::RgbaImage::from_raw(im.width, im.height, im.rgba).ok_or_else(|| err("the image's pixels don't match its size"))?;
    let stored = super::load::as_png(pixels, None, None)?;
    let image = ImageObject {
        key: stored.key.clone(),
        width: stored.width,
        height: stored.height,
        xf: im.transform,
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    doc.images.insert(stored.key, stored.blob);
    let mut n = Node::new(doc.alloc_id(), NodeKind::Image(Box::new(image)));
    n.name = im.name;
    Ok(n)
}

/// Gives `doc` the artboards `boards` an importer returned, in order, in place of its one.
fn set_artboards(doc: &mut Document, boards: Vec<import::OutArtboard>) {
    let Some(template) = doc.artboards.first().cloned() else { return };
    doc.artboards =
        boards.into_iter().zip(1u32..).map(|(b, id)| vectorcraft_doc::Artboard { id, name: b.name, rect: b.rect, ..template.clone() }).collect();
}

/// The artboard of art with bounds `r`: `r` within ±[`MAX_COORD`], at least 1 pt wide and high
/// about its center (the artboard of a single line or point). A side that would pass the edge
/// moves inward, so art at the edge keeps an artboard the document's edits accept.
fn artboard_rect(r: Rect) -> Rect {
    let side = |lo: f64, hi: f64| {
        let (lo, hi) = (lo.max(-MAX_COORD), hi.min(MAX_COORD));
        if hi - lo >= 1.0 {
            return (lo, hi);
        }
        let mid = ((lo + hi) / 2.0).clamp(-MAX_COORD + 0.5, MAX_COORD - 0.5);
        (mid - 0.5, mid + 0.5)
    };
    let ((x0, x1), (y0, y1)) = (side(r.x0, r.x1), side(r.y0, r.y1));
    Rect::new(x0, y0, x1, y1)
}
