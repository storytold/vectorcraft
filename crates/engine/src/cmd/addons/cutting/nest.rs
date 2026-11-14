//! Nest Shapes (`addon.cutting.nest`), a command of the built-in Cutting Tools add-on that packs
//! the selected objects onto a sheet or a roll of material by their bounding boxes
//! (<https://en.wikipedia.org/wiki/Packing_problems>). The sheet is an
//! artboard, or a set size from the artboard's top-left corner; a roll is as wide as the sheet
//! and has no end. The objects are moved, and with `rotate` turned a quarter turn when that places
//! them better; they are never scaled or changed otherwise.

#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};
use vectorcraft_render::effects;

use super::super::super::edit::selected_roots_to_transform;
use super::super::super::object::{keep_compound_aligned, transform_node, transform_patterns};
use super::super::super::pathops::length_given;
use super::super::super::{CommandSpec, bad, check_switches, cmd, has_selection, switch_or};
use crate::{Result, Session};

/// Nest Shapes' command.
pub const NEST: &str = "addon.cutting.nest";
/// The largest sheet width or height, in points.
const MAX_SIZE: f64 = 1_000_000.0;
/// The largest margin or spacing, in points.
const MAX_GAP: f64 = 100_000.0;
/// The most objects one run nests.
const MAX_SHAPES: usize = 10_000;
/// How far a box may reach past the sheet's edge and still fit (rounding).
const EPS: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.cutting.nest",
        "Nest Shapes…",
        ["Object", "Plug-ins", "Cutting Tools"],
        None,
        "{roll?: bool (false: a sheet `width` × `height`; true: a roll `width` wide with no end), width?: pt 0..1000000 (0: the artboard's width), height?: pt 0..1000000 (0: the artboard's height; not used with roll), margin?: pt 0..100000 (18: kept clear inside each edge of the sheet; on a roll, the sides and the start), spacing?: pt 0..100000 (9: kept between two objects), rotate?: bool (false: objects keep their orientation; true: an object may turn 90° clockwise where that puts its bottom edge higher, or where only the turned object fits), usePreviewBounds?: bool (true: objects are measured with their strokes and effects, the reach of shadows, glows and blurs included; false: by their geometry), artboard?: index (0-based; the sheet's artboard; default: the artboard under the center of the selection, else the first), patterns?: bool (Transform Patterns: pattern fills and strokes move with the art; default: prefs transformPatternTiles)} move the selected objects onto the sheet or roll, which starts at the artboard's top-left corner, packed by their bounding boxes: the tallest first (with rotate, the longest side first), each where its bottom edge is highest and then furthest left, without resizing them. An object on the pixel grid moves by whole points and takes 1 pt more room across and down; a path of a pixel-aligned compound path is not turned. Objects that do not fit on the sheet go in a column to the right of it, `spacing` from its edge and from each other, from its top, in stacking order (back to front). Objects without bounds (an empty group) are not moved. One undo step; nothing changes when no object moves → {placed: count, unplaced: [ids of the objects put beside the sheet], rotated: [ids], skipped: [ids of the objects without bounds], length: pt (from the sheet's top edge to the bottom margin below the lowest object placed)}",
        has_selection,
        nest
    )]
}

/// The length `p` gives for `key`, from 0 to `max`, or `default`.
fn length(p: &Value, key: &str, default: f64, max: f64) -> Result<f64> {
    match length_given(p, key, NEST)? {
        None => Ok(default),
        Some(v) if (0.0..=max).contains(&v) => Ok(v),
        Some(_) => Err(bad(NEST, format!("`{key}` must be from 0 to {max} pt"))),
    }
}

/// One stretch of a skyline: from `x`, `w` wide, filled down to `y`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    x: f64,
    w: f64,
    y: f64,
}

/// Bottom-left packing of boxes on a skyline: a strip `width` wide and `height` deep
/// (`f64::INFINITY` for a roll), whose filled part is the spans' outline along its top.
struct Skyline {
    spans: Vec<Span>,
    width: f64,
    height: f64,
}

impl Skyline {
    fn new(width: f64, height: f64) -> Self {
        Skyline { spans: vec![Span { x: 0.0, w: width, y: 0.0 }], width, height }
    }

    /// Where a box `w` × `h` goes: at the left edge of a span, as high as the spans under it
    /// allow, choosing the place where its bottom edge is highest and then furthest left. `None`
    /// when it fits nowhere. The spans under the box at each span are a window whose two ends only
    /// move right, and `window` holds the indices in it whose depth no later span in it reaches, so
    /// its front is the deepest: one call takes time in proportion to the number of spans.
    fn find(&self, w: f64, h: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        let mut window: VecDeque<usize> = VecDeque::new();
        let mut end = 0;
        for (i, s) in self.spans.iter().enumerate() {
            if s.x + w > self.width + EPS {
                break;
            }
            while window.front().is_some_and(|&j| j < i) {
                window.pop_front();
            }
            end = end.max(i);
            while let Some(t) = self.spans.get(end)
                && (end == i || t.x < s.x + w - EPS)
            {
                while window.back().and_then(|&j| self.spans.get(j)).is_some_and(|b| b.y <= t.y) {
                    window.pop_back();
                }
                window.push_back(end);
                end += 1;
            }
            let y = window.front().and_then(|&j| self.spans.get(j)).map_or(0.0, |t| t.y.max(0.0));
            if y + h > self.height + EPS {
                continue;
            }
            // The box is `h` tall at every place, so the place with the highest bottom edge is the
            // place with the highest top edge.
            if best.is_none_or(|(bx, by)| y < by - EPS || ((y - by).abs() <= EPS && s.x < bx)) {
                best = Some((s.x, y));
            }
        }
        best
    }

    /// Fills the skyline down to `y + h` from `x` across `w`.
    fn place(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if w <= 0.0 {
            return;
        }
        let end = x + w;
        let mut out = Vec::with_capacity(self.spans.len() + 2);
        let mut added = false;
        for s in &self.spans {
            let s_end = s.x + s.w;
            if s_end <= x || s.x >= end {
                out.push(*s);
                continue;
            }
            if s.x < x {
                out.push(Span { x: s.x, w: x - s.x, y: s.y });
            }
            if !added {
                out.push(Span { x, w, y: y + h });
                added = true;
            }
            if s_end > end {
                out.push(Span { x: end, w: s_end - end, y: s.y });
            }
        }
        // Neighbors filled to the same depth are one span.
        let mut merged: Vec<Span> = Vec::with_capacity(out.len());
        for s in out {
            match merged.last_mut() {
                Some(l) if l.y == s.y => l.w = s.x + s.w - l.x,
                _ => merged.push(s),
            }
        }
        self.spans = merged;
    }
}

/// One box to pack: its width and height as it stands, and when it may turn, as it is turned.
type Sizes = ((f64, f64), Option<(f64, f64)>);

/// The side a box is ordered by: its height, or its longest side when it may turn.
fn lead(b: &Sizes) -> (f64, f64) {
    let ((w, h), turned) = *b;
    if turned.is_some() { (w.max(h), w.min(h)) } else { (h, w) }
}

/// Packs `boxes` onto a strip `width` wide and `height` deep, the tallest first (a box that may
/// turn by its longest side; then the widest, then in their order), each where [`Skyline::find`]
/// puts it, turned when that puts its bottom edge higher (then further left), or when only the
/// turned box fits. Gives each box's top-left corner and whether it is turned, or `None` for a box
/// that does not fit.
fn pack(boxes: &[Sizes], width: f64, height: f64) -> Vec<Option<(f64, f64, bool)>> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| {
        let (la, lb) = (lead(&boxes[a]), lead(&boxes[b]));
        lb.0.total_cmp(&la.0).then(lb.1.total_cmp(&la.1))
    });
    let mut sky = Skyline::new(width, height);
    let mut at = vec![None; boxes.len()];
    for i in order {
        let Some(&((w, h), turned)) = boxes.get(i) else { continue };
        let mut best = sky.find(w, h).map(|(x, y)| (x, y, false, w, h));
        if let Some((tw, th)) = turned
            && let Some((x, y)) = sky.find(tw, th)
        {
            let better = best.is_none_or(|(bx, by, _, _, bh)| {
                let (was, now) = (by + bh, y + th);
                now < was - EPS || ((now - was).abs() <= EPS && x < bx - EPS)
            });
            if better {
                best = Some((x, y, true, tw, th));
            }
        }
        if let Some((x, y, t, bw, bh)) = best {
            sky.place(x, y, bw, bh);
            if let Some(slot) = at.get_mut(i) {
                *slot = Some((x, y, t));
            }
        }
    }
    at
}

/// A quarter turn clockwise on the page about `c`.
fn quarter_turn(c: Point) -> Affine {
    Affine::translate(c.to_vec2()) * vectorcraft_geom::rotate_deg(90.0) * Affine::translate(-c.to_vec2())
}

fn finite(r: &Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())
}

/// The box `id` is packed by: with `preview`, its visual bounds with geometry effects
/// ([`effects::preview::bounds_of`]) and the reach of its raster effects, such as shadows, glows
/// and blurs ([`vectorcraft_render::drawn_bounds`]); else its geometric bounds.
fn measure(d: &vectorcraft_doc::Document, id: NodeId, preview: bool) -> Option<Rect> {
    let b = effects::preview::bounds_of(d, &[id], preview).filter(finite)?;
    if !preview {
        return Some(b);
    }
    match d.node(id).and_then(|n| vectorcraft_render::drawn_bounds(d, n)).filter(finite) {
        Some(r) => Some(b.union(r)),
        None => Some(b),
    }
}

/// Whether `id` is a path of a pixel-aligned compound path.
fn in_aligned_compound(d: &Document, id: NodeId) -> bool {
    d.parent_of(id).and_then(|p| d.node(p)).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }) && n.extras().pixel_aligned)
}

/// Whether moving `id` snaps it, or an object in it, to the pixel grid: it, an object it holds or
/// the art of a linked opacity mask in it is pixel-aligned, or it is a path of a pixel-aligned
/// compound path.
fn snaps(d: &Document, id: NodeId) -> bool {
    if in_aligned_compound(d, id) {
        return true;
    }
    let mut stack: Vec<&Node> = d.node(id).into_iter().collect();
    while let Some(n) = stack.pop() {
        if n.extras().pixel_aligned {
            return true;
        }
        stack.extend(n.children().map_or(&[][..], Vec::as_slice).iter().map(|c| &**c));
        if let Some(m) = n.extras().mask.as_ref()
            && m.linked
        {
            stack.push(&*m.art);
        }
    }
    false
}

fn nest(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &["patterns"], NEST)?;
    let roll = switch_or(p, "roll", false, NEST)?;
    let width = length(p, "width", 0.0, MAX_SIZE)?;
    let height = length(p, "height", 0.0, MAX_SIZE)?;
    let margin = length(p, "margin", 18.0, MAX_GAP)?;
    let spacing = length(p, "spacing", 9.0, MAX_GAP)?;
    let turn = switch_or(p, "rotate", false, NEST)?;
    let preview = switch_or(p, "usePreviewBounds", true, NEST)?;
    let named = match p.get("artboard") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(NEST, "`artboard` must be an artboard's index"))?),
    };
    // A path of a compound path selected on its own moves without the rest of the compound path.
    let ids = selected_roots_to_transform(s)?;
    if ids.len() > MAX_SHAPES {
        return Err(bad(NEST, format!("select at most {MAX_SHAPES} objects to nest")));
    }
    // Objects without a finite box (an empty group, empty type) are left where they are and
    // listed as `skipped`.
    let mut skipped = vec![];
    let (items, board): (Vec<(NodeId, Rect)>, Rect) = {
        let d = &s.doc()?.doc;
        let mut items: Vec<(NodeId, Rect)> = Vec::with_capacity(ids.len());
        for id in &ids {
            match measure(d, *id, preview) {
                Some(r) => items.push((*id, r)),
                None => skipped.push(id.0),
            }
        }
        let board = match named {
            Some(i) => d.artboards.get(i).ok_or_else(|| bad(NEST, format!("there is no artboard {i}")))?.rect,
            None => {
                let all = items.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b));
                let i = all.and_then(|r| d.artboard_at(r.center())).unwrap_or(0);
                d.artboards.get(i).ok_or_else(|| bad(NEST, "there is no artboard for the sheet"))?.rect
            }
        };
        (items, board)
    };
    if items.is_empty() {
        return Err(bad(NEST, "the selection has nothing to nest"));
    }
    let sheet_w = if width > 0.0 { width } else { board.width() };
    let sheet_h = if roll {
        f64::INFINITY
    } else if height > 0.0 {
        height
    } else {
        board.height()
    };
    // Every box is `spacing` wider and taller, and the strip is `spacing` wider and deeper than
    // the room inside the margins, so the spacing falls between boxes and not at the far edges.
    let room_w = sheet_w - 2.0 * margin;
    let room_h = if roll { f64::INFINITY } else { sheet_h - 2.0 * margin };
    if room_w <= 0.0 || room_h <= 0.0 {
        return Err(bad(NEST, "the margin leaves no room on the sheet"));
    }
    let all: Vec<NodeId> = items.iter().map(|(id, _)| *id).collect();
    let grid: Vec<bool> = {
        let d = &s.doc()?.doc;
        all.iter().map(|id| snaps(d, *id)).collect()
    };
    let sc = Scaling { patterns: transform_patterns(s, p, &all)?, ..Scaling::default() };
    // With `rotate`, each object is turned in a copy of the document and measured there, so the
    // box packed for a turned object is the box its turned strokes and effects take. A path of a
    // pixel-aligned compound path keeps its orientation: turning it snaps the whole compound path
    // again, which would move the compound's other paths away from where they were measured.
    let turned: Vec<Option<(Affine, Rect)>> = if turn {
        let mut t = vectorcraft_doc::Document::clone(&s.doc()?.doc);
        let xfs: Vec<Option<Affine>> = items.iter().map(|(id, r)| (!in_aligned_compound(&t, *id)).then(|| quarter_turn(r.center()))).collect();
        for ((id, _), xf) in items.iter().zip(&xfs) {
            if let (Some(xf), Some(n)) = (xf, t.node_mut(*id)) {
                transform_node(n, *xf, sc, true);
            }
        }
        items.iter().zip(xfs).map(|((id, _), xf)| xf.and_then(|xf| measure(&t, *id, preview).map(|r| (xf, r)))).collect()
    } else {
        vec![None; items.len()]
    };
    // An object that snaps to the pixel grid moves by whole points, so it stays on the grid and the
    // snap after the move does not shift it; its box is 1 pt wider and taller to hold the rounding.
    let slack = |g: bool| if g { 1.0 } else { 0.0 };
    let boxes: Vec<Sizes> = items
        .iter()
        .zip(&turned)
        .zip(&grid)
        .map(|(((_, r), t), g)| {
            let pad = spacing + slack(*g);
            ((r.width() + pad, r.height() + pad), t.map(|(_, q)| (q.width() + pad, q.height() + pad)))
        })
        .collect();
    let at = pack(&boxes, room_w + spacing, room_h + spacing);
    let origin = Vec2::new(board.x0 + margin, board.y0 + margin);
    // The move that puts the top-left corner of box `b` at `to`: rounded up to whole points for an
    // object on the pixel grid, so it moves at most 1 pt further right and down, into its slack.
    let shift = |b: &Rect, to: Vec2, g: bool| {
        let v = Vec2::new(to.x - b.x0, to.y - b.y0);
        if g { Vec2::new(v.x.ceil(), v.y.ceil()) } else { v }
    };
    // Each move: the quarter turn first, when the object turns, then the shift.
    let mut moves: Vec<(NodeId, Option<Affine>, Vec2)> = vec![];
    let mut aside: Vec<(NodeId, Option<Affine>, Vec2)> = vec![];
    let mut unplaced = vec![];
    let mut rotated = vec![];
    let mut bottom = 0.0_f64;
    // Objects that do not fit go in a column to the right of the sheet, `spacing` from its edge
    // and from each other, so that no object placed on the sheet covers them.
    let aside_x = board.x0 + sheet_w + spacing;
    let mut aside_y = board.y0;
    for ((((id, r), spot), t), g) in items.iter().zip(&at).zip(&turned).zip(&grid) {
        match (spot, t) {
            (Some((x, y, true)), Some((xf, q))) => {
                let dv = shift(q, Vec2::new(origin.x + x, origin.y + y), *g);
                moves.push((*id, Some(*xf), dv));
                rotated.push(id.0);
                bottom = bottom.max(q.y1 + dv.y - origin.y);
            }
            (Some((x, y, _)), _) => {
                let dv = shift(r, Vec2::new(origin.x + x, origin.y + y), *g);
                moves.push((*id, None, dv));
                bottom = bottom.max(r.y1 + dv.y - origin.y);
            }
            (None, _) => {
                aside.push((*id, None, shift(r, Vec2::new(aside_x, aside_y), *g)));
                aside_y += r.height() + spacing + slack(*g);
                unplaced.push(id.0);
            }
        }
    }
    if moves.is_empty() {
        return Err(bad(NEST, "no selected object fits on the sheet"));
    }
    moves.extend(aside);
    let length = 2.0 * margin + bottom;
    let placed = moves.len() - unplaced.len();
    let result = json!({ "placed": placed, "unplaced": unplaced, "rotated": rotated, "skipped": skipped, "length": length });
    if moves.iter().all(|(_, xf, dv)| xf.is_none() && dv.x.abs() <= EPS && dv.y.abs() <= EPS) {
        return Ok(result);
    }
    s.edit("Nest Shapes", |d, _| {
        for (id, xf, dv) in &moves {
            // The turn is the one the object was measured with in the copy, and the shift after it
            // is whole points for an object on the pixel grid.
            for step in xf.iter().copied().chain([Affine::translate(*dv)]) {
                if let Some(n) = d.node_mut(*id) {
                    transform_node(n, step, sc, true);
                }
                keep_compound_aligned(d, *id, sc);
            }
        }
        Ok(())
    })?;
    Ok(result)
}
