//! Creating art: shapes, paths, text.

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, CharStyle, LiveShape, MAX_COORD, Node, NodeKind, TextObject};
use vectorcraft_geom::curvature::{self, NoCircle};
use vectorcraft_geom::{Affine, Anchor, AnchorKind, FillRule, PathData, Point, Rect, SubPath, shapes};

use super::newart::NewArt;
use super::{
    CommandSpec, NodeId, Result, Session, bad, bool_or, check_numbers, check_switches, cmd, color_value, f64_or, f64_req, finite_numbers, has_doc,
    index_opt, number_or, object_param, str_param, switch_or,
};
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "shape.rectangle",
            "Rectangle",
            [],
            None,
            "{x, y, width, height, radius?: pt, rotation?: deg clockwise about the centre (0: upright)} → {id}",
            has_doc,
            rectangle
        ),
        cmd!(
            "shape.ellipse",
            "Ellipse",
            [],
            None,
            "{x, y, width, height, rotation?: deg clockwise about the centre (0: upright), anchors=4 (2–1000: smooth anchors spaced evenly by angle; any count but 4 draws a plain path, not a live ellipse)} → {id}",
            has_doc,
            ellipse
        ),
        cmd!(
            "shape.polygon",
            "Polygon",
            [],
            None,
            "{cx, cy, radius, sides=6, rotation?: deg clockwise (0: a corner straight up)} → {id}",
            has_doc,
            polygon
        ),
        cmd!(
            "shape.star",
            "Star",
            [],
            None,
            "{cx, cy, radius1, radius2, points=5, rotation?: deg clockwise (0: a point straight up)} → {id}",
            has_doc,
            star
        ),
        cmd!(
            "shape.flare",
            "Flare",
            [],
            None,
            "{cx, cy, diameter=100, opacity=50 (%), brightness=30 (%), growth=20 (%), fuzziness=50 (%), rays=15, longest=300 (%), rayFuzziness=100 (%), rotation=0 (deg, turns the rays clockwise), x2?, y2? (ring end, both or neither: one alone is an error; else pathLength at direction), pathLength=300, rings=10, largest=50 (%), direction=45 (deg counter-clockwise from 3 o'clock: where the rings run from the centre), seed?, ringSeed? (whole number below 2^53: deterministically places every ring along the path, including the first for small seed values, keeping their count and sizes)} → {id}",
            has_doc,
            super::flare::flare
        ),
        cmd!(
            "shape.editFlare",
            "Edit Flare",
            [],
            None,
            "{ids?|id?: flares (default: the selected flares), cx?, cy? (the center, where it is drawn now), x2?, y2? (the rings' end point, where it is drawn now; else pathLength at direction from the center, both where it is drawn now, when either is given; else where it was), diameter?, opacity?, brightness?, growth?, fuzziness?, rays?, longest?, rayFuzziness?, rotation? (deg, turns the rays clockwise), pathLength?, rings?, largest?, direction? (as shape.flare: what is left out stays as it was)} draws each flare again in its place, moved, scaled, rotated, reflected or sheared as it has been since it was drawn, as one undo step (none when nothing changes); a flare whose parts were edited, or that is locked or hidden, is not one → {ids}",
            has_doc,
            super::flare::edit_flare
        ),
        cmd!(
            "shape.circleThroughPoints",
            "Circle Through Points",
            [],
            None,
            "{points?: [[x, y], [x, y]] or [[x, y], [x, y], [x, y]] (two: the ends of a diameter; left out: the two or three selected anchor points, where a path selected whole counts all its anchors and selected segments' ends do not count), anchors?: \"standard\"|\"points\" (default standard: four anchors at 0°, 90°, 180° and 270°; points: an anchor at each point and more between them so that no curve turns more than a quarter)} → {id, cx, cy, radius}: a circle with the current paint whose edge passes through the points, not pixel-aligned even while Align New Objects to Pixel Grid is on; two points that are the same, three on one line, or a circle reaching past the canvas are an error, and nothing is drawn",
            has_doc,
            circle_through_points
        ),
        cmd!(
            "shape.curvatureCircle",
            "Curvature Circle",
            [],
            None,
            "{id, subpath?=0, segment, t: 0..1, convex?=false} → {id, cx, cy, radius, nominal}: a circle with the current paint that touches path `id` at `t` on the segment and bends as the path bends there (its circle of curvature), on the side the path bends toward, or on the other side with convex; a segment that is the usual cubic of a circular arc of 90° or less gives a circle of that arc's own radius (nominal: true), on the concave side the arc's own circle; not pixel-aligned even while Align New Objects to Pixel Grid is on; a straight stretch, a cusp, a corner anchor (t 0 or 1 where two segments meet at an angle of more than half a degree and the anchor is not marked smooth with both handles pulled out, or where they turn by 90° or more; segments of no length between them are passed over, and an anchor reached past one is treated as not marked smooth), a guide, or a circle reaching past the canvas is an error, and nothing is drawn",
            has_doc,
            curvature_circle
        ),
        cmd!("shape.line", "Line Segment", [], None, "{x1, y1, x2, y2, fill?: bool (Fill Line: the current fill, else none)} → {id}", has_doc, line),
        cmd!(
            "shape.spiral",
            "Spiral",
            [],
            None,
            "{cx, cy, radius, decay=80 (%), segments=10, clockwise?, rotation?: deg clockwise about (cx, cy)} (0: outer end at (cx + radius, cy)) → {id}",
            has_doc,
            spiral
        ),
        cmd!(
            "shape.arc",
            "Arc",
            [],
            None,
            "{x1, y1, x2, y2, closed?, slope?: −100…100 (default 100), rotation?: deg clockwise about (x1, y1)} or {x, y, lengthX, lengthY, reference?: 0–8 (default 0), baseAlong?: x|y (either case; default x), slope?: −100…100 (default 50), closed?, fillArc?, rotation?: deg clockwise about the reference point (x, y)} (0: upright); a reference or baseAlong given as anything else (null included) or a slope outside its range is an error and draws nothing → {id}",
            has_doc,
            arc
        ),
        cmd!(
            "shape.threePointArc",
            "Three-Point Arc",
            [],
            None,
            "{start: [x, y], end: [x, y], through?: [x, y], toward?: [x, y], at?: \"start\"|\"end\" (default \"start\"; only with `toward`)} → {id, cx, cy, radius, sweep}: an open circular arc from `start` to `end` in the current stroke (black when it is none) with no fill, its ends exactly at the two points (not pixel-aligned). With `through`, the arc passes through that point, an anchor of the path; with `toward`, it leaves the end `at` names heading toward that point (its tangent there). Give exactly one of `through` and `toward`; `at` with `through` is an error. `sweep` is the arc's turn in degrees, positive clockwise on the page. Two points that are the same, three on one line, or an arc reaching past the canvas are an error, and nothing is drawn",
            has_doc,
            three_point_arc
        ),
        cmd!(
            "shape.rectangularGrid",
            "Rectangular Grid",
            [],
            None,
            "{x, y, width, height, rows=5, columns=5 (whole numbers; at most 999), rowSkew=0, columnSkew=0 (−500…500 %: + crowds the horizontal dividers toward the top, the vertical ones toward the right), frame=true (the outside as one rectangle, else its four sides), fill=false (the current fill, else none), rotation?: deg clockwise about the centre (0: upright)} → {id}",
            has_doc,
            rect_grid
        ),
        cmd!(
            "shape.polarGrid",
            "Polar Grid",
            [],
            None,
            "{x, y, width, height, concentric=5, radial=5 (whole numbers; at most 999), concentricSkew=0, radialSkew=0 (−500…500 %: + crowds the ellipses toward the outside, the radial dividers clockwise of the top one), compound=false (the ellipses pair up into compound-path rings), fill=false (the current fill, else none), rotation?: deg clockwise about the centre (0: upright)} → {id}",
            has_doc,
            polar_grid
        ),
        cmd!(
            "path.create",
            "Create Path",
            [],
            None,
            "{anchors: [{x, y, in?: [x,y], out?: [x,y]}], closed?: bool, d?: SVG path data} → {id}",
            has_doc,
            path_create
        ),
        cmd!(
            "view.drawMode",
            "Drawing Mode",
            [],
            Some("Shift+D"),
            "{mode?: normal|behind|inside} (inside needs one selected path, compound path or type; no param cycles normal → behind → inside → normal, going from behind back to normal when Draw Inside can't draw into the selection; when the Draw Inside target is isolated, drawing or pasting into it moves isolation to its clip group)",
            has_doc,
            draw_mode
        ),
        cmd!(
            "text.create",
            "Create Text",
            [],
            None,
            "{x, y, text, rotation?: deg clockwise about x, y (0: level), vertical?: bool = false, size?: pt 0.1..1296 (outside it: an error, and no type is made), font?: family, style? (default Regular, or the family's closest style when it has no Regular), color? (#rrggbbaa and #rgba are refused; size, font, style and color left out come from [Normal Character Style], the defaults unless the document redefined it), keepModel?: bool (as paint.setFill), area?: {width, height, fit?: none|autoHeight|shrinkText|block, fitMinPercent?, wordGap?, lineGap? (as text.areaOptions)}, placeholder?: bool (fill it with placeholder text instead, as the Type tools do with prefs placeholderText), justify?: \"auto\" (default: the start of each paragraph's direction)|\"left\"|\"center\"|\"right\"|\"justifyAll\" (point type: aligned on x, so \"center\" centres each line on the point and \"right\" ends it there; text.setStyle changes it later), leadingModel?: \"romanBaseline\"|\"emBoxTop\", charAlign?: \"romanBaseline\"|\"emBoxTop\"|\"emBoxCenter\"|\"emBoxBottom\"|\"icfTop\"|\"icfBottom\" (default: emBoxTop and emBoxCenter while the interface is in Japanese, else romanBaseline)} → {id, bounds: {x, y, width, height} (the new type's box as document.inspect reports it, after justify), overflow?: [id] (area type whose text doesn't fit its frame: the lines past it aren't drawn; left out when it fits), warning?: text (the font or style given isn't available: the type is drawn in the fallback font or the family's closest style)}; new area type gets Auto Size (fit autoHeight) when the autoSizeAreaType preference is on",
            has_doc,
            text_create
        ),
    ]
}

/// Insert a new object looking like `look` (drawing modes apply); select it.
pub(crate) fn add_look(s: &mut Session, label: &str, kind: NodeKind, look: NewArt, name: Option<String>) -> Result<Value> {
    insert_look(s, label, kind, look, name, true)
}

/// [`add_look`] that selects the new object only when `select` is set; otherwise the selection
/// stays as it was.
pub(crate) fn insert_look(s: &mut Session, label: &str, kind: NodeKind, look: NewArt, name: Option<String>, select: bool) -> Result<Value> {
    insert_look_turned(s, label, kind, look, name, select, 0.0, true)
}

/// Insert a new object at the top of the insertion parent with the current paint and new-art
/// template, made turned by a `rotation` param (degrees, clockwise on the page), and select it:
/// its bounding box turns with it, as after the Rotate tool.
fn add_art_turned(s: &mut Session, label: &str, kind: NodeKind, rotation: f64) -> Result<Value> {
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    insert_look_turned(s, label, kind, look, None, true, bbox_turn(rotation), true)
}

/// [`add_art`] for an object whose geometry must stay where it was computed: it is not
/// pixel-aligned, even while the document's Align New Objects to Pixel Grid is on.
fn add_art_unaligned(s: &mut Session, label: &str, kind: NodeKind) -> Result<Value> {
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    insert_look_turned(s, label, kind, look, None, true, 0.0, false)
}

/// The bounding-box angle (counter-clockwise, as `Node::bbox_angle` and the Rotate tool keep it)
/// of an object made with the clockwise `rotation` param: 0 exactly for none, a whole turn or a
/// non-finite angle, so such objects are as they were.
fn bbox_turn(rotation: f64) -> f64 {
    if !rotation.is_finite() {
        return 0.0;
    }
    let a = vectorcraft_geom::normalize_deg(-rotation);
    if a == 0.0 { 0.0 } else { a }
}

/// A polygon's or star's `rotation` when it is the base orientation turned by General › Constrain
/// Angle (a Shift-drawn or click-made one), so its bounding box turns with it; else 0: a polygon
/// turned by the drag's direction keeps its box on the page's axes, as before.
fn constrained_turn(s: &Session, rotation: f64) -> f64 {
    match vectorcraft_geom::constrain_rotation(s.prefs.constrain_angle) {
        Some(r) if rotation.is_finite() && vectorcraft_geom::normalize_deg(r - rotation).abs() < 1e-9 => rotation,
        _ => 0.0,
    }
}

/// [`insert_look`] with the new object's bounding box turned to `bbox_angle`. With `align`, a new
/// path or compound path is pixel-aligned while the document's Align New Objects to Pixel Grid is
/// on.
#[allow(clippy::too_many_arguments)]
fn insert_look_turned(
    s: &mut Session,
    label: &str,
    kind: NodeKind,
    look: NewArt,
    name: Option<String>,
    select: bool,
    bbox_angle: f64,
    align: bool,
) -> Result<Value> {
    let parent = s.doc()?.target_parent()?;
    let mode = s.draw_mode;
    let inside = s.draw_inside;
    let behind_of = s.doc()?.selection.in_paint_order(&s.doc()?.doc).first().copied();
    let (id, into) = s.edit(label, |d, sel| {
        let id = d.alloc_id();
        let mut n = Node::new(id, kind);
        look.apply(d, &mut n);
        if align {
            super::object::align_new_object(d, &mut n);
        }
        n.name = name;
        n.bbox_angle = bbox_angle;
        let mut into = None;
        match (mode, inside) {
            (crate::DrawMode::Inside, Some(target)) if d.node(target).is_some() => {
                let group = draw_inside_group(d, target)?;
                d.insert(Some(group), usize::MAX, n)?;
                into = Some((target, group));
            }
            (crate::DrawMode::Behind, _) => match behind_of.and_then(|b| d.position(b)) {
                Some((par, idx, _)) => {
                    d.insert(par, idx, n)?;
                }
                None => {
                    d.insert(parent, 0, n)?;
                }
            },
            _ => {
                d.insert(parent, usize::MAX, n)?;
            }
        }
        if select {
            sel.set([id]);
        }
        Ok((id, into))
    })?;
    if let Some((target, group)) = into {
        isolate_draw_inside_group(s, target, group)?;
    }
    Ok(json!({ "id": id.0 }))
}

/// Draw Inside's clip group for `target`, the object new art goes inside: the clip group
/// [mask copy, target, …] `target` already is the clipped object of, else one made where `target`
/// was, holding an unpainted clipping copy of it and then `target`. New art and pastes go on top
/// of its children.
pub(crate) fn draw_inside_group(d: &mut vectorcraft_doc::Document, target: NodeId) -> Result<NodeId> {
    if let Some(p) = d.parent_of(target)
        && d.node(p).is_some_and(|pn| matches!(&pn.kind, NodeKind::Group { clip: true, children } if children.get(1).is_some_and(|c| c.id == target)))
    {
        return Ok(p);
    }
    let (par, idx, _) = d.position(target).ok_or(crate::EngineError::NoNode(target))?;
    // The mask is a copy with ids of its own, its subpaths' too (a compound path's).
    let original = d.node(target).cloned().ok_or(crate::EngineError::NoNode(target))?;
    let mut mask = d.reid(&original);
    mask.appearance = Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 0.0).into();
    if let NodeKind::Path { clipping, .. } = &mut mask.kind {
        *clipping = true;
    }
    let gid = d.alloc_id();
    d.insert(par, idx + 1, Node::new(gid, NodeKind::Group { children: vec![std::sync::Arc::new(mask)], clip: true }))?;
    d.move_node(target, Some(gid), usize::MAX)?;
    // Copied type whose layout caches `reid` dropped has them again.
    super::typecmd::refresh_bounds_of(d, &[gid]);
    Ok(gid)
}

/// Draw Inside into the isolated object (#3705): isolation mode moves to the clip group the art
/// went into ([`draw_inside_group`]), so what is drawn or pasted inside the object is in isolation
/// mode with it, shown undimmed and picked by clicks, instead of locked beside it. Art added in
/// isolation mode belongs with what is isolated: it takes the isolated art's place when the mode
/// ends (<https://helpx.adobe.com/illustrator/desktop/manage-objects/select-objects/isolate-objects.html>).
/// A drag's next preview, replayed on the drag's snapshot, makes the group again under the same
/// id, so isolation stays on it; cancelling the drag restores the isolation it began with.
pub(crate) fn isolate_draw_inside_group(s: &mut Session, target: NodeId, group: NodeId) -> Result<()> {
    let st = s.doc_mut()?;
    if st.isolation == Some(target) {
        st.isolation = Some(group);
    }
    Ok(())
}

/// The object Draw Inside would draw into: the one selected object, when it is a path, a compound
/// path or type.
fn inside_target(st: &crate::DocState) -> Option<NodeId> {
    let first = st.selection.objects.first().copied()?;
    let fits = matches!(st.doc.node(first).map(|n| &n.kind), Some(NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_)));
    (fits && st.selection.len() == 1).then_some(first)
}

fn draw_mode(s: &mut Session, p: &Value) -> Result<Value> {
    let mode = match str_param(p, "mode") {
        Some("behind") => crate::DrawMode::Behind,
        Some("inside") => crate::DrawMode::Inside,
        Some("normal") => crate::DrawMode::Normal,
        // Cycling (Shift+D): Draw Behind goes on to Draw Inside only when it can draw inside the
        // selection, else back to Draw Normal (#3705: with one group selected it stayed in Draw
        // Behind).
        None => match s.draw_mode {
            crate::DrawMode::Normal => crate::DrawMode::Behind,
            crate::DrawMode::Behind if inside_target(s.doc()?).is_some() => crate::DrawMode::Inside,
            _ => crate::DrawMode::Normal,
        },
        Some(o) => return Err(bad("view.drawMode", format!("unknown mode `{o}`"))),
    };
    if mode == crate::DrawMode::Inside {
        match inside_target(s.doc()?) {
            Some(t) => s.draw_inside = Some(t),
            None => return Err(bad("view.drawMode", "Draw Inside needs exactly one selected path, compound path or text")),
        }
    } else {
        s.draw_inside = None;
    }
    s.draw_mode = mode;
    Ok(json!({ "mode": mode }))
}

fn path_kind(path: PathData, live: Option<LiveShape>) -> NodeKind {
    NodeKind::Path { path, rule: FillRule::NonZero, live: live.map(Box::new), clipping: false, guide: false }
}

/// A click-to-size shape dialog's command `cmd` with `params` drawn along General › Constrain
/// Angle, as the reference app draws new objects along it
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/arrange-objects/rotate-objects-by-specific-angles.html>):
/// a box (rectangle, ellipse, grid) runs along the turned axes from `pivot`, the point the dialog
/// placed at the click (its top-left corner `x`, `y`, or a grid's reference point), which stays
/// put; an arc sized by its lengths turns about its reference point `x`, `y`, which the dialog
/// placed at the click; a polygon, star or spiral turns its base orientation about its centre. The params as they are at
/// 0° or a whole turn, for other commands and when they already carry a `rotation`.
pub fn constrain_click(s: &Session, cmd: &str, mut params: Value, pivot: Point) -> Value {
    let base = s.prefs.constrain_angle;
    let (Some(rotation), Some(axes)) = (vectorcraft_geom::constrain_rotation(base), vectorcraft_geom::constrain_axes(base)) else { return params };
    let Some(o) = params.as_object_mut().filter(|o| !o.contains_key("rotation")) else { return params };
    match cmd {
        "shape.rectangle" | "shape.ellipse" | "shape.rectangularGrid" | "shape.polarGrid" => {
            let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
            let (Some(x), Some(y), Some(w), Some(h)) = (num("x"), num("y"), num("width"), num("height")) else { return params };
            // The box's centre along the turned axes from the pivot; the command turns it there.
            let c = pivot + (axes * Point::new(x + w / 2.0 - pivot.x, y + h / 2.0 - pivot.y)).to_vec2();
            o.insert("x".into(), json!(c.x - w / 2.0));
            o.insert("y".into(), json!(c.y - h / 2.0));
            o.insert("rotation".into(), json!(rotation));
        }
        "shape.polygon" | "shape.star" | "shape.spiral" => {
            o.insert("rotation".into(), json!(rotation));
        }
        "shape.arc" if o.contains_key("lengthX") || o.contains_key("lengthY") => {
            o.insert("rotation".into(), json!(rotation));
        }
        _ => {}
    }
    params
}

fn rect_of(p: &Value, cmd: &str) -> Result<Rect> {
    let x = f64_req(p, "x", cmd)?;
    let y = f64_req(p, "y", cmd)?;
    let w = f64_req(p, "width", cmd)?;
    let h = f64_req(p, "height", cmd)?;
    if !(w.is_finite() && h.is_finite()) {
        return Err(bad(cmd, "invalid size"));
    }
    Ok(Rect::new(x, y, x + w, y + h).abs())
}

fn rect_transform(r: Rect, p: &Value, cmd: &str) -> Result<Affine> {
    let rotation = if p.get("rotation").is_some() { f64_req(p, "rotation", cmd)? } else { 0.0 };
    if !rotation.is_finite() {
        return Err(bad(cmd, "rotation must be finite"));
    }
    if rotation == 0.0 {
        return Ok(Affine::translate(r.origin().to_vec2()));
    }
    // Exact at quarter turns (a Constrain Angle of 90°).
    Ok(Affine::translate(r.center().to_vec2()) * vectorcraft_geom::rotate_deg(rotation) * Affine::translate((-r.width() / 2.0, -r.height() / 2.0)))
}

fn rectangle(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["radius"], "shape.rectangle")?;
    let r = rect_of(p, "shape.rectangle")?;
    let radius = f64_or(p, "radius", 0.0).max(0.0);
    let live = LiveShape::Rectangle {
        w: r.width(),
        h: r.height(),
        radii: [radius; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: rect_transform(r, p, "shape.rectangle")?,
    };
    let label = if radius > 0.0 { "Rounded Rectangle" } else { "Rectangle" };
    add_art_turned(s, label, path_kind(live.to_path(), Some(live)), f64_or(p, "rotation", 0.0))
}

/// The most anchors `shape.ellipse` draws, as many as a polygon's sides.
const MAX_ELLIPSE_ANCHORS: u64 = 1000;

fn ellipse(s: &mut Session, p: &Value) -> Result<Value> {
    let r = rect_of(p, "shape.ellipse")?;
    // One message for every number that is not a whole count in range (a fraction, a negative,
    // 0, 1, too many); a value that is not a number gets `number_or`'s.
    let (key, min, max) = ("anchors", 2, MAX_ELLIPSE_ANCHORS);
    let n = number_or(p, key, 4.0, "shape.ellipse")?;
    if n.fract() != 0.0 || !(min as f64..=max as f64).contains(&n) {
        return Err(bad("shape.ellipse", format!("`{key}` must be a whole number between {min} and {max} (got {n})")));
    }
    let x = n as u64;
    let xf = rect_transform(r, p, "shape.ellipse")?;
    let rotation = f64_or(p, "rotation", 0.0);
    if x != 4 {
        // Another count is a plain path: a live ellipse always has 4 anchors.
        let path = shapes::ellipse_with_anchors(Rect::new(0.0, 0.0, r.width(), r.height()), x as u32).transformed(xf);
        return add_art_turned(s, "Ellipse", path_kind(path, None), rotation);
    }
    let live = LiveShape::Ellipse { w: r.width(), h: r.height(), pie: (0.0, 360.0), xf };
    add_art_turned(s, "Ellipse", path_kind(live.to_path(), Some(live)), rotation)
}

/// A live ellipse whose edge passes through two or three points: `points`, or the two or three
/// selected anchor points ([`selected_anchor_points`]). Two points are the ends of a diameter. With
/// `anchors: "points"` the circle is a plain path with an anchor at each point
/// ([`shapes::circle_with_anchors_at`]); otherwise it is a live ellipse. One undo step, named as an
/// Ellipse tool drag is.
fn circle_through_points(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shape.circleThroughPoints";
    let points = match p.get("points").filter(|v| !v.is_null()) {
        Some(v) => two_or_three_points(v).ok_or_else(|| bad(C, "`points` must be two or three [x, y] pairs of numbers"))?,
        None => selected_anchor_points(s)?.ok_or_else(|| bad(C, "direct-select two or three anchor points, or give `points`"))?,
    };
    let key = "anchors";
    let at_points = match p.get(key).map(Value::as_str) {
        None | Some(Some("standard")) => false,
        Some(Some("points")) => true,
        Some(_) => return Err(bad(C, format!("`{key}` must be one of {}", "standard, points"))),
    };
    let (center, radius) = shapes::circle_through_points(&points)
        .ok_or_else(|| bad(C, "no circle passes through these points: two are the same or all three lie on one line"))?;
    // Not pixel-aligned: the circle stays through the points.
    let mut out = if at_points {
        add_art_unaligned(s, "Ellipse", path_kind(shapes::circle_with_anchors_at(center, radius, &points), None))?
    } else {
        let r = Rect::new(center.x - radius, center.y - radius, center.x + radius, center.y + radius);
        let live = LiveShape::Ellipse { w: r.width(), h: r.height(), pie: (0.0, 360.0), xf: Affine::translate(r.origin().to_vec2()) };
        add_art_unaligned(s, "Ellipse", path_kind(live.to_path(), Some(live)))?
    };
    out["cx"] = json!(center.x);
    out["cy"] = json!(center.y);
    out["radius"] = json!(radius);
    Ok(out)
}

/// The two or three points `[[x, y], [x, y]]` or `[[x, y], [x, y], [x, y]]` holds, each pair two
/// finite numbers.
fn two_or_three_points(v: &Value) -> Option<Vec<Point>> {
    let pair = |v: &Value| match v.as_array().map(Vec::as_slice) {
        Some([x, y]) => x.as_f64().zip(y.as_f64()).filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| Point::new(x, y)),
        _ => None,
    };
    match v.as_array() {
        Some(pairs) if matches!(pairs.len(), 2 | 3) => pairs.iter().map(pair).collect(),
        _ => None,
    }
}

/// The positions of the selected anchor points, when there are two or three. They are counted as
/// the other anchor commands count them ([`super::path::anchor_targets`]): a selected path with
/// anchors picked by Direct Selection gives those, and a path selected whole gives all of its own.
/// A selected segment's ends are not selected anchor points, so a selection with segments gives
/// none; so does a selection holding an object that is not a path, or an anchor that is gone.
fn selected_anchor_points(s: &Session) -> Result<Option<Vec<Point>>> {
    let st = s.doc()?;
    if st.selection.segments.values().any(|set| !set.is_empty()) {
        return Ok(None);
    }
    let mut points = Vec::with_capacity(4);
    for id in &st.selection.objects {
        let Some(pd) = st.doc.node(*id).and_then(|n| n.path_data()) else { return Ok(None) };
        match st.selection.anchors.get(id) {
            Some(set) => {
                for &(si, ai) in set.iter().take(4) {
                    let Some(a) = pd.subpaths.get(si).and_then(|sp| sp.anchors.get(ai)) else { return Ok(None) };
                    points.push(a.p);
                }
            }
            None => points.extend(pd.anchors().take(4).map(|(_, _, a)| a.p)),
        }
        if points.len() > 3 {
            return Ok(None);
        }
    }
    Ok(matches!(points.len(), 2 | 3).then_some(points))
}

/// A live ellipse that touches a path at a point with the path's curvature there
/// ([`curvature::curvature_circle_on`]). One undo step, named as an Ellipse tool drag is.
fn curvature_circle(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shape.curvatureCircle";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let si = index_opt(p, "subpath", C)?.unwrap_or(0);
    let seg = index_opt(p, "segment", C)?.ok_or_else(|| bad(C, "missing segment"))?;
    let t = f64_req(p, "t", C)?;
    if !(0.0..=1.0).contains(&t) {
        return Err(bad(C, "`t` must be from 0 to 1"));
    }
    let convex = switch_or(p, "convex", false, C)?;
    let found = {
        let st = s.doc()?;
        let node = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
        if node.is_guide() {
            return Err(bad(C, "a guide has no curvature circle"));
        }
        let path = node.path_data().ok_or_else(|| bad(C, format!("object {id} is not a path")))?;
        let sp = path.subpaths.get(si).ok_or_else(|| bad(C, "no such subpath"))?;
        if seg >= sp.segment_count() {
            return Err(bad(C, "no such segment"));
        }
        curvature::curvature_circle_on(sp, seg, t, convex)
    };
    let circle = found.map_err(|e| match e {
        NoCircle::Straight => bad(C, "the path is straight there, so no circle matches its curvature"),
        NoCircle::Cusp => bad(C, "the path has a cusp there, so it has no single curvature"),
        NoCircle::Corner => bad(C, "the path has a corner there, so it has no single curvature"),
    })?;
    let (center, radius) = (circle.center, circle.radius);
    let r = Rect::new(center.x - radius, center.y - radius, center.x + radius, center.y + radius);
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.abs() <= MAX_COORD) {
        return Err(bad(C, "the circle that matches the path there would reach past the canvas"));
    }
    let live = LiveShape::Ellipse { w: r.width(), h: r.height(), pie: (0.0, 360.0), xf: Affine::translate(r.origin().to_vec2()) };
    // Not pixel-aligned: the circle stays on the path.
    let mut out = add_art_unaligned(s, "Ellipse", path_kind(live.to_path(), Some(live)))?;
    out["cx"] = json!(center.x);
    out["cy"] = json!(center.y);
    out["radius"] = json!(radius);
    out["nominal"] = json!(circle.nominal);
    Ok(out)
}

/// The whole-number count `p` gives for `key` (`5` or `5.0`), or `default` when it is left out or
/// null. A fraction, a negative, a non-finite or a non-number is an error rather than the default.
fn count_or(p: &Value, key: &str, default: u64, cmd: &str) -> Result<u64> {
    let n = number_or(p, key, default as f64, cmd)?;
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 {
        Ok(n as u64)
    } else {
        Err(bad(cmd, format!("`{key}` must be a whole number of at least 0, not {n}")))
    }
}

fn polygon(s: &mut Session, p: &Value) -> Result<Value> {
    let c = Point::new(f64_req(p, "cx", "shape.polygon")?, f64_req(p, "cy", "shape.polygon")?);
    let r = f64_req(p, "radius", "shape.polygon")?.abs();
    check_numbers(p, &["rotation"], "shape.polygon")?;
    let sides = count_or(p, "sides", 6, "shape.polygon")?.clamp(3, 1000) as u32;
    let rot = f64_or(p, "rotation", 0.0);
    let xf = Affine::translate(c.to_vec2()) * vectorcraft_geom::rotate_deg(rot);
    let live = LiveShape::Polygon { radius: r, sides, xf, radii: vec![], kinds: vec![], radii_relative: false };
    add_art_turned(s, "Polygon", path_kind(live.to_path(), Some(live)), constrained_turn(s, rot))
}

fn star(s: &mut Session, p: &Value) -> Result<Value> {
    let c = Point::new(f64_req(p, "cx", "shape.star")?, f64_req(p, "cy", "shape.star")?);
    let r1 = f64_req(p, "radius1", "shape.star")?.abs();
    check_numbers(p, &["radius2", "rotation"], "shape.star")?;
    let r2 = f64_or(p, "radius2", r1 / 2.0).abs();
    let n = count_or(p, "points", 5, "shape.star")?.clamp(2, 1000) as u32;
    let rot = f64_or(p, "rotation", 0.0);
    let turn = vectorcraft_geom::rotate_deg(rot);
    // A quarter or whole turn (a Constrain Angle of 90°, say) turns the upright star exactly.
    let path = if rot != 0.0 && turn.as_coeffs()[..4].iter().all(|c| c.fract() == 0.0) {
        shapes::star(Point::ZERO, r1, r2, n, 0.0).transformed(Affine::translate(c.to_vec2()) * turn)
    } else {
        shapes::star(c, r1, r2, n, rot)
    };
    add_art_turned(s, "Star", path_kind(path, None), constrained_turn(s, rot))
}

fn line(s: &mut Session, p: &Value) -> Result<Value> {
    let a = Point::new(f64_req(p, "x1", "shape.line")?, f64_req(p, "y1", "shape.line")?);
    let b = Point::new(f64_req(p, "x2", "shape.line")?, f64_req(p, "y2", "shape.line")?);
    let live = LiveShape::Line { a, b };
    // Lines are stroked and, unless Fill Line asks for the current fill, not filled.
    let fill = if switch_or(p, "fill", false, "shape.line")? { s.paint.fill.clone() } else { vectorcraft_color::Paint::None };
    let look = s.new_art_look(
        fill,
        if s.paint.stroke.is_none() { vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK) } else { s.paint.stroke.clone() },
        s.paint.stroke_width.max(0.1),
    );
    add_look(s, "Line", path_kind(live.to_path(), Some(live)), look, None)
}

fn spiral(s: &mut Session, p: &Value) -> Result<Value> {
    let rotation = if p.get("rotation").is_some() { f64_req(p, "rotation", "shape.spiral")? } else { 0.0 };
    if !rotation.is_finite() {
        return Err(bad("shape.spiral", "rotation must be finite"));
    }
    let c = Point::new(f64_req(p, "cx", "shape.spiral")?, f64_req(p, "cy", "shape.spiral")?);
    let segments = count_or(p, "segments", 10, "shape.spiral")?.clamp(2, 1000) as u32;
    let path = shapes::spiral(
        c,
        f64_req(p, "radius", "shape.spiral")?,
        number_or(p, "decay", 80.0, "shape.spiral")?,
        segments,
        switch_or(p, "clockwise", true, "shape.spiral")?,
    );
    // The outer end turns clockwise from (cx + radius, cy) about the center; 0 and whole turns
    // leave the path as it is, and quarter turns are exact.
    let path = if bbox_turn(rotation) == 0.0 {
        path
    } else {
        let c = c.to_vec2();
        path.transformed(Affine::translate(c) * vectorcraft_geom::rotate_deg(rotation) * Affine::translate(-c))
    };
    add_art_turned(s, "Spiral", path_kind(path, None), constrained_turn(s, rotation))
}

fn arc(s: &mut Session, p: &Value) -> Result<Value> {
    let rotation = if p.get("rotation").is_some() { f64_req(p, "rotation", "shape.arc")? } else { 0.0 };
    if !rotation.is_finite() {
        return Err(bad("shape.arc", "rotation must be finite"));
    }
    let closed = switch_or(p, "closed", false, "shape.arc")?;
    let fill_arc = switch_or(p, "fillArc", false, "shape.arc")?;
    // Given values are checked in both forms, null included; only a left-out key takes its default.
    let base_x = match p.get("baseAlong") {
        None => true,
        Some(Value::String(v)) if v.eq_ignore_ascii_case("x") || v.eq_ignore_ascii_case("y") => v.eq_ignore_ascii_case("x"),
        Some(_) => return Err(bad("shape.arc", "`baseAlong` must be one of x, y")),
    };
    let reference = match p.get("reference") {
        None => 0,
        Some(v) => match (v.as_u64(), v.as_f64()) {
            (Some(n), _) if n <= 8 => n as usize,
            (None, Some(x)) if (0.0..=8.0).contains(&x) && x.fract() == 0.0 => x as usize,
            _ => return Err(bad("shape.arc", "`reference` must be a whole number from 0 to 8")),
        },
    };
    let slope = |default: f64| {
        let (key, lo, hi) = ("slope", -100.0, 100.0);
        let v = number_or(p, key, default, "shape.arc")?;
        if (lo..=hi).contains(&v) { Ok(v) } else { Err(bad("shape.arc", format!("`{key}` must be from {lo} to {hi}, not {v}"))) }
    };
    // The upright path, and the point `rotation` turns it about.
    let (path, pivot) = if p.get("lengthX").is_some() || p.get("lengthY").is_some() {
        let anchor = Point::new(f64_req(p, "x", "shape.arc")?, f64_req(p, "y", "shape.arc")?);
        let path = shapes::arc_segment(
            anchor,
            reference,
            number_or(p, "lengthX", 100.0, "shape.arc")?,
            number_or(p, "lengthY", 100.0, "shape.arc")?,
            base_x,
            slope(50.0)?,
            closed,
        );
        (path, anchor)
    } else {
        let a = Point::new(f64_req(p, "x1", "shape.arc")?, f64_req(p, "y1", "shape.arc")?);
        let b = Point::new(f64_req(p, "x2", "shape.arc")?, f64_req(p, "y2", "shape.arc")?);
        (shapes::arc(a, b, slope(100.0)?, closed, base_x), a)
    };
    // 0 and whole turns leave the path as it is; quarter turns are exact.
    let bbox_angle = bbox_turn(rotation);
    let path = if bbox_angle == 0.0 {
        path
    } else {
        let c = pivot.to_vec2();
        path.transformed(Affine::translate(c) * vectorcraft_geom::rotate_deg(rotation) * Affine::translate(-c))
    };
    let fill = if fill_arc { s.paint.fill.clone() } else { vectorcraft_color::Paint::None };
    let look = s.new_art_look(
        fill,
        if s.paint.stroke.is_none() { vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK) } else { s.paint.stroke.clone() },
        s.paint.stroke_width.max(0.1),
    );
    insert_look_turned(s, "Arc", path_kind(path, None), look, None, true, bbox_angle, true)
}

/// An open circular arc from `start` to `end` ([`shapes::CircularArc`]): through `through`, or
/// leaving the end `at` names toward `toward`. One undo step, named as an Arc tool drag is.
fn three_point_arc(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shape.threePointArc";
    let point = |key: &str| finite_numbers::<2>(&p[key]).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad(C, format!("`{key}` must be [x, y]")));
    let (start, end) = (point("start")?, point("end")?);
    let from_end = match p.get("at") {
        None => false,
        Some(Value::String(v)) if v == "start" || v == "end" => v == "end",
        Some(_) => return Err(bad(C, "`at` must be \"start\" or \"end\"")),
    };
    // The arc, and the point it passes through (an anchor of the path, so the path meets it exactly).
    let (arc, via) = match (p.get("through"), p.get("toward")) {
        (Some(_), None) if p.get("at").is_some() => return Err(bad(C, "give `at` only with `toward`")),
        (Some(_), None) => {
            let through = point("through")?;
            (shapes::CircularArc::through(start, end, through), Some(through))
        }
        (None, Some(_)) if from_end => {
            let toward = point("toward")?;
            (shapes::CircularArc::arriving(start, end, end - toward), None)
        }
        (None, Some(_)) => {
            let toward = point("toward")?;
            (shapes::CircularArc::leaving(start, toward - start, end), None)
        }
        _ => return Err(bad(C, "give exactly one of `through` and `toward`")),
    };
    let arc = arc.ok_or_else(|| bad(C, "no arc fits these points: two are the same or all three lie on one line"))?;
    let path = via.map_or_else(|| arc.to_path(), |v| arc.to_path_via(v));
    let look = s.new_art_look(
        vectorcraft_color::Paint::None,
        if s.paint.stroke.is_none() { vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK) } else { s.paint.stroke.clone() },
        s.paint.stroke_width.max(0.1),
    );
    // Not pixel-aligned: the ends stay on the given points.
    let mut out = insert_look_turned(s, "Arc", path_kind(path, None), look, None, true, 0.0, false)?;
    out["cx"] = json!(arc.center.x);
    out["cy"] = json!(arc.center.y);
    out["radius"] = json!(arc.radius);
    out["sweep"] = json!(arc.sweep.to_degrees());
    Ok(out)
}

/// The grid `paths` (bottom first) as one selected group in the new-art stroke, filled with the
/// current fill when `fill`; a path of several subpaths becomes a compound path of them. The group
/// and its members get the bounding-box angle `bbox_angle` (a grid drawn along a Constrain Angle).
fn grid_group(s: &mut Session, label: &str, (paths, bbox_angle): (Vec<PathData>, f64), fill: bool) -> Result<Value> {
    let parent = s.doc()?.target_parent()?;
    let fill = if fill { s.paint.fill.clone() } else { vectorcraft_color::Paint::None };
    let look = s.new_art_look(fill, s.paint.stroke.clone(), s.paint.stroke_width);
    let id = s.edit(label, |d, sel| {
        let children = paths
            .into_iter()
            .map(|pd| {
                let mut n = if pd.subpaths.len() > 1 {
                    let members =
                        pd.subpaths.into_iter().map(|sp| std::sync::Arc::new(Node::path(d.alloc_id(), PathData::single(sp), Appearance::default())));
                    let children = members.collect();
                    Node::new(d.alloc_id(), NodeKind::Compound { children, rule: FillRule::NonZero })
                } else {
                    Node::path(d.alloc_id(), pd, Appearance::default())
                };
                look.apply(d, &mut n);
                super::object::align_new_object(d, &mut n);
                n.bbox_angle = bbox_angle;
                std::sync::Arc::new(n)
            })
            .collect();
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.bbox_angle = bbox_angle;
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// A grid's divider count `count` (5 when absent or null; a whole number of 0 or more, so 4.0 gives
/// 4, and anything else, such as 2.5, -1 or `"3"`, is an error; at most 999) with its skew `key` in
/// percent (0 when absent, clamped to ±[`shapes::GRID_SKEW_MAX`]).
fn dividers(p: &Value, count: &str, key: &str, cmd: &str) -> Result<(u32, f64)> {
    let n = count_or(p, count, 5, cmd)?.min(999) as u32;
    let skew = match p.get(key) {
        None | Some(Value::Null) => 0.0,
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?,
    };
    Ok((n, skew.clamp(-shapes::GRID_SKEW_MAX, shapes::GRID_SKEW_MAX)))
}

/// A grid's on/off option `key` (`default` when absent).
fn switch(p: &Value, key: &str, default: bool, cmd: &str) -> Result<bool> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, format!("`{key}` must be true or false"))),
    }
}

fn rect_grid(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.rectangularGrid";
    let r = rect_of(p, CMD)?;
    let paths =
        shapes::rectangular_grid(r, dividers(p, "rows", "rowSkew", CMD)?, dividers(p, "columns", "columnSkew", CMD)?, switch(p, "frame", true, CMD)?);
    grid_group(s, "Rectangular Grid", turned_grid(p, r, paths, CMD)?, switch(p, "fill", false, CMD)?)
}

fn polar_grid(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.polarGrid";
    let r = rect_of(p, CMD)?;
    let paths = shapes::polar_grid(
        r,
        dividers(p, "concentric", "concentricSkew", CMD)?,
        dividers(p, "radial", "radialSkew", CMD)?,
        switch(p, "compound", false, CMD)?,
    );
    grid_group(s, "Polar Grid", turned_grid(p, r, paths, CMD)?, switch(p, "fill", false, CMD)?)
}

/// A grid's paths in `r` turned by the `rotation` param about the centre of `r` (a grid drawn
/// along a Constrain Angle), and their bounding-box angle; as they are without it or at 0.
fn turned_grid(p: &Value, r: Rect, mut paths: Vec<PathData>, cmd: &str) -> Result<(Vec<PathData>, f64)> {
    let rotation = if p.get("rotation").is_some() { f64_req(p, "rotation", cmd)? } else { 0.0 };
    if !rotation.is_finite() {
        return Err(bad(cmd, "rotation must be finite"));
    }
    if rotation != 0.0 {
        let c = r.center().to_vec2();
        let m = Affine::translate(c) * vectorcraft_geom::rotate_deg(rotation) * Affine::translate(-c);
        for path in &mut paths {
            path.transform(m);
        }
    }
    Ok((paths, bbox_turn(rotation)))
}

/// An anchor `{x, y, in?, out?, smooth?}` given to `cmd` (handles `[x, y]`, at the anchor when
/// absent or null): anything malformed fails rather than being left out or replaced by a default,
/// which would change the path's shape.
pub(crate) fn anchor_from_json(v: &Value, cmd: &str) -> Result<Anchor> {
    let coord =
        |key: &str| v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).ok_or_else(|| bad(cmd, format!("anchor `{key}` must be a number")));
    let p = Point::new(coord("x")?, coord("y")?);
    let handle = |key: &str| match v.get(key).filter(|h| !h.is_null()) {
        None => Ok(p),
        Some(h) => finite_numbers(h).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad(cmd, format!("anchor `{key}` must be [x, y]"))),
    };
    let mut anchor = Anchor::with_handles(p, handle("in")?, handle("out")?);
    match v.get("smooth").filter(|b| !b.is_null()) {
        None => {}
        Some(Value::Bool(false)) => anchor.kind = AnchorKind::Corner,
        Some(Value::Bool(true)) => anchor.kind = AnchorKind::Smooth,
        Some(_) => return Err(bad(cmd, "anchor `smooth` must be true or false")),
    }
    Ok(anchor)
}

fn path_create(s: &mut Session, p: &Value) -> Result<Value> {
    // Checked with `d` too, where the path data alone decides whether the path is closed.
    let closed = switch_or(p, "closed", false, "path.create")?;
    let path = if let Some(d) = str_param(p, "d") {
        let bp = vectorcraft_geom::BezPath::from_svg(d).map_err(|e| bad("path.create", format!("bad path data: {e}")))?;
        PathData::from_bezpath(&bp)
    } else {
        let anchors: Vec<Anchor> = p
            .get("anchors")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("path.create", "missing anchors"))?
            .iter()
            .map(|a| anchor_from_json(a, "path.create"))
            .collect::<Result<_>>()?;
        if anchors.is_empty() {
            return Err(bad("path.create", "need at least one anchor"));
        }
        PathData::single(SubPath::new(anchors, closed))
    };
    let mut look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    // Open paths drawn with a stroke of None would be invisible: mimic Illustrator and keep what the user set.
    let ap = &mut look.appearance;
    if !path.is_closed() && ap.stroke_paint().is_none() && ap.fill_paint().is_none() {
        ap.set_stroke(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK));
    }
    add_look(s, "Pen", path_kind(path, None), look, None)
}

fn text_create(s: &mut Session, p: &Value) -> Result<Value> {
    let x = f64_req(p, "x", "text.create")?;
    let y = f64_req(p, "y", "text.create")?;
    super::paint::check_keep_model(p, "text.create")?;
    check_numbers(p, &["size"], "text.create")?;
    check_switches(p, &["vertical", "placeholder"], "text.create")?;
    super::color_param(p, "color", "text.create")?;
    if let Some(a) = p.get("area") {
        check_numbers(a, &["width", "height"], "text.create")?;
    }
    let text = str_param(p, "text").unwrap_or("");
    let mut t = TextObject::point(Point::new(x, y), text, new_type_style(s, p, "text.create")?);
    // Turned about the point (the first baseline's start, an area's top-left corner): type set
    // along a Constrain Angle.
    let rotation = if p.get("rotation").is_some() { f64_req(p, "rotation", "text.create")? } else { 0.0 };
    if !rotation.is_finite() {
        return Err(bad("text.create", "rotation must be finite"));
    }
    if rotation != 0.0 {
        t.xf *= vectorcraft_geom::rotate_deg(rotation);
    }
    t.vertical = switch_or(p, "vertical", false, "text.create")?;
    t.para = new_type_para();
    new_type_alignment(s, p, "text.create", &mut t)?;
    if let Some(a) = p.get("area") {
        let w = f64_or(a, "width", 200.0);
        let h = f64_or(a, "height", 100.0);
        t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, h)) };
        new_area_fit(s, a, "text.create", &mut t)?;
    }
    t.system_layout = s.doc()?.doc.setup.system_layout;
    if bool_or(p, "placeholder", false) {
        super::typemenu::fill_with_placeholder(&mut t);
    } else {
        // Lays the text out (and sizes an Auto Size frame).
        super::typecmd::refresh_bounds(&mut t);
    }
    let made =
        insert_look_turned(s, "Type", NodeKind::Text(Box::new(t)), NewArt::plain(Appearance::default()), None, true, bbox_turn(rotation), true)?;
    let id = made.get("id").and_then(Value::as_u64).map(NodeId);
    let made = super::typecmd::with_bounds(s, id, made);
    let mut made = super::typecmd::with_overflow(s, id.as_slice(), made);
    // A face from a redefined [Normal Character Style] is checked too, as one given here is.
    let normal_face = s
        .doc()
        .ok()
        .and_then(|d| super::textstyles::style_attrs(&d.doc, false, super::textstyles::NORMAL_CHAR))
        .is_some_and(|m| m.contains_key("font_family") || m.contains_key("font_style"));
    if p.get("font").is_some() || p.get("style").is_some() || normal_face {
        let runs = match id.and_then(|i| s.doc().ok()?.doc.node(i)).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => t.runs.iter().flat_map(super::fonts::run_fonts).collect(),
            _ => Default::default(),
        };
        if let Some(w) = super::fonts::set_font_warning(runs) {
            made["warning"] = json!(w);
        }
    }
    Ok(made)
}

/// The paragraph attributes new type gets (the Type tools, `text.create`): aligned to the start of
/// each paragraph's direction (left, right for Hebrew and Arabic), Japanese punctuation composed
/// with Line-end Punctuation Half Width and Standard burasagari, and Korean words kept whole
/// (Korean Line Break Anywhere off). Imported text keeps its own.
pub(crate) fn new_type_para() -> vectorcraft_doc::ParaStyle {
    vectorcraft_doc::ParaStyle {
        justify: vectorcraft_doc::Justify::Auto,
        mojikumi: vectorcraft_doc::Mojikumi::LineEndHalf,
        burasagari: vectorcraft_doc::Burasagari::Standard,
        korean_line_break_anywhere: false,
        ..Default::default()
    }
}

/// The paragraph alignment, leading model and character alignment new type starts with (`t`'s
/// paragraph and runs): `justify` from `p` (else as [`new_type_para`] left it), `leadingModel`
/// and `charAlign` from `p` (`charAlign` else from a redefined `[Normal Character Style]` that
/// sets it), else em box top to top and em box centre while the interface is in Japanese (#432),
/// else the Roman baseline for both. Noted in the journal, so a
/// replay sets the same whatever the interface.
pub(crate) fn new_type_alignment(s: &mut Session, p: &Value, cmd: &str, t: &mut TextObject) -> Result<()> {
    use vectorcraft_doc::{CharAlign, LeadingModel};
    if let Some(j) = str_param(p, "justify") {
        t.para.justify = super::typecmd::justify_param(j).ok_or_else(|| bad(cmd, "justify must be auto|left|center|right|justifyAll"))?;
    }
    let japanese = s.japanese_interface();
    let leading = match p.get("leadingModel").filter(|v| !v.is_null()) {
        Some(v) => match v.as_str() {
            Some("romanBaseline") => LeadingModel::RomanBaseline,
            Some("emBoxTop") => LeadingModel::EmBoxTop,
            _ => return Err(bad(cmd, "`leadingModel` must be \"romanBaseline\" or \"emBoxTop\"")),
        },
        None => {
            let l = if japanese { LeadingModel::EmBoxTop } else { LeadingModel::RomanBaseline };
            s.note_journal("leadingModel", json!(l));
            l
        }
    };
    // A redefined [Normal Character Style] that sets Character Alignment gives it to new type.
    let normal_align = s
        .doc()
        .ok()
        .and_then(|d| super::textstyles::style_attrs(&d.doc, false, super::textstyles::NORMAL_CHAR))
        .filter(|m| m.contains_key("charAlign"))
        .and(t.runs.first().map(|r| r.style.char_align));
    let align = match super::textedit::char_align_param(p, cmd)?.or(normal_align) {
        Some(a) => a,
        None => {
            let a = if japanese { CharAlign::EmBoxCenter } else { CharAlign::RomanBaseline };
            s.note_journal("charAlign", json!(a));
            a
        }
    };
    t.para.leading_model = leading;
    for r in &mut t.runs {
        r.style.char_align = align;
    }
    Ok(())
}

/// The fit new area type `t` gets: `fit`/`fitMinPercent` from `p` (as `text.areaOptions` takes
/// them, `block` turning Dynamic Text on), else Auto Size when the "Auto Size New Area Type"
/// preference is on; and Block's `wordGap` and `lineGap`.
pub(crate) fn new_area_fit(s: &Session, p: &Value, c: &str, t: &mut TextObject) -> Result<()> {
    let default = if s.prefs.auto_size_area_type { vectorcraft_doc::AreaFit::AutoHeight } else { vectorcraft_doc::AreaFit::None };
    let (word_gap, line_gap) = super::typecmd::block_gaps(p, c)?;
    (t.area.fit, t.dynamic_text) = super::typecmd::fit_param(p, default, false, c)?.unwrap_or((default, false));
    t.area.word_gap = word_gap.unwrap_or(t.area.word_gap);
    t.area.line_gap = line_gap.unwrap_or(t.area.line_gap);
    Ok(())
}

/// The character style new type gets: `size`, `font`, `style` and `color` from `p` (an RGB
/// `color` in a CMYK document as [`super::paint::new_color`] stores it), else
/// `[Normal Character Style]`'s (the default font, size and black fill unless redefined;
/// never the current fill, #7975), in the style's language, else the document's. A `size` outside
/// [`super::typecmd::TYPE_SIZES`] is an error.
pub(crate) fn new_type_style(s: &Session, p: &Value, cmd: &str) -> Result<CharStyle> {
    let mut style = s.doc().map_or_else(|_| CharStyle::default(), |d| super::textstyles::normal_char_style(&d.doc));
    if let Some(sz) = p.get("size").and_then(Value::as_f64) {
        style.size = super::typecmd::given_type_size(sz, cmd)?;
    }
    if str_param(p, "font").is_some() || str_param(p, "style").is_some() {
        // A face given to the command takes its own version and axis settings, as a font change
        // does, not those [Normal Character Style] gave another face.
        style.font_version = None;
        style.font_axes.clear();
    }
    if let Some(f) = str_param(p, "font") {
        style.font_family = f.to_string();
        // A family alone starts in Regular, whatever style [Normal Character Style] has.
        style.font_style = CharStyle::default().font_style;
    }
    if let Some(f) = str_param(p, "style") {
        style.font_style = f.to_string();
    } else if str_param(p, "font").is_some() {
        // A family without a Regular (#4128): its closest style, as text.setRangeStyle takes.
        super::textedit::fit_font_style(&mut style);
    }
    style.fill = match p.get("color").and_then(color_value) {
        Some(c) => vectorcraft_color::Paint::solid(super::paint::new_color(s, p, c)),
        None => style.fill,
    };
    // In the language a redefined [Normal Character Style] gives, else the one Document Setup chose,
    // else Preferences › Hyphenation › Default Language; none for the default. Type already in a
    // document keeps its own.
    if style.language.is_none() {
        let chosen = |name: &str| vectorcraft_doc::setup::char_language(name).ok().flatten();
        let document = s.doc().ok().and_then(|d| chosen(&d.doc.setup.language));
        style.language = document.or_else(|| chosen(&s.prefs.hyphenation_language)).map(str::to_string);
    }
    Ok(style)
}
