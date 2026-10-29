use super::artboards::{artboard_name_at, begin_artboard_name_edit, name_press_id, select_named_artboard};
use super::drag::{
    HoverContext, apply_requests, dispatch, drag_art_out, drag_id, drag_layer_id, drag_pos_id, drop_press, mods, on_canvas, pen_pressure,
    tool_drag_doc_id, tool_drag_moved, tool_frame_moves, tool_holds, tool_press_doc_id,
};
use super::view::{
    HOLD_DELAY, HOLD_ZOOM, SCRUB_SLOP, SCRUB_ZOOM, WheelTurn, animated_zoom, pan_pixels, rotate_about, scroll_view, touch_gestures, trackpad_turn,
    wheel, whole_pixels, zoom_about,
};
use super::{Color32, Drag, Mods, Point, PointerEvent, PointerKind, Pos2, Shape, Stroke, Ui, VectorcraftApp, View, Xf, json};

/// Dispatch canvas pointer, keyboard and drop events through the active document view.
pub(super) fn handle_input(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect) {
    let (pointer, m, space, hover) = wheel_view(app, ui, resp, rect);
    let v = *app.view().unwrap_or(&View::default());
    let xf = Xf::new(rect, &v);
    // A lifted pen or finger takes the pointer away in the frame it lifts (egui's `PointerGone`),
    // so a tap whose press and lift come in one frame has no hover position: it presses (and
    // double-taps) where it was seen last (#491).
    let at = hover.or_else(|| pointer.interact_pos().filter(|p| rect.contains(*p)));
    let view = app.view_info();
    let f = Pointing { resp, rect, pointer: &pointer, m, space, xf, v, view };
    touch_gestures(app, ui, rect, view);
    // Pressed and held still, a pen or a finger opens the context menu (egui's long touch): the
    // tool's press is dropped, as a right-click presses nothing.
    if resp.long_touched() {
        drop_press(app, ui, view);
    }
    let drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id()));
    // A modifier pressed or released over the canvas re-hovers the tool, so what it changes shows
    // without moving the mouse (Alt switches the Shape Builder to erase mode).
    let mods_changed = ui.data_mut(|d| {
        let id = egui::Id::unique("canvas-mods");
        let prev = d.get_temp::<egui::Modifiers>(id);
        d.insert_temp(id, m);
        prev.is_some_and(|prev| prev != m)
    });
    // Where the tool hovers: the pointer over the canvas itself, not over a floating panel, a menu,
    // a popup or a dialog drawn on top of it ([`on_canvas`]).
    let hover_tool = hover.filter(|p| on_canvas(ui, resp, *p));
    // Whether the tool hovered last frame: coming onto the canvas re-hovers it, even after a single
    // jump egui doesn't count as moving, and leaving it ends the tool's hover (below).
    let was_over = ui.data_mut(|d| {
        let id = egui::Id::unique("canvas-hovered");
        let prev = d.get_temp::<bool>(id).unwrap_or(false);
        d.insert_temp(id, hover_tool.is_some());
        prev
    });
    // What a hover means besides where the pointer is on screen: the tool, the canvas, the view
    // (scrolled, zoomed or turned by the wheel, a gesture, keys or a command) and the document and
    // selection as they are (another tab, an edit, an undo). Under a still pointer, a tool switched
    // to is hovered at once, and when the rest changes, a tool that asks for it
    // ([`Session::tool_wants_rehover`]) is hovered again where the pointer is.
    let context = HoverContext {
        tool: app.session.tool_id(),
        canvas: rect,
        center: v.center,
        zoom: v.zoom,
        rotation: v.rotation,
        doc: app.session.active().map(|st| (st.uid, st.revision)),
    };
    let (tool_switched, context_changed) = ui.data_mut(|d| {
        let id = egui::Id::unique("canvas-hover-context");
        let prev = d.get_temp::<HoverContext>(id);
        d.insert_temp(id, context);
        prev.map_or((false, false), |prev| (prev.tool != context.tool, prev != context))
    });

    if app.ui.screen_mode == 3 && matches!(drag, None | Some(Drag::Present | Drag::Gesture)) {
        present_step(app, ui, &f, drag, at);
        return;
    }

    // A pen turned to its eraser end selects the Eraser, and a pen turned back to its tip selects
    // the tool before it. The switch runs only while the pointer is in this window. It runs before
    // this frame's press, which then uses the new tool; while a press is held, it waits for the
    // release. A turn made over another window takes effect when the pointer comes in.
    if drag.is_none() && ui.input(|i| i.pointer.has_pointer()) {
        crate::stylus::sync_eraser_tool(app);
    }
    let middle_pan = matches!(drag, Some(Drag::Pan { middle: true, .. }));
    // egui counts a press a few pixels outside the canvas as on it (its interaction radius): only a
    // press on the canvas itself reaches the tools, else it would land at the canvas's centre.
    if pointer.primary_pressed()
        && resp.hovered()
        && !middle_pan
        && let Some(p) = at
    {
        ui.ctx().memory_mut(|mem| mem.stop_text_input());
        app.ui.flyout = None;
        // The artboard's name sits above its corner. A press there renames on a double-click
        // and does not start a tool gesture (a fitted artboard, a new one, Artboard Options).
        if let Some(i) = artboard_name_at(app, ui, &xf, p) {
            let press_shift = ui
                .input(|inp| {
                    inp.events.iter().rev().find_map(|event| match event {
                        egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: true, modifiers, .. } => Some(modifiers.shift),
                        _ => None,
                    })
                })
                .unwrap_or(m.shift);
            ui.data_mut(|dd| dd.insert_temp(name_press_id(), (i, press_shift)));
        } else {
            begin_drag(app, ui, &f, p);
        }
    } else if drag.is_none()
        && resp.hovered()
        && pointer.button_pressed(egui::PointerButton::Middle)
        && let Some(start) = hover
    {
        // Middle-button drag pans the view whatever the tool.
        ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::Pan { start, center: v.center, middle: true }));
        app.session.tool_hover_ended(view);
    } else if let Some(d) = drag {
        // The pointer gone (a pen lifted with its press), the drag ends where it was last seen.
        let p = pointer.interact_pos().or_else(|| ui.data(|dd| dd.get_temp(drag_pos_id()))).unwrap_or(rect.center());
        let held = if middle_pan { pointer.button_down(egui::PointerButton::Middle) } else { pointer.primary_down() };
        if held {
            drag_held(app, ui, &f, d, p);
        } else {
            drag_released(app, ui, &f, d, p);
        }
    } else if let Some(p) = hover_tool
        && (pointer.is_moving() || mods_changed || !was_over || tool_switched || (context_changed && app.session.tool_wants_rehover()))
    {
        let ev = PointerEvent { kind: PointerKind::Move, pos: xf.to_doc(p), mods: mods(m, space), pressure: 1.0 };
        dispatch(app, &ev, view);
    }
    // The pointer left the canvas (for a panel, or out of the window) or went under a floating
    // panel, menu, popup or dialog, with no press the tool holds: the tool's hover ends, which no
    // hover event would clear (Alt's Distance Guides).
    if was_over && hover_tool.is_none() && !drag.is_some_and(tool_holds) {
        app.session.tool_hover_ended(view);
        ui.ctx().request_repaint();
    }
    // The tool works on another thread (a large selection's planar map): it checks each frame
    // whether the work is done, and then shows the highlight under the pointer.
    if app.session.tool_waiting() {
        if let Err(e) = app.session.tool_poll(view, false) {
            app.status(e.to_string());
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    if pointer.primary_released()
        && let Some((i, shift)) = ui.data_mut(|d| d.remove_temp::<(usize, bool)>(name_press_id()))
    {
        let release_shift = ui
            .input(|inp| {
                inp.events.iter().rev().find_map(|event| match event {
                    egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } => Some(modifiers.shift),
                    _ => None,
                })
            })
            .unwrap_or(m.shift);
        select_named_artboard(app, i, shift || release_shift);
    }
    if resp.double_clicked()
        && let Some(p) = at
    {
        double_click(app, ui, &f, p);
    }
    if drag.is_some() || pointer.is_moving() || mods_changed {
        ui.ctx().request_repaint();
    }
}

/// What the pointer handlers share for one frame.
struct Pointing<'a> {
    resp: &'a egui::Response,
    rect: egui::Rect,
    pointer: &'a egui::PointerState,
    m: egui::Modifiers,
    space: bool,
    xf: Xf,
    v: View,
    view: vectorcraft_engine::ViewInfo,
}

/// Zoom around the pointer, and scroll ([`wheel`]); a trackpad rotation turns the view. Returns the pointer, the
/// modifiers, whether Space is held and where the pointer hovers over the canvas.
fn wheel_view(
    app: &mut VectorcraftApp,
    ui: &Ui,
    resp: &egui::Response,
    rect: egui::Rect,
) -> (egui::PointerState, egui::Modifiers, bool, Option<Pos2>) {
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let wheel_zooms = app.session.prefs.zoom_with_mouse_wheel;
    let animate = animated_zoom(&app.session.prefs);
    let turn_id = egui::Id::unique("canvas-wheel-turn");
    let turn_before: WheelTurn = ui.data(|d| d.get_temp(turn_id)).unwrap_or_default();
    let mut turn = turn_before;
    let (pointer, m, space, (factor, scroll)) =
        ui.input(|i| (i.pointer.clone(), i.modifiers, i.key_down(egui::Key::Space), wheel(i, wheel_zooms, animate, line, rect.height(), &mut turn)));
    if turn != turn_before {
        ui.data_mut(|d| d.insert_temp(turn_id, turn));
    }
    if turn.glide != 0.0 {
        ui.ctx().request_repaint();
    }
    let hover = pointer.hover_pos().filter(|p| rect.contains(*p));
    // Zoom around the pointer, and scroll ([`wheel`]): fingers pinching on a touch screen may also
    // slide. Then turn the view about the pointer for a trackpad rotation ([`trackpad_turn`]). The
    // tool's events below take their document points from the view this moves to.
    if resp.hovered() {
        if (factor - 1.0).abs() > 1e-6
            && let (Some(p), Some(vm)) = (hover, app.view_mut())
        {
            zoom_about(vm, rect, p, vm.zoom * factor);
        }
        if scroll != egui::Vec2::ZERO {
            scroll_view(app, ui, scroll);
        }
        if let Some(p) = hover
            && let Some(deg) = trackpad_turn(ui, app.session.prefs.trackpad_rotate_view)
            && let Some(vm) = app.view_mut()
        {
            rotate_about(vm, rect, p, deg);
        }
    }
    (pointer, m, space, hover)
}

/// In Presentation Mode a click anywhere goes to the next artboard. The tool receives no press, drag or hover, and a
/// drag does not pan the view. A drag that began before the mode was turned on ends as usual.
fn present_step(app: &mut VectorcraftApp, ui: &Ui, f: &Pointing, drag: Option<Drag>, at: Option<Pos2>) {
    let (resp, pointer) = (f.resp, f.pointer);
    let pressed = drag.is_none() && pointer.primary_pressed() && resp.hovered() && at.is_some();
    // `press` is this frame's new press, or else the press held so far. A quick tap can be
    // pressed and released in one frame.
    let press = if pressed { Some(Drag::Present) } else { drag };
    if press.is_some() && !pointer.primary_down() {
        ui.data_mut(|dd| dd.remove::<Drag>(drag_id()));
        // A tap of several fingers leaves `Drag::Gesture` (`touch_gestures`), and its release
        // does not move the view to the next artboard.
        if press == Some(Drag::Present) && resp.clicked() {
            crate::shortcuts::presentation_step(app, true);
        }
    } else if pressed {
        ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::Present));
    }
}

/// A press on the canvas at `p` starts a gesture: a zoom, pan or turn of the view, or the tool's drag.
fn begin_drag(app: &mut VectorcraftApp, ui: &Ui, f: &Pointing, p: Pos2) {
    let Pointing { resp, rect, m, space, xf, v, view, .. } = *f;
    let tool = app.session.tool_id();
    // Held, Space is the Hand tool for the moment, Space+Shift is the Rotate View tool and
    // Cmd+Space is the Zoom tool (with Alt, zooming out), whatever the tool.
    let zoom_mode = if space { m.command } else { tool == "zoom" };
    let rotate_mode = if space { m.shift && !m.command } else { tool == "rotateView" };
    let pan_mode = if space { !m.command && !m.shift } else { tool == "hand" };
    let d = if crate::touch::finger_navigates_only() {
        // A finger (or a palm) once a pen has been seen: it navigates, it doesn't draw.
        Drag::Gesture
    } else if zoom_mode {
        if animated_zoom(&app.session.prefs) { Drag::ZoomScrub { start: p, held: 0.0, moved: false } } else { Drag::ZoomBox { start: p } }
    } else if pan_mode {
        Drag::Pan { start: p, center: v.center, middle: false }
    } else if rotate_mode {
        let c = rect.center();
        Drag::RotateView { start_angle: (p.y - c.y).atan2(p.x - c.x) as f64, start_rot: v.rotation, snaps: !space }
    } else {
        // Cmd with another tool drags with the selection tool used last (the session lends it).
        let ev = PointerEvent { kind: PointerKind::Down, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, app, true) };
        dispatch(app, &ev, view);
        crate::document_drag::begin(app, ui.ctx(), ev.pos);
        Drag::Tool
    };
    // Panning, zooming or turning the view takes the pointer from the tool's hover.
    if !tool_holds(d) {
        app.session.tool_hover_ended(view);
    }
    ui.data_mut(|dd| {
        dd.insert_temp(drag_id(), d);
        dd.insert_temp(drag_pos_id(), p);
        dd.insert_temp(drag_layer_id(), resp.layer_id);
        // A new press has not dragged yet, whatever an interrupted one left behind.
        dd.remove::<Point>(tool_drag_doc_id());
        dd.insert_temp(tool_press_doc_id(), xf.to_doc(p));
        // Space held at the press (Cmd+Space lends the Zoom tool) sizes the marquee until
        // it is let go ([`zoom_marquee_start`]).
        if matches!(d, Drag::ZoomBox { .. }) {
            dd.insert_temp(zoom_space_free_id(), !space);
        }
    });
}

/// A gesture held at `p`: the view follows, or the tool gets drag events.
fn drag_held(app: &mut VectorcraftApp, ui: &Ui, f: &Pointing, d: Drag, p: Pos2) {
    let Pointing { resp, rect, pointer, m, space, xf, view, .. } = *f;
    // Where the pointer was in the frame before: a zoom marquee moved with Space follows it.
    let before: Option<Pos2> = ui.data(|dd| dd.get_temp(drag_pos_id()));
    ui.data_mut(|dd| dd.insert_temp(drag_pos_id(), p));
    match d {
        Drag::Pan { start, center, .. } => {
            let ppp = ui.ctx().pixels_per_point();
            if let Some(vm) = app.view_mut() {
                vm.center = center;
                pan_pixels(vm, ppp, whole_pixels(p - start, ppp));
            }
        }
        Drag::RotateView { start_angle, start_rot, snaps } => {
            let c = rect.center();
            let a = (p.y - c.y).atan2(p.x - c.x) as f64;
            let mut deg = start_rot + (a - start_angle).to_degrees();
            if snaps && m.shift {
                deg = (deg / 15.0).round() * 15.0;
            }
            if let Some(vm) = app.view_mut() {
                vm.rotation = vectorcraft_geom::normalize_deg(deg);
            }
        }
        Drag::ZoomBox { start } => {
            let start = zoom_marquee_start(ui, start, before, p, space);
            ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::ZoomBox { start }));
            marquee(ui.painter(), egui::Rect::from_two_pos(start, p));
        }
        Drag::ZoomScrub { start, held, moved } => {
            let dt = ui.input(|i| i.unstable_dt).min(0.25);
            let (held, moved) = (held + dt, moved || (p - start).length() > SCRUB_SLOP);
            // Dragged sideways it zooms with the pointer, held still it zooms on (Alt: out).
            let exponent = if moved {
                f64::from(pointer.delta().x) * SCRUB_ZOOM
            } else if held > HOLD_DELAY {
                f64::from(dt) * HOLD_ZOOM * if m.alt { -1.0 } else { 1.0 }
            } else {
                0.0
            };
            if exponent != 0.0
                && let Some(vm) = app.view_mut()
            {
                zoom_about(vm, rect, start, vm.zoom * exponent.exp());
            }
            ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::ZoomScrub { start, held, moved }));
        }
        // Without a pointer it is over a floating panel's own OS window (`drag_art_out`).
        Drag::Art if pointer.has_pointer() && on_canvas(ui, resp, p) => {
            // Back on the canvas: the move goes on from where it left, with the pointer.
            egui::DragAndDrop::clear_payload(ui.ctx());
            let ev = PointerEvent { kind: PointerKind::Drag, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, app, false) };
            dispatch(app, &ev, view);
            ui.data_mut(|dd| {
                dd.insert_temp(drag_id(), Drag::Tool);
                dd.insert_temp(drag_pos_id(), p);
            });
        }
        Drag::Art | Drag::Gesture | Drag::Present => {}
        Drag::Tool if drag_art_out(app, ui, resp, p) => {
            ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::Art));
        }
        Drag::Tool => {
            // Every position the pen or mouse passed this frame, not just the last, for a
            // tool that builds a stroke from them: a fast stroke keeps its curve (a pen or a
            // mouse reports far more often than frames come). Touch events (a pen on Windows
            // or the web, a finger) and a macOS pen's moves carry their own pressure; a
            // mouse's, a pen's on X11 among them, take the host's latest. Other tools get
            // the last position only, so a costly drag preview runs once a frame.
            let moves = tool_frame_moves(ui, app);
            // A frame whose pointer ends where the last one did still moved in between.
            if tool_drag_moved(ui, &xf, p, pointer) || !moves.is_empty() {
                for (at, force) in moves {
                    let pressure = force.unwrap_or_else(|| pen_pressure(ui, app, false));
                    dispatch(app, &PointerEvent { kind: PointerKind::Drag, pos: xf.to_doc(at), mods: mods(m, space), pressure }, view);
                }
                let ev = PointerEvent { kind: PointerKind::Drag, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, app, false) };
                dispatch(app, &ev, view);
            }
            // Time held (Twirl, Pucker and Bloat keep applying); a stalled frame counts
            // a quarter second at most.
            if app.session.tool_wants_ticks() {
                let dt = f64::from(ui.input(|i| i.unstable_dt).min(0.25));
                let r = app.session.tool_tick(dt, view);
                apply_requests(app, r);
            }
        }
    }
}

/// A gesture released at `p`: the view settles, or the tool gets its up.
fn drag_released(app: &mut VectorcraftApp, ui: &Ui, f: &Pointing, d: Drag, p: Pos2) {
    let Pointing { resp, rect, m, space, xf, view, .. } = *f;
    // The last position the tool's drag got: none if the press never dragged.
    let last_drag: Option<Point> = ui.data(|dd| dd.get_temp(tool_drag_doc_id()));
    // Where the pointer was in the frame before: a zoom marquee moved with Space follows it.
    let before: Option<Pos2> = ui.data(|dd| dd.get_temp(drag_pos_id()));
    ui.data_mut(|dd| {
        dd.remove::<Drag>(drag_id());
        dd.remove::<Pos2>(drag_pos_id());
        dd.remove::<Point>(tool_drag_doc_id());
        dd.remove::<Point>(tool_press_doc_id());
    });
    match d {
        Drag::ZoomBox { start } => {
            let start = zoom_marquee_start(ui, start, before, p, space);
            ui.data_mut(|dd| dd.remove::<bool>(zoom_space_free_id()));
            let r = egui::Rect::from_two_pos(start, p);
            if let Some(vm) = app.view_mut() {
                if r.width() > 8.0 && r.height() > 8.0 {
                    // The marquee's center and its size on screen set the view, at any rotation.
                    vm.center = xf.to_doc(r.center());
                    let fill = (rect.width() / r.width()).min(rect.height() / r.height());
                    vm.zoom = (xf.zoom * f64::from(fill)).clamp(0.0313, 640.0);
                } else {
                    zoom_about(vm, rect, p, crate::state::next_zoom(vm.zoom, !m.alt));
                }
            }
        }
        // A click (not held, not dragged) steps the zoom as with the marquee.
        Drag::ZoomScrub { held, moved: false, .. } if held <= HOLD_DELAY => {
            if let Some(vm) = app.view_mut() {
                zoom_about(vm, rect, p, crate::state::next_zoom(vm.zoom, !m.alt));
            }
        }
        Drag::ZoomScrub { .. } => {}
        Drag::Tool => {
            // The second click of a double-click: don't fit another artboard on this release.
            // A double-click on an object does not open Artboard Options. Empty artboard
            // space still does.
            if resp.double_clicked() && app.session.tool_id() == "artboard" {
                app.session.set_tool_option("suppressFitClick", &json!(true));
            }
            // A move that came in the release's own frame shapes the preview first: the
            // tools commit the last preview, which should be where the pointer let go
            // (#834). Only once the press has dragged, so a click's jitter stays a click
            // (the Pen's corner anchor, no nudge with Direct Selection).
            if let Some(last) = last_drag
                && last != xf.to_doc(p)
            {
                let ev = PointerEvent { kind: PointerKind::Drag, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, app, false) };
                dispatch(app, &ev, view);
            }
            let ev = PointerEvent { kind: PointerKind::Up, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, app, false) };
            dispatch(app, &ev, view);
        }
        Drag::Art if on_canvas(ui, resp, p) => {
            // Came back to the canvas and released in one frame: finish the move here.
            egui::DragAndDrop::clear_payload(ui.ctx());
            let pressure = pen_pressure(ui, app, false);
            let pos = xf.to_doc(p);
            dispatch(app, &PointerEvent { kind: PointerKind::Drag, pos, mods: mods(m, space), pressure }, view);
            dispatch(app, &PointerEvent { kind: PointerKind::Up, pos, mods: mods(m, space), pressure }, view);
        }
        Drag::Art => {
            // Released over a panel: the move goes back. (The app ends it before the panels
            // see the release, so a panel can take the art; this is the headless path.)
            // With no document open there is nothing to roll back; the tool still gets its up.
            let _ = app.session.cancel_interaction();
            dispatch(app, &PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), pressure: 1.0 }, view);
        }
        Drag::Gesture | Drag::Present => {}
        Drag::Pan { .. } | Drag::RotateView { .. } => {}
    }
}

/// A double-click at `p`: an artboard's name edits, else the tool gets it.
fn double_click(app: &mut VectorcraftApp, ui: &Ui, f: &Pointing, p: Pos2) {
    let Pointing { m, space, xf, view, .. } = *f;
    if let Some(i) = artboard_name_at(app, ui, &xf, p) {
        begin_artboard_name_edit(ui, app, i);
    } else {
        let ev = PointerEvent { kind: PointerKind::DoubleClick, pos: xf.to_doc(p), mods: mods(m, space), pressure: 1.0 };
        let before = app.session.tool_id().to_string();
        dispatch(app, &ev, view);
        // A double-click on type switched a selection tool to the Type tool: the caret goes where
        // the type was clicked, so typing edits it at once.
        if before != "type" && app.session.tool_id() == "type" {
            app.session.set_tool_option("prevTool", &json!(before));
            for kind in [PointerKind::Down, PointerKind::Up] {
                dispatch(app, &PointerEvent { kind, ..ev }, view);
            }
        }
    }
}

/// Whether Space moves the Zoom tool's marquee in the drag under way ([`zoom_marquee_start`]).
fn zoom_space_free_id() -> egui::Id {
    egui::Id::unique("canvas-zoom-space-free")
}

/// The start corner of the Zoom tool's marquee in this frame. With Space held, the marquee moves
/// as far as the pointer did since `before` and keeps its size; with Space let go, the drag sizes
/// it again from where it moved to. Space held since the press (Cmd+Space lends the Zoom tool)
/// sizes the marquee until it is let go. A marquee with no size yet is sized by the movement, so
/// Space pressed before the pointer moves does not keep it at the press point. The frame's pointer
/// moves and Space presses and releases are taken in the order they came, so each move follows the
/// Space state it came with.
fn zoom_marquee_start(ui: &Ui, start: Pos2, before: Option<Pos2>, p: Pos2, space: bool) -> Pos2 {
    enum Step {
        Space(bool),
        Move(Pos2),
    }
    let id = zoom_space_free_id();
    let steps: Vec<Step> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key: egui::Key::Space, pressed, repeat: false, .. } => Some(Step::Space(*pressed)),
                egui::Event::PointerMoved(q) => Some(Step::Move(*q)),
                _ => None,
            })
            .collect()
    });
    // Space as it was when the frame began: the opposite of its first press or release, else as it is now.
    let mut held = steps.iter().find_map(|s| if let Step::Space(down) = s { Some(!down) } else { None }).unwrap_or(space);
    let mut free = !held || ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    let (mut start, mut at) = (start, before);
    for s in steps.into_iter().chain(std::iter::once(Step::Move(p))) {
        match s {
            Step::Space(down) => {
                held = down;
                free |= !down;
            }
            Step::Move(q) => {
                if let Some(b) = at
                    && held
                    && free
                    && b != start
                {
                    start += q - b;
                }
                at = Some(q);
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(id, free));
    start
}

/// A marquee around screen rectangle `r`: dark dashes over a light line, so it shows on a white
/// artboard and on the grey pasteboard alike (#725).
fn marquee(p: &egui::Painter, r: egui::Rect) {
    marquee_through(p, &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]);
}

/// A [`marquee`] through the screen corners `q` in turn, back to the first: a document rectangle's
/// [`Xf::quad`] in a rotated view.
pub(super) fn marquee_through(p: &egui::Painter, q: &[Pos2]) {
    let pts: Vec<Pos2> = q.iter().chain(q.first()).copied().collect();
    p.add(Shape::line(pts.clone(), Stroke::new(1.0, Color32::from_white_alpha(230))));
    for w in pts.windows(2) {
        p.extend(Shape::dashed_line(&[w[0], w[1]], Stroke::new(1.0, Color32::from_gray(40)), 3.0, 3.0));
    }
}
