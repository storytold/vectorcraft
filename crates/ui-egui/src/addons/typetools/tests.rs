use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::addons::dialogs::tests::{dialog_id, frame, open_item, read, set};

/// A window's app with one type object, selected.
fn app() -> VectorcraftApp {
    app_with_text("abc")
}

fn app_with_text(text: &str) -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("text.create", json!({"x": 40, "y": 100, "text": text, "size": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app
}

/// The real form controls the same text changes as the command, including attribute-dependent
/// defaults, random seed and whether a space is included. Cancel restores the original text.
#[test]
fn keyboard_and_choice_edits_reach_text_tweaker_preview_and_approval() {
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use craft::ui::patterns::form_dialogs::HostedParamDialog;

    fn text(app: &VectorcraftApp) -> vectorcraft_doc::TextObject {
        let state = app.session.active().unwrap();
        let node = state.doc.node(state.selection.objects[0]).unwrap();
        match &node.kind {
            vectorcraft_doc::NodeKind::Text(text) => (**text).clone(),
            _ => panic!("selected object is not text"),
        }
    }

    fn type_number(panel: &mut Panel, label: &str, input: &str) {
        panel.click_role(label, Role::SPIN_BUTTON);
        panel.key(egui::Key::A, egui::Modifiers::COMMAND);
        panel.type_text(input);
        panel.key(egui::Key::Tab, egui::Modifiers::NONE);
    }

    let params = json!({"attribute": "rotation", "from": 10.0, "to": 40.0, "shape": "random", "seed": 42, "skipSpaces": false});
    let mut expected = app_with_text("a b");
    expected.run(super::COMMAND, params.clone()).unwrap();
    let expected = text(&expected);
    for cancel in [true, false] {
        let app = app_with_text("a b");
        let original = app.session.active().unwrap().doc.clone();
        let selection = app.session.active().unwrap().selection.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let mut panel = Panel::new(app, crate::tests::picks::dialogs);
        panel.room(egui::vec2(800.0, 700.0));
        open_item(&mut panel.app(), "Type Tools", "Text Tweaker…");
        panel.frame();
        let id = dialog_id(&panel.app());
        assert!(panel.app().dialogs().unwrap().with(|host| host.get_by_id::<HostedParamDialog>(id).unwrap().is_previewing()));
        panel.click_role("Preview", Role::CHECK_BOX);
        assert_eq!(panel.app().session.active().unwrap().doc, original);
        panel.click("Attribute");
        panel.click("Character Rotation");
        assert_eq!(read(&panel.app(), id, |fields| (fields.number("from"), fields.number("to"))), (Some(0.0), Some(30.0)));
        type_number(&mut panel, "From", "10");
        type_number(&mut panel, "To", "40");
        panel.click("Shape");
        panel.click("Random");
        assert!(panel.enabled("Seed"));
        type_number(&mut panel, "Seed", "42");
        panel.click_role("Skip Spaces", Role::CHECK_BOX);
        assert_eq!(read(&panel.app(), id, super::params), params);
        assert_eq!(panel.app().session.active().unwrap().doc, original, "Preview off leaves the text unchanged");
        panel.click_role("Preview", Role::CHECK_BOX);
        assert_eq!(text(&panel.app()), expected, "the real controls reach the same text formatting as automation");
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
        panel.click(if cancel { "Cancel" } else { "OK" });
        assert!(!panel.app().any_dialog_open());
        assert!(!panel.app().session.in_interaction());
        if cancel {
            let app = panel.app();
            let state = app.session.active().unwrap();
            assert_eq!(state.doc, original);
            assert_eq!(state.selection, selection);
            assert_eq!(state.history.undo.len(), undo);
        } else {
            assert_eq!(text(&panel.app()), expected);
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
            panel.app().run("edit.undo", json!({})).unwrap();
            assert_eq!(panel.app().session.active().unwrap().doc, original);
        }
    }
}

#[test]
fn another_attribute_starts_from_its_own_values() {
    let mut app = app();
    open_item(&mut app, "Type Tools", "Text Tweaker…");
    let dialog = dialog_id(&app);
    set(&app, dialog, "attribute", json!("rotation"));
    frame(&mut app);
    assert_eq!(read(&app, dialog, |fields| (fields.number("from"), fields.number("to"))), (Some(0.0), Some(30.0)));
}
