//! Paragraph panel menu › Justification… (or `ui.justification`): Word Spacing, Letter Spacing and
//! Glyph Scaling (each Minimum, Desired and Maximum, in percent), Auto Leading and Single Word
//! Justification of the paragraphs the Paragraph panel acts on. The dialog opens on the values the
//! Paragraph panel shows and previews them on the canvas. OK runs `text.setFormat` with the values
//! that changed, as one undo step; paragraphs that differ keep the values left alone. OK is
//! disabled while a row's Minimum exceeds its Desired value or Desired its Maximum.
//!
//! Fields: `wordMin`, `wordDesired`, `wordMax` (0..1000), `letterMin`, `letterDesired`,
//! `letterMax` (-100..500), `glyphMin`, `glyphDesired`, `glyphMax` (50..200), `autoLeading`
//! (0..500), `singleWord` (`fullJustify`, `alignLeft`, `alignCenter`, `alignRight`) and `preview`,
//! with `__initial` (the values it opened with) and `__targets` (the type and range it acts on).
//!
//! Behaviour reference (public documentation):
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/adjust-word-and-letterspacing-in-justified-text.html>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{JustificationOptions, SingleWordJustify};

use super::swatch_options::label;
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Justification.
pub const KIND: &str = "justification";

/// The field holding the values the dialog opened with.
const INITIAL: &str = "__initial";
/// The field holding the `ids`, `start` and `end` the dialog's `text.setFormat` acts on.
const TARGETS: &str = "__targets";
const CMD: &str = "text.setFormat";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Justification").into(),
    body,
    confirm,
    preview: true,
    min_width: 460.0,
    ok_enabled: Some(|_, d| problem(d).is_none()),
    ..DialogSpec::FORM
};

/// The three spacing rows: (`text.setFormat` key, field prefix, range).
const ROWS: [(&str, &str, (f64, f64)); 3] = [
    ("wordSpacing", "word", JustificationOptions::WORD_SPACING),
    ("letterSpacing", "letter", JustificationOptions::LETTER_SPACING),
    ("glyphScaling", "glyph", JustificationOptions::GLYPH_SCALING),
];
/// The columns of a row: (field suffix, `text.setFormat` key).
const COLUMNS: [(&str, &str); 3] = [("Min", "min"), ("Desired", "desired"), ("Max", "max")];

/// Single Word Justification's choices: (`text.setFormat` value, label).
fn single_word_choices() -> [(&'static str, &'static str); 4] {
    [("fullJustify", tl!("Full Justify")), ("alignLeft", tl!("Align left")), ("alignCenter", tl!("Align center")), ("alignRight", tl!("Align right"))]
}

fn single_word_name(s: SingleWordJustify) -> &'static str {
    match s {
        SingleWordJustify::FullJustify => "fullJustify",
        SingleWordJustify::AlignLeft => "alignLeft",
        SingleWordJustify::AlignCenter => "alignCenter",
        SingleWordJustify::AlignRight => "alignRight",
    }
}

/// Open Justification on the paragraph attributes the Paragraph panel shows.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let (_, para) = crate::panels::character::text_style(app).ok_or("select some type first")?;
    // While typing, the dialog acts on the paragraphs of the text being edited.
    let mut targets = json!({});
    if let Some((id, a, b)) = crate::panels::character::text_editing(app) {
        crate::panels::character::end_typing(app);
        targets = json!({"ids": [id.0], "start": a, "end": b});
    }
    let j = &para.justification;
    let mut values = Map::new();
    for ((_, prefix, _), v) in ROWS.iter().zip([j.word_spacing, j.letter_spacing, j.glyph_scaling]) {
        for ((suffix, _), x) in COLUMNS.iter().zip(v) {
            values.insert(format!("{prefix}{suffix}"), json!(x));
        }
    }
    values.insert("autoLeading".into(), json!(j.auto_leading));
    values.insert("singleWord".into(), json!(single_word_name(j.single_word)));
    let mut fields = Value::Object(values.clone());
    fields[INITIAL] = Value::Object(values);
    fields[TARGETS] = targets;
    fields["preview"] = json!(false);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// Why OK can't apply the dialog: a row whose values are out of order.
fn problem(d: &Dialog) -> Option<&'static str> {
    let out_of_order = ROWS.iter().any(|(_, prefix, _)| {
        let [min, desired, max] = COLUMNS.map(|(suffix, _)| d.f64(&format!("{prefix}{suffix}"), 0.0));
        !(min <= desired && desired <= max)
    });
    out_of_order.then(|| tl!("Minimum must not exceed Desired, nor Desired Maximum."))
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    // Its own grid: a label column and three value columns.
    egui::Grid::new("justification-spacing").num_columns(4).spacing([10.0, 8.0]).min_col_width(70.0).show(ui, |ui| {
        ui.label("");
        for heading in [tl!("Minimum"), tl!("Desired"), tl!("Maximum")] {
            ui.label(heading);
        }
        ui.end_row();
        for ((_, prefix, (lo, hi)), text) in ROWS.iter().zip([tl!("Word Spacing:"), tl!("Letter Spacing:"), tl!("Glyph Scaling:")]) {
            label(ui, text);
            for (suffix, _) in COLUMNS {
                let key = format!("{prefix}{suffix}");
                if let Some(v) = widgets::range_field(ui, ("justification", key.as_str()), d.f64(&key, 0.0), *lo..=*hi, "%", 0, 70.0) {
                    d.fields.insert(key, json!(v));
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    egui::Grid::new("justification-more").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        label(ui, tl!("Auto Leading:"));
        let (lo, hi) = JustificationOptions::AUTO_LEADING;
        if let Some(v) = widgets::range_field(ui, ("justification", "autoLeading"), d.f64("autoLeading", 120.0), lo..=hi, "%", 0, 64.0) {
            d.fields.insert("autoLeading".into(), json!(v));
        }
        ui.end_row();
        label(ui, tl!("Single Word Justification:"));
        let choices = single_word_choices();
        let current = d.str("singleWord");
        let shown = choices.iter().find(|(v, _)| *v == current).map_or(choices[0].1, |(_, l)| *l);
        let labels = choices.map(|(_, l)| l);
        if let Some(i) = widgets::dropdown(ui, ("justification", "singleWord"), shown, &labels, 140.0)
            && let Some((v, _)) = choices.get(i)
        {
            d.fields.insert("singleWord".into(), json!(v));
        }
        ui.end_row();
    });
    match problem(d) {
        Some(why) => {
            ui.add_space(6.0);
            ui.colored_label(crate::theme::Tokens::get(ui.ctx()).error, why);
        }
        None => form::preview(app, ui, d, tl!("Justification"), CMD, Value::Object(params(d))),
    }
    false
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with,
/// on the dialog's type and range.
fn params(d: &Dialog) -> Map<String, Value> {
    let initial = d.fields.get(INITIAL).and_then(Value::as_object);
    let was = |key: &str| initial.and_then(|m| m.get(key)).and_then(Value::as_f64);
    let mut p = d.fields.get(TARGETS).and_then(Value::as_object).cloned().unwrap_or_default();
    for (param, prefix, _) in ROWS {
        let mut changed = Map::new();
        for (suffix, name) in COLUMNS {
            let key = format!("{prefix}{suffix}");
            let v = d.f64(&key, 0.0);
            if was(&key) != Some(v) {
                changed.insert(name.into(), json!(v));
            }
        }
        if !changed.is_empty() {
            p.insert(param.into(), Value::Object(changed));
        }
    }
    let leading = d.f64("autoLeading", 120.0);
    if was("autoLeading") != Some(leading) {
        p.insert("autoLeading".into(), json!(leading));
    }
    let single = d.str("singleWord");
    if initial.and_then(|m| m.get("singleWord")).and_then(Value::as_str) != Some(single.as_str()) {
        p.insert("singleWordJustification".into(), json!(single));
    }
    p
}

/// Does `p` change anything (more than the targets)?
fn changes(p: &Map<String, Value>) -> bool {
    p.keys().any(|k| !matches!(k.as_str(), "ids" | "start" | "end"))
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if let Some(why) = problem(d) {
        return Err(why.into());
    }
    let p = params(d);
    if !changes(&p) {
        if app.session.in_interaction() {
            let _ = app.session.cancel_interaction();
        }
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    form::commit_preview(app, CMD, Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind, ParaStyle};

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

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn set(app: &mut VectorcraftApp, key: &str, v: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), v);
    }

    /// `ui.justification` opens the dialog on the selected type's values; OK sets the paragraphs'
    /// justification options in one undo step.
    #[test]
    fn justification_dialog_sets_the_paragraphs_options() {
        let (mut app, id) = app();
        app.run("ui.justification", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Justification opened");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.f64("wordMin", 0.0), d.f64("wordDesired", 0.0), d.f64("wordMax", 0.0)), (80.0, 100.0, 133.0));
        assert_eq!((d.f64("glyphDesired", 0.0), d.f64("autoLeading", 0.0), d.str("singleWord")), (100.0, 120.0, "fullJustify".to_string()));
        set(&mut app, "wordMax", json!(150));
        set(&mut app, "letterMax", json!(5));
        set(&mut app, "glyphMin", json!(97));
        set(&mut app, "autoLeading", json!(140));
        set(&mut app, "singleWord", json!("alignRight"));
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let want = JustificationOptions {
            word_spacing: [80.0, 100.0, 150.0],
            letter_spacing: [0.0, 0.0, 5.0],
            glyph_scaling: [97.0, 100.0, 100.0],
            auto_leading: 140.0,
            single_word: SingleWordJustify::AlignRight,
        };
        for p in [paras(&app, id).0, paras(&app, id).1] {
            assert_eq!(p.justification, want);
        }
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }

    /// OK sends only the values that changed: paragraphs whose other values differ keep them. Values
    /// out of order keep the dialog open and change nothing.
    #[test]
    fn justification_dialog_leaves_unchanged_values_alone_and_refuses_values_out_of_order() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "wordSpacing": [70, 100, 200], "start": 5, "end": 5})).unwrap();
        app.run("ui.justification", json!({})).unwrap();
        set(&mut app, "letterMax", json!(20));
        // Desired above Maximum: refused.
        set(&mut app, "glyphDesired", json!(150));
        let d = app.ui.dialog.clone().unwrap();
        assert!(problem(&d).is_some());
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some(), "the dialog stays open");
        assert_eq!(paras(&app, id).0.justification, JustificationOptions::default());
        set(&mut app, "glyphDesired", json!(100.0));
        super::super::confirm(&mut app).unwrap();
        let (first, second) = paras(&app, id);
        assert_eq!((first.justification.word_spacing, first.justification.letter_spacing), ([80.0, 100.0, 133.0], [0.0, 0.0, 20.0]));
        assert_eq!((second.justification.word_spacing, second.justification.letter_spacing), ([70.0, 100.0, 200.0], [0.0, 0.0, 20.0]));
        // Nothing changed: no undo step.
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.justification", json!({})).unwrap();
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.justification", json!({})).is_err());
        assert!(app.ui.dialog.is_none());
    }

    /// Preview shows the values on the canvas; Cancel rolls them back.
    #[test]
    fn justification_dialog_previews_and_cancel_restores() {
        let (mut app, id) = app();
        app.run("ui.justification", json!({})).unwrap();
        set(&mut app, "autoLeading", json!(200));
        set(&mut app, "preview", json!(true));
        frame(&mut app);
        assert!(app.session.in_interaction(), "the preview runs");
        assert_eq!(paras(&app, id).0.justification.auto_leading, 200.0);
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert_eq!(paras(&app, id).0.justification.auto_leading, 120.0);
    }
}
