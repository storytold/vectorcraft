//! Pixel-sample "golden" tests on small fixtures: gradients, blend modes, clip groups, stroke
//! alignment/dashes/arrowheads, effects, text, and opacity folding vs layer compositing.

use serde_json::json;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Arrowhead, Dash, Document, Effect, FillLayer, LineCap, Node, StrokeAlign, StrokeLayer};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_testkit::fixtures::{DocBuilder, exec, session_with};
use vectorcraft_testkit::raster::{Image, assert_rgb, assert_similar, diff, render_view};

const W: u32 = 100;

fn render(d: &Document) -> Image {
    render_view(d, W, W)
}

fn stops(a: Color, b: Color) -> Vec<GradientStop> {
    vec![GradientStop { offset: 0.0, color: a, opacity: 1.0, midpoint: 0.5 }, GradientStop { offset: 1.0, color: b, opacity: 1.0, midpoint: 0.5 }]
}

fn gradient(kind: GradientKind, a: Color, b: Color, angle: f64) -> Paint {
    let mut gp = GradientPaint::new(Gradient { kind, stops: stops(a, b) });
    gp.angle = angle;
    Paint::Gradient(Box::new(gp))
}

fn fill_only(p: Paint) -> Appearance {
    Appearance::basic(p, Paint::None, 0.0)
}

fn stroke_only(width: f64, f: impl FnOnce(&mut StrokeLayer)) -> Appearance {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    st.cap = LineCap::Butt;
    f(&mut st);
    Appearance { items: vec![AppearanceItem::Stroke(st)], effects: vec![] }
}

// ---------------------------------------------------------------- gradients

#[test]
fn linear_gradient_left_to_right() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(
        shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
        fill_only(gradient(GradientKind::Linear, Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0), 0.0)),
        |_| {},
    );
    let img = render(&b.build());
    assert_rgb(&img, 1, 50, [255, 0, 0], 12);
    assert_rgb(&img, 98, 50, [0, 0, 255], 12);
    let mid = img.over_white(50, 50);
    assert!(mid[0].abs_diff(128) < 20 && mid[2].abs_diff(128) < 20 && mid[1] < 10, "mid {mid:?}");
    // Red decreases monotonically along x; rows are identical (horizontal gradient).
    let reds: Vec<u8> = (0..100).step_by(5).map(|x| img.over_white(x, 50)[0]).collect();
    assert!(reds.windows(2).all(|w| w[1] <= w[0]), "{reds:?}");
    for x in [10, 40, 70] {
        assert_eq!(img.over_white(x, 10), img.over_white(x, 90));
    }
}

#[test]
fn linear_gradient_angle_90_is_vertical() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), fill_only(gradient(GradientKind::Linear, Color::WHITE, Color::BLACK, 90.0)), |_| {});
    let img = render(&b.build());
    // Constant along rows, varying along columns.
    for y in [10, 50, 90] {
        let a = img.over_white(10, y);
        let c = img.over_white(90, y);
        assert!(a[0].abs_diff(c[0]) <= 2, "row {y}: {a:?} vs {c:?}");
    }
    assert!(img.over_white(50, 5)[0].abs_diff(img.over_white(50, 95)[0]) > 180);
}

#[test]
fn radial_gradient_centre_to_edge() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(
        shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
        fill_only(gradient(GradientKind::Radial, Color::rgb(0.0, 1.0, 0.0), Color::BLACK, 0.0)),
        |_| {},
    );
    let img = render(&b.build());
    assert_rgb(&img, 50, 50, [0, 255, 0], 15);
    // Symmetric about the centre and darker towards the edge.
    let g = |x, y| img.over_white(x, y)[1];
    assert!(g(50, 50) > g(70, 50) && g(70, 50) > g(95, 50));
    // Pixel (x) has its centre at x + 0.5, so x=30 mirrors x=69 about the centre 50.
    assert!(g(30, 50).abs_diff(g(69, 50)) <= 2 && g(50, 30).abs_diff(g(50, 69)) <= 2, "{} {} {} {}", g(30, 50), g(69, 50), g(50, 30), g(50, 69));
    assert!(g(30, 50).abs_diff(g(50, 30)) <= 2, "not circular");
}

// ---------------------------------------------------------------- blend modes

fn blend_ref(mode: BlendMode, b: f64, s: f64) -> f64 {
    let hard = |b: f64, s: f64| {
        if s <= 0.5 {
            b * 2.0 * s
        } else {
            let t = 2.0 * s - 1.0;
            b + t - b * t
        }
    };
    match mode {
        BlendMode::Normal => s,
        BlendMode::Multiply => b * s,
        BlendMode::Screen => b + s - b * s,
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        BlendMode::HardLight => hard(b, s),
        BlendMode::Overlay => hard(s, b),
        BlendMode::ColorDodge => {
            if b == 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        BlendMode::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        _ => unreachable!(),
    }
}

#[test]
fn separable_blend_modes_match_formulas() {
    let back = [0.8, 0.4, 0.2];
    let src = [0.3, 0.6, 0.9];
    for mode in [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::HardLight,
        BlendMode::Overlay,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::SoftLight,
    ] {
        let mut b = DocBuilder::new(100.0, 100.0);
        b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(back[0] as f32, back[1] as f32, back[2] as f32), |_| {});
        b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(src[0] as f32, src[1] as f32, src[2] as f32), |n| n.blend = mode);
        let img = render(&b.build());
        let want: Vec<u8> = (0..3).map(|c| (blend_ref(mode, back[c], src[c]) * 255.0).round() as u8).collect();
        assert_rgb(&img, 50, 50, [want[0], want[1], want[2]], 3);
    }
}

fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn clip_color(c: [f64; 3]) -> [f64; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}
fn set_lum(c: [f64; 3], l: f64) -> [f64; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}
fn sat(c: [f64; 3]) -> f64 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}
fn set_sat(c: [f64; 3], s: f64) -> [f64; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    if mx > mn { c.map(|v| (v - mn) * s / (mx - mn)) } else { [0.0; 3] }
}

#[test]
fn non_separable_blend_modes_match_formulas() {
    let b = [0.8, 0.4, 0.2];
    let s = [0.3, 0.6, 0.9];
    for (mode, want) in [
        (BlendMode::Hue, set_lum(set_sat(s, sat(b)), lum(b))),
        (BlendMode::Saturation, set_lum(set_sat(b, sat(s)), lum(b))),
        (BlendMode::Color, set_lum(s, lum(b))),
        (BlendMode::Luminosity, set_lum(b, lum(s))),
    ] {
        let mut d = DocBuilder::new(100.0, 100.0);
        d.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(b[0] as f32, b[1] as f32, b[2] as f32), |_| {});
        d.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(s[0] as f32, s[1] as f32, s[2] as f32), |n| n.blend = mode);
        let img = render(&d.build());
        let w = want.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let got = img.over_white(50, 50);
        assert!((0..3).all(|c| got[c].abs_diff(w[c]) <= 4), "{mode:?}: got {got:?}, want {w:?}");
    }
}

// ---------------------------------------------------------------- clipping

#[test]
fn clip_group_masks_content() {
    let mut b = DocBuilder::new(100.0, 100.0);
    let clip = b.path_node(shapes::ellipse(Rect::new(20.0, 20.0, 80.0, 80.0)), fill_only(Paint::None));
    let red = b.rect_node(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(1.0, 0.0, 0.0));
    b.group(vec![clip, red], true);
    let img = render(&b.build());
    assert_rgb(&img, 50, 50, [255, 0, 0], 2);
    assert_rgb(&img, 25, 25, [255, 255, 255], 2); // inside the rect, outside the ellipse
    assert_rgb(&img, 5, 50, [255, 255, 255], 2);
    // Area of red ≈ circle area (π·30²).
    let red_px = img.count(|p| p[0] > 200 && p[1] < 60 && p[3] > 200);
    let want = std::f64::consts::PI * 900.0;
    assert!((red_px as f64 - want).abs() < want * 0.05, "{red_px} vs {want}");
}

#[test]
fn unclipped_group_draws_everything() {
    let mut b = DocBuilder::new(100.0, 100.0);
    let e = b.path_node(shapes::ellipse(Rect::new(20.0, 20.0, 80.0, 80.0)), fill_only(Paint::None));
    let red = b.rect_node(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(1.0, 0.0, 0.0));
    b.group(vec![e, red], false);
    let img = render(&b.build());
    assert_rgb(&img, 25, 25, [255, 0, 0], 2);
}

// ---------------------------------------------------------------- strokes

#[test]
fn center_stroke_straddles_the_edge() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)), stroke_only(10.0, |s| s.align = StrokeAlign::Center), |_| {});
    let img = render(&b.build());
    assert_rgb(&img, 17, 50, [0, 0, 0], 2);
    assert_rgb(&img, 23, 50, [0, 0, 0], 2);
    assert_rgb(&img, 13, 50, [255, 255, 255], 2);
    assert_rgb(&img, 27, 50, [255, 255, 255], 2);
}

#[test]
fn dashed_line_alternates() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(
        shapes::line(Point::new(10.0, 50.0), Point::new(90.0, 50.0)),
        stroke_only(4.0, |s| s.dash = Some(Dash { pattern: vec![10.0, 10.0], offset: 0.0, align_corners: false })),
        |_| {},
    );
    let img = render(&b.build());
    for (x, on) in [(15, true), (25, false), (35, true), (45, false), (55, true), (65, false)] {
        let px = img.over_white(x, 50);
        assert_eq!(px[0] < 60, on, "x={x}: {px:?}");
    }
    // Solid line for comparison: all on.
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::line(Point::new(10.0, 50.0), Point::new(90.0, 50.0)), stroke_only(4.0, |_| {}), |_| {});
    let solid = render(&b.build());
    assert!(solid.ink() > img.ink() * 3 / 2, "solid {} dashed {}", solid.ink(), img.ink());
}

#[test]
fn arrowheads_add_ink_at_the_ends() {
    let line = || shapes::line(Point::new(10.0, 50.0), Point::new(80.0, 50.0));
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(line(), stroke_only(2.0, |_| {}), |_| {});
    let plain = render(&b.build());
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(line(), stroke_only(2.0, |s| s.end_arrow = Some(Arrowhead::Triangle)), |_| {});
    let end = render(&b.build());
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(line(), stroke_only(2.0, |s| s.start_arrow = Some(Arrowhead::Circle)), |_| {});
    let start = render(&b.build());
    assert!(end.ink() > plain.ink() + 20, "end arrow adds no ink: {} vs {}", end.ink(), plain.ink());
    assert!(start.ink() > plain.ink() + 20);
    // The ink is at the right end for the end arrow and at the left for the start arrow.
    let col_ink =
        |img: &Image, x0: u32, x1: u32| (x0..x1).flat_map(|x| (40..60).map(move |y| (x, y))).filter(|&(x, y)| img.over_white(x, y)[0] < 128).count();
    assert!(col_ink(&end, 60, 100) > col_ink(&plain, 60, 100) + 10);
    assert_eq!(col_ink(&end, 0, 30), col_ink(&plain, 0, 30));
    assert!(col_ink(&start, 0, 30) > col_ink(&plain, 0, 30) + 10);
    // Every arrowhead kind renders.
    for a in Arrowhead::ALL {
        let mut b = DocBuilder::new(100.0, 100.0);
        b.path(line(), stroke_only(2.0, |s| s.end_arrow = Some(a)), |_| {});
        assert!(render(&b.build()).ink() > plain.ink(), "{a:?}");
    }
}

// ---------------------------------------------------------------- effects

fn with_effect(id: &str, params: serde_json::Value) -> Document {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.rect(Rect::new(20.0, 20.0, 60.0, 60.0), Color::rgb(0.0, 0.0, 1.0), |n| {
        n.appearance.effects.push(Effect { id: id.into(), params, visible: true });
    });
    b.build()
}

#[test]
fn drop_shadow_darkens_offset_region() {
    let plain = render(&with_effect("stylize.dropShadow", json!({})).clone());
    let mut no = with_effect("stylize.dropShadow", json!({}));
    // Same document without the effect.
    let l = no.layers[0].id;
    let id = no.children(Some(l)).unwrap()[0].id;
    no.node_mut(id).unwrap().appearance.effects.clear();
    let none = render(&no);
    let _ = l;
    // Shadow at +7,+7 with blur: below-right of the rect is darker than white; the rect itself is unchanged.
    let p = plain.over_white(64, 64);
    assert!(p[0] < 230, "no shadow at (64,64): {p:?}");
    assert_eq!(none.over_white(64, 64), [255, 255, 255]);
    assert_rgb(&plain, 40, 40, [0, 0, 255], 2);
    assert_rgb(&plain, 10, 10, [255, 255, 255], 2);
    // A shadow with zero opacity renders like no effect.
    let zero = render(&with_effect("stylize.dropShadow", json!({"opacity": 0})));
    assert_similar(&zero, &none, 2.0, 0.0);
}

#[test]
fn roughen_is_deterministic_and_seeded() {
    let p = json!({"size": 10, "detail": 20, "relative": true, "seed": 3});
    let a = render(&with_effect("distort.roughen", p.clone()));
    let b = render(&with_effect("distort.roughen", p));
    assert_eq!(a, b, "roughen is not deterministic");
    let c = render(&with_effect("distort.roughen", json!({"size": 10, "detail": 20, "relative": true, "seed": 4})));
    assert!(diff(&a, &c).max > 100, "seed has no effect");
    let plain = render(&with_effect("distort.roughen", json!({"size": 0, "detail": 20})));
    assert!(diff(&a, &plain).max > 100, "roughen does nothing");
}

#[test]
fn geometry_effects_render_without_panicking() {
    for (id, p) in [
        ("distort.zigZag", json!({"size": 5, "ridges": 4})),
        ("distort.puckerBloat", json!({"amount": 50})),
        ("distort.twist", json!({"angle": 45})),
        ("distort.tweak", json!({"h": 10, "v": 10, "seed": 1})),
        ("stylize.roundCorners", json!({"radius": 8})),
        ("stylize.innerGlow", json!({})),
        ("stylize.outerGlow", json!({})),
        ("stylize.feather", json!({"radius": 4})),
        ("blur.gaussian", json!({"radius": 3})),
        ("path.offsetPath", json!({"offset": 5})),
        ("warp.arc", json!({})),
    ] {
        let img = render(&with_effect(id, p));
        assert!(img.ink() > 500, "{id} rendered almost nothing ({} px)", img.ink());
    }
}

#[test]
fn hidden_effect_is_ignored() {
    let mut d = with_effect("distort.roughen", json!({"size": 20, "seed": 1}));
    let l = d.layers[0].id;
    let id = d.children(Some(l)).unwrap()[0].id;
    d.node_mut(id).unwrap().appearance.effects[0].visible = false;
    let hidden = render(&d);
    d.node_mut(id).unwrap().appearance.effects.clear();
    assert_eq!(hidden, render(&d));
}

// ---------------------------------------------------------------- text

#[test]
fn text_renders_ink_inside_its_bounds() {
    let mut s = session_with(200.0, 100.0);
    exec(&mut s, "text.create", json!({"x": 10, "y": 60, "text": "Hello", "size": 40}));
    exec(&mut s, "paint.setFill", json!({"color": "#000000"}));
    let doc = s.doc().unwrap().doc.clone();
    let img = render_view(&doc, 200, 100);
    let ink = img.ink();
    assert!(ink > 300, "text rendered {ink} px");
    // Nothing far away from the text.
    let far = (150..200).flat_map(|x| (0..100).map(move |y| (x, y))).filter(|&(x, y)| img.over_white(x, y) != [255, 255, 255]).count();
    assert_eq!(far, 0);
    // Outlined text renders the same pixels (within anti-aliasing).
    exec(&mut s, "select.all", json!({}));
    exec(&mut s, "type.createOutlines", json!({}));
    let outlined = render_view(&s.doc().unwrap().doc, 200, 100);
    assert_similar(&img, &outlined, 24.0, 0.01);
}

// ---------------------------------------------------------------- opacity folding

/// Opacity on a 1-item appearance (folded into the paint alpha) must match opacity on a 2-item
/// appearance (which forces a compositing layer) when the second item doesn't change the result.
#[test]
fn opacity_folding_equals_layer_compositing() {
    for (opacity, blend) in [(0.5, BlendMode::Normal), (0.25, BlendMode::Normal), (0.8, BlendMode::Multiply), (0.6, BlendMode::Screen)] {
        let red = Color::rgb(0.9, 0.1, 0.2);
        let one = {
            let mut b = DocBuilder::new(100.0, 100.0);
            b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.1, 0.5, 0.9), |_| {});
            b.path(shapes::ellipse(Rect::new(10.0, 10.0, 90.0, 90.0)), fill_only(Paint::solid(red)), |n| {
                n.opacity = opacity;
                n.blend = blend;
                n.appearance.items.retain(|i| matches!(i, AppearanceItem::Fill(_)));
            });
            render(&b.build())
        };
        let two = {
            let mut b = DocBuilder::new(100.0, 100.0);
            b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.1, 0.5, 0.9), |_| {});
            b.path(shapes::ellipse(Rect::new(10.0, 10.0, 90.0, 90.0)), fill_only(Paint::solid(red)), |n| {
                n.opacity = opacity;
                n.blend = blend;
                // An opaque green fill fully covered by the same red on top: the composite is red.
                n.appearance.items = vec![
                    AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.0, 1.0, 0.0)))),
                    AppearanceItem::Fill(FillLayer::new(Paint::solid(red))),
                ];
            });
            render(&b.build())
        };
        let st = diff(&one, &two);
        // Interior pixels within 2 levels; edges get a little slack for AA of two coverage passes.
        for (x, y) in [(50, 50), (30, 50), (50, 20), (70, 70)] {
            let (a, b) = (one.over_white(x, y), two.over_white(x, y));
            assert!((0..3).all(|c| a[c].abs_diff(b[c]) <= 2), "opacity {opacity} {blend:?} at ({x},{y}): {a:?} vs {b:?}");
        }
        assert!(st.mean < 0.5, "opacity {opacity} {blend:?}: {st:?}");
    }
}

/// Group opacity over a single child equals the child's own opacity.
#[test]
fn group_opacity_equals_child_opacity() {
    let child_op = {
        let mut b = DocBuilder::new(100.0, 100.0);
        b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::WHITE, |_| {});
        b.rect(Rect::new(20.0, 20.0, 80.0, 80.0), Color::rgb(0.0, 0.0, 0.0), |n| n.opacity = 0.4);
        render(&b.build())
    };
    let group_op = {
        let mut b = DocBuilder::new(100.0, 100.0);
        b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::WHITE, |_| {});
        let r = b.rect_node(Rect::new(20.0, 20.0, 80.0, 80.0), Color::rgb(0.0, 0.0, 0.0));
        let g = b.group(vec![r], false);
        b.doc.node_mut(g).unwrap().opacity = 0.4;
        render(&b.build())
    };
    assert_similar(&child_op, &group_op, 2.0, 0.0);
    assert_rgb(&group_op, 50, 50, [153, 153, 153], 3);
}

// ---------------------------------------------------------------- misc

#[test]
fn invisible_and_hidden_nodes_are_not_drawn() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::BLACK, |n| n.visible = false);
    assert_eq!(render(&b.build()).ink(), 0);
    let mut b = DocBuilder::new(100.0, 100.0);
    let id = b.rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::BLACK, |_| {});
    let d = b.build();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), hidden: vec![id], ..Default::default() };
    let r = vectorcraft_render::Renderer::new().render(&d, 100, 100, vectorcraft_geom::Affine::IDENTITY, &opts);
    assert_eq!(Image::from_rendered(&r).ink(), 0);
}

#[test]
fn outline_mode_draws_thin_black_edges_only() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.rect(Rect::new(20.0, 20.0, 80.0, 80.0), Color::rgb(1.0, 0.0, 0.0), |_| {});
    let d = b.build();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), outline: true, ..Default::default() };
    let img = Image::from_rendered(&vectorcraft_render::Renderer::new().render(&d, 100, 100, vectorcraft_geom::Affine::IDENTITY, &opts));
    assert_rgb(&img, 50, 50, [255, 255, 255], 2);
    assert_eq!(img.count(|p| p[0] > 200 && p[1] < 50), 0, "red paint visible in outline mode");
    assert!(img.ink() > 150 && img.ink() < 600, "outline ink {}", img.ink());
}

#[test]
fn png_encoding_roundtrips_pixels() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.rect(Rect::new(10.0, 10.0, 60.0, 60.0), Color::rgb(0.2, 0.4, 0.6), |n| n.opacity = 0.5);
    let d = b.build();
    let r = vectorcraft_render::Renderer::new().render_region(&d, Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, false);
    let decoded = Image::from_png(&r.to_png().unwrap()).unwrap();
    assert_eq!(decoded, Image::from_rendered(&r));
}

#[test]
fn renderer_reuse_matches_fresh_renderer() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.rect(Rect::new(10.0, 10.0, 60.0, 60.0), Color::rgb(0.2, 0.4, 0.6), |_| {});
    let d1 = b.build();
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::ellipse(Rect::new(30.0, 30.0, 90.0, 90.0)), fill_only(Paint::solid(Color::rgb(0.9, 0.2, 0.1))), |_| {});
    let d2 = b.build();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = vectorcraft_render::Renderer::new();
    let id = vectorcraft_geom::Affine::IDENTITY;
    let _ = r.render(&d1, 100, 100, id, &opts);
    let reused = Image::from_rendered(&r.render(&d2, 100, 100, id, &opts));
    assert_eq!(reused, render(&d2));
}

#[test]
fn scaled_render_matches_upscaled_geometry() {
    // Rendering at 2 px/pt equals rendering geometry scaled ×2 at 1 px/pt.
    let mut b = DocBuilder::new(50.0, 50.0);
    b.path(shapes::ellipse(Rect::new(5.0, 5.0, 45.0, 40.0)), fill_only(Paint::solid(Color::rgb(0.1, 0.6, 0.3))), |_| {});
    let d = b.build();
    let a = Image::from_rendered(&vectorcraft_render::Renderer::new().render_region(&d, Rect::new(0.0, 0.0, 50.0, 50.0), 2.0, true));
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::ellipse(Rect::new(10.0, 10.0, 90.0, 80.0)), fill_only(Paint::solid(Color::rgb(0.1, 0.6, 0.3))), |_| {});
    let c = Image::from_rendered(&vectorcraft_render::Renderer::new().render_region(&b.build(), Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, true));
    assert_similar(&a, &c, 2.0, 0.0);
    let _: Option<Node> = None;
}
