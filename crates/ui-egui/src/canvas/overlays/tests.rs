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
