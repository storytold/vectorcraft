//! The Layers panel: tree with visibility/lock columns, layer colour bars, thumbnails, target
//! circles and selection squares; bottom bar with new/delete.
//!
//! A target circle targets its layer, group or object (`layer.target`); dragging it onto another
//! row's circle moves the appearance there (Alt copies it, `appearance.transfer`) and dropping it
//! on the trash clears it. While an opacity mask is edited the panel lists only its art, under
//! an `<Opacity Mask>` entry.

use std::collections::HashSet;

use egui::{Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;
use vectorcraft_doc::{Node, NodeId, NodeKind};

use crate::theme::Tokens;
use crate::widgets::PanelDrag;
use crate::{VectorcraftApp, icons, widgets};

const ROW: f32 = 26.0;

fn expanded_id() -> egui::Id {
    egui::Id::new("layers-expanded")
}

/// The name painted for a row in `lang`: a generated `<Kind>` name is translated, anything else is
/// user data (an unnamed text object shows its text, which can look like `<Path>`; only an empty
/// one is called `<Text>`).
fn painted_name(n: &Node, name: &str, lang: crate::i18n::Lang) -> String {
    let generated = n.name.is_none() && !matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| !r.text.is_empty()));
    if generated && let Some(inner) = name.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        return format!("<{}>", crate::i18n::tr(lang, inner));
    }
    name.to_string()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    // A rename belongs to the document it started in (node ids are per document): drop it when
    // another document shows, so it can't rename that document's node with the same id.
    let shown_doc = egui::Id::new("layers-doc");
    let before = ui.data(|d| d.get_temp::<u64>(shown_doc));
    if before != Some(st.uid) {
        ui.data_mut(|d| {
            if before.is_some() {
                d.remove::<(u64, String)>(egui::Id::new("layers-rename"));
            }
            d.insert_temp(shown_doc, st.uid);
        });
    }
    let doc = st.doc.clone();
    let sel: HashSet<NodeId> = st.selection.objects.iter().copied().collect();
    let target = st.selection.target;
    let current = st.active_layer;
    // While an opacity mask is edited only its art is listed.
    let mask_layer = doc.mask_edit.map(|m| m.layer);
    let mut expanded: HashSet<u64> = ui.data(|d| d.get_temp(expanded_id())).unwrap_or_else(|| doc.layers.iter().map(|l| l.id.0).collect());
    let mut actions: Vec<(String, serde_json::Value)> = vec![];
    // Search field ("Search All").
    crate::widgets::search_field(ui, egui::Id::new("layers-search"), tl!("Search All"));
    ui.add_space(6.0);
    let h = ui.available_height() - 34.0;
    egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for l in doc.layers.iter().rev().filter(|l| mask_layer.is_none_or(|m| m == l.id)) {
            row(ui, &doc, l, 0, false, &sel, target, current, &mut expanded, &mut actions, &t);
        }
    });
    ui.data_mut(|d| d.insert_temp(expanded_id(), expanded));
    if ui.input(|i| i.pointer.any_released()) {
        ui.data_mut(|d| d.remove::<u64>(egui::Id::new("layers-drag")));
    }
    // Bottom bar.
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, t.divider));
    let n = doc.layers.len();
    ui.painter().text(
        bar.left_center() + vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        crate::i18n::tn(n as u64, "{n} Layer", "{n} Layers"),
        egui::FontId::proportional(11.5),
        t.text_dim,
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let trash = widgets::icon_button(&mut child, "trash-2", tl!("Delete Selection"), false, 24.0);
    // A target circle dropped on the trash clears that appearance.
    if let Some(d) = trash.dnd_release_payload::<PanelDrag>()
        && let PanelDrag::Appearance(id) = *d
    {
        actions.push(("appearance.clear".into(), json!({"ids": [id.0]})));
    } else if trash.clicked() {
        if app.session.active().is_some_and(|d| !d.selection.is_empty()) {
            actions.push(("edit.clear".into(), json!({})));
        } else {
            actions.push(("layer.delete".into(), json!({})));
        }
    }
    if widgets::icon_button(&mut child, "file-plus", tl!("Create New Layer"), false, 24.0).clicked() {
        actions.push(("layer.new".into(), json!({})));
    }
    if widgets::icon_button(&mut child, "plus", tl!("Create New Sublayer"), false, 24.0).clicked() {
        actions.push(("layer.newSublayer".into(), json!({})));
    }
    if widgets::icon_button(&mut child, "frame", tl!("Make/Release Clipping Mask"), false, 24.0).clicked() {
        actions.push(("layer.clippingMask.toggle".into(), json!({})));
    }
    if widgets::icon_button(&mut child, "search", tl!("Locate Object"), false, 24.0).clicked() {
        // Expand ancestors of the selection.
        if let Some(st) = app.session.active() {
            let mut ex: HashSet<u64> = ui.data(|d| d.get_temp(expanded_id())).unwrap_or_default();
            for id in &st.selection.objects {
                for a in st.doc.ancestry(*id).unwrap_or_default() {
                    ex.insert(a.0);
                }
            }
            ui.data_mut(|d| d.insert_temp(expanded_id(), ex));
        }
    }
    for (c, p) in actions {
        app.run(&c, p).ok();
    }
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    doc: &vectorcraft_doc::Document,
    n: &Node,
    depth: usize,
    clip_path: bool,
    sel: &HashSet<NodeId>,
    target: Option<NodeId>,
    current: Option<NodeId>,
    expanded: &mut HashSet<u64>,
    actions: &mut Vec<(String, serde_json::Value)>,
    t: &Tokens,
) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click_and_drag());
    let is_sel = sel.contains(&n.id);
    let child_sel = !is_sel
        && n.children().is_some_and(|_| {
            let mut any = false;
            n.walk(&mut |c| any |= c.id != n.id && sel.contains(&c.id));
            any
        });
    let color = {
        let c = doc.layer_color(n.id);
        Color32::from_rgb(c[0], c[1], c[2])
    };
    if n.is_layer() && Some(n.id) == current {
        ui.painter().rect_filled(r, 0.0, t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.6));
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    if n.is_layer() && Some(n.id) == current {
        // Current-layer marker: small triangle in the top-right corner.
        let c = r.right_top();
        ui.painter().add(egui::Shape::convex_polygon(vec![c, c + vec2(-6.0, 0.0), c + vec2(0.0, 6.0)], Color32::from_gray(0xcc), Stroke::NONE));
    }
    // Eye and lock columns.
    let eye = egui::Rect::from_min_size(r.min, vec2(25.0, ROW));
    let lock = egui::Rect::from_min_size(r.min + vec2(25.0, 0.0), vec2(25.0, ROW));
    ui.painter().line_segment([eye.right_top(), eye.right_bottom()], Stroke::new(1.0, t.input_border));
    ui.painter().line_segment([lock.right_top(), lock.right_bottom()], Stroke::new(1.0, t.input_border));
    let er = ui.interact(eye, ui.id().with(("eye", n.id.0)), Sense::click());
    if n.visible {
        icons::paint(ui, "eye", egui::Rect::from_center_size(eye.center(), vec2(14.0, 14.0)), t.icon);
    }
    if er.clicked() {
        let cmd = if n.is_layer() { "layer.setProps" } else { "object.setProps" };
        let key = if n.is_layer() { "id" } else { "ids" };
        let idv = if n.is_layer() { json!(n.id.0) } else { json!([n.id.0]) };
        actions.push((cmd.into(), json!({key: idv, "visible": !n.visible})));
    }
    let lr = ui.interact(lock, ui.id().with(("lock", n.id.0)), Sense::click());
    if n.locked {
        icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.icon);
    } else if lr.hovered() {
        icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.text_disabled);
    }
    if lr.clicked() {
        let cmd = if n.is_layer() { "layer.setProps" } else { "object.setProps" };
        let key = if n.is_layer() { "id" } else { "ids" };
        let idv = if n.is_layer() { json!(n.id.0) } else { json!([n.id.0]) };
        actions.push((cmd.into(), json!({key: idv, "locked": !n.locked})));
    }
    // Layer colour bar.
    let mut x = lock.right() + 2.0;
    if n.is_layer() {
        ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x - 1.0, r.top()), vec2(4.0, ROW)), 0.0, color);
    }
    x += 6.0 + depth as f32 * 14.0;
    // Disclosure.
    let has_children = n.children().is_some_and(|c| !c.is_empty()) && !matches!(n.kind, NodeKind::Compound { .. });
    if has_children {
        let open = expanded.contains(&n.id.0);
        let dr = egui::Rect::from_min_size(egui::pos2(x, r.top() + 5.0), vec2(14.0, 16.0));
        let dresp = ui.interact(dr, ui.id().with(("disc", n.id.0)), Sense::click());
        icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, dr, t.text_dim);
        if dresp.clicked() {
            if open {
                expanded.remove(&n.id.0);
            } else {
                expanded.insert(n.id.0);
            }
        }
    }
    x += 16.0;
    // Thumbnail.
    let th = egui::Rect::from_min_size(egui::pos2(x, r.top() + 2.0), vec2(22.0, 22.0));
    ui.painter().rect_filled(th, 0.0, Color32::WHITE);
    ui.painter().rect_stroke(th, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Outside);
    if !real_thumb(ui, doc, n, th) {
        thumb(ui, n, th);
    }
    x += 26.0;
    // Name.
    let name = if doc.mask_edit.is_some_and(|m| m.layer == n.id) { "<Opacity Mask>".to_string() } else { n.display_name() };
    let font = egui::FontId::proportional(13.0);
    let rename_id = egui::Id::new("layers-rename");
    let renaming: Option<(u64, String)> = ui.data(|d| d.get_temp(rename_id));
    let name_rect = egui::Rect::from_min_max(egui::pos2(x - 2.0, r.top() + 3.0), egui::pos2(r.right() - 44.0, r.bottom() - 3.0));
    match renaming {
        Some((rid, mut buf)) if rid == n.id.0 => {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
            let te = child.add(egui::TextEdit::singleline(&mut buf).desired_width(name_rect.width()).font(font.clone()));
            // Focus the field as it opens. Asking every frame took the focus back from Enter,
            // Escape or a click elsewhere, so the rename never ended.
            if !te.has_focus() && !te.lost_focus() {
                te.request_focus();
            }
            if te.lost_focus() {
                ui.data_mut(|d| d.remove::<(u64, String)>(rename_id));
                if child.input(|i| !i.key_pressed(egui::Key::Escape)) && buf != name {
                    let cmd = if n.is_layer() { "layer.setProps" } else { "object.setProps" };
                    let key = if n.is_layer() { "id" } else { "ids" };
                    let idv = if n.is_layer() { json!(n.id.0) } else { json!([n.id.0]) };
                    actions.push((cmd.into(), json!({key: idv, "name": buf})));
                }
            } else {
                ui.data_mut(|d| d.insert_temp(rename_id, (n.id.0, buf)));
            }
        }
        _ => {
            let painter = ui.painter().with_clip_rect(name_rect);
            // Generated names ("<Path>", "<Opacity Mask>") are translated where painted; the stored name stays English.
            let shown = if doc.mask_edit.is_some_and(|m| m.layer == n.id) {
                tl!("<Opacity Mask>").to_string()
            } else {
                painted_name(n, &name, crate::i18n::current())
            };
            let text = painter.text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, shown, font, t.text);
            // A clipping path's name is underlined, a masked object's with a dashed line.
            if clip_path {
                painter.line_segment([text.left_bottom(), text.right_bottom()], Stroke::new(1.0, t.text));
            } else if n.mask.is_some() {
                painter.extend(egui::Shape::dashed_line(&[text.left_bottom(), text.right_bottom()], Stroke::new(1.0, t.text), 3.0, 2.0));
            }
        }
    }
    // Target circle and selection square.
    let col_x = r.right() - 43.5;
    ui.painter().line_segment([egui::pos2(col_x, r.top()), egui::pos2(col_x, r.bottom())], Stroke::new(1.0, t.input_border));
    let tc = egui::pos2(r.right() - 28.0, r.center().y);
    let tresp = target_circle(ui, n, tc, target.map_or(is_sel, |id| id == n.id), actions, t);
    let sq = egui::pos2(r.right() - 11.0, r.center().y);
    if is_sel {
        let q = egui::Rect::from_center_size(sq, vec2(7.0, 7.0));
        ui.painter().rect_filled(q, 0.0, color);
        ui.painter().rect_stroke(q, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
    } else if child_sel {
        ui.painter().rect_filled(egui::Rect::from_center_size(sq, vec2(4.0, 4.0)), 0.0, color);
    }
    let sq_resp = ui.interact(egui::Rect::from_center_size(sq, vec2(18.0, ROW)), ui.id().with(("selsq", n.id.0)), Sense::click());
    if tresp.clicked() {
        actions.push(("layer.target".into(), json!({"id": n.id.0})));
    } else if sq_resp.clicked() {
        if n.is_layer() {
            actions.push(("layer.selectAll".into(), json!({"id": n.id.0})));
        } else if ui.input(|i| i.modifiers.shift) {
            actions.push(("select.toggle".into(), json!({"id": n.id.0})));
        } else {
            actions.push(("select.set".into(), json!({"ids": [n.id.0]})));
        }
    } else if resp.clicked() {
        if n.is_layer() {
            actions.push(("layer.setCurrent".into(), json!({"id": n.id.0})));
        } else {
            actions.push(("select.set".into(), json!({"ids": [n.id.0]})));
        }
    }
    if resp.double_clicked() {
        ui.data_mut(|d| d.insert_temp(egui::Id::new("layers-rename"), (n.id.0, n.display_name())));
    }
    // Drag to reorder: drop onto a row moves the dragged node above or below it (into its parent).
    // A layer's or group's row takes a drop on its middle half into it, on top of its contents.
    let drag_id = egui::Id::new("layers-drag");
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(drag_id, n.id.0));
    }
    let dragging: Option<u64> = ui.data(|d| d.get_temp(drag_id));
    if let Some(src) = dragging
        && src != n.id.0
        && ui.rect_contains_pointer(r)
    {
        let src_is_layer = doc.node(vectorcraft_doc::NodeId(src)).is_some_and(|x| x.is_layer());
        // Layers only go into layers.
        let container = n.is_layer() || (matches!(n.kind, NodeKind::Group { .. }) && !src_is_layer);
        let y = ui.input(|i| i.pointer.hover_pos()).map_or(r.center().y, |p| p.y);
        let into = container && (y - r.center().y).abs() < r.height() / 4.0;
        let above = y < r.center().y;
        if into {
            ui.painter().rect_stroke(r.shrink(1.0), 2.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        } else {
            let y = if above { r.top() } else { r.bottom() };
            ui.painter().line_segment([egui::pos2(r.left() + 46.0, y), egui::pos2(r.right(), y)], Stroke::new(2.0, t.accent));
        }
        if ui.input(|i| i.pointer.any_released()) {
            let (parent, index) = match doc.position(n.id) {
                _ if into => (Some(n.id), n.children().map_or(0, |c| c.len())),
                Some((par, idx, _)) => (par, if above { idx + 1 } else { idx }),
                None => (None, 0),
            };
            // A layer dropped on a non-layer goes into that row's container; top level only for layers.
            if parent.is_some() || src_is_layer {
                actions.push(("node.move".into(), json!({"id": src, "parent": parent.map(|p| p.0), "index": index})));
            }
            ui.data_mut(|d| d.remove::<u64>(drag_id));
        }
    }

    if has_children
        && expanded.contains(&n.id.0)
        && let Some(children) = n.children()
    {
        for (i, c) in children.iter().enumerate().rev() {
            row(ui, doc, c, depth + 1, i == 0 && n.clips(), sel, target, current, expanded, actions, t);
        }
    }
}

/// The target circle of `n`'s row at `c`: a ring, doubled while `n` is targeted, filled when `n`
/// has an appearance or transparency of its own. Its response is clicked to target `n`; dragging
/// it carries `n`'s appearance ([`PanelDrag::Appearance`]) onto another row's circle (Alt
/// copies it) or the trash.
fn target_circle(ui: &mut Ui, n: &Node, c: egui::Pos2, targeted: bool, actions: &mut Vec<(String, serde_json::Value)>, t: &Tokens) -> egui::Response {
    let resp = ui.interact(egui::Rect::from_center_size(c, vec2(16.0, ROW)), ui.id().with(("target", n.id.0)), Sense::click_and_drag());
    widgets::drag_source(ui, &resp, || PanelDrag::Appearance(n.id));
    let held = resp.dnd_hover_payload::<PanelDrag>().is_some_and(|d| matches!(*d, PanelDrag::Appearance(id) if id != n.id));
    if let Some(d) = resp.dnd_release_payload::<PanelDrag>()
        && let PanelDrag::Appearance(source) = *d
        && source != n.id
    {
        let copy = ui.input(|i| i.modifiers.alt);
        actions.push(("appearance.transfer".into(), json!({"source": source.0, "target": n.id.0, "copy": copy})));
    }
    let ring = if held { t.accent } else { t.icon };
    ui.painter().circle_stroke(c, 5.0, Stroke::new(if held { 1.5 } else { 1.0 }, ring));
    if targeted {
        ui.painter().circle_stroke(c, 2.8, Stroke::new(1.0, t.icon));
    }
    if has_styled_target(n) {
        ui.painter().circle_filled(c, 3.2, t.icon);
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    resp
}

/// Whether an object's target circle is filled: its appearance is not basic (a group or layer:
/// it has fills, strokes or effects of its own) or its transparency (opacity, blend mode,
/// isolation, knockout or an opacity mask) is not the default.
fn has_styled_target(n: &Node) -> bool {
    !n.appearance.is_basic()
        || (matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && !n.appearance.items.is_empty())
        || !n.has_default_transparency()
}

/// A real rendered thumbnail, cached by node identity (unchanged nodes keep their `Arc`
/// allocation, so the address is a free change detector). Only rendered for visible rows.
pub(crate) fn real_thumb(ui: &Ui, doc: &vectorcraft_doc::Document, n: &Node, r: egui::Rect) -> bool {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: RefCell<HashMap<(usize, u64), egui::TextureHandle>> = RefCell::new(HashMap::new());
    }
    if !ui.is_rect_visible(r) {
        return true;
    }
    let key = (n as *const Node as usize, n.id.0);
    let ppp = ui.ctx().pixels_per_point();
    let px = (r.width() * ppp).round() as u32;
    let tex = CACHE.with(|c| c.borrow().get(&key).cloned()).or_else(|| {
        let img = RENDERER.with(|rr| rr.borrow_mut().render_thumbnail(doc, n.id, px.max(8)))?;
        let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
        let tex = ui.ctx().load_texture(format!("layer-thumb-{}", n.id.0), color, egui::TextureOptions::LINEAR);
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 2000 {
                c.clear();
            }
            c.insert(key, tex.clone());
        });
        Some(tex)
    });
    match tex {
        Some(t) => {
            ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            true
        }
        None => false,
    }
}

/// Tiny vector thumbnail painted with egui (fallback when rendering isn't possible).
fn thumb(ui: &Ui, n: &Node, r: egui::Rect) {
    let Some(b) = n.visual_bounds() else { return };
    let s = ((r.width() - 3.0) as f64 / b.width().max(b.height()).max(1e-6)) as f32;
    let off = r.center() - vec2(b.center().x as f32 * s, b.center().y as f32 * s);
    let p = ui.painter().with_clip_rect(r);
    let mut count = 0;
    n.walk(&mut |c| {
        if count > 40 {
            return;
        }
        if let Some(pd) = c.path_data() {
            count += 1;
            let col = c.appearance.fill_paint().color().or(c.appearance.stroke_paint().color()).map(|cc| {
                let [a, b2, d, _] = cc.to_rgba8(1.0);
                Color32::from_rgb(a, b2, d)
            });
            if let Some(bb) = pd.bounds() {
                let rr = egui::Rect::from_min_max(off + vec2(bb.x0 as f32 * s, bb.y0 as f32 * s), off + vec2(bb.x1 as f32 * s, bb.y1 as f32 * s));
                p.rect_filled(rr, 0.0, col.unwrap_or(Color32::from_gray(120)));
            }
        }
    });
}

/// The Layers panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    const ITEMS: [(&str, &str); 6] = [
        ("New Layer…", "layer.new"),
        ("New Sublayer…", "layer.newSublayer"),
        ("Duplicate Layer", "layer.duplicate"),
        ("Delete Layer", "layer.delete"),
        ("Make/Release Clipping Mask", "layer.clippingMask.toggle"),
        ("Collect in New Layer", "layer.collectInNew"),
    ];
    for (i, (label, cmd)) in ITEMS.into_iter().enumerate() {
        if i == 4 {
            ui.separator();
        }
        if widgets::menu_item(ui, tl!(label), crate::menus::enabled(app, cmd), false) {
            app.run(cmd, json!({})).ok();
        }
    }
    ui.separator();
    let remembers = app.session.active().is_some_and(|d| d.doc.paste_remembers_layers);
    if widgets::menu_item(ui, tl!("Paste Remembers Layers"), app.session.active().is_some(), remembers) {
        app.run("layer.pasteRemembersLayers", json!({"on": !remembers})).ok();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    /// Filled target circles drawn by one headless frame of the panel.
    fn filled_targets(app: &mut VectorcraftApp, ctx: &egui::Context) -> usize {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Circle(cs) if cs.radius == 3.2 && cs.fill != Color32::TRANSPARENT)).count()
    }

    /// A generated `<Kind>` name is translated where painted; an unnamed text object's text never
    /// is, even when it reads like one.
    #[test]
    fn only_generated_names_are_translated() {
        let zh = crate::i18n::Lang::from_code("zh-hant").unwrap();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let rect = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        let text = run(&mut app, "text.create", json!({"x": 10, "y": 40, "text": "<Path>"}))["id"].as_u64().unwrap();
        let doc = &app.session.active().unwrap().doc;
        let names = |id: u64| {
            let n = doc.node(NodeId(id)).unwrap();
            (n.display_name(), painted_name(n, &n.display_name(), zh))
        };
        assert_eq!(names(text), ("<Path>".to_string(), "<Path>".to_string()));
        let (stored, painted) = names(rect);
        let inner = stored.trim_start_matches('<').trim_end_matches('>');
        assert_eq!(painted, format!("<{}>", crate::i18n::tr(zh, inner)));
        assert_ne!(painted, stored, "translated");
    }

    #[test]
    fn target_circle_fills_for_non_default_transparency() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let id = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
        run(&mut app, "select.set", json!({ "ids": [id] }));
        let ctx = egui::Context::default();
        assert_eq!(filled_targets(&mut app, &ctx), 0);
        run(&mut app, "transparency.set", json!({"blend": "multiply"}));
        assert_eq!(filled_targets(&mut app, &ctx), 1, "a Multiply object");
        run(&mut app, "transparency.set", json!({"blend": "normal", "knockout": true}));
        assert_eq!(filled_targets(&mut app, &ctx), 1, "a knockout group");
        run(&mut app, "transparency.set", json!({"knockout": false}));
        run(&mut app, "appearance.addStroke", json!({}));
        assert_eq!(filled_targets(&mut app, &ctx), 1, "two strokes");
    }

    #[test]
    fn a_layer_clipping_mask_underlines_its_clipping_path() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 25, "y": 25, "width": 50, "height": 50}));
        run(&mut app, "select.none", json!({}));
        let ctx = egui::Context::default();
        let text = Tokens::get(&ctx).text;
        // Line segments in the text colour: the underlines.
        let underlines = |app: &mut VectorcraftApp| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == text)).count()
        };
        assert_eq!(underlines(&mut app), 0);
        assert_eq!(run(&mut app, "layer.clippingMask.toggle", json!({}))["clip"], true);
        assert_eq!(underlines(&mut app), 1);
        run(&mut app, "layer.clippingMask.toggle", json!({}));
        assert_eq!(underlines(&mut app), 0);
    }

    /// One headless frame of the panel with `events` (Alt held when `alt`): the centres of the
    /// target circles, top row first, and the text-coloured line segments (underlines) drawn.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, alt: bool) -> (Vec<egui::Pos2>, usize) {
        let mut events = events;
        events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers { alt, ..Default::default() }));
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        let text = Tokens::get(ctx).text;
        let mut circles = vec![];
        let mut lines = 0;
        for c in &out.shapes {
            match &c.shape {
                egui::Shape::Circle(cs) if cs.radius == 5.0 => circles.push(cs.center),
                egui::Shape::LineSegment { stroke, .. } if stroke.color == text => lines += 1,
                _ => {}
            }
        }
        circles.sort_by(|a, b| a.y.total_cmp(&b.y));
        (circles, lines)
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// Drag from `a` to `b` over a few frames (Alt held when `alt`).
    fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, a: egui::Pos2, b: egui::Pos2, alt: bool) {
        let steps = [
            vec![egui::Event::PointerMoved(a)],
            vec![button(a, true)],
            vec![egui::Event::PointerMoved(a + vec2(0.0, 6.0))],
            vec![egui::Event::PointerMoved(b)],
            vec![button(b, false)],
            vec![],
        ];
        for e in steps {
            frame(app, ctx, e, alt);
        }
    }

    /// A document with two rectangles on one layer → (app, layer, [bottom, top]).
    fn two_rects() -> (VectorcraftApp, NodeId, [NodeId; 2]) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 200, "height": 200}));
        let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        let b = run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        run(&mut app, "select.none", json!({}));
        let layer = app.session.active().unwrap().doc.layers[0].id;
        (app, layer, [NodeId(a), NodeId(b)])
    }

    /// A row dropped on the middle of a layer's row goes into that layer; on its top or bottom
    /// quarter it goes above or below it, as before.
    #[test]
    fn dropping_a_row_on_the_middle_of_a_layer_moves_it_into_the_layer() {
        // Rows top down: Layer 2 (empty), Layer 1, its top rectangle, its bottom one.
        let setup = || {
            let (mut app, layer1, rects) = two_rects();
            let layer2 = NodeId(app.session.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
            (app, layer1, layer2, rects)
        };
        // Away from the row's buttons: on its name.
        let name = |c: egui::Pos2, dy: f32| egui::pos2(c.x - 120.0, c.y + dy);
        let ctx = egui::Context::default();
        let (mut app, _, layer2, [_, b]) = setup();
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(c.len(), 4);
        drag(&mut app, &ctx, name(c[2], 0.0), name(c[0], 0.0), false);
        assert_eq!(app.session.active().unwrap().doc.parent_of(b), Some(layer2), "the rectangle went into the empty layer");
        // A layer dropped on another's middle becomes its sublayer.
        let (mut app, layer1, layer2, _) = setup();
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        drag(&mut app, &ctx, name(c[0], 0.0), name(c[1], 0.0), false);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.layers.len(), doc.parent_of(layer2)), (1, Some(layer1)));
        // On its bottom quarter, it goes below it.
        let (mut app, layer1, layer2, _) = setup();
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        drag(&mut app, &ctx, name(c[0], 0.0), name(c[1], ROW / 2.0 - 3.0), false);
        let ids: Vec<NodeId> = app.session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
        assert_eq!(ids, vec![layer2, layer1], "Layer 2 below Layer 1, both top-level");
    }

    #[test]
    fn clicking_a_target_circle_targets_the_layer() {
        let (mut app, layer, [a, b]) = two_rects();
        let ctx = egui::Context::default();
        // Rows top down: the layer, the top rectangle, the bottom one.
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(c.len(), 3);
        for e in [egui::Event::PointerMoved(c[0]), button(c[0], true), button(c[0], false)] {
            frame(&mut app, &ctx, vec![e], false);
        }
        let st = app.session.active().unwrap();
        assert_eq!(st.selection.target, Some(layer));
        assert_eq!(st.selection.objects, vec![a, b]);
        assert_eq!(super::super::appearance::object_label(&app), "Layer", "the Appearance panel lists the layer");
        app.session.execute("transparency.set", &json!({"opacity": 25})).unwrap();
        assert!((app.session.active().unwrap().doc.node(layer).unwrap().opacity - 0.25).abs() < 1e-6);
        // The layer's circle is filled now; clicking an object's circle targets that object.
        assert_eq!(filled_targets(&mut app, &ctx), 1);
        for e in [egui::Event::PointerMoved(c[2]), button(c[2], true), button(c[2], false)] {
            frame(&mut app, &ctx, vec![e], false);
        }
        let st = app.session.active().unwrap();
        assert_eq!((st.selection.target, st.selection.objects.clone()), (Some(a), vec![a]));
    }

    #[test]
    fn dragging_a_target_circle_moves_or_copies_the_appearance_and_the_trash_clears_it() {
        let (mut app, layer, [a, b]) = two_rects();
        app.session.execute("transparency.set", &json!({"ids": [a.0], "opacity": 30})).unwrap();
        let ctx = egui::Context::default();
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        let opacity = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.node(id).unwrap().opacity;
        // Alt-drag the bottom rectangle's circle onto the top one's: both have it.
        drag(&mut app, &ctx, c[2], c[1], true);
        assert!((opacity(&app, b) - 0.3).abs() < 1e-6 && (opacity(&app, a) - 0.3).abs() < 1e-6);
        // A plain drag onto the layer's circle moves it there.
        drag(&mut app, &ctx, c[1], c[0], false);
        assert!((opacity(&app, layer) - 0.3).abs() < 1e-6 && opacity(&app, b) == 1.0);
        // Dropped on the trash, the layer's appearance is cleared (and nothing is deleted).
        let st = app.session.active().unwrap();
        let count = st.doc.node_count();
        let trash = {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
            out.textures_delta.clear();
            // The bottom bar's rightmost button, under the right end of its top divider.
            let divider = Tokens::get(&ctx).divider;
            let corner = out.shapes.iter().rev().find_map(|c| match &c.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == divider => Some(points[1]),
                _ => None,
            });
            corner.unwrap() + vec2(-12.0, 15.0)
        };
        drag(&mut app, &ctx, c[0], trash, false);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.node(layer).unwrap().opacity, 1.0);
        assert_eq!(st.doc.node_count(), count);
    }

    #[test]
    fn renaming_a_layer_commits_on_enter_and_escape_cancels() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let layer = app.session.doc().unwrap().doc.layers[0].id;
        let name = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].display_name();
        let ctx = egui::Context::default();
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let select_all =
            egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        // What a double-click on the row does: the name field opens with the name.
        let open = |app: &mut VectorcraftApp| {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("layers-rename"), (layer.0, name(app))));
            frame(app, &ctx, vec![], false);
            frame(app, &ctx, vec![], false);
        };
        open(&mut app);
        frame(&mut app, &ctx, vec![select_all.clone(), egui::Event::Text("Sky".into())], false);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)], false);
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(name(&app), "Sky", "Enter keeps the new name");
        assert!(ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::new("layers-rename"))).is_none(), "and closes the field");
        open(&mut app);
        frame(&mut app, &ctx, vec![select_all, egui::Event::Text("Ground".into())], false);
        frame(&mut app, &ctx, vec![key(egui::Key::Escape)], false);
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(name(&app), "Sky", "Escape cancels");
    }

    #[test]
    fn an_open_rename_does_not_follow_into_another_document() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let layer = app.session.doc().unwrap().doc.layers[0].id;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![], false);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("layers-rename"), (layer.0, "Renamed in the first".to_string())));
        frame(&mut app, &ctx, vec![], false);
        // A second document, whose first layer has the same node id, comes to the front.
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.layers[0].id, layer);
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&mut app, &ctx, vec![], false);
        frame(&mut app, &ctx, vec![enter], false);
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(app.session.doc().unwrap().doc.layers[0].display_name(), "Layer 1");
        assert!(ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::new("layers-rename"))).is_none());
    }

    #[test]
    fn masks_underline_dashed_and_mask_editing_lists_only_the_mask() {
        let (mut app, _, [a, _]) = two_rects();
        let ctx = egui::Context::default();
        let (_, before) = frame(&mut app, &ctx, vec![], false);
        app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        app.session.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        // Editing the new (empty) mask: one row, `<Opacity Mask>`.
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(c.len(), 1);
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
        assert!(texts.iter().any(|(t, _)| t == "<Opacity Mask>"), "{texts:?}");
        app.session.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        let (c, after) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(c.len(), 3);
        assert!(after > before + 1, "a dashed underline: {before} → {after}");
    }
}
