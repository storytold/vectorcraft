use super::controls::collect_for_export_menu;
use super::model::{Place, Row, opens, painted_name};
use super::thumbs::{node_arc, real_thumb, thumb};
use super::{
    Arc, COLUMN, Color32, ColumnDrag, Document, INDENT, LayersDrag, Node, NodeId, NodeKind, Out, PanelDrag, SQUARE_X, Sense, Stroke, StrokeKind,
    TARGET_X, Tokens, Ui, Value, View, icons, json, vec2, widgets,
};

/// Stable id for the layers panel's per-name UI state.
pub(super) fn key(name: &str) -> egui::Id {
    egui::Id::unique(("layers-panel", name))
}

/// Temp-memory key for the layer rename editor.
pub(super) fn rename_id() -> egui::Id {
    egui::Id::unique("layers-rename")
}

/// Ghost rows are drawn at this opacity.
const GHOST_OPACITY: f32 = 0.5;

/// The colour of the layer holding `id`.
fn layer_colour(doc: &Document, id: NodeId) -> Color32 {
    let [r, g, b] = doc.layer_color(id);
    Color32::from_rgb(r, g, b)
}

/// The layer colour bar of row `n` (rect `r`) at `x`: wider on layers.
fn colour_bar(ui: &Ui, n: &Node, r: egui::Rect, x: f32, colour: Color32) {
    let w = if n.is_layer() { 4.0 } else { 2.0 };
    ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x - 1.0, r.top()), vec2(w, r.height())), 0.0, colour);
}

/// The thumbnail of row `n` (rect `r`) at `x`; returns where it went.
fn row_thumb(ui: &Ui, view: &View, n: &Arc<Node>, r: egui::Rect, x: f32) -> egui::Rect {
    let s = (view.h - 4.0).min(40.0);
    let th = egui::Rect::from_min_size(egui::pos2(x, r.center().y - s / 2.0), vec2(s, s));
    ui.painter().rect_filled(th, 0.0, Color32::WHITE);
    ui.painter().rect_stroke(th, 0.0, Stroke::new(1.0, view.t.border), StrokeKind::Outside);
    if !real_thumb(ui, &view.thumbs, n, th) {
        thumb(ui, n, th);
    }
    th
}

/// The dragged row `src`, dimmed, following the pointer at `pos` above everything else: its
/// colour bar, thumbnail and name.
pub(super) fn drag_ghost(ui: &Ui, view: &View, src: NodeId, pos: egui::Pos2) {
    let (doc, t, h) = (view.doc, &view.t, view.h);
    let Some(n) = node_arc(doc, src) else { return };
    let n = &n;
    let (grab, width, depth) = ui.data(|d| d.get_temp::<(egui::Vec2, f32, usize)>(key("grab"))).unwrap_or((vec2(h, h / 2.0), 240.0, 0));
    // A plain Ui on the tooltip layer, not an Area: a new Area is invisible for a frame and fades in.
    let r = egui::Rect::from_min_size(pos - grab, vec2(width, h));
    let id = key("ghost");
    let mut ui = Ui::new(ui.ctx().clone(), id, egui::UiBuilder::new().layer_id(egui::LayerId::new(egui::Order::Tooltip, id)).max_rect(r));
    ui.set_opacity(GHOST_OPACITY);
    let p = ui.painter();
    p.rect_filled(r, 0.0, t.row_selected);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    let mut x = r.left() + 2.0 * COLUMN + 2.0;
    colour_bar(&ui, n, r, x, layer_colour(doc, src));
    x += 22.0 + depth as f32 * INDENT;
    if view.opts.thumb(n) {
        x = row_thumb(&ui, view, n, r, x).right() + 4.0;
    }
    let label = n.identify_label(view.identify_xml_id);
    let name = if view.identify_xml_id { label } else { painted_name(n, &label, crate::i18n::current()) };
    ui.painter().text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), t.text);
}

/// Resolve the row clicked this frame: plain (highlight it), Ctrl/Cmd (toggle it) or Shift
/// (highlight the rows from the last one clicked down to it).
pub(super) fn resolve_click(ui: &Ui, view: &View, list: &[Row], out: &mut Out) {
    let Some((id, m)) = out.click.take() else { return };
    let anchor: Option<NodeId> = ui.data(|d| d.get_temp(key("anchor")));
    ui.data_mut(|d| d.insert_temp(key("focus"), true));
    let at = |id: NodeId| list.iter().position(|r| r.node.id == id);
    if m.shift
        && let Some(a) = anchor.or(view.rows.last().copied())
        && let (Some(i), Some(j)) = (at(a), at(id))
    {
        let range: Vec<u64> = list.get(i.min(j)..=i.max(j)).unwrap_or_default().iter().map(|r| r.node.id.0).collect();
        out.actions.push(("layer.highlight".into(), json!({"ids": range, "mode": if m.command { "add" } else { "set" }})));
        return;
    }
    ui.data_mut(|d| d.insert_temp(key("anchor"), id));
    if m.command {
        out.actions.push(("layer.highlight".into(), json!({"ids": [id.0], "mode": "toggle"})));
    } else {
        out.actions.push(("layer.setCurrent".into(), json!({"id": id.0})));
    }
}

/// Whether row `n` can take `drag` inside it.
fn takes(n: &Node, drag: &LayersDrag, doc: &Document) -> bool {
    match n.kind {
        NodeKind::Layer { .. } => true,
        NodeKind::Group { .. } => match drag {
            LayersDrag::Rows(ids) => !ids.iter().any(|i| doc.node(*i).is_some_and(Node::is_layer)),
            LayersDrag::Art => true,
            LayersDrag::Guides(_) => false,
        },
        _ => false,
    }
}

/// The row of ruler guide `i` on the layer of `item`: a guide mark and its name, the selection
/// square when it is selected. A click selects the guide; Shift or Cmd adds it to (or takes it
/// from) the selected guides. A guide on a hidden or locked layer can't be selected or dragged.
fn guide_row(ui: &mut Ui, view: &View, item: &Row, i: usize, out: &mut Out) {
    let (t, h) = (&view.t, view.h);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
    let Some(g) = view.doc.guides.get(i) else { return };
    let selected = view.guides.contains(&i);
    let editable = view.doc.guide_editable(g);
    if editable && resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.6));
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    let mut x = r.left() + 2.0 * COLUMN + 2.0;
    colour_bar(ui, &item.node, r, x, item.colour);
    x += 6.0 + item.depth as f32 * INDENT + 16.0;
    // The guide mark: a short line across or down, in the guides' colour.
    let c = egui::pos2(x + 8.0, r.center().y);
    let half = if g.vertical { vec2(0.0, 6.0) } else { vec2(6.0, 0.0) };
    ui.painter().line_segment([c - half, c + half], Stroke::new(1.5, t.accent));
    x += 22.0;
    let name = format!("<{}>", if g.vertical { tl!("Vertical Guide") } else { tl!("Horizontal Guide") });
    let color = if item.dim { t.text_dim } else { t.text };
    ui.painter().text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), color);
    let col_x = r.right() - 43.5;
    ui.painter().line_segment([egui::pos2(col_x, r.top()), egui::pos2(col_x, r.bottom())], Stroke::new(1.0, t.input_border));
    if selected {
        let q = egui::Rect::from_center_size(egui::pos2(r.right() - SQUARE_X, r.center().y), vec2(7.0, 7.0));
        ui.painter().rect_filled(q, 0.0, item.colour);
        ui.painter().rect_stroke(q, 0.0, Stroke::new(1.0, t.text), StrokeKind::Inside);
    }
    if editable && resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        out.actions.push(("guide.select".into(), json!({"indexes": [i], "toggle": m.shift || m.command})));
    }
    // Dragged: the selected guides when this is one of them, else this one, to another layer's row.
    if editable && resp.drag_started() {
        let guides = if selected { view.guides.clone() } else { vec![i] };
        egui::DragAndDrop::set_payload(ui.ctx(), LayersDrag::Guides(guides));
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    // Guides dropped here go to this guide's layer.
    if let Some(drag) = resp.dnd_hover_payload::<LayersDrag>()
        && let LayersDrag::Guides(guides) = &*drag
    {
        guide_drop(ui, view, guides, item.node.id, r, &resp, out);
    }
}

/// Guide rows `guides` dragged over row `r` (`resp`) of layer `layer`: the row lights up, and the
/// release puts them on that layer (`guide.setLayer`, one undo step).
fn guide_drop(ui: &Ui, view: &View, guides: &[usize], layer: NodeId, r: egui::Rect, resp: &egui::Response, out: &mut Out) {
    ui.painter().rect_stroke(r.shrink(1.0), 0.0, Stroke::new(2.0, view.t.accent), StrokeKind::Inside);
    if resp.dnd_release_payload::<LayersDrag>().is_some() {
        out.actions.push(("guide.setLayer".into(), json!({"indexes": guides, "layer": layer.0})));
    }
}

/// The row's fill (selected, current or hovered), its bottom edge and the current layer's marker.
fn row_background(ui: &Ui, view: &View, n: &Node, r: egui::Rect, resp: &egui::Response) {
    let t = &view.t;
    // Highlighted rows (else the current layer's).
    let highlighted = if view.rows.is_empty() { n.is_layer() && Some(n.id) == view.current } else { view.rows.contains(&n.id) };
    if highlighted {
        ui.painter().rect_filled(r, 0.0, t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.6));
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    if n.is_layer() && Some(n.id) == view.current {
        // Current-layer marker: a small triangle in the top-right corner.
        let c = r.right_top();
        ui.painter().add(egui::Shape::convex_polygon(vec![c, c + vec2(-6.0, 0.0), c + vec2(0.0, 6.0)], t.text, Stroke::NONE));
    }
}

/// The disclosure triangle of a row at `(r, x)`; Alt opens or closes everything inside too.
fn disclosure(ui: &Ui, view: &View, n: &Node, (r, x): (egui::Rect, f32), open: bool, out: &mut Out) {
    let t = &view.t;
    let dr = egui::Rect::from_min_size(egui::pos2(x, r.center().y - 8.0), vec2(14.0, 16.0));
    let dresp = ui.interact(dr, ui.scope_id().with(("disc", n.id.0)), Sense::click());
    icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, dr, t.text_dim);
    if dresp.clicked() {
        out.toggle.push((n.id, !open));
        if ui.input(|i| i.modifiers.alt) {
            n.walk(&mut |c| {
                if c.id != n.id && opens(c, &view.opts) {
                    out.toggle.push((c.id, !open));
                }
            });
        }
    }
}

/// The widgets of row `item`.
pub(super) fn row(ui: &mut Ui, view: &View, item: &Row, out: &mut Out) {
    if let Some(i) = item.guide {
        return guide_row(ui, view, item, i, out);
    }
    let n = &*item.node;
    let (depth, clip_path, has_children, open, color) = (item.depth, item.clip_path, item.opens, item.open, item.colour);
    let doc = view.doc;
    let t = &view.t;
    let h = view.h;
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
    if view.reveal == Some(n.id) {
        ui.scroll_to_rect(r, None);
    }
    let is_sel = view.sel.contains(&n.id);
    let child_sel = !is_sel
        && n.children().is_some_and(|_| {
            let mut any = false;
            n.walk(&mut |c| any |= c.id != n.id && view.sel.contains(&c.id));
            any
        });
    row_background(ui, view, n, r, &resp);
    eye_and_lock(ui, view, n, r, out);
    // Layer colour bar (every row shows the colour of its layer).
    let mut x = r.left() + 2.0 * COLUMN + 2.0;
    colour_bar(ui, n, r, x, color);
    x += 6.0 + depth as f32 * INDENT;
    if has_children {
        disclosure(ui, view, n, (r, x), open, out);
    }
    x += 16.0;
    // Thumbnail.
    let thumb_rect = view.opts.thumb(n).then(|| {
        let th = row_thumb(ui, view, &item.node, r, x);
        x = th.right() + 4.0;
        th
    });
    // Name (Preferences › Units › Identify Objects By: Object Name or XML ID).
    let xml_id = view.identify_xml_id;
    let name = if doc.mask_edit.is_some_and(|m| m.layer == n.id) { "<Opacity Mask>".to_string() } else { n.identify_label(xml_id) };
    let font = egui::FontId::proportional(13.0);
    let renaming: Option<(u64, String)> = ui.data(|d| d.get_temp(rename_id()));
    let name_rect = egui::Rect::from_min_max(egui::pos2(x - 2.0, r.center().y - 10.0), egui::pos2(r.right() - 44.0, r.center().y + 10.0));
    let name_end;
    match renaming {
        Some((rid, mut buf)) if rid == n.id.0 => {
            let field = rename_id().with("field");
            let kept = crate::widgets::own_field_shift_enter(ui, field, &mut buf, &name);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
            let te = child.add(
                egui::TextEdit::singleline(&mut buf).id(field).align(egui::Align2::LEFT_CENTER).desired_width(name_rect.width()).font(font.clone()),
            );
            // Focus the field as it opens. Asking every frame took the focus back from Enter,
            // Escape or a click elsewhere, so the rename never ended.
            if !te.has_focus() && !te.lost_focus() {
                te.request_focus();
            }
            if te.lost_focus() {
                ui.data_mut(|d| d.remove::<(u64, String)>(rename_id()));
                if child.input(|i| !i.key_pressed(egui::Key::Escape)) && buf != name {
                    out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "name": buf})));
                }
            } else {
                // Shift+Enter renames it and keeps the field open.
                if kept && buf != name {
                    out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "name": buf.clone()})));
                }
                ui.data_mut(|d| d.insert_temp(rename_id(), (n.id.0, buf)));
            }
            name_end = name_rect.right();
        }
        _ => {
            let painter = ui.painter().with_clip_rect(name_rect);
            // Generated names ("<Path>", "<Opacity Mask>") are translated where painted; the stored name stays English.
            // XML IDs stay as ids (not translated).
            let shown = if doc.mask_edit.is_some_and(|m| m.layer == n.id) {
                tl!("<Opacity Mask>").to_string()
            } else if xml_id {
                name.clone()
            } else {
                painted_name(n, &n.display_name(), crate::i18n::current())
            };
            let text = painter.text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, shown, font, if item.dim { t.text_dim } else { t.text });
            name_end = text.right();
            // A clipping path's name is underlined, a masked object's with a dashed line.
            if clip_path {
                painter.line_segment([text.left_bottom(), text.right_bottom()], Stroke::new(1.0, t.text));
            } else if n.extras().mask.is_some() {
                painter.extend(egui::Shape::dashed_line(&[text.left_bottom(), text.right_bottom()], Stroke::new(1.0, t.text), 3.0, 2.0));
            }
        }
    }
    // Target circle and selection square.
    let col_x = r.right() - 43.5;
    ui.painter().line_segment([egui::pos2(col_x, r.top()), egui::pos2(col_x, r.bottom())], Stroke::new(1.0, t.input_border));
    let tc = egui::pos2(r.right() - TARGET_X, r.center().y);
    let tresp = target_circle(ui, n, tc, h, view.target.map_or(is_sel, |id| id == n.id), &mut out.actions, t);
    let sq = egui::pos2(r.right() - SQUARE_X, r.center().y);
    if is_sel {
        let q = egui::Rect::from_center_size(sq, vec2(7.0, 7.0));
        ui.painter().rect_filled(q, 0.0, color);
        ui.painter().rect_stroke(q, 0.0, Stroke::new(1.0, t.text), StrokeKind::Inside);
    } else if child_sel {
        ui.painter().rect_filled(egui::Rect::from_center_size(sq, vec2(4.0, 4.0)), 0.0, color);
    }
    let sq_resp = ui.interact(egui::Rect::from_center_size(sq, vec2(18.0, h)), ui.scope_id().with(("selsq", n.id.0)), Sense::click_and_drag());
    // Dragging the selected-art square carries the selection to another row.
    if (is_sel || child_sel) && sq_resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), LayersDrag::Art);
    }
    let m = ui.input(|i| i.modifiers);
    let on_name = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x <= name_end.max(name_rect.left() + 20.0) && p.x >= name_rect.left());
    if tresp.clicked() {
        out.actions.push(("layer.target".into(), json!({"id": n.id.0, "add": m.shift})));
    } else if sq_resp.clicked() {
        out.actions.push(("layer.selectAll".into(), json!({"id": n.id.0, "add": m.shift})));
    } else if resp.clicked() {
        out.click = Some((n.id, m));
        // Alt-click on a layer's or sublayer's name also selects its art, as its selection square does.
        // Only the drawn text counts, not the wider rename target of a short name.
        let on_text = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x >= name_rect.left() && p.x <= name_end.min(name_rect.right()));
        if m.alt && !m.shift && !m.command && n.is_layer() && on_text {
            out.actions.push(("layer.selectAll".into(), json!({"id": n.id.0})));
        }
    }
    if resp.double_clicked() {
        let on_thumb = thumb_rect.is_some_and(|th| ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| th.contains(p)));
        if on_name && !on_thumb {
            ui.data_mut(|d| d.insert_temp(rename_id(), (n.id.0, n.identify_label(view.identify_xml_id))));
        } else {
            out.actions.push(("ui.layerOptions".into(), json!({"ids": [n.id.0]})));
        }
    }
    // Row menu: Collect for Export on this row (or every highlighted row when this is one of them).
    let collect_ids: Vec<u64> = if view.rows.contains(&n.id) { view.rows.iter().map(|id| id.0).collect() } else { vec![n.id.0] };
    resp.context_menu(|ui| {
        collect_for_export_menu(ui, true, |multiple| {
            out.actions.push(("assets.add".into(), json!({ "ids": collect_ids.clone(), "multiple": multiple })));
        });
    });
    // Dragging rows: the highlighted rows when this is one of them, else this row.
    if resp.drag_started() && view.column.is_none() {
        let ids = if view.rows.contains(&n.id) { view.rows.clone() } else { vec![n.id] };
        egui::DragAndDrop::set_payload(ui.ctx(), LayersDrag::Rows(ids));
        let grab = resp.interact_pointer_pos().map_or(vec2(h, h / 2.0), |p| p - r.min);
        ui.data_mut(|d| d.insert_temp(key("grab"), (grab, r.width(), depth)));
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    drop_target(ui, view, n, r, x - 16.0, open, &resp, out);
}

/// Alt-clicking row `n`'s eye (`eye`) or lock: Hide Others or Lock Others, every top-level layer
/// but the one holding the row; when they all are hidden (locked) already, Show All Layers (Unlock
/// All Layers). Each is one undo step.
fn others_action(doc: &Document, n: &Node, eye: bool) -> (String, Value) {
    let own = doc.layer_of(n.id);
    let mut others = doc.layers.iter().filter(|l| Some(l.id) != own);
    let cmd = match (eye, if eye { others.any(|l| l.visible) } else { others.any(|l| !l.locked) }) {
        (true, true) => "layer.hideOthers",
        (true, false) => "layer.showAll",
        (false, true) => "layer.lockOthers",
        (false, false) => "layer.unlockAll",
    };
    // Show All and Unlock All take no rows.
    let params = if cmd.ends_with("Others") { json!({ "ids": [n.id.0] }) } else { json!({}) };
    (cmd.into(), params)
}

/// Ctrl/Cmd+Alt-clicking row `n`'s eye runs Outline Others, which puts every top-level layer but the
/// one holding the row in Outline and puts that one in Preview. When all the other layers are in
/// Outline already, it runs Preview All Layers. Each is one undo step.
fn outline_others_action(doc: &Document, n: &Node) -> (String, Value) {
    let own = doc.layer_of(n.id);
    if doc.layers.iter().any(|l| Some(l.id) != own && !matches!(l.kind, NodeKind::Layer { preview: false, .. })) {
        ("layer.outlineOthers".into(), json!({ "ids": [n.id.0] }))
    } else {
        // Preview All takes no rows.
        ("layer.previewAll".into(), json!({}))
    }
}

/// The eye and lock columns of row `n` at `r`.
fn eye_and_lock(ui: &mut Ui, view: &View, n: &Node, r: egui::Rect, out: &mut Out) {
    let t = &view.t;
    let h = view.h;
    let eye = egui::Rect::from_min_size(r.min, vec2(COLUMN, h));
    let lock = egui::Rect::from_min_size(r.min + vec2(COLUMN, 0.0), vec2(COLUMN, h));
    ui.painter().line_segment([eye.right_top(), eye.right_bottom()], Stroke::new(1.0, t.input_border));
    ui.painter().line_segment([lock.right_top(), lock.right_bottom()], Stroke::new(1.0, t.input_border));
    let preview = !matches!(n.kind, NodeKind::Layer { preview: false, .. });
    let er = ui.interact(eye, ui.scope_id().with(("eye", n.id.0)), Sense::click_and_drag());
    if n.visible {
        let icon = egui::Rect::from_center_size(eye.center(), vec2(14.0, 14.0));
        if n.is_template() {
            // A template layer shows the template mark instead of the eye.
            icons::paint(ui, "shapes", icon, t.icon);
        } else if preview {
            icons::paint(ui, "eye", icon, t.icon);
        } else {
            // Outline view: a hollow eye.
            icons::paint(ui, "eye", icon, t.text_dim);
            ui.painter().circle_stroke(eye.center(), 7.5, Stroke::new(1.0, t.text_dim));
        }
    }
    let lr = ui.interact(lock, ui.scope_id().with(("lock", n.id.0)), Sense::click_and_drag());
    if n.locked {
        icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.icon);
    } else if lr.hovered() {
        icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.text_disabled);
    }
    let m = ui.input(|i| i.modifiers);
    if er.clicked() {
        if m.alt && m.command {
            out.actions.push(outline_others_action(view.doc, n));
        } else if m.alt {
            out.actions.push(others_action(view.doc, n, true));
        } else if m.command && n.is_layer() {
            // Ctrl/Cmd-click: Preview ↔ Outline for this layer.
            out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "preview": !preview})));
        } else {
            out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "visible": !n.visible})));
        }
    }
    if lr.clicked() && m.alt {
        out.actions.push(others_action(view.doc, n, false));
    } else if lr.clicked() {
        out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "locked": !n.locked})));
    }
    // Dragging down a column gives every row it passes this row's new state.
    if er.drag_started() {
        out.column = Some(Some(ColumnDrag { eye: true, value: !n.visible, ids: vec![n.id.0] }));
    } else if lr.drag_started() {
        out.column = Some(Some(ColumnDrag { eye: false, value: !n.locked, ids: vec![n.id.0] }));
    } else if let Some(c) = out.column.clone().unwrap_or_else(|| view.column.clone())
        && !c.ids.contains(&n.id.0)
        && ui.input(|i| i.pointer.primary_down() && i.pointer.interact_pos().is_some_and(|p| p.y >= r.top() && p.y < r.bottom()))
    {
        let mut c = c;
        c.ids.push(n.id.0);
        out.column = Some(Some(c));
    }
}

/// Row `n` at `r` as a drop target for dragged rows or the selected-art square: shows where they
/// would go (a line above or below it, or a box around it for inside) and moves them on release
/// (Alt copies). `indent` is where the row's content starts.
#[allow(clippy::too_many_arguments)]
fn drop_target(ui: &Ui, view: &View, n: &Node, r: egui::Rect, indent: f32, open: bool, resp: &egui::Response, out: &mut Out) {
    let Some(drag) = resp.dnd_hover_payload::<LayersDrag>() else { return };
    let doc = view.doc;
    let moving: Vec<NodeId> = match &*drag {
        LayersDrag::Rows(ids) => ids.clone(),
        LayersDrag::Art => view.sel.iter().copied().collect(),
        LayersDrag::Guides(guides) => {
            // Guides go to the layer of the row (the row's own, for a layer).
            if let Some(layer) = if n.is_layer() { Some(n.id) } else { doc.layer_containing(n.id) } {
                guide_drop(ui, view, guides, layer, r, resp, out);
            }
            return;
        }
    };
    // Never into itself or a row inside it.
    let around = doc.ancestry(n.id).unwrap_or_default();
    if moving.is_empty() || moving.iter().any(|m| around.contains(m)) {
        return;
    }
    let Some(p) = ui.input(|i| i.pointer.hover_pos()) else { return };
    let rel = (p.y - r.top()) / r.height().max(1.0);
    let inside = takes(n, &drag, doc);
    let place = match &*drag {
        LayersDrag::Art if inside => Place::Inside,
        LayersDrag::Art => Place::Above,
        LayersDrag::Rows(_) if inside && rel >= 0.25 && (rel <= 0.75 || open) => Place::Inside,
        LayersDrag::Rows(_) if rel < 0.5 => Place::Above,
        LayersDrag::Rows(_) | LayersDrag::Guides(_) => Place::Below,
    };
    let stroke = Stroke::new(2.0, view.t.accent);
    match place {
        Place::Above => {
            ui.painter().line_segment([egui::pos2(indent, r.top()), egui::pos2(r.right(), r.top())], stroke);
        }
        Place::Below => {
            ui.painter().line_segment([egui::pos2(indent, r.bottom()), egui::pos2(r.right(), r.bottom())], stroke);
        }
        Place::Inside => {
            ui.painter().rect_stroke(r.shrink(1.0), 0.0, stroke, StrokeKind::Inside);
        }
    }
    if resp.dnd_release_payload::<LayersDrag>().is_some() {
        let copy = ui.input(|i| i.modifiers.alt);
        let ids: Vec<u64> = match &*drag {
            // The selection's top objects, as Arrange and Group take them.
            LayersDrag::Art => doc.paint_order(moving.iter().copied()).iter().map(|i| i.0).collect(),
            LayersDrag::Rows(ids) => ids.iter().map(|i| i.0).collect(),
            LayersDrag::Guides(_) => return,
        };
        out.actions.push(("layer.move".into(), json!({"ids": ids, "target": n.id.0, "place": place.id(), "copy": copy})));
    }
}

/// The target circle of `n`'s row at `c`: a ring, doubled while `n` is targeted, filled when `n`
/// has an appearance or transparency of its own. Its response is clicked to target `n`; dragging
/// it carries `n`'s appearance ([`PanelDrag::Appearance`]) onto another row's circle (Alt
/// copies it) or the trash.
fn target_circle(ui: &mut Ui, n: &Node, c: egui::Pos2, h: f32, targeted: bool, actions: &mut Vec<(String, Value)>, t: &Tokens) -> egui::Response {
    let resp = ui.interact(egui::Rect::from_center_size(c, vec2(16.0, h)), ui.scope_id().with(("target", n.id.0)), Sense::click_and_drag());
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
