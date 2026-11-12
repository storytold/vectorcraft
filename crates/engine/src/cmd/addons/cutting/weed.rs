//! Weed Box and Lines (`addon.cutting.weed`), a command of the built-in Cutting Tools add-on for
//! vinyl cutting: around the selected art it adds a weed box (a rectangle at a margin around the
//! art) and weeding lines across it that stop at the art, so the waste vinyl around the art lifts
//! off in pieces (<https://help.arlon.com/what-is-a-weeding-border>). The art is measured by the
//! outlines a cutter cuts ([`geometry`]).

mod geometry;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, shapes};

use super::super::super::edit::selected_roots;
use super::super::super::object::choice;
use super::super::super::paint::swatch_solid;
use super::super::super::pathops::{length_given, number_given};
use super::super::super::{CommandSpec, bad, cmd, has_selection, switch_or, text_param};
use crate::{EngineError, Result, Session};

/// Weed Box and Lines' command.
pub const WEED: &str = "addon.cutting.weed";
/// Which way the weeding lines run.
pub const DIRECTIONS: [&str; 3] = ["horizontal", "vertical", "both"];
/// The largest margin, gap or spacing, in points.
const MAX_DISTANCE: f64 = 100_000.0;
/// The most weeding lines each way.
const MAX_COUNT: usize = 1000;
/// The most pieces of weeding line one run adds.
const MAX_PIECES: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.cutting.weed",
        "Weed Box and Lines…",
        ["Object", "Plug-ins", "Cutting Tools"],
        None,
        "{margin?: pt 0..100000 (9: from the art to the weed box), box?: true (false: the weeding lines only), direction?: \"horizontal\" | \"vertical\" | \"both\" (\"both\"), count?: whole number 0..1000 of lines each way (1), spacing?: pt more than 0, at most 100000 (instead of count, never with it: as few lines as keep them at most this far apart, up to 1000 each way), gap?: pt 0..100000 (0: the lines stop at the art's outlines), weight?: pt 0.01..1000 (0.3), swatch?: a color swatch the lines are stroked in (\"[Registration]\")} add a weed box around the selected art and weeding lines across it that divide it evenly, each running from one side of the box to the other and left out over the art and within `gap` of its outlines, so the waste lifts off in pieces. The art is measured by the outlines a cutter cuts: closed paths enclose their area by their fill rule and open paths are lines, compound paths by all their subpaths, type by its glyph outlines, a clip group by what shows inside its clipping path, images by their frames, placed files by the art they show (by their boxes when the file can't be read), symbol instances by their art and live objects by the art they draw; strokes, effects and hidden objects are left out. One group named Weeding, stroked with no fill, on top of the current layer (outside the clip groups around a group edited in isolation), selected, as one undo step → {id: the group, box: its weed box or null, lines (pieces of weeding line)}",
        has_selection,
        weed
    )]
}

/// The distance `p` gives for `key` (0..[`MAX_DISTANCE`]), or `default`.
fn distance(p: &Value, key: &str, default: f64) -> Result<f64> {
    match length_given(p, key, WEED)? {
        None => Ok(default),
        Some(v) if (0.0..=MAX_DISTANCE).contains(&v) => Ok(v),
        Some(_) => Err(bad(WEED, format!("`{key}` must be from 0 to {MAX_DISTANCE} pt"))),
    }
}

/// How many lines divide a side `length` long: `count`, or as few as keep them at most `spacing`
/// apart.
fn lines_across(length: f64, count: Option<usize>, spacing: Option<f64>) -> Result<usize> {
    let n = match spacing {
        // `length / spacing` strips, rounded up, need one line fewer.
        Some(s) => ((length / s - 1e-9).ceil() - 1.0).max(0.0),
        None => return Ok(count.unwrap_or(1)),
    };
    if n > MAX_COUNT as f64 {
        return Err(bad(WEED, format!("`spacing` makes more than {MAX_COUNT} lines across the box")));
    }
    Ok(n as usize)
}

/// Where the Weeding group goes: `parent`, or, when `parent` is inside a clip group or clipping
/// sublayer (a group edited in isolation), the container around the outermost of them. A clipping
/// path would hide the lines and the box, which lie outside the art.
fn outside_clips(d: &Document, parent: Option<NodeId>) -> Option<NodeId> {
    let clips = |id: NodeId| d.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. }));
    let (mut at, mut out) = (parent, parent);
    while let Some(p) = at {
        let up = d.parent_of(p);
        if clips(p) {
            match up {
                Some(up) if d.is_editable(up) => out = Some(up),
                _ => break,
            }
        }
        at = up;
    }
    out
}

fn weed(s: &mut Session, p: &Value) -> Result<Value> {
    let margin = distance(p, "margin", 9.0)?;
    let gap = distance(p, "gap", 0.0)?;
    let with_box = switch_or(p, "box", true, WEED)?;
    let direction = choice(p, WEED, "direction", &DIRECTIONS)?.unwrap_or("both");
    let count = match number_given(p, "count", WEED)? {
        None => None,
        Some(n) if n.fract() == 0.0 && (0.0..=MAX_COUNT as f64).contains(&n) => Some(n as usize),
        Some(_) => return Err(bad(WEED, format!("`count` must be a whole number from 0 to {MAX_COUNT}"))),
    };
    let spacing = match length_given(p, "spacing", WEED)? {
        None => None,
        Some(v) if v > 0.0 && v <= MAX_DISTANCE => Some(v),
        Some(_) => return Err(bad(WEED, format!("`spacing` must be more than 0 and at most {MAX_DISTANCE} pt"))),
    };
    if count.is_some() && spacing.is_some() {
        return Err(bad(WEED, "give `count` or `spacing`, not both"));
    }
    let weight = match length_given(p, "weight", WEED)? {
        None => 0.3,
        Some(v) if (0.01..=1000.0).contains(&v) => v,
        Some(_) => return Err(bad(WEED, "`weight` must be from 0.01 to 1000 pt")),
    };
    let swatch = text_param(p, "swatch", WEED)?.unwrap_or(REGISTRATION);
    let stroke = swatch_solid(&s.doc()?.doc, swatch, None).map_err(|e| bad(WEED, e))?;
    let roots = selected_roots(s)?;
    if roots.is_empty() {
        return Err(bad(WEED, "select the art to weed"));
    }
    let d = &s.doc()?.doc;
    let parent = outside_clips(d, s.doc()?.target_parent()?);
    // Placed files' art uses their resources (symbols) under names of their own.
    let placed = d.with_placed_art();
    let region = geometry::Reader::new(&placed).read_ids(&roots).map_err(|e| bad(WEED, e))?;
    let region = region.ok_or_else(|| bad(WEED, "the selection has no outlines to weed around"))?;
    let outline = region.outline().map_err(|e| bad(WEED, e))?;
    let art = outline.iter().fold(None, |r: Option<Rect>, &(a, b)| Some(r.map_or(Rect::from_points(a, b), |r| r.union_pt(a).union_pt(b))));
    let frame = art.ok_or_else(|| bad(WEED, "the selection has no outlines to weed around"))?.inflate(margin, margin);
    if !(frame.width().is_finite() && frame.height().is_finite()) {
        return Err(bad(WEED, "the selected art is too large to measure"));
    }
    let each = |on: bool, length: f64| if on { lines_across(length, count, spacing) } else { Ok(0) };
    let counts =
        geometry::Counts { horizontal: each(direction != "vertical", frame.height())?, vertical: each(direction != "horizontal", frame.width())? };
    let pieces = geometry::lines(&region, &outline, frame, counts, gap, MAX_PIECES).map_err(|e| bad(WEED, e))?;
    if pieces.is_empty() && !with_box {
        return Err(EngineError::Other("Weed Box and Lines: no weeding line fits outside the art, and the weed box is off".into()));
    }
    let ap = Appearance::basic(Paint::None, stroke, weight);
    let (group, weed_box, lines) = s.edit("Weed Box and Lines", |d, sel| {
        let mut children = vec![];
        let mut weed_box = None;
        if with_box {
            let mut b = Node::path(d.alloc_id(), shapes::rectangle(frame), ap.clone());
            b.name = Some("Weed Box".into());
            weed_box = Some(b.id);
            children.push(Arc::new(b));
        }
        for (a, b) in &pieces {
            children.push(Arc::new(Node::path(d.alloc_id(), shapes::line(*a, *b), ap.clone())));
        }
        let mut g = Node::group(d.alloc_id(), children);
        g.name = Some("Weeding".into());
        let id = g.id;
        d.insert(parent, usize::MAX, g)?;
        sel.set([id]);
        Ok((id, weed_box, pieces.len()))
    })?;
    Ok(json!({ "id": group.0, "box": weed_box.map(|b| b.0), "lines": lines }))
}
