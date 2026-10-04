//! Object menu: transforms, arrange, group, lock/hide, compound paths, clipping masks, isolation,
//! align & distribute, object properties.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, Document, Knockout, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, Point, Rect, Vec2};

use super::edit::{duplicate_in, selected_roots};
use super::opacitymask::percent;
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.transform",
            "Transform",
            [],
            None,
            "{matrix: [a,b,c,d,e,f], copy?: bool, ids?} apply an affine to the selection (or ids)",
            has_doc,
            transform
        ),
        cmd!("object.move", "Move…", ["Object", "Transform"], Some("Cmd+Shift+M"), "{dx, dy, copy?}", has_selection, move_cmd),
        cmd!(
            "object.rotate",
            "Rotate…",
            ["Object", "Transform"],
            None,
            "{angle: deg (counter-clockwise), origin?: [x,y], copy?}",
            has_selection,
            rotate
        ),
        cmd!("object.scale", "Scale…", ["Object", "Transform"], None, "{sx: %, sy?: %, origin?: [x,y], copy?, strokes?: bool}", has_selection, scale),
        cmd!(
            "object.reflect",
            "Reflect…",
            ["Object", "Transform"],
            None,
            "{axis: \"vertical\"|\"horizontal\"|deg, origin?, copy?}",
            has_selection,
            reflect
        ),
        cmd!(
            "object.shear",
            "Shear…",
            ["Object", "Transform"],
            None,
            "{angle: deg, axis?: \"horizontal\"|\"vertical\", origin?, copy?}",
            has_selection,
            shear
        ),
        cmd!("object.transformAgain", "Transform Again", ["Object", "Transform"], Some("Cmd+D"), "{}", has_selection, transform_again),
        cmd!(
            "object.nudge",
            "Nudge",
            [],
            None,
            "{dx: -1|0|1, dy: -1|0|1, big?: bool (×10), copy?: bool} arrow-key nudge by the keyboard increment",
            has_selection,
            nudge
        ),
        cmd!("object.arrange.bringToFront", "Bring to Front", ["Object", "Arrange"], Some("Cmd+Shift+]"), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Front
        )),
        cmd!("object.arrange.bringForward", "Bring Forward", ["Object", "Arrange"], Some("Cmd+]"), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Forward
        )),
        cmd!("object.arrange.sendBackward", "Send Backward", ["Object", "Arrange"], Some("Cmd+["), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Backward
        )),
        cmd!("object.arrange.sendToBack", "Send to Back", ["Object", "Arrange"], Some("Cmd+Shift+["), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Back
        )),
        cmd!("object.arrange.sendToCurrentLayer", "Send to Current Layer", ["Object", "Arrange"], None, "{}", has_selection, send_to_current_layer),
        cmd!("object.group", "Group", ["Object"], Some("Cmd+G"), "{} → {id}", has_selection, group),
        cmd!("object.ungroup", "Ungroup", ["Object"], Some("Cmd+Shift+G"), "{}", has_selection, ungroup),
        cmd!("object.lock", "Selection", ["Object", "Lock"], Some("Cmd+2"), "{}", has_selection, lock),
        cmd!("object.unlockAll", "Unlock All", ["Object"], Some("Cmd+Alt+2"), "{}", has_doc, unlock_all),
        cmd!("object.hide", "Selection", ["Object", "Hide"], Some("Cmd+3"), "{}", has_selection, hide),
        cmd!("object.showAll", "Show All", ["Object"], Some("Cmd+Alt+3"), "{}", has_doc, show_all),
        cmd!("object.compoundPath.make", "Make", ["Object", "Compound Path"], Some("Cmd+8"), "{}", has_selection, compound_make),
        cmd!("object.compoundPath.release", "Release", ["Object", "Compound Path"], Some("Cmd+Alt+Shift+8"), "{}", has_selection, compound_release),
        cmd!(
            "object.clippingMask.make",
            "Make",
            ["Object", "Clipping Mask"],
            Some("Cmd+7"),
            "{} the topmost selected object (a path, compound path or text, which loses its paint) clips the others: compound holes, even-odd fills and glyph outlines clip as drawn → {id} of the clip group",
            has_multi,
            clip_make
        ),
        cmd!(
            "object.clippingMask.release",
            "Release",
            ["Object", "Clipping Mask"],
            Some("Cmd+Alt+7"),
            "{} the selected clip groups become plain groups; their clipping path (path, compound path or text) stays, unpainted",
            has_selection,
            clip_release
        ),
        cmd!(
            "object.clippingMask.editContents",
            "Edit Contents",
            ["Object", "Clipping Mask"],
            None,
            "{} select the clipped art of the selected clip groups → {count}",
            has_selection,
            |s, _| clip_edit(s, false)
        ),
        cmd!(
            "object.clippingMask.editMask",
            "Edit Clipping Path",
            ["Object", "Clipping Mask"],
            None,
            "{} select the clipping paths of the selected clip groups → {count}",
            has_selection,
            |s, _| clip_edit(s, true)
        ),
        cmd!("object.isolate", "Enter Isolation Mode", [], None, "{id}", has_doc, isolate),
        cmd!("object.exitIsolation", "Exit Isolation Mode", [], None, "{}", has_doc, exit_isolation),
        cmd!(
            "object.setProps",
            "Object Properties",
            [],
            None,
            "{ids?|id?, name?, visible?, locked?, opacity?: 0..100, blend?: \"Multiply\"…, isolate?, knockout?: \"on\"|\"off\"|\"neutral\"|bool (true = on, false = neutral), knockoutShape?: bool}",
            has_doc,
            set_props
        ),
        cmd!(
            "object.align",
            "Align",
            ["Window", "Align"],
            None,
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\", to?: \"selection\"|\"artboard\"|\"key\"}",
            has_selection,
            align
        ),
        cmd!(
            "object.distribute",
            "Distribute",
            ["Window", "Align"],
            None,
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\"}",
            has_multi,
            distribute
        ),
        cmd!(
            "object.distributeSpacing",
            "Distribute Spacing",
            ["Window", "Align"],
            None,
            "{axis: \"horizontal\"|\"vertical\", spacing?: pt}",
            has_multi,
            distribute_spacing
        ),
        cmd!(
            "object.setBounds",
            "Set Bounds",
            [],
            None,
            "{x?, y?, width?, height?, reference?: 0..8 (9-point grid), proportional?} (Transform panel)",
            has_selection,
            set_bounds
        ),
        cmd!("object.expandShape", "Expand Shape", ["Object", "Shape"], None, "{} convert live shapes to plain paths", has_selection, expand_shape),
        cmd!("object.setLiveShape", "Live Shape Properties", [], None, "{id?, radius?: pt (all corners), sides?: n}", has_selection, set_live_shape),
    ]
}

fn origin_of(s: &Session, p: &Value, ids: &[NodeId]) -> Result<Point> {
    if let Some(o) = point_param(p, "origin") {
        return Ok(o);
    }
    s.doc()?.doc.bounds_of(ids, false).map(|b| b.center()).ok_or_else(|| EngineError::Other("selection has no bounds".into()))
}

/// Apply `xf` to `ids` (copy = duplicate first). Records Transform Again.
pub(crate) fn apply_transform(s: &mut Session, label: &str, ids: Vec<NodeId>, xf: Affine, copy: bool) -> Result<Value> {
    let scale_strokes = s.prefs.scale_strokes;
    let ids = s.edit(label, |d, sel| {
        let targets = if copy { duplicate_in(d, sel, &ids, Affine::IDENTITY)? } else { ids.clone() };
        for id in &targets {
            if let Some(n) = d.node_mut(*id) {
                n.transform(xf, scale_strokes);
            }
        }
        Ok(targets)
    })?;
    let st = s.doc_mut()?;
    if st.interaction.is_none() {
        st.last_transform = Some((xf, copy));
    }
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn transform(s: &mut Session, p: &Value) -> Result<Value> {
    let m = matrix_param(p, "matrix").ok_or_else(|| bad("object.transform", "missing matrix [a,b,c,d,e,f]"))?;
    let ids = match ids_param(p, "ids") {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    apply_transform(s, "Transform", ids, m, bool_or(p, "copy", false))
}

fn move_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    apply_transform(s, "Move", ids, Affine::translate((f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0))), bool_or(p, "copy", false))
}

fn nudge(s: &mut Session, p: &Value) -> Result<Value> {
    let k = s.prefs.keyboard_increment * if bool_or(p, "big", false) { 10.0 } else { 1.0 };
    let dx = f64_or(p, "dx", 0.0) * k;
    let dy = f64_or(p, "dy", 0.0) * k;
    if !s.doc()?.selection.anchors.is_empty() {
        return super::path::move_anchors(s, &json!({ "dx": dx, "dy": dy }));
    }
    let ids = selected_roots(s)?;
    apply_transform(s, "Move", ids, Affine::translate((dx, dy)), bool_or(p, "copy", false))
}

fn about(o: Point, a: Affine) -> Affine {
    Affine::translate(o.to_vec2()) * a * Affine::translate(-o.to_vec2())
}

fn rotate(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    // Illustrator angles are counter-clockwise; y is down, so negate.
    let a = about(o, Affine::rotate(-f64_or(p, "angle", 0.0).to_radians()));
    apply_transform(s, "Rotate", ids, a, bool_or(p, "copy", false))
}

fn scale(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    let sx = f64_req(p, "sx", "object.scale")? / 100.0;
    let sy = f64_or(p, "sy", sx * 100.0) / 100.0;
    if sx == 0.0 || sy == 0.0 {
        return Err(bad("object.scale", "scale must be non-zero"));
    }
    let prev = s.prefs.scale_strokes;
    if let Some(b) = p.get("strokes").and_then(Value::as_bool) {
        s.prefs.scale_strokes = b;
    }
    let r = apply_transform(s, "Scale", ids, about(o, Affine::scale_non_uniform(sx, sy)), bool_or(p, "copy", false));
    s.prefs.scale_strokes = prev;
    r
}

fn reflect(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    let m = match p.get("axis") {
        Some(Value::String(a)) if a == "horizontal" => Affine::scale_non_uniform(1.0, -1.0),
        Some(Value::Number(n)) => {
            let t = -n.as_f64().unwrap_or(90.0).to_radians();
            Affine::rotate(t) * Affine::scale_non_uniform(1.0, -1.0) * Affine::rotate(-t)
        }
        _ => Affine::scale_non_uniform(-1.0, 1.0),
    };
    apply_transform(s, "Reflect", ids, about(o, m), bool_or(p, "copy", false))
}

fn shear(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    let t = f64_or(p, "angle", 0.0).clamp(-89.0, 89.0).to_radians().tan();
    let m =
        if str_param(p, "axis") == Some("vertical") { Affine::new([1.0, t, 0.0, 1.0, 0.0, 0.0]) } else { Affine::new([1.0, 0.0, -t, 1.0, 0.0, 0.0]) };
    apply_transform(s, "Shear", ids, about(o, m), bool_or(p, "copy", false))
}

fn transform_again(s: &mut Session, _: &Value) -> Result<Value> {
    let (m, copy) = s.doc()?.last_transform.ok_or_else(|| EngineError::Other("no previous transform".into()))?;
    let ids = selected_roots(s)?;
    apply_transform(s, "Transform Again", ids, m, copy)
}

enum Arrange {
    Front,
    Forward,
    Backward,
    Back,
}

fn arrange(s: &mut Session, how: Arrange) -> Result<Value> {
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
    s.edit(label, |d, _| {
        // Arrange within each parent independently (selections may span layers/groups).
        let mut by_parent: Vec<(Option<vectorcraft_doc::NodeId>, Vec<vectorcraft_doc::NodeId>)> = vec![];
        for id in &ids {
            let par = d.parent_of(*id);
            match by_parent.iter_mut().find(|(p, _)| *p == par) {
                Some((_, v)) => v.push(*id),
                None => by_parent.push((par, vec![*id])),
            }
        }
        for (par, group) in by_parent {
            for (k, id) in group.iter().enumerate() {
                let Some((_, idx, len)) = d.position(*id) else { continue };
                if len == 0 {
                    continue;
                }
                let last = len - 1;
                let to = match how {
                    Arrange::Front => last.saturating_sub(k),
                    Arrange::Back => k.min(last),
                    Arrange::Forward => (idx + 1).min(last.saturating_sub(k)).max(idx),
                    Arrange::Backward => idx.saturating_sub(1).max(k.min(idx)),
                };
                if to != idx {
                    d.move_node(*id, par, to)?;
                }
            }
        }
        Ok(())
    })?;
    ok()
}

fn send_to_current_layer(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // Ids are reused after undo: the remembered current layer must still be a layer.
    let st = s.doc()?;
    let layer = st.current_layer().or_else(|| st.doc.default_layer()).ok_or_else(|| EngineError::Other("no current layer".into()))?;
    s.edit("Send to Current Layer", |d, _| {
        for id in &ids {
            d.move_node(*id, Some(layer), usize::MAX)?;
        }
        Ok(())
    })?;
    ok()
}

fn group(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let Some(top) = ids.last().copied() else { return Err(EngineError::Other("nothing selected".into())) };
    let gid = s.edit("Group", |d, sel| {
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let gid = d.alloc_id();
        d.insert(par, idx + 1, Node::group(gid, vec![]))?;
        for id in &ids {
            d.move_node(*id, Some(gid), usize::MAX)?;
        }
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0 }))
}

fn ungroup(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    s.edit("Ungroup", |d, sel| {
        let mut new_sel = vec![];
        for id in &ids {
            let Some(n) = d.node(*id) else { continue };
            let NodeKind::Group { children, clip } = &n.kind else {
                new_sel.push(*id);
                continue;
            };
            let children = children.clone();
            let clip = *clip;
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            let opacity = n.opacity;
            d.remove(*id)?;
            for (k, c) in children.into_iter().enumerate() {
                let mut c = (*c).clone();
                if clip && k == 0 {
                    if let NodeKind::Path { clipping, .. } = &mut c.kind {
                        *clipping = false;
                    }
                    c.appearance = Appearance::default();
                }
                c.opacity *= opacity;
                new_sel.push(c.id);
                d.insert(par, idx + k, c)?;
            }
        }
        sel.set(new_sel);
        Ok(())
    })?;
    ok()
}

fn lock(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
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

fn unlock_all(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = collect_flag(&s.doc()?.doc, |n| n.locked);
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

fn hide(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
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

fn show_all(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = collect_flag(&s.doc()?.doc, |n| !n.visible);
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

fn compound_make(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let Some(top) = ids.last().copied() else { return Err(EngineError::Other("nothing selected".into())) };
    let id = s.edit("Make Compound Path", |d, sel| {
        let appearance = d.node(top).map(|n| n.appearance.clone()).unwrap_or_default();
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let mut paths: Vec<Arc<Node>> = vec![];
        for id in &ids {
            let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
            match &n.kind {
                NodeKind::Path { .. } => paths.push(Arc::new(n)),
                NodeKind::Compound { children, .. } => paths.extend(children.iter().cloned()),
                NodeKind::Group { .. } => {
                    n.walk(&mut |c| {
                        if matches!(c.kind, NodeKind::Path { .. }) {
                            paths.push(Arc::new(c.clone()))
                        }
                    });
                }
                _ => return Err(EngineError::Other("compound paths can only contain paths".into())),
            }
        }
        let cid = d.alloc_id();
        let mut c = Node::new(cid, NodeKind::Compound { children: vec![], rule: FillRule::NonZero });
        c.appearance = appearance;
        d.insert(par, idx + 1, c)?;
        for id in &ids {
            d.remove(*id)?;
        }
        let ch = d.node_mut(cid).and_then(Node::children_mut).ok_or(EngineError::NoNode(cid))?;
        for mut p in paths {
            let pn = Arc::make_mut(&mut p);
            pn.appearance = Appearance::default();
            if let NodeKind::Path { live, .. } = &mut pn.kind {
                *live = None;
            }
            ch.push(p);
        }
        sel.set([cid]);
        Ok(cid)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn compound_release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    s.edit("Release Compound Path", |d, sel| {
        let mut out = vec![];
        for id in &ids {
            let Some(n) = d.node(*id).cloned() else { continue };
            let NodeKind::Compound { children, .. } = &n.kind else { continue };
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            d.remove(*id)?;
            for (k, c) in children.iter().enumerate() {
                let mut c = (**c).clone();
                c.appearance = n.appearance.clone();
                out.push(c.id);
                d.insert(par, idx + k, c)?;
            }
        }
        sel.set(out);
        Ok(())
    })?;
    ok()
}

/// Make `top` (a path, compound path or text object) the clipping path: it loses its paint (it
/// stays unpainted after Release).
pub(super) fn make_clipping_path(d: &mut Document, top: NodeId) -> Result<()> {
    let c = d.node_mut(top).ok_or(EngineError::NoNode(top))?;
    if !matches!(c.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } | NodeKind::Text(_)) {
        return Err(EngineError::Other("the top object must be a path, compound path or text object to use as a clipping mask".into()));
    }
    c.appearance = Appearance::basic(Paint::None, Paint::None, 0.0);
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

/// Stop clip group or clipped layer `id` clipping; its clipping path stays, unpainted.
pub(super) fn release_clip(d: &mut Document, id: NodeId) {
    let Some(n) = d.node_mut(id).filter(|n| n.clips()) else { return };
    n.set_clips(false);
    let clip = n.children().and_then(|c| c.first()).map(|c| c.id);
    if let Some(c) = clip.and_then(|c| d.node_mut(c))
        && let NodeKind::Path { clipping, .. } = &mut c.kind
    {
        *clipping = false;
    }
}

fn clip_make(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let top = *ids.last().ok_or_else(|| EngineError::Other("select the art and a path on top to use as a clipping mask".into()))?;
    let gid = s.edit("Make Clipping Mask", |d, sel| {
        make_clipping_path(d, top)?;
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let gid = d.alloc_id();
        d.insert(par, idx + 1, Node::new(gid, NodeKind::Group { children: vec![], clip: true }))?;
        d.move_node(top, Some(gid), 0)?;
        for id in &ids[..ids.len() - 1] {
            d.move_node(*id, Some(gid), usize::MAX)?;
        }
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0 }))
}

fn clip_release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
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
fn clip_edit(s: &mut Session, mask: bool) -> Result<Value> {
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

fn isolate(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id")
        .or_else(|| s.active().and_then(|d| d.selection.objects.first().copied()))
        .ok_or_else(|| bad("object.isolate", "missing id"))?;
    let st = s.doc_mut()?;
    if st.doc.node(id).is_none_or(|n| !n.is_container()) {
        return Err(bad("object.isolate", "only groups and layers can be isolated"));
    }
    st.isolation = Some(id);
    st.selection.clear();
    st.revision += 1;
    ok()
}

fn exit_isolation(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    if let Some(i) = st.isolation.take() {
        st.selection.set([i]);
    }
    st.revision += 1;
    ok()
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    let opacity = p.get("opacity").and_then(Value::as_f64).map(percent);
    let blend = match str_param(p, "blend") {
        Some(b) => Some(BlendMode::parse(b).ok_or_else(|| bad("object.setProps", format!("unknown blend mode `{b}`")))?),
        None => None,
    };
    let knockout = match p.get("knockout") {
        Some(v) => Some(
            Knockout::from_value(v)
                .ok_or_else(|| bad("object.setProps", format!("knockout must be \"on\", \"off\", \"neutral\" or a bool, not {v}")))?,
        ),
        None => None,
    };
    s.edit("Object Properties", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            if let Some(v) = str_param(p, "name") {
                n.name = if v.is_empty() { None } else { Some(v.to_string()) };
            }
            if let Some(v) = p.get("visible").and_then(Value::as_bool) {
                n.visible = v;
            }
            if let Some(v) = p.get("locked").and_then(Value::as_bool) {
                n.locked = v;
            }
            if let Some(v) = opacity {
                n.opacity = v;
            }
            if let Some(b) = blend {
                n.blend = b;
            }
            if let Some(v) = p.get("isolate").and_then(Value::as_bool) {
                n.isolate = v;
            }
            if let Some(k) = knockout {
                n.knockout = k;
            }
            if let Some(v) = p.get("knockoutShape").and_then(Value::as_bool) {
                n.knockout_shape = v;
            }
        }
        Ok(())
    })?;
    ok()
}

fn reference_rect(s: &Session, p: &Value, ids: &[NodeId]) -> Result<Rect> {
    let st = s.doc()?;
    match str_param(p, "to") {
        Some("artboard") => {
            let b = st.doc.bounds_of(ids, false).unwrap_or_default();
            let i = st.doc.artboard_at(b.center()).unwrap_or(0);
            Ok(st.doc.artboards.get(i).map(|a| a.rect).unwrap_or(b))
        }
        Some("key") => {
            let k = st.selection.key.ok_or_else(|| EngineError::Other("no key object".into()))?;
            st.doc.node(k).and_then(|n| n.geometric_bounds()).ok_or(EngineError::NoNode(k))
        }
        _ => st.doc.bounds_of(ids, false).ok_or_else(|| EngineError::Other("nothing to align".into())),
    }
}

fn align(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let r = reference_rect(s, p, &ids)?;
    let h = str_param(p, "horizontal");
    let v = str_param(p, "vertical");
    let key = s.doc()?.selection.key;
    let moves: Vec<(NodeId, Vec2)> = {
        let d = &s.doc()?.doc;
        ids.iter()
            .filter(|id| Some(**id) != key || str_param(p, "to") != Some("key"))
            .filter_map(|id| {
                let b = d.node(*id)?.geometric_bounds()?;
                let dx = match h {
                    Some("left") => r.x0 - b.x0,
                    Some("center") => r.center().x - b.center().x,
                    Some("right") => r.x1 - b.x1,
                    _ => 0.0,
                };
                let dy = match v {
                    Some("top") => r.y0 - b.y0,
                    Some("center") => r.center().y - b.center().y,
                    Some("bottom") => r.y1 - b.y1,
                    _ => 0.0,
                };
                Some((*id, Vec2::new(dx, dy)))
            })
            .collect()
    };
    s.edit("Align", |d, _| {
        for (id, dv) in &moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), false);
            }
        }
        Ok(())
    })?;
    ok()
}

fn distribute(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let d0 = &s.doc()?.doc;
    let mut items: Vec<(NodeId, Rect)> = ids.iter().filter_map(|id| Some((*id, d0.node(*id)?.geometric_bounds()?))).collect();
    let (horiz, key): (bool, fn(&Rect, bool) -> f64) = match (str_param(p, "horizontal"), str_param(p, "vertical")) {
        (Some(h), _) => (
            true,
            match h {
                "left" => |r: &Rect, _| r.x0,
                "right" => |r: &Rect, _| r.x1,
                _ => |r: &Rect, _| r.center().x,
            },
        ),
        (_, Some(v)) => (
            false,
            match v {
                "top" => |r: &Rect, _| r.y0,
                "bottom" => |r: &Rect, _| r.y1,
                _ => |r: &Rect, _| r.center().y,
            },
        ),
        _ => return Err(bad("object.distribute", "give horizontal or vertical")),
    };
    items.sort_by(|a, b| key(&a.1, horiz).total_cmp(&key(&b.1, horiz)));
    let n = items.len();
    if n < 3 {
        return ok();
    }
    let first = key(&items[0].1, horiz);
    let last = key(&items[n - 1].1, horiz);
    let moves: Vec<(NodeId, Vec2)> = items
        .iter()
        .enumerate()
        .map(|(i, (id, r))| {
            let target = first + (last - first) * i as f64 / (n - 1) as f64;
            let delta = target - key(r, horiz);
            (*id, if horiz { Vec2::new(delta, 0.0) } else { Vec2::new(0.0, delta) })
        })
        .collect();
    s.edit("Distribute", |d, _| {
        for (id, dv) in &moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), false);
            }
        }
        Ok(())
    })?;
    ok()
}

fn distribute_spacing(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let horiz = str_param(p, "axis") != Some("vertical");
    let d0 = &s.doc()?.doc;
    let mut items: Vec<(NodeId, Rect)> = ids.iter().filter_map(|id| Some((*id, d0.node(*id)?.geometric_bounds()?))).collect();
    items.sort_by(|a, b| if horiz { a.1.x0.total_cmp(&b.1.x0) } else { a.1.y0.total_cmp(&b.1.y0) });
    let n = items.len();
    let size = |r: &Rect| if horiz { r.width() } else { r.height() };
    let start = |r: &Rect| if horiz { r.x0 } else { r.y0 };
    let gap = match p.get("spacing").and_then(Value::as_f64) {
        Some(g) => g,
        None => {
            let span = if horiz { items[n - 1].1.x1 - items[0].1.x0 } else { items[n - 1].1.y1 - items[0].1.y0 };
            (span - items.iter().map(|i| size(&i.1)).sum::<f64>()) / (n - 1) as f64
        }
    };
    let mut pos = start(&items[0].1);
    let mut moves = vec![];
    for (id, r) in &items {
        let delta = pos - start(r);
        moves.push((*id, if horiz { Vec2::new(delta, 0.0) } else { Vec2::new(0.0, delta) }));
        pos += size(r) + gap;
    }
    s.edit("Distribute Spacing", |d, _| {
        for (id, dv) in &moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), false);
            }
        }
        Ok(())
    })?;
    ok()
}

fn set_bounds(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let b = s.doc()?.doc.bounds_of(&ids, false).ok_or_else(|| EngineError::Other("no bounds".into()))?;
    let refi = p.get("reference").and_then(Value::as_u64).unwrap_or(4) as usize;
    let rp = vectorcraft_geom::reference_point(b, refi);
    let mut w = f64_or(p, "width", b.width());
    let mut h = f64_or(p, "height", b.height());
    if bool_or(p, "proportional", false) {
        if p.get("width").is_some() && b.width() > 0.0 {
            h = b.height() * w / b.width();
        } else if p.get("height").is_some() && b.height() > 0.0 {
            w = b.width() * h / b.height();
        }
    }
    let sx = if b.width() > 1e-9 { w / b.width() } else { 1.0 };
    let sy = if b.height() > 1e-9 { h / b.height() } else { 1.0 };
    let scale = about(rp, Affine::scale_non_uniform(sx, sy));
    let new_rp = scale * rp;
    let tx = f64_or(p, "x", new_rp.x) - new_rp.x;
    let ty = f64_or(p, "y", new_rp.y) - new_rp.y;
    apply_transform(s, "Transform", ids, Affine::translate((tx, ty)) * scale, false)
}

fn expand_shape(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
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

fn set_live_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    s.edit("Live Shape", |d, _| {
        for id in &ids {
            let Some(NodeKind::Path { path, live: Some(live), .. }) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            match live {
                vectorcraft_doc::LiveShape::Rectangle { radii, .. } => {
                    if let Some(r) = p.get("radius").and_then(Value::as_f64) {
                        *radii = [r.max(0.0); 4];
                    }
                }
                vectorcraft_doc::LiveShape::Polygon { sides, .. } => {
                    if let Some(n) = p.get("sides").and_then(Value::as_u64) {
                        *sides = n.clamp(3, 1000) as u32;
                    }
                }
                _ => {}
            }
            *path = live.to_path();
        }
        Ok(())
    })?;
    ok()
}
