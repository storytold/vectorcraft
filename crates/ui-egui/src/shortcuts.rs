//! Keyboard shortcuts: command shortcuts from the registry, single-key tool shortcuts, arrows,
//! and tool keys (Enter/Esc/↑/↓ while drawing).

use craft_keymap::egui::Hit;
use craft_keymap::{Active, CommandRef};
use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::json;
use vectorcraft_engine::cmd::clipboard::{Flavour, TEXT};
use vectorcraft_engine::{Result as EngineResult, UiRequest, ViewInfo};
use vectorcraft_tools::{Mods, SurroundingUnit, ToolKey};

use crate::VectorcraftApp;

mod paste;
mod presentation;
mod typing;

use paste::unclip_chords;
pub(crate) use paste::{PasteChord, paste_command};
pub(crate) use presentation::presentation_step;
use presentation::{escape_pair_id, escape_press, escape_resets_rotation, presentation_keys};
#[cfg(test)]
use typing::is_shortcut_chord;
pub(crate) use typing::keep_marked_text;
use typing::{drop_typing_presses, type_text};

/// Parse "Cmd+Shift+]" into an egui shortcut. `Cmd` is Command on macOS and Ctrl elsewhere.
/// `+` and `=` name one chord key ([`Key::Equals`]): `+` is Shift+`=` on many layouts, so a
/// `Cmd+=` chord also answers `+` however it is typed.
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
    // Most specific (most modifiers) first: `paste_command` takes the first V chord the held keys
    // match logically, so Cmd+Shift+V isn't taken by Cmd+V.
    v.sort_by_key(|(sc, ..)| {
        std::cmp::Reverse(sc.modifiers.shift as u8 + sc.modifiers.alt as u8 + sc.modifiers.command as u8 + sc.modifiers.ctrl as u8)
    });
    v
}

/// Does every press of `key` this frame hold a chord with modifiers that a command shortcut is bound
/// to, matched as [`consume`] matches it? `cmd_only`: only chords with Cmd count. False when `key`
/// isn't pressed.
fn runs_a_command(i: &egui::InputState, key: Key, cmd_only: bool) -> bool {
    let presses: Vec<Modifiers> = i
        .events
        .iter()
        .filter_map(|e| match e {
            egui::Event::Key { key: k, pressed: true, modifiers, .. } if *k == key => Some(*modifiers),
            _ => None,
        })
        .collect();
    let shortcuts = all_shortcuts();
    let bound = |m: &Modifiers| {
        shortcuts
            .iter()
            .any(|(sc, ..)| sc.logical_key == key && (sc.modifiers.command || (!cmd_only && sc.modifiers.any())) && m.matches_logically(sc.modifiers))
    };
    !presses.is_empty() && presses.iter().all(bound)
}

/// Whether a key press runs a shortcut of the keymap in effect, the user's own included. The web
/// build asks this of every Cmd and Ctrl press, before egui sees it, to cancel what the browser
/// would also do with it (zoom the page on `Cmd+=`, undo on `Cmd+Z`, go back on `Cmd+[`).
/// `key` and `code` are the browser's names for the character typed and the key's position, which
/// egui reads with [`Key::from_name`]; `modifiers` as egui reports them. The press is matched as
/// the dispatcher matches it ([`dispatch`]) on a layout about which nothing is known: what typing
/// has shown of the layout is left out, and so are double presses.
pub fn runs_a_shortcut(key: &str, code: &str, modifiers: Modifiers) -> bool {
    let physical_key = Key::from_name(code);
    let Some(key) = Key::from_name(key).or(physical_key) else { return false };
    let press = egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers };
    let Some(input) = craft_keymap::KeyInput::from_egui(&press, crate::keymap::platform()) else { return false };
    let mirror = crate::shortcut_editor::current();
    craft_keymap::Layout::default().candidates(&input).iter().any(|c| mirror.keymap.lookup(c, false, &Active::NONE) != craft_keymap::Lookup::None)
}

/// Process one frame's events against the keymap at most once per context, viewport and frame, so
/// reruns cannot fire a shortcut or tool key twice.
pub fn handle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // One UI pass takes a frame's events. A discarded rerun and a test helper that also calls
    // `logic` see the same events again; they must not fire a shortcut or tool key twice. The
    // key includes the context (a restarted host starts at frame 0) and the viewport (frame
    // numbers are per viewport), and remembers every key taken this host iteration: an
    // interleaved root/auxiliary pass cannot make an earlier frame look new again.
    let frame = (crate::graphics::context_tag(ctx), ctx.viewport_id(), ctx.cumulative_frame_nr_for(ctx.viewport_id()));
    if !crate::take_frame_once(&mut app.shortcuts_frames, frame) {
        return;
    }
    if !(ctx.input(|input| input.focused) || app.forwarded_frame && ctx.viewport_id() == egui::ViewportId::ROOT) {
        crate::ime::controls_without_editor(app, ctx);
        return;
    }
    // Taken before anything returns, so a press in a field or a dialog ends a pair of Escapes.
    let mut escape_pair = escape_press(ctx);
    // Followed before anything returns, so a key typed in a field isn't taken for a paste.
    let textless_paste = ctx.input(|i| app.paste_chord.textless_paste(&i.events));
    // A numeric field being scrubbed has the keyboard: Escape cancels the drag.
    if crate::scrub::phase(ctx) != crate::scrub::Phase::Idle {
        crate::ime::controls_without_editor(app, ctx);
        return;
    }
    if app.ui.dialog.is_some() || app.ui.palette_open {
        crate::ime::controls_without_editor(app, ctx);
        if app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active) && crate::dialogs::revolve_gizmo::dragging(ctx) {
            // Escape first restores this rotation gesture. The settings stay open.
            return;
        }
        if crate::shortcut_editor::is_recording(app) {
            return;
        }
        // Escape closes a dropdown list or popover open in the dialog (each closes itself on it) and
        // leaves the dialog open.
        let popup = app.ui.dialog.as_ref().is_some_and(|d| egui::Popup::is_any_open(ctx) || crate::dialogs::brush_options::tips_open(d));
        if !popup && ctx.input(|i| i.key_pressed(Key::Escape)) {
            crate::dialogs::cancel(app);
            app.ui.palette_open = false;
        }
        return;
    }
    if app.ui.screen_mode == 3 {
        crate::ime::controls_without_editor(app, ctx);
        presentation_keys(app, ctx, escape_pair);
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
            // A chord a command is bound to (Cmd+Enter) runs only that command, in the shortcuts
            // below, also when the tool claims the key. While the Type tool edits text only Cmd
            // chords run one, and while another tool is busy none does.
            let text = app.session.tool_wants_text();
            if (text || !busy) && ctx.input(|i| runs_a_command(i, k, text)) {
                continue;
            }
            // A key the tool claims (Esc with a loaded place cursor) is only the tool's.
            let claimed = app.session.tool_claims_key(tk, view);
            let mods = ctx.input(|i| {
                let m = i
                    .events
                    .iter()
                    .rev()
                    .find_map(|e| match e {
                        egui::Event::Key { key, pressed: true, modifiers, .. } if *key == k => Some(*modifiers),
                        _ => None,
                    })
                    .unwrap_or(i.modifiers);
                crate::canvas::mods(m, false)
            });
            let r = app.session.tool_key(tk, mods, view);
            // Enter also takes what the tool asks of the UI: Rotate, Scale, Reflect and Shear open
            // their dialog with it.
            if claimed || (k == Key::Enter && !busy) {
                crate::canvas::apply_requests(app, r);
            }
            if claimed {
                ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
                ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, k));
            }
            if k == Key::Escape && !busy && !claimed {
                if app.session.active().is_some_and(|d| d.doc.pattern_edit.is_some()) {
                    let _ = app.run("object.pattern.done", json!({}));
                } else if app.session.active().is_some_and(|d| d.isolation.is_some()) {
                    let _ = app.run("object.exitIsolation", json!({}));
                } else if app.session.active().is_some_and(|d| d.global_edit.is_some()) {
                    let _ = app.run("select.globalEdit", json!({}));
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
        crate::ime::controls_without_editor(app, ctx);
        return;
    }
    ctx.input_mut(unclip_chords);
    // Tab is ours when no field has the keyboard (it shows and hides the panels, or goes to the
    // Type tool): left to egui, it would also move the focus on to a field, and every key after it
    // would count as typing.
    if ctx.input(|i| i.key_pressed(Key::Tab)) {
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
    // While the Type tool edits text, a character typed with AltGr runs no command chord.
    if app.session.tool_wants_text() {
        ctx.input_mut(drop_typing_presses);
    }
    if app.session.tool_wants_text() {
        type_tool_input(app, ctx, composing);
        return;
    }
    crate::ime::controls_without_editor(app, ctx);
    handle_clipboard(app, ctx, textless_paste);
    if busy {
        busy_tool_keys(app, ctx, view);
        return;
    }
    if claimed_tool_keys(app, ctx, view) {
        return;
    }
    // Command, panel and tool shortcuts. Keys that type a character without Cmd/Alt/Ctrl are
    // handled below as text (tool shortcuts, X, D, /, `,` and `.`).
    let hits = dispatch(app, ctx, |hit| !(hit.trigger.chord.is_plain() && types_text(hit.trigger.chord.key)));
    let ran = !hits.is_empty();
    for c in hits {
        run_hit(app, ctx, &c);
    }
    if settings_on_a_mac(app, ctx, mac(ctx)) || ran {
        return;
    }
    if alt_type_arrows(app, ctx) {
        return;
    }
    nudge_and_clear(app, ctx);
    single_key_shortcuts(app, ctx);
}

/// Dispatch unmodified text keys to command bindings or tools.
fn single_key_shortcuts(app: &mut VectorcraftApp, ctx: &egui::Context) {
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
    let keymap = crate::shortcut_editor::current();
    for (text, m) in events {
        if m.command || m.alt || m.ctrl {
            continue;
        }
        let upper = text.to_uppercase();
        let key = if m.shift && text.chars().all(|c| c.is_alphabetic()) { format!("Shift+{upper}") } else { upper.clone() };
        // A shifted punctuation character can also be written with its Shift (the Curvature tool's
        // Shift+~ arrives as `~`).
        let shifted = m.shift.then(|| format!("Shift+{text}"));
        let hits: Vec<CommandRef> = [Some(key), Some(text), shifted]
            .into_iter()
            .flatten()
            .filter_map(|t| crate::keymap::vc_chord(&t).and_then(|c| crate::keymap::lookup(&keymap.keymap, &c)))
            .collect();
        // Single-key command shortcuts (X, Shift+X, D, /, Shift+D, F, Shift+F by default) before
        // tools.
        let Some(hit) = hits.iter().find(|c| !c.id.starts_with("tool:")).or(hits.first()) else { continue };
        match hit.id.strip_prefix("tool:") {
            Some(tool) => app.select_tool(tool),
            None if hit.params.is_null() => {
                let _ = app.run(&hit.id, json!({}));
            }
            None => crate::menus::invoke(app, &hit.id, hit.params.clone()),
        }
    }
}

/// Route Alt+arrows to spacing controls when text objects are selected.
fn alt_type_arrows(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    let m = ctx.input(|i| i.modifiers);
    if !m.alt
        || !app.session.active().is_some_and(|st| {
            st.selection.guides.is_empty()
                && !st.selection.objects.is_empty()
                && st.selection.objects.iter().all(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Text(_))))
        })
    {
        return false;
    }
    let keys = [(Key::ArrowLeft, ToolKey::Left), (Key::ArrowRight, ToolKey::Right), (Key::ArrowUp, ToolKey::Up), (Key::ArrowDown, ToolKey::Down)];
    for (key, tool_key) in keys {
        if ctx.input_mut(|i| i.consume_key(m, key)) {
            if let Some((attribute, by)) = vectorcraft_tools::text::step_key(tool_key, m.shift, false) {
                let by = if m.command { by * 5.0 } else { by };
                let _ = app.run("type.step", json!({"attribute": attribute, "by": by}));
            }
            return true;
        }
    }
    false
}

/// Route text-tool input and command chords while preserving IME composition ownership.
fn type_tool_input(app: &mut VectorcraftApp, ctx: &egui::Context, composing: bool) {
    // Route-level loss (no current target or caret geometry) retires a live guard before any
    // marker in this pass can be considered. The finalizer may re-arm on a later activation.
    crate::ime::suspend_if_route_lost(app, ctx);
    let old_composition_retired = type_text(app, ctx);
    // Keys of a frame that began composing are the IME's too (a Backspace that empties the
    // marked text must not delete the committed character before it).
    if composing && !old_composition_retired || app.session.tool_composing() {
        // The IME owns the keyboard until it commits: no editing keys, clipboard or shortcuts.
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))));
        return;
    }
    // A deferred pointer/navigation notice has not established its destination yet. Its
    // clipboard suffix must not cut or paste at the old caret; only the notice's own actual
    // handler (kept in the raw vector) may run.
    if crate::ime::deferred(app, ctx) {
        ctx.input_mut(|input| {
            input.events.retain(|event| match event {
                egui::Event::Cut | egui::Event::Paste(_) => false,
                egui::Event::Key { key: egui::Key::X | egui::Key::V, pressed: true, modifiers, .. }
                    if modifiers.command && !modifiers.shift && !modifiers.alt =>
                {
                    false
                }
                _ => true,
            })
        });
    }
    // Editing keys with modifiers and the clipboard (Cmd+A is Select All's, below: the text).
    crate::panels::character::route_type_input(app, ctx);
    // Enter was already delivered above as ToolKey::Enter (newline). Only chords with the
    // command key run commands: the rest type or edit.
    let hits = dispatch(app, ctx, |hit| hit.trigger.chord.mods.cmd);
    for c in hits {
        run_hit(app, ctx, &c);
    }
    settings_on_a_mac(app, ctx, mac(ctx));
}

/// Apply arrow-key movement and clear selected art when no text editor owns input.
fn nudge_and_clear(app: &mut VectorcraftApp, ctx: &egui::Context) {
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
}

/// Route clipboard events to the system clipboard or the app's rich-format paste path.
fn handle_clipboard(app: &mut VectorcraftApp, ctx: &egui::Context, textless_paste: Option<Modifiers>) {
    // A clipboard image (eframe's `clipboard` feature, 0.37): the host's text paste path never
    // carries it. With the app's rich-format `SystemClipboard`, it is only the signal that the
    // clipboard holds no text — the next tick invokes Paste, whose `adopt_system_clipboard` reads
    // the formats once (never paste twice). Without one (the web/simple host), turn the pixels
    // into a PNG flavour and paste that.
    let image_paste = app.services.system_clipboard.is_none();
    let mut pasted_image: Option<Vec<u8>> = None;
    let mut read_image_formats = false;
    // `MiddleClickPaste` is deliberately left in `i.events`: the Type tool above already took it,
    // and otherwise egui's text field under the pointer (focused or not) pastes it later in this
    // pass. It is not a shortcut.
    ctx.input_mut(|i| {
        i.events.retain(|e| match e {
            egui::Event::PasteImage(image) => {
                if image_paste {
                    if pasted_image.is_none() {
                        let pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                        let rendered = vectorcraft_render::Rendered {
                            width: u32::try_from(image.size[0]).unwrap_or(0),
                            height: u32::try_from(image.size[1]).unwrap_or(0),
                            pixels,
                        };
                        // A renderer failure leaves the paste as the no-op an empty flavour gets.
                        pasted_image = rendered.to_png().ok();
                    }
                } else {
                    read_image_formats = true;
                }
                false
            }
            _ => true,
        })
    });
    if read_image_formats {
        // A text paste in the same frame wins, as it does for a clipboard with text and a picture:
        // the paste chord path below reads the text. An image-only clipboard is read here.
        if pasted_image.is_none() && textless_paste.is_none() && !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))) {
            // Remember the chord as it was when the image was pasted: the next `tick` runs in a
            // later input pass whose modifiers need not describe this paste. The request also
            // schedules that tick; nothing else may be coming to run it.
            app.clipboard_image_request = Some(ctx.input(|i| i.modifiers));
            ctx.request_repaint();
        }
    }
    if let Some(png) = pasted_image {
        let held = ctx.input(|i| i.modifiers);
        app.clipboard_in = Some(Flavour { mime: vectorcraft_engine::cmd::clipboard::PNG, data: png });
        crate::menus::invoke(app, paste_command(held), json!({}));
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
}

/// Route arrows, digits and letters to a tool while a gesture is active.
fn busy_tool_keys(app: &mut VectorcraftApp, ctx: &egui::Context, view: ViewInfo) {
    // The arrows while dragging: a polygon's sides, a grid's rows (↑/↓) and columns (←/→)…
    let arrows = [(Key::ArrowUp, ToolKey::Up), (Key::ArrowDown, ToolKey::Down), (Key::ArrowLeft, ToolKey::Left), (Key::ArrowRight, ToolKey::Right)];
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
    // Tab, when the tool takes it (the Smart Pen Tool placing an anchor).
    if app.session.tool_claims_key(ToolKey::Tab, view) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
        let _ = app.session.tool_key(ToolKey::Tab, Mods::default(), view);
    }
    // The tool gets each letter it takes (E while dragging a selection marquee) once per press,
    // without Cmd, Ctrl or Alt. The key is the one egui reports, which on a layout without Latin
    // letters is the key at that position on a US layout.
    let letters: Vec<char> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if !(modifiers.command || modifiers.alt || modifiers.ctrl) => {
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
    // Between clicks, the active tool's shortcut chooses that tool again (W with the Blend
    // tool's first pick held finishes the blend). While the button is held, it does nothing.
    if !app.session.pointer_held() {
        let presses = ctx.input(|i| shortcut_presses(&i.events, i.modifiers));
        let keymap = crate::shortcut_editor::current();
        let active = app.session.tool_id();
        let again = presses.iter().any(|(names, letter)| {
            let claimed = letter.is_some_and(|c| app.session.tool_claims_key(ToolKey::Letter(c), view));
            // The first name bound to anything is the press's shortcut (Shift+W is the Width
            // tool's, not W's).
            !claimed
                && names
                    .iter()
                    .find_map(|name| crate::keymap::vc_chord(name).and_then(|c| crate::keymap::lookup(&keymap.keymap, &c)))
                    .is_some_and(|c| c.id.strip_prefix("tool:") == Some(active))
        });
        if again {
            app.select_tool(active);
        }
    }
}

/// Give keys claimed by the active tool precedence over their command shortcuts.
fn claimed_tool_keys(app: &mut VectorcraftApp, ctx: &egui::Context, view: ViewInfo) -> bool {
    let m = ctx.input(|i| i.modifiers);
    for (k, tk) in [
        (Key::Delete, ToolKey::Delete),
        (Key::Backspace, ToolKey::Backspace),
        (Key::ArrowLeft, ToolKey::Left),
        (Key::ArrowRight, ToolKey::Right),
        (Key::ArrowUp, ToolKey::Up),
        (Key::ArrowDown, ToolKey::Down),
        // The Smart Pen Tool's Tab while it draws a path (the key otherwise hides the panels).
        (Key::Tab, ToolKey::Tab),
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
            return true;
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
                return true;
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
            return true;
        }
    }
    false
}

/// Runs this frame's key presses against the keymap in effect; `accept` declines a hit, leaving
/// its press (and the text it typed) to what follows.
fn dispatch(app: &mut VectorcraftApp, ctx: &egui::Context, accept: impl FnMut(&Hit) -> bool) -> Vec<CommandRef> {
    let mirror = crate::shortcut_editor::current();
    let dispatcher = &mut app.dispatcher;
    ctx.input_mut(|i| {
        // Shift+`=` types `+`: as the Plus key it also reaches a `=` chord held with Shift
        // (Cmd+Shift+= zooms in as Cmd+= does).
        for e in &mut i.events {
            if let egui::Event::Key { key, modifiers, .. } = e
                && *key == Key::Equals
                && modifiers.shift
            {
                *key = Key::Plus;
            }
        }
        dispatcher.dispatch_filtered(i, &mirror.keymap, &Active::NONE, accept).into_iter().map(|h| h.command).collect()
    })
}

/// Runs what a shortcut is bound to: a tool, a panel (`window.panel {panel}`) or a command.
fn run_hit(app: &mut VectorcraftApp, ctx: &egui::Context, c: &CommandRef) {
    if let Some(tool) = c.id.strip_prefix("tool:") {
        app.select_tool(tool);
    } else if c.id == "select.all" && crate::panels::artboards::selects_all(app, ctx) {
        // With the Artboard tool, Select All's keys after a click on an Artboards panel row select
        // every artboard instead of the art.
        crate::panels::artboards::select_all(app);
    } else {
        let p = if c.params.is_null() { json!({}) } else { c.params.clone() };
        crate::menus::invoke(app, &c.id, p);
    }
}

/// Keys that type a character: letters, digits and punctuation (not Space).
fn types_text(key: craft_keymap::Key) -> bool {
    key.name().chars().count() == 1
}

/// Whether the app runs on a Mac (on the web, in a browser on a Mac).
fn mac(ctx: &egui::Context) -> bool {
    ctx.os() == egui::os::OperatingSystem::Mac
}

/// On a Mac, Cmd+, opens Settings as in every Mac app (#663), beside Preferences' own shortcut,
/// when no shortcut took the press this frame. Runs after [`dispatch`]. `true` when it ran.
fn settings_on_a_mac(app: &mut VectorcraftApp, ctx: &egui::Context, mac: bool) -> bool {
    if !mac {
        return false;
    }
    let pressed = ctx.input_mut(|i| {
        let at = i.events.iter().position(
            |e| matches!(e, egui::Event::Key { key: Key::Comma, pressed: true, modifiers, .. } if modifiers.matches_exact(Modifiers::COMMAND)),
        );
        at.map(|at| i.events.remove(at)).is_some()
    });
    if pressed {
        crate::menus::invoke(app, "edit.preferences", json!({}));
    }
    pressed
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

/// Each new press's shortcut names, first match first, and the letter it types, without Cmd, Ctrl
/// or Alt. A text-only press counts as well (Shift+~ arrives as the text `~` with Shift), unless
/// the key that typed it repeated. A text's key is the press right before it (egui-winit) or,
/// failing that, right after it (eframe's web build with no text field focused). Text reads its
/// key's modifiers, or the latest modifier change (`held` before any), and tries its key's own
/// name first.
fn shortcut_presses(events: &[egui::Event], held: Modifiers) -> Vec<(Vec<String>, Option<char>)> {
    let key_name = |key: &Key, m: &Modifiers| if m.shift { format!("Shift+{}", key.name()) } else { key.name().to_string() };
    let press_at = |j: Option<usize>| match j.and_then(|j| events.get(j)) {
        Some(egui::Event::Key { key, pressed: true, repeat, modifiers, .. }) => Some((*repeat, *modifiers, key_name(key, modifiers))),
        _ => None,
    };
    let mut latest = held;
    events
        .iter()
        .enumerate()
        .filter_map(|(j, e)| match e {
            egui::Event::ModifiersChanged(m) => {
                latest = *m;
                None
            }
            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if !(modifiers.command || modifiers.alt || modifiers.ctrl) => {
                Some((vec![key_name(key, modifiers)], letter_of(*key)))
            }
            egui::Event::Text(t) => {
                let typed_by = press_at(j.checked_sub(1)).or_else(|| press_at(Some(j + 1)));
                let (repeated, mods, key_name) = typed_by.map_or((false, latest, None), |(r, m, n)| (r, m, Some(n)));
                if repeated || mods.command || mods.alt || mods.ctrl {
                    return None;
                }
                let upper = t.to_uppercase();
                let alpha = t.chars().all(|c| c.is_alphabetic());
                // A shifted letter is only Shift+letter; other shifted text may also be written
                // without its Shift.
                let text_names = match (mods.shift, alpha) {
                    (true, true) => vec![format!("Shift+{upper}")],
                    (true, false) => vec![upper, t.clone(), format!("Shift+{t}")],
                    (false, _) => vec![upper, t.clone()],
                };
                let names: Vec<String> = key_name.into_iter().chain(text_names).collect();
                let mut chars = t.chars();
                let letter = chars.next().filter(|c| c.is_ascii_alphabetic() && chars.next().is_none()).map(|c| c.to_ascii_lowercase());
                Some((names, letter))
            }
            _ => None,
        })
        .collect()
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
mod tests;
