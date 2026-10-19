//! Object › Path › Simplify's slider: a strip under the selection with Auto-Simplify, a slider from
//! the fewest anchor points (left) to the closest fit to the original (right), and More Options,
//! which opens the Simplify dialog at the slider's tolerance. While the strip is open, the canvas
//! previews `object.path.simplify {level}`; Enter or a click outside the strip keeps the result as
//! one undo step, and Escape puts the paths back. The slider opens at level 50, a tolerance of
//! 1 pt (`vectorcraft_pathops::simplify_level_tolerance` maps levels to tolerances).
//!
//! Fields: `level` (0..=100, the command's parameter).

use egui::{Color32, CornerRadius, Order, Pos2, Sense, Stroke, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::{SIMPLIFY_LEVEL, simplify_level_tolerance};
use vectorcraft_geom::Rect;

use super::{DialogResult, DialogSpec, form, modal};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Simplify slider.
pub(crate) const KIND: &str = "simplifySlider";

pub(super) const SPEC: DialogSpec = DialogSpec { window: Some(show), confirm, preview: true, ..DialogSpec::FORM };

/// The command the slider previews, and its undo label.
const CMD: &str = "object.path.simplify";
const LABEL: &str = "Simplify";

/// The fields the slider opens with.
pub(crate) fn fields() -> Value {
    json!({ "level": SIMPLIFY_LEVEL })
}

/// Whether the Simplify slider is open.
pub(crate) fn showing(app: &VectorcraftApp) -> bool {
    app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND)
}

/// The slider's level, a whole number from 0 to 100.
fn level(d: &Dialog) -> f64 {
    let l = d.f64("level", SIMPLIFY_LEVEL);
    if l.is_finite() { l.round().clamp(0.0, 100.0) } else { SIMPLIFY_LEVEL }
}

fn params(d: &Dialog) -> Value {
    json!({ "level": level(d) })
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    form::commit_preview(app, CMD, params(d))
}

/// The open slider, or an error when it isn't open.
fn open_slider(app: &VectorcraftApp) -> Result<Dialog, String> {
    app.ui.dialog.clone().filter(|d| d.kind == KIND).ok_or_else(|| "the Simplify slider is not open".to_string())
}

/// Auto-Simplify (`ui.simplifyAuto`): the open slider moves to the level `object.path.simplify
/// {auto: true}` picks for the selection, and the canvas previews it → {level}.
pub(crate) fn auto(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut d = open_slider(app)?;
    let _ = app.session.begin_interaction(LABEL);
    let r = app.session.preview(CMD, &json!({ "auto": true })).map_err(|e| e.to_string())?;
    let level = r["level"].as_f64().unwrap_or(SIMPLIFY_LEVEL);
    d.fields.insert("level".into(), json!(level));
    d.fields.insert(form::PREVIEWED.into(), params(&d));
    app.ui.dialog = Some(d);
    Ok(json!({ "level": level }))
}

/// More Options (`ui.simplifyMoreOptions`): the Simplify dialog takes the slider's place, at the
/// slider's tolerance → {tolerance}.
pub(crate) fn more_options(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = open_slider(app)?;
    let tolerance = simplify_level_tolerance(level(&d));
    app.ui.dialog = Some(Dialog::new("simplify", super::path_ops::simplify_dialog(tolerance)));
    Ok(json!({ "tolerance": tolerance }))
}

/// The private field that holds the selection's bounds before the preview, `[x0, y0, x1, y1]`.
const BOUNDS: &str = "__bounds";

/// The selection's bounds before the preview: the interaction's snapshot when a preview runs, read
/// once and kept in `d`, so the strip stays where it opened while the preview changes the paths.
fn bounds(app: &mut VectorcraftApp, d: &mut Dialog) -> Option<Rect> {
    if let Some([x0, y0, x1, y1]) = d.fields.get(BOUNDS).and_then(|v| serde_json::from_value::<[f64; 4]>(v.clone()).ok()) {
        return Some(Rect::new(x0, y0, x1, y1));
    }
    let b = match app.session.active().and_then(|st| st.interaction.as_ref()) {
        Some(it) => it.doc.bounds_of(&it.selection.objects, true),
        None => app.selection_bounds(),
    }?;
    d.fields.insert(BOUNDS.into(), json!([b.x0, b.y0, b.x1, b.y1]));
    Some(b)
}

/// Where the strip goes: under the lowest corner on screen of `bounds` (the selection's, before the
/// preview), centered on them, kept on the canvas.
fn place(app: &mut VectorcraftApp, ctx: &egui::Context, id: egui::Id, bounds: Option<Rect>) -> Pos2 {
    let size = egui::AreaState::load(ctx, id).and_then(|s| s.size).unwrap_or(vec2(270.0, 42.0));
    let screen = ctx.content_rect();
    let Some((rect, view)) = app.canvas_rect.zip(app.view().copied()) else { return screen.center() - size / 2.0 };
    let xf = crate::canvas::Xf::new(rect, &view);
    let under = match bounds {
        Some(b) => {
            let lowest = xf.quad(b).iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
            Pos2::new(xf.to_screen(b.center()).x - size.x / 2.0, lowest + 28.0)
        }
        None => rect.center() - size / 2.0,
    };
    let bounds = rect.shrink(8.0);
    under.clamp(bounds.min, (bounds.max - size).max(bounds.min))
}

/// What a button of the strip asked for.
enum Action {
    Auto,
    MoreOptions,
}

fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let id = egui::Id::new(("dialog", KIND));
    let b = bounds(app, &mut d);
    let pos = place(app, ctx, id, b);
    // A click anywhere else keeps the result, and the rest of the app doesn't take it.
    let dim = modal::backdrop();
    let outside =
        egui::Area::new(dim.id).order(dim.order).fixed_pos(Pos2::ZERO).show(ctx, |ui| ui.allocate_rect(ctx.content_rect(), Sense::click())).inner;
    let mut action = None;
    egui::Area::new(id).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.tool_active))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(8, 6))
            .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(70) })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::icon_button(ui, "wand-sparkles", tl!("Auto-Simplify"), false, 28.0).clicked() {
                        action = Some(Action::Auto);
                    }
                    let rail = t.input_border;
                    form::slider_rail_named(ui, &mut d, "level", tl!("Simplify"), 0.0..=100.0, &|_| rail);
                    if widgets::icon_button(ui, "ellipsis", tl!("More Options"), false, 28.0).clicked() {
                        action = Some(Action::MoreOptions);
                    }
                });
            });
    });
    let p = params(&d);
    let _ = form::preview_now(app, &mut d, LABEL, CMD, p);
    app.ui.dialog = Some(d);
    let r = match action {
        Some(Action::Auto) => auto(app),
        Some(Action::MoreOptions) => more_options(app),
        None if outside.clicked() || ctx.input(|i| i.key_pressed(egui::Key::Enter)) => super::confirm(app),
        None => Ok(Value::Null),
    };
    if let Err(e) = r {
        app.status(e);
    }
}
