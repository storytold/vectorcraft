//! Panels hidden by Tab or Shift+Tab, shown for a while: resting the pointer on the left or right
//! edge of the dock area shows the docked panels on that side of the canvas
//! (`window.panels.peek`). They hide again once the pointer has been away from them for
//! [`GRACE`] seconds; Tab or Shift+Tab shows them for good. The canvas keeps the size it has with
//! every panel hidden, so the panels cover its edge and the artwork stays where it is. Floating
//! panels, the Tools panel and the Control bar stay hidden.

use craft_ui::docking::{Layout, Node};
use craft_ui::layout::SplitAxis;
use serde_json::{Value, json};

use super::CANVAS;
use crate::VectorcraftApp;

/// Width of the band along each edge of the dock area that shows the panels on that side.
pub(crate) const EDGE: f32 = 6.0;
/// How long the pointer rests on an edge band before the panels show, in seconds.
pub(crate) const DELAY: f64 = 0.3;
/// How long the pointer stays away from the panels shown before they hide, in seconds.
pub(crate) const GRACE: f64 = 0.3;

/// A side of the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Left,
    Right,
}

impl Edge {
    fn id(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }
}

/// The panels shown on one side.
#[derive(Clone, Copy, Debug)]
struct Shown {
    edge: Edge,
    /// When the pointer left them; `None` while it is over them or has not moved since they showed.
    left_at: Option<f64>,
    /// Whether a press, a modal dialog, a popup or a flyout held them open last frame.
    held: bool,
}

/// The session's state of the panels shown from the edges.
pub(crate) struct State {
    /// Whether Tab or Shift+Tab hides the panels this frame.
    hidden: bool,
    /// The dock area this frame.
    area: egui::Rect,
    /// The edge band the pointer rests on, and since when.
    resting: Option<(Edge, f64)>,
    shown: Option<Shown>,
    /// How far the canvas's corners lay from the dock area's with every panel hidden.
    canvas: Option<(egui::Vec2, egui::Vec2)>,
    /// The part of the canvas left uncovered while panels show.
    uncovered: Option<egui::Rect>,
}

impl Default for State {
    fn default() -> Self {
        Self { hidden: false, area: egui::Rect::NOTHING, resting: None, shown: None, canvas: None, uncovered: None }
    }
}

/// The docked panels on `edge`'s side of the canvas, at any depth of horizontal splits.
pub(crate) fn edge_panels(layout: &Layout<String>, edge: Edge) -> Vec<String> {
    let canvas = CANVAS.to_string();
    let tree = |node: &Node<String>| Layout { root: Some(node.clone()), floating: Vec::new() };
    let mut out = Vec::new();
    let mut node = layout.root.as_ref();
    let mut depth = 0usize;
    while let Some(Node::Split { axis, first, second, .. }) = node {
        depth = depth.saturating_add(1);
        if depth > 2 * craft_ui::docking::MAX_PANELS {
            break;
        }
        let (inner, outer, side) = if tree(first).contains(&canvas) { (first, second, Edge::Right) } else { (second, first, Edge::Left) };
        if matches!(axis, SplitAxis::Horizontal) && side == edge {
            out.extend(tree(outer).panels().into_iter().filter(|p| **p != canvas).cloned());
        }
        node = Some(inner);
    }
    out
}

/// The side whose panels show this frame. `area` is the dock area and `blocked` whether a modal
/// dialog or a popup takes the input.
pub(super) fn update(app: &mut VectorcraftApp, ctx: &egui::Context, area: egui::Rect, layout: &Layout<String>, blocked: bool) -> Option<Edge> {
    let hidden = !app.ui.dock && app.ui.screen_mode < 3;
    let flyout = app.ui.open_panel.is_some();
    // The docked Tools panel's rect, read only while Tools shows (Tab turns `toolbar` off).
    let tools = app.ui.toolbar.then(|| ctx.data(|d| d.get_temp::<egui::Rect>(crate::toolbar::docked_rect_id()))).flatten();
    let st = &mut app.dock_rt.peek;
    st.hidden = hidden;
    st.area = area;
    if !hidden {
        st.resting = None;
        st.shown = None;
        st.uncovered = None;
        return None;
    }
    let (now, pointer, down, moved) = ctx.input(|i| {
        // A release counts as moving away: a press may have held the panels while it left them. A
        // pointer entering the window has no delta on its first frame, so the move event counts too.
        let moved = i.pointer.delta() != egui::Vec2::ZERO
            || i.pointer.any_released()
            || i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerGone));
        (i.time, i.pointer.hover_pos(), i.pointer.any_down(), moved)
    });
    // A workspace switched to while panels show may dock none on that side.
    if st.shown.is_some_and(|shown| edge_panels(layout, shown.edge).is_empty()) {
        st.shown = None;
        st.uncovered = None;
    }
    if let Some(shown) = st.shown.as_mut() {
        // The panels shown sit beside the canvas, level with it.
        let over = pointer.is_some_and(|p| {
            let uncovered = st.uncovered.unwrap_or_else(|| area.shrink2(egui::vec2(EDGE, 0.0)));
            area.contains(p)
                && (uncovered.top()..=uncovered.bottom()).contains(&p.y)
                && match shown.edge {
                    Edge::Left => p.x < uncovered.left(),
                    Edge::Right => p.x > uncovered.right(),
                }
        });
        let held = down || blocked || flyout;
        if over || held {
            shown.left_at = None;
        } else if moved || shown.held {
            // A hold that ends with the pointer away from them counts as moving away, as closing a
            // popup or flyout with Escape does not move the pointer.
            shown.left_at.get_or_insert(now);
        }
        shown.held = held;
        match shown.left_at {
            Some(left) if now - left >= GRACE => {
                st.shown = None;
                st.uncovered = None;
                // A flyout opened from the panels shown closes with them.
                app.ui.open_panel = None;
                return None;
            }
            Some(left) => ctx.request_repaint_after_secs((GRACE - (now - left)) as f32),
            None => {}
        }
        return Some(shown.edge);
    }
    let edge = pointer.filter(|p| area.contains(*p) && !tools.is_some_and(|r| r.contains(*p))).and_then(|p| {
        if p.x <= area.left() + EDGE {
            Some(Edge::Left)
        } else if p.x >= area.right() - EDGE {
            Some(Edge::Right)
        } else {
            None
        }
    });
    let Some(edge) = edge.filter(|e| !down && !blocked && !edge_panels(layout, *e).is_empty()) else {
        st.resting = None;
        return None;
    };
    let since = match st.resting {
        Some((rested, since)) if rested == edge => since,
        _ => now,
    };
    if now - since >= DELAY {
        st.resting = None;
        st.shown = Some(Shown { edge, left_at: None, held: false });
        return Some(edge);
    }
    st.resting = Some((edge, since));
    ctx.request_repaint_after_secs((DELAY - (now - since)) as f32);
    // The strip the pointer rests on.
    let band = match edge {
        Edge::Left => egui::Rect::from_min_max(area.left_top(), egui::pos2(area.left() + EDGE, area.bottom())),
        Edge::Right => egui::Rect::from_min_max(egui::pos2(area.right() - EDGE, area.top()), area.right_bottom()),
    };
    let t = crate::theme::Tokens::get(ctx);
    let layer = egui::LayerId::new(egui::Order::Foreground, super::area_id().with("peek-strip"));
    ctx.layer_painter(layer).rect_filled(band, 0.0, t.accent.gamma_multiply(0.6));
    None
}

/// The side shown, for `ui.docking`.
pub(super) fn shown(app: &VectorcraftApp) -> Option<&'static str> {
    app.dock_rt.peek.shown.map(|s| s.edge.id())
}

/// Draw the canvas panel's body with `draw`. While panels show from an edge, the canvas keeps the
/// rect it has with every panel hidden, drawn under the panels and taking input only where they
/// leave it uncovered.
pub(super) fn canvas(app: &mut VectorcraftApp, ui: &mut egui::Ui, draw: impl FnOnce(&mut VectorcraftApp, &mut egui::Ui)) {
    let uncovered = ui.max_rect();
    let st = &mut app.dock_rt.peek;
    let wide = if !st.hidden {
        st.canvas = None;
        None
    } else if st.shown.is_none() {
        st.canvas = Some((uncovered.min - st.area.min, uncovered.max - st.area.max));
        None
    } else {
        st.uncovered = Some(uncovered);
        // The canvas's insets in the dock area with every panel hidden, kept as the window resizes;
        // none before such a frame was drawn, as it fills the area then.
        let (min, max) = st.canvas.unwrap_or((egui::Vec2::ZERO, egui::Vec2::ZERO));
        Some(egui::Rect::from_min_max(st.area.min + min, st.area.max + max))
    };
    match wide {
        Some(full) if full != uncovered && full.is_positive() => {
            let clip = uncovered.intersect(ui.clip_rect());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(full));
            child.set_clip_rect(clip);
            draw(app, &mut child);
        }
        _ => draw(app, ui),
    }
}

/// `window.panels.peek {side: left|right|null}`: show the docked panels on that side while Tab or
/// Shift+Tab hides the panels, as resting the pointer on that edge does, or (null) hide them.
pub(super) fn command(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let edge = match params.get("side") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "left" => Some(Edge::Left),
        Some(Value::String(s)) if s == "right" => Some(Edge::Right),
        Some(_) => return Err("`side` must be left, right or null".into()),
    };
    if app.ui.screen_mode >= 3 {
        return Err("panels can't be shown in Presentation Mode".into());
    }
    if app.ui.dock {
        return Err("the panels are not hidden".into());
    }
    match edge {
        Some(edge) => {
            if edge_panels(&super::layout_of(&app.ui), edge).is_empty() {
                return Err(format!("no panels are docked on the {} side", edge.id()));
            }
            let st = &mut app.dock_rt.peek;
            st.resting = None;
            st.shown = Some(Shown { edge, left_at: None, held: false });
            Ok(json!(edge.id()))
        }
        None => {
            let st = &mut app.dock_rt.peek;
            if st.shown.take().is_some() {
                st.uncovered = None;
                app.ui.open_panel = None;
            }
            Ok(Value::Null)
        }
    }
}
