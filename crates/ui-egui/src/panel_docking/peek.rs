//! Panels hidden by Tab or Shift+Tab, shown for a while from a window edge
//! (`window.panels.peek`), as resting the pointer on that edge shows them. The window's shell
//! reveals the dock on that edge until the pointer moves away; Tab or Shift+Tab shows the panels
//! for good.

use craft::behavior::Axis;
use craft::behavior::dock::{DockEdge, Layout, Node, tree_panels};
use craft::ui::runtime::env::RevealEdge;
use craft::ui::runtime::frame::{ChromeToggle, request_chrome_toggle};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// A side of the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Left,
    Right,
}

// PLAN: left and right map to the Start and End edges as in a left-to-right window; a command run outside a pass has no direction to read.
impl Edge {
    /// The side's name in `window.panels.peek` and `ui.docking`.
    pub(super) fn id(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    /// The dock edge on this side.
    fn dock(self) -> DockEdge {
        match self {
            Edge::Left => DockEdge::Start,
            Edge::Right => DockEdge::End,
        }
    }

    /// The window edge the shell reveals for this side.
    fn reveal(self) -> RevealEdge {
        match self {
            Edge::Left => RevealEdge::Start,
            Edge::Right => RevealEdge::End,
        }
    }

    /// The side a revealed window edge is; none for the top edge.
    pub(super) fn of_reveal(edge: RevealEdge) -> Option<Edge> {
        match edge {
            RevealEdge::Start => Some(Edge::Left),
            RevealEdge::End => Some(Edge::Right),
            _ => None,
        }
    }
}

/// The panels docked on `edge`'s side of the canvas. Tool rails are not among them: Shift+Tab
/// keeps them, and a peek shows panels.
pub(crate) fn edge_panels(layout: &Layout<String>, edge: Edge) -> Vec<String> {
    let mut out: Vec<String> = super::edge_columns(layout, edge.dock()).iter().flat_map(|c| c.panels()).cloned().collect();
    // Panels split off beside the canvas outside a dock: the other child of each side-by-side
    // split on the way down to the canvas.
    let canvas = super::CANVAS.to_string();
    let mut node = layout.root.as_ref();
    while let Some(Node::Split { axis, first, second, .. }) = node {
        let (inner, outer, side) = if tree_panels(first).contains(&&canvas) { (first, second, Edge::Right) } else { (second, first, Edge::Left) };
        if *axis == Axis::X && side == edge {
            for panel in tree_panels(outer) {
                if !out.contains(panel) {
                    out.push(panel.clone());
                }
            }
        }
        node = Some(inner);
    }
    out.retain(|p| !super::is_rail(p));
    out
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
    if super::panels_shown(app) {
        return Err("the panels are not hidden".into());
    }
    if let Some(edge) = edge
        && edge_panels(super::layout_of(&app.ui), edge).is_empty()
    {
        return Err(format!("no panels are docked on the {} side", edge.id()));
    }
    if let Some(ctx) = app.ui_ctx.clone() {
        // A flyout opened from the panels shown closes with them.
        if edge.is_none() && craft::ui::shell::chrome_state(&ctx, egui::ViewportId::ROOT).revealed.is_some() {
            app.ui.open_panel = None;
        }
        request_chrome_toggle(&ctx, egui::ViewportId::ROOT, ChromeToggle::Reveal(edge.map(Edge::reveal)));
    }
    Ok(edge.map_or(Value::Null, |edge| json!(edge.id())))
}
