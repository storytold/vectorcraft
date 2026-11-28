//! Divide into Dashes in the interface: its dialog (dash and gap lengths, Fit to Corners and Ends,
//! Remove Gaps), which previews on the canvas and runs `addon.pathtools.divideDashes` on OK.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Spec};
use crate::canvas::rulers::general_lengths;

/// The command.
pub const COMMAND: &str = "addon.pathtools.divideDashes";
/// The dash and gap lengths the command takes (pt).
const DASHES: std::ops::RangeInclusive<f64> = 0.01..=1e6;
const GAPS: std::ops::RangeInclusive<f64> = 0.0..=1e6;

/// How the dialog's values reach the command.
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: None };

/// The command's parameters from the dialog's fields.
pub(crate) fn params(fields: &DialogFields) -> Value {
    json!({
        "dash": fields.number("dash").unwrap_or(10.0),
        "gap": fields.number("gap").unwrap_or(0.0),
        "fit": fields.bool("fit").unwrap_or(false),
        "removeGaps": fields.bool("removeGaps").unwrap_or(true),
    })
}

/// Opens Divide into Dashes' dialog on the command's defaults.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = DialogFields::from_pairs([
        ("dash", FieldValue::Number(10.0)),
        ("gap", FieldValue::Number(0.0)),
        ("fit", FieldValue::Bool(false)),
        ("removeGaps", FieldValue::Bool(true)),
    ]);
    let lengths = general_lengths(app);
    dialogs::open(app, &SPEC, fields, move |fields| {
        let rows = vec![
            Param::number("dash", tl!("Dash:"), 10.0, 1.0, 2).length(lengths).bounds(Some(*DASHES.start()), Some(*DASHES.end())),
            Param::number("gap", tl!("Gap:"), 0.0, 1.0, 2).length(lengths).bounds(Some(*GAPS.start()), Some(*GAPS.end())),
            Param::new("fit", tl!("Fit to Corners and Ends"), ParamKind::bool(false)),
            // Without a gap there are no gap pieces to remove.
            // PLAN: assumed `Param::enabled_above(key, threshold)` (pl.F4, forms-w1)
            Param::new("removeGaps", tl!("Remove Gaps"), ParamKind::bool(true)).enabled_above("gap", 0.0),
        ];
        ParamDialog::new(COMMAND, tl!("Divide into Dashes"), rows).values(Bind::bound(fields)).previews(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addons::dialogs::tests::{dialog_id, open_item, read};
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let path = app.run("path.create", json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}]})).unwrap();
        app.run("select.set", json!({"ids": [path["id"]]})).unwrap();
        app
    }

    fn draw(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| host.run(ui));
        }
        crate::dialogs::host::pump(app, ui.ctx());
    }

    fn spans(app: &VectorcraftApp) -> Vec<(f64, f64)> {
        app.session.active().unwrap().doc.layers[0]
            .children()
            .unwrap()
            .iter()
            .map(|node| {
                let bounds = node.geometric_bounds().unwrap();
                (bounds.x0, bounds.x1)
            })
            .collect()
    }

    /// Real input controls the preview and approval; Cancel restores the undivided path.
    #[test]
    fn keyboard_and_toggle_edits_control_dashes_preview_cancel_and_ok() {
        let mut expected = app();
        expected.run(COMMAND, json!({"dash": 30, "gap": 10, "fit": true, "removeGaps": false})).unwrap();
        let expected = spans(&expected);
        assert!(expected.len() > 1);

        for cancel in [true, false] {
            let app = app();
            let original = app.session.active().unwrap().doc.clone();
            let selection = app.session.active().unwrap().selection.clone();
            let undo = app.session.active().unwrap().history.undo.len();
            let mut panel = Panel::new(app, draw);
            panel.room(egui::vec2(800.0, 600.0));
            open_item(&mut panel.app(), "Path Precision Tools", "Divide into Dashes…");
            panel.frame();
            panel.click_role("Preview", Role::CHECK_BOX);
            let id = dialog_id(&panel.app());
            for (label, key, text, value) in [("Dash", "dash", "30 pt", 30.0), ("Gap", "gap", "10 pt", 10.0)] {
                panel.click_role(label, Role::SPIN_BUTTON);
                panel.key(egui::Key::A, egui::Modifiers::COMMAND);
                panel.type_text(text);
                panel.key(egui::Key::Tab, egui::Modifiers::NONE);
                assert_eq!(read(&panel.app(), id, |fields| fields.number(key)), Some(value));
            }
            panel.click_role("Fit to Corners and Ends", Role::CHECK_BOX);
            panel.click_role("Remove Gaps", Role::CHECK_BOX);
            assert_eq!(read(&panel.app(), id, |fields| fields.bool("fit")), Some(true));
            assert_eq!(read(&panel.app(), id, |fields| fields.bool("removeGaps")), Some(false));
            panel.click_role("Preview", Role::CHECK_BOX);
            let preview = spans(&panel.app());
            assert_eq!(preview.len(), expected.len());
            assert!(preview.iter().zip(&expected).all(|(a, b)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6));
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
            panel.click(if cancel { "Cancel" } else { "OK" });
            assert!(panel.app().dialogs().unwrap().with(|host| host.open_dialogs().is_empty()));
            assert!(!panel.app().session.in_interaction());
            if cancel {
                let app = panel.app();
                let state = app.session.active().unwrap();
                assert_eq!(state.doc, original);
                assert_eq!(state.selection, selection);
                assert_eq!(state.history.undo.len(), undo);
            } else {
                assert_eq!(spans(&panel.app()), preview);
                assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
                panel.app().run("edit.undo", json!({})).unwrap();
                assert_eq!(panel.app().session.active().unwrap().doc, original);
            }
        }
    }
}
