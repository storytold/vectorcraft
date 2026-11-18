use super::{EngineResult, Key, Mods, SurroundingUnit, ToolKey, UiRequest, VectorcraftApp, ViewInfo, chord_runs_a_command, typing_presses};
use crate::panels::character::TextInput;

/// Drop each key press made with Ctrl and Alt held that is followed by the text it typed. Firefox
/// on Windows reports AltGr as Ctrl and Alt held, so AltGr+L on a Polish layout arrives as a
/// Ctrl+Alt+L press and then the text "ł", which the Type tool types. Without the press no command
/// chord runs for it: neither Cmd+Alt+L nor Cmd+L, which egui matches with the extra Alt held.
pub(super) fn drop_typing_presses(i: &mut egui::InputState) {
    let next = i.events.iter().skip(1).map(Some).chain([None]);
    let typed: Vec<bool> = i
        .events
        .iter()
        .zip(next)
        .map(|(e, next)| {
            matches!(e, egui::Event::Key { pressed: true, modifiers, .. } if modifiers.ctrl && modifiers.alt)
                && matches!(next, Some(egui::Event::Text(t)) if !t.is_empty())
        })
        .collect();
    let mut typed = typed.into_iter();
    i.events.retain(|_| !typed.next().unwrap_or(false));
}

/// End an IME composition outside the IME (a click away, the IME going away): the
/// marked text stays as typed, and the system IME is told to drop it.
pub(crate) fn keep_marked_text(app: &mut VectorcraftApp) {
    let Some(m) = app.ime_marked.take() else { return };
    app.ime_discard = true;
    if app.session.tool_composing() {
        // Errors are already reported by the engine's interaction.
        let _ = app.session.tool_typed_text(&m, app.view_info());
    }
}

/// Cmd/Ctrl chords are shortcuts (Create Outlines, Bold, …): their letter is never typed into the
/// text if a host still emits `Text` for them (#2094). Alt is left to type: Option+key on macOS
/// and AltGr (Ctrl+Alt) on Windows keyboards type characters such as é, ø and @.
pub(super) fn is_shortcut_chord(m: egui::Modifiers) -> bool {
    (m.command || m.ctrl) && !m.alt
}

/// The character an Alt chord types while the Type tool edits text on Windows and Linux, with the
/// characters its key types without Alt: Alt+8 •, Alt+G ©, Alt+; …, Alt+7 ¶, Alt+6 §, Alt+2 ™,
/// Alt+R ®, Alt+- en dash, Alt+Shift+- em dash, Alt+[ “, Alt+Shift+[ ”, Alt+] ‘ and Alt+Shift+] ’.
/// A chord with Ctrl or Cmd held, AltGr included, types none of them.
fn alt_chord_character(key: Key, m: egui::Modifiers) -> Option<(char, &'static str)> {
    if !m.alt || m.ctrl || m.command || m.mac_cmd {
        return None;
    }
    Some(match (key, m.shift) {
        (Key::Num8, false) => ('\u{2022}', "8"),
        (Key::G, false) => ('\u{00A9}', "gG"),
        (Key::Semicolon, false) => ('\u{2026}', ";"),
        (Key::Num7, false) => ('\u{00B6}', "7"),
        (Key::Num6, false) => ('\u{00A7}', "6"),
        (Key::Num2, false) => ('\u{2122}', "2"),
        (Key::R, false) => ('\u{00AE}', "rR"),
        (Key::Minus, false) => ('\u{2013}', "-"),
        (Key::Minus, true) => ('\u{2014}', "_"),
        (Key::OpenBracket, false) => ('\u{201C}', "["),
        (Key::OpenBracket | Key::OpenCurlyBracket, true) => ('\u{201D}', "{"),
        (Key::CloseBracket, false) => ('\u{2018}', "]"),
        (Key::CloseBracket | Key::CloseCurlyBracket, true) => ('\u{2019}', "}"),
        _ => return None,
    })
}

/// Turn each Alt chord of [`alt_chord_character`] into the text of its character. Windows and Linux
/// send the key's own character with the press (Alt+G is a G press and the text "g"); that text is
/// replaced. Any other text that the press typed is what the keyboard layout typed, and it is typed
/// as it came. A press that typed no text gets its character added after it. A text's press is the
/// one [`typing_presses`] pairs it with: the press before it, or in the web build the press after it.
/// An input method's committed or marked text belongs to the last press before it, past modifier
/// changes, key releases, empty pre-edits and deletions around the caret; that press is left as it came, since the input method's text is what the key typed.
fn type_alt_chords(events: &mut Vec<egui::Event>) {
    let chord = |e: &egui::Event| match e {
        egui::Event::Key { key, pressed: true, modifiers, .. } => alt_chord_character(*key, *modifiers),
        _ => None,
    };
    if !events.iter().any(|e| chord(e).is_some()) {
        return;
    }
    let ime_text = |e: &egui::Event| match e {
        egui::Event::Ime(egui::ImeEvent::Commit(t) | egui::ImeEvent::Preedit { text: t, .. }) => !t.is_empty(),
        _ => false,
    };
    // Events between a press and the input method's text it typed. Windows sends an empty pre-edit
    // before each commit, and an input method can delete the text around the caret first.
    let between = |e: &egui::Event| match e {
        egui::Event::ModifiersChanged(_) | egui::Event::Key { pressed: false, .. } => true,
        egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => text.is_empty(),
        egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. } | egui::ImeEvent::DeleteSurroundingBytes { .. }) => true,
        _ => false,
    };
    // The press an input method's text at `j` belongs to.
    let ime_press = |j: usize| {
        events
            .iter()
            .enumerate()
            .take(j)
            .rev()
            .find(|(_, e)| !between(e))
            .filter(|(_, e)| matches!(e, egui::Event::Key { pressed: true, .. }))
            .map(|(k, _)| k)
    };
    let pressed_by = typing_presses(events);
    let mut typed = vec![false; events.len()];
    let ime_presses = events.iter().enumerate().filter(|(_, e)| ime_text(e)).map(|(j, _)| ime_press(j));
    for k in pressed_by.iter().copied().chain(ime_presses).flatten() {
        if let Some(t) = typed.get_mut(k) {
            *t = true;
        }
    }
    let chords: Vec<_> = events.iter().map(chord).collect();
    let mut out = Vec::with_capacity(events.len() + 1);
    for (j, e) in std::mem::take(events).into_iter().enumerate() {
        match e {
            egui::Event::Text(t) => {
                let press = pressed_by.get(j).copied().flatten().and_then(|k| chords.get(k).copied().flatten());
                match press {
                    Some((c, own)) if t.is_empty() || (t.chars().count() == 1 && own.contains(t.as_str())) => {
                        out.push(egui::Event::Text(c.to_string()));
                    }
                    _ => out.push(egui::Event::Text(t)),
                }
            }
            e => {
                out.push(e);
                if let Some((c, _)) = chords.get(j).copied().flatten()
                    && !typed.get(j).copied().unwrap_or(false)
                {
                    out.push(egui::Event::Text(c.to_string()));
                }
            }
        }
    }
    *events = out;
}

enum Seq {
    Text(String),
    Preedit(String, Option<std::ops::Range<usize>>),
    Commit(String),
    /// A space the IME commits during a Zoom tool drag on the canvas, where Space moves the marquee.
    MarqueeSpace,
    Erase(ToolKey, Mods),
    /// An input method replaces text around the composition (upstream's character counts or
    /// the fork's exact UTF-8 byte counts), then commits.
    DeleteSurrounding(usize, usize, SurroundingUnit),
    // The actual pointer/key still belongs to its existing handler. Its position in this
    // sequence must nevertheless interrupt an old composition before later terminal input.
    InterruptPointer,
    InterruptNavigation,
    InterruptCommand,
    ResetCompleted(egui::ImeResetId),
    /// Enter, which types a line break. Cmd+Enter (Ctrl+Enter on Windows and Linux) ends editing.
    Enter(Mods),
    /// An editing key with modifiers, an arrow, Home, End, Tab or a clipboard event.
    Edit(TextInput),
}

/// Typed text, IME events, editing keys, Enter and clipboard events, in the order they came, to the
/// Type tool. Escape is handled before them, in [`handle`].
pub(super) fn type_text(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    let view = app.view_info();
    // Switching apps mid-composition needs nothing: the macOS IME keeps its marked text and goes
    // on with it when the window comes back (measured with Kotoeri), as the tool does.
    let shortcut_mods = ctx.input(|i| is_shortcut_chord(i.modifiers));
    // A frame that began composing keeps its editing keys for the IME: a Backspace that empties the
    // marked text must not then delete the committed character before it.
    let was_composing = app.session.tool_composing();
    crate::panels::character::keep_ctx(ctx);
    // Text, IME events and plain Backspace/Delete are taken together, in the order they arrived, so
    // a keyboard-hook IME (UniKey/OpenKey) whose tone key sends Backspace(s) and then re-injects
    // characters in one frame interleaves correctly. Taken as two passes (all text, then the keys)
    // the deletes land after the inserts and the diacritics are lost. Enter, the other editing keys
    // and the clipboard events join them: ← or a paste lands between the texts around it, and each
    // Enter types its own line break (or, as Cmd+Enter, ends editing there). Each handled event is
    // removed from the input.
    let seq = type_sequence(ctx);
    let viewport = ctx.viewport_id();
    let mut old_composition_retired = false;
    // Set once the IME commits text in this frame.
    let mut committed = false;
    for e in seq {
        // With text marked, the IME's space commits that text as usual.
        let e = match e {
            Seq::MarqueeSpace if app.session.tool_composing() || app.ime_marked.is_some() => Seq::Commit(" ".into()),
            e => e,
        };
        // Before the UI, a changed composition or guard is settled `pending`: a token bound here
        // could outlive a route this pass invalidates later (a palette/dialog/Home) and attach to
        // the stock descriptor. The post-UI finalizer binds the confirmed replacement.
        crate::ime::sync_incoming_before_ui(app, ctx);
        if let Seq::ResetCompleted(request) = &e {
            old_composition_retired |= crate::ime::complete(app, ctx, *request);
            continue;
        }
        if matches!(app.ime_compositions.get(&viewport), Some(crate::ime::IncomingComposition::Stale)) {
            match &e {
                // Accepted old payloads precede the fence even when their terminal arrives.
                // None of these events proves the local reset has reached its boundary.
                Seq::Commit(_) | Seq::MarqueeSpace | Seq::DeleteSurrounding(..) | Seq::Preedit(..) => continue,
                _ => {}
            }
        }
        // Otherwise the IME's input ends there, and the space is not typed.
        if matches!(e, Seq::MarqueeSpace) {
            committed = true;
            app.ime_compositions.remove(&viewport);
            continue;
        }
        // A pointer/navigation/command notice has not reached its real handler yet. Text and
        // erases after it belong to the old caret, not to the destination it will establish.
        if crate::ime::deferred(app, ctx) {
            match &e {
                // A cut or paste must not change the text at the old caret either.
                Seq::Text(_) | Seq::Erase(..) | Seq::Edit(TextInput::Cut | TextInput::Paste(_)) => continue,
                _ => {}
            }
        }
        // Editing keys and clipboard events belong to the IME while it composes, and in a frame
        // that began composing until the IME commits or the composition is retired.
        if matches!(&e, Seq::Edit(_)) && (app.session.tool_composing() || (was_composing && !committed && !old_composition_retired)) {
            continue;
        }
        if let Seq::Commit(t) = &e {
            committed |= !t.is_empty() && t != "\n" && t != "\r";
        }
        let preedit_empty = match &e {
            Seq::Preedit(text, _) => Some(text.is_empty()),
            _ => None,
        };
        let terminal_commit = matches!(&e, Seq::Commit(_));
        let from_ime = matches!(&e, Seq::Preedit(..) | Seq::Commit(_) | Seq::DeleteSurrounding(..));
        // Errors are already reported by the engine's interaction (a failed preview keeps it).
        let result = apply_type_event(app, ctx, view, e, shortcut_mods, was_composing, &mut old_composition_retired);
        if result.is_ok() && from_ime {
            if terminal_commit {
                app.ime_compositions.remove(&viewport);
            } else {
                crate::ime::remember_incoming(app, ctx, preedit_empty);
            }
        }
    }
    old_composition_retired
}

/// Take this frame's typed text, IME events, editing keys, Enter and clipboard events, in the order
/// they arrived. A space typed during the canvas's Zoom tool drag is not typed.
fn type_sequence(ctx: &egui::Context) -> Vec<Seq> {
    let mut seq = vec![];
    let mut edits = vec![];
    let zooming = crate::canvas::dragging_zoom(ctx);
    ctx.input_mut(|i| {
        // On a Mac, Option types these characters itself.
        if !crate::keymap::platform().is_mac() {
            type_alt_chords(&mut i.events);
        }
        // Move the events out instead of cloning their strings: a keyboard hook that reinjects a
        // batch keeps the batch in `seq` only, not in both the input list and a second copy.
        let mut kept = Vec::with_capacity(i.events.len());
        // InputState's held flag describes the end of the batch. Recover its initial state from
        // the first transition, so a move before a later press cannot cancel an earlier commit.
        let mut primary_down = i
            .events
            .iter()
            .find_map(|e| match e {
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => Some(!pressed),
                _ => None,
            })
            .unwrap_or_else(|| i.pointer.primary_down());
        for e in std::mem::take(&mut i.events) {
            match e {
                // The zoom drag lasts until the primary button goes up.
                egui::Event::Text(t) if zooming && primary_down && t == " " => {}
                egui::Event::Text(t) => seq.push(Seq::Text(t)),
                egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                    seq.push(Seq::Preedit(text, active_range_chars));
                }
                egui::Event::Ime(egui::ImeEvent::Commit(t)) if zooming && primary_down && t == " " => seq.push(Seq::MarqueeSpace),
                egui::Event::Ime(egui::ImeEvent::Commit(t)) => seq.push(Seq::Commit(t)),
                egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars, after_chars }) => {
                    seq.push(Seq::DeleteSurrounding(before_chars, after_chars, SurroundingUnit::Chars));
                }
                egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes }) => {
                    seq.push(Seq::DeleteSurrounding(before_bytes, after_bytes, SurroundingUnit::Bytes));
                }
                egui::Event::ImeResetCompleted { request_id } => seq.push(Seq::ResetCompleted(request_id)),
                // X11/Wayland PRIMARY middle-click: text at the pointer, inserted as typed text (never
                // routed as a key or shortcut).
                egui::Event::MiddleClickPaste { text, .. } => seq.push(Seq::Text(text)),
                // Only unmodified Backspace/Delete interleave with injected text; word-delete and other
                // modified forms are taken with the other editing keys below.
                egui::Event::Key { key: key @ (Key::Backspace | Key::Delete), pressed: true, modifiers: m, .. }
                    if !m.command && !m.alt && !m.ctrl =>
                {
                    let tk = if key == Key::Backspace { ToolKey::Backspace } else { ToolKey::Delete };
                    seq.push(Seq::Erase(tk, Mods { shift: m.shift, ..Mods::default() }));
                }
                // A command bound to the chord (Cmd+Enter) runs instead of ending editing, in the
                // shortcuts after this.
                egui::Event::Key { key: Key::Enter, pressed, modifiers: m, .. } if !chord_runs_a_command(Key::Enter, m, true) => {
                    if pressed {
                        seq.push(Seq::Enter(crate::canvas::mods(m, false)));
                    }
                }
                e @ egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => {
                    primary_down = pressed;
                    if pressed {
                        seq.push(Seq::InterruptPointer);
                    }
                    kept.push(e);
                }
                e @ egui::Event::PointerMoved(_) => {
                    if primary_down {
                        seq.push(Seq::InterruptPointer);
                    }
                    kept.push(e);
                }
                e => {
                    let interrupt = match &e {
                        egui::Event::Key { key, pressed: true, modifiers, .. }
                            if matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End)
                                || *key == Key::A && modifiers.command =>
                        {
                            Some(Seq::InterruptNavigation)
                        }
                        egui::Event::Key { pressed: true, modifiers, .. } if is_shortcut_chord(*modifiers) => Some(Seq::InterruptCommand),
                        egui::Event::Cut | egui::Event::Paste(_) => Some(Seq::InterruptCommand),
                        _ => None,
                    };
                    seq.extend(interrupt);
                    // The other editing keys and the clipboard events apply in this order too.
                    if crate::panels::character::type_input(&e, &mut edits) {
                        kept.push(e);
                    }
                    seq.extend(edits.drain(..).map(Seq::Edit));
                }
            }
        }
        i.events = kept;
    });
    seq
}

/// Apply one sequence item to the active tool; the caller filters stale/deferred events and
/// turns a successful IME mutation into its receipt.
fn apply_type_event(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    view: ViewInfo,
    e: Seq,
    shortcut_mods: bool,
    was_composing: bool,
    old_composition_retired: &mut bool,
) -> EngineResult<Vec<UiRequest>> {
    match e {
        Seq::InterruptPointer => {
            *old_composition_retired |= crate::ime::defer_interrupt(app, ctx);
            Ok(vec![])
        }
        Seq::InterruptNavigation => {
            // Active composition owns its navigation keys. Once cleared, application caret
            // navigation interrupts its still-pending terminal event, even in this batch.
            if !app.session.tool_composing() {
                *old_composition_retired |= crate::ime::defer_interrupt(app, ctx);
            }
            Ok(vec![])
        }
        Seq::InterruptCommand => {
            if crate::ime::blocked(app, ctx) {
                *old_composition_retired |= crate::ime::defer_interrupt(app, ctx);
            }
            Ok(vec![])
        }
        Seq::ResetCompleted(_) | Seq::MarqueeSpace => Ok(vec![]),
        // Keys the IME passed through come as text only when nothing is marked. This is an
        // application edit (also a hook IME's reinjected characters): `Other` dominates.
        Seq::Text(t) if !app.session.tool_composing() && !shortcut_mods => {
            let target_before = crate::ime::target(&mut app.session, view);
            let before = crate::ime::stamp(&mut app.session, view);
            let r = app.session.tool_typed_text(&t, view);
            if r.is_ok() {
                crate::ime::note_application_edit(app, ctx, view, target_before, before);
            }
            r
        }
        Seq::Text(_) => Ok(vec![]),
        // A pre-edit is not committed text: it changes neither the outgoing snapshot (the
        // engine query removes it) nor the cause.
        Seq::Preedit(text, active_range_chars) => {
            app.ime_marked = Some(text.clone()).filter(|t| !t.is_empty());
            app.session.tool_preedit(&text, active_range_chars, view)
        }
        // A bare line break confirms the composition (Enter): the marked text, not a newline.
        // It is an accepted IME commit of that marked text when the tool inserts the marked
        // text unchanged; a commit the tool rewrites (a quoted or filtered marked text) is an
        // application edit instead.
        Seq::Commit(t) if t == "\n" || t == "\r" => {
            let target_before = crate::ime::target(&mut app.session, view);
            let before = crate::ime::stamp(&mut app.session, view);
            let composing = app.session.tool_composing();
            let verbatim = app.session.tool_ime_session_matches(view)
                && match app.ime_marked.as_deref() {
                    Some(m) => app.session.tool_typed_text_is_verbatim(m, view),
                    None => true,
                };
            keep_marked_text(app);
            if composing {
                if verbatim {
                    crate::ime::note_ime_apply(app, ctx, view, target_before, before);
                } else {
                    crate::ime::note_application_edit(app, ctx, view, target_before, before);
                }
            }
            Ok(vec![])
        }
        // An accepted IME commit: stamp the bounded committed context immediately before and
        // after, so only a real mutation (not a no-op, rejection or failure) earns the receipt.
        // A commit the tool rewrites (Typographer's Quotes, a filtered control character) is an
        // application edit and must never earn `InputMethod`.
        Seq::Commit(t) => {
            app.ime_marked = None;
            let target_before = crate::ime::target(&mut app.session, view);
            let before = crate::ime::stamp(&mut app.session, view);
            // A document edit since the composition means this commit starts a fresh typing
            // session: that reset is application-side, so it cannot earn `InputMethod`.
            let verbatim = app.session.tool_ime_session_matches(view) && app.session.tool_typed_text_is_verbatim(&t, view);
            let r = app.session.tool_typed_text(&t, view);
            if r.is_ok() {
                if verbatim {
                    crate::ime::note_ime_apply(app, ctx, view, target_before, before);
                } else {
                    crate::ime::note_application_edit(app, ctx, view, target_before, before);
                }
            }
            r
        }
        // While composing (or in a frame that began composing), editing keys belong to the IME.
        Seq::Erase(tk, m) if !was_composing && !app.session.tool_composing() => {
            // Mirror `text_key`: surface an erase error in the status bar instead of discarding it,
            // and report the successful edit as application-origin.
            let target_before = crate::ime::target(&mut app.session, view);
            let before = crate::ime::stamp(&mut app.session, view);
            let r = app.session.tool_key(tk, m, view);
            match &r {
                Err(e) => app.status(e.to_string()),
                Ok(_) => crate::ime::note_application_edit(app, ctx, view, target_before, before),
            }
            r
        }
        Seq::Erase(..) => Ok(vec![]),
        // Deleting around the composition starts the same typing session as the commit that
        // follows, so the replacement is one undo step; an error reaches the status bar. The
        // receipt covers only this accepted deletion.
        Seq::DeleteSurrounding(before, after, unit) => {
            let target_before = crate::ime::target(&mut app.session, view);
            let stamp_before = crate::ime::stamp(&mut app.session, view);
            let r = app.session.tool_ime_delete_surrounding(before, after, unit, view);
            match &r {
                Err(e) => app.status(e.to_string()),
                Ok(_) => crate::ime::note_ime_apply(app, ctx, view, target_before, stamp_before),
            }
            r
        }
        // An Enter that confirms the composition types no line break.
        Seq::Enter(_) if was_composing || app.session.tool_composing() => Ok(vec![]),
        Seq::Enter(m) => {
            let target_before = crate::ime::target(&mut app.session, view);
            let before = crate::ime::stamp(&mut app.session, view);
            let r = app.session.tool_key(ToolKey::Enter, m, view);
            if r.is_ok() {
                crate::ime::note_application_edit(app, ctx, view, target_before, before);
            }
            crate::canvas::apply_requests(app, r);
            Ok(vec![])
        }
        // `text_key` and `text_paste` report the edit as application-origin.
        Seq::Edit(t) => {
            crate::panels::character::apply_type_input(app, t);
            Ok(vec![])
        }
    }
}
