use std::collections::HashSet;

use vectorcraft_geom::{PathData, Point, Rect, SubPath};

use super::super::{Blank, Folded, FoldedPanel, Panel, Pose, Stack};
use super::*;

fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> SubPath {
    SubPath::polyline(&[Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], true)
}

fn panel(subpaths: Vec<SubPath>) -> Panel {
    Panel { outline: PathData::new(subpaths), area: 0.0 }
}

/// A blank of `panels` and a fold that places each at `poses`, in the order given.
fn scene_of(panels: Vec<Panel>, poses: Vec<Pose>) -> (Blank, Folded) {
    let n = panels.len();
    let folded = Folded {
        base: 0,
        panels: poses.into_iter().map(|pose| FoldedPanel { pose, local: pose, parent: None, depth: 0 }).collect(),
        order: (0..n).collect(),
        hinges: vec![],
        collisions: vec![],
        stack: Stack { root: 0, parent: vec![None; n], order: (0..n).collect(), toward_parent: vec![None; n] },
        origin: [0.0; 3],
    };
    (Blank { panels, hinges: vec![], holes: vec![] }, folded)
}

/// Facing the printed side, 100 × 100 pixels, one pixel a point, the flat square 0..100 filling it.
fn front() -> Camera {
    Camera { width: 100, height: 100, rotate_x: 0.0, rotate_y: 0.0, scale: 1.0, pan: [0.0; 2], perspective: 0.0, center: [50.0, -50.0, 0.0] }
}

fn moved(z: f64) -> Pose {
    Pose { t: [0.0, 0.0, z], ..Pose::IDENTITY }
}

fn pixel(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = (y * img.width + x) as usize * 4;
    [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2], img.pixels[i + 3]]
}

/// A 2 × 2 print of the square 0..100: red, green on top; blue, white below.
fn quads() -> Print {
    let px = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 255]];
    Print::new(Rect::new(0.0, 0.0, 100.0, 100.0), 2, 2, px.concat()).expect("a print")
}

const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn a_flat_panel_takes_the_print_s_color_under_each_pixel() {
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0)])], vec![Pose::IDENTITY]);
    let print = quads();
    let img = render(&Scene { blank: &b, folded: &f, print: Some(&print), inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
    let shade = 0.6 + 0.4 * 0.74;
    let lit = |c: f64| (c * shade).round() as u8;
    assert_eq!(pixel(&img, 5, 5), [lit(255.0), 0, 0, 255], "top left is red");
    assert_eq!(pixel(&img, 94, 5), [0, lit(255.0), 0, 255], "top right is green");
    assert_eq!(pixel(&img, 5, 94), [0, 0, lit(255.0), 255], "bottom left is blue");
    assert_eq!(img.panel_at(50, 50), Some(0));
    // Where the print has no art, the board's printed side shows.
    let empty = Print::new(Rect::new(0.0, 0.0, 100.0, 100.0), 1, 1, vec![0; 4]).expect("a print");
    let img = render(&Scene { blank: &b, folded: &f, print: Some(&empty), inside: [0, 0, 0, 255], outside: [200, 100, 50, 255], camera: front() })
        .expect("drawn");
    assert_eq!(pixel(&img, 50, 50), [lit(200.0), lit(100.0), lit(50.0), 255]);
}

#[test]
fn the_board_is_opaque_whatever_the_alpha_of_its_colors() {
    let shade = 0.6 + 0.4 * 0.74;
    let lit = |c: f64| (c * shade).round() as u8;
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0)])], vec![Pose::IDENTITY]);
    for a in [0, 128] {
        let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: [200, 100, 50, a], camera: front() })
            .expect("drawn");
        assert_eq!(pixel(&img, 50, 50), [lit(200.0), lit(100.0), lit(50.0), 255], "outside alpha {a}");
    }
    // A transparent inside color draws opaque too, on the nearer of two panels seen from the back.
    let whole = || panel(vec![square(0.0, 0.0, 100.0, 100.0)]);
    let (b, f) = scene_of(vec![whole(), whole()], vec![Pose::IDENTITY, moved(5.0)]);
    let camera = Camera { rotate_y: 180.0, ..front() };
    let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 200, 0, 0], outside: WHITE, camera }).expect("drawn");
    let [r, g, bl, a] = pixel(&img, 50, 50);
    assert!(r == 0 && bl == 0 && g > 100 && a == 255, "{:?}", [r, g, bl, a]);
}

#[test]
fn a_print_squeezed_only_up_and_down_on_screen_samples_a_halved_level() {
    // 64 × 64, rows alternating black and white: halved, they average to gray.
    let mut px = vec![];
    for y in 0..64 {
        for _ in 0..64 {
            px.extend(if y % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
        }
    }
    let print = Print::new(Rect::new(0.0, 0.0, 100.0, 100.0), 64, 64, px).expect("a print");
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0)])], vec![Pose::IDENTITY]);
    // Tipped 80°: about 5.8 points a pixel up and down, 1 across.
    let camera = Camera { rotate_x: 80.0, ..front() };
    let img = render(&Scene { blank: &b, folded: &f, print: Some(&print), inside: [0; 4], outside: WHITE, camera }).expect("drawn");
    let reds: Vec<u8> = (0..img.height)
        .flat_map(|y| (0..img.width).map(move |x| (x, y)))
        .filter(|&(x, y)| img.panel_at(x, y).is_some())
        .map(|(x, y)| pixel(&img, x, y)[0])
        .collect();
    assert!(!reds.is_empty());
    let (lo, hi) = (reds.iter().min().copied().unwrap_or(0), reds.iter().max().copied().unwrap_or(0));
    assert!(hi - lo <= 2, "an even gray, not black and white rows: {lo} to {hi}");
}

#[test]
fn the_back_of_a_panel_shows_the_inside_color() {
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0)])], vec![Pose::IDENTITY]);
    let print = quads();
    let camera = Camera { rotate_y: 180.0, ..front() };
    let img = render(&Scene { blank: &b, folded: &f, print: Some(&print), inside: [0, 200, 0, 255], outside: WHITE, camera }).expect("drawn");
    let [r, g, bl, a] = pixel(&img, 50, 50);
    assert!(r == 0 && bl == 0 && g > 100 && a == 255, "{:?}", [r, g, bl, a]);
}

#[test]
fn the_nearer_of_two_panels_shows_and_a_tie_goes_to_the_later_one() {
    let whole = || panel(vec![square(0.0, 0.0, 100.0, 100.0)]);
    let draw = |z: f64| {
        let (b, f) = scene_of(vec![whole(), whole()], vec![Pose::IDENTITY, moved(z)]);
        render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn")
    };
    assert_eq!(draw(5.0).panel_at(50, 50), Some(1), "panel 1 is in front");
    assert_eq!(draw(-5.0).panel_at(50, 50), Some(0), "panel 1 is behind");
    assert_eq!(draw(0.0).panel_at(50, 50), Some(1), "a tie goes to the panel later in the order");
    // Panels that cross: each shows where it is nearer.
    let tilted = Pose { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.0, 1.0]], t: [0.0, 0.0, -25.0] };
    let (b, f) = scene_of(vec![whole(), whole()], vec![Pose::IDENTITY, tilted]);
    let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
    assert_eq!(img.panel_at(20, 50), Some(0), "the tilted panel is behind on the left");
    assert_eq!(img.panel_at(80, 50), Some(1), "and in front on the right");
}

#[test]
fn nearly_parallel_panels_that_cross_use_depth_on_both_sides() {
    let angle = 5e-10f64;
    let (sin, cos) = angle.sin_cos();
    let tilted = Pose { m: [[cos, 0.0, sin], [0.0, 1.0, 0.0], [-sin, 0.0, cos]], t: [50.0 * (1.0 - cos), 0.0, 50.0 * sin] };
    let whole = || panel(vec![square(0.0, 0.0, 100.0, 100.0)]);
    for (poses, left, right) in [(vec![Pose::IDENTITY, tilted], 1, 0), (vec![tilted, Pose::IDENTITY], 0, 1)] {
        let (b, f) = scene_of(vec![whole(), whole()], poses);
        let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
        assert_eq!(img.panel_at(25, 50), Some(left));
        assert_eq!(img.panel_at(75, 50), Some(right));
    }
}

#[test]
fn of_two_parallel_panels_the_nearer_shows_however_close() {
    // 2 and 2.0001 points deep: a hair more than `TIE` apart once subtracted, though 2 + `TIE`
    // equals 2.0001. The panels lie in different planes, so the nearer shows whichever is drawn
    // first.
    let whole = || panel(vec![square(0.0, 0.0, 100.0, 100.0)]);
    for (near, poses) in [(1, vec![moved(2.0), moved(2.0001)]), (0, vec![moved(2.0001), moved(2.0)])] {
        let (b, f) = scene_of(vec![whole(), whole()], poses);
        assert_eq!(planes(&f, &b).iter().collect::<HashSet<_>>().len(), 2, "two planes");
        let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
        assert_eq!(img.panel_at(50, 50), Some(near));
    }
}

#[test]
fn a_scene_whose_panels_claim_any_depth_still_draws() {
    let whole = || panel(vec![square(0.0, 0.0, 100.0, 100.0)]);
    let (b, mut f) = scene_of(vec![whole(), whole()], vec![moved(5.0), moved(5.0)]);
    for p in &mut f.panels {
        p.depth = usize::MAX;
    }
    let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
    assert!(img.panel_at(50, 50).is_some());
}

#[test]
fn a_flap_folded_flat_lies_behind_the_print_at_180_and_in_front_at_minus_180() {
    // A 100 × 60 blank with a crease at x = 60: the flap (x 60 to 100) folds over onto the base.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    // Folds a hair short of flat lie on the same side, everywhere within a tie of the base.
    for (angle, flap_in_front) in [(180.0, false), (-180.0, true), (179.9999, false), (-179.9999, true)] {
        let lines = [
            super::super::FoldLine {
                path: poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)], true),
                kind: super::super::LineKind::Cut,
                key: 1,
                angle: None,
            },
            super::super::FoldLine {
                path: poly(&[(60.0, 0.0), (60.0, 60.0)], false),
                kind: super::super::LineKind::Crease,
                key: 2,
                angle: Some(angle),
            },
        ];
        let (blanks, issues) = super::super::blanks(&lines, 0.1);
        assert!(issues.is_empty(), "{issues:?}");
        let b = &blanks[0];
        let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
        let flap = 1 - f.base;
        let views = [
            (front(), true),
            (Camera { rotate_y: 180.0, ..front() }, false),
            (Camera { perspective: 60.0, ..front() }, true),
            (Camera { rotate_y: 180.0, perspective: 60.0, ..front() }, false),
        ];
        for (camera, seen_from_print) in views {
            let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
            // The point x 40, y 30 of the blank, where the flap lies over the base.
            let x = if seen_from_print { 40 } else { 60 };
            let want = if flap_in_front == seen_from_print { flap } else { f.base };
            assert_eq!(img.panel_at(x, 30), Some(want), "{angle}°, seen from the print: {seen_from_print}, {camera:?}");
        }
    }
}

#[test]
fn a_hinge_left_at_0_degrees_keeps_its_panel_level_with_its_parent() {
    // A 100 × 60 blank: the base (x 0 to 60), a strip (60 to 65) left flat at 0°, and a flap (65 to
    // 100) folded back at 180°, which lies behind the base from x 30 to 60.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let crease = |x: f64, key: u64, angle: f64| super::super::FoldLine {
        path: poly(&[(x, 0.0), (x, 60.0)], false),
        kind: super::super::LineKind::Crease,
        key,
        angle: Some(angle),
    };
    let lines = [
        super::super::FoldLine {
            path: poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)], true),
            kind: super::super::LineKind::Cut,
            key: 1,
            angle: None,
        },
        crease(60.0, 2, 0.0),
        crease(65.0, 3, 180.0),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
    assert_eq!(img.panel_at(45, 30), Some(f.base), "the base is in front of the flap behind it");
}

#[test]
fn a_flap_folded_back_over_a_folded_panel_lies_between_it_and_the_base() {
    // A 110 × 60 strip: the base (x 0 to 60), a panel (60 to 100) folded back under the base at
    // 180°, and a flap (100 to 110) folded back at 180° from that panel, between it and the base, at
    // x 20 to 30.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let crease = |x: f64, key: u64| super::super::FoldLine {
        path: poly(&[(x, 0.0), (x, 60.0)], false),
        kind: super::super::LineKind::Crease,
        key,
        angle: Some(180.0),
    };
    let lines = [
        super::super::FoldLine {
            path: poly(&[(0.0, 0.0), (110.0, 0.0), (110.0, 60.0), (0.0, 60.0)], true),
            kind: super::super::LineKind::Cut,
            key: 1,
            angle: None,
        },
        crease(60.0, 2),
        crease(100.0, 3),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let middle = f.panels.iter().position(|p| p.parent == Some(f.base)).expect("the panel folded from the base");
    let draw = |camera| render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    assert_eq!(draw(front()).panel_at(25, 30), Some(f.base), "the base, from the printed side");
    assert_eq!(draw(Camera { rotate_y: 180.0, ..front() }).panel_at(75, 30), Some(middle), "the folded panel, from the back");
    // Folded about the flap instead, the blank only moves: the flap (x 100 to 110 now) still lies
    // between the other two, the 60 pt panel over it on the printed side.
    let tip = super::super::panel_at(b, Point::new(105.0, 30.0)).expect("the flap");
    let f = super::super::fold(b, Some(tip), 1.0, 0.1).expect("folds");
    let draw = |camera| render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    let at_tip = Camera { center: [105.0, -30.0, 0.0], ..front() };
    let wide = super::super::panel_at(b, Point::new(30.0, 30.0)).expect("the 60 pt panel");
    assert_eq!(draw(at_tip).panel_at(50, 50), Some(wide), "the 60 pt panel, from the printed side");
    assert_eq!(draw(Camera { rotate_y: 180.0, ..at_tip }).panel_at(50, 50), Some(middle), "the folded panel, from the back");
}

#[test]
fn a_strip_far_from_the_origin_stacks_the_same_about_any_base() {
    // The strip of the test above turned 45° and moved to about (3,000,000, 3,000,000) pt, folded
    // about its 10 pt flap: the 60 pt panel still lies over the flap on the printed side.
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let at = |x: f64, y: f64| Point::new(3_000_000.0 + (x - y) * s, 3_000_000.0 + (x + y) * s);
    let poly =
        |pts: &[(f64, f64)], closed: bool| PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| at(x, y)).collect::<Vec<_>>(), closed));
    let crease = |x: f64, key: u64| super::super::FoldLine {
        path: poly(&[(x, 0.0), (x, 60.0)], false),
        kind: super::super::LineKind::Crease,
        key,
        angle: Some(180.0),
    };
    let lines = [
        super::super::FoldLine {
            path: poly(&[(0.0, 0.0), (110.0, 0.0), (110.0, 60.0), (0.0, 60.0)], true),
            kind: super::super::LineKind::Cut,
            key: 1,
            angle: None,
        },
        crease(60.0, 2),
        crease(100.0, 3),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let panel = |x: f64, y: f64| super::super::panel_at(b, at(x, y)).expect("a panel");
    let (wide, middle, tip) = (panel(30.0, 30.0), panel(80.0, 30.0), panel(105.0, 30.0));
    let f = super::super::fold(b, Some(tip), 1.0, 0.1).expect("folds");
    let draw = |camera| render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    let c = at(105.0, 30.0);
    let at_tip = Camera { center: [c.x, -c.y, 0.0], ..front() };
    assert_eq!(draw(at_tip).panel_at(50, 50), Some(wide), "the 60 pt panel, from the printed side");
    assert_eq!(draw(Camera { rotate_y: 180.0, ..at_tip }).panel_at(50, 50), Some(middle), "the folded panel, from the back");
}

#[test]
fn a_flat_stack_far_from_the_origin_stays_in_one_plane() {
    // A strip from x = 3,000,000 pt: the base (60 pt wide), a 40 pt panel folded back under it at
    // 180° and a 35 pt flap folded back at 180° between the two.
    let x0 = 3_000_000.0;
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x0 + x, y)).collect::<Vec<_>>(), closed))
    };
    let crease = |x: f64, key: u64| super::super::FoldLine {
        path: poly(&[(x, 0.0), (x, 60.0)], false),
        kind: super::super::LineKind::Crease,
        key,
        angle: Some(180.0),
    };
    let lines = [
        super::super::FoldLine {
            path: poly(&[(0.0, 0.0), (135.0, 0.0), (135.0, 60.0), (0.0, 60.0)], true),
            kind: super::super::LineKind::Cut,
            key: 1,
            angle: None,
        },
        crease(60.0, 2),
        crease(100.0, 3),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let middle = super::super::panel_at(b, Point::new(x0 + 80.0, 30.0)).expect("the middle panel");
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    assert_eq!(planes(&f, b).iter().collect::<HashSet<_>>().len(), 1, "one plane");
    let draw = |rotate_y| {
        let camera = Camera { rotate_y, center: [x0 + 40.0, -30.0, 0.0], ..front() };
        render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn").panel_at(50, 50)
    };
    assert_eq!((draw(0.0), draw(180.0)), (Some(f.base), Some(middle)));
}

#[test]
fn a_panel_parallel_to_the_base_a_hair_behind_it_goes_by_depth() {
    // A strip 30 pt tall: the base (x 0 to 100), a 20 pt panel folded back at 179.9999° and a 30 pt
    // flap at 0.0001° off it, parallel to the base and about 0.000035 pt behind it; then the same
    // strip 100,000 pt to the right; then at 179.999999° and 0.000001°, about 0.00000035 pt behind,
    // 3,000,000 pt to the right; then with a 1 pt middle panel, about 0.0000000175 pt behind, at the
    // origin and 3,000,000 pt to the right.
    for (dx, back, w) in
        [(0.0, 0.0001, 20.0), (100_000.0, 0.0001, 20.0), (3_000_000.0, 0.000001, 20.0), (0.0, 0.000001, 1.0), (3_000_000.0, 0.000001, 1.0)]
    {
        parallel_a_hair_behind(dx, back, w);
    }
}

fn parallel_a_hair_behind(dx: f64, back: f64, w: f64) {
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x + dx, y)).collect::<Vec<_>>(), closed))
    };
    let crease = |x: f64, key: u64, angle: f64| super::super::FoldLine {
        path: poly(&[(x, 0.0), (x, 30.0)], false),
        kind: super::super::LineKind::Crease,
        key,
        angle: Some(angle),
    };
    let lines = [
        super::super::FoldLine {
            path: poly(&[(0.0, 0.0), (130.0 + w, 0.0), (130.0 + w, 30.0), (0.0, 30.0)], true),
            kind: super::super::LineKind::Cut,
            key: 1,
            angle: None,
        },
        crease(100.0, 2, 180.0 - back),
        crease(100.0 + w, 3, back),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.0001);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let at = Point::new(115.0 + w + dx, 15.0);
    let flap = super::super::panel_at(b, at).expect("the flap");
    let f = super::super::fold(b, None, 1.0, 0.0001).expect("folds");
    let m = f.panels[flap].pose.place(at);
    assert!(m[2] < 0.0 && m[2] > -1e-4, "the flap lies a hair behind the base: {m:?}");
    let draw = |rotate_y| {
        let camera = Camera { width: 100, height: 60, rotate_y, center: [m[0], -15.0, 0.0], ..front() };
        render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn").panel_at(50, 30)
    };
    assert_eq!((draw(0.0), draw(180.0)), (Some(f.base), Some(flap)), "at x + {dx}, a {w} pt middle panel");
}

#[test]
fn a_small_blank_folded_near_flat_draws_the_same_far_from_the_origin() {
    // A 4 pt strip, its second half folded back at 179.999999° so it lies a hair behind the first,
    // seen at a three-quarter turn with and without perspective: every pixel shows the same panel
    // at the origin and 3,000,000 pt right and down.
    let draw = |at: f64, perspective: f64| {
        let poly = |pts: &[(f64, f64)], closed: bool| {
            PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x + at, y + at)).collect::<Vec<_>>(), closed))
        };
        let lines = [
            super::super::FoldLine {
                path: poly(&[(0.0, 0.0), (4.0, 0.0), (4.0, 1.0), (0.0, 1.0)], true),
                kind: super::super::LineKind::Cut,
                key: 1,
                angle: None,
            },
            super::super::FoldLine {
                path: poly(&[(2.0, 0.0), (2.0, 1.0)], false),
                kind: super::super::LineKind::Crease,
                key: 2,
                angle: Some(179.999999),
            },
        ];
        let (blanks, issues) = super::super::blanks(&lines, 0.0001);
        assert!(issues.is_empty(), "{issues:?}");
        let b = &blanks[0];
        let base = super::super::panel_at(b, Point::new(0.5 + at, 0.5 + at)).expect("the base");
        let f = super::super::fold(b, Some(base), 1.0, 0.0001).expect("folds");
        let camera = Camera { rotate_x: 35.0, rotate_y: 40.0, scale: 40.0, perspective, center: [1.0 + at, -0.5 - at, 0.0], ..front() };
        let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
        let shown: Vec<Option<bool>> =
            (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).map(|(x, y)| img.panel_at(x, y).map(|p| p == base)).collect();
        assert!(shown.contains(&Some(true)), "the base shows");
        shown
    };
    for perspective in [0.0, 40.0] {
        assert!(draw(0.0, perspective) == draw(3_000_000.0, perspective), "perspective {perspective}");
    }
}

#[test]
fn a_blank_lying_flat_takes_its_ranks_from_the_order_alone() {
    // A strip of 64 panels, each crease at 180°, not yet folded: no panel overlaps another, so no
    // layer is worked out.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let mut lines = vec![super::super::FoldLine {
        path: poly(&[(0.0, 0.0), (640.0, 0.0), (640.0, 10.0), (0.0, 10.0)], true),
        kind: super::super::LineKind::Cut,
        key: 1,
        angle: None,
    }];
    for i in 1..64u32 {
        let x = f64::from(i) * 10.0;
        lines.push(super::super::FoldLine {
            path: poly(&[(x, 0.0), (x, 10.0)], false),
            kind: super::super::LineKind::Crease,
            key: u64::from(i) + 1,
            angle: Some(180.0),
        });
    }
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let f = super::super::fold(&blanks[0], None, 0.0, 0.1).expect("folds");
    let ranks = tie_ranks(&f, &front(), &planes(&f, &blanks[0]));
    assert_eq!(ranks, (1..=64).collect::<Vec<u16>>());
}

#[test]
fn round_a_loop_of_panels_the_stack_follows_the_hinges_that_placed_them() {
    // A 100 × 100 blank cut into four 50 pt squares by four creases, one per side of the middle
    // point. The crease that closes the loop when folded about the largest panel folds at 180° and
    // the rest at 0°; folded about a panel on that crease, the crease is in the tree and its flap
    // lies flat behind the panel.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let lines = |flat_key: Option<u64>| {
        let crease = |from: (f64, f64), to: (f64, f64), key: u64| super::super::FoldLine {
            path: poly(&[from, to], false),
            kind: super::super::LineKind::Crease,
            key,
            angle: Some(if Some(key) == flat_key { 180.0 } else { 0.0 }),
        };
        vec![
            super::super::FoldLine {
                path: poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true),
                kind: super::super::LineKind::Cut,
                key: 1,
                angle: None,
            },
            crease((50.0, 0.0), (50.0, 50.0), 2),
            crease((50.0, 50.0), (50.0, 100.0), 3),
            crease((0.0, 50.0), (50.0, 50.0), 4),
            crease((50.0, 50.0), (100.0, 50.0), 5),
        ]
    };
    let (blanks, issues) = super::super::blanks(&lines(None), 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let level = super::super::fold(&blanks[0], None, 1.0, 0.1).expect("folds");
    let closing = level.hinges.iter().position(|h| !h.tree).expect("a hinge closes the loop");
    let key = *blanks[0].hinges[closing].keys.first().expect("a line");
    let (blanks, issues) = super::super::blanks(&lines(Some(key)), 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let h = &b.hinges[closing];
    assert_eq!(h.keys, [key]);
    let f = super::super::fold(b, Some(h.a), 1.0, 0.1).expect("folds");
    assert!(f.hinges[closing].tree);
    // The panel's middle, in pixels: the blank's top left square is pixels 0 to 50.
    let middle = |p: usize| {
        let c = b.panels[p].outline.bounds().expect("bounds").center();
        (c.x as u32, c.y as u32)
    };
    let (x, y) = middle(h.a);
    let draw = |camera| render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    assert_eq!(draw(front()).panel_at(x, y), Some(h.a), "the panel, from its printed side");
    assert_eq!(draw(Camera { rotate_y: 180.0, ..front() }).panel_at(100 - x, y), Some(h.b), "the flap, from the back");
}

#[test]
fn round_a_flat_loop_the_stack_is_the_same_whichever_tree_reaches_the_flaps() {
    // A 2 × 2 grid of panels joined by 0° creases (the largest at the bottom left), and two flaps
    // folded back at 180° off the top right panel: the first (x 100 to 130) under that panel, the
    // tip (130 to 140) between the two. Folded about the tip, the tree reaches the grid another way,
    // and the stack stays as it is about the largest panel.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let crease = |from: (f64, f64), to: (f64, f64), key: u64, angle: f64| super::super::FoldLine {
        path: poly(&[from, to], false),
        kind: super::super::LineKind::Crease,
        key,
        angle: Some(angle),
    };
    let lines = [
        super::super::FoldLine {
            path: poly(&[(0.0, 0.0), (140.0, 0.0), (140.0, 40.0), (100.0, 40.0), (100.0, 100.0), (0.0, 100.0)], true),
            kind: super::super::LineKind::Cut,
            key: 1,
            angle: None,
        },
        crease((60.0, 0.0), (60.0, 40.0), 2, 0.0),
        crease((60.0, 40.0), (60.0, 100.0), 3, 0.0),
        crease((0.0, 40.0), (60.0, 40.0), 4, 0.0),
        crease((60.0, 40.0), (100.0, 40.0), 5, 0.0),
        crease((100.0, 0.0), (100.0, 40.0), 6, 180.0),
        crease((130.0, 0.0), (130.0, 40.0), 7, 180.0),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let at = |x: f64, y: f64| super::super::panel_at(b, Point::new(x, y)).expect("a panel");
    let (corner, under, tip) = (at(80.0, 20.0), at(115.0, 20.0), at(135.0, 20.0));
    let f = super::super::fold(b, Some(tip), 1.0, 0.1).expect("folds");
    let draw = |camera| render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    let at_tip = Camera { center: [135.0, -20.0, 0.0], ..front() };
    assert_eq!(draw(at_tip).panel_at(50, 50), Some(corner), "the top right panel, from the printed side");
    assert_eq!(draw(Camera { rotate_y: 180.0, ..at_tip }).panel_at(50, 50), Some(under), "the first flap, from the back");
}

#[test]
fn two_flaps_folded_flat_behind_the_base_show_opposite_ones_from_each_side() {
    // The base (x 30 to 80) with a window (52 to 58), a flap on each side folded back at 180°: the
    // left one to x 30 to 60, the right one to 50 to 80, so both lie behind the window.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let lines = [
        line(&[(0.0, 0.0), (110.0, 0.0), (110.0, 60.0), (0.0, 60.0)], true, Cut, 1, None),
        line(&[(52.0, 20.0), (58.0, 20.0), (58.0, 40.0), (52.0, 40.0)], true, Cut, 2, None),
        line(&[(30.0, 0.0), (30.0, 60.0)], false, Crease, 3, Some(180.0)),
        line(&[(80.0, 0.0), (80.0, 60.0)], false, Crease, 4, Some(180.0)),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let draw = |camera| render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    let through_window = draw(front()).panel_at(55, 30);
    let from_back = draw(Camera { rotate_y: 180.0, ..front() }).panel_at(45, 30);
    let flaps: Vec<usize> = (0..b.panels.len()).filter(|&i| i != f.base).collect();
    assert!(through_window.is_some_and(|p| flaps.contains(&p)), "{through_window:?}");
    assert!(from_back.is_some_and(|p| flaps.contains(&p)), "{from_back:?}");
    assert_ne!(through_window, from_back);
}

#[test]
fn a_flap_split_by_a_0_degree_crease_stacks_as_one_against_another_flap() {
    // The base (x 30 to 90), a right flap (90 to 140) folded back to 40 to 90, and a left flap (0 to
    // 30) folded back to 30 to 60 in two parts joined at 0° (x 15), both parts over the right flap.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let lines = [
        line(&[(0.0, 0.0), (140.0, 0.0), (140.0, 60.0), (0.0, 60.0)], true, Cut, 1, None),
        line(&[(30.0, 0.0), (30.0, 60.0)], false, Crease, 2, Some(180.0)),
        line(&[(90.0, 0.0), (90.0, 60.0)], false, Crease, 3, Some(180.0)),
        line(&[(15.0, 0.0), (15.0, 60.0)], false, Crease, 4, Some(0.0)),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let camera = Camera { width: 140, center: [70.0, -50.0, 0.0], rotate_y: 180.0, ..front() };
    let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    // From the back the image is mirrored: x on the blank is 140 - x in the image.
    let starting_at = |x0: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x0).abs() < 1e-6));
    let right_flap = starting_at(90.0).expect("the right flap");
    // Folded back about x 30, the left flap's part from 15 to 30 lies at 30 to 45 and its part from
    // 0 to 15 at 45 to 60.
    let (inner, outer) = (starting_at(15.0).expect("the left flap's inner part"), starting_at(0.0).expect("its outer part"));
    let (near, far) = (img.panel_at(140 - 42, 30), img.panel_at(140 - 52, 30));
    // Either flap may be in front, but the same one at both samples.
    let right = (near, far) == (Some(right_flap), Some(right_flap));
    let left = (near, far) == (Some(inner), Some(outer));
    assert!(right || left, "{near:?} {far:?}: right flap {right_flap}, left flap {inner} and {outer}");
}

#[test]
fn a_tip_folded_on_a_flap_stays_on_that_flap_s_side_of_another_flap() {
    // Left to right: a tip (x 0 to 20), a left flap (20 to 70), the base (70 to 140) and a right flap
    // (140 to 205). Both flaps fold back at 180° behind the base (to 70 to 120 and 75 to 140), and
    // the tip folds at -180° on the left flap (to 100 to 120), so the three overlap from 100 to 120.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let lines = [
        line(&[(0.0, 0.0), (205.0, 0.0), (205.0, 60.0), (0.0, 60.0)], true, Cut, 1, None),
        line(&[(20.0, 0.0), (20.0, 60.0)], false, Crease, 2, Some(-180.0)),
        line(&[(70.0, 0.0), (70.0, 60.0)], false, Crease, 3, Some(180.0)),
        line(&[(140.0, 0.0), (140.0, 60.0)], false, Crease, 4, Some(180.0)),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let starts_at = |x: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x).abs() < 1e-6)).expect("a panel");
    let (tip, left, right) = (starts_at(0.0), starts_at(20.0), starts_at(140.0));
    let camera = Camera { width: 210, center: [105.0, -50.0, 0.0], rotate_y: 180.0, ..front() };
    let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    // From the back the image is mirrored: x on the blank is 210 - x in the image.
    let (flaps_only, with_tip) = (img.panel_at(210 - 90, 30), img.panel_at(210 - 110, 30));
    if flaps_only == Some(right) {
        assert_eq!(with_tip, Some(right), "the right flap covers the left flap and the tip on it");
    } else {
        assert_eq!((flaps_only, with_tip), (Some(left), Some(tip)), "the left flap, and the tip on it, cover the right flap");
    }
}

#[test]
fn a_tip_folded_back_toward_a_folded_panel_s_print_covers_it_from_the_back() {
    // The base (x 0 to 60), a panel (60 to 100) folded back at 180° to 20 to 60, and a tip (100 to
    // 110) folded at -180° onto that panel's printed side, to 20 to 30, behind it from the front.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    // The same with the tip a hair short of flat, and with an 80 pt panel under a 100 pt base.
    for (middle, tip_angle) in [(40.0, -180.0), (40.0, -179.9999), (40.0, -179.999), (80.0, -179.9999), (1000.0, -179.999)] {
        let base = middle + 20.0;
        let end = base + middle + 10.0;
        let lines = [
            line(&[(0.0, 0.0), (end, 0.0), (end, 60.0), (0.0, 60.0)], true, Cut, 1, None),
            line(&[(base, 0.0), (base, 60.0)], false, Crease, 2, Some(180.0)),
            line(&[(base + middle, 0.0), (base + middle, 60.0)], false, Crease, 3, Some(tip_angle)),
        ];
        let (blanks, issues) = super::super::blanks(&lines, 0.1);
        assert!(issues.is_empty(), "{issues:?}");
        let b = &blanks[0];
        let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
        let tip = b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - (base + middle)).abs() < 1e-6)).expect("the tip");
        let back = Camera { rotate_y: 180.0, ..front() };
        let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: back }).expect("drawn");
        assert_eq!(img.panel_at(100 - 25, 30), Some(tip), "{middle} pt, {tip_angle}°");
        let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
        assert_eq!(img.panel_at(25, 30), Some(f.base), "{middle} pt, {tip_angle}°");
    }
}

#[test]
fn overlapping_lid_flaps_keep_their_order_as_the_view_turns_on_one_side_of_the_lid() {
    // A box strip: a left flap (x 0 to 60), a wall (60 to 100), the base (100 to 200), a wall (200 to
    // 240) and a right flap (240 to 300). The walls fold 90° down and the flaps 90° in, so the flaps
    // meet in one plane 40 pt below the base and overlap from x 140 to 160.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let crease = |x: f64, key| line(&[(x, 0.0), (x, 60.0)], false, Crease, key, Some(90.0));
    let lines = [
        line(&[(0.0, 0.0), (300.0, 0.0), (300.0, 60.0), (0.0, 60.0)], true, Cut, 1, None),
        crease(60.0, 2),
        crease(100.0, 3),
        crease(200.0, 4),
        crease(240.0, 5),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    // Looking up at the lid from below, a little to one side and then the other, at its middle.
    let shown = |rotate_y: f64| {
        let camera = Camera { width: 200, rotate_y, center: [150.0, -30.0, -40.0], ..front() };
        render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn").panel_at(100, 50)
    };
    let (left, right) = (shown(175.0), shown(185.0));
    assert!(left.is_some_and(|p| p != f.base), "{left:?}");
    assert_eq!(left, right);
}

#[test]
fn flaps_folded_onto_a_panel_keep_their_order_as_the_panel_tilts_either_way() {
    // The base (x 0 to 100, y -50 to 110) and a panel (100 to 200, 0 to 60) on a crease at x = 100,
    // with a flap above (y -40 to 0) and one below (60 to 100), each cut free of the base and folded
    // at -180° onto the panel's printed side, where they overlap from y 20 to 40.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let shown = |tilt: f64| {
        let outline = [(0.0, -50.0), (100.0, -50.0), (100.0, -40.0), (200.0, -40.0), (200.0, 100.0), (100.0, 100.0), (100.0, 110.0), (0.0, 110.0)];
        let lines = [
            line(&outline, true, Cut, 1, None),
            line(&[(100.0, -40.0), (100.0, 0.0)], false, Cut, 2, None),
            line(&[(100.0, 60.0), (100.0, 100.0)], false, Cut, 3, None),
            line(&[(100.0, 0.0), (100.0, 60.0)], false, Crease, 4, Some(tilt)),
            line(&[(100.0, 0.0), (200.0, 0.0)], false, Crease, 5, Some(-180.0)),
            line(&[(100.0, 60.0), (200.0, 60.0)], false, Crease, 6, Some(-180.0)),
        ];
        let (blanks, issues) = super::super::blanks(&lines, 0.1);
        assert!(issues.is_empty(), "{issues:?}");
        let b = &blanks[0];
        let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
        let camera = Camera { width: 200, center: [150.0, -30.0, 0.0], ..front() };
        let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
        let flap = |y: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.y0 - y).abs() < 1e-6 && r.x0 > 99.0));
        let shown = img.panel_at(100, 50);
        let above = flap(-40.0).expect("the upper flap exists");
        let below = flap(60.0).expect("the lower flap exists");
        if shown == Some(above) {
            "above"
        } else if shown == Some(below) {
            "below"
        } else {
            "neither"
        }
    };
    let (up, down) = (shown(1.0), shown(-1.0));
    assert_ne!(up, "neither");
    assert_eq!(up, down);
}

#[test]
fn a_flap_folded_onto_a_wall_standing_at_90_degrees_covers_its_print() {
    // The base (x 0 to 60), a wall (60 to 100) folded 90° and a flap (100 to 130) folded at -180°
    // onto the wall's printed side, where it covers the wall from 10 to 40 pt out from the base.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let lines = [
        line(&[(0.0, 0.0), (130.0, 0.0), (130.0, 60.0), (0.0, 60.0)], true, Cut, 1, None),
        line(&[(60.0, 0.0), (60.0, 60.0)], false, Crease, 2, Some(90.0)),
        line(&[(100.0, 0.0), (100.0, 60.0)], false, Crease, 3, Some(-180.0)),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let starts_at = |x: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x).abs() < 1e-6)).expect("a panel");
    let (wall, flap) = (starts_at(60.0), starts_at(100.0));
    let mut sides = vec![];
    for rotate_y in [90.0, -90.0] {
        // Facing the wall from one side and then the other, the middle row of the view.
        let camera = Camera { width: 200, height: 100, rotate_y, center: [60.0, -30.0, 0.0], ..front() };
        let printed = map_panel(&camera, &f.panels[wall].pose).is_some_and(|m| m.printed);
        let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
        let count = |p: usize| (0..img.width).filter(|&x| img.panel_at(x, 50) == Some(p)).count();
        let (walls, flaps) = (count(wall), count(flap));
        if printed {
            assert!(flaps > 25 && walls < 15, "from the wall's printed side the flap covers it: wall {walls} px, flap {flaps} px");
        } else {
            assert!(flaps == 0 && walls > 35, "from the back the wall covers the flap: wall {walls} px, flap {flaps} px");
        }
        sides.push(printed);
    }
    assert_ne!(sides[0], sides[1], "both sides of the wall are looked at");
}

#[test]
fn end_flaps_seen_straight_through_an_open_carton_show_opposite_ones_from_each_side() {
    // The base (x 40 to 140, y 0 to 60) and a side wall on each side (0 to 40 and 140 to 180), each
    // with an end flap (y 60 to 160) that folds in across the carton's end, so both flaps cover
    // the same 100 pt. Looking straight through that end, the base and walls are edge on.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let outline = [(0.0, 0.0), (180.0, 0.0), (180.0, 160.0), (140.0, 160.0), (140.0, 60.0), (40.0, 60.0), (40.0, 160.0), (0.0, 160.0)];
    let lines = [
        line(&outline, true, Cut, 1, None),
        line(&[(40.0, 0.0), (40.0, 60.0)], false, Crease, 2, Some(90.0)),
        line(&[(140.0, 0.0), (140.0, 60.0)], false, Crease, 3, Some(90.0)),
        line(&[(0.0, 60.0), (40.0, 60.0)], false, Crease, 4, Some(90.0)),
        line(&[(140.0, 60.0), (180.0, 60.0)], false, Crease, 5, Some(90.0)),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let flap = |x: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x).abs() < 1e-6 && r.y0 > 59.0)).expect("a flap");
    let (left, right) = (flap(0.0), flap(140.0));
    let shown = |rotate_x: f64| {
        let camera = Camera { width: 300, height: 300, rotate_x, center: [90.0, -60.0, 0.0], ..front() };
        let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
        let count =
            |p: usize| (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| img.panel_at(x, y) == Some(p)).count();
        (count(left), count(right))
    };
    let (one, other) = (shown(90.0), shown(-90.0));
    // Each view shows one flap over (nearly) the whole of the other.
    let winner = |(l, r): (usize, usize)| {
        if l > 10 * r.max(1) {
            Some(left)
        } else if r > 10 * l.max(1) {
            Some(right)
        } else {
            None
        }
    };
    assert!(winner(one).is_some() && winner(other).is_some(), "the flaps cover the same end: {one:?} {other:?}");
    assert_ne!(winner(one), winner(other), "{one:?} {other:?}");
}

#[test]
fn a_flap_looped_round_onto_the_base_shows_from_the_side_it_came_from() {
    // A strip: the base (x 0 to 100) and panels 40, 60, 40 and 80 pt wide, folded 90°, -90°, -90°
    // and -90°, so the last one comes round and lies on the base's plane over part of it.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let lines = [
        line(&[(0.0, 0.0), (320.0, 0.0), (320.0, 60.0), (0.0, 60.0)], true, Cut, 1, None),
        line(&[(100.0, 0.0), (100.0, 60.0)], false, Crease, 2, Some(90.0)),
        line(&[(140.0, 0.0), (140.0, 60.0)], false, Crease, 3, Some(-90.0)),
        line(&[(200.0, 0.0), (200.0, 60.0)], false, Crease, 4, Some(-90.0)),
        line(&[(240.0, 0.0), (240.0, 60.0)], false, Crease, 5, Some(-90.0)),
    ];
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let flap = b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - 240.0).abs() < 1e-6)).expect("the flap");
    // The panels between the base and the flap all lie behind the base's printed side.
    for (i, (panel, folded)) in b.panels.iter().zip(&f.panels).enumerate() {
        let r = panel.outline.bounds().expect("bounds");
        let z = folded.pose.place(Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0))[2];
        if i != f.base && i != flap {
            assert!(z < -1.0, "panel {i} at z {z}");
        }
    }
    // At x 90 the flap lies on the base: from the front the base covers it, from the back it
    // covers the base.
    let at = |rotate_y: f64, x: u32| {
        let camera = Camera { width: 400, height: 100, rotate_y, center: [100.0, -30.0, 0.0], ..front() };
        render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn").panel_at(x, 50)
    };
    assert_eq!(at(0.0, 200 - 10), Some(f.base));
    assert_eq!(at(180.0, 200 + 10), Some(flap));
}

#[test]
fn a_strip_wrapped_round_a_wall_lands_its_flap_on_the_print_side_it_came_from() {
    // A strip 60 pt tall: the base (x 0 to 100) and panels 20, 20, 40, 40 and 20 pt wide, then a
    // flap (240 to 270, y 0 to 30), folded 90°, -90°, -90°, -90°, -90° and 90°. The 20 pt wall the
    // flap folds from stands on the printed side of the base, and the flap lands on the base at x
    // 50 to 80. With `tab`, the wall also runs on past the flap's hinge (to x 340, y 30 to 60),
    // through the base's plane, which moves its middle to the other side of that plane.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    // With `cutout` of 5, a 10 pt hole on the middle of each crease breaks every hinge in two; with
    // a larger one, the hole runs to within 2 pt of each end, as wide as `cutout` either side.
    let shown = |tab: bool, cutout: f64| {
        let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
        use super::super::LineKind::{Crease, Cut};
        let mut lines = if tab {
            vec![
                line(&[(0.0, 0.0), (270.0, 0.0), (270.0, 30.0), (340.0, 30.0), (340.0, 60.0), (0.0, 60.0)], true, Cut, 1, None),
                line(&[(240.0, 30.0), (270.0, 30.0)], false, Cut, 2, None),
            ]
        } else {
            vec![line(&[(0.0, 0.0), (270.0, 0.0), (270.0, 30.0), (240.0, 30.0), (240.0, 60.0), (0.0, 60.0)], true, Cut, 1, None)]
        };
        let creases =
            [(100.0, 60.0, 90.0), (120.0, 60.0, -90.0), (140.0, 60.0, -90.0), (180.0, 60.0, -90.0), (220.0, 60.0, -90.0), (240.0, 30.0, 90.0)];
        for (i, (x, high, a)) in creases.into_iter().enumerate() {
            let key = 10 * (i as u64 + 1);
            if cutout > 0.0 {
                let (lo, hi) = if cutout > 5.0 { (2.0, high - 2.0) } else { (high / 2.0 - 5.0, high / 2.0 + 5.0) };
                let w = cutout.min(5.0);
                lines.push(line(&[(x - w, lo), (x + w, lo), (x + w, hi), (x - w, hi)], true, Cut, key, None));
                lines.push(line(&[(x, 0.0), (x, lo)], false, Crease, key + 1, Some(a)));
                lines.push(line(&[(x, hi), (x, high)], false, Crease, key + 2, Some(a)));
            } else {
                lines.push(line(&[(x, 0.0), (x, high)], false, Crease, key, Some(a)));
            }
        }
        let (blanks, issues) = super::super::blanks(&lines, 0.1);
        assert!(issues.is_empty(), "{issues:?}");
        let b = &blanks[0];
        let starts_at = |x: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x).abs() < 1e-6)).expect("a panel");
        let (wall, flap) = (starts_at(220.0), starts_at(240.0));
        let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
        let middle = |i: usize| {
            let r = b.panels[i].outline.bounds().expect("bounds");
            f.panels[i].pose.place(Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0))
        };
        let m = middle(flap);
        assert!(m[2].abs() < 1e-6 && (m[0] - 65.0).abs() < 1e-6, "the flap lies on the base: {m:?}");
        assert_eq!(middle(wall)[2] > 0.0, !tab, "the wall's middle is on the printed side without the tab only");
        // The flap's row at y 5, at x 65, from the front and from the back (mirrored).
        let at = |rotate_y: f64, px: u32| {
            let camera = Camera { width: 400, height: 100, rotate_y, center: [100.0, -30.0, 0.0], ..front() };
            render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn").panel_at(px, 25)
        };
        let named = |p: Option<usize>| match p {
            Some(p) if p == f.base => "base",
            Some(p) if p == flap => "flap",
            _ => "other",
        };
        (named(at(0.0, 200 - 35)), named(at(180.0, 200 + 35)))
    };
    assert_eq!(shown(false, 0.0), ("flap", "base"));
    assert_eq!(shown(true, 0.0), ("flap", "base"), "the side next to the hinge decides, not the wall's middle");
    assert_eq!(shown(false, 5.0), ("flap", "base"), "a hinge broken by a cutout");
    assert_eq!(shown(true, 5.0), ("flap", "base"), "both");
    assert_eq!(shown(true, 6.0), ("flap", "base"), "hinges joined only near their ends");
}

#[test]
fn a_flap_that_comes_down_onto_the_base_at_a_shallow_angle_lands_on_the_side_it_came_from() {
    // A strip 30 pt tall: the base (x 0 to `base`), a panel 10 pt wide folded down behind it, one
    // 20 pt wide back level, one up through the base's plane to `drop` in front of it, a ramp
    // `ramp` wide that comes back down to the base's plane over its width, and a flap (30 pt) that
    // lies on the base from where the ramp ends.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let shown = |base: f64, ramp: f64, drop: f64| {
        // To the millionth of a degree a line keeps, and the drop that ramp comes down by.
        let tilt = super::super::kept_angle((drop / ramp).asin().to_degrees());
        let drop = ramp * tilt.to_radians().sin();
        let up = base + 30.0;
        let creases = [(base, 90.0), (base + 10.0, -90.0), (up, -90.0), (up + 10.0 + drop, -90.0 - tilt), (up + 10.0 + drop + ramp, tilt)];
        let end = up + 10.0 + drop + ramp + 30.0;
        let mut lines = vec![super::super::FoldLine {
            path: poly(&[(0.0, 0.0), (end, 0.0), (end, 30.0), (0.0, 30.0)], true),
            kind: super::super::LineKind::Cut,
            key: 1,
            angle: None,
        }];
        for (i, (x, a)) in creases.into_iter().enumerate() {
            lines.push(super::super::FoldLine {
                path: poly(&[(x, 0.0), (x, 30.0)], false),
                kind: super::super::LineKind::Crease,
                key: 2 + i as u64,
                angle: Some(a),
            });
        }
        let (blanks, issues) = super::super::blanks(&lines, 0.0001);
        assert!(issues.is_empty(), "{issues:?}");
        let b = &blanks[0];
        let flap = b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - (end - 30.0)).abs() < 1e-6)).expect("the flap");
        let f = super::super::fold(b, None, 1.0, 0.0001).expect("folds");
        let r = b.panels[flap].outline.bounds().expect("bounds");
        let m = f.panels[flap].pose.place(Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0));
        let x = base + 20.0 - ramp - 15.0;
        assert!(m[2].abs() < 1e-6 && (m[0] - x).abs() < 1e-2 && x > 0.0, "the flap lies on the base at {x}: {m:?}");
        // The flap's middle, from the front and from the back.
        let at = |rotate_y: f64| {
            let camera = Camera { width: 100, height: 60, rotate_y, center: [m[0], -15.0, 0.0], ..front() };
            render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn").panel_at(50, 30)
        };
        (at(0.0) == Some(flap), at(180.0) == Some(f.base))
    };
    assert_eq!(shown(100.0, 40.0, 0.01), (true, true), "a 40 pt ramp 0.01 pt high");
    assert_eq!(shown(600.0, 500.0, 0.01), (true, true), "a 500 pt ramp 0.01 pt high");
    assert_eq!(shown(2100.0, 2000.0, 0.01), (true, true), "a 2,000 pt ramp 0.01 pt high");
    assert_eq!(shown(600.0, 500.0, 0.001), (true, true), "a 500 pt ramp 0.001 pt high");
}

#[test]
fn a_flap_brought_back_by_folds_all_near_flat_lands_on_the_side_its_ramp_came_from() {
    // A strip 30 pt tall: the base (x 0 to 1,000) and panels 100, 300, 200 and 50 pt wide, folded
    // `tilt`, 180°, -2 `tilt` and `tilt`: at 0.04° the 200 pt panel comes back down onto the base's
    // plane from about 0.14 pt off it, at 0.00001° from 0.00004 pt, and the last panel lies on the
    // base.
    for tilt in [0.04, 0.00001] {
        flap_brought_back_by_folds_all_near_flat(tilt);
    }
}

fn flap_brought_back_by_folds_all_near_flat(tilt: f64) {
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let creases = [(1000.0, tilt), (1100.0, 180.0), (1400.0, -2.0 * tilt), (1600.0, tilt)];
    let mut lines = vec![super::super::FoldLine {
        path: poly(&[(0.0, 0.0), (1650.0, 0.0), (1650.0, 30.0), (0.0, 30.0)], true),
        kind: super::super::LineKind::Cut,
        key: 1,
        angle: None,
    }];
    for (i, (x, a)) in creases.into_iter().enumerate() {
        lines.push(super::super::FoldLine {
            path: poly(&[(x, 0.0), (x, 30.0)], false),
            kind: super::super::LineKind::Crease,
            key: 2 + i as u64,
            angle: Some(a),
        });
    }
    let (blanks, issues) = super::super::blanks(&lines, 0.0001);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let starts_at = |x: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x).abs() < 1e-6)).expect("a panel");
    let (ramp, flap) = (starts_at(1400.0), starts_at(1600.0));
    let f = super::super::fold(b, None, 1.0, 0.0001).expect("folds");
    let middle = |i: usize| {
        let r = b.panels[i].outline.bounds().expect("bounds");
        f.panels[i].pose.place(Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0))
    };
    let (m, above) = (middle(flap), middle(ramp)[2]);
    assert!(m[2].abs() < 1e-3 && (0.0..1000.0).contains(&m[0]) && above.abs() > tilt * 0.25, "the flap lies on the base: {m:?}, the ramp at {above}");
    let at = |rotate_y: f64| {
        let camera = Camera { width: 100, height: 60, rotate_y, center: [m[0], -15.0, 0.0], ..front() };
        render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn").panel_at(50, 30)
    };
    let (from_front, from_back) = (at(0.0), at(180.0));
    if above > 0.0 {
        assert_eq!((from_front, from_back), (Some(flap), Some(f.base)), "{tilt}°");
    } else {
        assert_eq!((from_front, from_back), (Some(f.base), Some(flap)), "{tilt}°");
    }
}

#[test]
fn a_strip_wrapped_round_onto_a_panel_that_is_not_the_base_lands_on_the_side_it_came_from() {
    // The strip of the test above, with a larger base (x -150 to 0) on the left that the strip's
    // first panel (0 to 100) stands up from at -90°. The flap lands on that panel's printed side.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let line = |pts: &[(f64, f64)], closed: bool, kind, key, angle| super::super::FoldLine { path: poly(pts, closed), kind, key, angle };
    use super::super::LineKind::{Crease, Cut};
    let mut lines = vec![line(&[(-150.0, 0.0), (270.0, 0.0), (270.0, 30.0), (240.0, 30.0), (240.0, 60.0), (-150.0, 60.0)], true, Cut, 1, None)];
    lines.push(line(&[(0.0, 0.0), (0.0, 60.0)], false, Crease, 2, Some(-90.0)));
    for (i, (x, a)) in [(100.0, 90.0), (120.0, -90.0), (140.0, -90.0), (180.0, -90.0), (220.0, -90.0)].into_iter().enumerate() {
        lines.push(line(&[(x, 0.0), (x, 60.0)], false, Crease, 3 + i as u64, Some(a)));
    }
    lines.push(line(&[(240.0, 0.0), (240.0, 30.0)], false, Crease, 9, Some(90.0)));
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let starts_at = |x: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x).abs() < 1e-6)).expect("a panel");
    let (panel, flap) = (starts_at(0.0), starts_at(240.0));
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    assert_eq!(f.base, starts_at(-150.0));
    let mut seen = vec![];
    for rotate_y in [90.0, -90.0] {
        // Facing the panel from one side and then the other: how many pixels show the flap.
        let camera = Camera { width: 400, height: 200, rotate_y, center: [0.0, -30.0, 0.0], ..front() };
        let printed = map_panel(&camera, &f.panels[panel].pose).is_some_and(|m| m.printed);
        let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
        let flaps = (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| img.panel_at(x, y) == Some(flap)).count();
        seen.push((printed, flaps));
    }
    let (from_print, from_back) = if seen[0].0 { (seen[0].1, seen[1].1) } else { (seen[1].1, seen[0].1) };
    assert_ne!(seen[0].0, seen[1].0, "both sides of the panel are looked at");
    assert!(from_print > 600 && from_back == 0, "the flap shows on the panel's printed side only: {seen:?}");
}

#[test]
fn a_strip_rolled_up_45_times_keeps_the_base_in_front() {
    // A strip: the base (x 0 to 60) and 45 panels 50 pt wide, every crease folded back at 180°, so
    // each panel rolls under the one before it and all of them lie behind the base.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let end = 60.0 + 45.0 * 50.0;
    let mut lines = vec![super::super::FoldLine {
        path: poly(&[(0.0, 0.0), (end, 0.0), (end, 60.0), (0.0, 60.0)], true),
        kind: super::super::LineKind::Cut,
        key: 1,
        angle: None,
    }];
    lines.extend((0u32..45).map(|i| {
        let x = 60.0 + 50.0 * f64::from(i);
        super::super::FoldLine {
            path: poly(&[(x, 0.0), (x, 60.0)], false),
            kind: super::super::LineKind::Crease,
            key: 10 + u64::from(i),
            angle: Some(180.0),
        }
    }));
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let draw = |camera| render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    assert_eq!(draw(front()).panel_at(40, 30), Some(f.base));
}

#[test]
fn hinges_left_at_0_degrees_before_two_folds_back_do_not_use_up_layers() {
    // A 155 × 60 strip: the base (x 0 to 60), 45 strips 1 pt wide left flat at 0°, a panel (105 to
    // 145) folded back at 180° and a flap (145 to 155) folded back at 180° from it, which lies
    // behind the flat strips at x 65 to 75.
    let poly = |pts: &[(f64, f64)], closed: bool| {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), closed))
    };
    let crease = |x: f64, key: u64, angle: f64| super::super::FoldLine {
        path: poly(&[(x, 0.0), (x, 60.0)], false),
        kind: super::super::LineKind::Crease,
        key,
        angle: Some(angle),
    };
    let mut lines = vec![super::super::FoldLine {
        path: poly(&[(0.0, 0.0), (155.0, 0.0), (155.0, 60.0), (0.0, 60.0)], true),
        kind: super::super::LineKind::Cut,
        key: 1,
        angle: None,
    }];
    lines.extend((0u32..45).map(|i| crease(60.0 + f64::from(i), 10 + u64::from(i), 0.0)));
    lines.push(crease(105.0, 2, 180.0));
    lines.push(crease(145.0, 3, 180.0));
    let (blanks, issues) = super::super::blanks(&lines, 0.1);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &blanks[0];
    let f = super::super::fold(b, None, 1.0, 0.1).expect("folds");
    let starts_at = |x: f64| b.panels.iter().position(|p| p.outline.bounds().is_some_and(|r| (r.x0 - x).abs() < 1e-6)).expect("a panel");
    let (strip, middle, flap) = (starts_at(70.0), starts_at(105.0), starts_at(145.0));
    let camera = Camera { width: 160, center: [80.0, -50.0, 0.0], ..front() };
    let img = render(&Scene { blank: b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
    let shown = img.panel_at(70, 30);
    assert_eq!(shown, Some(strip), "a flat strip, not the flap ({flap}) or the folded panel ({middle})");
}

#[test]
fn a_hole_in_a_panel_is_left_empty() {
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0), square(30.0, 30.0, 70.0, 70.0)])], vec![Pose::IDENTITY]);
    let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera: front() }).expect("drawn");
    assert_eq!(img.panel_at(50, 50), None);
    assert_eq!(pixel(&img, 50, 50), [0; 4]);
    assert_eq!(img.panel_at(10, 50), Some(0));
}

#[test]
fn a_panel_past_the_eye_at_a_wide_perspective_is_cut_at_the_near_plane() {
    // A long panel standing out of the page toward the viewer, through the eye.
    let toward = Pose { m: [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]], t: [0.0, -50.0, 0.0] };
    let (b, f) = scene_of(vec![panel(vec![square(0.0, -1000.0, 100.0, 1000.0)])], vec![toward]);
    for perspective in [1.0, 60.0, 160.0, 179.0] {
        let camera = Camera { perspective, scale: 8.0, rotate_x: 30.0, ..front() };
        let img = render(&Scene { blank: &b, folded: &f, print: None, inside: [0, 0, 0, 255], outside: WHITE, camera }).expect("drawn");
        let drawn = img.panels.iter().filter(|&&p| p != 0).count();
        assert!(drawn > 0, "{perspective}°: some of it shows");
        assert_eq!(img.pixels.len(), 100 * 100 * 4);
    }
}

#[test]
fn unusable_views_and_prints_are_refused() {
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0)])], vec![Pose::IDENTITY]);
    for camera in [
        Camera { width: 0, ..front() },
        Camera { height: MAX_SIDE + 1, ..front() },
        Camera { width: MAX_SIDE, height: MAX_PIXELS / MAX_SIDE + 1, ..front() },
        Camera { scale: 0.0, ..front() },
        Camera { rotate_x: f64::NAN, ..front() },
        Camera { center: [f64::INFINITY, 0.0, 0.0], ..front() },
    ] {
        assert!(render(&Scene { blank: &b, folded: &f, print: None, inside: [0; 4], outside: WHITE, camera }).is_none(), "{camera:?}");
    }
    let region = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert!(Print::new(region, 2, 2, vec![0; 15]).is_none(), "too few bytes");
    assert!(Print::new(region, 0, 2, vec![]).is_none());
    assert!(Print::new(region, MAX_SIDE + 1, 1, vec![0; (MAX_SIDE as usize + 1) * 4]).is_none());
    assert!(Print::new(Rect::new(0.0, 0.0, 0.0, 10.0), 1, 1, vec![0; 4]).is_none(), "an empty region");
    let p = Print::new(region, 8, 4, vec![255; 8 * 4 * 4]).expect("a print");
    assert_eq!(p.levels.iter().map(|l| (l.width, l.height)).collect::<Vec<_>>(), vec![(8, 4), (4, 2), (2, 1), (1, 1)]);
}

#[test]
fn halving_an_odd_sized_print_keeps_its_last_row_and_column() {
    // 3 × 3, transparent but for an opaque white last column and last row.
    let mut px = vec![0u8; 3 * 3 * 4];
    for i in [2, 5, 6, 7, 8] {
        px[i * 4..i * 4 + 4].copy_from_slice(&[255; 4]);
    }
    let p = Print::new(Rect::new(0.0, 0.0, 30.0, 30.0), 3, 3, px).expect("a print");
    let half = &p.levels[1];
    assert_eq!((half.width, half.height), (2, 2));
    // The bottom right pixel of the half level covers only the corner pixel.
    assert_eq!(&half.pixels[12..16], &[255; 4]);
    assert_eq!(&half.pixels[0..4], &[0; 4], "the top left covers four transparent pixels");
}

#[test]
fn a_print_far_smaller_on_screen_samples_a_halved_level() {
    // Columns alternate black and white: halved, they average to gray.
    let mut px = vec![];
    for _ in 0..4 {
        for x in 0..4 {
            px.extend(if x % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
        }
    }
    let print = Print::new(Rect::new(0.0, 0.0, 100.0, 100.0), 4, 4, px).expect("a print");
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0)])], vec![Pose::IDENTITY]);
    // 100 points in 2 pixels: 2 print pixels a view pixel.
    let camera = Camera { width: 2, height: 2, scale: 0.02, ..front() };
    let img = render(&Scene { blank: &b, folded: &f, print: Some(&print), inside: [0; 4], outside: WHITE, camera }).expect("drawn");
    let [r, ..] = pixel(&img, 0, 0);
    assert!((80..=150).contains(&r), "gray, not a black or white column: {r}");
}

#[test]
fn a_print_sixteen_times_smaller_on_screen_samples_a_level_halved_four_times() {
    // 64 × 64, columns in bands of 4 alternating black and white: halved twice, they still alternate;
    // halved four times, they average to gray.
    let mut px = vec![];
    for _ in 0..64 {
        for x in 0..64 {
            px.extend(if (x / 4) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
        }
    }
    let print = Print::new(Rect::new(0.0, 0.0, 100.0, 100.0), 64, 64, px).expect("a print");
    assert_eq!(print.levels.len(), 7, "64 down to 1");
    let (b, f) = scene_of(vec![panel(vec![square(0.0, 0.0, 100.0, 100.0)])], vec![Pose::IDENTITY]);
    // 100 points in 3 pixels: about 21 print pixels a view pixel.
    let camera = Camera { width: 3, height: 3, scale: 0.03, ..front() };
    let img = render(&Scene { blank: &b, folded: &f, print: Some(&print), inside: [0; 4], outside: WHITE, camera }).expect("drawn");
    let [r, ..] = pixel(&img, 0, 1);
    assert!((90..=140).contains(&r), "gray, not a black or white band: {r}");
}

#[test]
fn a_halved_level_weighs_an_odd_last_column_by_the_area_it_covers() {
    // 9 × 9, black but for a white last column: a ninth of the print, so the one-pixel level is
    // a ninth white (255 / 9 is about 28), not half.
    let mut px = vec![];
    for _ in 0..9 {
        for x in 0..9 {
            px.extend(if x == 8 { [255, 255, 255, 255] } else { [0, 0, 0, 255] });
        }
    }
    let print = Print::new(Rect::new(0.0, 0.0, 100.0, 100.0), 9, 9, px).expect("a print");
    assert_eq!(print.levels.iter().map(|l| (l.width, l.height)).collect::<Vec<_>>(), vec![(9, 9), (5, 5), (3, 3), (2, 2), (1, 1)]);
    assert_eq!(print.levels.last().map(|l| l.pixels.clone()), Some(vec![28, 28, 28, 255]));
}

#[test]
fn the_bounds_take_in_every_folded_panel() {
    let (b, f) =
        scene_of(vec![panel(vec![square(0.0, 0.0, 10.0, 10.0)]), panel(vec![square(0.0, 0.0, 10.0, 10.0)])], vec![Pose::IDENTITY, moved(-7.0)]);
    let (lo, hi) = bounds(&b, &f).expect("bounds");
    assert_eq!((lo, hi), ([0.0, -10.0, -7.0], [10.0, 0.0, 0.0]));
}
