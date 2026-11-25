//! Headless frames of the Gradient panel's freeform section and the Color panel on a freeform
//! point.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::testing::Fixture;
use egui::{Key, Modifiers, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{Freeform, Paint};
use vectorcraft_engine::Session;

use super::*;

/// A selected rectangle with a freeform fill.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 300, "height": 300}));
    run(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}));
    run(&mut app, "paint.editGradient", json!({"kind": "freeform"}));
    app
}

fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
    app.session.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn freeform(app: &VectorcraftApp) -> Freeform {
    match current_paints(app).0 {
        Paint::Gradient(g) => g.freeform.expect("placed points"),
        p => panic!("expected a gradient fill, got {p:?}"),
    }
}

/// The Color panel targets the selected freeform point, not the fill: a hex typed into it
/// recolours that point and the paint stays a gradient.
#[test]
fn the_color_panel_recolours_the_selected_point() {
    let mut app = app();
    run(&mut app, "paint.freeform.selectPoint", json!({"index": 2}));
    let app = Rc::new(RefCell::new(app));
    let shown = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| color::show(&mut shown.borrow_mut(), ui));
    fixture.room(vec2(320.0, 700.0)).settle();
    let hex = fixture.by_label("Hex color").unwrap_or_else(|| panic!("the Color panel's hex field"));
    fixture.click(&hex).key(Key::A, Modifiers::COMMAND).type_text("00FF00").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(freeform(&app.borrow()).points[2].color.to_hex(), "#00ff00");
    assert!(matches!(current_paints(&app.borrow()).0, Paint::Gradient(_)), "still a gradient");
}
