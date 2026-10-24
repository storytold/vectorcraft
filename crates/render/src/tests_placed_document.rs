//! Placed documents on screen: drawn from bitmaps made in the background (a placeholder until
//! then), the same as drawn exactly; exports draw them exactly at once.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ImageBlob, LinkInfo, Node, NodeId, NodeKind, PlacedDocument};
use vectorcraft_geom::shapes;

use super::*;

/// Every file reads as a 40×20 artboard filled blue; a file whose bytes start with `loop` also
/// places itself (the same key) in its middle.
fn blue_page(p: &PlacedDocument, bytes: &[u8]) -> Option<(Document, Node, Rect)> {
    let mut d = Document::new(40.0, 20.0);
    d.layers.clear();
    let blue = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
    let mut children = vec![Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)), blue))];
    if bytes.starts_with(b"loop") {
        d.images.insert(p.key.clone(), ImageBlob::new("application/json", bytes.to_vec()));
        let me = PlacedDocument { xf: Affine::translate((10.0, 5.0)) * Affine::scale(0.5), ..p.clone() };
        children.push(Arc::new(Node::new(NodeId(2), NodeKind::PlacedDocument(Box::new(me)))));
    }
    Some((d, Node::new(NodeId(0), NodeKind::Group { children, clip: false }), Rect::new(0.0, 0.0, 40.0, 20.0)))
}

/// A 100×100 document showing one placed file (`bytes` under `key`) in the box (10, 10)–(90, 50).
fn doc_with(key: &str, bytes: &[u8]) -> Document {
    vectorcraft_doc::placed_document::set_loader(blue_page);
    let mut d = Document::new(100.0, 100.0);
    d.images.insert(key.into(), ImageBlob::new("application/json", bytes.to_vec()));
    let p = PlacedDocument {
        link: LinkInfo::new("/art/card.vectorcraft"),
        key: key.into(),
        bounding: false,
        page_box: Default::default(),
        width: 40.0,
        height: 20.0,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale(2.0),
        placement: Default::default(),
    };
    let l = d.layers[0].id;
    d.insert(Some(l), 0, Node::new(NodeId(50), NodeKind::PlacedDocument(Box::new(p)))).unwrap();
    d
}

fn doc(key: &str) -> Document {
    doc_with(key, format!("{{\"doc\": \"{key}\"}}").as_bytes())
}

fn render(d: &Document, progressive: bool) -> Rendered {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), progressive_placed: progressive, ..Default::default() };
    Renderer::new().render(d, 100, 100, Affine::IDENTITY, &opts)
}

/// Wait for the background bitmaps (at most 10 s).
fn settle() {
    let t = std::time::Instant::now();
    while placed_document::busy() && t.elapsed().as_secs() < 10 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!placed_document::busy(), "bitmaps still coming");
}

#[test]
fn a_placed_document_draws_exactly_with_its_own_art() {
    let d = doc("exact-1");
    let r = render(&d, false);
    assert_eq!(&r.pixel(50, 30)[..3], &[0, 0, 255], "blue inside the box");
    assert_eq!(&r.pixel(95, 95)[..3], &[255, 255, 255], "nothing outside");
}

#[test]
fn an_enveloped_placed_document_shows_its_file_once_read() {
    let mut d = doc("enveloped-1");
    let placed = d.remove(NodeId(50)).unwrap();
    let NodeKind::PlacedDocument(p) = &placed.kind else { panic!("a placed document") };
    let p = (**p).clone();
    let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
    let envelope = NodeKind::Envelope {
        content: vec![placed],
        kind: Box::new(kind),
        fidelity: 50.0,
        editing: false,
        options: Default::default(),
        frame: Box::new(Affine::IDENTITY),
    };
    let l = d.layers[0].id;
    d.insert(Some(l), 0, Node::new(NodeId(60), envelope)).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 100, 100, Affine::IDENTITY, &opts).pixels;
    // Exporting the document as PDF, SVG or EPS reads the file.
    d.with_placed_art();
    assert!(vectorcraft_doc::placed_document::read_cached(&p).is_some(), "the file is read");
    let after = r.render(&d, 100, 100, Affine::IDENTITY, &opts).pixels;
    assert!(after != before, "the file's art shows");
    assert!(after == Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

#[test]
fn a_placed_document_with_a_drop_shadow_shows_its_file_once_read() {
    let mut d = doc("shadowed-1");
    let n = d.node_mut(NodeId(50)).unwrap();
    let params = serde_json::json!({"x": 4, "y": 4, "blur": 1, "opacity": 100});
    n.appearance.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params, visible: true });
    let NodeKind::PlacedDocument(p) = &n.kind else { panic!("a placed document") };
    let p = (**p).clone();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 100, 100, Affine::IDENTITY, &opts).pixels;
    // Exporting the document as PDF, SVG or EPS reads the file.
    d.with_placed_art();
    assert!(vectorcraft_doc::placed_document::read_cached(&p).is_some(), "the file is read");
    let after = r.render(&d, 100, 100, Affine::IDENTITY, &opts).pixels;
    assert!(after != before, "the file's art shows");
    assert!(after == Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

#[test]
fn on_screen_a_placed_document_shows_a_placeholder_then_its_bitmap() {
    let d = doc("progressive-1");
    let generation = placed_document::generation();
    let first = render(&d, true);
    let shown = &first.pixel(50, 30)[..3];
    assert!(shown == [0, 0, 255] || shown != [255, 255, 255], "a placeholder or the art: {shown:?}");
    settle();
    assert!(placed_document::generation() > generation || shown == [0, 0, 255], "a bitmap was made");
    let later = render(&d, true);
    let exact = render(&d, false);
    assert_eq!(&later.pixel(50, 30)[..3], &[0, 0, 255]);
    for (x, y) in [(12, 12), (50, 30), (88, 48), (95, 95), (5, 5)] {
        let (a, b) = (later.pixel(x, y), exact.pixel(x, y));
        assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 24), "at ({x}, {y}): {a:?} vs {b:?}");
    }
}

#[test]
fn copies_of_one_placed_document_share_its_bitmap() {
    let mut d = doc("shared-1");
    let n = d.node(NodeId(50)).unwrap().clone();
    let l = d.layers[0].id;
    let mut copy = n.clone();
    copy.id = NodeId(51);
    copy.transform(Affine::translate((0.0, 45.0)), false);
    d.insert(Some(l), 1, copy).unwrap();
    render(&d, true);
    settle();
    let r = render(&d, true);
    assert!(!placed_document::pending_for("shared-1"), "nothing more to make");
    assert_eq!(&r.pixel(50, 30)[..3], &[0, 0, 255]);
    assert_eq!(&r.pixel(50, 75)[..3], &[0, 0, 255]);
}

#[test]
fn a_placed_document_kept_as_its_preview_draws_the_preview() {
    let mut d = doc("preview-1");
    // Only the preview (a 20×10 green PNG) is there, and the file can't be read here.
    let img = image::RgbaImage::from_pixel(20, 10, image::Rgba([0, 200, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut blob = ImageBlob::new("image/png", png);
    blob.proxy = Some(blob.bytes.clone());
    d.images.insert("preview-1".into(), blob);
    let exact = render(&d, false);
    assert_eq!(&exact.pixel(50, 30)[..3], &[0, 200, 0], "the preview, in the box");
    assert_eq!(&exact.pixel(95, 95)[..3], &[255, 255, 255]);
    // On screen past the preview's resolution (it has 0.5 px a point; this is 2): still the preview.
    render(&d, true);
    settle();
    let shown = render(&d, true);
    assert_eq!(&shown.pixel(50, 30)[..3], &[0, 200, 0]);
}

#[test]
fn a_document_that_places_itself_draws_a_few_levels_deep_then_stops() {
    let d = doc_with("loop-1", b"loop");
    let t = std::time::Instant::now();
    let r = render(&d, false);
    assert!(t.elapsed().as_secs() < 10);
    assert_eq!(&r.pixel(50, 30)[..3], &[0, 0, 255]);
    // On screen too.
    render(&d, true);
    settle();
    assert_eq!(&render(&d, true).pixel(50, 30)[..3], &[0, 0, 255]);
}
