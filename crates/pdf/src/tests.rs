use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, CharStyle, Dash, Document, ImageBlob, ImageObject, LineCap, LineJoin, Node, NodeId, NodeKind, TextObject,
};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect, shapes};

use crate::*;

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

fn roundtrip(d: &Document) -> Document {
    let bytes = export(d, &PdfOptions::default()).expect("export");
    import(&bytes).expect("import")
}

fn uncompressed(d: &Document) -> String {
    let bytes = export(d, &PdfOptions::uncompressed()).expect("export");
    String::from_utf8_lossy(&bytes).into_owned()
}

fn page_count(bytes: &[u8]) -> usize {
    hayro_syntax::Pdf::new(bytes.to_vec()).expect("parse").pages().len()
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
        show_center_mark: false,
        show_cross_hairs: false,
        ..Default::default()
    });
    d.artboards.push(Artboard {
        id: 3,
        name: "C".into(),
        rect: Rect::new(0.0, 500.0, 612.0, 1292.0),
        show_center_mark: false,
        show_cross_hairs: false,
        ..Default::default()
    });
    let bytes = export(&d, &PdfOptions::default()).unwrap();
    let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
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
        show_center_mark: false,
        show_cross_hairs: false,
        ..Default::default()
    });
    let bytes = export(&d, &PdfOptions { artboards: Some(vec![1]), ..Default::default() }).unwrap();
    let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
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
    let s = uncompressed(&d);
    assert!(!s.contains("/Font"), "no fonts when outlining text");
    let r = import_with_report(&export(&d, &PdfOptions::default()).unwrap(), &ImportOptions::default()).unwrap();
    let l = leaves(&r.document);
    assert!(!l.is_empty());
    assert!(l.iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    let b = l.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.geometric_bounds())).unwrap();
    assert!(b.x0 >= 9.0 && b.x1 < 120.0 && b.y1 <= 56.0 && b.y0 > 25.0, "{b:?}");
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
    };
    add(&mut d, Node::new(NodeId(0), NodeKind::Image(im)));
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
    n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
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
        show_center_mark: false,
        show_cross_hairs: false,
        ..Default::default()
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
fn opacity_mask_exports_as_luminosity_soft_mask() {
    let mut d = doc(100.0, 100.0);
    let mut n = rect_node(Rect::new(10.0, 10.0, 90.0, 90.0), Color::rgb(1.0, 0.0, 0.0));
    let art = rect_node(Rect::new(10.0, 10.0, 50.0, 90.0), Color::WHITE);
    n.mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
    add(&mut d, n);
    let bytes = export(&d, &PdfOptions::uncompressed()).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/SMask"), "soft mask in an ExtGState");
    assert!(text.contains("/Luminosity"));
}
