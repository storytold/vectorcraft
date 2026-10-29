//! DXF output read back as group codes: versions, entities, colours, units, layers, type, images.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Dash, Document, ImageBlob, ImageObject, Node, NodeKind, TextObject, Unit};
use vectorcraft_geom::{Affine, PathData, Point, Rect, shapes};

use crate::*;

type Pairs = Vec<(i32, String)>;

/// The `(code, value)` pairs of a DXF file.
fn pairs(dxf: &[u8]) -> Pairs {
    let text = std::str::from_utf8(dxf).expect("UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len() % 2, 0, "group codes and values come in pairs");
    lines.chunks(2).map(|p| (p[0].trim().parse().expect("a group code"), p[1].to_string())).collect()
}

/// The objects of section `name`: each the pairs from its `0` code on.
fn section(p: &[(i32, String)], name: &str) -> Vec<Pairs> {
    let start = p.windows(2).position(|w| w[0] == (0, "SECTION".into()) && w[1] == (2, name.into()));
    let Some(start) = start else { return vec![] };
    let mut out: Vec<Pairs> = vec![];
    for pair in &p[start + 2..] {
        if *pair == (0, "ENDSEC".into()) {
            break;
        }
        if pair.0 == 0 {
            out.push(vec![]);
        }
        if let Some(o) = out.last_mut() {
            o.push(pair.clone());
        }
    }
    out
}

fn kind(o: &[(i32, String)]) -> &str {
    o.first().map_or("", |p| p.1.as_str())
}

fn get(o: &[(i32, String)], code: i32) -> Option<&str> {
    o.iter().find(|p| p.0 == code).map(|p| p.1.as_str())
}

fn all(o: &[(i32, String)], code: i32) -> Vec<f64> {
    o.iter().filter(|p| p.0 == code).map(|p| p.1.trim().parse().expect("a number")).collect()
}

fn header(p: &[(i32, String)], var: &str) -> Option<String> {
    let i = p.iter().position(|x| *x == (9, var.into()))?;
    p.get(i + 1).map(|x| x.1.clone())
}

fn entities(out: &DxfOutput) -> Vec<Pairs> {
    section(&pairs(&out.bytes), "ENTITIES")
}

fn kinds(out: &DxfOutput) -> Vec<String> {
    entities(out).iter().map(|e| kind(e).to_string()).collect()
}

fn table(out: &DxfOutput, kind_: &str) -> Vec<Pairs> {
    section(&pairs(&out.bytes), "TABLES").into_iter().filter(|o| kind(o) == kind_).collect()
}

/// A 200 × 100 pt document whose first layer holds what `nodes` makes.
fn doc_with(nodes: impl FnOnce(&mut Document) -> Vec<Node>) -> Document {
    let mut d = Document::new(200.0, 100.0);
    let nodes = nodes(&mut d);
    let layer = d.layers[0].id;
    for n in nodes {
        d.insert(Some(layer), usize::MAX, n).unwrap();
    }
    d
}

fn path(d: &mut Document, p: PathData, fill: Paint, stroke: Paint, width: f64) -> Node {
    Node::path(d.alloc_id(), p, Appearance::basic(fill, stroke, width))
}

/// The options with the first artboard as the region, in points.
fn opts(d: &Document) -> DxfOptions {
    DxfOptions { region: d.artboards[0].rect, unit: Unit::Points, ..DxfOptions::default() }
}

/// A red-filled rectangle with a black 1 pt stroke, and an unfilled blue 2 pt ellipse.
fn shapes_doc() -> Document {
    doc_with(|d| {
        vec![
            path(d, shapes::rectangle(Rect::new(10.0, 10.0, 110.0, 60.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::BLACK), 1.0),
            path(d, shapes::ellipse(Rect::new(120.0, 20.0, 180.0, 80.0)), Paint::None, Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 2.0),
        ]
    })
}

#[test]
fn every_version_declares_its_acadver_and_is_well_formed() {
    let d = shapes_doc();
    for v in DxfVersion::ALL {
        let out = export(&d, &DxfOptions { version: v, ..opts(&d) }).unwrap();
        let p = pairs(&out.bytes);
        assert_eq!(header(&p, "$ACADVER").as_deref(), Some(v.acadver()), "{v:?}");
        assert_eq!(p.last(), Some(&(0, "EOF".to_string())), "{v:?}");
        let opened = p.iter().filter(|x| **x == (0, "SECTION".into())).count();
        assert_eq!(opened, p.iter().filter(|x| **x == (0, "ENDSEC".into())).count(), "{v:?}: balanced sections");
        assert!(!entities(&out).is_empty(), "{v:?}");
        if v >= DxfVersion::R13 {
            // Every handle is unique and below $HANDSEED.
            let body = &p[p.iter().position(|x| *x == (0, "ENDSEC".into())).unwrap()..];
            let handles: Vec<u32> = body.iter().filter(|x| x.0 == 5 || x.0 == 105).map(|x| u32::from_str_radix(&x.1, 16).unwrap()).collect();
            let unique: std::collections::HashSet<_> = handles.iter().collect();
            assert_eq!(unique.len(), handles.len(), "{v:?}: unique handles");
            let seed = u32::from_str_radix(&header(&p, "$HANDSEED").unwrap(), 16).unwrap();
            assert!(handles.iter().all(|h| *h < seed), "{v:?}: handles below the seed");
            assert!(section(&p, "OBJECTS").iter().any(|o| kind(o) == "DICTIONARY"), "{v:?}: root dictionary");
        } else {
            assert!(header(&p, "$HANDSEED").is_none());
        }
    }
    assert_eq!(DxfVersion::from_id("r2013"), Some(DxfVersion::R2013));
    assert_eq!(DxfVersion::from_id("AC1009"), Some(DxfVersion::R12));
    assert_eq!(DxfVersion::from_id("2019"), None);
}

#[test]
fn r12_viewport_has_no_duplicate_group_codes() {
    let d = shapes_doc();
    let out = export(&d, &DxfOptions { version: DxfVersion::R12, ..opts(&d) }).unwrap();
    let vport = table(&out, "VPORT").into_iter().find(|o| kind(o) == "VPORT").expect("VPORT table record");
    let codes: Vec<i32> = vport.iter().map(|pair| pair.0).collect();
    let unique: std::collections::HashSet<_> = codes.iter().collect();
    assert_eq!(unique.len(), codes.len(), "R12 VPORT record has duplicate group codes: {vport:?}");
    let z = vport.iter().find(|pair| pair.0 == 36).map(|pair| pair.1.trim().parse::<f64>().unwrap());
    assert_eq!(z, Some(1.0), "the view direction looks down the z axis");
}

#[test]
fn lines_are_lwpolylines_curves_splines_and_fills_hatches() {
    let d = shapes_doc();
    let out = export(&d, &opts(&d)).unwrap();
    assert_eq!(
        kinds(&out),
        ["LWPOLYLINE", "HATCH", "LWPOLYLINE", "CIRCLE"],
        "the fill's outline and hatch, then the stroke, then the circle's stroke"
    );
    let e = entities(&out);
    // The rectangle, y up from the artboard's bottom-left corner: 10..110 × 40..90.
    let corners = |poly: &Pairs| {
        let mut xs = all(poly, 10);
        xs.sort_by(f64::total_cmp);
        let mut ys = all(poly, 20);
        ys.sort_by(f64::total_cmp);
        (xs, ys)
    };
    let rect = (vec![10.0, 10.0, 110.0, 110.0], vec![40.0, 40.0, 90.0, 90.0]);
    for poly in [&e[0], &e[2]] {
        assert_eq!(get(poly, 70), Some("1"), "closed");
        assert_eq!(corners(poly), rect);
    }
    // Laser and cutter software skips hatches: the fill's outline comes first, without a lineweight.
    assert_eq!((get(&e[0], 370), get(&e[2], 370)), (None, Some("35")));
    let hatch = &e[1];
    assert_eq!((get(hatch, 2), get(hatch, 70), get(hatch, 91), get(hatch, 93)), (Some("SOLID"), Some("1"), Some("1"), Some("4")));
    // The circular ellipse is preserved as a native circle (#1299).
    let circle = &e[3];
    assert_eq!((all(circle, 10), all(circle, 20), all(circle, 40)), (vec![150.0], vec![50.0], vec![30.0]));
    // R14 has lightweight polylines but no lineweights: the stroke is a polyline width.
    let r14 = export(&d, &DxfOptions { version: DxfVersion::R14, ..opts(&d) }).unwrap();
    assert_eq!(get(&entities(&r14)[2], 43), Some("1"));
    assert_eq!(get(&entities(&r14)[2], 370), None);
    assert_eq!(get(&entities(&r14)[0], 43), None, "a fill's outline has no width");
    // R13 has splines and hatches, and polylines of vertices.
    let r13 = export(&d, &DxfOptions { version: DxfVersion::R13, ..opts(&d) }).unwrap();
    let poly = ["POLYLINE", "VERTEX", "VERTEX", "VERTEX", "VERTEX", "SEQEND"];
    assert_eq!(kinds(&r13), [&poly[..], &["HATCH"], &poly, &["CIRCLE"]].concat());
    // R12: visible stroke widths live on old POLYLINE entities, including curved strokes.
    let r12 = export(&d, &DxfOptions { version: DxfVersion::R12, ..opts(&d) }).unwrap();
    let k = kinds(&r12);
    assert!(k.iter().all(|k| matches!(k.as_str(), "POLYLINE" | "VERTEX" | "SEQEND")), "{k:?}");
    assert_eq!(k.iter().filter(|k| *k == "POLYLINE").count(), 3);
    assert_eq!(k.iter().filter(|k| *k == "CIRCLE").count(), 0);
    assert!(r12.warnings.iter().any(|w| w.contains("R12 has no fills")), "{:?}", r12.warnings);
}

#[test]
fn circles_and_circular_arcs_are_native_dxf_entities() {
    let d = doc_with(|d| {
        let circle = shapes::ellipse(Rect::new(20.0, 20.0, 80.0, 80.0));
        let c = Point::new(120.0, 50.0);
        let r = 30.0;
        let k = r * shapes::KAPPA;
        let mut arc = vectorcraft_geom::BezPath::new();
        arc.move_to(Point::new(c.x + r, c.y));
        arc.curve_to(Point::new(c.x + r, c.y - k), Point::new(c.x + k, c.y - r), Point::new(c.x, c.y - r));
        vec![
            path(d, circle, Paint::None, Paint::solid(Color::BLACK), 1.0),
            path(d, PathData::from_bezpath(&arc), Paint::None, Paint::solid(Color::BLACK), 1.0),
        ]
    });
    for version in DxfVersion::ALL.into_iter().filter(|version| *version != DxfVersion::R12) {
        let out = export(&d, &DxfOptions { version, ..opts(&d) }).unwrap();
        let e = entities(&out);
        assert_eq!(e.iter().map(|x| kind(x)).collect::<Vec<_>>(), ["CIRCLE", "ARC"], "{version:?}");
        assert_eq!((all(&e[0], 10), all(&e[0], 20), all(&e[0], 40)), (vec![50.0], vec![50.0], vec![30.0]), "{version:?}");
        assert_eq!((all(&e[1], 10), all(&e[1], 20), all(&e[1], 40)), (vec![120.0], vec![50.0], vec![30.0]), "{version:?}");
        assert_eq!((all(&e[1], 50), all(&e[1], 51)), (vec![0.0], vec![90.0]), "{version:?}");
    }

    // An actual ellipse is not close enough to a circle and remains a spline.
    let ellipse = doc_with(|d| vec![path(d, shapes::ellipse(Rect::new(20.0, 20.0, 100.0, 60.0)), Paint::None, Paint::solid(Color::BLACK), 1.0)]);
    assert_eq!(kinds(&export(&ellipse, &opts(&ellipse)).unwrap()), ["SPLINE"]);
}

#[test]
fn r12_stroked_circles_keep_polyline_width() {
    let d = doc_with(|d| vec![path(d, shapes::ellipse(Rect::new(20.0, 20.0, 80.0, 80.0)), Paint::None, Paint::solid(Color::BLACK), 3.0)]);
    let out = export(&d, &DxfOptions { version: DxfVersion::R12, ..opts(&d) }).unwrap();
    let e = entities(&out);
    assert_eq!(kind(&e[0]), "POLYLINE");
    assert_eq!((get(&e[0], 40), get(&e[0], 41)), (Some("3"), Some("3")));
    assert!(!e.iter().any(|entity| kind(entity) == "CIRCLE"));
}

#[test]
fn closed_circular_arc_with_a_closing_chord_is_not_a_circle() {
    let d = doc_with(|d| {
        let c = Point::new(50.0, 50.0);
        let r = 30.0;
        let k = r * shapes::KAPPA;
        let mut arc = vectorcraft_geom::BezPath::new();
        arc.move_to(Point::new(c.x + r, c.y));
        arc.curve_to(Point::new(c.x + r, c.y - k), Point::new(c.x + k, c.y - r), Point::new(c.x, c.y - r));
        arc.curve_to(Point::new(c.x - k, c.y - r), Point::new(c.x - r, c.y - k), Point::new(c.x - r, c.y));
        arc.curve_to(Point::new(c.x - r, c.y + k), Point::new(c.x - k, c.y + r), Point::new(c.x, c.y + r));
        arc.close_path();
        vec![path(d, PathData::from_bezpath(&arc), Paint::None, Paint::solid(Color::BLACK), 1.0)]
    });

    let modern = export(&d, &opts(&d)).unwrap();
    assert_eq!(kinds(&modern), ["SPLINE"]);
    let r12 = export(&d, &DxfOptions { version: DxfVersion::R12, ..opts(&d) }).unwrap();
    assert!(kinds(&r12).iter().any(|k| k == "POLYLINE"));
    assert!(!kinds(&r12).iter().any(|k| k == "CIRCLE"));
}

#[test]
fn a_closed_sliver_along_a_circle_is_not_a_circle() {
    let d = doc_with(|d| {
        let (cx, cy, r) = (60.0, 250.0, 200.0);
        let at = |deg: f64| {
            let rad = deg.to_radians();
            Point::new(cx + r * rad.cos(), cy + r * rad.sin())
        };
        let along = |sliver: &mut vectorcraft_geom::BezPath, deg0: f64, deg1: f64| {
            let p0 = at(deg0);
            let p3 = at(deg1);
            let step = (deg1 - deg0).to_radians();
            let k = 4.0 / 3.0 * (step / 4.0).tan();
            let r0 = deg0.to_radians();
            let r1 = deg1.to_radians();
            let p1 = Point::new(p0.x - r * r0.sin() * k, p0.y + r * r0.cos() * k);
            let p2 = Point::new(p3.x + r * r1.sin() * k, p3.y - r * r1.cos() * k);
            sliver.curve_to(p1, p2, p3);
        };
        let mut sliver = vectorcraft_geom::BezPath::new();
        sliver.move_to(at(-110.0));
        along(&mut sliver, -110.0, -90.0);
        along(&mut sliver, -90.0, -70.0);
        along(&mut sliver, -70.0, -110.0);
        sliver.close_path();
        vec![path(d, PathData::from_bezpath(&sliver), Paint::None, Paint::solid(Color::BLACK), 1.0)]
    });

    let modern = export(&d, &opts(&d)).unwrap();
    assert_ne!(kinds(&modern), ["CIRCLE"]);
    assert_eq!(kinds(&modern), ["SPLINE"]);
}

#[test]
fn cubic_with_hidden_angular_reversal_is_not_promoted_to_arc() {
    let d = doc_with(|d| {
        let mut b = vectorcraft_geom::BezPath::new();
        b.move_to(Point::new(-8.404028665134, 62.061475842818));
        b.curve_to(Point::new(-13.285253116737, 62.103123153545), Point::new(52.811163576591, 52.471253544459), Point::new(60.0, 50.0));
        vec![path(d, PathData::from_bezpath(&b), Paint::None, Paint::solid(Color::BLACK), 1.0)]
    });
    let dxf = export(&d, &opts(&d)).unwrap();
    assert_ne!(kinds(&dxf), ["ARC"]);
    assert_eq!(kinds(&dxf), ["SPLINE"]);
}

#[test]
fn spline_closed_flags_follow_path_intent_across_versions_without_changing_geometry() {
    let cases = [(true, true), (true, false), (false, false), (false, true)];
    let make_doc = |keep_closure: bool| {
        doc_with(|d| {
            cases
                .iter()
                .map(|&(closed, returns)| {
                    let start = Point::new(20.0, 20.0);
                    let mut b = kurbo::BezPath::new();
                    b.move_to(start);
                    b.curve_to((80.0, 0.0), (80.0, 80.0), (20.0, 80.0));
                    if returns {
                        b.curve_to((0.0, 80.0), (0.0, 20.0), (20.0, 20.0));
                    }
                    if closed {
                        if keep_closure {
                            b.close_path();
                        } else if !returns {
                            // The open reference has the same final straight segment.
                            b.line_to(start);
                        }
                    }
                    path(d, PathData::from_bezpath(&b), Paint::None, Paint::solid(Color::rgb(0.0, 0.0, 0.0)), 2.0)
                })
                .collect()
        })
    };
    let closed = make_doc(true);
    let open = make_doc(false);
    for version in DxfVersion::ALL {
        let out = export(&closed, &DxfOptions { version, ..opts(&closed) }).unwrap();
        let actual = entities(&out);
        if version == DxfVersion::R12 {
            let polys: Vec<_> = actual.iter().filter(|e| kind(e) == "POLYLINE").collect();
            assert_eq!(polys.len(), cases.len());
            for (poly, &(closed, _)) in polys.iter().zip(&cases) {
                assert_eq!(get(poly, 70), Some(if closed { "1" } else { "0" }));
            }
            assert!(!actual.iter().any(|e| kind(e) == "SPLINE"));
            continue;
        }
        let reference = entities(&export(&open, &DxfOptions { version, ..opts(&open) }).unwrap());
        assert_eq!(actual.len(), cases.len());
        for ((spline, reference), &(closed, _)) in actual.iter().zip(&reference).zip(&cases) {
            assert_eq!(kind(spline), "SPLINE");
            assert_eq!(get(spline, 70), Some(if closed { "9" } else { "8" }), "{version:?}");
            let mut normalized = spline.clone();
            normalized.iter_mut().filter(|p| p.0 == 70).for_each(|p| p.1 = "8".into());
            assert_eq!(&normalized, reference, "all other entity properties, knots and controls: {version:?}");
        }
    }
}

#[test]
fn colours_map_to_the_index_palette_or_true_colour() {
    let d = shapes_doc();
    let style = |o: &DxfOutput, i: usize| {
        let e = &entities(o)[i];
        (get(e, 62).map(str::to_string), get(e, 420).map(str::to_string))
    };
    let out = export(&d, &opts(&d)).unwrap();
    assert_eq!(style(&out, 0), (Some("1".into()), Some("16711680".into())), "red: index 1, true colour 0xFF0000");
    assert_eq!(style(&out, 1), style(&out, 0), "the fill's outline has the fill's colour");
    assert_eq!(style(&out, 2), (Some("7".into()), Some("0".into())), "black is index 7");
    assert_eq!(style(&out, 3), (Some("5".into()), Some("255".into())));
    for depth in [ColorDepth::Aci8, ColorDepth::Aci16, ColorDepth::Aci256] {
        let o = export(&d, &DxfOptions { colors: depth, ..opts(&d) }).unwrap();
        assert_eq!(style(&o, 0), (Some("1".into()), None), "{depth:?}");
    }
    let orange = doc_with(|d| {
        vec![path(d, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Paint::solid(Color::from_hex("#ff8000").unwrap()), Paint::None, 0.0)]
    });
    let aci = |depth| get(&entities(&export(&orange, &DxfOptions { colors: depth, ..opts(&orange) }).unwrap())[0], 62).unwrap().to_string();
    assert_eq!(
        (aci(ColorDepth::Aci256), aci(ColorDepth::Aci16), aci(ColorDepth::Aci8)),
        ("30".into(), "30".into(), "2".into()),
        "8 colours: yellow is nearest"
    );
    // True colour needs 2004: earlier versions fall back to indices, and say so.
    let old = export(&d, &DxfOptions { version: DxfVersion::R2000, ..opts(&d) }).unwrap();
    assert_eq!(style(&old, 0), (Some("1".into()), None));
    assert!(old.warnings.iter().any(|w| w.contains("true colour")));
}

#[test]
fn units_scale_coordinates_and_set_insunits() {
    let d = shapes_doc();
    let insunits = |o: &DxfOutput| header(&pairs(&o.bytes), "$INSUNITS");
    let max_x = |o: &DxfOutput| all(&entities(o)[2], 10).into_iter().fold(f64::MIN, f64::max);
    let mm = export(&d, &DxfOptions { unit: Unit::Millimeters, ..opts(&d) }).unwrap();
    assert_eq!(insunits(&mm).as_deref(), Some("4"));
    assert!((max_x(&mm) - 110.0 * 25.4 / 72.0).abs() < 1e-5);
    assert_eq!(header(&pairs(&mm.bytes), "$MEASUREMENT").as_deref(), Some("1"));
    let inches = export(&d, &DxfOptions { unit: Unit::Inches, ..opts(&d) }).unwrap();
    assert_eq!(insunits(&inches).as_deref(), Some("1"));
    // 1 in = 10 units: unitless, coordinates ten times larger.
    let scaled = export(&d, &DxfOptions { unit: Unit::Inches, scale: 10.0, ..opts(&d) }).unwrap();
    assert_eq!(insunits(&scaled).as_deref(), Some("0"));
    assert!((max_x(&scaled) - 110.0 / 72.0 * 10.0).abs() < 1e-5);
    // Lineweights: 1 pt is 0.35 mm, scaled with the drawing only when asked.
    assert_eq!(get(&entities(&scaled)[2], 370), Some("35"));
    let heavy = export(&d, &DxfOptions { unit: Unit::Inches, scale: 2.0, scale_lineweights: true, ..opts(&d) }).unwrap();
    assert_eq!(get(&entities(&heavy)[2], 370), Some("70"));
    assert!(export(&d, &DxfOptions { scale: 0.0, ..opts(&d) }).is_err());
    assert!(export(&d, &DxfOptions { scale: f64::NAN, ..opts(&d) }).is_err());
}

#[test]
fn layers_become_dxf_layers() {
    let mut d = shapes_doc();
    let second = d.add_layer(Some("Dims: <top>"));
    let hidden = d.add_layer(Some("Notes"));
    let template = d.add_layer(Some("Tracing"));
    for l in [second, hidden, template] {
        let n = path(&mut d, shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), Paint::solid(Color::BLACK), Paint::None, 0.0);
        d.insert(Some(l), 0, n).unwrap();
    }
    if let Some(n) = d.node_mut(hidden) {
        n.visible = false;
    }
    if let Some(NodeKind::Layer { template, .. }) = d.node_mut(template).map(|n| &mut n.kind) {
        *template = true;
    }
    let out = export(&d, &opts(&d)).unwrap();
    let layers: Vec<(String, String)> =
        table(&out, "LAYER").iter().map(|o| (get(o, 2).unwrap().to_string(), get(o, 62).unwrap().to_string())).collect();
    let names: Vec<&str> = layers.iter().map(|l| l.0.as_str()).collect();
    assert_eq!(names, ["0", "Layer 1", "Dims_ _top_", "Notes"], "the template layer is left out");
    assert!(layers[3].1.starts_with('-'), "a hidden layer is switched off");
    let on: Vec<String> = entities(&out).iter().map(|e| get(e, 8).unwrap().to_string()).collect();
    assert_eq!(on, ["Layer 1", "Layer 1", "Layer 1", "Layer 1", "Dims_ _top_", "Dims_ _top_", "Notes", "Notes"]);
    // Before 2000: capitals, letters, digits and $-_ only.
    let r12 = export(&d, &DxfOptions { version: DxfVersion::R12, ..opts(&d) }).unwrap();
    let names: Vec<String> = table(&r12, "LAYER").iter().map(|o| get(o, 2).unwrap().to_string()).collect();
    assert_eq!(names, ["0", "LAYER_1", "DIMS___TOP_", "NOTES"]);
}

#[test]
fn dashes_become_linetypes_and_opacity_transparency() {
    let d = doc_with(|d| {
        let mut n = path(d, shapes::line(Point::new(0.0, 50.0), Point::new(100.0, 50.0)), Paint::None, Paint::solid(Color::BLACK), 1.0);
        if let Some(st) = n.appearance.stroke_mut() {
            st.dash = Some(Dash { pattern: vec![6.0, 3.0], offset: 0.0, align_corners: false });
        }
        n.opacity = 0.5;
        vec![n]
    });
    let out = export(&d, &opts(&d)).unwrap();
    let e = &entities(&out)[0];
    let alpha = (0x0200_0000 | 128).to_string();
    assert_eq!((get(e, 6), get(e, 440)), (Some("DASHED1"), Some(alpha.as_str())));
    let lt = table(&out, "LTYPE").into_iter().find(|o| get(o, 2) == Some("DASHED1")).unwrap();
    assert_eq!(all(&lt, 49), [6.0, -3.0]);
    let old = export(&d, &DxfOptions { version: DxfVersion::R2000, ..opts(&d) }).unwrap();
    assert!(get(&entities(&old)[0], 440).is_none());
    assert!(old.warnings.iter().any(|w| w.contains("transparency")));
}

#[test]
fn alter_paths_writes_strokes_as_filled_outlines() {
    let d = shapes_doc();
    let out = export(&d, &DxfOptions { alter_paths: true, ..opts(&d) }).unwrap();
    // Each fill and stroke: its outlines, then its hatch.
    let fill = ["LWPOLYLINE", "HATCH"];
    let rect_stroke = ["LWPOLYLINE", "LWPOLYLINE", "HATCH"];
    assert_eq!(kinds(&out), [&fill[..], &rect_stroke, &["CIRCLE", "CIRCLE", "HATCH"]].concat());
    // The rectangle's stroke: an outer and an inner boundary.
    assert_eq!(get(&entities(&out)[4], 91), Some("2"));
    // Preserve Appearance outlines only the strokes a line can't draw.
    let mut d = shapes_doc();
    let rect = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
    if let Some(st) = d.node_mut(rect).and_then(|n| n.appearance.stroke_mut()) {
        st.align = vectorcraft_doc::StrokeAlign::Outside;
    }
    assert_eq!(kinds(&export(&d, &opts(&d)).unwrap()), [&fill[..], &rect_stroke, &["CIRCLE"]].concat());
    let editable = export(&d, &DxfOptions { preserve: Preserve::Editability, ..opts(&d) }).unwrap();
    assert_eq!(kinds(&editable), ["LWPOLYLINE", "HATCH", "LWPOLYLINE", "CIRCLE"]);
    assert!(editable.warnings.iter().any(|w| w.contains("stroke alignment")));
}

fn text_doc(text: &str, size: f64) -> Document {
    doc_with(|d| {
        let style = CharStyle { size, fill: Paint::solid(Color::rgb(0.0, 0.5, 0.0)), ..CharStyle::default() };
        let t = TextObject::point(Point::new(10.0, 50.0), text, style);
        vec![Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)))]
    })
}

#[test]
fn type_is_text_or_outlines() {
    let d = text_doc("Plan A", 20.0);
    let editable = export(&d, &DxfOptions { preserve: Preserve::Editability, ..opts(&d) }).unwrap();
    let e = entities(&editable);
    assert_eq!(e.len(), 1);
    assert_eq!((kind(&e[0]), get(&e[0], 1)), ("TEXT", Some("Plan A")));
    assert!((all(&e[0], 40)[0] - 14.0).abs() < 1e-6, "the cap height of 20 pt type");
    assert_eq!((all(&e[0], 10)[0], all(&e[0], 20)[0]), (10.0, 50.0), "the baseline start, y up");
    let family = CharStyle::default().font_family;
    let style = table(&editable, "STYLE").into_iter().find(|o| get(o, 3) == Some(&*format!("{family}.ttf"))).unwrap();
    assert_eq!(get(&e[0], 7), get(&style, 2));
    // Outline Text, and Preserve Appearance: the glyphs as filled outlines.
    for o in [DxfOptions { preserve: Preserve::Editability, outline_text: true, ..opts(&d) }, opts(&d)] {
        let k = kinds(&export(&d, &o).unwrap());
        assert!(k.iter().any(|k| k == "HATCH"), "{k:?}");
        assert!(k.iter().all(|k| matches!(k.as_str(), "HATCH" | "LWPOLYLINE" | "SPLINE")), "{k:?}");
        assert_eq!(k.first().map(String::as_str), Some("SPLINE"), "a glyph's outline before its hatch");
    }
}

/// A list paragraph's bullet is a TEXT entity of its own where it is drawn, and the paragraph's
/// text starts after it, at the left indent.
#[test]
fn a_bullet_is_text_of_its_own_before_its_paragraph() {
    let mut d = text_doc("Item", 20.0);
    let id = d.layers[0].children().unwrap()[0].id;
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.para.list = Some(vectorcraft_doc::text::ParaList::bulleted(vectorcraft_doc::text::Bullet::Dot));
        t.para.left_indent = 18.0;
        t.para.first_line_indent = -18.0;
    }
    let out = export(&d, &DxfOptions { version: DxfVersion::R2018, preserve: Preserve::Editability, ..opts(&d) }).unwrap();
    let e = entities(&out);
    assert_eq!(kinds(&out), ["TEXT", "TEXT"]);
    assert_eq!((get(&e[0], 1), get(&e[1], 1)), (Some("\u{2022}"), Some("Item")));
    assert_eq!((all(&e[0], 10)[0], all(&e[1], 10)[0]), (10.0, 28.0), "the bullet at the anchor, the text 18 pt on");
    // In a right-to-left paragraph the bullet is drawn after the text, at the right, and its tab
    // lies between the two.
    let mut d = text_doc("שלום", 20.0);
    let id = d.layers[0].children().unwrap()[0].id;
    let mut bullet_x = f64::NAN;
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.para.list = Some(vectorcraft_doc::text::ParaList::bulleted(vectorcraft_doc::text::Bullet::Dot));
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let m = &lay.markers[0];
        let bullet = m.glyphs.clone().find(|&i| i != m.tab).unwrap();
        assert!(lay.glyphs[m.tab].origin.x < lay.glyphs[bullet].origin.x, "the tab before the bullet");
        bullet_x = 10.0 + lay.glyphs[bullet].origin.x;
    }
    let out = export(&d, &DxfOptions { version: DxfVersion::R2018, preserve: Preserve::Editability, ..opts(&d) }).unwrap();
    let e = entities(&out);
    assert_eq!(kinds(&out), ["TEXT", "TEXT"]);
    assert_eq!((get(&e[0], 1), get(&e[1], 1)), (Some("שלום"), Some("\u{2022}")));
    assert!((all(&e[1], 10)[0] - bullet_x).abs() < 1e-6, "the bullet where it is drawn: {} and {bullet_x}", all(&e[1], 10)[0]);
}

/// A numbered list paragraph's number and period are a TEXT entity of their own where they are
/// drawn, in the order they are drawn: in a right-to-left paragraph the period comes first, at the
/// number's left.
#[test]
fn a_number_is_text_of_its_own_before_its_paragraph() {
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    let numbered = |text: &str, count: u32| {
        let mut d = text_doc(text, 20.0);
        let id = d.layers[0].children().unwrap()[0].id;
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            t.para.list = Some(ParaList::numbered(NumberStyle::Arabic));
            t.para.left_indent = 30.0;
            t.para.first_line_indent = -30.0;
            t.list_count = count;
        }
        let out = export(&d, &DxfOptions { version: DxfVersion::R2018, preserve: Preserve::Editability, ..opts(&d) }).unwrap();
        entities(&out).iter().map(|e| (get(e, 1).map(str::to_string), all(e, 10)[0])).collect::<Vec<_>>()
    };
    assert_eq!(numbered("Item", 11), [(Some("12.".to_string()), 10.0), (Some("Item".to_string()), 40.0)]);
    let rtl = numbered("שלום", 11);
    assert_eq!(rtl.iter().map(|(s, _)| s.clone().unwrap_or_default()).collect::<Vec<_>>(), ["שלום", ".12"]);
    assert!(rtl[1].1 > rtl[0].1, "the number right of the text: {rtl:?}");
}

/// Area type that Shrink Text to Fit scales down is written at the height it draws at.
#[test]
fn shrunk_area_type_is_text_at_the_size_it_draws_at() {
    let mut d = text_doc("Plan A, plan B and plan C", 20.0);
    let id = d.layers[0].children().unwrap()[0].id;
    let mut fit = 1.0;
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 120.0, 30.0)) };
        t.area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 10.0 };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        assert!(!lay.overflow && lay.fit_scale < 0.9, "the text is shrunk to fit: {}", lay.fit_scale);
        fit = lay.fit_scale;
    }
    let out = export(&d, &DxfOptions { preserve: Preserve::Editability, ..opts(&d) }).unwrap();
    let e = entities(&out);
    assert!(!e.is_empty() && e.iter().all(|e| kind(e) == "TEXT"), "{:?}", kinds(&out));
    for e in &e {
        assert!((all(e, 40)[0] - 14.0 * fit).abs() < 1e-6, "the cap height of {} pt type: {:?}", 20.0 * fit, all(e, 40));
    }
}

#[test]
fn text_outside_ascii_is_escaped_before_2007() {
    let d = text_doc("Größe", 12.0);
    let text = |v| {
        let out = export(&d, &DxfOptions { version: v, preserve: Preserve::Editability, ..opts(&d) }).unwrap();
        get(&entities(&out)[0], 1).unwrap().to_string()
    };
    assert_eq!(text(DxfVersion::R2004), "Gr\\U+00F6\\U+00DFe");
    assert_eq!(text(DxfVersion::R2018), "Größe");
}

#[test]
fn images_link_png_or_jpeg_files() {
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 128])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut d = doc_with(|d| {
        let im = ImageObject {
            key: "raster-1".into(),
            width: 4,
            height: 2,
            xf: Affine::translate((10.0, 20.0)) * Affine::scale(5.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        vec![Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)))]
    });
    d.images.insert("raster-1".into(), ImageBlob { mime: "image/png".into(), bytes: Arc::new(png), proxy: None });
    let out = export(&d, &opts(&d)).unwrap();
    let e = entities(&out);
    assert_eq!(kind(&e[0]), "IMAGE");
    // The bottom-left corner (10, 30) is (10, 70) y up; a pixel is 5 units.
    assert_eq!((all(&e[0], 10)[0], all(&e[0], 20)[0]), (10.0, 70.0));
    assert_eq!((all(&e[0], 11)[0], all(&e[0], 22)[0], all(&e[0], 13)[0], all(&e[0], 23)[0]), (5.0, 5.0, 4.0, 2.0));
    assert_eq!(out.images.len(), 1);
    assert!(out.images[0].name.ends_with(".png") && out.images[0].bytes.starts_with(b"\x89PNG"));
    let p = pairs(&out.bytes);
    let objects = section(&p, "OBJECTS");
    let def = objects.iter().find(|o| kind(o) == "IMAGEDEF").unwrap();
    assert_eq!(get(def, 1), Some(out.images[0].name.as_str()));
    assert_eq!(get(&e[0], 340), get(def, 5), "the image points at its definition");
    assert!(objects.iter().any(|o| kind(o) == "IMAGEDEF_REACTOR"));
    assert!(section(&p, "CLASSES").iter().any(|o| get(o, 1) == Some("IMAGE")));
    let jpeg = export(&d, &DxfOptions { raster: RasterFormat::Jpeg, ..opts(&d) }).unwrap();
    assert!(jpeg.images[0].name.ends_with(".jpg") && jpeg.images[0].bytes.starts_with(&[0xFF, 0xD8]));
    let r12 = export(&d, &DxfOptions { version: DxfVersion::R12, ..opts(&d) }).unwrap();
    assert!(entities(&r12).is_empty() && r12.images.is_empty());
    assert!(r12.warnings.iter().any(|w| w.contains("no images")));
}

#[test]
fn images_without_pixels_are_left_out_with_one_note() {
    // One image has no data in the document, and the other image's data is empty.
    let mut d = doc_with(|d| {
        ["raster-1", "raster-2"]
            .into_iter()
            .map(|key| {
                let im = ImageObject {
                    key: key.into(),
                    width: 4,
                    height: 2,
                    xf: Affine::IDENTITY,
                    link: None,
                    placement: Default::default(),
                    pixels: None,
                };
                Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)))
            })
            .collect()
    });
    d.images.insert("raster-2".into(), ImageBlob { mime: "image/png".into(), bytes: Arc::new(vec![]), proxy: None });
    let out = export(&d, &opts(&d)).unwrap();
    assert!(entities(&out).is_empty() && out.images.is_empty());
    assert_eq!(out.warnings, ["an image has no pixels and was left out"]);
}

#[test]
fn crop_leaves_out_art_outside_the_region() {
    let d = shapes_doc();
    // Only the ellipse (120..180) reaches into 115..200.
    let o = DxfOptions { region: Rect::new(115.0, 0.0, 200.0, 100.0), crop: true, ..opts(&d) };
    assert_eq!(kinds(&export(&d, &o).unwrap()), ["CIRCLE"]);
    assert_eq!(kinds(&export(&d, &DxfOptions { crop: false, ..o }).unwrap()).len(), 4);
}

#[test]
fn crop_keeps_an_instance_whose_art_reaches_the_region() {
    // The symbol's red rectangle, (100, 50) to (140, 90), drawn from (-50, -50): on the artboard,
    // while the 20 × 20 square around the origin, moved to (-60, -60) to (-40, -40), is off it.
    let mut d = doc_with(|d| vec![Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((-50.0, -50.0)) })]);
    let art = path(&mut d, shapes::rectangle(Rect::new(100.0, 50.0, 140.0, 90.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Far".into(), art: Arc::new(art) });
    let o = DxfOptions { crop: true, ..opts(&d) };
    assert!(!kinds(&export(&d, &o).unwrap()).is_empty());
}
