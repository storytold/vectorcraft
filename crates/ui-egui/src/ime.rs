//! VectorCraft's outgoing IME producer: the Type tool's committed logical text, its stable owner
//! and context generation, and the explicit `InputMethod`/`Other` cause.
//!
//! The shared [`egui::ime_context`] helper does the lifecycle bookkeeping. This module is the thin
//! app adapter the helper asks for: it names the edit target from the engine's document uid and
//! TypeTool edit session, copies only a bounded committed window (never the whole document), and
//! turns accepted IME mutations into receipts around the exact engine call that applied them.

use egui::ime_context::{CommittedText, ImeContextUpdate, ImeMutationStamp, ImeObservation, ImeTarget};
use egui::output::{IMEOutput, MAX_IME_SURROUNDING_TEXT_BYTES};
use egui::{Context, Id};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_tools::ImeSurrounding;

use crate::VectorcraftApp;

/// The same byte limit the fork's [`egui::output::IMESurroundingText::from_text`] enforces. Asking
/// the engine for exactly this window avoids cloning a large document just to discard most of it.
pub(crate) const SURROUNDING_LIMIT: usize = MAX_IME_SURROUNDING_TEXT_BYTES;

/// Incoming selection provenance, never part of the immutable native target identity. Use the
/// tool's actual offsets: the outgoing logical snapshot collapses offsets inside marked text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IncomingSelection {
    target: ImeTarget,
    anchor: u64,
    caret: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum IncomingComposition {
    Active(IncomingSelection),
    /// An empty preedit cleared the preview; its separate terminal mutation is still pending.
    TerminalPending(IncomingSelection),
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OutputContext {
    owner: Id,
    generation: u64,
    purpose: egui::IMEPurpose,
}

impl From<&IMEOutput> for OutputContext {
    /// Copy the output's owner, context generation and purpose into the comparable publication
    /// identity.
    fn from(output: &IMEOutput) -> Self {
        Self { owner: output.owner, generation: output.context_generation, purpose: output.purpose }
    }
}

/// A guard can remain blocked without an acceptable token (suspension/allocation failure).
/// The destination is separate from the old composition whose preview was settled.
#[derive(Clone, Debug)]
pub(crate) struct ResetGuard {
    request: Option<egui::ImeResetId>,
    context: u64,
    destination: Option<IncomingSelection>,
    committed: Option<ImeMutationStamp>,
    output: Option<OutputContext>,
    suspended: bool,
    deferred: bool,
    /// Settled before the UI ran: this pass's token is retired and only the post-UI finalizer,
    /// after the descriptor is confirmed, may bind the replacement.
    pending: bool,
}

/// Temp-memory key marking whether this viewport is currently inside a UI pass.
fn mode_id(viewport: egui::ViewportId) -> Id {
    Id::unique(("vectorcraft.ime.in-ui-pass", viewport))
}
/// Temp-memory key for the pass number and descriptor identity observed during this viewport's UI
/// pass.
fn output_id(viewport: egui::ViewportId) -> Id {
    Id::unique(("vectorcraft.ime.pass-output", viewport))
}

/// Install before the host's first pass. Callbacks borrow their own Ui's Context, never capture it.
pub(crate) fn install_hooks(ctx: &Context) {
    let installed = Id::unique("vectorcraft.ime.mode-hooks-installed");
    let first = ctx.data_mut(|data| {
        if data.get_temp::<bool>(installed).unwrap_or(false) {
            false
        } else {
            data.insert_temp(installed, true);
            true
        }
    });
    if !first {
        return;
    }
    ctx.on_begin_pass(
        "VectorCraft IME pass begin",
        std::sync::Arc::new(|ui| {
            let ctx = ui.ctx();
            let id = mode_id(ctx.viewport_id());
            ctx.data_mut(|data| data.insert_temp(id, true));
        }),
    );
    ctx.on_end_pass(
        "VectorCraft IME pass end",
        std::sync::Arc::new(|ui| {
            let ctx = ui.ctx();
            let id = mode_id(ctx.viewport_id());
            ctx.data_mut(|data| data.insert_temp(id, false));
        }),
    );
}

/// The base eligibility every custom-descriptor consumer shares: a hidden viewport, the Home
/// screen, a modal dialog, the palette, a file sheet, an unfocused window, another egui keyboard
/// owner or no Type-tool text edit all mean this pass publishes a stock descriptor or none. One
/// helper so the pre-UI route test, the post-UI finalizer and the end-of-pass observation cannot
/// drift apart; Home in particular is an ineligible route even though the Type tool keeps its
/// target across it.
fn pass_eligible(app: &VectorcraftApp, ctx: &Context) -> bool {
    !app.viewport_hidden(ctx)
        && !crate::menus::home_showing(app)
        && app.ui.dialog.is_none()
        && !app.ui.palette_open
        && !app.file_sheet_open()
        && ctx.input(|input| input.focused)
        && !ctx.egui_wants_keyboard_input()
        && app.session.tool_wants_text()
}

/// The one route test every allocation goes through: a request exists only when this pass can
/// publish the custom descriptor that consumes it. egui keeps the latest request in `Memory` until
/// such a descriptor arrives, so allocating on a hidden, Home, stock or geometry-less route would
/// attach a foreign token to whatever stock field publishes next. An ineligible route retires the
/// local guard instead; a later eligible activation allocates a fresh request.
fn route_live(app: &mut VectorcraftApp, ctx: &Context) -> bool {
    if !pass_eligible(app, ctx) {
        return false;
    }
    let view = app.view_info();
    target(&mut app.session, view).is_some() && app.session.tool_ime_caret(view).is_some()
}

/// Allocate a reset request on an eligible custom-editor route and replace the guard with the
/// confirmed destination, commit stamp and prior output identity; suspend instead when the route
/// cannot publish.
fn request_guard(app: &mut VectorcraftApp, ctx: &Context, deferred: bool) -> bool {
    if !route_live(app, ctx) {
        suspend(app, ctx);
        return false;
    }
    let viewport = ctx.viewport_id();
    let destination = incoming_selection(app);
    let view = app.view_info();
    let committed = stamp(&mut app.session, view);
    let output = app.ime_resets.get(&viewport).and_then(|guard| guard.output);
    let request = ctx.request_ime_reset(viewport);
    #[cfg(test)]
    note_test_request(ctx);
    app.ime_resets.insert(
        viewport,
        ResetGuard {
            request,
            context: crate::graphics::context_tag(ctx),
            destination,
            committed,
            output,
            suspended: false,
            deferred,
            pending: false,
        },
    );
    app.ime_compositions.insert(viewport, IncomingComposition::Stale);
    true
}

/// Temp-memory key for the test-only count of IME reset requests since the last take.
#[cfg(test)]
fn request_count_id() -> Id {
    Id::unique("vectorcraft.ime.test-request-count")
}

/// Increment the test-only request counter for every reset allocation, including intermediates
/// superseded before publication.
#[cfg(test)]
fn note_test_request(ctx: &Context) {
    ctx.data_mut(|data| {
        let count = data.get_temp::<u64>(request_count_id()).unwrap_or(0);
        data.insert_temp(request_count_id(), count.saturating_add(1));
    });
}

/// Tests read how many guards were requested since the last call. This makes an intermediate
/// pre-settlement request visible even when the final token is stable.
#[cfg(test)]
pub(crate) fn take_request_count(ctx: &Context) -> u64 {
    ctx.data_mut(|data| data.remove_temp::<u64>(request_count_id()).unwrap_or(0))
}

/// True only while a pointer/navigation/command notice is waiting for its real later handler.
/// Destination-dependent raw input must not mutate the old caret until that handler ran.
pub(crate) fn deferred(app: &VectorcraftApp, ctx: &Context) -> bool {
    app.ime_resets.get(&ctx.viewport_id()).is_some_and(|guard| guard.deferred)
}

/// Retire a live guard when this pass cannot publish the custom editor at all (the route lost
/// its target or caret geometry, or the viewport is hidden). This runs before any reset marker is
/// considered, so a marker for the old request cannot complete a route that no longer exists.
pub(crate) fn suspend_if_route_lost(app: &mut VectorcraftApp, ctx: &Context) {
    let viewport = ctx.viewport_id();
    if app.ime_resets.get(&viewport).is_none_or(|guard| guard.suspended) {
        return;
    }
    if !route_live(app, ctx) {
        suspend(app, ctx);
    }
}

/// Whether this viewport has a guard, including a suspended guard that currently owns no request.
pub(crate) fn blocked(app: &VectorcraftApp, ctx: &Context) -> bool {
    app.ime_resets.contains_key(&ctx.viewport_id())
}

/// Return the reset request awaiting native completion for this viewport; test helper.
#[cfg(test)]
pub(crate) fn pending_request(app: &VectorcraftApp, ctx: &Context) -> Option<egui::ImeResetId> {
    app.ime_resets.get(&ctx.viewport_id()).and_then(|guard| guard.request)
}

/// A notice precedes a later pointer/navigation/command handler. Retire the old request and keep
/// its suffix closed until that handler has established the real destination. No request is
/// allocated here: the final handler may still invalidate the route (a click that ends editing, a
/// modal, a stock field), and a token allocated now would survive in egui's memory for the next
/// descriptor. The eligible finalizer allocates for the confirmed destination instead.
pub(crate) fn defer_interrupt(app: &mut VectorcraftApp, ctx: &Context) -> bool {
    let live = route_live(app, ctx);
    let was_blocked = blocked(app, ctx);
    // Settle with the old suffix still fenced off, but never allocate: the raw suffix after this
    // notice belongs to the old caret whether or not the final route is still a custom editor.
    let settled = interrupt_incoming_impl(app, ctx, Fence::Deferred);
    if settled || was_blocked {
        // The finalizer owns the token, but this pass must keep a guard so the old marker cannot
        // complete and so a confirmed final descriptor requests its replacement. A new guard here
        // has no request: `needs_replacement` sees `deferred` and allocates later.
        place_deferred_guard(app, ctx);
    }
    if !live {
        // No custom editor owns this pass. Keep the guard retired until an eligible activation,
        // but leave its stale latch in place so no old commit/preedit is accepted meanwhile.
        if let Some(guard) = app.ime_resets.get_mut(&ctx.viewport_id()) {
            guard.request = None;
            guard.deferred = true;
            guard.suspended = true;
        }
    }
    settled || blocked(app, ctx)
}

/// Retire the current request but keep the guard awaiting the finalizer's confirmed destination.
fn place_deferred_guard(app: &mut VectorcraftApp, ctx: &Context) {
    let viewport = ctx.viewport_id();
    let destination = incoming_selection(app);
    let view = app.view_info();
    let committed = stamp(&mut app.session, view);
    match app.ime_resets.get_mut(&viewport) {
        Some(guard) => {
            guard.request = None;
            guard.deferred = true;
            guard.pending = false;
        }
        None => {
            let output = None;
            app.ime_resets.insert(
                viewport,
                ResetGuard {
                    request: None,
                    context: crate::graphics::context_tag(ctx),
                    destination,
                    committed,
                    output,
                    suspended: false,
                    deferred: true,
                    pending: false,
                },
            );
        }
    }
    app.ime_compositions.insert(viewport, IncomingComposition::Stale);
}

/// Pre-UI settlement. The old token is retired and the ordered latch is kept, but nothing is
/// allocated: the pass's final descriptor does not exist yet, so a token bound here could outlive
/// the route (a later palette/dialog/Home) in egui's memory. The post-UI finalizer, which confirms
/// the descriptor, binds the replacement for `pending`.
fn mark_pending(app: &mut VectorcraftApp, ctx: &Context) {
    let viewport = ctx.viewport_id();
    let destination = incoming_selection(app);
    let view = app.view_info();
    let committed = stamp(&mut app.session, view);
    match app.ime_resets.get_mut(&viewport) {
        Some(guard) => {
            guard.request = None;
            guard.deferred = false;
            guard.pending = true;
        }
        None => {
            app.ime_resets.insert(
                viewport,
                ResetGuard {
                    request: None,
                    context: crate::graphics::context_tag(ctx),
                    destination,
                    committed,
                    output: None,
                    suspended: false,
                    deferred: false,
                    pending: true,
                },
            );
        }
    }
    app.ime_compositions.insert(viewport, IncomingComposition::Stale);
}

/// Retire the guard's request and clear its deferred and pending flags while keeping the guard
/// blocked until an eligible route allocates a replacement.
fn suspend(app: &mut VectorcraftApp, ctx: &Context) {
    if let Some(guard) = app.ime_resets.get_mut(&ctx.viewport_id()) {
        guard.request = None;
        guard.suspended = true;
        guard.deferred = false;
        guard.pending = false;
    }
}

/// A close request ends this viewport's IME route without publishing a custom descriptor. Settle
/// any preview without a token (a later stock field must not inherit one) and retire the guard so
/// a cancelled close rearms on a fresh generation.
pub(crate) fn close_viewport(app: &mut VectorcraftApp, ctx: &Context) {
    let viewport = ctx.viewport_id();
    interrupt_incoming_impl(app, ctx, Fence::Unbound);
    app.ime_compositions.remove(&viewport);
    suspend(app, ctx);
}

/// Only this ordered raw control can lift an interrupted guard. Call-return and old terminals
/// do not acknowledge it. A missing, suspended, deferred or pending guard is inert: in particular
/// this must not synchronize an active composition into a fresh guard after `controls_without_editor`.
pub(crate) fn complete(app: &mut VectorcraftApp, ctx: &Context, request: egui::ImeResetId) -> bool {
    let viewport = ctx.viewport_id();
    if !app.ime_resets.get(&viewport).is_some_and(|guard| !guard.suspended && !guard.deferred && !guard.pending) {
        return false;
    }
    // This runs before the UI: a changed composition settles `pending` and its finalizer, after
    // the descriptor is confirmed, binds any replacement token.
    sync_incoming_impl(app, ctx, Fence::Pending);
    let destination = incoming_selection(app);
    if !app.ime_resets.get(&viewport).is_some_and(|guard| {
        !guard.suspended
            && !guard.deferred
            && !guard.pending
            && guard.request == Some(request)
            && guard.context == crate::graphics::context_tag(ctx)
            && request.viewport_id() == viewport
            && guard.destination == destination
    }) {
        return false;
    }
    app.ime_resets.remove(&viewport);
    app.ime_compositions.remove(&viewport);
    app.ime_marked = None;
    if app.ime_resets.is_empty() {
        app.ime_discard = false;
    }
    true
}

/// This path owns controls only when the custom editor cannot process ANY raw mutations.
/// Retire its activation before walking controls; leave the stock field's text/keys intact.
pub(crate) fn controls_without_editor(app: &mut VectorcraftApp, ctx: &Context) {
    suspend(app, ctx);
    let controls = ctx.input_mut(|input| {
        let mut controls = Vec::new();
        input.events.retain(|event| {
            if let egui::Event::ImeResetCompleted { request_id } = event {
                controls.push(*request_id);
                false
            } else {
                true
            }
        });
        controls
    });
    for request in controls {
        complete(app, ctx, request);
    }
}

/// Clear the previous pass's caret rectangle and published descriptor identity before this
/// viewport's UI runs.
pub(crate) fn begin_ui(app: &mut VectorcraftApp, ctx: &Context) {
    app.ime_caret_rect = None;
    let id = output_id(ctx.viewport_id());
    ctx.data_mut(|data| data.remove::<(u64, OutputContext)>(id));
}

/// Every UI exit, including discarded/stock/no-geometry output, observes this pass's descriptor.
/// Losing publication retires R1; reactivation allocates R2 even for the same model/caret.
pub(crate) fn finish_ui(app: &mut VectorcraftApp, ctx: &Context) {
    let viewport = ctx.viewport_id();
    let custom = ctx
        .data(|data| data.get_temp::<(u64, OutputContext)>(output_id(viewport)))
        .filter(|(pass, _)| *pass == ctx.cumulative_pass_nr_for(viewport))
        .map(|(_, key)| key);
    if !eligible(app, ctx, custom) {
        suspend(app, ctx);
        return;
    }
    // Only an eligible custom route may settle an active composition: doing this for a stock
    // field could allocate a request that core would attach to that field's descriptor.
    sync_incoming(app, ctx);
    let actual = ctx.output(|output| output.ime.as_ref().map(OutputContext::from));
    if !eligible_with(app, ctx, custom, actual) {
        suspend(app, ctx);
        return;
    }
    let destination = incoming_selection(app);
    let Some(custom) = custom else {
        suspend(app, ctx);
        return;
    };
    let replacement = app.ime_resets.get(&viewport).is_some_and(|guard| needs_replacement(guard, destination, custom));
    if replacement && !request_guard(app, ctx, false) {
        return;
    }
    if let Some(guard) = app.ime_resets.get_mut(&viewport) {
        guard.output = Some(custom);
    }
}

/// Whether this pass can publish a custom editor and the descriptor recorded during the pass is the
/// one actually published.
fn eligible(app: &VectorcraftApp, ctx: &Context, custom: Option<OutputContext>) -> bool {
    let actual = ctx.output(|output| output.ime.as_ref().map(OutputContext::from));
    eligible_with(app, ctx, custom, actual)
}

/// Require a custom descriptor, a matching actual output and an eligible route before an IME guard
/// may settle or re-arm.
fn eligible_with(app: &VectorcraftApp, ctx: &Context, custom: Option<OutputContext>, actual: Option<OutputContext>) -> bool {
    custom.is_some() && actual == custom && pass_eligible(app, ctx)
}

/// Whether a guard needs a new fence because it was suspended, deferred or pending, its destination
/// changed, or its published descriptor identity changed.
fn needs_replacement(guard: &ResetGuard, destination: Option<IncomingSelection>, custom: OutputContext) -> bool {
    guard.suspended || guard.deferred || guard.pending || guard.destination != destination || guard.output.is_some_and(|previous| previous != custom)
}

/// A full pass calls logic before UI; absence at that intermediate point is not publication
/// loss. Standalone run_logic has no fresh geometry and must retire a pending activation once.
pub(crate) fn finish_logic(app: &mut VectorcraftApp, ctx: &Context) {
    let id = mode_id(ctx.viewport_id());
    if !ctx.data(|data| data.get_temp::<bool>(id).unwrap_or(false)) {
        // A logic-only iteration has no fresh custom descriptor: settle a changed preview without
        // ever allocating a request (in particular for a hidden window with an external edit).
        sync_incoming_unbound(app, ctx);
        suspend(app, ctx);
    }
}

/// Read the current Type-tool target and its anchor and caret offsets, or `None` when no text edit
/// target or selection options are available.
fn incoming_selection(app: &mut VectorcraftApp) -> Option<IncomingSelection> {
    let view = app.view_info();
    let target = target(&mut app.session, view)?;
    let options = app.session.tool_options();
    Some(IncomingSelection { target, anchor: options.get("anchor")?.as_u64()?, caret: options.get("caret")?.as_u64()? })
}

/// Record a successfully accepted preedit. Accepted IME deletes refresh this selection without
/// invalidating the commit that belongs to them.
pub(crate) fn remember_incoming(app: &mut VectorcraftApp, ctx: &Context, preedit_empty: Option<bool>) {
    let viewport = ctx.viewport_id();
    let Some(selection) = incoming_selection(app) else { return };
    let next = match (preedit_empty, app.ime_compositions.get(&viewport)) {
        (Some(false), _) | (None, Some(IncomingComposition::Active(_))) => IncomingComposition::Active(selection),
        (Some(true), _) | (None, Some(IncomingComposition::TerminalPending(_))) => IncomingComposition::TerminalPending(selection),
        _ => return,
    };
    app.ime_compositions.insert(viewport, next);
}

/// How an incoming-composition settlement relates to a reset request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fence {
    /// Settle and allocate a request for this pass's custom descriptor.
    Bound,
    /// Settle and close the raw suffix, but leave the token to the finalizer that confirms the
    /// destination after the real pointer/navigation/command handler ran.
    Deferred,
    /// Settle before the UI without allocating or claiming a deferred notice: the ordered latch
    /// stays closed, text routing is unchanged, and the post-UI finalizer binds the replacement.
    Pending,
    /// Settle without a request: no custom editor owns this pass (hidden, stock, no geometry).
    Unbound,
}

/// Keep already-previewed marked text at its original position, without reinserting it through
/// `tool_text`: the caret or document may already have changed. Settle with no request on a route
/// that has no custom editor (a stock field, a hidden window); core could attach such a token to
/// the stock descriptor.
fn interrupt_incoming_unbound(app: &mut VectorcraftApp, ctx: &Context) {
    interrupt_incoming_impl(app, ctx, Fence::Unbound);
}

/// Returns whether an active/terminal preview was settled.
fn interrupt_incoming_impl(app: &mut VectorcraftApp, ctx: &Context, fence: Fence) -> bool {
    let viewport = ctx.viewport_id();
    let previous = app.ime_compositions.get(&viewport).copied();
    let Some(IncomingComposition::Active(expected) | IncomingComposition::TerminalPending(expected)) = previous else { return false };
    if fence == Fence::Unbound {
        // No fence will be bound on this route, so leave no Stale latch that only a marker could
        // clear. The composition is settled and the adapter returns to Clear.
        app.ime_compositions.remove(&viewport);
    } else {
        app.ime_compositions.insert(viewport, IncomingComposition::Stale);
    }
    app.ime_marked = None;
    app.ime_discard = true;
    let view = app.view_info();
    if app.session.tool_composing() && target(&mut app.session, view) == Some(expected.target) {
        match app.session.commit_interaction() {
            Ok(()) => app.session.set_tool_option("commitTyping", &serde_json::Value::Null),
            Err(error) => app.status(error.to_string()),
        }
    }
    match fence {
        Fence::Bound => {
            // Ask for the fence only after settlement: the guard's destination and committed stamp
            // are the post-commit ones, so an eligible pass does not immediately supersede it.
            // Refusal (the route became ineligible) retires the local guard and its latch.
            if !request_guard(app, ctx, false) {
                app.ime_compositions.remove(&viewport);
            }
        }
        Fence::Deferred => {
            if let Some(guard) = app.ime_resets.get_mut(&viewport) {
                guard.request = None;
                guard.deferred = true;
                guard.pending = false;
            }
        }
        Fence::Pending => mark_pending(app, ctx),
        Fence::Unbound => {}
    }
    true
}

/// The finalizer form: check application changes after the canvas/panels ran. A live preview
/// ending on its own is an interruption even if the caret happened to land at its old offset. No
/// prior composition means a direct commit remains legitimate. Only a post-UI call may bind.
pub(crate) fn sync_incoming(app: &mut VectorcraftApp, ctx: &Context) {
    sync_incoming_impl(app, ctx, Fence::Bound);
}

/// Pre-UI form: the pass's descriptor does not exist yet, so a changed composition or guard is
/// settled `pending` and the post-UI finalizer binds the replacement after confirming it.
pub(crate) fn sync_incoming_before_ui(app: &mut VectorcraftApp, ctx: &Context) {
    sync_incoming_impl(app, ctx, Fence::Pending);
}

/// Settlement for a pass that publishes no custom descriptor (standalone hidden logic). It settles
/// an interrupted preview without allocating a token that a later stock field could consume.
fn sync_incoming_unbound(app: &mut VectorcraftApp, ctx: &Context) {
    sync_incoming_impl(app, ctx, Fence::Unbound);
}

/// Settle an incoming composition whose selection, composing state, session or context no longer
/// matches the recorded expectation, applying the fence policy; with no recorded composition, also
/// retire or replace a guard whose destination, commit stamp, context or pending state changed.
fn sync_incoming_impl(app: &mut VectorcraftApp, ctx: &Context, fence: Fence) {
    let previous = app.ime_compositions.get(&ctx.viewport_id()).copied();
    let expected = match previous {
        Some(IncomingComposition::Active(selection) | IncomingComposition::TerminalPending(selection)) => selection,
        _ => {
            if !blocked(app, ctx) {
                return;
            }
            let selection = incoming_selection(app);
            let view = app.view_info();
            let committed = stamp(&mut app.session, view);
            if app.ime_resets.get(&ctx.viewport_id()).is_some_and(|guard| {
                !guard.suspended
                    && !guard.deferred
                    && (guard.pending
                        || guard.destination != selection
                        || guard.committed != committed
                        || guard.context != crate::graphics::context_tag(ctx))
            }) {
                match fence {
                    Fence::Bound => {
                        request_guard(app, ctx, false);
                    }
                    Fence::Pending => mark_pending(app, ctx),
                    Fence::Deferred | Fence::Unbound => suspend(app, ctx),
                }
            }
            return;
        }
    };
    let view = app.view_info();
    if incoming_selection(app) != Some(expected)
        || matches!(previous, Some(IncomingComposition::Active(_))) && !app.session.tool_composing()
        || !app.session.tool_ime_session_matches(view)
    {
        interrupt_incoming_impl(app, ctx, fence);
    }
}

/// Stable identity of the Type tool's current edit target.
///
/// The owner is the document uid plus the edited text node; the replacement epoch is the Type
/// tool's edit session, so leaving and re-entering the same text object starts a fresh IME
/// context while ordinary typing, selection and caret moves keep the old one.
pub(crate) fn target(session: &mut Session, view: ViewInfo) -> Option<ImeTarget> {
    session.tool_ime_target(view).map(|(document, node, session)| ImeTarget::new(Id::unique(("vectorcraft-type-ime", document, node)), session))
}

/// The bounded committed text and UTF-8 offsets, or `None` for an explicit unavailability reason.
pub(crate) fn committed(session: &mut Session, view: ViewInfo) -> Option<(String, usize, usize)> {
    match session.tool_ime_surrounding(SURROUNDING_LIMIT, view) {
        ImeSurrounding::Available { text, cursor, anchor } => Some((text, cursor, anchor)),
        ImeSurrounding::Unavailable(_) => None,
    }
}

/// A receipt stamp of the current committed context, for a successful mutation's boundary.
///
/// `None` means there is no edit target at all. An explicit unavailable query (a selection wider
/// than the limit, invalid offsets, or a typing session the document changed under) is still a
/// stamp: [`note_ime_apply`] refuses to earn a receipt from an unavailable boundary, so the
/// mutation stays [`egui::output::ImeChangeCause::Other`].
pub(crate) fn stamp(session: &mut Session, view: ViewInfo) -> Option<ImeMutationStamp> {
    target(session, view)?;
    let revision = session.active().map_or(0, |document| document.revision);
    Some(match committed(session, view) {
        Some((text, cursor, anchor)) => ImeMutationStamp::from_committed(&text, cursor, anchor, None),
        // Unavailable stamps need a producer revision to order two boundaries: without one,
        // before and after compare equal and an application edit cannot be distinguished from
        // an unchanged unavailable snapshot.
        None => ImeMutationStamp::unavailable_at_revision(revision),
    })
}

/// Record an accepted IME commit/delete after it succeeded, comparing the committed context
/// immediately before and after the engine call.
///
/// A stale target, a no-op (`before == after`), or an unavailable receipt earns nothing: the
/// tracker then reports `Other` for any snapshot change it cannot prove came from the IME.
pub(crate) fn note_ime_apply(
    app: &mut VectorcraftApp,
    ctx: &Context,
    view: ViewInfo,
    target_before: Option<ImeTarget>,
    before: Option<ImeMutationStamp>,
) {
    let (Some(target_before), Some(before)) = (target_before, before) else { return };
    if target(&mut app.session, view) != Some(target_before) {
        return;
    }
    let Some(after) = stamp(&mut app.session, view) else { return };
    if before == after {
        return;
    }
    // A receipt must describe the committed text on both sides of the mutation. An unavailable
    // snapshot (an external edit left the typing session stale, or a selection is wider than the
    // limit) cannot prove what the IME changed, so such a mutation stays `Other`.
    if before.surrounding_text().available().is_none() || after.surrounding_text().available().is_none() {
        return;
    }
    app.ime_context.note_input_method_apply(ctx.viewport_id(), ctx.cumulative_frame_nr_for(ctx.viewport_id()), target_before, before, after);
}

/// Record an application-origin text mutation (typing, paste, Backspace/Delete, a menu edit, a
/// correction). `Other` then dominates the whole frame, including any IME receipt in it.
///
/// `before == after` is a no-op and is not reported. A changed target (editing ended) is left to
/// the normal context-transition rule.
pub(crate) fn note_application_edit(
    app: &mut VectorcraftApp,
    ctx: &Context,
    view: ViewInfo,
    target_before: Option<ImeTarget>,
    before: Option<ImeMutationStamp>,
) {
    let (Some(target_before), Some(before)) = (target_before, before) else { return };
    if target(&mut app.session, view) != Some(target_before) {
        return;
    }
    let Some(after) = stamp(&mut app.session, view) else { return };
    if before == after {
        return;
    }
    app.ime_context.note_application_change(ctx.viewport_id(), ctx.cumulative_frame_nr_for(ctx.viewport_id()), target_before);
}

/// Observe the Type tool once, near the end of this viewport's pass.
///
/// `focused` is the canvas's own decision (the Type tool wants text, no other egui field owns the
/// keyboard, and no dialog or palette covers it). Pass `false` for a hidden/absent owner. The
/// tracker gets `cumulative_frame_nr_for` and `cumulative_pass_nr_for` separately, so cause
/// accumulates across a frame's discarded passes while an unchanged second pass keeps its cause.
pub(crate) fn observe(app: &mut VectorcraftApp, ctx: &Context, focused: bool) -> Option<ImeContextUpdate> {
    let viewport = ctx.viewport_id();
    if !focused {
        return app.ime_context.observe(viewport, None);
    }
    let view = app.view_info();
    let Some(target) = target(&mut app.session, view) else {
        return app.ime_context.observe(viewport, None);
    };
    let committed = committed(&mut app.session, view);
    let revision = app.session.active().map_or(0, |document| document.revision);
    let observation = ImeObservation::new(ctx.cumulative_frame_nr_for(viewport), ctx.cumulative_pass_nr_for(viewport), target)
        .focused(true)
        .purpose(egui::IMEPurpose::Normal)
        .committed(committed.as_ref().map(|(text, cursor, anchor)| CommittedText::new(text, *cursor, *anchor)))
        // The helper keeps this only for an unavailable snapshot; available text still compares
        // its exact bounded contents and selection.
        .unavailable_revision(revision);
    app.ime_context.observe(viewport, Some(observation))
}

/// Mark this viewport's IME owner absent for a pass (no document, Home, a hidden canvas).
pub(crate) fn observe_none(app: &mut VectorcraftApp, ctx: &Context) {
    app.ime_context.observe(ctx.viewport_id(), None);
}

/// Observe and write the outgoing descriptor once, at the end of the pass.
///
/// The canvas records only the caret geometry (`VectorcraftApp::ime_caret_rect`) while it draws.
/// By the time this runs, every dialog, floating panel, progress window and the palette has drawn
/// and ran its commands, so the owner, focused state, committed text and cause are the ones of the
/// latest completed pass. A dialog's own `TextEdit` descriptor is left alone: writing `None` here
/// would clobber it.
pub(crate) fn end_of_pass(app: &mut VectorcraftApp, ctx: &Context) {
    let ours = app.ime_caret_rect.is_some() && pass_eligible(app, ctx);
    if !ours {
        // No custom editor owns the IME this pass (Home, no document, a dialog, a palette, a hidden
        // window). Mark the context absent and let marked text stay as typed, as a click away does.
        // Do not synchronize or allocate here: the stock descriptor must carry no reset token.
        observe_none(app, ctx);
        interrupt_incoming_unbound(app, ctx);
        crate::shortcuts::keep_marked_text(app);
        return;
    }
    sync_incoming(app, ctx);
    let Some((rect, cursor_rect)) = app.ime_caret_rect else { return };
    // The tool ended the composition itself: the IME must drop what it still has marked.
    let mut interrupt = app.ime_marked.is_some() && !app.session.tool_composing();
    if interrupt {
        app.ime_marked = None;
        app.ime_discard = true;
        // The tool ended a composition that `sync_incoming` did not already settle (for example a
        // pre-edit without recorded geometry). Publish a request-bound interruption like every
        // other path, rather than an untokened `should_interrupt_composition`. If the route became
        // ineligible after `ours` was computed, publish no interruption without its fence.
        if !request_guard(app, ctx, false) {
            interrupt = false;
        }
    }
    let update = observe(app, ctx, true);
    write_output(ctx, update, rect, cursor_rect, interrupt);
}

/// Write every field of the descriptor the shared helper returned. Geometry stays the actual
/// caret rectangle; cause and surrounding text come from the tracker, never from a placeholder.
pub(crate) fn write_output(ctx: &Context, update: Option<ImeContextUpdate>, rect: egui::Rect, cursor_rect: egui::Rect, interrupt: bool) {
    let Some(update) = update else {
        ctx.output_mut(|o| o.ime = None);
        return;
    };
    ctx.output_mut(|o| {
        o.ime = Some(IMEOutput {
            owner: update.owner,
            context_generation: update.context_generation,
            purpose: update.purpose,
            rect,
            cursor_rect,
            should_interrupt_composition: interrupt,
            reset_request: None,
            change_cause: update.change_cause,
            surrounding_text: update.surrounding_text,
        });
    });
    if let Some(key) = ctx.output(|output| output.ime.as_ref().map(OutputContext::from)) {
        let id = output_id(ctx.viewport_id());
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|data| data.insert_temp(id, (pass, key)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A changed context generation or IME purpose for the same owner requires a replacement reset,
    /// while an unchanged descriptor keeps the existing request.
    #[test]
    fn same_owner_new_generation_or_purpose_requires_a_replacement_request() {
        let owner = Id::unique("vectorcraft-ime-output-context-test");
        let previous = OutputContext { owner, generation: 7, purpose: egui::IMEPurpose::Normal };
        let guard = ResetGuard {
            request: None,
            context: 1,
            destination: None,
            committed: None,
            output: Some(previous),
            suspended: false,
            deferred: false,
            pending: false,
        };
        let newer_generation = OutputContext { generation: 8, ..previous };
        assert_ne!(previous, newer_generation, "the same owner in a new context generation is a new output");
        assert!(needs_replacement(&guard, None, newer_generation));
        let other_purpose = OutputContext { purpose: egui::IMEPurpose::Password, ..previous };
        assert_ne!(previous, other_purpose, "the same owner and generation with another purpose is a new output");
        assert!(needs_replacement(&guard, None, other_purpose));
        assert!(!needs_replacement(&guard, None, previous), "an unchanged output keeps its request");
    }
}
