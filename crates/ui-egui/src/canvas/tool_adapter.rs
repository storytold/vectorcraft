//! The document tool as the framework's `CanvasTool`: the pointer and key calls of the tool host go to the engine
//! session, and what the session asks of the UI is applied here.

use std::cell::{Cell, RefCell, RefMut};

use craft::keymap::{Chord, Key};
use craft::behavior::{CanvasView, CancelReason, Cursor, PointerKind as Device, PressPhase};
use craft::ui::canvas::{
    Autoscroll, CanvasTool, Momentary, MomentaryShift, ToolAction, ToolClaims, ToolContext, ToolHost, ToolHostResponse, ToolKey, ToolPass, ToolPointer,
};
use craft::ui::canvas::ToolPath;
use serde_json::json;
use vectorcraft_engine::ViewInfo;
use vectorcraft_tools::{Mods, PenPose, PointerEvent, PointerKind, ToolKey as EngineKey};

use super::{Point, Ui, VectorcraftApp};
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
const PLANES: &[Chord] = &[Chord::plain(Key::Num1), Chord::plain(Key::Num2), Chord::plain(Key::Num3), Chord::plain(Key::Num4), Chord::plain(Key::Num5)];
const TAB: &[Chord] = &[Chord::plain(Key::Tab)];
const HOME_END: &[Chord] = &[Chord::plain(Key::Home), Chord::plain(Key::End)];

/// The modifiers of an egui key event, as the engine's tools read them.
pub fn mods(m: egui::Modifiers, space: bool) -> Mods {
    Mods { shift: m.shift, alt: m.alt, cmd: m.command, ctrl: m.ctrl, space }
}

fn tool_mods(m: craft::keymap::Mods, space: bool) -> Mods {
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

/// The clicks of the press run `clicks` counts, not counting the Image Trace pick the run began with: a pick consumed
/// its press, so the next press is a first click. A run that does not outgrow the pick starts over.
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

/// The session as a tool for one pass. The host's trait takes `&self` for the cursor and the claims, which the
/// session answers through `&mut`, so the app sits in a cell.
struct SessionTool<'a> {
    app: RefCell<&'a mut VectorcraftApp>,
    view: ViewInfo,
    /// The art of this gesture goes to a drag session (another tab or a panel): the view stays.
    carrying: Cell<bool>,
}

impl<'a> SessionTool<'a> {
    fn new(app: &'a mut VectorcraftApp, view: ViewInfo) -> Self {
        Self { app: RefCell::new(app), view, carrying: Cell::new(false) }
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
            // Presentation Mode: a click steps to the next artboard and the tool gets nothing; a modal blocks it.
            if app.ui.dialog.is_none() {
                crate::shortcuts::presentation_step(&mut app, true);
            }
            return ToolAction::Done;
        }
        let ev = pointer_event(PointerKind::Down, pointer, cx);
        let sampler = app.ui.trace_ignore_sampler.is_some() && app.session.tool_id() == "eyedropper" && app.session.lent_tool_id(&ev.mods).is_none();
        if sampler {
            // The armed pick takes this press whole: no drag or release follows, and it is not a click of a run.
            dispatch(&mut app, &ev, self.view);
            app.ui.trace_pick_clicks = pointer.clicks;
            return ToolAction::Done;
        }
        run_clicks(&mut app, pointer.clicks);
        dispatch(&mut app, &ev, self.view);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.send(PointerKind::Drag, pointer, cx);
        let carried = crate::drops::carry_out(&mut self.app(), cx, pointer);
        self.carrying.set(carried);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if cx.over_drop_target() {
            // The art goes to the drop target: the move is given up where it started.
            let _ = self.app().session.cancel_interaction();
        }
        self.send(PointerKind::Up, pointer, cx);
        let clicks = run_clicks(&mut self.app(), pointer.clicks);
        let follow = match clicks {
            2 => PointerKind::DoubleClick,
            3 => PointerKind::TripleClick,
            _ => return,
        };
        let before = self.app().session.tool_id();
        self.send(follow, pointer, cx);
        // A double click on type switched a selection tool to the Type tool: the caret goes where the type was
        // clicked, so typing edits it at once.
        let switched = follow == PointerKind::DoubleClick && before != "type" && self.app().session.tool_id() == "type";
        if switched {
            self.app().session.set_tool_option("prevTool", &json!(before));
            self.send(PointerKind::Down, pointer, cx);
            self.send(PointerKind::Up, pointer, cx);
        }
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.send(PointerKind::Move, pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        let mut app = self.app();
        let view = self.view;
        let mods = tool_mods(cx.mods, cx.held(Key::Space));
        let r = if reason == CancelReason::Escape {
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
        let (left, right, c) = (claim(&mut app, EngineKey::BracketLeft), claim(&mut app, EngineKey::BracketRight), claim(&mut app, EngineKey::Letter('c')));
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
        if claim(&mut app, EngineKey::Delete) {
            claims = claims.deletes();
        }
        if claim(&mut app, EngineKey::Left) {
            claims = claims.nudges();
        }
        claims
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let space = cx.held(Key::Space);
        let held = tool_mods(cx.mods, space);
        match key {
            ToolKey::Nudge { direction, mods } => {
                let k = match (direction.x.abs() > direction.y.abs(), direction.x < 0.0, direction.y < 0.0) {
                    (true, true, _) => EngineKey::Left,
                    (true, false, _) => EngineKey::Right,
                    (false, _, true) => EngineKey::Up,
                    (false, _, false) => EngineKey::Down,
                };
                self.send_key(k, tool_mods(*mods, space))
            }
            ToolKey::Delete { forward } => self.send_key(if *forward { EngineKey::Delete } else { EngineKey::Backspace }, held),
            ToolKey::Submit => self.send_key(EngineKey::Enter, held),
            ToolKey::Cancel => {
                let ends = self.app().session.tool_ends_on_escape(self.view);
                self.send_key(EngineKey::Escape, held) || ends
            }
            ToolKey::Kept(chord) => engine_key(*chord).is_some_and(|(k, shift)| self.send_key(k, Mods { shift, ..held })),
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
        app.session.tool_wants_samples() || app.session.active().is_some_and(|st| st.interaction.as_ref().is_some_and(|i| i.label == "Move" || i.label == "Copy"))
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

/// Runs the tool host on `tool` with the app's momentary set, which the app keeps across passes.
fn host_run(app: &mut VectorcraftApp, ui: &Ui, pass: &ToolPass, tool: &mut dyn CanvasTool) -> ToolHostResponse {
    let options = crate::app::view::navigation(&app.session.prefs);
    // PLAN: assumed `app.canvas.momentary: Option<Momentary>` (canvas-w10 owns the field).
    let mut momentary = app.canvas.momentary.take().unwrap_or_else(|| Momentary::new(options).shift(MomentaryShift::RotateView));
    momentary.set_options(options);
    momentary.hand_selected(app.session.tool_id() == "hand");
    let response = ToolHost::new().momentary(&mut momentary).run(ui, pass, tool);
    app.canvas.momentary = Some(momentary);
    response
}

/// One pass of the canvas's tool: the pointer and keys through the host, the view it moved, the document under the
/// pointer, and the tool's overlays over the picture.
pub(crate) fn run(app: &mut VectorcraftApp, ui: &Ui, pass: &ToolPass) -> ToolHostResponse {
    if crate::panels::three_d::gizmo_shown(app) {
        // PLAN: assumed `three_d::gizmo_shown(app) -> bool` (effects area): the Revolve gizmo owns the pointer.
        return ToolHostResponse::default();
    }
    if matches!(pass.press.phase, PressPhase::Idle) || !pass.press.phase.is_press() {
        crate::stylus::sync_eraser_tool(app);
    }
    let view_info = app.view_info();
    // PLAN: assumed `with_theme_pick(app, FnOnce(&mut VectorcraftApp, &mut dyn CanvasTool) -> ToolHostResponse) -> Option<ToolHostResponse>` (color-w20).
    let picked = crate::dialogs::recolor::edit_tab::with_theme_pick(app, |app, tool| host_run(app, ui, pass, tool));
    let response = match picked {
        Some(response) => response,
        None => {
            let mut tool = SessionTool::new(app, view_info);
            let response = host_run_session(&mut tool, ui, pass);
            drop(tool);
            response
        }
    };
    if app.session.tool_waiting() {
        let r = app.session.tool_poll(view_info, false);
        apply_requests(app, r);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    let mut view = response.view.or(pass.view).or_else(|| pass.space.map(|s| s.view));
    if let (Some(v), Some(step)) = (view.as_mut(), response.scroll) {
        // PLAN: assumed `VectorcraftApp::document_size() -> Option<Size>` for the autoscroll's limits (canvas-w10).
        if let Some(size) = app.document_size() {
            if Autoscroll::scroll(v, step, size).is_some() {
                ui.ctx().request_repaint();
            }
        }
    }
    let scaling = app.ui_scaling();
    if let (Some(v), Some(view)) = (app.view_mut(), view.as_ref()) {
        v.set_canvas(view, scaling);
    }
    // The press phase says whether the canvas has the pointer: a dialog, floating panel or menu over it hides the
    // document point, and a drag that began on the canvas keeps its own phase over them.
    app.hover_doc = match (pass.press.phase, view) {
        (PressPhase::Idle, _) | (_, None) => None,
        (_, Some(view)) => ui.input(|i| i.pointer.hover_pos()).map(|p| view.to_doc(p)),
    };
    show_overlays(app, ui, pass, view_info);
    response
}

fn host_run_session(tool: &mut SessionTool<'_>, ui: &Ui, pass: &ToolPass) -> ToolHostResponse {
    let mut app = tool.app();
    // The host borrows the tool and the app both through the cell, one call at a time.
    let options = crate::app::view::navigation(&app.session.prefs);
    // PLAN: assumed `app.canvas.momentary: Option<Momentary>` (canvas-w10 owns the field).
    let mut momentary = app.canvas.momentary.take().unwrap_or_else(|| Momentary::new(options).shift(MomentaryShift::RotateView));
    momentary.set_options(options);
    momentary.hand_selected(app.session.tool_id() == "hand");
    drop(app);
    let response = ToolHost::new().momentary(&mut momentary).run(ui, pass, tool);
    tool.app().canvas.momentary = Some(momentary);
    response
}

/// The session's overlays, then the text threads and Live Paint gaps under them, shown over the picture.
fn show_overlays(app: &mut VectorcraftApp, ui: &Ui, pass: &ToolPass, view_info: ViewInfo) {
    let Some(space) = pass.space else { return };
    let revolve = app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active);
    // View → Hide Gradient Annotator hides the Gradient tool's annotator.
    let list = if !revolve && (app.session.tool_id() != "gradient" || app.ui.view.gradient_annotator) { app.session.overlays(view_info) } else { vec![] };
    if app.session.tool_overlays_pending() {
        ui.ctx().request_repaint();
    }
    let mut extra = vec![];
    if app.ui.view.edges && !revolve {
        if app.ui.view.text_threads {
            extra.extend(super::marks::thread_overlays(app, space.view.zoom));
        }
        if app.ui.view.live_paint_gaps {
            extra.extend(super::marks::live_paint_gap_overlays(app));
        }
    }
    super::overlays::show(app, ui, space, &list, &extra);
}

/// Whether any canvas shown has a tool gesture in progress.
pub(crate) fn in_gesture(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    // PLAN: assumed `canvas::shown_canvas_ids(app) -> Vec<egui::Id>` from cv.S1 (canvas-w10).
    super::shown_canvas_ids(app).into_iter().any(|id| ToolHost::in_gesture(ctx, id))
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
    if ev.kind == PointerKind::Down && app.session.lent_tool_id(&ev.mods).is_none() {
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
