use super::*;

pub(super) fn compound_make(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let Some(backmost) = ids.first().copied() else { return Err(EngineError::Other("nothing selected".into())) };
    let id = s.edit("Make Compound Path", |d, sel| {
        d.node(backmost).ok_or(EngineError::NoNode(backmost))?;
        let mut units: Vec<Vec<Arc<Node>>> = vec![];
        let mut appearance = None;
        for id in &ids {
            let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
            if !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Group { .. }) {
                return Err(EngineError::Other("compound paths can only contain paths".into()));
            }
            compound_members(n, &mut appearance, &mut units);
        }
        let cid = d.alloc_id();
        let mut c = Node::new(cid, NodeKind::Compound { children: vec![], rule: FillRule::NonZero });
        c.appearance = appearance.unwrap_or_default();
        let ch = c.children_mut().ok_or(EngineError::NoNode(cid))?;
        // The paths in front run against the backmost one, so where they overlap it they cut holes
        // under the non-zero fill rule (#1197). A compound path's paths turn together, by its
        // first: its own holes (a letter's counters) stay holes.
        let direction = |n: &Node| n.path_data().and_then(|pd| pd.subpaths.first()).map_or(0.0, |sp| sp.signed_area().signum());
        let first = |unit: &[Arc<Node>]| unit.first().map_or(0.0, |p| direction(p));
        let back = units.first().map_or(0.0, |u| first(u));
        for (i, unit) in units.into_iter().enumerate() {
            let flip = i > 0 && back != 0.0 && first(&unit) == back;
            for mut p in unit {
                let pn = Arc::make_mut(&mut p);
                pn.appearance = Appearance::default().into();
                if let NodeKind::Path { path, live, .. } = &mut pn.kind {
                    *live = None;
                    if flip {
                        path.reverse();
                    }
                }
                ch.push(p);
            }
        }
        d.wrap(&ids, |_| c)?;
        sel.set([cid]);
        Ok(cid)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The paths in `n` (a path, a compound path, or a group searched in depth) onto `out`, in paint
/// order: each path alone, a compound path's paths together. Into `paint` goes the appearance of
/// the backmost found so far: the path's own, or its compound path's, which is what painted it. A
/// group has no paint of its own to give: a group of outlined type made a compound path that drew
/// nothing.
fn compound_members(n: &Node, paint: &mut Option<vectorcraft_doc::Shared<Appearance>>, out: &mut Vec<Vec<Arc<Node>>>) {
    match (&n.kind, n.children()) {
        (NodeKind::Path { .. }, _) => out.push(vec![Arc::new(n.clone())]),
        (NodeKind::Compound { .. }, Some(paths)) if !paths.is_empty() => out.push(paths.clone()),
        (_, children) => return children.into_iter().flatten().for_each(|c| compound_members(c, paint, out)),
    }
    paint.get_or_insert_with(|| n.appearance.clone());
}

/// Is `n` a compound path to release: a compound path, or a path of several subpaths (an SVG `d`
/// with several `M`s, a PDF path), which is one in Illustrator too (#1163)?
fn releases(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Compound { .. } => true,
        NodeKind::Path { path, .. } => path.subpaths.len() > 1,
        _ => false,
    }
}

pub(super) fn compound_release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no compound path selected, the document, the selection and the undo history stay as
    // they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(releases)) {
        return ok();
    }
    s.edit("Release Compound Path", |d, sel| {
        let mut out = vec![];
        let mut released = vec![];
        for id in &ids {
            let Some(n) = d.node(*id).cloned().filter(releases) else { continue };
            let mut parts = vec![];
            match &n.kind {
                NodeKind::Compound { children, .. } => {
                    for c in children {
                        let mut c = (**c).clone();
                        c.appearance = n.appearance.clone();
                        out.push(c.id);
                        parts.push(c);
                    }
                }
                // Each subpath becomes a path of its own with the path's paint, in the same
                // stacking order; the first keeps the path's id.
                NodeKind::Path { path, .. } => {
                    for (k, sp) in path.subpaths.iter().enumerate() {
                        let mut c = if k == 0 { n.clone() } else { d.reid(&n) };
                        if let NodeKind::Path { path, live, .. } = &mut c.kind {
                            (*path, *live) = (PathData::single(sp.clone()), None);
                        }
                        out.push(c.id);
                        parts.push(c);
                    }
                }
                _ => {}
            }
            released.push((*id, parts));
        }
        d.insert_beside(released, true)?;
        sel.set(out);
        Ok(())
    })?;
    ok()
}

/// Make `top` (a path, compound path or text object) the clipping path: it loses its paint (it
/// stays unpainted after Release).
pub(in super::super) fn make_clipping_path(d: &mut Document, top: NodeId) -> Result<()> {
    as_clipping_path(d.node_mut(top).ok_or(EngineError::NoNode(top))?)
}

/// Turn path, compound path or text `c` into a clipping path (its paint goes).
pub(in super::super) fn as_clipping_path(c: &mut Node) -> Result<()> {
    if !matches!(c.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } | NodeKind::Text(_)) {
        return Err(EngineError::Other("the top object must be a path, compound path or text object to use as a clipping mask".into()));
    }
    c.appearance = Appearance::basic(Paint::None, Paint::None, 0.0).into();
    match &mut c.kind {
        NodeKind::Path { clipping, .. } => *clipping = true,
        NodeKind::Text(t) => {
            for r in &mut t.runs {
                (r.style.fill, r.style.stroke) = (Paint::None, Paint::None);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Stop clip group or clipped layer `id` clipping; its clipping path stays, with its paint.
pub(in super::super) fn release_clip(d: &mut Document, id: NodeId) {
    let Some(n) = d.node_mut(id).filter(|n| n.clips()) else { return };
    n.set_clips(false);
    let clip = n.children().and_then(|c| c.first()).map(|c| c.id);
    if let Some(c) = clip.and_then(|c| d.node_mut(c))
        && let NodeKind::Path { clipping, .. } = &mut c.kind
    {
        *clipping = false;
    }
}

pub(super) fn clip_make(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let Some(&top) = ids.last() else { return Err(EngineError::Other("nothing selected".into())) };
    let gid = s.edit("Make Clipping Mask", |d, sel| {
        make_clipping_path(d, top)?;
        let gid = d.alloc_id();
        // The topmost object, the clipping path, goes first, and the others follow in paint order.
        d.wrap(&ids, |mut children| {
            children.rotate_right(1);
            Node::new(gid, NodeKind::Group { children, clip: true })
        })?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0 }))
}

pub(super) fn clip_release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no clip group selected, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(Node::clips)) {
        return ok();
    }
    s.edit("Release Clipping Mask", |d, _| {
        for id in &ids {
            release_clip(d, *id);
        }
        Ok(())
    })?;
    ok()
}

/// Selects the clipping path (`mask`) or the clipped contents of every selected clip group (or of the
/// clip group containing a selected object), like Illustrator's Edit Contents / Edit Clipping Path toggle.
pub(super) fn clip_edit(s: &mut Session, mask: bool) -> Result<Value> {
    let st = s.doc()?;
    let mut ids = vec![];
    for id in st.selection.objects.iter().copied() {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if let Some(NodeKind::Group { children, clip: true }) = st.doc.node(c).map(|n| &n.kind) {
                if mask {
                    ids.extend(children.first().map(|c| c.id));
                } else {
                    ids.extend(children.iter().skip(1).map(|c| c.id));
                }
                break;
            }
            cur = st.doc.position(c).and_then(|(par, _, _)| par);
        }
    }
    ids.dedup();
    if ids.is_empty() {
        return Err(EngineError::Other("select a clip group".into()));
    }
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    Ok(json!({ "count": ids.len() }))
}
