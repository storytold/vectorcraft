//! Character panel › Anti-aliasing Method: type drawn as pixels with None, Sharp, Crisp or Strong
//! edges.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::pattern::{PatternDef, PatternEdit};
use vectorcraft_doc::text::{CharStyle, InlineArt, TextObject, TextRun, TypeAntiAlias};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, Node, NodeKind, TextKind};
use vectorcraft_geom::Point;
use vectorcraft_geom::shapes::rectangle;

use crate::options::MAX_CONTEXT_SIDE;
use crate::*;

const W: u32 = 260;
const H: u32 = 90;

/// A page with "Hamburg" in 40 pt black type drawn with `method`, and `fill` as the type object's
/// own fill (instead of the characters' fill) when given.
fn page(method: TypeAntiAlias, fill: Option<FillLayer>) -> Document {
    let mut d = Document::new(f64::from(W), f64::from(H));
    let mut t = TextObject::point(Point::new(10.3, 60.4), "Hamburg", CharStyle { size: 40.0, ..Default::default() });
    t.anti_alias = method;
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    if let Some(fl) = fill {
        if let NodeKind::Text(t) = &mut n.kind {
            t.runs[0].style.fill = Paint::None;
        }
        n.appearance.items.push(AppearanceItem::Fill(fl));
    }
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// The opacity of each pixel of `d` drawn with `opts` on a transparent page.
fn alphas(d: &Document, opts: &RenderOptions) -> Vec<u8> {
    Renderer::new().render(d, W, H, Affine::IDENTITY, opts).pixels.chunks(4).map(|p| p[3]).collect()
}

/// Pixels the type covers only in part.
fn partial(a: &[u8]) -> usize {
    a.iter().filter(|v| **v > 0 && **v < 255).count()
}

/// The first pixels of `drawn` (type drawn with `m`) that are not the share `edge_coverage` gives
/// the same pixel of `sharp` (the type drawn with Sharp), within a step of rounding: (pixel,
/// Sharp, drawn).
fn off_the_share(m: TypeAntiAlias, sharp: &[u8], drawn: &[u8]) -> Vec<(usize, u8, u8)> {
    sharp
        .iter()
        .zip(drawn)
        .enumerate()
        .filter(|(_, (c, v))| {
            // One step of rounding either way in the coverage.
            let near = [c.saturating_sub(1), **c, c.saturating_add(1)].map(|c| edge_coverage(m, c));
            let (lo, hi) = (near.iter().min().copied().unwrap_or(0), near.iter().max().copied().unwrap_or(255));
            **v < lo.saturating_sub(1) || **v > hi.saturating_add(1)
        })
        .map(|(i, (c, v))| (i, *c, *v))
        .take(5)
        .collect()
}

/// On the canvas each method gives every pixel the share of it that `edge_coverage` gives the
/// share Sharp draws: None paints whole pixels from half coverage up, Crisp doubles the contrast
/// around half coverage, and Strong doubles the coverage.
#[test]
fn each_method_draws_the_share_of_each_pixel_it_defines() {
    let opts = RenderOptions::default();
    let sharp = alphas(&page(TypeAntiAlias::Sharp, None), &opts);
    assert!(partial(&sharp) > 200, "Sharp draws smooth edges: {} partial pixels", partial(&sharp));
    for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
        let drawn = alphas(&page(m, None), &opts);
        assert_ne!(drawn, sharp, "{m:?} changes the edges");
        let wrong = off_the_share(m, &sharp, &drawn);
        assert!(wrong.is_empty(), "{m:?}: (pixel, Sharp, drawn) {wrong:?}");
    }
    assert_eq!(partial(&alphas(&page(TypeAntiAlias::None, None), &opts)), 0, "None paints whole pixels");
}

/// `edge_coverage` gives each method the share its documentation defines: None shows all of a
/// pixel from half coverage (127) up and none of it below, Sharp shows the coverage, Crisp shows
/// `2c - 0.5` and Strong shows `2c`, each kept between 0 and all.
#[test]
fn edge_coverage_gives_each_methods_documented_share() {
    // Coverage, then what None, Sharp, Crisp and Strong show of the pixel (out of 255).
    let table: [(u8, [u8; 4]); 11] = [
        (0, [0, 0, 0, 0]),
        (1, [0, 1, 0, 2]),
        (63, [0, 63, 0, 126]),
        (64, [0, 64, 0, 128]),
        (65, [0, 65, 2, 130]),
        (126, [0, 126, 124, 252]),
        (127, [255, 127, 126, 254]),
        (128, [255, 128, 128, 255]),
        (191, [255, 191, 254, 255]),
        (192, [255, 192, 255, 255]),
        (255, [255, 255, 255, 255]),
    ];
    for (c, shown) in table {
        let got = [TypeAntiAlias::None, TypeAntiAlias::Sharp, TypeAntiAlias::Crisp, TypeAntiAlias::Strong].map(|m| edge_coverage(m, c));
        assert_eq!(got, shown, "coverage {c}: None, Sharp, Crisp, Strong");
    }
}

/// A raster export draws its art at several times its size and averages it down; type with None
/// still has hard edges on the exported pixels, Crisp has fewer partly covered pixels than Sharp,
/// and Strong draws heavier type.
#[test]
fn an_exports_pixels_take_the_method() {
    let opts = RenderOptions { supersample: true, precise: true, ..Default::default() };
    let sharp = alphas(&page(TypeAntiAlias::Sharp, None), &opts);
    let none = alphas(&page(TypeAntiAlias::None, None), &opts);
    let crisp = alphas(&page(TypeAntiAlias::Crisp, None), &opts);
    let strong = alphas(&page(TypeAntiAlias::Strong, None), &opts);
    assert!(partial(&sharp) > 200, "the export smooths Sharp type: {}", partial(&sharp));
    assert_eq!(partial(&none), 0, "None type has hard edges in the export");
    assert!(none.iter().filter(|v| **v == 255).count() > 1000);
    assert!(partial(&crisp) < partial(&sharp), "Crisp: {} partial pixels, Sharp {}", partial(&crisp), partial(&sharp));
    let ink = |a: &[u8]| a.iter().map(|v| u64::from(*v)).sum::<u64>();
    assert!(ink(&strong) > ink(&sharp), "Strong draws heavier type");
}

/// A frame drawn with hard edges (Anti-aliased Artwork off, an export's Anti-aliasing None) draws
/// the type of every method as it draws Sharp type.
#[test]
fn hard_edged_frames_draw_every_method_alike() {
    for opts in [
        RenderOptions { anti_alias: AntiAlias::None, ..Default::default() },
        RenderOptions { anti_alias: AntiAlias::None, precise: true, supersample: true, ..Default::default() },
    ] {
        let sharp = alphas(&page(TypeAntiAlias::Sharp, None), &opts);
        assert_eq!(partial(&sharp), 0);
        for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
            assert_eq!(alphas(&page(m, None), &opts), sharp, "{m:?}");
        }
    }
}

/// Crisp and Strong reshape the edges of type that is drawn with a translucent fill, and keep
/// its opacity inside the characters; None keeps it too.
#[test]
fn the_methods_keep_the_opacity_of_the_paint() {
    let half = FillLayer { opacity: 0.5, ..FillLayer::new(Paint::solid(Color::BLACK)) };
    let opts = RenderOptions::default();
    let sharp = alphas(&page(TypeAntiAlias::Sharp, Some(half.clone())), &opts);
    let most = |a: &[u8]| a.iter().copied().max().unwrap_or(0);
    assert!((126..=129).contains(&most(&sharp)), "half opaque: {}", most(&sharp));
    for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
        let drawn = alphas(&page(m, Some(half.clone())), &opts);
        assert!((126..=129).contains(&most(&drawn)), "{m:?} keeps the fill's opacity: {}", most(&drawn));
        assert_ne!(drawn, sharp, "{m:?} changes the edges");
    }
}

/// In an export that composites with floating point, None, Crisp and Strong type composite its
/// translucent fills as precisely as Sharp type: inside the characters each method draws the
/// pixels Sharp draws, within the one step of rounding that keeping the type's pixels as 8-bit
/// values adds.
#[test]
fn the_methods_keep_precise_compositing() {
    let fills = [(Color::rgb(0.9, 0.2, 0.1), 0.37), (Color::rgb(0.1, 0.6, 0.8), 0.53), (Color::rgb(0.3, 0.1, 0.7), 0.29)];
    let opts = RenderOptions { precise: true, ..Default::default() };
    let draw = |m: TypeAntiAlias| {
        let mut d = page(m, Some(FillLayer { opacity: 0.41, ..FillLayer::new(Paint::solid(Color::rgb(0.2, 0.7, 0.3))) }));
        let id = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
        let n = d.node_mut(id).unwrap();
        for (c, o) in fills {
            n.appearance.items.push(AppearanceItem::Fill(FillLayer { opacity: o, ..FillLayer::new(Paint::solid(c)) }));
        }
        Renderer::new().render(&d, W, H, Affine::IDENTITY, &opts).pixels
    };
    let sharp = draw(TypeAntiAlias::Sharp);
    let most = sharp.chunks(4).map(|p| p[3]).max().unwrap_or(0);
    let inside: Vec<usize> = sharp.chunks(4).enumerate().filter(|(_, p)| p[3] == most).map(|(i, _)| i).collect();
    assert!(inside.len() > 200, "{} pixels inside the characters", inside.len());
    for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
        let drawn = draw(m);
        let px = |p: &[u8], i: usize| [p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]];
        let off = |i: usize| px(&drawn, i).iter().zip(px(&sharp, i)).any(|(a, b)| a.abs_diff(b) > 1);
        let wrong: Vec<_> = inside.iter().filter(|i| off(**i)).take(5).map(|i| (*i, px(&sharp, *i), px(&drawn, *i))).collect();
        assert!(wrong.is_empty(), "{m:?}: (pixel, Sharp, drawn) {wrong:?}");
    }
}

/// Type whose own fill multiplies with the art below it is drawn as Sharp draws it, so the fill
/// still multiplies with that art.
#[test]
fn type_with_a_blended_fill_is_drawn_as_sharp() {
    let multiply = FillLayer { blend: vectorcraft_color::BlendMode::Multiply, ..FillLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0))) };
    let on_orange = |m: TypeAntiAlias| {
        let mut d = page(m, Some(multiply.clone()));
        let orange = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.5, 0.0)), Paint::None, 0.0);
        let back = Node::path(d.alloc_id(), rectangle(Rect::new(0.0, 0.0, f64::from(W), f64::from(H))), orange);
        let l = d.layers[0].id;
        d.insert(Some(l), 0, back).unwrap();
        Renderer::new().render(&d, W, H, Affine::IDENTITY, &RenderOptions::default()).pixels
    };
    let sharp = on_orange(TypeAntiAlias::Sharp);
    for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
        assert_eq!(on_orange(m), sharp, "{m:?}");
    }
}

/// Type with an inline graphic is drawn as Sharp draws it, so the graphic, which multiplies with
/// the art below it here, is drawn as before.
#[test]
fn type_with_an_inline_graphic_is_drawn_as_sharp() {
    let on_orange = |m: TypeAntiAlias| {
        let mut d = Document::new(f64::from(W), f64::from(H));
        let orange = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.5, 0.0)), Paint::None, 0.0);
        let back = Node::path(d.alloc_id(), rectangle(Rect::new(0.0, 0.0, f64::from(W), f64::from(H))), orange);
        let blue = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
        let mut art = Node::path(d.alloc_id(), rectangle(Rect::new(0.0, -30.0, 30.0, 0.0)), blue);
        art.blend = vectorcraft_color::BlendMode::Multiply;
        d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
        let style = CharStyle { size: 40.0, ..Default::default() };
        let mut t = TextObject::point(Point::new(10.3, 60.4), "I", style.clone());
        t.runs.push(TextRun::inline(InlineArt::new("S"), style));
        t.anti_alias = m;
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, back).unwrap();
        d.insert(Some(l), 1, n).unwrap();
        d.resolve_inline_art();
        Renderer::new().render(&d, W, H, Affine::IDENTITY, &RenderOptions::default()).pixels
    };
    let sharp = on_orange(TypeAntiAlias::Sharp);
    for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
        assert_eq!(on_orange(m), sharp, "{m:?}");
    }
}

/// Type with a color glyph (a bitmap in `sbix` or `CBDT`, or COLR v0 or v1 layers) is drawn as
/// Sharp draws it, so the glyph's own opacity is kept.
#[test]
fn type_with_a_color_glyph_is_drawn_as_sharp() {
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, colour_bitmap_family, colour_bitmap_font, colr_family, colr_font};
    let _adding = crate::tests_fontchange::adding_fonts();
    let fonts = [
        (colour_bitmap_family(true), colour_bitmap_font(true)),
        (colour_bitmap_family(false), colour_bitmap_font(false)),
        (colr_family(false), colr_font(false)),
        (colr_family(true), colr_font(true)),
    ];
    for (family, font) in fonts {
        let Some(font) = font else { panic!("{family}: the test font is built") };
        vectorcraft_text::FontDb::global().add_font(font);
        let draw = |m: TypeAntiAlias| {
            let mut d = Document::new(160.0, 160.0);
            let style = CharStyle { font_family: family.into(), size: 100.0, ..Default::default() };
            let mut t = TextObject::point(Point::new(20.3, 120.4), &COLOUR_CHAR.to_string(), style);
            t.anti_alias = m;
            let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
            let l = d.layers[0].id;
            d.insert(Some(l), 0, n).unwrap();
            Renderer::new().render(&d, 160, 160, Affine::IDENTITY, &RenderOptions::default()).pixels
        };
        let sharp = draw(TypeAntiAlias::Sharp);
        assert!(sharp.chunks(4).any(|p| p[3] > 0), "{family}: the glyph is drawn");
        for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
            assert!(draw(m) == sharp, "{family}, {m:?}");
        }
    }
}

/// In a supersampled export, None type in a group drawn offscreen (it isolates art that blends)
/// still has hard edges on the exported pixels, wherever the group's layer starts.
#[test]
fn type_in_an_offscreen_group_keeps_the_exports_pixels() {
    let opts = RenderOptions { supersample: true, precise: true, ..Default::default() };
    for x in [10.3, 10.55, 10.8, 11.05] {
        let mut d = Document::new(f64::from(W), f64::from(H));
        let mut t = TextObject::point(Point::new(x, 60.4), "Hamburg", CharStyle { size: 40.0, ..Default::default() });
        t.anti_alias = TypeAntiAlias::None;
        let text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        let black = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
        let mut square = Node::path(d.alloc_id(), rectangle(Rect::new(220.0, 10.0, 250.0, 30.0)), black);
        square.blend = vectorcraft_color::BlendMode::Multiply;
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(square), Arc::new(text)]);
        group.isolate = true;
        let l = d.layers[0].id;
        d.insert(Some(l), 0, group).unwrap();
        let a = alphas(&d, &opts);
        assert!(a.iter().filter(|v| **v == 255).count() > 1000, "x {x}");
        assert_eq!(partial(&a), 0, "x {x}: None type has hard edges");
    }
}

/// In a supersampled export, None type in a layer drawn offscreen over a copy of the art below it
/// (the layer multiplies, and art inside it blends) still has hard edges on the exported pixels,
/// wherever the layer's pixels start.
#[test]
fn type_in_an_offscreen_layer_keeps_the_exports_pixels() {
    let opts = RenderOptions { supersample: true, precise: true, ..Default::default() };
    for x in [10.3, 10.55, 10.8, 11.05] {
        let mut d = Document::new(f64::from(W), f64::from(H));
        let mut t = TextObject::point(Point::new(x, 60.4), "Hamburg", CharStyle { size: 40.0, ..Default::default() });
        t.anti_alias = TypeAntiAlias::None;
        let text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        let black = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
        let mut square = Node::path(d.alloc_id(), rectangle(Rect::new(220.0, 10.0, 250.0, 30.0)), black);
        square.blend = vectorcraft_color::BlendMode::Multiply;
        let l = d.layers[0].id;
        d.insert(Some(l), 0, square).unwrap();
        d.insert(Some(l), 1, text).unwrap();
        Arc::make_mut(&mut d.layers[0]).blend = vectorcraft_color::BlendMode::Multiply;
        let a = alphas(&d, &opts);
        assert!(a.iter().filter(|v| **v == 255).count() > 1000, "x {x}");
        assert_eq!(partial(&a), 0, "x {x}: None type has hard edges");
    }
}

/// Type in a placed document, placed at half and at twice its size with its frame starting part
/// of a pixel in, is drawn on the page's pixels: on the canvas each method gives every pixel the
/// share of it `edge_coverage` defines, and in a supersampled export None type has hard edges.
#[test]
fn type_in_a_placed_document_takes_the_pages_pixels() {
    use vectorcraft_doc::{ImageBlob, LinkInfo, PlacedDocument};
    // A page showing a file that holds "Hamburg" at 40 / `k` pt drawn with `m`, placed at `k` times
    // its size, so the type is 40 pt on the page.
    let placing = |m: TypeAntiAlias, k: f64| {
        let frame = Rect::new(0.0, 0.0, f64::from(W) / k, f64::from(H) / k);
        let mut t = TextObject::point(Point::new(10.0 / k, 60.0 / k), "Hamburg", CharStyle { size: 40.0 / k, ..Default::default() });
        t.anti_alias = m;
        let text = Node::new(NodeId(1), NodeKind::Text(Box::new(t)));
        let art = Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(text)], clip: false });
        let key = format!("typeedges-{m:?}-{k}");
        let blob = ImageBlob::new("application/json", key.clone().into_bytes());
        let p = PlacedDocument {
            link: LinkInfo::new("/art/type.vectorcraft"),
            key: key.clone(),
            bounding: false,
            page_box: Default::default(),
            width: f64::from(W),
            height: f64::from(H),
            xf: Affine::translate((0.3, 0.4)),
            placement: Default::default(),
        };
        // The file as read, kept for the object, so drawing it needs no loader.
        vectorcraft_doc::placed_document::prime(&p, blob.bytes.clone(), (Document::new(frame.width(), frame.height()), art, frame));
        let mut d = Document::new(f64::from(W), f64::from(H));
        d.images.insert(key, blob);
        let n = Node::new(d.alloc_id(), NodeKind::PlacedDocument(Box::new(p)));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    };
    let export = RenderOptions { supersample: true, precise: true, ..Default::default() };
    for k in [0.5, 2.0] {
        let sharp = alphas(&placing(TypeAntiAlias::Sharp, k), &RenderOptions::default());
        assert!(partial(&sharp) > 200, "k {k}: Sharp draws smooth edges: {} partial pixels", partial(&sharp));
        for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
            let drawn = alphas(&placing(m, k), &RenderOptions::default());
            assert_ne!(drawn, sharp, "k {k}: {m:?} changes the edges");
            let wrong = off_the_share(m, &sharp, &drawn);
            assert!(wrong.is_empty(), "k {k}, {m:?}: (pixel, Sharp, drawn) {wrong:?}");
        }
        let none = alphas(&placing(TypeAntiAlias::None, k), &export);
        assert!(none.iter().filter(|v| **v == 255).count() > 1000, "k {k}");
        assert_eq!(partial(&none), 0, "k {k}: None type has hard edges in the export");
    }
}

/// The opacity of each pixel of `d` drawn at `view` on a transparent canvas `w` × `h` pixels.
fn alphas_at(d: &Document, (w, h): (u32, u32), view: Affine) -> Vec<u8> {
    Renderer::new().render(d, w, h, view, &RenderOptions::default()).pixels.chunks(4).map(|p| p[3]).collect()
}

/// `page(m, None)` with the effect `id` (parameters `params`) on the type object, and the object's
/// id.
fn page_with_effect(m: TypeAntiAlias, id: &str, params: Value) -> (Document, NodeId) {
    let mut d = page(m, None);
    let n = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
    d.node_mut(n).unwrap().appearance.effects.push(Effect { id: id.into(), params, visible: true });
    (d, n)
}

/// "Ham" in 30 pt type drawn with `m`, its baseline at (5.3, 30.4).
fn ham(d: &mut Document, m: TypeAntiAlias) -> Node {
    let mut t = TextObject::point(Point::new(5.3, 30.4), "Ham", CharStyle { size: 30.0, ..Default::default() });
    t.anti_alias = m;
    Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)))
}

/// `draw` gives the pixels of type drawn with a method: Sharp type has smooth edges there, and
/// each other method changes them and gives every pixel the share of it `edge_coverage` defines.
fn each_method_takes_the_share(what: &str, draw: impl Fn(TypeAntiAlias) -> Vec<u8>) {
    let sharp = draw(TypeAntiAlias::Sharp);
    assert!(partial(&sharp) > 200, "{what}: Sharp draws smooth edges: {} partial pixels", partial(&sharp));
    for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
        let drawn = draw(m);
        assert_ne!(drawn, sharp, "{what}: {m:?} changes the edges");
        let wrong = off_the_share(m, &sharp, &drawn);
        assert!(wrong.is_empty(), "{what}, {m:?}: (pixel, Sharp, drawn) {wrong:?}");
    }
}

/// A drop shadow whose offset is not a whole number of pixels draws the type on the canvas's
/// pixels: each method gives every pixel, the shadow's below the type included, the share of it
/// `edge_coverage` defines.
#[test]
fn type_in_a_drop_shadow_takes_the_canvas_pixels() {
    let shadow = json!({"mode": "normal", "opacity": 100, "x": 3.4, "y": 45.3, "blur": 0, "color": "#000000"});
    let draw = |m| alphas_at(&page_with_effect(m, "stylize.dropShadow", shadow.clone()).0, (W, 130), Affine::IDENTITY);
    // Below the type's descender, the shadow alone.
    let below = |a: &[u8]| a[(W * 75) as usize..].iter().filter(|v| **v == 255).count();
    assert!(below(&draw(TypeAntiAlias::Sharp)) > 500, "the shadow is drawn below the type");
    each_method_takes_the_share("drop shadow", draw);
}

/// Effect › Rasterize draws the type on the pixels of its image: at 36 ppi each pixel of the image
/// is 2 × 2 pixels of a canvas whose grid it lies on, and each method gives every pixel the share
/// of it `edge_coverage` defines.
#[test]
fn type_in_a_rasterize_takes_the_images_pixels() {
    let draw = |m| {
        let (d, id) = page_with_effect(m, "rasterize", json!({"useDocumentResolution": false, "resolution": 36}));
        let image = drawn_bounds(&d, d.node(id).unwrap()).unwrap();
        // The image's corner on a corner of a canvas pixel.
        let view = Affine::translate((image.x0.ceil() - image.x0, image.y0.ceil() - image.y0));
        alphas_at(&d, (W, H), view)
    };
    each_method_takes_the_share("rasterize", draw);
}

/// A pattern tile holding type draws it on the pixels of the tile's image, also in the copies of
/// the tile 80.25 pixels away: in the swatch each method gives every pixel the share of it
/// `edge_coverage` defines.
#[test]
fn type_in_a_pattern_tile_takes_the_tiles_pixels() {
    let draw = |m| {
        let mut d = Document::new(f64::from(W), f64::from(H));
        let text = ham(&mut d, m);
        let mut def = PatternDef::new("Type", vec![Arc::new(text)]);
        // The swatch draws 2 pixels a point and shows the copies above and below the tile, 80.25
        // pixels away.
        def.tile = Rect::new(0.0, 0.0, 120.5, 40.125);
        d.patterns.push(def);
        let swatch = render_pattern_swatch(&d, "Type", 241).unwrap();
        swatch.pixels.chunks(4).map(|p| p[3]).collect::<Vec<u8>>()
    };
    // The rows of the copy above the tile.
    let above = |a: &[u8]| a[..241 * 60].iter().filter(|v| **v == 255).count();
    assert!(above(&draw(TypeAntiAlias::Sharp)) > 200, "the swatch shows the copy above the tile");
    each_method_takes_the_share("pattern tile", draw);
}

/// Pattern editing mode draws the copies of the tile, 120.25 pixels apart across and 40.125 pixels
/// apart down, on the canvas's pixels: each method gives every pixel the share of it
/// `edge_coverage` defines.
#[test]
fn type_in_the_copies_of_an_edited_tile_takes_the_canvas_pixels() {
    let draw = |m| {
        let mut d = Document::new(400.0, 200.0);
        let text = ham(&mut d, m);
        let mut def = PatternDef::new("Type", vec![Arc::new(text.clone())]);
        def.tile = Rect::new(0.0, 0.0, 120.25, 40.125);
        // The eight copies around the tile, at full opacity.
        (def.copies, def.dim_copies, def.show_tile_edge) = (3, 100.0, false);
        d.patterns.push(def);
        let layer = d.add_layer(Some("Pattern Editing Mode"));
        d.insert(Some(layer), 0, text).unwrap();
        d.pattern_edit = Some(PatternEdit { pattern: "Type".into(), layer, original: None });
        alphas_at(&d, (380, 130), Affine::translate((125.0, 45.0)))
    };
    // The rows of the copies above the tile.
    let above = |a: &[u8]| a[..380 * 40].iter().filter(|v| **v == 255).count();
    assert!(above(&draw(TypeAntiAlias::Sharp)) > 200, "the copies above the tile are drawn");
    each_method_takes_the_share("pattern editing", draw);
}

/// Type with a brush stroke of its own is drawn as Sharp draws it.
#[test]
fn type_with_a_brush_stroke_is_drawn_as_sharp() {
    let draw = |m: TypeAntiAlias| {
        let mut d = page(m, None);
        let id = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
        let n = d.node_mut(id).unwrap();
        let mut st = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 2.0);
        st.brush = Some("Charcoal".into());
        n.appearance.items.push(AppearanceItem::Stroke(st));
        alphas(&d, &RenderOptions::default())
    };
    let sharp = draw(TypeAntiAlias::Sharp);
    for m in [TypeAntiAlias::None, TypeAntiAlias::Crisp, TypeAntiAlias::Strong] {
        assert_eq!(draw(m), sharp, "{m:?}");
    }
}

/// On screen, type in a font the system doesn't have draws as Sharp draws it while Document
/// Setup highlights substituted fonts, and with its method when nothing is highlighted.
#[test]
fn type_with_a_substitution_highlight_is_drawn_as_sharp() {
    let draw = |m: TypeAntiAlias, highlight: bool| {
        let mut d = Document::new(f64::from(W), f64::from(H));
        d.setup.highlight_substituted_fonts = true;
        let style = CharStyle { size: 40.0, font_family: "No Such Family 7Q".into(), ..Default::default() };
        let mut t = TextObject::point(Point::new(10.3, 60.4), "Hamburg", style);
        t.anti_alias = m;
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        alphas(&d, &RenderOptions { highlight_substitutions: highlight, ..Default::default() })
    };
    assert_eq!(draw(TypeAntiAlias::Strong, true), draw(TypeAntiAlias::Sharp, true));
    assert_ne!(draw(TypeAntiAlias::Strong, false), draw(TypeAntiAlias::Sharp, false));
}

/// On screen, type with a ligature draws as Sharp draws it while Preferences › Type › Highlight
/// Alternate Glyphs is on, and with its method while it is off.
#[test]
fn type_with_an_alternate_glyph_highlight_is_drawn_as_sharp() {
    // Source Serif 4 (bundled with vectorcraft-text's `bundled-fonts`) ligates "fi".
    if vectorcraft_text::FontDb::global().face("Source Serif 4", "Regular").is_none() {
        eprintln!("SKIPPED: type_with_an_alternate_glyph_highlight_is_drawn_as_sharp; no Source Serif 4");
        return;
    }
    let draw = |m: TypeAntiAlias, alternates: bool| {
        let mut d = Document::new(f64::from(W), f64::from(H));
        d.setup.highlight_substituted_fonts = false;
        d.setup.highlight_substituted_glyphs = false;
        let style = CharStyle { size: 40.0, font_family: "Source Serif 4".into(), ..Default::default() };
        let mut t = TextObject::point(Point::new(10.3, 60.4), "fish", style);
        t.anti_alias = m;
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        alphas(&d, &RenderOptions { highlight_substitutions: true, highlight_alternate_glyphs: alternates, ..Default::default() })
    };
    assert_eq!(draw(TypeAntiAlias::Strong, true), draw(TypeAntiAlias::Sharp, true));
    assert_ne!(draw(TypeAntiAlias::Strong, false), draw(TypeAntiAlias::Sharp, false));
}

/// The type is drawn on the output's pixels unless, in a frame coarser than the output, they
/// would number more than the frame's there and more than the effects' pixel budget, or be more
/// than `MAX_RASTER_SIDE` a side.
#[test]
fn type_edges_are_drawn_on_at_most_the_budgets_pixels() {
    let finer = Affine::scale(0.25);
    assert_eq!(edge_grid(Rect::new(0.0, 0.0, 4000.0, 4000.0), finer), finer, "a supersampled export's pixels");
    let coarser = Affine::scale(4.0);
    assert_eq!(edge_grid(Rect::new(0.0, 0.0, 100.0, 100.0), coarser), coarser, "160,000 pixels");
    assert_eq!(edge_grid(Rect::new(0.0, 0.0, 1000.0, 1000.0), coarser), Affine::IDENTITY, "16 million pixels");
    assert_eq!(edge_grid(Rect::new(0.0, 0.0, 10_000.0, 1.0), coarser), Affine::IDENTITY, "40,000 pixels a side");
}

/// Type across a canvas of the largest side the renderer draws (`MAX_CONTEXT_SIDE` pixels) is
/// drawn with its method on every pixel.
#[test]
fn type_across_the_widest_canvas_is_drawn_with_its_method() {
    // "Hamburg" at 25,000%, about 37,000 pixels wide, past both sides of the canvas, on four rows
    // through its letters.
    let view = Affine::translate((-3_000.0, -12_500.0)) * Affine::scale(250.0);
    let draw = |m| {
        let pixels = Renderer::new().render(&page(m, None), MAX_CONTEXT_SIDE, 4, view, &RenderOptions::default()).pixels;
        pixels.chunks(4).map(|p| p[3]).collect::<Vec<u8>>()
    };
    let sharp = draw(TypeAntiAlias::Sharp);
    assert!(partial(&sharp) > 0, "the rows cross the edges of the letters");
    let none = draw(TypeAntiAlias::None);
    assert_eq!(none.len(), sharp.len());
    assert_eq!(partial(&none), 0, "None paints whole pixels");
    let wrong = off_the_share(TypeAntiAlias::None, &sharp, &none);
    assert!(wrong.is_empty(), "(pixel, Sharp, drawn) {wrong:?}");
}

/// The offscreen drawing takes in the farthest a stroke reaches: a mitered character stroke's
/// spikes up to its miter limit, transformed with the type (stretched along one axis too), and
/// the object's own strokes.
#[test]
fn the_offscreen_drawing_takes_in_the_strokes_reach() {
    let style = CharStyle {
        stroke: Paint::solid(Color::BLACK),
        stroke_width: 4.0,
        stroke_join: vectorcraft_doc::LineJoin::Miter,
        stroke_miter_limit: 10.0,
        ..Default::default()
    };
    let mut t = TextObject::point(Point::new(0.0, 0.0), "V", style);
    let n = |t: &TextObject| Node::new(NodeId(1), NodeKind::Text(Box::new(t.clone())));
    // How far the reach goes past the transformed glyph bounds on the left and on the top.
    let past = |n: &Node, t: &TextObject| {
        let g = text_geom(t);
        let (r, b) = (type_reach(n, t, &g), t.xf.transform_rect_bbox(g.bounds));
        (b.x0 - r.x0, b.y0 - r.y0)
    };
    let near = |(a, b): (f64, f64), (x, y): (f64, f64)| (a - x).abs() < 1e-6 && (b - y).abs() < 1e-6;
    assert!(near(past(&n(&t), &t), (20.0, 20.0)), "half the weight times the miter limit: {:?}", past(&n(&t), &t));
    t.xf = Affine::scale_non_uniform(10.0, 0.1);
    assert!(near(past(&n(&t), &t), (200.0, 2.0)), "stretched with the type: {:?}", past(&n(&t), &t));
    t.xf = Affine::scale(2.0);
    let mut own = n(&t);
    let mut st = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 10.0);
    st.miter_limit = 10.0;
    own.appearance.items.push(AppearanceItem::Stroke(st));
    assert!(near(past(&own, &t), (90.0, 90.0)), "the object's own stroke after the characters': {:?}", past(&own, &t));
}

/// Crisp type with a character stroke expands the stroke once for both of its offscreen drawings,
/// and keeps the outline as Sharp type does. Its pixels are those of a renderer that keeps no
/// glyph geometry, which expands the stroke in every drawing.
#[test]
fn crisp_type_keeps_its_character_stroke_outline() {
    let _fonts = crate::tests_fontchange::fonts_steady();
    let mut d = page(TypeAntiAlias::Crisp, None);
    let id = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
    if let Some(Node { kind: NodeKind::Text(t), .. }) = d.node_mut(id) {
        t.runs[0].style.stroke = Paint::solid(Color::BLACK);
        t.runs[0].style.stroke_width = 1.0;
    }
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let pixels = r.render(&d, W, H, Affine::IDENTITY, &opts).pixels;
    assert_eq!(r.stats.type_edges, 1, "the type is drawn offscreen");
    assert_eq!(r.char_strokes.len(), 1, "one outline for the one run's stroke, used by both drawings");
    let mut uncached = Renderer::new();
    uncached.texts = FrameCache::new(0, 0);
    assert!(uncached.render(&d, W, H, Affine::IDENTITY, &opts).pixels == pixels, "the kept outline draws the same pixels");
    assert_eq!(uncached.char_strokes.len(), 0);
}

/// Area type draws its punctuation outside its frame with Optical Margin Alignment, and None type
/// draws it there too.
#[test]
fn type_outside_its_frame_is_drawn() {
    let page = |m: TypeAntiAlias| {
        let mut d = Document::new(f64::from(W), f64::from(H));
        let mut t = TextObject::point(Point::new(0.0, 0.0), "“Quoted.”", CharStyle { size: 30.0, ..Default::default() });
        t.kind = TextKind::Area { frame: rectangle(Rect::new(40.0, 10.0, 240.0, 80.0)) };
        t.optical_margin = true;
        t.anti_alias = m;
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        alphas(&d, &RenderOptions::default())
    };
    let outside = |a: &[u8]| a.iter().enumerate().filter(|(i, v)| (*i as u32 % W) < 39 && **v > 0).count();
    assert!(outside(&page(TypeAntiAlias::Sharp)) > 20, "the opening quotation mark hangs outside the frame");
    assert!(outside(&page(TypeAntiAlias::None)) > 20, "None type draws it there too");
}

/// A frame draws Crisp type from the pixels an earlier frame kept when the view only moved by
/// whole pixels, and they are the pixels a new renderer draws. Another zoom, another method, a pan
/// that brings more of the type into view, a pan by part of a pixel and other edge smoothing draw
/// the type again.
#[test]
fn a_pan_draws_the_type_from_the_kept_pixels() {
    let _fonts = crate::tests_fontchange::fonts_steady();
    let opts = RenderOptions::default();
    let fonts = || vectorcraft_text::FontDb::global().generation();
    // How many type objects a renderer drew offscreen for `b` in `second` after drawing `a` in
    // `first`. The second frame's pixels are a new renderer's, each channel within `near`. The
    // frames are drawn again while the fonts change between them (other tests load fonts, and the
    // type is then drawn again).
    let drawn = |(a, first): (&Document, Affine), (b, second): (&Document, Affine), near: u8| {
        for _ in 0..5 {
            let before = fonts();
            let mut r = Renderer::new();
            r.render(a, W, H, first, &opts);
            assert_eq!(r.stats.type_edges, 1, "{first:?}");
            let pixels = r.render(b, W, H, second, &opts).pixels;
            if fonts() != before {
                continue;
            }
            let want = Renderer::new().render(b, W, H, second, &opts).pixels;
            let differ: Vec<_> = pixels
                .chunks(4)
                .zip(want.chunks(4))
                .enumerate()
                .filter(|(_, (p, q))| p.iter().zip(*q).any(|(p, q)| p.abs_diff(*q) > near))
                .map(|(i, (p, q))| (i as u32 % W, i as u32 / W, p[3], q[3]))
                .collect();
            assert!(
                differ.is_empty(),
                "{second:?}: {} pixels differ from a new renderer's, first (x, y, alpha, new alpha) {:?}",
                differ.len(),
                &differ[..differ.len().min(8)]
            );
            return r.stats.type_edges;
        }
        panic!("the fonts changed in every try");
    };
    let (crisp, strong) = (page(TypeAntiAlias::Crisp, None), page(TypeAntiAlias::Strong, None));
    let at = |d, (x, y): (f64, f64)| (d, Affine::translate((x, y)));
    for pan in [(7.0, -3.0), (2.0, 5.0), (-4.0, -1.0)] {
        assert_eq!(drawn(at(&crisp, (0.0, 0.0)), at(&crisp, pan), 0), 0, "a pan by {pan:?}");
    }
    assert_eq!(drawn(at(&crisp, (0.0, 0.0)), (&crisp, Affine::scale(1.5)), 0), 1, "another zoom");
    assert_eq!(drawn(at(&crisp, (0.0, 0.0)), at(&strong, (0.0, 0.0)), 0), 1, "another method");
    assert_eq!(drawn(at(&crisp, (0.0, 0.0)), at(&crisp, (0.5, 0.0)), 0), 1, "a pan by half a pixel");
    // At 400% the type fills the view, so the pixels kept hold every pixel of the next frame.
    let zoomed = Affine::translate((-200.0, -150.0)) * Affine::scale(4.0);
    let nudged = Affine::translate((0.3, 0.0)) * zoomed;
    assert_eq!(drawn((&crisp, zoomed), (&crisp, nudged), 0), 1, "a pan by part of a pixel at 400%");
    assert_eq!(drawn(at(&strong, (-120.0, 0.0)), at(&strong, (0.0, 0.0)), 0), 1, "a pan that brings the rest into view");
    // Type the view cuts is drawn offscreen from the view's edge, and its coverage there can round
    // a step the other way: Strong doubles the step, and the color takes a step of rounding.
    assert_eq!(drawn(at(&strong, (0.0, 0.0)), at(&strong, (-120.0, 0.0)), 3), 0, "a pan that takes half of it out of view");
    // With hard edges (Anti-aliased Artwork off) and with type snapped to pixels (an export's Type
    // Optimized), the frame draws the type again.
    for smoothing in [AntiAlias::None, AntiAlias::Type] {
        let other = RenderOptions { anti_alias: smoothing, ..Default::default() };
        let mut r = Renderer::new();
        r.render(&crisp, W, H, Affine::IDENTITY, &opts);
        let pixels = r.render(&crisp, W, H, Affine::IDENTITY, &other).pixels;
        assert_eq!(r.stats.type_edges, 1, "{smoothing:?}");
        assert!(pixels == Renderer::new().render(&crisp, W, H, Affine::IDENTITY, &other).pixels, "{smoothing:?}");
    }
}
