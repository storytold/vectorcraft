use super::entries::{Row, entry, pretty_shortcut};
use super::invoke::{field_event, focused_text_field, invoke, to_field, with_held_keys};
use super::tree::menu_tree_named;
use super::{Entry, Item, Tokens, Value, VectorcraftApp, widgets};

/// Render the menu bar.
/// The in-window menu bar. Returns where its titles end (x): the bar itself takes the full width.
pub fn menu_bar(app: &mut VectorcraftApp, ui: &mut egui::Ui) -> f32 {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    // The press that opens a menu takes the keyboard from a focused text field: the field and its
    // selection are kept while the menus are open, so their Select All, Cut, Copy and Paste act on
    // its text (#516), the frame after the click that chose them (that click takes the keyboard
    // from it again).
    type Field = (egui::Id, egui::text_edit::TextEditState);
    let (field_key, chosen_key) = (egui::Id::unique("menu-bar-field"), egui::Id::unique("menu-bar-field-event"));
    if let Some(((field, state), e)) = ctx.data(|d| d.get_temp::<(Field, egui::Event)>(chosen_key)) {
        ctx.data_mut(|d| d.remove::<(Field, egui::Event)>(chosen_key));
        state.store(&ctx, field);
        to_field(&ctx, field, e);
    }
    if let Some(field) = focused_text_field(&ctx).and_then(|id| Some((id, egui::TextEdit::load_state(&ctx, id)?))) {
        ctx.data_mut(|d| d.insert_temp::<Field>(field_key, field));
    }
    let mut clicked: Option<(String, Value)> = None;
    let tree = menu_tree_named(app.session.prefs.font_names_in_english);
    // No application menu in the window (as in Illustrator): the bar starts at File on every
    // platform. The VectorCraft menu stays in the tree: the native Mac bar takes it from there.
    let tree: Vec<(&str, Vec<Item>)> = tree.into_iter().filter(|(title, _)| *title != "VectorCraft").collect();
    // A menu bar for its layout and menu-button style, but the titles use `Popup::menu` directly:
    // egui 0.37's `MenuButton` opens the title under a resting pointer while another menu is open,
    // and the app switches titles on movement only (`switch_on_hover`, the native behaviour).
    let (end, open) = egui::MenuBar::new()
        .ui(ui, |ui| {
            let mut titles = Vec::with_capacity(tree.len());
            for (title, items) in tree.iter() {
                let text = egui::RichText::new(tl!(title)).size(13.0).color(t.text);
                let resp = ui.add(egui::Button::new(text));
                egui::Popup::menu(&resp).show(|ui| menu_body(app, ui, items, &mut clicked));
                titles.push(resp);
            }
            (ui.cursor().min.x, switch_on_hover(ui.ctx(), &titles))
        })
        .inner;
    let field = ctx.data(|d| d.get_temp::<Field>(field_key));
    if !open && focused_text_field(&ctx).is_none() {
        ctx.data_mut(|d| d.remove::<Field>(field_key));
    }
    if let Some((id, p)) = clicked {
        match field.and_then(|f| Some((f, field_event(app, &id)?))) {
            Some(chosen) => {
                ctx.data_mut(|d| d.insert_temp(chosen_key, chosen));
                ctx.request_repaint();
            }
            None => invoke(app, &id, p),
        }
    }
    end
}

/// Whether the pointer at `p` is really over the menu title `title`: inside it, and with no
/// popup above it (a tall menu that egui moves up can cover the bar; hovering that menu must
/// not switch to the title under it).
pub(super) fn pointer_reaches_title(ctx: &egui::Context, title: &egui::Response, p: egui::Pos2) -> bool {
    title.interact_rect.contains(p) && ctx.layer_id_at(p) == Some(title.layer_id)
}

/// Like a native menu bar: while one top-level menu is open, moving the pointer onto another
/// title opens that menu instead (egui alone needs a click on each title). Returns whether a menu
/// is open.
fn switch_on_hover(ctx: &egui::Context, titles: &[egui::Response]) -> bool {
    let ids: Vec<egui::Id> = titles.iter().map(egui::Popup::default_response_id).collect();
    let Some(open) = ids.iter().position(|id| egui::Popup::is_id_open(ctx, *id)) else {
        return false;
    };
    // `Response::hovered` is false while a menu's popup is open, so hit-test the titles here.
    // Only a moving pointer switches: one resting on a title leaves the open menu alone.
    let moving = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    if let Some(p) = ctx.pointer_hover_pos().filter(|_| moving)
        && let Some(i) = titles.iter().position(|title| pointer_reaches_title(ctx, title, p))
        && i != open
        && let Some(id) = ids.get(i)
    {
        egui::Popup::open_id(ctx, *id);
        ctx.request_repaint();
    }
    true
}

/// A top-level menu's popup: as wide as its widest item (label plus shortcut), at least 230 pt;
/// it scrolls when it is taller than the window.
pub(super) fn menu_body(app: &VectorcraftApp, ui: &mut egui::Ui, items: &[Item], clicked: &mut Option<(String, Value)>) {
    widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(230.0);
        render_items(app, ui, items, true, clicked);
    });
}

/// `items` as menu buttons; with `checks`, items that can be on or off show a check mark (or the
/// room for one).
pub(super) fn render_items(app: &VectorcraftApp, ui: &mut egui::Ui, items: &[Item], checks: bool, clicked: &mut Option<(String, Value)>) {
    let t = Tokens::get(ui.ctx());
    for e in items.iter().filter_map(|it| entry(app, it)) {
        match e {
            Entry::Sep => {
                ui.separator();
            }
            Entry::Header(h) => {
                ui.label(egui::RichText::new(h).size(11.0).color(t.text_dim));
            }
            Entry::Sub(label, children) => {
                ui.menu_button(label, |ui| {
                    widgets::menu_scroll(ui, |ui| {
                        ui.set_min_width(200.0);
                        render_items(app, ui, children, checks, clicked);
                    });
                });
            }
            Entry::Item(row) => render_row(app, ui, &row, checks, clicked),
        }
    }
}

/// One menu item: its label (with a check mark, or the room for one, under `checks`), its shortcut,
/// and Type › Font's samples.
fn render_row(app: &VectorcraftApp, ui: &mut egui::Ui, row: &Row, checks: bool, clicked: &mut Option<(String, Value)>) {
    let sc = row.shortcut.map(pretty_shortcut).unwrap_or_default();
    let Some((id, p)) = row.command else {
        ui.add_enabled_ui(false, |ui| {
            ui.add(egui::Button::new(row.label.as_ref()).shortcut_text(sc));
        })
        .response
        .on_disabled_hover_text(tl!("Coming soon — tracked in the parity plan"));
        return;
    };
    let text = match row.checked.filter(|_| checks) {
        Some(true) => format!("✓  {}", row.label),
        Some(false) => format!("     {}", row.label),
        None => row.label.to_string(),
    };
    // Type → Font: each family's sample after its name (Enable in-menu font previews).
    let sampled = p.get("font").and_then(Value::as_str).filter(|_| id == "text.setStyle" && app.session.prefs.font_preview);
    let r = match sampled {
        Some(family) => {
            let slot = egui::IdSalt::new(("font-sample", family));
            let button = egui::Button::new(text).right_text(egui::Atom::custom(slot, crate::font_menu::MENU_SAMPLE_SIZE));
            let out = ui.add_enabled_ui(row.enabled, |ui| button.atom_ui(ui)).inner;
            if let Some(rect) = out.rect(slot) {
                crate::font_menu::menu_item_sample(ui, rect, family);
            }
            out.response
        }
        None => ui.add_enabled(row.enabled, egui::Button::new(text).shortcut_text(sc)),
    };
    let r = match (row.enabled, disabled_hint(app, id)) {
        (false, Some(why)) => r.on_disabled_hover_text(why),
        _ => r,
    };
    if r.clicked() {
        *clicked = row.target().map(|target| with_held_keys(target, ui.input(|i| i.modifiers)));
        ui.close();
    }
}

/// Why a dimmed menu item is dimmed, for the ones that say.
fn disabled_hint(app: &VectorcraftApp, id: &str) -> Option<&'static str> {
    match id {
        "ui.expandDialog" if vectorcraft_engine::cmd::expand::has_appearance(&app.session) => Some(crate::dialogs::expand::appearance_first()),
        _ => None,
    }
}

/// What the Home screen remembers when it opens (`app.home`): the active document and the
/// document count. When either changes, the Home screen gives way to the document.
pub(crate) fn home_key(app: &VectorcraftApp) -> (Option<u64>, usize) {
    (app.session.active().map(|d| d.uid), app.session.documents().len())
}

/// Whether the Home screen is up: chosen with the Home button (`app.home`), or no document is open
/// and Preferences › General › Show The Home Screen When No Documents Are Open is on (#394). Off,
/// an app with no document shows an empty window, and the Home button still opens the screen.
pub(crate) fn home_showing(app: &VectorcraftApp) -> bool {
    app.ui.home.is_some() || (app.session.active().is_none() && app.session.prefs.show_home_screen)
}
