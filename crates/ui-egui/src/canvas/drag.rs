use super::*;

/// Whether drag `d` is a press the tool holds. Every other drag (panning, zooming, turning the
/// view, a touch gesture, Presentation Mode) never reaches the tool, so its hover ends with it.
pub(super) fn tool_holds(d: Drag) -> bool {
    matches!(d, Drag::Tool | Drag::Art)
}

/// What a hover depends on besides the pointer's screen position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HoverContext {
    /// The active tool: a tool switched to under a still pointer is hovered at once.
    pub(super) tool: &'static str,
    /// The canvas, and the view's centre, zoom and rotation.
    pub(super) canvas: egui::Rect,
    pub(super) center: Point,
    pub(super) zoom: f64,
    pub(super) rotation: f64,
    /// The active document's id and revision (which moves with every edit, undo and selection
    /// change).
    pub(super) doc: Option<(u64, u64)>,
}

/// Clear canvas interaction state before a tab transfer takes ownership of the artwork drag.
pub(crate) fn end_document_drag(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<Drag>(drag_id());
        d.remove::<Pos2>(drag_pos_id());
    });
}

/// Temp-memory key for the active canvas drag state.
pub(super) fn drag_id() -> egui::Id {
    egui::Id::unique("canvas-drag")
}

/// Where the pointer was last seen during a press on the canvas.
pub(super) fn drag_pos_id() -> egui::Id {
    egui::Id::unique("canvas-drag-pos")
}

/// The layer the canvas was last drawn on: a release over another layer is over a panel (#2826).
pub(super) fn drag_layer_id() -> egui::Id {
    egui::Id::unique("canvas-drag-layer")
}

/// Whether `p` is over the canvas itself, not a panel or a floating window over it.
pub(super) fn on_canvas(ui: &Ui, resp: &egui::Response, p: Pos2) -> bool {
    resp.rect.contains(p) && ui.clip_rect().contains(p) && ui.ctx().layer_id_at(p).is_none_or(|l| l == resp.layer_id)
}

/// The document point last sent to the tool as a drag during [`Drag::Tool`]; none until the press
/// has dragged.
pub(super) fn tool_drag_doc_id() -> egui::Id {
    egui::Id::unique("canvas-tool-drag-doc")
}

/// The document point of the press that began [`Drag::Tool`], sent to the tool as its Down.
pub(super) fn tool_press_doc_id() -> egui::Id {
    egui::Id::unique("canvas-tool-press-doc")
}

/// Whether the tool should get another drag event, and if so records `p` as the last position
/// sent: the pointer moved this frame, or its document position changed without a delta (#834).
/// egui's frame delta is zero when some Wayland compositors coalesce motion until release, and in
/// the frame the pointer comes back into the window after leaving it (`PointerGone` forgot where
/// it was), though it is somewhere new; judged on the delta alone, the preview stayed behind.
pub(super) fn tool_drag_moved(ui: &Ui, xf: &Xf, p: Pos2, pointer: &egui::PointerState) -> bool {
    let at = xf.to_doc(p);
    let id = tool_drag_doc_id();
    // Before the press has dragged, the last position the tool got is the press itself: a press
    // held still is not a drag.
    let prev = ui.data(|d| d.get_temp::<Point>(id).or_else(|| d.get_temp::<Point>(tool_press_doc_id())));
    if pointer.delta() == egui::Vec2::ZERO && prev == Some(at) {
        return false;
    }
    ui.data_mut(|d| d.insert_temp(id, at));
    true
}

/// The pen pressure (0..1) of the press in progress: the force of this frame's pen or touch
/// input, else the pressure in the host's feed while a pen is the pointer that moved last
/// ([`crate::Services::stylus`]: [`StylusFeed::press`] for a new press, [`StylusFeed::held`]
/// after it), else the last one seen since the press (`pressed`: a new press, whose mouse has none
/// until a pen reports it). A mouse presses fully (1).
pub(super) fn pen_pressure(ui: &Ui, app: &VectorcraftApp, pressed: bool) -> f32 {
    let stylus = app.services.stylus.as_ref();
    let id = egui::Id::unique("canvas-pressure");
    let force = ui.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { force: Some(f), .. } if f.is_finite() => Some(f.clamp(0.0, 1.0)),
            _ => None,
        })
    });
    let force = force.or_else(|| stylus.and_then(if pressed { StylusFeed::press } else { StylusFeed::held }));
    let p = force.or_else(|| if pressed { None } else { ui.data(|d| d.get_temp::<f32>(id)) }).unwrap_or(1.0);
    ui.data_mut(|d| d.insert_temp(id, p));
    p
}

/// The positions this frame's pointer moves passed through before its last one, each with the
/// pen force a touch event gave just before it, or the pressure the host's feed kept for that move
/// ([`StylusFeed::take_moves`], macOS), if any. The last move is the pointer's position. A mouse's
/// moves count too, with no pressure: a fast mouse stroke otherwise keeps one position a frame, a
/// sample every few pixels, too few for a low Fidelity to follow the hand (#3328).
pub(super) fn frame_moves(ui: &Ui, stylus: Option<&StylusFeed>) -> Vec<(Pos2, Option<f32>)> {
    let pressures = stylus.map(StylusFeed::take_moves).unwrap_or_default();
    ui.input(|i| {
        let mut moves = vec![];
        if i.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })) {
            let mut force = None;
            for e in &i.events {
                match e {
                    egui::Event::Touch { force: Some(f), .. } if f.is_finite() && *f > 0.0 => force = Some(f.clamp(0.0, 1.0)),
                    egui::Event::PointerMoved(p) => moves.push((*p, force)),
                    _ => {}
                }
            }
        } else if !pressures.is_empty() {
            // The feed's last pressure is the last move's: pair them from the end. A lift's 0 is
            // no pressure (the press keeps its last touch).
            let points: Vec<Pos2> = i.events.iter().filter_map(|e| if let egui::Event::PointerMoved(p) = e { Some(*p) } else { None }).collect();
            let forces = pressures.iter().rev().map(|f| (*f > 0.0).then_some(*f)).chain(std::iter::repeat(None));
            moves = points.into_iter().rev().zip(forces).collect();
            moves.reverse();
        } else {
            moves = i.events.iter().filter_map(|e| if let egui::Event::PointerMoved(p) = e { Some((*p, None)) } else { None }).collect();
        }
        moves.pop();
        // A burst of events (a stalled frame) is thinned to a bounded count.
        const MAX: usize = 64;
        if moves.len() > MAX {
            let step = moves.len().div_ceil(MAX);
            moves = moves.into_iter().step_by(step).collect();
        }
        moves
    })
}

/// The moves of this frame ([`frame_moves`]) the active tool gets before the pointer's position:
/// all of them for a tool that builds a stroke from them ([`vectorcraft_engine::Session::tool_wants_samples`]), none
/// for the others, which get one drag a frame. The moves are read either way, so the host's feed
/// pairs the next frame's with its own.
pub(super) fn tool_frame_moves(ui: &Ui, app: &VectorcraftApp) -> Vec<(Pos2, Option<f32>)> {
    let moves = frame_moves(ui, app.services.stylus.as_ref());
    if app.session.tool_wants_samples() { moves } else { vec![] }
}

pub fn mods(m: egui::Modifiers, space: bool) -> Mods {
    Mods { shift: m.shift, alt: m.alt, cmd: m.command, ctrl: m.ctrl, space }
}

/// Drop the tool press in progress, if any: nothing it began is kept, and the rest of the press
/// does nothing ([`Drag::Gesture`]).
pub(super) fn drop_press(app: &mut VectorcraftApp, ui: &Ui, view: vectorcraft_engine::ViewInfo) {
    match ui.data(|d| d.get_temp::<Drag>(drag_id())) {
        Some(Drag::Tool | Drag::Art) => {
            // The tool's mouse-up returns it to rest. A move waiting over a panel is still live
            // and would otherwise outlive the drag that owns it (#2826).
            // Nothing to roll back without a document; the tool still gets its mouse-up.
            let _ = app.session.cancel_interaction();
            dispatch(app, &PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), pressure: 1.0 }, view);
            egui::DragAndDrop::clear_payload(ui.ctx());
        }
        Some(_) => {}
        None => return,
    }
    ui.data_mut(|d| d.insert_temp(drag_id(), Drag::Gesture));
}

/// A Selection tool move dragged off the canvas (to `p`, over a panel) becomes a panel drag of
/// the selected art ([`widgets::PanelDrag::Art`]): the art waits where it last moved on the
/// canvas, and a panel it is released on takes it (the Graphic Styles panel makes a style of it).
/// Back on the canvas, [`Drag::Art`] takes the move up again where the pointer is (#2826).
pub(super) fn drag_art_out(app: &VectorcraftApp, ui: &Ui, resp: &egui::Response, p: Pos2) -> bool {
    let Some(st) = app.session.active().filter(|_| app.session.tool_id() == "selection") else { return false };
    // The Selection tool's interaction while it moves (or Alt-copies) the selection.
    if !st.interaction.as_ref().is_some_and(|i| i.label == "Move" || i.label == "Copy") {
        return false;
    }
    // A floating panel in an OS window of its own takes the art too: craft-ui hands that window
    // the pointer while it is over it, and this window sees the pointer go.
    craft_ui::docking::route_app_drag(ui.ctx(), crate::panel_docking::area_id());
    if on_canvas(ui, resp, p) && ui.input(|i| i.pointer.has_pointer()) {
        return false;
    }
    let ids = st.selection.objects.clone();
    egui::DragAndDrop::set_payload(ui.ctx(), widgets::PanelDrag::Art(ids));
    true
}

/// A Selection tool move released over a panel: end it before the panels see the release, so a
/// panel that takes the art ([`widgets::PanelDrag::Art`]) takes it where it was, and no move is
/// left open (#2826). The art stays where it last moved while the pointer is over the panel.
pub(crate) fn release_over_panel(app: &mut VectorcraftApp, ctx: &egui::Context) {
    crate::document_drag::cleanup(app, ctx);
    let art = matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Art));
    // Over a floating panel's own OS window the pointer is gone from this window, and the release
    // comes here later, at a point of this window that may lie on the canvas: that panel takes it.
    let gone_id = drag_id().with("art-gone");
    if !ctx.input(|i| i.pointer.primary_released()) {
        if art {
            let gone = ctx.input(|i| !i.pointer.has_pointer());
            ctx.data_mut(|d| d.insert_temp(gone_id, gone));
        }
        return;
    }
    let gone = ctx.data_mut(|d| d.remove_temp::<bool>(gone_id)).unwrap_or(false);
    let p = ctx.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
    if !art {
        return;
    }
    let layer = ctx.data(|d| d.get_temp::<egui::LayerId>(drag_layer_id()));
    let on = !gone && p.is_some_and(|p| app.canvas_rect.is_some_and(|r| r.contains(p)) && ctx.layer_id_at(p).is_none_or(|l| Some(l) == layer));
    if on {
        // The canvas's release finishes the move.
        return;
    }
    end_art_move(app, ctx);
}

/// A release in a floating panel's own OS window, during an art drag from the canvas: the move
/// ends before that panel takes the art. The main window would only see it on its next pass.
pub(crate) fn release_art_in_window(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let art = matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Art));
    if art && ctx.input(|i| i.pointer.primary_released()) {
        end_art_move(app, ctx);
    }
}

/// Roll back the art move dragged off the canvas; a panel takes the art instead.
fn end_art_move(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // With no document open there is nothing to roll back; the tool still gets its mouse-up.
    let _ = app.session.cancel_interaction();
    let view = app.view_info();
    dispatch(app, &PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), pressure: 1.0 }, view);
    ctx.data_mut(|d| {
        d.remove::<Drag>(drag_id());
        d.remove::<bool>(drag_id().with("art-gone"));
    });
}

/// Send a pointer event to the active tool and act on UI requests (dialogs, tool switches).
pub fn dispatch(app: &mut VectorcraftApp, ev: &PointerEvent, view: vectorcraft_engine::ViewInfo) {
    let r = app.session.pointer(ev, view);
    // The artboard the Artboard tool makes active is the active one (the navigator's, the
    // Artboards panel's).
    if app.session.tool_id() == "artboard"
        && let Some(i) = app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok())
        && let Some(v) = app.view_mut()
    {
        v.artboard = i;
    }
    // A press with a selection tool inside an artboard makes it the active one (#693), as the
    // Artboard tool's does: Paste in Place and in Front or Back then paste onto it, Align to
    // Artboard aligns to it, and All on Active Artboard selects on it.
    if ev.kind == PointerKind::Down
        && vectorcraft_tools::catalog::is_selection_tool(app.session.tool_id())
        && let Some(i) = app.session.active().and_then(|d| d.doc.artboard_at(ev.pos))
        && let Some(v) = app.view_mut()
    {
        v.artboard = i;
    }
    apply_requests(app, r);
}

/// Act on what a tool event asked the UI for (dialogs, tool switches), or show its error.
pub fn apply_requests(app: &mut VectorcraftApp, r: vectorcraft_engine::Result<Vec<vectorcraft_engine::UiRequest>>) {
    match r {
        Ok(reqs) => {
            for r in reqs {
                match r {
                    vectorcraft_engine::UiRequest::Dialog(kind, p) => crate::dialogs::open_tool_dialog(app, &kind, p),
                    vectorcraft_engine::UiRequest::SwitchTool(t) => app.select_tool(&t),
                    vectorcraft_engine::UiRequest::Status(msg) => app.status(msg),
                    vectorcraft_engine::UiRequest::Place(p) => crate::place::place_from_cursor(app, &p),
                }
            }
        }
        Err(e) => app.status(e.to_string()),
    }
}
