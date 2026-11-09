use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::{DRAG_PIXEL_BUDGET, Drawn, PIXEL_BUDGET, drag_scale, filter_region, filter_region_within, pan_margin};
use crate::tests_fontchange::{fonts_steady, with_fonts_unchanged};
use crate::{RenderOptions, Rendered, Renderer};

/// The hash renderers compare to redraw art linked to swatches changes with a global swatch's
/// color and spot flag and with the Spot Colors option.
#[test]
fn paint_names_follow_global_swatch_colors_and_spot_flags() {
    let mut d = Document::new(100.0, 100.0);
    d.swatches.push(vectorcraft_doc::color::Swatch {
        name: "Brand".into(),
        paint: Paint::solid(Color::rgb(1.0, 0.0, 0.0)),
        global: true,
        spot: false,
    });
    let last = d.swatches.len() - 1;
    let first = super::paint_names(&d);
    d.swatches[last].spot = true;
    assert_ne!(super::paint_names(&d), first, "spot flag");
    d.swatches[last].spot = false;
    assert_eq!(super::paint_names(&d), first, "nothing changed");
    d.swatches[last].paint = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    assert_ne!(super::paint_names(&d), first, "color");
    d.swatches[last].paint = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    d.spot_use_lab = !d.spot_use_lab;
    assert_ne!(super::paint_names(&d), first, "Spot Colors option");
}

/// #1706: the raster a shadow or glow is filtered in stays within the pixel budget however
/// far the blur spreads at the zoom (it was up to 65535 pixels a side: 16 GB).
#[test]
fn a_wide_blur_at_a_high_zoom_is_filtered_within_the_pixel_budget() {
    // A 100 pt square at 3200 % and at 6400 %, blurred by 1000 pt, in a 1600 × 1000 view.
    for zoom in [32.0, 64.0] {
        let reach = Rect::new(0.0, 0.0, 100.0 * zoom, 100.0 * zoom);
        let (x0, y0, w, h, k) = filter_region(reach, (1600, 1000), 1000.0 * zoom).unwrap();
        assert!(f64::from(w) * f64::from(h) <= PIXEL_BUDGET, "{w} × {h}");
        assert!(k < 1.0 && x0 < 0.0 && y0 < 0.0);
        // It still covers the view and the blur's spread around it, at that scale.
        assert!(f64::from(w) / k >= 1600.0 + 1500.0 * zoom && f64::from(h) / k >= 1000.0 + 1500.0 * zoom);
    }
    // A small blur is filtered at full size, cropped to what can show.
    assert_eq!(filter_region(Rect::new(10.0, 20.0, 110.0, 70.0), (1600, 1000), 4.0), Some((2.0, 12.0, 116, 66, 1.0)));
    assert_eq!(filter_region(Rect::new(5000.0, 0.0, 5100.0, 100.0), (1600, 1000), 4.0), None, "off the view");
    assert_eq!(filter_region(Rect::new(0.0, 0.0, 100.0, 100.0), (1600, 1000), f64::NAN).map(|r| r.4), Some(1.0));
}

/// #2558: while dragging, content and a shadow's region over the drag's budget at their scale
/// render at a lower scale, within the budget; otherwise their scale stays as it is.
#[test]
fn a_drag_lowers_the_scale_of_rasters_over_its_budget() {
    let d = drag_scale(true, (2560.0, 1440.0), 1.0).unwrap();
    assert!(d < 1.0 && (2560.0 * d) * (1440.0 * d) <= DRAG_PIXEL_BUDGET * (1.0 + 1e-9), "{d}");
    assert_eq!(drag_scale(false, (2560.0, 1440.0), 1.0), None, "not dragging");
    assert_eq!(drag_scale(true, (1000.0, 800.0), 1.0), None, "within the budget");
    // At a scale already lowered to 0.5, 1280 × 720 raster pixels: within the budget.
    assert_eq!(drag_scale(true, (2560.0, 1440.0), 0.5), None);
    // A 2000 × 1500 pixel square blurred by 20 pixels in a 1600 × 1000 view.
    let reach = Rect::new(-200.0, -200.0, 1800.0, 1300.0);
    let (x0, y0, w, h, k) = filter_region_within(reach, (1600, 1000), 20.0, DRAG_PIXEL_BUDGET).unwrap();
    assert!(f64::from(w) * f64::from(h) <= DRAG_PIXEL_BUDGET && k < 1.0, "{w} × {h} at {k}");
    // It covers the same pixels as the full-size region, at that scale.
    let full = filter_region(reach, (1600, 1000), 20.0).unwrap();
    assert_eq!((x0, y0, full.4), (full.0, full.1, 1.0));
    assert!((f64::from(w) / k - f64::from(full.2)).abs() <= 1.0 / k && (f64::from(h) / k - f64::from(full.3)).abs() <= 1.0 / k);
    let small = Rect::new(10.0, 20.0, 110.0, 70.0);
    assert_eq!(filter_region_within(small, (1600, 1000), 4.0, DRAG_PIXEL_BUDGET), filter_region(small, (1600, 1000), 4.0));
}

/// The margin a raster cropped to the view is made again with keeps it within the pixel budget,
/// a pixel of rounding on each edge included, and within the side limit; it is at most half the
/// view's longer side, and 0 when the view and its spread alone fill the budget.
#[test]
fn the_margin_for_pans_stays_within_the_pixel_budget() {
    for ((w, h), m, want) in [((2560.0, 1440.0), 32.0, 441.0), ((640.0, 360.0), 32.0, 320.0), ((3840.0, 2160.0), 32.0, 0.0)] {
        let e = pan_margin((w, h), m, PIXEL_BUDGET, 8192.0);
        assert_eq!(e, want, "{w} × {h}");
        let (rw, rh) = (w + 2.0 * (m + e) + 2.0, h + 2.0 * (m + e) + 2.0);
        assert!(e == 0.0 || rw * rh <= PIXEL_BUDGET, "{w} × {h}: {rw} × {rh}");
    }
    assert_eq!(pan_margin((2000.0, 10.0), 0.0, 1e9, 3000.0), 498.0, "within the side limit");
    assert_eq!(pan_margin((2000.0, 10.0), f64::NAN, PIXEL_BUDGET, 8192.0), 0.0);
}

fn doc_with(r: Rect, fill: Color, effects: Vec<(&str, serde_json::Value)>) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let id = d.alloc_id();
    let mut n = Node::path(id, shapes::rectangle(r), Appearance::basic(Paint::solid(fill), Paint::None, 0.0));
    n.appearance.effects = effects.into_iter().map(|(id, p)| Effect { id: id.into(), params: p, visible: true }).collect();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn lum(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

const WHITE: u32 = 765;

/// A page of objects with object-level and item-level geometry effects (Offset Path, Roughen,
/// Warp, Outline Stroke, Transform) under drop shadows and in groups.
fn effects_page() -> Document {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let fx = |id: &str, p: serde_json::Value| Effect { id: id.into(), params: p, visible: true };
    for i in 0..24 {
        let (x, y) = (10.0 + (i % 6) as f64 * 30.0, 10.0 + (i / 6) as f64 * 45.0);
        let id = d.alloc_id();
        let r = Rect::new(x, y, x + 20.0, y + 30.0);
        let shape = if i % 2 == 0 { shapes::ellipse(r) } else { shapes::star(r.center(), 12.0, 5.0, 5, 0.0) };
        let mut n = Node::path(id, shape, Appearance::basic(Paint::solid(Color::rgb(0.2, 0.4, 0.9)), Paint::solid(Color::BLACK), 1.5));
        n.appearance.effects = match i % 4 {
            0 => vec![fx("path.offsetPath", json!({"offset": 3}))],
            1 => vec![fx("distort.roughen", json!({"size": 4, "relative": true})), fx("stylize.dropShadow", json!({}))],
            2 => vec![fx("warp.arc", json!({"bend": 30})), fx("path.outlineStroke", json!({}))],
            _ => vec![fx("path.offsetPath", json!({"offset": -2})), fx("distort.transform", json!({"moveH": 4, "rotate": 10}))],
        };
        if let Some(f) = n.appearance.fill_mut() {
            f.effects.push(fx("path.offsetPath", json!({"offset": 2})));
        }
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    d
}

/// Cached live-effect geometry draws exactly what computing it afresh draws: a second (warm)
/// frame, a frame after panning, and a fresh renderer all give the same pixels, and an edited
/// object (a new node) draws its new geometry, not the cached one.
#[test]
fn cached_effect_geometry_draws_the_same_pixels() {
    let d = effects_page();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let view = Affine::scale(2.0);
    let mut r = Renderer::new();
    let cold = r.render(&d, 400, 400, view, &opts);
    let warm = r.render(&d, 400, 400, view, &opts);
    assert!(cold.pixels == warm.pixels, "a warm frame differs from the cold one");
    let _ = r.render(&d, 400, 400, Affine::translate((15.0, 0.0)) * view, &opts);
    assert!(r.render(&d, 400, 400, view, &opts).pixels == cold.pixels, "after a pan");
    assert!(Renderer::new().render(&d, 400, 400, view, &opts).pixels == cold.pixels, "a fresh renderer");
    // Edit one object's effect: the warm renderer must draw the new result.
    let mut e = d.clone();
    let id = e.layers[0].children().unwrap()[0].id;
    e.node_mut(id).unwrap().appearance.effects = vec![Effect { id: "path.offsetPath".into(), params: json!({"offset": 9}), visible: true }];
    let edited = r.render(&e, 400, 400, view, &opts);
    assert!(edited.pixels != cold.pixels, "the edit shows");
    assert!(edited.pixels == Renderer::new().render(&e, 400, 400, view, &opts).pixels, "and matches a fresh render");
}

/// Shadow rasters of objects in the tree are matched by the node's `Arc` (kept alive), not by
/// comparing nodes; over the byte budget the least recently drawn go first.
#[test]
fn shadow_rasters_keep_their_node_and_a_byte_budget() {
    let d = effects_page();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let first = r.render(&d, 400, 400, Affine::scale(2.0), &opts);
    assert!(!r.shadows.is_empty() && r.shadows.values().all(|e| matches!(e.node, Drawn::Shared(_))));
    // A frame of other objects makes rasters of its own; the older ones go first.
    let _ = r.render(&effects_page(), 400, 400, Affine::scale(2.0), &opts);
    let newest = r.stamp;
    let bytes = |r: &Renderer| r.shadows.values().map(|e| e.bytes()).sum::<usize>();
    let latest = r.shadows.values().filter(|e| e.stamp == newest).map(|e| e.bytes()).sum::<usize>();
    assert!(bytes(&r) > latest);
    r.trim_fx_rasters(latest);
    assert_eq!(bytes(&r), latest);
    assert!(r.shadows.values().all(|e| e.stamp == newest));
    assert!(r.render(&d, 400, 400, Affine::scale(2.0), &opts).pixels == first.pixels, "made again alike");
}

/// An Outer Glow whose whole reach is over the budget is made coarser over that whole reach and
/// kept for any pan: its grid starts at the reach's first whole device pixel, which a pan by whole
/// pixels moves with the art, so the raster drawn after the pan is the one a fresh renderer makes,
/// also for pans that aren't multiples of the raster's scale. A pan by a fraction of a pixel
/// places the raster at the nearest whole pixel, as a full-size raster kept whole is placed, and
/// here draws as a fresh renderer draws too. A 100 pt glow at 500% (scale 2) and a 2 pt glow
/// round the page at 800% (scale 3, near the narrowest blur made coarser).
#[test]
fn a_coarser_glow_kept_whole_draws_after_a_pan_as_made_again() {
    let size = (640, 360);
    for (rect, blur, zoom, center, scale) in
        [(Rect::new(200.0, 300.0, 400.0, 500.0), 100.0, 5.0, (440.0, 400.0), 2.0), (Rect::new(0.0, 0.0, 595.0, 842.0), 2.0, 8.0, (595.0, 421.0), 3.0)]
    {
        let mut d = Document::new(595.0, 842.0);
        let layer = d.layers[0].id;
        let mut n = Node::path(d.alloc_id(), shapes::rectangle(rect), Appearance::basic(Paint::solid(Color::rgb(0.1, 0.3, 0.9)), Paint::None, 0.0));
        n.appearance.effects = vec![Effect { id: "stylize.outerGlow".into(), params: json!({"blur": blur}), visible: true }];
        d.insert(Some(layer), 0, n).unwrap();
        let base = Affine::translate((320.0, 180.0)) * Affine::scale(zoom) * Affine::translate((-center.0, -center.1));
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        r.threads = 2;
        r.render(&d, size.0, size.1, base, &opts);
        let image = |r: &Renderer| r.shadows.values().map(|e| std::sync::Arc::as_ptr(&e.image)).collect::<Vec<_>>();
        let kept = image(&r);
        assert!(r.shadows.values().all(|e| e.scale == scale && e.view.is_none()), "blur {blur}: coarser, over the whole reach");
        for pan in [(37.0, -21.0), (1.0, 0.0), (-3.0, 5.0), (0.5, 0.0), (37.3, -21.6)] {
            let xf = Affine::translate(pan) * base;
            let panned = r.render(&d, size.0, size.1, xf, &opts);
            assert_eq!(image(&r), kept, "blur {blur}, pan {pan:?}: the raster kept is drawn");
            let mut fresh = Renderer::new();
            fresh.threads = 2;
            assert!(panned.pixels == fresh.render(&d, size.0, size.1, xf, &opts).pixels, "blur {blur}, pan {pan:?}");
        }
    }
}

/// The multithreaded rasterizer draws the same pixels with any number of workers, so the
/// automatic thread count can follow the machine.
#[test]
fn pixels_do_not_depend_on_the_thread_count() {
    let mut d = effects_page();
    // Overlapping translucent and blended objects on top of the effects.
    let l = d.layers[0].id;
    for i in 0..400 {
        let id = d.alloc_id();
        let (x, y) = ((i * 37 % 190) as f64, (i * 53 % 190) as f64);
        let mut n = Node::path(
            id,
            shapes::ellipse(Rect::new(x, y, x + 14.0, y + 9.0)),
            Appearance::basic(Paint::solid(Color::rgb(0.9, 0.3, 0.1)), Paint::solid(Color::BLACK), 0.7),
        );
        n.opacity = 0.6;
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut frames = vec![];
    // The test sets each renderer's count. `set_default_threads` would also change the count of the
    // renderers that other tests create while this one runs.
    for threads in [0, 1, 4, 8] {
        let mut r = Renderer::new();
        r.threads = threads;
        frames.push(r.render(&d, 1200, 900, Affine::scale(6.0), &opts).pixels);
    }
    assert!(frames.windows(2).all(|w| w[0] == w[1]), "the thread count changed pixels");
}

#[test]
fn thumbnails_of_glowing_objects_render_without_panicking() {
    // Regression: thumbnails used vello's default (multithreaded) context, which panics on filters.
    let d = doc_with(
        Rect::new(20.0, 20.0, 60.0, 60.0),
        Color::BLACK,
        vec![("stylize.outerGlow", json!({"color": "#ff0000", "opacity": 100, "blur": 6}))],
    );
    let id = d.layers[0].children().unwrap()[0].id;
    let t = Renderer::new().render_thumbnail(&d, id, 48).unwrap();
    assert_eq!((t.width, t.height), (48, 48));
}

#[test]
fn per_stroke_raster_effects_use_the_single_threaded_pipeline() {
    let mut d = doc_with(Rect::new(20.0, 20.0, 60.0, 60.0), Color::BLACK, vec![]);
    let l = d.layers[0].id;
    let id = d.layers[0].children().unwrap()[0].id;
    let mut n = (*d.node(id).unwrap()).clone();
    n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0).into();
    if let Some(vectorcraft_doc::AppearanceItem::Stroke(s)) =
        n.appearance.items.iter_mut().find(|i| matches!(i, vectorcraft_doc::AppearanceItem::Stroke(_)))
    {
        s.effects = vec![Effect { id: "stylize.outerGlow".into(), params: json!({"color": "#ff0000", "opacity": 100, "blur": 6}), visible: true }];
    }
    d.remove(id).unwrap();
    d.insert(Some(l), 0, n).unwrap();
    let mut r = Renderer::new();
    r.threads = 4;
    let _ = r.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
}

#[test]
fn drop_shadow_darkens_outside_the_shape() {
    let r = Rect::new(20.0, 20.0, 60.0, 60.0);
    let plain = render(&doc_with(r, Color::rgb(1.0, 0.0, 0.0), vec![]));
    assert_eq!(lum(plain.pixel(66, 66)), WHITE);
    let d = doc_with(r, Color::rgb(1.0, 0.0, 0.0), vec![("stylize.dropShadow", json!({"x": 10, "y": 10, "blur": 2, "opacity": 100}))]);
    let img = render(&d);
    assert!(lum(img.pixel(66, 66)) < 200, "shadow below-right: {:?}", img.pixel(66, 66));
    // The object stays on top and unshadowed; the far side has no shadow.
    assert_eq!(img.pixel(40, 40), [255, 0, 0, 255]);
    assert_eq!(lum(img.pixel(15, 15)), WHITE);
}

/// A red square's drop shadow at Darkness `darkness` (%), opaque and sharp, 10 pt down and
/// right; `shade` "color" or "darkness".
fn shaded_doc(mode: vectorcraft_doc::ColorMode, fill: Color, shade: &str, darkness: f64) -> Document {
    let mut d = doc_with(
        Rect::new(20.0, 20.0, 60.0, 60.0),
        fill,
        vec![("stylize.dropShadow", json!({"x": 10, "y": 10, "blur": 0, "opacity": 100, "mode": "normal", "shade": shade, "darkness": darkness}))],
    );
    d.color_mode = mode;
    d
}

/// Darkness: the shadow is the object's own color, with darkness subtracted from each RGB channel in an RGB
/// document (the reference app's guide gives the object's color at 0%,
/// <https://help.adobe.com/archive/en/illustrator/cc/2014/illustrator_reference.pdf> page 428), in the cached
/// raster and, at 6000 %, in the filter layer that a raster too large to cache is drawn with.
#[test]
fn drop_shadow_darkness_darkens_the_objects_own_color() {
    let rgb = vectorcraft_doc::ColorMode::Rgb;
    let red = Color::rgb(1.0, 0.0, 0.0);
    // At 6000 %, the view's center is (65, 65), where only the shadow lies.
    let zoomed = Affine::translate((50.0, 50.0)) * Affine::scale(60.0) * Affine::translate((-65.0, -65.0));
    for (darkness, want) in [(0.0, 255), (50.0, 128), (100.0, 0)] {
        let d = shaded_doc(rgb, red, "darkness", darkness);
        let px = render(&d).pixel(66, 66);
        assert!(px[0].abs_diff(want) <= 1 && px[1] == 0 && px[2] == 0, "{darkness}%: {px:?}");
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let px = Renderer::new().render(&d, 100, 100, zoomed, &opts).pixel(50, 50);
        assert!(px[0].abs_diff(want) <= 1 && px[1] == 0 && px[2] == 0, "{darkness}% at 6000 %: {px:?}");
    }
}

/// Intermediate Darkness adds black ink while preserving C/M/Y. The top endpoint becomes
/// gray black before ink conversion; its output below uses this application's gray-color model.
#[test]
fn drop_shadow_darkness_adds_black_in_a_cmyk_document() {
    let cmyk = vectorcraft_doc::ColorMode::Cmyk;
    let inks = |d: &Document| {
        let i = Renderer::new().render_region_inks(d, Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, &RenderOptions::default());
        let at = (66 * 100 + 66) * 4;
        [i[at], i[at + 1], i[at + 2], i[at + 3]]
    };
    let red = Color::cmyk(0.0, 1.0, 1.0, 0.0);
    for (darkness, k) in [(0.0, 0), (50.0, 128), (100.0, 255)] {
        let got = inks(&shaded_doc(cmyk, red, "darkness", darkness));
        let my = if darkness == 100.0 { 0 } else { 255 };
        assert!(got[0] == 0 && got[1] == my && got[2] == my && got[3].abs_diff(k) <= 1, "{darkness}%: {got:?}");
    }
    // Half black ink plus 50 percentage points clamps at full black.
    let got = inks(&shaded_doc(cmyk, Color::cmyk(0.0, 0.0, 0.0, 0.5), "darkness", 50.0));
    assert!(got[..3] == [0, 0, 0] && got[3].abs_diff(255) <= 1, "{got:?}");
    let got = inks(&shaded_doc(cmyk, Color::cmyk(0.0, 0.0, 0.0, 1.0), "darkness", 100.0));
    assert_eq!(got, [0, 0, 0, 255]);
}

/// The top endpoint is gray black before document/profile conversion, independently of source model.
#[test]
fn darkness_black_endpoint_and_gray_source_follow_their_stored_models() {
    for mode in [vectorcraft_doc::ColorMode::Rgb, vectorcraft_doc::ColorMode::Cmyk] {
        for source in [Color::rgb(0.8, 0.4, 0.2), Color::cmyk(0.1, 0.3, 0.4, 0.2), Color::gray(0.2)] {
            let got = render(&shaded_doc(mode, source, "darkness", 100.0)).pixel(66, 66);
            let want = render(&shaded_doc(mode, Color::gray(1.0), "darkness", 0.0)).pixel(66, 66);
            assert_eq!(got, want, "{mode:?}, {source:?}");
        }
        let got = render(&shaded_doc(mode, Color::gray(0.2), "darkness", 25.0)).pixel(66, 66);
        let want = render(&shaded_doc(mode, Color::gray(0.45), "darkness", 0.0)).pixel(66, 66);
        assert_eq!(got, want, "gray intermediate in {mode:?}");
    }
}

/// Color stays the default: a shadow without the new keys, and one with Color chosen and a
/// Darkness value, draw the same pixels, which a shadow with Darkness does not.
#[test]
fn a_drop_shadow_with_color_draws_as_before() {
    let rgb = vectorcraft_doc::ColorMode::Rgb;
    let red = Color::rgb(1.0, 0.0, 0.0);
    let old = doc_with(
        Rect::new(20.0, 20.0, 60.0, 60.0),
        red,
        vec![("stylize.dropShadow", json!({"x": 10, "y": 10, "blur": 0, "opacity": 100, "mode": "normal"}))],
    );
    let before = render(&old).pixels;
    assert_eq!(render(&shaded_doc(rgb, red, "color", 40.0)).pixels, before);
    assert_ne!(render(&shaded_doc(rgb, red, "darkness", 40.0)).pixels, before);
}

#[test]
fn shadow_of_offscreen_object_is_not_culled() {
    let d =
        doc_with(Rect::new(-60.0, 20.0, -5.0, 60.0), Color::WHITE, vec![("stylize.dropShadow", json!({"x": 30, "y": 0, "blur": 0, "opacity": 100}))]);
    let img = render(&d);
    let row: Vec<u32> = (0..10).map(|i| lum(img.pixel(i * 10, 40))).collect();
    assert!(lum(img.pixel(10, 40)) < 100, "{row:?}");
}

#[test]
fn geometry_effect_is_rendered() {
    let r = Rect::new(30.0, 30.0, 70.0, 70.0);
    let d = doc_with(r, Color::BLACK, vec![("path.offsetPath", json!({"offset": 10}))]);
    let img = render(&d);
    assert_eq!(lum(img.pixel(25, 50)), 0, "offset grows the fill");
    let d = doc_with(r, Color::BLACK, vec![("distort.transform", json!({"moveH": -25}))]);
    let img = render(&d);
    assert_eq!(lum(img.pixel(10, 50)), 0);
    assert_eq!(lum(img.pixel(60, 50)), WHITE);
}

#[test]
fn per_fill_effects_apply_to_that_fill_only() {
    let mut d = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::BLACK, vec![]);
    let id = d.layers[0].children().unwrap()[0].id;
    let n = d.node_mut(id).unwrap();
    n.appearance.fill_mut().unwrap().effects.push(Effect { id: "distort.transform".into(), params: json!({"moveH": 20}), visible: true });
    let img = render(&d);
    assert_eq!(lum(img.pixel(85, 50)), 0);
    assert_eq!(lum(img.pixel(40, 50)), WHITE);
}

#[test]
fn blur_glow_and_feather_soften() {
    let r = Rect::new(30.0, 30.0, 70.0, 70.0);
    let blur = render(&doc_with(r, Color::BLACK, vec![("blur.gaussian", json!({"radius": 6}))]));
    let edge = lum(blur.pixel(29, 50));
    assert!(edge > 0 && edge < WHITE, "blur spreads over the edge: {edge}");
    let glow =
        render(&doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]));
    let g = glow.pixel(27, 50);
    assert!(g[0] > g[1] + 20, "outer glow is red: {g:?}");
    let inner =
        render(&doc_with(r, Color::BLACK, vec![("stylize.innerGlow", json!({"color": "#ffffff", "mode": "normal", "opacity": 100, "blur": 4}))]));
    assert!(lum(inner.pixel(31, 50)) > 60, "inner glow lightens the edge: {:?}", inner.pixel(31, 50));
    assert!(lum(inner.pixel(50, 50)) < 30, "centre stays dark");
    let feather = render(&doc_with(r, Color::BLACK, vec![("stylize.feather", json!({"radius": 8}))]));
    assert!(lum(feather.pixel(31, 50)) > 20, "feather fades the edge inward");
    assert_eq!(lum(feather.pixel(26, 50)), WHITE, "feather stays inside");
}

#[test]
fn an_outer_glow_blurs_a_solid_silhouette_of_faint_art() {
    // GUARDS: the glow's strength comes from where the art is, not how opaque it is (a glow round a glow relies on it).
    let r = Rect::new(30.0, 30.0, 70.0, 70.0);
    let glow = |fill_opacity: f32| {
        let mut d = doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]);
        let id = d.layers[0].children().unwrap()[0].id;
        d.node_mut(id).unwrap().appearance.fill_mut().unwrap().opacity = fill_opacity;
        render(&d).pixel(29, 50)
    };
    let (faint, opaque) = (glow(0.2), glow(1.0));
    assert_eq!(faint[1], opaque[1], "faint {faint:?}, opaque {opaque:?}");
    assert!(faint[1] < 210, "the glow reaches past the edge: {faint:?}");
}

#[test]
fn an_outer_glow_grows_its_silhouette_by_a_quarter_raster_pixel() {
    // GUARDS: the glows AI files export measure as the solid silhouette grown by a quarter of a pixel
    // of the document's raster grid, then blurred: a quarter point at 72 ppi, next to nothing at 720.
    let r = Rect::new(30.0, 30.0, 70.0, 70.0);
    let green_outside = |ppi: f64| {
        let mut d = doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]);
        d.raster_effects_ppi = ppi;
        render(&d).pixel(27, 50)[1]
    };
    let (coarse, fine) = (green_outside(72.0), green_outside(720.0));
    assert!(coarse + 3 < fine, "the red glow is stronger at 72 ppi: 72 ppi {coarse}, 720 ppi {fine}");
}

#[test]
fn a_glow_glows_round_the_glows_before_it_in_the_stack() {
    // GUARDS: the second glow is of the art with the first glow, not of the bare art, as AI files and the SVG export draw it.
    let r = Rect::new(30.0, 30.0, 70.0, 70.0);
    let d = doc_with(
        r,
        Color::BLACK,
        vec![
            ("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 8})),
            ("stylize.outerGlow", json!({"color": "#0000ff", "mode": "normal", "opacity": 100, "blur": 2})),
        ],
    );
    let p = render(&d).pixel(22, 50);
    assert!(p[2] > p[0] + 100, "8 pt out, the tight blue glow round the red one shows: {p:?}");
}

#[test]
fn a_glow_too_large_for_a_full_size_raster_keeps_its_solid_silhouette() {
    // GUARDS: at 40× the glow's raster passes the 4 Mpx budget and is made coarser instead of falling back to the
    // filter layer, which blurs the faint alpha.
    let r = Rect::new(30.0, 30.0, 70.0, 70.0);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let view = Affine::translate((-1100.0, -1900.0)) * Affine::scale(40.0);
    let glow = |fill_opacity: f32| {
        let mut d = doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]);
        let id = d.layers[0].children().unwrap()[0].id;
        d.node_mut(id).unwrap().appearance.fill_mut().unwrap().opacity = fill_opacity;
        Renderer::new().render(&d, 200, 200, view, &opts).pixel(60, 100)
    };
    let (faint, opaque) = (glow(0.2), glow(1.0));
    assert!(faint[1].abs_diff(opaque[1]) <= 2, "faint {faint:?}, opaque {opaque:?}");
    assert!(faint[1] < 210, "the glow reaches past the edge: {faint:?}");
}

#[test]
fn thirty_stacked_glows_draw_thirty_silhouettes_cached_or_not() {
    // GUARDS: each glow is of the content with the glows before it; drawing those again for every glow costs 2^n.
    // A shape draws through the shadow cache, a symbol instance without it (the filter layers).
    // Each glow's silhouette is drawn once.
    let glows: Vec<(&str, serde_json::Value)> =
        (0..30).map(|k| ("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 20, "blur": 1 + k % 4}))).collect();
    let shape = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::BLACK, glows.clone());
    let mut instanced = Document::new(100.0, 100.0);
    let art = Node::path(
        instanced.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 40.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    instanced.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Square".into(), art: std::sync::Arc::new(art) });
    let mut instance =
        Node::new(instanced.alloc_id(), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Square".into(), xf: Affine::translate((30.0, 30.0)) });
    instance.appearance.effects = glows.into_iter().map(|(id, p)| Effect { id: id.into(), params: p, visible: true }).collect();
    let l = instanced.layers[0].id;
    instanced.insert(Some(l), 0, instance).unwrap();
    for d in [&shape, &instanced] {
        let before = super::renderer::silhouettes();
        let p = render(d).pixel(25, 50);
        let drawn = super::renderer::silhouettes() - before;
        assert_eq!(drawn, 30, "silhouettes drawn for 30 glows");
        assert!(p[1] < 250, "the glows show: {p:?}");
    }
}

#[test]
fn hidden_effects_are_ignored_and_outline_mode_uses_effected_geometry() {
    let r = Rect::new(30.0, 30.0, 70.0, 70.0);
    let mut d = doc_with(r, Color::BLACK, vec![("distort.transform", json!({"moveH": -25}))]);
    let id = d.layers[0].children().unwrap()[0].id;
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), outline: true, ..Default::default() };
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
    assert!(lum(img.pixel(5, 50)) < 400, "outline follows the moved path");
    d.node_mut(id).unwrap().appearance.effects[0].visible = false;
    let img = render(&d);
    assert_eq!(lum(img.pixel(50, 50)), 0);
    assert_eq!(lum(img.pixel(10, 50)), WHITE);
}

#[test]
fn visual_bounds_include_shadow() {
    let d = doc_with(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK, vec![("stylize.dropShadow", json!({"x": 7, "y": 7, "blur": 5}))]);
    let n = &d.layers[0].children().unwrap()[0];
    let b = super::visual_bounds(&d.symbol_boxes(), n).unwrap();
    assert!(b.x1 >= 10.0 + 7.0 + 7.5 - 1e-9);
}

/// A shadow after a 3D effect reaches as far past the solid as it says, once: the solid's art keeps
/// the raster effects that follow the 3D effect, and the object paints them over that art.
#[test]
fn a_shadow_after_a_3d_effect_reaches_past_the_solid_once() {
    let r = Rect::new(40.0, 40.0, 60.0, 60.0);
    let extrude = ("threeD.extrude", json!({"depth": 20.0, "position": "isometricTop"}));
    let shadow = ("stylize.dropShadow", json!({"x": 20, "y": 0, "blur": 0}));
    let measure = |d: &Document| super::cull_bounds(&d.symbol_boxes(), &d.layers[0].children().unwrap()[0]).unwrap();
    let solid = measure(&doc_with(r, Color::BLACK, vec![extrude.clone()]));
    let shadowed = measure(&doc_with(r, Color::BLACK, vec![extrude, shadow]));
    assert_eq!(shadowed.union(solid), shadowed);
    let grown = [solid.x0 - shadowed.x0, solid.y0 - shadowed.y0, shadowed.x1 - solid.x1, shadowed.y1 - solid.y1];
    assert!(grown.iter().all(|g| *g <= 20.0 + 1e-6) && grown.iter().any(|g| *g >= 20.0 - 1e-6), "{solid:?} {shadowed:?}");
}

/// Revolve builds no art from type: the type draws its glyphs, and its bounds hold them.
#[test]
fn revolved_type_is_measured_by_the_glyphs_it_draws() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let _fonts = fonts_steady();
    let mut d = Document::new(100.0, 100.0);
    let style = CharStyle { size: 30.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "KW", style))));
    n.appearance.effects = vec![Effect { id: "threeD.revolve".into(), params: json!({}), visible: true }];
    let l = d.layers[0].id;
    let id = d.insert(Some(l), 0, n).unwrap();
    let (img, b) = with_fonts_unchanged(|| (render(&d), super::cull_bounds(&d.symbol_boxes(), d.node(id).unwrap()).unwrap()));
    let inked: Vec<(u32, u32)> = (0..100).flat_map(|y| (0..100).map(move |x| (x, y))).filter(|&(x, y)| img.pixel(x, y)[0] < 250).collect();
    assert!(!inked.is_empty());
    assert!(inked.iter().all(|&(x, y)| b.inflate(1.0, 1.0).contains(vectorcraft_geom::Point::new(f64::from(x), f64::from(y)))), "{b:?}");
}

/// An export crops to the solid a 3D effect draws on type, not to the letters, and so does
/// Object › Rasterize.
#[test]
fn an_export_and_rasterize_hold_the_solid_of_3d_type() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let _fonts = fonts_steady();
    let mut d = Document::new(100.0, 100.0);
    let style = CharStyle { size: 30.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), "K", style))));
    let letters = with_fonts_unchanged(|| crate::encode::art_bounds(&doc_of(&mut d.clone(), n.clone())).unwrap());
    n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 60.0, "position": "isometricTop"}), visible: true }];
    let solid_doc = doc_of(&mut d, n.clone());
    let (solid, rasterized) = with_fonts_unchanged(|| {
        let n = &solid_doc.layers[0].children().unwrap()[0];
        (crate::encode::art_bounds(&solid_doc).unwrap(), crate::drawn_bounds(&solid_doc, n).unwrap())
    });
    assert!(solid.width() > letters.width() + 15.0, "{letters:?} {solid:?}");
    assert!(rasterized.width() > letters.width() + 15.0, "{letters:?} {rasterized:?}");
}

fn doc_of(d: &mut Document, n: Node) -> Document {
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d.clone()
}

/// A shadow on a fill after its own 3D effect is drawn on the fill's solid and reaches past it
/// once: Object › Rasterize takes the solid and the shadow, no more.
#[test]
fn a_fills_shadow_after_its_3d_effect_reaches_past_its_solid_once() {
    let mut d = doc_with(Rect::new(40.0, 40.0, 60.0, 60.0), Color::BLACK, vec![]);
    let id = d.layers[0].children().unwrap()[0].id;
    let fill = |shadow: bool| {
        let mut fx = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0, "position": "isometricTop"}), visible: true }];
        if shadow {
            fx.push(Effect { id: "stylize.dropShadow".into(), params: json!({"x": 20, "y": 0, "blur": 0}), visible: true });
        }
        fx
    };
    d.node_mut(id).unwrap().appearance.fill_mut().unwrap().effects = fill(false);
    let solid = crate::drawn_bounds(&d, d.node(id).unwrap()).unwrap();
    d.node_mut(id).unwrap().appearance.fill_mut().unwrap().effects = fill(true);
    let shadowed = crate::drawn_bounds(&d, d.node(id).unwrap()).unwrap();
    let grown = [solid.x0 - shadowed.x0, solid.y0 - shadowed.y0, shadowed.x1 - solid.x1, shadowed.y1 - solid.y1];
    assert!(grown.iter().all(|g| *g <= 20.0 + 1e-6) && grown.iter().any(|g| *g >= 20.0 - 1e-6), "{solid:?} {shadowed:?}");
}

/// A fill's own 3D effect leaves the brushed stroke drawn with its brush: Object › Rasterize, exports
/// and culling take the brush's art in, and Rasterize and exports no more than the solid and that
/// art.
#[test]
fn a_brushed_stroke_beside_a_fills_3d_effect_is_measured_with_its_brush() {
    let mut d = doc_with(Rect::new(40.0, 40.0, 60.0, 60.0), Color::BLACK, vec![]);
    let id = d.layers[0].children().unwrap()[0].id;
    {
        let n = d.node_mut(id).unwrap();
        n.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 1.0)));
        if let Some(vectorcraft_doc::AppearanceItem::Stroke(s)) = n.appearance.items.last_mut() {
            s.brush = Some("10 pt. Oval".into());
        }
        n.appearance.fill_mut().unwrap().effects =
            vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0, "position": "isometricTop"}), visible: true }];
    }
    let n = d.node(id).unwrap();
    let brush = vectorcraft_brush::expand(&d, n).and_then(|art| art.visual_bounds()).unwrap();
    let symbols = d.symbol_boxes();
    let rasterized = crate::drawn_bounds(&d, n).unwrap();
    let exported = crate::effect_bounds(&symbols, n).unwrap();
    for (what, b) in [("Rasterize", rasterized), ("exports", exported), ("culling", super::cull_bounds(&symbols, n).unwrap())] {
        assert_eq!(b.union(brush), b, "{what}: {b:?} holds the brush's art {brush:?}");
    }
    // The solid alone, without the stroke: what else may be measured.
    let mut plain = d.clone();
    plain.node_mut(id).unwrap().appearance.items.retain(|i| !matches!(i, vectorcraft_doc::AppearanceItem::Stroke(_)));
    let solid = crate::drawn_bounds(&plain, plain.node(id).unwrap()).unwrap();
    let most = solid.union(brush).inflate(1.0, 1.0);
    for (what, b) in [("Rasterize", rasterized), ("exports", exported)] {
        assert_eq!(most.union(b), most, "{what}: {b:?} no more than the solid and the brush's art {most:?}");
    }
}

/// The renderer measures a 3D object with Crop Marks as the free measure does: by the marks and
/// the object, which paints its own shadow inside them, counted once.
#[test]
fn the_renderer_measures_3d_art_with_crop_marks_as_drawn() {
    let fx = vec![
        ("threeD.extrude", json!({"depth": 30.0, "position": "isometricTop"})),
        ("stylize.dropShadow", json!({"x": 20, "y": 0, "blur": 0})),
        (crate::effects::CROP_MARKS, json!({})),
    ];
    let d = doc_with(Rect::new(40.0, 40.0, 60.0, 60.0), Color::BLACK, fx);
    let n = d.layers[0].children().unwrap()[0].clone();
    let symbols = d.symbol_boxes();
    let free = super::cull_bounds(&symbols, &n).unwrap();
    assert_eq!(Renderer::new().bounds_of(&symbols, &n), Some(free));
}

/// Vector Halftone draws dots in place of a 3D effect's solid: the renderer doesn't measure the
/// object by art it holds to draw it with, and measures it as the free measure does.
#[test]
fn the_renderer_measures_3d_art_with_vector_halftone_as_drawn() {
    let fx = vec![("threeD.extrude", json!({"depth": 30.0, "position": "isometricTop"})), (crate::effects::addons::halftone::HALFTONE, json!({}))];
    let d = doc_with(Rect::new(40.0, 40.0, 60.0, 60.0), Color::BLACK, fx);
    let n = d.layers[0].children().unwrap()[0].clone();
    let symbols = d.symbol_boxes();
    let free = super::cull_bounds(&symbols, &n).unwrap();
    let mut r = Renderer::new();
    // A consistency check: measured from the dots' art, the bounds come out the same.
    assert_eq!(r.bounds_of(&symbols, &n), Some(free));
    assert_eq!(r.measured_bytes(), 0, "no art held");
}

/// A renderer measures the art over each artboard as [`crate::encode::art_bounds_on`] does, and
/// builds each 3D solid once for every artboard it measures.
#[test]
fn a_renderer_measures_the_art_on_each_artboard_building_each_solid_once() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let boards = 30;
    for i in 0..boards {
        let x = f64::from(i) * 100.0;
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x + 30.0, 30.0, x + 60.0, 60.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects =
            vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 40.0, "position": "isometricTop"}), visible: true }];
        d.insert(Some(l), 0, n).unwrap();
    }
    let mut r = Renderer::new();
    for i in 0..boards {
        let rect = Rect::new(f64::from(i) * 100.0, 0.0, f64::from(i) * 100.0 + 100.0, 100.0);
        assert_eq!(r.art_bounds_on(&d, rect), crate::encode::art_bounds_on(&d, rect), "artboard {i}");
    }
    assert_eq!(r.stats.three_d_built, boards as usize, "each solid built once");
}

/// A 3D object measured but not drawn (off the view) doesn't keep its art past the frame: what is
/// kept is what is drawn.
#[test]
fn only_drawn_3d_art_is_kept() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let mut ids = vec![];
    for x in [30.0, 530.0] {
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x, 30.0, x + 30.0, 60.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
        ids.push(d.insert(Some(l), 0, n).unwrap());
    }
    let mut r = Renderer::new();
    r.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    let kept = |r: &Renderer, id| {
        let n = d.layers[0].children().unwrap().iter().find(|c| c.id == id).unwrap();
        r.fx_arts.contains_key(&(Arc::as_ptr(n) as usize))
    };
    assert!(kept(&r, ids[0]), "drawn, kept");
    assert!(!kept(&r, ids[1]), "off the view, let go");
    assert!(r.measured.is_empty());
}

/// Art made to measure objects is held within a budget: past it, objects are measured and their
/// art is made again if they are drawn.
#[test]
fn measured_art_is_held_within_its_budget() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    for i in 0..20 {
        let x = 500.0 + f64::from(i) * 40.0;
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x, 30.0, x + 30.0, 60.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
        d.insert(Some(l), 0, n).unwrap();
    }
    let symbols = d.symbol_boxes();
    let mut r = Renderer::new();
    let one = {
        let n = &d.layers[0].children().unwrap()[0];
        r.bounds_of(&symbols, n);
        r.measured_bytes
    };
    r.let_go_of_measured();
    r.measured_budget = one * 3;
    for n in d.layers[0].children().unwrap() {
        r.bounds_of(&symbols, n);
    }
    assert!(r.measured_bytes <= one * 3 && r.measured.len() <= 3, "{} bytes in {} entries", r.measured_bytes, r.measured.len());
    // Every object is measured right, those past the budget too, and draws as a new renderer
    // draws it, its art made again.
    for n in d.layers[0].children().unwrap() {
        assert_eq!(r.bounds_of(&symbols, n), super::cull_bounds(&symbols, n), "{:?}", n.id);
    }
    let (view, opts) = (Affine::translate((-500.0, 0.0)), RenderOptions::default());
    let drawn = r.render(&d, 800, 100, view, &opts).pixels;
    assert!(drawn == Renderer::new().render(&d, 800, 100, view, &opts).pixels);
}

/// An edit to 3D objects the frame doesn't draw (hidden, in a hidden group, drawn hidden) builds
/// nothing to measure them, and changes nothing.
#[test]
fn an_edit_out_of_sight_builds_nothing() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let extruded = |d: &mut Document, x: f64| {
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x, 30.0, x + 20.0, 50.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
        n
    };
    let mut hidden = extruded(&mut d, 10.0);
    hidden.visible = false;
    let member = extruded(&mut d, 40.0);
    let mut group = Node::group(d.alloc_id(), vec![Arc::new(member)]);
    group.visible = false;
    let drawn_hidden = extruded(&mut d, 70.0);
    let ids = [d.insert(Some(l), 0, hidden).unwrap(), group.children().unwrap()[0].id, d.insert(Some(l), 0, drawn_hidden).unwrap()];
    d.insert(Some(l), 0, group).unwrap();
    let opts = RenderOptions { hidden: vec![ids[2]], ..Default::default() };
    let mut r = Renderer::new();
    r.render(&d, 100, 100, Affine::IDENTITY, &opts);
    let mut e = d.clone();
    for id in ids {
        e.node_mut(id).unwrap().appearance.effects[0].params = json!({"depth": 5.0});
    }
    let built = r.stats.three_d_built;
    assert_eq!(r.changed(&d, &e, &opts), crate::Changed::Nothing);
    assert_eq!((r.stats.three_d_built - built, r.measured.len()), (0, 0));
}

/// The bounds of 3D art kept for a pass that only measures stay as many as the art still held.
#[test]
fn a_measuring_pass_keeps_the_bounds_of_held_art_only() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    for i in 0..3000 {
        let x = f64::from(i) * 30.0;
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x, 30.0, x + 20.0, 50.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 2.0}), visible: true }];
        d.insert(Some(l), 0, n).unwrap();
    }
    let symbols = d.symbol_boxes();
    let mut r = Renderer::new();
    // Nothing measured is held: the art is let go once measured.
    r.measured_budget = 0;
    for n in d.layers[0].children().unwrap() {
        r.bounds_of(&symbols, n);
    }
    assert_eq!(r.stats.three_d_built, 3000);
    assert!(r.three_d_bounds.len() <= 1024, "{} kept", r.three_d_bounds.len());
}

/// While its 3D art is held, a pass that only measures sweeps the bounds kept for 3D art each time
/// they double, not on every new art past a size.
#[test]
fn a_measuring_pass_sweeps_the_bounds_of_held_art_as_they_double() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    for i in 0..3000 {
        let x = f64::from(i) * 30.0;
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x, 30.0, x + 20.0, 50.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 2.0}), visible: true }];
        d.insert(Some(l), 0, n).unwrap();
    }
    let mut r = Renderer::new();
    // Held here, as the art kept to draw is.
    let held: Vec<_> = d.layers[0].children().unwrap().iter().map(|n| r.measured_fx_art(&d, n)).collect();
    assert_eq!((held.len(), r.three_d_bounds.len()), (3000, 3000));
    assert!(r.three_d_sweeps <= 2, "{} sweeps", r.three_d_sweeps);
}

/// A renderer checks the brush library between frames only once it has measured 3D art, and again
/// only after measuring 3D art since the library last changed.
#[test]
fn the_brush_library_is_checked_only_after_3d_art_is_measured() {
    let mut d = doc_with(Rect::new(30.0, 30.0, 60.0, 60.0), Color::BLACK, vec![]);
    let mut r = Renderer::new();
    r.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(r.brush_library, None);
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
    r.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert!(r.brush_library.is_some());
    // A library changed: the bounds measured with the old one go, and so does its key.
    let mut lib = vectorcraft_brush::library(&d);
    lib.retain(|b| b.name != "3 pt. Round");
    vectorcraft_brush::store(&mut d, &lib);
    r.check_symbols(&d);
    assert_eq!(r.brush_library, None);
}

/// An object whose 3D effect builds nothing (Revolve on a group) is measured by the renderer as the
/// free measure does it: as the renderer draws it, by its members.
#[test]
fn a_3d_effect_that_builds_nothing_is_measured_alike() {
    let mut d = Document::new(100.0, 100.0);
    let mut hidden = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(60.0, 10.0, 90.0, 20.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    hidden.visible = false;
    let shown = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    let mut g = Node::group(d.alloc_id(), vec![Arc::new(hidden), Arc::new(shown)]);
    g.appearance.effects = vec![Effect { id: "threeD.revolve".into(), params: json!({}), visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let n = d.layers[0].children().unwrap()[0].clone();
    let symbols = d.symbol_boxes();
    assert_eq!(Renderer::new().effect_bounds(&symbols, &n), crate::effect_bounds(&symbols, &n));
    assert_eq!(Renderer::new().bounds_of(&symbols, &n), crate::brush_fx::cull_bounds(&symbols, &n));
}

/// A group with effects of its own that holds an extruded path is measured once for every artboard
/// of a pass: its bounds are kept after the first.
#[test]
fn a_group_holding_3d_art_is_measured_once_a_pass() {
    let mut d = Document::new(100.0, 100.0);
    let mut inner = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(30.0, 30.0, 60.0, 60.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    inner.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
    let mut g = Node::group(d.alloc_id(), vec![Arc::new(inner)]);
    g.appearance.effects = vec![Effect { id: "stylize.dropShadow".into(), params: json!({"x": 4, "y": 4, "blur": 2}), visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let g = d.layers[0].children().unwrap()[0].clone();
    let mut r = Renderer::new();
    let first = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert_eq!(r.art_bounds_on(&d, first), crate::encode::art_bounds_on(&d, first));
    let key = Arc::as_ptr(&g) as usize;
    assert!(r.geom.contains_key(&key), "kept for the next artboard");
    // The group's bounds, planted over the next artboards: they read the kept bounds, measuring
    // nothing again.
    let planted = Rect::new(100.0, 0.0, 300.0, 100.0);
    r.geom.get_mut(&key).unwrap().bounds = Some(planted);
    for i in 1..3 {
        let rect = Rect::new(f64::from(i) * 100.0, 0.0, f64::from(i) * 100.0 + 100.0, 100.0);
        assert_eq!(r.art_bounds_on(&d, rect), Some(planted), "artboard {i}");
    }
}

/// A pass that only measures (Fit to Artwork Bounds, an export) holds no 3D art past its budget:
/// what it measured and didn't keep is let go.
#[test]
fn a_measuring_pass_holds_no_art_past_its_budget() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    for i in 0..20 {
        let x = f64::from(i) * 40.0;
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x, 30.0, x + 30.0, 60.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
        d.insert(Some(l), 0, n).unwrap();
    }
    let mut r = Renderer::new();
    r.measured_budget = 0;
    r.art_bounds_on(&d, Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(r.measured.is_empty());
    assert!(r.three_d_bounds.values().all(|e| e.0.strong_count() == 0), "no art held");
}

/// Drawing an object takes the art made to measure it out of the budget: a frame draws each 3D
/// object with that art, making none again, while what it measured fits the budget.
#[test]
fn drawing_takes_measured_art_out_of_the_budget() {
    let mut d = Document::new(800.0, 100.0);
    let l = d.layers[0].id;
    for i in 0..20 {
        let x = f64::from(i) * 40.0;
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(x, 30.0, x + 30.0, 60.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
        d.insert(Some(l), 0, n).unwrap();
    }
    let n = d.layers[0].children().unwrap()[0].clone();
    let mut r = Renderer::new();
    r.bounds_of(&d.symbol_boxes(), &n);
    assert!(r.measured_bytes > 0);
    r.fx_art(&d, &n, true);
    assert_eq!((r.measured_bytes, r.measured.len()), (0, 0), "taken out when drawn");
    let mut r = Renderer::new();
    r.render(&d, 800, 100, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(r.stats.three_d_built, 20, "each made once");
}

/// An object with a brushed stroke whose 3D effect builds nothing is measured by a pass as without
/// it: not with the room culling leaves for its brush.
#[test]
fn a_pass_measures_brushed_art_as_exports_do() {
    let mut d = Document::new(100.0, 100.0);
    let mut art = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    art.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 1.0)));
    if let Some(vectorcraft_doc::AppearanceItem::Stroke(s)) = art.appearance.items.last_mut() {
        s.brush = Some("10 pt. Oval".into());
    }
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Brushed".into(), art: Arc::new(art) });
    let mut n = Node::new(d.alloc_id(), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Brushed".into(), xf: Affine::translate((50.0, 50.0)) });
    n.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let n = d.layers[0].children().unwrap()[0].clone();
    let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert_eq!(Renderer::new().art_bounds_on(&d, rect), crate::encode::art_bounds_on(&d, rect));
    let symbols = d.symbol_boxes().with_brush_art();
    assert_eq!(Renderer::new().effect_bounds(&symbols, &n), crate::effect_bounds(&symbols, &n));
}

/// A 3D object whose brushed stroke's brush gets wider is drawn where the brush now reaches, by a
/// renderer that measured it before: culling leaves room for brush art.
#[test]
fn a_3d_objects_wider_brush_is_drawn_where_it_reaches() {
    let mut d = Document::new(100.0, 100.0);
    let mut ap = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 1.0);
    ap.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    let mut n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(104.0, 40.0, 124.0, 60.0)), ap);
    n.appearance.fill_mut().unwrap().effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 2.0}), visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    r.render(&d, 100, 100, Affine::IDENTITY, &opts);
    let mut lib = vectorcraft_brush::library(&d);
    let round = lib.iter_mut().find(|b| b.name == "3 pt. Round").unwrap();
    if let vectorcraft_brush::BrushKind::Calligraphic(c) = &mut round.kind {
        c.size = 30.0;
    }
    vectorcraft_brush::store(&mut d, &lib);
    let drawn = r.render(&d, 100, 100, Affine::IDENTITY, &opts).pixels;
    let fresh = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts).pixels;
    assert!(fresh.iter().any(|c| *c < 128), "the wider brush reaches into the view");
    assert!(drawn == fresh, "as a new renderer draws it");
}

/// A 20 pt square at `x` with a fill-level 3D effect, a brushed stroke and Rasterize, in a 100 pt
/// view: drawn by a renderer that drew it before its brush got wider (`size` pt), and by a new
/// renderer.
fn drawn_after_a_brush_widens(x: f64, size: f64) -> (Vec<u8>, Vec<u8>) {
    let brush = "3 pt. Round";
    let mut d = Document::new(100.0, 100.0);
    let mut ap = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 1.0);
    ap.stroke_mut().unwrap().brush = Some(brush.into());
    let mut n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 40.0, x + 20.0, 60.0)), ap);
    n.appearance.fill_mut().unwrap().effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 2.0}), visible: true }];
    n.appearance.effects = vec![Effect { id: "rasterize".into(), params: json!({"useDocumentResolution": false, "resolution": 72}), visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    r.render(&d, 100, 100, Affine::IDENTITY, &opts);
    let mut lib = vectorcraft_brush::library(&d);
    let b = lib.iter_mut().find(|b| b.name == brush).unwrap();
    if let vectorcraft_brush::BrushKind::Calligraphic(c) = &mut b.kind {
        c.size = size;
    }
    vectorcraft_brush::store(&mut d, &lib);
    let drawn = r.render(&d, 100, 100, Affine::IDENTITY, &opts).pixels;
    (drawn, Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts).pixels)
}

/// Rasterize on a brushed 3D object outside the view draws its art with the brush as it is now, by
/// a renderer that drew it with the brush before: the object is measured again when the brush
/// library changes, with the brush's art reaching past the room culling leaves for it, and
/// rasterized again.
#[test]
fn a_rasterized_3d_objects_brush_widened_is_drawn() {
    let (drawn, fresh) = drawn_after_a_brush_widens(140.0, 120.0);
    assert!(fresh.iter().any(|c| *c < 128), "the wider brush reaches into the view");
    assert!(drawn == fresh, "as a new renderer draws it");
}

/// A rasterized brushed 3D object in the view is rasterized again when its brush changes.
#[test]
fn a_rasterized_objects_brush_widened_in_view_is_drawn_again() {
    let (drawn, fresh) = drawn_after_a_brush_widens(40.0, 12.0);
    let (_, before) = drawn_after_a_brush_widens(40.0, 3.0);
    assert!(fresh != before, "the wider brush draws otherwise");
    assert!(drawn == fresh, "as a new renderer draws it");
}

/// A group with effects of its own that holds a hidden member is measured without it once a pass,
/// as exports measure it.
#[test]
fn a_group_with_a_hidden_member_is_measured_once_a_pass() {
    let mut d = Document::new(100.0, 100.0);
    let mut solid = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(30.0, 30.0, 60.0, 60.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    solid.appearance.effects = vec![Effect { id: "threeD.extrude".into(), params: json!({"depth": 20.0}), visible: true }];
    let mut hidden = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(80.0, 80.0, 95.0, 95.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    hidden.visible = false;
    let mut g = Node::group(d.alloc_id(), vec![Arc::new(solid), Arc::new(hidden)]);
    g.appearance.effects = vec![Effect { id: "stylize.dropShadow".into(), params: json!({"x": 4, "y": 4, "blur": 2}), visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let g = d.layers[0].children().unwrap()[0].clone();
    let mut r = Renderer::new();
    let first = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert_eq!(r.art_bounds_on(&d, first), crate::encode::art_bounds_on(&d, first));
    let key = Arc::as_ptr(&g) as usize;
    assert!(r.pass_bounds.contains_key(&key), "kept for the next artboard");
    // The group's bounds, planted over the next artboards: they read the kept bounds, measuring
    // nothing again.
    let planted = Rect::new(100.0, 0.0, 300.0, 100.0);
    r.pass_bounds.get_mut(&key).unwrap().1 = Some(planted);
    for i in 1..3 {
        let rect = Rect::new(f64::from(i) * 100.0, 0.0, f64::from(i) * 100.0 + 100.0, 100.0);
        assert_eq!(r.art_bounds_on(&d, rect), Some(planted), "artboard {i}");
    }
}

/// Art made again for an object, after what it read from outside it changed, takes the place of
/// the art held for it, and of its bytes.
#[test]
fn art_measured_again_takes_the_place_of_the_held() {
    let d = doc_with(Rect::new(30.0, 30.0, 60.0, 60.0), Color::BLACK, vec![("threeD.extrude", json!({"depth": 20.0}))]);
    let n = d.layers[0].children().unwrap()[0].clone();
    let key = Arc::as_ptr(&n) as usize;
    let mut r = Renderer::new();
    let art = r.measured_fx_art(&d, &n);
    let bytes = r.measured_bytes;
    // As if it had read something else then.
    r.measured.insert(key, (n.clone(), art, Some(7)));
    r.measured_fx_art(&d, &n);
    assert_eq!((r.measured_bytes, r.measured.len()), (bytes, 1));
}

/// Multithreaded rendering filters each effect offscreen; it must match the single-threaded
/// reference (blend modes against the real backdrop, objects partly off-screen, all effects).
#[test]
fn multithreaded_filters_match_single_threaded() {
    let cases: Vec<(Rect, Vec<(&str, serde_json::Value)>)> = vec![
        (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("stylize.dropShadow", json!({"x": 6, "y": 4, "blur": 5, "opacity": 75}))]),
        (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("stylize.outerGlow", json!({"color": "#ff00aa", "blur": 8, "opacity": 90}))]),
        (
            Rect::new(20.0, 20.0, 60.0, 60.0),
            vec![
                ("stylize.innerGlow", json!({"blur": 6})),
                ("stylize.innerGlow", json!({"blur": 4, "source": "center", "mode": "multiply", "color": "#0044ff"})),
            ],
        ),
        (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("stylize.feather", json!({"radius": 6}))]),
        (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("blur.gaussian", json!({"radius": 4})), ("stylize.dropShadow", json!({"blur": 3}))]),
        // Partly outside the frame: the blur still spreads in from off-screen content.
        (
            Rect::new(-30.0, 70.0, 30.0, 130.0),
            vec![("stylize.outerGlow", json!({"blur": 12, "mode": "normal"})), ("blur.gaussian", json!({"radius": 3}))],
        ),
    ];
    for (r, fx) in cases {
        let name = format!("{fx:?}");
        let mut d = doc_with(r, Color::rgb(0.2, 0.6, 0.3), fx);
        // A coloured backdrop so blend modes have something to blend with.
        let l = d.layers[0].id;
        let bid = d.alloc_id();
        let bg = Node::path(
            bid,
            shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
            Appearance::basic(Paint::solid(Color::rgb(0.9, 0.5, 0.1)), Paint::None, 0.0),
        );
        d.insert(Some(l), 0, bg).unwrap();
        let opts = RenderOptions::default();
        let mut st = Renderer::new();
        st.threads = 0;
        let mut mt = Renderer::new();
        mt.threads = 3;
        let a = st.render(&d, 100, 100, Affine::IDENTITY, &opts);
        let b = mt.render(&d, 100, 100, Affine::IDENTITY, &opts);
        let worst = a.pixels.iter().zip(&b.pixels).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        // The offscreen pass adds one 8-bit premultiplied round trip: a few levels of rounding.
        assert!(worst <= 4, "{name}: max channel difference {worst}");
    }
}

#[test]
fn photoshop_filters_draw_and_follow_a_pan_from_the_cache() {
    let d = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::BLACK, vec![("blur.radial", json!({"amount": 40, "quality": "draft"}))]);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    let a = r.render(&d, 100, 100, Affine::IDENTITY, &opts);
    assert!(lum(a.pixel(31, 31)) > 60, "the spin softens the corners: {:?}", a.pixel(31, 31));
    assert_eq!(lum(a.pixel(50, 50)), 0, "the centre stays");
    assert!(lum(a.pixel(50, 28)) < WHITE, "turned corners reach past the sides");
    // Panned: the cached raster moves with the view.
    let b = r.render(&d, 100, 100, Affine::translate((10.0, 3.0)), &opts);
    for (x, y) in [(31, 31), (50, 28), (69, 50), (40, 66)] {
        assert_eq!(a.pixel(x, y), b.pixel(x + 10, y + 3), "({x}, {y})");
    }
    // A fresh renderer draws the panned view the same.
    let c = Renderer::new().render(&d, 100, 100, Affine::translate((10.0, 3.0)), &opts);
    assert_eq!(b.pixels, c.pixels);
}

/// A blurred image that can't be decoded: every frame that draws it counts the failure
/// ([`crate::paint::decode_failures`]), from a new raster or from the raster kept.
#[test]
fn cached_effect_rasters_count_the_images_they_could_not_decode() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let mut d = Document::new(100.0, 100.0);
    // A PNG signature with nothing after it.
    d.images.insert("cut".into(), ImageBlob::new("image/png", b"\x89PNG\r\n\x1a\n".to_vec()));
    let im = ImageObject {
        key: "cut".into(),
        width: 40,
        height: 40,
        xf: Affine::translate((30.0, 30.0)),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.appearance.effects = vec![Effect { id: "blur.gaussian".into(), params: json!({"radius": 4.0}), visible: true }];
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    for frame in 0..3 {
        let before = crate::paint::decode_failures();
        r.render(&d, 100, 100, Affine::IDENTITY, &opts);
        assert!(crate::paint::decode_failures() > before, "frame {frame}");
    }
    assert_eq!(r.pixel_fx.len(), 1, "the later frames draw from the raster kept");
}

/// An image that can't be decoded, with Rasterize, a drop shadow and a blur: the shadow draws
/// the Rasterize image first, and the blurred content draws the image kept from that draw
/// ([`super::Kept`]). Every frame counts the failure, also when the shadow and the blurred
/// content are drawn from the rasters kept.
#[test]
fn rasters_drawn_from_a_kept_rasterize_image_count_the_images_it_could_not_decode() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let mut d = Document::new(100.0, 100.0);
    // A PNG signature with nothing after it.
    d.images.insert("cut".into(), ImageBlob::new("image/png", b"\x89PNG\r\n\x1a\n".to_vec()));
    let im = ImageObject {
        key: "cut".into(),
        width: 40,
        height: 40,
        xf: Affine::translate((30.0, 30.0)),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.appearance.effects = [
        ("rasterize", json!({"background": "white"})),
        ("stylize.dropShadow", json!({"mode": "normal", "opacity": 100, "x": 2, "y": 2, "blur": 1, "color": "#0000ff"})),
        ("blur.gaussian", json!({"radius": 4.0})),
    ]
    .into_iter()
    .map(|(id, params)| Effect { id: id.into(), params, visible: true })
    .collect();
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = Renderer::new();
    for frame in 0..3 {
        let before = crate::paint::decode_failures();
        r.render(&d, 100, 100, Affine::IDENTITY, &opts);
        assert!(crate::paint::decode_failures() > before, "frame {frame}");
    }
    assert_eq!((r.shadows.len(), r.pixel_fx.len()), (1, 1), "the later frames draw from the rasters kept");
}

/// While interacting, an object that only moved draws its drop shadow and filtered content
/// from the rasters of an earlier frame, at the nearest whole pixel, without running the
/// effects again (#1257). The frame after the interaction runs them again: it is what a fresh
/// renderer draws.
#[test]
fn moved_objects_draw_their_effect_rasters_while_interacting() {
    let effects = vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 2, "opacity": 80})), ("texture.craquelure", json!({"crackSpacing": 6}))];
    let base = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), effects);
    let id = base.layers[0].children().unwrap()[0].id;
    // The object moved from where it started, as each frame of a drag moves it.
    let moved = |dx: f64, dy: f64| {
        let mut d = base.clone();
        d.node_mut(id).unwrap().transform(Affine::translate((dx, dy)), false);
        d
    };
    let (view, still) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
    let live = RenderOptions { interacting: true, ..still.clone() };
    let fresh = |d: &Document| Renderer::new().render(d, 200, 200, view, &still).pixels;
    let mut r = Renderer::new();
    assert!(r.render(&base, 200, 200, view, &still).pixels == fresh(&base));
    assert_eq!(r.stats.moved, 0);
    let (mut d, mut last) = (base.clone(), vec![]);
    for (dx, dy) in [(3.0, 0.0), (3.3, 1.2), (5.15, 2.5), (6.3, 2.2)] {
        d = moved(dx, dy);
        last = r.render(&d, 200, 200, view, &live).pixels;
        assert_eq!(r.stats.moved, 2, "the shadow and the filtered content moved by ({dx}, {dy})");
        // At the nearest whole pixel (2 a point): what a fresh renderer draws there.
        let whole = moved((dx * 2.0).round() / 2.0, (dy * 2.0).round() / 2.0);
        assert!(last == fresh(&whole), "moved by ({dx}, {dy})");
    }
    // The frame after the drag, of the same document: the effects run again where the object is.
    let after = r.render(&d, 200, 200, view, &still).pixels;
    assert_eq!(r.stats.moved, 0);
    assert!(after == fresh(&d), "the frame after the drag");
    assert!(after != last, "6.3 is not on a whole pixel");
}

/// While dragging (#2558), filtered content and a drop shadow that the frame does not draw from a
/// raster of an earlier frame render within the drag's budget and keep no raster. At 6400% a
/// rectangle with Drop Shadow and Texturizer covers more than the raster budget, so the still
/// frame keeps rasters cropped to its view, which the moved rectangle can't take along, and a
/// 1600 × 1000 view near its corner needs more than the drag's budget. The
/// frame after the drag, of the same document, is what a fresh renderer draws, as is a frame
/// rendered while interacting without a drag (a dialog's preview).
#[test]
fn a_drag_renders_raster_effects_within_the_drag_budget() {
    let effects = vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 7, "opacity": 80})), ("texture.texturizer", json!({}))];
    let base = doc_with(Rect::new(20.0, 20.0, 80.0, 76.0), Color::rgb(0.9, 0.4, 0.1), effects);
    let id = base.layers[0].children().unwrap()[0].id;
    let mut d = base.clone();
    d.node_mut(id).unwrap().transform(Affine::translate((0.3, 0.2)), false);
    // The view is centered 10 pt inside the rectangle's right and bottom edges, and shows the
    // shadow beyond the right edge.
    let view = Affine::translate((800.0, 500.0)) * Affine::scale(64.0) * Affine::translate((-70.0, -66.0));
    let still = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let live = RenderOptions { interacting: true, ..still.clone() };
    let drag = RenderOptions { dragging: true, ..live.clone() };
    // Every renderer here has 3 threads on any machine. A drag renders drop shadows within its
    // budget only with `threads` above 0.
    let threaded = || {
        let mut r = Renderer::new();
        r.threads = 3;
        r
    };
    let fresh = threaded().render(&d, 1600, 1000, view, &still).pixels;
    let mut r = threaded();
    let _ = r.render(&base, 1600, 1000, view, &still);
    assert_eq!(r.stats.previewed, 0);
    let dragged = r.render(&d, 1600, 1000, view, &drag).pixels;
    assert_eq!(r.stats.previewed, 2, "the shadow and the filtered content");
    assert_eq!(r.stats.moved, 2, "counted as moved: the frame after the drag renders them again");
    assert!(r.pixel_fx.is_empty() && r.shadows.values().all(|e| e.stamp < r.stamp), "no raster kept");
    assert!(dragged != fresh, "at the drag's lower resolution");
    let after = r.render(&d, 1600, 1000, view, &still).pixels;
    assert_eq!((r.stats.previewed, r.stats.moved), (0, 0));
    assert!(after == fresh, "the frame after the drag");
    assert!(r.render(&d, 1600, 1000, view, &live).pixels == fresh, "interacting without a drag");
    assert_eq!(r.stats.previewed, 0);
    // A drop shadow of about 2 million pixels at 4000%, within the shadow cache's limit: scaled
    // by a drag, it can't take its raster along, and renders within the drag's budget.
    let shadowed = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), vec![("stylize.dropShadow", json!({"blur": 2}))]);
    let mut scaled = shadowed.clone();
    let id = scaled.layers[0].children().unwrap()[0].id;
    scaled.node_mut(id).unwrap().transform(Affine::scale_about(1.05, vectorcraft_geom::Point::new(35.0, 33.0)), false);
    let view = Affine::translate((800.0, 500.0)) * Affine::scale(40.0) * Affine::translate((-35.0, -33.0));
    let mut r = threaded();
    let _ = r.render(&shadowed, 1600, 1000, view, &still);
    let kept = r.shadows.len();
    assert_eq!((kept, r.stats.previewed), (1, 0), "a still frame keeps the shadow's raster");
    let _ = r.render(&scaled, 1600, 1000, view, &drag);
    assert_eq!((r.shadows.len(), r.stats.previewed), (kept, 1), "the scaled shadow");
    let after = r.render(&scaled, 1600, 1000, view, &still).pixels;
    assert!(after == threaded().render(&scaled, 1600, 1000, view, &still).pixels, "the frame after the drag");
}

/// While interacting, a group holding a symbol instance whose art lies outside the 20 × 20
/// square around the origin draws its drop shadow from the raster of an earlier frame. The
/// shadow is made from the instance's art, which the renderer measures the group's content by.
#[test]
fn a_moved_group_draws_the_shadow_of_the_instance_art_in_it_while_interacting() {
    let _fonts = fonts_steady();
    let mut base = Document::new(100.0, 100.0);
    let orange = Appearance::basic(Paint::solid(Color::rgb(0.9, 0.4, 0.1)), Paint::None, 0.0);
    let art = Node::path(base.alloc_id(), shapes::rectangle(Rect::new(100.0, 100.0, 130.0, 126.0)), orange);
    base.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Far".into(), art: std::sync::Arc::new(art) });
    // The instance draws the art from (20, 20) to (50, 46), and the 20 × 20 square around the
    // origin from (-90, -90) to (-70, -70).
    let xf = Affine::translate((-80.0, -80.0));
    let instance = Node::new(base.alloc_id(), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Far".into(), xf });
    let mut group = Node::group(base.alloc_id(), vec![std::sync::Arc::new(instance)]);
    let shadow = json!({"x": 4, "y": 3, "blur": 2, "opacity": 80});
    group.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: shadow, visible: true });
    let (id, l) = (group.id, base.layers[0].id);
    base.insert(Some(l), 0, group).unwrap();
    // The group moved from where it started, as each frame of a drag moves it.
    let moved = |dx: f64, dy: f64| {
        let mut d = base.clone();
        d.node_mut(id).unwrap().transform(Affine::translate((dx, dy)), false);
        d
    };
    let (view, still) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
    let live = RenderOptions { interacting: true, ..still.clone() };
    let fresh = |d: &Document| Renderer::new().render(d, 200, 200, view, &still);
    // Content that shows a symbol reads the fonts, so the frames come from a run in which the
    // fonts stayed the same.
    let moves = [(3.0, 0.0), (5.5, 2.0)];
    let (first, frames) = with_fonts_unchanged(|| {
        let mut r = Renderer::new();
        let first = r.render(&base, 200, 200, view, &still);
        let frames = moves.map(|(dx, dy)| {
            let d = moved(dx, dy);
            let frame = r.render(&d, 200, 200, view, &live).pixels;
            (r.stats.moved, frame == fresh(&d).pixels)
        });
        (first, frames)
    });
    assert!(first.pixels == fresh(&base).pixels);
    // The shadow, 4 pt right of the art's right edge at x 50, at (104, 70) in pixels.
    assert!(lum(first.pixel(104, 70)) < WHITE, "{:?}", first.pixel(104, 70));
    // Moves on whole pixels (2 a point): the moved shadow lies where a fresh renderer draws it.
    for ((dx, dy), (count, same)) in moves.into_iter().zip(frames) {
        assert_eq!(count, 1, "the shadow moved by ({dx}, {dy})");
        assert!(same, "moved by ({dx}, {dy})");
    }
}

/// A gradient bar with a drop shadow in a symbol with 9-slice guides, across the guides of an
/// instance scaled twice across: the bar is drawn cell by cell, and its shadow still shows past
/// the bar's right end, where the right cell ends.
#[test]
fn a_shadow_on_art_across_nine_slice_guides_reaches_past_the_outer_cells() {
    let _fonts = fonts_steady();
    let mut d = Document::new(300.0, 200.0);
    let gradient = Paint::Gradient(Box::new(vectorcraft_doc::color::GradientPaint::new(Default::default())));
    let frame = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default());
    let mut bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 100.0, 60.0)), Appearance::basic(gradient, Paint::None, 0.0));
    let shadow = json!({"x": 8, "y": 0, "blur": 0, "opacity": 100});
    bar.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: shadow, visible: true });
    let art = Node::group(d.alloc_id(), vec![std::sync::Arc::new(frame), std::sync::Arc::new(bar)]);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Bar".into(), art: std::sync::Arc::new(art) });
    d.set_symbol_nine_slice("Bar", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let xf = Affine::translate((20.0, 20.0)) * Affine::scale_non_uniform(2.0, 1.0);
    let instance = Node::new(d.alloc_id(), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Bar".into(), xf });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, instance).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let out = Renderer::new().render(&d, 300, 200, Affine::IDENTITY, &opts);
    // The bar runs from x 20 to 220 and y 60 to 80; its shadow, 8 pt to the right, to x 228.
    assert!(lum(out.pixel(224, 70)) < WHITE, "{:?}", out.pixel(224, 70));
    assert_eq!(lum(out.pixel(232, 70)), WHITE, "{:?}", out.pixel(232, 70));
}

/// While interacting, a raster moves with its object only when the content's pixels (for a
/// shadow, their alpha) and the effects stayed the same (#1257): a new fill color under
/// Craquelure, other effect settings, a pattern fill that stays on the page, a resized object,
/// art added beyond the old raster, a changed Feather outline and De-Interlace run the effects
/// again. A shadow keeps its raster under a new fill color.
#[test]
fn moved_rasters_need_the_same_content_and_effects() {
    let _fonts = fonts_steady();
    let (view, still) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
    let live = RenderOptions { interacting: true, ..still.clone() };
    // `before` drawn, then `after` while interacting: the rasters drawn moved, and whether the
    // frame is what a fresh renderer draws. Content painted with a pattern reads the fonts, so
    // the counts come from a run in which the fonts stayed the same.
    let drag = |before: &Document, after: &Document| {
        with_fonts_unchanged(|| {
            let mut r = Renderer::new();
            let _ = r.render(before, 200, 200, view, &still);
            let frame = r.render(after, 200, 200, view, &live).pixels;
            (r.stats.moved, frame == Renderer::new().render(after, 200, 200, view, &still).pixels)
        })
    };
    // `d`'s object moved 5, 3 (10, 6 pixels), then changed by `edit`.
    let moved = |d: &Document, patterns: bool, edit: &dyn Fn(&mut Node)| {
        let mut d = d.clone();
        let id = d.layers[0].children().unwrap()[0].id;
        let n = d.node_mut(id).unwrap();
        n.transform(Affine::translate((5.0, 3.0)), vectorcraft_doc::Scaling { patterns, ..Default::default() });
        edit(n);
        d
    };
    let cracks = || vec![("texture.craquelure", json!({"crackSpacing": 6}))];
    let base = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), cracks());
    assert_eq!(drag(&base, &moved(&base, false, &|_| {})), (1, true), "moved");
    let recolor = |n: &mut Node| n.appearance.fill_mut().unwrap().paint = Paint::solid(Color::rgb(0.1, 0.4, 0.9));
    assert_eq!(drag(&base, &moved(&base, false, &recolor)), (0, true), "recolored");
    let spacing = |n: &mut Node| n.appearance.effects[0].params = json!({"crackSpacing": 9});
    assert_eq!(drag(&base, &moved(&base, false, &spacing)), (0, true), "other effect settings");
    let resize = |n: &mut Node| n.transform(Affine::scale_about(1.5, vectorcraft_geom::Point::new(25.0, 23.0)), false);
    assert_eq!(drag(&base, &moved(&base, false, &resize)), (0, true), "resized");
    // A square added beyond the old raster leaves the content inside it as it was.
    let beyond = |n: &mut Node| {
        if let vectorcraft_doc::NodeKind::Path { path, .. } = &mut n.kind {
            path.subpaths.extend(shapes::rectangle(Rect::new(75.0, 30.0, 80.0, 35.0)).subpaths);
        }
    };
    assert_eq!(drag(&base, &moved(&base, false, &beyond)), (0, true), "art added beyond the old raster");
    // At 72 ppi, De-Interlace works on lines 1 point tall. The object moves 3 points down, so
    // De-Interlace makes its other field again.
    let fields = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), vec![("video.deinterlace", json!({}))]);
    assert_eq!(drag(&fields, &moved(&fields, false, &|_| {})), (0, true), "De-Interlace");
    // A pattern fill: its tiles stay on the page unless they move with the object.
    let mut tiled = base.clone();
    let red = Node::path(
        vectorcraft_doc::NodeId(900),
        shapes::rectangle(Rect::new(0.0, 0.0, 3.0, 3.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    let mut def = vectorcraft_doc::pattern::PatternDef::new("Dots", vec![std::sync::Arc::new(red)]);
    def.tile = Rect::new(0.0, 0.0, 7.0, 7.0);
    tiled.patterns.push(def);
    let id = tiled.layers[0].children().unwrap()[0].id;
    tiled.node_mut(id).unwrap().appearance.fill_mut().unwrap().paint = vectorcraft_doc::pattern::pattern_paint("Dots");
    assert_eq!(drag(&tiled, &moved(&tiled, false, &|_| {})), (0, true), "tiles that stay on the page");
    assert_eq!(drag(&tiled, &moved(&tiled, true, &|_| {})), (1, true), "tiles that move with the object");
    // Feather clips to the outline: a notch in the top edge where the tiles paint nothing
    // (x 29 to 33 and y 20 to 24 once moved) leaves the content's pixels as they were.
    let mut feathered = tiled.clone();
    feathered.node_mut(id).unwrap().appearance.effects = vec![Effect { id: "stylize.feather".into(), params: json!({"radius": 3}), visible: true }];
    assert_eq!(drag(&feathered, &moved(&feathered, true, &|_| {})), (1, true), "feathered");
    let notch = |n: &mut Node| {
        let mut bp = vectorcraft_geom::BezPath::new();
        bp.move_to((25.0, 23.0));
        for p in [(30.0, 23.0), (30.5, 23.6), (31.5, 23.6), (32.0, 23.0), (55.0, 23.0), (55.0, 49.0), (25.0, 49.0)] {
            bp.line_to(p);
        }
        bp.close_path();
        if let vectorcraft_doc::NodeKind::Path { path, .. } = &mut n.kind {
            *path = vectorcraft_geom::PathData::from_bezpath(&bp);
        }
    };
    assert_eq!(drag(&feathered, &moved(&feathered, true, &notch)), (0, true), "a notch in the feathered outline");
    // A shadow is made from the silhouette's alpha: recolored, the object takes its shadow
    // along; notched, the object makes its shadow again.
    let shadowed =
        doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 2}))]);
    assert_eq!(drag(&shadowed, &moved(&shadowed, false, &recolor)), (1, true), "a shadow under a new color");
    assert_eq!(drag(&shadowed, &moved(&shadowed, false, &notch)), (0, true), "a shadow under a notch");
    // A shadow with Darkness is made from the object's colors: recolored, it is made again.
    let shaded = doc_with(
        Rect::new(20.0, 20.0, 50.0, 46.0),
        Color::rgb(0.9, 0.4, 0.1),
        vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 2, "shade": "darkness", "darkness": 50}))],
    );
    assert_eq!(drag(&shaded, &moved(&shaded, false, &|_| {})), (1, true), "a shadow with Darkness, moved");
    assert_eq!(drag(&shaded, &moved(&shaded, false, &recolor)), (0, true), "a shadow with Darkness under a new color");
}

/// A raster made over moved rasters counts as moved itself (#1257): a group with a drop shadow
/// and Gaussian Blur of its own, whose filtered member moves by parts of a pixel while the
/// other member stays, draws after the drag what a fresh renderer draws.
#[test]
fn rasters_made_over_moved_rasters_count_as_moved() {
    let mut d = Document::new(100.0, 100.0);
    let fill = |c: Color| Appearance::basic(Paint::solid(c), Paint::None, 0.0);
    let (sid, mid, gid) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let still = Node::path(sid, shapes::rectangle(Rect::new(60.0, 60.0, 80.0, 80.0)), fill(Color::rgb(0.1, 0.5, 0.2)));
    let mut member = Node::path(mid, shapes::rectangle(Rect::new(20.0, 20.0, 50.0, 46.0)), fill(Color::rgb(0.9, 0.4, 0.1)));
    member.appearance.effects = vec![Effect { id: "texture.craquelure".into(), params: json!({"crackSpacing": 6}), visible: true }];
    let mut group = Node::group(gid, vec![std::sync::Arc::new(still), std::sync::Arc::new(member)]);
    group.appearance.effects = vec![
        Effect { id: "stylize.dropShadow".into(), params: json!({"x": 4, "y": 3, "blur": 2}), visible: true },
        Effect { id: "blur.gaussian".into(), params: json!({"radius": 1}), visible: true },
    ];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, group).unwrap();
    let (view, opts) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
    let mut r = Renderer::new();
    let _ = r.render(&d, 200, 200, view, &opts);
    let mut moved = d.clone();
    moved.node_mut(mid).unwrap().transform(Affine::translate((3.3, 1.2)), false);
    let _ = r.render(&moved, 200, 200, view, &RenderOptions { interacting: true, ..opts.clone() });
    assert!(r.stats.moved > 0, "the member's filtered content moved");
    let after = r.render(&moved, 200, 200, view, &opts).pixels;
    assert!(after == Renderer::new().render(&moved, 200, 200, view, &opts).pixels, "the frame after the drag");
}

/// Reproduce #1257's moving, filtered rectangle: the first frame renders as before a drag,
/// and the next eight render as the canvas renders a drag's frames (`interacting`). Then the
/// eighth position renders as the canvas renders the frame after the drag, and that frame
/// renders once more with every raster cached.
/// Run with `cargo test -p vectorcraft-render texture_drag_perf -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn texture_drag_perf() {
    for effect in ["texture.craquelure", "texture.mosaicTiles", "texture.stainedGlass"] {
        let mut base = Document::new(600.0, 500.0);
        base.color_mode = vectorcraft_doc::ColorMode::Cmyk;
        let (layer, id) = (base.layers[0].id, base.alloc_id());
        let mut n = Node::path(
            id,
            shapes::rectangle(Rect::new(100.0, 100.0, 309.298, 370.938)),
            Appearance::basic(Paint::solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), Paint::solid(Color::BLACK), 1.0),
        );
        n.appearance.effects = vec![
            Effect { id: "stylize.roundCorners".into(), params: json!({"radius": 25.175}), visible: true },
            Effect { id: effect.into(), params: json!({}), visible: true },
        ];
        base.insert(Some(layer), 0, n).unwrap();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let live = RenderOptions { interacting: true, ..opts.clone() };
        let view = Affine::scale(1.9007 * 1.5);
        let mut r = Renderer::new();
        r.threads = crate::default_threads();
        let (mut ms, mut doc) = (vec![], base.clone());
        for frame in 0..9 {
            doc = base.clone();
            doc.node_mut(id).unwrap().transform(Affine::translate((3.0 + f64::from(frame) * 0.75, f64::from(frame) * 0.5)), false);
            let start = std::time::Instant::now();
            let image = r.render(&doc, 1800, 1500, view, if frame > 0 { &live } else { &opts });
            let time = start.elapsed().as_secs_f64() * 1000.0;
            if frame > 0 {
                ms.push(time);
            }
            std::hint::black_box(image);
        }
        let mut time = |o: &RenderOptions| {
            let start = std::time::Instant::now();
            std::hint::black_box(r.render(&doc, 1800, 1500, view, o));
            start.elapsed().as_secs_f64() * 1000.0
        };
        let (after, again) = (time(&opts), time(&opts));
        ms.sort_by(f64::total_cmp);
        println!(
            "{effect}: moved CMYK rounded rectangle at 190.07%, DPI 1.5, 1800x1500: median {:.2} ms; samples {ms:?}; the frame after the drag {after:.2} ms, that frame again {again:.2} ms",
            (ms[3] + ms[4]) * 0.5
        );
    }
}

/// Reproduce #2558's drag of a textured or blurred rectangle when zoomed in, in a 2560 × 1440
/// view centered on the rectangle: the first frame renders as before a drag, and the next eight
/// render as a drag's frames, once with `interacting` alone and once with `dragging` as well (as
/// the canvas renders them). Then the eighth position renders as the canvas renders the frame
/// after the drag. The hashes of that frame and of a fresh renderer's frame compare builds.
/// Run with `cargo test -p vectorcraft-render zoomed_drag_perf -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn zoomed_drag_perf() {
    let sets: [(&str, Vec<(&str, serde_json::Value)>); 3] = [
        ("Texturizer", vec![("texture.texturizer", json!({}))]),
        ("Gaussian Blur 8 pt", vec![("blur.gaussian", json!({"radius": 8}))]),
        ("Drop Shadow and Craquelure", vec![("stylize.dropShadow", json!({"blur": 7})), ("texture.craquelure", json!({}))]),
    ];
    let hash = |px: &[u8]| {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        px.hash(&mut h);
        h.finish()
    };
    for (name, effects) in sets {
        for zoom in [1.0, 4.0, 16.0, 64.0] {
            let base = doc_with(Rect::new(100.0, 100.0, 300.0, 250.0), Color::rgb(0.9, 0.4, 0.1), effects.clone());
            let id = base.layers[0].children().unwrap()[0].id;
            let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
            let view = Affine::translate((1280.0, 720.0)) * Affine::scale(zoom) * Affine::translate((-200.0, -175.0));
            for dragging in [false, true] {
                let live = RenderOptions { interacting: true, dragging, ..opts.clone() };
                let mut r = Renderer::new();
                r.threads = crate::default_threads();
                let (mut ms, mut doc) = (vec![], base.clone());
                for frame in 0..9 {
                    doc = base.clone();
                    let by = (f64::from(frame) * 0.75 / zoom, f64::from(frame) * 0.5 / zoom);
                    doc.node_mut(id).unwrap().transform(Affine::translate(by), false);
                    let start = std::time::Instant::now();
                    std::hint::black_box(r.render(&doc, 2560, 1440, view, if frame > 0 { &live } else { &opts }));
                    if frame > 0 {
                        ms.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                let previewed = r.stats.previewed;
                let start = std::time::Instant::now();
                let still = r.render(&doc, 2560, 1440, view, &opts);
                let after = start.elapsed().as_secs_f64() * 1000.0;
                let fresh = Renderer::new().render(&doc, 2560, 1440, view, &opts);
                ms.sort_by(f64::total_cmp);
                println!(
                    "{name} at {:.0}%, dragging {dragging}: drag frames median {:.1} ms (min {:.1}, max {:.1}), {previewed} previewed; the frame after the drag {after:.1} ms; hashes {:016x} {:016x}",
                    zoom * 100.0,
                    (ms[3] + ms[4]) * 0.5,
                    ms[0],
                    ms[7],
                    hash(&still.pixels),
                    hash(&fresh.pixels)
                );
            }
        }
    }
}

#[test]
fn photoshop_filters_on_content_larger_than_the_budget() {
    // Zoomed in 200×: the 40 pt square covers 8000 × 8000 px around a 200 × 200 view.
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let view = Affine::translate((100.0, 100.0)) * Affine::scale(200.0) * Affine::translate((-70.0, -50.0));
    for (id, p) in [("sharpen.unsharpMask", json!({"amount": 200})), ("blur.smart", json!({})), ("blur.radial", json!({"quality": "draft"}))] {
        let d = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::rgb(0.2, 0.4, 0.8), vec![(id, p)]);
        let img = Renderer::new().render(&d, 200, 200, view, &opts);
        // The view shows the square's right side half way down: inside left, outside right.
        assert!(lum(img.pixel(40, 100)) < 600, "{id}: inside {:?}", img.pixel(40, 100));
        // (Smart Blur's wide samples read a blurred copy: a faint fringe under its threshold.)
        assert!(lum(img.pixel(190, 100)) >= WHITE - 3 * 25, "{id}: outside {:?}", img.pixel(190, 100));
    }
}

/// #5308: an image's Inner Glow follows its visible pixels, not its frame. A 40 × 40 image whose
/// left half is transparent and right half opaque black, on a black page, with a white glow from
/// the edges: the transparent half stays black, and the glow lights the opaque half's left edge
/// (inside the frame), on the single-threaded and the multithreaded renderer alike.
#[test]
fn an_image_inner_glow_follows_its_visible_pixels() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let rgba: Vec<u8> = (0..40 * 40).flat_map(|i| if i % 40 < 20 { [0, 0, 0, 0] } else { [0, 0, 0, 255] }).collect();
    let png = crate::encode::png::encode(&rgba, 40, 40, &Default::default()).unwrap();
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("half".into(), ImageBlob::new("image/png", png));
    let im = ImageObject {
        key: "half".into(),
        width: 40,
        height: 40,
        xf: Affine::translate((30.0, 30.0)),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.appearance.effects = vec![Effect {
        id: "stylize.innerGlow".into(),
        params: json!({"color": "#ffffff", "mode": "normal", "opacity": 100, "blur": 4}),
        visible: true,
    }];
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let opts = RenderOptions { background: Some([0, 0, 0, 255]), ..Default::default() };
    for threads in [0, 3] {
        let mut r = Renderer::new();
        r.threads = threads;
        let img = r.render(&d, 100, 100, Affine::IDENTITY, &opts);
        for (x, y) in [(31, 50), (40, 31), (45, 50)] {
            assert!(lum(img.pixel(x, y)) < 10, "threads {threads}: no glow on the transparent pixels at ({x}, {y}): {:?}", img.pixel(x, y));
        }
        assert!(lum(img.pixel(51, 50)) > 60, "threads {threads}: the glow lights the visible pixels' edge: {:?}", img.pixel(51, 50));
        assert!(lum(img.pixel(68, 50)) > 60, "threads {threads}: and the frame's edge where the pixels reach it: {:?}", img.pixel(68, 50));
        assert!(lum(img.pixel(60, 50)) < lum(img.pixel(51, 50)), "threads {threads}: fading inward");
    }
}

/// A 3D effect on a path's fill or stroke that builds nothing (Revolve after Roughen) draws the
/// item as without it: the art made for the object is drawn without it, not as an object with
/// effects again, and that art again, without end. A stroked object draws exactly as without the
/// 3D effect, with a raster effect of its own (Drop Shadow, Gaussian Blur) too. An object with
/// only a fill and a raster effect draws from that art, where without the 3D effect it draws as a
/// path, whose raster effects are clipped to the path's own geometry: there Roughen's spikes are
/// cut off, so the two differ in a few pixels at the edge and match inside the path. That clipping
/// is a known bug of drawing without the 3D effect, older than this test, not the design. Either
/// way the raster effect reaches past what the object draws without it.
#[test]
fn a_3d_effect_that_builds_nothing_draws_the_item_as_without_it() {
    let rough =
        Effect { id: "distort.roughen".into(), params: json!({"size": 10, "relative": false, "detail": 10, "points": "smooth"}), visible: true };
    let revolve = Effect { id: "threeD.revolve".into(), params: json!({}), visible: true };
    let shadow = Effect { id: "stylize.dropShadow".into(), params: json!({}), visible: true };
    let blur = Effect { id: "blur.gaussian".into(), params: json!({"radius": 2}), visible: true };
    let cases = [
        (vec![], true, true),
        (vec![], false, true),
        (vec![shadow.clone()], true, true),
        (vec![blur.clone()], false, true),
        (vec![], true, false),
        (vec![shadow], true, false),
        (vec![blur], true, false),
    ];
    for (object, on_fill, stroked) in cases {
        let draw = |object: &[Effect], three: bool| {
            let mut d = Document::new(100.0, 100.0);
            let stroke = if stroked { Paint::solid(Color::rgb(0.0, 0.0, 1.0)) } else { Paint::None };
            let ap = Appearance::basic(Paint::solid(Color::BLACK), stroke, 2.0);
            let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(30.0, 30.0, 60.0, 60.0)), ap);
            let l = d.layers[0].id;
            let id = d.insert(Some(l), 0, n).unwrap();
            let n = d.node_mut(id).unwrap();
            n.appearance.effects = object.to_vec();
            let item = if on_fill {
                n.appearance.fill_mut().unwrap().effects = vec![];
                &mut n.appearance.items[0]
            } else {
                &mut n.appearance.items[1]
            };
            *item.effects_mut() = [Some(rough.clone()), three.then(|| revolve.clone())].into_iter().flatten().collect();
            assert!(crate::effects::revolve_art(d.node(id).unwrap()).is_none(), "builds nothing");
            Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default()).pixels
        };
        let what = format!("{object:?} on the fill {on_fill}, stroked {stroked}");
        let (with, without) = (draw(&object, true), draw(&object, false));
        if stroked || object.is_empty() {
            assert!(with == without, "{what}: as without it");
        } else {
            // Under 4% of the pixels, all at the edge, where Roughen's spikes are: inside the path
            // they match.
            let differ: Vec<(usize, usize)> =
                (0..100 * 100).filter(|i| with[i * 4..i * 4 + 4] != without[i * 4..i * 4 + 4]).map(|i| (i % 100, i / 100)).collect();
            assert!(differ.len() * 25 < 100 * 100, "{what}: {} pixels differ", differ.len());
            let inside = differ.iter().find(|(x, y)| (33..57).contains(x) && (33..57).contains(y));
            assert!(inside.is_none(), "{what}: differs inside the path at {inside:?}");
        }
        if !object.is_empty() {
            // Ink where the object without its raster effect draws none.
            let plain = draw(&[], true);
            let past = (0..100 * 100).filter(|i| plain[i * 4 + 3] == 0 && with[i * 4 + 3] > 0).count();
            assert!(past > 0, "{what}: drawn past the object");
        }
    }
}
