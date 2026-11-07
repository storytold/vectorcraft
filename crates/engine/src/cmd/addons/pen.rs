//! Smart Pen (`addon.pen.*`), a built-in add-on: one tool that draws paths and edits their points,
//! handles and segments without switching tools, and a panel. The commands here are what the tool
//! and the panel do that the standard path commands don't: anchors added with a straight or curved
//! segment into them, segments switched between straight and curved, anchors removed with or
//! without refitting their neighbours, smooth/corner toggles, segments bent by a dragged point, and
//! the panel's next-segment choice and Close or Join. The tool, panel and submenu (Object ›
//! Plug-ins › Smart Pen) are the tools crate's and the UI's.

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{Anchor, AnchorKind, Point, SubPath, Vec2};
use vectorcraft_pathops::addons::{self as po, pathtools as pt, pen};
use vectorcraft_tools::addons::smart_pen;

use super::super::draw2::usize_opt;
use super::super::path::{compound_or_self, join, path_mut};
use super::super::{CommandSpec, always, bad, checked_id, cmd, finite_numbers, has_doc, has_selection, number_or, switch_or, text_param};
use crate::{EngineError, Result, Session};

/// The largest coordinate or move (pt) the commands take.
const LIMIT: f64 = 1e9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.pen.append",
            "Smart Pen",
            [],
            None,
            "{id, subpath?: index (default: the last open subpath), x, y, in?: [x, y], out?: [x, y], segment?: \"straight\" (the segment into the new anchor is a line: the previous anchor's handle on it goes, and the new anchor's incoming one)|\"curved\" (the previous anchor gets a handle on it if it has none: in line with its other handle, or with a straight segment before it)} add an anchor at the end of an open subpath of path id (default: the handles as given, the previous anchor as it is), as one undo step; the new anchor is selected → {anchor}",
            has_doc,
            append
        ),
        cmd!(
            "addon.pen.close",
            "Close Path",
            [],
            None,
            "{id, subpath?: index (default: the last open subpath), segment?: \"straight\"|\"curved\" (default: the closing segment takes the handles its ends have)} close an open subpath of two anchors or more, as one undo step",
            has_doc,
            close
        ),
        cmd!(
            "addon.pen.segment",
            "Straight or Curved Segment",
            [],
            None,
            "{id, subpath?: index (0), segment: index, curved: bool} make a segment straight (its handles retracted) or curved (each end without a handle on it gets one, in line with its other handle or with a straight segment before it), as one undo step → {changed}",
            has_doc,
            segment
        ),
        cmd!(
            "addon.pen.removePoint",
            "Remove Points, Keep Shape",
            [],
            None,
            "{id, subpath?: index (0), anchor: index, plain?: bool (false: the handles beside it are refitted so the outline keeps its shape, and an open end takes its segment with it; true: it goes and the handles beside it stay as they are)} remove one anchor, as one undo step; a subpath keeps two anchors (three when closed)",
            has_doc,
            remove_point
        ),
        cmd!(
            "addon.pen.togglePoint",
            "Convert Anchor Point",
            [],
            None,
            "{id, subpath?: index (0), anchor: index, ratio?: 0.1..0.5 (0.3905)} a smooth anchor becomes a corner with its handles where they are; any other anchor is made smooth by the auto-smooth rule (a lone handle mirrored; without handles both at right angles to the bisector of the angle at it, ratio × the nearer neighbour's distance long). One undo step → {smooth}",
            has_doc,
            toggle_point
        ),
        cmd!(
            "addon.pen.reshape",
            "Reshape Segment",
            [],
            None,
            "{id, subpath?: index (0), segment: index, t?: 0..1 (where it was grabbed, 0.5; held to 0.05..0.95), dx, dy: pt, mode?: \"free\" (both handles move, the one nearer the grab more)|\"keepAngles\" (each handle keeps its direction)|\"symmetric\" (both handles at right angles to the chord, at one length) (\"free\")} bend a segment so the point grabbed moves by (dx, dy); missing handles grow out of their anchors and a smooth anchor stays smooth. One undo step",
            has_doc,
            reshape
        ),
        cmd!(
            "addon.pen.nextSegment",
            "Next Segment",
            ["Object", "Plug-ins", "Smart Pen"],
            None,
            "{curved: bool} whether a click with the Smart Pen Tool adds a curved segment or a straight one (the tool's `next` option, kept with the preferences) → {next: \"straight\"|\"curved\"}",
            always,
            next_segment
        ),
        cmd!(
            "addon.pen.closeJoin",
            "Close or Join",
            ["Object", "Plug-ins", "Smart Pen"],
            None,
            "{} with exactly two open path ends direct-selected, join them as path.join does; else close the selected open paths keeping their ends' handles, as addon.pathtools.close does. One undo step",
            has_selection,
            close_join
        ),
    ]
}

/// The `segment` param: straight (false) or curved (true), `None` when not given.
fn segment_kind(p: &Value, cmd: &str) -> Result<Option<bool>> {
    match text_param(p, "segment", cmd)? {
        None => Ok(None),
        Some("straight") => Ok(Some(false)),
        Some("curved") => Ok(Some(true)),
        Some(_) => Err(bad(cmd, "`segment` must be \"straight\" or \"curved\"")),
    }
}

/// An index param that must be given.
fn index_req(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    usize_opt(p, key, cmd)?.ok_or_else(|| bad(cmd, format!("missing integer `{key}`")))
}

/// A true or false param that must be given.
fn switch_req(p: &Value, key: &str, cmd: &str) -> Result<bool> {
    p.get(key).and_then(Value::as_bool).ok_or_else(|| bad(cmd, format!("`{key}` must be true or false")))
}

/// Two number params that must be given, finite and within [`LIMIT`].
fn pair(p: &Value, (a, b): (&str, &str), cmd: &str) -> Result<(f64, f64)> {
    let (x, y) = (number_or(p, a, f64::NAN, cmd)?, number_or(p, b, f64::NAN, cmd)?);
    if !(x.abs() < LIMIT && y.abs() < LIMIT) {
        return Err(bad(cmd, format!("give `{a}` and `{b}`")));
    }
    Ok((x, y))
}

/// The subpath a request names: its path (`id`, editable), its index (`subpath`; without one the
/// last open subpath with `open`, else the first) and its data.
fn subpath_of(s: &Session, p: &Value, cmd: &str, open: bool) -> Result<(NodeId, usize, SubPath)> {
    let id = checked_id(s, p.get("id").unwrap_or(&Value::Null), cmd)?;
    let d = &s.doc()?.doc;
    if !d.is_editable(id) {
        return Err(bad(cmd, "the path is locked or hidden"));
    }
    let path = d.node(id).and_then(|n| n.path_data()).ok_or_else(|| bad(cmd, "`id` is not a path"))?;
    let si = match usize_opt(p, "subpath", cmd)? {
        Some(si) => si,
        None if open => path.subpaths.iter().rposition(|sp| !sp.closed).ok_or_else(|| bad(cmd, "the path has no open subpath"))?,
        None => 0,
    };
    let sp = path.subpaths.get(si).ok_or_else(|| bad(cmd, "no such subpath"))?;
    Ok((id, si, sp.clone()))
}

/// What the selection becomes once a subpath is edited.
enum Pick {
    /// As it was (no anchor moved to another index).
    Keep,
    /// The path (its compound path) selected whole.
    Whole,
    /// Only this anchor of the subpath, direct-selected.
    Anchor(usize),
}

/// Put `sp` in place of subpath `si` of path `id`, as one undo step labelled `label`.
fn put(s: &mut Session, label: &str, id: NodeId, si: usize, sp: SubPath, pick: Pick) -> Result<()> {
    s.edit(label, |d, sel| {
        let slot = path_mut(d, id)?.subpaths.get_mut(si).ok_or_else(|| EngineError::Other("no such subpath".into()))?;
        *slot = sp;
        let top = compound_or_self(d, id);
        match pick {
            Pick::Keep => {}
            Pick::Whole => sel.set([top]),
            Pick::Anchor(ai) => {
                sel.set([top]);
                sel.anchors.insert(id, BTreeSet::from([(si, ai)]));
            }
        }
        Ok(())
    })
}

/// An optional `[x, y]` param, finite and within [`LIMIT`].
fn point_opt(p: &Value, key: &str, cmd: &str) -> Result<Option<Point>> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => finite_numbers::<2>(v)
            .filter(|[x, y]| x.abs() < LIMIT && y.abs() < LIMIT)
            .map(|[x, y]| Some(Point::new(x, y)))
            .ok_or_else(|| bad(cmd, format!("`{key}` must be [x, y]"))),
    }
}

fn append(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pen.append";
    let (x, y) = pair(p, ("x", "y"), C)?;
    let at = Point::new(x, y);
    let (h_in, h_out) = (point_opt(p, "in", C)?.unwrap_or(at), point_opt(p, "out", C)?.unwrap_or(at));
    let curved = segment_kind(p, C)?;
    let (id, si, mut sp) = subpath_of(s, p, C, true)?;
    if sp.closed {
        return Err(bad(C, "the subpath is closed"));
    }
    sp.anchors.push(Anchor::with_handles(at, h_in, h_out));
    let k = sp.anchors.len() - 1;
    if let Some(prev) = k.checked_sub(1) {
        match curved {
            Some(true) => {
                pen::leave_handle(&mut sp, prev, true, pt::SMOOTH_RATIO);
            }
            Some(false) => {
                pen::straighten_segment(&mut sp, prev);
            }
            None => {}
        }
    }
    put(s, "Smart Pen", id, si, sp, Pick::Anchor(k))?;
    Ok(json!({ "anchor": k }))
}

fn close(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pen.close";
    let curved = segment_kind(p, C)?;
    let (id, si, mut sp) = subpath_of(s, p, C, true)?;
    if sp.closed || sp.anchors.len() < 2 {
        return Err(bad(C, "the subpath is closed or too short to close"));
    }
    sp.closed = true;
    let last = sp.anchors.len() - 1;
    match curved {
        Some(true) => {
            pen::curve_segment(&mut sp, last, pt::SMOOTH_RATIO);
        }
        Some(false) => {
            pen::straighten_segment(&mut sp, last);
        }
        None => {}
    }
    put(s, "Close Path", id, si, sp, Pick::Whole)?;
    Ok(json!({}))
}

fn segment(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pen.segment";
    let seg = index_req(p, "segment", C)?;
    let curved = switch_req(p, "curved", C)?;
    let (id, si, mut sp) = subpath_of(s, p, C, false)?;
    if seg >= sp.segment_count() {
        return Err(bad(C, "no such segment"));
    }
    let changed = if curved { pen::curve_segment(&mut sp, seg, pt::SMOOTH_RATIO) } else { pen::straighten_segment(&mut sp, seg) };
    if changed {
        put(s, "Smart Pen", id, si, sp, Pick::Keep)?;
    }
    Ok(json!({ "changed": changed }))
}

fn remove_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pen.removePoint";
    let ai = index_req(p, "anchor", C)?;
    let plain = switch_or(p, "plain", false, C)?;
    let (id, si, mut sp) = subpath_of(s, p, C, false)?;
    let n = sp.anchors.len();
    if ai >= n {
        return Err(bad(C, "no such anchor"));
    }
    let end = !sp.closed && (ai == 0 || ai + 1 == n);
    let removed = if plain {
        pen::delete_anchor(&mut sp, ai)
    } else if let Some((out, _)) = po::remove_anchor_keep_shape(&sp, ai) {
        sp = out;
        true
    } else {
        // An open end takes its segment with it.
        end && n > 2 && vectorcraft_pathops::remove_anchor(&mut sp, ai)
    };
    if !removed {
        return Err(bad(C, "the path is too short to remove a point from"));
    }
    let label = if plain { "Delete Anchor Point" } else { "Remove Points, Keep Shape" };
    put(s, label, id, si, sp, Pick::Whole)?;
    Ok(json!({}))
}

fn toggle_point(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pen.togglePoint";
    let ai = index_req(p, "anchor", C)?;
    let r = super::pathtools::ratio(p, C)?;
    let (id, si, mut sp) = subpath_of(s, p, C, false)?;
    if ai >= sp.anchors.len() {
        return Err(bad(C, "no such anchor"));
    }
    let changed = pen::toggle_smooth(&mut sp, ai, r);
    let smooth = sp.anchors.get(ai).is_some_and(|a| a.kind == AnchorKind::Smooth);
    if changed {
        put(s, "Convert Anchor Point", id, si, sp, Pick::Keep)?;
    }
    Ok(json!({ "smooth": smooth }))
}

fn reshape(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pen.reshape";
    let seg = index_req(p, "segment", C)?;
    let t = number_or(p, "t", 0.5, C)?;
    if !(0.0..=1.0).contains(&t) {
        return Err(bad(C, "`t` must be from 0 to 1"));
    }
    let (dx, dy) = pair(p, ("dx", "dy"), C)?;
    let dv = Vec2::new(dx, dy);
    let mode = match text_param(p, "mode", C)? {
        None => po::ReshapeMode::Free,
        Some(m) => po::ReshapeMode::parse(m).ok_or_else(|| bad(C, "`mode` must be \"free\", \"keepAngles\" or \"symmetric\""))?,
    };
    let (id, si, mut sp) = subpath_of(s, p, C, false)?;
    if seg >= sp.segment_count() {
        return Err(bad(C, "no such segment"));
    }
    if !po::reshape_segment(&mut sp, seg, t, dv, mode) {
        return Err(bad(C, "the segment can't be bent that far"));
    }
    put(s, "Reshape Segment", id, si, sp, Pick::Keep)?;
    Ok(json!({}))
}

fn next_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pen.nextSegment";
    let curved = switch_req(p, "curved", C)?;
    let next = if curved { "curved" } else { "straight" };
    let values = Map::from_iter([("next".to_string(), json!(next))]);
    s.set_tool_options(Some(smart_pen::ID), &values);
    Ok(json!({ "next": next }))
}

fn close_join(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let d = &st.doc;
    // The direct-selected anchors that are ends of open subpaths.
    let mut picked = 0;
    let mut ends = 0;
    for (id, set) in &st.selection.anchors {
        let path = d.node(*id).and_then(|n| n.path_data());
        for &(si, ai) in set {
            picked += 1;
            let sp = path.and_then(|pd| pd.subpaths.get(si));
            ends += usize::from(sp.is_some_and(|sp| !sp.closed && (ai == 0 || ai + 1 == sp.anchors.len())));
        }
    }
    if picked == 2 && ends == 2 {
        return join(s, &json!({}));
    }
    super::pathtools::close(s, &json!({ "mode": "handles" }))
}
