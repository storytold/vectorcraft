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

#[cfg(test)]
use control_bar::ISOLATION_BUTTON;
pub use control_bar::{AnchorControls, anchor_buttons, anchor_controls, control_bar};
#[cfg(test)]
use hints::hint_segments;
pub use hints::{hint_bar, hint_strings};
pub use status_bar::status_bar;
#[cfg(test)]
use status_bar::{StatusReadoutWidths, status_readouts};
#[cfg(test)]
use tabs::{CLOSE_BEFORE_TITLE, first_tab_shown, tab_title};
pub use tabs::{doc_tabs, isolation_bar, step_document};

/// The application bar's height, in points (compact, as in PhotoCraft).
pub const APP_BAR_HEIGHT: f32 = 32.0;
/// The brand mark's side.
const MARK: f32 = 18.0;
/// Height of the workspace switcher.
const WIDGET_H: f32 = 22.0;

/// The application bar: brand mark, Home, menus, then Discord, search and the workspace switcher
/// at the right. With [`VectorcraftApp::custom_titlebar`] it is also the window's title bar
/// ([`titlebar`]): the caption buttons take the right end and the rest of the bar drags the window.
/// On macOS, under the system's transparent title bar, its empty space drags and zooms the window
/// too ([`titlebar::drag_area`]). With the system title bar (Windows and Linux, `system_title_bar`) the OS draws its own icon and
/// title, so the in-app bar shows Home, menus and workspace controls only: no brand mark.
pub fn app_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let custom = app.custom_titlebar;
    let system = system_title_bar(app);
    let left = app.titlebar_inset.clamp(8.0, f32::from(i8::MAX)) as i8;
    let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin { left, right: if custom { 0 } else { 14 }, top: 0, bottom: 0 });
    let bar = egui::Panel::top("app_bar").exact_size(APP_BAR_HEIGHT).frame(frame.stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        // macOS draws the bar under its transparent title bar, so the bar gets the title bar's
        // clicks: without this a double-click there zooms nothing (#1932).
        if custom || cfg!(target_os = "macos") {
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
            let menus_end = if app.services.native_menu.is_some() { ui.cursor().min.x } else { menus::menu_bar(app, ui) };
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
