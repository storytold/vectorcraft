//! Keyboard shortcuts: command shortcuts from the registry, single-key tool shortcuts, arrows,
//! and tool keys (Enter/Esc/↑/↓ while drawing).

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::json;
use vectorcraft_tools::{Mods, ToolKey};

use crate::VectorcraftApp;

/// Parse "Cmd+Shift+]" into an egui shortcut. `Cmd` is Command on macOS and Ctrl elsewhere.
pub fn parse(s: &str) -> Option<KeyboardShortcut> {
    if s.is_empty() {
        return None;
    }
    let (mods_part, key_part) =
        if let Some(stripped) = s.strip_suffix("++") { (stripped.trim_end_matches('+'), "+") } else { s.rsplit_once('+').unwrap_or(("", s)) };
    let mut m = Modifiers::NONE;
    for part in mods_part.split('+').filter(|p| !p.is_empty()) {
        match part {
            "Cmd" => m |= Modifiers::COMMAND,
            "Shift" => m |= Modifiers::SHIFT,
            "Alt" => m |= Modifiers::ALT,
            "Ctrl" => m |= Modifiers::CTRL,
            _ => return None,
        }
    }
    let key = match key_part {
        "]" => Key::CloseBracket,
        "[" => Key::OpenBracket,
        ";" => Key::Semicolon,
        "'" => Key::Quote,
        "/" => Key::Slash,
        "\\" => Key::Backslash,
        "=" => Key::Equals,
        "-" => Key::Minus,
        "+" => Key::Plus,
        "Delete" => Key::Delete,
        "Backspace" => Key::Backspace,
        "Tab" => Key::Tab,
        "~" => Key::Backtick,
        k => Key::from_name(k)?,
    };
    Some(KeyboardShortcut::new(m, key))
}

/// Every command shortcut in effect (user overrides from Edit → Keyboard Shortcuts win), with the
/// command's params: `{}`, or `{panel}` for the panel shortcuts (`window.panel`).
pub(crate) fn all_shortcuts() -> Vec<(KeyboardShortcut, &'static str, serde_json::Value)> {
    let mut v = vec![];
    for c in vectorcraft_engine::command_specs() {
        if let Some(sc) = crate::menus::shortcut_of(c.id).and_then(parse) {
            v.push((sc, c.id, json!({})));
        }
    }
    for c in crate::menus::UI_COMMANDS {
        if let Some(sc) = crate::menus::shortcut_of(c.0).and_then(parse) {
            v.push((sc, c.0, json!({})));
        }
    }
    for (panel, _, _) in crate::state::ICON_PANELS {
        if let Some(sc) = crate::shortcut_editor::panel_shortcut(panel).and_then(parse) {
            v.push((sc, "window.panel", json!({ "panel": panel })));
        }
    }
    // Most specific (most modifiers) first so Cmd+Shift+Z isn't eaten by Cmd+Z.
    v.sort_by_key(|(sc, ..)| {
        std::cmp::Reverse(sc.modifiers.shift as u8 + sc.modifiers.alt as u8 + sc.modifiers.command as u8 + sc.modifiers.ctrl as u8)
    });
    v
}

/// Keys that paste with Cmd, or alone. (Shift+Insert is left out: Ctrl+Insert copies.)
const PASTE_KEYS: [Key; 2] = [Key::V, Key::Paste];

/// Keyboard pastes of something other than text. egui swallows a paste chord's key press and
/// sends a Paste event only when the clipboard holds text, so a paste key released without its
/// press (or a Paste event) reported was a paste of a bitmap or a PDF.
#[derive(Default)]
pub(crate) struct PasteChord {
    /// Paste keys whose press was reported (typed, not pasting).
    pressed: Vec<Key>,
    /// A Paste event came since the last paste key was released.
    pasted: bool,
}

impl PasteChord {
    /// Follow one frame's `events` → whether they hold a paste egui sent no event for.
    fn textless_paste(&mut self, events: &[egui::Event]) -> bool {
        let mut fire = false;
        for e in events {
            match e {
                egui::Event::Paste(_) => self.pasted = true,
                // A release that comes while the window is away is never seen.
                egui::Event::WindowFocused(false) => *self = Self::default(),
                egui::Event::Key { key, pressed: true, .. } if PASTE_KEYS.contains(key) && !self.pressed.contains(key) => self.pressed.push(*key),
                egui::Event::Key { key, pressed: false, .. } if PASTE_KEYS.contains(key) => match self.pressed.iter().position(|k| k == key) {
                    Some(i) => {
                        self.pressed.swap_remove(i);
                    }
                    None => fire |= !std::mem::take(&mut self.pasted),
                },
                _ => {}
            }
        }
        fire
    }
}

/// The paste command for a paste chord held with `m`. egui sends Cmd+V with any other modifiers
/// as the same paste event, so Paste in Place (Cmd+Shift+V), Paste on All Artboards and Paste
/// without Formatting are told apart here by the keys held, through their shortcuts.
fn paste_command(m: Modifiers) -> &'static str {
    all_shortcuts()
        .into_iter()
        .find(|(sc, id, _)| id.starts_with("edit.paste") && sc.logical_key == Key::V && m.matches_exact(sc.modifiers))
        .map_or("edit.paste", |(_, id, _)| id)
}

pub fn handle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // Followed before anything returns, so a key typed in a field isn't taken for a paste.
    let textless_paste = ctx.input(|i| app.paste_chord.textless_paste(&i.events));
    if app.ui.dialog.is_some() || app.ui.palette_open {
        if crate::shortcut_editor::is_recording(app) {
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            app.ui.dialog = None;
            app.ui.palette_open = false;
        }
        return;
    }
    let typing = ctx.egui_wants_keyboard_input();
    let view = app.view_info();
    // Tool keys first (Enter/Escape end paths; arrows change polygon sides while dragging).
    let busy = app.session.tool_busy();
    for (k, tk) in [(Key::Enter, ToolKey::Enter), (Key::Escape, ToolKey::Escape)] {
        if !typing && ctx.input(|i| i.key_pressed(k)) {
            // A key the tool claims (Esc with a loaded place cursor) is only the tool's.
            let claimed = app.session.tool_claims_key(tk, view);
            let r = app.session.tool_key(tk, Mods::default(), view);
            if claimed {
                crate::canvas::apply_requests(app, r);
            }
            if k == Key::Escape && !busy && !claimed {
                if app.session.active().is_some_and(|d| d.doc.pattern_edit.is_some()) {
                    let _ = app.run("object.pattern.done", json!({}));
                } else if app.session.active().is_some_and(|d| d.isolation.is_some()) {
                    let _ = app.run("object.exitIsolation", json!({}));
                } else if app.ui.flyout.is_some() {
                    app.ui.flyout = None;
                }
            }
        }
    }
    if typing {
        return;
    }
    // Type tool editing: text and editing keys go to the tool.
    if app.session.tool_wants_text() {
        let texts: Vec<String> =
            ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect());
        for t in texts {
            let _ = app.session.tool_text(&t, view);
        }
        // Editing keys with modifiers, clipboard and Cmd+A.
        crate::panels::character::route_type_input(app, ctx);
        // Enter was already delivered above as ToolKey::Enter (newline).
        let fire = all_shortcuts().into_iter().filter(|(sc, ..)| sc.modifiers.command).find(|(sc, ..)| ctx.input_mut(|i| i.consume_shortcut(sc)));
        if let Some((_, id, p)) = fire {
            crate::menus::invoke(app, id, p);
        }
        return;
    }
    // Clipboard keys arrive as events, not key presses (except where the native menu has them).
    let pasting = textless_paste || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
    let paste = if pasting { paste_command(ctx.input(|i| i.modifiers)) } else { "edit.paste" };
    let mut clip = vec![];
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let (id, text) = match e {
                egui::Event::Copy => ("edit.copy", None),
                egui::Event::Cut => ("edit.cut", None),
                egui::Event::Paste(t) => (paste, Some(t.clone())),
                _ => return true,
            };
            if app.native_shortcuts.contains(id) {
                return true;
            }
            clip.push((id, text));
            false
        })
    });
    // Only the system clipboard service reads what isn't text (the native menu has its own Paste).
    if textless_paste && app.services.system_clipboard.is_some() && !app.native_shortcuts.contains(paste) {
        clip.push((paste, None));
    }
    for (id, text) in clip {
        app.clipboard_in = text;
        crate::menus::invoke(app, id, json!({}));
    }
    if busy {
        for (k, tk) in [(Key::ArrowUp, ToolKey::Up), (Key::ArrowDown, ToolKey::Down)] {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k)) {
                let _ = app.session.tool_key(tk, Mods::default(), view);
            }
        }
        return;
    }
    // Keys the active tool claims ahead of their shortcuts (the Gradient tool's selected stop:
    // Delete/Backspace remove it, ←/→ nudge it; the loaded place cursor: the arrows cycle files).
    let m = ctx.input(|i| i.modifiers);
    for (k, tk) in [
        (Key::Delete, ToolKey::Delete),
        (Key::Backspace, ToolKey::Backspace),
        (Key::ArrowLeft, ToolKey::Left),
        (Key::ArrowRight, ToolKey::Right),
        (Key::ArrowUp, ToolKey::Up),
        (Key::ArrowDown, ToolKey::Down),
    ] {
        if ctx.input(|i| i.key_pressed(k)) && app.session.tool_claims_key(tk, view) && ctx.input_mut(|i| i.consume_key(m, k)) {
            let r = app.session.tool_key(tk, crate::canvas::mods(m, false), view);
            crate::canvas::apply_requests(app, r);
            return;
        }
    }
    // Command shortcuts.
    let mut fire = None;
    for (sc, id, p) in all_shortcuts() {
        // Letter / punctuation keys without Cmd/Alt/Ctrl are handled below as text (tool shortcuts,
        // X, D, /, `,` and `.`).
        let plain = !(sc.modifiers.command || sc.modifiers.alt || sc.modifiers.ctrl);
        if plain && (sc.logical_key.name().len() == 1 || matches!(sc.logical_key, Key::Slash | Key::Comma | Key::Period)) {
            continue;
        }
        if app.native_shortcuts.contains(id) {
            continue;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
            fire = Some((id, p));
            break;
        }
    }
    if let Some((id, p)) = fire {
        crate::menus::invoke(app, id, p);
        return;
    }
    // Arrow nudges (Shift = ×10, Alt = copy).
    let arrows = [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)];
    for (k, dx, dy) in arrows {
        let m = ctx.input(|i| i.modifiers);
        if ctx.input_mut(|i| i.consume_key(m, k)) && app.session.active().is_some_and(|d| !d.selection.is_empty()) {
            let _ = app.run("object.nudge", json!({"dx": dx, "dy": dy, "big": m.shift, "copy": m.alt}));
        }
    }
    // Delete / Backspace clear the selection.
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace)) && app.session.active().is_some_and(|d| !d.selection.is_empty()) {
        let _ = app.run("edit.clear", json!({}));
    }
    // Single-key tool shortcuts (no Cmd/Ctrl/Alt).
    let events: Vec<(String, Modifiers)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some((t.clone(), i.modifiers)),
                _ => None,
            })
            .collect()
    });
    for (text, m) in events {
        if m.command || m.alt || m.ctrl {
            continue;
        }
        let upper = text.to_uppercase();
        let key = if m.shift && text.chars().all(|c| c.is_alphabetic()) { format!("Shift+{upper}") } else { upper.clone() };
        // Single-key command shortcuts (X, Shift+X, D, /, Shift+D, F, Shift+F by default).
        if let Some(id) = crate::shortcut_editor::command_for_key(&key).or_else(|| crate::shortcut_editor::command_for_key(&text)) {
            let _ = app.run(id, json!({}));
            continue;
        }
        if let Some(t) = crate::shortcut_editor::tool_for_key(&key).or_else(|| crate::shortcut_editor::tool_for_key(&text)) {
            app.select_tool(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One headless frame delivering `events` to the shortcut handler.
    fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            handle(app, ui.ctx());
            app.logic(ui.ctx());
        });
        out.textures_delta.clear();
        out
    }

    #[test]
    fn copy_and_paste_events_use_the_system_clipboard() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        // Copy publishes SVG to the system clipboard (egui's CopyText output command).
        let out = frame(&mut app, vec![egui::Event::Copy]);
        let copied = out.platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        });
        let svg = copied.expect("copy publishes SVG");
        assert!(svg.contains("<svg"));
        // Pasting our own SVG back uses the internal clipboard; foreign SVG replaces it.
        frame(&mut app, vec![egui::Event::Paste(svg)]);
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
        let foreign = r##"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="5"/><circle cx="20" cy="5" r="5"/><circle cx="35" cy="5" r="5"/></svg>"##;
        frame(&mut app, vec![egui::Event::Paste(foreign.into())]);
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 5);
        // Plain text is not art: nothing is pasted from it (the internal clipboard is reused).
        frame(&mut app, vec![egui::Event::Paste("hello".into())]);
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 8);
    }

    #[test]
    fn shift_and_alt_pastes_are_paste_in_place_and_on_all_artboards() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let copied = frame(&mut app, vec![egui::Event::Copy]).platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        });
        let svg = copied.expect("copy publishes SVG");
        let last = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.last().map(|e| e.label.clone()).unwrap_or_default();
        // egui sends Cmd+V with any other modifiers as the same paste event: the keys held decide.
        let paste = |app: &mut VectorcraftApp, m: Modifiers| {
            frame(app, vec![egui::Event::ModifiersChanged(m), egui::Event::Paste(svg.clone())]);
        };
        paste(&mut app, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(last(&app), "Paste in Place");
        paste(&mut app, Modifiers::COMMAND | Modifiers::SHIFT | Modifiers::ALT);
        assert_eq!(last(&app), "Paste on All Artboards");
        paste(&mut app, Modifiers::COMMAND);
        assert_eq!(last(&app), "Paste");
    }

    #[test]
    fn comma_and_period_reapply_the_last_colour_and_gradient() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.session.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
        app.session.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
        let fill = |app: &VectorcraftApp| crate::panels::current_paints(app).0;
        frame(&mut app, vec![egui::Event::Text(".".into())]);
        assert!(matches!(fill(&app), vectorcraft_color::Paint::Gradient(_)), "`.` applies the last gradient");
        frame(&mut app, vec![egui::Event::Text(",".into())]);
        assert_eq!(fill(&app).color().unwrap().to_hex(), "#336699", "`,` applies the last colour");
    }

    #[test]
    fn parses() {
        let s = parse("Cmd+Shift+]").unwrap();
        assert_eq!(s.logical_key, Key::CloseBracket);
        assert!(s.modifiers.shift && s.modifiers.command);
        assert_eq!(parse("Cmd+=").unwrap().logical_key, Key::Equals);
        assert_eq!(parse("Cmd+Alt+2").unwrap().logical_key, Key::Num2);
        assert_eq!(parse("F12").unwrap().logical_key, Key::F12);
        assert!(parse("").is_none());
    }

    #[test]
    fn all_registered_shortcuts_parse() {
        for c in vectorcraft_engine::command_specs() {
            if let Some(s) = c.shortcut
                && s != "D"
                && s != "X"
                && s != "Shift+X"
                && s != "/"
            {
                assert!(parse(s).is_some(), "{} has unparsable shortcut {s}", c.id);
            }
        }
        for c in crate::menus::UI_COMMANDS {
            if !c.2.is_empty() && c.2 != "F" && c.2 != "Shift+F" {
                assert!(parse(c.2).is_some(), "{} has unparsable shortcut {}", c.0, c.2);
            }
        }
    }
}
