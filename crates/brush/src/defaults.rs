//! The default brush library. All art is original and drawn here in code.
//!
//! Art is authored at 1 pt stroke weight, left → right, centred on y = 0.

use std::sync::{Arc, OnceLock};

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, FillLayer, Node, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, FillRule, PathData, Point, Rect, SubPath, shapes};

use crate::{ArtBrush, ArtScale, Bristle, BristleShape, Brush, BrushKind, Calligraphic, Colorization, Direction, PatternBrush, PatternFit, Scatter};

fn fill(c: Color) -> Appearance {
    Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(c)))], ..Default::default() }
}

fn path(p: PathData, c: Color) -> Node {
    Node::path(NodeId(0), p, fill(c))
}

fn group(children: Vec<Node>) -> Node {
    Node::group(NodeId(0), children.into_iter().map(Arc::new).collect())
}

/// Even-odd compound of `paths`, filled `c`.
fn compound(paths: Vec<PathData>, c: Color) -> Node {
    let children = paths.into_iter().map(|p| Arc::new(Node::path(NodeId(0), p, Appearance::default()))).collect();
    let mut n = Node::new(NodeId(0), NodeKind::Compound { children, rule: FillRule::EvenOdd });
    n.appearance = fill(c);
    n
}

fn smooth_closed(pts: &[Point]) -> PathData {
    let n = pts.len();
    let anchors = (0..n)
        .map(|i| {
            let (prev, p, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let t = (next - prev) / 6.0;
            Anchor::with_handles(p, p - t, p + t)
        })
        .collect();
    PathData::single(SubPath::new(anchors, true))
}

/// A 4 pt dot (Dots scatter brush, default scatter art).
pub(crate) fn dot_art() -> Node {
    path(shapes::ellipse(Rect::new(-2.0, -2.0, 2.0, 2.0)), Color::BLACK)
}

/// A smooth lens tapering to points at both ends, 100 × 4 pt.
pub(crate) fn taper_art() -> Node {
    let sp = SubPath::new(
        vec![
            Anchor::with_handles(Point::new(0.0, 0.0), Point::new(30.0, 2.7), Point::new(30.0, -2.7)),
            Anchor::with_handles(Point::new(100.0, 0.0), Point::new(70.0, -2.7), Point::new(70.0, 2.7)),
        ],
        true,
    );
    path(PathData::single(sp), Color::BLACK)
}

/// A rough, tapered charcoal smear, 100 × 6 pt.
fn charcoal_art() -> Node {
    let n = 24;
    let jitter =
        [0.82, 1.0, 0.9, 0.7, 0.95, 0.85, 1.0, 0.75, 0.92, 0.8, 0.98, 0.72, 0.88, 1.0, 0.78, 0.93, 0.86, 0.7, 0.97, 0.83, 0.9, 0.76, 1.0, 0.85];
    let mut pts = vec![];
    for i in 0..=n {
        let x = 100.0 * i as f64 / n as f64;
        let env = (std::f64::consts::PI * x / 100.0).sin().powf(0.6);
        pts.push(Point::new(x, -3.0 * env * jitter[i % jitter.len()]));
    }
    for i in (1..n).rev() {
        let x = 100.0 * i as f64 / n as f64;
        let env = (std::f64::consts::PI * x / 100.0).sin().powf(0.6);
        pts.push(Point::new(x, 3.0 * env * jitter[(i * 7 + 3) % jitter.len()]));
    }
    let main = path(PathData::single(SubPath::polyline(&pts, true)), Color::gray(0.85));
    // A few grainy streaks inside.
    let streak = |y: f64, x0: f64, x1: f64| path(shapes::ellipse(Rect::new(x0, y - 0.35, x1, y + 0.35)), Color::WHITE);
    group(vec![main, streak(-1.2, 20.0, 45.0), streak(0.9, 55.0, 85.0), streak(0.1, 8.0, 22.0)])
}

/// A shaft with a triangular head at the right end, 100 × 8 pt.
fn arrow_art() -> Node {
    let shaft = shapes::rectangle(Rect::new(0.0, -1.0, 84.0, 1.0));
    let head = PathData::single(SubPath::polyline(&[Point::new(80.0, -4.0), Point::new(100.0, 0.0), Point::new(80.0, 4.0)], true));
    group(vec![path(shaft, Color::BLACK), path(head, Color::BLACK)])
}

/// A small leaf, 12 × 6 pt, pointing right.
fn leaf_art() -> Node {
    let body = smooth_closed(&[Point::new(0.0, 0.0), Point::new(6.0, -3.0), Point::new(12.0, 0.0), Point::new(6.0, 3.0)]);
    let vein = shapes::rectangle(Rect::new(1.0, -0.2, 11.0, 0.2));
    group(vec![path(body, Color::rgb8(0x3f, 0x8f, 0x4a)), path(vein, Color::rgb8(0xc9, 0xe8, 0xb5))])
}

/// Two confetti squares of different greys (tinted by the stroke colour).
fn confetti_art() -> Node {
    group(vec![
        path(shapes::rectangle(Rect::new(-1.5, -1.5, 1.5, 1.5)), Color::BLACK),
        path(shapes::rectangle(Rect::new(0.5, 0.5, 2.5, 2.5)), Color::gray(0.45)),
    ])
}

/// One chain link (an oval ring) plus half of the connecting links, 14 × 6 pt.
fn chain_tile() -> Node {
    let ring = compound(vec![shapes::ellipse(Rect::new(1.0, -3.0, 13.0, 3.0)), shapes::ellipse(Rect::new(3.0, -1.4, 11.0, 1.4))], Color::BLACK);
    let bar_l = path(shapes::rectangle(Rect::new(0.0, -0.6, 1.5, 0.6)), Color::BLACK);
    let bar_r = path(shapes::rectangle(Rect::new(12.5, -0.6, 14.0, 0.6)), Color::BLACK);
    group(vec![ring, bar_l, bar_r])
}

/// A round ring for chain corners, 8 × 8 pt.
fn chain_corner() -> Node {
    compound(vec![shapes::ellipse(Rect::new(-4.0, -4.0, 4.0, 4.0)), shapes::ellipse(Rect::new(-2.2, -2.2, 2.2, 2.2))], Color::BLACK)
}

/// A dash with a clear gap (running stitches), 10 × 1.5 pt.
pub(crate) fn stitch_tile() -> Node {
    let dash = path(shapes::rounded_rectangle(Rect::new(1.5, -0.75, 8.5, 0.75), 0.75), Color::BLACK);
    // Invisible spacer that fixes the tile width.
    let mut spacer = path(shapes::rectangle(Rect::new(0.0, -0.75, 10.0, 0.75)), Color::BLACK);
    spacer.appearance = Appearance::default();
    group(vec![spacer, dash])
}

/// A diamond tile for borders, 8 × 6 pt.
fn diamond_tile() -> Node {
    let d = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(4.0, -3.0), Point::new(8.0, 0.0), Point::new(4.0, 3.0)], true));
    path(d, Color::BLACK)
}

fn calli(name: &str, angle: f64, roundness: f64, size: f64) -> Brush {
    Brush { name: name.into(), kind: BrushKind::Calligraphic(Calligraphic { angle, roundness, size, ..Default::default() }) }
}

/// The default library.
pub fn defaults() -> &'static [Brush] {
    static LIB: OnceLock<Vec<Brush>> = OnceLock::new();
    LIB.get_or_init(|| {
        vec![
            calli("3 pt. Round", 0.0, 100.0, 3.0),
            calli("6 pt. Flat", 45.0, 15.0, 6.0),
            calli("10 pt. Oval", 30.0, 45.0, 10.0),
            Brush {
                name: "Charcoal".into(),
                kind: BrushKind::Art(ArtBrush { art: charcoal_art(), colorization: Colorization::Tints, ..Default::default() }),
            },
            Brush { name: "Tapered Stroke".into(), kind: BrushKind::Art(ArtBrush::default()) },
            Brush {
                name: "Arrow".into(),
                kind: BrushKind::Art(ArtBrush {
                    art: arrow_art(),
                    direction: Direction::LeftToRight,
                    scale: ArtScale::BetweenGuides { start: 0.0, end: 0.78 },
                    colorization: Colorization::Tints,
                    ..Default::default()
                }),
            },
            Brush {
                name: "Dots".into(),
                kind: BrushKind::Scatter(Scatter { spacing: (160.0, 160.0), colorization: Colorization::Tints, ..Default::default() }),
            },
            Brush {
                name: "Confetti".into(),
                kind: BrushKind::Scatter(Scatter {
                    art: confetti_art(),
                    size: (60.0, 140.0),
                    spacing: (120.0, 220.0),
                    scatter: (-150.0, 150.0),
                    rotation: (-180.0, 180.0),
                    rotation_relative_to_path: false,
                    colorization: Colorization::Tints,
                    modes: None,
                }),
            },
            Brush {
                name: "Leaves".into(),
                kind: BrushKind::Scatter(Scatter {
                    art: leaf_art(),
                    size: (80.0, 120.0),
                    spacing: (90.0, 130.0),
                    scatter: (-60.0, 60.0),
                    rotation: (-35.0, 35.0),
                    rotation_relative_to_path: true,
                    colorization: Colorization::None,
                    modes: None,
                }),
            },
            Brush {
                name: "Chain".into(),
                kind: BrushKind::Pattern(PatternBrush {
                    side: chain_tile(),
                    outer_corner: Some(chain_corner()),
                    inner_corner: Some(chain_corner()),
                    fit: PatternFit::Stretch,
                    ..Default::default()
                }),
            },
            Brush { name: "Stitches".into(), kind: BrushKind::Pattern(PatternBrush::default()) },
            Brush {
                name: "Diamonds".into(),
                kind: BrushKind::Pattern(PatternBrush { side: diamond_tile(), spacing: 25.0, fit: PatternFit::AddSpace, ..Default::default() }),
            },
            Brush { name: "Bristle Round".into(), kind: BrushKind::Bristle(Bristle::default()) },
            Brush {
                name: "Bristle Flat Fan".into(),
                kind: BrushKind::Bristle(Bristle {
                    shape: BristleShape::FlatFan,
                    size: 9.0,
                    density: 80.0,
                    thickness: 25.0,
                    opacity: 55.0,
                    ..Default::default()
                }),
            },
        ]
    })
}
