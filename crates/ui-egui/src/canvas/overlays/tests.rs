use super::*;
use vectorcraft_geom::{Point, Rect};

fn view() -> CanvasView {
    CanvasView::new(ScreenRect::from_min_size(Pos2::ZERO, Vec2::new(160.0, 200.0)), Point::new(80.0, 100.0), 1.0)
}

fn tip(x: f64) -> Overlay {
    Overlay::HoverText { center: Point::new(x, 100.0), radius: 10.0, text: "Tip" }
}

/// The badge names the current rule and the other one, and flips with `enclosed`.
#[test]
fn the_badge_names_the_rule_and_the_other_one() {
    let (touching, fully) = (badge_rule(false), badge_rule(true));
    assert_eq!((touching.0.as_str(), touching.1.as_str()), (tl!("Touching"), tl!("Fully Enclosed")));
    assert_eq!(fully, (touching.1.clone(), touching.0.clone()));
    assert_ne!(name(false), name(true));
}

/// A pointer at the canvas edge keeps its badge anchored inside the canvas.
#[test]
fn the_badge_stays_inside_the_canvas() {
    let view = view();
    for pointer in [Point::new(155.0, 195.0), Point::new(5.0, 5.0), Point::new(1.0e6, -1.0e6)] {
        assert!(view.rect.contains(badge_at(&view, pointer)), "{pointer:?}");
    }
}

/// Only a distance overlay gives a measure, with its own text and colour.
#[test]
fn a_distance_gives_a_measure_with_its_label() {
    let distance = Overlay::Distance { a: Point::new(0.0, 0.0), b: Point::new(24.0, 0.0), text: "24 px".into(), color: [255, 0, 255] };
    let (measure, color) = distance_measure(&distance).unwrap();
    assert_eq!(measure, Measure::new(Point::new(0.0, 0.0), Point::new(24.0, 0.0)).labelled("24 px"));
    assert_eq!(color, data([255, 0, 255]));
    assert!(distance_measure(&Overlay::Marquee(Rect::new(0.0, 0.0, 1.0, 1.0))).is_none());
}

/// A tip disc is declared for each hover region, and none once the hover ends or while quiet.
#[test]
fn tips_follow_the_regions_and_go_quiet() {
    let view = view();
    let (list, none): (&[Overlay], &[Overlay]) = (&[tip(80.0), tip(10.0)], &[]);
    assert_eq!(tip_regions(&[none, list], &view, false).len(), 2);
    assert!(tip_regions(&[none, none], &view, false).is_empty());
    assert!(tip_regions(&[none, list], &view, true).is_empty());
}

/// A region without a size has no tip, and a region keeps its place in a zoomed view.
#[test]
fn a_tip_disc_is_scaled_by_the_zoom_and_never_empty() {
    let mut view = view();
    view.zoom = 2.0;
    let empty = Overlay::HoverText { center: Point::new(80.0, 100.0), radius: 0.0, text: "Tip" };
    let (shape, _) = tip_regions(&[&[tip(80.0), empty]], &view, false).remove(0);
    assert_eq!(shape, HitShape::circle(Pos2::new(80.0, 100.0), 20.0));
    assert_eq!(tip_regions(&[&[empty_tip()]], &view, false).len(), 0);
}

fn empty_tip() -> Overlay {
    Overlay::HoverText { center: Point::new(f64::NAN, 0.0), radius: 5.0, text: "Tip" }
}

/// The dashes of `points` in `pattern`, as `Draw::dashed` lays them out.
fn dashes_along(points: &[Pos2], pattern: Dashes) -> Vec<egui::Shape> {
    let s = egui::Stroke::new(1.0, Color32::WHITE);
    egui::Shape::dashed_line_with_offset(points, s, &[pattern.dash], &[pattern.gap], pattern.phase)
}

/// The stretches of x that horizontal dashes cover inside `lo..hi`, in order.
fn spans(shapes: &[egui::Shape], lo: f32, hi: f32) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = shapes
        .iter()
        .map(|s| match s {
            egui::Shape::LineSegment { points: [a, b], .. } => (a.x.min(b.x).max(lo), a.x.max(b.x).min(hi)),
            other => panic!("{other:?}"),
        })
        .filter(|(a, b)| b > a)
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// A dashed line much longer than the canvas is dashed only where it shows, with the dashes on the canvas where the
/// whole line dashed in one go has them, for any phase of the pattern.
#[test]
fn a_long_dashed_line_is_dashed_only_on_the_canvas() {
    let view = view();
    let clip = view.rect.expand(8.0);
    let (lo, hi) = (view.rect.left(), view.rect.right());
    // From 1,000 points left of the canvas to 1,740 points right of it.
    let (a, b) = (Point::new(-1000.0, 100.0), Point::new(1900.0, 100.0));
    for phase in [0.0, 2.5, 6.9] {
        let pattern = Dashes::new(4.0, 3.0).phase(phase);
        let whole = dashes_along(&[view.to_screen(a), view.to_screen(b)], pattern);
        let (run, from) = visible_run(&view, a, b, clip).unwrap();
        let drawn = dashes_along(&run, dashes_from(pattern, from));
        let (want, got) = (spans(&whole, lo, hi), spans(&drawn, lo, hi));
        assert_eq!(want.len(), got.len(), "{phase}: {want:?} {got:?}");
        assert!(want.iter().zip(&got).all(|(w, g)| (w.0 - g.0).abs() < 1e-2 && (w.1 - g.1).abs() < 1e-2), "{phase}: {want:?} {got:?}");
        assert!(drawn.len() <= 30, "only the dashes on the canvas: {}", drawn.len());
    }
}

/// Ends 10,000,000,000 points away on both sides still keep the part on the canvas, dashed in whole dashes.
#[test]
fn a_dashed_line_with_far_ends_keeps_its_part_on_the_canvas() {
    let view = view();
    let pattern = Dashes::new(4.0, 3.0);
    let (run, from) = visible_run(&view, Point::new(-1e10, 100.0), Point::new(1e10, 100.0), view.rect.expand(8.0)).unwrap();
    assert!(from > 1e9, "{from}");
    let drawn = spans(&dashes_along(&run, dashes_from(pattern, from)), view.rect.left(), view.rect.right());
    assert!((20..=26).contains(&drawn.len()), "{drawn:?}");
    assert!(drawn.iter().all(|(a, b)| b - a <= 4.0 + 1e-3), "{drawn:?}");
    // Ends so far off that the clipped ends round to one point along the line still keep the canvas's width.
    let clip = view.rect.expand(8.0);
    let (run, _) = visible_run(&view, Point::new(-1e20, 100.0), Point::new(1e20, 100.0), clip).unwrap();
    assert_eq!(run, [Pos2::new(clip.left(), 100.0), Pos2::new(clip.right(), 100.0)]);
    // A slanted line keeps its slant there too.
    let (run, _) = visible_run(&view, Point::new(-1e20, -1e20), Point::new(1e20, 1e20), clip).unwrap();
    assert!(run.iter().all(|p| (p.x - p.y).abs() < 1e-3 && clip.expand(1e-2).contains(*p)), "{run:?}");
    assert!(run[0].distance(run[1]) > 200.0, "{run:?}");
    // Far-off ends off one diagonal keep the line's offset: these ends, as `f64` holds them, give a line that
    // crosses x = 0 at y = 72.35.
    let (run, _) = visible_run(&view, Point::new(-1e18, -1e18 + 100.0), Point::new(1.3e18, 1.3e18 + 100.0), clip).unwrap();
    assert!(run.iter().all(|p| (p.y - p.x - 72.35).abs() < 1e-2 && clip.expand(1e-2).contains(*p)), "{run:?}");
}

/// The line segments in `shapes`, with nested shape lists opened.
fn segments(shapes: &[egui::Shape]) -> Vec<[Pos2; 2]> {
    let mut out = Vec::new();
    for s in shapes {
        match s {
            egui::Shape::LineSegment { points, .. } => out.push(*points),
            egui::Shape::Vec(inner) => out.extend(segments(inner)),
            other => panic!("{other:?}"),
        }
    }
    out
}

/// A long dashed line overlay painted through `paint` gives only the dashes near the canvas, each on the line.
#[test]
fn a_long_dashed_line_overlay_paints_only_its_dashes_on_the_canvas() {
    use craft::i18n::Direction;
    use craft::theme::{Contrast, Mode};
    use craft::ui::style::Ink;
    use craft::ui::testing::Frames;
    use craft::ui::testing::style::{paint_frame, tokens};

    let view = view();
    let tokens = tokens(Mode::Light, Contrast::Standard);
    let style = ToolMarksStyle {
        metrics: Metrics::new(),
        dark: StyleColor(Color32::BLACK),
        ink: StyleColor(Color32::WHITE),
        accent: StyleColor(Color32::BLUE),
        pad: Vec2::ZERO,
        radius: CornerRadius::ZERO,
        handle: 3.0,
    };
    let line = Overlay::Line { a: Point::new(-1000.0, 100.0), b: Point::new(1900.0, 100.0), color: [255, 0, 255], dashed: true };
    let mut frames = Frames::new();
    let (shapes, _) = paint_frame(&mut frames, &tokens, Direction::Ltr, Ink::OnPanel, |draw| {
        paint(draw, &view, std::slice::from_ref(&line), &PathOptionsSettings::default(), &style);
    });
    let drawn = segments(&shapes);
    assert!(!drawn.is_empty(), "the part on the canvas is drawn");
    assert!(drawn.len() <= 40, "only the dashes near the canvas: {}", drawn.len());
    let near = view.rect.expand(32.0);
    assert!(drawn.iter().flatten().all(|p| near.contains(*p) && (p.y - 100.0).abs() < 1e-3), "{drawn:?}");
}

/// A turned view clips the line to the canvas too, and a line off the canvas or with an end that is not finite draws
/// nothing.
#[test]
fn a_dashed_line_off_the_canvas_draws_nothing() {
    let mut view = view();
    view.rotation = 0.3;
    let clip = view.rect.expand(8.0);
    let (run, _) = visible_run(&view, Point::new(-1e6, 100.0), Point::new(1e6, 100.0), clip).unwrap();
    assert!(run.iter().all(|p| clip.expand(1e-2).contains(*p)), "{run:?}");
    assert!(visible_run(&view, Point::new(-1e6, 5000.0), Point::new(1e6, 5000.0), clip).is_none());
    assert!(visible_run(&view, Point::new(0.0, 100.0), Point::new(f64::INFINITY, 100.0), clip).is_none());
    assert!(visible_run(&view, Point::new(f64::NAN, 100.0), Point::new(10.0, 100.0), clip).is_none());
}
