use super::spans::Spans;
use super::transform::{TRANSFORM_SWITCHES, about, apply_transform, scaling_options, transforms_objects};
use super::*;

pub(super) fn set_bounds(s: &mut Session, p: &Value) -> Result<Value> {
    match bounds_transform(s, p, selected_roots_to_transform(s)?, 0.0)? {
        Bounds::Anchors(ids, b) => set_anchor_bounds(s, p, ids, b),
        Bounds::Objects(ids, xf) => apply_transform(s, "Transform", ids, xf, p, false),
    }
}

/// What `object.setBounds` does with its values ([`bounds_transform`]).
enum Bounds {
    /// With anchors selected with Direct Selection: the objects and the box of the anchors that
    /// [`set_anchor_bounds`] moves or scales.
    Anchors(Vec<NodeId>, Rect),
    /// The objects and the transform that gives them the values.
    Objects(Vec<NodeId>, Affine),
}

/// A previewed `object.setBounds` that transforms objects runs as `object.transform` with the
/// transform [`bounds_transform`] finds for it, so the interaction commits that transform (Transform
/// Again repeats it, and the journal holds it). A bounding-box handle drag with Use Preview Bounds
/// previews `object.setBounds`. The objects are those `object.transform` transforms (a path of a
/// compound path selected on its own is one), and the art keeps at least 1e-4 of its width and
/// height, as with [`vectorcraft_tools::bbox::scale_for_drag`]. `None` with anchors selected with
/// Direct Selection and when the values are not valid.
pub(crate) fn bounds_rewrite(s: &Session, cmd: &str, params: &Value) -> Option<(String, Value)> {
    if cmd != "object.setBounds" {
        return None;
    }
    let Ok(Bounds::Objects(_, xf)) = bounds_transform(s, params, selected_roots_to_transform(s).ok()?, 1e-4) else { return None };
    let mut p = params.as_object().cloned().unwrap_or_default();
    for k in ["x", "y", "width", "height", "reference", "proportional"] {
        p.remove(k);
    }
    p.insert("matrix".into(), json!(xf.as_coeffs()));
    Some(("object.transform".to_string(), Value::Object(p)))
}

/// How `object.setBounds` with params `p` changes the objects `ids`. A scale that would leave the
/// art with a width or height below `least` of the one it has keeps `least` of it.
fn bounds_transform(s: &Session, p: &Value, ids: Vec<NodeId>, least: f64) -> Result<Bounds> {
    check_numbers(p, &["x", "y", "width", "height", "reference"], "object.setBounds")?;
    let st = s.doc()?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.setBounds")?;
    check_switches(p, &["proportional"], "object.setBounds")?;
    if transforms_objects(p)
        && !bool_or(p, "copy", false)
        && let Some(b) = effects::preview::anchor_bounds(&st.selection, &st.doc, s.prefs.use_preview_bounds)
    {
        return Ok(Bounds::Anchors(ids, b));
    }
    let d = &s.doc()?.doc;
    // Sizes are measured square to the bounding box, which a rotation turns with the objects:
    // the math below runs in its frame.
    let frame = d.oriented_bounds(&ids, false).ok_or_else(|| EngineError::Other("no bounds".into()))?;
    let g = frame.rect;
    // With Use Preview Bounds the values measure the visual box as drawn, whose margins around the
    // geometric box (left, top, right, bottom) scale with the strokes or stay. The margins of
    // `vs`, the visual box without the strokes in symbols' art, scale; those strokes keep their
    // weight when an instance scales, so the rest of the margins stay.
    let (v, vs) = if s.prefs.use_preview_bounds {
        (effects::preview::oriented_bounds(d, &ids, true).map_or(g, |b| b.rect), scaling_box(d, &ids, &frame).unwrap_or(g))
    } else {
        (g, g)
    };
    let margins = |b: Rect| [g.x0 - b.x0, g.y0 - b.y0, b.x1 - g.x1, b.y1 - g.y1];
    let (m, all) = (margins(vs), margins(v));
    let fixed = [all[0] - m[0], all[1] - m[1], all[2] - m[2], all[3] - m[3]];
    let refi = p.get("reference").and_then(Value::as_u64).unwrap_or(4).min(8) as usize;
    let rp = vectorcraft_geom::reference_point(v, refi);
    let (w, h) = wanted_size(p, v);
    // The geometric scale that gives the visual size: margins grow by the strokes' scale `f`. It is
    // exact for proportional scales of one path; [`Spans::refit`] checks it, and solves for the
    // scales where it misses.
    let strokes = bool_or(p, "strokes", s.prefs.scale_strokes);
    let ratio = |size: f64, geo: f64, margin: f64| if geo > 1e-9 { (size - margin) / geo } else { 1.0 };
    let f = |sx: f64, sy: f64| if strokes { (sx * sy).abs().sqrt() } else { 1.0 };
    // The strokes' scale `f0`: the size left after the margins in symbols' art, which stay, over
    // `vs`, whose margins scale.
    let (lx, ly) = (ratio(w - fixed[0] - fixed[2], vs.width(), 0.0), ratio(h - fixed[1] - fixed[3], vs.height(), 0.0));
    let f0 = f(if g.width() > 1e-9 { lx } else { 1.0 }, if g.height() > 1e-9 { ly } else { 1.0 });
    // When a size of 0 or more is smaller than the margins and some of them stay, the geometry
    // gets no width or height: the visual box is then as small as it can be. The margins in
    // symbols' art stay, and so does every margin when strokes do not scale.
    let fit = |size: f64, geo: f64, scaling: f64, kept: f64| {
        let s = ratio(size, geo, scaling * f0 + kept);
        let stays = if strokes { kept } else { scaling + kept };
        if s < 0.0 && size >= 0.0 && stays > 0.0 { 0.0 } else { s }
    };
    let sx = fit(w, g.width(), m[0] + m[2], fixed[0] + fixed[2]);
    let sy = fit(h, g.height(), m[1] + m[3], fixed[1] + fixed[3]);
    let scale = about(rp, Affine::scale_non_uniform(sx, sy));
    // Move the new visual box's reference point to x, y (default: where it was): the geometric
    // box's reference point moves with the art, the margin from it to `vs`'s by `f`, and the
    // margin from that one to the visual box's stays.
    let gp = vectorcraft_geom::reference_point(g, refi);
    let sp = vectorcraft_geom::reference_point(vs, refi);
    let new_rp = scale * gp + (sp - gp) * f(sx, sy) + (rp - sp);
    // x, y place the reference point on the page.
    let page = frame.to_doc() * rp;
    // When every value given is the current one, the transform is the identity. Computing it can
    // leave a rounding error in it on a rotated object or with Use Preview Bounds.
    let current = |k: &str, now: f64| p.get(k).and_then(Value::as_f64).is_none_or(|x| x == now);
    if current("x", page.x) && current("y", page.y) && current("width", v.width()) && current("height", v.height()) {
        return Ok(Bounds::Objects(ids, Affine::IDENTITY));
    }
    let to = frame.to_local(Point::new(f64_or(p, "x", page.x), f64_or(p, "y", page.y)));
    // The scales and the point above can miss the size or the reference point when the margins of
    // the objects differ, the members of groups and the objects in symbols' art included, and with
    // strokes that scale, when the sizes are out of proportion. The objects are then measured one
    // by one ([`Spans::measure`]), and the scales solved for. Objects with geometry effects are
    // measured as drawn after each scale tried instead ([`refit_drawn`]).
    let symbols = d.symbol_boxes();
    let drawn = || ids.iter().filter_map(|id| d.node(*id)).any(|n| effects::preview::draws_geometry(&symbols, n));
    let solved = if !(s.prefs.use_preview_bounds && w >= 0.0 && h >= 0.0) {
        None
    } else if transforms_objects(p) && !bool_or(p, "typeAreas", false) && drawn() {
        let options = scaling_options(s, p);
        let to_local = frame.to_doc().inverse();
        let measure = |sx: f64, sy: f64| {
            let xf = frame.conjugate(about(rp, Affine::scale_non_uniform(sx, sy)));
            let sc = if Scaling::factor(xf).is_some() { options } else { Scaling::default() };
            ids.iter().filter_map(|id| d.node(*id)).fold(None, |acc, n| {
                let mut n = n.clone();
                n.transform(xf, sc);
                vectorcraft_geom::union_opt(acc, effects::preview::bounds_in(&symbols, &n, to_local))
            })
        };
        // The scales above keep the effects' reach as a margin that stays, and give 0 for a size
        // smaller than that reach. Strokes and effects that scale shrink the reach with the art, so
        // the search then starts from the size wanted over the size as drawn.
        let seed = |s: f64, size: f64, now: f64| if s <= 0.0 && size > 0.0 && now > 1e-9 { size / now } else { s };
        refit_drawn(measure, refi, [w, h], (seed(sx, w, v.width()), seed(sy, h, v.height())), [g.width() > 1e-9, g.height() > 1e-9])
    } else {
        Spans::measure(d, &ids, &frame, strokes).refit(rp, refi, [w, h], (sx, sy, new_rp))
    };
    let (sx, sy, new_rp) = solved.unwrap_or((sx, sy, new_rp));
    // A scale below `least` of art with a width (height) keeps `least`, and the reference point
    // moves with the scales as above.
    let keep = |s: f64, geo: f64| if geo > 1e-9 && (0.0..least).contains(&s) { least } else { s };
    let (kx, ky) = (keep(sx, g.width()), keep(sy, g.height()));
    let scale = about(rp, Affine::scale_non_uniform(kx, ky));
    let new_rp = if (kx, ky) == (sx, sy) { new_rp } else { scale * gp + (sp - gp) * f(kx, ky) + (rp - sp) };
    let xf = frame.conjugate(Affine::translate(to - new_rp) * scale);
    Ok(Bounds::Objects(ids, xf))
}

/// The scales along x and y that give the box `measure` returns for them the size `size`, and point
/// `refi` of that box: the point that goes to the reference point's place. The search starts at
/// `start`. Each step measures how the width and the height change with each scale that `free`
/// marks true, by measuring again with that scale moved by 1e-4 of it (at least 1e-6), and moves
/// those scales by the least-squares solution of that linear change, never below 0. The width can
/// change with the y scale and the height with the x scale (a geometry effect can turn the art). A
/// scale that `free` marks false stays as it is. The search stops when both sizes are within 1e-9
/// of the sizes wanted (of the largest of 1 and those sizes), when no scale moves, when `measure`
/// finds no box, or after 16 steps, and returns the scales that came nearest. `None` when `measure`
/// finds no box for `start`.
fn refit_drawn(
    measure: impl Fn(f64, f64) -> Option<Rect>,
    refi: usize,
    size: [f64; 2],
    start: (f64, f64),
    free: [bool; 2],
) -> Option<(f64, f64, Point)> {
    let [w, h] = size;
    let tolerance = 1e-9 * w.max(h).max(1.0);
    let miss = |b: Rect| (b.width() - w).abs() + (b.height() - h).abs();
    let (mut sx, mut sy) = start;
    let mut b = measure(sx, sy)?;
    let mut best = (sx, sy, b);
    for _ in 0..16 {
        let f = [b.width() - w, b.height() - h];
        if f[0].abs() <= tolerance && f[1].abs() <= tolerance {
            break;
        }
        // How the width and the height change with the x scale (`a`) and the y scale (`c`).
        let change = |x: bool| -> Option<[f64; 2]> {
            let (on, s) = if x { (free[0], sx) } else { (free[1], sy) };
            if !on {
                return Some([0.0, 0.0]);
            }
            let ds = (s.abs() * 1e-4).max(1e-6);
            let m = if x { measure(sx + ds, sy) } else { measure(sx, sy + ds) }?;
            Some([(m.width() - b.width()) / ds, (m.height() - b.height()) / ds])
        };
        let (Some(a), Some(c)) = (change(true), change(false)) else { break };
        let dot = |u: [f64; 2], v: [f64; 2]| u[0] * v[0] + u[1] * v[1];
        let (aa, ac, cc, af, cf) = (dot(a, a), dot(a, c), dot(c, c), dot(a, f), dot(c, f));
        let det = aa * cc - ac * ac;
        let (dx, dy) = if det > 1e-12 * aa * cc && det > 0.0 {
            ((ac * cf - cc * af) / det, (ac * af - aa * cf) / det)
        } else if aa >= cc && aa > 0.0 {
            (-af / aa, 0.0)
        } else if cc > 0.0 {
            (0.0, -cf / cc)
        } else {
            break;
        };
        let (nx, ny) = ((sx + dx).max(0.0), (sy + dy).max(0.0));
        if !(nx.is_finite() && ny.is_finite()) || (nx == sx && ny == sy) {
            break;
        }
        let Some(next) = measure(nx, ny) else { break };
        (sx, sy, b) = (nx, ny, next);
        if miss(b) < miss(best.2) {
            best = (sx, sy, b);
        }
    }
    let (sx, sy, b) = best;
    Some((sx, sy, vectorcraft_geom::reference_point(b, refi)))
}

/// The `width` and `height` `object.setBounds` asks for a box now `b`: the one not given stays, or
/// with `proportional` follows the one given.
fn wanted_size(p: &Value, b: Rect) -> (f64, f64) {
    let mut w = f64_or(p, "width", b.width());
    let mut h = f64_or(p, "height", b.height());
    if bool_or(p, "proportional", false) {
        if p.get("width").is_some() && b.width() > 0.0 {
            h = b.height() * w / b.width();
        } else if p.get("height").is_some() && b.height() > 0.0 {
            w = b.width() * h / b.height();
        }
    }
    (w, h)
}

/// `object.setBounds` with anchors selected with Direct Selection (#1824): the values are those of
/// `b`, the box around the anchors (and the objects selected whole) square to the page, and only
/// the anchors, their handles with them, and those objects move or scale.
fn set_anchor_bounds(s: &mut Session, p: &Value, ids: Vec<NodeId>, b: Rect) -> Result<Value> {
    let refi = p.get("reference").and_then(Value::as_u64).unwrap_or(4).min(8) as usize;
    let rp = vectorcraft_geom::reference_point(b, refi);
    let (w, h) = wanted_size(p, b);
    // A single anchor, or anchors in a line, have no size to scale on that side.
    let ratio = |size: f64, now: f64| if now > 1e-9 { size / now } else { 1.0 };
    let scale = about(rp, Affine::scale_non_uniform(ratio(w, b.width()), ratio(h, b.height())));
    let to = Point::new(f64_or(p, "x", rp.x), f64_or(p, "y", rp.y));
    apply_transform(s, "Transform", ids, Affine::translate(to - rp) * scale, p, true)
}

/// The visual box of `ids` square to `frame`, as [`Document::oriented_bounds`] measures it, with
/// symbol instances measured without the strokes in their art
/// ([`vectorcraft_doc::SymbolBoxes::without_art_strokes`]).
fn scaling_box(d: &Document, ids: &[NodeId], frame: &OrientedBox) -> Option<Rect> {
    let symbols = d.symbol_boxes().without_art_strokes();
    let to_local = frame.to_doc().inverse();
    ids.iter().filter_map(|id| d.node(*id)).fold(None, |acc, n| {
        let b = if frame.angle == 0.0 { n.visual_bounds_with(&symbols) } else { n.bounds_in(&symbols, to_local, true) };
        vectorcraft_geom::union_opt(acc, b)
    })
}
