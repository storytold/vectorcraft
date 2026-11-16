use vectorcraft_geom::{PathData, Point, Rect, SubPath};

use super::super::{Blank, Folded, FoldedPanel, Panel, Pose};
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
        panels: poses.into_iter().map(|pose| FoldedPanel { pose, parent: None, depth: 0 }).collect(),
        order: (0..n).collect(),
        hinges: vec![],
        collisions: vec![],
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
