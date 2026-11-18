//! The Tools panel floats by its title bar and docks again on the window's left edge
//! ([`crate::toolbar`]). It stays outside the docking layout: dragging its title bar out of the
//! docked panel floats it under the pointer, a floating panel's title bar moves it, and a release
//! over the strip along the window's left edge (lit up) docks it there. A release over the strip
//! along the canvas's left edge, right of any panels docked on the left, docks it beside the
//! canvas. The release sets Keep Tools Beside the Canvas (`UiState::tools_beside_canvas`) to the
//! place it docked at. With no panels docked on the left, both strips put Tools in the same
//! place (left of any toolbars made with New Toolbar docked there), and a drop on either docks it
//! where the setting puts it and leaves the setting as it is; so does a drop in the column both
//! strips share when the panels are collapsed narrower than the window's strip. A release over
//! the canvas's right edge (left of the panels) or the window's right edge docks it on the right,
//! between the canvas and the panels.
//! Escape or losing the window's focus puts it back where the drag began.
//!
//! `window.panel.float {panel: "tools"}` / `window.panel.dock {panel: "tools"}` do the same for
//! agents (`side: right` docks it on the right). Its position and side are kept with the preferences and in user workspaces, and a position saved
//! on a bigger window is clamped into this one when drawn.

use egui::{CornerRadius, Id, Key, Pos2, Rect, Stroke, Vec2, pos2, vec2};
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
    /// The frame that floated it from the window's left edge, whose canvas-edge strip was laid out
    /// right of the docked panel and is not a target.
    floated_from_edge: Option<u64>,
}

fn move_id() -> Id {
    Id::unique("tools-float-move")
}

fn escape_cancel_id() -> Id {
    Id::unique("tools-float-escape-cancel")
}

pub(crate) fn escape_cancelled_move(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<u64>(escape_cancel_id())) == Some(ctx.cumulative_frame_nr_for(ctx.viewport_id()))
}

/// This frame's Escape put a moved bar back ([`escape_cancelled_move`]): a toolbar made with New
/// Toolbar ([`crate::custom_toolbars::cancel_move_on_escape`]) as well as the Tools panel.
pub(crate) fn mark_escape_cancelled(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr_for(ctx.viewport_id());
    ctx.data_mut(|d| d.insert_temp(escape_cancel_id(), frame));
}

/// The strip along the canvas's left edge where the Tools panel docks beside the canvas, as last
/// laid out.
pub(crate) fn tools_zone_id() -> Id {
    Id::unique("tools-float-zone")
}

/// The strip along the canvas's right edge, left of the panels, where the Tools panel docks on the
/// right, as last laid out.
pub(crate) fn tools_zone_right_id() -> Id {
    Id::unique("tools-float-zone-right")
}

/// Accept a drop left of the panels docked right: the canvas's right edge.
pub(crate) fn set_right_dock_zone(ctx: &egui::Context, canvas: Rect, width: f32) {
    let zone = Rect::from_min_max(pos2(canvas.right() - width, canvas.top()), canvas.max);
    ctx.data_mut(|d| d.insert_temp(tools_zone_right_id(), zone));
}

fn window_zone_id() -> Id {
    Id::unique("tools-float-window-zone")
}

/// Accept a drop next to panels docked left, independently of the window-edge gesture.
pub(crate) fn set_dock_zones(ctx: &egui::Context, canvas: Rect, width: f32) {
    ctx.data_mut(|d| {
        d.insert_temp(tools_zone_id(), Rect::from_min_size(canvas.min, vec2(width, canvas.height())));
    });
}

fn dock_area_id() -> Id {
    Id::unique("tools-float-dock-area-left")
}

/// The left edge of the space the canvas and the panels share, as last laid out: right of the
/// lane of New Toolbar toolbars docked on the left, and of Tools docked at the window's edge.
pub(crate) fn set_dock_area_left(ctx: &egui::Context, left: f32) {
    ctx.data_mut(|d| d.insert_temp(dock_area_id(), left));
}

/// The legacy 48-point window-edge target spans the full layout below the bars, including
/// panels above and below the canvas. Its width does not follow the toolbar's column count.
pub(crate) fn set_window_dock_zone(ctx: &egui::Context, window: Rect) {
    ctx.data_mut(|d| d.insert_temp(window_zone_id(), Rect::from_min_size(window.min, vec2(48.0, window.height()))));
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

/// Cancel a move before shortcuts route the same Escape press.
pub(crate) fn cancel_move_on_escape(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        track(app, ctx);
    }
}

fn start(app: &VectorcraftApp, ctx: &egui::Context, grab: Vec2, floated_from_edge: Option<u64>) {
    let Some(from) = ctx.input(|i| i.pointer.press_origin().or(i.pointer.interact_pos())) else { return };
    let m = Move { grab, from, original: app.ui.toolbar_pos, right: app.ui.toolbar_right, floated_from_edge };
    ctx.data_mut(|d| d.insert_temp(move_id(), m));
}

/// The Tools panel's title bar was pressed (`title`, the panel's bounds `bounds`): docked, dragging
/// it out of the panel floats the panel under the pointer; floating at `floating`, a drag moves it.
pub(crate) fn drag_tools(app: &mut VectorcraftApp, ctx: &egui::Context, title: &egui::Response, bounds: Rect, floating: Option<Pos2>) {
    if !gesture_allowed(ctx) || ctx.data(|d| d.get_temp::<Move>(move_id()).is_some()) {
        return;
    }
    let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    match floating {
        Some(pos) if title.drag_started() => start(app, ctx, at - pos, None),
        None if title.dragged() && !bounds.contains(at) => {
            // Held where the press grabbed the title bar.
            let grab = ctx.input(|i| i.pointer.press_origin()).map_or(vec2(10.0, 7.0), |p| p - bounds.min);
            let at_edge = !app.ui.tools_beside_canvas && !app.ui.toolbar_right;
            start(app, ctx, grab, at_edge.then(|| ctx.cumulative_frame_nr_for(ctx.viewport_id())));
            let p = at - grab;
            app.ui.toolbar_pos = Some([p.x, p.y]);
        }
        _ => {}
    }
}

/// Where a release docks the Tools panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dock {
    /// On the left: beside the canvas (`Some(true)`), at the window's edge (`Some(false)`), or
    /// where Keep Tools Beside the Canvas puts it (`None`, where the two strips share a column).
    Left(Option<bool>),
    /// On the right, between the canvas and the panels.
    Right,
}

/// The docking strip under `at`, if any, and where it docks the panel: the window's left edge,
/// the canvas's left edge, and on the right the canvas's right edge or anywhere along the
/// window's right edge (which lights up the strip left of the panels, where it docks). With no
/// panels docked on the left, both left strips put Tools in the same place, and a drop on either
/// docks it where `beside_canvas` puts it; with panels collapsed narrower than the window's strip,
/// so does a drop in the column the two strips share. The canvas's strip lights up where both are
/// under `at` and `beside_canvas` is on; with no panels docked on the left, the window's strip lights
/// up, where Tools docks. Without `canvas_laid_out`, the canvas's strip is skipped.
fn drop_zone(ctx: &egui::Context, at: Pos2, beside_canvas: bool, canvas_laid_out: bool) -> Option<(Rect, Dock)> {
    let (canvas, window, right, area_left) = ctx.data(|d| {
        let canvas = d.get_temp::<Rect>(tools_zone_id()).filter(|_| canvas_laid_out);
        (canvas, d.get_temp::<Rect>(window_zone_id()), d.get_temp::<Rect>(tools_zone_right_id()), d.get_temp::<f32>(dock_area_id()))
    });
    let panels_left = matches!((canvas, area_left), (Some(c), Some(left)) if c.left() > left + 0.5);
    let shared = !panels_left || matches!((canvas, window), (Some(c), Some(w)) if c.x_range().contains(at.x) && w.x_range().contains(at.x));
    let follow = |beside: bool| Dock::Left((!shared).then_some(beside));
    // With no panels docked on the left, Tools docks at the window's edge either way.
    let shown = |hit: Rect| if panels_left { hit } else { window.unwrap_or(hit) };
    match (canvas.filter(|r| r.contains(at)), window.filter(|r| r.contains(at))) {
        (Some(c), Some(w)) => return Some((shown(if beside_canvas { c } else { w }), follow(beside_canvas))),
        (Some(c), None) => return Some((shown(c), follow(true))),
        (None, Some(w)) => return Some((w, follow(false))),
        (None, None) => {}
    }
    let r = right?;
    // Read outside the data lock above: egui's context locks are not re-entrant.
    let at_edge = at.x >= ctx.content_rect().right() - crate::toolbar::WIDTH && r.y_range().contains(at.y);
    (r.contains(at) || at_edge).then_some((r, Dock::Right))
}

/// The strip that lights up with the pointer at `at` during a move.
#[cfg(test)]
pub(crate) fn drop_highlight(ctx: &egui::Context, at: Pos2, beside_canvas: bool) -> Option<Rect> {
    drop_zone(ctx, at, beside_canvas, true).map(|(zone, _)| zone)
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
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            let frame = ctx.cumulative_frame_nr_for(ctx.viewport_id());
            ctx.data_mut(|d| d.insert_temp(escape_cancel_id(), frame));
        }
        return;
    }
    let at = ctx.input(|i| i.pointer.interact_pos());
    // The frame after Tools floated from the window's edge reads the canvas-edge strip that frame
    // laid out right of the docked panel.
    let frame = ctx.cumulative_frame_nr_for(ctx.viewport_id());
    let canvas_laid_out = m.floated_from_edge.is_none_or(|f| frame > f + 1);
    let zone = at.filter(|at| at.distance(m.from) > TRAVEL).and_then(|at| drop_zone(ctx, at, app.ui.tools_beside_canvas, canvas_laid_out));
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
    match zone {
        Some((_, Dock::Left(beside))) => {
            app.ui.toolbar_pos = None;
            app.ui.toolbar_right = false;
            if let Some(beside) = beside {
                app.ui.tools_beside_canvas = beside;
            }
        }
        Some((_, Dock::Right)) => {
            app.ui.toolbar_pos = None;
            app.ui.toolbar_right = true;
        }
        None => {}
    }
}

/// The `x`, `y` params: a top-left corner in screen points, if given.
pub(crate) fn position(p: &Value) -> Result<Option<Pos2>, String> {
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
    // Docking: on the left, or on the right left of the panels (default: the side it docked at last).
    let right = match p.get("side") {
        None | Some(Value::Null) => Ok(app.ui.toolbar_right),
        // `Start` and `End`, as panels take them, too.
        Some(Value::String(s)) if s.eq_ignore_ascii_case("left") || s.eq_ignore_ascii_case("start") => Ok(false),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("right") || s.eq_ignore_ascii_case("end") => Ok(true),
        Some(Value::String(s)) => Err(format!("side is left or right, not `{s}`")),
        // A number, a list…: an error, not the last side.
        Some(v) => Err(format!("side is left or right, not `{v}`")),
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
