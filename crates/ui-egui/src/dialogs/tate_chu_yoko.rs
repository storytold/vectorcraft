//! Character panel menu › Tate-chu-yoko Settings… (or `ui.tateChuYokoSettings`): the Up/Down and
//! Left/Right of the characters the Character panel acts on, in points, which move a block of
//! tate-chu-yoko characters in vertical type up or down and right or left. The dialog opens on
//! the values the selected characters share (a blank field where they differ). OK runs
//! `text.setFormat` (`text.setRangeStyle` while the Type tool edits text) with the values that
//! changed, as one undo step; characters that differ keep a value left blank.
//!
//! Fields: `tcyUpDown` and `tcyLeftRight` (pt, -1296..1296), as `text.setFormat` takes them.
//!
//! Behavior reference (public documentation):
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/format-east-asian-characters-in-vertical-text.html>.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog};
// PLAN: assumed path of `LengthUnits`
use craft::ui::runtime::env::LengthUnits;
use serde_json::{Map, Value, json};

use super::warichu::set_shared;
use crate::VectorcraftApp;

/// The dialog kind of Tate-chu-yoko Settings.
pub const KIND: &str = "tateChuYokoSettings";

/// The fields, as `text.setFormat` names them.
const KEYS: [&str; 2] = ["tcyUpDown", "tcyLeftRight"];

/// The range of both fields, in points.
const RANGE: std::ops::RangeInclusive<f64> = -1296.0..=1296.0;

/// The rows of the dialog, shown in `lengths`.
fn rows(lengths: LengthUnits) -> Vec<Param> {
    KEYS.into_iter()
        .zip([tl!("Up/Down:"), tl!("Left/Right:")])
        .map(|(key, label)| Param::number(key, label, 0.0, 1.0, 2).length(lengths).bounds(Some(*RANGE.start()), Some(*RANGE.end())))
        .collect()
}

/// Open Tate-chu-yoko Settings on the characters the Character panel acts on.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let target = crate::panels::character::format_target_kept(app)?;
    let active = app.session.active().ok_or("no document")?;
    let (document, reverts) = (active.uid, active.reverts);
    let (up, right) = crate::panels::character::shared_tcy_offsets(app);
    let mut fields = DialogFields::new();
    for (key, v) in KEYS.into_iter().zip([up, right]) {
        set_shared(&mut fields, key, v);
    }
    let initial = fields.clone();
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let lengths = crate::canvas::rulers::type_lengths(app);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        ParamDialog::new(KIND, tl!("Tate-chu-yoko Settings"), rows(lengths)).values(Bind::bound(fields))
    });
    super::host::open_held(app, dialog, move |app, result| {
        let p = params(&result.fields, &initial);
        if p.is_empty() {
            return Ok(());
        }
        let active = app
            .session
            .active()
            .filter(|active| active.uid == document)
            .ok_or_else(|| tl!("Tate-chu-yoko Settings can only change the document where it was opened.").to_owned())?;
        if active.reverts != reverts {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").to_owned());
        }
        crate::panels::character::run_format_at_target(app, Value::Object(p), &target).map(|_| ())
    })
    .ok_or("no window")?;
    Ok(Value::Null)
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with,
/// in points and kept within -1296..1296. A mixed field is left alone.
fn params(fields: &DialogFields, initial: &DialogFields) -> Map<String, Value> {
    let mut p = Map::new();
    for key in KEYS {
        let Some(v) = fields.number(key).map(|v| v.clamp(*RANGE.start(), *RANGE.end())) else { continue };
        if initial.number(key) != Some(v) {
            p.insert(key.into(), json!(v));
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::super::warichu::driver;
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind};

    /// A selected point type object with four characters.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "2026"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    /// (Up/Down, Left/Right) of each run.
    fn offsets(app: &VectorcraftApp, id: u64) -> Vec<(f64, f64)> {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.runs.iter().map(|r| (r.style.tcy_up_down, r.style.tcy_left_right)).collect(),
            _ => panic!("not text"),
        }
    }

    #[test]
    fn tate_chu_yoko_settings_step_the_opening_number_on_ok() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"tcyUpDown": 3, "tcyLeftRight": -7})).unwrap();
        let mut panel = crate::tests::panel_harness::Panel::new(app, crate::tests::picks::dialogs);
        panel.app().run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::step_number_and_ok(&mut panel, "Up/Down");
        assert_eq!(offsets(&panel.app(), id), [(4.0, -7.0)]);
        assert!(!driver::is_open(&panel.app()));
    }

    /// `ui.tateChuYokoSettings` opens on the characters' values; OK sets both in one undo step.
    /// Automation sets the offsets in points, as a number row takes them.
    #[test]
    fn tate_chu_yoko_settings_dialog_sets_up_down_and_left_right() {
        let (mut app, id) = app();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        let f = driver::fields(&app);
        assert_eq!((f.number("tcyUpDown"), f.number("tcyLeftRight")), (Some(0.0), Some(0.0)));
        driver::set(&mut app, "tcyUpDown", json!(3));
        driver::set(&mut app, "tcyLeftRight", json!(-72));
        let undo = app.session.active().unwrap().history.undo.len();
        driver::ok(&mut app);
        assert!(!driver::is_open(&app));
        assert_eq!(offsets(&app, id), [(3.0, -72.0)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // Cancel changes nothing.
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::set(&mut app, "tcyUpDown", json!(9));
        driver::cancel(&mut app);
        assert_eq!(offsets(&app, id), [(3.0, -72.0)]);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.tateChuYokoSettings", json!({})).is_err());
        assert!(!driver::is_open(&app));
    }

    /// A command the engine refuses keeps the dialog open and the characters unchanged.
    #[test]
    fn tate_chu_yoko_settings_dialog_returns_a_refused_command_error() {
        let (mut app, id) = app();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::set(&mut app, "tcyUpDown", json!(5));
        app.run("select.set", json!({"ids": []})).unwrap();
        driver::ok(&mut app);
        assert!(driver::is_open(&app));
        assert_eq!(offsets(&app, id), [(0.0, 0.0)]);
    }

    #[test]
    fn tate_chu_yoko_settings_refuses_confirmation_in_another_drawing() {
        use craft::ui::dialog::{HeldOk, dialog_id};

        let (mut app, source_id) = app();
        let source_before = offsets(&app, source_id);
        let source_undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::set(&mut app, "tcyUpDown", json!(7));
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        let id = dialog_id(KIND);
        assert!(!host.with(|h| h.confirm_by_id(&ctx, id)), "the edit waits for the app");

        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let target_id = app.run("text.create", json!({"x": 10, "y": 50, "text": "other"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [target_id]})).unwrap();
        let target_before = offsets(&app, target_id);
        let target_undo = app.session.active().unwrap().history.undo.len();
        driver::pump(&mut app);
        assert_eq!(
            (offsets(&app, target_id), app.session.active().unwrap().history.undo.len()),
            (target_before, target_undo),
            "the other drawing's text and undo history stay untouched"
        );
        assert!(matches!(host.with(|h| h.held_state(id)), Some(HeldOk::Refused(_))));
        assert_eq!(driver::fields(&app).number("tcyUpDown"), Some(7.0));

        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("select.set", json!({"ids": [source_id]})).unwrap();
        assert_eq!(offsets(&app, source_id), source_before);
        assert!(crate::dialogs::tests::confirm(&mut app), "confirm the retained edit on its original drawing");
        assert_eq!(offsets(&app, source_id), [(7.0, 0.0)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), source_undo + 1);
    }

    /// While the Type tool edits text, OK sets the selected characters.
    #[test]
    fn tate_chu_yoko_settings_dialog_sets_the_range_the_type_tool_selects() {
        let (mut app, id) = app();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": id, "anchor": 0, "caret": 2}));
        assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(id), 0, 2)));
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::set(&mut app, "tcyLeftRight", json!(6));
        driver::ok(&mut app);
        assert_eq!(offsets(&app, id), [(0.0, 6.0), (0.0, 0.0)]);
    }

    #[test]
    fn tate_chu_yoko_approval_keeps_its_opening_text_target() {
        let (mut app, id) = app();
        let select = |app: &mut VectorcraftApp, anchor, caret| {
            app.session.set_tool_option("textSelection", &json!({"editing": id, "anchor": anchor, "caret": caret}));
        };
        app.session.select_tool("type", Default::default()).unwrap();
        select(&mut app, 0, 2);
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::set(&mut app, "tcyLeftRight", json!(6));

        select(&mut app, 2, 4);
        assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(id), 2, 4)));
        let dialog = craft::ui::dialog::dialog_id(KIND);
        crate::tests::picks::answer_dialog(&mut app, dialog, &[], true);
        assert_eq!(offsets(&app, id), [(0.0, 0.0)], "the new text range keeps its formatting");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(matches!(app.dialogs().unwrap().with(|h| h.held_state(dialog)), Some(craft::ui::dialog::HeldOk::Refused(_))));

        select(&mut app, 0, 2);
        assert!(crate::dialogs::tests::confirm(&mut app));
        assert_eq!(offsets(&app, id), [(0.0, 6.0), (0.0, 0.0)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }

    #[test]
    fn tate_chu_yoko_approval_refuses_to_change_restored_text() {
        let (mut app, id) = app();
        let dir = vectorcraft_testkit::TempDir::new("tate_chu_yoko-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        let before = offsets(&app, id);
        app.run("text.setFormat", json!({"ids": [id], "tcyUpDown": 3})).unwrap();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::set(&mut app, "tcyUpDown", json!(7));

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        let dialog = craft::ui::dialog::dialog_id(KIND);
        crate::tests::picks::answer_dialog(&mut app, dialog, &[], true);
        assert_eq!(offsets(&app, id), before);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(matches!(
            app.dialogs().unwrap().with(|h| h.held_state(dialog)),
            Some(craft::ui::dialog::HeldOk::Refused(reason)) if reason.as_str().contains("document was reverted")
        ));

        driver::cancel(&mut app);
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        driver::set(&mut app, "tcyUpDown", json!(7));
        driver::ok(&mut app);
        assert_eq!(offsets(&app, id)[0].0, 7.0);
    }
}
