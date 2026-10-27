use super::common::{fix_ids, ids_json, is_envelope, roots_param, selected_of, wrap};
use super::mesh::{handle_param, index_param, whole};
use super::*;

// ---------- Envelope ----------

/// The number keys of a warp: `bend`, and the two distortions by their short and long names.
const WARP_NUMBERS: &[&str] = &["bend", "h", "v", "horizontalDistortion", "verticalDistortion"];

fn warp_kind(p: &Value, cmd: &str, cur: Option<&EnvelopeKind>) -> Result<EnvelopeKind> {
    check_numbers(p, WARP_NUMBERS, cmd)?;
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
    let given = switch_or(p, "horizontal", horizontal, cmd)?;
    horizontal = match str_param(p, "orientation") {
        Some("vertical") => false,
        Some("horizontal") => true,
        _ => given,
    };
    Ok(EnvelopeKind::Warp { style, bend, h, v, horizontal })
}

/// The options and fidelity new envelopes get (Envelope Options with no envelope selected).
fn envelope_defaults(s: &Session) -> (EnvelopeOptions, f64) {
    s.envelope_defaults.unwrap_or((EnvelopeOptions::NEW, live::default_fidelity()))
}

fn envelope(id: NodeId, content: Vec<Arc<Node>>, kind: EnvelopeKind, (options, fidelity): (EnvelopeOptions, f64)) -> Node {
    Node::new(id, NodeKind::Envelope { content, kind: Box::new(kind), fidelity, editing: false, options, frame: Box::new(Affine::IDENTITY) })
}

/// `rows` and `cols` (or `columns`), whole numbers 1..50 each, defaulting to `default`.
pub(super) fn rows_cols(p: &Value, cmd: &str, default: (u32, u32)) -> Result<(u32, u32)> {
    check_numbers(p, &["rows", "cols", "columns"], cmd)?;
    let rows = f64_or(p, "rows", default.0 as f64);
    let cols_key = if p.get("cols").and_then(Value::as_f64).is_some() { "cols" } else { "columns" };
    let cols = f64_or(p, cols_key, default.1 as f64);
    if !(1.0..=50.0).contains(&rows) || !(1.0..=50.0).contains(&cols) {
        return Err(bad(cmd, "rows and cols must be 1..50"));
    }
    Ok((whole(rows, "rows", cmd)? as u32, whole(cols, cols_key, cmd)? as u32))
}

pub(super) fn env_make_warp(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn env_make_mesh(s: &mut Session, p: &Value) -> Result<Value> {
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
pub(super) fn outline_of(n: &Node) -> Option<PathData> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(path.clone()),
        NodeKind::Compound { children, .. } => {
            Some(PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.clone()).collect()))
        }
        _ => None,
    }
}

pub(super) fn env_make_top(s: &mut Session, p: &Value) -> Result<Value> {
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
                **kind = k;
            }
        }
        Ok(())
    })?;
    ok()
}

pub(super) fn env_reset_warp(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.resetWithWarp";
    let kinds = reshaped(s, |_, k| warp_kind(p, C, Some(k)))?;
    set_kinds(s, "Reset with Warp", kinds)
}

pub(super) fn env_reset_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.resetWithMesh";
    let maintain = switch_or(p, "maintainShape", true, C)?;
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
        anti_alias: switch_or(p, "antiAlias", cur.anti_alias, cmd)?,
        preserve_shape,
        distort_appearance: switch_or(p, "distortAppearance", cur.distort_appearance, cmd)?,
        distort_linear_gradients: switch_or(p, "distortLinearGradients", cur.distort_linear_gradients, cmd)?,
        distort_pattern_fills: switch_or(p, "distortPatternFills", cur.distort_pattern_fills, cmd)?,
    })
}

pub(super) fn env_info(s: &mut Session, _: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let env = selected_of(s, is_envelope).into_iter().find_map(|e| Some((e, &doc.node(e)?.kind)));
    let Some((id, NodeKind::Envelope { kind, fidelity, editing, options, .. })) = env else {
        let (options, fidelity) = envelope_defaults(s);
        return Ok(envelope_options_json(json!({ "id": null }), &options, fidelity));
    };
    let v = match &**kind {
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
        n.extras_mut().mask = env.extras().mask.clone();
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

pub(super) fn env_release(s: &mut Session, _: &Value) -> Result<Value> {
    let envs = selected_of(s, is_envelope);
    let ids = s.edit("Release Envelope", |d, sel| {
        let mut out = vec![];
        let mut released = Beside::replacing(d, &envs);
        for e in &envs {
            let Some(n) = d.node(*e).cloned() else { continue };
            let NodeKind::Envelope { content, kind, .. } = &n.kind else { continue };
            let mut nodes = carry_transparency(d, &n, content.iter().map(|c| (**c).clone()).collect());
            if let Some(mut shape) = envelope_shape(&n, kind) {
                shape.id = d.alloc_id();
                nodes.push(shape);
            }
            out.extend(nodes.iter().map(|c| c.id));
            released.put(d, *e, nodes)?;
        }
        released.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

pub(super) fn env_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.options";
    check_numbers(p, &["fidelity"], C)?;
    check_numbers(p, WARP_NUMBERS, C)?;
    check_switches(p, &["horizontal"], C)?;
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
            let kind = match &**kind {
                k @ EnvelopeKind::Warp { .. } => warp_kind(p, C, Some(k))?,
                k => k.clone(),
            };
            updates.push((*e, kind, options_param(p, C, *options)?, fid.unwrap_or(*fidelity)));
        }
    }
    s.edit("Envelope Options", |d, _| {
        for (e, k, o, f) in updates {
            if let Some(NodeKind::Envelope { kind, options, fidelity, .. }) = d.node_mut(e).map(|n| &mut n.kind) {
                (**kind, *options, *fidelity) = (k, o, f);
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
    // Outlined type's emoji take their pixels into the document.
    d.adopt_pixels(&mut g);
    // Outlined type repeats its object's id on its pieces.
    vectorcraft_render::effects::fresh_ids(d, &mut g, &mut Default::default());
    *d.node_mut(id).ok_or(EngineError::NoNode(id))? = g;
    Ok(())
}

/// Object → Expand: every envelope in `root`'s subtree (itself included) expanded
/// ([`expand_envelope`]; envelopes inside envelopes go with them). Returns how many.
pub(in super::super) fn expand_envelopes_under(d: &mut Document, root: NodeId) -> Result<usize> {
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

pub(super) fn env_expand(s: &mut Session, _: &Value) -> Result<Value> {
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

pub(super) fn env_edit_contents(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.editContents";
    let envs = selected_of(s, is_envelope);
    let cur = matches!(envs.first().and_then(|e| s.doc().ok()?.doc.node(*e)).map(|n| &n.kind), Some(NodeKind::Envelope { editing: true, .. }));
    let want = switch_or(p, "editing", !cur, C)?;
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

pub(super) fn env_set_mesh_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.envelope.setMeshPoint";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    let index = index_param(p, "index", C)?;
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let handle = handle_param(p, C)?;
    s.edit("Move Envelope Point", |d, _| {
        let Some(NodeKind::Envelope { kind, .. }) = d.node_mut(id).map(|n| &mut n.kind) else {
            return Err(bad(C, "not a mesh envelope point"));
        };
        let EnvelopeKind::Mesh { rows, cols, points, handles } = &mut **kind else {
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
