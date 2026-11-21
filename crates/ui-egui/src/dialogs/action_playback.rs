//! Actions panel menu › Playback Options…: the speed actions play at (`ui.action.playbackOptions`).
//!
//! Fields: `speed` (`accelerated`, `step` or `pause`) and `seconds`, the pause between steps with
//! `pause`. OK sets them; an invalid value is an error, and the dialog stays open.

use craft::behavior::NumberCodec;
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldOk, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::FieldGroup;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::NumberField;
use egui::{Context, Ui};
use serde_json::Value;

use super::actions::{ActionDialog, Form, label, open_or_say};
use crate::VectorcraftApp;
use crate::panels::actions::{PAUSE_SECONDS, PLAYBACK_OPTIONS, PlaybackOptions, PlaybackSpeed};

/// The key the dialog answers for.
const KEY: &str = "actionPlayback";

/// The speeds, with the id their radio button has.
const SPEEDS: [(PlaybackSpeed, &str); 3] =
    [(PlaybackSpeed::Accelerated, "accelerated"), (PlaybackSpeed::Step, "step"), (PlaybackSpeed::Pause, "pause")];

/// OK is always on; the command refuses a pause outside its range.
fn check(_: &PlaybackOptions) -> Option<Label> {
    None
}

/// The Playback Options dialog on the dialog host.
pub struct PlaybackDialog {
    id: DialogId,
    form: Form<PlaybackOptions>,
}

/// Open Playback Options with the speed and pause actions play with now.
pub fn open(app: &mut VectorcraftApp) {
    let dialog = PlaybackDialog { id: dialog_id(KEY), form: Form::new(app.ui.action_playback, check) };
    open_or_say(app, dialog);
}

impl ActionDialog for PlaybackDialog {
    type Answer = PlaybackOptions;

    fn take_ok(&mut self) -> Option<PlaybackOptions> {
        self.form.take()
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.form.answer(answer);
    }

    fn run(app: &mut VectorcraftApp, options: PlaybackOptions) -> Result<(), String> {
        let params = serde_json::to_value(options).map_err(|e| e.to_string())?;
        app.run(PLAYBACK_OPTIONS, params).map(|_| ())
    }
}

impl StackDialog for PlaybackDialog {
    type Result = PlaybackOptions;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PlaybackOptions>> {
        let units = Env::of(ui).units();
        let range = f64::from(*PAUSE_SECONDS.start())..=f64::from(*PAUSE_SECONDS.end());
        self.form.show(ui, label("Playback Options"), None, |ui, options| {
            FieldGroup::new(label("Playback Options")).show(ui, |ui, _| {
                let chosen = SPEEDS.iter().find(|(speed, _)| *speed == options.speed).map_or("accelerated", |(_, id)| *id);
                let group = RadioGroup::new(label("Speed"), Bind::given(chosen.to_owned())).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        Radio::new("accelerated".to_owned()).label(label("Accelerated")).show(ui);
                        Radio::new("step".to_owned()).label(label("Step by Step")).show(ui);
                        Row::new().show(ui, |ui, _| {
                            Radio::new("pause".to_owned()).label(label("Pause For")).show(ui);
                            let seconds = NumberField::new(label("Pause For"), Bind::given(Some(f64::from(options.seconds))))
                                .codec(NumberCodec::new(units.reader(None), 0))
                                .range(range.clone())
                                .id_salt("seconds")
                                .show(ui)
                                .edit
                                .value()
                                .copied()
                                .flatten();
                            if let Some(seconds) = seconds.filter(|seconds| seconds.is_finite()) {
                                options.seconds = seconds.round().clamp(f64::from(*PAUSE_SECONDS.start()), f64::from(*PAUSE_SECONDS.end())) as u32;
                            }
                            Text::new("s").show(ui);
                        });
                    });
                });
                if let Some((speed, _)) = group.output.value().and_then(|id| SPEEDS.iter().find(|(_, listed)| *listed == id.as_str())) {
                    options.speed = *speed;
                }
            });
        })
    }

    fn confirm(&mut self, _: &Context) -> Option<PlaybackOptions> {
        self.form.confirm()
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.form.held_state()
    }

    fn json_fields(&self) -> Option<Value> {
        self.form.json_fields()
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        self.form.set_field(ctx, field, value)
    }
}
