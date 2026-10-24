//! Object → Slice → Slice Options… and Divide Slices….
//!
//! Slice Options (`sliceOptions`, also a double click with the Slice Selection tool) edits the
//! selected slices' options: fields `kind` (`image`, `noImage`, `htmlText`), `name`, `url`,
//! `target`, `message`, `alt`, `text`, `background` (`""`, `matte` or `#rrggbb`), `hAlign`,
//! `vAlign`, and `__html` (whether HTML Text applies). OK runs `object.slice.options`.
//!
//! Divide Slices (`divideSlices`): `divideRows` with `rows` (slices down) or, with `rowMode:
//! "size"`, `rowHeight` (pt); `divideColumns` with `columns` or `columnWidth`. OK runs
//! `object.slice.divide`.

use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::{CellAlign, CellVAlign, SliceKind};

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Slice Options.
pub const OPTIONS: &str = "sliceOptions";
/// The dialog kind of Divide Slices.
pub const DIVIDE: &str = "divideSlices";

pub(super) const OPTIONS_SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Slice Options").into(), body: options_body, confirm: options_confirm, min_width: 380.0, ..DialogSpec::FORM };

pub(super) const DIVIDE_SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Divide Slices").into(), body: divide_body, confirm: divide_confirm, min_width: 320.0, ..DialogSpec::FORM };

/// Label column and field widths of the dialogs.
const LABEL_W: f32 = 110.0;
const FIELD_W: f32 = 220.0;

/// Open Slice Options on the selected slices' options (those of the first one).
pub fn open_options(app: &mut VectorcraftApp) -> Result<Value, String> {
    let v = app.session.execute("object.slice.options", &json!({})).map_err(|e| e.to_string())?;
    let mut fields: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    let html = fields.remove("htmlText").unwrap_or(json!(false));
    fields.insert("__html".into(), html);
    app.ui.dialog = Some(Dialog { kind: OPTIONS.into(), fields });
    Ok(Value::Null)
}

/// Open Divide Slices: two rows and two columns, the sizes halving the first selected slice.
pub fn open_divide(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let first = st.selection.slices.iter().chain(&st.selection.objects).find_map(|id| st.doc.slice_bounds(*id));
    let (w, h) = first.map_or((50.0, 50.0), |r| (r.width() / 2.0, r.height() / 2.0));
    app.ui.dialog = Some(Dialog::new(
        DIVIDE,
        json!({"divideRows": true, "rows": 2, "rowMode": "count", "rowHeight": h, "divideColumns": true, "columns": 2, "columnMode": "count", "columnWidth": w}),
    ));
    Ok(Value::Null)
}

/// `(value, label)` pairs of a keyed enum's values.
fn pairs<T: Copy>(all: &[T], key: fn(T) -> &'static str, label: fn(T) -> &'static str) -> Vec<(&'static str, &'static str)> {
    all.iter().map(|v| (key(*v), label(*v))).collect()
}

fn options_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let kinds: Vec<_> = SliceKind::ALL.iter().filter(|k| **k != SliceKind::HtmlText || d.bool("__html")).copied().collect();
    form::choice(ui, d, "kind", tl!("Slice Type:"), (LABEL_W, FIELD_W), &pairs(&kinds, SliceKind::key, SliceKind::label));
    ui.add_space(6.0);
    let kind = SliceKind::parse(&d.str("kind")).unwrap_or_default();
    match kind {
        SliceKind::Image => {
            for (key, label) in
                [("name", tl!("Name:")), ("url", tl!("URL:")), ("target", tl!("Target:")), ("message", tl!("Message:")), ("alt", tl!("Alt:"))]
            {
                widgets::label_row(ui, label, LABEL_W, |ui| {
                    form::text(ui, d, key, FIELD_W - 12.0);
                });
            }
        }
        SliceKind::NoImage => {
            widgets::dim_label(ui, tl!("Text Displayed in Cell:"));
            form::text_area(ui, d, "text", LABEL_W + FIELD_W - 12.0, 4);
        }
        SliceKind::HtmlText => {
            widgets::dim_label(ui, tl!("The cell shows the type object's text as HTML."));
        }
    }
    if kind != SliceKind::Image {
        ui.add_space(6.0);
        form::choice(ui, d, "hAlign", tl!("Horizontal:"), (LABEL_W, FIELD_W), &pairs(CellAlign::ALL, CellAlign::key, CellAlign::label));
        form::choice(ui, d, "vAlign", tl!("Vertical:"), (LABEL_W, FIELD_W), &pairs(CellVAlign::ALL, CellVAlign::key, CellVAlign::label));
    }
    ui.add_space(6.0);
    background_row(ui, d);
    false
}

/// Background: None, Matte, White, Black or Other (a colour button for any colour).
fn background_row(ui: &mut egui::Ui, d: &mut Dialog) {
    const NAMES: [&str; 5] = ["None", "Matte", "White", "Black", "Other"];
    let cur = d.str("background").to_ascii_lowercase();
    let shown = match cur.as_str() {
        "" => 0,
        "matte" => 1,
        "#ffffff" => 2,
        "#000000" => 3,
        _ => 4,
    };
    widgets::label_row(ui, tl!("Background:"), LABEL_W, |ui| {
        if let Some(i) = widgets::dropdown(ui, "slice-background", NAMES[shown], &NAMES, 120.0) {
            let v = match i {
                0 => "",
                1 => "matte",
                2 => "#ffffff",
                3 => "#000000",
                _ => "#808080",
            };
            d.fields.insert("background".into(), json!(v));
        }
        if let Some(c) = Color::from_hex(&cur) {
            let [r, g, b, _] = c.to_rgba8(1.0);
            let mut rgb = [r, g, b];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                d.fields.insert("background".into(), json!(Color::rgb8(rgb[0], rgb[1], rgb[2]).to_hex()));
            }
        }
    });
}

fn options_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p: Map<String, Value> = d.fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    run_and_close(app, "object.slice.options", Value::Object(p))
}

/// One direction of Divide Slices: the checkbox, then "N slices …, evenly spaced" or "N pt per
/// slice".
fn divide_section(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, keys: [&str; 4], labels: [&str; 2]) {
    let [on, count, mode, size] = keys;
    if widgets::check(ui, labels[0], d.bool(on), true) {
        d.fields.insert(on.into(), json!(!d.bool(on)));
    }
    let enabled = d.bool(on);
    let by_size = d.str(mode) == "size";
    ui.add_enabled_ui(enabled, |ui| {
        ui.indent(on, |ui| {
            ui.horizontal(|ui| {
                if widgets::radio_button(ui, labels[1], !by_size, enabled) {
                    d.fields.insert(mode.into(), json!("count"));
                }
                if let Some(n) = widgets::range_field(ui, ("divide", count), d.f64(count, 2.0), 1.0..=f64::INFINITY, "", 0, 50.0) {
                    d.fields.insert(count.into(), json!(n as u64));
                }
                widgets::dim_label(ui, labels[1]);
            });
            ui.horizontal(|ui| {
                if widgets::radio_button(ui, tl!("per slice"), by_size, enabled) {
                    d.fields.insert(mode.into(), json!("size"));
                }
                form::length(ui, d, size, app.session.general_unit(), 70.0);
                widgets::dim_label(ui, tl!("per slice"));
            });
        });
    });
}

fn divide_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    divide_section(app, ui, d, ["divideRows", "rows", "rowMode", "rowHeight"], [tl!("Divide Horizontally Into"), tl!("slices down, evenly spaced")]);
    ui.add_space(8.0);
    divide_section(
        app,
        ui,
        d,
        ["divideColumns", "columns", "columnMode", "columnWidth"],
        [tl!("Divide Vertically Into"), tl!("slices across, evenly spaced")],
    );
    false
}

fn divide_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = json!({});
    for [on, count, mode, size] in [["divideRows", "rows", "rowMode", "rowHeight"], ["divideColumns", "columns", "columnMode", "columnWidth"]] {
        if !d.bool(on) {
            continue;
        }
        if d.str(mode) == "size" {
            p[size] = json!(d.f64(size, 0.0));
        } else {
            p[count] = json!(d.f64(count, 2.0).round().max(1.0) as u64);
        }
    }
    if p.as_object().is_none_or(|o| o.is_empty()) {
        return Err("choose a direction to divide in".into());
    }
    run_and_close(app, "object.slice.divide", p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.session.paint.stroke = vectorcraft_color::Paint::None;
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("object.slice.make", json!({"ids": [id]})).unwrap();
        app
    }

    /// Draw the open dialog once, headlessly.
    fn draw(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
    }

    #[test]
    fn slice_options_opens_on_the_selection_and_ok_sets_them() {
        let mut app = app();
        // The menu item (no params) opens the dialog through the app.
        app.run("object.slice.options", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, OPTIONS);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("kind"), "image");
        for kind in ["image", "noImage"] {
            set(&mut app, "kind", json!(kind));
            draw(&mut app);
        }
        set(&mut app, "text", json!("Hello <i>cell</i>"));
        set(&mut app, "background", json!("#00ff00"));
        draw(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let l = app.run("slice.list", json!({})).unwrap();
        let own = l["slices"].as_array().unwrap().iter().find(|s| s["source"] == "object").unwrap().clone();
        assert_eq!(own["options"]["kind"], "noImage");
        assert_eq!(own["options"]["text"], "Hello <i>cell</i>");
        assert_eq!(own["options"]["background"], "#00ff00");
        // One undo step.
        app.run("edit.undo", json!({})).unwrap();
        let l = app.run("slice.list", json!({})).unwrap();
        assert!(l["slices"].as_array().unwrap().iter().any(|s| s["options"]["kind"] == "image" && s["source"] == "object"));
    }

    /// Screen readers name Divide Slices' radio buttons by the text after their fields.
    #[test]
    fn screen_readers_name_the_divide_radio_buttons() {
        use crate::widgets::accessibility_tests::{role_and_state, tree};
        use egui::accesskit::{Role, Toggled};
        let mut app = app();
        app.run("object.slice.divide", json!({})).unwrap();
        set(&mut app, "divideRows", json!(true));
        let shown = tree(|ui| super::super::show(&mut app, ui.ctx()));
        assert_eq!(role_and_state(&shown, "slices down, evenly spaced"), (Role::RadioButton, Some(Toggled::True)));
        assert_eq!(role_and_state(&shown, "per slice"), (Role::RadioButton, Some(Toggled::False)));
    }

    #[test]
    fn divide_slices_cuts_by_count_or_size() {
        let mut app = app();
        app.run("object.slice.divide", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, DIVIDE);
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("columnWidth", 0.0), 30.0);
        draw(&mut app);
        set(&mut app, "columns", json!(3));
        set(&mut app, "rowMode", json!("size"));
        set(&mut app, "rowHeight", json!(15));
        super::super::confirm(&mut app).unwrap();
        // 40 pt in 15 pt rows: 3 rows; 3 columns.
        assert_eq!(app.session.active().unwrap().doc.slices.len(), 9);
        app.run("object.slice.divide", json!({})).unwrap();
        set(&mut app, "divideRows", json!(false));
        set(&mut app, "divideColumns", json!(false));
        assert!(super::super::confirm(&mut app).is_err());
    }
}
