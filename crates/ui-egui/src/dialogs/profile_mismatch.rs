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
//! Pasting ([`PASTE_KIND`]): colors copied, or dragged ([`DROP`]), from a document whose profile is
//! not this document's ask whether to keep their numbers or convert them. OK runs the paste or the
//! drop with `profileMismatch`; Cancel pastes or drops nothing. Fields: `mismatches` (as
//! `clipboard.profileMismatch` lists them), `choice` (`preserveNumbers` | `convert`, or with both
//! models `{rgb, cmyk}` of those, one for each), `__command`
//! and `__params` (the paste or drop to run), and `__doc` (the uid of the document it was asked in).

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, ProfileKind};
use vectorcraft_engine::cmd::colormgmt::{ProfileChoice, ProfileQuestion};
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
/// The command a drag of artwork from another document runs, which asks as a paste does.
pub const DROP: &str = "edit.dropSelection";

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
    /// the document is installed, under them and those of the caller that confirms.
    scope: ScopeHandle,
    /// It was read off the UI thread ([`crate::opening`]): the rest of its read is done there too
    /// once the question is answered. An open that answers at once (a path a caller asked for)
    /// finishes before OK returns.
    background: bool,
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
/// Settings asks about its profile ([`ReadFile::ask`]): then the dialog asks first, and the result
/// is `{dialog}`. A file read while another one's question is open waits for its own question
/// until that one is answered (`{dialog, queued: true}`). Opened, the views follow and the file
/// goes on the recent files ([`crate::io::opened`]). `background`: the file was read off the UI
/// thread, and the rest of its read after an answer is done there too.
pub fn open_or_ask(app: &mut VectorcraftApp, file: ReadFile, path: Option<String>, background: bool) -> Result<Value, String> {
    ask_or_open(app, PendingOpen { file, path, scope: ScopeHandle::capture(), background })
}

/// [`open_or_ask`] for a file that has its scope already.
fn ask_or_open(app: &mut VectorcraftApp, mut pending: PendingOpen) -> Result<Value, String> {
    let Some(q) = pending.file.ask() else {
        let PendingOpen { file, path, scope, .. } = pending;
        return scope.run(|| {
            let r = fileio::open_read(&mut app.session, file).map_err(|e| e.to_string())?;
            crate::io::opened(app, &r);
            if let Some(p) = &path {
                crate::io::note_recent(app, p);
            }
            Ok(r)
        });
    };
    if app.pending_profile_open.is_some() {
        app.queued_profile_opens.push_back(pending);
        return Ok(json!({ "dialog": OPEN_KIND, "queued": true }));
    }
    app.ui.dialog = Some(Dialog::new(OPEN_KIND, question_fields(&q, &pending.file.name())));
    app.pending_profile_open = Some(pending);
    Ok(json!({ "dialog": OPEN_KIND }))
}

/// The open dialog's fields for question `q` about the file named `name`.
fn question_fields(q: &ProfileQuestion, name: &str) -> Value {
    json!({
        "issue": q.issue.id(),
        "name": name,
        "model": q.model_label(),
        "profile": q.profile,
        "working": q.working,
        "choice": q.default.id(),
        "assignProfile": q.working,
    })
}

/// With no dialog open, the next file waiting for its profile question asks it, or opens as its
/// policy says when the question is gone (Color Settings changed meanwhile). A file that fails to
/// open is reported, and the next one comes.
pub(crate) fn ask_next(app: &mut VectorcraftApp) {
    while app.pending_profile_open.is_none() && app.ui.dialog.is_none() {
        let Some(next) = app.queued_profile_opens.pop_front() else { return };
        let name = next.file.name();
        if let Err(e) = ask_or_open(app, next) {
            crate::io::report_open_error(app, &name, &e);
        }
    }
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

/// OK: open the waiting file as chosen, under the automation roots of the open and those of the
/// caller that confirms both. A choice the file refuses keeps the dialog open. When Color Settings
/// changed the question since it was asked, the dialog asks the question as it stands now instead,
/// or the file opens as its policy says when there is no question any more. The next file waiting
/// for its question asks it once no dialog is open ([`settle`]).
fn open_confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let choice = open_choice(d)?;
    let Some(mut pending) = app.pending_profile_open.take() else {
        app.ui.dialog = None;
        return Ok(Value::Null);
    };
    let scope = pending.scope.clone().within_current();
    if let Err(e) = scope.clone().run(|| pending.file.answer(choice)) {
        // The question changed: ask it as it is now (or open by the policy when it is gone).
        let now = |q: &ProfileQuestion| question_fields(q, "");
        let asked = |q: &ProfileQuestion| ["issue", "model", "profile", "working"].iter().all(|k| now(q).get(*k) == d.fields.get(*k));
        return match pending.file.ask() {
            Some(q) if asked(&q) => {
                app.pending_profile_open = Some(pending);
                Err(e.to_string())
            }
            Some(q) => {
                app.ui.dialog = Some(Dialog::new(OPEN_KIND, question_fields(&q, &pending.file.name())));
                app.pending_profile_open = Some(pending);
                Ok(json!({ "dialog": OPEN_KIND }))
            }
            None => {
                app.ui.dialog = None;
                open_pending(app, pending, scope)
            }
        };
    }
    app.ui.dialog = None;
    open_pending(app, pending, scope)
}

/// Open `pending`, answered or without a question, under `scope`; a failure is reported. A file read
/// off the UI thread does the rest of its read (its linked files and type) there too
/// ([`crate::opening::finish_answered`]).
fn open_pending(app: &mut VectorcraftApp, pending: PendingOpen, scope: ScopeHandle) -> DialogResult {
    let PendingOpen { file, path, background, .. } = pending;
    let name = file.name();
    let file = match &path {
        Some(p) if background && crate::opening::available() => match crate::opening::finish_answered(app, p, file, scope.clone()) {
            Ok(r) => return Ok(r),
            Err(Some(file)) => *file,
            Err(None) => {
                let e = "the file stopped being read (please report this bug)".to_string();
                crate::io::report_open_error(app, &name, &e);
                return Err(e);
            }
        },
        _ => file,
    };
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

/// Cancel, or the dialog closed some other way: the waiting file doesn't open. The next file waiting
/// for its question asks it once no dialog is open ([`settle`]).
pub(crate) fn cancel_open(app: &mut VectorcraftApp) {
    app.pending_profile_open = None;
}

/// A file still waiting once its dialog is gone (another dialog took its place) doesn't open; with
/// no dialog open, the next file waiting for its question asks it.
pub(crate) fn settle(app: &mut VectorcraftApp) {
    if app.pending_profile_open.is_some() && !app.ui.dialog.as_ref().is_some_and(|d| d.kind == OPEN_KIND) {
        app.pending_profile_open = None;
    }
    ask_next(app);
}

/// Before running paste `command` (or `edit.dropSelection`, a drag from another document) with
/// `params`: when Ask When Pasting is on, `params` don't say what to do (`profileMismatch`) and the
/// pasted or dropped colors come from another profile, open the dialog instead and answer
/// `{dialog}`.
pub fn ask_paste(app: &mut VectorcraftApp, command: &str, params: &Value) -> Option<DialogResult> {
    // A null choice is no choice, as the paste commands read it.
    if params.get("profileMismatch").is_some_and(|v| !v.is_null()) || !cms::working_settings().ask_mismatch_paste {
        return None;
    }
    let mismatches = paste_mismatches(app, command, params).filter(|a| !a.is_empty())?;
    let choice = first_choice(&mismatches);
    let doc = app.session.active().map(|st| st.uid);
    let fields = json!({ "mismatches": mismatches, "choice": choice, "__command": command, "__params": params, DOC: doc });
    // A deferred paste this dialog asked about gives way to the new one.
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == PASTE_KIND) {
        crate::placing::cancel_swatch_paste(app);
    }
    app.ui.dialog = Some(Dialog::new(PASTE_KIND, fields));
    Some(Ok(json!({ "dialog": PASTE_KIND })))
}

/// The models whose profiles differ for paste `command` with `params` (`clipboard.profileMismatch`):
/// a drop brings the objects it names, a paste the clipboard's, and Paste on All Artboards leaves the
/// backgrounds of copied artboards behind. `None` when they can't be listed.
fn paste_mismatches(app: &mut VectorcraftApp, command: &str, params: &Value) -> Option<Vec<Value>> {
    let query = match command {
        DROP => json!({ "sourceDocument": params.get("sourceDocument"), "ids": params.get("ids") }),
        ALL_ARTBOARDS => json!({ "allArtboards": true }),
        _ => json!({}),
    };
    let found = app.session.execute("clipboard.profileMismatch", &query).ok()?;
    found["mismatches"].as_array().cloned()
}

/// Paste on All Artboards, which leaves copied artboards behind.
const ALL_ARTBOARDS: &str = "edit.pasteOnAllArtboards";

/// Can the colors of mismatch `m` convert (both profiles are available)?
fn convertible(m: &Value) -> bool {
    m["convertible"].as_bool().unwrap_or(true)
}

/// May the colors of mismatch `m` take `value`? Preserve Numbers always; Convert only when they can
/// convert.
fn allowed(m: &Value, value: &str) -> bool {
    value != "convert" || convertible(m)
}

/// The choice the paste question starts on: the model's policy for one model, and for both a
/// choice for each (`{rgb, cmyk}`), each starting on its model's policy.
fn first_choice(mismatches: &[Value]) -> Value {
    let default = |m: &Value| m["default"].as_str().unwrap_or("preserveNumbers").to_string();
    match mismatches {
        [m] => json!(default(m)),
        _ => Value::Object(mismatches.iter().filter_map(|m| Some((m["model"].as_str()?.to_string(), json!(default(m))))).collect()),
    }
}

/// What `choice` (one choice for every model, or `{rgb?, cmyk?}`) picks for the colors of `model`.
fn picked<'a>(choice: &'a Value, model: &str) -> &'a str {
    choice.as_str().or_else(|| choice.get(model).and_then(Value::as_str)).unwrap_or_default()
}

/// `choice` with `value` picked for the colors of `model`: with one model the value alone, with
/// both a choice for each in which the other model's stays as `choice` has it.
fn pick(choice: &Value, mismatches: &[Value], model: &str, value: &str) -> Value {
    if mismatches.len() < 2 {
        return json!(value);
    }
    let each =
        mismatches.iter().filter_map(|m| m["model"].as_str()).map(|m| (m.to_string(), json!(if m == model { value } else { picked(choice, m) })));
    Value::Object(each.collect())
}

/// Is `choice` one choice for every model, or a choice for each of `mismatches`' models? (Convert
/// for a model whose colors can't convert keeps their numbers, as the paste commands do.)
fn valid_choice(choice: &Value, mismatches: &[Value]) -> bool {
    let known = |v: &str| paste_choices().iter().any(|(c, _)| *c == v);
    match choice {
        Value::String(s) => known(s),
        Value::Object(o) => {
            let models: Vec<&str> = mismatches.iter().filter_map(|m| m["model"].as_str()).collect();
            o.len() == models.len() && models.iter().all(|m| o.get(*m).and_then(Value::as_str).is_some_and(known))
        }
        _ => false,
    }
}

fn paste_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let mismatches = d.fields.get("mismatches").and_then(Value::as_array).cloned().unwrap_or_default();
    let choice = d.fields.get("choice").cloned().unwrap_or(Value::Null);
    // With both models, each has its own choice under its line; with one, the choice follows it.
    for m in &mismatches {
        let id = m["model"].as_str().unwrap_or_default();
        let model = id.to_ascii_uppercase();
        let (from, to) = (m["clipboard"].as_str().unwrap_or_default(), m["document"].as_str().unwrap_or_default());
        let text = tl!("The pasted colors come from a document whose {model} profile is “{from}”; this document's is “{to}”.");
        ui.label(crate::i18n::fmt(text, &[("model", model.as_str()), ("from", from), ("to", to)]));
        ui.add_space(6.0);
        let now = picked(&choice, id);
        let clicked = ui.push_id(id, |ui| {
            grid(ui, |ui| {
                let mut clicked = None;
                for (value, text) in paste_choices() {
                    label(ui, "");
                    // Colors whose profile is not available can only keep their numbers.
                    if widgets::radio(ui, text, now == value, allowed(m, value)) {
                        clicked = Some(value);
                    }
                    ui.end_row();
                }
                clicked
            })
        });
        if let Some(value) = clicked.inner {
            d.fields.insert("choice".into(), pick(&choice, &mismatches, id, value));
        }
        ui.add_space(10.0);
    }
    false
}

/// The field that holds the uid of the document the paste question is about.
const DOC: &str = "__doc";

/// OK: run the paste with the choice; any Swatch Conflict question comes next. When another
/// document has become the active one since a paste that runs at once asked, it closes as Cancel
/// does and pastes nothing. When the profiles that paste or drop would bring have changed since it
/// asked (the clipboard, the dragged objects' document or the destination's profile changed), it
/// asks again about the new ones; a paste that waited for its clipboard is checked in the document
/// and with the clipboard it runs with.
fn paste_confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let choice = d.fields.get("choice").cloned().unwrap_or(Value::Null);
    let asked = d.fields.get("mismatches").and_then(Value::as_array).cloned().unwrap_or_default();
    if !valid_choice(&choice, &asked) {
        let key = "choice";
        return Err(format!("`{key}` must be one of {}", "preserveNumbers, convert, {rgb, cmyk} of those"));
    }
    let deferred = d.fields.get("__deferredPaste").and_then(Value::as_u64);
    if deferred.is_none() && d.fields.get(DOC).and_then(Value::as_u64) != app.session.active().map(|st| st.uid) {
        app.ui.dialog = None;
        return Ok(json!({}));
    }
    let command = d.str("__command");
    let mut params = d.fields.get("__params").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    params["profileMismatch"] = choice;
    app.ui.dialog = None;
    // Without the answer, the paste asks again about what it brings now.
    let changed = |app: &mut VectorcraftApp, command: &str, params: &Value| {
        paste_mismatches(app, command, params).is_some_and(|now| !now.is_empty() && now != asked)
    };
    let recheck = crate::placing::Recheck { key: "profileMismatch", changed: &changed };
    // A paste that waited for its clipboard runs in the context it was given in.
    crate::placing::confirm_swatch_paste(app, &command, params, deferred, Some(&recheck))
}

#[cfg(test)]
mod tests;
