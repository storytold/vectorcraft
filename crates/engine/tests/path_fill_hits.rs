//! Path fill hit testing follows the visible paint stack while transparent paint remains selectable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::hit::{HitKind, HitOptions, hit_test};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, FillRule, Point, Rect, shapes};
use vectorcraft_render::{RenderOptions, Renderer};

fn fill(paint: Paint) -> FillLayer {
    FillLayer::new(paint)
}

fn rectangle(fills: Vec<FillLayer>, stroke: bool) -> Document {
    let mut doc = Document::new(128.0, 128.0);
    let layer = doc.layers[0].id;
    let id = doc.alloc_id();
    let mut appearance = Appearance::default();
    appearance.items.extend(fills.into_iter().map(AppearanceItem::Fill));
    if stroke {
        appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 2.0)));
    }
    let path = shapes::rectangle(Rect::new(20.0, 20.0, 100.0, 100.0));
    doc.insert(Some(layer), usize::MAX, Node::path(id, path, appearance)).unwrap();
    doc
}

fn compound(fills: Vec<FillLayer>) -> Document {
    let mut doc = Document::new(128.0, 128.0);
    let layer = doc.layers[0].id;
    let outer_id = doc.alloc_id();
    let inner_id = doc.alloc_id();
    let compound_id = doc.alloc_id();
    let children = vec![
        Arc::new(Node::path(outer_id, shapes::rectangle(Rect::new(20.0, 20.0, 100.0, 100.0)), Appearance::default())),
        Arc::new(Node::path(inner_id, shapes::rectangle(Rect::new(50.0, 40.0, 70.0, 80.0)), Appearance::default())),
    ];
    let mut appearance = Appearance::default();
    appearance.items.extend(fills.into_iter().map(AppearanceItem::Fill));
    appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 2.0)));
    let mut node = Node::new(compound_id, NodeKind::Compound { children, rule: FillRule::EvenOdd });
    node.appearance = appearance.into();
    doc.insert(Some(layer), usize::MAX, node).unwrap();
    doc
}

#[test]
fn hit_uses_the_visible_fill_stack() {
    let black = || fill(Paint::solid(Color::BLACK));

    let mut hidden = black();
    hidden.visible = false;
    let mut transparent = black();
    transparent.opacity = 0.0;
    let mut hidden_lower = black();
    hidden_lower.visible = false;

    let cases = vec![
        ("visible black fill", rectangle(vec![black()], true)),
        ("lower black fill under a top None fill", rectangle(vec![black(), fill(Paint::None)], true)),
        ("lower black fill under a top None fill without stroke", rectangle(vec![black(), fill(Paint::None)], false)),
        ("only hidden non-None fill", rectangle(vec![hidden], true)),
        ("basic Fill None", rectangle(vec![fill(Paint::None)], true)),
        ("hidden lower black and top None fills", rectangle(vec![hidden_lower, fill(Paint::None)], true)),
        ("visible fill at zero opacity", rectangle(vec![transparent], true)),
    ];

    let mut observed = Vec::new();
    for (name, doc) in cases {
        let image = Renderer::new().render(
            &doc,
            128,
            128,
            Affine::IDENTITY,
            &RenderOptions { background: None, artboards: false, ..RenderOptions::default() },
        );
        let center_alpha = image.pixel(60, 60)[3];
        let edge_alpha = image.pixel(100, 60)[3];
        let center = hit_test(&doc, Point::new(60.0, 60.0), HitOptions::default()).map(|hit| hit.kind);
        let edge = hit_test(&doc, Point::new(100.5, 60.0), HitOptions::default()).map(|hit| hit.kind);
        let outline = hit_test(&doc, Point::new(60.0, 60.0), HitOptions { outline: true, ..HitOptions::default() }).map(|hit| hit.kind);
        let path_only = hit_test(&doc, Point::new(60.0, 60.0), HitOptions { path_only: true, ..HitOptions::default() }).map(|hit| hit.kind);
        observed.push((name, center_alpha, edge_alpha, center, edge, outline, path_only));
    }

    let expected = vec![
        ("visible black fill", 255, 255, Some(HitKind::Fill), Some(HitKind::Stroke), None, None),
        ("lower black fill under a top None fill", 255, 255, Some(HitKind::Fill), Some(HitKind::Stroke), None, None),
        ("lower black fill under a top None fill without stroke", 255, 0, Some(HitKind::Fill), Some(HitKind::Outline), None, None),
        ("only hidden non-None fill", 0, 255, None, Some(HitKind::Stroke), None, None),
        ("basic Fill None", 0, 255, None, Some(HitKind::Stroke), None, None),
        ("hidden lower black and top None fills", 0, 255, None, Some(HitKind::Stroke), None, None),
        ("visible fill at zero opacity", 0, 255, Some(HitKind::Fill), Some(HitKind::Stroke), None, None),
    ];
    assert_eq!(observed, expected, "render and hit-test observations by fill-stack case");
}

#[test]
fn compound_hit_uses_visible_fill_stack() {
    let black = || fill(Paint::solid(Color::BLACK));

    let mut hidden = black();
    hidden.visible = false;
    let mut transparent = black();
    transparent.opacity = 0.0;

    let cases = vec![
        ("visible black fill", compound(vec![black()])),
        ("lower black fill under a top None fill", compound(vec![black(), fill(Paint::None)])),
        ("only hidden non-None fill", compound(vec![hidden])),
        ("basic Fill None", compound(vec![fill(Paint::None)])),
        ("visible fill at zero opacity", compound(vec![transparent])),
    ];

    let mut observed = Vec::new();
    for (name, doc) in cases {
        let image = Renderer::new().render(
            &doc,
            128,
            128,
            Affine::IDENTITY,
            &RenderOptions { background: None, artboards: false, ..RenderOptions::default() },
        );
        let ring_alpha = image.pixel(30, 60)[3];
        let hole_alpha = image.pixel(60, 60)[3];
        let edge_alpha = image.pixel(100, 60)[3];
        let ring = hit_test(&doc, Point::new(30.0, 60.0), HitOptions::default()).map(|hit| hit.kind);
        let hole = hit_test(&doc, Point::new(60.0, 60.0), HitOptions::default()).map(|hit| hit.kind);
        let edge = hit_test(&doc, Point::new(100.5, 60.0), HitOptions::default()).map(|hit| hit.kind);
        let outline = hit_test(&doc, Point::new(30.0, 60.0), HitOptions { outline: true, ..HitOptions::default() }).map(|hit| hit.kind);
        let path_only = hit_test(&doc, Point::new(30.0, 60.0), HitOptions { path_only: true, ..HitOptions::default() }).map(|hit| hit.kind);
        observed.push((name, ring_alpha, hole_alpha, edge_alpha, ring, hole, edge, outline, path_only));
    }

    let expected = vec![
        ("visible black fill", 255, 0, 255, Some(HitKind::Fill), None, Some(HitKind::Stroke), None, None),
        ("lower black fill under a top None fill", 255, 0, 255, Some(HitKind::Fill), None, Some(HitKind::Stroke), None, None),
        ("only hidden non-None fill", 0, 0, 255, None, None, Some(HitKind::Stroke), None, None),
        ("basic Fill None", 0, 0, 255, None, None, Some(HitKind::Stroke), None, None),
        ("visible fill at zero opacity", 0, 0, 255, Some(HitKind::Fill), None, Some(HitKind::Stroke), None, None),
    ];
    assert_eq!(observed, expected, "render and hit-test observations by Compound fill-stack case");
}
