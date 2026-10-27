//! Effect › Rasterize on the canvas: the art drawn as square pixels of the effect's resolution
//! over its bounds and the room around them, on white or on nothing, its paths left as they are.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Effect, Node};
use vectorcraft_geom::{Anchor, PathData, Point, SubPath, shapes};

use crate::*;

/// A red shape (`path`, over 10..30 pt) with the effects `fx` (id, params), in a document whose
/// raster effects resolution is `ppi`.
fn doc(path: PathData, fx: &[(&str, Value)], ppi: f64) -> Document {
    let mut d = Document::new(100.0, 100.0);
    d.raster_effects_ppi = ppi;
    let l = d.layers[0].id;
    let mut n = Node::path(d.alloc_id(), path, Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0));
    n.appearance.effects = fx.iter().map(|(id, params)| Effect { id: (*id).into(), params: params.clone(), visible: true }).collect();
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// A right triangle over 10..30 pt whose long side runs diagonally.
fn triangle() -> PathData {
    let corner = |x, y| Anchor::corner(Point::new(x, y));
    PathData::single(SubPath::new(vec![corner(10.0, 10.0), corner(30.0, 10.0), corner(10.0, 30.0)], true))
}

/// `d` drawn at `zoom` device pixels a point on a transparent canvas `size` pixels square.
fn draw(d: &Document, zoom: f64, size: u32) -> Rendered {
    Renderer::new().render(d, size, size, Affine::scale(zoom), &RenderOptions { background: None, artboards: false, ..Default::default() })
}

fn pixel(r: &Rendered, x: usize, y: usize) -> [u8; 4] {
    let i = (y * r.width as usize + x) * 4;
    [r.pixels[i], r.pixels[i + 1], r.pixels[i + 2], r.pixels[i + 3]]
}

/// Whether each of the `n` × `n` squares `side` device pixels wide from (`at`, `at`) is one color.
fn squares_are_flat(r: &Rendered, at: usize, side: usize, n: usize) -> bool {
    (0..n * n).all(|k| {
        let (x0, y0) = (at + (k % n) * side, at + (k / n) * side);
        let first = pixel(r, x0, y0);
        (0..side * side).all(|i| pixel(r, x0 + i % side, y0 + i / side) == first)
    })
}

#[test]
fn rasterize_draws_the_art_in_pixels_of_its_resolution() {
    // (params, the document's resolution, device pixels a point, device pixels an image pixel)
    let cases = [
        (json!({"useDocumentResolution": false, "resolution": 72}), 300.0, 10.0, 10),
        (json!({"useDocumentResolution": false, "resolution": 72}), 300.0, 2.0, 2),
        (json!({"useDocumentResolution": false, "resolution": 300}), 72.0, 25.0, 6),
        (json!({}), 150.0, 25.0, 12),
    ];
    for (params, ppi, zoom, side) in cases {
        let at = (10.0 * zoom) as usize;
        // The whole image pixels inside the 20 pt triangle's bounds.
        let n = (20.0 * zoom) as usize / side;
        let canvas = (40.0 * zoom) as u32;
        let rasterized = draw(&doc(triangle(), &[("rasterize", params.clone())], ppi), zoom, canvas);
        assert!(squares_are_flat(&rasterized, at, side, n), "{params}");
        // Along the diagonal an image pixel takes the mean of the art it covers.
        let partly = (0..n).map(|k| pixel(&rasterized, at + k * side, at + (n - 1 - k) * side)[3]).filter(|a| *a > 0 && *a < 255).count();
        assert!(partly > n / 2, "{params}: {partly} of {n}");
        // Without the effect the diagonal edge runs through those squares.
        assert!(!squares_are_flat(&draw(&doc(triangle(), &[], ppi), zoom, canvas), at, side, n));
    }
}

#[test]
fn rasterize_is_drawn_where_only_its_image_reaches_the_view() {
    // At 1 ppi a pixel of the image is 72 pt wide: the 20 pt square's image runs to 82 pt.
    let d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &[("rasterize", json!({"background": "white"}))], 1.0);
    // A view of 40..80 pt across, where only the image's white reaches.
    let opts = RenderOptions { background: None, artboards: false, ..Default::default() };
    let r = Renderer::new().render(&d, 40, 40, Affine::translate((-40.0, 0.0)), &opts);
    assert_eq!(pixel(&r, 20, 20)[3], 255);
    // A blur of 4 pt (6 pt of spread) and 10 pt of white around it: the white reaches 46 pt.
    let fx = [("blur.gaussian", json!({"radius": 4})), ("rasterize", json!({"background": "white", "addAround": 10}))];
    let d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &fx, 72.0);
    let r = Renderer::new().render(&d, 40, 40, Affine::translate((-45.0, 0.0)), &opts);
    assert_eq!(pixel(&r, 0, 20), [255; 4]);
    // A fill's own Rasterize with 100 pt of white around: the white reaches 130 pt.
    let mut d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &[], 72.0);
    let l = d.layers[0].id;
    let n = Arc::make_mut(&mut d.children_mut(Some(l)).unwrap()[0]);
    n.appearance.items[0].effects_mut().push(Effect {
        id: "rasterize".into(),
        params: json!({"background": "white", "addAround": 100}),
        visible: true,
    });
    let r = Renderer::new().render(&d, 40, 40, Affine::translate((-115.0, 0.0)), &opts);
    assert_eq!(pixel(&r, 5, 20), [255; 4]);
}

#[test]
fn art_that_paints_nothing_takes_no_room_in_the_image() {
    // A second fill, hidden, under a Gaussian Blur that would spread 15 pt.
    let mut d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &[("rasterize", json!({"background": "white"}))], 72.0);
    let l = d.layers[0].id;
    let n = Arc::make_mut(&mut d.children_mut(Some(l)).unwrap()[0]);
    let mut hidden = n.appearance.items[0].clone();
    if let vectorcraft_doc::AppearanceItem::Fill(f) = &mut hidden {
        f.visible = false;
        f.effects.push(Effect { id: "blur.gaussian".into(), params: json!({"radius": 10}), visible: true });
    }
    n.appearance.items.push(hidden.clone());
    // A third, hidden too, offset 5 pt outward.
    if let vectorcraft_doc::AppearanceItem::Fill(f) = &mut hidden {
        f.effects = vec![Effect { id: "path.offsetPath".into(), params: json!({"offset": 5}), visible: true }];
    }
    n.appearance.items.push(hidden);
    let r = draw(&d, 10.0, 400);
    assert_eq!((pixel(&r, 105, 105), pixel(&r, 95, 200)[3]), ([255, 0, 0, 255], 0));
    // Nor does a hidden member of a group.
    let mut d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &[], 72.0);
    let mut away = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(34.0, 10.0, 38.0, 30.0)), Appearance::default());
    away.visible = false;
    let l = d.layers[0].id;
    let mut members = d.children_mut(Some(l)).map(std::mem::take).unwrap();
    members.push(Arc::new(away));
    let mut group = Node::group(d.alloc_id(), members);
    group.appearance.effects.push(Effect { id: "rasterize".into(), params: json!({"background": "white"}), visible: true });
    d.insert(Some(l), 0, group).unwrap();
    assert_eq!(pixel(&draw(&d, 10.0, 400), 350, 200)[3], 0);
    // Nor does a Shaper group's source art, its first member, which paints nothing.
    let mut d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &[], 72.0);
    let none = Appearance::basic(Paint::None, Paint::None, 0.0);
    let source = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(34.0, 10.0, 38.0, 30.0)), none);
    let l = d.layers[0].id;
    let mut members = d.children_mut(Some(l)).map(std::mem::take).unwrap();
    members.insert(0, Arc::new(source));
    let mut group = Node::group(d.alloc_id(), members);
    group.extras_mut().shaper = Some(Box::new(vectorcraft_doc::shaper::ShaperSpec { faces: vec![], edges: vec![] }));
    group.appearance.effects.push(Effect { id: "rasterize".into(), params: json!({"background": "white"}), visible: true });
    d.insert(Some(l), 0, group).unwrap();
    assert_eq!(pixel(&draw(&d, 10.0, 400), 350, 200)[3], 0);
    // Nor does the path where its only fill's own Transform effect moves the fill away from it.
    let mut d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &[("rasterize", json!({"background": "white"}))], 72.0);
    let l = d.layers[0].id;
    let n = Arc::make_mut(&mut d.children_mut(Some(l)).unwrap()[0]);
    if let vectorcraft_doc::AppearanceItem::Fill(f) = &mut n.appearance.items[0] {
        f.effects.push(Effect { id: "distort.transform".into(), params: json!({"moveH": 40}), visible: true });
    }
    let r = draw(&d, 10.0, 800);
    assert_eq!((pixel(&r, 600, 200), pixel(&r, 200, 200)[3]), ([255, 0, 0, 255], 0));
}

#[test]
fn rasterize_covers_the_share_of_a_screen_pixel_its_image_covers() {
    // A 1 pt square from x = 10.25 at 72 ppi and 100%: its image pixel covers a quarter of
    // screen column 11, on white and on nothing.
    let square = || shapes::rectangle(Rect::new(10.25, 10.0, 11.25, 11.0));
    for (params, want) in [(json!({}), [64, 0, 0, 64]), (json!({"background": "white"}), [64, 0, 0, 64])] {
        let r = draw(&doc(square(), &[("rasterize", params.clone())], 72.0), 1.0, 20);
        assert_eq!(pixel(&r, 11, 10), want, "{params}");
        assert_eq!(pixel(&r, 12, 10)[3], 0, "{params}");
    }
}

#[test]
fn objects_wider_than_a_raster_can_be_are_drawn_with_their_pixel_effects() {
    // 65,530 pt across at 100%: wider than the rasters the effects are made in can safely be.
    let bar = || shapes::rectangle(Rect::new(0.0, 10.0, 65_530.0, 11.0));
    let opts = RenderOptions { background: None, artboards: false, ..Default::default() };
    for fx in [("rasterize", json!({"background": "white"})), ("blur.gaussian", json!({"radius": 1}))] {
        let r = Renderer::new().render(&doc(bar(), std::slice::from_ref(&fx), 72.0), 40, 20, Affine::translate((-65_500.0, 0.0)), &opts);
        assert!(pixel(&r, 10, 10)[3] > 0, "{}", fx.0);
    }
}

#[test]
fn a_stack_of_ten_thousand_rasterize_effects_is_drawn() {
    let fx = vec![("rasterize", json!({"background": "white"})); 10_000];
    let r = draw(&doc(shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), &fx, 72.0), 1.0, 40);
    assert_eq!(pixel(&r, 20, 20), [255, 0, 0, 255]);
}

#[test]
fn a_stack_of_rasterize_effects_with_shadows_draws_the_content_of_each_once() {
    let shadow = ("stylize.dropShadow", json!({"mode": "normal", "opacity": 100, "x": 2, "y": 2, "blur": 1, "color": "#0000ff"}));
    let glow = ("stylize.outerGlow", json!({"mode": "normal", "opacity": 100, "blur": 2, "color": "#00ff00"}));
    // Five Rasterize effects, each followed by a drop shadow and an outer glow: on a 20 pt square
    // at 72 ppi, and on a 300 pt square at 2,400 ppi, whose images are too large for the canvas to
    // draw on their own grid.
    for (ppi, side, zoom) in [(72.0, 20.0, 1.0), (2400.0, 300.0, 0.1)] {
        let rasterize = ("rasterize", json!({"useDocumentResolution": false, "resolution": ppi}));
        let fx: Vec<_> = (0..5).flat_map(|_| [rasterize.clone(), shadow.clone(), glow.clone()]).collect();
        let d = doc(shapes::rectangle(Rect::new(10.0, 10.0, 10.0 + side, 10.0 + side)), &fx, 72.0);
        let drawn = || crate::fx::RASTERIZED.with(std::cell::Cell::get);
        let before = drawn();
        let r = draw(&d, zoom, 40);
        assert_eq!(drawn() - before, 5, "{ppi} ppi");
        assert_eq!(pixel(&r, 15, 15), [255, 0, 0, 255], "{ppi} ppi");
    }
}

#[test]
fn a_new_document_resolution_redraws_a_rasterized_member_of_a_blurred_group() {
    let mut d = doc(triangle(), &[("rasterize", json!({}))], 72.0);
    let l = d.layers[0].id;
    let members = d.children_mut(Some(l)).map(std::mem::take).unwrap();
    let mut group = Node::group(d.alloc_id(), members);
    group.appearance.effects.push(Effect { id: "blur.gaussian".into(), params: json!({"radius": 0.5}), visible: true });
    d.insert(Some(l), 0, group).unwrap();
    let opts = RenderOptions { background: None, artboards: false, ..Default::default() };
    let mut r = Renderer::new();
    r.render(&d, 400, 400, Affine::scale(10.0), &opts);
    d.raster_effects_ppi = 18.0;
    let again = r.render(&d, 400, 400, Affine::scale(10.0), &opts);
    assert_eq!(again.pixels, Renderer::new().render(&d, 400, 400, Affine::scale(10.0), &opts).pixels);
}

#[test]
fn an_export_draws_the_image_with_the_effects_own_anti_aliasing() {
    // A supersampled Art Optimized export draws its art with hard edges and then averages them;
    // the effect's image keeps its smooth diagonal.
    let d = doc(triangle(), &[("rasterize", json!({"useDocumentResolution": false, "resolution": 72}))], 72.0);
    let opts = RenderOptions { background: None, artboards: false, anti_alias: AntiAlias::Art, supersample: true, ..Default::default() };
    let r = Renderer::new().render_region_with(&d, Rect::new(0.0, 0.0, 40.0, 40.0), 10.0, &opts);
    let partly = (0..20).map(|k| pixel(&r, 105 + k * 10, 105 + (19 - k) * 10)[3]).filter(|a| *a > 0 && *a < 255).count();
    assert!(partly > 10, "{partly} of 20");
}

#[test]
fn rasterize_without_anti_aliasing_has_hard_edges_at_any_zoom() {
    let fx = [("rasterize", json!({"useDocumentResolution": false, "resolution": 72, "antiAlias": "none"}))];
    for zoom in [10.0, 1.0] {
        let r = draw(&doc(triangle(), &fx, 72.0), zoom, (40.0 * zoom) as u32);
        let alphas: Vec<u8> = r.pixels.chunks(4).map(|p| p[3]).collect();
        assert!(alphas.iter().all(|a| *a == 0 || *a == 255), "zoom {zoom}");
        assert!(alphas.contains(&255));
    }
    // A bar 0.6 pt wide covers the center of the image pixel it starts, which it then fills, and
    // which at 50% covers half a screen pixel: it shows there, partly covering it.
    let bar = || shapes::rectangle(Rect::new(10.3, 10.0, 10.9, 30.0));
    assert_eq!(pixel(&draw(&doc(bar(), &fx, 72.0), 10.0, 400), 105, 200)[3], 255);
    let half = pixel(&draw(&doc(bar(), &fx, 72.0), 0.5, 20), 5, 10)[3];
    assert!(half > 0 && half < 255, "{half}");
    // At 100% that image pixel, 10.3..11.3 pt, covers 70% of screen column 10 and 30% of column 11.
    let full = draw(&doc(bar(), &fx, 72.0), 1.0, 40);
    assert_eq!((pixel(&full, 10, 20)[3], pixel(&full, 11, 20)[3]), (178, 77));
}

#[test]
fn rasterize_keeps_what_an_earlier_effect_spreads_beyond_the_bounds() {
    let fx = [("blur.gaussian", json!({"radius": 4})), ("rasterize", json!({"useDocumentResolution": false, "resolution": 72}))];
    let r = draw(&doc(triangle(), &fx, 72.0), 10.0, 400);
    // 1.5 pt left of the triangle's bounds, the blur still shows.
    assert!(pixel(&r, 85, 150)[3] > 0);
    // So does a blur of the fill's own.
    let mut d = doc(triangle(), &fx[1..], 72.0);
    let l = d.layers[0].id;
    let n = Arc::make_mut(&mut d.children_mut(Some(l)).unwrap()[0]);
    n.appearance.items[0].effects_mut().push(Effect { id: "blur.gaussian".into(), params: json!({"radius": 4}), visible: true });
    assert!(pixel(&draw(&d, 10.0, 400), 85, 150)[3] > 0);
    // A fill's own Add Around that stays inside a 40 pt stroke takes no room past the stroke.
    let mut d = doc(shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0)), &[("rasterize", json!({"background": "white"}))], 72.0);
    let l = d.layers[0].id;
    let n = Arc::make_mut(&mut d.children_mut(Some(l)).unwrap()[0]);
    n.appearance.items[0].effects_mut().push(Effect { id: "rasterize".into(), params: json!({"addAround": 10}), visible: true });
    n.appearance.items[1] = vectorcraft_doc::AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 40.0));
    let r = draw(&d, 2.0, 200);
    assert_eq!((pixel(&r, 156, 100)[3], pixel(&r, 170, 100)[3]), (255, 0));
}

#[test]
fn rasterize_and_shadows_paint_in_stack_order() {
    let square = || shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0));
    let shadow = ("stylize.dropShadow", json!({"mode": "normal", "opacity": 100, "x": 5, "y": 5, "blur": 0, "color": "#0000ff"}));
    let gray = ("rasterize", json!({"colorModel": "grayscale"}));
    // Below the square's corner, only the shadow paints.
    let first = draw(&doc(square(), &[shadow.clone(), gray.clone()], 72.0), 10.0, 400);
    let [r, g, b, a] = pixel(&first, 330, 330);
    assert!(a == 255 && r == g && g == b, "the shadow is rasterized in grayscale: {:?}", [r, g, b, a]);
    let after = draw(&doc(square(), &[gray.clone(), shadow.clone()], 72.0), 10.0, 400);
    assert_eq!(pixel(&after, 330, 330), [0, 0, 255, 255], "a shadow after Rasterize keeps its color");
    // A shadow after Rasterize is the image's: a circle's square image on white casts a square shadow.
    let circle = shapes::ellipse(Rect::new(10.0, 10.0, 30.0, 30.0));
    let cast = draw(&doc(circle, &[("rasterize", json!({"background": "white"})), shadow.clone()], 72.0), 10.0, 400);
    assert_eq!(pixel(&cast, 320, 160), [0, 0, 255, 255]);
    // Between two Rasterize effects, the second one takes it in.
    let between = draw(&doc(square(), &[("rasterize", json!({})), shadow.clone(), gray], 72.0), 10.0, 400);
    let [r, g, b, _] = pixel(&between, 330, 330);
    assert!(r == g && g == b && b > 0, "{:?}", [r, g, b]);
    // A member's shadow is part of a group's art.
    let mut d = doc(square(), std::slice::from_ref(&shadow), 72.0);
    let l = d.layers[0].id;
    let members = d.children_mut(Some(l)).map(std::mem::take).unwrap();
    let mut group = Node::group(d.alloc_id(), members);
    group.appearance.effects.push(Effect { id: "rasterize".into(), params: json!({}), visible: true });
    d.insert(Some(l), 0, group).unwrap();
    assert_eq!(pixel(&draw(&d, 10.0, 400), 330, 330), [0, 0, 255, 255]);
    // So are a member's Crop Marks: the top right mark runs from x 39 to 57 at y 10.
    let mut d = doc(square(), &[("cropMarks", json!({}))], 72.0);
    let l = d.layers[0].id;
    let members = d.children_mut(Some(l)).map(std::mem::take).unwrap();
    let mut group = Node::group(d.alloc_id(), members);
    group.appearance.effects.push(Effect { id: "rasterize".into(), params: json!({}), visible: true });
    d.insert(Some(l), 0, group).unwrap();
    let r = draw(&d, 10.0, 400);
    assert!((95..105).any(|y| pixel(&r, 395, y)[3] > 0));
    // So is the shadow in a symbol's art, for its instance.
    let mut d = Document::new(100.0, 100.0);
    let mut art = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    art.appearance.effects.push(Effect { id: shadow.0.into(), params: shadow.1.clone(), visible: true });
    d.symbols.push(vectorcraft_doc::Symbol { name: "Shadowed".into(), art: Arc::new(art) });
    let mut instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Shadowed".into(), xf: Affine::translate((20.0, 20.0)) });
    instance.appearance.effects.push(Effect { id: "rasterize".into(), params: json!({}), visible: true });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, instance).unwrap();
    assert_eq!(pixel(&draw(&d, 10.0, 400), 330, 330), [0, 0, 255, 255]);
}

#[test]
fn rasterize_takes_in_the_art_of_a_3d_effect() {
    let square = || shapes::rectangle(Rect::new(30.0, 30.0, 50.0, 50.0));
    let extrude = ("threeD.extrude", json!({}));
    let solid = draw(&doc(square(), std::slice::from_ref(&extrude), 72.0), 10.0, 800);
    // Pixels the extruded art covers outside the square.
    let outside: Vec<(usize, usize)> = (0..800 * 800)
        .map(|i| (i % 800, i / 800))
        .filter(|(x, y)| !((300..500).contains(x) && (300..500).contains(y)) && pixel(&solid, *x, *y)[3] == 255)
        .collect();
    assert!(!outside.is_empty());
    let rasterized = draw(&doc(square(), &[extrude, ("rasterize", json!({}))], 72.0), 10.0, 800);
    assert!(outside.iter().all(|(x, y)| pixel(&rasterized, *x, *y)[3] > 0));
}

#[test]
fn rasterize_takes_in_brush_art_wider_than_the_stroke() {
    // A 1 pt line under the 10 pt. Oval brush, which paints about 2 pt from its center line.
    let mut d = Document::new(200.0, 100.0);
    let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    ap.stroke_mut().unwrap().brush = Some("10 pt. Oval".into());
    let mut n = Node::path(d.alloc_id(), shapes::line((20.0, 50.0).into(), (180.0, 50.0).into()), ap);
    n.appearance.effects.push(Effect { id: "rasterize".into(), params: json!({}), visible: true });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = Renderer::new().render(&d, 200, 100, Affine::IDENTITY, &RenderOptions { background: None, artboards: false, ..Default::default() });
    assert!(pixel(&r, 100, 52)[3] > 128, "{:?}", pixel(&r, 100, 52));
}

#[test]
fn rasterize_takes_in_type_that_its_own_fills_effects_move() {
    // "IIII" is set at 40 pt from x 5 pt with a red fill of its own under the characters, and a
    // Transform effect on that fill moves it 100 pt to the right.
    let typed = |fx: &[Effect]| {
        let mut d = Document::new(200.0, 100.0);
        let style = vectorcraft_doc::text::CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
        let text = vectorcraft_doc::text::TextObject::point(Point::new(5.0, 60.0), "IIII", style);
        let mut n = Node::new(d.alloc_id(), vectorcraft_doc::NodeKind::Text(Box::new(text)));
        let mut fill = vectorcraft_doc::FillLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
        fill.effects.push(Effect { id: "distort.transform".into(), params: json!({"moveH": 100}), visible: true });
        n.appearance.items.push(vectorcraft_doc::AppearanceItem::Fill(fill));
        n.appearance.set_contents_at(1);
        n.appearance.effects = fx.to_vec();
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        Renderer::new().render(&d, 200, 100, Affine::IDENTITY, &RenderOptions { background: None, artboards: false, ..Default::default() })
    };
    // The pixels right of x 100 that the moved fill paints.
    let moved = |r: &Rendered| (0..200 * 100).filter(|i| i % 200 >= 100 && pixel(r, i % 200, i / 200)[0] > 128).count();
    assert!(moved(&typed(&[])) > 50);
    let rasterize = Effect { id: "rasterize".into(), params: json!({"useDocumentResolution": false, "resolution": 72}), visible: true };
    assert!(moved(&typed(&[rasterize])) > 50);
}

#[test]
fn rasterize_fills_its_area_on_white_and_add_around_grows_it() {
    let circle = || shapes::ellipse(Rect::new(10.0, 10.0, 30.0, 30.0));
    let at = |params: Value| draw(&doc(circle(), &[("rasterize", params)], 72.0), 10.0, 400);
    let white = at(json!({"background": "white"}));
    // Inside the bounds, outside the circle: white; outside the bounds: nothing.
    assert_eq!((pixel(&white, 105, 105), pixel(&white, 200, 200)), ([255; 4], [255, 0, 0, 255]));
    assert_eq!(pixel(&white, 95, 200)[3], 0);
    let clear = at(json!({"background": "transparent"}));
    assert_eq!((pixel(&clear, 105, 105)[3], pixel(&clear, 200, 200)), (0, [255, 0, 0, 255]));
    // Add Around Object: 2 pt more on every side.
    let around = at(json!({"background": "white", "addAround": 2}));
    assert_eq!((pixel(&around, 85, 200), pixel(&around, 75, 200)[3]), ([255; 4], 0));
    // A second Rasterize makes its image over the first one's whole pixels, no larger.
    let twice = draw(&doc(circle(), &[("rasterize", json!({})), ("rasterize", json!({"background": "white"}))], 72.0), 10.0, 400);
    assert_eq!((pixel(&twice, 105, 105), pixel(&twice, 95, 200)[3], pixel(&twice, 305, 200)[3]), ([255; 4], 0, 0));
    // A Feather or an Inner Glow after a Rasterize on white works on the image's square edge.
    let white = ("rasterize", json!({"background": "white"}));
    let feathered = draw(&doc(circle(), &[white.clone(), ("stylize.feather", json!({"radius": 1}))], 72.0), 10.0, 400);
    assert!(pixel(&feathered, 106, 106)[3] > 0);
    let glow = ("stylize.innerGlow", json!({"mode": "normal", "opacity": 100, "blur": 2, "color": "#ff0000", "source": "edge"}));
    let glowing = draw(&doc(circle(), &[white.clone(), glow.clone()], 72.0), 10.0, 400);
    assert!(pixel(&glowing, 102, 102)[1] < 200, "{:?}", pixel(&glowing, 102, 102));
    // With a Rasterize on nothing between them, the glow follows the same edge.
    let glowing = draw(&doc(circle(), &[white, ("rasterize", json!({})), glow], 72.0), 10.0, 400);
    assert!(pixel(&glowing, 102, 102)[1] < 200, "{:?}", pixel(&glowing, 102, 102));
    // Create Clipping Mask: the white stays inside the circle.
    let clipped = at(json!({"background": "white", "clippingMask": true}));
    assert_eq!((pixel(&clipped, 105, 105)[3], pixel(&clipped, 200, 200)), (0, [255, 0, 0, 255]));
    // Inside the outline the white shows where the art paints nothing: a circle stroked only.
    let mut ring = doc(circle(), &[], 72.0);
    let l = ring.layers[0].id;
    let n = Arc::make_mut(&mut ring.children_mut(Some(l)).unwrap()[0]);
    n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 1.0).into();
    n.appearance.effects.push(Effect { id: "rasterize".into(), params: json!({"background": "white", "clippingMask": true}), visible: true });
    let ring = draw(&ring, 10.0, 400);
    assert_eq!((pixel(&ring, 200, 200), pixel(&ring, 105, 105)[3]), ([255; 4], 0));
    // A stroke 1 pt wide widens the image by half a point a side, as it widens the visual bounds.
    let mut stroked = doc(circle(), &[], 72.0);
    let n = Arc::make_mut(&mut stroked.children_mut(Some(l)).unwrap()[0]);
    n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 1.0).into();
    n.appearance.effects.push(Effect {
        id: "rasterize".into(),
        params: json!({"background": "white", "resolution": 720, "useDocumentResolution": false}),
        visible: true,
    });
    let stroked = draw(&stroked, 10.0, 400);
    assert_eq!((pixel(&stroked, 96, 105), pixel(&stroked, 93, 105)[3]), ([255; 4], 0));
}
