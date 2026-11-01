//! Pathfinder › Trap, shared by the Pathfinder panel's Trap command (`object.pathfinder.trap`) and
//! the live effect Effect › Pathfinder › Trap ([`TRAP`]).
//!
//! Where two colours printed from separate plates meet, a press that registers the plates slightly
//! off leaves a sliver of paper showing between them. A trap closes that gap: a thin extra shape
//! along the edge where the colours meet, lying over the darker colour, painted with the lighter
//! colour and set to overprint, so the lighter colour seems to spread into the darker one (a
//! lighter object on a darker background) or to choke it (a lighter background around a darker
//! object). The options:
//!
//! - **Thickness** (0.01–5000 pt): how far the lighter colour reaches into the darker one across
//!   vertical edges.
//! - **Height/Width** (%): the reach across horizontal edges as a percentage of the vertical one
//!   (100%: the same everywhere), for paper that stretches more one way on press.
//! - **Tint Reduction** (%): the trap paints the lighter colour at this percentage of its tint
//!   (100: at full strength), so two light colours don't print a dark border where they overlap.
//! - **Traps with Process Color**: a trap of a swatch colour (a spot colour) is painted with the
//!   equivalent CMYK process colour instead of the swatch.
//! - **Reverse Traps**: the darker colour spreads into the lighter one instead; not when the
//!   darker colour is a rich black (100% K with other inks), which stays where it is.
//! - **Precision** and **Remove Redundant Points** (the effect only): the tolerance in points the
//!   trap's outline is refitted to, and whether anchors that don't change its shape are dropped.
//!
//! Only objects filled with a solid colour take part; each object's visible part (what the objects
//! above leave of it) is what traps, so a trap follows the edges that actually show. Two objects
//! filled with the same colour need no trap. The lighter colour is the one with the higher CIE
//! lightness (L*); between colours of equal lightness, the one with less ink.
//!
//! Behaviour from the public descriptions only:
//! <https://helpx.adobe.com/illustrator/using/trapping.html> and
//! <https://helpx.adobe.com/illustrator/using/pathfinder-panel-overview.html> (feature names and
//! option ranges; the defaults of 0.25 pt, 100% and 40% are VectorCraft's own choices, the pages
//! give none).

use serde_json::{Value, json};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, FillLayer, Node, NodeId};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_pathops as po;

use crate::util::{flag, num};

/// Effect id of Effect › Pathfinder › Trap.
pub const TRAP: &str = "pathfinder.trap";

/// Undo label and name of the Pathfinder panel's Trap.
pub const TRAP_LABEL: &str = "Trap";

/// Parameter documentation of the effect (the panel command `object.pathfinder.trap` documents the
/// same keys but precision and removeRedundantPoints).
pub const EFFECT_PARAMS_DOC: &str = "{thickness?: pt 0.01..5000 (0.25; how far the lighter colour reaches into the darker one across vertical edges), heightWidth?: % 1..1000 (100; the reach across horizontal edges as a percentage of the vertical one), tintReduction?: % 0..100 (40; the trap paints the lighter colour at this percentage of its tint), trapsWithProcessColor?: bool (false; a swatch (spot) colour's trap is painted with its CMYK process equivalent), reverseTraps?: bool (false; the darker colour spreads into the lighter one, except a rich black), precision?: pt 0.001..10 (0.01; the tolerance the trap's outline is refitted to), removeRedundantPoints?: bool (true)} adds overprinting traps where the members' visible areas of different solid colours meet (groups and layers: live Pathfinder over the members)";

/// The options of a trap; [`TrapOptions::from_params`] reads and bounds them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrapOptions {
    /// Reach across vertical edges, in points (0.01..=5000).
    pub thickness: f64,
    /// Reach across horizontal edges as a fraction of `thickness` (0.01..=10).
    pub height_width: f64,
    /// The fraction of the lighter colour's tint the trap paints with (0..=1).
    pub tint: f32,
    /// Paint traps of swatch colours with their CMYK equivalent.
    pub process: bool,
    /// Spread the darker colour into the lighter one.
    pub reverse: bool,
    /// Refit tolerance of the trap outline, in points (0.001..=10).
    pub precision: f64,
    /// Drop anchors that don't change the trap's shape.
    pub remove_redundant: bool,
}

impl Default for TrapOptions {
    fn default() -> Self {
        TrapOptions {
            thickness: 0.25,
            height_width: 1.0,
            tint: 0.4,
            process: false,
            reverse: false,
            precision: po::DEFAULT_PRECISION,
            remove_redundant: true,
        }
    }
}

impl TrapOptions {
    /// The options in `p` (keys of [`EFFECT_PARAMS_DOC`]): a missing or non-numeric value takes its
    /// default and a value out of range its nearest bound, so a live effect always evaluates.
    pub fn from_params(p: &Value) -> Self {
        let d = TrapOptions::default();
        let (hw_min, hw_max) = po::TRAP_HEIGHT_WIDTH_RANGE;
        TrapOptions {
            thickness: num(p, "thickness", d.thickness).clamp(po::TRAP_MIN_WIDTH, po::TRAP_MAX_WIDTH),
            height_width: (num(p, "heightWidth", d.height_width * 100.0) / 100.0).clamp(hw_min, hw_max),
            tint: (num(p, "tintReduction", f64::from(d.tint) * 100.0) / 100.0).clamp(0.0, 1.0) as f32,
            process: flag(p, "trapsWithProcessColor", d.process),
            reverse: flag(p, "reverseTraps", d.reverse),
            precision: num(p, "precision", d.precision).clamp(0.001, 10.0),
            remove_redundant: flag(p, "removeRedundantPoints", d.remove_redundant),
        }
    }
}

/// The Pathfinder panel Trap dialog's fields, at their defaults.
pub fn command_defaults() -> Value {
    let d = TrapOptions::default();
    json!({
        "thickness": d.thickness,
        "heightWidth": d.height_width * 100.0,
        "tintReduction": (f64::from(d.tint) * 100.0).round(),
        "trapsWithProcessColor": d.process,
        "reverseTraps": d.reverse,
    })
}

/// The Effect › Pathfinder › Trap dialog's fields, at their defaults.
pub fn effect_defaults() -> Value {
    let d = TrapOptions::default();
    let mut v = command_defaults();
    if let Some(o) = v.as_object_mut() {
        o.insert("precision".into(), json!(d.precision));
        o.insert("removeRedundantPoints".into(), json!(d.remove_redundant));
    }
    v
}

/// One object taking part in a trap, back to front: its outline and its fill.
#[derive(Clone, Debug)]
pub struct TrapSource {
    pub path: PathData,
    pub rule: FillRule,
    pub fill: Paint,
}

/// One trap: its outline, its paint (the spreading colour, tinted) and the sources it spreads
/// from and into (indices into the sources given to [`traps`]).
#[derive(Clone, Debug)]
pub struct TrapPiece {
    pub path: PathData,
    pub paint: Paint,
    pub from: usize,
    pub into: usize,
}

/// The solid colour a source traps with: none for no fill, gradients, patterns and Registration.
fn trap_color(p: &Paint) -> Option<Color> {
    if p.is_registration() { None } else { p.color() }
}

fn finite(p: &PathData) -> bool {
    p.subpaths.iter().flat_map(|sp| sp.anchors.iter()).all(|a| a.p.x.is_finite() && a.p.y.is_finite())
}

/// Is `a` lighter than `b`? Higher L*; at equal lightness, less ink.
fn lighter(a: Color, b: Color) -> bool {
    let (la, lb) = (a.to_lab().l, b.to_lab().l);
    if (la - lb).abs() > 1e-3 {
        return la > lb;
    }
    let ink = |c: Color| c.to_cmyk().iter().sum::<f32>();
    ink(a) < ink(b)
}

/// 100% black with other inks under it.
fn rich_black(c: Color) -> bool {
    matches!(c, Color::Cmyk { c, m, y, k } if k >= 0.995 && c + m + y > 0.005)
}

/// Do `a` and `b` print the same (no trap needed)?
fn same_color(a: &Paint, b: &Paint, ca: Color, cb: Color) -> bool {
    let link = |p: &Paint| match p {
        Paint::Solid { swatch, .. } => swatch.clone(),
        _ => None,
    };
    link(a) == link(b) && ca.to_cmyk().iter().zip(cb.to_cmyk()).all(|(x, y)| (x - y).abs() < 1e-4)
}

/// The paint of a trap spreading `p` at tint `t`.
fn trap_paint(p: &Paint, color: Color, o: &TrapOptions) -> Paint {
    // A solid colour shows its swatch at `tint` already, so tinting it again gives the swatch at
    // `tint × t` ([`Color::tinted`] multiplies in every model).
    let shown = color.tinted(o.tint);
    match p {
        Paint::Solid { swatch: Some(_), .. } if o.process => {
            let [c, m, y, k] = shown.to_cmyk();
            Paint::solid(Color::cmyk(c, m, y, k))
        }
        Paint::Solid { swatch: Some(s), tint, .. } => Paint::Solid { color: shown, swatch: Some(s.clone()), tint: tint * o.tint },
        _ => Paint::solid(shown),
    }
}

/// The visible part of each source: the faces of their arrangement that it is the top one of.
/// Sources that paint no fill hide nothing and have none.
fn visible(sources: &[TrapSource]) -> Vec<PathData> {
    let mut shapes = vec![];
    for (i, s) in sources.iter().enumerate() {
        if !s.fill.is_none() && !s.path.is_empty() && finite(&s.path) {
            shapes.push(po::Shape::new(s.path.clone(), s.rule, i as u64));
        }
    }
    let mut faces: Vec<Vec<PathData>> = vec![vec![]; sources.len()];
    for r in po::regions(&shapes) {
        if let Some(face) = r.sources.last().and_then(|&si| shapes.get(si)).and_then(|s| faces.get_mut(s.key as usize)) {
            face.push(r.path);
        }
    }
    faces
        .into_iter()
        .map(|mut f| match f.len() {
            0 => PathData::default(),
            1 => f.pop().unwrap_or_default(),
            _ => po::unite_all(&f.iter().map(|p| (p, FillRule::NonZero)).collect::<Vec<_>>()),
        })
        .collect()
}

/// The pairs `(i, j)`, `i < j`, in ascending order, of `rects` (`None`: left out) that come within
/// `reach_x` of each other across x and `reach_y` across y. A sweep over the rects sorted by their
/// left edge, so shapes spread over the page cost about one check each rather than one per pair.
fn near_pairs(rects: &[Option<Rect>], reach_x: f64, reach_y: f64) -> Vec<(usize, usize)> {
    let mut order: Vec<(usize, Rect)> = rects.iter().enumerate().filter_map(|(i, r)| r.map(|r| (i, r))).collect();
    order.sort_unstable_by(|a, b| a.1.x0.total_cmp(&b.1.x0).then(a.0.cmp(&b.0)));
    let mut pairs = vec![];
    for (k, &(i, a)) in order.iter().enumerate() {
        // Later rects start at or right of `a`, so only how far right they start can rule them out.
        for &(j, b) in order[k + 1..].iter().take_while(|(_, b)| b.x0 <= a.x1 + reach_x) {
            if b.y0 <= a.y1 + reach_y && a.y0 <= b.y1 + reach_y {
                pairs.push((i.min(j), i.max(j)));
            }
        }
    }
    pairs.sort_unstable();
    pairs
}

/// The traps between `sources` (back to front) with options `o`: for each two sources of
/// different solid colours whose visible parts meet, the strip of the darker one's visible part
/// next to the lighter one's (the other way round with [`TrapOptions::reverse`]), painted per
/// [`trap_paint`]. Empty when no visible edges of different colours meet.
pub fn traps(sources: &[TrapSource], o: &TrapOptions) -> Vec<TrapPiece> {
    let colors: Vec<Option<Color>> = sources.iter().map(|s| trap_color(&s.fill)).collect();
    if colors.iter().flatten().count() < 2 {
        return vec![];
    }
    let (w, r) = (o.thickness, o.height_width);
    if !(w.is_finite() && w > 0.0 && r.is_finite() && r > 0.0) {
        return vec![]; // `trap_band` cuts nothing with these.
    }
    let vis = visible(sources);
    // Only pairs whose visible parts come within the trap's reach can trap (`trap_band` rejects
    // the rest by the same bounds), so find those first instead of trying every pair. The hair
    // keeps rounding from rejecting a pair `trap_band` would keep.
    let rects: Vec<Option<Rect>> =
        vis.iter().zip(&colors).map(|(p, c)| c.and(p.bounds()).filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()))).collect();
    let (w, r) = (w.clamp(po::TRAP_MIN_WIDTH, po::TRAP_MAX_WIDTH), r.clamp(po::TRAP_HEIGHT_WIDTH_RANGE.0, po::TRAP_HEIGHT_WIDTH_RANGE.1));
    let extent = rects.iter().flatten().flat_map(|b| [b.x0, b.y0, b.x1, b.y1]).fold(0.0_f64, |m, v| m.max(v.abs()));
    let hair = 1e-9 * (1.0 + extent + w * r.max(1.0));
    let mut out = vec![];
    for (i, j) in near_pairs(&rects, w + hair, w * r + hair) {
        let (Some(ci), Some(cj)) = (colors[i], colors[j]) else { continue };
        if vis[i].is_empty() || vis[j].is_empty() || same_color(&sources[i].fill, &sources[j].fill, ci, cj) {
            continue;
        }
        let (mut from, mut into) = if lighter(ci, cj) { (i, j) } else { (j, i) };
        if o.reverse && colors[into].is_some_and(|c| !rich_black(c)) {
            (from, into) = (into, from);
        }
        let mut band = po::trap_band(&vis[from], FillRule::NonZero, &vis[into], FillRule::NonZero, o.thickness, o.height_width, o.precision);
        if o.remove_redundant {
            band = po::remove_redundant_points(&band, o.precision);
        }
        if band.is_empty() || po::area(&band, FillRule::NonZero) <= 1e-9 {
            continue;
        }
        let Some(color) = colors[from] else { continue };
        out.push(TrapPiece { path: band, paint: trap_paint(&sources[from].fill, color, o), from, into });
    }
    out
}

/// A trap's appearance: one overprinting fill of `paint`, no stroke.
pub fn trap_appearance(paint: Paint) -> Appearance {
    Appearance { items: vec![AppearanceItem::Fill(FillLayer { overprint: true, ..FillLayer::new(paint) })], ..Default::default() }
}

/// A trap as a path object with id `id`.
pub fn trap_node(id: NodeId, piece: TrapPiece) -> Node {
    Node::path(id, piece.path, trap_appearance(piece.paint))
}

#[cfg(test)]
mod tests {
    use vectorcraft_geom::Rect;
    use vectorcraft_geom::shapes::rectangle;

    use super::*;

    fn src(r: Rect, c: Color) -> TrapSource {
        TrapSource { path: rectangle(r), rule: FillRule::NonZero, fill: Paint::solid(c) }
    }

    fn yellow() -> Color {
        Color::cmyk(0.0, 0.0, 1.0, 0.0)
    }

    fn blue() -> Color {
        Color::cmyk(1.0, 0.6, 0.0, 0.0)
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.02
    }

    fn abutting() -> Vec<TrapSource> {
        vec![src(Rect::new(0.0, 0.0, 50.0, 40.0), yellow()), src(Rect::new(50.0, 0.0, 100.0, 40.0), blue())]
    }

    #[test]
    fn the_lighter_colour_spreads_into_the_darker_along_the_shared_edge() {
        let o = TrapOptions { thickness: 3.0, tint: 1.0, ..Default::default() };
        let t = traps(&abutting(), &o);
        assert_eq!(t.len(), 1);
        let b = t[0].path.bounds().unwrap();
        assert!(near(b.x0, 50.0) && near(b.x1, 53.0) && near(b.y0, 0.0) && near(b.y1, 40.0), "inside the blue, 3 pt wide: {b:?}");
        assert_eq!((t[0].from, t[0].into), (0, 1));
        assert_eq!(t[0].paint.color(), Some(yellow()));
        let n = trap_node(NodeId(7), t[0].clone());
        assert!(n.appearance.fill().is_some_and(|f| f.overprint), "the trap overprints");
        assert!(n.appearance.stroke_paint().is_none());
    }

    #[test]
    fn reverse_traps_spread_the_darker_colour() {
        let o = TrapOptions { thickness: 2.0, reverse: true, tint: 1.0, ..Default::default() };
        let t = traps(&abutting(), &o);
        assert_eq!(t.len(), 1);
        let b = t[0].path.bounds().unwrap();
        assert!(near(b.x0, 48.0) && near(b.x1, 50.0), "inside the yellow: {b:?}");
        assert_eq!(t[0].paint.color(), Some(blue()));
    }

    #[test]
    fn reverse_traps_leave_rich_black_alone() {
        let rich = Color::cmyk(0.6, 0.4, 0.4, 1.0);
        let s = vec![src(Rect::new(0.0, 0.0, 50.0, 40.0), yellow()), src(Rect::new(50.0, 0.0, 100.0, 40.0), rich)];
        let t = traps(&s, &TrapOptions { reverse: true, ..Default::default() });
        assert_eq!(t[0].from, 0, "the yellow still spreads into the rich black");
    }

    #[test]
    fn tint_reduction_tints_the_trap_and_keeps_a_swatch_unless_process_colour() {
        let mut s = abutting();
        s[0].fill = Paint::Solid { color: yellow(), swatch: Some("PANTONE Yellow".into()), tint: 1.0 };
        let t = traps(&s, &TrapOptions { tint: 0.5, ..Default::default() });
        match &t[0].paint {
            Paint::Solid { color, swatch, tint } => {
                assert_eq!(swatch.as_deref(), Some("PANTONE Yellow"));
                assert!((tint - 0.5).abs() < 1e-6);
                assert!((color.to_cmyk()[2] - 0.5).abs() < 1e-4, "{color:?}");
            }
            p => panic!("{p:?}"),
        }
        let t = traps(&s, &TrapOptions { tint: 0.5, process: true, ..Default::default() });
        match &t[0].paint {
            Paint::Solid { color, swatch: None, .. } => assert!(matches!(color, Color::Cmyk { .. })),
            p => panic!("{p:?}"),
        }
    }

    #[test]
    fn a_lighter_object_on_a_darker_background_spreads_all_round() {
        // Yellow square on top of a blue background: a ring around the yellow, inside the blue.
        let s = vec![src(Rect::new(0.0, 0.0, 100.0, 100.0), blue()), src(Rect::new(40.0, 40.0, 60.0, 60.0), yellow())];
        let t = traps(&s, &TrapOptions { thickness: 1.0, ..Default::default() });
        assert_eq!(t.len(), 1);
        let b = t[0].path.bounds().unwrap();
        assert!(near(b.x0, 39.0) && near(b.x1, 61.0) && near(b.y0, 39.0) && near(b.y1, 61.0), "{b:?}");
        assert!(near(po::area(&t[0].path, FillRule::NonZero), 22.0 * 22.0 - 400.0));
    }

    #[test]
    fn no_trap_without_adjoining_edges_of_different_colours() {
        let apart = vec![src(Rect::new(0.0, 0.0, 10.0, 10.0), yellow()), src(Rect::new(30.0, 0.0, 40.0, 10.0), blue())];
        assert!(traps(&apart, &TrapOptions::default()).is_empty());
        let same = vec![src(Rect::new(0.0, 0.0, 10.0, 10.0), blue()), src(Rect::new(10.0, 0.0, 20.0, 10.0), blue())];
        assert!(traps(&same, &TrapOptions::default()).is_empty());
        let mut unfilled = abutting();
        unfilled[1].fill = Paint::None;
        assert!(traps(&unfilled, &TrapOptions::default()).is_empty());
    }

    #[test]
    fn options_are_bounded_and_bad_numbers_take_defaults() {
        let o = TrapOptions::from_params(&json!({"thickness": 1e300, "heightWidth": -5, "tintReduction": 400, "precision": "x"}));
        assert_eq!(o.thickness, po::TRAP_MAX_WIDTH);
        assert_eq!(o.height_width, po::TRAP_HEIGHT_WIDTH_RANGE.0);
        assert_eq!(o.tint, 1.0);
        assert_eq!(o.precision, po::DEFAULT_PRECISION);
        let o = TrapOptions::from_params(&json!({"thickness": "NaN"}));
        assert_eq!(o.thickness, TrapOptions::default().thickness);
        // The widest trap still stays inside the darker shape.
        let t = traps(&abutting(), &TrapOptions { thickness: f64::NAN, ..Default::default() });
        assert!(t.is_empty(), "a NaN thickness traps nothing");
        let t = traps(&abutting(), &TrapOptions { thickness: po::TRAP_MAX_WIDTH, ..Default::default() });
        let b = t[0].path.bounds().unwrap();
        assert!(near(b.x0, 50.0) && near(b.x1, 100.0), "{b:?}");
    }

    #[test]
    fn nan_coordinates_are_left_out() {
        let mut s = abutting();
        s[1].path.subpaths[0].anchors[0].p.x = f64::NAN;
        assert!(traps(&s, &TrapOptions::default()).is_empty());
    }

    /// 2,000 separated 10 pt squares in alternating colours on a 20 pt grid, then a yellow square
    /// abutting the first (blue) one's right edge.
    fn many_apart_and_one_abutting() -> Vec<TrapSource> {
        let mut s: Vec<TrapSource> = (0..2000)
            .map(|k| {
                let (x, y) = (f64::from(k % 50) * 20.0, f64::from(k / 50) * 20.0);
                src(Rect::new(x, y, x + 10.0, y + 10.0), if (k + k / 50) % 2 == 0 { blue() } else { yellow() })
            })
            .collect();
        s.push(src(Rect::new(10.0, 0.0, 15.0, 10.0), yellow()));
        s
    }

    #[test]
    fn only_shapes_within_reach_are_paired() {
        let s = many_apart_and_one_abutting();
        let rects: Vec<Option<Rect>> = s.iter().map(|t| t.path.bounds()).collect();
        assert_eq!(near_pairs(&rects, 1.0, 1.0), vec![(0, 2000)], "the squares 10 pt apart are never paired");
        // Reaching 10 pt, neighbours in a row pair, in ascending order; left-out rects never do.
        let row: Vec<Option<Rect>> = [3, 1, 0, 2].iter().map(|&k| s[k].path.bounds()).chain([None]).collect();
        assert_eq!(near_pairs(&row, 10.0, 10.0), vec![(0, 3), (1, 2), (1, 3)]);
        let t = traps(&s, &TrapOptions { thickness: 1.0, ..Default::default() });
        assert_eq!(t.len(), 1, "only the abutting pair traps");
        assert_eq!((t[0].from, t[0].into), (2000, 0));
        let b = t[0].path.bounds().unwrap();
        assert!(near(b.x0, 9.0) && near(b.x1, 10.0) && near(b.y0, 0.0) && near(b.y1, 10.0), "{b:?}");
    }

    #[test]
    fn pairing_by_bounds_keeps_every_trap_in_order() {
        // Shapes abutting across x and y, overlapping, and half a point apart (within a 1 pt trap),
        // none of them hiding another's edges: the traps are those of each pair alone.
        let s = vec![
            src(Rect::new(0.0, 0.0, 100.0, 100.0), blue()),
            src(Rect::new(40.0, 40.0, 60.0, 60.0), yellow()),
            src(Rect::new(100.0, 0.0, 120.0, 50.0), yellow()),
            src(Rect::new(0.0, 100.5, 50.0, 120.0), yellow()),
            src(Rect::new(55.0, 100.0, 80.0, 130.0), Color::cmyk(0.0, 1.0, 0.0, 0.0)),
        ];
        let o = TrapOptions { thickness: 1.0, ..Default::default() };
        let got: Vec<(usize, usize)> = traps(&s, &o).iter().map(|t| (t.from, t.into)).collect();
        let mut want = vec![];
        for i in 0..s.len() {
            for j in i + 1..s.len() {
                if let Some(t) = traps(&[s[i].clone(), s[j].clone()], &o).first() {
                    want.push(if t.from == 0 { (i, j) } else { (j, i) });
                }
            }
        }
        assert_eq!(want.len(), 4, "{want:?}");
        assert_eq!(got, want);
    }
}
