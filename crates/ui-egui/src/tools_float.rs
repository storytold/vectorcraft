//! The Tools panel floats by its title bar and docks again on the window's left edge, or on the
//! right, left of the panels ([`crate::toolbar`]). It stays outside the docking layout: dragging
//! its title bar out of the docked panel floats it under the pointer, a floating panel's title bar
//! moves it, and a release over the strip along the window's left edge, or the strip left of the
//! panels or along the window's right edge (lit up), docks it there. Escape or losing the
//! window's focus puts it back where the drag began.
//!
//! `window.panel.float {panel: "tools"}` / `window.panel.dock {panel: "tools", side?}` do the same
//! for agents. Its position and side are kept with the preferences and in user workspaces, and a
//! position saved on a bigger window is clamped into this one when drawn.

use egui::{CornerRadius, Id, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::theme::Tokens;

/// Where `window.panel.float {panel: "tools"}` floats the Tools panel given no position.
const TOOLS_AT: [f32; 2] = [60.0, 120.0];
/// Points the pointer travels from its press before a drop counts (a click on the title bar of a
/// panel floating over the left edge doesn't dock it).
const TRAVEL: f32 = 4.0;

/// A move in progress: where the pointer holds the panel from its top-left corner, where the press
/// began, and where the panel was before it (to put it back on Escape).
#[derive(Clone, Debug)]
struct Move {
    grab: Vec2,
    from: Pos2,
    original: Option<[f32; 2]>,
    /// The side it was docked on before the drag.
    right: bool,
}

fn move_id() -> Id {
    Id::unique("tools-float-move")
}

/// The strip along the window's left edge where the Tools panel docks, as last laid out.
pub(crate) fn tools_zone_id() -> Id {
    Id::unique("tools-float-zone")
}

/// The strip left of the panels (along the canvas's right edge) where the Tools panel docks on the
/// right, as last laid out.
pub(crate) fn tools_zone_right_id() -> Id {
    Id::unique("tools-float-zone-right")
}

/// The canvas's right edge as last laid out: a Tools panel docked right stands just inside it,
/// left of the panels.
pub(crate) fn canvas_right_id() -> Id {
    Id::unique("tools-float-canvas-right")
}

/// `pos` moved so a panel of `size` stays inside `screen` (a position saved on a bigger window).
pub fn clamp(pos: [f32; 2], size: Vec2, screen: Rect) -> Pos2 {
    let [x, y] = pos;
    let (x, y) = if x.is_finite() && y.is_finite() { (x, y) } else { (screen.left() + 60.0, screen.top() + 100.0) };
    let fit = |v: f32, max: f32| if v > max { max.floor() } else { v };
    pos2(fit(x, screen.right() - size.x).max(screen.left()), fit(y, screen.bottom() - size.y).max(screen.top()))
}

fn gesture_allowed(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.focused && !input.key_pressed(egui::Key::Escape))
}

fn start(app: &VectorcraftApp, ctx: &egui::Context, grab: Vec2) {
    let Some(from) = ctx.input(|i| i.pointer.press_origin().or(i.pointer.interact_pos())) else { return };
    ctx.data_mut(|d| d.insert_temp(move_id(), Move { grab, from, original: app.ui.toolbar_pos, right: app.ui.toolbar_right }));
}

/// The Tools panel's title bar was pressed (`title`, the panel's bounds `bounds`): docked, dragging
/// it out of the panel floats the panel under the pointer; floating at `floating`, a drag moves it.
pub(crate) fn drag_tools(app: &mut VectorcraftApp, ctx: &egui::Context, title: &egui::Response, bounds: Rect, floating: Option<Pos2>) {
    if !gesture_allowed(ctx) || ctx.data(|d| d.get_temp::<Move>(move_id()).is_some()) {
        return;
    }
    let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    match floating {
        Some(pos) if title.drag_started() => start(app, ctx, at - pos),
        None if title.dragged() && !bounds.contains(at) => {
            // Held where the press grabbed the title bar.
            let grab = ctx.input(|i| i.pointer.press_origin()).map_or(vec2(10.0, 7.0), |p| p - bounds.min);
            start(app, ctx, grab);
            let p = at - grab;
            app.ui.toolbar_pos = Some([p.x, p.y]);
        }
        _ => {}
    }
}

/// The docking strip under `at`, if any, and whether it docks the panel on the right. Left of the
/// panels or anywhere along the window's right edge, it docks left of the panels, so that strip
/// lights up.
fn drop_zone(ctx: &egui::Context, at: Pos2) -> Option<(Rect, bool)> {
    let (left, right) = ctx.data(|d| (d.get_temp::<Rect>(tools_zone_id()), d.get_temp::<Rect>(tools_zone_right_id())));
    if let Some(r) = left.filter(|r| r.contains(at)) {
        return Some((r, false));
    }
    let r = right?;
    let at_edge = at.x >= ctx.content_rect().right() - crate::toolbar::WIDTH && r.y_range().contains(at.y);
    (r.contains(at) || at_edge).then_some((r, true))
}

fn highlight(ctx: &egui::Context, zone: Rect) {
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("tools-float-drop")));
    painter.rect(zone.shrink(1.0), CornerRadius::same(2), t.accent.gamma_multiply(0.2), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
}

/// Carry a move on (at the start of a frame, before the panels are drawn): the panel follows the
/// pointer and the docking strip lights up under it; the release docks it there or leaves it
/// floating where it is.
pub fn track(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(m) = ctx.data(|d| d.get_temp::<Move>(move_id())) else { return };
    if !gesture_allowed(ctx) {
        app.ui.toolbar_pos = m.original;
        app.ui.toolbar_right = m.right;
        ctx.data_mut(|d| d.remove::<Move>(move_id()));
        return;
    }
    let at = ctx.input(|i| i.pointer.interact_pos());
    let zone = at.filter(|at| at.distance(m.from) > TRAVEL).and_then(|at| drop_zone(ctx, at));
    if ctx.input(|i| i.pointer.primary_down()) {
        if let Some(p) = at.map(|at| at - m.grab)
            && p.x.is_finite()
            && p.y.is_finite()
            && app.ui.toolbar_pos.is_some()
        {
            app.ui.toolbar_pos = Some([p.x, p.y]);
        }
        if let Some((zone, _)) = zone {
            highlight(ctx, zone);
        }
        return;
    }
    ctx.data_mut(|d| d.remove::<Move>(move_id()));
    if let Some((_, right)) = zone {
        app.ui.toolbar_pos = None;
        app.ui.toolbar_right = right;
    }
}

/// The `x`, `y` params: a top-left corner in screen points, if given.
fn position(p: &Value) -> Result<Option<Pos2>, String> {
    let coord = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Some(None),
        Some(v) => v.as_f64().filter(|f| f.abs() < 1e6).map(|f| Some(f as f32)),
    };
    match (coord("x"), coord("y")) {
        (Some(Some(x)), Some(Some(y))) => Ok(Some(pos2(x, y))),
        (Some(None), Some(None)) => Ok(None),
        _ => Err("x and y must both be numbers".into()),
    }
}

/// `window.panel.float` / `window.panel.dock` for the Tools panel (`panel: "tools"`); `None` for
/// any other panel.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let float = match id {
        "window.panel.float" => true,
        "window.panel.dock" => false,
        _ => return None,
    };
    let raw = p.get("panel").and_then(Value::as_str)?.trim();
    if !(raw.eq_ignore_ascii_case("tools") || raw.eq_ignore_ascii_case("toolbar")) {
        return None;
    }
    // Docking: at the window's left edge, or on the right left of the panels (default: the side it
    // docked at last).
    let right = match p.get("side").and_then(Value::as_str) {
        None => Ok(app.ui.toolbar_right),
        // `Start` and `End`, as panels take them, too.
        Some(s) if s.eq_ignore_ascii_case("left") || s.eq_ignore_ascii_case("start") => Ok(false),
        Some(s) if s.eq_ignore_ascii_case("right") || s.eq_ignore_ascii_case("end") => Ok(true),
        Some(s) => Err(format!("side is left or right, not `{s}`")),
    };
    Some(position(p).and_then(|at| {
        let right = right?;
        app.ui.toolbar_pos = float.then(|| at.map_or(TOOLS_AT, |p| [p.x, p.y]));
        app.ui.toolbar |= float;
        if !float {
            app.ui.toolbar_right = right;
        }
        let side = if app.ui.toolbar_right { "right" } else { "left" };
        Ok(json!({ "panel": "tools", "floating": float, "pos": app.ui.toolbar_pos, "side": side }))
    }))
}
