//! Window chrome: application bar (menus), Control bar, document tabs, status bar.

use std::borrow::Cow;

use egui::{CornerRadius, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;

use crate::panels::stroke as stroke_panel;
use crate::state::{ZOOM_STOPS, zoom_label};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{VectorcraftApp, icons, menus, titlebar};

/// The application bar: brand mark, Home, menus, then Discord, search and the workspace switcher
/// at the right. With [`VectorcraftApp::custom_titlebar`] it is also the window's title bar
/// ([`titlebar`]): the caption buttons take the right end and the rest of the bar drags the window.
pub fn app_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let custom = app.custom_titlebar;
    let left = if app.integrated_titlebar { 78 } else { 8 };
    let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin { left, right: if custom { 0 } else { 14 }, top: 0, bottom: 0 });
    let bar = egui::Panel::top("app_bar").exact_size(44.0).frame(frame.stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        if custom {
            titlebar::drag_area(ui, ui.max_rect());
        }
        ui.horizontal_centered(|ui| {
            // Brand mark: the app icon.
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            crate::brand::paint_mark(ui, r);
            ui.add_space(4.0);
            let on_home = app.ui.home.is_some() || app.session.active().is_none();
            if widgets::icon_button(ui, "house", tl!("Home"), on_home, 24.0).clicked() {
                app.run("app.home", json!({})).ok();
            }
            ui.add_space(2.0);
            let menus_end = if app.native_menu {
                let full = ui.max_rect();
                ui.painter().text(
                    full.center(),
                    egui::Align2::CENTER_CENTER,
                    vectorcraft_engine::cmd::help::APP_NAME,
                    egui::FontId::proportional(13.5),
                    t.text,
                );
                ui.cursor().min.x
            } else {
                menus::menu_bar(app, ui)
            };
            // The right-side group fills the space after the menus from the right; when it runs
            // short, the search box becomes an icon, then the workspace switcher narrows.
            let full = ui.max_rect();
            let right_edge = if custom { full.right() - titlebar::WIDTH - 10.0 } else { full.right() };
            let right = egui::Rect::from_min_max(egui::pos2(menus_end + 8.0, full.top()), egui::pos2(right_edge, full.bottom()));
            let room = right.width();
            // Built-in workspace names translate; the user's own stay as typed.
            let ws_name =
                if crate::workspaces::is_builtin(&app.ui.workspace) { tl!(&app.ui.workspace).to_string() } else { app.ui.workspace.clone() };
            let ws = ui.painter().layout_no_wrap(ws_name, egui::FontId::proportional(12.0), t.text);
            let ws_w = (ws.size().x + 36.0).clamp(112.0, 190.0);
            let gap = ui.spacing().item_spacing.x;
            let with_search = ws_w + 8.0 + gap + 200.0;
            let search_full = room >= with_search;
            let ws_w = if search_full { ws_w } else { ws_w.min(room - 8.0 - gap - 24.0).max(64.0) };
            let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));
            let ui = &mut rui;
            // Workspace switcher: shows the current workspace, opens Window → Workspace.
            let (wr, wresp) = ui.allocate_exact_size(vec2(ws_w, 24.0), Sense::click());
            ui.painter().rect_filled(wr, CornerRadius::same(4), if wresp.hovered() { t.hover } else { t.panel });
            ui.painter().with_clip_rect(wr.shrink2(vec2(4.0, 0.0))).galley(wr.left_center() + vec2(10.0, -ws.size().y / 2.0), ws, t.text);
            icons::paint(ui, "chevron-down", egui::Rect::from_center_size(wr.right_center() - vec2(12.0, 0.0), vec2(12.0, 12.0)), t.text_dim);
            let wresp = wresp.on_hover_text(tl!("Switch workspace"));
            egui::Popup::menu(&wresp).show(|ui| crate::workspaces::popup(app, ui));
            ui.add_space(8.0);
            // Search box → command palette.
            let open_palette = if search_full {
                let (r, resp) = ui.allocate_exact_size(vec2(200.0, 24.0), Sense::click());
                ui.painter().rect_filled(r, CornerRadius::same(12), t.input);
                ui.painter().rect_stroke(
                    r,
                    CornerRadius::same(12),
                    Stroke::new(1.0, if resp.hovered() { t.input_border } else { t.divider }),
                    StrokeKind::Inside,
                );
                icons::paint(ui, "search", egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(13.0, 13.0)), t.text_dim);
                ui.painter().text(
                    r.left_center() + vec2(26.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    tl!("Search commands and tools"),
                    egui::FontId::proportional(11.5),
                    t.text_dim,
                );
                resp.clicked()
            } else {
                widgets::icon_button(ui, "search", tl!("Search commands and tools"), false, 24.0).clicked()
            };
            if open_palette {
                app.ui.palette_open = true;
                app.ui.palette_query.clear();
            }
        });
    });
    if custom {
        titlebar::caption_buttons(app, ui, bar.response.rect);
    }
}

/// The Control bar (Window → Control), context-sensitive like Illustrator's.
/// The Control bar's Snapping button: a popover with the snapping toggles (Smart Guides, Snap to
/// Point, Snap to Grid, Snap to Pixel) and a link to the Smart Guides preferences.
fn snapping_popover(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let v = &app.ui.view;
    let any = v.smart_guides || v.snap_to_point || v.snap_to_grid || v.snap_to_pixel;
    let resp = widgets::icon_button(ui, "grid-3x3", tl!("Snapping"), any, 24.0);
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(180.0);
        ui.label(egui::RichText::new(tl!("Snapping")).font(theme::semibold(12.0)).color(t.text));
        let v = app.ui.view.clone();
        for (label, id, on) in [
            ("Smart Guides", "view.smartGuides", v.smart_guides),
            ("Snap to Point", "view.snapToPoint", v.snap_to_point),
            ("Snap to Grid", "view.snapToGrid", v.snap_to_grid),
            ("Snap to Pixel", "view.snapToPixel", v.snap_to_pixel),
        ] {
            if widgets::check(ui, tl!(label), on, true) {
                app.run(id, json!({})).ok();
            }
        }
        ui.separator();
        if ui.button(tl!("Smart Guides Preferences…")).clicked() {
            app.run("edit.preferences", json!({ "category": "Smart Guides" })).ok();
        }
    });
}

pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("control_bar")
        .exact_size(34.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Gripper
                let (g, _) = ui.allocate_exact_size(vec2(6.0, 20.0), Sense::hover());
                for i in 0..5 {
                    ui.painter().circle_filled(g.center_top() + vec2(0.0, 2.0 + i as f32 * 4.0), 0.9, t.text_disabled);
                }
                let Some(st) = app.session.active() else {
                    ui.label(egui::RichText::new(tl!("No Document")).color(t.text_dim));
                    return;
                };
                let sel = st.selection.objects.clone();
                let units = app.session.general_unit();
                let first = sel.first().and_then(|id| st.doc.node(*id)).cloned();
                let anchor_mode = !st.selection.anchors.is_empty();
                let label = match &first {
                    Some(_) if sel.len() == 1 && anchor_mode => tl!("Anchor Point"),
                    Some(vectorcraft_doc::Node { kind: NodeKind::Image(im), .. }) if sel.len() == 1 => {
                        if im.link.is_some() {
                            tl!("Linked File")
                        } else {
                            tl!("Embedded")
                        }
                    }
                    _ => tl!(crate::panels::appearance::object_label(app)),
                };
                ui.label(egui::RichText::new(label).font(theme::semibold(12.0)).color(t.text));
                ui.add_space(6.0);
                crate::place::control_bar_details(app, ui);
                crate::toolbar::control_bar_options(app, ui);
                crate::dialogs::envelope::control_bar(app, ui);
                let shown_stroke = crate::panels::current_stroke(app);
                let mixed = crate::panels::stroke_mixed(app, ui.ctx());
                let weight = stroke_panel::shown_weight(app, shown_stroke.as_ref(), &mixed);
                let opacity = if first.is_some() { crate::panels::current_transparency(app).map_or(1.0, |t| t.0) } else { 1.0 };
                crate::panels::paint_chip(app, ui, false, 22.0, true);
                crate::panels::paint_chip(app, ui, true, 22.0, true);
                // The link opens the Stroke panel as a popover under it; then the weight spinner
                // (with presets) and the width profile.
                let link = ui.link(egui::RichText::new(tl!("Stroke:")).size(12.0).color(t.text).underline()).on_hover_text(tl!("Stroke options"));
                stroke_panel::popover(app, &link);
                stroke_panel::weight_field(app, ui, "cb-stroke", weight, 100.0);
                if let Some(id) = stroke_panel::profile_dropdown(app, ui, shown_stroke.as_ref().and_then(|s| s.profile.as_ref())) {
                    app.run("stroke.set", json!({"profile": id})).ok();
                }
                ui.add_space(4.0);
                ui.separator();
                if ui.link(egui::RichText::new(tl!("Opacity:")).size(12.0).color(t.text).underline()).clicked() {
                    app.ui.open_panel = Some("transparency".into());
                }
                if let Some(o) = widgets::plain_field(ui, "cb-opacity", opacity as f64 * 100.0, "%", 0, 56.0)
                    && !sel.is_empty()
                {
                    app.run("transparency.set", json!({"opacity": o.clamp(0.0, 100.0)})).ok();
                }
                if !sel.is_empty() {
                    // Style picker: the selection's graphic style; its menu applies another.
                    ui.add_space(4.0);
                    if ui.link(egui::RichText::new(tl!("Style:")).size(12.0).color(t.text).underline()).clicked() {
                        app.ui.open_panel = Some("graphicStyles".into());
                    }
                    let resp = widgets::chip_button(ui, 22.0, true, |ui, r| crate::panels::graphic_styles::paint_linked(app, ui, r))
                        .on_hover_text(tl!("Graphic Style"));
                    egui::Popup::menu(&resp).show(|ui| crate::panels::graphic_styles::picker(app, ui));
                }
                ui.separator();
                snapping_popover(app, ui);
                ui.separator();
                if sel.is_empty() {
                    if widgets::flat_button(ui, tl!("Document Setup"), 112.0).clicked() {
                        app.run("file.documentSetup", json!({})).ok();
                    }
                    if widgets::flat_button(ui, tl!("Preferences"), 90.0).clicked() {
                        app.run("edit.preferences", json!({})).ok();
                    }
                    return;
                }
                // Align buttons.
                for (icon, tip, p) in [
                    ("align-start-vertical", "Horizontal Align Left", json!({"horizontal": "left"})),
                    ("align-center-vertical", "Horizontal Align Center", json!({"horizontal": "center"})),
                    ("align-end-vertical", "Horizontal Align Right", json!({"horizontal": "right"})),
                    ("align-start-horizontal", "Vertical Align Top", json!({"vertical": "top"})),
                    ("align-center-horizontal", "Vertical Align Center", json!({"vertical": "center"})),
                    ("align-end-horizontal", "Vertical Align Bottom", json!({"vertical": "bottom"})),
                ] {
                    if widgets::icon_button(ui, icon, tl!(tip), false, 24.0).clicked() {
                        let mut p = p;
                        if sel.len() == 1 {
                            p["to"] = json!("artboard");
                        }
                        app.run("object.align", p).ok();
                    }
                }
                ui.separator();
                // Transform fields.
                // The bounding box, rotated with rotated objects: its centre and its own sides.
                if let Some(b) = app.selection_box() {
                    let c = b.center();
                    let link = app.session.prefs.constrain_proportions;
                    for (k, lbl, v) in [("x", "X:", c.x), ("y", "Y:", c.y), ("width", "W:", b.rect.width()), ("height", "H:", b.rect.height())] {
                        // The W/H link sits between W and H.
                        if k == "height" {
                            crate::panels::transform::constrain_link(app, ui);
                        }
                        ui.label(egui::RichText::new(tl!(lbl)).size(12.0).color(t.text_dim));
                        if let Some(nv) = widgets::num_field(ui, ("cb", k), Some(v), units, 80.0) {
                            app.run("object.setBounds", json!({k: nv, "reference": 4, "proportional": link})).ok();
                        }
                    }
                }
            });
        });
}

/// A document tab's title: "Name* @ 66.67 % (RGB/Preview)", or while an opacity mask is edited
/// "Name* @ 66.67 % (<Opacity Mask>/Opacity Mask)".
fn tab_title(d: &vectorcraft_engine::DocState, zoom: f64, outline: bool) -> String {
    let mode = if d.doc.mask_edit.is_some() {
        format!("<{0}>/{0}", tl!("Opacity Mask"))
    } else {
        let color = if d.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk { "CMYK" } else { "RGB" };
        format!("{color}/{}", if outline { tl!("Outline") } else { tl!("Preview") })
    };
    format!("{}{} @ {} ({mode})", d.title(), if d.is_dirty() { "*" } else { "" }, zoom_label(zoom).replace('%', " %"))
}

/// Document tab strip: "Name* @ 66.67% (RGB/Preview)".
pub fn doc_tabs(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), 35.0), Sense::hover());
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    let mut x = strip.left();
    let mut activate = None;
    let mut close = None;
    let active = app.session.active_index();
    for (i, d) in app.session.documents().iter().enumerate() {
        let zoom = app.views.get(i).map(|v| v.zoom).unwrap_or(1.0);
        let title = tab_title(d, zoom, app.ui.view.outline);
        // On the Home screen no tab is the current one.
        let is_active = Some(i) == active && app.ui.home.is_none();
        let galley = ui.painter().layout_no_wrap(title, theme::semibold(12.5), if is_active { t.text_strong } else { t.text_dim });
        let w = galley.size().x + 50.0;
        let r = egui::Rect::from_min_size(egui::pos2(x, strip.top()), vec2(w, strip.height() - 1.0));
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        if is_active {
            ui.painter().rect_filled(r, 0.0, t.panel);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.4));
        }
        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.5, t.border));
        // × at the left like Illustrator.
        let xr = egui::Rect::from_center_size(egui::pos2(r.left() + 16.0, r.center().y), vec2(12.0, 12.0));
        let xresp = ui.interact(xr.expand(3.0), ui.id().with(("tabx", i)), Sense::click());
        icons::paint(ui, "x", xr, if xresp.hovered() { t.text_strong } else { t.text });
        ui.painter().galley(egui::pos2(r.left() + 32.0, r.center().y - galley.size().y / 2.0), galley, t.text);
        if xresp.clicked() {
            close = Some(i);
        } else if resp.clicked() {
            activate = Some(i);
        }
        x += w;
    }
    if let Some(i) = close {
        if let Err(e) = crate::unsaved::close(app, i) {
            app.status(e);
        }
    } else if let Some(i) = activate {
        app.ui.home = None;
        app.session.set_active(i);
    }
    // Isolation mode breadcrumb bar.
    if let Some(st) = app.session.active()
        && let Some(iso) = st.isolation
    {
        let mut crumbs: Vec<String> =
            st.doc.ancestry(iso).unwrap_or_default().iter().filter_map(|id| st.doc.node(*id)).map(|n| n.display_name()).collect();
        // Pattern editing mode: the pattern's name, and Save a Copy / Done / Cancel at the right.
        let pattern_edit = st.doc.pattern_edit.as_ref().map(|e| e.pattern.clone());
        if let Some(pn) = &pattern_edit {
            crumbs.push(pn.clone());
        }
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        ui.painter().rect_filled(bar, 0.0, t.panel);
        let back = egui::Rect::from_min_size(bar.min + vec2(6.0, 3.0), vec2(18.0, 18.0));
        let bresp = ui.interact(back, ui.id().with("iso-back"), Sense::click());
        icons::paint(ui, "chevron-left", back, if bresp.hovered() { t.text } else { t.icon });
        ui.painter().text(
            bar.left_center() + vec2(30.0, 0.0),
            egui::Align2::LEFT_CENTER,
            crumbs.join("  ›  "),
            egui::FontId::proportional(12.0),
            t.text,
        );
        let mut pattern_cmd = None;
        if pattern_edit.is_some() {
            let mut x = bar.right() - 6.0;
            for (label, cmd) in [("Cancel", "object.pattern.cancel"), ("Done", "object.pattern.done"), ("Save a Copy", "object.pattern.saveCopy")] {
                let label = tl!(label);
                let w = 12.0 + 7.0 * label.len() as f32;
                let r = egui::Rect::from_min_max(egui::pos2(x - w, bar.top() + 3.0), egui::pos2(x, bar.bottom() - 3.0));
                let resp = ui.interact(r, ui.id().with(("pat-bar", cmd)), Sense::click());
                ui.painter().rect_filled(r, 3.0, if resp.hovered() { t.hover } else { t.panel });
                ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), t.text);
                if resp.clicked() {
                    pattern_cmd = Some(cmd);
                }
                x -= w + 6.0;
            }
        }
        if bresp.clicked() {
            pattern_cmd = pattern_cmd.or(pattern_edit.as_ref().map(|_| "object.pattern.done"));
            if pattern_cmd.is_none() {
                app.run("object.exitIsolation", json!({})).ok();
            }
        }
        if let Some(cmd) = pattern_cmd {
            app.run(cmd, json!({})).ok();
        }
    }
}

pub fn status_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status_bar")
        .exact_size(24.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(8, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let zoom = app.view().map(|v| v.zoom).unwrap_or(1.0);
                egui::ComboBox::from_id_salt("zoom-combo").selected_text(egui::RichText::new(zoom_label(zoom)).size(11.5)).width(76.0).show_ui(
                    ui,
                    |ui| {
                        for z in ZOOM_STOPS.iter().rev() {
                            if ui.selectable_label((z / 100.0 - zoom).abs() < 1e-4, zoom_label(z / 100.0)).clicked() {
                                app.run("view.setZoom", json!({"zoom": z})).ok();
                            }
                        }
                        ui.separator();
                        if ui.selectable_label(false, tl!("Fit on Screen")).clicked() {
                            app.run("view.fitArtboard", json!({})).ok();
                        }
                        if ui.selectable_label(false, tl!("Fit All")).clicked() {
                            app.run("view.fitAll", json!({})).ok();
                        }
                    },
                );
                let rot = app.view().map(|v| v.rotation).unwrap_or(0.0);
                egui::ComboBox::from_id_salt("rot-combo").selected_text(egui::RichText::new(format!("{rot:.0}°")).size(11.5)).width(52.0).show_ui(
                    ui,
                    |ui| {
                        for a in [0.0, 15.0, 30.0, 45.0, 60.0, 90.0, 180.0, -15.0, -30.0, -45.0, -60.0, -90.0] {
                            if ui.selectable_label(rot == a, format!("{a:.0}°")).clicked()
                                && let Some(v) = app.view_mut()
                            {
                                v.rotation = a;
                            }
                        }
                    },
                );
                ui.separator();
                let nab = app.session.active().map(|d| d.doc.artboards.len()).unwrap_or(0);
                for icon in ["chevrons-left", "chevron-left"] {
                    widgets::icon_button(ui, icon, "", false, 18.0);
                }
                ui.label(egui::RichText::new(if nab > 0 { "1" } else { "–" }).size(11.5));
                for icon in ["chevron-right", "chevrons-right"] {
                    widgets::icon_button(ui, icon, "", false, 18.0);
                }
                ui.separator();
                let tool = vectorcraft_tools::tool_info(app.session.tool_id())
                    .map(|t| {
                        // The catalog keys carry the full label ("Selection Tool"); only English drops the suffix.
                        let shown = tl!(t.label);
                        if crate::i18n::current() == crate::i18n::Lang::EN { shown.trim_end_matches(" Tool") } else { shown }
                    })
                    .unwrap_or("");
                ui.label(egui::RichText::new(tool).size(11.5).color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "render {:.1} ms · ui {:.1} ms · {:.0} fps",
                            app.perf.render_ms, app.perf.frame_ms, app.perf.fps
                        ))
                        .size(10.5)
                        .color(t.text_disabled),
                    );
                    if let Some(p) = app.hover_doc {
                        let u = app.session.general_unit();
                        ui.label(
                            egui::RichText::new(format!("X: {}  Y: {}", u.readout(p.x), u.readout(p.y))).font(theme::mono(10.5)).color(t.text_dim),
                        );
                    }
                    // Background saves and exports in progress, else the last message.
                    if let Some(job) = app.background.jobs.first() {
                        let more = app.background.jobs.len() - 1;
                        let text = if more > 0 { format!("{}… (+{more})", job.label) } else { format!("{}…", job.label) };
                        ui.label(egui::RichText::new(text).size(11.0).color(t.text));
                        ui.add(egui::Spinner::new().size(12.0).color(t.accent));
                    } else if !app.ui.status.is_empty() {
                        ui.label(egui::RichText::new(&app.ui.status).size(11.0).color(t.text));
                    }
                });
            });
        });
}

/// Contextual hint for the active tool: segments of (text, bold). Keys in bold segments are
/// written as shortcuts are ("Alt+Click"); [`hint_segments`] names them for the platform.
fn hint_for(tool: &str) -> Option<&'static [(&'static str, bool)]> {
    Some(match tool {
        "selection" => &[
            ("Click", true),
            (" the object to select  |  ", false),
            ("Shift+Click", true),
            (" to select multiple objects  |  ", false),
            ("Alt+Drag", true),
            (" the object to duplicate", false),
        ],
        "directSelection" => &[
            ("Click", true),
            (" an anchor or path segment to select it  |  ", false),
            ("Drag", true),
            (" to move  |  ", false),
            ("Shift+Click", true),
            (" to add", false),
        ],
        "pen" => &[
            ("Click", true),
            (" to add a corner point  |  ", false),
            ("Drag", true),
            (" to add a smooth point  |  ", false),
            ("Click the first point", true),
            (" to close  |  ", false),
            ("Enter", true),
            (" to finish", false),
        ],
        "curvature" => &[
            ("Click", true),
            (" to add smooth points  |  ", false),
            ("Double-click", true),
            (" to toggle corner  |  ", false),
            ("Esc", true),
            (" to finish", false),
        ],
        "type" => &[
            ("Click", true),
            (" to create point type  |  ", false),
            ("Drag", true),
            (" to create area type  |  ", false),
            ("Esc", true),
            (" to exit editing", false),
        ],
        "rectangle" | "roundedRectangle" | "ellipse" | "polygon" | "star" => &[
            ("Drag", true),
            (" to draw  |  ", false),
            ("Shift+Drag", true),
            (" to constrain proportions  |  ", false),
            ("Alt+Drag", true),
            (" from center  |  ", false),
            ("Click", true),
            (" for exact size", false),
        ],
        "rotate" | "reflect" | "scale" | "shear" => &[
            ("Click", true),
            (" to set the reference point  |  ", false),
            ("Drag", true),
            (" to transform  |  ", false),
            ("Alt+Click", true),
            (" for exact values", false),
        ],
        "hand" => &[("Drag", true), (" to pan the view", false)],
        "zoom" => &[
            ("Click", true),
            (" to zoom in  |  ", false),
            ("Alt+Click", true),
            (" to zoom out  |  ", false),
            ("Drag", true),
            (" to zoom into an area", false),
        ],
        "eyedropper" => &[
            ("Click", true),
            (" an object to copy its attributes  |  ", false),
            ("Alt+Click", true),
            (" to apply the selection's to it  |  ", false),
            ("Shift+Click", true),
            (" to sample a color", false),
        ],
        "gradient" => &[("Drag", true), (" across a selected object to set the gradient direction", false)],
        "artboard" => &[("Click", true), (" to select an artboard  |  ", false), ("Drag", true), (" on the canvas to create one", false)],
        "width" => &[
            ("Drag", true),
            (" a stroke to add a width point  |  ", false),
            ("Alt+Drag", true),
            (" to change one side  |  ", false),
            ("Shift+Click", true),
            (" to select more points  |  ", false),
            ("Double-click", true),
            (" a point to edit it  |  ", false),
            ("Delete", true),
            (" to remove points", false),
        ],
        "puppetWarp" => &[
            ("Click", true),
            (" the art to add a pin  |  ", false),
            ("Shift+Click", true),
            (" to select more pins  |  ", false),
            ("Drag", true),
            (" a pin to warp  |  ", false),
            ("Alt+Drag", true),
            (" near a selected pin to rotate  |  ", false),
            ("Delete", true),
            (" to remove pins", false),
        ],
        "warp" => &[
            ("Drag", true),
            (" across paths to push them along  |  ", false),
            ("Alt+Drag", true),
            (" to size the brush (", false),
            ("Shift", true),
            (" keeps its proportions)", false),
        ],
        "twirl" | "pucker" | "bloat" => &[
            ("Click or drag", true),
            (" over paths, and hold still to keep going  |  ", false),
            ("Alt+Drag", true),
            (" to size the brush (", false),
            ("Shift", true),
            (" keeps its proportions)", false),
        ],
        "scallop" | "crystallize" | "wrinkle" => &[
            ("Click or drag", true),
            (" over paths to roughen their outlines  |  ", false),
            ("Alt+Drag", true),
            (" to size the brush (", false),
            ("Shift", true),
            (" keeps its proportions)", false),
        ],
        "perspectiveSelection" => &[
            ("Drag", true),
            (" to move in perspective  |  ", false),
            ("Alt+Drag", true),
            (" to copy  |  ", false),
            ("5", true),
            (" while dragging to move perpendicular to the plane  |  ", false),
            ("Drag a handle", true),
            (" to scale in perspective", false),
        ],
        "blend" => &[
            ("Click", true),
            (" an object, then another to blend them  |  ", false),
            ("Click an anchor point", true),
            (" to blend from it  |  ", false),
            ("Alt+Click", true),
            (" to set spacing and orientation", false),
        ],
        _ => return None,
    })
}

/// The hint bar's segments for `tool`, with keys named as the menus name them (Alt and Ctrl on
/// Windows and Linux, ⌥ and ⌘ on macOS). Tools without a hint point to Search Commands.
/// Every hint-bar fragment (the action after the last `+` of a chord included), so the catalog tests
/// can insist a complete language covers them.
pub fn hint_strings() -> Vec<&'static str> {
    let mut v = Vec::new();
    for tool in vectorcraft_tools::catalog::all_tools() {
        for &(text, bold) in hint_for(tool.id).unwrap_or(&[]) {
            let text = if bold { text.rsplit_once('+').map_or(text, |(_, k)| k) } else { text };
            if !text.is_empty() && !text.chars().all(|c| c.is_ascii_digit()) && !v.contains(&text) {
                v.push(text);
            }
        }
    }
    v
}

fn hint_segments(tool: &str) -> Vec<(Cow<'static, str>, bool)> {
    let search;
    let segments = match hint_for(tool) {
        Some(s) => s,
        None => {
            search = match menus::shortcut_of("help.commandPalette") {
                Some(sc) => [(tl!("Press "), false), (sc, true), (tl!(" to search every command"), false)],
                None => [(tl!("Use "), false), (tl!("Help › Search Commands…"), true), (tl!(" to search every command"), false)],
            };
            &search
        }
    };
    segments
        .iter()
        .map(|&(text, bold)| {
            // Key names ("Click", "Esc") translate; in a chord ("Alt+Drag") the modifiers are named for
            // the platform and only the action after the last `+` translates.
            if !bold {
                return (Cow::Borrowed(tl!(text)), bold);
            }
            match text.rsplit_once('+') {
                Some((_, key)) if !key.is_empty() => {
                    let pretty = menus::pretty_shortcut(text);
                    let shown = pretty.strip_suffix(key).map_or(pretty.clone(), |m| format!("{m}{}", tl!(key)));
                    (Cow::Owned(shown), bold)
                }
                _ => (Cow::Owned(menus::pretty_shortcut(tl!(text))), bold),
            }
        })
        .collect()
}

pub fn hint_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("hint_bar")
        .exact_size(28.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.2, t.text));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "?", theme::semibold(11.0), t.text);
                ui.add_space(6.0);
                let mut job = egui::text::LayoutJob::default();
                for (txt, bold) in hint_segments(app.session.tool_id()) {
                    let font = if bold { theme::semibold(12.5) } else { egui::FontId::proportional(12.5) };
                    job.append(&txt, 0.0, egui::TextFormat { font_id: font, color: if bold { t.text_strong } else { t.text }, ..Default::default() });
                }
                ui.label(job);
            });
        });
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    #[test]
    fn hints_name_keys_as_the_menus_do() {
        let text = |tool: &str| super::hint_segments(tool).into_iter().map(|(s, _)| s).collect::<String>();
        for tool in vectorcraft_tools::catalog::all_tools().map(|t| t.id) {
            let hint = text(tool);
            assert!(!hint.contains("Option"), "{tool}: {hint}");
            assert!(cfg!(target_os = "macos") || !hint.contains("Cmd"), "{tool}: {hint}");
        }
        let pretty = crate::menus::pretty_shortcut;
        assert!(text("selection").contains(&pretty("Alt+Drag")));
        assert!(text("rectangle").contains(&pretty("Alt+Drag")));
        assert!(text("zoom").contains(&pretty("Alt+Click")));
        assert!(text("rotate").contains(&pretty("Alt+Click")));
        assert!(text("eyedropper").contains(&pretty("Alt+Click")));
        assert!(text("perspectiveSelection").contains(&pretty("Alt+Drag")) && text("perspectiveSelection").contains("perpendicular"));
        assert!(text("paintbrush").contains(&pretty("Cmd+Shift+/")), "the Search Commands shortcut");
        if !cfg!(target_os = "macos") {
            assert!(text("zoom").contains("Alt+Click") && text("paintbrush").contains("Ctrl+Shift+/"));
        }
    }

    #[test]
    fn the_tab_title_names_the_opacity_mask_while_it_is_edited() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        let title = |s: &Session| super::tab_title(s.active().unwrap(), 1.0, false);
        assert!(title(&s).ends_with("(RGB/Preview)"), "{}", title(&s));
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        assert!(title(&s).ends_with("(<Opacity Mask>/Opacity Mask)"), "{}", title(&s));
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        assert!(title(&s).ends_with("(RGB/Preview)"));
    }
}
