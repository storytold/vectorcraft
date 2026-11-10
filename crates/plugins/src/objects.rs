//! The object model plug-ins read and write: paths and compound paths as JSON.
//!
//! The host hands a plug-in `{"objects": [...]}` (see [`encode`]) and reads the same shape back
//! ([`decode`]). Geometry and paint use the native file format's forms (`PathData`, `Paint`), so
//! nothing is lost on the way through; everything a plug-in returns is validated (finite,
//! in-range coordinates, capped counts, clamped colours) before it reaches the document.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Cms, CmykLut, lab};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::cmyk::oversized_tiff;
use vectorcraft_doc::{
    Appearance, AppearanceItem, Document, Effect, FillLayer, ImageBlob, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeLayer, SymbolBoxes,
    TextKind, TextObject,
};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect};

use crate::{Error, InputKind, Result};

/// Coordinates a plug-in may return, in points (the document's own canvas limit).
pub const MAX_COORD: f64 = 4.0e6;
/// Most objects one output may hold.
pub const MAX_OBJECTS: usize = 100_000;
/// Most anchor points one output may hold, over all objects.
pub const MAX_ANCHORS: usize = 4_000_000;
/// Most fills or strokes per object, gradient stops per gradient and freeform points or lines
/// per freeform gradient.
pub const MAX_ITEMS: usize = 256;
/// Thickest stroke a plug-in may set, in points.
pub const MAX_STROKE_WIDTH: f64 = 1000.0;
/// The range of a stroke's miter limit (as the Stroke panel's).
pub const MITER_LIMIT: std::ops::RangeInclusive<f64> = 1.0..=500.0;
/// Longest effect id a plug-in may return, in bytes.
pub const MAX_EFFECT_ID: usize = 128;
/// Largest data a plug-in may keep with a document (`documentData`), in bytes of JSON.
pub const MAX_DOCUMENT_DATA: usize = 1 << 20;
/// Most text a filter is given of one type object, in bytes of UTF-8 over all its runs: longer
/// type comes with `truncated` and no runs or paragraphs.
pub const MAX_INPUT_TEXT: usize = 256 * 1024;
/// Most runs, and most paragraphs, a filter is given of one type object (as [`MAX_INPUT_TEXT`]).
pub const MAX_INPUT_RUNS: usize = 4_096;
/// Largest pixels a filter is given of one embedded image, in characters of base64: a larger
/// image comes with `pixelsOmitted`.
pub const MAX_INPUT_PIXELS: usize = 1 << 20;
/// Most pixels a filter is given over all the images of one input, in characters of base64: an
/// image whose pixels don't fit what the images before it left comes with `pixelsOmitted`, and an
/// image the host starts to decode takes its size from the total even when decoding fails.
pub const MAX_INPUT_PIXELS_TOTAL: usize = 16 << 20;
/// Most text a filter is given over all the type of one input, in bytes of the JSON of their runs
/// and paragraphs: type that doesn't fit what the type before it left comes with `truncated`.
pub const MAX_INPUT_TEXT_TOTAL: usize = 16 << 20;

/// One object returned by a plug-in. `None` fields keep the object's value (new objects take the
/// defaults: a path, non-zero rule, no fills or strokes, full opacity).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutObject {
    /// The input object this updates; `None`: a new object.
    pub id: Option<NodeId>,
    /// `"compound"` (true) or `"path"` (false).
    pub compound: Option<bool>,
    /// `Some("")` clears the name.
    pub name: Option<String>,
    pub path: Option<PathData>,
    pub rule: Option<FillRule>,
    /// Fill paints in paint order (bottom first).
    pub fills: Option<Vec<Paint>>,
    pub strokes: Option<Vec<OutStroke>>,
    pub opacity: Option<f32>,
    /// The object's effects, in order, replacing its own (not its fills' or strokes'). The host
    /// admits them before they reach the document ([`AdmitEffects`]).
    pub effects: Option<Vec<Effect>>,
    /// Makes the object a guide (`true`, as View › Guides › Make Guides does) or a plain path.
    pub guide: Option<bool>,
    /// Locks or unlocks the object (as Object › Lock and Unlock do).
    pub locked: Option<bool>,
    /// A read-only input object (type, an image or a symbol instance) returned as it was given:
    /// it stays as it is, and new objects listed after it go above it.
    pub kept: bool,
}

/// One stroke of an [`OutObject`]. `None` fields keep the stroke's value (a new stroke's
/// default: 1 pt, butt cap, miter join, miter limit 10).
#[derive(Clone, Debug, PartialEq)]
pub struct OutStroke {
    pub paint: Paint,
    pub width: Option<f64>,
    pub join: Option<LineJoin>,
    pub cap: Option<LineCap>,
    pub miter_limit: Option<f64>,
}

impl OutStroke {
    /// A stroke of `paint` that keeps everything else.
    pub fn paint(paint: Paint) -> Self {
        Self { paint, width: None, join: None, cap: None, miter_limit: None }
    }
}

/// How the host admits the effect records a filter returns for object `n` (written with the
/// output, its effects still its own): the records to give it, or why they are refused. The
/// plug-in crate checks their form; which effects exist and what parameters they take is the
/// host's to say.
pub type AdmitEffects<'a> = &'a dyn Fn(&[Effect], &Node) -> std::result::Result<Vec<Effect>, String>;

/// The name plug-ins use for line join `j`, as SVG's `stroke-linejoin`.
fn join_name(j: LineJoin) -> &'static str {
    match j {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    }
}

/// The name plug-ins use for line cap `c`, as SVG's `stroke-linecap`.
fn cap_name(c: LineCap) -> &'static str {
    match c {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    }
}

fn rule_name(r: FillRule) -> &'static str {
    match r {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    }
}

fn rect_json(r: Option<Rect>) -> Value {
    r.map_or(Value::Null, |r| json!([r.x0, r.y0, r.x1, r.y1]))
}

fn paint_json(p: &Paint) -> Value {
    match p {
        Paint::None => Value::Null,
        p => serde_json::to_value(p).unwrap_or(Value::Null),
    }
}

/// The geometry of a path or compound path (a compound's members' subpaths in order) and its fill
/// rule. `None` for other objects.
pub fn geometry(n: &Node) -> Option<(PathData, FillRule, bool)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule, false)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule, true))
        }
        _ => None,
    }
}

/// The fills and strokes of appearance `ap` as plug-ins see them, each in paint order: a fill is
/// its paint, a stroke `{paint, width, join, cap, miterLimit}`. Filters and live effects are given
/// the same.
pub(crate) fn paint_lists(ap: &Appearance) -> (Vec<Value>, Vec<Value>) {
    let fills = ap.items.iter().filter_map(|i| if let AppearanceItem::Fill(f) = i { Some(paint_json(&f.paint)) } else { None }).collect();
    let stroke = |s: &StrokeLayer| json!({"paint": paint_json(&s.paint), "width": s.width, "join": join_name(s.join), "cap": cap_name(s.cap), "miterLimit": s.miter_limit});
    let strokes = ap.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(stroke(s)) } else { None }).collect();
    (fills, strokes)
}

/// A path or compound path as plug-ins see it (`None` for other objects): `{id, type, name?, path,
/// fillRule, fills, strokes, effects, opacity, bounds}`.
pub fn encode(n: &Node) -> Option<Value> {
    let (path, rule, compound) = geometry(n)?;
    let (fills, strokes) = paint_lists(&n.appearance);
    let mut o = json!({
        "id": n.id.0, "type": if compound { "compound" } else { "path" }, "path": path, "fillRule": rule_name(rule),
        "fills": fills, "strokes": strokes, "effects": n.appearance.effects, "opacity": n.opacity, "bounds": rect_json(path.bounds()),
    });
    if let Some(name) = &n.name {
        o["name"] = json!(name);
    }
    Some(o)
}

/// The input document of a filter run: `{"objects": [...]}` for `ids` (paths and compound paths;
/// others are left out).
pub fn encode_input(doc: &Document, ids: &[NodeId]) -> Vec<u8> {
    encode_input_with(doc, ids, &[], usize::MAX).map(|(bytes, _)| bytes).unwrap_or_default()
}

/// [`encode_input`] for a filter that asks for `kinds` too: objects of those kinds among `ids` are
/// in it as [`encode_read_only`] gives them, within [`MAX_INPUT_PIXELS_TOTAL`] and
/// [`MAX_INPUT_TEXT_TOTAL`] over all of them in order, and within `capacity`, the most bytes the
/// input may be. Every object counts toward `capacity` in order, ids, bounds, transforms and names
/// included: pixels or text that don't fit what is left are omitted (`pixelsOmitted`,
/// `truncated`), a read-only object whose other fields alone don't fit is an [`Error::Limit`]
/// naming it and its size, a path or compound path that doesn't fit is the limit the runtime gives
/// an input past its share, and more than [`MAX_OBJECTS`] read-only objects, more than an output
/// may list, is an [`Error::Limit`] too. Also returns where those objects are in the input, which
/// the filter must return as they were ([`decode_output_with`]).
pub fn encode_input_with(doc: &Document, ids: &[NodeId], kinds: &[InputKind], capacity: usize) -> Result<(Vec<u8>, ReadOnlySpans)> {
    encode_input_within(doc, ids, kinds, Allowance { input: capacity.saturating_sub(EMPTY_INPUT.len()), ..Allowance::FULL })
}

/// Where each read-only object of an input is in the input's bytes.
pub type ReadOnlySpans = HashMap<NodeId, Range<usize>>;

/// The input with no objects; each object adds its JSON and a comma.
const EMPTY_INPUT: &[u8] = b"{\"objects\":[]}";

/// What an input's objects may still be given of its totals, as they are encoded in order:
/// characters of base64 for pixels, bytes of JSON for runs and paragraphs, and bytes of the whole
/// input.
#[derive(Clone, Copy, Debug)]
struct Allowance {
    pixels: usize,
    text: usize,
    input: usize,
}

impl Allowance {
    const FULL: Allowance = Allowance { pixels: MAX_INPUT_PIXELS_TOTAL, text: MAX_INPUT_TEXT_TOTAL, input: usize::MAX };
}

/// [`encode_input_with`] within `left` of the totals. Each object is held only while it is written
/// into the input.
fn encode_input_within(doc: &Document, ids: &[NodeId], kinds: &[InputKind], mut left: Allowance) -> Result<(Vec<u8>, ReadOnlySpans)> {
    let mut read_only = HashMap::new();
    let symbols = doc.symbol_boxes();
    let mut out = b"{\"objects\":[".to_vec();
    let mut first = true;
    for n in ids.iter().filter_map(|id| doc.node(*id)) {
        let (o, checked) = match input_kind(n).filter(|k| kinds.contains(k)) {
            // The output must return every read-only object, and lists at most `MAX_OBJECTS`.
            Some(_) if read_only.len() >= MAX_OBJECTS => return Err(too_many_read_only()),
            Some(_) => match encode_read_only_within(doc, n, &symbols, &mut left)? {
                Some(o) => (o, true),
                None => continue,
            },
            None => match encode(n) {
                Some(o) => (o, false),
                None => continue,
            },
        };
        let start = out.len();
        if !std::mem::take(&mut first) {
            out.push(b',');
        }
        let object = out.len();
        if serde_json::to_writer(&mut out, &o).is_err() {
            return Ok((Vec::new(), HashMap::new()));
        }
        if checked {
            read_only.insert(n.id, object..out.len());
        }
        let size = out.len() - start;
        // A path or compound path past what is left fails the run as the runtime would fail it,
        // before the rest of the selection is written.
        if size > left.input {
            return Err(if checked { too_large(n, size, left.input) } else { Error::Limit(crate::runtime::SELECTION_TOO_LARGE.into()) });
        }
        left.input -= size;
    }
    out.extend_from_slice(b"]}");
    Ok((out, read_only))
}

/// The error for read-only object `n`, which needs `size` bytes of input where `left` are left.
fn too_large(n: &Node, size: usize, left: usize) -> Error {
    Error::Limit(format!("memory budget (object {} needs {size} bytes of input, {left} left)", n.id.0))
}

/// The error for an input with more read-only objects than an output may list.
fn too_many_read_only() -> Error {
    Error::Limit(format!("object budget (more than {MAX_OBJECTS} read-only objects)"))
}

/// The kind of read-only object `n` is: type, an image or a symbol instance (`None` for others).
pub fn input_kind(n: &Node) -> Option<InputKind> {
    match &n.kind {
        NodeKind::Text(_) => Some(InputKind::Type),
        NodeKind::Image(_) => Some(InputKind::Image),
        NodeKind::SymbolInstance { .. } => Some(InputKind::SymbolInstance),
        _ => None,
    }
}

fn affine_json(xf: Affine) -> Value {
    json!(xf.as_coeffs())
}

/// Type, an image or a symbol instance of `doc` as a filter that asks for its kind reads it (`None`
/// for other objects). Each has `{id, type, name?, bounds, transform}`, `transform` mapping its own
/// space (text space, pixel space, the symbol's art) into the document, and:
/// - type (`"type"`): `text: {kind: "point"|"area"|"path", vertical, runs: [{text, style, inline?}],
///   paragraphs: [style]}` with the native character and paragraph attributes, one paragraph style
///   for each paragraph, and `truncated`: true, with no runs or paragraphs, past [`MAX_INPUT_TEXT`]
///   or [`MAX_INPUT_RUNS`];
/// - an image (`"image"`): `pixelSize: [w, h]`, `colorMode: "rgb"|"cmyk"|"gray"|"unknown"`,
///   `linked`, `pixelsOmitted`, and for an embedded image of at most [`MAX_INPUT_PIXELS`]
///   `pixelsBase64`: straight RGBA, 8 bits a channel, row by row from the top (a CMYK image's inks
///   through the working CMYK profile, as sRGB). A linked image's file and path are never read or
///   given;
/// - a symbol instance (`"symbolInstance"`): `symbolId` (the symbol's name), `missing: true` when
///   the document has no such symbol, and `overrides: {}`.
pub fn encode_read_only(doc: &Document, n: &Node) -> Option<Value> {
    let mut left = Allowance::FULL;
    encode_read_only_within(doc, n, &doc.symbol_boxes(), &mut left).ok().flatten()
}

/// The bytes the `pixelsBase64` key adds to an image's JSON besides the base64 itself.
const PIXELS_KEY: usize = ",\"pixelsBase64\":\"\"".len();

/// [`encode_read_only`] measuring symbol instances from `symbols`, a cache shared across the
/// objects of one input, and taking the pixels and text it gives from `left`: pixels or text that
/// don't fit what is left of their totals or of the input are omitted (`pixelsOmitted`,
/// `truncated`), and an image's size is checked against it before anything is decoded. The object's
/// other fields are measured first, with a comma before it: when they alone don't fit what is left
/// of the input, it is an [`Error::Limit`] naming the object and that size.
fn encode_read_only_within(doc: &Document, n: &Node, symbols: &SymbolBoxes, left: &mut Allowance) -> Result<Option<Value>> {
    let Some(kind) = input_kind(n) else { return Ok(None) };
    let mut o = json!({"id": n.id.0, "type": kind.name(), "bounds": rect_json(doc.bounds_of_with(&[n.id], false, symbols))});
    // Each flag is measured as `false`, its longer value.
    match &n.kind {
        NodeKind::Text(t) => {
            o["transform"] = affine_json(t.xf);
            o["text"] = text_json(t, None).0;
            o["truncated"] = json!(false);
        }
        NodeKind::Image(im) => {
            o["transform"] = affine_json(im.xf);
            o["pixelSize"] = json!([im.width, im.height]);
            o["colorMode"] = json!(doc.image_blob(im).map_or("unknown", color_mode));
            o["linked"] = json!(im.link.is_some());
            o["pixelsOmitted"] = json!(false);
        }
        NodeKind::SymbolInstance { symbol, xf } => {
            o["transform"] = affine_json(*xf);
            o["symbolId"] = json!(symbol);
            if !doc.symbols.iter().any(|s| s.name == *symbol) {
                o["missing"] = json!(true);
            }
            o["overrides"] = json!({});
        }
        _ => return Ok(None),
    }
    if let Some(name) = &n.name {
        o["name"] = json!(name);
    }
    let fields = serde_json::to_vec(&o).map_or(usize::MAX, |b| b.len()).saturating_add(1);
    if fields > left.input {
        return Err(too_large(n, fields, left.input));
    }
    let room = left.input - fields;
    match &n.kind {
        NodeKind::Text(t) => {
            let mut budget = left.text.min(room);
            let (text, truncated) = text_json(t, Some(&mut budget));
            left.text -= left.text.min(room) - budget;
            o["text"] = text;
            o["truncated"] = json!(truncated);
        }
        NodeKind::Image(im) => {
            let blob = doc.image_blob(im);
            let max = MAX_INPUT_PIXELS.min(left.pixels).min(room.saturating_sub(PIXELS_KEY));
            let mut charged = 0;
            // A linked image's pixels are its file's: never given (nor its path).
            let pixels = blob.filter(|_| im.link.is_none()).and_then(|b| rgba_pixels(b, im.width, im.height, max, &mut charged));
            left.pixels = left.pixels.saturating_sub(charged);
            o["pixelsOmitted"] = json!(pixels.is_none());
            if let Some(p) = pixels {
                o["pixelsBase64"] = json!(craft_vectorcraft_format::base64::encode(&p));
            }
        }
        _ => {}
    }
    Ok(Some(o))
}

/// Counts the bytes written to it and fails past `left`, so nothing is held to find a size.
struct Measure {
    bytes: usize,
    left: usize,
}

impl std::io::Write for Measure {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buf.len());
        if self.bytes > self.left { Err(std::io::ErrorKind::OutOfMemory.into()) } else { Ok(buf.len()) }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Adds `item` as JSON to `list` when it fits what is left of `limit` after `used`, counting the
/// comma before it; `false`, with nothing built, when it doesn't.
fn push_within<T: serde::Serialize>(item: &T, list: &mut Vec<Value>, used: &mut usize, limit: usize) -> bool {
    let comma = usize::from(!list.is_empty());
    let Some(left) = limit.checked_sub(used.saturating_add(comma)) else { return false };
    if serde_json::to_writer(Measure { bytes: 0, left }, item).is_err() {
        return false;
    }
    let Ok(value) = serde_json::to_value(item) else { return false };
    // A `Value` writes a number in its widest form, so the bytes given are counted from it.
    let mut measure = Measure { bytes: 0, left };
    if serde_json::to_writer(&mut measure, &value).is_err() {
        return false;
    }
    *used += comma + measure.bytes;
    list.push(value);
    true
}

/// The text of type `t` as a filter reads it, and whether it was too long to give
/// ([`MAX_INPUT_TEXT`], [`MAX_INPUT_RUNS`], or more bytes of JSON than `left`, which what it gives
/// is taken from, with the commas between them). With `left` `None`, its kind and direction with no
/// runs or paragraphs.
fn text_json(t: &TextObject, left: Option<&mut usize>) -> (Value, bool) {
    let kind = match t.kind {
        TextKind::Point => "point",
        TextKind::Area { .. } => "area",
        TextKind::OnPath { .. } => "path",
    };
    let (mut runs, mut paras): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    let mut truncated = false;
    if let Some(left) = left {
        let bytes = t.runs.iter().fold(0usize, |n, r| n.saturating_add(r.text.len()));
        let paragraphs = t.paragraph_count();
        // The text alone is never more bytes than its JSON, so type that can't fit isn't built.
        truncated = bytes > MAX_INPUT_TEXT.min(*left) || t.runs.len() > MAX_INPUT_RUNS || paragraphs > MAX_INPUT_RUNS;
        if !truncated {
            // Each run and paragraph is measured before it is built, with the comma before it, so
            // no more than `left` is held before the type is found too long.
            let (limit, mut used) = (*left, 0usize);
            let fits = t.runs.iter().all(|r| push_within(r, &mut runs, &mut used, limit))
                && (0..paragraphs).all(|i| push_within(&t.para_at(i), &mut paras, &mut used, limit));
            if fits {
                *left -= used;
            } else {
                (runs, paras, truncated) = (Vec::new(), Vec::new(), true);
            }
        }
    }
    (json!({"kind": kind, "vertical": t.vertical, "runs": runs, "paragraphs": paras}), truncated)
}

/// How image `b` stores its colors, read from its header: `"cmyk"`, `"gray"`, `"rgb"`, or
/// `"unknown"` when it can't be read, is a TIFF too large for the decoder to count, or only a
/// linked image's preview is there.
fn color_mode(b: &ImageBlob) -> &'static str {
    if oversized_tiff(&b.bytes).is_some() {
        return "unknown";
    }
    if b.proxy.as_ref().is_some_and(|p| Arc::ptr_eq(p, &b.bytes) || **p == *b.bytes) {
        return "unknown";
    }
    if b.is_cmyk() {
        return "cmyk";
    }
    if b.gray_source().is_some() {
        return "gray";
    }
    let readable =
        image::ImageReader::new(std::io::Cursor::new(b.bytes.as_slice())).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok());
    if readable.is_some() { "rgb" } else { "unknown" }
}

#[cfg(test)]
thread_local! {
    /// How many images [`rgba_pixels`] has started to decode on this thread.
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The pixels of image `b` as straight RGBA, when it is `w` × `h` pixels and they fit `max`
/// characters of base64; the size is checked, and read from the header, before anything is decoded.
/// `charged` gets the characters of base64 an image that reaches the decoder takes from the total,
/// which it takes even when decoding fails.
fn rgba_pixels(b: &ImageBlob, w: u32, h: u32, max: usize, charged: &mut usize) -> Option<Vec<u8>> {
    let bytes = u64::from(w).checked_mul(u64::from(h))?.checked_mul(4)?;
    let cost = bytes.div_ceil(3).saturating_mul(4);
    if bytes == 0 || cost > max as u64 || oversized_tiff(&b.bytes).is_some() {
        return None;
    }
    let cmyk = b.is_cmyk();
    let reader = || image::ImageReader::new(std::io::Cursor::new(b.bytes.as_slice())).with_guessed_format().ok();
    let size = if cmyk { b.header_size()? } else { reader()?.into_dimensions().ok()? };
    if size != (w, h) {
        return None;
    }
    *charged = cost as usize;
    #[cfg(test)]
    DECODES.with(|n| n.set(n.get() + 1));
    if cmyk {
        return cmyk_rgba(b, w, h);
    }
    let mut r = reader()?;
    let mut limits = image::Limits::default();
    // The decoder's own buffers besides the RGBA it gives.
    limits.max_alloc = Some(64 << 20);
    r.limits(limits);
    let img = r.decode().ok()?.into_rgba8();
    (img.dimensions() == (w, h)).then(|| img.into_raw())
}

/// The inks → XYZ table of [`cmyk_rgba`] and the color settings it was built with.
static CMYK_DISPLAY: Mutex<Option<(Arc<Cms>, Arc<CmykLut>)>> = Mutex::new(None);

/// The pixels of CMYK image `b` as straight RGBA, when it is `w` × `h` pixels: its inks shown
/// through the working CMYK profile, by a table built as `vectorcraft-render`'s `InkLut` builds its
/// own and kept for the active color settings.
fn cmyk_rgba(b: &ImageBlob, w: u32, h: u32) -> Option<Vec<u8>> {
    let inks = b.cmyk().filter(|i| (i.width, i.height) == (w, h))?;
    let lut = {
        let cms = cms::active();
        let mut slot = CMYK_DISPLAY.lock().unwrap_or_else(|e| e.into_inner());
        match &*slot {
            Some((owner, lut)) if Arc::ptr_eq(owner, &cms) => lut.clone(),
            _ => {
                let lut = Arc::new(CmykLut::build(|c| {
                    let [x, y, z] = lab::lab_to_xyz(cms.cmyk_display_lab(c));
                    [x, y, z, 0.0]
                }));
                *slot = Some((cms, lut.clone()));
                lut
            }
        }
    };
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut out = Vec::with_capacity(inks.data.len());
    // Runs of one color convert once.
    let mut last: Option<([u8; 4], [u8; 4])> = None;
    for p in inks.data.as_chunks::<4>().0 {
        let rgba = match last {
            Some((k, v)) if k == *p => v,
            _ => {
                let [x, y, z, _] = lut.apply(p.map(|v| f32::from(v) / 255.0));
                let [r, g, b] = lab::xyz_to_srgb([x, y, z]).map(q);
                last = Some((*p, [r, g, b, 255]));
                [r, g, b, 255]
            }
        };
        out.extend_from_slice(&rgba);
    }
    Some(out)
}

pub(crate) fn out_err(at: &str, msg: impl std::fmt::Display) -> Error {
    Error::Output(format!("{at}: {msg}"))
}

pub(crate) fn check_point(p: Point, at: &str) -> Result<()> {
    if p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD {
        Ok(())
    } else {
        Err(out_err(at, format!("coordinate out of range (limit ±{MAX_COORD} pt)")))
    }
}

/// A path from plug-in output: native `PathData` JSON, every coordinate finite and in range, empty
/// subpaths dropped; `anchors` counts towards [`MAX_ANCHORS`].
pub(crate) fn decode_path(v: &Value, at: &str, anchors: &mut usize) -> Result<PathData> {
    let mut path: PathData = serde_json::from_value(v.clone()).map_err(|e| out_err(at, e))?;
    path.subpaths.retain(|sp| !sp.anchors.is_empty());
    *anchors = anchors.saturating_add(path.anchor_count());
    if *anchors > MAX_ANCHORS {
        return Err(out_err(at, format!("too many anchor points (limit {MAX_ANCHORS})")));
    }
    for (_, _, a) in path.anchors() {
        for p in [a.p, a.h_in, a.h_out] {
            check_point(p, at)?;
        }
    }
    if path.is_empty() {
        return Err(out_err(at, "the path has no anchor points"));
    }
    Ok(path)
}

fn unit(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// `c` with its components in range.
pub(crate) fn clamp_color(c: Color) -> Color {
    match c {
        Color::Rgb { r, g, b } => Color::Rgb { r: unit(r), g: unit(g), b: unit(b) },
        Color::Cmyk { c, m, y, k } => Color::Cmyk { c: unit(c), m: unit(m), y: unit(y), k: unit(k) },
        Color::Gray { k } => Color::Gray { k: unit(k) },
        Color::Lab { l, a, b } => Color::Lab { l: l.clamp(0.0, 100.0), a: a.clamp(-128.0, 127.0), b: b.clamp(-128.0, 127.0) },
    }
}

/// A paint from plug-in output (`null`: none), with colours, opacities and offsets clamped and
/// gradient and pattern placements checked.
fn decode_paint(v: &Value, at: &str) -> Result<Paint> {
    if v.is_null() {
        return Ok(Paint::None);
    }
    let mut p: Paint = serde_json::from_value(v.clone()).map_err(|e| out_err(at, e))?;
    match &mut p {
        Paint::None => {}
        Paint::Solid { color, tint, .. } => {
            *color = clamp_color(*color);
            *tint = unit(*tint);
        }
        Paint::Gradient(g) => {
            let stops = &mut g.gradient.stops;
            if stops.is_empty() || stops.len() > MAX_ITEMS {
                return Err(out_err(at, format!("a gradient needs 1-{MAX_ITEMS} stops")));
            }
            for s in stops.iter_mut() {
                s.offset = unit(s.offset);
                s.opacity = unit(s.opacity);
                s.midpoint = s.midpoint.clamp(0.13, 0.87);
                s.tint = unit(s.tint);
                s.color = clamp_color(s.color);
            }
            stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            g.angle = g.angle.rem_euclid(360.0);
            if let Some(geom) = &mut g.geom {
                check_point(geom.start, at)?;
                check_point(geom.end, at)?;
                if let Some(f) = geom.focal {
                    check_point(f, at)?;
                }
                geom.aspect = if geom.aspect.is_finite() { geom.aspect.clamp(0.01, 100.0) } else { 1.0 };
            }
            if let Some(ff) = &mut g.freeform {
                if ff.points.len() > MAX_ITEMS || ff.lines.len() > MAX_ITEMS || ff.lines.iter().any(|l| l.len() > MAX_ITEMS) {
                    return Err(out_err(at, format!("a freeform gradient holds at most {MAX_ITEMS} points and lines")));
                }
                let n = ff.points.len();
                if ff.lines.iter().flatten().any(|i| *i >= n) {
                    return Err(out_err(at, "a freeform gradient line names a point it doesn't have"));
                }
                for pt in &mut ff.points {
                    check_point(pt.at, at)?;
                    pt.color = clamp_color(pt.color);
                    pt.opacity = unit(pt.opacity);
                    pt.spread = unit(pt.spread);
                }
            }
        }
        Paint::Pattern { xf, .. } => {
            if !xf.as_coeffs().iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD) {
                return Err(out_err(at, "pattern transform out of range"));
            }
        }
    }
    Ok(p)
}

fn opt<'a>(o: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Value> {
    o.get(key).filter(|v| !v.is_null())
}

/// Object `v` of a plug-in's output (`at` names it in errors), its anchors added to `anchors` and
/// held to [`MAX_ANCHORS`]: each key it has checked, numbers clamped into range, as [`decode`] reads
/// them.
fn decode_object(v: &Value, at: &str, anchors: &mut usize) -> Result<OutObject> {
    let o = v.as_object().ok_or_else(|| out_err(at, "an object must be a JSON object"))?;
    let mut out = OutObject::default();
    if let Some(id) = opt(o, "id") {
        out.id = Some(NodeId(id.as_u64().ok_or_else(|| out_err(at, "`id` must be the id of an input object"))?));
    }
    if let Some(t) = opt(o, "type") {
        out.compound = Some(match t.as_str() {
            Some("path") => false,
            Some("compound") => true,
            _ => return Err(out_err(at, "`type` must be \"path\" or \"compound\"")),
        });
    }
    if let Some(n) = o.get("name") {
        let name = match n {
            Value::Null => String::new(),
            n => n.as_str().ok_or_else(|| out_err(at, "`name` must be a string"))?.chars().filter(|c| !c.is_control()).take(256).collect(),
        };
        out.name = Some(name);
    }
    if let Some(p) = opt(o, "path") {
        out.path = Some(decode_path(p, &format!("{at}.path"), anchors)?);
    }
    if let Some(r) = opt(o, "fillRule") {
        out.rule = Some(match r.as_str().map(str::to_ascii_lowercase).as_deref() {
            Some("nonzero") => FillRule::NonZero,
            Some("evenodd") => FillRule::EvenOdd,
            _ => return Err(out_err(at, "`fillRule` must be \"nonzero\" or \"evenodd\"")),
        });
    }
    let list = |key: &str| -> Result<Option<&Vec<Value>>> {
        match opt(o, key) {
            None => Ok(None),
            Some(Value::Array(a)) if a.len() <= MAX_ITEMS => Ok(Some(a)),
            Some(_) => Err(out_err(at, format!("`{key}` must be an array of at most {MAX_ITEMS} entries"))),
        }
    };
    if let Some(fills) = list("fills")? {
        out.fills = Some(fills.iter().enumerate().map(|(i, f)| decode_paint(f, &format!("{at}.fills[{i}]"))).collect::<Result<_>>()?);
    }
    if let Some(strokes) = list("strokes")? {
        out.strokes = Some(strokes.iter().enumerate().map(|(i, s)| decode_stroke(s, &format!("{at}.strokes[{i}]"))).collect::<Result<_>>()?);
    }
    if let Some(effects) = list("effects")? {
        out.effects = Some(effects.iter().enumerate().map(|(i, e)| decode_effect(e, &format!("{at}.effects[{i}]"))).collect::<Result<_>>()?);
    }
    if let Some(op) = opt(o, "opacity") {
        out.opacity = Some(unit(op.as_f64().ok_or_else(|| out_err(at, "`opacity` must be a number"))? as f32));
    }
    let flag = |key: &str| -> Result<Option<bool>> {
        opt(o, key).map(|v| v.as_bool().ok_or_else(|| out_err(at, format!("`{key}` must be true or false")))).transpose()
    };
    out.guide = flag("guide")?;
    out.locked = flag("locked")?;
    Ok(out)
}

/// A stroke from plug-in output: `{paint, width?, join?, cap?, miterLimit?}`, its weight clamped
/// to 0–[`MAX_STROKE_WIDTH`] and its miter limit into [`MITER_LIMIT`].
fn decode_stroke(v: &Value, at: &str) -> Result<OutStroke> {
    let s = v.as_object().ok_or_else(|| out_err(at, "a stroke must be {\"paint\", \"width\"?, \"join\"?, \"cap\"?, \"miterLimit\"?}"))?;
    let number = |key: &str, range: std::ops::RangeInclusive<f64>| -> Result<Option<f64>> {
        let Some(v) = opt(s, key) else { return Ok(None) };
        let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| out_err(at, format!("`{key}` must be a number")))?;
        Ok(Some(x.clamp(*range.start(), *range.end())))
    };
    let join = match opt(s, "join").map(|j| j.as_str()) {
        None => None,
        Some(Some("miter")) => Some(LineJoin::Miter),
        Some(Some("round")) => Some(LineJoin::Round),
        Some(Some("bevel")) => Some(LineJoin::Bevel),
        Some(_) => return Err(out_err(at, "`join` must be \"miter\", \"round\" or \"bevel\"")),
    };
    let cap = match opt(s, "cap").map(|c| c.as_str()) {
        None => None,
        Some(Some("butt")) => Some(LineCap::Butt),
        Some(Some("round")) => Some(LineCap::Round),
        Some(Some("square")) => Some(LineCap::Square),
        Some(_) => return Err(out_err(at, "`cap` must be \"butt\", \"round\" or \"square\"")),
    };
    Ok(OutStroke {
        paint: decode_paint(s.get("paint").unwrap_or(&Value::Null), at)?,
        width: number("width", 0.0..=MAX_STROKE_WIDTH)?,
        join,
        cap,
        miter_limit: number("miterLimit", MITER_LIMIT)?,
    })
}

/// An effect record from plug-in output: `{id, params?, visible?}`, in form only (the host says
/// whether the effect exists and takes the parameters). `params` stays as given, so a record
/// handed back unchanged equals the object's own.
fn decode_effect(v: &Value, at: &str) -> Result<Effect> {
    let e = v.as_object().ok_or_else(|| out_err(at, "an effect must be {\"id\", \"params\"?, \"visible\"?}"))?;
    let id = e.get("id").and_then(Value::as_str).filter(|id| (1..=MAX_EFFECT_ID).contains(&id.len()) && !id.chars().any(char::is_control));
    let id = id.ok_or_else(|| out_err(at, format!("`id` must be an effect id of 1-{MAX_EFFECT_ID} bytes")))?;
    let params = match e.get("params") {
        None => Value::Null,
        Some(p @ (Value::Null | Value::Object(_))) => p.clone(),
        Some(_) => return Err(out_err(at, "`params` must be an object")),
    };
    let visible = match e.get("visible") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(out_err(at, "`visible` must be true or false")),
    };
    Ok(Effect { id: id.to_string(), params, visible })
}

/// What a filter returned: its objects, and what it keeps with the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub objects: Vec<OutObject>,
    /// `documentData`, as `Document::set_plugin_data` takes it: `None` when the output has none
    /// (what the plug-in keeps stays as it is), `Some(None)` for `null` (it forgets it), else what
    /// it keeps from now on, at most [`MAX_DOCUMENT_DATA`] bytes of JSON.
    pub document_data: Option<Option<Value>>,
}

/// Parses a filter's output: `{"objects": [...], "documentData"?: …}`, or `{"error": "message"}`
/// when the plug-in declined (the message reaches the user).
pub fn decode_output(bytes: &[u8]) -> Result<Output> {
    decode_output_with(bytes, &[], &HashMap::new())
}

/// [`decode_output`] for a filter given `read_only` objects, at those places in `input`
/// ([`encode_input_with`]): it must list each of them exactly once, as `{"id": …}` alone or exactly
/// as it was given (numbers compared by value), and such an entry is [`OutObject::kept`]. Any other
/// entry for one of them, and one left out, is refused naming the object and the first field that
/// differs, before anything changes. Each object is parsed from `input` only to compare it, one at
/// a time, and an object that can't be read from there is an error.
pub fn decode_output_with(bytes: &[u8], input: &[u8], read_only: &ReadOnlySpans) -> Result<Output> {
    let mut v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    let objects = decode_objects(&v, input, read_only)?;
    let document_data = match v.get_mut("documentData").map(Value::take) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(data) => {
            let size = serde_json::to_vec(&data).map_or(usize::MAX, |b| b.len());
            if size > MAX_DOCUMENT_DATA {
                return Err(Error::Output(format!("`documentData` is {size} bytes (limit {MAX_DOCUMENT_DATA})")));
            }
            Some(Some(data))
        }
    };
    Ok(Output { objects, document_data })
}

/// Parses a plug-in's objects: `{"objects": [...]}`, or `{"error": "message"}` when the plug-in
/// declined (the message reaches the user). A live effect's other keys don't matter.
pub fn decode(bytes: &[u8]) -> Result<Vec<OutObject>> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    decode_objects(&v, &[], &HashMap::new())
}

/// The objects of output `v`, or its decline; entries for `read_only` objects as
/// [`decode_output_with`] reads them.
fn decode_objects(v: &Value, input: &[u8], read_only: &ReadOnlySpans) -> Result<Vec<OutObject>> {
    if let Some(e) = declined(v) {
        return Err(e);
    }
    let objects = v.get("objects").and_then(Value::as_array).ok_or_else(|| Error::Output("expected {\"objects\": [...]}".into()))?;
    if objects.len() > MAX_OBJECTS {
        return Err(Error::Output(format!("{} objects (limit {MAX_OBJECTS})", objects.len())));
    }
    let mut anchors = 0;
    let mut returned: HashSet<NodeId> = HashSet::new();
    let out = objects
        .iter()
        .enumerate()
        .map(|(i, o)| match output_id(o).and_then(|id| Some((id, read_only.get(&id)?))) {
            Some((id, span)) => {
                let at = format!("objects[{i}] (id {})", id.0);
                if !returned.insert(id) {
                    return Err(out_err(&at, "is listed twice"));
                }
                // `{"id": …}` alone keeps the object, so there is nothing to compare it with.
                if o.as_object().is_none_or(|m| m.len() != 1) {
                    let given = serde_json::from_slice(input.get(span.clone()).unwrap_or_default()).map_err(|e| Error::Io(e.to_string()))?;
                    check_unchanged(o, &given, &at)?;
                }
                Ok(OutObject { id: Some(id), kept: true, ..OutObject::default() })
            }
            None => decode_object(o, &format!("objects[{i}]"), &mut anchors),
        })
        .collect::<Result<Vec<_>>>()?;
    let mut missing: Vec<NodeId> = read_only.keys().filter(|id| !returned.contains(id)).copied().collect();
    missing.sort();
    if let Some(id) = missing.first() {
        return Err(out_err(
            &format!("objects (id {})", id.0),
            "a read-only object was left out: return it, as {\"id\": …} alone or as it was given",
        ));
    }
    Ok(out)
}

/// The id entry `o` gives, a whole number written either way (`17` or `17.0`).
fn output_id(o: &Value) -> Option<NodeId> {
    let id = o.get("id")?;
    id.as_u64().or_else(|| id.as_f64().filter(|f| f.fract() == 0.0 && (0.0..=9_007_199_254_740_992.0).contains(f)).map(|f| f as u64)).map(NodeId)
}

/// Checks entry `o` of a filter's output for a read-only object it was `given`: `given` itself,
/// numbers compared by value. Otherwise the error names the first field that differs
/// (`text.runs[0].text`): a field the entry gives before one it leaves out, shown as
/// [`crate::permissions::display_text`] shows a manifest string, since the entry's keys are the
/// plug-in's.
fn check_unchanged(o: &Value, given: &Value, at: &str) -> Result<()> {
    let changed = match (o, given) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().collect();
            keys.sort();
            keys.into_iter().find_map(|k| match y.get(k) {
                Some(q) => first_difference(&x[k], q, k),
                None => Some(k.clone()),
            })
        }
        _ => None,
    };
    match changed.or_else(|| first_difference(o, given, "")) {
        None => Ok(()),
        Some(field) => {
            let field = crate::permissions::display_text(&field, crate::permissions::MAX_SHOWN_FIELD);
            Err(out_err(at, format!("read-only field '{field}' changed")))
        }
    }
}

/// The path of the first place `a` and `b` differ (`""`: they differ as a whole), `None` when they
/// are the same JSON with numbers compared by value.
fn first_difference(a: &Value, b: &Value, at: &str) -> Option<String> {
    let key = |k: &str| if at.is_empty() { k.to_string() } else { format!("{at}.{k}") };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64() != y.as_f64()).then(|| at.into()),
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|k| match (x.get(k), y.get(k)) {
                (Some(p), Some(q)) => first_difference(p, q, &key(k)),
                _ => Some(key(k)),
            })
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(at.into());
            }
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| first_difference(p, q, &format!("{at}[{i}]")))
        }
        (a, b) => (a != b).then(|| at.into()),
    }
}

/// The decline in a plug-in's output `v`, `{"error": "…"}`: its message as the user reads it,
/// without control characters and at most 300 of them.
pub(crate) fn declined(v: &Value) -> Option<Error> {
    let e = v.get("error").filter(|e| !e.is_null())?;
    Some(Error::Failed(printable(e, 300)))
}

/// A plug-in's text the user reads: `v` as a string (any other JSON as its text), without control
/// characters, at most `max` characters of it.
pub(crate) fn printable(v: &Value, max: usize) -> String {
    let s = v.as_str().map_or_else(|| v.to_string(), str::to_string);
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// The paints of `ap`'s fills (`fill`) or strokes, in paint order.
fn paints(ap: &Appearance, fill: bool) -> Vec<&Paint> {
    ap.items.iter().filter(|i| i.is_fill() == fill).map(AppearanceItem::paint).collect()
}

/// `p` without swatch links: a plug-in's colours are its own, so only paints it returned
/// unchanged keep theirs.
fn unlinked(mut p: Paint) -> Paint {
    match &mut p {
        Paint::Solid { swatch, tint, .. } => {
            *swatch = None;
            *tint = 1.0;
        }
        Paint::Gradient(g) => {
            g.swatch = None;
            for s in &mut g.gradient.stops {
                s.swatch = None;
                s.tint = 1.0;
            }
        }
        Paint::None | Paint::Pattern { .. } => {}
    }
    p
}

/// `p` as it may enter `doc`: unchanged from `was` (the paint in the same slot before), or
/// without swatch links; a pattern must exist in the document.
fn admit_paint(doc: &Document, p: Paint, was: Option<&Paint>, at: &str) -> Result<Paint> {
    if let Paint::Pattern { pattern, .. } = &p
        && doc.pattern(pattern).is_none()
    {
        return Err(out_err(at, format!("the document has no pattern {pattern:?}")));
    }
    Ok(if was == Some(&p) { p } else { unlinked(p) })
}

/// Sets the paints of `ap`'s fills (`fill`) or strokes to `new` (a fill keeps everything but its
/// paint), in paint order: extra items are removed (from the top), missing ones added above the
/// last item of their kind (fills at the bottom, strokes on top when the object has none).
pub(crate) fn set_items(ap: &mut Appearance, fill: bool, new: Vec<OutStroke>) {
    let idx: Vec<usize> = ap.items.iter().enumerate().filter(|(_, i)| i.is_fill() == fill).map(|(k, _)| k).collect();
    for &i in idx.iter().skip(new.len()).rev() {
        ap.remove_item(i);
    }
    let kept = new.len().min(idx.len());
    let mut at = idx.get(..kept).and_then(<[usize]>::last).map_or(if fill { 0 } else { ap.items.len() }, |i| i + 1);
    for (k, o) in new.into_iter().enumerate() {
        match idx.get(k).and_then(|&i| ap.items.get_mut(i)) {
            Some(AppearanceItem::Fill(f)) => f.paint = o.paint,
            Some(AppearanceItem::Stroke(s)) => style(s, o),
            None => {
                let item = if fill {
                    AppearanceItem::Fill(FillLayer::new(o.paint))
                } else {
                    let mut s = StrokeLayer::new(Paint::None, 1.0);
                    style(&mut s, o);
                    AppearanceItem::Stroke(s)
                };
                ap.insert_item(at, item);
                at += 1;
            }
        }
    }
}

/// Gives stroke `s` the paint of `o` and each of its weight, join, cap and miter limit it has.
pub(crate) fn style(s: &mut StrokeLayer, o: OutStroke) {
    s.paint = o.paint;
    s.width = o.width.unwrap_or(s.width);
    s.join = o.join.unwrap_or(s.join);
    s.cap = o.cap.unwrap_or(s.cap);
    s.miter_limit = o.miter_limit.unwrap_or(s.miter_limit);
}

/// A compound path's members: one path per subpath of `path`, keeping the ids of `old` members.
fn members(doc: &mut Document, old: &[Arc<Node>], path: PathData) -> Vec<Arc<Node>> {
    path.subpaths
        .into_iter()
        .enumerate()
        .map(|(i, sp)| {
            let id = old.get(i).map_or_else(|| doc.alloc_id(), |c| c.id);
            Arc::new(Node::path(id, PathData::single(sp), Appearance::default()))
        })
        .collect()
}

/// Writes `o` into `n` (an input object, or a fresh node for a new one); its effect records once
/// `admit` lets them in.
fn write(doc: &mut Document, n: &mut Node, o: OutObject, at: &str, admit: AdmitEffects) -> Result<()> {
    let (cur_path, cur_rule, cur_compound) = geometry(n).unwrap_or((PathData::default(), FillRule::NonZero, false));
    let compound = o.compound.unwrap_or(cur_compound);
    let rule = o.rule.unwrap_or(cur_rule);
    match (&mut n.kind, compound) {
        (NodeKind::Path { path, rule: r, live, .. }, false) => {
            // A path handed back as it was stays the live shape it was.
            if let Some(p) = o.path.filter(|p| p != path) {
                *path = p;
                *live = None;
            }
            *r = rule;
        }
        (NodeKind::Compound { children, rule: r }, true) => {
            if let Some(p) = o.path {
                *children = members(doc, children, p);
            }
            *r = rule;
        }
        (NodeKind::Compound { .. }, false) => {
            n.kind = NodeKind::Path { path: o.path.unwrap_or(cur_path), rule, live: None, clipping: false, guide: false };
        }
        (_, true) => n.kind = NodeKind::Compound { children: members(doc, &[], o.path.unwrap_or(cur_path)), rule },
        (_, false) => n.kind = NodeKind::Path { path: o.path.unwrap_or(cur_path), rule, live: None, clipping: false, guide: false },
    }
    for (fill, new) in [(true, o.fills.map(|f| f.into_iter().map(OutStroke::paint).collect::<Vec<_>>())), (false, o.strokes)] {
        let Some(new) = new else { continue };
        let was = paints(&n.appearance, fill);
        let admitted = new
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let at = format!("{at}.{}[{i}]", if fill { "fills" } else { "strokes" });
                Ok(OutStroke { paint: admit_paint(doc, s.paint.clone(), was.get(i).copied(), &at)?, ..s })
            })
            .collect::<Result<Vec<_>>>()?;
        set_items(&mut n.appearance, fill, admitted);
    }
    if let Some(op) = o.opacity {
        n.opacity = op;
    }
    if let Some(name) = o.name {
        n.name = Some(name).filter(|s| !s.trim().is_empty());
    }
    if let Some(effects) = o.effects {
        n.appearance.effects = admit(&effects, n).map_err(|why| out_err(&format!("{at}.effects"), why))?;
    }
    if let Some(on) = o.guide {
        set_guide(n, on);
    }
    if let Some(locked) = o.locked {
        n.locked = locked;
    }
    Ok(())
}

/// Makes path or compound path `n` a guide or a plain path, as Make Guides and Release Guides do: a
/// compound path's members carry the flag.
fn set_guide(n: &mut Node, on: bool) {
    match &mut n.kind {
        NodeKind::Path { guide, clipping: false, .. } => *guide = on,
        NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                set_guide(Arc::make_mut(c), on);
            }
        }
        _ => {}
    }
}

/// Whether `n` is a guide: a guide path, or a compound path of guide paths.
fn is_guide(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Path { guide, .. } => *guide,
        NodeKind::Compound { children, .. } => !children.is_empty() && children.iter().all(|c| is_guide(c)),
        _ => false,
    }
}

/// Whether a filter's result `n` is selected after the run: locked objects and guides aren't, as
/// Object › Lock and Make Guides leave them deselected.
pub fn selectable(n: &Node) -> bool {
    !n.locked && !is_guide(n)
}

/// The paths and compound paths a filter reads from the selection `selected`, in paint order:
/// selected ones, and those inside selected groups and layers; hidden, locked, guide (compound
/// guides too) and clipping paths are left out, and so are compound paths that clip a clip group
/// and paths inside a group that clips one (and paths inside compound paths, blends and other
/// live objects).
pub fn filter_targets(doc: &Document, selected: &[NodeId]) -> Vec<NodeId> {
    filter_targets_with(doc, selected, &[])
}

/// [`filter_targets`] for a filter that asks for `kinds` too: type, images and symbol instances of
/// those kinds join the paths in paint order, under the same rules (objects that clip a clip
/// group are left out, inside a group that clips one too, and so is anything inside a symbol, a
/// placed document or a live object).
pub fn filter_targets_with(doc: &Document, selected: &[NodeId], kinds: &[InputKind]) -> Vec<NodeId> {
    /// `source`: `list` is inside the clipping object of a clip group.
    fn visit(list: &[Arc<Node>], sel: &HashSet<NodeId>, inside: bool, clips: bool, source: bool, kinds: &[InputKind], out: &mut Vec<NodeId>) {
        for (i, n) in list.iter().enumerate() {
            if !n.visible || n.locked {
                continue;
            }
            let picked = inside || sel.contains(&n.id);
            let clipping = source || (clips && i == 0);
            match &n.kind {
                NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. } if picked && !clipping && !is_guide(n) => {
                    out.push(n.id)
                }
                NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => visit(children, sel, picked, n.clips(), clipping, kinds, out),
                _ if picked && !clipping && input_kind(n).is_some_and(|k| kinds.contains(&k)) => out.push(n.id),
                _ => {}
            }
        }
    }
    let sel: HashSet<NodeId> = selected.iter().copied().collect();
    let mut out = vec![];
    visit(&doc.layers, &sel, false, false, false, kinds, &mut out);
    out
}

/// The tree edits of [`apply_output`], made in one pass over the document.
#[derive(Default)]
struct Plan {
    replace: HashMap<NodeId, Node>,
    delete: HashSet<NodeId>,
    before: HashMap<NodeId, Vec<Node>>,
    after: HashMap<NodeId, Vec<Node>>,
    /// New objects that go on top of a container (`None`: the top-level list).
    top: Option<(Option<NodeId>, Vec<Node>)>,
}

impl Plan {
    fn pending(&self) -> bool {
        !(self.replace.is_empty() && self.delete.is_empty() && self.before.is_empty() && self.after.is_empty() && self.top.is_none())
    }

    /// The edited `list` (the children of `parent`), `None` when nothing in it changed.
    fn list(&mut self, parent: Option<NodeId>, list: &[Arc<Node>]) -> Option<Vec<Arc<Node>>> {
        let mut out: Option<Vec<Arc<Node>>> = None;
        for (i, c) in list.iter().enumerate() {
            if !self.pending() {
                if let Some(o) = &mut out {
                    o.extend(list.iter().skip(i).cloned());
                }
                return out;
            }
            let mut emitted: Vec<Arc<Node>> = self.before.remove(&c.id).unwrap_or_default().into_iter().map(Arc::new).collect();
            let mut changed = !emitted.is_empty();
            if self.delete.remove(&c.id) {
                changed = true;
            } else if let Some(r) = self.replace.remove(&c.id) {
                emitted.push(Arc::new(r));
                changed = true;
            } else if let Some(ch) = match &c.kind {
                NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => self.list(Some(c.id), children),
                _ => None,
            } {
                let mut m = (**c).clone();
                if let Some(slot) = m.children_mut() {
                    *slot = ch;
                }
                emitted.push(Arc::new(m));
                changed = true;
            } else {
                emitted.push(c.clone());
            }
            if let Some(a) = self.after.remove(&c.id) {
                emitted.extend(a.into_iter().map(Arc::new));
                changed = true;
            }
            if changed && out.is_none() {
                out = Some(list.iter().take(i).cloned().collect());
            }
            if let Some(o) = &mut out {
                o.extend(emitted);
            }
        }
        if self.top.as_ref().is_some_and(|(p, _)| *p == parent)
            && let Some((_, nodes)) = self.top.take()
        {
            out.get_or_insert_with(|| list.to_vec()).extend(nodes.into_iter().map(Arc::new));
        }
        out
    }
}

/// Applies an object filter's output to `doc`.
///
/// `inputs` are the objects the plug-in was given, in paint order ([`filter_targets`]). Output
/// objects with an `id` update that input object (keys they leave out keep their values); objects
/// without one are new and go just above the object listed before them, or, listed before any
/// input object, just below the first one listed after them (at the first input object's place
/// when the output keeps none; on top of `parent` when there were no inputs). Input objects missing
/// from the output are deleted. An object's effect records replace its own once `admit` lets them
/// in. Returns the resulting objects in output order (to select).
pub fn apply_output(doc: &mut Document, inputs: &[NodeId], out: Vec<OutObject>, parent: Option<NodeId>, admit: AdmitEffects) -> Result<Vec<NodeId>> {
    let wanted: HashSet<NodeId> = inputs.iter().copied().collect();
    let mut originals: HashMap<NodeId, Node> = HashMap::new();
    doc.walk(|n| {
        if wanted.contains(&n.id) {
            originals.insert(n.id, n.clone());
        }
    });
    let mut plan = Plan::default();
    let mut result = Vec::with_capacity(out.len());
    // The run of new objects since the last input object listed, and that object.
    let mut run: Vec<Node> = vec![];
    let mut prev: Option<NodeId> = None;
    for (i, o) in out.into_iter().enumerate() {
        let at = format!("objects[{i}]");
        match o.id {
            Some(id) => {
                let mut n = originals.remove(&id).ok_or_else(|| {
                    let why = if wanted.contains(&id) { "is listed twice" } else { "is not the id of an input object" };
                    out_err(&at, format!("{} {why}", id.0))
                })?;
                // Type, images and symbol instances are only read: writing one would make it a path.
                if input_kind(&n).is_some() && !o.kept {
                    return Err(out_err(&at, format!("{} is read-only", id.0)));
                }
                let kept = o.kept;
                if !kept {
                    write(doc, &mut n, o, &at, admit)?;
                }
                if !run.is_empty() {
                    let nodes = std::mem::take(&mut run);
                    match prev {
                        Some(p) => plan.after.insert(p, nodes),
                        None => plan.before.insert(id, nodes),
                    };
                }
                if !kept {
                    plan.replace.insert(id, n);
                }
                prev = Some(id);
                result.push(id);
            }
            None => {
                if o.path.is_none() {
                    return Err(out_err(&at, "a new object needs a `path`"));
                }
                let mut n = Node::path(doc.alloc_id(), PathData::default(), Appearance::default());
                write(doc, &mut n, o, &at, admit)?;
                result.push(n.id);
                run.push(n);
            }
        }
    }
    if !run.is_empty() {
        match prev.or(inputs.first().copied()) {
            Some(p) if prev.is_some() => {
                plan.after.insert(p, run);
            }
            Some(first) => {
                plan.before.insert(first, run);
            }
            None => plan.top = Some((parent, run)),
        }
    }
    // Input objects the output left out are deleted, except read-only ones, which it must return.
    if let Some(n) = originals.values().filter(|n| input_kind(n).is_some()).min_by_key(|n| n.id) {
        return Err(out_err(&format!("objects (id {})", n.id.0), "a read-only object was left out"));
    }
    plan.delete = originals.into_keys().collect();
    if let Some(list) = plan.list(None, &doc.layers) {
        doc.layers = list;
    }
    if plan.pending() {
        return Err(Error::Output("the objects to change are no longer in the document".into()));
    }
    Ok(result)
}

/// The geometry of an effect's output: every returned object's path, in order.
pub fn output_geometry(objects: Vec<OutObject>) -> PathData {
    PathData::new(objects.into_iter().filter_map(|o| o.path).flat_map(|p| p.subpaths).collect())
}

#[cfg(test)]
mod tests;
