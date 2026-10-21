//! New Brush (the Brushes panel's New Brush button, its menu's New Brush…, and art dropped on the
//! panel): the type of brush to make. Scatter, Art and Pattern are off when no art is selected or
//! dropped. OK opens that type's Brush Options for the new brush
//! ([`super::brush_options::open_new`]), whose OK makes it; Cancel makes nothing.
//!
//! Fields: `type` (`calligraphic` | `scatter` | `art` | `bristle` | `pattern`), `__art` (art is
//! selected or was dropped) and `__ids` (the ids of the dropped art; without it, the selection's).

use serde_json::{Value, json};

use super::{DialogSpec, brush_options};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of New Brush.
pub const KIND: &str = "newBrush";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("New Brush").into(), body, confirm, ok_enabled: Some(ok_enabled), min_width: 300.0, ..DialogSpec::FORM };

/// The brush types, in the Brushes panel's order.
const TYPES: [&str; 5] = ["calligraphic", "scatter", "art", "bristle", "pattern"];

/// Open New Brush for the art of `ids` (art dropped on the Brushes panel), else for the selection.
/// It starts on Art when there is art, and it starts on Calligraphic otherwise.
pub fn open(app: &mut VectorcraftApp, ids: Option<Value>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let art = match &ids {
        Some(ids) => ids.as_array().is_some_and(|a| !a.is_empty()),
        None => !st.selection.is_empty(),
    };
    let mut f = json!({ "type": if art { "art" } else { "calligraphic" }, "__art": art });
    if let Some(ids) = ids {
        f["__ids"] = ids;
    }
    app.ui.dialog = Some(Dialog::new(KIND, f));
    Ok(Value::Null)
}

/// Does a brush of type `ty` need art?
fn needs_art(ty: &str) -> bool {
    matches!(ty, "scatter" | "art" | "pattern")
}

/// The type the fields choose: `type`, or Calligraphic when it names none.
fn chosen(d: &Dialog) -> &'static str {
    let ty = d.str("type");
    TYPES.into_iter().find(|t| *t == ty).unwrap_or("calligraphic")
}

fn ok_enabled(_: &VectorcraftApp, d: &Dialog) -> bool {
    d.bool("__art") || !needs_art(chosen(d))
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let labels = [tl!("Calligraphic Brush"), tl!("Scatter Brush"), tl!("Art Brush"), tl!("Bristle Brush"), tl!("Pattern Brush")];
    let (art, cur) = (d.bool("__art"), chosen(d));
    for (ty, label) in TYPES.into_iter().zip(labels) {
        if widgets::radio(ui, label, cur == ty, art || !needs_art(ty)) {
            d.fields.insert("type".into(), json!(ty));
        }
    }
    if !art {
        ui.add_space(6.0);
        widgets::dim_label(ui, tl!("Select art to make a scatter, art or pattern brush."));
    }
    false
}

/// OK: the chosen type's Brush Options for the new brush. A type that needs art, without art,
/// keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let ty = chosen(d);
    if needs_art(ty) && !d.bool("__art") {
        return Err("nothing selected".into());
    }
    brush_options::open_new(app, ty, d.fields.get("__ids").cloned())
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::widgets::accessibility_tests::{named, tree};

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app
    }

    fn set(app: &mut VectorcraftApp, fields: Value) {
        let d = app.ui.dialog.as_mut().unwrap();
        for (k, v) in fields.as_object().unwrap() {
            d.fields.insert(k.clone(), v.clone());
        }
    }

    fn library(app: &mut VectorcraftApp) -> Vec<(String, String)> {
        crate::panels::brushes::brushes(app).0
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// Which of the five types screen readers find on, in the dialog's order.
    fn types_on(app: &mut VectorcraftApp) -> Vec<bool> {
        let t = tree(|ui| super::super::show(app, ui.ctx()));
        ["Calligraphic Brush", "Scatter Brush", "Art Brush", "Bristle Brush", "Pattern Brush"].map(|l| !named(&t, l).is_disabled()).to_vec()
    }

    /// With nothing selected, Scatter, Art and Pattern are off, New Brush starts on Calligraphic,
    /// and OK on one of them keeps the dialog open. With art selected, every type is on and New
    /// Brush starts on Art.
    #[test]
    fn scatter_art_and_pattern_brushes_need_art() {
        let mut app = app();
        app.run("ui.newBrush", json!({})).unwrap();
        assert_eq!(types_on(&mut app), [true, false, false, true, false]);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("type"), "calligraphic");
        set(&mut app, json!({"type": "art"}));
        assert_eq!(super::super::confirm(&mut app), Err("nothing selected".into()));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        app.ui.dialog = None;
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        app.run("ui.newBrush", json!({})).unwrap();
        assert_eq!(types_on(&mut app), [true; 5]);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("type"), "art");
    }

    /// OK opens the chosen type's Brush Options for a new brush, named New <Type> Brush; their OK
    /// makes the brush with the name and values set, from the selected art, in one undo step.
    #[test]
    fn ok_opens_the_types_brush_options_and_their_ok_makes_the_brush() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        let (before, steps) = (library(&mut app), undo_steps(&app));
        app.run("ui.newBrush", json!({})).unwrap();
        set(&mut app, json!({"type": "scatter"}));
        super::super::confirm(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().expect("Scatter Brush Options");
        assert_eq!((d.kind.as_str(), d.str("__type"), d.str("name")), (super::brush_options::KIND, "scatter".into(), "New Scatter Brush".into()));
        assert!(d.fields["__def"]["art"].is_object(), "the selected art, for the preview");
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        assert!(text.contains("Scatter Brush Options") && text.contains("Spacing:"), "{text}");
        assert_eq!(library(&mut app), before, "no brush until OK");
        // The name is trimmed, and a name another brush has gets a number, as `brush.new` gives it.
        set(&mut app, json!({"name": " Dots ", "sizeMin": 40, "spacingMin": 150}));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(library(&mut app)[before.len()..], [("Dots 2".to_string(), "scatter".to_string())]);
        let def = app.run("brush.get", json!({"name": "Dots 2"})).unwrap();
        assert_eq!((&def["size"][0], &def["spacing"][0]), (&json!(40.0), &json!(150.0)));
        assert!(def["art"].is_object(), "{def}");
        assert_eq!(undo_steps(&app), steps + 1);
        // A Calligraphic brush, with no art.
        app.run("ui.newBrush", json!({})).unwrap();
        set(&mut app, json!({"type": "calligraphic"}));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "New Calligraphic Brush");
        set(&mut app, json!({"size": 9}));
        super::super::confirm(&mut app).unwrap();
        let def = app.run("brush.get", json!({"name": "New Calligraphic Brush"})).unwrap();
        assert_eq!((&def["type"], &def["size"]), (&json!("calligraphic"), &json!(9.0)));
    }

    /// Cancel in New Brush, or in the Brush Options it opens, adds no brush and no undo step.
    #[test]
    fn cancel_adds_no_brush() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        let before = (library(&mut app), undo_steps(&app));
        app.run("ui.newBrush", json!({})).unwrap();
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        for ty in TYPES {
            app.run("ui.newBrush", json!({})).unwrap();
            set(&mut app, json!({"type": ty}));
            super::super::confirm(&mut app).unwrap();
            assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::brush_options::KIND), "{ty}");
            super::super::cancel(&mut app);
            assert!(app.ui.dialog.is_none(), "{ty}");
            assert_eq!((library(&mut app), undo_steps(&app)), before, "{ty}");
        }
    }
}
