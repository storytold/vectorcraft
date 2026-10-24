//! The parameters of a live heart, arrow or gear (#3865): what `shape.heart`, `shape.arrow` and
//! `shape.gear` draw, kept so they stay editable after drawing ([`crate::LiveShape::Figure`]).

use serde::{Deserialize, Serialize};
use vectorcraft_geom::figures::{self, ARROW_MIN, GEAR_DEPTH_RANGE, GEAR_MAX_TEETH, GEAR_MIN_RIM};
use vectorcraft_geom::{PathData, Rect};

/// A figure's own parameters, as fractions of its box or radius (the commands take percents).
/// Values read from a file are clamped as the commands clamp them ([`Figure::clamped`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "figure", rename_all = "lowercase", from = "FigureWire")]
pub enum Figure {
    Heart,
    Arrow {
        /// The shaft's thickness, a fraction of the height.
        shaft: f64,
        /// Each head's length, a fraction of the width (with two heads at most half of it).
        head: f64,
        /// 1: pointing right, 2: both ways.
        heads: u32,
    },
    Gear {
        /// 3 to [`GEAR_MAX_TEETH`].
        teeth: u32,
        /// How far the teeth stand out, a fraction of the outer radius.
        depth: f64,
        /// The hole's radius, a fraction of the outer radius (0: none).
        hole: f64,
    },
}

/// [`Figure`] as a file has it, before clamping.
#[derive(Deserialize)]
#[serde(tag = "figure", rename_all = "lowercase")]
enum FigureWire {
    Heart,
    Arrow { shaft: f64, head: f64, heads: u32 },
    Gear { teeth: u32, depth: f64, hole: f64 },
}

impl From<FigureWire> for Figure {
    fn from(w: FigureWire) -> Self {
        match w {
            FigureWire::Heart => Figure::Heart,
            FigureWire::Arrow { shaft, head, heads } => Figure::Arrow { shaft, head, heads },
            FigureWire::Gear { teeth, depth, hole } => Figure::Gear { teeth, depth, hole },
        }
        .clamped()
    }
}

/// `v` clamped into `lo..=hi`, or `fallback` when it isn't a number.
fn clamp(v: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { fallback }
}

impl Figure {
    /// The arrow `shape.arrow` draws with its defaults.
    pub const ARROW: Figure = Figure::Arrow { shaft: 0.4, head: 0.35, heads: 1 };
    /// The gear `shape.gear` draws with its defaults.
    pub const GEAR: Figure = Figure::Gear { teeth: 12, depth: 0.2, hole: 0.3 };

    /// The figure's name (the Layers panel's and the undo step's).
    pub fn label(&self) -> &'static str {
        match self {
            Figure::Heart => "Heart",
            Figure::Arrow { .. } => "Arrow",
            Figure::Gear { .. } => "Gear",
        }
    }

    /// The parameters as the outline uses them: an arrow's shaft and head in their ranges (a head
    /// at most half the width with two heads), 1 or 2 heads; a gear's teeth, depth and hole in
    /// theirs (the hole leaving a rim). Values that aren't numbers become the defaults.
    pub fn clamped(self) -> Self {
        match self {
            Figure::Heart => Figure::Heart,
            Figure::Arrow { shaft, head, heads } => {
                let heads = heads.clamp(1, 2);
                Figure::Arrow {
                    shaft: clamp(shaft, ARROW_MIN, 1.0, 0.4),
                    head: clamp(head, ARROW_MIN, if heads == 2 { 0.5 } else { 1.0 }, 0.35),
                    heads,
                }
            }
            Figure::Gear { teeth, depth, hole } => {
                let depth = clamp(depth, GEAR_DEPTH_RANGE.0, GEAR_DEPTH_RANGE.1, 0.2);
                Figure::Gear { teeth: teeth.clamp(3, GEAR_MAX_TEETH), depth, hole: clamp(hole, 0.0, 1.0 - depth - GEAR_MIN_RIM, 0.0) }
            }
        }
    }

    /// The outline in `r`.
    pub fn outline(&self, r: Rect) -> PathData {
        match *self {
            Figure::Heart => figures::heart(r),
            Figure::Arrow { shaft, head, heads } => figures::arrow(r, shaft, head, heads),
            Figure::Gear { teeth, depth, hole } => figures::gear(r, teeth, depth, hole),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_are_the_shape_commands_outlines() {
        let r = Rect::new(0.0, 0.0, 120.0, 80.0);
        assert_eq!(Figure::Heart.outline(r), figures::heart(r));
        assert_eq!(Figure::ARROW.outline(r), figures::arrow(r, 0.4, 0.35, 1));
        assert_eq!(Figure::GEAR.outline(r), figures::gear(r, 12, 0.2, 0.3));
        assert_eq!([Figure::Heart.label(), Figure::ARROW.label(), Figure::GEAR.label()], ["Heart", "Arrow", "Gear"]);
    }

    #[test]
    fn parameters_are_clamped_as_the_outlines_clamp_them() {
        assert_eq!(Figure::Arrow { shaft: 3.0, head: 0.9, heads: 7 }.clamped(), Figure::Arrow { shaft: 1.0, head: 0.5, heads: 2 });
        assert_eq!(Figure::Arrow { shaft: f64::NAN, head: -1.0, heads: 0 }.clamped(), Figure::Arrow { shaft: 0.4, head: ARROW_MIN, heads: 1 });
        assert_eq!(Figure::Gear { teeth: 1, depth: 0.5, hole: 0.9 }.clamped(), Figure::Gear { teeth: 3, depth: 0.5, hole: 0.5 - GEAR_MIN_RIM });
        assert_eq!(
            Figure::Gear { teeth: 10_000, depth: f64::INFINITY, hole: f64::NAN }.clamped(),
            Figure::Gear { teeth: GEAR_MAX_TEETH, depth: 0.2, hole: 0.0 }
        );
        assert_eq!(Figure::Heart.clamped(), Figure::Heart);
    }

    #[test]
    fn a_file_names_the_figure_and_its_values_are_clamped_when_read() {
        let text = serde_json::to_string(&Figure::GEAR).unwrap();
        assert_eq!(text, r#"{"figure":"gear","teeth":12,"depth":0.2,"hole":0.3}"#);
        assert_eq!(serde_json::from_str::<Figure>(&text).unwrap(), Figure::GEAR);
        assert_eq!(serde_json::to_string(&Figure::Heart).unwrap(), r#"{"figure":"heart"}"#);
        let huge: Figure = serde_json::from_str(r#"{"figure":"gear","teeth":4000000000,"depth":0,"hole":1}"#).unwrap();
        assert_eq!(huge, Figure::Gear { teeth: GEAR_MAX_TEETH, depth: GEAR_DEPTH_RANGE.0, hole: 1.0 - GEAR_DEPTH_RANGE.0 - GEAR_MIN_RIM });
        assert!(serde_json::from_str::<Figure>(r#"{"figure":"cog"}"#).is_err());
    }

    fn live(figure: Figure) -> crate::LiveShape {
        crate::LiveShape::Figure {
            figure,
            w: 120.0,
            h: 80.0,
            xf: vectorcraft_geom::Affine::translate((20.0, 30.0)),
            radii: vec![],
            kinds: vec![],
            radii_relative: false,
        }
    }

    #[test]
    fn a_live_figure_draws_its_outline_in_its_box_and_is_saved_flat() {
        let l = live(Figure::GEAR);
        assert_eq!(l.label(), "Gear");
        assert_eq!(l.figure(), Some(Figure::GEAR));
        let want = figures::gear(Rect::new(20.0, 30.0, 140.0, 110.0), 12, 0.2, 0.3);
        let got = l.to_path();
        assert_eq!(got.anchor_count(), want.anchor_count());
        assert!(got.anchors().zip(want.anchors()).all(|((_, _, x), (_, _, y))| x.p.distance(y.p) < 1e-9 && x.h_out.distance(y.h_out) < 1e-9));
        let v = serde_json::to_value(&l).unwrap();
        assert_eq!(v["shape"], "figure");
        assert_eq!(v["figure"], "gear");
        assert_eq!(v["teeth"], 12);
        assert!(v.get("radii").is_none() && v.get("kinds").is_none(), "{v}");
        assert_eq!(serde_json::from_value::<crate::LiveShape>(v).unwrap(), l);
        // A count from a file is clamped as the commands clamp it.
        let mut v = serde_json::to_value(live(Figure::ARROW)).unwrap();
        v["heads"] = 9.into();
        assert_eq!(serde_json::from_value::<crate::LiveShape>(v).unwrap().figure(), Some(Figure::Arrow { shaft: 0.4, head: 0.35, heads: 2 }));
    }

    #[test]
    fn a_live_figure_transforms_with_its_box_and_keeps_its_parameters() {
        let mut l = live(Figure::ARROW);
        let a = vectorcraft_geom::Affine::rotate(0.5) * vectorcraft_geom::Affine::scale_non_uniform(2.0, 0.5);
        let mut want = l.to_path();
        want.transform(a);
        assert!(!l.transform(a), "no corners to cut again");
        let got = l.to_path();
        for ((_, _, x), (_, _, y)) in got.anchors().zip(want.anchors()) {
            assert!(x.p.distance(y.p) < 1e-9);
        }
        assert_eq!(l.figure(), Some(Figure::ARROW));
        assert!((l.angle_deg() - 0.5f64.to_degrees()).abs() < 1e-9);
    }

    #[test]
    fn new_parameters_keep_a_figures_kind_and_its_shared_corners() {
        let mut l = live(Figure::GEAR);
        assert!(!l.set_figure(Figure::GEAR), "the same values change nothing");
        assert!(!l.set_figure(Figure::ARROW), "a gear stays a gear");
        // Round every corner, then give it more teeth: the new corners are as round.
        let (mut path, mut boxed) = (l.to_path(), Some(Box::new(l)));
        // 48 anchors round the teeth and 4 round the hole.
        let all: std::collections::BTreeSet<usize> = (0..52).collect();
        crate::corners::set_corners(&mut path, &mut boxed, &all, Some(1.0), None);
        let mut l = *boxed.unwrap();
        assert!(matches!(&l, crate::LiveShape::Figure { radii, .. } if radii.len() == 52 && radii.iter().all(|r| *r == 1.0)), "{l:?}");
        assert!(l.set_figure(Figure::Gear { teeth: 20, depth: 0.2, hole: 0.3 }));
        assert!(matches!(&l, crate::LiveShape::Figure { radii, .. } if radii.len() == 84 && radii.iter().all(|r| *r == 1.0)));
        // A second head: the arrow's corners spread as well, and a heart has nothing to set.
        let mut a = live(Figure::ARROW);
        assert!(a.set_figure(Figure::Arrow { shaft: 0.4, head: 0.35, heads: 2 }));
        assert_eq!(a.polygon_outline().anchor_count(), 10);
        assert!(!live(Figure::Heart).set_figure(Figure::Heart));
        // Other live shapes have no figure.
        let mut e = crate::LiveShape::Line { a: vectorcraft_geom::Point::ZERO, b: vectorcraft_geom::Point::new(1.0, 1.0) };
        assert!(e.figure().is_none() && !e.set_figure(Figure::Heart));
    }
}
