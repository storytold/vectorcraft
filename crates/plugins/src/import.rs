//! Importer plug-ins: a plug-in of kind `importer` reads files in a format of its own, such as a
//! pen plotter's or a cutting machine's, into a new document.
//!
//! File › Open opens a file with an importer only when no built-in format reads it: the built-in
//! formats are found by the file's content first and then by its extension, and an importer is
//! picked by an extension it lists. When several importers list the extension, the one whose id
//! sorts first opens it ([`for_extension`]). File › Open With and `document.open {importer}` pick an
//! importer by its format id (`plugin.<id>`), also for an extension a built-in format reads.
//!
//! An importer whose manifest sets `place` also reads its files for File › Place (`file.place`),
//! by the same rules among the importers that set it ([`placing`]): its objects become one group in
//! the open document, and it is told so in `_context.mode` ([`Mode::Place`]).
//!
//! The host hands the importer the file's bytes as they are, at most [`MAX_INPUT_BYTES`] of them,
//! and reads back the objects of the new document in the same form filters return them
//! ([`crate::objects`]): `{"objects": [...], "warnings"?: ["…"]}`, or `{"error": "…"}` to decline.
//! An importer at plug-in API level 4 whose manifest lists `outputKinds` may also return point type
//! and embedded images in `objects`, and `layers` and `artboards` ([`decode_with`]).
//! An import is one run the user asked for, so it gets an export's budget ([`crate::export::FUEL`]
//! and [`crate::export::WALL_TIME_MS`]), and Cancel stops it between two slices of the budget
//! ([`Error::Cancelled`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_geom::{Affine, Rect};

use crate::objects::{MAX_COORD, MAX_OBJECTS, OutObject, declined, out_err, printable};
use crate::{Error, Kind, Limits, OutputKind, Plugin, Result, registry};

/// The largest file an importer is given, in bytes: 128 MiB, a quarter of a plug-in's memory
/// limit, so the file and the objects made from it fit beside each other.
pub const MAX_INPUT_BYTES: usize = 128 << 20;

/// The keys an importer's output may have.
const OUTPUT_KEYS: &[&str] = &["objects", "warnings", "error"];

/// The installed importers, by plug-in id.
pub fn installed() -> Vec<Arc<Plugin>> {
    registry::list().into_iter().filter(|p| p.manifest().kind == Kind::Importer).collect()
}

/// The installed importer of format id `format_id` (`plugin.<id>`).
pub fn by_format(format_id: &str) -> Option<Arc<Plugin>> {
    registry::get(crate::export::plugin_id(format_id)?).filter(|p| p.manifest().kind == Kind::Importer)
}

/// The installed importer `importer` names: its format id (`plugin.<id>`) or its plug-in id. A
/// plug-in of another kind, and one that isn't installed, are errors that say so.
pub fn named(importer: &str) -> Result<Arc<Plugin>> {
    let id = crate::export::plugin_id(importer).unwrap_or(importer);
    let plugin = registry::get(id).ok_or_else(|| Error::NotFound(id.into()))?;
    match plugin.manifest().kind {
        Kind::Importer => Ok(plugin),
        _ => Err(not_importer(plugin.manifest())),
    }
}

/// The error of plug-in `m`, which is not an importer, asked to import.
fn not_importer(m: &crate::Manifest) -> Error {
    Error::Params(format!("`{}` is a plug-in of kind `{}`, not an importer", m.id, m.kind.name()))
}

/// The installed importers that list extension `ext` (any case, leading dot allowed), by plug-in
/// id: the first is the one File › Open uses.
pub fn for_extension(ext: &str) -> Vec<Arc<Plugin>> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    installed().into_iter().filter(|p| p.manifest().extensions.contains(&ext)).collect()
}

/// The installed importers whose manifests set `place` and that list extension `ext` (any case,
/// leading dot allowed), by plug-in id: the first is the one File › Place uses.
pub fn placing(ext: &str) -> Vec<Arc<Plugin>> {
    for_extension(ext).into_iter().filter(|p| p.manifest().place).collect()
}

/// The error of importer `m`, whose manifest doesn't set `place`, asked to place a file.
pub fn not_placing(m: &crate::Manifest) -> Error {
    let shown = crate::permissions::display_text(&m.name, crate::permissions::MAX_SHOWN_NAME);
    Error::Params(format!("“{shown}” opens files but doesn't place them: its manifest doesn't set `place`"))
}

/// What an import is for, which the importer is told as `_context.mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// File › Open: the objects become a new document (`"import"`).
    Import,
    /// File › Place: the objects become one group in the open document (`"place"`). Only an
    /// importer whose manifest sets `place` runs for it.
    Place,
}

impl Mode {
    /// Its name in `_context.mode`.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Import => "import",
            Mode::Place => "place",
        }
    }
}

/// What an importer returned.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// The objects of the new document, in paint order (bottom first): paths and compound paths
    /// checked as a filter's are, and type and images when the importer lists them in `outputKinds`.
    pub objects: Vec<ImportObject>,
    /// The top-level layers, bottom first, each holding the objects it lists (`layers`, with
    /// `outputKinds` `layer`); empty: every object goes in one layer.
    pub layers: Vec<OutLayer>,
    /// The artboards (`artboards`, with `outputKinds` `artboard`); empty: one artboard around the art.
    pub artboards: Vec<OutArtboard>,
    /// What the import left out or changed, for the user (at most
    /// [`crate::export::MAX_WARNINGS`]).
    pub warnings: Vec<String>,
}

impl Imported {
    /// Whether it holds paths and compound paths alone, as an importer below API level 4 returns.
    pub fn paths_only(&self) -> bool {
        self.layers.is_empty() && self.artboards.is_empty() && self.objects.iter().all(|o| matches!(o, ImportObject::Path(_)))
    }
}

/// One object of an import.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportObject {
    /// A path or a compound path (`"type": "path"` or `"compound"`, or no `type`).
    Path(OutObject),
    /// Point type (`"type": "type"`).
    Type(OutType),
    /// An embedded image (`"type": "image"`).
    Image(OutImage),
}

/// Point type an importer returned: `{"type": "type", text, fontFamily, fontStyle?, fontSize,
/// fill?, transform?, name?}`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutType {
    /// Its text, a line break (`\n`) between two paragraphs.
    pub text: String,
    pub font_family: String,
    /// `None`: the family's regular style.
    pub font_style: Option<String>,
    /// In points, from [`TYPE_SIZES`]`.0` to [`TYPE_SIZES`]`.1`.
    pub font_size: f64,
    /// `None`: the default fill (black).
    pub fill: Option<Paint>,
    /// Maps the type's own space (the first baseline starts at its origin) into the document.
    pub transform: Affine,
    pub name: Option<String>,
}

/// An embedded image an importer returned: `{"type": "image", pixelSize: [w, h], pixelsBase64,
/// transform?, name?}`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutImage {
    pub width: u32,
    pub height: u32,
    /// Straight RGBA, 8 bits a channel, row by row from the top: `4 × width × height` bytes.
    pub rgba: Vec<u8>,
    /// Maps pixel space (one unit a pixel, y down) into the document.
    pub transform: Affine,
    pub name: Option<String>,
}

/// A top-level layer an importer returned: `{name, visible?, locked?, objects: [index…]}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutLayer {
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    /// Indexes into [`Imported::objects`], increasing: the layer's objects in paint order.
    pub objects: Vec<usize>,
}

/// An artboard an importer returned: `{name, rect: [x, y, width, height]}`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutArtboard {
    pub name: String,
    pub rect: Rect,
}

/// Most layers an import may return.
pub const MAX_LAYERS: usize = 256;

/// Most artboards an import may return.
pub const MAX_ARTBOARDS: usize = 100;

/// Longest layer, artboard, object or font name an import may return, in characters.
pub const MAX_NAME_CHARS: usize = 256;

/// Most bytes of UTF-8 text one type object may hold.
pub const MAX_TYPE_TEXT: usize = 1 << 20;

/// Most bytes of UTF-8 text all the type of one import may hold.
pub const MAX_TEXT_TOTAL: usize = 16 << 20;

/// Most pixels an image may have along either side.
pub const MAX_IMAGE_SIDE: u64 = 16_384;

/// Most bytes of RGBA all the images of one import may hold.
pub const MAX_PIXEL_BYTES: u64 = 32 << 20;

/// The type sizes an import may give, in points (those Character panel sizes take).
pub const TYPE_SIZES: (f64, f64) = (0.1, 1296.0);

/// The file an import reads, as the importer is told in `_context`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source<'a> {
    /// Its extension, without the dot: one the importer lists.
    pub extension: &'a str,
    /// Its name without the folder and extension.
    pub name: &'a str,
}

/// The error of a file larger than [`MAX_INPUT_BYTES`], which no importer is given.
pub fn too_large() -> Error {
    Error::Io(format!("the file is larger than the {MAX_INPUT_BYTES} bytes an importer reads"))
}

/// The limits of an import by `plugin`: its own, with the budgets of an export.
pub fn limits(plugin: &Plugin) -> Limits {
    crate::export::limits(plugin)
}

/// Runs importer `plugin` on `file`, the bytes of the file being opened, with `params` (checked
/// against its manifest: missing ones take their defaults) for the file `source`.
pub fn run(plugin: &Plugin, file: &[u8], params: &Value, source: Source) -> Result<Imported> {
    run_as(plugin, file, params, source, Mode::Import)
}

/// [`run`] for `mode`. [`Mode::Place`] is refused for an importer whose manifest doesn't set `place`
/// ([`not_placing`]), before it runs.
pub fn run_as(plugin: &Plugin, file: &[u8], params: &Value, source: Source, mode: Mode) -> Result<Imported> {
    let m = plugin.manifest();
    if m.kind != Kind::Importer {
        return Err(not_importer(m));
    }
    if mode == Mode::Place && !m.place {
        return Err(not_placing(m));
    }
    if file.len() > MAX_INPUT_BYTES {
        return Err(too_large());
    }
    let mut params = Value::Object(m.resolve_params(params)?);
    params["_context"] = json!({"mode": mode.name(), "extension": source.extension, "name": source.name});
    let params = serde_json::to_vec(&params).map_err(|e| Error::Params(e.to_string()))?;
    decode_with(&plugin.run_limited(file, &params, &limits(plugin))?, &m.output_kinds())
}

/// An importer's output: `{"objects": [...], "warnings"?: ["…"]}`, or `{"error": "…"}` to decline
/// with a message for the user. The objects are new ones (no `id`), checked as a filter's are; an
/// output without any, or with keys of its own, is refused.
pub fn decode(bytes: &[u8]) -> Result<Imported> {
    decode_with(bytes, &[])
}

/// [`decode`] for an importer whose manifest lists `kinds` (`outputKinds`): `objects` may also
/// hold type and images, and the output `layers` and `artboards`, of the kinds listed. Content of
/// a kind not listed is refused naming the kind. Everything is checked before anything is returned,
/// against the limits of this module, and one bad entry refuses the whole output. With `kinds`
/// listing layers or artboards, an output of no objects is an empty document when it has any.
pub fn decode_with(bytes: &[u8], kinds: &[OutputKind]) -> Result<Imported> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    if let Some(e) = declined(&v) {
        return Err(e);
    }
    for key in v.as_object().into_iter().flat_map(|o| o.keys()).filter(|k| !OUTPUT_KEYS.contains(&k.as_str())) {
        let rich = match key.as_str() {
            "layers" => Some(OutputKind::Layer),
            "artboards" => Some(OutputKind::Artboard),
            _ => None,
        };
        match rich {
            Some(kind) if kinds.contains(&kind) => {}
            Some(kind) if !kinds.is_empty() => return Err(undeclared(kind)),
            _ => {
                let key = printable(&json!(key), 64);
                return Err(Error::Output(format!("an importer returns `objects` and `warnings`, not `{key}`")));
            }
        }
    }
    let (objects, layers, artboards) = if kinds.is_empty() {
        let objects = crate::objects::decode(bytes)?;
        (objects.into_iter().map(ImportObject::Path).collect::<Vec<_>>(), Vec::new(), Vec::new())
    } else {
        let objects = rich_objects(&v, kinds)?;
        let layers = match v.get("layers") {
            None => Vec::new(),
            Some(l) => decode_layers(l, objects.len())?,
        };
        let artboards = match v.get("artboards") {
            None => Vec::new(),
            Some(a) => decode_artboards(a)?,
        };
        (objects, layers, artboards)
    };
    for (i, object) in objects.iter().enumerate() {
        if let ImportObject::Path(path) = object {
            let at = format!("objects[{i}]");
            if path.id.is_some() {
                return Err(out_err(&at, "an importer returns new paths without `id`"));
            }
            if path.path.is_none() {
                return Err(out_err(&at, "an imported path must include its geometry in `path`"));
            }
        }
    }
    if objects.is_empty() && layers.is_empty() && artboards.is_empty() {
        return Err(Error::Output("the importer returned no objects".into()));
    }
    let warnings = match v.get("warnings") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(w)) => w.iter().take(crate::export::MAX_WARNINGS).map(|w| printable(w, crate::export::MAX_WARNING_CHARS)).collect(),
        Some(_) => return Err(Error::Output("`warnings` must be a list of strings".into())),
    };
    Ok(Imported { objects, layers, artboards, warnings })
}

/// The error of output holding content of `kind`, which the importer's `outputKinds` doesn't list.
fn undeclared(kind: OutputKind) -> Error {
    let kind = kind.name();
    Error::Output(format!("the output holds `{kind}`, which the importer's `outputKinds` doesn't list"))
}

/// The keys a type object may have.
const TYPE_KEYS: &[&str] = &["type", "text", "fontFamily", "fontStyle", "fontSize", "fill", "transform", "name"];

/// The keys an image object may have.
const IMAGE_KEYS: &[&str] = &["type", "pixelSize", "pixelsBase64", "transform", "name"];

/// The keys a layer may have.
const LAYER_KEYS: &[&str] = &["name", "visible", "locked", "objects"];

/// The keys an artboard may have.
const ARTBOARD_KEYS: &[&str] = &["name", "rect"];

/// The objects of output `v` of an importer listing `kinds`: paths and compound paths as a
/// filter's, type and images of the kinds listed.
fn rich_objects(v: &Value, kinds: &[OutputKind]) -> Result<Vec<ImportObject>> {
    let objects = v.get("objects").and_then(Value::as_array).ok_or_else(|| Error::Output("expected {\"objects\": [...]}".into()))?;
    if objects.len() > MAX_OBJECTS {
        return Err(Error::Output(format!("{} objects (limit {MAX_OBJECTS})", objects.len())));
    }
    let kind_of = |o: &Value| match o.get("type").and_then(Value::as_str) {
        Some("type") => Some(OutputKind::Type),
        Some("image") => Some(OutputKind::Image),
        _ => None,
    };
    if let Some(kind) = objects.iter().filter_map(kind_of).find(|k| !kinds.contains(k)) {
        return Err(undeclared(kind));
    }
    check_totals(objects)?;
    let mut anchors = 0;
    objects
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let at = format!("objects[{i}]");
            match kind_of(o) {
                Some(OutputKind::Type) => decode_type(o, &at).map(ImportObject::Type),
                Some(_) => decode_image(o, &at).map(ImportObject::Image),
                None => crate::objects::decode_object(o, &at, &mut anchors).map(ImportObject::Path),
            }
        })
        .collect()
}

/// Refuses `objects` whose type, all together, holds more than [`MAX_TEXT_TOTAL`] bytes of text, or
/// whose images, by the sizes they give, more than [`MAX_PIXEL_BYTES`] bytes of pixels, before any
/// object is read further or any pixels are decoded.
fn check_totals(objects: &[Value]) -> Result<()> {
    let (mut text, mut pixel_bytes) = (0usize, 0u64);
    for (i, o) in objects.iter().enumerate() {
        let at = format!("objects[{i}]");
        match o.get("type").and_then(Value::as_str) {
            Some("type") => {
                text = text.saturating_add(o.get("text").and_then(Value::as_str).map_or(0, str::len));
                if text > MAX_TEXT_TOTAL {
                    return Err(out_err(&at, format!("the type holds more than {MAX_TEXT_TOTAL} bytes of text")));
                }
            }
            Some("image") => {
                let side = |k: usize| o.get("pixelSize").and_then(|s| s.get(k)).and_then(Value::as_u64).unwrap_or(0);
                let (w, h) = (side(0), side(1));
                if w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
                    return Err(out_err(&at, format!("{w} × {h} pixels (limit {MAX_IMAGE_SIDE} pixels a side)")));
                }
                pixel_bytes = pixel_bytes.saturating_add(w.saturating_mul(h).saturating_mul(4));
                if pixel_bytes > MAX_PIXEL_BYTES {
                    return Err(out_err(&at, format!("the images hold more than {MAX_PIXEL_BYTES} bytes of pixels")));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Refuses a key of JSON object `o` that isn't one of `keys`.
fn known_keys(o: &serde_json::Map<String, Value>, keys: &[&str], at: &str) -> Result<()> {
    match o.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(key) => {
            let key = printable(&json!(key), 64);
            Err(out_err(at, format!("unknown field `{key}`")))
        }
        None => Ok(()),
    }
}

/// Is `c` a character that changes the direction text is laid out in (an embedding, override,
/// isolate or mark)?
fn is_direction_mark(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The name `key` of `o` (`at` names it): 1 to [`MAX_NAME_CHARS`] characters, none of them a
/// control character or a direction mark, and not only spaces. `None` when it is left out and not
/// `required`.
fn name(o: &serde_json::Map<String, Value>, key: &str, required: bool, at: &str) -> Result<Option<String>> {
    let v = match o.get(key) {
        None | Some(Value::Null) if required => return Err(out_err(at, format!("missing field `{key}`"))),
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v,
    };
    let s = v.as_str().ok_or_else(|| out_err(at, format!("`{key}` must be a string")))?;
    let count = s.chars().count();
    if count == 0 || count > MAX_NAME_CHARS || s.trim().is_empty() || s.chars().any(|c| c.is_control() || is_direction_mark(c)) {
        return Err(out_err(at, format!("`{key}` must be 1-{MAX_NAME_CHARS} characters, without control or text direction characters")));
    }
    Ok(Some(s.to_string()))
}

/// The affine `key` of `o` (`at` names it): six finite numbers within ±[`MAX_COORD`] that don't
/// flatten what it places. Left out, no change of place.
fn transform(o: &serde_json::Map<String, Value>, key: &str, at: &str) -> Result<Affine> {
    let Some(v) = o.get(key).filter(|v| !v.is_null()) else { return Ok(Affine::IDENTITY) };
    let wrong = || out_err(at, format!("`{key}` must be six finite numbers within ±{MAX_COORD} that don't flatten the object"));
    let list = v.as_array().filter(|a| a.len() == 6).ok_or_else(wrong)?;
    let mut c = [0.0; 6];
    for (slot, n) in c.iter_mut().zip(list) {
        *slot = n.as_f64().filter(|x| x.is_finite() && x.abs() <= MAX_COORD).ok_or_else(wrong)?;
    }
    let det = c[0] * c[3] - c[1] * c[2];
    if !det.is_normal() {
        return Err(wrong());
    }
    Ok(Affine::new(c))
}

/// A type object of the output (`at` names it).
fn decode_type(v: &Value, at: &str) -> Result<OutType> {
    let o = v.as_object().ok_or_else(|| out_err(at, "an object must be a JSON object"))?;
    known_keys(o, TYPE_KEYS, at)?;
    let text = match o.get("text") {
        None | Some(Value::Null) => {
            let key = "text";
            return Err(out_err(at, format!("missing field `{key}`")));
        }
        Some(t) => t.as_str().ok_or_else(|| {
            let key = "text";
            out_err(at, format!("`{key}` must be a string"))
        })?,
    };
    let len = text.len();
    if len > MAX_TYPE_TEXT {
        return Err(out_err(at, format!("`text` is {len} bytes (limit {MAX_TYPE_TEXT})")));
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(out_err(at, "`text` must not hold control characters other than line breaks and tabs"));
    }
    let font_family = name(o, "fontFamily", true, at)?.unwrap_or_default();
    let font_style = name(o, "fontStyle", false, at)?;
    let key = "fontSize";
    let font_size = match o.get(key) {
        None | Some(Value::Null) => return Err(out_err(at, format!("missing field `{key}`"))),
        Some(s) => s.as_f64().ok_or_else(|| out_err(at, format!("`{key}` must be a number")))?,
    };
    let (min, max) = TYPE_SIZES;
    if !(min..=max).contains(&font_size) {
        let x = font_size;
        return Err(out_err(at, format!("`{key}` must be between {min} and {max} (got {x})")));
    }
    let fill = match o.get("fill") {
        None => None,
        Some(f) => {
            let paint = crate::objects::decode_paint(f, &format!("{at}.fill"))?;
            if matches!(paint, Paint::Pattern { .. }) {
                return Err(out_err(at, "a type fill must be a color or a gradient"));
            }
            Some(crate::objects::unlinked(paint))
        }
    };
    Ok(OutType {
        text: text.to_string(),
        font_family,
        font_style,
        font_size,
        fill,
        transform: transform(o, "transform", at)?,
        name: name(o, "name", false, at)?,
    })
}

/// An image object of the output (`at` names it). Its size has been checked against the limits
/// ([`check_totals`]), and its base64's length is checked against its size before anything is
/// decoded.
fn decode_image(v: &Value, at: &str) -> Result<OutImage> {
    let o = v.as_object().ok_or_else(|| out_err(at, "an object must be a JSON object"))?;
    known_keys(o, IMAGE_KEYS, at)?;
    let size = o.get("pixelSize").and_then(Value::as_array).filter(|a| a.len() == 2).and_then(|a| {
        let side = |v: &Value| v.as_u64().filter(|n| (1..=MAX_IMAGE_SIDE).contains(n));
        Some((side(a.first()?)?, side(a.get(1)?)?))
    });
    let (w, h) = size.ok_or_else(|| out_err(at, format!("`pixelSize` must be [width, height]: two whole numbers from 1 to {MAX_IMAGE_SIDE}")))?;
    let want = w * h * 4;
    let wrong = || out_err(at, format!("`pixelsBase64` must hold 4 × width × height = {want} bytes as base64"));
    let b64 = o.get("pixelsBase64").and_then(Value::as_str).ok_or_else(wrong)?;
    // The length is checked first, so nothing of the wrong size is decoded.
    if b64.len() as u64 != want.div_ceil(3) * 4 || craft_vectorcraft_format::base64::decoded_len(b64) != want {
        return Err(wrong());
    }
    let rgba = craft_vectorcraft_format::base64::decode(b64).filter(|p| p.len() as u64 == want).ok_or_else(wrong)?;
    Ok(OutImage { width: w as u32, height: h as u32, rgba, transform: transform(o, "transform", at)?, name: name(o, "name", false, at)? })
}

/// The `layers` of the output, for `count` objects: 1 to [`MAX_LAYERS`] layers, bottom first,
/// each listing the indexes of its objects in increasing order, and every object in exactly one.
fn decode_layers(v: &Value, count: usize) -> Result<Vec<OutLayer>> {
    let key = "layers";
    let list = entries(v, key, MAX_LAYERS)?;
    let mut owner: Vec<bool> = vec![false; count];
    let mut layers = Vec::with_capacity(list.len());
    for (i, l) in list.iter().enumerate() {
        let at = format!("layers[{i}]");
        let o = l.as_object().ok_or_else(|| out_err(&at, format!("`{key}` must be an object")))?;
        known_keys(o, LAYER_KEYS, &at)?;
        let flag = |key: &str, default: bool| -> Result<bool> {
            match o.get(key) {
                None | Some(Value::Null) => Ok(default),
                Some(v) => v.as_bool().ok_or_else(|| out_err(&at, format!("`{key}` must be true or false"))),
            }
        };
        let wrong = || out_err(&at, format!("`objects` must list object indexes in increasing order, each below {count}"));
        let indexes = match o.get("objects") {
            None | Some(Value::Null) => &[][..],
            Some(v) => v.as_array().ok_or_else(wrong)?.as_slice(),
        };
        let mut objects = Vec::with_capacity(indexes.len().min(count));
        for v in indexes {
            let k = v.as_u64().and_then(|k| usize::try_from(k).ok()).filter(|k| *k < count).ok_or_else(wrong)?;
            if objects.last().is_some_and(|last| *last >= k) {
                return Err(wrong());
            }
            if std::mem::replace(&mut owner[k], true) {
                return Err(out_err(&at, format!("object {k} must be in exactly one layer")));
            }
            objects.push(k);
        }
        layers.push(OutLayer {
            name: name(o, "name", true, &at)?.unwrap_or_default(),
            visible: flag("visible", true)?,
            locked: flag("locked", false)?,
            objects,
        });
    }
    if let Some(k) = owner.iter().position(|owned| !owned) {
        return Err(out_err("layers", format!("object {k} must be in exactly one layer")));
    }
    Ok(layers)
}

/// The `artboards` of the output: 1 to [`MAX_ARTBOARDS`], each named once, their rectangles
/// within ±[`MAX_COORD`].
fn decode_artboards(v: &Value) -> Result<Vec<OutArtboard>> {
    let key = "artboards";
    let list = entries(v, key, MAX_ARTBOARDS)?;
    let mut boards: Vec<OutArtboard> = Vec::with_capacity(list.len());
    for (i, a) in list.iter().enumerate() {
        let at = format!("artboards[{i}]");
        let o = a.as_object().ok_or_else(|| out_err(&at, format!("`{key}` must be an object")))?;
        known_keys(o, ARTBOARD_KEYS, &at)?;
        let name = name(o, "name", true, &at)?.unwrap_or_default();
        if boards.iter().any(|b| b.name == name) {
            let shown = printable(&json!(name), 64);
            return Err(out_err(&at, format!("artboard name `{shown}` is used twice")));
        }
        let wrong = || out_err(&at, format!("`rect` must be [x, y, width, height]: finite numbers within ±{MAX_COORD}, width and height above 0"));
        let r = o.get("rect").and_then(Value::as_array).filter(|r| r.len() == 4).ok_or_else(wrong)?;
        let n: Vec<f64> = r.iter().map(Value::as_f64).collect::<Option<_>>().ok_or_else(wrong)?;
        let (x, y, w, h) = (n[0], n[1], n[2], n[3]);
        let (x1, y1) = (x + w, y + h);
        let inside = |v: f64| v.is_finite() && v.abs() <= MAX_COORD;
        if !(w > 0.0 && h > 0.0 && x1 > x && y1 > y && inside(x) && inside(y) && inside(x1) && inside(y1)) {
            return Err(wrong());
        }
        boards.push(OutArtboard { name, rect: Rect::new(x, y, x1, y1) });
    }
    Ok(boards)
}

/// The list `v`, the output's `key`, of 1 to `max` entries.
fn entries<'a>(v: &'a Value, key: &str, max: usize) -> Result<&'a Vec<Value>> {
    let list = v.as_array().ok_or_else(|| Error::Output(format!("`{key}` must be an array")))?;
    let n = list.len();
    if !(1..=max).contains(&n) {
        return Err(Error::Output(format!("`{key}` must hold 1 to {max} entries (it holds {n})")));
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r#"{"type":"path","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[10,0]},{"p":[10,10]}]}]}}"#;

    #[test]
    fn output_is_the_objects_and_the_warnings_or_a_decline() {
        let out = decode(format!(r#"{{"objects":[{SQUARE}],"warnings":["a",2]}}"#).as_bytes()).unwrap();
        assert!(out.paths_only());
        assert!(out.objects.iter().all(|o| matches!(o, ImportObject::Path(o) if o.id.is_none() && o.path.is_some())));
        assert_eq!((out.objects.len(), out.warnings), (1, vec!["a".to_string(), "2".to_string()]));
        assert_eq!(decode(format!(r#"{{"objects":[{SQUARE}]}}"#).as_bytes()).unwrap().warnings, Vec::<String>::new());
        assert_eq!(decode(br#"{"error": "Not a\nplot file."}"#), Err(Error::Failed("Not aplot file.".into())));
        // A decline is reported as one, also with keys the host doesn't read beside it.
        assert_eq!(decode(br#"{"error": "No.", "layers": []}"#), Err(Error::Failed("No.".into())));
    }

    /// Nothing an importer returns reaches a document unchecked: malformed JSON, no objects, keys
    /// the host doesn't read, bad warnings and bad objects (coordinates out of range, unknown paints)
    /// are all errors.
    #[test]
    fn bad_output_is_refused() {
        let far = r#"{"objects":[{"path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[1e9,0]}]}]}}]}"#;
        for (bad, says) in [
            (&b"[]"[..], "objects"),
            (b"{}", "objects"),
            (b"not json", "not JSON"),
            (br#"{"objects":[]}"#, "no objects"),
            (br#"{"objects":{}}"#, "objects"),
            (br#"{"objects":[1]}"#, "JSON object"),
            (far.as_bytes(), "out of range"),
            (br#"{"objects":[{"path":{"subpaths":[]},"fills":[{"type":"hatch"}]}]}"#, "objects[0]"),
            (br#"{"objects":[],"artboard":[0,0,1,1]}"#, "not `artboard`"),
        ] {
            let e = decode(bad).unwrap_err().to_string();
            assert!(e.contains(says), "{}: {e}", String::from_utf8_lossy(bad));
        }
        let warns = format!(r#"{{"objects":[{SQUARE}],"warnings":"x"}}"#);
        assert!(decode(warns.as_bytes()).unwrap_err().to_string().contains("`warnings`"));
    }

    const ALL: &[OutputKind] = &OutputKind::ALL;

    /// A change made to [`rich`] output.
    type Edit = Box<dyn FnOnce(&mut Value)>;

    /// Type, an image of 2 × 1 pixels, the square, two layers and two artboards.
    fn rich() -> Value {
        let pixels = craft_vectorcraft_format::base64::encode(&[255, 0, 0, 255, 0, 0, 255, 128]);
        json!({
            "objects": [
                {"type": "type", "text": "Hello\nWorld", "fontFamily": "Source Sans 3", "fontStyle": "Bold", "fontSize": 24,
                 "fill": {"type": "solid", "color": {"model": "rgb", "r": 0, "g": 0, "b": 1}}, "transform": [1, 0, 0, 1, 10, 40], "name": "Title"},
                {"type": "image", "pixelSize": [2, 1], "pixelsBase64": pixels, "transform": [10, 0, 0, 10, 0, 100]},
                serde_json::from_str::<Value>(SQUARE).unwrap(),
            ],
            "layers": [{"name": "Art", "objects": [1, 2]}, {"name": "Words", "visible": false, "locked": true, "objects": [0]}],
            "artboards": [{"name": "Front", "rect": [0, 0, 200, 100]}, {"name": "Back", "rect": [220, 0, 200, 100]}],
        })
    }

    fn rich_with(edit: impl FnOnce(&mut Value)) -> Result<Imported> {
        let mut v = rich();
        edit(&mut v);
        decode_with(v.to_string().as_bytes(), ALL)
    }

    #[test]
    fn importer_output_rejects_filter_only_path_updates() {
        let square: Value = serde_json::from_str(SQUARE).unwrap();
        let mut identified = square.clone();
        identified["id"] = json!(3);
        for object in [json!({"id": 3}), json!({"type": "path"}), identified] {
            let output = json!({"objects": [object]});
            for kinds in [&[OutputKind::Type][..], &[][..]] {
                assert!(decode_with(output.to_string().as_bytes(), kinds).is_err(), "{kinds:?}: {output}");
            }
        }
        let mut untyped = square.clone();
        untyped.as_object_mut().unwrap().remove("type");
        for object in [square, untyped] {
            let output = json!({"objects": [object]});
            for kinds in [&[OutputKind::Type][..], &[][..]] {
                assert!(decode_with(output.to_string().as_bytes(), kinds).is_ok(), "{kinds:?}: {output}");
            }
        }
    }

    #[test]
    fn importer_output_rejects_artboards_with_collapsed_endpoints() {
        for rect in [[1_000_000.0, 0.0, 1e-12, 10.0], [0.0, 1_000_000.0, 10.0, 1e-12]] {
            assert!(rich_with(|v| v["artboards"][0]["rect"] = json!(rect)).is_err(), "{rect:?}");
        }
        assert!(rich_with(|v| v["artboards"][0]["rect"] = json!([3_999_999.0, 3_999_999.0, 1.0, 1.0])).is_ok());
    }

    /// An importer listing every output kind returns type, an embedded image, paths, layers and
    /// artboards, each read as it was given.
    #[test]
    fn each_output_kind_is_read_as_given() {
        let out = decode_with(rich().to_string().as_bytes(), ALL).unwrap();
        assert!(!out.paths_only());
        let [ImportObject::Type(t), ImportObject::Image(im), ImportObject::Path(p)] = out.objects.as_slice() else { panic!("{:?}", out.objects) };
        assert_eq!(
            (t.text.as_str(), t.font_family.as_str(), t.font_style.as_deref(), t.font_size),
            ("Hello\nWorld", "Source Sans 3", Some("Bold"), 24.0)
        );
        assert_eq!((t.transform, t.name.as_deref()), (Affine::translate((10.0, 40.0)), Some("Title")));
        assert!(matches!(t.fill, Some(Paint::Solid { .. })));
        assert_eq!((im.width, im.height, im.rgba.as_slice()), (2, 1, &[255, 0, 0, 255, 0, 0, 255, 128][..]));
        assert!(p.path.is_some());
        let layers: Vec<_> = out.layers.iter().map(|l| (l.name.as_str(), l.visible, l.locked, l.objects.clone())).collect();
        assert_eq!(layers, [("Art", true, false, vec![1, 2]), ("Words", false, true, vec![0])]);
        let boards: Vec<_> = out.artboards.iter().map(|a| (a.name.as_str(), a.rect)).collect();
        assert_eq!(boards, [("Front", Rect::new(0.0, 0.0, 200.0, 100.0)), ("Back", Rect::new(220.0, 0.0, 420.0, 100.0))]);
        // Artboards alone are a blank document.
        let blank = decode_with(br#"{"objects":[],"artboards":[{"name":"A","rect":[0,0,10,10]}]}"#, &[OutputKind::Artboard]).unwrap();
        assert_eq!((blank.objects.len(), blank.artboards.len()), (0, 1));
    }

    /// Output of a kind the manifest doesn't list is refused naming the kind; an importer listing
    /// none reads paths alone, as before.
    #[test]
    fn a_kind_not_listed_is_refused_naming_it() {
        let v = rich().to_string();
        for (kinds, named) in [
            (&[OutputKind::Image, OutputKind::Layer, OutputKind::Artboard][..], "`type`"),
            (&[OutputKind::Type, OutputKind::Layer, OutputKind::Artboard][..], "`image`"),
            (&[OutputKind::Type, OutputKind::Image, OutputKind::Artboard][..], "`layer`"),
            (&[OutputKind::Type, OutputKind::Image, OutputKind::Layer][..], "`artboard`"),
        ] {
            let e = decode_with(v.as_bytes(), kinds).unwrap_err().to_string();
            assert!(e.contains(named) && e.contains("`outputKinds` doesn't list"), "{named}: {e}");
        }
        let typed = r#"{"objects":[{"type":"type","text":"a","fontFamily":"x","fontSize":12}]}"#;
        assert!(decode(typed.as_bytes()).unwrap_err().to_string().contains("`type` must be \"path\" or \"compound\""));
        let layered = format!(r#"{{"objects":[{SQUARE}],"layers":[{{"name":"A","objects":[0]}}]}}"#);
        assert!(decode(layered.as_bytes()).unwrap_err().to_string().contains("not `layers`"));
        assert!(decode(br#"{"objects":[],"artboards":[]}"#).unwrap_err().to_string().contains("not `artboards`"));
    }

    /// Each limit refuses the whole output and names the limit.
    #[test]
    fn each_limit_is_enforced_naming_it() {
        let names = |n: usize| (0..n).map(|i| json!({"name": format!("L{i}")})).collect::<Vec<_>>();
        let cases: Vec<(Edit, String)> = vec![
            (Box::new(move |v: &mut Value| v["layers"] = json!(names(MAX_LAYERS + 1))), format!("1 to {MAX_LAYERS} entries")),
            (Box::new(|v: &mut Value| v["layers"] = json!([])), "1 to".into()),
            (
                Box::new(|v: &mut Value| {
                    v["artboards"] = json!((0..=MAX_ARTBOARDS).map(|i| json!({"name": format!("A{i}"), "rect": [0, 0, 1, 1]})).collect::<Vec<_>>())
                }),
                format!("1 to {MAX_ARTBOARDS} entries"),
            ),
            (Box::new(|v: &mut Value| v["objects"][0]["text"] = json!("x".repeat(MAX_TYPE_TEXT + 1))), format!("(limit {MAX_TYPE_TEXT})")),
            (
                Box::new(|v: &mut Value| v["objects"][1]["pixelSize"] = json!([MAX_IMAGE_SIDE + 1, 1])),
                format!("(limit {MAX_IMAGE_SIDE} pixels a side)"),
            ),
            (Box::new(|v: &mut Value| v["objects"][0]["fontSize"] = json!(TYPE_SIZES.1 * 2.0)), "`fontSize` must be between".into()),
            (
                Box::new(|v: &mut Value| v["objects"][0]["fontFamily"] = json!("f".repeat(MAX_NAME_CHARS + 1))),
                format!("1-{MAX_NAME_CHARS} characters"),
            ),
        ];
        for (edit, says) in cases {
            let e = rich_with(edit).unwrap_err().to_string();
            assert!(e.contains(&says), "{says}: {e}");
        }
        // Totals over all type and all images, found from the sizes they give before any pixels are
        // decoded: these images carry no pixels at all.
        let big = json!({"type": "image", "pixelSize": [MAX_IMAGE_SIDE, MAX_IMAGE_SIDE]});
        let e = decode_with(json!({"objects": [big]}).to_string().as_bytes(), ALL).unwrap_err().to_string();
        assert!(e.contains(&format!("more than {MAX_PIXEL_BYTES} bytes of pixels")), "{e}");
        let text = "t".repeat(MAX_TYPE_TEXT);
        let many: Vec<Value> = (0..=MAX_TEXT_TOTAL / MAX_TYPE_TEXT).map(|_| json!({"type": "type", "text": text, "fontSize": 12})).collect();
        let e = decode_with(json!({"objects": many}).to_string().as_bytes(), ALL).unwrap_err().to_string();
        assert!(e.contains(&format!("more than {MAX_TEXT_TOTAL} bytes of text")), "{e}");
    }

    /// An image whose bytes aren't 4 × width × height, or aren't base64, is refused; so are names
    /// holding control or direction characters, text holding control characters, layers that don't
    /// hold each object once in order, and bad artboards.
    #[test]
    fn malformed_content_is_refused() {
        let cases: Vec<(Edit, &str)> = vec![
            (Box::new(|v: &mut Value| v["objects"][1]["pixelsBase64"] = json!(craft_vectorcraft_format::base64::encode(&[1, 2, 3, 4]))), "= 8 bytes"),
            (Box::new(|v: &mut Value| v["objects"][1]["pixelsBase64"] = json!("!!!!!!!!!!!!")), "= 8 bytes"),
            (Box::new(|v: &mut Value| v["objects"][1]["pixelSize"] = json!([0, 1])), "`pixelSize`"),
            (Box::new(|v: &mut Value| v["objects"][1]["linked"] = json!(true)), "unknown field `linked`"),
            (Box::new(|v: &mut Value| v["objects"][0]["kind"] = json!("area")), "unknown field `kind`"),
            (Box::new(|v: &mut Value| v["objects"][0]["text"] = json!("a\u{7}b")), "control characters"),
            (Box::new(|v: &mut Value| v["objects"][0]["fill"] = json!({"type": "pattern", "pattern": "p"})), "objects[0]"),
            (Box::new(|v: &mut Value| v["objects"][0]["transform"] = json!([0, 0, 0, 0, 0, 0])), "six finite numbers"),
            (Box::new(|v: &mut Value| v["layers"][0]["name"] = json!("Art\u{202E}txt")), "text direction characters"),
            (Box::new(|v: &mut Value| v["artboards"][0]["name"] = json!("A\u{0}")), "text direction characters"),
            (Box::new(|v: &mut Value| v["layers"][0]["objects"] = json!([2, 1])), "increasing order"),
            (Box::new(|v: &mut Value| v["layers"][0]["objects"] = json!([0, 1, 2])), "object 0 must be in exactly one layer"),
            (Box::new(|v: &mut Value| v["layers"][0]["objects"] = json!([1])), "object 2 must be in exactly one layer"),
            (Box::new(|v: &mut Value| v["layers"][0]["objects"] = json!([1, 2, 3])), "each below 3"),
            (Box::new(|v: &mut Value| v["artboards"][1]["name"] = json!("Front")), "`Front` is used twice"),
            (Box::new(|v: &mut Value| v["artboards"][0]["rect"] = json!([0, 0, 0, 10])), "width and height above 0"),
            (Box::new(|v: &mut Value| v["artboards"][0]["rect"] = json!([3_999_999, 0, 10, 10])), "within ±4000000"),
        ];
        for (edit, says) in cases {
            let e = rich_with(edit).unwrap_err().to_string();
            assert!(e.contains(says), "{says}: {e}");
        }
    }

    #[test]
    fn warnings_are_capped_in_number_and_length() {
        let many: Vec<String> = (0..crate::export::MAX_WARNINGS + 5).map(|i| format!("w{i}")).collect();
        let square: Value = serde_json::from_str(SQUARE).unwrap();
        let out = decode(json!({"objects": [square], "warnings": many}).to_string().as_bytes()).unwrap();
        assert_eq!(out.warnings.len(), crate::export::MAX_WARNINGS);
        let long = "x".repeat(crate::export::MAX_WARNING_CHARS + 1);
        let out = decode(json!({"objects": [square], "warnings": [long]}).to_string().as_bytes()).unwrap();
        assert_eq!(out.warnings.first().map(|w| w.chars().count()), Some(crate::export::MAX_WARNING_CHARS));
    }
}
