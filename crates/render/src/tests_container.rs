//! The appearance of groups, layers and type: a container's own fills and strokes paint its
//! members' geometry below or above them (the contents slot), its effects apply to the composite
//! (one combined shadow) and reshape its members as one piece; type's own fills and strokes paint
//! under or over its characters, reshaped by their geometry effects.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, FillLayer, ImageBlob, ImageObject, Knockout, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{Point, shapes};

use super::*;

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

fn solid(r: f32, g: f32, b: f32) -> Paint {
    Paint::solid(Color::rgb(r, g, b))
}

/// A rectangle filled with `paint` and no stroke.
fn rect(d: &mut Document, r: Rect, paint: Paint) -> Arc<Node> {
    Arc::new(Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(paint, Paint::None, 0.0)))
}

/// A 100×100 document holding a group of a red square (10..40) and a blue one (50..80), on a row
/// from y 10 to 40, given its own appearance by `style`.
fn group_doc(style: impl FnOnce(&mut Node)) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 40.0, 40.0), solid(1.0, 0.0, 0.0));
    let b = rect(&mut d, Rect::new(50.0, 10.0, 80.0, 40.0), solid(0.0, 0.0, 1.0));
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    style(&mut g);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    d
}

fn render_threads(d: &Document, threads: u16) -> Rendered {
    let mut r = Renderer::new();
    r.threads = threads;
    r.render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn render(d: &Document) -> Rendered {
    render_threads(d, 0)
}

const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn green_fill(g: &mut Node) {
    g.appearance.items.push(AppearanceItem::Fill(FillLayer::new(solid(0.0, 1.0, 0.0))));
}

#[test]
fn a_group_fill_paints_every_member_above_or_below_them() {
    // Above the contents (the default): both members turn green; the gap between them stays white.
    let img = render(&group_doc(green_fill));
    assert_eq!((img.pixel(25, 25), img.pixel(65, 25), img.pixel(45, 25)), (GREEN, GREEN, WHITE));
    // Below the contents the members' own fills cover it.
    let img = render(&group_doc(|g| {
        green_fill(g);
        g.appearance.set_contents_at(1);
    }));
    assert_eq!((img.pixel(25, 25), img.pixel(65, 25)), (RED, BLUE));
    // A group stroke strokes each member's outline.
    let img = render(&group_doc(|g| {
        g.appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(solid(0.0, 1.0, 0.0), 4.0)));
    }));
    assert_eq!((img.pixel(10, 25), img.pixel(80, 25), img.pixel(25, 25)), (GREEN, GREEN, RED));
    // Hidden fills paint nothing.
    let img = render(&group_doc(|g| {
        green_fill(g);
        if let AppearanceItem::Fill(f) = &mut g.appearance.items[0] {
            f.visible = false;
        }
    }));
    assert_eq!(img.pixel(25, 25), RED);
}

#[test]
fn a_group_fill_paints_the_underline_of_its_type() {
    // The group holds underlined black "IIII" at 40 pt with its baseline at y 60, and has a green fill.
    let mut d = Document::new(100.0, 100.0);
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), underline: true, ..Default::default() };
    let t = Arc::new(Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style)))));
    let mut g = Node::group(d.alloc_id(), vec![t]);
    green_fill(&mut g);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    // The glyphs have no ink below the baseline: the bar's pixels there are green, none black.
    let img = render(&d);
    let bar: Vec<[u8; 4]> = (61..70).flat_map(|y| (0..100).map(move |x| (x, y))).map(|(x, y)| img.pixel(x, y)).filter(|p| p[0] < 160).collect();
    assert!(bar.len() > 50 && bar.iter().all(|p| p[1] > p[0] + 20 && p[1] > p[2] + 20), "{bar:?}");
}

#[test]
fn a_group_fill_with_opacity_composites_as_one_layer() {
    // Two overlapping members under a 50 % group fill: the overlap is as light as the rest.
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 60.0, 40.0), Paint::None);
    let b = rect(&mut d, Rect::new(40.0, 10.0, 90.0, 40.0), Paint::None);
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    let mut f = FillLayer::new(Paint::solid(Color::BLACK));
    f.opacity = 0.5;
    g.appearance.items.push(AppearanceItem::Fill(f));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let img = render(&d);
    let (single, both) = (img.pixel(20, 25), img.pixel(50, 25));
    assert!(single[0].abs_diff(128) <= 2 && single == both, "{single:?} vs {both:?}");
}

#[test]
fn a_group_shadow_is_one_combined_shadow() {
    // The members' shadows (offset 30 pt right) overlap at x 65 but not at x 80; one combined
    // shadow is equally dark at both, two separate ones would double up at 65.
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 40.0, 40.0), Paint::solid(Color::BLACK));
    let b = rect(&mut d, Rect::new(30.0, 10.0, 60.0, 40.0), Paint::solid(Color::BLACK));
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    g.appearance.effects.push(fx("stylize.dropShadow", json!({"x": 30, "y": 0, "blur": 0, "opacity": 50, "mode": "normal"})));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    for threads in [0, 3] {
        let img = render_threads(&d, threads);
        let (overlap, single) = (img.pixel(65, 25), img.pixel(80, 25));
        assert!(single[0] < 160 && single[0] > 100, "{single:?}");
        assert!(overlap[0].abs_diff(single[0]) <= 2, "one shadow, not two: {overlap:?} vs {single:?}");
        assert_eq!(img.pixel(95, 25), WHITE);
    }
}

#[test]
fn group_geometry_effects_move_the_members_as_one_piece() {
    let img = render(&group_doc(|g| g.appearance.effects.push(fx("distort.transform", json!({"moveV": 50})))));
    assert_eq!((img.pixel(25, 25), img.pixel(25, 75), img.pixel(65, 75)), (WHITE, RED, BLUE));
    // The group's own fill paints the moved members, and the shadow follows them.
    let img = render(&group_doc(|g| {
        g.appearance.effects.push(fx("distort.transform", json!({"moveV": 50})));
        green_fill(g);
    }));
    assert_eq!((img.pixel(25, 75), img.pixel(65, 75)), (GREEN, GREEN));
}

/// #2023: a Transform effect on a group moves and copies its images as it does its paths (the
/// image's placement takes the map, and every copy is drawn at its own place).
#[test]
fn group_geometry_effects_move_and_copy_images_too() {
    let mut d = Document::new(100.0, 100.0);
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let b = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), solid(0.0, 0.0, 1.0));
    let im = ImageObject {
        key: "red".into(),
        width: 30,
        height: 30,
        xf: Affine::translate((10.0, 30.0)),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let image = Arc::new(Node::new(d.alloc_id(), NodeKind::Image(Box::new(im))));
    let mut g = Node::group(d.alloc_id(), vec![b, image]);
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 50, "copies": 1})));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let img = render(&d);
    assert_eq!(img.pixel(25, 45), RED, "the image itself");
    assert_eq!(img.pixel(75, 45), RED, "its copy, 50 pt to the right");
    assert_eq!(img.pixel(60, 10), BLUE, "the path's copy");
    assert_eq!(img.pixel(5, 10), BLUE, "the path itself");
}

/// A 100×100 document holding a group of a red square (10..90) and black 40 pt type "IIII" over
/// it, the type changed by `ty` and the group changed by `group`.
fn type_group_doc(ty: impl FnOnce(&mut Node), group: impl FnOnce(&mut Node)) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 90.0, 90.0), solid(1.0, 0.0, 0.0));
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    ty(&mut t);
    let mut g = Node::group(d.alloc_id(), vec![a, Arc::new(t)]);
    group(&mut g);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    d
}

#[test]
fn type_in_a_group_keeps_its_transparency_and_visibility_under_the_groups_geometry_effects() {
    // Offset Path 0 leaves every path as it is, so the group is drawn the same with it as without
    // it, for type at 50 %, hidden type, and type at 50 % defining its knockout shape in a
    // knockout group.
    let half: fn(&mut Node) = |t| t.opacity = 0.5;
    let hidden: fn(&mut Node) = |t| t.visible = false;
    let shape: fn(&mut Node) = |t| (t.opacity, t.knockout_shape) = (0.5, true);
    let plain: fn(&mut Node) = |_| {};
    let knockout: fn(&mut Node) = |g| g.knockout = Knockout::On;
    for (case, ty, group) in [("50 %", half, plain), ("hidden", hidden, plain), ("knockout shape", shape, knockout)] {
        let without = render(&type_group_doc(ty, group));
        let with = render(&type_group_doc(ty, |g| {
            group(g);
            g.appearance.effects.push(fx("path.offsetPath", json!({"offset": 0})));
        }));
        let worst = without.pixels.iter().zip(&with.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 4, "{case}: max channel difference {worst}");
    }
}

#[test]
fn type_in_a_clipping_path_clips_alike_under_the_groups_geometry_effects() {
    // The red square is clipped by type with a Transform of its own, which the clip leaves out,
    // then by a group of that type and a small square, with the type shown and hidden. Offset
    // Path 0 on the clip group leaves every path as it is.
    let doc = |effect: bool, grouped: bool, hidden: bool| {
        let mut d = Document::new(100.0, 100.0);
        let style = CharStyle { size: 40.0, fill: Paint::None, ..Default::default() };
        let mut t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
        t.appearance.effects.push(fx("distort.transform", json!({"moveH": 30})));
        t.visible = !hidden;
        let mut clip = Arc::new(t);
        if grouped {
            let small = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), Paint::None);
            clip = Arc::new(Node::group(d.alloc_id(), vec![clip, small]));
        }
        let a = rect(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), solid(1.0, 0.0, 0.0));
        let mut g = Node::new(d.alloc_id(), NodeKind::Group { children: vec![clip, a], clip: true });
        if effect {
            g.appearance.effects.push(fx("path.offsetPath", json!({"offset": 0})));
        }
        let l = d.layers[0].id;
        d.insert(Some(l), 0, g).unwrap();
        d
    };
    for (grouped, hidden) in [(false, false), (true, false), (true, true)] {
        let (without, with) = (render(&doc(false, grouped, hidden)), render(&doc(true, grouped, hidden)));
        let worst = without.pixels.iter().zip(&with.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 4, "grouped {grouped}, hidden {hidden}: max channel difference {worst}");
    }
}

#[test]
fn a_layer_paints_its_own_fill_and_a_knockout_group_keeps_knocking_out() {
    let mut d = group_doc(|_| {});
    let l = Arc::make_mut(&mut d.layers[0]);
    l.appearance.items.push(AppearanceItem::Fill(FillLayer::new(solid(0.0, 1.0, 0.0))));
    let img = render(&d);
    assert_eq!((img.pixel(25, 25), img.pixel(65, 25)), (GREEN, GREEN));

    // A knockout group with a drop shadow: its half-transparent members still knock each other
    // out (the overlap shows the top one over the backdrop), and it casts its shadow.
    let mut d = Document::new(100.0, 100.0);
    let half = |d: &mut Document, r: Rect, p: Paint| {
        let mut n = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(p, Paint::None, 0.0));
        n.opacity = 0.5;
        Arc::new(n)
    };
    let a = half(&mut d, Rect::new(10.0, 10.0, 50.0, 40.0), solid(1.0, 0.0, 0.0));
    let b = half(&mut d, Rect::new(30.0, 10.0, 70.0, 40.0), solid(0.0, 0.0, 1.0));
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    g.knockout = Knockout::On;
    g.appearance.effects.push(fx("stylize.dropShadow", json!({"x": 0, "y": 40, "blur": 0, "opacity": 100, "mode": "normal"})));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let img = render(&d);
    let overlap = img.pixel(40, 25);
    assert!(overlap[0] > 120 && overlap[2] > 250, "the blue knocks the red out: {overlap:?}");
    // The shadow of the composite: as opaque as the knocked-out art (half), the overlap too.
    assert!(img.pixel(40, 65)[0].abs_diff(127) <= 2 && img.pixel(20, 65) == img.pixel(40, 65), "the shadow: {:?}", img.pixel(40, 65));
}

/// "IIII" in black at 40 pt with its own red fill; `below`: under the characters.
fn type_doc(below: bool) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    n.appearance.items.push(AppearanceItem::Fill(FillLayer::new(solid(1.0, 0.0, 0.0))));
    if below {
        n.appearance.set_contents_at(1);
    }
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn red_pixels(img: &Rendered) -> usize {
    (0..100).flat_map(|y| (0..100).map(move |x| (x, y))).filter(|&(x, y)| matches!(img.pixel(x, y), [r, g, _, _] if r > 200 && g < 60)).count()
}

#[test]
fn a_fill_below_the_characters_is_hidden_by_the_glyphs() {
    assert!(red_pixels(&render(&type_doc(false))) > 50, "above: the red fill covers the glyphs");
    assert_eq!(red_pixels(&render(&type_doc(true))), 0, "below: the black glyphs cover it");
}

/// "IIII" in black at 40 pt with its own red 2 pt stroke under the characters. The stroke carries
/// the effects `row`, and the type carries the effects `object`.
fn stroked_type_doc(row: &[Effect], object: &[Effect]) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    let mut stroke = StrokeLayer::new(solid(1.0, 0.0, 0.0), 2.0);
    stroke.effects = row.to_vec();
    n.appearance.items.push(AppearanceItem::Stroke(stroke));
    n.appearance.set_contents_at(1);
    n.appearance.effects = object.to_vec();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// The red pixels more than 2 px outside the box of the black ones.
fn red_past_the_glyphs(img: &Rendered) -> usize {
    let all = || (0..100).flat_map(|y| (0..100).map(move |x| (x, y)));
    let black: Vec<(u32, u32)> = all().filter(|&(x, y)| img.pixel(x, y)[..3].iter().all(|c| *c < 60)).collect();
    let (x0, x1) = (black.iter().map(|p| p.0).min().unwrap(), black.iter().map(|p| p.0).max().unwrap());
    let (y0, y1) = (black.iter().map(|p| p.1).min().unwrap(), black.iter().map(|p| p.1).max().unwrap());
    let past = |x: u32, y: u32| x + 2 < x0 || x > x1 + 2 || y + 2 < y0 || y > y1 + 2;
    all().filter(|&(x, y)| past(x, y) && matches!(img.pixel(x, y), [r, g, _, _] if r > 200 && g < 60)).count()
}

#[test]
fn an_offset_path_on_the_types_own_stroke_draws_the_offset_outline() {
    assert_eq!(red_past_the_glyphs(&render(&stroked_type_doc(&[], &[]))), 0, "the stroke without the effect stays at the glyphs");
    let offset = [fx("path.offsetPath", json!({"offset": 4}))];
    assert!(red_past_the_glyphs(&render(&stroked_type_doc(&offset, &[]))) > 50, "the stroke runs 4 pt outside the glyphs");
}

#[test]
fn a_shadow_on_the_type_keeps_the_offset_outline_of_its_own_stroke() {
    let offset = [fx("path.offsetPath", json!({"offset": 4}))];
    let shadow = [fx("stylize.dropShadow", json!({"x": 0, "y": 30, "blur": 0, "opacity": 50, "mode": "normal"}))];
    assert!(red_past_the_glyphs(&render(&stroked_type_doc(&offset, &shadow))) > 50);
}

#[test]
fn the_offset_outline_of_the_types_own_stroke_is_drawn_when_only_it_is_in_view() {
    // The view's right edge is 5 pt left of the glyphs, so only the stroke 8 pt outside them
    // reaches into it. The second case also gives the type object Offset Path 0, which leaves its
    // outlines as they are.
    let offset = [fx("path.offsetPath", json!({"offset": 8}))];
    for object in [vec![], vec![fx("path.offsetPath", json!({"offset": 0}))]] {
        let d = stroked_type_doc(&offset, &object);
        let glyphs = effects::outline_text(&d.layers[0].children().unwrap()[0]).unwrap().geometric_bounds().unwrap();
        let view = Affine::translate((105.0 - glyphs.x0, 0.0));
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let img = Renderer::new().render(&d, 100, 100, view, &opts);
        assert!(red_pixels(&img) > 20, "{} object effects: {} red pixels", object.len(), red_pixels(&img));
    }
}
