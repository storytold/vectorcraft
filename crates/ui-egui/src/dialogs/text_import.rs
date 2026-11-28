//! Text Import Options: placing a text file asks how to read it (platform and character set) and
//! what to clean up (line breaks inside paragraphs, blank lines between them, runs of spaces). OK
//! places it as area type (`file.place` with `text`) and remembers the options: the next text file
//! starts from them, and is placed with them when Show Import Options is off.
//!
//! Fields: `platform` (`windows` | `mac`), `characterSet` (one of [`CHARACTER_SETS`]),
//! `removeLineReturns`, `removeParagraphReturns`, `replaceSpaces` (on or off) and `spaces` (how
//! many spaces make a tab, 2–100). The `file.place` params naming the file stay with the dialog's
//! OK, which places the file and keeps the dialog open with the error when that fails.
//!
//! PLAN: no `pump`: `open_held`'s `ok` closure runs the import, so fl.A9's `text_import::pump`
//! line has no work to call.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use super::host::open_held;
use crate::VectorcraftApp;

/// The dialog kind of Text Import Options.
pub const KIND: &str = "textImport";

/// The Character Set choices: `file.place`'s `text.characterSet` value and the menu label.
const CHARACTER_SETS: [(&str, &str); 11] = [
    ("unicode", "Unicode"),
    ("ansi", "ANSI"),
    ("shiftJis", "Shift JIS"),
    ("gb2312", "GB2312"),
    ("chineseBig5", "Chinese Big 5"),
    ("cyrillic", "Cyrillic"),
    ("gb18030", "GB18030"),
    ("greek", "Greek"),
    ("turkish", "Turkish"),
    ("baltic", "Baltic"),
    ("centralEuropean", "Central European"),
];

/// The platforms a file can come from: `file.place`'s `text.platform` value and the menu label.
const PLATFORMS: [(&str, &str); 2] = [("windows", "Windows"), ("mac", "Mac")];

/// The fewest and most spaces that make a tab.
const SPACES: std::ops::RangeInclusive<i64> = 2..=100;

/// Is the file `p` names (`path` or `name`) a text file?
pub fn is_text(p: &Value) -> bool {
    let name = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).unwrap_or_default();
    vectorcraft_engine::cmd::fileio::TEXT_EXTS.contains(&vectorcraft_engine::cmd::fileio::extension(name).as_str())
}

/// The rows of the dialog.
fn params() -> Vec<Param> {
    let choices =
        |pairs: &[(&str, &'static str)]| -> Vec<(String, Label)> { pairs.iter().map(|(id, label)| ((*id).to_string(), (*label).into())).collect() };
    vec![
        Param::section(tl!("Encoding")),
        Param::new("platform", tl!("Platform"), ParamKind::choice(choices(&PLATFORMS), None)),
        Param::new("characterSet", tl!("Character Set"), ParamKind::choice(choices(&CHARACTER_SETS), None)),
        Param::section(tl!("Line Breaks")),
        Param::new("removeLineReturns", tl!("Join the lines of each paragraph"), ParamKind::bool(false)),
        Param::new("removeParagraphReturns", tl!("Remove blank lines between paragraphs"), ParamKind::bool(false)),
        Param::section(tl!("Spaces")),
        Param::new("replaceSpaces", tl!("Replace"), ParamKind::bool(false)),
        Param::new("spaces", tl!("or more spaces with a tab"), ParamKind::int(Some(*SPACES.start()), Some(*SPACES.end()), 3))
            .enabled_when("replaceSpaces", true),
    ]
}

/// The dialog over its values.
fn build(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new(KIND, tl!("Text Import Options"), params()).values(Bind::bound(fields))
}

/// The values the dialog opens with: the options confirmed last where they are ones the dialog offers
/// (a remembered value from another version falls back to the default).
fn starting_fields(app: &VectorcraftApp) -> DialogFields {
    let last = &app.ui.text_import;
    let text = |key: &str, offered: &[(&str, &str)], default: &str| {
        last.get(key).and_then(Value::as_str).filter(|v| offered.iter().any(|(id, _)| id == v)).unwrap_or(default).to_string()
    };
    let flag = |key: &str| last.get(key).and_then(Value::as_bool).unwrap_or(false);
    let spaces = last.get("replaceSpaces").and_then(Value::as_i64).filter(|n| SPACES.contains(n));
    let mut fields = DialogFields::new();
    let platform = if cfg!(target_os = "macos") { "mac" } else { "windows" };
    fields.set("platform", FieldValue::Text(text("platform", &PLATFORMS, platform)));
    fields.set("characterSet", FieldValue::Text(text("characterSet", &CHARACTER_SETS, "unicode")));
    fields.set("removeLineReturns", FieldValue::Bool(flag("removeLineReturns")));
    fields.set("removeParagraphReturns", FieldValue::Bool(flag("removeParagraphReturns")));
    fields.set("replaceSpaces", FieldValue::Bool(spaces.is_some()));
    fields.set("spaces", FieldValue::Int(spaces.unwrap_or(3)));
    fields
}

/// Ask how to place the text file `params` (`file.place` params) names, starting from the options
/// confirmed last.
pub fn open(app: &mut VectorcraftApp, params: &Value) -> Value {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let dialog = HostedParamDialog::new(&ctx, KIND, starting_fields(app), build).held_ok();
    let drawing = crate::place::import_options_drawing(app, params);
    let params = params.clone();
    open_held(app, dialog, move |app, result| {
        crate::place::check_import_options_drawing(app, drawing)?;
        confirm(app, &params, result)
    });
    json!({ "dialog": KIND })
}

/// The `text` options a text file is placed with when Show Import Options is off: those confirmed
/// last, else `file.place`'s defaults.
pub fn last_options(app: &VectorcraftApp) -> Value {
    if app.ui.text_import.is_object() { app.ui.text_import.clone() } else { json!({}) }
}

/// `file.place`'s `text` options from the answered values.
fn options(fields: &DialogFields) -> Value {
    let mut o = json!({
        "platform": fields.text("platform").unwrap_or("windows"),
        "characterSet": fields.text("characterSet").unwrap_or("unicode"),
        "removeLineReturns": fields.bool("removeLineReturns").unwrap_or(false),
        "removeParagraphReturns": fields.bool("removeParagraphReturns").unwrap_or(false),
    });
    if fields.bool("replaceSpaces").unwrap_or(false) {
        o["replaceSpaces"] = json!(fields.int("spaces").unwrap_or(3).clamp(*SPACES.start(), *SPACES.end()));
    }
    o
}

/// Places the file `params` name with the options and remembers them for the next text file; an
/// error keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, params: &Value, result: &ParamResult) -> Result<(), String> {
    let mut params = params.clone();
    params["text"] = options(&result.fields);
    crate::place::run_for_user(app, &params)?;
    app.ui.text_import = params["text"].clone();
    Ok(())
}

/// What the three import dialogs' tests share: a window for the host, and the open dialog's fields,
/// field sets and held OK by dialog key.
// PLAN: the shared home of these helpers is `dialogs/host.rs` (hub-w2); they live here until it has one.
#[cfg(test)]
pub(super) mod test_support {
    use craft::ui::dialog::{HeldOk, OpenDialog};
    use serde_json::Value;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    /// An empty app with a window.
    pub fn app() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("the test app has a window")
    }

    /// The open dialog `key` names.
    pub fn open_dialog(app: &VectorcraftApp, key: &str) -> Option<OpenDialog> {
        app.dialogs()?.with(|h| h.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)))
    }

    /// Whether a dialog `key` names is open.
    pub fn is_open(app: &VectorcraftApp, key: &str) -> bool {
        open_dialog(app, key).is_some()
    }

    /// The fields of the open dialog `key` names, as automation lists them.
    pub fn fields(app: &VectorcraftApp, key: &str) -> Value {
        open_dialog(app, key).and_then(|d| d.fields).expect("the dialog is open and lists its fields")
    }

    /// Sets `field` of the open dialog as the control channel does; `Err` is the dialog's refusal.
    pub fn try_set(app: &VectorcraftApp, key: &str, field: &str, value: Value) -> Result<(), String> {
        let id = open_dialog(app, key).ok_or("the dialog is not open")?.id;
        let windows = app.dialogs().ok_or("no window")?;
        windows.with(|h| h.set_json_field(&ctx(app), id, field, value))
    }

    /// [`try_set`] that must succeed.
    pub fn set(app: &VectorcraftApp, key: &str, field: &str, value: Value) {
        try_set(app, key, field, value).expect("the dialog takes the value");
    }

    /// Presses OK on the open dialog `key` names and lets the host run the import: `Err` is the reason
    /// the import refused, with the dialog left open.
    pub fn confirm(app: &mut VectorcraftApp, key: &str) -> Result<(), String> {
        let id = open_dialog(app, key).ok_or("the dialog is not open")?.id;
        let windows = app.dialogs().ok_or("no window")?;
        let ctx = ctx(app);
        windows.with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
        let state = windows.with(|h| h.held_state(id));
        match state {
            Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_owned()),
            Some(HeldOk::Accepted) => {
                windows.with(|h| h.confirm_by_id(&ctx, id));
                Ok(())
            }
            Some(_) => Err("the import did not answer".into()),
            None if is_open(app, key) => Err("OK was not filed".into()),
            None => Ok(()),
        }
    }

    /// Cancels the open dialog `key` names.
    pub fn cancel(app: &VectorcraftApp, key: &str) {
        let Some(id) = open_dialog(app, key).map(|d| d.id) else { return };
        if let Some(windows) = app.dialogs() {
            windows.with(|h| h.cancel_by_id(&ctx(app), id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support as host;
    use super::*;
    use vectorcraft_engine::doc::NodeKind;

    /// The text of the one text object in the active document.
    fn placed_text(app: &VectorcraftApp) -> Option<String> {
        let mut found = None;
        app.session.doc().unwrap().doc.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                found = Some(t.plain_text());
            }
        });
        found
    }

    #[test]
    fn refused_text_import_does_not_change_the_next_files_options() {
        use craft::ui::dialog::HeldOk;
        use egui::accesskit::{Role, TreeUpdate};
        use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        fn control(tree: &TreeUpdate, source: &str, role: Role, scope: Option<Rect>) -> Rect {
            fn plain(label: &str) -> &str {
                label.trim_end().trim_end_matches([':', '：']).trim_end()
            }
            let caption = crate::i18n::t(source);
            tree.nodes
                .iter()
                .map(|(_, node)| node)
                .filter(|node| node.role() == role && node.label().is_some_and(|label| plain(label) == plain(caption)))
                .filter_map(|node| {
                    let b = node.bounds()?;
                    Some(Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)))
                })
                .find(|rect| scope.is_none_or(|modal| modal.contains(rect.center())))
                .unwrap_or_else(|| panic!("missing {role:?} {caption:?} in {scope:?}"))
        }

        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        let mut app = host::app();
        app.session.prefs.interface_language = "en".into();
        app.placing.off_the_ui_thread = true;
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [rect]})).unwrap();
        let prior = json!({"platform": "windows", "characterSet": "unicode", "removeLineReturns": false, "removeParagraphReturns": false});
        app.ui.text_import = prior.clone();

        let ctx = app.ui_ctx.clone().unwrap();
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
            time += 0.1;
            let raw = RawInput { events, time: Some(time), screen_rect: Some(screen), focused: true, ..Default::default() };
            crate::testing::app_frame(app, &ctx, raw).platform_output.accesskit_update.unwrap()
        };
        let settle = |app: &mut VectorcraftApp, frame: &mut dyn FnMut(&mut VectorcraftApp, Vec<Event>) -> TreeUpdate| {
            for _ in 0..5 {
                frame(app, vec![]);
            }
            frame(app, vec![])
        };
        let click = |app: &mut VectorcraftApp, frame: &mut dyn FnMut(&mut VectorcraftApp, Vec<Event>) -> TreeUpdate, pos| {
            frame(app, vec![Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(app, vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
            }
            frame(app, vec![]);
        };
        let press = |app: &mut VectorcraftApp, frame: &mut dyn FnMut(&mut VectorcraftApp, Vec<Event>) -> TreeUpdate, key, modifiers| {
            for pressed in [true, false] {
                frame(app, vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }]);
            }
        };

        let file = json!({"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(b"one    two")});
        let mut replace = file.clone();
        replace["replace"] = json!(true);
        settle(&mut app, &mut frame);
        assert_eq!(app.run("file.place", replace).unwrap()["dialog"], KIND);
        let tree = settle(&mut app, &mut frame);
        let modal = control(&tree, "Text Import Options", Role::Dialog, None);
        assert!(screen.expand(0.5).contains_rect(modal));
        let checkbox = control(&tree, "Replace", Role::CheckBox, Some(modal));
        click(&mut app, &mut frame, checkbox.center());
        let tree = settle(&mut app, &mut frame);
        let number = control(&tree, "or more spaces with a tab", Role::SpinButton, Some(modal));
        let ok = control(&tree, "OK", Role::Button, Some(modal));
        click(&mut app, &mut frame, number.center());
        press(&mut app, &mut frame, Key::A, Modifiers::COMMAND);
        frame(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE), Event::Text("4".into())]);
        press(&mut app, &mut frame, Key::Tab, Modifiers::NONE);
        settle(&mut app, &mut frame);
        let fields = host::fields(&app, KIND);
        assert_eq!(fields["replaceSpaces"], true, "actual toggle");
        assert_eq!(fields["spaces"].as_f64(), Some(4.0), "actual focused text");

        // The original drawing is still open, but an automation command removed the Replace target.
        app.run("select.set", json!({"ids": []})).unwrap();
        let history = app.session.doc().unwrap().history.undo.len();
        click(&mut app, &mut frame, ok.center());
        let tree = settle(&mut app, &mut frame);
        let opened = host::open_dialog(&app, KIND).expect("the refusal kept the dialog open");
        let refusal = app.dialogs().unwrap().with(|h| h.held_state(opened.id));
        assert!(
            matches!(refusal, Some(HeldOk::Refused(reason)) if reason.as_str().contains("select the one object")),
            "expected Replace target refusal"
        );
        assert_eq!(app.session.doc().unwrap().history.undo.len(), history, "refused import did not edit the document");
        let modal = control(&tree, "Text Import Options", Role::Dialog, None);
        let cancel = control(&tree, "Cancel", Role::Button, Some(modal));
        click(&mut app, &mut frame, cancel.center());
        settle(&mut app, &mut frame);
        assert!(!host::is_open(&app, KIND), "Cancel closed the refusal");
        assert_eq!(app.run("file.place", file).unwrap()["dialog"], KIND);
        let fields = host::fields(&app, KIND);
        assert_eq!((fields["replaceSpaces"].as_bool(), fields["spaces"].as_f64()), (Some(false), Some(3.0)));
        assert_eq!(app.ui.text_import, prior, "only accepted settings are remembered");
    }

    #[test]
    fn placing_a_text_file_asks_for_the_import_options_then_places_area_type() {
        let mut app = host::app();
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let text = b"one\r\ntwo\r\n\r\nthree\r\n";
        let file = json!({"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(text)});
        let r = app.run("file.place", file).unwrap();
        assert_eq!(r["dialog"], KIND);
        let fields = host::fields(&app, KIND);
        assert_eq!((&fields["characterSet"], &fields["replaceSpaces"]), (&json!("unicode"), &json!(false)));
        host::set(&app, KIND, "removeLineReturns", json!(true));
        host::set(&app, KIND, "removeParagraphReturns", json!(true));
        host::confirm(&mut app, KIND).unwrap();
        assert!(!host::is_open(&app, KIND));
        assert_eq!(placed_text(&app).as_deref(), Some("one two\nthree"), "one paragraph per block");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), 1);
        assert_eq!(app.ui.text_import["removeLineReturns"], true, "remembered for the next file");
    }

    #[test]
    fn normal_text_place_options_wait_for_the_original_drawing() {
        let mut app = host::app();
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let file = json!({"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(b"one\r\ntwo")});
        assert_eq!(app.run("file.place", file).unwrap()["dialog"], KIND);
        host::set(&app, KIND, "removeLineReturns", json!(true));
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let error = host::confirm(&mut app, KIND).expect_err("the other drawing must not receive the file");
        assert!(error.contains("original drawing"), "{error}");
        assert!(placed_text(&app).is_none());
        assert_eq!(host::fields(&app, KIND)["removeLineReturns"], true);
        app.run("document.activate", json!({"index": 0})).unwrap();
        host::confirm(&mut app, KIND).unwrap();
        assert_eq!(placed_text(&app).as_deref(), Some("one two"));
    }

    /// Every character set is a choice: one confirmed last comes back on the next text file and
    /// decodes it, an unknown remembered value falls back to Unicode, and with Show Import Options
    /// off the remembered set is the one the file is placed with.
    #[test]
    fn a_remembered_character_set_comes_back_and_places_the_file() {
        let mut app = host::app();
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let file = json!({"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(&[0x93, 0xFA, 0x96, 0x7B])});
        app.ui.text_import = json!({"characterSet": "shiftJis", "platform": "windows"});
        app.run("file.place", file.clone()).unwrap();
        assert_eq!(host::fields(&app, KIND)["characterSet"], "shiftJis");
        host::confirm(&mut app, KIND).unwrap();
        assert_eq!(placed_text(&app).as_deref(), Some("日本"));
        assert_eq!(last_options(&app)["characterSet"], "shiftJis", "remembered for Show Import Options off");
        app.ui.text_import = json!({"characterSet": "latin9"});
        app.run("file.place", file).unwrap();
        assert_eq!(host::fields(&app, KIND)["characterSet"], "unicode");
    }

    /// A text file dropped on a canvas is placed by the dialog's OK in the document it was dropped
    /// on, even when another document became active meanwhile.
    #[test]
    fn a_dropped_text_files_ok_places_in_the_document_it_was_dropped_on() {
        use crate::place::{DropAt, DropTarget};
        let mut app = host::app();
        for _ in 0..2 {
            app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        }
        app.session.set_active(0);
        let st = app.session.doc().unwrap();
        let target = DropTarget::Place(DropAt { doc: st.uid, reverts: st.reverts, at: vectorcraft_geom::Point::new(100.0, 100.0), embed: true });
        crate::place::drop_files(&mut app, vec![(target, ("notes.txt".into(), None, b"one".to_vec()))]);
        assert!(host::is_open(&app, KIND));
        app.session.set_active(1);
        host::confirm(&mut app, KIND).unwrap();
        let objects = |i: usize| app.session.documents()[i].doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum::<usize>();
        assert_eq!((objects(0), objects(1)), (1, 0), "placed in the document it was dropped on");
        assert_eq!(app.session.active_index(), Some(0));
    }
}
