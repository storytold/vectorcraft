//! Path Effects (Path Precision Tools): live geometry effects that rewrite an object's path with the
//! add-on's path kernels (Effect › Path Effects). Each is evaluated with the other geometry effects
//! ([`crate::apply_geometry`]), so it draws, measures, exports and expands as they do; the geometry
//! is written only by Expand Appearance.
//!
//! Parameters come from documents and scripts, so each is clamped to its documented range, and a
//! path with a coordinate past [`MAX_COORDINATE`] or not finite is left as it is.

use serde_json::{Value, json};
use vectorcraft_doc::Dash;
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_pathops::addons::pathtools::{self as tools, AnchorSpacing, CloseMode, ExtendMode, SMOOTH_RATIO, SegmentKind};
use vectorcraft_pathops::addons::{PT_PER_TOLERANCE, retract_tiny_handles, simplify_keep_shape};

use crate::catalog::{EffectInfo, Lengths};
use crate::stroke;
use crate::util::{flag, num, text};

/// What every Path Effect's id starts with.
pub const PREFIX: &str = "addon.path.";
pub const DASHES: &str = "addon.path.dashes";
pub const ADD_ANCHORS: &str = "addon.path.addAnchors";
pub const EXTEND: &str = "addon.path.extend";
pub const CLOSE: &str = "addon.path.close";
pub const SMOOTH: &str = "addon.path.smooth";
pub const RETRACT: &str = "addon.path.retract";
pub const REVERSE: &str = "addon.path.reverse";
pub const SIMPLIFY: &str = "addon.path.simplify";
pub const SPLIT: &str = "addon.path.split";

const MENU: &[&str] = &["Effect", "Path Effects"];

/// The dash and gap parameters of [`DASHES`], pair by pair.
const DASH_PAIRS: [(&str, &str); 5] = [("dash1", "gap1"), ("dash2", "gap2"), ("dash3", "gap3"), ("dash4", "gap4"), ("dash5", "gap5")];
/// Longest dash or gap, and the farthest offset (pt).
const MAX_DASH: f64 = 10_000.0;
/// Longest stretch [`EXTEND`] adds or takes off an end, in points and in percent of the path.
const MAX_EXTEND: f64 = 100_000.0;
const MAX_EXTEND_PERCENT: f64 = 1000.0;
/// Most anchors [`ADD_ANCHORS`] adds to one segment.
const MAX_PER_SEGMENT: f64 = 1000.0;
/// Closest and farthest apart [`ADD_ANCHORS`]' anchors are (pt).
const DISTANCE_RANGE: (f64, f64) = (0.01, 100_000.0);
/// The range of [`SMOOTH`]'s ratio.
const RATIO_RANGE: (f64, f64) = (0.01, 0.5);
/// Longest handle length [`RETRACT`] takes (pt).
const MAX_HANDLE: f64 = 100_000.0;
/// Highest tolerance of [`SIMPLIFY`].
const MAX_TOLERANCE: f64 = 200.0;
// shortcut: simplifying measures every anchor on every draw, so a larger path draws as it is; lift
// this if the kernel gets an incremental pass.
/// Most anchors [`SIMPLIFY`] takes in one path.
const MAX_SIMPLIFY_ANCHORS: usize = 50_000;
/// A path with a coordinate past this, or one that isn't finite, is left as it is: lengths measured
/// on it would overflow.
const MAX_COORDINATE: f64 = 1e12;

/// The effects' catalogue entries.
pub(crate) fn catalog() -> Vec<EffectInfo> {
    let entry = |id, label, params, defaults, always| EffectInfo {
        id,
        label,
        menu: MENU,
        params,
        defaults,
        raster: false,
        lengths: Lengths { always, absolute: &[] },
    };
    vec![
        entry(
            DASHES,
            "Dashes to Geometry…",
            "{dash1: pt 0..10000 (12), gap1: pt 0..10000 (6), dash2: pt 0..10000 (0), gap2: pt 0..10000 (0), dash3: pt 0..10000 (0), gap3: pt 0..10000 (0), dash4: pt 0..10000 (0), gap4: pt 0..10000 (0), dash5: pt 0..10000 (0), gap5: pt 0..10000 (0), offset: pt -10000..10000 (0; how far into the pattern the path starts; not with alignCorners), alignCorners: bool (false; every run between corners and path ends holds whole periods, stretched or squeezed to fit, with a dash centred on each corner and end)} cuts the path into its dashes, open subpaths along it that the stroke paints as it paints any path. A dash of 0 is a point its stroke's round or projecting cap paints; a pair of two zeros is left out, and the pattern repeats. A pattern of nothing, or of more than 200000 dashes, leaves the path as it is; a dash longer than a closed subpath keeps it closed",
            json!({"dash1": 12.0, "gap1": 6.0, "dash2": 0.0, "gap2": 0.0, "dash3": 0.0, "gap3": 0.0, "dash4": 0.0, "gap4": 0.0, "dash5": 0.0, "gap5": 0.0, "offset": 0.0, "alignCorners": false}),
            &["dash1", "gap1", "dash2", "gap2", "dash3", "gap3", "dash4", "gap4", "dash5", "gap5", "offset"],
        ),
        entry(
            ADD_ANCHORS,
            "Add Anchors…",
            "{target: \"all\"|\"curved\"|\"straight\" (all; the segments that get anchors), spacing: \"count\"|\"distance\" (count), count: 1..1000 (1; anchors on each segment, evenly spaced by length), distance: pt 0.01..100000 (20; an anchor every so far along each segment from its start), maxCount: 1..1000 (100; most anchors added to one segment)} adds anchors without changing the shape; past 2 million anchors in all the path stays as it is",
            json!({"target": "all", "spacing": "count", "count": 1, "distance": 20.0, "maxCount": 100}),
            &["distance"],
        ),
        entry(
            EXTEND,
            "Extend Path…",
            "{startLength: pt -100000..100000 (10), startPercent: % of the path's length -1000..1000 (0), startShape: \"straight\"|\"arc\" (straight), endLength: pt -100000..100000 (10), endPercent: % of the path's length -1000..1000 (0), endShape: \"straight\"|\"arc\" (straight)} lengthens each end of an open subpath by its length and percent together, or shortens it back along itself when they come to less than 0. Straight runs on along the end's direction; arc follows the circle of the end's curvature, at most half way round. Closed subpaths stay as they are, and so does an end that would be shortened to nothing",
            json!({"startLength": 10.0, "startPercent": 0.0, "startShape": "straight", "endLength": 10.0, "endPercent": 0.0, "endShape": "straight"}),
            &["startLength", "endLength"],
        ),
        entry(
            CLOSE,
            "Close Path…",
            "{mode: \"straight\"|\"smooth\" (straight; smooth: the closing segment leaves both ends smoothly)} closes every open subpath of two anchors or more",
            json!({"mode": "straight"}),
            &[],
        ),
        entry(
            SMOOTH,
            "Smooth Anchors…",
            "{ratio: 0.01..0.5 (0.3905; a handle's length as a share of the distance to the nearer neighbour; 0.3905 makes a square's corners a circle)} makes every anchor smooth: a lone handle is mirrored, two are brought into line keeping their lengths, and an anchor without handles gets them at right angles to the bisector of its neighbours",
            json!({"ratio": SMOOTH_RATIO}),
            &[],
        ),
        entry(
            RETRACT,
            "Retract Handles…",
            "{handles: \"all\"|\"shorter\" (all; shorter: only those shorter than length), length: pt 0..100000 (5)} takes the handles back to their anchors; an anchor that loses one becomes a corner",
            json!({"handles": "all", "length": 5.0}),
            &["length"],
        ),
        entry(
            REVERSE,
            "Reverse Direction",
            "{} runs every subpath the other way: the shape stays, and arrowheads, dashes and the effects after it swap ends",
            json!({}),
            &[],
        ),
        entry(
            SIMPLIFY,
            "Simplify Anchors…",
            "{tolerance: 0..200 (10; higher removes more anchors and lets the outline move further, up to tolerance × 0.02 pt), protectCorners: bool (true; anchors where the path turns by more than 30° stay)} removes the anchors a path can do without, refitting the handles beside each; open ends stay; a path of more than 50000 anchors stays as it is",
            json!({"tolerance": 10.0, "protectCorners": true}),
            &[],
        ),
        entry(
            SPLIT,
            "Split at Anchors…",
            "{anchors: \"corner\"|\"smooth\"|\"both\" (both; smooth: where the handles stand in line, corner: the others)} cuts subpaths into open pieces at those anchors; the shape stays, and an open path's ends aren't cuts",
            json!({"anchors": "both"}),
            &[],
        ),
    ]
}

/// Path Effect `id` with parameters `p` on `path`, which keeps to `max` anchors where anchors are
/// added one by one. `path` as it is when `id` is no such effect or the path can't be measured.
pub(crate) fn apply(id: &str, p: &Value, path: &PathData, max: usize) -> PathData {
    if !measurable(path) {
        return path.clone();
    }
    let edit = |change: &dyn Fn(&mut SubPath)| {
        let mut out = path.clone();
        out.subpaths.iter_mut().for_each(change);
        out
    };
    match id {
        DASHES => dashes(path, p),
        ADD_ANCHORS => add_anchors(path, p, max).unwrap_or_else(|| path.clone()),
        EXTEND => PathData::new(path.subpaths.iter().map(|sp| extend(sp, p)).collect()),
        CLOSE => {
            let mode = if text(p, "mode", "straight") == "smooth" { CloseMode::Smooth } else { CloseMode::Straight };
            edit(&|sp| {
                tools::close(sp, mode, SMOOTH_RATIO);
            })
        }
        SMOOTH => {
            let ratio = num(p, "ratio", SMOOTH_RATIO).clamp(RATIO_RANGE.0, RATIO_RANGE.1);
            edit(&|sp| tools::smooth_anchors(sp, ratio))
        }
        RETRACT if text(p, "handles", "all") == "shorter" => {
            let length = num(p, "length", 5.0).clamp(0.0, MAX_HANDLE);
            edit(&|sp| {
                retract_tiny_handles(sp, length);
            })
        }
        RETRACT => edit(&|sp| sp.anchors.iter_mut().for_each(|a| a.retract())),
        REVERSE => edit(&SubPath::reverse),
        SIMPLIFY if path.anchor_count() <= MAX_SIMPLIFY_ANCHORS => {
            let tolerance = num(p, "tolerance", 10.0).clamp(0.0, MAX_TOLERANCE) * PT_PER_TOLERANCE;
            simplify_keep_shape(path, tolerance, flag(p, "protectCorners", true))
        }
        SPLIT => {
            let at = text(p, "anchors", "both");
            let (corners, smooth) = (at != "smooth", at != "corner");
            PathData::new(path.subpaths.iter().flat_map(|sp| tools::split_at_anchors(sp, corners, smooth)).collect())
        }
        _ => path.clone(),
    }
}

/// Is every coordinate of `path` within [`MAX_COORDINATE`]?
fn measurable(path: &PathData) -> bool {
    let near = |q: Point| q.x.abs() <= MAX_COORDINATE && q.y.abs() <= MAX_COORDINATE;
    path.anchors().all(|(_, _, a)| near(a.p) && near(a.h_in) && near(a.h_out))
}

/// [`DASHES`]: the dashes of `path` as open subpaths, those of no length as subpaths that stay on
/// their point; `path` as it is when the pattern draws no dashes.
fn dashes(path: &PathData, p: &Value) -> PathData {
    let length = |key| num(p, key, 0.0).clamp(0.0, MAX_DASH);
    let pairs = DASH_PAIRS.iter().map(|(dash, gap)| (length(dash), length(gap)));
    let pattern = pairs.filter(|(d, g)| *d > 0.0 || *g > 0.0).flat_map(|(d, g)| [d, g]).collect();
    let dash = Dash { pattern, offset: num(p, "offset", 0.0).clamp(-MAX_DASH, MAX_DASH), align_corners: flag(p, "alignCorners", false) };
    let Some(dashed) = stroke::dash(&path.to_bezpath(), &dash) else { return path.clone() };
    let mut out = PathData::from_bezpath(&dashed.path);
    out.subpaths.extend(dashed.dots.iter().map(|d| SubPath::polyline(&[d.at, d.at], false)));
    out
}

/// [`ADD_ANCHORS`] within `max` anchors in all; `None` past them.
fn add_anchors(path: &PathData, p: &Value, max: usize) -> Option<PathData> {
    let which = match text(p, "target", "all") {
        "curved" => SegmentKind::Curved,
        "straight" => SegmentKind::Straight,
        _ => SegmentKind::All,
    };
    let spacing = match text(p, "spacing", "count") {
        "distance" => AnchorSpacing::Distance(num(p, "distance", 20.0).clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1)),
        _ => AnchorSpacing::Count(num(p, "count", 1.0).clamp(1.0, MAX_PER_SEGMENT) as usize),
    };
    let cap = num(p, "maxCount", 100.0).clamp(1.0, MAX_PER_SEGMENT) as usize;
    let mut room = max.saturating_sub(path.anchor_count());
    let mut out = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let added = tools::add_anchors(sp, which, spacing, cap, room)?;
        room = room.saturating_sub(added.anchors.len().saturating_sub(sp.anchors.len()));
        out.push(added);
    }
    Some(PathData::new(out))
}

/// [`EXTEND`] on one subpath, the start then the end; what can't be done stays as it is.
fn extend(sp: &SubPath, p: &Value) -> SubPath {
    let total = sp.anchor_arc_positions().last().copied().unwrap_or(0.0);
    let by = |length, percent| {
        num(p, length, 0.0).clamp(-MAX_EXTEND, MAX_EXTEND) + num(p, percent, 0.0).clamp(-MAX_EXTEND_PERCENT, MAX_EXTEND_PERCENT) / 100.0 * total
    };
    let shape = |key| if text(p, key, "straight") == "arc" { ExtendMode::Arc } else { ExtendMode::Straight };
    let ends = [(true, by("startLength", "startPercent"), shape("startShape")), (false, by("endLength", "endPercent"), shape("endShape"))];
    let mut out = sp.clone();
    for (at_start, by, shape) in ends {
        if by != 0.0
            && let Some(changed) = tools::extend_open(&out, at_start, by, shape)
        {
            out = changed;
        }
    }
    out
}

#[cfg(test)]
mod tests;
