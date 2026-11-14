//! Expand Clipping Masks: a clipping group written as clipped geometry instead of `<clipPath>`,
//! so readers that ignore `<clipPath>` (Qt SVG behind KDE's thumbnails and viewers, some Office
//! and CAD importers) draw it as the app does.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Knockout, Node, NodeKind};
use vectorcraft_geom::{FillRule, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export, export_full};

fn fill(r: u8, g: u8, b: u8) -> Appearance {
    Appearance::basic(Paint::solid(Color::rgb8(r, g, b)), Paint::None, 0.0)
}

/// A fill of `kind` through red to blue, fitted to the object's box (no placed geometry).
fn gradient(kind: GradientKind) -> Appearance {
    let g = Gradient::new(kind, vec![GradientStop::new(0.0, Color::rgb8(255, 0, 0)), GradientStop::new(1.0, Color::rgb8(0, 0, 255))]);
    Appearance::basic(Paint::Gradient(Box::new(GradientPaint::new(g))), Paint::None, 0.0)
}

/// A fill at half opacity.
fn half(r: u8, g: u8, b: u8) -> Appearance {
    let mut a = fill(r, g, b);
    if let Some(f) = a.fill_mut() {
        f.opacity = 0.5;
    }
    a
}

/// A document whose clipping group clips a 60×60 red square (30,30)–(90,90) to the blue square
/// (10,10)–(50,50): most of the art lies outside the clip.
fn clipped_square(art: Appearance) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), fill(0, 0, 255));
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(30.0, 30.0, 90.0, 90.0)), art);
    let group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(art)], clip: true });
    d.insert(Some(layer), usize::MAX, group).expect("insert");
    d
}

/// The exported SVG rendered on transparent at 1 px/pt.
fn render(svg: &str) -> resvg::tiny_skia::Pixmap {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(100, 100).expect("pixmap");
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    pm
}

fn pixel(pm: &resvg::tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * 100 + x) * 4) as usize;
    let d = pm.data();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

/// The largest per-channel difference between two renders of the same document.
fn max_channel_diff(a: &resvg::tiny_skia::Pixmap, b: &resvg::tiny_skia::Pixmap) -> u8 {
    a.data().iter().zip(b.data().iter()).map(|(p, q)| p.abs_diff(*q)).max().unwrap_or(0)
}

#[test]
fn expanding_writes_clipped_geometry_without_a_clip_path() {
    let d = clipped_square(fill(255, 0, 0));
    let kept = export(&d, &ExportOptions::default());
    assert!(kept.contains("<clipPath"), "{kept}");
    assert!(kept.contains("clip-path=\"url(#"), "{kept}");

    let expanded = export(&d, &ExportOptions { expand_clips: true, ..Default::default() });
    assert!(!expanded.contains("<clipPath"), "{expanded}");
    assert!(!expanded.contains("clip-path"), "{expanded}");
    // The clipping path's fill still paints behind the art, inside the clip.
    assert!(expanded.contains("fill=\"#0000ff\""), "{expanded}");
    assert!(expanded.contains("fill=\"#ff0000\""), "{expanded}");
}

#[test]
fn an_expanded_clip_paints_where_the_clip_lets_it() {
    let d = clipped_square(fill(255, 0, 0));
    let expanded = render(&export(&d, &ExportOptions { expand_clips: true, ..Default::default() }));
    // Inside both the clip and the art.
    assert_eq!(pixel(&expanded, 40, 40), [255, 0, 0, 255]);
    // Inside the clip, where the art does not reach: the clipping path's fill.
    assert_eq!(pixel(&expanded, 15, 15), [0, 0, 255, 255]);
    // Inside the art but outside the clip: nothing.
    assert_eq!(pixel(&expanded, 60, 60), [0, 0, 0, 0]);
    // A spec renderer draws the `<clipPath>` the same way.
    let kept = render(&export(&d, &ExportOptions::default()));
    assert_eq!(kept.data(), expanded.data());
}

#[test]
fn art_a_clip_cannot_expand_stays_a_clip_path() {
    // A stroke is clipped by its outline, which this option does not make: the group keeps its
    // `<clipPath>` and says so.
    let stroked = Appearance::basic(Paint::solid(Color::rgb8(255, 0, 0)), Paint::solid(Color::BLACK), 2.0);
    let d = clipped_square(stroked);
    let out = export_full(&d, &ExportOptions { expand_clips: true, ..Default::default() }, None);
    assert!(out.svg.contains("<clipPath"), "{}", out.svg);
    assert!(out.svg.contains("clip-path=\"url(#"), "{}", out.svg);
    assert!(out.warnings.iter().any(|w| w.contains("Expand Clipping Masks")), "{:?}", out.warnings);
}

#[test]
fn groups_inside_a_clip_group_expand_too() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), fill(0, 0, 255));
    let inner = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 90.0, 90.0)), fill(255, 0, 0));
    let group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(inner)], clip: false });
    let outer = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(group)], clip: true });
    d.insert(Some(layer), usize::MAX, outer).expect("insert");

    let svg = export(&d, &ExportOptions { expand_clips: true, ..Default::default() });
    assert!(!svg.contains("clip-path"), "{svg}");
    let pm = render(&svg);
    assert_eq!(pixel(&pm, 40, 40), [255, 0, 0, 255]);
    assert_eq!(pixel(&pm, 70, 70), [0, 0, 0, 0]);
}

#[test]
fn an_expanded_gradient_keeps_its_placement() {
    for kind in [GradientKind::Linear, GradientKind::Radial] {
        let d = clipped_square(gradient(kind));
        let expanded = export(&d, &ExportOptions { expand_clips: true, ..Default::default() });
        let kept = export(&d, &ExportOptions::default());
        assert!(!expanded.contains("<clipPath"), "{kind:?}: {expanded}");
        // Intersecting the art must hide the part outside the clip, not restretch the gradient
        // over the part that is left.
        let diff = max_channel_diff(&render(&expanded), &render(&kept));
        assert!(diff <= 1, "{kind:?}: the clipped geometry differs from the clip path by {diff}");
    }
}

#[test]
fn an_expanded_compound_keeps_its_gradient_placement() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), fill(0, 0, 255));
    let top = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(30.0, 30.0, 90.0, 60.0)), Appearance::default());
    let bottom = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(30.0, 60.0, 90.0, 90.0)), Appearance::default());
    let mut compound = Node::new(d.alloc_id(), NodeKind::Compound { children: vec![Arc::new(top), Arc::new(bottom)], rule: FillRule::NonZero });
    compound.appearance = gradient(GradientKind::Linear).into();
    let group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(compound)], clip: true });
    d.insert(Some(layer), usize::MAX, group).expect("insert");

    let expanded = export(&d, &ExportOptions { expand_clips: true, ..Default::default() });
    let kept = export(&d, &ExportOptions::default());
    assert!(!expanded.contains("<clipPath"), "{expanded}");
    let diff = max_channel_diff(&render(&expanded), &render(&kept));
    assert!(diff <= 1, "the clipped compound differs from the clip path by {diff}");
}

#[test]
fn a_fill_with_an_effect_keeps_its_clip_path() {
    // A blur paints outside the trimmed shape, so the geometry cannot stand in for the clip.
    let mut art = fill(255, 0, 0);
    art.fill_mut().expect("fill").effects.push(Effect { id: "blur.gaussian".into(), params: json!({ "radius": 4.0 }), visible: true });
    let d = clipped_square(art);
    let out = export_full(&d, &ExportOptions { expand_clips: true, ..Default::default() }, None);
    assert!(out.svg.contains("<clipPath"), "{}", out.svg);
    assert!(out.svg.contains("clip-path=\"url(#"), "{}", out.svg);
    assert!(out.warnings.iter().any(|w| w.contains("Expand Clipping Masks")), "{:?}", out.warnings);
}

#[test]
fn a_dithered_gradient_keeps_its_clip_path() {
    // A dithered fill is written as a raster image behind an internal `<clipPath>`: a reader that
    // ignores clip paths would show the image's box.
    let mut g =
        Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::rgb8(255, 0, 0)), GradientStop::new(1.0, Color::rgb8(0, 0, 255))]);
    g.dither = true;
    let art = Appearance::basic(Paint::Gradient(Box::new(GradientPaint::new(g))), Paint::None, 0.0);
    let d = clipped_square(art);
    let out = export_full(&d, &ExportOptions { expand_clips: true, ..Default::default() }, None);
    assert!(out.svg.contains("<clipPath"), "{}", out.svg);
    assert!(out.warnings.iter().any(|w| w.contains("Expand Clipping Masks")), "{:?}", out.warnings);
}

#[test]
fn a_knockout_clip_group_keeps_its_clip_path() {
    // Written directly, overlapping children would show through each other instead of knocking
    // each other out.
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), fill(0, 0, 255));
    let first = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)), fill(255, 0, 0));
    let second = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(40.0, 40.0, 90.0, 90.0)), fill(0, 255, 0));
    let mut group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(first), Arc::new(second)], clip: true });
    group.knockout = Knockout::On;
    d.insert(Some(layer), usize::MAX, group).expect("insert");

    let out = export_full(&d, &ExportOptions { expand_clips: true, ..Default::default() }, None);
    assert!(out.svg.contains("<clipPath"), "{}", out.svg);
    assert!(out.warnings.iter().any(|w| w.contains("Expand Clipping Masks")), "{:?}", out.warnings);
}

#[test]
fn an_off_clip_group_inside_a_knockout_parent_composites_normally() {
    // The group is `Off`, so its children composite normally even though the parent knocks out;
    // written while the parent's knockout state is still in force, their overlaps would be masked.
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), fill(0, 0, 255));
    let first = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 70.0, 70.0)), half(255, 0, 0));
    let second = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(40.0, 40.0, 90.0, 90.0)), half(0, 255, 0));
    let subgroup = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(first), Arc::new(second)], clip: false });
    let mut group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(subgroup)], clip: true });
    group.knockout = Knockout::Off;
    let mut outer = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(group)], clip: false });
    outer.knockout = Knockout::On;
    d.insert(Some(layer), usize::MAX, outer).expect("insert");

    let expanded = export(&d, &ExportOptions { expand_clips: true, ..Default::default() });
    let kept = export(&d, &ExportOptions::default());
    assert!(!expanded.contains("<clipPath"), "{expanded}");
    let diff = max_channel_diff(&render(&expanded), &render(&kept));
    assert!(diff <= 1, "the Off clip group's children composite normally inside a knockout parent: {diff}");
}
