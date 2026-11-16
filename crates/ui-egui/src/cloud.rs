//! VectorCraft's half of Open and Save against cloud storage.
//!
//! The contract types here ([`CloudBackend`], [`Job`], [`Event`], [`Post`], [`Fault`],
//! [`Account`]) are kept in the same shape as `crates/ui/src/cloud/backend.rs` on
//! wdunn001/craft-libs, which is where storytold/craft-libs#160 is adding the same contract to
//! `craft-ui` itself. They are not pulled from that crate directly: craft-ui's own `cloud`
//! feature exists today only on that fork, pinned to egui 0.36, a release older than the one this
//! workspace already builds against (egui 0.37, from `craft-update`). Once #160 merges, craft-ui
//! carries this contract at the egui release this workspace uses, and this module is replaced by
//! `pub use craft_ui::cloud::{...}` in one commit alongside the `craft-ui` / `craft-cloud` pin
//! bump in the workspace `Cargo.toml`.
//!
//! VectorCraft holds no provider credential and speaks no provider protocol. A [`CloudBackend`]
//! is the one thing a host application supplies (see `apps/vectorcraft` / `apps/vectorcraft-web`);
//! this crate never learns what it reaches. Nothing here opens a socket, so the pages stay
//! buildable for the web and testable with no service running.
//!
//! With no backend connected, [`available`] is `false` and the cloud menu items, the Open / Save
//! page and the account badge are not drawn anywhere in this crate (see `menus/tree.rs` and
//! `chrome.rs`).
//!
//! # Scope
//!
//! Only opening a drawing and saving one under a new name round-trip through here. The fuller
//! contract #160 is adding also covers an overwrite save with a refused-on-conflict write, reload
//! discarding local edits, what other people shared with you, linking an account, and the Share
//! dialog's collaboration room. Each of those needs the real `CloudBackend`/`ShareBackend` pair to
//! behave correctly (a save that can silently clobber somebody else's edit is worse than no save
//! button), and a hand-maintained stand-in for all of it would be a second implementation of
//! #160's own job. They are unreachable without a connected backend already, so cutting them here
//! removes no behavior a user could reach; Open and Save As are added back once #160 lands and
//! this module is replaced by the real contract.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

pub use craft_cloud::{Capability, Entry, EntryKind};

use crate::VectorcraftApp;

/// Work for a host's worker. Each one answers with one [`Event`].
pub enum Job {
    /// Who is signed in, and what the provider behind the service can do.
    Account,
    /// One folder's contents. An empty path is the user's root.
    List(String),
    /// A document's bytes.
    Read { entry: Entry },
    /// Save under a new name in `folder`.
    Create { folder: String, name: String, bytes: Vec<u8> },
}

/// What a worker reports back.
pub enum Event {
    Account(Result<Account, Fault>),
    Listed { path: String, entries: Result<Vec<Entry>, Fault> },
    Read { entry: Entry, bytes: Result<Vec<u8>, Fault> },
    Created(Result<Entry, Fault>),
}

/// Where a worker's answers go. Handed to [`CloudBackend::submit`] with the job it belongs to.
pub struct Post {
    tx: Sender<Event>,
}

impl Post {
    fn new(tx: Sender<Event>) -> Post {
        Post { tx }
    }

    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}

/// Somewhere a [`Job`] can run without blocking a frame.
///
/// `submit` must return immediately. The answer arrives through `post` whenever it is ready, or
/// never; the pages survive either.
pub trait CloudBackend: Send + Sync {
    fn submit(&self, job: Job, post: Post);
}

/// What the service said about the signed-in user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub display: String,
    pub capabilities: Vec<Capability>,
}

/// A call that did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    Unreachable(String),
    Refused(String),
}

impl Fault {
    /// One sentence for a person to read.
    pub fn text(&self) -> String {
        match self {
            Fault::Unreachable(detail) => format!("The cloud service could not be reached. {detail}"),
            Fault::Refused(detail) => detail.clone(),
        }
    }
}

/// One folder's contents, as last fetched.
#[derive(Clone, Default)]
pub struct Folder {
    pub path: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<Fault>,
}

/// Open and Save against cloud storage. Inert until the host connects a backend.
pub struct CloudFiles {
    backend: Option<Arc<dyn CloudBackend>>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    pub account: Option<Result<Account, Fault>>,
    pub mine: Folder,
    /// The name typed into the Save As page.
    pub save_name: String,
}

impl Default for CloudFiles {
    fn default() -> CloudFiles {
        let (tx, rx) = std::sync::mpsc::channel();
        CloudFiles { backend: None, tx, rx, account: None, mine: Folder::default(), save_name: String::new() }
    }
}

impl CloudFiles {
    pub fn new() -> CloudFiles {
        CloudFiles::default()
    }

    pub fn connect(&mut self, backend: impl CloudBackend + 'static) {
        self.backend = Some(Arc::new(backend));
    }
}

pub const UNAVAILABLE: &str = "No cloud service is configured.";

/// Whether a backend is connected at all.
pub fn available(app: &VectorcraftApp) -> bool {
    app.cloud.backend.is_some()
}

fn submit(app: &VectorcraftApp, job: Job) {
    if let Some(backend) = &app.cloud.backend {
        backend.submit(job, Post::new(app.cloud.tx.clone()));
    }
}

/// Which cloud page the File menu opened, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudPage {
    /// File ▸ Open from the Cloud…, for `.vectorcraft` drawings.
    Open,
    /// File ▸ Save to the Cloud…, which writes the drawing.
    SaveAs,
}

/// Ensures the account and the current folder are loaded.
pub fn start(app: &mut VectorcraftApp) {
    if !available(app) {
        return;
    }
    if app.cloud.account.is_none() {
        submit(app, Job::Account);
    }
    if !app.cloud.mine.loaded && !app.cloud.mine.loading {
        let path = app.cloud.mine.path.clone();
        browse(app, &path);
    }
}

/// List a folder.
pub fn browse(app: &mut VectorcraftApp, path: &str) {
    if !available(app) {
        return;
    }
    app.cloud.mine.path = path.to_owned();
    app.cloud.mine.loading = true;
    app.cloud.mine.loaded = false;
    app.cloud.mine.error = None;
    submit(app, Job::List(path.to_owned()));
}

/// Open or browse into a cloud entry, from the Open page.
fn open_entry(app: &mut VectorcraftApp, entry: Entry) {
    if entry.kind == EntryKind::Folder {
        let path = entry.path.clone();
        browse(app, &path);
        return;
    }
    submit(app, Job::Read { entry });
}

/// Save the active drawing under `name` in `folder`.
pub fn save_as(app: &mut VectorcraftApp, folder: &str, name: &str) -> Result<(), String> {
    if !available(app) {
        return Err(UNAVAILABLE.to_owned());
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        return Err("no drawing open".to_owned());
    };
    let bytes = vectorcraft_engine::cmd::fileio::encode(&doc, "vectorcraft", &serde_json::json!({})).map_err(|e| e.to_string())?;
    let name = drawing_name(name);
    submit(app, Job::Create { folder: folder.to_owned(), name, bytes });
    Ok(())
}

/// A typed name as a drawing file name: folder separators dropped, the native extension appended
/// if missing.
fn drawing_name(typed: &str) -> String {
    let leaf = typed.rsplit(['/', '\\']).next().unwrap_or(typed).trim();
    let leaf = if leaf.is_empty() { "drawing" } else { leaf };
    let suffix = format!(".{}", vectorcraft_format::EXTENSION);
    if leaf.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase()) { leaf.to_owned() } else { format!("{leaf}{suffix}") }
}

/// Drain the backend's answers and apply them. Called once a frame.
pub fn pump(app: &mut VectorcraftApp) {
    let events: Vec<Event> = app.cloud.rx.try_iter().collect();
    for event in events {
        match event {
            Event::Account(answer) => app.cloud.account = Some(answer),
            Event::Listed { path, entries } => {
                if path != app.cloud.mine.path {
                    continue;
                }
                app.cloud.mine.loading = false;
                match entries {
                    Ok(entries) => {
                        app.cloud.mine.entries = entries;
                        app.cloud.mine.loaded = true;
                    }
                    Err(fault) => app.cloud.mine.error = Some(fault),
                }
            }
            Event::Read { entry, bytes } => match bytes {
                Ok(bytes) => match crate::io::open_bytes(app, &entry.name, &bytes, None) {
                    Ok(_) => app.cloud_page = None,
                    Err(e) => app.ui.status = format!("could not open \"{}\" from the cloud: {e}", entry.name),
                },
                Err(fault) => app.ui.status = format!("could not read \"{}\" from the cloud: {}", entry.name, fault.text()),
            },
            Event::Created(answer) => match answer {
                Ok(entry) => {
                    app.ui.status = format!("Saved \"{}\" to the cloud", entry.name);
                    app.cloud_page = None;
                }
                Err(fault) => app.ui.status = format!("could not save to the cloud: {}", fault.text()),
            },
        }
    }
}

/// The account badge: display name once known, a spinner while the first account check is in
/// flight. Hidden entirely by the caller (`chrome.rs`) when [`available`] is `false`.
pub fn who(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    start(app);
    match &app.cloud.account {
        None => {
            ui.add(egui::Spinner::new().size(14.0));
        }
        Some(Ok(account)) => {
            ui.label(egui::RichText::new(&account.display).size(12.5));
        }
        Some(Err(fault)) => {
            ui.label(egui::RichText::new(fault.text()).size(11.0));
        }
    }
}

/// Cloud Open / Save As as a modal over the editor.
pub fn modal(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(page) = app.cloud_page else { return };
    start(app);
    let title = match page {
        CloudPage::Open => tl!("Open from the Cloud"),
        CloudPage::SaveAs => tl!("Save to the Cloud"),
    };
    let mut close = false;
    egui::Modal::new(egui::Id::unique("vectorcraft_cloud_page")).show(ctx, |ui| {
        ui.set_min_width(360.0);
        ui.set_max_width(520.0);
        ui.label(egui::RichText::new(title).strong().size(16.0));
        ui.add_space(10.0);
        let folder = app.cloud.mine.clone();
        let here = if folder.path.is_empty() { tl!("Cloud storage").to_owned() } else { folder.path.clone() };
        ui.label(egui::RichText::new(here).size(12.0));
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if folder.loading {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(tl!("Loading…"));
                });
                return;
            }
            if let Some(fault) = &folder.error {
                ui.label(fault.text());
                return;
            }
            let mut chosen: Option<Entry> = None;
            for entry in &folder.entries {
                let label = if entry.kind == EntryKind::Folder { format!("\u{1f4c1} {}", entry.name) } else { entry.name.clone() };
                if ui.selectable_label(false, label).clicked() {
                    chosen = Some(entry.clone());
                }
            }
            if let Some(entry) = chosen {
                match page {
                    CloudPage::Open => open_entry(app, entry),
                    CloudPage::SaveAs if entry.kind == EntryKind::Folder => browse(app, &entry.path),
                    CloudPage::SaveAs => {}
                }
            }
        });
        if page == CloudPage::SaveAs {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Name:"));
                ui.text_edit_singleline(&mut app.cloud.save_name);
                if ui.button(tl!("Save")).clicked() {
                    let folder = app.cloud.mine.path.clone();
                    let name = app.cloud.save_name.clone();
                    if let Err(e) = save_as(app, &folder, &name) {
                        app.ui.status = e;
                    }
                }
            });
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        });
    });
    if close {
        app.cloud_page = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noop;
    impl CloudBackend for Noop {
        fn submit(&self, _job: Job, _post: Post) {}
    }

    #[test]
    fn unavailable_without_a_connected_backend() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        assert!(!available(&app));
    }

    #[test]
    fn available_once_connected() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.cloud.connect(Noop);
        assert!(available(&app));
    }

    #[test]
    fn save_as_requires_a_connected_backend() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        assert_eq!(save_as(&mut app, "", "demo"), Err(UNAVAILABLE.to_owned()));
    }

    #[test]
    fn drawing_name_appends_the_extension_and_drops_folders() {
        let ext = vectorcraft_format::EXTENSION;
        assert_eq!(drawing_name("demo"), format!("demo.{ext}"));
        assert_eq!(drawing_name(&format!("demo.{ext}")), format!("demo.{ext}"));
        assert_eq!(drawing_name("folder/demo"), format!("demo.{ext}"));
        assert_eq!(drawing_name(""), format!("drawing.{ext}"));
    }

    /// An in-process mock [`CloudBackend`]: answers `Job::List` with a fixed entry list, so the
    /// Open page's listing and selection logic is exercised with no cloud service running.
    struct MockBackend {
        entries: Vec<Entry>,
    }

    fn mock_entry(name: &str) -> Entry {
        Entry {
            id: name.to_owned(),
            room: name.to_owned(),
            name: name.to_owned(),
            path: String::new(),
            kind: EntryKind::File,
            size: Some(0),
            modified: None,
            mime: None,
            etag: None,
            writable: true,
            shared_by: None,
        }
    }

    impl CloudBackend for MockBackend {
        fn submit(&self, job: Job, post: Post) {
            match job {
                Job::Account => post.send(Event::Account(Ok(Account { display: "mock@example.test".into(), capabilities: vec![] }))),
                Job::List(path) => post.send(Event::Listed { path, entries: Ok(self.entries.clone()) }),
                Job::Read { entry } => post.send(Event::Read { entry, bytes: Ok(b"not a real drawing".to_vec()) }),
                Job::Create { name, .. } => post.send(Event::Created(Ok(Entry { etag: Some("1".into()), ..mock_entry(&name) }))),
            }
        }
    }

    #[test]
    fn mock_backend_lists_and_updates_the_folder() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.cloud.connect(MockBackend { entries: vec![mock_entry("demo.vectorcraft")] });
        start(&mut app);
        pump(&mut app);
        assert!(app.cloud.mine.loaded, "the folder should be loaded after one pump");
        assert_eq!(app.cloud.mine.entries.len(), 1);
        assert_eq!(app.cloud.mine.entries[0].name, "demo.vectorcraft");
        assert!(matches!(&app.cloud.account, Some(Ok(a)) if a.display == "mock@example.test"));
    }

    #[test]
    fn mock_backend_save_as_reports_success_and_closes_the_page() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.cloud.connect(MockBackend { entries: vec![] });
        app.cloud_page = Some(CloudPage::SaveAs);
        app.run("file.new", serde_json::json!({})).unwrap();
        save_as(&mut app, "", "demo").expect("a connected backend with an open drawing accepts the save");
        pump(&mut app);
        assert!(app.cloud_page.is_none(), "a successful save closes the page");
        assert!(app.ui.status.contains("demo"));
    }
}
