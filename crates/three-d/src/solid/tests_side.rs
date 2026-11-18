use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect};

use super::super::{Bevel, Extrude, View, extrude, extrude_side_size};
use super::{across_steps, along_ring, longest};

fn square(r: Rect, hole: Option<Rect>) -> PathData {
    let mut b = BezPath::new();
    b.move_to((r.x0, r.y0));
    b.line_to((r.x1, r.y0));
    b.line_to((r.x1, r.y1));
    b.line_to((r.x0, r.y1));
    b.close_path();
    if let Some(h) = hole {
        b.move_to((h.x0, h.y0));
        b.line_to((h.x0, h.y1));
        b.line_to((h.x1, h.y1));
        b.line_to((h.x1, h.y0));
        b.close_path();
    }
    PathData::from_bezpath(&b)
}

/// x is 0 at the top of the middle line and grows clockwise on the page; the side is cut at the
/// anchor nearest half way around (the first of two equally near).
#[test]
fn x_runs_clockwise_from_the_top_middle() {
    let ring = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
    assert_eq!(along_ring(&ring).unwrap(), vec![[-5.0, 5.0], [5.0, 15.0], [-25.0, -15.0], [-15.0, -5.0]]);
    // The same outline run the other way gives each point the same x, apart from the seam, which
    // is now at (0, 10), its first anchor of the two equally near.
    let back: Vec<Point> = ring.iter().rev().copied().collect();
    let x = along_ring(&back).unwrap();
    assert_eq!(x, vec![[25.0, 15.0], [15.0, 5.0], [5.0, -5.0], [-5.0, -15.0]]);
    assert_eq!(along_ring(&[Point::ZERO, Point::ZERO]), None);
}

#[test]
fn y_runs_across_the_steps_from_their_middle() {
    // A 4 × 3 pt bevel step, then 20 pt of wall, on a solid 30 pt deep.
    let y = across_steps(&[(0.0, 0.0), (-4.0, 0.1), (-4.0, 0.1 + 20.0 / 30.0)], 30.0);
    let expected = [-12.5, -7.5, 12.5];
    for (a, b) in y.iter().zip(expected) {
        assert!((a - b).abs() < 1e-9, "{y:?}");
    }
}

/// With graphics on the side, every piece of the walls of the longest contour carries its place on
/// the side unrolled, and no other facet does: x along the outline and y from the front to the back.
#[test]
fn the_longest_contours_walls_carry_their_place_on_the_side() {
    let (outer, inner) = (Rect::new(0.0, 0.0, 100.0, 60.0), Rect::new(30.0, 20.0, 70.0, 40.0));
    let path = square(outer, Some(inner));
    let o = Extrude { depth: 40.0, map_surface: true, ..Extrude::default() };
    // Seen from above: the top wall faces the viewer and keeps its x on the page.
    let top = View { rotation_x: -90.0, ..View::default() };
    let facets = extrude(&path, FillRule::NonZero, outer, o, top).unwrap();
    let cells: Vec<_> = facets.iter().filter_map(|f| f.surface.as_ref().map(|c| (f, c))).collect();
    assert!(!cells.is_empty());
    assert!(cells.iter().all(|(f, c)| f.side && f.end.is_none() && c.uv.len() == 4 && c.page.len() == 4));
    let seen: Vec<_> = cells.iter().filter(|(_, c)| c.outward).collect();
    assert_eq!(seen.len(), 1, "only the top wall of the outline faces the viewer");
    for (_, c) in seen {
        for (uv, page) in c.uv.iter().zip(&c.page) {
            assert!((page.x - (50.0 + uv.x)).abs() < 1e-6, "{uv:?} {page:?}");
            assert!(uv.y.abs() <= 20.0 + 1e-9);
        }
    }
    // Only the outline's top wall is drawn (the walls seen edge-on have no area on the page, and the
    // bottom wall faces away); the hole's walls carry none. Without graphics no facet carries one.
    assert_eq!(cells.len(), 1);
    let turned = View { rotation_x: -60.0, rotation_y: 30.0, ..View::default() };
    let hollow = extrude(&path, FillRule::NonZero, outer, Extrude { cap: false, ..o }, turned).unwrap();
    let hollow: Vec<_> = hollow.iter().filter_map(|f| f.surface.as_ref()).collect();
    // A hollow solid shows the insides of its far walls: they carry a cell that is not outward.
    assert!(hollow.iter().any(|c| c.outward) && hollow.iter().any(|c| !c.outward), "{}", hollow.len());
    let plain = extrude(&path, FillRule::NonZero, outer, Extrude { map_surface: false, ..o }, top).unwrap();
    assert!(plain.iter().all(|f| f.surface.is_none()));
    assert_eq!(plain.len(), facets.len());
}

#[test]
fn the_side_measures_around_the_outline_and_across_the_bevels() {
    let outer = Rect::new(0.0, 0.0, 100.0, 60.0);
    let path = square(outer, None);
    let plain = Extrude { depth: 40.0, ..Extrude::default() };
    // The cut is at a bottom corner, 110 pt each way from the top's middle: 220 pt about x = 0
    // stay clear of it.
    assert_eq!(extrude_side_size(&path, FillRule::NonZero, outer, plain), Some((320.0, 40.0, 220.0)));
    // A 3 × 4 pt classic bevel outside adds its 5 pt slant and takes 4 pt of the wall.
    let bevel = Extrude { bevel: Some(Bevel { width: 3.0, height: 4.0, ..Bevel::default() }), ..plain };
    let (w, h, span) = extrude_side_size(&path, FillRule::NonZero, outer, bevel).unwrap();
    assert!((w - 320.0).abs() < 1e-9 && (h - 41.0).abs() < 1e-9 && (span - 220.0).abs() < 1e-9, "{w} {h} {span}");
    assert_eq!(extrude_side_size(&path, FillRule::NonZero, outer, Extrude { depth: 0.0, ..plain }), None);
    let rings = [&[Point::ZERO, Point::new(1.0, 0.0), Point::new(0.0, 1.0)][..], &[Point::ZERO, Point::new(5.0, 0.0), Point::new(0.0, 5.0)][..]];
    assert_eq!(longest(&rings), Some(1));
}
