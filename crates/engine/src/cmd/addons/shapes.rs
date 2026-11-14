//! 3D Box and customizable shapes (`addon.shapes.*`), a built-in add-on: creates rotatable 3D boxes
//! (`addon.shapes.box3d`) and customizable rectangles, ellipses, polygons, stars and spirals
//! (`addon.shapes.rectangle`, `addon.shapes.ellipse`, `addon.shapes.polygon`, `addon.shapes.star`,
//! `addon.shapes.spiral`), and configures extended live-shape parameters on selected shapes
//! (`addon.shapes.configure`: symmetric or per-corner radii, pie/chord/arc/whole modes, polygon/star
//! spoke ratio, corner rounding, randomization, spiral turns/divergence/inner radius, and 3D box
//! rotation angles).

#[cfg(test)]
mod tests;

use std::ops::RangeInclusive;

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, Point, Rect, shapes};
use vectorcraft_tools::addons::box3d::CustomShape;

use super::super::create::insert_look;
use super::super::{
    CommandSpec, Result, Session, bad, bool_or, check_numbers, check_switches, cmd, f64_or, f64_req, has_doc, has_selection, number_or, str_param,
    targets,
};

/// The sides of a polygon and the points of a star: whole numbers in this range.
const COUNTS: RangeInclusive<f64> = 3.0..=1000.0;
/// `rounded` and `randomized` of a polygon or star.
const ROUNDING: RangeInclusive<f64> = -10.0..=10.0;
/// A star's inner radius as a fraction of its outer radius.
const SPOKE_RATIO: RangeInclusive<f64> = 0.01..=1.0;
/// A spiral's `turns`.
const TURNS: RangeInclusive<f64> = 0.01..=1024.0;
/// A spiral's `divergence`.
const DIVERGENCE: RangeInclusive<f64> = 0.0..=1000.0;
/// A spiral's `innerRadius`, as a fraction of its radius.
const INNER_RADIUS: RangeInclusive<f64> = 0.0..=0.999;
/// The `arcMode` names; `slice` and `open` are also read as `pie` and `arc`.
const ARC_MODES: [&str; 4] = ["whole", "pie", "chord", "arc"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.shapes.rectangle",
            "Rectangle",
            [],
            None,
            "{x, y, width, height: pt (0 or more), radius?: pt (0 or more), radii?: [tl,tr,br,bl] (each 0 or more), startAngle?, endAngle?: degrees (the part of the outline kept, clockwise from its first anchor, the top-left corner; each side an equal share of 360; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise about the centre (0: upright)} → {id}",
            has_doc,
            rectangle
        ),
        cmd!(
            "addon.shapes.ellipse",
            "Ellipse",
            [],
            None,
            "{x, y, width, height: pt (0 or more), startAngle?, endAngle?: degrees (counter-clockwise from 3 o'clock, as object.setLiveShape's pieStart and pieEnd; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise about the centre (0: upright)} → {id}",
            has_doc,
            ellipse
        ),
        cmd!(
            "addon.shapes.polygon",
            "Polygon",
            [],
            None,
            "{cx, cy, radius: pt (0 or more), sides=6: a whole number 3 to 1000, rounded=0, randomized=0: -10 to 10, startAngle?, endAngle?: degrees (the part of the outline kept, clockwise from its first anchor, the top corner; each side an equal share of 360; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise (0: a corner straight up)} → {id}",
            has_doc,
            polygon
        ),
        cmd!(
            "addon.shapes.star",
            "Star",
            [],
            None,
            "{cx, cy, radius1: pt (0 or more), radius2?: pt (0.01 to 1 times radius1; default half of it), points=5: a whole number 3 to 1000, rounded=0, randomized=0: -10 to 10, startAngle?, endAngle?: degrees (the part of the outline kept, clockwise from its first anchor, the top point; each side an equal share of 360; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise (0: a point straight up)} → {id}",
            has_doc,
            star
        ),
        cmd!(
            "addon.shapes.spiral",
            "Spiral",
            [],
            None,
            "{cx, cy, radius: pt (0 or more), turns=3: 0.01 to 1024, divergence=1: 0 to 1000, innerRadius=0: 0 to 0.999 of radius, clockwise=true} → {id}",
            has_doc,
            spiral
        ),
        cmd!(
            "addon.shapes.box3d",
            "3D Box",
            [],
            None,
            "{x, y, width, height: pt (0 or more), angleX=30 (degrees about the level axis: positive tips the top away from you), angleY=30 (degrees about the upright axis: positive turns the right side away from you), angleZ=0 (degrees clockwise in the page), parallelX=false, parallelY=true, parallelZ=false} → {id}",
            has_doc,
            box3d
        ),
        cmd!(
            "addon.shapes.configure",
            "Configure Shape",
            [],
            None,
            "{ids?, radius?: pt (0 or more; fitted to half each rectangle's shorter side), radii?: [tl,tr,br,bl] (each 0 or more), startAngle?, endAngle?: degrees (as addon.shapes.rectangle, ellipse, polygon and star take them: an ellipse's counter-clockwise from 3 o'clock, the others' clockwise along the outline), arcMode?: whole|pie|chord|arc, sides?: a whole number 3 to 1000, isStar?, spokeRatio?: 0.01 to 1, rounded?, randomized?: -10 to 10, turns?: 0.01 to 1024, divergence?: 0 to 1000, innerRadius?: 0 to 0.999, angleX?, angleY?: degrees (as addon.shapes.box3d: positive tips the top, or turns the right side, away from you), angleZ?: degrees clockwise in the page, parallelX?, parallelY?, parallelZ?, reset?: bool} an omitted or null value keeps each shape's setting → {updated}",
            has_selection,
            configure
        ),
    ]
}

/// The number `p` gives for `key`, or `default` when `p` leaves it out or gives null. A number
/// outside `range` is an error that names `key` and the range.
fn ranged(p: &Value, key: &str, default: f64, range: &RangeInclusive<f64>, cmd: &str) -> Result<f64> {
    let x = number_or(p, key, default, cmd)?;
    if !range.contains(&x) {
        let (min, max) = (range.start(), range.end());
        return Err(bad(cmd, format!("`{key}` must be between {min} and {max} (got {x})")));
    }
    Ok(x)
}

/// The number of sides or points `p` gives for `key`, or `default` when `p` leaves it out or gives
/// null. A fraction or a number outside [`COUNTS`] is an error.
fn count(p: &Value, key: &str, default: u32, cmd: &str) -> Result<u32> {
    let x = number_or(p, key, f64::from(default), cmd)?;
    if x.fract() != 0.0 || !COUNTS.contains(&x) {
        let (min, max) = (COUNTS.start(), COUNTS.end());
        return Err(bad(cmd, format!("`{key}` must be a whole number between {min} and {max} (got {x})")));
    }
    Ok(x as u32)
}

/// The size `p` gives for `key`: a number that is not negative.
fn size(p: &Value, key: &str, cmd: &str) -> Result<f64> {
    not_negative(f64_req(p, key, cmd)?, key, cmd)
}

/// The size `p` gives for `key`, or `default` when `p` leaves it out or gives null.
fn size_or(p: &Value, key: &str, default: f64, cmd: &str) -> Result<f64> {
    not_negative(number_or(p, key, default, cmd)?, key, cmd)
}

fn not_negative(x: f64, key: &str, cmd: &str) -> Result<f64> {
    if x < 0.0 {
        return Err(bad(cmd, format!("`{key}` must be a number not below 0")));
    }
    Ok(x)
}

/// The four corner radii `p` gives as `radii` ([tl, tr, br, bl]), or `None` when `p` leaves it out
/// or gives null. Anything but four numbers that are not negative is an error.
fn radii_param(p: &Value, cmd: &str) -> Result<Option<[f64; 4]>> {
    let key = "radii";
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let four = v.as_array().filter(|a| a.len() == 4).and_then(|a| {
                let mut out = [0.0; 4];
                for (slot, item) in out.iter_mut().zip(a) {
                    *slot = item.as_f64().filter(|r| *r >= 0.0)?;
                }
                Some(out)
            });
            four.map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be four numbers not below 0: [tl, tr, br, bl]")))
        }
    }
}

/// The mode `p` gives as `arcMode`, or `None` when `p` leaves it out or gives null. Any other
/// value than one of [`ARC_MODES`] (or `slice` or `open`) is an error.
fn arc_mode(p: &Value, cmd: &str) -> Result<Option<shapes::EllipseMode>> {
    let key = "arcMode";
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().and_then(parse_mode).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be one of {}", ARC_MODES.join(", ")))),
    }
}

fn parse_arc_params(p: &Value, cmd: &str) -> Result<((f64, f64), shapes::EllipseMode)> {
    let start = f64_or(p, "startAngle", 0.0);
    let end = f64_or(p, "endAngle", 360.0);
    let mode = arc_mode(p, cmd)?.unwrap_or(shapes::EllipseMode::Whole);
    let span = (end - start).rem_euclid(360.0);
    let pie = if mode == shapes::EllipseMode::Whole && (span < 1e-6 || (span - 360.0).abs() < 1e-6) { (0.0, 360.0) } else { (start, end) };
    Ok((pie, mode))
}

fn rect_of(p: &Value, c: &str) -> Result<Rect> {
    let x = f64_req(p, "x", c)?;
    let y = f64_req(p, "y", c)?;
    let w = size(p, "width", c)?;
    let h = size(p, "height", c)?;
    Ok(Rect::new(x, y, x + w, y + h))
}

fn rect_xf(r: Rect, rot: f64) -> Affine {
    if rot == 0.0 || !rot.is_finite() {
        Affine::translate(r.origin().to_vec2())
    } else {
        Affine::translate(r.center().to_vec2()) * vectorcraft_geom::rotate_deg(rot) * Affine::translate((-r.width() / 2.0, -r.height() / 2.0))
    }
}

fn insert_custom(s: &mut Session, label: &str, live: Option<LiveShape>, cs: CustomShape) -> Result<Value> {
    let path = match (&live, cs.needs_extra_storage()) {
        (Some(l), false) => l.to_path(),
        _ => cs.to_path(),
    };
    let kind = NodeKind::Path { path, rule: FillRule::NonZero, live: live.map(Box::new), clipping: false, guide: false };
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    let val = insert_look(s, label, kind, look, None, true)?;
    if cs.needs_extra_storage()
        && let Some(raw_id) = val.get("id").and_then(Value::as_u64)
        && let Ok(st) = s.doc_mut()
        && let Some(n) = std::sync::Arc::make_mut(&mut st.doc).node_mut(NodeId(raw_id))
    {
        cs.write_to_node(n);
        // A shape drawn pixel-aligned (Align New Objects to Pixel Grid) snaps its stored outline.
        super::super::object::snap_if_aligned(n);
    }
    Ok(val)
}

fn rectangle(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.rectangle";
    check_numbers(p, &["radius", "startAngle", "endAngle", "rotation"], c)?;
    let r = rect_of(p, c)?;
    let radius = size_or(p, "radius", 0.0, c)?;
    let radii = radii_param(p, c)?.unwrap_or([radius; 4]);
    let (pie, arc_mode) = parse_arc_params(p, c)?;
    let xf = rect_xf(r, f64_or(p, "rotation", 0.0));
    let live = LiveShape::Rectangle { w: r.width(), h: r.height(), radii, kinds: Default::default(), radii_relative: false, xf };
    let cs = CustomShape::Rectangle { w: r.width(), h: r.height(), radii, kinds: Default::default(), radii_relative: false, xf, pie, arc_mode };
    let label = if radii.iter().any(|&v| v > 0.0) { "Rounded Rectangle" } else { "Rectangle" };
    insert_custom(s, label, Some(live), cs)
}

fn ellipse(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.ellipse";
    check_numbers(p, &["startAngle", "endAngle", "rotation"], c)?;
    let r = rect_of(p, c)?;
    let (pie, arc_mode) = parse_arc_params(p, c)?;
    let xf = rect_xf(r, f64_or(p, "rotation", 0.0));
    let live = LiveShape::Ellipse { w: r.width(), h: r.height(), pie, xf };
    let cs = CustomShape::Ellipse { w: r.width(), h: r.height(), pie, arc_mode, xf };
    insert_custom(s, "Ellipse", Some(live), cs)
}

fn polygon(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.polygon";
    let center = Point::new(f64_req(p, "cx", c)?, f64_req(p, "cy", c)?);
    let r = size(p, "radius", c)?;
    check_numbers(p, &["sides", "rotation", "rounded", "randomized", "startAngle", "endAngle"], c)?;
    let sides = count(p, "sides", 6, c)?;
    let rot = f64_or(p, "rotation", 0.0);
    let rounded = ranged(p, "rounded", 0.0, &ROUNDING, c)?;
    let randomized = ranged(p, "randomized", 0.0, &ROUNDING, c)?;
    let (pie, arc_mode) = parse_arc_params(p, c)?;
    let xf = Affine::translate(center.to_vec2()) * vectorcraft_geom::rotate_deg(rot);
    let live = LiveShape::Polygon { radius: r, sides, xf, radii: vec![], kinds: vec![], radii_relative: false };
    let cs = CustomShape::Polygon {
        radius: r,
        sides,
        xf,
        radii: vec![],
        kinds: vec![],
        radii_relative: false,
        spoke_ratio: None,
        rounded,
        randomized,
        pie,
        arc_mode,
    };
    insert_custom(s, "Polygon", Some(live), cs)
}

fn star(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.star";
    let center = Point::new(f64_req(p, "cx", c)?, f64_req(p, "cy", c)?);
    let r1 = size(p, "radius1", c)?;
    check_numbers(p, &["radius2", "points", "rotation", "rounded", "randomized", "startAngle", "endAngle"], c)?;
    // The inner radius is a fraction of the outer one, in `SPOKE_RATIO`.
    let r2 = ranged(p, "radius2", r1 / 2.0, &(r1 * SPOKE_RATIO.start()..=r1 * SPOKE_RATIO.end()), c)?;
    let points = count(p, "points", 5, c)?;
    let rot = f64_or(p, "rotation", 0.0);
    let rounded = ranged(p, "rounded", 0.0, &ROUNDING, c)?;
    let randomized = ranged(p, "randomized", 0.0, &ROUNDING, c)?;
    let (pie, arc_mode) = parse_arc_params(p, c)?;
    let xf = Affine::translate(center.to_vec2()) * vectorcraft_geom::rotate_deg(rot);
    let ratio = if r1 > 1e-9 { (r2 / r1).clamp(*SPOKE_RATIO.start(), *SPOKE_RATIO.end()) } else { 0.5 };
    let cs = CustomShape::Polygon {
        radius: r1,
        sides: points,
        xf,
        radii: vec![],
        kinds: vec![],
        radii_relative: false,
        spoke_ratio: Some(ratio),
        rounded,
        randomized,
        pie,
        arc_mode,
    };
    insert_custom(s, "Star", None, cs)
}

fn spiral(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.spiral";
    let center = Point::new(f64_req(p, "cx", c)?, f64_req(p, "cy", c)?);
    let r = size(p, "radius", c)?;
    check_numbers(p, &["turns", "divergence", "innerRadius"], c)?;
    check_switches(p, &["clockwise"], c)?;
    let turns = ranged(p, "turns", 3.0, &TURNS, c)?;
    let divergence = ranged(p, "divergence", 1.0, &DIVERGENCE, c)?;
    let inner_radius = ranged(p, "innerRadius", 0.0, &INNER_RADIUS, c)?;
    let clockwise = bool_or(p, "clockwise", true);
    let cs = CustomShape::Spiral { center, radius: r, turns, divergence, inner_radius, clockwise };
    insert_custom(s, "Spiral", None, cs)
}

fn box3d(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.box3d";
    let r = rect_of(p, c)?;
    check_numbers(p, &["angleX", "angleY", "angleZ"], c)?;
    check_switches(p, &["parallelX", "parallelY", "parallelZ"], c)?;
    let angle_x = f64_or(p, "angleX", 30.0);
    let angle_y = f64_or(p, "angleY", 30.0);
    let angle_z = f64_or(p, "angleZ", 0.0);
    let parallel_x = bool_or(p, "parallelX", false);
    let parallel_y = bool_or(p, "parallelY", true);
    let parallel_z = bool_or(p, "parallelZ", false);
    let cs = CustomShape::Box3d {
        w: r.width(),
        h: r.height(),
        angle_x,
        angle_y,
        angle_z,
        parallel_x,
        parallel_y,
        parallel_z,
        xf: Affine::translate(r.origin().to_vec2()),
    };
    let kind = NodeKind::Path { path: cs.to_path(), rule: FillRule::NonZero, live: None, clipping: false, guide: false };
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    let val = insert_look(s, "3D Box", kind, look, Some("3D Box".into()), true)?;
    if let Some(raw_id) = val.get("id").and_then(Value::as_u64)
        && let Ok(st) = s.doc_mut()
        && let Some(n) = std::sync::Arc::make_mut(&mut st.doc).node_mut(NodeId(raw_id))
    {
        cs.write_to_node(n);
        super::super::object::snap_if_aligned(n);
    }
    Ok(val)
}

fn configure(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.configure";
    check_numbers(
        p,
        &[
            "radius",
            "startAngle",
            "endAngle",
            "sides",
            "spokeRatio",
            "rounded",
            "randomized",
            "turns",
            "divergence",
            "innerRadius",
            "angleX",
            "angleY",
            "angleZ",
        ],
        c,
    )?;
    check_switches(p, &["isStar", "parallelX", "parallelY", "parallelZ", "reset"], c)?;
    check_configure(p, c)?;
    let ids = targets(s, p)?;
    let reset = bool_or(p, "reset", false);
    let spiral_hint = p.get("turns").is_some() || p.get("divergence").is_some() || p.get("innerRadius").is_some();
    // The shapes the params change, worked out first so that a call that changes nothing records
    // no undo step.
    let changes: Vec<(NodeId, CustomShape)> = {
        let doc = &s.doc()?.doc;
        ids.into_iter()
            .filter_map(|id| {
                let n = doc.node(id)?;
                let before = CustomShape::read_for_update(n, spiral_hint)?;
                let mut cs = before.clone();
                let stored = CustomShape::read(n).is_some();
                (apply_shape_params(&mut cs, p, reset) && (cs != before || !stored)).then_some((id, cs))
            })
            .collect()
    };
    if changes.is_empty() {
        return Ok(json!({ "updated": 0 }));
    }
    let updated = s.edit("Live Shape", |d, _| {
        let mut count = 0usize;
        for (id, cs) in &changes {
            let Some(n) = d.node_mut(*id) else { continue };
            cs.write_to_node(n);
            count += 1;
        }
        Ok(count)
    })?;
    Ok(json!({ "updated": updated }))
}

/// Error unless every bounded value `p` gives is in its range, before any shape changes.
fn check_configure(p: &Value, c: &str) -> Result<()> {
    size_or(p, "radius", 0.0, c)?;
    radii_param(p, c)?;
    arc_mode(p, c)?;
    count(p, "sides", 6, c)?;
    ranged(p, "spokeRatio", 0.5, &SPOKE_RATIO, c)?;
    ranged(p, "rounded", 0.0, &ROUNDING, c)?;
    ranged(p, "randomized", 0.0, &ROUNDING, c)?;
    ranged(p, "turns", 3.0, &TURNS, c)?;
    ranged(p, "divergence", 1.0, &DIVERGENCE, c)?;
    ranged(p, "innerRadius", 0.0, &INNER_RADIUS, c)?;
    Ok(())
}

fn num_param(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn apply_arc_params(p: &Value, pie: &mut (f64, f64), arc_mode: &mut shapes::EllipseMode) -> bool {
    let mut changed = false;
    if let Some(a0) = num_param(p, "startAngle") {
        pie.0 = a0.rem_euclid(360.0);
        changed = true;
    }
    if let Some(a1) = num_param(p, "endAngle") {
        pie.1 = a1.rem_euclid(360.0);
        changed = true;
    }
    if let Some(m) = str_param(p, "arcMode").and_then(parse_mode) {
        *arc_mode = m;
        if *arc_mode == shapes::EllipseMode::Whole {
            *pie = (0.0, 360.0);
        } else if (pie.1 - pie.0).abs() < 1e-6 || ((pie.1 - pie.0).abs() - 360.0).abs() < 1e-6 {
            *pie = (0.0, 270.0);
        }
        changed = true;
    }
    changed
}

fn apply_rect_params(p: &Value, reset: bool, max_r: f64, radii: &mut [f64; 4], pie: &mut (f64, f64), arc_mode: &mut shapes::EllipseMode) -> bool {
    let mut changed = false;
    if reset {
        *radii = [0.0; 4];
        *pie = (0.0, 360.0);
        *arc_mode = shapes::EllipseMode::Whole;
        changed = true;
    }
    // A radius larger than half the rectangle's shorter side is fitted to that size.
    if let Some(r) = num_param(p, "radius") {
        *radii = [r.clamp(0.0, max_r); 4];
        changed = true;
    }
    if let Ok(Some(four)) = radii_param(p, "addon.shapes.configure") {
        *radii = four.map(|r| r.clamp(0.0, max_r));
        changed = true;
    }
    changed | apply_arc_params(p, pie, arc_mode)
}

fn apply_poly_params(p: &Value, reset: bool, sides: &mut u32, spoke_ratio: &mut Option<f64>, rounded: &mut f64, randomized: &mut f64) -> bool {
    let mut changed = false;
    if reset {
        *rounded = 0.0;
        *randomized = 0.0;
        if spoke_ratio.is_some() {
            *spoke_ratio = Some(0.5);
        }
        changed = true;
    }
    // `check_configure` has refused values outside their ranges.
    if let Some(n_sides) = num_param(p, "sides") {
        *sides = n_sides as u32;
        changed = true;
    }
    if let Some(star) = p.get("isStar").and_then(Value::as_bool) {
        *spoke_ratio = if star { Some(spoke_ratio.unwrap_or(0.5)) } else { None };
        changed = true;
    }
    if let Some(sr) = num_param(p, "spokeRatio") {
        *spoke_ratio = Some(sr);
        changed = true;
    }
    if let Some(rnd) = num_param(p, "rounded") {
        *rounded = rnd;
        changed = true;
    }
    if let Some(rand) = num_param(p, "randomized") {
        *randomized = rand;
        changed = true;
    }
    changed
}

fn apply_shape_params(cs: &mut CustomShape, p: &Value, reset: bool) -> bool {
    match cs {
        CustomShape::Rectangle { w, h, radii, pie, arc_mode, .. } => apply_rect_params(p, reset, w.min(*h) / 2.0, radii, pie, arc_mode),
        CustomShape::Ellipse { pie, arc_mode, .. } => {
            let mut changed = false;
            if reset {
                *pie = (0.0, 360.0);
                *arc_mode = shapes::EllipseMode::Whole;
                changed = true;
            }
            changed | apply_arc_params(p, pie, arc_mode)
        }
        CustomShape::Polygon { sides, spoke_ratio, rounded, randomized, pie, arc_mode, .. } => {
            if reset {
                *pie = (0.0, 360.0);
                *arc_mode = shapes::EllipseMode::Whole;
            }
            apply_poly_params(p, reset, sides, spoke_ratio, rounded, randomized) | apply_arc_params(p, pie, arc_mode)
        }
        CustomShape::Spiral { turns, divergence, inner_radius, .. } => {
            let mut changed = false;
            if reset {
                *turns = 3.0;
                *divergence = 1.0;
                *inner_radius = 0.0;
                changed = true;
            }
            if let Some(t) = num_param(p, "turns") {
                *turns = t;
                changed = true;
            }
            if let Some(div) = num_param(p, "divergence") {
                *divergence = div;
                changed = true;
            }
            if let Some(ir) = num_param(p, "innerRadius") {
                *inner_radius = ir;
                changed = true;
            }
            changed
        }
        CustomShape::Box3d { angle_x, angle_y, angle_z, parallel_x, parallel_y, parallel_z, .. } => {
            let mut changed = false;
            if reset {
                *angle_x = 30.0;
                *angle_y = 30.0;
                *angle_z = 0.0;
                *parallel_x = false;
                *parallel_y = true;
                *parallel_z = false;
                changed = true;
            }
            for (k, slot) in [("angleX", angle_x), ("angleY", angle_y), ("angleZ", angle_z)] {
                if let Some(v) = num_param(p, k) {
                    *slot = v;
                    changed = true;
                }
            }
            for (k, slot) in [("parallelX", parallel_x), ("parallelY", parallel_y), ("parallelZ", parallel_z)] {
                if let Some(v) = p.get(k).and_then(Value::as_bool) {
                    *slot = v;
                    changed = true;
                }
            }
            changed
        }
    }
}

fn parse_mode(m: &str) -> Option<shapes::EllipseMode> {
    match m {
        "whole" => Some(shapes::EllipseMode::Whole),
        "pie" | "slice" => Some(shapes::EllipseMode::Slice),
        "chord" => Some(shapes::EllipseMode::Chord),
        "arc" | "open" => Some(shapes::EllipseMode::Arc),
        _ => None,
    }
}
