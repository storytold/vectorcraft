//! Save As › `.ai` with `aiVersion: "7"`: legacy `.ai` files of version 7 (the PostScript-form `.ai`
//! that `vectorcraft_eps::export_legacy_ai` writes), which keep the layers, groups, compound
//! paths and clipping groups that a PDF-compatible `.ai` loses in apps reading only its PDF part
//! (#868).
//!
//! The format has one artboard: each chosen artboard (default all) is a file of its own, holding
//! the art that touches it, named `{stem}-{artboard}.ai` beside the path when there are several.
//! Appearances the format can't hold (several fills or strokes, effects, brushes) are expanded into
//! objects. Transparency isn't flattened: flattening merges the art that overlaps transparent
//! objects, across layers, into one flattened result, which empties the layers under them, and
//! editable layers are what this format is saved for. Transparent objects are written opaque (a
//! warning says so).
//!
//! Images are the exception (`transparentImages`): an image with see-through pixels, opacity or a
//! blending mode written opaque is a box hiding the art under it (version 7 images have no alpha
//! channel, §8.1). With `flatten` (the default) each becomes an opaque image of itself and the art
//! under it, rendered from the document as it is, clipped to the area where it changes what shows;
//! the art under it stays as it was. With `omit` they are left out. An image that can't be decoded
//! is missing from the images that flattening and expanding raster effects draw, and the warnings
//! then carry the raster exports' note once.
//!
//! The format can't hide objects, only layers (`hiddenObjects`): with `layer` (the default) each
//! layer's hidden objects go to a hidden layer of their own just above it, with `omit` they are
//! left out, with `visible` they show where they are. Type is written as text the format's reader
//! lays out again, in the fonts it names; `outlineText` writes its outlines instead (as type that
//! 8-bit text can't hold always is).

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Anchor, FillRule, PathData, Point, Rect, SubPath};

use super::super::*;
use super::{ArtboardPick, Encoded, Format};

/// Does `p` ask a save in `f` for a legacy `.ai` file of version 7 (`aiVersion: "7"`)? An unknown
/// version is an error, and so is version 7 for a `.ait` template, which is PDF-compatible only.
pub(super) fn wanted(cmd: &str, f: &Format, p: &Value) -> Result<bool> {
    let legacy = match p.get("aiVersion") {
        None | Some(Value::Null) => false,
        Some(Value::String(v)) if v.trim().is_empty() || v.eq_ignore_ascii_case("pdf") => false,
        Some(Value::String(v)) if v.trim() == "7" => true,
        Some(Value::Number(n)) if n.as_f64() == Some(7.0) => true,
        Some(v) => return Err(bad(cmd, format!("aiVersion {v}: \"pdf\" (PDF-compatible, the default) or \"7\" (legacy .ai, version 7)"))),
    };
    if legacy && f.id == "ait" {
        return Err(bad(cmd, "aiVersion 7 writes legacy .ai files; a .ait template is always PDF-compatible"));
    }
    Ok(legacy)
}

/// What a legacy `.ai` does with the images it can't hold (`transparentImages`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeeThrough {
    Flatten,
    Omit,
}

/// Where a legacy `.ai`'s hidden objects go (`hiddenObjects`): the format hides only layers.
fn hidden_objects(cmd: &str, p: &Value) -> Result<vectorcraft_eps::HiddenObjects> {
    use vectorcraft_eps::HiddenObjects;
    match p.get("hiddenObjects") {
        None | Some(Value::Null) => Ok(HiddenObjects::Layer),
        Some(Value::String(v)) if v.trim().is_empty() || v.trim().eq_ignore_ascii_case("layer") => Ok(HiddenObjects::Layer),
        Some(Value::String(v)) if v.trim().eq_ignore_ascii_case("omit") => Ok(HiddenObjects::Omit),
        Some(Value::String(v)) if v.trim().eq_ignore_ascii_case("visible") => Ok(HiddenObjects::Visible),
        Some(v) => Err(bad(cmd, format!("hiddenObjects {v}: \"layer\" (the default), \"omit\" or \"visible\""))),
    }
}

/// Type as outlines (`outlineText`), not text objects.
fn outline_text(cmd: &str, p: &Value) -> Result<bool> {
    match p.get("outlineText") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(v) => Err(bad(cmd, format!("outlineText {v}: true or false"))),
    }
}

fn see_through(cmd: &str, p: &Value) -> Result<SeeThrough> {
    match p.get("transparentImages") {
        None | Some(Value::Null) => Ok(SeeThrough::Flatten),
        Some(Value::String(v)) if v.trim().is_empty() || v.trim().eq_ignore_ascii_case("flatten") => Ok(SeeThrough::Flatten),
        Some(Value::String(v)) if v.trim().eq_ignore_ascii_case("omit") => Ok(SeeThrough::Omit),
        Some(v) => Err(bad(cmd, format!("transparentImages {v}: \"flatten\" (the default) or \"omit\""))),
    }
}

/// What the format loses against a VectorCraft document, reported with every save.
const FIDELITY: &str = "a legacy .ai (version 7) keeps layers, groups, compound paths, clipping groups, type, colours, gradients and images, not the rest: save as VectorCraft to keep everything editable";
const FLATTENED: &str =
    "transparent images (see-through pixels, opacity or a blending mode) are flattened with the art under them into opaque images";
const OMITTED: &str = "transparent images (see-through pixels, opacity or a blending mode) are left out";
const SEPARATE: &str = "legacy .ai files have one artboard: each artboard is saved to a file of its own";

/// Encode `doc` as legacy `.ai` files of version 7: one per chosen artboard (`artboard`, `artboards` or
/// `range`; default all).
pub(super) fn encode(cmd: &str, f: &Format, doc: &Document, p: &Value) -> Result<Encoded> {
    let n = doc.artboards.len();
    if n == 0 {
        return Err(bad(cmd, "the document has no artboard"));
    }
    let mode = see_through(cmd, p)?;
    let (hidden, outline_text) = (hidden_objects(cmd, p)?, outline_text(cmd, p)?);
    let picked: ArtboardPick = super::encode::options(f, p)?;
    let boards = picked.resolve(n).map_err(|e| bad(cmd, e))?.unwrap_or_else(|| (0..n).collect());
    let mut enc = Encoded { warnings: vec![FIDELITY.to_string()], ..Encoded::default() };
    if boards.len() > 1 {
        enc.warnings.push(SEPARATE.to_string());
    }
    let created = vectorcraft_doc::metadata::output_time();
    let failed = vectorcraft_render::decode_failures();
    for b in boards {
        let one = super::save::artboard_doc(doc, b);
        let (full, placed) = crate::cmd::place::document::full_documents(&one);
        let mut d = full.into_owned();
        expand_appearances(&mut d);
        let images = see_through_images(&d, mode);
        if !images.is_empty() {
            note(&mut enc.warnings, if mode == SeeThrough::Flatten { FLATTENED } else { OMITTED });
            if mode == SeeThrough::Flatten {
                flatten_images(&mut d, &images)?;
            } else {
                for (_, id) in images {
                    let _ = d.remove(id); // Inside an image left out before: gone already.
                }
            }
        }
        let o = vectorcraft_eps::LegacyAiOptions { title: doc.title.clone(), created, hidden, outline_text };
        let out = vectorcraft_eps::export_legacy_ai(&d, &o).map_err(|e| bad(cmd, e))?;
        for w in placed.iter().chain(&out.warnings) {
            note(&mut enc.warnings, w);
        }
        enc.files.push((Some(b), out.bytes));
    }
    // Expanding raster effects and flattening images draw the art, and an image the renderer
    // couldn't decode drew nothing there.
    if let Some(w) = super::super::rasterfx::undecoded_warning(failed) {
        note(&mut enc.warnings, w);
    }
    Ok(enc)
}

/// Expand Appearance of every layer's art whose appearance the format can't hold.
fn expand_appearances(d: &mut Document) {
    let brushes = vectorcraft_brush::library(d);
    let layers: Vec<NodeId> = d.layers.iter().map(|l| l.id).collect();
    let mut out = vec![];
    for id in layers {
        super::super::effectcmd::expand_node(d, id, &brushes, &mut out);
    }
}

/// The images of `d` a legacy `.ai` can't hold, in paint order with their places in it (as
/// [`Document::walk`] counts): those with see-through pixels, opacity or a blending mode. Only
/// visible ones flatten (a hidden one, which the hidden objects option may write, is written over
/// white); all are left out with `omit`.
fn see_through_images(d: &Document, mode: SeeThrough) -> Vec<(usize, NodeId)> {
    let keys = super::super::flatten::see_through_images(d);
    let mut out = vec![];
    let mut at = 0;
    d.walk(|n| {
        if let NodeKind::Image(im) = &n.kind
            && (keys.contains(&im.key) || n.opacity < 1.0 || n.blend != BlendMode::Normal)
        {
            out.push((at, n.id));
        }
        at += 1;
    });
    if mode == SeeThrough::Flatten {
        out.retain(|(_, id)| d.is_visible(*id));
    }
    out
}

/// Replace each of `images` (from [`see_through_images`]) with an opaque image of what shows
/// where it is, clipped to where it changes that; one that changes nothing is left out. Each is
/// rendered from `d` as it was, so images over one another flatten as they look.
fn flatten_images(d: &mut Document, images: &[(usize, NodeId)]) -> Result<()> {
    let src = d.clone();
    for &(at, id) in images {
        let Some((parent, idx, _)) = d.position(id) else { continue };
        let flat = flattened(&src, at, id)?;
        d.remove(id)?;
        if let Some(flat) = flat {
            let png = flat.png;
            let key = super::super::menucmds::unique_key(d, "flattened");
            d.images.insert(key.clone(), ImageBlob::new("image/png", png));
            let image = ImageObject { key, width: flat.width, height: flat.height, xf: flat.xf, link: None, placement: Default::default() };
            let image = Node::new(d.alloc_id(), NodeKind::Image(Box::new(image)));
            let mut group = super::super::rasterfx::clip_group(d, flat.clip, FillRule::NonZero, image);
            group.name = src.node(id).and_then(|n| n.name.clone());
            d.insert(parent, idx, group)?;
        }
    }
    Ok(())
}

/// An image made by [`flattened`]: PNG pixels, their place, and the outline they are clipped to.
struct Flat {
    png: Vec<u8>,
    width: u32,
    height: u32,
    xf: Affine,
    clip: PathData,
}

/// Highest and lowest resolution of a flattened image (pixels per inch); it takes the image's own
/// in between.
const MAX_PPI: f64 = 600.0;
const MIN_PPI: f64 = 72.0;
/// A rendered pixel channel changes when it changes by more than this.
const CHANGED: u8 = 2;
/// The clip outline follows cells of about this many points.
const CELL: f64 = 2.0;

/// The image `id` (the `at`th node in paint order of `src`) flattened: `src` drawn up to and
/// including it, over white, cropped and clipped to the cells where drawing it changes the
/// result. `None` when it changes nothing.
fn flattened(src: &Document, at: usize, id: NodeId) -> Result<Option<Flat>> {
    let (Some(n), Some(bounds)) = (src.node(id), src.bounds_of(&[id], true)) else { return Ok(None) };
    let NodeKind::Image(im) = &n.kind else { return Ok(None) };
    // The image's own resolution (points per pixel from its placement).
    let pt = im.xf.determinant().abs().sqrt();
    let ppi = if pt.is_finite() && pt > 0.0 { (72.0 / pt).clamp(MIN_PPI, MAX_PPI) } else { MIN_PPI };
    let area = (bounds.width() * bounds.height()).max(1.0);
    let scale = (ppi / 72.0).min((super::super::menucmds::MAX_PIXELS / area).sqrt());
    if !(scale.is_finite() && scale > 0.0) {
        return Ok(None);
    }
    let region = Rect::new(
        bounds.x0,
        bounds.y0,
        bounds.x0 + (bounds.width() * scale).ceil().max(1.0) / scale,
        bounds.y0 + (bounds.height() * scale).ceil().max(1.0) / scale,
    );
    let mut with = src.clone();
    let mut i = 0;
    with.layers = src.layers.iter().filter_map(|l| up_to(l, at, &mut i)).map(Arc::new).collect();
    let mut without = with.clone();
    without.remove(id)?;
    let mut r = vectorcraft_render::Renderer::new();
    let shown = r.render_region(&with, region, scale, true);
    let under = r.render_region(&without, region, scale, true);
    let (w, h) = (shown.width as usize, shown.height as usize);
    if w == 0 || h == 0 || shown.pixels.len() != w * h * 4 || under.pixels.len() != shown.pixels.len() {
        return Ok(None);
    }
    // The cells where it changes anything.
    let c = (CELL * scale).round().max(1.0) as usize;
    let (cw, ch) = (w.div_ceil(c), h.div_ceil(c));
    let mut cells = vec![false; cw * ch];
    for (k, (a, b)) in shown.pixels.as_chunks::<4>().0.iter().zip(under.pixels.as_chunks::<4>().0).enumerate() {
        if a.iter().zip(b).any(|(a, b)| a.abs_diff(*b) > CHANGED) {
            let (x, y) = (k % w, k / w);
            if let Some(cell) = cells.get_mut(y / c * cw + x / c) {
                *cell = true;
            }
        }
    }
    let marked = || cells.iter().enumerate().filter(|(_, on)| **on).map(|(k, _)| (k % cw, k / cw));
    let (Some(x0), Some(x1), Some(y0), Some(y1)) =
        (marked().map(|p| p.0).min(), marked().map(|p| p.0).max(), marked().map(|p| p.1).min(), marked().map(|p| p.1).max())
    else {
        return Ok(None);
    };
    // Crop to them.
    let (px0, py0) = (x0 * c, y0 * c);
    let (px1, py1) = (((x1 + 1) * c).min(w), ((y1 + 1) * c).min(h));
    let mut pixels = Vec::with_capacity((px1 - px0) * (py1 - py0) * 4);
    for y in py0..py1 {
        pixels.extend_from_slice(shown.pixels.get((y * w + px0) * 4..(y * w + px1) * 4).unwrap_or_default());
    }
    let crop = vectorcraft_render::Rendered { width: (px1 - px0) as u32, height: (py1 - py0) as u32, pixels };
    let png = crop.to_png().map_err(EngineError::Other)?;
    let at = |x: usize, y: usize| Point::new(region.x0 + (x.min(w)) as f64 / scale, region.y0 + (y.min(h)) as f64 / scale);
    let mut clip = PathData::new(vec![]);
    for comp in vectorcraft_trace::trace_mask(&cells, cw, ch) {
        for lp in std::iter::once(&comp.outer).chain(&comp.holes) {
            let pts: Vec<(i32, i32)> = corners(&lp.pts);
            let anchors = pts.iter().map(|&(x, y)| Anchor::corner(at(x.max(0) as usize * c, y.max(0) as usize * c))).collect();
            clip.subpaths.push(SubPath::new(anchors, true));
        }
    }
    let xf = Affine::translate(at(px0, py0).to_vec2()) * Affine::scale(1.0 / scale);
    Ok(Some(Flat { png, width: crop.width, height: crop.height, xf, clip }))
}

/// `n` without the nodes after the `last`th in paint order (`i` counts them, as
/// [`Document::walk`] does); `None` when `n` itself comes after it.
fn up_to(n: &Node, last: usize, i: &mut usize) -> Option<Node> {
    if *i > last {
        return None;
    }
    *i += 1;
    let mut n = n.clone();
    if let Some(ch) = n.children_mut() {
        let kept: Vec<Arc<Node>> = ch.iter().filter_map(|c| up_to(c, last, i)).map(Arc::new).collect();
        *ch = kept;
    }
    Some(n)
}

/// A loop's corners: its points without those in the middle of a straight run.
fn corners(pts: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let n = pts.len();
    let get = |k: usize| pts.get(k % n.max(1)).copied();
    (0..n)
        .filter_map(|k| {
            let (a, b, c) = (get(k + n - 1)?, get(k)?, get(k + 1)?);
            let straight = (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1);
            (!straight).then_some(b)
        })
        .collect()
}

fn note(warnings: &mut Vec<String>, w: &str) {
    if !warnings.iter().any(|x| x == w) {
        warnings.push(w.to_string());
    }
}
