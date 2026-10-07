//! Live Blends (Object → Blend), Envelope Distort (Object → Envelope Distort) and Gradient Mesh
//! (Object → Create Gradient Mesh, Mesh tool edits).
//!
//! The objects are live document nodes (`NodeKind::Blend`, `NodeKind::Envelope`,
//! `NodeKind::Mesh`); evaluation lives in `vectorcraft_doc::live` and the renderer. **Expand**
//! replaces a live object by its evaluated geometry (`vectorcraft_render::expand_live`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::live::{
    self, BlendDefaults, BlendOrientation, BlendSpacing, BlendSpec, EnvelopeKind, EnvelopeMap, EnvelopeOptions, GradientMesh, MeshAppearance,
    PreserveShape, Spine,
};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Affine, PathData, Point};

use super::edit::selected_roots;
use super::expand::gradient_sampler;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // ---------- Blend ----------
        cmd!(
            "object.blend.make",
            "Make",
            ["Object", "Blend"],
            Some("Cmd+Alt+B"),
            "{ids?, steps?: n | distance?: pt | smooth?: bool, orientation?: page|path (default: the Blend Options set with nothing selected, else smooth colour and page), starts?: [anchor index | null, …] (per object of `ids`, else of the selection in paint order: the anchor of its first subpath the blend starts from; an open path's last anchor runs it the other way)} blend the selected objects (paint order) into a live blend; a blend among them takes the others in as more key objects (keeping its options, name and transparency) instead of nesting → {id}",
            has_doc,
            blend_make
        ),
        cmd!(
            "object.blend.release",
            "Release",
            ["Object", "Blend"],
            Some("Cmd+Alt+Shift+B"),
            "{} release the selected blends: the key objects come back and the spine stays as a path with no fill or stroke, below them; a blend with a name, opacity, blend mode, isolation, opacity mask or appearance of its own comes back as a group keeping them (and its knockout) → {ids (the keys), spines: [ids], groups: [ids]}",
            has_blend,
            blend_release
        ),
        cmd!(
            "object.blend.options",
            "Blend Options…",
            ["Object", "Blend"],
            None,
            "{spacing?: smooth|steps|distance, value?: n, steps?: n, distance?: pt, orientation?: page|path, easing?: linear|easeIn|easeOut|easeInOut (how the steps bunch between keys), strength?: 0..100 % (default 50), colorEasing?: same|linear|easeIn|easeOut|easeInOut (colour acceleration, independent of the spacing; same: follows the steps), colorStrength?: 0..100 %} set the options of the selected blends; with no blend selected, the options new blends start with (a tool setting, not an undo step) → {defaults?: true}",
            always,
            blend_options
        ),
        cmd!(
            "object.blend.expand",
            "Expand",
            ["Object", "Blend"],
            None,
            "{} replace the selected blends by groups of their keys and steps, each keeping the blend's id, name, transparency (knockout and isolation included), opacity mask and appearance → {ids}",
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
            query "object.blend.info",
            "Blend Info",
            [],
            None,
            "{} the options of the first selected blend, else those new blends start with → {target: blend|defaults, id?, spacing: smooth|steps|distance, steps, distance, orientation: page|path, keys?: [ids], starts?: [anchor|null], spine?: {anchors: [{x, y, in: [x, y], out: [x, y]}], closed, keyAnchors: [anchor each key sits on] | null (keys spread evenly: a spine from Replace Spine), explicit: false for the straight lines between the key centres}}",
            always,
            blend_info
        ),
        cmd!(
            "object.blend.spine.moveAnchor",
            "Move Spine Point",
            [],
            None,
            "{id? (default: the selected blend), anchor: index, x, y, handle?: in|out, independent?: bool} move a point of the blend's spine (its handles along), or with `handle` place that handle's end at (x, y) (a smooth point keeps the other handle in line unless `independent`). A key object sitting on the point moves with it. The first edit turns the straight spine into a path",
            has_doc,
            spine_move_anchor
        ),
        cmd!(
            "object.blend.spine.addAnchor",
            "Add Spine Point",
            [],
            None,
            "{id? (default: the selected blend), x, y} add a point to the blend's spine where it passes nearest (x, y) → {anchor}",
            has_doc,
            spine_add_anchor
        ),
        cmd!(
            "object.blend.spine.removeAnchor",
            "Delete Spine Point",
            [],
            None,
            "{id? (default: the selected blend), anchor: index} delete a point of the blend's spine that no key object sits on",
            has_doc,
            spine_remove_anchor
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
            "{ids?, style?: arc|arcLower|arcUpper|arch|bulge|shellLower|shellUpper|flag|wave|fish|rise|fisheye|inflate|squeeze|twist (arc), bend?: % -100..100 (50), h?: horizontal distortion % -100..100 (0), v?: vertical distortion % (0), horizontal?: bool (true) | orientation?: horizontal|vertical} envelope the selected objects with a warp (Envelope Options from object.envelope.options with nothing selected) → {id}",
            has_selection,
            env_make_warp
        ),
        cmd!(
            "object.envelope.makeWithMesh",
            "Make with Mesh…",
            ["Object", "Envelope Distort"],
            Some("Cmd+Alt+M"),
            "{ids?, rows?: 1..50 (4), cols?: 1..50 (4)} envelope the selected objects with a mesh of rows × cols patches → {id}",
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
            "object.envelope.resetWithWarp",
            "Reset with Warp…",
            ["Object", "Envelope Distort"],
            None,
            "{style?, bend?, h?, v?, horizontal? | orientation? (as makeWithWarp; unset: the envelope's current warp, else arc 50 %)} make the selected envelopes warp envelopes, keeping their content",
            has_envelope,
            env_reset_warp
        ),
        cmd!(
            "object.envelope.resetWithMesh",
            "Reset with Mesh…",
            ["Object", "Envelope Distort"],
            None,
            "{rows?: 1..50, cols?: 1..50 (unset: a mesh envelope's own, else 4), maintainShape?: bool (true: the mesh follows the current envelope shape; false: a flat grid over the content)} make the selected envelopes mesh envelopes, keeping their content",
            has_envelope,
            env_reset_mesh
        ),
        cmd!(
            query "object.envelope.info",
            "Envelope Info",
            [],
            None,
            "{} the selected envelope's settings, or with none selected the defaults new envelopes get → {id: n|null, type: warp|mesh|topObject, style, bend, h, v, horizontal, rows, cols, fidelity, antiAlias, preserveShape: clippingMask|transparency, distortAppearance, distortLinearGradients, distortPatternFills, editing}",
            has_doc,
            env_info
        ),
        cmd!(
            "object.envelope.release",
            "Release",
            ["Object", "Envelope Distort"],
            None,
            "{} release the selected envelopes into their content and their shape (a grey gradient mesh for warp and mesh envelopes, the path for a top-object envelope; the content keeps the envelope's opacity, blend mode and opacity mask) → {ids}",
            has_envelope,
            env_release
        ),
        cmd!(
            "object.envelope.options",
            "Envelope Options…",
            ["Object", "Envelope Distort"],
            None,
            "{fidelity?: 0..100, antiAlias?: bool, preserveShape?: clippingMask|transparency, distortAppearance?: bool, distortLinearGradients?: bool, distortPatternFills?: bool (the last two work with distortAppearance), style?, bend?, h?, v?, horizontal? (warp envelopes)} set the selected envelopes' options; with no envelope selected, the options new envelopes get (see object.envelope.info)",
            has_doc,
            env_options
        ),
        cmd!(
            "object.envelope.expand",
            "Expand",
            ["Object", "Envelope Distort"],
            None,
            "{} replace the selected envelopes by groups of their distorted content (type outlined), keeping name, opacity, blend mode, isolation, knockout and opacity mask → {ids}",
            has_envelope,
            env_expand
        ),
        cmd!(
            "object.envelope.editContents",
            "Edit Contents",
            ["Object", "Envelope Distort"],
            None,
            "{editing?: bool} toggle Edit Contents / Edit Envelope → {editing}",
            has_envelope,
            env_edit_contents
        ),
        cmd!(
            "object.envelope.setMeshPoint",
            "Move Envelope Mesh Point",
            [],
            None,
            "{id, index, x, y, handle?: 0 right|1 left|2 down|3 up} move a point of a mesh envelope (its handles follow), or with `handle` place that handle end at (x, y); object.mesh.movePoint, addLine and deletePoint edit mesh envelopes too",
            has_doc,
            env_set_mesh_point
        ),
        // ---------- Gradient mesh ----------
        cmd!(
            "object.mesh.create",
            "Create Gradient Mesh…",
            ["Object"],
            None,
            "{ids?, rows?: n (4), cols?: n (4), appearance?: flat|center|edge, highlight?: % (100), at?: [x,y]} convert filled paths into gradient meshes in their fill colour (a gradient fill: the colour it paints at each point), lightened per `appearance` (with `at`: 1×1 mesh plus lines through that point) → {ids, index?}",
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
            "{id, index, x, y, handle?: 0 right|1 left|2 down|3 up} move a point of a gradient mesh or mesh envelope (its handles follow), or with `handle` place that handle end at (x, y)",
            has_doc,
            mesh_move_point
        ),
        cmd!(
            "object.mesh.addLine",
            "Add Mesh Line",
            [],
            None,
            "{id, x, y, color?} add a row and a column through (x, y) to a gradient mesh (new point in `color`) or a mesh envelope → {index}",
            has_doc,
            mesh_add_line
        ),
        cmd!(
            "object.mesh.deletePoint",
            "Delete Mesh Point",
            [],
            None,
            "{id, index} delete the mesh lines through a point of a gradient mesh or mesh envelope",
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

/// The Specified Steps and Specified Distance a spacing mode starts with.
const DEFAULT_STEPS: u32 = 5;
const DEFAULT_DISTANCE: f64 = 10.0;

fn spacing_param(p: &Value, cmd: &str, current: BlendSpacing) -> Result<BlendSpacing> {
    let num = |k: &str| p.get(k).and_then(Value::as_f64);
    let mode = str_param(p, "spacing");
    let value = num("value");
    let sp = match mode {
        Some("smooth") | Some("smoothColor") => BlendSpacing::SmoothColor,
        Some("steps") => BlendSpacing::Steps(value.or(num("steps")).unwrap_or(DEFAULT_STEPS as f64) as u32),
        Some("distance") => BlendSpacing::Distance(value.or(num("distance")).unwrap_or(DEFAULT_DISTANCE)),
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

/// `starts`: per object of `ids` (else of `roots`), the anchor its blend starts from.
fn starts_param(p: &Value, roots: &[NodeId]) -> Result<Vec<(NodeId, u32)>> {
    const C: &str = "object.blend.make";
    let Some(v) = p.get("starts").filter(|v| !v.is_null()) else { return Ok(vec![]) };
    let list = v.as_array().ok_or_else(|| bad(C, "`starts` must be an array of anchor indices (or nulls)"))?;
    let owners = ids_param(p, "ids").unwrap_or_else(|| roots.to_vec());
    let mut out = vec![];
    for (id, a) in owners.iter().zip(list) {
        if a.is_null() {
            continue;
        }
        let i = a.as_u64().and_then(|i| u32::try_from(i).ok()).ok_or_else(|| bad(C, "a start must be an anchor index ≥ 0 or null"))?;
        out.push((*id, i));
    }
    Ok(out)
}

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

fn blend_make(s: &mut Session, p: &Value) -> Result<Value> {
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
            let mut spec = BlendSpec { spacing, orientation, starts, ..Default::default() };
            if let NodeKind::Blend { children: old, spec: old_spec } = &n.kind {
                extend_spine(&mut spec, old, old_spec, &children);
            }
            n.id = id;
            n.kind = NodeKind::Blend { children, spec };
            n
        })
    })?;
    Ok(json!({ "id": id.0 }))
}

fn blend_release(s: &mut Session, _: &Value) -> Result<Value> {
    let blends = selected_of(s, is_blend);
    let (ids, spines, groups) = s.edit("Release Blend", |d, sel| {
        let (mut keys, mut spines, mut groups) = (vec![], vec![], vec![]);
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
                || n.mask.is_some()
                || !n.appearance.items.is_empty()
                || !n.appearance.effects.is_empty()
            {
                let mut g = n.clone();
                g.kind = NodeKind::Group { children: nodes.into_iter().map(Arc::new).collect(), clip: false };
                groups.push(g.id);
                nodes = vec![g];
            }
            replace_with(d, *b, nodes)?;
        }
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

fn spine_move_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.spine.moveAnchor";
    let index = index_param(p, "anchor", C)?;
    let q = finite_point(p, C)?;
    let handle = match str_param(p, "handle") {
        None => None,
        Some("out") => Some(true),
        Some("in") => Some(false),
        Some(_) => return Err(bad(C, "handle must be in or out")),
    };
    let independent = bool_or(p, "independent", false);
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

fn spine_add_anchor(s: &mut Session, p: &Value) -> Result<Value> {
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

fn spine_remove_anchor(s: &mut Session, p: &Value) -> Result<Value> {
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
        NodeKind::Blend { spec, children } => Some((b, spec.clone(), children.iter().map(|c| c.id).collect())),
        _ => None,
    }
}

fn blend_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.blend.options";
    let Some((_, cur, _)) = selected_spec(s) else {
        // Nothing to set them on: the options new blends start with.
        let d = s.prefs.blend_options.unwrap_or_default();
        let spacing = spacing_param(p, C, d.spacing)?;
        s.prefs.blend_options = Some(BlendDefaults { spacing, orientation: orientation_param(p, d.orientation) });
        return Ok(json!({ "defaults": true }));
    };
    let spacing = spacing_param(p, C, cur.spacing)?;
    let orientation = orientation_param(p, cur.orientation);
    let easing = easing_param(p, C, "easing", "strength", cur.easing)?;
    let color_easing = match str_param(p, "colorEasing") {
        Some(v) if v.eq_ignore_ascii_case("same") || v.eq_ignore_ascii_case("spacing") => None,
        Some(_) => Some(easing_param(p, C, "colorEasing", "colorStrength", cur.color_easing.unwrap_or_default())?),
        None => match (cur.color_easing, p.get("colorStrength")) {
            (Some(e), Some(_)) => Some(easing_param(p, C, "colorEasing", "colorStrength", e)?),
            (e, _) => e,
        },
    };
    edit_blends(s, "Blend Options", |_, spec| {
        spec.spacing = spacing;
        spec.orientation = orientation;
        spec.easing = easing;
        spec.color_easing = color_easing;
    })
}

/// An easing from the `key` (curve name) and `strength_key` (0..100 %) params, else `cur`'s.
fn easing_param(p: &Value, c: &str, key: &str, strength_key: &str, cur: live::BlendEasing) -> Result<live::BlendEasing> {
    let ease = match str_param(p, key) {
        Some(v) => live::BlendEase::parse(v).ok_or_else(|| bad(c, format!("unknown {key} `{v}` (linear, easeIn, easeOut, easeInOut)")))?,
        None => cur.ease,
    };
    let strength = match p.get(strength_key) {
        Some(v) => v.as_f64().filter(|s| s.is_finite()).ok_or_else(|| bad(c, format!("{strength_key} must be a number 0..100")))?.clamp(0.0, 100.0),
        None => cur.strength,
    };
    Ok(live::BlendEasing { ease, strength })
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

fn blend_info(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(match selected_spec(s) {
        Some((id, spec, keys)) => {
            let mut v = blend_options_json(spec.spacing, spec.orientation);
            v["easing"] = json!(spec.easing.ease.name());
            v["strength"] = json!(spec.easing.strength);
            v["colorEasing"] = json!(spec.color_easing.map_or("same", |e| e.ease.name()));
            v["colorStrength"] = json!(spec.color_easing.map_or(spec.easing.strength, |e| e.strength));
            v["target"] = json!("blend");
            v["id"] = json!(id.0);
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

fn blend_expand(s: &mut Session, _: &Value) -> Result<Value> {
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

fn blend_reverse_spine(s: &mut Session, _: &Value) -> Result<Value> {
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

fn blend_reverse_stack(s: &mut Session, _: &Value) -> Result<Value> {
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

/// The options and fidelity new envelopes get (Envelope Options with no envelope selected).
fn envelope_defaults(s: &Session) -> (EnvelopeOptions, f64) {
    s.envelope_defaults.unwrap_or((EnvelopeOptions::NEW, live::default_fidelity()))
}

fn envelope(id: NodeId, content: Vec<Arc<Node>>, kind: EnvelopeKind, (options, fidelity): (EnvelopeOptions, f64)) -> Node {
    Node::new(id, NodeKind::Envelope { content, kind, fidelity, editing: false, options, frame: Affine::IDENTITY })
}

/// `rows` and `cols` (or `columns`), 1..50 each, defaulting to `default`.
fn rows_cols(p: &Value, cmd: &str, default: (u32, u32)) -> Result<(u32, u32)> {
    let rows = f64_or(p, "rows", default.0 as f64);
    let cols = f64_or(p, "cols", f64_or(p, "columns", default.1 as f64));
    if !(1.0..=50.0).contains(&rows) || !(1.0..=50.0).contains(&cols) {
        return Err(bad(cmd, "rows and cols must be 1..50"));
    }
    Ok((rows as u32, cols as u32))
}

fn env_make_warp(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = warp_kind(p, "object.envelope.makeWithWarp", None)?;
    let roots = roots_param(s, p)?;
    if roots.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let defaults = envelope_defaults(s);
    let id = s.edit("Make Envelope", |d, sel| wrap(d, sel, &roots, |id, content| envelope(id, content, kind, defaults)))?;
    Ok(json!({ "id": id.0 }))
}

/// A mesh envelope of `rows`×`cols` patches for an envelope with map `map`: following its
/// current surface (`maintain`), or a flat grid over its content.
fn mesh_kind(map: &EnvelopeMap, maintain: bool, rows: u32, cols: u32) -> EnvelopeKind {
    if !maintain {
        return EnvelopeKind::Mesh { rows, cols, points: map.grid(rows, cols), handles: vec![] };
    }
    // The surface's points with handles along it: the grid lines keep the current shape.
    let m = map.surface_mesh(rows, cols, Color::BLACK);
    EnvelopeKind::Mesh { rows, cols, points: m.points.iter().map(|q| q.p).collect(), handles: m.points.iter().map(|q| q.handles).collect() }
}

fn env_make_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.makeWithMesh";
    let (rows, cols) = rows_cols(p, C, (4, 4))?;
    let roots = roots_param(s, p)?;
    let st = s.doc()?;
    let nodes: Vec<Arc<Node>> = roots.iter().filter_map(|id| st.doc.node(*id).cloned()).map(Arc::new).collect();
    let src = live::nodes_bounds(&nodes).ok_or_else(|| bad(C, "selection has no bounds"))?;
    let kind = EnvelopeKind::Mesh { rows, cols, points: live::grid_points(src, rows, cols), handles: vec![] };
    let defaults = envelope_defaults(s);
    let id = s.edit("Make Envelope", |d, sel| wrap(d, sel, &roots, |id, content| envelope(id, content, kind, defaults)))?;
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
    let Some((&top, content)) = roots.split_last() else { return Err(bad(C, "select the content and a path on top")) };
    let content = content.to_vec();
    let path = s.doc()?.doc.node(top).and_then(outline_of).filter(|p| p.bounds().is_some()).ok_or_else(|| bad(C, "the top object must be a path"))?;
    let defaults = envelope_defaults(s);
    let id = s.edit("Make Envelope", |d, sel| {
        let id = wrap(d, sel, &content, |id, content| envelope(id, content, EnvelopeKind::TopObject { path }, defaults))?;
        d.remove(top)?;
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The kind `f` makes of each selected envelope (from the envelope and its current kind).
fn reshaped(s: &Session, f: impl Fn(&Node, &EnvelopeKind) -> Result<EnvelopeKind>) -> Result<Vec<(NodeId, EnvelopeKind)>> {
    let doc = &s.doc()?.doc;
    let mut out = vec![];
    for e in selected_of(s, is_envelope) {
        if let Some(n) = doc.node(e)
            && let NodeKind::Envelope { kind, .. } = &n.kind
        {
            out.push((e, f(n, kind)?));
        }
    }
    Ok(out)
}

/// Give each envelope of `kinds` its new kind (as one undo step `label`).
fn set_kinds(s: &mut Session, label: &str, kinds: Vec<(NodeId, EnvelopeKind)>) -> Result<Value> {
    s.edit(label, |d, _| {
        for (e, k) in kinds {
            if let Some(NodeKind::Envelope { kind, .. }) = d.node_mut(e).map(|n| &mut n.kind) {
                *kind = k;
            }
        }
        Ok(())
    })?;
    ok()
}

fn env_reset_warp(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.resetWithWarp";
    let kinds = reshaped(s, |_, k| warp_kind(p, C, Some(k)))?;
    set_kinds(s, "Reset with Warp", kinds)
}

fn env_reset_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.resetWithMesh";
    let maintain = bool_or(p, "maintainShape", true);
    let kinds = reshaped(s, |env, kind| {
        let own = match kind {
            EnvelopeKind::Mesh { rows, cols, .. } => (*rows, *cols),
            _ => (4, 4),
        };
        let (rows, cols) = rows_cols(p, C, own)?;
        let map = EnvelopeMap::of(env).ok_or_else(|| bad(C, "the envelope's content has no bounds"))?;
        Ok(mesh_kind(&map, maintain, rows, cols))
    })?;
    set_kinds(s, "Reset with Mesh", kinds)
}

/// `v` with an envelope's options and fidelity added.
fn envelope_options_json(mut v: Value, o: &EnvelopeOptions, fidelity: f64) -> Value {
    v["fidelity"] = json!(fidelity);
    v["antiAlias"] = json!(o.anti_alias);
    v["preserveShape"] = json!(o.preserve_shape.id());
    v["distortAppearance"] = json!(o.distort_appearance);
    v["distortLinearGradients"] = json!(o.distort_linear_gradients);
    v["distortPatternFills"] = json!(o.distort_pattern_fills);
    v
}

/// `cur` with the options `p` sets.
fn options_param(p: &Value, cmd: &str, cur: EnvelopeOptions) -> Result<EnvelopeOptions> {
    let preserve_shape = match str_param(p, "preserveShape") {
        Some(v) => PreserveShape::parse(v).ok_or_else(|| bad(cmd, "preserveShape must be clippingMask|transparency"))?,
        None => cur.preserve_shape,
    };
    Ok(EnvelopeOptions {
        anti_alias: bool_or(p, "antiAlias", cur.anti_alias),
        preserve_shape,
        distort_appearance: bool_or(p, "distortAppearance", cur.distort_appearance),
        distort_linear_gradients: bool_or(p, "distortLinearGradients", cur.distort_linear_gradients),
        distort_pattern_fills: bool_or(p, "distortPatternFills", cur.distort_pattern_fills),
    })
}

fn env_info(s: &mut Session, _: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let env = selected_of(s, is_envelope).into_iter().find_map(|e| Some((e, &doc.node(e)?.kind)));
    let Some((id, NodeKind::Envelope { kind, fidelity, editing, options, .. })) = env else {
        let (options, fidelity) = envelope_defaults(s);
        return Ok(envelope_options_json(json!({ "id": null }), &options, fidelity));
    };
    let v = match kind {
        EnvelopeKind::Warp { style, bend, h, v, horizontal } => {
            json!({"type": "warp", "style": style, "bend": bend, "h": h, "v": v, "horizontal": horizontal})
        }
        EnvelopeKind::Mesh { rows, cols, .. } => json!({"type": "mesh", "rows": rows, "cols": cols}),
        EnvelopeKind::TopObject { .. } => json!({"type": "topObject"}),
    };
    let mut v = envelope_options_json(v, options, *fidelity);
    v["id"] = json!(id.0);
    v["editing"] = json!(editing);
    Ok(v)
}

/// What Release gives back as the shape of envelope `env` (of `kind`): a top object's path, or the
/// surface of a warp or mesh envelope as a gradient mesh, painted grey (id 0).
fn envelope_shape(env: &Node, kind: &EnvelopeKind) -> Option<Node> {
    let grey = Color::gray(0.25);
    let (rows, cols) = match kind {
        EnvelopeKind::TopObject { path } => {
            return Some(Node::path(NodeId(0), path.clone(), Appearance::basic(Paint::solid(grey), Paint::None, 0.0)));
        }
        // The mesh itself, handles and all.
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            let mut m = live::envelope_grid(*rows, *cols, points, handles)?;
            m.points.iter_mut().for_each(|q| q.color = grey);
            return Some(Node::new(NodeId(0), NodeKind::Mesh(m)));
        }
        EnvelopeKind::Warp { .. } => (4, 4),
    };
    EnvelopeMap::of(env).map(|map| Node::new(NodeId(0), NodeKind::Mesh(map.surface_mesh(rows, cols, grey))))
}

/// Released content keeps envelope `env`'s opacity, blend mode, isolation, knockout and opacity
/// mask: a single object without its own takes them, else a new group around the content does.
fn carry_transparency(d: &mut Document, env: &Node, mut nodes: Vec<Node>) -> Vec<Node> {
    if env.has_default_transparency() {
        return nodes;
    }
    let carry = |n: &mut Node| {
        n.opacity = env.opacity;
        n.blend = env.blend;
        n.isolate = env.isolate;
        n.knockout = env.knockout;
        n.knockout_shape = env.knockout_shape;
        n.mask = env.mask.clone();
    };
    if let [one] = nodes.as_mut_slice()
        && one.has_default_transparency()
    {
        carry(one);
        return nodes;
    }
    let mut g = Node::group(d.alloc_id(), nodes.into_iter().map(Arc::new).collect());
    carry(&mut g);
    vec![g]
}

fn env_release(s: &mut Session, _: &Value) -> Result<Value> {
    let envs = selected_of(s, is_envelope);
    let ids = s.edit("Release Envelope", |d, sel| {
        let mut out = vec![];
        for e in &envs {
            let Some(n) = d.node(*e).cloned() else { continue };
            let NodeKind::Envelope { content, kind, .. } = &n.kind else { continue };
            let mut nodes = carry_transparency(d, &n, content.iter().map(|c| (**c).clone()).collect());
            if let Some(mut shape) = envelope_shape(&n, kind) {
                shape.id = d.alloc_id();
                nodes.push(shape);
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
    let fid = p.get("fidelity").and_then(Value::as_f64);
    if fid.is_some_and(|f| !(0.0..=100.0).contains(&f)) {
        return Err(bad(C, "fidelity must be 0..100"));
    }
    let envs = selected_of(s, is_envelope);
    if envs.is_empty() {
        // No envelope selected: the options new envelopes get.
        let (o, f) = envelope_defaults(s);
        s.envelope_defaults = Some((options_param(p, C, o)?, fid.unwrap_or(f)));
        return ok();
    }
    // Validate everything up front.
    let doc = &s.doc()?.doc;
    let mut updates = vec![];
    for e in &envs {
        if let Some(NodeKind::Envelope { kind, options, fidelity, .. }) = doc.node(*e).map(|n| &n.kind) {
            let kind = match kind {
                EnvelopeKind::Warp { .. } => warp_kind(p, C, Some(kind))?,
                k => k.clone(),
            };
            updates.push((*e, kind, options_param(p, C, *options)?, fid.unwrap_or(*fidelity)));
        }
    }
    s.edit("Envelope Options", |d, _| {
        for (e, k, o, f) in updates {
            if let Some(NodeKind::Envelope { kind, options, fidelity, .. }) = d.node_mut(e).map(|n| &mut n.kind) {
                (*kind, *options, *fidelity) = (k, o, f);
            }
        }
        Ok(())
    })?;
    ok()
}

/// Replace envelope `id` by a group of its distorted content (type outlined) with its id, name,
/// transparency and opacity mask; the generated pieces get ids of their own.
fn expand_envelope(d: &mut Document, id: NodeId) -> Result<()> {
    let n = d.node(id).cloned().ok_or(EngineError::NoNode(id))?;
    let mut g = vectorcraft_render::effects::expanded_live_group(Some(d), &n);
    fix_ids(d, &mut g);
    // Outlined type repeats its object's id on its pieces.
    vectorcraft_render::effects::fresh_ids(d, &mut g, &mut Default::default());
    *d.node_mut(id).ok_or(EngineError::NoNode(id))? = g;
    Ok(())
}

/// Object → Expand: every envelope in `root`'s subtree (itself included) expanded
/// ([`expand_envelope`]; envelopes inside envelopes go with them). Returns how many.
pub(super) fn expand_envelopes_under(d: &mut Document, root: NodeId) -> Result<usize> {
    fn collect(n: &Node, out: &mut Vec<NodeId>) {
        if is_envelope(n) {
            out.push(n.id);
        } else {
            for c in n.children().into_iter().flatten() {
                collect(c, out);
            }
        }
    }
    let mut envs = vec![];
    if let Some(n) = d.node(root) {
        collect(n, &mut envs);
    }
    for e in &envs {
        expand_envelope(d, *e)?;
    }
    Ok(envs.len())
}

fn env_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let envs = selected_of(s, is_envelope);
    let ids = s.edit("Expand Envelope", |d, sel| {
        for e in &envs {
            expand_envelope(d, *e)?;
        }
        sel.set(envs.iter().copied());
        Ok(envs.clone())
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
    let handle = handle_param(p, C)?;
    s.edit("Move Envelope Point", |d, _| {
        let Some(NodeKind::Envelope { kind: EnvelopeKind::Mesh { rows, cols, points, handles }, .. }) = d.node_mut(id).map(|n| &mut n.kind) else {
            return Err(bad(C, "not a mesh envelope point"));
        };
        let at = *points.get(index).ok_or_else(|| bad(C, "not a mesh envelope point"))?;
        match handle {
            // The handles are offsets: they follow.
            None => points.get_mut(index).into_iter().for_each(|p| *p = q),
            Some(h) => {
                // The first handle edited fills in the smooth mesh's handles.
                if handles.len() != points.len() {
                    *handles = live::smooth_handles(*rows, *cols, points);
                }
                if let Some(hs) = handles.get_mut(index) {
                    hs[h] = q - at;
                }
            }
        }
        Ok(())
    })?;
    ok()
}

// ---------- Gradient mesh ----------

fn mesh_create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.create";
    let (rows, cols) = rows_cols(p, C, (4, 4))?;
    let app = match str_param(p, "appearance") {
        Some(a) => MeshAppearance::parse(a).ok_or_else(|| bad(C, "appearance must be flat|center|edge"))?,
        None => MeshAppearance::Flat,
    };
    let highlight = f64_or(p, "highlight", 100.0).clamp(0.0, 100.0);
    let at = point_param(p, "at");
    let (rows, cols) = if at.is_some() { (1, 1) } else { (rows, cols) };
    let roots = roots_param(s, p)?;
    let (ids, index) = s.edit("Create Gradient Mesh", |d, sel| {
        let mut out = vec![];
        let mut index = None;
        for id in &roots {
            let Some(n) = d.node(*id).cloned() else { continue };
            let Some(path) = outline_of(&n) else { continue };
            let m = match n.appearance.fill_paint() {
                // The points take the colours the gradient paints there.
                Paint::Gradient(g) => {
                    path.bounds().and_then(|b| GradientMesh::for_path_with(&path, rows, cols, &*gradient_sampler(&g, b))).map(|mut m| {
                        m.highlight(app, highlight);
                        m
                    })
                }
                p => GradientMesh::for_path(&path, rows, cols, p.color().unwrap_or(Color::WHITE), app, highlight),
            };
            let Some(mut m) = m else { continue };
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

/// Run `f` on the gradient mesh `p.id` (one undo step `label`); with `envelopes`, a mesh
/// envelope's grid too (its points and handles are written back: editing fills in the handles).
fn with_mesh<T>(s: &mut Session, p: &Value, cmd: &str, label: &str, envelopes: bool, f: impl FnOnce(&mut GradientMesh) -> Result<T>) -> Result<T> {
    let id = id_param(p, "id").ok_or_else(|| bad(cmd, "missing `id`"))?;
    let c = cmd.to_string();
    s.edit(label, move |d, _| match d.node_mut(id).map(|n| &mut n.kind) {
        Some(NodeKind::Mesh(m)) => f(m),
        Some(NodeKind::Envelope { kind: EnvelopeKind::Mesh { rows, cols, points, handles }, .. }) if envelopes => {
            let mut m = live::envelope_grid(*rows, *cols, points, handles).ok_or_else(|| bad(&c, "the envelope's mesh is malformed"))?;
            let r = f(&mut m)?;
            (*rows, *cols) = (m.rows, m.cols);
            *points = m.points.iter().map(|q| q.p).collect();
            *handles = m.points.iter().map(|q| q.handles).collect();
            Ok(r)
        }
        Some(_) => Err(bad(&c, if envelopes { "not a gradient mesh or mesh envelope" } else { "not a gradient mesh" })),
        None => Err(EngineError::NoNode(id)),
    })
}

/// The `handle` parameter (0 right, 1 left, 2 down, 3 up), if any.
fn handle_param(p: &Value, cmd: &str) -> Result<Option<usize>> {
    match p.get("handle") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(h) if h <= 3 => Ok(Some(h as usize)),
            _ => Err(bad(cmd, "handle must be 0..3")),
        },
    }
}

fn index_param(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    let i = f64_req(p, key, cmd)?;
    if i < 0.0 {
        return Err(bad(cmd, "index must be ≥ 0"));
    }
    Ok(i as usize)
}

fn mesh_set_color(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.setPointColor";
    let index = index_param(p, "index", C)?;
    let color = p.get("color").and_then(color_value);
    let opacity = p.get("opacity").and_then(Value::as_f64);
    if color.is_none() && opacity.is_none() {
        return Err(bad(C, "missing `color` or `opacity`"));
    }
    with_mesh(s, p, C, "Mesh Point Color", false, |m| {
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
    let index = index_param(p, "index", C)?;
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let handle = handle_param(p, C)?;
    with_mesh(s, p, C, "Move Mesh Point", true, |m| {
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
    let index = with_mesh(s, p, C, "Add Mesh Line", true, |m| {
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
    let index = index_param(p, "index", C)?;
    with_mesh(s, p, C, "Delete Mesh Point", true, |m| {
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
