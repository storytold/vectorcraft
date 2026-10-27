use super::*;

pub(super) enum Arrange {
    Front,
    Forward,
    Backward,
    Back,
}

pub(super) fn arrange(s: &mut Session, how: Arrange) -> Result<Value> {
    let mut ids = selected_roots(s)?;
    if matches!(how, Arrange::Forward | Arrange::Front) {
        ids.reverse();
    }
    let label = match how {
        Arrange::Front => "Bring to Front",
        Arrange::Forward => "Bring Forward",
        Arrange::Backward => "Send Backward",
        Arrange::Back => "Send to Back",
    };
    // When nothing changes order, the document and its undo history stay as they are.
    crate::cmd::overprint::edit_counted(s, label, |d| {
        let mut moved = 0;
        // Arrange within each parent independently (selections may span layers/groups).
        let mut by_parent: Vec<(Option<vectorcraft_doc::NodeId>, Vec<vectorcraft_doc::NodeId>)> = vec![];
        for id in &ids {
            let par = d.parent_of(*id);
            match by_parent.iter_mut().find(|(p, _)| *p == par) {
                Some((_, v)) => v.push(*id),
                None => by_parent.push((par, vec![*id])),
            }
        }
        for (par, mut group) in by_parent {
            // In a clip group or a clipping layer the bottom child is the clipping path: it stays
            // there, and the art above it never goes below it (#3188).
            let floor = usize::from(par.and_then(|p| d.node(p)).is_some_and(vectorcraft_doc::Node::clips));
            if floor > 0 {
                group.retain(|id| d.position(*id).is_some_and(|(_, idx, _)| idx >= floor));
            }
            for (k, id) in group.iter().enumerate() {
                let Some((_, idx, len)) = d.position(*id) else { continue };
                if len <= floor {
                    continue;
                }
                let last = len - 1;
                let to = match how {
                    Arrange::Front => last.saturating_sub(k).max(floor),
                    Arrange::Back => (floor + k).min(last),
                    Arrange::Forward => (idx + 1).min(last.saturating_sub(k)).max(idx),
                    Arrange::Backward => idx.saturating_sub(1).max(floor + k.min(idx.saturating_sub(floor))),
                };
                if to != idx {
                    d.move_node(*id, par, to)?;
                    moved += 1;
                }
            }
        }
        Ok(moved)
    })?;
    ok()
}

pub(super) fn send_to_current_layer(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // Ids are reused after undo: the remembered current layer must still be a layer.
    let st = s.doc()?;
    let layer = st.current_layer().or_else(|| st.doc.default_layer()).ok_or_else(|| EngineError::Other("no current layer".into()))?;
    // When the current layer is hidden or locked, or is inside a hidden or locked layer, the
    // command fails with the error that `DocState::target_parent` gives for new art.
    if !st.doc.is_editable(layer) {
        return Err(EngineError::Other("the target layer is locked or hidden".into()));
    }
    // When the selected objects are already the topmost in the current layer, in paint order, the
    // document and its undo history stay as they are.
    if st.doc.children(Some(layer)).is_some_and(|c| c.iter().rev().map(|n| n.id).take(ids.len()).eq(ids.iter().rev().copied())) {
        return ok();
    }
    s.edit("Send to Current Layer", |d, _| {
        let moved = d.remove_many(&ids)?;
        d.children_mut(Some(layer))?.extend(moved);
        Ok(())
    })?;
    ok()
}

pub(super) fn group(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let gid = s.edit("Group", |d, sel| {
        let gid = group_nodes(d, &ids)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0 }))
}

/// Put `ids` (top-level objects, in paint order) in a new group where the front-most one was.
pub(crate) fn group_nodes(d: &mut Document, ids: &[NodeId]) -> Result<NodeId> {
    if ids.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let gid = d.alloc_id();
    Ok(d.wrap(ids, |children| Node::group(gid, children))?)
}

pub(super) fn ungroup(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no group selected, the document, the selection and the undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. }))) {
        return ok();
    }
    s.edit("Ungroup", |d, sel| {
        let mut new_sel = vec![];
        let mut ungrouped = vec![];
        for id in &ids {
            let Some(n) = d.node(*id) else { continue };
            let NodeKind::Group { children, clip } = &n.kind else {
                new_sel.push(*id);
                continue;
            };
            let mut out = Vec::with_capacity(children.len());
            for (k, c) in children.iter().enumerate() {
                let mut c = (**c).clone();
                // The clipping path keeps its paint (none, unless painted after Make).
                if *clip
                    && k == 0
                    && let NodeKind::Path { clipping, .. } = &mut c.kind
                {
                    *clipping = false;
                }
                c.opacity *= n.opacity;
                new_sel.push(c.id);
                out.push(c);
            }
            ungrouped.push((*id, out));
        }
        d.insert_beside(ungrouped, true)?;
        sel.set(new_sel);
        Ok(())
    })?;
    ok()
}

pub(super) fn lock(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With every selected object already locked, the document and its undo history stay as they
    // are, and the objects are deselected.
    let d = &s.doc()?.doc;
    if ids.iter().all(|id| d.node(*id).is_none_or(|n| n.locked)) {
        s.select(|_, sel| sel.clear())?;
        return ok();
    }
    s.edit("Lock", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.locked = true;
            }
        }
        sel.clear();
        Ok(())
    })?;
    ok()
}

fn collect_flag(d: &Document, f: impl Fn(&Node) -> bool) -> Vec<NodeId> {
    let mut v = vec![];
    d.walk(|n| {
        if !n.is_layer() && f(n) {
            v.push(n.id)
        }
    });
    v
}

pub(super) fn unlock_all(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = collect_flag(&s.doc()?.doc, |n| n.locked);
    // With nothing locked, the document, the selection and the undo history stay as they are.
    if ids.is_empty() {
        return Ok(json!({ "count": 0 }));
    }
    s.edit("Unlock All", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.locked = false;
            }
        }
        sel.set(ids.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "count": ids.len() }))
}

pub(super) fn hide(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With every selected object already hidden, the document and its undo history stay as they
    // are, and the objects are deselected.
    let d = &s.doc()?.doc;
    if ids.iter().all(|id| d.node(*id).is_none_or(|n| !n.visible)) {
        s.select(|_, sel| sel.clear())?;
        return ok();
    }
    s.edit("Hide", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.visible = false;
            }
        }
        sel.clear();
        Ok(())
    })?;
    ok()
}

pub(super) fn show_all(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = collect_flag(&s.doc()?.doc, |n| !n.visible);
    // With nothing hidden, the document, the selection and the undo history stay as they are.
    if ids.is_empty() {
        return Ok(json!({ "count": 0 }));
    }
    s.edit("Show All", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.visible = true;
            }
        }
        sel.set(ids.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "count": ids.len() }))
}

pub(super) fn isolate(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id")
        .or_else(|| s.active().and_then(|d| d.selection.objects.first().copied()))
        .ok_or_else(|| bad("object.isolate", "missing id"))?;
    let guides_locked = s.menu.guides_locked;
    let st = s.doc_mut()?;
    // Groups, layers and single objects (a path, a compound path, an image…); type isn't
    // isolated: a double-click on it edits its text.
    if matches!(st.doc.node(id).ok_or(EngineError::NoNode(id))?.kind, NodeKind::Text(_)) {
        return Err(bad("object.isolate", "type objects can't be isolated"));
    }
    let isolation = st.doc.node(id).filter(|n| n.extras().shaper.is_some()).and_then(|n| n.children()).and_then(|c| c.first()).map_or(id, |n| n.id);
    // A hidden or locked object, one in a hidden or locked group or layer, and a guide path while
    // Lock Guides is on aren't isolated either: a click with the Selection tool doesn't select them.
    // A Shaper Group isolates its source group, so that group is the one checked.
    if !st.doc.is_editable(isolation) {
        return Err(bad("object.isolate", "the object is hidden or locked"));
    }
    if guides_locked && crate::is_guide_path(&st.doc, isolation) {
        return Err(EngineError::Other("guides are locked".into()));
    }
    st.isolation = Some(isolation);
    st.selection.clear();
    // New art on a locked artboard, now deselected, is protected.
    st.sync_new_art();
    st.revision += 1;
    ok()
}

pub(super) fn exit_isolation(s: &mut Session, _: &Value) -> Result<Value> {
    let menu = s.menu.clone();
    let st = s.doc_mut()?;
    if let Some(i) = st.isolation.take() {
        crate::cmd::distortcmds::finish_edit_text(st, i);
        let target = st.doc.parent_of(i).filter(|p| st.doc.node(*p).is_some_and(|n| n.extras().shaper.is_some())).unwrap_or(i);
        st.selection.set([target]);
        // A guide path isolated while Lock Guides is on isn't selected (#1534).
        crate::deselect_hidden(&menu, &st.doc, &mut st.selection);
        // The art on a locked artboard can't be selected.
        st.protect_locked_artboards(&[], &Default::default(), None);
    }
    st.revision += 1;
    ok()
}
