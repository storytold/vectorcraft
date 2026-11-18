//! Window chrome: application bar (menus), Control bar, document tabs, status bar.

use std::borrow::Cow;
use std::sync::Arc;

use egui::{CornerRadius, Sense, Stroke, Ui, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;

use crate::panels::stroke as stroke_panel;
use crate::state::{ROTATE_VIEW_ANGLES, ZOOM_STOPS, zoom_label};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{VectorcraftApp, icons, menus, titlebar};

mod control_bar;
mod hints;
mod status_bar;
mod tabs;

pub use control_bar::{
    AnchorControls, CONTROL_BAR_EDGES, CONTROL_BAR_GROUPS, anchor_buttons, anchor_controls, control_bar, control_bar_moving, control_bar_placement,
    read_hidden_groups,
};
#[cfg(test)]
use control_bar::{CONTROL_BAR_FLOAT_AT, ISOLATION_BUTTON};
pub(crate) use control_bar::{Fit, group_separator, left_out, link_reserve};
#[cfg(test)]
use hints::hint_segments;
pub use hints::{hint_bar, hint_strings};
pub use status_bar::{StatusBarShow, deserialize_status_bar_show, set_status_bar_show, status_bar};
#[cfg(test)]
use status_bar::{StatusReadoutWidths, color_profile_name, date_time_label, status_readouts};
#[cfg(test)]
use tabs::{CLOSE_BEFORE_TITLE, first_tab_shown, tab_drop, tab_groups, tab_title};
pub use tabs::{doc_tabs, isolation_bar, step_document};
pub(crate) use tabs::{document_window_title, show_document, window_tab};

/// The application bar's height, in points (compact).
pub const APP_BAR_HEIGHT: f32 = 32.0;
/// The brand mark's side.
const MARK: f32 = 18.0;
/// Height of the workspace switcher.
const WIDGET_H: f32 = 22.0;

/// The application bar: brand mark, Home, menus, then Discord, search and the workspace switcher
/// at the right. With [`VectorcraftApp::custom_titlebar`] it is also the window's title bar
/// ([`titlebar`]): the caption buttons take the ends [`VectorcraftApp::caption_layout`] names and the
/// rest of the bar drags the window.
/// On macOS, under the system's transparent title bar, its empty space drags and zooms the window
/// too ([`titlebar::drag_area`]). With the system title bar (Windows and Linux, `system_title_bar`) the OS draws its own icon and
/// title, so the in-app bar shows Home, menus and workspace controls only: no brand mark.
pub fn app_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let custom = app.custom_titlebar;
    let system = system_title_bar(app);
    let left = app.titlebar_inset.clamp(8.0, f32::from(i8::MAX)) as i8;
    // The room the caption buttons take at each end of the bar.
    let (captions_left, captions_right) = if custom { (app.caption_layout.left_width(), app.caption_layout.right_width()) } else { (0.0, 0.0) };
    let right_margin = if captions_right > 0.0 { 0 } else { 14 };
    let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin { left, right: right_margin, top: 0, bottom: 0 });
    let bar = egui::Panel::top("app_bar").exact_size(APP_BAR_HEIGHT).frame(frame.stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        // macOS draws the bar under its transparent title bar, so the bar gets the title bar's
        // clicks: without this a double-click there zooms nothing (#1932).
        if custom || cfg!(target_os = "macos") {
            titlebar::drag_area(ui, ui.max_rect());
        }
        ui.horizontal_centered(|ui| {
            if captions_left > 0.0 {
                ui.add_space(captions_left);
            }
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
            let menus_end = if app.services.native_menu.is_some() { ui.cursor().min.x } else { menus::menu_bar(app, ui) };
            // The right-side group fills the space after the menus from the right, in the order
            // [`fit_right_group`] gives.
            let full = ui.max_rect();
            let right_edge = if captions_right > 0.0 { full.right() - captions_right - 10.0 } else { full.right() };
            let right = egui::Rect::from_min_max(egui::pos2(menus_end + 8.0, full.top()), egui::pos2(right_edge, full.bottom()));
            // In Safe Mode, a label that stays in the bar whatever document is open, and opens
            // Help › Safe Mode; the other controls fit in the room it leaves, and the workspace
            // switcher (also under Window › Workspace) goes when a minimal one doesn't fit.
            let safe = vectorcraft_plugins::safe_mode::active().then(|| egui::RichText::new(tl!("Safe Mode")).color(t.warning).size(12.0));
            let safe_w = safe.as_ref().map_or(0.0, |label| {
                let text = ui.painter().layout_no_wrap(label.text().to_string(), egui::FontId::proportional(12.0), t.warning);
                text.size().x + 2.0 * ui.spacing().button_padding.x + 8.0 + ui.spacing().item_spacing.x
            });
            // Built-in workspace names translate; the user's own stay as typed.
            let ws_name =
                if crate::workspaces::is_builtin(&app.ui.workspace) { tl!(&app.ui.workspace).to_string() } else { app.ui.workspace.clone() };
            let ws = ui.painter().layout_no_wrap(ws_name, egui::FontId::proportional(12.0), t.text);
            let shortcut = menus::shortcut_of("help.commandPalette").map(menus::shown_shortcut);
            let field = SearchField::new(ui, &t, shortcut);
            let fit = fit_right_group(&RightWidths {
                room: (right.width() - safe_w).max(0.0),
                safe_mode: safe.is_some(),
                switcher: (ws.size().x + 36.0).clamp(112.0, 190.0),
                field: field.width,
                icon: BAR_ICON,
                gap: ui.spacing().item_spacing.x,
                community: crate::community::discord_width(ui, false),
            });
            let (switcher, ws_w, arrange, learn, discord) = (fit.switcher, fit.switcher_w, fit.arrange, fit.learn, fit.community);
            let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));
            let ui = &mut rui;
            // Workspace switcher: shows the current workspace, opens Window → Workspace.
            if switcher {
                let (wr, wresp) = ui.allocate_exact_size(vec2(ws_w, WIDGET_H), Sense::click());
                ui.painter().rect_filled(wr, CornerRadius::same(4), if wresp.hovered() { t.hover } else { t.panel });
                ui.painter().with_clip_rect(wr.shrink2(vec2(4.0, 0.0))).galley(wr.left_center() + vec2(10.0, -ws.size().y / 2.0), ws, t.text);
                icons::paint(ui, "chevron-down", egui::Rect::from_center_size(wr.right_center() - vec2(12.0, 0.0), vec2(12.0, 12.0)), t.text_dim);
                let wresp = wresp.on_hover_text(tl!("Switch workspace"));
                egui::Popup::menu(&wresp).show(|ui| crate::workspaces::popup(app, ui));
                ui.add_space(8.0);
            }
            if let Some(label) = safe {
                if ui.add(egui::Button::new(label).frame(false)).on_hover_text(tl!("Safe Mode…")).clicked() {
                    app.run("help.safeMode", json!({})).ok();
                }
                ui.add_space(8.0);
            }
            // Arrange Documents: Consolidate All Windows and the Tile layouts.
            if arrange {
                let resp = widgets::icon_button(ui, "layout-grid", tl!("Arrange Documents"), app.ui.tiled, BAR_ICON);
                egui::Popup::menu(&resp).show(|ui| arrange_documents_popup(app, ui));
            }
            // Search → command palette (its shortcut opens it too).
            let with_shortcut = |text: &str| match &field.shortcut {
                Some(sc) => format!("{text}  ({sc})"),
                None => text.to_string(),
            };
            let tip = with_shortcut(tl!("Search commands, tools, panels and effects"));
            let name = with_shortcut(tl!("Search commands"));
            // The field and the icon toggle the palette as its shortcut does, and like the shortcut
            // they do nothing while a dialog is open, so they are drawn disabled then.
            let enabled = app.ui.dialog.is_none();
            let open = app.ui.palette_open;
            let clicked = ui
                .add_enabled_ui(enabled, |ui| match fit.search {
                    Search::Field => field.show(ui, &t, &name, &tip, open).clicked(),
                    Search::Icon => {
                        let (_, resp) = ui.allocate_exact_size(vec2(BAR_ICON, BAR_ICON), Sense::click());
                        widgets::paint_icon_button_named(ui, resp, "search", &name, &tip, open).clicked()
                    }
                    Search::Hidden => false,
                })
                .inner;
            if clicked && enabled {
                app.ui.palette_open = !app.ui.palette_open;
                app.ui.palette_query.clear();
            }
            if learn {
                let shown = crate::panels::learn::visible(&app.ui);
                if widgets::icon_button(ui, "graduation-cap", tl!("Explore tutorials"), shown, BAR_ICON).clicked()
                    && let Err(e) = app.run("window.learn", json!({}))
                {
                    app.ui.status = e;
                }
            }
            if discord {
                ui.add_space(10.0);
                crate::community::discord_button(app, ui, false);
            }
            if app.services.native_menu.is_some() {
                let title = ui.painter().layout_no_wrap("VectorCraft".into(), egui::FontId::proportional(13.5), t.text);
                let title_rect = egui::Rect::from_center_size(full.center(), title.size());
                // Keep the title centered only while the interactive groups leave room for it.
                if title_rect.left() >= menus_end + 8.0 && title_rect.right() <= ui.min_rect().left() - 8.0 {
                    ui.painter().galley(title_rect.min, title, t.text);
                }
            }
        });
    });
    if custom {
        titlebar::caption_buttons(app, ui, bar.response.rect);
    }
}

/// The Arrange Documents menu ([`menus::arrange_documents_items`]): each item as the Window menu
/// shows it, grayed out while its command can't run, the Tile layout the panes show checked.
fn arrange_documents_popup(app: &mut VectorcraftApp, ui: &mut Ui) {
    ui.set_min_width(200.0);
    let mut clicked = None;
    for it in menus::arrange_documents_items() {
        match it {
            menus::Item::Sep => {
                ui.separator();
            }
            menus::Item::Cmd(label, id, p) => {
                let Some((enabled, checked)) = menus::shown_state(app, id, &p) else { continue };
                let label = crate::i18n::t_id(id, label);
                let text = if checked == Some(true) { format!("✓  {label}") } else { format!("     {label}") };
                if ui.add_enabled(enabled, egui::Button::new(text).min_size(vec2(200.0, 0.0))).clicked() {
                    clicked = Some((id, if p.is_null() { json!({}) } else { p }));
                    ui.close();
                }
            }
            _ => {}
        }
    }
    if let Some((id, p)) = clicked {
        menus::invoke(app, id, p);
    }
}

/// The side of the app bar's square icon buttons (the search icon, Arrange Documents, Explore tutorials).
const BAR_ICON: f32 = 28.0;
/// The narrowest the workspace switcher gets.
const MIN_SWITCHER: f32 = 64.0;

/// How the app bar shows the search that opens the command palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Search {
    /// A field with the magnifier, "Search commands" and the palette's shortcut.
    Field,
    /// The magnifier alone.
    Icon,
    /// Not in the bar; the palette stays under its shortcut and Help › Search Commands….
    Hidden,
}

/// The widths [`fit_right_group`] fits into `room`, in points.
#[derive(Clone, Copy, Debug)]
struct RightWidths {
    /// The room after the menus, less the Safe Mode label's.
    room: f32,
    /// Whether the bar shows the Safe Mode label.
    safe_mode: bool,
    /// The workspace switcher's full width.
    switcher: f32,
    /// The search field's width.
    field: f32,
    /// The side of a square icon button.
    icon: f32,
    /// The space between two controls.
    gap: f32,
    /// The community button's width.
    community: f32,
}

/// What the right-hand group of the app bar shows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RightGroup {
    /// Whether the workspace switcher shows.
    switcher: bool,
    /// The workspace switcher's width.
    switcher_w: f32,
    /// How the search shows.
    search: Search,
    /// Whether Arrange Documents shows.
    arrange: bool,
    /// Whether Explore tutorials shows.
    learn: bool,
    /// Whether the community button shows.
    community: bool,
}

/// The right-hand group for the room it has. As the room runs short, the community button goes first
/// (it is also under Help), then Explore tutorials (also Window › Learn), then Arrange Documents (also Window ›
/// Arrange), then the search field becomes the search icon, then the workspace switcher narrows to [`MIN_SWITCHER`], then the icon goes. In Safe
/// Mode the label's width is taken first, and the switcher (also under Window › Workspace) goes when
/// a minimal one does not fit.
fn fit_right_group(w: &RightWidths) -> RightGroup {
    let room = w.room;
    let switcher = !w.safe_mode || room >= MIN_SWITCHER + 8.0;
    let icon = switcher && room >= MIN_SWITCHER + 8.0 + w.gap + w.icon;
    let field = icon && room >= w.switcher + 8.0 + w.gap + w.field;
    let search = if field {
        Search::Field
    } else if icon {
        Search::Icon
    } else {
        Search::Hidden
    };
    let search_w = match search {
        Search::Field => w.gap + w.field,
        Search::Icon => w.gap + w.icon,
        Search::Hidden => 0.0,
    };
    let switcher_w = w.switcher.min(room - 8.0 - search_w).max(MIN_SWITCHER);
    let arrange_end = switcher_w + 8.0 + search_w + w.gap + w.icon;
    let arrange = field && room >= arrange_end;
    let learn_end = arrange_end + w.gap + w.icon;
    let learn = arrange && room >= learn_end;
    let community = learn && room >= learn_end + 10.0 + w.gap + w.community;
    RightGroup { switcher, switcher_w, search, arrange, learn, community }
}

/// The app bar's search field: a button drawn as a field. The command palette's popup holds the
/// only text field.
struct SearchField {
    hint: Arc<egui::Galley>,
    /// The palette's shortcut as the menus show it; none when the user cleared it.
    shortcut: Option<String>,
    key: Option<Arc<egui::Galley>>,
    width: f32,
}

impl SearchField {
    /// Space from the field's left edge to the hint text (the magnifier sits in it).
    const TEXT_LEFT: f32 = 26.0;
    /// The widest the field gets.
    const MAX_WIDTH: f32 = 220.0;

    fn new(ui: &Ui, t: &Tokens, shortcut: Option<String>) -> SearchField {
        let hint = ui.painter().layout_no_wrap(tl!("Search commands").to_string(), egui::FontId::proportional(12.0), t.text_dim);
        let text_w = Self::TEXT_LEFT + hint.size().x + 8.0;
        // A shortcut that leaves no room for the hint in the widest field is left to the tooltip.
        let key = shortcut
            .as_ref()
            .map(|s| ui.painter().layout_no_wrap(s.clone(), egui::FontId::proportional(11.0), t.text_dim))
            .filter(|k| text_w + 12.0 + k.size().x <= Self::MAX_WIDTH);
        let key_w = key.as_ref().map_or(0.0, |k| 12.0 + k.size().x);
        let width = (text_w + key_w + 24.0).clamp(120.0, Self::MAX_WIDTH);
        SearchField { hint, shortcut, key, width }
    }

    /// Draws the field. A click, or Enter or Space with the keyboard on it, clicks it. Screen
    /// readers get a button called `name`, and hovering it shows `tip`.
    fn show(self, ui: &mut Ui, t: &Tokens, name: &str, tip: &str, open: bool) -> egui::Response {
        let (r, resp) = ui.allocate_exact_size(vec2(self.width, WIDGET_H), Sense::click());
        let fill = if open {
            t.tool_active
        } else if resp.hovered() {
            t.hover
        } else {
            t.input
        };
        let border = if resp.has_focus() { t.accent } else { t.input_border };
        ui.painter().rect(r, CornerRadius::same(4), fill, Stroke::new(1.0, border), egui::StrokeKind::Inside);
        icons::paint(ui, "search", egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(14.0, 14.0)), t.icon);
        let mut text_right = r.right() - 8.0;
        if let Some(key) = self.key {
            let at = egui::pos2(text_right - key.size().x, r.center().y - key.size().y / 2.0);
            text_right = at.x - 12.0;
            ui.painter().with_clip_rect(r).galley(at, key, t.text_dim);
        }
        let clip = egui::Rect::from_min_max(r.min, egui::pos2(text_right.max(r.left()), r.bottom()));
        let at = egui::pos2(r.left() + Self::TEXT_LEFT, r.center().y - self.hint.size().y / 2.0);
        ui.painter().with_clip_rect(clip).galley(at, self.hint, t.text_dim);
        resp.widget_info(|| widgets::icon_button_info(ui.is_enabled(), open, name));
        resp.on_hover_text(tip)
    }
}

/// Whether the window shows the system's title bar instead of the app drawing its own
/// ([`VectorcraftApp::desktop_frame`]): Windows with Preferences › User Interface › System Title
/// Bar, and Linux whenever the desktop draws the frame, with or without the preference. The
/// preference is read when the app starts, so changing it later does not change this. macOS
/// always has system decorations, but keeps the in-app brand mark, so it never counts as system
/// mode here; nor does the web build (the browser tab has no document title), nor an embedder
/// without a window of its own.
pub fn system_title_bar(app: &VectorcraftApp) -> bool {
    app.desktop_frame && !app.custom_titlebar && !cfg!(any(target_os = "macos", target_arch = "wasm32"))
}

/// The OS window title: the active document's name suffixed with the app name (a `*` prefix
/// marks unsaved changes, like the `*` in the document tabs), or just the app name with no
/// document open; in Safe Mode, followed by "(Safe Mode)".
pub fn window_title(app: &VectorcraftApp) -> String {
    let title = app
        .session
        .active()
        .map(|d| format!("{}{} \u{2014} VectorCraft", if d.is_dirty() { "*" } else { "" }, d.title()))
        .unwrap_or_else(|| "VectorCraft".into());
    if vectorcraft_plugins::safe_mode::active() { format!("{title} ({})", tl!("Safe Mode")) } else { title }
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

/// Full Screen Mode and Presentation Mode fill the screen (#4365: the taskbar and the window's
/// frame stayed): the window goes full screen when one of them starts, and back when a mode with
/// the menu bar returns. A window that was full screen already stays so.
pub fn sync_fullscreen(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let want = app.ui.screen_mode >= 2;
    let is = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    if want && !is && !app.fullscreen_for_mode {
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        app.fullscreen_for_mode = true;
    } else if !want && app.fullscreen_for_mode {
        if is {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        app.fullscreen_for_mode = false;
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod visual_regressions {
    use super::*;

    fn scaled_input(size: egui::Vec2, scale: f32) -> egui::RawInput {
        let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
        input
    }

    struct NativeMenus;
    impl crate::native_menu::Backend for NativeMenus {
        fn sync(&mut self, _: &crate::native_menu::MenuBar) {}
        fn drain(&mut self) -> Vec<crate::native_menu::Event> {
            Vec::new()
        }
    }

    #[test]
    fn capture_chrome_visual_fixtures() {
        let Some(directory) = std::env::var_os("CRAFT_UI_VISUAL_FIXTURES").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&directory).unwrap();
        for brightness in theme::Brightness::ALL {
            for width in [800.0, 1280.0] {
                for scale in [1.0, 1.5, 2.0] {
                    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
                    app.services.native_menu = Some(crate::native_menu::NativeMenu::new(Box::new(NativeMenus)));
                    app.ui.status = format!("Saved /Users/example/Documents/{}/illustration.svg", "long-folder-name/".repeat(10));
                    let mut ready = false;
                    let mut harness =
                        egui_kittest::Harness::builder().with_size(vec2(width, 200.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                            move |ui, app: &mut VectorcraftApp| {
                                if !ready {
                                    egui_extras::install_image_loaders(ui.ctx());
                                    theme::install_fonts(ui.ctx());
                                    theme::apply(ui.ctx(), brightness);
                                    ready = true;
                                    return;
                                }
                                app_bar(app, ui);
                                status_bar(app, ui);
                            },
                            app,
                        );
                    harness.input_mut().max_texture_side = Some(8192);
                    harness.run_steps(4);
                    let stem = format!("vectorcraft-chrome-{brightness:?}-{width}-{scale}x");
                    harness.render().unwrap().save(directory.join(format!("{stem}.png"))).unwrap();
                    let mut regions = serde_json::Map::new();
                    for id in ["app_bar", "status_bar"] {
                        let rect = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::unique(id)).unwrap().outer_rect;
                        regions.insert(id.into(), json!([rect.left(), rect.top(), rect.width(), rect.height()]));
                    }
                    let dynamic: Vec<_> = harness
                        .output()
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text.contains(" fps") => {
                                let r = text.visual_bounding_rect();
                                Some(json!([r.left(), r.top(), r.width(), r.height()]))
                            }
                            _ => None,
                        })
                        .collect();
                    std::fs::write(
                        directory.join(format!("{stem}.json")),
                        serde_json::to_vec_pretty(&json!({
                            "screen": [width, 200.0], "scale": scale, "regions": regions, "native_dynamic_text_regions": dynamic
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn native_title_does_not_overlap_header_controls() {
        for brightness in theme::Brightness::ALL {
            for width in [800.0, 1280.0] {
                for scale in [1.0, 1.5, 2.0] {
                    let ctx = egui::Context::default();
                    egui_extras::install_image_loaders(&ctx);
                    theme::install_fonts(&ctx);
                    theme::apply(&ctx, brightness);
                    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
                    app.services.native_menu = Some(crate::native_menu::NativeMenu::new(Box::new(NativeMenus)));
                    let mut output = ctx.run_ui(scaled_input(vec2(width, 640.0), scale), |ui| app_bar(&mut app, ui));
                    output.textures_delta.clear();
                    assert_eq!(ctx.content_rect().size(), vec2(width, 640.0));
                    assert_eq!(ctx.pixels_per_point(), scale);
                    let texts: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some(text),
                            _ => None,
                        })
                        .collect();
                    let title = texts.iter().find(|text| text.galley.job.text == "VectorCraft");
                    if width >= 1280.0 {
                        assert!(title.is_some(), "the title remains visible when the header has room");
                    }
                    if let Some(title) = title {
                        for text in texts.iter().filter(|text| text.galley.job.text != "VectorCraft") {
                            assert!(
                                !title.visual_bounding_rect().intersects(text.visual_bounding_rect()),
                                "{width}: title overlaps {:?}",
                                text.galley.job.text
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn long_status_stays_between_navigation_and_performance_readouts() {
        for brightness in theme::Brightness::ALL {
            for width in [800.0, 1280.0] {
                for scale in [1.0, 1.5, 2.0] {
                    let ctx = egui::Context::default();
                    egui_extras::install_image_loaders(&ctx);
                    theme::install_fonts(&ctx);
                    theme::apply(&ctx, brightness);
                    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
                    app.ui.status = format!("Saved /Users/example/Documents/{}/illustration.svg", "long-folder-name/".repeat(10));
                    let mut output = ctx.run_ui(scaled_input(vec2(width, 640.0), scale), |ui| status_bar(&mut app, ui));
                    output.textures_delta.clear();
                    assert_eq!(ctx.content_rect().size(), vec2(width, 640.0));
                    assert_eq!(ctx.pixels_per_point(), scale);
                    let texts: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some(text),
                            _ => None,
                        })
                        .collect();
                    let status = texts.iter().find(|text| text.galley.job.text.starts_with("Saved ")).expect("status stays visible");
                    assert_eq!(status.galley.rows.len(), 1, "status is one line");
                    assert!(status.galley.elided, "long status retains the full text for the label tooltip");
                    let bounds = status.visual_bounding_rect();
                    assert!(bounds.left() >= 0.0 && bounds.right() <= width, "{width}: {bounds:?}");
                    for text in texts.iter().filter(|text| !text.galley.job.text.starts_with("Saved ")) {
                        assert!(!bounds.intersects(text.visual_bounding_rect()), "{width}: status overlaps {:?}", text.galley.job.text);
                    }
                }
            }
        }
    }
}
