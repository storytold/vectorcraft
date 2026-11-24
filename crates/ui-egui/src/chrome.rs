//! Window chrome: application bar (menus), Control bar, document tabs, status bar.

use std::borrow::Cow;
use std::cell::RefCell;

use craft::behavior::IconId;
use craft::behavior::fit::{FitItem, FitKeep, fit_items};
use craft::menu::MenuBarUi;
use craft::ui::bars::BarHome;
use craft::ui::buttons::{Button, ButtonChord, IconButton, PopupMark};
use craft::ui::contract::{Bind, BodyCx, Props, Show, ShowContainer, Trigger};
use craft::ui::display::{Divider, Picture};
use craft::ui::layout::measure_slot;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::{HAlign, IconSpec};
use craft::ui::runtime::windows::WindowHost;
use craft::ui::shell::{Frame, Title, TitleBar};
use craft::ui::style::Variant;
use egui::Ui;
use serde_json::json;

use crate::menus::Node;
use crate::state::{ROTATE_VIEW_ANGLES, zoom_label};
use crate::{VectorcraftApp, brand, menus};

mod control_bar;
mod hints;
mod status_bar;
mod tabs;

pub use control_bar::{
    AnchorControls, CONTROL_BAR_EDGES, CONTROL_BAR_GROUPS, anchor_buttons, anchor_controls, control_bar, control_bar_placement, read_hidden_groups,
};
pub(crate) use control_bar::{control_bar_place, rank, set_control_bar_place};
#[cfg(test)]
use hints::hint_segments;
pub(crate) use hints::tool_hint;
pub use hints::{HINT_SLOT, hint_bar, hint_strings};
pub use status_bar::{StatusBarShow, deserialize_status_bar_show, set_status_bar_show, status_bar};
#[cfg(test)]
use status_bar::{color_profile_name, date_time_label};
#[cfg(test)]
use tabs::tab_title;
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

    /// Shows the control, in its shorter form when `short` (the search as its icon); the fit measures it through the
    /// same call.
    fn show(self, app: &mut VectorcraftApp, ui: &mut Ui, short: bool) {
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
                IconButton::command(IconId::new("graduation-cap"), "window.learn")
                    .label(tl!("Explore tutorials"))
                    .selected(crate::panels::learn::visible(app))
                    .show(ui);
            }
            Trailing::Arrange => arrange_documents(app, ui),
            Trailing::Search if short => {
                IconButton::command(IconId::new("search"), PALETTE).label(tl!("Search commands")).show(ui);
            }
            Trailing::Search => {
                // PLAN: assumed `Button::command` is bound by a `&str` command id; the command opens the palette.
                // Named as production's field, not after the command's menu row.
                Button::command(PALETTE)
                    .label(tl!("Search commands"))
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
    let opener =
        IconButton::new(IconId::new("layout_grid")).label(tl!("Arrange Documents")).selected(app.ui.tiled).popup_mark(PopupMark::Menu).show(ui);
    let mut chosen = None;
    let state = &*app;
    // PLAN: the sheet names `Trigger::Manual`; `Trigger::Press` is the pop-up menu button's trigger and needs no open binding.
    Menu::new(tl!("Arrange Documents")).opened_by(&opener.response, Trigger::Press).show(ui, |ui, _| {
        for item in menus::arrange_documents_items() {
            match item {
                Node::Separator => {
                    Divider::horizontal().show(ui);
                }
                Node::Item(item) => {
                    let (id, params) = (item.command.id, item.command.params);
                    let Some((enabled, checked)) = menus::shown_state(state, &id, &params) else { continue };
                    let mut row = MenuItem::new(menus::shown_label(state, &id, &params, &item.label)).enabled(enabled);
                    if let Some(checked) = checked {
                        row = row.check(checked);
                    }
                    let row = row.show(ui);
                    if row.ran {
                        chosen = Some((id, if params.is_null() { json!({}) } else { params }));
                    }
                }
                _ => {}
            }
        }
    });
    if let Some((id, params)) = chosen {
        menus::invoke(app, &id, params);
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
    if app.services.native_menu.is_none() && MenuBarUi::new(&app.menus.laid).show(ui).opened {
        // A menu opened: File › Scripts lists the Scripts folder as it is now.
        crate::scripts_menu::refresh(app, ui.ctx());
    }
}

/// The end of the bar: the controls that fit in the room, the first-listed going last.
fn trailing(ui: &mut Ui, app: &mut VectorcraftApp) {
    let safe = vectorcraft_plugins::safe_mode::active();
    let order: Vec<Trailing> = Trailing::DROPPED_LAST_FIRST.into_iter().filter(|t| safe || *t != Trailing::Safe).collect();
    let gap = ui.spacing().item_spacing.x;
    let room = ui.available_width();
    let mut measure = |t: Trailing, short: bool| measure_slot(ui, room, |ui| t.show(app, ui, short)).x;
    let items: Vec<FitItem> = order
        .iter()
        .map(|&t| {
            let item = FitItem::new(measure(t, false), 0, t.keep());
            // The search narrows to its icon before it goes.
            if t == Trailing::Search { item.short(measure(t, true)) } else { item }
        })
        .collect();
    let fit = fit_items(&items, room, gap, gap, 0.0);
    ui.horizontal_centered(|ui| {
        for t in Trailing::SHOWN {
            if let Some(at) = order.iter().position(|o| *o == t).filter(|at| fit.shown.contains(at)) {
                t.show(app, ui, fit.short.contains(&at));
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
        |ui, _| leading(ui, &mut app.borrow_mut()),
        |ui, _| trailing(ui, &mut app.borrow_mut()),
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
