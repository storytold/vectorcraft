//! Sample Color from Screen (`ui.sampleScreenColor`): the platform's screen capture
//! (`craft::platform::screen`, through the framework's [`ScreenPick`]) picks a color anywhere on the
//! screen. The command answers at once; the color arrives in a later frame ([`tick`]) and becomes
//! the fill or the stroke through `paint.setFill` / `paint.setStroke` (one undo step), or the open
//! Color Picker's color. When the person cancels, the colors stay as they were.
//!
//! The capture answers in sRGB. [`sampled`] converts that into the working RGB space, so when the
//! working space is not sRGB, a color inside sRGB looks as it did on the screen.

use std::hash::{Hash, Hasher};

use craft::color_ui::ui::{ScreenColorPicker, ScreenPickGesture, ScreenPickerWords};
use craft::theme::Srgba;
use craft::ui::runtime::env::Env;
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, Cms};

use crate::VectorcraftApp;
use crate::menus::opt_bool;

/// The owner of a pick for the fill.
const FILL: &str = "fill";
/// The owner of a pick for the stroke.
const STROKE: &str = "stroke";

/// The window a pick belongs to: a pick whose window changed is cancelled.
pub(crate) fn target(ctx: &egui::Context) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ctx.viewport_id().hash(&mut hasher);
    hasher.finish()
}

/// Where the document active when the pick started is kept until its color arrives.
fn doc_key() -> egui::Id {
    egui::Id::unique("screen-color-doc")
}

fn remember_doc(app: &VectorcraftApp, ctx: &egui::Context) {
    let uid = app.session.active().map(|d| d.uid);
    ctx.data_mut(|d| d.insert_temp(doc_key(), uid));
}

/// The platform can capture the screen: the shell's capabilities say so and no capture has said it
/// cannot. Without a window there is nothing to capture.
pub fn available(app: &VectorcraftApp) -> bool {
    let Some(ctx) = app.ui_ctx.as_ref() else { return false };
    app.screen_pick.available(Env::of_root(ctx).view().shell().caps)
}

/// `ui.sampleScreenColor {stroke?, picker?}`: start a pick for the fill, the stroke (default: the
/// proxy in front) or, with `picker`, the open Color Picker.
pub fn start(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let stroke = opt_bool(p, "stroke")?.unwrap_or(!app.session.fill_active);
    let picker = if opt_bool(p, "picker")?.unwrap_or(false) {
        Some(crate::dialogs::color_picker::open_picker(app).ok_or("the Color Picker is not open")?)
    } else {
        None
    };
    if app.screen_pick.busy() {
        return Err("the screen color sampler is already open".into());
    }
    if !available(app) {
        return Err("this system has no screen color sampler".into());
    }
    if let Some(ctx) = app.ui_ctx.clone() {
        remember_doc(app, &ctx);
    }
    match picker {
        Some(id) => app.screen_pick.start(crate::dialogs::color_picker::owner(id)),
        None => app.screen_pick.start(if stroke { STROKE } else { FILL }),
    }
    Ok(Value::Null)
}

/// Follows a picker's pipette press for `owner`; the colour of a held drag or of a release inside
/// the app comes back at once.
pub(crate) fn hold(app: &mut VectorcraftApp, ctx: &egui::Context, owner: impl Hash, gesture: ScreenPickGesture) -> Option<Srgba> {
    if !app.screen_pick.busy() {
        remember_doc(app, ctx);
    }
    app.screen_pick.hold(ctx, owner, gesture)
}

/// The color a display sRGB value stands for in `cms`'s working RGB space: it shows as that sRGB
/// value. Components outside 0 to 1 are clamped, and a component that is not a number reads as 0.
pub fn sampled(cms: &Cms, srgb: [f32; 3]) -> Color {
    let [r, g, b] = cms.srgb_to_rgb(srgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }));
    Color::rgb(r, g, b)
}

/// The color `owner` picked, once. One that arrives after another document became active is
/// dropped, and the status line says why.
pub(crate) fn take(app: &mut VectorcraftApp, ctx: &egui::Context, owner: impl Hash) -> Option<Srgba> {
    let color = app.screen_pick.take(owner)?;
    let doc = ctx.data(|d| d.get_temp::<Option<u64>>(doc_key())).flatten();
    if app.session.active().map(|d| d.uid) != doc {
        app.status(tl!("The active document changed while the screen color sampler was open"));
        return None;
    }
    Some(color)
}

/// Each pass after the UI: runs the pick and settles what finished.
pub(crate) fn tick(app: &mut VectorcraftApp, ctx: &egui::Context) {
    app.screen_pick.tick(ctx, target(ctx));
    settle(app, ctx);
}

/// The capture windows of a running pick; the last thing the app's UI shows in a pass.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    ScreenColorPicker::new(&mut app.screen_pick, "VectorCraft").show(ui);
}

/// Applies the fill's and the stroke's color, and reports a failed capture. A Color Picker takes
/// its own in its pass ([`crate::dialogs::color_picker::pump`]).
fn settle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    for (owner, cmd) in [(FILL, "paint.setFill"), (STROKE, "paint.setStroke")] {
        if let Some(c) = take(app, ctx, owner) {
            let color = crate::panels::color_json(&sampled(&cms::active(), [c.r, c.g, c.b]));
            if let Err(e) = app.run(cmd, json!({ "color": color })) {
                app.status(e);
            }
        }
    }
    if let Some(message) = app.screen_pick.take_status(&ScreenPickerWords::default()) {
        app.status(message.text.as_str().to_owned());
    }
}

#[cfg(test)]
mod tests;
