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
//! under it, rendered from the document as it is, clipped to the area where it changes what shows
//! (a smooth outline of its rendered pixels, not a staircase of them);
//! the art under it stays as it was. An image in a group with opacity or a blending mode is
//! flattened the same way, and a pattern fill that paints a transparent image (see-through
//! pixels, or opacity or a blending mode on the image, the fill, the shape or a group it is in) is
//! expanded into its tiles first so its images flatten too. With `omit` the images with
//! see-through pixels, opacity or a blending mode of their own are left out, and pattern fills stay
//! as the writer draws them (their tiles opaque over white). A colorized image is flattened or
//! left out the same way, unless its pixels are all black or white: the writer writes that one as
//! a 1-bit image mask in its color, which keeps its white pixels clear. An image that can't be decoded is
//! missing from the images that flattening and expanding raster effects draw, and the warnings
//! then carry the raster exports' note once.
//!
//! The format can't hide objects, only layers (`hiddenObjects`): with `layer` (the default) each
//! layer's hidden objects go to a hidden layer of their own just above it, with `omit` they are
//! left out, with `visible` they show where they are. Type is written as text the format's reader
//! lays out again, in the fonts it names; `outlineText` writes its outlines instead (as type that
//! 8-bit text can't hold and type with rotated characters always are).

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Anchor, FillRule, PathData, Point, Rect, SubPath};

use super::super::{Affine, EngineError, NodeId, Result, bad};
use super::{ArtboardPick, Encoded, Format};

/// Does `p` ask a save in `f` for a legacy `.ai` file of version 7 (`aiVersion: "7"`)? An unknown
/// version is an error, and so is version 7 for a `.ait` template, which is never a legacy file.
pub(super) fn wanted(cmd: &str, f: &Format, p: &Value) -> Result<bool> {
    let legacy = match p.get("aiVersion") {
        None | Some(Value::Null) => false,
        Some(Value::String(v)) if v.trim().is_empty() || v.eq_ignore_ascii_case("pdf") => false,
        Some(Value::String(v)) if v.trim() == "7" => true,
        Some(Value::Number(n)) if n.as_f64() == Some(7.0) => true,
        Some(v) => return Err(bad(cmd, format!("aiVersion {v}: \"pdf\" (PDF-compatible, the default) or \"7\" (legacy .ai, version 7)"))),
    };
    if legacy && f.id == "ait" {
        return Err(bad(cmd, "aiVersion 7 writes legacy .ai files, not .ait templates"));
    }
    Ok(legacy)
}

/// `f` (the `.ai` format, whose label names the PDF-compatible file) named, in its errors, as the
/// legacy `.ai` file.
pub(super) fn named(f: &Format) -> Format {
    Format { label: "legacy .ai (version 7)", ..*f }
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
    let picked: ArtboardPick = super::encode::options(&named(f), p)?;
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
        if mode == SeeThrough::Flatten {
            expand_image_patterns(&mut d)?;
        }
        let images = see_through_images(&d, mode);
        if !images.is_empty() {
            if mode == SeeThrough::Flatten {
                note(&mut enc.warnings, FLATTENED);
                flatten_images(&mut d, &images)?;
            } else {
                // A note on the options: `transparentImages: omit` leaves them out.
                enc.add_setting_notes([OMITTED.to_string()]);
                for (_, id) in images {
                    let _ = d.remove(id); // Inside an image left out before: gone already.
                }
            }
        }
        let o = vectorcraft_eps::LegacyAiOptions { title: doc.info_title().to_owned(), created, hidden, outline_text };
        let out = vectorcraft_eps::export_legacy_ai(&d, &o).map_err(|e| bad(cmd, e))?;
        for w in placed.iter().chain(&out.warnings) {
            note(&mut enc.warnings, w);
        }
        enc.add_setting_notes(out.setting_notes);
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

/// Expand every layer's pattern fills that paint an image with see-through pixels, a colorize paint,
/// opacity or blending into their tiles, so those images can be flattened.
fn expand_image_patterns(d: &mut Document) -> Result<()> {
    let keys = super::super::flatten::see_through_images(d);
    let layers: Vec<NodeId> = d.layers.iter().map(|l| l.id).collect();
    for id in layers {
        super::super::expand::expand_transparent_image_patterns(d, id, &keys, &|_, _| true)?;
    }
    Ok(())
}

/// The images of `d` a legacy `.ai` can't hold, in paint order with their places in it (as
/// [`Document::walk`] counts). These have see-through pixels (a color-masked image's masked pixels
/// counted as its own) or a colorize paint (except the
/// colorized images the writer writes as image masks, clear pixels included,
/// [`vectorcraft_eps::writes_image_mask`]), or
/// have opacity or a blending mode of their own or, with `flatten`, of a group they are in.
/// With `flatten` only visible images are returned, because a hidden one (which the hidden objects
/// option may write) is written over white. With `omit` all of them are returned.
fn see_through_images(d: &Document, mode: SeeThrough) -> Vec<(usize, NodeId)> {
    let keys = super::super::flatten::see_through_images(d);
    let mut out = vec![];
    let mut at = 0;
    d.walk(|n| {
        if let NodeKind::Image(im) = &n.kind
            && (((keys.contains(&im.key) || keys.contains(&d.shown_image(n, im).key) || n.extras().colorize.is_some())
                && !vectorcraft_eps::writes_image_mask(d, n))
                || n.opacity < 1.0
                || n.blend != BlendMode::Normal
                || (mode == SeeThrough::Flatten && transparent(d, n.id)))
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

/// Whether node `id` or a group it is in has opacity or a blending mode. [`Document::ancestry`]
/// lists `id` itself last, so its own opacity and blending mode count too.
fn transparent(d: &Document, id: NodeId) -> bool {
    d.ancestry(id).unwrap_or_default().iter().filter_map(|a| d.node(*a)).any(|n| n.opacity < 1.0 || n.blend != BlendMode::Normal)
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
            let image =
                ImageObject { key, width: flat.width, height: flat.height, xf: flat.xf, link: None, placement: Default::default(), pixels: None };
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
/// The clip outline follows the rendered pixels, in blocks of them when there would be more than
/// this many: a huge image's clip would have millions of anchors.
const MAX_CELLS: usize = 4 << 20;
/// How far (in cells) the clip outline may stray from the staircase it smooths.
const SMOOTH: f64 = 0.35;

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
    // No side longer than the renderer draws (rounded up to whole pixels below).
    let longest = (f64::from(vectorcraft_render::MAX_RASTER_SIDE) - 2.0) / bounds.width().max(bounds.height()).max(1.0);
    let scale = (ppi / 72.0).min((super::super::menucmds::MAX_PIXELS / area).sqrt()).min(longest);
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
    // The cells where it changes anything: its pixels, or blocks of them for a huge image.
    let c = if w * h <= MAX_CELLS { 1 } else { ((w * h) as f64 / MAX_CELLS as f64).sqrt().ceil() as usize };
    let (cw, ch) = (w.div_ceil(c), h.div_ceil(c));
    let mut changed = vec![false; cw * ch];
    for (k, (a, b)) in shown.pixels.as_chunks::<4>().0.iter().zip(under.pixels.as_chunks::<4>().0).enumerate() {
        if a.iter().zip(b).any(|(a, b)| a.abs_diff(*b) > CHANGED) {
            let (x, y) = (k % w, k / w);
            if let Some(cell) = changed.get_mut(y / c * cw + x / c) {
                *cell = true;
            }
        }
    }
    // Grown by a cell, so the clip takes in the whole anti-aliased edge: where the image changes
    // nothing it is the art under it, so a clip a little too large shows nothing more.
    let cells = grown(&changed, cw, ch);
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
    // A cell corner, in cells, as a point of the page.
    let (wf, hf, cf) = (w as f64, h as f64, c as f64);
    let page = |(x, y): (f64, f64)| Point::new(region.x0 + (x * cf).clamp(0.0, wf) / scale, region.y0 + (y * cf).clamp(0.0, hf) / scale);
    let mut clip = PathData::new(vec![]);
    for comp in vectorcraft_trace::trace_mask(&cells, cw, ch) {
        for lp in std::iter::once(&comp.outer).chain(&comp.holes) {
            let anchors: Vec<Anchor> = smoothed(&lp.pts).into_iter().map(|p| Anchor::corner(page(p))).collect();
            if anchors.len() >= 3 {
                clip.subpaths.push(SubPath::new(anchors, true));
            }
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

/// `mask` (`w × h` cells) with every cell next to a set one set too.
fn grown(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let set = |x: usize, y: usize| mask.get(y * w + x).copied().unwrap_or(false);
    let mut out = vec![false; mask.len()];
    for (k, cell) in out.iter_mut().enumerate() {
        let (x, y) = (k % w, k / w);
        *cell = (y.saturating_sub(1)..(y + 2).min(h)).any(|y| (x.saturating_sub(1)..(x + 2).min(w)).any(|x| set(x, y)));
    }
    out
}

/// A loop of cell corners (a staircase) as a smooth outline, in cells: the middle of each cell
/// edge along it, so that a run of one-cell steps becomes the sloped line it stands for, without
/// the points that lie (within [`SMOOTH`]) on a straight line between their neighbours.
fn smoothed(pts: &[(i32, i32)]) -> Vec<(f64, f64)> {
    let mut mids = Vec::with_capacity(pts.len());
    for (k, &(ax, ay)) in pts.iter().enumerate() {
        let Some(&(bx, by)) = pts.get(k + 1).or(pts.first()) else { break };
        let (dx, dy) = ((bx - ax).signum(), (by - ay).signum());
        let steps = (bx - ax).unsigned_abs().max((by - ay).unsigned_abs());
        for i in 0..steps as i32 {
            let (x, y) = (ax + dx * i, ay + dy * i);
            mids.push((f64::from(x) + 0.5 * f64::from(dx), f64::from(y) + 0.5 * f64::from(dy)));
        }
    }
    simplify_closed(&mids, SMOOTH)
}

/// The points of closed polyline `pts` that Douglas–Peucker keeps with tolerance `tol`.
fn simplify_closed(pts: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n < 4 {
        return pts.to_vec();
    }
    let d2 = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
    let Some(&first) = pts.first() else { return vec![] };
    // Split at the first point and the point farthest from it: two open runs.
    let far = (1..n).max_by(|&i, &j| d2(first, pts[i]).total_cmp(&d2(first, pts[j]))).unwrap_or(n / 2);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    let mut todo = vec![(0, far), (far, n)];
    while let Some((a, b)) = todo.pop() {
        if b <= a + 1 {
            continue;
        }
        let (p, q) = (pts[a], pts[b % n]);
        let (ux, uy) = (q.0 - p.0, q.1 - p.1);
        let len = ux.hypot(uy);
        let dist = |r: (f64, f64)| if len > 0.0 { ((r.0 - p.0) * uy - (r.1 - p.1) * ux).abs() / len } else { d2(p, r).sqrt() };
        let Some((m, dm)) = (a + 1..b).map(|i| (i, dist(pts[i]))).max_by(|x, y| x.1.total_cmp(&y.1)) else { continue };
        if dm > tol {
            keep[m] = true;
            todo.push((a, m));
            todo.push((m, b));
        }
    }
    pts.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

fn note(warnings: &mut Vec<String>, w: &str) {
    if !warnings.iter().any(|x| x == w) {
        warnings.push(w.to_string());
    }
}
