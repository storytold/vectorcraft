//! Brushes panel: the document's brush library with rendered stroke previews. Clicking a brush
//! applies it to the selected paths (and makes it the Paintbrush's current brush); double-clicking
//! one opens its Brush Options.

use std::collections::HashMap;

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, Node, PressureProfile, color::Color, color::Paint};

use super::{first_selected, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::dialogs::brush_options;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};

const KINDS: [(&str, &str); 5] =
    [("calligraphic", "Calligraphic"), ("scatter", "Scatter"), ("art", "Art"), ("bristle", "Bristle"), ("pattern", "Pattern")];

/// (name, type) of every brush in the active document's library.
pub fn brushes(app: &mut VectorcraftApp) -> (Vec<(String, String)>, Option<String>) {
    let Ok(v) = app.run("brush.list", json!({})) else { return (vec![], None) };
    let list = v["brushes"]
        .as_array()
        .map(|a| a.iter().map(|b| (b["name"].as_str().unwrap_or("").to_string(), b["type"].as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    (list, v["current"].as_str().map(str::to_string))
}

/// A stroke preview for brush definition `def`, rendered at `size` and cached by the definition's
/// JSON. The stroke goes from light to heavy pen pressure and back, so pressure-sensitive brushes
/// show how they vary.
fn preview(ui: &Ui, def: &Value, size: egui::Vec2) -> Option<egui::TextureHandle> {
    widgets::doc_preview(ui, &format!("brush:{def}"), size, |w, h| {
        let mut doc = Document::new(w, h);
        doc.unknown.insert("brushes".into(), json!([def]));
        let name = def["name"].as_str()?.to_string();
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        let st = ap.stroke_mut()?;
        st.brush = Some(name);
        st.pressure = Some(PressureProfile::sample());
        // A gentle S-curve across the swatch.
        let mut bp = vectorcraft_geom::BezPath::new();
        let (x0, x1) = (h * 0.5, w - h * 0.5);
        bp.move_to((x0, h * 0.5));
        bp.curve_to((x0 + (x1 - x0) * 0.35, h * 0.1), (x0 + (x1 - x0) * 0.65, h * 0.9), (x1, h * 0.5));
        let id = doc.alloc_id();
        let l = doc.layers[0].id;
        doc.insert(Some(l), 0, Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), ap)).ok()?;
        Some(doc)
    })
}

/// The stroke preview of brush definition `def` on white in `r` (the chip a dragged brush shows at
/// the pointer).
pub(crate) fn chip(ui: &Ui, r: egui::Rect, def: &Value) {
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    if let Some(tex) = preview(ui, def, r.size()) {
        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

fn selected_brush(app: &VectorcraftApp) -> Option<String> {
    first_selected(app).and_then(|n| n.appearance.stroke().and_then(|s| s.brush.clone()))
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (list, current) = brushes(app);
    let sel_brush = selected_brush(app);
    let list_view: bool = pstate(ui.ctx(), "br-list");
    let hidden: Vec<String> = pstate(ui.ctx(), "br-hidden");
    let defs: HashMap<String, Value> =
        list.iter().filter_map(|(n, _)| app.run("brush.get", json!({"name": n})).ok().map(|v| (n.clone(), v))).collect();
    let mut clicked: Option<String> = None;
    let mut options: Option<String> = None;
    let list_rect = widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.set_width(ui.available_width());
        if list.is_empty() {
            super::empty_state(ui, "paintbrush", tl!("No brushes"), tl!("Select art and use New Brush to make one."));
            return ui.min_rect();
        }
        for (ty, _) in KINDS {
            if hidden.iter().any(|h| h == ty) {
                continue;
            }
            let group: Vec<&(String, String)> = list.iter().filter(|b| b.1 == ty).collect();
            if group.is_empty() {
                continue;
            }
            let row = |ui: &mut Ui, name: &str, size: egui::Vec2, label: bool| -> egui::Response {
                let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                // Dragged onto a path, the brush is applied to it.
                widgets::drag_source(ui, &resp, || PanelDrag::Brush { name: name.to_string(), def: defs.get(name).cloned().unwrap_or_default() });
                let on = sel_brush.as_deref() == Some(name) || (sel_brush.is_none() && current.as_deref() == Some(name));
                if on {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                let pw = if label { 72.0 } else { size.x - 4.0 };
                let pr = egui::Rect::from_min_size(r.left_top() + vec2(2.0, 2.0), vec2(pw, size.y - 4.0));
                ui.painter().rect_filled(pr, 0.0, egui::Color32::WHITE);
                if let Some(def) = defs.get(name)
                    && let Some(tex) = preview(ui, def, pr.size())
                {
                    ui.painter().image(tex.id(), pr, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                if label {
                    ui.painter().text(
                        r.left_center() + vec2(pw + 10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(12.5),
                        t.text,
                    );
                }
                resp.on_hover_text(name)
            };
            // A click applies the brush; the second click of a double-click opens its options.
            let mut pick = |resp: egui::Response, name: &String| {
                if resp.double_clicked() {
                    options = Some(name.clone());
                } else if resp.clicked() {
                    clicked = Some(name.clone());
                }
            };
            if list_view {
                for (name, _) in group {
                    pick(row(ui, name, vec2(ui.available_width(), 26.0), true), name);
                }
            } else {
                ui.horizontal_wrapped(|ui| {
                    for (name, _) in group {
                        pick(row(ui, name, vec2(64.0, 28.0), false), name);
                    }
                });
            }
            ui.separator();
        }
        ui.min_rect()
    });
    // Art dragged off the canvas and dropped on the list becomes an Art brush, as New Brush makes.
    let zone = ui.interact(list_rect, ui.id().with("brushes-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone)
        && let Err(e) = app.run("brush.new", json!({ "type": "art", "ids": ids }))
    {
        app.status(e);
    }
    if let Some(name) = clicked {
        let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
        if has_sel {
            app.run("brush.apply", json!({"name": name})).ok();
        } else {
            app.run("brush.setCurrent", json!({"name": name})).ok();
        }
    }
    if let Some(name) = options
        && let Err(e) = app.run("ui.brushOptions", json!({ "name": name }))
    {
        app.status(e);
    }
    let has_brush = sel_brush.is_some();
    let target = sel_brush.clone().or(current.clone());
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    widgets::bottom_bar(ui, |ui| {
        widgets::icon_button_enabled(ui, "library", tl!("Brush Libraries (on the roadmap)"), false, false, 24.0);
        if widgets::icon_button_enabled(ui, "dc-remove-brush", tl!("Remove Brush Stroke"), false, has_brush, 24.0).clicked() {
            app.run("brush.remove", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "dc-options", tl!("Expand Brush Strokes"), false, has_brush, 24.0).clicked() {
            app.run("object.expandBrush", json!({})).ok();
        }
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "dc-new-item", tl!("New Art Brush from Selection"), false, has_sel, 24.0).clicked() {
            app.run("brush.new", json!({"type": "art"})).ok();
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Brush"), false, target.is_some(), 24.0).clicked()
            && let Some(n) = &target
        {
            app.run("brush.delete", json!({"name": n})).ok();
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel = selected_brush(app);
    let (_, current) = brushes(app);
    let target = sel.clone().or(current);
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    for (label, ty) in [(tl!("New Calligraphic Brush"), "calligraphic"), (tl!("New Bristle Brush"), "bristle")] {
        if menu_item(ui, label, true, false) {
            app.run("brush.new", json!({"type": ty})).ok();
        }
    }
    for (label, ty) in [
        (tl!("New Art Brush from Selection"), "art"),
        (tl!("New Scatter Brush from Selection"), "scatter"),
        (tl!("New Pattern Brush from Selection"), "pattern"),
    ] {
        if menu_item(ui, label, has_sel, false) {
            app.run("brush.new", json!({"type": ty})).ok();
        }
    }
    if menu_item(ui, tl!("Duplicate Brush"), target.is_some(), false)
        && let Some(n) = &target
    {
        app.run("brush.duplicate", json!({"name": n})).ok();
    }
    if menu_item(ui, tl!("Delete Brush"), target.is_some(), false)
        && let Some(n) = &target
    {
        app.run("brush.delete", json!({"name": n})).ok();
    }
    if menu_item(ui, tl!("Remove Brush Stroke"), sel.is_some(), false) {
        app.run("brush.remove", json!({})).ok();
    }
    if menu_item(ui, tl!("Expand Brush Strokes"), sel.is_some(), false) {
        app.run("object.expandBrush", json!({})).ok();
    }
    // On the selected path's brush, else the current one, as the other items.
    if menu_item(ui, tl!("Brush Options…"), brush_options::available(app), false)
        && let Err(e) = app.run("ui.brushOptions", json!({}))
    {
        app.status(e);
    }
    ui.separator();
    let mut hidden: Vec<String> = pstate(ui.ctx(), "br-hidden");
    for (ty, label) in KINDS {
        let shown = !hidden.iter().any(|h| h == ty);
        if menu_item(ui, &crate::i18n::fmt(tl!("Show {kind} Brushes"), &[("kind", tl!(label))]), true, shown) {
            if shown {
                hidden.push(ty.to_string());
            } else {
                hidden.retain(|h| h != ty);
            }
            set_pstate(ui.ctx(), "br-hidden", hidden.clone());
        }
    }
    ui.separator();
    let list: bool = pstate(ui.ctx(), "br-list");
    if menu_item(ui, tl!("Thumbnail View"), true, !list) {
        set_pstate(ui.ctx(), "br-list", false);
    }
    if menu_item(ui, tl!("List View"), true, list) {
        set_pstate(ui.ctx(), "br-list", true);
    }
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, pos2};
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    use super::*;

    /// One headless frame of the panel, 236 pt wide as in the dock.
    fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    }

    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// #852: double-clicking a brush opens its Brush Options; OK changes the brush (and strokes
    /// painted with it). A single click only applies the brush.
    #[test]
    fn double_clicking_a_brush_opens_its_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        let clicks = |at: Pos2, n: usize| -> Vec<egui::Event> {
            std::iter::once(egui::Event::PointerMoved(at)).chain((0..n).flat_map(|_| [button(at, true), button(at, false)])).collect()
        };
        let at_time = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
        };
        at_time(&mut app, 0.0, vec![]);
        let (list, _) = brushes(&mut app);
        let first = list[0].0.clone();
        assert_eq!(list[0].1, "calligraphic");
        // The first tile, clicked once: it becomes current, no dialog.
        let tile = pos2(36.0, 16.0);
        at_time(&mut app, 1.0, clicks(tile, 1));
        assert!(app.ui.dialog.is_none());
        assert_eq!(brushes(&mut app).1.as_deref(), Some(first.as_str()));
        // Double-clicked: Calligraphic Brush Options on that brush.
        at_time(&mut app, 3.0, clicks(tile, 2));
        let d = app.ui.dialog.as_mut().expect("Brush Options opened");
        assert_eq!((d.kind.as_str(), d.str("name"), d.str("sizeMode")), (crate::dialogs::brush_options::KIND, first.clone(), "fixed".into()));
        d.fields.insert("sizeMode".into(), json!("pressure"));
        d.fields.insert("sizeVariation".into(), json!(2));
        d.fields.insert("name".into(), json!("Pressure Round"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let def = app.run("brush.get", json!({"name": "Pressure Round"})).unwrap();
        assert_eq!((def["modes"].clone(), def["variation"][2].clone()), (json!(["fixed", "fixed", "pressure"]), json!(2.0)));
        assert_eq!(brushes(&mut app).1.as_deref(), Some("Pressure Round"), "renamed, still current");
    }

    #[test]
    fn art_dropped_on_the_panel_becomes_an_art_brush_and_a_brush_drags_out() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 9, "points": 5})).unwrap()["id"].as_u64().unwrap();
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, vec![]);
        let (before, _) = brushes(&mut app);
        // Art dragged off the canvas and released over the list.
        let over = pos2(100.0, 60.0);
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(id)]));
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(over), button(over, false)]);
        let (after, _) = brushes(&mut app);
        assert_eq!(after.len(), before.len() + 1);
        let new: Vec<_> = after.iter().filter(|b| !before.contains(b)).collect();
        assert!(matches!(new.as_slice(), [(_, ty)] if ty == "art"), "{new:?}");
        // Dragging the first tile out of the panel carries that brush.
        frame(&ctx, &mut app, vec![]);
        let tile = pos2(36.0, 16.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
        let drag = egui::DragAndDrop::payload::<PanelDrag>(&ctx);
        assert!(matches!(drag.as_deref(), Some(PanelDrag::Brush { name, def }) if *name == after[0].0 && def["name"] == json!(name)), "{drag:?}");
    }
}
