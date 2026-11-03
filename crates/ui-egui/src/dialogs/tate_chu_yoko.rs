//! Character panel menu › Tate-chu-yoko Settings… (or `ui.tateChuYokoSettings`): the Up/Down and
//! Left/Right of the characters the Character panel acts on, in points, which move a block of
//! tate-chu-yoko characters in vertical type up or down and right or left. The dialog opens on
//! the values the selected characters share (a blank field where they differ). OK runs
//! `text.setFormat` (`text.setRangeStyle` while the Type tool edits text) with the values that
//! changed, as one undo step; characters that differ keep a value left blank.
//!
//! Fields: `tcyUpDown` and `tcyLeftRight` (pt, -1296..1296), as `text.setFormat` takes them, and
//! `__initial` (the values it opened with).
//!
//! Behavior reference (public documentation):
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/format-east-asian-characters-in-vertical-text.html>.

use serde_json::{Map, Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Tate-chu-yoko Settings.
pub const KIND: &str = "tateChuYokoSettings";

/// The field holding the values the dialog opened with.
const INITIAL: &str = "__initial";

/// The fields, as `text.setFormat` names them.
const KEYS: [&str; 2] = ["tcyUpDown", "tcyLeftRight"];

/// The range of both fields, in points.
const RANGE: std::ops::RangeInclusive<f64> = -1296.0..=1296.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Tate-chu-yoko Settings").into(),
    body,
    confirm,
    min_width: 280.0,
    // A value the characters don't share opens blank, without its field.
    inputs: |_| &KEYS,
    ..DialogSpec::FORM
};

/// Open Tate-chu-yoko Settings on the characters the Character panel acts on.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let (up, right) = crate::panels::character::shared_tcy_offsets(app);
    let mut values = Map::new();
    for (key, v) in KEYS.into_iter().zip([up, right]) {
        if let Some(v) = v {
            values.insert(key.into(), json!(v));
        }
    }
    let mut fields = Value::Object(values.clone());
    fields[INITIAL] = Value::Object(values);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.type_unit();
    grid(ui, |ui| {
        for (key, text) in KEYS.into_iter().zip([tl!("Up/Down:"), tl!("Left/Right:")]) {
            label(ui, text);
            form::length_within(ui, d, key, unit, 90.0, Some(RANGE));
            ui.end_row();
        }
    });
    false
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with,
/// in points (a length typed with its unit, "1 mm", is converted) and kept within -1296..1296.
fn params(d: &Dialog) -> Map<String, Value> {
    let initial = d.fields.get(INITIAL).and_then(Value::as_object);
    let mut p = Map::new();
    for key in KEYS {
        if !d.fields.contains_key(key) {
            continue;
        }
        let v = d.f64(key, 0.0).clamp(*RANGE.start(), *RANGE.end());
        if initial.and_then(|m| m.get(key)).and_then(Value::as_f64) != Some(v) {
            p.insert(key.into(), json!(v));
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
    crate::panels::character::run_format(app, Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind};

    /// A selected point type object with three characters.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
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

    /// `ui.tateChuYokoSettings` opens on the characters' values; OK sets both in one undo step,
    /// and a length typed with its unit is converted to points.
    #[test]
    fn tate_chu_yoko_settings_dialog_sets_up_down_and_left_right() {
        let (mut app, id) = app();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Tate-chu-yoko Settings opened");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.f64("tcyUpDown", -1.0), d.f64("tcyLeftRight", -1.0)), (0.0, 0.0));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("tcyUpDown".into(), json!(3));
        d.fields.insert("tcyLeftRight".into(), json!("-1 in"));
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(offsets(&app, id), [(3.0, -72.0)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // It opens on the values set, and OK with nothing changed adds no undo step.
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.f64("tcyUpDown", 0.0), d.f64("tcyLeftRight", 0.0)), (3.0, -72.0));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // Cancel changes nothing.
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("tcyUpDown".into(), json!(9));
        app.ui.dialog = None;
        assert_eq!(offsets(&app, id), [(3.0, -72.0)]);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.tateChuYokoSettings", json!({})).is_err());
        assert!(app.ui.dialog.is_none());
    }

    /// Characters that differ open a blank field; OK leaves that value alone and sets the other.
    #[test]
    fn tate_chu_yoko_settings_dialog_leaves_mixed_values_blank() {
        let (mut app, id) = app();
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 2, "tcyUpDown": 4})).unwrap();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert!(!d.fields.contains_key("tcyUpDown"));
        assert_eq!(d.f64("tcyLeftRight", -1.0), 0.0);
        app.ui.dialog.as_mut().unwrap().fields.insert("tcyLeftRight".into(), json!(2));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(offsets(&app, id), [(4.0, 2.0), (0.0, 2.0)]);
    }

    /// OK returns the error of a command that is refused, and the characters keep their values.
    #[test]
    fn tate_chu_yoko_settings_dialog_returns_a_refused_command_error() {
        let (mut app, id) = app();
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("tcyUpDown".into(), json!(5));
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_none());
        assert_eq!(offsets(&app, id), [(0.0, 0.0)]);
    }

    /// While the Type tool edits text, OK sets the selected characters and returns the result.
    #[test]
    fn tate_chu_yoko_settings_dialog_sets_the_range_the_type_tool_selects() {
        let (mut app, id) = app();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": id, "anchor": 0, "caret": 2}));
        assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(id), 0, 2)));
        app.run("ui.tateChuYokoSettings", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("tcyLeftRight".into(), json!(6));
        assert!(super::super::confirm(&mut app).is_ok());
        assert_eq!(offsets(&app, id), [(0.0, 6.0), (0.0, 0.0)]);
    }
}
