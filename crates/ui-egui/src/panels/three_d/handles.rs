//! The artboard's handles of the mapped graphics while the panel's Materials page is open: a press
//! on one grabs it, and dragging moves that graphic over its face, previewed while the pointer is
//! down and committed as one undo step when it is released.

use std::sync::Arc;

use craft::behavior::kurbo::Point as DocPoint;
use craft::behavior::{CanvasView, HitShape, Metrics, PartKey, PointerButton, PressPhase, Sense};
use craft::ui::canvas::{CanvasSpace, canvas_space};
use craft::ui::contract::{
    Access, Common, Component, Cx, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role, Show as _,
};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::runtime::focus::KeyUse;
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Pos2, Response, Ui, Vec2};
use serde_json::{Map, Value};
use vectorcraft_effects::{GRAPHICS, GraphicFrame, MappedGraphic};
use vectorcraft_geom::{Point, Vec2 as DocVec};

use super::{Target, find, send, set_selected_graphic};
use crate::VectorcraftApp;
use crate::panels::Live;

/// What the cached frame was built for: the document's uid and revision, the object's id, and the
/// fill or stroke and index of its 3D effect.
pub(crate) type FrameKey = (u64, u64, u64, Option<usize>, usize);

/// How near a press must be to a graphic's handle to grab it, in screen points.
const HANDLE_REACH: f32 = 8.0;
/// The radius of the handle of the graphic the page edits, and of the others.
const SELECTED_RADIUS: f32 = 6.0;
const RADIUS: f32 = 4.5;

/// The part of the handle of one graphic: `HANDLE.at(index)`.
const HANDLE: PartKey = PartKey::new("graphic");

/// What [`graphic_handles`] gives: the object's frame, its graphics, and each seen graphic's index
/// and page point.
type Handles = (Arc<Option<GraphicFrame>>, Vec<MappedGraphic>, Vec<(usize, Point)>);

/// The active document's uid and its selected objects' ids: what a handle drag moves graphics for.
type Owner = (u64, Vec<u64>);

fn drag_owner(app: &VectorcraftApp) -> Owner {
    app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect())).unwrap_or_default()
}

/// The handles of the graphics mapped onto the first selected object, while the Materials page is
/// open: each graphic's index, its face and where its center lands on the page, for the graphics
/// whose face the viewer sees, and none while a geometry effect follows a selected object's 3D
/// effect. The frame is built once for each revision of the document ([`super::Model::frames`]).
pub(super) fn graphic_handles(app: &mut VectorcraftApp) -> Option<Handles> {
    if app.three_d.page != "materials" || !crate::panel_docking::visible(app, super::ID) {
        return None;
    }
    let st = app.session.active()?;
    let n = st.selection.objects.first().and_then(|id| st.doc.node(*id))?;
    let t = find(n)?;
    // A hidden fill or stroke draws no solid and no graphics, so its graphics get no handles.
    if t.item.is_some_and(|k| n.appearance.items.get(k).is_none_or(|it| !it.visible())) {
        return None;
    }
    // A geometry effect after the 3D effect of any selected object moves or bends its drawn
    // graphics away from their places in the frame: no handles, so a drag does not put a graphic
    // away from the pointer.
    let reshaped = |n: &vectorcraft_doc::Node| find(n).is_some_and(|t| vectorcraft_effects::graphics_reshaped_after(n, t.item, t.index));
    if st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).any(reshaped) {
        return None;
    }
    let graphics_of = |t: &Target| vectorcraft_effects::mapped_graphics(&vectorcraft_effects::merged_params(&t.effect, &t.params));
    let list = graphics_of(&t);
    // Without graphics there are no handles, and no frame is built.
    if list.is_empty() {
        return None;
    }
    // Selected objects with different graphics show no handles: a drag sends the whole list.
    if st.selection.objects.iter().skip(1).filter_map(|id| find(st.doc.node(*id)?)).any(|o| graphics_of(&o) != list) {
        return None;
    }
    let key: FrameKey = (st.uid, st.revision, n.id.0, t.item, t.index);
    let frame = match &app.three_d.frames {
        Some((k, frame)) if *k == key => frame.clone(),
        _ => {
            let frame = Arc::new(vectorcraft_effects::graphic_frame_at(Some(&st.doc), n, t.item, t.index));
            app.three_d.frames = Some((key, frame.clone()));
            frame
        }
    };
    let handles = {
        let built = frame.as_ref().as_ref()?;
        list.iter().enumerate().filter_map(|(i, g)| built.handle(g).filter(|(_, seen)| *seen).map(|(p, _)| (i, p))).collect()
    };
    Some((frame, list, handles))
}

/// The offset, rounded to 0.1 pt, that a handle drag to offset (`x`, `y`) gives graphic `g`. `None`
/// when the graphic stays where it is: the pointer is where the handle was grabbed, or the rounded
/// offset is the graphic's own, so a click on a handle records no undo step.
pub(super) fn moved_to(g: &MappedGraphic, x: f64, y: f64) -> Option<(f64, f64)> {
    let rounded = ((x * 10.0).round() / 10.0, (y * 10.0).round() / 10.0);
    let still = (x - g.x).abs() < 1e-6 && (y - g.y).abs() < 1e-6;
    (!still && rounded != (g.x, g.y)).then_some(rounded)
}

fn to_screen(view: &CanvasView, p: Point) -> Pos2 {
    view.to_screen(DocPoint::new(p.x, p.y))
}

fn to_doc(view: &CanvasView, at: Pos2) -> Point {
    let p = view.to_doc(at);
    Point::new(p.x, p.y)
}

/// A grab of one handle: its graphic, the page offset from the handle to the point where it was
/// grabbed, and what it was grabbed for.
#[derive(Clone, Debug, Default, PartialEq)]
struct Held {
    index: usize,
    grab: DocVec,
    owner: Owner,
}

/// What the handles report in a pass.
#[derive(Clone, Debug)]
enum HandleEdit {
    /// A press grabbed the handle of graphic `index`.
    Grab(usize),
    /// The pointer moved the grabbed graphic to `to` (page point), or released it there.
    Move { held: Held, to: Point, phase: Live },
    /// The grab ended without a move to apply: the selection or the document changed, or it was cancelled.
    End,
}

/// The look of the handles.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(super) struct GraphicHandlesStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The dot.
    ink: StyleColor,
    /// The ring around it, in the surface's edge colour.
    edge: StyleColor,
}

/// What the handles report.
#[derive(Debug)]
pub(super) struct GraphicHandlesOutput {
    response: Response,
    edit: Option<HandleEdit>,
}

impl Outcome for GraphicHandlesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A dot at the center of each seen mapped graphic, larger for the one the page edits; a press on
/// one grabs it, and its drag reports where the graphic goes.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub(super) struct GraphicHandles<'a> {
    #[common]
    common: Common,
    handles: &'a [(usize, Point)],
    /// The graphic the page edits.
    #[prop]
    selected: usize,
    owner: Owner,
    edit: Option<HandleEdit>,
}

impl<'a> GraphicHandles<'a> {
    fn new(handles: &'a [(usize, Point)], selected: usize, owner: Owner) -> Self {
        GraphicHandles { common: Common::new(), handles, selected, owner, edit: None }.label(tl!("Graphic handles"))
    }
}

impl Component for GraphicHandles<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = GraphicHandlesStyle;
    type Output = GraphicHandlesOutput;

    fn style(cx: &StyleCx<'_>, _: GraphicHandlesStyleKey) -> GraphicHandlesStyle {
        GraphicHandlesStyle { metrics: Metrics::new(), ink: cx.color(cx.tokens.color.accent.solid), edge: cx.color(cx.tokens.color.surface.panel) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn parts(&mut self, _: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        for (index, at) in self.handles {
            cx.part(HANDLE.at(*index as u64), HitShape::circle(to_screen(&space.view, *at), HANDLE_REACH)).sense(Sense::DRAG | Sense::CLICK);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let mut held = cx.press_slot::<Held>().clone();
        if held.as_ref().is_some_and(|h| h.owner != self.owner) {
            // The selection or the document changed during the grab: it ends, so its grab offset
            // does not move another object's graphic.
            *cx.press_slot::<Held>() = None;
            self.edit = Some(HandleEdit::End);
            return;
        }
        for (index, at) in self.handles {
            let Some(press) = cx.part_if_declared(HANDLE.at(*index as u64)).map(|result| result.press) else { continue };
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let grab = to_doc(&space.view, start.origin);
                    held = Some(Held { index: *index, grab: DocVec::new(grab.x - at.x, grab.y - at.y), owner: self.owner.clone() });
                    self.edit = Some(HandleEdit::Grab(*index));
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    if let Some(h) = held.clone().filter(|h| h.index == *index) {
                        self.edit = Some(HandleEdit::Move { held: h, to: to_doc(&space.view, pos), phase: Live::Dragging });
                    }
                }
                PressPhase::Release { pos, .. } => {
                    if let Some(h) = held.take().filter(|h| h.index == *index) {
                        self.edit = Some(HandleEdit::Move { held: h, to: to_doc(&space.view, pos), phase: Live::Released });
                    }
                }
                PressPhase::Cancel { .. } if held.take().is_some() => {
                    self.edit = Some(HandleEdit::End);
                }
                _ => {}
            }
        }
        *cx.press_slot::<Held>() = held;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let draw = ui.draw();
        for (index, at) in self.handles {
            let radius = if *index == self.selected { SELECTED_RADIUS } else { RADIUS };
            let dot = PaintShape::Circle(to_screen(&space.view, *at), radius);
            draw.fill(&dot, cx.style.ink);
            draw.stroke(&dot, StrokeWidth::Medium, cx.style.edge);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        for (index, _) in self.handles {
            let node = access.node(HANDLE.at(*index as u64), Role::BUTTON);
            node.name = craft::ui::contract::Name::Text(format!("{} {}", tl!("Graphic"), index + 1));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> GraphicHandlesOutput {
        GraphicHandlesOutput { response: cx.response.clone(), edit: self.edit }
    }
}

/// Shows the handles over the canvas `space` and applies what they report. A press with Space held,
/// or with the Hand, Zoom or Rotate View tool, is left to the canvas, which pans, zooms or turns
/// the view as anywhere else.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) {
    let Some((frame, list, handles)) = graphic_handles(app) else {
        // Nothing to grab: a grab in progress ends with what it previewed.
        send(app, Map::new(), Live::Released);
        return;
    };
    let view_gesture = ui.input(|i| i.key_down(egui::Key::Space)) || matches!(app.session.tool_id(), "hand" | "zoom" | "rotateView");
    let owner = drag_owner(app);
    let out = GraphicHandles::new(&handles, app.three_d.graphic, owner).enabled(!view_gesture).show_at(ui, space.view.rect);
    match out.edit {
        Some(HandleEdit::Grab(index)) => set_selected_graphic(app, index),
        Some(HandleEdit::Move { held, to, phase }) => {
            let moved = list.get(held.index).and_then(|g| {
                let (x, y) = frame.as_ref().as_ref()?.offset_at(g.face, Point::new(to.x - held.grab.x, to.y - held.grab.y))?;
                moved_to(g, x, y)
            });
            let mut changed = Map::new();
            if let Some((x, y)) = moved {
                let mut list = list.clone();
                if let Some(g) = list.get_mut(held.index) {
                    (g.x, g.y) = (x, y);
                }
                changed.insert(GRAPHICS.into(), Value::Array(list.iter().map(MappedGraphic::to_value).collect()));
            }
            // A release with no move commits what was previewed, or nothing: a click records no step.
            send(app, changed, phase);
        }
        Some(HandleEdit::End) => send(app, Map::new(), Live::Released),
        None => {}
    }
}
