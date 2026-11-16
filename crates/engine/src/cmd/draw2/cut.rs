use super::common::{
    PenRecord, SIZE_VARIATIONS, SIZES, Sample, center_strokes, dist_to_polyline, fill_targets, ids_json, path_mut, path_of, points_param,
    poly_bounds, pressure_region, ranged, replace_with_pieces, samples_param, sibling_with, size_pressure, target_paths, usize_opt,
};
use super::{
    Anchor, AnchorKind, Beside, BezPath, BoolOp, Cap, EngineError, FillRule, Join, NodeId, Paint, ParamCurve, PathData, Point, Rect, Result, Session,
    SubPath, Value, Vec2, bad, eraser_area, eraser_tip, f64_or, fill_contains, json, number_or, object_param, po,
};
use kurbo::{CubicBez, Line, PathSeg};
use vectorcraft_brush::{Brush, BrushKind, Calligraphic, Variation};
use vectorcraft_doc::StrokeLayer;
use vectorcraft_tools::draw2::MIN_BRUSH_SIZE;

/// Split a filled path into connected pieces (each outer contour with its holes).
pub(in super::super) fn components(pd: &PathData) -> Vec<PathData> {
    let subs: Vec<&SubPath> = pd.subpaths.iter().filter(|s| s.closed && s.anchors.len() >= 2).collect();
    if subs.len() <= 1 {
        return if pd.is_empty() { vec![] } else { vec![pd.clone()] };
    }
    let bps: Vec<BezPath> = subs
        .iter()
        .map(|s| {
            let mut b = BezPath::new();
            s.to_bezpath_into(&mut b);
            b
        })
        .collect();
    let samples: Vec<Point> = subs.iter().map(|s| s.segment(0).eval(0.5)).collect();
    let n = subs.len();
    let inside = |j: usize, i: usize| fill_contains(&bps[j], FillRule::NonZero, samples[i]);
    let depth: Vec<usize> = (0..n).map(|i| (0..n).filter(|&j| j != i && inside(j, i)).count()).collect();
    let areas: Vec<f64> = subs.iter().map(|s| s.area().abs()).collect();
    let outers: Vec<usize> = (0..n).filter(|&i| depth[i].is_multiple_of(2)).collect();
    let mut groups: Vec<Vec<SubPath>> = outers.iter().map(|&o| vec![subs[o].clone()]).collect();
    for h in (0..n).filter(|&i| !depth[i].is_multiple_of(2)) {
        let best = outers
            .iter()
            .enumerate()
            .filter(|(_, o)| depth[**o] + 1 == depth[h] && inside(**o, h))
            .min_by(|a, b| areas[*a.1].total_cmp(&areas[*b.1]));
        if let Some((k, _)) = best {
            groups[k].push(subs[h].clone());
        }
    }
    groups.into_iter().map(PathData::new).collect()
}

/// The pieces of `sp` where `remove(point)` is false. `None` when nothing is removed. Each segment
/// is sampled at least every `step` pt (and at least 24 times), so a removed stretch at least
/// `step` long is found, and also at the parameters `extra` gives for it.
fn cut_subpath(sp: &SubPath, step: f64, remove: &dyn Fn(Point) -> bool, extra: &dyn Fn(CubicBez) -> Vec<f64>) -> Option<Vec<SubPath>> {
    let nseg = sp.segment_count();
    if nseg == 0 {
        return sp.anchors.first().filter(|a| remove(a.p)).map(|_| vec![]);
    }
    const K: usize = 24;
    const MAX_SAMPLES: usize = 4096;
    let mut runs: Vec<Vec<(usize, f64, f64)>> = vec![];
    let mut cur: Vec<(usize, f64, f64)> = vec![];
    let mut any = false;
    for i in 0..nseg {
        let c = sp.segment(i);
        // The curve moves at most 3 times its longest control-polygon edge per unit of t, so with
        // n samples, neighboring samples are at most 3 · edge / n apart.
        let edge = (c.p1 - c.p0).hypot().max((c.p2 - c.p1).hypot()).max((c.p3 - c.p2).hypot());
        let wanted = (3.0 * edge / step).ceil();
        let samples = if wanted.is_finite() && wanted > K as f64 { (wanted as usize).min(MAX_SAMPLES) } else { K };
        let mut ts: Vec<f64> = (1..=samples).map(|k| k as f64 / samples as f64).collect();
        let more = extra(c);
        if !more.is_empty() {
            ts.extend(more.into_iter().filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0));
            ts.sort_by(f64::total_cmp);
            ts.dedup();
        }
        let mut prev_t = 0.0;
        let mut prev_rm = remove(c.eval(0.0));
        let mut start = if prev_rm { None } else { Some(0.0) };
        if prev_rm {
            any = true;
            if !cur.is_empty() {
                runs.push(std::mem::take(&mut cur));
            }
        }
        for t in ts {
            let rm = remove(c.eval(t));
            if rm != prev_rm {
                let (mut lo, mut hi) = (prev_t, t);
                for _ in 0..30 {
                    let m = 0.5 * (lo + hi);
                    if remove(c.eval(m)) == prev_rm { lo = m } else { hi = m }
                }
                let b = 0.5 * (lo + hi);
                if prev_rm {
                    start = Some(b);
                } else {
                    any = true;
                    if let Some(s) = start.take() {
                        cur.push((i, s, b));
                    }
                    runs.push(std::mem::take(&mut cur));
                }
            }
            prev_t = t;
            prev_rm = rm;
        }
        if let Some(s) = start {
            cur.push((i, s, 1.0));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs.retain(|r| !r.is_empty());
    if !any {
        return None;
    }
    // A closed path's last and first runs meet at anchor 0.
    if sp.closed && runs.len() >= 2 {
        let first_starts = runs.first().and_then(|r| r.first()).is_some_and(|s| s.0 == 0 && s.1 == 0.0);
        let last_ends = runs.last().and_then(|r| r.last()).is_some_and(|s| s.0 + 1 == nseg && s.2 == 1.0);
        if first_starts && last_ends {
            let first = runs.remove(0);
            if let Some(last) = runs.last_mut() {
                last.extend(first);
            }
        }
    }
    let out = runs
        .iter()
        .filter(|r| r.len() > 1 || r[0].2 - r[0].1 > 1e-4)
        .map(|r| {
            let mut anchors: Vec<Anchor> = vec![];
            for &(i, t0, t1) in r {
                let c = sp.segment(i).subsegment(t0..t1);
                let (h1, h2) = if sp.segment_is_line(i) { (c.p0, c.p3) } else { (c.p1, c.p2) };
                match anchors.last_mut() {
                    Some(a) => a.h_out = h1,
                    None => anchors.push(Anchor { h_out: h1, ..Anchor::corner(c.p0) }),
                }
                anchors.push(Anchor { p: c.p3, h_in: h2, h_out: c.p3, kind: AnchorKind::Corner });
            }
            for a in &mut anchors {
                *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
            }
            SubPath::new(anchors, false)
        })
        .filter(|s| s.anchors.len() >= 2)
        .collect();
    Some(out)
}

/// Erase from an outline path (open or closed) where `remove` holds, sampling each segment at least
/// every `step` pt (`f64::INFINITY` for the fixed sampling). Returns the new geometry per output
/// object (one object per piece when the path had a single subpath), or None if unchanged.
pub(in super::super) fn erase_outline(pd: &PathData, step: f64, remove: &dyn Fn(Point) -> bool) -> Option<Vec<PathData>> {
    erase_outline_at(pd, step, remove, &|_| vec![])
}

/// [`erase_outline`], also sampling each segment at the parameters `extra` gives for it.
fn erase_outline_at(pd: &PathData, step: f64, remove: &dyn Fn(Point) -> bool, extra: &dyn Fn(CubicBez) -> Vec<f64>) -> Option<Vec<PathData>> {
    let mut changed = false;
    let mut subs = vec![];
    for sp in &pd.subpaths {
        match cut_subpath(sp, step, remove, extra) {
            Some(pieces) => {
                changed = true;
                subs.extend(pieces);
            }
            None => subs.push(sp.clone()),
        }
    }
    if !changed {
        return None;
    }
    Some(if pd.subpaths.len() == 1 {
        subs.into_iter().map(PathData::single).collect()
    } else if subs.is_empty() {
        vec![]
    } else {
        vec![PathData::new(subs)]
    })
}

// ---------- Scissors ----------

pub(super) fn split(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.split";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let anchor = usize_opt(p, "anchor", C)?;
    let seg = usize_opt(p, "segment", C)?;
    if anchor.is_none() && seg.is_none() {
        return Err(bad(C, "need `anchor` or `segment` + `t`"));
    }
    let t = f64_or(p, "t", 0.5);
    if !t.is_finite() || p.get("t").is_some_and(|v| !v.is_f64() && !v.is_i64() && !v.is_u64()) {
        return Err(bad(C, "invalid t"));
    }
    if !(0.0..=1.0).contains(&t) {
        return Err(bad(C, "`t` must be from 0 to 1"));
    }
    let ids = s.edit("Split Path", |d, sel| {
        let path = path_mut(d, id)?;
        let mut sp = path.subpaths.get(si).cloned().ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        let n = sp.anchors.len();
        let k = match (anchor, seg) {
            (Some(a), _) if a < n => a,
            (Some(_), _) => return Err(EngineError::Other("no such anchor".into())),
            (None, Some(g)) if g < sp.segment_count() => {
                if t <= 1e-6 {
                    g
                } else if t >= 1.0 - 1e-6 {
                    (g + 1) % n
                } else {
                    sp.insert_anchor(g, t)
                }
            }
            _ => return Err(EngineError::Other("no such segment".into())),
        };
        let mut pieces = sp.cut_at(&std::collections::BTreeSet::from([k]));
        // The cut leaves one of its two coincident end points selected, as `path.cutAtAnchors`
        // does, so a Direct Selection drag pulls the path apart there.
        let pick = |sel: &mut vectorcraft_doc::Selection, id: NodeId, si: usize| {
            sel.clear();
            sel.add(id);
            sel.anchors.entry(id).or_default().insert((si, 0));
        };
        // The paths a cut makes keep the source's appearance with every stroke centered.
        if sp.closed {
            path.subpaths[si] = pieces.pop().ok_or_else(|| EngineError::Other("no such anchor".into()))?;
            center_strokes(d, id);
            pick(sel, id, si);
            return Ok(vec![id]);
        }
        let [left, right] = <[SubPath; 2]>::try_from(pieces).map_err(|_| EngineError::Other("cannot split an open path at its end point".into()))?;
        if path.subpaths.len() == 1 {
            path.subpaths[0] = left;
            center_strokes(d, id);
            let node = sibling_with(d, id, PathData::single(right))?;
            let (parent, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
            let nid = d.insert(parent, idx + 1, node)?;
            pick(sel, nid, 0);
            Ok(vec![id, nid])
        } else {
            path.subpaths[si] = left;
            path.subpaths.insert(si + 1, right);
            center_strokes(d, id);
            pick(sel, id, si + 1);
            Ok(vec![id])
        }
    })?;
    Ok(ids_json(&ids))
}

// ---------- Knife / Eraser ----------

/// Width of the slit the knife leaves between pieces.
const KNIFE_GAP: f64 = 0.05;

pub(super) fn knife(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.knife";
    let pts = points_param(p, "points", C)?;
    if pts.len() < 2 {
        return Err(bad(C, "need at least two points"));
    }
    let st = s.doc()?;
    let strip = po::outline_stroke(&PathData::single(SubPath::polyline(&pts, false)), KNIFE_GAP, Cap::Butt, Join::Round, 4.0);
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for (id, pd, rule) in fill_targets(&st.doc, &st.selection, poly_bounds(&pts, 1.0))? {
        if !pd.is_closed() {
            continue;
        }
        let pieces = components(&po::boolean(&pd, rule, &strip, FillRule::NonZero, BoolOp::Difference));
        if pieces.len() >= 2 {
            plan.push((id, pieces));
        }
    }
    if plan.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    let ids = s.edit("Knife", |d, sel| {
        let mut all = vec![];
        let mut cut = Beside::replacing(d, &plan.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, pieces) in plan {
            all.extend(replace_with_pieces(d, &mut cut, id, pieces)?);
        }
        cut.finish(d)?;
        sel.set(all.iter().copied());
        Ok(all)
    })?;
    Ok(ids_json(&ids))
}

pub(super) fn erase_region(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.eraseRegion";
    if let Some(rect) = p.get("rect").filter(|v| !v.is_null()) {
        if p.get("points").is_some_and(|v| !v.is_null()) {
            return Err(bad(C, "give `points` or `rect`, not both"));
        }
        return erase_rect(s, marquee_param(rect, C)?);
    }
    let samples = samples_param(p, "points", C)?;
    let size = ranged(p, "size", 10.0, SIZES, C)?;
    let angle = number_or(p, "angle", 0.0, C)?;
    if !(-180.0..=180.0).contains(&angle) {
        return Err(bad(C, "`angle` must be from -180 to 180 degrees"));
    }
    let roundness = number_or(p, "roundness", 100.0, C)?;
    if !(1.0..=100.0).contains(&roundness) {
        return Err(bad(C, "`roundness` must be from 1 to 100%"));
    }
    let vary = eraser_variations(p, C)?;
    if varies_beyond_size_pressure(&vary) {
        return erase_varied(s, &samples, [angle, roundness, size], vary);
    }
    let pressure = if vary[2].0 == Variation::Pressure { size_pressure(p, &samples, C)? } else { None };
    let pts: Vec<Point> = samples.into_iter().map(|s| s.pt).collect();
    let r = size / 2.0;
    // The tip's short axis (its narrowest with Pressure): an open path is sampled at least every half of it.
    let narrowest = pressure.as_ref().map_or(size, |(_, v)| vectorcraft_tools::draw2::pressure_size(size, *v, 0.0));
    let short = narrowest * roundness / 100.0;
    let st = s.doc()?;
    // A point is under the tip when, with the tip squashed back into a circle, it is within the
    // circle's radius of the squashed drag.
    let stretch = eraser_tip(angle, roundness);
    let squash = stretch.inverse();
    let round: Vec<Point> = pts.iter().map(|q| squash * *q).collect();
    // With Size set to Pressure the tip is as wide as the pen pressed: the area is the squashed
    // drag outlined at those widths, stretched back, and a point is under it when the area holds it.
    let pressed = match (&pressure, round.len() > 1) {
        (Some((prof, v)), true) => Some(pressure_region(&PathData::single(SubPath::polyline(&round, false)), size, *v, prof)?.transformed(stretch)),
        (Some((prof, v)), false) => Some(eraser_area(&pts, vectorcraft_tools::draw2::pressure_size(size, *v, prof.at(0.0)), angle, roundness)),
        (None, _) => None,
    };
    let reach = pressure.as_ref().map_or(r, |(_, v)| (size + v) / 2.0);
    let region = pressed.clone().unwrap_or_else(|| eraser_area(&pts, size, angle, roundness));
    let outline = pressed.map(|pd| pd.to_bezpath());
    let under = |q: Point| match &outline {
        Some(bp) => fill_contains(bp, FillRule::NonZero, q),
        None => dist_to_polyline(&round, squash * q) <= r,
    };
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for (id, pd, rule) in fill_targets(&st.doc, &st.selection, poly_bounds(&pts, reach))? {
        if pd.is_closed() {
            if po::area(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Intersect), FillRule::NonZero) <= 1e-6 {
                continue;
            }
            plan.push((id, components(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Difference))));
        } else if let Some(pieces) = erase_outline(&pd, short / 2.0, &under) {
            plan.push((id, pieces));
        }
    }
    apply_plan(s, "Eraser", plan)
}

/// `path.eraseRegion`'s `rect: [x, y, width, height]`: four finite numbers, with a width and a
/// height above 0.
fn marquee_param(v: &Value, cmd: &str) -> Result<Rect> {
    const SHAPE: &str = "rect is [x, y, width, height] in points, with a width and a height above 0";
    let n: Vec<f64> = match v.as_array() {
        Some(a) if a.len() == 4 => a.iter().map(Value::as_f64).collect::<Option<_>>().ok_or_else(|| bad(cmd, SHAPE))?,
        _ => return Err(bad(cmd, SHAPE)),
    };
    let [x, y, w, h] = [n[0], n[1], n[2], n[3]];
    if !(n.iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0 && (x + w).is_finite() && (y + h).is_finite()) {
        return Err(bad(cmd, SHAPE));
    }
    Ok(Rect::new(x, y, x + w, y + h))
}

/// `path.eraseRegion` with a marquee: everything inside `r` is erased from the paths (selected, or
/// all). Closed paths lose the rectangle's area, and open paths are cut where they cross its edges.
fn erase_rect(s: &mut Session, r: Rect) -> Result<Value> {
    let region =
        PathData::single(SubPath::polyline(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], true));
    let inside = |q: Point| q.x >= r.x0 && q.x <= r.x1 && q.y >= r.y0 && q.y <= r.y1;
    // An open path is sampled at least every half of the rectangle's shorter side, and also where
    // each segment crosses the rectangle's edges and halfway between those crossings, so a stretch
    // inside the rectangle is found however long the segment is.
    let step = r.width().min(r.height()) / 2.0;
    let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
    let crossings = |c: CubicBez| {
        let mut ts: Vec<f64> = (0..4)
            .flat_map(|k| PathSeg::Cubic(c).intersect_line(Line::new(corners[k], corners[(k + 1) % 4])))
            .map(|x| x.segment_t)
            .filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0)
            .collect();
        ts.sort_by(f64::total_cmp);
        let mids: Vec<f64> = [0.0].iter().chain(&ts).zip(ts.iter().chain(&[1.0])).map(|(a, b)| 0.5 * (a + b)).collect();
        ts.extend(mids);
        ts
    };
    let st = s.doc()?;
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for (id, pd, rule) in fill_targets(&st.doc, &st.selection, r)? {
        if pd.is_closed() {
            if po::area(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Intersect), FillRule::NonZero) <= 1e-6 {
                continue;
            }
            plan.push((id, components(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Difference))));
        } else if let Some(pieces) = erase_outline_at(&pd, step, &inside, &crossings) {
            plan.push((id, pieces));
        }
    }
    apply_plan(s, "Eraser", plan)
}

/// The variation modes `path.eraseRegion` takes for each of `angleMode`, `roundnessMode` and
/// `sizeMode`.
const ERASER_MODES: &str = "\"fixed\", \"random\", \"pressure\", \"tilt\", \"bearing\" or \"rotation\"";

/// Eraser Tool Options' Angle, Roundness and Size variations: for each of angle (`angleMode` with
/// `angleVariation` in degrees, at most 180), roundness (`roundnessMode` with `roundnessVariation`
/// in percent, at most 100) and size (`sizeMode` with `sizeVariation` in points, at most 1000), its
/// mode and how far it varies either way. A mode left out or null is Fixed, and a variation left
/// out or null is 0; any other mode, or a variation that is negative or past its most, is refused,
/// in every mode.
fn eraser_variations(p: &Value, cmd: &str) -> Result<[(Variation, f64); 3]> {
    const KEYS: [(&str, &str, f64); 3] =
        [("angleMode", "angleVariation", 180.0), ("roundnessMode", "roundnessVariation", 100.0), ("sizeMode", "sizeVariation", SIZE_VARIATIONS.1)];
    let mut out = [(Variation::Fixed, 0.0); 3];
    for (slot, (mode_key, var_key, most)) in out.iter_mut().zip(KEYS) {
        let mode = match p.get(mode_key).filter(|v| !v.is_null()) {
            None => Variation::Fixed,
            Some(Value::String(id)) => Variation::parse(id).ok_or_else(|| bad(cmd, format!("{mode_key} is one of {ERASER_MODES}, not `{id}`")))?,
            Some(other) => return Err(bad(cmd, format!("{mode_key} is one of {ERASER_MODES}, not {other}"))),
        };
        let variation = number_or(p, var_key, 0.0, cmd)?;
        if !(variation.is_finite() && variation >= 0.0) {
            return Err(bad(cmd, format!("{var_key} must be 0 or more")));
        }
        if variation > most {
            // Size Variation is refused as path.blob refuses it.
            return Err(match var_key {
                "sizeVariation" => bad(cmd, format!("`{var_key}` must be from {} to {most}", SIZE_VARIATIONS.0)),
                _ => bad(cmd, format!("{var_key} must be at most {most}")),
            });
        }
        *slot = (mode, variation);
    }
    Ok(out)
}

/// Does any of the Eraser's Angle, Roundness and Size vary with something other than Size on
/// Pressure? A mode whose variation is 0 does not vary.
fn varies_beyond_size_pressure(vary: &[(Variation, f64); 3]) -> bool {
    vary.iter().enumerate().any(|(i, (mode, v))| *v > 0.0 && *mode != Variation::Fixed && !(i == 2 && *mode == Variation::Pressure))
}

/// `path.eraseRegion` with its tip varying along the drag ([`eraser_variations`]): the area the
/// tip covers is swept as a Calligraphic brush nib of the same angle, roundness, size, modes and
/// variations, with what the pen recorded along the drag. Random draws one value for the drag
/// and the pen modes follow the pen at each point; a drag without that recording (a mouse) takes
/// the value itself. `tip`: the angle, roundness and size.
fn erase_varied(s: &mut Session, samples: &[Sample], tip: [f64; 3], vary: [(Variation, f64); 3]) -> Result<Value> {
    let [angle, roundness, size] = tip;
    let mut pts: Vec<Point> = samples.iter().map(|s| s.pt).collect();
    // A press without a drag is a dab: a sweep needs some length.
    if let [only] = pts[..] {
        pts.push(only + Vec2::new(0.01, 0.0));
    }
    // A nib's size is kept within 1296 pt and the stroke's width scales it: a larger tip is swept
    // at a fraction of its size on a stroke as many times wider.
    let [(am, av), (rm, rv), (sm, sv)] = vary;
    let sv = if sm == Variation::Fixed { 0.0 } else { sv };
    let scale = ((size + sv) / Calligraphic::RANGES[2].1).max(1.0);
    let nib = Calligraphic {
        angle,
        roundness: roundness.clamp(1.0, 100.0),
        size: size / scale,
        variation: [av, rv, sv / scale],
        modes: Some([am, rm, sm]),
    };
    let brush = Brush { name: "Eraser".into(), kind: BrushKind::Calligraphic(nib) };
    let mut stroke = StrokeLayer::new(Paint::None, scale);
    PenRecord::of_samples(samples).write(&mut stroke);
    let spine = SubPath::polyline(&pts, false);
    let st = s.doc()?;
    let pieces = vectorcraft_brush::stroke_pieces(&st.doc, &brush, &PathData::single(spine).to_bezpath(), &stroke);
    let areas: Vec<PathData> = pieces.iter().filter_map(|n| n.path_data().cloned()).collect();
    if areas.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    let region =
        po::try_unite_all(&areas.iter().map(|a| (a, FillRule::NonZero)).collect::<Vec<_>>()).map_err(|e| EngineError::Other(e.to_string()))?;
    let bp = region.to_bezpath();
    // The narrowest the tip gets across: an open path is sampled at least every half of it.
    let low = |i: usize| match vary[i] {
        (Variation::Fixed, _) => tip[i],
        (_, v) => tip[i] - v,
    };
    let short = low(2).max(MIN_BRUSH_SIZE) * low(1).clamp(1.0, 100.0) / 100.0;
    let reach = (size + sv) / 2.0;
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for (id, pd, rule) in fill_targets(&st.doc, &st.selection, poly_bounds(&pts, reach))? {
        if pd.is_closed() {
            if po::area(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Intersect), FillRule::NonZero) <= 1e-6 {
                continue;
            }
            plan.push((id, components(&po::boolean(&pd, rule, &region, FillRule::NonZero, BoolOp::Difference))));
        } else if let Some(pieces) = erase_outline(&pd, short / 2.0, &|q| fill_contains(&bp, FillRule::NonZero, q)) {
            plan.push((id, pieces));
        }
    }
    apply_plan(s, "Eraser", plan)
}

fn apply_plan(s: &mut Session, label: &str, plan: Vec<(NodeId, Vec<PathData>)>) -> Result<Value> {
    if plan.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    let ids = s.edit(label, |d, sel| {
        let mut all = vec![];
        let mut cut = Beside::replacing(d, &plan.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, pieces) in plan {
            all.extend(replace_with_pieces(d, &mut cut, id, pieces)?);
        }
        cut.finish(d)?;
        sel.set(all.iter().copied());
        Ok(all)
    })?;
    Ok(ids_json(&ids))
}

pub(super) fn erase_segments(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.eraseSegments";
    let pts = points_param(p, "points", C)?;
    let w = number_or(p, "width", 8.0, C)?;
    if !(w.is_finite() && w > 0.0) {
        return Err(bad(C, "width must be positive"));
    }
    let st = s.doc()?;
    let mut plan = vec![];
    for id in target_paths(&st.doc, &st.selection, poly_bounds(&pts, w)) {
        let (pd, _) = path_of(&st.doc, id)?;
        if let Some(pieces) = erase_outline(&pd, w / 2.0, &|q| dist_to_polyline(&pts, q) <= w / 2.0) {
            plan.push((id, pieces));
        }
    }
    apply_plan(s, "Path Eraser", plan)
}
