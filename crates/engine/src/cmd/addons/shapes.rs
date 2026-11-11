//! 3D Box and customizable shapes (`addon.shapes.*`), a built-in add-on: creates rotatable 3D boxes
//! (`addon.shapes.box3d`) and customizable rectangles, ellipses, polygons, stars and spirals
//! (`addon.shapes.rectangle`, `addon.shapes.ellipse`, `addon.shapes.polygon`, `addon.shapes.star`,
//! `addon.shapes.spiral`), and configures extended live-shape parameters on selected shapes
//! (`addon.shapes.configure`: symmetric or per-corner radii, pie/chord/arc/whole modes, polygon/star
//! spoke ratio, corner rounding, randomization, spiral turns/divergence/inner radius, and 3D box
//! rotation angles).

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, Point, Rect, shapes};
use vectorcraft_tools::addons::box3d::CustomShape;

use super::super::create::insert_look;
use super::super::{
    CommandSpec, Result, Session, bool_or, check_numbers, check_switches, cmd, f64_or, f64_req, has_doc, has_selection, str_param, targets,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.shapes.rectangle",
            "Rectangle",
            [],
            None,
            "{x, y, width, height, radius?: pt, radii?: [tl,tr,br,bl], startAngle?, endAngle?: degrees (the part of the outline kept, clockwise from its first anchor, the top-left corner; each side an equal share of 360; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise about the centre (0: upright)} → {id}",
            has_doc,
            rectangle
        ),
        cmd!(
            "addon.shapes.ellipse",
            "Ellipse",
            [],
            None,
            "{x, y, width, height, startAngle?, endAngle?: degrees (counter-clockwise from 3 o'clock, as object.setLiveShape's pieStart and pieEnd; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise about the centre (0: upright)} → {id}",
            has_doc,
            ellipse
        ),
        cmd!(
            "addon.shapes.polygon",
            "Polygon",
            [],
            None,
            "{cx, cy, radius, sides=6, rounded=0, randomized=0, startAngle?, endAngle?: degrees (the part of the outline kept, clockwise from its first anchor, the top corner; each side an equal share of 360; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise (0: a corner straight up)} → {id}",
            has_doc,
            polygon
        ),
        cmd!(
            "addon.shapes.star",
            "Star",
            [],
            None,
            "{cx, cy, radius1, radius2?, points=5, rounded=0, randomized=0, startAngle?, endAngle?: degrees (the part of the outline kept, clockwise from its first anchor, the top point; each side an equal share of 360; default 0 and 360), arcMode?: whole|pie|chord|arc, rotation?: deg clockwise (0: a point straight up)} → {id}",
            has_doc,
            star
        ),
        cmd!(
            "addon.shapes.spiral",
            "Spiral",
            [],
            None,
            "{cx, cy, radius, turns=3, divergence=1, innerRadius=0, clockwise=true} → {id}",
            has_doc,
            spiral
        ),
        cmd!(
            "addon.shapes.box3d",
            "3D Box",
            [],
            None,
            "{x, y, width, height, angleX=30 (degrees about the level axis: positive tips the top away from you), angleY=30 (degrees about the upright axis: positive turns the right side away from you), angleZ=0 (degrees clockwise in the page), parallelX=false, parallelY=true, parallelZ=false} → {id}",
            has_doc,
            box3d
        ),
        cmd!(
            "addon.shapes.configure",
            "Configure Shape",
            [],
            None,
            "{ids?, radius?, radii?: [tl,tr,br,bl], startAngle?, endAngle?: degrees (as addon.shapes.rectangle, ellipse, polygon and star take them: an ellipse's counter-clockwise from 3 o'clock, the others' clockwise along the outline), arcMode?: whole|pie|chord|arc, sides?, isStar?, spokeRatio?, rounded?, randomized?, turns?, divergence?, innerRadius?, angleX?, angleY?: degrees (as addon.shapes.box3d: positive tips the top, or turns the right side, away from you), angleZ?: degrees clockwise in the page, parallelX?, parallelY?, parallelZ?, reset?: bool} → {updated}",
            has_selection,
            configure
        ),
    ]
}

fn parse_arc_params(p: &Value) -> ((f64, f64), shapes::EllipseMode) {
    let start = f64_or(p, "startAngle", 0.0);
    let end = f64_or(p, "endAngle", 360.0);
    let mode = match str_param(p, "arcMode") {
        Some("pie" | "slice") => shapes::EllipseMode::Slice,
        Some("chord") => shapes::EllipseMode::Chord,
        Some("arc" | "open") => shapes::EllipseMode::Arc,
        _ => shapes::EllipseMode::Whole,
    };
    let span = (end - start).rem_euclid(360.0);
    let pie = if mode == shapes::EllipseMode::Whole && (span < 1e-6 || (span - 360.0).abs() < 1e-6) { (0.0, 360.0) } else { (start, end) };
    (pie, mode)
}

fn rect_of(p: &Value, c: &str) -> Result<Rect> {
    let x = f64_req(p, "x", c)?;
    let y = f64_req(p, "y", c)?;
    let w = f64_req(p, "width", c)?;
    let h = f64_req(p, "height", c)?;
    Ok(Rect::new(x, y, x + w.abs(), y + h.abs()).abs())
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
    }
    Ok(val)
}

fn rectangle(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.rectangle";
    check_numbers(p, &["radius", "startAngle", "endAngle", "rotation"], c)?;
    let r = rect_of(p, c)?;
    let radius = f64_or(p, "radius", 0.0).max(0.0);
    let mut radii = [radius; 4];
    if let Some(arr) = p.get("radii").and_then(Value::as_array)
        && arr.len() == 4
    {
        for (i, item) in arr.iter().enumerate() {
            if let Some(val) = item.as_f64().filter(|n| n.is_finite()) {
                radii[i] = val.max(0.0);
            }
        }
    }
    let (pie, arc_mode) = parse_arc_params(p);
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
    let (pie, arc_mode) = parse_arc_params(p);
    let xf = rect_xf(r, f64_or(p, "rotation", 0.0));
    let live = LiveShape::Ellipse { w: r.width(), h: r.height(), pie, xf };
    let cs = CustomShape::Ellipse { w: r.width(), h: r.height(), pie, arc_mode, xf };
    insert_custom(s, "Ellipse", Some(live), cs)
}

fn polygon(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "addon.shapes.polygon";
    let center = Point::new(f64_req(p, "cx", c)?, f64_req(p, "cy", c)?);
    let r = f64_req(p, "radius", c)?.abs();
    check_numbers(p, &["sides", "rotation", "rounded", "randomized", "startAngle", "endAngle"], c)?;
    let sides = (f64_or(p, "sides", 6.0).round() as i64).clamp(3, 1000) as u32;
    let rot = f64_or(p, "rotation", 0.0);
    let rounded = f64_or(p, "rounded", 0.0).clamp(-10.0, 10.0);
    let randomized = f64_or(p, "randomized", 0.0).clamp(-10.0, 10.0);
    let (pie, arc_mode) = parse_arc_params(p);
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
    let r1 = f64_req(p, "radius1", c)?.abs();
    check_numbers(p, &["radius2", "points", "rotation", "rounded", "randomized", "startAngle", "endAngle"], c)?;
    let r2 = f64_or(p, "radius2", r1 / 2.0).abs();
    let points = (f64_or(p, "points", 5.0).round() as i64).clamp(3, 1000) as u32;
    let rot = f64_or(p, "rotation", 0.0);
    let rounded = f64_or(p, "rounded", 0.0).clamp(-10.0, 10.0);
    let randomized = f64_or(p, "randomized", 0.0).clamp(-10.0, 10.0);
    let (pie, arc_mode) = parse_arc_params(p);
    let xf = Affine::translate(center.to_vec2()) * vectorcraft_geom::rotate_deg(rot);
    let ratio = if r1 > 1e-9 { (r2 / r1).clamp(0.01, 1.0) } else { 0.5 };
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
    let r = f64_req(p, "radius", c)?.abs();
    check_numbers(p, &["turns", "divergence", "innerRadius"], c)?;
    check_switches(p, &["clockwise"], c)?;
    let turns = f64_or(p, "turns", 3.0).clamp(0.01, 1024.0);
    let divergence = f64_or(p, "divergence", 1.0).clamp(0.0, 1000.0);
    let inner_radius = f64_or(p, "innerRadius", 0.0).clamp(0.0, 0.999);
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
    let ids = targets(s, p)?;
    let reset = bool_or(p, "reset", false);
    let spiral_hint = p.get("turns").is_some() || p.get("divergence").is_some() || p.get("innerRadius").is_some();
    let updated = s.edit("Live Shape", |d, _| {
        let mut count = 0usize;
        for id in ids {
            let Some(n) = d.node_mut(id) else { continue };
            let Some(mut cs) = CustomShape::read_for_update(n, spiral_hint) else { continue };
            if apply_shape_params(&mut cs, p, reset) {
                cs.write_to_node(n);
                count += 1;
            }
        }
        Ok(count)
    })?;
    Ok(json!({ "updated": updated }))
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
    if let Some(m) = str_param(p, "arcMode") {
        *arc_mode = parse_mode(m);
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
    if let Some(r) = num_param(p, "radius") {
        *radii = [r.clamp(0.0, max_r); 4];
        changed = true;
    }
    if let Some(arr) = p.get("radii").and_then(Value::as_array)
        && arr.len() == 4
    {
        for (i, v) in arr.iter().enumerate() {
            if let Some(r) = v.as_f64().filter(|x| x.is_finite()) {
                radii[i] = r.clamp(0.0, max_r);
                changed = true;
            }
        }
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
    if let Some(n_sides) = p.get("sides").and_then(Value::as_u64) {
        *sides = (n_sides as u32).clamp(3, 1000);
        changed = true;
    }
    if let Some(star) = p.get("isStar").and_then(Value::as_bool) {
        *spoke_ratio = if star { Some(spoke_ratio.unwrap_or(0.5)) } else { None };
        changed = true;
    }
    if let Some(sr) = num_param(p, "spokeRatio") {
        *spoke_ratio = Some(sr.clamp(0.01, 1.0));
        changed = true;
    }
    if let Some(rnd) = num_param(p, "rounded") {
        *rounded = rnd.clamp(-10.0, 10.0);
        changed = true;
    }
    if let Some(rand) = num_param(p, "randomized") {
        *randomized = rand.clamp(-10.0, 10.0);
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
                *turns = t.clamp(0.01, 1024.0);
                changed = true;
            }
            if let Some(div) = num_param(p, "divergence") {
                *divergence = div.clamp(0.0, 1000.0);
                changed = true;
            }
            if let Some(ir) = num_param(p, "innerRadius") {
                *inner_radius = ir.clamp(0.0, 0.999);
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

fn parse_mode(m: &str) -> shapes::EllipseMode {
    match m {
        "pie" | "slice" => shapes::EllipseMode::Slice,
        "chord" => shapes::EllipseMode::Chord,
        "arc" | "open" => shapes::EllipseMode::Arc,
        _ => shapes::EllipseMode::Whole,
    }
}
