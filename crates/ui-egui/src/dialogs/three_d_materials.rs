//! The 3D and Materials panel's Materials tab: the document's Graphics (its symbols) as thumbnails,
//! Add as Single Graphic, and the graphics mapped onto the selected 3D object's planar faces with
//! their face, position, size and turn. Picking a thumbnail maps that graphic onto the selected
//! objects (`threeD.mapGraphic`); art dropped on the thumbnails becomes a graphic
//! (`threeD.addGraphic`). Editing a mapped graphic changes the effect's `graphics` parameter, which
//! the panel sends as it sends every other setting.

use egui::{Sense, Stroke, StrokeKind, vec2};
use serde_json::{Value, json};
use vectorcraft_effects::{GRAPHICS, MappedGraphic, PlanarFace, ROTATE};

use super::revolve::{heading, label, number};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets::{self, dim_label};

/// The field of [`Dialog::fields`] that holds what the panel runs after drawing the tab
/// ([`Action`]), as JSON. It is UI state and never enters the effect's parameters.
pub(crate) const ACTION: &str = "__graphicAction";
/// The field that holds the index of the mapped graphic the tab edits.
pub(crate) const SELECTED: &str = "__graphic";

/// What a click in the Graphics list asks the panel to do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// Map the graphic of this symbol onto the selected 3D objects.
    Map(String),
    /// Keep art as a new graphic: the objects dropped on the list, or the selection (`None`).
    Add(Option<Vec<u64>>),
}

impl Action {
    pub(crate) fn to_value(&self) -> Value {
        match self {
            Action::Map(name) => json!({"map": name}),
            Action::Add(ids) => json!({"add": ids}),
        }
    }

    pub(crate) fn from_value(v: &Value) -> Option<Self> {
        if let Some(name) = v.get("map").and_then(Value::as_str) {
            return Some(Action::Map(name.to_string()));
        }
        let ids = v.get("add")?;
        Some(Action::Add(ids.as_array().map(|a| a.iter().filter_map(Value::as_u64).collect())))
    }
}

/// The document's graphics as thumbnails, with Add as Single Graphic below them. A thumbnail
/// clicked while `apply` (a 3D object is selected) maps that graphic. Returns the action asked for.
pub(crate) fn library(app: &VectorcraftApp, ui: &mut egui::Ui, apply: bool) -> Option<Action> {
    heading(ui, tl!("Graphics"));
    let st = app.session.active()?;
    let names: Vec<String> = st.doc.symbols.iter().map(|s| s.name.clone()).collect();
    let keys = crate::panels::symbols::document_symbols(st);
    let t = Tokens::get(ui.ctx());
    let mut action = None;
    let area = ui
        .horizontal_wrapped(|ui| {
            for name in &names {
                let (r, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
                crate::panels::symbols::chip(ui, &st.doc, &keys, r.shrink(2.0), name);
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
                let tip = if apply { name.clone() } else { format!("{name}: {}", tl!("Select a 3D object to map the graphic onto it.")) };
                if resp.on_hover_text(tip).clicked() && apply {
                    action = Some(Action::Map(name.clone()));
                }
            }
            if names.is_empty() {
                dim_label(ui, tl!("Add art with Add as Single Graphic, or drop art here."));
            }
        })
        .response
        .rect;
    let zone = ui.interact(area, ui.scope_id().with("three-d-graphics-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone) {
        action = Some(Action::Add(Some(ids)));
    }
    let selected = !st.selection.objects.is_empty();
    if ui.add_enabled(selected, egui::Button::new(tl!("Add as Single Graphic"))).clicked() {
        action = Some(Action::Add(None));
    }
    action
}

/// The Materials tab of the 3D effect in `d.fields["__effect"]`: the Graphics list, then the
/// graphics mapped onto the object, the selected one with its settings. Edits `d.fields`; a click
/// in the list is left in [`ACTION`].
pub(crate) fn controls(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let effect = d.str("__effect");
    let takes = vectorcraft_effects::takes_graphics(&effect);
    if let Some(action) = library(app, ui, takes) {
        d.fields.insert(ACTION.into(), action.to_value());
    }
    ui.add_space(10.0);
    if !takes {
        dim_label(ui, tl!("Graphics map onto planar faces: pick the Plane or Extrude object type."));
        return;
    }
    let raw = d.fields.get(GRAPHICS).cloned().unwrap_or(Value::Null);
    if raw.is_null() && d.fields.contains_key(GRAPHICS) {
        dim_label(ui, tl!("The selected objects have different graphics."));
        return;
    }
    let mut list: Vec<MappedGraphic> = vectorcraft_effects::mapped_graphics(&json!({ GRAPHICS: raw }));
    heading(ui, tl!("Mapped graphics"));
    if list.is_empty() {
        dim_label(ui, tl!("Pick a graphic above to map it onto the object."));
        return;
    }
    let mut index = d.fields.get(SELECTED).and_then(Value::as_u64).map_or(0, |i| i as usize).min(list.len() - 1);
    for (i, g) in list.iter().enumerate() {
        let face = if g.face == PlanarFace::Back { tl!("Back") } else { tl!("Front") };
        if ui.selectable_label(i == index, format!("{} · {face}", g.symbol)).clicked() {
            index = i;
        }
    }
    d.fields.insert(SELECTED.into(), json!(index));
    ui.add_space(6.0);
    let Some(g) = list.get(index).cloned() else { return };
    let mut edited = g.clone();
    if effect != ROTATE {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            label(ui, tl!("Face"));
            let labels = [tl!("Front"), tl!("Back")];
            let at = usize::from(g.face == PlanarFace::Back);
            if let Some(k) = widgets::dropdown(ui, "three-d-graphic-face", labels[at], &labels, 120.0) {
                edited.face = PlanarFace::ALL[k.min(1)];
            }
        });
    }
    let unit = app.session.general_unit();
    let mut fields = Dialog { kind: "threeDGraphic".into(), fields: Default::default() };
    fields.fields.insert("__defaults".into(), json!({"graphicX": 0.0, "graphicY": 0.0, "graphicScale": 100.0, "graphicRotation": 0.0}));
    fields.fields.insert("graphicX".into(), json!(g.x));
    fields.fields.insert("graphicY".into(), json!(g.y));
    fields.fields.insert("graphicScale".into(), json!(g.scale));
    fields.fields.insert("graphicRotation".into(), json!(g.rotation));
    // The ranges the effect takes, so every stored value shows as it is.
    let offsets = (-vectorcraft_effects::GRAPHIC_MAX_OFFSET, vectorcraft_effects::GRAPHIC_MAX_OFFSET);
    number(ui, &mut fields, "graphicX", "X", offsets, "", 1, Some(unit));
    number(ui, &mut fields, "graphicY", "Y", offsets, "", 1, Some(unit));
    number(ui, &mut fields, "graphicScale", tl!("Scale"), vectorcraft_effects::GRAPHIC_SCALE_RANGE, "%", 1, None);
    number(ui, &mut fields, "graphicRotation", tl!("Rotation"), (-360.0, 360.0), "°", 1, None);
    edited.x = fields.f64("graphicX", g.x);
    edited.y = fields.f64("graphicY", g.y);
    edited.scale = fields.f64("graphicScale", g.scale);
    edited.rotation = fields.f64("graphicRotation", g.rotation);
    let remove = ui.button(tl!("Remove Graphic")).clicked();
    if remove {
        list.remove(index);
        d.fields.insert(SELECTED.into(), json!(index.saturating_sub(1)));
    } else if edited != g {
        list[index] = edited;
    } else {
        return;
    }
    d.fields.insert(GRAPHICS.into(), Value::Array(list.iter().map(MappedGraphic::to_value).collect()));
}
