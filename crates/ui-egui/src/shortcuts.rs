//! Keyboard shortcuts: command shortcuts from the registry, single-key tool shortcuts, arrows,
//! and tool keys (Enter/Esc/↑/↓ while drawing).

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::json;
use vectorcraft_engine::cmd::clipboard::{Flavour, TEXT};
use vectorcraft_tools::{Mods, ToolKey};

use crate::VectorcraftApp;

/// Parse "Cmd+Shift+]" into an egui shortcut. `Cmd` is Command on macOS and Ctrl elsewhere.
/// `+` and `=` name one chord key ([`Key::Equals`]): `+` is Shift+`=` on many layouts, so a
/// `Cmd+=` chord also answers `+` however it is typed (see [`consume`]).
pub fn parse(s: &str) -> Option<KeyboardShortcut> {
    if s.is_empty() {
        return None;
    }
    // A bare `+` (the Add Anchor Point tool's key) is the key itself, not a separator with
    // nothing on either side.
    let (mods_part, key_part) = if s == "+" {
        ("", "+")
    } else if let Some(stripped) = s.strip_suffix("++") {
        (stripped.trim_end_matches('+'), "+")
    } else {
        s.rsplit_once('+').unwrap_or(("", s))
    };
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
        "=" | "+" => Key::Equals,
        "-" => Key::Minus,
        "Delete" => Key::Delete,
        "Backspace" => Key::Backspace,
        "Tab" => Key::Tab,
        "~" => Key::Backtick,
        // A modifier alone is never a chord's key: it couldn't fire (#487).
        k => Key::from_name(k).filter(|k| !is_modifier(*k))?,
    };
    Some(KeyboardShortcut::new(m, key))
}

/// The modifier keys, which egui also reports as key presses of their own.
pub(crate) fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft | Key::ShiftRight | Key::ControlLeft | Key::ControlRight | Key::AltLeft | Key::AltRight | Key::SuperLeft | Key::SuperRight
    )
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
    for (panel, _) in crate::state::all_panels() {
        if let Some(sc) = crate::shortcut_editor::panel_shortcut(panel).and_then(parse) {
            v.push((sc, "window.panel", json!({ "panel": panel })));
        }
    }
    settings_chord(&mut v, cfg!(target_os = "macos"));
    // Most specific (most modifiers) first so Cmd+Shift+Z isn't eaten by Cmd+Z.
    v.sort_by_key(|(sc, ..)| {
        std::cmp::Reverse(sc.modifiers.shift as u8 + sc.modifiers.alt as u8 + sc.modifiers.command as u8 + sc.modifiers.ctrl as u8)
    });
    v
}

/// On a Mac, Cmd+, opens Settings as in every Mac app (#663), beside Preferences' own shortcut,
/// unless a command or a user's shortcut already took it.
fn settings_chord(v: &mut Vec<(KeyboardShortcut, &'static str, serde_json::Value)>, mac: bool) {
    if let Some(sc) = parse("Cmd+,").filter(|sc| mac && !v.iter().any(|(s, ..)| s == sc)) {
        v.push((sc, "edit.preferences", json!({})));
    }
}

/// Consume a press of `sc`. A `=` chord also takes [`Key::Plus`], which is how `+` arrives from
/// the numpad and from layouts where it has its own key (Shift+`=` arrives as either; extra Shift
/// is ignored).
pub(crate) fn consume(i: &mut egui::InputState, sc: &KeyboardShortcut) -> bool {
    i.consume_shortcut(sc)
        || (sc.logical_key == Key::Equals && i.consume_key(sc.modifiers, Key::Plus))
        || (sc.modifiers.shift && shifted(sc.logical_key).is_some_and(|k| i.consume_key(sc.modifiers, k)))
}

/// The key a shifted punctuation key arrives as: egui reports the character typed, so Cmd+Shift+[
/// comes in as Cmd+Shift+`{` (US-style layouts).
fn shifted(k: Key) -> Option<Key> {
    Some(match k {
        Key::OpenBracket => Key::OpenCurlyBracket,
        Key::CloseBracket => Key::CloseCurlyBracket,
        Key::Slash => Key::Questionmark,
        Key::Semicolon => Key::Colon,
        Key::Backslash => Key::Pipe,
        _ => return None,
    })
}

/// Keys that paste with Cmd, or alone. (Shift+Insert is left out: Ctrl+Insert copies.)
const PASTE_KEYS: [Key; 2] = [Key::V, Key::Paste];

/// The paste command whose V chord `held` holds (Paste in Place for Cmd+Shift+V by default), else
/// Paste: egui reports every Cmd+V chord as a paste, never as its key.
pub(crate) fn paste_command(held: Modifiers) -> &'static str {
    all_shortcuts()
        .into_iter()
        .find(|(sc, id, _)| sc.logical_key == Key::V && id.starts_with("edit.paste") && held.matches_logically(sc.modifiers))
        .map_or("edit.paste", |(_, id, _)| id)
}

/// Give a copy or cut chord held with more modifiers its key back when it has a shortcut of its own:
/// egui reports every Cmd+C and Cmd+X chord as Copy or Cut, Shift and Alt held too, so Cmd+Shift+C
/// (Align center) copied instead (#1940).
fn unclip_chords(i: &mut egui::InputState) {
    if !i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)) {
        return;
    }
    let (held, shortcuts) = (i.modifiers, all_shortcuts());
    for e in &mut i.events {
        let (key, own) = match e {
            egui::Event::Copy => (Key::C, "edit.copy"),
            egui::Event::Cut => (Key::X, "edit.cut"),
            _ => continue,
        };
        if shortcuts.iter().any(|(sc, id, _)| sc.logical_key == key && *id != own && held.matches_exact(sc.modifiers)) {
            *e = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: held };
        }
    }
}

/// Keyboard pastes of something other than text. egui swallows a paste chord's key press and
/// sends a Paste event only when the clipboard holds text, so a paste key released without its
/// press (or a Paste event) reported was a paste of a bitmap or a PDF.
#[derive(Clone, Default)]
pub(crate) struct PasteChord {
    /// Paste keys whose press was reported (typed, not pasting).
    pressed: Vec<Key>,
    /// A Paste event came since the last paste key was released.
    pasted: bool,
}

impl PasteChord {
    /// Follow one frame's `events` → the modifiers of a paste egui sent no event for (the chord's,
    /// as its key is released).
    pub(crate) fn textless_paste(&mut self, events: &[egui::Event]) -> Option<Modifiers> {
        let mut fire = None;
        for e in events {
            match e {
                egui::Event::Paste(_) => self.pasted = true,
                // A release that comes while the window is away is never seen.
                egui::Event::WindowFocused(false) => *self = Self::default(),
                egui::Event::Key { key, pressed: true, .. } if PASTE_KEYS.contains(key) && !self.pressed.contains(key) => self.pressed.push(*key),
                egui::Event::Key { key, pressed: false, modifiers, .. } if PASTE_KEYS.contains(key) => {
                    match self.pressed.iter().position(|k| k == key) {
                        Some(i) => {
                            self.pressed.swap_remove(i);
                        }
                        // Unless a Paste event came (the guard takes it either way).
                        None if !std::mem::take(&mut self.pasted) => fire = Some(*modifiers),
                        None => {}
                    }
                }
                _ => {}
            }
        }
        fire
    }
}

/// End an IME composition outside the IME (a click away, the IME going away): the
/// marked text stays as typed, and the system IME is told to drop it.
pub(crate) fn keep_marked_text(app: &mut VectorcraftApp) {
    let Some(m) = app.ime_marked.take() else { return };
    app.ime_discard = true;
    if app.session.tool_composing() {
        // Errors are already reported by the engine's interaction.
        let _ = app.session.tool_text(&m, app.view_info());
    }
}

/// Cmd/Ctrl chords are shortcuts (Create Outlines, Bold, …): their letter is never typed into the
/// text if a host still emits `Text` for them (#2094). Alt is left to type: Option+key on macOS
/// and AltGr (Ctrl+Alt) on Windows keyboards type characters such as é, ø and @.
fn is_shortcut_chord(m: egui::Modifiers) -> bool {
    (m.command || m.ctrl) && !m.alt
}

/// Typed text and IME events, in the order they came, to the Type tool.
fn type_text(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let view = app.view_info();
    // Switching apps mid-composition needs nothing: the macOS IME keeps its marked text and goes
    // on with it when the window comes back (measured with Kotoeri), as the tool does.
    let shortcut_mods = ctx.input(|i| is_shortcut_chord(i.modifiers));
    let events: Vec<egui::Event> =
        ctx.input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Text(_) | egui::Event::Ime(_))).cloned().collect());
    for e in events {
        // Errors are already reported by the engine's interaction (a failed preview keeps it).
        let _ = match e {
            // Keys the IME passed through come as text only when nothing is marked.
            egui::Event::Text(t) if !app.session.tool_composing() && !shortcut_mods => app.session.tool_text(&t, view),
            egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                app.ime_marked = Some(text.clone()).filter(|t| !t.is_empty());
                app.session.tool_preedit(&text, active_range_chars, view)
            }
            // A bare line break confirms the composition (Enter): the marked text, not a newline.
            egui::Event::Ime(egui::ImeEvent::Commit(t)) if t == "\n" || t == "\r" => {
                keep_marked_text(app);
                Ok(vec![])
            }
            egui::Event::Ime(egui::ImeEvent::Commit(t)) => {
                app.ime_marked = None;
                app.session.tool_text(&t, view)
            }
            // `DeleteSurrounding` isn't sent by egui-winit (desktop); the web host has no IME
            // path to the canvas yet.
            _ => Ok(vec![]),
        };
    }
}

pub fn handle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // Taken before anything returns, so a press in a field or a dialog ends a pair of Escapes.
    let mut escape_pair = escape_press(ctx);
    // Followed before anything returns, so a key typed in a field isn't taken for a paste.
    let textless_paste = ctx.input(|i| app.paste_chord.textless_paste(&i.events));
    // A numeric field being scrubbed has the keyboard: Escape cancels the drag.
    if crate::scrub::phase(ctx) != crate::scrub::Phase::Idle {
        return;
    }
    if app.ui.dialog.is_some() || app.ui.palette_open {
        if app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active) && crate::dialogs::revolve_gizmo::dragging(ctx) {
            // Escape first restores this rotation gesture. The settings stay open.
            return;
        }
        if crate::shortcut_editor::is_recording(app) {
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            crate::dialogs::cancel(app);
            app.ui.palette_open = false;
        }
        return;
    }
    let typing = ctx.egui_wants_keyboard_input();
    // Escape closes Edit Toolbar's drawer first (egui drops a drag from it on Escape already).
    if app.ui.tools_drawer && !typing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        app.ui.tools_drawer = false;
    }
    let view = app.view_info();
    // While an IME composes, Enter and Escape are its own (confirm / cancel).
    let composing = app.session.tool_composing();
    // Tool keys first (Enter/Escape end paths; arrows change polygon sides while dragging).
    let busy = app.session.tool_busy();
    for (k, tk) in [(Key::Enter, ToolKey::Enter), (Key::Escape, ToolKey::Escape)] {
        if !typing && !composing && ctx.input(|i| i.key_pressed(k)) {
            // A key the tool claims (Esc with a loaded place cursor) is only the tool's.
            let claimed = app.session.tool_claims_key(tk, view);
            let r = app.session.tool_key(tk, Mods::default(), view);
            // Enter also takes what the tool asks of the UI: Rotate, Scale, Reflect and Shear open
            // their dialog with it.
            if claimed || (k == Key::Enter && !busy) {
                crate::canvas::apply_requests(app, r);
            }
            if claimed {
                ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
            }
            if k == Key::Escape && !busy && !claimed {
                // Escape leaves Presentation Mode first: it hides the menus and panels.
                if app.ui.screen_mode == 3 {
                    let _ = app.run("view.presentation", json!({}));
                } else if app.session.active().is_some_and(|d| d.doc.pattern_edit.is_some()) {
                    let _ = app.run("object.pattern.done", json!({}));
                } else if app.session.active().is_some_and(|d| d.isolation.is_some()) {
                    let _ = app.run("object.exitIsolation", json!({}));
                } else if app.ui.flyout.is_some() {
                    app.ui.flyout = None;
                } else if escape_pair.is_some_and(|second| escape_resets_rotation(app, ctx, second)) {
                    // Escape pressed twice resets a rotated view (with the Rotate View tool, once).
                    let _ = app.run("view.rotateReset", json!({}));
                    escape_pair = None;
                }
            }
            // An Escape that ended something can start a pair too. One that closed a popup cannot.
            if k == Key::Escape && escape_pair.is_some() && !egui::Popup::is_any_open(ctx) {
                let now = ctx.input(|i| i.time);
                ctx.data_mut(|d| d.insert_temp(escape_pair_id(), now));
            }
            if k == Key::Enter && !busy && !claimed {
                // Enter with a selection tool opens the Move dialog, as a double-click on its button
                // does.
                let tool = app.session.tool_id();
                if app.ui.dialog.is_none() && vectorcraft_tools::catalog::is_selection_tool(tool) {
                    // Nothing selected: it fails and nothing opens (the menu item is disabled then too).
                    let _ = crate::toolbar::open_options(app, tool);
                }
                // The Enter that opened a dialog isn't also its OK: dialogs take Enter, this frame too.
                if app.ui.dialog.is_some() {
                    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                }
            }
        }
    }
    if typing {
        return;
    }
    ctx.input_mut(unclip_chords);
    // Tab is ours when no field has the keyboard (it shows and hides the panels, or goes to the
    // Type tool): left to egui, it would also move the focus on to a field, and every key after it
    // would count as typing.
    if ctx.input(|i| i.key_pressed(Key::Tab)) {
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
    // In Presentation Mode, ← and → go to the previous and the next artboard. Every arrow key press
    // is removed from the input here, so no arrow key moves the selection or is sent to the tool.
    if app.ui.screen_mode == 3 {
        let mut steps = vec![];
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key: Key::ArrowLeft, pressed: true, .. } => {
                    steps.push(false);
                    false
                }
                egui::Event::Key { key: Key::ArrowRight, pressed: true, .. } => {
                    steps.push(true);
                    false
                }
                egui::Event::Key { key: Key::ArrowUp | Key::ArrowDown, pressed: true, .. } => false,
                _ => true,
            })
        });
        for next in steps {
            presentation_step(app, next);
        }
    }
    // Type tool editing: text, IME composition and editing keys go to the tool.
    if app.session.tool_wants_text() {
        type_text(app, ctx);
        // Keys of a frame that began composing are the IME's too (a Backspace that empties the
        // marked text must not delete the committed character before it).
        if composing || app.session.tool_composing() {
            // The IME owns the keyboard until it commits: no editing keys, clipboard or shortcuts.
            ctx.input_mut(|i| {
                i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)))
            });
            return;
        }
        // Editing keys with modifiers and the clipboard (Cmd+A is Select All's, below: the text).
        crate::panels::character::route_type_input(app, ctx);
        // Enter was already delivered above as ToolKey::Enter (newline).
        let fire = all_shortcuts().into_iter().filter(|(sc, ..)| sc.modifiers.command).find(|(sc, ..)| ctx.input_mut(|i| consume(i, sc)));
        if let Some((_, id, p)) = fire {
            crate::menus::invoke(app, id, p);
        }
        return;
    }
    // Clipboard keys arrive as events, not key presses; the chord held says which paste.
    let mut clip = vec![];
    ctx.input_mut(|i| {
        let held = i.modifiers;
        i.events.retain(|e| {
            let (id, text) = match e {
                egui::Event::Copy => ("edit.copy", None),
                egui::Event::Cut => ("edit.cut", None),
                egui::Event::Paste(t) => (paste_command(held), Some(t.clone())),
                _ => return true,
            };
            clip.push((id, text));
            false
        })
    });
    // Only the system clipboard service reads what isn't text.
    if let Some(id) = textless_paste.map(paste_command)
        && app.services.system_clipboard.is_some()
    {
        clip.push((id, None));
    }
    for (id, text) in clip {
        app.clipboard_in = text.map(|t| Flavour { mime: TEXT, data: t.into_bytes() });
        crate::menus::invoke(app, id, json!({}));
    }
    if busy {
        // The arrows while dragging: a polygon's sides, a grid's rows (↑/↓) and columns (←/→)…
        let arrows =
            [(Key::ArrowUp, ToolKey::Up), (Key::ArrowDown, ToolKey::Down), (Key::ArrowLeft, ToolKey::Left), (Key::ArrowRight, ToolKey::Right)];
        for (k, tk) in arrows {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k)) {
                let _ = app.session.tool_key(tk, Mods::default(), view);
            }
        }
        // Digits (5 while dragging with the Perspective Selection tool), once per press.
        let digits: Vec<u8> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, repeat: false, .. } => digit_of(*key),
                    _ => None,
                })
                .collect()
        });
        for d in digits {
            let _ = app.session.tool_key(ToolKey::Digit(d), Mods::default(), view);
        }
        // The tool gets each letter it takes (E while dragging a selection marquee) once per press,
        // without Cmd, Ctrl or Alt. The key is the one egui reports, which on a layout without Latin
        // letters is the key at that position on a US layout.
        let letters: Vec<char> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                        if !(modifiers.command || modifiers.alt || modifiers.ctrl) =>
                    {
                        letter_of(*key)
                    }
                    _ => None,
                })
                .collect()
        });
        for c in letters {
            if app.session.tool_claims_key(ToolKey::Letter(c), view) {
                let _ = app.session.tool_key(ToolKey::Letter(c), Mods::default(), view);
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
            // Keyboard actions can change the Artboard tool's active board too. Keep the
            // navigator aligned with its valid survivor, as pointer dispatch does.
            if r.is_ok()
                && app.session.tool_id() == "artboard"
                && let Some(i) = app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok())
                && app.session.active().is_some_and(|d| i < d.doc.artboards.len())
                && let Some(v) = app.view_mut()
            {
                v.artboard = i;
            }
            crate::canvas::apply_requests(app, r);
            return;
        }
    }
    // Digits the active tool or the perspective grid takes (1–4 pick the plane while it shows).
    if !(m.command || m.alt || m.ctrl) {
        let digits: Vec<(u8, Key)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => digit_of(*key).map(|d| (d, *key)),
                    _ => None,
                })
                .collect()
        });
        for (d, k) in digits {
            if app.session.tool_claims_key(ToolKey::Digit(d), view) && ctx.input_mut(|i| i.consume_key(m, k)) {
                // The digit's text is the key's too: no single-key shortcut sees it.
                let text = d.to_string();
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if *t == text)));
                let r = app.session.tool_key(ToolKey::Digit(d), crate::canvas::mods(m, false), view);
                crate::canvas::apply_requests(app, r);
                return;
            }
        }
    }
    // `[` and `]` the active tool takes (the Blob Brush's and the Eraser's size, the Symbolism
    // tools' diameter, with Shift their intensity). Read as typed, so
    // layouts where a bracket takes Option or AltGr count too; `{` and `}` are Shift's.
    if !(m.command || m.ctrl) {
        let typed: Vec<(String, ToolKey, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => match t.as_str() {
                        "[" => Some((t.clone(), ToolKey::BracketLeft, false)),
                        "]" => Some((t.clone(), ToolKey::BracketRight, false)),
                        "{" => Some((t.clone(), ToolKey::BracketLeft, true)),
                        "}" => Some((t.clone(), ToolKey::BracketRight, true)),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        });
        let mut taken = false;
        for (text, key, shift) in typed {
            if app.session.tool_claims_key(key, view) {
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if *t == text)));
                let r = app.session.tool_key(key, Mods { shift, ..Mods::default() }, view);
                crate::canvas::apply_requests(app, r);
                taken = true;
            }
        }
        if taken {
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
        if ctx.input_mut(|i| consume(i, &sc)) {
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
        if ctx.input_mut(|i| i.consume_key(m, k)) && app.session.active().is_some_and(|d| d.selection.has_objects_or_guides()) {
            let _ = app.run("object.nudge", json!({"dx": dx, "dy": dy, "big": m.shift, "copy": m.alt}));
        }
    }
    // Delete / Backspace clear the selection.
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace)) && app.session.active().is_some_and(|d| d.selection.has_objects_or_guides())
    {
        let _ = app.run("edit.clear", json!({}));
    }
    // Single-key tool shortcuts (no Cmd/Ctrl/Alt). With a layout without Latin letters (Persian,
    // Arabic, Russian, Greek…) the key's position stands in for what it types (#793).
    let events: Vec<(String, Modifiers)> = ctx.input(|i| {
        let mut physical = None;
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { physical_key, pressed: true, .. } => {
                    physical = *physical_key;
                    None
                }
                egui::Event::Text(t) => Some((latin(t, physical), i.modifiers)),
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
        // A shifted punctuation character can also be written with its Shift (the Curvature tool's
        // Shift+~ arrives as `~`).
        let shifted = m.shift.then(|| format!("Shift+{text}"));
        let lookup = |find: fn(&str) -> Option<&'static str>| find(&key).or_else(|| find(&text)).or_else(|| shifted.as_deref().and_then(find));
        // Single-key command shortcuts (X, Shift+X, D, /, Shift+D, F, Shift+F by default).
        if let Some(id) = lookup(crate::shortcut_editor::command_for_key) {
            let _ = app.run(id, json!({}));
            continue;
        }
        if let Some(t) = lookup(crate::shortcut_editor::tool_for_key) {
            app.select_tool(t);
        }
    }
}

/// The second press of Escape pressed twice comes less than this many seconds after the first.
const ESCAPE_PAIR_SECONDS: f64 = 1.0;

/// The key of the last counted Escape's time in egui's temporary data.
fn escape_pair_id() -> egui::Id {
    egui::Id::new("escape-pair")
}

/// Reads this frame's presses for Escape pressed twice. `None` when the frame has no Escape press, or
/// has another key or pointer button press as well (a key repeat counts as another press, and the
/// modifier keys do not count). Otherwise, whether the last counted Escape came less than
/// [`ESCAPE_PAIR_SECONDS`] before. A press of Escape, another key or a pointer button removes the
/// last counted Escape's time.
fn escape_press(ctx: &egui::Context) -> Option<bool> {
    let (escape, other, now) = ctx.input(|i| {
        let (mut escape, mut other) = (false, false);
        for e in &i.events {
            match e {
                egui::Event::Key { key: Key::Escape, pressed: true, repeat: false, .. } => escape = true,
                egui::Event::Key { key, pressed: true, .. } if !is_modifier(*key) => other = true,
                egui::Event::PointerButton { pressed: true, .. } => other = true,
                _ => {}
            }
        }
        (escape, other, i.time)
    });
    if !escape && !other {
        return None;
    }
    let counted = ctx.data_mut(|d| d.remove_temp::<f64>(escape_pair_id()));
    (escape && !other).then(|| counted.is_some_and(|t| now - t < ESCAPE_PAIR_SECONDS))
}

/// Does this Escape reset the view's rotation? It does when the view is rotated, no popup is open,
/// the ruler origin is not being dragged, and the press is the second of a pair (`second`) or the
/// Rotate View tool is chosen.
fn escape_resets_rotation(app: &VectorcraftApp, ctx: &egui::Context, second: bool) -> bool {
    app.view().is_some_and(|v| v.rotation != 0.0)
        && (second || app.session.tool_id() == "rotateView")
        && !egui::Popup::is_any_open(ctx)
        && !crate::canvas::ruler_origin_dragging(ctx)
}

/// Go to the next artboard (`next`) or the previous one with `view.goToArtboard`, as the status bar
/// navigator's Next and Previous buttons do, for the arrow keys and a click in Presentation Mode.
/// On the last artboard a step forward leaves the view as it is, and so does a step back on the
/// first.
pub(crate) fn presentation_step(app: &mut VectorcraftApp, next: bool) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let at = crate::panels::artboards::selected(app, n);
    let to = if next { (at + 1 < n).then_some("next") } else { (at > 0).then_some("previous") };
    if let Some(to) = to
        && let Err(e) = app.run("view.goToArtboard", json!({ "index": to }))
    {
        app.status(e);
    }
}

/// What a single-key shortcut reads for `text`, typed by the key at `physical`: the text itself,
/// or, when it holds no ASCII character (a layout without Latin letters), what that key types on a
/// US layout, so V selects the Selection tool whatever the V key types.
fn latin(text: &str, physical: Option<Key>) -> String {
    match physical {
        Some(k) if !text.is_ascii() => k.symbol_or_name().to_lowercase(),
        _ => text.to_string(),
    }
}

/// The digit a number-row or keypad key types.
fn digit_of(k: Key) -> Option<u8> {
    const DIGITS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    DIGITS.iter().position(|d| *d == k).map(|i| i as u8)
}

/// The lowercase letter a letter key A–Z stands for.
fn letter_of(k: Key) -> Option<char> {
    let mut name = k.name().chars();
    match (name.next(), name.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
        _ => None,
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
    fn option_and_altgr_still_type_but_cmd_and_ctrl_chords_do_not() {
        use egui::Modifiers as M;
        assert!(is_shortcut_chord(M::COMMAND | M::SHIFT));
        assert!(is_shortcut_chord(M::CTRL));
        assert!(!is_shortcut_chord(M::NONE));
        assert!(!is_shortcut_chord(M::SHIFT));
        assert!(!is_shortcut_chord(M::ALT), "Option+key types é, ø on macOS");
        assert!(!is_shortcut_chord(M::CTRL | M::ALT), "AltGr types @ on a German keyboard");
    }

    /// #2094: Cmd+Shift+O is Type › Create Outlines (not File › Open's Cmd+O, not the Artboard
    /// tool's Shift+O), with the Selection tool or while the Type tool edits the text.
    #[test]
    fn cmd_shift_o_creates_outlines() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        assert_eq!(crate::menus::shortcut_of("type.createOutlines"), Some("Cmd+Shift+O"));
        let open = parse("Cmd+O").unwrap();
        let outlines = parse("Cmd+Shift+O").unwrap();
        let pos = |want: egui::KeyboardShortcut, id: &str| super::all_shortcuts().iter().position(|(sc, i, _)| *i == id && *sc == want);
        assert!(pos(outlines, "type.createOutlines").unwrap() < pos(open, "file.open").unwrap(), "more modifiers win so Open does not eat the chord");
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let chord = egui::Event::Key { key: Key::O, physical_key: None, pressed: true, repeat: false, modifiers: cmd_shift };
        let is_text = |app: &VectorcraftApp, id: u64| {
            matches!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Text(_)))
        };
        let group_kids = |app: &VectorcraftApp| -> usize {
            let id = app.session.active().unwrap().selection.objects.first().copied().unwrap();
            match &app.session.doc().unwrap().doc.node(id).unwrap().kind {
                vectorcraft_doc::NodeKind::Group { children, .. } => children.len(),
                _ => 0,
            }
        };
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
        assert!(is_text(&app, id));
        frame(&mut app, vec![chord.clone()]);
        assert!(!is_text(&app, id), "Selection tool: the chord outlines the text");
        assert_eq!(group_kids(&app), 2, "H and i");
        assert_ne!(app.session.tool_id(), "artboard", "not the Artboard tool's Shift+O");
        assert!(app.ui.dialog.is_none(), "not File › Open");
        // While the Type tool edits, the same chord outlines; a Text event with Cmd held must not type.
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 80, "text": "Yo"})).unwrap()["id"].as_u64().unwrap();
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        assert!(app.session.tool_wants_text());
        frame(&mut app, vec![egui::Event::ModifiersChanged(cmd_shift), chord, egui::Event::Text("o".into())]);
        assert!(!is_text(&app, id), "Type tool: the chord outlines the text being edited");
        assert_eq!(group_kids(&app), 2, "Yo only: the stray Text with Cmd was not typed");
        assert!(app.ui.dialog.is_none(), "not File › Open while typing");
    }

    /// Cmd+Shift+B while the Type tool edits text is Type › Bold, not Hide Bounding Box (#724).
    #[test]
    fn cmd_shift_b_while_typing_is_bold_not_the_bounding_box() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.select_tool("type");
        let view = app.view_info();
        for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
            app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, 50.0, 50.0), view).unwrap();
        }
        frame(&mut app, vec![egui::Event::Text("Bold".into())]);
        assert!(app.session.tool_wants_text());
        let chord =
            egui::Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::SHIFT };
        frame(&mut app, vec![chord.clone()]);
        assert!(app.ui.view.bounding_box, "the bounding box stays");
        // The chord reached Bold: the text is in a bold face, or its family has none and says so.
        let face = crate::panels::character::text_style(&app).map(|(s, _)| s.font_style).unwrap_or_default();
        assert!(face.contains("Bold") || app.ui.status.contains("has no Bold style"), "{face:?} {:?}", app.ui.status);
        // Out of the text, the chord hides the bounding box as before.
        app.select_tool("selection");
        frame(&mut app, vec![chord]);
        assert!(!app.ui.view.bounding_box);
    }

    /// Cmd+Shift+L, C, R, J and F align paragraphs (#1111): every paragraph of the selected type,
    /// or while the Type tool edits text, those its selection touches.
    #[test]
    fn cmd_shift_letters_align_paragraphs() {
        use vectorcraft_doc::Justify;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One\nTwo"})).unwrap()["id"].as_u64().unwrap();
        let chord =
            |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::SHIFT };
        let paras = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => (0..t.paragraph_count()).map(|i| t.para_at(i).justify).collect::<Vec<_>>(),
            _ => panic!("text"),
        };
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        for (key, want) in [
            (Key::C, Justify::Center),
            (Key::R, Justify::Right),
            (Key::J, Justify::JustifyLeft),
            (Key::F, Justify::JustifyAll),
            (Key::L, Justify::Left),
        ] {
            frame(&mut app, vec![chord(key)]);
            assert_eq!(paras(&app), [want, want], "{key:?}");
        }
        // With the Type tool, only the paragraph its selection touches.
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 5, "end": 6}));
        assert!(app.session.tool_wants_text());
        frame(&mut app, vec![chord(Key::R)]);
        assert_eq!(paras(&app), [Justify::Left, Justify::Right]);
        // Nothing selected: nothing to align.
        app.select_tool("selection");
        app.session.execute("select.none", &json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "type.alignCenter"));
    }

    /// Cmd+Alt+Q resets the tracking and kerning of the selected type (tracking 0, kerning Auto),
    /// and of the Type tool's selected text while it edits.
    #[test]
    fn cmd_alt_q_resets_tracking_and_kerning() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two"})).unwrap()["id"].as_u64().unwrap();
        let spaced =
            |app: &mut VectorcraftApp| app.session.execute("text.setRangeStyle", &json!({"id": id, "tracking": 200, "kerning": -40})).unwrap();
        let spacing = |app: &mut VectorcraftApp| {
            let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
            r["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|run| (run["text"].clone(), run["style"]["tracking"].clone(), run["style"]["kerning"].clone()))
                .collect::<Vec<_>>()
        };
        let chord =
            egui::Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::ALT };
        spaced(&mut app);
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        frame(&mut app, vec![chord.clone()]);
        assert_eq!(spacing(&mut app), [(json!("One Two"), json!(0.0), json!(null))]);
        // While the Type tool edits: only its selection.
        spaced(&mut app);
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
        assert!(app.session.tool_wants_text());
        frame(&mut app, vec![chord]);
        assert_eq!(spacing(&mut app), [(json!("One "), json!(200.0), json!(-40.0)), (json!("Two"), json!(0.0), json!(null))]);
    }

    /// Cmd+Shift+X resets the horizontal scale of the selected type to 100%, and of the Type
    /// tool's selected text while it edits. egui reports the chord as a Cut with Shift held, and
    /// the type object is not cut.
    #[test]
    fn cmd_shift_x_resets_horizontal_scale() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two"})).unwrap()["id"].as_u64().unwrap();
        let stretched = |app: &mut VectorcraftApp| app.session.execute("text.setRangeStyle", &json!({"id": id, "hScale": 150})).unwrap();
        let scales = |app: &mut VectorcraftApp| {
            let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
            r["runs"].as_array().unwrap().iter().map(|run| (run["text"].clone(), run["style"]["h_scale"].clone())).collect::<Vec<_>>()
        };
        let chord = vec![egui::Event::ModifiersChanged(Modifiers::COMMAND | Modifiers::SHIFT), egui::Event::Cut];
        stretched(&mut app);
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        frame(&mut app, chord.clone());
        assert_eq!(scales(&mut app), [(json!("One Two"), json!(100.0))]);
        // While the Type tool edits: only its selection, and the text stays.
        stretched(&mut app);
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
        assert!(app.session.tool_wants_text());
        frame(&mut app, chord);
        assert_eq!(scales(&mut app), [(json!("One "), json!(150.0)), (json!("Two"), json!(100.0))]);
    }

    /// Tab shows and hides the panels, and in the Type tool it types a tab, without moving the
    /// keyboard focus on to a field: the keys after it (a tool letter, the next letter typed) still
    /// work.
    #[test]
    fn tab_leaves_the_keyboard_to_the_shortcuts_and_the_type_tool() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0));
        let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        };
        let press = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        for _ in 0..3 {
            run(&mut app, vec![]);
        }
        assert!(app.ui.dock && app.ui.toolbar);
        run(&mut app, vec![press(Key::Tab)]);
        run(&mut app, vec![]);
        assert!(!app.ui.dock && !app.ui.toolbar, "Tab hid the panels");
        run(&mut app, vec![press(Key::Tab)]);
        run(&mut app, vec![]);
        assert!(app.ui.dock && app.ui.toolbar, "a second Tab shows them again");
        run(&mut app, vec![press(Key::P), egui::Event::Text("p".into())]);
        assert_eq!(app.session.tool_id(), "pen", "and a tool letter after it works");
        // The Type tool: a, Tab, b is one text.
        app.select_tool("type");
        let view = app.view_info();
        for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
            app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, 50.0, 50.0), view).unwrap();
        }
        run(&mut app, vec![egui::Event::Text("a".into())]);
        run(&mut app, vec![press(Key::Tab)]);
        run(&mut app, vec![egui::Event::Text("b".into())]);
        let doc = app.session.execute("document.inspect", &json!({})).unwrap();
        assert_eq!(doc["layers"][0]["children"][0]["name"], "a\tb");
    }

    /// Shift+Tab shows and hides the panels and leaves the toolbar and the Control bar as they are,
    /// while Tab hides all three. Hiding the panels closes a panel popped out of the icon column.
    /// In a focused field neither key shows or hides anything.
    #[test]
    fn shift_tab_hides_the_panels_but_not_the_toolbar_or_the_control_bar() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0));
        let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        };
        let press = |modifiers| egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers };
        let shown = |app: &VectorcraftApp| (app.ui.dock, app.ui.toolbar, app.ui.control_bar);
        app.ui.control_bar = true;
        for _ in 0..3 {
            run(&mut app, vec![]);
        }
        assert_eq!(shown(&app), (true, true, true));
        app.run("window.panel", json!({"panel": "color"})).unwrap();
        run(&mut app, vec![press(Modifiers::SHIFT)]);
        run(&mut app, vec![]);
        assert_eq!(shown(&app), (false, true, true), "Shift+Tab hid the panels only");
        assert_eq!(app.ui.open_panel, None, "and the panel popped out of the icon column");
        run(&mut app, vec![press(Modifiers::SHIFT)]);
        run(&mut app, vec![]);
        assert_eq!(shown(&app), (true, true, true), "a second Shift+Tab shows them again");
        app.run("window.panel", json!({"panel": "color"})).unwrap();
        run(&mut app, vec![press(Modifiers::NONE)]);
        run(&mut app, vec![]);
        assert_eq!(shown(&app), (false, false, false), "Tab hides the toolbar and the Control bar too");
        assert_eq!(app.ui.open_panel, None);
        run(&mut app, vec![press(Modifiers::NONE)]);
        run(&mut app, vec![]);
        assert_eq!(shown(&app), (true, true, true));
        // A field of the Color panel has the keyboard focus: Shift+Tab and Tab do not show or hide anything.
        app.run("window.panel", json!({"panel": "color"})).unwrap();
        for _ in 0..3 {
            run(&mut app, vec![]);
        }
        let fields: Vec<(egui::Id, egui::Rect)> = ctx.viewport(|vp| {
            vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.sense.senses_click()).map(|w| (w.id, w.rect)).collect()
        });
        let (id, rect) =
            fields.into_iter().find(|&(id, _)| egui::TextEdit::load_state(&ctx, id).is_some()).unwrap_or_else(|| panic!("no field on screen"));
        let click =
            |pressed| egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        run(&mut app, vec![egui::Event::PointerMoved(rect.center()), click(true)]);
        run(&mut app, vec![click(false)]);
        assert!(ctx.memory(|m| m.has_focus(id)), "the field has the keyboard");
        for modifiers in [Modifiers::SHIFT, Modifiers::NONE] {
            run(&mut app, vec![press(modifiers)]);
            run(&mut app, vec![]);
            assert_eq!(shown(&app), (true, true, true), "{modifiers:?}");
            assert_eq!(app.ui.open_panel.as_deref(), Some("color"), "{modifiers:?}");
        }
    }

    /// The second Tab shows again only the bars the first Tab hid: a Control bar that was off stays
    /// off, and a toolbar hidden with Window › Tools stays hidden, after a restart too. Shift+Tab
    /// between two Tabs, and Tab between two Shift+Tabs, leave a state the same key goes back from.
    /// After a switch of workspace, Tab hides and shows that workspace's bars.
    #[test]
    fn tab_shows_again_only_the_bars_it_hid() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0));
        let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        };
        let press = |modifiers| egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers };
        let tab = |app: &mut VectorcraftApp, modifiers| {
            run(app, vec![press(modifiers)]);
            run(app, vec![]);
            (app.ui.dock, app.ui.toolbar, app.ui.control_bar)
        };
        for _ in 0..3 {
            run(&mut app, vec![]);
        }
        assert_eq!((app.ui.dock, app.ui.toolbar, app.ui.control_bar), (true, true, false), "the Control bar starts off");
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "the second Tab leaves the Control bar off");
        app.run("window.toolbar", json!({})).unwrap();
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (true, false, false), "a toolbar hidden with Window › Tools stays hidden");
        app.run("window.toolbar", json!({})).unwrap();
        // Hidden by Tab when the app quits: the next Tab after a restart shows the same bars.
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        app.ui = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "after a restart");
        // Shift+Tab after Tab shows the panels, and a second Shift+Tab hides them again; Tab then
        // shows what the first Tab hid.
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::SHIFT), (true, false, false));
        assert_eq!(tab(&mut app, Modifiers::SHIFT), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::SHIFT), (true, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "Tab after Shift+Tab");
        // Tab after Shift+Tab hides the toolbar, and a second Tab goes back to Shift+Tab's state.
        assert_eq!(tab(&mut app, Modifiers::SHIFT), (false, true, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, true, false), "Shift+Tab's state");
        // Panels shown with Shift+Tab between two Tabs stay shown.
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::SHIFT), (true, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "the panels Shift+Tab showed");
        // A bar shown another way after Tab: the next Tab hides it, and the Tab after that shows it again.
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        app.run("window.control", json!({})).unwrap();
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, true), "only the Control bar");
        // Nothing shows and Tab hid nothing: Tab shows all three.
        app.run("window.control", json!({})).unwrap();
        assert_eq!((app.ui.dock, app.ui.toolbar, app.ui.control_bar), (false, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, true));
        // After a switch of workspace, Tab hides that workspace's bars and shows them again.
        app.run("window.toolbar", json!({})).unwrap();
        app.run("window.control", json!({})).unwrap();
        app.run("window.workspace.new", json!({"name": "Panels Only"})).unwrap();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
        app.run("window.workspace", json!({"name": "Panels Only"})).unwrap();
        assert_eq!((app.ui.dock, app.ui.toolbar, app.ui.control_bar), (true, false, false));
        assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false), "Tab after a switch of workspace");
        assert_eq!(tab(&mut app, Modifiers::NONE), (true, false, false));
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

    /// While a grid is dragged out, ↑/↓ change its rows and ←/→ its columns.
    #[test]
    fn arrows_while_dragging_a_grid_change_its_rows_and_columns() {
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool("rectangularGrid");
        let view = app.view_info();
        // Drag a grid out, pressing `keys` on the way: its (horizontal, vertical) dividers.
        let grid = |app: &mut VectorcraftApp, keys: &[Key]| {
            app.session.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 80.0), view).unwrap();
            app.session.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 220.0), view).unwrap();
            for &key in keys {
                let press = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
                frame(app, vec![press]);
            }
            app.session.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 220.0), view).unwrap();
            let d = &app.session.doc().unwrap().doc;
            let g = d.layers[0].children().unwrap().last().unwrap().clone();
            let vectorcraft_doc::NodeKind::Group { children, .. } = &g.kind else { panic!("not a grid: {:?}", g.kind) };
            let bounds: Vec<_> = children.iter().filter_map(|c| c.geometric_bounds()).collect();
            (bounds.iter().filter(|b| b.height() < 0.5).count(), bounds.iter().filter(|b| b.width() < 0.5).count())
        };
        let (rows, columns) = grid(&mut app, &[]);
        assert_eq!(grid(&mut app, &[Key::ArrowRight, Key::ArrowRight, Key::ArrowUp]), (rows + 1, columns + 2));
        assert_eq!(grid(&mut app, &[Key::ArrowLeft, Key::ArrowDown]), (rows, columns + 1));
    }

    /// Enter opens the tool's dialog: the Move dialog for the selection tools, the tool's own for
    /// Rotate, Scale, Reflect and Shear. The Enter that opens it isn't also its OK.
    #[test]
    fn enter_opens_the_tools_dialog_and_leaves_it_open() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        let enter = || egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        // A whole app frame: the shortcuts first, then the UI, where a dialog takes its Enter.
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)));
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        };
        let kind = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.kind.clone());
        // The undo steps and the journal entries (the dialog's OK runs its command, which the journal records).
        let steps = |app: &VectorcraftApp| (app.session.active().unwrap().history.undo.len(), app.session.journal.len());
        app.select_tool("selection");
        frame(&mut app, vec![enter()]);
        assert_eq!(kind(&app), None, "nothing is selected");
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
        for (tool, dialog) in [
            ("selection", "move"),
            ("directSelection", "move"),
            ("groupSelection", "move"),
            ("rotate", "rotate"),
            ("scale", "scale"),
            ("reflect", "reflect"),
            ("shear", "shear"),
        ] {
            app.select_tool(tool);
            let before = steps(&app);
            frame(&mut app, vec![enter()]);
            assert_eq!(kind(&app).as_deref(), Some(dialog), "{tool}: Enter opens it");
            frame(&mut app, vec![]);
            assert_eq!(kind(&app).as_deref(), Some(dialog), "{tool}: and it stays open");
            assert_eq!(steps(&app), before, "{tool}: the Enter that opened it didn't confirm it");
            // The next Enter is its OK. With its default values only Reflect changes the art, so only
            // Reflect records an undo step.
            frame(&mut app, vec![enter()]);
            let after = (before.0 + usize::from(dialog == "reflect"), before.1 + 1);
            assert_eq!((kind(&app), steps(&app)), (None, after), "{tool}: Enter confirms");
        }
        app.select_tool("rectangle");
        frame(&mut app, vec![enter()]);
        assert_eq!(kind(&app), None, "another tool has its own Enter");
    }

    /// #1661: typed `[` and `]` reach the tool that takes them: the Blob Brush's and the Eraser's
    /// size (Option or AltGr brackets too), the stroke weight left as it is; other tools leave them
    /// alone.
    #[test]
    fn brackets_size_the_blob_brush_and_the_eraser() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("stroke.set", &json!({"weight": 3})).unwrap();
        let size = |app: &mut VectorcraftApp| app.session.tool_options()["size"].as_f64().unwrap();
        let text = |t: &str| egui::Event::Text(t.into());
        for tool in ["blobBrush", "eraser"] {
            app.select_tool(tool);
            let before = size(&mut app);
            frame(&mut app, vec![text("]")]);
            frame(&mut app, vec![text("]")]);
            assert_eq!(size(&mut app), before + 2.0, "{tool}");
            // Option-8 types `[` on a Turkish Mac keyboard.
            let option_8 = egui::Event::Key { key: Key::Num8, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT };
            frame(&mut app, vec![option_8, text("[")]);
            assert_eq!(size(&mut app), before + 1.0, "{tool}: a bracket typed with Option");
        }
        assert_eq!(app.session.paint.stroke_width, 3.0, "the stroke weight stays");
        app.select_tool("rectangle");
        frame(&mut app, vec![text("[")]);
        assert_eq!(app.session.paint.stroke_width, 3.0);
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
    fn shifted_punctuation_shortcuts_work_as_typed() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let a = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
        let b = app.session.execute("shape.ellipse", &json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        let order =
            |app: &VectorcraftApp| -> Vec<u64> { app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect() };
        let (a, b) = (a.as_u64().unwrap(), b.as_u64().unwrap());
        assert_eq!(order(&app), [a, b]);
        let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // Recorded in the Keyboard Shortcuts editor, they read as the keys (and so conflict).
        assert_eq!(crate::shortcut_editor::chord_from_event(Key::CloseCurlyBracket, cmd_shift).as_deref(), Some("Cmd+Shift+]"));
        assert_eq!(crate::shortcut_editor::chord_from_event(Key::Questionmark, cmd_shift).as_deref(), Some("Cmd+Shift+/"));
        // Cmd+Shift+] arrives as Cmd+Shift+`}`, Cmd+Shift+[ as Cmd+Shift+`{`.
        frame(&mut app, vec![press(Key::CloseCurlyBracket, cmd_shift)]);
        assert_eq!(order(&app), [b, a], "Bring to Front");
        frame(&mut app, vec![press(Key::OpenCurlyBracket, cmd_shift)]);
        assert_eq!(order(&app), [a, b], "Send to Back");
        // Cmd+Shift+/ arrives as Cmd+Shift+`?`: Search Commands.
        frame(&mut app, vec![press(Key::Questionmark, cmd_shift)]);
        assert!(app.ui.palette_open, "Search Commands");
        app.ui.palette_open = false;
        // The Curvature tool's Shift+~ arrives as the text `~` with Shift.
        frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), egui::Event::Text("~".into())]);
        assert_eq!(app.session.tool_id(), "curvature");
    }

    /// Cmd/Ctrl+Alt+Shift+. and , step the font size by five Size/Leading increments, and
    /// Cmd/Ctrl+Shift+. still steps it by one: the chord with Alt does not reach the one-step
    /// command.
    #[test]
    fn cmd_alt_shift_period_and_comma_step_the_font_size_by_five() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two", "size": 12})).unwrap()["id"].as_u64().unwrap();
        let sizes = |app: &mut VectorcraftApp| {
            let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
            r["runs"].as_array().unwrap().iter().map(|run| (run["text"].clone(), run["style"]["size"].clone())).collect::<Vec<_>>()
        };
        let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let five = Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT;
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        frame(&mut app, vec![press(Key::Period, five)]);
        assert_eq!(sizes(&mut app), [(json!("One Two"), json!(22.0))]);
        frame(&mut app, vec![press(Key::Period, Modifiers::COMMAND | Modifiers::SHIFT)]);
        assert_eq!(sizes(&mut app), [(json!("One Two"), json!(24.0))], "one step");
        frame(&mut app, vec![press(Key::Comma, five)]);
        assert_eq!(sizes(&mut app), [(json!("One Two"), json!(14.0))]);
        // While the Type tool edits: only its selection.
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
        assert!(app.session.tool_wants_text());
        frame(&mut app, vec![press(Key::Period, five)]);
        assert_eq!(sizes(&mut app), [(json!("One "), json!(14.0)), (json!("Two"), json!(24.0))]);
    }

    /// #793: with a layout without Latin letters the one-key shortcuts go by the key's position:
    /// the P key typing Persian `ح` chooses the Pen, the V key typing Russian `м` the Selection
    /// tool. A Latin layout keeps the letter typed: AZERTY's A (the Q key's place) is A.
    #[test]
    fn one_key_shortcuts_follow_the_key_on_layouts_without_latin_letters() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let typed = |key, physical, text: &str| {
            vec![
                egui::Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers: Modifiers::NONE },
                egui::Event::Text(text.into()),
            ]
        };
        frame(&mut app, typed(Key::P, Key::P, "ح"));
        assert_eq!(app.session.tool_id(), "pen");
        frame(&mut app, typed(Key::V, Key::V, "м"));
        assert_eq!(app.session.tool_id(), "selection");
        frame(&mut app, typed(Key::A, Key::Q, "a"));
        assert_eq!(app.session.tool_id(), "directSelection", "AZERTY: the letter typed");
    }

    #[test]
    fn plus_typed_any_way_zooms_in() {
        assert_eq!(parse("Cmd++"), parse("Cmd+="), "`+` and `=` are one chord key");
        assert_eq!(crate::shortcut_editor::chord_from_event(Key::Plus, Modifiers::COMMAND).as_deref(), Some("Cmd+="), "a recorded `+` too");
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let zoom = |app: &mut VectorcraftApp| app.view_mut().unwrap().zoom;
        let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // `=`, Shift+`=` (reported as `=` or as `+`), and the numpad `+` / a layout's own `+` key.
        for (key, m) in [(Key::Equals, Modifiers::COMMAND), (Key::Equals, cmd_shift), (Key::Plus, cmd_shift), (Key::Plus, Modifiers::COMMAND)] {
            let before = zoom(&mut app);
            frame(&mut app, vec![press(key, m)]);
            assert!(zoom(&mut app) > before, "{key:?} with {m:?} zooms in");
        }
        let before = zoom(&mut app);
        frame(&mut app, vec![press(Key::Minus, Modifiers::COMMAND)]);
        assert!(zoom(&mut app) < before, "Cmd+- zooms out");
    }

    /// `+` selects the Add Anchor Point tool, as `-` selects Delete Anchor Point (#2142): typed as
    /// Shift+`=` on a US layout, or by a layout's own `+` key or the numpad's.
    #[test]
    fn plus_selects_the_add_anchor_point_tool() {
        assert_eq!(parse("+"), parse("="), "a bare `+` is the `=` key");
        assert_eq!(crate::shortcut_editor::tool_for_key("+"), Some("addAnchor"));
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        for m in [Modifiers::SHIFT, Modifiers::NONE] {
            app.select_tool("selection");
            frame(&mut app, vec![egui::Event::ModifiersChanged(m), egui::Event::Text("+".into())]);
            assert_eq!(app.session.tool_id(), "addAnchor", "`+` with {m:?}");
        }
        frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::NONE), egui::Event::Text("-".into())]);
        assert_eq!(app.session.tool_id(), "deleteAnchor");
    }

    #[test]
    fn digit_keys_reach_a_dragging_tool_once_per_press() {
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        app.session.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 450, "y": 380, "width": 60, "height": 60})).unwrap()["id"].clone();
        app.session.execute("perspective.attach", &json!({"ids": [id], "plane": "right"})).unwrap();
        app.select_tool("perspectiveSelection");
        let v = app.view_info();
        let c =
            app.session.doc().unwrap().doc.node(vectorcraft_engine::doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().center();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, c.x, c.y), v).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, c.x + 30.0, c.y), v).unwrap();
        let key = |repeat| egui::Event::Key { key: Key::Num5, physical_key: None, pressed: true, repeat, modifiers: Modifiers::NONE };
        // A held key repeats: only its first press toggles.
        frame(&mut app, vec![key(false), key(true)]);
        let preview = app.session.doc().unwrap().interaction.as_ref().unwrap().preview.clone().unwrap();
        assert_eq!((preview.0.as_str(), &preview.1["perpendicular"]), ("perspective.move", &json!(true)));
        assert_eq!(digit_of(Key::Num0), Some(0));
        assert_eq!(digit_of(Key::A), None);
    }

    /// E pressed while the Selection tool drags a marquee makes it select only the objects inside
    /// it: once per press, without Cmd, and from the key at E's position on a layout without Latin
    /// letters.
    #[test]
    fn e_reaches_a_dragged_marquee_once_per_press() {
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        let inside = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
        let touched = app.session.execute("shape.rectangle", &json!({"x": 300, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.select_tool("selection");
        let e = |key, repeat, modifiers| egui::Event::Key { key, physical_key: Some(Key::E), pressed: true, repeat, modifiers };
        let mut marquee = |events: Vec<egui::Event>| {
            let v = app.view_info();
            app.session.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
            app.session.pointer(&PointerEvent::new(PointerKind::Drag, 350.0, 250.0), v).unwrap();
            frame(&mut app, events);
            app.session.pointer(&PointerEvent::new(PointerKind::Up, 350.0, 250.0), v).unwrap();
            let ids = app.session.doc().unwrap().selection.objects.iter().map(|i| json!(i.0)).collect::<Vec<_>>();
            assert_eq!(app.session.tool_id(), "selection");
            ids
        };
        assert_eq!(marquee(vec![]), vec![inside.clone(), touched.clone()]);
        // A held key repeats: only its first press switches.
        assert_eq!(marquee(vec![e(Key::E, false, Modifiers::NONE), e(Key::E, true, Modifiers::NONE)]), vec![inside.clone()]);
        let up = egui::Event::Key { key: Key::E, physical_key: Some(Key::E), pressed: false, repeat: false, modifiers: Modifiers::NONE };
        let twice = vec![e(Key::E, false, Modifiers::NONE), up, e(Key::E, false, Modifiers::NONE)];
        assert_eq!(marquee(twice), vec![inside.clone(), touched.clone()], "pressed twice: what it touches");
        // A Russian layout: egui-winit reports the key at E's position, and the text it types.
        assert_eq!(marquee(vec![e(Key::E, false, Modifiers::NONE), egui::Event::Text("у".into())]), vec![inside.clone()]);
        assert_eq!(marquee(vec![e(Key::E, false, Modifiers::COMMAND)]), vec![inside, touched], "Cmd+E is not E");
        assert_eq!(letter_of(Key::E), Some('e'));
        assert_eq!(letter_of(Key::Num1), None);
        assert_eq!(letter_of(Key::Escape), None);
    }

    /// Cmd+, opens Settings on a Mac only, and never takes a chord a command already has.
    #[test]
    fn cmd_comma_opens_settings_on_a_mac() {
        let comma = parse("Cmd+,").unwrap();
        let mut v = vec![];
        super::settings_chord(&mut v, false);
        assert!(v.is_empty(), "not elsewhere");
        super::settings_chord(&mut v, true);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].0, v[0].1), (comma, "edit.preferences"));
        let mut taken = vec![(comma, "view.zoomIn", json!({}))];
        super::settings_chord(&mut taken, true);
        assert_eq!(taken.len(), 1, "a command that has it keeps it");
        if cfg!(target_os = "macos") {
            assert!(super::all_shortcuts().iter().any(|(sc, id, _)| *sc == comma && *id == "edit.preferences"));
        }
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
