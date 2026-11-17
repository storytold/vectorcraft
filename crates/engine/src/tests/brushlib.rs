//! Brush libraries: the built-in libraries, saving a document's brushes as a library (User
//! Defined, or a file), reopening it and adding its brushes to another document (the patterns and
//! symbols their art uses along, one undo step, deduped, renamed on a clash, applied), other
//! documents' brushes, and files that can't be read.

use serde_json::{Value, json};
use vectorcraft_brush::BrushKind;
use vectorcraft_doc::{NodeId, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn brushes(s: &Session) -> Vec<String> {
    vectorcraft_brush::library(doc(s)).into_iter().map(|b| b.name).collect()
}

fn brush(s: &Session, name: &str) -> vectorcraft_brush::Brush {
    vectorcraft_brush::find(doc(s), name).unwrap_or_else(|| panic!("no brush {name}"))
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect()
}

/// The patterns a brush's art fills with.
fn art_patterns(b: &vectorcraft_brush::Brush) -> Vec<String> {
    let mut out = vec![];
    for a in b.art() {
        a.walk(&mut |n| {
            for it in &n.appearance.items {
                if let vectorcraft_color::Paint::Pattern { pattern, .. } = it.paint() {
                    out.push(pattern.clone());
                }
            }
        });
    }
    out
}

/// A folder of its own under the temp folder, emptied.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-brushlib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Pattern `name` (a 4 pt dot of `color` in a 10 pt tile) with its swatch, as one step.
fn add_pattern(s: &mut Session, name: &str, color: &str) {
    let color = vectorcraft_color::Color::from_hex(color).unwrap();
    s.edit("Pattern", |d, _| {
        let dot = vectorcraft_doc::Node::path(
            d.alloc_id(),
            vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(0.0, 0.0, 4.0, 4.0)),
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(color), vectorcraft_color::Paint::None, 0.0),
        );
        let def = vectorcraft_doc::PatternDef {
            tile: vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0),
            ..vectorcraft_doc::PatternDef::new(name, vec![std::sync::Arc::new(dot)])
        };
        d.swatches.push(vectorcraft_color::Swatch {
            name: name.into(),
            paint: vectorcraft_color::Paint::Pattern { pattern: name.into(), xf: Default::default() },
            global: false,
            spot: false,
        });
        d.patterns.push(def);
        Ok(())
    })
    .unwrap();
}

/// A document with brushes of its own: "My Nib" (calligraphic) and "Dotted Band" (an art brush
/// whose art is filled with pattern "Dots").
fn kit() -> Session {
    let mut s = session();
    run(&mut s, "brush.new", json!({"type": "calligraphic", "name": "My Nib", "params": {"angle": 20, "roundness": 40, "size": 7}}));
    add_pattern(&mut s, "Dots", "#ff0000");
    let band = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 8}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Dots"}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Dotted Band", "ids": [band]}));
    s
}

#[test]
fn the_builtin_libraries_are_original_brushes_generated_in_code() {
    let mut s = Session::new();
    let list = run(&mut s, "brush.libraries", json!({}));
    let libs = list["libraries"].as_array().unwrap();
    let ids: Vec<&str> = libs.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "art-ribbons",
            "ink-and-wash",
            "calligraphic-nibs",
            "bristle-brushes",
            "pattern-borders",
            "scatter-shapes",
            "game-trails",
            "energy-beams",
            "dry-media",
            "tech-circuit"
        ]
    );
    assert!(libs.iter().all(|l| l["category"] == "builtIn"));
    let lib = run(&mut s, "brush.library", json!({"library": "art ribbons"}));
    assert_eq!(
        lib["brushes"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect::<Vec<_>>(),
        ["Notched Ribbon", "Double Line", "Wave", "Teardrop", "Striped Ribbon"]
    );
    assert_eq!(libs[0]["count"].as_u64(), Some(5));
    let trails = run(&mut s, "brush.library", json!({"library": "game-trails"}));
    assert_eq!(
        trails["brushes"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect::<Vec<_>>(),
        ["Speed Line", "Dash Trail", "Burst Ray", "Zigzag"]
    );
    assert_eq!(libs[6]["count"].as_u64(), Some(4));
    let beams = run(&mut s, "brush.library", json!({"library": "energy-beams"}));
    assert_eq!(
        beams["brushes"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect::<Vec<_>>(),
        ["Laser Bolt", "Plasma Arc", "Charge Wave", "Spark Trail"]
    );
    assert_eq!(libs[7]["count"].as_u64(), Some(4));
    let dry = run(&mut s, "brush.library", json!({"library": "Dry Media"}));
    assert_eq!(
        dry["brushes"].as_array().unwrap().iter().map(|b| (b["name"].as_str().unwrap(), b["type"].as_str().unwrap())).collect::<Vec<_>>(),
        [("Chalk Grain", "art"), ("Graphite Pencil", "art"), ("Wax Crayon", "art"), ("Charcoal Dust", "art")]
    );
    assert_eq!((libs[8]["name"].as_str(), libs[8]["count"].as_u64()), (Some("Dry Media"), Some(4)));
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 20, "width": 40, "height": 8}))["id"].as_u64().unwrap();
    let added = run(&mut s, "brush.addFromLibrary", json!({"library": "energy-beams", "name": "Laser Bolt", "apply": true, "ids": [id]}));
    assert_eq!(strs(&added["added"]), ["Laser Bolt"]);
    // One kind of brush per library (Ink and Wash: art brushes and a calligraphic one), every
    // brush drawing something.
    for (id, kind) in [
        ("art-ribbons", "art"),
        ("ink-and-wash", "art"),
        ("calligraphic-nibs", "calligraphic"),
        ("bristle-brushes", "bristle"),
        ("pattern-borders", "pattern"),
        ("scatter-shapes", "scatter"),
        ("game-trails", "art"),
        ("energy-beams", "art"),
        ("dry-media", "art"),
    ] {
        let (_, lib) = cmd::brushlib::library(&s, id).unwrap();
        assert!(lib.len() >= 4, "{id}");
        for b in &lib.brushes {
            let kind = if b.name == "Brush Pen" { "calligraphic" } else { kind };
            assert_eq!(b.kind.type_id(), kind, "{}", b.name);
            let mut line = vectorcraft_geom::BezPath::new();
            line.move_to((0.0, 0.0));
            line.curve_to((60.0, -40.0), (140.0, 40.0), (200.0, 0.0));
            let stroke = vectorcraft_doc::StrokeLayer::new(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), 1.0);
            assert!(!vectorcraft_brush::stroke_pieces(&lib.doc, b, &line, &stroke).is_empty(), "{} draws nothing", b.name);
        }
    }
    // The resources their art uses come with them: Striped Ribbon's pattern, Sparkles' symbol.
    let (_, ribbons) = cmd::brushlib::library(&s, "art-ribbons").unwrap();
    assert_eq!(art_patterns(ribbons.brush("Striped Ribbon").unwrap()), ["Pinstripes"]);
    assert!(ribbons.doc.pattern("Pinstripes").is_some());
    let (_, shapes) = cmd::brushlib::library(&s, "scatter-shapes").unwrap();
    let BrushKind::Scatter(sparkles) = &shapes.brush("Sparkles").unwrap().kind else { panic!("not a scatter brush") };
    assert!(matches!(&sparkles.art.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Sparkle"));
    assert!(shapes.doc.symbols.iter().any(|x| x.name == "Sparkle"));
    // None of them is a default brush.
    let defaults: Vec<&str> = vectorcraft_brush::defaults().iter().map(|b| b.name.as_str()).collect();
    for id in ids {
        let (_, lib) = cmd::brushlib::library(&s, id).unwrap();
        assert!(lib.brushes.iter().all(|b| !defaults.contains(&b.name.as_str())), "{id}");
    }
    assert!(s.execute("brush.library", &json!({"library": "nope"})).is_err());
}

/// Every path of brush art `n`: (anchors, opacity).
fn art_paths(n: &vectorcraft_doc::Node) -> Vec<(usize, f32)> {
    let mut out = vec![];
    n.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            out.push((p.anchor_count(), c.opacity));
        }
    });
    out
}

/// The Ink and Wash library, made once.
fn inks(s: &Session) -> std::sync::Arc<cmd::brushlib::BrushLibrary> {
    cmd::brushlib::library(s, "ink-and-wash").unwrap().1
}

/// A straight line from the origin to (`x`, 0).
fn line_to(x: f64) -> vectorcraft_geom::BezPath {
    let mut line = vectorcraft_geom::BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((x, 0.0));
    line
}

fn black_stroke() -> vectorcraft_doc::StrokeLayer {
    vectorcraft_doc::StrokeLayer::new(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), 1.0)
}

/// Original circuit art and repeat tiles survive adding, applying and saving a library.
#[test]
fn the_tech_circuit_library_constructs_adds_and_applies_each_brush() {
    let mut s = session();
    let listed = run(&mut s, "brush.libraries", json!({}));
    let entry = listed["libraries"].as_array().unwrap().iter().find(|l| l["id"] == "tech-circuit").unwrap();
    assert_eq!((entry["name"].as_str(), entry["category"].as_str(), entry["count"].as_u64()), (Some("Tech Circuit"), Some("builtIn"), Some(3)));
    let (_, lib) = cmd::brushlib::library(&s, "Tech Circuit").unwrap();
    let expected = [("Circuit Trace", "art"), ("Bus Lines", "pattern"), ("Terminal Nodes", "pattern")];
    assert_eq!(lib.brushes.iter().map(|b| (b.name.as_str(), b.kind.type_id())).collect::<Vec<_>>(), expected);
    let path = run(&mut s, "shape.line", json!({"x1": 10, "y1": 20, "x2": 210, "y2": 20}))["id"].as_u64().unwrap();
    let curve = {
        let mut p = line_to(40.0);
        p.curve_to((60.0, -40.0), (140.0, 40.0), (200.0, 0.0));
        p
    };
    let corner = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 80.0, 60.0)).to_bezpath();
    for b in &lib.brushes {
        let mut sanitized = b.clone();
        sanitized.sanitize();
        assert_eq!(&sanitized, b, "{} changes on load", b.name);
        for tile in b.art() {
            let pts = anchors(&[tile]);
            assert!(!pts.is_empty() && pts.len() <= 100, "{}", b.name);
            assert!(pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
            tile.walk(&mut |n| {
                if let Some(p) = n.path_data() {
                    assert!(p.subpaths.iter().all(|s| s.closed));
                }
            });
        }
        if let BrushKind::Pattern(p) = &b.kind {
            assert_eq!(p.colorization, vectorcraft_brush::Colorization::Tints);
            assert_eq!(p.spacing, 0.0, "conductors join adjacent tiles");
            if b.name == "Bus Lines" {
                let lanes = p.side.children().unwrap();
                assert_eq!(lanes.len(), 3);
                let boxes: Vec<_> = lanes.iter().map(|n| n.path_data().unwrap().bounds().unwrap()).collect();
                for r in &boxes {
                    assert_eq!((r.x0, r.x1), (0.0, 24.0));
                }
                assert!(boxes.windows(2).all(|r| r[0].y1 < r[1].y0), "lanes stay separate");
            }
            if b.name == "Terminal Nodes" {
                let mut rings = 0;
                p.side.walk(&mut |n| {
                    if matches!(n.kind, NodeKind::Compound { rule: vectorcraft_geom::FillRule::EvenOdd, .. }) {
                        rings += 1;
                    }
                });
                assert_eq!(rings, 1, "a hollow circular test point per tile");
            }
        }
        for p in [line_to(200.0), line_to(2.0), curve.clone(), corner.clone()] {
            let pieces = vectorcraft_brush::stroke_pieces(&lib.doc, b, &p, &black_stroke());
            let pts = anchors(&pieces.iter().collect::<Vec<_>>());
            assert!(!pts.is_empty() && pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{}", b.name);
        }
        let before = undo_len(&s);
        let r = run(&mut s, "brush.addFromLibrary", json!({"library": "tech-circuit", "name": b.name, "apply": true, "ids": [path]}));
        assert_eq!(strs(&r["added"]), [b.name.as_str()]);
        assert_eq!(r["applied"], json!([path]));
        assert_eq!(undo_len(&s), before + 1);
        assert_eq!(brush(&s, &b.name), *b);
        assert_eq!(doc(&s).node(NodeId(path)).unwrap().appearance.stroke().unwrap().brush.as_deref(), Some(b.name.as_str()));
        run(&mut s, "edit.undo", json!({}));
        assert!(vectorcraft_brush::find(doc(&s), &b.name).is_none());
        run(&mut s, "edit.redo", json!({}));
        let before = undo_len(&s);
        let again = run(&mut s, "brush.addFromLibrary", json!({"library": "tech-circuit", "name": b.name}));
        assert!(strs(&again["added"]).is_empty());
        assert_eq!(strs(&again["existing"]), [b.name.as_str()]);
        assert_eq!(undo_len(&s), before);
    }
    let names: Vec<&str> = expected.iter().map(|(name, _)| *name).collect();
    let saved = run(&mut s, "brush.saveLibrary", json!({"names": names}));
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    let reopened = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(vectorcraft_brush::library(&reopened), lib.brushes);
}

/// Ink and Wash: a tapered ink art brush, a brush pen whose size follows the pen pressure and two
/// translucent washes, with names of their own and a few paths of art each.
#[test]
fn the_ink_and_wash_library_has_tapered_ink_a_brush_pen_and_two_washes() {
    let mut s = session();
    let lib = run(&mut s, "brush.library", json!({"library": "ink-and-wash"}));
    assert_eq!((lib["name"].as_str(), lib["category"].as_str()), (Some("Ink and Wash"), Some("builtIn")));
    let listed: Vec<(&str, &str)> =
        lib["brushes"].as_array().unwrap().iter().map(|b| (b["name"].as_str().unwrap(), b["type"].as_str().unwrap())).collect();
    assert_eq!(listed, [("Tapered Ink", "art"), ("Brush Pen", "calligraphic"), ("Soft Wash", "art"), ("Pooling Wash", "art")]);
    assert_eq!(run(&mut s, "brush.library", json!({"library": "Ink and Wash"}))["id"], "ink-and-wash");
    let inks = inks(&s);
    // Names no other built-in library uses.
    for b in cmd::brushlib::BUILTIN.iter().filter(|b| b.id != "ink-and-wash") {
        let (_, other) = cmd::brushlib::library(&s, b.id).unwrap();
        assert!(inks.brushes.iter().all(|x| other.brush(&x.name).is_none()), "{}", b.id);
    }
    // The art: within its 100 pt length, a few paths each, opaque ink and translucent washes.
    let mut anchors = 0;
    for name in ["Tapered Ink", "Soft Wash", "Pooling Wash"] {
        let BrushKind::Art(a) = &inks.brush(name).unwrap().kind else { panic!("{name} is not an art brush") };
        assert_eq!((a.scale, a.colorization), (vectorcraft_brush::ArtScale::Stretch, vectorcraft_brush::Colorization::Tints), "{name}");
        let b = a.art.geometric_bounds().unwrap();
        assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()), "{name}");
        assert!(b.x0 >= 0.0 && b.x1 <= 100.0 && b.x1 - b.x0 > 99.0, "{name}: {b:?}");
        assert!(b.y0 >= -8.0 && b.y1 <= 8.0 && b.y1 - b.y0 > 5.0, "{name}: {b:?}");
        let paths = art_paths(&a.art);
        let count: usize = paths.iter().map(|p| p.0).sum();
        assert!(paths.len() <= 80 && count <= 500, "{name}: {} paths, {count} anchors", paths.len());
        anchors += count;
        let opacities: Vec<f32> = paths.iter().map(|p| p.1).collect();
        if name == "Tapered Ink" {
            assert_eq!(opacities, [1.0], "opaque ink");
        } else {
            assert!(paths.len() >= 3, "{name}: a wash of several layers");
            assert!(opacities.iter().all(|o| (0.15..=0.55).contains(o)), "{name}: {opacities:?}");
        }
    }
    assert!(anchors <= 1200, "{anchors} anchors");
    let BrushKind::Calligraphic(pen) = &inks.brush("Brush Pen").unwrap().kind else { panic!("Brush Pen is not calligraphic") };
    use vectorcraft_brush::Variation::{Fixed, Pressure};
    assert_eq!((pen.size, pen.modes(), pen.variation), (5.0, [Fixed, Fixed, Pressure], [0.0, 0.0, 2.5]), "no random variation");
}

/// Every Ink and Wash brush draws along straight, curved, short and tight paths within finite
/// bounds, and Tapered Ink is broad near its start and fine at its end.
#[test]
fn ink_and_wash_brushes_draw_along_straight_curved_short_and_tight_paths() {
    let inks = inks(&Session::new());
    let mut curve = vectorcraft_geom::BezPath::new();
    curve.move_to((0.0, 0.0));
    curve.curve_to((60.0, -40.0), (140.0, 40.0), (200.0, 0.0));
    let tight = vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(0.0, 0.0, 6.0, 6.0)).to_bezpath();
    let paths = [line_to(200.0), line_to(3.0), curve, tight];
    for b in &inks.brushes {
        for line in &paths {
            let pieces = vectorcraft_brush::stroke_pieces(&inks.doc, b, line, &black_stroke());
            assert!(!pieces.is_empty(), "{} draws nothing", b.name);
            let bounds: Vec<_> = pieces.iter().map(|p| p.geometric_bounds()).collect();
            assert!(bounds.iter().all(|r| r.is_some_and(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()))), "{}", b.name);
        }
    }
    let pieces = vectorcraft_brush::stroke_pieces(&inks.doc, inks.brush("Tapered Ink").unwrap(), &line_to(100.0), &black_stroke());
    let widest = |from: f64, to: f64| {
        pieces
            .iter()
            .filter_map(|n| n.path_data())
            .flat_map(|p| p.anchors().map(|(_, _, a)| a.p).collect::<Vec<_>>())
            .filter(|p| p.x >= from && p.x <= to)
            .map(|p| p.y.abs())
            .fold(0.0, f64::max)
    };
    assert!(widest(15.0, 30.0) > 4.0 * widest(95.0, 100.0), "broad start, fine tip");
}

/// Ink and Wash brushes are added and applied in one step, undone and redone, not added twice,
/// and kept with the brushed stroke by a saved and reopened document.
#[test]
fn ink_and_wash_brushes_add_apply_undo_and_survive_a_saved_document() {
    let mut s = session();
    let inks = inks(&s);
    let path = run(&mut s, "shape.line", json!({"x1": 10, "y1": 10, "x2": 150, "y2": 60}))["id"].as_u64().unwrap();
    let (defaults, before) = (brushes(&s), undo_len(&s));
    let names = ["Tapered Ink", "Brush Pen", "Soft Wash", "Pooling Wash"];
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "ink-and-wash", "name": "Soft Wash", "apply": true, "ids": [path]}));
    assert_eq!((strs(&r["added"]), r["applied"].clone(), undo_len(&s)), (vec!["Soft Wash"], json!([path]), before + 1));
    let stroke_brush = |s: &Session| doc(s).node(NodeId(path)).unwrap().appearance.stroke().and_then(|st| st.brush.clone());
    assert_eq!(stroke_brush(&s).as_deref(), Some("Soft Wash"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((brushes(&s), stroke_brush(&s)), (defaults.clone(), None));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(stroke_brush(&s).as_deref(), Some("Soft Wash"));
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "ink-and-wash"}));
    assert_eq!(
        (strs(&r["names"]), strs(&r["existing"]), strs(&r["added"])),
        (names.to_vec(), vec!["Soft Wash"], vec!["Tapered Ink", "Brush Pen", "Pooling Wash"])
    );
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "ink-and-wash"}));
    assert_eq!((strs(&r["existing"]), undo_len(&s)), (names.to_vec(), before));
    assert_eq!(brushes(&s).len(), defaults.len() + 4);
    for n in names {
        assert_eq!(&brush(&s, n), inks.brush(n).unwrap(), "{n}");
    }

    // Saved and opened again, the document keeps the definitions and the brushed stroke.
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let reopened = vectorcraft_format::load(&vectorcraft_format::base64_decode(&b64).unwrap()).unwrap();
    for n in names {
        assert_eq!(vectorcraft_brush::find(&reopened, n).as_ref(), inks.brush(n), "{n}");
    }
    let n = reopened.node(NodeId(path)).unwrap();
    assert_eq!(n.appearance.stroke().and_then(|st| st.brush.clone()).as_deref(), Some("Soft Wash"));
    assert_eq!(
        vectorcraft_brush::node_pieces(&reopened, n).map(|p| p.len()),
        vectorcraft_brush::node_pieces(doc(&s), doc(&s).node(NodeId(path)).unwrap()).map(|p| p.len())
    );
}

/// Whether `p` lies inside the polygon through `pts` (even-odd ray cast).
fn inside(p: vectorcraft_geom::Point, pts: &[vectorcraft_geom::Point]) -> bool {
    let mut odd = false;
    for (i, a) in pts.iter().enumerate() {
        let b = pts[(i + 1) % pts.len()];
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
            odd = !odd;
        }
    }
    odd
}

/// Every anchor of the paths in `nodes` and their descendants.
fn anchors(nodes: &[&vectorcraft_doc::Node]) -> Vec<vectorcraft_geom::Point> {
    let mut out = vec![];
    for n in nodes {
        n.walk(&mut |c| out.extend(c.path_data().into_iter().flat_map(|p| p.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)))));
    }
    out
}

/// Dry Media: four art brushes of grainy dry marks (chalk, pencil, crayon, charcoal) with names
/// of their own. Each one's art is a bounded set of filled vector paths 100 pt long that takes the
/// stroke color as tints, with real even-odd gaps in the chalk and the crayon; it draws finite art
/// on curved and short paths, is the same every time it is made, and goes into a document in one
/// undo step, once, applied, and through a saved library unchanged.
#[test]
fn the_dry_media_library_holds_grainy_art_brushes_with_bounded_art() {
    use vectorcraft_brush::{ArtScale, Colorization};
    use vectorcraft_geom::FillRule;
    let mut s = session();
    let (info, lib) = cmd::brushlib::library(&s, "dry-media").unwrap();
    assert_eq!((info.name.as_str(), info.category, lib.name.as_str()), ("Dry Media", "builtIn", "Dry Media"));
    let names: Vec<String> = lib.brushes.iter().map(|b| b.name.clone()).collect();
    assert_eq!(names, ["Chalk Grain", "Graphite Pencil", "Wax Crayon", "Charcoal Dust"]);
    let mut total = 0;
    for (b, (height, gaps)) in lib.brushes.iter().zip([(8.0, true), (2.5, false), (6.0, true), (7.0, false)]) {
        let BrushKind::Art(a) = &b.kind else { panic!("{} is not an art brush", b.name) };
        assert_eq!((a.colorization, a.scale, a.width), (Colorization::Tints, ArtScale::Stretch, 100.0), "{}", b.name);
        let (mut paths, mut holes) = (0, 0);
        a.art.walk(&mut |n| match &n.kind {
            NodeKind::Path { path, .. } => {
                paths += 1;
                assert!(path.subpaths.iter().all(|sp| sp.closed && sp.anchors.len() >= 4), "{}", b.name);
            }
            NodeKind::Compound { children, rule } => {
                assert_eq!(*rule, FillRule::EvenOdd, "{}", b.name);
                // The gaps lie inside the band and apart from each other, so each one is a hole.
                let body = anchors(&[&*children[0]]);
                let boxes: Vec<vectorcraft_geom::Rect> = children[1..].iter().map(|c| c.path_data().unwrap().bounds().unwrap()).collect();
                for (i, c) in children[1..].iter().enumerate() {
                    assert!(anchors(&[&**c]).iter().all(|p| inside(*p, &body)), "{}: gap {i} leaves the band", b.name);
                    let r = boxes[i];
                    assert!(boxes[..i].iter().all(|o| r.x0 >= o.x1 || o.x0 >= r.x1 || r.y0 >= o.y1 || o.y0 >= r.y1), "{}: gap {i} overlaps", b.name);
                }
                holes += children.len() - 1;
            }
            NodeKind::Group { .. } => {}
            other => panic!("{}: unexpected {other:?}", b.name),
        });
        let pts = anchors(&[&a.art]);
        assert!(pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        assert!(paths <= 80 && pts.len() <= 500, "{}: {paths} paths, {} anchors", b.name, pts.len());
        assert_eq!(holes > 0, gaps, "{}: {holes} gaps", b.name);
        let r = vectorcraft_geom::Rect::from_points(pts[0], pts[0]);
        let r = pts.iter().fold(r, |r, p| r.union_pt(*p));
        assert!(r.x0 >= 0.0 && r.x0 < 0.5 && r.x1 <= 100.0 && r.x1 > 99.5, "{}: {r:?}", b.name);
        assert!((r.height() - height).abs() <= height * 0.25 && r.y0 < 0.0 && r.y1 > 0.0, "{}: {r:?}", b.name);
        total += pts.len();
        // Finite art on a curve and on a 2 pt path.
        for (line, width) in [(vec![(0.0, 0.0), (60.0, -40.0), (140.0, 40.0), (200.0, 0.0)], 1.0), (vec![(0.0, 0.0), (2.0, 0.0)], 3.0)] {
            let mut bp = vectorcraft_geom::BezPath::new();
            bp.move_to(line[0]);
            if line.len() == 4 {
                bp.curve_to(line[1], line[2], line[3]);
            } else {
                bp.line_to(line[1]);
            }
            let stroke = vectorcraft_doc::StrokeLayer::new(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), width);
            let pieces = vectorcraft_brush::stroke_pieces(&lib.doc, b, &bp, &stroke);
            let drawn = anchors(&pieces.iter().collect::<Vec<_>>());
            assert!(!drawn.is_empty() && drawn.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{} on {line:?}", b.name);
        }
    }
    assert!(total <= 1200, "{total} anchors");
    // Made again, the same bytes; the checksum pins them across runs and platforms.
    let bytes = serde_json::to_string(&cmd::brushlib::dry_media().brushes).unwrap();
    assert_eq!(bytes, serde_json::to_string(&lib.brushes).unwrap());
    let fnv = bytes.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    assert_eq!(fnv, 12_859_086_488_513_909_464, "the Dry Media art changed");
    // Names of their own: no default brush's, no other built-in library's.
    let defaults: Vec<&str> = vectorcraft_brush::defaults().iter().map(|b| b.name.as_str()).collect();
    let others: Vec<String> = cmd::brushlib::BUILTIN
        .iter()
        .filter(|b| b.id != "dry-media")
        .flat_map(|b| cmd::brushlib::library(&s, b.id).unwrap().1.brushes.iter().map(|x| x.name.clone()).collect::<Vec<_>>())
        .collect();
    assert!(names.iter().all(|n| !defaults.contains(&n.as_str()) && !others.contains(n)), "{names:?}");

    // Added and applied to a path in one undo step, which undo takes back.
    let path = run(&mut s, "shape.line", json!({"x1": 10, "y1": 10, "x2": 150, "y2": 60}))["id"].as_u64().unwrap();
    let stroke_brush = |s: &Session| doc(s).node(NodeId(path)).unwrap().appearance.stroke().and_then(|st| st.brush.clone());
    let before = (undo_len(&s), brushes(&s));
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Dry Media", "name": "Wax Crayon", "apply": true, "ids": [path]}));
    assert_eq!((strs(&r["added"]), r["applied"].clone()), (vec!["Wax Crayon"], json!([path])));
    assert_eq!((undo_len(&s), stroke_brush(&s).as_deref()), (before.0 + 1, Some("Wax Crayon")));
    assert_eq!(&brush(&s, "Wax Crayon"), lib.brush("Wax Crayon").unwrap());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((brushes(&s), stroke_brush(&s)), (before.1.clone(), None));
    run(&mut s, "edit.redo", json!({}));
    // The rest in one more step; then nothing new to add.
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "dry-media"}));
    assert_eq!((strs(&r["added"]), strs(&r["existing"])), (vec!["Chalk Grain", "Graphite Pencil", "Charcoal Dust"], vec!["Wax Crayon"]));
    assert_eq!(undo_len(&s), before.0 + 2);
    assert_eq!(
        brushes(&s),
        [before.1.clone(), vec!["Wax Crayon".into(), "Chalk Grain".into(), "Graphite Pencil".into(), "Charcoal Dust".into()]].concat()
    );
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "dry-media"}));
    assert_eq!((strs(&r["added"]).len(), undo_len(&s)), (0, before.0 + 2));
    // Saved as a library and loaded back: the same brushes, in the document's order.
    let saved = run(&mut s, "brush.saveLibrary", json!({ "names": names, "name": "Dry" }));
    let b64 = saved["dataBase64"].as_str().unwrap().to_string();
    let r = run(&mut s, "brush.loadLibrary", json!({"dataBase64": b64, "name": "Dry.vectorcraft"}));
    let (_, reopened) = cmd::brushlib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert_eq!(reopened.len(), lib.len());
    for b in &lib.brushes {
        assert_eq!(reopened.brush(&b.name), Some(b), "{}", b.name);
    }
}

/// Bristle Brushes: one brush per tip shape, every value inside the range the brush keeps (a value
/// outside would be clamped without notice), names no other library or default brush uses; added
/// in one step, not twice, and saved as a library they reopen unchanged.
#[test]
fn the_bristle_brushes_library_covers_every_tip_shape_within_the_bristle_ranges() {
    use vectorcraft_brush::{Bristle, BristleShape};
    let mut s = session();
    let (_, lib) = cmd::brushlib::library(&s, "bristle-brushes").unwrap();
    assert_eq!(lib.name, "Bristle Brushes");
    let names: Vec<String> = lib.brushes.iter().map(|b| b.name.clone()).collect();
    assert!((8..=10).contains(&names.len()), "{names:?}");
    let within = |v: f64, (lo, hi): (f64, f64)| (lo..=hi).contains(&v);
    let mut shapes = vec![];
    for b in &lib.brushes {
        let BrushKind::Bristle(x) = &b.kind else { panic!("{} is not a bristle brush", b.name) };
        assert!(within(x.size, Bristle::SIZE_RANGE) && within(x.length, Bristle::LENGTH_RANGE), "{}", b.name);
        for v in [x.density, x.thickness, x.opacity, x.stiffness] {
            assert!(within(v, Bristle::PERCENT_RANGE), "{}: {v}", b.name);
        }
        let mut kept = b.clone();
        kept.sanitize();
        assert_eq!(&kept, b, "sanitizing changes {}", b.name);
        assert!(b.name.chars().count() <= Bristle::MAX_NAME_CHARS, "{}", b.name);
        if !shapes.contains(&x.shape) {
            shapes.push(x.shape);
        }
    }
    let every = [
        BristleShape::RoundPoint,
        BristleShape::FlatPoint,
        BristleShape::RoundBlunt,
        BristleShape::FlatBlunt,
        BristleShape::RoundCurve,
        BristleShape::FlatCurve,
        BristleShape::RoundAngle,
        BristleShape::FlatAngle,
        BristleShape::RoundFan,
        BristleShape::FlatFan,
    ];
    assert!(every.iter().all(|x| shapes.contains(x)), "{shapes:?}");
    // Names of its own: no default brush's, no other built-in library's, none twice.
    let defaults: Vec<&str> = vectorcraft_brush::defaults().iter().map(|b| b.name.as_str()).collect();
    let others: Vec<String> = cmd::brushlib::BUILTIN
        .iter()
        .filter(|b| b.id != "bristle-brushes")
        .flat_map(|b| cmd::brushlib::library(&s, b.id).unwrap().1.brushes.iter().map(|x| x.name.clone()).collect::<Vec<_>>())
        .collect();
    for (i, n) in names.iter().enumerate() {
        assert!(!defaults.contains(&n.as_str()) && !others.contains(n) && !names[..i].contains(n), "{n}");
    }

    // Added to a document in one undo step; adding them again adds nothing.
    let before = (undo_len(&s), brushes(&s));
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Bristle Brushes"}));
    assert_eq!(strs(&r["added"]), names.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(undo_len(&s), before.0 + 1);
    assert_eq!(brushes(&s), [before.1.clone(), names.clone()].concat());
    for b in &lib.brushes {
        assert_eq!(&brush(&s, &b.name), b);
    }
    let again = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "bristle-brushes"}));
    assert_eq!((strs(&r["added"]).len(), strs(&r["existing"]).len(), undo_len(&s)), (0, names.len(), again));
    // Saved as a library and loaded back: the same brushes, in the same order.
    let saved = run(&mut s, "brush.saveLibrary", json!({ "names": names, "name": "Bristles" }));
    let b64 = saved["dataBase64"].as_str().unwrap().to_string();
    let r = run(&mut s, "brush.loadLibrary", json!({"dataBase64": b64, "name": "Bristles.vectorcraft"}));
    let (_, reopened) = cmd::brushlib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert_eq!(reopened.brushes, lib.brushes);
}

#[test]
fn a_saved_library_reopens_and_its_brushes_join_another_document_in_one_step() {
    let dir = folder("user");
    let mut s = kit();
    s.brush_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let r = run(&mut s, "brush.saveLibrary", json!({"user": true, "name": "Inks", "names": ["My Nib", "Dotted Band"]}));
    assert_eq!((r["library"].as_str(), r["count"].as_u64()), (Some("user/Inks.vectorcraft"), Some(2)));
    let path = r["path"].as_str().unwrap().to_string();
    let list = run(&mut s, "brush.libraries", json!({}));
    let user: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "user").collect();
    assert_eq!(user.len(), 1);
    assert_eq!((user[0]["name"].as_str(), user[0]["count"].as_u64()), (Some("Inks"), Some(2)));
    // The file is a document: the two brushes and the pattern their art uses, no art.
    let saved = vectorcraft_format::load(&std::fs::read(&path).unwrap()).unwrap();
    let names: Vec<String> = vectorcraft_brush::library(&saved).into_iter().map(|b| b.name).collect();
    assert_eq!(names, ["My Nib", "Dotted Band"]);
    assert_eq!(saved.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Dots"]);
    assert!(saved.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())));
    // Loading the file gives that User Defined library.
    let r = run(&mut s, "brush.loadLibrary", json!({ "path": path }));
    assert_eq!((r["library"].as_str(), r["name"].as_str()), (Some("user/Inks.vectorcraft"), Some("Inks")));

    // Into a new document: Dotted Band and its pattern, as one step.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let defaults = brushes(&s);
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Inks", "name": "Dotted Band"}));
    assert_eq!((strs(&r["names"]), strs(&r["added"]), strs(&r["existing"])), (vec!["Dotted Band"], vec!["Dotted Band"], vec![]));
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Add to Brushes");
    assert_eq!(brushes(&s), [defaults.clone(), vec!["Dotted Band".to_string()]].concat());
    let (_, lib) = cmd::brushlib::library(&s, "Inks").unwrap();
    assert_eq!(&brush(&s, "Dotted Band"), lib.brush("Dotted Band").unwrap());
    assert!(doc(&s).pattern("Dots").is_some() && doc(&s).swatch("Dots").is_some(), "the pattern its art fills with");
    assert_eq!(run(&mut s, "brush.list", json!({}))["current"], "Dotted Band");
    // Undo takes it all back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(brushes(&s), defaults);
    assert!(doc(&s).pattern("Dots").is_none());
    run(&mut s, "edit.redo", json!({}));
    // Again: already there, so no step and no copy.
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "user/Inks.vectorcraft", "names": ["Dotted Band", "My Nib", "Dotted Band"]}));
    assert_eq!((strs(&r["names"]), strs(&r["existing"]), strs(&r["added"])), (vec!["Dotted Band", "My Nib"], vec!["Dotted Band"], vec!["My Nib"]));
    assert_eq!(undo_len(&s), before + 1, "My Nib was new");
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Inks"}));
    assert_eq!((strs(&r["existing"]).len(), undo_len(&s)), (2, before));
    assert_eq!(brushes(&s).len(), defaults.len() + 2);
    assert_eq!(doc(&s).patterns.len(), 1);
    assert_eq!(run(&mut s, "brush.list", json!({}))["current"], "My Nib", "the library's first brush");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn library_brushes_bring_their_patterns_and_symbols_rename_clashes_and_apply() {
    let mut s = session();
    // The document has a pattern "Pinstripes" and a brush "Wave" of its own.
    add_pattern(&mut s, "Pinstripes", "#00ff00");
    let mine = doc(&s).pattern("Pinstripes").unwrap().clone();
    run(&mut s, "brush.new", json!({"type": "calligraphic", "name": "Wave"}));
    let my_wave = brush(&s, "Wave");
    let path = run(&mut s, "shape.line", json!({"x1": 10, "y1": 10, "x2": 150, "y2": 60}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [path]}));
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "names": ["Striped Ribbon", "Wave"], "apply": true}));
    assert_eq!(undo_len(&s), before + 1, "adding and applying are one step");
    assert_eq!(strs(&r["names"]), ["Striped Ribbon", "Wave 2"]);
    assert_eq!(r["applied"], json!([path]));
    assert_eq!(brush(&s, "Wave"), my_wave, "the document's own Wave stays");
    // The library's Pinstripes comes in under a free name, and the brush's art follows it.
    assert_eq!(doc(&s).pattern("Pinstripes").unwrap(), &mine);
    assert!(doc(&s).pattern("Pinstripes 2").is_some() && doc(&s).swatch("Pinstripes 2").is_some());
    assert_eq!(art_patterns(&brush(&s, "Striped Ribbon")), ["Pinstripes 2"]);
    // The first one is applied to the selected path and is the current brush.
    let n = doc(&s).node(NodeId(path)).unwrap();
    assert_eq!(n.appearance.stroke().and_then(|st| st.brush.clone()).as_deref(), Some("Striped Ribbon"));
    assert_eq!(run(&mut s, "brush.list", json!({}))["current"], "Striped Ribbon");
    // Adding it again finds it, its pattern renamed, and adds nothing.
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "names": ["Striped Ribbon", "Wave"]}));
    assert_eq!((strs(&r["names"]), strs(&r["existing"])), (vec!["Striped Ribbon", "Wave 2"], vec!["Striped Ribbon", "Wave 2"]));
    assert_eq!(undo_len(&s), before);
    assert!(doc(&s).pattern("Pinstripes 3").is_none() && vectorcraft_brush::find(doc(&s), "Striped Ribbon 2").is_none());
    // Undo of the first add takes the brushes, the pattern and the brush stroke back.
    run(&mut s, "edit.undo", json!({}));
    assert!(vectorcraft_brush::find(doc(&s), "Striped Ribbon").is_none() && doc(&s).pattern("Pinstripes 2").is_none());
    assert!(doc(&s).node(NodeId(path)).unwrap().appearance.stroke().is_none_or(|st| st.brush.is_none()));
    // A scatter brush brings the symbol it scatters.
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Scatter Shapes", "name": "Sparkles"}));
    assert_eq!(strs(&r["added"]), ["Sparkles"]);
    assert_eq!(doc(&s).symbols.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Sparkle"]);
    // Applying a brush the document has still applies (one step) to the paths given, without
    // adding it again.
    run(&mut s, "select.none", json!({}));
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "scatter-shapes", "name": "Sparkles", "apply": true, "ids": [path]}));
    assert_eq!((undo_len(&s), strs(&r["existing"]), r["applied"].clone()), (before + 1, vec!["Sparkles"], json!([path])));
    assert_eq!(doc(&s).node(NodeId(path)).unwrap().appearance.stroke().and_then(|st| st.brush.clone()).as_deref(), Some("Sparkles"));
    assert_eq!(doc(&s).symbols.len(), 1);
    assert!(s.execute("brush.addFromLibrary", &json!({"library": "art-ribbons", "name": "Nope"})).is_err());
    assert!(s.execute("brush.addFromLibrary", &json!({"name": "Wave"})).is_err());
}

/// The undo step is Add to Brushes when a brush comes in (applied or not), and Apply Brush when
/// the brushes are all there and the click only applies one; undo and redo of either restore the
/// document as it was and as the step left it.
#[test]
fn adding_library_brushes_is_add_to_brushes_and_only_applying_one_is_apply_brush() {
    let mut s = session();
    let path = run(&mut s, "shape.line", json!({"x1": 10, "y1": 10, "x2": 150, "y2": 60}))["id"].as_u64().unwrap();
    let label = |s: &Session| s.doc().unwrap().history.undo.last().unwrap().label.clone();
    let stroke_brush = |s: &Session| doc(s).node(NodeId(path)).unwrap().appearance.stroke().and_then(|st| st.brush.clone());
    run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Wave", "apply": true, "ids": [path]}));
    assert_eq!((label(&s), stroke_brush(&s).as_deref()), ("Add to Brushes".to_string(), Some("Wave")));
    run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Teardrop"}));
    assert_eq!(label(&s), "Add to Brushes");
    let (before, brushes_before) = (undo_len(&s), brushes(&s));
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Teardrop", "apply": true, "ids": [path]}));
    assert_eq!((strs(&r["added"]), strs(&r["existing"])), (vec![], vec!["Teardrop"]));
    assert_eq!((undo_len(&s), label(&s), brushes(&s)), (before + 1, "Apply Brush".to_string(), brushes_before.clone()));
    assert_eq!(stroke_brush(&s).as_deref(), Some("Teardrop"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((stroke_brush(&s).as_deref(), brushes(&s)), (Some("Wave"), brushes_before.clone()));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!((stroke_brush(&s).as_deref(), label(&s)), (Some("Teardrop"), "Apply Brush".to_string()));
}

#[test]
fn pasted_brushed_art_brings_the_patterns_its_brush_art_uses() {
    let mut s = kit();
    let line = run(&mut s, "shape.line", json!({"x1": 10, "y1": 100, "x2": 150, "y2": 120}))["id"].as_u64().unwrap();
    run(&mut s, "brush.apply", json!({"name": "Dotted Band", "ids": [line]}));
    run(&mut s, "select.set", json!({"ids": [line]}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    run(&mut s, "edit.paste", json!({}));
    assert!(vectorcraft_brush::find(doc(&s), "Dotted Band").is_some());
    assert!(doc(&s).pattern("Dots").is_some(), "the brush's art is filled with it");
}

#[test]
fn other_documents_brushes_load_and_unreadable_files_are_errors() {
    let mut s = kit();
    // Another document's brushes load as a library, without its art.
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let count = brushes(&s).len();
    let r = run(&mut s, "brush.loadLibrary", json!({"dataBase64": b64, "name": "Poster.vectorcraft"}));
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Poster"), Some(count as u64)));
    let (_, lib) = cmd::brushlib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert!(lib.doc.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())));
    assert!(lib.brush("Dotted Band").is_some() && lib.doc.pattern("Dots").is_some());
    // An SVG has the default brushes, as it does opened.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="10" height="10"/></svg>"##;
    let r = run(&mut s, "brush.loadLibrary", json!({"data": svg, "name": "Box.svg"}));
    assert_eq!(r["count"].as_u64(), Some(vectorcraft_brush::defaults().len() as u64), "{r}");
    // Files that aren't documents, missing files and damaged native files are errors.
    assert!(s.execute("brush.loadLibrary", &json!({"data": "hello", "name": "x.txt"})).is_err());
    assert!(s.execute("brush.loadLibrary", &json!({"path": "/no/such/Library.vectorcraft"})).is_err());
    let dir = folder("bad");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("Broken.vectorcraft");
    std::fs::write(&bad, b"{\"format\": \"vectorcraft\", \"doc\": 7").unwrap();
    assert!(s.execute("brush.loadLibrary", &json!({"path": bad.to_string_lossy()})).is_err());
    // A damaged file in the user folder isn't listed.
    s.brush_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let list = run(&mut s, "brush.libraries", json!({}));
    assert!(list["libraries"].as_array().unwrap().iter().all(|l| l["category"] != "user"));
    s.brush_libraries.set_user_dir(None);
    let _ = std::fs::remove_dir_all(dir);
    // No such brush, or a headless session's user folder, is an error.
    assert!(s.execute("brush.saveLibrary", &json!({"names": ["Nope"]})).is_err());
    assert!(s.execute("brush.saveLibrary", &json!({"user": true})).is_err());
    // A document whose brushes were all deleted has none to save.
    let all = brushes(&s);
    run(&mut s, "brush.delete", json!({ "names": all }));
    assert!(s.execute("brush.saveLibrary", &json!({})).is_err());
    // Without a path the library comes back as the file's bytes.
    let mut s = kit();
    let r = run(&mut s, "brush.saveLibrary", json!({"names": ["My Nib"]}));
    let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(vectorcraft_brush::library(&vectorcraft_format::load(&bytes).unwrap()).len(), 1);
}

/// A library file whose brushes repeat a name loads with the later brush numbered, and each one
/// can be added to a document by its own name.
#[test]
fn a_library_file_that_repeats_a_brush_name_loads_with_the_later_one_numbered() {
    use vectorcraft_brush::{Brush, Calligraphic};
    let nib = |size: f64| Brush { name: "Nib".into(), kind: BrushKind::Calligraphic(Calligraphic { size, ..Default::default() }) };
    let mut file = vectorcraft_doc::Document::new(100.0, 100.0);
    vectorcraft_brush::store(&mut file, &[nib(1.0), nib(2.0)]);
    let b64 = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&file));
    let mut s = session();
    let r = run(&mut s, "brush.loadLibrary", json!({"dataBase64": b64, "name": "Nibs.vectorcraft"}));
    let library = r["library"].as_str().unwrap().to_string();
    let listed = run(&mut s, "brush.library", json!({ "library": library }));
    let names: Vec<&str> = listed["brushes"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Nib", "Nib 2"]);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": library, "name": "Nib 2"}));
    assert_eq!(strs(&r["added"]), ["Nib 2"]);
    let size = |b: Brush| if let BrushKind::Calligraphic(c) = b.kind { c.size } else { 0.0 };
    assert_eq!(size(brush(&s, "Nib 2")), 2.0);
}
