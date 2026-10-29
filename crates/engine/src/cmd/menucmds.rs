//! Object menu long tail: Lock/Hide All Artwork Above & Other Layers, Transform Each, Reset
//! Bounding Box, Rasterize, Crop Image, Mask (an image), Create Trim Marks, Convert to Shape and
//! Artboards → Convert / Rearrange. (Live blends are in `live.rs`.)

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::marks::{MarkStyle, TrimMarks};
use vectorcraft_doc::{Appearance, ImageObject, LiveShape, Node, NodeId, NodeKind, RasterColorModel, RasterEffectsSettings, Scaling};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Rect, Vec2};
use vectorcraft_render::AntiAlias;
use vectorcraft_render::encode::{RasterExportOptions, RasterFormat};

use super::edit::{duplicate_in, selected_roots, selected_roots_to_transform};
use super::{
    CommandSpec, EngineError, Result, Session, bad, check_numbers, check_switches, cmd, f64_or, fileio, find_command, has_doc, has_selection,
    has_selection_outside_isolation, switch_or,
};

/// Session-level (not saved) state owned by the menu commands.
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// View → Guides → Lock Guides.
    pub guides_locked: bool,
    /// Transparency panel menu: "New Opacity Masks Are Clipping" turned off.
    pub new_masks_unclipped: bool,
    /// Transparency panel menu: "New Opacity Masks Are Inverted".
    pub new_masks_inverted: bool,
    /// Magic Wand panel settings.
    pub wand: super::wand::WandSettings,
    /// View → Hide Slices.
    pub slices_hidden: bool,
    /// View → Lock Slices.
    pub slices_locked: bool,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.lock.above",
            "All Artwork Above",
            ["Object", "Lock"],
            None,
            "{} lock objects in the same layer that are above and overlap the selection, deselecting every selected object left locked or hidden (the key object too) → {count}",
            has_selection_outside_isolation,
            |s, _| above(s, true)
        ),
        cmd!(
            "object.lock.otherLayers",
            "Other Layers",
            ["Object", "Lock"],
            None,
            "{} lock every layer that holds no selected object → {count}",
            has_selection_outside_isolation,
            |s, _| other_layers(s, true)
        ),
        cmd!(
            "object.hide.above",
            "All Artwork Above",
            ["Object", "Hide"],
            None,
            "{} hide objects in the same layer that are above and overlap the selection, deselecting every selected object left hidden or locked (the key object too) → {count}",
            has_selection_outside_isolation,
            |s, _| above(s, false)
        ),
        cmd!(
            "object.hide.otherLayers",
            "Other Layers",
            ["Object", "Hide"],
            None,
            "{} hide every layer that holds no selected object → {count}",
            has_selection_outside_isolation,
            |s, _| other_layers(s, false)
        ),
        cmd!(
            "object.hide.unselected",
            "Unselected Artwork",
            ["Object", "Hide"],
            Some("Cmd+Alt+Shift+3"),
            "{} hide, in one undo step, every object select.all takes that is not selected and holds no selected object; in a group that holds one it hides the other members one by one, and a clip group, compound path or other object that holds one stays as it is; the objects of a selected layer count as selected; while Lock Guides is on, guide paths count as selected and stay as they are; object.showAll shows them again; disabled in isolation mode → {count}",
            has_selection_outside_isolation,
            hide_unselected
        ),
        cmd!(
            "object.transformEach",
            "Transform Each…",
            ["Object", "Transform"],
            Some("Cmd+Alt+Shift+D"),
            "{scaleH?: % (100), scaleV?: % (100), moveH?: pt, moveV?: pt (down = +), rotate?: deg (counter-clockwise), reflectX?: bool (flip left-right, as Reflect X in the Transform Each dialog), reflectY?: bool (flip top-bottom, as Reflect Y), random?: bool, seed?: n, reference?: 0..8 (9-point grid, 4 = centre), copy?: bool, strokes?: bool (Scale Strokes & Effects), corners?: bool (Scale Corners; both default to the preferences), objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns; default: prefs transformPatternTiles)} transform every selected object about its own reference point → {ids}",
            has_selection,
            transform_each
        ),
        cmd!(
            "object.resetBoundingBox",
            "Reset Bounding Box",
            ["Object", "Transform"],
            None,
            "{} square the bounding box of the selected objects to the page again after a rotation (their geometry stays; one undo step, none when no box is rotated) → {changed: objects reset}",
            has_selection,
            reset_bounding_box
        ),
        cmd!(
            "object.rasterize",
            "Rasterize…",
            ["Object"],
            None,
            "{ppi?, background?: transparent|white|black|\"#rrggbb\", antiAlias?: none|art|type (text snapped to pixels)|bool, padding?|addAround?: pt (0–1000), colorModel?: \"rgb\"|\"cmyk\"|\"grayscale\"|\"bitmap\", clippingMask?: bool (a clip group with the art's outline)} replace the selection with an embedded PNG image; each defaults to document.rasterEffectsSettings; a frame of threaded text that is rasterized leaves its thread with the text it shows → {id (the image, or its clip group), width, height, warning?: \"an image couldn't be decoded and is missing from the picture\" (as raster exports say)}",
            has_selection,
            rasterize
        ),
        cmd!(
            "object.createObjectMosaic",
            "Create Object Mosaic…",
            ["Object"],
            None,
            "{columns=10, rows=10 (whole numbers, kept within 1..1000), width?, height? (new size in pt; default the image's), spacingX=0, spacingY=0 (pt between tiles), gray?: bool, deleteRaster?: bool} a group of rectangles coloured by the selected image's average tile colours → {id, tiles}",
            has_image,
            object_mosaic
        ),
        cmd!(
            "object.cropImage",
            "Crop Image",
            ["Object"],
            None,
            "{rect?: [x, y, width, height] | trim?: true (to the pixels that aren't fully transparent, pixel for pixel; an image with none to cut is left as it is, trimmed: false)} crop the selected image (default: to the artboard it sits on) → {id, width, height, trimmed? (with trim), warning? (without trim: the image couldn't be decoded, as object.rasterize says)}",
            has_image,
            crop_image
        ),
        cmd!(
            "object.maskImage",
            "Mask",
            [],
            None,
            "{} clip the selected image with a rectangle around it (its own outline, rotated with it) and select that clipping path, whose handles then crop it; one undo step → {id: the clip group, path: the clipping path}",
            has_image,
            mask_image
        ),
        cmd!(
            "object.createTrimMarks",
            "Create Trim Marks",
            ["Object"],
            None,
            "{style?: \"roman\"|\"japanese\" (default: the japaneseCropMarks preference; japanese: double lines at the trim and bleed edges plus centre marks), allArtboards?: bool (default: true when nothing is selected), offset?: pt (9, roman), length?: pt (18), bleed?: pt (8.5, japanese), weight?: pt (0.3)} trim marks stroked in [Registration] (printing on every plate) around the selection, or around every artboard, one group each, as one undo step → {id: the first group, ids}",
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
            "{layout?: gridByRow|gridByColumn|row|column (gridByRow; byColumn?: true is gridByColumn), order?: leftToRight|rightToLeft (leftToRight: right-to-left mirrors the order along the rows), columns?: n (2; the rows for gridByColumn; row and column ignore it), spacing?: pt (20), moveArtwork?: true (each object of a layer or sublayer moves with the artboard its center is on; locked and hidden art, and art on a locked artboard, only with prefs moveLockedWithArtboard), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), byColumn?: bool (an older way to ask for layout gridByColumn)} lay the artboards out in their order, the grid's top-left at the first artboard's; a locked artboard stays where it is → {artboards, rows, columns}",
            has_doc,
            rearrange_artboards
        ),
    ]
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// The whole number `p` gives for `key`, written `4` or `4.0`, or `default` when `p` gives none or null. A value that
/// is not a number gives the [`check_numbers`] error, and a fraction or a negative number gives
/// "`{key}` must be a whole number".
fn whole_or(p: &Value, key: &str, default: u64, cmd: &str) -> Result<u64> {
    check_numbers(p, &[key], cmd)?;
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(default) };
    match (v.as_u64(), v.as_f64()) {
        (Some(n), _) => Ok(n),
        // A whole number past `u64::MAX` saturates to it.
        (None, Some(x)) if x >= 0.0 && x.fract() == 0.0 => Ok(x as u64),
        _ => Err(bad(cmd, format!("`{key}` must be a whole number"))),
    }
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
            st.history.recost_last(&st.doc);
        }
    }
}

// ---------- Reset Bounding Box ----------

fn reset_bounding_box(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let turned: Vec<NodeId> = ids.into_iter().filter(|id| d.node(*id).is_some_and(|n| n.bbox_angle != 0.0)).collect();
    if !turned.is_empty() {
        s.edit("Reset Bounding Box", |d, _| {
            for id in &turned {
                if let Some(n) = d.node_mut(*id) {
                    n.bbox_angle = 0.0;
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({ "changed": turned.len() }))
}

// ---------- Lock / Hide ----------

fn above(s: &mut Session, lock: bool) -> Result<Value> {
    let roots = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let symbols = d.symbol_boxes();
    // `selected` holds each selected object's visual bounds, by layer, with the index there of its
    // ancestor directly inside the layer. Symbol instances are measured by their art.
    let mut selected: BTreeMap<NodeId, Vec<(usize, Rect)>> = BTreeMap::new();
    for id in &roots {
        let Some(layer) = d.layer_of(*id) else { continue };
        // The selected object's ancestor directly inside the layer.
        let anc = d.ancestry(*id).unwrap_or_default();
        let Some(pos) = anc.iter().position(|a| *a == layer) else { continue };
        let top = anc.get(pos + 1).copied().unwrap_or(*id);
        let Some(b) = d.node(*id).and_then(|n| n.visual_bounds_with(&symbols)) else { continue };
        let Some((_, idx, _)) = d.position(top) else { continue };
        selected.entry(layer).or_default().push((idx, b));
    }
    // The loop walks up each layer once. Each object that is not selected and overlaps a selected
    // object below it is locked (Lock) or hidden (Hide).
    let roots: HashSet<NodeId> = roots.into_iter().collect();
    let mut hits: BTreeSet<NodeId> = BTreeSet::new();
    for (layer, mut below) in selected {
        below.sort_by_key(|(idx, _)| *idx);
        let mut below = below.into_iter().peekable();
        // `under` holds the bounds of the selected objects below the object at `i`.
        let mut under: Vec<Rect> = vec![];
        for (i, n) in d.children(Some(layer)).into_iter().flatten().enumerate() {
            while let Some((_, b)) = below.next_if(|(idx, _)| *idx < i) {
                under.push(b);
            }
            if under.is_empty() || roots.contains(&n.id) {
                continue;
            }
            if n.visual_bounds_with(&symbols).is_some_and(|nb| under.iter().any(|b| nb.intersect(*b).area() > 0.0 || nb.contains(b.center()))) {
                hits.insert(n.id);
            }
        }
    }
    // When there is no object above, or every one is locked (Lock) or hidden (Hide) already, the
    // document and its undo history stay as they are, and selected objects that are hidden or
    // locked are deselected.
    if hits.iter().all(|id| d.node(*id).is_none_or(|n| if lock { n.locked } else { !n.visible })) {
        s.select(|d, sel| sel.deselect_uneditable(d))?;
        return Ok(json!({ "count": hits.len() }));
    }
    let label = if lock { "Lock" } else { "Hide" };
    let n = hits.len();
    s.edit(label, |d, sel| {
        for id in &hits {
            if let Some(n) = d.node_mut(*id) {
                if lock {
                    n.locked = true;
                } else {
                    n.visible = false;
                }
            }
        }
        sel.deselect_uneditable(d);
        Ok(())
    })?;
    Ok(json!({ "count": n }))
}

fn other_layers(s: &mut Session, lock: bool) -> Result<Value> {
    let st = s.doc()?;
    let keep: Vec<NodeId> = st.selection.objects.iter().filter_map(|id| st.doc.ancestry(*id)).flatten().collect();
    let others: Vec<NodeId> = st.doc.layers.iter().map(|l| l.id).filter(|l| !keep.contains(l)).collect();
    let n = others.len();
    // When there is no other layer, or every one is locked (Lock) or hidden (Hide) already, the
    // document and its undo history stay as they are.
    if others.iter().all(|id| st.doc.node(*id).is_none_or(|l| if lock { l.locked } else { !l.visible })) {
        return Ok(json!({ "count": n }));
    }
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

/// The objects [`super::select::walk_selectable`] visits that are not among the selected objects `roots` and hold none of
/// them. The objects of a selected layer count as selected. Guide paths are left out while `guides_locked`, and so are
/// the objects that hold one. The walk goes into a plain group that holds a selected object or such a guide path; a
/// clip group, a Shaper group, a compound path and every other object that holds one is left out with its members.
fn unselected(d: &vectorcraft_doc::Document, iso: Option<NodeId>, selection: &[NodeId], roots: &[NodeId], guides_locked: bool) -> Vec<NodeId> {
    let mut roots: HashSet<NodeId> = roots.iter().copied().collect();
    roots.extend(selection.iter().filter_map(|id| d.node(*id)).filter(|n| n.is_layer()).flat_map(|l| l.layer_art(true)));
    let mut kept: Vec<NodeId> = roots.iter().copied().collect();
    if guides_locked {
        super::select::walk_selectable(d, iso, |n| {
            if matches!(n.kind, NodeKind::Path { guide: true, .. }) {
                kept.push(n.id);
            }
            true
        });
    }
    let holds: HashSet<NodeId> = d.index_paths(kept).values().filter_map(|p| d.ancestry_at(p)).flatten().collect();
    let mut out = vec![];
    super::select::walk_selectable(d, iso, |n| {
        if !holds.contains(&n.id) {
            out.push(n.id);
            return false;
        }
        !roots.contains(&n.id) && matches!(n.kind, NodeKind::Group { clip: false, .. }) && n.extras().shaper.is_none()
    });
    out
}

fn hide_unselected(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots_to_transform(s)?;
    let guides_locked = s.menu.guides_locked;
    let st = s.doc()?;
    let ids = unselected(&st.doc, st.isolation, &st.selection.objects, &roots, guides_locked);
    // With nothing to hide, the document and its undo history stay as they are.
    if !ids.is_empty() {
        s.edit("Hide", |d, _| {
            // Hiding moves no node in the tree, so the paths found up front stay right.
            let paths = d.index_paths(ids.iter().copied());
            for id in &ids {
                if let Some(n) = paths.get(id).and_then(|p| d.node_mut_at(p)) {
                    n.visible = false;
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({ "count": ids.len() }))
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
    transform_each_as(s, p, "Transform Each")
}

/// Object → Transform → Transform Again (⌘D) repeating a Transform Each: the same per-object
/// transform again (or another copy), run as the command so each object turns about its own
/// bounds (see [`transform_again`](super::object::transform_again)).
pub(crate) fn transform_each_again(s: &mut Session, p: &Value) -> Result<Value> {
    transform_each_as(s, p, "Transform Again")
}

/// [`transform_each`] under an undo label. Outside an interaction it records what Transform Again
/// repeats; a dialog's preview records it as it commits (`Session::commit_interaction`).
fn transform_each_as(s: &mut Session, p: &Value, label: &str) -> Result<Value> {
    let roots = selected_roots(s)?;
    check_numbers(p, &["scaleH", "scaleV", "moveH", "moveV", "rotate"], "object.transformEach")?;
    // Bounded so coordinates stay finite (and the document stays serializable).
    let sh = f64_or(p, "scaleH", 100.0).clamp(-100_000.0, 100_000.0);
    let sv = f64_or(p, "scaleV", 100.0).clamp(-100_000.0, 100_000.0);
    let mh = f64_or(p, "moveH", 0.0).clamp(-1.0e7, 1.0e7);
    let mv = f64_or(p, "moveV", 0.0).clamp(-1.0e7, 1.0e7);
    let rot = f64_or(p, "rotate", 0.0);
    let rx = switch_or(p, "reflectX", false, "object.transformEach")?;
    let ry = switch_or(p, "reflectY", false, "object.transformEach")?;
    let random = switch_or(p, "random", false, "object.transformEach")?;
    let refi = whole_or(p, "reference", 4, "object.transformEach")?.min(8) as usize;
    let copy = switch_or(p, "copy", false, "object.transformEach")?;
    check_switches(p, &["strokes", "corners", "objects", "patterns"], "object.transformEach")?;
    let objects = super::object::transforms_objects(p);
    if sh == 0.0 || sv == 0.0 {
        return Err(bad("object.transformEach", "scale must be non-zero"));
    }
    let mut rng = Rng(whole_or(p, "seed", 0x9E37_79B9_7F4A_7C15, "object.transformEach")?.max(1));
    let mut sc = if sh != 100.0 || sv != 100.0 { super::object::scaling(s, p) } else { Default::default() };
    sc.patterns = super::object::transform_patterns(s, p, &roots)?;
    // With no object to transform, or without a copy when both scales are 100% and there is no
    // move, rotation or reflection (with or without Random), the document and its undo history
    // stay as they are.
    if roots.is_empty() || (!copy && sh == 100.0 && sv == 100.0 && mh == 0.0 && mv == 0.0 && rot == 0.0 && !rx && !ry) {
        return Ok(ids_json(&roots));
    }
    let ids = s.edit(label, |d, sel| {
        let targets = if copy { duplicate_in(d, sel, &roots, Affine::IDENTITY)? } else { roots.clone() };
        // Measure each object's geometric bounds first, with symbol instances measured by their art.
        let symbols = d.symbol_boxes();
        let boxes: Vec<Option<Rect>> = targets.iter().map(|id| d.node(*id).and_then(|n| n.geometric_bounds_with(&symbols))).collect();
        for (id, b) in targets.iter().zip(boxes) {
            let Some(b) = b else { continue };
            let (mut k_sh, mut k_sv, mut k_mh, mut k_mv, mut k_rot) = (sh, sv, mh, mv, rot);
            if random {
                k_sh = 100.0 + (sh - 100.0) * rng.next();
                k_sv = 100.0 + (sv - 100.0) * rng.next();
                k_mh = mh * (rng.next() * 2.0 - 1.0);
                k_mv = mv * (rng.next() * 2.0 - 1.0);
                k_rot = rot * (rng.next() * 2.0 - 1.0);
            }
            let o = vectorcraft_geom::reference_point(b, refi);
            // Reflect X / Reflect Y match Object › Transform › Transform Each and Effect › Distort › Transform:
            // Reflect X flips left-right (negate x), Reflect Y flips top-bottom (negate y).
            let refl = Affine::scale_non_uniform(if rx { -1.0 } else { 1.0 }, if ry { -1.0 } else { 1.0 });
            let m = Affine::translate((k_mh, k_mv))
                * about(o, Affine::rotate(-k_rot.to_radians()) * refl * Affine::scale_non_uniform(k_sh / 100.0, k_sv / 100.0));
            if let Some(n) = d.node_mut(*id) {
                super::object::transform_node(n, m, sc, objects);
            }
        }
        Ok(targets)
    })?;
    let st = s.doc_mut()?;
    if st.interaction.is_none() {
        st.last_transform = Some(crate::LastTransform::Each(p.clone()));
        st.last_perspective = None;
    }
    Ok(ids_json(&ids))
}

// ---------- Rasterize / Crop ----------

pub(crate) const MAX_PIXELS: f64 = 64.0e6;

/// A copy of `doc` whose only content is `nodes` (paint order), for offscreen rendering.
pub(crate) fn isolated_doc(doc: &vectorcraft_doc::Document, nodes: Vec<Node>) -> vectorcraft_doc::Document {
    let mut tmp = doc.clone();
    let mut layer = Node::layer(NodeId(u64::MAX), "raster", vectorcraft_doc::LayerColor::Preset(0));
    if let Some(ch) = layer.children_mut() {
        *ch = nodes.into_iter().map(Arc::new).collect();
    }
    tmp.layers = vec![Arc::new(layer)];
    tmp
}

pub(crate) fn unique_key(d: &vectorcraft_doc::Document, stem: &str) -> String {
    let mut i = d.images.len() + 1;
    loop {
        let k = format!("{stem}-{i}");
        if !d.images.contains_key(&k) {
            return k;
        }
        i += 1;
    }
}

/// `doc` rendered over `region` with `opts` as a PNG in colour model `color`, with the note that an
/// image in it couldn't be decoded ([`super::rasterfx::undecoded_warning`]).
fn render_png(
    doc: &vectorcraft_doc::Document,
    region: Rect,
    scale: f64,
    opts: &vectorcraft_render::RenderOptions,
    color: RasterColorModel,
) -> Result<(Vec<u8>, u32, u32, Option<&'static str>)> {
    if region.width() * region.height() * scale * scale > MAX_PIXELS || region.width() * scale > 65535.0 || region.height() * scale > 65535.0 {
        return Err(EngineError::Other("image would be too large; lower the resolution".into()));
    }
    let mut r = vectorcraft_render::Renderer::new();
    let failed = vectorcraft_render::decode_failures();
    let mut img = r.render_region_with(doc, region, scale, opts);
    let warning = super::rasterfx::undecoded_warning(failed);
    RasterEffectsSettings { color_model: color, ..Default::default() }.finish_pixels(&mut img.pixels);
    Ok((img.to_png().map_err(EngineError::Other)?, img.width, img.height, warning))
}

/// The image Object › Rasterize makes of some art, before it goes in a document
/// ([`rasterize_art`]).
pub(crate) struct Rasterized {
    png: Vec<u8>,
    width: u32,
    height: u32,
    /// Image pixels → document.
    xf: Affine,
    /// The art's outline, with Create Clipping Mask.
    clip: Option<(PathData, vectorcraft_geom::FillRule)>,
    /// The note that an image in the art couldn't be decoded ([`render_png`]).
    warning: Option<&'static str>,
}

impl Rasterized {
    /// The image as a new object of `d` (its pixels in `d.images`), in a clip group with the art's
    /// outline when it has one.
    pub(crate) fn into_node(self, d: &mut vectorcraft_doc::Document) -> Node {
        let key = unique_key(d, "raster");
        d.images.insert(key.clone(), vectorcraft_doc::ImageBlob::new("image/png", self.png));
        let id = d.alloc_id();
        let image = ImageObject { key, width: self.width, height: self.height, xf: self.xf, link: None, placement: Default::default(), pixels: None };
        let node = Node::new(id, NodeKind::Image(Box::new(image)));
        match self.clip {
            Some((path, rule)) => super::rasterfx::clip_group(d, path, rule, node),
            None => node,
        }
    }
}

/// Object › Rasterize's image of `nodes` (drawn also where hidden) with the params `p` of command
/// `cmd`, each defaulting to `doc`'s raster effects settings: over the bounds of what the art draws
/// ([`vectorcraft_render::drawn_bounds`]) and the room around them, in whole pixels.
pub(crate) fn rasterize_art(doc: &vectorcraft_doc::Document, nodes: Vec<Node>, p: &Value, cmd: &str) -> Result<Rasterized> {
    check_numbers(p, &["ppi"], cmd)?;
    let ppi = f64_or(p, "ppi", doc.raster_effects_ppi);
    if !(1.0..=2400.0).contains(&ppi) {
        return Err(bad(cmd, "ppi must be between 1 and 2400"));
    }
    // The document's raster effects settings, changed by the params.
    let mut look = doc.raster_effects.clone();
    let mut background = (look.background == vectorcraft_doc::Background::White).then_some([255; 3]);
    let mut anti_alias = if look.anti_alias { AntiAlias::Art } else { AntiAlias::None };
    for (k, v) in p.as_object().into_iter().flatten().filter(|(_, v)| !v.is_null()) {
        match k.as_str() {
            "background" => background = fileio::background(v).map_err(|e| bad(cmd, e))?,
            "antiAlias" => {
                anti_alias = match v {
                    Value::Bool(b) => {
                        if *b {
                            AntiAlias::Art
                        } else {
                            AntiAlias::None
                        }
                    }
                    v => fileio::anti_alias(v.as_str().unwrap_or_default()).map_err(|e| bad(cmd, e))?,
                }
            }
            "padding" | "addAround" => look.add_around = super::rasterfx::add_around(v, cmd, k)?,
            "colorModel" => look.color_model = super::rasterfx::color_model(v, doc.color_mode, cmd)?,
            "clippingMask" => look.clipping_mask = v.as_bool().ok_or_else(|| bad(cmd, "clippingMask must be true or false"))?,
            _ => {}
        }
    }
    let raster = RasterExportOptions { ppi, background, anti_alias, ..Default::default() };
    let pad = look.add_around;
    let nodes: Vec<Node> = nodes
        .into_iter()
        .map(|mut n| {
            n.visible = true;
            n
        })
        .collect();
    // What the art draws, its geometry effects applied.
    let b = nodes.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, vectorcraft_render::drawn_bounds(doc, n)));
    let b = b.ok_or_else(|| bad(cmd, "selection has no bounds"))?.inflate(pad, pad);
    let scale = ppi / 72.0;
    // Snap to whole pixels.
    let region = Rect::new(b.x0, b.y0, b.x0 + (b.width() * scale).ceil().max(1.0) / scale, b.y0 + (b.height() * scale).ceil().max(1.0) / scale);
    // Create Clipping Mask: the image is clipped to the art's outline with its geometry effects
    // applied, as on the canvas.
    let clip = look
        .clipping_mask
        .then(|| {
            let outlines = nodes.iter().filter_map(|n| {
                let (bp, rule) = vectorcraft_render::drawn_outline(doc, n)?;
                let mut p = Node::path(n.id, PathData::from_bezpath(&bp), Appearance::default());
                if let NodeKind::Path { rule: r, .. } = &mut p.kind {
                    *r = rule;
                }
                Some(Arc::new(p))
            });
            vectorcraft_render::effects::clip_outline(&Node::group(NodeId(u64::MAX), outlines.collect()))
        })
        .flatten()
        .map(|(bp, rule)| (PathData::from_bezpath(&bp), rule));
    let tmp = isolated_doc(doc, nodes);
    let (png, width, height, warning) = render_png(&tmp, region, scale, &raster.render_options(RasterFormat::Png), look.color_model)?;
    let xf = Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale);
    Ok(Rasterized { png, width, height, xf, clip, warning })
}

fn rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.rasterize";
    let roots = selected_roots(s)?;
    let st = s.doc()?;
    let nodes: Vec<Node> = roots.iter().filter_map(|id| st.doc.node(*id).cloned()).collect();
    let image = rasterize_art(&st.doc, nodes, p, C)?;
    let (w, h, warning) = (image.width, image.height, image.warning);
    if roots.is_empty() {
        return Err(bad(C, "nothing selected"));
    }
    let id = s.edit("Rasterize", |d, sel| {
        let threaded = super::threads::frames(d);
        let node = image.into_node(d);
        let id = d.wrap(&roots, |_| node)?;
        // Rasterized frames of threaded text leave their threads with the text they show.
        super::threads::leave_threads(d, &threaded);
        sel.set([id]);
        Ok(id)
    })?;
    let mut out = json!({ "id": id.0, "width": w, "height": h });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

fn has_image(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
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
    let NodeKind::Image(im) = &node.kind else { return Err(bad(C, "select an image")) };
    let blob = st.doc.images.get(&im.key).filter(|b| !b.bytes.is_empty()).ok_or_else(|| bad(C, "the image has no embedded pixels"))?;
    let raster = vectorcraft_trace::Raster::decode(&blob.bytes).map_err(|e| bad(C, e.to_string()))?;
    let cols = whole_or(p, "columns", 10, C)?.clamp(1, 1000) as u32;
    let rows = whole_or(p, "rows", 10, C)?.clamp(1, 1000) as u32;
    let gray = switch_or(p, "gray", false, C)?;
    let colors = vectorcraft_trace::mosaic(&raster, cols, rows);
    // Tiles in image space, placed by the image transform (scaled to a new size if asked).
    let b = node.geometric_bounds().ok_or_else(|| bad(C, "image has no bounds"))?;
    check_numbers(p, &["width", "height", "spacingX", "spacingY"], C)?;
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
    let delete = switch_or(p, "deleteRaster", false, C)?;
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
    let NodeKind::Image(im) = &node.kind else { return Err(bad(C, "select an image")) };
    if switch_or(p, "trim", false, C)? {
        return trim_image(s, id, (**im).clone());
    }
    let ib = node.geometric_bounds().ok_or_else(|| bad(C, "image has no bounds"))?;
    let rect = match p.get("rect").filter(|v| !v.is_null()) {
        Some(Value::Array(a)) if a.len() == 4 => {
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
    bare.appearance = Appearance::default().into();
    bare.visible = true;
    let tmp = isolated_doc(&st.doc, vec![bare]);
    let region = Rect::new(r.x0, r.y0, r.x0 + (r.width() * scale).round().max(1.0) / scale, r.y0 + (r.height() * scale).round().max(1.0) / scale);
    let (png, w, h, warning) =
        render_png(&tmp, region, scale, &RasterExportOptions::default().render_options(RasterFormat::Png), RasterColorModel::Document)?;
    s.edit("Crop Image", |d, _| {
        let key = unique_key(d, "crop");
        d.images.insert(key.clone(), vectorcraft_doc::ImageBlob::new("image/png", png));
        if let Some(n) = d.node_mut(id) {
            n.kind = NodeKind::Image(Box::new(ImageObject {
                key,
                width: w,
                height: h,
                xf: Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale),
                link: None,
                placement: Default::default(),
                pixels: None,
            }));
        }
        Ok(())
    })?;
    let mut out = json!({ "id": id.0, "width": w, "height": h });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

/// Crop Image's trim: the image cut to the box of its pixels that aren't fully transparent, the
/// pixels copied as they are (no resampling) and placed where they were.
fn trim_image(s: &mut Session, id: NodeId, im: ImageObject) -> Result<Value> {
    const C: &str = "object.cropImage";
    let st = s.doc()?;
    let blob = st.doc.images.get(&im.key).ok_or_else(|| bad(C, "the image's pixels are missing"))?;
    // A CMYK image (kept as its ink amounts) has no alpha: nothing to cut.
    if blob.is_cmyk() {
        return Ok(json!({ "id": id.0, "width": im.width, "height": im.height, "trimmed": false }));
    }
    // Print rasters run to gigabytes decoded: the same allowance opening them has.
    let reader = || {
        image::ImageReader::new(std::io::Cursor::new(blob.bytes.as_slice()))
            .with_guessed_format()
            .map_err(|_| bad(C, "the image's pixels can't be read"))
    };
    // The image's size as RGBA, read from its header, is held to that allowance before anything
    // is decoded (#1717).
    let (w, h) = reader()?.into_dimensions().map_err(|e| bad(C, format!("the image's pixels can't be read: {e}")))?;
    if u64::from(w) * u64::from(h) > crate::cmd::fileio::MAX_RASTER_ALLOC / 4 {
        return Err(bad(C, format!("an image of {w} × {h} pixels is too large to draw")));
    }
    let mut reader = reader()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(crate::cmd::fileio::MAX_RASTER_ALLOC);
    reader.limits(limits);
    let px = reader.decode().map_err(|e| bad(C, format!("the image's pixels can't be read: {e}")))?.to_rgba8();
    let (w, h) = px.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in px.enumerate_pixels() {
        if p[3] > 0 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return Err(bad(C, "the image is fully transparent"));
    }
    if (x0, y0, x1, y1) == (0, 0, w, h) {
        // Already trimmed: not an error, so a script can trim every image it opens.
        return Ok(json!({ "id": id.0, "width": w, "height": h, "trimmed": false }));
    }
    let cut = image::imageops::crop_imm(&px, x0, y0, x1 - x0, y1 - y0).to_image();
    let mut png = Vec::new();
    cut.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| bad(C, e.to_string()))?;
    let (cw, ch) = (x1 - x0, y1 - y0);
    s.edit("Crop Image", |d, _| {
        let key = unique_key(d, "crop");
        d.images.insert(key.clone(), vectorcraft_doc::ImageBlob::new("image/png", png));
        if let Some(n) = d.node_mut(id) {
            n.kind = NodeKind::Image(Box::new(ImageObject {
                key,
                width: cw,
                height: ch,
                xf: im.xf * Affine::translate((f64::from(x0), f64::from(y0))),
                link: None,
                placement: Default::default(),
                pixels: None,
            }));
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0, "width": cw, "height": ch, "trimmed": true }))
}

/// The Control bar's Mask for an image: a clip group of the image and a clipping path on its
/// outline, with that path selected so dragging its handles crops the image.
fn mask_image(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let (id, outline) = st
        .selection
        .objects
        .iter()
        .find_map(|id| match st.doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::Image(im)) => {
                Some((*id, vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, im.width as f64, im.height as f64)).transformed(im.xf)))
            }
            _ => None,
        })
        .ok_or_else(|| bad("object.maskImage", "select an image"))?;
    let (gid, pid) = s.edit("Mask", |d, sel| {
        let mut clip = super::pathops::shape_node(d, outline, None);
        super::object::as_clipping_path(&mut clip)?;
        let pid = clip.id;
        let (par, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        let gid = d.alloc_id();
        d.insert(par, idx + 1, Node::new(gid, NodeKind::Group { children: vec![], clip: true }))?;
        d.insert(Some(gid), 0, clip)?;
        d.move_node(id, Some(gid), usize::MAX)?;
        sel.set([pid]);
        Ok((gid, pid))
    })?;
    Ok(json!({ "id": gid.0, "path": pid.0 }))
}

// ---------- Trim marks ----------

impl Session {
    /// The trim mark style the preferences ask for (General → Use Japanese Crop Marks).
    pub fn crop_mark_style(&self) -> MarkStyle {
        if self.prefs.japanese_crop_marks { MarkStyle::Japanese } else { MarkStyle::Roman }
    }
}

fn trim_marks(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.createTrimMarks";
    check_numbers(p, &["offset", "length", "bleed", "weight"], C)?;
    let style = match p.get("style") {
        None | Some(Value::Null) => s.crop_mark_style(),
        Some(v) => v.as_str().and_then(MarkStyle::parse).ok_or_else(|| bad(C, "`style` must be roman or japanese"))?,
    };
    let d = TrimMarks::default();
    let marks = TrimMarks {
        style,
        offset: f64_or(p, "offset", d.offset).max(0.0),
        length: f64_or(p, "length", d.length).max(0.1),
        bleed: f64_or(p, "bleed", d.bleed).max(0.0),
        weight: f64_or(p, "weight", d.weight).max(0.01),
    };
    let st = s.doc()?;
    let rects: Vec<Rect> = if switch_or(p, "allArtboards", st.selection.is_empty(), C)? {
        st.doc.artboards.iter().map(|a| a.rect).collect()
    } else {
        st.doc.bounds_of(&selected_roots(s)?, true).into_iter().collect()
    };
    if rects.is_empty() {
        return Err(bad(C, "nothing to mark"));
    }
    let parent = s.doc()?.target_parent()?;
    let ids = s.edit("Create Trim Marks", |d, sel| {
        let mut ids = vec![];
        for r in &rects {
            let g = marks.group(*r, &mut || d.alloc_id());
            ids.push(g.id);
            d.insert(parent, usize::MAX, g)?;
        }
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    Ok(json!({ "id": ids[0].0, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
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
        return Some(LiveShape::Rectangle {
            w,
            h,
            radii: [0.0; 4],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::translate(a[0].p.to_vec2()) * rot * flip,
        });
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
                *live = Some(Box::new(l));
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
        // Measure each object's geometric bounds first, with symbol instances measured by their art.
        let symbols = d.symbol_boxes();
        let boxes: Vec<(NodeId, Rect)> = roots.iter().filter_map(|id| Some((*id, d.node(*id)?.geometric_bounds_with(&symbols)?))).collect();
        let mut converted = vec![];
        for (id, b) in boxes {
            if b.width() <= 0.0 || b.height() <= 0.0 {
                continue;
            }
            // Named one past the highest artboard id the document has, whatever id it is given.
            let number = d.artboards.iter().map(|a| a.id).max().map_or(1, |m| m.saturating_add(1));
            let aid = d.next_artboard_id();
            d.artboards.push(vectorcraft_doc::Artboard {
                id: aid,
                name: format!("Artboard {number}"),
                rect: b,
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            });
            converted.push(id);
        }
        if converted.is_empty() {
            return Err(EngineError::Other("Convert to Artboards: selection has no area".into()));
        }
        d.remove_many(&converted)?;
        sel.clear();
        Ok(converted.len())
    })?;
    Ok(json!({ "artboards": n }))
}

fn rearrange_artboards(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "artboard.rearrange";
    use super::newdoc::ArtboardLayout as L;
    // A value given (not null) must be one of the names.
    let named = |key: &str| p.get(key).filter(|v| !v.is_null()).map(|v| v.as_str().unwrap_or_default());
    let by_column = switch_or(p, "byColumn", false, CMD)?;
    let layout = match named("layout") {
        Some(l) => L::parse(l).ok_or_else(|| bad(CMD, "layout must be gridByRow, gridByColumn, row or column"))?,
        None if by_column => L::GridByColumn,
        None => L::GridByRow,
    };
    let rtl = match named("order") {
        None => false,
        Some(o) if o.eq_ignore_ascii_case("leftToRight") => false,
        Some(o) if o.eq_ignore_ascii_case("rightToLeft") => true,
        Some(_) => return Err(bad(CMD, "order must be leftToRight or rightToLeft")),
    };
    let count = s.doc()?.doc.artboards.len();
    let n_ab = count.max(1);
    // Columns of a grid by row, rows of a grid by column.
    let lines = whole_or(p, "columns", 2, CMD)?.clamp(1, n_ab as u64) as usize;
    let (rows, cols) = match layout {
        L::GridByRow => (n_ab.div_ceil(lines), lines),
        L::GridByColumn => (lines, n_ab.div_ceil(lines)),
        L::Row => (1, n_ab),
        L::Column => (n_ab, 1),
    };
    check_numbers(p, &["spacing"], CMD)?;
    let spacing = f64_or(p, "spacing", 20.0).clamp(-1.0e5, 1.0e5);
    let move_art = switch_or(p, "moveArtwork", true, CMD)?;
    check_switches(p, &["patterns"], CMD)?;
    let locked_and_hidden = s.prefs.move_locked_with_artboard;
    let d = &s.doc()?.doc;
    let rects: Vec<Rect> = d.artboards.iter().map(|a| a.rect).collect();
    let sizes: Vec<(f64, f64)> = rects.iter().map(|r| (r.width(), r.height())).collect();
    let first = rects.first().map_or(Point::ZERO, |r| r.origin());
    let origins = super::newdoc::grid_origins_rc(&sizes, first, (rows, cols), spacing, layout == L::GridByColumn, rtl);
    // A locked artboard stays where it is, and keeps its place in the grid.
    let deltas: Vec<Vec2> =
        rects.iter().zip(origins).zip(&d.artboards).map(|((r, o), a)| if a.locked { Vec2::ZERO } else { o - r.origin() }).collect();
    // When no artboard moves more than 1e-9 pt on either axis, or there is none, the document and
    // its undo history stay as they are.
    if deltas.iter().all(|dl| dl.x.abs() <= 1e-9 && dl.y.abs() <= 1e-9) {
        return Ok(json!({ "artboards": count, "rows": rows, "columns": cols }));
    }
    // With moveArtwork, each top-level object (a child of a layer or sublayer) moves with the
    // artboard its center is on, with symbol instances measured by their art as
    // `Document::art_on_artboard` measures them; locked and hidden art, and art on a locked
    // artboard, moves only when prefs moveLockedWithArtboard is on.
    fn objects<'a>(n: &'a Node, all: bool, out: &mut Vec<&'a Node>) {
        for c in n.children().into_iter().flatten().filter(|c| c.rides_with_artboard(all)) {
            if c.is_layer() { objects(c, all, out) } else { out.push(c) }
        }
    }
    let locks = d.locked_artboards();
    let symbols = d.symbol_boxes();
    let art: Vec<(NodeId, Vec2)> = if move_art {
        let mut tops = vec![];
        for l in d.layers.iter().filter(|l| l.rides_with_artboard(locked_and_hidden)) {
            objects(l, locked_and_hidden, &mut tops);
        }
        tops.into_iter()
            .filter(|n| locked_and_hidden || !locks.holds(n))
            .filter_map(|n| {
                let c = n.geometric_bounds_with(&symbols)?.center();
                let dl = *d.artboard_holding(c, false).and_then(|i| deltas.get(i))?;
                (dl != Vec2::ZERO).then_some((n.id, dl))
            })
            .collect()
    } else {
        vec![]
    };
    let ids: Vec<NodeId> = art.iter().map(|(id, _)| *id).collect();
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &ids)?, ..Scaling::default() };
    s.edit("Rearrange Artboards", |d, _| {
        for (id, dl) in &art {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dl), sc);
            }
        }
        let mut moved = vec![];
        for (a, dl) in d.artboards.iter_mut().zip(&deltas) {
            a.rect = a.rect + *dl;
            moved.push((a.id, *dl));
        }
        for (id, dl) in moved {
            d.move_artboard_guides(id, dl);
        }
        Ok(())
    })?;
    Ok(json!({ "artboards": count, "rows": rows, "columns": cols }))
}
