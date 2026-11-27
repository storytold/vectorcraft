//! New Brush (the Brushes panel's New Brush button, its menu's New Brush…, and art dropped on the
//! panel): the type of brush to make. Scatter, Art and Pattern are off when no art is selected or
//! dropped. OK opens that type's Brush Options for the new brush
//! ([`super::brush_options::open_new`]), whose OK makes it; Cancel makes nothing.
//!
//! Fields: `type` (`calligraphic` | `scatter` | `art` | `bristle` | `pattern`) and `__art` (art is
//! selected or was dropped). The ids of the dropped art (without them, the selection's) stay with
//! the dialog's OK.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use serde_json::Value;

use super::{brush_options, host};
use crate::VectorcraftApp;

/// The dialog key of New Brush.
pub const KIND: &str = "newBrush";

/// The brush types, in the Brushes panel's order.
const TYPES: [&str; 5] = ["calligraphic", "scatter", "art", "bristle", "pattern"];

/// Open New Brush for the art of `ids` (art dropped on the Brushes panel), else for the selection.
/// It starts on Art when there is art, and it starts on Calligraphic otherwise.
pub fn open(app: &mut VectorcraftApp, ids: Option<Value>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let document = st.uid;
    let art = match &ids {
        Some(ids) => ids.as_array().is_some_and(|a| !a.is_empty()),
        None => !st.selection.is_empty(),
    };
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let fields =
        DialogFields::from_pairs([("type", FieldValue::Text(if art { "art" } else { "calligraphic" }.into())), ("__art", FieldValue::Bool(art))]);
    let (rows, title) = (params(art), Label::from(tl!("New Brush")));
    // PLAN: `HostedParamDialog::new` takes (ctx, key, fields, build); the sheet's (KIND, title, build) has no such form.
    let dialog =
        HostedParamDialog::new(&ctx, KIND, fields, move |f| ParamDialog::new(KIND, title.clone(), rows.clone()).values(Bind::bound(f))).held_ok();
    host::open_held(app, dialog, move |app, answer| {
        if app.session.active().is_none_or(|st| st.uid != document) {
            return Err(tl!("Return to the original drawing to make the brush").into());
        }
        confirm(app, &answer.fields, ids.clone())
    })
    .ok_or("no window")?;
    Ok(Value::Null)
}

/// Does a brush of type `ty` need art?
fn needs_art(ty: &str) -> bool {
    matches!(ty, "scatter" | "art" | "pattern")
}

/// The type the fields choose: `type`, or Calligraphic when it names none.
fn chosen(d: &DialogFields) -> &'static str {
    let ty = d.text("type").unwrap_or_default();
    TYPES.into_iter().find(|t| *t == ty).unwrap_or("calligraphic")
}

/// The rows: the five types, those that need art off when there is none, and the note that says why.
fn params(art: bool) -> Vec<Param> {
    let labels = [tl!("Calligraphic Brush"), tl!("Scatter Brush"), tl!("Art Brush"), tl!("Bristle Brush"), tl!("Pattern Brush")];
    let options = TYPES.into_iter().zip(labels).map(|(ty, label)| (ty.to_string(), Label::from(label))).collect();
    let mut kind = Param::new("type", "", ParamKind::radio(options, Some("calligraphic".into())));
    if !art {
        kind = kind.disabled_options(TYPES.into_iter().filter(|ty| needs_art(ty)));
    }
    vec![
        kind,
        Param::read_only_text("note", "", tl!("Select art to make a scatter, art or pattern brush.")).full_width().visible_when("__art", false),
    ]
}

/// OK: the chosen type's Brush Options for the new brush. A type that needs art, without art,
/// keeps the dialog open (automation can set such a type without art).
fn confirm(app: &mut VectorcraftApp, d: &DialogFields, ids: Option<Value>) -> Result<(), String> {
    let ty = chosen(d);
    if needs_art(ty) && !d.bool("__art").unwrap_or(false) {
        return Err("nothing selected".into());
    }
    brush_options::open_new(app, ty, ids).map(|_| ())
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::StackDialog;
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app
    }

    fn library(app: &mut VectorcraftApp) -> Vec<(String, String)> {
        crate::panels::brushes::brushes(app).0
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// The values of the open hosted dialog.
    fn open_fields(app: &VectorcraftApp) -> DialogFields {
        app.dialogs().unwrap().with(|h| h.get::<HostedParamDialog>().expect("a hosted dialog is open").fields().clone())
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|h| h.get::<HostedParamDialog>().is_some())
    }

    /// Sets `values` on the open hosted dialog, presses OK, then runs the held-dialog pump and a frame,
    /// which closes the dialog the answer was for.
    fn press_ok(app: &mut VectorcraftApp, values: Value) {
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        let id = host.with(|h| {
            let id = h.get::<HostedParamDialog>().expect("a hosted dialog is open").id();
            for (key, value) in values.as_object().unwrap() {
                h.set_json_field(&ctx, id, key, value.clone()).unwrap();
            }
            // A held OK files its answer here (`false`): the first frame runs it, the second closes the dialog.
            h.confirm_by_id(&ctx, id);
            id
        });
        for _ in 0..2 {
            crate::testing::app_frame(app, &ctx, Default::default());
        }
        assert!(host.with(|h| h.open_dialogs().iter().all(|d| d.id != id)), "OK closed the dialog");
    }

    /// OK opens the chosen type's Brush Options for a new brush, named New <Type> Brush; their OK
    /// makes the brush with the name and values set, from the selected art, in one undo step.
    #[test]
    fn ok_opens_the_types_brush_options_and_their_ok_makes_the_brush() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        let (before, steps) = (library(&mut app), undo_steps(&app));
        app.run("ui.newBrush", json!({})).unwrap();
        press_ok(&mut app, json!({"type": "scatter"}));
        let d = open_fields(&app);
        assert_eq!((d.text("__type"), d.text("name")), (Some("scatter"), Some("New Scatter Brush")));
        assert!(matches!(d.get("__def"), Some(FieldValue::Json(def)) if def["art"].is_object()), "the selected art, for the preview");
        assert_eq!(library(&mut app), before, "no brush until OK");
        // The name is trimmed, and a name another brush has gets a number, as `brush.new` gives it.
        press_ok(&mut app, json!({"name": " Dots ", "sizeMin": 40, "spacingMin": 150}));
        assert!(!is_open(&app));
        assert_eq!(library(&mut app)[before.len()..], [("Dots 2".to_string(), "scatter".to_string())]);
        let def = app.run("brush.get", json!({"name": "Dots 2"})).unwrap();
        assert_eq!((&def["size"][0], &def["spacing"][0]), (&json!(40.0), &json!(150.0)));
        assert!(def["art"].is_object(), "{def}");
        assert_eq!(undo_steps(&app), steps + 1);
        // A Calligraphic brush, with no art.
        app.run("ui.newBrush", json!({})).unwrap();
        press_ok(&mut app, json!({"type": "calligraphic"}));
        assert_eq!(open_fields(&app).text("name"), Some("New Calligraphic Brush"));
        press_ok(&mut app, json!({"size": 9}));
        let def = app.run("brush.get", json!({"name": "New Calligraphic Brush"})).unwrap();
        assert_eq!((&def["type"], &def["size"]), (&json!("calligraphic"), &json!(9.0)));
    }

    #[test]
    fn the_new_brush_type_chooser_waits_for_its_source_drawing() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        app.run("ui.newBrush", json!({})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        let id = craft::ui::dialog::dialog_id(KIND);
        app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, "type", json!("scatter"))).unwrap();
        app.run("file.new", json!({"width": 500, "height": 400})).unwrap();
        app.run("shape.ellipse", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();

        let error = super::super::tests_export::try_confirm(&mut app).expect_err("cannot take another drawing's artwork");
        assert!(error.contains("original drawing"), "{error}");
        assert_eq!(open_fields(&app).text("type"), Some("scatter"));
        app.run("document.activate", json!({"index": 0})).unwrap();
        press_ok(&mut app, json!({}));
        assert_eq!(open_fields(&app).text("__type"), Some("scatter"));
    }

    /// The new brush takes the art its Brush Options opened with: changing or clearing the
    /// selection while the dialog is open does not change the art, and OK still makes the brush.
    #[test]
    fn ok_makes_the_brush_from_the_art_the_dialog_opened_with() {
        for (ty, key) in [("scatter", "art"), ("art", "art"), ("pattern", "side")] {
            for change in ["select.set", "select.none"] {
                let mut app = app();
                app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
                app.run("ui.newBrush", json!({})).unwrap();
                press_ok(&mut app, json!({"type": ty}));
                let Some(FieldValue::Json(def)) = open_fields(&app).get("__def").cloned() else { panic!("{ty}: no definition") };
                let captured = def[key].clone();
                assert!(captured.is_object(), "{ty}");
                if change == "select.set" {
                    app.run("shape.ellipse", json!({"x": 100, "y": 50, "width": 120, "height": 90})).unwrap();
                } else {
                    app.run("select.none", json!({})).unwrap();
                }
                press_ok(&mut app, json!({}));
                assert!(!is_open(&app), "{ty} after {change}");
                let name = format!("New {}{} Brush", ty[..1].to_uppercase(), &ty[1..]);
                let def = app.run("brush.get", json!({ "name": name })).unwrap();
                assert_eq!(def[key], captured, "{ty} after {change}");
            }
        }
    }
}
