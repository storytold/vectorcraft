//! Type redraws in a font that becomes available while it is on screen (fonts added, Refresh Font
//! List).

use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Point;

use crate::*;

/// Taken to add fonts, or to look up an installed one by name (`FontDb::face` loads it, which
/// changes the fonts too). Renderers lay type out again when the fonts change, so tests of what
/// they keep between frames hold it shared ([`fonts_steady`]) and the tests that change the fonts
/// hold it alone ([`adding_fonts`]).
static FONTS: RwLock<()> = RwLock::new(());

/// No fonts are added by these tests until the guard is dropped.
pub(crate) fn fonts_steady() -> RwLockReadGuard<'static, ()> {
    // A test that failed holding it leaves nothing to undo.
    FONTS.read().unwrap_or_else(PoisonError::into_inner)
}

/// No test holding [`fonts_steady`] runs until the guard is dropped.
pub(crate) fn adding_fonts() -> RwLockWriteGuard<'static, ()> {
    FONTS.write().unwrap_or_else(PoisonError::into_inner)
}

/// What `step` returns from a run in which the fonts stayed the same. The fonts can change while
/// [`fonts_steady`] is held: the first lookup by family name catalogs the installed fonts, and
/// resolving a cataloged family, or a character no loaded font has, loads a face. `step` runs up
/// to 100 times, until [`vectorcraft_text::FontDb::generation`] is the same after a run as before
/// it.
pub(crate) fn with_fonts_unchanged<T>(mut step: impl FnMut() -> T) -> T {
    let db = vectorcraft_text::FontDb::global();
    for _ in 0..100 {
        let before = db.generation();
        let out = step();
        if db.generation() == before {
            return out;
        }
    }
    panic!("the fonts changed during each of 100 runs");
}

/// The bundled Source Serif 4 renamed `family` (as long as the original name).
fn serif_as(family: &str) -> Vec<u8> {
    let mut data = include_bytes!("../../../../assets/fonts/SourceSerif4-Regular.ttf").to_vec();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Serif 4"), utf16(family)), (b"Source Serif 4".to_vec(), family.as_bytes().to_vec())] {
        assert_eq!(from.len(), to.len());
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

#[test]
fn type_redraws_when_its_font_becomes_available() {
    let _adding = adding_fonts();
    const FAMILY: &str = "Arrival Serif4";
    let mut d = Document::new(300.0, 150.0);
    let style = CharStyle { size: 60.0, font_family: FAMILY.into(), ..Default::default() };
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 100.0), "Hamburg", style))));
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert_ne!(before, after, "drawn in the font that arrived");
    assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

/// Same serialized text and node ids are not the same glyph geometry when each document owns
/// different file-font bytes. Both fonts are bound before either render, so generation is steady.
#[test]
fn file_font_geometry_is_scoped_to_owned_resources_not_serialized_keys() {
    use vectorcraft_doc::{FILE_FONT_MIME, FileFont, FileFonts, ImageBlob};
    use vectorcraft_text::{FileFontInfo, FileGlyph, FontDb, build_file_font};

    let _adding = adding_fonts();
    let make = |width| {
        const FAMILY: &str = "Rendered Owned File Font Regression";
        let info = FileFontInfo { family: FAMILY.into(), style: "Regular".into(), ..Default::default() };
        let g = FileGlyph { ch: 'x', outline: Rect::new(0.0, 0.0, width, 700.0).to_path(0.1), advance: width + 100.0 };
        let mut doc = Document::new(100.0, 100.0);
        doc.images.insert("chosen".into(), ImageBlob::new(FILE_FONT_MIME, build_file_font(&info, &[g]).unwrap()));
        let style = CharStyle { size: 60.0, font_family: FAMILY.into(), ..Default::default() };
        let mut t = TextObject::point(Point::new(10.0, 70.0), "x", style);
        t.file_fonts = Some(Box::new(FileFonts::new(vec![FileFont { family: FAMILY.into(), style: "Regular".into(), key: "chosen".into() }], "x")));
        let id = doc.alloc_id();
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
        assert!(FontDb::global().bind_document_file_fonts(&mut doc).is_empty());
        doc
    };
    let (a, b) = (make(200.0), make(800.0));
    let renderer = with_fonts_unchanged(|| {
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut reused = Renderer::new();
        let first = reused.render(&a, 100, 100, Affine::IDENTITY, &opts).pixels;
        let second = reused.render(&b, 100, 100, Affine::IDENTITY, &opts).pixels;
        let fresh = Renderer::new().render(&b, 100, 100, Affine::IDENTITY, &opts).pixels;
        assert_ne!(first, second, "B must draw its own wider glyph");
        assert_eq!(second, fresh, "a renderer must not reuse A's glyph geometry for B");
        reused
    });
    let mut faces = vec![];
    for doc in [&a, &b] {
        doc.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                faces.extend(FontDb::global().file_faces(t).into_iter().flatten().map(|face| Arc::downgrade(&face)));
            }
        });
    }
    assert_eq!(faces.len(), 2);
    drop(a);
    drop(b);
    assert!(!renderer.texts.is_empty(), "the idle renderer still holds its old text nodes");
    assert!(faces.iter().all(|face| face.upgrade().is_none()), "geometry-cache nodes must not pin document font allowance");
}

/// Type with raster effects redraws in a font that becomes available: its drop shadow and its
/// blurred content are made again.
#[test]
fn type_with_raster_effects_redraws_when_its_font_becomes_available() {
    let _adding = adding_fonts();
    const FAMILY: &str = "Raster Serif 4";
    let mut d = Document::new(300.0, 150.0);
    let l = d.layers[0].id;
    let effects =
        [("stylize.dropShadow", serde_json::json!({"x": 4, "y": 4, "blur": 1, "opacity": 100})), ("blur.gaussian", serde_json::json!({"radius": 1}))];
    for (i, (id, params)) in effects.into_iter().enumerate() {
        let style = CharStyle { size: 50.0, font_family: FAMILY.into(), ..Default::default() };
        let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 60.0 + 70.0 * i as f64), "Hamburg", style))));
        n.appearance.effects.push(vectorcraft_doc::Effect { id: id.into(), params, visible: true });
        d.insert(Some(l), i, n).unwrap();
    }
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert_ne!(before, after, "drawn in the font that arrived");
    assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

/// Type with Variable Font settings is drawn in a face of their own, which is not a substitute:
/// Document Setup's highlight of substituted glyphs leaves it alone, and so does Highlight
/// Alternate Glyphs.
#[test]
fn variable_font_settings_are_not_highlighted_as_substituted_glyphs() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    let _adding = adding_fonts();
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let font_axes = [("wght".to_string(), 650.0)].into_iter().collect();
    let style = CharStyle { size: 40.0, font_family: TWO_AXIS_FAMILY.into(), font_axes, ..Default::default() };
    let t = TextObject::point(Point::new(10.0, 60.0), "lIn", style);
    let g = text_geom(&t);
    assert!(!g.all.elements().is_empty());
    assert!(g.substituted_glyphs.elements().is_empty() && g.substituted_fonts.elements().is_empty());
    assert!(g.alternate_glyphs(&t).elements().is_empty());
}

/// Type in a composite font draws each class of character in its part's face, which is not a
/// substitute: Document Setup's highlight of substituted glyphs leaves it alone.
#[test]
fn composite_font_parts_are_not_highlighted_as_substituted_glyphs() {
    use vectorcraft_doc::{CompositeFont, CompositePart, CompositeParts};
    let _adding = adding_fonts();
    let db = vectorcraft_text::FontDb::global();
    let mut parts = CompositeParts::all(CompositePart::new("Source Sans 3", "Regular"));
    parts.roman = CompositePart::new("JetBrains Mono", "Regular");
    parts.numerals = CompositePart::new("Inter", "Regular");
    db.set_composite_font(&CompositeFont { name: "CF Highlight".into(), parts });
    let style = CharStyle { size: 40.0, font_family: "CF Highlight".into(), ..Default::default() };
    let t = TextObject::point(Point::new(10.0, 60.0), "Ab12", style);
    let g = text_geom(&t);
    db.remove_composite_font("CF Highlight");
    assert!(!g.all.elements().is_empty());
    assert!(g.substituted_glyphs.elements().is_empty() && g.substituted_fonts.elements().is_empty());
}

/// Type in one of a document's composite fonts draws in the document's definition when nothing has
/// registered it: in a render, an ink render, a thumbnail and a placed document's art, as type set
/// in the part's font draws.
/// The font database is left without it afterwards.
#[test]
fn a_documents_own_composite_font_draws_without_being_registered() {
    use vectorcraft_doc::{CompositeFont, CompositePart, CompositeParts};
    let _adding = adding_fonts();
    const NAME: &str = "CF Unregistered";
    let db = vectorcraft_text::FontDb::global();
    let typed = |family: &str| {
        let mut d = Document::new(300.0, 150.0);
        let style = CharStyle { size: 60.0, font_family: family.into(), ..Default::default() };
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 100.0), "Hamburg", style))));
        let l = d.layers[0].id;
        let id = d.insert(Some(l), 0, n).unwrap();
        (d, id)
    };
    let (mut composite, id) = typed(NAME);
    let mut parts = CompositeParts::all(CompositePart::new("Source Sans 3", "Regular"));
    parts.roman = CompositePart::new("JetBrains Mono", "Regular");
    composite.composite_fonts.push(CompositeFont { name: NAME.into(), parts });
    let (plain, plain_id) = typed("JetBrains Mono");
    assert!(db.composite_font(NAME).is_none());
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let view = Affine::IDENTITY;
    assert_eq!(
        Renderer::new().render(&composite, 300, 150, view, &opts).pixels,
        Renderer::new().render(&plain, 300, 150, view, &opts).pixels,
        "rendered"
    );
    let region = Rect::new(0.0, 0.0, 300.0, 150.0);
    assert_eq!(
        Renderer::new().render_region_inks(&composite, region, 1.0, &opts),
        Renderer::new().render_region_inks(&plain, region, 1.0, &opts),
        "rendered as ink amounts"
    );
    let thumbnail = |d: &Document, id| Renderer::new().render_thumbnail(d, id, 64).map(|r| r.pixels);
    assert_eq!(thumbnail(&composite, id), thumbnail(&plain, plain_id), "thumbnail");
    let placed = |d: &Document| crate::placed_document::render_inside(&mut Renderer::new(), d, &d.layers[0], 300, 150, view).map(|r| r.pixels);
    assert_eq!(placed(&composite), placed(&plain), "a placed document's art");
    // A page that places the document, drawn exactly (no bitmap of the placed art).
    let page = |d: &Document, id: NodeId, key: &str| {
        use vectorcraft_doc::{ImageBlob, LinkInfo, PlacedDocument};
        let blob = ImageBlob::new("application/json", key.as_bytes().to_vec());
        let p = PlacedDocument {
            link: LinkInfo::new("/art/type.vectorcraft"),
            key: key.into(),
            bounding: false,
            page_box: Default::default(),
            width: 300.0,
            height: 150.0,
            xf: Affine::IDENTITY,
            placement: Default::default(),
        };
        let art = d.node(id).unwrap().clone();
        vectorcraft_doc::placed_document::prime(&p, blob.bytes.clone(), (d.clone(), art, Rect::new(0.0, 0.0, 300.0, 150.0)));
        let mut page = Document::new(300.0, 150.0);
        page.images.insert(key.into(), blob);
        let n = Node::new(page.alloc_id(), NodeKind::PlacedDocument(Box::new(p)));
        let l = page.layers[0].id;
        page.insert(Some(l), 0, n).unwrap();
        Renderer::new().render(&page, 300, 150, view, &opts).pixels
    };
    assert_eq!(
        page(&composite, id, "fontchange-composite-placed"),
        page(&plain, plain_id, "fontchange-plain-placed"),
        "a placed document's art drawn exactly"
    );
    assert!(db.composite_font(NAME).is_none(), "given back after each render");
}

/// Type at 80% opacity, in Multiply and with a drop shadow redraws in a font that becomes
/// available: the glyph geometry kept for it is laid out again.
#[test]
fn translucent_blended_and_shadowed_type_redraws_when_its_font_becomes_available() {
    let _adding = adding_fonts();
    const FAMILY: &str = "Blended Serif4";
    let mut d = Document::new(300.0, 150.0);
    let l = d.layers[0].id;
    for i in 0..3 {
        let style = CharStyle { size: 36.0, font_family: FAMILY.into(), ..Default::default() };
        let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 40.0 + 45.0 * i as f64), "Hamburg", style))));
        match i {
            0 => n.opacity = 0.8,
            1 => n.blend = vectorcraft_color::BlendMode::Multiply,
            _ => n.appearance.effects.push(vectorcraft_doc::Effect {
                id: "stylize.dropShadow".into(),
                params: serde_json::json!({"x": 4, "y": 4, "blur": 1, "opacity": 100}),
                visible: true,
            }),
        }
        d.insert(Some(l), i, n).unwrap();
    }
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert_eq!(r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, before, "drawn again from the kept glyph geometry");
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert_ne!(before, after, "drawn in the font that arrived");
    assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

/// Type with a geometry effect in a symbol's art redraws in a font that becomes available: the art
/// kept for the instance is placed again when the fonts change.
#[test]
fn symbol_type_redraws_when_its_font_becomes_available() {
    let _adding = adding_fonts();
    const FAMILY: &str = "Placed Serif 4";
    let mut d = Document::new(300.0, 150.0);
    let style = CharStyle { size: 60.0, font_family: FAMILY.into(), ..Default::default() };
    let mut t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(-140.0, 25.0), "Hamburg", style))));
    t.appearance.effects.push(vectorcraft_doc::Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 0.0}), visible: true });
    let art = Node::group(d.alloc_id(), vec![Arc::new(t)]);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Word".into(), art: Arc::new(art) });
    let (l, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Word".into(), xf: Affine::translate((150.0, 75.0)) })).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert_ne!(before, after, "drawn in the font that arrived");
    assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

/// Type that a geometry effect outlines redraws in a font that becomes available: the art drawn for
/// the effects of the type, of a group that holds it and of a symbol instance whose art holds it
/// is made again when the fonts change.
#[test]
fn type_with_geometry_effects_redraws_when_its_font_becomes_available() {
    let _adding = adding_fonts();
    use vectorcraft_doc::Effect;
    let moved = || Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 0.0}), visible: true };
    let cases = [
        ("Moving Serif 4", "the type with a Transform effect"),
        ("Shadow Serif 5", "the type with a Transform effect and a drop shadow"),
        ("Massed Serif 4", "the group with a Transform effect"),
        ("Stamps Serif 4", "the symbol instance with a Transform effect"),
        ("Darker Serif 4", "the type with a Transform effect in Multiply"),
    ];
    for (i, (family, what)) in cases.into_iter().enumerate() {
        let mut d = Document::new(300.0, 150.0);
        let style = CharStyle { size: 50.0, font_family: family.into(), ..Default::default() };
        let at = if i == 3 { Point::new(-140.0, 20.0) } else { Point::new(10.0, 80.0) };
        let mut object = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(at, "Hamburg", style))));
        match i {
            0 => object.appearance.effects.push(moved()),
            1 => {
                object.appearance.effects.push(moved());
                let params = serde_json::json!({"x": 4, "y": 4, "blur": 1, "opacity": 100});
                object.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params, visible: true });
            }
            2 => {
                object = Node::group(d.alloc_id(), vec![Arc::new(object)]);
                object.appearance.effects.push(moved());
            }
            3 => {
                let art = Node::group(d.alloc_id(), vec![Arc::new(object)]);
                d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Word".into(), art: Arc::new(art) });
                object = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Word".into(), xf: Affine::translate((150.0, 75.0)) });
                object.appearance.effects.push(moved());
            }
            _ => {
                object.appearance.effects.push(moved());
                object.blend = vectorcraft_color::BlendMode::Multiply;
            }
        }
        let l = d.layers[0].id;
        d.insert(Some(l), 0, object).unwrap();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
        assert!(vectorcraft_text::FontDb::global().add_font(serif_as(family)) > 0);
        let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
        assert_ne!(before, after, "{what}: drawn in the font that arrived");
        assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "{what}: as a new renderer draws it");
    }
}

/// Type that an envelope bends redraws in a font that becomes available, in the envelope and in
/// the art of a symbol whose instance an envelope bends: the envelopes are evaluated again when the
/// fonts change.
#[test]
fn enveloped_type_redraws_when_its_font_becomes_available() {
    let _adding = adding_fonts();
    const FAMILY: &str = "Warped Serif 4";
    let mut d = Document::new(300.0, 150.0);
    let word = |d: &mut Document, at: Point| {
        let style = CharStyle { size: 50.0, font_family: FAMILY.into(), ..Default::default() };
        Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(at, "Hamburg", style))))
    };
    let typed = word(&mut d, Point::new(10.0, 60.0));
    let art = word(&mut d, Point::new(-140.0, 20.0));
    let art = Node::group(d.alloc_id(), vec![Arc::new(art)]);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Word".into(), art: Arc::new(art) });
    let shown = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Word".into(), xf: Affine::translate((150.0, 120.0)) });
    let l = d.layers[0].id;
    for (i, content) in [typed, shown].into_iter().enumerate() {
        let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
        let envelope = NodeKind::Envelope {
            content: vec![Arc::new(content)],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: Default::default(),
            frame: Box::new(Affine::IDENTITY),
        };
        let id = d.alloc_id();
        d.insert(Some(l), i, Node::new(id, envelope)).unwrap();
    }
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
    assert_ne!(before, after, "drawn in the font that arrived");
    assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "as a new renderer draws it");
}

/// Type that an envelope bends redraws in a font that becomes available when the art drawn for
/// effects is made from the envelope's evaluation: the envelope with a drop shadow, and the
/// envelope in a group with a geometry effect or with a fill of its own.
#[test]
fn enveloped_type_under_effects_redraws_when_its_font_becomes_available() {
    let _adding = adding_fonts();
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{AppearanceItem, Effect, FillLayer};
    let cases = [
        ("Shadow Serif 4", "the envelope with a drop shadow"),
        ("Effect Serif 4", "the group with a geometry effect"),
        ("Filled Serif 4", "the group with a fill"),
    ];
    for (i, (family, what)) in cases.into_iter().enumerate() {
        let mut d = Document::new(300.0, 150.0);
        let style = CharStyle { size: 50.0, font_family: family.into(), ..Default::default() };
        let typed = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 80.0), "Hamburg", style))));
        let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
        let envelope = NodeKind::Envelope {
            content: vec![Arc::new(typed)],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: Default::default(),
            frame: Box::new(Affine::IDENTITY),
        };
        let mut object = Node::new(d.alloc_id(), envelope);
        match i {
            0 => {
                let params = serde_json::json!({"x": 4, "y": 4, "blur": 1, "opacity": 100});
                object.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params, visible: true });
            }
            1 => {
                object = Node::group(d.alloc_id(), vec![Arc::new(object)]);
                object.appearance.effects.push(Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 0.0}), visible: true });
            }
            _ => {
                object = Node::group(d.alloc_id(), vec![Arc::new(object)]);
                object.appearance.items.push(AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)))));
            }
        }
        let l = d.layers[0].id;
        d.insert(Some(l), 0, object).unwrap();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        let before = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
        assert!(vectorcraft_text::FontDb::global().add_font(serif_as(family)) > 0);
        let after = r.render(&d, 300, 150, Affine::IDENTITY, &opts).pixels;
        assert_ne!(before, after, "{what}: drawn in the font that arrived");
        assert_eq!(after, Renderer::new().render(&d, 300, 150, Affine::IDENTITY, &opts).pixels, "{what}: as a new renderer draws it");
    }
}

/// Without an engine session, a renderer maps an envelope of type from its glyph outline bounds,
/// and measures the type again when its font becomes available.
#[test]
fn a_renderer_maps_enveloped_type_from_its_glyphs_in_the_current_font() {
    let _adding = adding_fonts();
    const FAMILY: &str = "Mapped Serif 4";
    let style = CharStyle { size: 50.0, font_family: FAMILY.into(), ..Default::default() };
    let t = TextObject::point(Point::new(10.0, 60.0), "Hamburg", style);
    let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
    let envelope = Node::new(
        vectorcraft_doc::NodeId(1),
        NodeKind::Envelope {
            content: vec![Arc::new(Node::new(vectorcraft_doc::NodeId(2), NodeKind::Text(Box::new(t.clone()))))],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: Default::default(),
            frame: Box::new(Affine::IDENTITY),
        },
    );
    let ink = || t.xf.transform_rect_bbox(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).ink_bounds().unwrap());
    let src = || vectorcraft_doc::live::EnvelopeMap::of(&envelope).unwrap().src();
    let off = |a: Rect, b: Rect| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let _ = Renderer::new();
    let before = ink();
    assert!(off(src(), before) < 1e-6, "{:?} vs {before:?}", src());
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let after = ink();
    assert!(off(before, after) > 0.5, "the type draws in the font that arrived: {before:?} vs {after:?}");
    assert!(off(src(), after) < 1e-6, "{:?} vs {after:?}", src());
}

/// 3D type measured for an edit ([`Renderer::changed`]) and drawn after its font became available
/// is drawn in that font: the art made to measure it, in the font it had, isn't taken.
#[test]
fn three_d_type_measured_before_its_font_arrives_draws_in_it() {
    let _adding = adding_fonts();
    const FAMILY: &str = "Solid Serif 44";
    let mut old = Document::new(300.0, 150.0);
    let style = CharStyle { size: 60.0, font_family: FAMILY.into(), ..Default::default() };
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 100.0), "Hamburg", style))));
    n.id = old.alloc_id();
    let extrude = |depth: f64| vectorcraft_doc::Effect {
        id: "threeD.extrude".into(),
        params: serde_json::json!({"depth": depth, "position": "isometricTop"}),
        visible: true,
    };
    n.appearance.effects.push(extrude(40.0));
    let l = old.layers[0].id;
    let id = old.insert(Some(l), 0, n).unwrap();
    let mut new = old.clone();
    new.node_mut(id).unwrap().appearance.effects[0] = extrude(10.0);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    r.render(&old, 300, 150, Affine::IDENTITY, &opts);
    let _ = r.changed(&old, &new, &opts);
    assert!(vectorcraft_text::FontDb::global().add_font(serif_as(FAMILY)) > 0);
    let drawn = r.render(&new, 300, 150, Affine::IDENTITY, &opts).pixels;
    let fresh = Renderer::new().render(&new, 300, 150, Affine::IDENTITY, &opts).pixels;
    let differ = drawn.iter().zip(&fresh).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "as a new renderer draws it");
}
