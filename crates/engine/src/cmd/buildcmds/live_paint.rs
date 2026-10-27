use super::*;

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
        return merge_live(d, sel, roots);
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

pub(super) fn lp_make(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = root_ids(s, p)?;
    let (id, faces, edges) = s.edit("Make Live Paint", |d, sel| make_live(d, sel, &roots))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
}

/// Add the other `roots` (back → front) to the first Live Paint group among them, which keeps its
/// id, the paint of its faces and edges, and its own properties apart from the Gap Options, which
/// are merged; returns (group, faces, edges).
fn merge_live(d: &mut Document, sel: &mut Selection, roots: &[NodeId]) -> Result<(NodeId, usize, usize)> {
    let target = roots
        .iter()
        .copied()
        .find(|r| d.node(*r).is_some_and(is_live_paint))
        .ok_or_else(|| EngineError::Other("Live Paint Merge: select a Live Paint group and the paths to add".into()))?;
    check_live_paint_takes(d, roots)?;
    let old = d.node(target).cloned().ok_or(EngineError::NoNode(target))?;
    // Paint to carry over: (point inside face, fill) and (point on edge, stroke).
    let keep_faces: Vec<(Point, Paint)> = b::faces(&old)
        .iter()
        .filter(|f| !f.appearance.fill_paint().is_none())
        .filter_map(|f| Some((po::interior_point(f.path_data()?)?, f.appearance.fill_paint())))
        .collect();
    let keep_edges: Vec<(Point, Appearance)> = b::edges(&old)
        .iter()
        .filter_map(|e| {
            let pd = e.path_data()?;
            let sp = pd.subpaths.first().filter(|sp| sp.segment_count() > 0)?;
            Some((kurbo::ParamCurve::eval(&sp.segment(0), 0.5), (*e.appearance).clone()))
        })
        .collect();
    let mut sources = source_nodes(&old);
    for r in roots {
        if *r != target {
            sources.extend(source_nodes(d.node(*r).ok_or(EngineError::NoNode(*r))?));
        }
    }
    // Merge applies the Gap Options' merge rules to the groups' options, bottom-most first.
    let groups: Vec<GapOptions> = roots.iter().filter_map(|r| d.node(*r)).filter(|n| is_live_paint(n)).map(Node::gap_options).collect();
    let options = merged_gap_options(&groups);
    let (children, nf, ne) = build_children(d, sources, options.max_gap())?;
    // The group keeps its name, lock, visibility, transparency, appearance, opacity mask, graphic
    // style link and the rest; its children are made again and its Gap Options set to the merged ones.
    let mut g = old;
    if let Some(ch) = g.children_mut() {
        *ch = children;
    }
    g.extras_mut().live_paint_gaps = stored_gap_options(options);
    for (pt, paint) in keep_faces {
        if let Some(fid) = face_at(&g, pt).map(|f| f.id)
            && let Some(ch) = g.children_mut()
            && let Some(f) = ch.iter_mut().find(|c| c.id == fid)
        {
            Arc::make_mut(f).appearance.set_fill(paint);
        }
    }
    for (pt, app) in keep_edges {
        if let Some(eid) = edge_near(&g, pt, 0.5).map(|e| e.id)
            && let Some(ch) = g.children_mut()
            && let Some(e) = ch.iter_mut().find(|c| c.id == eid)
        {
            Arc::make_mut(e).appearance = app.into();
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
    let (id, faces, edges) = s.edit("Merge Live Paint", |d, sel| merge_live(d, sel, &roots))?;
    Ok(json!({ "id": id.0, "faces": faces, "edges": edges }))
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
            let srcs: Vec<Node> = source_nodes(&n).iter().map(|c| (**c).clone()).collect();
            out.extend(srcs.iter().map(|c| c.id));
            released.put(d, *g, srcs)?;
        }
        released.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
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
            if let Some(ch) = n.children_mut() {
                ch.retain(|c| match c.name.as_deref() {
                    Some(SOURCES_NAME) => false,
                    Some(FACE_NAME) => !c.appearance.fill_paint().is_none(),
                    Some(EDGE_NAME) => !c.appearance.stroke_paint().is_none() && c.appearance.stroke_width() > 0.0,
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
    let (faces, gaps, closed) = s.edit("Gap Options", |d, sel| {
        let (mut faces, mut gaps, mut closed) = (0, vec![], 0);
        let mut changed = Beside::replacing(d, &targets);
        for g in &targets {
            let old = d.node(*g).cloned().ok_or(EngineError::NoNode(*g))?;
            let mut options = old.gap_options();
            options.detect = detect.unwrap_or(options.detect);
            options.size = size.unwrap_or(options.size);
            options.color = color.unwrap_or(options.color);
            let mut sources = source_nodes(&old);
            if close {
                // Close Gaps With Paths adds an unpainted line across each gap to the group's paths.
                for gap in po::find_gaps(&source_shapes(&sources).0, options.size.points()) {
                    let line = PathData::single(SubPath::polyline(&gap, false));
                    sources.push(Arc::new(Node::path(d.alloc_id(), line, Appearance::default())));
                    closed += 1;
                }
            }
            let mut n = old.clone();
            // The faces change only with what paint stops at, or with new paths.
            if close || options.max_gap() != old.gap_options().max_gap() {
                let (children, _, _) = build_children(d, sources, options.max_gap())?;
                if let Some(ch) = n.children_mut() {
                    *ch = children;
                }
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

pub(super) fn lp_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.fill";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    crate::cmd::paint::check_keep_model(p, C)?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.fill.clone());
    let group = id_param(p, "group");
    let roots = if group.is_none() { root_ids(s, p)? } else { vec![] };
    let (g, face) = s.edit("Live Paint Bucket", |d, sel| {
        let gid = match group {
            Some(g) => {
                if !d.node(g).is_some_and(is_live_paint) {
                    return Err(bad(C, format!("{g} is not a Live Paint group")));
                }
                g
            }
            None => {
                let live_at = |face: fn(&Node, Point) -> Option<&Arc<Node>>| {
                    roots.iter().copied().find(|r| d.node(*r).is_some_and(|n| is_live_paint(n) && face(n, pt).is_some()))
                };
                match (live_at(b::visible_face_at), live_at(face_at)) {
                    (Some(g), _) => g,
                    // At a point on a hidden face of a Live Paint group among the objects, the command returns an error
                    // and does not merge the objects.
                    (None, Some(_)) => return Err(EngineError::Other("no Live Paint face at that point".into())),
                    (None, None) => make_live(d, sel, &roots)?.0,
                }
            }
        };
        let fid = d
            .node(gid)
            .and_then(|n| b::visible_face_at(n, pt))
            .map(|f| f.id)
            .ok_or_else(|| EngineError::Other("no Live Paint face at that point".into()))?;
        d.node_mut(fid).ok_or(EngineError::NoNode(fid))?.appearance.set_fill(paint.clone());
        Ok((gid, fid))
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "face": face.0 }))
}

pub(super) fn lp_stroke_edge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "livePaint.strokeEdge";
    let pt = point_param(p, "point").filter(|q| finite(*q)).ok_or_else(|| bad(C, "missing point [x,y]"))?;
    let g = id_param(p, "group").ok_or_else(|| bad(C, "missing group"))?;
    crate::cmd::paint::check_keep_model(p, C)?;
    let paint = paint_from(s, p)?.unwrap_or_else(|| s.paint.stroke.clone());
    let width = number_or(p, "width", s.paint.stroke_width.max(0.0), C)?;
    if !(0.0..=1000.0).contains(&width) {
        return Err(bad(C, "width out of range"));
    }
    let tol = number_or(p, "tolerance", 4.0, C)?.clamp(0.0, 1000.0);
    let edge = s.edit("Live Paint Bucket", |d, _| {
        let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad(C, format!("{g} is not a Live Paint group")))?;
        let eid = b::visible_edge_near(n, pt, tol).map(|e| e.id).ok_or_else(|| EngineError::Other("no Live Paint edge near that point".into()))?;
        let e = d.node_mut(eid).ok_or(EngineError::NoNode(eid))?;
        e.appearance.set_stroke(paint.clone());
        if let Some(st) = e.appearance.stroke_mut() {
            st.width = width;
        }
        Ok(eid)
    })?;
    s.remember_paint(&paint);
    Ok(json!({ "group": g.0, "edge": edge.0 }))
}

pub(super) fn lp_info(s: &mut Session, p: &Value) -> Result<Value> {
    let g = id_param(p, "group").or_else(|| id_param(p, "id")).ok_or_else(|| bad("livePaint.info", "missing group"))?;
    let d = &s.doc()?.doc;
    let n = d.node(g).filter(|n| is_live_paint(n)).ok_or_else(|| bad("livePaint.info", format!("{g} is not a Live Paint group")))?;
    let faces: Vec<Value> = b::faces(n)
        .iter()
        .map(|f| {
            let bb = f.geometric_bounds().unwrap_or(Rect::ZERO);
            json!({ "id": f.id.0, "fill": f.appearance.fill_paint().label(), "bounds": [bb.x0, bb.y0, bb.x1, bb.y1] })
        })
        .collect();
    let edges: Vec<Value> = b::edges(n)
        .iter()
        .map(|e| json!({ "id": e.id.0, "stroke": e.appearance.stroke_paint().label(), "width": e.appearance.stroke_width() }))
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
