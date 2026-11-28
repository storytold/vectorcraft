//! The cells at a row's trailing end: the target circle and the selection square.
//!
//! - The target circle targets its layer, group or object, with a row click's modifier rules
//!   ([`rows::target_click`](super::rows::target_click)); dragging it carries the appearance
//!   ([`PanelDrag::Appearance`]) onto another row's circle (Alt copies it) or the trash. A circle
//!   lights up while another row's appearance is held over it.
//! - The selection square selects the row's art, with the modifier rules of
//!   [`rows::art_click`](super::rows::art_click) (Shift selects a range of rows); a row whose art is
//!   selected shows it in its layer's colour, a row holding selected art a small one, and dragging
//!   either onto another row moves the selected art there (Alt copies it).
//!
//! A ruler guide's row and a spine's row have no circle; their square shows while the guide or the
//! blend is selected.

use craft::behavior::{DropOperation, Metrics, PointerButton, PressPhase, Sense};
use craft::keymap::Mods;
use craft::layers::ui::panel::LayerPosition;
use craft::layers::ui::tree::LayerInfo;
use craft::ui::contract::{
    Common, Component, Cx, DragPreview, DropMark, DropTarget, MeasureCx, NoValue, Outcome, PaintUi, Pointer as _, Props, Role,
};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2, vec2};

use super::model::{DocLayers, RowId};
use super::rows::{TARGET_RADIUS, has_styled_target, shows_targeted, target_marks};
use super::{Node, NodeId, Value, json};
use crate::panel_drag::PanelDrag;

/// The side of the square of a row whose art is selected.
const SQUARE_SIDE: f32 = 7.0;
/// The side of the square of a row that holds selected art.
const HELD_SIDE: f32 = 4.0;

/// The selected art a row's selection square carries to another row in the same document.
/// `document` is the document's uid and revert count: File › Revert keeps the uid but replaces the drawing.
#[derive(Clone, Debug)]
pub(super) struct ArtDrag {
    pub(super) document: (u64, u64),
    pub(super) ids: Vec<NodeId>,
}

/// What a cell reports.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CellPick {
    /// The circle was clicked with these modifiers.
    Target(Mods),
    /// The square was clicked with these modifiers: select the row's art, by the rules of
    /// [`rows::art_click`](super::rows::art_click).
    SelectArt(Mods),
    /// Another row's appearance was dropped on the circle (Alt copies it).
    TakeAppearance { source: NodeId, copy: bool },
}

/// The style of the cells: the side of one, the ink of the marks and the accent of a circle that
/// takes a drop.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(super) struct CellStyle {
    #[style(metrics)]
    metrics: Metrics,
    side: f32,
    ink: StyleColor,
    accent: StyleColor,
}

fn cell_style(cx: &StyleCx<'_>) -> CellStyle {
    CellStyle {
        metrics: Metrics::new(),
        side: cx.control().icon,
        ink: cx.color(cx.tokens.color.text.normal),
        accent: cx.color(cx.tokens.color.accent.solid),
    }
}

/// What a cell gives back: its response and what it reports this pass.
#[derive(Debug)]
pub(super) struct CellOutput {
    response: Response,
    pick: Option<CellPick>,
}

impl Outcome for CellOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A row's target circle: a ring, doubled while the row is targeted, with a dot when it has an
/// appearance or transparency of its own ([`target_marks`]).
#[derive(Component, Props)]
#[component(sample = TargetCircle::sample)]
pub(super) struct TargetCircle {
    #[common]
    common: Common,
    document: u64,
    id: NodeId,
    targeted: bool,
    styled: bool,
    /// Another row's appearance is held over the circle.
    held: bool,
    pick: Option<CellPick>,
}

impl TargetCircle {
    fn new(document: u64, id: NodeId, targeted: bool, styled: bool) -> Self {
        Self { common: Common::new(), document, id, targeted, styled, held: false, pick: None }
    }

    /// A targeted circle with an appearance, for the suite.
    fn sample() -> Self {
        Self::new(1, NodeId(1), true, true).label("Target")
    }
}

/// The circle as a drop target: it takes another row's appearance.
struct TakesAppearance {
    document: u64,
    own: NodeId,
    rect: Rect,
}

impl DropTarget<PanelDrag> for TakesAppearance {
    type Drop = (NodeId, bool);

    fn accept(&self, payload: &PanelDrag, _: Pos2, op: DropOperation) -> Option<(NodeId, bool)> {
        match payload {
            PanelDrag::Appearance { document, source } if *document == self.document && *source != self.own => {
                Some((*source, op == DropOperation::Copy))
            }
            _ => None,
        }
    }

    fn mark(&self, _: &(NodeId, bool)) -> DropMark {
        DropMark::Into(self.rect)
    }
}

/// Whether the press of `cx` started its drag this pass with the primary button: only it carries
/// anything away.
fn primary_drag_start<C: Component>(cx: &Cx<'_, C>) -> bool {
    let phase = &cx.press().phase;
    matches!(phase, PressPhase::DragStart { .. }) && phase.start().is_some_and(|start| start.button == PointerButton::Primary)
}

impl Component for TargetCircle {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = CellStyle;
    type Output = CellOutput;

    fn style(cx: &StyleCx<'_>, _: TargetCircleStyleKey) -> CellStyle {
        cell_style(cx)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(activation) = cx.activation() {
            self.pick = Some(CellPick::Target(activation.mods));
        }
        if primary_drag_start(cx) {
            cx.start_drag(PanelDrag::Appearance { document: self.document, source: self.id }, DragPreview::None);
        }
        let rect = cx.rect;
        let target = TakesAppearance { document: self.document, own: self.id, rect };
        self.held = cx.dragging::<PanelDrag>().is_some_and(|(drag, at)| rect.contains(at) && target.accept(&drag, at, DropOperation::Move).is_some());
        if let Some((source, copy)) = cx.drop_target::<PanelDrag, _>(&target) {
            self.pick = Some(CellPick::TakeAppearance { source, copy });
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let center = cx.content_rect.center();
        let draw = ui.draw();
        let (ring, width) = if self.held { (cx.style.accent, StrokeWidth::Medium) } else { (cx.style.ink, StrokeWidth::Thin) };
        draw.stroke(&PaintShape::Circle(center, TARGET_RADIUS), width, ring);
        let (inner, dot) = target_marks(self.targeted, self.styled);
        if let Some(radius) = inner {
            draw.stroke(&PaintShape::Circle(center, radius), StrokeWidth::Thin, cx.style.ink);
        }
        if let Some(radius) = dot {
            draw.fill(&PaintShape::Circle(center, radius), cx.style.ink);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CellOutput {
        CellOutput { response: cx.response.clone(), pick: self.pick }
    }
}

/// A row's selection square: filled in its layer's colour while the row's art is selected (for a
/// layer or sublayer, all of its visible, unlocked art), a small square while the row holds
/// selected art. Without `id` it is only a mark (a ruler guide's row, a
/// spine's row) and takes no press.
#[derive(Component, Props)]
#[component(sample = SelectionSquare::sample)]
pub(super) struct SelectionSquare {
    #[common]
    common: Common,
    document: (u64, u64),
    id: Option<NodeId>,
    selected: bool,
    holds: bool,
    color: DataColor,
    /// What a drag of the square carries: the selected objects.
    art: Vec<NodeId>,
    pick: Option<CellPick>,
}

impl SelectionSquare {
    fn new(document: (u64, u64), id: Option<NodeId>, selected: bool, holds: bool, color: DataColor, art: Vec<NodeId>) -> Self {
        Self { common: Common::new(), document, id, selected, holds, color, art, pick: None }
    }

    /// A selected row's square, for the suite.
    fn sample() -> Self {
        Self::new((1, 0), Some(NodeId(1)), true, false, DataColor::new(Color32::from_rgb(79, 128, 255)), vec![NodeId(1)]).label("Select art")
    }
}

impl Component for SelectionSquare {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = CellStyle;
    type Output = CellOutput;

    fn style(cx: &StyleCx<'_>, _: SelectionSquareStyleKey) -> CellStyle {
        cell_style(cx)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        if self.id.is_some() { Sense::CLICK | Sense::DRAG } else { Sense::empty() }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if self.id.is_none() {
            return;
        }
        if let Some(activation) = cx.activation() {
            self.pick = Some(CellPick::SelectArt(activation.mods));
        }
        if (self.selected || self.holds) && !self.art.is_empty() && primary_drag_start(cx) {
            cx.start_drag(ArtDrag { document: self.document, ids: self.art.clone() }, DragPreview::None);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let center = cx.content_rect.center();
        let draw = ui.draw();
        if self.selected {
            let square = PaintShape::Rect(Rect::from_center_size(center, Vec2::splat(SQUARE_SIDE)), CornerRadius::ZERO);
            draw.data_fill(&square, self.color);
            draw.stroke(&square, StrokeWidth::Hairline, cx.style.ink);
        } else if self.holds {
            draw.data_fill(&PaintShape::Rect(Rect::from_center_size(center, Vec2::splat(HELD_SIDE)), CornerRadius::ZERO), self.color);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CellOutput {
        CellOutput { response: cx.response.clone(), pick: self.pick }
    }
}

/// A row's two cells side by side: the target circle (empty room on a row without one), then the
/// selection square, so every row's square lines up.
#[derive(Component, Props)]
#[component(sample = Cells::sample)]
pub(super) struct Cells {
    #[common]
    common: Common,
    circle: Option<TargetCircle>,
    square: Option<SelectionSquare>,
    picks: Vec<CellPick>,
}

impl Cells {
    fn new(circle: Option<TargetCircle>, square: SelectionSquare) -> Self {
        Self { common: Common::new(), circle, square: Some(square), picks: Vec::new() }
    }

    /// An object row's cells, for the suite.
    fn sample() -> Self {
        Self::new(Some(TargetCircle::sample()), SelectionSquare::sample())
    }
}

/// What a row's cells report this pass.
#[derive(Debug)]
pub(super) struct CellsOutput {
    response: Response,
    picks: Vec<CellPick>,
}

impl Outcome for CellsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl Component for Cells {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = CellStyle;
    type Output = CellsOutput;

    fn style(cx: &StyleCx<'_>, _: CellsStyleKey) -> CellStyle {
        cell_style(cx)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        vec2(2.0 * cx.style.side, cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.content_rect;
        let cell = vec2(rect.width() / 2.0, rect.height());
        if let Some(circle) = self.circle.take() {
            self.picks.extend(ui.child("target", Rect::from_min_size(rect.min, cell), circle).pick);
        }
        if let Some(square) = self.square.take() {
            self.picks.extend(ui.child("square", Rect::from_min_size(rect.min + vec2(cell.x, 0.0), cell), square).pick);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CellsOutput {
        CellsOutput { response: cx.response.clone(), picks: self.picks }
    }
}

/// The marks of node row `n`'s selection square in the panel over `tree`: whether it shows the
/// full-sized square (the row is selected, or it is a layer or sublayer whose visible, unlocked art
/// is all selected), and whether it shows the small one (it holds selected art otherwise).
pub(super) fn square_marks(tree: &DocLayers<'_>, n: &Node) -> (bool, bool) {
    let full = tree.is_selected(n.id) || tree.is_full(n.id);
    let holds = !full && {
        let mut any = false;
        n.walk(&mut |c| any |= c.id != n.id && tree.is_selected(c.id));
        any
    };
    (full, holds)
}

/// The cells of row `row` (shown as `info`) of the panel over `tree`.
pub(super) fn build(tree: &DocLayers<'_>, row: &RowId, info: &LayerInfo) -> Cells {
    let st = tree.state();
    let document = (st.uid, st.reverts);
    let color = info.color.unwrap_or_else(|| DataColor::new(Color32::TRANSPARENT));
    let selected = |id: NodeId| tree.is_selected(id);
    match *row {
        RowId::Node(id) => {
            let Some(n) = tree.doc().node(id) else { return Cells::new(None, SelectionSquare::new(document, None, false, false, color, vec![])) };
            let is_sel = selected(id);
            let (full, holds) = square_marks(tree, n);
            let art = if full || holds { st.selection.objects.clone() } else { vec![] };
            let circle = TargetCircle::new(st.uid, id, shows_targeted(tree, n, is_sel), has_styled_target(n))
                .label(crate::i18n::fmt(tl!("Target {row}"), &[("row", info.name.as_str())]));
            let square = SelectionSquare::new(document, Some(id), full, holds, color, art)
                .label(crate::i18n::fmt(tl!("Select art of {row}"), &[("row", info.name.as_str())]));
            Cells::new(Some(circle), square)
        }
        RowId::Guide(i) => Cells::new(None, SelectionSquare::new(document, None, tree.guides().contains(&i), false, color, vec![])),
        RowId::Spine(blend) => Cells::new(None, SelectionSquare::new(document, None, selected(blend), false, color, vec![])),
    }
}

/// What row `row`'s cells reported: the command `appearance.transfer` on the row's node, added to `out`, and a click on
/// its target circle (`target`) or its selection square (`art`) with its modifiers, which the panel resolves once it
/// knows its rows.
pub(super) fn output(
    out: &mut Vec<(String, Value)>,
    target: &mut Option<(NodeId, Mods)>,
    art: &mut Option<(NodeId, Mods)>,
    row: &RowId,
    shown: CellsOutput,
) {
    let RowId::Node(id) = *row else { return };
    for pick in shown.picks {
        match pick {
            CellPick::Target(mods) => *target = Some((id, mods)),
            CellPick::SelectArt(mods) => *art = Some((id, mods)),
            CellPick::TakeAppearance { source, copy } => {
                out.push(("appearance.transfer".to_string(), json!({"source": source.0, "target": id.0, "copy": copy})));
            }
        }
    }
}

/// Whether a row takes the selected art carried by a selection square ([`ArtDrag`]): only into a
/// layer or group that takes the objects ([`DocLayers::lands_in`]).
pub(super) fn accepts_art<'a>(tree: &'a DocLayers<'a>) -> impl Fn(&ArtDrag, &RowId, LayerPosition) -> bool + 'a {
    move |art: &ArtDrag, row: &RowId, position: LayerPosition| {
        art.document == (tree.state().uid, tree.state().reverts)
            && position == LayerPosition::Into
            && matches!(*row, RowId::Node(id) if tree.lands_in(id, &art.ids, true))
    }
}
