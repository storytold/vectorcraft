//! Live Blends (Object → Blend), Envelope Distort (Object → Envelope Distort) and Gradient Mesh
//! (Object → Create Gradient Mesh, Mesh tool edits).
//!
//! The objects are live document nodes (`NodeKind::Blend`, `NodeKind::Envelope`,
//! `NodeKind::Mesh`); evaluation lives in `vectorcraft_doc::live` and the renderer. **Expand**
//! replaces a live object by its evaluated geometry (`vectorcraft_render::expand_live`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::live::{self, BlendOrientation, BlendSpacing, BlendSpec, EnvelopeKind, GradientMesh, MeshAppearance, Spine};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Affine, PathData, Point};

use super::edit::selected_roots;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // ---------- Blend ----------
        cmd!(
            "object.blend.make",
            "Make",
            ["Object", "Blend"],
            Some("Cmd+Alt+B"),
            "{ids?, steps?: n | distance?: pt | smooth?: bool (default smooth colour), orientation?: page|path} blend the selected objects (paint order) into a live blend → {id}",
            has_doc,
            blend_make
        ),
        cmd!(
            "object.blend.release",
            "Release",
            ["Object", "Blend"],
            Some("Cmd+Alt+Shift+B"),
            "{} release the selected blends, keeping the key objects → {ids}",
            has_blend,
            blend_release
        ),
        cmd!(
            "object.blend.options",
            "Blend Options…",
            ["Object", "Blend"],
            None,
            "{spacing?: smooth|steps|distance, value?: n, steps?: n, distance?: pt, orientation?: page|path} set the options of the selected blends",
            has_blend,
            blend_options
        ),
        cmd!(
            "object.blend.expand",
            "Expand",
            ["Object", "Blend"],
            None,
            "{} replace the selected blends by groups of their steps → {ids}",
            has_blend,
            blend_expand
        ),
        cmd!(
            "object.blend.replaceSpine",
            "Replace Spine",
            ["Object", "Blend"],
            None,
            "{} use the selected path as the spine of the selected blend (the path is consumed)",
            has_blend,
            blend_replace_spine
        ),
        cmd!(
            "object.blend.reverseSpine",
            "Reverse Spine",
            ["Object", "Blend"],
            None,
            "{} reverse the order of the key objects along the spine",
            has_blend,
            blend_reverse_spine
        ),
        cmd!(
            "object.blend.reverseFrontToBack",
            "Reverse Front to Back",
            ["Object", "Blend"],
            None,
            "{} reverse the stacking order of the selected blends",
            has_blend,
            blend_reverse_stack
        ),
        // ---------- Envelope ----------
        cmd!(
            "object.envelope.makeWithWarp",
            "Make with Warp…",
            ["Object", "Envelope Distort"],
            Some("Cmd+Alt+Shift+W"),
            "{ids?, style?: arc|arcLower|…|twist (arc), bend?: % (50), h?: %, v?: %, horizontal?: bool (true)} envelope the selected objects with a warp → {id}",
            has_selection,
            env_make_warp
        ),
        cmd!(
            "object.envelope.makeWithMesh",
            "Make with Mesh…",
            ["Object", "Envelope Distort"],
            Some("Cmd+Alt+M"),
            "{ids?, rows?: n (4), cols?: n (4)} envelope the selected objects with a point mesh → {id}",
            has_selection,
            env_make_mesh
        ),
        cmd!(
            "object.envelope.makeWithTopObject",
            "Make with Top Object",
            ["Object", "Envelope Distort"],
            Some("Cmd+Alt+C"),
            "{ids?} the topmost selected path becomes the envelope of the others → {id}",
            has_multi,
            env_make_top
        ),
        cmd!(
            "object.envelope.release",
            "Release",
            ["Object", "Envelope Distort"],
            None,
            "{} release the selected envelopes → {ids}",
            has_envelope,
            env_release
        ),
        cmd!(
            "object.envelope.options",
            "Envelope Options…",
            ["Object", "Envelope Distort"],
            None,
            "{fidelity?: 0..100, style?, bend?, h?, v?, horizontal?} set envelope options (warp params for warp envelopes)",
            has_envelope,
            env_options
        ),
        cmd!(
            "object.envelope.expand",
            "Expand",
            ["Object", "Envelope Distort"],
            None,
            "{} replace the selected envelopes by their distorted content → {ids}",
            has_envelope,
            env_expand
        ),
        cmd!(
            "object.envelope.editContents",
            "Edit Contents",
            ["Object", "Envelope Distort"],
            Some("Cmd+Shift+V"),
            "{editing?: bool} toggle Edit Contents / Edit Envelope → {editing}",
            has_envelope,
            env_edit_contents
        ),
        cmd!(
            "object.envelope.setMeshPoint",
            "Move Envelope Mesh Point",
            [],
            None,
            "{id, index, x, y} move one point of a mesh envelope",
            has_doc,
            env_set_mesh_point
        ),
        // ---------- Gradient mesh ----------
        cmd!(
            "object.mesh.create",
            "Create Gradient Mesh…",
            ["Object"],
            None,
            "{ids?, rows?: n (4), cols?: n (4), appearance?: flat|center|edge, highlight?: % (100), at?: [x,y]} convert filled paths into gradient meshes (with `at`: 1×1 mesh plus lines through that point) → {ids, index?}",
            has_selection,
            mesh_create
        ),
        cmd!(
            "object.mesh.setPointColor",
            "Set Mesh Point Color",
            [],
            None,
            "{id, index, color, opacity?: 0..1} colour of one mesh point",
            has_doc,
            mesh_set_color
        ),
        cmd!(
            "object.mesh.movePoint",
            "Move Mesh Point",
            [],
            None,
            "{id, index, x, y, handle?: 0 right|1 left|2 down|3 up} move a mesh point (its handles follow), or with `handle` place that handle end at (x, y)",
            has_doc,
            mesh_move_point
        ),
        cmd!(
            "object.mesh.addLine",
            "Add Mesh Line",
            [],
            None,
            "{id, x, y, color?} add a mesh row and column through (x, y) → {index}",
            has_doc,
            mesh_add_line
        ),
        cmd!(
            "object.mesh.deletePoint",
            "Delete Mesh Point",
            [],
            None,
            "{id, index} delete the mesh lines through a point",
            has_doc,
            mesh_delete_point
        ),
        cmd!(
            "object.mesh.expand",
            "Expand Gradient Mesh",
            ["Object"],
            None,
            "{} replace the selected meshes by groups of flat-coloured pieces → {ids}",
            has_mesh,
            mesh_expand
        ),
    ]
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Explicit `ids` (in paint order) or the selected roots.
fn roots_param(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    if let Some(ids) = ids_param(p, "ids") {
        let st = s.doc()?;
        for id in &ids {
            if st.doc.node(*id).is_none() {
                return Err(EngineError::NoNode(*id));
            }
        }
        let mut sel = Selection::default();
        sel.set(ids.iter().copied().filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_layer())));
        let ordered = sel.in_paint_order(&st.doc);
        // Drop ids nested inside other given ids.
        return Ok(ordered
            .iter()
            .copied()
            .filter(|id| {
                let anc = st.doc.ancestry(*id).unwrap_or_default();
                !anc[..anc.len().saturating_sub(1)].iter().any(|a| ordered.contains(a))
            })
            .collect());
    }
    selected_roots(s)
}

/// Selected nodes of a kind (the selection itself or an ancestor).
fn selected_of(s: &Session, pred: fn(&Node) -> bool) -> Vec<NodeId> {
    let Some(st) = s.active() else { return vec![] };
    let mut out = vec![];
    for id in &st.selection.objects {
        for a in st.doc.ancestry(*id).unwrap_or_default().into_iter().rev() {
            if st.doc.node(a).is_some_and(pred) {
                if !out.contains(&a) {
                    out.push(a);
                }
                break;
            }
        }
    }
    out
}

fn is_blend(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Blend { .. })
}
fn is_envelope(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Envelope { .. })
}
fn is_mesh(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Mesh(_))
}

fn has_blend(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_of(s, is_blend).is_empty() { Err("select a blend".into()) } else { Ok(()) }
}
fn has_envelope(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_of(s, is_envelope).is_empty() { Err("select an envelope".into()) } else { Ok(()) }
}
fn has_mesh(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_of(s, is_mesh).is_empty() { Err("select a gradient mesh".into()) } else { Ok(()) }
}

/// Wrap `roots` (paint order) into a new node built by `make`, placed where the topmost root was.
fn wrap(d: &mut Document, sel: &mut Selection, roots: &[NodeId], make: impl FnOnce(NodeId, Vec<Arc<Node>>) -> Node) -> Result<NodeId> {
    let top = *roots.last().ok_or_else(|| EngineError::Other("nothing to wrap".into()))?;
    let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
    let nodes: Vec<Arc<Node>> = roots.iter().filter_map(|id| d.node(*id).cloned()).map(Arc::new).collect();
    let id = d.alloc_id();
    d.insert(par, idx + 1, make(id, nodes))?;
    for r in roots {
        d.remove(*r)?;
    }
    sel.set([id]);
    Ok(id)
}

/// Put `nodes` where `id` was (replacing it). Returns their ids.
fn replace_with(d: &mut Document, id: NodeId, nodes: Vec<Node>) -> Result<Vec<NodeId>> {
    let (par, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    d.remove(id)?;
    let mut out = vec![];
    for (k, n) in nodes.into_iter().enumerate() {
        out.push(n.id);
        d.insert(par, idx + k, n)?;
    }
    Ok(out)
}

/// Fresh ids for any id-0 (generated) node in `n`'s subtree; keeps real ids.
fn fix_ids(d: &mut Document, n: &mut Node) {
    if n.id == NodeId(0) {
        n.id = d.alloc_id();
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            fix_ids(d, Arc::make_mut(c));
        }
    }
}

// ---------- Blend ----------

fn spacing_param(p: &Value, cmd: &str, current: BlendSpacing) -> Result<BlendSpacing> {
    let num = |k: &str| p.get(k).and_then(Value::as_f64);
    let mode = str_param(p, "spacing");
    let value = num("value");
    let sp = match mode {
        Some("smooth") | Some("smoothColor") => BlendSpacing::SmoothColor,
        Some("steps") => BlendSpacing::Steps(value.or(num("steps")).unwrap_or(5.0) as u32),
        Some("distance") => BlendSpacing::Distance(value.or(num("distance")).unwrap_or(10.0)),
        Some(o) => return Err(bad(cmd, format!("unknown spacing `{o}` (smooth|steps|distance)"))),
        None => {
            if let Some(n) = num("steps") {
                BlendSpacing::Steps(n as u32)
            } else if let Some(d) = num("distance") {
                BlendSpacing::Distance(d)
            } else if bool_or(p, "smooth", false) {
                BlendSpacing::SmoothColor
            } else {
                current
            }
        }
    };
    match sp {
        BlendSpacing::Steps(n) if !(1..=1000).contains(&n) => Err(bad(cmd, "steps must be 1..1000")),
        BlendSpacing::Distance(d) if !(d.is_finite() && d > 0.0) => Err(bad(cmd, "distance must be > 0")),
        s => Ok(s),
    }
}

fn orientation_param(p: &Value, current: BlendOrientation) -> BlendOrientation {
    match str_param(p, "orientation") {
        Some("path") | Some("alignToPath") => BlendOrientation::AlignToPath,
        Some("page") | Some("alignToPage") => BlendOrientation::AlignToPage,
        _ => current,
    }
}

fn blend_make(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.make";
    let spacing = spacing_param(p, C, BlendSpacing::SmoothColor)?;
    let orientation = orientation_param(p, BlendOrientation::AlignToPage);
    let roots = roots_param(s, p)?;
    if roots.len() < 2 {
        return Err(bad(C, "select at least two objects"));
    }
    let id = s.edit("Make Blend", |d, sel| {
        wrap(d, sel, &roots, |id, children| Node::new(id, NodeKind::Blend { children, spec: BlendSpec { spacing, orientation, spine: None } }))
    })?;
    Ok(json!({ "id": id.0 }))
}

fn blend_release(s: &mut Session, _: &Value) -> Result<Value> {
    let blends = selected_of(s, is_blend);
    let ids = s.edit("Release Blend", |d, sel| {
        let mut out = vec![];
        for b in &blends {
            let Some(n) = d.node(*b).cloned() else { continue };
            let keys: Vec<Node> = n.children().into_iter().flatten().map(|c| (**c).clone()).collect();
            out.extend(replace_with(d, *b, keys)?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
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

fn blend_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.options";
    let blends = selected_of(s, is_blend);
    let cur = match blends.first().and_then(|b| s.doc().ok()?.doc.node(*b)).map(|n| &n.kind) {
        Some(NodeKind::Blend { spec, .. }) => spec.clone(),
        _ => BlendSpec::default(),
    };
    let spacing = spacing_param(p, C, cur.spacing)?;
    let orientation = orientation_param(p, cur.orientation);
    edit_blends(s, "Blend Options", |_, spec| {
        spec.spacing = spacing;
        spec.orientation = orientation;
    })
}

fn blend_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let blends = selected_of(s, is_blend);
    let ids = s.edit("Expand Blend", |d, sel| {
        let mut out = vec![];
        for b in &blends {
            let Some(n) = d.node(*b).cloned() else { continue };
            let children = vectorcraft_render::expand_live(&n).into_iter().map(Arc::new).collect();
            let mut g = Node::new(n.id, NodeKind::Group { children, clip: false });
            g.opacity = n.opacity;
            g.blend = n.blend;
            g.visible = n.visible;
            fix_ids(d, &mut g);
            out.extend(replace_with(d, *b, vec![g])?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

/// Move the keys onto their spine positions (so what's stored matches what's drawn).
fn keys_to_spine(children: &mut [Arc<Node>], spec: &BlendSpec) {
    let Some(sp) = spec.spine.as_ref().and_then(Spine::new) else { return };
    let k = children.len();
    for (i, c) in children.iter_mut().enumerate() {
        let f = if k > 1 { i as f64 / (k - 1) as f64 } else { 0.0 };
        let (pt, _) = sp.at(f);
        if let Some(b) = c.geometric_bounds() {
            Arc::make_mut(c).transform(Affine::translate(pt - b.center()), false);
        }
    }
}

fn blend_replace_spine(s: &mut Session, _: &Value) -> Result<Value> {
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
    let b = *blends.first().ok_or_else(|| EngineError::Other("Replace Spine: select a blend and a path".into()))?;
    s.edit("Replace Spine", |d, sel| {
        let spine = d.node(path).and_then(|n| n.path_data().cloned()).ok_or(EngineError::NoNode(path))?;
        d.remove(path)?;
        if let Some(NodeKind::Blend { children, spec }) = d.node_mut(b).map(|n| &mut n.kind) {
            spec.spine = Some(spine);
            keys_to_spine(children, spec);
        }
        sel.set([b]);
        Ok(())
    })?;
    ok()
}

fn blend_reverse_spine(s: &mut Session, _: &Value) -> Result<Value> {
    edit_blends(s, "Reverse Spine", |children, spec| {
        if let Some(sp) = &mut spec.spine {
            sp.reverse();
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

fn blend_reverse_stack(s: &mut Session, _: &Value) -> Result<Value> {
    edit_blends(s, "Reverse Front to Back", |children, spec| {
        children.reverse();
        if let Some(sp) = &mut spec.spine {
            sp.reverse();
        }
    })
}

// ---------- Envelope ----------

fn warp_kind(p: &Value, cmd: &str, cur: Option<&EnvelopeKind>) -> Result<EnvelopeKind> {
    let (mut style, mut bend, mut h, mut v, mut horizontal) = ("arc".to_string(), 50.0, 0.0, 0.0, true);
    if let Some(EnvelopeKind::Warp { style: s0, bend: b0, h: h0, v: v0, horizontal: z0 }) = cur {
        (style, bend, h, v, horizontal) = (s0.clone(), *b0, *h0, *v0, *z0);
    }
    if let Some(s) = str_param(p, "style") {
        let s = s.strip_prefix("warp.").unwrap_or(s);
        if live::WarpStyle::from_id(s).is_none() {
            return Err(bad(cmd, format!("unknown warp style `{s}`")));
        }
        style = s.to_string();
    }
    bend = f64_or(p, "bend", bend).clamp(-100.0, 100.0);
    h = f64_or(p, "h", f64_or(p, "horizontalDistortion", h)).clamp(-100.0, 100.0);
    v = f64_or(p, "v", f64_or(p, "verticalDistortion", v)).clamp(-100.0, 100.0);
    horizontal = match str_param(p, "orientation") {
        Some("vertical") => false,
        Some("horizontal") => true,
        _ => bool_or(p, "horizontal", horizontal),
    };
    Ok(EnvelopeKind::Warp { style, bend, h, v, horizontal })
}

fn envelope(id: NodeId, content: Vec<Arc<Node>>, kind: EnvelopeKind) -> Node {
    Node::new(id, NodeKind::Envelope { content, kind, fidelity: live::default_fidelity(), editing: false })
}

fn env_make_warp(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = warp_kind(p, "object.envelope.makeWithWarp", None)?;
    let roots = roots_param(s, p)?;
    if roots.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let id = s.edit("Make Envelope", |d, sel| wrap(d, sel, &roots, |id, content| envelope(id, content, kind)))?;
    Ok(json!({ "id": id.0 }))
}

fn env_make_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.makeWithMesh";
    let rows = f64_or(p, "rows", 4.0);
    let cols = f64_or(p, "cols", f64_or(p, "columns", 4.0));
    if !(1.0..=50.0).contains(&rows) || !(1.0..=50.0).contains(&cols) {
        return Err(bad(C, "rows and cols must be 1..50"));
    }
    let roots = roots_param(s, p)?;
    let st = s.doc()?;
    let src = roots
        .iter()
        .filter_map(|id| st.doc.node(*id))
        .fold(None, |a, n| vectorcraft_geom::union_opt(a, n.geometric_bounds()))
        .ok_or_else(|| bad(C, "selection has no bounds"))?;
    let kind = EnvelopeKind::Mesh { rows: rows as u32, cols: cols as u32, points: live::grid_points(src, rows as u32, cols as u32) };
    let id = s.edit("Make Envelope", |d, sel| wrap(d, sel, &roots, |id, content| envelope(id, content, kind)))?;
    Ok(json!({ "id": id.0 }))
}

/// Path data of a path or compound path.
fn outline_of(n: &Node) -> Option<PathData> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(path.clone()),
        NodeKind::Compound { children, .. } => {
            Some(PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.clone()).collect()))
        }
        _ => None,
    }
}

fn env_make_top(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.makeWithTopObject";
    let roots = roots_param(s, p)?;
    if roots.len() < 2 {
        return Err(bad(C, "select the content and a path on top"));
    }
    let top = *roots.last().ok_or_else(|| bad(C, "select the content and a path on top"))?;
    let path = s.doc()?.doc.node(top).and_then(outline_of).filter(|p| p.bounds().is_some()).ok_or_else(|| bad(C, "the top object must be a path"))?;
    let content = roots[..roots.len() - 1].to_vec();
    let id = s.edit("Make Envelope", |d, sel| {
        let id = wrap(d, sel, &content, |id, content| envelope(id, content, EnvelopeKind::TopObject { path }))?;
        d.remove(top)?;
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn env_release(s: &mut Session, _: &Value) -> Result<Value> {
    let envs = selected_of(s, is_envelope);
    let ids = s.edit("Release Envelope", |d, sel| {
        let mut out = vec![];
        for e in &envs {
            let Some(n) = d.node(*e).cloned() else { continue };
            let NodeKind::Envelope { content, kind, .. } = &n.kind else { continue };
            let mut nodes: Vec<Node> = content.iter().map(|c| (**c).clone()).collect();
            // The envelope shape comes back as a grey path above the content.
            let shape = match kind {
                EnvelopeKind::TopObject { path } => Some(path.clone()),
                EnvelopeKind::Mesh { .. } | EnvelopeKind::Warp { .. } => None,
            };
            if let Some(path) = shape {
                let id = d.alloc_id();
                nodes.push(Node::path(id, path, Appearance::basic(Paint::solid(Color::gray(0.25)), Paint::None, 0.0)));
            }
            out.extend(replace_with(d, *e, nodes)?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

fn env_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.options";
    let envs = selected_of(s, is_envelope);
    let fid = p.get("fidelity").and_then(Value::as_f64);
    if fid.is_some_and(|f| !(0.0..=100.0).contains(&f)) {
        return Err(bad(C, "fidelity must be 0..100"));
    }
    // Validate warp params up front.
    let doc = &s.doc()?.doc;
    let mut kinds = vec![];
    for e in &envs {
        if let Some(NodeKind::Envelope { kind: k @ EnvelopeKind::Warp { .. }, .. }) = doc.node(*e).map(|n| &n.kind) {
            kinds.push((*e, warp_kind(p, C, Some(k))?));
        }
    }
    s.edit("Envelope Options", |d, _| {
        for e in &envs {
            if let Some(NodeKind::Envelope { fidelity, kind, .. }) = d.node_mut(*e).map(|n| &mut n.kind) {
                if let Some(f) = fid {
                    *fidelity = f;
                }
                if let Some((_, k)) = kinds.iter().find(|(id, _)| id == e) {
                    *kind = k.clone();
                }
            }
        }
        Ok(())
    })?;
    ok()
}

fn env_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let envs = selected_of(s, is_envelope);
    let ids = s.edit("Expand Envelope", |d, sel| {
        let mut out = vec![];
        for e in &envs {
            let Some(n) = d.node(*e).cloned() else { continue };
            let children = vectorcraft_render::expand_live(&n).into_iter().map(Arc::new).collect();
            let mut g = Node::new(n.id, NodeKind::Group { children, clip: false });
            g.opacity = n.opacity;
            g.blend = n.blend;
            fix_ids(d, &mut g);
            out.extend(replace_with(d, *e, vec![g])?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

fn env_edit_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let envs = selected_of(s, is_envelope);
    let cur = matches!(envs.first().and_then(|e| s.doc().ok()?.doc.node(*e)).map(|n| &n.kind), Some(NodeKind::Envelope { editing: true, .. }));
    let want = bool_or(p, "editing", !cur);
    s.edit(if want { "Edit Contents" } else { "Edit Envelope" }, |d, sel| {
        let mut content_ids = vec![];
        for e in &envs {
            if let Some(n) = d.node_mut(*e)
                && let NodeKind::Envelope { editing, content, .. } = &mut n.kind
            {
                *editing = want;
                content_ids.extend(content.iter().map(|c| c.id));
            }
        }
        if want {
            sel.set(content_ids);
        } else {
            sel.set(envs.iter().copied());
        }
        Ok(())
    })?;
    Ok(json!({ "editing": want }))
}

fn env_set_mesh_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.setMeshPoint";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    let index = f64_req(p, "index", C)? as usize;
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    s.edit("Move Envelope Point", |d, _| {
        match d.node_mut(id).map(|n| &mut n.kind) {
            Some(NodeKind::Envelope { kind: EnvelopeKind::Mesh { points, .. }, .. }) if index < points.len() => points[index] = q,
            _ => return Err(bad(C, "not a mesh envelope point")),
        }
        Ok(())
    })?;
    ok()
}

// ---------- Gradient mesh ----------

fn mesh_create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.create";
    let rows = f64_or(p, "rows", 4.0);
    let cols = f64_or(p, "cols", f64_or(p, "columns", 4.0));
    if !(1.0..=50.0).contains(&rows) || !(1.0..=50.0).contains(&cols) {
        return Err(bad(C, "rows and cols must be 1..50"));
    }
    let app = match str_param(p, "appearance") {
        Some(a) => MeshAppearance::parse(a).ok_or_else(|| bad(C, "appearance must be flat|center|edge"))?,
        None => MeshAppearance::Flat,
    };
    let highlight = f64_or(p, "highlight", 100.0).clamp(0.0, 100.0);
    let at = point_param(p, "at");
    let (rows, cols) = if at.is_some() { (1, 1) } else { (rows as u32, cols as u32) };
    let roots = roots_param(s, p)?;
    let (ids, index) = s.edit("Create Gradient Mesh", |d, sel| {
        let mut out = vec![];
        let mut index = None;
        for id in &roots {
            let Some(n) = d.node(*id).cloned() else { continue };
            let Some(path) = outline_of(&n) else { continue };
            let base = match n.appearance.fill_paint() {
                Paint::Solid { color, .. } => color,
                Paint::Gradient(g) => g.gradient.sample(0.5).0,
                _ => Color::WHITE,
            };
            let Some(mut m) = GradientMesh::for_path(&path, rows, cols, base, app, highlight) else { continue };
            if let Some(q) = at {
                index = m.add_lines_at(q);
            }
            let mut mn = Node::new(n.id, NodeKind::Mesh(m));
            mn.name = n.name.clone();
            mn.opacity = n.opacity;
            mn.blend = n.blend;
            mn.locked = n.locked;
            mn.visible = n.visible;
            *d.node_mut(*id).ok_or(EngineError::NoNode(*id))? = mn;
            out.push(*id);
        }
        if out.is_empty() {
            return Err(bad(C, "select a path or compound path"));
        }
        sel.set(out.iter().copied());
        Ok((out, index))
    })?;
    let mut r = ids_json(&ids);
    if let Some(i) = index {
        r["index"] = json!(i);
    }
    Ok(r)
}

fn with_mesh<T>(s: &mut Session, p: &Value, cmd: &str, label: &str, f: impl FnOnce(&mut GradientMesh) -> Result<T>) -> Result<T> {
    let id = id_param(p, "id").ok_or_else(|| bad(cmd, "missing `id`"))?;
    let c = cmd.to_string();
    s.edit(label, move |d, _| match d.node_mut(id).map(|n| &mut n.kind) {
        Some(NodeKind::Mesh(m)) => f(m),
        Some(_) => Err(bad(&c, "not a gradient mesh")),
        None => Err(EngineError::NoNode(id)),
    })
}

fn index_param(p: &Value, cmd: &str) -> Result<usize> {
    let i = f64_req(p, "index", cmd)?;
    if i < 0.0 {
        return Err(bad(cmd, "index must be ≥ 0"));
    }
    Ok(i as usize)
}

fn mesh_set_color(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.setPointColor";
    let index = index_param(p, C)?;
    let color = p.get("color").and_then(color_value);
    let opacity = p.get("opacity").and_then(Value::as_f64);
    if color.is_none() && opacity.is_none() {
        return Err(bad(C, "missing `color` or `opacity`"));
    }
    with_mesh(s, p, C, "Mesh Point Color", |m| {
        let pt = m.points.get_mut(index).ok_or_else(|| bad(C, "index out of range"))?;
        if let Some(c) = color {
            pt.color = c;
        }
        if let Some(o) = opacity {
            pt.opacity = o.clamp(0.0, 1.0) as f32;
        }
        Ok(())
    })?;
    ok()
}

fn mesh_move_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.movePoint";
    let index = index_param(p, C)?;
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let handle = p.get("handle").and_then(Value::as_u64).map(|h| h as usize);
    if handle.is_some_and(|h| h > 3) {
        return Err(bad(C, "handle must be 0..3"));
    }
    with_mesh(s, p, C, "Move Mesh Point", |m| {
        let pt = m.points.get_mut(index).ok_or_else(|| bad(C, "index out of range"))?;
        match handle {
            Some(h) => pt.handles[h] = q - pt.p,
            None => pt.p = q,
        }
        Ok(())
    })?;
    ok()
}

fn mesh_add_line(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.addLine";
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let color = p.get("color").and_then(color_value);
    let index = with_mesh(s, p, C, "Add Mesh Line", |m| {
        let i = m.add_lines_at(q).ok_or_else(|| bad(C, "point is outside the mesh"))?;
        if let Some(c) = color {
            m.points[i].color = c;
        }
        Ok(i)
    })?;
    Ok(json!({ "index": index }))
}

fn mesh_delete_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.deletePoint";
    let index = index_param(p, C)?;
    with_mesh(s, p, C, "Delete Mesh Point", |m| {
        if m.remove_point_lines(index) { Ok(()) } else { Err(bad(C, "only interior mesh lines can be deleted")) }
    })?;
    ok()
}

fn mesh_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let meshes = selected_of(s, is_mesh);
    let ids = s.edit("Expand Gradient Mesh", |d, sel| {
        let mut out = vec![];
        for m in &meshes {
            let Some(n) = d.node(*m).cloned() else { continue };
            let mut g = live::expanded_group(&n, None);
            fix_ids(d, &mut g);
            out.extend(replace_with(d, *m, vec![g])?);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}
