//! Brushes panel: the document's brush library with rendered stroke previews. Clicking a brush
//! applies it to the selected paths (and makes it the Paintbrush's current brush); double-clicking
//! one opens its Brush Options. Shift-click selects a range of brushes, Cmd/Ctrl-click adds or
//! removes one, and the panel menu's Select All Unused selects the brushes no stroke paints with,
//! for Delete Brush and Duplicate Brush.

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

/// What the panel's last click was made with: the document, its selected objects and the current
/// brush.
type ClickKey = (u64, Vec<u64>, Option<String>);

fn click_key(app: &VectorcraftApp, current: Option<&str>) -> Option<ClickKey> {
    app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect(), current.map(str::to_string)))
}

/// Whether the panel's last click was made with the document, objects and current brush active
/// now. Panel state `br-selected` holds the multi-selection and the key of that click.
fn same_click(app: &VectorcraftApp, ctx: &egui::Context, current: Option<&str>) -> bool {
    let (_, key): (Option<Vec<String>>, Option<ClickKey>) = pstate(ctx, "br-selected");
    key.is_some() && key == click_key(app, current)
}

/// The brushes that Shift-clicks, Cmd/Ctrl-clicks and Select All Unused selected, of those in
/// `list`. It is `None` after a plain click, once none of its brushes is left, and once another
/// document, other objects or another current brush is active, which also clears it and the key
/// of the last click; `Some` of an empty list after Cmd/Ctrl-clicks deselected every brush or
/// Select All Unused found none.
fn multi(app: &VectorcraftApp, ctx: &egui::Context, list: &[(String, String)], current: Option<&str>) -> Option<Vec<String>> {
    if !same_click(app, ctx, current) {
        set_pstate(ctx, "br-selected", (None::<Vec<String>>, None::<ClickKey>));
        return None;
    }
    let (names, _): (Option<Vec<String>>, Option<ClickKey>) = pstate(ctx, "br-selected");
    let names = names?;
    if names.is_empty() {
        return Some(names);
    }
    let kept: Vec<String> = names.into_iter().filter(|n| list.iter().any(|b| &b.0 == n)).collect();
    (!kept.is_empty()).then_some(kept)
}

/// The brushes Delete Brush and Duplicate Brush act on: the multi-selection, else the selected
/// path's brush, else the current one.
fn targets(app: &VectorcraftApp, ctx: &egui::Context, list: &[(String, String)], current: Option<&str>) -> Vec<String> {
    multi(app, ctx, list, current).unwrap_or_else(|| selected_brush(app).or(current.map(str::to_string)).into_iter().collect())
}

/// Runs `brush.delete` or `brush.duplicate` on `names`.
fn run_on(app: &mut VectorcraftApp, id: &str, names: &[String]) {
    let p = match names {
        [name] => json!({ "name": name }),
        _ => json!({ "names": names }),
    };
    app.run(id, p).ok();
}

/// The names in `list` in the order the panel shows them: by type in [`KINDS`] order, without the
/// hidden types.
fn shown<'a>(list: &'a [(String, String)], hidden: &[String]) -> Vec<&'a str> {
    let kinds = KINDS.iter().filter(|(ty, _)| !hidden.iter().any(|h| h == ty));
    kinds.flat_map(|(ty, _)| list.iter().filter(move |b| b.1 == *ty).map(|b| b.0.as_str())).collect()
}

/// A brush dragged over the tile or row `resp` of brush `name`, the `i`th brush of type `ty`
/// (`list`: rows stack downwards), lands before or after it by the pointer's half; a bar marks the
/// slot when the dragged brush is of type `ty` too (`types`: the (name, type) of every brush).
/// Released there, it sets `moved` to the `brush.move` params, which refuse a brush of another type.
fn reorder_drop(
    ui: &Ui,
    resp: &egui::Response,
    types: &[(String, String)],
    (name, ty, i): (&str, &str, usize),
    list: bool,
    moved: &mut Option<Value>,
) {
    let Some(drag) = resp.dnd_hover_payload::<PanelDrag>() else { return };
    let PanelDrag::Brush { name: moving, .. } = &*drag else { return };
    if moving == name {
        // Dropped on itself: it stays where it is.
        resp.dnd_release_payload::<PanelDrag>();
        return;
    }
    let r = resp.rect;
    let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
    let after = if list { at.y > r.center().y } else { at.x > r.center().x };
    if types.iter().any(|b| &b.0 == moving && b.1 == ty) {
        let accent = egui::Stroke::new(2.0, Tokens::get(ui.ctx()).accent);
        if list {
            let y = if after { r.bottom() } else { r.top() };
            ui.painter().line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], accent);
        } else {
            let x = if after { r.right() + 1.5 } else { r.left() - 1.5 };
            ui.painter().line_segment([egui::pos2(x, r.top()), egui::pos2(x, r.bottom())], accent);
        }
    }
    if resp.dnd_release_payload::<PanelDrag>().is_some() {
        *moved = Some(json!({ "name": moving, "to": i + usize::from(after), "type": ty }));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (list, current) = brushes(app);
    let sel_brush = selected_brush(app);
    let mut multi = multi(app, ui.ctx(), &list, current.as_deref());
    let list_view: bool = pstate(ui.ctx(), "br-list");
    let hidden: Vec<String> = pstate(ui.ctx(), "br-hidden");
    let defs: HashMap<String, Value> =
        list.iter().filter_map(|(n, _)| app.run("brush.get", json!({"name": n})).ok().map(|v| (n.clone(), v))).collect();
    let mut clicked: Option<(String, egui::Modifiers)> = None;
    let mut options: Option<String> = None;
    let mut moved: Option<Value> = None;
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
                let on = match &multi {
                    Some(multi) => multi.iter().any(|m| m == name),
                    None => sel_brush.as_deref() == Some(name) || (sel_brush.is_none() && current.as_deref() == Some(name)),
                };
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
            // A click applies or selects the brush; the second click of a double-click opens its
            // options.
            let mut pick = |resp: egui::Response, name: &String| {
                if resp.double_clicked() {
                    options = Some(name.clone());
                } else if resp.clicked() {
                    clicked = Some((name.clone(), resp.ctx.input(|i| i.modifiers)));
                }
            };
            if list_view {
                for (i, (name, _)) in group.into_iter().enumerate() {
                    let resp = row(ui, name, vec2(ui.available_width(), 26.0), true);
                    reorder_drop(ui, &resp, &list, (name, ty, i), true, &mut moved);
                    pick(resp, name);
                }
            } else {
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, _)) in group.into_iter().enumerate() {
                        let resp = row(ui, name, vec2(64.0, 28.0), false);
                        reorder_drop(ui, &resp, &list, (name, ty, i), false, &mut moved);
                        pick(resp, name);
                    }
                });
            }
            ui.separator();
        }
        ui.min_rect()
    });
    // Art dragged off the canvas and dropped on the list opens New Brush for that art, as the New
    // Brush button opens it for the selection.
    let zone = ui.interact(list_rect, ui.id().with("brushes-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone)
        && let Err(e) = app.run("ui.newBrush", json!({ "ids": ids }))
    {
        app.status(e);
    }
    // A brush dragged within the list moves among the brushes of its type.
    if let Some(p) = moved
        && let Err(e) = app.run("brush.move", p)
    {
        app.status(e);
    }
    let highlighted = sel_brush.clone().or(current.clone());
    match clicked {
        // Shift and Cmd/Ctrl only select, as in the Swatches panel: they start from the
        // multi-selection, else from the highlighted brush.
        Some((name, m)) if m.shift || m.command => {
            let order = shown(&list, &hidden);
            // A range starts from the last brush clicked without Shift while the same document,
            // art and current brush are active, else from the highlighted brush.
            let anchor: String = pstate(ui.ctx(), "br-anchor");
            if !same_click(app, ui.ctx(), current.as_deref()) || !order.contains(&anchor.as_str()) {
                set_pstate(ui.ctx(), "br-anchor", highlighted.clone().unwrap_or_default());
            }
            let from = multi.unwrap_or_else(|| highlighted.iter().cloned().collect());
            let sel = super::swatches::click_selection(ui, "br-anchor", from, &order, &name, m);
            set_pstate(ui.ctx(), "br-selected", (Some(sel.clone()), click_key(app, current.as_deref())));
            multi = Some(sel);
        }
        Some((name, _)) => {
            set_pstate(ui.ctx(), "br-anchor", name.clone());
            multi = None;
            let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
            if has_sel {
                app.run("brush.apply", json!({"name": name})).ok();
            } else {
                app.run("brush.setCurrent", json!({"name": name})).ok();
            }
            let now = brushes(app).1;
            set_pstate(ui.ctx(), "br-selected", (None::<Vec<String>>, click_key(app, now.as_deref())));
        }
        None => {}
    }
    if let Some(name) = options
        && let Err(e) = app.run("ui.brushOptions", json!({ "name": name }))
    {
        app.status(e);
    }
    let has_brush = sel_brush.is_some();
    let targets = multi.unwrap_or_else(|| highlighted.into_iter().collect());
    let has_doc = app.session.active().is_some();
    widgets::bottom_bar(ui, |ui| {
        widgets::icon_button_enabled(ui, "library", tl!("Brush Libraries (on the roadmap)"), false, false, 24.0);
        if widgets::icon_button_enabled(ui, "dc-remove-brush", tl!("Remove Brush Stroke"), false, has_brush, 24.0).clicked() {
            app.run("brush.remove", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "dc-options", tl!("Expand Brush Strokes"), false, has_brush, 24.0).clicked() {
            app.run("object.expandBrush", json!({})).ok();
        }
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        // New Brush asks for the type, then opens that type's Brush Options.
        if widgets::icon_button_enabled(ui, "dc-new-item", tl!("New Brush"), false, has_doc, 24.0).clicked()
            && let Err(e) = app.run("ui.newBrush", json!({}))
        {
            app.status(e);
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Brush"), false, !targets.is_empty(), 24.0).clicked() {
            run_on(app, "brush.delete", &targets);
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel = selected_brush(app);
    let (list, current) = brushes(app);
    let targets = targets(app, ui.ctx(), &list, current.as_deref());
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    if menu_item(ui, tl!("New Brush…"), app.session.active().is_some(), false)
        && let Err(e) = app.run("ui.newBrush", json!({}))
    {
        app.status(e);
    }
    // The types' own items make a brush at once, without its Brush Options.
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
    if menu_item(ui, tl!("Duplicate Brush"), !targets.is_empty(), false) {
        run_on(app, "brush.duplicate", &targets);
    }
    if menu_item(ui, tl!("Delete Brush"), !targets.is_empty(), false) {
        run_on(app, "brush.delete", &targets);
    }
    if menu_item(ui, tl!("Remove Brush Stroke"), sel.is_some(), false) {
        app.run("brush.remove", json!({})).ok();
    }
    if menu_item(ui, tl!("Expand Brush Strokes"), sel.is_some(), false) {
        app.run("object.expandBrush", json!({})).ok();
    }
    // On the selected path's brush, else the current one.
    if menu_item(ui, tl!("Brush Options…"), brush_options::available(app), false)
        && let Err(e) = app.run("ui.brushOptions", json!({}))
    {
        app.status(e);
    }
    ui.separator();
    // Selects the brushes no stroke paints with, for Delete Brush and Duplicate Brush.
    if menu_item(ui, tl!("Select All Unused"), app.session.active().is_some(), false)
        && let Ok(r) = app.run("brush.unused", json!({}))
    {
        let names: Vec<String> = serde_json::from_value(r["names"].clone()).unwrap_or_default();
        set_pstate(ui.ctx(), "br-selected", (Some(names), click_key(app, current.as_deref())));
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

    /// One frame of `draw` at `time` with `modifiers` held, its output kept.
    fn frame_with(
        ctx: &egui::Context,
        app: &mut VectorcraftApp,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
        time: f64,
        draw: fn(&mut VectorcraftApp, &mut Ui),
    ) -> egui::FullOutput {
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 600.0)));
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        let raw = egui::RawInput { screen_rect, time: Some(time), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| draw(app, ui));
        out.textures_delta.clear();
        out
    }

    /// A click at `at` with `modifiers` held, in a frame of `draw` at `time`.
    fn click(ctx: &egui::Context, app: &mut VectorcraftApp, at: Pos2, modifiers: egui::Modifiers, time: f64, draw: fn(&mut VectorcraftApp, &mut Ui)) {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        frame_with(ctx, app, vec![egui::Event::PointerMoved(at), press(true), press(false)], modifiers, time, draw);
    }

    /// The accessibility nodes of `out` that take a click, with their rectangles, top to bottom
    /// and then left to right.
    fn clickable(out: &egui::FullOutput) -> Vec<(Option<String>, Rect)> {
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let mut v: Vec<(Option<String>, Rect)> = nodes
            .iter()
            .filter(|(_, n)| n.supports_action(egui::accesskit::Action::Click))
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                Some((n.label().map(|l| l.trim().to_string()), Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))))
            })
            .collect();
        v.sort_by(|a, b| a.1.min.y.total_cmp(&b.1.min.y).then(a.1.min.x.total_cmp(&b.1.min.x)));
        v
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

    /// The New Brush button and the panel menu's New Brush… open New Brush, with nothing selected
    /// too, and make no brush.
    #[test]
    fn the_new_brush_button_and_menu_item_open_new_brush() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let before = names(&mut app);
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 0.0, show);
        let new = clickable(&out).into_iter().find(|(l, _)| l.as_deref() == Some("New Brush")).expect("a New Brush button").1.center();
        click(&ctx, &mut app, new, egui::Modifiers::NONE, 1.0, show);
        let kind = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.kind.clone());
        assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::new_brush::KIND));
        app.ui.dialog = None;
        menu_click(&ctx, &mut app, "New Brush…", 2.0);
        assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::new_brush::KIND));
        assert_eq!(names(&mut app), before);
    }

    /// The rectangles `out` fills with the selected-row color.
    fn lit(ctx: &egui::Context, out: &egui::FullOutput) -> Vec<Rect> {
        let t = Tokens::get(ctx);
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == t.row_selected => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    fn brush_of(app: &VectorcraftApp, id: NodeId) -> Option<String> {
        app.session.active().unwrap().doc.node(id).unwrap().appearance.stroke().and_then(|s| s.brush.clone())
    }

    fn names(app: &mut VectorcraftApp) -> Vec<String> {
        brushes(app).0.into_iter().map(|b| b.0).collect()
    }

    /// Click the panel menu's item `label` (frames at `time`, `time + 0.5`).
    fn menu_click(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, time: f64) {
        let out = frame_with(ctx, app, vec![], egui::Modifiers::NONE, time, menu);
        let at = clickable(&out).into_iter().find(|(l, _)| l.as_deref() == Some(label)).unwrap_or_else(|| panic!("no `{label}`")).1.center();
        click(ctx, app, at, egui::Modifiers::NONE, time + 0.5, menu);
    }

    /// The first three brushes (the Calligraphic ones): a click on the first applies it to the
    /// selected rectangle, and then a Shift-click on the third selects all three without applying
    /// the third.
    fn select_three(ctx: &egui::Context, app: &mut VectorcraftApp, rect: NodeId) -> (Vec<String>, Vec<Rect>) {
        let out = frame_with(ctx, app, vec![], egui::Modifiers::NONE, 0.0, show);
        let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).take(3).collect();
        let (list, _) = brushes(app);
        assert!(list[..3].iter().all(|b| b.1 == "calligraphic"), "{list:?}");
        let three: Vec<String> = list[..3].iter().map(|b| b.0.clone()).collect();
        click(ctx, app, tiles[0].center(), egui::Modifiers::NONE, 1.0, show);
        assert_eq!(brush_of(app, rect).as_deref(), Some(three[0].as_str()));
        click(ctx, app, tiles[2].center(), egui::Modifiers::SHIFT, 2.0, show);
        assert_eq!(brush_of(app, rect).as_deref(), Some(three[0].as_str()), "a Shift-click only selects");
        (three, tiles)
    }

    /// Shift-click selects a range of brushes and Cmd/Ctrl-click adds or removes one; the panel
    /// highlights them, and Duplicate Brush and Delete Brush act on all of them in one undo step.
    #[test]
    fn shift_and_command_clicks_select_brushes_for_delete_and_duplicate() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let rect = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let (three, tiles) = select_three(&ctx, &mut app, rect);
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 2.5, show);
        let lit_tiles = |out: &egui::FullOutput| -> Vec<usize> { (0..3).filter(|i| lit(&ctx, out).contains(&tiles[*i])).collect() };
        assert_eq!(lit_tiles(&out), [0, 1, 2]);
        // Cmd/Ctrl-click takes the second out again.
        click(&ctx, &mut app, tiles[1].center(), egui::Modifiers::COMMAND, 3.0, show);
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 3.5, show);
        assert_eq!(lit_tiles(&out), [0, 2]);
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let before = (names(&mut app), undo(&app));
        menu_click(&ctx, &mut app, "Duplicate Brush", 4.0);
        let copies = [format!("{} copy", three[0]), format!("{} copy", three[2])];
        assert_eq!(names(&mut app)[before.0.len()..], copies);
        assert_eq!(undo(&app), before.1 + 1);
        // The bottom bar's last button, Delete Brush.
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 5.0, show);
        let trash = clickable(&out).last().unwrap().1.center();
        click(&ctx, &mut app, trash, egui::Modifiers::NONE, 6.0, show);
        let left = names(&mut app);
        assert!(!left.contains(&three[0]) && !left.contains(&three[2]) && left.contains(&three[1]), "{left:?}");
        assert!(copies.iter().all(|c| left.contains(c)), "{left:?}");
        assert_eq!(brush_of(&app, rect), None);
        assert_eq!(undo(&app), before.1 + 2);
    }

    /// Selecting other art ends the multi-selection: selecting the first art again does not bring
    /// it back, and with a path stroked with another brush selected, the panel highlights that
    /// brush and Delete Brush deletes it alone.
    #[test]
    fn selecting_other_art_ends_the_brush_multi_selection() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let rect = |app: &mut VectorcraftApp, x: f64| {
            NodeId(app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap())
        };
        let a = rect(&mut app, 10.0);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let (three, tiles) = select_three(&ctx, &mut app, a);
        let lit_tiles = |out: &egui::FullOutput| -> Vec<usize> { (0..3).filter(|i| lit(&ctx, out).contains(&tiles[*i])).collect() };
        // A path without a brush, then the first one again: the current brush is highlighted.
        rect(&mut app, 60.0);
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 3.0, show);
        assert_eq!(lit_tiles(&out), [0]);
        app.run("select.set", json!({"ids": [a.0]})).unwrap();
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 3.5, show);
        assert_eq!(lit_tiles(&out), [0]);
        // A path stroked with the second brush.
        let c = rect(&mut app, 110.0);
        app.run("brush.apply", json!({"name": three[1]})).unwrap();
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 4.0, show);
        assert_eq!(lit_tiles(&out), [1]);
        menu_click(&ctx, &mut app, "Delete Brush", 5.0);
        let left = names(&mut app);
        assert!(left.contains(&three[0]) && !left.contains(&three[1]) && left.contains(&three[2]), "{left:?}");
        assert_eq!((brush_of(&app, a).as_deref(), brush_of(&app, c)), (Some(three[0].as_str()), None));
    }

    /// After other art is selected, a Shift-click selects the range from that art's brush, not from
    /// the brush clicked before.
    #[test]
    fn a_shift_click_after_selecting_other_art_starts_from_its_brush() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 0.0, show);
        let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).take(3).collect();
        let (list, _) = brushes(&mut app);
        click(&ctx, &mut app, tiles[0].center(), egui::Modifiers::NONE, 1.0, show);
        app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 40})).unwrap();
        app.run("brush.apply", json!({"name": list[2].0})).unwrap();
        click(&ctx, &mut app, tiles[1].center(), egui::Modifiers::SHIFT, 2.0, show);
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 2.5, show);
        assert_eq!(lit(&ctx, &out).iter().filter(|r| tiles.contains(r)).collect::<Vec<_>>(), [&tiles[1], &tiles[2]]);
    }

    /// Cmd/Ctrl-clicks can deselect every brush: none is highlighted, and Delete Brush and
    /// Duplicate Brush are off.
    #[test]
    fn command_clicks_deselect_every_brush() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let a = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let (_, tiles) = select_three(&ctx, &mut app, a);
        for (i, tile) in tiles.iter().enumerate() {
            click(&ctx, &mut app, tile.center(), egui::Modifiers::COMMAND, 3.0 + i as f64, show);
        }
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 7.0, show);
        assert!(!lit(&ctx, &out).iter().any(|r| tiles.contains(r)));
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 8.0, menu);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        for label in ["Duplicate Brush", "Delete Brush"] {
            let item = nodes.iter().find(|(_, n)| n.label().map(str::trim) == Some(label)).unwrap_or_else(|| panic!("no `{label}`"));
            assert!(item.1.is_disabled(), "{label}");
        }
    }

    /// Select All Unused in the panel menu highlights every brush but the one the rectangle is
    /// painted with, and Delete Brush then deletes all of them.
    #[test]
    fn select_all_unused_selects_the_brushes_no_stroke_paints_with() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let (list, _) = brushes(&mut app);
        let a = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap());
        app.run("brush.apply", json!({"name": list[1].0})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        menu_click(&ctx, &mut app, "Select All Unused", 1.0);
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 2.0, show);
        let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).collect();
        assert_eq!(tiles.len(), list.len());
        let on: Vec<usize> = (0..tiles.len()).filter(|i| lit(&ctx, &out).contains(&tiles[*i])).collect();
        assert_eq!(on, (0..tiles.len()).filter(|i| *i != 1).collect::<Vec<_>>());
        menu_click(&ctx, &mut app, "Delete Brush", 3.0);
        assert_eq!(names(&mut app), [list[1].0.clone()]);
        assert_eq!(brush_of(&app, a), Some(list[1].0.clone()));
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
        // New Brush opens on Art for the dropped art, and OK twice makes the brush.
        let d = app.ui.dialog.as_ref().expect("New Brush opened");
        assert_eq!((d.kind.as_str(), d.str("type"), d.fields.get("__ids")), (crate::dialogs::new_brush::KIND, "art".into(), Some(&json!([id]))));
        assert_eq!(brushes(&mut app).0, before);
        crate::dialogs::confirm(&mut app).unwrap();
        crate::dialogs::confirm(&mut app).unwrap();
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

    /// A brush dragged onto the left half of another brush of its type goes before it, with a bar
    /// marking the slot while it is held there, as one undo step; dropped on a brush of another
    /// type, it stays where it is and the status bar says why.
    #[test]
    fn a_brush_dragged_onto_another_of_its_type_moves_before_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let out = frame_with(&ctx, &mut app, vec![], egui::Modifiers::NONE, 0.0, show);
        let tiles: Vec<Rect> = clickable(&out).into_iter().map(|c| c.1).filter(|r| r.width() == 64.0).collect();
        let (list, _) = brushes(&mut app);
        let order: Vec<String> = shown(&list, &[]).into_iter().map(str::to_string).collect();
        assert_eq!(order[..4], ["3 pt. Round", "6 pt. Flat", "10 pt. Oval", "Dots"]);
        let accent = Tokens::get(&ctx).accent;
        let bars = |out: &egui::FullOutput| -> Vec<f32> {
            let lines = out.shapes.iter().filter_map(|c| match &c.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == accent => Some(points[0].x),
                _ => None,
            });
            lines.collect()
        };
        // Press on a tile, drag it to `to`, hold it there for a frame (its output returned) and release.
        let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2, time: f64| -> egui::FullOutput {
            frame_with(&ctx, app, vec![egui::Event::PointerMoved(from), button(from, true)], egui::Modifiers::NONE, time, show);
            frame_with(&ctx, app, vec![egui::Event::PointerMoved(from + vec2(20.0, 0.0))], egui::Modifiers::NONE, time + 0.1, show);
            frame_with(&ctx, app, vec![egui::Event::PointerMoved(to)], egui::Modifiers::NONE, time + 0.2, show);
            let held = frame_with(&ctx, app, vec![], egui::Modifiers::NONE, time + 0.3, show);
            frame_with(&ctx, app, vec![button(to, false)], egui::Modifiers::NONE, time + 0.4, show);
            held
        };
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let steps = undo(&app);
        let calligraphic =
            |app: &mut VectorcraftApp| -> Vec<String> { brushes(app).0.into_iter().filter(|b| b.1 == "calligraphic").map(|b| b.0).collect() };
        // "10 pt. Oval" onto the left half of "3 pt. Round".
        let held = drag(&mut app, tiles[2].center(), tiles[0].center() - vec2(16.0, 0.0), 1.0);
        assert_eq!(bars(&held), [tiles[0].left() - 1.5]);
        assert_eq!(calligraphic(&mut app), ["10 pt. Oval", "3 pt. Round", "6 pt. Flat"]);
        assert_eq!(undo(&app), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(calligraphic(&mut app), ["3 pt. Round", "6 pt. Flat", "10 pt. Oval"]);
        // "3 pt. Round" onto "Dots", a scatter brush: no bar, and nothing moves.
        let held = drag(&mut app, tiles[0].center(), tiles[3].center(), 3.0);
        assert!(bars(&held).is_empty(), "{:?}", bars(&held));
        assert_eq!(brushes(&mut app).0, list);
        assert_eq!(undo(&app), steps);
        assert!(app.ui.status.contains("brushes move only within their type"), "{}", app.ui.status);
    }
}
