//! Paragraph panel menu › Hyphenation… (or `ui.hyphenation`): the Hyphenation checkbox and the
//! limits automatic hyphenation keeps to in the paragraphs the Paragraph panel acts on: Words
//! Longer Than, After First, Before Last, Hyphen Limit, Hyphenation Zone and Hyphenate Capitalized
//! Words. The dialog opens on the values the Paragraph panel shows (blank fields and dashed
//! checkboxes where paragraphs in scope disagree). OK runs `text.setFormat` with the values that
//! changed, as one undo step, and paragraphs that differ keep the values left alone.
//!
//! Fields: `hyphenate`, `wordsLongerThan` (letters, 1..25), `afterFirst` and `beforeLast`
//! (letters, 1..15), `hyphenLimit` (hyphens, 0..25, 0: no limit), `hyphenationZone` (pt) and
//! `hyphenateCapitalized`, as `text.setFormat` takes them, and `__initial` (the values it opened
//! with).

use serde_json::{Map, Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Hyphenation.
pub const KIND: &str = "hyphenation";

/// The field holding the values the dialog opened with.
const INITIAL: &str = "__initial";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Hyphenation").into(),
    body,
    confirm,
    min_width: 300.0,
    // A value the paragraphs don't share opens blank, without its field.
    inputs: |_| &["hyphenate", "wordsLongerThan", "afterFirst", "beforeLast", "hyphenLimit", "hyphenationZone", "hyphenateCapitalized"],
    ..DialogSpec::FORM
};

/// Open Hyphenation on the paragraph attributes the Paragraph panel shows.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let mut values = Map::new();
    if let Some(v) = crate::panels::paragraph::shared_hyphenate(app) {
        values.insert("hyphenate".into(), json!(v));
    }
    if let Some(v) = crate::panels::paragraph::shared_hyphenation_field(app, |h| h.words_longer_than) {
        values.insert("wordsLongerThan".into(), json!(v));
    }
    if let Some(v) = crate::panels::paragraph::shared_hyphenation_field(app, |h| h.after_first) {
        values.insert("afterFirst".into(), json!(v));
    }
    if let Some(v) = crate::panels::paragraph::shared_hyphenation_field(app, |h| h.before_last) {
        values.insert("beforeLast".into(), json!(v));
    }
    if let Some(v) = crate::panels::paragraph::shared_hyphenation_field(app, |h| h.hyphen_limit) {
        values.insert("hyphenLimit".into(), json!(v));
    }
    if let Some(v) = crate::panels::paragraph::shared_hyphenation_zone(app) {
        values.insert("hyphenationZone".into(), json!(v));
    }
    if let Some(v) = crate::panels::paragraph::shared_hyphenation_field(app, |h| h.capitalized) {
        values.insert("hyphenateCapitalized".into(), json!(v));
    }
    let mut fields = Value::Object(values.clone());
    fields[INITIAL] = Value::Object(values);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let hyphenate = d.fields.get("hyphenate").and_then(|v| v.as_bool());
    if widgets::check3(ui, tl!("Hyphenation"), hyphenate, true) {
        d.fields.insert("hyphenate".into(), json!(hyphenate != Some(true)));
    }
    ui.add_space(8.0);
    // The whole-number fields: (field, label, unit, range).
    let counts = [
        ("wordsLongerThan", tl!("Words Longer Than:"), tl!("letters"), (1.0, 25.0)),
        ("afterFirst", tl!("After First:"), tl!("letters"), (1.0, 15.0)),
        ("beforeLast", tl!("Before Last:"), tl!("letters"), (1.0, 15.0)),
        ("hyphenLimit", tl!("Hyphen Limit:"), tl!("hyphens"), (0.0, 25.0)),
    ];
    grid(ui, |ui| {
        for (key, text, suffix, (lo, hi)) in counts {
            label(ui, text);
            ui.horizontal(|ui| {
                let value = d.fields.get(key).and_then(|v| v.as_f64());
                if let Some(v) = widgets::range_field(ui, ("hyphenation", key), value, lo..=hi, "", 0, 60.0) {
                    d.fields.insert(key.into(), json!(v));
                }
                ui.label(suffix);
            });
            ui.end_row();
        }
        label(ui, tl!("Hyphenation Zone:"));
        form::length(ui, d, "hyphenationZone", unit, 90.0);
        ui.end_row();
    });
    ui.add_space(8.0);
    let capitalized = d.fields.get("hyphenateCapitalized").and_then(|v| v.as_bool());
    if widgets::check3(ui, tl!("Hyphenate Capitalized Words"), capitalized, true) {
        d.fields.insert("hyphenateCapitalized".into(), json!(capitalized != Some(true)));
    }
    false
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with.
fn params(d: &Dialog) -> Map<String, Value> {
    let initial = d.fields.get(INITIAL).and_then(Value::as_object);
    let mut p = Map::new();
    for key in ["hyphenate", "wordsLongerThan", "afterFirst", "beforeLast", "hyphenLimit", "hyphenationZone", "hyphenateCapitalized"] {
        let Some(v) = d.fields.get(key) else { continue };
        // Numbers are compared as numbers (a field stores 3 as 3.0), and a length typed with its
        // unit ("6 mm") is sent in points.
        let v = if v.is_boolean() { v.clone() } else { json!(d.f64(key, 0.0)) };
        let same = match (initial.and_then(|m| m.get(key)), &v) {
            (Some(Value::Bool(was)), Value::Bool(now)) => was == now,
            (Some(was), now) => was.as_f64().zip(now.as_f64()).is_some_and(|(a, b)| a == b),
            (None, _) => false,
        };
        if !same {
            p.insert(key.into(), v);
        }
    }
    p
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(d);
    app.ui.dialog = None;
    if p.is_empty() {
        return Ok(Value::Null);
    }
    crate::panels::paragraph::run_on_paragraphs(app, "text.setFormat", Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{HyphenationOptions, NodeId, NodeKind, ParaStyle};

    /// A selected area type object with two paragraphs.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 10, "text": "One\nTwo", "area": {"width": 200, "height": 100}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    fn paras(app: &VectorcraftApp, id: u64) -> (ParaStyle, ParaStyle) {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => (t.para_at(0).clone(), t.para_at(1).clone()),
            _ => panic!("not text"),
        }
    }

    /// `ui.hyphenation` opens the dialog on the selected type; OK sets the paragraphs' hyphenation
    /// options in one undo step.
    #[test]
    fn hyphenation_dialog_sets_the_paragraphs_options() {
        let (mut app, id) = app();
        app.run("ui.hyphenation", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Hyphenation opened");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.bool("hyphenate"), d.f64("wordsLongerThan", 0.0), d.f64("hyphenLimit", -1.0)), (false, 5.0, 0.0));
        assert!(d.bool("hyphenateCapitalized"));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("hyphenate".into(), json!(true));
        d.fields.insert("wordsLongerThan".into(), json!(8));
        d.fields.insert("hyphenLimit".into(), json!(2));
        d.fields.insert("hyphenationZone".into(), json!("1 in"));
        d.fields.insert("hyphenateCapitalized".into(), json!(false));
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let want = HyphenationOptions { words_longer_than: 8, hyphen_limit: 2, zone: 72.0, capitalized: false, ..HyphenationOptions::default() };
        for p in [paras(&app, id).0, paras(&app, id).1] {
            assert!(p.hyphenate);
            assert_eq!(p.hyphenation, want);
        }
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }

    /// OK sends only the values that changed: paragraphs whose other options differ keep them.
    #[test]
    fn hyphenation_dialog_leaves_unchanged_values_alone() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "afterFirst": 2, "start": 5, "end": 5})).unwrap();
        app.run("ui.hyphenation", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("beforeLast".into(), json!(4));
        super::super::confirm(&mut app).unwrap();
        let (first, second) = paras(&app, id);
        assert_eq!((first.hyphenation.after_first, first.hyphenation.before_last), (3, 4));
        assert_eq!((second.hyphenation.after_first, second.hyphenation.before_last), (2, 4));
        // Nothing changed: no undo step.
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.hyphenation", json!({})).unwrap();
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.hyphenation", json!({})).is_err());
        assert!(app.ui.dialog.is_none());
    }

    /// When paragraphs in scope disagree, only shared hyphenation values are filled in.
    #[test]
    fn hyphenation_dialog_mixed_paragraph_fields_show_blank() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "afterFirst": 3})).unwrap();
        app.run("text.setFormat", json!({"ids": [id], "afterFirst": 2, "start": 5, "end": 5})).unwrap();
        app.run("ui.hyphenation", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert!(!d.fields.contains_key("afterFirst"));
        assert_eq!(d.f64("beforeLast", 0.0), 3.0);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("afterFirst".into(), json!(4));
        super::super::confirm(&mut app).unwrap();
        let (first, second) = paras(&app, id);
        assert_eq!((first.hyphenation.after_first, second.hyphenation.after_first), (4, 4));
        assert_eq!((first.hyphenation.before_last, second.hyphenation.before_last), (3, 3));
    }
}
