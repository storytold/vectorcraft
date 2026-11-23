//! What waits on a file dialog. The host's file service shows the picker and queues requests in
//! order; an ask files the continuation that handles the answer, and [`poll`] runs it once the
//! answer arrives, on the document it was asked for and under the access it was asked with.
// PLAN: this crate needs `craft-host` and `craft-dialogs` in its Cargo.toml (hub-w1 owns the manifest).

use std::path::PathBuf;

use craft::dialogs::ui::UnavailableWords;
use craft::host::{FileFilter, Host, PickId, PickKind, PickRequest, Picked};
use craft::ui::dialog::{DialogId, DialogResult, PromptChoice, StandardPrompt};
// PLAN: assumed re-export paths for `HostedParamDialog`, `PathRequest` and `Env`.
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::requests::PathRequest;
use serde_json::{Map, Value, json};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;
use crate::dialogs::confirm::{ReplaceFiles, ReplaceFilesPrompt};

type Then = Box<dyn FnOnce(&mut VectorcraftApp, Picked) -> Result<Value, String>>;
/// What proceeding past a replace question runs.
type Proceed = Box<dyn FnOnce(&mut VectorcraftApp) -> Result<Value, String>>;

/// Whose answer a continuation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The active document when it was asked: dropped with [`closed`] when it has closed, made
    /// active before the continuation runs.
    Document,
    /// Acts on no document (an app preference, a library, a shortcut set): it goes on whatever is open.
    Free,
}

/// A pick asked for and not yet answered.
struct Waiting {
    ticket: PickId,
    owner: Owner,
    /// The active document's uid when asked (`DocState::uid`); `None` for [`Owner::Free`].
    doc: Option<u64>,
    /// The automation roots in force when asked: the continuation runs under them, not under the
    /// desktop's access of the frame the answer comes in.
    scope: ScopeHandle,
    /// Automation asked ([`VectorcraftApp::automation_asked`]): the continuation runs as its.
    automated: bool,
    then: Then,
}

/// A replace question on the dialog host, and what proceeding runs.
struct Replacing {
    dialog: DialogId,
    then: Proceed,
}

/// The app's pending file asks ([`VectorcraftApp`]'s `file_asks`).
#[derive(Default)]
pub(crate) struct FileAsks {
    waiting: Vec<Waiting>,
    replacing: Vec<Replacing>,
    /// Numbers the replace questions so each is its own dialog.
    next_question: u64,
    automated: bool,
}

impl FileAsks {
    /// What runs now handles the answer of a file dialog automation opened ([`as_automated`]).
    pub(crate) fn automated(&self) -> bool {
        self.automated
    }

    /// A file dialog or a replace question is waiting to answer.
    pub(crate) fn asking(&self) -> bool {
        !self.waiting.is_empty() || !self.replacing.is_empty()
    }
}

// PLAN: callers use the function form of `BUSY` and `CLOSED`; `tl!` is a runtime lookup, so they are not consts.
/// What the status bar says when a file dialog is open and the app must not go on.
pub(crate) fn busy() -> &'static str {
    tl!("A file dialog is open: choose there first")
}

/// What the status bar says when the document a file dialog was asked for closed before it answered.
pub(crate) fn closed() -> &'static str {
    tl!("The document was closed while its file dialog was open")
}

/// Run `f` as the continuation of a dialog automation opened (`automated`), or as a person's.
pub(crate) fn as_automated<R>(app: &mut VectorcraftApp, automated: bool, f: impl FnOnce(&mut VectorcraftApp) -> R) -> R {
    let before = std::mem::replace(&mut app.file_asks.automated, automated);
    let r = f(app);
    app.file_asks.automated = before;
    r
}

/// Whether a dialog for `kind` can open: every kind on the desktop, only the opening ones on the web.
pub(crate) fn can(app: &VectorcraftApp, kind: PickKind) -> bool {
    matches!(kind, PickKind::Open | PickKind::OpenMany) || !crate::io::is_web(app)
}

/// Show `request` and run `then` with the answer. Returns `{pending: "fileDialog"}`, or `Null` when
/// no file service is there to ask.
pub(crate) fn ask(
    app: &mut VectorcraftApp,
    request: PickRequest,
    owner: Owner,
    then: impl FnOnce(&mut VectorcraftApp, Picked) -> Result<Value, String> + 'static,
) -> Value {
    let Some(host) = app.ui_ctx.as_ref().and_then(Host::try_of_ctx) else { return Value::Null };
    let ticket = host.files().pick(request);
    let doc = match owner {
        Owner::Document => app.session.active().map(|d| d.uid),
        Owner::Free => None,
    };
    let (scope, automated) = (ScopeHandle::capture(), app.automation_asked());
    app.file_asks.waiting.push(Waiting { ticket, owner, doc, scope, automated, then: Box::new(then) });
    json!({ "pending": "fileDialog" })
}

/// Ask whether to replace the file `path`, which the save dialog did not ask about (it answered
/// `answered`); `then` runs when the user proceeds. Returns `{pending: "replaceFile"}`.
pub(crate) fn ask_replace(
    app: &mut VectorcraftApp,
    path: &str,
    answered: &str,
    then: impl FnOnce(&mut VectorcraftApp) -> Result<Value, String> + 'static,
) -> Value {
    let Some(dialogs) = app.dialogs() else { return Value::Null };
    let name = |p: &str| std::path::Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().into_owned());
    app.file_asks.next_question += 1;
    let prompt = StandardPrompt::replace_file(name(path), Some(name(answered))).instance(("vectorcraft.replaceFile", app.file_asks.next_question));
    let dialog = dialogs.with(|host| host.open(prompt));
    app.file_asks.replacing.push(Replacing { dialog, then: Box::new(then) });
    json!({ "pending": "replaceFile" })
}

/// A parameter dialog's Browse button: show `pick` for `request` and give the dialog the path with
/// `then` ([`answer_path`] for a plain answer). A cancelled or unavailable dialog ends the request;
/// nothing happens once the dialog has closed.
pub(crate) fn browse(
    app: &mut VectorcraftApp,
    dialog: DialogId,
    request: PathRequest,
    pick: PickRequest,
    then: impl FnOnce(&mut VectorcraftApp, DialogId, String, PathBuf) + 'static,
) -> Value {
    let key = request.key;
    ask(app, pick, Owner::Free, move |app, picked| {
        let Some(dialogs) = app.dialogs() else { return Ok(Value::Null) };
        if dialogs.with(|host| host.get_by_id::<HostedParamDialog>(dialog).is_none()) {
            return Ok(Value::Null);
        }
        let path = match &picked {
            Picked::Paths(_) | Picked::Saved { .. } => picked.first_path(),
            _ => None,
        };
        match path {
            Some(path) => then(app, dialog, key, path),
            None => dialogs.with(|host| {
                if let Some(d) = host.get_mut_by_id::<HostedParamDialog>(dialog) {
                    d.cancel_requests();
                }
            }),
        }
        Ok(Value::Null)
    })
}

/// Give the open dialog `dialog` the path picked for its row `key`.
pub(crate) fn answer_path(app: &mut VectorcraftApp, dialog: DialogId, key: String, path: PathBuf) {
    let Some(dialogs) = app.dialogs() else { return };
    dialogs.with(|host| {
        if let Some(d) = host.get_mut_by_id::<HostedParamDialog>(dialog) {
            d.answer_path(key, path);
        }
    });
}

/// The file types of `list`, `(label, extensions)` as the engine lists them, as a picker offers them.
pub(crate) fn filters(list: &[(&str, &[&str])]) -> Vec<FileFilter> {
    list.iter().map(|&(name, extensions)| FileFilter::new(name, extensions)).collect()
}

/// The first file of an Open answer as the params of an open command: `path`, or the bytes with the
/// `name` the browser gave. `None` for any other answer.
pub(crate) fn open_params(picked: &Picked) -> Option<Map<String, Value>> {
    let mut params = Map::new();
    match picked {
        Picked::Paths(paths) => {
            params.insert("path".into(), json!(paths.first()?.to_string_lossy()));
        }
        Picked::Contents(files) => {
            let (name, bytes) = files.first()?;
            params.insert("name".into(), json!(name));
            params.insert("dataBase64".into(), json!(vectorcraft_svg::base64_encode(bytes)));
        }
        _ => return None,
    }
    Some(params)
}

/// Make the document asked for (`doc`, by uid) active before its continuation runs: another may be
/// active by now, or it may be gone → false when it can't run (the status bar says why).
fn on_document(app: &mut VectorcraftApp, doc: Option<u64>) -> bool {
    let Some(uid) = doc else { return true };
    let Some(i) = app.session.documents().iter().position(|d| d.uid == uid) else {
        app.status(closed());
        return false;
    };
    if app.session.active_index() != Some(i)
        && let Err(e) = app.run("document.activate", json!({ "index": i }))
    {
        app.status(e);
        return false;
    }
    true
}

/// An `Err` other than "cancelled" from a continuation goes to the status line.
fn report(app: &mut VectorcraftApp, result: Result<Value, String>) {
    if let Err(e) = result
        && e != "cancelled"
    {
        app.status(e);
    }
}

/// Once per frame, before the save flow's pump: run the continuations of the answered picks.
pub(crate) fn poll(app: &mut VectorcraftApp) {
    app.file_asks.automated = false;
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let Some(host) = Host::try_of_ctx(&ctx) else { return };
    let mut answered = Vec::new();
    // Taken first: a continuation may ask again.
    for w in std::mem::take(&mut app.file_asks.waiting) {
        match host.files().take_answer(w.ticket) {
            Some(picked) => answered.push((w, picked)),
            None => app.file_asks.waiting.push(w),
        }
    }
    for (w, picked) in answered {
        match &picked {
            Picked::Cancelled => continue,
            Picked::Unavailable(why) => {
                app.status(Env::of_root(&ctx).words::<UnavailableWords>().sentence(why));
                continue;
            }
            _ => {}
        }
        if w.owner == Owner::Document && !on_document(app, w.doc) {
            continue;
        }
        let Waiting { scope, automated, then, .. } = w;
        let result = scope.run(|| as_automated(app, automated, |app| then(app, picked)));
        report(app, result);
    }
}

/// Once per frame on the dialog host: run what the answered replace questions allow, and the
/// replace-files question's command.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let mut proceed = Vec::new();
    let mut open = Vec::new();
    let previous = std::mem::take(&mut app.file_asks.replacing);
    let files = dialogs.with(|host| {
        for r in previous {
            match host.take_by_id::<StandardPrompt>(r.dialog) {
                Some(DialogResult::Ok(answer)) if answer.choice == PromptChoice::Proceed => proceed.push(r.then),
                Some(_) => {}
                None if host.get_by_id::<StandardPrompt>(r.dialog).is_some() => open.push(r),
                // Neither open nor holding a result: closed from outside.
                None => {}
            }
        }
        host.take::<ReplaceFilesPrompt>()
    });
    app.file_asks.replacing.extend(open);
    for then in proceed {
        let result = then(app);
        report(app, result);
    }
    if let Some(DialogResult::Ok(mut answer)) = files
        && answer.choice == PromptChoice::Proceed
        && let Some(r) = answer.take_tag::<ReplaceFiles>()
        && on_document(app, Some(r.doc))
    {
        let all: Vec<String> = r.files.iter().chain(&r.allowed).cloned().collect();
        crate::io::save::allow_replacing(app, &all);
        let result = app.run(&r.command, r.params);
        report(app, result);
    }
}
