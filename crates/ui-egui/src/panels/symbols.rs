//! Symbols panel: the document's symbols as rendered thumbnails or a list. Click selects a symbol
//! (it becomes the Symbol Sprayer's current symbol); double-click places an instance.

use std::cell::RefCell;
use std::collections::HashMap;

use egui::{Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};

/// A thumbnail of symbol `name` (an instance at its natural size), cached by the art's identity.
fn thumb(ui: &Ui, doc: &Document, name: &str, size: f32) -> Option<egui::TextureHandle> {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<(usize, String, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    let sym = doc.symbols.iter().find(|s| s.name == name)?;
    let px = (size * ui.ctx().pixels_per_point()).round().max(8.0) as u32;
    let key = (std::sync::Arc::as_ptr(&sym.art) as usize, name.to_string(), px);
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    let (w, h) = doc
        .unknown
        .get("symbolSizes")
        .and_then(|m| m.get(name))
        .and_then(|v| Some((v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
        .unwrap_or((20.0, 20.0));
    let mut d = Document::new(w.max(1.0), h.max(1.0));
    d.symbols.push(sym.clone());
    let id = d.alloc_id();
    let xf = vectorcraft_geom::Affine::scale_non_uniform(w / 20.0, h / 20.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf })).ok()?;
    let img = RENDERER.with(|r| r.borrow_mut().render_thumbnail(&d, id, px))?;
    let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
    let tex = ui.ctx().load_texture(format!("sym-{name}-{px}"), color, egui::TextureOptions::LINEAR);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    Some(tex)
}

/// Symbol `name`'s thumbnail on white in `r` (the panel's tiles and the chip a dragged symbol
/// shows at the pointer).
pub(crate) fn chip(ui: &Ui, doc: &Document, r: egui::Rect, name: &str) {
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    if let Some(tex) = thumb(ui, doc, name, r.width()) {
        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

fn current(app: &mut VectorcraftApp) -> Option<String> {
    app.run("symbol.list", json!({})).ok().and_then(|v| v["current"].as_str().map(str::to_string))
}

fn has_instances_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| {
            let mut any = false;
            if let Some(n) = st.doc.node(*id) {
                n.walk(&mut |c| any |= matches!(c.kind, NodeKind::SymbolInstance { .. }));
            }
            any
        })
    })
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let names: Vec<String> = doc.symbols.iter().map(|s| s.name.clone()).collect();
    let list: bool = pstate(ui.ctx(), "sym-list");
    let sel = current(app);
    let mut clicked: Option<(String, bool)> = None;
    let list_rect = widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.set_width(ui.available_width());
        if names.is_empty() {
            super::empty_state(ui, "spray-can", tl!("No symbols in this document"), tl!("Select art and click New Symbol to make one."));
            return ui.min_rect();
        }
        if list {
            for n in &names {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
                // Dragged onto the canvas, the symbol is placed there.
                widgets::drag_source(ui, &resp, || PanelDrag::Symbol(n.clone()));
                if sel.as_deref() == Some(n.as_str()) {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                chip(ui, &doc, egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(22.0, 22.0)), n);
                ui.painter().text(r.left_center() + vec2(32.0, 0.0), egui::Align2::LEFT_CENTER, n, egui::FontId::proportional(12.5), t.text);
                if resp.double_clicked() {
                    clicked = Some((n.clone(), true));
                } else if resp.clicked() {
                    clicked = Some((n.clone(), false));
                }
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                for n in &names {
                    let (r, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click_and_drag());
                    widgets::drag_source(ui, &resp, || PanelDrag::Symbol(n.clone()));
                    chip(ui, &doc, r.shrink(2.0), n);
                    let on = sel.as_deref() == Some(n.as_str());
                    ui.painter().rect_stroke(
                        r,
                        0.0,
                        Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.border }),
                        StrokeKind::Inside,
                    );
                    let resp = resp.on_hover_text(n);
                    if resp.double_clicked() {
                        clicked = Some((n.clone(), true));
                    } else if resp.clicked() {
                        clicked = Some((n.clone(), false));
                    }
                }
            });
        }
        ui.min_rect()
    });
    let zone = ui.interact(list_rect, ui.id().with("symbols-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone)
        && let Err(e) = app.run("symbol.new", json!({ "ids": ids }))
    {
        app.status(e);
    }
    if let Some((n, place)) = clicked {
        app.run("symbol.setCurrent", json!({"name": n})).ok();
        if place {
            app.run("symbol.place", json!({"name": n})).ok();
        }
    }
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let inst = has_instances_selected(app);
    widgets::bottom_bar(ui, |ui| {
        widgets::icon_button_enabled(ui, "library", tl!("Symbol Libraries (on the roadmap)"), false, false, 24.0);
        if widgets::icon_button_enabled(ui, "dc-place-symbol", tl!("Place Symbol Instance"), false, sel.is_some(), 24.0).clicked() {
            app.run("symbol.place", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "link-2-off", tl!("Break Link to Symbol"), false, inst, 24.0).clicked() {
            app.run("symbol.breakLink", json!({})).ok();
        }
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "dc-new-item", tl!("New Symbol"), false, has_sel, 24.0).clicked() {
            // Alt skips Symbol Options: the symbol is made under the default name.
            let cmd = if ui.input(|i| i.modifiers.alt) { "symbol.new" } else { "ui.symbolOptions" };
            app.run(cmd, json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Symbol"), false, sel.is_some(), 24.0).clicked()
            && let Some(n) = &sel
        {
            app.run("symbol.delete", json!({"name": n})).ok();
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel = current(app);
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let inst = has_instances_selected(app);
    let name = json!({"name": sel});
    let items: [(&str, bool, &str, Value); 10] = [
        (tl!("New Symbol…"), has_sel, "ui.symbolOptions", json!({})),
        (tl!("Redefine Symbol"), has_sel && sel.is_some(), "symbol.update", name.clone()),
        (tl!("Duplicate Symbol"), sel.is_some(), "symbol.duplicate", name.clone()),
        (tl!("Delete Symbol"), sel.is_some(), "symbol.delete", name.clone()),
        (tl!("Symbol Options…"), sel.is_some(), "ui.symbolOptions", json!({"edit": true, "name": sel})),
        (tl!("Edit Symbol"), inst, "symbol.edit", json!({})),
        (tl!("Place Symbol Instance"), sel.is_some(), "symbol.place", name.clone()),
        (tl!("Replace Symbol"), inst && sel.is_some(), "symbol.replace", name.clone()),
        (tl!("Break Link to Symbol"), inst, "symbol.breakLink", json!({})),
        (tl!("Select All Instances"), sel.is_some(), "symbol.selectInstances", name),
    ];
    for (label, enabled, cmd, params) in items {
        if menu_item(ui, label, enabled, false) {
            app.run(cmd, params).ok();
        }
    }
    ui.separator();
    let list: bool = pstate(ui.ctx(), "sym-list");
    if menu_item(ui, tl!("Thumbnail View"), true, !list) {
        set_pstate(ui.ctx(), "sym-list", false);
    }
    if menu_item(ui, tl!("List View"), true, list) {
        set_pstate(ui.ctx(), "sym-list", true);
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
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    }

    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    fn symbols(app: &mut VectorcraftApp) -> Vec<String> {
        let list = app.run("symbol.list", json!({})).unwrap();
        list["symbols"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap_or_default().to_string()).collect()
    }

    #[test]
    fn art_dropped_on_the_panel_becomes_a_symbol_and_a_symbol_drags_out() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
        // Selected, as art dragged off the canvas is.
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, vec![]);
        // Art dragged off the canvas and released over the list.
        let over = pos2(100.0, 50.0);
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(id)]));
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(over), button(over, false)]);
        let names = symbols(&mut app);
        assert_eq!(names.len(), 1, "{names:?}");
        let doc = &app.session.active().unwrap().doc;
        let first = doc.layers[0].children().unwrap().first().unwrap();
        assert!(matches!(first.kind, NodeKind::SymbolInstance { .. }), "the art became an instance");
        // Dragging the symbol's tile out of the panel carries the symbol.
        frame(&ctx, &mut app, vec![]);
        let tile = pos2(26.0, 26.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
        assert_eq!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).as_deref(), Some(&PanelDrag::Symbol(names[0].clone())));
    }

    /// One headless frame of the panel menu; returns its painted labels.
    fn menu_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| menu(app, ui));
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    }

    #[test]
    fn new_symbol_asks_for_symbol_options_before_making_the_symbol() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let labels = menu_frame(&ctx, &mut app, vec![]);
        let item = labels.iter().find(|(t, _)| t.trim() == "New Symbol…").map(|(_, r)| r.center()).expect("New Symbol… in the menu");
        menu_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(item), button(item, true)]);
        menu_frame(&ctx, &mut app, vec![button(item, false)]);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("symbolOptions"));
        assert!(symbols(&mut app).is_empty(), "no symbol before OK");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "New Symbol");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("exportType"), "movieClip");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let list = app.run("symbol.list", json!({})).unwrap();
        assert_eq!(list["symbols"][0]["exportType"], "movieClip");
        // The next one is offered a name no symbol has, and OK keeps the name typed and Export Type.
        app.run("shape.rectangle", json!({"x": 60, "y": 10, "width": 30, "height": 20})).unwrap();
        app.run("ui.symbolOptions", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "New Symbol 2");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Badge"));
        app.ui.dialog.as_mut().unwrap().fields.insert("exportType".into(), json!("graphic"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["New Symbol", "Badge"]);
        let list = app.run("symbol.list", json!({})).unwrap();
        assert_eq!(list["symbols"][1]["exportType"], "graphic");
        // Symbol Options on the current symbol renames and keeps Export Type.
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Badge"})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("exportType"), "graphic");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Mark"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["New Symbol", "Mark"]);
        assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][1]["exportType"], "graphic");
    }

    #[test]
    fn an_imported_symbols_thumbnail_frames_its_whole_art() {
        // The symbol of the SVG import tests: an orange 20 × 20 square from (0, 0) with a white dot.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Star" data-name="Gold Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/><circle cx="10" cy="10" r="4" fill="#fff"/></symbol></defs><use href="#Star" x="20" y="30"/></svg>"##;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("document.open", json!({"name": "star.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
        // The tile's thumbnail: 40 × 40 pixels.
        let thumb = out.textures_delta.set.values().flatten().find_map(|d| match &d.image {
            egui::ImageData::Color(img) if img.size == [40, 40] => Some(img.clone()),
            _ => None,
        });
        out.textures_delta.clear();
        let thumb = thumb.expect("the thumbnail");
        // The art fills it, a pixel from each edge: its top-left corner is orange too, not only the
        // quarter the 20 × 20 square around the origin would frame.
        let orange = |x: usize, y: usize| {
            let [r, g, b, a] = thumb.pixels[y * 40 + x].to_array();
            a == 255 && r > 200 && (130..190).contains(&g) && b < 40
        };
        assert!(orange(3, 3) && orange(36, 36), "{:?} {:?}", thumb.pixels[3 * 40 + 3], thumb.pixels[36 * 40 + 36]);
    }
}
