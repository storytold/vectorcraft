//! Where the parameter dialogs draw their rows: option rows stand beside or under the choice they belong to, and
//! every check box and picker shows its name.

use egui::{Pos2, Rect};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

type Window = egui_kittest::Harness<'static, VectorcraftApp>;

/// A document with one selected rectangle, in a window that runs whole app frames.
fn window() -> Window {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 120, "height": 90})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::testing::install(ui.ctx());
                return;
            }
            app.tick(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    let root = h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default();
    root.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 900.0)));
    root.focused = Some(true);
    for _ in 0..2 {
        step(&mut h);
    }
    h
}

/// One frame as the hosts run the window's; the commands it fired run as the next frame begins.
fn step(h: &mut Window) {
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    app.adopt_context(&ctx);
    craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
    h.step();
    crate::testing::run_fired(h.state_mut(), &ctx);
}

/// The texts the window paints with `command` run, each with where it was drawn, a field label's colon left off.
fn texts(command: &str, params: Value) -> Vec<(String, Rect)> {
    fn collect(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().trim().trim_end_matches(':').to_owned(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut h = window();
    crate::menus::invoke(h.state_mut(), command, params);
    for _ in 0..6 {
        step(&mut h);
    }
    let mut out = vec![];
    h.output().shapes.iter().for_each(|c| collect(&c.shape, &mut out));
    out
}

/// Where each painted `label` was drawn, top to bottom.
fn all(texts: &[(String, Rect)], label: &str) -> Vec<Rect> {
    let mut found: Vec<Rect> = texts.iter().filter(|(t, _)| t == label).map(|(_, r)| *r).collect();
    found.sort_by(|a, b| a.top().total_cmp(&b.top()));
    found
}

/// The one painted `label`.
fn one(texts: &[(String, Rect)], label: &str) -> Rect {
    let found = all(texts, label);
    assert_eq!(found.len(), 1, "`{label}` drawn {} times in {:?}", found.len(), texts.iter().map(|t| &t.0).collect::<Vec<_>>());
    found[0]
}

/// `b` stands on `a`'s line, to its right.
fn beside(a: Rect, b: Rect) -> bool {
    (a.center().y - b.center().y).abs() < a.height() / 2.0 && b.left() > a.right()
}

/// The Transform effect names its Reflect X, Reflect Y and Random check boxes.
#[test]
fn the_transform_effect_names_its_reflect_options() {
    let shown = texts("effect.dialog", json!({"effect": "distort.transform"}));
    for label in ["Reflect X", "Reflect Y", "Random"] {
        one(&shown, label);
    }
}

/// Transform Each names its reference point picker.
#[test]
fn transform_each_names_its_reference_point() {
    let shown = texts("object.transformEach", json!({}));
    one(&shown, "Reference Point");
}

/// Roughen's Relative and Absolute stand on one line, and so do Smooth and Corner.
#[test]
fn roughen_puts_each_choice_on_one_line() {
    let shown = texts("effect.dialog", json!({"effect": "distort.roughen"}));
    assert!(beside(one(&shown, "Relative"), one(&shown, "Absolute")), "Relative, then Absolute on its line");
    // The Properties panel also shows a Corner row; the dialog's Corner is the one on Smooth's line.
    let smooth = one(&shown, "Smooth");
    assert!(all(&shown, "Corner").into_iter().any(|corner| beside(smooth, corner)), "Smooth, then Corner on its line");
}

/// The Corners dialog's Relative and Absolute stand on one line.
#[test]
fn corners_puts_relative_and_absolute_on_one_line() {
    let shown = texts("ui.corners", json!({}));
    assert!(beside(one(&shown, "Relative"), one(&shown, "Absolute")), "Relative, then Absolute on its line");
}

/// Drop Shadow's color row stands under the Color choice, before Darkness, and the Darkness row under Darkness.
#[test]
fn drop_shadow_rows_stand_under_their_choice() {
    let shown = texts("effect.dialog", json!({"effect": "stylize.dropShadow"}));
    let mut order: Vec<(f32, &str)> = ["Color", "Darkness"].iter().flat_map(|l| all(&shown, l).into_iter().map(move |r| (r.top(), *l))).collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    let labels: Vec<&str> = order.iter().map(|(_, l)| *l).collect();
    assert_eq!(labels, ["Color", "Color", "Darkness", "Darkness"], "each choice, then its row");
}

/// Print's Range field stands under the Range choice, before Ignore Artboards.
#[test]
fn print_range_stands_under_its_choice() {
    let shown = texts("file.print", json!({}));
    let ignore = one(&shown, "Ignore Artboards (all the art as one page)");
    let ranges = all(&shown, "Range");
    assert_eq!(ranges.len(), 2, "the choice and the field");
    assert!(ranges.iter().all(|r| r.bottom() <= ignore.top()), "the Range field before Ignore Artboards");
}
