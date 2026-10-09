//! Pathfinder panel: Shape Modes (+ Expand) and Pathfinders, with icons drawn for VectorCraft.
//! Alt-click a Shape Mode for a live compound shape (`object.compoundShape.make`); with members of
//! a compound shape selected, a Shape Mode sets their mode.

use egui::Ui;
use serde_json::json;

use super::{alt_held, pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

pub const SHAPE_MODES: [(&str, &str, &str); 4] = [
    ("dc-pf-unite", "Unite", "unite"),
    ("dc-pf-minus-front", "Minus Front", "minusFront"),
    ("dc-pf-intersect", "Intersect", "intersect"),
    ("dc-pf-exclude", "Exclude", "exclude"),
];

/// The compound-shape mode (`object.compoundShape.*` `mode`) a Shape Mode stands for.
pub fn shape_mode_of(op: &str) -> &'static str {
    match op {
        "minusFront" => "subtract",
        "intersect" => "intersect",
        "exclude" => "exclude",
        _ => "add",
    }
}

/// Are all the selected objects members of a compound shape (a Shape Mode sets their mode)?
pub fn members_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|d| {
        !d.selection.objects.is_empty()
            && d.selection.objects.iter().all(|id| {
                d.doc.parent_of(*id).and_then(|p| d.doc.node(p)).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::CompoundShape { .. }))
            })
    })
}

/// A Shape Mode clicked: with Alt (or on compound-shape members), live; else destructive.
pub fn run_shape_mode(app: &mut VectorcraftApp, ui: &Ui, op: &str, label: &str) {
    if alt_held(ui) {
        app.run("object.compoundShape.make", json!({ "mode": shape_mode_of(op) })).ok();
    } else {
        run(app, ui, op, label);
    }
}

pub const PATHFINDERS: [(&str, &str, &str); 6] = [
    ("dc-pf-divide", "Divide", "divide"),
    ("dc-pf-trim", "Trim", "trim"),
    ("dc-pf-merge", "Merge", "merge"),
    ("dc-pf-crop", "Crop", "crop"),
    ("dc-pf-outline", "Outline", "outline"),
    ("dc-pf-minus-back", "Minus Back", "minusBack"),
];

fn run(app: &mut VectorcraftApp, ui: &Ui, op: &str, label: &str) {
    if app.run(&format!("object.pathfinder.{op}"), json!({})).is_ok() {
        set_pstate(ui.ctx(), "pf-last", Some((op.to_string(), label.to_string())));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = selection_len(app);
    widgets::subheader(ui, tl!("Shape Modes:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let members = members_selected(app);
        for (icon, tip, op) in SHAPE_MODES {
            if widgets::icon_button_enabled(ui, icon, tip, false, n >= 2 || members, 34.0).clicked() {
                run_shape_mode(app, ui, op, tip);
            }
        }
        let expand = crate::menus::enabled(app, "object.compoundShape.expand");
        let width = ui.available_width().min(90.0);
        let r = ui
            .add_enabled_ui(expand, |ui| widgets::flat_button(ui, tl!("Expand"), width))
            .inner
            .on_hover_text(tl!("Expand the compound shape into a path"))
            .on_disabled_hover_text(tl!("Expand applies to compound shapes (Alt-click a shape mode)"));
        if r.clicked() {
            app.run("object.compoundShape.expand", json!({})).ok();
        }
    });
    widgets::subheader(ui, tl!("Pathfinders:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (icon, tip, op) in PATHFINDERS {
            let min = if op == "outline" || op == "divide" { 1 } else { 2 };
            if widgets::icon_button_enabled(ui, icon, tip, false, n >= min, 34.0).clicked() {
                run(app, ui, op, tip);
            }
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    menu_item(ui, tl!("Trap…"), false, false);
    let last: Option<(String, String)> = pstate(ui.ctx(), "pf-last");
    let label =
        last.as_ref().map(|(_, l)| crate::i18n::fmt(tl!("Repeat {name}"), &[("name", tl!(l))])).unwrap_or_else(|| tl!("Repeat Pathfinder").into());
    if menu_item(ui, &label, last.is_some() && selection_len(app) >= 1, false)
        && let Some((op, l)) = last
    {
        run(app, ui, &op, &l);
    }
    menu_item(ui, tl!("Pathfinder Options…"), false, false);
    ui.separator();
    for (label, id) in [
        (tl!("Make Compound Shape"), "object.compoundShape.make"),
        (tl!("Release Compound Shape"), "object.compoundShape.release"),
        (tl!("Expand Compound Shape"), "object.compoundShape.expand"),
    ] {
        let on = crate::menus::enabled(app, id) && (id != "object.compoundShape.make" || selection_len(app) >= 2);
        if menu_item(ui, label, on, false) {
            app.run(id, json!({})).ok();
        }
    }
}
