//! The 3D and Materials panel's Materials page: the document's Graphics (its symbols) as thumbnails,
//! Add as Single Graphic, and the graphics mapped onto the selected 3D object (its planar faces,
//! Extrude & Bevel's side, Revolve's side, or Inflate's puffed faces) with their face, position, size and turn. Picking a
//! thumbnail maps that graphic
//! onto the selected objects (`threeD.mapGraphic`); art dropped on the thumbnails becomes a graphic
//! (`threeD.addGraphic`). Editing a mapped graphic changes the effect's `graphics` parameter, which
//! the panel sends as it sends every other setting.

use std::borrow::Cow;
use std::sync::{Arc, RwLock};

use craft::behavior::keyed;
use craft::ui::buttons::Button;
use craft::ui::contract::{ActivateOn, Bind, CollectionEvent, Props as _, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, Param, ParamRows};
use craft::ui::sections::DropAction;
use craft::ui::tile_grid::{Tile, TileGrid};
use egui::{Response, Ui};
use serde_json::{Value, json};
use vectorcraft_effects::{GRAPHICS, INFLATE, MappedGraphic, PlanarFace, REVOLVE, ROTATE};

use super::three_d::Data;
use crate::panel_drag::PanelDrag;

/// The field that holds what the panel runs after drawing the page ([`Action`]), as JSON. It is UI
/// state and never enters the effect's parameters.
pub(crate) const ACTION: &str = "__graphicAction";
/// The field that holds the index of the mapped graphic the page edits.
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

/// The name of `face` in the interface.
fn face_label(face: PlanarFace) -> &'static str {
    match face {
        PlanarFace::Front => tl!("Front"),
        PlanarFace::Back => tl!("Back"),
        PlanarFace::Side => tl!("Side"),
    }
}

/// The JSON a custom row's `key` holds in `fields`.
fn json_of(fields: &DialogFields, key: &str) -> Option<Value> {
    match fields.get(key)? {
        FieldValue::Json(v) => Some(v.clone()),
        _ => None,
    }
}

/// The Materials page as a full-width row: the Graphics, Add as Single Graphic, then the graphics
/// mapped onto the object, the selected one with its settings. It writes the mapped list into
/// [`GRAPHICS`], the selected index into [`SELECTED`] and a click in the Graphics into [`ACTION`].
/// With no 3D effect (`data.effect` is `None`) only the Graphics show, and a thumbnail maps nothing.
pub(crate) fn row(data: Arc<RwLock<Data>>) -> Param {
    Param::custom(GRAPHICS, tl!("Materials"), move |ui, fields, cx| draw(ui, fields, cx, &data)).full_width()
}

fn draw(ui: &mut Ui, fields: &mut DialogFields, cx: &mut CustomContext, data: &RwLock<Data>) -> Response {
    let out = Stack::new().show(ui, |ui, _| {
        if let Ok(data) = data.read() {
            body(ui, fields, cx, &data);
        }
    });
    out.output
}

fn body(ui: &mut Ui, fields: &mut DialogFields, cx: &mut CustomContext, data: &Data) {
    let effect = data.effect.as_deref();
    let takes = effect.is_some_and(vectorcraft_effects::takes_graphics);
    if let Some(action) = library(ui, data, takes) {
        fields.set(ACTION, FieldValue::Json(action.to_value()));
    }
    let Some(effect) = effect else { return };
    if !takes {
        Text::new(tl!("Graphics map onto the Plane, Extrude, Revolve and Inflate object types.")).shade(Shade::Faint).show(ui);
        return;
    }
    if fields.is_mixed(GRAPHICS) {
        Text::new(tl!("The selected objects have different graphics.")).shade(Shade::Faint).show(ui);
        return;
    }
    let raw = json_of(fields, GRAPHICS).unwrap_or(Value::Null);
    let mut list: Vec<MappedGraphic> = vectorcraft_effects::mapped_graphics(&json!({ GRAPHICS: raw }));
    Text::new(tl!("Mapped graphics")).show(ui);
    if list.is_empty() {
        Text::new(tl!("Pick a graphic above to map it onto the object.")).shade(Shade::Faint).show(ui);
        return;
    }
    let shown = fields.int(SELECTED).map_or(0, |i| usize::try_from(i).unwrap_or(0));
    let mut index = shown.min(list.len() - 1);
    for (i, g) in list.iter().enumerate() {
        if clicked(&Button::new(format!("{} · {}", g.symbol, face_label(g.face))).selected(i == index).id_salt(("mapped", i)).show(ui)) {
            index = i;
        }
    }
    let Some(g) = list.get(index).cloned() else { return };
    let mut edited = g.clone();
    // A plane has one face, and Revolve's graphics go on its side. Extrude & Bevel's graphics go on
    // its front or back cap, or on its side (its walls and bevels). Inflate's go on its puffed front
    // or back.
    if effect != ROTATE && effect != REVOLVE {
        let faces: &[PlanarFace] = if effect == INFLATE { &[PlanarFace::Front, PlanarFace::Back] } else { &PlanarFace::ALL };
        Text::new(tl!("Face")).show(ui);
        for face in faces {
            if clicked(&Button::new(face_label(*face)).selected(*face == g.face).id_salt(("face", face_label(*face))).show(ui)) {
                edited.face = *face;
            }
        }
    }
    // The ranges the effect takes, so every stored value shows as it is.
    let max = vectorcraft_effects::GRAPHIC_MAX_OFFSET;
    let (scale_lo, scale_hi) = vectorcraft_effects::GRAPHIC_SCALE_RANGE;
    let params = [
        Param::number("graphicX", "X", 0.0, 1.0, 1).bounds(Some(-max), Some(max)).resettable(),
        Param::number("graphicY", "Y", 0.0, 1.0, 1).bounds(Some(-max), Some(max)).resettable(),
        Param::number("graphicScale", tl!("Scale"), 100.0, 1.0, 1).bounds(Some(scale_lo), Some(scale_hi)).resettable(),
        Param::number("graphicRotation", tl!("Rotation"), 0.0, 1.0, 1).bounds(Some(-360.0), Some(360.0)).resettable(),
    ];
    let numbers = DialogFields::from_pairs([("graphicX", g.x), ("graphicY", g.y), ("graphicScale", g.scale), ("graphicRotation", g.rotation)]);
    let rows = ParamRows::new(tl!("Mapped graphic"), &params, Bind::given(numbers)).show(ui);
    // A drag of a number is one step of the outer rows.
    cx.gesture(&rows.events);
    if let Some(v) = &rows.values {
        edited.x = v.number("graphicX").unwrap_or(g.x);
        edited.y = v.number("graphicY").unwrap_or(g.y);
        edited.scale = v.number("graphicScale").unwrap_or(g.scale);
        edited.rotation = v.number("graphicRotation").unwrap_or(g.rotation);
    }
    let remove = clicked(&Button::new(tl!("Remove Graphic")).show(ui));
    if remove {
        list.remove(index);
        index = index.saturating_sub(1);
    } else if edited != g {
        list[index] = edited;
    }
    if index != shown {
        fields.set(SELECTED, index as i64);
    }
    if remove || list.get(index).is_some_and(|now| *now != g) {
        fields.set(GRAPHICS, FieldValue::Json(Value::Array(list.iter().map(MappedGraphic::to_value).collect())));
    }
}

/// The document's graphics as thumbnails, with Add as Single Graphic below them. A thumbnail
/// clicked while `apply` (a 3D object is selected) maps that graphic; art dropped on them is added.
/// Returns the action asked for.
fn library(ui: &mut Ui, data: &Data, apply: bool) -> Option<Action> {
    Text::new(tl!("Graphics")).show(ui);
    let mut action = None;
    let (Some(doc), true) = (data.doc.clone(), true) else { return action };
    let names: Vec<String> = doc.symbols.iter().map(|s| s.name.clone()).collect();
    let keys = crate::panels::symbols::document_symbols(&doc, data.revision);
    let source = keyed(&names[..], |name: &String| name.clone()).with_text(|name| Cow::Borrowed(name.as_str()));
    let mut dropped = DropAction::<PanelDrag>::new(|drag| matches!(drag, PanelDrag::Art { .. })).show(ui, |ui, _| {
        TileGrid::new(tl!("Graphics")).activate(ActivateOn::Click).show_items(ui, &source, &mut |name, _| {
            let tip = if apply { name.clone() } else { format!("{name}: {}", tl!("Select a 3D object to map the graphic onto it.")) };
            let (doc, keys, name) = (doc.clone(), keys.clone(), name.clone());
            Tile::new(name.clone()).tooltip(tip).thumb(ThumbAspect::new(1, 1), move |painter, rect| {
                crate::panels::symbols::paint_symbol(painter, rect, &doc, &keys, &name);
            })
        })
    });
    if names.is_empty() {
        Text::new(tl!("Add art with Add as Single Graphic, or drop art here.")).shade(Shade::Faint).show(ui);
    }
    let picked = dropped.settle(|_, grid| std::mem::take(&mut grid.response.events)).unwrap_or_default();
    for event in picked {
        if let CollectionEvent::Activate(name) = event
            && apply
        {
            action = Some(Action::Map(name));
        }
    }
    if let Some(PanelDrag::Art { ids, .. }) = dropped.output.dropped {
        action = Some(Action::Add(Some(ids.iter().map(|id| id.0).collect())));
    }
    if clicked(&Button::new(tl!("Add as Single Graphic")).enabled(data.selected).show(ui)) {
        action = Some(Action::Add(None));
    }
    action
}
