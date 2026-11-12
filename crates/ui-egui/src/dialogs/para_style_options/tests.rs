use super::*;
use vectorcraft_engine::Session;
use vectorcraft_engine::doc::{Composer, Justify, NodeId, NodeKind, ParaStyle};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app
}

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
    app.ui.dialog.as_mut().expect("a dialog is open").fields.insert(key.into(), value);
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
    app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.kind, KIND);
    assert_eq!(d.str("justify"), "Left");
    assert_eq!(d.str("left_indent"), "0 pt", "an explicit 0 is shown");
    assert_eq!(d.str("right_indent"), "", "an indent the style leaves out is empty");
    assert!(d.fields["composer"].is_null() && d.fields["roman_hanging"].is_null() && d.fields["hyphenate"].is_null());
    assert_eq!((d.str("words_longer_than"), d.str("hyphenation_zone")), ("5".into(), "0 pt".into()));
    assert_eq!(d.fields["hyphenate_capitalized"], json!(true));
    assert_eq!((d.str("word_min"), d.str("auto_leading")), (String::new(), String::new()));
    assert!(d.fields["single_word"].is_null());
    assert_eq!(edited_attrs(d).unwrap(), stored);
    super::super::confirm(&mut app).unwrap();
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
    app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
    set(&mut app, "justify", json!("JustifyAll"));
    set(&mut app, "space_before", json!("1 in"));
    let a = edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap();
    let mut want = def(&app, "Body");
    want.insert("justify".into(), json!("JustifyAll"));
    want.insert("space_before".into(), json!(72.0));
    assert_eq!(a, want, "Basic changes only its own attributes");

    set(&mut app, "composer", json!("singleLine"));
    set(&mut app, "roman_hanging", json!(false));
    set(&mut app, "hyphenate", json!(true));
    set(&mut app, "after_first", json!("4"));
    set(&mut app, "hyphenation_zone", json!("18"));
    set(&mut app, "word_desired", json!("90"));
    set(&mut app, "single_word", json!("alignCenter"));
    let a = edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap();
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
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(def(&app, "Body"), a);

    // An empty field leaves its attribute out; Justification fields all emptied remove it.
    app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
    set(&mut app, "space_before", json!(""));
    for key in JUSTIFICATION_FIELDS {
        set(&mut app, key, if key == "single_word" { Value::Null } else { json!("") });
    }
    let a = edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap();
    assert!(!a.contains_key("space_before") && !a.contains_key("justification"), "{a:?}");
}

/// A value out of range, one that is not a number and a spacing row out of order are errors that
/// keep the dialog open and the style as it was.
#[test]
fn invalid_values_keep_the_dialog_open() {
    let mut app = app();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    for (key, value, why) in [
        ("left_indent", "2000", "Left Indent must be a number from -1296 pt to 1296 pt"),
        ("first_line_indent", "wide", "First-line Left Indent must be a number"),
        ("words_longer_than", "26", "Words Longer Than must be a number from 1 to 25"),
        ("after_first", "1.5", "After First must be a whole number from 1 to 15"),
        ("hyphen_limit", "-1", "Hyphen Limit must be a number from 0 to 25"),
        ("hyphenation_zone", "-1", "Hyphenation Zone must be a number from 0 pt to 1296 pt"),
        ("letter_max", "501", "Letter Spacing must be a number from -100 to 500"),
        ("auto_leading", "x", "Auto Leading must be a number"),
        ("word_min", "120", "Minimum must not exceed Desired, nor Desired Maximum."),
    ] {
        app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
        set(&mut app, key, json!(value));
        assert_eq!(edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap_err(), why, "{key} = {value}");
        assert!(super::super::confirm(&mut app).is_err(), "{key} = {value}");
        assert!(app.ui.dialog.is_some(), "{key} = {value}");
        super::super::cancel(&mut app);
        assert_eq!(def(&app, "Body"), Map::new());
    }
    // A spacing row out of order in a style read from a file does not block a change to another
    // option; the definition stores the row as the paragraph applies it.
    let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
    let held = json!({"word_spacing": [120.0, 100.0, 133.0], "auto_leading": 120.0});
    doc.para_styles.iter_mut().find(|d| d.name == "Body").unwrap().attrs.insert("justification".into(), held);
    app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
    set(&mut app, "auto_leading", json!("150"));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(def(&app, "Body")["justification"]["auto_leading"], json!(150.0));
}

/// Reset Panel clears only the section shown, and General has none.
#[test]
fn reset_panel_clears_only_its_section() {
    let mut app = app();
    let all = json!({"justify": "Center", "space_after": 6.0, "composer": "singleLine", "roman_hanging": true, "hyphenate": true,
        "hyphenation": {"words_longer_than": 8}, "justification": {"auto_leading": 150.0}, "tabs": [{"position": 20.0}]});
    app.run("paraStyle.new", json!({"name": "Body", "attrs": all})).unwrap();
    let stored = def(&app, "Body");
    for (section, cleared) in [
        ("basic", &["justify", "space_after"][..]),
        ("composition", &["composer", "roman_hanging"]),
        ("hyphenation", &["hyphenate", "hyphenation"]),
        ("justification", &["justification"]),
    ] {
        app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
        assert!(app.run("ui.paraStyleOptions.resetPanel", json!({})).is_err(), "General has no Reset Panel");
        set(&mut app, "section", json!(section));
        app.run("ui.paraStyleOptions.resetPanel", json!({})).unwrap();
        let mut want = stored.clone();
        for key in cleared {
            want.remove(*key);
        }
        assert_eq!(edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap(), want, "{section}");
        super::super::cancel(&mut app);
    }
    assert!(app.run("ui.paraStyleOptions.resetPanel", json!({})).is_err(), "no dialog open");
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
    app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().bool("preview"));
    set(&mut app, "left_indent", json!("12"));
    set(&mut app, "composer", json!("singleLine"));
    frame(&mut app);
    assert_eq!(paras(&app, id)[0].left_indent, 12.0, "Preview shows the change");
    super::super::cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

    app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
    set(&mut app, "name", json!("Text"));
    set(&mut app, "left_indent", json!("12"));
    set(&mut app, "composer", json!("singleLine"));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
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

/// New Paragraph Style opens with the sections, filled in from the selected paragraph; a changed
/// field goes into the new style, and the others keep the captured values. The style is not
/// applied to the text.
#[test]
fn new_paragraph_style_offers_the_sections() {
    let mut app = app();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "One"})).unwrap()["id"].as_u64().unwrap();
    app.run("text.setStyle", json!({"id": id, "justify": "center"})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("ui.newParaStyle", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("section"), d.str("justify")), ("general".into(), "Center".into()));
    let captured = d.fields["__attrs"].as_object().cloned().unwrap();
    let mut texts = String::new();
    for section in SECTIONS {
        set(&mut app, "section", json!(section));
        texts += &crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
    }
    for label in ["Basic Paragraph Formats", "Composition", "First-line Left Indent", "Hyphenate Capitalized Words", "Single Word Justification:"] {
        assert!(texts.contains(label), "{label} is drawn: {texts}");
    }
    set(&mut app, "section", json!("basic"));
    app.run("ui.paraStyleOptions.resetPanel", json!({})).unwrap();
    set(&mut app, "space_after", json!("3"));
    set(&mut app, "name", json!("Lead"));
    super::super::confirm(&mut app).unwrap();
    let mut want = captured;
    for key in ["justify", "left_indent", "right_indent", "first_line_indent", "space_before"] {
        want.remove(key);
    }
    want.insert("space_after".into(), json!(3.0));
    assert_eq!(def(&app, "Lead"), want);
    assert_eq!(paras(&app, id)[0].style_name, None, "making the style does not apply it");
    assert_eq!(paras(&app, id)[0].justify, Justify::Center);
}

/// The section list, Reset Panel and both buttons are drawn in every section, and the Hyphenation
/// options are drawn whether or not Hyphenation is on.
#[test]
fn every_section_draws_its_fields() {
    let mut app = app();
    app.run("paraStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
    app.run("ui.paraStyleOptions", json!({"name": "Body"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for (section, label) in [
        ("general", "Style Name:"),
        ("basic", "Space After Paragraph"),
        ("composition", "Roman Hanging Punctuation"),
        ("hyphenation", "Hyphenation Zone:"),
        ("justification", "Glyph Scaling:"),
    ] {
        set(&mut app, "section", json!(section));
        let mut texts = String::new();
        for _ in 0..5 {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 900.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| super::super::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            texts = crate::tests_labels::shapes_text(&out);
        }
        for shown in [label, "General", "Basic Paragraph Formats", "Justification", "Reset Panel", "OK", "Cancel"] {
            assert!(texts.contains(shown), "{shown} is drawn in {section}: {texts}");
        }
    }
}
