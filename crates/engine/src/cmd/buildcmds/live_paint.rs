use super::{
    Appearance, AppearanceItem, Arc, Beside, Document, EDGE_NAME, EngineError, FACE_NAME, GapOptions, GapSize, LIVE_PAINT_NAME, Node, NodeId,
    PathData, Point, Rect, Result, SOURCES_NAME, Selection, Session, SubPath, Value, b, bad, build_children, edge_near, face_at, finite,
    fit_to_old_edge, ids_json, is_live_paint, json, number_or, object_param, paint_from, po, point_param, repaint_from, replace_roots, root_ids,
    source_nodes, source_shapes, switch_or,
};

/// `options` as stored on a Live Paint group: none for the defaults.
fn stored_gap_options(options: GapOptions) -> Option<Box<GapOptions>> {
    (options != GapOptions::default()).then(|| Box::new(options))
}

/// The Gap Options of Live Paint groups merged into one, given bottom-most first: the bottom-most
/// group's when its Gap Detection is on; else Gap Detection on with Small Gaps when another
/// group's is on; else the bottom-most group's.
fn merged_gap_options(groups: &[GapOptions]) -> GapOptions {
    let Some(&bottom) = groups.first() else { return GapOptions::default() };
    if !bottom.detect && groups.iter().any(|g| g.detect) { GapOptions { detect: true, size: GapSize::Small, ..bottom } } else { bottom }
}

/// Refuse `roots` when one of them is something Live Paint can't take ([`b::live_paint_takes`]):
/// its faces and edges come from path outlines alone, so type or an image would vanish among the
/// group's hidden sources. Checked before anything changes.
fn check_live_paint_takes(d: &Document, roots: &[NodeId]) -> Result<()> {
    for r in roots {
        if !b::live_paint_takes(d.node(*r).ok_or(EngineError::NoNode(*r))?) {
            return Err(EngineError::Other(
                "Live Paint takes paths and compound paths: convert type, images and other objects to paths first".into(),
            ));
        }
    }
    Ok(())
}

/// Make a Live Paint group from `roots`, or add them to the first Live Paint group among them
/// as Merge does; returns (group, faces, edges).
fn make_live(d: &mut Document, sel: &mut Selection, roots: &[NodeId]) -> Result<(NodeId, usize, usize)> {
    check_live_paint_takes(d, roots)?;
    if roots.iter().any(|r| d.node(*r).is_some_and(is_live_paint)) {
        return merge_live(d, sel, roots, false);
    }
    let mut sources = vec![];
    for r in roots {
        let n = d.node(*r).ok_or(EngineError::NoNode(*r))?;
        sources.extend(source_nodes(n));
    }
    let (children, nf, ne) = build_children(d, sources, None)?;
    let gid = d.alloc_id();
    let mut g = Node::group(gid, children);
    g.name = Some(LIVE_PAINT_NAME.into());
    replace_roots(d, roots, vec![g])?;
    sel.set([gid]);
    Ok((gid, nf, ne))
}

/// The children of Live Paint group `g` besides its sources group ([`b::sources`]) and its faces and
/// edges (paths named [`FACE_NAME`] or [`EDGE_NAME`]), back to front: objects drawn in it in
/// isolation mode. A group or other object named like a part is one of them.
fn others_in(g: &Node) -> impl Iterator<Item = &Arc<Node>> {
    let src = b::sources(g).map(|s| s.id);
    g.children()
        .into_iter()
        .flatten()
        .filter(move |c| Some(c.id) != src && !(c.path_data().is_some() && matches!(c.name.as_deref(), Some(FACE_NAME | EDGE_NAME))))
}

/// Make `parts` (a sources group, faces and edges made again) the children of Live Paint group `g`,
/// with its other children ([`others_in`]) that are not among `gone` kept in front of them, in their
/// order.
fn set_parts(g: &mut Node, mut parts: Vec<Arc<Node>>, gone: &[NodeId]) {
    parts.extend(others_in(g).filter(|c| !gone.contains(&c.id)).cloned());
    if let Some(ch) = g.children_mut() {
        *ch = parts;
    }
}

/// Whether a path named `name` directly in a Live Paint group is one of its faces or edges: the
/// name is [`FACE_NAME`] or [`EDGE_NAME`].
fn part_name(name: Option<&str>) -> bool {
    matches!(name, Some(FACE_NAME | EDGE_NAME))
}

/// Whether `n` is a path whose parent `parent` is a Live Paint group.
fn path_in_live_paint(d: &Document, parent: Option<NodeId>, n: &Node) -> bool {
    n.path_data().is_some() && parent.and_then(|p| d.node(p)).is_some_and(is_live_paint)
}

/// Clear the name of `n`, put directly in `parent`, when it is a path named like a face or an edge
/// ([`part_name`]) and `parent` is a Live Paint group, so that the group's other children
/// ([`others_in`]) keep it when its faces and edges are made again. Paste uses it.
pub(crate) fn clear_part_name(d: &Document, parent: Option<NodeId>, n: &mut Node) {
    if part_name(n.name.as_deref()) && path_in_live_paint(d, parent, n) {
        n.name = None;
    }
}

/// Refuse naming rows `ids` `name` when it is the name of a face or an edge ([`part_name`]) and one
/// of them is a path directly in a Live Paint group that is not named `name` already. Checked
/// before anything changes.
pub(crate) fn check_part_name(d: &Document, ids: &[NodeId], name: &str) -> Result<()> {
    if !part_name(Some(name)) {
        return Ok(());
    }
    for id in ids {
        if let Some(n) = d.node(*id)
            && n.name.as_deref() != Some(name)
            && path_in_live_paint(d, d.parent_of(*id), n)
        {
            return Err(EngineError::Other("`Face` and `Edge` name the faces and edges of a Live Paint group: give the path another name".into()));
        }
    }
    Ok(())
}

/// Select `ids` alone, without an undo step, when they are not selected that way already.
fn select_only(s: &mut Session, ids: &[NodeId]) -> Result<()> {
    let st = s.doc()?;
    let mut sel = st.selection.clone();
    sel.set(ids.iter().copied());
    if sel != st.selection {
        s.select(|_, selection| *selection = sel)?;
    }
    Ok(())
}

/// Make or Merge given one Live Paint group and nothing else has nothing to add to it, so it leaves
/// the document as it is, records no undo step and selects the group: → its reply. `None` for any
/// other `roots`.
fn lone_live_group(s: &mut Session, roots: &[NodeId]) -> Result<Option<Value>> {
    let d = &s.doc()?.doc;
    let Some(&g) = roots.first().filter(|g| roots.iter().all(|r| r == *g)) else { return Ok(None) };
    let Some(n) = d.node(g).filter(|n| is_live_paint(n)) else { return Ok(None) };
    let reply = json!({ "id": g.0, "faces": b::faces(n).len(), "edges": b::edges(n).len() });
    select_only(s, &[g])?;
    Ok(Some(reply))
}

pub(super) fn lp_make(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    if let Some(reply) = lone_live_group(s, &roots)? {
        return Ok(reply);
    }
    let (id, faces, edges) = s.edit("Make Live Paint", |d, sel| make_live(d, sel, &roots))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

/// How far, in points, the middle of an edge Merge makes may be from the old edge it lies on.
const ON_OLD_EDGE: f64 = 1e-3;

/// What Merge carries from a face or edge to the part it makes again in its place, another path:
/// the appearance, visibility, opacity and blending mode. Gradients fitted to the old outline keep
/// that fit, and the strokes' pen pressure, tilt, bearing and rotation stay behind.
struct Carried {
    appearance: Appearance,
    visible: bool,
    opacity: f32,
    blend: vectorcraft_color::BlendMode,
}

impl Carried {
    fn of(n: &Node) -> Self {
        let mut appearance = (*n.appearance).clone().without_pressure();
        if appearance.has_unplaced_gradient()
            && let Some(bounds) = n.geometric_bounds()
        {
            appearance.pin_gradients(bounds);
        }
        Self { appearance, visible: n.visible, opacity: n.opacity, blend: n.blend }
    }

    fn onto(self, n: &mut Node) {
        n.appearance = self.appearance.into();
        n.visible = self.visible;
        n.opacity = self.opacity;
        n.blend = self.blend;
    }
}

/// Add the other `roots` (back → front) to the first Live Paint group among them, which keeps its
/// id, the paint of its faces and edges, and its own properties apart from the Gap Options, which
/// are merged; returns (group, faces, edges). When the groups and paths hold no path outline
/// between them, `keep_empty` leaves the group with its paths in its sources group and no faces or
/// edges; without it, that is an error.
fn merge_live(d: &mut Document, sel: &mut Selection, roots: &[NodeId], keep_empty: bool) -> Result<(NodeId, usize, usize)> {
    let target = roots
        .iter()
        .copied()
        .find(|r| d.node(*r).is_some_and(is_live_paint))
        .ok_or_else(|| EngineError::Other("Live Paint Merge: select a Live Paint group and the paths to add".into()))?;
    check_live_paint_takes(d, roots)?;
    let old = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
    // The rows that join are children of the group already when Merge runs after a drag, and a joining
    // path named like a face or an edge is not one of the group's parts.
    let mut old_parts = old.clone();
    if let Some(ch) = old_parts.children_mut() {
        ch.retain(|c| !roots.contains(&c.id));
    }
    // Paint to carry over: (point inside face, paint) for faces that draw; edges are matched to the
    // old edges below.
    let keep_faces: Vec<(Point, Carried)> =
        b::faces(&old_parts).iter().filter(|f| draws(f)).filter_map(|f| Some((po::interior_point(f.path_data()?)?, Carried::of(f)))).collect();
    let mut sources = source_nodes(&old);
    // The objects drawn in the other Live Paint groups besides their parts stay in the merged group.
    let mut others = vec![];
    for r in roots {
        if *r != target {
            let n = d.node(*r).ok_or(EngineError::NoNode(*r))?;
            sources.extend(source_nodes(n));
            if is_live_paint(n) {
                others.extend(others_in(n).cloned());
            }
        }
    }
    // Merge applies the Gap Options' merge rules to the groups' options, bottom-most first.
    let groups: Vec<GapOptions> = roots.iter().filter_map(|r| d.node(*r)).filter(|n| is_live_paint(n)).map(Node::gap_options).collect();
    let options = merged_gap_options(&groups);
    let (children, nf, ne) = if keep_empty && source_shapes(&sources).0.is_empty() {
        let mut src = Node::group(d.alloc_id(), sources);
        src.name = Some(SOURCES_NAME.into());
        src.visible = false;
        (vec![Arc::new(src)], 0, 0)
    } else {
        build_children(d, sources, options.max_gap())?
    };
    // The group keeps its name, lock, visibility, transparency, appearance, opacity mask, graphic
    // style link and the rest; its children are made again and its Gap Options set to the merged ones.
    let mut g = old;
    set_parts(&mut g, children, roots);
    if let Some(ch) = g.children_mut() {
        ch.extend(others);
    }
    g.extras_mut().live_paint_gaps = stored_gap_options(options);
    for (pt, carried) in keep_faces {
        if let Some(fid) = face_at(&g, pt).map(|f| f.id)
            && let Some(ch) = g.children_mut()
            && let Some(f) = ch.iter_mut().find(|c| c.id == fid)
        {
            carried.onto(Arc::make_mut(f));
        }
    }
    // Each edge whose middle lies on an old edge, every piece of an edge an added path cuts
    // included, takes the old edge's paint, with its rows fitted to its part of the old edge.
    if let Some(ch) = g.children_mut() {
        for c in ch.iter_mut().filter(|c| c.name.as_deref() == Some(EDGE_NAME)) {
            let Some(mid) = c
                .path_data()
                .and_then(|pd| pd.subpaths.first())
                .filter(|sp| sp.segment_count() > 0)
                .map(|sp| kurbo::ParamCurve::eval(&sp.segment(0), 0.5))
            else {
                continue;
            };
            let Some(was) = edge_near(&old_parts, mid, ON_OLD_EDGE) else { continue };
            let e = Arc::make_mut(c);
            Carried::of(was).onto(e);
            if let Some(outline) = was.path_data() {
                fit_to_old_edge(e, outline);
            }
        }
    }
    let mut merged = Beside::replacing(d, roots);
    for r in roots.iter().filter(|r| **r != target) {
        if d.node(*r).is_some() {
            merged.put(d, *r, vec![])?;
        }
    }
    merged.put(d, target, vec![g])?;
    merged.finish(d)?;
    sel.set([target]);
    Ok((target, nf, ne))
}

pub(super) fn lp_merge(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    if let Some(reply) = lone_live_group(s, &roots)? {
        return Ok(reply);
    }
    let (id, faces, edges) = s.edit("Merge Live Paint", |d, sel| merge_live(d, sel, &roots, false))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

/// The Live Paint groups whose drawn art can be added when isolation mode leaves `isolated`, innermost
/// first: every Live Paint group that is `isolated` or holds it, such as a group drawn in an isolated
/// Live Paint group and then isolated itself, and the group it was drawn in. Empty when `isolated` is
/// not in a Live Paint group.
pub(crate) fn isolated_live_paints(d: &Document, isolated: NodeId) -> Vec<NodeId> {
    d.ancestry(isolated).unwrap_or_default().into_iter().rev().filter(|id| d.node(*id).is_some_and(is_live_paint)).collect()
}

/// The children of Live Paint group `g` besides its parts ([`others_in`]) that Live Paint takes
/// ([`b::live_paint_takes`]) and that are shown and unlocked with everything in them, back to
/// front: the paths drawn in it in isolation mode. A Live Paint group's sources group, which is
/// always hidden, does not count.
fn drawn_in(g: &Node) -> Vec<NodeId> {
    fn open(n: &Node) -> bool {
        let src = b::sources(n).filter(|_| is_live_paint(n)).map(|s| s.id);
        n.visible && !n.locked && n.children().is_none_or(|ch| ch.iter().filter(|c| Some(c.id) != src).all(|c| open(c)))
    }
    others_in(g).filter(|c| open(c) && b::live_paint_takes(c)).map(|c| c.id).collect()
}

/// Leaving isolation mode in Live Paint group `g` adds the paths drawn in it to the group as
/// `livePaint.merge` does, in one "Merge Live Paint" undo step; → whether any were added. Type,
/// images, other objects Live Paint does not take, and hidden or locked objects stay where they
/// are. With nothing to add, or when `g` is not a Live Paint group, nothing changes and no undo
/// step is recorded.
pub(crate) fn merge_drawn_in_isolation(s: &mut Session, g: NodeId) -> Result<bool> {
    let drawn = s.doc()?.doc.node(g).filter(|n| is_live_paint(n)).map(drawn_in).unwrap_or_default();
    if drawn.is_empty() {
        return Ok(false);
    }
    let roots: Vec<NodeId> = std::iter::once(g).chain(drawn).collect();
    s.edit("Merge Live Paint", |d, sel| merge_live(d, sel, &roots, false))?;
    Ok(true)
}

/// Where a row sits for Live Paint: among the paths of a Live Paint group (in its sources group,
/// at any depth), directly in a Live Paint group, or neither.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Spot {
    Source(NodeId),
    In(NodeId),
    Outside,
}

/// The [`Spot`] of a row whose parent is `parent` (None: the top level).
fn spot_in(d: &Document, parent: Option<NodeId>) -> Spot {
    let Some(p) = parent else { return Spot::Outside };
    for x in d.ancestry(p).unwrap_or_default().into_iter().rev() {
        let n = d.node(x);
        if n.is_some_and(is_live_paint) {
            return if x == p { Spot::In(x) } else { Spot::Outside };
        }
        if let Some(g) = d.parent_of(x)
            && d.node(g).filter(|n| is_live_paint(n)).and_then(b::sources).is_some_and(|s| s.id == x)
        {
            return Spot::Source(g);
        }
    }
    Spot::Outside
}

/// The Live Paint groups that hold Live Paint group `g` among their paths (in their sources group,
/// at any depth), innermost first.
fn enclosing_live_paints(d: &Document, g: NodeId) -> Vec<NodeId> {
    let mut out = vec![];
    let mut at = g;
    while let Spot::Source(o) = spot_in(d, d.parent_of(at)) {
        if out.contains(&o) {
            break;
        }
        out.push(o);
        at = o;
    }
    out
}

/// Whether a shown row whose parent is `parent`, among Live Paint group `g`'s paths, gives `g` faces
/// and edges: every group between it and `g`'s sources group is shown.
fn shown_in_sources(d: &Document, parent: Option<NodeId>, g: NodeId) -> bool {
    let src = d.node(g).and_then(b::sources).map(|s| s.id);
    let mut at = parent;
    while let Some(x) = at {
        if Some(x) == src {
            return true;
        }
        if !d.node(x).is_some_and(|n| n.visible) {
            return false;
        }
        at = d.parent_of(x);
    }
    true
}

/// What moving rows in the Layers panel (`layer.move`, `node.move`) does to Live Paint groups.
/// Rows that go into a Live Paint group from elsewhere are added to it as `livePaint.merge` adds
/// paths. Rows that go into or leave a group's paths (its sources group) change those paths, and
/// the group's faces and edges are made again from them, keeping the paint of the faces and edges
/// that stay, and so are those of every Live Paint group that holds a changed group among its
/// paths. Rows moved within one group's paths change no faces unless a shown row goes into or out
/// of a hidden group there; rows moved among a group's own children change no faces. A group's
/// sources group itself stays in the group.
pub(crate) struct LivePaintMove {
    /// The Live Paint group the rows go into, and for each row whether it joins the group's paths.
    join: Option<(NodeId, Vec<bool>)>,
    /// The Live Paint group the rows go among the children of, with its sources group as it is
    /// before the move.
    keep_sources: Option<(NodeId, NodeId)>,
    /// The Live Paint groups whose paths change.
    rebuild: Vec<NodeId>,
}

impl LivePaintMove {
    /// Plan moving `rows` (with `copy`, copies of them) into `parent`, before anything changes.
    /// Refuses rows Live Paint can't take ([`b::live_paint_takes`]) that would go among a group's
    /// paths, with the message Make and Merge give, and a group's sources group moved out of the
    /// group.
    pub(crate) fn plan(d: &Document, rows: &[NodeId], parent: Option<NodeId>, copy: bool) -> Result<Self> {
        if !copy {
            for r in rows {
                if let Some(g) = d.parent_of(*r)
                    && parent != Some(g)
                    && d.node(g).filter(|n| is_live_paint(n)).and_then(b::sources).is_some_and(|s| s.id == *r)
                {
                    return Err(EngineError::Other("a Live Paint group's sources group stays in the group: move the paths in it instead".into()));
                }
            }
        }
        let to = spot_in(d, parent);
        let from: Vec<Spot> = rows.iter().map(|r| if copy { Spot::Outside } else { spot_in(d, d.parent_of(*r)) }).collect();
        let mut rebuild = vec![];
        for (r, f) in rows.iter().zip(&from) {
            let Spot::Source(g) = *f else { continue };
            // A shown row that goes into or out of a hidden group among the same paths starts or
            // stops giving the group faces and edges.
            let shown_changes =
                *f == to && d.node(*r).is_some_and(|n| n.visible) && shown_in_sources(d, d.parent_of(*r), g) != shown_in_sources(d, parent, g);
            if (*f != to || shown_changes) && !rebuild.contains(&g) {
                rebuild.push(g);
            }
        }
        if let Spot::Source(g) = to
            && from.iter().any(|f| *f != to)
            && !rebuild.contains(&g)
        {
            rebuild.push(g);
        }
        let entering: Vec<NodeId> = rows.iter().zip(&from).filter(|(_, f)| **f != to).map(|(r, _)| *r).collect();
        let join = match to {
            Spot::In(g) if !entering.is_empty() => Some((g, from.iter().map(|f| *f != to).collect())),
            _ => None,
        };
        if to != Spot::Outside {
            check_live_paint_takes(d, &entering)?;
        }
        let keep_sources = match to {
            Spot::In(g) => d.node(g).and_then(b::sources).map(|s| (g, s.id)),
            _ => None,
        };
        Ok(Self { join, keep_sources, rebuild })
    }

    /// Whether rows join a Live Paint group, which makes the move a "Merge Live Paint" step.
    pub(crate) fn joins(&self) -> bool {
        self.join.is_some()
    }

    /// After the rows were moved, now `moved` (in the order [`LivePaintMove::plan`] took them),
    /// make the faces and edges of the groups whose paths changed again, and add the rows that
    /// joined a group to it. → `moved` with the joined rows replaced by their group, once, and
    /// without the faces and edges that were made again.
    pub(crate) fn finish(self, d: &mut Document, sel: &mut Selection, moved: &[NodeId]) -> Result<Vec<NodeId>> {
        // A moved object named like the sources group may now sit before it, where a lookup by name
        // finds the object first. The group's own sources group goes back in front.
        if let Some((g, src)) = self.keep_sources {
            let ch = d.children_mut(Some(g))?;
            if let Some(at) = ch.iter().position(|c| c.id == src)
                && ch[..at].iter().any(|c| c.name.as_deref() == Some(SOURCES_NAME))
            {
                let s = ch.remove(at);
                ch.insert(0, s);
            }
        }
        if self.join.is_none() && self.rebuild.is_empty() {
            return Ok(moved.to_vec());
        }
        // The group the rows join is made again by the merge below, from its paths as they are now
        // and with the paint of its faces and edges as they were before the move.
        let join_group = self.join.as_ref().map(|(g, _)| *g);
        // A Live Paint group among another group's paths gives that group its faces and edges, so
        // the groups holding a changed group among their paths are made again too, innermost first.
        let mut groups: Vec<NodeId> = vec![];
        for g in self.rebuild.iter().copied().chain(join_group) {
            for x in std::iter::once(g).chain(enclosing_live_paints(d, g)) {
                if !groups.contains(&x) {
                    groups.push(x);
                }
            }
        }
        groups.sort_by_cached_key(|g| std::cmp::Reverse(d.ancestry(*g).map_or(0, |a| a.len())));
        let joined: Vec<NodeId> = match &self.join {
            Some((_, mask)) => moved.iter().zip(mask).filter(|(_, j)| **j).map(|(r, _)| *r).collect(),
            None => vec![],
        };
        for g in groups {
            if Some(g) == join_group {
                let roots: Vec<NodeId> = std::iter::once(g).chain(joined.iter().copied()).collect();
                // Rows that join a group with no path outline among them keep the move: the group
                // is left with no faces or edges, as when its last path leaves it.
                merge_live(d, sel, &roots, true)?;
            } else if d.node(g).is_some() {
                rebuild_paths(d, g)?;
            }
        }
        let Some((g, mask)) = self.join else { return Ok(moved.iter().copied().filter(|r| d.node(*r).is_some()).collect()) };
        // `moved` holds each row once, so only the group can come twice. Faces and edges of the
        // group moved along with the joining rows were made again, so they are left out.
        let mut out = vec![];
        let mut listed = false;
        for (r, j) in moved.iter().zip(&mask) {
            if *j {
                if !listed {
                    out.push(g);
                    listed = true;
                }
            } else if d.node(*r).is_some() {
                out.push(*r);
            }
        }
        Ok(out)
    }
}

/// Make Live Paint group `g`'s faces and edges again from the paths its sources group holds now,
/// with its Gap Options, carrying over the paint of the faces and edges that stay. With no path
/// left, the group keeps its sources group and its other children, and has no faces or edges.
fn rebuild_paths(d: &mut Document, g: NodeId) -> Result<()> {
    let old = d.node(g).cloned().ok_or(EngineError::NoNode(g))?;
    let sources = source_nodes(&old);
    let mut n = old.clone();
    if source_shapes(&sources).0.is_empty() {
        let src = b::sources(&old).cloned().ok_or(EngineError::NoNode(g))?;
        set_parts(&mut n, vec![src], &[]);
    } else {
        let (children, _, _) = build_children(d, sources, old.gap_options().max_gap())?;
        set_parts(&mut n, children, &[]);
        repaint_from(&mut n, &old);
    }
    let mut changed = Beside::replacing(d, &[g]);
    changed.put(d, g, vec![n])?;
    changed.finish(d)
}

fn live_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids = root_ids(s, p)?;
    let v: Vec<NodeId> = ids.into_iter().filter(|i| d.node(*i).is_some_and(is_live_paint)).collect();
    if v.is_empty() {
        return Err(EngineError::Other("select a Live Paint group".into()));
    }
    Ok(v)
}

pub(super) fn lp_release(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    let ids = s.edit("Release Live Paint", |d, sel| {
        let mut out = vec![];
        let mut released = Beside::replacing(d, &targets);
        for g in &targets {
            let n = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            // The objects drawn in the group besides its parts are released in front of its paths.
            let srcs: Vec<Node> = source_nodes(&n).iter().chain(others_in(&n)).map(|c| (**c).clone()).collect();
            out.extend(srcs.iter().map(|c| c.id));
            released.put(d, *g, srcs)?;
        }
        released.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

/// Does face or edge `n` draw on the canvas: it is shown, its opacity is above 0, and a fill or
/// stroke row of its appearance draws (see [`Appearance::has_drawn_fill`] and
/// [`Appearance::has_drawn_stroke`])?
fn draws(n: &Node) -> bool {
    n.visible && n.opacity > 0.0 && (n.appearance.has_drawn_fill() || n.appearance.has_drawn_stroke())
}

pub(super) fn lp_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = live_targets(s, p)?;
    s.edit("Expand Live Paint", |d, sel| {
        for g in &targets {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            n.name = None;
            {
                let mut e = n.extras_mut();
                e.trace = None;
                e.live_paint_gaps = None;
            }
            // Objects drawn in the group ([`others_in`]) stay, whatever they are named.
            let src = b::sources(n).map(|c| c.id);
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| match c.name.as_deref() {
                    _ if Some(c.id) == src => false,
                    Some(FACE_NAME | EDGE_NAME) if c.path_data().is_some() => draws(c),
                    _ => true,
                });
            }
        }
        sel.set(targets.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}

/// Gaps as `[x0, y0, x1, y1]` (the loose end, then where it closes to).
fn gaps_json(gaps: &[[Point; 2]]) -> Value {
    json!(gaps.iter().map(|[a, b]| [a.x, a.y, b.x, b.y]).collect::<Vec<_>>())
}

/// A group's Gap Options as `livePaint.info` gives them.
fn gap_options_json(o: GapOptions) -> Value {
    let [r, g, b] = o.color;
    let size = o.size.id().map_or_else(|| json!(o.size.points()), |id| json!(id));
    json!({ "detect": o.detect, "size": size, "points": o.size.points(), "color": format!("#{r:02x}{g:02x}{b:02x}") })
}

/// The gaps up to `options`' size wide among a Live Paint group's sources.
fn open_gaps(group: &Node, options: GapOptions) -> Vec<[Point; 2]> {
    po::find_gaps(&source_shapes(&source_nodes(group)).0, options.size.points())
}

/// The gaps View › Show Live Paint Gaps shows on `group`: those up to its Gap Options' size wide,
/// as `livePaint.info` lists them. None when `group` is not a Live Paint group.
pub fn live_paint_gaps(group: &Node) -> Vec<[Point; 2]> {
    if is_live_paint(group) { open_gaps(group, group.gap_options()) } else { vec![] }
}

pub(super) fn lp_gap_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.gapOptions";
    let detect = match p.get("detect").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(C, "detect must be true or false"))?),
        None => None,
    };
    let size =
        match p.get("size").filter(|v| !v.is_null()) {
            Some(v) => Some(GapSize::parse(v).ok_or_else(|| {
                bad(C, format!("size must be \"small\", \"medium\", \"large\" or a number of points over 0 up to {}", GapSize::MAX))
            })?),
            None => None,
        };
    let color = match p.get("color").filter(|v| !v.is_null()) {
        Some(v) => Some(crate::cmd::layer::layer_color_param(v).map_err(|e| bad(C, e))?.rgb()),
        None => None,
    };
    let close = switch_or(p, "closeGaps", false, C)?;
    let targets = live_targets(s, p)?;
    let with_params = |mut options: GapOptions| {
        options.detect = detect.unwrap_or(options.detect);
        options.size = size.unwrap_or(options.size);
        options.color = color.unwrap_or(options.color);
        options
    };
    // When every group already has these options and Close Gaps With Paths has no gap to close,
    // nothing changes: no undo step, and the reply is the same.
    let unchanged = {
        let d = &s.doc()?.doc;
        let (mut faces, mut gaps, mut same) = (0, vec![], true);
        for g in &targets {
            let old = d.node(*g).ok_or(EngineError::NoNode(*g))?;
            let options = old.gap_options();
            if with_params(options) != options {
                same = false;
                break;
            }
            let open = open_gaps(old, options);
            if close && !open.is_empty() {
                same = false;
                break;
            }
            faces += b::faces(old).len();
            gaps.extend(open);
        }
        same.then_some((faces, gaps))
    };
    if let Some((faces, gaps)) = unchanged {
        select_only(s, &targets)?;
        return Ok(json!({ "ids": ids_json(&targets), "faces": faces, "gaps": gaps_json(&gaps), "closed": 0 }));
    }
    let (faces, gaps, closed) = s.edit("Gap Options", |d, sel| {
        let (mut faces, mut gaps, mut closed) = (0, vec![], 0);
        let mut changed = Beside::replacing(d, &targets);
        for g in &targets {
            let old = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let options = with_params(old.gap_options());
            let mut sources = source_nodes(&old);
            let mut added = 0;
            if close {
                // Close Gaps With Paths adds an unpainted line across each gap to the group's paths.
                for gap in po::find_gaps(&source_shapes(&sources).0, options.size.points()) {
                    let line = PathData::single(SubPath::polyline(&gap, false));
                    sources.push(Arc::new(Node::path(d.alloc_id(), line, Appearance::default())));
                    added += 1;
                }
            }
            closed += added;
            let mut n = old.clone();
            // The faces change only with what paint stops at, or with new paths.
            if added > 0 || options.max_gap() != old.gap_options().max_gap() {
                let (children, _, _) = build_children(d, sources, options.max_gap())?;
                set_parts(&mut n, children, &[]);
                repaint_from(&mut n, &old);
            }
            n.extras_mut().live_paint_gaps = stored_gap_options(options);
            faces += b::faces(&n).len();
            gaps.extend(open_gaps(&n, options));
            changed.put(d, *g, vec![n])?;
        }
        changed.finish(d)?;
        sel.set(targets.iter().copied());
        Ok((faces, gaps, closed))
    })?;
    Ok(json!({ "ids": ids_json(&targets), "faces": faces, "gaps": gaps_json(&gaps), "closed": closed }))
}

/// Which parts a Live Paint Bucket click paints besides the one under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LpScope {
    /// The part under the pointer alone (a click).
    One,
    /// A face and the faces reached from it across edges that draw no stroke ([`b::flood_faces`]),
    /// or an edge and the edges joined to it end to end that draw with the same stroke paint
    /// ([`b::connected_edges`], [`b::lp_stroke_paint`]) (a double-click).
    Connected,
    /// Every face with the same fill, or every edge that draws with the same stroke paint, in the
    /// group (a triple-click).
    SamePaint,
}

/// The `scope` of `livePaint.fill` and `livePaint.strokeEdge`: `one` when left out or null.
fn scope_param(p: &Value, c: &str) -> Result<LpScope> {
    match p.get("scope").filter(|v| !v.is_null()) {
        None => Ok(LpScope::One),
        Some(v) => match v.as_str() {
            Some("one") => Ok(LpScope::One),
            Some("connected") => Ok(LpScope::Connected),
            Some("samePaint") => Ok(LpScope::SamePaint),
            _ => Err(bad(c, "scope must be \"one\", \"connected\" or \"samePaint\"")),
        },
    }
}

/// The `match` of `livePaint.fill` and `livePaint.strokeEdge`: a paint as the document stores it,
/// None when left out or null. With `allowed` false (a scope that compares no paints) a `match` is
/// an error naming `scopes`, the scopes it applies to.
fn match_param(p: &Value, c: &str, allowed: bool, scopes: &str) -> Result<Option<super::Paint>> {
    let Some(v) = p.get("match").filter(|v| !v.is_null()) else { return Ok(None) };
    if !allowed {
        return Err(bad(c, format!("match applies only with scope {scopes}")));
    }
    serde_json::from_value::<super::Paint>(v.clone()).map(Some).map_err(|_| bad(c, "match must be a paint as the document stores it"))
}

/// The drag path of `livePaint.fill` or `livePaint.strokeEdge` (command `c`) pressed at `start`:
/// `points` followed by each chunk of `continuation`, where every chunk holds 1 to
/// [`b::LP_FILL_CHUNK_POINTS`] finite canvas points and starts at the last point of the chunk
/// before it (`points` at `start`). None when `points` is left out. With a `scope` other than
/// `one`, `points` is an error.
fn path_param(p: &Value, c: &str, start: Point, scope: LpScope) -> Result<Option<Vec<Vec<Point>>>> {
    if scope != LpScope::One && p.get("points").is_some() {
        return Err(bad(c, "scope applies to a click: leave out points"));
    }
    let parse_chunk = |input: &Value, start: Point| -> Result<Vec<Point>> {
        let values = input
            .as_array()
            .filter(|points| !points.is_empty() && points.len() <= b::LP_FILL_CHUNK_POINTS)
            .ok_or_else(|| bad(c, "each path chunk must contain 1 to 4096 points"))?;
        let points: Vec<_> = values
            .iter()
            .map(|point| {
                let pair = point.as_array().filter(|pair| pair.len() == 2).ok_or_else(|| bad(c, "points must be [x,y] pairs"))?;
                let x = pair.first().and_then(Value::as_f64).ok_or_else(|| bad(c, "point x must be a number"))?;
                let y = pair.get(1).and_then(Value::as_f64).ok_or_else(|| bad(c, "point y must be a number"))?;
                let point = Point::new(x, y);
                if !finite(point) {
                    return Err(bad(c, "points must be finite canvas points"));
                }
                Ok(point)
            })
            .collect::<Result<_>>()?;
        if points.first() != Some(&start) {
            return Err(bad(c, "path chunks must share their boundary point"));
        }
        Ok(points)
    };
    let Some(points) = p.get("points") else {
        return if p.get("continuation").is_some() { Err(bad(c, "continuation requires points")) } else { Ok(None) };
    };
    let continuation = match p.get("continuation") {
        None => &[][..],
        Some(input) => input.as_array().ok_or_else(|| bad(c, "continuation must be an array of path chunks"))?.as_slice(),
    };
    let mut chunks: Vec<Vec<Point>> = Vec::with_capacity(continuation.len() + 1);
    let mut last = start;
    for input in std::iter::once(points).chain(continuation) {
        let chunk = parse_chunk(input, last)?;
        last = chunk.last().copied().unwrap_or(last);
        chunks.push(chunk);
    }
    Ok(Some(chunks))
}

pub(super) fn lp_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.fill";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let scope = scope_param(p, C)?;
    let like = match_param(p, C, scope == LpScope::SamePaint, "\"samePaint\"")?;
    let path = path_param(p, C, pt, scope)?;
    crate::cmd::paint::check_keep_model(p, C)?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.fill.clone());
    let group = object_param(p, "group", C)?;
    let roots = if group.is_none() { root_ids(s, p)? } else { vec![] };
    // The faces the click or drag paints in Live Paint group `gid`: the visible face at the point,
    // the faces its scope adds, and every visible face the drag crosses; → (the face at the point,
    // the faces).
    let targets_in = |d: &Document, gid: NodeId| -> Result<(NodeId, std::collections::BTreeSet<NodeId>)> {
        let fid = d
            .node(gid)
            .and_then(|n| b::visible_face_at(n, pt))
            .map(|f| f.id)
            .ok_or_else(|| EngineError::Other("no Live Paint face at that point".into()))?;
        let mut targets = std::collections::BTreeSet::from([fid]);
        if scope != LpScope::One {
            let n = d.node(gid).ok_or(EngineError::NoNode(gid))?;
            if scope == LpScope::Connected {
                targets.extend(b::flood_faces(n, fid));
            } else {
                let like = match &like {
                    Some(m) => m.clone(),
                    None => d.node(fid).ok_or(EngineError::NoNode(fid))?.appearance.fill_paint(),
                };
                targets.extend(b::faces(n).into_iter().filter(|f| f.visible && f.appearance.fill_paint() == like).map(|f| f.id));
            }
        }
        if let Some(chunks) = &path {
            let n = d.node(gid).ok_or(EngineError::NoNode(gid))?;
            // Use the existing region-crossing probes, so even a face narrower than the distance
            // between pointer events is filled. Reversing matches visible_face_at's front first.
            let faces: Vec<_> = b::faces(n).into_iter().rev().filter(|f| f.visible).collect();
            let mut face_ids = vec![];
            let mut regions = vec![];
            for f in faces {
                if let Some(path) = f.path_data()
                    && let Some(bounds) = path.bounds()
                {
                    face_ids.push(f.id);
                    regions.push((po::Region { path: path.clone(), sources: vec![] }, path.to_bezpath(), bounds));
                }
            }
            let map = b::BuilderMap { regions, ..Default::default() };
            for chunk in chunks {
                targets.extend(map.hits_along(chunk, None).regions.iter().filter_map(|i| face_ids.get(*i).copied()));
            }
        }
        Ok((fid, targets))
    };
    // The Live Paint group the fill paints; `None` when the objects are made into one first.
    let live = {
        let d = &s.doc()?.doc;
        match group {
            Some(g) => {
                if !d.node(g).is_some_and(is_live_paint) {
                    return Err(bad(C, format!("{g} is not a Live Paint group")));
                }
                Some(g)
            }
            None => {
                let live_at = |face: fn(&Node, Point) -> Option<&Arc<Node>>| {
                    roots.iter().copied().find(|r| d.node(*r).is_some_and(|n| is_live_paint(n) && face(n, pt).is_some()))
                };
                match (live_at(b::visible_face_at), live_at(face_at)) {
                    (Some(g), _) => Some(g),
                    // At a point on a hidden face of a Live Paint group among the objects, the command returns an error
                    // and does not merge the objects.
                    (None, Some(_)) => return Err(EngineError::Other("no Live Paint face at that point".into())),
                    (None, None) => None,
                }
            }
        }
    };
    let (g, face, count) = match live {
        Some(gid) => {
            let (fid, targets) = targets_in(&s.doc()?.doc, gid)?;
            let count = targets.len();
            // Faces that already have only the fill the bucket would leave ([`bucket_keeps`]) stay as
            // they are: when every face does, nothing changes and no undo step is recorded.
            let d = &s.doc()?.doc;
            let unpainted: Vec<NodeId> =
                targets.into_iter().filter(|t| d.node(*t).is_none_or(|f| !bucket_keeps(&f.appearance, true, &paint, 0.0))).collect();
            if unpainted.is_empty() {
                return Ok(json!({ "group": gid.0, "face": fid.0, "count": count }));
            }
            s.edit("Live Paint Bucket", |d, _| {
                for target in unpainted {
                    bucket_paint(&mut d.node_mut(target).ok_or(EngineError::NoNode(target))?.appearance, true, &paint, 0.0);
                }
                Ok((gid, fid, count))
            })?
        }
        None => s.edit("Live Paint Bucket", |d, sel| {
            let gid = make_live(d, sel, &roots)?.0;
            let (fid, targets) = targets_in(d, gid)?;
            let count = targets.len();
            for target in targets {
                bucket_paint(&mut d.node_mut(target).ok_or(EngineError::NoNode(target))?.appearance, true, &paint, 0.0);
            }
            Ok((gid, fid, count))
        })?,
    };
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "face": face.0, "count": count }))
}

pub(super) fn lp_stroke_edge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.strokeEdge";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let g = object_param(p, "group", C)?.ok_or_else(|| bad(C, "missing group"))?;
    crate::cmd::paint::check_keep_model(p, C)?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.stroke.clone());
    let width = number_or(p, "width", s.paint.stroke_width.max(0.0), C)?;
    if !(0.0..=1000.0).contains(&width) {
        return Err(bad(C, "width out of range"));
    }
    let tol = number_or(p, "tolerance", 4.0, C)?.clamp(0.0, 1000.0);
    let scope = scope_param(p, C)?;
    let like = match_param(p, C, scope != LpScope::One, "\"connected\" or \"samePaint\"")?;
    let path = path_param(p, C, pt, scope)?;
    let through: Vec<NodeId> = match p.get("through") {
        None => vec![],
        Some(_) if scope != LpScope::Connected => return Err(bad(C, "through applies to scope \"connected\"")),
        Some(v) => {
            let ids = v.as_array().filter(|ids| ids.len() <= 4096).ok_or_else(|| bad(C, "through must be an array of at most 4096 edge ids"))?;
            ids.iter()
                .map(|id| id.as_u64().map(NodeId).ok_or_else(|| bad(C, "through must be an array of at most 4096 edge ids")))
                .collect::<Result<_>>()?
        }
    };
    let (eid, count, changed) = {
        let d = &s.doc()?.doc;
        let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad(C, format!("{g} is not a Live Paint group")))?;
        // Each id in `through` is an edge of the group, given once.
        let group_edges: std::collections::HashSet<NodeId> = b::edges(n).iter().map(|e| e.id).collect();
        let mut seen = std::collections::HashSet::new();
        if let Some(id) = through.iter().find(|id| !group_edges.contains(id) || !seen.insert(**id)) {
            return Err(bad(C, format!("through: {id} is not an edge of {g}, or is given twice")));
        }
        let start = b::visible_edge_near(n, pt, tol).ok_or_else(|| EngineError::Other("no Live Paint edge near that point".into()))?;
        let eid = start.id;
        let like = like.unwrap_or_else(|| b::lp_stroke_paint(start));
        let targets = match scope {
            // A drag also reaches every edge of the group that is not hidden and that its path
            // crosses or touches.
            LpScope::One => std::iter::once(eid)
                .chain(
                    path.as_deref()
                        .map(|chunks| b::visible_edges_along(n, chunks))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|e| e.id)
                        .filter(|id| *id != eid),
                )
                .collect(),
            LpScope::Connected => b::connected_edges_through(n, eid, &like, &through),
            LpScope::SamePaint => std::iter::once(eid)
                .chain(b::edges(n).into_iter().filter(|e| e.visible && e.id != eid && b::lp_stroke_paint(e) == like).map(|e| e.id))
                .collect(),
        };
        let count = targets.len();
        // An edge that already has only the stroke the bucket would leave ([`bucket_keeps`]) stays
        // as it is; when every edge does, nothing changes and no undo step is recorded.
        let changed: Vec<NodeId> =
            targets.into_iter().filter(|t| d.node(*t).is_none_or(|e| !bucket_keeps(&e.appearance, false, &paint, width))).collect();
        (eid, count, changed)
    };
    if changed.is_empty() {
        return Ok(json!({ "group": g.0, "edge": eid.0, "count": count }));
    }
    s.edit("Live Paint Bucket", |d, _| {
        for target in &changed {
            bucket_paint(&mut d.node_mut(*target).ok_or(EngineError::NoNode(*target))?.appearance, false, &paint, width);
        }
        Ok(())
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "edge": eid.0, "count": count }))
}

/// Whether the Live Paint Bucket leaves appearance `a` as it is when it paints its fills (`fill`)
/// or strokes with `paint` (a stroke at `width`): it has exactly one row of that kind, shown, fully
/// opaque, blending normally and with the paint (a stroke also with the width, unless the paint is
/// none), or it has no row of that kind and the paint is none.
fn bucket_keeps(a: &Appearance, fill: bool, paint: &super::Paint, width: f64) -> bool {
    let mut rows = a.items.iter().filter(|i| i.is_fill() == fill);
    match (rows.next(), rows.next()) {
        (None, _) => paint.is_none(),
        (Some(row), None) => {
            row.visible()
                && row.opacity() == 1.0
                && row.blend() == vectorcraft_color::BlendMode::Normal
                && row.paint() == paint
                && match row {
                    AppearanceItem::Fill(_) => true,
                    AppearanceItem::Stroke(st) => paint.is_none() || st.width == width,
                }
        }
        (Some(_), Some(_)) => false,
    }
}

/// Paint the fills (`fill`) or strokes of appearance `a` with `paint` (a stroke at `width`), as the
/// Live Paint Bucket does: every row of that kind is replaced by one row, shown, fully opaque and
/// blending normally, in the place of the topmost row of that kind. The new row keeps the other
/// settings (a stroke's dashes, caps and arrowheads, a row's effects) of the topmost row of that
/// kind that is shown, or of the topmost one when every row is hidden. Rows of the other kind stay
/// as they are. With no row of that kind, a fill is added below the strokes and a stroke above the
/// fills.
fn bucket_paint(a: &mut Appearance, fill: bool, paint: &super::Paint, width: f64) {
    let of_kind = |i: &AppearanceItem| i.is_fill() == fill;
    let Some(top) = a.items.iter().rposition(of_kind) else {
        if fill {
            a.set_fill(paint.clone());
        } else {
            a.items.push(AppearanceItem::Stroke(super::StrokeLayer::new(paint.clone(), width)));
        }
        return;
    };
    let base = a.items.iter().rposition(|i| of_kind(i) && i.visible()).unwrap_or(top);
    let Some(mut row) = a.items.get(base).cloned() else { return };
    match &mut row {
        AppearanceItem::Fill(f) => {
            f.paint = paint.clone();
            f.opacity = 1.0;
            f.blend = vectorcraft_color::BlendMode::Normal;
            f.visible = true;
        }
        AppearanceItem::Stroke(st) => {
            st.paint = paint.clone();
            st.width = width;
            st.opacity = 1.0;
            st.blend = vectorcraft_color::BlendMode::Normal;
            st.visible = true;
        }
    }
    let mut row = Some(row);
    a.items = std::mem::take(&mut a.items)
        .into_iter()
        .enumerate()
        .filter_map(|(i, item)| {
            if i == top {
                row.take()
            } else if of_kind(&item) {
                None
            } else {
                Some(item)
            }
        })
        .collect();
}

/// Every fill and stroke row of face or edge `n`, bottom first, as `livePaint.info` lists them. A
/// row is `drawn` when the part is shown with an opacity above 0 and the row is shown, has paint
/// and has an opacity above 0 (a stroke also a weight above 0): the rule of [`draws`], row by row.
fn appearance_rows(n: &Node) -> Vec<Value> {
    let part_shown = n.visible && n.opacity > 0.0;
    n.appearance
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let shown = part_shown && item.visible() && !item.paint().is_none() && item.opacity() > 0.0;
            let mut row = json!({
                "index": i,
                "kind": item.kind_name(),
                "paint": item.paint().label(),
                "opacity": item.opacity(),
                "visible": item.visible(),
            });
            let drawn = match item {
                AppearanceItem::Fill(_) => shown,
                AppearanceItem::Stroke(st) => {
                    row["width"] = json!(st.width);
                    shown && st.width > 0.0
                }
            };
            row["drawn"] = json!(drawn);
            row
        })
        .collect()
}

pub(super) fn lp_info(s: &mut Session, p: &Value) -> Result<Value> {
    let g = match object_param(p, "group", "livePaint.info")? {
        Some(g) => g,
        None => object_param(p, "id", "livePaint.info")?.ok_or_else(|| bad("livePaint.info", "missing group"))?,
    };
    let d = &s.doc()?.doc;
    let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad("livePaint.info", format!("{g} is not a Live Paint group")))?;
    let faces: Vec<Value> = b::faces(n)
        .iter()
        .map(|f| {
            let bb = f.geometric_bounds().unwrap_or(Rect::ZERO);
            json!({
                "id": f.id.0,
                "fill": f.appearance.fill_paint().label(),
                "bounds": [bb.x0, bb.y0, bb.x1, bb.y1],
                "appearance": appearance_rows(f),
            })
        })
        .collect();
    let edges: Vec<Value> = b::edges(n)
        .iter()
        .map(|e| {
            json!({
                "id": e.id.0,
                "stroke": e.appearance.stroke_paint().label(),
                "width": e.appearance.stroke_width(),
                "appearance": appearance_rows(e),
            })
        })
        .collect();
    let sources = b::sources(n).and_then(|g| g.children()).map(|c| c.len()).unwrap_or(0);
    let options = n.gap_options();
    Ok(json!({
        "faces": faces,
        "edges": edges,
        "sources": sources,
        "gapOptions": gap_options_json(options),
        "gaps": gaps_json(&open_gaps(n, options)),
    }))
}
