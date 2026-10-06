//! DXF import: a fixture per entity, bulge and spline accuracy, blocks as symbols, layers,
//! colours, lineweights, layouts, the options, damaged and binary files, and drawings the export
//! wrote read back.

use std::sync::Arc;

use kurbo::ParamCurve;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Justify, Node, NodeKind, TextObject, Unit};
use vectorcraft_geom::{Affine, FillRule, Point, Rect, shapes};

use super::*;

/// A DXF file from `(code, value)` pairs written one per line.
fn file(pairs: &[(i32, &str)]) -> Vec<u8> {
    let mut s = String::new();
    for (c, v) in pairs {
        s.push_str(&format!("{c:>3}\n{v}\n"));
    }
    s.push_str("  0\nEOF\n");
    s.into_bytes()
}

/// A drawing with `header` variables, `tables`, `blocks` and `entities` (each a run of pairs).
fn drawing(header: &[(i32, &str)], tables: &[(i32, &str)], blocks: &[(i32, &str)], entities: &[(i32, &str)]) -> Vec<u8> {
    let mut p: Vec<(i32, &str)> = vec![(0, "SECTION"), (2, "HEADER")];
    p.extend_from_slice(header);
    p.extend([(0, "ENDSEC"), (0, "SECTION"), (2, "TABLES")]);
    p.extend_from_slice(tables);
    p.extend([(0, "ENDSEC"), (0, "SECTION"), (2, "BLOCKS")]);
    p.extend_from_slice(blocks);
    p.extend([(0, "ENDSEC"), (0, "SECTION"), (2, "ENTITIES")]);
    p.extend_from_slice(entities);
    p.push((0, "ENDSEC"));
    file(&p)
}

fn entities(e: &[(i32, &str)]) -> Vec<u8> {
    drawing(&[], &[], &[], e)
}

/// Points at 1:1, not centred: drawing (x, y) lands at document (x, artboard height − y).
fn points() -> ImportOptions {
    ImportOptions { unit: Some(Unit::Points), scale: Some(1.0), center: false, ..ImportOptions::default() }
}

fn open(bytes: &[u8], o: &ImportOptions) -> Imported {
    import(bytes, o).unwrap()
}

/// Every leaf object, in paint order.
fn leaves(doc: &Document) -> Vec<Node> {
    let mut out = vec![];
    doc.walk(|n| {
        if n.children().is_none() {
            out.push(n.clone());
        }
    });
    out
}

/// The artboard's height (document y = height − drawing y).
fn height(doc: &Document) -> f64 {
    doc.artboards[0].rect.height()
}

fn path_points(n: &Node) -> Vec<Point> {
    let NodeKind::Path { path, .. } = &n.kind else { panic!("not a path: {:?}", n.kind) };
    path.to_bezpath().segments().flat_map(|s| (0..=16).map(move |i| s.eval(f64::from(i) / 16.0))).collect()
}

fn bounds(n: &Node) -> Rect {
    n.geometric_bounds().unwrap()
}

fn close(a: Rect, b: Rect, tol: f64) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() <= tol)
}

fn rgba(p: Option<&Paint>) -> [u8; 4] {
    match p {
        Some(Paint::Solid { color, .. }) => color.to_rgba8(1.0),
        other => panic!("not a solid paint: {other:?}"),
    }
}

fn stroke_rgb(n: &Node) -> [u8; 4] {
    rgba(n.appearance.stroke().map(|s| &s.paint))
}

fn fill_rgb(n: &Node) -> [u8; 4] {
    rgba(n.appearance.fill().map(|f| &f.paint))
}

fn text_of(n: &Node) -> &TextObject {
    let NodeKind::Text(t) = &n.kind else { panic!("not text: {:?}", n.kind) };
    t
}

const LINE: &[(i32, &str)] = &[(0, "LINE"), (8, "0"), (10, "10"), (20, "20"), (11, "110"), (21, "70")];

#[test]
fn a_line_comes_in_at_the_drawings_unit_and_origin() {
    let doc = open(&entities(LINE), &points()).document;
    let [line] = &leaves(&doc)[..] else { panic!() };
    let board = doc.artboards[0].rect;
    assert_eq!((board.width(), board.height()), (100.0, 50.0), "the artboard is the art's size");
    // Not centred: the origin sits on the artboard's bottom-left corner.
    let pts = path_points(line);
    assert!(pts.first().unwrap().distance(Point::new(10.0, 50.0 - 20.0)) < 1e-9, "{pts:?}");
    assert!(pts.last().unwrap().distance(Point::new(110.0, 50.0 - 70.0)) < 1e-9);
    assert_eq!(stroke_rgb(line), [0, 0, 0, 255], "colour 7 is black on paper");
    let w = line.appearance.stroke().unwrap().width;
    assert!((w - 0.25 * 72.0 / 25.4).abs() < 1e-9, "the default lineweight is 0.25 mm: {w}");
    assert!(line.appearance.fill().is_none_or(|f| f.paint.is_none()));
}

#[test]
fn units_choose_the_default_scale() {
    // Millimetres at 1:1: 100 units are 100 mm.
    let mm = drawing(&[(9, "$INSUNITS"), (70, "4")], &[], &[], LINE);
    let doc = open(&mm, &ImportOptions::default()).document;
    assert!((doc.artboards[0].rect.width() - 100.0 * 72.0 / 25.4).abs() < 1e-6);
    assert_eq!(doc.units, Unit::Millimeters);
    // Inches; microns (1 mm = 1000 units); unitless imperial drawings read inches.
    let inch = open(&drawing(&[(9, "$INSUNITS"), (70, "1")], &[], &[], LINE), &ImportOptions::default()).document;
    assert!((inch.artboards[0].rect.width() - 7200.0).abs() < 1e-6);
    let um = info(&drawing(&[(9, "$INSUNITS"), (70, "13")], &[], &[], LINE)).unwrap();
    assert_eq!((um.unit, um.units.as_str()), (Unit::Millimeters, "Microns"));
    assert!((um.scale - 1000.0).abs() < 1e-6);
    let imperial = info(&drawing(&[(9, "$MEASUREMENT"), (70, "0")], &[], &[], LINE)).unwrap();
    assert_eq!((imperial.unit, imperial.scale), (Unit::Inches, 1.0));
    // A ratio: 1 cm = 10 units is 1:1 in millimetres again.
    let o = ImportOptions { unit: Some(Unit::Centimeters), scale: Some(10.0), ..ImportOptions::default() };
    let cm = open(&mm, &o).document;
    assert!((cm.artboards[0].rect.width() - 100.0 * 72.0 / 25.4).abs() < 1e-6);
    assert_eq!(cm.units, Unit::Centimeters);
}

#[test]
fn circles_arcs_and_ellipses() {
    let e = [
        (0, "CIRCLE"),
        (10, "50"),
        (20, "50"),
        (40, "25"),
        (0, "ARC"),
        (10, "200"),
        (20, "50"),
        (40, "20"),
        (50, "0"),
        (51, "90"),
        (0, "ELLIPSE"),
        (10, "400"),
        (20, "50"),
        (11, "40"),
        (21, "0"),
        (40, "0.5"),
        (41, "0"),
        (42, "6.283185307179586"),
    ];
    let doc = open(&entities(&e), &points()).document;
    let h = height(&doc);
    let flip = |r: Rect| Rect::new(r.x0, h - r.y1, r.x1, h - r.y0);
    let [circle, arc, ellipse] = &leaves(&doc)[..] else { panic!() };
    assert!(close(bounds(circle), flip(Rect::new(25.0, 25.0, 75.0, 75.0)), 1e-3), "{:?}", bounds(circle));
    for p in path_points(circle) {
        assert!((p.distance(Point::new(50.0, h - 50.0)) - 25.0).abs() < 1e-4);
    }
    // A quarter arc counter-clockwise from 0° to 90°.
    assert!(close(bounds(arc), flip(Rect::new(200.0, 50.0, 220.0, 70.0)), 1e-3), "{:?}", bounds(arc));
    assert!(close(bounds(ellipse), flip(Rect::new(360.0, 30.0, 440.0, 70.0)), 1e-3), "{:?}", bounds(ellipse));
    let NodeKind::Path { path, .. } = &circle.kind else { panic!() };
    assert!(path.is_closed());
}

#[test]
fn polyline_bulges_are_exact_arcs() {
    // A slot: two straight sides and two half circles (bulge 1) of radius 10.
    let e = [
        (0, "LWPOLYLINE"),
        (90, "4"),
        (70, "1"),
        (10, "0"),
        (20, "0"),
        (10, "100"),
        (20, "0"),
        (42, "1"),
        (10, "100"),
        (20, "20"),
        (10, "0"),
        (20, "20"),
        (42, "1"),
    ];
    let doc = open(&entities(&e), &points()).document;
    let [slot] = &leaves(&doc)[..] else { panic!() };
    let h = height(&doc);
    // The arcs bulge outward, so the art is 120 wide.
    assert!(close(bounds(slot), Rect::new(-10.0, h - 20.0, 110.0, h), 1e-4), "{:?}", bounds(slot));
    for p in path_points(slot).into_iter().filter(|p| p.x > 100.0) {
        let err = (p.distance(Point::new(100.0, h - 10.0)) - 10.0).abs();
        assert!(err < 1e-4, "bulge error {err}");
    }
    // Old-style polylines read their vertices and bulges too.
    let e = [
        (0, "POLYLINE"),
        (66, "1"),
        (70, "0"),
        (0, "VERTEX"),
        (10, "0"),
        (20, "0"),
        (42, "-1"),
        (0, "VERTEX"),
        (10, "20"),
        (20, "0"),
        (0, "SEQEND"),
        (0, "LINE"),
        (10, "0"),
        (20, "0"),
        (11, "0"),
        (21, "-30"),
    ];
    let doc = open(&entities(&e), &points()).document;
    let [arc, _] = &leaves(&doc)[..] else { panic!() };
    // Clockwise from (0, 0) to (20, 0): over the chord, 10 high.
    let b = bounds(arc);
    assert!((b.width() - 20.0).abs() < 1e-4 && (b.height() - 10.0).abs() < 1e-4, "{b:?}");
    assert!((b.y1 - height(&doc)).abs() < 1e-4, "the arc's chord is at y 0: {b:?}");
}

#[test]
fn nurbs_splines_stay_within_a_hundredth() {
    // A rational quadratic circle of radius 100: nine weighted control points.
    let w = std::f64::consts::FRAC_1_SQRT_2.to_string();
    let mut e: Vec<(i32, String)> = vec![(0, "SPLINE".into()), (70, "12".into()), (71, "2".into()), (72, "12".into()), (73, "9".into())];
    for k in ["0", "0", "0", "1", "1", "2", "2", "3", "3", "4", "4", "4"] {
        e.push((40, k.into()));
    }
    for i in 0..9 {
        e.push((41, if i % 2 == 0 { "1".into() } else { w.clone() }));
    }
    for (x, y) in [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1), (1, 0)] {
        e.extend([(10, (200 + x * 100).to_string()), (20, (200 + y * 100).to_string()), (30, "0".into())]);
    }
    let e: Vec<(i32, &str)> = e.iter().map(|(c, v)| (*c, v.as_str())).collect();
    let doc = open(&entities(&e), &points()).document;
    let [circle] = &leaves(&doc)[..] else { panic!() };
    let h = height(&doc);
    let worst = path_points(circle).iter().map(|p| (p.distance(Point::new(200.0, h - 200.0)) - 100.0).abs()).fold(0.0, f64::max);
    assert!(worst < 0.01, "NURBS error {worst}");
    // A spline given by fit points alone passes through them.
    let e = [(0, "SPLINE"), (71, "3"), (74, "3"), (11, "0"), (21, "0"), (11, "50"), (21, "40"), (11, "100"), (21, "0")];
    let doc = open(&entities(&e), &points()).document;
    let [s] = &leaves(&doc)[..] else { panic!() };
    let h = height(&doc);
    assert!(path_points(s).iter().any(|p| p.distance(Point::new(50.0, h - 40.0)) < 1e-9));
}

/// A square with a round hole: a polyline loop and a loop of one edge (a whole arc).
fn hatch(solid: &'static str) -> Vec<(i32, &'static str)> {
    vec![
        (0, "HATCH"),
        (62, "1"),
        (10, "0"),
        (20, "0"),
        (30, "0"),
        (2, if solid == "1" { "SOLID" } else { "ANSI31" }),
        (70, solid),
        (71, "0"),
        (91, "2"),
        (92, "3"),
        (72, "0"),
        (73, "1"),
        (93, "4"),
        (10, "0"),
        (20, "0"),
        (10, "100"),
        (20, "0"),
        (10, "100"),
        (20, "100"),
        (10, "0"),
        (20, "100"),
        (97, "0"),
        (92, "0"),
        (93, "1"),
        (72, "2"),
        (10, "50"),
        (20, "50"),
        (40, "20"),
        (50, "0"),
        (51, "360"),
        (73, "1"),
        (97, "0"),
        (75, "0"),
        (76, "1"),
        (98, "0"),
    ]
}

#[test]
fn solid_hatches_fill_and_pattern_hatches_outline() {
    let r = open(&entities(&hatch("1")), &points());
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let compound = r.document.layers[0].children().unwrap()[0].clone();
    let NodeKind::Compound { children, rule } = &compound.kind else { panic!("{:?}", compound.kind) };
    assert_eq!((children.len(), *rule), (2, FillRule::EvenOdd), "the hole is a second loop");
    assert_eq!(fill_rgb(&compound), [255, 0, 0, 255]);
    assert!(compound.appearance.stroke().is_none_or(|s| s.paint.is_none()));
    assert_eq!(bounds(&compound).area(), 10_000.0);
    let r = open(&entities(&hatch("0")), &points());
    assert!(r.warnings.iter().any(|w| w.contains("pattern hatches")), "{:?}", r.warnings);
    let outline = r.document.layers[0].children().unwrap()[0].clone();
    assert_eq!(stroke_rgb(&outline), [255, 0, 0, 255]);
    // Solids fill their corners in zigzag order.
    let e = [(0, "SOLID"), (10, "0"), (20, "0"), (11, "10"), (21, "0"), (12, "0"), (22, "10"), (13, "10"), (23, "10")];
    let doc = open(&entities(&e), &points()).document;
    let [square] = &leaves(&doc)[..] else { panic!() };
    assert_eq!(bounds(square).area(), 100.0);
    assert!(square.appearance.fill().is_some_and(|f| !f.paint.is_none()));
}

/// A solid hatch: a loop of a line and a spline edge (`fit` its fit data, if any), followed by
/// `source` (the loop's source-object count and handles), and a square hole.
fn spline_edged_hatch(fit: &[(i32, &'static str)], source: &[(i32, &'static str)]) -> Vec<(i32, &'static str)> {
    let mut e = vec![(0, "HATCH"), (62, "1"), (10, "0"), (20, "0"), (30, "0"), (2, "SOLID"), (70, "1"), (71, "1"), (91, "2")];
    e.extend([(92, "1"), (93, "2"), (72, "1"), (10, "0"), (20, "0"), (11, "100"), (21, "0")]);
    e.extend([(72, "4"), (94, "3"), (73, "0"), (74, "0"), (95, "8"), (96, "4")]);
    e.extend(["0", "0", "0", "0", "1", "1", "1", "1"].map(|k| (40, k)));
    e.extend([(10, "100"), (20, "0"), (10, "100"), (20, "100"), (10, "0"), (20, "100"), (10, "0"), (20, "0")]);
    e.extend_from_slice(fit);
    e.extend_from_slice(source);
    e.extend([(92, "2"), (72, "0"), (73, "1"), (93, "4"), (10, "40"), (20, "20"), (10, "60"), (20, "20")]);
    e.extend([(10, "60"), (20, "40"), (10, "40"), (20, "40"), (97, "0"), (75, "0"), (76, "1"), (98, "0")]);
    e
}

#[test]
fn hatch_spline_edges_with_or_without_fit_data() {
    // Before DXF 2010 a spline edge has no fit data, so a 97 right after it is the loop's own
    // count of source objects; from 2010 on it has a fit count, here 0, before that.
    let source = [(97, "1"), (330, "1F")];
    for fit in [&[][..], &[(97, "0")][..]] {
        let r = open(&entities(&spline_edged_hatch(fit, &source)), &points());
        let [hatch] = &r.document.layers[0].children().unwrap()[..] else { panic!("fit data {fit:?}: the hatch is lost") };
        let NodeKind::Compound { children, .. } = &hatch.kind else { panic!("{:?}", hatch.kind) };
        assert_eq!(children.len(), 2, "the outline and the hole, fit data {fit:?}");
        // The spline edge peaks three quarters of the way up its control polygon.
        let b = bounds(hatch);
        assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 75.0).abs() < 1e-6, "{b:?}");
    }
    // Fit points after the count are still read (and ignored when there are control points).
    let fit = [(97, "2"), (11, "100"), (21, "0"), (11, "0"), (21, "0")];
    let r = open(&entities(&spline_edged_hatch(&fit, &source)), &points());
    assert_eq!(r.document.layers[0].children().unwrap().len(), 1);
}

#[test]
fn text_and_multiline_text() {
    let tables = [(0, "TABLE"), (2, "STYLE"), (0, "STYLE"), (2, "Notes"), (3, "DejaVuSans.ttf"), (0, "ENDTAB")];
    let e = [
        (0, "TEXT"),
        (10, "10"),
        (20, "10"),
        (40, "7"),
        (1, "Room %%c20"),
        (50, "90"),
        (7, "Notes"),
        (0, "MTEXT"),
        (10, "100"),
        (20, "100"),
        (40, "3.5"),
        (71, "2"),
        (3, "{\\fArial|b0;First}"),
        (1, "\\PSecond"),
    ];
    let doc = open(&drawing(&[], &tables, &[], &e), &points()).document;
    let [single, multi] = &leaves(&doc)[..] else { panic!() };
    let h = height(&doc);
    let t = text_of(single);
    assert_eq!(t.plain_text(), "Room ⌀20");
    let st = t.first_style();
    assert!((st.size - 10.0).abs() < 1e-9, "a 7-unit cap height is 10 pt type: {}", st.size);
    assert_eq!(st.font_family, "DejaVuSans");
    // Turned a quarter counter-clockwise, its baseline starting at the insertion point.
    let origin = t.xf * Point::ZERO;
    assert!(origin.distance(Point::new(10.0, h - 10.0)) < 1e-9, "{origin:?}");
    let along = t.xf * Point::new(1.0, 0.0) - origin;
    assert!(along.x.abs() < 1e-9 && (along.y + 1.0).abs() < 1e-9, "{along:?}");
    let m = text_of(multi);
    assert_eq!(m.plain_text(), "First\nSecond");
    assert_eq!(m.para.justify, Justify::Center, "attached top centre");
    assert_eq!(m.first_style().font_family, "Arial");
    // Top attachment: the first baseline one cap height below the insertion point.
    let o = m.xf * Point::ZERO;
    assert!(o.distance(Point::new(100.0, h - 100.0 + 3.5)) < 1e-9, "{o:?}");
    let leading = m.first_style().leading.unwrap();
    assert!((leading - 3.5 * 5.0 / 3.0).abs() < 1e-9, "{leading}");
}

#[test]
fn fit_and_aligned_text_reach_their_second_point() {
    let spanning = |text: &'static str, y: &'static str, justify: &'static str| {
        [(0, "TEXT"), (10, "10"), (20, y), (40, "5"), (1, text), (72, justify), (11, "90"), (21, y)]
    };
    let mut e = spanning("FIT BETWEEN", "40", "5").to_vec();
    e.extend(spanning("ALIGNED", "10", "3"));
    e.extend([(0, "TEXT"), (10, "10"), (20, "70"), (40, "5"), (1, "PLAIN")]);
    let doc = open(&entities(&e), &points()).document;
    let [fit, aligned, plain] = &leaves(&doc)[..] else { panic!() };
    let width = |n: &Node| {
        let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), text_of(n));
        l.lines[0].x1 - l.lines[0].x0
    };
    // Both run from x = 10 to x = 90.
    for n in [fit, aligned] {
        assert!((width(n) - 80.0).abs() < 1e-6, "{}", width(n));
    }
    let (f, a, p) = (text_of(fit).first_style(), text_of(aligned).first_style(), text_of(plain).first_style());
    // Fit text keeps its height and is stretched; Aligned text keeps its proportions and grows.
    assert_eq!(f.size, p.size);
    assert!(f.h_scale > 100.0, "{}", f.h_scale);
    assert_eq!(a.h_scale, 100.0);
    assert!(a.size > p.size, "{} vs {}", a.size, p.size);
}

/// A block "Door" (base point 5, 5) holding `block_art`, and `inserts`.
fn with_door(block_art: &[(i32, &'static str)], inserts: &[(i32, &'static str)]) -> Vec<u8> {
    let mut blocks = vec![(0, "BLOCK"), (8, "0"), (2, "Door"), (70, "0"), (10, "5"), (20, "5"), (3, "Door")];
    blocks.extend_from_slice(block_art);
    blocks.push((0, "ENDBLK"));
    let tables = [(0, "TABLE"), (2, "LAYER"), (0, "LAYER"), (2, "Doors"), (70, "0"), (62, "5"), (6, "CONTINUOUS"), (0, "ENDTAB")];
    drawing(&[], &tables, &blocks, inserts)
}

/// A red line 10 units along x from the base point, on layer 0.
const DOOR_LINE: &[(i32, &str)] = &[(0, "LINE"), (8, "0"), (62, "1"), (10, "5"), (20, "5"), (11, "15"), (21, "5")];

#[test]
fn inserts_become_symbol_instances_with_their_transform() {
    let inserts = [(0, "INSERT"), (8, "Doors"), (2, "Door"), (10, "100"), (20, "50"), (41, "2"), (42, "3"), (50, "90")];
    let doc = open(&with_door(DOOR_LINE, &inserts), &points()).document;
    assert_eq!(doc.symbols.len(), 1);
    assert_eq!(doc.symbols[0].name, "Door");
    let [inst] = &leaves(&doc)[..] else { panic!() };
    let NodeKind::SymbolInstance { symbol, xf } = &inst.kind else { panic!("{:?}", inst.kind) };
    assert_eq!(symbol, "Door");
    assert_eq!(doc.layers[0].name.as_deref(), Some("Doors"));
    // Scaled 2 and turned a quarter: from the insertion point 20 units up.
    let art = &doc.symbols[0].art;
    let pts: Vec<Point> = path_points(art).iter().map(|p| *xf * *p).collect();
    let h = height(&doc);
    let (a, b) = (pts.first().unwrap(), pts.last().unwrap());
    assert!(a.distance(Point::new(100.0, h - 50.0)) < 1e-9, "{a:?}");
    assert!(b.distance(Point::new(100.0, h - 70.0)) < 1e-9, "{b:?}");
    assert_eq!(stroke_rgb(art), [255, 0, 0, 255], "the block's own colour");
}

#[test]
fn by_block_colours_make_a_symbol_per_look() {
    // The block's line takes the insert's colour: red and green make two symbols; a third red
    // insert reuses the first.
    let by_block: &[(i32, &str)] = &[(0, "LINE"), (8, "0"), (62, "0"), (10, "5"), (20, "5"), (11, "15"), (21, "5")];
    let inserts = [
        (0, "INSERT"),
        (2, "Door"),
        (62, "1"),
        (10, "0"),
        (20, "0"),
        (0, "INSERT"),
        (2, "Door"),
        (62, "3"),
        (10, "50"),
        (20, "0"),
        (0, "INSERT"),
        (2, "Door"),
        (62, "1"),
        (10, "100"),
        (20, "0"),
    ];
    let doc = open(&with_door(by_block, &inserts), &points()).document;
    let names: Vec<&str> = doc.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Door", "Door 2"]);
    assert_eq!(stroke_rgb(&doc.symbols[0].art), [255, 0, 0, 255]);
    assert_eq!(stroke_rgb(&doc.symbols[1].art), [0, 255, 0, 255]);
    assert_eq!(leaves(&doc).len(), 3);
    // Layer 0 art takes the insert's layer's colour.
    let on_zero: &[(i32, &str)] = &[(0, "LINE"), (8, "0"), (10, "5"), (20, "5"), (11, "15"), (21, "5")];
    let inserts = [(0, "INSERT"), (8, "Doors"), (2, "Door"), (10, "0"), (20, "0"), (0, "INSERT"), (8, "Doors"), (2, "Door"), (10, "40"), (20, "0")];
    let doc = open(&with_door(on_zero, &inserts), &points()).document;
    assert_eq!(doc.symbols.len(), 1);
    assert_eq!(stroke_rgb(&doc.symbols[0].art), [0, 0, 255, 255], "layer Doors is blue");
    // Art with its own colour shares one symbol, whatever the inserts' colours.
    let inserts = [(0, "INSERT"), (2, "Door"), (62, "1"), (10, "0"), (20, "0"), (0, "INSERT"), (2, "Door"), (62, "3"), (10, "40"), (20, "0")];
    let doc = open(&with_door(&[(0, "LINE"), (8, "Doors"), (62, "4"), (10, "5"), (20, "5"), (11, "15"), (21, "5")], &inserts), &points()).document;
    assert_eq!(doc.symbols.len(), 1);
}

#[test]
fn array_inserts_attributes_and_dimensions() {
    let inserts = [
        (0, "INSERT"),
        (66, "1"),
        (2, "Door"),
        (10, "0"),
        (20, "0"),
        (70, "3"),
        (71, "2"),
        (44, "20"),
        (45, "30"),
        (0, "ATTRIB"),
        (10, "0"),
        (20, "-10"),
        (40, "2"),
        (1, "D-01"),
        (2, "TAG"),
        (70, "0"),
        (0, "ATTRIB"),
        (10, "0"),
        (20, "-20"),
        (40, "2"),
        (1, "hidden"),
        (2, "NOTE"),
        (70, "1"),
        (0, "SEQEND"),
    ];
    let doc = open(&with_door(DOOR_LINE, &inserts), &points()).document;
    let all = leaves(&doc);
    let instances: Vec<Point> =
        all.iter().filter_map(|n| if let NodeKind::SymbolInstance { xf, .. } = &n.kind { Some(*xf * Point::ZERO) } else { None }).collect();
    assert_eq!(instances.len(), 6, "3 columns × 2 rows");
    let xs: std::collections::BTreeSet<i64> = instances.iter().map(|p| p.x.round() as i64).collect();
    assert_eq!(xs.len(), 3, "columns 20 apart: {instances:?}");
    let texts: Vec<String> = all.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None }).collect();
    assert_eq!(texts, ["D-01"], "invisible attributes stay out");
    // A dimension draws its anonymous block as art, not a symbol.
    let blocks = [(0, "BLOCK"), (2, "*D1"), (70, "1"), (10, "0"), (20, "0"), (0, "LINE"), (10, "0"), (20, "0"), (11, "50"), (21, "0"), (0, "ENDBLK")];
    let dims = [(0, "DIMENSION"), (2, "*D1"), (10, "0"), (20, "0")];
    let doc = open(&drawing(&[], &[], &blocks, &dims), &points()).document;
    assert!(doc.symbols.is_empty());
    assert_eq!(bounds(&leaves(&doc)[0]).width(), 50.0);
}

#[test]
fn blocks_inside_themselves_are_left_out_without_hanging() {
    let blocks = [
        (0, "BLOCK"),
        (2, "Loop"),
        (10, "0"),
        (20, "0"),
        (0, "LINE"),
        (10, "0"),
        (20, "0"),
        (11, "10"),
        (21, "0"),
        (0, "INSERT"),
        (2, "Loop"),
        (10, "20"),
        (20, "0"),
        (0, "ENDBLK"),
    ];
    let r = open(&drawing(&[], &[], &blocks, &[(0, "INSERT"), (2, "Loop"), (10, "0"), (20, "0")]), &points());
    assert!(r.warnings.iter().any(|w| w.contains("inside themselves")), "{:?}", r.warnings);
    assert_eq!(r.document.symbols.len(), 1);
}

#[test]
fn layers_keep_their_state_colour_lineweight_and_linetype() {
    let tables = [
        (0, "TABLE"),
        (2, "LTYPE"),
        (0, "LTYPE"),
        (2, "DASHED"),
        (73, "2"),
        (40, "15"),
        (49, "10"),
        (49, "-5"),
        (0, "ENDTAB"),
        (0, "TABLE"),
        (2, "LAYER"),
        (0, "LAYER"),
        (2, "Hidden"),
        (70, "0"),
        (62, "-3"),
        (6, "DASHED"),
        (370, "50"),
        (0, "LAYER"),
        (2, "Frozen"),
        (70, "1"),
        (62, "2"),
        (0, "LAYER"),
        (2, "Locked"),
        (70, "4"),
        (62, "1"),
        (420, "16744448"),
        (290, "0"),
        (0, "ENDTAB"),
    ];
    let line = |layer: &'static str| [(0, "LINE"), (8, layer), (10, "0"), (20, "0"), (11, "100"), (21, "0")];
    let e: Vec<(i32, &str)> = [line("Hidden"), line("Frozen"), line("Locked"), line("Unlisted")].concat();
    let doc = open(&drawing(&[], &tables, &[], &e), &points()).document;
    let names: Vec<&str> = doc.layers.iter().filter_map(|l| l.name.as_deref()).collect();
    assert_eq!(names, ["Hidden", "Frozen", "Locked", "Unlisted"]);
    let [hidden, frozen, locked, plain] = &doc.layers[..] else { panic!() };
    assert!(!hidden.visible && !frozen.visible && locked.visible && plain.visible);
    assert!(locked.locked && !hidden.locked);
    assert!(matches!(locked.kind, NodeKind::Layer { printable: false, .. }));
    let first = |l: &Arc<Node>| l.children().unwrap()[0].clone();
    // Colour 3 (green) and a 0.5 mm dashed line from the layer.
    let h = first(hidden);
    assert_eq!(stroke_rgb(&h), [0, 255, 0, 255]);
    let st = h.appearance.stroke().unwrap();
    assert!((st.width - 0.5 * 72.0 / 25.4).abs() < 1e-9);
    assert_eq!(st.dash.as_ref().unwrap().pattern, [10.0, 5.0]);
    assert_eq!(stroke_rgb(&first(frozen)), [255, 255, 0, 255]);
    assert_eq!(stroke_rgb(&first(locked)), [255, 128, 0, 255], "true colour wins over the index");
    // Merge Layers: one layer.
    let merged = open(&drawing(&[], &tables, &[], &e), &ImportOptions { merge_layers: true, ..points() }).document;
    assert_eq!(merged.layers.len(), 1);
    assert_eq!(merged.layers[0].children().unwrap().len(), 4);
}

#[test]
fn entity_colours_lineweights_and_transparency() {
    let e = [
        (0, "LINE"),
        (62, "30"),
        (370, "100"),
        (440, "33554559"),
        (10, "0"),
        (20, "0"),
        (11, "10"),
        (21, "0"),
        (0, "LINE"),
        (62, "5"),
        (420, "65280"),
        (370, "0"),
        (10, "0"),
        (20, "10"),
        (11, "10"),
        (21, "10"),
    ];
    let doc = open(&entities(&e), &points()).document;
    let [a, b] = &leaves(&doc)[..] else { panic!() };
    assert_eq!(stroke_rgb(a), [255, 127, 0, 255], "index 30 is orange");
    assert!((a.appearance.stroke().unwrap().width - 72.0 / 25.4).abs() < 1e-9, "1 mm");
    assert!((a.opacity - 127.0 / 255.0).abs() < 1e-6);
    assert_eq!(stroke_rgb(b), [0, 255, 0, 255]);
    assert_eq!(b.appearance.stroke().unwrap().width, 0.25, "lineweight 0 is the thinnest line");
    // Scale Lineweights: at 1 pt = 10 units (a tenth of 1:1 in millimetres) they shrink too.
    let o = ImportOptions { unit: Some(Unit::Points), scale: Some(10.0), scale_lineweights: true, ..ImportOptions::default() };
    let doc = open(&drawing(&[(9, "$INSUNITS"), (70, "4")], &[], &[], &e), &o).document;
    let w = leaves(&doc)[0].appearance.stroke().unwrap().width;
    let k = 0.1 / (72.0 / 25.4);
    assert!((w - 72.0 / 25.4 * k).abs() < 1e-9, "{w}");
}

#[test]
fn fit_and_centre_place_the_art_on_the_artboard() {
    let doc = open(&entities(LINE), &ImportOptions { fit: true, ..ImportOptions::default() }).document;
    let board = doc.artboards[0].rect;
    assert_eq!((board.width(), board.height()), (792.0, 612.0), "landscape letter for wide art");
    let b = bounds(&leaves(&doc)[0]);
    assert!((b.width() - 792.0).abs() < 1e-6 && (b.center().y - 306.0).abs() < 1e-6, "{b:?}");
    let o = ImportOptions { fit: true, fit_to: (200.0, 100.0), center: false, ..ImportOptions::default() };
    let b = bounds(&leaves(&open(&entities(LINE), &o).document)[0]);
    assert!(close(b, Rect::new(0.0, 0.0, 200.0, 100.0), 1e-6), "bottom-left: {b:?}");
    // Centred at a ratio, the artboard is the art's bounds.
    let doc = open(&entities(LINE), &ImportOptions { unit: Some(Unit::Points), scale: Some(1.0), ..ImportOptions::default() }).document;
    assert!(close(bounds(&leaves(&doc)[0]), doc.artboards[0].rect, 1e-9));
    // Art too large for the canvas at the scale asked for says so.
    let o = ImportOptions { unit: Some(Unit::Meters), scale: Some(0.001), ..ImportOptions::default() };
    let e = import(&entities(LINE), &o).unwrap_err();
    assert!(e.contains("canvas"), "{e}");
    let e = import(&entities(LINE), &ImportOptions { scale: Some(-1.0), ..ImportOptions::default() }).unwrap_err();
    assert!(e.contains("scale"), "{e}");
}

#[test]
fn paper_layouts() {
    let mut e = LINE.to_vec();
    e.extend([(0, "CIRCLE"), (67, "1"), (10, "0"), (20, "0"), (40, "5")]);
    let bytes = entities(&e);
    assert_eq!(info(&bytes).unwrap().layouts, ["Model", "Layout1"]);
    let model = leaves(&open(&bytes, &points()).document);
    assert_eq!(model.len(), 1);
    assert_eq!(bounds(&model[0]).width(), 100.0, "model space holds the line");
    let paper = leaves(&open(&bytes, &ImportOptions { layout: Some("layout1".into()), ..points() }).document);
    assert_eq!(bounds(&paper[0]).width(), 10.0, "the paper layout holds the circle");
    let e = import(&bytes, &ImportOptions { layout: Some("Sheet".into()), ..points() }).unwrap_err();
    assert!(e.contains("Model, Layout1"), "{e}");
}

#[test]
fn binary_dwg_damaged_and_foreign_files() {
    let mut binary = BINARY_SENTINEL.to_vec();
    binary.extend([0u8; 40]);
    assert!(is_dxf(&binary));
    let e = import(&binary, &ImportOptions::default()).unwrap_err();
    assert!(e.contains("binary DXF") && e.contains("ASCII"), "{e}");
    let e = import(b"AC1032\x00\x00\x00", &ImportOptions::default()).unwrap_err();
    assert!(e.contains("DWG"), "{e}");
    for junk in [&b""[..], b"hello", b"<svg/>", b"999\nnote\n"] {
        assert!(import(junk, &ImportOptions::default()).is_err(), "{junk:?}");
        assert!(!is_dxf(junk), "{junk:?}");
    }
    assert!(is_dxf(b"999\nwritten by hand\n  0\nSECTION\n  2\nHEADER\n"));
    // A file cut off in the middle keeps what came before.
    let mut cut = entities(LINE);
    cut.truncate(cut.len() - 30);
    cut.extend(b"\nnot a code\n");
    let r = open(&cut, &points());
    assert_eq!(leaves(&r.document).len(), 1);
    assert!(r.warnings.iter().any(|w| w.contains("damaged")), "{:?}", r.warnings);
    // Hostile numbers are dropped; unknown entities are listed.
    let e =
        [(0, "CIRCLE"), (10, "1e999"), (20, "NaN"), (40, "1e300"), (0, "XLINE"), (10, "0"), (0, "LINE"), (10, "0"), (20, "0"), (11, "1"), (21, "1")];
    let r = open(&entities(&e), &points());
    assert!(r.warnings.iter().any(|w| w.contains("1 XLINE")), "{:?}", r.warnings);
}

#[test]
fn drawings_the_export_writes_read_back() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let id = doc.alloc_id();
    let rect = shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 70.0));
    let blue = Appearance::basic(Paint::None, Paint::solid(Color::rgb8(0, 0, 255)), 2.0);
    doc.insert(Some(layer), 0, Node::path(id, rect, blue)).unwrap();
    let id = doc.alloc_id();
    let circle = shapes::ellipse(Rect::new(140.0, 20.0, 180.0, 60.0));
    doc.insert(Some(layer), 1, Node::path(id, circle, Appearance::basic(Paint::solid(Color::rgb8(255, 0, 0)), Paint::None, 0.0))).unwrap();
    for version in [crate::DxfVersion::R12, crate::DxfVersion::R2000, crate::DxfVersion::R2018] {
        let opts = crate::DxfOptions { version, unit: Unit::Points, region: doc.artboards[0].rect, ..crate::DxfOptions::default() };
        let out = crate::export(&doc, &opts).unwrap();
        let back = open(&out.bytes, &points());
        let art = leaves(&back.document);
        // Same place on the artboard (the export's origin is the artboard's bottom-left).
        let shift = Affine::translate((0.0, 100.0 - height(&back.document)));
        let all = art.iter().filter_map(|n| n.geometric_bounds()).map(|b| shift.transform_rect_bbox(b)).reduce(|a, b| a.union(b)).unwrap();
        assert!(close(all, Rect::new(20.0, 20.0, 180.0, 70.0), 0.1), "{version:?}: {all:?}");
        let stroked = art.iter().find(|n| n.appearance.stroke().is_some_and(|s| !s.paint.is_none())).unwrap();
        assert_eq!(stroke_rgb(stroked), [0, 0, 255, 255], "{version:?}");
        if version >= crate::DxfVersion::R2000 {
            assert!(art.iter().any(|n| n.appearance.fill().is_some_and(|f| !f.paint.is_none())), "{version:?}: the fill comes back as a hatch");
            let w = stroked.appearance.stroke().unwrap().width;
            assert!((w - 2.0).abs() < 0.2, "{version:?}: lineweight {w}");
        }
    }
}

#[test]
fn info_lists_layers_layouts_and_version() {
    let tables = [(0, "TABLE"), (2, "LAYER"), (0, "LAYER"), (2, "0"), (62, "7"), (0, "LAYER"), (2, "Walls"), (62, "1"), (0, "ENDTAB")];
    let i = info(&drawing(&[(9, "$ACADVER"), (1, "AC1015"), (9, "$INSUNITS"), (70, "6")], &tables, &[], LINE)).unwrap();
    assert_eq!(i.version, "2000");
    assert_eq!(i.units, "Meters");
    assert_eq!((i.unit, i.scale), (Unit::Meters, 1.0));
    assert_eq!(i.layers, ["0", "Walls"]);
    assert_eq!(i.layouts, ["Model"]);
    assert!(info(b"not a drawing").is_err());
}

#[test]
fn entities_extruded_along_minus_z_are_mirrored() {
    let e = [(0, "CIRCLE"), (10, "50"), (20, "0"), (40, "10"), (210, "0"), (220, "0"), (230, "-1")];
    let doc = open(&entities(&e), &points()).document;
    let b = bounds(&leaves(&doc)[0]);
    assert!((b.center().x + 50.0).abs() < 1e-6, "the centre's x is mirrored: {b:?}");
}
