//! Object › Path › Simplify's slider: a strip under the selection with Auto-Simplify, a slider from
//! the fewest anchor points (left) to the closest fit to the original (right), and More Options,
//! which opens the Simplify dialog at the slider's tolerance. While the strip is open, the canvas
//! previews `object.path.simplify {level}`; Enter or a click outside the strip keeps the result as
//! one undo step, and Escape puts the paths back. The slider opens at Auto-Simplify's level for
//! the selection (`vectorcraft_engine::auto_simplify_level`), so the canvas first shows the
//! auto-simplified paths, as the Simplify help page describes
//! (https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/manually-simplify-paths.html).
//! `vectorcraft_pathops::simplify_level_tolerance` maps levels to tolerances.
//!
//! The strip itself ([`Strip`]) is shared with Object › Path › Smooth's slider (`smooth_slider`).
//!
//! Fields: `level` (0..=100, the command's parameter).

use egui::{Color32, CornerRadius, Order, Pos2, Sense, Stroke, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::{SIMPLIFY_LEVEL, auto_simplify_level, simplify_level_tolerance};
use vectorcraft_geom::Rect;

use super::{DialogResult, DialogSpec, form, modal};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Simplify slider.
pub(crate) const KIND: &str = "simplifySlider";

pub(super) const SPEC: DialogSpec = DialogSpec { window: Some(show), confirm, preview: true, ..DialogSpec::FORM };

/// What the Simplify slider previews, and its buttons.
const STRIP: Strip = Strip {
    kind: KIND,
    cmd: "object.path.simplify",
    label: "Simplify",
    default_level: SIMPLIFY_LEVEL,
    auto: ("Auto-Simplify", auto),
    more_options: Some(more_options),
};

/// A UI command a button of a strip runs.
type Button = fn(&mut VectorcraftApp) -> Result<Value, String>;

/// A slider strip under the selection that previews `cmd {level}` while it is open: its first
/// button picks the level `cmd {auto: true}` does, and an optional last one, More Options, opens a
/// dialog in its place. Enter or a click outside the strip keeps the result as one undo step, and
/// Escape puts the paths back.
pub(super) struct Strip {
    /// The dialog kind.
    pub kind: &'static str,
    /// The command previewed.
    pub cmd: &'static str,
    /// The undo step's name, and the slider's.
    pub label: &'static str,
    /// The level when the field holds no number.
    pub default_level: f64,
    /// The auto button's tip, and what it runs.
    pub auto: (&'static str, Button),
    /// What More Options runs, when the strip has it.
    pub more_options: Option<Button>,
}

impl Strip {
    /// Whether this strip is open.
    pub fn showing(&self, app: &VectorcraftApp) -> bool {
        app.ui.dialog.as_ref().is_some_and(|d| d.kind == self.kind)
    }

    /// The open strip, if it is this one.
    pub fn open(&self, app: &VectorcraftApp) -> Option<Dialog> {
        app.ui.dialog.clone().filter(|d| d.kind == self.kind)
    }

    /// The slider's level, a whole number from 0 to 100.
    fn level(&self, d: &Dialog) -> f64 {
        let l = d.f64("level", self.default_level);
        if l.is_finite() { l.round().clamp(0.0, 100.0) } else { self.default_level }
    }

    fn params(&self, d: &Dialog) -> Value {
        json!({ "level": self.level(d) })
    }

    pub fn confirm(&self, app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
        form::commit_preview(app, self.cmd, self.params(d))
    }

    /// The open strip `d` moves to the level `cmd {auto: true}` picks for the selection, and the
    /// canvas previews it → {level}.
    pub fn auto(&self, app: &mut VectorcraftApp, mut d: Dialog) -> Result<Value, String> {
        let _ = app.session.begin_interaction(self.label);
        let r = app.session.preview(self.cmd, &json!({ "auto": true })).map_err(|e| e.to_string())?;
        let level = r["level"].as_f64().unwrap_or(self.default_level);
        d.fields.insert("level".into(), json!(level));
        d.fields.insert(form::PREVIEWED.into(), self.params(&d));
        app.ui.dialog = Some(d);
        Ok(json!({ "level": level }))
    }

    pub fn show(&self, app: &mut VectorcraftApp, ctx: &egui::Context) {
        let Some(mut d) = app.ui.dialog.clone() else { return };
        let t = Tokens::get(ctx);
        let id = egui::Id::unique(("dialog", self.kind));
        let b = bounds(app, &mut d);
        let pos = place(app, ctx, id, b);
        // A click anywhere else keeps the result, and the rest of the app doesn't take it.
        let dim = modal::backdrop();
        let outside =
            egui::Area::new(dim.id).order(dim.order).fixed_pos(Pos2::ZERO).show(ctx, |ui| ui.allocate_rect(ctx.content_rect(), Sense::click())).inner;
        let mut action: Option<Button> = None;
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
                        if widgets::icon_button(ui, "wand-sparkles", tl!(self.auto.0), false, 28.0).clicked() {
                            action = Some(self.auto.1);
                        }
                        let rail = t.input_border;
                        form::slider_rail_named(ui, &mut d, "level", tl!(self.label), 0.0..=100.0, &|_| rail);
                        if let Some(more) = self.more_options
                            && widgets::icon_button(ui, "ellipsis", tl!("More Options"), false, 28.0).clicked()
                        {
                            action = Some(more);
                        }
                    });
                });
        });
        let p = self.params(&d);
        let _ = form::preview_now(app, &mut d, self.label, self.cmd, p);
        app.ui.dialog = Some(d);
        let r = match action {
            Some(run) => run(app),
            None if outside.clicked() || ctx.input(|i| i.key_pressed(egui::Key::Enter)) => super::confirm(app),
            None => Ok(Value::Null),
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
}

/// The fields the slider opens with: Auto-Simplify's level for the selection, or
/// [`SIMPLIFY_LEVEL`] without a document.
pub(crate) fn fields(app: &VectorcraftApp) -> Value {
    json!({ "level": auto_simplify_level(&app.session).unwrap_or(SIMPLIFY_LEVEL) })
}

/// Whether the Simplify slider is open.
pub(crate) fn showing(app: &VectorcraftApp) -> bool {
    STRIP.showing(app)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    STRIP.confirm(app, d)
}

fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    STRIP.show(app, ctx);
}

/// Auto-Simplify (`ui.simplifyAuto`): the open slider moves to the level `object.path.simplify
/// {auto: true}` picks for the selection, and the canvas previews it → {level}.
pub(crate) fn auto(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = STRIP.open(app).ok_or_else(|| "the Simplify slider is not open".to_string())?;
    STRIP.auto(app, d)
}

/// More Options (`ui.simplifyMoreOptions`): the Simplify dialog takes the slider's place, at the
/// slider's tolerance → {tolerance}.
pub(crate) fn more_options(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = STRIP.open(app).ok_or_else(|| "the Simplify slider is not open".to_string())?;
    let tolerance = simplify_level_tolerance(STRIP.level(&d));
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
