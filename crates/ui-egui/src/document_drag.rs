//! A selection dragged across document tabs, or into another document's tiled pane or floating
//! window, remains editable and leaves its source intact.

use std::sync::Arc;

use egui::{Context, Id, Response};
use serde_json::json;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::Point;

use crate::{VectorcraftApp, canvas, widgets};

const HOVER_SECONDS: f64 = 0.4;

#[derive(Clone)]
struct Origin {
    uid: u64,
    grab: Point,
}

#[derive(Clone)]
struct Artwork {
    origin: Origin,
    ids: Vec<NodeId>,
    /// The source document when the drag left it. The drag ends when the source's content differs
    /// from it ([`vectorcraft_engine::DocState::same_content`]). A redraw that raises its revision
    /// (another document's assigned color profiles brought to the front) and a change that is not
    /// an edit (its type measured again) keep it.
    source: Arc<Document>,
    target: u64,
}

#[derive(Clone)]
struct Hover {
    uid: u64,
    /// The hovered tab's view of its document (Window › New Window).
    view: u32,
    since: f64,
}

/// Key for the source document and original canvas grab point.
fn origin_id() -> Id {
    Id::unique("document-drag-origin")
}
/// Key for the tab identity and dwell start time of the current hover.
fn hover_id() -> Id {
    Id::unique("document-drag-hover")
}
/// Key for the pointer position latched after tab activation to prevent cascading switches.
fn activated_at_id() -> Id {
    Id::unique("document-drag-activated-at")
}

/// Whether a Selection drag is carrying artwork across the document tabs, which then switch
/// documents and never move along the strip.
pub(crate) fn carrying_artwork(ctx: &Context) -> bool {
    egui::DragAndDrop::payload::<Artwork>(ctx).is_some()
}

/// Record the Selection drag source and grab point, clearing any previous tab-hover timer.
pub(crate) fn begin(app: &VectorcraftApp, ctx: &Context, grab: Point) {
    ctx.data_mut(|d| {
        d.remove::<Origin>(origin_id());
        d.remove::<Hover>(hover_id());
        d.remove::<egui::Pos2>(activated_at_id());
        if app.session.tool_id() == "selection"
            && let Some(st) = app.session.active()
        {
            d.insert_temp(origin_id(), Origin { uid: st.uid, grab });
        }
    });
}

/// Called before panels. Cancelled or abandoned transfers must never reach a later mouse-up.
pub(crate) fn cleanup(app: &VectorcraftApp, ctx: &Context) {
    // Activating a tab can scroll the strip or change title widths. A tab moving under a
    // stationary pointer must not start another hover; actual pointer movement rearms it.
    let at = ctx.input(|i| i.pointer.interact_pos());
    ctx.data_mut(|d| {
        if d.get_temp::<egui::Pos2>(activated_at_id()) != at {
            d.remove::<egui::Pos2>(activated_at_id());
        }
    });
    if let Some(art) = egui::DragAndDrop::payload::<Artwork>(ctx) {
        let valid = app.session.documents().iter().any(|d| d.uid == art.origin.uid && d.same_content(&art.source))
            && app.session.active().is_some_and(|d| d.uid == art.target);
        let cancel = ctx.input(|i| {
            !i.focused
                || i.key_pressed(egui::Key::Escape)
                || (!i.pointer.primary_down() && !i.pointer.primary_released())
                || i.multi_touch().is_some()
                || i.events.iter().any(|e| matches!(e, egui::Event::Touch { phase: egui::TouchPhase::Cancel, .. }))
        });
        // The canvas and the other windows' tiled panes take a release ([`drop_on_pane`]).
        let target = |p: egui::Pos2| app.canvas_rect.is_some_and(|r| r.contains(p)) || app.canvas.panes.last.iter().any(|(_, r)| r.contains(p));
        let outside_release = ctx.input(|i| i.pointer.primary_released() && !released_at(i).is_some_and(target));
        if !valid || cancel || outside_release || app.ui.dialog.is_some() {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }
    // The source and grab point last through the release, for the pane it is released over.
    if !ctx.input(|i| i.pointer.primary_down() || i.pointer.primary_released()) {
        ctx.data_mut(|d| {
            d.remove::<Origin>(origin_id());
            d.remove::<Hover>(hover_id());
            d.remove::<egui::Pos2>(activated_at_id());
        });
    }
}

/// Hover uses `contains_pointer`: the source canvas still owns egui's dragged widget. The tab is
/// window `win`'s: another view of the current document's is a tab to switch to as well.
pub(crate) fn hover_tab(app: &mut VectorcraftApp, resp: &Response, win: crate::doc_windows::Win, close: egui::Rect) -> bool {
    let ctx = &resp.ctx;
    let (index, view) = win;
    let eligible = resp.contains_pointer()
        && ctx
            .input(|i| i.pointer.primary_down() && !i.key_pressed(egui::Key::Escape) && i.pointer.interact_pos().is_some_and(|p| !close.contains(p)))
        && app.ui.dialog.is_none();
    if !eligible || crate::doc_windows::current(app) == Some(win) || ctx.data(|d| d.get_temp::<egui::Pos2>(activated_at_id()).is_some()) {
        return false;
    }
    let transfer = egui::DragAndDrop::has_payload_of_type::<Artwork>(ctx);
    let art = egui::DragAndDrop::payload::<widgets::PanelDrag>(ctx).is_some_and(|p| matches!(*p, widgets::PanelDrag::Art(_)));
    if !transfer && !art {
        return false;
    }
    let Some(uid) = app.session.documents().get(index).map(|d| d.uid) else {
        return false;
    };
    let now = ctx.input(|i| i.time);
    let hover = ctx.data(|d| d.get_temp::<Hover>(hover_id())).filter(|h| h.uid == uid && h.view == view).unwrap_or(Hover { uid, view, since: now });
    ctx.data_mut(|d| d.insert_temp(hover_id(), hover.clone()));
    let remaining = HOVER_SECONDS - (now - hover.since);
    if remaining > 0.0 {
        ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(remaining).unwrap_or(std::time::Duration::from_millis(400)));
        return true;
    }
    if !transfer {
        let Some(origin) = ctx.data(|d| d.get_temp::<Origin>(origin_id())) else {
            return true;
        };
        if !app
            .session
            .active()
            .is_some_and(|d| d.uid == origin.uid && d.interaction.as_ref().is_some_and(|i| i.label == "Move" || i.label == "Copy"))
        {
            return true;
        }
        if let Err(e) = app.session.cancel_interaction() {
            app.status(e.to_string());
            return true;
        }
        // Alt-drag previews use temporary duplicate ids; only the restored originals travel.
        let Some((ids, source)) = app.session.active().map(|d| (d.selection.objects.clone(), d.doc.clone())) else {
            return true;
        };
        if ids.is_empty() {
            return true;
        }
        canvas::end_document_drag(ctx);
        egui::DragAndDrop::set_payload(ctx, Artwork { origin, ids, source, target: uid });
    }
    if transfer && let Some(art) = egui::DragAndDrop::payload::<Artwork>(ctx) {
        let mut art = (*art).clone();
        art.target = uid;
        egui::DragAndDrop::set_payload(ctx, art);
    }
    app.ui.home = None;
    crate::doc_windows::show_window(app, win);
    ctx.request_repaint();
    let at = ctx.input(|i| i.pointer.interact_pos());
    ctx.data_mut(|d| {
        d.remove::<Hover>(hover_id());
        if let Some(at) = at {
            d.insert_temp(activated_at_id(), at);
        }
    });
    true
}

/// Reset the dwell timer when no eligible inactive tab is under the artwork.
pub(crate) fn finish_tabs(ctx: &Context, hovering: bool) {
    if !hovering {
        ctx.data_mut(|d| d.remove::<Hover>(hover_id()));
    }
}

/// Release artwork into the active canvas; returning to the source resumes Move or Alt-copy.
pub(crate) fn drop_on_canvas(app: &mut VectorcraftApp, resp: &Response, xf: &canvas::Xf) {
    let Some(payload) = egui::DragAndDrop::payload::<Artwork>(&resp.ctx) else {
        return;
    };
    let same = app.session.active().is_some_and(|d| d.uid == payload.origin.uid);
    let copy = resp.ctx.input(|i| i.modifiers.alt);
    if resp.contains_pointer() {
        resp.ctx.set_cursor_icon(if same && !copy { egui::CursorIcon::Move } else { egui::CursorIcon::Copy });
    }
    // The canvas takes a release where the button went up, as [`cleanup`] judges it, and not where
    // a later move in the same frame leaves the pointer.
    let ctx = &resp.ctx;
    let over = |p: egui::Pos2| resp.interact_rect.contains(p) && !ctx.layer_id_at(p).is_some_and(|l| l != resp.layer_id);
    if let Some(p) = ctx.input(|i| i.pointer.primary_released().then(|| released_at(i)).flatten()).filter(|p| over(*p))
        && let Some(art) = egui::DragAndDrop::take_payload::<Artwork>(ctx)
    {
        let at = xf.to_doc(p);
        let result = if same {
            // Back in the source tab, resume the ordinary move (Alt copies).
            let copy = release_alt(&resp.ctx);
            app.run("object.move", json!({"ids": art.ids, "dx": at.x-art.origin.grab.x, "dy": at.y-art.origin.grab.y, "copy": copy}))
        } else {
            app.run(
                "edit.dropSelection",
                json!({"sourceDocument": art.origin.uid, "ids": art.ids, "grab": [art.origin.grab.x, art.origin.grab.y], "at": [at.x, at.y]}),
            )
        };
        if let Err(e) = result {
            app.status(e);
        }
    }
}

/// The artwork a Selection drag carries out of its canvas, taken by another window's tiled pane
/// ([`take_for_pane`]): its source and grab point, and the objects that travel.
pub(crate) struct Carried {
    origin: Origin,
    ids: Vec<NodeId>,
}

/// The artwork a Selection drag carries: the transfer a tab hover started, or a Selection move of
/// the current document that left its canvas. The canvas hands that move over as a panel drag
/// ([`widgets::PanelDrag::Art`]) the frame after the pointer leaves, and Escape drops that; until
/// then the move under way is the drag. Either way the objects the move started with travel, not
/// an Alt-drag's preview copies.
fn carried(app: &VectorcraftApp, ctx: &Context) -> Option<Carried> {
    if ctx.input(|i| !i.focused || i.multi_touch().is_some() || i.key_pressed(egui::Key::Escape)) || app.ui.dialog.is_some() {
        return None;
    }
    if let Some(art) = egui::DragAndDrop::payload::<Artwork>(ctx) {
        // The same checks as [`cleanup`]: a floating window's pane takes the release before cleanup
        // runs, and an edited or closed source, or a target that is no longer the current
        // document, carries nothing.
        let source = app.session.documents().iter().any(|d| d.uid == art.origin.uid && d.same_content(&art.source));
        let target = app.session.active().is_some_and(|d| d.uid == art.target);
        return (source && target).then(|| Carried { origin: art.origin.clone(), ids: art.ids.clone() });
    }
    let origin = ctx.data(|d| d.get_temp::<Origin>(origin_id()))?;
    let st = app.session.active().filter(|d| d.uid == origin.uid)?;
    let ids = match egui::DragAndDrop::payload::<widgets::PanelDrag>(ctx).as_deref() {
        Some(widgets::PanelDrag::Art(ids)) => ids.clone(),
        Some(_) => return None,
        None if canvas::art_out(ctx) => return None,
        None => st.interaction.as_ref().filter(|i| i.label == "Move" || i.label == "Copy")?.selection.objects.clone(),
    };
    (!ids.is_empty()).then_some(Carried { origin, ids })
}

/// Over the tiled pane of a window of document `uid`, while a Selection drag carries artwork: the
/// cursor that shows a move for another view of the source document without Alt, and a copy
/// otherwise. None when no artwork is carried.
pub(crate) fn pane_cursor(app: &VectorcraftApp, ctx: &Context, uid: u64) -> Option<egui::CursorIcon> {
    let c = carried(app, ctx)?;
    let copy = ctx.input(|i| i.modifiers.alt);
    Some(if c.origin.uid == uid && !copy { egui::CursorIcon::Move } else { egui::CursorIcon::Copy })
}

/// A release over another window's tiled pane takes the carried artwork: the drag ends, and a
/// move still under way on the canvas goes back to where it started, before the canvas sees the
/// release ([`canvas::end_art_move`]).
pub(crate) fn take_for_pane(app: &mut VectorcraftApp, ctx: &Context) -> Option<Carried> {
    let c = carried(app, ctx)?;
    let moving = !egui::DragAndDrop::has_payload_of_type::<Artwork>(ctx) && app.session.in_interaction();
    egui::DragAndDrop::clear_payload(ctx);
    ctx.data_mut(|d| d.remove::<Origin>(origin_id()));
    if moving {
        canvas::end_art_move(app, ctx);
    }
    Some(c)
}

/// The artwork `c` taken over window `w`'s tiled pane ([`take_for_pane`]), released at document
/// point `at` in that pane's view: `w` becomes the current window, and the artwork lands with its
/// grab point at `at`. Into another document it is copied (`edit.dropSelection`), and the source
/// stays as it was; in another view of the source document it moves, and Alt copies it
/// (`object.move`). When the command fails, the window that was current stays current and the
/// status bar shows the error.
pub(crate) fn drop_on_pane(app: &mut VectorcraftApp, ctx: &Context, w: crate::doc_windows::Win, at: Point, c: Carried) {
    let Carried { origin, ids } = c;
    let Some(uid) = app.session.documents().get(w.0).map(|d| d.uid) else { return };
    let before = crate::doc_windows::current(app);
    app.ui.home = None;
    crate::doc_windows::show_window(app, w);
    let result = if uid == origin.uid {
        let copy = release_alt(ctx);
        app.run("object.move", json!({"ids": ids, "dx": at.x - origin.grab.x, "dy": at.y - origin.grab.y, "copy": copy}))
    } else {
        app.run("edit.dropSelection", json!({"sourceDocument": origin.uid, "ids": ids, "grab": [origin.grab.x, origin.grab.y], "at": [at.x, at.y]}))
    };
    if let Err(e) = result {
        if let Some(b) = before {
            crate::doc_windows::show_window(app, b);
        }
        app.status(e);
    }
    ctx.request_repaint();
}

/// The primary button's release in this frame: where it went up, and the modifiers held then.
fn primary_release(i: &egui::InputState) -> Option<(egui::Pos2, egui::Modifiers)> {
    i.events.iter().find_map(|e| match e {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers } => Some((*pos, *modifiers)),
        _ => None,
    })
}

/// Where the primary button went up in this frame, else where the pointer is. A pointer move later
/// in the frame of the release does not change where the artwork is released.
pub(crate) fn released_at(i: &egui::InputState) -> Option<egui::Pos2> {
    primary_release(i).map(|(p, _)| p).or(i.pointer.interact_pos())
}

/// Whether Alt was held when the primary button went up in this frame, else whether it is held now.
/// Alt let go later in the frame of the release still copies.
fn release_alt(ctx: &Context) -> bool {
    ctx.input(|i| primary_release(i).map_or(i.modifiers.alt, |(_, m)| m.alt))
}
