//! Creating art: shapes, paths, text.

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, CharStyle, LiveShape, Node, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Anchor, AnchorKind, FillRule, PathData, Point, Rect, SubPath, shapes};

use super::newart::NewArt;
use super::{
    CommandSpec, NodeId, Result, Session, bad, bool_or, check_numbers, check_switches, cmd, color_value, f64_or, f64_req, finite_numbers, has_doc,
    number_or, str_param, switch_or,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "shape.rectangle",
            "Rectangle",
            [],
            None,
            "{x, y, width, height, radius?: pt, rotation?: deg (about centre)} → {id}",
            has_doc,
            rectangle
        ),
        cmd!("shape.ellipse", "Ellipse", [], None, "{x, y, width, height, rotation?: deg (about centre)} → {id}", has_doc, ellipse),
        cmd!("shape.polygon", "Polygon", [], None, "{cx, cy, radius, sides=6, rotation?: deg} → {id}", has_doc, polygon),
        cmd!("shape.star", "Star", [], None, "{cx, cy, radius1, radius2, points=5, rotation?: deg} → {id}", has_doc, star),
        cmd!(
            "shape.flare",
            "Flare",
            [],
            None,
            "{cx, cy, diameter=100, opacity=50 (%), brightness=30 (%), growth=20 (%), fuzziness=50 (%), rays=15, longest=300 (%), rayFuzziness=100 (%), rotation=0 (deg, turns the rays clockwise), x2?, y2? (ring end; else pathLength at direction), pathLength=300, rings=10, largest=50 (%), direction=45 (deg), seed?} → {id}",
            has_doc,
            flare
        ),
        cmd!("shape.line", "Line Segment", [], None, "{x1, y1, x2, y2, fill?: bool (Fill Line: the current fill, else none)} → {id}", has_doc, line),
        cmd!("shape.spiral", "Spiral", [], None, "{cx, cy, radius, decay=80 (%), segments=10, clockwise?} → {id}", has_doc, spiral),
        cmd!(
            "shape.arc",
            "Arc",
            [],
            None,
            "{x1, y1, x2, y2, closed?, slope?} or {x, y, lengthX, lengthY, reference?, baseAlong?: x|y, slope?, closed?, fillArc?} → {id}",
            has_doc,
            arc
        ),
        cmd!(
            "shape.rectangularGrid",
            "Rectangular Grid",
            [],
            None,
            "{x, y, width, height, rows=5, columns=5, rowSkew=0, columnSkew=0 (−500…500 %: + crowds the horizontal dividers toward the top, the vertical ones toward the right), frame=true (the outside as one rectangle, else its four sides), fill=false (the current fill, else none), rotation?: deg (about centre)} → {id}",
            has_doc,
            rect_grid
        ),
        cmd!(
            "shape.polarGrid",
            "Polar Grid",
            [],
            None,
            "{x, y, width, height, concentric=5, radial=5, concentricSkew=0, radialSkew=0 (−500…500 %: + crowds the ellipses toward the outside, the radial dividers clockwise of the top one), compound=false (the ellipses pair up into compound-path rings), fill=false (the current fill, else none), rotation?: deg (about centre)} → {id}",
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
            "{x, y, text, rotation?: deg (about x, y), vertical?: bool = false, size?: pt, font?: family, style? (default Regular, or the family's closest style when it has no Regular), color?, keepModel?: bool (as paint.setFill), area?: {width, height, fit?: none|autoHeight|shrinkText, fitMinPercent?}, placeholder?: bool (fill it with placeholder text instead, as the Type tools do with prefs placeholderText), justify?: \"auto\" (default: the start of each paragraph's direction)|\"left\"|\"center\"|\"right\"|\"justifyAll\" (point type: aligned on x, so \"center\" centres each line on the point and \"right\" ends it there; text.setStyle changes it later), leadingModel?: \"romanBaseline\"|\"emBoxTop\", charAlign?: \"romanBaseline\"|\"emBoxTop\"|\"emBoxCenter\"|\"emBoxBottom\"|\"icfTop\"|\"icfBottom\" (default: emBoxTop and emBoxCenter while the interface is in Japanese, else romanBaseline)} → {id, bounds: {x, y, width, height} (the new type's box as document.inspect reports it, after justify), overflow?: [id] (area type whose text doesn't fit its frame: the lines past it aren't drawn; left out when it fits)}; new area type gets Auto Size (fit autoHeight) when the autoSizeAreaType preference is on",
            has_doc,
            text_create
        ),
    ]
}

/// Insert a new object at the top of the insertion parent with the current paint and new-art
/// template; select it.
pub(crate) fn add_art(s: &mut Session, label: &str, kind: NodeKind, name: Option<String>) -> Result<Value> {
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    add_look(s, label, kind, look, name)
}

/// Insert a new object looking like `look` (drawing modes apply); select it.
pub(crate) fn add_look(s: &mut Session, label: &str, kind: NodeKind, look: NewArt, name: Option<String>) -> Result<Value> {
    insert_look(s, label, kind, look, name, true)
}

/// [`add_look`] that selects the new object only when `select` is set; otherwise the selection
/// stays as it was.
pub(crate) fn insert_look(s: &mut Session, label: &str, kind: NodeKind, look: NewArt, name: Option<String>, select: bool) -> Result<Value> {
    insert_look_turned(s, label, kind, look, name, select, 0.0)
}

/// [`add_art`] for an object made turned by a `rotation` param (degrees, clockwise on the page):
/// its bounding box turns with it, as after the Rotate tool.
fn add_art_turned(s: &mut Session, label: &str, kind: NodeKind, rotation: f64) -> Result<Value> {
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    insert_look_turned(s, label, kind, look, None, true, bbox_turn(rotation))
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

/// [`insert_look`] with the new object's bounding box turned to `bbox_angle`.
fn insert_look_turned(
    s: &mut Session,
    label: &str,
    kind: NodeKind,
    look: NewArt,
    name: Option<String>,
    select: bool,
    bbox_angle: f64,
) -> Result<Value> {
    let parent = s.doc()?.target_parent()?;
    let mode = s.draw_mode;
    let inside = s.draw_inside;
    let behind_of = s.doc()?.selection.in_paint_order(&s.doc()?.doc).first().copied();
    let (id, into) = s.edit(label, |d, sel| {
        let id = d.alloc_id();
        let mut n = Node::new(id, kind);
        look.apply(d, &mut n);
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
/// put; a polygon or star turns its base orientation about its centre. The params as they are at
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
        "shape.polygon" | "shape.star" => {
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

fn ellipse(s: &mut Session, p: &Value) -> Result<Value> {
    let r = rect_of(p, "shape.ellipse")?;
    let live = LiveShape::Ellipse { w: r.width(), h: r.height(), pie: (0.0, 360.0), xf: rect_transform(r, p, "shape.ellipse")? };
    add_art_turned(s, "Ellipse", path_kind(live.to_path(), Some(live)), f64_or(p, "rotation", 0.0))
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

/// Flare tool: a lens flare built from radial-gradient paths in Screen mode — a bright centre, a halo
/// ring, rays of varying length and rings along the path to the end point.
fn flare(s: &mut Session, p: &Value) -> Result<Value> {
    use vectorcraft_color::{BlendMode, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
    let c = Point::new(f64_req(p, "cx", "shape.flare")?, f64_req(p, "cy", "shape.flare")?);
    let numbers = [
        "diameter",
        "opacity",
        "brightness",
        "growth",
        "fuzziness",
        "rays",
        "longest",
        "rayFuzziness",
        "rotation",
        "rings",
        "largest",
        "x2",
        "y2",
        "direction",
        "pathLength",
        "seed",
    ];
    check_numbers(p, &numbers, "shape.flare")?;
    let pct = |k: &str, d: f64| (f64_or(p, k, d) / 100.0).clamp(0.0, 10.0);
    let r = (f64_or(p, "diameter", 100.0) / 2.0).max(0.5);
    let opacity = pct("opacity", 50.0).min(1.0) as f32;
    let brightness = pct("brightness", 30.0).min(1.0) as f32;
    let growth = pct("growth", 20.0);
    let fuzz = pct("fuzziness", 50.0).min(1.0);
    let rays = p.get("rays").and_then(Value::as_u64).unwrap_or(15).min(50) as usize;
    let longest = pct("longest", 300.0);
    let ray_fuzz = pct("rayFuzziness", 100.0).min(1.0);
    let ray_turn = f64_or(p, "rotation", 0.0).to_radians();
    let rings = p.get("rings").and_then(Value::as_u64).unwrap_or(10).min(50) as usize;
    let largest = pct("largest", 50.0);
    let end = match (p.get("x2").and_then(Value::as_f64), p.get("y2").and_then(Value::as_f64)) {
        (Some(x), Some(y)) => Point::new(x, y),
        _ => {
            let a = f64_or(p, "direction", 45.0).to_radians();
            c + vectorcraft_geom::Vec2::new(a.cos(), -a.sin()) * f64_or(p, "pathLength", 300.0)
        }
    };
    // Deterministic pseudo-random sizes (same parameters → same flare).
    let mut seed = p.get("seed").and_then(Value::as_u64).unwrap_or(0x2545_f491_4f6c_dd1d) | 1;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let stop = |offset: f32, opacity: f32| GradientStop { opacity, ..GradientStop::new(offset, vectorcraft_color::Color::WHITE) };
    let radial = |centre: Point, radius: f64, stops: Vec<GradientStop>| {
        let mut g = GradientPaint::new(Gradient::new(GradientKind::Radial, stops));
        g.geom = Some(GradientGeom { start: centre, end: centre + vectorcraft_geom::Vec2::new(radius, 0.0), aspect: 1.0, focal: None });
        Paint::Gradient(Box::new(g))
    };
    let circle = |centre: Point, radius: f64| shapes::ellipse(Rect::from_center_size(centre, (radius * 2.0, radius * 2.0)));
    let mut parts: Vec<(&str, PathData, Paint, f32)> = vec![];
    // Halo: a soft ring just outside the centre.
    let halo_r = r * (1.0 + growth) * 1.6;
    let inner = (1.0 - fuzz as f32 * 0.6).clamp(0.05, 0.95);
    parts.push(("Halo", circle(c, halo_r), radial(c, halo_r, vec![stop(0.0, 0.0), stop(inner * 0.75, 0.0), stop(inner, 0.5), stop(1.0, 0.0)]), 1.0));
    // Rays: thin spikes of random length (Longest = % of the centre radius), fading outwards.
    // `rotation` turns them clockwise about the center.
    if rays > 0 {
        let mut bp = vectorcraft_geom::BezPath::new();
        let mut reach: f64 = 0.0;
        for i in 0..rays {
            let a = std::f64::consts::TAU * (i as f64 + rnd() * 0.6) / rays as f64 + ray_turn;
            let len = r * longest * (1.0 - ray_fuzz * 0.75 * rnd()).max(0.1);
            reach = reach.max(len);
            let w = r * 0.035;
            let dir = vectorcraft_geom::Vec2::new(a.cos(), a.sin());
            let n = vectorcraft_geom::Vec2::new(-dir.y, dir.x) * w;
            bp.move_to(c + n);
            bp.line_to(c + dir * len);
            bp.line_to(c - n);
            bp.line_to(c - dir * (w * 4.0));
            bp.close_path();
        }
        parts.push(("Rays", PathData::from_bezpath(&bp), radial(c, reach, vec![stop(0.0, 0.9), stop(0.25, 0.35), stop(1.0, 0.0)]), 1.0));
    }
    // Centre: bright core fading to transparent.
    parts.push(("Center", circle(c, r), radial(c, r, vec![stop(0.0, 1.0), stop(0.25 + brightness * 0.5, 0.6), stop(1.0, 0.0)]), opacity));
    // Rings along the path to the end point (Largest = % of the average ring size).
    for _ in 0..rings {
        let t = rnd();
        let at = c + (end - c) * t;
        let rr = (r * largest * (0.25 + rnd() * 0.75)).max(0.5);
        let solid = rnd() < 0.5;
        let stops = if solid {
            vec![stop(0.0, 0.25), stop(0.8, 0.2), stop(1.0, 0.0)]
        } else {
            vec![stop(0.0, 0.0), stop(0.7, 0.05), stop(0.92, 0.35), stop(1.0, 0.0)]
        };
        parts.push(("Ring", circle(at, rr), radial(at, rr, stops), 1.0));
    }
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Flare", |d, sel| {
        let children = parts
            .into_iter()
            .map(|(name, pd, paint, op)| {
                let mut n = Node::path(d.alloc_id(), pd, Appearance::basic(paint, Paint::None, 0.0));
                n.name = Some(name.into());
                n.blend = BlendMode::Screen;
                n.opacity = op;
                std::sync::Arc::new(n)
            })
            .collect();
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = Some("Flare".into());
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0 }))
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
    let c = Point::new(f64_req(p, "cx", "shape.spiral")?, f64_req(p, "cy", "shape.spiral")?);
    let segments = count_or(p, "segments", 10, "shape.spiral")?.clamp(2, 1000) as u32;
    let path = shapes::spiral(
        c,
        f64_req(p, "radius", "shape.spiral")?,
        number_or(p, "decay", 80.0, "shape.spiral")?,
        segments,
        switch_or(p, "clockwise", true, "shape.spiral")?,
    );
    add_art(s, "Spiral", path_kind(path, None), None)
}

fn arc(s: &mut Session, p: &Value) -> Result<Value> {
    let closed = switch_or(p, "closed", false, "shape.arc")?;
    let fill_arc = switch_or(p, "fillArc", false, "shape.arc")?;
    let base_x = !matches!(p.get("baseAlong").and_then(Value::as_str), Some("y" | "Y"));
    let path = if p.get("lengthX").is_some() || p.get("lengthY").is_some() {
        let anchor = Point::new(f64_req(p, "x", "shape.arc")?, f64_req(p, "y", "shape.arc")?);
        let reference = p.get("reference").and_then(Value::as_u64).unwrap_or(0) as usize;
        shapes::arc_segment(
            anchor,
            reference,
            number_or(p, "lengthX", 100.0, "shape.arc")?,
            number_or(p, "lengthY", 100.0, "shape.arc")?,
            base_x,
            number_or(p, "slope", 50.0, "shape.arc")?,
            closed,
        )
    } else {
        let a = Point::new(f64_req(p, "x1", "shape.arc")?, f64_req(p, "y1", "shape.arc")?);
        let b = Point::new(f64_req(p, "x2", "shape.arc")?, f64_req(p, "y2", "shape.arc")?);
        let slope = number_or(p, "slope", 100.0, "shape.arc")?;
        shapes::arc(a, b, slope, closed, base_x)
    };
    let fill = if fill_arc { s.paint.fill.clone() } else { vectorcraft_color::Paint::None };
    let look = s.new_art_look(
        fill,
        if s.paint.stroke.is_none() { vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK) } else { s.paint.stroke.clone() },
        s.paint.stroke_width.max(0.1),
    );
    add_look(s, "Arc", path_kind(path, None), look, None)
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

/// A grid's divider count `count` (an error unless it is a number or null; 5 unless it is a JSON
/// whole number of 0 or more, so 4.0 gives 4 and 2.5 gives 5; at most 999) with its skew `key` in percent (0 when
/// absent, clamped to ±[`shapes::GRID_SKEW_MAX`]).
fn dividers(p: &Value, count: &str, key: &str, cmd: &str) -> Result<(u32, f64)> {
    check_numbers(p, &[count], cmd)?;
    let whole = |v: &Value| v.as_u64().or_else(|| v.as_f64().filter(|f| f.is_finite() && *f >= 0.0 && f.fract() == 0.0).map(|f| f as u64));
    let n = p.get(count).and_then(whole).unwrap_or(5).min(999) as u32;
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
        None | Some(Value::Bool(false)) => {}
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
    if let Some(a) = p.get("area") {
        check_numbers(a, &["width", "height"], "text.create")?;
    }
    let text = str_param(p, "text").unwrap_or("");
    let mut t = TextObject::point(Point::new(x, y), text, new_type_style(s, p));
    // Turned about the point (the first baseline's start, an area's top-left corner): type set
    // along a Constrain Angle.
    let rotation = if p.get("rotation").is_some() { f64_req(p, "rotation", "text.create")? } else { 0.0 };
    if !rotation.is_finite() {
        return Err(bad("text.create", "rotation must be finite"));
    }
    if rotation != 0.0 {
        t.xf *= vectorcraft_geom::rotate_deg(rotation);
    }
    t.vertical = p.get("vertical").and_then(Value::as_bool).unwrap_or(false);
    t.para = new_type_para();
    new_type_alignment(s, p, "text.create", &mut t)?;
    if let Some(a) = p.get("area") {
        let w = f64_or(a, "width", 200.0);
        let h = f64_or(a, "height", 100.0);
        t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, h)) };
        t.area.fit = new_area_fit(s, a, "text.create")?;
    }
    t.system_layout = s.doc()?.doc.setup.system_layout;
    if bool_or(p, "placeholder", false) {
        super::typemenu::fill_with_placeholder(&mut t);
    } else {
        // Lays the text out (and sizes an Auto Size frame).
        super::typecmd::refresh_bounds(&mut t);
    }
    let made = insert_look_turned(s, "Type", NodeKind::Text(Box::new(t)), NewArt::plain(Appearance::default()), None, true, bbox_turn(rotation))?;
    let id = made.get("id").and_then(Value::as_u64).map(NodeId);
    let made = super::typecmd::with_bounds(s, id, made);
    Ok(super::typecmd::with_overflow(s, id.as_slice(), made))
}

/// The paragraph attributes new type gets (the Type tools, `text.create`): aligned to the start of
/// each paragraph's direction (left, right for Hebrew and Arabic), Japanese punctuation composed
/// with Line-end Punctuation Half Width and Standard burasagari. Imported text keeps its own.
pub(crate) fn new_type_para() -> vectorcraft_doc::ParaStyle {
    vectorcraft_doc::ParaStyle {
        justify: vectorcraft_doc::Justify::Auto,
        mojikumi: vectorcraft_doc::Mojikumi::LineEndHalf,
        burasagari: vectorcraft_doc::Burasagari::Standard,
        ..Default::default()
    }
}

/// The paragraph alignment, leading model and character alignment new type starts with (`t`'s
/// paragraph and runs): `justify` from `p` (else as [`new_type_para`] left it), `leadingModel`
/// and `charAlign` from `p`, else em box top to top and em box centre while the
/// interface is in Japanese (#432), else the Roman baseline for both. Noted in the journal, so a
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
    let align = match super::textedit::char_align_param(p, cmd)? {
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

/// The fit new area type gets: `fit`/`fitMinPercent` from `p` (as `text.areaOptions` takes
/// them), else Auto Size when the "Auto Size New Area Type" preference is on.
pub(crate) fn new_area_fit(s: &Session, p: &Value, c: &str) -> Result<vectorcraft_doc::AreaFit> {
    let default = if s.prefs.auto_size_area_type { vectorcraft_doc::AreaFit::AutoHeight } else { vectorcraft_doc::AreaFit::None };
    Ok(super::typecmd::fit_param(p, default, c)?.unwrap_or(default))
}

/// The character style new type gets: `size`, `font`, `style` and `color` from `p` (an RGB
/// `color` in a CMYK document as [`super::paint::new_color`] stores it), else the defaults,
/// filled with the current fill unless that is None or white (black then).
pub(crate) fn new_type_style(s: &Session, p: &Value) -> CharStyle {
    let mut style = CharStyle::default();
    if let Some(sz) = p.get("size").and_then(Value::as_f64) {
        style.size = sz.clamp(0.1, 1296.0);
    }
    if let Some(f) = str_param(p, "font") {
        style.font_family = f.to_string();
    }
    if let Some(f) = str_param(p, "style") {
        style.font_style = f.to_string();
    } else if str_param(p, "font").is_some() {
        // A family without a Regular (#4128): its closest style, as text.setRangeStyle takes.
        super::textedit::fit_font_style(&mut style);
    }
    style.fill = match p.get("color").and_then(color_value) {
        Some(c) => vectorcraft_color::Paint::solid(super::paint::new_color(s, p, c)),
        None if !s.paint.fill.is_none() && s.paint.fill != vectorcraft_color::Paint::solid(vectorcraft_color::Color::WHITE) => s.paint.fill.clone(),
        None => vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
    };
    style
}
