use super::*;
use proptest::prelude::*;
use vectorcraft_geom::Rect;

const TOL: f64 = 0.1;

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn poly(points: &[(f64, f64)], closed: bool) -> PathData {
    PathData::single(SubPath::polyline(&points.iter().map(|&(x, y)| p(x, y)).collect::<Vec<_>>(), closed))
}

fn line(kind: LineKind, points: &[(f64, f64)], closed: bool, key: u64) -> FoldLine {
    FoldLine { path: poly(points, closed), kind, key, angle: None }
}

fn cut(points: &[(f64, f64)], key: u64) -> FoldLine {
    line(LineKind::Cut, points, true, key)
}

fn crease(a: (f64, f64), b: (f64, f64), key: u64) -> FoldLine {
    line(LineKind::Crease, &[a, b], false, key)
}

fn codes(issues: &[Issue]) -> Vec<&'static str> {
    issues.iter().map(|i| i.code.id()).collect()
}

fn areas(b: &Blank) -> Vec<i64> {
    let mut a: Vec<i64> = b.panels.iter().map(|p| (p.area * 1000.0).round() as i64).collect();
    a.sort_unstable();
    a
}

/// A tray: a 100 × 60 base with a 30 pt wall on each side, drawn as one closed cross, and the four
/// creases round the base (keys 10 to 13).
fn tray() -> Vec<FoldLine> {
    vec![
        cut(
            &[
                (30.0, 0.0),
                (130.0, 0.0),
                (130.0, 30.0),
                (160.0, 30.0),
                (160.0, 90.0),
                (130.0, 90.0),
                (130.0, 120.0),
                (30.0, 120.0),
                (30.0, 90.0),
                (0.0, 90.0),
                (0.0, 30.0),
                (30.0, 30.0),
            ],
            1,
        ),
        crease((30.0, 30.0), (130.0, 30.0), 10),
        crease((130.0, 30.0), (130.0, 90.0), 11),
        crease((130.0, 90.0), (30.0, 90.0), 12),
        crease((30.0, 90.0), (30.0, 30.0), 13),
    ]
}

/// The outline of a slotted box in the style of FEFCO 0201 (glue flap, four body panels 100, 60,
/// 100 and 60 wide and 80 high, 30 pt flaps top and bottom) with its 4 pt slots drawn as notches.
const NOTCHED: [(f64, f64); 30] = [
    (20.0, 0.0),
    (118.0, 0.0),
    (118.0, 30.0),
    (122.0, 30.0),
    (122.0, 0.0),
    (178.0, 0.0),
    (178.0, 30.0),
    (182.0, 30.0),
    (182.0, 0.0),
    (278.0, 0.0),
    (278.0, 30.0),
    (282.0, 30.0),
    (282.0, 0.0),
    (340.0, 0.0),
    (340.0, 140.0),
    (282.0, 140.0),
    (282.0, 110.0),
    (278.0, 110.0),
    (278.0, 140.0),
    (182.0, 140.0),
    (182.0, 110.0),
    (178.0, 110.0),
    (178.0, 140.0),
    (122.0, 140.0),
    (122.0, 110.0),
    (118.0, 110.0),
    (118.0, 140.0),
    (20.0, 140.0),
    (20.0, 110.0),
    (0.0, 110.0),
];

/// The slotted box's creases: the glue flap's and the body panels' (vertical), and the flaps'
/// (horizontal, from slot to slot).
fn box_creases() -> Vec<FoldLine> {
    let mut out = vec![];
    for (k, x) in [20.0, 120.0, 180.0, 280.0].into_iter().enumerate() {
        out.push(crease((x, 30.0), (x, 110.0), 100 + k as u64));
    }
    for (k, (x0, x1)) in [(20.0, 118.0), (122.0, 178.0), (182.0, 278.0), (282.0, 340.0)].into_iter().enumerate() {
        out.push(crease((x0, 30.0), (x1, 30.0), 110 + k as u64));
        out.push(crease((x0, 110.0), (x1, 110.0), 120 + k as u64));
    }
    out
}

fn notched_box() -> Vec<FoldLine> {
    let mut outline: Vec<(f64, f64)> = NOTCHED.to_vec();
    outline.extend([(0.0, 30.0), (20.0, 30.0)]);
    let mut lines = vec![cut(&outline, 1)];
    lines.extend(box_creases());
    lines
}

fn check(lines: &[FoldLine]) -> (Vec<Blank>, Vec<Issue>) {
    blanks(lines, TOL)
}

#[test]
fn a_tray_has_a_base_four_walls_and_four_hinges() {
    let (b, issues) = check(&tray());
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b.len(), 1);
    assert_eq!(areas(&b[0]), vec![1_800_000, 1_800_000, 3_000_000, 3_000_000, 6_000_000]);
    assert_eq!(b[0].hinges.len(), 4);
    for h in &b[0].hinges {
        assert!(h.straight && h.keys.len() == 1, "{h:?}");
        assert!((h.from.distance(h.to) - if h.keys[0] % 2 == 0 { 100.0 } else { 60.0 }).abs() < 1e-6, "{h:?}");
    }
    assert_eq!(base_panel(&b[0].panels).map(|i| (b[0].panels[i].area).round()), Some(6000.0));
}

#[test]
fn a_slotted_box_gives_the_same_panels_however_its_slots_and_outline_are_drawn() {
    let (notched, issues) = check(&notched_box());
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(notched.len(), 1);
    assert_eq!(notched[0].panels.len(), 13, "glue flap, four body panels, eight flaps");
    assert_eq!(notched[0].hinges.len(), 12);
    assert!(notched[0].holes.is_empty());

    // The slots as rectangles across a plain outline.
    let mut across =
        vec![cut(&[(20.0, 0.0), (340.0, 0.0), (340.0, 140.0), (20.0, 140.0), (20.0, 110.0), (0.0, 110.0), (0.0, 30.0), (20.0, 30.0)], 1)];
    for (k, x) in [120.0, 180.0, 280.0].into_iter().enumerate() {
        across.push(cut(&[(x - 2.0, -5.0), (x + 2.0, -5.0), (x + 2.0, 30.0), (x - 2.0, 30.0)], 2 + k as u64));
        across.push(cut(&[(x - 2.0, 110.0), (x + 2.0, 110.0), (x + 2.0, 145.0), (x - 2.0, 145.0)], 5 + k as u64));
    }
    across.extend(box_creases());
    let (b, issues) = check(&across);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b.len(), 1);
    assert_eq!(areas(&b[0]), areas(&notched[0]));
    assert_eq!(b[0].hinges.len(), 12);

    // The notched outline as four open paths, end to end.
    let mut outline: Vec<(f64, f64)> = NOTCHED.to_vec();
    outline.extend([(0.0, 30.0), (20.0, 30.0), (20.0, 0.0)]);
    let mut open: Vec<FoldLine> =
        [0..9, 8..17, 16..25, 24..33].into_iter().enumerate().map(|(k, r)| line(LineKind::Cut, &outline[r], false, 1 + k as u64)).collect();
    open.extend(box_creases());
    let (b, issues) = check(&open);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(areas(&b[0]), areas(&notched[0]));
    assert_eq!(b[0].hinges.len(), 12);

    // Input order does not change the result.
    let mut reversed = notched_box();
    reversed.reverse();
    let (b, _) = check(&reversed);
    assert_eq!(areas(&b[0]), areas(&notched[0]));
}

#[test]
fn a_round_tuck_flap_folds_like_any_panel() {
    let mut bp = BezPath::new();
    bp.move_to(p(0.0, 40.0));
    bp.line_to(p(0.0, 140.0));
    bp.line_to(p(100.0, 140.0));
    bp.line_to(p(100.0, 40.0));
    bp.curve_to(p(100.0, 10.0), p(80.0, 0.0), p(50.0, 0.0));
    bp.curve_to(p(20.0, 0.0), p(0.0, 10.0), p(0.0, 40.0));
    bp.close_path();
    let lines = vec![FoldLine { path: PathData::from_bezpath(&bp), kind: LineKind::Cut, key: 1, angle: None }, crease((0.0, 40.0), (100.0, 40.0), 2)];
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b[0].panels.len(), 2);
    assert_eq!(b[0].hinges.len(), 1);
}

#[test]
fn a_window_is_a_hole_in_its_panel() {
    let mut lines = tray();
    lines.push(cut(&[(60.0, 45.0), (100.0, 45.0), (100.0, 75.0), (60.0, 75.0)], 2));
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b[0].holes.len(), 1);
    assert_eq!(areas(&b[0]), vec![1_800_000, 1_800_000, 3_000_000, 3_000_000, 4_800_000]);
}

/// The tray's outline as one open path whose end stops `short` before its start.
fn tray_with_gap(short: f64) -> Vec<FoldLine> {
    let mut lines = tray();
    lines[0] = line(
        LineKind::Cut,
        &[
            (30.0 + short, 0.0),
            (130.0, 0.0),
            (130.0, 30.0),
            (160.0, 30.0),
            (160.0, 90.0),
            (130.0, 90.0),
            (130.0, 120.0),
            (30.0, 120.0),
            (30.0, 90.0),
            (0.0, 90.0),
            (0.0, 30.0),
            (30.0, 30.0),
            (30.0, 0.0),
        ],
        false,
        1,
    );
    lines
}

#[test]
fn a_gap_within_the_tolerance_is_closed_and_one_past_it_is_a_loose_end() {
    let (closed, _) = check(&tray());
    let (b, issues) = check(&tray_with_gap(0.05));
    assert_eq!(codes(&issues), vec!["gap"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![1]);
    assert_eq!(areas(&b[0]), areas(&closed[0]));
    assert_eq!(b[0].hinges.len(), 4);
    // The tolerance boundary.
    let (_, issues) = blanks(&tray_with_gap(0.05), 0.06);
    assert_eq!(codes(&issues), vec!["gap"]);
    let (b, issues) = blanks(&tray_with_gap(0.05), 0.04);
    assert!(codes(&issues).starts_with(&["open", "looseEnd", "looseEnd"]) || codes(&issues).starts_with(&["looseEnd"]), "{issues:?}");
    assert!(issues.iter().any(|i| i.code == IssueCode::LooseEnd && i.at.is_some_and(|a| a.distance(p(30.05, 0.0)) < 1e-9)), "{issues:?}");
    assert!(b.iter().all(|b| b.panels.len() < 5), "the outline does not close");
}

#[test]
fn no_cut_lines_or_no_enclosed_area_build_nothing() {
    let (b, issues) = check(&[crease((0.0, 0.0), (10.0, 0.0), 1)]);
    assert!(b.is_empty());
    assert_eq!(codes(&issues), vec!["noCut"]);
    let (b, issues) = check(&[]);
    assert!(b.is_empty());
    assert_eq!(codes(&issues), vec!["noCut"]);
    let (b, issues) = check(&[line(LineKind::Cut, &[(0.0, 0.0), (50.0, 0.0), (50.0, 50.0)], false, 1)]);
    assert!(b.is_empty());
    assert!(codes(&issues).contains(&"open") && codes(&issues).contains(&"looseEnd"), "{issues:?}");
    // A non-finite line is left out.
    let mut lines = tray();
    lines.push(line(LineKind::Crease, &[(0.0, 0.0), (f64::NAN, 1.0)], false, 9));
    let (b, issues) = check(&lines);
    assert!(issues.is_empty() && b[0].panels.len() == 5, "{issues:?}");
}

#[test]
fn a_cut_end_inside_the_blank_is_a_loose_end() {
    let mut lines = tray();
    lines.push(line(LineKind::Cut, &[(80.0, 0.0), (80.0, 20.0)], false, 2));
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["looseEnd"], "{issues:?}");
    assert_eq!((issues[0].keys.clone(), issues[0].at), (vec![2], Some(p(80.0, 20.0))));
    assert_eq!(b[0].panels.len(), 5, "a cut into a wall does not divide it");
    // A slit that ends on a crease is not loose: it cuts the wall in two.
    let mut lines = tray();
    lines.push(line(LineKind::Cut, &[(80.0, 0.0), (80.0, 30.0)], false, 2));
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!((b[0].panels.len(), b[0].hinges.len()), (6, 5));
}

#[test]
fn a_cut_line_that_comes_back_through_its_own_end_meets_it() {
    // From (80, 10) out to (90, 20), then back through (80, 10) to the outline at (70, 0): the
    // start lies on the line's own second segment, so it is not loose.
    let mut lines = tray();
    lines.push(line(LineKind::Cut, &[(80.0, 10.0), (90.0, 20.0), (70.0, 0.0)], false, 2));
    let (_, issues) = check(&lines);
    assert!(!codes(&issues).contains(&"looseEnd"), "{issues:?}");
    // A first segment shorter than the tolerance does not make the second one meet the end.
    let mut lines = tray();
    lines.push(line(LineKind::Cut, &[(90.0, 25.0), (90.05, 25.0), (80.0, 20.0), (80.0, 0.0)], false, 2));
    let (_, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["looseEnd"], "{issues:?}");
    assert_eq!(issues[0].at, Some(p(90.0, 25.0)));
}

#[test]
fn two_outlines_joined_by_a_cut_line_are_both_material() {
    let lines = vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1),
        cut(&[(150.0, 0.0), (180.0, 0.0), (180.0, 30.0), (150.0, 30.0)], 2),
        line(LineKind::Cut, &[(100.0, 10.0), (150.0, 10.0)], false, 3),
    ];
    let (b, issues) = check(&lines);
    assert_eq!(b.len(), 1, "one connected piece");
    assert_eq!(areas(&b[0]), vec![900_000, 10_000_000], "{issues:?}");
    assert!(b[0].holes.is_empty());
    // A slot drawn across the outline is still cut out, not made material where it sticks out.
    let lines =
        vec![cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1), cut(&[(48.0, -10.0), (52.0, -10.0), (52.0, 20.0), (48.0, 20.0)], 2)];
    let (b, _) = check(&lines);
    assert_eq!(b.len(), 1);
    assert_eq!(areas(&b[0]), vec![10_000_000 - 80_000]);
    // The same slot drawn as four open lines end to end, one of them backwards.
    let slot = [(48.0, -10.0), (52.0, -10.0), (52.0, 20.0), (48.0, 20.0)];
    let mut lines = vec![cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1)];
    lines.push(line(LineKind::Cut, &[slot[0], slot[1]], false, 2));
    lines.push(line(LineKind::Cut, &[slot[2], slot[1]], false, 3));
    lines.push(line(LineKind::Cut, &[slot[2], slot[3]], false, 4));
    lines.push(line(LineKind::Cut, &[slot[3], slot[0]], false, 5));
    let (b, issues) = check(&lines);
    assert_eq!(b.len(), 1, "{issues:?}");
    assert_eq!(areas(&b[0]), vec![10_000_000 - 80_000]);
    // And with a gap within the tolerance at one corner.
    lines[4] = line(LineKind::Cut, &[slot[3], (48.0, -9.95)], false, 5);
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["gap"], "{issues:?}");
    assert_eq!(b.len(), 1);
    assert_eq!(areas(&b[0]), vec![10_000_000 - 80_000]);
    // A slot right across the blank, with no corner inside the outline and no outline corner
    // inside the slot, cuts it in two.
    let lines = vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1),
        cut(&[(10.0, -210.0), (20.0, -210.0), (20.0, 110.0), (10.0, 110.0)], 2),
    ];
    let (b, _) = check(&lines);
    assert_eq!(b.len(), 1);
    assert_eq!(areas(&b[0]), vec![1_000_000, 8_000_000]);
}

#[test]
fn an_outline_drawn_twice_keeps_its_material() {
    let square = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    let (b, issues) = check(&[cut(&square, 1), cut(&square, 2)]);
    assert_eq!(codes(&issues), vec!["duplicate"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![1, 2]);
    assert_eq!(b.len(), 1);
    assert_eq!(areas(&b[0]), vec![10_000_000]);
    // The same two squares as the subpaths of one compound path.
    let mut twice = poly(&square, true);
    twice.subpaths.extend(poly(&square, true).subpaths);
    let (b, issues) = check(&[FoldLine { path: twice, kind: LineKind::Cut, key: 1, angle: None }]);
    assert_eq!(codes(&issues), vec!["duplicate"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![1]);
    assert_eq!(areas(&b[0]), vec![10_000_000]);
}

#[test]
fn a_large_tolerance_still_closes_nearly_touching_ends_and_short_outlines() {
    // Ends 0.0005 apart, closer than 1/10,000 of the tolerance.
    let nearly = line(LineKind::Cut, &[(0.0005, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)], false, 1);
    let (b, issues) = blanks(&[nearly], 10.0);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(areas(&b[0]), vec![10_000_000]);
    // An outline shorter than the tolerance, 0.05 pt open: a gap, and no loose end.
    let short = line(LineKind::Cut, &[(0.05, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)], false, 1);
    let (b, issues) = blanks(&[short], 10.0);
    assert_eq!(codes(&issues), vec!["gap"], "{issues:?}");
    assert_eq!(areas(&b[0]), vec![1000]);
}

#[test]
fn a_lookup_wider_than_the_occupied_cells_reads_only_those_cells() {
    // Two points 1e-9 apart make cells about 2.4e-13 wide: a lookup 1e-6 around one spans some
    // 10^14 cells, and finds both points by reading the two cells that hold them.
    let cells = Cells::new(vec![(Rect::from_points(p(0.0, 0.0), p(0.0, 0.0)), 1), (Rect::from_points(p(1e-9, 0.0), p(1e-9, 0.0)), 2)], 0.0, 4096.0);
    assert_eq!(cells.near(p(0.0, 0.0), 1e-6), vec![1, 2]);
    assert_eq!(cells.near(p(5.0, 5.0), 1e-6), Vec::<i32>::new());
    // A square open by 0.0000011 pt at a 10 pt tolerance.
    let square = line(LineKind::Cut, &[(0.0000011, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)], false, 1);
    let (b, issues) = blanks(&[square], 10.0);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b.len(), 1);
}

#[test]
fn a_window_of_open_lines_with_a_slit_to_its_corner_is_still_a_hole() {
    let outline = cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1);
    let window = [(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)];
    let slit = line(LineKind::Cut, &[(40.0, 0.0), (40.0, 40.0)], false, 2);
    let (closed, issues) = check(&[outline.clone(), slit.clone(), cut(&window, 3)]);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!((closed[0].holes.len(), areas(&closed[0])), (1, vec![10_000_000 - 400_000]));
    let mut lines = vec![outline, slit];
    for k in 0..4 {
        lines.push(line(LineKind::Cut, &[window[k], window[(k + 1) % 4]], false, 3 + k as u64));
    }
    let (open, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!((open[0].holes.len(), areas(&open[0])), (1, areas(&closed[0])));
    // The outline as four open lines too, the slit from its corner to the window's.
    let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    let mut lines: Vec<FoldLine> = (0..4).map(|k| line(LineKind::Cut, &[corners[k], corners[(k + 1) % 4]], false, 10 + k as u64)).collect();
    lines.push(line(LineKind::Cut, &[(0.0, 0.0), (40.0, 40.0)], false, 2));
    for k in 0..4 {
        lines.push(line(LineKind::Cut, &[window[k], window[(k + 1) % 4]], false, 3 + k as u64));
    }
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!((b[0].holes.len(), areas(&b[0])), (1, areas(&closed[0])));
    // A cut across the window of open lines, so that three lines meet at two of its corners.
    let mut lines = vec![cut(&corners, 1), line(LineKind::Cut, &[(40.0, 0.0), (40.0, 40.0)], false, 2)];
    for k in 0..4 {
        lines.push(line(LineKind::Cut, &[window[k], window[(k + 1) % 4]], false, 3 + k as u64));
    }
    lines.push(line(LineKind::Cut, &[(40.0, 40.0), (60.0, 60.0)], false, 20));
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!((b[0].holes.len(), areas(&b[0])), (1, areas(&closed[0])));
    // The outline and the window both as open lines, joined by two slits: the same as when both are
    // closed.
    let slits = [line(LineKind::Cut, &[(0.0, 0.0), (40.0, 40.0)], false, 30), line(LineKind::Cut, &[(100.0, 100.0), (60.0, 60.0)], false, 31)];
    let (want, want_issues) = check(&[cut(&corners, 1), cut(&window, 3), slits[0].clone(), slits[1].clone()]);
    let mut lines: Vec<FoldLine> = (0..4).map(|k| line(LineKind::Cut, &[corners[k], corners[(k + 1) % 4]], false, 10 + k as u64)).collect();
    for k in 0..4 {
        lines.push(line(LineKind::Cut, &[window[k], window[(k + 1) % 4]], false, 3 + k as u64));
    }
    lines.extend(slits);
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), codes(&want_issues), "{issues:?}");
    assert_eq!((b[0].holes.len(), areas(&b[0])), (want[0].holes.len(), areas(&want[0])));
    assert_eq!(want[0].holes.len(), 1);
}

#[test]
fn a_slot_drawn_across_the_outline_is_a_notch_not_a_hole() {
    let outline = || cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1);
    let (b, issues) = check(&[outline(), cut(&[(48.0, -10.0), (52.0, -10.0), (52.0, 20.0), (48.0, 20.0)], 2)]);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(b[0].holes.is_empty(), "{:?}", b[0].holes);
    let notched = cut(&[(0.0, 0.0), (48.0, 0.0), (48.0, 20.0), (52.0, 20.0), (52.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1);
    assert_eq!(areas(&b[0]), areas(&check(std::slice::from_ref(&notched)).0[0]));
    // A crease in the notch lies outside the blank, and one that runs on into it from a panel too.
    let slot = || cut(&[(48.0, -10.0), (52.0, -10.0), (52.0, 20.0), (48.0, 20.0)], 2);
    let (_, issues) = check(&[outline(), slot(), crease((49.0, 5.0), (51.0, 5.0), 3)]);
    assert_eq!(codes(&issues), vec!["outside"], "{issues:?}");
    let (_, issues) = check(&[outline(), slot(), crease((50.0, 5.0), (50.0, 100.0), 3)]);
    let (_, want) = check(&[notched, crease((50.0, 5.0), (50.0, 100.0), 3)]);
    assert_eq!(codes(&issues), codes(&want), "{issues:?}");
    assert!(codes(&issues).contains(&"outside"), "{issues:?}");
    // A cutout whose top side lies on the outline opens onto the outside directly.
    let flush = || cut(&[(48.0, 0.0), (52.0, 0.0), (52.0, 20.0), (48.0, 20.0)], 2);
    let (b, issues) = check(&[outline(), flush()]);
    assert!(b[0].holes.is_empty() && !codes(&issues).contains(&"ambiguous"), "{:?} {issues:?}", b[0].holes);
    let (_, issues) = check(&[outline(), flush(), crease((49.0, 5.0), (51.0, 5.0), 3)]);
    assert!(codes(&issues).contains(&"outside") && !codes(&issues).contains(&"ambiguous"), "{issues:?}");
    // A window joined to the outline by a slit stays a hole when a cut across it leaves a sliver.
    let window = || cut(&[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)], 4);
    let slit = || line(LineKind::Cut, &[(40.0, 0.0), (40.0, 40.0)], false, 5);
    let (b, _) = check(&[outline(), window(), slit(), line(LineKind::Cut, &[(40.0, 40.004), (60.0, 40.004)], false, 6)]);
    assert_eq!(b[0].holes.len(), 1, "{:?}", b[0].holes);
    // And a window 0.004 pt below the outline, joined to it at both corners, leaving a sliver.
    let near = cut(&[(40.0, 0.004), (60.0, 0.004), (60.0, 20.0), (40.0, 20.0)], 4);
    let joins = [line(LineKind::Cut, &[(40.0, 0.0), (40.0, 0.004)], false, 5), line(LineKind::Cut, &[(60.0, 0.0), (60.0, 0.004)], false, 6)];
    let (b, _) = check(&[outline(), near, joins[0].clone(), joins[1].clone()]);
    assert_eq!(b[0].holes.len(), 1, "{:?}", b[0].holes);
}

#[test]
fn many_small_outlines_of_open_lines_fit_the_gap_search() {
    // 784 unit squares 3 pt apart, each four open lines, at a tolerance of 0.5 pt.
    let mut lines = vec![];
    for i in 0..28u64 {
        for j in 0..28u64 {
            let (x, y) = (3.0 * i as f64, 3.0 * j as f64);
            let c = [(x, y), (x + 1.0, y), (x + 1.0, y + 1.0), (x, y + 1.0)];
            for k in 0..4 {
                lines.push(line(LineKind::Cut, &[c[k], c[(k + 1) % 4]], false, 4 * (28 * i + j) + k as u64));
            }
        }
    }
    let (b, issues) = blanks(&lines, 0.5);
    assert!(!codes(&issues).contains(&"tooComplex"), "{issues:?}");
    assert_eq!(b.len(), 784);
}

#[test]
fn thousands_of_cut_lines_on_top_of_each_other_are_too_complex_before_the_gap_search() {
    let lines: Vec<FoldLine> = (0..3000u64).map(|k| line(LineKind::Cut, &[(0.0, 0.0), (100.0, 0.0)], false, k)).collect();
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["tooComplex"], "{issues:?}");
    assert!(b.is_empty());
}

#[test]
fn cut_lines_that_cross_an_outline_of_open_lines_leave_every_cell_material() {
    // A 90 pt square whose sides are open lines ending every 30 pt, and four cut lines right across
    // it: nine pieces of material, as with a closed outline.
    let mut lines = vec![];
    for k in 0..3 {
        let (a, b) = (30.0 * k as f64, 30.0 * (k + 1) as f64);
        for (i, seg) in [[(a, 0.0), (b, 0.0)], [(90.0, a), (90.0, b)], [(b, 90.0), (a, 90.0)], [(0.0, b), (0.0, a)]].iter().enumerate() {
            lines.push(line(LineKind::Cut, seg, false, 10 + 4 * k as u64 + i as u64));
        }
    }
    let across = [[(30.0, 0.0), (30.0, 90.0)], [(60.0, 0.0), (60.0, 90.0)], [(0.0, 30.0), (90.0, 30.0)], [(0.0, 60.0), (90.0, 60.0)]];
    for (i, seg) in across.iter().enumerate() {
        lines.push(line(LineKind::Cut, seg, false, 30 + i as u64));
    }
    let mut closed = vec![cut(&[(0.0, 0.0), (90.0, 0.0), (90.0, 90.0), (0.0, 90.0)], 1)];
    closed.extend(lines.iter().skip(12).cloned());
    let (want, want_issues) = check(&closed);
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), codes(&want_issues), "{issues:?}");
    let pieces = |b: &[Blank]| b.iter().map(|x| (x.panels.len(), x.holes.len())).collect::<Vec<_>>();
    assert_eq!(pieces(&b), pieces(&want));
    assert_eq!(b.iter().map(|x| x.panels.len()).sum::<usize>(), 9);
    // With the crossing lines drawn in 30 pt pieces, the four pieces round the middle cell meet end
    // to end and count as one closed cut line, whether the outline is closed or drawn in pieces.
    let mut split = vec![];
    for (i, seg) in across.iter().enumerate() {
        let (a, b) = (seg[0], seg[1]);
        for k in 0..3 {
            let at = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            split.push(line(LineKind::Cut, &[at(k as f64 / 3.0), at((k + 1) as f64 / 3.0)], false, 40 + 3 * i as u64 + k as u64));
        }
    }
    let mut closed = vec![cut(&[(0.0, 0.0), (90.0, 0.0), (90.0, 90.0), (0.0, 90.0)], 1)];
    closed.extend(split.iter().cloned());
    let (want, want_issues) = check(&closed);
    let mut open: Vec<FoldLine> = lines.iter().take(12).cloned().collect();
    open.extend(split);
    let (b, issues) = check(&open);
    assert_eq!(codes(&issues), codes(&want_issues), "{issues:?}");
    assert_eq!(pieces(&b), pieces(&want));
}

#[test]
fn a_curved_crease_is_measured_at_its_farthest_point_from_its_line() {
    // A crease from corner to corner of a square: its curve strays about 10.1 pt from the diagonal,
    // where samples at quarters of it see 9.8 pt.
    let off = 35.0 / 2f64.sqrt();
    let mut bp = BezPath::new();
    bp.move_to(p(0.0, 0.0));
    bp.curve_to(p(10000.0 / 3.0 - off, 10000.0 / 3.0 + off), p(20000.0 / 3.0 + off, 20000.0 / 3.0 - off), p(10000.0, 10000.0));
    let lines = vec![
        cut(&[(0.0, 0.0), (10000.0, 0.0), (10000.0, 10000.0), (0.0, 10000.0)], 1),
        FoldLine { path: PathData::from_bezpath(&bp), kind: LineKind::Crease, key: 2, angle: None },
    ];
    let (b, issues) = blanks(&lines, 10.0);
    assert_eq!(codes(&issues), vec!["notStraight"], "{issues:?}");
    assert_eq!(b[0].hinges.len(), 1);
    let (_, issues) = blanks(&lines, 10.5);
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn a_grid_of_many_touching_outlines_is_classified_without_comparing_every_pair() {
    // 100 × 100 touching squares, each its own closed outline: one piece of 10,000 panels.
    let mut lines = vec![];
    for i in 0..100u64 {
        for j in 0..100u64 {
            let (x, y) = (10.0 * i as f64, 10.0 * j as f64);
            lines.push(cut(&[(x, y), (x + 10.0, y), (x + 10.0, y + 10.0), (x, y + 10.0)], 100 * i + j));
        }
    }
    let (b, issues) = check(&lines);
    assert!(b.is_empty());
    assert!(codes(&issues).contains(&"tooComplex"), "{issues:?}");
}

#[test]
fn open_cut_lines_far_from_the_origin_are_checked_without_overflow() {
    let lines = [
        line(LineKind::Cut, &[(1e13, 0.0), (1e13 + 4096.0, 0.0), (1e13 + 4096.0, 4096.0)], false, 1),
        line(LineKind::Cut, &[(1e13 + 4096.0, 4096.0), (1e13, 4096.0), (1e13, 0.0)], false, 2),
    ];
    let (b, issues) = check(&lines);
    assert!(b.len() <= 1, "{issues:?}");
}

#[test]
fn a_blank_in_the_hole_of_a_narrow_ring_is_a_blank_of_its_own() {
    // The ring's own area (1,900) is less than the hole's (8,100), but the square lies in the hole.
    let lines = vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1),
        cut(&[(5.0, 5.0), (95.0, 5.0), (95.0, 95.0), (5.0, 95.0)], 2),
        cut(&[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)], 3),
    ];
    let (b, issues) = check(&lines);
    assert_eq!(b.len(), 2, "{issues:?}");
    assert_eq!(codes(&issues), vec!["ambiguous"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![3]);
}

#[test]
fn too_many_segments_are_refused_before_the_lines_are_copied() {
    // Many lines that together pass the limit by one segment.
    let each: Vec<(f64, f64)> = (0..=64).map(|i| (i as f64, 0.0)).collect();
    let lines: Vec<FoldLine> = (0..=(MAX_SEGMENTS / 64) as u64).map(|k| line(LineKind::Cut, &each, false, k)).collect();
    assert!(lines.len() * 64 > MAX_SEGMENTS);
    let (b, issues) = check(&lines);
    assert!(b.is_empty());
    assert_eq!(codes(&issues), vec!["tooComplex"]);
}

#[test]
fn many_separate_blanks_each_with_a_window_are_told_apart() {
    // A 50 × 40 grid of 10 pt squares 5 pt apart, each with a 4 pt window and a crease beside it.
    let mut lines = vec![];
    for i in 0..50u64 {
        for j in 0..40u64 {
            let (x, y) = (15.0 * i as f64, 15.0 * j as f64);
            let k = 10 * (40 * i + j);
            lines.push(cut(&[(x, y), (x + 10.0, y), (x + 10.0, y + 10.0), (x, y + 10.0)], k));
            lines.push(cut(&[(x + 1.0, y + 1.0), (x + 5.0, y + 1.0), (x + 5.0, y + 5.0), (x + 1.0, y + 5.0)], k + 1));
            lines.push(crease((x + 7.0, y), (x + 7.0, y + 10.0), k + 2));
        }
    }
    let (blanks, issues) = check(&lines);
    assert!(issues.is_empty(), "{:?}", issues.first());
    assert_eq!(blanks.len(), 2000);
    assert!(blanks.iter().all(|b| b.panels.len() == 2 && b.hinges.len() == 1 && b.holes.len() == 1));
}

#[test]
fn many_separate_cut_lines_are_checked_by_the_cells_near_each_end() {
    // 2,000 short slits from the top edge of a long blank, each ending loose inside it.
    let mut lines = vec![cut(&[(0.0, 0.0), (4000.0, 0.0), (4000.0, 100.0), (0.0, 100.0)], 1)];
    for i in 0..2000u64 {
        let x = 1.0 + 2.0 * i as f64;
        lines.push(line(LineKind::Cut, &[(x, 0.0), (x, 50.0)], false, 10 + i));
    }
    let (_, issues) = check(&lines);
    let loose: Vec<&Issue> = issues.iter().filter(|i| i.code == IssueCode::LooseEnd).collect();
    assert_eq!(loose.len(), 2000);
    assert!(loose.iter().all(|i| i.at.is_some_and(|at| (at.y - 50.0).abs() < 1e-9)));
}

#[test]
fn too_many_segments_or_panels_are_too_complex() {
    // One point more than segments.
    let mut zigzag: Vec<(f64, f64)> = (0..=MAX_SEGMENTS + 1).map(|i| (i as f64 * 0.5, (i % 2) as f64)).collect();
    let (b, issues) = check(&[line(LineKind::Cut, &zigzag, false, 1)]);
    assert!(b.is_empty());
    assert_eq!(codes(&issues), vec!["tooComplex"]);
    zigzag.truncate(MAX_SEGMENTS + 1);
    let (_, issues) = check(&[line(LineKind::Cut, &zigzag, false, 1)]);
    assert!(!codes(&issues).contains(&"tooComplex"), "at the limit");
    // A grid of 16 × 16 panels is at the panel limit; 17 × 16 is past it.
    let grid = |cols: usize| {
        let mut lines = vec![cut(&[(0.0, 0.0), (cols as f64 * 10.0, 0.0), (cols as f64 * 10.0, 160.0), (0.0, 160.0)], 1)];
        for i in 1..cols {
            lines.push(crease((i as f64 * 10.0, 0.0), (i as f64 * 10.0, 160.0), 100 + i as u64));
        }
        for j in 1..16 {
            lines.push(crease((0.0, j as f64 * 10.0), (cols as f64 * 10.0, j as f64 * 10.0), 200 + j as u64));
        }
        lines
    };
    let (b, issues) = check(&grid(16));
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b[0].panels.len(), MAX_PANELS);
    let (b, issues) = check(&grid(17));
    assert!(b.is_empty());
    assert_eq!(codes(&issues), vec!["tooComplex"]);
}

#[test]
fn lines_on_top_of_each_other_are_reported() {
    let mut lines = tray();
    lines.push(crease((40.0, 0.0), (120.0, 0.0), 20));
    let (_, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["cutOverCrease"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![1, 20]);

    let mut lines = tray();
    lines.push(crease((130.0, 30.0), (30.0, 30.0), 20));
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["duplicate"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![10, 20]);
    assert_eq!(b[0].hinges.len(), 4);
    assert_eq!(b[0].hinges.iter().find(|h| h.keys.contains(&20)).map(|h| h.keys.clone()), Some(vec![10, 20]));

    // A perforation over a crease, lines of two kinds that fold.
    let mut lines = tray();
    lines.push(line(LineKind::Perforation, &[(130.0, 30.0), (30.0, 30.0)], false, 20));
    let (_, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["duplicate"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![10, 20]);

    // A crease drawn twice outside the blank is reported where it lies as well.
    let mut lines = tray();
    lines.push(crease((200.0, 0.0), (250.0, 0.0), 20));
    lines.push(crease((250.0, 0.0), (200.0, 0.0), 21));
    let (_, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["duplicate", "outside", "outside"], "{issues:?}");
    assert_eq!((issues[1].keys.clone(), issues[2].keys.clone()), (vec![20], vec![21]));
}

#[test]
fn a_crease_that_runs_past_the_outline_is_reported() {
    let outline = || cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], 1);
    // 5 pt past the outline at each end: the hinge is built, and the line is reported.
    let (b, issues) = check(&[outline(), crease((50.0, -5.0), (50.0, 55.0), 2)]);
    assert_eq!(codes(&issues), vec!["outside"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![2]);
    assert_eq!(b[0].hinges.len(), 1);
    // Past it by less than the tolerance: no issue.
    let (b, issues) = check(&[outline(), crease((50.0, -0.05), (50.0, 50.0), 2)]);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b[0].hinges.len(), 1);
    // A crease drawn round a sliver inside the blank does not run outside it.
    let (_, issues) = check(&[outline(), line(LineKind::Crease, &[(50.0, 10.0), (50.004, 10.0), (50.004, 40.0), (50.0, 40.0)], true, 2)]);
    assert!(!codes(&issues).contains(&"outside"), "{issues:?}");
}

#[test]
fn a_crease_that_ends_inside_a_panel_is_dangling() {
    let mut lines = tray();
    lines.push(crease((80.0, 30.0), (80.0, 60.0), 20));
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["danglingCrease"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![20]);
    assert_eq!(b[0].panels.len(), 5);
}

#[test]
fn a_hinge_shorter_than_the_tolerance_divides_nothing_and_leaves_a_loose_panel() {
    let lines = vec![
        cut(&[(0.0, 0.0), (50.0, 0.0), (50.0, 49.95), (100.0, 49.95), (100.0, 99.95), (50.0, 99.95), (50.0, 50.0), (0.0, 50.0)], 1),
        crease((50.0, 49.95), (50.0, 50.0), 2),
    ];
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["noSplit", "loosePanel"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![2]);
    assert!(b[0].hinges.is_empty());
    // The same with a hinge as long as the panels: no issue.
    let lines = vec![cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], 1), crease((50.0, 0.0), (50.0, 50.0), 2)];
    let (_, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn hinge_pieces_far_apart_are_measured_by_their_own_length() {
    // A window leaves two 0.02 pt strips joining the halves, each crossed by a crease: 0.04 pt of
    // hinge, though its pieces span 100 pt.
    let lines = vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1),
        cut(&[(40.0, 0.02), (60.0, 0.02), (60.0, 99.98), (40.0, 99.98)], 2),
        crease((50.0, 0.0), (50.0, 0.02), 3),
        crease((50.0, 99.98), (50.0, 100.0), 4),
    ];
    let (b, issues) = check(&lines);
    assert!(codes(&issues).contains(&"noSplit"), "{issues:?}");
    assert!(b[0].hinges.is_empty());
}

#[test]
fn a_window_split_by_a_cut_line_is_one_hole() {
    let mut lines =
        vec![cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1), cut(&[(40.0, 40.0), (60.0, 40.0), (60.0, 60.0), (40.0, 60.0)], 2)];
    lines.push(line(LineKind::Cut, &[(40.0, 50.0), (60.0, 50.0)], false, 3));
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b[0].holes.len(), 1);
    let area = vectorcraft_pathops::area(&b[0].holes[0], FillRule::NonZero);
    assert!((area - 400.0).abs() < 1e-3, "{area}");
}

#[test]
fn a_hinge_drawn_as_short_pieces_is_measured_whole() {
    // Ten 5 pt pieces make a 50 pt hinge, longer than a 10 pt tolerance though each piece is not.
    let mut lines = vec![cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], 1)];
    for k in 0..10u64 {
        let y = 5.0 * k as f64;
        lines.push(crease((50.0, y), (50.0, y + 5.0), 10 + k));
    }
    let (b, issues) = blanks(&lines, 10.0);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(b[0].hinges.len(), 1);
    let h = &b[0].hinges[0];
    assert!((h.from.distance(h.to) - 50.0).abs() < 1e-6 && h.keys.len() == 10, "{h:?}");
}

#[test]
fn a_crease_outside_the_blank_is_reported() {
    let mut lines = tray();
    lines.push(crease((200.0, 0.0), (250.0, 0.0), 20));
    let (_, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["outside"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![20]);
}

#[test]
fn a_bent_crease_folds_along_its_longest_piece() {
    let mut lines = tray();
    lines[1] = line(LineKind::Crease, &[(30.0, 30.0), (90.0, 30.0), (130.0, 32.0)], false, 10);
    lines[0] = cut(
        &[
            (30.0, 0.0),
            (130.0, 0.0),
            (130.0, 32.0),
            (160.0, 32.0),
            (160.0, 90.0),
            (130.0, 90.0),
            (130.0, 120.0),
            (30.0, 120.0),
            (30.0, 90.0),
            (0.0, 90.0),
            (0.0, 30.0),
            (30.0, 30.0),
        ],
        1,
    );
    lines[2] = crease((130.0, 32.0), (130.0, 90.0), 11);
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["notStraight"], "{issues:?}");
    let h = b[0].hinges.iter().find(|h| h.keys == vec![10]).unwrap();
    assert!(!h.straight);
    assert!((h.from.y - 30.0).abs() < 1e-9 && (h.to.y - 30.0).abs() < 1e-9, "along the 60 pt piece: {h:?}");
    // Within the tolerance, a crease is straight.
    let mut lines = tray();
    lines[1] = line(LineKind::Crease, &[(30.0, 30.0), (90.0, 30.05), (130.0, 30.0)], false, 10);
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(b[0].hinges.iter().all(|h| h.straight));
}

#[test]
fn a_slot_too_narrow_to_map_is_a_sliver() {
    let mut lines = tray();
    lines.push(cut(&[(60.0, 45.0), (60.004, 45.0), (60.004, 75.0), (60.0, 75.0)], 2));
    let (_, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["sliver"], "{issues:?}");
    assert!(issues[0].at.is_some_and(|a| Rect::new(59.9, 44.0, 60.1, 76.0).contains(a)));
}

#[test]
fn lines_in_a_hole_are_ambiguous() {
    let mut window = tray();
    window.push(cut(&[(60.0, 45.0), (100.0, 45.0), (100.0, 75.0), (60.0, 75.0)], 2));
    let mut lines = window.clone();
    lines.push(crease((70.0, 45.0), (70.0, 75.0), 20));
    let (_, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["ambiguous"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![20]);
    let mut lines = window;
    lines.push(cut(&[(70.0, 50.0), (90.0, 50.0), (90.0, 70.0), (70.0, 70.0)], 3));
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["ambiguous"], "{issues:?}");
    assert_eq!(issues[0].keys, vec![3]);
    assert_eq!(b.len(), 2, "the square in the window is a blank of its own");
}

#[test]
fn partial_cuts_and_perforations_make_hinges_as_creases_do() {
    let mut lines = tray();
    lines[1].kind = LineKind::PartialCut;
    lines[3].kind = LineKind::Perforation;
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    let mut keys: Vec<Vec<u64>> = b[0].hinges.iter().map(|h| h.keys.clone()).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec![vec![10], vec![11], vec![12], vec![13]]);
}

#[test]
fn every_code_has_an_id_and_a_severity() {
    let ids: Vec<&str> = IssueCode::ALL.iter().map(|c| c.id()).collect();
    assert_eq!(ids.len(), 15);
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 15);
    let errors: Vec<&str> = IssueCode::ALL.iter().filter(|c| c.is_error()).map(|c| c.id()).collect();
    assert_eq!(errors, vec!["noCut", "open", "looseEnd", "tooComplex", "cutOverCrease"]);
    let mut sorted = IssueCode::ALL;
    sorted.sort();
    assert_eq!(sorted, IssueCode::ALL, "listed in issue order");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// Random fold lines never panic, and the panels and hinges they give are consistent.
    #[test]
    fn random_fold_lines_give_consistent_blanks(
        segs in proptest::collection::vec((0u8..4, -50.0f64..50.0, -50.0f64..50.0, -50.0f64..50.0, -50.0f64..50.0), 1..10),
        boxed in any::<bool>(),
    ) {
        let kinds = [LineKind::Cut, LineKind::Crease, LineKind::PartialCut, LineKind::Perforation];
        let mut lines: Vec<FoldLine> = segs
            .iter()
            .enumerate()
            .map(|(i, &(k, a, b, c, d))| line(kinds[usize::from(k)], &[(a, b), (c, d)], false, i as u64))
            .collect();
        if boxed {
            lines.push(cut(&[(-40.0, -40.0), (40.0, -40.0), (40.0, 40.0), (-40.0, 40.0)], 99));
        }
        let (blanks, issues) = blanks(&lines, TOL);
        for b in &blanks {
            prop_assert!(!b.panels.is_empty() && b.panels.len() <= MAX_PANELS);
            for h in &b.hinges {
                prop_assert!(h.a < h.b && h.b < b.panels.len());
                prop_assert!(h.from.is_finite() && h.to.is_finite());
            }
        }
        let mut sorted = issues.clone();
        sorted.sort_by_key(|i| i.code);
        prop_assert_eq!(sorted, issues);
    }

    /// Random fold lines fold at any amount without a panic, every pose finite and every panel
    /// keeping its shape.
    #[test]
    fn random_blanks_fold_to_finite_poses(
        segs in proptest::collection::vec((1u8..4, -50.0f64..50.0, -50.0f64..50.0, -50.0f64..50.0, -50.0f64..50.0), 1..8),
        amount in 0.0f64..=1.0,
    ) {
        let kinds = [LineKind::Cut, LineKind::Crease, LineKind::PartialCut, LineKind::Perforation];
        let mut lines: Vec<FoldLine> = segs
            .iter()
            .enumerate()
            .map(|(i, &(k, a, b, c, d))| line(kinds[usize::from(k)], &[(a, b), (c, d)], false, i as u64))
            .collect();
        lines.push(cut(&[(-40.0, -40.0), (40.0, -40.0), (40.0, 40.0), (-40.0, 40.0)], 99));
        let (blanks, _) = blanks(&lines, TOL);
        for b in &blanks {
            let Some(f) = fold(b, None, amount, TOL) else { continue };
            prop_assert_eq!(f.panels.len(), b.panels.len());
            prop_assert_eq!(f.order.len(), b.panels.len());
            for p in &f.panels {
                prop_assert!(p.pose.m.iter().flatten().chain(&p.pose.t).all(|v| v.is_finite()));
                let n = p.pose.normal();
                prop_assert!((n[0] * n[0] + n[1] * n[1] + n[2] * n[2] - 1.0).abs() < 1e-9);
            }
        }
    }
}

/// The 3D corners of panel `i` of `b` folded as `f` places them.
fn corners(b: &Blank, f: &Folded, i: usize) -> Vec<[f64; 3]> {
    let rings = polygon(&b.panels[i].outline, 0.01).expect("a polygon");
    rings.into_iter().flatten().map(|p| f.panels[i].pose.place(p)).collect()
}

fn flat_center(b: &Blank, i: usize) -> [f64; 3] {
    let c = point_in(&b.panels[i].outline).expect("a point inside");
    [c.x, -c.y, 0.0]
}

#[test]
fn a_slotted_box_folds_every_panel_onto_a_face_of_the_box_with_the_print_outside() {
    let (b, issues) = check(&notched_box());
    assert!(issues.is_empty(), "{issues:?}");
    let b = &b[0];
    let f = fold(b, None, 1.0, TOL).expect("folds");
    // The base panel is the leftmost of the two largest body panels: x 20 to 120, y 30 to 110.
    let base = &b.panels[f.base];
    assert_eq!(base.outline.bounds().map(|r| (r.x0.round(), r.y0.round())), Some((20.0, 30.0)));
    // The box: the base panel's face (z = 0), 60 deep (the narrow body panels' width).
    let (lo, hi) = ([20.0, -110.0, -60.0], [120.0, -30.0, 0.0]);
    let middle = [70.0, -70.0, -30.0];
    for i in 0..b.panels.len() {
        let points = corners(b, &f, i);
        let face = (0..3).find_map(|k| [lo[k], hi[k]].into_iter().find(|&at| points.iter().all(|p| (p[k] - at).abs() < 1e-6)).map(|at| (k, at)));
        let (k, at) = face.unwrap_or_else(|| panic!("panel {i} lies on no face of the box: {points:?}"));
        for p in &points {
            for j in 0..3 {
                assert!(p[j] > lo[j] - 1e-6 && p[j] < hi[j] + 1e-6, "panel {i} reaches out of the box: {p:?}");
            }
        }
        // The printed side faces out of the box.
        let n = f.panels[i].pose.normal();
        assert!((n[k].abs() - 1.0).abs() < 1e-9, "panel {i} is square to its face: {n:?}");
        assert!(n[k] * (at - middle[k]) > 0.0, "panel {i}'s print faces out: {n:?}");
    }
    assert!(f.hinges.iter().all(|h| h.tree && h.closes && (h.angle - 90.0).abs() < 1e-12), "{:?}", f.hinges);
    assert!(f.collisions.is_empty(), "the stacked flaps and the glue flap do not cross: {:?}", f.collisions);
    assert_eq!(f.order.first(), Some(&f.base));
    assert_eq!(f.order.len(), 13);
}

#[test]
fn at_amount_0_the_blank_is_flat() {
    let (b, _) = check(&notched_box());
    let f = fold(&b[0], None, 0.0, TOL).expect("folds");
    for p in &f.panels {
        for (row, want) in p.pose.m.iter().zip(Pose::IDENTITY.m) {
            assert!(row.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-12), "{:?}", p.pose);
        }
        assert!(p.pose.t.iter().all(|v| v.abs() < 1e-9), "{:?}", p.pose);
    }
}

/// A 100 × 60 rectangle split by a line of `kind` at x = 60 (drawn upward when `up`), turned by
/// `quarter` quarter turns about the origin and mirrored in x when `mirror`.
fn two_panels(kind: LineKind, quarter: u8, mirror: bool, up: bool) -> Vec<FoldLine> {
    let at = |(x, y): (f64, f64)| {
        let x = if mirror { -x } else { x };
        (0..quarter).fold((x, y), |(x, y), _| (-y, x))
    };
    let outline: Vec<(f64, f64)> = [(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)].into_iter().map(at).collect();
    let (a, b) = if up { ((60.0, 60.0), (60.0, 0.0)) } else { ((60.0, 0.0), (60.0, 60.0)) };
    vec![cut(&outline, 1), line(kind, &[at(a), at(b)], false, 2)]
}

#[test]
fn a_crease_folds_away_from_the_print_and_a_partial_cut_toward_it_however_it_is_drawn() {
    for quarter in 0..4 {
        for mirror in [false, true] {
            for up in [false, true] {
                for (kind, sign) in [(LineKind::Crease, -1.0), (LineKind::Perforation, -1.0), (LineKind::PartialCut, 1.0)] {
                    let case = format!("{kind:?}, {quarter} quarter turns, mirrored {mirror}, drawn up {up}");
                    let (b, issues) = check(&two_panels(kind, quarter, mirror, up));
                    assert!(issues.is_empty(), "{case}: {issues:?}");
                    let b = &b[0];
                    let f = fold(b, None, 1.0, TOL).expect("folds");
                    let child = 1 - f.base;
                    let h = &b.hinges[0];
                    let hinge = [(h.from.x + h.to.x) / 2.0, -(h.from.y + h.to.y) / 2.0, 0.0];
                    let c = flat_center(b, child);
                    let away = [c[0] - hinge[0], c[1] - hinge[1], 0.0];
                    let along = [h.to.x - h.from.x, h.from.y - h.to.y];
                    let off_line = (along[0] * away[1] - along[1] * away[0]).abs() / along[0].hypot(along[1]);
                    let placed = f.panels[child].pose.apply(c);
                    // The child stands up out of the plane: behind it for a crease, in front of it
                    // for a partial cut, at its distance from the hinge.
                    assert!(placed[2] * sign > 0.0, "{case}: {placed:?}");
                    assert!((placed[2].abs() - off_line).abs() < 1e-6, "{case}: {placed:?} {off_line}");
                    // Its printed side faces away from the base panel after a crease, back toward
                    // it after a partial cut.
                    let n = f.panels[child].pose.normal();
                    assert!((n[0] * away[0] + n[1] * away[1]) * -sign > 0.0, "{case}: {n:?}");
                    // Half way, the child is half way round.
                    let half = fold(b, None, 0.5, TOL).expect("folds");
                    let z = half.panels[child].pose.normal()[2];
                    assert!((z - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9, "{case}: {z}");
                }
            }
        }
    }
}

/// A 100 × 100 square divided into four panels by a crease across it each way.
fn crossed() -> Vec<FoldLine> {
    vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], 1),
        crease((50.0, 0.0), (50.0, 100.0), 2),
        crease((0.0, 50.0), (100.0, 50.0), 3),
    ]
}

#[test]
fn a_loop_of_hinges_closes_when_flat_and_reports_its_gap_when_folded() {
    let (b, issues) = check(&crossed());
    assert!(issues.is_empty(), "{issues:?}");
    let b = &b[0];
    assert_eq!((b.panels.len(), b.hinges.len()), (4, 4));
    let folded = fold(b, None, 1.0, TOL).expect("folds");
    let closing: Vec<&FoldedHinge> = folded.hinges.iter().filter(|h| !h.tree).collect();
    assert_eq!(closing.len(), 1, "three hinges make the tree, one closes the loop");
    assert!(!closing[0].closes && closing[0].gap > 10.0, "{closing:?}");
    // The gap is measured folded all the way, whatever the amount: flat or half way, the loop is
    // still reported open by the same gap.
    for amount in [0.0, 0.5] {
        let part = fold(b, None, amount, TOL).expect("folds");
        let h = part.hinges.iter().find(|h| !h.tree).expect("a closing hinge");
        assert!(!h.closes && (h.gap - closing[0].gap).abs() < 1e-9, "{amount}: {h:?}");
    }
    // The base is the top-left panel: four equal panels tie, and the highest, then leftmost, wins.
    assert_eq!(b.panels[folded.base].outline.bounds().map(|r| (r.x0.round(), r.y0.round())), Some((0.0, 0.0)));
}

#[test]
fn hinges_that_start_at_one_point_fold_the_same_in_any_line_order() {
    // About the bottom-right panel, its two hinges both start at the middle of the square.
    let lines = crossed();
    let orders = [[0, 1, 2], [0, 2, 1], [2, 1, 0], [1, 2, 0], [1, 0, 2], [2, 0, 1]];
    let mut seen = None;
    for order in orders {
        let lines: Vec<FoldLine> = order.iter().map(|&i| lines[i].clone()).collect();
        let (b, issues) = check(&lines);
        assert!(issues.is_empty(), "{order:?}: {issues:?}");
        let b = &b[0];
        let base = panel_at(b, p(75.0, 75.0)).expect("the bottom-right panel");
        let f = fold(b, Some(base), 1.0, TOL).expect("folds");
        let closing = f.hinges.iter().zip(&b.hinges).find(|(h, _)| !h.tree).map(|(_, h)| ((h.from.x + h.to.x) / 2.0, (h.from.y + h.to.y) / 2.0));
        let top_left = panel_at(b, p(25.0, 25.0)).expect("the top-left panel");
        let placed = f.panels[top_left].pose.place(p(25.0, 25.0)).map(|v| (v * 1e6).round());
        let got = (closing, placed);
        assert_eq!(seen.get_or_insert(got), &got, "{order:?}");
    }
}

/// A 100 × 60 rectangle: the left 60 pt is one panel, the right is split across at y = 30, and the
/// line between them bends by `lean` pt over its lower half.
fn leaning(lean: f64) -> Vec<FoldLine> {
    vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)], 1),
        crease((60.0, 0.0), (60.0, 30.0), 2),
        crease((60.0, 30.0), (60.0 + lean, 60.0), 3),
        crease((60.0, 30.0), (100.0, 30.0), 4),
    ]
}

#[test]
fn a_loop_closes_within_the_tolerance_or_half_a_percent_of_its_hinge() {
    // Both right panels turn about one line, so the hinge between them closes exactly.
    let gap = |lean: f64, amount: f64| {
        let (b, issues) = check(&leaning(lean));
        assert!(issues.is_empty(), "{lean}: {issues:?}");
        let b = &b[0];
        assert_eq!((b.panels.len(), b.hinges.len()), (3, 3));
        let base = panel_at(b, p(30.0, 30.0)).expect("the left panel");
        let f = fold(b, Some(base), amount, TOL).expect("folds");
        let h = f.hinges.iter().find(|h| !h.tree).expect("a closing hinge").clone();
        (h.closes, h.gap)
    };
    let (closes, at) = gap(0.0, 1.0);
    assert!(closes && at < 1e-9, "{at}");
    // A small lean opens it by more than the 0.1 pt tolerance but less than 0.5% of its 40 pt
    // length (0.2 pt): still closed.
    let (closes, at) = gap(0.1, 1.0);
    assert!(closes && at > TOL && at <= 0.2, "{at}");
    // A larger lean opens it past both, flat or folded.
    for amount in [0.0, 1.0] {
        let (closes, at) = gap(1.0, amount);
        assert!(!closes && at > 0.2, "{amount}: {at}");
    }
}

#[test]
fn a_panel_no_hinge_joins_stays_flat_where_it_is() {
    let lines = vec![
        cut(&[(0.0, 0.0), (50.0, 0.0), (50.0, 49.95), (100.0, 49.95), (100.0, 99.95), (50.0, 99.95), (50.0, 50.0), (0.0, 50.0)], 1),
        crease((50.0, 49.95), (50.0, 50.0), 2),
    ];
    let (b, _) = check(&lines);
    let b = &b[0];
    assert_eq!((b.panels.len(), b.hinges.len()), (2, 0));
    let f = fold(b, None, 1.0, TOL).expect("folds");
    let loose = 1 - f.base;
    assert_eq!(f.panels[loose].parent, None);
    assert_eq!(f.panels[loose].pose, Pose::IDENTITY);
    assert_eq!(f.order, vec![f.base, loose], "the loose panel comes last");
}

#[test]
fn a_child_of_a_notched_panel_folds_away_from_the_print() {
    // The base is an L: a 30 × 140 strip with an 80 pt arm along its top running right to x = 300.
    // The child, below the arm past a 2 pt slot, is hinged to the strip along x = 30.
    let lines = vec![
        cut(&[(0.0, 0.0), (300.0, 0.0), (300.0, 80.0), (30.0, 80.0), (30.0, 82.0), (300.0, 82.0), (300.0, 140.0), (0.0, 140.0)], 1),
        crease((30.0, 82.0), (30.0, 140.0), 2),
    ];
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &b[0];
    let base = panel_at(b, p(15.0, 110.0)).expect("the L");
    let child = panel_at(b, p(150.0, 110.0)).expect("the child");
    // The L's inside point lies in its arm, on the child's side of the hinge's line.
    let inside = b.panels[base].point_inside().expect("a point inside");
    assert!(inside.x > 30.0, "{inside:?}");
    for (kind, sign) in [(LineKind::Crease, -1.0), (LineKind::PartialCut, 1.0)] {
        let mut lines = lines.clone();
        lines[1].kind = kind;
        let (b, _) = check(&lines);
        let f = fold(&b[0], Some(base), 1.0, TOL).expect("folds");
        let placed = f.panels[child].pose.place(p(150.0, 110.0));
        assert!(placed[2] * sign > 99.0, "{kind:?}: {placed:?}");
    }
}

#[test]
fn a_lid_folded_over_the_base_crosses_the_taller_wall_opposite() {
    // A strip: a 120 pt lid, a 30 pt wall, the 100 pt base and a 60 pt wall, all 50 pt deep.
    let lines = vec![
        cut(&[(0.0, 0.0), (310.0, 0.0), (310.0, 50.0), (0.0, 50.0)], 1),
        crease((120.0, 0.0), (120.0, 50.0), 2),
        crease((150.0, 0.0), (150.0, 50.0), 3),
        crease((250.0, 0.0), (250.0, 50.0), 4),
    ];
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &b[0];
    let base = panel_at(b, p(200.0, 25.0)).expect("the base panel");
    let (lid, wall) = (panel_at(b, p(60.0, 25.0)).unwrap(), panel_at(b, p(280.0, 25.0)).unwrap());
    let f = fold(b, Some(base), 1.0, TOL).expect("folds");
    assert_eq!(f.collisions, vec![(lid.min(wall), lid.max(wall))]);
    // Half folded, nothing crosses yet.
    assert!(fold(b, Some(base), 0.4, TOL).expect("folds").collisions.is_empty());
}

#[test]
fn the_base_is_the_panel_under_a_point_and_the_same_in_any_input_order() {
    let lines = vec![cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)], 1), crease((50.0, 0.0), (50.0, 60.0), 2)];
    let (b, _) = check(&lines);
    assert_eq!(panel_at(&b[0], p(500.0, 500.0)), None, "a point outside every panel");
    let right = panel_at(&b[0], p(75.0, 30.0)).expect("the right panel");
    assert_eq!(fold(&b[0], Some(right), 1.0, TOL).expect("folds").base, right);
    let mut reversed = lines.clone();
    reversed.reverse();
    let (r, _) = check(&reversed);
    for blank in [&b[0], &r[0]] {
        let f = fold(blank, None, 1.0, TOL).expect("folds");
        // Equal panels: the leftmost wins.
        assert_eq!(blank.panels[f.base].outline.bounds().map(|r| r.x0.round()), Some(0.0));
    }
    assert!(fold(&b[0], Some(9), 1.0, TOL).is_none(), "no panel 9");
    assert!(fold(&b[0], None, 1.5, TOL).is_none(), "past 1");
    assert!(fold(&b[0], None, f64::NAN, TOL).is_none());
    assert!(fold(&Blank::default(), None, 1.0, TOL).is_none(), "no panels");
}

#[test]
fn a_partial_cut_takes_its_own_angle_and_a_hinge_its_longest_piece_s() {
    let (b, _) = check(&two_panels(LineKind::PartialCut, 0, false, false));
    assert_eq!(b[0].hinges[0].angle, -90.0);
    let (b, _) = check(&two_panels(LineKind::Perforation, 0, false, false));
    assert_eq!(b[0].hinges[0].angle, 90.0);
    // A crease and a short partial cut on one line: the crease is the longer piece.
    let lines = vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)], 1),
        line(LineKind::PartialCut, &[(60.0, 0.0), (60.0, 10.0)], false, 2),
        crease((60.0, 10.0), (60.0, 60.0), 3),
    ];
    let (b, _) = check(&lines);
    assert_eq!(b[0].hinges.len(), 1);
    assert_eq!(b[0].hinges[0].angle, 90.0);
}

#[test]
fn a_line_s_own_angle_takes_the_place_of_its_kind_s() {
    let own = |kind, angle| FoldLine { path: poly(&[(0.0, 0.0), (1.0, 0.0)], false), kind, key: 1, angle };
    assert_eq!(own(LineKind::Crease, Some(45.0)).fold_angle(), Some(45.0));
    assert_eq!(own(LineKind::PartialCut, Some(30.0)).fold_angle(), Some(30.0));
    assert_eq!(own(LineKind::Perforation, Some(-120.0)).fold_angle(), Some(-120.0));
    assert_eq!(own(LineKind::PartialCut, None).fold_angle(), Some(-90.0));
    // An angle that is not a number folds at the kind's; a cut never folds.
    assert_eq!(own(LineKind::Crease, Some(f64::NAN)).fold_angle(), Some(90.0));
    assert_eq!(own(LineKind::Crease, Some(f64::INFINITY)).fold_angle(), Some(90.0));
    assert_eq!(own(LineKind::Crease, Some(f64::MAX)).fold_angle(), Some(90.0));
    // Past -180 to 180, before rounding, as for an angle the engine stores.
    assert_eq!(own(LineKind::Crease, Some(360.0)).fold_angle(), Some(90.0));
    assert_eq!(own(LineKind::PartialCut, Some(-180.0000001)).fold_angle(), Some(-90.0));
    assert_eq!(own(LineKind::Crease, Some(-180.0)).fold_angle(), Some(-180.0));
    assert_eq!(own(LineKind::Cut, Some(45.0)).fold_angle(), None);
    // To the millionth of a degree: closer to flat than that is flat.
    assert_eq!(own(LineKind::Crease, Some(-179.99999998)).fold_angle(), Some(-180.0));
    assert_eq!(own(LineKind::Crease, Some(179.9999994)).fold_angle(), Some(179.999999));
    assert_eq!(own(LineKind::Crease, Some(-0.0000001)).fold_angle().map(f64::to_bits), Some(0.0f64.to_bits()));
}

#[test]
fn a_tray_wall_with_its_own_angle_stands_at_that_angle() {
    let mut lines = tray();
    // The wall below the base (y 90 to 120) folds at 45°, the others at 90°.
    for l in lines.iter_mut().filter(|l| l.key == 12) {
        l.angle = Some(45.0);
    }
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &b[0];
    let f = fold(b, None, 1.0, TOL).expect("folds");
    for (i, panel) in b.panels.iter().enumerate().filter(|(i, _)| *i != f.base) {
        let n = f.panels[i].pose.normal();
        // The cosine of the angle between the wall's print and the base's.
        let lower = panel.outline.bounds().is_some_and(|r| r.y0 > 89.0);
        let want = if lower { std::f64::consts::FRAC_1_SQRT_2 } else { 0.0 };
        assert!((n[2] - want).abs() < 1e-9, "panel {i} ({lower}): {n:?}");
    }
    let h = b.hinges.iter().find(|h| h.keys == [12]).expect("the lower wall's hinge");
    assert_eq!(h.angle, 45.0);
}

#[test]
fn a_hinge_takes_the_angle_of_its_longest_line_however_the_map_splits_it() {
    // One hinge along y = 60: a 60 pt crease at 30° and a 40 pt crease at 120°. A crease that
    // stops inside the panel above meets the first at x = 30, splitting it into two 30 pt pieces.
    let mut lines = vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 160.0), (0.0, 160.0)], 1),
        crease((0.0, 60.0), (60.0, 60.0), 2),
        crease((60.0, 60.0), (100.0, 60.0), 3),
        crease((30.0, 60.0), (30.0, 90.0), 4),
    ];
    lines[1].angle = Some(30.0);
    lines[2].angle = Some(120.0);
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["danglingCrease"], "{issues:?}");
    let hinges: Vec<_> = b[0].hinges.iter().filter(|h| h.keys.contains(&2)).collect();
    assert_eq!(hinges.len(), 1, "{:?}", b[0].hinges);
    assert_eq!((hinges[0].keys.clone(), hinges[0].angle), (vec![2, 3], 30.0));
}

#[test]
fn a_line_s_subpaths_on_top_of_each_other_count_once_toward_the_hinge_angle() {
    // One hinge along y = 60: a 40 pt crease at 30° drawn as two subpaths on top of each other,
    // and a 60 pt crease at 120°.
    let mut twice = poly(&[(0.0, 60.0), (40.0, 60.0)], false);
    twice.subpaths.extend(poly(&[(0.0, 60.0), (40.0, 60.0)], false).subpaths);
    let lines = vec![
        cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 160.0), (0.0, 160.0)], 1),
        FoldLine { path: twice, kind: LineKind::Crease, key: 2, angle: Some(30.0) },
        FoldLine { path: poly(&[(40.0, 60.0), (100.0, 60.0)], false), kind: LineKind::Crease, key: 3, angle: Some(120.0) },
    ];
    let (b, issues) = check(&lines);
    assert_eq!(codes(&issues), vec!["duplicate"], "{issues:?}");
    assert_eq!(b[0].hinges.len(), 1, "{:?}", b[0].hinges);
    assert_eq!(b[0].hinges[0].angle, 120.0);
}

#[test]
fn lines_of_equal_length_on_one_hinge_give_the_lowest_key_s_angle_in_any_order() {
    let mut a = crease((0.0, 60.0), (50.0, 60.0), 5);
    a.angle = Some(30.0);
    let mut b = crease((50.0, 60.0), (100.0, 60.0), 3);
    b.angle = Some(120.0);
    let outline = cut(&[(0.0, 0.0), (100.0, 0.0), (100.0, 160.0), (0.0, 160.0)], 1);
    for lines in [vec![outline.clone(), a.clone(), b.clone()], vec![outline.clone(), b.clone(), a.clone()]] {
        let (blanks, issues) = check(&lines);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(blanks[0].hinges.len(), 1);
        assert_eq!(blanks[0].hinges[0].angle, 120.0, "{:?}", lines.iter().map(|l| l.key).collect::<Vec<_>>());
    }
    // The same along a diagonal, with the line of the higher key split in two by a crease that
    // stops inside a panel: its pieces' lengths (2√2 and 10√2) add up to a little more than the
    // other line's (12√2) in floating point.
    let mut a = crease((0.0, 0.0), (12.0, 12.0), 5);
    a.angle = Some(30.0);
    let mut b = crease((12.0, 12.0), (24.0, 24.0), 3);
    b.angle = Some(120.0);
    let outline = cut(&[(0.0, 0.0), (24.0, 0.0), (24.0, 24.0), (0.0, 24.0)], 1);
    let dangling = crease((2.0, 2.0), (6.0, 3.0), 4);
    for lines in [vec![outline.clone(), a.clone(), b.clone(), dangling.clone()], vec![outline, dangling, b, a]] {
        let (blanks, issues) = check(&lines);
        assert_eq!(codes(&issues), vec!["danglingCrease"], "{issues:?}");
        assert_eq!(blanks[0].hinges.len(), 1, "{:?}", blanks[0].hinges);
        assert_eq!(blanks[0].hinges[0].angle, 120.0, "{:?}", lines.iter().map(|l| l.key).collect::<Vec<_>>());
    }
    // A line 0.0005 pt longer than the other at 1,000,000 pt is longer, not a tie: its angle wins
    // over the lower key's.
    let mut a = crease((0.0, 60.0), (1_000_000.0, 60.0), 3);
    a.angle = Some(30.0);
    let end = 2_000_000.000_5;
    let mut b = crease((1_000_000.0, 60.0), (end, 60.0), 5);
    b.angle = Some(120.0);
    let outline = cut(&[(0.0, 0.0), (end, 0.0), (end, 160.0), (0.0, 160.0)], 1);
    for lines in [vec![outline.clone(), a.clone(), b.clone()], vec![outline, b, a]] {
        let (blanks, issues) = check(&lines);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(blanks[0].hinges.len(), 1, "{:?}", blanks[0].hinges);
        assert_eq!(blanks[0].hinges[0].angle, 120.0, "{:?}", lines.iter().map(|l| l.key).collect::<Vec<_>>());
    }
}

#[test]
fn a_negative_angle_folds_a_crease_toward_the_print() {
    let mut lines = two_panels(LineKind::Crease, 0, false, false);
    for l in lines.iter_mut().filter(|l| l.key == 2) {
        l.angle = Some(-90.0);
    }
    let (b, issues) = check(&lines);
    assert!(issues.is_empty(), "{issues:?}");
    let b = &b[0];
    let f = fold(b, None, 1.0, TOL).expect("folds");
    let child = 1 - f.base;
    let placed = f.panels[child].pose.apply(flat_center(b, child));
    assert!(placed[2] > 0.0, "in front of the print: {placed:?}");
}
