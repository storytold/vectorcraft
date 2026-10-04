//! Object menu long tail: Lock/Hide All Artwork Above & Other Layers, Transform Each, Reset
//! Bounding Box, Expand, Rasterize, Crop Image, Create Trim Marks, Convert to Shape and
//! Artboards → Convert / Rearrange. (Live blends are in `live.rs`.)

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ImageObject, LiveShape, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Rect, Vec2, shapes};

use super::edit::{duplicate_in, selected_roots};
use super::*;

/// Session-level (not saved) state owned by the menu commands.
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// Saved selections: (document title, name, object ids).
    pub saved_selections: Vec<(String, String, Vec<NodeId>)>,
    /// View → Guides → Lock Guides.
    pub guides_locked: bool,
    /// Transparency panel menu: "New Opacity Masks Are Clipping" turned off.
    pub new_masks_unclipped: bool,
    /// Transparency panel menu: "New Opacity Masks Are Inverted".
    pub new_masks_inverted: bool,
    /// Magic Wand panel settings.
    pub wand: super::wand::WandSettings,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.lock.above",
            "All Artwork Above",
            ["Object", "Lock"],
            None,
            "{} lock objects in the same layer that are above and overlap the selection → {count}",
            has_selection,
            |s, _| above(s, true)
        ),
        cmd!(
            "object.lock.otherLayers",
            "Other Layers",
            ["Object", "Lock"],
            None,
            "{} lock every layer that holds no selected object → {count}",
            has_selection,
            |s, _| other_layers(s, true)
        ),
        cmd!(
            "object.hide.above",
            "All Artwork Above",
            ["Object", "Hide"],
            None,
            "{} hide objects in the same layer that are above and overlap the selection → {count}",
            has_selection,
            |s, _| above(s, false)
        ),
        cmd!(
            "object.hide.otherLayers",
            "Other Layers",
            ["Object", "Hide"],
            None,
            "{} hide every layer that holds no selected object → {count}",
            has_selection,
            |s, _| other_layers(s, false)
        ),
        cmd!(
            "object.transformEach",
            "Transform Each…",
            ["Object", "Transform"],
            Some("Cmd+Alt+Shift+D"),
            "{scaleH?: % (100), scaleV?: % (100), moveH?: pt, moveV?: pt (down = +), rotate?: deg (counter-clockwise), reflectX?: bool (flip vertically), reflectY?: bool (flip horizontally), random?: bool, seed?: n, reference?: 0..8 (9-point grid, 4 = centre), copy?: bool} transform every selected object about its own reference point → {ids}",
            has_selection,
            transform_each
        ),
        cmd!(
            "object.resetBoundingBox",
            "Reset Bounding Box",
            ["Object", "Transform"],
            None,
            "{} no-op: VectorCraft bounding boxes are always axis-aligned to the document → {changed: 0}",
            has_selection,
            |_, _| Ok(json!({ "changed": 0 }))
        ),
        cmd!(
            "object.expand",
            "Expand…",
            ["Object"],
            None,
            "{object?: true (text → outlines, live shapes → paths, effects baked), fill?: true, stroke?: false (strokes → filled outlines)} one undo step → {ids}",
            has_selection,
            expand
        ),
        cmd!(
            "object.rasterize",
            "Rasterize…",
            ["Object"],
            None,
            "{ppi?: (document raster effects resolution), background?: \"transparent\"|\"white\", padding?: pt} replace the selection with an embedded PNG image → {id, width, height}",
            has_selection,
            rasterize
        ),
        cmd!(
            "object.createObjectMosaic",
            "Create Object Mosaic…",
            ["Object"],
            None,
            "{columns=10, rows=10, width?, height? (new size in pt; default the image's), spacingX=0, spacingY=0 (pt between tiles), gray?: bool, deleteRaster?: bool} a group of rectangles coloured by the selected image's average tile colours → {id, tiles}",
            has_image,
            object_mosaic
        ),
        cmd!(
            "object.cropImage",
            "Crop Image",
            ["Object"],
            None,
            "{rect?: [x, y, width, height]} crop the selected image (default: to the artboard it sits on) → {id, width, height}",
            has_image,
            crop_image
        ),
        cmd!(
            "object.createTrimMarks",
            "Create Trim Marks",
            ["Object"],
            None,
            "{offset?: pt (9), length?: pt (18), weight?: pt (0.3)} trim marks around the selection (or the first artboard) as a group → {id}",
            has_doc,
            trim_marks
        ),
        cmd!(
            "object.shape.convertToShape",
            "Convert to Shape",
            ["Object", "Shape"],
            None,
            "{} turn paths that are rectangles (any rotation) or axis-aligned ellipses into live shapes → {converted}",
            has_selection,
            convert_to_shape
        ),
        cmd!(
            "artboard.convertToArtboards",
            "Convert to Artboards",
            ["Object", "Artboards"],
            None,
            "{} turn each selected object's bounds into a new artboard (the objects are removed) → {artboards}",
            has_selection,
            convert_to_artboards
        ),
        cmd!(
            "artboard.rearrange",
            "Rearrange All Artboards…",
            ["Object", "Artboards"],
            None,
            "{columns?: n (2), spacing?: pt (20), byColumn?: false, moveArtwork?: true} lay artboards out in a grid",
            has_doc,
            rearrange_artboards
        ),
    ]
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Run a command's implementation without the enablement check or journaling.
pub(crate) fn run_raw(s: &mut Session, id: &str, p: &Value) -> Result<Value> {
    let c = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.into()))?;
    (c.run)(s, p)
}

/// Merge the undo entries pushed since the history had `from` entries into one labelled step.
pub(crate) fn squash(s: &mut Session, from: usize, label: &str) {
    if let Some(st) = s.active_mut() {
        let h = &mut st.history.undo;
        if h.len() > from {
            h.truncate(from + 1);
            h[from].label = label.to_string();
        }
    }
}

// ---------- Lock / Hide ----------

fn above(s: &mut Session, lock: bool) -> Result<Value> {
    let roots = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let mut hits: Vec<NodeId> = vec![];
    for id in &roots {
        let Some(layer) = d.layer_of(*id) else { continue };
        // The selected object's ancestor directly inside the layer.
        let anc = d.ancestry(*id).unwrap_or_default();
        let Some(pos) = anc.iter().position(|a| *a == layer) else { continue };
        let top = anc.get(pos + 1).copied().unwrap_or(*id);
        let Some(b) = d.node(*id).and_then(|n| n.visual_bounds()) else { continue };
        let Some((_, idx, _)) = d.position(top) else { continue };
        for n in d.children(Some(layer)).into_iter().flatten().skip(idx + 1) {
            if roots.contains(&n.id) || hits.contains(&n.id) {
                continue;
            }
            if n.visual_bounds().is_some_and(|nb| nb.intersect(b).area() > 0.0 || nb.contains(b.center())) {
                hits.push(n.id);
            }
        }
    }
    let label = if lock { "Lock" } else { "Hide" };
    let n = hits.len();
    s.edit(label, |d, _| {
        for id in &hits {
            if let Some(n) = d.node_mut(*id) {
                if lock {
                    n.locked = true;
                } else {
                    n.visible = false;
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "count": n }))
}

fn other_layers(s: &mut Session, lock: bool) -> Result<Value> {
    let st = s.doc()?;
    let keep: Vec<NodeId> = st.selection.objects.iter().filter_map(|id| st.doc.ancestry(*id)).flatten().collect();
    let others: Vec<NodeId> = st.doc.layers.iter().map(|l| l.id).filter(|l| !keep.contains(l)).collect();
    let n = others.len();
    s.edit(if lock { "Lock Others" } else { "Hide Others" }, |d, _| {
        for id in &others {
            if let Some(n) = d.node_mut(*id) {
                if lock {
                    n.locked = true;
                } else {
                    n.visible = false;
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "count": n }))
}

// ---------- Transform Each ----------

/// Small deterministic PRNG (xorshift64*) for Transform Each → Random.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn about(o: Point, a: Affine) -> Affine {
    Affine::translate(o.to_vec2()) * a * Affine::translate(-o.to_vec2())
}

fn transform_each(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    // Bounded so coordinates stay finite (and the document stays serializable).
    let sh = f64_or(p, "scaleH", 100.0).clamp(-100_000.0, 100_000.0);
    let sv = f64_or(p, "scaleV", 100.0).clamp(-100_000.0, 100_000.0);
    let mh = f64_or(p, "moveH", 0.0).clamp(-1.0e7, 1.0e7);
    let mv = f64_or(p, "moveV", 0.0).clamp(-1.0e7, 1.0e7);
    let rot = f64_or(p, "rotate", 0.0);
    let rx = bool_or(p, "reflectX", false);
    let ry = bool_or(p, "reflectY", false);
    let random = bool_or(p, "random", false);
    let refi = p.get("reference").and_then(Value::as_u64).unwrap_or(4).min(8) as usize;
    let copy = bool_or(p, "copy", false);
    if sh == 0.0 || sv == 0.0 {
        return Err(bad("object.transformEach", "scale must be non-zero"));
    }
    let mut rng = Rng(p.get("seed").and_then(Value::as_u64).unwrap_or(0x9E37_79B9_7F4A_7C15).max(1));
    let scale_strokes = s.prefs.scale_strokes;
    let ids = s.edit("Transform Each", |d, sel| {
        let targets = if copy { duplicate_in(d, sel, &roots, Affine::IDENTITY)? } else { roots.clone() };
        for id in &targets {
            let Some(b) = d.node(*id).and_then(|n| n.geometric_bounds()) else { continue };
            let (mut k_sh, mut k_sv, mut k_mh, mut k_mv, mut k_rot) = (sh, sv, mh, mv, rot);
            if random {
                k_sh = 100.0 + (sh - 100.0) * rng.next();
                k_sv = 100.0 + (sv - 100.0) * rng.next();
                k_mh = mh * (rng.next() * 2.0 - 1.0);
                k_mv = mv * (rng.next() * 2.0 - 1.0);
                k_rot = rot * (rng.next() * 2.0 - 1.0);
            }
            let o = vectorcraft_geom::reference_point(b, refi);
            let refl = Affine::scale_non_uniform(if ry { -1.0 } else { 1.0 }, if rx { -1.0 } else { 1.0 });
            let m = Affine::translate((k_mh, k_mv))
                * about(o, Affine::rotate(-k_rot.to_radians()) * refl * Affine::scale_non_uniform(k_sh / 100.0, k_sv / 100.0));
            if let Some(n) = d.node_mut(*id) {
                n.transform(m, scale_strokes);
            }
        }
        Ok(targets)
    })?;
    Ok(ids_json(&ids))
}

// ---------- Expand ----------

fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let object = bool_or(p, "object", true);
    let fill = bool_or(p, "fill", true);
    let stroke = bool_or(p, "stroke", false);
    let from = s.doc()?.history.undo.len();
    let rev0 = s.doc()?.revision;
    let roots = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let (mut has_text, mut has_fx, mut has_live, mut has_stroke) = (false, false, false, false);
    for r in &roots {
        if let Some(n) = d.node(*r) {
            n.walk(&mut |c| {
                has_text |= matches!(c.kind, NodeKind::Text(_));
                has_live |= matches!(c.kind, NodeKind::Path { live: Some(_), .. });
                has_fx |= !c.appearance.effects.is_empty();
                has_stroke |= matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && !c.appearance.stroke_paint().is_none();
            });
        }
    }
    if object && has_fx {
        let _ = run_raw(s, "effect.expandAppearance", &json!({}));
    }
    if object && has_live {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| {
            for r in &roots {
                if let Some(n) = d.node_mut(*r) {
                    drop_live(n);
                }
            }
            Ok(())
        })?;
    }
    if object && has_text {
        let _ = run_raw(s, "type.createOutlines", &json!({}));
    }
    if stroke && has_stroke {
        let _ = run_raw(s, "object.path.outlineStroke", &json!({}));
    }
    if !fill {
        // Fill unchecked: strokes only survive (Illustrator expands just the selected attributes).
    }
    if s.doc()?.revision == rev0 {
        return Err(EngineError::Other("Expand: nothing to expand".into()));
    }
    squash(s, from, "Expand");
    let ids = s.doc()?.selection.objects.clone();
    Ok(ids_json(&ids))
}

fn drop_live(n: &mut Node) {
    if let NodeKind::Path { live, .. } = &mut n.kind {
        *live = None;
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            drop_live(Arc::make_mut(c));
        }
    }
}

// ---------- Rasterize / Crop ----------

const MAX_PIXELS: f64 = 64.0e6;

/// A copy of `doc` whose only content is `nodes` (paint order), for offscreen rendering.
fn isolated_doc(doc: &vectorcraft_doc::Document, nodes: Vec<Node>) -> vectorcraft_doc::Document {
    let mut tmp = doc.clone();
    let mut layer = Node::layer(NodeId(u64::MAX), "raster", vectorcraft_doc::LayerColor::Preset(0));
    if let Some(ch) = layer.children_mut() {
        *ch = nodes.into_iter().map(Arc::new).collect();
    }
    tmp.layers = vec![Arc::new(layer)];
    tmp
}

fn unique_key(d: &vectorcraft_doc::Document, stem: &str) -> String {
    let mut i = d.images.len() + 1;
    loop {
        let k = format!("{stem}-{i}");
        if !d.images.contains_key(&k) {
            return k;
        }
        i += 1;
    }
}

fn render_png(doc: &vectorcraft_doc::Document, region: Rect, scale: f64, white: bool) -> Result<(Vec<u8>, u32, u32)> {
    if region.width() * region.height() * scale * scale > MAX_PIXELS || region.width() * scale > 65535.0 || region.height() * scale > 65535.0 {
        return Err(EngineError::Other("image would be too large; lower the resolution".into()));
    }
    let mut r = vectorcraft_render::Renderer::new();
    let img = r.render_region(doc, region, scale, white);
    Ok((img.to_png().map_err(EngineError::Other)?, img.width, img.height))
}

fn rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.rasterize";
    let roots = selected_roots(s)?;
    let st = s.doc()?;
    let ppi = f64_or(p, "ppi", st.doc.raster_effects_ppi);
    if !(1.0..=2400.0).contains(&ppi) {
        return Err(bad(C, "ppi must be between 1 and 2400"));
    }
    let pad = f64_or(p, "padding", 0.0).max(0.0);
    let white = str_param(p, "background") == Some("white");
    let b = st.doc.bounds_of(&roots, true).ok_or_else(|| bad(C, "selection has no bounds"))?.inflate(pad, pad);
    let scale = ppi / 72.0;
    // Snap to whole pixels.
    let region = Rect::new(b.x0, b.y0, b.x0 + (b.width() * scale).ceil().max(1.0) / scale, b.y0 + (b.height() * scale).ceil().max(1.0) / scale);
    let nodes: Vec<Node> = roots
        .iter()
        .filter_map(|id| st.doc.node(*id).cloned())
        .map(|mut n| {
            n.visible = true;
            n
        })
        .collect();
    let tmp = isolated_doc(&st.doc, nodes);
    let (png, w, h) = render_png(&tmp, region, scale, white)?;
    let top = *roots.last().ok_or_else(|| bad(C, "nothing selected"))?;
    let id = s.edit("Rasterize", |d, sel| {
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let key = unique_key(d, "raster");
        d.images.insert(key.clone(), vectorcraft_doc::ImageBlob { mime: "image/png".into(), bytes: Arc::new(png) });
        let id = d.alloc_id();
        let xf = Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale);
        let node = Node::new(id, NodeKind::Image(ImageObject { key, width: w, height: h, xf, link: None }));
        d.insert(par, idx + 1, node)?;
        for r in &roots {
            d.remove(*r)?;
        }
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0, "width": w, "height": h }))
}

fn has_image(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().unwrap();
    if st.selection.objects.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_)))) {
        Ok(())
    } else {
        Err("select an image".into())
    }
}

fn object_mosaic(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.createObjectMosaic";
    let st = s.doc()?;
    let id = st
        .selection
        .objects
        .iter()
        .copied()
        .find(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_))))
        .ok_or_else(|| bad(C, "select an image"))?;
    let node = st.doc.node(id).cloned().ok_or(EngineError::NoNode(id))?;
    let NodeKind::Image(im) = &node.kind else { unreachable!() };
    let blob = st.doc.images.get(&im.key).filter(|b| !b.bytes.is_empty()).ok_or_else(|| bad(C, "the image has no embedded pixels"))?;
    let raster = vectorcraft_trace::Raster::decode(&blob.bytes).map_err(|e| bad(C, e.to_string()))?;
    let cols = p.get("columns").and_then(Value::as_u64).unwrap_or(10).clamp(1, 1000) as u32;
    let rows = p.get("rows").and_then(Value::as_u64).unwrap_or(10).clamp(1, 1000) as u32;
    let gray = bool_or(p, "gray", false);
    let colors = vectorcraft_trace::mosaic(&raster, cols, rows);
    // Tiles in image space, placed by the image transform (scaled to a new size if asked).
    let b = node.geometric_bounds().ok_or_else(|| bad(C, "image has no bounds"))?;
    let (nw, nh) = (f64_or(p, "width", b.width()).max(0.1), f64_or(p, "height", b.height()).max(0.1));
    let resize = Affine::translate(b.origin().to_vec2())
        * Affine::scale_non_uniform(nw / b.width().max(1e-9), nh / b.height().max(1e-9))
        * Affine::translate(-b.origin().to_vec2());
    let xf = resize * im.xf;
    let (tw, th) = (im.width as f64 / cols as f64, im.height as f64 / rows as f64);
    // Spacing is in points; convert to image pixels along each axis.
    let ppx = (xf * Point::new(1.0, 0.0) - xf * Point::ZERO).hypot().max(1e-9);
    let ppy = (xf * Point::new(0.0, 1.0) - xf * Point::ZERO).hypot().max(1e-9);
    let (gx, gy) = ((f64_or(p, "spacingX", 0.0).max(0.0) / ppx).min(tw * 0.9), (f64_or(p, "spacingY", 0.0).max(0.0) / ppy).min(th * 0.9));
    let delete = bool_or(p, "deleteRaster", false);
    let (gid, n) = s.edit("Create Object Mosaic", |d, sel| {
        let mut tiles = vec![];
        for r in 0..rows {
            for c in 0..cols {
                let [cr, cg, cb, ca] = colors[(r * cols + c) as usize];
                if ca == 0 {
                    continue;
                }
                let x0 = c as f64 * tw + gx / 2.0;
                let y0 = r as f64 * th + gy / 2.0;
                let quad = [Point::new(x0, y0), Point::new(x0 + tw - gx, y0), Point::new(x0 + tw - gx, y0 + th - gy), Point::new(x0, y0 + th - gy)]
                    .map(|q| xf * q);
                let color = if gray {
                    let l = (0.299 * cr as f32 + 0.587 * cg as f32 + 0.114 * cb as f32) / 255.0;
                    Color::rgb(l, l, l)
                } else {
                    Color::rgb(cr as f32 / 255.0, cg as f32 / 255.0, cb as f32 / 255.0)
                };
                let mut tile = Node::path(d.alloc_id(), polygon_path(&quad), Appearance::basic(Paint::solid(color), Paint::None, 0.0));
                if ca < 255 {
                    tile.opacity = ca as f32 / 255.0;
                }
                tiles.push(Arc::new(tile));
            }
        }
        let n = tiles.len();
        let (par, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        let gid = d.alloc_id();
        let mut g = Node::group(gid, tiles);
        g.name = Some("Object Mosaic".into());
        d.insert(par, idx + 1, g)?;
        if delete {
            d.remove(id)?;
        }
        sel.set([gid]);
        Ok((gid, n))
    })?;
    Ok(json!({ "id": gid.0, "tiles": n }))
}

fn polygon_path(pts: &[Point]) -> PathData {
    PathData::single(vectorcraft_geom::SubPath::new(pts.iter().map(|p| Anchor::corner(*p)).collect(), true))
}

fn crop_image(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.cropImage";
    let st = s.doc()?;
    let id = st
        .selection
        .objects
        .iter()
        .copied()
        .find(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_))))
        .ok_or_else(|| bad(C, "select an image"))?;
    let node = st.doc.node(id).cloned().ok_or(EngineError::NoNode(id))?;
    let NodeKind::Image(im) = &node.kind else { unreachable!() };
    let ib = node.geometric_bounds().ok_or_else(|| bad(C, "image has no bounds"))?;
    let rect = match p.get("rect").and_then(Value::as_array) {
        Some(a) if a.len() == 4 => {
            let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
            if v.len() != 4 {
                return Err(bad(C, "rect must be [x, y, width, height]"));
            }
            Rect::new(v[0], v[1], v[0] + v[2], v[1] + v[3]).abs()
        }
        Some(_) => return Err(bad(C, "rect must be [x, y, width, height]")),
        None => {
            let ab = st.doc.artboard_at(ib.center()).and_then(|i| st.doc.artboards.get(i)).map(|a| a.rect);
            match ab {
                Some(r) => r,
                None => return Err(bad(C, "the image is not on an artboard; pass rect")),
            }
        }
    };
    let r = rect.intersect(ib);
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return Err(bad(C, "crop rectangle doesn't overlap the image"));
    }
    if (r.x0 - ib.x0).abs() < 1e-6 && (r.y0 - ib.y0).abs() < 1e-6 && (r.x1 - ib.x1).abs() < 1e-6 && (r.y1 - ib.y1).abs() < 1e-6 {
        return Err(bad(C, "nothing to crop"));
    }
    // Keep the image's pixel density.
    let scale = (im.width as f64 / ib.width().max(1e-9)).max(im.height as f64 / ib.height().max(1e-9));
    let mut bare = node.clone();
    bare.opacity = 1.0;
    bare.blend = Default::default();
    bare.appearance = Appearance::default();
    bare.visible = true;
    let tmp = isolated_doc(&st.doc, vec![bare]);
    let region = Rect::new(r.x0, r.y0, r.x0 + (r.width() * scale).round().max(1.0) / scale, r.y0 + (r.height() * scale).round().max(1.0) / scale);
    let (png, w, h) = render_png(&tmp, region, scale, false)?;
    s.edit("Crop Image", |d, _| {
        let key = unique_key(d, "crop");
        d.images.insert(key.clone(), vectorcraft_doc::ImageBlob { mime: "image/png".into(), bytes: Arc::new(png) });
        if let Some(n) = d.node_mut(id) {
            n.kind = NodeKind::Image(ImageObject {
                key,
                width: w,
                height: h,
                xf: Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale),
                link: None,
            });
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0, "width": w, "height": h }))
}

// ---------- Trim marks ----------

fn trim_marks(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let off = f64_or(p, "offset", 9.0).max(0.0);
    let len = f64_or(p, "length", 18.0).max(0.1);
    let weight = f64_or(p, "weight", 0.3).max(0.01);
    let r = if st.selection.is_empty() {
        st.doc.artboards.first().map(|a| a.rect)
    } else {
        let roots = selected_roots(s)?;
        s.doc()?.doc.bounds_of(&roots, true)
    }
    .ok_or_else(|| EngineError::Other("nothing to mark".into()))?;
    let parent = s.doc()?.insertion_parent();
    let lines = [
        // Top-left.
        (Point::new(r.x0 - off - len, r.y0), Point::new(r.x0 - off, r.y0)),
        (Point::new(r.x0, r.y0 - off - len), Point::new(r.x0, r.y0 - off)),
        // Top-right.
        (Point::new(r.x1 + off, r.y0), Point::new(r.x1 + off + len, r.y0)),
        (Point::new(r.x1, r.y0 - off - len), Point::new(r.x1, r.y0 - off)),
        // Bottom-right.
        (Point::new(r.x1 + off, r.y1), Point::new(r.x1 + off + len, r.y1)),
        (Point::new(r.x1, r.y1 + off), Point::new(r.x1, r.y1 + off + len)),
        // Bottom-left.
        (Point::new(r.x0 - off - len, r.y1), Point::new(r.x0 - off, r.y1)),
        (Point::new(r.x0, r.y1 + off), Point::new(r.x0, r.y1 + off + len)),
    ];
    let id = s.edit("Create Trim Marks", |d, sel| {
        let mut children = vec![];
        for (a, b) in lines {
            let id = d.alloc_id();
            children.push(Arc::new(Node::path(id, shapes::line(a, b), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), weight))));
        }
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = Some("Trim Marks".into());
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0 }))
}

// ---------- Convert to Shape ----------

fn no_handles(a: &Anchor) -> bool {
    !a.has_in() && !a.has_out()
}

/// Recognise a rectangle (any rotation) or an axis-aligned ellipse.
pub(crate) fn detect_shape(path: &PathData) -> Option<LiveShape> {
    let [sp] = path.subpaths.as_slice() else { return None };
    if !sp.closed || sp.anchors.len() != 4 {
        return None;
    }
    let a = &sp.anchors;
    let size = path.bounds()?.size();
    let tol = 1e-3 * size.width.max(size.height).max(1.0);
    if a.iter().all(no_handles) {
        let e0 = a[1].p - a[0].p;
        let e1 = a[2].p - a[1].p;
        let e2 = a[3].p - a[2].p;
        let (w, h) = (e0.hypot(), e1.hypot());
        if w < 1e-9 || h < 1e-9 {
            return None;
        }
        if e0.dot(e1).abs() > tol * (w + h) || (e2 + e0).hypot() > tol {
            return None;
        }
        let rot = Affine::rotate(e0.y.atan2(e0.x));
        let flip = if e0.cross(e1) < 0.0 { Affine::scale_non_uniform(1.0, -1.0) } else { Affine::IDENTITY };
        return Some(LiveShape::Rectangle { w, h, radii: [0.0; 4], xf: Affine::translate(a[0].p.to_vec2()) * rot * flip });
    }
    let b = path.bounds()?;
    let c = b.center();
    let mids = [Point::new(c.x, b.y0), Point::new(b.x1, c.y), Point::new(c.x, b.y1), Point::new(b.x0, c.y)];
    let k = 0.5523;
    for an in a {
        if !mids.iter().any(|m| (an.p - *m).hypot() < tol) {
            return None;
        }
        // Handles tangent to the bounds and ~κ·radius long.
        let horizontal = (an.p.y - b.y0).abs() < tol || (an.p.y - b.y1).abs() < tol;
        let want = if horizontal { k * b.width() / 2.0 } else { k * b.height() / 2.0 };
        for hnd in [an.h_in, an.h_out] {
            let v = hnd - an.p;
            let (along, across) = if horizontal { (v.x.abs(), v.y.abs()) } else { (v.y.abs(), v.x.abs()) };
            if across > tol * 10.0 || (along - want).abs() > want * 0.05 + tol {
                return None;
            }
        }
    }
    Some(LiveShape::Ellipse { w: b.width(), h: b.height(), pie: (0.0, 360.0), xf: Affine::translate(b.origin().to_vec2()) })
}

fn convert_to_shape(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let n = s.edit("Convert to Shape", |d, _| {
        let mut n = 0;
        for id in &roots {
            let Some(NodeKind::Path { path, live, .. }) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            if live.is_some() {
                continue;
            }
            if let Some(l) = detect_shape(path) {
                *path = l.to_path();
                *live = Some(l);
                n += 1;
            }
        }
        if n == 0 {
            return Err(EngineError::Other("Convert to Shape: no rectangles or ellipses selected".into()));
        }
        Ok(n)
    })?;
    Ok(json!({ "converted": n }))
}

// ---------- Artboards ----------

fn convert_to_artboards(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let n = s.edit("Convert to Artboards", |d, sel| {
        let mut n = 0;
        for id in &roots {
            let Some(b) = d.node(*id).and_then(|n| n.geometric_bounds()) else { continue };
            if b.width() <= 0.0 || b.height() <= 0.0 {
                continue;
            }
            let aid = d.next_artboard_id();
            d.artboards.push(vectorcraft_doc::Artboard {
                id: aid,
                name: format!("Artboard {aid}"),
                rect: b,
                show_center_mark: false,
                show_cross_hairs: false,
            });
            d.remove(*id)?;
            n += 1;
        }
        if n == 0 {
            return Err(EngineError::Other("Convert to Artboards: selection has no area".into()));
        }
        sel.clear();
        Ok(n)
    })?;
    Ok(json!({ "artboards": n }))
}

fn rearrange_artboards(s: &mut Session, p: &Value) -> Result<Value> {
    let n_ab = s.doc()?.doc.artboards.len().max(1);
    let cols = (f64_or(p, "columns", 2.0).clamp(1.0, n_ab as f64)) as usize;
    let spacing = f64_or(p, "spacing", 20.0).clamp(-1.0e5, 1.0e5);
    let by_col = bool_or(p, "byColumn", false);
    let move_art = bool_or(p, "moveArtwork", true);
    let scale_strokes = false;
    s.edit("Rearrange Artboards", |d, _| {
        let rects: Vec<Rect> = d.artboards.iter().map(|a| a.rect).collect();
        let Some(first) = rects.first().copied() else { return Ok(()) };
        let n = rects.len();
        let rows = n.div_ceil(cols);
        // Grid cell (row, col) of artboard i.
        let cell = |i: usize| if by_col { (i % rows, i / rows) } else { (i / cols, i % cols) };
        let mut col_w = vec![0.0f64; cols];
        let mut row_h = vec![0.0f64; rows];
        for (i, r) in rects.iter().enumerate() {
            let (ro, co) = cell(i);
            col_w[co] = col_w[co].max(r.width());
            row_h[ro] = row_h[ro].max(r.height());
        }
        let x_of = |c: usize| first.x0 + col_w[..c].iter().map(|w| w + spacing).sum::<f64>();
        let y_of = |r: usize| first.y0 + row_h[..r].iter().map(|h| h + spacing).sum::<f64>();
        let deltas: Vec<Vec2> = rects
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let (ro, co) = cell(i);
                Point::new(x_of(co), y_of(ro)) - r.origin()
            })
            .collect();
        if move_art {
            let tops: Vec<(NodeId, Point)> = d
                .layers
                .iter()
                .flat_map(|l| l.children().into_iter().flatten())
                .filter_map(|n| Some((n.id, n.geometric_bounds()?.center())))
                .collect();
            for (id, c) in tops {
                if let Some(i) = rects.iter().position(|r| r.contains(c))
                    && deltas[i] != Vec2::ZERO
                    && let Some(n) = d.node_mut(id)
                {
                    n.transform(Affine::translate(deltas[i]), scale_strokes);
                }
            }
        }
        for (a, dl) in d.artboards.iter_mut().zip(&deltas) {
            a.rect = a.rect + *dl;
        }
        Ok(())
    })?;
    ok()
}
