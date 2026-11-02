//! Arrange Tools in the interface: its submenu (Object › Plug-ins › Arrange Tools); the dialogs of
//! Arrange in Grid, Replace with Top Object, Set Size of Each, Randomize Opacity and Sort Objects,
//! which preview on the canvas and run their `addon.arrange.*` command on OK; Group Overlapping
//! Objects; and Zoom to Selection ([`ZOOM`]), which fits the active view to the selection.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::menus::Item;
use crate::state::Dialog;
use crate::widgets;

/// Zoom to Selection: a view command, since the view is the interface's.
pub const ZOOM: &str = "addon.arrange.zoomSelection";
/// The dialogs' kinds.
pub const GRID: &str = "arrangeGrid";
pub const REPLACE: &str = "arrangeReplace";
pub const SIZE: &str = "arrangeSize";
pub const OPACITY: &str = "arrangeOpacity";
pub const SORT: &str = "arrangeSort";
/// Each dialog's kind, command and heading.
const DIALOGS: [(&str, &str, &str); 5] = [
    (GRID, "addon.arrange.grid", "Arrange in Grid"),
    (REPLACE, "addon.arrange.replace", "Replace with Top Object"),
    (SIZE, "addon.arrange.size", "Set Size of Each"),
    (OPACITY, "addon.arrange.randomizeOpacity", "Randomize Opacity"),
    (SORT, "addon.arrange.sort", "Sort Objects"),
];
/// The widths and heights Set Size of Each takes (pt), and the gaps Arrange in Grid takes.
const SIZES: std::ops::RangeInclusive<f64> = 0.001..=1e6;
const GAPS: std::ops::RangeInclusive<f64> = -1e6..=1e6;
/// The seeds the Randomize Opacity dialog offers.
const SEEDS: u64 = 1_000_000;
const LABEL_W: f32 = 110.0;
const FIELD_W: f32 = 90.0;

/// Object › Plug-ins › Arrange Tools.
pub fn menu() -> Vec<Item> {
    vec![
        Item::Cmd("Arrange in Grid…", "addon.arrange.grid", Value::Null),
        Item::Cmd("Swap Objects", "addon.arrange.swap", json!({})),
        Item::Cmd("Swap Objects and Sizes", "addon.arrange.swap", json!({ "size": true })),
        Item::Cmd("Replace with Top Object…", "addon.arrange.replace", Value::Null),
        Item::Cmd("Set Size of Each…", "addon.arrange.size", Value::Null),
        Item::Cmd("Group Overlapping Objects", "addon.arrange.groupOverlapping", json!({})),
        Item::Cmd("Randomize Opacity…", "addon.arrange.randomizeOpacity", Value::Null),
        Item::Cmd("Sort Objects…", "addon.arrange.sort", Value::Null),
        Item::Sep,
        Item::Cmd("Zoom to Selection", ZOOM, json!({})),
    ]
}

/// The dialog (kind, fields) the menu item of command `id` opens; `None` for other commands and
/// without a document.
pub(crate) fn dialog(app: &VectorcraftApp, id: &str) -> Option<(&'static str, Value)> {
    let &(kind, ..) = DIALOGS.iter().find(|(_, cmd, _)| *cmd == id)?;
    let st = app.session.active()?;
    let strokes = app.session.prefs.scale_strokes;
    let fields = match kind {
        // A square grid to start with: as many columns as the square root of the count.
        GRID => json!({
            "layout": "grid", "columns": (st.selection.objects.len().max(1) as f64).sqrt().ceil() as u64,
            "gapX": 10, "gapY": 10, "centerOnArtboard": false,
        }),
        REPLACE => json!({ "fit": "keep", "keep": false, "scaleStrokes": strokes }),
        // A new seed each time the document has changed, so running it again varies the result.
        OPACITY => json!({ "min": 30, "max": 100, "seed": st.revision % SEEDS }),
        // The random order's seed (not shown): new whenever the document has changed, so each run
        // shuffles anew while Preview and OK agree.
        SORT => json!({
            "by": "area", "apply": "stacking", "order": "ascending", "distribute": false, "seed": st.revision % SEEDS, "scaleStrokes": strokes,
        }),
        // The first selected object's size, as the command measures it (a side it lacks: the least).
        _ => {
            let b = st.selection.objects.first().and_then(|id| app.session.transform_box(&[*id])).map_or((100.0, 100.0), |b| {
                (b.rect.width().clamp(*SIZES.start(), *SIZES.end()), b.rect.height().clamp(*SIZES.start(), *SIZES.end()))
            });
            json!({
                "width": b.0, "height": b.1, "__width": true, "__height": true, "proportional": false, "reference": 4, "scaleStrokes": strokes,
            })
        }
    };
    Some((kind, fields))
}

pub(crate) const SPEC: DialogSpec = DialogSpec {
    heading: |d| tl!(command(d).1).to_string(),
    body,
    confirm,
    preview: true,
    min_width: 320.0,
    ok_enabled: Some(|_, d| match d.kind.as_str() {
        SIZE => d.bool("__width") || d.bool("__height"),
        OPACITY => d.f64("min", 0.0) <= d.f64("max", 100.0),
        _ => true,
    }),
    ..DialogSpec::FORM
};

/// The command and heading of dialog `d`.
fn command(d: &Dialog) -> (&'static str, &'static str) {
    DIALOGS.iter().find(|(kind, ..)| *kind == d.kind).map_or(("", ""), |&(_, cmd, label)| (cmd, label))
}

/// The command's params: the fields, and of Set Size of Each's width and height those ticked.
fn params(d: &Dialog) -> Value {
    let mut p = form::params(d);
    if let Some(o) = p.as_object_mut() {
        for side in ["width", "height"] {
            if !d.bool(&format!("__{side}")) {
                o.remove(side);
            }
        }
    }
    p
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let widths = (LABEL_W, 150.0);
    match d.kind.as_str() {
        GRID => {
            let layouts = [("grid", "Grid by Row"), ("row", "Arrange by Row"), ("column", "Arrange by Column")];
            form::choice(ui, d, "layout", tl!("Layout:"), widths, &layouts);
            ui.add_enabled_ui(d.str("layout") == "grid", |ui| {
                widgets::label_row(ui, tl!("Columns:"), LABEL_W, |ui| {
                    if let Some(v) = widgets::range_field(ui, ("arrange", "columns"), d.f64("columns", 1.0), 1.0..=10_000.0, "", 0, FIELD_W) {
                        d.fields.insert("columns".into(), json!(v as u64));
                    }
                });
            });
            widgets::label_row(ui, tl!("H Spacing:"), LABEL_W, |ui| {
                form::length_within(ui, d, "gapX", unit, FIELD_W, Some(GAPS));
            });
            widgets::label_row(ui, tl!("V Spacing:"), LABEL_W, |ui| {
                form::length_within(ui, d, "gapY", unit, FIELD_W, Some(GAPS));
            });
            form::check(ui, d, "centerOnArtboard", tl!("Center on Artboard"));
        }
        REPLACE => {
            let fits = [("keep", "Actual Size"), ("bounds", "Bounds"), ("proportional", "Bounds (Fit Proportionally)")];
            form::choice(ui, d, "fit", tl!("Size:"), widths, &fits);
            form::check(ui, d, "keep", tl!("Keep Original"));
            form::check(ui, d, "scaleStrokes", tl!("Scale Strokes & Effects"));
        }
        OPACITY => {
            for (key, label) in [("min", tl!("Minimum:")), ("max", tl!("Maximum:"))] {
                widgets::label_row(ui, label, LABEL_W, |ui| {
                    if let Some(v) = widgets::range_field(ui, ("arrange", key), d.f64(key, 0.0), 0.0..=100.0, "%", 0, FIELD_W) {
                        d.fields.insert(key.into(), json!(v));
                    }
                });
            }
            widgets::label_row(ui, tl!("Seed:"), LABEL_W, |ui| {
                if let Some(v) = widgets::range_field(ui, ("arrange", "seed"), d.f64("seed", 0.0), 0.0..=(SEEDS - 1) as f64, "", 0, FIELD_W) {
                    d.fields.insert("seed".into(), json!(v as u64));
                }
            });
        }
        SORT => {
            let attributes = [
                ("width", "Width"),
                ("height", "Height"),
                ("area", "Area"),
                ("x", "X Position"),
                ("y", "Y Position"),
                ("opacity", "Opacity"),
                ("stacking", "Stacking Order"),
            ];
            form::choice(ui, d, "by", tl!("Sort By:"), widths, &attributes);
            form::choice(ui, d, "apply", tl!("Apply To:"), widths, &attributes);
            let orders = [("ascending", "Small to Large"), ("descending", "Large to Small"), ("random", "Random")];
            form::choice(ui, d, "order", tl!("Order:"), widths, &orders);
            // Stacking order only trades places; only sizes scale strokes.
            let apply = d.str("apply");
            ui.add_enabled_ui(apply != "stacking", |ui| form::check(ui, d, "distribute", tl!("Distribute Evenly")));
            ui.add_enabled_ui(matches!(apply.as_str(), "width" | "height" | "area"), |ui| {
                form::check(ui, d, "scaleStrokes", tl!("Scale Strokes & Effects"));
            });
        }
        _ => {
            egui::Grid::new("arrange-size").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                for (side, tick, label) in [("width", "__width", tl!("Width:")), ("height", "__height", tl!("Height:"))] {
                    form::check(ui, d, tick, label);
                    let on = d.bool(tick);
                    ui.add_enabled_ui(on, |ui| {
                        form::length_within(ui, d, side, unit, FIELD_W, Some(SIZES));
                    });
                    ui.end_row();
                }
            });
            // Only one size given: the other can follow it.
            let one = d.bool("__width") != d.bool("__height");
            if widgets::check(ui, tl!("Constrain Width and Height Proportions"), d.bool("proportional"), one) {
                let on = !d.bool("proportional");
                d.fields.insert("proportional".into(), json!(on));
            }
            ui.horizontal(|ui| {
                if let Some(i) = widgets::reference_point(ui, d.f64("reference", 4.0).clamp(0.0, 8.0) as usize) {
                    d.fields.insert("reference".into(), json!(i));
                }
                widgets::dim_label(ui, tl!("Reference Point"));
            });
            form::check(ui, d, "scaleStrokes", tl!("Scale Strokes & Effects"));
        }
    }
    let (cmd, label) = command(d);
    form::preview(app, ui, d, label, cmd, params(d));
    false
}

/// OK: run the command (keep its preview) and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, command(d).0, params(d))
}

/// Zoom to Selection: the active view zooms and centres so the selection's bounds fill the
/// window, with a margin. An error with nothing selected.
pub(crate) fn zoom(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.active().is_none_or(|st| st.selection.objects.is_empty()) {
        return Err("nothing selected".into());
    }
    crate::canvas::fit(app, ZOOM);
    Ok(Value::Null)
}

#[cfg(test)]
mod tests;
