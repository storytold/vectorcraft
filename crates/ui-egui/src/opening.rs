//! Opening documents off the UI thread (desktop): a file the user asks to open (File › Open, Open
//! Recent, the Home screen, files handed to the app) is decoded on a thread of its own, which also
//! reads its linked files and lays out its type, while the status bar shows it, and opens in a new tab
//! once it is ready, as it would at once: the same notes, recent files and errors. On wasm (no
//! threads) and in unit tests it opens at once ([`available`]).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::{self, ReadFile};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// A file being read.
pub struct Pending {
    /// Where it is.
    pub path: String,
    /// What the status bar shows ("Opening Poster.vectorcraft").
    pub label: String,
    done: Receiver<Result<ReadFile, String>>,
}

/// The files being read.
#[derive(Default)]
pub struct Opening {
    pub pending: Vec<Pending>,
}

/// Is a long operation running that should show the busy cursor and status spinner (background
/// save/export with a label, a file opening or being read for Place off the UI thread, a heavy plug-in filter, or an
/// Image Trace)?
pub fn long_operation_busy(app: &VectorcraftApp) -> bool {
    app.background.jobs.iter().any(|j| !j.label.is_empty() && !j.cancelled())
        || !app.opening.pending.is_empty()
        || !app.plugin_runs.running.is_empty()
        || !app.placing.pending.is_empty()
        || !app.trace_runs.running.is_empty()
}

/// Does opening happen off the UI thread here? Not on wasm (no threads), in unit tests (they look
/// at the document right after asking for it), or with `VECTORCRAFT_SYNC_OPEN` set.
pub fn available() -> bool {
    !(cfg!(target_arch = "wasm32") || cfg!(test) || std::env::var_os("VECTORCRAFT_SYNC_OPEN").is_some())
}

/// Decode `bytes`, the file at `path`, read its linked files and lay out its type on a thread of its
/// own ([`fileio::read_for_open`]) → `{background: true, status}`, or the bytes back when no thread
/// could start (the caller opens the file at once).
pub fn start(app: &mut VectorcraftApp, path: &str, bytes: Vec<u8>) -> Result<Value, Vec<u8>> {
    let (tx, done) = channel();
    let bytes = Arc::new(bytes);
    let (name, file) = (path.to_string(), bytes.clone());
    // Update Links (Preferences › File Handling) applies there as it is set now.
    let update_links = app.session.prefs.update_links == "automatically";
    let low_res_proxy_eps = app.session.prefs.low_res_proxy_eps;
    // The automation roots in force here are in force there.
    let scope = ScopeHandle::capture();
    let spawned = std::thread::Builder::new().name("vectorcraft-open".into()).spawn(move || {
        let r = scope
            .run(|| {
                vectorcraft_engine::guard::catch_panic(|| {
                    fileio::read_for_open(&name, &file, Some(name.clone()), &Value::Null, update_links, low_res_proxy_eps).map_err(|e| e.to_string())
                })
            })
            .unwrap_or_else(|msg| Err(format!("internal error: {msg} (please report this bug)")));
        // The file's bytes go before the document is handed over.
        drop(file);
        // The app may have quit meanwhile: nothing to hand it to.
        let _ = tx.send(r);
    });
    if spawned.is_err() {
        return Err(Arc::try_unwrap(bytes).unwrap_or_else(|b| b.to_vec()));
    }
    let label = format!("Opening {}", fileio::file_name(path));
    app.opening.pending.push(Pending { path: path.to_string(), label: label.clone(), done });
    let status = format!("{label}…");
    app.status(status.clone());
    Ok(json!({ "background": true, "status": status }))
}

/// Open the files that are ready (each frame). Not while a dialog is open: a document that opens
/// becomes the active one, and the dialog's OK acts on the active document (#1694: Save As
/// options confirmed after another file had finished opening saved that file's art under the
/// first one's name). The files wait, their status line too, until the dialog is closed. Nor while
/// an interaction is open (a drag, a slider's preview, or a tool's preview between its clicks) or
/// a tool is busy with a drag of its own while the pointer button is held: a document that opens
/// cancels it. A tool that is busy while the button is not held and no interaction is open is
/// waiting between its clicks (the Dimension tool after its first point): the file opens, and the
/// tool starts afresh, as when the user switches documents.
pub fn poll(app: &mut VectorcraftApp) {
    let s = &app.session;
    if app.ui.dialog.is_some() || s.in_interaction() || (s.tool_busy() && s.pointer_held()) {
        return;
    }
    let mut i = 0;
    while let Some(p) = app.opening.pending.get(i) {
        let r = match p.done.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                i += 1;
                continue;
            }
            Err(TryRecvError::Disconnected) => Err("the file stopped being read (please report this bug)".to_string()),
        };
        let p = app.opening.pending.remove(i);
        // The status bar no longer says it is opening.
        if app.ui.status == format!("{}…", p.label) {
            app.ui.status.clear();
        }
        finish(app, &p.path, r);
    }
}

/// Open a file read off the UI thread, as [`crate::io::open_reporting`] opens one read at once.
fn finish(app: &mut VectorcraftApp, path: &str, r: Result<ReadFile, String>) {
    let opened = r.and_then(|file| fileio::open_read(&mut app.session, file).map_err(|e| e.to_string()));
    match opened {
        Ok(r) => {
            crate::io::opened(app, &r);
            crate::io::note_recent(app, path);
        }
        Err(e) => crate::io::report_open_error(app, &fileio::file_name(path), &e),
    }
}

#[cfg(test)]
mod tests;
