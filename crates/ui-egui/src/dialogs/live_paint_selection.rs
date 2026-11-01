//! Live Paint Selection Tool Options (double-click the Live Paint Selection tool): what a click
//! with the tool selects and how the face or edge under the pointer highlights. The options last
//! across tool switches and are saved with the preferences. OK sets them with
//! `tool.setOption {tool: "livePaintSelection", values}`.
//!
//! Fields: `selectFills` and `selectStrokes` (the faces and the edges of Live Paint groups),
//! `color` (the highlight color: `#rrggbb`, or a layer color's name such as "Red") and `width`
//! (the highlight's width in screen pixels).

use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::LAYER_COLORS;
use vectorcraft_tools::builder::LP_SELECT_WIDTH_RANGE;

use super::tile_edge_color::resolve;
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Live Paint Selection Tool Options.
pub const KIND: &str = "livePaintSelectionOptions";

/// The tool the dialog serves.
const TOOL: &str = "livePaintSelection";

/// The options the dialog edits.
const KEYS: [&str; 4] = ["selectFills", "selectStrokes", "color", "width"];

/// Width of the label column.
const LABEL_W: f32 = 60.0;

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 300.0, ..DialogSpec::FORM };

fn heading(_: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(TOOL).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the Live Paint Selection Tool Options with the tool's current values.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let o = app.session.tool_options_of(TOOL);
    let fields: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({ "dialog": KIND }))
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    form::check(ui, d, "selectFills", tl!("Select Fills"));
    form::check(ui, d, "selectStrokes", tl!("Select Strokes"));
    ui.add_space(6.0);
    let color = resolve(&d.str("color")).unwrap_or(Color::BLACK);
    let [r, g, b, _] = color.to_rgba8(1.0);
    let preset = LAYER_COLORS.iter().position(|(_, c)| *c == [r, g, b]);
    let names: Vec<&str> = LAYER_COLORS.iter().map(|(n, _)| *n).chain(["Custom"]).collect();
    widgets::label_row(ui, tl!("Color:"), LABEL_W, |ui| {
        if let Some((_, [r, g, b])) =
            widgets::dropdown(ui, "lp-select-color", names[preset.unwrap_or(LAYER_COLORS.len())], &names, 130.0).and_then(|i| LAYER_COLORS.get(i))
        {
            d.fields.insert("color".into(), json!(Color::rgb8(*r, *g, *b).to_hex()));
        }
        // The color button sets any other color; the dropdown then shows Custom.
        let mut rgb = [r, g, b];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            d.fields.insert("color".into(), json!(Color::rgb8(rgb[0], rgb[1], rgb[2]).to_hex()));
        }
    });
    let (min, max) = LP_SELECT_WIDTH_RANGE;
    let v = d.f64("width", min);
    widgets::label_row(ui, tl!("Width:"), LABEL_W, |ui| {
        if let Some(x) = widgets::range_field(ui, "lp-select-width", v, min..=max, "px", 0, 72.0) {
            d.fields.insert("width".into(), json!(x));
        }
    });
    false
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = KEYS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": TOOL, "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_tools::{Overlay, PointerEvent, PointerKind};

    /// The labels the open dialog paints.
    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn double_clicking_the_tool_opens_its_options_and_ok_sets_them() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        // Two squares side by side, made a Live Paint group: two faces and the edge between them.
        for x in [0, 100] {
            app.run("shape.rectangle", json!({"x": 50 + x, "y": 50, "width": 100, "height": 100})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("livePaint.make", json!({})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        app.select_tool(TOOL);
        let click = |app: &mut VectorcraftApp, x: f64| {
            let view = app.view_info();
            crate::canvas::dispatch(app, &PointerEvent::new(PointerKind::Down, x, 100.0), view);
            crate::canvas::dispatch(app, &PointerEvent::new(PointerKind::Up, x, 100.0), view);
            app.session.doc().unwrap().selection.objects.len()
        };
        assert_eq!(click(&mut app, 100.0), 1, "a face is selected by default");

        app.run("tool.options", json!({"tool": TOOL})).unwrap();
        let text = painted(&mut app);
        for label in ["Live Paint Selection Tool Options", "Select Fills", "Select Strokes", "Color:", "Width:"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.kind, KIND);
        assert_eq!((d.bool("selectFills"), d.bool("selectStrokes"), d.str("color"), d.f64("width", 0.0)), (true, true, "#ff2a2a".to_string(), 4.0));
        d.fields.insert("selectFills".into(), json!(false));
        d.fields.insert("color".into(), json!("Green"));
        d.fields.insert("width".into(), json!(8));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        // The tool takes them: a click inside a face selects nothing, and the highlight shows the edge
        // in the new color and width.
        assert_eq!(click(&mut app, 100.0), 0);
        let view = app.view_info();
        crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Move, 151.0, 100.0), view);
        let ov = app.session.overlays(view);
        assert!(ov.iter().any(|o| matches!(o, Overlay::Path { color: [0x4f, 0xff, 0x4f], width: 8.0, .. })), "{ov:?}");
        // They last while another tool is chosen, and the dialog opens on them again.
        app.select_tool("selection");
        let o = app.session.tool_options_of(TOOL);
        assert_eq!((o["selectFills"].as_bool(), o["color"].as_str(), o["width"].as_f64()), (Some(false), Some("#4fff4f"), Some(8.0)));
        app.run("tool.options", json!({"tool": TOOL})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.bool("selectFills"), d.str("color")), (false, "#4fff4f".to_string()));
    }
}
