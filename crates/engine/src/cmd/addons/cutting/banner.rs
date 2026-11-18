//! Banner Hems and Grommets (`addon.cutting.banner`), a command of the built-in Cutting Tools
//! add-on for finishing a printed banner: on each chosen side of the banner it adds a hem line,
//! where the edge is folded back and sewn (<https://en.wikipedia.org/wiki/Hem>), and grommet
//! marks, where rings are set into the material to hang it
//! (<https://en.wikipedia.org/wiki/Grommet>), spread evenly at most a set spacing apart. The
//! banner is the selected art's bounds, or each artboard.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::{Point, Rect, shapes};

use super::super::super::edit::selected_roots;
use super::super::super::paint::swatch_solid;
use super::super::super::pathops::length_given;
use super::super::super::{CommandSpec, bad, cmd, has_doc, switch_or, text_param};
use super::weed::outside_clips;
use crate::{Result, Session};

/// Banner Hems and Grommets' command.
pub const BANNER: &str = "addon.cutting.banner";
/// The sides of a banner, in the order the grommets go round it.
const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
/// The largest hem, inset or spacing, in points.
const MAX_DISTANCE: f64 = 100_000.0;
/// The most grommets one run adds.
const MAX_GROMMETS: usize = 100_000;
/// Two grommet centers closer than this are one grommet (the corner that two sides share).
const SAME: f64 = 1e-6;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.cutting.banner",
        "Banner Hems and Grommets…",
        ["Object", "Plug-ins", "Cutting Tools"],
        None,
        "{hem?: pt 0..100000 (72: from each edge to its hem line; 0: no hem lines), grommets?: bool (true), spacing?: pt more than 0 and at most 100000 (1728: the most the grommets on a side are apart), grommetInset?: pt 0..100000 (36: from each edge to the grommets' centers), diameter?: pt 0.01..1000 (27: each grommet mark's circle), top?, right?, bottom?, left?: bool (true: the sides that get a hem and grommets), weight?: pt 0.01..1000 (0.3), swatch?: a color swatch the marks are stroked in (\"[Registration]\"), allArtboards?: bool (default: true when nothing is selected)} mark the hems and grommets of a banner, the selected art's bounds or each artboard: a hem line `hem` inside each chosen edge, running the edge's whole length, and grommet marks (a circle with a cross at its center) `grommetInset` inside each chosen edge, one at each end of the side and the rest spread evenly between them, as few as keep them at most `spacing` apart. One group named Banner Marks per banner, stroked with no fill, on top of the current layer, selected, as one undo step → {id: the first group, ids, hems (hem lines), grommets (grommet marks)}",
        has_doc,
        banner
    )]
}

/// The distance `p` gives for `key` (0..[`MAX_DISTANCE`]), or `default`.
fn distance(p: &Value, key: &str, default: f64) -> Result<f64> {
    match length_given(p, key, BANNER)? {
        None => Ok(default),
        Some(v) if (0.0..=MAX_DISTANCE).contains(&v) => Ok(v),
        Some(_) => Err(bad(BANNER, format!("`{key}` must be from 0 to {MAX_DISTANCE} pt"))),
    }
}

/// The length `p` gives for `key` (0.01..1000), or `default`.
fn size(p: &Value, key: &str, default: f64) -> Result<f64> {
    match length_given(p, key, BANNER)? {
        None => Ok(default),
        Some(v) if (0.01..=1000.0).contains(&v) => Ok(v),
        Some(_) => Err(bad(BANNER, format!("`{key}` must be from 0.01 to 1000 pt"))),
    }
}

/// Which sides are on, in [`SIDES`] order.
type Sides = [bool; 4];

/// Whether marks `depth` inside a banner's near and far edges, `len` apart, fit: they stay inside
/// the banner, and when both edges are chosen the two sets stay apart.
fn fits(depth: f64, len: f64, near: bool, far: bool) -> bool {
    match (near, far) {
        (true, true) => 2.0 * depth < len,
        (true, false) | (false, true) => depth < len,
        (false, false) => true,
    }
}

/// The hem lines of banner `r`: one `hem` inside each chosen edge, from one end of the edge to the
/// other.
fn hem_lines(r: Rect, hem: f64, sides: Sides) -> Vec<(Point, Point)> {
    if hem <= 0.0 {
        return vec![];
    }
    let lines = [
        (Point::new(r.x0, r.y0 + hem), Point::new(r.x1, r.y0 + hem)),
        (Point::new(r.x1 - hem, r.y0), Point::new(r.x1 - hem, r.y1)),
        (Point::new(r.x0, r.y1 - hem), Point::new(r.x1, r.y1 - hem)),
        (Point::new(r.x0 + hem, r.y0), Point::new(r.x0 + hem, r.y1)),
    ];
    lines.into_iter().zip(sides).filter(|(_, on)| *on).map(|(l, _)| l).collect()
}

/// The grommet centers of banner `r`: on each chosen side, `inset` inside the edge, one at each end
/// (`inset` from the edges across it) and the rest spread evenly between them, as few as keep them
/// at most `spacing` apart. They go round the banner clockwise from the top left; a corner that two
/// chosen sides share has one grommet. `None` when there would be more than `limit`.
fn grommet_centers(r: Rect, inset: f64, spacing: f64, sides: Sides, limit: usize) -> Option<Vec<Point>> {
    let (l, t, rt, b) = (r.x0 + inset, r.y0 + inset, r.x1 - inset, r.y1 - inset);
    // Each side from its start to its end, clockwise.
    let runs = [
        (Point::new(l, t), Point::new(rt, t)),
        (Point::new(rt, t), Point::new(rt, b)),
        (Point::new(rt, b), Point::new(l, b)),
        (Point::new(l, b), Point::new(l, t)),
    ];
    let mut out: Vec<Point> = vec![];
    for ((a, z), on) in runs.into_iter().zip(sides) {
        if !on {
            continue;
        }
        let gaps = ((z - a).hypot() / spacing - 1e-9).ceil().max(1.0);
        if !gaps.is_finite() || gaps > limit as f64 {
            return None;
        }
        let gaps = gaps as usize;
        for i in 0..=gaps {
            let p = a.lerp(z, i as f64 / gaps as f64);
            // A side starts where the one before it ends, and the last one ends where the first
            // starts.
            let shared = |q: Option<&Point>| q.is_some_and(|q| (*q - p).hypot() < SAME);
            if !shared(out.last()) && !shared(out.first()) {
                out.push(p);
            }
            if out.len() > limit {
                return None;
            }
        }
    }
    Some(out)
}

fn banner(s: &mut Session, p: &Value) -> Result<Value> {
    let hem = distance(p, "hem", 72.0)?;
    let inset = distance(p, "grommetInset", 36.0)?;
    let spacing = match length_given(p, "spacing", BANNER)? {
        None => 1728.0,
        Some(v) if v > 0.0 && v <= MAX_DISTANCE => v,
        Some(_) => return Err(bad(BANNER, format!("`spacing` must be more than 0 and at most {MAX_DISTANCE} pt"))),
    };
    let diameter = size(p, "diameter", 27.0)?;
    let weight = size(p, "weight", 0.3)?;
    let with_grommets = switch_or(p, "grommets", true, BANNER)?;
    let mut sides = [true; 4];
    for (on, key) in sides.iter_mut().zip(SIDES) {
        *on = switch_or(p, key, true, BANNER)?;
    }
    if !sides.contains(&true) {
        return Err(bad(BANNER, "turn on at least one side"));
    }
    if hem == 0.0 && !with_grommets {
        return Err(bad(BANNER, "with no hem and no grommets there is nothing to mark"));
    }
    let swatch = text_param(p, "swatch", BANNER)?.unwrap_or(REGISTRATION);
    let stroke = swatch_solid(&s.doc()?.doc, swatch, None).map_err(|e| bad(BANNER, e))?;
    let all = switch_or(p, "allArtboards", s.doc()?.selection.is_empty(), BANNER)?;
    let banners: Vec<Rect> = if all {
        s.doc()?.doc.artboards.iter().map(|a| a.rect).collect()
    } else {
        let roots = selected_roots(s)?;
        if roots.is_empty() {
            return Err(bad(BANNER, "select the banner, or mark every artboard"));
        }
        s.doc()?.doc.bounds_of(&roots, false).into_iter().collect()
    };
    if banners.is_empty() {
        return Err(bad(BANNER, "there is no banner to mark"));
    }
    let [top, right, bottom, left] = sides;
    let mut marks = vec![];
    let mut total = 0;
    for r in &banners {
        let (w, h) = (r.width(), r.height());
        if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
            return Err(bad(BANNER, "the banner has no width or height"));
        }
        // A hem line must stay inside the banner and apart from the hem line on the far side.
        if hem > 0.0 && !(fits(hem, h, top, bottom) && fits(hem, w, left, right)) {
            return Err(bad(BANNER, "the hem is too deep for the banner"));
        }
        let centers = if with_grommets {
            // The grommets on a side run from `inset` past one end to `inset` before the other,
            // stay inside the banner and stay apart from the grommets on the far side.
            let runs_fit = (!(top || bottom) || 2.0 * inset <= w) && (!(left || right) || 2.0 * inset <= h);
            if !(runs_fit && fits(inset, h, top, bottom) && fits(inset, w, left, right)) {
                return Err(bad(BANNER, "the grommet inset is too large for the banner"));
            }
            grommet_centers(*r, inset, spacing, sides, MAX_GROMMETS - total)
                .ok_or_else(|| bad(BANNER, format!("the spacing makes more than {MAX_GROMMETS} grommets")))?
        } else {
            vec![]
        };
        total += centers.len();
        marks.push((hem_lines(*r, hem, sides), centers));
    }
    let parent = outside_clips(&s.doc()?.doc, s.doc()?.target_parent()?);
    let ap = Appearance::basic(Paint::None, stroke, weight);
    let half = diameter / 2.0;
    let (ids, hems) = s.edit("Banner Hems and Grommets", |d, sel| {
        let (mut ids, mut hems) = (vec![], 0);
        for (lines, centers) in &marks {
            let mut children = vec![];
            for (a, b) in lines {
                let mut n = Node::path(d.alloc_id(), shapes::line(*a, *b), ap.clone());
                n.name = Some("Hem".into());
                children.push(Arc::new(n));
            }
            hems += lines.len();
            for c in centers {
                let ring = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(c.x - half, c.y - half, c.x + half, c.y + half)), ap.clone());
                let across = Node::path(d.alloc_id(), shapes::line(Point::new(c.x - half, c.y), Point::new(c.x + half, c.y)), ap.clone());
                let down = Node::path(d.alloc_id(), shapes::line(Point::new(c.x, c.y - half), Point::new(c.x, c.y + half)), ap.clone());
                let mut g = Node::group(d.alloc_id(), vec![Arc::new(ring), Arc::new(across), Arc::new(down)]);
                g.name = Some("Grommet".into());
                children.push(Arc::new(g));
            }
            let mut g = Node::group(d.alloc_id(), children);
            g.name = Some("Banner Marks".into());
            ids.push(g.id);
            d.insert(parent, usize::MAX, g)?;
        }
        sel.set(ids.iter().copied());
        Ok((ids, hems))
    })?;
    let first = ids.first().map(|i| i.0);
    Ok(json!({ "id": first, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "hems": hems, "grommets": total }))
}
