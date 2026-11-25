//! The document tool as the framework's `CanvasTool`: the pointer and key calls of the tool host go to the engine
//! session, and what the session asks of the UI is applied here.

use std::cell::{Cell, RefCell, RefMut};

use craft::behavior::{CancelReason, CanvasView, Cursor, DropOperation, PointerKind as Device, PressPhase};
use craft::keymap::{Chord, Key};
use craft::ui::canvas::ToolPath;
use craft::ui::canvas::{
    Autoscroll, CanvasTool, Momentary, MomentaryShift, ToolAction, ToolClaims, ToolContext, ToolHost, ToolHostResponse, ToolKey, ToolPass,
    ToolPointer,
};
use serde_json::json;
use vectorcraft_engine::ViewInfo;
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PenPose, PointerEvent, PointerKind, ToolKey as EngineKey};

use super::{Ui, VectorcraftApp};
use crate::cursors::tool_cursor;

/// The tools that change only the view: they neither autoscroll nor take the document's presses under a modal.
const VIEW_TOOLS: [&str; 3] = ["hand", "zoom", "rotateView"];

const BRACKETS: &[Chord] = &[Chord::plain(Key::OpenBracket), Chord::plain(Key::CloseBracket)];
const BRACKETS_CURLY: &[Chord] = &[
    Chord::plain(Key::OpenBracket),
    Chord::plain(Key::CloseBracket),
    Chord::plain(Key::OpenCurlyBracket),
    Chord::plain(Key::CloseCurlyBracket),
    Chord::new(craft::keymap::Mods::SHIFT, Key::OpenBracket),
    Chord::new(craft::keymap::Mods::SHIFT, Key::CloseBracket),
];
const BRACKETS_C: &[Chord] = &[Chord::plain(Key::OpenBracket), Chord::plain(Key::CloseBracket), Chord::plain(Key::C)];
const CLOSE_BRACKET: &[Chord] = &[Chord::plain(Key::CloseBracket)];
const PLANES: &[Chord] =
    &[Chord::plain(Key::Num1), Chord::plain(Key::Num2), Chord::plain(Key::Num3), Chord::plain(Key::Num4), Chord::plain(Key::Num5)];
const TAB: &[Chord] = &[Chord::plain(Key::Tab)];
const HOME_END: &[Chord] = &[Chord::plain(Key::Home), Chord::plain(Key::End)];

/// The modifiers of an egui key event, as the engine's tools read them.
pub fn mods(m: egui::Modifiers, space: bool) -> Mods {
    Mods { shift: m.shift, alt: m.alt, cmd: m.command, ctrl: m.ctrl, space }
}

pub(super) fn tool_mods(m: craft::keymap::Mods, space: bool) -> Mods {
    Mods { shift: m.shift, alt: m.alt, cmd: m.cmd, ctrl: m.ctrl, space }
}

/// `pose` with its tilt, given in screen axes, turned into the document's (the view rotated by Rotate View): the pen
/// leans the same way on the page as on the screen. The rotation is kept.
pub(crate) fn pose_on_page(pose: PenPose, view: &CanvasView) -> PenPose {
    let tilt = pose.tilt.map(|[x, y]| {
        let (tx, ty) = (f64::from(x).to_radians().tan(), f64::from(y).to_radians().tan());
        let (sn, cs) = view.rotation.to_radians().sin_cos();
        let (u, v) = (tx * cs + ty * sn, -tx * sn + ty * cs);
        [u.atan().to_degrees() as f32, v.atan().to_degrees() as f32]
    });
    PenPose { tilt, ..pose }
}

/// The pen's reading of `pointer` on the page; `None` for a mouse or a pen that reads neither tilt nor rotation.
fn pen_pose(pointer: &ToolPointer, view: &CanvasView) -> Option<PenPose> {
    if pointer.kind == Device::Mouse {
        return None;
    }
    let s = pointer.sample;
    let pose = PenPose {
        tilt: (s.tilt != [0.0, 0.0]).then(|| s.tilt.map(f32::to_degrees)),
        rotation: (s.rotation != 0.0).then(|| s.rotation.to_degrees().rem_euclid(360.0)),
        wheel: s.wheel,
    };
    (pose != PenPose::default()).then(|| pose_on_page(pose, view).sanitized())
}

/// The engine's event of `kind` for `pointer`: the document point under it, the modifiers (Space and Backquote from
/// the keys held), the pressure and the pen's pose.
pub(crate) fn pointer_event(kind: PointerKind, pointer: &ToolPointer, cx: &ToolContext<'_>) -> PointerEvent {
    let Some(space) = cx.space() else { return PointerEvent::new(kind, 0.0, 0.0) };
    PointerEvent {
        kind,
        pos: space.view.to_doc(pointer.pos),
        mods: tool_mods(pointer.mods, cx.held(Key::Space)),
        backquote: cx.held(Key::Backtick),
        pressure: pointer.sample.pressure,
        pen: pen_pose(pointer, &space.view),
    }
}

/// The clicks of the press run `clicks` counts, not counting the press the run began with that was no click: an Image
/// Trace pick consumed it, or it dragged, so the next press is a first click. A run that does not outgrow it starts over.
fn run_clicks(app: &mut VectorcraftApp, clicks: u32) -> u32 {
    let picked = app.ui.trace_pick_clicks;
    if clicks > picked {
        clicks - picked
    } else {
        app.ui.trace_pick_clicks = 0;
        clicks
    }
}

/// The artboard the Artboard tool makes active is the active one (the navigator's, the Artboards panel's).
fn sync_artboard(app: &mut VectorcraftApp) {
    if app.session.tool_id() == "artboard"
        && let Some(i) = app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok())
        && app.session.active().is_some_and(|d| i < d.doc.artboards.len())
        && let Some(v) = app.view_mut()
    {
        v.artboard = i;
    }
}

/// Enter that the tool does not claim, while no gesture runs: it takes what the tool asks of the UI (Rotate, Scale,
/// Reflect and Shear open their dialog), and with a selection tool it opens the Move dialog, as a double-click on the
/// tool's button does. Taken only when a dialog opened, so that Enter is not also its OK.
pub(crate) fn tool_enter(app: &mut VectorcraftApp, mods: Mods, view: ViewInfo) -> bool {
    if app.session.tool_busy() {
        return false;
    }
    let open = |app: &VectorcraftApp| app.dialogs().map_or(0, |d| d.with(|h| h.open_dialogs().len()));
    let before = open(app);
    let r = app.session.tool_key(EngineKey::Enter, mods, view);
    apply_requests(app, r);
    let tool = app.session.tool_id().to_owned();
    if open(app) == before && vectorcraft_tools::catalog::is_selection_tool(&tool) {
        // Nothing selected: it fails and nothing opens (the menu item is disabled then too).
        let _ = crate::toolbar::open_options(app, &tool);
    }
    let opened = open(app) > before;
    // The dialog uses the tool a held key chose.
    if opened {
        crate::shortcuts::note_tool_use(app);
    }
    opened
}

/// The modifiers held when the primary button went up in this pass: its first release, as a pass carries a press only
/// as its last pointer event. `None` without a release in the pass.
fn release_mods(ctx: &egui::Context) -> Option<egui::Modifiers> {
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } => Some(*modifiers),
            _ => None,
        })
    })
}

/// The session as a tool for one pass. The host's trait takes `&self` for the cursor and the claims, which the
/// session answers through `&mut`, so the app sits in a cell.
struct SessionTool<'a> {
    app: RefCell<&'a mut VectorcraftApp>,
    view: ViewInfo,
    /// The art of this gesture goes to a drag session (another tab or a panel): the view stays.
    carrying: Cell<bool>,
    /// The canvas's context and layer: a window over the canvas (a floating document's) hides it from the pointer.
    ctx: egui::Context,
    layer: egui::LayerId,
}

impl<'a> SessionTool<'a> {
    fn new(app: &'a mut VectorcraftApp, view: ViewInfo, ui: &Ui) -> Self {
        Self { app: RefCell::new(app), view, carrying: Cell::new(false), ctx: ui.ctx().clone(), layer: ui.layer_id() }
    }

    /// `ev` with Shift, Alt, Cmd and Ctrl as they were held when the primary button went up in this pass to end the
    /// gesture, instead of as they are at the end of the pass: a modifier pressed or let go after the release does not
    /// count for it. Space and Backtick stay as the host holds them. Unchanged without a release in the pass.
    fn at_release(&self, mut ev: PointerEvent) -> PointerEvent {
        if let Some(m) = release_mods(&self.ctx) {
            ev.mods = tool_mods(craft::keymap::Mods::from_egui(&m, crate::keymap::platform()), ev.mods.space);
        }
        ev
    }

    /// Whether another window covers the canvas at `pointer`, so art moved there leaves the canvas: a floating
    /// document's or a floating panel's OS window.
    fn covered(&self, pointer: &ToolPointer) -> bool {
        self.ctx.layer_id_at(pointer.pos).is_some_and(|l| l != self.layer) || crate::panel_docking::os_window_at(&self.app(), &self.ctx, pointer.pos)
    }

    fn app(&self) -> RefMut<'_, &'a mut VectorcraftApp> {
        self.app.borrow_mut()
    }

    fn send(&self, kind: PointerKind, pointer: &ToolPointer, cx: &ToolContext<'_>) {
        dispatch(&mut self.app(), &pointer_event(kind, pointer, cx), self.view);
    }

    /// Takes `key` for the tool when it claims it; whether it did.
    fn send_key(&self, key: EngineKey, mods: Mods) -> bool {
        let mut app = self.app();
        if !app.session.tool_claims_key_with(key, mods, self.view) {
            return false;
        }
        let r = app.session.tool_key(key, mods, self.view);
        if r.is_ok() {
            sync_artboard(&mut app);
        }
        apply_requests(&mut app, r);
        true
    }

    /// [`tool_enter`] in this canvas's view.
    fn enter(&self, mods: Mods) -> bool {
        tool_enter(&mut self.app(), mods, self.view)
    }

    /// Enter places Rearrange waiting for its click: at the pointer over the canvas, else at the first selected
    /// artboard's top-left → whether one was waiting. The Enter that confirmed its dialog is not taken.
    fn place_rearrange(&self) -> bool {
        use crate::dialogs::rearrange_artboards as rearrange;
        let mut app = self.app();
        if !rearrange::refresh_placement(&mut app) || !rearrange::armed(&app) {
            return false;
        }
        let at = app.hover_doc;
        rearrange::place(&mut app, at);
        true
    }

    /// A key the tool does not claim while its gesture runs: arrows and digits still reach it, with no modifier (a
    /// grid's rows and columns, the Perspective Selection tool's planes) → whether it took the key; other keys are
    /// [`Self::again`]'s.
    fn busy_key(&self, key: EngineKey) -> bool {
        let mut app = self.app();
        if !busy_tool(&app) || !matches!(key, EngineKey::Left | EngineKey::Right | EngineKey::Up | EngineKey::Down | EngineKey::Digit(_)) {
            return false;
        }
        let r = app.session.tool_key(key, Mods::default(), self.view);
        apply_requests(&mut app, r);
        true
    }

    /// While the gesture runs, the active tool's own shortcut chooses it again between clicks, which ends the gesture
    /// (W with the Blend tool's first pick held finishes the blend); any other plain key does nothing.
    fn again(&self, chord: Chord) -> bool {
        let mut app = self.app();
        if !busy_tool(&app) {
            return false;
        }
        let active = format!("tool:{}", app.session.tool_id());
        let own = app.ui_ctx.as_ref().and_then(craft::ui::runtime::commands::keymap).is_some_and(|keymap| {
            match keymap.lookup(&chord, false, &craft::keymap::Active::NONE) {
                craft::keymap::Lookup::Run(command) => command.id == active,
                craft::keymap::Lookup::Cycle(commands) => commands.iter().any(|command| command.id == active),
                _ => false,
            }
        });
        if own && !app.session.pointer_held() {
            let tool = app.session.tool_id().to_owned();
            app.select_tool(&tool);
        }
        true
    }
}

/// Whether the active tool's gesture runs (a press, an open pick or loop), outside text editing: production gave the
/// keys to the tool then and ran no shortcut.
pub(crate) fn busy_tool(app: &VectorcraftApp) -> bool {
    app.session.tool_busy() && !app.session.tool_wants_text()
}

/// The engine's key for the letter or digit key `key`, from its physical name (`KeyA`, `Digit5`).
fn engine_key(chord: Chord) -> Option<(EngineKey, bool)> {
    let shift = chord.mods.shift;
    Some(match chord.key {
        Key::OpenBracket => (EngineKey::BracketLeft, shift),
        Key::CloseBracket => (EngineKey::BracketRight, shift),
        Key::OpenCurlyBracket => (EngineKey::BracketLeft, true),
        Key::CloseCurlyBracket => (EngineKey::BracketRight, true),
        Key::Tab => (EngineKey::Tab, shift),
        Key::Home => (EngineKey::Home, shift),
        Key::End => (EngineKey::End, shift),
        Key::Left => (EngineKey::Left, shift),
        Key::Right => (EngineKey::Right, shift),
        Key::Up => (EngineKey::Up, shift),
        Key::Down => (EngineKey::Down, shift),
        key => {
            let name = key.code_name();
            if let Some(letter) = name.strip_prefix("Key").and_then(|l| l.chars().next()) {
                (EngineKey::Letter(letter.to_ascii_lowercase()), shift)
            } else {
                (EngineKey::Digit(name.strip_prefix("Digit")?.parse().ok()?), shift)
            }
        }
    })
}

impl CanvasTool for SessionTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let mut app = self.app();
        if app.ui.screen_mode == 3 {
            // Presentation Mode: the tool gets nothing; the release of a click steps to the next artboard.
            app.canvas.last_drag = None;
            return ToolAction::Claimed;
        }
        // Rearrange waiting for its click takes the press whole: the grid's top-left goes where it was pressed, and the
        // tool gets no press, drag or release.
        if crate::dialogs::rearrange_artboards::refresh_placement(&mut app) && !app.any_dialog_open() {
            let at = pointer_event(PointerKind::Down, pointer, cx).pos;
            crate::dialogs::rearrange_artboards::place(&mut app, Some(at));
            return ToolAction::Done;
        }
        // Text typed into a panel's field (its preview) goes to the object it edits before the press can select another.
        if app.session.active().and_then(|st| st.interaction.as_ref()).is_some_and(|it| it.keep_on_switch) {
            if let Err(e) = app.session.commit_interaction() {
                app.ui.status = e.to_string();
            }
            app.sync_views();
            app.canvas.committed_preview = true;
        }
        let mut ev = pointer_event(PointerKind::Down, pointer, cx);
        // Brush Options' Key Color pick samples the click whatever is held: Cmd or Ctrl lends no selection tool.
        let key_pick = crate::dialogs::brush_art_options::key_color_pick_active(&app);
        if key_pick {
            (ev.mods.cmd, ev.mods.ctrl) = (false, false);
        }
        let sampler = key_pick
            || app.ui.trace_ignore_sampler.is_some() && app.session.tool_id() == "eyedropper" && app.session.lent_tool_id(&ev.mods).is_none();
        if sampler || std::mem::take(&mut app.ui.trace_pick_stale) {
            // An armed pick takes this press whole, as does the one of the pass that ended a stale pick: no drag or
            // release follows, and it is not a click of a run.
            if sampler {
                dispatch(&mut app, &ev, self.view);
            }
            app.ui.trace_pick_clicks = pointer.clicks;
            return ToolAction::Done;
        }
        run_clicks(&mut app, pointer.clicks);
        // Where the art was grabbed, should a move carry it off the canvas.
        app.canvas.grab = Some(ev.pos);
        app.canvas.last_drag = None;
        dispatch(&mut app, &ev, self.view);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if self.app().ui.screen_mode == 3 {
            // A drag in Presentation Mode is no click.
            self.app().canvas.last_drag = Some((pointer.pos, pointer_event(PointerKind::Drag, pointer, cx)));
            return;
        }
        // Art carried off the canvas waits where it last moved; back on the canvas the move goes on.
        let covered = self.covered(pointer);
        let carried = super::drops::carry_out(&mut self.app(), cx, pointer, covered);
        if !carried {
            // The moves of the pass that releases the press came before its release, with the keys held then.
            let ev = self.at_release(pointer_event(PointerKind::Drag, pointer, cx));
            dispatch(&mut self.app(), &ev, self.view);
            self.app().canvas.last_drag = Some((pointer.pos, ev));
        }
        self.carrying.set(carried);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let mut app = self.app();
        if app.ui.screen_mode == 3 {
            // Presentation Mode: a click steps to the next artboard, a drag does nothing; a modal blocks it. A press
            // that becomes a touch gesture is cancelled, so a tap of two fingers steps nowhere.
            if app.canvas.last_drag.take().is_none() && !app.any_dialog_open() {
                crate::shortcuts::presentation_step(&mut app, true);
            }
            return;
        }
        drop(app);
        // The press ends where it went up, with the keys held at its release.
        let up = self.at_release(pointer_event(PointerKind::Up, pointer, cx));
        let covered = self.covered(pointer);
        let carried = super::drops::carried(&self.app(), cx, pointer, covered);
        self.app().canvas.panes.released_from = carried.as_ref().map(|art| (art.document, art.revision));
        let given_up = cx.over_drop_target() || carried.is_some();
        if given_up {
            // The art goes to the drop target, or nowhere off the canvas: the move is given up where it started.
            let _ = self.app().session.cancel_interaction();
        }
        if let Some(art) = carried.filter(|_| !cx.over_drop_target()) {
            // Released over a pane in the frame the pointer reached it: no target could take it yet.
            let op = if up.mods.alt { DropOperation::Copy } else { DropOperation::Move };
            let mut app = self.app();
            app.canvas.panes.released = super::drops::released_on_pane(&app, art, pointer.pos, op);
        }
        self.app().canvas.grab = None;
        // When the pointer moved or the keys changed since the drag's last move, the tool gets a move where the press went
        // up first, as it does while held: the last move of the release's pass still sizes the shape, and Shift pressed in
        // that pass still draws a square. A move that was given up gets none.
        let last = self.app().canvas.last_drag.map(|(_, ev)| ev);
        if !given_up && last.is_some_and(|last| (last.pos, last.mods, last.backquote) != (up.pos, up.mods, up.backquote)) {
            dispatch(&mut self.app(), &PointerEvent { kind: PointerKind::Drag, ..up }, self.view);
        }
        dispatch(&mut self.app(), &up, self.view);
        if self.app().canvas.last_drag.is_some() {
            // A press that dragged is no click, and the framework counts presses: the next press starts a new run.
            self.app().ui.trace_pick_clicks = pointer.clicks;
            return;
        }
        let clicks = run_clicks(&mut self.app(), pointer.clicks);
        let follow = match clicks {
            2 => PointerKind::DoubleClick,
            3 => PointerKind::TripleClick,
            _ => return,
        };
        let before = self.app().session.tool_id();
        dispatch(&mut self.app(), &PointerEvent { kind: follow, ..up }, self.view);
        // A double click on type switched a selection tool to the Type tool: the caret goes where the type was
        // clicked, so typing edits it at once.
        let switched = follow == PointerKind::DoubleClick && before != "type" && self.app().session.tool_id() == "type";
        if switched {
            self.app().session.set_tool_option("prevTool", &json!(before));
            for kind in [PointerKind::Down, PointerKind::Up] {
                // Without Cmd, so the Type tool takes the click instead of lending a selection tool.
                let mut ev = pointer_event(kind, pointer, cx);
                ev.mods.cmd = false;
                dispatch(&mut self.app(), &ev, self.view);
            }
        }
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.send(PointerKind::Move, pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        let mut app = self.app();
        if app.ui.screen_mode == 3 {
            // The tool had no press of Presentation Mode's.
            app.canvas.last_drag = None;
            return;
        }
        app.canvas.grab = None;
        let view = self.view;
        let mods = tool_mods(cx.mods, cx.held(Key::Space));
        let r = if reason == CancelReason::Escape {
            app.canvas.escaped_press = true;
            app.session.tool_key(EngineKey::Escape, mods, view)
        } else {
            app.session.drop_tool_press(view).map(|()| vec![])
        };
        apply_requests(&mut app, r);
    }

    fn claims(&self) -> ToolClaims {
        let mut app = self.app();
        let view = self.view;
        let claim = |app: &mut VectorcraftApp, key: EngineKey| app.session.tool_claims_key(key, view);
        let curly = app.session.tool_claims_key_with(EngineKey::BracketLeft, Mods { shift: true, ..Mods::default() }, view);
        let (left, right, c) =
            (claim(&mut app, EngineKey::BracketLeft), claim(&mut app, EngineKey::BracketRight), claim(&mut app, EngineKey::Letter('c')));
        // shortcut: the sets are the combinations the tools have; a tool with another mix needs its own slice.
        let keys = match (left, right, curly, c) {
            (true, true, true, _) => BRACKETS_CURLY,
            (true, true, false, true) => BRACKETS_C,
            (true, true, false, false) => BRACKETS,
            (false, true, ..) => CLOSE_BRACKET,
            _ if claim(&mut app, EngineKey::Digit(1)) => PLANES,
            _ if claim(&mut app, EngineKey::Tab) => TAB,
            _ if claim(&mut app, EngineKey::Home) => HOME_END,
            _ => &[],
        };
        let mut claims = ToolClaims::NONE.keys(keys);
        // While a gesture runs the plain keys are the tool's: no single-key shortcut runs then.
        if busy_tool(&app) {
            claims = claims.keyboard();
        }
        if claim(&mut app, EngineKey::Delete) {
            claims = claims.deletes();
        }
        if claim(&mut app, EngineKey::Left) {
            claims = claims.nudges();
        }
        if claim(&mut app, EngineKey::AltTap) {
            claims = claims.alt_tap();
        }
        claims
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let space = cx.held(Key::Space);
        let held = tool_mods(cx.mods, space);
        match key {
            ToolKey::Nudge { direction, mods, held: with, .. } => {
                let (k, arrow) = match (direction.x.abs() > direction.y.abs(), direction.x < 0.0, direction.y < 0.0) {
                    (true, true, _) => (EngineKey::Left, Key::Left),
                    (true, false, _) => (EngineKey::Right, Key::Right),
                    (false, _, true) => (EngineKey::Up, Key::Up),
                    (false, _, false) => (EngineKey::Down, Key::Down),
                };
                if self.send_key(k, tool_mods(*mods, space)) || self.busy_key(k) {
                    return true;
                }
                // An arrow an application command is bound to runs it, as production's keymap ran it first: the
                // canvas takes every arrow, so no shortcut runs it otherwise. One it leaves moves the selection, with
                // Backtick held only its pattern paint. A panel's own arrows (a page preview's) are not the canvas's.
                let chord = Chord::new(*mods, arrow);
                let ctx = self.app().ui_ctx.clone();
                let bound = ctx.as_ref().and_then(craft::ui::runtime::commands::keymap).and_then(|keymap| {
                    let global = |command: &craft::keymap::CommandRef| {
                        keymap.bindable(command).is_some_and(|def| keymap.scopes().kind(&def.scope) == craft::keymap::ScopeKind::Application)
                    };
                    match keymap.lookup_where(&chord, false, &craft::keymap::Active::NONE, &[], &global) {
                        craft::keymap::Lookup::Run(command) => Some(command),
                        // The key's own press fires the group's first tool, as here.
                        craft::keymap::Lookup::Cycle(commands) => commands.into_iter().next(),
                        _ => None,
                    }
                });
                match ctx.zip(bound) {
                    Some((ctx, command)) => {
                        craft::ui::runtime::commands::fire(&ctx, command, craft::ui::runtime::commands::FireSource::Key);
                    }
                    None => crate::shortcuts::nudge(&mut self.app(), *direction, *mods, with.contains(&Key::Backtick)),
                }
                true
            }
            ToolKey::Delete { forward, .. } => self.send_key(if *forward { EngineKey::Delete } else { EngineKey::Backspace }, held),
            ToolKey::Submit => {
                // In Presentation Mode Enter opens no dialog: the tool takes none.
                let presenting = self.app().ui.screen_mode == 3;
                if !presenting && self.place_rearrange() {
                    return true;
                }
                !presenting && (self.send_key(EngineKey::Enter, held) || self.enter(held))
            }
            ToolKey::Cancel => {
                // In Presentation Mode Escape leaves the mode: the tool takes none.
                if self.app().ui.screen_mode == 3 {
                    return false;
                }
                // Escape drops Rearrange waiting for its click.
                let mut app = self.app();
                if crate::dialogs::rearrange_artboards::refresh_placement(&mut app) {
                    crate::dialogs::rearrange_artboards::cancel_placement(&mut app);
                    return true;
                }
                drop(app);
                let ends = self.app().session.tool_ends_on_escape(self.view);
                if self.send_key(EngineKey::Escape, held) {
                    return true;
                }
                // What Escape ends (a loop, a path being drawn, a running gesture) gets it whether or not the tool
                // claims it.
                let mut app = self.app();
                if !ends && !app.session.tool_busy() {
                    return false;
                }
                let r = app.session.tool_key(EngineKey::Escape, held, self.view);
                apply_requests(&mut app, r);
                true
            }
            ToolKey::Kept(chord) => {
                engine_key(*chord).is_some_and(|(k, shift)| self.send_key(k, Mods { shift, ..held }) || self.busy_key(k)) || self.again(*chord)
            }
            ToolKey::AltTap => self.send_key(EngineKey::AltTap, held),
            _ => false,
        }
    }

    fn paint(&self, _draw: &craft::ui::primitives::Draw<'_>, _cx: &ToolContext<'_>) {
        // The document's overlays are shown by `overlays::show` after the host, in one list.
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let Some(space) = cx.space() else { return tool_cursor(vectorcraft_tools::Cursor::Arrow) };
        let at = cx.pointer.map_or(Point::ZERO, |p| space.view.to_doc(p.pos));
        let c = self.app().session.cursor(at, tool_mods(cx.mods, cx.held(Key::Space)), self.view);
        // PLAN: assumed the engine's own precise step is dropped (canvas-w25); the canvas's Caps Lock/preference state arrives as `cx.precise()`.
        tool_cursor(if cx.precise() { c.precise() } else { c })
    }

    fn takes_every_move(&self) -> bool {
        let app = self.app();
        app.session.tool_wants_samples()
            || app.session.active().is_some_and(|st| st.interaction.as_ref().is_some_and(|i| i.label == "Move" || i.label == "Copy"))
    }

    fn autoscrolls(&self) -> bool {
        !self.carrying.get() && !self.view_only()
    }

    fn view_only(&self) -> bool {
        VIEW_TOOLS.contains(&self.app().session.tool_id())
    }

    fn command_form(&self) -> ToolPath {
        if self.app().session.tool_wants_samples() {
            ToolPath::Freehand("It follows the pointer; the Object and Path commands make the same shapes by number.")
        } else {
            ToolPath::Command("the Object and Path commands")
        }
    }
}

/// Runs the tool host on `tool` with the app's momentary set, which the app keeps across passes. A finger, once a pen
/// has been seen, only navigates (palm rejection).
fn host_run(app: &mut VectorcraftApp, ui: &Ui, pass: &ToolPass, tool: &mut dyn CanvasTool) -> ToolHostResponse {
    let options = crate::app::view::navigation(&app.session.prefs);
    let mut momentary = app.canvas.momentary.take().unwrap_or_else(|| Momentary::new(options).shift(MomentaryShift::RotateView));
    momentary.set_options(options);
    momentary.hand_selected(app.session.tool_id() == "hand");
    let response = ToolHost::new().momentary(&mut momentary).navigates(crate::touch::finger_navigates_only()).run(ui, pass, tool);
    app.canvas.momentary = Some(momentary);
    response
}

/// One pass of the canvas's tool: the pointer and keys through the host, the view it moved, the document under the
/// pointer, and the tool's overlays over the picture.
pub(crate) fn run(app: &mut VectorcraftApp, ui: &mut Ui, pass: &ToolPass) -> ToolHostResponse {
    if crate::dialogs::three_d::gizmo_shown(app) {
        // The 3D dialog's gizmo owns the pointer.
        return ToolHostResponse::default();
    }
    // Once a frame, before its press, which then uses the new tool; a held press (its grab) waits for the frame after
    // the release, and a pointer over another window for the pointer to come in.
    if ui.ctx().current_pass_index() == 0 && app.canvas.grab.is_none() && ui.input(|i| i.pointer.has_pointer()) {
        crate::stylus::sync_eraser_tool(app);
    }
    // A press on the canvas that a view tool takes (a middle-button pan, a held Space's Hand) counts as a press
    // elsewhere for the clicks the tool's double-click or triple-click compares.
    let over = |at: Option<egui::Pos2>| at.zip(pass.space).is_some_and(|(at, space)| space.view.rect.contains(at));
    let held = view_tool_held(app);
    let view_press = ui.input(|i| {
        let pressed = i.pointer.button_pressed(egui::PointerButton::Middle) || held && i.pointer.primary_pressed();
        pressed && over(i.pointer.press_origin())
    });
    if ui.ctx().current_pass_index() == 0 && view_press {
        app.session.tool_press_elsewhere();
    }
    let view_info = app.view_info();
    let mut picked = ToolHostResponse::default();
    let mut response = if crate::dialogs::recolor::edit_tab::with_theme_pick(app, |app, tool| picked = host_run(app, ui, pass, tool)) {
        picked
    } else {
        let mut tool = SessionTool::new(app, view_info, ui);
        let response = host_run_session(&mut tool, ui, pass);
        drop(tool);
        response
    };
    // The framework ends a press at Escape; the engine's tool still takes the release that ends it.
    if app.canvas.escaped_press && !ui.input(|i| i.pointer.primary_down()) {
        app.canvas.escaped_press = false;
        dispatch(
            app,
            &PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), backquote: false, pressure: 1.0, pen: None },
            view_info,
        );
    }
    if app.session.tool_waiting() {
        let r = app.session.tool_poll(view_info, false);
        apply_requests(app, r);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    let mut view = response.view.or(pass.view).or_else(|| pass.space.map(|s| s.view));
    let mut scrolled = false;
    if let (Some(v), Some(step)) = (view.as_mut(), response.scroll)
        && Autoscroll::scroll(v, step, super::document_size(app.content_bounds())).is_some()
    {
        ui.ctx().request_repaint();
        // The body takes the reply's view as the one the canvas showed.
        response.view = Some(*v);
        scrolled = true;
    }
    let scaling = app.ui_scaling();
    if let (Some(v), Some(view)) = (app.view_mut(), view.as_ref()) {
        v.set_canvas(view, scaling);
    }
    // The view scrolled under the held pointer: the drag goes on where the pointer now is in the document.
    if let (true, Some(view), Some((at, mut ev))) = (scrolled && app.session.in_interaction(), view, app.canvas.last_drag) {
        ev.pos = view.to_doc(at);
        let info = app.view_info();
        dispatch(app, &ev, info);
    }
    // The press phase says whether the canvas has the pointer: a dialog, floating panel or menu over it hides the
    // document point, and a drag that began on the canvas keeps its own phase over them.
    app.hover_doc = match (pass.press.phase, view) {
        (PressPhase::Idle, _) | (_, None) => None,
        (_, Some(view)) => ui.input(|i| i.pointer.hover_pos()).map(|p| view.to_doc(p)),
    };
    // Rearrange waiting for its click: dropped with its document or tool, and from the next pass Enter places it.
    if crate::dialogs::rearrange_artboards::refresh_placement(app) {
        crate::dialogs::rearrange_artboards::arm(app);
    }
    show_overlays(app, ui, pass, view_info);
    response
}

fn host_run_session(tool: &mut SessionTool<'_>, ui: &Ui, pass: &ToolPass) -> ToolHostResponse {
    let mut app = tool.app();
    // The host borrows the tool and the app both through the cell, one call at a time.
    let options = crate::app::view::navigation(&app.session.prefs);
    let mut momentary = app.canvas.momentary.take().unwrap_or_else(|| Momentary::new(options).shift(MomentaryShift::RotateView));
    momentary.set_options(options);
    momentary.hand_selected(app.session.tool_id() == "hand");
    drop(app);
    let response = ToolHost::new().momentary(&mut momentary).navigates(crate::touch::finger_navigates_only()).run(ui, pass, tool);
    tool.app().canvas.momentary = Some(momentary);
    response
}

/// The session's overlays, then the text threads and Live Paint gaps under them, shown over the picture.
fn show_overlays(app: &mut VectorcraftApp, ui: &mut Ui, pass: &ToolPass, view_info: ViewInfo) {
    let Some(space) = pass.space else { return };
    let revolve = crate::dialogs::three_d::gizmo_shown(app);
    // View → Hide Gradient Annotator hides the Gradient tool's annotator.
    let list =
        if !revolve && (app.session.tool_id() != "gradient" || app.ui.view.gradient_annotator) { app.session.overlays(view_info) } else { vec![] };
    if app.session.tool_overlays_pending() {
        ui.ctx().request_repaint();
    }
    // Gap Options' Preview shows the gaps it finds.
    let mut extra = crate::dialogs::live_paint_gaps::overlays(app);
    // Rearrange waiting for its click shows where its artboards go.
    extra.extend(crate::dialogs::rearrange_artboards::placement_overlays(app));
    if app.ui.view.edges && !revolve {
        if app.ui.view.text_threads {
            extra.extend(super::marks::thread_overlays(app, space.view.zoom));
        }
        if app.ui.view.live_paint_gaps {
            extra.extend(super::marks::live_paint_gap_overlays(app));
        }
    }
    super::overlays::show(app, ui, &space, &list, &extra);
}

/// Whether the canvas has a tool gesture in progress. Only the current window's body is live: panes and the other
/// windows take no presses.
pub(crate) fn in_gesture(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    crate::doc_windows::current(app).is_some_and(|win| ToolHost::in_gesture(ctx, super::canvas_id(win)))
}

/// Whether a momentary view tool (Space, Cmd+Space, ...) is in effect.
pub(crate) fn view_tool_held(app: &VectorcraftApp) -> bool {
    app.canvas.momentary.as_ref().is_some_and(|m| m.held().is_some())
}

/// Send a pointer event to the active tool and act on UI requests (dialogs, tool switches).
pub fn dispatch(app: &mut VectorcraftApp, ev: &PointerEvent, view: ViewInfo) {
    if ev.kind == PointerKind::Down && crate::panels::image_trace::discard_stale_ignore_click(app) {
        return;
    }
    // A press lent to a selection tool (Cmd/Ctrl held) does not use the tool of a held key.
    if ev.kind == PointerKind::Down && !app.session.press_lends_tool(ev, view) {
        crate::shortcuts::note_tool_use(app);
    }
    let r = app.session.pointer(ev, view);
    sync_artboard(app);
    if ev.kind == PointerKind::Down {
        pressed_with(app, app.session.active_index(), ev.pos, app.session.tool_id());
    }
    apply_requests(app, r);
}

/// A press at `pos` in document `index` with tool `tool` (a selection tool) inside an artboard makes it that
/// document's active one, as the Artboard tool's does: Paste in Place and in Front or Back then paste onto it, Align
/// to Artboard aligns to it, and All on Active Artboard selects on it.
fn pressed_with(app: &mut VectorcraftApp, index: Option<usize>, pos: Point, tool: &str) {
    if !vectorcraft_tools::catalog::is_selection_tool(tool) {
        return;
    }
    app.sync_views();
    if let Some(i) = index
        && let Some(board) = app.session.documents().get(i).and_then(|d| d.doc.artboard_at(pos))
        && let Some(v) = app.views.get_mut(i)
    {
        v.artboard = board;
    }
}

/// Act on what a tool event asked the UI for (dialogs, tool switches), or show its error. When an edit the tool
/// committed or ran no longer fits in the action being recorded, the edit stays and the status bar names the limit
/// ([`crate::panels::actions::after_journaled_edit`]).
pub fn apply_requests(app: &mut VectorcraftApp, r: vectorcraft_engine::Result<Vec<vectorcraft_engine::UiRequest>>) {
    match r {
        Ok(reqs) => {
            let mut sampler_return = None;
            for r in reqs {
                match r {
                    vectorcraft_engine::UiRequest::Dialog(kind, p) => {
                        let back = if kind == "imageTraceIgnoreColor" && p["cancel"] == true {
                            app.ui.trace_ignore_sampler.as_ref().map(|sampler| sampler.back.clone())
                        } else {
                            None
                        };
                        crate::dialogs::open_tool_dialog(app, &kind, p);
                        // A matching callback must actually consume the active sampler.
                        sampler_return = back.filter(|_| app.ui.trace_ignore_sampler.is_none());
                    }
                    vectorcraft_engine::UiRequest::Panel(panel) => activate_panel(app, &panel),
                    vectorcraft_engine::UiRequest::SwitchTool(t) => {
                        if sampler_return.take().is_some_and(|back| back == t) {
                            app.return_to_tool(&t);
                        } else {
                            app.select_tool(&t);
                        }
                    }
                    vectorcraft_engine::UiRequest::Status(msg) => app.status(msg),
                    vectorcraft_engine::UiRequest::Place(p) => crate::place::place_from_cursor(app, &p),
                    // In the document the press was made in, which may no longer be the active one.
                    vectorcraft_engine::UiRequest::HeldPress { doc, pos, tool } => {
                        let index = doc.and_then(|uid| app.session.documents().iter().position(|d| d.uid == uid));
                        pressed_with(app, index, pos, tool);
                    }
                }
            }
            crate::panels::actions::after_journaled_edit(app);
        }
        Err(e) => app.status(e.to_string()),
    }
}

/// Show and select `panel` where it is (`window.panel.activate`): a panel already on show stays on show, and a dock
/// hidden by Tab shows again.
fn activate_panel(app: &mut VectorcraftApp, panel: &str) {
    if let Err(e) = app.run("window.panel.activate", json!({ "panel": panel })) {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;
