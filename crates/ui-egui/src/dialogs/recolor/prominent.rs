//! `ProminentBar`: the Recolor Artwork Edit tab's Prominent Colors bar, a part per colour as wide as its row's share
//! of the art's uses, with an edge between neighbours that moves width from one to the other.

use craft::behavior::{Axis, CursorIcon, HitLevel, HitPriority, Metrics, PartKey, PressPhase, Sense};
use craft::theme::Srgba;
use craft::ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Name, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role,
};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Rect, Vec2, pos2, vec2};

use super::edit_tab::color_name;

/// The smallest share of the bar a drag leaves a part.
const MIN_PART: f64 = 0.01;

/// Part `n` of the bar: `PART.at(n)`.
pub(super) const PART: PartKey = PartKey::new("part");
/// The edge after part `k`, which a drag moves: `EDGE.at(k)`.
pub(super) const EDGE: PartKey = PartKey::new("edge");

/// One part of a [`ProminentBar`]: the row it stands for and the colour it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ProminentPart {
    /// The row's index.
    pub(super) row: usize,
    /// The row's new colour, or its first current colour when it has none.
    pub(super) color: Srgba,
}

/// The `weights` a drag of the bar files: each part's width in percent with two decimals at its row, and 0 for the rows
/// the bar leaves out.
pub(super) fn weights(parts: &[ProminentPart], widths: &[f64], rows: usize) -> Vec<f64> {
    let mut weights = vec![0.0; rows];
    for (part, width) in parts.iter().zip(widths) {
        if let Some(weight) = weights.get_mut(part.row) {
            *weight = (width * 10_000.0).round() / 100.0;
        }
    }
    weights
}

/// The style of the bar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(super) struct ProminentBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The bar's height.
    height: f32,
    /// The width the bar asks for when the layout offers none.
    length: f32,
    /// How wide the band round an edge is that a press takes.
    grab: f32,
    /// The outline of a bar with no parts.
    edge: StyleColor,
    /// The line of a hovered or dragged edge.
    accent: StyleColor,
}

/// The edge a press grabbed: the widths at the press, which every pass of the drag moves afresh, and where it was.
#[derive(Clone, Debug)]
struct Drag {
    edge: usize,
    base: Vec<f64>,
    from: f32,
}

impl Drag {
    /// The widths with the edge at `x` of a bar `width` wide: the two parts beside it share their width, each keeping
    /// [`MIN_PART`] of the bar, or half of the pair when the pair is narrower.
    fn to(&self, x: f32, width: f64) -> Vec<f64> {
        let mut widths = self.base.clone();
        let (Some(&a), Some(&b)) = (widths.get(self.edge), widths.get(self.edge + 1)) else { return widths };
        let pair = a + b;
        if !(pair.is_finite() && width > 0.0) {
            return widths;
        }
        let least = MIN_PART.min(pair / 2.0);
        let moved = (a + f64::from(x - self.from) / width).clamp(least, pair - least);
        widths[self.edge] = moved;
        widths[self.edge + 1] = pair - moved;
        widths
    }
}

/// The drag the bar holds between the passes of one gesture.
#[derive(Clone, Debug, Default)]
struct Held {
    drag: Option<Drag>,
}

/// The x of the edge after each part but the last.
fn edges(rect: Rect, widths: &[f64]) -> Vec<f32> {
    let mut at = 0.0;
    let inner = widths.len().saturating_sub(1);
    widths
        .iter()
        .take(inner)
        .map(|width| {
            at += width;
            rect.left() + (at * f64::from(rect.width())) as f32
        })
        .collect()
}

/// Each part's rect: from the edge before it to the edge after it; the last part reaches the bar's end.
fn spans(rect: Rect, widths: &[f64]) -> Vec<Rect> {
    let edges = edges(rect, widths);
    (0..widths.len())
        .map(|n| {
            let left = n.checked_sub(1).and_then(|k| edges.get(k)).copied().unwrap_or(rect.left());
            let right = edges.get(n).copied().unwrap_or(rect.right());
            Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom()))
        })
        .collect()
}

/// A part's share of the bar, in whole percent.
fn percent(width: f64) -> String {
    format!("{}%", (width * 100.0).round())
}

/// The Prominent Colors bar: a part per [`ProminentPart`], as wide as its width in `widths` (the parts' shares of the
/// bar, adding up to 1), and an edge between each two neighbours.
///
/// - Pointer: a drag of an edge moves width from one of its parts to the other, each keeping [`MIN_PART`] of the bar
///   or half of the pair when the pair is narrower; one gesture, whose commit is the widths at the release. A click on
///   an edge with no drag changes nothing. The hover tooltip of a part gives its share in percent.
/// - Keys: none; the dialog's `weights` field sets the shares.
/// - AccessKit: a group named by the label; each part a graphics object named "Color N", valued by its share.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(content_painter, sample = sample)]
pub(super) struct ProminentBar<'a> {
    #[common]
    common: Common,
    parts: &'a [ProminentPart],
    widths: Bind<'a, Vec<f64>>,
}

impl<'a> ProminentBar<'a> {
    /// A bar named `label` over `parts`, each as wide as its width in `widths`.
    pub(super) fn new(label: impl Into<Label>, parts: &'a [ProminentPart], widths: Bind<'a, Vec<f64>>) -> Self {
        ProminentBar { common: Common::new(), parts, widths }.label(label).ltr_axis()
    }
}

/// Three parts, for the suite.
const SAMPLE: [ProminentPart; 3] = [
    ProminentPart { row: 0, color: Srgba::rgb(0.85, 0.3, 0.25) },
    ProminentPart { row: 1, color: Srgba::rgb(0.25, 0.55, 0.85) },
    ProminentPart { row: 2, color: Srgba::rgb(0.95, 0.8, 0.3) },
];

/// A three-part bar, for the suite.
fn sample() -> ProminentBar<'static> {
    ProminentBar::new("Prominent Colors", &SAMPLE, Bind::given(vec![0.5, 0.3, 0.2]))
}

impl Component for ProminentBar<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = ProminentBarStyle;
    type Output = EditResponse<Vec<f64>>;

    fn style(cx: &StyleCx<'_>, _: ProminentBarStyleKey) -> ProminentBarStyle {
        let tokens = cx.tokens;
        ProminentBarStyle {
            metrics: Metrics::new(),
            height: tokens.size.handle.md,
            length: tokens.size.length.track,
            grab: tokens.size.hit.target,
            edge: cx.color(tokens.color.line.control_edge),
            accent: cx.color(tokens.color.accent.solid),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let width = cx.room.filter(|room| room.is_finite()).unwrap_or(cx.style.length);
        Vec2::new(width, cx.style.height)
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, Vec<f64>>> {
        Some(self.widths.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(widths) = cx.value().cloned() else { return };
        let grab = cx.style.grab;
        for (n, (part, span)) in self.parts.iter().zip(spans(rect, &widths)).enumerate() {
            let share = widths.get(n).copied().map(percent).unwrap_or_default();
            cx.part(PART.at(n as u64), span).tooltip(format!("{} {share}", crate::i18n::colon(&color_name(part.row))));
        }
        for (k, x) in edges(rect, &widths).into_iter().enumerate() {
            cx.part(EDGE.at(k as u64), Rect::from_center_size(pos2(x, rect.center().y), vec2(grab, rect.height())))
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(CursorIcon::ResizeHorizontal)
                .pointer_only("the Recolor Artwork dialog's weights field sets the shares");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(widths) = cx.value().cloned() else { return };
        let width = f64::from(cx.rect.width());
        for k in 0..widths.len().saturating_sub(1) {
            let press = cx.part(EDGE.at(k as u64)).press;
            match press.phase {
                PressPhase::DragStart { start, pos, .. } => {
                    let drag = Drag { edge: k, base: widths.clone(), from: start.origin.x };
                    cx.begin();
                    cx.propose(drag.to(pos.x, width));
                    cx.memory::<Held>().drag = Some(drag);
                }
                PressPhase::Drag { pos, .. } => {
                    if let Some(drag) = cx.memory::<Held>().drag.clone() {
                        cx.propose(drag.to(pos.x, width));
                    }
                }
                // A release with no drag before it held nothing: the click changes no width.
                PressPhase::Release { pos, .. } => {
                    if let Some(drag) = cx.memory::<Held>().drag.take() {
                        cx.propose(drag.to(pos.x, width));
                        cx.commit();
                    }
                }
                PressPhase::Cancel { .. } => cx.memory::<Held>().drag = None,
                _ => {}
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(widths) = cx.value() else { return };
        for (n, (part, width)) in self.parts.iter().zip(widths).enumerate() {
            let node = access.node(PART.at(n as u64), Role::GRAPHICS_OBJECT);
            node.name = Name::Text(color_name(part.row));
            node.value(percent(*width));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let draw = ui.draw();
        if self.parts.is_empty() {
            draw.stroke(&PaintShape::Rect(cx.rect, CornerRadius::ZERO), StrokeWidth::Thin, style.edge);
            return;
        }
        let widths = cx.value().cloned().unwrap_or_default();
        for (part, span) in self.parts.iter().zip(spans(cx.rect, &widths)) {
            draw.data_fill(&PaintShape::Rect(span, CornerRadius::ZERO), DataColor::new(Color32::from(part.color)));
        }
        let dragged = cx.recall::<Held>().drag.as_ref().map(|drag| drag.edge);
        for (k, x) in edges(cx.rect, &widths).into_iter().enumerate() {
            let lit = match dragged {
                Some(edge) => edge == k,
                None => cx.hover_pos(EDGE.at(k as u64)).is_some(),
            };
            if lit {
                draw.stroke(&PaintShape::Path(vec![pos2(x, cx.rect.top()), pos2(x, cx.rect.bottom())]), StrokeWidth::Thick, style.accent);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Vec<f64>> {
        cx.edit_response()
    }
}
