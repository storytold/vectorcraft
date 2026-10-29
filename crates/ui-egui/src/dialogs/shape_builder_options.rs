//! Shape Builder Tool Options (double-click the Shape Builder tool): the options the tool keeps,
//! which last across tool switches and are saved with the preferences. OK sets them with
//! `tool.setOption {tool: "shapeBuilder", values}`.
//!
//! Fields: `gapDetection` and `gapLength` (Gap Detection), `openFilledAsClosed` (Consider Open
//! Filled Paths as Closed), `clickStrokeSplitsPath` (In Merge Mode, Clicking Stroke Splits the
//! Path), `pickColorFrom` (`artwork` or `swatches`), `cursorSwatchPreview` (with `swatches`),
//! `selection` (`freeform` or `straightLine`), `highlightFill` (Highlight: Fill),
//! `highlightStroke` (Highlight: Stroke When Editable) and `highlightColor` (its color: `#rrggbb`,
//! or a layer color's name such as "Red").

use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::LAYER_COLORS;

use super::tile_edge_color::resolve;
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Shape Builder Tool Options.
pub const KIND: &str = "shapeBuilderOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// The options the dialog edits.
const KEYS: [&str; 10] = [
    "openFilledAsClosed",
    "pickColorFrom",
    "cursorSwatchPreview",
    "selection",
    "highlightFill",
    "gapDetection",
    "gapLength",
    "clickStrokeSplitsPath",
    "highlightStroke",
    "highlightColor",
];

/// Width of the label column.
const LABEL_W: f32 = 110.0;

fn heading(_: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info("shapeBuilder").map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the Shape Builder's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let o = app.session.tool_options_of("shapeBuilder");
    let fields: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    form::check(ui, d, "gapDetection", tl!("Gap Detection"));
    ui.add_enabled_ui(d.bool("gapDetection"), |ui| {
        widgets::label_row(ui, tl!("Gap Length:"), LABEL_W, |ui| {
            let lengths = vectorcraft_tools::builder::GAP_LENGTHS;
            let len = d.f64("gapLength", lengths[0]);
            let current = lengths.iter().position(|n| *n == len).unwrap_or(3);
            let labels = [tl!("Small"), tl!("Medium"), tl!("Large"), tl!("Custom")];
            if let Some(i) = widgets::dropdown(ui, "shape-builder-gap", labels[current], &labels, 120.0) {
                d.fields.insert("gapLength".into(), json!(lengths.get(i).copied().unwrap_or(9.0)));
            }
            ui.add_enabled_ui(current == 3, |ui| {
                if let Some(n) =
                    widgets::range_field(ui, "shape-builder-gap-length", len, 0.0..=vectorcraft_tools::builder::SHAPE_BUILDER_MAX_GAP, "pt", 1, 64.0)
                {
                    d.fields.insert("gapLength".into(), json!(n));
                }
            });
        });
    });
    ui.add_space(8.0);
    form::check(ui, d, "openFilledAsClosed", tl!("Consider Open Filled Paths as Closed"));
    form::check(ui, d, "clickStrokeSplitsPath", tl!("In Merge Mode, Clicking Stroke Splits the Path"));
    ui.add_space(8.0);
    let sources = [("artwork", tl!("Artwork")), ("swatches", tl!("Color Swatches"))];
    form::choice(ui, d, "pickColorFrom", tl!("Pick Color From:"), (LABEL_W, 160.0), &sources);
    // Cursor Swatch Preview can be changed with Color Swatches only.
    let preview = d.bool("cursorSwatchPreview");
    if widgets::check(ui, tl!("Cursor Swatch Preview"), preview, d.str("pickColorFrom") == "swatches") {
        d.fields.insert("cursorSwatchPreview".into(), json!(!preview));
    }
    ui.add_space(8.0);
    let selection = d.str("selection");
    widgets::label_row(ui, tl!("Selection:"), LABEL_W, |ui| {
        for (value, label) in [("freeform", tl!("Freeform")), ("straightLine", tl!("Straight Line"))] {
            if widgets::radio(ui, label, selection == value, true) {
                d.fields.insert("selection".into(), json!(value));
            }
        }
    });
    widgets::label_row(ui, tl!("Highlight:"), LABEL_W, |ui| {
        form::check(ui, d, "highlightFill", tl!("Fill"));
        form::check(ui, d, "highlightStroke", tl!("Stroke When Editable"));
    });
    // The stroke highlight takes a layer color, or any other color from the color button.
    let color = resolve(&d.str("highlightColor")).unwrap_or(Color::BLACK);
    let [r, g, b, _] = color.to_rgba8(1.0);
    let preset = LAYER_COLORS.iter().position(|(_, c)| *c == [r, g, b]);
    let names: Vec<&str> = LAYER_COLORS.iter().map(|(n, _)| *n).chain(["Custom"]).collect();
    ui.add_enabled_ui(d.bool("highlightStroke"), |ui| {
        widgets::label_row(ui, tl!("Color:"), LABEL_W, |ui| {
            if let Some((_, [r, g, b])) = widgets::dropdown(ui, "shape-builder-color", names[preset.unwrap_or(LAYER_COLORS.len())], &names, 130.0)
                .and_then(|i| LAYER_COLORS.get(i))
            {
                d.fields.insert("highlightColor".into(), json!(Color::rgb8(*r, *g, *b).to_hex()));
            }
            let mut rgb = [r, g, b];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                d.fields.insert("highlightColor".into(), json!(Color::rgb8(rgb[0], rgb[1], rgb[2]).to_hex()));
            }
        });
    });
    false
}

/// OK: set the Shape Builder's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = KEYS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": "shapeBuilder", "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    /// The labels the open dialog paints.
    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn gap_options_share_the_existing_dialog_and_custom_is_capped() {
        let mut a = app();
        a.run("tool.options", json!({"tool": "shapeBuilder"})).unwrap();
        let text = painted(&mut a);
        assert!(text.contains("Gap Detection") && text.contains("Gap Length:"), "{text}");
        let d = a.ui.dialog.as_mut().unwrap();
        d.fields.insert("gapDetection".into(), json!(true));
        d.fields.insert("gapLength".into(), json!(1000));
        super::super::confirm(&mut a).unwrap();
        let opts = a.session.tool_options_of("shapeBuilder");
        assert_eq!(opts["gapLength"], 72.0);
        assert_eq!(opts["gapDetection"], true);
        assert_eq!(opts["openFilledAsClosed"], true);
    }

    #[test]
    fn double_clicking_the_tool_opens_its_options_and_ok_keeps_them() {
        let mut app = app();
        let r = app.run("tool.options", json!({"tool": "shapeBuilder"})).unwrap();
        assert_eq!(r["dialog"], json!(KIND));
        let text = painted(&mut app);
        for label in [
            "Shape Builder Tool Options",
            "Consider Open Filled Paths as Closed",
            "Pick Color From:",
            "Artwork",
            "Cursor Swatch Preview",
            "Selection:",
            "Freeform",
            "Straight Line",
            "Highlight:",
            "Fill",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        // The defaults: VectorCraft's Shape Builder as it works without the dialog.
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(
            (d.bool("openFilledAsClosed"), d.str("pickColorFrom"), d.bool("cursorSwatchPreview"), d.str("selection"), d.bool("highlightFill")),
            (true, "artwork".into(), false, "freeform".into(), true)
        );
        for (k, v) in [
            ("openFilledAsClosed", json!(false)),
            ("pickColorFrom", json!("swatches")),
            ("cursorSwatchPreview", json!(true)),
            ("selection", json!("straightLine")),
            ("highlightFill", json!(false)),
        ] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        // They last while another tool is chosen, and the dialog opens on them again.
        app.select_tool("shapeBuilder");
        app.select_tool("selection");
        let o = app.session.tool_options_of("shapeBuilder");
        assert_eq!(
            o,
            json!({
                "openFilledAsClosed": false,
                "pickColorFrom": "swatches",
                "cursorSwatchPreview": true,
                "selection": "straightLine",
                "highlightFill": false,
                "gapDetection": false,
                "gapLength": 3.0,
                "clickStrokeSplitsPath": false,
                "highlightStroke": true,
                "highlightColor": "#ff4f4f"
            })
        );
        app.run("tool.options", json!({"tool": "shapeBuilder"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("selection"), d.bool("highlightFill")), ("straightLine".into(), false));
    }

    #[test]
    fn the_stroke_rows_turn_on_stroke_splitting_and_its_highlight_color() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
        app.run("tool.options", json!({"tool": "shapeBuilder"})).unwrap();
        let text = painted(&mut app);
        for label in ["In Merge Mode, Clicking Stroke Splits the Path", "Stroke When Editable", "Color:", "Red"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.bool("clickStrokeSplitsPath"), d.bool("highlightStroke"), d.str("highlightColor")), (false, true, "#ff4f4f".into()));
        d.fields.insert("clickStrokeSplitsPath".into(), json!(true));
        d.fields.insert("highlightColor".into(), json!("Green"));
        super::super::confirm(&mut app).unwrap();
        let o = app.session.tool_options_of("shapeBuilder");
        assert_eq!((&o["clickStrokeSplitsPath"], &o["highlightStroke"], &o["highlightColor"]), (&json!(true), &json!(true), &json!("#4fff4f")));
        // A click on the rectangle's top side highlights it in that color and splits the rectangle
        // into the side and the rest.
        app.run("select.all", json!({})).unwrap();
        app.select_tool("shapeBuilder");
        let view = app.view_info();
        let ev = |kind| vectorcraft_tools::PointerEvent::new(kind, 200.0, 100.5);
        crate::canvas::dispatch(&mut app, &ev(vectorcraft_tools::PointerKind::Move), view);
        let ov = app.session.overlays(view);
        assert!(ov.iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Path { color: [0x4f, 0xff, 0x4f], .. })), "{ov:?}");
        crate::canvas::dispatch(&mut app, &ev(vectorcraft_tools::PointerKind::Down), view);
        crate::canvas::dispatch(&mut app, &ev(vectorcraft_tools::PointerKind::Up), view);
        let doc = &app.session.doc().unwrap().doc;
        let mut bounds: Vec<_> =
            doc.layers[0].children().unwrap().iter().filter_map(|n| doc.bounds_of(&[n.id], false)).map(|b| [b.x0, b.y0, b.x1, b.y1]).collect();
        bounds.sort_by(|a, b| a[3].total_cmp(&b[3]));
        assert_eq!(bounds, vec![[100.0, 100.0, 300.0, 100.0], [100.0, 100.0, 300.0, 200.0]]);
    }

    #[test]
    fn a_straight_line_drag_merges_only_the_regions_the_line_crosses() {
        // A wide rectangle and a tall one over the middle of its top edge: three regions, the wide
        // one's own, the overlap and the tall one's top.
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 180, "y": 50, "width": 40, "height": 100})).unwrap();
        // A drag that goes up into the tall rectangle's top and back down; the bounds of each
        // object afterwards.
        let drag = |app: &mut VectorcraftApp| {
            app.run("select.all", json!({})).unwrap();
            app.select_tool("shapeBuilder");
            let view = app.view_info();
            let ev = |kind, (x, y): (f64, f64)| vectorcraft_tools::PointerEvent::new(kind, x, y);
            let pts = [(120.0, 180.0), (200.0, 75.0), (280.0, 180.0)];
            crate::canvas::dispatch(app, &ev(vectorcraft_tools::PointerKind::Down, pts[0]), view);
            for p in &pts[1..] {
                crate::canvas::dispatch(app, &ev(vectorcraft_tools::PointerKind::Drag, *p), view);
            }
            crate::canvas::dispatch(app, &ev(vectorcraft_tools::PointerKind::Up, pts[2]), view);
            let doc = &app.session.doc().unwrap().doc;
            let ids: Vec<_> = doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
            ids.iter().filter_map(|id| doc.bounds_of(&[*id], false)).map(|b| [b.x0, b.y0, b.x1, b.y1].map(f64::round)).collect::<Vec<_>>()
        };
        // Freeform: the drag merges all three regions into one shape.
        assert_eq!(drag(&mut app), vec![[100.0, 50.0, 300.0, 200.0]]);
        app.run("edit.undo", json!({})).unwrap();
        // Straight Line: the line from the first point to the last stays in the wide rectangle's
        // own region, so only that region is merged: the shape made from it leaves the tall
        // rectangle's top out, and the overlap stays an object of its own.
        app.run("tool.setOption", json!({"tool": "shapeBuilder", "values": {"selection": "straightLine"}})).unwrap();
        let after = drag(&mut app);
        assert!(!after.iter().any(|b| b[1] == 50.0 && b[0] == 100.0), "{after:?}");
        assert!(after.contains(&[180.0, 100.0, 220.0, 150.0]), "{after:?}");
    }
}
