//! The Quick Pen tool's commands: `path.quickPen` draws a path of straight lines and circular arcs,
//! and `path.quickPenLength` makes the line or arc at an end of a path longer or shorter.

use vectorcraft_tools::quickpen::length as qpl;
use vectorcraft_tools::quickpen::{self as qp, Piece, Stroke};

use super::common::{path_kind, path_mut, point_entry, usize_opt, within_input_bound};
use super::{Color, Paint, PathData, Result, Session, SubPath, Value, add_look, bad, json, number_or, object_param, switch_or};
use crate::cmd::f64_req;

const C: &str = "path.quickPen";
/// The most pieces one request takes.
const MAX_PIECES: usize = 10_000;

/// The `pieces` param.
fn pieces(p: &Value) -> Result<Vec<Piece>> {
    let list = match p.get("pieces") {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Array(list)) if list.len() <= MAX_PIECES => list,
        Some(_) => return Err(bad(C, "`pieces` must be a list of at most 10000 pieces")),
    };
    list.iter()
        .map(|v| {
            let to = point_entry(v, C)?;
            if !within_input_bound(to) {
                return Err(bad(C, "x and y must be finite"));
            }
            let stroke = match v.get("action") {
                None | Some(Value::Null) => Stroke::Line,
                Some(a) => a.as_str().and_then(Stroke::parse).ok_or_else(|| {
                    let key = "action";
                    bad(C, format!("`{key}` must be one of {}", Stroke::ALL.map(Stroke::name).join(", ")))
                })?,
            };
            let turn = number_or(v, "turn", qp::DEFAULT_TURN, C)?;
            if !(qp::MIN_TURN..=qp::MAX_TURN).contains(&turn) {
                let (key, min, max) = ("turn", qp::MIN_TURN, qp::MAX_TURN);
                return Err(bad(C, format!("{key} must be a number from {min} to {max}")));
            }
            let flip = switch_or(v, "flip", false, C)?;
            Ok(Piece { to, stroke, turn, flip })
        })
        .collect()
}

pub(super) fn quick_pen(s: &mut Session, p: &Value) -> Result<Value> {
    let pieces = pieces(p)?;
    let closed = switch_or(p, "closed", false, C)?;
    let start_given = p.get("start").is_some_and(|v| !v.is_null());
    let either = "give either `start` for a new path or `id`, not both";
    let Some(id) = object_param(p, "id", C)? else {
        let start = match p.get("start") {
            Some(v) if start_given => point_entry(v, C)?,
            _ => return Err(bad(C, either)),
        };
        if !within_input_bound(start) {
            return Err(bad(C, "x and y must be finite"));
        }
        if ["subpath", "keep"].iter().any(|key| p.get(key).is_some_and(|v| !v.is_null())) {
            return Err(bad(C, "`subpath` and `keep` need `id`"));
        }
        let sp = qp::build(&SubPath::new(vec![vectorcraft_geom::Anchor::corner(start)], false), &pieces, closed);
        let mut look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
        let ap = &mut look.appearance;
        if ap.stroke_paint().is_none() && ap.fill_paint().is_none() {
            ap.set_stroke(Paint::solid(Color::BLACK));
        }
        return add_look(s, "Quick Pen", path_kind(PathData::single(sp)), look, None);
    };
    if start_given {
        return Err(bad(C, either));
    }
    let si = usize_opt(p, "subpath", C)?.unwrap_or(0);
    let d = &s.doc()?.doc;
    if !d.is_editable(id) {
        return Err(bad(C, "the path is locked or hidden"));
    }
    let path = d.node(id).and_then(|n| n.path_data()).ok_or_else(|| bad(C, "`id` is not a path"))?;
    let sp = path.subpaths.get(si).ok_or_else(|| bad(C, "no such subpath"))?;
    if sp.closed {
        return Err(bad(C, "the subpath is closed"));
    }
    let n = sp.anchors.len();
    let keep = usize_opt(p, "keep", C)?.unwrap_or(n);
    if keep == 0 || keep > n {
        let (key, min, max) = ("keep", 1, n);
        return Err(bad(C, format!("{key} must be a number from {min} to {max}")));
    }
    let base = SubPath::new(sp.anchors.get_range(..keep).unwrap_or_default(), false);
    let out = qp::build(&base, &pieces, closed);
    let top = super::super::path::compound_or_self(d, id);
    s.edit("Quick Pen", |d, sel| {
        let slot = path_mut(d, id)?.subpaths.get_mut(si).ok_or_else(|| crate::EngineError::Other("no such subpath".into()))?;
        *slot = out;
        sel.set([top]);
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}

const L: &str = "path.quickPenLength";

/// `path.quickPenLength`: the line or arc at an end of an open subpath made `length` long.
pub(super) fn quick_pen_length(s: &mut Session, p: &Value) -> Result<Value> {
    let Some(id) = object_param(p, "id", L)? else { return Err(bad(L, "missing `id`")) };
    let si = usize_opt(p, "subpath", L)?.unwrap_or(0);
    let at_start = match p.get("end") {
        None | Some(Value::Null) => false,
        Some(v) => match v.as_str() {
            Some("end") => false,
            Some("start") => true,
            _ => {
                let key = "end";
                return Err(bad(L, format!("`{key}` must be one of {}", ["start", "end"].join(", "))));
            }
        },
    };
    let length = f64_req(p, "length", L)?;
    let d = &s.doc()?.doc;
    let path = d.node(id).and_then(|n| n.path_data()).ok_or_else(|| bad(L, "`id` is not a path"))?;
    if !d.is_editable(id) {
        return Err(bad(L, "the path is locked or hidden"));
    }
    let sp = path.subpaths.get(si).ok_or_else(|| bad(L, "no such subpath"))?;
    if sp.closed {
        return Err(bad(L, "the subpath is closed"));
    }
    let piece = qpl::end_piece(sp, at_start).ok_or_else(|| bad(L, "the end is not a straight line or a circular arc"))?;
    let (key, lo, hi) = ("length", qpl::MIN_LENGTH, piece.max_length());
    if !(lo..=hi).contains(&length) {
        // An arc's limit, 359° of its circle, is given to the thousandth of a point, rounded down.
        let hi = (hi * 1000.0).floor() / 1000.0;
        return Err(bad(L, format!("`{key}` must be from {lo} to {hi}")));
    }
    let out = qpl::with_length(sp, at_start, length).ok_or_else(|| bad(L, "the end is not a straight line or a circular arc"))?;
    if !qp::on_canvas(&out) {
        return Err(bad(L, "result would exceed the canvas (coordinates out of range)"));
    }
    let kind = match piece.shape {
        qpl::EndShape::Line => "line",
        qpl::EndShape::Arc { .. } => "arc",
    };
    if (length - piece.length).abs() <= 1e-9 * length.max(1.0) {
        return Ok(json!({ "id": id.0, "kind": kind, "length": piece.length }));
    }
    let top = super::super::path::compound_or_self(d, id);
    s.edit("Quick Pen", |d, sel| {
        let slot = path_mut(d, id)?.subpaths.get_mut(si).ok_or_else(|| crate::EngineError::Other("no such subpath".into()))?;
        *slot = out;
        sel.set([top]);
        Ok(())
    })?;
    Ok(json!({ "id": id.0, "kind": kind, "length": length }))
}
