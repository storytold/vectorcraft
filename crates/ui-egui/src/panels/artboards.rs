//! Artboards panel: numbered list with inline rename (double-click), a lock per artboard, move
//! up / down, new, delete. The selected row is the document's active artboard.

use egui::{Sense, Ui, pos2, vec2};
use serde_json::json;

use super::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

/// The active artboard (the selected row).
fn selected(app: &VectorcraftApp, n: usize) -> usize {
    app.session.active().map_or(0, |st| st.active_artboard).min(n.saturating_sub(1))
}

fn set_selected(app: &mut VectorcraftApp, i: usize) {
    app.run("artboard.setActive", json!({ "index": i })).ok();
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let abs: Vec<String> = app.session.active().map(|d| d.doc.artboards.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
    let locks: Vec<bool> = app.session.active().map(|d| d.doc.artboards.iter().map(|a| a.locked).collect()).unwrap_or_default();
    if abs.is_empty() {
        super::empty_state(ui, "dc-artboards", tl!("No document"), tl!("Open a document to see its artboards."));
        return;
    }
    let sel = selected(app, abs.len());
    let editing: Option<usize> = pstate(ui.ctx(), "ab-edit");
    widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt("ab-scroll").max_height(220.0).show(ui, |ui| {
            for (i, name) in abs.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if i == sel {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                ui.painter().text(
                    pos2(r.left() + 22.0, r.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{}", i + 1),
                    egui::FontId::proportional(12.0),
                    t.text_dim,
                );
                let name_rect = egui::Rect::from_min_max(pos2(r.left() + 32.0, r.top() + 2.0), pos2(r.right() - 48.0, r.bottom() - 2.0));
                if editing == Some(i) {
                    let id = ui.id().with(("ab-name", i));
                    let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| name.clone());
                    let te = ui.put(name_rect, egui::TextEdit::singleline(&mut buf).id(id).font(egui::FontId::proportional(12.0)));
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
                    if te.lost_focus() {
                        set_pstate::<Option<usize>>(ui.ctx(), "ab-edit", None);
                        ui.data_mut(|d| d.remove::<String>(id));
                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && !buf.trim().is_empty() && buf != *name {
                            app.run("artboard.setProps", json!({"index": i, "name": buf.trim()})).ok();
                        }
                    }
                } else {
                    ui.painter().text(name_rect.left_center(), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.5), t.text);
                }
                let opt = egui::Rect::from_center_size(r.right_center() - vec2(14.0, 0.0), vec2(14.0, 14.0));
                let oresp = ui.interact(opt, ui.id().with(("ab-opt", i)), Sense::click());
                icons::paint(ui, "dc-artboard-options", opt, if oresp.hovered() { t.text_strong } else { t.icon });
                if oresp.on_hover_text(tl!("Artboard Options: edit with the Artboard tool")).clicked() {
                    set_selected(app, i);
                    app.select_tool("artboard");
                }
                // The lock: locks the artboard and the art on it.
                let locked = locks.get(i).copied().unwrap_or(false);
                let lk = egui::Rect::from_center_size(r.right_center() - vec2(34.0, 0.0), vec2(14.0, 14.0));
                let lresp = ui.interact(lk, ui.id().with(("ab-lock", i)), Sense::click());
                if locked || lresp.hovered() || resp.hovered() {
                    let icon = if locked { "lock" } else { "lock-open" };
                    icons::paint(ui, icon, lk, if locked || lresp.hovered() { t.icon } else { t.text_disabled });
                }
                let tip = if locked { tl!("Unlock Artboard") } else { tl!("Lock Artboard") };
                if lresp.on_hover_text(tip).clicked() {
                    app.run("artboard.lock", json!({"index": i, "locked": !locked})).ok();
                }
                if resp.clicked() {
                    set_selected(app, i);
                }
                if resp.double_clicked() {
                    set_pstate(ui.ctx(), "ab-edit", Some(i));
                }
            }
        });
    });
    let n = abs.len();
    widgets::bottom_bar(ui, |ui| {
        widgets::icon_button_enabled(ui, "dc-rearrange", tl!("Rearrange All Artboards (on the roadmap)"), false, false, 24.0);
        ui.add_space((ui.available_width() - 4.0 * 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "dc-arrow-up", tl!("Move Up"), false, sel > 0, 24.0).clicked()
            && app.run("artboard.reorder", json!({"index": sel, "to": sel - 1})).is_ok()
        {
            set_selected(app, sel - 1);
        }
        if widgets::icon_button_enabled(ui, "dc-arrow-down", tl!("Move Down"), false, sel + 1 < n, 24.0).clicked()
            && app.run("artboard.reorder", json!({"index": sel, "to": sel + 1})).is_ok()
        {
            set_selected(app, sel + 1);
        }
        if widgets::icon_button(ui, "dc-new-item", tl!("New Artboard"), false, 24.0).clicked() && app.run("artboard.new", json!({})).is_ok() {
            set_selected(app, n);
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Artboard"), false, n > 1, 24.0).clicked() {
            app.run("artboard.delete", json!({"index": sel})).ok();
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = app.session.active().map(|d| d.doc.artboards.len()).unwrap_or(0);
    let sel = selected(app, n);
    let locked = app.session.active().and_then(|d| d.doc.artboards.get(sel)).is_some_and(|a| a.locked);
    if menu_item(ui, tl!("New Artboard"), n > 0, false) {
        app.run("artboard.new", json!({})).ok();
    }
    if menu_item(ui, tl!("Duplicate Artboards"), n > 0, false) {
        app.run("artboard.duplicate", json!({"index": sel})).ok();
    }
    if menu_item(ui, tl!("Delete Artboards"), n > 1, false) {
        app.run("artboard.delete", json!({"index": sel})).ok();
    }
    if menu_item(ui, tl!("Rename"), n > 0, false) {
        set_pstate(ui.ctx(), "ab-edit", Some(sel));
    }
    if menu_item(ui, if locked { tl!("Unlock Artboard") } else { tl!("Lock Artboard") }, n > 0, false) {
        app.run("artboard.lock", json!({"index": sel, "locked": !locked})).ok();
    }
    if menu_item(ui, tl!("Export Artboard…"), n > 0, false) {
        app.run("artboard.export", json!({"index": sel})).ok();
    }
    menu_item(ui, tl!("Delete Empty Artboards"), false, false);
    ui.separator();
    menu_item(ui, tl!("Convert to Artboards"), false, false);
    if menu_item(ui, tl!("Artboard Options…"), n > 0, false) {
        app.select_tool("artboard");
    }
    menu_item(ui, tl!("Rearrange All Artboards…"), false, false);
    ui.separator();
    if menu_item(ui, tl!("Fit to Artwork Bounds"), n > 0, false) {
        app.run("artboard.fitToArt", json!({"index": sel})).ok();
    }
}
