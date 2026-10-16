//! What renderers keep between frames: decoded images shared by all of them, image mips, symbol
//! art placed for instances and inline graphics, stroke outlines, raster effects made again when
//! what they read changes, and caches swept by every kind of render. Each test holds [`fonts_steady`]:
//! fonts added while it runs would make its renderers lay type out again.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};

use super::*;
use crate::tests_fontchange::fonts_steady;

/// A `w`×`h` PNG whose pixels depend on `seed` (each test its own image: the store is shared by
/// the tests running alongside).
fn png(w: u32, h: u32, seed: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let v = (x * 7 + y * 13) as u8;
        image::Rgba([v ^ seed, v.wrapping_mul(3), seed, 255])
    });
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A document showing `bytes` as image `key` of `w`×`h` pixels, one point a pixel.
fn doc_with_image(key: &str, bytes: Vec<u8>, w: u32, h: u32) -> Document {
    let mut d = Document::new(w as f64, h as f64);
    d.images.insert(key.into(), ImageBlob::new("image/png", bytes));
    let (id, layer) = (d.alloc_id(), d.layers[0].id);
    let im = ImageObject { key: key.into(), width: w, height: h, xf: Affine::IDENTITY, link: None, placement: Default::default() };
    d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    d
}

#[test]
fn renderers_share_one_decode_of_an_image() {
    let _fonts = fonts_steady();
    let bytes = png(16, 16, 101);
    let a = doc_with_image("img", bytes.clone(), 16, 16);
    // Another document holding the same image under another key (its own copy of the bytes).
    let b = doc_with_image("photo", bytes, 16, 16);
    let (mut r1, mut r2) = (Renderer::new(), Renderer::new());
    let (k1, p1) = r1.image_pixmap(&a, "img", false, true).unwrap();
    let (k2, p2) = r2.image_pixmap(&b, "photo", false, true).unwrap();
    assert_eq!(k1, k2);
    assert!(Arc::ptr_eq(&p1, &p2), "decoded once for both renderers");
    // Drawing goes through the store too: the pixels stay the ones decoded.
    let _ = r2.render(&b, 16, 16, Affine::IDENTITY, &RenderOptions::default());
    assert!(Arc::ptr_eq(&r1.image_pixmap(&b, "photo", false, true).unwrap().1, &p1));
    // A document using the same key for other pixels gets its own.
    let c = doc_with_image("img", png(16, 16, 102), 16, 16);
    let (k3, p3) = r1.image_pixmap(&c, "img", false, true).unwrap();
    assert!(k3 != k1 && !Arc::ptr_eq(&p3, &p1));
    assert!(
        r1.render(&c, 16, 16, Affine::IDENTITY, &RenderOptions::default()).pixels
            != r1.render(&a, 16, 16, Affine::IDENTITY, &RenderOptions::default()).pixels
    );
}

/// Thumbnails and single-node renders end their frame as full renders do: entries of nodes no
/// longer drawn are dropped, with the nodes they kept alive.
#[test]
fn node_renders_sweep_stale_cache_entries() {
    let _fonts = fonts_steady();
    let mut d = Document::new(10.0, 10.0);
    let id = d.alloc_id();
    let square = |x: f64| {
        let p = vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 5.0, 5.0));
        Arc::new(Node::path(id, p, vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0)))
    };
    let first = square(0.0);
    let mut r = Renderer::new();
    r.render_node(&d, &first, 10, 10, Affine::IDENTITY).unwrap();
    assert!(Arc::strong_count(&first) > 1, "cached while drawn");
    for i in 0..1100 {
        let n = square(i as f64);
        if i % 2 == 0 {
            r.render_node(&d, &n, 10, 10, Affine::IDENTITY).unwrap();
        } else {
            // A thumbnail of a group caches its member's geometry.
            r.render_node_thumbnail(&d, &Node::group(id, vec![n]), 16, None).unwrap();
        }
    }
    assert_eq!(Arc::strong_count(&first), 1, "swept: the renderer no longer keeps the node");
    assert!(r.geom.len() <= 1025, "{} geometry entries", r.geom.len());
}

/// With `image_mips` off (the default; exports) an image draws from its full pixels whatever the
/// store holds; on, a reduced image draws from a box-filtered half (quarter…) instead, and at
/// more than half size from the full pixels as before.
#[test]
fn image_mips_are_opt_in_and_change_nothing_off() {
    let _fonts = fonts_steady();
    let d = doc_with_image("img", png(64, 64, 103), 64, 64);
    let (off, on) = (RenderOptions::default(), RenderOptions { image_mips: true, ..Default::default() });
    let quarter = Affine::scale(0.25);
    let cold = Renderer::new().render(&d, 16, 16, quarter, &off);
    // Mips made by another renderer don't change what this one draws with them off.
    let mipped = Renderer::new().render(&d, 16, 16, quarter, &on);
    let mut warm = Renderer::new();
    assert!(warm.render(&d, 16, 16, quarter, &off).pixels == cold.pixels);
    assert!(warm.render(&d, 16, 16, quarter, &off).pixels == cold.pixels, "warm");
    // On, the quarter-size image samples the quarter-size mip: close to the full pixels' average
    // but not the same pixels.
    assert!(mipped.pixels != cold.pixels);
    let (key, _) = warm.image_pixmap(&d, "img", false, true).unwrap();
    let mip = images::store().peek(&images::ImageKey { mip: 2, ..key }).expect("quarter mip kept");
    assert_eq!((mip.width(), mip.height()), (16, 16));
    // Above half size, mips on draw the full pixels.
    let near = Affine::scale(0.75);
    assert!(Renderer::new().render(&d, 48, 48, near, &on).pixels == Renderer::new().render(&d, 48, 48, near, &off).pixels);
}

/// A document of `n` point type objects reading `word`, in rows.
fn text_doc(n: usize, word: &str) -> Document {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    for i in 0..n {
        let at = vectorcraft_geom::Point::new(10.0 + (i % 8) as f64 * 48.0, 30.0 + (i / 8) as f64 * 30.0);
        let style = CharStyle { size: 18.0, ..Default::default() };
        let mut t = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(at, word, style))));
        t.id = d.alloc_id();
        d.insert(Some(l), 0, t).unwrap();
    }
    d
}

/// The cache keys of `d`'s type objects (their nodes' addresses).
fn text_keys(d: &Document) -> Vec<usize> {
    let mut keys = vec![];
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            keys.push(n as *const Node as usize);
        }
    });
    keys
}

/// More type than the glyph cache's budget holds: the entries kept stay from frame to frame (the
/// rest is laid out again), where emptying the cache when full laid out every object every frame.
#[test]
fn type_over_the_text_budget_keeps_its_entries_between_frames() {
    let _fonts = fonts_steady();
    let d = text_doc(40, "Type");
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let want = r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels;
    assert_eq!(r.texts.len(), 40);
    let all = r.texts.bytes();
    let mut r = Renderer::new();
    r.texts = FrameCache::new(all / 2, 1024);
    let keys = text_keys(&d);
    let kept = |r: &Renderer| keys.iter().filter(|k| r.texts.contains_key(*k)).copied().collect::<Vec<_>>();
    assert_eq!(r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels, want);
    let first = kept(&r);
    assert!(!first.is_empty() && first.len() < 40 && r.texts.bytes() <= all / 2, "{} entries, {} bytes", first.len(), r.texts.bytes());
    for _ in 0..3 {
        assert_eq!(r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels, want);
        assert_eq!(kept(&r), first, "the same entries stay");
    }
    // Other type: its entries replace the earlier frames' ones, within the budget.
    let e = text_doc(40, "Other");
    r.render(&e, 400, 400, Affine::IDENTITY, &opts);
    assert!(kept(&r).is_empty(), "entries of earlier frames went first");
    assert!(!r.texts.is_empty() && r.texts.bytes() <= all / 2);
}

/// Type no longer drawn leaves the glyph cache after a few frames, and its bytes with it.
#[test]
fn swept_type_entries_give_back_their_bytes() {
    let _fonts = fonts_steady();
    let d = text_doc(12, "Swept");
    let mut r = Renderer::new();
    r.texts = FrameCache::new(lru::TEXT_BUDGET, 0);
    r.render(&d, 400, 400, Affine::IDENTITY, &RenderOptions::default());
    let full = r.texts.bytes();
    assert!(full > 0 && r.texts.len() == 12);
    let empty = Document::new(10.0, 10.0);
    for _ in 0..5 {
        r.render(&empty, 10, 10, Affine::IDENTITY, &RenderOptions::default());
    }
    assert_eq!((r.texts.len(), r.texts.bytes()), (0, 0));
}

/// [`text_doc`] of three type objects reading `word`: one at 80% opacity, one in Multiply and one
/// with a drop shadow.
fn faded_text_doc(word: &str) -> Document {
    let mut d = text_doc(3, word);
    let ids: Vec<NodeId> = d.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    d.node_mut(ids[0]).unwrap().opacity = 0.8;
    d.node_mut(ids[1]).unwrap().blend = vectorcraft_color::BlendMode::Multiply;
    d.node_mut(ids[2]).unwrap().appearance.effects.push(drop_shadow());
    d
}

/// The glyph geometry `r` keeps for each type object of `d`: under its node, or under the art
/// drawn for it when it has effects.
fn kept_glyphs(r: &mut Renderer, d: &Document) -> Vec<Option<Arc<TextGeom>>> {
    let stamp = r.stamp;
    text_keys(d)
        .into_iter()
        .map(|k| {
            let k = r.fx_arts.get(&k, stamp).map_or(k, |e| Arc::as_ptr(&e.1) as usize);
            r.texts.get(&k, stamp).map(|(_, g)| g.clone())
        })
        .collect()
}

/// `f`'s result from a run during which the fonts did not change: other tests add fonts, and the
/// renderer lays all type out again when the fonts change.
fn fonts_unchanged<T>(mut f: impl FnMut() -> T) -> T {
    loop {
        let fonts = vectorcraft_text::FontDb::global().generation();
        let t = f();
        if vectorcraft_text::FontDb::global().generation() == fonts {
            return t;
        }
    }
}

/// Type at 80% opacity, in Multiply and with a drop shadow keeps its glyph geometry between
/// frames, as opaque type does: the second frame draws the geometry the first laid out. Frames
/// that snap type to pixels keep none, and neither does type art that the effect-art cache has no
/// room for.
#[test]
fn translucent_blended_and_shadowed_type_keeps_its_glyph_geometry_between_frames() {
    let d = faded_text_doc("Kept");
    let opts = RenderOptions::default();
    let (first, kept, entries, second, again) = fonts_unchanged(|| {
        let mut r = Renderer::new();
        let first = r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels;
        let (kept, entries) = (kept_glyphs(&mut r, &d), r.texts.len());
        let second = r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels;
        (first, kept, entries, second, kept_glyphs(&mut r, &d))
    });
    assert!(kept.iter().all(Option::is_some) && entries == 3, "{entries} entries");
    assert!(second == first);
    let same = kept.iter().zip(&again).all(|pair| matches!(pair, (Some(a), Some(b)) if Arc::ptr_eq(a, b)));
    assert!(same, "the second frame draws the geometry the first laid out");
    // Type anti-aliasing snaps the glyphs to pixels.
    let mut r = Renderer::new();
    r.render(&d, 400, 400, Affine::IDENTITY, &RenderOptions { anti_alias: AntiAlias::Type, ..Default::default() });
    assert!(r.texts.is_empty());
    // With no room for effect art, the shadowed type's art is made in every frame.
    let frames = fonts_unchanged(|| {
        let mut r = Renderer::new();
        r.fx_arts = FrameCache::new(0, 0);
        (0..2).map(|_| (r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels == first, r.texts.len())).collect::<Vec<_>>()
    });
    assert_eq!(frames, [(true, 2), (true, 2)]);
}

/// Editing the text of type at 80% opacity, in Multiply or with a drop shadow lays it out again:
/// the next frame draws what a new renderer draws.
#[test]
fn edited_translucent_blended_and_shadowed_type_draws_as_new() {
    for i in 0..3 {
        let mut d = faded_text_doc("Kept");
        let id = d.layers[0].children().unwrap()[i].id;
        let opts = RenderOptions::default();
        draws_as_new(&mut d, &opts, &opts, |d| {
            if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
                t.runs[0].text = "Edit".into();
            }
        });
    }
}

/// Type showing an inline graphic, at 80% opacity, in Multiply, with a drop shadow and opaque:
/// when the symbol's art changes size and the document's inline graphics are not resolved again,
/// the next frame lays the type out with the new size, as a new renderer does.
#[test]
fn type_with_an_inline_graphic_draws_its_symbols_new_size() {
    use vectorcraft_doc::text::{CharStyle, InlineArt, TextObject, TextRun};
    for i in 0..4 {
        let mut d = Document::new(200.0, 100.0);
        let art = symbol_art(&mut d);
        d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
        let style = CharStyle { size: 24.0, ..Default::default() };
        let mut t = TextObject::point(vectorcraft_geom::Point::new(10.0, 50.0), "A", style.clone());
        t.runs.push(TextRun::inline(InlineArt::new("S"), style.clone()));
        t.runs.push(TextRun::new("B", style));
        let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        match i {
            0 => n.opacity = 0.8,
            1 => n.blend = vectorcraft_color::BlendMode::Multiply,
            2 => n.appearance.effects.push(drop_shadow()),
            _ => {}
        }
        let mut d = with_art(d, vec![n]);
        d.resolve_inline_art();
        // `S` becomes a wide red bar: the inline graphic grows, and the type after it moves.
        edit_draws_as_new(d, |d| {
            let bar = vectorcraft_geom::shapes::rectangle(Rect::new(-20.0, -4.0, 20.0, 4.0));
            let red = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
            d.symbols[0].art = Arc::new(Node::path(d.alloc_id(), bar, red));
        });
    }
}

/// A symbol's art inside the 20 × 20 square around the origin: a group of a square filled blue
/// with a 2 pt black stroke and a black line.
fn symbol_art(d: &mut Document) -> Node {
    use vectorcraft_doc::Appearance;
    let square = vectorcraft_geom::shapes::rectangle(Rect::new(-8.0, -8.0, 8.0, 8.0));
    let square = Node::path(d.alloc_id(), square, Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::solid(Color::BLACK), 2.0));
    let line = vectorcraft_geom::shapes::line((-8.0, 4.0).into(), (8.0, 4.0).into());
    let line = Node::path(d.alloc_id(), line, Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.5));
    Node::group(d.alloc_id(), vec![Arc::new(square), Arc::new(line)])
}

/// Three instances of symbol `S` ([`symbol_art`]): two in a row, the second stained red, and a
/// third turned and scaled by 1.5.
fn symbol_doc() -> Document {
    let mut d = Document::new(200.0, 100.0);
    let art = symbol_art(&mut d);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    let l = d.layers[0].id;
    let xfs = [
        Affine::translate((30.0, 30.0)),
        Affine::translate((80.0, 30.0)),
        Affine::translate((140.0, 60.0)) * Affine::rotate(0.5) * Affine::scale(1.5),
    ];
    for (i, xf) in xfs.into_iter().enumerate() {
        let mut inst = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf });
        if i == 1 {
            inst.appearance.set_fill(Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
        }
        d.insert(Some(l), i, inst).unwrap();
    }
    d
}

/// The addresses of the nodes `r` keeps geometry for that are not the own nodes of any version
/// of the document drawn (`docs`): the symbol art it placed for instances and inline graphics.
fn placed_copies(r: &Renderer, docs: &[&Document]) -> std::collections::BTreeSet<usize> {
    let mut own = std::collections::HashSet::new();
    for d in docs {
        d.walk(|n| {
            own.insert(n as *const Node as usize);
        });
    }
    r.geom.values().map(|e| Arc::as_ptr(&e.node) as usize).filter(|p| !own.contains(p)).collect()
}

/// A second frame of an unchanged document draws the symbol art placed for its instances in the
/// first: it copies no art, and the geometry kept for the art's paths is used again. The renderer
/// holds the symbol's art itself only weakly.
#[test]
fn symbol_instances_keep_their_placed_art_between_frames() {
    let _fonts = fonts_steady();
    let d = symbol_doc();
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let first = r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let placed = placed_copies(&r, &[&d]);
    assert_eq!(placed.len(), 6, "the square and the line of three instances");
    assert!(r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels == first);
    assert_eq!(placed_copies(&r, &[&d]), placed, "the second frame draws the art the first placed");
    assert_eq!(Arc::strong_count(&d.symbols[0].art), 1, "only the document holds the symbol's art");
}

/// The stroke outlines and the gradient slices of strokes that `r` keeps and that its last frame made
/// or used.
fn stroke_outlines(r: &Renderer) -> (Vec<Arc<BezPath>>, Vec<Slices>) {
    let outlines = r.strokes.values().filter(|e| e.stamp == r.stamp).map(|e| e.outline.clone()).collect();
    (outlines, r.stroke_slices.values().filter(|e| e.stamp == r.stamp).map(|e| e.slices.clone()).collect())
}

/// How many of the outlines and of the slices in `before` (see [`stroke_outlines`]) the last frame of
/// `r` used again.
fn used_again(r: &Renderer, before: &(Vec<Arc<BezPath>>, Vec<Slices>)) -> (usize, usize) {
    fn again<T>(was: &[Arc<T>], now: &[Arc<T>]) -> usize {
        was.iter().filter(|a| now.iter().any(|b| Arc::ptr_eq(a, b))).count()
    }
    let (outlines, slices) = stroke_outlines(r);
    (again(&before.0, &outlines), again(&before.1, &slices))
}

/// Copies can share the original's appearance: three copies of [`symbol_art`] made as
/// `edit.duplicate` makes them, and a wave whose stroke has a gradient along it with a copy made as
/// the Layers panel's Duplicate Selection makes it. Each object keeps its own stroke outlines and
/// gradient slices from frame to frame, a copy moved gets new ones while the others keep theirs,
/// and each frame draws what a new renderer draws.
#[test]
fn copies_sharing_an_appearance_keep_their_own_stroke_outlines() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::{Appearance, StrokeGradientMode};
    let mut d = Document::new(200.0, 100.0);
    let art = symbol_art(&mut d);
    let l = d.layers[0].id;
    let mut copies = vec![];
    for x in [30.0, 80.0, 130.0] {
        let mut copy = d.reid(&art);
        copy.transform(Affine::translate((x, 30.0)), true);
        copies.push(d.insert(Some(l), usize::MAX, copy).unwrap());
    }
    let mut st = StrokeLayer::new(Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default()))), 6.0);
    st.gradient_mode = StrokeGradientMode::Along;
    let mut wave = BezPath::new();
    wave.move_to((20.0, 75.0));
    wave.curve_to((60.0, 55.0), (100.0, 95.0), (180.0, 75.0));
    let ap = Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() };
    let wave = Node::path(d.alloc_id(), vectorcraft_geom::PathData::from_bezpath(&wave), ap);
    let duplicate = d.reid(&wave);
    d.insert(Some(l), usize::MAX, wave).unwrap();
    d.insert(Some(l), usize::MAX, duplicate).unwrap();
    let opts = RenderOptions::default();
    let frame = |r: &mut Renderer, d: &Document| r.render(d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let mut r = Renderer::new();
    frame(&mut r, &d);
    let before = stroke_outlines(&r);
    assert_eq!((before.0.len(), before.1.len()), (8, 2), "the strokes of three copies and two waves, and the waves' slices");
    assert!(frame(&mut r, &d) == frame(&mut Renderer::new(), &d));
    assert_eq!(used_again(&r, &before), (8, 2), "the second frame draws the outlines the first made");
    // The first copy moves 5 pt right.
    if let Some(n) = d.node_mut(copies[0]) {
        n.transform(Affine::translate((5.0, 0.0)), true);
    }
    assert!(frame(&mut r, &d) == frame(&mut Renderer::new(), &d));
    assert_eq!(used_again(&r, &before), (6, 2), "the other copies and the waves keep theirs");
}

/// The art placed for unstained symbol instances shares its appearance with the symbol's art (the
/// first and the third instance of [`symbol_doc`]). Each instance keeps its own stroke outlines from
/// frame to frame, and the second frame draws what a new renderer draws.
#[test]
fn symbol_instances_keep_their_own_stroke_outlines_between_frames() {
    let _fonts = fonts_steady();
    let d = symbol_doc();
    let opts = RenderOptions::default();
    let frame = |r: &mut Renderer| r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let mut r = Renderer::new();
    frame(&mut r);
    let before = stroke_outlines(&r);
    assert_eq!(before.0.len(), 6, "the square's and the line's strokes of three instances");
    assert!(frame(&mut r) == frame(&mut Renderer::new()));
    assert_eq!(used_again(&r, &before), (6, 0), "the second frame draws the outlines the first made");
}

/// The art of symbol `I` ([`symbol_art`] with an image over it) is placed again in every frame.
/// The copies placed for its three unstained instances and for an inline graphic at the art's own
/// size replace each other's stroke outlines: the renderer keeps one outline for each of the art's
/// two strokes. Two copies of [`symbol_art`] made as `edit.duplicate` makes them and two instances
/// of symbol `S` ([`symbol_art`]), drawn after those, keep their own outlines. With no room in the
/// placed-art cache, `S`'s art is placed again in every frame as well, and its copies keep one
/// outline for each stroke. Each frame draws what a new renderer draws.
#[test]
fn art_placed_again_in_every_frame_keeps_one_outline_per_stroke() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::text::{CharStyle, InlineArt, TextObject, TextRun};
    let mut d = Document::new(200.0, 100.0);
    d.images.insert("dot".into(), ImageBlob::new("image/png", png(4, 4, 108)));
    let mut art = symbol_art(&mut d);
    let im = ImageObject { key: "dot".into(), width: 4, height: 4, xf: Affine::translate((-2.0, -2.0)), link: None, placement: Default::default() };
    let image = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    if let Some(c) = art.children_mut() {
        c.push(Arc::new(image));
    }
    d.symbols.push(vectorcraft_doc::Symbol { name: "I".into(), art: Arc::new(art) });
    let art = symbol_art(&mut d);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    let l = d.layers[0].id;
    let instance = |d: &mut Document, symbol: &str, x: f64, y: f64| {
        let id = d.alloc_id();
        d.insert(Some(l), usize::MAX, Node::new(id, NodeKind::SymbolInstance { symbol: symbol.into(), xf: Affine::translate((x, y)) })).unwrap();
    };
    for x in [25.0, 65.0, 105.0] {
        instance(&mut d, "I", x, 25.0);
    }
    // Type as tall as `I`'s art shows the inline graphic at scale 1.
    let size = d.symbol_art_bounds("I").map_or(0.0, |b| b.height());
    let style = CharStyle { size, ..Default::default() };
    let mut t = TextObject::point(vectorcraft_geom::Point::new(140.0, 32.0), "A", style.clone());
    t.runs.push(TextRun::inline(InlineArt::new("I"), style));
    let id = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    let art = symbol_art(&mut d);
    for x in [105.0, 145.0] {
        let mut copy = d.reid(&art);
        copy.transform(Affine::translate((x, 72.0)), true);
        d.insert(Some(l), usize::MAX, copy).unwrap();
    }
    for x in [25.0, 65.0] {
        instance(&mut d, "S", x, 72.0);
    }
    let opts = RenderOptions::default();
    let frame = |r: &mut Renderer| r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let counts = |r: &Renderer| (r.strokes.len(), r.stroke_slices.len());
    let mut r = Renderer::new();
    let mut full = Renderer::new();
    full.placed_arts = FrameCache::new(0, 0);
    let mut before = (vec![], vec![]);
    for n in 1..=4 {
        let want = frame(&mut Renderer::new());
        assert!(frame(&mut r) == want && frame(&mut full) == want, "frame {n}");
        // Two outlines for the art placed for `I`, two for each copy and two for each instance of
        // `S`; with no room in the cache, two for the art placed for `S`.
        assert_eq!((counts(&r), counts(&full)), ((10, 0), (8, 0)), "frame {n}");
        if n == 1 {
            before = stroke_outlines(&r);
        }
    }
    assert_eq!(used_again(&r, &before), (8, 0), "the instances of `S` and the copies keep theirs");
}

/// The document holds a Grid Repeat of [`symbol_art`] (nine groups) and a group with a Transform
/// effect around two copies of [`symbol_art`] made as `edit.duplicate` makes them. With no room in
/// the caches of evaluated live objects and of effect art, the repeat is evaluated again and the
/// group's art is made again in every frame. Their copies then replace each other's stroke
/// outlines: the renderer keeps two outlines for the repeat and two for the copies. With room, each
/// copy keeps its own. Each frame draws what a new renderer draws.
#[test]
fn art_evaluated_again_in_every_frame_keeps_one_outline_per_stroke() {
    let _fonts = fonts_steady();
    let mut d = Document::new(200.0, 100.0);
    let l = d.layers[0].id;
    let mut source = symbol_art(&mut d);
    source.transform(Affine::translate((20.0, 20.0)), true);
    let repeat = vectorcraft_doc::pattern::RepeatSpec::grid(vec![Arc::new(source)], 4.0, 4.0);
    let id = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::new(id, NodeKind::Repeat(repeat))).unwrap();
    let art = symbol_art(&mut d);
    let mut copies = vec![];
    for x in [130.0, 170.0] {
        let mut copy = d.reid(&art);
        copy.transform(Affine::translate((x, 30.0)), true);
        copies.push(Arc::new(copy));
    }
    let mut moved = Node::group(d.alloc_id(), copies);
    moved.appearance.effects.push(effect("distort.transform", serde_json::json!({"moveV": 30.0})));
    d.insert(Some(l), usize::MAX, moved).unwrap();
    let opts = RenderOptions::default();
    let frame = |r: &mut Renderer| r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let mut r = Renderer::new();
    let mut full = Renderer::new();
    full.live = live::LiveCache::with_budget(0);
    full.fx_arts = FrameCache::new(0, 0);
    for n in 1..=4 {
        let want = frame(&mut Renderer::new());
        assert!(frame(&mut r) == want && frame(&mut full) == want, "frame {n}");
        // Two outlines for each of the nine repeated groups and each copy; with no room in the
        // caches, two for the repeat and two for the copies.
        assert_eq!((r.strokes.len(), full.strokes.len()), (22, 4), "frame {n}");
    }
}

/// The instances of a symbol and the copies of a repeat replace each other's brush art, so their
/// brush art is made again in every frame. A brushed path gets new brush art each time its brush
/// changes. The renderer keeps no outline or gradient slices for the squares' strokes, and each
/// frame draws what a new renderer draws.
#[test]
fn strokes_of_brush_art_made_again_draw_as_a_new_renderer_does() {
    let _fonts = fonts_steady();
    use vectorcraft_brush::{Brush, BrushKind, Scatter};
    use vectorcraft_doc::{Appearance, StrokeGradientMode};
    let mut d = Document::new(200.0, 140.0);
    // Brush `Squares` scatters 8 × 8 squares stroked 2 pt with a gradient along the stroke and
    // 0.5 pt in black over it, with a spacing of `spacing` percent of the square's width.
    let squares = |d: &mut Document, spacing: f64| {
        let mut st = StrokeLayer::new(Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default()))), 2.0);
        st.gradient_mode = StrokeGradientMode::Along;
        let black = StrokeLayer::new(Paint::solid(Color::BLACK), 0.5);
        let square = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 8.0, 8.0));
        let strokes = Appearance { items: vec![AppearanceItem::Stroke(st), AppearanceItem::Stroke(black)], ..Default::default() };
        let art = Node::path(NodeId(0), square, strokes);
        let mut lib = vectorcraft_brush::library(d);
        lib.retain(|b| b.name != "Squares");
        lib.push(Brush { name: "Squares".into(), kind: BrushKind::Scatter(Scatter { art, spacing: (spacing, spacing), ..Default::default() }) });
        vectorcraft_brush::store(d, &lib);
    };
    // A wave around `at`, brushed with `Squares`.
    let wave = |d: &mut Document, at: (f64, f64)| {
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        if let Some(s) = ap.stroke_mut() {
            s.brush = Some("Squares".into());
        }
        let (x, y) = at;
        let mut wave = BezPath::new();
        wave.move_to((x - 40.0, y));
        wave.curve_to((x - 15.0, y - 20.0), (x + 15.0, y + 20.0), (x + 40.0, y));
        Node::path(d.alloc_id(), vectorcraft_geom::PathData::from_bezpath(&wave), ap)
    };
    // Three instances show symbol `B`, a group of a wave. A fourth wave is a path of the document,
    // and a Mirror repeat draws a fifth wave and its reflection.
    let art = Node::group(d.alloc_id(), vec![Arc::new(wave(&mut d, (0.0, 0.0)))]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "B".into(), art: Arc::new(art) });
    let l = d.layers[0].id;
    for at in [(50.0, 30.0), (150.0, 30.0), (50.0, 70.0)] {
        let id = d.alloc_id();
        d.insert(Some(l), usize::MAX, Node::new(id, NodeKind::SymbolInstance { symbol: "B".into(), xf: Affine::translate(at) })).unwrap();
    }
    let path = wave(&mut d, (150.0, 70.0));
    d.insert(Some(l), usize::MAX, path).unwrap();
    let source = wave(&mut d, (50.0, 110.0));
    let mirror = vectorcraft_doc::pattern::RepeatSpec::mirror(vec![Arc::new(source)], 90.0, 10.0);
    let id = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::new(id, NodeKind::Repeat(mirror))).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    // Six frames of the same document, then a frame after each change of the brush's spacing.
    for (frame, spacing) in [150.0, 150.0, 150.0, 150.0, 150.0, 150.0, 120.0, 150.0, 90.0, 150.0, 120.0, 135.0].into_iter().enumerate() {
        squares(&mut d, spacing);
        let want = Renderer::new().render(&d, 200, 140, Affine::IDENTITY, &opts).pixels;
        assert!(r.render(&d, 200, 140, Affine::IDENTITY, &opts).pixels == want, "frame {frame}");
    }
    assert_eq!((r.strokes.len(), r.stroke_slices.len()), (0, 0), "no outline or slices kept for the strokes of brush art");
}

/// Moving an instance places its art again, and editing a symbol's art places the art of its
/// instances again: the other instances keep theirs, and each frame draws what a new renderer
/// draws.
#[test]
fn editing_an_instance_or_a_symbol_places_only_its_art_again() {
    let _fonts = fonts_steady();
    let mut d = symbol_doc();
    // Symbol `T` is a green square, and one instance of it stands below the others.
    let green = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(0.0, 1.0, 0.0)), Paint::None, 0.0);
    let square = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(-5.0, -5.0, 5.0, 5.0)), green);
    let t = Node::group(d.alloc_id(), vec![Arc::new(square)]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "T".into(), art: Arc::new(t) });
    let (l, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "T".into(), xf: Affine::translate((30.0, 80.0)) })).unwrap();
    let opts = RenderOptions::default();
    let frame = |r: &mut Renderer, d: &Document| r.render(d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let mut r = Renderer::new();
    frame(&mut r, &d);
    let before = placed_copies(&r, &[&d]);
    assert_eq!(before.len(), 7);
    // The first instance of `S` moves 5 pt right.
    let unmoved = d.clone();
    let first = d.layers[0].children().unwrap()[1].id;
    if let Some(NodeKind::SymbolInstance { xf, .. }) = d.node_mut(first).map(|n| &mut n.kind) {
        *xf = Affine::translate((5.0, 0.0)) * *xf;
    }
    assert!(frame(&mut r, &d) == frame(&mut Renderer::new(), &d));
    let moved = placed_copies(&r, &[&unmoved, &d]);
    assert!(before.is_subset(&moved) && moved.len() == before.len() + 2, "{} copies, then {}", before.len(), moved.len());
    // `S`'s square is repainted red: its instances show it, and `T`'s instance keeps its art.
    let mut art = (*d.symbols[0].art).clone();
    if let Some(square) = art.children_mut().and_then(|c| c.first_mut()) {
        Arc::make_mut(square).appearance.set_fill(Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    }
    d.symbols[0].art = Arc::new(art);
    let red = frame(&mut r, &d);
    assert!(red == frame(&mut Renderer::new(), &d));
    let img = Rendered { width: 200, height: 100, pixels: red };
    assert_eq!(img.pixel(140, 60), [255, 0, 0, 255], "the turned instance's square");
    let edited = placed_copies(&r, &[&unmoved, &d]);
    assert!(moved.is_subset(&edited) && edited.len() == moved.len() + 6, "{} copies, then {}", moved.len(), edited.len());
}

/// A symbol whose art, kept between frames, lies outside the 20 × 20 square around the origin:
/// each frame culls the instance by the art the symbol holds then and draws what a new renderer
/// draws. When the symbol holds earlier art again, the renderer draws the art it placed for it
/// then.
#[test]
fn instances_of_art_outside_the_square_follow_their_symbols_art() {
    let _fonts = fonts_steady();
    /// A blue square in a group, 20 × 20 from (x, 0).
    fn square_at(d: &mut Document, x: f64) -> Arc<Node> {
        let blue = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
        let square = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 20.0)), blue);
        Arc::new(Node::group(d.alloc_id(), vec![Arc::new(square)]))
    }
    let mut d = Document::new(200.0, 100.0);
    // The instance moves the art by (-160, 40): the square from x 100 lies left of the view, and
    // the one from x 200 lies from (40, 40) to (60, 60), in it. The 20 × 20 square around the
    // origin lies from (-170, 30) to (-150, 50), left of the view.
    let (off, on) = (square_at(&mut d, 100.0), square_at(&mut d, 200.0));
    d.symbols.push(vectorcraft_doc::Symbol { name: "Far".into(), art: off.clone() });
    let (l, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((-160.0, 40.0)) })).unwrap();
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let mut placed = vec![];
    for art in [&off, &on, &off, &on] {
        d.symbols[0].art = art.clone();
        let frame = r.render(&d, 200, 100, Affine::IDENTITY, &opts);
        assert!(frame.pixels == Renderer::new().render(&d, 200, 100, Affine::IDENTITY, &opts).pixels);
        assert_eq!(frame.pixel(50, 50) == [0, 0, 255, 255], Arc::ptr_eq(art, &on), "{:?}", frame.pixel(50, 50));
        placed.push(placed_copies(&r, &[&d]));
    }
    assert!(!placed[1].is_empty() && placed[3] == placed[1], "the fourth frame draws the art placed in the second");
}

/// Draws `d` (200 × 100 points, on white), applies `edit` and draws it again with the same
/// renderer: the frame changed, a new renderer draws the same, and only the document holds the
/// symbols' art.
#[track_caller]
fn edit_draws_as_new(mut d: Document, edit: impl FnOnce(&mut Document)) {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let _warm = draws_as_new(&mut d, &opts, &opts, edit);
    assert!(d.symbols.iter().all(|s| Arc::strong_count(&s.art) == 1), "only the document holds the symbols' art");
}

/// Draws `d` (200 × 100 points) with `before`, applies `edit` and draws it with `after` and the
/// same renderer, which it returns: the frame changed, and a new renderer draws the same.
#[track_caller]
fn draws_as_new(d: &mut Document, before: &RenderOptions, after: &RenderOptions, edit: impl FnOnce(&mut Document)) -> Renderer {
    let mut r = Renderer::new();
    let first = r.render(d, 200, 100, Affine::IDENTITY, before).pixels;
    edit(d);
    let second = r.render(d, 200, 100, Affine::IDENTITY, after).pixels;
    assert!(second != first, "the edit changes the frame");
    assert!(second == Renderer::new().render(d, 200, 100, Affine::IDENTITY, after).pixels, "a new renderer draws the same");
    r
}

/// `d` with symbol `Holder` (a group of `members`) shown by one instance at (100, 50).
fn holder_doc(mut d: Document, members: Vec<Node>) -> Document {
    let holder = Node::group(d.alloc_id(), members.into_iter().map(Arc::new).collect());
    d.symbols.push(vectorcraft_doc::Symbol { name: "Holder".into(), art: Arc::new(holder) });
    let (l, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Holder".into(), xf: Affine::translate((100.0, 50.0)) })).unwrap();
    d
}

/// Symbol art that shows another symbol (type with an inline graphic of it, an instance of it with
/// a Transform effect) is placed again in every frame: when that symbol's art changes, the next
/// frame draws what a new renderer draws.
#[test]
fn art_showing_another_symbol_draws_its_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::text::{CharStyle, InlineArt, TextObject, TextRun};
    let mut d = Document::new(200.0, 100.0);
    let art = symbol_art(&mut d);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    let style = CharStyle { size: 24.0, ..Default::default() };
    let mut t = TextObject::point(vectorcraft_geom::Point::new(-60.0, 10.0), "A", style.clone());
    t.runs.push(TextRun::inline(InlineArt::new("S"), style.clone()));
    t.runs.push(TextRun::new("B", style));
    let text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    let mut moved = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((40.0, 0.0)) });
    moved.appearance.effects.push(vectorcraft_doc::Effect {
        id: "distort.transform".into(),
        params: serde_json::json!({"moveH": 4.0}),
        visible: true,
    });
    let d = holder_doc(d, vec![text, moved]);
    // `S` becomes a wide red bar: the inline graphic grows, and the type after it moves.
    edit_draws_as_new(d, |d| {
        let bar = vectorcraft_geom::shapes::rectangle(Rect::new(-20.0, -4.0, 20.0, 4.0));
        let red = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        d.symbols[0].art = Arc::new(Node::path(d.alloc_id(), bar, red));
    });
}

/// Symbol art with raster effects is placed again in every frame: when a pattern under a blur or
/// the document's raster resolution under De-Interlace changes, the next frame draws what a new
/// renderer draws.
#[test]
fn art_with_raster_effects_draws_pattern_and_resolution_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
    let effect = |id: &str, params: serde_json::Value| vectorcraft_doc::Effect { id: id.into(), params, visible: true };
    let mut d = Document::new(200.0, 100.0);
    d.raster_effects_ppi = 72.0;
    // Pattern `Checks` is a red 10 × 10 square in a 20 × 20 tile.
    let tile = |c: Color| {
        let square = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        vec![Arc::new(Node::path(NodeId(900), square, vectorcraft_doc::Appearance::basic(Paint::solid(c), Paint::None, 0.0)))]
    };
    let mut def = PatternDef::new("Checks", tile(Color::rgb(1.0, 0.0, 0.0)));
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    d.patterns.push(def);
    let band = vectorcraft_geom::shapes::rectangle(Rect::new(-90.0, -45.0, -20.0, -25.0));
    let mut blurred = Node::path(d.alloc_id(), band, vectorcraft_doc::Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
    blurred.appearance.effects.push(effect("blur.gaussian", serde_json::json!({"radius": 2.0})));
    // The path holds ten bars, 1 pt high and 2 pt apart.
    let mut bars = vectorcraft_geom::shapes::rectangle(Rect::new(20.0, -40.0, 90.0, -39.0));
    for k in 1..10 {
        let y = -40.0 + 2.0 * k as f64;
        bars.subpaths.extend(vectorcraft_geom::shapes::rectangle(Rect::new(20.0, y, 90.0, y + 1.0)).subpaths);
    }
    let mut striped = Node::path(d.alloc_id(), bars, vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    striped.appearance.effects.push(effect("video.deinterlace", serde_json::json!({})));
    let d = holder_doc(d, vec![blurred, striped]);
    edit_draws_as_new(d, |d| {
        if let Some(def) = d.pattern_mut("Checks") {
            def.art = tile(Color::rgb(0.0, 0.0, 1.0));
        }
        d.raster_effects_ppi = 24.0;
    });
}

/// Symbol art with a live object (an envelope that distorts a pattern fill) is placed again in
/// every frame: when the pattern's tile type changes, the next frame draws what a new renderer
/// draws.
#[test]
fn art_with_live_objects_draws_pattern_layout_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::live::EnvelopeOptions;
    use vectorcraft_doc::pattern::{PatternDef, TileType, pattern_paint};
    let mut d = Document::new(200.0, 100.0);
    // Pattern `Checks` is a red 10 × 10 square in a 20 × 20 tile.
    let square = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    let red = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let mut def = PatternDef::new("Checks", vec![Arc::new(Node::path(NodeId(900), square, red))]);
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    d.patterns.push(def);
    let band = vectorcraft_geom::shapes::rectangle(Rect::new(-80.0, -40.0, 80.0, 40.0));
    let filled = Node::path(d.alloc_id(), band, vectorcraft_doc::Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
    let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
    let options = EnvelopeOptions { distort_appearance: true, distort_pattern_fills: true, ..Default::default() };
    let envelope = Node::new(
        d.alloc_id(),
        NodeKind::Envelope {
            content: vec![Arc::new(filled)],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options,
            frame: Box::new(Affine::IDENTITY),
        },
    );
    let d = holder_doc(d, vec![envelope]);
    edit_draws_as_new(d, |d| {
        if let Some(def) = d.pattern_mut("Checks") {
            def.tile_type = TileType::BrickByRow { offset: 0.5 };
        }
    });
}

/// An envelope of `d` bending `content` with an arc warp (30%); with `patterns`, Distort
/// Appearance and Distort Pattern Fills are on, and the tiles of its pattern fills bend with it.
fn arc_envelope(d: &mut Document, content: Vec<Node>, patterns: bool) -> Node {
    let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
    let options = vectorcraft_doc::live::EnvelopeOptions { distort_appearance: patterns, distort_pattern_fills: patterns, ..Default::default() };
    let content = content.into_iter().map(Arc::new).collect();
    Node::new(
        d.alloc_id(),
        NodeKind::Envelope { content, kind: Box::new(kind), fidelity: 50.0, editing: false, options, frame: Box::new(Affine::IDENTITY) },
    )
}

/// An envelope that bends the tiles of its pattern fills is evaluated again when a pattern it tiles
/// changes its tile type or its overlap, as Pattern Options sets them: a band filled with the
/// pattern, and type whose characters are filled with it.
#[test]
fn live_objects_draw_pattern_layout_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::pattern::{Overlap, PatternDef, TileType, pattern_paint};
    use vectorcraft_doc::text::{CharStyle, TextObject};
    use vectorcraft_geom::shapes::rectangle;
    let bricks = |d: &mut Document| {
        if let Some(def) = d.pattern_mut("Checks") {
            def.tile_type = TileType::BrickByRow { offset: 0.5 };
        }
    };
    let mut d = Document::new(200.0, 100.0);
    d.patterns.push(checks(Color::rgb(1.0, 0.0, 0.0)));
    let band = filled(&mut d, rectangle(Rect::new(20.0, 20.0, 180.0, 80.0)), pattern_paint("Checks"), vec![]);
    let envelope = arc_envelope(&mut d, vec![band], true);
    edit_draws_as_new(with_art(d, vec![envelope]), bricks);
    // Pattern `Halves` is a 30 × 10 bar, red on the left and blue on the right, in a 20 × 20 tile:
    // each tile's bar reaches into the next tile, and the bar drawn in front depends on the overlap.
    let mut d = Document::new(200.0, 100.0);
    let half = |x: f64, c: Color| {
        let paint = vectorcraft_doc::Appearance::basic(Paint::solid(c), Paint::None, 0.0);
        Arc::new(Node::path(NodeId(900), rectangle(Rect::new(x, 0.0, x + 15.0, 10.0)), paint))
    };
    let mut halves = PatternDef::new("Halves", vec![half(0.0, Color::rgb(1.0, 0.0, 0.0)), half(15.0, Color::rgb(0.0, 0.0, 1.0))]);
    halves.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    d.patterns.push(halves);
    let band = filled(&mut d, rectangle(Rect::new(20.0, 20.0, 180.0, 80.0)), pattern_paint("Halves"), vec![]);
    let envelope = arc_envelope(&mut d, vec![band], true);
    edit_draws_as_new(with_art(d, vec![envelope]), |d| {
        if let Some(def) = d.pattern_mut("Halves") {
            def.overlap = Overlap { right_in_front: true, bottom_in_front: true };
        }
    });
    // Type whose characters are filled with `Checks`.
    let mut d = Document::new(200.0, 100.0);
    d.patterns.push(checks(Color::rgb(1.0, 0.0, 0.0)));
    let style = CharStyle { size: 90.0, fill: pattern_paint("Checks"), ..Default::default() };
    let text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(10.0, 80.0), "HHH", style))));
    let envelope = arc_envelope(&mut d, vec![text], true);
    edit_draws_as_new(with_art(d, vec![envelope]), bricks);
}

/// An envelope is evaluated again when a symbol or a pattern changes that it shows through other
/// art: a symbol shown in the art of the symbol it bends, a symbol that is the opacity mask of the
/// object it bends, and, with the tiles of its pattern fills bending with it, a pattern filling
/// the art of the symbol it bends and a symbol in the art of a pattern it tiles.
#[test]
fn live_objects_draw_symbol_and_pattern_art_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::pattern::{PatternDef, TileType, pattern_paint};
    use vectorcraft_geom::shapes::rectangle;
    // Symbol `S` ([`symbol_art`]) becomes a wide red bar.
    let with_s = || {
        let mut d = Document::new(200.0, 100.0);
        let art = symbol_art(&mut d);
        d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
        d
    };
    let red_bar = |d: &mut Document| {
        let bar = rectangle(Rect::new(-20.0, -4.0, 20.0, 4.0));
        let red = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        let s = d.symbols.iter().position(|s| s.name == "S").unwrap();
        d.symbols[s].art = Arc::new(Node::path(d.alloc_id(), bar, red));
    };
    let instance = |d: &mut Document, symbol: &str, xf: Affine| Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: symbol.into(), xf });
    // An instance of symbol `A`, whose art shows `S`.
    let mut d = with_s();
    let shown = instance(&mut d, "S", Affine::IDENTITY);
    let a = Node::group(d.alloc_id(), vec![Arc::new(shown)]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "A".into(), art: Arc::new(a) });
    let a = instance(&mut d, "A", Affine::translate((100.0, 50.0)) * Affine::scale(2.0));
    let envelope = arc_envelope(&mut d, vec![a], false);
    edit_draws_as_new(with_art(d, vec![envelope]), red_bar);
    // A black rectangle whose opacity mask is an instance of `S`.
    let mut d = with_s();
    let mask = instance(&mut d, "S", Affine::translate((100.0, 50.0)) * Affine::scale(3.0));
    let mut masked = filled(&mut d, rectangle(Rect::new(20.0, 20.0, 180.0, 80.0)), Paint::solid(Color::BLACK), vec![]);
    masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(mask, true)));
    let envelope = arc_envelope(&mut d, vec![masked], false);
    edit_draws_as_new(with_art(d, vec![envelope]), red_bar);
    // An instance of symbol `Banded`, a band filled with `Checks`, whose tile type turns to bricks.
    let mut d = Document::new(200.0, 100.0);
    d.patterns.push(checks(Color::rgb(1.0, 0.0, 0.0)));
    let band = filled(&mut d, rectangle(Rect::new(-80.0, -30.0, 80.0, 30.0)), pattern_paint("Checks"), vec![]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Banded".into(), art: Arc::new(band) });
    let banded = instance(&mut d, "Banded", Affine::translate((100.0, 50.0)));
    let envelope = arc_envelope(&mut d, vec![banded], true);
    edit_draws_as_new(with_art(d, vec![envelope]), |d| {
        if let Some(def) = d.pattern_mut("Checks") {
            def.tile_type = TileType::BrickByRow { offset: 0.5 };
        }
    });
    // A band filled with pattern `Dots`, whose art is an instance of `S` at half size.
    let mut d = with_s();
    let dot = instance(&mut d, "S", Affine::translate((10.0, 10.0)) * Affine::scale(0.5));
    let mut dots = PatternDef::new("Dots", vec![Arc::new(dot)]);
    dots.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    d.patterns.push(dots);
    let band = filled(&mut d, rectangle(Rect::new(20.0, 20.0, 180.0, 80.0)), pattern_paint("Dots"), vec![]);
    let envelope = arc_envelope(&mut d, vec![band], true);
    edit_draws_as_new(with_art(d, vec![envelope]), red_bar);
}

/// Installs live-effect plug-in `id`, replacing one with the same id: its effect draws `shape` in
/// place of the geometry it is given.
fn install_shape(id: &str, shape: &vectorcraft_geom::PathData) {
    use vectorcraft_plugins::wat as tpl;
    let output = serde_json::json!({"objects": [{"path": shape}]}).to_string();
    let module = wat::parse_str(tpl::returning(&tpl::manifest(id, "effect", "{}"), &output)).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
}

/// The triangle with corners (`x`, `y`), (`x` + `size`, `y`) and (`x` + `size` / 2, `y` + `size` / 2).
fn triangle(x: f64, y: f64, size: f64) -> vectorcraft_geom::PathData {
    use vectorcraft_geom::{PathData, Point, SubPath};
    let corners = [Point::new(x, y), Point::new(x + size, y), Point::new(x + size / 2.0, y + size / 2.0)];
    PathData::single(SubPath::polyline(&corners, true))
}

/// Draws `d` with live-effect plug-in `id` drawing `before` (`None`: not installed), installs the
/// plug-in drawing `after` (`None`: removes it) and draws `d` again, as [`edit_draws_as_new`] does.
#[track_caller]
fn plugin_change_draws_as_new(d: Document, id: &str, before: Option<vectorcraft_geom::PathData>, after: Option<vectorcraft_geom::PathData>) {
    match &before {
        Some(shape) => install_shape(id, shape),
        None => {
            vectorcraft_plugins::registry::remove(id);
        }
    }
    edit_draws_as_new(d, |_| match &after {
        Some(shape) => install_shape(id, shape),
        None => assert!(vectorcraft_plugins::registry::remove(id), "the plug-in was installed"),
    });
}

/// What plug-in effects draw follows the plug-ins installed. In the next frame, a square with a
/// plug-in effect is drawn as the plug-in installed again with another shape draws it, and as it
/// is without the plug-in after the plug-in is removed; after the plug-in is installed again, so
/// are the square with a drop shadow, the square blurred, and an instance of a symbol whose art, a
/// group kept between frames, holds the square. A square left of the view, alone and in a group,
/// is drawn where a plug-in installed for the first time draws it, in the view, and so is an
/// instance left of the view whose symbol's art holds the square, when the plug-in installed again
/// draws a shape that reaches the view.
#[test]
fn objects_with_plugin_effects_follow_the_plugins_installed() {
    let _fonts = fonts_steady();
    use vectorcraft_geom::shapes::rectangle;
    const ID: &str = "test.render.objects";
    let (small, big) = (Some(triangle(30.0, 40.0, 30.0)), Some(triangle(30.0, 40.0, 120.0)));
    // A black square of `d` from (`x`, `y`), 40 × 40, with the plug-in effect and `more` effects.
    let square = |d: &mut Document, x: f64, y: f64, more: Vec<vectorcraft_doc::Effect>| {
        let effects = std::iter::once(effect(&format!("plugin.{ID}"), serde_json::json!({}))).chain(more).collect();
        filled(d, rectangle(Rect::new(x, y, x + 40.0, y + 40.0)), Paint::solid(Color::BLACK), effects)
    };
    let alone = |x: f64, more: Vec<vectorcraft_doc::Effect>| {
        let mut d = Document::new(200.0, 100.0);
        let n = square(&mut d, x, 30.0, more);
        with_art(d, vec![n])
    };
    plugin_change_draws_as_new(alone(20.0, vec![]), ID, small.clone(), big.clone());
    plugin_change_draws_as_new(alone(20.0, vec![]), ID, small.clone(), None);
    plugin_change_draws_as_new(alone(20.0, vec![drop_shadow()]), ID, small.clone(), big.clone());
    let blur = effect("blur.gaussian", serde_json::json!({"radius": 1.0}));
    plugin_change_draws_as_new(alone(20.0, vec![blur]), ID, small.clone(), big.clone());
    // An instance of symbol `Shaped`, a group of the square around the origin.
    let mut d = Document::new(200.0, 100.0);
    let art = square(&mut d, -20.0, -20.0, vec![]);
    let art = Node::group(d.alloc_id(), vec![Arc::new(art)]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Shaped".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Shaped".into(), xf: Affine::translate((100.0, 50.0)) });
    plugin_change_draws_as_new(with_art(d, vec![instance]), ID, small.clone(), big);
    // Left of the view, the square is culled until the plug-in draws it in the view.
    plugin_change_draws_as_new(alone(-100.0, vec![]), ID, None, small.clone());
    let mut d = Document::new(200.0, 100.0);
    let n = square(&mut d, -100.0, 30.0, vec![]);
    let group = Node::group(d.alloc_id(), vec![Arc::new(n)]);
    plugin_change_draws_as_new(with_art(d, vec![group]), ID, None, small);
    // The instance's box lies from (-50, 30) to (-10, 70). Its cull bounds grow by how far the
    // effects in the art paint past the art, which the plug-in's first shape does not.
    let mut d = Document::new(200.0, 100.0);
    let art = square(&mut d, -20.0, -20.0, vec![]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Shaped".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Shaped".into(), xf: Affine::translate((-30.0, 50.0)) });
    plugin_change_draws_as_new(with_art(d, vec![instance]), ID, Some(triangle(-15.0, -15.0, 10.0)), Some(triangle(0.0, 0.0, 150.0)));
}

/// Installing a plug-in between two frames leaves the path a renderer keeps for a plain square and
/// the art it keeps for a group with a Transform effect: the second frame draws them from what was
/// kept.
#[test]
fn installing_a_plugin_keeps_what_is_kept_for_other_objects() {
    let _fonts = fonts_steady();
    use vectorcraft_geom::shapes::rectangle;
    let mut d = Document::new(200.0, 100.0);
    let square = filled(&mut d, rectangle(Rect::new(20.0, 30.0, 60.0, 70.0)), Paint::solid(Color::BLACK), vec![]);
    let inner = filled(&mut d, rectangle(Rect::new(120.0, 30.0, 160.0, 70.0)), Paint::solid(Color::BLACK), vec![]);
    let mut group = Node::group(d.alloc_id(), vec![Arc::new(inner)]);
    group.appearance.effects.push(effect("distort.transform", serde_json::json!({"moveH": 10.0})));
    let d = with_art(d, vec![square, group]);
    let [square, group] = [0, 1].map(|i| d.layers[0].children().unwrap()[i].clone());
    let kept = |r: &mut Renderer| {
        let path = r.geom.get(&(Arc::as_ptr(&square) as usize)).and_then(|e| e.path.clone()).unwrap();
        let art = r.fx_arts.get(&(Arc::as_ptr(&group) as usize), r.stamp).map(|e| e.1.clone()).unwrap();
        (path, art)
    };
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    r.render(&d, 200, 100, Affine::IDENTITY, &opts);
    let before = kept(&mut r);
    install_shape("test.render.other", &triangle(0.0, 0.0, 10.0));
    r.render(&d, 200, 100, Affine::IDENTITY, &opts);
    let after = kept(&mut r);
    assert!(Arc::ptr_eq(&before.0, &after.0), "the square's path is kept");
    assert!(Arc::ptr_eq(&before.1, &after.1), "the group's art is kept");
}

/// The art drawn for an object's effects follows the plug-ins installed: in the next frame, a group
/// with a plug-in effect, the same group feathered (Feather clips to the art's outline), an
/// instance of symbol `S` ([`symbol_art`]) and type, each with a plug-in effect, are drawn as the
/// plug-in installed again with another shape draws them.
#[test]
fn effect_art_follows_the_plugins_installed() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::text::{CharStyle, TextObject};
    use vectorcraft_geom::shapes::rectangle;
    const ID: &str = "test.render.art";
    let plugin = || effect(&format!("plugin.{ID}"), serde_json::json!({}));
    let (small, big) = (Some(triangle(30.0, 40.0, 30.0)), Some(triangle(30.0, 40.0, 120.0)));
    // A group of a black square, with the plug-in effect and `more` effects.
    let group = |more: Vec<vectorcraft_doc::Effect>| {
        let mut d = Document::new(200.0, 100.0);
        let square = filled(&mut d, rectangle(Rect::new(20.0, 30.0, 60.0, 70.0)), Paint::solid(Color::BLACK), vec![]);
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(square)]);
        group.appearance.effects = std::iter::once(plugin()).chain(more).collect();
        with_art(d, vec![group])
    };
    plugin_change_draws_as_new(group(vec![]), ID, small.clone(), big.clone());
    let feather = effect("stylize.feather", serde_json::json!({"radius": 2.0}));
    plugin_change_draws_as_new(group(vec![feather]), ID, small.clone(), big.clone());
    let mut d = Document::new(200.0, 100.0);
    let art = symbol_art(&mut d);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    let mut instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((100.0, 50.0)) });
    instance.appearance.effects.push(plugin());
    plugin_change_draws_as_new(with_art(d, vec![instance]), ID, small.clone(), big.clone());
    let mut d = Document::new(200.0, 100.0);
    let style = CharStyle { size: 30.0, ..Default::default() };
    let mut text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(10.0, 60.0), "Hamburg", style))));
    text.appearance.effects.push(plugin());
    plugin_change_draws_as_new(with_art(d, vec![text]), ID, small, big);
}

/// Live objects follow the plug-ins installed: in the next frame, an envelope that distorts the
/// appearance of a band with a plug-in effect, the same envelope without Distort Appearance, an
/// envelope that distorts the appearance of an instance of a symbol whose art has a plug-in effect,
/// and a blend of two squares with a plug-in effect are drawn as the plug-in installed again with
/// another shape draws them.
#[test]
fn live_objects_follow_the_plugins_installed() {
    let _fonts = fonts_steady();
    use vectorcraft_geom::shapes::rectangle;
    const ID: &str = "test.render.live";
    let plugin = || effect(&format!("plugin.{ID}"), serde_json::json!({}));
    let (small, big) = (Some(triangle(30.0, 40.0, 30.0)), Some(triangle(30.0, 40.0, 120.0)));
    let black = |d: &mut Document, r: Rect| filled(d, rectangle(r), Paint::solid(Color::BLACK), vec![plugin()]);
    let mut d = Document::new(200.0, 100.0);
    let band = black(&mut d, Rect::new(20.0, 30.0, 180.0, 70.0));
    let envelope = arc_envelope(&mut d, vec![band], true);
    plugin_change_draws_as_new(with_art(d, vec![envelope]), ID, small.clone(), big.clone());
    let mut d = Document::new(200.0, 100.0);
    let band = black(&mut d, Rect::new(20.0, 30.0, 180.0, 70.0));
    let envelope = arc_envelope(&mut d, vec![band], false);
    plugin_change_draws_as_new(with_art(d, vec![envelope]), ID, small.clone(), big.clone());
    // Symbol `Shaped` is a square with the plug-in effect.
    let mut d = Document::new(200.0, 100.0);
    let art = black(&mut d, Rect::new(-20.0, -20.0, 20.0, 20.0));
    d.symbols.push(vectorcraft_doc::Symbol { name: "Shaped".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Shaped".into(), xf: Affine::translate((100.0, 50.0)) });
    let envelope = arc_envelope(&mut d, vec![instance], true);
    plugin_change_draws_as_new(with_art(d, vec![envelope]), ID, small.clone(), big.clone());
    let mut d = Document::new(200.0, 100.0);
    let keys = [Rect::new(20.0, 30.0, 40.0, 50.0), Rect::new(140.0, 30.0, 160.0, 50.0)].map(|r| Arc::new(black(&mut d, r)));
    let spec = vectorcraft_doc::live::BlendSpec { spacing: vectorcraft_doc::live::BlendSpacing::Steps(3), ..Default::default() };
    let blend = Node::new(d.alloc_id(), NodeKind::Blend { children: keys.to_vec(), spec: Box::new(spec) });
    plugin_change_draws_as_new(with_art(d, vec![blend]), ID, small, big);
}

/// Pattern tiles and the rasters of raster effects follow the plug-ins installed when the art of a
/// pattern or of a symbol has a plug-in effect: in the next frame, a band filled with such a
/// pattern, the band with a drop shadow, and a group with a drop shadow around an instance of such
/// a symbol are drawn as the plug-in installed again with another shape draws them.
#[test]
fn pattern_and_symbol_art_with_plugin_effects_follow_the_plugins_installed() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
    use vectorcraft_geom::shapes::rectangle;
    const ID: &str = "test.render.tiles";
    let plugin = || effect(&format!("plugin.{ID}"), serde_json::json!({}));
    // Pattern `Shaped` is a 10 × 10 square with the plug-in effect in a 20 × 20 tile, inside which
    // the plug-in draws; a band of `d` is filled with it.
    let band = |d: &mut Document, effects: Vec<vectorcraft_doc::Effect>| {
        let square = filled(d, rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Paint::solid(Color::BLACK), vec![plugin()]);
        let mut def = PatternDef::new("Shaped", vec![Arc::new(square)]);
        def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
        d.patterns.push(def);
        filled(d, rectangle(Rect::new(20.0, 20.0, 180.0, 80.0)), pattern_paint("Shaped"), effects)
    };
    let (small, big) = (Some(triangle(0.0, 0.0, 10.0)), Some(triangle(0.0, 0.0, 20.0)));
    let mut d = Document::new(200.0, 100.0);
    let n = band(&mut d, vec![]);
    plugin_change_draws_as_new(with_art(d, vec![n]), ID, small.clone(), big.clone());
    let mut d = Document::new(200.0, 100.0);
    let n = band(&mut d, vec![drop_shadow()]);
    plugin_change_draws_as_new(with_art(d, vec![n]), ID, small, big);
    // Symbol `Shaped` is a square around the origin with the plug-in effect, which draws inside
    // the instance's box, from (80, 30) to (120, 70).
    let mut d = Document::new(200.0, 100.0);
    let art = filled(&mut d, rectangle(Rect::new(-20.0, -20.0, 20.0, 20.0)), Paint::solid(Color::BLACK), vec![plugin()]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Shaped".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Shaped".into(), xf: Affine::translate((100.0, 50.0)) });
    let mut group = Node::group(d.alloc_id(), vec![Arc::new(instance)]);
    group.appearance.effects.push(drop_shadow());
    plugin_change_draws_as_new(with_art(d, vec![group]), ID, Some(triangle(82.0, 32.0, 10.0)), Some(triangle(82.0, 32.0, 30.0)));
}

fn effect(id: &str, params: serde_json::Value) -> vectorcraft_doc::Effect {
    vectorcraft_doc::Effect { id: id.into(), params, visible: true }
}

/// A drop shadow 4 pt right and down, blurred 1 pt, at full opacity.
fn drop_shadow() -> vectorcraft_doc::Effect {
    effect("stylize.dropShadow", serde_json::json!({"x": 4, "y": 4, "blur": 1, "opacity": 100}))
}

/// A path of `d` filled with `fill` (no stroke), with `effects`.
fn filled(d: &mut Document, shape: vectorcraft_geom::PathData, fill: Paint, effects: Vec<vectorcraft_doc::Effect>) -> Node {
    let mut n = Node::path(d.alloc_id(), shape, vectorcraft_doc::Appearance::basic(fill, Paint::None, 0.0));
    n.appearance.effects = effects;
    n
}

/// `d` with `art` added to its first layer, bottom first.
fn with_art(mut d: Document, art: Vec<Node>) -> Document {
    let l = d.layers[0].id;
    for n in art {
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    d
}

/// Pattern `Checks`: a 10 × 10 square of `color` in a 20 × 20 tile.
fn checks(color: Color) -> vectorcraft_doc::pattern::PatternDef {
    let square = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    let art = Node::path(NodeId(900), square, vectorcraft_doc::Appearance::basic(Paint::solid(color), Paint::None, 0.0));
    let mut def = vectorcraft_doc::pattern::PatternDef::new("Checks", vec![Arc::new(art)]);
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    def
}

/// Raster effects on a document's own objects are made again when a pattern they paint with
/// changes: its art (as Done Editing Pattern writes it) under a blur, and its tile type (as
/// Pattern Options sets it) under a blur and a drop shadow.
#[test]
fn rasters_draw_pattern_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::pattern::{TileType, pattern_paint};
    use vectorcraft_geom::shapes::rectangle;
    let doc = || {
        let mut d = Document::new(200.0, 100.0);
        d.patterns.push(checks(Color::rgb(1.0, 0.0, 0.0)));
        let blur = effect("blur.gaussian", serde_json::json!({"radius": 2.0}));
        let blurred = filled(&mut d, rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), pattern_paint("Checks"), vec![blur]);
        let shadowed = filled(&mut d, rectangle(Rect::new(110.0, 10.0, 180.0, 80.0)), pattern_paint("Checks"), vec![drop_shadow()]);
        with_art(d, vec![blurred, shadowed])
    };
    edit_draws_as_new(doc(), |d| {
        if let Some(def) = d.pattern_mut("Checks") {
            def.art = checks(Color::rgb(0.0, 0.0, 1.0)).art;
        }
    });
    edit_draws_as_new(doc(), |d| {
        if let Some(def) = d.pattern_mut("Checks") {
            def.tile_type = TileType::BrickByRow { offset: 0.5 };
        }
    });
}

/// #1723: a pattern's tile is drawn again when what its art shows changes while the art itself
/// doesn't: a symbol an instance in it shows, another pattern it is painted with, or a brush it is
/// stroked with. So is a raster effect on an object painted with such a pattern.
#[test]
fn pattern_tiles_draw_edits_to_what_their_art_shows() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
    use vectorcraft_geom::shapes::rectangle;
    let tile = |name: &str, art: Node| {
        let mut def = PatternDef::new(name, vec![Arc::new(art)]);
        def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
        def
    };
    // Pattern `Stamps` shows symbol `S`; pattern `Outer` is a square painted with pattern `Checks`.
    let doc = |pattern: &str, effects: Vec<vectorcraft_doc::Effect>| {
        let mut d = Document::new(200.0, 100.0);
        let art = symbol_art(&mut d);
        d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
        let shown = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((10.0, 10.0)) });
        d.patterns.push(tile("Stamps", shown));
        d.patterns.push(checks(Color::rgb(1.0, 0.0, 0.0)));
        let square = filled(&mut d, rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), pattern_paint("Checks"), vec![]);
        d.patterns.push(tile("Outer", square));
        let painted = filled(&mut d, rectangle(Rect::new(10.0, 10.0, 190.0, 90.0)), pattern_paint(pattern), effects);
        with_art(d, vec![painted])
    };
    let new_symbol = |d: &mut Document| {
        let red = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        let bar = Node::path(d.alloc_id(), rectangle(Rect::new(-8.0, -2.0, 8.0, 2.0)), red);
        d.symbols[0].art = Arc::new(bar);
    };
    let new_checks = |d: &mut Document| {
        if let Some(def) = d.pattern_mut("Checks") {
            def.art = checks(Color::rgb(0.0, 0.0, 1.0)).art;
        }
    };
    let blur = || vec![effect("blur.gaussian", serde_json::json!({"radius": 2.0}))];
    edit_draws_as_new(doc("Stamps", vec![]), new_symbol);
    edit_draws_as_new(doc("Stamps", blur()), new_symbol);
    edit_draws_as_new(doc("Outer", vec![]), new_checks);
    edit_draws_as_new(doc("Outer", blur()), new_checks);
    // Pattern `Brushed` is a line stroked with a calligraphic brush; the brush gets wider.
    let mut d = Document::new(200.0, 100.0);
    let mut stroke = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    stroke.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    let brushed = Node::path(d.alloc_id(), vectorcraft_geom::shapes::line((2.0, 10.0).into(), (18.0, 10.0).into()), stroke);
    d.patterns.push(tile("Brushed", brushed));
    let painted = filled(&mut d, rectangle(Rect::new(10.0, 10.0, 190.0, 90.0)), pattern_paint("Brushed"), vec![]);
    edit_draws_as_new(with_art(d, vec![painted]), |d| {
        let mut lib = vectorcraft_brush::library(d);
        if let vectorcraft_brush::BrushKind::Calligraphic(c) = &mut lib[0].kind {
            c.size = 12.0;
        }
        vectorcraft_brush::store(d, &lib);
    });
}

/// Raster effects are made again when a symbol shown inside another symbol changes (a group's drop
/// shadow around an instance of the other symbol), when a brush definition changes (a brushed
/// stroke's drop shadow), and when an image's pixels change under the same key, as when a linked
/// file is read again (a blur).
#[test]
fn rasters_draw_symbol_brush_and_image_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_geom::shapes::{line, rectangle};
    // A group of an instance of symbol `A`, whose art shows symbol `S`, and a green square.
    let mut d = Document::new(200.0, 100.0);
    let art = symbol_art(&mut d);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    let shown = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::IDENTITY });
    let a = Node::group(d.alloc_id(), vec![Arc::new(shown)]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "A".into(), art: Arc::new(a) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "A".into(), xf: Affine::translate((50.0, 50.0)) });
    let green = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(0.0, 0.8, 0.0)), Paint::None, 0.0);
    let square = Node::path(d.alloc_id(), rectangle(Rect::new(120.0, 30.0, 160.0, 70.0)), green);
    let mut group = Node::group(d.alloc_id(), vec![Arc::new(instance), Arc::new(square)]);
    group.appearance.effects.push(drop_shadow());
    edit_draws_as_new(with_art(d, vec![group]), |d| {
        let bar = rectangle(Rect::new(-20.0, -4.0, 20.0, 4.0));
        let red = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        let s = d.symbols.iter().position(|s| s.name == "S").unwrap();
        d.symbols[s].art = Arc::new(Node::path(d.alloc_id(), bar, red));
    });
    // A line stroked with a calligraphic brush, with a drop shadow; the brush gets wider.
    let mut d = Document::new(200.0, 100.0);
    let mut stroke = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    stroke.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    let mut brushed = Node::path(d.alloc_id(), line((20.0, 50.0).into(), (180.0, 50.0).into()), stroke);
    brushed.appearance.effects.push(drop_shadow());
    edit_draws_as_new(with_art(d, vec![brushed]), |d| {
        let mut lib = vectorcraft_brush::library(d);
        if let vectorcraft_brush::BrushKind::Calligraphic(c) = &mut lib[0].kind {
            c.size = 20.0;
        }
        vectorcraft_brush::store(d, &lib);
    });
    // A 16 × 16 image drawn 80 × 80, blurred.
    edit_draws_as_new(blurred_image_doc("swapped", 104), |d| {
        d.images.insert("swapped".into(), ImageBlob::new("image/png", png(16, 16, 105)));
    });
}

/// A document showing a 16 × 16 image (`key`, pixels from `seed`) drawn 80 × 80 at (20, 10), under
/// a 1 pt Gaussian Blur.
fn blurred_image_doc(key: &str, seed: u8) -> Document {
    let mut d = Document::new(200.0, 100.0);
    d.images.insert(key.into(), ImageBlob::new("image/png", png(16, 16, seed)));
    let xf = Affine::translate((20.0, 10.0)) * Affine::scale(5.0);
    let im = ImageObject { key: key.into(), width: 16, height: 16, xf, link: None, placement: Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.appearance.effects.push(effect("blur.gaussian", serde_json::json!({"radius": 1.0})));
    with_art(d, vec![n])
}

/// Type in a family no font has, 30 pt at (10, 60), with effect `fx`.
fn missing_font_type(d: &mut Document, fx: vectorcraft_doc::Effect) -> Node {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let style = CharStyle { size: 30.0, font_family: "No Such Family".into(), ..Default::default() };
    let mut t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(10.0, 60.0), "Hamburg", style))));
    t.appearance.effects.push(fx);
    t
}

/// Raster effects are made again when a document or layer setting they read changes: the raster
/// resolution under De-Interlace, Page Knockout Group around a blurred group of translucent
/// objects, Document Setup's highlight of substituted fonts under a blur, and the layer's Dim
/// Images under an image's blur.
#[test]
fn rasters_draw_document_and_layer_setting_edits() {
    let _fonts = fonts_steady();
    use vectorcraft_geom::shapes::rectangle;
    // The path holds ten bars, 1 pt high and 2 pt apart.
    let mut d = Document::new(200.0, 100.0);
    d.raster_effects_ppi = 72.0;
    let mut bars = rectangle(Rect::new(20.0, 20.0, 180.0, 21.0));
    for k in 1..10 {
        let y = 20.0 + 2.0 * k as f64;
        bars.subpaths.extend(rectangle(Rect::new(20.0, y, 180.0, y + 1.0)).subpaths);
    }
    let striped = filled(&mut d, bars, Paint::solid(Color::BLACK), vec![effect("video.deinterlace", serde_json::json!({}))]);
    edit_draws_as_new(with_art(d, vec![striped]), |d| d.raster_effects_ppi = 24.0);
    // A blurred group of two overlapping squares at 50% opacity.
    let mut d = Document::new(200.0, 100.0);
    let mut squares = vec![];
    for (x, color) in [(40.0, Color::rgb(1.0, 0.0, 0.0)), (80.0, Color::rgb(0.0, 0.0, 1.0))] {
        let mut n = filled(&mut d, rectangle(Rect::new(x, 20.0, x + 60.0, 80.0)), Paint::solid(color), vec![]);
        n.opacity = 0.5;
        squares.push(Arc::new(n));
    }
    let mut group = Node::group(d.alloc_id(), squares);
    group.appearance.effects.push(effect("blur.gaussian", serde_json::json!({"radius": 1.0})));
    edit_draws_as_new(with_art(d, vec![group]), |d| d.page_knockout = true);
    // Blurred type in a missing family, with the substitution highlights on screen.
    let mut d = Document::new(200.0, 100.0);
    d.setup.highlight_substituted_fonts = false;
    let blurred = missing_font_type(&mut d, effect("blur.gaussian", serde_json::json!({"radius": 1.0})));
    let mut d = with_art(d, vec![blurred]);
    let highlights = RenderOptions { background: Some([255, 255, 255, 255]), highlight_substitutions: true, ..Default::default() };
    // The first lookup of the missing family catalogs the installed fonts, which changes the font
    // generation: a frame drawn first does that.
    Renderer::new().render(&d, 200, 100, Affine::IDENTITY, &highlights);
    draws_as_new(&mut d, &highlights, &highlights, |d| d.setup.highlight_substituted_fonts = true);
    // The layer of a blurred image dims its images to 50%.
    edit_draws_as_new(blurred_image_doc("dimmed", 106), |d| {
        if let NodeKind::Layer { dim_images, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
            *dim_images = Some(50);
        }
    });
}

/// Raster effects are made again when a view option their content is drawn with changes: Pixel
/// Preview's hard-edged images under a blur, Anti-aliased Artwork off under a blur, Overprint
/// Preview for overprinting characters in a blurred group, and isolation mode for a group inside a
/// blurred group, entered and left.
#[test]
fn rasters_follow_the_view_options() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::text::{CharStyle, TextObject};
    use vectorcraft_geom::shapes::{ellipse, rectangle};
    let on_white = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let unchanged = |_: &mut Document| {};
    draws_as_new(&mut blurred_image_doc("pixels", 107), &on_white, &RenderOptions { smooth_images: false, ..on_white.clone() }, unchanged);
    let mut d = Document::new(200.0, 100.0);
    let blur = effect("blur.gaussian", serde_json::json!({"radius": 0.5}));
    let oval = filled(&mut d, ellipse(Rect::new(30.0, 10.0, 170.0, 90.0)), Paint::solid(Color::BLACK), vec![blur]);
    let hard = RenderOptions { anti_alias: AntiAlias::None, ..on_white.clone() };
    draws_as_new(&mut with_art(d, vec![oval]), &on_white, &hard, unchanged);
    // A blurred group of a red rectangle and cyan type whose characters overprint.
    let mut d = Document::new(200.0, 100.0);
    let red = filled(&mut d, rectangle(Rect::new(0.0, 0.0, 200.0, 100.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), vec![]);
    let style = CharStyle { size: 50.0, fill: Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)), overprint_fill: true, ..Default::default() };
    let text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "Hamburg", style))));
    let mut group = Node::group(d.alloc_id(), vec![Arc::new(red), Arc::new(text)]);
    group.appearance.effects.push(effect("blur.gaussian", serde_json::json!({"radius": 1.0})));
    let overprints = RenderOptions { overprint_preview: true, ..on_white.clone() };
    draws_as_new(&mut with_art(d, vec![group]), &on_white, &overprints, unchanged);
    // A blurred group of a red square and a group holding a blue square: isolating the inner group
    // dims the red square.
    let mut d = Document::new(200.0, 100.0);
    let red = filled(&mut d, rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), vec![]);
    let blue = filled(&mut d, rectangle(Rect::new(110.0, 20.0, 170.0, 80.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), vec![]);
    let inner = Node::group(d.alloc_id(), vec![Arc::new(blue)]);
    let isolated = RenderOptions { isolated: Some(inner.id), ..on_white.clone() };
    let mut outer = Node::group(d.alloc_id(), vec![Arc::new(red), Arc::new(inner)]);
    outer.appearance.effects.push(effect("blur.gaussian", serde_json::json!({"radius": 1.0})));
    let mut d = with_art(d, vec![outer]);
    draws_as_new(&mut d, &on_white, &isolated, unchanged);
    draws_as_new(&mut d, &isolated, &on_white, unchanged);
}

/// A document for the proof previews: a cyan square, a magenta circle over it whose black stroke
/// overprints, a white square over the cyan one that overprints, a blurred rectangle in spot color
/// `Spot`, and an instance of symbol `S`, a group holding a yellow square.
fn proof_doc() -> Document {
    use vectorcraft_geom::shapes::{ellipse, rectangle};
    let mut d = Document::new(200.0, 100.0);
    let orange = Color::cmyk(0.0, 0.6, 1.0, 0.0);
    d.swatches.push(vectorcraft_color::Swatch { name: "Spot".into(), paint: Paint::solid(orange), global: true, spot: true });
    let cyan = filled(&mut d, rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)), vec![]);
    let black = Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0));
    let ring = vectorcraft_doc::Appearance::basic(Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)), black, 4.0);
    let mut circle = Node::path(d.alloc_id(), ellipse(Rect::new(40.0, 20.0, 100.0, 80.0)), ring);
    circle.appearance.stroke_mut().unwrap().overprint = true;
    let mut white = filled(&mut d, rectangle(Rect::new(20.0, 30.0, 40.0, 50.0)), Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 0.0)), vec![]);
    white.appearance.fill_mut().unwrap().overprint = true;
    let spot = Paint::Solid { color: orange, swatch: Some("Spot".into()), tint: 1.0 };
    let blur = effect("blur.gaussian", serde_json::json!({"radius": 1.0}));
    let spot = filled(&mut d, rectangle(Rect::new(110.0, 10.0, 150.0, 50.0)), spot, vec![blur]);
    let yellow = filled(&mut d, rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)), Paint::solid(Color::cmyk(0.0, 0.0, 1.0, 0.0)), vec![]);
    let art = Node::group(d.alloc_id(), vec![Arc::new(yellow)]);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((175.0, 75.0)) });
    with_art(d, vec![cyan, circle, white, spot, instance])
}

/// Overprint Preview, and Separations Preview showing `plates`, which turns Overprint Preview on
/// too (as the canvas draws them).
fn proof_views(plates: &[&str]) -> (RenderOptions, RenderOptions) {
    let overprint = RenderOptions { overprint_preview: true, ..Default::default() };
    let proof = proof::ProofSetup { separations: Some(plates.iter().map(|p| p.to_string()).collect()), ..Default::default() };
    (overprint.clone(), RenderOptions { proof: Some(proof), ..overprint })
}

/// [`proof_views`] with every plate of [`proof_doc`].
fn every_plate() -> (RenderOptions, RenderOptions) {
    proof_views(&["Cyan", "Magenta", "Yellow", "Black", "Spot"])
}

/// With Overprint Preview or Separations Preview on, a second frame of an unchanged document draws
/// the art the first prepared for the preview: it makes no geometry entries. After an object
/// moves, a frame makes them for the object and its layer only. A frame drawn with neither preview
/// lets go of the art, and each frame drawn with a preview draws what a new renderer draws.
#[test]
fn proof_previews_keep_their_art_between_frames() {
    let _fonts = fonts_steady();
    let d = proof_doc();
    let (overprint, separations) = every_plate();
    let frame = |r: &mut Renderer, d: &Document, opts: &RenderOptions| r.render(d, 200, 100, Affine::IDENTITY, opts).pixels;
    let geometry = |r: &Renderer| r.geom.keys().copied().collect::<std::collections::HashSet<_>>();
    for opts in [&overprint, &separations] {
        let mut r = Renderer::new();
        let first = frame(&mut r, &d, opts);
        let kept = geometry(&r);
        assert!(frame(&mut r, &d, opts) == first);
        assert_eq!(geometry(&r), kept, "the second frame makes no geometry entries");
        // The cyan square moves 5 pt right.
        let mut moved = d.clone();
        let square = moved.layers[0].children().unwrap()[0].id;
        moved.node_mut(square).unwrap().transform(Affine::translate((5.0, 0.0)), false);
        assert!(frame(&mut r, &moved, opts) == frame(&mut Renderer::new(), &moved, opts));
        assert_eq!(geometry(&r).difference(&kept).count(), 2, "the square and its layer");
        frame(&mut r, &moved, &RenderOptions::default());
        assert!(r.proof_art.is_none(), "drawn without the previews, the renderer keeps no art for them");
    }
}

/// With Overprint Preview or Separations Preview on, a renderer draws what a new renderer draws
/// after a change to the art or to what the previews read: an object recolored, Discard White
/// Overprint turned off, a symbol's art recolored (its instance shows the plates of the new art),
/// the spot color redefined, a spot color in a color group redefined, the Spot Colors option set
/// to CMYK values, a plate hidden, Simulate Paper Color, another proof profile, and one preview
/// after the other.
#[test]
fn proof_previews_draw_changes_as_a_new_renderer_does() {
    let _fonts = fonts_steady();
    let (overprint, separations) = every_plate();
    let unchanged = |_: &mut Document| {};
    let recolor_square = |d: &mut Document| {
        let square = d.layers[0].children().unwrap()[0].id;
        d.node_mut(square).unwrap().appearance.set_fill(Paint::solid(Color::cmyk(0.0, 0.0, 1.0, 0.0)));
    };
    let recolor_symbol = |d: &mut Document| {
        let mut art = (*d.symbols[0].art).clone();
        if let Some(square) = art.children_mut().and_then(|c| c.first_mut()) {
            Arc::make_mut(square).appearance.set_fill(Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)));
        }
        d.symbols[0].art = Arc::new(art);
    };
    for opts in [&overprint, &separations] {
        draws_as_new(&mut proof_doc(), opts, opts, recolor_square);
        draws_as_new(&mut proof_doc(), opts, opts, |d| d.setup.discard_white_overprint = false);
    }
    draws_as_new(&mut proof_doc(), &separations, &separations, recolor_symbol);
    draws_as_new(&mut proof_doc(), &separations, &separations, |d| {
        if let Some(s) = d.swatch_mut("Spot") {
            s.paint = Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0));
        }
    });
    // A square in spot color `Lab Spot`, defined in Lab in a color group, on its own plate.
    let lab_doc = || {
        let mut d = proof_doc();
        let lab = Color::lab(50.0, 60.0, 40.0);
        let swatch = vectorcraft_color::Swatch { name: "Lab Spot".into(), paint: Paint::solid(lab), global: true, spot: true };
        d.swatch_groups.push(vectorcraft_color::SwatchGroup { name: "Lab".into(), swatches: vec![swatch] });
        let fill = Paint::Solid { color: lab, swatch: Some("Lab Spot".into()), tint: 1.0 };
        let square = filled(&mut d, vectorcraft_geom::shapes::rectangle(Rect::new(115.0, 60.0, 155.0, 95.0)), fill, vec![]);
        with_art(d, vec![square])
    };
    let lab_plate = proof_views(&["Cyan", "Magenta", "Yellow", "Black", "Lab Spot"]).1;
    draws_as_new(&mut lab_doc(), &lab_plate, &lab_plate, |d| {
        if let Some(s) = d.swatch_mut("Lab Spot") {
            s.paint = Paint::solid(Color::lab(70.0, -40.0, 30.0));
        }
    });
    draws_as_new(&mut lab_doc(), &lab_plate, &lab_plate, |d| d.spot_use_lab = false);
    let hidden = proof_views(&["Magenta", "Yellow", "Black", "Spot"]).1;
    draws_as_new(&mut proof_doc(), &separations, &hidden, unchanged);
    let with = |f: fn(&mut proof::ProofSetup)| {
        let mut opts = separations.clone();
        if let Some(p) = &mut opts.proof {
            f(p);
        }
        opts
    };
    draws_as_new(&mut proof_doc(), &separations, &with(|p| p.simulate_paper = true), unchanged);
    let device = with(|p| p.target = proof::ProofTarget::Cmyk(vectorcraft_color::cms::DEVICE_CMYK.into()));
    draws_as_new(&mut proof_doc(), &separations, &device, unchanged);
    let cyan = proof_views(&["Cyan"]).1;
    draws_as_new(&mut proof_doc(), &overprint, &cyan, unchanged);
    draws_as_new(&mut proof_doc(), &cyan, &overprint, unchanged);
    // On the Yellow plate the instance is solid while its art is yellow, and blank once it is cyan.
    let yellow = proof_views(&["Yellow"]).1;
    let mut d = proof_doc();
    let mut r = Renderer::new();
    let instance = |r: &mut Renderer, d: &Document| {
        let pixels = r.render(d, 200, 100, Affine::IDENTITY, &yellow).pixels;
        Rendered { width: 200, height: 100, pixels }.pixel(175, 75)
    };
    assert_eq!(instance(&mut r, &d), [0, 0, 0, 255]);
    recolor_symbol(&mut d);
    assert_eq!(instance(&mut r, &d), [255, 255, 255, 255]);
}

/// A thumbnail of an object under a filter that treats color channels apart (Color Halftone) is
/// made again when the document's color mode changes: CMYK documents screen ink channels, RGB
/// documents light ones.
#[test]
fn filtered_thumbnails_follow_the_color_mode() {
    let _fonts = fonts_steady();
    use vectorcraft_geom::shapes::rectangle;
    let mut d = Document::new(200.0, 100.0);
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    let halftone = effect("pixelate.colorHalftone", serde_json::json!({"maxRadius": 4}));
    let orange = filled(&mut d, rectangle(Rect::new(20.0, 20.0, 180.0, 80.0)), Paint::solid(Color::rgb(1.0, 0.5, 0.0)), vec![halftone]);
    let id = orange.id;
    let mut d = with_art(d, vec![orange]);
    let mut r = Renderer::new();
    let before = r.render_thumbnail(&d, id, 128).unwrap().pixels;
    d.color_mode = vectorcraft_doc::ColorMode::Rgb;
    let after = r.render_thumbnail(&d, id, 128).unwrap().pixels;
    assert!(after != before, "the color mode changes the thumbnail");
    assert!(after == Renderer::new().render_thumbnail(&d, id, 128).unwrap().pixels, "a new renderer draws the same");
}

/// Symbol instances draw what groups of their art, stained and placed, draw: in every frame, when
/// two instances placed alike are stained differently, when the art is a single path, and with a
/// cache too small for all of the art. Art no longer drawn leaves the cache after a few frames.
#[test]
fn symbol_instances_draw_as_groups_of_their_placed_art() {
    let _fonts = fonts_steady();
    let mut d = symbol_doc();
    let l = d.layers[0].id;
    // An instance placed like the first one lies over it, stained green.
    let mut over = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((30.0, 30.0)) });
    over.appearance.set_fill(Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
    d.insert(Some(l), 1, over).unwrap();
    // Symbol `U` is a single path, a circle with a black stroke.
    let circle = vectorcraft_geom::shapes::ellipse(Rect::new(-6.0, -6.0, 6.0, 6.0));
    let ap = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.8, 0.0)), Paint::solid(Color::BLACK), 1.0);
    let circle = Node::path(d.alloc_id(), circle, ap);
    d.symbols.push(vectorcraft_doc::Symbol { name: "U".into(), art: Arc::new(circle) });
    let id = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::new(id, NodeKind::SymbolInstance { symbol: "U".into(), xf: Affine::translate((175.0, 25.0)) })).unwrap();
    // `groups` holds the same art in place of the instances: groups, and the circle as a path.
    let mut groups = d.clone();
    let placed = d.layers[0].children().unwrap().iter().map(|c| match &c.kind {
        NodeKind::SymbolInstance { symbol, xf } => {
            let sym = d.symbols.iter().find(|s| s.name == *symbol).unwrap();
            let mut art = instance_art(&sym.art, c);
            art.transform(*xf, false);
            Arc::new(art)
        }
        _ => c.clone(),
    });
    *groups.children_mut(Some(l)).unwrap() = placed.collect();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let want = Renderer::new().render(&groups, 200, 100, Affine::IDENTITY, &opts).pixels;
    let mut r = Renderer::new();
    r.placed_arts = FrameCache::new(lru::PLACED_ART_BUDGET, 0);
    for frame in 0..3 {
        assert!(r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels == want, "frame {frame}");
    }
    // A cache that holds half the bytes of that art keeps what fits and places the rest every frame.
    let all = r.placed_arts.bytes();
    let mut small = Renderer::new();
    small.placed_arts = FrameCache::new(all / 2, 0);
    for frame in 0..3 {
        assert!(small.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels == want, "frame {frame} over the budget");
        assert!(!small.placed_arts.is_empty() && small.placed_arts.bytes() <= all / 2, "{} of {all} bytes", small.placed_arts.bytes());
    }
    let empty = Document::new(10.0, 10.0);
    for _ in 0..5 {
        r.render(&empty, 10, 10, Affine::IDENTITY, &opts);
    }
    assert_eq!((r.placed_arts.len(), r.placed_arts.bytes()), (0, 0));
}

/// Inline graphics in type draw the art placed for them in an earlier frame, as instances do.
#[test]
fn inline_graphics_keep_their_placed_art_between_frames() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::text::{CharStyle, InlineArt, TextObject, TextRun};
    let mut d = Document::new(200.0, 100.0);
    let art = symbol_art(&mut d);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    let style = CharStyle { size: 24.0, ..Default::default() };
    let mut t = TextObject::point(vectorcraft_geom::Point::new(10.0, 50.0), "A", style.clone());
    t.runs.push(TextRun::inline(InlineArt::new("S"), style.clone()));
    t.runs.push(TextRun::new("B", style));
    let (l, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let first = r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let placed = placed_copies(&r, &[&d]);
    assert_eq!(placed.len(), 2, "the square and the line of the inline graphic");
    assert!(r.render(&d, 200, 100, Affine::IDENTITY, &opts).pixels == first);
    assert_eq!(placed_copies(&r, &[&d]), placed, "the second frame draws the art the first placed");
}

/// Each run draws exactly the outlines it drew when kept as a path of its own: its glyphs and
/// decorations in layout order (the whole text's path is kept once, the runs as spans of it).
#[test]
fn text_runs_draw_their_own_glyphs_from_the_shared_outlines() {
    let _fonts = fonts_steady();
    use vectorcraft_doc::text::{CharStyle, TextObject, TextRun};
    let plain = CharStyle { size: 24.0, ..Default::default() };
    let under = CharStyle { underline: true, ..plain.clone() };
    let mut t = TextObject::point(vectorcraft_geom::Point::new(10.0, 40.0), "Ab", plain.clone());
    t.runs.push(TextRun::new("cd", under));
    t.runs.push(TextRun::new("ef", plain));
    let g = text_geom(&t);
    let db = vectorcraft_text::FontDb::global();
    let layout = vectorcraft_text::layout(db, &t);
    let mut want = vec![BezPath::new(); t.runs.len()];
    for gl in &layout.glyphs {
        want[gl.run].extend(gl.outline.iter());
    }
    for (run, bar) in vectorcraft_text::decorations(&layout, db, &t) {
        want[run].extend(bar.iter());
    }
    for (i, w) in want.iter().enumerate() {
        assert_eq!(g.run(i).elements(), w.elements(), "run {i}");
    }
    assert!(g.run(7).elements().is_empty());
}
