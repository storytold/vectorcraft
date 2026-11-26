//! New Swatch: the Swatch Options fields (name, colour type, Global, mode, sliders, hex and the
//! colour warnings) prefilled from the active fill or stroke; a gradient, a pattern or a tint of a global colour
//! (saved as a tint swatch, "Name 40%") only takes a name. OK runs `swatch.new`.
//!
//! Fields: `name`, `type`, `global`, `mode` and `color` as in Swatch Options; a gradient, pattern or tint has
//! `name` only. The colour group the swatch goes into and the `swatch.new` paint of a gradient, pattern or
//! tint are kept by the dialog's OK.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, ParamResult};
use serde_json::{Value, json};
use vectorcraft_color::Paint;

use super::host::{HeldOptions, open_held_with};
use super::swatch_options::{NO_WINDOW, color_of, colour_fields, colour_params, colour_rows, hosted, name_row, with_dialog};
use crate::VectorcraftApp;
use crate::panels::color::gamut;

/// The dialog kind of New Swatch.
pub const KIND: &str = "newSwatch";

/// The open New Swatch dialog: the held dialog and the default name, which follows the colour until it is edited.
#[derive(Clone, Debug)]
pub(crate) struct Open {
    /// The held dialog.
    pub(crate) id: DialogId,
    /// The name the dialog proposes for its colour.
    auto: String,
}

/// Open New Swatch for the active paint, going into color `group`. A color opens as a spot color
/// with `spot`, and as a global process color with `global` alone.
pub fn open(app: &mut VectorcraftApp, spot: bool, global: bool, group: Option<&str>) -> Result<Value, String> {
    let paint = crate::panels::active_paint(app);
    let name = app.session.active().ok_or("no document open")?.doc.new_swatch_name(&paint);
    let mut fields = DialogFields::new();
    fields.set("name", name.as_str());
    let (rows, kept) = match &paint {
        Paint::Solid { swatch: Some(_), tint, .. } if *tint < 1.0 => (vec![name_row()], Some(crate::panels::paint_params(&paint))),
        Paint::Solid { color, .. } => {
            colour_fields(&mut fields, *color, spot, global);
            (colour_rows(gamut(app)), None)
        }
        Paint::None => return Err("pick a colour, gradient or pattern to save as a swatch".into()),
        other => (vec![name_row()], Some(crate::panels::swatch_paint_params(other))),
    };
    let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
    let dialog = hosted(&ctx, KIND, tl!("New Swatch"), fields, rows, false);
    let group = group.unwrap_or_default().to_owned();
    let ok = move |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        app.run("swatch.new", params(kept.as_ref(), &group, &answer.fields))?;
        app.ui.new_swatch = None;
        Ok(())
    };
    let options = HeldOptions { parent: None, cancel: Some(Box::new(|app: &mut VectorcraftApp| app.ui.new_swatch = None)) };
    let id = open_held_with(app, dialog, ok, options).ok_or(NO_WINDOW)?;
    app.ui.new_swatch = Some(Open { id, auto: name });
    Ok(Value::Null)
}

/// Keeps the default name following the colour while the name has not been edited, and applies the corrections the
/// colour row asked for.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some((id, auto)) = app.ui.new_swatch.as_ref().map(|o| (o.id, o.auto.clone())) else { return };
    super::swatch_options::correct(app, id);
    let Some(fields) = with_dialog(app, id, |d| d.fields().clone()) else { return };
    if !fields.contains("color") || fields.text("name") != Some(auto.as_str()) {
        return;
    }
    let Some(next) = app.session.active().map(|st| st.doc.new_swatch_name(&Paint::solid(color_of(&fields)))) else { return };
    if next != auto {
        with_dialog(app, id, |d| d.set_field("name", FieldValue::Text(next.clone())));
        if let Some(open) = app.ui.new_swatch.as_mut() {
            open.auto = next;
        }
    }
}

/// `swatch.new` parameters: the `paint` of a gradient, pattern or tint, else the dialog's colour; with the name and
/// the colour `group` when there is one.
fn params(paint: Option<&Value>, group: &str, fields: &DialogFields) -> Value {
    let mut p = paint.cloned().unwrap_or_else(|| colour_params(fields));
    p["name"] = json!(fields.text("name").unwrap_or_default());
    if !group.is_empty() {
        p["group"] = json!(group);
    }
    p
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::super::swatch_options::{Warning, corrected};
    use super::*;
    use craft::color_ui::ui::ColorPanelMode;

    /// An app with a window for its dialogs and one open document.
    fn setup() -> (egui::Context, VectorcraftApp) {
        let ctx = crate::testing::ui_context();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui_ctx = Some(ctx.clone());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        (ctx, app)
    }

    /// Answers OK in the open dialog and lets the host run it.
    fn ok(app: &mut VectorcraftApp, ctx: &egui::Context) {
        app.dialogs().unwrap().with(|h| h.confirm_by_id(ctx, dialog_id(KIND)));
        crate::dialogs::host::pump(app, ctx);
    }

    #[test]
    fn a_warning_corrects_the_new_colour_only_until_ok() {
        let (ctx, mut app) = setup();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("paint.setFill", json!({"color": "#0000fe"})).unwrap();
        let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.swatches_iter().count();
        let before = count(&app);
        open(&mut app, false, false, None).unwrap();
        let blue = vectorcraft_color::Color::from_hex("#0000fe").unwrap();
        let (web, mode) = corrected(&mut app, Warning::Web, ColorPanelMode::RgbSliders, &blue).unwrap().unwrap();
        assert_eq!((web.to_hex(), mode), ("#0000ff".to_string(), ColorPanelMode::RgbSliders));
        let host = app.dialogs().unwrap();
        host.with(|h| h.set_json_field(&ctx, dialog_id(KIND), "color", json!(web))).unwrap();
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!(count(&app), before, "no swatch before OK");
        let id = app.ui.new_swatch.as_ref().unwrap().id;
        let name = with_dialog(&app, id, |d| d.fields().text("name").map(str::to_owned)).flatten().unwrap();
        ok(&mut app, &ctx);
        assert_eq!(app.session.active().unwrap().doc.swatch(&name).and_then(|s| s.paint.color()).map(|c| c.to_hex()), Some("#0000ff".into()));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(count(&app), before, "one undo step");
    }

    #[test]
    fn a_tint_opens_as_a_named_tint_swatch() {
        let (ctx, mut app) = setup();
        app.run("swatch.new", json!({"name": "Ink", "color": "#cc0066", "spot": true})).unwrap();
        app.run("paint.setFill", json!({"swatch": "Ink", "tint": 40})).unwrap();
        open(&mut app, false, false, None).unwrap();
        ok(&mut app, &ctx);
        let w = app.session.doc().unwrap().doc.swatch("Ink 40%").cloned().unwrap();
        assert_eq!(w.tint_of(), Some(("Ink", 0.4)));
    }
}
