use std::collections::{BTreeMap, BTreeSet};

use super::common::{fix_ids, ids_json, is_mesh, replace_with, roots_param, selected_of};
use super::envelope::{outline_of, rows_cols};
use super::{
    Color, Document, EngineError, EnvelopeKind, GradientMesh, MeshAppearance, Node, NodeId, NodeKind, Paint, Point, Result, Session, Value, bad,
    check_numbers, color_param, f64_or, f64_req, finite_numbers, gradient_sampler, json, live, object_param, ok, str_param,
};

// ---------- Gradient mesh ----------

pub(super) fn mesh_create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.create";
    let (rows, cols) = rows_cols(p, C, (4, 4))?;
    let app = match str_param(p, "appearance") {
        Some(a) => MeshAppearance::parse(a).ok_or_else(|| bad(C, "appearance must be flat|center|edge"))?,
        None => MeshAppearance::Flat,
    };
    check_numbers(p, &["highlight", "tolerance"], C)?;
    let highlight = f64_or(p, "highlight", 100.0).clamp(0.0, 100.0);
    let tol = f64_or(p, "tolerance", 0.0).max(0.0);
    let at = match p.get("at") {
        None | Some(Value::Null) => None,
        Some(v) => Some(finite_numbers(v).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad(C, "`at` must be [x, y]"))?),
    };
    let (rows, cols) = if at.is_some() { (1, 1) } else { (rows, cols) };
    let roots = roots_param(s, p)?;
    let (ids, index) = s.edit("Create Gradient Mesh", |d, sel| {
        let mut out = vec![];
        let mut index = None;
        let mut refused = None;
        let model = d.color_mode.model();
        for id in &roots {
            let Some(n) = d.node(*id).cloned() else { continue };
            let m = if let NodeKind::Image(im) = &n.kind {
                // An embedded image: the points take the picture's colours there.
                match image_mesh(d, im, rows, cols) {
                    Ok(mut m) => {
                        m.highlight(app, highlight);
                        Some(m)
                    }
                    Err(why) => {
                        refused.get_or_insert(why);
                        continue;
                    }
                }
            } else {
                let Some(path) = outline_of(&n) else { continue };
                match n.appearance.fill_paint() {
                    // The points take the colours the gradient paints there.
                    Paint::Gradient(g) => {
                        path.bounds().and_then(|b| GradientMesh::for_path_shaded(&path, rows, cols, &*gradient_sampler(&g, b, model), app, highlight))
                    }
                    p => GradientMesh::for_path(&path, rows, cols, p.color().unwrap_or(Color::WHITE), app, highlight),
                }
            };
            let Some(mut m) = m else { continue };
            if let Some(q) = at {
                // A point the new mesh can't place its lines at leaves the object as it is.
                match m.add_lines_at(q, tol) {
                    Some(i) => index = Some(i),
                    None => {
                        refused.get_or_insert("point is outside the object");
                        continue;
                    }
                }
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
            return Err(bad(C, refused.unwrap_or("select a path, a compound path or an embedded image")));
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

/// Create Gradient Mesh on an embedded image: a `rows` × `cols` mesh over the picture's rectangle
/// (placed with the image's transform), each point taking the average colour and opacity of the
/// area around it. Linked images are refused: the documentation allows embedded ones only.
fn image_mesh(d: &Document, im: &vectorcraft_doc::ImageObject, rows: u32, cols: u32) -> std::result::Result<GradientMesh, &'static str> {
    if im.link.is_some() {
        return Err("a linked image can't become a gradient mesh: embed it first");
    }
    let xf = im.xf.as_coeffs();
    if im.width == 0 || im.height == 0 || !xf.iter().all(|k| k.is_finite()) {
        return Err("the image has no usable size or position");
    }
    let samples = d.images.get(&im.key).and_then(|b| b.grid_samples(rows, cols)).ok_or("the image's pixels can't be read")?;
    let (w, h) = (im.width as f64, im.height as f64);
    let at = |u: f64, v: f64| im.xf * Point::new(u * w, v * h);
    if ![at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)].iter().all(|p| p.x.abs() < 1e15 && p.y.abs() < 1e15) {
        return Err("the image has no usable size or position");
    }
    let cols_n = cols as usize + 1;
    let steps = |n: u32| (0..=n).map(|i| i as f64 / n as f64).collect::<Vec<_>>();
    Ok(GradientMesh::from_grid(&steps(cols), &steps(rows), &at, &|r, c, _| {
        let [pr, pg, pb, pa] = samples.get(r * cols_n + c).copied().unwrap_or([0; 4]);
        (Color::rgb8(pr, pg, pb), pa as f32 / 255.0)
    }))
}

/// Run `f` on the gradient mesh `p.id` (one undo step `label`); with `envelopes`, a mesh
/// envelope's grid too (see [`edit_grid`]).
fn with_mesh<T>(s: &mut Session, p: &Value, cmd: &str, label: &str, envelopes: bool, f: impl FnOnce(&mut GradientMesh) -> Result<T>) -> Result<T> {
    let id = object_param(p, "id", cmd)?.ok_or_else(|| bad(cmd, "missing `id`"))?;
    s.edit(label, move |d, _| edit_grid(d, id, cmd, envelopes, f))
}

/// The grid an edit of envelope `n`'s points starts from: a mesh envelope's own grid, or a warp's
/// grid ([`live::warp_grid`]), which the edit turns into a mesh envelope. `None` for other nodes.
pub(super) fn envelope_edit_grid(n: &Node, cmd: &str) -> Option<Result<GradientMesh>> {
    let NodeKind::Envelope { kind, .. } = &n.kind else { return None };
    match &**kind {
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            Some(live::envelope_grid(*rows, *cols, points, handles).ok_or_else(|| bad(cmd, "the envelope's mesh is malformed")))
        }
        EnvelopeKind::Warp { .. } => {
            Some(live::warp_grid(n).ok_or_else(|| bad(cmd, "this warp's points can't be edited: use Reset with Mesh first")))
        }
        EnvelopeKind::TopObject { .. } => None,
    }
}

/// The mesh envelope kind of grid `m`, with its points' handles.
pub(super) fn mesh_envelope(m: &GradientMesh) -> EnvelopeKind {
    EnvelopeKind::Mesh {
        rows: m.rows,
        cols: m.cols,
        points: m.points.iter().map(|q| q.p).collect(),
        handles: m.points.iter().map(|q| q.handles).collect(),
    }
}

/// Does placing point `index` of warp envelope `id` of `d`, or that point's `handle` end, at `q`
/// leave it where it is? Such a placement is no edit. `false` for other objects and for an index
/// out of range, whose edits run as before.
pub(super) fn warp_point_stays(d: &Document, id: NodeId, index: usize, handle: Option<usize>, q: Point, cmd: &str) -> Result<bool> {
    let Some(n) = d.node(id).filter(|n| matches!(&n.kind, NodeKind::Envelope { kind, .. } if matches!(**kind, EnvelopeKind::Warp { .. }))) else {
        return Ok(false);
    };
    let grid = envelope_grid_to_edit(n, cmd, true)?;
    Ok(grid.points.get(index).is_some_and(|pt| match handle {
        Some(h) => pt.handles.get(h).is_some_and(|v| *v == q - pt.p),
        None => pt.p == q,
    }))
}

/// The grid of envelope `n` that [`edit_grid`] edits (only `envelopes`), or the error it fails with.
fn envelope_grid_to_edit(n: &Node, cmd: &str, envelopes: bool) -> Result<GradientMesh> {
    match envelope_edit_grid(n, cmd).filter(|_| envelopes) {
        Some(grid) => grid,
        None => Err(bad(cmd, if envelopes { "not a gradient mesh or mesh envelope" } else { "not a gradient mesh" })),
    }
}

/// The grid of mesh `id` of `d` that [`edit_grid`] edits, as it is now.
fn grid_to_edit(d: &Document, id: NodeId, cmd: &str, envelopes: bool) -> Result<GradientMesh> {
    match d.node(id) {
        Some(Node { kind: NodeKind::Mesh(m), .. }) => Ok(m.clone()),
        Some(n) => envelope_grid_to_edit(n, cmd, envelopes),
        None => Err(EngineError::NoNode(id)),
    }
}

/// Run `f` on the gradient mesh `id` of `d`; with `envelopes`, a warp or mesh envelope's grid too
/// (see [`edit_node`]).
fn edit_grid<T>(d: &mut Document, id: NodeId, cmd: &str, envelopes: bool, f: impl FnOnce(&mut GradientMesh) -> Result<T>) -> Result<T> {
    edit_node(d.node_mut(id).ok_or(EngineError::NoNode(id))?, cmd, envelopes, f)
}

/// [`edit_grid`] on a node: with `envelopes`, a warp or mesh envelope's grid
/// ([`envelope_edit_grid`]) is edited too, and its points and handles are written back as a mesh
/// envelope (editing fills in the handles, and makes a warp a mesh envelope).
fn edit_node<T>(n: &mut Node, cmd: &str, envelopes: bool, f: impl FnOnce(&mut GradientMesh) -> Result<T>) -> Result<T> {
    if let NodeKind::Mesh(m) = &mut n.kind {
        return f(m);
    }
    let start = envelope_grid_to_edit(n, cmd, envelopes)?;
    let mut m = start.clone();
    let r = f(&mut m)?;
    // A warp stays a warp when the edit changed nothing.
    let warp = matches!(&n.kind, NodeKind::Envelope { kind, .. } if matches!(**kind, EnvelopeKind::Warp { .. }));
    if warp && m == start {
        return Ok(r);
    }
    if let NodeKind::Envelope { kind, .. } = &mut n.kind {
        **kind = mesh_envelope(&m);
    }
    Ok(r)
}

/// Most mesh points one command names, over all its meshes.
const MAX_POINTS: usize = 1 << 20;

/// The mesh points `p` names: `{id, index}`, `{id, points: [index…]}` or `{items: [{id, index |
/// points}…]}`. Each mesh comes once, with its points in order and without repeats.
fn point_items(p: &Value, cmd: &str) -> Result<Vec<(NodeId, BTreeSet<usize>)>> {
    let one = |q: &Value| -> Result<(NodeId, BTreeSet<usize>)> {
        let id = object_param(q, "id", cmd)?.ok_or_else(|| bad(cmd, "missing `id`"))?;
        let points = match q.get("points") {
            None | Some(Value::Null) => BTreeSet::from([index_param(q, "index", cmd)?]),
            Some(Value::Array(v)) if v.len() <= MAX_POINTS => v
                .iter()
                .map(|i| index_of(i.as_f64().ok_or_else(|| bad(cmd, "`points` must be point indexes"))?, "points", cmd))
                .collect::<Result<_>>()?,
            Some(_) => return Err(bad(cmd, "`points` must be a list of point indexes")),
        };
        Ok((id, points))
    };
    let items: Vec<(NodeId, BTreeSet<usize>)> = match p.get("items") {
        None | Some(Value::Null) => vec![one(p)?],
        Some(Value::Array(v)) if v.len() <= MAX_POINTS => v.iter().map(one).collect::<Result<_>>()?,
        Some(_) => return Err(bad(cmd, "`items` must be a list of {id, points}")),
    };
    let mut merged: BTreeMap<NodeId, BTreeSet<usize>> = BTreeMap::new();
    for (id, points) in items {
        merged.entry(id).or_default().extend(points);
    }
    if merged.values().map(BTreeSet::len).sum::<usize>() > MAX_POINTS {
        return Err(bad(cmd, format!("at most {MAX_POINTS} points")));
    }
    Ok(merged.into_iter().collect())
}

/// Run `f` on each mesh point `items` names, as one undo step `label` (an index out of range
/// fails it, changing nothing); with `envelopes`, on warp and mesh envelopes' points too. When `f`
/// changes no point, nothing changes and no undo step is recorded.
fn with_points(
    s: &mut Session,
    items: &[(NodeId, BTreeSet<usize>)],
    cmd: &str,
    label: &str,
    envelopes: bool,
    mut f: impl FnMut(&mut live::MeshPoint) -> Result<()>,
) -> Result<()> {
    let doc = &s.doc()?.doc;
    let mut changes = false;
    for (id, points) in items {
        let mut m = grid_to_edit(doc, *id, cmd, envelopes)?;
        for i in points {
            let q = m.points.get_mut(*i).ok_or_else(|| bad(cmd, "index out of range"))?;
            let before = q.clone();
            f(q)?;
            changes |= *q != before;
        }
    }
    if !changes {
        return Ok(());
    }
    s.edit(label, |d, _| {
        for (id, points) in items {
            edit_grid(d, *id, cmd, envelopes, |m| {
                for i in points {
                    f(m.points.get_mut(*i).ok_or_else(|| bad(cmd, "index out of range"))?)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    })
}

/// The `handle` parameter (0 right, 1 left, 2 down, 3 up), if any.
pub(super) fn handle_param(p: &Value, cmd: &str) -> Result<Option<usize>> {
    match p.get("handle") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(h) if h <= 3 => Ok(Some(h as usize)),
            _ => Err(bad(cmd, "handle must be 0..3")),
        },
    }
}

pub(super) fn index_param(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    index_of(f64_req(p, key, cmd)?, key, cmd)
}

/// `i`, the value of `key`, as an index: a whole number ≥ 0.
fn index_of(i: f64, key: &str, cmd: &str) -> Result<usize> {
    if i < 0.0 {
        return Err(bad(cmd, "index must be ≥ 0"));
    }
    Ok(whole(i, key, cmd)? as usize)
}

/// `x`, the value of `key`, when it is a whole number, and an error otherwise.
pub(super) fn whole(x: f64, key: &str, cmd: &str) -> Result<f64> {
    if x.fract() == 0.0 { Ok(x) } else { Err(bad(cmd, format!("`{key}` must be a whole number"))) }
}

pub(super) fn mesh_set_color(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.setPointColor";
    check_numbers(p, &["opacity"], C)?;
    crate::cmd::paint::check_keep_model(p, C)?;
    let items = point_items(p, C)?;
    let color = color_param(p, "color", C)?.map(|c| crate::cmd::paint::new_color(s, p, c));
    let opacity = p.get("opacity").and_then(Value::as_f64);
    if color.is_none() && opacity.is_none() {
        return Err(bad(C, "missing `color` or `opacity`"));
    }
    with_points(s, &items, C, "Mesh Point Color", false, |pt| {
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

pub(super) fn mesh_move_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.movePoint";
    let index = index_param(p, "index", C)?;
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let handle = handle_param(p, C)?;
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing `id`"))?;
    if warp_point_stays(&s.doc()?.doc, id, index, handle, q, C)? {
        return ok();
    }
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

pub(super) fn mesh_move_points(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.movePoints";
    let items = point_items(p, C)?;
    let (dx, dy) = (f64_req(p, "dx", C)?, f64_req(p, "dy", C)?);
    // `Session::edit` refuses a move past the canvas's coordinate range.
    with_points(s, &items, C, "Move Mesh Points", true, |pt| {
        pt.p = Point::new(pt.p.x + dx, pt.p.y + dy);
        Ok(())
    })?;
    ok()
}

pub(super) fn mesh_add_line(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.addLine";
    let q = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    crate::cmd::paint::check_keep_model(p, C)?;
    check_numbers(p, &["tolerance"], C)?;
    let tol = f64_or(p, "tolerance", 0.0).max(0.0);
    let color = color_param(p, "color", C)?.map(|c| crate::cmd::paint::new_color(s, p, c));
    let add = move |m: &mut GradientMesh| {
        let i = m.add_lines_at(q, tol).ok_or_else(|| bad(C, "point is outside the mesh"))?;
        if let Some(c) = color {
            m.points[i].color = c;
        }
        Ok(i)
    };
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing `id`"))?;
    if s.script_running() {
        return Err(EngineError::Busy);
    }
    // The lines are added to a copy of the object. A point that is already a mesh point adds none:
    // when its color stays too, nothing changes and no undo step is recorded. The grid is compared,
    // not the node: a mesh envelope stored without handles takes them in any edit, a warp becomes
    // a mesh envelope, and neither keeps colors, so only their points and handles count.
    let orig = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?.clone();
    let colored = matches!(orig.kind, NodeKind::Mesh(_));
    let mut node = orig.clone();
    let (i, same) = edit_node(&mut node, C, true, |m| {
        let before = m.clone();
        let i = add(m)?;
        let shape = |g: &GradientMesh| (g.rows, g.cols, g.points.iter().map(|q| (q.p, q.handles)).collect::<Vec<_>>());
        Ok((i, if colored { *m == before } else { shape(m) == shape(&before) }))
    })?;
    if same {
        return Ok(json!({ "index": i }));
    }
    let index = s.edit("Add Mesh Line", move |d, _| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        // Settling typed values before the edit may have changed the object: add the lines again.
        if n.kind != orig.kind {
            return edit_node(n, C, true, add);
        }
        n.kind = node.kind;
        Ok(i)
    })?;
    Ok(json!({ "index": index }))
}

pub(super) fn mesh_delete_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.mesh.deletePoint";
    let index = index_param(p, "index", C)?;
    with_mesh(s, p, C, "Delete Mesh Point", true, |m| {
        if m.remove_point_lines(index) { Ok(()) } else { Err(bad(C, "only interior mesh lines can be deleted")) }
    })?;
    ok()
}

pub(super) fn mesh_expand(s: &mut Session, _: &Value) -> Result<Value> {
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
