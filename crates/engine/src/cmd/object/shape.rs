use super::spans::Span;
use super::*;

/// The largest scale of 0 or more at which `spans`, with the margins that scale grown by `k`,
/// reach across `size` together. Returns 1 when their geometric boxes reach 1e-9 or less across
/// together, as the closed form in [`set_bounds`] does, and 0 when the spans reach farther than
/// `size` at the scale 0.
pub(super) fn scale_across(spans: &[Span], k: f64, size: f64) -> f64 {
    let o = spans.iter().map(|s| s.lo).fold(f64::INFINITY, f64::min);
    let across = spans.iter().map(|s| s.hi).fold(f64::NEG_INFINITY, f64::max) - o;
    // How far the spans reach together after a scale by `s` about `o`, and how fast that changes
    // with `s`: the far geometric end minus the near one, of the objects that reach farthest at the
    // two ends.
    let reach = |s: f64| {
        let (mut start, mut end, mut slope_start, mut slope_end) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0.0);
        for sp in spans {
            let (a, b) = sp.scaled(o, s, k);
            if a < start {
                (start, slope_start) = (a, sp.lo - o);
            }
            if b > end {
                (end, slope_end) = (b, sp.hi - o);
            }
        }
        (end - start, slope_end - slope_start)
    };
    if across.is_nan() || across <= 1e-9 {
        return 1.0;
    }
    let least = reach(0.0).0;
    if least.is_nan() || least > size {
        return 0.0;
    }
    // `reach` is convex in `s` and at most `size` at 0, so it equals `size` at one largest scale.
    // Newton's method, started at a larger scale, reaches it one linear piece of `reach` at a
    // time; bisection finishes the search when a step makes no progress.
    let (mut lo, mut hi) = (0.0, size / across);
    for _ in 0..64 {
        if reach(hi).0 >= size {
            break;
        }
        hi = 2.0 * hi + 1.0;
    }
    for _ in 0..64 {
        let (r, slope) = reach(hi);
        if r <= size {
            return hi;
        }
        let next = hi - (r - size) / slope;
        if next.is_nan() || next <= lo || next >= hi {
            break;
        }
        hi = next;
    }
    for _ in 0..64 {
        let mid = lo + (hi - lo) / 2.0;
        if mid <= lo || mid >= hi {
            break;
        }
        if reach(mid).0 <= size {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    if size - reach(lo).0 <= reach(hi).0 - size { lo } else { hi }
}

pub(super) fn expand_shape(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no live shape selected, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { live: Some(_), .. }))) {
        return ok();
    }
    s.edit("Expand Shape", |d, _| {
        for id in &ids {
            if let Some(NodeKind::Path { live, .. }) = d.node_mut(*id).map(|n| &mut n.kind) {
                *live = None;
            }
        }
        Ok(())
    })?;
    ok()
}

/// The `corners` of `object.setLiveShape`: anchor indices of a path's uncut outline.
fn corners_param(p: &Value) -> Result<Option<BTreeSet<usize>>> {
    let Some(v) = p.get("corners").filter(|v| !v.is_null()) else { return Ok(None) };
    let err = || bad("object.setLiveShape", format!("`corners` must list anchor indices (0, 1, 2…), not {v}"));
    let list = v.as_array().ok_or_else(err)?;
    list.iter().map(|k| k.as_u64().and_then(|k| usize::try_from(k).ok()).ok_or_else(err)).collect::<Result<_>>().map(Some)
}

pub(super) fn set_live_shape(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &["invertPie", "makeSidesEqual", "relativeRadii"], "object.setLiveShape")?;
    check_numbers(p, &["radius", "sides"], "object.setLiveShape")?;
    // `items`: objects each with its own `corners`; else `ids` (or the selection) sharing them.
    let items = match p.get("items").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|it| {
                    let id = it
                        .get("id")
                        .and_then(Value::as_u64)
                        .map(NodeId)
                        .ok_or_else(|| bad("object.setLiveShape", format!("each of `items` is {{id, corners?}} with an object id, not {it}")))?;
                    Ok((id, corners_param(it)?))
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        Some(v) => return Err(bad("object.setLiveShape", format!("`items` must be an array of {{id, corners?}}, not {v}"))),
    };
    let ids = match &items {
        Some(items) => items.iter().map(|(id, _)| *id).collect(),
        None => targets(s, p)?,
    };
    let shared = corners_param(p)?;
    let corners_of = |id: NodeId| match &items {
        Some(items) => items.iter().find(|(i, _)| *i == id).and_then(|(_, c)| c.clone()),
        None => shared.clone(),
    };
    let kind = p
        .get("kind")
        .filter(|v| !v.is_null())
        .map(|v| {
            serde_json::from_value::<CornerKind>(v.clone())
                .map_err(|_| bad("object.setLiveShape", format!("`kind` must be \"round\", \"invertedRound\" or \"chamfer\", not {v}")))
        })
        .transpose()?;
    let radius = p.get("radius").and_then(Value::as_f64);
    let relative_radii = p.get("relativeRadii").and_then(Value::as_bool);
    let sides = p.get("sides").and_then(Value::as_u64);
    // A number that must be `what` (None when absent).
    let number = |k: &str, ok: fn(f64) -> bool, what: &str| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|x| x.is_finite() && ok(*x))
                .map(Some)
                .ok_or_else(|| bad("object.setLiveShape", format!("`{k}` must be {what}, not {v}"))),
        }
    };
    let angle = |k: &str| Ok::<_, EngineError>(number(k, |_| true, "an angle in degrees")?.map(|a| a.rem_euclid(360.0)));
    let length = |k: &str| number(k, |x| x > 0.0 && x <= crate::MAX_COORD, "a length in points above 0");
    // An ellipse's pie (Ellipse Properties: Pie Start and End Angle, degrees; Invert Pie swaps them).
    let (pie_start, pie_end) = (angle("pieStart")?, angle("pieEnd")?);
    let invert = p.get("invertPie").and_then(Value::as_bool) == Some(true);
    // A polygon's Polygon Properties: its angle, radius or side length, Make Sides Equal.
    let (polygon_angle, polygon_radius, side) = (angle("polygonAngle")?, length("polygonRadius")?, length("sideLength")?);
    let equal = p.get("makeSidesEqual").and_then(Value::as_bool) == Some(true);
    // An ellipse's pie becomes `pie_of(pie)`. An end of 0 is a full turn (Illustrator shows a whole
    // ellipse as 0° to 360°).
    let end_of = |a: f64| if a == 0.0 { 360.0 } else { a };
    let pie_of = |pie: (f64, f64)| {
        let pie = (pie_start.unwrap_or(pie.0), pie_end.map_or(pie.1, end_of));
        if invert { (pie.1.rem_euclid(360.0), end_of(pie.0)) } else { pie }
    };
    // When every value given is the current one, the document and its undo history stay as they
    // are.
    let st = s.doc()?;
    let has_values = |id: &NodeId| {
        let Some(NodeKind::Path { path, live, .. }) = st.doc.node(*id).map(|n| &n.kind) else { return true };
        match live.as_deref() {
            Some(l @ LiveShape::Polygon { sides: now, .. })
                if !(sides.is_none_or(|n| n.clamp(3, 1000) == u64::from(*now))
                    && (!equal || l.polygon_sides_equal())
                    && polygon_angle.is_none_or(|a| l.polygon_angle() == Some(a))
                    && side.and_then(|len| l.polygon_radius_for_side(len)).or(polygon_radius).is_none_or(|r| l.polygon_radius() == Some(r))) =>
            {
                return false;
            }
            Some(LiveShape::Ellipse { pie, .. }) if pie_of(*pie) != *pie => return false,
            _ => {}
        }
        if relative_radii.is_some_and(|on| live.as_deref().map(|l| l.radii_relative()) != Some(on)) {
            return false;
        }
        if radius.is_none() && kind.is_none() {
            return true;
        }
        let Some(c) = LiveCorners::new(path, live.as_deref()) else { return true };
        let picked = corners_of(*id).unwrap_or_else(|| c.picked(st.selection.anchors.get(id)));
        // Setting corners folds a scale left in a rectangle's transform into its size, which
        // changes it.
        live.as_deref().is_none_or(|l| l.folded() == *l)
            && picked.iter().all(|k| {
                *k < c.base.anchor_count()
                    && radius.is_none_or(|r| (c.drawn_radius(*k) - r.max(0.0)).abs() < 1e-9)
                    && kind.is_none_or(|kd| c.kind(*k) == kd)
            })
    };
    if ids.iter().all(has_values) {
        return ok();
    }
    s.edit("Live Shape", |d, sel| {
        for id in &ids {
            let Some(NodeKind::Path { path, live, .. }) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            if let Some(l @ LiveShape::Polygon { .. }) = live.as_deref_mut()
                && (sides.is_some() || polygon_angle.is_some() || polygon_radius.is_some() || side.is_some() || equal)
            {
                if let Some(n) = sides {
                    l.set_sides(n);
                }
                if equal {
                    l.make_sides_equal();
                }
                if let Some(a) = polygon_angle {
                    l.set_polygon_angle(a);
                }
                // A side length sets the radius that gives it (with the new side count).
                if let Some(r) = side.and_then(|len| l.polygon_radius_for_side(len)).or(polygon_radius) {
                    l.set_polygon_radius(r);
                }
                *path = l.to_path();
            }
            if let Some(l @ LiveShape::Ellipse { .. }) = live.as_deref_mut()
                && (pie_start.is_some() || pie_end.is_some() || invert)
            {
                if let LiveShape::Ellipse { pie, .. } = l {
                    *pie = pie_of(*pie);
                }
                *path = l.to_path();
            }
            if let Some(on) = relative_radii
                && let Some(l) = live.as_deref_mut()
            {
                convert_radii_relative(l, on);
                *path = l.to_path();
            }
            if radius.is_none() && kind.is_none() {
                continue;
            }
            let partial = sel.anchors.get_mut(id);
            // The corners to edit, the Direct-Selected ones and the anchor layout before the edit.
            let Some(c) = LiveCorners::new(path, live.as_deref()) else { continue };
            let corners = corners_of(*id);
            let picked = match &corners {
                Some(k) if k.last().is_some_and(|k| *k >= c.base.anchor_count()) => {
                    let n = c.base.anchor_count();
                    let past: Vec<_> = k.range(n..).collect();
                    return Err(bad("object.setLiveShape", format!("object {id} has anchors 0–{} for `corners`, not {past:?}", n.saturating_sub(1))));
                }
                Some(k) => k.clone(),
                None => c.picked(partial.as_deref()),
            };
            let (selected, layout) = (partial.as_deref().map(|a| c.corners_of(a)), c.sources().to_vec());
            set_corners(path, live, &picked, radius, kind);
            // Direct-Selected corners stay selected as they gain or lose anchors.
            if let (Some(anchors), Some(selected)) = (partial, selected)
                && let Some(c) = LiveCorners::new(path, live.as_deref())
                && c.sources() != layout
            {
                *anchors = c.anchors_of(&selected);
            }
        }
        Ok(())
    })?;
    ok()
}
