//! Offset Path and Outline Stroke.

use kurbo::{BezPath, PathEl, Stroke, StrokeOpts};
use vectorcraft_geom::{FillRule, PathData, SubPath};

pub use kurbo::{Cap, Join};

use linesweeper::topology::{ContourIdx, Contours};

use crate::boolean::{Arrangement, DEFAULT_PRECISION, Tidy, all_contours_to_path, contours_to_path, fill_bezpath, normalize_bez};
use crate::edit::remove_redundant_points;

/// Flattening/fit tolerance handed to kurbo's stroker.
const STROKE_TOL: f64 = 1e-3;

/// Drop collinear and coincident spine vertices before stroking (#3446). Uses `tol` only for
/// redundancy; does not simplify or refit the spine.
fn compact_stroke_spine(bp: &BezPath, tol: f64) -> BezPath {
    if bp.elements().is_empty() {
        return BezPath::new();
    }
    let tol = if tol.is_finite() && tol > 0.0 { tol } else { STROKE_TOL };
    remove_redundant_points(&PathData::from_bezpath(bp), tol).to_bezpath()
}

fn stroke_bez(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> BezPath {
    let spine = compact_stroke_spine(bp, STROKE_TOL);
    let style = Stroke::new(width).with_join(join).with_caps(cap).with_miter_limit(miter_limit.max(1.0));
    close_all(&kurbo::stroke(spine.iter(), &style, &StrokeOpts::default(), STROKE_TOL))
}

/// Like [`stroke_bez`] but without [`compact_stroke_spine`] (regression baseline in unit tests).
#[cfg(test)]
fn stroke_bez_without_spine_compact(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> BezPath {
    let style = Stroke::new(width).with_join(join).with_caps(cap).with_miter_limit(miter_limit.max(1.0));
    close_all(&kurbo::stroke(bp.iter(), &style, &StrokeOpts::default(), STROKE_TOL))
}

/// kurbo's stroker may leave the outline of an open path without a final `ClosePath`.
fn close_all(bp: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in bp.iter() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(el);
    }
    if open {
        out.close_path();
    }
    out
}

/// Object → Path → Outline Stroke: the filled area painted by stroking `path` with `width`.
/// Overlaps produced by the stroker are removed, so the result is a clean compound path.
pub fn outline_stroke(path: &PathData, width: f64, cap: Cap, join: Join, miter_limit: f64) -> PathData {
    if width <= 0.0 || !width.is_finite() || path.is_empty() {
        return PathData::default();
    }
    stroke_region(&[stroke_bez(&path.to_bezpath(), width, cap, join, miter_limit)])
}

/// The area covered by stroke outlines (each filled with the non-zero rule, open subpaths
/// closed) as one clean path: overlaps removed and the stroker's pieces refitted.
pub fn stroke_region(outlines: &[BezPath]) -> PathData {
    let tidy = Tidy::free(DEFAULT_PRECISION);
    let parts: Vec<BezPath> = outlines.iter().map(close_all).filter(|o| !o.elements().is_empty()).collect();
    if let [one] = &parts[..] {
        return normalize_bez(one, FillRule::NonZero).map(|c| all_contours_to_path(&c, &tidy)).unwrap_or_default();
    }
    // Each outline is normalised on its own first: a hollow arrowhead may wind against the line
    // it overlaps, and the raw windings would cancel there under the non-zero rule.
    let mut all = BezPath::new();
    for p in &parts {
        if let Ok(c) = normalize_bez(p, FillRule::NonZero) {
            for ct in c.contours() {
                all.extend(ct.path.iter());
            }
        }
    }
    if all.elements().is_empty() {
        return PathData::default();
    }
    normalize_bez(&all, FillRule::NonZero).map(|c| all_contours_to_path(&c, &tidy)).unwrap_or_default()
}

/// Object → Path → Offset Path. Positive `delta` grows the filled area, negative insets it.
/// Closed subpaths are treated as a non-zero filled region; open subpaths are outlined with a
/// stroke of width `2|delta|` (butt caps). An open subpath whose last anchor sits on its first is
/// offset as a closed one.
pub fn offset_path(path: &PathData, delta: f64, join: Join, miter_limit: f64) -> PathData {
    if !delta.is_finite() || path.is_empty() {
        return PathData::default();
    }
    let subpaths: Vec<SubPath> = path.subpaths.iter().map(close_if_ends_meet).collect();
    let closed = PathData::new(subpaths.iter().filter(|s| s.closed && s.anchors.len() > 1).cloned().collect());
    let open: Vec<SubPath> = subpaths.into_iter().filter(|s| !s.closed && s.anchors.len() > 1).collect();
    let d = delta.abs();
    let fill = fill_bezpath(&closed);
    let ring = if d > 0.0 { stroke_bez(&closed.to_bezpath(), 2.0 * d, Cap::Butt, join, miter_limit) } else { BezPath::new() };
    let open_bp = if d > 0.0 && !open.is_empty() {
        stroke_bez(&PathData::new(open).to_bezpath(), 2.0 * d, Cap::Butt, join, miter_limit)
    } else {
        BezPath::new()
    };
    let Ok(arr) = Arrangement::new(vec![(fill, FillRule::NonZero), (ring, FillRule::NonZero), (open_bp, FillRule::NonZero)]) else {
        return PathData::default();
    };
    let c = if delta >= 0.0 { arr.contours(|m| m[0] || m[1] || m[2]) } else { arr.contours(|m| m[0] && !m[1]) };
    // When `d` exceeds a curvature radius the stroker's inner offset inverts and its loops cancel
    // winding, leaving faces uncovered that are really within `d` of the path. Every face of the
    // true offset is bounded by arrangement edges, so a contour is spurious iff a point deep inside
    // it lies closer than `d`: holes when growing, islands when insetting.
    let src = path.to_bezpath();
    let drop_outer = delta < 0.0;
    let spurious: Vec<bool> =
        c.contours().map(|k| k.outer == drop_outer && deep_point(&k.path).is_some_and(|p| dist_to(&src, p) < d * (1.0 - 1e-4) - 1e-6)).collect();
    if !spurious.contains(&true) {
        return all_contours_to_path(&c, &Tidy::free(DEFAULT_PRECISION));
    }
    let keep = (0..spurious.len()).filter(|&i| !has_marked_ancestor(&c, i, &spurious)).map(ContourIdx);
    contours_to_path(&c, keep, &Tidy::free(DEFAULT_PRECISION))
}

/// An open subpath of three or more anchors whose last anchor lies within 1e-6 of its first (the
/// test Object → Path → Join applies to one path) as a closed subpath: the last anchor is merged
/// into the first, which takes its in-handle. Any other subpath is returned unchanged.
fn close_if_ends_meet(sp: &SubPath) -> SubPath {
    let mut sp = sp.clone();
    if !sp.closed
        && sp.anchors.len() > 2
        && sp.anchors.first().zip(sp.anchors.last()).is_some_and(|(f, l)| f.p.distance(l.p) < 1e-6)
        && let Some(last) = sp.anchors.pop()
        && let Some(first) = sp.anchors.first_mut()
    {
        first.h_in = last.h_in;
        sp.closed = true;
    }
    sp
}

fn has_marked_ancestor(c: &Contours, mut i: usize, marked: &[bool]) -> bool {
    loop {
        if marked[i] {
            return true;
        }
        match c[ContourIdx(i)].parent {
            Some(ContourIdx(p)) => i = p,
            None => return false,
        }
    }
}

/// A point well inside a closed contour: the midpoint of the widest span on the horizontal line
/// through its vertical centre.
fn deep_point(bp: &BezPath) -> Option<kurbo::Point> {
    use kurbo::Shape;
    let r = bp.bounding_box();
    if !(r.width() > 0.0 && r.height() > 0.0) {
        return None;
    }
    let mut best: Option<(f64, kurbo::Point)> = None;
    for f in [0.5, 0.3, 0.7] {
        let y = r.y0 + r.height() * f;
        let mut xs = vec![];
        let mut prev: Option<kurbo::Point> = None;
        let mut start = kurbo::Point::ZERO;
        let edge = |a: kurbo::Point, b: kurbo::Point, xs: &mut Vec<f64>| {
            if (a.y <= y) != (b.y <= y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        };
        kurbo::flatten(bp.iter(), 0.01 * r.width().min(r.height()).max(1e-6), |el| match el {
            PathEl::MoveTo(p) => {
                start = p;
                prev = Some(p);
            }
            PathEl::LineTo(p) => {
                if let Some(a) = prev {
                    edge(a, p, &mut xs);
                }
                prev = Some(p);
            }
            PathEl::ClosePath => {
                if let Some(a) = prev {
                    edge(a, start, &mut xs);
                }
                prev = Some(start);
            }
            _ => {}
        });
        xs.sort_by(f64::total_cmp);
        for w in xs.as_chunks::<2>().0 {
            let span = w[1] - w[0];
            if best.is_none_or(|(b, _)| span > b) {
                best = Some((span, kurbo::Point::new((w[0] + w[1]) / 2.0, y)));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn dist_to(bp: &BezPath, p: kurbo::Point) -> f64 {
    use kurbo::ParamCurveNearest;
    bp.segments().map(|s| s.nearest(p, 1e-6).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

#[cfg(test)]
mod tests {
    use kurbo::Shape;

    use super::*;

    #[test]
    fn stroke_region_unites_outlines_whatever_their_winding() {
        // A bar, and a square ring wound the other way that the bar runs into.
        let bar = kurbo::Rect::new(0.0, 0.0, 20.0, 4.0).to_path(0.1);
        let mut ring = kurbo::Rect::new(10.0, -8.0, 26.0, 12.0).to_path(0.1).reverse_subpaths();
        ring.extend(kurbo::Rect::new(14.0, -4.0, 22.0, 8.0).to_path(0.1).iter());
        let region = stroke_region(&[bar, ring]);
        let area = crate::area(&region, FillRule::NonZero);
        // Ring 16·20 − 8·12 = 224, plus the bar outside it (10·4) and inside its hole (6·4).
        assert!((area - (224.0 + 40.0 + 24.0)).abs() < 1e-6, "{area}");
    }

    #[test]
    fn miter_offset_finishes_where_the_sweep_used_to_loop() {
        // Issue #1028: with linesweeper 0.4 the sweep over this miter offset never finished and
        // its memory grew without bound, freezing the app. A synthetic outline (found by search)
        // with the same failure as the reported glyph; the exact coordinates matter.
        const OUTLINE: &str = "M74.359137297823,82.83738194552835 L88.097432485874,103.04979803227248 \
            L67.06936731196308,105.95779886823588 L53.29382873396223,98.53943925153727 \
            L29.684713319694094,94.17996020306241 L41.65480824807079,75.639672165115 \
            Q47.491034578751446,52.12555333014565 46.157302824282816,55.67193079127421 \
            Q69.08365151786553,53.9608681393331 68.37499772324333,51.17242466680129 \
            Q74.4136309339628,75.13284223919301 71.3309887459222,73.69128869058812 Z";
        let path = PathData::from_bezpath(&BezPath::from_svg(OUTLINE).unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let input = path.clone();
        std::thread::spawn(move || tx.send(offset_path(&input, 1.8350459854022054, Join::Miter, 4.0)));
        let Ok(out) = rx.recv_timeout(std::time::Duration::from_secs(30)) else {
            // the stuck sweep keeps allocating on its thread: end the test binary now
            eprintln!("Offset Path didn't finish in 30 s (issue #1028)");
            std::process::exit(101);
        };
        let (before, after) = (crate::area(&path, FillRule::NonZero), crate::area(&out, FillRule::NonZero));
        assert_eq!(out.subpaths.len(), 1);
        assert!(after > before, "{before} → {after}");
    }

    fn outline_max_deviation(a: &BezPath, b: &BezPath) -> f64 {
        use kurbo::ParamCurve;
        use vectorcraft_geom::Point;
        let pa = PathData::from_bezpath(a);
        let pb = PathData::from_bezpath(b);
        let mut worst: f64 = 0.0;
        for (src, dst) in [(a, &pb), (b, &pa)] {
            for seg in src.segments() {
                for i in 0..=12 {
                    let p = seg.eval(i as f64 / 12.0);
                    let pt = Point::new(p.x, p.y);
                    worst = worst.max(dst.nearest(pt).unwrap().4);
                }
            }
        }
        worst
    }

    #[test]
    fn compact_stroke_spine_drops_colinear_points_without_moving_curved_outlines() {
        let width = 10.0;
        let cap = Cap::Round;
        let join = Join::Round;
        let miter = 4.0;
        // Dense colinear spine: compact outline should match the uncompacted one closely.
        let dense: Vec<vectorcraft_geom::Point> = (0..=40).map(|i| vectorcraft_geom::Point::new(i as f64 * 2.5, 50.0)).collect();
        let dense_path = PathData::single(SubPath::polyline(&dense, false));
        let dense_bez = dense_path.to_bezpath();
        let compact = stroke_bez(&dense_bez, width, cap, join, miter);
        let raw = stroke_bez_without_spine_compact(&dense_bez, width, cap, join, miter);
        assert!(outline_max_deviation(&compact, &raw) <= 1e-3, "colinear compaction must not refit the stroke outline");
        let sparse =
            PathData::single(SubPath::polyline(&[vectorcraft_geom::Point::new(0.0, 50.0), vectorcraft_geom::Point::new(100.0, 50.0)], false));
        let sparse_outline = outline_stroke(&sparse, width, cap, join, miter);
        let compact_region = stroke_region(&[compact]);
        assert!(
            outline_max_deviation(&compact_region.to_bezpath(), &sparse_outline.to_bezpath()) <= 1e-3,
            "compacted colinear spine should outline like the endpoints alone"
        );
        // Sampled arc: only redundant vertices are removed; outline must track the raw stroke.
        let arc: Vec<vectorcraft_geom::Point> = (0..=32)
            .map(|i| {
                let t = i as f64 / 32.0 * std::f64::consts::PI;
                vectorcraft_geom::Point::new(50.0 + 50.0 * t.cos(), 50.0 + 8.0 * t.sin())
            })
            .collect();
        let arc_bez = PathData::single(SubPath::polyline(&arc, false)).to_bezpath();
        let arc_compact = stroke_bez(&arc_bez, width, cap, join, miter);
        let arc_raw = stroke_bez_without_spine_compact(&arc_bez, width, cap, join, miter);
        assert!(outline_max_deviation(&arc_compact, &arc_raw) <= 1e-3, "curved spine compaction must not refit the stroke outline");
    }
}
