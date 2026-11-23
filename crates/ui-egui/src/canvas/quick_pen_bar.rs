//! The Quick Pen toolbar: a floating bar on the canvas while the Quick Pen tool is active. It holds
//! the tool's actions (Straight Line, Continuous Arc, Sharp Corner Arc), Flip Arc for the last
//! sharp-corner arc, and a More Options menu with Hide Toolbar, Pin Toolbar Position and Reset
//! Toolbar Position. Its handle, at its left end, drags it.
//!
//! Public behavior: selecting the tool shows the toolbar, its actions decide what a click or drag
//! draws, Flip Arc flips the sharp-corner arc just drawn, the More menu hides the toolbar and resets
//! its position, and the toolbar moves by dragging its left side
//! ([Draw with the Quick Pen tool](https://helpx.adobe.com/illustrator/web/add-and-edit-artwork/draw-and-edit-paths/draw-with-quick-pen-tool.html)).
//! The toolbar follows the drawing, beside the point the path goes on from and away from what was
//! drawn ([Behind the design: the Quick Pen tool](https://adobe.design/ideas/behind-the-design-adobe-illustrator-s-quick-pen-tool)).
//! VectorCraft's own rules: the exact place beside the point, moving to the drawn side where the
//! other side runs off the canvas, the place with no path being drawn, how a drag's offset is kept,
//! Pin Toolbar Position (which holds the toolbar at its place in the canvas window, not on the
//! artboard), and Window › Quick Pen Toolbar to show it again.

use vectorcraft_tools::quickpen::{self, Stroke as Action};

use super::{Color32, CornerRadius, Pos2, Sense, Stroke, Tokens, Ui, Vec2, VectorcraftApp, Xf, json, pos2, vec2, widgets};

/// The space between the point the path goes on from and the toolbar's nearest corner.
const GAP: f32 = 24.0;
/// The width of the toolbar's handle.
const HANDLE: f32 = 4.0;
/// The side of the toolbar's buttons.
const BUTTON: f32 = 28.0;
/// The space between the toolbar's controls.
const SPACING: f32 = 4.0;

fn bar_id() -> egui::Id {
    egui::Id::unique("quick-pen-bar")
}

/// Where the Quick Pen toolbar was last drawn (none while it isn't shown).
pub(crate) fn quick_pen_bar_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = bar_id();
    ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, id)).then(|| m.area_rect(id)).flatten())
}

/// The toolbar's top-left corner, of `size`, beside `end` (the point the path goes on from, on
/// screen), on the side the path arrives there from `dir` (on screen): to its right and below it
/// for a path drawn to the right and down, so the toolbar stays off what was drawn. A path of one
/// point puts it to the right and below. Where that side runs out of `room` (the canvas) and the
/// other side fits, it goes on the other side; where neither fits, on the side with more room.
/// Kept in `room` ([`keep_in`]), it covers `end` only when neither side fits across or down.
fn beside(end: Pos2, dir: Option<Vec2>, size: Vec2, room: egui::Rect) -> Pos2 {
    // `first` unless it runs out of room and `other` fits or runs out by less.
    let pick = |first: f32, other: f32, lo: f32, hi: f32, len: f32| {
        let over = |p: f32| (lo - p).max(0.0) + (p + len - hi).max(0.0);
        if over(other) < over(first) { other } else { first }
    };
    let (right, left) = (end.x + GAP, end.x - GAP - size.x);
    let (below, above) = (end.y + GAP, end.y - GAP - size.y);
    let x = if dir.is_none_or(|d| d.x >= 0.0) { (right, left) } else { (left, right) };
    let y = if dir.is_none_or(|d| d.y >= 0.0) { (below, above) } else { (above, below) };
    pos2(pick(x.0, x.1, room.left(), room.right(), size.x), pick(y.0, y.1, room.top(), room.bottom(), size.y))
}

/// The toolbar's place with no path being drawn: centered at the bottom of the canvas area `room`.
fn resting(room: egui::Rect, size: Vec2) -> Pos2 {
    pos2(room.center().x - size.x / 2.0, room.bottom() - size.y)
}

/// `p` moved, as little as it takes, for a toolbar of `size` to lie inside `room`.
fn keep_in(p: Pos2, size: Vec2, room: egui::Rect) -> Pos2 {
    p.clamp(room.min, (room.max - size).max(room.min))
}

/// Where an unpinned toolbar of `size` sits: `home` moved by the drag `offset`, kept in `room`,
/// unless that would cover `end` (the point the path goes on from, on screen), where it sits at
/// `home` as if never dragged.
fn unpinned(home: Pos2, offset: Vec2, end: Option<Pos2>, size: Vec2, room: egui::Rect) -> Pos2 {
    let moved = keep_in(home + offset, size, room);
    if end.is_some_and(|e| egui::Rect::from_min_size(moved, size).contains(e)) { keep_in(home, size, room) } else { moved }
}

/// What a click on the toolbar does.
enum Click {
    Action(Action),
    Run(&'static str, serde_json::Value),
}

/// Is a drag under way (past the drag threshold) that began on the canvas `canvas` outside the
/// toolbar's last place `bar`? A stroke, a Space or middle-button pan and a drag from a path's first
/// point all are; a drag of the toolbar's handle is not.
fn canvas_drag(dragging: bool, press: Option<Pos2>, canvas: egui::Rect, bar: Option<egui::Rect>) -> bool {
    dragging && press.is_some_and(|p| canvas.contains(p) && !bar.is_some_and(|b| b.contains(p)))
}

/// The Quick Pen toolbar, while the Quick Pen tool is active, Window › Quick Pen Toolbar is on and
/// no press is dragging on the canvas. Selecting the tool turns Window › Quick Pen Toolbar on
/// ([`VectorcraftApp::try_select_tool`]).
pub(super) fn quick_pen_bar(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    if !app.ui.quick_pen_bar || app.session.tool_id() != quickpen::ID {
        return;
    }
    let (dragging, press, down) = ui.input(|i| (i.pointer.is_decidedly_dragging(), i.pointer.press_origin(), i.pointer.any_down()));
    // The handle's drag moves the toolbar away from where the press began.
    let handle_id = bar_id().with("handle");
    if !down {
        ui.ctx().data_mut(|d| d.insert_temp(handle_id, false));
    }
    let on_handle = ui.ctx().data(|d| d.get_temp::<bool>(handle_id)).unwrap_or(false);
    let shown = app.ui.quick_pen_bar_place.shown_at.zip(egui::AreaState::load(ui.ctx(), bar_id()).and_then(|s| s.size));
    if canvas_drag(dragging && !on_handle, press, xf.rect, shown.map(|(at, size)| egui::Rect::from_min_size(xf.rect.min + at, size))) {
        return;
    }
    let Some(doc) = app.session.active().map(|st| st.uid) else { return };
    let view = app.view_info();
    let end = app.session.tool_drawing_end(view);
    let flip = app.session.tool_claims_key(vectorcraft_tools::ToolKey::Letter('c'), view);
    let current = app.session.tool_options().get("action").and_then(serde_json::Value::as_str).and_then(Action::parse).unwrap_or_default();
    let t = Tokens::get(ui.ctx());
    let id = bar_id();
    let bounds = xf.rect.shrink(8.0);
    let est = vec2(HANDLE + 5.0 * BUTTON + 6.0 * SPACING + 16.0, BUTTON + 12.0);
    let size = egui::AreaState::load(ui.ctx(), id).and_then(|s| s.size).unwrap_or(est);
    let end = end.map(|(p, dir)| {
        let at = xf.to_screen(p);
        (at, dir.map(|d| xf.to_screen(p + d) - at))
    });
    let drawing = end.is_some();
    let home = match end {
        Some((at, dir)) => beside(at, dir, size, bounds),
        None => resting(bounds, size),
    };
    let place = &mut app.ui.quick_pen_bar_place;
    // Just unpinned: follow the drawing from the pinned spot.
    if !place.pinned
        && let Some(at) = place.pin_at.take()
    {
        place.offset = Some((doc, drawing, xf.rect.min + at - home));
    }
    // A drag's offset holds in the document and the kind of place (beside the drawing, or resting)
    // it was made in.
    let offset = place.offset.filter(|(d, beside, _)| *d == doc && *beside == drawing).map_or(Vec2::ZERO, |(_, _, o)| o);
    let pos = match place.pin_at {
        Some(at) => keep_in(xf.rect.min + at, size, bounds),
        None => unpinned(home, offset, end.map(|(at, _)| at), size, bounds),
    };
    place.shown_at = Some(pos - xf.rect.min);
    // Pinned before it ever showed: hold it here.
    if place.pinned && place.pin_at.is_none() {
        place.pin_at = place.shown_at;
    }
    let pinned = place.pinned;
    let mut moved = Vec2::ZERO;
    let mut click: Option<Click> = None;
    egui::Area::new(id).order(egui::Order::Middle).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.tool_active))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(8, 6))
            .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(70) })
            .show(ui, |ui| {
                // In a canvas pane narrower than the toolbar, its controls wrap onto more rows.
                ui.set_max_width((bounds.width() - 18.0).max(HANDLE + SPACING + BUTTON));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(SPACING, SPACING);
                    let (g, grip) = ui.allocate_exact_size(vec2(HANDLE, BUTTON), Sense::drag());
                    let active = grip.hovered() || grip.dragged();
                    ui.painter().rect_filled(g.shrink2(vec2(0.5, 4.0)), CornerRadius::same(2), if active { t.text_dim } else { t.button_border });
                    if grip.dragged() {
                        ui.ctx().data_mut(|d| d.insert_temp(handle_id, true));
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        moved = grip.drag_delta();
                    } else if grip.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    let actions = [
                        (Action::Line, "dc-line", tl!("Straight Line")),
                        (Action::ContinuousArc, "spline", tl!("Continuous Arc")),
                        (Action::SharpArc, "dc-arc", tl!("Sharp Corner Arc")),
                    ];
                    for (action, icon, tip) in actions {
                        if widgets::icon_button(ui, icon, tip, current == action, BUTTON).clicked() {
                            click = Some(Click::Action(action));
                        }
                    }
                    if widgets::icon_button_enabled(ui, "flip-horizontal-2", tl!("Flip Arc"), false, flip, BUTTON).clicked() {
                        click = Some(Click::Run("tool.quickPen.flipArc", json!({})));
                    }
                    let more = widgets::icon_button(ui, "ellipsis", tl!("More Options"), false, BUTTON);
                    egui::Popup::menu(&more).show(|ui| {
                        ui.set_min_width(170.0);
                        let items = [
                            (tl!("Hide Toolbar"), "window.quickPenToolbar", false),
                            (tl!("Pin Toolbar Position"), "window.quickPenToolbar.pin", pinned),
                            (tl!("Reset Toolbar Position"), "window.quickPenToolbar.reset", false),
                        ];
                        for (label, cmd, checked) in items {
                            if widgets::menu_item(ui, label, true, checked) {
                                click = Some(Click::Run(cmd, json!({})));
                            }
                        }
                    });
                });
            });
    });
    if moved != Vec2::ZERO {
        let to = keep_in(pos + moved, size, bounds);
        let place = &mut app.ui.quick_pen_bar_place;
        if place.pinned {
            place.pin_at = Some(to - xf.rect.min);
        } else {
            place.offset = Some((doc, drawing, to - home));
        }
        ui.ctx().request_repaint();
    }
    let r = match click {
        Some(Click::Action(a)) => app.run("tool.setOption", json!({"key": "action", "value": a.name()})).map(|_| ()),
        Some(Click::Run(cmd, p)) => app.run(cmd, p).map(|_| ()),
        None => Ok(()),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The toolbar sits off what was drawn: ahead of a path drawn right and down, behind-left and
    /// above for one drawn left and up, and to the right and below a single point.
    #[test]
    fn the_toolbar_sits_beside_the_end_away_from_the_drawing() {
        let (end, size) = (pos2(500.0, 300.0), vec2(180.0, 40.0));
        let room = egui::Rect::from_min_max(pos2(8.0, 8.0), pos2(808.0, 608.0));
        assert_eq!(beside(end, Some(vec2(1.0, 0.0)), size, room), pos2(524.0, 324.0));
        assert_eq!(beside(end, Some(vec2(-1.0, -1.0)), size, room), pos2(500.0 - 24.0 - 180.0, 300.0 - 24.0 - 40.0));
        assert_eq!(beside(end, Some(vec2(0.0, 1.0)), size, room), pos2(524.0, 324.0));
        assert_eq!(beside(end, Some(vec2(-1.0, 0.5)), size, room), pos2(296.0, 324.0));
        assert_eq!(beside(end, None, size, room), pos2(524.0, 324.0));
        assert_eq!(resting(room, size), pos2(318.0, 568.0));
        // Near the canvas's bottom right corner, the side that runs off it gives way to the other.
        let corner = pos2(700.0, 580.0);
        assert_eq!(beside(corner, Some(vec2(1.0, 1.0)), size, room), pos2(700.0 - 24.0 - 180.0, 580.0 - 24.0 - 40.0));
        // Near the top left corner, a path heading up and left still gets a toolbar off its end.
        let corner = pos2(30.0, 30.0);
        assert_eq!(beside(corner, Some(vec2(-1.0, -1.0)), size, room), pos2(54.0, 54.0));
        // On a canvas too narrow for either side across, the toolbar goes on the side with more
        // room and stays off the end by going below it.
        let narrow = egui::Rect::from_min_max(pos2(8.0, 8.0), pos2(208.0, 608.0));
        let end = pos2(60.0, 300.0);
        let at = keep_in(beside(end, Some(vec2(-1.0, 1.0)), size, narrow), size, narrow);
        assert_eq!(at, pos2(28.0, 324.0));
        assert!(!egui::Rect::from_min_size(at, size).contains(end));
    }

    /// Any drag that began on the canvas hides the toolbar, whether or not the tool has a press
    /// (a pan does not); a drag that began on the toolbar, and no drag, do not.
    #[test]
    fn a_drag_that_began_on_the_canvas_hides_the_toolbar() {
        let canvas = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
        let bar = Some(egui::Rect::from_min_size(pos2(300.0, 500.0), vec2(180.0, 40.0)));
        assert!(canvas_drag(true, Some(pos2(100.0, 100.0)), canvas, bar), "a pan or stroke on the canvas");
        assert!(canvas_drag(true, Some(pos2(100.0, 100.0)), canvas, None), "no toolbar shown yet");
        assert!(!canvas_drag(false, Some(pos2(100.0, 100.0)), canvas, bar), "a click, not a drag");
        assert!(!canvas_drag(true, Some(pos2(310.0, 510.0)), canvas, bar), "the toolbar's own drag");
        assert!(!canvas_drag(true, Some(pos2(900.0, 100.0)), canvas, bar), "a drag in another panel");
        assert!(!canvas_drag(true, None, canvas, bar));
    }

    /// A drag's offset moves the toolbar from its place, unless that would put it over the point
    /// the path goes on from.
    #[test]
    fn a_dragged_toolbar_never_covers_the_end() {
        let size = vec2(180.0, 40.0);
        let room = egui::Rect::from_min_max(pos2(8.0, 8.0), pos2(808.0, 608.0));
        let (end, home) = (pos2(500.0, 300.0), pos2(524.0, 324.0));
        assert_eq!(unpinned(home, vec2(30.0, 40.0), Some(end), size, room), pos2(554.0, 364.0));
        assert_eq!(unpinned(home, vec2(-60.0, -40.0), Some(end), size, room), home, "over the end");
        assert_eq!(unpinned(home, vec2(-60.0, -40.0), None, size, room), pos2(464.0, 284.0));
        assert_eq!(unpinned(home, vec2(900.0, 0.0), Some(end), size, room), pos2(628.0, 324.0), "kept on the canvas");
    }
}
