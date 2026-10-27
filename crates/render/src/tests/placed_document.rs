//! Placed documents on screen: drawn from bitmaps made in the background (a placeholder until
//! then), the same as drawn exactly; exports draw them exactly at once.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ImageBlob, LinkInfo, Node, NodeId, NodeKind, PlacedDocument};
use vectorcraft_geom::shapes;

use crate::*;

/// Every file reads as a 40×20 artboard filled blue; a file whose bytes start with `loop` also
/// places itself (the same key) in its middle, and one whose bytes start with `bad` can't be read.
fn blue_page(p: &PlacedDocument, bytes: &[u8]) -> Option<(Document, Node, Rect)> {
    if bytes.starts_with(b"bad") {
        return None;
    }
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
    let l = d.layers[0].id;
    d.insert(Some(l), 0, placed_node(50, key, 40.0, 20.0, Affine::translate((10.0, 10.0)) * Affine::scale(2.0))).unwrap();
    d
}

/// Object `id` showing file `key` in a `width`×`height` box placed by `xf`.
fn placed_node(id: u64, key: &str, width: f64, height: f64, xf: Affine) -> Node {
    let p = PlacedDocument {
        link: LinkInfo::new("/art/card.vectorcraft"),
        key: key.into(),
        bounding: false,
        page_box: Default::default(),
        width,
        height,
        xf,
        placement: Default::default(),
    };
    Node::new(NodeId(id), NodeKind::PlacedDocument(Box::new(p)))
}

/// Held by the tests that draw placed documents from bitmaps, which share the bitmaps, their
/// generation and the jobs being made: one test's bitmaps don't push out another's, and
/// [`settle`] waits for its own.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
    let _serial = serial();
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
    let _serial = serial();
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

/// Nine files on screen, each drawn from a 4096 × 4096 bitmap (576 MiB together, past the 512 MiB
/// the bitmaps keep): once made, the next frames draw all nine and ask for none again.
#[test]
fn nine_large_bitmaps_on_screen_are_not_made_again() {
    let _serial = serial();
    vectorcraft_doc::placed_document::set_loader(blue_page);
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let keys: Vec<String> = (0..9).map(|i| format!("nine-{i}")).collect();
    for (i, key) in keys.iter().enumerate() {
        d.images.insert(key.clone(), ImageBlob::new("application/json", format!("{{\"doc\": \"{key}\"}}").into_bytes()));
        // A 1 × 1 point box at 4096 pixels a point: a 4096-pixel bitmap.
        d.insert(Some(l), i, placed_node(100 + i as u64, key, 1.0, 1.0, Affine::translate((i as f64 * 0.001, 0.0)))).unwrap();
    }
    let (view, opts) = (Affine::scale(4096.0), RenderOptions { progressive_placed: true, ..Default::default() });
    let mut r = Renderer::new();
    r.render(&d, 32, 32, view, &opts);
    settle();
    let asked = || keys.iter().map(|k| placed_document::scheduled(k)).collect::<Vec<_>>();
    assert_eq!(asked(), vec![1; 9], "each asked for once");
    for frame in 1..=2 {
        let shown = r.render(&d, 32, 32, view, &opts);
        assert_eq!(asked(), vec![1; 9], "frame {frame} asks for none again");
        assert_eq!(&shown.pixel(16, 16)[..3], &[0, 0, 255], "frame {frame} draws the art");
    }
}

/// A bitmap that was not kept (it would have taken the bitmaps past the most they may hold) is
/// not asked for again: its file draws exactly at that zoom step, at once.
#[test]
fn a_bitmap_not_kept_draws_exactly_without_being_asked_for() {
    let _serial = serial();
    let d = doc("unkept-1");
    // The box is drawn at 2 pixels a point: zoom step 2.
    placed_document::mark_unkept("unkept-1", 2.0);
    let shown = render(&d, true);
    assert_eq!(&shown.pixel(50, 30)[..3], &[0, 0, 255], "the art, drawn exactly");
    assert_eq!(placed_document::scheduled("unkept-1"), 0, "not asked for");
}

/// A file that can't be read, without a preview: its bitmap is asked for once, the screen is
/// not told to draw again, and later frames draw the placeholder without asking again.
#[test]
fn a_bitmap_that_cant_be_made_is_not_asked_for_again() {
    let _serial = serial();
    let d = doc_with("unreadable-1", b"bad");
    let generation = placed_document::generation();
    render(&d, true);
    settle();
    assert_eq!(placed_document::generation(), generation, "nothing new to draw");
    let later = render(&d, true);
    assert_eq!(placed_document::scheduled("unreadable-1"), 1, "asked for once");
    let shown = &later.pixel(50, 30)[..3];
    assert!(shown != [255, 255, 255] && shown != [0, 0, 255], "the placeholder: {shown:?}");
}

/// Once bitmaps are ready, drawing again where [`placed_area`] says gives what drawing the whole
/// frame again gives. The area of a file covers every object showing it (a group's member and
/// an object with a drop shadow too) and no object showing another file; art of a brush or a
/// symbol showing it makes it everything.
#[test]
fn a_ready_bitmap_draws_again_where_its_file_shows() {
    let _serial = serial();
    vectorcraft_doc::placed_document::set_loader(blue_page);
    let mut d = Document::new(100.0, 100.0);
    for key in ["area-a", "area-b"] {
        d.images.insert(key.into(), ImageBlob::new("application/json", format!("{{\"doc\": \"{key}\"}}").into_bytes()));
    }
    let l = d.layers[0].id;
    // 20 × 10 point boxes: A loose at (5, 5), with a shadow at (30, 40) and in a group at (5, 80);
    // B at (75, 5). Only the shadow reaches past x = 50, and only the group's member past y = 50.
    let at = |x: f64, y: f64| Affine::translate((x, y)) * Affine::scale(0.5);
    d.insert(Some(l), 0, placed_node(60, "area-a", 40.0, 20.0, at(5.0, 5.0))).unwrap();
    let group = NodeKind::Group { children: vec![Arc::new(placed_node(61, "area-a", 40.0, 20.0, at(5.0, 80.0)))], clip: false };
    d.insert(Some(l), 1, Node::new(NodeId(62), group)).unwrap();
    let mut shadowed = placed_node(63, "area-a", 40.0, 20.0, at(30.0, 40.0));
    let params = serde_json::json!({"x": 4, "y": 4, "blur": 1, "opacity": 100});
    shadowed.appearance.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params, visible: true });
    d.insert(Some(l), 2, shadowed).unwrap();
    d.insert(Some(l), 3, placed_node(64, "area-b", 40.0, 20.0, at(75.0, 5.0))).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), progressive_placed: true, ..Default::default() };
    let mut r = Renderer::new();
    let generation = placed_document::generation();
    let before = r.render(&d, 100, 100, Affine::IDENTITY, &opts);
    settle();
    let made = placed_document::made_between(generation, placed_document::generation()).expect("known");
    let files = |keys: &[&str]| keys.iter().map(|k| k.to_string()).collect::<std::collections::HashSet<_>>();
    assert_eq!(made, files(&["area-a", "area-b"]), "both files' bitmaps");
    let only_a = files(&["area-a"]);
    let Changed::Area(a) = placed_area(&d, &only_a) else { panic!("an area") };
    for (x, y) in [(6.0, 6.0), (24.0, 14.0), (31.0, 41.0), (53.0, 53.0), (6.0, 81.0), (24.0, 89.0)] {
        assert!(a.contains(vectorcraft_geom::Point::new(x, y)), "({x}, {y}) in {a:?}");
    }
    assert!(a.x1 < 75.0, "not B: {a:?}");
    // Drawn again only there, it is what the whole frame drawn again shows.
    let Changed::Area(area) = placed_area(&d, &made) else { panic!("an area") };
    let after = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
    let [x0, y0, x1, y1] = view_pixels(area, Affine::IDENTITY, 100, 100).expect("on screen");
    let part = r.render(&d, x1 - x0, y1 - y0, Affine::translate((-f64::from(x0), -f64::from(y0))), &opts);
    let mut patched = before.pixels.clone();
    for y in 0..y1 - y0 {
        let (from, to, n) = ((y * (x1 - x0) * 4) as usize, (((y0 + y) * 100 + x0) * 4) as usize, ((x1 - x0) * 4) as usize);
        patched[to..to + n].copy_from_slice(&part.pixels[from..from + n]);
    }
    assert!(patched == after.pixels, "drawn again in {area:?}, it differs from drawing it all");
    assert!(before.pixels != after.pixels, "the bitmaps show");
    // Other files change nothing; art of a brush or a symbol showing the file changes everything.
    assert_eq!(placed_area(&d, &files(&["area-z"])), Changed::Nothing);
    let mut brushed = d.clone();
    let scatter = vectorcraft_brush::Scatter { art: placed_node(66, "area-a", 40.0, 20.0, Affine::IDENTITY), ..Default::default() };
    let brush = vectorcraft_brush::Brush { name: "Cards".into(), kind: vectorcraft_brush::BrushKind::Scatter(scatter) };
    vectorcraft_brush::store(&mut brushed, &[brush]);
    assert_eq!(placed_area(&brushed, &only_a), Changed::Everything);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Card".into(), art: Arc::new(placed_node(65, "area-a", 40.0, 20.0, Affine::IDENTITY)) });
    assert_eq!(placed_area(&d, &only_a), Changed::Everything);
}

#[test]
fn a_placed_document_kept_as_its_preview_draws_the_preview() {
    let _serial = serial();
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
    let _serial = serial();
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
