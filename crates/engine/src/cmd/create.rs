//! Creating art: shapes, paths, text.

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, CharStyle, LiveShape, Node, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Anchor, AnchorKind, FillRule, PathData, Point, Rect, SubPath, shapes};

use super::newart::NewArt;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("shape.rectangle", "Rectangle", [], None, "{x, y, width, height, radius?: pt} → {id}", has_doc, rectangle),
        cmd!("shape.ellipse", "Ellipse", [], None, "{x, y, width, height} → {id}", has_doc, ellipse),
        cmd!("shape.polygon", "Polygon", [], None, "{cx, cy, radius, sides=6, rotation?: deg} → {id}", has_doc, polygon),
        cmd!("shape.star", "Star", [], None, "{cx, cy, radius1, radius2, points=5, rotation?: deg} → {id}", has_doc, star),
        cmd!(
            "shape.flare",
            "Flare",
            [],
            None,
            "{cx, cy, diameter=100, opacity=50 (%), brightness=30 (%), growth=20 (%), fuzziness=50 (%), rays=15, longest=300 (%), rayFuzziness=100 (%), x2?, y2? (ring end; else pathLength at direction), pathLength=300, rings=10, largest=50 (%), direction=45 (deg), seed?} → {id}",
            has_doc,
            flare
        ),
        cmd!("shape.line", "Line Segment", [], None, "{x1, y1, x2, y2} → {id}", has_doc, line),
        cmd!("shape.spiral", "Spiral", [], None, "{cx, cy, radius, decay=80 (%), segments=10, clockwise?} → {id}", has_doc, spiral),
        cmd!("shape.arc", "Arc", [], None, "{x1, y1, x2, y2, closed?} → {id}", has_doc, arc),
        cmd!("shape.rectangularGrid", "Rectangular Grid", [], None, "{x, y, width, height, rows=5, columns=5} → {id}", has_doc, rect_grid),
        cmd!("shape.polarGrid", "Polar Grid", [], None, "{x, y, width, height, concentric=5, radial=5} → {id}", has_doc, polar_grid),
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
            "{mode?: normal|behind|inside} (no param cycles; inside needs one selected path)",
            has_doc,
            draw_mode
        ),
        cmd!(
            "text.create",
            "Create Text",
            [],
            None,
            "{x, y, text, vertical?: bool = false, size?: pt, font?: family, style?, color?, area?: {width, height}} → {id}",
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

/// [`add_look`] with just `appearance`.
pub(crate) fn add_node(s: &mut Session, label: &str, kind: NodeKind, appearance: Appearance, name: Option<String>) -> Result<Value> {
    add_look(s, label, kind, NewArt::plain(appearance), name)
}

/// Insert a new object looking like `look` (drawing modes apply); select it.
pub(crate) fn add_look(s: &mut Session, label: &str, kind: NodeKind, look: NewArt, name: Option<String>) -> Result<Value> {
    let parent = s.doc()?.insertion_parent();
    let mode = s.draw_mode;
    let inside = s.draw_inside;
    let behind_of = s.doc()?.selection.in_paint_order(&s.doc()?.doc).first().copied();
    let id = s.edit(label, |d, sel| {
        let id = d.alloc_id();
        let mut n = Node::new(id, kind);
        look.apply(d, &mut n);
        n.name = name;
        match (mode, inside) {
            (crate::DrawMode::Inside, Some(target)) if d.node(target).is_some() => {
                // Wrap the target once in a clip group [mask copy, target, …new art].
                let group = match d.parent_of(target).and_then(|p| d.node(p).map(|pn| (p, pn.kind.clone()))) {
                    Some((p, NodeKind::Group { clip: true, children })) if children.get(1).is_some_and(|c| c.id == target) => p,
                    _ => {
                        let (par, idx, _) = d.position(target).ok_or(crate::EngineError::NoNode(target))?;
                        let mut mask = d.node(target).cloned().ok_or(crate::EngineError::NoNode(target))?;
                        mask.id = d.alloc_id();
                        mask.appearance = Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 0.0);
                        if let NodeKind::Path { clipping, .. } = &mut mask.kind {
                            *clipping = true;
                        }
                        let gid = d.alloc_id();
                        d.insert(par, idx + 1, Node::new(gid, NodeKind::Group { children: vec![std::sync::Arc::new(mask)], clip: true }))?;
                        d.move_node(target, Some(gid), usize::MAX)?;
                        gid
                    }
                };
                d.insert(Some(group), usize::MAX, n)?;
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
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn draw_mode(s: &mut Session, p: &Value) -> Result<Value> {
    let mode = match str_param(p, "mode") {
        Some("behind") => crate::DrawMode::Behind,
        Some("inside") => crate::DrawMode::Inside,
        Some("normal") => crate::DrawMode::Normal,
        None => match s.draw_mode {
            crate::DrawMode::Normal => crate::DrawMode::Behind,
            crate::DrawMode::Behind if s.doc()?.selection.len() == 1 => crate::DrawMode::Inside,
            _ => crate::DrawMode::Normal,
        },
        Some(o) => return Err(bad("view.drawMode", format!("unknown mode `{o}`"))),
    };
    if mode == crate::DrawMode::Inside {
        let st = s.doc()?;
        let target =
            st.selection.objects.first().copied().filter(|id| {
                matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_)))
            });
        match target {
            Some(t) if st.selection.len() == 1 => s.draw_inside = Some(t),
            _ => return Err(bad("view.drawMode", "Draw Inside needs exactly one selected path, compound path or text")),
        }
    } else {
        s.draw_inside = None;
    }
    s.draw_mode = mode;
    Ok(json!({ "mode": mode }))
}

fn path_kind(path: PathData, live: Option<LiveShape>) -> NodeKind {
    NodeKind::Path { path, rule: FillRule::NonZero, live, clipping: false, guide: false }
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

fn rectangle(s: &mut Session, p: &Value) -> Result<Value> {
    let r = rect_of(p, "shape.rectangle")?;
    let radius = f64_or(p, "radius", 0.0).max(0.0);
    let live = LiveShape::Rectangle { w: r.width(), h: r.height(), radii: [radius; 4], xf: Affine::translate(r.origin().to_vec2()) };
    let label = if radius > 0.0 { "Rounded Rectangle" } else { "Rectangle" };
    add_art(s, label, path_kind(live.to_path(), Some(live)), None)
}

fn ellipse(s: &mut Session, p: &Value) -> Result<Value> {
    let r = rect_of(p, "shape.ellipse")?;
    let live = LiveShape::Ellipse { w: r.width(), h: r.height(), pie: (0.0, 360.0), xf: Affine::translate(r.origin().to_vec2()) };
    add_art(s, "Ellipse", path_kind(live.to_path(), Some(live)), None)
}

fn polygon(s: &mut Session, p: &Value) -> Result<Value> {
    let c = Point::new(f64_req(p, "cx", "shape.polygon")?, f64_req(p, "cy", "shape.polygon")?);
    let r = f64_req(p, "radius", "shape.polygon")?.abs();
    let sides = p.get("sides").and_then(Value::as_u64).unwrap_or(6).clamp(3, 1000) as u32;
    let rot = f64_or(p, "rotation", 0.0);
    let live = LiveShape::Polygon { radius: r, sides, xf: Affine::translate(c.to_vec2()) * Affine::rotate(rot.to_radians()) };
    add_art(s, "Polygon", path_kind(live.to_path(), Some(live)), None)
}

fn star(s: &mut Session, p: &Value) -> Result<Value> {
    let c = Point::new(f64_req(p, "cx", "shape.star")?, f64_req(p, "cy", "shape.star")?);
    let r1 = f64_req(p, "radius1", "shape.star")?.abs();
    let r2 = f64_or(p, "radius2", r1 / 2.0).abs();
    let n = p.get("points").and_then(Value::as_u64).unwrap_or(5).clamp(2, 1000) as u32;
    let path = shapes::star(c, r1, r2, n, f64_or(p, "rotation", 0.0));
    add_art(s, "Star", path_kind(path, None), None)
}

/// Flare tool: a lens flare built from radial-gradient paths in Screen mode — a bright centre, a halo
/// ring, rays of varying length and rings along the path to the end point.
fn flare(s: &mut Session, p: &Value) -> Result<Value> {
    use vectorcraft_color::{BlendMode, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
    let c = Point::new(f64_req(p, "cx", "shape.flare")?, f64_req(p, "cy", "shape.flare")?);
    let pct = |k: &str, d: f64| (f64_or(p, k, d) / 100.0).clamp(0.0, 10.0);
    let r = (f64_or(p, "diameter", 100.0) / 2.0).max(0.5);
    let opacity = pct("opacity", 50.0).min(1.0) as f32;
    let brightness = pct("brightness", 30.0).min(1.0) as f32;
    let growth = pct("growth", 20.0);
    let fuzz = pct("fuzziness", 50.0).min(1.0);
    let rays = p.get("rays").and_then(Value::as_u64).unwrap_or(15).min(50) as usize;
    let longest = pct("longest", 300.0);
    let ray_fuzz = pct("rayFuzziness", 100.0).min(1.0);
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
    if rays > 0 {
        let mut bp = vectorcraft_geom::BezPath::new();
        let mut reach: f64 = 0.0;
        for i in 0..rays {
            let a = std::f64::consts::TAU * (i as f64 + rnd() * 0.6) / rays as f64;
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
    let parent = s.doc()?.insertion_parent();
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
    // Lines are stroked, never filled (Illustrator ignores the fill for new open lines).
    let look = s.new_art_look(
        vectorcraft_color::Paint::None,
        if s.paint.stroke.is_none() { vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK) } else { s.paint.stroke.clone() },
        s.paint.stroke_width.max(0.1),
    );
    add_look(s, "Line", path_kind(live.to_path(), Some(live)), look, None)
}

fn spiral(s: &mut Session, p: &Value) -> Result<Value> {
    let c = Point::new(f64_req(p, "cx", "shape.spiral")?, f64_req(p, "cy", "shape.spiral")?);
    let path = shapes::spiral(
        c,
        f64_req(p, "radius", "shape.spiral")?,
        f64_or(p, "decay", 80.0),
        p.get("segments").and_then(Value::as_u64).unwrap_or(10).clamp(2, 1000) as u32,
        bool_or(p, "clockwise", true),
    );
    add_art(s, "Spiral", path_kind(path, None), None)
}

fn arc(s: &mut Session, p: &Value) -> Result<Value> {
    let a = Point::new(f64_req(p, "x1", "shape.arc")?, f64_req(p, "y1", "shape.arc")?);
    let b = Point::new(f64_req(p, "x2", "shape.arc")?, f64_req(p, "y2", "shape.arc")?);
    add_art(s, "Arc", path_kind(shapes::arc(a, b, 0.0, bool_or(p, "closed", false)), None), None)
}

fn grid_group(s: &mut Session, label: &str, paths: Vec<PathData>) -> Result<Value> {
    let parent = s.doc()?.insertion_parent();
    let look = s.new_art_look(vectorcraft_color::Paint::None, s.paint.stroke.clone(), s.paint.stroke_width);
    let id = s.edit(label, |d, sel| {
        let children = paths
            .into_iter()
            .map(|pd| {
                let mut n = Node::path(d.alloc_id(), pd, Appearance::default());
                look.apply(d, &mut n);
                std::sync::Arc::new(n)
            })
            .collect();
        let gid = d.alloc_id();
        d.insert(parent, usize::MAX, Node::group(gid, children))?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn rect_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let r = rect_of(p, "shape.rectangularGrid")?;
    let rows = p.get("rows").and_then(Value::as_u64).unwrap_or(5).min(999) as u32;
    let cols = p.get("columns").and_then(Value::as_u64).unwrap_or(5).min(999) as u32;
    grid_group(s, "Rectangular Grid", shapes::rectangular_grid(r, rows, cols, true))
}

fn polar_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let r = rect_of(p, "shape.polarGrid")?;
    let c = p.get("concentric").and_then(Value::as_u64).unwrap_or(5).min(999) as u32;
    let rad = p.get("radial").and_then(Value::as_u64).unwrap_or(5).min(999) as u32;
    grid_group(s, "Polar Grid", shapes::polar_grid(r, c, rad))
}

pub(crate) fn anchor_from_json(v: &Value) -> Option<Anchor> {
    let p = Point::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?);
    let h_in = point_param(v, "in").unwrap_or(p);
    let h_out = point_param(v, "out").unwrap_or(p);
    let mut a = Anchor::with_handles(p, h_in, h_out);
    if v.get("smooth").and_then(Value::as_bool) == Some(true) {
        a.kind = AnchorKind::Smooth;
    }
    Some(a)
}

fn path_create(s: &mut Session, p: &Value) -> Result<Value> {
    let path = if let Some(d) = str_param(p, "d") {
        let bp = vectorcraft_geom::BezPath::from_svg(d).map_err(|e| bad("path.create", format!("bad path data: {e}")))?;
        PathData::from_bezpath(&bp)
    } else {
        let anchors: Vec<Anchor> = p
            .get("anchors")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("path.create", "missing anchors"))?
            .iter()
            .filter_map(anchor_from_json)
            .collect();
        if anchors.is_empty() {
            return Err(bad("path.create", "need at least one anchor"));
        }
        PathData::single(SubPath::new(anchors, bool_or(p, "closed", false)))
    };
    let mut look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    // Only closed shapes are filled: an open path is just its lines (the Pen fills it when it
    // closes), stroked in black if the stroke is None so it stays visible.
    let ap = &mut look.appearance;
    if !path.is_closed() {
        ap.items.retain(|i| !i.is_fill());
        if ap.stroke_paint().is_none() {
            ap.set_stroke(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK));
        }
    }
    add_look(s, "Pen", path_kind(path, None), look, None)
}

fn text_create(s: &mut Session, p: &Value) -> Result<Value> {
    let x = f64_req(p, "x", "text.create")?;
    let y = f64_req(p, "y", "text.create")?;
    let text = str_param(p, "text").unwrap_or("");
    let mut t = TextObject::point(Point::new(x, y), text, new_type_style(s, p));
    t.vertical = p.get("vertical").and_then(Value::as_bool).unwrap_or(false);
    if let Some(a) = p.get("area") {
        let w = f64_or(a, "width", 200.0);
        let h = f64_or(a, "height", 100.0);
        t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, h)) };
    }
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    t.cached_bounds = Some(lay.bounds);
    add_node(s, "Type", NodeKind::Text(Box::new(t)), Appearance::default(), None)
}

/// The character style new type gets: `size`, `font`, `style` and `color` from `p`, else the
/// defaults, filled with the current fill unless that is None or white (black then).
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
    }
    style.fill = match p.get("color").and_then(color_value) {
        Some(c) => vectorcraft_color::Paint::solid(c),
        None if !s.paint.fill.is_none() && s.paint.fill != vectorcraft_color::Paint::solid(vectorcraft_color::Color::WHITE) => s.paint.fill.clone(),
        None => vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
    };
    style
}
