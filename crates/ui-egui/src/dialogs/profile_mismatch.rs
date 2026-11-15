//! The questions Edit › Color Settings' Ask boxes add (Profile Mismatches: Ask When Opening and
//! Ask When Pasting, Missing Profiles: Ask When Opening), one dialog kind each for opening and for
//! pasting.
//!
//! Opening ([`OPEN_KIND`]): a document whose color mode's profile is not the working space asks
//! whether to use that profile, convert the colors to the working space or discard the profile; one
//! with no profile asks whether to leave it, assign the working space or assign a profile. The
//! choice the policy makes is chosen first. OK opens the document as chosen; Cancel doesn't open
//! it. The file read waits in [`VectorcraftApp::pending_profile_open`]. Fields: `issue` (`mismatch`
//! | `missing`), `name` (the file's), `model` (`RGB` | `CMYK`), `profile` (the one it is tagged
//! with), `working`, `choice` (a `profileChoice` of `document.open`) and `assignProfile`.
//!
//! Pasting ([`PASTE_KIND`]): colors copied from a document whose profile is not this document's ask
//! whether to keep their numbers or convert them. OK runs the paste with `profileMismatch`; Cancel
//! pastes nothing. Fields: `mismatches` (as `clipboard.profileMismatch` lists them), `choice`
//! (`preserveNumbers` | `convert`), `__command` and `__params` (the paste to run), and `__doc` (the
//! uid of the document it was asked in).

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, ProfileKind};
use vectorcraft_engine::cmd::colormgmt::ProfileChoice;
use vectorcraft_engine::cmd::fileio::{self, ReadFile};
use vectorcraft_engine::file_access::ScopeHandle;

use super::swatch_options::{grid, label};
use super::{DialogResult, DialogSpec};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the question opening asks.
pub const OPEN_KIND: &str = "profileMismatchOpen";
/// The dialog kind of the question pasting asks.
pub const PASTE_KIND: &str = "profileMismatchPaste";

pub(super) const OPEN_SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str("issue") == "missing" { tl!("Missing Profile") } else { tl!("Embedded Profile Mismatch") }.into(),
    body: open_body,
    confirm: open_confirm,
    min_width: 400.0,
    max_width: Some(480.0),
    ..DialogSpec::FORM
};

pub(super) const PASTE_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Paste Profile Mismatch").into(),
    body: paste_body,
    confirm: paste_confirm,
    min_width: 400.0,
    max_width: Some(480.0),
    ..DialogSpec::FORM
};

/// A file read for File › Open that waits for the answer to its profile question.
pub(crate) struct PendingOpen {
    file: ReadFile,
    /// Where it was read from: it goes on the recent files once it opens.
    path: Option<String>,
    /// The automation roots in force where the open was asked for: its linked files are read, and
    /// the document is installed, under them.
    scope: ScopeHandle,
}

/// The choices of a mismatch (`missing`: of a missing profile), as (`choice`, label).
fn open_choices(missing: bool) -> [(&'static str, &'static str); 3] {
    if missing {
        [("leave", tl!("Leave as is")), ("assignWorking", tl!("Assign the working space")), ("assign", tl!("Assign profile:"))]
    } else {
        [
            ("useEmbedded", tl!("Use the embedded profile")),
            ("convert", tl!("Convert the colors to the working space")),
            ("discard", tl!("Discard the embedded profile")),
        ]
    }
}

/// The choices pasting offers, as (`choice`, label).
fn paste_choices() -> [(&'static str, &'static str); 2] {
    [("preserveNumbers", tl!("Preserve the color numbers")), ("convert", tl!("Convert to keep the colors' appearance"))]
}

/// Open `file`, read for File › Open from `path`, as `document.open` does, unless Edit › Color
/// Settings asks about its profile ([`ReadFile::question`]): then the dialog asks first, and the
/// result is `{dialog}`. Opened, the views follow and the file goes on the recent files
/// ([`crate::io::opened`]).
pub fn open_or_ask(app: &mut VectorcraftApp, file: ReadFile, path: Option<String>) -> Result<Value, String> {
    // One question at a time: a file read while another waits opens as its policy says.
    if app.pending_profile_open.is_none()
        && let Some(q) = file.question()
    {
        let fields = json!({
            "issue": q.issue.id(),
            "name": file.name(),
            "model": q.model_label(),
            "profile": q.profile,
            "working": q.working,
            "choice": q.default.id(),
            "assignProfile": q.working,
        });
        app.pending_profile_open = Some(PendingOpen { file, path, scope: ScopeHandle::capture() });
        app.ui.dialog = Some(Dialog::new(OPEN_KIND, fields));
        return Ok(json!({ "dialog": OPEN_KIND }));
    }
    let r = fileio::open_read(&mut app.session, file).map_err(|e| e.to_string())?;
    crate::io::opened(app, &r);
    if let Some(p) = &path {
        crate::io::note_recent(app, p);
    }
    Ok(r)
}

/// The `profileChoice` the open dialog's fields give.
fn open_choice(d: &Dialog) -> Result<ProfileChoice, String> {
    let choice = d.str("choice");
    let profile = d.str("assignProfile");
    let profile = (choice == "assign").then_some(profile.as_str());
    ProfileChoice::parse(&choice, profile, "ui.dialog.confirm").map_err(|e| e.to_string())
}

fn open_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let missing = d.str("issue") == "missing";
    let (name, model, profile, working) = (d.str("name"), d.str("model"), d.str("profile"), d.str("working"));
    let vars = [("name", name.as_str()), ("model", model.as_str()), ("profile", profile.as_str()), ("working", working.as_str())];
    let text = if missing {
        tl!("“{name}” is not tagged with a {model} profile.")
    } else {
        tl!("“{name}” is tagged with the {model} profile “{profile}”, which is not the {model} working space “{working}”.")
    };
    ui.label(crate::i18n::fmt(text, &vars));
    ui.add_space(10.0);
    let choice = d.str("choice");
    for (value, text) in open_choices(missing) {
        ui.horizontal(|ui| {
            if widgets::radio(ui, text, choice == value, true) {
                d.fields.insert("choice".into(), json!(value));
            }
            if value == "assign" {
                let kind = if model == "CMYK" { ProfileKind::Cmyk } else { ProfileKind::Rgb };
                let current = d.str("assignProfile");
                ui.add_enabled_ui(choice == "assign", |ui| {
                    if let Some(n) = crate::panels::separations::profile_dropdown(ui, "profile-mismatch-assign", &current, kind, None) {
                        d.fields.insert("assignProfile".into(), json!(n));
                    }
                });
            }
        });
    }
    false
}

/// OK: open the waiting file as chosen, under the automation roots of the open. A choice the file
/// refuses keeps the dialog open.
fn open_confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let choice = open_choice(d)?;
    let Some(mut pending) = app.pending_profile_open.take() else {
        app.ui.dialog = None;
        return Ok(Value::Null);
    };
    if let Err(e) = pending.scope.clone().run(|| pending.file.answer(choice)) {
        app.pending_profile_open = Some(pending);
        return Err(e.to_string());
    }
    app.ui.dialog = None;
    let PendingOpen { file, path, scope } = pending;
    let name = file.name();
    scope.run(|| match fileio::open_read(&mut app.session, file) {
        Ok(r) => {
            crate::io::opened(app, &r);
            if let Some(p) = &path {
                crate::io::note_recent(app, p);
            }
            Ok(r)
        }
        Err(e) => {
            let e = e.to_string();
            crate::io::report_open_error(app, &name, &e);
            Err(e)
        }
    })
}

/// Cancel, or the dialog closed some other way: the waiting file doesn't open.
pub(crate) fn cancel_open(app: &mut VectorcraftApp) {
    app.pending_profile_open = None;
}

/// A file still waiting once its dialog is gone (another dialog took its place) doesn't open.
pub(crate) fn settle(app: &mut VectorcraftApp) {
    if app.pending_profile_open.is_some() && !app.ui.dialog.as_ref().is_some_and(|d| d.kind == OPEN_KIND) {
        cancel_open(app);
    }
}

/// Before running paste `command` with `params`: when Ask When Pasting is on, `params` don't say
/// what to do (`profileMismatch`) and the pasted colors come from another profile, open the dialog
/// instead and answer `{dialog}`.
pub fn ask_paste(app: &mut VectorcraftApp, command: &str, params: &Value) -> Option<DialogResult> {
    if params.get("profileMismatch").is_some() || !cms::working_settings().ask_mismatch_paste {
        return None;
    }
    let found = app.session.execute("clipboard.profileMismatch", &json!({})).ok()?;
    let mismatches = found["mismatches"].as_array().filter(|a| !a.is_empty())?.clone();
    // The policy of the first model starts chosen.
    let choice = mismatches[0]["default"].as_str().unwrap_or("preserveNumbers").to_string();
    let doc = app.session.active().map(|st| st.uid);
    let fields = json!({ "mismatches": mismatches, "choice": choice, "__command": command, "__params": params, DOC: doc });
    // A deferred paste this dialog asked about gives way to the new one.
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == PASTE_KIND) {
        crate::placing::cancel_swatch_paste(app);
    }
    app.ui.dialog = Some(Dialog::new(PASTE_KIND, fields));
    Some(Ok(json!({ "dialog": PASTE_KIND })))
}

fn paste_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let mismatches = d.fields.get("mismatches").and_then(Value::as_array).cloned().unwrap_or_default();
    for m in &mismatches {
        let model = m["model"].as_str().unwrap_or_default().to_ascii_uppercase();
        let (from, to) = (m["clipboard"].as_str().unwrap_or_default(), m["document"].as_str().unwrap_or_default());
        let text = tl!("The pasted colors come from a document whose {model} profile is “{from}”; this document's is “{to}”.");
        ui.label(crate::i18n::fmt(text, &[("model", model.as_str()), ("from", from), ("to", to)]));
    }
    ui.add_space(10.0);
    let choice = d.str("choice");
    grid(ui, |ui| {
        for (value, text) in paste_choices() {
            label(ui, "");
            if widgets::radio(ui, text, choice == value, true) {
                d.fields.insert("choice".into(), json!(value));
            }
            ui.end_row();
        }
    });
    false
}

/// The field that holds the uid of the document the paste question is about.
const DOC: &str = "__doc";

/// OK: run the paste with the choice; any Swatch Conflict question comes next. When another
/// document has become the active one since a paste that runs at once asked, it closes as Cancel
/// does and pastes nothing.
fn paste_confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let choice = d.str("choice");
    if !paste_choices().iter().any(|(v, _)| *v == choice) {
        let key = "choice";
        return Err(format!("`{key}` must be one of {}", "preserveNumbers, convert"));
    }
    let deferred = d.fields.get("__deferredPaste").and_then(Value::as_u64);
    if deferred.is_none() && d.fields.get(DOC).and_then(Value::as_u64) != app.session.active().map(|st| st.uid) {
        app.ui.dialog = None;
        return Ok(json!({}));
    }
    let mut params = d.fields.get("__params").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    params["profileMismatch"] = json!(choice);
    app.ui.dialog = None;
    // A paste that waited for its clipboard runs in the context it was given in.
    crate::placing::confirm_swatch_paste(app, &d.str("__command"), params, deferred)
}

#[cfg(test)]
mod tests;
