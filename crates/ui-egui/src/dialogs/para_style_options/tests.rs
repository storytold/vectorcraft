use super::*;
use crate::tests::panel_harness::Panel;
use crate::tests::picks::dialogs;
use craft::ui::dialog::{DialogHost, DialogId, StackDialog};
use vectorcraft_engine::Session;
use vectorcraft_engine::doc::{Composer, NodeId, NodeKind, ParaStyle};

/// The rows Hyphenation gates.
const HYPHENATION_COUNTS: [&str; 4] = ["words_longer_than", "after_first", "hyphen_limit", "hyphenation_zone"];

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app
}

fn open(app: &mut VectorcraftApp, name: &str) {
    app.run("ui.paraStyleOptions", json!({"name": name})).unwrap();
}

/// Runs `f` on the host with the id of the open dialog; never call a session method inside `f`.
fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost, DialogId) -> R) -> R {
    let dialogs = app.dialogs().expect("the window has a dialog host");
    dialogs.with(|host| {
        let id = host.get::<ParaStyleOptions>().expect("the dialog is open").id();
        f(host, id)
    })
}

fn is_open(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.get::<ParaStyleOptions>().is_some()))
}

/// The open dialog's values as automation lists them.
fn fields(app: &VectorcraftApp) -> Value {
    with_dialog(app, |host, id| host.get_by_id::<ParaStyleOptions>(id).and_then(|d| d.json_fields())).unwrap_or(Value::Null)
}

/// Sets a value as the user editing that control would.
fn set(app: &VectorcraftApp, key: &str, value: Value) {
    let ctx = app.ui_ctx.clone().expect("a window");
    with_dialog(app, |host, id| host.set_json_field(&ctx, id, key, value)).unwrap_or_else(|why| panic!("{key}: {why}"));
}

/// Opens the dialog on style `name` with Tabs shown, where its Add, Delete Tab and Clear All Tabs
/// buttons are.
fn open_tabs(app: &mut VectorcraftApp, name: &str) {
    open(app, name);
    set(app, "tab", json!("tabs"));
}

/// One pass of the per-frame dialog pump: previews run, results are applied.
fn pump(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    crate::dialogs::host::pump(app, &ctx);
}

/// Presses OK; true when the dialog closed with its answer (a refused OK leaves it open).
fn ok(app: &mut VectorcraftApp) -> bool {
    // A second round answers an OK the dialog held for the app.
    for _ in 0..2 {
        if !is_open(app) {
            break;
        }
        let ctx = app.ui_ctx.clone().expect("a window");
        with_dialog(app, |host, id| host.confirm_by_id(&ctx, id));
        pump(app);
    }
    !is_open(app)
}

fn cancel(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    with_dialog(app, |host, id| host.cancel_by_id(&ctx, id));
    pump(app);
}

/// Presses Reset Panel on the shown tab.
fn reset_shown(app: &VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    assert!(with_dialog(app, |host, id| host.reset_by_id(&ctx, id)), "the shown tab has a Reset Panel");
}

fn can_reset_shown(app: &VectorcraftApp) -> bool {
    with_dialog(app, |host, id| host.can_reset_by_id(id))
}

fn def(app: &VectorcraftApp, name: &str) -> Map<String, Value> {
    app.session.doc().unwrap().doc.para_styles.iter().find(|d| d.name == name).map(|d| d.attrs.clone()).unwrap_or_default()
}

fn paras(app: &VectorcraftApp, id: u64) -> Vec<ParaStyle> {
    match &app.session.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.paragraph_styles(),
        _ => panic!("not text"),
    }
}

/// A style read from a file or captured from text, with attributes no section shows.
const KEPT: &str = r#"{"justify": "Left", "left_indent": 0.0, "space_before": 7.123456, "tabs": [{"position": 36.0, "align": "right"}], "direction": "rightToLeft",
    "mojikumi": "lineEndHalf", "list": {"bullet": "dash"}, "hyphenation": {"words_longer_than": 5, "after_first": 3, "before_last": 3,
    "hyphen_limit": 0, "zone": 0.0, "capitalized": true}}"#;

/// Paragraph Style Options opens on a style's attributes: an attribute the style sets, a default
/// value included, fills its field, one it leaves out is empty or unset. OK with nothing changed
/// keeps every attribute, those no section shows and a distance finer than its field shows
/// included, and records no undo step.
#[test]
fn unchanged_fields_keep_every_attribute_and_set_no_undo_step() {
    let mut app = app();
    let kept: Value = serde_json::from_str(KEPT).unwrap();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": kept})).unwrap();
    let stored = def(&app, "Body");
    let undo = app.session.doc().unwrap().history.undo.len();
    open(&mut app, "Body");
    let d = fields(&app);
    assert_eq!(d["justify"], json!("Left"));
    assert_eq!(d["left_indent"], json!("0 pt"), "an explicit 0 is shown");
    assert_eq!(d["right_indent"], json!(""), "an indent the style leaves out is empty");
    assert!(d["composer"].is_null() && d["roman_hanging"].is_null() && d["hyphenate"].is_null());
    assert_eq!((d["words_longer_than"].clone(), d["hyphenation_zone"].clone()), (json!("5"), json!("0 pt")));
    assert_eq!(d["hyphenate_capitalized"], json!(true));
    assert_eq!((d["word_min"].clone(), d["auto_leading"].clone()), (json!(""), json!("")));
    assert!(d["single_word"].is_null());
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Body"), stored);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    assert!(app.run("ui.paraStyleOptions", json!({"name": "Missing"})).is_err());
    assert!(app.run("ui.paraStyleOptions", json!({})).is_err());
}

/// Each section's fields set their attributes: Alignment and a distance typed with a unit in Basic,
/// the composer and Roman Hanging Punctuation in Composition, Hyphenation and one option in
/// Hyphenation (the other options at their defaults), and a spacing row and Single Word
/// Justification in Justification. Changing Basic leaves the other attributes as they were.
#[test]
fn each_section_sets_its_attributes_and_keeps_the_others() {
    let mut app = app();
    let kept: Value = serde_json::from_str(KEPT).unwrap();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": kept})).unwrap();
    let start = def(&app, "Body");
    open(&mut app, "Body");
    set(&app, "justify", json!("JustifyAll"));
    set(&app, "space_before", json!("1 in"));
    assert!(ok(&mut app));
    let a = def(&app, "Body");
    let mut want = start.clone();
    want.insert("justify".into(), json!("JustifyAll"));
    want.insert("space_before".into(), json!(72.0));
    assert_eq!(a, want, "Basic changes only its own attributes");

    open(&mut app, "Body");
    set(&app, "composer", json!("singleLine"));
    set(&app, "roman_hanging", json!(false));
    set(&app, "hyphenate", json!(true));
    set(&app, "after_first", json!("4"));
    set(&app, "hyphenation_zone", json!("18"));
    set(&app, "word_desired", json!("90"));
    set(&app, "single_word", json!("alignCenter"));
    assert!(ok(&mut app));
    let a = def(&app, "Body");
    assert_eq!((&a["composer"], &a["roman_hanging"], &a["hyphenate"]), (&json!("singleLine"), &json!(false), &json!(true)));
    assert_eq!(
        a["hyphenation"],
        json!({"words_longer_than": 5, "after_first": 4, "before_last": 3, "hyphen_limit": 0, "zone": 18.0, "capitalized": true})
    );
    let j = &a["justification"];
    assert_eq!((&j["word_spacing"], &j["auto_leading"], &j["single_word"]), (&json!([80.0, 90.0, 133.0]), &json!(120.0), &json!("alignCenter")));
    for key in ["tabs", "direction", "mojikumi", "list", "left_indent"] {
        assert_eq!(a.get(key), want.get(key), "{key} stays as it was");
    }

    // An empty field leaves its attribute out; Justification fields all emptied remove it.
    open(&mut app, "Body");
    set(&app, "space_before", json!(""));
    for key in JUSTIFICATION_FIELDS {
        // Single Word Justification's unset entry is the empty choice.
        set(&app, key, json!(""));
    }
    assert!(ok(&mut app));
    let a = def(&app, "Body");
    assert!(!a.contains_key("space_before") && !a.contains_key("justification"), "{a:?}");
}

/// A value out of range, one that is not a number and a spacing row out of order are errors that
/// keep the dialog open and the style as it was.
#[test]
fn invalid_values_keep_the_dialog_open() {
    let mut app = app();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    for (key, value) in [
        ("left_indent", "2000"),
        ("first_line_indent", "wide"),
        ("words_longer_than", "26"),
        ("after_first", "1.5"),
        ("hyphen_limit", "-1"),
        ("hyphenation_zone", "-1"),
        ("letter_max", "501"),
        ("auto_leading", "x"),
        ("word_min", "120"),
    ] {
        open(&mut app, "Body");
        // The hyphenation rows take input while Hyphenation is on.
        if HYPHENATION_COUNTS.contains(&key) {
            set(&app, "hyphenate", json!(true));
        }
        set(&app, key, json!(value));
        assert!(!ok(&mut app), "{key} = {value}");
        assert!(is_open(&app), "{key} = {value}");
        cancel(&mut app);
        assert_eq!(def(&app, "Body"), Map::new());
    }
    // A spacing row out of order in a style read from a file does not block a change to another
    // option; the definition stores the row as the paragraph applies it.
    let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
    let held = json!({"word_spacing": [120.0, 100.0, 133.0], "auto_leading": 120.0});
    doc.para_styles.iter_mut().find(|d| d.name == "Body").unwrap().attrs.insert("justification".into(), held);
    open(&mut app, "Body");
    set(&app, "auto_leading", json!("150"));
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Body")["justification"]["auto_leading"], json!(150.0));
}

/// Reset Panel clears only the section shown, and General has none.
#[test]
fn reset_panel_clears_only_its_section() {
    let mut app = app();
    let all = json!({"justify": "Center", "space_after": 6.0, "composer": "singleLine", "roman_hanging": true, "hyphenate": true,
        "hyphenation": {"words_longer_than": 8}, "justification": {"auto_leading": 150.0}, "tabs": [{"position": 20.0}],
        "korean_line_break_anywhere": true, "bunri_kinshi": true});
    for (at, (section, cleared)) in [
        ("basic", &["justify", "space_after"][..]),
        ("tabs", &["tabs"]),
        ("composition", &["composer", "roman_hanging"]),
        ("hyphenation", &["hyphenate", "hyphenation"]),
        ("justification", &["justification"]),
        ("eastAsian", &["korean_line_break_anywhere", "bunri_kinshi"]),
    ]
    .into_iter()
    .enumerate()
    {
        // OK applies the reset, so each section starts from a style of its own.
        let name = format!("Body{at}");
        app.run("paraStyle.new", json!({"name": name, "attrs": all})).unwrap();
        let stored = def(&app, &name);
        open(&mut app, &name);
        assert!(!can_reset_shown(&app), "General has no Reset Panel");
        set(&app, "tab", json!(section));
        reset_shown(&app);
        assert!(ok(&mut app));
        let mut want = stored.clone();
        for key in cleared {
            want.remove(*key);
        }
        assert_eq!(def(&app, &name), want, "{section}");
    }
}

/// Preview shows a change on the type that uses the style, and Cancel puts the document and its
/// history back. OK renames the style and changes its paragraphs in one undo step, a paragraph's
/// local override stays, and Undo and Redo restore both.
#[test]
fn preview_cancel_and_ok_update_the_paragraphs_in_one_undo_step() {
    let mut app = app();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "One\nTwo"})).unwrap()["id"].as_u64().unwrap();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {"left_indent": 6.0}})).unwrap();
    app.run("paraStyle.apply", json!({"name": "Body", "id": id})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 4, "leftIndent": 20.0})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    open(&mut app, "Body");
    assert_eq!(fields(&app)["preview"], json!(true));
    set(&app, "left_indent", json!("12"));
    set(&app, "composer", json!("singleLine"));
    pump(&mut app);
    assert_eq!(paras(&app, id)[0].left_indent, 12.0, "Preview shows the change");
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

    open(&mut app, "Body");
    set(&app, "name", json!("Text"));
    set(&app, "left_indent", json!("12"));
    set(&app, "composer", json!("singleLine"));
    pump(&mut app);
    assert!(ok(&mut app));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    let p = paras(&app, id);
    assert_eq!(
        p.iter().map(|p| (p.style_name.as_deref(), p.left_indent, p.composer)).collect::<Vec<_>>(),
        [(Some("Text"), 12.0, Composer::SingleLine), (Some("Text"), 20.0, Composer::SingleLine)]
    );
    let changed = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, changed);
}

/// A paragraph style Body with the left indent `left_indent` and a selected text object that uses it.
fn styled(app: &mut VectorcraftApp, left_indent: f64, text: &str) {
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": text})).unwrap()["id"].as_u64().unwrap();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {"left_indent": left_indent}})).unwrap();
    app.run("paraStyle.apply", json!({"name": "Body", "id": id})).unwrap();
}

/// Two documents that each have a paragraph style Body with their own left indent, with Paragraph
/// Style Options open on Body in the first.
fn options_in_the_first_of_two_documents() -> VectorcraftApp {
    let mut app = app();
    styled(&mut app, 6.0, "Opening document");
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    styled(&mut app, 3.0, "Other document");
    app.run("document.activate", json!({"index": 0})).unwrap();
    open(&mut app, "Body");
    app
}

/// Preview changes only the document the dialog opened on: while another document is active it
/// leaves that document's Body as it is, it resumes on switching back, and Cancel restores the
/// opening document.
#[test]
fn the_style_preview_stays_in_its_opening_document() {
    let mut app = options_in_the_first_of_two_documents();
    let opening = app.session.doc().unwrap().doc.clone();
    set(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    app.run("document.activate", json!({"index": 1})).unwrap();
    let other = app.session.doc().unwrap().doc.clone();
    set(&app, "left_indent", json!("12"));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, other, "Preview leaves the other document's Body style unchanged");
    assert!(!app.session.in_interaction());

    app.run("document.activate", json!({"index": 0})).unwrap();
    pump(&mut app);
    assert!(app.session.doc().unwrap().doc.para_styles.iter().any(|style| style.name == "Revised"));
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, opening);
}

/// OK with another document active is refused and keeps the dialog open; OK back in the opening
/// document saves the edit there.
#[test]
fn a_style_options_confirmation_stays_in_its_opening_document() {
    let mut app = options_in_the_first_of_two_documents();
    set(&app, "preview", json!(false));
    set(&app, "name", json!("Revised"));
    app.run("document.activate", json!({"index": 1})).unwrap();
    let other = app.session.doc().unwrap().doc.clone();
    let closed = ok(&mut app);

    assert_eq!(app.session.doc().unwrap().doc, other, "OK leaves the other document's Body style unchanged");
    assert!(!closed, "the same draft remains available");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(ok(&mut app));
    assert!(app.session.doc().unwrap().doc.para_styles.iter().any(|style| style.name == "Revised"));
}

/// OK after the opening document has closed is refused and leaves the remaining document as it is.
#[test]
fn a_style_options_confirmation_is_refused_after_its_document_closes() {
    let mut app = options_in_the_first_of_two_documents();
    set(&app, "preview", json!(false));
    set(&app, "name", json!("Revised"));
    assert!(app.session.close_document(0));
    let other = app.session.doc().unwrap().doc.clone();
    assert!(!ok(&mut app), "the dialog stays open with the refusal");
    assert_eq!(app.session.doc().unwrap().doc, other);
}

/// Cancel with another document active leaves that document's own gesture running.
#[test]
fn cancelling_style_options_leaves_the_other_documents_gesture() {
    let mut app = options_in_the_first_of_two_documents();
    set(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.session.begin_interaction("Another gesture").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    let gesture = app.session.doc().unwrap().doc.clone();

    cancel(&mut app);

    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

/// A gesture started in the opening document after switching away and back is not the dialog's:
/// Preview pauses, OK is refused, and Cancel leaves the gesture running.
#[test]
fn switching_back_leaves_a_new_gesture_in_the_opening_document() {
    let mut app = options_in_the_first_of_two_documents();
    set(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.session.begin_interaction("Another gesture").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    let gesture = app.session.doc().unwrap().doc.clone();

    set(&app, "left_indent", json!("12"));
    pump(&mut app);
    assert!(!ok(&mut app));
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

/// Paragraph Style Options open with a preview that a newer gesture of the same name replaced in
/// the same document.
fn options_with_a_newer_gesture() -> VectorcraftApp {
    let mut app = app();
    styled(&mut app, 6.0, "Opening document");
    open(&mut app, "Body");
    set(&app, "name", json!("Revised"));
    pump(&mut app);
    assert!(app.session.in_interaction());
    let resets = app.session.tool_resets();
    app.session.cancel_interaction().unwrap();
    app.session.begin_interaction("Paragraph Style Options").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    assert_eq!(app.session.tool_resets(), resets);
    app
}

/// Cancel leaves a newer gesture in the same document running.
#[test]
fn cancelling_style_options_preserves_a_newer_gesture_in_the_same_document() {
    let mut app = options_with_a_newer_gesture();
    let gesture = app.session.doc().unwrap().doc.clone();
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

/// Preview does not replace a newer gesture in the same document.
#[test]
fn style_options_preview_preserves_a_newer_gesture_in_the_same_document() {
    let mut app = options_with_a_newer_gesture();
    let gesture = app.session.doc().unwrap().doc.clone();
    set(&app, "left_indent", json!("12"));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

/// OK is refused while a newer gesture is active in the same document, and records no undo step.
#[test]
fn style_options_confirmation_preserves_a_newer_gesture_in_the_same_document() {
    let mut app = options_with_a_newer_gesture();
    let gesture = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    assert!(!ok(&mut app));
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
}

/// The first preview does not replace a gesture that was already active when the dialog opened.
#[test]
fn a_first_style_preview_preserves_an_existing_gesture() {
    let mut app = app();
    styled(&mut app, 6.0, "Opening document");
    open(&mut app, "Body");
    app.session.begin_interaction("Paragraph Style Options").unwrap();
    app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
    let gesture = app.session.doc().unwrap().doc.clone();
    set(&app, "left_indent", json!("12"));
    pump(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, gesture);
    assert!(app.session.in_interaction());
}

/// East Asian Composition Settings › Korean Line Break Anywhere sets the style's attribute, which
/// its paragraphs take; unset, the style leaves it out and the paragraphs keep their own.
#[test]
fn korean_line_break_anywhere_is_set_by_the_east_asian_section() {
    let mut app = app();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "가나 다라"})).unwrap()["id"].as_u64().unwrap();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    app.run("paraStyle.apply", json!({"name": "Body", "id": id})).unwrap();
    assert!(!paras(&app, id)[0].korean_line_break_anywhere);
    open(&mut app, "Body");
    assert!(fields(&app)["korean_line_break_anywhere"].is_null());
    set(&app, "tab", json!("eastAsian"));
    set(&app, "korean_line_break_anywhere", json!(true));
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Body").get("korean_line_break_anywhere"), Some(&json!(true)));
    assert!(paras(&app, id)[0].korean_line_break_anywhere, "the style's paragraphs take it");
}

/// East Asian Composition Settings › Bunri-Kinshi sets the style's attribute, which its paragraphs
/// take; set off again, the style holds it off, and unset, the style leaves it out.
#[test]
fn bunri_kinshi_is_set_by_the_east_asian_section() {
    let mut app = app();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "あ——い"})).unwrap()["id"].as_u64().unwrap();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    app.run("paraStyle.apply", json!({"name": "Body", "id": id})).unwrap();
    assert!(!paras(&app, id)[0].bunri_kinshi);
    open(&mut app, "Body");
    assert!(fields(&app)["bunri_kinshi"].is_null());
    set(&app, "tab", json!("eastAsian"));
    set(&app, "bunri_kinshi", json!(true));
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Body").get("bunri_kinshi"), Some(&json!(true)));
    assert!(paras(&app, id)[0].bunri_kinshi, "the style's paragraphs take it");
    open(&mut app, "Body");
    assert_eq!(fields(&app)["bunri_kinshi"], json!(true));
    set(&app, "bunri_kinshi", json!(false));
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Body").get("bunri_kinshi"), Some(&json!(false)));
    assert!(!paras(&app, id)[0].bunri_kinshi);
    open(&mut app, "Body");
    set(&app, "bunri_kinshi", Value::Null);
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Body").get("bunri_kinshi"), None);
}

/// The rows of the `tabs` field of the open dialog.
fn tab_rows(app: &VectorcraftApp) -> Vec<Value> {
    fields(app)["tabs"].as_array().cloned().unwrap_or_default()
}

#[test]
fn clear_all_tabs_can_be_clicked_in_a_narrow_viewport() {
    use egui::accesskit::Role;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

    for width in [800.0, 540.0] {
        let mut app = app();
        app.run("paraStyle.new", json!({"name": "Body", "attrs": {"tabs": [{"position": 36.0}, {"position": 72.0}]}})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 500.0));
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, events| {
            time += 0.25;
            crate::testing::app_frame(app, &ctx, RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() })
        };
        frame(&mut app, vec![]);
        open_tabs(&mut app, "Body");
        for _ in 0..5 {
            frame(&mut app, vec![]);
        }
        let out = frame(&mut app, vec![]);
        let nodes = out.platform_output.accesskit_update.unwrap().nodes;
        let clear = nodes.iter().find(|(_, node)| node.role() == Role::Button && node.label() == Some("Clear All Tabs")).unwrap();
        let bounds = clear.1.bounds().unwrap();
        let at = pos2(((bounds.x0 + bounds.x1) * 0.5) as f32, ((bounds.y0 + bounds.y1) * 0.5) as f32);

        assert_eq!(tab_rows(&app).len(), 2);
        frame(&mut app, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            frame(&mut app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
        }
        frame(&mut app, vec![]);
        assert!(tab_rows(&app).is_empty(), "Clear All Tabs at {bounds:?} was not reachable in a {width}-point viewport");
    }
}

/// Tabs shows a style's stops in the General unit with their alignment, leader and Align On
/// character; a stop left as it was keeps its position exactly, a changed one is read in the unit,
/// and the stops are stored sorted by position. Add puts a left stop half an inch past the furthest
/// stop, Delete Tab removes the selected stop and Clear All Tabs leaves the style an empty list.
#[test]
fn tabs_section_edits_the_style_stops() {
    let mut app = app();
    let stops = json!([{"position": 36.0, "align": "right"}, {"position": 72.123456, "align": "decimal", "leader": ".", "alignOn": ","}]);
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {"left_indent": 3.0, "tabs": stops}})).unwrap();
    let stored = def(&app, "Body");
    open(&mut app, "Body");
    let rows = tab_rows(&app);
    let shown: Vec<(&str, &str, &str, &str)> = rows
        .iter()
        .map(|r| (r["position"].as_str().unwrap(), r["align"].as_str().unwrap(), r["leader"].as_str().unwrap(), r["alignOn"].as_str().unwrap()))
        .collect();
    assert_eq!(shown, [("36 pt", "right", "", "."), ("72.123 pt", "decimal", ".", ",")]);
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Body"), stored, "nothing changed");

    // The first stop moves past the second and turns centered; the second keeps its exact position.
    open(&mut app, "Body");
    let mut rows = tab_rows(&app);
    rows[0]["position"] = json!("1.5 in");
    rows[0]["align"] = json!("center");
    set(&app, "tabs", Value::from(rows));
    assert!(ok(&mut app));
    let a = def(&app, "Body");
    assert_eq!(
        a["tabs"],
        json!([{"position": 72.123456, "align": "decimal", "leader": ".", "alignOn": ","}, {"position": 108.0, "align": "center", "alignOn": "."}])
    );
    assert_eq!(a["left_indent"], json!(3.0), "other attributes stay");

    let mut p = Panel::new(app, dialogs);
    open_tabs(&mut p.app(), "Body");
    p.click("Add");
    {
        let app = p.app();
        // The stops list is a choice row: it holds the stop's index as its id.
        assert_eq!((tab_rows(&app).len(), fields(&app)["__tabSel"].clone()), (3, json!("2")));
        set(&app, "__tabSel", json!("0"));
    }
    p.click("Delete Tab");
    assert!(ok(&mut p.app()));
    let a = def(&p.app(), "Body");
    assert_eq!(a["tabs"].as_array().unwrap().iter().map(|s| s["position"].clone()).collect::<Vec<_>>(), [json!(108.0), json!(144.0)]);
    assert_eq!(a["tabs"][1], json!({"position": 144.0, "align": "left", "alignOn": "."}), "half an inch past the furthest");

    // Clear All Tabs leaves an empty list set, which Reset Panel removes; a new stop on a style
    // without stops goes at half an inch.
    open_tabs(&mut p.app(), "Body");
    p.click("Clear All Tabs");
    assert!(ok(&mut p.app()));
    assert_eq!(def(&p.app(), "Body")["tabs"], json!([]), "an empty list is stored");
    open_tabs(&mut p.app(), "Body");
    reset_shown(&p.app());
    p.click("Add");
    assert!(ok(&mut p.app()));
    let a = def(&p.app(), "Body");
    assert_eq!(a["tabs"], json!([{"position": 36.0, "align": "left", "alignOn": "."}]));
    assert_eq!(a["left_indent"], json!(3.0), "{a:?}");
}

/// Of stops typed at one position, the one edited last is kept; with none edited, the first in the
/// list is.
#[test]
fn a_repeated_tab_position_keeps_the_stop_edited_last() {
    let mut app = app();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    let rows = json!([{"position": "36", "align": "right"}, {"position": "0.5 in", "align": "center"}, {"position": "10", "align": "left"}]);
    open(&mut app, "Body");
    set(&app, "tabs", rows.clone());
    assert!(ok(&mut app));
    assert_eq!(
        def(&app, "Body")["tabs"],
        json!([{"position": 10.0, "align": "left", "alignOn": "."}, {"position": 36.0, "align": "right", "alignOn": "."}])
    );
    app.run("paraStyle.new", json!({"name": "Edited", "attrs": {}})).unwrap();
    open(&mut app, "Edited");
    set(&app, "tabs", rows);
    set(&app, "__tabEdited", json!(1));
    assert!(ok(&mut app));
    assert_eq!(def(&app, "Edited")["tabs"][1]["align"], json!("center"));
}

/// Add does not put a stop where one already is: with a stop at 100000 pt, the largest position,
/// it adds nothing and the Add button is disabled, so the existing stop keeps its settings.
#[test]
fn add_leaves_a_stop_at_the_largest_position_alone() {
    let mut app = app();
    let stops = json!([{"position": 100000.0, "align": "right", "leader": "-"}]);
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {"tabs": stops}})).unwrap();
    let stored = def(&app, "Body");
    let mut p = Panel::new(app, dialogs);
    open_tabs(&mut p.app(), "Body");
    let before = tab_rows(&p.app());
    // The dialog opened after the last frame: one shows it.
    p.frame();
    assert!(!p.enabled("Add"), "Add is disabled");
    p.click("Add");
    assert_eq!(tab_rows(&p.app()), before, "no stop is added");
    assert!(ok(&mut p.app()));
    assert_eq!(def(&p.app(), "Body"), stored, "the stop is kept");
}

/// A tab position out of range or not a distance, an unknown alignment, a leader of more than eight
/// characters, an Align On entry that is not one character, more than 256 stops, a `tabs` value that
/// is not a list and a stop that is not an object are errors that keep the dialog open and the style
/// as it was.
#[test]
fn invalid_tab_stops_keep_the_dialog_open() {
    let mut app = app();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    let stop = |key: &str, value: Value| {
        let mut stop = json!({"position": "36", "align": "decimal", "leader": "", "alignOn": "."});
        stop[key] = value;
        json!([stop])
    };
    let many = Value::from((0..257).map(|i| json!({"position": format!("{i}"), "align": "left"})).collect::<Vec<_>>());
    for tabs in [
        stop("position", json!("-1")),
        stop("position", json!("100001 pt")),
        stop("position", json!("far")),
        stop("align", json!("diagonal")),
        stop("leader", json!("123456789")),
        stop("alignOn", json!("")),
        stop("alignOn", json!(",.")),
        many,
        json!("36"),
        json!({"position": "36"}),
        json!([5]),
    ] {
        open(&mut app, "Body");
        set(&app, "tabs", tabs.clone());
        assert!(!ok(&mut app), "{tabs}");
        assert!(is_open(&app), "{tabs}");
        cancel(&mut app);
        assert_eq!(def(&app, "Body"), Map::new());
    }
    // Stops a style read from a file holds out of range stay while another stop is added.
    let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
    let held = json!([{"position": 200000.0, "align": "left", "leader": "123456789", "alignOn": "."}]);
    doc.para_styles.iter_mut().find(|d| d.name == "Body").unwrap().attrs.insert("tabs".into(), held.clone());
    let mut p = Panel::new(app, dialogs);
    open_tabs(&mut p.app(), "Body");
    p.click("Add");
    assert!(ok(&mut p.app()));
    assert_eq!(def(&p.app(), "Body")["tabs"][1], held[0]);
}

/// Preview shows a style's new stops on the paragraphs that use it, and Cancel puts the document
/// and its history back. OK sets them in one undo step, a paragraph whose stops the Tabs panel set
/// keeps its own, and Undo and Redo restore both. Saving and opening the document keeps the stops.
#[test]
fn tabs_preview_cancel_and_ok_update_the_paragraphs_in_one_undo_step() {
    let mut app = app();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "a\tb\nc\td"})).unwrap()["id"].as_u64().unwrap();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {"tabs": [{"position": 36.0}]}})).unwrap();
    app.run("paraStyle.apply", json!({"name": "Body", "id": id})).unwrap();
    app.run("text.tabs.set", json!({"ids": [id], "start": 4, "end": 4, "stops": [{"position": 50.0}]})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let new_stops = json!([{"position": "20", "align": "right", "leader": "."}, {"position": "80", "align": "decimal", "alignOn": ","}]);
    let positions = |app: &VectorcraftApp| paras(app, id).iter().map(|p| p.tabs.iter().map(|t| t.position).collect::<Vec<_>>()).collect::<Vec<_>>();
    open(&mut app, "Body");
    set(&app, "tab", json!("tabs"));
    set(&app, "tabs", new_stops.clone());
    pump(&mut app);
    assert_eq!(positions(&app), [vec![20.0, 80.0], vec![50.0]], "Preview shows the change");
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

    open(&mut app, "Body");
    set(&app, "tabs", new_stops);
    pump(&mut app);
    assert!(ok(&mut app));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(positions(&app), [vec![20.0, 80.0], vec![50.0]]);
    let p = paras(&app, id);
    assert_eq!((p[0].tabs[0].leader.as_str(), p[0].tabs[1].align_on), (".", ','));
    let changed = app.session.doc().unwrap().doc.clone();
    let saved = vectorcraft_format::load(&vectorcraft_format::save(&changed, false)).unwrap();
    let def_of = |d: &vectorcraft_doc::Document| d.para_styles.iter().find(|s| s.name == "Body").map(|s| s.attrs.clone());
    assert_eq!(def_of(&saved), def_of(&changed));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, changed);
}
