//! What the document canvas takes by drop: panel payloads (swatches, appearances, styles, symbols, brushes, library
//! items) and artwork carried out of a document's canvas by a Selection move, in the source's own window or another
//! tab, pane or window. The drag session resolves a drop in the pass after the release; `show` applies it.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use craft::behavior::{CanvasView, CursorIcon, CursorPriority, DropOperation, Input, Metrics, Sense};
use craft::ui::canvas::{ToolContext, ToolPointer};
use craft::ui::contract::{
    Common, Component, Cx, DragPreview, DropMark, DropTarget, ListDrag, MeasureCx, OsFiles, Pointer as _, Props, Role, Show as _,
};
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use super::BodyKind;
use super::outline::{hit_at, hit_with};
use crate::VectorcraftApp;
use crate::doc_windows::Win;
use crate::panel_drag::PanelDrag;
use crate::panels::brushes::BrushKey;
use crate::panels::brushes::BrushLibraries;
use crate::panels::graphic_styles::StyleDrag;
use crate::panels::library_panel::{self, LibraryKey, LibraryKind as _};
use crate::panels::symbols::{SymbolKey, SymbolLibraries};
use crate::place::{CanvasDrop, CanvasFiles};

/// Artwork a Selection move carries out of its canvas: the objects the move started with (an Alt-copy's preview copies
/// are not carried), the press point they were grabbed at, and the source's revision when the carry began, so an edited
/// source carries nothing. A switch to another document gives the move up, which puts back the document the move started
/// from: the art is still carried while the source has that document's content.
#[derive(Clone, Debug)]
pub(crate) struct CarriedArt {
    /// The source document's uid.
    pub(crate) document: u64,
    /// The objects that travel.
    pub(crate) ids: Vec<NodeId>,
    /// The document point the objects were grabbed at.
    pub(crate) grab: Point,
    /// `DocState::revision` of the source when the carry began.
    pub(crate) revision: u64,
    /// The source's document before the move began.
    pub(crate) before: Arc<vectorcraft_doc::Document>,
}

/// What a drop carries to `apply_drop`.
#[derive(Clone, Debug)]
enum Payload {
    Panel(PanelDrag),
    Style(StyleDrag),
    Symbol(SymbolKey),
    Brush(BrushKey),
    Library(LibraryKey),
    Art(CarriedArt),
}

/// A drop released on the canvas: its payload, the document point it landed at, the canvas zoom there (for hit
/// tolerance) and the operation the held modifiers pick (Alt copies, adds a style on top).
#[derive(Clone, Debug)]
pub(crate) struct Dropped {
    payload: Payload,
    at: Point,
    zoom: f64,
    op: DropOperation,
}

/// The style of the drop catcher: it paints nothing.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CanvasDropsStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A drop target for one payload type `P`: `make` turns the payload into the canvas's own, or refuses it. `seen` is
/// the operation of the last payload it accepted at the pointer. Where `covered` (another window lies over the canvas),
/// it takes nothing: that window's targets take the drop.
struct Target<'a, F> {
    rect: Rect,
    view: &'a CanvasView,
    covered: &'a dyn Fn(Pos2) -> bool,
    make: F,
    seen: Cell<Option<DropOperation>>,
}

impl<'a, F> Target<'a, F> {
    fn new(rect: Rect, view: &'a CanvasView, covered: &'a dyn Fn(Pos2) -> bool, make: F) -> Self {
        Target { rect, view, covered, make, seen: Cell::new(None) }
    }
}

impl<P: ?Sized, F: Fn(&P) -> Option<Payload>> DropTarget<P> for Target<'_, F> {
    type Drop = Dropped;

    fn accept(&self, payload: &P, at: Pos2, op: DropOperation) -> Option<Dropped> {
        if (self.covered)(at) {
            return None;
        }
        let payload = (self.make)(payload)?;
        self.seen.set(Some(op));
        Some(Dropped { payload, at: self.view.to_doc(at), zoom: self.view.zoom, op })
    }

    fn mark(&self, _: &Dropped) -> DropMark {
        DropMark::Into(self.rect)
    }
}

/// The catcher of every drop the canvas takes. A pane (`BodyKind::Pane`) takes carried artwork only, as no panel
/// payload or file ever dropped on a pane.
#[derive(Component, Props)]
#[component(canvas_control)]
pub(crate) struct CanvasDrops<'a> {
    #[common]
    common: Common,
    view: CanvasView,
    kind: BodyKind,
    /// The uid of the document the canvas shows.
    document: u64,
    /// `(uid, revision)` of every open document: carried art whose source is not listed unchanged is refused.
    sources: Vec<(u64, u64)>,
    /// Whether the art being carried is taken though its source's revision moved: its move was given up and the source
    /// has the content the move started from ([`CarriedArt::before`]), and the document a tab rest showed is still the
    /// current one.
    given_up: bool,
    /// Whether another window lies over the canvas at a point of it: a floating panel's OS window.
    covered: &'a dyn Fn(Pos2) -> bool,
    out: &'a RefCell<Option<Dropped>>,
    /// The files from the OS the canvas places, and where their drop goes.
    files: (CanvasFiles<'a>, &'a RefCell<Option<CanvasDrop>>),
}

impl Component for CanvasDrops<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = CanvasDropsStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: CanvasDropsStyleKey) -> CanvasDropsStyle {
        CanvasDropsStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let rect = cx.rect;
        let (sources, given_up, view, covered) = (&self.sources, self.given_up, &self.view, self.covered);
        let unchanged = |a: &CarriedArt| given_up || sources.contains(&(a.document, a.revision));
        let art = Target::new(rect, view, covered, |a: &CarriedArt| unchanged(a).then(|| Payload::Art(a.clone())));
        let mut dropped = cx.drop_target::<CarriedArt, _>(&art);
        if self.kind == BodyKind::Pane
            && let (Some(op), Some((carried, _))) = (art.seen.get(), cx.dragging::<CarriedArt>())
        {
            // Into another view of its document the art moves, unless Alt copies it; another document takes a copy.
            let icon = if carried.document == self.document && op != DropOperation::Copy { CursorIcon::Move } else { CursorIcon::Copy };
            cx.cursor(icon, CursorPriority::Drag);
        }
        if self.kind == BodyKind::Live {
            // The art of this canvas's own document is not accepted as a panel payload: its move resumes.
            let panel = Target::new(rect, view, covered, |p: &PanelDrag| (!matches!(p, PanelDrag::Art { .. })).then(|| Payload::Panel(p.clone())));
            let style = Target::new(rect, view, covered, |d: &StyleDrag| (d.document == self.document).then(|| Payload::Style(d.clone())));
            let symbol = Target::new(rect, view, covered, |d: &ListDrag<SymbolKey>| Some(Payload::Symbol(d.grabbed.clone())));
            let brush = Target::new(rect, view, covered, |d: &ListDrag<BrushKey>| Some(Payload::Brush(d.grabbed.clone())));
            let library = Target::new(rect, view, covered, |d: &ListDrag<LibraryKey>| Some(Payload::Library(d.grabbed.clone())));
            let panel = cx.drop_target::<PanelDrag, _>(&panel);
            let style = cx.drop_target::<StyleDrag, _>(&style);
            let symbol = cx.drop_target::<ListDrag<SymbolKey>, _>(&symbol);
            let brush = cx.drop_target::<ListDrag<BrushKey>, _>(&brush);
            let library = cx.drop_target::<ListDrag<LibraryKey>, _>(&library);
            dropped = dropped.or(panel).or(style).or(symbol).or(brush).or(library);
            let (files, out) = &self.files;
            *out.borrow_mut() = cx.drop_target::<OsFiles, _>(files);
        }
        *self.out.borrow_mut() = dropped;
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Shows the drop catcher over the body of window `win` and applies the drop released on it. While the tiled panes
/// draw before the current window's body ([`super::panes`]), a pane's catcher waits for that body, whose tool starts
/// the carry it catches: the pane's area is noted instead.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui, win: Win, kind: BodyKind) {
    let rect = ui.max_rect();
    if kind == BodyKind::Pane {
        if let Some(later) = &mut app.canvas.panes.drops {
            later.push((win, rect, ui.layer_id()));
            return;
        }
        // A pane outside Tile (a floating window's, the main window's while the current window floats), as shown: a
        // measuring run's area is not where it draws.
        if !craft::ui::runtime::measure::active(ui.ctx()) {
            app.canvas.windows.noted.push((win, rect));
        }
    }
    let view = match kind {
        BodyKind::Live => app.canvas_view(),
        BodyKind::Pane => pane_view(app, win, rect),
    };
    let (Some(view), Some(document)) = (view, app.session.documents().get(win.0).map(|d| d.uid)) else { return };
    let sources = app.session.documents().iter().map(|d| (d.uid, d.revision)).chain(app.canvas.panes.released_from).collect();
    // A tab rest leaves the art to the document it showed while that is the current one: any canvas or pane takes it
    // then, and nothing once that document was closed or left by other means.
    let current = app.session.active().map(|d| d.uid);
    let given_up = craft::ui::runtime::drag::active::<CarriedArt>(ui.ctx()).is_some_and(|a| {
        app.canvas.carry_target.is_none_or(|target| Some(target) == current)
            && app.session.documents().iter().any(|d| d.uid == a.document && d.same_content(&a.before))
    });
    let (out, files_out) = (RefCell::new(None), RefCell::new(None));
    let ctx = ui.ctx().clone();
    let covered = |at: Pos2| crate::panel_docking::os_window_at(app, &ctx, at);
    let files = (CanvasFiles::new(app, rect, &view), &files_out);
    let drops = CanvasDrops { common: Common::new(), view, kind, document, sources, given_up, covered: &covered, out: &out, files };
    drops.id_salt(("canvas-drops", win)).show_at(ui, rect);
    if let Some(files) = files_out.take() {
        crate::place::canvas_drop(app, ui.ctx(), files);
    }
    let Some(drop) = out.take() else { return };
    match kind {
        BodyKind::Live => {
            if let Err(e) = apply_drop(app, &drop) {
                app.status(e);
            }
        }
        BodyKind::Pane => drop_on_pane(app, win, &drop),
    }
    ui.ctx().request_repaint();
}

/// Window `win`'s view over its pane's art area `rect`.
fn pane_view(app: &VectorcraftApp, win: Win, rect: Rect) -> Option<CanvasView> {
    crate::doc_windows::view(app, win).map(|v| v.canvas(app.ui_scaling(), rect))
}

/// Applies `drop`, released on window `win`'s pane, which becomes the current window first; when the drop fails, the
/// window that was current comes back and the status bar shows the error.
pub(crate) fn drop_on_pane(app: &mut VectorcraftApp, win: Win, drop: &Dropped) {
    let before = crate::doc_windows::current(app);
    app.ui.home = None;
    crate::doc_windows::show_window(app, win);
    let result = apply_drop(app, drop);
    if let (Err(_), Some(b)) = (&result, before) {
        crate::doc_windows::show_window(app, b);
    }
    if let Err(e) = result {
        app.status(e);
    }
}

/// Art carried off the canvas and released at `at` (screen) over a pane this pass noted, or a pane outside Tile the
/// last frame drew (the front one first), in the frame the pointer reached it, before the pane could take it as a drop:
/// the pane's window and the drop there.
pub(crate) fn released_on_pane(app: &VectorcraftApp, art: CarriedArt, at: Pos2, op: DropOperation) -> Option<(Win, Dropped)> {
    let outside = app.canvas.windows.last_noted.iter().rev();
    let waiting = app.canvas.panes.drops.iter().flatten().rev().map(|(win, rect, _)| (*win, *rect));
    let (win, rect) = outside.copied().chain(waiting).find(|(_, rect)| rect.contains(at))?;
    let view = pane_view(app, win, rect)?;
    Some((win, Dropped { payload: Payload::Art(art), at: view.to_doc(at), zoom: view.zoom, op }))
}

/// Starts carrying the art of a Selection, Direct Selection or Group Selection move that left the canvas, handing it
/// to the drag session. Moves of anchors or segments stay reshapes. `covered`: another window is over the canvas at the
/// pointer. → whether a carry runs.
pub(crate) fn carry_out(app: &mut VectorcraftApp, cx: &mut ToolContext<'_>, pointer: &ToolPointer, covered: bool) -> bool {
    let Some(art) = carried(app, cx, pointer, covered) else { return false };
    let panel = PanelDrag::Art { document: art.document, ids: art.ids.clone() };
    cx.start_drag_also(art, Arc::new(panel), DragPreview::None);
    true
}

/// The art a Selection, Direct Selection or Group Selection move carries with the pointer at `pointer`, off the
/// canvas or where another window (`covered`) is over it; `None` on the canvas or for any other gesture.
pub(crate) fn carried(app: &VectorcraftApp, cx: &ToolContext<'_>, pointer: &ToolPointer, covered: bool) -> Option<CarriedArt> {
    let tool = app.session.tool_id();
    if !matches!(tool, "selection" | "directSelection" | "groupSelection") || (cx.rect.contains(pointer.pos) && !covered) {
        return None;
    }
    let st = app.session.active()?;
    let it = st.interaction.as_ref().filter(|i| i.label == "Move" || i.label == "Copy")?;
    if tool != "selection" && st.selection.objects.iter().any(|id| st.selection.partial(*id).is_some() || st.selection.segments.contains_key(id)) {
        return None;
    }
    // The art the drag started with: an Alt-copy's copies, selected while it previews, are not carried.
    let grab = app.canvas.grab?;
    Some(CarriedArt { document: st.uid, ids: it.selection.objects.clone(), grab, revision: st.revision, before: it.doc.clone() })
}

/// Runs the effect of `drop`, at the point it landed. Art of the canvas's own document resumes its move instead (a
/// drop in the source document moves, Alt copies); art of another document is copied and the source stays.
// PLAN: cv.S11 gives `apply_drop(app, drop, op)`; the operation is `drop.op`, the one `accept` received.
pub(crate) fn apply_drop(app: &mut VectorcraftApp, drop: &Dropped) -> Result<(), String> {
    let (at, zoom) = (drop.at, drop.zoom);
    let run = |app: &mut VectorcraftApp, id: &str, params: Value| app.run(id, params).map(drop_value).map_err(|e| e.to_string());
    let hit = || hit_at(app, at, zoom);
    match &drop.payload {
        Payload::Art(art) => {
            if app.session.active().is_some_and(|d| d.uid == art.document) {
                let (dx, dy) = (at.x - art.grab.x, at.y - art.grab.y);
                run(app, "object.move", json!({"ids": art.ids, "dx": dx, "dy": dy, "copy": drop.op == DropOperation::Copy}))
            } else {
                run(
                    app,
                    "edit.dropSelection",
                    json!({"sourceDocument": art.document, "ids": art.ids, "grab": [art.grab.x, art.grab.y], "at": [at.x, at.y]}),
                )
            }
        }
        Payload::Symbol(key) => run(app, "symbol.placeFromDocument", json!({"sourceDocument": key.doc, "name": key.name, "x": at.x, "y": at.y})),
        Payload::Library(key) if key.folder => Ok(()),
        Payload::Library(key) => match library_panel::parse(&key.panel) {
            Some((kind, library)) if kind.kind == SymbolLibraries::KIND => {
                run(app, "symbol.addFromLibrary", json!({"library": library, "name": key.name, "place": true, "x": at.x, "y": at.y}))
            }
            Some((kind, library)) if kind.kind == BrushLibraries::KIND => {
                let Some(hit) = hit() else { return Ok(()) };
                let owner = owner(app, &hit)?;
                run(app, "brush.addFromLibrary", json!({"library": library, "name": key.name, "apply": true, "ids": [owner]}))
            }
            _ => Ok(()),
        },
        Payload::Panel(PanelDrag::LibraryGraphic { library, item }) => {
            run(app, "library.use", json!({"library": library, "kind": "graphic", "item": item, "center": [at.x, at.y]}))
        }
        Payload::Brush(key) => match hit() {
            None if brush_has_art(app, key) => run(app, "brush.placeArt", json!({"name": key.name, "x": at.x, "y": at.y})),
            None => Ok(()),
            Some(hit) => {
                let owner = owner(app, &hit)?;
                run(app, "brush.apply", json!({"name": key.name, "ids": [owner]}))
            }
        },
        Payload::Style(style) if app.session.active().is_some_and(|st| st.uid == style.document) => {
            let Some(hit) = hit() else { return Ok(()) };
            let top = top_object(app, &hit)?;
            run(app, "graphicStyle.apply", json!({"name": style.name, "ids": [top], "add": drop.op == DropOperation::Copy}))
        }
        Payload::Style(_) => Ok(()),
        Payload::Panel(PanelDrag::Paint { params, paint, .. }) if !params.is_null() => {
            if let Some(color) = paint.color()
                && crate::panels::proxy_cmd(app, false) == "paint.setFill"
                && let Some((id, points)) = selected_mesh_drop(app, at, zoom)
            {
                let params = json!({"id": id.0, "points": points, "color": crate::panels::color_json(&color), "keepModel": true});
                return run(app, "object.mesh.setPointColor", params);
            }
            let Some(hit) = hit() else { return Ok(()) };
            let mut params = params.clone();
            params["ids"] = json!([owner(app, &hit)?]);
            params["focus"] = json!(false);
            let cmd = crate::panels::proxy_cmd(app, false);
            run(app, cmd, params)
        }
        Payload::Panel(PanelDrag::Appearance(source)) => {
            let Some(hit) = hit() else { return Ok(()) };
            let target = top_object(app, &hit)?;
            if target == source.0 {
                return Ok(());
            }
            run(app, "appearance.copyFrom", json!({"source": source.0, "ids": [target]}))
        }
        Payload::Panel(_) => Ok(()),
    }
}

/// A command's result is no use to a drop.
fn drop_value(_: Value) {}

/// The id a paint or brush lands on: the path hit, a compound path as a whole.
fn owner(app: &VectorcraftApp, hit: &vectorcraft_doc::hit::Hit) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    Ok(vectorcraft_tools::xform::paint_owner(&st.doc, hit.leaf).0)
}

/// The topmost object a hit belongs to, in the current isolation.
fn top_object(app: &VectorcraftApp, hit: &vectorcraft_doc::hit::Hit) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    Ok(hit.top_object(st.isolation).0)
}

/// Whether the brush `key` names has art to place: a scatter, art or pattern brush of its source document.
fn brush_has_art(app: &VectorcraftApp, key: &BrushKey) -> bool {
    app.session
        .documents()
        .iter()
        .find(|d| d.uid == key.doc)
        .and_then(|d| vectorcraft_brush::find(&d.doc, &key.name))
        .and_then(|b| serde_json::to_value(b).ok()) // PLAN: assumed `Brush: Serialize` with a "type" tag, as the panel's `def`
        .is_some_and(|v| matches!(v["type"].as_str(), Some("scatter" | "art" | "pattern")))
}

/// The selected, editable gradient mesh and its points that a solid color dropped at document
/// point `p` at `zoom` recolors: the topmost object within 5 screen pixels of `p` must be that mesh
/// (fills count even with Object Selection by Path Only, and clipping applies), and the points are
/// its point within 5 screen pixels, else the four points around its patch under `p`.
fn selected_mesh_drop(app: &VectorcraftApp, p: Point, zoom: f64) -> Option<(NodeId, Vec<usize>)> {
    let tol = 5.0 / zoom.max(1e-9);
    let leaf = hit_with(app, p, zoom, |doc, p, opt| {
        vectorcraft_doc::hit::hit_test(doc, p, vectorcraft_doc::hit::HitOptions { path_only: false, tol: opt.tol.max(tol), ..opt })
    })?
    .leaf;
    let st = app.session.active()?;
    if !st.selection.contains(leaf) || !st.doc.editable()(leaf) {
        return None;
    }
    let Some(vectorcraft_doc::NodeKind::Mesh(m)) = st.doc.node(leaf).map(|n| &n.kind) else { return None };
    Some((leaf, vectorcraft_tools::meshedit::color_points(m, p, tol)?))
}

#[cfg(test)]
mod tests;
