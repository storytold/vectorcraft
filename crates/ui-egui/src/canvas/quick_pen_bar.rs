//! The Quick Pen toolbar: a floating bar on the canvas while the Quick Pen tool is active. It holds
//! the tool's actions (Straight Line, Continuous Arc, Sharp Corner Arc), Flip Arc for the last
//! sharp-corner arc, and the task bar's More menu (hide, pin, reset the position).
//!
//! Public behavior: selecting the tool shows the toolbar, its actions decide what a click or drag
//! draws, Flip Arc flips the sharp-corner arc just drawn, the More menu hides the toolbar and resets
//! its position, and the toolbar moves by dragging it
//! ([Draw with the Quick Pen tool](https://helpx.adobe.com/illustrator/web/add-and-edit-artwork/draw-and-edit-paths/draw-with-quick-pen-tool.html)).
//! The toolbar follows the drawing, beside the point the path goes on from and away from what was
//! drawn ([Behind the design: the Quick Pen tool](https://adobe.design/ideas/behind-the-design-adobe-illustrator-s-quick-pen-tool)).
//! VectorCraft's own rules: moving to the drawn side where the other side runs off the canvas, the
//! place with no path being drawn, and Window › Quick Pen Toolbar to show it again.

use craft::behavior::IconId;
use craft::behavior::overlay::{Align, Side};
use craft::ui::buttons::IconButton;
use craft::ui::canvas::CanvasSpace;
use craft::ui::canvas_controls::TaskBar;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::{Direction, Env};
use vectorcraft_tools::quickpen::{self, Stroke as Action};

use super::{Ui, VectorcraftApp, json};

/// The Quick Pen toolbar, while the Quick Pen tool is active, Window › Quick Pen Toolbar is on and
/// no gesture is under way on the canvas. Selecting the tool turns Window › Quick Pen Toolbar on
/// ([`VectorcraftApp::try_select_tool`]). Returns the toolbar's rect when it showed.
pub(super) fn quick_pen_bar(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) -> Option<egui::Rect> {
    if !app.ui.quick_pen_bar || app.session.tool_id() != quickpen::ID {
        return None;
    }
    // PLAN: assumed `super::tool_adapter::in_gesture(app: &VectorcraftApp, ctx: &egui::Context) -> bool` (cv.S3)
    if super::tool_adapter::in_gesture(app, ui.ctx()) {
        return None;
    }
    // A middle-button drag pans the canvas, which hides the toolbar as a tool's drag does.
    if ui.input(|i| i.pointer.middle_down() && i.pointer.press_origin().is_some_and(|p| space.view.rect.contains(p))) {
        return None;
    }
    let uid = app.session.active().map(|st| st.uid)?;
    let view = app.view_info();
    let end = app.session.tool_drawing_end(view);
    let flip = app.session.tool_claims_key(vectorcraft_tools::ToolKey::Letter('c'), view);
    let current = app.session.tool_options().get("action").and_then(serde_json::Value::as_str).and_then(Action::parse).unwrap_or_default();
    let key = (uid, end.is_some());
    let mut local = app.ui.quick_pen_bar_place.for_key(key);
    let before = local;
    let env = Env::of(ui);
    let rtl = env.view().direction() == Direction::Rtl;
    // Beside the end, on the side the path arrives from (a path of one point: right and below), a gap off it
    // both ways, as production's corner did; the bar's sides are reading-order ones, so a rightward arrival is
    // `End` under LTR and `Start` under RTL. The anchor moves the gap down (or up), the side adds it across.
    // The framework flips the side across; up or down is chosen here, as the framework's flip would line the
    // bar up with the gapped point instead of a gap off the other side of the end.
    let height_id = egui::Id::unique("quick-pen-bar-height");
    let (anchor, side, align) = match end {
        Some((p, dir)) => {
            let at = space.view.to_screen(p);
            let heading = dir.map(|d| space.view.to_screen(p + d) - at);
            let right = heading.is_none_or(|d| d.x >= 0.0);
            let mut down = heading.is_none_or(|d| d.y >= 0.0);
            let gap = env.tokens.space.gap.section;
            // With the bar's height from the last pass, that side gives way where it runs off the canvas and the
            // other fits or runs off by less, as in production.
            if let Some(height) = ui.ctx().data(|d| d.get_temp::<f32>(height_id)) {
                let area = space.view.rect;
                let over = |down: bool| {
                    let top = if down { at.y + gap } else { at.y - gap - height };
                    (area.top() - top).max(0.0) + (top + height - area.bottom()).max(0.0)
                };
                if over(!down) < over(down) {
                    down = !down;
                }
            }
            let off = space.view.to_doc(at + egui::vec2(0.0, if down { gap } else { -gap }));
            (vec![off], if right != rtl { Side::End } else { Side::Start }, if down { Align::Start } else { Align::End })
        }
        None => (vec![space.view.to_doc(space.view.rect.center_bottom())], Side::Above, Align::Center),
    };
    let mut bar = TaskBar::new(tl!("Quick Pen"), &anchor, Bind::bound(&mut local)).side(side, align);
    if let Some((p, _)) = end {
        bar = bar.keep_clear(p);
    }
    let entries = [
        Entry::new(Action::Line, tl!("Straight Line")).icon(IconSpec::svg(IconId::new("dc-line"))),
        Entry::new(Action::ContinuousArc, tl!("Continuous Arc")).icon(IconSpec::svg(IconId::new("spline"))),
        Entry::new(Action::SharpArc, tl!("Sharp Corner Arc")).icon(IconSpec::svg(IconId::new("dc-arc"))),
    ];
    let mut run: Option<(&'static str, serde_json::Value)> = None;
    let shown = bar.show(ui, space.view.rect, |ui, _| {
        if let Some(action) = Segmented::new(tl!("Action"), Bind::given(current), &entries).show(ui).value() {
            run = Some(("tool.setOption", json!({"key": "action", "value": action.name()})));
        }
        let flip_arc = IconButton::new(IconSpec::svg(IconId::new("flip-horizontal-2"))).label(tl!("Flip Arc")).enabled(flip).show(ui);
        if flip_arc.clicked() {
            run = Some(("tool.quickPen.flipArc", json!({})));
        }
    });
    if shown.output.rect.is_positive() {
        ui.ctx().data_mut(|d| d.insert_temp(height_id, shown.output.rect.height()));
    }
    if shown.output.hide {
        run = Some(("window.quickPenToolbar", json!({})));
    }
    if local != before {
        app.ui.quick_pen_bar_place.set(key, local);
    }
    if let Some((cmd, params)) = run
        && let Err(e) = app.run(cmd, params)
    {
        app.ui.status = e;
    }
    shown.output.rect.is_positive().then_some(shown.output.rect)
}
