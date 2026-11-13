//! HP-GL export: the visible art over a region as pen moves, for pen plotters and cutters.
//!
//! Every visible path is drawn once along its own line: its fills and strokes are not painted, so
//! a filled shape is its closed outline (what a cutter cuts) and a stroke is its center line. Each
//! color is a pen: by default pen 1 is the first color the plot draws in, pen 2 the next new one,
//! and so on; with a pen table each path takes the pen whose color is nearest its own. The plots
//! of one export ([`export_regions`]) share the numbering. A path's color is its topmost visible
//! stroke's, else its topmost visible fill's (one at 0% opacity does not count); a gradient is the
//! average of its stops and a pattern its first visible solid fill (gray without one). Curves are
//! flattened to straight moves within a tolerance. Type is drawn as its glyph outlines, a gradient
//! mesh as its outline in the average of its shown points' colors, clipping masks cut what they clip,
//! and the art is cut at the region's edges. Hidden and non-printing layers, hidden objects,
//! objects at 0% opacity, template layers, guides and placed images are left out.
//!
//! Coordinates are plotter units (1016 to the inch, y up) from the region's bottom-left corner or
//! its center, rounded to whole units. Art past ±8,388,607 units (2²³ − 1, the smallest range
//! the HP-GL/2 reference asks a device to take for encoded polylines) from the origin is cut off.
//! An HP-GL plot with a coordinate outside −32,768 to 32,767 (16-bit numbers, the range older
//! devices read) gets a warning; HP-GL/2's encoded polylines take the wider range.
//! The pens are drawn in turn (pen 1's paths, then pen 2's), each in paint order or, with
//! `sort`, each next path the one that starts nearest where the pen stopped. With `inside_first`,
//! a pen draws a path only after every path of its own that lies inside it (a letter's counter
//! before the letter, a hole before the part).
//!
//! HP-GL writes `IN;`, then for each pen `PU;SP<n>;` and per path `PUx,y;PDx,y,…;`, then `PU;SP0;`.
//! HP-GL/2 also writes the palette (`NP` and a `PC` color for each pen) and draws each pen's paths
//! as one encoded polyline (`PE`, base 64).

mod clip;
mod nest;

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::sync::Arc;

use kurbo::PathEl;
use vectorcraft_color::Paint;
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeKind, StrokeLayer, SymbolBoxes};
use vectorcraft_geom::{Affine, BezPath, FillRule, Point, Rect};

use crate::MAX_NEST;
use clip::Region;

/// Plotter units per point.
const PLU_PER_PT: f64 = 1016.0 / 72.0;
/// The largest coordinate written, plotter units.
pub const MAX_COORD: i64 = (1 << 23) - 1;
/// The coordinates an HP-GL plot keeps to without a warning: 16-bit numbers, plotter units.
const CLASSIC_RANGE: std::ops::RangeInclusive<i64> = -32_768..=32_767;
/// The curve tolerance a path is flattened to by default, points.
pub const DEFAULT_TOLERANCE: f64 = 0.1;
/// The smallest and largest curve tolerance, points.
pub const TOLERANCE_RANGE: std::ops::RangeInclusive<f64> = 0.01..=10.0;
/// The most pens: a pen table's length, and the colors that get a pen of their own.
pub const MAX_PENS: usize = 255;
/// The most points the plots of one export hold.
const MAX_POINTS: usize = if cfg!(test) { 100_000 } else { 10_000_000 };
/// The most work sorting paths and finding which lie inside others may take, over the whole plot:
/// one for each point or box compared.
const MAX_SORT_WORK: u64 = if cfg!(test) { 100_000 } else { 500_000_000 };
/// The most edge tests clipping masks may take, over the whole plot.
const MAX_CLIP_TESTS: u64 = if cfg!(test) { 1_000_000 } else { 200_000_000 };
/// The most points the clipping masks around an object may hold, together.
const MAX_MASK_POINTS: usize = if cfg!(test) { 10_000 } else { 10_000_000 };
/// The most objects live art (blends, envelopes, repeats) may make for a plot, together, as one
/// evaluation of live art may make ([`vectorcraft_doc::live::MAX_GENERATED`]).
const MAX_GENERATED: u64 = if cfg!(test) { 1_000 } else { vectorcraft_doc::live::MAX_GENERATED as u64 };

const TEXT_OUTLINES: &str = "type is plotted as its glyph outlines";
const INLINE_GRAPHICS: &str = "inline graphics in type are left out";
const IMAGES: &str = "placed images are left out: a pen draws only lines";
const MASKS: &str = "opacity masks are left out";
const RASTER_FX: &str = "raster effects (shadows, glows, blurs, feathering) are left out";
const STROKE_LOOK: &str = "stroke widths, dashes, brushes and arrowheads are left out: each path is drawn once along its own line";
const TOO_MANY_POINTS: &str = "the export is too large: the art past its first 10 million points was left out";
const CLIP_WORK: &str = "clipping masks are too complex to apply in full: the art past that point is drawn whole";
const MASK_POINTS: &str = "clipping masks past 10 million points together are left out: the art they clip is drawn unclipped by them";
const OUT_OF_RANGE: &str = "art past 8,388,607 plotter units (about 210 m) from the origin is cut off";
const PAST_CLASSIC_RANGE: &str = "the plot has coordinates past 32,767 plotter units (about 82 cm) from the origin, which older HP-GL devices may not read: HP-GL/2 takes a wider range";
const MANY_COLORS: &str = "the art has more than 255 colors: the colors past those draw with the pen of the nearest color";
const UNSORTED: &str = "the plot has too many paths to sort in full: the rest are drawn in paint order";
const UNSORTED_INSIDE: &str = "the plot has too many paths to sort in full: the rest are drawn in paint order, each after the paths inside it";
const UNNESTED: &str = "the plot has too many paths to find in full which lie inside others: from the pen where that stopped, a path may be drawn before the paths inside it";
const TOO_DEEP: &str = "symbols, blends, envelopes and repeats nested more than 8 deep were left out";
const LIVE_LEFT_OUT: &str = "blends, envelopes and repeats make too many objects: the art past 4 million objects was left out";
const SYMBOLS_LEFT_OUT: &str = "symbols placed inside symbols hold too many objects: the instances past that were left out";

/// The language written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Version {
    /// `IN`, `SP`, `PU` and `PD` only.
    #[default]
    Hpgl,
    /// HP-GL/2: pen colors (`NP`, `PC`) and encoded polylines (`PE`).
    Hpgl2,
}

impl Version {
    pub const ALL: [Self; 2] = [Self::Hpgl, Self::Hpgl2];

    /// The `hpglVersion` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::Hpgl => "hpgl",
            Self::Hpgl2 => "hpgl2",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hpgl => "HP-GL",
            Self::Hpgl2 => "HP-GL/2",
        }
    }

    /// A version by id (`hpgl`, `hpgl2`; `hp-gl` and `hp-gl/2` also read), any case.
    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL.into_iter().find(|v| s.eq_ignore_ascii_case(v.id()) || s.eq_ignore_ascii_case(v.label()))
    }
}

/// Where the plot's origin (0, 0) lies on the region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    BottomLeft,
    Center,
}

impl Origin {
    pub const ALL: [Self; 2] = [Self::BottomLeft, Self::Center];

    /// The `origin` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::BottomLeft => "bottomLeft",
            Self::Center => "center",
        }
    }

    /// An origin by id, any case.
    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| s.trim().eq_ignore_ascii_case(o.id()))
    }
}

/// The HP-GL export settings.
#[derive(Clone, Debug)]
pub struct HpglOptions {
    pub version: Version,
    /// How far a flattened curve may lie from the curve, points ([`TOLERANCE_RANGE`]; a value
    /// outside it is an error).
    pub tolerance: f64,
    /// The pen table: pen 1's color first, up to [`MAX_PENS`] (more is an error). Empty: a pen for
    /// each color the art draws in.
    pub pens: Vec<[u8; 3]>,
    pub origin: Origin,
    /// Draw each pen's paths nearest first, instead of in paint order.
    pub sort: bool,
    /// Draw a path only after the pen's paths that lie inside it.
    pub inside_first: bool,
    /// The area plotted (document space): its bottom-left corner or center is the origin, and the
    /// art is cut at its edges.
    pub region: Rect,
}

impl Default for HpglOptions {
    fn default() -> Self {
        Self {
            version: Version::default(),
            tolerance: DEFAULT_TOLERANCE,
            pens: vec![],
            origin: Origin::default(),
            sort: false,
            inside_first: false,
            region: Rect::ZERO,
        }
    }
}

/// A written plot.
#[derive(Clone, Debug, Default)]
pub struct HpglOutput {
    pub bytes: Vec<u8>,
    /// The pens' colors, pen 1's first.
    pub pens: Vec<[u8; 3]>,
    /// What was approximated or left out.
    pub warnings: Vec<String>,
    /// Notes on the options: sorting the paths or drawing the inner paths first was applied only
    /// in part.
    pub setting_notes: Vec<String>,
}

/// Write the visible art of `doc` over `opts.region` as an HP-GL plot. Live geometry effects are
/// applied first.
pub fn export(doc: &Document, opts: &HpglOptions) -> Result<HpglOutput, String> {
    Ok(export_regions(doc, opts, &[opts.region])?.pop().unwrap_or_default())
}

/// Write the visible art of `doc` over each of `regions` as an HP-GL plot, one per region, as
/// [`export`] does with `opts.region`. The plots share one pen numbering, so a color is the same
/// pen in each, and one point budget.
pub fn export_regions(doc: &Document, opts: &HpglOptions, regions: &[Rect]) -> Result<Vec<HpglOutput>, String> {
    let regions: Vec<Rect> = regions.iter().map(|r| r.abs()).collect();
    if regions.iter().any(|r| ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) || r.width() <= 0.0 || r.height() <= 0.0) {
        return Err("the export region is empty".into());
    }
    if !TOLERANCE_RANGE.contains(&opts.tolerance) {
        return Err("the curve tolerance must be from 0.01 to 10 points".into());
    }
    if opts.pens.len() > MAX_PENS {
        return Err("the pen table holds more than 255 pens".into());
    }
    let tolerance = opts.tolerance;
    // Layers the plot leaves out go first: baking can wrap one in art that would be drawn (its
    // crop marks).
    let shown = plotted(&doc.layers, false).map(|layers| {
        let mut d = doc.clone();
        d.layers = layers;
        d
    });
    let doc = shown.as_ref().unwrap_or(doc);
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    // Each plot's lines in plotter units, with the points a move doesn't change dropped, and its
    // warnings.
    let mut plots: Vec<(Lines, Vec<String>)> = Vec::with_capacity(regions.len());
    let mut points = 0;
    for r in regions {
        // Document space to plotter units, y up.
        let o = match opts.origin {
            Origin::BottomLeft => Point::new(r.x0, r.y1),
            Origin::Center => r.center(),
        };
        let span = MAX_COORD as f64 / PLU_PER_PT;
        let range = Rect::new(o.x - span, o.y - span, o.x + span, o.y + span);
        let mut walk = Walk::new(doc, r.intersect(range), tolerance);
        walk.points = points;
        // Only a region past the range can have art the range cuts off.
        if r.intersect(range) != r {
            walk.range = Some((range, r));
            walk.asked = Region::new(vec![corners(r)], FillRule::NonZero);
        }
        for l in &doc.layers {
            walk.node(l, false);
        }
        points = walk.points;
        let Walk { runs, warnings, .. } = walk;
        let to_plu = |p: Point| {
            let x = ((p.x - o.x) * PLU_PER_PT).round().clamp(-(MAX_COORD as f64), MAX_COORD as f64) as i64;
            let y = ((o.y - p.y) * PLU_PER_PT).round().clamp(-(MAX_COORD as f64), MAX_COORD as f64) as i64;
            (x, y)
        };
        let lines = runs
            .into_iter()
            .filter_map(|run| {
                let mut pts: Vec<(i64, i64)> = Vec::with_capacity(run.points.len());
                for p in run.points {
                    let q = to_plu(p);
                    if pts.last() != Some(&q) {
                        pts.push(q);
                    }
                }
                // A line shorter than a plotter unit draws nothing and takes no pen.
                (pts.len() >= 2).then_some((run.rgb, pts))
            })
            .collect();
        plots.push((lines, warnings));
    }
    let mut pen_warnings = vec![];
    let colors: Vec<[u8; 3]> = plots.iter().flat_map(|(lines, _)| lines.iter().map(|(c, _)| *c)).collect();
    let (pens, of_line) = assign_pens(&colors, &opts.pens, &mut pen_warnings);
    let mut of_line = of_line.into_iter();
    let mut out = Vec::with_capacity(plots.len());
    for (lines, mut warnings) in plots {
        let mut by_pen: Vec<Vec<Vec<(i64, i64)>>> = vec![vec![]; pens.len()];
        for ((_, pts), pen) in lines.into_iter().zip(of_line.by_ref()) {
            if let Some(paths) = by_pen.get_mut(pen) {
                paths.push(pts);
            }
        }
        if !by_pen.iter().all(Vec::is_empty) {
            pen_warnings.iter().for_each(|w| warn(&mut warnings, w));
        }
        let mut setting_notes = vec![];
        if opts.sort || opts.inside_first {
            let (mut at, mut work) = ((0, 0), MAX_SORT_WORK);
            for paths in &mut by_pen {
                at = reorder(paths, at, &mut work, opts.sort, opts.inside_first, &mut setting_notes);
            }
        }
        if opts.version == Version::Hpgl && by_pen.iter().flatten().flatten().any(|(x, y)| !CLASSIC_RANGE.contains(x) || !CLASSIC_RANGE.contains(y)) {
            warn(&mut warnings, PAST_CLASSIC_RANGE);
        }
        let bytes = match opts.version {
            Version::Hpgl => plain(&by_pen),
            Version::Hpgl2 => encoded(&by_pen, &pens),
        };
        out.push(HpglOutput { bytes, pens: pens.clone(), warnings, setting_notes });
    }
    Ok(out)
}

/// A plot's lines in plotter units, each with its color.
type Lines = Vec<([u8; 3], Vec<(i64, i64)>)>;

fn warn(warnings: &mut Vec<String>, w: &str) {
    if !warnings.iter().any(|x| x == w) {
        warnings.push(w.to_string());
    }
}

/// The corners of `r`, as a clipping ring.
fn corners(r: Rect) -> Vec<Point> {
    vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}

/// Does `r` hold `p`, its edges included?
fn holds(r: Rect, p: Point) -> bool {
    p.x >= r.x0 && p.x <= r.x1 && p.y >= r.y0 && p.y <= r.y1
}

/// The pens and each line's pen (0-based), from the lines' `colors`: with a table, the table and
/// the nearest of its colors; else each new color a pen of its own, up to [`MAX_PENS`], and the
/// nearest after that.
fn assign_pens(colors: &[[u8; 3]], table: &[[u8; 3]], warnings: &mut Vec<String>) -> (Vec<[u8; 3]>, Vec<usize>) {
    let mut pens: Vec<[u8; 3]> = table.iter().take(MAX_PENS).copied().collect();
    let fixed = !pens.is_empty();
    let mut of_line = Vec::with_capacity(colors.len());
    for &rgb in colors {
        let pen = match pens.iter().position(|c| *c == rgb) {
            Some(i) => i,
            None if !fixed && pens.len() < MAX_PENS => {
                pens.push(rgb);
                pens.len() - 1
            }
            None => {
                if !fixed {
                    warn(warnings, MANY_COLORS);
                }
                nearest(&pens, rgb)
            }
        };
        of_line.push(pen);
    }
    (pens, of_line)
}

/// The index of the color in `pens` nearest `c` (the first of equally near ones).
fn nearest(pens: &[[u8; 3]], c: [u8; 3]) -> usize {
    let d = |p: &[u8; 3]| (0..3).map(|i| (i32::from(p[i]) - i32::from(c[i])).pow(2)).sum::<i32>();
    pens.iter().enumerate().min_by_key(|(_, p)| d(p)).map_or(0, |(i, _)| i)
}

fn dist2(a: (i64, i64), b: (i64, i64)) -> i128 {
    let (dx, dy) = (i128::from(a.0) - i128::from(b.0), i128::from(a.1) - i128::from(b.1));
    dx * dx + dy * dy
}

/// The squared distance from `p` to box `b` (`[x0, y0, x1, y1]`), 0 inside it.
fn box_dist2(p: (i64, i64), b: [i64; 4]) -> i128 {
    let d = |v: i64, lo: i64, hi: i64| i128::from(lo.saturating_sub(v).max(v.saturating_sub(hi)).max(0));
    let (dx, dy) = (d(p.0, b[0], b[2]), d(p.1, b[1], b[3]));
    dx * dx + dy * dy
}

/// `paths` (one pen's) reordered from `at`. With `nearest`, each next path is the one nearest
/// where the pen stopped, by its start or, for an open path (then drawn backwards), its end, or
/// for a closed path any of its points (where it then starts); without, the first in paint order.
/// With `inside_first`, a path is drawn only once every path lying inside it ([`nest::outer`]) is.
/// Once `work` is spent, the paths left keep paint order, each after the paths inside it with
/// `inside_first`, with a note in `notes`. Returns where the pen stops.
fn reorder(
    paths: &mut Vec<Vec<(i64, i64)>>,
    mut at: (i64, i64),
    work: &mut u64,
    nearest: bool,
    inside_first: bool,
    notes: &mut Vec<String>,
) -> (i64, i64) {
    let boxes: Vec<[i64; 4]> = paths.iter().map(|p| nest::bounds(p)).collect();
    // The paths each path lies inside, and how many paths inside each are still to draw.
    let outer = inside_first.then(|| nest::outer(paths, &boxes, work)).flatten();
    if inside_first && outer.is_none() {
        warn(notes, UNNESTED);
    }
    let mut waiting = vec![0usize; paths.len()];
    for o in outer.iter().flatten().flatten() {
        if let Some(w) = waiting.get_mut(*o) {
            *w += 1;
        }
    }
    if !nearest {
        if let Some(outer) = &outer {
            let all = std::mem::take(paths).into_iter().enumerate().collect();
            inner_first(all, outer, &mut waiting, paths);
        }
        return at;
    }
    // Each path with its index and box.
    let mut left: Vec<_> = std::mem::take(paths).into_iter().zip(boxes).enumerate().map(|(i, (p, b))| (i, p, b)).collect();
    while !left.is_empty() {
        // (index, reversed, the closed path's point to start at, distance)
        let mut best: (usize, bool, Option<usize>, i128) = (usize::MAX, false, None, i128::MAX);
        for (i, (j, p, b)) in left.iter().enumerate() {
            let (Some(&s), Some(&e)) = (p.first(), p.last()) else { continue };
            *work = work.saturating_sub(1);
            if waiting.get(*j).is_some_and(|w| *w > 0) {
                continue;
            }
            if s == e && p.len() > 2 {
                if box_dist2(at, *b) >= best.3 {
                    continue;
                }
                *work = work.saturating_sub(p.len() as u64);
                if let Some((k, d)) = p.iter().enumerate().map(|(k, q)| (k, dist2(at, *q))).min_by_key(|(_, d)| *d)
                    && d < best.3
                {
                    best = (i, false, Some(k), d);
                }
                continue;
            }
            let (ds, de) = (dist2(at, s), dist2(at, e));
            if ds < best.3 {
                best = (i, false, None, ds);
            }
            if de < best.3 {
                best = (i, true, None, de);
            }
        }
        // Spent, or no path free to draw (which only a fault could cause): the rest in paint
        // order, each after the paths inside it.
        if *work == 0 || best.0 >= left.len() {
            match &outer {
                Some(outer) => {
                    warn(notes, UNSORTED_INSIDE);
                    inner_first(left.into_iter().map(|(j, p, _)| (j, p)).collect(), outer, &mut waiting, paths);
                }
                None => {
                    warn(notes, UNSORTED);
                    paths.extend(left.into_iter().map(|(_, p, _)| p));
                }
            }
            return at;
        }
        let (j, mut p, _) = left.remove(best.0);
        for o in outer.as_ref().and_then(|outer| outer.get(j)).into_iter().flatten() {
            if let Some(w) = waiting.get_mut(*o) {
                *w = w.saturating_sub(1);
            }
        }
        if best.1 {
            p.reverse();
        } else if let Some(k) = best.2 {
            // A closed path starts at its point nearest the pen.
            p.pop();
            let k = k.min(p.len().saturating_sub(1));
            p.rotate_left(k);
            if let Some(&s) = p.first() {
                p.push(s);
            }
        }
        if let Some(&e) = p.last() {
            at = e;
        }
        paths.push(p);
    }
    at
}

/// Appends `left` (each path with its index among the pen's paths) to `paths` in paint order,
/// each after the paths lying inside it: `outer` lists the paths each lies inside, and `waiting`
/// counts, for each, the paths inside it still to draw. [`nest::outer`] puts a path only inside
/// larger ones, so every path comes free; a path left waiting by a fault is drawn last.
fn inner_first(left: Vec<(usize, Vec<(i64, i64)>)>, outer: &[Vec<usize>], waiting: &mut [usize], paths: &mut Vec<Vec<(i64, i64)>>) {
    let mut held: Vec<Option<Vec<(i64, i64)>>> = (0..outer.len()).map(|_| None).collect();
    let mut free = BinaryHeap::new();
    for (j, p) in left {
        if let Some(h) = held.get_mut(j) {
            if waiting.get(j).is_none_or(|w| *w == 0) {
                free.push(Reverse(j));
            }
            *h = Some(p);
        } else {
            paths.push(p);
        }
    }
    while let Some(Reverse(j)) = free.pop() {
        let Some(p) = held.get_mut(j).and_then(Option::take) else { continue };
        paths.push(p);
        for &o in outer.get(j).into_iter().flatten() {
            if let Some(w) = waiting.get_mut(o) {
                *w = w.saturating_sub(1);
                if *w == 0 {
                    free.push(Reverse(o));
                }
            }
        }
    }
    paths.extend(held.into_iter().flatten());
}

/// HP-GL: `IN;`, each pen's paths as `PU` and `PD`, `PU;SP0;`.
fn plain(by_pen: &[Vec<Vec<(i64, i64)>>]) -> Vec<u8> {
    let mut s = String::from("IN;");
    for (i, paths) in by_pen.iter().enumerate().filter(|(_, p)| !p.is_empty()) {
        s.push_str(&format!("PU;SP{};", i + 1));
        for path in paths {
            let Some(((x, y), rest)) = path.split_first() else { continue };
            s.push_str(&format!("PU{x},{y};PD"));
            for (k, (x, y)) in rest.iter().enumerate() {
                if k > 0 {
                    s.push(',');
                }
                s.push_str(&format!("{x},{y}"));
            }
            s.push(';');
        }
    }
    s.push_str("PU;SP0;");
    s.into_bytes()
}

/// HP-GL/2: `IN;`, the palette, each pen's paths as one `PE`, `PU;SP0;`.
fn encoded(by_pen: &[Vec<Vec<(i64, i64)>>], pens: &[[u8; 3]]) -> Vec<u8> {
    let mut out = b"IN;".to_vec();
    if !pens.is_empty() {
        // Pens 1 to n on a plotter, and on a raster device (whose pens are 0 to NP − 1) too.
        let np = (pens.len() + 1).next_power_of_two().max(2);
        out.extend(format!("NP{np};").bytes());
        for (i, [r, g, b]) in pens.iter().enumerate() {
            out.extend(format!("PC{},{r},{g},{b};", i + 1).bytes());
        }
    }
    let mut at: Option<(i64, i64)> = None;
    for (i, paths) in by_pen.iter().enumerate().filter(|(_, p)| !p.is_empty()) {
        out.extend(format!("PU;SP{};PE", i + 1).bytes());
        for path in paths {
            for (k, &p) in path.iter().enumerate() {
                if k == 0 {
                    out.push(b'<');
                }
                // The first move of the plot is absolute, and so is one longer than the range a
                // device takes for each number; every other is relative.
                match at.map(|a| (p.0 - a.0, p.1 - a.1)).filter(|d| d.0.abs() <= MAX_COORD && d.1.abs() <= MAX_COORD) {
                    None => {
                        out.push(b'=');
                        pe_number(&mut out, p.0);
                        pe_number(&mut out, p.1);
                    }
                    Some((dx, dy)) => {
                        pe_number(&mut out, dx);
                        pe_number(&mut out, dy);
                    }
                }
                at = Some(p);
            }
        }
        out.push(b';');
    }
    out.extend(b"PU;SP0;");
    out
}

/// `v` as a base-64 PE number: the sign in the low bit, the low digits from 63 up and the last
/// digit from 191 up.
fn pe_number(out: &mut Vec<u8>, v: i64) {
    let mut n = (v.unsigned_abs() << 1) | u64::from(v < 0);
    while n >= 64 {
        out.push(63 + (n % 64) as u8);
        n /= 64;
    }
    out.push(191 + n as u8);
}

/// One line the pen draws, in document space, and its color.
struct Run {
    rgb: [u8; 3],
    points: Vec<Point>,
}

/// The document walk: each visible path becomes runs, clipped, in paint order.
struct Walk<'a> {
    doc: &'a Document,
    symbols: SymbolBoxes<'a>,
    /// The region the art is cut at.
    region: Rect,
    edge: Option<Region>,
    /// The plotter's range and the region asked for, when that reaches past the range.
    range: Option<(Rect, Rect)>,
    /// The region asked for, to cut a line at before testing it against the range.
    asked: Option<Region>,
    tolerance: f64,
    /// The clipping masks around the current object, outermost first.
    clips: Vec<Region>,
    /// The points those masks hold ([`MAX_MASK_POINTS`]).
    mask_points: usize,
    runs: Vec<Run>,
    points: usize,
    clip_tests: u64,
    warnings: Vec<String>,
    nest: u32,
    /// Objects the symbol instances written held, together ([`vectorcraft_doc::MAX_SYMBOL_NODES`]).
    symbol_nodes: u64,
    /// Objects the live art written made, together ([`MAX_GENERATED`]).
    generated: u64,
    /// A copy of the document without its layers, to bake symbol art in ([`Walk::baked`]).
    scratch: Option<Document>,
    /// The symbols whose art holds geometry effects ([`Walk::baked`]), or instances of such
    /// symbols.
    effect_symbols: HashSet<String>,
}

impl<'a> Walk<'a> {
    fn new(doc: &'a Document, region: Rect, tolerance: f64) -> Self {
        Self {
            doc,
            symbols: doc.symbol_boxes().quick(),
            region,
            edge: Region::new(vec![corners(region)], FillRule::NonZero),
            range: None,
            asked: None,
            tolerance,
            clips: vec![],
            mask_points: 0,
            runs: vec![],
            points: 0,
            clip_tests: 0,
            warnings: vec![],
            nest: 0,
            symbol_nodes: 0,
            generated: 0,
            scratch: None,
            effect_symbols: effect_symbols(doc),
        }
    }

    fn warn(&mut self, w: &str) {
        warn(&mut self.warnings, w);
    }

    fn full(&self) -> bool {
        self.points >= MAX_POINTS
    }

    fn node(&mut self, n: &Node, force: bool) {
        if n.is_ruler_guide() || n.is_template() {
            return;
        }
        if self.full() {
            self.warn(TOO_MANY_POINTS);
            return;
        }
        if !force && !n.visible {
            return;
        }
        // Art at no opacity shows nothing, and nor does anything inside it.
        if n.opacity <= 0.0 {
            return;
        }
        // Bounds don't measure the effects in symbol art, which apply only once an instance is
        // placed: an instance of such art, and what holds one, is not skipped by its bounds.
        let unmeasured = !self.effect_symbols.is_empty() && holds_instance_of(n, &self.effect_symbols);
        // The box round all the object can paint, a blend's steps on its spine included.
        if !n.is_layer() && !unmeasured {
            match n.reach_bounds_with(&self.symbols) {
                Some(b) if !overlaps(b, self.region) => {
                    // Art over the asked-for region but past the plotter's range is cut off.
                    if self.range.is_some_and(|(_, whole)| overlaps(b, whole)) {
                        self.warn(OUT_OF_RANGE);
                    }
                    return;
                }
                None if !n.is_container() => return,
                _ => {}
            }
        }
        if n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn(MASKS);
        }
        let raster = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id));
        if raster(&n.appearance.effects) || n.appearance.items.iter().any(|i| raster(i.effects())) {
            self.warn(RASTER_FX);
        }
        match &n.kind {
            // Non-printing layers aren't plotted.
            NodeKind::Layer { children, printable, clip, .. } => {
                if *printable {
                    self.children(children, *clip);
                }
            }
            NodeKind::Group { children, clip } => self.children(children, *clip),
            NodeKind::Path { path, guide, .. } => {
                if !*guide {
                    self.shape(n, &path.to_bezpath());
                }
            }
            NodeKind::Compound { children, .. } => {
                let mut bp = BezPath::new();
                for p in children.iter().filter_map(|c| c.path_data()) {
                    bp.extend(p.to_bezpath());
                }
                self.shape(n, &bp);
            }
            NodeKind::Text(t) => {
                if t.runs.iter().any(|r| r.inline.is_some()) {
                    self.warn(INLINE_GRAPHICS);
                }
                if let Some(o) = vectorcraft_effects::outline_text(n) {
                    self.warn(TEXT_OUTLINES);
                    self.node(&o, true);
                }
            }
            NodeKind::Image(_) => self.warn(IMAGES),
            NodeKind::SymbolInstance { symbol, xf } => self.symbol(n, symbol, *xf),
            // A gradient mesh is its outline, in the average of its shown points' colors: its
            // patches are shading, not lines. A mesh whose every point is at 0% opacity shows
            // nothing.
            NodeKind::Mesh(m) => {
                let shown: Vec<[u8; 3]> = m.points.iter().filter(|p| p.opacity > 0.0).map(|p| rgb8(&p.color)).collect();
                if !shown.is_empty() {
                    self.draw(average(shown.into_iter()), &m.outline().to_bezpath(), true);
                }
            }
            // Live blends, envelopes and repeats are drawn as their evaluated art, a level at a
            // time, so a gradient mesh a blend makes is its outline too.
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) | NodeKind::PlacedDocument(_) | NodeKind::Intertwine { .. } => {
                if self.nest >= MAX_NEST {
                    self.warn(TOO_DEEP);
                    return;
                }
                // Once the budget is spent, nothing more is evaluated.
                if self.generated >= MAX_GENERATED {
                    self.warn(LIVE_LEFT_OUT);
                    return;
                }
                let art = vectorcraft_effects::expand_live(Some(self.doc), n);
                let count = art.iter().map(|c| c.count() as u64).sum::<u64>();
                if self.generated.saturating_add(count) > MAX_GENERATED {
                    self.generated = MAX_GENERATED;
                    self.warn(LIVE_LEFT_OUT);
                    return;
                }
                self.generated += count;
                self.nest += 1;
                art.iter().for_each(|c| self.node(c, false));
                self.nest -= 1;
            }
            NodeKind::RulerGuide { .. } => {}
        }
    }

    /// A symbol instance's art, placed (clip groups included), within the budget for the export.
    fn symbol(&mut self, n: &Node, symbol: &str, xf: Affine) {
        let doc = self.doc;
        if self.symbol_nodes >= vectorcraft_doc::MAX_SYMBOL_NODES {
            self.warn(SYMBOLS_LEFT_OUT);
            return;
        }
        let Some(sym) = doc.symbols.iter().find(|s| s.name == symbol) else { return };
        if self.nest >= MAX_NEST {
            self.warn(TOO_DEEP);
            return;
        }
        // The instance's own changes and its stain, as the canvas draws it.
        let art = vectorcraft_brush::instance_art(&sym.art, n);
        let count = art.count_with_masks() as u64;
        let most = doc.symbol_slice_map(symbol, xf).map_or(count, |m| m.most_placed(&art, doc));
        let placed = vectorcraft_doc::charge_symbol_art(&mut self.symbol_nodes, most, || {
            let placed = doc.place_symbol_art(symbol, &art, xf);
            let n = placed.as_ref().map_or(0, |a| a.count_with_masks() as u64);
            (placed, n)
        });
        match placed {
            Some(Some(art)) => {
                let art = self.baked(art);
                self.nest += 1;
                self.node(&art, true);
                self.nest -= 1;
            }
            Some(None) => {}
            None => {
                self.symbol_nodes = vectorcraft_doc::MAX_SYMBOL_NODES;
                self.warn(SYMBOLS_LEFT_OUT);
            }
        }
    }

    /// Placed symbol art with its geometry effects applied: the document's bake reaches the
    /// layers, not the symbols' art.
    fn baked(&mut self, art: Node) -> Node {
        if !vectorcraft_effects::needs_bake(&art) {
            return art;
        }
        // Guides and layers left out go first, as for the document's layers.
        let art = Arc::new(art);
        let art = match plotted(std::slice::from_ref(&art), false) {
            Some(mut kept) => match kept.pop() {
                Some(a) => a,
                None => return Node::group(vectorcraft_doc::NodeId(0), vec![]),
            },
            None => art,
        };
        let art = Arc::unwrap_or_clone(art);
        let doc = self.doc;
        let scratch = self.scratch.get_or_insert_with(|| {
            let mut d = doc.clone();
            d.layers.clear();
            d
        });
        scratch.layers = vec![Arc::new(art)];
        let baked = vectorcraft_effects::bake_document(scratch).and_then(|b| b.layers.into_iter().next());
        let art = std::mem::take(&mut scratch.layers).into_iter().next();
        baked.or(art).map(Arc::unwrap_or_clone).unwrap_or_else(|| Node::group(vectorcraft_doc::NodeId(0), vec![]))
    }

    /// A container's children. A clipping container's first child cuts the rest; it is drawn
    /// itself, once and whole, when it paints: where its stroke paints (over the clipped art) when
    /// it has one, else where its fill paints (below it). Nothing to clip by hides the clipped art,
    /// as on the canvas.
    fn children(&mut self, children: &[Arc<Node>], clip: bool) {
        let Some((first, rest)) = children.split_first().filter(|_| clip) else {
            children.iter().for_each(|c| self.node(c, false));
            return;
        };
        let mut paint = first.clip_paint();
        // A stroke at 0% opacity isn't drawn, so the clipping path is drawn in its fill.
        let type_stroke = |s: &Node| matches!(&s.kind, NodeKind::Text(t) if t.runs.iter().any(|r| r.style.has_stroke()));
        paint.stroke = paint.stroke.filter(|s| top_stroke(s).is_some() || type_stroke(s));
        if paint.stroke.is_none()
            && let Some(fill) = &paint.fill
        {
            self.node(fill, false);
        }
        match vectorcraft_effects::clip_outline(first).map(|(bp, rule)| (self.rings(&bp), rule)) {
            Some((Some(rings), rule)) => {
                let held: usize = rings.iter().map(Vec::len).sum();
                if let Some(region) = Region::new(rings, rule) {
                    self.mask_points += held;
                    self.clips.push(region);
                    rest.iter().for_each(|c| self.node(c, false));
                    self.clips.pop();
                    self.mask_points -= held;
                }
            }
            // A mask past the budget isn't built: the art it would clip is drawn unclipped by it.
            Some((None, _)) => {
                self.warn(MASK_POINTS);
                rest.iter().for_each(|c| self.node(c, false));
            }
            None => {}
        }
        // Drawn whole where its stroke paints: a fill closes its outline, and type's characters
        // without a stroke are drawn too.
        if paint.stroke.is_some() {
            self.node(first, false);
        }
    }

    /// `bp` flattened into closed rings for a clipping mask; `None` when they would take the
    /// masks around it past [`MAX_MASK_POINTS`].
    fn rings(&self, bp: &BezPath) -> Option<Vec<Vec<Point>>> {
        let left = MAX_MASK_POINTS.saturating_sub(self.mask_points);
        let (mut rings, whole) = self.flatten_within(bp, true, left);
        if !whole || rings.iter().map(Vec::len).sum::<usize>() > left {
            return None;
        }
        for r in &mut rings {
            r.pop();
        }
        Some(rings)
    }

    /// A path drawn in its color: the topmost stroke's that paints, else the topmost fill's that
    /// paints (visible, with a paint and some opacity). A filled path's every subpath is closed,
    /// as its fill closes it, also when its stroke gives the color.
    fn shape(&mut self, n: &Node, bp: &BezPath) {
        if bp.elements().is_empty() {
            return;
        }
        let fill = top_fill(n);
        let paint = match (top_stroke(n), fill) {
            (Some(s), _) => {
                if s.dash.as_ref().is_some_and(|d| d.is_dashed())
                    || s.brush.is_some()
                    || s.profile.is_some()
                    || s.start_arrow.is_some()
                    || s.end_arrow.is_some()
                {
                    self.warn(STROKE_LOOK);
                }
                &s.paint
            }
            (None, Some(f)) => f,
            (None, None) => return,
        };
        let rgb = self.flat(paint);
        self.draw(rgb, bp, fill.is_some());
    }

    /// `bp` drawn in `rgb`: flattened (with `close` every subpath closed), then cut and added.
    fn draw(&mut self, rgb: [u8; 3], bp: &BezPath, close: bool) {
        // Bounded by the plot's point budget; each curve by its own segment cap.
        let budget = MAX_POINTS.saturating_sub(self.points).saturating_mul(4).max(4);
        let (lines, whole) = self.flatten_within(bp, close, budget);
        if !whole {
            self.warn(TOO_MANY_POINTS);
        }
        for line in lines {
            self.add(rgb, line);
        }
    }

    /// One color for a paint: a solid color, the average of a gradient's stops, or a pattern's
    /// first solid fill (gray without one).
    fn flat(&self, p: &Paint) -> [u8; 3] {
        match p {
            Paint::None => [0; 3],
            Paint::Solid { color, .. } => rgb8(color),
            Paint::Gradient(g) => average(g.gradient.stops.iter().map(|s| rgb8(&s.color))),
            Paint::Pattern { pattern, .. } => {
                self.doc.pattern(pattern).and_then(|def| def.art.iter().find_map(|a| first_solid(a))).map_or([128; 3], |c| rgb8(&c))
            }
        }
    }

    /// `bp` as polylines (each subpath one), within the tolerance, stopping once about `budget`
    /// moves are made; with `close` every subpath ends at its start, else those that close. Also
    /// whether every element of `bp` was flattened.
    fn flatten_within(&self, bp: &BezPath, close: bool, budget: usize) -> (Vec<Vec<Point>>, bool) {
        let mut out: Vec<Vec<Point>> = vec![];
        let mut cur: Vec<Point> = vec![];
        let end = |cur: &mut Vec<Point>, out: &mut Vec<Vec<Point>>, closing: bool| {
            if (closing || close)
                && let Some(&s) = cur.first()
                && cur.last() != Some(&s)
            {
                cur.push(s);
            }
            out.push(std::mem::take(cur));
        };
        let (made, taken) = (std::cell::Cell::new(0usize), std::cell::Cell::new(0usize));
        let elements = bp.iter().take_while(|_| made.get() < budget).inspect(|_| taken.set(taken.get() + 1));
        vectorcraft_geom::flatten(elements, self.tolerance, |el| {
            made.set(made.get() + 1);
            match el {
                PathEl::MoveTo(p) => {
                    end(&mut cur, &mut out, false);
                    cur.push(p);
                }
                PathEl::LineTo(p) => {
                    if cur.last() != Some(&p) {
                        cur.push(p);
                    }
                }
                PathEl::ClosePath => {
                    let s = cur.first().copied();
                    end(&mut cur, &mut out, true);
                    // A subpath that goes on after its close starts where it closed.
                    if let Some(s) = s {
                        cur.push(s);
                    }
                }
                _ => {}
            }
        });
        let whole = taken.get() == bp.elements().len();
        // A subpath the budget cut short stays open: a move back to its start isn't on its outline.
        if whole {
            end(&mut cur, &mut out, false);
        } else {
            out.push(cur);
        }
        out.retain(|l| l.len() >= 2);
        (out, whole)
    }

    /// A polyline drawn in `rgb`, cut by the region and the clipping masks around it.
    fn add(&mut self, rgb: [u8; 3], line: Vec<Point>) {
        // Only the part over the region asked for counts: the rest is cut off anyway.
        if let (Some((range, _)), Some(asked)) = (self.range, &self.asked)
            && !self.warnings.iter().any(|w| w == OUT_OF_RANGE)
        {
            let mut unlimited = u64::MAX;
            if asked.cut(&line, &mut unlimited).unwrap_or_default().iter().flatten().any(|p| !holds(range, *p)) {
                self.warn(OUT_OF_RANGE);
            }
        }
        // The region has four edges, so its cut costs a few tests a point and needs no limit.
        let mut unlimited = u64::MAX;
        let mut pieces = self.edge.as_ref().and_then(|edge| edge.cut(&line, &mut unlimited)).unwrap_or_default();
        for clip in &self.clips {
            let mut left = MAX_CLIP_TESTS.saturating_sub(self.clip_tests);
            let cut: Option<Vec<Vec<Vec<Point>>>> = pieces.iter().map(|p| clip.cut(p, &mut left)).collect();
            self.clip_tests = MAX_CLIP_TESTS.saturating_sub(left);
            match cut {
                Some(cut) => pieces = cut.into_iter().flatten().collect(),
                None => {
                    // Spent: this art keeps the parts this mask and the ones inside it would cut,
                    // and later art that reaches a mask isn't clipped either.
                    self.clip_tests = MAX_CLIP_TESTS;
                    warn(&mut self.warnings, CLIP_WORK);
                    break;
                }
            }
        }
        for mut points in pieces {
            let left = MAX_POINTS.saturating_sub(self.points);
            if points.len() > left {
                self.warn(TOO_MANY_POINTS);
                points.truncate(left);
            }
            if points.len() < 2 {
                return;
            }
            self.points += points.len();
            self.runs.push(Run { rgb, points });
        }
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// `nodes` (the children of a clipping container when `clipping`) without the layers and
/// sublayers a plot leaves out (hidden, non-printing, template or at 0% opacity) and without
/// guides, in layers and groups at any depth, so that baking can't wrap one in art that is drawn
/// (its Crop Marks); `None` when nothing changes. A guide that is a clipping path keeps its place
/// without its effects.
fn plotted(nodes: &[Arc<Node>], clipping: bool) -> Option<Vec<Arc<Node>>> {
    let mut changed = false;
    let mut out = Vec::with_capacity(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        match &n.kind {
            NodeKind::Layer { printable, template, .. } if !(n.visible && n.opacity > 0.0 && *printable && !*template) => {
                changed = true;
                continue;
            }
            NodeKind::Path { guide: true, .. } => {
                changed = true;
                if clipping && i == 0 {
                    let mut g = (**n).clone();
                    g.appearance.effects.clear();
                    out.push(Arc::new(g));
                }
                continue;
            }
            _ => {}
        }
        let inner = match &n.kind {
            NodeKind::Layer { children, clip, .. } | NodeKind::Group { children, clip } => plotted(children, *clip),
            _ => None,
        };
        match inner {
            Some(children) => {
                changed = true;
                let mut c = (**n).clone();
                if let Some(ch) = c.children_mut() {
                    *ch = children;
                }
                out.push(Arc::new(c));
            }
            None => out.push(n.clone()),
        }
    }
    changed.then_some(out)
}

/// The symbols of `doc` whose art holds geometry effects, and those whose art holds instances of
/// them, as deep as symbols are drawn ([`MAX_NEST`]).
fn effect_symbols(doc: &Document) -> HashSet<String> {
    let mut set: HashSet<String> = doc.symbols.iter().filter(|s| vectorcraft_effects::needs_bake(&s.art)).map(|s| s.name.clone()).collect();
    for _ in 0..MAX_NEST {
        if set.is_empty() {
            break;
        }
        let more: Vec<String> =
            doc.symbols.iter().filter(|s| !set.contains(&s.name) && holds_instance_of(&s.art, &set)).map(|s| s.name.clone()).collect();
        if more.is_empty() {
            break;
        }
        set.extend(more);
    }
    set
}

/// Is `n`, or anything in it, an instance of one of `symbols`?
fn holds_instance_of(n: &Node, symbols: &HashSet<String>) -> bool {
    match &n.kind {
        NodeKind::SymbolInstance { symbol, .. } => symbols.contains(symbol),
        _ => n.children().into_iter().flatten().any(|c| holds_instance_of(c, symbols)),
    }
}

fn rgb8(c: &vectorcraft_color::Color) -> [u8; 3] {
    let [r, g, b, _] = c.to_rgba8(1.0);
    [r, g, b]
}

/// The average of `colors` (black without any).
fn average(colors: impl Iterator<Item = [u8; 3]>) -> [u8; 3] {
    let (sum, n) = colors.fold(([0.0; 3], 0.0), |(a, n), c| ([0, 1, 2].map(|i| a[i] + f64::from(c[i])), n + 1.0));
    sum.map(|v| (v / f64::max(n, 1.0)).round().clamp(0.0, 255.0) as u8)
}

/// The first visible solid fill color in `n` (a pattern tile's art), hidden objects and objects
/// or fills at 0% opacity left out.
fn first_solid(n: &Node) -> Option<vectorcraft_color::Color> {
    if !n.visible || n.opacity <= 0.0 {
        return None;
    }
    let own = n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Fill(f) if f.visible && f.opacity > 0.0 => f.paint.color(),
        _ => None,
    });
    own.or_else(|| n.children().into_iter().flatten().find_map(|c| first_solid(c)))
}

/// The topmost stroke of `n` that paints: visible, with a paint, a weight and some opacity.
fn top_stroke(n: &Node) -> Option<&StrokeLayer> {
    n.appearance.items.iter().rev().find_map(|i| match i {
        AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() && s.width > 0.0 && s.opacity > 0.0 => Some(s),
        _ => None,
    })
}

/// The paint of the topmost fill of `n` that paints: visible, with a paint and some opacity.
fn top_fill(n: &Node) -> Option<&Paint> {
    n.appearance.items.iter().rev().find_map(|i| match i {
        AppearanceItem::Fill(f) if f.visible && !f.paint.is_none() && f.opacity > 0.0 => Some(&f.paint),
        _ => None,
    })
}

#[cfg(test)]
mod tests;
