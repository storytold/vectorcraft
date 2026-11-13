//! Character panel menu › Warichu Settings… (or `ui.warichuSettings`): the Lines, Line Gap,
//! Scale and Alignment of the characters the Character panel acts on, which set how a block of
//! warichu characters is stacked inside its line. The dialog opens on the values the selected
//! characters share (a blank field where they differ). OK runs `text.setFormat`
//! (`text.setRangeStyle` while the Type tool edits text) with the values that changed, as one undo
//! step; characters that differ keep a value left blank. It does not turn Warichu on or off.
//!
//! Fields: `warichuLines` (1..6), `warichuGap` (pt, 0..100), `warichuScale` (%, 10..100) and
//! `warichuAlign`, as `text.setFormat` takes them, and `__initial` (the values it opened with).
//!
//! Behavior reference (public documentation):
//! <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::WarichuAlign;
use vectorcraft_doc::text::{MAX_WARICHU_GAP, WARICHU_LINES, WARICHU_SCALE};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Warichu Settings.
pub const KIND: &str = "warichuSettings";

/// The field holding the values the dialog opened with.
const INITIAL: &str = "__initial";

/// The fields, as `text.setFormat` names them.
const KEYS: [&str; 4] = ["warichuLines", "warichuGap", "warichuScale", "warichuAlign"];

/// The Alignment choices, by the value `text.setFormat` takes and their label.
fn aligns() -> [(&'static str, &'static str); 8] {
    WarichuAlign::ALL.map(|a| {
        let label = match a {
            WarichuAlign::Auto => tl!("Auto"),
            WarichuAlign::Left => tl!("Align left"),
            WarichuAlign::Center => tl!("Align center"),
            WarichuAlign::Right => tl!("Align right"),
            WarichuAlign::JustifyLeft => tl!("Justify with last line aligned left"),
            WarichuAlign::JustifyCenter => tl!("Justify with last line aligned center"),
            WarichuAlign::JustifyRight => tl!("Justify with last line aligned right"),
            WarichuAlign::JustifyAll => tl!("Justify all lines"),
        };
        (a.id(), label)
    })
}

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Warichu Settings").into(),
    body,
    confirm,
    min_width: 360.0,
    // A value the characters don't share opens blank, without its field.
    inputs: |_| &KEYS,
    ..DialogSpec::FORM
};

/// Open Warichu Settings on the characters the Character panel acts on.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let shared = crate::panels::character::shared_warichu(app);
    let mut values = Map::new();
    if let Some(v) = shared.lines {
        values.insert("warichuLines".into(), json!(v));
    }
    if let Some(v) = shared.gap {
        values.insert("warichuGap".into(), json!(v));
    }
    if let Some(v) = shared.scale {
        values.insert("warichuScale".into(), json!(v));
    }
    if let Some(v) = shared.align {
        values.insert("warichuAlign".into(), json!(v.id()));
    }
    let mut fields = Value::Object(values.clone());
    fields[INITIAL] = Value::Object(values);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.type_unit();
    let lines = f64::from(*WARICHU_LINES.start())..=f64::from(*WARICHU_LINES.end());
    grid(ui, |ui| {
        label(ui, tl!("Lines:"));
        let value = d.fields.get("warichuLines").and_then(Value::as_f64);
        if let Some(n) = crate::widgets::range_field(ui, ("warichu", "warichuLines"), value, lines, "", 0, 60.0) {
            d.fields.insert("warichuLines".into(), json!(n));
        }
        ui.end_row();
        label(ui, tl!("Line Gap:"));
        form::length_within(ui, d, "warichuGap", unit, 90.0, Some(0.0..=MAX_WARICHU_GAP));
        ui.end_row();
        label(ui, tl!("Scale:"));
        let value = d.fields.get("warichuScale").and_then(Value::as_f64);
        if let Some(n) = crate::widgets::range_field(ui, ("warichu", "warichuScale"), value, WARICHU_SCALE, "%", 1, 70.0) {
            d.fields.insert("warichuScale".into(), json!(n));
        }
        ui.end_row();
        label(ui, tl!("Alignment:"));
        let current = d.str("warichuAlign");
        let options = aligns();
        let shown = options.iter().find(|(id, _)| *id == current).map_or("", |(_, l)| l);
        let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
        if let Some((id, _)) = crate::widgets::dropdown_names(ui, "warichuAlign", shown, &labels, 240.0).and_then(|i| options.get(i)) {
            d.fields.insert("warichuAlign".into(), json!(id));
        }
        ui.end_row();
    });
    false
}

/// A number field's value: a number, or a string that reads as one (`"3"`, `"40%"`); `None` for a
/// blank field. `Err` names the field when it holds anything else.
fn number(d: &Dialog, key: &str, name: &str) -> Result<Option<f64>, String> {
    let v = match d.fields.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => return Ok(None),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().trim_end_matches('%').trim().parse::<f64>().ok(),
        Some(_) => None,
    };
    v.filter(|v| v.is_finite()).map(Some).ok_or_else(|| format!("{name} must be a number"))
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with.
/// Lines is rounded to a whole number and kept within 1..6, Line Gap is in points (a length typed
/// with its unit, "1 mm", is converted) and kept within 0..100, and Scale is kept within 10..100.
/// A blank field is left alone; a field that is not a number is an error.
fn params(d: &Dialog) -> Result<Map<String, Value>, String> {
    let initial = d.fields.get(INITIAL).and_then(Value::as_object);
    let unchanged = |key: &str, v: &Value| initial.and_then(|m| m.get(key)) == Some(v);
    let mut p = Map::new();
    if let Some(n) = number(d, "warichuLines", "Lines")? {
        let n = n.round().clamp(f64::from(*WARICHU_LINES.start()), f64::from(*WARICHU_LINES.end()));
        // Within 1..6 after the clamp, so the cast keeps the value.
        let v = json!(n as u8);
        if !unchanged("warichuLines", &v) {
            p.insert("warichuLines".into(), v);
        }
    }
    let gap = match d.fields.get("warichuGap") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::Number(n)) => Some(n.as_f64().filter(|v| v.is_finite()).ok_or("Line Gap must be a number")?),
        Some(Value::String(s)) => Some(vectorcraft_doc::Unit::Points.parse(s).filter(|v| v.is_finite()).ok_or("Line Gap must be a number")?),
        Some(_) => return Err("Line Gap must be a number".into()),
    };
    if let Some(v) = gap.map(|v| json!(v.clamp(0.0, MAX_WARICHU_GAP)))
        && initial.and_then(|m| m.get("warichuGap")).and_then(Value::as_f64) != v.as_f64()
    {
        p.insert("warichuGap".into(), v);
    }
    if let Some(n) = number(d, "warichuScale", "Scale")? {
        let v = n.clamp(*WARICHU_SCALE.start(), *WARICHU_SCALE.end());
        if initial.and_then(|m| m.get("warichuScale")).and_then(Value::as_f64) != Some(v) {
            p.insert("warichuScale".into(), json!(v));
        }
    }
    match d.fields.get("warichuAlign") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s.is_empty() => {}
        Some(Value::String(s)) if WarichuAlign::from_id(s).is_some() => {
            let v = json!(s);
            if !unchanged("warichuAlign", &v) {
                p.insert("warichuAlign".into(), v);
            }
        }
        Some(_) => return Err("Alignment must be one of the choices".into()),
    }
    Ok(p)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(d)?;
    app.ui.dialog = None;
    if p.is_empty() {
        return Ok(Value::Null);
    }
    crate::panels::character::run_format(app, Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind};

    /// A selected point type object with four characters.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "abcd"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    /// (warichu, Lines, Line Gap, Scale, Alignment) of each run.
    fn settings(app: &VectorcraftApp, id: u64) -> Vec<(bool, u8, f64, f64, WarichuAlign)> {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t
                .runs
                .iter()
                .map(|r| (r.style.warichu, r.style.warichu_lines, r.style.warichu_gap, r.style.warichu_scale, r.style.warichu_align))
                .collect(),
            _ => panic!("not text"),
        }
    }

    /// `ui.warichuSettings` opens on the characters' values; OK sets the changed ones in one undo
    /// step without turning Warichu on, a length typed with its unit is converted to points, and
    /// values past the ends are kept to them.
    #[test]
    fn warichu_settings_dialog_sets_lines_gap_scale_and_alignment() {
        let (mut app, id) = app();
        app.run("ui.warichuSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Warichu Settings opened");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.f64("warichuLines", 0.0), d.f64("warichuGap", -1.0), d.f64("warichuScale", 0.0)), (2.0, 0.0, 50.0));
        assert_eq!(d.str("warichuAlign"), "auto");
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("warichuLines".into(), json!(3));
        d.fields.insert("warichuGap".into(), json!("0.5 in"));
        d.fields.insert("warichuScale".into(), json!(400));
        d.fields.insert("warichuAlign".into(), json!("center"));
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(settings(&app, id), [(false, 3, 36.0, 100.0, WarichuAlign::Center)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // It opens on the values set, and OK with nothing changed adds no undo step.
        app.run("ui.warichuSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.f64("warichuLines", 0.0), d.f64("warichuGap", 0.0), d.f64("warichuScale", 0.0)), (3.0, 36.0, 100.0));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.warichuSettings", json!({})).is_err());
        assert!(app.ui.dialog.is_none());
    }

    /// Characters that differ open a blank field; OK leaves that value alone and sets the others.
    #[test]
    fn warichu_settings_dialog_leaves_mixed_values_blank() {
        let (mut app, id) = app();
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 2, "warichuLines": 4})).unwrap();
        app.run("ui.warichuSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert!(!d.fields.contains_key("warichuLines"));
        assert_eq!(d.f64("warichuScale", 0.0), 50.0);
        app.ui.dialog.as_mut().unwrap().fields.insert("warichuScale".into(), json!(60));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(settings(&app, id), [(false, 4, 0.0, 60.0, WarichuAlign::Auto), (false, 2, 0.0, 60.0, WarichuAlign::Auto)]);
    }

    /// OK with a value that is not a number returns an error naming the field, keeps the dialog
    /// open and leaves the characters alone.
    #[test]
    fn warichu_settings_dialog_rejects_a_value_that_is_not_a_number() {
        let (mut app, id) = app();
        for (key, bad, message) in [
            ("warichuLines", json!("two"), "Lines must be a number"),
            ("warichuGap", json!("wide"), "Line Gap must be a number"),
            ("warichuScale", json!(true), "Scale must be a number"),
            ("warichuAlign", json!("sideways"), "Alignment must be one of the choices"),
        ] {
            app.run("ui.warichuSettings", json!({})).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), bad);
            assert_eq!(super::super::confirm(&mut app), Err(message.to_string()));
            assert!(app.ui.dialog.is_some());
            assert_eq!(settings(&app, id), [(false, 2, 0.0, 50.0, WarichuAlign::Auto)]);
            app.ui.dialog = None;
        }
    }

    /// While the Type tool edits text, OK sets the selected characters.
    #[test]
    fn warichu_settings_dialog_sets_the_range_the_type_tool_selects() {
        let (mut app, id) = app();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": id, "anchor": 0, "caret": 2}));
        app.run("ui.warichuSettings", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("warichuAlign".into(), json!("justifyAll"));
        assert!(super::super::confirm(&mut app).is_ok());
        assert_eq!(settings(&app, id), [(false, 2, 0.0, 50.0, WarichuAlign::JustifyAll), (false, 2, 0.0, 50.0, WarichuAlign::Auto)]);
    }
}
