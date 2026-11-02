use super::{
    Arc, Beside, BuilderMap, EngineError, FillRule, Node, NodeId, PathData, Point, Rect, Result, Session, Touched, Value, b, bad, check_numbers,
    f64_or, fill_param, finite, gap_param, ids_json, json, marquee_param, po, points_param, remnant, replace_leaves, replace_with_pieces, root_ids,
    shape_node, switch_or,
};

pub(super) fn sb_merge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shapeBuilder.merge";
    let marquee = marquee_param(p, C)?;
    let pts = match points_param(p, "points") {
        Some(pts) => pts,
        // A marquee needs no drag path.
        None if marquee.is_some() && p.get("points").is_none_or(Value::is_null) => vec![],
        None => return Err(bad(C, "missing points [[x,y]…]")),
    };
    if (pts.is_empty() && marquee.is_none()) || !pts.iter().all(|q| finite(*q)) {
        return Err(bad(C, "points must be a non-empty list of finite [x,y]"));
    }
    let erase = switch_or(p, "erase", false, C)?;
    check_numbers(p, &["tolerance", "gap"], C)?;
    // Line pieces and edges are only deleted (erase mode), never merged.
    let piece_tol = erase.then(|| f64_or(p, "tolerance", 2.0).clamp(0.0, 1000.0));
    // In Merge Mode, Clicking Stroke Splits the Path: a click (one point) on a stroke splits it; a
    // marquee does not.
    let split_tol = (switch_or(p, "splitStroke", false, C)? && !erase && pts.len() == 1 && marquee.is_none())
        .then(|| f64_or(p, "tolerance", 2.0).clamp(0.0, 1000.0));
    let open_filled_as_closed = switch_or(p, "openFilledAsClosed", true, C)?;
    let swatches = match p.get("pickColorFrom") {
        None | Some(Value::Null) => false,
        Some(v) => match v.as_str() {
            Some("artwork") => false,
            Some("swatches") => true,
            _ => return Err(bad(C, format!("pickColorFrom is \"artwork\" or \"swatches\", not {v}"))),
        },
    };
    let fill = match fill_param(s, p, C)? {
        None if swatches => Some(s.paint.fill.clone()),
        f => f,
    };
    let roots = root_ids(s, p)?;
    let gap = gap_param(p);
    // The map the Shape Builder tool already has, which a click or drag of it is made over.
    let shared = s.tool.shape_builder_map().filter(|(_, g)| *g == gap);
    let prepared = s.tool.shape_builder_merge();
    let label = if erase { "Shape Builder Delete" } else { "Shape Builder" };
    let (ids, merged, touched, split) = s.edit(label, |d, sel| {
        let (shapes, leaves) = b::shape_builder_shapes(d, &roots, open_filled_as_closed);
        let built;
        let map = match &shared {
            // The tool's map has the outline edges, which erasing and splitting need.
            Some((m, _)) if m.shapes == shapes => m.as_ref(),
            _ => {
                built = BuilderMap::of_shapes(shapes, erase || split_tol.is_some(), gap);
                &built
            }
        };
        let outlines = b::drawn_outlines(d, &leaves);
        let leaves: Vec<Node> = leaves.into_iter().cloned().collect();
        if map.shapes.is_empty() {
            return Err(EngineError::Other("Shape Builder: select paths or compound paths".into()));
        }
        if let Some(tol) = split_tol
            && let Some(&q) = pts.first()
            && let Some((i, run)) = map.stroke_at(&outlines, q, tol)
            && let (Some(leaf), Some(drawn)) = (leaves.get(i), outlines.get(i))
            && let Some(rest) = po::cut_out(drawn, &run, open_filled_as_closed && !leaf.appearance.fill_paint().is_none(), b::outline_snap(drawn))
            && rest.length() > b::outline_snap(drawn)
        {
            // The path keeps the rest of it, and a copy of it with the run under the click goes
            // directly above it; the other objects stay as they are. Gradients fitted to the
            // path's bounds keep that fit on both.
            if let Some(n) = d.node_mut(leaf.id) {
                n.pin_gradients();
            }
            let pieces = [rest, PathData::single(run)].into_iter().filter(|pd| !pd.is_empty()).collect();
            let mut replaced = Beside::replacing(d, &[leaf.id]);
            let new_ids = replace_with_pieces(d, &mut replaced, leaf.id, pieces)?;
            replaced.finish(d)?;
            let split = *new_ids.last().ok_or(EngineError::NoNode(leaf.id))?;
            // Pick Color From Color Swatches (or `fill`): the stroke takes that paint.
            if let Some(f) = &fill
                && let Some(n) = d.node_mut(split)
            {
                n.appearance.set_stroke(f.clone());
            }
            // The roots stay selected, with the two paths in place of the path when it was one.
            let ids: Vec<NodeId> = roots.iter().flat_map(|r| if *r == leaf.id { new_ids.clone() } else { vec![*r] }).collect();
            sel.set(ids.iter().copied());
            let mut touched = Touched::default();
            touched.add(map.hit(q, Some(tol)));
            return Ok((ids, None, touched, Some(split)));
        }
        let touched = map.merge_touched(&pts, marquee, piece_tol);
        if touched.is_empty() {
            return Err(EngineError::Other("Shape Builder: no region under the pointer".into()));
        }
        let inputs = b::merge_inputs(&leaves.iter().collect::<Vec<_>>(), open_filled_as_closed);
        // The tool computed this merge on another thread when it would come out the same.
        let ready = prepared.as_ref().filter(|m| {
            shared.as_ref().is_some_and(|(s, _)| Arc::ptr_eq(s, &m.map) && std::ptr::eq(s.as_ref(), map))
                && m.gap == gap
                && m.touched == touched
                && m.inputs == inputs
        });
        let computed;
        let geometry = match ready {
            Some(m) => &m.geometry,
            None => {
                computed = b::merge_geometry(map, &touched, gap, &inputs);
                &computed
            }
        };
        let picked: Vec<&po::Region> = touched.regions.iter().filter_map(|&i| map.regions.get(i).map(|r| &r.0)).collect();
        let union = &geometry.union;
        // Pick Color From Artwork: the merged shape takes the look of the front-most object under the
        // drag's first point, else under its last point, else of the front-most object. Color
        // Swatches: the current fill, with the rest of the look of the front-most object over the
        // first region the drag touched that an object covers, else of the front-most object. A
        // marquee's drag starts at its first corner and ends at the opposite one.
        let top_at = |q: &Point| map.region_at(*q).and_then(|k| map.regions.get(k)).and_then(|(r, ..)| r.top());
        let (first, last) = match marquee {
            Some((a, b)) => (Some(a), Some(b)),
            None => (pts.first().copied(), pts.last().copied()),
        };
        let covered =
            if swatches { picked.iter().find_map(|r| r.top()) } else { first.as_ref().and_then(top_at).or_else(|| last.as_ref().and_then(top_at)) };
        let style_src = covered.or(map.shapes.len().checked_sub(1));
        // What goes in place of each object the map was made from (none: it stays as it is).
        let mut outs: Vec<Option<Vec<Node>>> = vec![];
        let mut merged_id = None;
        for (i, leaf) in leaves.iter().take(map.shapes.len()).enumerate() {
            let mut out = None;
            let pieces: Vec<usize> = map.lines.iter().enumerate().filter(|(_, (l, _))| l.key == i as u64).map(|(k, _)| k).collect();
            if !pieces.is_empty() {
                // An open path loses the pieces deleted and those between two merged (or deleted) regions.
                let keep: Vec<usize> =
                    pieces.iter().copied().filter(|k| !touched.lines.contains(k) && !map.line_between(*k, &touched.regions)).collect();
                if keep.len() != pieces.len() {
                    let mut kept = vec![];
                    for k in keep {
                        if let Some((l, _)) = map.lines.get(k) {
                            kept.push(remnant(d, leaf, l.path.clone(), None));
                        }
                    }
                    out = Some(kept);
                }
            } else if let Some(Some(rest)) = geometry.rests.get(i) {
                // A shape loses the regions deleted or merged, then the edges deleted (each one on
                // its outline, shared edges from every shape along them). What is left of its own
                // outline keeps its fill rule; united faces fill with non-zero.
                out = Some(if rest.path.is_empty() {
                    vec![]
                } else {
                    vec![remnant(d, leaf, rest.path.clone(), rest.united.then_some(FillRule::NonZero))]
                });
            }
            // The merged shape goes in place of the object it takes its look from, so it stays in
            // that object's group (a clip group too).
            if !erase && !union.is_empty() && Some(i) == style_src {
                let mut m = shape_node(d, union.clone(), Some(leaf));
                if let Some(f) = &fill {
                    m.appearance.set_fill(f.clone());
                }
                merged_id = Some(m.id);
                out.get_or_insert_with(|| vec![leaf.clone()]).push(m);
            }
            outs.push(out);
        }
        let new_ids = replace_leaves(d, &roots, &leaves, outs)?;
        sel.set(new_ids.iter().copied());
        Ok((new_ids, merged_id, touched, None))
    })?;
    let mut out = json!({
        "ids": ids_json(&ids),
        "merged": merged.map(|m| m.0),
        "regions": touched.regions.len(),
        "lines": touched.lines.len(),
        "edges": touched.edges.len(),
    });
    if let Some(e) = split {
        out["split"] = json!(e.0);
    }
    Ok(out)
}

pub(super) fn sb_regions(s: &mut Session, p: &Value) -> Result<Value> {
    let open_filled_as_closed = switch_or(p, "openFilledAsClosed", true, "shapeBuilder.regions")?;
    check_numbers(p, &["gap"], "shapeBuilder.regions")?;
    let roots = root_ids(s, p)?;
    let d = &s.doc()?.doc;
    let (map, leaves) = BuilderMap::with_gap(d, &roots, true, open_filled_as_closed, gap_param(p));
    let id_of = |i: usize| leaves.get(i).map(|n| n.id.0);
    let regions: Vec<Value> = map
        .regions
        .iter()
        .map(|(r, _, bb)| {
            json!({
                "bounds": [bb.x0, bb.y0, bb.x1, bb.y1],
                "area": po::area(&r.path, FillRule::NonZero),
                "sources": r.sources.iter().filter_map(|&i| id_of(i)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let pieces = |list: &[(po::Shape, Rect)]| -> Vec<Value> {
        list.iter().map(|(l, bb)| json!({ "bounds": [bb.x0, bb.y0, bb.x1, bb.y1], "source": id_of(l.key as usize) })).collect()
    };
    Ok(json!({ "regions": regions, "lines": pieces(&map.lines), "edges": pieces(&map.edges) }))
}
