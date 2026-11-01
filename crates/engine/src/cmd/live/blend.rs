use super::common::{
    DEFAULT_DISTANCE, DEFAULT_STEPS, ease_param, fix_ids, ids_json, is_blend, orientation_param, percent_param, roots_param, selected_of,
    spacing_param, starts_param, wrap,
};
use super::mesh::index_param;
use super::{
    Affine, Anchor, Appearance, Arc, Beside, BlendDefaults, BlendOrientation, BlendSpacing, BlendSpec, Document, EngineError, Node, NodeId, NodeKind,
    Paint, PathData, Point, Result, Session, Spine, Value, Vec2, bad, f64_req, id_param, json, live, number_or, ok, str_param, switch_or,
};

/// The first blend among `roots`.
fn first_blend(d: &Document, roots: &[NodeId]) -> Option<Node> {
    roots.iter().filter_map(|r| d.node(*r)).find(|n| is_blend(n)).cloned()
}

/// Key objects of a blend made of `nodes` (paint order) and their start points: a blend among
/// them gives its keys (with theirs), the other objects are keys themselves.
fn merge_keys(nodes: Vec<Arc<Node>>, starts: &[(NodeId, u32)]) -> (Vec<Arc<Node>>, Vec<Option<u32>>) {
    let (mut keys, mut st) = (vec![], vec![]);
    for n in nodes {
        match &n.kind {
            NodeKind::Blend { children, spec } => {
                st.extend((0..children.len()).map(|i| spec.starts.get(i).copied().flatten()));
                keys.extend(children.iter().cloned());
            }
            _ => {
                st.push(starts.iter().find(|(id, _)| *id == n.id).map(|(_, a)| *a));
                keys.push(n);
            }
        }
    }
    if st.iter().all(Option::is_none) {
        st.clear();
    }
    (keys, st)
}

/// The spine of a blend of `keys` made by giving blend `old` (spec `old_spec`) more keys: its
/// spine pinned, with a straight segment to each new key before or after its own (a closed spine
/// gives way to the straight lines between the keys).
fn extend_spine(spec: &mut BlendSpec, old: &[Arc<Node>], old_spec: &BlendSpec, keys: &[Arc<Node>]) {
    if old_spec.spine.is_none() {
        return;
    }
    let Some((mut path, anchors)) = live::pin_spine(old, old_spec) else { return };
    let Some(first) = old.first().and_then(|f| keys.iter().position(|k| k.id == f.id)) else { return };
    let Some(sp) = path.subpaths.first_mut().filter(|sp| !sp.closed) else { return };
    let center = |k: &Arc<Node>| vectorcraft_geom::Anchor::corner(k.geometric_bounds().map(|b| b.center()).unwrap_or_default());
    let before: Vec<_> = keys.iter().take(first).map(center).collect();
    let after: Vec<_> = keys.iter().skip(first + old.len()).map(center).collect();
    let shift = before.len();
    sp.anchors.splice(0..0, before);
    let n = sp.anchors.len();
    sp.anchors.extend(after);
    let mut ka: Vec<usize> = (0..shift).collect();
    ka.extend(anchors.iter().map(|a| a + shift));
    ka.extend(n..sp.anchors.len());
    spec.key_anchors = ka.iter().map(|a| u32::try_from(*a).unwrap_or(u32::MAX)).collect();
    spec.spine = Some(path);
}

pub(super) fn blend_make(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.make";
    let roots = roots_param(s, p)?;
    if roots.len() < 2 {
        return Err(bad(C, "select at least two objects"));
    }
    let starts = starts_param(p, &roots)?;
    let base = first_blend(&s.doc()?.doc, &roots);
    let cur = match base.as_ref().map(|n| &n.kind) {
        Some(NodeKind::Blend { spec, .. }) => BlendDefaults { spacing: spec.spacing, orientation: spec.orientation },
        _ => s.prefs.blend_options.unwrap_or_default(),
    };
    let spacing = spacing_param(p, C, cur.spacing)?;
    let orientation = orientation_param(p, cur.orientation);
    let (ease0, color0) = match base.as_ref().map(|n| &n.kind) {
        Some(NodeKind::Blend { spec, .. }) => (spec.ease, spec.color_shift),
        _ => Default::default(),
    };
    let ease = ease_param(p, C, ease0)?;
    let color_shift = percent_param(p, C, "colorShift", color0, -100.0)?;
    let id = s.edit("Make Blend", |d, sel| {
        wrap(d, sel, &roots, |id, nodes| {
            let (children, starts) = merge_keys(nodes, &starts);
            // A blend taking more keys keeps its name, transparency and spine; a new one is a
            // knockout group, so translucent steps don't show through each other.
            let mut n = base.unwrap_or_else(|| {
                let mut n = Node::new(id, NodeKind::Group { children: vec![], clip: false });
                n.knockout = vectorcraft_doc::Knockout::On;
                n
            });
            let mut spec = BlendSpec { spacing, orientation, ease, color_shift, starts, ..Default::default() };
            if let NodeKind::Blend { children: old, spec: old_spec } = &n.kind {
                extend_spine(&mut spec, old, old_spec, &children);
            }
            n.id = id;
            n.kind = NodeKind::Blend { children, spec: Box::new(spec) };
            n
        })
    })?;
    Ok(json!({ "id": id.0 }))
}

pub(super) fn blend_release(s: &mut Session, _: &Value) -> Result<Value> {
    let blends = selected_of(s, is_blend);
    let (ids, spines, groups) = s.edit("Release Blend", |d, sel| {
        let (mut keys, mut spines, mut groups) = (vec![], vec![], vec![]);
        let mut released = Beside::replacing(d, &blends);
        for b in &blends {
            let Some(n) = d.node(*b).cloned() else { continue };
            let NodeKind::Blend { children, spec } = &n.kind else { continue };
            // The spine stays behind as a path that paints nothing, below the keys.
            let mut nodes = vec![];
            if let Some((path, _)) = live::blend_spine(children, spec) {
                let id = d.alloc_id();
                spines.push(id);
                nodes.push(Node::path(id, path, Appearance::basic(Paint::None, Paint::None, 0.0)));
            }
            keys.extend(children.iter().map(|c| c.id));
            nodes.extend(children.iter().map(|c| (**c).clone()));
            // What the blend itself carries stays on a group around what it gave back.
            if n.name.is_some()
                || n.opacity < 1.0
                || n.blend != vectorcraft_color::BlendMode::Normal
                || n.isolate
                || n.extras().mask.is_some()
                || !n.appearance.items.is_empty()
                || !n.appearance.effects.is_empty()
            {
                let mut g = n.clone();
                g.kind = NodeKind::Group { children: nodes.into_iter().map(Arc::new).collect(), clip: false };
                groups.push(g.id);
                nodes = vec![g];
            }
            released.put(d, *b, nodes)?;
        }
        released.finish(d)?;
        if groups.is_empty() {
            sel.set(spines.iter().chain(&keys).copied());
        } else {
            sel.set(groups.iter().copied());
        }
        Ok((keys, spines, groups))
    })?;
    let mut r = ids_json(&ids);
    r["spines"] = json!(spines.iter().map(|i| i.0).collect::<Vec<_>>());
    r["groups"] = json!(groups.iter().map(|i| i.0).collect::<Vec<_>>());
    Ok(r)
}

/// The blend `id` (default: the first selected), its spine pinned to its keys (see
/// [`live::pin_spine`]) and given to `f` with the key anchors and the keys; the result is stored
/// as the blend's spine, one undo step.
fn edit_spine<T>(
    s: &mut Session,
    p: &Value,
    cmd: &str,
    label: &str,
    f: impl FnOnce(&mut vectorcraft_geom::SubPath, &mut Vec<usize>, &mut [Arc<Node>]) -> Result<T>,
) -> Result<T> {
    let id = match id_param(p, "id") {
        Some(id) => id,
        None => *selected_of(s, is_blend).first().ok_or_else(|| bad(cmd, "select a blend or give `id`"))?,
    };
    let c = cmd.to_string();
    s.edit(label, move |d, _| {
        let Some(NodeKind::Blend { children, spec }) = d.node_mut(id).map(|n| &mut n.kind) else { return Err(bad(&c, "not a blend")) };
        let (mut path, mut anchors) = live::pin_spine(children, spec).ok_or_else(|| bad(&c, "the blend has no spine"))?;
        let sp = path.subpaths.first_mut().ok_or_else(|| bad(&c, "the blend has no spine"))?;
        let out = f(sp, &mut anchors, children)?;
        spec.key_anchors = anchors.iter().map(|a| u32::try_from(*a).unwrap_or(u32::MAX)).collect();
        spec.spine = Some(path);
        Ok(out)
    })
}

/// A point parameter that must be finite.
fn finite_point(p: &Value, cmd: &str) -> Result<Point> {
    let q = Point::new(f64_req(p, "x", cmd)?, f64_req(p, "y", cmd)?);
    if q.x.is_finite() && q.y.is_finite() && q.x.abs() < 1e9 && q.y.abs() < 1e9 { Ok(q) } else { Err(bad(cmd, "x and y must be finite")) }
}

pub(super) fn spine_move_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.spine.moveAnchor";
    let index = index_param(p, "anchor", C)?;
    let q = finite_point(p, C)?;
    let handle = match str_param(p, "handle") {
        None => None,
        Some("out") => Some(true),
        Some("in") => Some(false),
        Some(_) => return Err(bad(C, "handle must be in or out")),
    };
    let independent = switch_or(p, "independent", false, C)?;
    edit_spine(s, p, C, "Reshape Spine", |sp, anchors, keys| {
        let a = sp.anchors.get_mut(index).ok_or_else(|| bad(C, "no such spine point"))?;
        if let Some(out) = handle {
            a.set_handle(out, q, independent);
            return Ok(());
        }
        let delta = q - a.p;
        a.translate(delta);
        // The key objects on the point move with it.
        for (k, _) in anchors.iter().enumerate().filter(|(_, a)| **a == index) {
            if let Some(key) = keys.get_mut(k) {
                Arc::make_mut(key).transform(Affine::translate(delta), false);
            }
        }
        Ok(())
    })?;
    ok()
}

pub(super) fn spine_convert_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.spine.convertAnchor";
    let index = index_param(p, "anchor", C)?;
    let smooth = match str_param(p, "to") {
        Some("smooth") => true,
        Some("corner") => false,
        _ => return Err(bad(C, "to must be smooth or corner")),
    };
    let target = match (p.get("x"), p.get("y")) {
        (None, None) => None,
        (Some(_), Some(_)) => Some(finite_point(p, C)?),
        _ => return Err(bad(C, "x and y must be given together")),
    };
    edit_spine(s, p, C, "Convert Anchor Point", |sp, _, _| {
        if smooth && target.is_none() {
            if index >= sp.anchors.len() {
                return Err(bad(C, "no such spine point"));
            }
            vectorcraft_tools::draw2::curvature_smooth(sp, index);
            return Ok(());
        }
        let a = sp.anchors.get_mut(index).ok_or_else(|| bad(C, "no such spine point"))?;
        match target {
            Some(q) if smooth && q.distance(a.p) > 1e-9 => *a = Anchor::smooth(a.p, q),
            _ => a.retract(),
        }
        Ok(())
    })?;
    ok()
}

pub(super) fn spine_reshape_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.spine.reshapeSegment";
    let seg = index_param(p, "segment", C)?;
    let t = number_or(p, "t", 0.5, C)?.clamp(0.05, 0.95);
    let dv = Vec2::new(number_or(p, "dx", 0.0, C)?, number_or(p, "dy", 0.0, C)?);
    if !(t.is_finite() && dv.x.is_finite() && dv.y.is_finite() && dv.hypot() < 1e9) {
        return Err(bad(C, "t, dx and dy must be finite"));
    }
    let perpendicular = switch_or(p, "perpendicular", false, C)?;
    edit_spine(s, p, C, "Reshape Spine", |sp, _, _| {
        let bent = if perpendicular { sp.bend_segment_perpendicular(seg, t, dv) } else { sp.bend_segment(seg, t, dv) };
        if bent { Ok(()) } else { Err(bad(C, "no such spine segment")) }
    })?;
    ok()
}

pub(super) fn spine_add_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.spine.addAnchor";
    let q = finite_point(p, C)?;
    let index = edit_spine(s, p, C, "Add Spine Point", |sp, anchors, _| {
        let (_, seg, t, _, _) = PathData::single(sp.clone()).nearest(q).ok_or_else(|| bad(C, "the spine has no segments"))?;
        let i = sp.insert_anchor(seg, t.clamp(1e-3, 1.0 - 1e-3));
        for a in anchors.iter_mut().filter(|a| **a >= i) {
            *a += 1;
        }
        Ok(i)
    })?;
    Ok(json!({ "anchor": index }))
}

pub(super) fn spine_remove_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.spine.removeAnchor";
    let index = index_param(p, "anchor", C)?;
    edit_spine(s, p, C, "Delete Spine Point", |sp, anchors, _| {
        if index >= sp.anchors.len() {
            return Err(bad(C, "no such spine point"));
        }
        if anchors.contains(&index) {
            return Err(bad(C, "a key object sits on that point"));
        }
        if sp.anchors.len() <= 2 {
            return Err(bad(C, "a spine keeps at least two points"));
        }
        sp.anchors.remove(index);
        for a in anchors.iter_mut().filter(|a| **a > index) {
            *a -= 1;
        }
        Ok(())
    })?;
    ok()
}

fn edit_blends(s: &mut Session, label: &str, f: impl Fn(&mut Vec<Arc<Node>>, &mut BlendSpec)) -> Result<Value> {
    let blends = selected_of(s, is_blend);
    s.edit(label, |d, _| {
        for b in &blends {
            if let Some(NodeKind::Blend { children, spec }) = d.node_mut(*b).map(|n| &mut n.kind) {
                f(children, spec);
            }
        }
        Ok(())
    })?;
    ok()
}

/// The first selected blend: its id, spec and key ids.
fn selected_spec(s: &Session) -> Option<(NodeId, BlendSpec, Vec<NodeId>)> {
    let b = *selected_of(s, is_blend).first()?;
    match &s.doc().ok()?.doc.node(b)?.kind {
        NodeKind::Blend { spec, children } => Some((b, (**spec).clone(), children.iter().map(|c| c.id).collect())),
        _ => None,
    }
}

pub(super) fn blend_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.options";
    let Some((_, cur, _)) = selected_spec(s) else {
        if ["easingType", "ramp", "colorShift"].iter().any(|key| p.get(*key).is_some_and(|v| !v.is_null())) {
            return Err(bad(C, "select a blend to set easing or color shift"));
        }
        // Nothing to set them on: the options new blends start with (easing is per blend).
        let d = s.prefs.blend_options.unwrap_or_default();
        let spacing = spacing_param(p, C, d.spacing)?;
        s.prefs.blend_options = Some(BlendDefaults { spacing, orientation: orientation_param(p, d.orientation) });
        return Ok(json!({ "defaults": true }));
    };
    let spacing = spacing_param(p, C, cur.spacing)?;
    let orientation = orientation_param(p, cur.orientation);
    let ease = ease_param(p, C, cur.ease)?;
    let color_shift = percent_param(p, C, "colorShift", cur.color_shift, -100.0)?;
    edit_blends(s, "Blend Options", |_, spec| {
        spec.spacing = spacing;
        spec.orientation = orientation;
        spec.ease = ease;
        spec.color_shift = color_shift;
    })
}

/// Spacing and orientation as `object.blend.options` takes them; the step count and distance
/// not in use show their defaults.
fn blend_options_json(spacing: BlendSpacing, orientation: BlendOrientation) -> Value {
    let (mode, steps, distance) = match spacing {
        BlendSpacing::SmoothColor => ("smooth", DEFAULT_STEPS, DEFAULT_DISTANCE),
        BlendSpacing::Steps(n) => ("steps", n, DEFAULT_DISTANCE),
        BlendSpacing::Distance(d) => ("distance", DEFAULT_STEPS, d),
    };
    let orientation = if orientation == BlendOrientation::AlignToPath { "path" } else { "page" };
    json!({ "spacing": mode, "steps": steps, "distance": distance, "orientation": orientation })
}

pub(super) fn blend_info(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(match selected_spec(s) {
        Some((id, spec, keys)) => {
            let mut v = blend_options_json(spec.spacing, spec.orientation);
            v["target"] = json!("blend");
            v["id"] = json!(id.0);
            v["easingType"] = json!(spec.ease.kind.id());
            v["ramp"] = json!(spec.ease.ramp * 100.0);
            v["colorShift"] = json!(spec.color_shift);
            v["starts"] = json!((0..keys.len()).map(|i| spec.start(i)).collect::<Vec<_>>());
            v["keys"] = json!(keys.iter().map(|k| k.0).collect::<Vec<_>>());
            if let Some(NodeKind::Blend { children, .. }) = s.doc()?.doc.node(id).map(|n| &n.kind)
                && let Some((path, anchors)) = live::blend_spine(children, &spec)
                && let Some(sp) = path.subpaths.first()
            {
                let pt = |p: Point| json!([p.x, p.y]);
                let points: Vec<Value> = sp.anchors.iter().map(|a| json!({"x": a.p.x, "y": a.p.y, "in": pt(a.h_in), "out": pt(a.h_out)})).collect();
                v["spine"] = json!({"anchors": points, "closed": sp.closed, "keyAnchors": anchors, "explicit": spec.spine.is_some()});
            }
            v
        }
        None => {
            let d = s.prefs.blend_options.unwrap_or_default();
            let mut v = blend_options_json(d.spacing, d.orientation);
            v["target"] = json!("defaults");
            v
        }
    })
}

pub(super) fn blend_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let blends = selected_of(s, is_blend);
    let ids = s.edit("Expand Blend", |d, sel| {
        for b in &blends {
            expand_blend(d, *b)?;
        }
        sel.set(blends.iter().copied());
        Ok(blends.clone())
    })?;
    Ok(ids_json(&ids))
}

/// Replace blend `id` by a group of its keys and steps in place, keeping everything about the
/// blend itself: id, name, visibility, lock, transparency (knockout and isolation included),
/// opacity mask and appearance.
fn expand_blend(d: &mut Document, id: NodeId) -> Result<()> {
    let Some(n) = d.node(id).filter(|n| is_blend(n)).cloned() else { return Ok(()) };
    let mut g = n.clone();
    g.kind = NodeKind::Group { children: vectorcraft_render::expand_live(&n).into_iter().map(Arc::new).collect(), clip: false };
    fix_ids(d, &mut g);
    *d.node_mut(id).ok_or(EngineError::NoNode(id))? = g;
    Ok(())
}

/// Object › Expand: every blend in the subtrees of `roots` (outer ones first) becomes a group of
/// its steps ([`expand_blend`]). Returns how many.
pub(crate) fn expand_blends(d: &mut Document, roots: &[NodeId]) -> Result<usize> {
    let mut blends = vec![];
    for r in roots {
        if let Some(n) = d.node(*r) {
            n.walk(&mut |c| {
                if is_blend(c) {
                    blends.push(c.id);
                }
            });
        }
    }
    for b in &blends {
        expand_blend(d, *b)?;
    }
    Ok(blends.len())
}

/// Move the keys onto their spine positions (so what's stored matches what's drawn): onto their
/// anchors, or spread evenly by arc length along a spine from Replace Spine.
fn keys_to_spine(children: &mut [Arc<Node>], spec: &BlendSpec) {
    let Some(path) = &spec.spine else { return };
    let k = children.len();
    let pinned = spec.key_anchors.len() == k;
    let sp = Spine::new(path);
    for (i, c) in children.iter_mut().enumerate() {
        let pt = if pinned {
            spec.key_anchors.get(i).and_then(|a| path.subpaths.first()?.anchors.get(*a as usize)).map(|a| a.p)
        } else {
            sp.as_ref().map(|sp| sp.at(if k > 1 { i as f64 / (k - 1) as f64 } else { 0.0 }).0)
        };
        if let (Some(pt), Some(b)) = (pt, c.geometric_bounds()) {
            Arc::make_mut(c).transform(Affine::translate(pt - b.center()), false);
        }
    }
}

/// Store the blend's spine pinned to its keys (see [`live::pin_spine`]); false when it has none.
fn pin(children: &[Arc<Node>], spec: &mut BlendSpec) -> bool {
    match live::pin_spine(children, spec) {
        Some((path, anchors)) => {
            spec.spine = Some(path);
            spec.key_anchors = anchors.iter().map(|a| u32::try_from(*a).unwrap_or(u32::MAX)).collect();
            true
        }
        None => false,
    }
}

/// `spec`'s spine reversed: the key anchors count from the other end.
fn reverse_spine(spec: &mut BlendSpec) {
    if let Some(sp) = &mut spec.spine {
        sp.reverse();
        let n = sp.subpaths.first().map_or(0, |s| s.anchors.len() as u32);
        for a in &mut spec.key_anchors {
            *a = n.saturating_sub(1).saturating_sub(*a);
        }
    }
}

pub(super) fn blend_replace_spine(s: &mut Session, _: &Value) -> Result<Value> {
    let blends = selected_of(s, is_blend);
    let st = s.doc()?;
    let path = st
        .selection
        .objects
        .iter()
        .copied()
        .find(|id| {
            !blends.contains(id)
                && st.doc.node(*id).is_some_and(|n| n.path_data().is_some())
                && !blends.iter().any(|b| st.doc.ancestry(*id).unwrap_or_default().contains(b))
        })
        .ok_or_else(|| EngineError::Other("Replace Spine: select a blend and a path".into()))?;
    let Some(&b) = blends.first() else { return Err(EngineError::Other("Replace Spine: select a blend and a path".into())) };
    s.edit("Replace Spine", |d, sel| {
        let spine = d.node(path).and_then(|n| n.path_data().cloned()).ok_or(EngineError::NoNode(path))?;
        d.remove(path)?;
        if let Some(NodeKind::Blend { children, spec }) = d.node_mut(b).map(|n| &mut n.kind) {
            spec.spine = Some(spine);
            spec.key_anchors.clear();
            keys_to_spine(children, spec);
        }
        sel.set([b]);
        Ok(())
    })?;
    ok()
}

pub(super) fn blend_reverse_spine(s: &mut Session, _: &Value) -> Result<Value> {
    edit_blends(s, "Reverse Spine", |children, spec| {
        if spec.spine.is_some() && pin(children, spec) {
            // The keys keep their anchors, which now count from the other end.
            if let Some(sp) = &mut spec.spine {
                sp.reverse();
            }
            keys_to_spine(children, spec);
        } else {
            let centers: Vec<Point> = children.iter().map(|k| k.geometric_bounds().map(|b| b.center()).unwrap_or_default()).collect();
            let n = children.len();
            for (i, k) in children.iter_mut().enumerate() {
                Arc::make_mut(k).transform(Affine::translate(centers[n - 1 - i] - centers[i]), false);
            }
        }
    })
}

pub(super) fn blend_reverse_stack(s: &mut Session, _: &Value) -> Result<Value> {
    edit_blends(s, "Reverse Front to Back", |children, spec| {
        children.reverse();
        // Each key keeps its start point.
        if !spec.starts.is_empty() {
            spec.starts.resize(children.len(), None);
            spec.starts.reverse();
        }
        // Each key keeps its place: on its anchor, or (spread evenly) with the spine reversed.
        spec.key_anchors.reverse();
        reverse_spine(spec);
    })
}
