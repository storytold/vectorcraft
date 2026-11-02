use kurbo::{Point, Vec2};
use vectorcraft_geom::{Anchor, AnchorKind, SubPath};

use super::{close_nearly_closed, merge_duplicate_anchors, near_point_groups, rejoin, remove_collinear_anchors, retract_tiny_handles};

fn line(pts: &[(f64, f64)], closed: bool) -> SubPath {
    SubPath::polyline(&pts.iter().map(|&p| p.into()).collect::<Vec<Point>>(), closed)
}

fn points(sp: &SubPath) -> Vec<(f64, f64)> {
    sp.anchors.iter().map(|a| (a.p.x, a.p.y)).collect()
}

#[test]
fn collinear_anchors_go_and_corners_stay() {
    let mut sp = line(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 5.0)], true);
    assert_eq!(remove_collinear_anchors(&mut sp, 0.005), 2);
    assert_eq!(points(&sp), [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    assert_eq!(remove_collinear_anchors(&mut sp, 0.005), 0, "clean art is left alone");
    // A turn back on itself is not a straight run; an open end stays.
    let mut back = line(&[(0.0, 0.0), (10.0, 0.0), (5.0, 0.0)], false);
    assert_eq!(remove_collinear_anchors(&mut back, 0.005), 0);
    // An anchor with a handle is not a corner on a straight run.
    let mut curved = line(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)], false);
    curved.anchors[1] = Anchor::smooth((5.0, 0.0).into(), (7.0, 0.0).into());
    assert_eq!(remove_collinear_anchors(&mut curved, 0.005), 0);
    // A slight bend within the tolerance goes; a larger one stays.
    let mut bend = line(&[(0.0, 0.0), (100.0, 0.1), (200.0, 0.0)], false);
    assert_eq!(remove_collinear_anchors(&mut bend.clone(), 0.0001), 0);
    assert_eq!(remove_collinear_anchors(&mut bend, 0.005), 1);
}

#[test]
fn duplicate_anchors_merge_keeping_the_outer_handles() {
    let mut sp = SubPath::new(
        vec![
            Anchor::with_handles((0.0, 0.0).into(), (0.0, 0.0).into(), (0.0, 0.0).into()),
            Anchor::with_handles((10.0, 0.0).into(), (8.0, 0.0).into(), (10.0, 0.0).into()),
            Anchor::with_handles((10.005, 0.0).into(), (10.005, 0.0).into(), (10.005, 3.0).into()),
            Anchor::corner((10.0, 10.0).into()),
        ],
        false,
    );
    assert_eq!(merge_duplicate_anchors(&mut sp, 0.01), 1);
    assert_eq!(sp.anchors.len(), 3);
    let a = sp.anchors[1];
    assert_eq!((a.p, a.h_in), (Point::new(10.0, 0.0), Point::new(8.0, 0.0)));
    assert!(a.h_out.distance(Point::new(10.0, 3.0)) < 1e-9, "{:?}", a.h_out);
    assert_eq!(merge_duplicate_anchors(&mut sp, 0.01), 0);
    // A closed path's last anchor on its first merges into it.
    let mut ring = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)], true);
    assert_eq!(merge_duplicate_anchors(&mut ring, 0.01), 1);
    assert_eq!(points(&ring), [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
}

#[test]
fn tiny_handles_retract() {
    let p = Point::new(5.0, 5.0);
    let mut sp = SubPath::new(vec![Anchor { p, h_in: p - Vec2::new(0.004, 0.0), h_out: p + Vec2::new(3.0, 0.0), kind: AnchorKind::Smooth }], false);
    assert_eq!(retract_tiny_handles(&mut sp, 0.01), 1);
    let a = sp.anchors[0];
    assert_eq!((a.h_in, a.kind), (p, AnchorKind::Corner));
    assert_eq!(a.h_out, p + Vec2::new(3.0, 0.0), "a long handle stays");
    assert_eq!(retract_tiny_handles(&mut sp, 0.01), 0);
}

#[test]
fn nearly_closed_paths_close() {
    let mut sp = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.005, 0.0)], false);
    assert!(close_nearly_closed(&mut sp, 0.01));
    assert!(sp.closed);
    assert_eq!(points(&sp), [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
    let mut apart = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (1.0, 0.0)], false);
    assert!(!close_nearly_closed(&mut apart, 0.01));
    assert!(!apart.closed);
    let mut two = line(&[(0.0, 0.0), (0.0, 0.0)], false);
    assert!(!close_nearly_closed(&mut two, 0.01), "two anchors make no shape");
}

#[test]
fn near_points_group_but_linked_and_equal_ones_do_not() {
    let pts: Vec<Point> = [(0.0, 0.0), (0.005, 0.0), (5.0, 5.0), (5.0, 5.0), (9.0, 9.0), (9.004, 9.0), (f64::NAN, 0.0)].map(Into::into).to_vec();
    let groups = near_point_groups(&pts, 0.01, |a, b| (a, b) == (4, 5));
    assert_eq!(groups, [vec![0, 1]], "equal points and linked ones are not snapped");
    assert!(near_point_groups(&pts, 0.0, |_, _| false).is_empty());
    assert!(near_point_groups(&pts, f64::NAN, |_, _| false).is_empty());
    // Far coordinates don't overflow the grid.
    let far: Vec<Point> = [(1e300, 1e300), (-1e300, 1e300)].map(Into::into).to_vec();
    assert!(near_point_groups(&far, 1e-300, |_, _| false).is_empty());
}

#[test]
fn rejoin_welds_pieces_within_the_gap() {
    // Three pieces of one zig-zag, the middle one drawn backwards, plus one too far away.
    let pieces = [
        line(&[(0.0, 0.0), (10.0, 0.0)], false),
        line(&[(20.0, 10.0), (10.05, 0.0)], false),
        line(&[(20.0, 10.0), (30.0, 10.0)], false),
        line(&[(31.0, 10.0), (40.0, 10.0)], false),
    ];
    let out = rejoin(&pieces, 0.1);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].members, [0, 1, 2]);
    assert_eq!(points(&out[0].path), [(0.0, 0.0), (10.0, 0.0), (20.0, 10.0), (30.0, 10.0)]);
    assert!(!out[0].path.closed);
    // With a wider gap the last piece joins too.
    assert_eq!(rejoin(&pieces, 1.0)[0].members, [0, 1, 2, 3]);
    // Nothing within the gap: nothing joins.
    assert!(rejoin(&pieces[2..], 0.5).is_empty());
}

#[test]
fn rejoin_takes_the_straightest_continuation_and_closes_loops() {
    // Piece 0 ends at the origin; piece 1 goes straight on, piece 2 turns back.
    let pieces = [line(&[(-10.0, 0.0), (0.0, 0.0)], false), line(&[(0.0, 0.0), (0.0, 10.0)], false), line(&[(0.0, 0.0), (10.0, 0.0)], false)];
    let out = rejoin(&pieces, 0.1);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].members, [0, 2]);
    // Four sides of a square close into one path.
    let sides = [
        line(&[(0.0, 0.0), (10.0, 0.0)], false),
        line(&[(10.0, 0.0), (10.0, 10.0)], false),
        line(&[(0.0, 10.0), (10.0, 10.0)], false),
        line(&[(0.0, 10.0), (0.0, 0.0)], false),
    ];
    let out = rejoin(&sides, 0.1);
    assert_eq!(out.len(), 1);
    assert!(out[0].path.closed);
    assert_eq!(out[0].path.anchors.len(), 4);
    // Closed, single-anchor and non-finite pieces stay out; bad gaps join nothing.
    let odd = [line(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)], true), line(&[(1.0, 0.0)], false), line(&[(f64::NAN, 0.0), (0.0, 1.0)], false)];
    assert!(rejoin(&odd, 1.0).is_empty());
    assert!(rejoin(&sides, f64::NAN).is_empty() && rejoin(&sides, -1.0).is_empty());
}
