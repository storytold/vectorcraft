//! Asset Export panel (Window → Asset Export): the document's assets as thumbnails with their
//! names. Art dragged in off the canvas becomes assets, one per object (with Alt held, one asset
//! of them all), and so does the selection with the + button. Clicking selects assets (Shift or
//! Cmd adds), double-clicking one renames it and the bin removes the selected ones. Below are the
//! export settings, the format rows Export for Screens shows too (one set per document), and
//! Export, which writes the selected assets (else all of them) into a picked folder (on the
//! desktop with Background Export on, on the background thread); the web downloads them. The
//! screen button opens Export for Screens on its Assets tab.
//!
//! Everything runs through the `assets.*` commands.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{ExportAsset, Node, NodeId};
use vectorcraft_engine::DocState;

use super::{pstate, set_pstate};
use crate::dialogs::{EXPORT_FOR_SCREENS, open_export_for_screens_assets, screen_formats, screen_saved_options, screen_saved_rows};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, io};

pub const ID: &str = "assetExport";
/// A tile: the thumbnail over the name.
const TILE: egui::Vec2 = vec2(70.0, 84.0);
const THUMB: f32 = 58.0;
const SELECTED: &str = "asset-export-selected";

/// The assets selected in the panel (the panel's own state) that are still in the document.
fn selected(ctx: &egui::Context, st: &DocState) -> Vec<u64> {
    let mut sel: Vec<u64> = pstate(ctx, SELECTED);
    sel.retain(|id| st.doc.asset(*id).is_some());
    sel
}

/// The addresses of an asset's objects, back to front: the identity of its look.
type Look = Vec<usize>;
/// A document (uid) at one revision.
type Revision = (u64, u64);

/// Asset `a`'s art fitted into `r` on white. Rendered once per look of its objects (an edited
/// object is a new allocation, so its address is a free change detector, as in the Layers panel).
pub(crate) fn paint_thumb(ui: &Ui, st: &DocState, a: &ExportAsset, r: Rect) {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        /// The addresses of each asset's objects in one document revision (no tree walks per frame).
        static KEYS: RefCell<(Revision, HashMap<u64, Look>)> = RefCell::new(Default::default());
        static CACHE: crate::graphics::TexCache<HashMap<(Look, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    ui.painter().rect_filled(r, 0.0, Color32::WHITE);
    if !ui.is_rect_visible(r) {
        return;
    }
    let ids = || st.doc.paint_order(a.nodes.iter().copied());
    let nodes = KEYS.with(|k| {
        let mut k = k.borrow_mut();
        if k.0 != (st.uid, st.revision) {
            *k = ((st.uid, st.revision), HashMap::new());
        }
        k.1.entry(a.id).or_insert_with(|| ids().iter().filter_map(|id| st.doc.node(*id)).map(|n| std::ptr::from_ref(n) as usize).collect()).clone()
    });
    let px = (r.width() * ui.ctx().pixels_per_point()).round().max(8.0) as u32;
    let key = (nodes, px);
    let tex = CACHE.with(|c| c.borrow().get(&key).cloned()).or_else(|| {
        let art: Vec<&Node> = ids().iter().filter_map(|id| st.doc.node(*id)).collect();
        // Same Art Optimized supersample as assets.export (#983): abutting fills leave no seam in
        // the tile (Layers row thumbs stay cheap without it).
        let opts = vectorcraft_render::RenderOptions { image_mips: true, precise: true, supersample: true, ..Default::default() };
        let img = RENDERER.with(|rr| match art.as_slice() {
            [one] => rr.borrow_mut().render_node_thumbnail_with(&st.doc, one, px, None, &opts),
            many => {
                let group = Node::group(NodeId(0), many.iter().map(|n| Arc::new((*n).clone())).collect());
                rr.borrow_mut().render_node_thumbnail_with(&st.doc, &group, px, None, &opts)
            }
        })?;
        let color = crate::canvas::color_image(&img);
        let tex = ui.ctx().load_texture(format!("asset-thumb-{}", a.id), color, egui::TextureOptions::LINEAR);
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 512 {
                c.clear();
            }
            c.insert(key, tex.clone());
        });
        Some(tex)
    });
    if let Some(t) = tex {
        ui.painter().image(t.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// What the tiles saw this frame (acted on after drawing).
#[derive(Default)]
struct Events {
    /// An asset clicked: (id, Shift/Cmd held).
    click: Option<(u64, bool)>,
    /// An asset renamed.
    rename: Option<(u64, String)>,
    /// Art dropped on the panel: (ids, as one asset).
    drop: Option<(Vec<NodeId>, bool)>,
}

/// The asset tiles: thumbnail, name (a text field while renaming), selection.
fn tiles(ui: &mut Ui, st: &DocState, sel: &[u64], ev: &mut Events) {
    let t = Tokens::get(ui.ctx());
    let rename_id = egui::Id::unique("asset-export-rename");
    let renaming: Option<(u64, String)> = ui.data(|d| d.get_temp(rename_id));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for a in &st.doc.assets {
            let (r, resp) = ui.allocate_exact_size(TILE, Sense::click());
            let on = sel.contains(&a.id);
            if on {
                ui.painter().rect_filled(r, 2.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 2.0, t.hover);
            }
            let th = Rect::from_min_size(egui::pos2(r.center().x - THUMB / 2.0, r.top() + 4.0), vec2(THUMB, THUMB));
            paint_thumb(ui, st, a, th);
            ui.painter().rect_stroke(th, 0.0, Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.border }), StrokeKind::Outside);
            let name_rect = Rect::from_min_max(egui::pos2(r.left() + 2.0, th.bottom() + 3.0), egui::pos2(r.right() - 2.0, r.bottom() - 1.0));
            match &renaming {
                Some((id, buf)) if *id == a.id => {
                    let mut buf = buf.clone();
                    let field = rename_id.with("field");
                    let kept = widgets::own_field_shift_enter(ui, field, &mut buf, &a.name);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
                    let te = child
                        .add(egui::TextEdit::singleline(&mut buf).id(field).desired_width(name_rect.width()).font(egui::FontId::proportional(11.5)));
                    // Enter or a click elsewhere commits, Escape cancels; Shift+Enter commits and
                    // keeps the field open.
                    if te.lost_focus() {
                        ui.data_mut(|d| d.remove::<(u64, String)>(rename_id));
                        if child.input(|i| !i.key_pressed(egui::Key::Escape)) && buf.trim() != a.name && !buf.trim().is_empty() {
                            ev.rename = Some((a.id, buf.trim().to_string()));
                        }
                    } else {
                        if kept && buf.trim() != a.name && !buf.trim().is_empty() {
                            ev.rename = Some((a.id, buf.trim().to_string()));
                        }
                        te.request_focus();
                        ui.data_mut(|d| d.insert_temp(rename_id, (a.id, buf)));
                    }
                }
                _ => {
                    ui.painter().with_clip_rect(name_rect).text(
                        egui::pos2(name_rect.center().x, name_rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        &a.name,
                        egui::FontId::proportional(11.5),
                        t.text,
                    );
                }
            }
            let resp = resp.on_hover_text(format!("{}\n{}", a.name, tl!("Double-click to rename")));
            if resp.double_clicked() {
                ui.data_mut(|d| d.insert_temp(rename_id, (a.id, a.name.clone())));
            } else if resp.clicked() {
                ev.click = Some((a.id, ui.input(|i| i.modifiers.shift || i.modifiers.command)));
            }
        }
    });
}

/// Art dragged off the canvas over the list: outline it; dropped, it becomes assets.
fn drop_zone(ui: &Ui, zone: &egui::Response, ev: &mut Events) {
    if let Some(ids) = widgets::art_drop(ui, zone) {
        ev.drop = Some((ids.into_iter().map(NodeId).collect(), ui.input(|i| i.modifiers.alt)));
    }
}

/// Run `cmd`, reporting a failure in the status bar.
fn run(app: &mut VectorcraftApp, cmd: &str, params: Value) -> Option<Value> {
    app.run(cmd, params).map_err(|e| app.status(e)).ok()
}

/// Collect `ids` (else the selection) as assets (one of them all with `single`), and select them
/// in the panel.
fn collect(app: &mut VectorcraftApp, ctx: &egui::Context, ids: Option<Vec<NodeId>>, single: bool) {
    let mut p = json!({ "multiple": !single });
    if let Some(ids) = ids {
        p["ids"] = json!(ids.iter().map(|id| id.0).collect::<Vec<_>>());
    }
    if let Some(r) = run(app, "assets.add", p) {
        let ids: Vec<u64> = r["assets"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
        set_pstate(ctx, SELECTED, ids);
    }
}

/// Export assets `ids` (`files_each`: the files one asset makes) into a picked folder (without a
/// folder picker: the last folder Export for Screens used, else the Desktop); the web downloads
/// them, several as one zip.
fn export(app: &mut VectorcraftApp, ids: Vec<u64>, files_each: usize) {
    // The folder picked off the UI thread exports them then.
    crate::picks::button(app, move |app| export_now(app, ids.clone(), files_each));
}

fn export_now(app: &mut VectorcraftApp, ids: Vec<u64>, files_each: usize) {
    let mut p = json!({ "assets": ids });
    // Match the shared dialog's TIFF defaults, retaining saved choices and other formats.
    if let Some(st) = app.session.active()
        && screen_saved_rows(&st.doc.export_settings)
            .0
            .iter()
            .any(|row| row["format"].as_str().and_then(vectorcraft_engine::cmd::fileio::format).is_some_and(|f| f.id == "tiff"))
    {
        let saved = &st.doc.export_settings;
        let mut settings = saved.get("settings").and_then(Value::as_object).cloned().unwrap_or_default();
        settings.insert("tiff".into(), Value::Object(screen_saved_options(app, saved, "tiff")));
        p["settings"] = Value::Object(settings);
    }
    if io::is_web(app) {
        p["zip"] = json!(ids.len() * files_each > 1);
    } else {
        let saved = app.session.active().map(|st| st.doc.export_settings.clone()).unwrap_or_default();
        let folder = if crate::picks::can(app, &crate::picks::PickRequest::Folder) {
            match crate::picks::folder(app) {
                Some(f) => f,
                None => return,
            }
        } else {
            match saved
                .get("folder")
                .and_then(Value::as_str)
                .filter(|f| !f.is_empty())
                .map(str::to_string)
                .or_else(vectorcraft_engine::cmd::fileio::export_folder)
            {
                Some(f) => f,
                None => return app.status("choose a folder to export to".to_string()),
            }
        };
        p["folder"] = json!(folder);
        p["openLocation"] = json!(saved.get("openLocation").and_then(Value::as_bool).unwrap_or(true));
    }
    if let Err(e) = io::export_files(app, "assets.export", p) {
        app.status(e);
    }
}

/// What the export settings rows asked for.
enum Settings {
    /// Change the shared settings (`assets.settings.set`).
    Set(Value),
    /// Show a format's Format Settings (in Export for Screens).
    Format(String),
}

/// The export settings: Export for Screens' format rows on the document's shared settings.
/// Returns the change asked for and how many files one asset makes.
fn settings(ui: &mut Ui, st: &DocState) -> (Option<Settings>, usize) {
    let (rows, preset) = screen_saved_rows(&st.doc.export_settings);
    let preset = preset.unwrap_or_default().to_string();
    let n = rows.len();
    let mut d = Dialog::new(EXPORT_FOR_SCREENS, json!({ "formats": &rows, "preset": preset, "__settings": "" }));
    screen_formats(ui, &mut d);
    let (shown, picked) = (d.str("__settings"), d.str("preset"));
    let formats = d.fields.remove("formats").unwrap_or(Value::Null);
    let change = if !shown.is_empty() {
        Some(Settings::Format(shown))
    } else if picked != preset && !picked.is_empty() {
        Some(Settings::Set(json!({ "preset": picked, "subfolders": true })))
    } else if picked != preset || formats.as_array() != Some(&rows) {
        // Rows edited, or Custom picked: the rows as they are, without the preset.
        Some(Settings::Set(json!({ "formats": formats, "preset": "" })))
    } else {
        None
    };
    (change, n)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(st) = app.session.active() else {
        widgets::dim_label(ui, tl!("No document"));
        return;
    };
    let sel = selected(&ctx, st);
    let mut ev = Events::default();
    widgets::list_box(ui, |ui| {
        let out = egui::ScrollArea::vertical().id_salt("asset-export-scroll").max_height(270.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.set_min_height(110.0);
            ui.set_width(ui.available_width());
            if st.doc.assets.is_empty() {
                super::empty_state(
                    ui,
                    "share-2",
                    tl!("No assets"),
                    tl!("Drag art here, or select it and click +. Hold Alt for one asset of several objects."),
                );
            } else {
                tiles(ui, st, &sel, &mut ev);
            }
        });
        let zone = ui.interact(out.inner_rect, ui.scope_id().with("asset-export-drop"), Sense::hover());
        drop_zone(ui, &zone, &mut ev);
    });
    ui.add_space(6.0);
    widgets::section_header(ui, tl!("Export Settings"));
    let (change, files_each) = settings(ui, st);
    let all: Vec<u64> = st.doc.assets.iter().map(|a| a.id).collect();
    let has_art = !st.selection.is_empty();
    let alt = ui.input(|i| i.modifiers.alt);
    ui.add_space(6.0);
    let mut export_clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(!all.is_empty(), |ui| {
                let label = if sel.is_empty() || sel.len() == all.len() {
                    tl!("Export").to_string()
                } else {
                    crate::i18n::fmt(tl!("Export {count} Selected"), &[("count", &sel.len().to_string())])
                };
                export_clicked = widgets::primary_button(ui, &label).on_hover_text(tl!("Export the selected assets (else all of them)")).clicked();
            });
        });
    });
    let (mut screens, mut add, mut remove) = (false, false, false);
    widgets::bottom_bar(ui, |ui| {
        screens = widgets::icon_button(ui, "monitor", tl!("Export for Screens…"), false, 24.0).clicked();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            remove = widgets::icon_button_enabled(ui, "trash-2", tl!("Remove Selected Assets"), false, !sel.is_empty(), 24.0).clicked();
            add = widgets::icon_button_enabled(ui, "plus", tl!("Add Selected Artwork (Alt: as one asset)"), false, has_art, 24.0).clicked();
        });
    });
    // Act on what the panel saw (the document is no longer borrowed).
    if let Some((id, toggle)) = ev.click {
        let mut s = sel.clone();
        match (toggle, s.iter().position(|x| *x == id)) {
            (true, Some(i)) => _ = s.remove(i),
            (true, None) => s.push(id),
            (false, _) => s = vec![id],
        }
        set_pstate(&ctx, SELECTED, s);
    }
    if let Some((id, name)) = ev.rename {
        run(app, "assets.rename", json!({ "asset": id, "name": name }));
    }
    if let Some((ids, single)) = ev.drop {
        collect(app, &ctx, Some(ids), single);
    }
    match change {
        Some(Settings::Set(p)) => _ = run(app, "assets.settings.set", p),
        Some(Settings::Format(f)) => {
            open_export_for_screens_assets(app, Some(&sel_or_all(&sel, &all)));
            if let Some(d) = app.ui.dialog.as_mut() {
                d.fields.insert("__settings".into(), json!(f));
            }
        }
        None => {}
    }
    if add {
        collect(app, &ctx, None, alt);
    }
    if remove {
        run(app, "assets.remove", json!({ "assets": sel }));
        set_pstate(&ctx, SELECTED, Vec::<u64>::new());
    }
    if screens {
        open_export_for_screens_assets(app, Some(&sel_or_all(&sel, &all)));
    }
    if export_clicked {
        export(app, sel_or_all(&sel, &all), files_each);
    }
}

/// The selected assets, else all of them.
fn sel_or_all(sel: &[u64], all: &[u64]) -> Vec<u64> {
    if sel.is_empty() { all.to_vec() } else { sel.to_vec() }
}

/// The panel (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(st) = app.session.active() else { return };
    let (has_art, sel) = (!st.selection.is_empty(), selected(&ctx, st));
    let all: Vec<u64> = st.doc.assets.iter().map(|a| a.id).collect();
    if menu_item(ui, tl!("Add Selected Artwork as One Asset"), has_art, false) {
        collect(app, &ctx, None, true);
    }
    if menu_item(ui, tl!("Add Selected Artwork as Multiple Assets"), has_art, false) {
        collect(app, &ctx, None, false);
    }
    ui.separator();
    if menu_item(ui, tl!("Select All Assets"), !all.is_empty(), false) {
        set_pstate(&ctx, SELECTED, all.clone());
    }
    if menu_item(ui, tl!("Remove Selected Assets"), !sel.is_empty(), false) {
        run(app, "assets.remove", json!({ "assets": sel }));
        set_pstate(&ctx, SELECTED, Vec::<u64>::new());
    }
    ui.separator();
    if menu_item(ui, tl!("Export for Screens…"), true, false) {
        open_export_for_screens_assets(app, Some(&sel_or_all(&sel, &all)));
    }
}
