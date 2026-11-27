//! The questions Edit › Color Settings' Ask boxes add (Profile Mismatches: Ask When Opening and
//! Ask When Pasting, Missing Profiles: Ask When Opening), one dialog each for opening and for
//! pasting, both on the window's dialog host.
//!
//! Opening ([`OPEN_KIND`]): a document whose color mode's profile is not the working space asks
//! whether to use that profile, convert the colors to the working space or discard the profile; one
//! with no profile asks whether to leave it, assign the working space or assign a profile. The
//! choice the policy makes is chosen first. OK opens the document as chosen; Cancel doesn't open
//! it. The file read waits in [`VectorcraftApp::pending_profile_open`]; [`pump`] takes the answer.
//! The dialog's fields are `action` (`useEmbedded`, `convertToWorking`, `discard`, `leaveAsIs`,
//! `assignWorking` or `assignProfile`) and, with `assignProfile`, `profile`.
//!
//! Pasting ([`PASTE_KIND`]): colors copied, or dragged ([`DROP`]), from a document whose profile is
//! not this document's ask whether to keep their numbers or convert them. OK runs the paste or the
//! drop with `profileMismatch`; Cancel pastes or drops nothing. The dialog has one radio field per
//! model (`rgb`, `cmyk`: `preserveNumbers` | `convert`). What is being pasted waits in
//! [`VectorcraftApp::pending_paste`].

use craft::color_ui::ui::{ProfileAction, ProfileAnswer, ProfileMismatchDialog, ProfileMode, ProfileQuestion as PromptQuestion};
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, ProfileKind};
use vectorcraft_engine::cmd::colormgmt::{ProfileChoice, ProfileQuestion};
use vectorcraft_engine::cmd::fileio::{self, ReadFile};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;
use crate::placing::Taken;

/// The key of the question opening asks (the `dialog` an open answers with).
pub const OPEN_KIND: &str = "profileMismatchOpen";
/// The key of the question pasting asks.
pub const PASTE_KIND: &str = "profileMismatchPaste";
/// The command a drag of artwork from another document runs, which asks as a paste does.
pub const DROP: &str = "edit.dropSelection";

/// A file read for File › Open that waits for the answer to its profile question.
pub(crate) struct PendingOpen {
    file: ReadFile,
    /// Where it was read from: it goes on the recent files once it opens.
    path: Option<String>,
    /// The automation roots in force where the open was asked for: its linked files are read, and
    /// the document is installed, under them and those of the caller that answers.
    scope: ScopeHandle,
    /// It was read off the UI thread ([`crate::opening`]): the rest of its read is done there too
    /// once the question is answered. An open that answers at once (a path a caller asked for)
    /// finishes before the answer returns.
    background: bool,
    /// The question the dialog asked ([`question_key`]); `Null` before one was asked. An answer the
    /// file refuses while the question is unchanged is a failure, not a new question.
    asked: Value,
}

/// A paste or drop that waits for the answer to its Paste Profile Mismatch question.
pub(crate) struct PasteAsk {
    /// The paste command, or [`DROP`].
    command: String,
    /// The command's params, without `profileMismatch`.
    params: Value,
    /// The models whose profiles differ, as `clipboard.profileMismatch` lists them.
    mismatches: Vec<Value>,
    /// The drawing and content generation the question was asked in.
    drawing: Option<(u64, u64)>,
    /// The chain of a deferred paste (one that waits for its clipboard), which placing answers.
    deferred: Option<u64>,
}

/// The engine's choice id and the dialog's action for each answer but Assign profile.
const ACTIONS: [(&str, ProfileAction); 5] = [
    ("useEmbedded", ProfileAction::UseEmbedded),
    ("convert", ProfileAction::ConvertToWorking),
    ("discard", ProfileAction::Discard),
    ("leave", ProfileAction::LeaveAsIs),
    ("assignWorking", ProfileAction::AssignWorking),
];

/// The choices pasting offers, as (`choice`, label).
const PASTE_CHOICES: [(&str, &str); 2] = [("preserveNumbers", "Preserve the color numbers"), ("convert", "Convert to keep the colors' appearance")];

/// Open `file`, read for File › Open from `path`, as `document.open` does, unless Edit › Color
/// Settings asks about its profile ([`ReadFile::ask`]): then the dialog asks first, and the result
/// is `{dialog}`. A file read while another one's question, or another dialog, is open waits for
/// its own question until that one closes (`{dialog, queued: true}`). Opened, the views follow and the file
/// goes on the recent files ([`crate::io::opened`]). `background`: the file was read off the UI
/// thread, and the rest of its read after an answer is done there too.
pub fn open_or_ask(app: &mut VectorcraftApp, file: ReadFile, path: Option<String>, background: bool) -> Result<Value, String> {
    ask_or_open(app, PendingOpen { file, path, scope: ScopeHandle::capture(), background, asked: Value::Null })
}

/// [`open_or_ask`] for a file that has its scope already. While another dialog is open (a paste
/// waiting for its Swatch Conflict answer, say), the question waits for it to close too.
fn ask_or_open(app: &mut VectorcraftApp, mut pending: PendingOpen) -> Result<Value, String> {
    let Some(q) = pending.file.ask() else {
        // The question is gone (or there was none): a file read off the UI thread does the rest of
        // its read there, as an answered one does.
        let scope = pending.scope.clone();
        return open_now(app, pending, scope);
    };
    if app.pending_profile_open.is_some() || app.any_dialog_open() {
        app.queued_profile_opens.push_back(pending);
        return Ok(json!({ "dialog": OPEN_KIND, "queued": true }));
    }
    ask(app, pending, &q)
}

/// Ask question `q` about `pending` on the host, and keep the file waiting for the answer.
fn ask(app: &mut VectorcraftApp, mut pending: PendingOpen, q: &ProfileQuestion) -> Result<Value, String> {
    pending.asked = question_key(q);
    let Some(dialogs) = app.dialogs() else {
        // No window to ask in: the policy decides, as when the Ask box is off.
        let choice = ProfileChoice::parse(q.default.id(), None, "document.open").map_err(|e| e.to_string())?;
        return finish_open(app, pending, choice);
    };
    let dialog = open_dialog(&pending.file.name(), q);
    app.pending_profile_open = Some(pending);
    dialogs.with(|h| h.open(dialog));
    Ok(json!({ "dialog": OPEN_KIND }))
}

/// What identifies question `q`: when it is the same after a refused answer, nothing changed.
fn question_key(q: &ProfileQuestion) -> Value {
    json!({ "issue": q.issue.id(), "model": q.model_label(), "profile": q.profile, "working": q.working })
}

/// The dialog that asks `q` about the file named `name`, starting on the choice the policy makes.
// PLAN: the Assign profile list starts on its first name, where production started on the working space.
fn open_dialog(name: &str, q: &ProfileQuestion) -> ProfileMismatchDialog {
    let (mode, kind) = if q.model_label() == "CMYK" { (ProfileMode::Cmyk, ProfileKind::Cmyk) } else { (ProfileMode::Rgb, ProfileKind::Rgb) };
    let missing = q.issue.id() == "missing";
    let question =
        if missing { PromptQuestion::missing(&q.working) } else { PromptQuestion::mismatch(q.profile.as_deref().unwrap_or_default(), &q.working) };
    let mut dialog = ProfileMismatchDialog::new(name, mode, question).rememberable(false);
    if let Some((_, action)) = ACTIONS.iter().find(|(id, _)| *id == q.default.id()) {
        dialog = dialog.action(*action);
    }
    if missing {
        let names = crate::panels::separations::profile_entries(kind, None).into_iter().map(|(id, _)| id).collect();
        dialog = dialog.profiles(names);
    }
    dialog
}

/// With no dialog open, the next file waiting for its profile question asks it, or opens as its
/// policy says when the question is gone (Color Settings changed meanwhile). A file that fails to
/// open is reported, and the next one comes.
pub(crate) fn ask_next(app: &mut VectorcraftApp) {
    while app.pending_profile_open.is_none() && !app.any_dialog_open() {
        let Some(next) = app.queued_profile_opens.pop_front() else { return };
        let name = next.file.name();
        if let Err(e) = ask_or_open(app, next) {
            crate::io::report_open_error(app, &name, &e);
        }
    }
}

/// The `profileChoice` the dialog's answer gives.
fn open_choice(answer: ProfileAnswer) -> Result<ProfileChoice, String> {
    let (id, profile) = match answer.action {
        ProfileAction::AssignProfile => ("assign", answer.profile.as_deref()),
        action => (ACTIONS.iter().find(|(_, a)| *a == action).map(|(id, _)| *id).ok_or("this question has no such choice")?, None),
    };
    ProfileChoice::parse(id, profile, "ui.dialog.confirm").map_err(|e| e.to_string())
}

/// Run what the open dialogs and the paste question left to do, once per frame: open the file the
/// answer opens (or forget it on Cancel), run the paste that was answered, and ask the next file's
/// question once no dialog is open.
pub(crate) fn pump(app: &mut VectorcraftApp, _: &egui::Context) {
    let answered = app.dialogs().and_then(|d| d.with(|h| h.take::<ProfileMismatchDialog>()));
    match answered.map(DialogResult::ok) {
        Some(Some(answer)) => answer_open(app, answer),
        Some(None) => cancel_open(app),
        None => {}
    }
    pump_paste(app);
    if !app.any_dialog_open() {
        ask_next(app);
    }
}

/// OK: open the waiting file as chosen.
fn answer_open(app: &mut VectorcraftApp, answer: ProfileAnswer) {
    let Some(pending) = app.pending_profile_open.take() else { return };
    match open_choice(answer) {
        // A failed open is reported where it fails.
        Ok(choice) => drop(finish_open(app, pending, choice)),
        Err(e) => app.status(e),
    }
}

/// Open the waiting `pending` as `choice`, under the automation roots of the open and those of the
/// caller that answers both. When Color Settings changed the question since it was asked, the
/// dialog asks the question as it stands now instead, or the file opens as its policy says when
/// there is no question any more.
fn finish_open(app: &mut VectorcraftApp, mut pending: PendingOpen, choice: ProfileChoice) -> Result<Value, String> {
    let scope = pending.scope.clone().within_current();
    if let Err(e) = scope.clone().run(|| pending.file.answer(choice)) {
        let e = e.to_string();
        return match pending.file.ask() {
            Some(q) if question_key(&q) == pending.asked => {
                crate::io::report_open_error(app, &pending.file.name(), &e);
                Err(e)
            }
            Some(q) => ask(app, pending, &q),
            None => open_pending(app, pending, scope),
        };
    }
    open_pending(app, pending, scope)
}

/// Open `pending`, answered or without a question, under `scope`; a failure is reported. A file read
/// off the UI thread does the rest of its read (its linked files and type) there too
/// ([`crate::opening::finish_answered`]).
fn open_pending(app: &mut VectorcraftApp, pending: PendingOpen, scope: ScopeHandle) -> Result<Value, String> {
    let name = pending.file.name();
    let r = open_now(app, pending, scope);
    if let Err(e) = &r {
        crate::io::report_open_error(app, &name, e);
    }
    r
}

/// Open `pending` under `scope` without reporting a failure. A file read off the UI thread with part
/// of its read left ([`ReadFile::read_left`]) does that part on a job of its own
/// ([`crate::opening::finish_answered`]) → `{background: true, status}`.
fn open_now(app: &mut VectorcraftApp, pending: PendingOpen, scope: ScopeHandle) -> Result<Value, String> {
    let PendingOpen { file, path, background, .. } = pending;
    if let Some(p) = path.as_deref().filter(|_| background && file.read_left() && crate::opening::available()) {
        return Ok(crate::opening::finish_answered(app, p, file, scope));
    }
    scope.run(|| {
        let r = fileio::open_read(&mut app.session, file).map_err(|e| e.to_string())?;
        crate::io::opened(app, &r);
        if let Some(p) = &path {
            crate::io::note_recent(app, p);
        }
        Ok(r)
    })
}

/// Cancel, or the dialog closed some other way: the waiting file doesn't open. The next file waiting
/// for its question asks it once no dialog is open ([`pump`]).
pub(crate) fn cancel_open(app: &mut VectorcraftApp) {
    app.pending_profile_open = None;
}

/// Before running paste `command` (or `edit.dropSelection`, a drag from another document) with
/// `params`: when Ask When Pasting is on, `params` don't say what to do (`profileMismatch`) and the
/// pasted or dropped colors come from another profile, open the dialog instead and answer
/// `{dialog}`.
pub fn ask_paste(app: &mut VectorcraftApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    // A null choice is no choice, as the paste commands read it.
    if params.get("profileMismatch").is_some_and(|v| !v.is_null()) || !cms::working_settings().ask_mismatch_paste {
        return None;
    }
    let mismatches = paste_mismatches(app, command, params).filter(|a| !a.is_empty())?;
    open_paste(app, command, params, mismatches)
}

/// Open the paste question about `mismatches` for `command` with `params`; `None` when there is no
/// window to ask in (the paste then runs as the policy says, as when the box is off). One question
/// is open at a time: the host replaces an open one, and a deferred paste it asked about gives way.
fn open_paste(app: &mut VectorcraftApp, command: &str, params: &Value, mismatches: Vec<Value>) -> Option<Result<Value, String>> {
    let ctx = app.ui_ctx.clone()?;
    let dialogs = app.dialogs()?;
    if app.pending_paste.as_ref().is_some_and(|ask| ask.deferred.is_some()) {
        crate::placing::cancel_swatch_paste(app);
    }
    let dialog = paste_dialog(&ctx, &mismatches);
    let drawing = app.session.active().map(|st| (st.uid, st.reverts));
    app.pending_paste = Some(PasteAsk { command: command.to_string(), params: params.clone(), mismatches, drawing, deferred: None });
    dialogs.with(|h| h.open(dialog));
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

/// The choice mismatch `m` starts on: its model's policy.
fn default_choice(m: &Value) -> &str {
    m["default"].as_str().unwrap_or("preserveNumbers")
}

/// The choice the paste question starts on: the model's policy for one model, and for both a
/// choice for each (`{rgb, cmyk}`), each starting on its model's policy.
fn first_choice(mismatches: &[Value]) -> Value {
    match mismatches {
        [m] => json!(default_choice(m)),
        _ => Value::Object(mismatches.iter().filter_map(|m| Some((m["model"].as_str()?.to_string(), json!(default_choice(m))))).collect()),
    }
}

/// What `choice` (one choice for every model, or `{rgb?, cmyk?}`) picks for the colors of `model`.
fn picked<'a>(choice: &'a Value, model: &str) -> &'a str {
    choice.as_str().or_else(|| choice.get(model).and_then(Value::as_str)).unwrap_or_default()
}

/// The paste question about `mismatches`: a line and a pair of choices for each model, each
/// starting on its model's policy. Colors whose profile is not available can only keep their numbers.
fn paste_dialog(ctx: &egui::Context, mismatches: &[Value]) -> HostedParamDialog {
    let start = first_choice(mismatches);
    let options: Vec<(String, Label)> = PASTE_CHOICES.iter().map(|(value, text)| (value.to_string(), Label::from(tl!(*text)))).collect();
    let mut params = Vec::new();
    for m in mismatches {
        let id = m["model"].as_str().unwrap_or_default();
        let model = id.to_ascii_uppercase();
        let (from, to) = (m["clipboard"].as_str().unwrap_or_default(), m["document"].as_str().unwrap_or_default());
        let text = tl!("The pasted colors come from a document whose {model} profile is “{from}”; this document's is “{to}”.");
        let note = crate::i18n::fmt(text, &[("model", model.as_str()), ("from", from), ("to", to)]);
        let blocked: Vec<&str> = PASTE_CHOICES.iter().map(|(value, _)| *value).filter(|value| !allowed(m, value)).collect();
        let kind = ParamKind::radio(options.clone(), Some(picked(&start, id).to_string()));
        params.push(Param::read_only_text(format!("{id}Note"), model.clone(), note));
        params.push(Param::new(id, model, kind).disabled_options(blocked));
    }
    HostedParamDialog::new(ctx, PASTE_KIND, DialogFields::new(), move |fields| {
        ParamDialog::new(PASTE_KIND, tl!("Paste Profile Mismatch"), params.clone()).values(Bind::bound(fields))
    })
}

/// What the closed paste question `id` answered, once: `Some(Some(choice))` for OK, `Some(None)` for
/// Cancel, `None` while it is open (a replaced question keeps its `Cancel` under the same id until
/// the new one answers) or when no paste waits.
fn paste_answer(app: &VectorcraftApp, id: DialogId) -> Option<Option<Value>> {
    let ask = app.pending_paste.as_ref()?;
    app.dialogs()?.with(|h| {
        if h.get_dyn(id).is_some() {
            return None;
        }
        h.take_by_id::<HostedParamDialog>(id).map(|result| result.ok().map(|result| chosen(&result.fields, &ask.mismatches)))
    })
}

/// The `profileMismatch` value the dialog's fields give: one choice for one model, `{rgb, cmyk}`
/// for both.
fn chosen(fields: &DialogFields, mismatches: &[Value]) -> Value {
    let pick = |m: &Value| fields.text(m["model"].as_str().unwrap_or_default()).unwrap_or_else(|| default_choice(m)).to_string();
    match mismatches {
        [m] => json!(pick(m)),
        _ => Value::Object(mismatches.iter().filter_map(|m| Some((m["model"].as_str()?.to_string(), json!(pick(m))))).collect()),
    }
}

/// `params` of a paste with the answer `choice` as its `profileMismatch`.
fn with_choice(params: &Value, choice: Value) -> Value {
    let mut params = if params.is_object() { params.clone() } else { json!({}) };
    params["profileMismatch"] = choice;
    params
}

/// Run the paste whose question closed, unless it waits for placing (a deferred one).
fn pump_paste(app: &mut VectorcraftApp) {
    if app.pending_paste.as_ref().is_none_or(|ask| ask.deferred.is_some()) {
        return;
    }
    let Some(answer) = paste_answer(app, dialog_id(PASTE_KIND)) else { return };
    let Some(ask) = app.pending_paste.take() else { return };
    if let Some(choice) = answer
        && let Err(e) = paste_confirm(app, ask, choice)
    {
        app.status(e);
    }
}

/// OK: run the paste with the choice; any Swatch Conflict question comes next. An immediate paste
/// is canceled if its drawing is inactive or reverted. When the profiles it would bring have changed
/// since it asked (the clipboard, dragged objects or destination changed), it asks about the new ones.
fn paste_confirm(app: &mut VectorcraftApp, ask: PasteAsk, choice: Value) -> Result<Value, String> {
    if ask.deferred.is_none() && ask.drawing != app.session.active().map(|st| (st.uid, st.reverts)) {
        return Ok(json!({}));
    }
    let params = with_choice(&ask.params, choice);
    // Without the answer, the paste asks again about what it brings now.
    let changed = |app: &mut VectorcraftApp, command: &str, params: &Value| paste_changed(app, command, params, &ask.mismatches);
    let recheck = crate::placing::Recheck { key: "profileMismatch", changed: &changed };
    crate::placing::confirm_swatch_paste(app, &ask.command, params, ask.deferred, Some(&recheck))
}

/// Do the profiles paste `command` with `params` would bring differ from `asked`, the ones the
/// question was about? A paste that no longer brings any asks nothing.
pub(crate) fn paste_changed(app: &mut VectorcraftApp, command: &str, params: &Value, asked: &[Value]) -> bool {
    paste_mismatches(app, command, params).is_some_and(|now| !now.is_empty() && now != asked)
}

/// The paste question's dialog, while a paste waits for it.
pub(crate) fn paste_id(app: &VectorcraftApp) -> Option<DialogId> {
    app.pending_paste.as_ref().map(|_| dialog_id(PASTE_KIND))
}

/// The paste the question asks about waits for placing (its clipboard is being read) on `chain`.
pub(crate) fn defer_paste(app: &mut VectorcraftApp, chain: u64) {
    if let Some(ask) = &mut app.pending_paste {
        ask.deferred = Some(chain);
    }
}

/// What the question `id` a deferred paste waits on answered: `Answered({command, params, deferred,
/// asked})` with the choice in `params`, `Cancelled`, or `Open` while it is still asked.
pub(crate) fn take_paste(app: &mut VectorcraftApp, id: DialogId) -> Taken {
    let Some(answer) = paste_answer(app, id) else { return Taken::Open };
    let Some(ask) = app.pending_paste.take() else { return Taken::Cancelled };
    let Some(choice) = answer else { return Taken::Cancelled };
    let params = with_choice(&ask.params, choice);
    Taken::Answered(json!({ "command": ask.command, "params": params, "deferred": ask.deferred, "asked": ask.mismatches }))
}

#[cfg(test)]
mod tests;
