//! The add-on dialogs' host side: each is a hosted parameter dialog keyed by its command, whose
//! values reach that command as params. Each previews on the canvas and runs its command at OK as
//! one undo step.

use std::mem;

use craft::ui::dialog::{DialogResult, StackDialog};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, ParamDialog};
use egui::Context;
use serde_json::{Value, json};

use crate::VectorcraftApp;
// PLAN: assumed `crate::dialogs::preview::LivePreview` (widgets-w11 owns wg.A20, the module is not given)
use crate::dialogs::preview::LivePreview;

#[cfg(test)]
pub(crate) mod tests;

/// Shows a preview run's answer (or its error) in the dialog.
pub(crate) type Found = fn(&mut HostedParamDialog, Result<&Value, &str>);

/// What one add-on dialog is: the command it answers for and how its values become that command's params.
pub(crate) struct Spec {
    /// The command id: the dialog's key, the command OK or Preview runs, and the `dialog` of the reply.
    pub command: &'static str,
    /// The command's params from the dialog's values.
    pub params: fn(&DialogFields) -> Value,
    /// Receives the answer of each preview run (or its error), to show it in the dialog.
    pub found: Option<Found>,
}

/// The add-on dialog open now and the live preview it drives; one at a time, since a preview owns the document.
#[derive(Default)]
pub(crate) struct AddonDialogs {
    open: Option<(craft::ui::dialog::DialogId, &'static Spec)>,
    preview: LivePreview,
    /// The open Tangent Circle Tool radius dialog.
    pub(crate) tangent_radius: Option<super::pathtools::tangent_circle_radius::Open>,
}

/// Closes the open add-on dialog without an answer and rolls its preview back.
fn close_open(app: &mut VectorcraftApp) {
    let Some((id, _)) = app.addon_dialogs.open.take() else { return };
    if let (Some(host), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) {
        host.with(|h| {
            h.cancel_by_id(&ctx, id);
            // The cancel's answer is of no use: the preview is rolled back below.
            let _ = h.take_by_id::<HostedParamDialog>(id);
        });
    }
    mem::take(&mut app.addon_dialogs.preview).cancel(app);
}

/// Opens the dialog `spec` describes over `fields`, built each pass by `build`.
pub(crate) fn open(
    app: &mut VectorcraftApp,
    spec: &'static Spec,
    fields: DialogFields,
    build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let dialog = HostedParamDialog::new(&ctx, spec.command, fields, build);
    let host = app.dialogs().ok_or("no window")?;
    close_open(app);
    let id = dialog.id();
    host.with(|h| h.open(dialog));
    app.addon_dialogs.open = Some((id, spec));
    Ok(json!({ "dialog": spec.command }))
}

/// The dialog the menu item of command `id` opens, with no params; `None` for another command.
pub(crate) fn open_from_menu(app: &mut VectorcraftApp, id: &str) -> Option<Result<Value, String>> {
    use super::{color, pathtools, select, typetools};
    let open: fn(&mut VectorcraftApp) -> Result<Value, String> = match id {
        pathtools::set_corners::COMMAND => pathtools::set_corners::open,
        pathtools::dashes::COMMAND => pathtools::dashes::open,
        pathtools::bridge::COMMAND => pathtools::bridge::open,
        pathtools::tangent_circle::COMMAND => pathtools::tangent_circle::open,
        typetools::COMMAND => typetools::open,
        color::legend::COMMAND => color::legend::open,
        color::nearest::COMMAND => color::nearest::open,
        select::COMMAND => select::open,
        _ => return super::arrange::open(app, id),
    };
    Some(open(app))
}

/// Once per frame: runs the preview of the open add-on dialog, commits or cancels it when the dialog has
/// answered, then lets the dialogs that take their own answer do the same. Each takes its dialog by id, never
/// by type: two hosted parameter dialogs would take each other's answers.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &Context) {
    pump_commit(app);
    super::pathtools::tangent_circle_radius::pump(app, ctx);
    super::color::nearest::pump(app, ctx);
    super::select::pump(app, ctx);
    super::select::link_status::pump(app, ctx);
}

fn pump_commit(app: &mut VectorcraftApp) {
    let Some((id, spec)) = app.addon_dialogs.open else { return };
    let Some(host) = app.dialogs() else { return };
    let live = host.with(|h| h.get_mut_by_id::<HostedParamDialog>(id).map(|d| (d.is_previewing(), (spec.params)(d.fields()))));
    if let Some((previewing, params)) = &live {
        let mut preview = mem::take(&mut app.addon_dialogs.preview);
        // PLAN: the undo step's label is the command id; `Spec` carries no heading (wg.A20 takes a label)
        let answer = preview.run(app, *previewing, spec.command, spec.command, params);
        app.addon_dialogs.preview = preview;
        if let (Some(found), Some(answer)) = (spec.found, answer) {
            host.with(|h| {
                if let Some(d) = h.get_mut_by_id::<HostedParamDialog>(id) {
                    found(d, answer.as_ref().map_err(String::as_str));
                }
            });
        }
    }
    match host.with(|h| h.take_by_id::<HostedParamDialog>(id)).map(DialogResult::ok) {
        Some(Some(result)) => {
            app.addon_dialogs.open = None;
            let preview = mem::take(&mut app.addon_dialogs.preview);
            // The dialog is closed already, so a refusal can only go to the status line.
            if let Err(error) = preview.commit(app, spec.command, (spec.params)(&result.fields)) {
                app.status(error);
            }
        }
        Some(None) => {
            app.addon_dialogs.open = None;
            mem::take(&mut app.addon_dialogs.preview).cancel(app);
        }
        None if live.is_none() => {
            app.addon_dialogs.open = None;
            mem::take(&mut app.addon_dialogs.preview).cancel(app);
        }
        None => {}
    }
}
