//! Align and Distribute for artboards: what the Align panel's buttons run while the Artboard tool
//! is chosen.

use serde_json::{Value, json};
use vectorcraft_doc::Scaling;
use vectorcraft_geom::{Rect, Vec2};

use super::object::choice;
use super::panelcmds::{artboard_index, artboards_art, locked_and_hidden, named_or_selected, note_artboards};
use super::{Affine, CommandSpec, NodeId, Result, Session, bad, check_switches, cmd, has_doc, switch_or};
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "artboard.align",
            "Align",
            [],
            None,
            "{index? | indices?: [n…], selectedArtboards?: [n…] (as artboard.duplicate; default: the artboards the Artboard tool has selected, else its active one), horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\" (one of the two is needed), to?: \"selection\"|\"artboard\" (default \"selection\": the bounds of the artboards together; \"artboard\": artboard `artboard`, which stays where it is), artboard?: n (0-based; the artboard to align to with to: \"artboard\"; default: the Artboard tool's active one; it need not be one of the artboards aligned), activeArtboard?: n (the journal records that default here, for a replay; `artboard` goes ahead of it), moveArt?: bool (the art fully inside each artboard moves with it, art inside two of them with the first by index, which may stay; default: the Artboard tool's Move Artwork with Artboard option), patterns?: bool, lockedAndHidden?: bool (as artboard.move)} (Align panel › Align Objects with the Artboard tool chosen) each artboard moves until its edge or center is on that edge or center of what it aligns to, with its guides, in one undo step; a locked artboard stays where it is and still counts in the bounds, and the call fails with `the artboard is locked` when every artboard it names is locked; when no artboard moves, no undo step is recorded → {moved: [index…]}",
            has_doc,
            align
        ),
        cmd!(
            "artboard.distribute",
            "Distribute",
            [],
            None,
            "{index? | indices?, selectedArtboards? (as artboard.align), horizontal?: \"left\"|\"center\"|\"right\" | vertical?: \"top\"|\"center\"|\"bottom\" (one is needed; horizontal goes first), to?: \"selection\"|\"artboard\", artboard?: n, activeArtboard?: n (as artboard.align; with `spacing` the artboard must be one of those distributed), spacing?: pt (used with to: \"artboard\"), moveArt?, patterns?, lockedAndHidden? (as artboard.align)} (Align panel › Distribute Objects with the Artboard tool chosen) the named edges or centers of the artboards are spaced evenly between the first and the last along the axis, which stay where they are (three or more artboards); with to: \"artboard\", the artboards are spaced the same way and then moved together so that artboard `artboard`, when it is one of them, stays where it is; with to: \"artboard\" and `spacing`, that artboard stays and the edges or centers are placed `spacing` apart, outward from it in their order (two or more); a locked artboard stays where it is and keeps its place in the order, and the call fails with `the artboard is locked` when every artboard it names is locked; a spacing that would move an artboard past the canvas is an error; with fewer artboards, or when none moves, no undo step is recorded → {moved: [index…]}",
            has_doc,
            distribute
        ),
        cmd!(
            "artboard.distributeSpacing",
            "Distribute Spacing",
            [],
            None,
            "{index? | indices?, selectedArtboards? (as artboard.align), axis?: \"horizontal\"|\"vertical\" (default horizontal), to?: \"selection\"|\"artboard\", artboard?: n, activeArtboard?: n (as artboard.distribute), spacing?: pt (used with to: \"artboard\"), moveArt?, patterns?, lockedAndHidden? (as artboard.align)} (Align panel › Distribute Spacing with the Artboard tool chosen) the gaps between the artboards along the axis become equal, the first and the last staying where they are (three or more artboards); with to: \"artboard\", the gaps become equal the same way and the artboards then move together so that artboard `artboard`, when it is one of them, stays where it is; with to: \"artboard\" and `spacing`, that artboard stays and the others are placed with a gap of `spacing` between neighbors (two or more; a negative spacing overlaps them); a locked artboard stays where it is and keeps its place in the order, and the call fails with `the artboard is locked` when every artboard it names is locked; a spacing that would move an artboard past the canvas is an error; with fewer artboards, or when none moves, no undo step is recorded → {moved: [index…]}",
            has_doc,
            distribute_spacing
        ),
    ]
}

/// What an artboard Align or Distribute command acts on, read from its params.
struct Boards {
    /// The artboards named, in artboard order, each with its rectangle.
    items: Vec<(usize, Rect)>,
    /// With `to: "artboard"`, the artboard to align to and its rectangle.
    reference: Option<(usize, Rect)>,
    /// Whether the art inside each artboard moves with it.
    move_art: bool,
    /// Whether locked and hidden art moves too.
    locked_art: bool,
}

/// Reads the params that `artboard.align`, `artboard.distribute` and `artboard.distributeSpacing`
/// (command `cmd`) share, and adds the artboards and the art choice to the journal entry. The
/// artboard to align to is `artboard`, else `activeArtboard` (a replay of the journal), else the
/// Artboard tool's active one, which the journal entry then records as `activeArtboard`. It fails
/// when every artboard named is locked.
fn boards(s: &mut Session, p: &Value, cmd: &str) -> Result<Boards> {
    check_switches(p, &["patterns"], cmd)?;
    let to_artboard = choice(p, cmd, "to", &["selection", "artboard"])? == Some("artboard");
    let indices = named_or_selected(s, p, cmd)?;
    let given = |key: &str| -> Result<Option<usize>> {
        match p.get(key).filter(|v| !v.is_null()) {
            Some(v) => artboard_index(&s.doc()?.doc, &json!({ "index": v }), cmd).map(Some),
            None => Ok(None),
        }
    };
    let (named, recorded) = (given("artboard")?, given("activeArtboard")?);
    let move_art = switch_or(p, "moveArt", s.tool_options_of("artboard")["moveArt"].as_bool().unwrap_or(true), cmd)?;
    let locked_art = locked_and_hidden(s, p, move_art, cmd)?;
    note_artboards(s, p, &indices);
    s.note_journal("moveArt", json!(move_art));
    let reference = match (to_artboard, named.or(recorded)) {
        (false, _) => None,
        (true, Some(i)) => Some(i),
        (true, None) => {
            let active = s.tool_options_of("artboard")["active"].clone();
            Some(artboard_index(&s.doc()?.doc, &json!({ "index": active }), cmd)?)
        }
    };
    if let Some(i) = reference.filter(|_| named.is_none()) {
        s.note_journal("activeArtboard", json!(i));
    }
    let d = &s.doc()?.doc;
    let rect = |i: usize| d.artboards.get(i).map(|a| (i, a.rect)).ok_or_else(|| EngineError::Other("no such artboard".into()));
    let items = indices.iter().map(|i| rect(*i)).collect::<Result<Vec<_>>>()?;
    if indices.iter().all(|i| d.artboards.get(*i).is_some_and(|a| a.locked)) {
        return Err(EngineError::Other("the artboard is locked".into()));
    }
    Ok(Boards { items, reference: reference.map(rect).transpose()?, move_art, locked_art })
}

/// `spacing` of the Distribute commands: a finite number, or `None` when it is missing or null.
fn spacing(p: &Value, cmd: &str) -> Result<Option<f64>> {
    p.get("spacing")
        .filter(|v| !v.is_null())
        .map(|v| v.as_f64().filter(|g| g.is_finite()).ok_or_else(|| bad(cmd, "`spacing` must be a number")))
        .transpose()
}

/// The place in `items` of the artboard `b` aligns to, which the Distribute commands keep where it
/// is. `None` without one, or when it is not one of `items`; with a `gap` that is an error
/// (`no such artboard`), since the gap is measured from it.
fn kept(b: &Boards, items: &[(usize, Rect)], gap: Option<f64>) -> Result<Option<usize>> {
    let Some((reference, _)) = b.reference else { return Ok(None) };
    match items.iter().position(|(i, _)| *i == reference) {
        None if gap.is_some() => Err(EngineError::Other("no such artboard".into())),
        at => Ok(at),
    }
}

/// Moves each artboard of `moves` by its offset as undo step `label`, with its guides and, when
/// `b` says so, the art inside it; art inside two of the artboards `b` names goes with the one
/// first in artboard order, and stays when that one stays. Locked artboards stay where they are.
/// When no artboard moves
/// more than 1e-9 pt on either axis, the document and its undo history stay as they are. The edit
/// fails with nothing changed when an artboard would end up past the canvas, as every edit does.
fn apply(s: &mut Session, label: &str, p: &Value, b: &Boards, moves: &[(usize, Vec2)]) -> Result<Value> {
    let d = &s.doc()?.doc;
    let mut moves: Vec<(usize, Vec2)> =
        moves.iter().copied().filter(|(i, dv)| (dv.x.abs() > 1e-9 || dv.y.abs() > 1e-9) && d.artboards.get(*i).is_some_and(|a| !a.locked)).collect();
    // In artboard order, as the other artboard edits take them, whatever order the Distribute
    // commands spaced them in.
    moves.sort_by_key(|(i, _)| *i);
    let moved: Vec<usize> = moves.iter().map(|(i, _)| *i).collect();
    if moves.is_empty() {
        return Ok(json!({ "moved": moved }));
    }
    // The art of every artboard named, those that stay included, so that art on overlapping
    // artboards goes with the first by index; then that of the artboards that move.
    let named: Vec<usize> = b.items.iter().map(|(i, _)| *i).collect();
    let arts: Vec<(usize, Vec<NodeId>)> =
        artboards_art(s, &named, b.move_art, b.locked_art, true)?.into_iter().filter(|(i, _)| moved.contains(i)).collect();
    let ids: Vec<NodeId> = arts.iter().flat_map(|(_, a)| a.iter().copied()).collect();
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &ids)?, ..Scaling::default() };
    s.edit(label, |d, _| {
        for ((i, dv), (_, art)) in moves.iter().zip(&arts) {
            let a = d.artboards.get_mut(*i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
            a.rect = a.rect + *dv;
            let id = a.id;
            d.move_artboard_guides(id, *dv);
            for id in art {
                if let Some(n) = d.node_mut(*id) {
                    n.transform(Affine::translate(*dv), sc);
                    super::object::keep_pixel_aligned(n, sc);
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "moved": moved }))
}

fn align(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.align";
    let h = choice(p, C, "horizontal", &["left", "center", "right"])?;
    let v = choice(p, C, "vertical", &["top", "center", "bottom"])?;
    if h.is_none() && v.is_none() {
        return Err(bad(C, "give horizontal or vertical"));
    }
    let b = boards(s, p, C)?;
    let Some(r) = b.reference.map(|(_, r)| r).or_else(|| b.items.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b))) else {
        return Ok(json!({ "moved": [] }));
    };
    let moves: Vec<(usize, Vec2)> = b
        .items
        .iter()
        .map(|(i, a)| {
            let dx = match h {
                Some("left") => r.x0 - a.x0,
                Some("center") => r.center().x - a.center().x,
                Some("right") => r.x1 - a.x1,
                _ => 0.0,
            };
            let dy = match v {
                Some("top") => r.y0 - a.y0,
                Some("center") => r.center().y - a.center().y,
                Some("bottom") => r.y1 - a.y1,
                _ => 0.0,
            };
            (*i, Vec2::new(dx, dy))
        })
        .collect();
    apply(s, "Align", p, &b, &moves)
}

fn distribute(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.distribute";
    let h = choice(p, C, "horizontal", &["left", "center", "right"])?;
    let v = choice(p, C, "vertical", &["top", "center", "bottom"])?;
    let (horiz, line): (bool, fn(&Rect) -> f64) = match (h, v) {
        (Some("left"), _) => (true, |r| r.x0),
        (Some("right"), _) => (true, |r| r.x1),
        (Some(_), _) => (true, |r| r.center().x),
        (None, Some("top")) => (false, |r| r.y0),
        (None, Some("bottom")) => (false, |r| r.y1),
        (None, Some(_)) => (false, |r| r.center().y),
        (None, None) => return Err(bad(C, "give horizontal or vertical")),
    };
    let gap = spacing(p, C)?;
    let b = boards(s, p, C)?;
    let mut items = b.items.clone();
    items.sort_by(|x, y| line(&x.1).total_cmp(&line(&y.1)));
    let kept = kept(&b, &items, gap)?;
    // With a spacing: the place in the order and the line of the artboard it is measured from.
    let from = kept.zip(gap).and_then(|(at, gap)| items.get(at).map(|(_, r)| (at, gap, line(r))));
    let n = items.len();
    // Two artboards are the ends already; with a spacing the second is placed from the first.
    if n < if from.is_some() { 2 } else { 3 } {
        return Ok(json!({ "moved": [] }));
    }
    let (Some((_, first)), Some((_, last))) = (items.first(), items.last()) else { return Ok(json!({ "moved": [] })) };
    let (first, last) = (line(first), line(last));
    let deltas: Vec<f64> = items
        .iter()
        .enumerate()
        .map(|(k, (_, r))| {
            let target = match from {
                Some((at, gap, origin)) => origin + gap * (k as f64 - at as f64),
                None => first + (last - first) * k as f64 / (n - 1) as f64,
            };
            target - line(r)
        })
        .collect();
    // The artboard aligned to stays where it is, and the others keep their places from it.
    let hold = kept.and_then(|at| deltas.get(at)).copied().unwrap_or(0.0);
    let moves: Vec<(usize, Vec2)> = items.iter().zip(&deltas).map(|((i, _), d)| (*i, along(horiz, d - hold))).collect();
    apply(s, "Distribute", p, &b, &moves)
}

fn distribute_spacing(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.distributeSpacing";
    let horiz = choice(p, C, "axis", &["horizontal", "vertical"])? != Some("vertical");
    let gap = spacing(p, C)?;
    let b = boards(s, p, C)?;
    let start = |r: &Rect| if horiz { r.x0 } else { r.y0 };
    let size = |r: &Rect| if horiz { r.width() } else { r.height() };
    let mut items = b.items.clone();
    items.sort_by(|x, y| start(&x.1).total_cmp(&start(&y.1)));
    let kept = kept(&b, &items, gap)?;
    let gap = gap.filter(|_| kept.is_some());
    let n = items.len();
    if n < if gap.is_some() { 2 } else { 3 } {
        return Ok(json!({ "moved": [] }));
    }
    let (Some((_, first)), Some((_, last))) = (items.first(), items.last()) else { return Ok(json!({ "moved": [] })) };
    let (lo, hi) = (start(first), start(last) + size(last));
    let gap = gap.unwrap_or_else(|| (hi - lo - items.iter().map(|(_, r)| size(r)).sum::<f64>()) / (n - 1) as f64);
    let mut pos = lo;
    let mut deltas = Vec::with_capacity(n);
    for (i, r) in &items {
        deltas.push((*i, pos - start(r)));
        pos += size(r) + gap;
    }
    // The artboard aligned to stays where it is, and the others keep their places from it.
    let hold = kept.and_then(|at| deltas.get(at)).map_or(0.0, |d| d.1);
    let moves: Vec<(usize, Vec2)> = deltas.into_iter().map(|(i, d)| (i, along(horiz, d - hold))).collect();
    apply(s, "Distribute Spacing", p, &b, &moves)
}

/// An offset of `d` along the axis: x when `horiz`, else y.
fn along(horiz: bool, d: f64) -> Vec2 {
    if horiz { Vec2::new(d, 0.0) } else { Vec2::new(0.0, d) }
}

#[cfg(test)]
mod tests;
