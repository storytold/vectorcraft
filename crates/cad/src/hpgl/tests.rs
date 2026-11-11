//! HP-GL import: units and the y axis, pens, runs, curves, filled and edged areas, polygon mode,
//! line types, scaling, rotation, encoded polylines, escapes around the instructions, what is
//! skipped with a warning, which files are taken to be HP-GL, and hostile files.

use kurbo::{BezPath, PathEl, Point, Shape as _};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, LineCap, LineJoin, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::FillRule;

use super::plot::{FAR_AWAY, HATCHES, LABELS, PAGES, RASTER, SYMBOLS, TOO_MANY_POINTS, WINDOW};
use super::*;

/// Points per millimeter.
const MM: f64 = 72.0 / 25.4;

fn plot(s: &str) -> Imported {
    plot_bytes(s.as_bytes())
}

fn plot_bytes(b: &[u8]) -> Imported {
    match import(b) {
        Ok(i) => i,
        Err(e) => panic!("{e}: {:?}", String::from_utf8_lossy(b)),
    }
}

/// Every object, in paint order.
fn leaves(doc: &Document) -> Vec<Node> {
    let mut out = vec![];
    doc.walk(|n| {
        if n.children().is_none() {
            out.push(n.clone());
        }
    });
    out
}

fn bez(n: &Node) -> BezPath {
    match &n.kind {
        NodeKind::Path { path, .. } => path.to_bezpath(),
        k => panic!("a path, got {k:?}"),
    }
}

fn stroke(n: &Node) -> StrokeLayer {
    n.appearance.items.iter().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.clone()) } else { None }).expect("a stroke")
}

fn fill(n: &Node) -> Option<Paint> {
    n.appearance.items.iter().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f.paint.clone()) } else { None })
}

fn rule(n: &Node) -> FillRule {
    match &n.kind {
        NodeKind::Path { rule, .. } => *rule,
        k => panic!("a path, got {k:?}"),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Paint {
    Paint::solid(Color::rgb8(r, g, b))
}

/// The end points of `bp`'s elements.
fn ends(bp: &BezPath) -> Vec<Point> {
    bp.elements().iter().filter_map(|e| e.end_point()).collect()
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

fn assert_points(got: &[Point], want: &[(f64, f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(near(*g, Point::new(w.0, w.1)), "{got:?} != {want:?}");
    }
}

/// A PE number as the reference guide's encoding writes it (base 64, or base 32 for 7-bit data).
fn pe_number(v: i64, seven_bit: bool) -> Vec<u8> {
    let mut n = if v >= 0 { 2 * v as u64 } else { 2 * v.unsigned_abs() + 1 };
    let (base, high) = if seven_bit { (32, 95) } else { (64, 191) };
    let mut out = vec![];
    while n >= base {
        out.push(63 + (n % base) as u8);
        n /= base;
    }
    out.push(high + n as u8);
    out
}

#[test]
fn plotter_units_become_points_with_y_down() {
    // An inch right and an inch up, with the thinnest pen (0.25 pt): the artboard reaches 0.125 pt
    // past the line at the corner's miter, and stops at the line's butt ends.
    let i = plot("IN;SP1;PW0;PU0,0;PD1016,0,1016,1016;PU;");
    let doc = &i.document;
    let r = doc.artboards[0].rect;
    assert!((r.width() - 72.125).abs() < 1e-9 && (r.height() - 72.125).abs() < 1e-9, "{r:?}");
    let shapes = leaves(doc);
    assert_eq!(shapes.len(), 1);
    assert_points(&ends(&bez(&shapes[0])), &[(0.0, 72.0), (72.0, 72.0), (72.0, 0.0)]);
    let s = stroke(&shapes[0]);
    assert_eq!((s.width, s.paint.clone(), s.cap, s.join, s.miter_limit), (0.25, rgb(0, 0, 0), LineCap::Butt, LineJoin::Miter, 5.0));
    assert!(fill(&shapes[0]).is_none() && i.warnings.is_empty(), "{:?}", i.warnings);

    // The artboard holds what the strokes paint, the tip of a sharp miter join included.
    let i = plot("IN;SP1;PW2;PD2032,0,0,1000;");
    let r = i.document.artboards[0].rect;
    let painted = leaves(&i.document)[0].visual_bounds().unwrap();
    let path = bez(&leaves(&i.document)[0]).bounding_box();
    assert!(painted.x1 - path.x1 > 2.0 * MM, "the miter reaches past half the width: {painted:?} {path:?}");
    let inside = r.x0 <= painted.x0 + 1e-9 && r.y0 <= painted.y0 + 1e-9 && r.x1 >= painted.x1 - 1e-9 && r.y1 >= painted.y1 - 1e-9;
    assert!(inside, "{r:?} {painted:?}");
}

#[test]
fn pens_pick_colors_and_widths() {
    // Before any SP the pen is pen 1; SP0 draws nothing; pens past the palette wrap round it.
    let i = plot("IN;PD0,0,100,0;PU;SP2;PD0,100,100,100;PU;SP0;PD0,200,100,200;PU;SP9;PD0,300,100,300;PU;SP3;PD0,400,100,400;PU;");
    let colors: Vec<Paint> = leaves(&i.document).iter().map(|n| stroke(n).paint).collect();
    assert_eq!(colors, [rgb(0, 0, 0), rgb(255, 0, 0), rgb(255, 0, 0), rgb(0, 255, 0)]);

    // PC gives a pen a color in the color range; PC with the pen alone gives back its default.
    let i = plot("IN;CR0,1,0,1,0,1;PC3,0,0,1;SP3;PD0,0,10,0;PU;PC3;PD0,10,10,10;PU;");
    let colors: Vec<Paint> = leaves(&i.document).iter().map(|n| stroke(n).paint).collect();
    assert_eq!(colors, [rgb(0, 0, 255), rgb(0, 255, 0)]);

    // PW in millimeters for every pen, or for one; IN gives back 0.35 mm.
    let i = plot("IN;SP1;PW1;PD0,0,10,0;PU;PW2,2;SP2;PD0,10,10,10;PU;SP1;PD0,20,10,20;PU;IN;SP1;PD0,30,10,30;");
    let widths: Vec<f64> = leaves(&i.document).iter().map(|n| stroke(n).width).collect();
    let want = [MM, 2.0 * MM, MM, 0.35 * MM];
    assert!(widths.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "{widths:?}");
}

#[test]
fn each_run_with_the_pen_down_is_one_path() {
    // Relative moves; a run that comes back to its start is closed; a pen-up move elsewhere starts
    // another path, and one back to where the line ended continues it.
    let i = plot("IN;SP1;PW0;PA0,0;PR;PD1016,0,0,1016,-1016,0,0,-1016;PU2032,0;PD0,1016;PU0,1016;PU0,-1016;PD508,0;PU0,500;PD0,500;");
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 3, "{shapes:#?}");
    let NodeKind::Path { path: square, .. } = &shapes[0].kind else { panic!("a path") };
    assert_eq!((square.subpaths.len(), square.subpaths[0].anchors.len(), square.subpaths[0].closed), (1, 4, true));
    assert_eq!(ends(&bez(&shapes[1])).len(), 3, "the run went on from where it ended");
    assert_eq!(ends(&bez(&shapes[2])).len(), 2);

    // A pen-down move of no length draws a dot.
    let i = plot("IN;SP1;PA500,500;PD;PA500,500;PU;");
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 1);
    let b = bez(&shapes[0]).bounding_box();
    assert!((b.width() - 72.0 / 1016.0).abs() < 1e-9, "{b:?}");
}

#[test]
fn circles_arcs_and_curves_are_drawn_as_curves() {
    // A circle of an inch round the pen, which stays at its center (the next line starts there).
    let i = plot("IN;SP1;PW0;PA2032,2032;CI1016;PD3048,2032;");
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 2);
    let c = bez(&shapes[0]);
    assert!(c.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))) && matches!(c.elements().last(), Some(PathEl::ClosePath)));
    // The curves stay within a thousandth of a point of the circle.
    let b = c.bounding_box();
    assert!((b.width() - 144.0).abs() < 1e-3 && (b.height() - 144.0).abs() < 1e-3, "{b:?}");
    let line = ends(&bez(&shapes[1]));
    assert!((line[0] - b.center()).hypot() < 1e-3, "{line:?} {b:?}");

    // AA: a quarter turn counterclockwise round (0, 0) from (1016, 0) ends at (0, 1016), and the
    // pen goes on from there. AR takes its center relative to the pen.
    for s in ["IN;SP1;PW0;PA1016,0;PD;AA0,0,90;PA0,2032;", "IN;SP1;PW0;PA1016,0;PD;AR-1016,0,90;PA0,2032;"] {
        let shapes = leaves(&plot(s).document);
        assert_eq!(shapes.len(), 1, "{s}");
        let p = bez(&shapes[0]);
        let pts = ends(&p);
        // y down: (1016, 0) is at the bottom right, the arc's end on the left a quarter turn up.
        let (start, end) = (pts[0], pts[pts.len() - 2]);
        assert!(near(start, Point::new(72.125, 144.0)) && near(end, Point::new(0.125, 72.0)), "{s}: {pts:?}");
        assert!(near(*pts.last().unwrap(), Point::new(0.125, 0.0)), "{s}: {pts:?}");
    }

    // AT: the arc from the pen through a point to an end; points in a line give a line.
    let shapes = leaves(&plot("IN;SP1;PW0;PA1016,0;PD;AT0,1016,-1016,0;").document);
    let pts = ends(&bez(&shapes[0]));
    let b = bez(&shapes[0]).bounding_box();
    assert!((b.width() - 144.0).abs() < 1e-6 && (b.height() - 72.0).abs() < 1e-6, "a half circle: {b:?}");
    assert!(near(pts[0], Point::new(144.125, 72.125)) && near(*pts.last().unwrap(), Point::new(0.125, 72.125)), "{pts:?}");
    let shapes = leaves(&plot("IN;SP1;PW0;PA0,0;PD;AT508,0,1016,0;").document);
    assert_eq!(ends(&bez(&shapes[0])).len(), 2);

    // BZ and BR: cubic Béziers, BR's points relative to each curve's start.
    for s in [
        "IN;SP1;PW0;PA0,0;PD;BZ0,1016,1016,1016,1016,0,1016,-1016,2032,-1016,2032,0;",
        "IN;SP1;PW0;PA0,0;PD;BR0,1016,1016,1016,1016,0,0,-1016,1016,-1016,1016,0;",
    ] {
        let shapes = leaves(&plot(s).document);
        let els = bez(&shapes[0]).elements().to_vec();
        assert_eq!(els.iter().filter(|e| matches!(e, PathEl::CurveTo(..))).count(), 2, "{s}");
        // The curves reach 762 plotter units (three quarters of their control points) up and down.
        assert!(near(els.last().and_then(PathEl::end_point).unwrap(), Point::new(144.125, 54.125)), "{s}: {els:?}");
    }
}

#[test]
fn rectangles_wedges_and_polygons_fill_and_edge() {
    // RA fills, EA edges; neither moves the pen.
    let i = plot("IN;SP2;PW0;PA0,0;RA1016,1016;EA1016,1016;PD508,0;");
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 3);
    assert_eq!((fill(&shapes[0]), shapes[0].appearance.items.len()), (Some(rgb(255, 0, 0)), 1));
    assert!(fill(&shapes[1]).is_none() && stroke(&shapes[1]).paint == rgb(255, 0, 0));
    assert!(near(ends(&bez(&shapes[2]))[0], Point::new(0.125, 72.125)), "the line starts where the pen was");

    // WG fills a wedge, EW edges one; a whole turn is a circle.
    let i = plot("IN;SP1;PA0,0;WG1016,0,90;EW1016,0,360;");
    let shapes = leaves(&i.document);
    let wedge = bez(&shapes[0]);
    assert!(fill(&shapes[0]).is_some() && ends(&wedge).len() >= 3);
    let ring = bez(&shapes[1]).bounding_box();
    assert!((ring.width() - 144.0).abs() < 1e-6);

    // Polygon mode: two squares, one inside the other; FP fills even-odd by default, FP1
    // non-zero; EP edges only what was drawn with the pen down.
    let poly = "IN;SP1;PW0;PA0,0;PM0;PD3048,0,3048,3048,0,3048,0,0;PM1;PU1016,1016;PD2032,1016;PU2032,2032;PD1016,2032,1016,1016;PM2;";
    let i = plot(&format!("{poly}FP;FP1;EP;"));
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 3);
    assert_eq!((rule(&shapes[0]), rule(&shapes[1])), (FillRule::EvenOdd, FillRule::NonZero));
    let filled = bez(&shapes[0]);
    assert_eq!(filled.elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count(), 2);
    let edged = bez(&shapes[2]);
    assert_eq!(edged.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(..))).count(), 3, "the pen-up side isn't edged: {edged:?}");
    // Nothing is drawn in polygon mode itself.
    assert!(leaves(&plot(poly).document).is_empty());

    // A circle in polygon mode is a subpolygon of its own; CI and RA after PM2 work as before.
    let i = plot("IN;SP1;PA2032,2032;PM0;CI1016;PM2;FP;");
    assert_eq!(leaves(&i.document).len(), 1);

    // Fill types: shading mixes the pen with white; hatching fills solid with a warning.
    let i = plot("IN;SP5;PA0,0;FT10,50;RA100,100;FT3,10,45;RA200,200;");
    let shapes = leaves(&i.document);
    assert_eq!((fill(&shapes[0]), fill(&shapes[1])), (Some(rgb(128, 128, 255)), Some(rgb(0, 0, 255))));
    assert_eq!(i.warnings, [HATCHES]);
}

#[test]
fn line_types_become_dash_patterns() {
    // A 10 mm pattern of line type 2 is 5 mm on, 5 mm off.
    let i = plot("IN;SP1;LT2,10,1;PD0,0,4000,0;");
    let d = stroke(&leaves(&i.document)[0]).dash.expect("dashed");
    assert!(d.pattern.iter().all(|v| (v - 5.0 * MM).abs() < 1e-9) && d.pattern.len() == 2 && !d.align_corners, "{d:?}");
    // The default length is 4% of the P1-P2 diagonal; IP sets P1 and P2.
    let i = plot("IN;IP0,0,3000,4000;SP1;LT3;PD0,0,4000,0;");
    let d = stroke(&leaves(&i.document)[0]).dash.expect("dashed");
    let total = 200.0 * 72.0 / 1016.0;
    assert!((d.pattern[0] - 0.7 * total).abs() < 1e-9 && (d.pattern[1] - 0.3 * total).abs() < 1e-9, "{d:?}");
    // Adaptive types stretch to each line; a zero length is a dot one plotter unit long.
    let i = plot("IN;SP1;LT-4,10,1;PD0,0,4000,0;");
    let d = stroke(&leaves(&i.document)[0]).dash.expect("dashed");
    assert!(d.align_corners && (d.pattern[2] - 72.0 / 1016.0).abs() < 1e-12, "{d:?}");
    // UL redefines a type; LT; draws solid and LT99 gives the type back.
    let i = plot("IN;SP1;UL2,1,3;LT2,10,1;PD0,0,4000,0;PU;LT;PD0,100,4000,100;PU;LT99;PD0,200,4000,200;");
    let shapes = leaves(&i.document);
    let d = stroke(&shapes[0]).dash.expect("dashed");
    assert!((d.pattern[0] - 2.5 * MM).abs() < 1e-9 && (d.pattern[1] - 7.5 * MM).abs() < 1e-9, "{d:?}");
    assert!(stroke(&shapes[1]).dash.is_none() && stroke(&shapes[2]).dash.is_some());
    // Line type 0 draws a dot at each point.
    let i = plot("IN;SP1;LT0;PD0,0,1000,0,2000,0;");
    assert_eq!(leaves(&i.document).len(), 3);
    // LA sets the caps, joins and miter limit.
    let s = stroke(&leaves(&plot("IN;SP1;LA1,4,2,4,3,9;PD0,0,100,0;").document)[0]);
    assert_eq!((s.cap, s.join, s.miter_limit), (LineCap::Round, LineJoin::Round, 9.0));
}

#[test]
fn scaling_maps_user_units_onto_p1_and_p2() {
    // Anisotropic: 100 user units across an inch.
    let size = |s: &str| {
        let shapes = leaves(&plot(s).document);
        bez(&shapes[0]).bounding_box().size()
    };
    let s = size("IN;IP0,0,1016,2032;SC0,100,0,100;SP1;PD0,0,100,100;");
    assert!((s.width - 72.0).abs() < 1e-9 && (s.height - 144.0).abs() < 1e-9, "{s:?}");
    // Isotropic: the smaller of the two scales on both axes.
    let s = size("IN;IP0,0,1016,2032;SC0,100,0,100,1;SP1;PU0,0;PD100,100;");
    assert!((s.width - 72.0).abs() < 1e-9 && (s.height - 72.0).abs() < 1e-9, "{s:?}");
    // Point factor: 10 plotter units per user unit.
    let s = size("IN;SC0,10,0,10,2;SP1;PD0,0,101.6,50.8;");
    assert!((s.width - 72.0).abs() < 1e-9 && (s.height - 36.0).abs() < 1e-9, "{s:?}");
    // Without IP, P2 lies 11 × 8.5 in from the origin.
    let s = size("IN;SC0,11,0,8.5;SP1;PD0,0,1,1;");
    assert!((s.width - 72.0).abs() < 1e-9 && (s.height - 72.0).abs() < 1e-9, "{s:?}");
    // SC; goes back to plotter units; scaling that makes no area is ignored.
    let s = size("IN;SC0,1,0,1;SC;SC0,0,0,1;SP1;PD0,0,1016,1016;");
    assert!((s.width - 72.0).abs() < 1e-9, "{s:?}");
    // RO turns the coordinate system: x runs up.
    let shapes = leaves(&plot("IN;RO90;SP1;PW0;PA0,0;PD1016,0;").document);
    let b = bez(&shapes[0]).bounding_box();
    assert!(b.width() < 1e-9 && (b.height() - 72.0).abs() < 1e-9, "{b:?}");
}

#[test]
fn polyline_encoded_data_is_read() {
    let mut f = b"IN;SP1;PW0;PE".to_vec();
    // Pen 2, a pen-up move to the absolute (1016, 0), then a relative line an inch up.
    f.push(b':');
    f.extend(pe_number(2, false));
    f.extend(b"<=");
    f.extend(pe_number(1016, false));
    f.extend(pe_number(0, false));
    f.extend(pe_number(0, false));
    f.extend(pe_number(1016, false));
    f.push(b';');
    let i = plot_bytes(&f);
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 1);
    assert_eq!(stroke(&shapes[0]).paint, rgb(255, 0, 0));
    assert_points(&ends(&bez(&shapes[0])), &[(0.125, 72.0), (0.125, 0.0)]);

    // 7-bit data with two fractional bits: 4064 / 4 = 1016.
    let mut f = b"IN;SP1;PW0;PE7>".to_vec();
    f.extend(pe_number(2, true));
    f.extend(pe_number(-4064, true));
    f.extend(pe_number(0, true));
    f.push(b';');
    let b = bez(&leaves(&plot_bytes(&f).document)[0]).bounding_box();
    assert!((b.width() - 72.0).abs() < 1e-9, "{b:?}");

    // Data that can't be decoded ends the instruction at its semicolon: none of it is read as
    // instructions.
    let shapes = leaves(&plot("IN;SP1;PE#PD5000,5000;PD1016,0;").document);
    assert_eq!(shapes.len(), 1);
    assert!((bez(&shapes[0]).bounding_box().width() - 72.0).abs() < 1e-9);

    // Rectangle mode: a pen-up move, then a filled rectangle relative to it.
    let mut f = b"IN;SP3;PE9".to_vec();
    for v in [100, 100, 1016, 508] {
        f.extend(pe_number(v, false));
    }
    f.push(b';');
    let shapes = leaves(&plot_bytes(&f).document);
    assert_eq!(fill(&shapes[0]), Some(rgb(0, 255, 0)));
    let b = bez(&shapes[0]).bounding_box();
    assert!((b.width() - 72.0).abs() < 1e-9 && (b.height() - 36.0).abs() < 1e-9, "{b:?}");
}

#[test]
fn printer_escapes_and_job_lines_are_skipped() {
    let mut f = b"\x1b%-12345X@PJL JOB\r\n@PJL ENTER LANGUAGE=PCL\r\n\x1bE\x1b%0BIN;SP1;PW0;PD0,0,1016,0;PU;\x1b%0A".to_vec();
    // An HP RTL raster image whose data spells out instructions.
    f.extend(b"\x1b*r1A\x1b*b11WPD9999,9999\x1b*rC");
    f.extend(b"\x1b%1BPD1016,1016;\x1b%-12345X@PJL EOJ\r\n");
    assert!(is_hpgl(&f));
    let i = plot_bytes(&f);
    let shapes = leaves(&i.document);
    // The pen went up and came down where the line ended, so the line goes on.
    assert_eq!(shapes.len(), 1, "{shapes:#?}");
    assert_eq!(ends(&bez(&shapes[0])).len(), 3);
    assert_eq!(i.warnings, [RASTER], "{:?}", i.warnings);
    let r = i.document.artboards[0].rect;
    assert!((r.width() - 72.125).abs() < 1e-9 && (r.height() - 72.125).abs() < 1e-9, "{r:?}");

    // The device-control escapes of older plotters.
    let f = b"\x1b.(\x1b.I81;;17:\x1b.N;19:IN;SP1;PD0,0,1016,0;\x1b.)";
    assert!(is_hpgl(f));
    assert_eq!(leaves(&plot_bytes(f).document).len(), 1);
}

#[test]
fn labels_comments_and_unknown_instructions_are_skipped() {
    // A label's text, up to its terminator (ETX, or the one DT sets), isn't read as instructions.
    let i = plot("IN;SP1;LBPD9999,9999\x03DT*,1;LBPU0,0;PD5000,5000*CO\"PD8000,8000\";PD1016,0;PU;ZZ1,2;QQ;");
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 1);
    assert_eq!(i.warnings, [LABELS.to_string(), "2 instructions of kinds VectorCraft doesn't read were skipped".to_string()]);
    // Symbol mode's symbols aren't drawn, with a warning; its lines are.
    let i = plot("IN;SP1;SM*;PD100,0;SM;");
    assert_eq!((i.warnings.clone(), leaves(&i.document).len()), (vec![SYMBOLS.to_string()], 1));
    // Later pages and the input window are warned about.
    let i = plot("IN;IW0,0,10,10;SP1;PD100,0;PU;PG;PD0,100;PG;");
    assert_eq!(i.warnings, [WINDOW, PAGES]);
    // A page advance raises the pen at the origin: nothing joins the pages.
    let i = plot("IN;SP1;PD100,0;PG;PA200,0;PD300,0;");
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 2);
    assert_eq!(ends(&bez(&shapes[1])).len(), 2, "the second page's line starts at (200, 0)");
    // A page advance at the end changes nothing.
    assert!(plot("IN;SP1;PD100,0;PG;").warnings.is_empty());
    // A file that draws nothing opens as a letter-size page.
    let i = plot("IN;SP1;PU100,100;");
    assert!(leaves(&i.document).is_empty() && i.document.artboards[0].rect.size() == kurbo::Size::new(612.0, 792.0));
}

#[test]
fn only_plot_files_are_taken_for_hpgl() {
    for f in [
        &b"IN;SP1;PU0,0;PD100,100;"[..],
        b"in;sp1;pu0,0;pd100,100;",
        b";IN;\r\nSP1;\r\nPA100,100;",
        b"\x03IN;",
        b"SP1PD0,0,10,10PU",
        b"IN",
        b"CO\"made by hand\";IN;",
        b"IN;SP1;DT*;LBhello world*PA0,0;SMx;PA1,1;SM;",
        b"IN;SP1;PE<=\xbf\xbf;PU;",
        b"\x1b%-12345X@PJL ENTER LANGUAGE=HPGL2\nIN;SP1;",
    ] {
        assert!(is_hpgl(f), "{:?}", String::from_utf8_lossy(f));
    }
    for f in [
        &b""[..],
        b"IN THE BEGINNING",
        b"set terminal png\nplot sin(x)\n",
        b"Paris, 10 to 12",
        b"SPDX-License-Identifier: MIT",
        b"CONTENTS",
        b"CO \"a comment\";hello world",
        b"DT*;LBtext*nope",
        b"\x89PNG\r\n\x1a\n",
        b"IN;SP1;PD0,0;hello world",
        b"\x1b%-12345X@PJL ENTER LANGUAGE=PCL\n\x1bE\x1b*r1A",
    ] {
        assert!(!is_hpgl(f), "{:?}", String::from_utf8_lossy(f));
        assert_eq!(import(f).err().as_deref(), Some("not an HP-GL file"), "{:?}", String::from_utf8_lossy(f));
    }
    // A comment needs no string; instructions of an older or a device's own set count when a
    // semicolon ends them.
    for f in [&b"CO;IN;SP1;PD0,0,10,10;"[..], b"IN;CO;SP1;PD0,0,10,10;", b"IN;SP1;VA;VS10;PU0,0;PD1016,0;", b"IN;GC5;SP1;PD0,0,10,0;"] {
        assert!(is_hpgl(f), "{:?}", String::from_utf8_lossy(f));
    }
    assert!(!is_hpgl(b"IN;GC5") && !is_hpgl(b"IN;Hello, world;"));
    // A printer prolog of any length comes before the first instruction.
    let long = [&b"\x1b%-12345X"[..], &b"@PJL COMMENT a job line of some length\r\n".repeat(400), b"\x1bE\x1b%0BIN;SP1;PD100,0;"].concat();
    assert!(long.len() > 16_000 && is_hpgl(&long));
    assert_eq!(leaves(&plot_bytes(&long).document).len(), 1);
    // Instructions of any kind may follow the first eight, garbage included.
    assert!(is_hpgl(b"IN;SP1;PU;PD;PU;PD;PU;PD;hello world"));
}

#[test]
fn escapes_that_carry_values_or_data_are_skipped_whole() {
    // A reset leaves HP-GL until a language switch; symbol-set escapes carry a value and a letter.
    for f in [
        &b"\x1bE\x1b(8U\x1b%1BIN;SP1;PD0,0,1016,0;"[..],
        b"\x1bE\x1b(10U\x1b)0N\x1b%1BIN;SP1;PD0,0,1016,0;",
        b"\x1bEHello\x1b%1BIN;SP1;PD0,0,1016,0;",
    ] {
        assert!(is_hpgl(f), "{:?}", String::from_utf8_lossy(f));
        assert_eq!(leaves(&plot_bytes(f).document).len(), 1, "{:?}", String::from_utf8_lossy(f));
    }
    // A letter left over from an escape doesn't join the next mnemonic.
    let i = plot_bytes(b"IN;SP1;\x1b(8UPD0,0,1016,0;");
    assert!(i.warnings.is_empty() && leaves(&i.document).len() == 1, "{:?}", i.warnings);
    // Raster data by plane and transparent print data are skipped by their byte counts, however
    // they read.
    let line = |data_escape: &[u8]| {
        let data = b"\x1b%1BPD0,5000;";
        let mut f = b"IN;SP1;PW0;PD0,0,0,1016;\x1b%0A".to_vec();
        f.extend(data_escape);
        f.extend(data.len().to_string().as_bytes());
        f.extend(if data_escape.ends_with(b"b") { &b"V"[..] } else { b"X" });
        f.extend(data);
        f.extend(b"\x1b*rC\x1b%1BPD0,0;");
        bez(&leaves(&plot_bytes(&f).document)[0]).bounding_box()
    };
    for escape in [&b"\x1b*r1A\x1b*b"[..], b"\x1b&p"] {
        let b = line(escape);
        assert!((b.height() - 72.0).abs() < 1e-9, "{:?}: {b:?}", String::from_utf8_lossy(escape));
    }
}

#[test]
fn polygon_mode_follows_its_own_rules() {
    // The point after a circle starts a new subpolygon: the triangle after it doesn't reach back
    // to the circle's center, filled or edged.
    let i = plot("IN;SP1;PW0;PA1000,1000;PM0;CI200;PD2000,1000,2000,2000,1000,2000;PM2;FP;EP;");
    let shapes = leaves(&i.document);
    assert_eq!(shapes.len(), 2);
    for s in &shapes {
        let p = bez(s);
        let first = p.elements().iter().position(|e| matches!(e, PathEl::ClosePath)).unwrap_or(p.elements().len());
        let circle = BezPath::from_vec(p.elements()[..first].to_vec()).bounding_box();
        let center = circle.center();
        assert!(ends(&p).iter().all(|e| !near(*e, center)), "{p:?}");
    }
    // PM1 closes with the pen up: EP draws two sides of the triangle, not the third; PM2 closes
    // with the pen as it is.
    let sides = |s: &str| {
        let p = bez(&leaves(&plot(s).document)[0]);
        p.elements().iter().filter(|e| matches!(e, PathEl::LineTo(..))).count()
    };
    assert_eq!(sides("IN;SP1;PA0,0;PM0;PD1000,0,1000,1000;PM1;PM2;EP;"), 2);
    assert_eq!(sides("IN;SP1;PA0,0;PM0;PD1000,0,1000,1000;PM2;EP;"), 3);
    // The closing point moves the pen: a relative move after PM1 starts from the start.
    let i = plot("IN;SP1;PA0,0;PM0;PD1000,0,1000,1000;PM1;PU;PR0,2000;PD1000,0,0,1000;PM2;FP;");
    let b = bez(&leaves(&i.document)[0]).bounding_box();
    assert!((b.height() - 3000.0 * PT_PER_PLU).abs() < 1e-9, "{b:?}");
    // A subpolygon of fewer than three points (the closing point counted) is neither filled nor
    // edged.
    assert!(leaves(&plot("IN;SP1;PA0,0;PM0;PD0,0;PM2;FP;EP;").document).is_empty());
    let i = plot("IN;SP1;PA0,0;PM0;PR0,0;PM1;PA;PD1000,0,0,1000,0,0;PM2;FP;");
    let filled = bez(&leaves(&i.document)[0]);
    assert_eq!(filled.elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count(), 1, "{filled:?}");
    // Polygon mode ignores the instructions it doesn't allow: pens, scaling and labels.
    let i = plot("IN;SP1;PA0,0;PM0;SP2;SC0,1,0,1;LBPD9999,9999\x03PD1016,0,1016,1016;PM2;FP;");
    let shapes = leaves(&i.document);
    assert_eq!((shapes.len(), fill(&shapes[0])), (1, Some(rgb(0, 0, 0))));
    assert!((bez(&shapes[0]).bounding_box().width() - 72.0).abs() < 1e-9 && i.warnings.is_empty(), "{:?}", i.warnings);
}

#[test]
fn rotation_keeps_the_pen_and_turns_the_defaults() {
    // After RO90, IP's default P2 is the rotated hard-clip corner: the user square spans 8.5 in
    // across its rotated x axis and 11 in up it, which lie up and across the page.
    let i = plot("IN;RO90;IP;SC0,100,0,100;SP1;PW0;PA0,0;PD100,0,100,100,0,100,0,0;");
    let b = bez(&leaves(&i.document)[0]).bounding_box();
    assert!((b.width() - 792.0).abs() < 1e-9 && (b.height() - 612.0).abs() < 1e-9, "{b:?}");
    // IR's percentages are of the rotated limits too.
    let i = plot("IN;RO90;IR0,0,100,100;SC0,1,0,1;SP1;PW0;PA0,0;PD1,1;");
    let b = bez(&leaves(&i.document)[0]).bounding_box();
    assert!((b.width() - 792.0).abs() < 1e-9 && (b.height() - 612.0).abs() < 1e-9, "{b:?}");
    // The pen stays where it was on the page: a line after RO starts where the one before ended.
    let i = plot("IN;SP1;PW0;PD0,0,1016,0;PU;RO90;PD;PR1016,0;");
    let all = leaves(&i.document).iter().map(|n| bez(n).bounding_box()).reduce(|a, b| a.union(b)).unwrap();
    assert!((all.width() - 72.0).abs() < 1e-9 && (all.height() - 72.0).abs() < 1e-9, "{all:?}");
}

#[test]
fn line_type_zero_and_odd_patterns_follow_the_reference() {
    // Line type 0: one dot for an arc (at its end) and one for a circle (at its center).
    for s in ["IN;SP1;LT0;PA0,0;PD;AA1016,0,180;", "IN;SP1;LT0;PA0,0;CI500;"] {
        let shapes = leaves(&plot(s).document);
        assert_eq!(shapes.len(), 1, "{s}");
        assert!(bez(&shapes[0]).bounding_box().width() < 0.1, "{s}");
    }
    // An odd number of gaps repeats with the last pen-down stretch running into the first.
    let i = plot("IN;SP1;UL3,50,25,25;LT3,10,1;PD0,0,4000,0;");
    let d = stroke(&leaves(&i.document)[0]).dash.expect("dashed");
    assert!((d.pattern[0] - 7.5 * MM).abs() < 1e-9 && (d.pattern[1] - 2.5 * MM).abs() < 1e-9 && d.pattern.len() == 2, "{d:?}");
    assert!((d.offset - 2.5 * MM).abs() < 1e-9, "{d:?}");
    // One pen-down stretch is a solid line; a dot keeps the pattern's length.
    let d = stroke(&leaves(&plot("IN;SP1;UL3,100;LT3,10,1;PD0,0,4000,0;").document)[0]).dash.expect("dashed");
    assert_eq!(d.pattern[1], 0.0);
    let d = stroke(&leaves(&plot("IN;SP1;LT4,10,1;PD0,0,4000,0;").document)[0]).dash.expect("dashed");
    assert!((d.pattern.iter().sum::<f64>() - 10.0 * MM).abs() < 1e-9, "{d:?}");
}

#[test]
fn pen_colors_ignore_two_or_three_parameters() {
    let i = plot("IN;PC3,0,0,255;PC3,255;PC3,255,0;SP3;PD0,0,10,0;");
    assert_eq!(stroke(&leaves(&i.document)[0]).paint, rgb(0, 0, 255));
}

#[test]
fn hostile_files_import_without_panics() {
    let mut cases: Vec<Vec<u8>> = vec![
        // Coordinates past any plotter's range, and numbers with too many digits.
        b"IN;SP1;PD99999999999999999999999999999,0;PU0,0;PD1016,1016;".to_vec(),
        b"IN;SP1;PD0.000000000000000000000000001,1e999,--,++,..;".to_vec(),
        // Unterminated labels, comments and encoded polylines.
        b"IN;SP1;LBno end".to_vec(),
        b"IN;CO\"no end".to_vec(),
        b"IN;SP1;PE<=\xff\xff\xff\xfe\xfe".to_vec(),
        b"IN;SP1;PE>\x7e\x7e\x7e\x7e\x7e\x7e\x7e\x7e\x7e\x7e\x7e\x7e\xfe\x40\xc0;".to_vec(),
        // Arcs and wedges of every degenerate kind, with huge sweeps.
        b"IN;SP1;PD;AA0,0,32767;AR0,0,90;AT0,0,0,0;RT1,1,2,2;CI0;CI-5;WG0,0,0;EW5,99999,-99999;WG-5,0,360;CI99999999999999;".to_vec(),
        // Scaling onto one point, rotations the format doesn't define, huge palettes and pens.
        b"IN;IP5,5,5,5;SC0,1,0,1,1;SP1;PD1,1;SC0,0,0,0,2;RO45;NP99999999;SP99999999999;PC99999999,1,2,3;PW99999,99999;PD2,2;".to_vec(),
        // Polygon mode left open, PM1 and PM2 without PM0, FP and EP with nothing buffered.
        b"IN;SP1;FP;EP;PM1;PM2;PM0;PD10,10;PM1;PM1;PM1;CI5;PE<\xc0\xc0;".to_vec(),
        // Escapes cut short, and data lengths past the end.
        b"\x1b%-12345X@PJL".to_vec(),
        b"IN;SP1;PD0,0;\x1b*b99999999999W".to_vec(),
        b"IN;\x1b".to_vec(),
        b"IN;\x1b.".to_vec(),
    ];
    // A job line with no end.
    cases.push([&b"\x1b%-12345X@PJL "[..], &b"x".repeat(1 << 20)].concat());
    // Bytes of every value after a valid start.
    cases.push([&b"IN;SP1;PD"[..], &(0..=255u8).collect::<Vec<_>>()].concat());
    for f in &cases {
        let r = std::panic::catch_unwind(|| import(f));
        assert!(r.is_ok(), "{:?}", String::from_utf8_lossy(f));
    }
    let far = import(&cases[0]).unwrap();
    assert!(far.warnings.iter().any(|w| w == FAR_AWAY), "{:?}", far.warnings);
    assert_eq!(leaves(&far.document).len(), 1, "the line in range is kept");

    // Past the budget for path points, the rest is left out with a warning; pen-up moves, which
    // make no path, don't count.
    let many = format!("IN;SP1;PR;PD{}", "1,0,".repeat(300_000));
    let i = plot(&many);
    assert!(i.warnings.iter().any(|w| w == TOO_MANY_POINTS), "{:?}", i.warnings);
    // Scaling points past any range give relative pen widths no wider than the widest pen.
    let i = plot("IN;IP-99999999999999999999,0,99999999999999999999,0;WU1;PW32767;SP1;PD100,0;");
    let r = i.document.artboards[0].rect;
    assert!(r.width().is_finite() && r.height() <= 16384.0 * 72.0 / 1016.0 + 1e-6, "{r:?}");
    assert!((stroke(&leaves(&i.document)[0]).width - 16384.0 * 72.0 / 1016.0).abs() < 1e-9);

    // Each fill of the polygon buffer is a copy of it, and counts.
    let big = format!("IN;SP1;PM0;PR;PD{}PM2;{}", "1,1,".repeat(1000), "FP;EP;".repeat(1000));
    let i = plot(&big);
    assert!(i.warnings.iter().any(|w| w == TOO_MANY_POINTS) && leaves(&i.document).len() < 400, "{:?}", i.warnings);
    for pen in ["SP1;PU", "SP0;PD"] {
        let moves = format!("IN;{pen}{};SP1;PU0,0;PD1016,0;", "1,0,".repeat(300_000));
        let i = plot(&moves);
        assert!(i.warnings.is_empty() && leaves(&i.document).len() == 1, "{pen}: {:?}", i.warnings);
    }
}
