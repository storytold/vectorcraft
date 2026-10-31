//! Sample Color from Screen (`ui.sampleScreenColor`): the system's color sampler picks a color
//! anywhere on the screen ([`crate::Services::screen_color`]: `NSColorSampler` on macOS, the
//! browser's EyeDropper where the browser has one). The command shows the sampler and answers at
//! once. The color arrives in a later frame ([`poll`]) and becomes the fill or the stroke through
//! `paint.setFill` / `paint.setStroke` (one undo step), or the open Color Picker's color. When the
//! person cancels the sampler (Escape), the colors stay as they were.
//!
//! The sampler answers in sRGB (macOS clips a color outside sRGB to it). [`sampled`] converts that
//! into the working RGB space, so when the working space is not sRGB, a color inside sRGB looks as
//! it did on the screen.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, Cms};

use crate::menus::opt_bool;
use crate::{VectorcraftApp, widgets};

/// What the sampler answers: the picked color's display sRGB components (0 to 1), `None` when the
/// person canceled it, or an error.
pub type Answer = Result<Option<[f32; 3]>, String>;

/// Shows the system's color sampler. The answer arrives on the receiver when the person picks a
/// color or cancels, and the host then asks the UI for a frame. An error when the sampler can't be
/// shown.
pub type StartSampler = Box<dyn FnMut() -> Result<Receiver<Answer>, String>>;

/// Where the sampled color goes.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Fill,
    Stroke,
    /// The Color Picker that was open, known by its [`PICKER_ID`].
    Picker(u64),
}

/// The Color Picker's hidden field that holds a number unique to each Color Picker opened
/// ([`picker_id`]). The sampled color goes only to the Color Picker the sampler was shown for.
pub(crate) const PICKER_ID: &str = "__pickerId";

/// A new number for a Color Picker's [`PICKER_ID`].
pub(crate) fn picker_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A sampler that is shown and has not answered yet.
pub struct Waiting {
    answer: Receiver<Answer>,
    target: Target,
    /// The document active when it was shown ([`vectorcraft_engine::DocState::uid`]).
    doc: Option<u64>,
}

/// The host can show a color sampler.
pub fn available(app: &VectorcraftApp) -> bool {
    app.services.screen_color.is_some()
}

/// The open Color Picker dialog, as [`Target::Picker`] names it.
fn open_picker(app: &VectorcraftApp) -> Option<Target> {
    let d = app.ui.dialog.as_ref().filter(|d| crate::dialogs::DialogKind::of(&d.kind) == Some(crate::dialogs::DialogKind::ColorPicker))?;
    d.fields.get(PICKER_ID).and_then(Value::as_u64).map(Target::Picker)
}

/// `ui.sampleScreenColor {stroke?, picker?}`: show the sampler for the fill, the stroke (default:
/// the proxy in front) or, with `picker`, the open Color Picker.
pub fn start(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let stroke = opt_bool(p, "stroke")?.unwrap_or(!app.session.fill_active);
    let target = if opt_bool(p, "picker")?.unwrap_or(false) {
        open_picker(app).ok_or("the Color Picker is not open")?
    } else if stroke {
        Target::Stroke
    } else {
        Target::Fill
    };
    if app.screen_sampler.is_some() {
        return Err("the screen color sampler is already open".into());
    }
    let show = app.services.screen_color.as_mut().ok_or("this system has no screen color sampler")?;
    let answer = show()?;
    let doc = app.session.active().map(|d| d.uid);
    app.screen_sampler = Some(Waiting { answer, target, doc });
    Ok(Value::Null)
}

/// The color a display sRGB value stands for in `cms`'s working RGB space: it shows as that sRGB
/// value. Components outside 0 to 1 are clamped, and a component that is not a number reads as 0.
pub fn sampled(cms: &Cms, srgb: [f32; 3]) -> Color {
    let [r, g, b] = cms.srgb_to_rgb(srgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }));
    Color::rgb(r, g, b)
}

/// The sRGB components of a sampler's hex answer (`#rrggbb` as the browser's EyeDropper gives
/// `sRGBHex`; the Color panel's hex rules apply), or `None` when it is not hex.
pub fn srgb_from_hex(hex: &str) -> Option<[f32; 3]> {
    match crate::panels::color::parse_hex(hex)? {
        Color::Rgb { r, g, b } => Some([r, g, b]),
        _ => None,
    }
}

/// Each frame: take the sampler's answer when it has arrived.
pub(crate) fn poll(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(w) = &app.screen_sampler else { return };
    let answer = match w.answer.try_recv() {
        Ok(answer) => answer,
        Err(TryRecvError::Empty) => {
            // The host wakes the window when the sampler answers; this is in case it can't.
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
            return;
        }
        // The sampler is gone without an answer: as canceled.
        Err(TryRecvError::Disconnected) => Ok(None),
    };
    let Some(Waiting { target, doc, .. }) = app.screen_sampler.take() else { return };
    let srgb = match answer {
        Ok(Some(srgb)) => srgb,
        Ok(None) => return,
        Err(e) => return app.status(e),
    };
    if app.session.active().map(|d| d.uid) != doc {
        return app.status(tl!("The active document changed while the screen color sampler was open"));
    }
    let color = crate::panels::color_json(&sampled(&cms::active(), srgb));
    let cmd = match target {
        Target::Fill => "paint.setFill",
        Target::Stroke => "paint.setStroke",
        // The Color Picker previews it on the selection, as any pick. The color is dropped when the
        // Color Picker the sampler was shown for is no longer open (it was closed meanwhile, or
        // another Color Picker replaced it).
        picker @ Target::Picker(_) => {
            if open_picker(app) == Some(picker)
                && let Some(d) = app.ui.dialog.as_mut()
            {
                d.fields.insert("color".into(), color);
            }
            return;
        }
    };
    if let Err(e) = app.run(cmd, json!({ "color": color })) {
        app.status(e);
    }
}

/// The eyedropper button that starts the sampler (gray without one). Returns true when clicked.
pub(crate) fn button(app: &VectorcraftApp, ui: &mut egui::Ui) -> bool {
    let on = available(app);
    let tip = if on { tl!("Sample Color from Screen") } else { tl!("This system has no screen color sampler") };
    widgets::icon_button_enabled(ui, "pipette", tip, app.screen_sampler.is_some(), on, 24.0).clicked()
}

#[cfg(test)]
mod tests;
