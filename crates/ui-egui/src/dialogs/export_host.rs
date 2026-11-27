//! The export dialogs on the dialog host: Export As, the raster, SVG, EPS, DXF and HP-GL options, Save for
//! Office, Text Export and the Save options. Each is a hosted parameter dialog whose OK runs its kind's `confirm`
//! (through `host::open_held_with`); a refusal keeps the dialog open with its reason, and what has to follow the
//! dialog's closing (the save panel, the next dialog, the file) waits on the record here until the host reports
//! the dialog gone. The rows' own requests (the SVG browser preview, the EPS `Custom…` and HP-GL pen table
//! buttons) are answered in [`pump`].

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, ParamResult};
use serde_json::Value;

use super::host::{self, HeldOptions};
use super::{dxf_options, eps_options, export_as, hpgl_options, office_export, png_options, save_options, svg_options, text_export};
use crate::VectorcraftApp;

/// What runs once the dialog that confirmed has closed. It has no reply to give: its failure goes to the status
/// line, and a command that returns a value calls the step itself.
pub(crate) type After = Option<Box<dyn FnOnce(&mut VectorcraftApp) -> Result<Value, String>>>;

/// Which export dialog a record is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Export {
    /// Export As.
    As {
        /// The drawing whose export the dialog configures, when one was open.
        document: Option<u64>,
    },
    /// The options of the raster format `format`.
    Raster {
        /// The format's id.
        format: &'static str,
    },
    /// SVG Options, for Export As, Save As or Save a Copy.
    Svg {
        /// What OK does.
        mode: svg_options::Mode,
    },
    /// EPS Options.
    Eps,
    /// Custom Transparency Flattener Options, over EPS Options.
    EpsCustom {
        /// The EPS Options dialog it edits.
        parent: DialogId,
    },
    /// DXF Options.
    Dxf,
    /// HP-GL Options, for the document `doc`.
    Hpgl {
        /// The uid of the document the pens are read from.
        doc: u64,
    },
    /// Save for Office Documents.
    Office,
    /// Text Export Options.
    Text,
    /// The options a save or open asks for before it goes on, for `action`.
    Save {
        /// The action the options belong to.
        action: String,
    },
}

/// One open export dialog.
pub(crate) struct Open {
    /// The hosted dialog's id.
    pub id: DialogId,
    /// The drawing whose options and output this dialog controls.
    document: Option<u64>,
    /// Which dialog it is.
    pub kind: Export,
    /// Whether Export Selection opened it: OK then writes the selection alone.
    pub selection: bool,
    /// What runs once the dialog has closed after an OK; empty for a dialog that was cancelled.
    pub after: After,
}

/// The export dialogs that are open (`app.export_dialogs`), and HP-GL Options closed for the save panel its OK
/// asked for, shown again when the panel closes without a file.
#[derive(Default)]
pub(crate) struct ExportDialogs(Vec<Open>, pub(super) Option<hpgl_options::Asked>);

impl ExportDialogs {
    fn find(&self, id: DialogId) -> Option<&Open> {
        self.0.iter().find(|o| o.id == id)
    }

    /// The id of the dialog opened last, for a caller that opened it through a kind's own `open`.
    pub(super) fn last_id(&self) -> Option<DialogId> {
        self.0.last().map(|o| o.id)
    }
}

impl Open {
    fn check_source(&self, app: &VectorcraftApp) -> Result<(), String> {
        if !matches!(self.kind, Export::Hpgl { .. }) && self.document != app.session.active().map(|doc| doc.uid) {
            return Err(tl!("Return to the original drawing to export it").into());
        }
        Ok(())
    }
}

/// `ids` paired with `labels` as the options of a choice row.
pub(super) fn options<L: From<&'static str>>(ids: &[&'static str], labels: &[&'static str]) -> Vec<(String, L)> {
    ids.iter().zip(labels).map(|(id, label)| (id.to_string(), L::from(label))).collect()
}

/// Opens `dialog` as the export dialog `kind` and records it; `None` without a window. A dialog of the same key
/// replaces the one open, and its record.
pub(crate) fn open(app: &mut VectorcraftApp, kind: Export, selection: bool, dialog: HostedParamDialog) -> Option<DialogId> {
    let document = match &kind {
        Export::As { document } | Export::Hpgl { doc: document } => Some(*document),
        Export::EpsCustom { parent } => app.export_dialogs.find(*parent)?.document,
        _ => app.session.active().map(|doc| doc.uid),
    };
    let options = match &kind {
        Export::EpsCustom { parent } => HeldOptions { parent: Some(*parent), cancel: None },
        _ => HeldOptions::default(),
    };
    let id = host::open_held_with(app, dialog, answer, options)?;
    app.export_dialogs.0.retain(|o| o.id != id);
    app.export_dialogs.0.push(Open { id, document, kind, selection, after: None });
    Some(id)
}

/// Sets `fields` on the open dialog `id`, as the user editing its rows would.
pub(super) fn set_fields(app: &VectorcraftApp, id: DialogId, fields: &[(&str, Value)]) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    // PLAN: assumed `VectorcraftApp::dialogs(&self) -> Option<WindowDialogs>` (wg.S1)
    let dialogs = app.dialogs().ok_or("no window")?;
    let fields: Vec<(String, Value)> = fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    dialogs.with(|host| host.set_json_fields(&ctx, id, &fields))
}

/// OK of the dialog `result` is from: its kind's `confirm`, and the step that follows kept for [`pump`].
fn answer(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    let open = app.export_dialogs.find(result.id).ok_or("this export dialog is no longer open")?;
    open.check_source(app)?;
    let id = open.id;
    let open = Open { id, document: open.document, kind: open.kind.clone(), selection: open.selection, after: None };
    let after = confirm(app, &open, &result.fields)?;
    if let Some(record) = app.export_dialogs.0.iter_mut().find(|o| o.id == id) {
        record.after = after;
    }
    Ok(())
}

fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    match &open.kind {
        Export::As { document } => export_as::confirm(app, *document, fields),
        Export::Raster { .. } => png_options::confirm(app, open, fields),
        Export::Svg { .. } => svg_options::confirm(app, open, fields),
        Export::Eps => eps_options::confirm(app, open, fields),
        Export::EpsCustom { parent } => confirm_custom(app, *parent, fields),
        Export::Dxf => dxf_options::confirm(app, open, fields),
        Export::Hpgl { .. } => hpgl_options::confirm(app, open, fields),
        Export::Office => office_export::confirm(app, open, fields),
        Export::Text => text_export::confirm(app, open, fields),
        Export::Save { .. } => save_options::confirm(app, open, fields),
    }
}

/// The values of the open dialog `id`.
fn fields_of(app: &VectorcraftApp, id: DialogId) -> Option<DialogFields> {
    app.dialogs()?.with(|host| host.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone()))
}

/// Custom… of EPS Options `parent`: Custom Transparency Flattener Options over it.
fn open_custom(app: &mut VectorcraftApp, parent: DialogId) {
    let Some(fields) = fields_of(app, parent) else { return };
    let dialog = eps_options::custom_dialog(app, parent, &fields);
    if open(app, Export::EpsCustom { parent }, false, dialog).is_none() {
        app.status("no window");
    }
}

/// OK of Custom Transparency Flattener Options: EPS Options `parent` takes the custom options. The host's lock is
/// released while a held OK runs, so the parent is written through it here.
fn confirm_custom(app: &mut VectorcraftApp, parent: DialogId, fields: &DialogFields) -> Result<After, String> {
    let mut values = fields_of(app, parent).ok_or("EPS Options is no longer open")?;
    eps_options::custom_ok(app, &mut values, fields)?;
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| {
        if let Some(d) = host.get_mut_by_id::<HostedParamDialog>(parent) {
            for (key, value) in values.iter() {
                d.set_field(key, value.clone());
            }
        }
    });
    Ok(None)
}

/// Reports a failed step on the status line.
fn report<T>(app: &mut VectorcraftApp, result: Result<T, String>) {
    if let Err(e) = result {
        app.status(e);
    }
}

/// Once per frame: answers the rows' requests of each open export dialog, and runs the step after a confirmed
/// dialog that has closed. Each dialog's requests are taken once, so a second pass in the same frame finds none.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    if !app.file_asks.asking()
        && let Some(asked) = app.export_dialogs.1.take()
    {
        hpgl_options::reopen(app, asked);
    }
    // PLAN: assumed `VectorcraftApp::dialogs(&self) -> Option<WindowDialogs>` (wg.S1)
    let Some(dialogs) = app.dialogs() else { return };
    let ids: Vec<DialogId> = app.export_dialogs.0.iter().map(|o| o.id).collect();
    for id in ids {
        let taken = dialogs.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(|d| (d.take_action(), d.take_actions())));
        let Some(open) = app.export_dialogs.find(id) else { continue };
        let (kind, selection) = (open.kind.clone(), open.selection);
        let Some((footer, actions)) = taken else {
            // The dialog closed: the step after an OK runs, a cancelled one has none.
            if let Some(i) = app.export_dialogs.0.iter().position(|o| o.id == id) {
                let mut record = app.export_dialogs.0.remove(i);
                if let Some(after) = record.after.take() {
                    let result = record.check_source(app).and_then(|()| after(app));
                    report(app, result);
                }
            }
            continue;
        };
        if let Err(error) = open.check_source(app) {
            if footer.is_some() || !actions.is_empty() {
                app.status(error);
            }
            continue;
        }
        if let Export::Svg { mode } = kind {
            if footer.as_deref() == Some(svg_options::BROWSER_ACTION)
                && let Some(fields) = fields_of(app, id)
            {
                let result = svg_options::browser_preview(app, &svg_options::preview_params(mode, selection, &fields));
                report(app, result);
            }
            svg_options::refresh_code(app, id, mode, selection);
        }
        for action in actions {
            match (&kind, action.as_str()) {
                (Export::Eps, "custom") => open_custom(app, id),
                (Export::Hpgl { .. }, "penTable") => hpgl_options::pen_ask(app, id, selection),
                _ => {}
            }
        }
    }
}
