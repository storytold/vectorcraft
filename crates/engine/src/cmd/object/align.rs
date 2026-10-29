use super::transform::transform_patterns;
use super::{
    Affine, Document, EngineError, NodeId, OrientedBox, Point, Rect, Result, Scaling, Session, Value, Vec2, bad, check_numbers, check_switches,
    effects, has_multi, ok, selected_roots, str_param,
};

impl Session {
    /// The bounds of `ids` the Transform panel, the bounding box, Align and Distribute measure:
    /// with Use Preview Bounds on, visual bounds as drawn (strokes and geometry effects included,
    /// see [`effects::preview`]), else geometric bounds.
    pub fn transform_bounds(&self, ids: &[NodeId]) -> Option<Rect> {
        effects::preview::bounds_of(&self.doc().ok()?.doc, ids, self.prefs.use_preview_bounds)
    }
    /// [`Session::transform_bounds`] square to the objects' own angle (their rotated bounding
    /// box, see [`Document::oriented_bounds`]).
    pub fn transform_box(&self, ids: &[NodeId]) -> Option<OrientedBox> {
        effects::preview::oriented_bounds(&self.doc().ok()?.doc, ids, self.prefs.use_preview_bounds)
    }
}

/// Measure with preview (visual) bounds? The `bounds` param, else Use Preview Bounds.
fn preview_bounds(s: &Session, p: &Value, cmd: &str) -> Result<bool> {
    match choice(p, cmd, "bounds", &["preview", "geometric"])? {
        None => Ok(s.prefs.use_preview_bounds),
        Some(b) => Ok(b == "preview"),
    }
}

/// Param `key` of the Align commands, Shear, Unlock All, Show All and `text.setFormat`'s lists: one
/// of `values`, or `None` when it is missing or null. Any other value is an error that lists `values`.
pub(crate) fn choice<'a>(p: &'a Value, cmd: &str, key: &str, values: &[&str]) -> Result<Option<&'a str>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    match v.as_str() {
        Some(s) if values.contains(&s) => Ok(Some(s)),
        _ => Err(bad(cmd, format!("`{key}` must be one of {}", values.join(", ")))),
    }
}

/// `horizontal` and `vertical` of `object.align` and `object.distribute`, each checked by [`choice`].
fn direction<'a>(p: &'a Value, cmd: &str) -> Result<(Option<&'a str>, Option<&'a str>)> {
    Ok((choice(p, cmd, "horizontal", &["left", "center", "right"])?, choice(p, cmd, "vertical", &["top", "center", "bottom"])?))
}

/// `spacing` of `object.distribute` and `object.distributeSpacing`: a finite number, or `None`
/// when it is missing or null. Any other value is an error.
fn spacing(p: &Value, cmd: &str) -> Result<Option<f64>> {
    p.get("spacing")
        .filter(|v| !v.is_null())
        .map(|v| v.as_f64().filter(|g| g.is_finite()).ok_or_else(|| bad(cmd, "`spacing` must be a number")))
        .transpose()
}

/// `ids` with their bounds (`preview`: visual bounds as drawn, see [`effects::preview::bounds_of`]).
fn items_bounds(d: &Document, ids: &[NodeId], preview: bool) -> Vec<(NodeId, Rect)> {
    ids.iter().filter_map(|id| Some((*id, effects::preview::bounds_of(d, &[*id], preview)?))).collect()
}

/// Move each object of `moves` by its offset and each ruler guide of `guides` (index, position)
/// to its position, as undo step `label`; pattern tiles move too when [`transform_patterns`] says
/// so for `ids`. When every move is a rounding error (1e-9 pt or less on both axes), the document
/// and its undo history stay as they are.
fn apply_moves(s: &mut Session, label: &str, p: &Value, ids: &[NodeId], moves: &[(NodeId, Vec2)], guides: &[(usize, f64)]) -> Result<Value> {
    let sc = Scaling { patterns: transform_patterns(s, p, ids)?, ..Scaling::default() };
    let guides: Vec<(usize, f64)> = {
        let d = &s.doc()?.doc;
        guides.iter().copied().filter(|&(i, pos)| d.guides.get(i).is_some_and(|g| (g.pos - pos).abs() > 1e-9)).collect()
    };
    if moves.iter().all(|(_, dv)| dv.x.abs() <= 1e-9 && dv.y.abs() <= 1e-9) && guides.is_empty() {
        return ok();
    }
    s.edit(label, |d, _| {
        for (id, dv) in moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), sc);
            }
        }
        for &(i, pos) in &guides {
            if let Some(g) = d.guides.get_mut(i) {
                g.pos = pos;
            }
        }
        Ok(())
    })?;
    ok()
}

/// What `object.align` aligns to: `to`, else the key object (or key guide) when there is one,
/// else the selection.
fn align_to<'a>(s: &Session, p: &'a Value) -> Option<&'a str> {
    str_param(p, "to").or_else(|| s.doc().ok()?.selection.has_key().then_some("key"))
}

/// The key guide `object.align` lines the objects up on (aligning to the key with one set,
/// [`vectorcraft_doc::Selection::key_guide`]): its position, and whether it is vertical.
fn key_guide(s: &Session, p: &Value) -> Option<(f64, bool)> {
    let st = s.doc().ok()?;
    let g = st.doc.guides.get(st.selection.key_guide?)?;
    (align_to(s, p) == Some("key")).then_some((g.pos, g.vertical))
}

/// Where Align puts the objects, `r`, and the axes it moves them along (x, y): a key guide gives
/// the line across its axis only (a vertical guide fixes x, a horizontal one y), so the other
/// axis stays as it is.
fn align_reference(s: &Session, p: &Value, ids: &[NodeId], preview: bool, own: Option<Rect>) -> Result<(Rect, bool, bool)> {
    if let Some((pos, vertical)) = key_guide(s, p) {
        return Ok((Rect::new(pos, pos, pos, pos), vertical, !vertical));
    }
    let r = match own {
        Some(b) if align_to(s, p) != Some("artboard") => b,
        _ => reference_rect(s, p, ids, preview)?,
    };
    Ok((r, true, true))
}

/// Bounds of the selected ruler guides (each guide is a zero-thickness line), for Align to
/// Selection when no objects are selected.
fn selected_guides_bounds(st: &crate::DocState) -> Option<Rect> {
    st.selection.guides.iter().filter_map(|&i| st.doc.guides.get(i)).fold(None, |acc: Option<Rect>, g| {
        let r = if g.vertical { Rect::new(g.pos, 0.0, g.pos, 0.0) } else { Rect::new(0.0, g.pos, 0.0, g.pos) };
        Some(acc.map(|a| a.union(r)).unwrap_or(r))
    })
}

fn reference_rect(s: &Session, p: &Value, ids: &[NodeId], preview: bool) -> Result<Rect> {
    let st = s.doc()?;
    match align_to(s, p) {
        Some("artboard") => {
            let b = effects::preview::bounds_of(&st.doc, ids, preview)
                .or_else(|| ids.is_empty().then(|| selected_guides_bounds(st)).flatten())
                .unwrap_or_default();
            // Align to Artboard uses artboard `artboard` (the app passes the active one). Without
            // it, or with an index the document doesn't have, it uses the artboard under the center
            // of the selection, or the first one when no artboard is there.
            let named = p.get("artboard").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).and_then(|i| st.doc.artboards.get(i));
            let i = st.doc.artboard_at(b.center()).unwrap_or(0);
            Ok(named.or(st.doc.artboards.get(i)).map(|a| a.rect).unwrap_or(b))
        }
        Some("key") => {
            let k = st.selection.key.ok_or_else(|| EngineError::Other("no key object".into()))?;
            effects::preview::bounds_of(&st.doc, &[k], preview).ok_or(EngineError::NoNode(k))
        }
        _ => {
            if ids.is_empty() {
                selected_guides_bounds(st).ok_or_else(|| EngineError::Other("nothing to align".into()))
            } else {
                effects::preview::bounds_of(&st.doc, ids, preview).ok_or_else(|| EngineError::Other("nothing to align".into()))
            }
        }
    }
}

/// Returns the artboard for `to: "artboard"` of `object.distribute` and `object.distributeSpacing`
/// (command `cmd`), the same artboard as `object.align` uses ([`reference_rect`]). It returns
/// `None` for the other choices. A `to` outside the choices is an error. Both commands call it
/// before they space selected anchor points, and they space those points without the artboard.
fn distribute_to(s: &Session, p: &Value, ids: &[NodeId], preview: bool, cmd: &str) -> Result<Option<Rect>> {
    match choice(p, cmd, "to", &["selection", "artboard", "key"])? {
        Some("artboard") => reference_rect(s, p, ids, preview).map(Some),
        _ => Ok(None),
    }
}

/// Returns the selected root (one of `ids`) that is the key object or contains it, such as the
/// compound path or selected group the key is in. Align to the key object, and Distribute and
/// Distribute Spacing with a spacing, keep it in place.
fn key_root(s: &Session, ids: &[NodeId]) -> Option<NodeId> {
    let st = s.doc().ok()?;
    let ancestry = st.doc.ancestry(st.selection.key?)?;
    ids.iter().copied().find(|id| ancestry.contains(id))
}

pub(super) fn align(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let preview = preview_bounds(s, p, "object.align")?;
    let (h, v) = direction(p, "object.align")?;
    choice(p, "object.align", "to", &["selection", "artboard", "key"])?;
    check_switches(p, &["patterns"], "object.align")?;
    check_numbers(p, &["artboard"], "object.align")?;
    if h.is_none() && v.is_none() {
        return Err(bad("object.align", "give horizontal or vertical"));
    }
    // Anchor points selected with Direct Selection align themselves, as in Illustrator (#1180).
    if !s.doc()?.selection.anchors.is_empty() {
        return align_anchors(s, p, &ids, (h, v));
    }
    let (r, along_x, along_y) = align_reference(s, p, &ids, preview, None)?;
    let (h, v) = (h.filter(|_| along_x), v.filter(|_| along_y));
    let key = key_root(s, &ids);
    let moves: Vec<(NodeId, Vec2)> = {
        let d = &s.doc()?.doc;
        items_bounds(d, &ids, preview)
            .into_iter()
            .filter(|(id, _)| Some(*id) != key || align_to(s, p) != Some("key"))
            .map(|(id, b)| {
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
                (id, Vec2::new(dx, dy))
            })
            .collect()
    };
    let guides = guide_moves(s, p, r, h, v)?;
    apply_moves(s, "Align", p, &ids, &moves, &guides)
}

/// The selected ruler guides that Align moves, and where: a vertical guide goes to the left edge,
/// centre or right edge of `r` that `h` names, a horizontal one to the top, centre or bottom that
/// `v` names. Align to the key object (or key guide) moves them too, and the key guide stays put.
/// Align to the artboard moves them (#3463). Align to the selection moves them when only guides are
/// selected; with objects in the selection they stay put.
fn guide_moves(s: &Session, p: &Value, r: Rect, h: Option<&str>, v: Option<&str>) -> Result<Vec<(usize, f64)>> {
    let st = s.doc()?;
    let to = align_to(s, p);
    let move_guides = match to {
        Some("key" | "artboard") => true,
        Some("selection") => st.selection.is_empty(),
        _ => false,
    };
    if !move_guides {
        return Ok(vec![]);
    }
    let at = |vertical: bool| match (vertical, h, v) {
        (true, Some("left"), _) => Some(r.x0),
        (true, Some("center"), _) => Some(r.center().x),
        (true, Some("right"), _) => Some(r.x1),
        (false, _, Some("top")) => Some(r.y0),
        (false, _, Some("center")) => Some(r.center().y),
        (false, _, Some("bottom")) => Some(r.y1),
        _ => None,
    };
    let skip_key = to == Some("key");
    Ok(st
        .selection
        .guides
        .iter()
        .filter(|&&i| !skip_key || Some(i) != st.selection.key_guide)
        .filter_map(|&i| Some((i, at(st.doc.guides.get(i)?.vertical)?)))
        .collect())
}

/// [`align`] of the selected anchor points (whole paths among them give all theirs): each point
/// moves to the edge or centre of the points' bounds, or of the artboard with Align To Artboard.
fn align_anchors(s: &mut Session, p: &Value, ids: &[NodeId], (h, v): (Option<&str>, Option<&str>)) -> Result<Value> {
    let targets = crate::cmd::path::anchor_targets(s)?;
    let d = &s.doc()?.doc;
    let pts: Vec<Point> = targets
        .iter()
        .flat_map(|(id, refs)| refs.iter().filter_map(move |(si, ai)| d.node(*id)?.path_data()?.subpaths.get(*si)?.anchors.get(*ai).map(|a| a.p)))
        .collect();
    let Some(bounds) = pts.iter().skip(1).fold(pts.first().map(|q| Rect::from_points(*q, *q)), |b, q| b.map(|b| b.union_pt(*q))) else {
        return ok();
    };
    // shortcut: no key anchor yet, so Align To Key Object aligns to the points' bounds as Selection does
    // (or to a key guide).
    let (r, along_x, along_y) = align_reference(s, p, ids, false, Some(bounds))?;
    let (h, v) = (h.filter(|_| along_x), v.filter(|_| along_y));
    let target = |q: Point| {
        let x = match h {
            Some("left") => r.x0,
            Some("center") => r.center().x,
            Some("right") => r.x1,
            _ => q.x,
        };
        let y = match v {
            Some("top") => r.y0,
            Some("center") => r.center().y,
            Some("bottom") => r.y1,
            _ => q.y,
        };
        Point::new(x, y)
    };
    if pts.iter().all(|q| {
        let dv = target(*q) - *q;
        dv.x.abs() <= 1e-9 && dv.y.abs() <= 1e-9
    }) {
        return ok();
    }
    s.edit("Align", |d, _| {
        for (id, refs) in &targets {
            let path = crate::cmd::path::path_mut(d, *id)?;
            for &(si, ai) in refs {
                if let Some(a) = path.anchor_mut(si, ai) {
                    a.translate(target(a.p) - a.p);
                }
            }
        }
        Ok(())
    })?;
    ok()
}

/// Distribute needs two or more objects and ruler guides, or anchor points selected with Direct
/// Selection (they may all be on one path).
pub(super) fn can_distribute(s: &Session) -> std::result::Result<(), String> {
    if s.active().is_some_and(|st| !st.selection.anchors.is_empty() || st.selection.len() + st.selection.guides.len() >= 2) {
        Ok(())
    } else {
        has_multi(s)
    }
}

/// What Distribute and Distribute Spacing space: a selected object, or a selected ruler guide
/// across the axis (#1597), which has no size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spaced {
    Object(NodeId),
    Guide(usize),
}

/// The selected objects `ids` with their bounds (`preview`: visual bounds), and the selected
/// ruler guides across the axis (vertical ones when `horiz`) with their line as their bounds.
/// Guides along the axis are left out, so they stay where they are.
fn spaced_items(s: &Session, ids: &[NodeId], preview: bool, horiz: bool) -> Result<Vec<(Spaced, Rect)>> {
    let st = s.doc()?;
    let objects = items_bounds(&st.doc, ids, preview).into_iter().map(|(id, r)| (Spaced::Object(id), r));
    let guides = st.selection.guides.iter().filter_map(|&i| {
        let g = st.doc.guides.get(i).filter(|g| g.vertical == horiz)?;
        Some((Spaced::Guide(i), Rect::new(g.pos, g.pos, g.pos, g.pos)))
    });
    Ok(objects.chain(guides).collect())
}

/// The index in `items` of what Distribute and Distribute Spacing keep in place with a spacing:
/// the selected root that is the key object or contains it ([`key_root`]), or the key guide when
/// aligning to the key (#2143).
fn spaced_key(s: &Session, p: &Value, ids: &[NodeId], items: &[(Spaced, Rect)]) -> Option<usize> {
    let key = match key_root(s, ids) {
        Some(id) => Spaced::Object(id),
        None => key_guide(s, p).and(s.doc().ok()?.selection.key_guide).map(Spaced::Guide)?,
    };
    items.iter().position(|(it, _)| *it == key)
}

/// Whether the key is a guide along the axis (a horizontal guide for a horizontal distribution),
/// aligning to the key. It has no line there, so a spacing from it leaves everything where it
/// is, as Align does.
fn key_guide_along(s: &Session, p: &Value, horiz: bool) -> bool {
    key_guide(s, p).is_some_and(|(_, vertical)| vertical != horiz)
}

/// Moves each of `deltas` that far along the axis (x when `horiz`) as undo step `label`, as
/// [`apply_moves`] does: an object by that offset, a ruler guide to its new position.
fn apply_spaced(s: &mut Session, label: &str, p: &Value, ids: &[NodeId], horiz: bool, deltas: &[(Spaced, f64)]) -> Result<Value> {
    let along = |d: f64| if horiz { Vec2::new(d, 0.0) } else { Vec2::new(0.0, d) };
    let (mut moves, mut guides) = (vec![], vec![]);
    let d = &s.doc()?.doc;
    for &(it, delta) in deltas {
        match it {
            Spaced::Object(id) => moves.push((id, along(delta))),
            Spaced::Guide(i) => guides.extend(d.guides.get(i).map(|g| (i, g.pos + delta))),
        }
    }
    apply_moves(s, label, p, ids, &moves, &guides)
}

/// [`distribute`] and [`distribute_spacing`] of the selected anchor points (whole paths among them
/// give all theirs): the points between the first and the last along the axis are spaced evenly,
/// or `gap` apart from the first. A point has no size, so every Distribute option of an axis
/// gives the same result.
fn distribute_anchors(s: &mut Session, label: &str, horiz: bool, gap: Option<f64>) -> Result<Value> {
    let targets = crate::cmd::path::anchor_targets(s)?;
    let d = &s.doc()?.doc;
    let along = |q: Point| if horiz { q.x } else { q.y };
    let mut pts: Vec<(NodeId, usize, usize, f64)> = targets
        .iter()
        .flat_map(|(id, refs)| {
            refs.iter().filter_map(move |&(si, ai)| Some((*id, si, ai, along(d.node(*id)?.path_data()?.subpaths.get(si)?.anchors.get(ai)?.p))))
        })
        .collect();
    pts.sort_by(|a, b| a.3.total_cmp(&b.3));
    let (Some(first), Some(last)) = (pts.first().map(|q| q.3), pts.last().map(|q| q.3)) else {
        return ok();
    };
    // Two points are the ends already; with a gap the second is placed from the first.
    if pts.len() < if gap.is_some() { 2 } else { 3 } {
        return ok();
    }
    let step = gap.unwrap_or((last - first) / (pts.len() - 1) as f64);
    let moves: Vec<(NodeId, usize, usize, f64)> =
        pts.iter().enumerate().map(|(i, &(id, si, ai, at))| (id, si, ai, first + step * i as f64 - at)).collect();
    if moves.iter().all(|m| m.3.abs() <= 1e-9) {
        return ok();
    }
    s.edit(label, |d, _| {
        for &(id, si, ai, delta) in &moves {
            if let Some(a) = crate::cmd::path::path_mut(d, id)?.anchor_mut(si, ai) {
                a.translate(if horiz { Vec2::new(delta, 0.0) } else { Vec2::new(0.0, delta) });
            }
        }
        Ok(())
    })?;
    ok()
}

pub(super) fn distribute(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &["patterns"], "object.distribute")?;
    check_numbers(p, &["artboard"], "object.distribute")?;
    let ids = selected_roots(s)?;
    let preview = preview_bounds(s, p, "object.distribute")?;
    let board = distribute_to(s, p, &ids, preview, "object.distribute")?;
    let spacing = spacing(p, "object.distribute")?;
    // Anchor points selected with Direct Selection are distributed themselves, as Align aligns them.
    if !s.doc()?.selection.anchors.is_empty() {
        return match direction(p, "object.distribute")? {
            (None, None) => Err(bad("object.distribute", "give horizontal or vertical")),
            (h, _) => distribute_anchors(s, "Distribute", h.is_some(), None),
        };
    }
    let (horiz, key): (bool, fn(&Rect, bool) -> f64) = match direction(p, "object.distribute")? {
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
    // The selected guides across the axis are spaced with the objects (#1597). A guide has no
    // size, so its left edge, center and right edge are all its line.
    let mut items = spaced_items(s, &ids, preview, horiz)?;
    items.sort_by(|a, b| key(&a.1, horiz).total_cmp(&key(&b.1, horiz)));
    let n = items.len();
    if spacing.is_some() && key_guide_along(s, p, horiz) {
        return ok();
    }
    // With a spacing, the key object (or the compound path or selected group it is in) or the key
    // guide stays where it is, and the lines are placed `spacing` apart, outward from its line, in
    // their order. Without a key the spacing is not used. `from_key` is the key's index in
    // `items` and the spacing.
    let from_key = spacing.zip(spaced_key(s, p, &ids, &items)).map(|(g, k)| (k, g));
    // Two items are enough on an artboard, where they go to its edges, and when they are spaced
    // from a key.
    if n < 2 || (n < 3 && board.is_none() && from_key.is_none()) {
        return ok();
    }
    let (first_box, last_box) = (&items[0].1, &items[n - 1].1);
    // `first` and `last` are the lines that the first and last items end on. Without an artboard
    // the first and last items stay where they are. On an artboard the first item moves until
    // its left (top) edge is on the artboard's left (top) edge, and the last item moves until its
    // right (bottom) edge is on the artboard's right (bottom) edge.
    let (first, last) = match board {
        Some(r) if horiz => (key(first_box, horiz) + r.x0 - first_box.x0, key(last_box, horiz) + r.x1 - last_box.x1),
        Some(r) => (key(first_box, horiz) + r.y0 - first_box.y0, key(last_box, horiz) + r.y1 - last_box.y1),
        None => (key(first_box, horiz), key(last_box, horiz)),
    };
    let deltas: Vec<(Spaced, f64)> = items
        .iter()
        .enumerate()
        .map(|(i, (it, r))| {
            let target = match from_key {
                Some((k, g)) => key(&items[k].1, horiz) + g * (i as f64 - k as f64),
                None => first + (last - first) * i as f64 / (n - 1) as f64,
            };
            (*it, target - key(r, horiz))
        })
        .collect();
    apply_spaced(s, "Distribute", p, &ids, horiz, &deltas)
}

pub(super) fn distribute_spacing(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &["patterns"], "object.distributeSpacing")?;
    check_numbers(p, &["artboard"], "object.distributeSpacing")?;
    let ids = selected_roots(s)?;
    let horiz = choice(p, "object.distributeSpacing", "axis", &["horizontal", "vertical"])? != Some("vertical");
    let preview = preview_bounds(s, p, "object.distributeSpacing")?;
    let board = distribute_to(s, p, &ids, preview, "object.distributeSpacing")?;
    let spacing = spacing(p, "object.distributeSpacing")?;
    if !s.doc()?.selection.anchors.is_empty() {
        return distribute_anchors(s, "Distribute Spacing", horiz, spacing);
    }
    // The selected guides across the axis are spaced with the objects, as items with no size (#1597).
    let mut items = spaced_items(s, &ids, preview, horiz)?;
    items.sort_by(|a, b| if horiz { a.1.x0.total_cmp(&b.1.x0) } else { a.1.y0.total_cmp(&b.1.y0) });
    if spacing.is_some() && key_guide_along(s, p, horiz) {
        return ok();
    }
    let n = items.len();
    let &[(_, first), .., (_, last)] = items.as_slice() else {
        // Selected guides along the axis have nothing to space.
        return if s.doc()?.selection.guides.is_empty() { Err(EngineError::Other("select two or more objects to distribute".into())) } else { ok() };
    };
    let size = |r: &Rect| if horiz { r.width() } else { r.height() };
    let start = |r: &Rect| if horiz { r.x0 } else { r.y0 };
    // The items start at `lo`, and without a spacing the last one ends at `hi`. These are the
    // first item's left (top) edge and the last one's right (bottom) edge, or the artboard's
    // edges with Align to Artboard and no spacing.
    let (lo, hi) = match board.filter(|_| spacing.is_none()) {
        Some(r) if horiz => (r.x0, r.x1),
        Some(r) => (r.y0, r.y1),
        None if horiz => (first.x0, last.x1),
        None => (first.y0, last.y1),
    };
    let gap = match spacing {
        Some(g) => g,
        None => (hi - lo - items.iter().map(|i| size(&i.1)).sum::<f64>()) / (n - 1) as f64,
    };
    let mut pos = lo;
    let mut deltas = vec![];
    for (it, r) in &items {
        deltas.push((*it, pos - start(r)));
        pos += size(r) + gap;
    }
    // With a spacing the key stays where it is: the key object, with the compound path or
    // selected group it is in, or the key guide (#2143). The others are spaced from it.
    let fixed = spacing.and_then(|_| spaced_key(s, p, &ids, &items)).and_then(|k| deltas.get(k)).map_or(0.0, |k| k.1);
    let deltas: Vec<(Spaced, f64)> = deltas.into_iter().map(|(it, d)| (it, d - fixed)).collect();
    apply_spaced(s, "Distribute Spacing", p, &ids, horiz, &deltas)
}
