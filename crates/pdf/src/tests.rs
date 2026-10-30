use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, CharStyle, Dash, Document, ImageBlob, ImageObject, LineCap, LineJoin, Node, NodeId, NodeKind, TextObject,
};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect, shapes};

use crate::*;

mod blend;
mod charstroke;
mod cmykblend;
mod cmykimages;
mod colorize;
mod compression;
mod dashalign;
mod editing;
mod encrypt;
mod file_info;
mod focal;
mod fx;
mod import_color;
pub(super) mod import_fidelity;
mod import_layers;
mod import_lines;
mod import_options;
mod import_overprint;
mod layers;
pub(super) mod live_text;
mod marks;
mod mesh_export;
mod output;
mod pdf13;
mod pdfx;
mod presets;
mod print;
mod printadvanced;
mod reader_compat;
mod reader_streaming;
mod realtext;
mod settings;
pub(super) mod stroke;
mod strokeout;
mod text_placement;
mod textstroke;
mod webview;

fn doc(w: f64, h: f64) -> Document {
    Document::new(w, h)
}

fn add(d: &mut Document, mut n: Node) -> NodeId {
    n.id = d.alloc_id();
    let layer = d.default_layer().expect("layer");
    let len = d.children(Some(layer)).map_or(0, |c| c.len());
    d.insert(Some(layer), len, n).expect("insert")
}

fn rect_node(r: Rect, fill: Color) -> Node {
    Node::path(NodeId(0), shapes::rectangle(r), Appearance::basic(Paint::solid(fill), Paint::None, 0.0))
}

fn leaves(d: &Document) -> Vec<&Node> {
    let mut out = vec![];
    d.walk(|n| {
        if !n.is_container() && !matches!(n.kind, NodeKind::Path { clipping: true, .. }) {
            out.push(n);
        }
    });
    out
}

fn close(a: Rect, b: Rect, tol: f64) -> bool {
    (a.x0 - b.x0).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y1 - b.y1).abs() < tol
}

/// `d` exported with the default options and imported back: the document a round trip through a PDF
/// gives.
fn roundtrip(d: &Document) -> Document {
    let bytes = export(d, &PdfOptions::default()).expect("export");
    import(&bytes).expect("import")
}

/// `d` exported uncompressed, dated `created: Some(0)` rather than by the clock: two exports of one
/// document are then the same bytes, where the clock's dates, in whole seconds, differ whenever a
/// second ticks between them.
fn uncompressed(d: &Document) -> String {
    let bytes = export(d, &PdfOptions { created: Some(0), ..PdfOptions::uncompressed() }).expect("export");
    String::from_utf8_lossy(&bytes).into_owned()
}

fn page_count(bytes: &[u8]) -> usize {
    craft_pdf::syntax::Pdf::new(bytes.to_vec()).expect("parse").pages().len()
}

#[test]
fn export_is_valid_pdf() {
    let mut d = doc(200.0, 100.0);
    add(&mut d, rect_node(Rect::new(10.0, 10.0, 50.0, 50.0), Color::rgb(1.0, 0.0, 0.0)));
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.7"));
    let tail = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(64)..]).into_owned();
    assert!(tail.contains("startxref") && tail.contains("%%EOF"), "{tail}");
    assert!(String::from_utf8_lossy(&bytes).contains("xref"));
    assert_eq!(page_count(&bytes), 1);
}

#[test]
fn metadata_creator_and_title() {
    let mut d = doc(100.0, 100.0);
    d.title = "Poster".into();
    let s = uncompressed(&d);
    assert!(s.contains("VectorCraft"));
    assert!(s.contains("Poster"));
}

#[test]
fn one_page_per_artboard_with_sizes() {
    let mut d = doc(200.0, 100.0);
    d.artboards.push(Artboard {
        id: 2,
        name: "B".into(),
        rect: Rect::new(300.0, 0.0, 400.0, 400.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });
    d.artboards.push(Artboard {
        id: 3,
        name: "C".into(),
        rect: Rect::new(0.0, 500.0, 612.0, 1292.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    let pdf = craft_pdf::syntax::Pdf::new(bytes).unwrap();
    let sizes: Vec<(f32, f32)> = pdf.pages().iter().map(|p| p.render_dimensions()).collect();
    assert_eq!(sizes, vec![(200.0, 100.0), (100.0, 400.0), (612.0, 792.0)]);
}

#[test]
fn artboard_range_and_bad_index() {
    let mut d = doc(200.0, 100.0);
    d.artboards.push(Artboard {
        id: 2,
        name: "B".into(),
        rect: Rect::new(300.0, 0.0, 400.0, 400.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });
    let bytes = export(&d, &PdfOptions { artboards: Some(vec![1]), ..Default::default() }).unwrap();
    let pdf = craft_pdf::syntax::Pdf::new(bytes).unwrap();
    assert_eq!(pdf.pages().len(), 1);
    assert_eq!(pdf.pages()[0].render_dimensions(), (100.0, 400.0));
    assert_eq!(export(&d, &PdfOptions { artboards: Some(vec![5]), ..Default::default() }), Err(PdfError::BadArtboard(5)));
    d.artboards.clear();
    assert_eq!(export(&d, &PdfOptions::default()), Err(PdfError::NoArtboards));
}

#[test]
fn roundtrip_rect_and_ellipse_bounds() {
    let mut d = doc(300.0, 200.0);
    add(&mut d, rect_node(Rect::new(10.0, 20.0, 110.0, 70.0), Color::rgb(1.0, 0.0, 0.0)));
    let e = Node::path(
        NodeId(0),
        shapes::ellipse(Rect::new(150.0, 40.0, 250.0, 180.0)),
        Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0),
    );
    add(&mut d, e);
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 2);
    assert!(close(l[0].geometric_bounds().unwrap(), Rect::new(10.0, 20.0, 110.0, 70.0), 0.01), "{:?}", l[0].geometric_bounds());
    assert!(close(l[1].geometric_bounds().unwrap(), Rect::new(150.0, 40.0, 250.0, 180.0), 0.05), "{:?}", l[1].geometric_bounds());
    assert_eq!(out.artboards.len(), 1);
    assert!(close(out.artboards[0].rect, Rect::new(0.0, 0.0, 300.0, 200.0), 1e-6));
}

#[test]
fn an_instance_whose_art_reaches_the_page_is_written() {
    // The symbol's red rectangle, (100, 50) to (140, 90), drawn from (-50, -50): on the page, while
    // the 20 × 20 square around the origin, moved to (-60, -60) to (-40, -40), is off it.
    let mut d = doc(200.0, 100.0);
    let red = rect_node(Rect::new(100.0, 50.0, 140.0, 90.0), Color::rgb(1.0, 0.0, 0.0));
    let far = vectorcraft_doc::Symbol { name: "Far".into(), art: Arc::new(red) };
    d.symbols.push(far.clone());
    let instance = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((-50.0, -50.0)) });
    add(&mut d, instance.clone());
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 1);
    assert!(close(l[0].geometric_bounds().unwrap(), Rect::new(50.0, 0.0, 90.0, 40.0), 0.01), "{:?}", l[0].geometric_bounds());
    // An element of a knockout group, beside a blue rectangle.
    let mut d = doc(200.0, 100.0);
    d.symbols.push(far);
    let blue = rect_node(Rect::new(150.0, 10.0, 170.0, 30.0), Color::rgb(0.0, 0.0, 1.0));
    let mut group = Node::group(NodeId(0), vec![Arc::new(instance), Arc::new(blue)]);
    group.knockout = vectorcraft_doc::Knockout::On;
    add(&mut d, group);
    let fills: Vec<[u8; 4]> = leaves(&roundtrip(&d)).iter().filter_map(|n| Some(n.appearance.fill_paint().color()?.to_rgba8(1.0))).collect();
    assert!(fills.contains(&[255, 0, 0, 255]) && fills.contains(&[0, 0, 255, 255]), "{fills:?}");
}

#[test]
fn an_instance_whose_brush_art_reaches_the_page_is_written() {
    // The symbol's art: a line from (0, 40) to (0, 60) with a 1 pt stroke painted by a 20 pt round
    // nib. Drawn from (-5, 0), the line lies left of the page, and the nib paints 10 pt either side
    // of it, onto the page.
    let mut d = doc(200.0, 100.0);
    let mut lib = vectorcraft_brush::library(&d);
    let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
        panic!("the 3 pt. Round brush")
    };
    c.size = 20.0;
    vectorcraft_brush::store(&mut d, &lib);
    let mut stroke = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    stroke.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    let line = Node::path(NodeId(0), shapes::line((0.0, 40.0).into(), (0.0, 60.0).into()), stroke);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Nib".into(), art: Arc::new(line) });
    add(&mut d, Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Nib".into(), xf: Affine::translate((-5.0, 0.0)) }));
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 1, "the nib's art");
    assert!(close(l[0].geometric_bounds().unwrap(), Rect::new(-15.0, 30.0, 5.0, 70.0), 0.01), "{:?}", l[0].geometric_bounds());
}

#[test]
fn roundtrip_colours_preserved() {
    let mut d = doc(300.0, 100.0);
    let cols = [Color::rgb8(0x12, 0x34, 0x56), Color::rgb8(0xff, 0x80, 0x00), Color::rgb8(0x00, 0xc0, 0x40)];
    for (i, c) in cols.iter().enumerate() {
        add(&mut d, rect_node(Rect::new(10.0 + 90.0 * i as f64, 10.0, 90.0 + 90.0 * i as f64, 90.0), *c));
    }
    let out = roundtrip(&d);
    let got: Vec<String> = leaves(&out).iter().map(|n| n.appearance.fill_paint().color().unwrap().to_hex()).collect();
    assert_eq!(got, cols.iter().map(|c| c.to_hex()).collect::<Vec<_>>());
}

#[test]
fn roundtrip_stroke_params_merge_fill_and_stroke() {
    let mut d = doc(200.0, 200.0);
    let mut p = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 120.0)),
        Appearance::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 4.0),
    );
    {
        let st = p.appearance.stroke_mut().unwrap();
        st.cap = LineCap::Round;
        st.join = LineJoin::Bevel;
        st.miter_limit = 7.0;
        st.dash = Some(Dash { pattern: vec![6.0, 3.0], offset: 1.0, align_corners: false });
    }
    add(&mut d, p);
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 1, "fill + stroke of one path should import as one object");
    let st = l[0].appearance.stroke().unwrap();
    assert!((st.width - 4.0).abs() < 1e-3);
    assert_eq!(st.cap, LineCap::Round);
    assert_eq!(st.join, LineJoin::Bevel);
    assert!((st.miter_limit - 7.0).abs() < 1e-3);
    let dash = st.dash.as_ref().unwrap();
    assert_eq!(dash.pattern.len(), 2);
    assert!((dash.pattern[0] - 6.0).abs() < 1e-3 && (dash.offset - 1.0).abs() < 1e-3);
    assert_eq!(l[0].appearance.fill_paint().color().unwrap().to_hex(), "#ffffff");
}

#[test]
fn roundtrip_open_path_and_even_odd() {
    let mut d = doc(200.0, 200.0);
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((10.0, 10.0));
    bp.curve_to((50.0, 0.0), (80.0, 100.0), (150.0, 60.0));
    add(&mut d, Node::path(NodeId(0), PathData::from_bezpath(&bp), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0)));
    let mut ring = shapes::rectangle(Rect::new(20.0, 100.0, 120.0, 190.0));
    ring.subpaths.extend(shapes::rectangle(Rect::new(40.0, 120.0, 100.0, 170.0)).subpaths);
    let mut n = rect_node(Rect::ZERO, Color::BLACK);
    n.kind = NodeKind::Path { path: ring, rule: FillRule::EvenOdd, live: None, clipping: false, guide: false };
    add(&mut d, n);
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 2);
    let pb = l[0].path_data().unwrap();
    assert!(!pb.is_closed());
    let b = l[0].geometric_bounds().unwrap();
    let want = bp.bounding_box_exact();
    assert!(close(b, want, 0.05), "{b:?} vs {want:?}");
    assert!(matches!(l[1].kind, NodeKind::Path { rule: FillRule::EvenOdd, .. }));
    assert_eq!(l[1].path_data().unwrap().subpaths.len(), 2);
}

trait Exact {
    fn bounding_box_exact(&self) -> Rect;
}
impl Exact for vectorcraft_geom::BezPath {
    fn bounding_box_exact(&self) -> Rect {
        kurbo::Shape::bounding_box(self)
    }
}

#[test]
fn gradients_exported_as_shadings_and_reimported() {
    let mut d = doc(300.0, 100.0);
    let lin = GradientPaint::new(Gradient::default());
    let mut n = rect_node(Rect::new(0.0, 0.0, 140.0, 100.0), Color::BLACK);
    n.appearance.set_fill(Paint::Gradient(Box::new(lin)));
    add(&mut d, n);
    let rad = GradientPaint::new(Gradient { kind: GradientKind::Radial, ..Gradient::default() });
    let mut n = rect_node(Rect::new(150.0, 0.0, 250.0, 100.0), Color::BLACK);
    n.appearance.set_fill(Paint::Gradient(Box::new(rad)));
    add(&mut d, n);
    let s = uncompressed(&d);
    let s = s.replace(' ', "");
    assert!(s.contains("/ShadingType2"), "axial shading");
    assert!(s.contains("/ShadingType3"), "radial shading");
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 2);
    let Paint::Gradient(g) = l[0].appearance.fill_paint() else { panic!("linear gradient lost: {:?}", l[0].appearance) };
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    let geom = g.geom.unwrap();
    assert!((geom.start.x - 0.0).abs() < 0.1 && (geom.end.x - 140.0).abs() < 0.1, "{geom:?}");
    assert_eq!(g.gradient.stops.first().unwrap().color.to_hex(), "#ffffff");
    assert_eq!(g.gradient.stops.last().unwrap().color.to_hex(), "#000000");
    let Paint::Gradient(g) = l[1].appearance.fill_paint() else { panic!("radial gradient lost") };
    assert_eq!(g.gradient.kind, GradientKind::Radial);
    assert!((g.geom.unwrap().start - Point::new(200.0, 50.0)).hypot() < 0.1);
}

#[test]
fn smooth_revolve_exports_axial_vector_shading_without_images_and_keeps_the_source_live() {
    let mut d = doc(200.0, 200.0);
    let bp = vectorcraft_geom::BezPath::from_svg("M100 30 C140 50 140 120 100 160").unwrap();
    let mut n = Node::path(NodeId(0), PathData::from_bezpath(&bp), Appearance::basic(Paint::solid(Color::rgb(0.2, 0.55, 0.9)), Paint::None, 0.0));
    n.appearance.effects.push(vectorcraft_doc::Effect {
        id: "threeD.revolve".into(),
        visible: true,
        params: serde_json::json!({"offset":20,"rotationX":-25,"cap":true,"segments":16}),
    });
    add(&mut d, n);
    let before = d.clone();
    let pdf = uncompressed(&d).replace(' ', "");
    assert!(pdf.matches("/ShadingType2").count() > 50);
    assert!(!pdf.contains("/Subtype/Image"));
    assert_eq!(d, before);
    let back = roundtrip(&d);
    let strokes: Vec<_> = leaves(&back)
        .into_iter()
        .flat_map(|n| n.appearance.items.iter())
        .filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None })
        .collect();
    assert!(strokes.len() >= 2, "continuous cap rims remain vector strokes");
    assert!(strokes.iter().all(|s| s.join == LineJoin::Round), "seam strokes keep round joins in PDF");
}

#[test]
fn hidden_objects_and_template_layers_skipped() {
    let mut d = doc(200.0, 200.0);
    add(&mut d, rect_node(Rect::new(10.0, 10.0, 50.0, 50.0), Color::BLACK));
    let mut hidden = rect_node(Rect::new(60.0, 10.0, 90.0, 50.0), Color::BLACK);
    hidden.visible = false;
    add(&mut d, hidden);
    let t = d.add_layer(Some("Template"));
    if let NodeKind::Layer { template, .. } = &mut d.node_mut(t).unwrap().kind {
        *template = true;
    }
    let mut n = rect_node(Rect::new(100.0, 100.0, 150.0, 150.0), Color::BLACK);
    n.id = d.alloc_id();
    d.insert(Some(t), 0, n).unwrap();
    let mut guide = rect_node(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK);
    if let NodeKind::Path { guide: g, .. } = &mut guide.kind {
        *g = true;
    }
    add(&mut d, guide);
    let out = roundtrip(&d);
    assert_eq!(leaves(&out).len(), 1);
}

#[test]
fn objects_outside_artboard_are_culled() {
    let mut d = doc(100.0, 100.0);
    add(&mut d, rect_node(Rect::new(10.0, 10.0, 50.0, 50.0), Color::BLACK));
    add(&mut d, rect_node(Rect::new(500.0, 500.0, 550.0, 550.0), Color::BLACK));
    assert_eq!(leaves(&roundtrip(&d)).len(), 1);
}

/// A line from (10, `y`) to (90, `y`).
fn line(y: f64) -> PathData {
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((10.0, y));
    bp.line_to((90.0, y));
    PathData::from_bezpath(&bp)
}

/// Art that the PDF would hold a value beyond a billion points for is left out: a path that far,
/// a gradient placed that far, a stroke so wide that its group's box reaches that far, and an
/// image placed that far. Each would be written with -2,147,483,648, which the PDF writer panics
/// on in builds with overflow checks.
#[test]
fn art_reaching_beyond_a_billion_points_is_left_out() {
    let far = -2_147_483_649.0;
    let shaded = |kind, start, end| {
        let mut n = rect_node(Rect::new(60.0, 10.0, 90.0, 50.0), Color::BLACK);
        let geom = GradientGeom { start, end, aspect: 1.0, focal: None };
        let g = GradientPaint { geom: Some(geom), ..GradientPaint::new(Gradient { kind, ..Gradient::default() }) };
        n.appearance.set_fill(Paint::Gradient(Box::new(g)));
        n
    };
    let mut wide = Node::path(NodeId(0), line(0.0), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4_294_967_296.0));
    wide.opacity = 0.5;
    let image = ImageObject {
        key: "img1".into(),
        width: 4,
        height: 2,
        xf: Affine::new([5.5e8, 0.0, 0.0, 10.0, far, 60.0]),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    for (what, n) in [
        ("path", rect_node(Rect::new(far, 60.0, 90.0, 90.0), Color::BLACK)),
        ("linear gradient", shaded(GradientKind::Linear, Point::new(far, 0.0), Point::new(90.0, 0.0))),
        ("radial gradient", shaded(GradientKind::Radial, Point::new(far, 30.0), Point::new(far + 100.0, 30.0))),
        ("stroke", wide),
        ("image", Node::new(NodeId(0), NodeKind::Image(Box::new(image)))),
    ] {
        let mut d = doc(100.0, 100.0);
        d.images.insert("img1".into(), ImageBlob::new("image/png", tiny_png()));
        add(&mut d, rect_node(Rect::new(10.0, 10.0, 50.0, 50.0), Color::BLACK));
        add(&mut d, n);
        assert_eq!(leaves(&roundtrip(&d)).len(), 1, "{what}");
    }
}

/// A dash offset beyond a billion points is written as its remainder after whole periods of the
/// pattern: -2,147,483,649 (-2,147,483,648 as an f32, which the PDF writer panics on in builds
/// with overflow checks) is 1 in a pattern of 5 and 5.
#[test]
fn dash_offsets_beyond_a_billion_points_wrap_into_their_pattern() {
    let mut d = doc(100.0, 100.0);
    let mut n = Node::path(NodeId(0), line(50.0), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0));
    n.appearance.stroke_mut().unwrap().dash = Some(Dash { pattern: vec![5.0, 5.0], offset: -2_147_483_649.0, align_corners: false });
    add(&mut d, n);
    assert!(uncompressed(&d).contains("[5 5]1 d"));
}

/// The page of an artboard beyond a billion points is left blank: what is on it would be placed
/// through -2,147,483,648, which the PDF writer panics on in builds with overflow checks.
#[test]
fn artboards_beyond_a_billion_points_export_blank_pages() {
    let mut d = doc(100.0, 100.0);
    d.artboards[0].rect = Rect::new(2_147_483_649.0, 0.0, 2_147_483_749.0, 100.0);
    let mut n = rect_node(Rect::new(2_147_483_659.0, 10.0, 2_147_483_699.0, 50.0), Color::BLACK);
    n.opacity = 0.5;
    add(&mut d, n);
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    assert_eq!(page_count(&bytes), 1);
    assert!(leaves(&import(&bytes).unwrap()).is_empty());
}

#[test]
fn opacity_and_blend_modes() {
    let mut d = doc(200.0, 200.0);
    let mut n = rect_node(Rect::new(10.0, 10.0, 100.0, 100.0), Color::rgb(1.0, 0.0, 0.0));
    n.opacity = 0.5;
    n.blend = BlendMode::Multiply;
    add(&mut d, n);
    let s = uncompressed(&d);
    let s = s.replace(' ', "");
    assert!(s.contains("/BM/Multiply"), "blend mode in ExtGState");
    assert!(s.contains("/ca0.5"), "opacity in ExtGState");
    assert!(s.contains("/Transparency"), "transparency group");
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 1);
    // Opacity/blend end up on the object or an enclosing group.
    let mut op = 1.0;
    let mut bm = BlendMode::Normal;
    out.walk(|n| {
        op *= n.opacity * n.appearance.fill().map_or(1.0, |f| f.opacity);
        if n.blend != BlendMode::Normal {
            bm = n.blend;
        }
    });
    assert!((op - 0.5).abs() < 0.01, "{op}");
    assert_eq!(bm, BlendMode::Multiply);
}

#[test]
fn clip_group_roundtrip() {
    let mut d = doc(200.0, 200.0);
    let clip = Node::new(
        NodeId(0),
        NodeKind::Path {
            path: shapes::ellipse(Rect::new(20.0, 20.0, 120.0, 120.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: true,
            guide: false,
        },
    );
    let mut clip = clip;
    clip.id = d.alloc_id();
    let mut a = rect_node(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.0, 1.0, 0.0));
    a.id = d.alloc_id();
    let mut b = rect_node(Rect::new(50.0, 50.0, 150.0, 150.0), Color::rgb(0.0, 0.0, 1.0));
    b.id = d.alloc_id();
    let g = Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(a), Arc::new(b)], clip: true });
    add(&mut d, g);
    let out = roundtrip(&d);
    let mut groups = vec![];
    out.walk(|n| {
        if let NodeKind::Group { clip: true, children } = &n.kind {
            groups.push(children.len());
        }
    });
    assert_eq!(groups, vec![3]);
    let cg = {
        let mut r = None;
        out.walk(|n| {
            if matches!(n.kind, NodeKind::Group { clip: true, .. }) {
                r = n.geometric_bounds();
            }
        });
        r.unwrap()
    };
    assert!(close(cg, Rect::new(20.0, 20.0, 120.0, 120.0), 0.05), "{cg:?}");
}

#[test]
fn text_exported_as_outlines() {
    let mut d = doc(300.0, 100.0);
    let t = TextObject::point(Point::new(10.0, 50.0), "Hello", CharStyle { size: 24.0, ..CharStyle::default() });
    add(&mut d, Node::new(NodeId(0), NodeKind::Text(Box::new(t))));
    let mut outlined = PdfOptions::uncompressed();
    outlined.settings.advanced.outline_text = true;
    let s = String::from_utf8_lossy(&export(&d, &outlined).unwrap()).into_owned();
    assert!(!s.contains("/Font"), "no fonts when outlining text");
    // Real text in an embedded font by default, as a PDF from Illustrator has it (#733).
    assert!(uncompressed(&d).contains("/FontFile"), "a font embedded by default");
    let r = import_with_report(&export(&d, &outlined).unwrap(), &ImportOptions::default()).unwrap();
    let l = leaves(&r.document);
    assert!(!l.is_empty());
    assert!(l.iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    let b = l.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.geometric_bounds())).unwrap();
    assert!(b.x0 >= 9.0 && b.x1 < 120.0 && b.y1 <= 56.0 && b.y0 > 25.0, "{b:?}");
}

/// Colour bitmap glyphs (emoji) are images in the PDF: in their font's Type 3 glyphs with real
/// text, else drawn where the glyphs are.
#[test]
fn colour_bitmap_glyphs_export_as_images() {
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, colour_bitmap_family, colour_bitmap_font};
    for sbix in [true, false] {
        vectorcraft_text::FontDb::global().add_font(colour_bitmap_font(sbix).unwrap());
        let mut d = doc(300.0, 100.0);
        let st = CharStyle { font_family: colour_bitmap_family(sbix).into(), size: 24.0, ..CharStyle::default() };
        add(&mut d, Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 50.0), &format!("{COLOUR_CHAR}F"), st)))));
        let text = uncompressed(&d).replace(' ', "");
        assert!(text.contains("/Type3") && text.contains("/Subtype/Image"), "an image in a Type 3 glyph (sbix {sbix})");
        let mut outlined = PdfOptions::uncompressed();
        outlined.settings.advanced.outline_text = true;
        let r = import_with_report(&export(&d, &outlined).unwrap(), &ImportOptions::default()).unwrap();
        let l = leaves(&r.document);
        let images: Vec<_> = l.iter().filter(|n| matches!(n.kind, NodeKind::Image(_))).collect();
        assert_eq!(images.len(), 1, "outlined, the glyph is an image (sbix {sbix})");
        let b = images[0].geometric_bounds().unwrap();
        assert!((b.width() - 24.0).abs() < 0.1 && (b.height() - 24.0).abs() < 0.1, "one em: {b:?}");
        assert!(l.iter().any(|n| matches!(n.kind, NodeKind::Path { .. })), "the other glyph stays an outline");
    }
}

fn tiny_png() -> Vec<u8> {
    let mut img = image::RgbaImage::new(4, 2);
    for (x, _, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba([if x < 2 { 255 } else { 0 }, 0, 0, 255]);
    }
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

#[test]
fn image_roundtrip() {
    let mut d = doc(200.0, 200.0);
    d.images.insert("img1".into(), ImageBlob::new("image/png", tiny_png()));
    let im = ImageObject {
        key: "img1".into(),
        width: 4,
        height: 2,
        xf: Affine::translate((20.0, 30.0)) * Affine::scale(10.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    add(&mut d, Node::new(NodeId(0), NodeKind::Image(Box::new(im))));
    let out = roundtrip(&d);
    let l = leaves(&out);
    assert_eq!(l.len(), 1);
    let NodeKind::Image(im) = &l[0].kind else { panic!("expected image, got {:?}", l[0].kind_label()) };
    assert_eq!((im.width, im.height), (4, 2));
    assert!(close(l[0].geometric_bounds().unwrap(), Rect::new(20.0, 30.0, 60.0, 50.0), 0.01), "{:?}", l[0].geometric_bounds());
    let blob = &out.images[&im.key];
    let decoded = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(decoded.get_pixel(3, 0).0, [0, 0, 0, 255]);
}

/// Images are written with /Interpolate true, so viewers may smooth them; a pixelated image (an
/// envelope's with Anti-Alias off) is written without it.
#[test]
fn a_pixelated_image_asks_viewers_not_to_smooth_it() {
    for pixelated in [false, true] {
        let mut d = doc(200.0, 200.0);
        d.images.insert("img1".into(), ImageBlob::new("image/png", tiny_png()));
        let im = ImageObject {
            key: "img1".into(),
            width: 4,
            height: 2,
            xf: Affine::translate((20.0, 30.0)) * Affine::scale(10.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let mut n = Node::new(NodeId(0), NodeKind::Image(Box::new(im)));
        n.extras_mut().pixelated = pixelated;
        add(&mut d, n);
        let pdf = uncompressed(&d);
        assert_eq!(pdf.matches("/Width 4").count(), 1, "the image is written");
        assert_eq!(pdf.matches("/Interpolate").count(), usize::from(!pixelated), "pixelated: {pixelated}");
    }
}

/// A 4 × 2 red image encoded as `format`, cut short `keep` bytes past the first `tag` in it: its
/// header reads, and its pixel data can't be decoded.
fn cut_image(format: image::ImageFormat, tag: &[u8], keep: usize) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
    let at = out.windows(tag.len()).position(|w| w == tag).expect("tag");
    out.truncate(at + tag.len() + keep);
    out
}

/// An image whose pixel data can't be decoded (a PNG, GIF or WebP whose header reads) is left out
/// of the PDF and of a print job, with one note however many there are, and the rest of the page is
/// written.
#[test]
fn an_image_whose_pixels_cant_be_decoded_is_left_out_with_one_note() {
    const NOTE: &str = "an image couldn't be decoded and is missing from the picture";
    let mut d = doc(200.0, 300.0);
    let blobs = [
        ("good", ImageBlob::new("image/png", tiny_png())),
        ("png", ImageBlob::new("image/png", cut_image(image::ImageFormat::Png, b"IDAT", 4))),
        ("png2", ImageBlob::new("image/png", cut_image(image::ImageFormat::Png, b"IDAT", 6))),
        ("gif", ImageBlob::new("image/gif", cut_image(image::ImageFormat::Gif, b"GIF89a", 7))),
        ("webp", ImageBlob::new("image/webp", cut_image(image::ImageFormat::WebP, b"VP8L", 9))),
    ];
    for (i, (key, blob)) in blobs.into_iter().enumerate() {
        d.images.insert(key.into(), blob);
        let xf = Affine::translate((20.0, 20.0 + 50.0 * i as f64)) * Affine::scale(10.0);
        let im = ImageObject { key: key.into(), width: 4, height: 2, xf, link: None, placement: Default::default(), pixels: None };
        add(&mut d, Node::new(NodeId(0), NodeKind::Image(Box::new(im))));
    }
    add(&mut d, rect_node(Rect::new(100.0, 100.0, 150.0, 150.0), Color::BLACK));
    let written = |bytes: &[u8]| {
        let back = import(bytes).expect("import");
        let l = leaves(&back);
        let images: Vec<Rect> = l.iter().filter(|n| matches!(n.kind, NodeKind::Image(_))).filter_map(|n| n.geometric_bounds()).collect();
        (images, l.iter().filter(|n| n.path_data().is_some()).count())
    };
    let r = export_with_report(&d, &PdfOptions::default()).expect("export");
    assert_eq!(r.warnings, [NOTE]);
    let (images, paths) = written(&r.bytes);
    assert!(images.len() == 1 && close(images[0], Rect::new(20.0, 20.0, 60.0, 40.0), 0.01), "{images:?}");
    assert_eq!(paths, 1);
    let p = print(&d, &PrintOptions::default()).expect("print");
    assert_eq!(p.warnings.iter().filter(|w| *w == NOTE).count(), 1, "{:?}", p.warnings);
    assert_eq!(written(&p.bytes).0.len(), 1);
}

/// #2023: a group's Transform effect copies an image among its members as it does the paths: the
/// PDF draws the image three times, where the effect put it.
#[test]
fn a_group_transform_effect_exports_its_image_copies() {
    let mut d = doc(200.0, 100.0);
    d.images.insert("img1".into(), ImageBlob::new("image/png", tiny_png()));
    let im = ImageObject {
        key: "img1".into(),
        width: 4,
        height: 2,
        xf: Affine::translate((20.0, 30.0)) * Affine::scale(5.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let image = Arc::new(Node::new(d.alloc_id(), NodeKind::Image(Box::new(im))));
    let mut square = rect_node(Rect::new(20.0, 60.0, 30.0, 70.0), Color::BLACK);
    square.id = d.alloc_id();
    let mut g = Node::group(NodeId(0), vec![Arc::new(square), image]);
    g.appearance.effects.push(vectorcraft_doc::Effect {
        id: "distort.transform".into(),
        params: serde_json::json!({"moveH": 50, "copies": 2}),
        visible: true,
    });
    add(&mut d, g);
    let out = roundtrip(&d);
    let lefts: Vec<f64> =
        leaves(&out).iter().filter(|n| matches!(n.kind, NodeKind::Image(_))).filter_map(|n| n.geometric_bounds()).map(|b| b.x0.round()).collect();
    assert_eq!(lefts, [20.0, 70.0, 120.0]);
}

/// Build a PDF with a correct xref table from object bodies.
fn handmade_pdf(content: &str, media: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox {media} /Contents 4 0 R /Resources << >> >>"),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![];
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
    out
}

#[test]
fn import_handmade_pdf() {
    let content = "1 0 0 RG 0 0 1 rg 3 w 10 10 50 30 re B \
                   0 1 1 0 k 100 100 20 20 re f \
                   0.5 g q 1 0 0 1 150 0 cm 0 0 m 40 0 l 20 30 l h f Q";
    let bytes = handmade_pdf(content, "[0 0 200 300]");
    let r = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    let d = r.document;
    assert!(close(d.artboards[0].rect, Rect::new(0.0, 0.0, 200.0, 300.0), 1e-9));
    let l = leaves(&d);
    assert_eq!(l.len(), 3);
    // y-up PDF rect (10,10)-(60,40) → y-down (10,260)-(60,290).
    assert!(close(l[0].geometric_bounds().unwrap(), Rect::new(10.0, 260.0, 60.0, 290.0), 1e-6), "{:?}", l[0].geometric_bounds());
    assert_eq!(l[0].appearance.fill_paint().color().unwrap().to_hex(), "#0000ff");
    assert_eq!(l[0].appearance.stroke_paint().color().unwrap().to_hex(), "#ff0000");
    assert!((l[0].appearance.stroke_width() - 3.0).abs() < 1e-6);
    // CMYK red-ish (0,1,1,0) → approximately red through the CMYK profile.
    let [r_, g_, b_] = l[1].appearance.fill_paint().color().unwrap().to_rgb();
    assert!(r_ > 0.8 && g_ < 0.3 && b_ < 0.3, "{r_} {g_} {b_}");
    // Gray 0.5 and the `cm` translation.
    assert_eq!(l[2].appearance.fill_paint().color().unwrap().to_hex(), "#808080");
    assert!(close(l[2].geometric_bounds().unwrap(), Rect::new(150.0, 270.0, 190.0, 300.0), 1e-6));
}

/// Art off the page opens on the pasteboard round its artboard, not cut away (#472).
#[test]
fn import_keeps_art_outside_the_page() {
    let content = "0 0 1 rg 10 10 50 30 re f 0 1 0 rg -300 -300 50 50 re f 1 0 0 rg 500 500 20 20 re f";
    let d = import(&handmade_pdf(content, "[0 0 200 300]")).unwrap();
    assert!(close(d.artboards[0].rect, Rect::new(0.0, 0.0, 200.0, 300.0), 1e-9));
    let bounds: Vec<Rect> = leaves(&d).iter().map(|n| n.geometric_bounds().unwrap()).collect();
    let want = [Rect::new(10.0, 260.0, 60.0, 290.0), Rect::new(-300.0, 550.0, -250.0, 600.0), Rect::new(500.0, -220.0, 520.0, -200.0)];
    assert_eq!(bounds.len(), want.len(), "{bounds:?}");
    for (b, w) in bounds.iter().zip(want) {
        assert!(close(*b, w, 1e-6), "{b:?} vs {w:?}");
    }
}

#[test]
fn import_invalid_dash_array_strokes_solid() {
    // A dash array with a negative value is invalid (PDF 32000-1 §8.4.3.6): the line is solid. A valid
    // array next to it stays dashed.
    let content = "0 0 1 RG 4 w [-5 3] 0 d 10 10 m 90 10 l S [6 2] 1 d 10 50 m 90 50 l S";
    let d = import(&handmade_pdf(content, "[0 0 100 100]")).unwrap();
    let l = leaves(&d);
    assert_eq!(l.len(), 2);
    let dash = |n: &Node| match &n.appearance.items[..] {
        [.., AppearanceItem::Stroke(st)] => st.dash.clone(),
        items => panic!("no stroke: {items:?}"),
    };
    assert_eq!(dash(l[0]), None);
    assert_eq!(dash(l[1]).map(|d| d.pattern), Some(vec![6.0, 2.0]));
    // And the exporter never writes an invalid array back.
    let mut d2 = doc(100.0, 100.0);
    let mut n = rect_node(Rect::new(10.0, 10.0, 90.0, 90.0), Color::rgb(1.0, 0.0, 0.0));
    n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0).into();
    if let Some(AppearanceItem::Stroke(st)) = n.appearance.items.last_mut() {
        st.dash = Some(Dash { pattern: vec![-5.0, 3.0], offset: 0.0, align_corners: false });
    }
    add(&mut d2, n);
    let s = uncompressed(&d2);
    assert!(!s.contains("-5 3]") && !s.contains("[0 3]"), "invalid dash written:\n{s}");
}

#[test]
fn import_clip_from_handmade_pdf() {
    let content = "q 0 0 50 50 re W n 1 0 0 rg 0 0 100 100 re f Q";
    let d = import(&handmade_pdf(content, "[0 0 100 100]")).unwrap();
    let mut clip_groups = 0;
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Group { clip: true, .. }) {
            clip_groups += 1;
            assert!(close(n.geometric_bounds().unwrap(), Rect::new(0.0, 50.0, 50.0, 100.0), 1e-6));
        }
    });
    assert_eq!(clip_groups, 1);
}

#[test]
fn import_max_pages_and_multi_page() {
    let mut d = doc(200.0, 100.0);
    d.artboards.push(Artboard {
        id: 2,
        name: "B".into(),
        rect: Rect::new(300.0, 0.0, 400.0, 400.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });
    add(&mut d, rect_node(Rect::new(10.0, 10.0, 50.0, 50.0), Color::BLACK));
    add(&mut d, rect_node(Rect::new(310.0, 10.0, 350.0, 50.0), Color::BLACK));
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    let all = import(&bytes).unwrap();
    assert_eq!(all.artboards.len(), 2);
    assert_eq!(all.layers.len(), 2);
    assert!((all.artboards[1].rect.width() - 100.0).abs() < 1e-9 && (all.artboards[1].rect.height() - 400.0).abs() < 1e-9);
    // The second page's rect lands at the same place relative to its artboard.
    let second = leaves(&all)[1].geometric_bounds().unwrap();
    let ab = all.artboards[1].rect;
    assert!(close(second, Rect::new(ab.x0 + 10.0, 10.0, ab.x0 + 50.0, 50.0), 0.01), "{second:?}");
    let one = import_with_report(&bytes, &ImportOptions { max_pages: Some(1), ..Default::default() }).unwrap();
    assert_eq!(one.document.artboards.len(), 1);
    assert_eq!(leaves(&one.document).len(), 1);
}

#[test]
fn import_rejects_garbage() {
    assert!(matches!(import(b"not a pdf at all"), Err(PdfError::Parse(_)) | Err(PdfError::NoPages)));
}

/// Options with `settings` changed by `f`.
fn with(f: impl FnOnce(&mut PdfSettings)) -> PdfOptions {
    let mut o = PdfOptions::default();
    f(&mut o.settings);
    o
}

#[test]
fn compatibility_levels() {
    let mut d = doc(100.0, 100.0);
    add(&mut d, rect_node(Rect::new(10.0, 10.0, 50.0, 50.0), Color::rgb(1.0, 0.0, 0.0)));
    for (c, header) in [(Compatibility::Pdf14, "%PDF-1.4"), (Compatibility::Pdf16, "%PDF-1.6"), (Compatibility::Pdf20, "%PDF-2.0")] {
        let b = export(&d, &with(|s| s.compatibility = c)).unwrap();
        assert!(b.starts_with(header.as_bytes()), "{c:?}");
    }
    let a = export(&d, &with(|s| s.standard = Standard::PdfA2b)).unwrap();
    assert!(String::from_utf8_lossy(&a).contains("pdfaid"), "PDF/A identification in XMP");
    // PDF/X files are PDF 1.3 (PDF/X-1a, PDF/X-3) or at most 1.6 (PDF/X-4): not the default 1.7.
    for (x, header) in [(Standard::PdfX1a, "%PDF-1.3"), (Standard::PdfX3, "%PDF-1.3"), (Standard::PdfX4, "%PDF-1.6")] {
        assert!(matches!(export(&d, &with(|s| s.standard = x)), Err(PdfError::BadSetting(_))), "{x:?}");
        let b = export(&d, &with(|s| (s.standard, s.compatibility) = (x, x.version()))).unwrap();
        assert!(b.starts_with(header.as_bytes()), "{x:?}");
    }
    // PDF/A-2b is a PDF 1.7 standard.
    let e = export(&d, &with(|s| (s.standard, s.compatibility) = (Standard::PdfA2b, Compatibility::Pdf20)));
    assert!(matches!(e, Err(PdfError::BadSetting(_))), "{e:?}");
}

#[test]
fn compress_option_changes_output() {
    let mut d = doc(100.0, 100.0);
    for i in 0..20 {
        add(&mut d, rect_node(Rect::new(i as f64, i as f64, 50.0 + i as f64, 50.0), Color::BLACK));
    }
    let c = export(&d, &PdfOptions::default()).unwrap();
    let u = export(&d, &PdfOptions::uncompressed()).unwrap();
    assert!(u.len() > c.len());
    assert!(String::from_utf8_lossy(&u).contains(" re") || String::from_utf8_lossy(&u).contains(" l"));
}

#[test]
fn stroke_alignment_and_arrowheads_export() {
    let mut d = doc(200.0, 200.0);
    let mut p = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 120.0)),
        Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 6.0),
    );
    p.appearance.stroke_mut().unwrap().align = vectorcraft_doc::StrokeAlign::Inside;
    add(&mut d, p);
    let mut line = Node::path(
        NodeId(0),
        shapes::line(Point::new(10.0, 150.0), Point::new(150.0, 150.0)),
        Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0),
    );
    line.appearance.stroke_mut().unwrap().end_arrow = Some(vectorcraft_doc::Arrowhead::Triangle);
    add(&mut d, line);
    let r = export_with_report(&d, &PdfOptions::default()).unwrap();
    let out = import(&r.bytes).unwrap();
    let mut widths = vec![];
    out.walk(|n| {
        if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
            widths.push(s.width);
        }
    });
    assert!(widths.iter().any(|w| (w - 12.0).abs() < 1e-3), "inside stroke doubled and clipped: {widths:?}");
    // The arrowhead is an extra filled object.
    assert!(leaves(&out).len() >= 3);
}

#[test]
fn creation_date_written() {
    let d = doc(100.0, 100.0);
    // 2024-02-29 12:34:56 UTC
    let bytes = export(&d, &PdfOptions { created: Some(1_709_210_096), ..PdfOptions::uncompressed() }).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("D:20240229123456"));
}

#[test]
fn a_later_modification_date_is_written_beside_the_creation_date() {
    let d = doc(100.0, 100.0);
    // 2020-01-01 00:00:00 and 2026-10-17 14:03:00 UTC.
    let (created, modified) = (1_577_836_800, 1_792_245_780);
    for standard in [Standard::None, Standard::PdfX1a, Standard::PdfX3, Standard::PdfX4, Standard::PdfA2b] {
        let mut o = PdfOptions { created: Some(created), modified: Some(modified), ..PdfOptions::uncompressed() };
        (o.settings.standard, o.settings.compatibility) = (standard, standard.version());
        let bytes = export(&d, &o).unwrap();
        let t = String::from_utf8_lossy(&bytes);
        assert!(t.contains("/CreationDate(D:20200101000000Z)") && t.contains("/ModDate(D:20261017140300Z)"), "{standard:?}: document information");
        let info = file_info(&bytes, None).unwrap();
        assert_eq!((info.created, info.modified), (Some(created), Some(modified)), "{standard:?}: XMP");
        // PDF/A files record when they were saved and converted: at the time of writing too.
        assert_eq!(t.contains("<stEvt:when>2026-10-17T14:03:00"), standard == Standard::PdfA2b, "{standard:?}");
        assert!(!t.contains("<stEvt:when>2020"), "{standard:?}");
        assert_eq!(import(&bytes).unwrap().artboards.len(), 1, "{standard:?}: the file reads back");
    }
    // Without a modification date, the file is modified when it was created.
    let bytes = export(&d, &PdfOptions { created: Some(created), ..PdfOptions::uncompressed() }).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("/ModDate(D:20200101000000Z)"));
    assert_eq!(file_info(&bytes, None).unwrap().modified, Some(created));
}

#[test]
fn opacity_mask_exports_as_luminosity_soft_mask() {
    let mut d = doc(100.0, 100.0);
    let mut n = rect_node(Rect::new(10.0, 10.0, 90.0, 90.0), Color::rgb(1.0, 0.0, 0.0));
    let art = rect_node(Rect::new(10.0, 10.0, 50.0, 90.0), Color::WHITE);
    n.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
    add(&mut d, n);
    let bytes = export(&d, &PdfOptions::uncompressed()).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/SMask"), "soft mask in an ExtGState");
    assert!(text.contains("/Luminosity"));
}

#[test]
fn empty_and_populated_file_info_preserve_key_presence() {
    let mut d = doc(100.0, 100.0);
    d.title.clear();
    let s = uncompressed(&d);
    for key in ["/Title", "/Author", "/Keywords", "<dc:title>", "<dc:creator>", "<pdf:Keywords>"] {
        assert!(!s.contains(key), "unexpected empty metadata {key}");
    }
    assert!(!s.contains("/DisplayDocTitle true"), "the default export does not request displaying its metadata title");
    d.title = "Synthetic title".into();
    d.metadata.author = " Test author ".into();
    d.metadata.keywords = vec!["test keyword".into()];
    let bytes = export(&d, &PdfOptions { display_doc_title: true, created: Some(1_700_000_000), ..PdfOptions::uncompressed() }).unwrap();
    let s = String::from_utf8_lossy(&bytes);
    for key in ["/Title", "/Author", "/Keywords", "/CreationDate", "<dc:title>", "<dc:creator>", "<pdf:Keywords>"] {
        assert!(s.contains(key), "missing populated metadata {key}");
    }
    assert!(s.contains("Synthetic title") && s.contains("Test author") && s.contains("test keyword"));
    assert!(s.contains("/ViewerPreferences<</DisplayDocTitle true>>"), "viewers should show the document title (#3646)");
}

/// A function or colour space built on itself is refused before the interpreter builds it (by
/// recursion, with no cap: a stack overflow), and those of real files are read.
#[test]
fn functions_and_colour_spaces_built_on_themselves_are_refused() {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};
    let first = first_extra(1);
    let (me, next) = (format!("{first} 0 R"), format!("{} 0 R", first + 1));
    let shaded = PdfPage {
        resources: format!("/Shading << /S << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function {me} >> >>"),
        ..PdfPage::new(100.0, 100.0, "/S sh")
    };
    let filled = PdfPage { resources: format!("/ColorSpace << /C {me} >>"), ..PdfPage::new(100.0, 100.0, "/C cs 0 sc 0 0 50 50 re f") };
    let stitch = |on: &str| format!("<< /FunctionType 3 /Domain [0 1] /Functions [{on}] /Bounds [] /Encode [0 1] >>");
    let indexed = |on: &str| format!("[/Indexed {on} 1 <000000ffffff>]");
    let ramp = "<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>";
    // `n` functions, each stitching the next, then a plain one.
    let chain = |n: usize| (1..=n).map(|i| stitch(&format!("{} 0 R", first + i))).chain([ramp.to_string()]).collect::<Vec<_>>();
    let calculator =
        |program: &str| format!("<< /FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n{program}\nendstream", program.len());
    for (what, page, objects) in [
        ("a function stitching itself", &shaded, vec![stitch(&me)]),
        ("two functions stitching each other", &shaded, vec![stitch(&next), stitch(&me)]),
        ("twenty functions, each stitching the next", &shaded, chain(20)),
        ("procedures in procedures", &shaded, vec![calculator(&format!("{}{}", "{".repeat(100), "}".repeat(100)))]),
        ("a colour space on itself", &filled, vec![indexed(&me)]),
        ("two colour spaces on each other", &filled, vec![indexed(&next), indexed(&me)]),
        // Its leading items are read by the interpreter all the same: an item that can't be read
        // (a stream of an external file) after them doesn't hide them.
        ("a function stitching itself, then an item that can't be read", &shaded, vec![stitch(&format!("{me} << /F (x) >> stream"))]),
        ("a colour space on itself, then an item that can't be read", &filled, vec![format!("[/Indexed {me} 1 <000000ffffff> << /F (x) >> stream]")]),
    ] {
        let objects: Vec<&str> = objects.iter().map(String::as_str).collect();
        let e = import(&pdf_with(std::slice::from_ref(page), &objects, None)).err().map(|e| e.to_string());
        // A damaged array is refused as damaged: what it holds can't be looked at whole.
        let refused = |e: &str| e.contains("built on one another without end") || e.contains("is damaged and can't be read");
        assert!(e.as_deref().is_some_and(refused), "{what}: {e:?}");
    }
    for (what, page, objects) in [
        ("a function stitching one", &shaded, vec![stitch(&next), ramp.to_string()]),
        ("three functions, each stitching the next", &shaded, chain(3)),
        ("a calculator function", &shaded, vec![calculator("{ dup 1 exch sub 0 }")]),
        ("an indexed colour space", &filled, vec![indexed("/DeviceRGB")]),
    ] {
        let objects: Vec<&str> = objects.iter().map(String::as_str).collect();
        let d = import(&pdf_with(std::slice::from_ref(page), &objects, None)).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(d.layers.iter().any(|l| l.children().is_some_and(|c| !c.is_empty())), "{what}: its art is read");
    }
}

/// A dictionary value the reader can't parse (a name with a broken `#` escape, a number with a
/// stray `-`) is left out of the nesting check, which goes on with the rest of the file: a file
/// whose shading has a plain function is read, and one whose function stitches itself is refused.
#[test]
fn dictionary_values_the_reader_cannot_parse_are_skipped() {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};
    let me = format!("{} 0 R", first_extra(1));
    let shaded = PdfPage {
        resources: format!("/Shading << /S << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function {me} >> >>"),
        ..PdfPage::new(100.0, 100.0, "/S sh")
    };
    let ramp = "<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>";
    let stitch = format!("<< /FunctionType 3 /Domain [0 1] /Functions [{me}] /Bounds [] /Encode [0 1] >>");
    for junk in ["<< /A /#zz >>", "<< /A 12-.5 >>"] {
        let [plain, looped] = [ramp, stitch.as_str()].map(|f| pdf_with(std::slice::from_ref(&shaded), &[f, junk], None));
        assert_eq!(info(&plain, None).map(|i| i.pages.len()).ok(), Some(1), "{junk}");
        let d = import(&plain).unwrap_or_else(|e| panic!("{junk}: {e}"));
        assert!(d.layers.iter().any(|l| l.children().is_some_and(|c| !c.is_empty())), "{junk}: its art is read");
        let e = import(&looped).err().map(|e| e.to_string());
        assert!(e.as_deref().is_some_and(|e| e.contains("built on one another without end")), "{junk}: {e:?}");
    }
}

/// Illustrator's documentation describes dither as the gradient converted to pixels with subtle
/// noise, at the Document Raster Effects resolution
/// (https://helpx.adobe.com/illustrator/desktop/paint-and-fill/create-and-edit-gradients/control-dither-in-gradients.html):
/// a dithered gradient fill is drawn as an image at that resolution, clipped to the shape; a
/// stroke (dither applies to fills only) and an undithered fill stay shadings.
#[test]
fn dithered_gradient_fills_export_as_images_and_strokes_as_shadings() {
    let grad = |dither| Paint::Gradient(Box::new(GradientPaint::new(Gradient { dither, ..Default::default() })));
    let shape = shapes::rectangle(Rect::new(10.0, 10.0, 110.0, 60.0));
    let mut d = doc(200.0, 100.0);
    d.raster_effects_ppi = 144.0;
    add(&mut d, Node::path(NodeId(0), shape.clone(), Appearance::basic(grad(true), grad(true), 4.0)));
    let pdf = uncompressed(&d).replace(' ', "");
    assert!(pdf.contains("/Subtype/Image"), "the fill is an image");
    assert!(pdf.contains("/Width200") && pdf.contains("/Height100"), "two pixels a point at 144 ppi");
    assert!(pdf.contains("/ShadingType2"), "the stroke keeps its shading");
    let mut plain = doc(200.0, 100.0);
    add(&mut plain, Node::path(NodeId(0), shape, Appearance::basic(grad(false), Paint::None, 0.0)));
    let pdf = uncompressed(&plain).replace(' ', "");
    assert!(!pdf.contains("/Subtype/Image") && pdf.contains("/ShadingType2"));
}

/// Illustrator's documentation: perceptual gradients aren't supported in PDF exports, and are
/// rasterised when saved that way
/// (https://helpx.adobe.com/illustrator/desktop/paint-and-fill/create-and-edit-gradients/control-perceptual-interpolation-in-gradients.html).
/// A Perceptual fill and stroke are drawn as images at the raster effects resolution, clipped to
/// what they paint; a Classic gradient stays a shading, and so does a gradient along a stroke
/// (slices through stops on the perceptual curve, as the canvas draws it).
#[test]
fn perceptual_gradients_export_as_images_and_classic_ones_as_shadings() {
    let grad = |method| Paint::Gradient(Box::new(GradientPaint::new(Gradient { method, ..Default::default() })));
    let p = grad(vectorcraft_color::GradientMethod::Perceptual);
    let shape = shapes::rectangle(Rect::new(10.0, 10.0, 110.0, 60.0));
    let mut d = doc(200.0, 100.0);
    d.raster_effects_ppi = 144.0;
    add(&mut d, Node::path(NodeId(0), shape.clone(), Appearance::basic(p.clone(), p.clone(), 4.0)));
    let pdf = uncompressed(&d).replace(' ', "");
    assert_eq!(pdf.matches("/Subtype/Image").count(), 2, "the fill and the stroke are images");
    assert!(pdf.contains("/Width200") && pdf.contains("/Height100"), "two pixels a point at 144 ppi");
    assert!(!pdf.contains("/ShadingType2"), "no shading");
    let mut along = Appearance::basic(Paint::None, p, 4.0);
    if let Some(AppearanceItem::Stroke(st)) = along.items.get_mut(1) {
        st.gradient_mode = vectorcraft_doc::StrokeGradientMode::Along;
    }
    let mut a = doc(200.0, 100.0);
    add(&mut a, Node::path(NodeId(0), shape.clone(), along));
    let pdf = uncompressed(&a).replace(' ', "");
    assert!(!pdf.contains("/Subtype/Image") && pdf.contains("/ShadingType2"), "along the stroke: slices");
    let c = grad(vectorcraft_color::GradientMethod::Classic);
    let mut plain = doc(200.0, 100.0);
    add(&mut plain, Node::path(NodeId(0), shape, Appearance::basic(c.clone(), c, 4.0)));
    let pdf = uncompressed(&plain).replace(' ', "");
    assert!(!pdf.contains("/Subtype/Image") && pdf.contains("/ShadingType2"));
}

/// A dithered Perceptual gradient: the fill and the stroke are images (Perceptual), and dither
/// adds its noise to the fill's pixels only.
#[test]
fn dithered_perceptual_gradients_export_as_images_with_noise_on_the_fill() {
    let grad = |dither| {
        Paint::Gradient(Box::new(GradientPaint::new(Gradient {
            dither,
            method: vectorcraft_color::GradientMethod::Perceptual,
            ..Default::default()
        })))
    };
    let shape = shapes::rectangle(Rect::new(10.0, 10.0, 110.0, 60.0));
    let export_with = |dither, stroke: bool| {
        let mut d = doc(200.0, 100.0);
        let (fill, stroke) = if stroke { (Paint::None, grad(dither)) } else { (grad(dither), Paint::None) };
        add(&mut d, Node::path(NodeId(0), shape.clone(), Appearance::basic(fill, stroke, 4.0)));
        uncompressed(&d).replace(' ', "")
    };
    for stroke in [false, true] {
        let pdf = export_with(true, stroke);
        assert_eq!(pdf.matches("/Subtype/Image").count(), 1, "stroke {stroke}: an image");
        assert!(!pdf.contains("/ShadingType"), "stroke {stroke}: no shading");
    }
    assert_ne!(export_with(true, false), export_with(false, false), "the fill's pixels carry the noise");
    assert_eq!(export_with(true, true), export_with(false, true), "a stroke has no dither");
}

/// COLR glyphs in the PDF: as real text, in their font's Type 3 glyphs (the foreground layer in
/// the fill); outlined, as vector art, a sweep glyph as an image.
#[test]
fn colr_glyphs_export_as_vector_art() {
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, COLR_SWEEP_CHAR, colr_family, colr_font};
    for v1 in [false, true] {
        vectorcraft_text::FontDb::global().add_font(colr_font(v1).unwrap());
    }
    let page = |v1: bool, ch: char, outline_text: bool| {
        let mut d = doc(300.0, 100.0);
        let fill = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
        let st = CharStyle { font_family: colr_family(v1).into(), size: 24.0, fill, ..CharStyle::default() };
        add(&mut d, Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 50.0), &ch.to_string(), st)))));
        let mut o = PdfOptions::uncompressed();
        o.settings.advanced.outline_text = outline_text;
        import_with_report(&export(&d, &o).unwrap(), &ImportOptions::default()).unwrap().document
    };
    let fills = |d: &Document| -> Vec<Paint> { leaves(d).iter().map(|n| n.appearance.fill_paint()).collect() };
    let near =
        |p: &Paint, rgb: [f32; 3]| matches!(p, Paint::Solid { color, .. } if color.to_rgb().iter().zip(rgb).all(|(a, b)| (a - b).abs() < 0.02));
    for outline_text in [false, true] {
        let v0 = fills(&page(false, COLOUR_CHAR, outline_text));
        assert!(v0.iter().any(|p| near(p, [1.0, 0.0, 0.0])), "the red layer (outlined {outline_text}): {v0:?}");
        assert!(v0.iter().any(|p| near(p, [0.0, 0.0, 1.0])), "the foreground layer in the fill (outlined {outline_text}): {v0:?}");
    }
    let v1 = page(true, COLOUR_CHAR, true);
    assert!(!leaves(&v1).iter().any(|n| matches!(n.kind, NodeKind::Image(_))), "vector art");
    assert!(fills(&v1).iter().any(|p| matches!(p, Paint::Gradient(_))), "with its gradients");
    let sweep = page(true, COLR_SWEEP_CHAR, true);
    assert!(leaves(&sweep).iter().any(|n| matches!(n.kind, NodeKind::Image(_))), "a sweep glyph is an image");
}
