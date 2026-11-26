//! PostScript print jobs: DSC page comments, the setup of each sheet, and a file the PostScript
//! reader reads back.

use vectorcraft_color::{
    Color, Freeform, FreeformPoint, Gradient, GradientGeom, GradientKind, GradientMethod, GradientPaint, GradientSpread, GradientStop, Paint, Swatch,
};
use vectorcraft_doc::{Appearance, CharStyle, Document, Node, NodeId, NodeKind, StrokeGradientMode, TextObject};
use vectorcraft_geom::{Affine, Point, Rect, Vec2, shapes};
use vectorcraft_testkit::raster::{Image, render_region};

use super::*;

/// A 200 × 100 pt document with a red rectangle.
fn doc() -> Document {
    let mut d = Document::new(200.0, 100.0);
    let layer = d.layers[0].id;
    let look = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 60.0)), look);
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The document on a letter page, 36 pt in from its top-left corner.
fn page<'a>(d: &'a Document, marks: Option<&'a Node>) -> PrintPage<'a> {
    PrintPage {
        doc: d,
        size: (612.0, 792.0),
        view: Affine::IDENTITY,
        window: None,
        place: Affine::translate((36.0, 36.0)),
        copies: &[],
        area: d.artboards[0].rect,
        marks,
        negative: false,
        ink: None,
    }
}

fn text(out: &EpsOutput) -> String {
    String::from_utf8(out.bytes.clone()).unwrap()
}

#[test]
fn pages_follow_dsc_and_repeat_for_copies() {
    let d = doc();
    let mark =
        Node::path(NodeId(9999), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::None, Paint::registration(), 0.25));
    let pages = [page(&d, None), PrintPage { size: (792.0, 612.0), ..page(&d, Some(&mark)) }];
    let job = PrintJob { title: "Job".into(), created: Some(0), ..PrintJob::default() };
    let out = print(&pages, &[0, 1, 0, 1], &job).unwrap();
    let ps = text(&out);
    assert!(ps.starts_with("%!PS-Adobe-3.0\n") && ps.ends_with("%%EOF\n"));
    let dsc = |key: &str| ps.lines().filter_map(|l| l.strip_prefix(key)).map(str::trim).collect::<Vec<_>>();
    assert_eq!(dsc("%%Pages:"), ["4"]);
    assert_eq!(dsc("%%Page:"), ["1 1", "2 2", "3 3", "4 4"]);
    assert_eq!(dsc("%%PageBoundingBox:"), ["0 0 612 792", "0 0 792 612", "0 0 612 792", "0 0 792 612"]);
    assert_eq!(dsc("%%BoundingBox:"), ["0 0 792 792"]);
    assert_eq!(dsc("%%LanguageLevel:"), ["3"]);
    assert_eq!(ps.matches("showpage").count(), 4);
    assert!(ps.contains("<< /PageSize [792 612] >> setpagedevice"));
    // The marks are drawn in Registration on the pages that have them.
    assert_eq!(ps.matches("/All /DeviceCMYK").count(), 2, "{ps}");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    // The PostScript reader reads the first page back: the rectangle 36 pt in, clipped to the
    // artboard.
    let back = crate::import(&out.bytes).unwrap();
    let art = back.document.layers[0].children().unwrap().clone();
    let clipped = art[0].children().unwrap();
    assert_eq!(clipped[0].geometric_bounds().unwrap(), Rect::new(36.0, 36.0, 236.0, 136.0));
    assert_eq!(clipped[1].geometric_bounds().unwrap(), Rect::new(56.0, 56.0, 156.0, 96.0));
    assert!(back.warnings.iter().any(|w| w.contains("first of the file's 4 pages")), "{:?}", back.warnings);
}

#[test]
fn copies_on_a_sheet_draw_the_art_and_marks_once_each() {
    let d = doc();
    let mark =
        Node::path(NodeId(9999), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::None, Paint::registration(), 0.25));
    let moves = [kurbo::Vec2::ZERO, kurbo::Vec2::new(218.0, 0.0), kurbo::Vec2::new(0.0, 118.0)];
    let sheet = PrintPage { copies: &moves, ..page(&d, Some(&mark)) };
    let ps = text(&print(&[sheet], &[0], &PrintJob::default()).unwrap());
    // The art is placed three times, the second 218 pt right and the third 118 pt down, and its
    // spot color space and the marks' are defined once.
    assert_eq!(ps.matches("[1 0 0 1 36 36] cm\n").count(), 1, "{ps}");
    assert!(ps.contains("[1 0 0 1 254 36] cm\n") && ps.contains("[1 0 0 1 36 154] cm\n"), "{ps}");
    assert_eq!(ps.matches("/All /DeviceCMYK").count(), 1, "{ps}");
    assert!(ps.contains("q\n[1 0 0 1 218 0] cm\n") && ps.contains("q\n[1 0 0 1 0 118] cm\n"), "{ps}");
    // A mark off the page around the first copy still prints around a later copy that has it on the page.
    let look = Appearance::basic(Paint::None, Paint::registration(), 0.25);
    let off = Node::path(NodeId(9998), shapes::rectangle(Rect::new(-100.0, -100.0, -90.0, -90.0)), look);
    let later = [kurbo::Vec2::ZERO, kurbo::Vec2::new(200.0, 200.0)];
    let ps = text(&print(&[PrintPage { copies: &later, ..page(&d, Some(&off)) }], &[0], &PrintJob::default()).unwrap());
    assert_eq!(ps.matches("/All /DeviceCMYK").count(), 1, "{ps}");
    // One copy unmoved is the page without copies.
    let one = [kurbo::Vec2::ZERO];
    let single = text(&print(&[PrintPage { copies: &one, ..page(&d, Some(&mark)) }], &[0], &PrintJob::default()).unwrap());
    assert_eq!(single, text(&print(&[page(&d, Some(&mark))], &[0], &PrintJob::default()).unwrap()));
}

#[test]
fn separations_screens_flatness_negatives_and_tiles() {
    let d = doc();
    let sheet = PrintPage { negative: true, ink: Some(("Cyan", 85.0, 15.0)), window: Some(Rect::new(0.0, 0.0, 100.0, 100.0)), ..page(&d, None) };
    let job = PrintJob { level: Level::Two, flatness: Some(3.0), ..PrintJob::default() };
    let ps = text(&print(&[sheet], &[0], &job).unwrap());
    assert!(ps.contains("%%PlateColor: Cyan\n"));
    assert!(ps.contains("85 15 {dup mul exch dup mul add 1 exch sub} setscreen"));
    assert!(ps.contains("{1 exch sub} settransfer 1 g 0 0 612 792 rectfill"));
    assert!(ps.contains("3 setflat"));
    assert!(ps.contains("%%LanguageLevel: 2"));
    // The tile clips the page.
    assert!(ps.contains("0 0 m\n100 0 l\n100 100 l\n0 100 l\n"), "{ps}");
    assert!(print(&[], &[], &job).is_err());
    assert!(print(&[PrintPage { size: (f64::NAN, 1.0), ..page(&d, None) }], &[0], &job).is_err());
}

/// A 200 × 100 pt CMYK document with spot swatch `Ink` (100% K): a rectangle over the whole page
/// painted `under`, when there is one, then `art`.
fn over(under: Option<Paint>, art: &[Node]) -> Document {
    let mut d = Document::new(200.0, 100.0);
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    d.swatches.push(Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)), global: true, spot: true });
    let layer = d.layers[0].id;
    let under = under.map(|p| Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 100.0)), Appearance::basic(p, Paint::None, 0.0)));
    for mut n in under.into_iter().chain(art.iter().cloned()) {
        n.id = d.alloc_id();
        d.insert(Some(layer), usize::MAX, n).unwrap();
    }
    d
}

/// Ink `ink`'s plate of `d` as the print plan separates it for PostScript, and as a PostScript
/// separation at `level` read back, both drawn at 1 px a point.
fn plate_of(d: &Document, ink: &str, level: Level) -> (Image, Image) {
    let area = Rect::new(0.0, 0.0, 200.0, 100.0);
    let (plate, out) = separation(d, ink, level);
    let back = crate::import(&out.bytes).unwrap().document;
    (render_region(&plate, area, 1.0), render_region(&back, area, 1.0))
}

/// Ink `ink`'s plate of `d` as the print plan separates it for PostScript, and that plate written
/// as a PostScript separation at `level`.
fn separation(d: &Document, ink: &str, level: Level) -> (Document, EpsOutput) {
    let area = Rect::new(0.0, 0.0, 200.0, 100.0);
    let settings = serde_json::from_value(serde_json::json!({"output": {"mode": "separations"}})).unwrap();
    let plan = vectorcraft_pdf::plan(d, &vectorcraft_pdf::PrintOptions { settings, postscript: true, ..Default::default() }).unwrap();
    let i = plan.sheets.iter().find(|s| s.ink.as_ref().is_some_and(|p| p.name == ink)).and_then(|s| s.plate).unwrap();
    let plate = &plan.plates[i];
    let page = PrintPage {
        doc: plate,
        size: (200.0, 100.0),
        view: Affine::IDENTITY,
        window: None,
        place: Affine::IDENTITY,
        copies: &[],
        area,
        marks: None,
        negative: false,
        ink: Some((ink, 85.0, 15.0)),
    };
    let out = print(&[page], &[0], &PrintJob { level, ..PrintJob::default() }).unwrap();
    assert!(level == Level::Three || !text(&out).contains("shfill"), "Level 2 has no smooth shadings");
    (plate.clone(), out)
}

/// Ink `ink`'s PostScript plate of `art` over a page of 100% `ink` (`under`), checked against the
/// plate of `art` alone (drawn from the print plan, or with `postscript` from its PostScript
/// plate): where `art` puts none of the ink down, the ink under it prints, and where it puts ink
/// down, its own amount prints. The pixels of the page where `art` alone puts no ink, those of
/// them where the ink under it does not print, and the pixels where `art` puts ink down that do
/// not print its amount.
fn keeps_the_ink_below(art: &[Node], under: &Paint, ink: &str, level: Level, postscript: bool) -> (usize, usize, usize) {
    let (plan, own) = plate_of(&over(None, art), ink, level);
    let alone = if postscript { own } else { plan };
    let (_, printed) = plate_of(&over(Some(under.clone()), art), ink, level);
    // Pixels whose neighbors all have paper (or all have ink) on the plate of `art` alone: an
    // edge falls on either side of a pixel's center, and the edge of the inked part is lightest.
    let around = |x: u32, y: u32, paper: bool| {
        (x.saturating_sub(1)..=(x + 1).min(alone.width - 1))
            .all(|i| (y.saturating_sub(1)..=(y + 1).min(alone.height - 1)).all(|j| (alone.over_white(i, j)[0] >= 254) == paper))
    };
    let (mut paper, mut knocked_out, mut wrong) = (0, 0, 0);
    for y in 0..alone.height {
        for x in 0..alone.width {
            let (a, p) = (alone.over_white(x, y)[0], printed.over_white(x, y)[0]);
            if around(x, y, true) {
                paper += 1;
                knocked_out += usize::from(p > 60);
            } else if around(x, y, false) {
                wrong += usize::from(a.abs_diff(p) > 48);
            }
        }
    }
    (paper, knocked_out, wrong)
}

/// A rectangle from (20, 10) to (180, 90) with `fill` and `stroke` (16 pt), each set to
/// overprint.
fn overprinting(fill: Paint, stroke: Paint) -> Node {
    let mut n = Node::path(NodeId(0), shapes::rectangle(Rect::new(20.0, 10.0, 180.0, 90.0)), Appearance::basic(fill, stroke, 16.0));
    for it in &mut n.appearance.items {
        *it.overprint_mut() = true;
    }
    n
}

/// `stops` (offset, color) as a `kind` gradient placed at `geom`, spreading as `spread`.
fn gradient(kind: GradientKind, stops: &[(f32, Color)], geom: GradientGeom, spread: GradientSpread) -> Paint {
    let stops = stops.iter().map(|(o, c)| GradientStop::new(*o, *c)).collect();
    let mut g = GradientPaint::new(Gradient { spread, repeats: 2, ..Gradient::new(kind, stops) });
    g.geom = Some(geom);
    Paint::Gradient(Box::new(g))
}

#[test]
fn an_overprinting_gradient_keeps_the_ink_below_where_it_puts_none_on_a_postscript_plate() {
    // Cyan in the outer stops, none from 0.3 to 0.6 (magenta only there).
    let (inked, none) = (Color::cmyk(0.6, 0.3, 0.0, 0.0), Color::cmyk(0.0, 0.3, 0.0, 0.0));
    let stops = [(0.0, inked), (0.3, none), (0.6, none), (1.0, inked)];
    let c = Point::new(100.0, 50.0);
    let around = GradientGeom { start: c, end: c + Vec2::new(70.0, -20.0), aspect: 0.7, focal: None };
    let across = GradientGeom { start: Point::new(30.0, 40.0), end: Point::new(170.0, 60.0), aspect: 1.0, focal: None };
    let pad = GradientSpread::Pad;
    let fills = [
        ("linear", gradient(GradientKind::Linear, &stops, across, pad)),
        ("radial", gradient(GradientKind::Radial, &stops, around, pad)),
        ("off-center radial", gradient(GradientKind::Radial, &stops, GradientGeom { focal: Some(c + Vec2::new(-20.0, 10.0)), ..around }, pad)),
        ("conic", gradient(GradientKind::Conic, &stops, around, pad)),
        ("diamond", gradient(GradientKind::Diamond, &stops, around, pad)),
        ("repeating linear", gradient(GradientKind::Linear, &stops, across, GradientSpread::Repeat)),
        ("reflecting radial", gradient(GradientKind::Radial, &stops, around, GradientSpread::Reflect)),
        // A hard edge: two stops at 0.5, the inked half first.
        ("hard edge", gradient(GradientKind::Linear, &[(0.0, inked), (0.5, inked), (0.5, none), (1.0, none)], across, pad)),
        // Starting and ending with no cyan, padding past both ends.
        ("none at the ends", gradient(GradientKind::Linear, &[(0.2, none), (0.4, none), (0.5, inked), (0.8, none)], across, pad)),
    ];
    // Many stretches with no cyan.
    let many: Vec<(f32, Color)> = (0..16).map(|i| (i as f32 / 15.0, if i % 4 < 2 { inked } else { none })).collect();
    let fills = fills.into_iter().chain([("many stops", gradient(GradientKind::Linear, &many, across, pad))]);
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    let cases: Vec<_> = fills.map(|(name, fill)| (name, overprinting(fill, Paint::None), cyan.clone(), "Cyan")).collect();
    assert_keeps_the_ink_below(&cases);
}

/// With Discard White Overprint, a process white fill or stroke marked to overprint knocks out the
/// cyan below it on the Cyan plate and is written without overprinting, as in composite output.
/// Without the setting, and for a white linked to a spot swatch with a color at a tint above 0%,
/// the cyan below it prints.
#[test]
fn white_overprints_knock_out_on_postscript_plates_with_discard_white_overprint() {
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    let white = Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 0.0));
    let spot_white = Paint::Solid { color: Color::cmyk(0.0, 0.0, 0.0, 0.0), swatch: Some("White Ink".into()), tint: 1.0 };
    // The fill is sampled in the rectangle's middle, the stroke on its top edge.
    let (fill_at, stroke_at) = ((100, 50), (100, 10));
    let cases = [
        ("process white fill", overprinting(white.clone(), Paint::None), fill_at, true, true),
        ("process white stroke", overprinting(Paint::None, white.clone()), stroke_at, true, true),
        ("process white fill without the setting", overprinting(white.clone(), Paint::None), fill_at, false, false),
        ("process white stroke without the setting", overprinting(Paint::None, white), stroke_at, false, false),
        ("spot white fill", overprinting(spot_white.clone(), Paint::None), fill_at, true, false),
        ("spot white stroke", overprinting(Paint::None, spot_white), stroke_at, true, false),
    ];
    let mut failed = vec![];
    for (name, art, (x, y), discard, knocks_out) in cases {
        let mut d = over(Some(cyan.clone()), &[art]);
        d.swatches.push(Swatch { name: "White Ink".into(), paint: Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 0.0)), global: true, spot: true });
        d.setup.discard_white_overprint = discard;
        for level in [Level::Three, Level::Two] {
            let (_, printed) = plate_of(&d, "Cyan", level);
            let paper = printed.over_white(x, y)[0] >= 254;
            if paper != knocks_out {
                failed.push(format!("{name} at level {level:?}: knocked out {paper}, expected {knocks_out}"));
            }
            if text(&separation(&d, "Cyan", level).1).contains("true op") {
                failed.push(format!("{name} at level {level:?}: overprints on the plate"));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn an_overprinting_gradient_with_a_hard_stop_at_an_end_keeps_the_ink_below_on_a_postscript_plate() {
    // Two stops at an end, one with cyan and one without: Level 2's bands pad with the end stop,
    // Level 3's shading with the stretch next to it, so each level is checked against its own
    // plate of the gradient alone.
    let (inked, none) = (Color::cmyk(0.6, 0.3, 0.0, 0.0), Color::cmyk(0.0, 0.3, 0.0, 0.0));
    // Each with where it puts cyan at Level 3 and at Level 2: halfway along its vector, before its
    // start and past its end.
    let ends = [
        ("none then cyan at 0", [(0.0, none), (0.0, inked), (1.0, inked)], [[true, true, true], [true, false, true]]),
        ("cyan then none at 1", [(0.0, inked), (1.0, inked), (1.0, none)], [[true, true, true], [true, true, false]]),
        ("cyan then none at 0", [(0.0, inked), (0.0, none), (1.0, none)], [[false, false, false], [false, true, false]]),
        ("none then cyan at 1", [(0.0, none), (1.0, none), (1.0, inked)], [[false, false, false], [false, false, true]]),
    ];
    let c = Point::new(100.0, 50.0);
    let around = GradientGeom { start: c, end: c + Vec2::new(50.0, -10.0), aspect: 0.7, focal: None };
    let across = GradientGeom { start: Point::new(40.0, 40.0), end: Point::new(160.0, 60.0), aspect: 1.0, focal: None };
    let pad = GradientSpread::Pad;
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    // The points probed: halfway along the vector, before its start (a linear gradient only) and
    // past its end, all inside the art.
    let at = |g: GradientGeom, t: f64| g.start + (g.end - g.start) * t;
    let shapes = [
        (GradientKind::Linear, across, "linear", [Some(at(across, 0.5)), Some(at(across, -0.125)), Some(at(across, 1.125))]),
        (GradientKind::Radial, around, "radial", [Some(at(around, 0.5)), None, Some(at(around, 1.2))]),
        (GradientKind::Diamond, around, "diamond", [Some(at(around, 0.5)), None, Some(at(around, 1.2))]),
    ];
    let mut cases = vec![];
    let mut wrong = vec![];
    for (name, stops, inks) in &ends {
        for (kind, geom, shape, probes) in shapes {
            let fill = gradient(kind, stops, geom, pad);
            let art = overprinting(fill, Paint::None);
            // The gradient's own cyan prints at the probes where it puts some down, alone and over
            // the cyan below; elsewhere the plate is paper alone and the cyan below prints.
            for (level, inks) in [Level::Three, Level::Two].into_iter().zip(inks) {
                let (_, alone) = plate_of(&over(None, std::slice::from_ref(&art)), "Cyan", level);
                let (_, printed) = plate_of(&over(Some(cyan.clone()), std::slice::from_ref(&art)), "Cyan", level);
                for (p, ink) in probes.iter().zip(inks).filter_map(|(p, ink)| p.map(|p| (p, *ink))) {
                    let (a, b) = (alone.over_white(p.x as u32, p.y as u32)[0], printed.over_white(p.x as u32, p.y as u32)[0]);
                    let ok = if ink { (60..=150).contains(&a) && (60..=150).contains(&b) } else { a >= 250 && b <= 10 };
                    if !ok {
                        wrong.push(format!("{shape}, {name} at level {level:?} at {p:?}: {a} alone, {b} over the cyan, cyan expected: {ink}"));
                    }
                }
            }
            cases.push((format!("{shape}, {name}"), art, cyan.clone(), "Cyan"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    let cases: Vec<_> = cases.iter().map(|(name, art, under, ink)| (name.as_str(), art.clone(), under.clone(), *ink)).collect();
    assert_keeps_the_ink_below_of(&cases, true);
}

#[test]
fn an_overprinting_diamond_gradient_without_a_frame_keeps_the_ink_below_on_a_postscript_plate() {
    // A vector of no length: the diamond is its color at 1 everywhere. With two stops at 1, that is
    // the first of them, which ends the stretch before it, not the last stop.
    let (inked, none) = (Color::cmyk(0.6, 0.3, 0.0, 0.0), Color::cmyk(0.0, 0.3, 0.0, 0.0));
    let c = Point::new(100.0, 50.0);
    let flat = GradientGeom { start: c, end: c, aspect: 0.7, focal: None };
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    let cases = [
        ("none last", [(0.0, inked), (1.0, inked), (1.0, none)]),
        ("cyan last", [(0.0, none), (1.0, none), (1.0, inked)]),
        ("none throughout", [(0.0, none), (0.5, none), (1.0, none)]),
        ("cyan throughout", [(0.0, inked), (0.5, inked), (1.0, inked)]),
    ];
    let cases: Vec<_> = cases
        .iter()
        .map(|(name, stops)| {
            (*name, overprinting(gradient(GradientKind::Diamond, stops, flat, GradientSpread::Pad), Paint::None), cyan.clone(), "Cyan")
        })
        .collect();
    assert_keeps_the_ink_below(&cases);
}

#[test]
fn an_overprinting_gradient_with_stops_written_between_its_stops_keeps_the_ink_below_on_a_postscript_plate() {
    // None of the cyan from 0.3 to 0.6: the stops written between those two stops (along a
    // Perceptual or hue mix, or at a Classic gradient's off-centre midpoint) are paper white too.
    let (inked, none) = (Color::cmyk(0.6, 0.3, 0.0, 0.0), Color::cmyk(0.0, 0.3, 0.0, 0.0));
    let stops = [(0.0, inked), (0.3, none), (0.6, none), (1.0, inked)];
    let c = Point::new(100.0, 50.0);
    let around = GradientGeom { start: c, end: c + Vec2::new(70.0, -20.0), aspect: 0.7, focal: None };
    let across = GradientGeom { start: Point::new(30.0, 40.0), end: Point::new(170.0, 60.0), aspect: 1.0, focal: None };
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    let shapes = [(GradientKind::Linear, across, "linear"), (GradientKind::Radial, around, "radial"), (GradientKind::Diamond, around, "diamond")];
    let mut cases = vec![];
    for method in [GradientMethod::Classic, GradientMethod::Perceptual, GradientMethod::HueClockwise, GradientMethod::HueCounterclockwise] {
        for (kind, geom, shape) in shapes {
            let mut fill = gradient(kind, &stops, geom, GradientSpread::Pad);
            if let Paint::Gradient(g) = &mut fill {
                g.gradient.method = method;
                g.gradient.stops[1].midpoint = 0.3;
            }
            cases.push((format!("{shape}, {}", method.label()), overprinting(fill, Paint::None), cyan.clone(), "Cyan"));
        }
    }
    let cases: Vec<_> = cases.iter().map(|(name, art, under, ink)| (name.as_str(), art.clone(), under.clone(), *ink)).collect();
    assert_keeps_the_ink_below(&cases);
}

/// [`keeps_the_ink_below`] for each of `cases` (a name, the art, the paint under it and the ink
/// of the plate) at Level 3 and Level 2, with every case that fails named.
fn assert_keeps_the_ink_below(cases: &[(&str, Node, Paint, &str)]) {
    assert_keeps_the_ink_below_of(cases, false);
}

/// [`assert_keeps_the_ink_below`], checked against the PostScript plate of each case's art alone
/// with `postscript`.
fn assert_keeps_the_ink_below_of(cases: &[(&str, Node, Paint, &str)], postscript: bool) {
    let mut failed = vec![];
    for (name, art, under, ink) in cases {
        for level in [Level::Three, Level::Two] {
            let (paper, knocked_out, wrong) = keeps_the_ink_below(std::slice::from_ref(art), under, ink, level, postscript);
            if knocked_out + wrong >= 100 {
                failed.push(format!("{name} at level {level:?}: {knocked_out} of {paper} knocked out, {wrong} of the others differ"));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn overprinting_gradient_strokes_type_freeforms_and_spot_inks_keep_the_ink_below_on_postscript_plates() {
    let (inked, none) = (Color::cmyk(0.6, 0.3, 0.0, 0.0), Color::cmyk(0.0, 0.3, 0.0, 0.0));
    let stops = [(0.0, inked), (0.3, none), (0.6, none), (1.0, inked)];
    let across = GradientGeom { start: Point::new(30.0, 40.0), end: Point::new(170.0, 60.0), aspect: 1.0, focal: None };
    let linear = gradient(GradientKind::Linear, &stops, across, GradientSpread::Pad);
    let stroked = |mode: StrokeGradientMode| {
        let mut n = overprinting(Paint::None, linear.clone());
        if let Some(st) = n.appearance.stroke_mut() {
            st.gradient_mode = mode;
        }
        n
    };
    // Type: a gradient placed on the characters' bounds.
    let on_type = GradientPaint::new(Gradient::new(GradientKind::Linear, stops.iter().map(|(o, c)| GradientStop::new(*o, *c)).collect()));
    let style = CharStyle { fill: Paint::Gradient(Box::new(on_type)), overprint_fill: true, size: 80.0, ..Default::default() };
    let text = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 85.0), "HIM", style))));
    // A freeform gradient with wide spreads of pure color around its points: none of the cyan
    // around the left one.
    let mut freeform = GradientPaint::new(Gradient::new(GradientKind::Freeform, vec![GradientStop::new(0.0, none), GradientStop::new(1.0, inked)]));
    let pure = |at: Point, c: Color| FreeformPoint { spread: 0.9, ..FreeformPoint::new(at, c) };
    let points = vec![pure(Point::new(50.0, 50.0), none), pure(Point::new(150.0, 50.0), inked)];
    freeform.freeform = Some(Freeform { points, ..Default::default() });
    // A spot ink's own plate: its tints in the outer stops, none of it from 0.3 to 0.6.
    let tint = |t: f32| GradientStop { swatch: Some("Ink".into()), tint: t, ..GradientStop::new(0.0, Color::cmyk(0.0, 0.0, 0.0, t)) };
    let spot = [
        GradientStop { offset: 0.0, ..tint(0.8) },
        GradientStop::new(0.3, none),
        GradientStop::new(0.6, none),
        GradientStop { offset: 1.0, ..tint(0.5) },
    ];
    let mut on_spot = GradientPaint::new(Gradient::new(GradientKind::Linear, spot.to_vec()));
    on_spot.geom = Some(across);
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    let full = Paint::Solid { color: Color::cmyk(0.0, 0.0, 0.0, 1.0), swatch: Some("Ink".into()), tint: 1.0 };
    let cases = [
        ("stroke within", stroked(StrokeGradientMode::Within), cyan.clone(), "Cyan"),
        ("stroke along", stroked(StrokeGradientMode::Along), cyan.clone(), "Cyan"),
        ("stroke across", stroked(StrokeGradientMode::Across), cyan.clone(), "Cyan"),
        ("type", text, cyan.clone(), "Cyan"),
        ("freeform", overprinting(Paint::Gradient(Box::new(freeform)), Paint::None), cyan, "Cyan"),
        ("spot", overprinting(Paint::Gradient(Box::new(on_spot)), Paint::None), full, "Ink"),
    ];
    assert_keeps_the_ink_below(&cases);
}

#[test]
fn an_overprinting_freeform_gradient_leaves_out_the_cells_its_samples_write_without_ink_on_a_postscript_plate() {
    // 0.1% cyan around the left point: not paper white, but the plate's 8-bit samples write it as
    // none of the ink, so the cyan below prints there.
    let (faint, inked) = (Color::cmyk(0.001, 0.3, 0.0, 0.0), Color::cmyk(0.6, 0.3, 0.0, 0.0));
    let mut freeform = GradientPaint::new(Gradient::new(GradientKind::Freeform, vec![GradientStop::new(0.0, faint), GradientStop::new(1.0, inked)]));
    let pure = |at: Point, c: Color| FreeformPoint { spread: 0.9, ..FreeformPoint::new(at, c) };
    freeform.freeform =
        Some(Freeform { points: vec![pure(Point::new(50.0, 50.0), faint), pure(Point::new(150.0, 50.0), inked)], ..Default::default() });
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    assert_keeps_the_ink_below(&[("faint", overprinting(Paint::Gradient(Box::new(freeform)), Paint::None), cyan, "Cyan")]);
}

/// A 200 × 100 pt document with `words` at 24 pt in black, in family `family`.
fn typed(words: &str, family: &str) -> Document {
    let mut d = Document::new(200.0, 100.0);
    let layer = d.layers[0].id;
    let style = CharStyle { font_family: family.into(), size: 24.0, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 60.0), words, style))));
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The regular face of `family`.
fn face_of(family: &str) -> std::sync::Arc<vectorcraft_text::FontFace> {
    vectorcraft_text::FontDb::global().face(family, "Regular").unwrap()
}

/// The bundled fallback face as another family `family` (13 characters, like the bundled name),
/// whose license flags (OS/2 `fsType`) are `fs`.
fn licensed(family: &str, fs: u16) -> String {
    let fallback = vectorcraft_text::FALLBACK_FAMILY;
    let mut bytes = face_of(fallback).file_data().to_vec();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16(fallback), utf16(family)), (fallback.as_bytes().to_vec(), family.as_bytes().to_vec())] {
        assert_eq!(from.len(), to.len());
        let mut at = 0;
        while let Some(i) = bytes[at..].windows(from.len()).position(|w| w == from) {
            bytes[at + i..at + i + from.len()].copy_from_slice(&to);
            at += i + from.len();
        }
    }
    let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let os2 = (0..tables).map(|i| 12 + 16 * i).find(|r| &bytes[*r..*r + 4] == b"OS/2").unwrap();
    let off = u32::from_be_bytes(bytes[os2 + 8..os2 + 12].try_into().unwrap()) as usize;
    bytes[off + 8..off + 10].copy_from_slice(&fs.to_be_bytes());
    vectorcraft_text::FontDb::global().add_font(bytes);
    assert!(vectorcraft_text::FontDb::global().has_family(family));
    family.into()
}

/// The job's setup and each page's program.
fn sections(ps: &str) -> (&str, Vec<&str>) {
    let setup = ps.split_once("%%BeginSetup\n").and_then(|(_, rest)| rest.split_once("%%EndSetup")).map_or("", |(setup, _)| setup);
    (setup, ps.split("%%Page: ").skip(1).collect())
}

/// How many glyph procedures the Type 3 fonts in `ps` define.
fn procs(ps: &str) -> usize {
    ps.lines().filter(|l| l.starts_with("/g") && l.ends_with("setcachedevice")).count()
}

fn fonts_job(fonts: PrintFonts) -> PrintJob {
    PrintJob { fonts, ..PrintJob::default() }
}

#[test]
fn type_prints_as_outlines_unless_the_job_sends_fonts() {
    assert_eq!(PrintJob::default().fonts, PrintFonts::Outlines);
    let d = typed("AAB", vectorcraft_text::FALLBACK_FAMILY);
    let ps = text(&print(&[page(&d, None)], &[0], &PrintJob::default()).unwrap());
    assert!(!ps.contains("FontType") && !ps.contains("> show") && !ps.contains("Resources:"), "{ps}");
}

#[test]
fn subset_downloads_used_glyphs_once_per_page() {
    let d = typed("AAB", vectorcraft_text::FALLBACK_FAMILY);
    let sheets = [page(&d, None), PrintPage { ink: Some(("Black", 60.0, 45.0)), ..page(&d, None) }];
    for level in [Level::Two, Level::Three] {
        let out = print(&sheets, &[0, 1, 0], &PrintJob { level, ..fonts_job(PrintFonts::Subset) }).unwrap();
        let ps = text(&out);
        assert!(ps.contains("%%DocumentSuppliedResources: font VCft0.0\n"), "{ps}");
        let (setup, pages) = sections(&ps);
        assert!(!setup.contains("FontType"), "{setup}");
        assert_eq!(pages.len(), 3);
        for p in pages {
            // Each printed page, copies and plates included, defines the font with A and B alone.
            let (page_setup, body) = p.split_once("%%EndPageSetup").unwrap();
            assert_eq!(page_setup.matches("/FontType 3 def").count(), 1, "{page_setup}");
            assert_eq!(procs(page_setup), 2, "{page_setup}");
            assert_eq!(body.matches("> show").count(), 3, "{body}");
            assert!(body.contains("/VCft0.0 findfont"), "{body}");
            assert_eq!(procs(body), 0);
        }
        assert!(out.warnings.is_empty() && out.setting_notes.is_empty(), "{:?} {:?}", out.warnings, out.setting_notes);
    }
    // Read back, the type the font shows draws where its outlines do.
    let area = Rect::new(36.0, 36.0, 236.0, 136.0);
    let drawn = |fonts| {
        let out = print(&sheets[..1], &[0], &fonts_job(fonts)).unwrap();
        render_region(&crate::import(&out.bytes).unwrap().document, area, 2.0)
    };
    let (outlined, shown) = (drawn(PrintFonts::Outlines), drawn(PrintFonts::Subset));
    assert!(outlined.ink() > 200, "{}", outlined.ink());
    let (a, b) = (outlined.ink() as f64, shown.ink() as f64);
    assert!((a - b).abs() < a * 0.05, "outlines {a} px, fonts {b} px");
    vectorcraft_testkit::raster::assert_similar(&outlined, &shown, 24.0, 0.002);
}

#[test]
fn complete_downloads_whole_font_once_before_pages() {
    let d = typed("AAB", vectorcraft_text::FALLBACK_FAMILY);
    let glyphs = face_of(vectorcraft_text::FALLBACK_FAMILY).glyph_count() as usize;
    assert!(glyphs > 256, "{glyphs}");
    let out = print(&[page(&d, None)], &[0, 0], &fonts_job(PrintFonts::Complete)).unwrap();
    let ps = text(&out);
    let (setup, pages) = sections(&ps);
    // Every glyph of the face, in fonts of 256 glyphs, before the first page.
    assert_eq!(procs(setup), glyphs);
    assert_eq!(setup.matches("/FontType 3 def").count(), glyphs.div_ceil(256));
    let header = ps.split_once("%%EndComments").unwrap().0;
    let supplied = header.lines().filter(|l| l.starts_with("%%DocumentSuppliedResources: font ") || l.starts_with("%%+ font VCft")).count();
    assert_eq!(supplied, glyphs.div_ceil(256), "{header}");
    assert_eq!(pages.len(), 2);
    for p in pages {
        assert!(!p.contains("FontType") && procs(p) == 0, "{p}");
        assert_eq!(p.matches("> show").count(), 3);
    }
    assert!(out.warnings.is_empty() && out.setting_notes.is_empty(), "{:?} {:?}", out.warnings, out.setting_notes);
    // A sheet the order doesn't print sends no font, in any mode.
    let plain = doc();
    for fonts in [PrintFonts::Complete, PrintFonts::Subset, PrintFonts::Resident] {
        let out = print(&[page(&plain, None), page(&d, None)], &[0], &fonts_job(fonts)).unwrap();
        let ps = text(&out);
        assert!(!ps.contains("FontType") && !ps.contains("Resources:") && !ps.contains("findfont"), "{fonts:?}: {ps}");
        assert!(out.setting_notes.is_empty(), "{fonts:?}: {:?}", out.setting_notes);
    }
}

#[test]
fn none_references_font_without_downloading_it() {
    let face = face_of(vectorcraft_text::FALLBACK_FAMILY);
    let name = face.postscript_name();
    let d = typed("AAB", vectorcraft_text::FALLBACK_FAMILY);
    let out = print(&[page(&d, None)], &[0, 0], &fonts_job(PrintFonts::Resident)).unwrap();
    let ps = text(&out);
    assert!(!ps.contains("FontType") && !ps.contains("setcachedevice") && !ps.contains("%%BeginResource"), "{ps}");
    assert!(ps.contains(&format!("%%DocumentNeededResources: font {name}\n")), "{ps}");
    let (_, pages) = sections(&ps);
    assert_eq!(pages.len(), 2);
    for p in pages {
        let (page_setup, body) = p.split_once("%%EndPageSetup").unwrap();
        assert!(page_setup.contains(&format!("%%IncludeResource: font {name}\n")), "{page_setup}");
        // The printer's font, re-encoded with the names of the glyphs the page shows.
        assert!(page_setup.contains(&format!("/{name} findfont dup length dict begin")), "{page_setup}");
        assert!(page_setup.contains(" /A put\n") && page_setup.contains(" /B put\n"), "{page_setup}");
        assert_eq!(body.matches("> show").count(), 3, "{body}");
    }
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(out.setting_notes.len(), 1, "{:?}", out.setting_notes);
    assert!(out.setting_notes[0].contains(&name), "{:?}", out.setting_notes);
}

#[test]
fn forbidden_font_stays_art_with_a_note() {
    let locked = licensed("Locked Sans 3", 0x0002);
    let whole = licensed("Entire Sans 3", 0x0100);
    let fonts_of = |family: &str, fonts| {
        let d = typed("AB", family);
        let out = print(&[page(&d, None)], &[0], &fonts_job(fonts)).unwrap();
        (text(&out), out.warnings)
    };
    for fonts in [PrintFonts::Subset, PrintFonts::Complete] {
        let (ps, warnings) = fonts_of(&locked, fonts);
        assert!(!ps.contains("FontType") && !ps.contains("> show"), "{ps}");
        assert!(warnings.iter().any(|w| w.contains(&locked) && w.contains("doesn't allow")), "{warnings:?}");
    }
    // A font that may only be sent whole prints as outlines in a subset job and is sent complete.
    let (ps, warnings) = fonts_of(&whole, PrintFonts::Subset);
    assert!(!ps.contains("FontType") && !ps.contains("> show"), "{ps}");
    assert!(warnings.iter().any(|w| w.contains(&whole) && w.contains("whole")), "{warnings:?}");
    let (ps, warnings) = fonts_of(&whole, PrintFonts::Complete);
    assert!(sections(&ps).0.contains("/FontType 3 def") && ps.contains("> show"), "{ps}");
    assert!(warnings.is_empty(), "{warnings:?}");
    // None sends no font program, so the license doesn't keep it from naming the font.
    let (ps, warnings) = fonts_of(&locked, PrintFonts::Resident);
    assert!(ps.contains("> show") && !ps.contains("FontType"), "{ps}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn complete_large_font_fails_before_partial_output() {
    let face = face_of(vectorcraft_text::FALLBACK_FAMILY);
    let gid = face.glyph_for('A');
    let elements = vectorcraft_text::FontDb::global().outline(&face, gid).elements().len();
    let mut job = fonts::FontJob::new(PrintFonts::Complete);
    let (i, _) = job.glyph(face.id(), gid, elements).unwrap();
    let used = job.every_glyph(&fonts::UsedGlyphs::from([(i, std::collections::BTreeSet::from([gid]))]));
    job.max = 64 << 10;
    let mut s = String::from("before\n");
    let e = job.define(&used, &mut s).unwrap_err();
    assert!(e.contains("64 KB") && e.contains("Subset"), "{e}");
    assert_eq!(s, "before\n");
    // With room, every glyph is defined.
    job.max = fonts::MAX_FONTS;
    job.define(&used, &mut s).unwrap();
    assert_eq!(procs(&s), face.glyph_count() as usize);
}

#[test]
fn type_left_out_for_the_page_limit_sends_none_of_its_glyphs() {
    let family = vectorcraft_text::FALLBACK_FAMILY;
    let face = face_of(family);
    let kept = typed("A", family);
    let mut both = kept.clone();
    let layer = both.layers[0].id;
    let style = CharStyle { font_family: family.into(), size: 24.0, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
    let n = Node::new(both.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 90.0), "BCDEFGH", style))));
    both.insert(Some(layer), 1, n).unwrap();
    let o = EpsOptions { region: kept.artboards[0].rect, ..EpsOptions::default() };
    let walk = |d: &Document, max: usize| {
        let job = RefCell::new(fonts::FontJob::new(PrintFonts::Subset));
        crate::scene::Scene::new(d, &o).with_fonts(Some(&job)).with_max_body(max).run()
    };
    let alone = walk(&kept, usize::MAX);
    assert_eq!(alone.fonts.values().map(|g| g.len()).sum::<usize>(), 1, "{:?}", alone.fonts);
    // The second object passes the limit and is left out: only the glyph of the first is used.
    let page = walk(&both, alone.body.len() + 10);
    assert!(page.warnings.iter().any(|w| w.contains("too large")), "{:?}", page.warnings);
    assert_eq!(page.body, alone.body);
    assert_eq!(page.fonts, alone.fonts);
    assert!(page.fonts.values().all(|g| g.contains(&face.glyph_for('A'))), "{:?}", page.fonts);
}

#[test]
fn ruby_prints_as_outlines_where_the_canvas_draws_it() {
    let family = vectorcraft_text::FALLBACK_FAMILY;
    let mut d = Document::new(200.0, 100.0);
    let layer = d.layers[0].id;
    let style = CharStyle { font_family: family.into(), size: 24.0, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
    let mut t = TextObject::point(Point::new(20.0, 60.0), "AB", style);
    t.ruby = vec![vectorcraft_doc::RubySpan::new(0, 2, "xy")];
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    d.insert(Some(layer), 0, n).unwrap();
    let area = Rect::new(36.0, 36.0, 236.0, 136.0);
    let drawn = |fonts| {
        let out = print(&[page(&d, None)], &[0], &fonts_job(fonts)).unwrap();
        (text(&out), render_region(&crate::import(&out.bytes).unwrap().document, area, 2.0))
    };
    let (_, outlined) = drawn(PrintFonts::Outlines);
    for fonts in [PrintFonts::Subset, PrintFonts::Complete] {
        let (ps, shown) = drawn(fonts);
        // The base text shows through the font; its reading is drawn as outlines, in place.
        assert_eq!(ps.matches("> show").count(), 2, "{fonts:?}: {ps}");
        if fonts == PrintFonts::Subset {
            assert_eq!(procs(&ps), 2, "{ps}");
        }
        vectorcraft_testkit::raster::assert_similar(&outlined, &shown, 24.0, 0.002);
    }
}
