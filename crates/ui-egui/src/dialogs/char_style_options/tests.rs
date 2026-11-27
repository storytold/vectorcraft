use super::*;
// PLAN: assumed paths for the framework items; the hosted dialogs come from type-w21, the pump from hub-w2.
use crate::dialogs::new_text_style::NewTextStyle;
use craft::ui::dialog::{DialogHost, DialogId, StackDialog};
use craft::ui::patterns::form_dialogs::{FieldValue, PropertiesDialog};
use vectorcraft_doc::DocSetup;
use vectorcraft_engine::Session;

/// The dialog of an existing style.
type Existing = CharStyleOptions;
/// The dialog that makes a style.
type Fresh = NewTextStyle;

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app
}

/// Opens Character Style Options on the style `name` through its command.
fn open(app: &mut VectorcraftApp, name: &str) {
    app.run("ui.charStyleOptions", json!({"name": name})).unwrap();
}

/// Runs `f` on the host with the id of the open dialog `D`; never call a session method inside `f`.
fn with_dialog<D: StackDialog, R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost, DialogId) -> R) -> R {
    let dialogs = app.dialogs().expect("the window has a dialog host");
    dialogs.with(|host| {
        let id = host.get::<D>().expect("the dialog is open").id();
        f(host, id)
    })
}

fn is_open<D: StackDialog>(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.get::<D>().is_some()))
}

/// The open dialog's values as automation lists them.
fn fields<D: StackDialog>(app: &VectorcraftApp) -> Value {
    with_dialog::<D, _>(app, |host, id| host.get_by_id::<D>(id).and_then(|d| d.json_fields())).unwrap_or(Value::Null)
}

/// Sets a value as the user editing that control would.
fn set<D: StackDialog>(app: &VectorcraftApp, key: &str, value: Value) {
    let ctx = app.ui_ctx.clone().expect("a window");
    with_dialog::<D, _>(app, |host, id| host.set_json_field(&ctx, id, key, value)).unwrap_or_else(|why| panic!("{key}: {why}"));
}

/// One pass of the per-frame dialog pump: previews run, results are applied.
fn pump(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    crate::dialogs::host::pump(app, &ctx);
}

/// Presses OK; true when the dialog closed with its answer (a refused OK leaves it open).
fn ok<D: StackDialog>(app: &mut VectorcraftApp) -> bool {
    // A second round answers an OK the dialog held for the app.
    for _ in 0..2 {
        if !is_open::<D>(app) {
            break;
        }
        let ctx = app.ui_ctx.clone().expect("a window");
        with_dialog::<D, _>(app, |host, id| host.confirm_by_id(&ctx, id));
        pump(app);
    }
    !is_open::<D>(app)
}

fn cancel<D: StackDialog>(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    with_dialog::<D, _>(app, |host, id| host.cancel_by_id(&ctx, id));
    pump(app);
}

/// Presses Reset Panel on the shown tab.
fn reset_shown<D: StackDialog>(app: &VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    assert!(with_dialog::<D, _>(app, |host, id| host.reset_by_id(&ctx, id)), "the shown tab has a Reset Panel");
}

fn can_reset_shown<D: StackDialog>(app: &VectorcraftApp) -> bool {
    with_dialog::<D, _>(app, |host, id| host.can_reset_by_id(id))
}

/// A field's value as JSON; `Null` for a key the fields do not hold.
fn val(fields: &DialogFields, key: &str) -> Value {
    serde_json::to_value(fields).ok().and_then(|v| v.get(key).cloned()).unwrap_or(Value::Null)
}

fn put(fields: &mut DialogFields, key: &str, value: Value) {
    fields.set(key, serde_json::from_value::<FieldValue>(value).unwrap());
}

fn text(fields: &DialogFields, key: &str) -> String {
    fields.text(key).unwrap_or_default().to_string()
}

/// Resets the tab `key` of the fields as the footer's Reset Panel does.
fn reset(fields: &mut DialogFields, opening: &Opening, key: &str) {
    let tabs = tabs(fields, opening);
    let at = tabs.iter().position(|t| t.key.as_deref() == Some(key)).unwrap();
    // PLAN: assumed `PropertiesDialog::reset_tab(&PropertiesTab, usize, &mut DialogFields)` (ty.X78)
    PropertiesDialog::reset_tab(&tabs[at], at, fields);
}

fn can_reset(fields: &DialogFields, opening: &Opening, key: &str) -> bool {
    tabs(fields, opening).iter().any(|t| t.key.as_deref() == Some(key) && t.resettable)
}

fn attrs(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// The fields and the opening state of a dialog opened on a style with the attributes `original`.
fn dialog_for(original: Value) -> (DialogFields, Opening) {
    let map = attrs(original);
    let opening = Opening::new("", map, &DocSetup::default(), Value::Null);
    (opening.fields(""), opening)
}

/// A style, a text object in it, and the document before anything is edited.
fn styled(app: &mut VectorcraftApp, attrs: Value, text: &str) -> u64 {
    app.run("charStyle.new", json!({"name": "Body", "attrs": attrs})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": text})).unwrap()["id"].as_u64().unwrap();
    app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
    id
}

fn options_in_the_first_of_two_documents() -> VectorcraftApp {
    let mut app = app();
    styled(&mut app, json!({"size": 24.0}), "Opening document");
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    styled(&mut app, json!({"size": 12.0}), "Other document");
    app.run("document.activate", json!({"index": 0})).unwrap();
    open(&mut app, "Body");
    app
}

#[test]
fn the_style_preview_stays_in_its_opening_document() {
    let mut app = options_in_the_first_of_two_documents();
    let opening = app.session.doc().unwrap().doc.clone();
    set::<Existing>(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    app.run("document.activate", json!({"index": 1})).unwrap();
    let other = app.session.doc().unwrap().doc.clone();
    set::<Existing>(&app, "size", json!(18));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, other, "Preview leaves the other document's Body style unchanged");
    assert!(!app.session.in_interaction());

    app.run("document.activate", json!({"index": 0})).unwrap();
    pump(&mut app);
    assert!(app.session.doc().unwrap().doc.char_styles.iter().any(|style| style.name == "Revised"));
    cancel::<Existing>(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, opening);
}

#[test]
fn a_style_options_confirmation_stays_in_its_opening_document() {
    let mut app = options_in_the_first_of_two_documents();
    set::<Existing>(&app, "preview", json!(false));
    set::<Existing>(&app, "name", json!("Revised"));
    app.run("document.activate", json!({"index": 1})).unwrap();
    let other = app.session.doc().unwrap().doc.clone();
    let closed = ok::<Existing>(&mut app);

    assert_eq!(app.session.doc().unwrap().doc, other, "OK leaves the other document's Body style unchanged");
    assert!(!closed, "the same draft remains available");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(ok::<Existing>(&mut app));
    assert!(app.session.doc().unwrap().doc.char_styles.iter().any(|style| style.name == "Revised"));
}

#[test]
fn cancelling_style_options_leaves_the_other_documents_gesture() {
    let mut app = options_in_the_first_of_two_documents();
    set::<Existing>(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.session.begin_interaction("Another gesture").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    let gesture = app.session.doc().unwrap().doc.clone();

    cancel::<Existing>(&mut app);

    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

#[test]
fn switching_back_leaves_a_new_gesture_in_the_opening_document() {
    let mut app = options_in_the_first_of_two_documents();
    set::<Existing>(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.session.begin_interaction("Another gesture").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    let gesture = app.session.doc().unwrap().doc.clone();

    set::<Existing>(&app, "size", json!(18));
    pump(&mut app);
    assert!(!ok::<Existing>(&mut app));
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    cancel::<Existing>(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

fn options_with_a_newer_gesture() -> VectorcraftApp {
    let mut app = app();
    styled(&mut app, json!({"size": 24.0}), "Opening document");
    open(&mut app, "Body");
    set::<Existing>(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    let resets = app.session.tool_resets();
    app.session.cancel_interaction().unwrap();
    app.session.begin_interaction("Character Style Options").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    assert_eq!(app.session.tool_resets(), resets);
    app
}

#[test]
fn cancelling_style_options_preserves_a_newer_gesture_in_the_same_document() {
    let mut app = options_with_a_newer_gesture();
    let gesture = app.session.doc().unwrap().doc.clone();
    cancel::<Existing>(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

#[test]
fn style_options_preview_preserves_a_newer_gesture_in_the_same_document() {
    let mut app = options_with_a_newer_gesture();
    let gesture = app.session.doc().unwrap().doc.clone();
    set::<Existing>(&app, "size", json!(18));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

#[test]
fn style_options_confirmation_preserves_a_newer_gesture_in_the_same_document() {
    let mut app = options_with_a_newer_gesture();
    let gesture = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    assert!(!ok::<Existing>(&mut app));
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn a_first_style_preview_preserves_an_existing_gesture() {
    let mut app = app();
    styled(&mut app, json!({"size": 24.0}), "Opening document");
    open(&mut app, "Body");
    app.session.begin_interaction("Character Style Options").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    let gesture = app.session.doc().unwrap().doc.clone();
    set::<Existing>(&app, "size", json!(18));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

#[test]
fn unchanged_basic_fields_preserve_original_keys() {
    for attrs in [
        json!({"all_caps": false}),
        json!({"all_caps": true}),
        json!({"small_caps": 70.0}),
        json!({"font_family": "Example", "font_style": "Regular", "font_version": "1", "size": 24,
            "leading": null, "kerning": null, "tracking": 3, "underline": null,
            "strikethrough": false, "position": {"kind": "normal", "size": 80}, "charAlign": null}),
    ] {
        let original = attrs.as_object().unwrap().clone();
        let (fields, opening) = dialog_for(attrs);
        assert_eq!(edited_attrs(&fields, &opening).unwrap(), original);
    }
    let (mut fields, opening) = dialog_for(json!({"all_caps": false}));
    put(&mut fields, "case", json!("allCaps"));
    assert_eq!(edited_attrs(&fields, &opening).unwrap(), json!({"all_caps": true, "small_caps": null}).as_object().unwrap().clone());
}

#[test]
fn unchanged_options_keep_local_small_caps_and_history() {
    for preview in [false, true] {
        let mut app = app();
        let id = styled(&mut app, json!({"all_caps": false}), "Text");
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 4, "smallCaps": true})).unwrap();
        let state = app.session.doc().unwrap();
        let before = state.doc.clone();
        let history = (state.history.undo.len(), state.history.redo.len());
        open(&mut app, "Body");
        set::<Existing>(&app, "preview", json!(preview));
        if preview {
            pump(&mut app);
            assert_eq!(app.session.doc().unwrap().doc, before);
        }
        assert!(ok::<Existing>(&mut app));
        let state = app.session.doc().unwrap();
        assert_eq!(state.doc, before);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
        assert_eq!(
            state.doc.node(vectorcraft_doc::NodeId(id)).and_then(|node| match &node.kind {
                vectorcraft_doc::NodeKind::Text(text) => text.runs.first().map(|run| run.style.small_caps),
                _ => None,
            }),
            Some(Some(state.doc.setup.small_caps_size))
        );
    }
}

#[test]
fn options_preview_cancel_reset_and_commit_keep_sparse_definition() {
    let mut app = app();
    let id = styled(&mut app, json!({"size": 24.0, "baseline_shift": 3.0}), "Text");
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    open(&mut app, "Body");
    assert_eq!(fields::<Existing>(&app)["preview"], json!(true));
    assert!(!can_reset_shown::<Existing>(&app), "General has no Reset Panel");
    assert!(fields::<Existing>(&app)["underline"].is_null());
    assert_eq!(fields::<Existing>(&app)["kerning"], json!(""));
    set::<Existing>(&app, "name", json!("Revised"));
    set::<Existing>(&app, "size", json!(18));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc.char_styles[0].name, "Revised");
    cancel::<Existing>(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    open(&mut app, "Body");
    set::<Existing>(&app, "tab", json!("basic"));
    reset_shown::<Existing>(&app);
    set::<Existing>(&app, "name", json!("Revised"));
    assert!(ok::<Existing>(&mut app));
    let state = app.session.doc().unwrap();
    assert_eq!(state.history.undo.len(), undo + 1);
    assert_eq!(state.doc.char_styles[0].name, "Revised");
    assert_eq!(state.doc.char_styles[0].attrs, serde_json::from_value(json!({"baseline_shift": 3.0})).unwrap());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.char_styles[0].name, "Revised");
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("ui.newCharStyle", json!({})).unwrap();
    assert_eq!(fields::<Fresh>(&app)["size"], json!("12.0"));
    set::<Fresh>(&app, "size", json!(36.0));
    set::<Fresh>(&app, "name", json!("Future"));
    assert!(ok::<Fresh>(&mut app));
    assert_eq!(app.session.doc().unwrap().doc.char_styles.iter().find(|def| def.name == "Future").unwrap().attrs["size"], json!(36.0));
}

/// Standard Vertical Roman Alignment is its own on, off or unset value that the style saves.
#[test]
fn standard_vertical_roman_alignment_has_its_own_row() {
    let (mut d, o) = dialog_for(json!({}));
    assert!(val(&d, "standardVerticalRoman").is_null());
    assert_eq!(edited_attrs(&d, &o).unwrap(), Map::new());
    put(&mut d, "standardVerticalRoman", json!(true));
    put(&mut d, "charAlign", json!("emBoxCenter"));
    assert_eq!(edited_attrs(&d, &o).unwrap(), json!({"standardVerticalRoman": true, "charAlign": "emBoxCenter"}).as_object().unwrap().clone());
    let (off, _) = dialog_for(json!({"standardVerticalRoman": false}));
    assert_eq!(val(&off, "standardVerticalRoman"), json!(false));
    reset(&mut d, &o, "basic");
    assert!(val(&d, "standardVerticalRoman").is_null());

    let mut app = app();
    app.run("charStyle.new", json!({"name": "Upright", "attrs": {}})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "AB", "vertical": true})).unwrap()["id"].as_u64().unwrap();
    app.run("charStyle.apply", json!({"name": "Upright", "id": id})).unwrap();
    open(&mut app, "Upright");
    set::<Existing>(&app, "standardVerticalRoman", json!(true));
    assert!(ok::<Existing>(&mut app));
    let doc = &app.session.doc().unwrap().doc;
    assert_eq!(doc.char_styles[0].attrs["standardVerticalRoman"], json!(true));
    let vectorcraft_doc::NodeKind::Text(t) = &doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(t.runs.iter().all(|r| r.style.standard_vertical_roman));
}

#[test]
fn advanced_fields_show_set_values_and_keep_untouched_ones() {
    let (d, o) = dialog_for(json!({}));
    for key in ADVANCED_NUMBERS {
        assert_eq!(val(&d, key), json!(""), "{key}");
    }
    assert!(val(&d, "language").is_null());
    assert_eq!(edited_attrs(&d, &o).unwrap(), Map::new());
    for original in [
        json!({"h_scale": 100.0, "rotation": 0.0, "language": null}),
        json!({"v_scale": 50.0, "baseline_shift": -3.0, "language": "de"}),
        json!({"language": "x-file", "size": 12.0}),
        // Values a style captures from text in a scaled type object, Size and Leading below 0.1 included.
        json!({"h_scale": 20000.0, "baseline_shift": 3000.0, "size": 0.05, "leading": 0.05}),
    ] {
        let (d, o) = dialog_for(original.clone());
        assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(original.clone()), "{original}");
    }
    let (d, _) = dialog_for(json!({"h_scale": 100.0, "rotation": 0.0, "language": null}));
    assert_eq!((val(&d, "h_scale"), val(&d, "v_scale")), (json!("100.0"), json!("")));
    assert_eq!(val(&d, "language"), json!("English: USA"));
    assert_eq!(val(&dialog_for(json!({"language": "de"})).0, "language"), json!("German"));
    assert_eq!(val(&dialog_for(json!({"language": "x-file"})).0, "language"), json!("x-file"));

    let (mut d, o) = dialog_for(json!({"h_scale": 100.0, "language": "de"}));
    put(&mut d, "h_scale", json!(""));
    put(&mut d, "v_scale", json!("150"));
    put(&mut d, "baseline_shift", json!("-4"));
    put(&mut d, "rotation", json!("190"));
    put(&mut d, "language", json!("English: USA"));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"v_scale": 150.0, "baseline_shift": -4.0, "rotation": -170.0, "language": null})));
    put(&mut d, "language", json!("French"));
    assert_eq!(edited_attrs(&d, &o).unwrap()["language"], json!("fr"));
    put(&mut d, "language", Value::Null);
    assert!(!edited_attrs(&d, &o).unwrap().contains_key("language"));
}

#[test]
fn character_style_numbers_outside_their_limits_are_errors() {
    for (key, value, message) in [
        ("size", "1296.1", "Size must be a number at most 1296"),
        ("leading", "5000.1", "Leading must be a number at most 5000"),
        ("h_scale", "0", "Horizontal Scale must be a number from 1 to 10000"),
        ("h_scale", "0.9", "Horizontal Scale must be a number from 1 to 10000"),
        ("v_scale", "10000.1", "Vertical Scale must be a number from 1 to 10000"),
        ("baseline_shift", "1296.1", "Baseline Shift must be a number from -1296 to 1296"),
        ("baseline_shift", "-1296.1", "Baseline Shift must be a number from -1296 to 1296"),
        ("rotation", "inf", "Character Rotation must be a number"),
        ("h_scale", "wide", "Horizontal Scale must be a number"),
    ] {
        let (mut d, o) = dialog_for(json!({}));
        put(&mut d, key, json!(value));
        assert_eq!(edited_attrs(&d, &o).unwrap_err(), message, "{key} {value}");
    }
    let (mut d, o) = dialog_for(json!({}));
    for (key, value) in
        [("size", "1296"), ("leading", "5000"), ("h_scale", "1"), ("v_scale", "10000"), ("baseline_shift", "-1296"), ("rotation", "-200")]
    {
        put(&mut d, key, json!(value));
    }
    assert_eq!(
        edited_attrs(&d, &o).unwrap(),
        attrs(json!({"size": 1296.0, "leading": 5000.0, "h_scale": 1.0, "v_scale": 10000.0, "baseline_shift": -1296.0, "rotation": 160.0}))
    );
}

#[test]
fn advanced_options_preview_cancel_reset_and_commit() {
    let mut app = app();
    let id = styled(&mut app, json!({"size": 24.0, "h_scale": 90.0, "baseline_shift": 0.0}), "Text");
    app.run("text.setRangeStyle", json!({"id": id, "start": 2, "end": 4, "baselineShift": 5.0})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    open(&mut app, "Body");
    assert_eq!(fields::<Existing>(&app)["h_scale"], json!("90.0"));
    assert_eq!(fields::<Existing>(&app)["rotation"], json!(""));
    set::<Existing>(&app, "tab", json!("advanced"));
    set::<Existing>(&app, "h_scale", json!("80"));
    set::<Existing>(&app, "baseline_shift", json!("2"));
    set::<Existing>(&app, "rotation", json!("45"));
    set::<Existing>(&app, "language", json!("Turkish"));
    assert!(can_reset_shown::<Existing>(&app));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["rotation"], json!(45.0));
    cancel::<Existing>(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

    open(&mut app, "Body");
    set::<Existing>(&app, "tab", json!("advanced"));
    set::<Existing>(&app, "size", json!(18));
    set::<Existing>(&app, "v_scale", json!("70"));
    reset_shown::<Existing>(&app);
    assert_eq!(fields::<Existing>(&app)["v_scale"], json!(""), "Reset Panel empties the Advanced fields");
    set::<Existing>(&app, "h_scale", json!("80"));
    set::<Existing>(&app, "v_scale", json!("60"));
    set::<Existing>(&app, "baseline_shift", json!("2"));
    set::<Existing>(&app, "rotation", json!("45"));
    set::<Existing>(&app, "language", json!("Turkish"));
    assert!(ok::<Existing>(&mut app));
    let state = app.session.doc().unwrap();
    assert_eq!(state.history.undo.len(), undo + 1);
    assert_eq!(
        state.doc.char_styles[0].attrs,
        attrs(json!({"size": 18.0, "h_scale": 80.0, "v_scale": 60.0, "baseline_shift": 2.0, "rotation": 45.0, "language": "tr"}))
    );
    let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    assert_eq!(
        t.runs
            .iter()
            .map(|r| (r.style.h_scale, r.style.v_scale, r.style.baseline_shift, r.style.rotation, r.style.language.clone()))
            .collect::<Vec<_>>(),
        [(80.0, 60.0, 2.0, 45.0, Some("tr".to_string())), (80.0, 60.0, 5.0, 45.0, Some("tr".to_string()))]
    );
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

#[test]
fn new_character_style_sets_and_resets_advanced_formats() {
    let mut app = app();
    app.run("ui.newCharStyle", json!({})).unwrap();
    set::<Fresh>(&app, "tab", json!("advanced"));
    set::<Fresh>(&app, "v_scale", json!("130"));
    reset_shown::<Fresh>(&app);
    assert_eq!(fields::<Fresh>(&app)["v_scale"], json!(""));
    set::<Fresh>(&app, "name", json!("Raised"));
    set::<Fresh>(&app, "baseline_shift", json!("3"));
    set::<Fresh>(&app, "language", json!("French"));
    assert!(ok::<Fresh>(&mut app));
    let doc = &app.session.doc().unwrap().doc;
    let def = doc.char_styles.iter().find(|def| def.name == "Raised").unwrap();
    assert_eq!((def.attrs["baseline_shift"].clone(), def.attrs["language"].clone()), (json!(3.0), json!("fr")));
    assert!(!def.attrs.contains_key("v_scale"));
}

#[test]
fn tate_chu_yoko_offsets_show_set_reset_and_keep_their_limits() {
    let (d, o) = dialog_for(json!({"tcyUpDown": 2.0, "size": 12.0}));
    assert_eq!((val(&d, "tcyUpDown"), val(&d, "tcyLeftRight")), (json!("2.0"), json!("")));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"tcyUpDown": 2.0, "size": 12.0})));
    // A value a style captured from scaled text stays while its field is untouched.
    let (d, o) = dialog_for(json!({"tcyLeftRight": 3000.0}));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"tcyLeftRight": 3000.0})));

    let (mut d, o) = dialog_for(json!({"tcyUpDown": 2.0}));
    put(&mut d, "tcyUpDown", json!(""));
    put(&mut d, "tcyLeftRight", json!("-1296"));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"tcyLeftRight": -1296.0})));
    put(&mut d, "tcyLeftRight", json!("0"));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"tcyLeftRight": 0.0})));
    for (key, value, message) in [
        ("tcyUpDown", "1296.1", "Tate-chu-yoko Up/Down must be a number from -1296 to 1296"),
        ("tcyLeftRight", "-1296.1", "Tate-chu-yoko Left/Right must be a number from -1296 to 1296"),
        ("tcyUpDown", "high", "Tate-chu-yoko Up/Down must be a number"),
        ("tcyLeftRight", "NaN", "Tate-chu-yoko Left/Right must be a number"),
    ] {
        let (mut d, o) = dialog_for(json!({}));
        put(&mut d, key, json!(value));
        assert_eq!(edited_attrs(&d, &o).unwrap_err(), message, "{key} {value}");
    }

    let (mut d, o) = dialog_for(json!({"tcyUpDown": 2.0, "tcyLeftRight": -1.0, "size": 12.0}));
    assert!(can_reset(&d, &o, "advanced"));
    reset(&mut d, &o, "advanced");
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"size": 12.0})));
}

#[test]
fn tate_chu_yoko_offsets_preview_cancel_and_commit_to_the_style_and_its_text() {
    let mut app = app();
    let id = styled(&mut app, json!({"size": 24.0, "tcyLeftRight": 0.0}), "Text");
    app.run("text.setRangeStyle", json!({"id": id, "start": 2, "end": 4, "tcyLeftRight": 5.0})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    open(&mut app, "Body");
    set::<Existing>(&app, "tab", json!("advanced"));
    set::<Existing>(&app, "tcyUpDown", json!("4"));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["tcyUpDown"], json!(4.0));
    cancel::<Existing>(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

    open(&mut app, "Body");
    set::<Existing>(&app, "tcyUpDown", json!("4"));
    set::<Existing>(&app, "tcyLeftRight", json!("-3"));
    assert!(ok::<Existing>(&mut app));
    let state = app.session.doc().unwrap();
    assert_eq!(state.history.undo.len(), undo + 1);
    assert_eq!(state.doc.char_styles[0].attrs, attrs(json!({"size": 24.0, "tcyUpDown": 4.0, "tcyLeftRight": -3.0})));
    let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    assert_eq!(t.runs.iter().map(|r| (r.style.tcy_up_down, r.style.tcy_left_right)).collect::<Vec<_>>(), [(4.0, -3.0), (4.0, 5.0)]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);

    app.run("ui.newCharStyle", json!({})).unwrap();
    set::<Fresh>(&app, "name", json!("Shifted"));
    set::<Fresh>(&app, "tcyLeftRight", json!("6"));
    assert!(ok::<Fresh>(&mut app));
    let doc = &app.session.doc().unwrap().doc;
    let def = doc.char_styles.iter().find(|def| def.name == "Shifted").unwrap();
    assert_eq!(def.attrs["tcyLeftRight"], json!(6.0));
}

#[test]
fn insert_aki_fields_show_set_reset_and_stop_at_the_character_panel_limits() {
    let (d, o) = dialog_for(json!({"akiLeft": 250.0, "size": 12.0}));
    assert_eq!((val(&d, "akiLeft"), val(&d, "akiRight")), (json!("250.0"), json!("")));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"akiLeft": 250.0, "size": 12.0})));

    let (mut d, o) = dialog_for(json!({"akiLeft": 250.0}));
    put(&mut d, "akiLeft", json!(""));
    put(&mut d, "akiRight", json!("500"));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"akiRight": 500.0})));
    // A typed value stops at 0 and at 1000, as in the Character panel's fields.
    for (typed, kept) in [("5000", 1000.0), ("-5", 0.0), ("1000", 1000.0)] {
        put(&mut d, "akiRight", json!(typed));
        assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"akiRight": kept})), "{typed}");
    }
    put(&mut d, "akiLeft", json!("wide"));
    assert_eq!(edited_attrs(&d, &o).unwrap_err(), "Insert Aki Left must be a number");

    let (mut d, o) = dialog_for(json!({"akiLeft": 250.0, "akiRight": 500.0, "size": 12.0}));
    reset(&mut d, &o, "advanced");
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"size": 12.0})));
}

#[test]
fn insert_aki_fields_commit_to_the_style_and_its_text() {
    let mut app = app();
    let id = styled(&mut app, json!({"size": 24.0}), "Text");
    open(&mut app, "Body");
    set::<Existing>(&app, "akiLeft", json!("250"));
    set::<Existing>(&app, "akiRight", json!("2000"));
    assert!(ok::<Existing>(&mut app));
    let state = app.session.doc().unwrap();
    assert_eq!(state.doc.char_styles[0].attrs, attrs(json!({"size": 24.0, "akiLeft": 250.0, "akiRight": 1000.0})));
    let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(t.runs.iter().all(|r| (r.style.aki_left, r.style.aki_right) == (250.0, 1000.0)));
}

/// Adjust Diacritic Positions: empty is unset, a typed value stops at -1000 and 1000, Reset Panel
/// unsets them, and they commit to the style and the text set in it.
#[test]
fn diacritic_position_fields_show_set_reset_and_commit() {
    let (d, o) = dialog_for(json!({"diacriticHorizontal": -250.0, "size": 12.0}));
    assert_eq!((val(&d, "diacriticHorizontal"), val(&d, "diacriticVertical")), (json!("-250.0"), json!("")));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"diacriticHorizontal": -250.0, "size": 12.0})));
    let (mut d, o) = dialog_for(json!({"diacriticHorizontal": -250.0}));
    put(&mut d, "diacriticHorizontal", json!(""));
    for (typed, kept) in [("0", 0.0), ("150", 150.0), ("5000", 1000.0), ("-5000", -1000.0)] {
        put(&mut d, "diacriticVertical", json!(typed));
        assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"diacriticVertical": kept})), "{typed}");
    }
    put(&mut d, "diacriticHorizontal", json!("left"));
    assert_eq!(edited_attrs(&d, &o).unwrap_err(), "Adjust Horizontal Diacritic Position must be a number");
    let (mut d, o) = dialog_for(json!({"diacriticHorizontal": 100.0, "diacriticVertical": 200.0, "size": 12.0}));
    reset(&mut d, &o, "advanced");
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"size": 12.0})));

    let mut app = app();
    let id = styled(&mut app, json!({"size": 24.0}), "بَب");
    open(&mut app, "Body");
    set::<Existing>(&app, "tab", json!("advanced"));
    set::<Existing>(&app, "diacriticHorizontal", json!("-120"));
    set::<Existing>(&app, "diacriticVertical", json!("3000"));
    assert!(ok::<Existing>(&mut app));
    let state = app.session.doc().unwrap();
    assert_eq!(state.doc.char_styles[0].attrs, attrs(json!({"size": 24.0, "diacriticHorizontal": -120.0, "diacriticVertical": 1000.0})));
    let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(t.runs.iter().all(|r| (r.style.diacritic_horizontal, r.style.diacritic_vertical) == (-120.0, 1000.0)));
}

/// The number fields read typed text as the app's other number fields do: `6x2`, `10+4`, `200/2%`,
/// `45°`, a decimal comma, and a length with its unit. `%` and `°` only where the field takes them,
/// and an exponent is not read.
#[test]
fn number_fields_take_field_arithmetic() {
    let typed = [
        ("size", "6x2", 12.0),
        ("tracking", "10+4", 14.0),
        ("h_scale", "200/2%", 100.0),
        ("rotation", "45°", 45.0),
        ("fill_tint", "25*2", 50.0),
        ("leading", "1 in", 72.0),
        ("baseline_shift", "\u{2212}2", -2.0),
        ("kerning", "12,5", 12.5),
        ("h_scale", "12,5%", 12.5),
        ("rotation", "12,5°", 12.5),
    ];
    for (key, text, want) in typed {
        let (mut d, _) = dialog_for(json!({}));
        put(&mut d, key, json!(text));
        assert_eq!(optional_number(&Values::new(&d), key), Ok(Some(json!(want))), "{key} {text}");
    }
    for (key, text) in [
        ("size", "12°"),
        ("leading", "50%"),
        ("tracking", "10%"),
        ("rotation", "45%"),
        ("h_scale", "1%2"),
        ("size", "1e3"),
        ("h_scale", "1,000"),
        ("h_scale", "1,000%"),
        ("rotation", "1,000°"),
    ] {
        let (mut d, _) = dialog_for(json!({}));
        put(&mut d, key, json!(text));
        assert!(optional_number(&Values::new(&d), key).is_err(), "{key} {text}");
    }
}

/// Tsume: empty is unset, an explicit 0 stays set, a typed value stops at 0 and 100, Reset Panel
/// unsets it, and it commits to the style and the text set in it.
#[test]
fn tsume_field_shows_sets_resets_and_commits() {
    let (d, _) = dialog_for(json!({"size": 12.0}));
    assert_eq!(val(&d, "tsume"), json!(""));
    let (mut d, o) = dialog_for(json!({"tsume": 50.0}));
    assert_eq!(val(&d, "tsume"), json!("50.0"));
    for (typed, kept) in [("0", 0.0), ("25.5", 25.5), ("25%", 25.0), ("50/2%", 25.0), ("150", 100.0), ("-5", 0.0)] {
        put(&mut d, "tsume", json!(typed));
        assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"tsume": kept})), "{typed}");
    }
    put(&mut d, "tsume", json!(""));
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({})));
    put(&mut d, "tsume", json!("tight"));
    assert_eq!(edited_attrs(&d, &o).unwrap_err(), "Tsume must be a number");
    let (mut d, o) = dialog_for(json!({"tsume": 50.0, "size": 12.0}));
    reset(&mut d, &o, "advanced");
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"size": 12.0})));

    let mut app = app();
    let id = styled(&mut app, json!({"size": 24.0}), "Text");
    open(&mut app, "Body");
    set::<Existing>(&app, "tab", json!("advanced"));
    set::<Existing>(&app, "tsume", json!("75"));
    assert!(ok::<Existing>(&mut app));
    let state = app.session.doc().unwrap();
    assert_eq!(state.doc.char_styles[0].attrs, attrs(json!({"size": 24.0, "tsume": 75.0})));
    let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(t.runs.iter().all(|r| r.style.tsume == 75.0));
}

/// The Insert Aki, Tsume and diacritic rows each have the Character panel's presets menu beside the field, and a
/// preset picked from it is the field's value and commits to the style, with each frame drawn once.
#[test]
fn aki_tsume_and_diacritic_rows_pick_from_the_character_panel_presets() {
    use craft::ui::testing::Fixture;
    let mut fx = Fixture::new(|ui| crate::dialogs::host::window_dialogs(ui.ctx()).with(|host| host.run(ui)));
    fx.room(egui::vec2(900.0, 900.0));
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui_ctx = Some(fx.ctx().clone());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    styled(&mut app, json!({"size": 24.0}), "Text");
    open(&mut app, "Body");
    set::<Existing>(&app, "tab", json!("advanced"));
    fx.settle();

    let presets_in_row = |fx: &Fixture, label: &str| {
        let field = fx.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        let tree = fx.tree();
        tree.keys().into_iter().filter(|key| tree.name_of(*key) == Some("Presets")).filter_map(|key| fx.target(key)).find(|chevron| {
            chevron.rect.center().y > field.rect.top() && chevron.rect.center().y < field.rect.bottom() && chevron.rect.left() >= field.rect.left()
        })
    };
    for label in ["Insert Aki Left", "Insert Aki Right", "Tsume (%)", "Adjust Horizontal Diacritic Position", "Adjust Vertical Diacritic Position"] {
        assert!(presets_in_row(&fx, label).is_some(), "{label} has a presets menu");
    }

    let chevron = presets_in_row(&fx, "Insert Aki Left").unwrap();
    fx.click(&chevron).settle();
    let pick = fx.by_label("500").expect("the 500 preset");
    fx.click(&pick).settle();
    assert_eq!(fields::<Existing>(&app)["akiLeft"], json!("500"));
    assert!(ok::<Existing>(&mut app));
    assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs, attrs(json!({"size": 24.0, "akiLeft": 500.0})));
}

/// The negated tags of the twenty stylistic sets: an explicit empty Stylistic Sets choice.
fn no_sets() -> Vec<String> {
    vectorcraft_text::EXTRA_FEATURES[..20].iter().map(|s| format!("-{s}")).collect()
}

#[test]
fn color_and_opentype_fields_keep_unset_explicit_and_untouched_values() {
    let (d, o) = dialog_for(json!({}));
    for key in ["fill", "stroke", "overprint_fill", "overprint_stroke", "ot_figure", "ot_position", "ot_liga", "ot_frac", "ot_sets"] {
        assert!(val(&d, key).is_null(), "{key}");
    }
    assert_eq!((text(&d, "stroke_width"), text(&d, "fill_tint")), (String::new(), String::new()));
    assert_eq!(edited_attrs(&d, &o).unwrap(), Map::new());

    let mut explicit =
        vec!["-tnum", "-onum", "-pnum", "-lnum", "-sups", "-subs", "-numr", "-dnom", "-liga"].into_iter().map(String::from).collect::<Vec<_>>();
    explicit.extend(no_sets());
    for original in [
        json!({"fill": {"type": "none"}, "stroke_width": 0.0, "overprint_fill": false, "features": explicit}),
        json!({"stroke": {"type": "pattern", "pattern": "Dots"}, "fill": {"type": "solid", "color": {"model": "gray", "k": 0.5}}}),
        json!({"features": ["zzzz", "dlig", "ss03"], "size": 9.0}),
    ] {
        let (d, o) = dialog_for(original.clone());
        assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(original.clone()), "{original}");
    }
    let (d, _) = dialog_for(json!({"fill": {"type": "none"}, "overprint_fill": false, "features": explicit}));
    assert_eq!(val(&d, "fill"), json!({"type": "none"}));
    assert_eq!(val(&d, "overprint_fill"), json!(false));
    assert_eq!((text(&d, "ot_figure"), text(&d, "ot_position")), ("default".to_string(), "default".to_string()));
    assert_eq!((val(&d, "ot_liga"), val(&d, "ot_dlig"), val(&d, "ot_sets")), (json!(false), Value::Null, json!(no_sets())));

    // One changed control keeps every other tag, unknown ones included.
    let (mut d, o) = dialog_for(json!({"features": ["zzzz", "dlig", "ss03"], "stroke": {"type": "pattern", "pattern": "Dots"}}));
    assert_eq!((val(&d, "ot_dlig"), val(&d, "ot_sets")), (json!(true), json!(["ss03"])));
    put(&mut d, "ot_liga", json!(false));
    put(&mut d, "overprint_stroke", json!(true));
    assert_eq!(
        edited_attrs(&d, &o).unwrap(),
        attrs(json!({"features": ["zzzz", "dlig", "ss03", "-liga"], "stroke": {"type": "pattern", "pattern": "Dots"}, "overprint_stroke": true}))
    );
    put(&mut d, "ot_sets", json!(["ss05", "ss03"]));
    put(&mut d, "ot_figure", json!("proportionalOldstyle"));
    put(&mut d, "ot_position", json!("numerator"));
    put(&mut d, "ot_dlig", Value::Null);
    let features = edited_attrs(&d, &o).unwrap()["features"].clone();
    // Sets the list leaves out stay unset, so text that uses the style keeps its own.
    assert_eq!(features, json!(["zzzz", "-liga", "onum", "pnum", "-tnum", "-lnum", "-sups", "-subs", "numr", "-dnom", "ss05", "ss03"]));
    let (reopened, _) = dialog_for(json!({"features": features}));
    assert_eq!(
        ["ot_figure", "ot_position", "ot_liga", "ot_dlig"].map(|k| val(&reopened, k)),
        [json!("proportionalOldstyle"), json!("numerator"), json!(false), Value::Null]
    );
    assert_eq!(val(&reopened, "ot_sets"), json!(["ss03", "ss05"]));
    // Every control unset leaves the attribute unset.
    let (mut d, o) = dialog_for(json!({"features": ["dlig"]}));
    put(&mut d, "ot_dlig", Value::Null);
    assert_eq!(edited_attrs(&d, &o).unwrap(), Map::new());
}

#[test]
fn a_stored_zero_value_reads_as_off_and_turning_it_on_saves_it() {
    let (mut d, o) = dialog_for(json!({"features": ["ss01=0", "salt=0", "ss02=2"]}));
    assert_eq!(val(&d, "ot_salt"), json!(false));
    assert_eq!(val(&d, "ot_sets"), json!(["-ss01", "ss02"]));
    put(&mut d, "ot_sets", json!(["ss01", "ss02"]));
    put(&mut d, "ot_salt", json!(true));
    assert_eq!(edited_attrs(&d, &o).unwrap()["features"], json!(["salt", "ss01", "ss02=2"]));
    for last in ["ss01=0", "-ss01", "ss01=65536"] {
        let (mut d, o) = dialog_for(json!({"features": ["ss01=2", last]}));
        put(&mut d, "ot_sets", json!(["ss01", "ss02"]));
        assert_eq!(edited_attrs(&d, &o).unwrap()["features"], json!(["ss01", "ss02"]), "{last}: only a valid last on-value is kept");
    }
}

#[test]
fn figure_tags_no_menu_entry_matches_leave_the_figure_unset() {
    for features in [json!(["pnum"]), json!(["lnum"]), json!(["onum", "lnum"]), json!(["tnum", "pnum"])] {
        let (mut d, o) = dialog_for(json!({ "features": features }));
        assert!(val(&d, "ot_figure").is_null(), "{features}");
        assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({ "features": features })), "{features}");
        put(&mut d, "ot_figure", json!("default"));
        assert_eq!(edited_attrs(&d, &o).unwrap()["features"], json!(["-tnum", "-onum", "-pnum", "-lnum"]), "{features}");
    }
    for (features, shown) in [(json!(["tnum"]), "tabularLining"), (json!(["-pnum"]), "default"), (json!(["pnum", "lnum"]), "proportionalLining")] {
        assert_eq!(text(&dialog_for(json!({ "features": features })).0, "ot_figure"), shown, "{features}");
    }
}

#[test]
fn color_tint_and_weight_outside_their_limits_are_errors() {
    let linked = json!({"type": "solid", "color": {"model": "gray", "k": 0.5}, "swatch": "Ink"});
    for (key, value, message) in [
        ("fill_tint", "150", "Fill Tint must be a number from 0 to 100"),
        ("stroke_tint", "-1", "Stroke Tint must be a number from 0 to 100"),
        ("stroke_width", "-1", "Weight must be a number from 0 to 4000000"),
        ("stroke_width", "thick", "Weight must be a number"),
    ] {
        let (mut d, o) = dialog_for(json!({"fill": linked, "stroke": linked}));
        put(&mut d, key, json!(value));
        assert_eq!(edited_attrs(&d, &o).unwrap_err(), message, "{key} {value}");
    }
    let (mut d, o) = dialog_for(json!({"fill": linked}));
    assert_eq!(val(&d, "fill_tint"), json!("100.0"));
    put(&mut d, "fill_tint", json!("40"));
    put(&mut d, "stroke_width", json!("1.5"));
    assert_eq!(
        edited_attrs(&d, &o).unwrap(),
        attrs(json!({"fill": {"type": "solid", "color": {"model": "gray", "k": 0.5}, "swatch": "Ink", "tint": 0.4}, "stroke_width": 1.5}))
    );
}

#[test]
fn reset_panel_in_color_and_opentype_clears_only_that_section() {
    let original =
        json!({"size": 12.0, "h_scale": 90.0, "fill": {"type": "none"}, "stroke_width": 2.0, "overprint_stroke": true, "features": ["dlig", "zzzz"]});
    let (mut d, o) = dialog_for(original.clone());
    assert!(can_reset(&d, &o, "color"));
    reset(&mut d, &o, "color");
    assert_eq!(edited_attrs(&d, &o).unwrap(), attrs(json!({"size": 12.0, "h_scale": 90.0, "features": ["dlig", "zzzz"]})));
    let (mut d, o) = dialog_for(original);
    reset(&mut d, &o, "opentype");
    assert!(val(&d, "ot_dlig").is_null());
    assert_eq!(
        edited_attrs(&d, &o).unwrap(),
        attrs(json!({"size": 12.0, "h_scale": 90.0, "fill": {"type": "none"}, "stroke_width": 2.0, "overprint_stroke": true}))
    );
    put(&mut d, "ot_frac", json!(true));
    assert_eq!(edited_attrs(&d, &o).unwrap()["features"], json!(["frac"]));
}

/// H or V Style and Roman Italics are on, off or unset switches of OpenType Features: they show
/// the style's `hkna` and `ital` tags, a changed switch replaces its tag and keeps the others, an
/// unset one removes it, and a style without them keeps none.
#[test]
fn kana_style_and_roman_italics_are_opentype_switches() {
    let (mut d, o) = dialog_for(json!({"features": ["hkna", "-ital", "dlig"]}));
    assert_eq!((val(&d, "ot_hkna"), val(&d, "ot_ital")), (json!(true), json!(false)));
    assert_eq!(edited_attrs(&d, &o).unwrap()["features"], json!(["hkna", "-ital", "dlig"]), "untouched");
    put(&mut d, "ot_ital", json!(true));
    put(&mut d, "ot_hkna", Value::Null);
    assert_eq!(edited_attrs(&d, &o).unwrap()["features"], json!(["dlig", "ital"]));
    let (mut d, o) = dialog_for(json!({}));
    assert!(val(&d, "ot_hkna").is_null() && val(&d, "ot_ital").is_null());
    put(&mut d, "ot_hkna", json!(true));
    assert_eq!(edited_attrs(&d, &o).unwrap()["features"], json!(["hkna"]));
}

#[test]
fn color_and_features_preview_cancel_commit_and_new_style() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Brand", "color": "#2050a0", "global": true})).unwrap();
    let id = styled(&mut app, json!({"size": 24.0}), "Text");
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let brand = json!(before.tint_paint("Brand", 1.0).unwrap());
    let edit = |app: &VectorcraftApp| {
        set::<Existing>(app, "tab", json!("color"));
        set::<Existing>(app, "fill", brand.clone());
        set::<Existing>(app, "stroke", brand.clone());
        set::<Existing>(app, "stroke_tint", json!("40"));
        set::<Existing>(app, "stroke_width", json!("2"));
        set::<Existing>(app, "overprint_fill", json!(true));
        set::<Existing>(app, "ot_dlig", json!(true));
        set::<Existing>(app, "ot_figure", json!("tabularLining"));
    };
    open(&mut app, "Body");
    edit(&app);
    assert!(can_reset_shown::<Existing>(&app));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["fill"], brand);
    cancel::<Existing>(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

    open(&mut app, "Body");
    edit(&app);
    assert!(ok::<Existing>(&mut app));
    let state = app.session.doc().unwrap();
    assert_eq!(state.history.undo.len(), undo + 1);
    let tint40 = state.doc.tint_paint("Brand", 0.4).unwrap();
    let def = state.doc.char_styles[0].attrs.clone();
    assert_eq!(def["stroke"], json!(tint40));
    assert_eq!(def["features"], json!(["dlig", "tnum", "lnum", "-onum", "-pnum"]));
    let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    for run in &t.runs {
        assert_eq!(
            (run.style.fill.clone(), run.style.stroke.clone(), run.style.stroke_width),
            (state.doc.tint_paint("Brand", 1.0).unwrap(), tint40.clone(), 2.0)
        );
        assert_eq!(run.style.features, ["dlig", "tnum", "lnum", "-onum", "-pnum"]);
    }
    open(&mut app, "Body");
    let shown = fields::<Existing>(&app);
    assert_eq!((shown["stroke_tint"].clone(), shown["ot_figure"].clone()), (json!("40.0"), json!("tabularLining")));
    cancel::<Existing>(&mut app);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);

    app.run("select.set", json!({"ids": []})).unwrap();
    app.run("ui.newCharStyle", json!({})).unwrap();
    let shown = fields::<Fresh>(&app);
    assert!(shown["fill"].is_null() && shown["ot_sets"].is_null());
    set::<Fresh>(&app, "name", json!("Ink"));
    set::<Fresh>(&app, "tab", json!("opentype"));
    set::<Fresh>(&app, "ot_sets", json!(["-ss02", "ss04"]));
    set::<Fresh>(&app, "ot_frac", json!(false));
    assert!(can_reset_shown::<Fresh>(&app));
    assert!(ok::<Fresh>(&mut app));
    let doc = &app.session.doc().unwrap().doc;
    let def = doc.char_styles.iter().find(|def| def.name == "Ink").unwrap();
    assert_eq!(def.attrs["features"], json!(["-frac", "-ss02", "ss04"]));
    let vectorcraft_doc::NodeKind::Text(t) = &doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(t.runs.iter().all(|r| r.style.style_name.as_deref() == Some("Body")));
}

/// A size or leading below the minimum is refused by OK and runs no preview.
#[test]
fn size_and_leading_below_minimum_do_not_preview_or_commit() {
    for (key, value) in [("size", json!(0)), ("size", json!(-2)), ("leading", json!(0.09)), ("leading", json!(-1))] {
        let mut app = app();
        styled(&mut app, json!({"size": 24.0, "leading": 28.0}), "Text");
        let before = app.session.doc().unwrap().doc.clone();
        open(&mut app, "Body");
        set::<Existing>(&app, "name", json!("Revised"));
        set::<Existing>(&app, key, value);
        pump(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, before, "{key} preview");
        assert!(!ok::<Existing>(&mut app), "{key} confirm");
        assert!(is_open::<Existing>(&app), "{key} the dialog stays open");
        assert_eq!(app.session.doc().unwrap().doc, before, "{key} confirm");
    }
}

#[test]
fn new_character_style_refuses_size_and_leading_errors_at_ok() {
    for (key, value) in [("size", json!(0)), ("leading", json!(0.05))] {
        let mut app = app();
        app.run("ui.newCharStyle", json!({})).unwrap();
        set::<Fresh>(&app, "tab", json!("basic"));
        set::<Fresh>(&app, key, value);
        let styles = app.session.doc().unwrap().doc.char_styles.len();
        assert!(!ok::<Fresh>(&mut app), "{key} confirm");
        assert!(is_open::<Fresh>(&app), "{key} dialog stays open");
        assert_eq!(app.session.doc().unwrap().doc.char_styles.len(), styles, "{key} style count");
    }
}

#[test]
fn invalid_number_keeps_last_valid_preview() {
    let mut app = app();
    styled(&mut app, json!({"size": 24.0}), "Text");
    open(&mut app, "Body");
    set::<Existing>(&app, "name", json!("Revised"));
    set::<Existing>(&app, "size", json!(18));
    pump(&mut app);
    let valid = app.session.doc().unwrap().doc.clone();
    assert_eq!(valid.char_styles[0].name, "Revised");
    assert_eq!(valid.char_styles[0].attrs["size"], json!(18.0));
    set::<Existing>(&app, "size", json!("invalid"));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, valid);
}

#[test]
fn script_positions_use_document_setup_in_existing_and_new_style_dialogs() {
    let mut app = app();
    app.run("document.setup", json!({"superscript": {"size": 67.0, "position": 22.0}, "subscript": {"size": 72.0, "position": 18.0}})).unwrap();
    app.run("charStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    open(&mut app, "Body");
    set::<Existing>(&app, "position", json!("superscript"));
    assert!(ok::<Existing>(&mut app));
    assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["position"], json!({"kind": "superscript", "size": 67.0, "position": 22.0}));
    app.run("ui.newCharStyle", json!({})).unwrap();
    set::<Fresh>(&app, "position", json!("subscript"));
    assert!(ok::<Fresh>(&mut app));
    assert_eq!(
        app.session.doc().unwrap().doc.char_styles.last().unwrap().attrs["position"],
        json!({"kind": "subscript", "size": 72.0, "position": 18.0})
    );
}
