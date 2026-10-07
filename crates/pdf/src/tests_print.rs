//! File → Print as a PDF: paper, scale, placement, tiles, artboard ranges, copies, layers,
//! separations, marks and the page turns.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{Appearance, Artboard, Document, LayerColor, Node, NodeKind};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_testkit::raster::render_region;

use crate::*;

/// A `w` × `h` document with filled rectangles `(rect, paint)` on its one layer.
fn doc(w: f64, h: f64, shapes_: &[(Rect, Paint)]) -> Document {
    let mut d = Document::new(w, h);
    let layer = d.default_layer().unwrap();
    for (r, p) in shapes_ {
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::path(id, shapes::rectangle(*r), Appearance::basic(p.clone(), Paint::None, 0.0))).unwrap();
    }
    d
}

fn artboard(id: u32, x: f64) -> Artboard {
    Artboard {
        id,
        name: format!("Artboard {id}"),
        rect: Rect::new(x, 0.0, x + 100.0, 100.0),
        show_center_mark: false,
        show_cross_hairs: false,
        ..Default::default()
    }
}

fn rgb(r: f32, g: f32, b: f32) -> Paint {
    Paint::solid(Color::rgb(r, g, b))
}

fn settings(v: serde_json::Value) -> PrintSettings {
    serde_json::from_value(v).unwrap()
}

fn job(d: &Document, v: serde_json::Value) -> PrintReport {
    let opts = PrintOptions { settings: settings(v), created: Some(1_791_200_000), uncompressed: true, ..Default::default() };
    print(d, &opts).unwrap()
}

/// A printed page read back: its size and every filled path on it (bounds in page space, y
/// down; paint), left to right.
type Page = ((f64, f64), Vec<(Rect, Paint)>);

fn pages(bytes: &[u8]) -> Vec<Page> {
    let back = import(bytes).unwrap();
    back.artboards
        .iter()
        .map(|ab| {
            let r = ab.rect;
            let mut paths = vec![];
            back.walk(|n| {
                if let (Some(b), Some(f)) = (n.path_data().and_then(|p| p.bounds()), n.appearance.fill())
                    && f.paint.color().is_some()
                    && b.intersect(r).area() > 0.0
                {
                    paths.push((b - r.origin().to_vec2(), f.paint.clone()));
                }
            });
            paths.sort_by(|a, b| a.0.x0.total_cmp(&b.0.x0));
            ((r.width(), r.height()), paths)
        })
        .collect()
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 0.05)
}

fn rgb8(p: &Paint) -> [u8; 3] {
    let c = p.color().unwrap().to_rgba8(1.0);
    [c[0], c[1], c[2]]
}

#[test]
fn defaults_and_json() {
    let s = PrintSettings::default();
    assert_eq!(
        (s.copies, s.collate, s.media, s.scaling, s.print_layers),
        (1, true, Media::Letter, PrintScaling::None, PrintLayers::VisiblePrintable)
    );
    assert!(s.auto_rotate && s.bleed.use_document && s.check().is_ok());
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["printLayers"], "visiblePrintable");
    assert_eq!(v["output"]["mode"], "composite");
    assert_eq!(serde_json::from_value::<PrintSettings>(v).unwrap(), s);
    // Every field is optional.
    let s = settings(json!({"scaling": "tileFull", "placement": {"origin": "topLeft"}, "color": {"intent": "perceptual"}}));
    assert_eq!((s.scaling, s.placement.origin, s.copies), (PrintScaling::TileFull, Origin::TopLeft, 1));
    for bad in [json!({"copies": 0}), json!({"scale": {"width": 0}}), json!({"media": "custom", "width": 10}), json!({"marks": {"weight": 9}})] {
        assert!(matches!(settings(bad.clone()).check(), Err(PdfError::BadSetting(_))), "{bad}");
    }
    assert_eq!(Media::A4.size().map(|(w, h)| (w.round(), h.round())), Some((595.0, 842.0)));
}

#[test]
fn fit_to_letter_scales_the_artboard_into_the_paper() {
    // Twice Letter, landscape, a red square in its top-left quarter.
    let d = doc(1584.0, 1224.0, &[(Rect::new(0.0, 0.0, 792.0, 612.0), rgb(1.0, 0.0, 0.0))]);
    let r = job(&d, json!({"scaling": "fit"}));
    assert_eq!(r.pages, 1);
    let p = pages(&r.bytes);
    assert_eq!(p[0].0, (792.0, 612.0), "auto-rotate turns Letter landscape");
    assert!(near(p[0].1[0].0, Rect::new(0.0, 0.0, 396.0, 306.0)), "half size from the corner: {:?}", p[0].1);
    // Without auto-rotate a portrait Letter fits it smaller, centred.
    let r = job(&d, json!({"scaling": "fit", "autoRotate": false}));
    let p = pages(&r.bytes);
    assert_eq!(p[0].0, (612.0, 792.0));
    let s = 612.0 / 1584.0;
    let top = (792.0 - 1224.0 * s) / 2.0;
    assert!(near(p[0].1[0].0, Rect::new(0.0, top, 792.0 * s, top + 612.0 * s)), "{:?}", p[0].1);
    let pv = preview(&d, &settings(json!({"scaling": "fit", "autoRotate": false}))).unwrap();
    assert!((pv.sheets[0].scale[0] - s * 100.0).abs() < 1e-9 && pv.sheets[0].orientation == Orientation::Portrait);
    // With marks the artboard shrinks to leave them room on the paper.
    let pv = preview(&d, &settings(json!({"scaling": "fit", "marks": {"trim": true}}))).unwrap();
    assert!(pv.sheets[0].scale[0] < 50.0 && pv.warnings.is_empty(), "{pv:?}");
}

#[test]
fn placement_and_custom_scale() {
    let d = doc(100.0, 100.0, &[(Rect::new(0.0, 0.0, 100.0, 100.0), rgb(0.0, 0.0, 1.0))]);
    let at = |v| pages(&job(&d, v).bytes)[0].1[0].0;
    assert!(near(at(json!({})), Rect::new(256.0, 346.0, 356.0, 446.0)), "centred");
    assert!(near(at(json!({"placement": {"origin": "topLeft", "x": 10, "y": 20}})), Rect::new(10.0, 20.0, 110.0, 120.0)));
    assert!(near(at(json!({"placement": {"origin": "bottomRight"}, "margin": 18})), Rect::new(494.0, 674.0, 594.0, 774.0)), "inside the margin");
    assert!(near(
        at(json!({"placement": {"origin": "topLeft"}, "scaling": "custom", "scale": {"width": 200, "height": 50}})),
        Rect::new(0.0, 0.0, 200.0, 50.0)
    ));
    // Art larger than the paper is cut off, with a warning.
    let big = doc(1000.0, 100.0, &[]);
    assert!(job(&big, json!({"autoRotate": false})).warnings.iter().any(|w| w.contains("larger than the imageable area")));
}

#[test]
fn tiles_cover_the_artboard_with_overlap() {
    // 1000 × 700 on Letter tiles (612 × 792), 36 pt overlap: 2 × 1.
    let d = doc(1000.0, 700.0, &[(Rect::new(560.0, 100.0, 600.0, 140.0), rgb(0.0, 0.6, 0.0))]);
    let set = json!({"scaling": "tileFull", "overlap": 36, "autoRotate": false});
    let pv = preview(&d, &settings(set.clone())).unwrap();
    assert_eq!((pv.pages, pv.tiles[0].columns, pv.tiles[0].rows), (2, 2, 1));
    assert_eq!(pv.tiles[0].tiles[1][0], 576.0, "the second tile starts a tile less the overlap along");
    let p = pages(&job(&d, set).bytes);
    assert_eq!(p.len(), 2);
    // The square lies in the overlap: on both pages, where each tile shows it.
    assert!(near(p[0].1[0].0, Rect::new(560.0, 100.0, 600.0, 140.0)), "{:?}", p[0].1);
    assert!(near(p[1].1[0].0, Rect::new(-16.0, 100.0, 24.0, 140.0)), "{:?}", p[1].1);
    // 1100 × 1400: 2 × 2, and a tile range.
    let d = doc(1100.0, 1400.0, &[]);
    let pv = preview(&d, &settings(json!({"scaling": "tileFull", "overlap": 36, "autoRotate": false}))).unwrap();
    assert_eq!((pv.pages, pv.tiles[0].columns, pv.tiles[0].rows), (4, 2, 2));
    let pv = preview(&d, &settings(json!({"scaling": "tileFull", "tileRange": "2-3", "autoRotate": false}))).unwrap();
    assert_eq!((pv.pages, pv.tiles[0].printed.clone()), (2, vec![2, 3]));
    assert_eq!(pv.sheets.iter().map(|s| s.tile).collect::<Vec<_>>(), [Some(2), Some(3)]);
    assert!(preview(&d, &settings(json!({"scaling": "tileFull", "tileRange": "9"}))).is_err());
    // Imageable-area tiles are smaller (inside the margin).
    let pv = preview(&d, &settings(json!({"scaling": "tileImageable", "margin": 72, "autoRotate": false}))).unwrap();
    assert_eq!((pv.tiles[0].columns, pv.tiles[0].rows), (3, 3));
    assert!(preview(&d, &settings(json!({"scaling": "tileFull", "overlap": 400}))).is_err(), "overlap of half a tile");
}

#[test]
fn artboard_range_skip_blank_and_ignore() {
    let mut d =
        doc(100.0, 100.0, &[(Rect::new(10.0, 10.0, 20.0, 20.0), rgb(1.0, 0.0, 0.0)), (Rect::new(410.0, 10.0, 420.0, 20.0), rgb(0.0, 0.0, 1.0))]);
    d.artboards.extend([artboard(2, 200.0), artboard(3, 400.0)]);
    assert_eq!(job(&d, json!({})).pages, 3);
    let p = pages(&job(&d, json!({"artboards": "range", "range": "3, 1"})).bytes);
    assert_eq!(p.len(), 2);
    assert_eq!(rgb8(&p[0].1[0].1), [0, 0, 255], "in the range's order");
    assert_eq!(rgb8(&p[1].1[0].1), [255, 0, 0]);
    assert!(preview(&d, &settings(json!({"artboards": "range", "range": "4"}))).is_err());
    let pv = preview(&d, &settings(json!({"skipBlank": true}))).unwrap();
    assert_eq!(pv.sheets.iter().map(|s| s.artboard).collect::<Vec<_>>(), [Some(0), Some(2)], "the middle artboard is blank");
    // Ignoring artboards: all the art on one page.
    let pv = preview(&d, &settings(json!({"artboards": "ignore"}))).unwrap();
    assert_eq!((pv.pages, pv.sheets[0].artboard), (1, None));
    assert!(preview(&doc(100.0, 100.0, &[]), &settings(json!({"artboards": "ignore"}))).is_err(), "no art");
}

#[test]
fn copies_collate_and_reverse() {
    let mut d =
        doc(100.0, 100.0, &[(Rect::new(10.0, 10.0, 20.0, 20.0), rgb(1.0, 0.0, 0.0)), (Rect::new(210.0, 10.0, 220.0, 20.0), rgb(0.0, 0.0, 1.0))]);
    d.artboards.push(artboard(2, 200.0));
    let order = |v| pages(&job(&d, v).bytes).iter().map(|p| if rgb8(&p.1[0].1) == [255, 0, 0] { 'a' } else { 'b' }).collect::<String>();
    assert_eq!(order(json!({"copies": 2})), "abab");
    assert_eq!(order(json!({"copies": 2, "collate": false})), "aabb");
    assert_eq!(order(json!({"copies": 2, "reverse": true})), "baba");
    assert!(preview(&d, &settings(json!({"copies": 999}))).is_ok_and(|p| p.pages == 1998));
    assert!(preview(&d, &settings(json!({"copies": 999, "output": {"mode": "separations"}}))).is_err(), "too many pages");
}

#[test]
fn print_layers_and_template_layers() {
    let mut d = doc(200.0, 200.0, &[(Rect::new(0.0, 0.0, 10.0, 10.0), rgb(1.0, 0.0, 0.0))]);
    // A hidden layer, a non-printing one and a template, each with a blue square.
    for (i, name) in ["Hidden", "Notes", "Template"].into_iter().enumerate() {
        let mut layer = Node::layer(d.alloc_id(), name, LayerColor::Preset(1));
        let x = 20.0 + 20.0 * i as f64;
        let sq =
            Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::basic(rgb(0.0, 0.0, 1.0), Paint::None, 0.0));
        if let NodeKind::Layer { children, printable, template, .. } = &mut layer.kind {
            *printable = name != "Notes";
            *template = name == "Template";
            children.push(Arc::new(sq));
        }
        layer.visible = name != "Hidden";
        d.layers.push(Arc::new(layer));
    }
    let blues = |v| pages(&job(&d, v).bytes)[0].1.iter().filter(|(_, p)| rgb8(p) == [0, 0, 255]).count();
    assert_eq!(blues(json!({})), 0, "visible & printable: the hidden and non-printing layers are left out");
    assert_eq!(blues(json!({"printLayers": "visible"})), 1, "the non-printing layer");
    assert_eq!(blues(json!({"printLayers": "all"})), 2, "the hidden layer too, never the template");
}

#[test]
fn cmyk_and_a_spot_separate_into_five_plates() {
    let mut d = doc(200.0, 100.0, &[]);
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(Color::cmyk(0.0, 0.2, 0.8, 0.1)), global: true, spot: true });
    let layer = d.default_layer().unwrap();
    let cyan = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
    let gold = Paint::Solid { color: Color::cmyk(0.0, 0.1, 0.4, 0.05), swatch: Some("Gold".into()), tint: 0.5 };
    for (r, p) in [(Rect::new(0.0, 0.0, 50.0, 50.0), cyan), (Rect::new(100.0, 0.0, 150.0, 50.0), gold)] {
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::path(id, shapes::rectangle(r), Appearance::basic(p, Paint::None, 0.0))).unwrap();
    }
    let set = json!({"output": {"mode": "separations"}, "marks": {"trim": true}});
    let pv = preview(&d, &settings(set.clone())).unwrap();
    assert_eq!(pv.inks.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Cyan", "Magenta", "Yellow", "Black", "Gold"]);
    assert_eq!(pv.sheets.iter().filter_map(|s| s.ink.clone()).collect::<Vec<_>>(), ["Cyan", "Magenta", "Yellow", "Black", "Gold"]);
    assert_eq!((pv.inks[0].angle, pv.inks[3].angle, pv.inks[0].frequency), (15.0, 45.0, DEFAULT_FREQUENCY));
    let r = job(&d, set);
    assert_eq!(r.pages, 5);
    assert!(!String::from_utf8_lossy(&r.bytes).contains("/Separation"), "plates are grey");
    // The ink of the cyan and the gold square on a plate, in percent (0: none, a knockout).
    let inks = |page: &Page| -> Vec<f32> {
        page.1
            .iter()
            .filter(|(b, _)| (b.width() - 50.0).abs() < 1.0 && (b.height() - 50.0).abs() < 1.0)
            .map(|(_, p)| match p.color() {
                Some(Color::Gray { k }) => (k * 100.0).round(),
                c => panic!("{c:?}"),
            })
            .collect()
    };
    let p = pages(&r.bytes);
    assert_eq!(inks(&p[0]), [100.0, 0.0], "cyan plate");
    assert_eq!(inks(&p[1]), [0.0, 0.0], "magenta plate: both knock out");
    assert_eq!(inks(&p[4]), [0.0, 50.0], "the spot plate at the tint");
    // Trim marks print on every plate, in full ink.
    let back = import(&r.bytes).unwrap();
    let mut strokes = 0;
    back.walk(|n| strokes += usize::from(n.appearance.stroke().is_some_and(|s| matches!(s.paint.color(), Some(Color::Gray { k }) if k > 0.99))));
    assert_eq!(strokes, 8 * 5, "8 trim marks on each plate");
    // Spots as process: four plates, the gold in process inks; an ink can be left out.
    let pv = preview(&d, &settings(json!({"output": {"mode": "separations", "spotsToProcess": true, "inks": [{"name": "Black", "print": false}]}})))
        .unwrap();
    assert_eq!(pv.pages, 3);
    let p = pages(&job(&d, json!({"output": {"mode": "separations", "spotsToProcess": true}})).bytes);
    assert_eq!(p.len(), 4);
    assert_eq!(inks(&p[2]), [0.0, 40.0], "the gold's yellow");
}

#[test]
fn trim_marks_surround_the_artboard_on_the_paper() {
    let d = doc(200.0, 100.0, &[(Rect::new(0.0, 0.0, 200.0, 100.0), rgb(1.0, 0.0, 0.0))]);
    let r = job(&d, json!({"marks": {"trim": true}, "autoRotate": false}));
    let pdf = info(&r.bytes, None).unwrap();
    let trim = pdf.pages[0].boxes.iter().find(|(c, _)| *c == CropTo::Trim).unwrap().1;
    assert_eq!((trim.width().round(), trim.height().round()), (200.0, 100.0), "the TrimBox is the artboard on the paper");
    let back = import(&r.bytes).unwrap();
    let ab = Rect::new(206.0, 346.0, 406.0, 446.0) + back.artboards[0].rect.origin().to_vec2();
    let mut marks = vec![];
    back.walk(|n| {
        if n.appearance.stroke().is_some_and(|s| s.paint.is_registration())
            && let Some(b) = n.path_data().and_then(|p| p.bounds())
        {
            marks.push(b);
        }
    });
    assert_eq!(marks.len(), 8);
    assert!(marks.iter().all(|m| m.intersect(ab.inflate(-0.5, -0.5)).area() <= 0.0 && ab.inflate(40.0, 40.0).contains_rect(*m)), "{marks:?}");
    // The document's bleed by default: the BleedBox is the artboard grown by it.
    let mut bled = d.clone();
    bled.setup.bleed = [9.0; 4];
    let pdf = info(&job(&bled, json!({"autoRotate": false})).bytes, None).unwrap();
    let bleed = pdf.pages[0].boxes.iter().find(|(c, _)| *c == CropTo::Bleed).unwrap().1;
    assert_eq!((bleed.width().round(), bleed.height().round()), (218.0, 118.0));
    let pdf = info(&job(&bled, json!({"autoRotate": false, "bleed": {"useDocument": false}})).bytes, None).unwrap();
    let bleed = pdf.pages[0].boxes.iter().find(|(c, _)| *c == CropTo::Bleed).unwrap().1;
    assert_eq!((bleed.width().round(), bleed.height().round()), (200.0, 100.0));
}

#[test]
fn turns_mirror_and_negative() {
    let d = doc(100.0, 100.0, &[(Rect::new(0.0, 0.0, 50.0, 100.0), rgb(1.0, 0.0, 0.0))]);
    let first = |v| pages(&job(&d, v).bytes).remove(0);
    let with = |k: &str, v| {
        let mut s = json!({"placement": {"origin": "topLeft"}, "autoRotate": false});
        s[k] = v;
        s
    };
    assert!(near(first(with("output", json!({"emulsion": "down"}))).1[0].0, Rect::new(562.0, 0.0, 612.0, 100.0)), "mirrored");
    assert!(near(first(with("orientation", json!("portraitFlipped"))).1[0].0, Rect::new(562.0, 692.0, 612.0, 792.0)), "upside down");
    assert_eq!(first(with("orientation", json!("landscape"))).0, (792.0, 612.0));
    let p = first(with("transverse", json!(true)));
    assert_eq!(p.0, (792.0, 612.0), "a quarter turn");
    assert!(near(p.1[0].0, Rect::new(692.0, 0.0, 792.0, 50.0)), "{:?}", p.1);
    // A negative: black paper, the red square cyan.
    let back = import(&job(&d, with("output", json!({"image": "negative"}))).bytes).unwrap();
    let img = render_region(&back, back.artboards[0].rect, 1.0);
    assert_eq!(img.over_white(300, 300), [0, 0, 0]);
    let c = img.over_white(25, 50);
    assert!(c[0] < 10 && c[1] > 245 && c[2] > 245, "{c:?}");
}

#[test]
fn golden_page_one() {
    // 300 × 200: red left half, blue right half, a black frame; on Letter, turned landscape,
    // centred at 100%, with trim marks and registration targets.
    let mut d =
        doc(300.0, 200.0, &[(Rect::new(0.0, 0.0, 150.0, 200.0), rgb(1.0, 0.0, 0.0)), (Rect::new(150.0, 0.0, 300.0, 200.0), rgb(0.0, 0.0, 1.0))]);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    d.insert(
        Some(layer),
        2,
        Node::path(id, shapes::rectangle(Rect::new(10.0, 10.0, 290.0, 190.0)), Appearance::basic(Paint::None, rgb(0.0, 0.0, 0.0), 4.0)),
    )
    .unwrap();
    let r = job(&d, json!({"marks": {"trim": true, "registration": true, "weight": 2}}));
    let back = import(&r.bytes).unwrap();
    let page = back.artboards[0].rect;
    assert_eq!((page.width(), page.height()), (792.0, 612.0));
    let img = render_region(&back, page, 1.0);
    let (x, y) = (246, 206);
    for (px, py, want) in [
        (x + 75, y + 100, [255, 0, 0]),
        (x + 225, y + 100, [0, 0, 255]),
        (x + 10, y + 100, [0, 0, 0]),
        (x + 150, y + 10, [0, 0, 0]),
        (40, 40, [255, 255, 255]),
        (x - 3, y + 100, [255, 255, 255]),
        // A trim mark along the top edge, left of the corner past the 6 pt offset.
        (x - 12, y, [0, 0, 0]),
    ] {
        let c = img.over_white(px, py);
        assert!(c.iter().zip(want).all(|(a, b)| a.abs_diff(b) < 40), "({px}, {py}): {c:?} vs {want:?}");
    }
}
