//! Actions panel menu › Playback Options…: the speed actions play at (`ui.action.playbackOptions`).
//!
//! Fields: `speed` (`accelerated`, `step` or `pause`) and `seconds`, the pause between steps with
//! `pause`. OK sets them; an invalid value is an error, and the dialog stays open.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Playback Options.
pub const KIND: &str = "actionPlayback";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Playback Options").into(), body, confirm, min_width: 260.0, ..DialogSpec::FORM };

/// Open Playback Options with the speed and pause actions play with now.
pub fn open(app: &mut VectorcraftApp) {
    let o = app.ui.action_playback;
    app.ui.dialog = Some(Dialog::new(KIND, json!({"speed": o.speed, "seconds": o.seconds})));
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let speed = d.str("speed");
    for (value, label) in [("accelerated", tl!("Accelerated")), ("step", tl!("Step by Step"))] {
        if widgets::radio(ui, label, speed == value, true) {
            d.fields.insert("speed".into(), json!(value));
        }
    }
    ui.horizontal(|ui| {
        let pause = tl!("Pause For");
        if widgets::radio(ui, pause, speed == "pause", true) {
            d.fields.insert("speed".into(), json!("pause"));
        }
        let range = f64::from(*crate::panels::actions::PAUSE_SECONDS.start())..=f64::from(*crate::panels::actions::PAUSE_SECONDS.end());
        if let Some(n) = widgets::range_field_named(ui, "action-pause-seconds", Some(pause), d.f64("seconds", *range.start()), range, "s", 0, 50.0) {
            d.fields.insert("seconds".into(), json!(n as u64));
        }
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = json!({"speed": d.str("speed"), "seconds": d.fields.get("seconds").cloned().unwrap_or(Value::Null)});
    let r = app.run(crate::panels::actions::PLAYBACK_OPTIONS, params)?;
    app.ui.dialog = None;
    Ok(r)
}
