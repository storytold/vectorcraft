//! Path Precision Tools in the interface: its submenu (Object › Plug-ins › Path Precision Tools),
//! which selects its tools, opens the Anchor Inspector and runs Split / Add Points at Crossings;
//! the Anchor Inspector panel (the selected anchor's position and handles as numbers, and the
//! add-on's anchor edits); and its tools' options dialog (a double-click on the tool). Everything
//! acts through `addon.pathtools.*` and `tool.setOption`.

use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_geom::Point;
use vectorcraft_tools::addons::{anchor_eraser, extend_path, measure_inspect};

use crate::VectorcraftApp;
use crate::dialogs::DialogSpec;
use crate::menus::Item;
use crate::panels::{empty_state, pstate, set_pstate};
use crate::state::Dialog;
use crate::widgets::{self, dim_label};

/// The Anchor Inspector panel's id (`window.panel {panel: "anchorInspector"}`).
pub const ID: &str = "anchorInspector";
/// Where the panel keeps the count Add Points adds (panel state).
const COUNT: &str = "anchor-inspector-count";
/// The tools' options dialog.
pub const KIND: &str = "pathToolOptions";
/// The close modes, as `addon.pathtools.close` names them, and their labels.
const CLOSE_MODES: [(&str, &str); 3] = [("handles", "Keep Handles"), ("straight", "Straight"), ("smooth", "Smooth")];
const LABEL_W: f32 = 64.0;

/// Object › Plug-ins › Path Precision Tools.
pub fn menu() -> Vec<Item> {
    let tool = |label, id: &str| Item::Cmd(label, "tool.select", json!({ "tool": id }));
    vec![
        tool("Anchor Eraser Brush", anchor_eraser::ID),
        tool("Extend Path Tool", extend_path::ID),
        tool("Measure & Inspect Tool", measure_inspect::ID),
        Item::Sep,
        Item::Cmd("Anchor Inspector", "window.panel", json!({ "panel": ID })),
        Item::Sep,
        Item::Cmd("Add Points at Crossings", "addon.pathtools.crossings", json!({})),
        Item::Cmd("Split at Crossings", "addon.pathtools.crossings", json!({ "cut": true })),
    ]
}

/// The first direct-selected anchor (path, subpath, anchor) and how many are selected.
fn first_anchor(app: &VectorcraftApp) -> Option<(vectorcraft_geom::Anchor, usize)> {
    let st = app.session.active()?;
    let count = st.selection.anchors.values().map(|a| a.len()).sum();
    let (id, set) = st.selection.anchors.iter().find(|(_, a)| !a.is_empty())?;
    let (si, ai) = set.first()?;
    let a = st.doc.node(*id)?.path_data()?.subpaths.get(*si)?.anchors.get(*ai)?;
    Some((*a, count))
}

/// A handle's (length, direction in degrees counter-clockwise on the page); `None` for none.
fn polar(p: Point, h: Point) -> Option<(f64, f64)> {
    let v = h - p;
    (v.hypot() > 1e-9).then(|| (v.hypot(), (-v.y).atan2(v.x).to_degrees() + 0.0))
}

fn run(app: &mut VectorcraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.status(e);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        empty_state(ui, "dc-anchor-smooth", tl!("No document"), tl!("Open a document to edit its anchor points."));
        return;
    };
    let selected = !st.selection.is_empty();
    let unit = app.session.general_unit();
    let origin = app.ruler_origin();
    let anchor = first_anchor(app);
    let mut set: Option<Value> = None;
    match anchor {
        Some((_, n)) => dim_label(ui, &crate::i18n::fmt(tl!("Anchors selected: {count}"), &[("count", &n.to_string())])),
        None => dim_label(ui, tl!("Direct-select anchor points to edit them.")),
    };
    ui.add_enabled_ui(anchor.is_some(), |ui| {
        let a = anchor.map(|(a, _)| a);
        egui::Grid::new("anchor-inspector-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            for (label, key, v) in [("X:", "x", a.map(|a| a.p.x - origin.x)), ("Y:", "y", a.map(|a| a.p.y - origin.y))] {
                dim_label(ui, label);
                if let Some(v) = widgets::num_field_named(ui, ("anchor-inspector", key), Some(tl!(label)), v, unit, 72.0) {
                    let o = if key == "x" { origin.x } else { origin.y };
                    set = Some(json!({ key: v + o }));
                }
            }
            ui.end_row();
            for (name, out) in [("In:", false), ("Out:", true)] {
                let h = a.and_then(|a| polar(a.p, if out { a.h_out } else { a.h_in }));
                let side = if out { "out" } else { "in" };
                dim_label(ui, name);
                let len = h.map(|h| h.0).or(a.map(|_| 0.0));
                if let Some(v) = widgets::size_field_named(ui, ("anchor-inspector-len", side), Some(tl!("Length")), len, unit, 72.0) {
                    set = Some(json!({ format!("{side}Length"): v }));
                }
                dim_label(ui, "∠");
                if let Some(v) = widgets::mixed_field_named(ui, ("anchor-inspector-angle", side), Some(tl!("Angle")), h.map(|h| h.1), "°", 1, 56.0) {
                    set = Some(json!({ format!("{side}Angle"): v }));
                }
                ui.end_row();
            }
        });
    });
    widgets::divider(ui);
    let width = ui.available_width();
    let mut action: Option<(&str, Value)> = None;
    ui.add_enabled_ui(selected, |ui| {
        for (label, id) in [
            ("Make Smooth", "addon.pathtools.smooth"),
            ("Make Corner", "addon.pathtools.corner"),
            ("Equalize Handles", "addon.pathtools.equalize"),
            ("Remove Points, Keep Shape", "addon.pathtools.removeKeepShape"),
        ] {
            if widgets::flat_button(ui, tl!(label), width).clicked() {
                action = Some((id, json!({})));
            }
        }
        let mut count: f64 = pstate::<Option<f64>>(ui.ctx(), COUNT).unwrap_or(1.0);
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, tl!("Add Points"), width - 70.0).clicked() {
                action = Some(("addon.pathtools.addPoints", json!({ "count": count })));
            }
            if let Some(v) =
                widgets::range_field_named(ui, "anchor-inspector-count", Some(tl!("Points per segment")), count, 1.0..=100.0, "", 0, 60.0)
            {
                count = v;
            }
        });
        set_pstate(ui.ctx(), COUNT, Some(count));
        // The label above and the three ways sharing the row: beside the label they overflow a narrow dock.
        widgets::subheader(ui, "Close Path:");
        ui.horizontal(|ui| {
            let n = CLOSE_MODES.len() as f32;
            let each = ((width - ui.spacing().item_spacing.x * (n - 1.0)) / n).max(40.0);
            for (mode, label) in CLOSE_MODES {
                if widgets::flat_button(ui, tl!(label), each).clicked() {
                    action = Some(("addon.pathtools.close", json!({ "mode": mode })));
                }
            }
        });
    });
    if let Some(p) = set {
        run(app, "addon.pathtools.setAnchor", p);
    }
    if let Some((id, p)) = action {
        run(app, id, p);
    }
}

// ---------- the tools' options ----------

/// Tools whose options the dialog edits.
pub fn has_options(tool: &str) -> bool {
    [anchor_eraser::ID, extend_path::ID].contains(&tool)
}

/// Open the options of add-on tool `tool` with its current values.
pub fn open_options(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    let mut fields: Map<String, Value> = app.session.tool_options_of(tool).as_object().cloned().unwrap_or_default();
    fields.insert("__tool".into(), json!(tool));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({ "dialog": KIND }))
}

pub(crate) const OPTIONS_SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 280.0, ..DialogSpec::FORM };

fn heading(d: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(&d.str("__tool")).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

fn body(_app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    if d.str("__tool") == extend_path::ID {
        let modes = [("straight", "Straight"), ("bezier", "Stretch Curve"), ("arc", "Arc")];
        let labels: Vec<&str> = modes.iter().map(|(_, l)| *l).collect();
        let current = modes.iter().find(|(m, _)| *m == d.str("mode")).map_or("Straight", |(_, l)| *l);
        widgets::label_row(ui, tl!("Mode:"), LABEL_W, |ui| {
            if let Some((m, _)) = widgets::dropdown(ui, "extend-path-mode", current, &labels, 140.0).and_then(|i| modes.get(i)) {
                d.fields.insert("mode".into(), json!(m));
            }
        });
        return false;
    }
    let rows = [
        ("size", "Size:", anchor_eraser::SIZE_RANGE, "px", anchor_eraser::SIZE),
        ("tolerance", "Tolerance:", anchor_eraser::TOLERANCE_RANGE, "", anchor_eraser::TOLERANCE),
    ];
    for (key, label, (lo, hi), suffix, default) in rows {
        let v = d.f64(key, default);
        widgets::label_row(ui, tl!(label), LABEL_W, |ui| {
            if let Some(x) = widgets::range_field(ui, ("anchor-eraser", key), v, lo..=hi, suffix, 0, 72.0) {
                d.fields.insert(key.into(), json!(x));
            }
        });
    }
    if widgets::check(ui, "Pressure", d.bool("pressure"), true) {
        let on = !d.bool("pressure");
        d.fields.insert("pressure".into(), json!(on));
    }
    false
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let tool = d.str("__tool");
    let values: Map<String, Value> = d.fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    let r = app.run("tool.setOption", json!({ "tool": tool, "values": values }));
    app.ui.dialog = None;
    r
}

#[cfg(test)]
mod tests;
