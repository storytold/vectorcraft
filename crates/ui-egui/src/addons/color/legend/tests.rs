use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use serde_json::json;

use crate::VectorcraftApp;
use crate::tests::panel_harness::Panel;
use craft::behavior::Mixed;
use craft::ui::contract::Role;
use vectorcraft_doc::NodeKind;

fn assert_edited_legend(app: &VectorcraftApp) {
    let state = app.session.active().unwrap();
    let group = state.doc.node(state.selection.objects[0]).unwrap();
    assert_eq!(group.name.as_deref(), Some("Swatch Legend"));
    let children = group.children().unwrap();
    let chips: Vec<_> = children.iter().filter(|node| matches!(node.kind, NodeKind::Path { .. })).collect();
    let labels: Vec<_> = children
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Text(text) => Some(text.plain_text()),
            _ => None,
        })
        .collect();
    assert_eq!(chips.len(), 3, "only the three selected swatches are drawn");
    assert_eq!(labels.len(), 3);
    let bounds = |index: usize| chips[index].geometric_bounds().unwrap();
    assert!((bounds(0).width() - 40.0).abs() < 1e-6, "the typed chip size reaches the legend");
    assert!((bounds(1).x0 - bounds(0).x0 - 50.0).abs() < 1e-6, "the typed gap separates columns");
    assert!((bounds(2).x0 - bounds(0).x0).abs() < 1e-6 && (bounds(2).y0 - bounds(0).y0 - 50.0).abs() < 1e-6, "two columns wrap the third chip");
    for label in labels {
        assert!(
            label.lines().any(|line| line.starts_with('#')) && label.lines().any(|line| line.starts_with("C=")),
            "the selected value formats reach the label: {label}"
        );
        assert!(["Ink", "Paper", "Signal"].contains(&label.lines().next().unwrap()));
    }
}

/// Real controls change the preview's layout and labels; Cancel restores the drawing, while
/// OK with Preview switched off applies the same hosted fields in one undo step.
#[test]
fn keyboard_and_checkbox_edits_reach_legend_preview_ok_and_cancel() {
    for commit in [false, true] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
        for (name, color) in [("Ink", "#123456"), ("Paper", "#f8f4e8"), ("Signal", "#cc2211")] {
            app.run("swatch.new", json!({"name": name, "color": color})).unwrap();
        }
        app.ui.swatches_panel.set_swatches(vec!["Ink".into(), "Paper".into(), "Signal".into()]);
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let mut panel = Panel::new(app, crate::tests::picks::dialogs);
        panel.room(egui::vec2(800.0, 700.0));
        crate::menus::invoke(&mut panel.app(), super::COMMAND, json!({}));
        panel.frame();
        assert_eq!(panel.toggled("Preview"), Some(Mixed::Same(true)), "Preview starts on");
        assert!(panel.app().session.in_interaction(), "the initial legend is previewed");
        if commit {
            panel.click_role("Preview", Role::CHECK_BOX);
            assert!(!panel.app().session.in_interaction());
        }
        panel.click_role("Selected Swatches Only", Role::CHECK_BOX);
        panel.click_role("HEX", Role::CHECK_BOX);
        panel.click_role("CMYK", Role::CHECK_BOX);
        for (label, input) in [("Columns", "2"), ("Size", "40 pt"), ("Gap", "10 pt")] {
            panel.click_role(label, Role::SPIN_BUTTON);
            panel.key(egui::Key::A, egui::Modifiers::COMMAND);
            panel.type_text(input);
            panel.key(egui::Key::Tab, egui::Modifiers::NONE);
        }
        if commit {
            assert!(panel.app().session.active().unwrap().doc == before, "Preview off leaves the drawing alone");
            panel.click("OK");
            assert_edited_legend(&panel.app());
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
            panel.app().run("edit.undo", json!({})).unwrap();
        } else {
            assert_edited_legend(&panel.app());
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo, "Preview adds no undo step");
            panel.click("Cancel");
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
        }
        assert!(!panel.app().session.in_interaction());
        assert!(panel.app().session.active().unwrap().doc == before, "Cancel or Undo restores the original drawing");
        assert!(crate::tests::picks::open_dialogs(&panel.app()).is_empty());
    }
}

/// The names of the swatches chosen in the Swatches panel are sent only with Selected Swatches Only, and only when
/// there are some; the values go only for the ticked boxes.
#[test]
fn params_sends_names_only_for_selected_swatches_only_with_some_names() {
    let mut fields = DialogFields::from_pairs([
        ("columns", FieldValue::Int(2)),
        ("selectedOnly", FieldValue::Bool(true)),
        ("names", FieldValue::List(vec![FieldValue::Text("Ink".into())])),
    ]);
    assert_eq!(super::params(&fields)["names"], json!(["Ink"]));
    assert_eq!(super::params(&fields)["columns"], json!(2));
    assert!(super::params(&fields).get("values").is_none(), "none ticked: each swatch shows its own values");

    fields.set("selectedOnly", FieldValue::Bool(false));
    assert!(super::params(&fields).get("names").is_none());

    fields.set("selectedOnly", FieldValue::Bool(true));
    fields.set("names", FieldValue::List(Vec::new()));
    assert!(super::params(&fields).get("names").is_none());

    fields.set("hex", FieldValue::Bool(true));
    fields.set("cmyk", FieldValue::Bool(true));
    assert_eq!(super::params(&fields)["values"], json!(["hex", "cmyk"]));
}
