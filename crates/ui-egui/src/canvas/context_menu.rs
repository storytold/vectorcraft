use craft::ui::canvas::CanvasSpace;
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{Bind, ShowContainer as _, Trigger};
use craft::ui::menu::Menu;
use egui::Pos2;
use serde_json::json;
use vectorcraft_geom::Point;

use super::marks;
use super::outline::{hit_with, selection_hit_at};
use crate::VectorcraftApp;

/// A request for the canvas menu at `at` (screen points): a secondary click at the pointer, or the Menu key or
/// Shift+F10 (`key`), which the canvas reports at the view centre.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    pub(crate) at: Pos2,
    pub(crate) key: bool,
}

/// The canvas context menu (right-click, Menu key, Shift+F10). A right-click selects the object under the
/// pointer unless it already is, then the menu lists what applies to the selection
/// ([`crate::menus::context_items`]) at that point until it closes. The locked objects under the pointer
/// ([`vectorcraft_doc::hit::locked_objects_at`]) are kept in `ui.context_locked` for its Unlock submenu. With
/// the Intertwine tool, a right-click where three or more objects of the group overlap keeps the selection and
/// puts what it arranges in `ui.context_intertwine` ([`vectorcraft_tools::intertwine::menu_target`]). With the
/// Artboard tool, an artboard takes the place of the object ([`crate::panels::artboards::right_click`]). A key
/// request keeps the selection and opens the menu at it ([`prepare_key`]).
///
/// Called on every pass after the canvas shows, with its response as `opener`: the menu is shown from the
/// point stored in `ui.context_menu_at`, so a later pass without a request keeps it open.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace, opener: &egui::Response, request: Option<Request>) {
    if let Some(request) = request {
        let at = if request.key {
            prepare_key(app, space)
        } else {
            prepare(app, space, request.at);
            request.at
        };
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

/// The one-shot work of a key request: the menu is for the selection as it is (the selected artboards with the
/// Artboard tool), at the centre of the part of it in view, or at the view centre when none of it is in view.
/// Nothing under a point is looked up: no locked objects, word or Intertwine target. Returns the menu's point.
fn prepare_key(app: &mut VectorcraftApp, space: &CanvasSpace) -> Pos2 {
    app.ui.context_locked = None;
    app.ui.context_intertwine = None;
    app.ui.context_spelling = None;
    let on_artboards = app.session.tool_id() == "artboard";
    let bounds = if on_artboards {
        app.session.active().and_then(|st| {
            let picked = crate::panels::artboards::picked(app, st.doc.artboards.len());
            picked.iter().filter_map(|&i| st.doc.artboards.get(i).map(|a| a.rect)).reduce(|a, b| a.union(b))
        })
    } else {
        app.selection_bounds()
    };
    let view = &space.view;
    let at = bounds
        .map(|b| {
            egui::Rect::from_points(&[b.origin(), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)].map(|p| view.to_screen(p)))
        })
        .map(|r| r.intersect(view.rect))
        .filter(|r| !r.is_negative())
        .map_or(view.rect.center(), |r| r.center());
    if on_artboards {
        crate::panels::artboards::measure_menu(app, view.to_doc(at));
    }
    at
}
