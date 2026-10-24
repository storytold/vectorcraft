//! Shaper compositions are groups of generated fills/strokes with transparent, editable source
//! art retained underneath. Recipes select topology regions; source edits re-evaluate them in
//! the same undo transaction. Rendering and exporters only see ordinary groups and paths.
use super::pathops::Beside;
use super::*;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use vectorcraft_doc::shaper::{EdgeEdit, FILL_PREFIX, FaceEdit, FaceSelector, SOURCES, STROKE_PREFIX, ShaperSpec};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Point, Rect, Shape as _};
use vectorcraft_pathops as po;
use vectorcraft_tools::builder::{self as b, BuilderMap};

const MAX_SOURCES: usize = 64;
const MAX_POINTS: usize = 4096;
const MAX_EDITS: usize = 8192;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "shaper.scribble",
            "Shaper",
            [],
            None,
            "{points: [[x,y]…], ids?: [id…] (default: nearby art), tolerance?: pt (4)} interior scribbles remove fills; scribbles through several regions merge with the origin fill; ending outside removes fills and boundary strokes; outside scribbles erase stroke pieces. Originals remain editable → {id, regions, strokes}",
            has_doc,
            scribble
        ),
        cmd!(
            "shaper.select",
            "Select Shaper Face",
            [],
            None,
            "{point: [x,y], source?: bool} click selects a group, then a face; source enters construction isolation and selects an original; while a group's originals are isolated, it selects one of that group's originals → {id}",
            has_doc,
            select
        ),
        cmd!(
            "shaper.release",
            "Release Shaper Group",
            ["Object", "Shaper"],
            None,
            "{id? (default: selection)} restore the original paths → {ids}",
            has_doc,
            release
        ),
        cmd!(
            "shaper.expand",
            "Expand Shaper Group",
            ["Object", "Shaper"],
            None,
            "{id? (default: selection)} keep the visible result as ordinary paths → {id}",
            has_doc,
            expand
        ),
    ]
}

fn source_group(n: &Node) -> Option<&Node> {
    n.extras().shaper.as_ref()?;
    n.children()?.first().map(AsRef::as_ref).filter(|g| g.name.as_deref() == Some(SOURCES))
}
fn sources(n: &Node) -> Vec<Arc<Node>> {
    source_group(n).and_then(Node::children).cloned().unwrap_or_default()
}
fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 1e9 && p.y.abs() <= 1e9
}

fn map_of(src: &[Arc<Node>]) -> Result<BuilderMap> {
    if src.is_empty() || src.len() > MAX_SOURCES {
        return Err(bad("shaper.scribble", "Shaper needs 1–64 source paths"));
    }
    let mut count = 0usize;
    let mut shapes = vec![];
    for (i, n) in src.iter().enumerate() {
        let (mut path, rule) = b::node_outline(n).ok_or_else(|| bad("shaper.scribble", "use paths or compound paths"))?;
        count = count.saturating_add(path.subpaths.iter().map(|s| s.segment_count()).sum::<usize>());
        if count > 4096 || path.subpaths.iter().flat_map(|s| &s.anchors).any(|a| !finite(a.p) || !finite(a.h_in) || !finite(a.h_out)) {
            return Err(bad("shaper.scribble", "source paths are too large for Shaper"));
        }
        if !n.visible {
            path = PathData::default();
        } else if !n.appearance.fill_paint().is_none() {
            for sub in path.subpaths.iter_mut().filter(|s| !s.closed) {
                sub.closed = po::encloses_area(sub);
            }
        }
        shapes.push(po::Shape::new(path, rule, i as u64));
    }
    let arrangement = po::shape_builder(&shapes, true);
    let bounded = |pieces: Vec<po::Shape>| pieces.into_iter().filter_map(|p| p.path.bounds().map(|b| (p, b))).collect();
    // A Shaper Group's faces are what its shapes fill and what its open paths enclose: the gaps
    // closed paths enclose are the Shape Builder's only.
    let open = po::has_open_path(&shapes);
    let regions = arrangement
        .regions
        .into_iter()
        .filter(|r| open || !r.sources.is_empty())
        .filter_map(|r| {
            let bounds = r.path.bounds()?;
            let path = r.path.to_bezpath();
            Some((r, path, bounds))
        })
        .collect();
    let map = BuilderMap { shapes, regions, edges: bounded(arrangement.edges), lines: bounded(arrangement.lines) };
    if map.regions.is_empty() && map.lines.is_empty() && map.edges.is_empty() {
        return Err(bad("shaper.scribble", "these paths are too degenerate for Shaper"));
    }
    Ok(map)
}

fn relative(src: &[Arc<Node>], i: usize, p: Point) -> Point {
    let b = src.get(i).and_then(|n| n.geometric_bounds()).unwrap_or(Rect::ZERO);
    Point::new((p.x - b.x0) / b.width().max(1e-9), (p.y - b.y0) / b.height().max(1e-9))
}
fn absolute(src: &[Arc<Node>], i: usize, p: Point) -> Point {
    let b = src.get(i).and_then(|n| n.geometric_bounds()).unwrap_or(Rect::ZERO);
    Point::new(b.x0 + p.x * b.width(), b.y0 + p.y * b.height())
}
fn face_selector(map: &BuilderMap, src: &[Arc<Node>], i: usize) -> Option<FaceSelector> {
    let (r, _, bounds) = map.regions.get(i)?;
    let source = *r.sources.first()?;
    let point = po::interior_point(&r.path).unwrap_or(bounds.center());
    Some(FaceSelector { sources: r.sources.clone(), anchor: relative(src, source, point) })
}
fn selected_face(map: &BuilderMap, src: &[Arc<Node>], selector: &FaceSelector) -> Option<usize> {
    let point = absolute(src, *selector.sources.first()?, selector.anchor);
    map.regions
        .iter()
        .enumerate()
        .filter(|(_, (r, _, _))| r.sources == selector.sources)
        .min_by(|(_, a), (_, b)| {
            let distance = |r: &b::Face| if r.1.winding(point) != 0 { 0.0 } else { r.2.center().distance(point) };
            distance(a).total_cmp(&distance(b))
        })
        .map(|(i, _)| i)
}
fn piece_point(path: &PathData) -> Option<Point> {
    use vectorcraft_geom::ParamCurve as _;
    let sub = path.subpaths.first()?;
    (sub.segment_count() > 0).then(|| sub.segment(sub.segment_count() / 2).eval(0.5))
}
fn pieces(map: &BuilderMap) -> Vec<&po::Shape> {
    map.edges.iter().chain(&map.lines).map(|(p, _)| p).collect()
}
fn side_sources(map: &BuilderMap, p: &po::Shape) -> [Vec<usize>; 2] {
    let mut sides = map.sides(p).map(|i| i.and_then(|i| map.regions.get(i)).map_or(vec![], |r| r.0.sources.clone()));
    sides.sort();
    sides
}
fn edge_selector(map: &BuilderMap, src: &[Arc<Node>], piece: &po::Shape) -> Option<EdgeEdit> {
    let source = usize::try_from(piece.key).ok()?;
    Some(EdgeEdit { source, sides: side_sources(map, piece), anchor: relative(src, source, piece_point(&piece.path)?), erase: true, paint: None })
}
fn selected_edge(map: &BuilderMap, src: &[Arc<Node>], edit: &EdgeEdit) -> Option<usize> {
    let point = absolute(src, edit.source, edit.anchor);
    pieces(map)
        .iter()
        .enumerate()
        .filter(|(_, p)| p.key == edit.source as u64 && side_sources(map, p) == edit.sides)
        .filter_map(|(i, p)| p.path.nearest(point).map(|(_, _, _, _, distance)| (i, distance)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|p| p.0)
}
fn face_edits<'a>(map: &BuilderMap, src: &[Arc<Node>], spec: &'a ShaperSpec) -> BTreeMap<usize, &'a FaceEdit> {
    spec.faces.iter().filter_map(|e| selected_face(map, src, &e.selector).map(|i| (i, e))).collect()
}
fn fill_source(src: &[Arc<Node>], r: &po::Region) -> Option<usize> {
    r.sources.iter().rev().copied().find(|i| src.get(*i).is_some_and(|s| s.visible && !s.appearance.fill_paint().is_none()))
}
/// Regions separated only by covered strokes are one visible face. A visible line
/// through a circle still separates two faces, even though their fills have the same source.
fn visible_components(map: &BuilderMap, src: &[Arc<Node>], edits: &BTreeMap<usize, &FaceEdit>) -> Vec<usize> {
    fn root(parents: &[usize], mut i: usize) -> usize {
        for _ in 0..parents.len() {
            let Some(&next) = parents.get(i) else { return i };
            if next == i {
                return i;
            }
            i = next;
        }
        i
    }
    let owner = |i: usize| {
        let edit = edits.get(&i);
        if edit.is_some_and(|e| e.erase || e.paint.as_ref().is_some_and(|a| a.fill_paint().is_none())) {
            return None;
        }
        edit.and_then(|e| e.paint_source).or_else(|| map.regions.get(i).and_then(|r| fill_source(src, &r.0)))
    };
    let mut parents: Vec<usize> = (0..map.regions.len()).collect();
    for piece in pieces(map) {
        let [Some(a), Some(b)] = map.sides(piece) else { continue };
        let (Some(left), Some(right)) = (owner(a), owner(b)) else { continue };
        let merge = |i| edits.get(&i).and_then(|e| e.merge);
        let merged = merge(a).is_some() && merge(a) == merge(b);
        let covered = left as u64 > piece.key && right as u64 > piece.key;
        let unstroked = src.get(piece.key as usize).is_none_or(|n| !n.visible || n.appearance.stroke_paint().is_none());
        if (left == right || merged) && (merged || covered || unstroked) {
            let (a, b) = (root(&parents, a), root(&parents, b));
            if let Some(parent) = parents.get_mut(a) {
                *parent = b;
            }
        }
    }
    (0..parents.len()).map(|i| root(&parents, i)).collect()
}

fn fill_style(mut a: Appearance) -> Appearance {
    a.items.retain(AppearanceItem::is_fill);
    a
}
fn stroke_style(mut a: Appearance) -> Appearance {
    a.items.retain(|i| !i.is_fill());
    a
}

/// Join compatible strokes at two-way junctions. Branches stay separate; a line
/// crossing a circle must not turn into the circle's outline at the intersection.
fn join_strokes(nodes: Vec<Arc<Node>>) -> Vec<Arc<Node>> {
    const TOL: f64 = 1e-5;
    type End = (usize, bool);
    let mut buckets: BTreeMap<(i64, i64), Vec<(End, Point)>> = BTreeMap::new();
    let key = |p: Point| finite(p).then(|| ((p.x / TOL).floor() as i64, (p.y / TOL).floor() as i64));
    for (i, n) in nodes.iter().enumerate() {
        let Some(sp) = n.path_data().filter(|p| p.subpaths.len() == 1).and_then(|p| p.subpaths.first()).filter(|p| !p.closed) else { continue };
        for (last, a) in [(false, sp.anchors.first()), (true, sp.anchors.last())] {
            if let Some(a) = a
                && let Some(key) = key(a.p)
            {
                buckets.entry(key).or_default().push(((i, last), a.p));
            }
        }
    }
    let mut connections: BTreeMap<End, End> = BTreeMap::new();
    for entries in buckets.values() {
        for &(end, p) in entries {
            let Some((x, y)) = key(p) else { continue };
            let mut near = vec![];
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(entries) = buckets.get(&(x + dx, y + dy)) {
                        near.extend(entries.iter().filter(|(_, q)| p.distance(*q) <= TOL).map(|(end, _)| *end));
                    }
                }
            }
            if near.len() != 2 {
                continue;
            }
            let Some(other) = near.into_iter().find(|e| *e != end) else { continue };
            let Some((a, b)) = nodes.get(end.0).zip(nodes.get(other.0)) else { continue };
            if a.appearance == b.appearance && a.opacity == b.opacity && a.blend == b.blend {
                connections.insert(end, other);
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut out = vec![];
    // Open chains first, then closed loops. This avoids beginning in a chain's middle.
    let starts = nodes
        .iter()
        .enumerate()
        .flat_map(|(i, _)| [(i, false), (i, true)])
        .filter(|e| !connections.contains_key(e))
        .chain(nodes.iter().enumerate().map(|(i, _)| (i, false)));
    for start in starts {
        if seen.contains(&start.0) {
            continue;
        }
        if let Some(node) = nodes.get(start.0).filter(|n| n.path_data().is_none_or(|p| p.subpaths.len() != 1)) {
            seen.insert(start.0);
            out.push(node.clone());
            continue;
        }
        let mut end = start;
        let mut prototype = start.0;
        let mut path = vectorcraft_geom::SubPath::default();
        loop {
            if !seen.insert(end.0) {
                break;
            }
            prototype = prototype.min(end.0);
            let Some(mut next) = nodes.get(end.0).and_then(|n| n.path_data()).and_then(|p| p.subpaths.first()).cloned() else { break };
            if end.1 {
                next.reverse();
            }
            if path.anchors.is_empty() {
                path = next;
            } else {
                if let Some((a, b)) = path.anchors.last_mut().zip(next.anchors.first()) {
                    *a = vectorcraft_geom::Anchor::with_handles(a.p, a.h_in, b.h_out);
                }
                path.anchors.extend(next.anchors.into_iter().skip(1));
            }
            let Some(&next) = connections.get(&(end.0, !end.1)) else { break };
            if next == start {
                if let Some(last) = path.anchors.pop()
                    && let Some(first) = path.anchors.first_mut()
                {
                    *first = vectorcraft_geom::Anchor::with_handles(first.p, last.h_in, first.h_out);
                }
                path.closed = true;
                break;
            }
            end = next;
        }
        if let Some(mut node) = nodes.get(prototype).cloned() {
            if let Some(p) = Arc::make_mut(&mut node).path_data_mut() {
                *p = PathData::new(vec![path]);
            }
            out.push(node);
        }
    }
    out
}

fn generated(d: &mut Document, src: &[Arc<Node>], spec: &ShaperSpec, map: &BuilderMap) -> Result<Vec<Arc<Node>>> {
    if spec.faces.len() + spec.edges.len() > MAX_EDITS {
        return Err(bad("shaper.scribble", "too many Shaper edits"));
    }
    let edits = face_edits(map, src, spec);
    let mut fills: BTreeMap<(Option<u64>, usize), Vec<usize>> = BTreeMap::new();
    for (i, (r, _, _)) in map.regions.iter().enumerate() {
        let edit = edits.get(&i);
        if edit.is_some_and(|e| e.erase) {
            continue;
        }
        if edit.and_then(|e| e.paint_source).or_else(|| fill_source(src, r)).is_none() {
            continue;
        }
        let merge = edit.and_then(|e| e.merge);
        fills.entry((merge, if merge.is_some() { 0 } else { i })).or_default().push(i);
    }
    let mut out = vec![];
    for indices in fills.values() {
        let Some(&i) = indices.first() else { continue };
        let Some((region, _, _)) = map.regions.get(i) else { continue };
        let edit = edits.get(&i);
        let Some(source) = edit.and_then(|e| e.paint_source).or_else(|| fill_source(src, region)).and_then(|i| src.get(i)) else { continue };
        let path = po::unite_all(&indices.iter().filter_map(|i| map.regions.get(*i).map(|r| (&r.0.path, FillRule::NonZero))).collect::<Vec<_>>());
        let style = edit.and_then(|e| e.paint.clone()).unwrap_or_else(|| (*source.appearance).clone());
        let mut n = Node::path(d.alloc_id(), path, fill_style(style));
        n.opacity = source.opacity;
        n.blend = source.blend;
        n.name = Some(format!("{FILL_PREFIX}{i}"));
        out.push(Arc::new(n));
    }
    let edge_edits: BTreeMap<usize, &EdgeEdit> = spec.edges.iter().filter_map(|e| selected_edge(map, src, e).map(|i| (i, e))).collect();
    let mut strokes = vec![];
    for (i, piece) in pieces(map).into_iter().enumerate() {
        let sides = map.sides(piece);
        let merge = |r: Option<usize>| r.and_then(|r| edits.get(&r)).and_then(|e| e.merge);
        let merged_inside = merge(sides[0]).is_some() && merge(sides[0]) == merge(sides[1]);
        // An exiting scribble removes the exposed stroke around the deleted area.
        // Its shared boundary with surviving art still closes that art's contour.
        let erased_boundary =
            sides.iter().any(Option::is_some) && sides.iter().all(|r| r.is_none_or(|r| edits.get(&r).is_some_and(|e| e.erase && e.erase_edges)));
        let occluded = sides
            .iter()
            .all(|r| r.and_then(|r| map.regions.get(r)).and_then(|r| fill_source(src, &r.0)).is_some_and(|source| source as u64 > piece.key));
        if merged_inside || erased_boundary || occluded || edge_edits.get(&i).is_some_and(|e| e.erase) {
            continue;
        }
        let Some(source) = src.get(piece.key as usize) else { continue };
        let style = edge_edits.get(&i).and_then(|e| e.paint.clone()).unwrap_or_else(|| (*source.appearance).clone());
        if style.stroke_paint().is_none() {
            continue;
        }
        let mut n = Node::path(d.alloc_id(), piece.path.clone(), stroke_style(style));
        n.opacity = source.opacity;
        n.blend = source.blend;
        n.name = Some(format!("{STROKE_PREFIX}{i}"));
        strokes.push(Arc::new(n));
    }
    out.extend(join_strokes(strokes));
    Ok(out)
}

/// Re-evaluate only groups whose original art changed. Errors reach Session::edit's rollback.
pub(crate) fn refresh(before: &Document, d: &mut Document) -> Result<()> {
    let mut ids = vec![];
    d.walk(|n| {
        if n.extras().shaper.is_some() {
            ids.push(n.id)
        }
    });
    // Groups whose source art is all gone are removed through `emptied`, which is made when the
    // first one is found.
    let mut emptied: Option<Beside> = None;
    for &id in &ids {
        let Some(current) = d.node(id).cloned() else { continue };
        let Some(old) = before.node(id) else { continue };
        let src = sources(&current);
        let source_changed = sources(old) != src;
        let output_changed = current.children().is_some_and(|children| {
            children
                .iter()
                .skip(1)
                .any(|n| old.children().and_then(|c| c.iter().find(|o| o.id == n.id)).is_some_and(|o| o.appearance != n.appearance))
        });
        if !source_changed && !output_changed {
            continue;
        }
        if src.is_empty() {
            emptied.get_or_insert_with(|| Beside::replacing(d, &ids)).put(d, id, vec![])?;
            continue;
        }
        let map = map_of(&src)?;
        let mut spec = current.extras().shaper.as_deref().cloned().unwrap_or_default();
        let previous = sources(old);
        if previous.iter().map(|s| s.id).ne(src.iter().map(|s| s.id)) {
            // Source insertion, deletion or reordering updates the index-based recipes.
            let index = |i: usize| previous.get(i).and_then(|old| src.iter().position(|n| n.id == old.id));
            spec.faces.retain_mut(|e| {
                let Some(indices) = e.selector.sources.iter().map(|i| index(*i)).collect::<Option<Vec<_>>>() else { return false };
                let paint_source = match e.paint_source {
                    Some(i) => {
                        let Some(i) = index(i) else { return false };
                        Some(i)
                    }
                    None => None,
                };
                e.selector.sources = indices;
                e.selector.sources.sort();
                e.paint_source = paint_source;
                true
            });
            spec.edges.retain_mut(|e| {
                let Some(source) = index(e.source) else { return false };
                for side in &mut e.sides {
                    let Some(indices) = side.iter().map(|i| index(*i)).collect::<Option<Vec<_>>>() else { return false };
                    *side = indices;
                    side.sort();
                }
                e.sides.sort();
                e.source = source;
                true
            });
        }
        if output_changed {
            for n in current.children().into_iter().flatten().skip(1) {
                let Some(o) = old.children().and_then(|c| c.iter().find(|o| o.id == n.id)) else { continue };
                if o.appearance == n.appearance {
                    continue;
                }
                if let Some(i) = n.name.as_deref().and_then(|s| s.strip_prefix(FILL_PREFIX)).and_then(|s| s.parse::<usize>().ok()) {
                    let existing = face_edits(&map, &src, &spec);
                    let merge = existing.get(&i).and_then(|e| e.merge);
                    let components = visible_components(&map, &src, &existing);
                    let related: Vec<usize> = if let Some(m) = merge {
                        existing.iter().filter(|(_, e)| e.merge == Some(m)).map(|(i, _)| *i).collect()
                    } else {
                        components.iter().enumerate().filter(|(_, c)| Some(*c) == components.get(i)).map(|(i, _)| i).collect()
                    };
                    for i in related {
                        let Some(selector) = face_selector(&map, &src, i) else { continue };
                        if let Some(e) = spec.faces.iter_mut().rev().find(|e| e.selector == selector) {
                            e.paint = Some((*n.appearance).clone());
                        } else {
                            spec.faces.push(FaceEdit {
                                selector,
                                erase: false,
                                erase_edges: false,
                                paint_source: None,
                                merge,
                                paint: Some((*n.appearance).clone()),
                            });
                        }
                    }
                } else if n.name.as_deref().is_some_and(|s| s.starts_with(STROKE_PREFIX)) {
                    // One joined output may contain pieces from several original paths.
                    for piece in pieces(&map) {
                        let contained = piece
                            .path
                            .subpaths
                            .iter()
                            .flat_map(|s| &s.anchors)
                            .all(|a| n.path_data().and_then(|p| p.nearest(a.p)).is_some_and(|p| p.4 < 1e-5))
                            && piece_point(&piece.path).is_some_and(|point| n.path_data().and_then(|p| p.nearest(point)).is_some_and(|p| p.4 < 1e-5));
                        if contained && let Some(mut e) = edge_selector(&map, &src, piece) {
                            e.erase = false;
                            e.paint = Some((*n.appearance).clone());
                            spec.edges.push(e);
                        }
                    }
                }
            }
        }
        let mut children = current.children().and_then(|c| c.first()).cloned().into_iter().collect::<Vec<_>>();
        let old_ids: BTreeMap<&str, NodeId> =
            current.children().into_iter().flatten().skip(1).filter_map(|n| n.name.as_deref().map(|name| (name, n.id))).collect();
        for mut n in generated(d, &src, &spec, &map)? {
            if let Some(id) = n.name.as_deref().and_then(|name| old_ids.get(name)) {
                Arc::make_mut(&mut n).id = *id;
            }
            children.push(n);
        }
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        n.extras_mut().shaper = Some(Box::new(spec));
        if let Some(ch) = n.children_mut() {
            *ch = children;
        }
    }
    emptied.map_or(Ok(()), |e| e.finish(d))
}

fn candidates(s: &Session, p: &Value, points: &[Point], tolerance: f64) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let area = points.iter().fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |b, p| b.union_pt(*p)).inflate(tolerance, tolerance);
    let supplied = p.get("ids").map(|_| checked_ids_param(s, p, "ids", "shaper.scribble")).transpose()?;
    let expand = supplied.is_none() && st.selection.objects.len() < 2;
    let ids = supplied.unwrap_or_else(|| if st.selection.objects.len() > 1 { st.selection.objects.clone() } else { st.doc.selectable_art() });
    let editable = st.doc.editable();
    let eligible: Vec<NodeId> = ids
        .into_iter()
        .filter(|id| {
            editable(*id)
                && st.doc.node(*id).is_some_and(|n| {
                    !matches!(n.kind, NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. })
                        && (n.extras().shaper.is_some() || b::node_outline(n).is_some())
                })
        })
        .collect();
    let bounds = |id| st.doc.node(id).and_then(Node::geometric_bounds).map(|b| b.inflate(tolerance, tolerance));
    let overlaps = |a: Rect, b: Rect| a.intersect(b).area() > 0.0;
    let mut ids: Vec<NodeId> = eligible.iter().copied().filter(|id| bounds(*id).is_some_and(|b| overlaps(b, area))).collect();
    if expand {
        // A protruding line must still be split at the shape it passes through, even when
        // the scribble itself never enters that shape. Include the connected neighbourhood.
        loop {
            let next: Vec<NodeId> = eligible
                .iter()
                .copied()
                .filter(|id| {
                    !ids.contains(id)
                        && bounds(*id).is_some_and(|bb| {
                            ids.iter()
                                .any(|other| st.doc.parent_of(*other) == st.doc.parent_of(*id) && bounds(*other).is_some_and(|b| overlaps(bb, b)))
                        })
                })
                .collect();
            if next.is_empty() {
                break;
            }
            ids.extend(next);
            if ids.len() > MAX_SOURCES {
                return Err(bad("shaper.scribble", "too many overlapping paths for Shaper"));
            }
        }
    }
    let ids = b::sorted_roots(&st.doc, &ids);
    let parent = ids.first().and_then(|id| st.doc.position(*id)).map(|p| p.0);
    if ids.iter().any(|id| st.doc.position(*id).map(|p| p.0) != parent) {
        return Err(bad("shaper.scribble", "overlapping Shaper paths must be in the same layer or group"));
    }
    Ok(ids)
}

fn scribble(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shaper.scribble";
    let input = p
        .get("points")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= MAX_POINTS)
        .ok_or_else(|| bad(C, "points must contain 1–4096 coordinates"))?;
    let pts = input
        .iter()
        .map(|v| point_param(&json!({"p": v}), "p").filter(|p| finite(*p)).ok_or_else(|| bad(C, "invalid point")))
        .collect::<Result<Vec<_>>>()?;
    let tolerance = f64_or(p, "tolerance", 4.0);
    if !tolerance.is_finite() || !(0.01..=1000.0).contains(&tolerance) {
        return Err(bad(C, "invalid tolerance"));
    }
    let step = (tolerance / 2.0).max(0.5);
    let sample_count = pts.windows(2).map(|w| (w[0].distance(w[1]) / step).ceil()).sum::<f64>();
    if sample_count > 65_536.0 {
        return Err(bad(C, "scribble is too long"));
    }
    let points = b::sample_polyline(&pts, step);
    let roots = candidates(s, p, &points, tolerance)?;
    if roots.is_empty() {
        return Ok(json!({"id": null, "regions": 0, "strokes": 0}));
    }
    let mut src = vec![];
    let mut spec = ShaperSpec::default();
    let mut next_merge = 1u64;
    for id in &roots {
        let n = s.doc()?.doc.node(*id).ok_or(EngineError::NoNode(*id))?;
        if let Some(previous) = n.extras().shaper.as_deref() {
            let offset = src.len();
            let merge_offset = next_merge;
            for mut e in previous.faces.clone() {
                for i in &mut e.selector.sources {
                    *i = i.saturating_add(offset);
                }
                e.paint_source = e.paint_source.map(|i| i.saturating_add(offset));
                e.merge = e.merge.map(|i| i.saturating_add(merge_offset));
                next_merge = next_merge.max(e.merge.unwrap_or(0).saturating_add(1));
                spec.faces.push(e);
            }
            for mut e in previous.edges.clone() {
                e.source = e.source.saturating_add(offset);
                for side in &mut e.sides {
                    for i in side {
                        *i = i.saturating_add(offset);
                    }
                }
                spec.edges.push(e);
            }
            src.extend(sources(n));
        } else {
            src.push(Arc::new(n.clone()));
        }
    }
    let map = map_of(&src)?;
    let edits = face_edits(&map, &src, &spec);
    let filled = |i: usize| edits.get(&i).is_none_or(|e| !e.erase) && map.regions.get(i).and_then(|r| fill_source(&src, &r.0)).is_some();
    let components = visible_components(&map, &src, &edits);
    // Every face the scribble crosses, however narrow.
    let faces: std::collections::BTreeSet<usize> =
        map.hits_along(&pts, None).regions.into_iter().filter(|i| filled(*i)).filter_map(|i| components.get(i).copied()).collect();
    let touched: Vec<usize> = components.iter().enumerate().filter(|(_, c)| faces.contains(c)).map(|(i, _)| i).collect();
    let first = points.first().copied().unwrap_or(Point::ZERO);
    let last = points.last().copied().unwrap_or(first);
    let origin = map.region_at(first).filter(|i| filled(*i));
    let exits = map.region_at(last).is_none();
    let merge = origin.is_some() && faces.len() > 1 && !exits;
    let paint_source =
        origin.and_then(|i| edits.get(&i).and_then(|e| e.paint_source).or_else(|| map.regions.get(i).and_then(|r| fill_source(&src, &r.0))));
    let origin_paint = origin.and_then(|i| edits.get(&i).and_then(|e| e.paint.clone()));
    for &i in &touched {
        if let Some(selector) = face_selector(&map, &src, i) {
            spec.faces.push(FaceEdit {
                selector,
                erase: !merge,
                erase_edges: exits,
                paint_source,
                merge: merge.then_some(next_merge),
                paint: origin_paint.clone(),
            });
        }
    }
    let mut strokes = 0;
    if touched.is_empty() {
        // Every stroke the scribble crosses or passes within `tolerance` of, however small that is.
        let mut hits = map.hits_along(&pts, Some(tolerance));
        for point in &points {
            hits.add(map.hit(*point, Some(tolerance)));
        }
        let piece_ids = hits.edges.iter().copied().chain(hits.lines.iter().map(|i| map.edges.len() + i));
        for i in piece_ids {
            if let Some(e) = pieces(&map).get(i).and_then(|p| edge_selector(&map, &src, p)) {
                spec.edges.push(e);
                strokes += 1;
            }
        }
    }
    if touched.is_empty() && strokes == 0 {
        return Ok(json!({"id": null, "regions": 0, "strokes": 0}));
    }
    let id = s.edit("Shaper", |d, sel| {
        let top = *roots.last().ok_or_else(|| bad(C, "no source art"))?;
        let (parent, index, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let earlier = roots.iter().filter(|id| **id != top && d.position(**id).is_some_and(|p| p.1 < index)).count();
        let source_id = d.alloc_id();
        let mut source = Node::group(source_id, src.clone());
        source.name = Some(SOURCES.into());
        source.opacity = 0.0;
        let mut children = vec![Arc::new(source)];
        children.extend(generated(d, &src, &spec, &map)?);
        let id = d.alloc_id();
        let mut group = Node::group(id, children);
        group.name = Some("Shaper Group".into());
        group.extras_mut().shaper = Some(Box::new(spec));
        d.remove_many(&roots)?;
        d.insert(parent, index.saturating_sub(earlier), group)?;
        sel.set([id]);
        Ok(id)
    })?;
    s.doc_mut()?.isolation = None;
    Ok(json!({"id": id.0, "regions": touched.len(), "strokes": strokes}))
}

/// The topmost editable Shaper Group at `point`, measured by its source shapes in `construction`.
/// When `within` is given, only that group is considered.
fn group_at(d: &Document, point: Point, construction: bool, within: Option<NodeId>) -> Option<NodeId> {
    let mut groups = vec![];
    let editable = d.editable();
    d.walk(|n| {
        if n.extras().shaper.is_some()
            && within.is_none_or(|g| g == n.id)
            && editable(n.id)
            && (if construction { source_group(n).and_then(Node::geometric_bounds) } else { n.geometric_bounds() })
                .is_some_and(|b| b.inflate(6.0, 6.0).contains(point))
        {
            groups.push(n.id)
        }
    });
    groups.last().copied()
}
fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let point = point_param(p, "point").filter(|p| finite(*p)).ok_or_else(|| bad("shaper.select", "missing point"))?;
    let st = s.doc()?;
    let source = bool_or(p, "source", false);
    // In Construction mode a source pick stays in the group whose source shapes are isolated.
    let isolated =
        st.isolation.and_then(|i| st.doc.parent_of(i)).filter(|g| st.doc.node(*g).and_then(source_group).is_some_and(|s| st.isolation == Some(s.id)));
    let Some(group) = group_at(&st.doc, point, source, isolated.filter(|_| source)) else { return Ok(json!({"id": null})) };
    let node = st.doc.node(group).ok_or(EngineError::NoNode(group))?;
    let target = if source {
        sources(node)
            .iter()
            .rev()
            .find(|n| {
                b::node_outline(n)
                    .is_some_and(|(path, _)| path.to_bezpath().winding(point) != 0 || path.nearest(point).is_some_and(|(_, _, _, _, d)| d < 6.0))
            })
            .map(|n| n.id)
            .unwrap_or(group)
    } else if st.selection.contains(group) {
        node.children()
            .into_iter()
            .flatten()
            .skip(1)
            .find(|n| {
                n.name.as_deref().is_some_and(|n| n.starts_with(FILL_PREFIX)) && n.path_data().is_some_and(|p| p.to_bezpath().winding(point) != 0)
            })
            .map_or(group, |n| n.id)
    } else {
        group
    };
    let isolation = source.then(|| source_group(node).map(|n| n.id)).flatten();
    s.select(|_, sel| sel.set([target]))?;
    if let Some(id) = isolation {
        s.doc_mut()?.isolation = Some(id);
    }
    Ok(json!({"id": target.0}))
}
fn target(s: &Session, p: &Value) -> Result<NodeId> {
    let id = id_param(p, "id")
        .or_else(|| s.active().and_then(|d| d.selection.objects.first().copied()))
        .ok_or_else(|| bad("shaper.release", "select a Shaper group"))?;
    s.doc()?.doc.node(id).filter(|n| n.extras().shaper.is_some()).ok_or_else(|| bad("shaper.release", "select a Shaper group"))?;
    Ok(id)
}
fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p)?;
    let ids = s.edit("Release Shaper Group", |d, sel| {
        let original = d.node(id).map(sources).ok_or(EngineError::NoNode(id))?;
        let (parent, index, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        d.remove(id)?;
        let mut ids = vec![];
        for (i, n) in original.into_iter().enumerate() {
            ids.push(d.insert(parent, index + i, (*n).clone())?);
        }
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    s.doc_mut()?.isolation = None;
    Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}
fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p)?;
    s.edit("Expand Shaper Group", |d, sel| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        n.extras_mut().shaper = None;
        n.name = None;
        if let Some(children) = n.children_mut()
            && children.first().is_some_and(|c| c.name.as_deref() == Some(SOURCES))
        {
            children.remove(0);
        }
        sel.set([id]);
        Ok(())
    })?;
    s.doc_mut()?.isolation = None;
    Ok(json!({"id": id.0}))
}
