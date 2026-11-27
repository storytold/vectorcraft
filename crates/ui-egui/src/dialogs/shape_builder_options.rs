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

use craft::ui::contract::ShowContainer as _;
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};
use vectorcraft_tools::builder::{GAP_LENGTHS, SHAPE_BUILDER_MAX_GAP};

use super::option_form::{self, OptionSpec, Rows, is_on, pick};
use crate::VectorcraftApp;

/// The dialog kind of Shape Builder Tool Options.
pub const KIND: &str = "shapeBuilderOptions";

/// The tool whose options the dialog sets. The Fill Area tool shares them (`vectorcraft_tools::settings`), so its
/// Gap Detection and open-filled-path options take them too.
// PLAN: the sheet has `run` take the tool from `extra` ("fillArea"); the engine already gives Fill Area the Shape
// Builder's options, so the dialog always sets "shapeBuilder".
const TOOL: &str = "shapeBuilder";

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

/// The gap length picking Custom starts from, in points.
const CUSTOM_GAP: f64 = 9.0;

/// The dialog's description for the option form.
pub(super) static SPEC: OptionSpec = OptionSpec {
    key: KIND,
    title: || option_form::tool_title(TOOL),
    keys: &KEYS,
    colors: &["highlightColor"],
    preview: false,
    body,
    inactive: |values, key| match key {
        "gapLength" => !is_on(values, "gapDetection"),
        "highlightColor" => !is_on(values, "highlightStroke"),
        // Cursor Swatch Preview can be changed with Color Swatches only.
        "cursorSwatchPreview" => values.get("pickColorFrom").and_then(Value::as_str) != Some("swatches"),
        _ => false,
    },
    run: |app, values, _| option_form::set_tool_options(app, TOOL, values),
    cancelled: None,
};

/// Open the Shape Builder's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    option_form::open_tool(app, &SPEC, TOOL).unwrap_or_else(|e| json!({ "error": e }))
}

fn body(ui: &mut egui::Ui, rows: &mut Rows<'_>) {
    rows.check(ui, "gapDetection", tl!("Gap Detection"), true);
    let detection = rows.flag("gapDetection");
    let length = rows.float("gapLength", GAP_LENGTHS[0]);
    // The preset the length is, else Custom, the entry after the presets.
    let shown = GAP_LENGTHS.iter().position(|n| *n == length).unwrap_or(GAP_LENGTHS.len());
    rows.field(ui, tl!("Gap Length"), detection, |ui, rows, label| {
        Row::new().show(ui, |ui, _| {
            if let Some(at) = pick(ui, label, shown, &[tl!("Small"), tl!("Medium"), tl!("Large"), tl!("Custom")]) {
                rows.set("gapLength", json!(GAP_LENGTHS.get(at).copied().unwrap_or(CUSTOM_GAP)));
            }
            rows.number(ui, "gapLength", tl!("Custom"), 0.0..=SHAPE_BUILDER_MAX_GAP, UnitKind::Points, shown == GAP_LENGTHS.len());
        });
    });
    rows.check(ui, "openFilledAsClosed", tl!("Consider Open Filled Paths as Closed"), true);
    rows.check(ui, "clickStrokeSplitsPath", tl!("In Merge Mode, Clicking Stroke Splits the Path"), true);
    rows.field(ui, tl!("Pick Color From"), true, |ui, rows, label| {
        rows.select(ui, "pickColorFrom", label, &[("artwork", tl!("Artwork")), ("swatches", tl!("Color Swatches"))]);
    });
    let swatches = rows.text("pickColorFrom") == "swatches";
    rows.check(ui, "cursorSwatchPreview", tl!("Cursor Swatch Preview"), swatches);
    rows.field(ui, tl!("Selection"), true, |ui, rows, label| {
        rows.radios(ui, "selection", label, &[("freeform", tl!("Freeform")), ("straightLine", tl!("Straight Line"))]);
    });
    rows.field(ui, tl!("Highlight"), true, |ui, rows, _| {
        Row::new().show(ui, |ui, _| {
            rows.check(ui, "highlightFill", tl!("Fill"), true);
            rows.check(ui, "highlightStroke", tl!("Stroke When Editable"), true);
        });
    });
    // The stroke highlight takes a layer color, or any other color from Select Custom Color….
    let stroke = rows.flag("highlightStroke");
    rows.field(ui, tl!("Color"), stroke, |ui, rows, label| rows.color(ui, "highlightColor", label, stroke));
}

#[cfg(test)]
mod tests {
    use super::super::option_form::tests::{app, press_ok, set};
    use super::*;

    #[test]
    fn gap_options_share_the_existing_dialog_and_custom_is_capped() {
        let mut a = app();
        crate::toolbar::open_options(&mut a, "shapeBuilder").unwrap();
        set(&a, "gapDetection", json!(true)).unwrap();
        set(&a, "gapLength", json!(1000)).unwrap();
        assert!(press_ok(&mut a));
        let opts = a.session.tool_options_of("shapeBuilder");
        assert_eq!(opts["gapLength"], SHAPE_BUILDER_MAX_GAP);
        assert_eq!(opts["gapDetection"], true);
        assert_eq!(opts["openFilledAsClosed"], true);
    }

    /// Double-clicking the Fill Area tool opens the Shape Builder Tool Options, whose Gap
    /// Detection the active Fill Area tool takes on OK.
    #[test]
    fn the_fill_area_tool_opens_the_shape_builder_options() {
        let mut a = app();
        a.select_tool("fillArea");
        let r = crate::toolbar::open_options(&mut a, "fillArea").unwrap();
        assert_eq!(r["dialog"], json!(KIND));
        set(&a, "gapDetection", json!(true)).unwrap();
        set(&a, "gapLength", json!(6)).unwrap();
        assert!(press_ok(&mut a));
        let opts = a.session.tool_options_of("fillArea");
        assert_eq!((opts["gapDetection"].clone(), opts["gapLength"].as_f64()), (json!(true), Some(6.0)));
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
