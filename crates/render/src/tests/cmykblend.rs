//! Blending in CMYK documents ([`crate::ink`]): blend modes and opacity composite inks.

use crate::*;
use vectorcraft_color::blend::MASK_LUM;
use vectorcraft_color::{BlendMode as Blend, Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, ColorMode, ImageBlob, ImageObject, Node, OpacityMask};
use vectorcraft_geom::shapes;

fn rect(d: &mut Document, r: Rect, c: Color) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(c), Paint::None, 0.0))
}

fn add(d: &mut Document, n: Node) {
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

/// `below` over the whole 20 pt page and `top` (10 pt in the middle) with `mode` and `opacity`.
fn doc(mode: ColorMode, below: Option<Color>, top: Color, blend: Blend, opacity: f32) -> Document {
    let mut d = Document::new_with_mode(20.0, 20.0, mode);
    if let Some(b) = below {
        let n = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), b);
        add(&mut d, n);
    }
    let mut n = rect(&mut d, Rect::new(5.0, 5.0, 15.0, 15.0), top);
    (n.blend, n.opacity) = (blend, opacity);
    add(&mut d, n);
    d
}

/// The centre pixel, rendered on white.
fn centre(d: &Document) -> [u8; 4] {
    Renderer::new().render_region(d, d.artboards[0].rect, 1.0, true).pixel(10, 10)
}

/// Screen colour of inks `cmyk`.
fn shown(cmyk: [f32; 4]) -> [u8; 4] {
    Color::cmyk(cmyk[0], cmyk[1], cmyk[2], cmyk[3]).to_rgba8(1.0)
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

const CYAN: Color = Color::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0 };
const MAGENTA: Color = Color::Cmyk { c: 0.0, m: 1.0, y: 0.0, k: 0.0 };

#[test]
fn cyan_multiplied_over_magenta_prints_both_inks() {
    let got = centre(&doc(ColorMode::Cmyk, Some(MAGENTA), CYAN, Blend::Multiply, 1.0));
    let want = shown([1.0, 1.0, 0.0, 0.0]);
    assert!(near(got, want, 1), "{got:?} ≠ C100 M100 {want:?}");
    // Multiplying the screen colours instead gives a visibly different blue.
    let (c, m) = (CYAN.to_rgb(), MAGENTA.to_rgb());
    let screen = [0, 1, 2].map(|i| (c[i] * m[i] * 255.0).round() as i32);
    assert!((0..3).any(|i| (screen[i] - want[i] as i32).abs() > 10), "{screen:?} vs {want:?}");
}

#[test]
fn opacity_mixes_inks() {
    let got = centre(&doc(ColorMode::Cmyk, None, CYAN, Blend::Normal, 0.5));
    assert!(near(got, shown([0.5, 0.0, 0.0, 0.0]), 2), "{got:?}");
}

#[test]
fn black_blends_on_its_own_plane() {
    let k = |k| Color::cmyk(0.0, 0.0, 0.0, k);
    // Multiply adds black ink: K50 over K50 leaves 25% of the paper.
    let got = centre(&doc(ColorMode::Cmyk, Some(k(0.5)), k(0.5), Blend::Multiply, 1.0));
    assert!(near(got, shown([0.0, 0.0, 0.0, 0.75]), 2), "{got:?}");
    // Luminosity takes the source's black, Color keeps the backdrop's.
    let got = centre(&doc(ColorMode::Cmyk, Some(CYAN), k(0.6), Blend::Luminosity, 1.0));
    assert!(near(got, shown([0.0, 0.0, 0.0, 0.6]), 3), "{got:?}");
    // (Over a backdrop of black ink alone the C, M, Y plane is paper: its luminance takes the
    // source's colour out.)
    let got = centre(&doc(ColorMode::Cmyk, Some(k(0.5)), CYAN, Blend::Color, 1.0));
    assert!(near(got, shown([0.0, 0.0, 0.0, 0.5]), 2), "{got:?}");
}

#[test]
fn rgb_documents_and_opaque_cmyk_documents_keep_screen_colours() {
    let rgb = |c: &Color| {
        let [r, g, b] = c.to_rgb();
        Color::rgb(r, g, b)
    };
    // RGB documents multiply screen colours.
    let got = centre(&doc(ColorMode::Rgb, Some(rgb(&MAGENTA)), rgb(&CYAN), Blend::Multiply, 1.0));
    let (c, m) = (CYAN.to_rgb(), MAGENTA.to_rgb());
    let want = [0, 1, 2].map(|i| (c[i] * m[i] * 255.0).round() as u8);
    assert!(near(got, [want[0], want[1], want[2], 255], 1), "{got:?} ≠ {want:?}");
    // A CMYK document without transparency paints each colour as it is.
    let d = doc(ColorMode::Cmyk, Some(MAGENTA), CYAN, Blend::Normal, 1.0);
    assert!(!Renderer::new().blends_in_cmyk(&d, &RenderOptions::default()));
    assert_eq!(centre(&d), CYAN.to_rgba8(1.0));
}

/// Whether a document shows transparency is found once for its layers, symbol art and pattern
/// art, and kept while the document holds the same art: an edit, or a new symbol, finds it again.
#[test]
fn the_transparency_test_is_kept_while_the_art_stays_the_same() {
    let frame = |r: &mut Renderer, d: &Document| r.render_region(d, d.artboards[0].rect, 1.0, true).pixels;
    let kept = |r: &Renderer| r.transparency.as_ref().map(|(art, shows)| (art.as_ptr(), *shows));
    // The ink planes show a Gray fill lighter than screen colors do, so a frame drawn the other
    // way differs.
    let d = doc(ColorMode::Cmyk, Some(Color::gray(0.9)), CYAN, Blend::Normal, 0.5);
    let mut r = Renderer::new();
    let first = frame(&mut r, &d);
    let found = kept(&r);
    assert_eq!(found.map(|(_, shows)| shows), Some(true));
    assert!(frame(&mut r, &d) == first);
    assert_eq!(kept(&r), found, "the same art was tested again");
    // The top object made opaque: the frame paints screen colors.
    let mut e = d.clone();
    let top = e.layers[0].children().unwrap()[1].id;
    e.node_mut(top).unwrap().opacity = 1.0;
    assert!(frame(&mut r, &e) == frame(&mut Renderer::new(), &e));
    assert_eq!(kept(&r).map(|(_, shows)| shows), Some(false));
    // A symbol whose art shows transparency, placed nowhere, makes the page draw by ink planes again.
    let mut art = rect(&mut e, Rect::new(0.0, 0.0, 5.0, 5.0), CYAN);
    art.opacity = 0.5;
    e.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art) });
    assert!(frame(&mut r, &e) == frame(&mut Renderer::new(), &e));
    assert_eq!(kept(&r).map(|(_, shows)| shows), Some(true));
}

/// A 200 × 150 pt CMYK page drawn by ink planes: a Gray background, translucent ellipses in each
/// blend mode, an object with an opacity mask, one with a drop shadow, a gradient with a
/// translucent stop, an overprinting fill and objects across the page's edges.
fn busy_page() -> Document {
    let mut d = Document::new_with_mode(200.0, 150.0, ColorMode::Cmyk);
    let back = rect(&mut d, Rect::new(0.0, 0.0, 200.0, 150.0), Color::gray(0.3));
    add(&mut d, back);
    for i in 0..240 {
        let (x, y) = ((i * 37 % 200) as f64, (i * 53 % 150) as f64);
        let c = Color::cmyk((i % 5) as f32 / 4.0, (i % 7) as f32 / 6.0, (i % 3) as f32 / 2.0, (i % 4) as f32 / 8.0);
        let mut n = Node::path(
            d.alloc_id(),
            shapes::ellipse(Rect::new(x - 9.0, y - 6.0, x + 9.0, y + 6.0)),
            Appearance::basic(Paint::solid(c), Paint::solid(MAGENTA), 0.6),
        );
        (n.blend, n.opacity) = (Blend::ALL[i % Blend::ALL.len()], 0.4 + (i % 6) as f32 / 10.0);
        add(&mut d, n);
    }
    let mut masked = rect(&mut d, Rect::new(20.0, 20.0, 90.0, 70.0), CYAN);
    let mask = Node::path(
        d.alloc_id(),
        shapes::ellipse(Rect::new(30.0, 25.0, 80.0, 65.0)),
        Appearance::basic(Paint::solid(Color::gray(0.4)), Paint::None, 0.0),
    );
    masked.extras_mut().mask = Some(Box::new(OpacityMask::new(mask, true)));
    add(&mut d, masked);
    let mut shadowed = rect(&mut d, Rect::new(110.0, 20.0, 170.0, 60.0), Color::cmyk(0.0, 0.2, 0.9, 0.0));
    let shadow = serde_json::json!({"x": 6.0, "y": 6.0, "blur": 4.0, "opacity": 75.0, "mode": "multiply"});
    shadowed.appearance.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: shadow, visible: true });
    add(&mut d, shadowed);
    let stops = vec![GradientStop::new(0.0, CYAN), GradientStop { opacity: 0.3, ..GradientStop::new(1.0, MAGENTA) }];
    let ramp = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 90.0, 120.0, 130.0)), Appearance::basic(ramp, Paint::None, 0.0));
    add(&mut d, n);
    let mut over = rect(&mut d, Rect::new(100.0, 80.0, 160.0, 120.0), Color::cmyk(0.0, 0.0, 1.0, 0.0));
    if let Some(AppearanceItem::Fill(f)) = over.appearance.items.first_mut() {
        f.overprint = true;
    }
    add(&mut d, over);
    for r in [Rect::new(-15.0, -10.0, 25.0, 20.0), Rect::new(180.0, 130.0, 230.0, 170.0), Rect::new(-30.0, 60.0, 260.0, 75.0)] {
        let mut n = rect(&mut d, r, Color::cmyk(0.7, 0.0, 0.3, 0.1));
        n.opacity = 0.7;
        add(&mut d, n);
    }
    d
}

/// The renderer's threads each compose a band of a frame's two ink planes, and the frame has the
/// pixels that one thread composes, with and without a background.
#[test]
fn ink_planes_compose_on_the_renderer_threads() {
    let d = busy_page();
    let canvas = RenderOptions { background: Some([224, 224, 224, 255]), artboards: true, overprint_preview: true, ..Default::default() };
    for opts in [RenderOptions::default(), canvas] {
        let frame = |threads: u16| {
            let mut r = Renderer::new();
            r.threads = threads;
            let pixels = r.render(&d, 1200, 900, Affine::scale(6.0), &opts).pixels;
            (pixels, r.stats.ink_threads)
        };
        let (one, threads) = frame(0);
        assert_eq!(threads, 1);
        for n in [3, 8] {
            let (pixels, threads) = frame(n);
            assert_eq!(threads, usize::from(n), "composed on {threads} threads, not {n}");
            assert!(pixels == one, "{n} threads changed pixels");
        }
    }
}

#[test]
fn opacity_masks_take_screen_luminance() {
    let mut d = Document::new_with_mode(20.0, 20.0, ColorMode::Cmyk);
    let mut n = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), Color::cmyk(0.0, 0.0, 0.0, 1.0));
    let art = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), CYAN);
    n.extras_mut().mask = Some(Box::new(OpacityMask::new(art, true)));
    add(&mut d, n);
    let [r, g, b] = CYAN.to_rgb();
    let cover = MASK_LUM[0] * r + MASK_LUM[1] * g + MASK_LUM[2] * b;
    let got = centre(&d);
    assert!(near(got, shown([0.0, 0.0, 0.0, cover]), 3), "{got:?} ≠ K{cover}");
}

#[test]
fn placed_images_are_separated_not_blackened() {
    let mut d = doc(ColorMode::Cmyk, None, CYAN, Blend::Normal, 0.5);
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let n = Node::new(
        d.alloc_id(),
        NodeKind::Image(Box::new(ImageObject {
            key: "red".into(),
            width: 4,
            height: 4,
            xf: Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    add(&mut d, n);
    let got = Renderer::new().render_region(&d, d.artboards[0].rect, 1.0, true).pixel(1, 1);
    let cms = vectorcraft_color::cms::active();
    let [r, g, b] = cms.cmyk_to_srgb(cms.srgb_to_cmyk([1.0, 0.0, 0.0], cms.settings().intent), false).map(|v| (v * 255.0).round() as u8);
    assert!(near(got, [r, g, b, 255], 4), "{got:?} ≠ {:?}", [r, g, b]);
}

/// Appearance of Black's Rich Black (`rich_black`, #394): pure CMYK black (black ink alone at
/// 100%) shows as RGB black, in an opaque CMYK document (screen colours) and in one drawn by ink
/// planes (Multiply); a rich black, a black tint and placed images show as they do without it.
#[test]
fn rich_black_shows_black_ink_alone_as_rgb_black() {
    let k = |k| Color::cmyk(0.0, 0.0, 0.0, k);
    let centre_with = |d: &Document, rich_black| {
        let opts = RenderOptions { background: Some([255; 4]), skip_templates: true, rich_black, ..Default::default() };
        Renderer::new().render_region_with(d, d.artboards[0].rect, 1.0, &opts).pixel(10, 10)
    };
    for blend in [Blend::Normal, Blend::Multiply] {
        let black = doc(ColorMode::Cmyk, None, k(1.0), blend, 1.0);
        let accurate = centre_with(&black, false);
        assert!(near(accurate, shown([0.0, 0.0, 0.0, 1.0]), 1) && accurate[0] > 10, "{blend:?}: accurate {accurate:?}");
        assert_eq!(centre_with(&black, true), [0, 0, 0, 255], "{blend:?}: rich");
        for c in [Color::cmyk(0.6, 0.4, 0.4, 1.0), k(0.5)] {
            let d = doc(ColorMode::Cmyk, None, c, blend, 1.0);
            assert_eq!(centre_with(&d, true), centre_with(&d, false), "{blend:?}: {c:?}");
        }
    }
    let mut d = Document::new_with_mode(20.0, 20.0, ColorMode::Cmyk);
    let img = image::RgbaImage::from_pixel(20, 20, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let im = ImageObject { key: "red".into(), width: 20, height: 20, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    add(&mut d, n);
    assert_eq!(centre_with(&d, true), centre_with(&d, false), "an image shows as it is");
}

/// An overprinting stroke in the colour of the fill under it shows no darker band in Overprint
/// Preview: the same ink printed over itself is that ink (#2415). Multiply would square the colour.
#[test]
fn overprint_of_the_same_colour_shows_no_darker_band() {
    for (mode, c) in [(ColorMode::Rgb, Color::rgb(0.0, 0.55, 0.29)), (ColorMode::Cmyk, Color::cmyk(0.85, 0.0, 1.0, 0.0))] {
        let mut d = Document::new_with_mode(40.0, 40.0, mode);
        let mut n =
            Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), Appearance::basic(Paint::solid(c), Paint::solid(c), 8.0));
        for it in &mut n.appearance.items {
            *it.overprint_mut() = true;
        }
        add(&mut d, n);
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), overprint_preview: true, ..Default::default() };
        let r = Renderer::new().render_region_with(&d, d.artboards[0].rect, 1.0, &opts);
        // (20, 20) is fill only; (12, 20) lies in the stroke's inner half, over the fill.
        assert!(near(r.pixel(12, 20), r.pixel(20, 20), 2), "{mode:?}: {:?} vs {:?}", r.pixel(12, 20), r.pixel(20, 20));
    }
}
