use craft::ui::canvas::CanvasSpace;
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{Bind, ShowContainer as _, Trigger};
use craft::ui::menu::Menu;
use egui::Pos2;
use serde_json::json;

use super::marks;
use super::outline::{hit_with, selection_hit_at};
use crate::VectorcraftApp;

/// The canvas context menu (right-click, Menu key, Shift+F10). A `request` (screen points: the pointer, or
/// the view centre for a key) selects the object under it unless it already is, then the menu lists what
/// applies to the selection ([`crate::menus::context_items`]) at that point until it closes. The locked
/// objects under the pointer ([`vectorcraft_doc::hit::locked_objects_at`]) are kept in `ui.context_locked`
/// for its Unlock submenu. With the Intertwine tool, a right-click where three or more objects of the group
/// overlap keeps the selection and puts what it arranges in `ui.context_intertwine`
/// ([`vectorcraft_tools::intertwine::menu_target`]). With the Artboard tool, an artboard takes the place of
/// the object ([`crate::panels::artboards::right_click`]).
///
/// Called on every pass after the canvas shows, with its response as `opener`: the menu is shown from the
/// point stored in `ui.context_menu_at`, so a later pass without a request keeps it open.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace, opener: &egui::Response, request: Option<Pos2>) {
    if let Some(at) = request {
        prepare(app, space, at);
        app.ui.context_menu_at = Some(at);
    }
    let Some(at) = app.ui.context_menu_at else { return };
    let nodes = crate::menus::context_items(app);
    let mut open = true;
    let shown = Menu::new(tl!("Canvas").to_string())
        .opened_by(opener, Trigger::Manual)
        .anchor(Anchor::Point(at))
        .open(Bind::bound(&mut open))
        .show(ui, |ui, _| craft::menu::context::rows(ui, &nodes));
    if shown.output.closed.is_some() || !open {
        app.ui.context_menu_at = None;
        app.ui.context_locked = None;
        app.ui.context_intertwine = None;
        app.ui.context_artboards = None;
        app.ui.context_spelling = None;
    }
}

/// The one-shot work of a request at screen point `at`: the facts the menu's rows read, and the selection.
fn prepare(app: &mut VectorcraftApp, space: &CanvasSpace, at: Pos2) {
    let p = space.view.to_doc(at);
    let zoom = space.view.zoom;
    let locked = hit_with(app, p, zoom, |doc, p, opt| Some(vectorcraft_doc::hit::locked_objects_at(doc, p, opt)));
    app.ui.context_locked = app.session.active().map(|st| st.uid).zip(locked);
    app.ui.context_intertwine = app
        .session
        .active()
        .filter(|_| app.session.tool_id() == "intertwine")
        .and_then(|st| vectorcraft_tools::intertwine::menu_target(&st.doc, &st.selection.objects, p).map(|(g, m)| (st.uid, g, p, m)));
    // Auto Spell Check: the underlined word right-clicked, for its suggestions.
    app.ui.context_spelling = if app.ui.view.auto_spell_check { marks::misspelled_word_at(app, p) } else { None };
    let on_artboards = app.session.tool_id() == "artboard";
    if on_artboards {
        crate::panels::artboards::right_click(app, p);
    }
    if !on_artboards
        && app.ui.context_intertwine.is_none()
        && let Some(st) = app.session.active()
        && let Some(top) = selection_hit_at(app, p, zoom).map(|h| h.top_object(st.isolation))
        && !st.selection.contains(top)
    {
        // A locked or hidden object can't be selected; the menu is then for the selection as is.
        let _ = app.run("select.set", json!({ "ids": [top.0] }));
    }
}
