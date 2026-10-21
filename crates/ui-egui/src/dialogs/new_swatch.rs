//! New Swatch: the Swatch Options fields (name, colour type, Global, mode, sliders and hex)
//! prefilled from the active fill or stroke; a gradient, a pattern or a tint of a global colour
//! (saved as a tint swatch, "Name 40%") only takes a name. OK runs `swatch.new`.
//!
//! Fields: `name`, `spot`, `global`, `mode` and `color` as in Swatch Options, `group` (the colour
//! group the colour goes into; empty for none), `__paint` (the `swatch.new` paint of a gradient,
//! pattern or tint; absent for a colour) and `__auto` (the default name, which follows the colour until
//! the name is edited).

use serde_json::{Value, json};
use vectorcraft_color::Paint;

use super::swatch_options::{color_of, editor, grid, mode_id, name_row};
use super::{DialogSpec, run_and_close};
use crate::VectorcraftApp;
use crate::panels::color::Mode;
use crate::state::Dialog;

/// The dialog kind of New Swatch.
pub const KIND: &str = "newSwatch";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("New Swatch").into(), body, confirm, min_width: 340.0, ..DialogSpec::FORM };

/// Open New Swatch for the active paint, going into color `group`. A color opens as a spot color
/// with `spot`, and as a global process color with `global` alone.
pub fn open(app: &mut VectorcraftApp, spot: bool, global: bool, group: Option<&str>) -> Result<Value, String> {
    let paint = crate::panels::active_paint(app);
    let name = app.session.active().ok_or("no document open")?.doc.new_swatch_name(&paint);
    let mut fields = match &paint {
        Paint::Solid { swatch: Some(_), tint, .. } if *tint < 1.0 => {
            json!({"__paint": crate::panels::paint_params(&paint), "group": group.unwrap_or_default()})
        }
        Paint::Solid { color, .. } => {
            json!({"color": color, "mode": mode_id(Mode::of(color)), "spot": spot, "global": spot || global, "group": group.unwrap_or_default()})
        }
        Paint::None => return Err("pick a colour, gradient or pattern to save as a swatch".into()),
        other => json!({"__paint": crate::panels::swatch_paint_params(other)}),
    };
    fields["name"] = json!(name);
    fields["__auto"] = json!(name);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if d.fields.contains_key("__paint") {
        grid(ui, |ui| name_row(ui, d));
    } else if editor(ui, d)
        && d.str("name") == d.str("__auto")
        && let Some(st) = app.session.active()
    {
        // The default name follows the colour until it is edited.
        let auto = st.doc.new_swatch_name(&Paint::solid(color_of(d)));
        d.fields.insert("name".into(), json!(auto));
        d.fields.insert("__auto".into(), json!(auto));
    }
    false
}

/// `swatch.new` parameters from the fields.
fn params(d: &Dialog) -> Value {
    let mut p = match d.fields.get("__paint") {
        Some(paint) => paint.clone(),
        None => json!({"color": color_of(d), "mode": d.str("mode"), "spot": d.bool("spot"), "global": d.bool("global")}),
    };
    p["name"] = json!(d.str("name"));
    let group = d.str("group");
    if !group.is_empty() {
        p["group"] = json!(group);
    }
    p
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    run_and_close(app, "swatch.new", params(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_carry_the_colour_or_the_paint() {
        let d = Dialog::new(KIND, json!({"name": "Sky", "color": "#2a6fb0", "mode": "rgb", "spot": true, "global": true, "group": "Mine"}));
        assert_eq!(params(&d), json!({"name": "Sky", "color": color_of(&d), "mode": "rgb", "spot": true, "global": true, "group": "Mine"}));
        let d = Dialog::new(KIND, json!({"name": "Dots", "__paint": {"pattern": "Dots"}, "group": ""}));
        assert_eq!(params(&d), json!({"name": "Dots", "pattern": "Dots"}));
    }

    #[test]
    fn a_tint_opens_as_a_named_tint_swatch() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": "#cc0066", "spot": true})).unwrap();
        app.run("paint.setFill", json!({"swatch": "Ink", "tint": 40})).unwrap();
        open(&mut app, false, false, None).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!(params(&d), json!({"name": "Ink 40%", "swatch": "Ink", "tint": 40.0}));
        run_and_close(&mut app, "swatch.new", params(&d)).unwrap();
        let w = app.session.doc().unwrap().doc.swatch("Ink 40%").cloned().unwrap();
        assert_eq!(w.tint_of(), Some(("Ink", 0.4)));
    }
}
