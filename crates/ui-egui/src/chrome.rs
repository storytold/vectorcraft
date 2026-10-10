//! Window chrome: application bar (menus), Control bar, document tabs, status bar.

use std::borrow::Cow;

use egui::{CornerRadius, Sense, Stroke, Ui, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;

use crate::panels::stroke as stroke_panel;
use crate::state::{ZOOM_STOPS, zoom_label};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{VectorcraftApp, icons, menus, titlebar};

/// The application bar's height, in points (compact, as in PhotoCraft).
pub const APP_BAR_HEIGHT: f32 = 32.0;
/// The brand mark's side.
const MARK: f32 = 18.0;
/// Height of the workspace switcher.
const WIDGET_H: f32 = 22.0;

/// The application bar: brand mark, Home, menus, then Discord, search and the workspace switcher
/// at the right. With [`VectorcraftApp::custom_titlebar`] it is also the window's title bar
/// ([`titlebar`]): the caption buttons take the right end and the rest of the bar drags the window.
/// With the system title bar (Windows and Linux, `system_title_bar`) the OS draws its own icon and
/// title, so the in-app bar shows Home, menus and workspace controls only: no brand mark.
pub fn app_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let custom = app.custom_titlebar;
    let system = system_title_bar(app);
    let left = app.titlebar_inset.clamp(8.0, f32::from(i8::MAX)) as i8;
    let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin { left, right: if custom { 0 } else { 14 }, top: 0, bottom: 0 });
    let bar = egui::Panel::top("app_bar").exact_size(APP_BAR_HEIGHT).frame(frame.stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        if custom {
            titlebar::drag_area(ui, ui.max_rect());
        }
        ui.horizontal_centered(|ui| {
            // With the system title bar the OS draws its own icon: no in-app brand mark.
            if !system {
                let (r, _) = ui.allocate_exact_size(vec2(MARK, MARK), Sense::hover());
                crate::brand::paint_mark(ui, r);
                ui.add_space(6.0);
            }
            let on_home = menus::home_showing(app);
            if widgets::icon_button(ui, "house", tl!("Home"), on_home, 20.0).clicked() {
                app.run("app.home", json!({})).ok();
            }
            ui.add_space(2.0);
            // With the macOS menu bar the menus are at the top of the screen instead.
            let menus_end = if app.services.native_menu.is_some() {
                let full = ui.max_rect();
                ui.painter().text(full.center(), egui::Align2::CENTER_CENTER, "VectorCraft", egui::FontId::proportional(13.5), t.text);
                ui.cursor().min.x
            } else {
                menus::menu_bar(app, ui)
            };
            // The right-side group fills the space after the menus from the right; when it runs
            // short, Discord goes first (it is also under Help), then the search icon (the palette
            // stays under its shortcut), then the workspace switcher narrows.
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
            // The search icon, as in PhotoCraft: shown while a minimal switcher fits beside it.
            let icon = 28.0;
            let search = room >= 64.0 + 8.0 + gap + icon;
            let ws_w = ws_w.min(room - 8.0 - gap - if search { icon } else { 0.0 }).max(64.0);
            let discord = search && room >= ws_w + 8.0 + gap + icon + 10.0 + gap + crate::community::discord_width(ui, false);
            let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));
            let ui = &mut rui;
            // Workspace switcher: shows the current workspace, opens Window → Workspace.
            let (wr, wresp) = ui.allocate_exact_size(vec2(ws_w, WIDGET_H), Sense::click());
            ui.painter().rect_filled(wr, CornerRadius::same(4), if wresp.hovered() { t.hover } else { t.panel });
            ui.painter().with_clip_rect(wr.shrink2(vec2(4.0, 0.0))).galley(wr.left_center() + vec2(10.0, -ws.size().y / 2.0), ws, t.text);
            icons::paint(ui, "chevron-down", egui::Rect::from_center_size(wr.right_center() - vec2(12.0, 0.0), vec2(12.0, 12.0)), t.text_dim);
            let wresp = wresp.on_hover_text(tl!("Switch workspace"));
            egui::Popup::menu(&wresp).show(|ui| crate::workspaces::popup(app, ui));
            ui.add_space(8.0);
            // Search → command palette (its shortcut opens it too).
            if search {
                let tip = match menus::shortcut_of("help.commandPalette") {
                    Some(sc) => format!("{}  ({})", tl!("Search commands and tools"), menus::pretty_shortcut(sc)),
                    None => tl!("Search commands and tools").to_string(),
                };
                if widgets::icon_button(ui, "search", &tip, app.ui.palette_open, icon).clicked() {
                    app.ui.palette_open = !app.ui.palette_open;
                    app.ui.palette_query.clear();
                }
            }
            if discord {
                ui.add_space(10.0);
                crate::community::discord_button(app, ui, false);
            }
        });
    });
    if custom {
        titlebar::caption_buttons(app, ui, bar.response.rect);
    }
}

/// Whether the window shows the system's title bar instead of the app drawing its own: Windows
/// and Linux with Preferences › User Interface › System Title Bar. macOS always has system
/// decorations, but keeps the in-app brand mark, so it never counts as system mode here; nor
/// does the web build (the browser tab has no document title), nor an embedder that never
/// turned the preference on.
pub fn system_title_bar(app: &VectorcraftApp) -> bool {
    !app.custom_titlebar && app.session.prefs.system_title_bar && !cfg!(any(target_os = "macos", target_arch = "wasm32"))
}

/// The OS window title: the active document's name suffixed with the app name (a `*` prefix
/// marks unsaved changes, like the `*` in the document tabs), or just the app name with no
/// document open.
pub fn window_title(app: &VectorcraftApp) -> String {
    app.session
        .active()
        .map(|d| format!("{}{} \u{2014} VectorCraft", if d.is_dirty() { "*" } else { "" }, d.title()))
        .unwrap_or_else(|| "VectorCraft".into())
}

/// Keep the OS window title (and the taskbar / Alt-Tab entry) on the active file. Sends
/// `ViewportCommand::Title` only when the title changed since the last frame.
pub fn sync_window_title(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let want = window_title(app);
    if app.last_window_title != want {
        app.last_window_title = want.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(want));
        // One more frame: a new title makes AppKit lay its title bar out again, putting the window
        // buttons back where it keeps them until the next frame centres them on the bar (#968).
        ctx.request_repaint();
    }
}

/// An anchor button: icon, tooltip, command and the convert command's `to`.
type AnchorButton<'a> = (&'a str, &'a str, &'a str, Option<&'a str>);

/// The anchor controls the Control bar and the Properties panel show.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnchorControls {
    None,
    /// Paths selected as a whole with a tool that edits anchors (Direct Selection, the Pen…),
    /// which shows all their anchors selected: "Convert:" only, as removing or cutting at every
    /// anchor isn't what they're for.
    Convert,
    /// Anchors direct-selected: "Convert:" and "Anchors:".
    All,
}

/// Which anchor controls show for the selection and the active tool.
pub fn anchor_controls(app: &VectorcraftApp) -> AnchorControls {
    let Some(st) = app.session.active() else { return AnchorControls::None };
    if !st.selection.anchors.is_empty() {
        return AnchorControls::All;
    }
    let paths = st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. })));
    if paths && vectorcraft_tools::catalog::edits_anchors(app.session.tool_id()) { AnchorControls::Convert } else { AnchorControls::None }
}

/// What the Control bar and the Properties panel offer for selected anchors ([`anchor_controls`]):
/// "Convert:" corner or smooth, then "Anchors:" remove, connect (Join) and cut. Each group is a
/// row of its own: inline in the Control bar, one under the other in a panel.
pub fn anchor_buttons(app: &mut VectorcraftApp, ui: &mut Ui, controls: AnchorControls) {
    let t = Tokens::get(ui.ctx());
    let mut run = None;
    let groups: [(&str, &[AnchorButton]); 2] = [
        (
            tl!("Convert:"),
            &[
                ("dc-anchor", tl!("Convert Selected Anchor Points to Corner"), "path.convertAnchors", Some("corner")),
                ("dc-anchor-smooth", tl!("Convert Selected Anchor Points to Smooth"), "path.convertAnchors", Some("smooth")),
            ],
        ),
        (
            tl!("Anchors:"),
            &[
                ("pen-tool-delete", tl!("Remove Anchor Points"), "path.removeAnchors", None),
                ("dc-join", tl!("Connect Selected End Points"), "path.join", None),
                ("scissors", tl!("Cut Path at Selected Anchor Points"), "path.cutAtAnchors", None),
            ],
        ),
    ];
    let shown = match controls {
        AnchorControls::None => 0,
        AnchorControls::Convert => 1,
        AnchorControls::All => groups.len(),
    };
    for (label, buttons) in groups.into_iter().take(shown) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).size(12.0).color(t.text));
            for &(icon, tip, id, to) in buttons {
                if widgets::icon_button(ui, icon, tip, false, 24.0).clicked() {
                    run = Some((id, to.map_or_else(|| json!({}), |to| json!({ "to": to }))));
                }
            }
        });
    }
    if let Some((id, p)) = run {
        crate::menus::invoke(app, id, p);
    }
}

/// The room the Control bar's inline X/Y/W/H fields need; with less, only the Transform link shows.
const INLINE_TRANSFORM_WIDTH: f32 = 440.0;

/// The Control bar (Window → Control), context-sensitive like Illustrator's.
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
                let anchors = anchor_controls(app);
                let label = match &first {
                    Some(_) if sel.len() == 1 && anchors != AnchorControls::None => tl!("Anchor Point"),
                    Some(vectorcraft_doc::Node { kind: NodeKind::Image(im), .. }) if sel.len() == 1 => {
                        if im.link.is_some() {
                            tl!("Linked File")
                        } else {
                            tl!("Embedded")
                        }
                    }
                    Some(n) if sel.len() == 1 && crate::panels::image_trace::is_trace(n) => tl!("Image Tracing"),
                    _ => tl!(crate::panels::appearance::object_label(app)),
                };
                ui.label(egui::RichText::new(label).font(theme::semibold(12.0)).color(t.text));
                if anchors != AnchorControls::None {
                    anchor_buttons(app, ui, anchors);
                }
                ui.add_space(6.0);
                // An image or an Image Trace object shows its own controls in place of Fill and Stroke.
                let image = crate::place::control_bar_details(app, ui) || crate::panels::image_trace::control_bar(app, ui);
                crate::toolbar::control_bar_options(app, ui);
                crate::dialogs::envelope::control_bar(app, ui);
                let opacity = if first.is_some() { crate::panels::current_transparency(app).map_or(1.0, |t| t.0) } else { 1.0 };
                if !image {
                    let shown_stroke = crate::panels::current_stroke(app);
                    let mixed = crate::panels::stroke_mixed(app, ui.ctx());
                    let weight = stroke_panel::shown_weight(app, shown_stroke.as_ref(), &mixed);
                    crate::panels::paint_chip(app, ui, false, 22.0, true);
                    crate::panels::paint_chip(app, ui, true, 22.0, true);
                    // The link opens the Stroke panel as a popover under it; then the weight spinner
                    // (with presets) and the width profile.
                    stroke_panel::link(app, ui, tl!("Stroke:"));
                    stroke_panel::weight_field(app, ui, "cb-stroke", weight, 100.0);
                    if let Some(id) = stroke_panel::profile_dropdown(app, ui, shown_stroke.as_ref().and_then(|s| s.profile.as_ref())) {
                        app.run("stroke.set", json!({"profile": id})).ok();
                    }
                    ui.add_space(4.0);
                    ui.separator();
                }
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
                crate::panels::character::control_bar(app, ui);
                if sel.is_empty() {
                    if widgets::flat_button(ui, tl!("Document Setup"), 112.0).clicked() {
                        app.run("file.documentSetup", json!({})).ok();
                    }
                    if widgets::flat_button(ui, tl!("Preferences"), 90.0).clicked() {
                        app.run("edit.preferences", json!({})).ok();
                    }
                    return;
                }
                crate::panels::align::align_buttons(app, ui, 24.0);
                ui.separator();
                // The Transform link opens the whole Transform panel (reference point, rotate,
                // shear, options) in a popover; X/Y/W/H follow inline while the bar has room.
                crate::panels::transform::link(app, ui);
                // The bounding box, rotated with rotated objects: its centre and its own sides.
                if let Some(b) = app.selection_box()
                    && ui.available_width() >= INLINE_TRANSFORM_WIDTH
                {
                    let c = b.center();
                    let link = app.session.prefs.constrain_proportions;
                    for (k, lbl, v) in [("x", "X:", c.x), ("y", "Y:", c.y), ("width", "W:", b.rect.width()), ("height", "H:", b.rect.height())] {
                        // The W/H link sits between W and H.
                        if k == "height" {
                            crate::panels::transform::constrain_link(app, ui);
                        }
                        widgets::field_label(ui, egui::RichText::new(tl!(lbl)).size(12.0).color(t.text_dim));
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

/// Whether a document tab's × comes before its title (macOS) rather than after it (Windows,
/// Linux and the web).
const CLOSE_BEFORE_TITLE: bool = cfg!(target_os = "macos");

/// The first of tabs `widths` wide that the strip shows in `room`: the first one, or a later one so
/// that the `active` tab fits.
fn first_tab_shown(widths: &[f32], active: Option<usize>, room: f32) -> usize {
    let Some(a) = active else { return 0 };
    let mut first = 0;
    while first < a && widths.get(first..=a).map_or(0.0, |w| w.iter().sum::<f32>()) > room {
        first += 1;
    }
    first
}

/// Document tab strip: "Name* @ 66.67% (RGB/Preview)". User Interface › Large Tabs makes the tabs
/// taller, with larger titles. Tabs that don't fit are reached from a » button at the strip's
/// right end, which lists every open document, and the active tab is always in view (#746).
pub fn doc_tabs(app: &mut VectorcraftApp, ui: &mut Ui) {
    /// The width of the » button.
    const MORE: f32 = 30.0;
    let t = Tokens::get(ui.ctx());
    let (height, title_size) = if app.session.prefs.large_tabs { (44.0, 14.0) } else { (35.0, 12.5) };
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    // On the Home screen no tab is the current one.
    let active = app.session.active_index().filter(|_| app.ui.home.is_none());
    let tabs: Vec<(String, std::sync::Arc<egui::Galley>)> = app
        .session
        .documents()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let zoom = app.views.get(i).map(|v| v.zoom).unwrap_or(1.0);
            let title = tab_title(d, zoom, app.ui.view.outline);
            let color = if Some(i) == active { t.text_strong } else { t.text_dim };
            (title.clone(), ui.painter().layout_no_wrap(title, theme::semibold(title_size), color))
        })
        .collect();
    let widths: Vec<f32> = tabs.iter().map(|(_, g)| g.size().x + 50.0).collect();
    let overflow = widths.iter().sum::<f32>() > strip.width();
    let room = strip.width() - if overflow { MORE } else { 0.0 };
    let first = if overflow { first_tab_shown(&widths, active, room) } else { 0 };
    let mut x = strip.left();
    let mut activate = None;
    let mut close = None;
    for (i, ((_, galley), w)) in tabs.iter().zip(&widths).enumerate().skip(first) {
        if x + w > strip.left() + room && Some(i) != active {
            break;
        }
        let is_active = Some(i) == active;
        let r = egui::Rect::from_min_size(egui::pos2(x, strip.top()), vec2(*w, strip.height() - 1.0));
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        if is_active {
            ui.painter().rect_filled(r, 0.0, t.panel);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.4));
        }
        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.5, t.border));
        // The × where each platform puts a tab's: before the title on macOS, after it elsewhere
        // (#673).
        let (x_center, title_left) = if CLOSE_BEFORE_TITLE { (r.left() + 16.0, r.left() + 32.0) } else { (r.right() - 16.0, r.left() + 14.0) };
        let xr = egui::Rect::from_center_size(egui::pos2(x_center, r.center().y), vec2(12.0, 12.0));
        let xresp = ui.interact(xr.expand(3.0), ui.id().with(("tabx", i)), Sense::click());
        icons::paint(ui, "x", xr, if xresp.hovered() { t.text_strong } else { t.text });
        ui.painter().galley(egui::pos2(title_left, r.center().y - galley.size().y / 2.0), galley.clone(), t.text);
        if xresp.clicked() {
            close = Some(i);
        } else if resp.clicked() {
            activate = Some(i);
        }
        x += w;
    }
    // The » button: every open document, the active one checked.
    if overflow {
        let more = egui::Rect::from_min_max(egui::pos2(strip.right() - MORE, strip.top()), egui::pos2(strip.right(), strip.bottom() - 1.0));
        let resp = ui.interact(more, ui.id().with("tabs-more"), Sense::click());
        ui.painter().rect_filled(more, 0.0, if resp.hovered() { t.hover.gamma_multiply(0.4) } else { t.tab_strip });
        icons::paint(ui, "chevrons-right", egui::Rect::from_center_size(more.center(), vec2(14.0, 14.0)), t.text);
        let resp = resp.on_hover_text(tl!("Show all open documents"));
        egui::Popup::menu(&resp).show(|ui| {
            for (i, (title, _)) in tabs.iter().enumerate() {
                if widgets::menu_item_name(ui, title, true, Some(i) == active) {
                    activate = Some(i);
                    ui.close();
                }
            }
        });
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
                // The artboard navigator: first, previous, the current artboard's number, next, last.
                let nab = app.session.active().map(|d| d.doc.artboards.len()).unwrap_or(0);
                let cur = app.view().map_or(0, |v| v.artboard).min(nab.saturating_sub(1));
                let mut go = None;
                for (icon, to) in [("chevrons-left", "first"), ("chevron-left", "previous")] {
                    if widgets::icon_button_enabled(ui, icon, "", false, cur > 0, 18.0).clicked() {
                        go = Some(to);
                    }
                }
                ui.label(egui::RichText::new(if nab > 0 { (cur + 1).to_string() } else { "–".into() }).size(11.5));
                for (icon, to) in [("chevron-right", "next"), ("chevrons-right", "last")] {
                    if widgets::icon_button_enabled(ui, icon, "", false, cur + 1 < nab, 18.0).clicked() {
                        go = Some(to);
                    }
                }
                if let Some(to) = go
                    && let Err(e) = app.run("view.goToArtboard", json!({ "index": to }))
                {
                    app.status(e);
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
                        let label = crate::i18n::msg(&job.label);
                        let text = if more > 0 { format!("{label}… (+{more})") } else { format!("{label}…") };
                        ui.label(egui::RichText::new(text).size(11.0).color(t.text));
                        ui.add(egui::Spinner::new().size(12.0).color(t.accent));
                    } else if !app.ui.status.is_empty() {
                        // The message stays English in `ui.status` (agents and tests read it).
                        ui.label(egui::RichText::new(crate::i18n::msg(&app.ui.status)).size(11.0).color(t.text));
                    }
                });
            });
        });
}

/// Contextual hint for the active tool: segments of (text, bold). Keys in bold segments are
/// written as shortcuts are ("Alt+Click"); [`hint_segments`] names them for the platform.
/// The hint of the Zoom tool with Performance › Animated Zoom on.
const ANIMATED_ZOOM_HINT: &str = "zoom.animated";

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
        ANIMATED_ZOOM_HINT => &[
            ("Click", true),
            (" to zoom in  |  ", false),
            ("Alt+Click", true),
            (" to zoom out  |  ", false),
            ("Drag", true),
            (" right or left to zoom in or out  |  ", false),
            ("Hold", true),
            (" to keep zooming", false),
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
    for tool in vectorcraft_tools::catalog::all_tools().map(|t| t.id).chain([ANIMATED_ZOOM_HINT]) {
        for &(text, bold) in hint_for(tool).unwrap_or(&[]) {
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
                let segments = if let Some(d) = app.ui.dialog.as_ref().filter(|d| crate::dialogs::revolve_gizmo::active(d)) {
                    vec![(crate::dialogs::revolve_gizmo::hint(d).into(), false)]
                } else {
                    let tool = app.session.tool_id();
                    let hint = if tool == "zoom" && crate::canvas::animated_zoom(&app.session.prefs) { ANIMATED_ZOOM_HINT } else { tool };
                    hint_segments(hint)
                };
                for (txt, bold) in segments {
                    let font = if bold { theme::semibold(12.5) } else { egui::FontId::proportional(12.5) };
                    job.append(&txt, 0.0, egui::TextFormat { font_id: font, color: if bold { t.text_strong } else { t.text }, ..Default::default() });
                }
                ui.label(job);
            });
        });
}

#[cfg(test)]
mod tests {

    /// With more tabs than fit, the strip starts late enough to show the active one (#746).
    #[test]
    fn the_active_tab_stays_in_view() {
        let widths = [100.0; 10];
        assert_eq!(super::first_tab_shown(&widths, Some(2), 350.0), 0, "it fits from the start");
        assert_eq!(super::first_tab_shown(&widths, Some(9), 350.0), 7, "the last three tabs shown");
        assert_eq!(super::first_tab_shown(&widths, None, 350.0), 0);
        assert_eq!(super::first_tab_shown(&widths, Some(4), 50.0), 4, "a tab wider than the room still shows");
    }
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

    /// A document tab's × sits where the platform puts it (#673): after the title on Windows,
    /// Linux and the web, before it on macOS; clicking it closes that document.
    #[test]
    fn a_tabs_close_box_sits_where_the_platform_puts_it() {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 200.0));
        let frame = |app: &mut crate::VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::doc_tabs(app, ui));
            out.textures_delta.clear();
            let mut titles = vec![];
            for c in &out.shapes {
                // The titles, not the × icons.
                if let egui::Shape::Text(t) = &c.shape
                    && t.visual_bounding_rect().width() > 30.0
                {
                    titles.push(t.visual_bounding_rect());
                }
            }
            titles.sort_by(|a, b| a.left().total_cmp(&b.left()));
            titles
        };
        let titles = frame(&mut app, vec![]);
        assert_eq!(titles.len(), 2, "{titles:?}");
        // The first tab's ×: 20 pt past its title's end, or 16 pt before its start.
        let first = titles[0];
        let x = if super::CLOSE_BEFORE_TITLE { first.left() - 16.0 } else { first.right() + 20.0 };
        let at = egui::pos2(x, first.center().y);
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(&mut app, vec![press(false)]);
        assert_eq!(app.session.documents().len(), 1, "the × closed the first document");
        assert!(super::CLOSE_BEFORE_TITLE == cfg!(target_os = "macos"));
    }

    /// One headless frame of the status bar; returns the artboard navigator's buttons, left to
    /// right: first, previous, next, last.
    fn navigator(app: &mut crate::VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::Rect> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 300.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::status_bar(app, ui));
        out.textures_delta.clear();
        let size = egui::vec2(18.0, 18.0);
        let mut r: Vec<egui::Rect> =
            ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size).map(|w| w.rect).collect());
        r.sort_by(|a, b| a.left().total_cmp(&b.left()));
        r
    }

    #[test]
    fn the_artboard_navigator_goes_to_artboards_and_fitting_shows_the_current_one() {
        use vectorcraft_geom::Point;
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        // Three 200 × 100 artboards in a row, centred at x = 100, 320 and 540.
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("artboard.new", json!({})).unwrap();
        app.run("artboard.new", json!({})).unwrap();
        app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 260.0)));
        let ctx = egui::Context::default();
        let buttons = navigator(&mut app, &ctx, vec![]);
        assert_eq!(buttons.len(), 4);
        let click = |app: &mut crate::VectorcraftApp, r: egui::Rect| {
            let b = |pressed| egui::Event::PointerButton {
                pos: r.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            navigator(app, &ctx, vec![egui::Event::PointerMoved(r.center())]);
            navigator(app, &ctx, vec![b(true)]);
            navigator(app, &ctx, vec![b(false)]);
        };
        let at = |app: &crate::VectorcraftApp| (app.view().unwrap().artboard, app.view().unwrap().center);
        click(&mut app, buttons[2]);
        assert_eq!(at(&app), (1, Point::new(320.0, 50.0)), "next");
        click(&mut app, buttons[3]);
        assert_eq!(at(&app), (2, Point::new(540.0, 50.0)), "last");
        click(&mut app, buttons[1]);
        assert_eq!(at(&app), (1, Point::new(320.0, 50.0)), "previous");
        // Fit Artboard in Window and Actual Size show the navigator's artboard.
        app.run("view.setZoom", json!({"zoom": 300, "center": [0, 0]})).unwrap();
        app.run("view.fitArtboard", json!({})).unwrap();
        assert_eq!(at(&app), (1, Point::new(320.0, 50.0)));
        app.run("view.actualSize", json!({})).unwrap();
        assert_eq!((at(&app).1, app.view().unwrap().zoom), (Point::new(320.0, 50.0), 1.0));
        click(&mut app, buttons[0]);
        assert_eq!(at(&app), (0, Point::new(100.0, 50.0)), "first");
        // The command also takes an index, clamped to the last artboard.
        assert_eq!(app.run("view.goToArtboard", json!({"index": 9})), Ok(json!({"index": 2})));
        assert!(app.run("view.goToArtboard", json!({"index": "sideways"})).is_err());
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

    /// The Zoom tool's hint follows Performance › Animated Zoom (#394): a drag zooms as it goes,
    /// or (off) zooms into the area dragged across.
    #[test]
    fn the_zoom_hint_follows_animated_zoom() {
        fn texts(s: &egui::Shape, out: &mut String) {
            match s {
                egui::Shape::Text(t) => out.push_str(t.galley.text()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.select_tool("zoom");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let hint = |app: &mut crate::VectorcraftApp| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::hint_bar(app, ui));
            out.textures_delta.clear();
            let mut shown = String::new();
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
            shown
        };
        let on = hint(&mut app);
        assert!(on.contains(" right or left to zoom in or out") && on.contains("Hold"), "{on}");
        app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
        let off = hint(&mut app);
        assert!(off.contains(" to zoom into an area") && !off.contains("Hold"), "{off}");
    }

    fn title_test_app(custom_titlebar: bool) -> crate::VectorcraftApp {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.custom_titlebar = custom_titlebar;
        // The preference that makes the shell launch without its own title bar.
        app.run("prefs.set", json!({"key": "systemTitleBar", "value": !custom_titlebar})).unwrap();
        app
    }

    #[test]
    fn without_the_preference_the_mark_stays() {
        // The default (e.g. the web build): no custom bar, no preference.
        let app = crate::VectorcraftApp::new(Session::new(), Default::default());
        assert!(!super::system_title_bar(&app));
    }

    #[test]
    fn window_title_with_no_document_is_the_app_name() {
        assert_eq!(super::window_title(&title_test_app(true)), "VectorCraft");
    }

    #[test]
    fn window_title_follows_the_active_file() {
        let mut app = title_test_app(true);
        app.run("file.new", json!({"width": 100, "height": 100, "name": "foo.svg"})).unwrap();
        assert_eq!(super::window_title(&app), "foo.svg \u{2014} VectorCraft");
        // Unsaved changes gain a `*` prefix, like the `*` in the document tabs.
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        assert_eq!(super::window_title(&app), "*foo.svg \u{2014} VectorCraft");
    }

    #[test]
    fn system_mode_hides_the_mark() {
        for custom in [true, false] {
            let mut app = title_test_app(custom);
            app.run("file.new", json!({"width": 100, "height": 100, "name": "foo.svg"})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::app_bar(&mut app, ui));
            let uploads = out.textures_delta.set.values().flat_map(|d| d.iter()).filter(|d| d.image.size() == [128, 128]).count();
            out.textures_delta.clear();
            let marks: Vec<egui::Rect> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Mesh(m) if m.texture_id == crate::brand::texture_id(&ctx) => Some(m.calc_bounds()),
                    _ => None,
                })
                .collect();
            let system = super::system_title_bar(&app);
            assert_eq!(system, !custom && !cfg!(target_os = "macos"));
            assert_eq!(marks.is_empty(), system, "mark in system mode (custom={custom})");
            assert_eq!(uploads, if system { 0 } else { 1 }, "mark texture in system mode (custom={custom})");
        }
    }

    #[test]
    fn sync_window_title_sends_only_on_change() {
        let mut app = title_test_app(true);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        super::sync_window_title(&mut app, &ctx);
        assert_eq!(app.last_window_title, "VectorCraft");
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        assert!(cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(t) if t == "VectorCraft")), "{cmds:?}");
        // Same document state again: the cached title stops a repeat command.
        super::sync_window_title(&mut app, &ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        assert!(!cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(_))), "repeat Title: {cmds:?}");
        app.run("file.new", json!({"width": 100, "height": 100, "name": "foo.svg"})).unwrap();
        super::sync_window_title(&mut app, &ctx);
        assert_eq!(app.last_window_title, "foo.svg \u{2014} VectorCraft");
    }
}
