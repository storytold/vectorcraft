//! What waits on a file dialog. The host's file service shows the picker and queues requests in
//! order; an ask files the continuation that handles the answer, and [`poll`] runs it once the
//! answer arrives, on the document it was asked for and under the access it was asked with.
// PLAN: this crate needs `craft-host` and `craft-dialogs` in its Cargo.toml (hub-w1 owns the manifest).

use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use craft::dialogs::ui::UnavailableWords;
use craft::host::{FileFilter, Files, Host, PickId, PickKind, PickRequest, Picked};
use craft::ui::dialog::{DialogId, DialogResult, PromptChoice, StandardPrompt};
// PLAN: assumed re-export paths for `HostedParamDialog`, `PathRequest` and `Env`.
use craft::ui::patterns::form_dialogs::HostedParamDialog;
#[cfg(not(target_arch = "wasm32"))]
use craft::ui::runtime::env::Env;
use craft::ui::runtime::requests::PathRequest;
use serde_json::{Map, Value, json};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;
use crate::dialogs::confirm::{ReplaceFiles, ReplaceFilesPrompt};

type Then = Box<dyn FnOnce(&mut VectorcraftApp, Picked) -> Result<Value, String>>;
/// Whether the part of the app that requested a pick is still waiting for it.
type PickGuard = Box<dyn FnOnce(&VectorcraftApp) -> bool>;
/// What proceeding past a replace question runs.
type Proceed = Box<dyn FnOnce(&mut VectorcraftApp) -> Result<Value, String>>;

/// Whose answer a continuation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The active document when asked, made active before the continuation runs. Closing or
    /// reverting it retires the request.
    Document,
    /// Acts on no document (an app preference, a library, a shortcut set): it goes on whatever is open.
    Free,
}

/// A pick asked for and not yet answered.
struct Waiting {
    ticket: PickId,
    /// The active drawing's UID and Revert counter when asked, if the request is document-owned.
    doc: Option<(u64, u64)>,
    /// The automation roots in force when asked: the continuation runs under them, not under the
    /// desktop's access of the frame the answer comes in.
    scope: ScopeHandle,
    /// Automation asked ([`VectorcraftApp::automation_asked`]): the continuation runs as its.
    automated: bool,
    current: Option<PickGuard>,
    then: Then,
}

/// A replace question on the dialog host, and what proceeding runs.
struct Replacing {
    dialog: DialogId,
    /// The automation roots in force when asked, as [`Waiting::scope`].
    scope: ScopeHandle,
    /// Automation asked, as [`Waiting::automated`].
    automated: bool,
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

    /// Whether the newest open replace question goes on confined to automation roots, and whether
    /// as automation.
    #[cfg(test)]
    pub(crate) fn replace_access(&self) -> Option<(bool, bool)> {
        let r = self.replacing.last()?;
        Some((r.scope.clone().run(vectorcraft_engine::file_access::current).is_some(), r.automated))
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
    start(app, owner, |files| files.pick(request), None, then)
}

/// Like [`ask`], but drops a reply if its requester is gone, before choosing a document or reporting a picker error.
pub(crate) fn ask_while(
    app: &mut VectorcraftApp,
    request: PickRequest,
    owner: Owner,
    current: impl FnOnce(&VectorcraftApp) -> bool + 'static,
    then: impl FnOnce(&mut VectorcraftApp, Picked) -> Result<Value, String> + 'static,
) -> Value {
    start(app, owner, |files| files.pick(request), Some(Box::new(current)), then)
}

/// Save `bytes` as the file service saves them (`Files::save`: the desktop asks where with `request` and
/// writes the file, the browser opens its save picker or downloads them) and run `then` with the answer.
/// Returns as [`ask`] does.
pub(crate) fn save(
    app: &mut VectorcraftApp,
    request: PickRequest,
    bytes: Vec<u8>,
    owner: Owner,
    then: impl FnOnce(&mut VectorcraftApp, Picked) -> Result<Value, String> + 'static,
) -> Value {
    start(app, owner, |files| files.save(request, bytes), None, then)
}

/// Asks the file service with `ask` and files `then` for its answer.
fn start(
    app: &mut VectorcraftApp,
    owner: Owner,
    ask: impl FnOnce(&dyn Files) -> PickId,
    current: Option<PickGuard>,
    then: impl FnOnce(&mut VectorcraftApp, Picked) -> Result<Value, String> + 'static,
) -> Value {
    let Some(host) = app.ui_ctx.as_ref().and_then(Host::try_of_ctx) else { return Value::Null };
    let ticket = ask(host.files());
    let doc = match owner {
        Owner::Document => app.session.active().map(|d| (d.uid, d.reverts)),
        Owner::Free => None,
    };
    let (scope, automated) = (ScopeHandle::capture(), app.automation_asked());
    app.file_asks.waiting.push(Waiting { ticket, doc, scope, automated, current, then: Box::new(then) });
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
    let (scope, automated) = (ScopeHandle::capture(), app.automation_asked());
    app.file_asks.replacing.push(Replacing { dialog, scope, automated, then: Box::new(then) });
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

/// The file types of `list`, `(label, extensions)` as the engine lists them, as a picker offers them,
/// named in the interface language.
pub(crate) fn filters(list: &[(&str, &[&str])]) -> Vec<FileFilter> {
    filters_in(crate::i18n::current(), list)
}

/// [`filters`] with the names in `lang`: a label's `file-filter` row, else its plain row, else the
/// English label.
pub(crate) fn filters_in(lang: crate::i18n::Lang, list: &[(&str, &[&str])]) -> Vec<FileFilter> {
    list.iter().map(|&(name, extensions)| FileFilter::new(filter_name(lang, name), extensions)).collect()
}

/// The name a picker shows for the file type labeled `name` in `lang`.
pub(crate) fn filter_name(lang: crate::i18n::Lang, name: &str) -> &str {
    crate::i18n::tr_ctx(lang, "file-filter", name)
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

/// Make the original drawing active before running a continuation. The status bar explains why
/// a closed drawing or a different loaded generation cannot receive the result.
fn on_document(app: &mut VectorcraftApp, uid: u64, generation: Option<u64>) -> bool {
    back_to(app, uid, generation, closed(), tl!("The drawing was reverted while its file dialog was open"))
}

/// Makes drawing `uid` active again, at its revert count `generation` when one is given. When the
/// drawing is closed, or was reverted since, the status bar shows `closed` or `reverted` and the
/// answer is false.
pub(crate) fn back_to(app: &mut VectorcraftApp, uid: u64, generation: Option<u64>, closed: &str, reverted: &str) -> bool {
    let Some((i, state)) = app.session.documents().iter().enumerate().find(|(_, d)| d.uid == uid) else {
        app.status(closed);
        return false;
    };
    if generation.is_some_and(|reverts| state.reverts != reverts) {
        app.status(reverted);
        return false;
    }
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
        if w.current.is_some_and(|current| !current(app)) {
            continue;
        }
        match &picked {
            Picked::Cancelled => continue,
            Picked::Unavailable(why) => {
                // The desktop says what gives it a file dialog; the browser has none for this kind of pick.
                #[cfg(not(target_arch = "wasm32"))]
                app.status(Env::of_root(&ctx).words::<UnavailableWords>().sentence(why));
                #[cfg(target_arch = "wasm32")]
                app.status(why.clone());
                continue;
            }
            _ => {}
        }
        if w.doc.is_some_and(|(uid, reverts)| !on_document(app, uid, Some(reverts))) {
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
                Some(DialogResult::Ok(answer)) if answer.choice == PromptChoice::Proceed => proceed.push(r),
                Some(_) => {}
                None if host.get_by_id::<StandardPrompt>(r.dialog).is_some() => open.push(r),
                // Neither open nor holding a result: closed from outside.
                None => {}
            }
        }
        host.take::<ReplaceFilesPrompt>()
    });
    app.file_asks.replacing.extend(open);
    for Replacing { scope, automated, then, .. } in proceed {
        let result = scope.run(|| as_automated(app, automated, then));
        report(app, result);
    }
    if let Some(DialogResult::Ok(mut answer)) = files
        && answer.choice == PromptChoice::Proceed
        && let Some(r) = answer.take_tag::<ReplaceFiles>()
        && on_document(app, r.doc, None)
    {
        let all: Vec<String> = r.files.iter().chain(&r.allowed).cloned().collect();
        crate::io::save::allow_replacing(app, &all);
        let ReplaceFiles { scope, automated, command, params, .. } = r;
        let result = scope.run(|| as_automated(app, automated, |app| app.run(&command, params)));
        report(app, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn an_svg_picker_returns_to_its_opening_tab_only_if_that_drawing_was_not_reverted() {
        const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="Shadow"><feOffset dx="2"/></filter></svg>"#;
        for revert in [true, false] {
            let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
            let picker = crate::tests::picks::scripted(&mut app);
            app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
            app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
            let dir = vectorcraft_testkit::TempDir::new("svg-picker-original-drawing");
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            app.run("file.saveAs", json!({"path": dir.join("filters.vectorcraft")})).unwrap();
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
            let original = app.session.active().unwrap();
            let version = (original.uid, original.reverts);
            let pending = app.run("ui.importSvgFilter", json!({})).unwrap();
            assert_eq!(pending["pending"], json!("fileDialog"));
            if revert {
                app.run("file.revert", json!({"confirmed": true})).unwrap();
                let doc = app.session.active().unwrap();
                assert_eq!(doc.uid, version.0);
                assert_ne!(doc.reverts, version.1);
            }
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            picker.answer_contents(&[("shadow.svg", SVG)]);
            poll(&mut app);
            let docs = app.session.documents();
            assert!(docs[1].doc.svg_filters.is_empty());
            if revert {
                assert!(docs[0].doc.svg_filters.is_empty());
                assert_eq!(app.session.active_index(), Some(1));
            } else {
                assert_eq!(docs[0].doc.svg_filters[0].name, "Shadow");
                assert_eq!(app.session.active_index(), Some(0));
            }
        }
    }
}
