//! Hold shortcuts (Edit › Keyboard Shortcuts, Hold Shortcuts on, the Hold column): a character key
//! with a hold shortcut waits when it goes down. Held for the Hold Shortcut Delay (`keyHoldDelay`),
//! it runs its hold shortcut once, and its repeats are ignored until it is let go. Let go sooner, it
//! runs what a press of the key runs without a hold shortcut, at the release. A key without a hold
//! shortcut runs on its press, as before, and so does every key while Hold Shortcuts is off.

use super::{Key, Modifiers, VectorcraftApp};
use vectorcraft_tools::ToolKey;

/// A press of a key with a hold shortcut, waiting for its release or for the delay to pass.
pub(crate) struct Pending {
    /// The entry the hold shortcut runs (`tool:<id>`, a command id or `panel:<id>`).
    entry: String,
    /// The chords the press names, first match first, for the tap.
    chords: Vec<craft_keymap::Chord>,
    /// The key as egui reported the press, before any rewrite of the dispatcher.
    key: Key,
    /// The key's position, when the press gave one.
    physical: Option<Key>,
    /// The modifiers held at the press.
    mods: Modifiers,
    /// When the key went down (`egui::InputState::time`).
    since: f64,
    /// The Hold Shortcut Delay read at the press: a later change does not move this wait.
    delay: f64,
    /// The viewport whose frame saw the press. Frames of other viewports leave the wait alone.
    viewport: egui::ViewportId,
}

/// A key whose repeats, and the text they type, are dropped until it is let go.
#[derive(Clone, Copy, PartialEq)]
struct Swallowed {
    key: Key,
    physical: Option<Key>,
    viewport: egui::ViewportId,
}

/// The hold shortcut state of the app: at most one waiting press, and the keys held down after
/// their wait ended.
#[derive(Default)]
pub(crate) struct HoldKeys {
    pending: Option<Pending>,
    swallowed: Vec<Swallowed>,
}

/// Is `e` the release of the key `key` at `physical`? The position decides when both events give
/// one; otherwise the key decides.
fn releases(e: &egui::Event, key: Key, physical: Option<Key>) -> bool {
    matches!(e, egui::Event::Key { key: k, physical_key: p, pressed: false, .. } if same_key(*k, *p, key, physical))
}

fn same_key(k: Key, p: Option<Key>, key: Key, physical: Option<Key>) -> bool {
    match (p, physical) {
        (Some(p), Some(q)) => p == q,
        _ => k == key || Some(k) == physical,
    }
}

/// Is the key still down? egui keeps the keys down by the key it reported for the press.
fn is_down(i: &egui::InputState, key: Key, physical: Option<Key>) -> bool {
    i.key_down(key) || physical.is_some_and(|p| i.key_down(p))
}

/// What stops a wait: anything that takes the keyboard from the canvas.
fn blocked(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    crate::scrub::phase(ctx) != crate::scrub::Phase::Idle
        || crate::updates_ui::blocks_input(app)
        || app.ui.dialog.is_some()
        || app.ui.palette_open
        || app.ui.screen_mode == 3
        || ctx.egui_wants_keyboard_input()
        || app.session.tool_wants_text()
        || app.session.tool_composing()
        || app.session.tool_busy()
        // An input method composing on the canvas with any tool.
        || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) if !text.is_empty())))
}

/// Every frame, before anything can return: drops the repeats of the keys held after their wait
/// ended or while it waits, and the text they type, also on frames that run no shortcut. Ends the
/// wait without running anything when the window lost the keyboard, or when a field, a dialog,
/// Presentation Mode, a numeric scrub, an IME composition or a busy tool has it.
pub(super) fn guard(app: &mut VectorcraftApp, ctx: &egui::Context, focused: bool) {
    let viewport = ctx.viewport_id();
    if !focused {
        // egui lets go of every key when the window loses the keyboard.
        if app.hold_keys.pending.as_ref().is_some_and(|p| p.viewport == viewport) {
            app.hold_keys.pending = None;
        }
        app.hold_keys.swallowed.retain(|s| s.viewport != viewport);
        return;
    }
    let mut held: Vec<(Key, Option<Key>)> = app.hold_keys.swallowed.iter().filter(|s| s.viewport == viewport).map(|s| (s.key, s.physical)).collect();
    held.extend(app.hold_keys.pending.as_ref().filter(|p| p.viewport == viewport).map(|p| (p.key, p.physical)));
    // A wait that ends here still drops this frame's repeats of its key, also when the frame lets
    // the key go.
    if app.hold_keys.pending.as_ref().is_some_and(|p| p.viewport == viewport) && (blocked(app, ctx) || !app.session.prefs.hold_shortcuts) {
        cancel(app, ctx);
    }
    if held.is_empty() {
        return;
    }
    drop_repeats(ctx, &held);
    // A key let go, or pressed anew, is no longer held back after this frame: a repeat that came
    // in the same frame as its release was dropped above.
    ctx.input(|i| {
        app.hold_keys.swallowed.retain(|s| {
            s.viewport != viewport
                || is_down(i, s.key, s.physical)
                    && !i.events.iter().any(|e| {
                        releases(e, s.key, s.physical)
                            || matches!(e, egui::Event::Key { key, physical_key, pressed: true, repeat: false, .. } if same_key(*key, *physical_key, s.key, s.physical))
                    })
        });
    });
}

/// Removes this frame's repeats of the keys `held`, and the text they type.
fn drop_repeats(ctx: &egui::Context, held: &[(Key, Option<Key>)]) {
    let repeat = |e: Option<&egui::Event>| {
        matches!(e, Some(egui::Event::Key { key, physical_key, pressed: true, repeat: true, .. })
            if held.iter().any(|(k, p)| same_key(*key, *physical_key, *k, *p)))
    };
    ctx.input_mut(|i| {
        let typed_by = super::typing_presses(&i.events);
        let drop: Vec<bool> = i
            .events
            .iter()
            .zip(typed_by)
            .map(|(e, by)| repeat(Some(e)) || matches!(e, egui::Event::Text(_)) && repeat(by.and_then(|k| i.events.get(k))))
            .collect();
        let mut drop = drop.into_iter();
        i.events.retain(|_| !drop.next().unwrap_or(false));
    });
}

/// Ends the wait without running anything; the key's repeats stay held back until it is let go.
fn cancel(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(p) = app.hold_keys.pending.take() else { return };
    if ctx.input(|i| is_down(i, p.key, p.physical)) {
        app.hold_keys.swallowed.push(Swallowed { key: p.key, physical: p.physical, viewport: p.viewport });
    }
}

/// Runs the result of the wait: the hold shortcut when `hold`, otherwise the tap. `down`: the key
/// is still down, so its repeats are held back until it is let go. True when the result opened a
/// dialog or the palette.
fn settle(app: &mut VectorcraftApp, ctx: &egui::Context, hold: bool, down: bool) -> bool {
    let Some(p) = app.hold_keys.pending.take() else { return false };
    if down {
        app.hold_keys.swallowed.push(Swallowed { key: p.key, physical: p.physical, viewport: p.viewport });
    }
    if hold {
        super::run_hit(app, ctx, &crate::keymap::command_of(&p.entry));
    } else {
        // The key is already up, or another key went down: a tool it chooses is chosen for good.
        super::run_single_key(app, ctx, &p.chords, None, false);
    }
    let opened = super::dialog_or_palette_open(app);
    if opened && down {
        // The key's repeats do not reach the dialog.
        let id = super::opening_key_id(ctx);
        ctx.data_mut(|m| m.insert_temp(id, p.key));
    }
    opened
}

/// The chords a press names for a hold shortcut, first match first: the key itself with its Shift
/// (Shift+1 typing `!`), then those of the text it typed, as a single-key shortcut reads them. When
/// one entry holds Shift+1 and another `!`, Shift+1 runs the first. A letter names only the chord
/// with its own Shift state: a hold on X does not catch Shift+X.
fn press_chords(text: &str, key: Key, m: Modifiers) -> Vec<craft_keymap::Chord> {
    let mut chords = super::text_chords(text, m, Some(key));
    if !text.is_empty() && text.chars().all(char::is_alphabetic) {
        chords.retain(|c| c.mods.shift == m.shift);
    }
    let shift = Modifiers { shift: m.shift, ..Modifiers::NONE };
    if let Some(c) = crate::shortcut_editor::chord_from_event(key, shift).and_then(|t| crate::keymap::vc_chord(&t)) {
        chords.retain(|t| *t != c);
        chords.insert(0, c);
    }
    chords
}

/// The chords a quick press runs, as the key runs without a hold shortcut: a shortcut on a shifted
/// punctuation key itself (Shift+.), which the shortcuts read from the key press, then those of
/// the text it typed.
fn tap_chords(text: &str, key: Key, m: Modifiers) -> Vec<craft_keymap::Chord> {
    let shift = Modifiers { shift: m.shift, ..Modifiers::NONE };
    let pressed = crate::shortcut_editor::chord_from_event(key, shift)
        .and_then(|t| crate::keymap::vc_chord(&t))
        .filter(|c| c.mods.shift && c.key.name().chars().all(|ch| !ch.is_alphanumeric()));
    let mut chords: Vec<craft_keymap::Chord> = super::text_chords(text, m, Some(key)).into_iter().filter(|c| pressed.as_ref() != Some(c)).collect();
    if let Some(c) = pressed {
        chords.insert(0, c);
    }
    chords
}

/// Does the active tool take this key itself (a digit with the perspective grid showing, a bracket
/// with the Blob Brush, E with the Extend Path tool)? Then the key keeps doing that and does not
/// wait.
fn tool_takes(app: &mut VectorcraftApp, key: Key, text: &str) -> bool {
    let view = app.view_info();
    let tool_key = match text {
        "[" | "{" => Some(ToolKey::BracketLeft),
        "]" | "}" => Some(ToolKey::BracketRight),
        _ => super::digit_of(key).map(ToolKey::Digit).or_else(|| super::letter_of(key).map(ToolKey::Letter)),
    };
    tool_key.is_some_and(|k| app.session.tool_claims_key(k, view))
}

/// Is there a press before the event at `start` that no wait took and that the frame handles: one
/// with Cmd, Ctrl or Alt, or one that typed a text of a key the active tool takes ([`tool_takes`])?
fn chord_before(app: &mut VectorcraftApp, events: &[egui::Event], typed_by: &[Option<usize>], start: usize, taken: &[usize]) -> bool {
    let mut before = events.iter().enumerate().take(start).filter(|(k, _)| !taken.contains(k));
    before.any(|(k, e)| match e {
        egui::Event::Key { key, pressed: true, modifiers: m, .. } if !super::is_modifier(*key) => {
            m.command
                || m.ctrl
                || m.alt
                || m.mac_cmd
                || typed_by
                    .iter()
                    .enumerate()
                    .any(|(t, by)| *by == Some(k) && matches!(events.get(t), Some(egui::Event::Text(text)) if tool_takes(app, *key, text)))
        }
        _ => false,
    })
}

/// Runs the single-key shortcuts of the texts typed before the event at `start` that no wait took,
/// in order, as the frame's shortcuts would run them, and takes them and their presses from the
/// frame. A text typed with Cmd, Ctrl or Alt, by a key the active tool takes, or with no shortcut,
/// is left to the frame. When one opened a dialog or the palette, the position of its first event
/// in the frame: its press, or its text where the text came first (the web build).
fn run_typed_before(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: &[egui::Event],
    typed_by: &[Option<usize>],
    start: usize,
    taken: &mut Vec<usize>,
) -> Option<usize> {
    for (k, e) in events.iter().enumerate().take(start) {
        let egui::Event::Text(text) = e else { continue };
        let by = typed_by.get(k).copied().flatten();
        if taken.contains(&k) || by.is_some_and(|b| taken.contains(&b)) {
            continue;
        }
        let (key, physical, repeat, m) = match by.and_then(|b| events.get(b)) {
            Some(egui::Event::Key { key, physical_key, repeat, modifiers, .. }) => (Some(*key), *physical_key, *repeat, *modifiers),
            _ => (None, None, false, ctx.input(|i| i.modifiers)),
        };
        if m.command || m.ctrl || m.alt || m.mac_cmd || key.is_some_and(|key| tool_takes(app, key, text)) {
            continue;
        }
        let typed = super::latin(text, physical);
        let chords = match key {
            Some(key) => tap_chords(&typed, key, m),
            None => super::text_chords(&typed, m, None),
        };
        // A key with no shortcut stays in the frame for what reads it later (Space for the canvas).
        let keymap = crate::shortcut_editor::current();
        if !chords.iter().any(|c| crate::keymap::lookup(&keymap.keymap, c).is_some()) {
            continue;
        }
        taken.push(k);
        taken.extend(by);
        if super::run_single_key(app, ctx, &chords, key, repeat) {
            return Some(by.map_or(k, |b| b.min(k)));
        }
    }
    None
}

/// Before the frame's shortcuts run: goes through the frame's events in order. A release of the
/// waiting key ends the wait (the hold when the delay has passed, otherwise the tap); a press of
/// another key, or a change of Cmd, Ctrl or Alt, ends it first, so the keys act in the order they
/// went down; and a new press of a key with a hold shortcut starts a wait, taking the press and
/// its text from the frame, with the key's repeats that follow in the frame and their text. When a
/// wait that started in this frame also ends in it, the keys typed before its key run first
/// ([`run_typed_before`]), so they act in the order they went down; after a chord with Cmd, Ctrl
/// or Alt, or a key the active tool takes, before its key, the key goes back to the frame instead
/// ([`chord_before`]). Then a wait
/// whose delay has passed with the key down runs the hold shortcut. True when a result opened a
/// dialog or the palette: the rest of the frame's keys are dropped and no other shortcut runs.
pub(super) fn intake(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    // Off (Keyboard Shortcuts › Hold Shortcuts), every key runs on its press.
    if !app.session.prefs.hold_shortcuts || blocked(app, ctx) {
        return false;
    }
    let viewport = ctx.viewport_id();
    let (events, now) = ctx.input(|i| (i.events.clone(), i.time));
    let typed_by = super::typing_presses(&events);
    let text_of = |k: usize| typed_by.iter().position(|by| *by == Some(k)).and_then(|j| events.get(j).map(|e| (j, e)));
    let mut taken: Vec<usize> = vec![];
    let mut stop_at = None;
    // The press of a wait that started in this frame, and how many events were taken before it:
    // when the wait also ends in this frame, the keys typed before it run first.
    let mut started: Option<(usize, usize)> = None;
    for (j, e) in events.iter().enumerate() {
        if let Some(p) = app.hold_keys.pending.as_ref().filter(|p| p.viewport == viewport) {
            let overdue = now - p.since >= p.delay;
            let ended = if releases(e, p.key, p.physical) {
                Some((overdue, false))
            } else {
                match e {
                    egui::Event::Key { key, physical_key, pressed: true, .. }
                        if !super::is_modifier(*key) && !same_key(*key, *physical_key, p.key, p.physical) =>
                    {
                        Some((overdue, true))
                    }
                    // A new press of the waiting key whose release was never seen.
                    egui::Event::Key { key, physical_key, pressed: true, repeat: false, .. } if same_key(*key, *physical_key, p.key, p.physical) => {
                        Some((false, false))
                    }
                    egui::Event::ModifiersChanged(m) if (m.command, m.ctrl, m.alt) != (p.mods.command, p.mods.ctrl, p.mods.alt) => {
                        Some((false, true))
                    }
                    _ => None,
                }
            };
            if let Some((hold, down)) = ended {
                let start = started.take();
                // After a chord with Cmd, Ctrl or Alt pressed before it, which the frame's shortcuts
                // run, or a key the active tool takes, the key goes back to the frame without
                // waiting, so the frame handles it as it does with Hold Shortcuts off and its tap
                // does not run before that key. Its press, its repeats and their texts stay in the
                // frame, and its later repeats reach later frames.
                if let Some((start, before)) = start
                    && chord_before(app, &events, &typed_by, start, &taken)
                {
                    app.hold_keys.pending = None;
                    taken.truncate(before);
                } else if let Some((start, _)) = start
                    && let Some(opened) = run_typed_before(app, ctx, &events, &typed_by, start, &mut taken)
                {
                    // A key typed before it that opened a dialog or the palette ends the wait
                    // without running it.
                    cancel(app, ctx);
                    stop_at = Some(opened);
                    break;
                } else if settle(app, ctx, hold, down) {
                    stop_at = Some(j);
                    break;
                }
            }
        }
        let egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers: m } = e else { continue };
        if m.command || m.ctrl || m.alt || m.mac_cmd || super::is_modifier(*key) {
            continue;
        }
        // Only a press that typed a text waits: a text alone has no key to let go.
        let Some((t, egui::Event::Text(text))) = text_of(j) else { continue };
        let typed = super::latin(text, *physical_key);
        let Some(entry) =
            press_chords(&typed, *key, *m).iter().find_map(|c| crate::shortcut_editor::hold_for(&app.ui.shortcut_holds, c)).map(str::to_string)
        else {
            continue;
        };
        let chords = tap_chords(&typed, *key, *m);
        if tool_takes(app, *key, text) {
            continue;
        }
        started = Some((j, taken.len()));
        taken.extend([j, t]);
        // The key's repeats in this frame, and the text they type, stay out of the frame too.
        for (r, later) in events.iter().enumerate().skip(j + 1) {
            if releases(later, *key, *physical_key) {
                break;
            }
            if matches!(later, egui::Event::Key { key: k, physical_key: p, pressed: true, repeat: true, .. } if same_key(*k, *p, *key, *physical_key))
            {
                taken.push(r);
                taken.extend(text_of(r).map(|(rt, _)| rt));
            }
        }
        // A wait left in another viewport ends without running: that window lost the keyboard to
        // this one. Its key's repeats stay held back there until it is let go.
        if let Some(old) = app.hold_keys.pending.take_if(|p| p.viewport != viewport) {
            app.hold_keys.swallowed.push(Swallowed { key: old.key, physical: old.physical, viewport: old.viewport });
        }
        let delay = app.session.prefs.key_hold_delay();
        app.hold_keys.pending = Some(Pending { entry, chords, key: *key, physical: *physical_key, mods: *m, since: now, delay, viewport });
    }
    // Take the waiting presses and their texts, and after a result that opened a dialog, the key
    // event that ended the wait (or the typed key that opened the dialog, when that came earlier)
    // and every later key event and text of the frame, with the text a
    // press typed before it (the web build sends a press's text first): none of them reaches the
    // dialog.
    ctx.input_mut(|i| {
        if i.events.len() != events.len() {
            return;
        }
        let mut j = 0;
        i.events.retain(|e| {
            let k = j;
            j += 1;
            let later = stop_at.is_some_and(|s| {
                k >= s && matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))
                    || matches!(e, egui::Event::Text(_)) && typed_by.get(k).copied().flatten().is_some_and(|by| by >= s)
            });
            !(taken.contains(&k) || later)
        });
    });
    if stop_at.is_some() {
        return true;
    }
    let Some(p) = app.hold_keys.pending.as_ref().filter(|p| p.viewport == viewport) else { return false };
    let deadline = p.since + p.delay;
    let down = ctx.input(|i| is_down(i, p.key, p.physical));
    if !down {
        // Let go without a release this frame saw.
        return settle(app, ctx, now >= deadline, false);
    }
    if now >= deadline {
        return settle(app, ctx, true, true);
    }
    // egui draws only on input, and a held key need not repeat: draw again at the deadline.
    ctx.request_repaint_after_secs((deadline - now) as f32);
    false
}

#[cfg(test)]
mod tests;
