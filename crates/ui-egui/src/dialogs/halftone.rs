//! Object → Vector Halftone: the dot shape, the screen frequency and angle, mono (in one colour)
//! or CMYK screens, Invert, Clip to Art and Keep Original, previewed live on the canvas; OK keeps
//! the preview as one undo step (`object.vectorHalftone`).
//!
//! Fields: `shape`, `frequency`, `angle`, `mode`, `color`, `invert`, `clip`, `keepOriginal` (the
//! command's parameters). The live effect's dialog shows the same rows but Keep Original
//! ([`screen_params`]).

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::value_drag::SliderScale;
use serde_json::{Value, json};

use super::form::{fields_of, json_of};
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;

/// The command Vector Halftone runs, and the key of its dialog.
pub(crate) const CMD: &str = "object.vectorHalftone";

/// The open Object › Vector Halftone dialog and its live preview.
pub(crate) struct Open {
    /// The dialog's id on the host.
    pub id: DialogId,
    /// The drawing whose art the dialog previews and changes.
    document: u64,
    /// The temporary dots the dialog shows while it previews.
    pub preview: LivePreview,
}

/// The dialog's fields as it opens: the command's defaults.
pub fn fields() -> Value {
    json!({"shape": "circle", "frequency": 20, "angle": 45, "mode": "mono", "color": "#000000", "invert": false, "clip": true, "keepOriginal": false})
}

/// A choice's options from `(id, label)` pairs, the labels translated.
pub(super) fn options<I: AsRef<str>>(list: &[(I, &str)]) -> Vec<(String, Label)> {
    list.iter().map(|(id, label)| (id.as_ref().to_string(), Label::from(tl!(label).to_owned()))).collect()
}

/// The rows Object › Vector Halftone and the live effect (Effect › Color Effects › Vector
/// Halftone…, `live`) share: the dot shape, the screen frequency and angle, mono or CMYK screens,
/// the dot colour (or, with the effect's `fromArt`, the art's), Invert and Clip to Art. Object ›
/// Vector Halftone adds Keep Original. The Color row depends on two values, so the rows are built
/// from `fields`.
pub(crate) fn screen_params(live: bool, fields: &DialogFields) -> Vec<Param> {
    let shapes = [("circle", "Circle"), ("ellipse", "Ellipse"), ("square", "Square"), ("diamond", "Diamond"), ("line", "Line")];
    let mut rows = vec![
        Param::new("shape", tl!("Shape"), ParamKind::choice(options(&shapes), Some("circle".into()))),
        Param::new("frequency", tl!("Frequency"), ParamKind::range(1.0, 150.0, 20.0, SliderScale::Linear)).unit(UnitKind::LinesPerInch),
        Param::new("angle", tl!("Angle"), ParamKind::range(-90.0, 90.0, 45.0, SliderScale::Linear)).unit(UnitKind::Degrees),
        Param::new("mode", tl!("Screens"), ParamKind::choice(options(&[("mono", "Mono"), ("cmyk", "CMYK")]), Some("mono".into()))),
    ];
    if live {
        rows.push(Param::new("fromArt", tl!("Dot Color from Art"), ParamKind::bool(false)).visible_when("mode", "mono"));
    }
    let from_art = live && fields.bool("fromArt").unwrap_or(false);
    if fields.text("mode") != Some("cmyk") && !from_art {
        rows.push(Param::new("color", tl!("Color"), ParamKind::color(Some([0.0; 3]))));
    }
    rows.push(Param::new("invert", tl!("Invert"), ParamKind::bool(false)));
    rows.push(Param::new("clip", tl!("Clip to Art"), ParamKind::bool(true)));
    if !live {
        rows.push(Param::new("keepOriginal", tl!("Keep Original"), ParamKind::bool(false)));
    }
    rows
}

/// Opens Object › Vector Halftone, previewing from its first frame.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let document = app.session.active().ok_or("no document")?.uid;
    let defaults = fields();
    let rows = screen_params(false, &DialogFields::new());
    let initial = fields_of(&rows, defaults.as_object().unwrap_or(&Default::default()));
    let dialog = HostedParamDialog::command(&ctx, CMD, initial, |fields| {
        let rows = screen_params(false, fields);
        ParamDialog::command(CMD, tl!("Vector Halftone"), rows).previews(true).values(Bind::bound(fields))
    });
    let ok = |app: &mut VectorcraftApp, result: &ParamResult| {
        let open = app.effect_dialogs.halftone.as_ref().ok_or("the dialog is closed")?;
        if !open.in_source(app) {
            return Err(tl!("Return to the original drawing to apply Vector Halftone").into());
        }
        let params = Value::Object(json_of(&screen_params(false, &result.fields), &result.fields));
        let mut open = app.effect_dialogs.halftone.take().ok_or("the dialog is closed")?;
        let done = std::mem::take(&mut open.preview).commit(app, CMD, params);
        if done.is_err() {
            app.effect_dialogs.halftone = Some(open);
        }
        done.map(|_| ())
    };
    let cancel: Box<dyn FnOnce(&mut VectorcraftApp)> = Box::new(|app| {
        if let Some(open) = app.effect_dialogs.halftone.take()
            && open.in_source(app)
            && owns_preview(app)
        {
            open.preview.cancel(app);
        }
    });
    let id = open_held_with(app, dialog, ok, HeldOptions { cancel: Some(cancel), ..Default::default() }).ok_or("no window")?;
    app.effect_dialogs.halftone = Some(Open { id, document, preview: LivePreview::default() });
    Ok(Value::Null)
}

/// Runs or ends the open dialog's preview for this frame.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.effect_dialogs.halftone.take() else { return };
    if !open.in_source(app) {
        open.preview = LivePreview::default();
        app.effect_dialogs.halftone = Some(open);
        return;
    }
    let shown = app.dialogs().and_then(|host| {
        host.with(|h| {
            h.get_by_id::<HostedParamDialog>(open.id).map(|d| {
                let params = json_of(&screen_params(false, d.fields()), d.fields());
                (d.is_previewing(), Value::Object(params))
            })
        })
    });
    if let Some((on, params)) = shown {
        // `run` reports a failed preview on the status line itself.
        let _ = open.preview.run(app, on, "Vector Halftone", CMD, &params);
    }
    app.effect_dialogs.halftone = Some(open);
}

/// Whether the open Object → Vector Halftone dialog is showing its temporary dot group.
pub(crate) fn previewing(app: &VectorcraftApp) -> bool {
    let Some(open) = app.effect_dialogs.halftone.as_ref() else { return false };
    open.in_source(app)
        && owns_preview(app)
        && app.dialogs().is_some_and(|host| host.with(|h| h.get_by_id::<HostedParamDialog>(open.id).is_some_and(HostedParamDialog::is_previewing)))
}

impl Open {
    fn in_source(&self, app: &VectorcraftApp) -> bool {
        app.session.active().is_some_and(|doc| doc.uid == self.document)
    }
}

fn owns_preview(app: &VectorcraftApp) -> bool {
    app.session.active().and_then(|st| st.interaction.as_ref()?.preview.as_ref()).is_some_and(|(cmd, _)| cmd == CMD)
}
