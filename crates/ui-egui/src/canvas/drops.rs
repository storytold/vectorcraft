//! What the document canvas takes by drop: panel payloads (swatches, appearances, styles, symbols, brushes, library
//! items) and artwork carried out of a document's canvas by a Selection move, in the source's own window or another
//! tab, pane or window. The drag session resolves a drop in the pass after the release; `show` applies it.

use std::cell::RefCell;
use std::sync::Arc;

use craft::behavior::{CanvasView, DropOperation, Input, Metrics, Sense};
use craft::ui::canvas::{CanvasSpace, ToolContext, ToolPointer}; // PLAN: assumed path of the ui-canvas items
use craft::ui::contract::{Common, Component, Cx, DragPreview, DropMark, DropTarget, ListDrag, MeasureCx, Pointer as _, Props, Role, Show as _};
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use super::BodyKind;
use super::outline::{hit_at, hit_with};
use crate::VectorcraftApp;
use crate::doc_windows::Win;
use crate::panels::brushes::BrushKey;
use crate::panels::graphic_styles::StyleKey;
use crate::panels::libraries::{LibraryKey, LibraryKind}; // PLAN: assumed `LibraryKind::of_panel(&str) -> Option<LibraryKind>` with Graphic, Symbol, Brush
use crate::panels::symbols::SymbolKey;
use crate::panel_drag::PanelDrag;

/// Artwork a Selection move carries out of its canvas: the objects the move started with (an Alt-copy's preview copies
/// are not carried), the press point they were grabbed at, and the source's revision when the carry began, so an edited
/// source carries nothing.
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
}

/// What a drop carries to `apply_drop`.
#[derive(Clone, Debug)]
enum Payload {
    Panel(PanelDrag),
    Style(StyleKey),
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

/// A drop target for one payload type `P`: `make` turns the payload into the canvas's own, or refuses it.
struct Target<'a, F> {
    rect: Rect,
    view: &'a CanvasView,
    make: F,
}

impl<P: ?Sized, F: Fn(&P) -> Option<Payload>> DropTarget<P> for Target<'_, F> {
    type Drop = Dropped;

    fn accept(&self, payload: &P, at: Pos2, op: DropOperation) -> Option<Dropped> {
        Some(Dropped { payload: (self.make)(payload)?, at: self.view.to_doc(at), zoom: self.view.zoom, op })
    }

    fn mark(&self, _: &Dropped) -> DropMark {
        DropMark::Into(self.rect)
    }
}

/// The catcher of every drop the canvas takes. A pane (`BodyKind::Pane`) takes carried artwork only, as no panel
/// payload ever dropped on a pane.
#[derive(Component, Props)]
#[component(canvas_control)]
pub(crate) struct CanvasDrops<'a> {
    #[common]
    common: Common,
    view: CanvasView,
    kind: BodyKind,
    /// `(uid, revision)` of every open document: carried art whose source is not listed unchanged is refused.
    sources: Vec<(u64, u64)>,
    out: &'a RefCell<Option<Dropped>>,
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
        let rect = cx.rect();
        let sources = &self.sources;
        let art = Target {
            rect,
            view: &self.view,
            make: |a: &CarriedArt| sources.contains(&(a.document, a.revision)).then(|| Payload::Art(a.clone())),
        };
        let mut dropped = cx.drop_target::<CarriedArt, _>(&art);
        if self.kind == BodyKind::Live {
            let to = |make: fn(&_) -> Option<Payload>| Target { rect, view: &self.view, make };
            // The art of this canvas's own document is not accepted as a panel payload: its move resumes.
            let panel = cx.drop_target::<PanelDrag, _>(&to(|p: &PanelDrag| (!matches!(p, PanelDrag::Art { .. })).then(|| Payload::Panel(p.clone()))));
            let style = cx.drop_target::<ListDrag<StyleKey>, _>(&to(|d: &ListDrag<StyleKey>| Some(Payload::Style(d.grabbed.clone()))));
            let symbol = cx.drop_target::<ListDrag<SymbolKey>, _>(&to(|d: &ListDrag<SymbolKey>| Some(Payload::Symbol(d.grabbed.clone()))));
            let brush = cx.drop_target::<ListDrag<BrushKey>, _>(&to(|d: &ListDrag<BrushKey>| Some(Payload::Brush(d.grabbed.clone()))));
            let library = cx.drop_target::<ListDrag<LibraryKey>, _>(&to(|d: &ListDrag<LibraryKey>| Some(Payload::Library(d.grabbed.clone()))));
            dropped = dropped.or(panel).or(style).or(symbol).or(brush).or(library);
        }
        *self.out.borrow_mut() = dropped;
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Shows the drop catcher over the body of window `win` and applies the drop released on it. A pane shows window
/// `win` first, as the drop makes it the current window.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui, _space: &CanvasSpace, win: Win, kind: BodyKind) {
    let rect = ui.max_rect();
    let view = match kind {
        BodyKind::Live => app.canvas_view(),
        BodyKind::Pane => crate::doc_windows::view(app, win).map(|v| v.canvas(app.ui_scaling(), rect)),
    };
    let Some(view) = view else { return };
    let sources = app.session.documents().iter().map(|d| (d.uid, d.revision)).collect();
    let out = RefCell::new(None);
    CanvasDrops { common: Common::new(), view, kind, sources, out: &out }.id_salt(("canvas-drops", win)).show_at(ui, rect);
    let Some(drop) = out.take() else { return };
    let result = match kind {
        BodyKind::Live => apply_drop(app, &drop),
        BodyKind::Pane => {
            let before = crate::doc_windows::current(app);
            app.ui.home = None;
            crate::doc_windows::show_window(app, win);
            let result = apply_drop(app, &drop);
            if let (Err(_), Some(b)) = (&result, before) {
                crate::doc_windows::show_window(app, b);
            }
            result
        }
    };
    if let Err(e) = result {
        app.status(e);
    }
    ui.ctx().request_repaint();
}

/// Starts carrying the art of a Selection, Direct Selection or Group Selection move that left the canvas, handing it
/// to the drag session. Moves of anchors or segments stay reshapes. → whether a carry runs.
pub(crate) fn carry_out(app: &mut VectorcraftApp, cx: &mut ToolContext<'_>, pointer: &ToolPointer) -> bool {
    let tool = app.session.tool_id();
    if !matches!(tool, "selection" | "directSelection" | "groupSelection") || cx.rect.contains(pointer.pos) {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let Some(it) = st.interaction.as_ref().filter(|i| i.label == "Move" || i.label == "Copy") else { return false };
    if tool != "selection" && st.selection.objects.iter().any(|id| st.selection.partial(*id).is_some() || st.selection.segments.contains_key(id)) {
        return false;
    }
    // The art the drag started with: an Alt-copy's copies, selected while it previews, are not carried.
    let ids = it.selection.objects.clone();
    let Some(grab) = app.canvas.grab else { return false }; // PLAN: assumed `canvas.grab: Option<Point>`, the press point, set by the adapter (canvas-w20)
    let art = CarriedArt { document: st.uid, ids: ids.clone(), grab, revision: st.revision };
    let panel = PanelDrag::Art { document: art.document, ids };
    cx.start_drag_also(art, Arc::new(panel), DragPreview::None);
    true
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
                run(app, "edit.dropSelection", json!({"sourceDocument": art.document, "ids": art.ids, "grab": [art.grab.x, art.grab.y], "at": [at.x, at.y]}))
            }
        }
        Payload::Symbol(key) => {
            run(app, "symbol.placeFromDocument", json!({"sourceDocument": key.doc, "name": key.name, "x": at.x, "y": at.y}))
        }
        Payload::Library(key) if key.folder => Ok(()),
        Payload::Library(key) => match LibraryKind::of_panel(&key.panel) {
            Some(LibraryKind::Graphic) => {
                run(app, "library.use", json!({"library": key.panel, "kind": "graphic", "item": key.name, "center": [at.x, at.y]}))
            }
            Some(LibraryKind::Symbol) => {
                run(app, "symbol.addFromLibrary", json!({"library": key.panel, "name": key.name, "place": true, "x": at.x, "y": at.y}))
            }
            Some(LibraryKind::Brush) => {
                let Some(hit) = hit() else { return Ok(()) };
                let owner = owner(app, &hit)?;
                run(app, "brush.addFromLibrary", json!({"library": key.panel, "name": key.name, "apply": true, "ids": [owner]}))
            }
            None => Ok(()),
        },
        Payload::Brush(key) => match hit() {
            None if brush_has_art(app, key) => run(app, "brush.placeArt", json!({"name": key.name, "x": at.x, "y": at.y})),
            None => Ok(()),
            Some(hit) => {
                let owner = owner(app, &hit)?;
                run(app, "brush.apply", json!({"name": key.name, "ids": [owner]}))
            }
        },
        Payload::Style(key) => {
            let Some(hit) = hit() else { return Ok(()) };
            let top = top_object(app, &hit)?;
            run(app, "graphicStyle.apply", json!({"name": key.0, "ids": [top], "add": drop.op == DropOperation::Copy}))
        }
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
