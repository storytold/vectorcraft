//! What the app does with the commands the key stage fires: tool keys and their holds, the Escape chain,
//! nudges and the window's panel and artboard shortcuts.

use craft::keymap::{CommandRef, Mods};
use craft::ui::runtime::commands::{FirePhase, FireSource, Fired};
use craft::ui::runtime::keys::{self, Intent};
use craft::ui::runtime::windows::WindowHost;
use serde_json::json;
use vectorcraft_tools::ToolKey;

use crate::VectorcraftApp;

mod held_tool;
mod presentation;

pub(crate) use held_tool::{HeldTool, note_tool_use};
use presentation::escape_resets_rotation;
pub(crate) use presentation::presentation_step;

/// Runs one command the key stage fired. A tool key chooses its tool (and starts or ends a hold), `Select All`
/// with the Artboard tool selects artboards, and every other command runs as its menu item does.
pub(crate) fn run_fired(app: &mut VectorcraftApp, ctx: &egui::Context, fired: &Fired) {
    // A menu's Select All (the macOS menu bar's, or the window's menu that gave the keys back) selects the text of the
    // field that has the keys, as Cmd+A does: it goes to the field as the framework's edit command. The Type tool's
    // text is selected by the app's own Select All.
    if fired.command.id == "select.all" && fired.source == FireSource::Menu && WindowHost::of(ctx).typing() && !app.session.tool_wants_text() {
        craft::ui::runtime::commands::fire(ctx, CommandRef::new("edit.select_all"), FireSource::Menu);
        return;
    }
    let renamed;
    let command = match engine_id(&fired.command.id) {
        Some(id) => {
            renamed = CommandRef::with_params(id, fired.command.params.clone());
            &renamed
        }
        None => &fired.command,
    };
    // A palette pick of a dimmed result runs nothing and says why; one that ran, opened its dialog
    // or chose its tool is counted.
    let picked = fired.source == FireSource::Palette;
    if picked && let Some(why) = crate::palette::refusal(app, command) {
        app.status(why);
        return;
    }
    // While a tool's gesture runs, keys run no command; the active tool's own shortcut, between clicks, chooses it
    // again, which ends the gesture.
    if fired.source == FireSource::Key && fired.phase != FirePhase::Release && crate::canvas::busy_tool(app) {
        let active = app.session.tool_id().to_owned();
        if command.id.strip_prefix("tool:") == Some(active.as_str()) && !app.session.pointer_held() {
            app.select_tool(&active);
        }
        return;
    }
    if let Some(tool) = command.id.strip_prefix("tool:") {
        match fired.phase {
            FirePhase::Release => held_tool::release(app, ctx, fired),
            FirePhase::Press | FirePhase::Repeat => held_tool::choose(app, ctx, fired),
        }
        if picked && app.session.tool_id() == tool {
            crate::palette::record_pick(app, command);
        }
        return;
    }
    // The end of a momentary key is the board's; no command of the app acts on it.
    if fired.phase == FirePhase::Release {
        return;
    }
    // A panel's pick shows the panel, where `window.panel` would put a showing one away.
    if picked
        && command.id == "window.panel"
        && let Some(panel) = command.params.get("panel").and_then(serde_json::Value::as_str)
    {
        match crate::panel_docking::reveal_panel(app, panel) {
            Ok(_) => crate::palette::record_pick(app, command),
            Err(e) => app.status(e),
        }
        return;
    }
    if command.id == "select.all" && crate::panels::artboards::selects_all(app) {
        // With the Artboard tool, Select All's keys after a click on an Artboards panel row select every artboard
        // instead of the art.
        crate::panels::artboards::select_all(app);
        return;
    }
    match menu_target(command).or_else(|| clicked_row(app, fired.source, command)) {
        Some((id, params)) => crate::menus::invoke(app, &id, params),
        None => crate::menus::invoke(app, &command.id, shortcut_params(command)),
    }
    if picked {
        crate::palette::record_pick(app, command);
    }
}

/// A key bound only to commands that can't run now (`FrameOutput::dimmed`) does what production's key did, its menu
/// item's action, which says why it can't run or does nothing; nothing while a dialog or a text field has the keys or the
/// gate holds the commands (a file dialog, a shortcut being recorded, Presentation Mode).
pub(crate) fn run_dimmed(app: &mut VectorcraftApp, ctx: &egui::Context, fired: &Fired) {
    if !app.any_dialog_open() && !ctx.egui_wants_keyboard_input() && crate::menus::model::gate_lets(app, &fired.command) {
        run_fired(app, ctx, fired);
    }
}

/// The engine's id of a framework edit command that no focused part took: Select All and Delete. Cut, Copy, Paste,
/// Undo and Redo have the same id in both.
pub(crate) fn engine_id(id: &str) -> Option<&'static str> {
    match id {
        "edit.select_all" => Some("select.all"),
        "edit.delete" => Some("edit.clear"),
        _ => None,
    }
}

/// Once per pass: the returns of held tools that a gesture or a dialog deferred.
pub(crate) fn follow_holds(app: &mut VectorcraftApp, ctx: &egui::Context) {
    held_tool::follow(app, ctx);
}

/// On a Mac, Cmd+, opens Settings as in every Mac app (#663), beside Preferences' own shortcut, unless a shortcut
/// has the chord.
pub(crate) fn settings_on_a_mac(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if ctx.os() != egui::os::OperatingSystem::Mac {
        return;
    }
    let pressed = ctx.input(|i| {
        i.events.iter().any(|e| {
            matches!(e, egui::Event::Key { key: egui::Key::Comma, pressed: true, repeat: false, modifiers, .. } if modifiers.matches_exact(egui::Modifiers::COMMAND))
        })
    });
    let chord = craft::keymap::Chord::new(Mods::CMD, craft::keymap::Key::Comma);
    let bound = || {
        craft::ui::runtime::commands::keymap(ctx)
            .is_some_and(|keymap| !matches!(keymap.lookup(&chord, false, &craft::keymap::Active::NONE), craft::keymap::Lookup::None))
    };
    if pressed
        && !bound()
        && let Err(e) = app.run("edit.preferences", json!({}))
    {
        app.status(e);
    }
}

/// An Escape the window took (nothing focused, no popup or session took it) leaves Presentation Mode, or else ends
/// the first thing on: pattern editing, isolation, Global Edit, a rotated view or a full screen mode.
pub(crate) fn escape(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // Cancel reads the same in either reading direction.
    if !keys::with_window_intents(ctx, false, |window| window.take(Intent::Cancel)) {
        return;
    }
    if app.ui.screen_mode == 3 {
        let _ = app.run("view.presentation", json!({}));
        return;
    }
    // An Escape the tool claims (the Type tool ends its edit and goes back to the tool used before) is only the tool's.
    let view = app.view_info();
    if app.session.tool_claims_key(ToolKey::Escape, view) {
        let r = app.session.tool_key(ToolKey::Escape, vectorcraft_tools::Mods::default(), view);
        crate::canvas::apply_requests(app, r);
        return;
    }
    escape_ends_one_thing(app, ctx);
}

/// In a floating panel's OS window, the keys none of its controls took, which production's palette windows passed
/// on to the main window: Escape ends one thing as there ([`escape`]), and Enter is the canvas's tool's (in
/// Presentation Mode it opens nothing).
pub(crate) fn palette_window_keys(app: &mut VectorcraftApp, ctx: &egui::Context) {
    escape(app, ctx);
    if keys::with_window_intents(ctx, false, |window| window.take(Intent::Submit)) && app.ui.screen_mode != 3 {
        let view = app.view_info();
        crate::canvas::tool_enter(app, vectorcraft_tools::Mods::default(), view);
    }
}

/// An Escape that nothing else took ends the first of these that is on.
fn escape_ends_one_thing(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.session.active().is_some_and(|d| d.doc.pattern_edit.is_some()) {
        let _ = app.run("object.pattern.done", json!({}));
    } else if app.session.active().is_some_and(|d| d.isolation.is_some()) {
        // From an isolated sublayer, Escape leaves isolation mode rather than going up one layer.
        let _ = app.run("object.exitIsolation", json!({ "all": true }));
    } else if app.session.active().is_some_and(|d| d.global_edit.is_some()) {
        let _ = app.run("select.globalEdit", json!({}));
    } else if app.session.tool_id() == "rotateView" && escape_resets_rotation(app, ctx) {
        // With the Rotate View tool, the view resets before a full screen mode ends.
        let _ = app.run("view.rotateReset", json!({}));
    } else if matches!(app.ui.screen_mode, 1 | 2) && keys::escape_mods(ctx).is_some_and(|mods| mods.is_none()) {
        let _ = app.run("view.screenMode", json!({ "mode": 0 }));
    } else if escape_resets_rotation(app, ctx) {
        // An Escape that has nothing else to end resets a rotated view. A modified Escape does not leave a full
        // screen mode, so it resets the view there.
        let _ = app.run("view.rotateReset", json!({}));
    }
}

/// An arrow nudge of the window or of a tool that left it. `direction` points the way of the arrow. In
/// Presentation Mode Left and Right step the artboard and nothing moves; with only type selected, Alt steps the
/// type's spacing; otherwise the selection moves (Shift ×10, Alt copies, `patterns_only` moves pattern fills only).
pub(crate) fn nudge(app: &mut VectorcraftApp, direction: egui::Vec2, mods: Mods, patterns_only: bool) {
    if app.ui.screen_mode == 3 {
        if direction.x != 0.0 {
            presentation_step(app, direction.x > 0.0);
        }
        return;
    }
    if mods.alt
        && let Some(vertical) = selected_type_is_vertical(app)
    {
        let key = match (direction.x, direction.y) {
            (x, _) if x < 0.0 => ToolKey::Left,
            (x, _) if x > 0.0 => ToolKey::Right,
            (_, y) if y < 0.0 => ToolKey::Up,
            _ => ToolKey::Down,
        };
        if let Some((attribute, by)) = vectorcraft_tools::text::step_key(key, mods.shift, false, vertical) {
            let by = if mods.cmd { by * 5.0 } else { by };
            let _ = app.run("type.step", json!({"attribute": attribute, "by": by}));
        }
        return;
    }
    if app.session.active().is_some_and(|d| d.selection.has_objects_or_guides()) {
        let mut params = json!({"dx": f64::from(direction.x), "dy": f64::from(direction.y), "big": mods.shift, "copy": mods.alt});
        if patterns_only {
            params["objects"] = json!(false);
            params["patterns"] = json!(true);
        }
        let _ = app.run("object.nudge", params);
    }
}

/// Whether the selection is only type, and then whether all of it is vertical.
fn selected_type_is_vertical(app: &VectorcraftApp) -> Option<bool> {
    let st = app.session.active()?;
    if !st.selection.guides.is_empty() || st.selection.objects.is_empty() {
        return None;
    }
    let mut all_vertical = true;
    for id in &st.selection.objects {
        let vectorcraft_doc::NodeKind::Text(t) = &st.doc.node(*id)?.kind else { return None };
        all_vertical &= t.vertical;
    }
    Some(all_vertical)
}

/// What a shortcut without params runs when its command's menu bar item gives params
/// ([`crate::shortcut_editor::menu_target`]): what a click on that item runs.
fn menu_target(c: &CommandRef) -> Option<(String, serde_json::Value)> {
    c.params.as_object().is_none_or(serde_json::Map::is_empty).then(|| crate::shortcut_editor::menu_target(&c.id)).flatten()
}

/// What a click on the menu bar's or the canvas menu's row `c` runs ([`crate::menus::click_target`]): a "…" row with
/// params opens its dialog on them (Clean Up…). `None` for a command that no menu fired.
fn clicked_row(app: &VectorcraftApp, source: FireSource, c: &CommandRef) -> Option<(String, serde_json::Value)> {
    if source != FireSource::Menu || c.params.as_object().is_none_or(serde_json::Map::is_empty) {
        return None;
    }
    let row = crate::menus::menu_entries(app)
        .into_iter()
        .chain(crate::menus::context_entries(app))
        .find(|e| e.command.as_deref() == Some(c.id.as_str()) && e.params == c.params)?;
    Some(crate::menus::click_target(&row.label, &c.id, &c.params))
}

/// Params used when a shortcut invokes its command. The named panel function keys retain their
/// toggle action when their bindings are reassigned.
fn shortcut_params(c: &CommandRef) -> serde_json::Value {
    let mut params = if c.params.is_null() { json!({}) } else { c.params.clone() };
    if c.id == "window.panel"
        && matches!(
            params.get("panel").and_then(serde_json::Value::as_str),
            Some(
                "brushes"
                    | "color"
                    | "layers"
                    | "info"
                    | "gradient"
                    | "stroke"
                    | "attributes"
                    | "graphicStyles"
                    | "appearance"
                    | "align"
                    | "transform"
                    | "pathfinder"
                    | "transparency"
                    | "symbols"
            )
        )
    {
        params["toggle"] = json!(true);
        params["toggleGroup"] = json!(true);
    }
    params
}

#[cfg(test)]
mod tests;
