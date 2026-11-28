//! Headless frames of the Document Info, Info, Image Trace and Links panels: values line up, groups stand apart from
//! their dividers, every check box shows its label, and an empty Links panel says how to bring an image in.

use egui::Rect;
use serde_json::json;

use super::tests_appearance::{app_with_rect, run};
use super::*;
use crate::tests::panel_harness::{Panel, ROOM};

/// The first run the latest frame painted that starts with `prefix`.
fn run_starting(panel: &Panel, prefix: &str) -> Rect {
    let texts = panel.texts();
    texts.iter().find(|(t, _)| t.starts_with(prefix)).map(|(_, r)| *r).unwrap_or_else(|| panic!("no run starting {prefix:?} in {texts:?}"))
}

/// The single run of `text` the latest frame painted.
fn one(panel: &Panel, text: &str) -> Rect {
    let rects = panel.painted(text);
    assert_eq!(rects.len(), 1, "{text:?} painted {} times in {:?}", rects.len(), panel.texts());
    rects[0]
}

#[test]
fn document_info_values_end_at_one_edge() {
    let mut panel = Panel::new(app_with_rect(), doc_info::show);
    panel.frame();
    let ends: Vec<f32> = ["Untitled-1", "RGB", "Points"].iter().map(|v| one(&panel, v).right()).collect();
    for end in &ends {
        assert!((end - ends[0]).abs() < 1.0, "the values end at {ends:?}, not at one edge");
    }
    let (label, value) = (one(&panel, "Color Mode"), one(&panel, "RGB"));
    assert!(value.left() > label.right() + label.width(), "the value {value:?} sits against its label {label:?}");
}

#[test]
fn info_paints_stand_a_section_apart_from_the_numbers() {
    let mut app = app_with_rect();
    run(&mut app, "shape.rectangle", json!({"x": 150, "y": 60, "width": 80, "height": 50}));
    run(&mut app, "select.all", json!({}));
    let mut panel = Panel::new(app, info::show);
    panel.frame();
    let numbers = run_starting(&panel, "D:");
    let paint = run_starting(&panel, "R:");
    let gap = paint.top() - numbers.bottom();
    assert!(gap >= numbers.height(), "the paints start {gap} below the numbers, closer than a line");
    // The two columns split the panel in halves: the right column starts at the panel's middle.
    let (x, w) = (run_starting(&panel, "X:"), run_starting(&panel, "W:"));
    let half = (ROOM.x - x.left()) / 2.0;
    assert!(w.left() - x.left() >= half - 1.0, "W: starts {} in, before the half at {half}", w.left() - x.left());
}

#[test]
fn image_trace_create_shows_fills_and_strokes() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    app.ui.image_trace_advanced = true;
    let mut panel = Panel::new(app, image_trace::show);
    panel.frame();
    let create = one(&panel, "Create:");
    for label in ["Fills", "Strokes"] {
        let at = one(&panel, label);
        assert!(at.left() > create.right() && (at.center().y - create.center().y).abs() < create.height(), "{label} at {at:?}");
    }
}

#[test]
fn empty_links_panel_says_how_to_bring_an_image_in() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let mut panel = Panel::new(app, links::show);
    panel.frame();
    one(&panel, "No images");
    one(&panel, "File › Place… brings in an image, linked or embedded.");
}
