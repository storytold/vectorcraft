//! Symbol Options: name, Export Type (Graphic or Movie Clip), Symbol Type (Dynamic or Static) and
//! Registration for a new symbol
//! from the selection (`symbol.new`), or for the current symbol in the Symbols panel
//! (`symbol.options`).
//!
//! Fields: `name`, `nineSlice` (bool) with `sliceLeft` / `sliceTop` / `sliceRight` / `sliceBottom`
//! (percent of the symbol's size, -1600..1600: below 0 or above 100 for a guide outside the art),
//! `exportType` (`movieClip` | `graphic`), `type` (`dynamic` |
//! `static`; a new symbol is dynamic), `registration` (0..8, the 9-point grid;
//! 4 = centre), `alignToPixelGrid` (bool, off for a new symbol). Whether the dialog edits an existing
//! symbol, and whether a new symbol leaves the selection as ordinary art, are held by its OK.
//! Help:
//! <https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html>.

use std::collections::HashSet;

use craft::ui::contract::{Bind, Outcome as _, Show as _};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::pickers::{Anchor9, ReferencePoint, ReferencePointLook};
use craft::ui::runtime::env::{Label, UnitKind};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The dialog kind of Symbol Options.
pub const KIND: &str = "symbolOptions";

/// The 9-slice guide fields, in `[left, top, right, bottom]` order.
const SLICE_KEYS: [&str; 4] = ["sliceLeft", "sliceTop", "sliceRight", "sliceBottom"];

/// A guide field's value when the dialog has none: a quarter of the symbol's size, in percent.
const DEFAULT_SLICE_PERCENT: f64 = 25.0;

/// The range of a guide field, in percent: a guide may lie outside the art, up to
/// [`vectorcraft_doc::MAX_GUIDE_FRACTION`] times its size from its edge.
const SLICE_PERCENT_RANGE: std::ops::RangeInclusive<f64> = -vectorcraft_doc::MAX_GUIDE_FRACTION * 100.0..=vectorcraft_doc::MAX_GUIDE_FRACTION * 100.0;

/// Sets the 9-slice fields for `guides` (percent of the symbol's size, unrounded; a quarter each when
/// off) and returns the guides as fractions, `[left, top, right, bottom]`, for [`slice_params`].
fn slice_fields(fields: &mut DialogFields, guides: Option<vectorcraft_doc::SymbolNineSlice>) -> [f64; 4] {
    let g = guides.unwrap_or_default();
    fields.set("nineSlice", FieldValue::Bool(guides.is_some()));
    let opened = [g.left, g.top, g.right, g.bottom];
    for (key, v) in SLICE_KEYS.iter().zip(opened) {
        fields.set(*key, FieldValue::Number(v * 100.0));
    }
    opened
}

/// The guides the dialog's fields describe, as the `nineSlice` / `nineSliceGuides` params. A field
/// still showing the guide it opened with (`opened`) gives that guide exactly.
fn slice_params(fields: &DialogFields, opened: [f64; 4]) -> Value {
    let [l, t, r, b] = std::array::from_fn(|i| {
        let percent = fields.number(SLICE_KEYS[i]).unwrap_or(DEFAULT_SLICE_PERCENT);
        if opened[i] * 100.0 == percent { opened[i] } else { percent / 100.0 }
    });
    if fields.bool("nineSlice").unwrap_or(false) {
        json!({ "nineSlice": true, "nineSliceGuides": [l, t, r, b] })
    } else {
        json!({ "nineSlice": false })
    }
}

/// The dialog's rows. `name` is what the name field shows when it has no value.
fn rows(name: &str) -> Vec<Param> {
    // The reference app's default Export Type is Movie Clip.
    let export = vec![("movieClip".to_string(), Label::from(tl!("Movie Clip"))), ("graphic".to_string(), Label::from(tl!("Graphic")))];
    // A dynamic symbol's instances may change the appearance of parts of its art and stay linked.
    let kind = vec![("dynamic".to_string(), Label::from(tl!("Dynamic Symbol"))), ("static".to_string(), Label::from(tl!("Static Symbol")))];
    let mut rows = vec![
        Param::new("name", tl!("Name:"), ParamKind::text(name)),
        Param::new("exportType", tl!("Export Type:"), ParamKind::choice(export, Some("movieClip".to_string()))),
        Param::new("type", tl!("Symbol Type:"), ParamKind::radio(kind, Some("dynamic".to_string()))),
        Param::custom("registration", tl!("Registration:"), |ui, fields, _| {
            let at = fields
                .int("registration")
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| Anchor9::ALL.get(i).copied())
                .unwrap_or(Anchor9::Centre);
            let shown = ReferencePoint::new(tl!("Registration:"), Bind::given(Some(at))).look(ReferencePointLook::Squares).show(ui);
            if let Some(i) = shown.value().and_then(Option::as_ref).and_then(|picked| Anchor9::ALL.iter().position(|a| a == picked)) {
                fields.set("registration", FieldValue::Int(i as i64));
            }
            shown.response().clone()
        })
        .opens_with(FieldValue::Int(4)),
        Param::new("nineSlice", tl!("Enable Guides for 9-Slice Scaling"), ParamKind::bool(false)),
    ];
    for (key, label) in SLICE_KEYS.into_iter().zip([tl!("Left:"), tl!("Top:"), tl!("Right:"), tl!("Bottom:")]) {
        rows.push(
            Param::number(key, label, DEFAULT_SLICE_PERCENT, 1.0, 3)
                .unit(UnitKind::Percent)
                .bounds(Some(*SLICE_PERCENT_RANGE.start()), Some(*SLICE_PERCENT_RANGE.end()))
                .visible_when("nineSlice", true),
        );
    }
    rows.push(Param::new("alignToPixelGrid", tl!("Align to Pixel Grid"), ParamKind::bool(false)));
    rows
}

/// Opens Symbol Options on `fields`. OK makes a new symbol, or, with `edit` (the symbol being
/// edited), applies the fields to it; `keep_art` leaves the selection as ordinary art for a new one;
/// `opened` are the guides the dialog opened with.
fn open_dialog(app: &mut VectorcraftApp, fields: DialogFields, edit: Option<String>, keep_art: bool, opened: [f64; 4]) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let (document, reverts) = (st.uid, st.reverts);
    let selection = edit.is_none().then(|| st.selection.objects.iter().copied().collect::<HashSet<_>>());
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let name = fields.text("name").unwrap_or_default().to_string();
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        ParamDialog::new(KIND, tl!("Symbol Options"), rows(&name)).values(Bind::bound(fields))
    });
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| -> Result<(), String> {
        let Some(active) = app.session.active().filter(|st| st.uid == document) else {
            return Err(tl!("This symbol belongs to the document that opened the dialog.").into());
        };
        if active.reverts != reverts {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").into());
        }
        if let Some(selection) = &selection
            && (active.selection.objects.len() != selection.len() || active.selection.objects.iter().any(|id| !selection.contains(id)))
        {
            return Err(tl!("The selected artwork changed. Restore the selection to continue.").into());
        }
        let f = &result.fields;
        let name = f.text("name").unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return Err("name is empty".into());
        }
        let from = edit.as_deref().unwrap_or(&name);
        let mut params = json!({
            "name": from,
            "exportType": f.text("exportType").unwrap_or_default(),
            "type": f.text("type").unwrap_or_default(),
            "registration": f.int("registration").unwrap_or(4),
            "alignToPixelGrid": f.bool("alignToPixelGrid").unwrap_or(false),
        });
        merge(&mut params, slice_params(f, opened));
        if edit.is_some() {
            if from != name {
                params["newName"] = json!(name);
            }
            app.run("symbol.options", params)
        } else {
            if keep_art {
                params["keepArt"] = json!(true);
            }
            app.run("symbol.new", params)
        }
        .map(|_| ())
    };
    crate::dialogs::host::open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

/// Open Symbol Options to name a new symbol made from the selection. With `keep_art`, OK leaves
/// the selection as ordinary art instead of replacing it with an instance.
pub fn open(app: &mut VectorcraftApp, keep_art: bool) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.selection.is_empty() {
        return Err("nothing selected".into());
    }
    let mut fields = DialogFields::new();
    fields.set("name", FieldValue::Text(st.doc.unique_symbol_name("New Symbol")));
    fields.set("exportType", FieldValue::Text("movieClip".into()));
    fields.set("type", FieldValue::Text("dynamic".into()));
    fields.set("registration", FieldValue::Int(4));
    fields.set("alignToPixelGrid", FieldValue::Bool(false));
    let opened = slice_fields(&mut fields, None);
    open_dialog(app, fields, None, keep_art, opened)
}

/// Open Symbol Options on an existing symbol (`name`, else the Symbols panel's current).
pub fn open_edit(app: &mut VectorcraftApp, name: Option<&str>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let name = match name.filter(|n| !n.is_empty()) {
        Some(n) if st.doc.symbols.iter().any(|s| s.name == n) => n.to_string(),
        Some(n) => return Err(format!("no symbol named `{n}`")),
        None => st
            .doc
            .unknown
            .get("currentSymbol")
            .and_then(|v| v.as_str())
            .filter(|n| st.doc.symbols.iter().any(|s| s.name == *n))
            .map(str::to_string)
            .or_else(|| st.doc.symbols.first().map(|s| s.name.clone()))
            .ok_or_else(|| "the document has no symbols".to_string())?,
    };
    let export = st.doc.symbol_export_type(&name).id();
    let registration = st.doc.symbol_registration(&name).0;
    let kind = if st.doc.symbols.iter().any(|s| s.name == name && s.dynamic) { "dynamic" } else { "static" };
    let mut fields = DialogFields::new();
    fields.set("name", FieldValue::Text(name.clone()));
    fields.set("exportType", FieldValue::Text(export.to_string()));
    fields.set("type", FieldValue::Text(kind.into()));
    fields.set("registration", FieldValue::Int(registration as i64));
    fields.set("alignToPixelGrid", FieldValue::Bool(st.doc.symbol_pixel_aligned(&name)));
    let opened = slice_fields(&mut fields, st.doc.symbol_nine_slice(&name));
    open_dialog(app, fields, Some(name), false, opened)
}

/// Copy the keys of the object `extra` into the object `into`.
fn merge(into: &mut Value, extra: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), extra) {
        a.extend(b);
    }
}

#[cfg(test)]
mod document_tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    use super::super::tests_warp::{accept, dismiss, set, windowed};
    use super::*;
    use crate::tests::picks::answer_dialog;

    #[test]
    fn keyboard_and_pointer_edits_reach_new_and_existing_symbol_options() {
        use crate::tests::panel_harness::Panel;
        use craft::ui::contract::Role;

        fn draw(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| host.run(ui));
            }
            super::super::host::pump(app, ui.ctx());
        }

        for editing in [false, true] {
            let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
            if editing {
                app.run("symbol.new", json!({"name": "Badge", "type": "static", "alignToPixelGrid": true})).unwrap();
            }
            let mut panel = Panel::new(app, draw);
            panel.room(egui::vec2(800.0, 800.0));
            if editing {
                open_edit(&mut panel.app(), Some("Badge")).unwrap();
            } else {
                open(&mut panel.app(), false).unwrap();
            }
            panel.frame();
            let undo = panel.app().session.active().unwrap().history.undo.len();
            assert_eq!(
                panel.toggled("Align to Pixel Grid"),
                Some(craft::behavior::Mixed::Same(editing)),
                "the options open with the symbol's current pixel-alignment setting"
            );
            panel.click_role("Name", Role::TEXT_INPUT);
            panel.key(egui::Key::A, egui::Modifiers::COMMAND);
            panel.type_text("Keyboard badge");
            panel.key(egui::Key::Tab, egui::Modifiers::NONE);
            assert_eq!(super::super::tests_warp::json_fields(&panel.app(), dialog_id(KIND))["name"], "Keyboard badge");
            assert_eq!(super::super::tests_warp::json_fields(&panel.app(), dialog_id(KIND))["type"], if editing { "static" } else { "dynamic" });
            panel.click_role(if editing { "Dynamic Symbol" } else { "Static Symbol" }, Role::RADIO_BUTTON);
            assert_eq!(super::super::tests_warp::json_fields(&panel.app(), dialog_id(KIND))["type"], if editing { "dynamic" } else { "static" });
            panel.click_role("Align to Pixel Grid", Role::CHECK_BOX);
            panel.click_role("Enable Guides for 9-Slice Scaling", Role::CHECK_BOX);
            for (label, key, input, value) in [
                ("Left", "sliceLeft", "10", 10.0),
                ("Top", "sliceTop", "20", 20.0),
                ("Right", "sliceRight", "30", 30.0),
                ("Bottom", "sliceBottom", "40", 40.0),
            ] {
                panel.click_role(label, Role::SPIN_BUTTON);
                panel.key(egui::Key::A, egui::Modifiers::COMMAND);
                panel.type_text(input);
                panel.key(egui::Key::Tab, egui::Modifiers::NONE);
                assert_eq!(super::super::tests_warp::json_fields(&panel.app(), dialog_id(KIND))[key], value);
            }
            panel.click("OK");
            assert!(!super::super::tests_warp::open_ids(&panel.app()).contains(&dialog_id(KIND)));
            let app = panel.app();
            let state = app.session.active().unwrap();
            let symbol = state.doc.symbols.iter().find(|symbol| symbol.name == "Keyboard badge").unwrap();
            assert_eq!(symbol.dynamic, editing);
            assert_eq!(state.doc.symbol_pixel_aligned("Keyboard badge"), !editing);
            assert_eq!(
                state.doc.symbol_nine_slice("Keyboard badge"),
                Some(vectorcraft_doc::SymbolNineSlice { left: 0.1, top: 0.2, right: 0.3, bottom: 0.4 })
            );
            assert_eq!(state.doc.symbols.len(), 1);
            assert_eq!(state.history.undo.len(), undo + 1);
        }
    }

    #[test]
    fn editing_symbol_options_does_not_rename_another_drawings_symbol() {
        let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
        for side in [30, 50] {
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": side, "height": side})).unwrap();
            app.run("symbol.new", json!({"name": "Badge"})).unwrap();
        }
        let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
        let other = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();

        activate(&mut app, 0);
        open_edit(&mut app, Some("Badge")).unwrap();
        let id = dialog_id(KIND);
        set(&app, id, "name", json!("Source badge"));
        activate(&mut app, 1);
        accept(&mut app, id);
        assert_eq!(app.session.active().unwrap().doc, other);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);

        activate(&mut app, 0);
        accept(&mut app, id);
        assert!(app.session.active().unwrap().doc.symbols.iter().any(|symbol| symbol.name == "Source badge"));
    }

    #[test]
    fn new_symbol_does_not_replace_art_selected_after_opening() {
        let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let original = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
        let other = app.run("shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [original]})).unwrap();
        open(&mut app, false).unwrap();
        let id = dialog_id(KIND);
        set(&app, id, "name", json!("Badge"));
        app.run("select.set", json!({"ids": [other]})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();

        answer_dialog(&mut app, id, &[], true);
        assert!(app.session.active().unwrap().doc == before, "OK cannot turn the later selection into a symbol");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);

        app.run("select.set", json!({"ids": [original]})).unwrap();
        answer_dialog(&mut app, id, &[], true);
        let symbols = app.run("symbol.list", json!({})).unwrap();
        assert_eq!(symbols["symbols"][0]["name"], "Badge");
        assert_eq!(symbols["symbols"][0]["size"], json!([30.0, 30.0]), "the original art becomes the symbol");
        assert!(app.session.active().unwrap().doc.node(NodeId(other)).is_some(), "the unrelated rectangle remains");
    }

    #[test]
    fn editing_symbol_options_keeps_the_definition_restored_by_revert() {
        let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap();
        app.run("symbol.new", json!({"name": "Badge"})).unwrap();
        let file = std::env::temp_dir().join(format!("vectorcraft-symbol-options-revert-{}.vectorcraft", std::process::id()));
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|err| err.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|err| err.to_string())));
        app.run("file.saveAs", json!({"path": file.to_string_lossy()})).unwrap();
        let st = app.session.active().unwrap();
        let (uid, reverts) = (st.uid, st.reverts);

        app.run("symbol.options", json!({"name": "Badge", "registration": 0})).unwrap();
        open_edit(&mut app, Some("Badge")).unwrap();
        let id = dialog_id(KIND);
        set(&app, id, "name", json!("Edited badge"));
        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let _ = std::fs::remove_file(file);
        let st = app.session.active().unwrap();
        assert_eq!((st.uid, st.reverts), (uid, reverts + 1));
        assert_eq!(st.doc.symbol_registration("Badge").0, 4);
        let restored = st.doc.clone();
        let undo = st.history.undo.len();

        answer_dialog(&mut app, id, &[], true);
        assert_eq!(app.session.active().unwrap().doc.symbols[0].name, "Badge", "the restored symbol keeps its name");
        assert_eq!(app.session.active().unwrap().doc, restored);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);

        dismiss(&mut app, id);
        open_edit(&mut app, Some("Badge")).unwrap();
        set(&app, id, "name", json!("New badge"));
        accept(&mut app, id);
        assert_eq!(app.session.active().unwrap().doc.symbols[0].name, "New badge");
    }
}
