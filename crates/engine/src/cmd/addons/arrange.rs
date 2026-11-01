//! Arrange Tools (`addon.arrange.*`), a built-in add-on: Arrange in Grid lays the selected objects
//! out in rows and columns, a row or a column; Swap Objects swaps the places (and sizes) of two;
//! Replace with Top Object puts a copy of the topmost selected object in place of each of the
//! others; and Set Size of Each gives every selected object an exact width and height. Places and
//! sizes are measured and set as `object.setBounds` does. Group Overlapping Objects groups the
//! objects whose visual bounds overlap. Its submenu (Object › Plug-ins › Arrange
//! Tools), its dialogs and Zoom to Selection (a view command) are the UI's.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, OrientedBox};
use vectorcraft_geom::{Affine, Point, Rect};
use vectorcraft_render::effects;

use super::super::edit::selected_roots;
use super::super::object::{choice, object_bounds_transform, scaling, transform_node, transform_patterns};
use super::super::typecmd::refresh_bounds_of;
use super::super::{CommandSpec, bad, cmd, has_doc, has_multi, has_selection, number_or, switch_or};
use crate::{EngineError, Result, Session};

/// The largest gap, width or height (pt) the commands take.
const LIMIT: f64 = 1e6;
/// The smallest width or height (pt) Set Size of Each takes.
const LEAST: f64 = 0.001;
/// The most columns Arrange in Grid takes.
const MAX_COLUMNS: u64 = 10_000;
/// The most objects Group Overlapping Objects looks at.
const MAX_OBJECTS: usize = 20_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.arrange.grid",
            "Arrange in Grid",
            ["Object", "Plug-ins", "Arrange Tools"],
            None,
            "{layout?: \"grid\" (rows and columns) | \"row\" (one row) | \"column\" (one column), columns?: 1..10000 (grid; default ⌈√count⌉; never more than the count), gapX?: pt −1000000..1000000 (10), gapY?: pt −1000000..1000000 (gapX), centerOnArtboard?: false (true: the grid is centred on the artboard under the selection's centre, else the first; false: it starts at the selection's top-left)} lay the selected objects (the top-level ones) out in their stacking order from the back, left to right and top to bottom, as one undo step. Each cell is as wide as the widest object in its column and as tall as the tallest in its row, each object is centred in its cell, and everything is measured by visual bounds (strokes and effects as drawn) → {count}",
            has_selection,
            grid
        ),
        cmd!(
            "addon.arrange.swap",
            "Swap Objects",
            ["Object", "Plug-ins", "Arrange Tools"],
            None,
            "{size?: false (true: each also takes the other's width and height), scaleStrokes?: the Scale Strokes & Effects preference} swap the places (bounds centres) of the two selected objects, measured as object.setBounds measures them (Use Preview Bounds), as one undo step. Off unless exactly two objects are selected",
            exactly_two,
            swap
        ),
        cmd!(
            "addon.arrange.replace",
            "Replace with Top Object",
            ["Object", "Plug-ins", "Arrange Tools"],
            None,
            "{fit?: \"keep\" (the copy keeps its size) | \"bounds\" (scaled to the replaced object's width and height) | \"proportional\" (scaled to fit inside them, in proportion) (\"keep\"), scaleStrokes?: the Scale Strokes & Effects preference, keep?: false (true: the replaced objects stay, under their copies)} put a copy of the topmost selected object centred on each of the other selected objects, in that object's place in the stacking order, as one undo step; sizes and centres are measured as object.setBounds measures them, and the copies are selected → {ids}",
            has_multi,
            replace
        ),
        cmd!(
            "addon.arrange.size",
            "Set Size of Each",
            ["Object", "Plug-ins", "Arrange Tools"],
            None,
            "{width?: pt, height?: pt (at least one; each 0.001..1000000), proportional?: false (with only one of them given, the other follows in proportion; else it stays), reference?: 0..8 (the 9-point grid row by row from the top-left; 4, the centre: the point of each object that stays put), scaleStrokes?: the Scale Strokes & Effects preference} give each selected object (the top-level ones) that width and height, measured as object.setBounds measures them (Use Preview Bounds), as one undo step → {count}",
            has_selection,
            size
        ),
        cmd!(
            "addon.arrange.groupOverlapping",
            "Group Overlapping Objects",
            ["Object", "Plug-ins", "Arrange Tools"],
            None,
            "{} group the selected objects (the top-level ones, at most 20000) wherever their visual bounds overlap or touch, chains included (when A overlaps B and B overlaps C, all three go in one group), as one undo step. Only objects in the same layer or group are grouped together, objects that overlap nothing stay as they are, and each new group goes where its frontmost member was in the stacking order; the new groups and the objects left alone stay selected → {groups: [id…], grouped (how many objects went into groups)}",
            has_multi,
            group_overlapping
        ),
    ]
}

/// Exactly two objects are selected (Swap Objects).
fn exactly_two(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    let n = selected_roots(s).map_err(|e| e.to_string())?.len();
    if n == 2 { Ok(()) } else { Err("select exactly two objects".into()) }
}

/// Number param `key` (`default` when left out) within `lo..=hi`.
fn number_in(p: &Value, key: &str, default: f64, (lo, hi): (f64, f64), cmd: &str) -> Result<f64> {
    let v = number_or(p, key, default, cmd)?;
    if (lo..=hi).contains(&v) { Ok(v) } else { Err(bad(cmd, format!("`{key}` must be from {lo} to {hi}"))) }
}

/// Whole-number param `key` within `lo..=hi`, `None` when left out.
fn whole_in(p: &Value, key: &str, (lo, hi): (u64, u64), cmd: &str) -> Result<Option<u64>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let n = v.as_u64().ok_or_else(|| bad(cmd, format!("`{key}` must be a whole number")))?;
    if (lo..=hi).contains(&n) { Ok(Some(n)) } else { Err(bad(cmd, format!("`{key}` must be from {lo} to {hi}"))) }
}

/// A width or height param of Set Size of Each, `None` when left out.
fn size_param(p: &Value, key: &str, cmd: &str) -> Result<Option<f64>> {
    p.get(key).filter(|v| !v.is_null()).map(|_| number_in(p, key, 0.0, (LEAST, LIMIT), cmd)).transpose()
}

/// `object.setBounds` params that centre an object on `at`, with Scale Strokes & Effects `strokes`
/// and, when given, a width and height.
fn place(at: Point, size: Option<(f64, f64)>, strokes: bool) -> Value {
    let mut q = json!({ "x": at.x, "y": at.y, "reference": 4, "strokes": strokes });
    if let Some((w, h)) = size {
        q["width"] = json!(w);
        q["height"] = json!(h);
    }
    q
}

/// Transform each object of `xfs` by its own transform as undo step `label`, strokes and effects
/// scaling with Scale Strokes & Effects `strokes`, pattern tiles as Transform Pattern Tiles says.
/// When every transform is (within rounding) the identity, nothing changes.
fn transform_each(s: &mut Session, label: &str, strokes: bool, xfs: &[(NodeId, Affine)]) -> Result<()> {
    let still = |xf: &Affine| xf.as_coeffs().iter().zip(Affine::IDENTITY.as_coeffs()).all(|(a, b)| (a - b).abs() <= 1e-9);
    if xfs.iter().all(|(_, xf)| still(xf)) {
        return Ok(());
    }
    let p = json!({ "strokes": strokes });
    let ids: Vec<NodeId> = xfs.iter().map(|(id, _)| *id).collect();
    let mut sc = scaling(s, &p);
    sc.patterns = transform_patterns(s, &p, &ids)?;
    s.edit(label, |d, _| {
        for (id, xf) in xfs {
            if let Some(n) = d.node_mut(*id) {
                transform_node(n, *xf, sc, true);
            }
        }
        Ok(())
    })
}

/// The box of object `id` as `object.setBounds` measures it.
fn box_of(s: &Session, id: NodeId) -> Result<OrientedBox> {
    s.transform_box(&[id]).ok_or_else(|| EngineError::Other("no bounds".into()))
}

fn grid(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.arrange.grid";
    let layout = choice(p, C, "layout", &["grid", "row", "column"])?.unwrap_or("grid");
    let columns = whole_in(p, "columns", (1, MAX_COLUMNS), C)?;
    let gap_x = number_in(p, "gapX", 10.0, (-LIMIT, LIMIT), C)?;
    let gap_y = number_in(p, "gapY", gap_x, (-LIMIT, LIMIT), C)?;
    let center = switch_or(p, "centerOnArtboard", false, C)?;
    let roots = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let items: Vec<(NodeId, Rect)> = roots.iter().filter_map(|id| Some((*id, effects::preview::bounds_of(d, &[*id], true)?))).collect();
    let Some(all) = items.iter().map(|(_, b)| *b).reduce(|a, b| a.union(b)) else { return Ok(json!({ "count": 0 })) };
    let n = items.len();
    let cols = match layout {
        "row" => n,
        "column" => 1,
        _ => columns.map_or_else(|| (n as f64).sqrt().ceil() as usize, |c| usize::try_from(c).unwrap_or(n)),
    }
    .clamp(1, n);
    let mut widths = vec![0.0_f64; cols];
    let mut heights = vec![0.0_f64; n.div_ceil(cols)];
    for (i, (_, b)) in items.iter().enumerate() {
        if let Some(w) = widths.get_mut(i % cols) {
            *w = w.max(b.width());
        }
        if let Some(h) = heights.get_mut(i / cols) {
            *h = h.max(b.height());
        }
    }
    let total = |sizes: &[f64], gap: f64| sizes.iter().sum::<f64>() + gap * sizes.len().saturating_sub(1) as f64;
    let origin = if center {
        let board = d.artboards.get(d.artboard_at(all.center()).unwrap_or(0)).map_or(all, |a| a.rect);
        Point::new(board.center().x - total(&widths, gap_x) / 2.0, board.center().y - total(&heights, gap_y) / 2.0)
    } else {
        Point::new(all.x0, all.y0)
    };
    // The middle of each column (row): the sizes and gaps before it, and half its own size.
    let middles = |sizes: &[f64], gap: f64, from: f64| -> Vec<f64> {
        sizes
            .iter()
            .scan(from, |at, size| {
                let mid = *at + size / 2.0;
                *at += size + gap;
                Some(mid)
            })
            .collect()
    };
    let (xs, ys) = (middles(&widths, gap_x, origin.x), middles(&heights, gap_y, origin.y));
    let xfs: Vec<(NodeId, Affine)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, (id, b))| Some((*id, Affine::translate(Point::new(*xs.get(i % cols)?, *ys.get(i / cols)?) - b.center()))))
        .collect();
    transform_each(s, "Arrange in Grid", false, &xfs)?;
    Ok(json!({ "count": n }))
}

fn swap(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.arrange.swap";
    let size = switch_or(p, "size", false, C)?;
    let strokes = switch_or(p, "scaleStrokes", s.prefs.scale_strokes, C)?;
    let roots = selected_roots(s)?;
    let &[a, b] = roots.as_slice() else { return Err(bad(C, "select exactly two objects")) };
    let (ba, bb) = (box_of(s, a)?, box_of(s, b)?);
    let to = |o: &OrientedBox| place(o.center(), size.then(|| (o.rect.width(), o.rect.height())), strokes);
    let xfs = [(a, object_bounds_transform(s, &to(&bb), a)?), (b, object_bounds_transform(s, &to(&ba), b)?)];
    transform_each(s, "Swap Objects", strokes, &xfs)?;
    Ok(json!({}))
}

fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.arrange.replace";
    let fit = choice(p, C, "fit", &["keep", "bounds", "proportional"])?.unwrap_or("keep");
    let strokes = switch_or(p, "scaleStrokes", s.prefs.scale_strokes, C)?;
    let keep = switch_or(p, "keep", false, C)?;
    let roots = selected_roots(s)?;
    let Some((&top, others)) = roots.split_last().filter(|(_, others)| !others.is_empty()) else {
        return Err(bad(C, "select at least two objects"));
    };
    let tb = box_of(s, top)?;
    let (tw, th) = (tb.rect.width(), tb.rect.height());
    let mut xfs = Vec::with_capacity(others.len());
    for &id in others {
        let Some(b) = s.transform_box(&[id]) else { continue };
        let (w, h) = (b.rect.width(), b.rect.height());
        let size = match fit {
            "bounds" => Some((w, h)),
            // The smaller of the two scales that fit; a side the copy doesn't have doesn't count.
            "proportional" => {
                [(w, tw), (h, th)].into_iter().filter(|(_, t)| *t > 1e-9).map(|(v, t)| v / t).reduce(f64::min).map(|k| (tw * k, th * k))
            }
            _ => None,
        };
        xfs.push((id, object_bounds_transform(s, &place(b.center(), size, strokes), top)?));
    }
    let p = json!({ "strokes": strokes });
    let mut sc = scaling(s, &p);
    sc.patterns = transform_patterns(s, &p, &[top])?;
    let ids = s.edit("Replace with Top Object", |d, sel| {
        let Some(n) = d.node(top).cloned() else { return Ok(vec![]) };
        let mut items = Vec::with_capacity(xfs.len());
        for (id, xf) in &xfs {
            let mut c = d.reid(&n);
            transform_node(&mut c, *xf, sc, true);
            items.push((*id, vec![c]));
        }
        let out: Vec<NodeId> = items.iter().flat_map(|(_, c)| c.iter().map(|n| n.id)).collect();
        d.insert_beside(items, !keep)?;
        refresh_bounds_of(d, &out);
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn size(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.arrange.size";
    let width = size_param(p, "width", C)?;
    let height = size_param(p, "height", C)?;
    if width.is_none() && height.is_none() {
        return Err(bad(C, "give `width` or `height`"));
    }
    let proportional = switch_or(p, "proportional", false, C)?;
    let reference = whole_in(p, "reference", (0, 8), C)?.unwrap_or(4);
    let strokes = switch_or(p, "scaleStrokes", s.prefs.scale_strokes, C)?;
    let mut q = json!({ "reference": reference, "strokes": strokes, "proportional": proportional && (width.is_none() || height.is_none()) });
    if let Some(w) = width {
        q["width"] = json!(w);
    }
    if let Some(h) = height {
        q["height"] = json!(h);
    }
    let mut xfs = vec![];
    for id in selected_roots(s)? {
        if s.transform_box(&[id]).is_some() {
            xfs.push((id, object_bounds_transform(s, &q, id)?));
        }
    }
    transform_each(s, "Set Size of Each", strokes, &xfs)?;
    Ok(json!({ "count": xfs.len() }))
}

/// The sets of two or more of `items` (each in the order given) whose rects overlap or touch,
/// chains included, among items of the same key (parent). A sweep along x: only the rects still
/// open where each one starts are compared with it.
fn overlapping<K: PartialEq>(items: &[(K, Rect)]) -> Vec<Vec<usize>> {
    fn root(up: &mut [usize], mut i: usize) -> usize {
        while up[i] != i {
            up[i] = up[up[i]];
            i = up[i];
        }
        i
    }
    let mut up: Vec<usize> = (0..items.len()).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|a, b| items[*a].1.x0.total_cmp(&items[*b].1.x0));
    let mut open: Vec<usize> = vec![];
    for i in order {
        let (key, r) = (&items[i].0, items[i].1);
        open.retain(|&j| items[j].1.x1 >= r.x0);
        for &j in &open {
            let (k, o) = (&items[j].0, items[j].1);
            if k == key && o.y0 <= r.y1 && r.y0 <= o.y1 {
                let (a, b) = (root(&mut up, i), root(&mut up, j));
                up[a] = b;
            }
        }
        open.push(i);
    }
    let mut sets: Vec<Vec<usize>> = vec![vec![]; items.len()];
    for i in 0..items.len() {
        let r = root(&mut up, i);
        sets[r].push(i);
    }
    sets.retain(|s| s.len() > 1);
    sets.sort_by_key(|s| s.first().copied());
    sets
}

fn group_overlapping(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "addon.arrange.groupOverlapping";
    let roots = selected_roots(s)?;
    if roots.len() > MAX_OBJECTS {
        return Err(bad(C, "too many objects selected (at most 20000)"));
    }
    let d = &s.doc()?.doc;
    let items: Vec<(NodeId, (Option<NodeId>, Rect))> =
        roots.iter().filter_map(|id| Some((*id, (d.parent_of(*id), effects::preview::bounds_of(d, &[*id], true)?)))).collect();
    let keyed: Vec<(Option<NodeId>, Rect)> = items.iter().map(|(_, k)| *k).collect();
    let sets: Vec<Vec<NodeId>> = overlapping(&keyed).into_iter().map(|set| set.iter().filter_map(|i| Some(items.get(*i)?.0)).collect()).collect();
    if sets.is_empty() {
        return Ok(json!({ "groups": [], "grouped": 0 }));
    }
    let grouped: std::collections::HashSet<NodeId> = sets.iter().flatten().copied().collect();
    let groups = s.edit("Group Overlapping Objects", |d, sel| {
        // Every group at once: the members behind each front one go, then each front one is
        // replaced by its group (one pass over the tree each, however many groups).
        let (mut places, mut behind, mut groups) = (vec![], vec![], vec![]);
        for set in &sets {
            let Some((&front, rest)) = set.split_last() else { continue };
            let children: Vec<Arc<Node>> = set.iter().filter_map(|id| d.node(*id).cloned().map(Arc::new)).collect();
            let gid = d.alloc_id();
            places.push((front, vec![Node::group(gid, children)]));
            behind.extend_from_slice(rest);
            groups.push(gid);
        }
        d.remove_many(&behind)?;
        d.insert_beside(places, true)?;
        sel.set(groups.iter().copied().chain(roots.iter().copied().filter(|id| !grouped.contains(id))));
        Ok(groups)
    })?;
    Ok(json!({ "groups": groups.iter().map(|i| i.0).collect::<Vec<_>>(), "grouped": grouped.len() }))
}
