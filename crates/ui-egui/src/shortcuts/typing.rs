use super::{EngineResult, Key, Mods, SurroundingUnit, ToolKey, UiRequest, VectorcraftApp, ViewInfo};

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
        let _ = app.session.tool_text(&m, app.view_info());
    }
}

/// Cmd/Ctrl chords are shortcuts (Create Outlines, Bold, …): their letter is never typed into the
/// text if a host still emits `Text` for them (#2094). Alt is left to type: Option+key on macOS
/// and AltGr (Ctrl+Alt) on Windows keyboards type characters such as é, ø and @.
pub(super) fn is_shortcut_chord(m: egui::Modifiers) -> bool {
    (m.command || m.ctrl) && !m.alt
}

enum Seq {
    Text(String),
    Preedit(String, Option<std::ops::Range<usize>>),
    Commit(String),
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
}

/// Typed text and IME events, in the order they came, to the Type tool.
pub(super) fn type_text(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    let view = app.view_info();
    // Switching apps mid-composition needs nothing: the macOS IME keeps its marked text and goes
    // on with it when the window comes back (measured with Kotoeri), as the tool does.
    let shortcut_mods = ctx.input(|i| is_shortcut_chord(i.modifiers));
    // A frame that began composing keeps its editing keys for the IME: a Backspace that empties the
    // marked text must not then delete the committed character before it.
    let was_composing = app.session.tool_composing();
    // Text, IME events and plain Backspace/Delete are taken together, in the order they arrived, so
    // a keyboard-hook IME (UniKey/OpenKey) whose tone key sends Backspace(s) and then re-injects
    // characters in one frame interleaves correctly. Taken as two passes (all text, then the keys)
    // the deletes land after the inserts and the diacritics are lost. Each handled event is removed
    // so `route_type_input` below does not process the same Backspace/Delete again.
    let seq = type_sequence(ctx);
    let viewport = ctx.viewport_id();
    let mut old_composition_retired = false;
    for e in seq {
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
                Seq::Commit(_) | Seq::DeleteSurrounding(..) | Seq::Preedit(..) => continue,
                _ => {}
            }
        }
        // A pointer/navigation/command notice has not reached its real handler yet. Text and
        // erases after it belong to the old caret, not to the destination it will establish.
        if crate::ime::deferred(app, ctx) {
            match &e {
                Seq::Text(_) | Seq::Erase(..) => continue,
                _ => {}
            }
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

/// Take this frame's typed text, IME events and plain Backspace/Delete, in the order they arrived.
fn type_sequence(ctx: &egui::Context) -> Vec<Seq> {
    let mut seq = vec![];
    ctx.input_mut(|i| {
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
                egui::Event::Text(t) => seq.push(Seq::Text(t)),
                egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                    seq.push(Seq::Preedit(text, active_range_chars));
                }
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
                // modified forms stay with `route_type_input`.
                egui::Event::Key { key: key @ (Key::Backspace | Key::Delete), pressed: true, modifiers: m, .. }
                    if !m.command && !m.alt && !m.ctrl =>
                {
                    let tk = if key == Key::Backspace { ToolKey::Backspace } else { ToolKey::Delete };
                    seq.push(Seq::Erase(tk, Mods { shift: m.shift, ..Mods::default() }));
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
                e @ egui::Event::Key { key, pressed: true, modifiers, .. }
                    if matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End)
                        || key == Key::A && modifiers.command =>
                {
                    seq.push(Seq::InterruptNavigation);
                    kept.push(e);
                }
                e @ egui::Event::Key { pressed: true, modifiers, .. } if is_shortcut_chord(modifiers) => {
                    seq.push(Seq::InterruptCommand);
                    kept.push(e);
                }
                e @ (egui::Event::Cut | egui::Event::Paste(_)) => {
                    seq.push(Seq::InterruptCommand);
                    kept.push(e);
                }
                other => kept.push(other),
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
        Seq::ResetCompleted(_) => Ok(vec![]),
        // Keys the IME passed through come as text only when nothing is marked. This is an
        // application edit (also a hook IME's reinjected characters): `Other` dominates.
        Seq::Text(t) if !app.session.tool_composing() && !shortcut_mods => {
            let target_before = crate::ime::target(&mut app.session, view);
            let before = crate::ime::stamp(&mut app.session, view);
            let r = app.session.tool_text(&t, view);
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
                    Some(m) => app.session.tool_text_is_verbatim(m, view),
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
            let verbatim = app.session.tool_ime_session_matches(view) && app.session.tool_text_is_verbatim(&t, view);
            let r = app.session.tool_text(&t, view);
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
            // Mirror `route_type_input`/`text_key`: surface an erase error in the status bar
            // instead of discarding it, and report the successful edit as application-origin.
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
    }
}
