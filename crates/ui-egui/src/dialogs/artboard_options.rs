//! Artboard Options (double-click with the Artboard tool, double-click the tool, or the Artboards
//! panel's options icon or menu): the artboard's fields, sizes parsed with units, and its background
//! color (`color`: a color as the paint commands take it, or null for none).

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_engine::cmd::color_value;

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::panels::artboards::{FILLS, fill_colors, fill_name};
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Artboard Options").into(), body, confirm, ..DialogSpec::FORM };

/// The dialog kind of Artboard Options.
pub const KIND: &str = "artboardOptions";

/// Open Artboard Options for the active artboard: the Artboard tool's while it is in use, else the
/// window's.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let index = if app.session.tool_id() == "artboard" {
        app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0)
    } else {
        app.view().map_or(0, |v| v.artboard)
    };
    let st = app.session.active().ok_or("no document")?;
    let a = st.doc.artboards.get(index).ok_or("no artboard")?;
    let r = a.rect;
    let color = a.color.as_ref().map(crate::panels::color_json);
    let fields = json!({"index": index, "name": a.name, "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height(), "color": color});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

/// The fields, then Background Color: the Fill list (None, White, Black; Custom for another color)
/// and a swatch that picks a color of its own.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    form::grid(ui, d, app.session.general_unit());
    ui.add_space(8.0);
    let color = d.fields.get("color").and_then(color_value);
    let fills = fill_colors(app);
    let mut set = None;
    ui.horizontal(|ui| {
        form::row_label(ui, tl!("Background Color:"));
        if let Some(k) = crate::widgets::dropdown(ui, "ab-options-fill", fill_name(app, color), &FILLS, 110.0) {
            set = fills.get(k).copied();
        }
        let [r, g, b, _] = color.unwrap_or(Color::WHITE).to_rgba8(1.0);
        let mut rgb = [r, g, b];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            set = Some(Some(Color::rgb(f32::from(rgb[0]) / 255.0, f32::from(rgb[1]) / 255.0, f32::from(rgb[2]) / 255.0)));
        }
    });
    if let Some(c) = set {
        d.fields.insert("color".into(), c.as_ref().map_or(Value::Null, crate::panels::color_json));
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = Value::Object(d.fields.clone());
    for k in ["x", "y", "width", "height"] {
        if d.fields.contains_key(k) {
            p[k] = json!(d.f64(k, 0.0));
        }
    }
    p["moveArt"] = json!(crate::panels::artboards::move_art(app));
    run_and_close(app, "artboard.setProps", p)
}
