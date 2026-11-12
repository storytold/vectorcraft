//! Expand Clipping Masks: a clipping group written as clipped geometry instead of `<clipPath>`,
//! so readers that ignore `<clipPath>` (Qt SVG behind KDE's thumbnails and viewers, some Office
//! and CAD importers) draw it as the app does.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeKind};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export, export_full};

fn fill(r: u8, g: u8, b: u8) -> Appearance {
    Appearance::basic(Paint::solid(Color::rgb8(r, g, b)), Paint::None, 0.0)
}

/// A document whose clipping group clips a 60×60 red square (30,30)–(90,90) to the blue square
/// (10,10)–(50,50): three quarters of the art lie outside the clip.
fn clipped_square(art: Appearance) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), fill(0, 0, 255));
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(30.0, 30.0, 50.0, 50.0)), art);
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
