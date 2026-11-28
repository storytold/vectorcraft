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
    let Some(doc) = data.doc.clone() else { return action };
    let source_drawing = data.document;
    let names: Vec<String> = doc.symbols.iter().map(|s| s.name.clone()).collect();
    let keys = crate::panels::symbols::document_symbols(&doc, data.revision);
    let source = keyed(&names[..], |name: &String| name.clone()).with_text(|name| Cow::Borrowed(name.as_str()));
    let drop_art = DropAction::<PanelDrag>::new(move |drag| source_drawing.is_some_and(|(uid, reverts)| drag.artwork_for(uid, reverts).is_some()));
    let mut dropped = drop_art.show(ui, |ui, _| {
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
    if let Some((uid, reverts)) = source_drawing
        && let Some(ids) = dropped.output.dropped.as_ref().and_then(|drag| drag.artwork_for(uid, reverts))
    {
        action = Some(Action::Add(Some(ids.iter().map(|id| id.0).collect())));
    }
    if clicked(&Button::new(tl!("Add as Single Graphic")).enabled(data.selected).show(ui)) {
        action = Some(Action::Add(None));
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VectorcraftApp;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::{ListDrag, Role};
    use craft::ui::list::{List, ListRow};
    use craft::ui::runtime::drag;

    fn draw_graphics(app: &mut VectorcraftApp, ui: &mut Ui) {
        let st = app.session.active().unwrap();
        let payload = PanelDrag::Art { document: st.uid, reverts: st.reverts, ids: st.selection.objects.clone() };
        ui.allocate_ui(egui::vec2(ui.available_width(), 80.0), |ui| {
            let keys = keyed(&[0_usize], |key: &usize| *key);
            List::<usize>::new("Artwork")
                .drag_payload(move |_: &ListDrag<usize>| Some(payload.clone()))
                .show_items(ui, &keys, &mut |_, _| ListRow::new("Dragged artwork"));
        });
        if let Some(open) = &app.three_d_dialog {
            let data = open.data.read().unwrap();
            let _ = library(ui, &data, true);
        }
    }

    #[test]
    fn held_graphics_drag_is_refused_after_the_opening_drawing_reverts() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let dir = vectorcraft_testkit::TempDir::new("three-d-graphics-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let shape = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("threeD.addGraphic", json!({"name": "Opening Graphic"})).unwrap();
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run("shape.rectangle", json!({"x": 120, "y": 90, "width": 45, "height": 40})).unwrap();
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        assert!(app.session.set_active(0));
        app.run("select.set", json!({"ids": [shape]})).unwrap();

        let mut panel = Panel::new(app, draw_graphics);
        panel.room(egui::vec2(800.0, 820.0));
        super::super::three_d::open(&mut panel.app(), &json!({"effect": INFLATE})).unwrap();
        panel.frame();
        let from = panel.rect("Dragged artwork").center();
        let to = panel.rect("Opening Graphic").center();
        panel.drag_hold_role("Dragged artwork", Role::LIST_ITEM, to - from);
        assert!(drag::mark(panel.ctx()).is_some(), "the opening drawing's drag is accepted");
        assert!(panel.app().session.set_active(1));
        panel.frame();
        assert!(drag::mark(panel.ctx()).is_some(), "switching drawings retains the opening drawing's drag");

        {
            let mut app = panel.app();
            assert!(app.session.set_active(0));
            let source = app.session.active().unwrap();
            let (uid, generation) = (source.uid, source.reverts);
            app.run("file.revert", json!({"confirmed": true})).unwrap();
            let restored = app.session.active().unwrap();
            assert_eq!(restored.uid, uid);
            assert_ne!(restored.reverts, generation);
            assert!(app.session.set_active(1));
        }
        panel.frame();
        assert!(drag::active::<PanelDrag>(panel.ctx()).is_some(), "the same drag is still in progress");
        assert!(drag::mark(panel.ctx()).is_none(), "the drop stops accepting artwork from the reverted generation");
        let _ = panel.rect("Opening Graphic");
        panel.release();
    }
}
