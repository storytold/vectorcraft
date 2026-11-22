//! File → Print as a PDF: paper, scale, placement, tiles, artboard ranges, copies, layers,
//! separations, marks and the page turns.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint, GradientStop, Paint, Swatch};
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
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
        guide_label: None,
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
fn the_scale_link_is_on_for_new_settings_and_off_for_a_saved_scale_without_it() {
    let s = PrintSettings::default();
    assert!(s.scale.linked, "new settings keep W and H equal");
    assert_eq!(serde_json::to_value(&s).unwrap()["scale"], json!({"width": 100.0, "height": 100.0, "linked": true}));
    // Settings without a scale are new settings; a scale saved before the link reads as unlinked,
    // so its width and height print as they did.
    assert!(settings(json!({"copies": 2})).scale.linked);
    let old = settings(json!({"scaling": "custom", "scale": {"width": 50, "height": 200}}));
    assert_eq!((old.scale.width, old.scale.height, old.scale.linked), (50.0, 200.0, false));
    assert!(old.check().is_ok());
    // Linked, the two must be equal; the range still holds for both.
    let apart = settings(json!({"scale": {"width": 50, "height": 200, "linked": true}}));
    assert!(matches!(apart.check(), Err(PdfError::BadSetting(_))));
    for bad in
        [json!({"width": 0, "height": 0, "linked": true}), json!({"width": 1001, "height": 1001, "linked": true}), json!({"width": -5, "height": 50})]
    {
        assert!(matches!(settings(json!({ "scale": bad })).check(), Err(PdfError::BadSetting(_))), "{bad}");
    }
    assert!(settings(json!({"scale": {"width": 1000, "height": 1000, "linked": true}})).check().is_ok());
}

#[test]
fn japanese_paper_sizes() {
    // The Japanese papers: JIS B4, B5 and B6 are the sizes Japanese printing uses (JIS X 6310),
    // a little larger than ISO's; the others are はがき, 往復はがき, 封筒 and 四六判.
    let near = |m: Media, w: f64, h: f64| {
        let (mw, mh) = m.size().unwrap();
        assert!((mw - w).abs() < 0.01 && (mh - h).abs() < 0.01, "{m:?} is {mw} × {mh}, not {w} × {h}");
    };
    near(Media::B4Jis, 728.50, 1031.81);
    near(Media::B5Jis, 515.91, 728.50);
    near(Media::B6Jis, 362.83, 515.91);
    near(Media::A6, 297.64, 419.53);
    near(Media::JapanesePostcard, 283.46, 419.53);
    near(Media::JapaneseDoublePostcard, 419.53, 566.93);
    near(Media::JapaneseEnvelope3, 340.16, 666.14);
    near(Media::JapaneseEnvelope4, 255.12, 637.80);
    near(Media::Kaku2, 680.31, 941.10);
    near(Media::Kaku3, 612.28, 785.20);
    near(Media::YonrokuBan, 360.0, 532.91);
    // The ISO B sizes keep the dimensions settings and presets saved before the Japanese papers
    // existed: they are not the JIS sizes.
    near(Media::B4, 708.66, 1000.63);
    near(Media::B5, 498.90, 708.66);

    // A paper is named by its JSON id or its dialog label, any case and spacing.
    for (name, want) in [
        ("b4 (jis)", Media::B4Jis),
        ("B4 (JIS)", Media::B4Jis),
        ("b4jis", Media::B4Jis),
        ("b5", Media::B5),
        ("B5 (ISO)", Media::B5),
        ("b5 (jis)", Media::B5Jis),
        ("japanese postcard", Media::JapanesePostcard),
        ("Japanese Postcard", Media::JapanesePostcard),
        ("Japanese Envelope #3", Media::JapaneseEnvelope3),
        ("kaku #2", Media::Kaku2),
        ("yonroku-ban", Media::YonrokuBan),
    ] {
        assert_eq!(Media::from_name(name), Some(want), "{name}");
    }
    assert_eq!(Media::from_name("b7"), None);
    assert_eq!(Media::from_name("  "), None);
    for m in Media::ALL {
        assert_eq!(Media::from_name(m.id()), Some(*m), "id {:?}", m.id());
        assert_eq!(Media::from_name(m.label()), Some(*m), "label {:?}", m.label());
        assert_eq!(serde_json::to_value(m).unwrap(), json!(m.id()));
        assert_eq!(serde_json::from_value::<Media>(json!(m.id())).unwrap(), *m);
    }
    assert!(serde_json::from_value::<Media>(json!("b7")).is_err(), "an unknown paper is refused");

    // Every paper prints as a page of its own size, the added ones included.
    let d = doc(50.0, 50.0, &[(Rect::new(0.0, 0.0, 50.0, 50.0), rgb(0.0, 0.0, 0.0))]);
    for (media, w, h) in [
        ("japanese postcard", 283.46, 419.53),
        ("japanese double postcard", 419.53, 566.93),
        ("B5 (JIS)", 515.91, 728.50),
        ("kaku #2", 680.31, 941.10),
        ("yonroku-ban", 360.0, 532.91),
        // The ISO B sizes keep their page size.
        ("b5", 498.90, 708.66),
    ] {
        let r = job(&d, json!({"media": media, "autoRotate": false}));
        assert_eq!(r.pages, 1, "{media}");
        let p = pages(&r.bytes);
        assert_eq!(p.len(), 1, "{media}");
        assert!((p[0].0.0 - w).abs() < 0.01 && (p[0].0.1 - h).abs() < 0.01, "{media}: {:?}, not {w} × {h}", p[0].0);
    }
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

/// The top-left corners of the copies on the first page of `pv`.
fn corners(pv: &PrintPreview) -> Vec<(f64, f64)> {
    pv.sheets[0].copies.iter().map(|c| (c[0], c[1])).collect()
}

/// Every `(x, y)` of `xs` and `ys`, across then down.
fn grid(xs: &[f64], ys: &[f64]) -> Vec<(f64, f64)> {
    ys.iter().flat_map(|y| xs.iter().map(move |x| (*x, *y))).collect()
}

#[test]
fn copies_per_sheet_fill_the_page_in_a_grid() {
    // A 2 × 3.5 in card (144 × 252 pt) on Letter (612 × 792 pt): 4 across and 3 down, the
    // 576 × 756 pt grid centered.
    let d = doc(144.0, 252.0, &[(Rect::new(0.0, 0.0, 144.0, 252.0), rgb(1.0, 0.0, 0.0))]);
    let pv = preview(&d, &settings(json!({"copiesPerSheet": true}))).unwrap();
    assert_eq!((pv.pages, pv.sheets.len(), pv.sheets[0].orientation), (1, 1, Orientation::Portrait));
    assert_eq!(corners(&pv), grid(&[18.0, 162.0, 306.0, 450.0], &[18.0, 270.0, 522.0]));
    assert!(pv.sheets[0].copies.iter().all(|c| (c[2] - c[0], c[3] - c[1]) == (144.0, 252.0)), "{:?}", pv.sheets[0].copies);
    assert_eq!(pv.sheets[0].trim, [18.0, 18.0, 594.0, 774.0], "the TrimBox holds every copy");
    assert!(pv.warnings.is_empty(), "{:?}", pv.warnings);
    // The printed page holds the twelve cards there.
    let p = pages(&job(&d, json!({"copiesPerSheet": true})).bytes);
    assert_eq!(p.len(), 1);
    let mut printed: Vec<(f64, f64)> = p[0].1.iter().map(|(r, _)| (r.x0.round(), r.y0.round())).collect();
    printed.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
    assert_eq!(printed, grid(&[18.0, 162.0, 306.0, 450.0], &[18.0, 270.0, 522.0]), "{:?}", p[0].1);
    assert!(p[0].1.iter().all(|(r, paint)| rgb8(paint) == [255, 0, 0] && near(*r, Rect::new(r.x0, r.y0, r.x0 + 144.0, r.y0 + 252.0))));
    // An 18 pt gutter: 3 across and 3 down, the 468 × 792 pt grid centered.
    let pv = preview(&d, &settings(json!({"copiesPerSheet": true, "gutter": 18}))).unwrap();
    assert_eq!(corners(&pv), grid(&[72.0, 234.0, 396.0], &[0.0, 270.0, 540.0]));
    // The placement places the grid, inside the margin.
    let pv = preview(&d, &settings(json!({"copiesPerSheet": true, "margin": 9, "placement": {"origin": "topLeft"}}))).unwrap();
    assert_eq!(corners(&pv), grid(&[9.0, 153.0, 297.0, 441.0], &[9.0, 261.0, 513.0]));
    // Moved 400 pt right, two columns leave the paper: the TrimBox holds the copies still on it.
    let pv = preview(&d, &settings(json!({"copiesPerSheet": true, "placement": {"origin": "topLeft", "x": 400}}))).unwrap();
    assert_eq!(pv.sheets[0].trim, [400.0, 0.0, 612.0, 756.0]);
}

#[test]
fn copies_per_sheet_is_off_by_default_and_off_prints_as_before() {
    let s = PrintSettings::default();
    assert_eq!((s.copies_per_sheet, s.gutter), (false, 0.0));
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!((&v["copiesPerSheet"], &v["gutter"]), (&json!(false), &json!(0.0)));
    assert!(settings(json!({"gutter": -1})).check().is_err() && settings(json!({"gutter": 721})).check().is_err());
    // Off, the job is the same file whatever the gutter, and its pages list no copies.
    let d = doc(144.0, 252.0, &[(Rect::new(0.0, 0.0, 144.0, 252.0), rgb(1.0, 0.0, 0.0))]);
    assert_eq!(job(&d, json!({})).bytes, job(&d, json!({"copiesPerSheet": false, "gutter": 36})).bytes);
    let pv = serde_json::to_value(preview(&d, &settings(json!({}))).unwrap()).unwrap();
    assert!(pv["sheets"][0].get("copies").is_none(), "{pv}");
    // Tiles print one artboard across pages: copies per sheet is left out.
    let pv = preview(&d, &settings(json!({"copiesPerSheet": true, "scaling": "tileFull"}))).unwrap();
    assert!(pv.sheets[0].copies.is_empty() && pv.sheets[0].tile.is_some());
}

#[test]
fn an_artboard_that_does_not_fit_prints_one_copy_per_sheet() {
    // 700 × 900 pt is larger than Letter either way round.
    let d = doc(700.0, 900.0, &[(Rect::new(0.0, 0.0, 700.0, 900.0), rgb(0.0, 0.0, 1.0))]);
    let r = job(&d, json!({"copiesPerSheet": true}));
    assert_eq!(r.pages, 1);
    // A note on the options; the art that doesn't print is what the job loses.
    assert_eq!(r.setting_notes, ["Artboard 1 with its bleed and marks doesn't fit the imageable area: it prints one copy per sheet"]);
    assert!(r.warnings.iter().any(|w| w.contains("larger than the imageable area")), "{:?}", r.warnings);
    assert_eq!(r.bytes, job(&d, json!({})).bytes, "printed as without copies per sheet");
    assert!(preview(&d, &settings(json!({"copiesPerSheet": true}))).unwrap().sheets[0].copies.is_empty());
    // When one copy fits, it prints alone, centered, whatever the gutter.
    let d = doc(400.0, 500.0, &[]);
    let pv = preview(&d, &settings(json!({"copiesPerSheet": true, "gutter": 300}))).unwrap();
    assert_eq!(corners(&pv), [(106.0, 146.0)]);
    // More copies than a sheet takes are refused.
    let tiny = doc(1.0, 1.0, &[]);
    assert!(preview(&tiny, &settings(json!({"copiesPerSheet": true}))).is_err_and(|e| e.to_string().contains("more than 1000 copies")));
}

#[test]
fn each_copy_prints_with_its_marks() {
    let d = doc(100.0, 100.0, &[(Rect::new(0.0, 0.0, 100.0, 100.0), rgb(1.0, 0.0, 0.0))]);
    let set = json!({"copiesPerSheet": true, "marks": {"trim": true}});
    let pv = preview(&d, &settings(set.clone())).unwrap();
    let copies = pv.sheets[0].copies.len();
    assert!(copies > 1, "{pv:?}");
    let back = import(&job(&d, set).bytes).unwrap();
    let mut marks = 0;
    back.walk(|n| marks += usize::from(n.appearance.stroke().is_some_and(|s| s.paint.is_registration())));
    assert_eq!(marks, 8 * copies, "8 trim marks around each copy");
}

#[test]
fn every_copy_on_a_sheet_draws_the_same_symbol_instances() {
    // A symbol of three squares (4 objects with its group), placed twice.
    let mut d = doc(100.0, 100.0, &[]);
    let squares = (0..3)
        .map(|i| {
            let r = Rect::new(f64::from(i) * 10.0, 0.0, f64::from(i) * 10.0 + 5.0, 5.0);
            Arc::new(Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(rgb(1.0, 0.0, 0.0), Paint::None, 0.0)))
        })
        .collect();
    let art = Node::group(d.alloc_id(), squares);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Squares".into(), art: Arc::new(art) });
    let layer = d.default_layer().unwrap();
    for y in [10.0, 50.0] {
        let instance = NodeKind::SymbolInstance { symbol: "Squares".into(), xf: vectorcraft_geom::Affine::translate((10.0, y)) };
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::new(id, instance)).unwrap();
    }
    let set = PdfSettings { compression: CompressionSettings { compress_text: false, ..Default::default() }, ..Default::default() };
    let mut w = crate::export::Writer::new(&d, &set, "Copies", None, false).unwrap();
    let mut ex = crate::export::Exporter::new(&d, &set);
    // Room for 10 symbol objects: the first copy could write both instances (8) and leave the
    // second copy none.
    ex.symbol_limit = 10;
    let copies = [vectorcraft_geom::Vec2::ZERO, vectorcraft_geom::Vec2::new(200.0, 0.0)];
    let page = Rect::new(0.0, 0.0, 300.0, 100.0);
    let sheet = crate::export::Sheet {
        size: (300.0, 100.0),
        unit: 1.0,
        trim: page,
        bleed: page,
        view: vectorcraft_geom::Affine::IDENTITY,
        window: None,
        place: vectorcraft_geom::Affine::IDENTITY,
        copies: &copies,
        area: Rect::new(0.0, 0.0, 100.0, 100.0),
        clip: false,
        marks: None,
        negative: false,
        links: false,
    };
    w.page(&mut ex, &sheet).unwrap();
    assert_eq!(ex.symbol_limit, 10, "the limit is the export's again after the sheet");
    w.absorb(ex);
    let (bytes, ..) = w.finish(&mut std::collections::HashSet::new(), false).unwrap();
    let p = pages(&bytes);
    let left = p[0].1.iter().filter(|(r, _)| r.x0 < 200.0).count();
    assert_eq!((left, p[0].1.len() - left), (3, 3), "one instance in each copy: {:?}", p[0].1);
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
fn a_job_of_too_many_artboards_is_refused_while_laying_out() {
    let mut d = doc(100.0, 100.0, &[]);
    d.artboards.extend((2..=MAX_PAGES as u32 + 2).map(|i| artboard(i, 0.0)));
    let e = preview(&d, &settings(json!({}))).unwrap_err().to_string();
    assert!(e.contains("more than 2000 pages"), "{e}");
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
fn a_symbol_instance_prints_by_its_art() {
    // The symbol's red rectangle, (100, 100) to (140, 140), drawn from (-50, -50): on the artboard,
    // while the 20 × 20 square around the origin, moved to (-60, -60) to (-40, -40), is off it.
    let mut d = doc(100.0, 100.0, &[]);
    let red = Appearance::basic(rgb(1.0, 0.0, 0.0), Paint::None, 0.0);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 100.0, 140.0, 140.0)), red);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Far".into(), art: Arc::new(art) });
    let (layer, id) = (d.default_layer().unwrap(), d.alloc_id());
    let xf = vectorcraft_geom::Affine::translate((-50.0, -50.0));
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Far".into(), xf })).unwrap();
    // Skip Blank Artboards keeps the artboard.
    assert_eq!(preview(&d, &settings(json!({"skipBlank": true}))).unwrap().sheets.len(), 1);
    // Ignoring artboards, the page holds the art.
    let p = pages(&job(&d, json!({"artboards": "ignore"})).bytes);
    assert_eq!(p.len(), 1);
    let printed: Vec<_> = p[0].1.iter().map(|(r, paint)| (r.width().round(), r.height().round(), rgb8(paint))).collect();
    assert_eq!(printed, [(40.0, 40.0, [255, 0, 0])], "{p:?}");
}

#[test]
fn a_symbol_instances_brush_art_keeps_its_artboard_printed() {
    // The symbol's art: a line from (0, 40) to (0, 60) with a 1 pt red stroke painted by a 20 pt
    // round nib. Drawn from (-5, 0), the line lies left of the artboard, and the nib paints 10 pt
    // either side of it, onto the artboard.
    let mut d = doc(100.0, 100.0, &[]);
    let mut lib = vectorcraft_brush::library(&d);
    let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
        panic!("the 3 pt. Round brush")
    };
    c.size = 20.0;
    vectorcraft_brush::store(&mut d, &lib);
    let mut stroke = Appearance::basic(Paint::None, rgb(1.0, 0.0, 0.0), 1.0);
    stroke.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    let art = Node::path(d.alloc_id(), shapes::line((0.0, 40.0).into(), (0.0, 60.0).into()), stroke);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Nib".into(), art: Arc::new(art) });
    let (layer, id) = (d.default_layer().unwrap(), d.alloc_id());
    let xf = vectorcraft_geom::Affine::translate((-5.0, 0.0));
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Nib".into(), xf })).unwrap();
    // Skip Blank Artboards keeps the artboard, and its page holds the nib's art.
    let p = pages(&job(&d, json!({"skipBlank": true})).bytes);
    assert_eq!(p.len(), 1);
    assert!(p[0].1.iter().any(|(_, paint)| rgb8(paint) == [255, 0, 0]), "{p:?}");
}

#[test]
fn a_brushed_paths_brush_art_keeps_its_artboard_printed_and_framed() {
    // A line from (-5, 40) to (-5, 60) with a 1 pt red stroke painted by a 20 pt round nib, alone,
    // in a group or as the art of a symbol instance: the line lies left of the artboard, and the nib
    // paints 10 pt either side of it, onto the artboard.
    let line = || {
        let mut stroke = Appearance::basic(Paint::None, rgb(1.0, 0.0, 0.0), 1.0);
        stroke.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
        Node::path(vectorcraft_doc::NodeId(0), shapes::line((-5.0, 40.0).into(), (-5.0, 60.0).into()), stroke)
    };
    for what in ["path", "group", "symbol instance"] {
        let mut d = doc(100.0, 100.0, &[]);
        let mut lib = vectorcraft_brush::library(&d);
        let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
            panic!("the 3 pt. Round brush")
        };
        c.size = 20.0;
        vectorcraft_brush::store(&mut d, &lib);
        let mut n = match what {
            "path" => line(),
            "group" => Node::group(vectorcraft_doc::NodeId(0), vec![Arc::new(line())]),
            _ => {
                d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Nib".into(), art: Arc::new(line()) });
                Node::new(vectorcraft_doc::NodeId(0), NodeKind::SymbolInstance { symbol: "Nib".into(), xf: vectorcraft_geom::Affine::IDENTITY })
            }
        };
        n.id = d.alloc_id();
        let layer = d.default_layer().unwrap();
        d.insert(Some(layer), 0, n).unwrap();
        // Skip Blank Artboards keeps the artboard, and its page holds the nib's art.
        let p = pages(&job(&d, json!({"skipBlank": true})).bytes);
        assert_eq!(p.len(), 1, "{what}");
        assert!(p[0].1.iter().any(|(_, paint)| rgb8(paint) == [255, 0, 0]), "{what}: {p:?}");
        // Ignoring artboards, the page holds all the nib's art, 20 × 40 pt.
        let pv = preview(&d, &settings(json!({"artboards": "ignore"}))).unwrap();
        let t = pv.sheets[0].trim;
        assert!(t[2] - t[0] >= 20.0 && t[3] - t[1] >= 40.0, "{what}: {t:?}");
    }
    // A compound path is painted with its own appearance: the brushed stroke left on its path
    // paints nothing, and the artboard stays blank.
    let mut d = doc(100.0, 100.0, &[]);
    let kind = NodeKind::Compound { children: vec![Arc::new(line())], rule: vectorcraft_geom::FillRule::NonZero };
    let mut compound = Node::new(d.alloc_id(), kind);
    compound.appearance = Appearance::basic(Paint::None, rgb(1.0, 0.0, 0.0), 1.0).into();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, compound).unwrap();
    assert!(preview(&d, &settings(json!({"skipBlank": true}))).is_err());
    // A brushed stroke without a paint draws no brush art: the artboard stays blank.
    let mut d = doc(100.0, 100.0, &[]);
    let mut unpainted = line();
    unpainted.id = d.alloc_id();
    unpainted.appearance.stroke_mut().unwrap().paint = Paint::None;
    d.insert(Some(layer), 0, unpainted).unwrap();
    assert!(preview(&d, &settings(json!({"skipBlank": true}))).is_err());
}

#[test]
fn symbol_art_effects_print_where_they_draw() {
    // A red square and the symbol's blue square, each moved 100 pt right by a Transform effect,
    // print at the same distance from the page's left edge.
    let moved = |mut n: Node| {
        n.appearance.effects.push(vectorcraft_doc::Effect { id: "distort.transform".into(), params: json!({"moveH": 100.0}), visible: true });
        n
    };
    let mut d = doc(200.0, 100.0, &[]);
    let (layer, id) = (d.default_layer().unwrap(), d.alloc_id());
    let red = Node::path(id, shapes::rectangle(Rect::new(10.0, 10.0, 20.0, 20.0)), Appearance::basic(rgb(1.0, 0.0, 0.0), Paint::None, 0.0));
    d.insert(Some(layer), 0, moved(red)).unwrap();
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 40.0, 20.0, 50.0)), Appearance::basic(rgb(0.0, 0.0, 1.0), Paint::None, 0.0));
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Moved".into(), art: Arc::new(moved(art)) });
    let id = d.alloc_id();
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Moved".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
    let p = pages(&job(&d, json!({})).bytes);
    assert_eq!(p.len(), 1);
    let left = |c: [u8; 3]| p[0].1.iter().find(|(_, paint)| rgb8(paint) == c).map(|(r, _)| r.x0);
    let (red, blue) = (left([255, 0, 0]), left([0, 0, 255]));
    assert!(red.zip(blue).is_some_and(|(r, b)| (r - b).abs() < 0.05), "{p:?}");
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

/// A CMYK document with spot swatches Gold and Pine, and three 50 pt squares left to right: Gold
/// at 50%, Pine at 100% and Registration.
fn gold_and_pine() -> Document {
    let mut d = doc(200.0, 100.0, &[]);
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(Color::cmyk(0.0, 0.2, 0.8, 0.1)), global: true, spot: true });
    d.swatches.push(Swatch { name: "Pine".into(), paint: Paint::solid(Color::cmyk(0.8, 0.0, 0.3, 0.0)), global: true, spot: true });
    let layer = d.default_layer().unwrap();
    let linked = |color: Color, swatch: &str, tint: f32| Paint::Solid { color, swatch: Some(swatch.into()), tint };
    let squares = [
        (0.0, linked(Color::cmyk(0.0, 0.1, 0.4, 0.05), "Gold", 0.5)),
        (60.0, linked(Color::cmyk(0.8, 0.0, 0.3, 0.0), "Pine", 1.0)),
        (120.0, linked(Color::cmyk(1.0, 1.0, 1.0, 1.0), vectorcraft_color::swatch::REGISTRATION, 1.0)),
    ];
    for (x, p) in squares {
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 50.0, 50.0)), Appearance::basic(p, Paint::None, 0.0)))
            .unwrap();
    }
    d
}

/// The ink of each 50 pt square on a plate, left to right, in percent.
fn square_inks(page: &Page) -> Vec<f32> {
    page.1
        .iter()
        .filter(|(b, _)| (b.width() - 50.0).abs() < 1.0 && (b.height() - 50.0).abs() < 1.0)
        .map(|(_, p)| match p.color() {
            Some(Color::Gray { k }) => (k * 100.0).round(),
            c => panic!("{c:?}"),
        })
        .collect()
}

#[test]
fn one_spot_ink_converts_to_process_and_the_others_keep_their_plates() {
    let d = gold_and_pine();
    // Gold prints as process even with its plate turned off: converting is not leaving it out.
    let set = json!({"output": {"mode": "separations", "inks": [{"name": "Gold", "convertToProcess": true, "print": false, "frequency": 100}]}});
    let pv = preview(&d, &settings(set.clone())).unwrap();
    assert_eq!(pv.inks.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Cyan", "Magenta", "Yellow", "Black", "Pine"]);
    let all: Vec<(&str, bool)> = pv.document_inks.iter().map(|i| (i.name.as_str(), i.convert_to_process)).collect();
    assert_eq!(all, [("Cyan", false), ("Magenta", false), ("Yellow", false), ("Black", false), ("Gold", true), ("Pine", false)]);
    let gold = &pv.document_inks[4];
    assert_eq!((gold.print, gold.frequency), (false, 100.0), "its plate options are kept");
    assert!(pv.setting_notes.iter().all(|n| !n.contains("no ink named") && !n.contains("only spot inks")), "{:?}", pv.setting_notes);
    let p = pages(&job(&d, set).bytes);
    assert_eq!(p.len(), 5);
    // (Gold, Pine, Registration) on Cyan, Magenta, Yellow, Black and Pine.
    assert_eq!(
        p.iter().map(square_inks).collect::<Vec<_>>(),
        [[0.0, 0.0, 100.0], [10.0, 0.0, 100.0], [40.0, 0.0, 100.0], [5.0, 0.0, 100.0], [0.0, 100.0, 100.0]]
    );
}

#[test]
fn converting_every_spot_ink_overrides_and_keeps_each_inks_own_choice() {
    let d = gold_and_pine();
    let inks = json!([{"name": "Gold", "convertToProcess": false, "angle": 30}, {"name": "Pine", "convertToProcess": true}]);
    let pv = preview(&d, &settings(json!({"output": {"mode": "separations", "spotsToProcess": true, "inks": inks}}))).unwrap();
    assert_eq!(pv.inks.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Cyan", "Magenta", "Yellow", "Black"]);
    let spots: Vec<(&str, bool, f64)> = pv.document_inks[4..].iter().map(|i| (i.name.as_str(), i.convert_to_process, i.angle)).collect();
    assert_eq!(spots, [("Gold", true, 30.0), ("Pine", true, 45.0)]);
    // The options of the converted inks still name inks of the document.
    assert!(pv.setting_notes.iter().all(|n| !n.contains("no ink named")), "{:?}", pv.setting_notes);
    // Without the option, each ink's own choice applies again.
    let pv = preview(&d, &settings(json!({"output": {"mode": "separations", "inks": inks}}))).unwrap();
    assert_eq!(pv.inks.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Cyan", "Magenta", "Yellow", "Black", "Gold"]);
    // A process ink doesn't convert: it keeps its plate, and a note says so.
    let pv = preview(&d, &settings(json!({"output": {"mode": "separations", "inks": [{"name": "Cyan", "convertToProcess": true}]}}))).unwrap();
    assert_eq!(pv.inks.len(), 6);
    assert!(pv.document_inks.iter().all(|i| !i.convert_to_process));
    assert_eq!(pv.setting_notes, ["`Cyan` print as process inks already: only spot inks convert to process"]);
}

#[test]
fn an_ink_option_without_convert_to_process_keeps_its_spot_plate() {
    let set = settings(json!({"output": {"inks": [{"name": "Gold", "angle": 30}]}}));
    assert!(!set.output.inks[0].convert_to_process);
    let back = serde_json::to_value(&set.output.inks[0]).unwrap();
    assert_eq!(back, json!({"name": "Gold", "print": true, "angle": 30.0}), "off is not written");
    let on = InkSettings { convert_to_process: true, ..set.output.inks[0].clone() };
    assert_eq!(serde_json::to_value(&on).unwrap()["convertToProcess"], json!(true));
}

/// A 200 × 100 pt CMYK document filled with cyan, with spot swatch `Ink` (100% K) and the objects
/// `art` over the cyan.
fn over_cyan(art: Vec<Node>) -> Document {
    let mut d = doc(200.0, 100.0, &[(Rect::new(0.0, 0.0, 200.0, 100.0), Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)))]);
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    d.swatches.push(Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)), global: true, spot: true });
    let layer = d.default_layer().unwrap();
    for mut n in art {
        n.id = d.alloc_id();
        d.insert(Some(layer), usize::MAX, n).unwrap();
    }
    d
}

#[test]
fn overprint_black_keeps_the_inks_under_solid_black_on_separations() {
    let k = |v: f32| Paint::solid(Color::cmyk(0.0, 0.0, 0.0, v));
    let spot = Paint::Solid { color: Color::cmyk(0.0, 0.0, 0.0, 1.0), swatch: Some("Ink".into()), tint: 1.0 };
    let stops = vec![GradientStop::new(0.0, Color::cmyk(0.0, 0.0, 0.0, 1.0)), GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 0.0))];
    let fade = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let square = |x: f64, fill: Paint, stroke: Paint| {
        Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(x, 15.0, x + 20.0, 35.0)), Appearance::basic(fill, stroke, 4.0))
    };
    // (left edge, fill, whether the cyan below prints under it with the option on).
    let cases = [
        (10.0, k(1.0), true),
        (40.0, Paint::solid(Color::gray(1.0)), true),
        (70.0, Paint::solid(Color::cmyk(0.6, 0.4, 0.4, 1.0)), false),
        (100.0, k(0.99), false),
        (130.0, spot, false),
        (160.0, fade, false),
    ];
    let mut art: Vec<Node> = cases.iter().map(|(x, p, _)| square(*x, p.clone(), Paint::None)).collect();
    // A black stroke with no fill: its left side runs down x = 184.
    art.push(square(184.0, Paint::None, k(1.0)));
    let d = over_cyan(art);
    let plates = |v: serde_json::Value| -> Vec<vectorcraft_testkit::raster::Image> {
        let opts = PrintOptions { settings: settings(v), ..Default::default() };
        plan(&d, &opts).unwrap().plates.iter().map(|p| render_region(p, Rect::new(0.0, 0.0, 200.0, 100.0), 1.0)).collect()
    };
    // Full ink at x on the plate (a rich black's 60% cyan is not).
    let inked = |img: &vectorcraft_testkit::raster::Image, x: f64| img.over_white(x as u32, 25)[0] < 40;
    let off = plates(json!({"output": {"mode": "separations"}}));
    let on = plates(json!({"output": {"mode": "separations", "overprintBlack": true}}));
    for (x, _, overprints) in &cases {
        assert!(!inked(&off[0], x + 10.0), "off: the art at {x} knocks the cyan out");
        assert_eq!(inked(&on[0], x + 10.0), *overprints, "on: the cyan under the art at {x}");
    }
    assert!(!inked(&off[0], 184.0) && inked(&on[0], 184.0), "a black stroke overprints too");
    assert!(inked(&on[0], 194.0), "inside the unfilled square: cyan");
    assert!(inked(&on[3], 20.0) && inked(&off[3], 20.0), "the black prints on the Black plate either way");
    assert!(!inked(&on[3], 2.0), "no black is added where there is none");
    // The K and gray squares put no cyan down, so the cyan plate draws them as nothing: a writer
    // without blending modes (PostScript) keeps the cyan under them too.
    let opts = PrintOptions {
        settings: settings(json!({"output": {"mode": "separations", "overprintBlack": true}})),
        postscript: true,
        ..Default::default()
    };
    let p = plan(&d, &opts).unwrap();
    let fills = |doc: &Document| {
        let mut count = 0;
        doc.walk(|n| count += usize::from(n.appearance.fill().is_some_and(|f| !f.paint.is_none())));
        count
    };
    assert_eq!((fills(&d), fills(&p.plates[0]), fills(&p.plates[3])), (7, 5, 7), "the cyan plate, the Black plate");
    // The document keeps its own settings; composite output ignores the option.
    assert!(!d.layers.iter().any(|l| l.has_overprint()));
    let marked = |v| printed_document(&d, &settings(v)).layers.iter().any(|l| l.has_overprint());
    assert!(marked(json!({"output": {"mode": "separations", "overprintBlack": true}})));
    assert!(!marked(json!({"output": {"overprintBlack": true}})), "composite");
    assert!(!marked(json!({"output": {"mode": "separations"}})), "off by default");
}

#[test]
fn overprint_black_keeps_the_cyan_under_near_and_freeform_blacks_on_postscript_plates() {
    // 0.4% cyan beside 100% K counts as black, as do a freeform gradient of black points and a
    // linear gradient of black stops, and the cyan plate draws them all as nothing.
    let near = Paint::solid(Color::cmyk(0.004, 0.0, 0.0, 1.0));
    let k = Color::cmyk(0.0, 0.0, 0.0, 1.0);
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Freeform, vec![GradientStop::new(0.0, k), GradientStop::new(1.0, k)]));
    let points = vec![FreeformPoint::new(kurbo::Point::new(45.0, 20.0), k), FreeformPoint::new(kurbo::Point::new(55.0, 30.0), Color::gray(1.0))];
    g.freeform = Some(Freeform { points, ..Default::default() });
    // A linear gradient draws its black stops, not the cyan point it still holds.
    let mut linear = g.clone();
    linear.gradient.kind = GradientKind::Linear;
    if let Some(f) = &mut linear.freeform {
        f.points[1].color = Color::cmyk(1.0, 0.0, 0.0, 0.0);
    }
    let square = |x: f64, fill: Paint| {
        Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(x, 15.0, x + 20.0, 35.0)), Appearance::basic(fill, Paint::None, 0.0))
    };
    let d = over_cyan(vec![square(10.0, near), square(40.0, Paint::Gradient(Box::new(g))), square(70.0, Paint::Gradient(Box::new(linear)))]);
    let opts = PrintOptions {
        settings: settings(json!({"output": {"mode": "separations", "overprintBlack": true}})),
        postscript: true,
        ..Default::default()
    };
    let p = plan(&d, &opts).unwrap();
    let fills = |doc: &Document| {
        let mut count = 0;
        doc.walk(|n| count += usize::from(n.appearance.fill().is_some_and(|f| !f.paint.is_none())));
        count
    };
    assert_eq!((fills(&p.plates[0]), fills(&p.plates[3])), (1, 4), "the cyan plate, the Black plate");
}

#[test]
fn overprint_black_keeps_the_cyan_under_a_near_black_linked_to_a_missing_swatch_on_postscript_plates() {
    // 0.4% cyan beside 100% K, linked to a swatch the document lacks: it separates from its own
    // values and the option marks it to overprint, so it counts as black like an unlinked color.
    let near = Paint::Solid { color: Color::cmyk(0.004, 0.0, 0.0, 1.0), swatch: Some("Missing".into()), tint: 1.0 };
    let square =
        Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(10.0, 15.0, 30.0, 35.0)), Appearance::basic(near, Paint::None, 0.0));
    let d = over_cyan(vec![square]);
    assert!(d.swatch("Missing").is_none());
    let opts = PrintOptions {
        settings: settings(json!({"output": {"mode": "separations", "overprintBlack": true}})),
        postscript: true,
        ..Default::default()
    };
    let p = plan(&d, &opts).unwrap();
    let fills = |doc: &Document| {
        let mut count = 0;
        doc.walk(|n| count += usize::from(n.appearance.fill().is_some_and(|f| !f.paint.is_none())));
        count
    };
    assert_eq!((fills(&p.plates[0]), fills(&p.plates[3])), (1, 2), "the cyan plate, the Black plate");
}

#[test]
fn overprinting_near_blacks_linked_to_a_missing_swatch_keep_the_cyan_on_postscript_plates() {
    // 0.4% cyan beside 100% K, linked to a swatch the document lacks and set to overprint, on a
    // fill, a stroke, a character fill and a linear gradient's stops: a PostScript cyan plate draws
    // it as nothing, as it does an unlinked black, and a PDF cyan plate draws it. A faint cyan that
    // is not black, linked to the same name, stays on both.
    let missing = |color: Color| Paint::Solid { color, swatch: Some("Missing".into()), tint: 1.0 };
    let near = Color::cmyk(0.004, 0.0, 0.0, 1.0);
    let faint = Color::cmyk(0.004, 0.5, 0.0, 0.0);
    let square = |x: f64, fill: Paint, stroke: Paint| {
        let mut n =
            Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(x, 15.0, x + 20.0, 35.0)), Appearance::basic(fill, stroke, 2.0));
        if let Some(f) = n.appearance.fill_mut() {
            f.overprint = true;
        }
        if let Some(s) = n.appearance.stroke_mut() {
            s.overprint = true;
        }
        n
    };
    let stop = |offset: f32| GradientStop { swatch: Some("Missing".into()), ..GradientStop::new(offset, near) };
    let linear = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![stop(0.0), stop(1.0)]));
    let style = vectorcraft_doc::CharStyle { fill: missing(near), overprint_fill: true, ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "Hi", style);
    let d = over_cyan(vec![
        square(10.0, missing(near), Paint::None),
        square(40.0, Paint::None, missing(near)),
        square(70.0, Paint::Gradient(Box::new(linear)), Paint::None),
        square(100.0, missing(faint), Paint::None),
        Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text))),
    ]);
    assert!(d.swatch("Missing").is_none());
    // The fills, strokes and character fills plate `plate` draws.
    let drawn = |postscript: bool, plate: usize| {
        let opts = PrintOptions { settings: settings(json!({"output": {"mode": "separations"}})), postscript, ..Default::default() };
        let p = plan(&d, &opts).unwrap();
        let (mut fills, mut strokes, mut chars) = (0, 0, 0);
        p.plates[plate].walk(|n| {
            fills += usize::from(n.appearance.fill().is_some_and(|f| !f.paint.is_none()));
            strokes += usize::from(n.appearance.stroke().is_some_and(|s| !s.paint.is_none()));
            if let NodeKind::Text(t) = &n.kind {
                chars += t.runs.iter().filter(|r| !r.style.fill.is_none()).count();
            }
        });
        (fills, strokes, chars)
    };
    assert_eq!(drawn(true, 0), (2, 0, 0), "PostScript cyan plate: the cyan and the faint fill");
    assert_eq!(drawn(false, 0), (4, 1, 1), "PDF cyan plate: the cyan, the fill, the gradient, the faint fill, the stroke and the type");
    assert_eq!(drawn(true, 3), (3, 1, 1), "PostScript Black plate: every black, not the faint fill");
}

#[test]
fn an_overprinting_fill_keeps_its_faint_ink_on_pdf_plates() {
    // 0.4% cyan beside 100% K, set to overprint: a PDF plate draws it with Darken, a PostScript
    // plate (it has no blending modes) draws it as nothing, since the color counts as black.
    let mut square = Node::path(
        vectorcraft_doc::NodeId(0),
        shapes::rectangle(Rect::new(10.0, 15.0, 30.0, 35.0)),
        Appearance::basic(Paint::solid(Color::cmyk(0.004, 0.0, 0.0, 1.0)), Paint::None, 0.0),
    );
    if let Some(f) = square.appearance.fill_mut() {
        f.overprint = true;
    }
    let d = over_cyan(vec![square]);
    let fills = |postscript: bool| {
        let opts = PrintOptions { settings: settings(json!({"output": {"mode": "separations"}})), postscript, ..Default::default() };
        let p = plan(&d, &opts).unwrap();
        let mut count = 0;
        p.plates[0].walk(|n| count += usize::from(n.appearance.fill().is_some_and(|f| !f.paint.is_none())));
        count
    };
    assert_eq!((fills(false), fills(true)), (2, 1), "the cyan plate as PDF, as PostScript");
}

#[test]
fn overprinting_paints_keep_their_faint_ink_on_postscript_plates() {
    // 0.4% cyan in a color that is not black, set to overprint, on a fill, a stroke, a character
    // fill, a linear gradient's stop and a freeform gradient's point: the cyan plate draws it on
    // PDF and PostScript plates alike. A magenta with no cyan at all draws as nothing.
    let faint = Color::cmyk(0.004, 0.5, 0.0, 0.0);
    let magenta = Color::cmyk(0.0, 0.5, 0.0, 0.0);
    let square = |x: f64, fill: Paint, stroke: Paint| {
        let mut n =
            Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(x, 15.0, x + 20.0, 35.0)), Appearance::basic(fill, stroke, 2.0));
        if let Some(f) = n.appearance.fill_mut() {
            f.overprint = true;
        }
        if let Some(s) = n.appearance.stroke_mut() {
            s.overprint = true;
        }
        n
    };
    let linear = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, faint), GradientStop::new(1.0, magenta)]));
    let mut freeform =
        GradientPaint::new(Gradient::new(GradientKind::Freeform, vec![GradientStop::new(0.0, magenta), GradientStop::new(1.0, magenta)]));
    let points = vec![FreeformPoint::new(kurbo::Point::new(105.0, 20.0), magenta), FreeformPoint::new(kurbo::Point::new(115.0, 30.0), faint)];
    freeform.freeform = Some(Freeform { points, ..Default::default() });
    let style = vectorcraft_doc::CharStyle { fill: Paint::solid(faint), overprint_fill: true, ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "Hi", style);
    let d = over_cyan(vec![
        square(10.0, Paint::solid(faint), Paint::None),
        square(40.0, Paint::None, Paint::solid(faint)),
        square(70.0, Paint::Gradient(Box::new(linear)), Paint::None),
        square(100.0, Paint::Gradient(Box::new(freeform)), Paint::None),
        square(130.0, Paint::solid(magenta), Paint::None),
        Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text))),
        // A black linked to the spot swatch at a 0.4% tint: 0.4% on the spot plate, none on the cyan.
        square(160.0, Paint::Solid { color: Color::cmyk(0.0, 0.0, 0.0, 1.0), swatch: Some("Ink".into()), tint: 0.004 }, Paint::None),
    ]);
    let opts = PrintOptions { settings: settings(json!({"output": {"mode": "separations"}})), postscript: true, ..Default::default() };
    let p = plan(&d, &opts).unwrap();
    let mut spot = vec![];
    p.plates[4].walk(|n| {
        spot.extend(n.appearance.fill().and_then(|f| f.paint.color()).map(|c| match c {
            Color::Gray { k } => (k * 1000.0).round(),
            c => panic!("{c:?}"),
        }))
    });
    assert!(spot.contains(&4.0), "the PostScript spot plate draws the spot tint: {spot:?}");
    // The fills, strokes and character fills the cyan plate draws, and the gray of the solid fills.
    let drawn = |postscript: bool| {
        let opts = PrintOptions { settings: settings(json!({"output": {"mode": "separations"}})), postscript, ..Default::default() };
        let p = plan(&d, &opts).unwrap();
        let (mut fills, mut strokes, mut chars, mut grays) = (0, 0, 0, vec![]);
        p.plates[0].walk(|n| {
            if let Some(f) = n.appearance.fill().filter(|f| !f.paint.is_none()) {
                fills += 1;
                if let Some(Color::Gray { k }) = f.paint.color() {
                    grays.push((k * 1000.0).round());
                }
            }
            strokes += usize::from(n.appearance.stroke().is_some_and(|s| !s.paint.is_none()));
            if let NodeKind::Text(t) = &n.kind {
                chars += t.runs.iter().filter(|r| !r.style.fill.is_none()).count();
            }
        });
        (fills, strokes, chars, grays)
    };
    for postscript in [false, true] {
        let (fills, strokes, chars, grays) = drawn(postscript);
        let at = if postscript { "PostScript" } else { "PDF" };
        assert_eq!((strokes, chars), (1, 1), "{at}: the stroke and the character fill");
        assert_eq!(fills, 4, "{at}: the cyan, the faint fill and both gradients, not the magenta");
        assert_eq!(grays, [1000.0, 4.0], "{at}: the cyan and the faint fill's 0.4%");
    }
}

#[test]
fn a_near_black_stop_of_an_overprinting_gradient_puts_no_cyan_on_postscript_plates() {
    // 0.4% cyan beside 100% K counts as black on a PostScript plate stop by stop, as a solid fill
    // of it does: the cyan plate draws that stop of an overprinting gradient as paper white. A
    // PDF plate and a gradient that does not overprint keep the 0.4%.
    let stops = vec![GradientStop::new(0.0, Color::cmyk(0.004, 0.0, 0.0, 1.0)), GradientStop::new(1.0, Color::cmyk(0.5, 0.0, 0.0, 0.0))];
    let fade = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let square = |x: f64, overprint: bool| {
        let r = shapes::rectangle(Rect::new(x, 15.0, x + 20.0, 35.0));
        let mut n = Node::path(vectorcraft_doc::NodeId(0), r, Appearance::basic(fade.clone(), Paint::None, 0.0));
        if let Some(f) = n.appearance.fill_mut() {
            f.overprint = overprint;
        }
        n
    };
    let d = over_cyan(vec![square(10.0, true), square(40.0, false)]);
    // The cyan plate's stops of each gradient, in thousandths of the ink.
    let stops = |postscript: bool| {
        let opts = PrintOptions { settings: settings(json!({"output": {"mode": "separations"}})), postscript, ..Default::default() };
        let p = plan(&d, &opts).unwrap();
        let mut all = vec![];
        p.plates[0].walk(|n| {
            if let Some(Paint::Gradient(g)) = n.appearance.fill().map(|f| &f.paint) {
                let gray = |s: &GradientStop| match s.color {
                    Color::Gray { k } if s.swatch.is_none() => (k * 1000.0).round(),
                    c => panic!("{c:?}"),
                };
                all.push(g.gradient.stops.iter().map(gray).collect::<Vec<_>>());
            }
        });
        all
    };
    assert_eq!(stops(true), [[0.0, 500.0], [4.0, 500.0]], "PostScript: the overprinting gradient, the other");
    assert_eq!(stops(false), [[4.0, 500.0], [4.0, 500.0]], "PDF");
}

#[test]
fn overprinting_type_leaves_the_inks_below_on_separations() {
    let style = vectorcraft_doc::CharStyle { fill: Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)), ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "Hi", style);
    let mut d = over_cyan(vec![Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text)))]);
    let darkens = |d: &Document, v| String::from_utf8_lossy(&job(d, v).bytes).contains("/Darken");
    let seps = json!({"output": {"mode": "separations"}});
    assert!(!darkens(&d, seps.clone()), "black type knocks out");
    assert!(darkens(&d, json!({"output": {"mode": "separations", "overprintBlack": true}})), "Overprint Black: the type overprints");
    // A character fill set to overprint does so on the plates without the option too.
    let layer = d.default_layer().unwrap();
    let id = d.node(layer).and_then(|l| l.children()).and_then(|c| c.last()).map(|n| n.id).unwrap();
    let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("type") };
    t.runs[0].style.overprint_fill = true;
    assert!(darkens(&d, seps), "an overprinting character fill");
}

#[test]
fn a_type_objects_own_black_fill_overprints_on_separations() {
    // No character fill: the type object's own fill paints the characters.
    let style = vectorcraft_doc::CharStyle { fill: Paint::None, ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "Hi", style);
    let mut n = Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text)));
    n.appearance = Appearance::basic(Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)), Paint::None, 0.0).into();
    let d = over_cyan(vec![n]);
    let darkens = |v| String::from_utf8_lossy(&job(&d, v).bytes).contains("/Darken");
    assert!(!darkens(json!({"output": {"mode": "separations"}})), "it knocks out");
    assert!(darkens(json!({"output": {"mode": "separations", "overprintBlack": true}})), "Overprint Black: it overprints");
}

#[test]
fn overprint_black_follows_a_symbol_instances_own_fill_on_separations() {
    use vectorcraft_doc::symbol_overrides::SymbolOverride;
    let (k, magenta) = (Color::cmyk(0.0, 0.0, 0.0, 1.0), Color::cmyk(0.0, 1.0, 0.0, 0.0));
    let square = |x: f64, c: Color| {
        let r = shapes::rectangle(Rect::new(x, 15.0, x + 20.0, 35.0));
        Arc::new(Node::path(vectorcraft_doc::NodeId(0), r, Appearance::basic(Paint::solid(c), Paint::None, 0.0)))
    };
    let instance = |symbol: &str, fill: Color| {
        let mut n = Node::new(vectorcraft_doc::NodeId(0), NodeKind::SymbolInstance { symbol: symbol.into(), xf: Default::default() });
        n.extras_mut().symbol_overrides = vec![SymbolOverride { path: vec![], fill: Some(Paint::solid(fill)), ..Default::default() }];
        n
    };
    // An instance of "Outer", whose art is the magenta symbol's instance painted black, moved right.
    let outer =
        Node::new(vectorcraft_doc::NodeId(0), NodeKind::SymbolInstance { symbol: "Outer".into(), xf: kurbo::Affine::translate((140.0, 0.0)) });
    let mut d = over_cyan(vec![instance("Magenta", k), instance("Black", magenta), outer]);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: true, name: "Magenta".into(), art: square(10.0, magenta) });
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: true, name: "Black".into(), art: square(40.0, k) });
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Outer".into(), art: Arc::new(instance("Magenta", k)) });
    let opts = PrintOptions { settings: settings(json!({"output": {"mode": "separations", "overprintBlack": true}})), ..Default::default() };
    let p = plan(&d, &opts).unwrap();
    let cyan = render_region(&p.plates[0], Rect::new(0.0, 0.0, 200.0, 100.0), 1.0);
    let inked = |x: u32| cyan.over_white(x, 25)[0] < 40;
    assert!(inked(20), "a magenta symbol's instance painted black overprints the cyan");
    assert!(!inked(50), "a black symbol's instance painted magenta knocks the cyan out");
    assert!(inked(160), "the instance painted black inside another symbol's art overprints too");
    assert!(!d.symbols.iter().any(|s| s.art.has_overprint()), "the document keeps its own settings");
}

#[test]
fn overprint_black_follows_instance_paints_in_inline_graphics() {
    use vectorcraft_doc::symbol_overrides::SymbolOverride;
    // Type shows symbol "Holder" inline; its art is an instance of a black symbol painted magenta.
    let k = Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0));
    let r = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    let mut inst = Node::new(vectorcraft_doc::NodeId(0), NodeKind::SymbolInstance { symbol: "Black".into(), xf: Default::default() });
    let magenta = Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0));
    inst.extras_mut().symbol_overrides = vec![SymbolOverride { path: vec![], fill: Some(magenta), ..Default::default() }];
    let mut text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "Hi", Default::default());
    text.runs.push(vectorcraft_doc::TextRun::inline(vectorcraft_doc::InlineArt::new("Holder"), Default::default()));
    let mut d = over_cyan(vec![Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text)))]);
    let black = Arc::new(Node::path(vectorcraft_doc::NodeId(0), r, Appearance::basic(k, Paint::None, 0.0)));
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: true, name: "Black".into(), art: black });
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Holder".into(), art: Arc::new(inst) });
    let printed = printed_document(&d, &settings(json!({"output": {"mode": "separations", "overprintBlack": true}})));
    let holder = printed.symbols.iter().find(|s| s.name == "Holder").unwrap();
    let mut instances = 0;
    holder.art.walk(&mut |n: &Node| instances += usize::from(matches!(n.kind, NodeKind::SymbolInstance { .. })));
    assert_eq!(instances, 0, "the instance is placed as the art it prints");
    assert!(!holder.art.has_overprint(), "its magenta fill knocks out");
    assert!(matches!(d.symbols[1].art.kind, NodeKind::SymbolInstance { .. }), "the document keeps its art");
}

#[test]
fn overprinting_type_in_an_opacity_mask_does_not_darken_on_separations() {
    // Mask art has no inks: an overprinting gray character in an inverted clipping mask keeps its
    // luminance on every plate.
    let style = vectorcraft_doc::CharStyle { fill: Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 0.5)), overprint_fill: true, ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "Hi", style);
    let mut mask = vectorcraft_doc::OpacityMask::new(Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text))), true);
    mask.invert = true;
    let square = Node::path(
        vectorcraft_doc::NodeId(0),
        shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)),
        Appearance::basic(Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)), Paint::None, 0.0),
    )
    .with_mask(Some(Box::new(mask)));
    let d = over_cyan(vec![square]);
    let bytes = job(&d, json!({"output": {"mode": "separations"}})).bytes;
    assert!(!String::from_utf8_lossy(&bytes).contains("/Darken"), "the mask's character is drawn as it is");
}

/// A 100 pt CMYK document with spot swatch `Gold` and a square filled with gradient `g`.
fn gold_gradient_doc(g: GradientPaint) -> Document {
    let mut d = doc(100.0, 100.0, &[(Rect::new(0.0, 0.0, 50.0, 50.0), Paint::Gradient(Box::new(g)))]);
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(Color::cmyk(0.0, 0.2, 0.8, 0.1)), global: true, spot: true });
    d
}

/// The grey of each plate of a separation of `d` (job options `v`), in percent: of the first
/// gradient stop, and of the first freeform point when there is one. No plate keeps a link or
/// process plates.
fn gradient_plates(d: &Document, v: serde_json::Value) -> Vec<(f32, Option<f32>)> {
    let opts = PrintOptions { settings: settings(v), ..Default::default() };
    let gray = |c: &Color| match c {
        Color::Gray { k } => (k * 100.0).round(),
        c => panic!("{c:?}"),
    };
    let plan = plan(d, &opts).unwrap();
    plan.plates
        .iter()
        .map(|p| {
            let mut found = vec![];
            p.walk(|n| {
                if let Some(Paint::Gradient(g)) = n.appearance.fill().map(|f| &f.paint) {
                    let s = &g.gradient.stops[0];
                    assert_eq!((s.swatch.as_deref(), s.tint, s.plates), (None, 1.0, None), "the stop is unlinked");
                    found.push((gray(&s.color), g.freeform.as_ref().and_then(|f| f.points.first()).map(|q| gray(&q.color))));
                }
            });
            assert_eq!(found.len(), 1);
            found[0]
        })
        .collect()
}

#[test]
fn a_spot_gradient_stop_prints_its_process_plates() {
    let mut mixed = GradientStop::new(0.0, Color::cmyk(0.25, 0.1, 0.8, 0.05));
    (mixed.swatch, mixed.tint, mixed.plates) = (Some("Gold".into()), 0.5, Some([0.25, 0.0, 0.4, 0.0]));
    let g = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![mixed, GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 0.0))]));
    let d = gold_gradient_doc(g);
    let first = |v| gradient_plates(&d, v).into_iter().map(|(s, _)| s).collect::<Vec<_>>();
    assert_eq!(first(json!({"output": {"mode": "separations"}})), [25.0, 0.0, 40.0, 0.0, 50.0], "Cyan, Magenta, Yellow, Black, Gold");
    // Spots as process: the plates layered with the gold tint's process inks (0, 10, 40, 5).
    assert_eq!(first(json!({"output": {"mode": "separations", "spotsToProcess": true}})), [25.0, 10.0, 64.0, 5.0]);
    // Gold converted on its own: the same.
    assert_eq!(first(json!({"output": {"mode": "separations", "inks": [{"name": "Gold", "convertToProcess": true}]}})), [25.0, 10.0, 64.0, 5.0]);
}

#[test]
fn a_spot_gradient_stop_whose_tint_is_not_a_number_prints_no_spot_ink() {
    let mut mixed = GradientStop::new(0.0, Color::cmyk(0.25, 0.1, 0.8, 0.05));
    (mixed.swatch, mixed.tint, mixed.plates) = (Some("Gold".into()), f32::NAN, Some([0.25, 0.0, 0.4, 0.0]));
    let g = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![mixed, GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 0.0))]));
    let d = gold_gradient_doc(g);
    let first = |v| gradient_plates(&d, v).into_iter().map(|(s, _)| s).collect::<Vec<_>>();
    assert_eq!(first(json!({"output": {"mode": "separations"}})), [25.0, 0.0, 40.0, 0.0, 0.0], "Cyan, Magenta, Yellow, Black, Gold");
    assert_eq!(first(json!({"output": {"mode": "separations", "spotsToProcess": true}})), [25.0, 0.0, 40.0, 0.0]);
}

#[test]
fn a_freeform_gradients_points_separate_into_plates() {
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Freeform, vec![GradientStop::new(0.0, Color::cmyk(0.0, 0.0, 0.0, 1.0))]));
    let point = FreeformPoint::new(vectorcraft_geom::Point::new(25.0, 25.0), Color::cmyk(0.0, 0.6, 0.0, 0.0));
    g.freeform = Some(Freeform { points: vec![point], ..Default::default() });
    let d = gold_gradient_doc(g);
    let set = json!({"output": {"mode": "separations"}});
    let points = gradient_plates(&d, set.clone()).into_iter().map(|(_, p)| p).collect::<Vec<_>>();
    assert_eq!(points, [Some(0.0), Some(60.0), Some(0.0), Some(0.0), Some(0.0)], "the point's magenta, on its plate only");
    // Each plate draws the square in the grey of the point's ink on it.
    let plan = plan(&d, &PrintOptions { settings: settings(set), ..Default::default() }).unwrap();
    let drawn: Vec<u8> = plan.plates.iter().map(|p| render_region(p, Rect::new(0.0, 0.0, 50.0, 50.0), 1.0).over_white(25, 25)[0]).collect();
    for (plate, (v, ink)) in drawn.iter().zip([0.0, 0.6, 0.0, 0.0, 0.0]).enumerate() {
        assert!((f32::from(*v) - 255.0 * (1.0 - ink)).abs() <= 3.0, "plate {plate}: {drawn:?}");
    }
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
fn for_a_printer_landscape_pages_print_upright_turned_anticlockwise() {
    let d = doc(100.0, 100.0, &[(Rect::new(0.0, 0.0, 50.0, 100.0), rgb(1.0, 0.0, 0.0))]);
    let s = json!({"placement": {"origin": "topLeft"}, "autoRotate": false, "orientation": "landscape"});
    let spooled = |v| {
        let opts = PrintOptions { settings: settings(v), created: Some(1_791_200_000), uncompressed: true, for_printer: true, ..Default::default() };
        pages(&print(&d, &opts).unwrap().bytes).remove(0)
    };
    // Saved, the page stays landscape.
    assert_eq!(pages(&job(&d, s.clone()).bytes)[0].0, (792.0, 612.0));
    // For a printer, Letter upright: the art's top at the paper's left edge, its left half at the bottom.
    let p = spooled(s.clone());
    assert_eq!(p.0, (612.0, 792.0));
    assert!(near(p.1[0].0, Rect::new(0.0, 742.0, 100.0, 792.0)), "{:?}", p.1);
    // A portrait page and a transverse one (already upright) don't turn.
    let p = spooled(json!({"placement": {"origin": "topLeft"}, "autoRotate": false}));
    assert_eq!(p.0, (612.0, 792.0));
    assert!(near(p.1[0].0, Rect::new(0.0, 0.0, 50.0, 100.0)), "{:?}", p.1);
    let mut t = s;
    t["transverse"] = json!(true);
    assert_eq!(spooled(t).0, (612.0, 792.0));
    // Transverse in Portrait is wide on purpose: it stays wide.
    assert_eq!(spooled(json!({"autoRotate": false, "transverse": true})).0, (792.0, 612.0));
}

#[test]
fn for_a_printer_paper_wide_in_portrait_stays_as_it_is() {
    // A 4 × 2 in label in Portrait: the queue's media is wide too.
    let d = doc(288.0, 144.0, &[(Rect::new(0.0, 0.0, 144.0, 144.0), rgb(1.0, 0.0, 0.0))]);
    let v =
        json!({"media": "custom", "width": 288, "height": 144, "orientation": "portrait", "autoRotate": false, "placement": {"origin": "topLeft"}});
    let opts = PrintOptions { settings: settings(v), uncompressed: true, for_printer: true, ..Default::default() };
    let p = pages(&print(&d, &opts).unwrap().bytes).remove(0);
    assert_eq!(p.0, (288.0, 144.0));
    assert!(near(p.1[0].0, Rect::new(0.0, 0.0, 144.0, 144.0)), "{:?}", p.1);
}

#[test]
fn for_a_printer_auto_rotated_artboards_all_print_upright() {
    // A wide artboard and a tall one, the paper turned to each.
    let mut d = doc(100.0, 100.0, &[(Rect::new(0.0, 0.0, 10.0, 10.0), rgb(1.0, 0.0, 0.0))]);
    let mut wide = artboard(1, 0.0);
    wide.rect = Rect::new(0.0, 0.0, 300.0, 100.0);
    let mut tall = artboard(2, 400.0);
    tall.rect = Rect::new(400.0, 0.0, 500.0, 300.0);
    d.artboards = vec![wide, tall];
    let set = settings(json!({"autoRotate": true}));
    let saved = print(&d, &PrintOptions { settings: set.clone(), ..Default::default() }).unwrap();
    let sizes = |bytes: &[u8]| pages(bytes).into_iter().map(|p| p.0).collect::<Vec<_>>();
    assert_eq!(sizes(&saved.bytes), [(792.0, 612.0), (612.0, 792.0)]);
    let spooled = print(&d, &PrintOptions { settings: set, for_printer: true, ..Default::default() }).unwrap();
    assert_eq!(sizes(&spooled.bytes), [(612.0, 792.0), (612.0, 792.0)]);
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

/// Separations go through the document's colour spaces: the CMYK profile assigned to it, not the
/// working one (a document opened again in a new session keeps its profile, while the working
/// spaces go back to the defaults).
#[test]
fn separations_use_the_cmyk_profile_assigned_to_the_document() {
    let mut d = doc(100.0, 100.0, &[(Rect::new(0.0, 0.0, 50.0, 50.0), rgb(0.5, 0.5, 0.5))]);
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    // The cyan of the grey square, in percent.
    let cyan = |d: &Document| -> f32 {
        let p = pages(&job(d, json!({"output": {"mode": "separations"}})).bytes);
        match p[0].1.iter().find(|(b, _)| (b.width() - 50.0).abs() < 1.0).and_then(|(_, p)| p.color()) {
            Some(Color::Gray { k }) => (k * 100.0).round(),
            c => panic!("{c:?}"),
        }
    };
    let working = cyan(&d);
    assert!(working > 0.0, "the working CMYK profile separates grey into four inks: cyan {working}");
    d.color_profiles.cmyk = Some(vectorcraft_color::cms::DEVICE_CMYK.into());
    assert_eq!(cyan(&d), 0.0, "Device CMYK puts a neutral grey on the black plate only");
}

/// PDF print embeds font subsets (where the font's license allows it) whatever Fonts says: None
/// and Complete get a note in the print
/// and its preview, Subset none. PostScript output, which follows Fonts, gets none.
#[test]
fn non_subset_font_choices_report_pdf_subset_output() {
    let d = doc(200.0, 100.0, &[(Rect::new(10.0, 10.0, 60.0, 60.0), rgb(1.0, 0.0, 0.0))]);
    for (fonts, label) in [("none", Some("None")), ("subset", None), ("complete", Some("Complete"))] {
        let v = json!({"graphics": {"fonts": fonts}});
        let note = |notes: &[String]| notes.iter().filter(|n| n.starts_with("PDF print embeds subsets")).cloned().collect::<Vec<_>>();
        let want: Vec<String> = label
            .map(|l| {
                format!("PDF print embeds subsets of the fonts whose licenses allow it and draws the others as outlines: Fonts: {l} applies to PostScript output")
            })
            .into_iter()
            .collect();
        assert_eq!(note(&job(&d, v.clone()).setting_notes), want, "{fonts}");
        assert_eq!(note(&preview(&d, &settings(v.clone())).unwrap().setting_notes), want, "{fonts}");
        let plan = plan(&d, &PrintOptions { settings: settings(v), postscript: true, ..Default::default() }).unwrap();
        assert!(note(&plan.setting_notes).is_empty(), "{fonts}: {:?}", plan.setting_notes);
    }
}
