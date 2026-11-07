use super::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::pattern::{PatternEdit, RepeatSpec, pattern_paint};
use vectorcraft_doc::{Appearance, NodeId, NodeKind};
use vectorcraft_geom::shapes;

/// A 20×20 tile with a red 10×10 square in its top-left corner.
fn doc_with_pattern(tt: TileType) -> Document {
    let mut d = Document::new(200.0, 200.0);
    let red = Node::path(
        NodeId(900),
        shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    let mut def = PatternDef::new("Checks", vec![Arc::new(red)]);
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    def.tile_type = tt;
    d.patterns.push(def);
    let id = d.alloc_id();
    let n = Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn render(d: &Document, scale: f64) -> Rendered {
    let px = (200.0 * scale) as u32;
    Renderer::new().render(d, px, px, Affine::scale(scale), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}
fn is_white(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] > 200 && p[2] > 200
}

/// #1536: a raster effect drawn after a pattern fill is the same as without the pattern (the
/// pattern's paint transform must not resample the effect's raster).
#[test]
fn effects_after_a_pattern_fill_keep_their_place_and_size() {
    let fx = |id: &str, params: serde_json::Value| vectorcraft_doc::Effect { id: id.into(), params, visible: true };
    for effect in [
        fx("blur.gaussian", serde_json::json!({"radius": 4.0})),
        fx("stylize.dropShadow", serde_json::json!({"x": 6, "y": 6, "blur": 3, "opacity": 100})),
    ] {
        let make = |with_pattern: bool| {
            let mut d = doc_with_pattern(TileType::Grid);
            let l = d.layers[0].id;
            let filled = d.children(Some(l)).unwrap()[0].id;
            d.remove(filled).unwrap();
            if with_pattern {
                let id = d.alloc_id();
                let n =
                    Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 80.0, 80.0)), Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
                d.insert(Some(l), 0, n).unwrap();
            }
            let id = d.alloc_id();
            let mut n = Node::path(
                id,
                shapes::ellipse(Rect::new(120.0, 110.0, 180.0, 170.0)),
                Appearance::basic(Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0),
            );
            n.appearance.effects.push(effect.clone());
            let at = d.children(Some(l)).map_or(0, Vec::len);
            d.insert(Some(l), at, n).unwrap();
            d
        };
        // Supersampled, as raster exports draw.
        let one = |d: &Document| {
            let mut r = Renderer::new();
            r.render(
                d,
                200,
                200,
                Affine::IDENTITY,
                &RenderOptions { background: Some([255, 255, 255, 255]), supersample: true, ..Default::default() },
            )
        };
        let (with, without) = (one(&make(true)), one(&make(false)));
        for (x, y) in [(150, 140), (125, 140), (176, 160), (150, 172), (184, 176)] {
            assert_eq!(with.pixel(x, y), without.pixel(x, y), "{} at ({x}, {y})", effect.id);
        }
    }
}

#[test]
fn pattern_fill_is_periodic() {
    let d = doc_with_pattern(TileType::Grid);
    let r = render(&d, 1.0);
    for (x, y) in [(5, 5), (25, 5), (125, 145), (185, 65)] {
        assert!(is_red(r.pixel(x, y)), "({x},{y}) {:?}", r.pixel(x, y));
    }
    for (x, y) in [(15, 5), (5, 15), (135, 135), (175, 195)] {
        assert!(is_white(r.pixel(x, y)), "({x},{y}) {:?}", r.pixel(x, y));
    }
    // Periodic: pixel (x, y) equals (x + 20k, y + 20m).
    for y in (1..20).step_by(3) {
        for x in (1..20).step_by(3) {
            if x == 10 || y == 10 {
                continue;
            }
            assert_eq!(r.pixel(x, y), r.pixel(x + 60, y + 100));
        }
    }
}

#[test]
fn pattern_fill_is_clipped_to_shape() {
    let mut d = doc_with_pattern(TileType::Grid);
    let id = d.layers[0].children().unwrap()[0].id;
    *d.node_mut(id).unwrap().path_data_mut().unwrap() = shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0));
    let r = render(&d, 1.0);
    assert!(is_red(r.pixel(45, 45)));
    assert!(is_white(r.pixel(65, 65)));
}

#[test]
fn brick_pattern_shifts_rows() {
    let d = doc_with_pattern(TileType::BrickByRow { offset: 0.5 });
    let r = render(&d, 1.0);
    assert!(is_red(r.pixel(5, 5)));
    // Row 1 is shifted by half a tile.
    assert!(is_white(r.pixel(5, 25)) && is_red(r.pixel(15, 25)), "{:?} {:?}", r.pixel(5, 25), r.pixel(15, 25));
    assert!(is_red(r.pixel(5, 45)));
}

#[test]
fn pattern_follows_zoom_and_paint_transform() {
    let d = doc_with_pattern(TileType::Grid);
    let r = render(&d, 2.0);
    assert!(is_red(r.pixel(10, 10)) && is_white(r.pixel(30, 10)) && is_red(r.pixel(50, 10)));
    let mut d2 = d.clone();
    let id = d2.layers[0].children().unwrap()[0].id;
    vectorcraft_doc::pattern::transform_pattern_paints(d2.node_mut(id).unwrap(), Affine::translate((10.0, 0.0)));
    let r = render(&d2, 1.0);
    assert!(is_white(r.pixel(5, 5)) && is_red(r.pixel(15, 5)));
}

/// `doc_with_pattern(TileType::Grid)` with a 40×40 square on top of its rectangle, filled with
/// the same pattern scaled by `s`, as Transform Pattern Tiles leaves it.
fn doc_with_two_scales(s: f64) -> Document {
    let mut d = doc_with_pattern(TileType::Grid);
    let square = shapes::rectangle(Rect::new(120.0, 120.0, 160.0, 160.0));
    let mut n = Node::path(d.alloc_id(), square, Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
    vectorcraft_doc::pattern::transform_pattern_paints(&mut n, Affine::scale(s));
    let l = d.layers[0].id;
    d.insert(Some(l), 1, n).unwrap();
    d
}

/// `d` drawn by `r` at `scale`.
fn frame(r: &mut Renderer, d: &Document, scale: f64) -> Rendered {
    let px = (200.0 * scale) as u32;
    r.render(d, px, px, Affine::scale(scale), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

/// The tiles rasterized on this thread so far: the tile renderer counts a frame for each.
fn tiles_rasterized() -> u64 {
    CACHE.with(|c| c.borrow().renderer.as_ref().map_or(0, |r| r.stamp))
}

#[test]
fn objects_at_two_pattern_scales_keep_their_tiles_between_frames() {
    // At 100% the rectangle's tile is rasterized at scale 1, and the square's tile is
    // rasterized at 0.25, or at 2 when its tiles are scaled to 101%.
    for s in [0.25, 1.01] {
        let d = doc_with_two_scales(s);
        let mut r = Renderer::new();
        let start = tiles_rasterized();
        let first = frame(&mut r, &d, 1.0).pixels;
        assert_eq!(tiles_rasterized() - start, 2, "tiles scaled by {s}: a tile for each object");
        for _ in 0..3 {
            assert!(frame(&mut r, &d, 1.0).pixels == first, "tiles scaled by {s}");
        }
        assert_eq!(tiles_rasterized() - start, 2, "tiles scaled by {s}: later frames draw both objects from their tiles");
        // After a frame at 200%, the next frame at 100% draws from the tiles made at 100%.
        frame(&mut r, &d, 2.0);
        let zoomed = tiles_rasterized();
        assert!(frame(&mut r, &d, 1.0).pixels == first, "tiles scaled by {s}");
        assert_eq!(tiles_rasterized(), zoomed, "tiles scaled by {s}: back at 100%");
    }
}

#[test]
fn kept_tiles_draw_what_a_new_renderer_draws() {
    // A new renderer on a thread of its own starts with no tiles.
    let fresh = |d: &Document, zoom: f64| {
        let d = d.clone();
        std::thread::spawn(move || frame(&mut Renderer::new(), &d, zoom).pixels).join().unwrap()
    };
    let mut d = doc_with_two_scales(0.25);
    let mut r = Renderer::new();
    // Pixel (130, 130) is red in the square's tiles and clear in the rectangle's, (133, 133) is
    // clear in both, and (103, 103), outside the square, is red in the rectangle's.
    let first = frame(&mut r, &d, 1.0);
    assert!(is_red(first.pixel(130, 130)) && is_white(first.pixel(133, 133)) && is_red(first.pixel(103, 103)));
    for zoom in [1.0, 2.0, 1.0, 0.5, 2.0, 1.0] {
        assert!(frame(&mut r, &d, zoom).pixels == fresh(&d, zoom), "zoom {zoom}");
    }
    // Change the tile type as Pattern Options does.
    d.patterns[0].tile_type = TileType::BrickByRow { offset: 0.5 };
    for zoom in [2.0, 1.0] {
        assert!(frame(&mut r, &d, zoom).pixels == fresh(&d, zoom), "brick, zoom {zoom}");
    }
    // Replace the art as Done Editing Pattern does.
    let blue = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
    d.patterns[0].art = vec![Arc::new(Node::path(NodeId(901), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), blue))];
    for zoom in [1.0, 2.0] {
        assert!(frame(&mut r, &d, zoom).pixels == fresh(&d, zoom), "new art, zoom {zoom}");
    }
}

#[test]
fn kept_tiles_stay_within_the_budget() {
    // The rectangle's tile is 20 pixels a side per 100% of zoom, and the square's tile is 5
    // pixels a side per 100% of zoom. The budget holds the rectangle's tile at 400% and both
    // tiles at 200%; the tiles drawn from 100% to 400% need more.
    let d = doc_with_two_scales(0.25);
    let tile = |side: usize| size_of::<TileEntry>() + side * side * 4;
    let budget = tile(80) + tile(40) + tile(10);
    CACHE.with(|c| c.borrow_mut().tiles[Ink::Display.index()] = FrameCache::new(budget, usize::MAX));
    let kept = || {
        CACHE.with(|c| {
            let c = c.borrow();
            let tiles = &c.tiles[Ink::Display.index()];
            (tiles.len(), tiles.bytes())
        })
    };
    let mut r = Renderer::new();
    for zoom in [1.0, 2.0, 4.0] {
        frame(&mut r, &d, zoom);
        let drawn = tiles_rasterized();
        frame(&mut r, &d, zoom);
        assert_eq!(tiles_rasterized(), drawn, "zoom {zoom}: the next frame draws from the tiles just made");
        let (len, bytes) = kept();
        assert!(len >= 2 && bytes <= budget, "zoom {zoom}: {len} tiles, {bytes} bytes");
    }
    // Tiles were rasterized at five scales (0.25, 0.5, 1, 2 and 4); the cache dropped the least
    // recently used tiles first.
    assert!(kept().0 < 5, "{:?}", kept());
    let before = tiles_rasterized();
    frame(&mut r, &d, 1.0);
    assert!(tiles_rasterized() > before, "a tile of 100% was dropped");
    assert!(kept().1 <= budget, "{:?}", kept());
}

/// A pattern whose art is an image that can't be decoded: every frame that draws the pattern
/// counts the failure ([`crate::paint::decode_failures`]), from a new tile or from the
/// tile kept.
#[test]
fn kept_tiles_count_the_images_they_could_not_decode() {
    use vectorcraft_doc::{ImageBlob, ImageObject};
    let mut d = doc_with_pattern(TileType::Grid);
    // A PNG signature with nothing after it.
    d.images.insert("cut".into(), ImageBlob::new("image/png", b"\x89PNG\r\n\x1a\n".to_vec()));
    let im = ImageObject { key: "cut".into(), width: 10, height: 10, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    d.patterns[0].art = vec![Arc::new(Node::new(NodeId(901), NodeKind::Image(Box::new(im))))];
    let mut r = Renderer::new();
    let start = tiles_rasterized();
    for n in 0..3 {
        let before = crate::paint::decode_failures();
        frame(&mut r, &d, 1.0);
        assert!(crate::paint::decode_failures() > before, "frame {n}");
    }
    assert_eq!(tiles_rasterized() - start, 1, "the later frames draw from the tile kept");
}

#[test]
fn missing_pattern_is_grey() {
    let mut d = doc_with_pattern(TileType::Grid);
    d.patterns.clear();
    let p = render(&d, 1.0).pixel(50, 50);
    assert_eq!(p, [128, 128, 128, 255]);
}

#[test]
fn swatch_thumbnail_renders() {
    let d = doc_with_pattern(TileType::Grid);
    let r = render_pattern_swatch(&d, "Checks", 20).unwrap();
    assert!(is_red(r.pixel(3, 3)));
    assert_eq!(r.pixel(15, 15)[3], 0);
    assert!(render_pattern_swatch(&d, "Nope", 20).is_none());
}

#[test]
fn edit_mode_draws_only_tile_layer_with_copies() {
    let mut d = doc_with_pattern(TileType::Grid);
    let l = d.add_layer(Some("Pattern Editing Mode"));
    let art = d.patterns[0].art[0].clone();
    d.insert(Some(l), 0, (*art).clone()).unwrap();
    d.pattern_edit = Some(PatternEdit { pattern: "Checks".into(), layer: l, original: None });
    d.patterns[0].dim_copies = 50.0;
    d.patterns[0].show_tile_edge = false;
    let r = render(&d, 1.0);
    // The tile itself at full strength, copies dimmed, the filled document rect hidden.
    assert!(is_red(r.pixel(5, 5)));
    let c = r.pixel(25, 5);
    assert!(c[0] > 200 && c[1] > 100 && c[1] < 160, "{c:?}");
    assert!(is_white(r.pixel(15, 5)));
    assert!(is_white(r.pixel(125, 125)), "outside the copies");
}

#[test]
fn repeat_renders_instances() {
    let mut d = Document::new(200.0, 200.0);
    let sq =
        Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, Node::new(id, NodeKind::Repeat(RepeatSpec::grid(vec![Arc::new(sq)], 10.0, 10.0)))).unwrap();
    let r = render(&d, 1.0);
    for (x, y) in [(5, 5), (25, 5), (45, 45)] {
        assert_eq!(r.pixel(x, y), [0, 0, 0, 255], "({x},{y})");
    }
    assert!(is_white(r.pixel(15, 5)) && is_white(r.pixel(65, 5)));
}

#[test]
fn raster_scale_is_power_of_two_and_capped() {
    assert_eq!(raster_scale(1.0, (10.0, 10.0)), 1.0);
    assert_eq!(raster_scale(1.5, (10.0, 10.0)), 2.0);
    assert_eq!(raster_scale(3.0, (10.0, 10.0)), 4.0);
    // A change within 2× keeps the raster.
    assert_eq!(raster_scale(2.1, (10.0, 10.0)), raster_scale(3.9, (10.0, 10.0)));
    assert!(raster_scale(64.0, (1000.0, 10.0)) * 1000.0 <= MAX_TILE_PX);
}

#[test]
fn tile_renderer_depth_survives_caught_panics() {
    for _ in 0..4 {
        let r = std::panic::catch_unwind(|| with_tile_renderer(|_| panic!("tile boom")));
        assert!(r.is_err());
    }
    assert!(with_tile_renderer(|_| ()).is_some(), "depth leaked after caught panics");
}
