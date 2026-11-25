//! Edit → Edit Colors → Adjust Color Balance: the colour mode (Gray, RGB, CMYK or Global), a slider
//! per channel (−100..100 %; Global has one, Tint, for the tints of global and spot colors),
//! Convert, Fill and Stroke, previewed live on the canvas; OK keeps the preview as one undo step
//! (`edit.colors.adjustBalance`).
//!
//! The dialog lives on the dialog host as a `HostedParamDialog`; [`Adjust`] is the one mechanism it
//! shares with Saturate: it opens the dialog, runs the live preview from the dialog's values each
//! frame (`pump`) and commits the preview at OK.
//!
//! Fields: `mode` (`gray`, `rgb`, `cmyk` or `global`), the channels `r`, `g`, `b`, `c`, `m`, `y`,
//! `k`, `gray`, `tint`, then `convert`, `fill` and `stroke`. The preview check box is the host's.

use craft::color_ui::ui::{ChannelRow, ChannelUnit};
use craft::theme::Srgba;
use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::color_picker::ColorDialogs;
use super::host;
use super::preview::LivePreview;
use crate::VectorcraftApp;
use crate::panels::c32;

/// The dialog kind of Adjust Color Balance.
pub const KIND: &str = "colorBalance";

const CMD: &str = "edit.colors.adjustBalance";

/// A colour mode: its `mode` id, label and channels as (field, label).
struct Mode {
    id: &'static str,
    label: &'static str,
    channels: &'static [(&'static str, &'static str)],
}

const MODES: [Mode; 4] = [
    Mode { id: "gray", label: "Grayscale", channels: &[("gray", "Black")] },
    Mode { id: "rgb", label: "RGB", channels: &[("r", "Red"), ("g", "Green"), ("b", "Blue")] },
    Mode { id: "cmyk", label: "CMYK", channels: &[("c", "Cyan"), ("m", "Magenta"), ("y", "Yellow"), ("k", "Black")] },
    Mode { id: "global", label: "Global", channels: &[("tint", "Tint")] },
];

/// Global mode: it shifts the tints of linked colours, which keep their model (no Convert).
const GLOBAL: &str = "global";

/// The record of an open Edit Colors dialog: its id on the host and the live preview it shows on the canvas.
pub(crate) struct Open {
    /// The dialog's id on the host.
    pub id: DialogId,
    /// The preview the dialog's values run as one interaction.
    pub preview: LivePreview,
}

/// What an Edit Colors dialog is: how it opens, previews and commits. Balance and Saturate differ only in these values.
#[derive(Clone, Copy)]
pub(super) struct Adjust {
    /// The dialog's key on the host.
    pub(super) kind: &'static str,
    /// The command the dialog previews and commits.
    pub(super) cmd: &'static str,
    /// The title and the name of the preview's undo step.
    pub(super) label: &'static str,
    /// Where the open dialog's record is kept.
    pub(super) slot: fn(&mut ColorDialogs) -> &mut Option<Open>,
    /// The values the dialog opens with.
    pub(super) initial: fn() -> DialogFields,
    /// The dialog's rows.
    pub(super) rows: fn() -> Vec<Param>,
    /// The command's parameters for the dialog's values.
    pub(super) params: fn(&DialogFields) -> Value,
}

impl Adjust {
    /// Opens the dialog with its initial values; one still open is replaced and its preview rolled back.
    pub(super) fn open(self, app: &mut VectorcraftApp) {
        if let Err(reason) = self.try_open(app) {
            app.status(reason);
        }
    }

    fn try_open(self, app: &mut VectorcraftApp) -> Result<(), String> {
        const NO_WINDOW: &str = "no window to open the dialog in";
        let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
        if let Some(old) = (self.slot)(&mut app.color_dialogs).take() {
            old.preview.cancel(app);
        }
        let dialog = HostedParamDialog::new(&ctx, self.kind, (self.initial)(), move |fields| {
            ParamDialog::new(self.kind, self.label, (self.rows)()).values(Bind::bound(fields)).previews(true)
        });
        let id = host::open_held(app, dialog, move |app, answer| self.commit(app, answer)).ok_or(NO_WINDOW)?;
        *(self.slot)(&mut app.color_dialogs) = Some(Open { id, preview: LivePreview::default() });
        Ok(())
    }

    /// OK: keeps the preview as one undo step; an `Err` keeps the dialog open with the reason.
    fn commit(self, app: &mut VectorcraftApp, answer: &ParamResult) -> Result<(), String> {
        let live = (self.slot)(&mut app.color_dialogs).as_ref().map(|open| open.preview.clone()).unwrap_or_default();
        // PLAN: assumed `LivePreview::commit(self, ..)` as the contract row says; the clone keeps the record when it fails.
        live.commit(app, self.cmd, (self.params)(&answer.fields))?;
        *(self.slot)(&mut app.color_dialogs) = None;
        Ok(())
    }

    /// Once per frame: previews the dialog's values while its Preview box is on, and rolls the preview back once the
    /// dialog is gone without OK.
    pub(super) fn pump(self, app: &mut VectorcraftApp) {
        let Some(Open { id, mut preview }) = (self.slot)(&mut app.color_dialogs).take() else { return };
        let shown = app.dialogs().and_then(|dialogs| {
            dialogs.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(|dialog| (dialog.is_previewing(), (self.params)(dialog.fields()))))
        });
        match shown {
            Some((on, params)) => {
                // `run` reports a failed preview to the status line itself.
                let _ = preview.run(app, on, self.label, self.cmd, &params);
                *(self.slot)(&mut app.color_dialogs) = Some(Open { id, preview });
            }
            None => preview.cancel(app),
        }
    }
}

/// A channel row over the percent value `key` of `fields` (−100..100 %), its rail the colours `track` gives from the
/// rail's start to its end.
pub(super) fn percent_row(ui: &mut egui::Ui, fields: &mut DialogFields, key: &str, label: &'static str, track: impl Fn(f32) -> Srgba) {
    const RAIL_STOPS: usize = 5;
    let rail = (0..RAIL_STOPS).map(|i| track(i as f32 / (RAIL_STOPS - 1) as f32)).collect();
    let rule = crate::panels::whole_steps(-100.0..=100.0);
    let value = fields.number(key).unwrap_or(0.0);
    let edit = ChannelRow::new(label, Bind::given(value), rule).unit(ChannelUnit::Percent).track(rail).show(ui);
    if let Some(value) = edit.as_ref().and_then(|edit| edit.value()) {
        fields.set(key, *value);
    }
}

const ADJUST: Adjust = Adjust { kind: KIND, cmd: CMD, label: "Adjust Colors", slot, initial, rows, params };

fn slot(dialogs: &mut ColorDialogs) -> &mut Option<Open> {
    &mut dialogs.balance
}

/// Open the dialog with every channel at 0, adjusting fills and strokes.
pub fn open(app: &mut VectorcraftApp) {
    ADJUST.open(app);
}

/// Previews the open dialog; one line of `dialogs::host::pump`.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    ADJUST.pump(app);
}

/// Every channel field of every mode.
fn channel_keys() -> impl Iterator<Item = &'static str> {
    MODES.iter().flat_map(|m| m.channels.iter()).map(|(key, _)| *key)
}

fn initial() -> DialogFields {
    let mut fields = DialogFields::new();
    for key in channel_keys() {
        fields.set(key, 0.0);
    }
    fields
}

fn mode(fields: &DialogFields) -> &'static Mode {
    let id = fields.text("mode").unwrap_or_default();
    MODES.iter().find(|m| m.id == id).unwrap_or(&MODES[1])
}

/// `edit.colors.adjustBalance` parameters: the mode, its channels and the options.
fn params(fields: &DialogFields) -> Value {
    let m = mode(fields);
    let flag = |key: &str| fields.bool(key).unwrap_or_default();
    let mut p = json!({"mode": m.id, "convert": flag("convert"), "fill": flag("fill"), "stroke": flag("stroke")});
    for (k, _) in m.channels {
        p[*k] = json!(fields.number(k).unwrap_or(0.0));
    }
    p
}

/// The rail of channel `key`: the channel from −100 % to +100 % over a mid colour.
fn track(key: &str, x: f32) -> Srgba {
    let v = x.clamp(0.0, 1.0);
    let c = match key {
        "r" => Color::rgb(v, 0.5, 0.5),
        "g" => Color::rgb(0.5, v, 0.5),
        "b" => Color::rgb(0.5, 0.5, v),
        "c" => Color::cmyk(v, 0.0, 0.0, 0.0),
        "m" => Color::cmyk(0.0, v, 0.0, 0.0),
        "y" => Color::cmyk(0.0, 0.0, v, 0.0),
        "k" => Color::cmyk(0.0, 0.0, 0.0, v),
        _ => Color::gray(v),
    };
    Srgba::from(c32(&c))
}

/// The channel rows of the current mode.
fn channels(ui: &mut egui::Ui, fields: &mut DialogFields, _cx: &mut CustomContext) -> egui::Response {
    super::form::stacked(ui, fields, |ui, fields| {
        for (key, label) in mode(fields).channels {
            percent_row(ui, fields, key, label, |x| track(key, x));
        }
    })
}

fn rows() -> Vec<Param> {
    let modes = MODES.iter().map(|m| (m.id.to_owned(), Label::from(m.label))).collect();
    let with_model = MODES.iter().map(|m| m.id).filter(|id| *id != GLOBAL);
    vec![
        Param::new("mode", "Color Mode", ParamKind::choice(modes, Some("rgb".to_owned()))),
        Param::new("convert", "Convert", ParamKind::bool(false)).enabled_when_in("mode", with_model),
        Param::read_only_text("note", "", "Shifts the tints of global and spot colors; other colors stay.").visible_when("mode", GLOBAL),
        Param::custom("channels", "Channels", channels).companions(channel_keys()),
        Param::section("Adjust Options"),
        Param::new("fill", "Fill", ParamKind::bool(true)),
        Param::new("stroke", "Stroke", ParamKind::bool(true)),
    ]
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::*;

    fn fill_hex(app: &VectorcraftApp, id: u64) -> String {
        let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        n.appearance.fill_paint().color().unwrap().to_hex()
    }

    /// An app with the window context that hosts the dialog.
    fn app() -> (VectorcraftApp, egui::Context) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        (app, ctx)
    }

    fn set(app: &VectorcraftApp, ctx: &egui::Context, field: &str, value: Value) {
        app.dialogs().unwrap().with(|host| host.set_json_field(ctx, dialog_id(KIND), field, value)).unwrap();
    }

    /// OK as the button files it, then the frame that drains it.
    fn confirm(app: &mut VectorcraftApp, ctx: &egui::Context) {
        app.dialogs().unwrap().with(|host| host.confirm_by_id(ctx, dialog_id(KIND)));
        host::pump(app, ctx);
    }

    #[test]
    fn previews_live_and_commits_one_undo_step() {
        let (mut app, ctx) = app();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"color": "#808080"})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        open(&mut app);
        assert!(app.dialogs().unwrap().with(|host| host.is_open::<HostedParamDialog>()));
        set(&app, &ctx, "r", json!(20));
        host::pump(&mut app, &ctx);
        assert!(app.session.in_interaction(), "the preview runs");
        assert_eq!(fill_hex(&app, id), "#b38080", "previewed on the canvas");
        // A field set from outside previews on the next frame.
        set(&app, &ctx, "r", json!(40));
        host::pump(&mut app, &ctx);
        assert_eq!(fill_hex(&app, id), "#e68080");
        // Cancel rolls the preview back.
        app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, dialog_id(KIND)));
        host::pump(&mut app, &ctx);
        assert_eq!(fill_hex(&app, id), "#808080");
        // OK keeps it as one undo step.
        open(&mut app);
        set(&app, &ctx, "mode", json!("cmyk"));
        set(&app, &ctx, "k", json!(-50));
        host::pump(&mut app, &ctx);
        confirm(&mut app, &ctx);
        assert!(!app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        let lighter = vectorcraft_color::Color::from_hex(&fill_hex(&app, id)).unwrap().to_rgb()[0];
        assert!(lighter > 0.6, "{lighter}");
    }

    #[test]
    fn global_mode_shifts_the_tints_of_global_colours() {
        let (mut app, ctx) = app();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"swatch": "Ink", "tint": 50})).unwrap();
        open(&mut app);
        set(&app, &ctx, "mode", json!("global"));
        set(&app, &ctx, "tint", json!(-20));
        host::pump(&mut app, &ctx);
        let tint =
            app.dialogs().unwrap().with(|host| host.get_by_id::<HostedParamDialog>(dialog_id(KIND)).map(|d| params(d.fields())["tint"].clone()));
        assert_eq!(tint, Some(json!(-20.0)));
        confirm(&mut app, &ctx);
        let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        let vectorcraft_color::Paint::Solid { swatch, tint, .. } = n.appearance.fill_paint() else { panic!() };
        assert_eq!(swatch.as_deref(), Some("Ink"), "the colour stays linked");
        assert!((tint - 0.3).abs() < 1e-6, "{tint}");
    }
}
