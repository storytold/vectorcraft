//! A selection dragged across document tabs remains editable and leaves its source intact.

use egui::{Context, Id, Response};
use serde_json::json;
use vectorcraft_doc::NodeId;
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
    revision: u64,
    target: u64,
}

#[derive(Clone)]
struct Hover {
    uid: u64,
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
        let valid = app.session.documents().iter().any(|d| d.uid == art.origin.uid && d.revision == art.revision)
            && app.session.active().is_some_and(|d| d.uid == art.target);
        let cancel = ctx.input(|i| {
            !i.focused
                || i.key_pressed(egui::Key::Escape)
                || (!i.pointer.primary_down() && !i.pointer.primary_released())
                || i.multi_touch().is_some()
                || i.events.iter().any(|e| matches!(e, egui::Event::Touch { phase: egui::TouchPhase::Cancel, .. }))
        });
        let outside_release =
            ctx.input(|i| i.pointer.primary_released() && !app.canvas_rect.is_some_and(|r| i.pointer.interact_pos().is_some_and(|p| r.contains(p))));
        if !valid || cancel || outside_release || app.ui.dialog.is_some() {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }
    if !ctx.input(|i| i.pointer.primary_down()) {
        ctx.data_mut(|d| {
            d.remove::<Origin>(origin_id());
            d.remove::<Hover>(hover_id());
            d.remove::<egui::Pos2>(activated_at_id());
        });
    }
}

/// Hover uses `contains_pointer`: the source canvas still owns egui's dragged widget.
pub(crate) fn hover_tab(app: &mut VectorcraftApp, resp: &Response, index: usize, close: egui::Rect) -> bool {
    let ctx = &resp.ctx;
    let eligible = resp.contains_pointer()
        && ctx
            .input(|i| i.pointer.primary_down() && !i.key_pressed(egui::Key::Escape) && i.pointer.interact_pos().is_some_and(|p| !close.contains(p)))
        && app.ui.dialog.is_none();
    if !eligible || app.session.active_index() == Some(index) || ctx.data(|d| d.get_temp::<egui::Pos2>(activated_at_id()).is_some()) {
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
    let hover = ctx.data(|d| d.get_temp::<Hover>(hover_id())).filter(|h| h.uid == uid).unwrap_or(Hover { uid, since: now });
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
        let ids = app.session.active().map(|d| d.selection.objects.clone()).unwrap_or_default();
        if ids.is_empty() {
            return true;
        }
        canvas::end_document_drag(ctx);
        let revision = app.session.active().map(|d| d.revision).unwrap_or_default();
        egui::DragAndDrop::set_payload(ctx, Artwork { origin, ids, revision, target: uid });
    }
    if transfer && let Some(art) = egui::DragAndDrop::payload::<Artwork>(ctx) {
        let mut art = (*art).clone();
        art.target = uid;
        egui::DragAndDrop::set_payload(ctx, art);
    }
    app.ui.home = None;
    app.session.set_active(index);
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
    if let Some(art) = resp.dnd_release_payload::<Artwork>()
        && let Some(at) = resp.ctx.input(|i| i.pointer.interact_pos()).map(|p| xf.to_doc(p))
    {
        let result = if same {
            // Back in the source tab, resume the ordinary move (Alt copies).
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
