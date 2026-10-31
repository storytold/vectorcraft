use super::outline::{hit_at, hit_with, selection_hit_at};
use super::{Ui, VectorcraftApp, Xf, json, widgets};

/// Right-click: the object under the pointer is selected first unless it already is, then the
/// context menu lists what applies to the selection ([`crate::menus::context_items`]). The locked
/// objects under the pointer ([`vectorcraft_doc::hit::locked_objects_at`]) are kept in
/// `ui.context_locked` while the menu is open, for its Unlock submenu.
pub(super) fn context_menu(app: &mut VectorcraftApp, resp: &egui::Response, xf: &Xf) {
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let p = xf.to_doc(p);
        let locked = hit_with(app, p, xf.zoom, |doc, p, opt| Some(vectorcraft_doc::hit::locked_objects_at(doc, p, opt)));
        app.ui.context_locked = app.session.active().map(|st| st.uid).zip(locked);
        if let Some(st) = app.session.active()
            && let Some(top) = selection_hit_at(app, p, xf.zoom).map(|h| h.top_object(st.isolation))
            && !st.selection.contains(top)
        {
            // A locked or hidden object can't be selected; the menu is then for the selection as is.
            let _ = app.run("select.set", json!({ "ids": [top.0] }));
        }
    }
    let mut clicked = None;
    if resp.context_menu(|ui| crate::menus::context_menu_body(app, ui, &mut clicked)).is_none() {
        app.ui.context_locked = None;
    }
    if let Some((id, p)) = clicked {
        crate::menus::invoke(app, &id, p);
    }
}

/// A panel drag ([`widgets::PanelDrag`]) dropped on art acts on the object under the pointer,
/// selected or not: a paint (swatches, a Fill/Stroke proxy, the Gradient panel's thumbnail) goes
/// to its active proxy (`paint.setFill`/`paint.setStroke` with its `ids`; a gradient fits it), the
/// Appearance panel's thumbnail gives the object (the topmost one hit) the appearance it carries
/// (`appearance.copyFrom`), a graphic style is applied to it (`graphicStyle.apply`; with Alt, on
/// top of its appearance). A chip follows the pointer meanwhile.
pub(super) fn panel_drop(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, xf: &Xf) {
    crate::panels::swatches::drag_preview(app, ui.ctx());
    let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else { return };
    // Not through a floating panel over the canvas.
    if ui.ctx().layer_id_at(pos).is_some_and(|l| l != resp.layer_id) {
        return;
    }
    let Some(d) = resp.dnd_release_payload::<widgets::PanelDrag>() else { return };
    // A Libraries panel graphic: a copy centred where it is dropped.
    if let widgets::PanelDrag::LibraryGraphic { library, item } = &*d {
        let at = xf.to_doc(pos);
        if let Err(e) = app.run("library.use", json!({"library": library, "kind": "graphic", "item": item, "center": [at.x, at.y]})) {
            app.status(e);
        }
        return;
    }
    // A symbol from the Symbols panel: an instance centred where it is dropped, on art or not.
    if let widgets::PanelDrag::Symbol(name) = &*d {
        let at = xf.to_doc(pos);
        if let Err(e) = app.run("symbol.place", json!({"name": name, "x": at.x, "y": at.y})) {
            app.status(e);
        }
        return;
    }
    // A symbol from a symbol library panel: added to the document's Symbols and placed there.
    if let widgets::PanelDrag::LibrarySymbol { library, name } = &*d {
        let at = xf.to_doc(pos);
        if let Err(e) = app.run("symbol.addFromLibrary", json!({"library": library, "name": name, "place": true, "x": at.x, "y": at.y})) {
            app.status(e);
        }
        return;
    }
    let Some(hit) = hit_at(app, xf.to_doc(pos), xf.zoom) else { return };
    let Some(st) = app.session.active() else { return };
    let (cmd, params) = match &*d {
        widgets::PanelDrag::Paint { params, .. } => {
            // Colour groups paint nothing.
            if params.is_null() {
                return;
            }
            let mut params = params.clone();
            params["ids"] = json!([vectorcraft_tools::xform::paint_owner(&st.doc, hit.leaf).0]);
            params["focus"] = json!(false);
            (crate::panels::proxy_cmd(app, false), params)
        }
        widgets::PanelDrag::Appearance(source) => {
            let target = hit.top_object(st.isolation);
            if target == *source {
                return;
            }
            ("appearance.copyFrom", json!({"source": source.0, "ids": [target.0]}))
        }
        widgets::PanelDrag::GraphicStyle(name) => {
            let add = ui.input(|i| i.modifiers.alt);
            ("graphicStyle.apply", json!({"name": name, "ids": [hit.top_object(st.isolation).0], "add": add}))
        }
        // A brush from the Brushes panel: the path it lands on takes it (a compound path as a whole).
        widgets::PanelDrag::Brush { name, .. } => {
            ("brush.apply", json!({"name": name, "ids": [vectorcraft_tools::xform::paint_owner(&st.doc, hit.leaf).0]}))
        }
        // A brush from a brush library panel: added to the document's Brushes and applied to the
        // path it lands on, in one step.
        widgets::PanelDrag::LibraryBrush { library, name } => (
            "brush.addFromLibrary",
            json!({"library": library, "name": name, "apply": true, "ids": [vectorcraft_tools::xform::paint_owner(&st.doc, hit.leaf).0]}),
        ),
        // Art dragged back onto the canvas: its move was already dropped. (A symbol was placed
        // above.)
        widgets::PanelDrag::Art(_)
        | widgets::PanelDrag::Symbol(_)
        | widgets::PanelDrag::LibrarySymbol { .. }
        | widgets::PanelDrag::LibraryGraphic { .. } => return,
        // A Libraries panel style or group only moves within that panel.
        widgets::PanelDrag::LibraryItem { .. } | widgets::PanelDrag::LibraryGroup { .. } => return,
    };
    if let Err(e) = app.run(cmd, params) {
        app.status(e);
    }
}
