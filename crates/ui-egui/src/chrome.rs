//! Window chrome: application bar (menus), Control bar, document tabs, status bar.

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use craft_ui::bars::BarHome;
use craft_ui::buttons::{Button, ButtonChord, IconButton, PopupMark};
use craft_ui::contract::{Bind, BodyCx, Props, Show, ShowContainer, Trigger};
use craft_ui::display::{Divider, Picture};
use craft_ui::layout::measure_slot;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::{HAlign, IconSpec};
use craft_ui::runtime::windows::WindowHost;
use craft_ui::shell::{Frame, Title, TitleBar};
use craft_ui::style::Variant;
use craft::behavior::{FitItem, FitKeep, IconId, fit_items};
use egui::{Sense, Stroke, Ui, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;

use crate::panels::stroke as stroke_panel;
use crate::state::{ROTATE_VIEW_ANGLES, ZOOM_STOPS, zoom_label};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{VectorcraftApp, brand, icons, menus};

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
pub(crate) use hints::tool_hint;
pub use hints::{hint_bar, hint_strings};
pub use status_bar::{StatusBarShow, deserialize_status_bar_show, set_status_bar_show, status_bar};
#[cfg(test)]
use status_bar::{StatusReadoutWidths, color_profile_name, date_time_label, status_readouts};
#[cfg(test)]
use tabs::{CLOSE_BEFORE_TITLE, first_tab_shown, leading_tab, shortened_title, tab_drop, tab_groups, tab_room, tab_title};
pub use tabs::{doc_tabs, isolation_bar, step_document};
pub(crate) use tabs::{document_window_title, show_document, window_tab};

/// The application bar's height, in points (compact).
pub const APP_BAR_HEIGHT: f32 = 32.0;

/// The command the search button runs and shows the chord of.
const PALETTE: &str = "help.commandPalette";

/// A control of the bar's trailing group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trailing {
    /// The Safe Mode label, shown only in Safe Mode.
    Safe,
    /// The community button.
    Discord,
    /// Explore tutorials.
    Learn,
    /// Arrange Documents.
    Arrange,
    /// The search that opens the command palette.
    Search,
    /// The workspace switcher.
    Switcher,
}

impl Trailing {
    /// The controls in the order they are drawn.
    const SHOWN: [Trailing; 6] = [Trailing::Discord, Trailing::Learn, Trailing::Search, Trailing::Arrange, Trailing::Safe, Trailing::Switcher];
    /// The controls in the order the fit gives them up, last first: each is also in a menu, so a hidden one goes nowhere.
    const DROPPED_LAST_FIRST: [Trailing; 6] =
        [Trailing::Safe, Trailing::Switcher, Trailing::Search, Trailing::Arrange, Trailing::Learn, Trailing::Discord];

    /// Whether the fit may drop it.
    fn keep(self) -> FitKeep {
        if matches!(self, Trailing::Safe | Trailing::Switcher) { FitKeep::Pinned } else { FitKeep::Free }
    }

    /// Shows the control; the fit measures it through the same call.
    fn show(self, app: &mut VectorcraftApp, ui: &mut Ui) {
        match self {
            Trailing::Safe => {
                let shown = Button::new(tl!("Safe Mode")).variant(Variant::GHOST).tooltip(tl!("Safe Mode…")).show(ui);
                if shown.clicks > 0 {
                    app.run("help.safeMode", json!({})).ok();
                }
            }
            Trailing::Discord => crate::community::discord_button(app, ui),
            Trailing::Learn => {
                // PLAN: assumed `IconButton::command` is bound by a `&str` command id.
                IconButton::command(IconId::new("graduation-cap"), "window.learn").selected(crate::panels::learn::visible(&app.ui)).show(ui);
            }
            Trailing::Arrange => arrange_documents(app, ui),
            Trailing::Search => {
                // PLAN: assumed `Button::command` is bound by a `&str` command id; the command opens the palette.
                Button::command(PALETTE)
                    .icon(IconSpec::svg(IconId::new("search")))
                    .variant(Variant::SECONDARY)
                    .align(HAlign::Start)
                    .chord(ButtonChord::Trailing)
                    .show(ui);
            }
            // PLAN: assumed `crate::workspaces::switcher(app: &mut VectorcraftApp, ui: &mut Ui)` (chrome-w14's `WorkspaceSwitcher`).
            Trailing::Switcher => crate::workspaces::switcher(app, ui),
        }
    }
}

/// Arrange Documents: Consolidate All Windows and the Tile layouts, each item as the Window menu shows it.
fn arrange_documents(app: &mut VectorcraftApp, ui: &mut Ui) {
    let opener = IconButton::new(IconId::new("layout_grid")).label(tl!("Arrange Documents")).selected(app.ui.tiled).popup_mark(PopupMark::Menu).show(ui);
    let mut chosen = None;
    let state = &*app;
    // PLAN: the sheet names `Trigger::Manual`; `Trigger::Press` is the pop-up menu button's trigger and needs no open binding.
    Menu::new(tl!("Arrange Documents")).opened_by(&opener.response, Trigger::Press).show(ui, |ui, _| {
        for item in menus::arrange_documents_items() {
            match item {
                menus::Item::Sep => {
                    Divider::horizontal().show(ui);
                }
                menus::Item::Cmd(label, id, params) => {
                    let Some((enabled, checked)) = menus::shown_state(state, id, &params) else { continue };
                    let row = MenuItem::new(crate::i18n::t_id(id, label)).enabled(enabled).check(checked).show(ui);
                    if row.ran {
                        chosen = Some((id, if params.is_null() { json!({}) } else { params }));
                    }
                }
                _ => {}
            }
        }
    });
    if let Some((id, params)) = chosen {
        menus::invoke(app, id, params);
    }
}

/// The start of the bar: the brand mark (the OS draws its own icon in a native frame), Home and the menus.
fn leading(ui: &mut Ui, app: &mut VectorcraftApp) {
    if !matches!(WindowHost::of(ui.ctx()).frame(), Frame::Native) {
        let image = || (*brand::image()).clone();
        Picture::new(tl!("VectorCraft"), brand::BRAND_KEY, &image).show(ui);
    }
    let mut home = menus::home_showing(app);
    let before = home;
    BarHome::new(tl!("Home"), Bind::bound(&mut home)).show(ui);
    if home != before {
        app.run("app.home", json!({})).ok();
    }
    // With the macOS menu bar the menus are at the top of the screen instead.
    if app.services.native_menu.is_none() {
        // PLAN: assumed menus-w3's `menus::menu_bar(app, ui)` shows `MenuBarUi` and returns its `MenuBarOutput`; `opened` refreshes the scripts menu (menus-w6).
        let _ = menus::menu_bar(app, ui);
    }
}

/// The end of the bar: the controls that fit in the room, the first-listed going last.
fn trailing(ui: &mut Ui, app: &mut VectorcraftApp) {
    let safe = vectorcraft_plugins::safe_mode::active();
    let order: Vec<Trailing> = Trailing::DROPPED_LAST_FIRST.into_iter().filter(|t| safe || *t != Trailing::Safe).collect();
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width();
    let items: Vec<FitItem> = order.iter().map(|t| FitItem::new(measure_slot(ui, room, |ui| t.show(app, ui)).x, 0, t.keep())).collect();
    let fit = fit_items(&items, room, gap, gap, 0.0);
    ui.horizontal_centered(|ui| {
        for t in Trailing::SHOWN {
            if order.iter().position(|o| *o == t).is_some_and(|at| fit.shown.contains(&at)) {
                t.show(app, ui);
            }
        }
    });
}

/// The application bar, the body of the shell's title bar: brand mark, Home and menus, then Discord,
/// tutorials, Arrange Documents, search and the workspace switcher at the end. The title bar draws
/// the caption buttons, drags the window and shows the title.
pub fn app_bar(ui: &mut Ui, _cx: &mut BodyCx<'_>, app: &mut VectorcraftApp) {
    let app = RefCell::new(app);
    // Both closures need the app; the bar runs them one after the other.
    let _ = TitleBar::new().show_titled(
        ui,
        Title::new(tl!("VectorCraft")),
        |ui, _| leading(ui, &mut **app.borrow_mut()),
        |ui, _| trailing(ui, &mut **app.borrow_mut()),
    );
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

#[cfg(test)]
mod tests;
