//! Saving a document to a writable remote mount ([`crate::remote_write`] has the protocol). The
//! app's write is synchronous, so the upload runs in the background (`.part` file, check of the
//! target's ETag, move) and a failure comes back as a `SaveFailed` event: the document is marked
//! unsaved again and the user is told why.
//!
//! One upload runs per path at a time; a save made meanwhile replaces the one waiting.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use vectorcraft_ui_egui::remote::{RemoteEvent, RemoteInbox};

use crate::remote::cfg_and_origin;
use crate::remote_http::{Resp, fetch, header, read_body};
use crate::remote_write::{self as rw, Known, Outcome, Precondition, Req};

#[derive(Default)]
struct Uploads {
    /// The ETag each path had when this session opened or last saved it (`None`: the server sent none).
    seen: HashMap<String, Option<String>>,
    running: HashSet<String>,
    /// The save waiting behind the running one: its bytes, and whether it creates a file (a Save As
    /// to a new name, see [`write`]).
    waiting: HashMap<String, (Vec<u8>, bool)>,
}

thread_local! {
    static UPLOADS: RefCell<Uploads> = RefCell::new(Uploads::default());
}

/// The text of a save that found the file changed on the server.
const CONFLICT: &str = "Document changed on the server; use Save As";

/// Remember the ETag of a file just read from the server.
pub fn note_opened(path: &str, etag: Option<String>) {
    UPLOADS.with(|u| u.borrow_mut().seen.insert(path.to_string(), etag));
}

/// Forget the version of `path`: it cannot be tied to what this session wrote, so the next save is
/// a conflict or Save As.
fn forget_version(path: &str) {
    UPLOADS.with(|u| u.borrow_mut().seen.remove(path));
}

/// What this session knows about the version of `path` on the server. A save that creates a file
/// (`new_file`) knows nothing of it, whatever was opened under that name before: an existing
/// target is then refused, never replaced.
fn known(path: &str, new_file: bool) -> Known {
    if new_file {
        return Known::Unknown;
    }
    UPLOADS.with(|u| match u.borrow().seen.get(path) {
        Some(Some(e)) => Known::Etag(e.clone()),
        Some(None) => Known::NoEtag,
        None => Known::Unknown,
    })
}

/// Send one request of a save with its time limit.
async fn send(r: &Req, body: Option<&[u8]>) -> Result<Resp, String> {
    let headers: Vec<(&str, &str)> = r.headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    fetch(r.method, &r.url, &headers, body).await
}

/// The app's write to a path on a mount: a read-only mount refuses; otherwise the upload starts
/// and the write succeeds. `new_file`: a Save As to a path other than the document's own, which
/// must not replace a file that is there.
pub fn write(path: &str, data: &[u8], new_file: bool, inbox: &RemoteInbox) -> Result<(), String> {
    let (cfg, _) = cfg_and_origin()?;
    rw::check_writable(&cfg, path)?;
    if data.len() > rw::MAX_UPLOAD {
        return Err("the document is too large to save to the server".into());
    }
    let start = UPLOADS.with(|u| {
        let mut u = u.borrow_mut();
        // a save queued behind one that creates the file still creates it
        let new_file = new_file || u.waiting.get(path).is_some_and(|(_, n)| *n);
        u.waiting.insert(path.to_string(), (data.to_vec(), new_file));
        u.running.insert(path.to_string())
    });
    if start {
        let (path, inbox) = (path.to_string(), inbox.clone());
        wasm_bindgen_futures::spawn_local(async move {
            while let Some((bytes, new_file)) = UPLOADS.with(|u| u.borrow_mut().waiting.remove(&path)) {
                let outcome = upload(&path, &bytes, new_file).await;
                // Saved is reported once nothing newer waits: that upload holds the newest bytes
                let newest = UPLOADS.with(|u| !u.borrow().waiting.contains_key(&path));
                report(&path, &outcome, newest, &inbox);
            }
            UPLOADS.with(|u| u.borrow_mut().running.remove(&path));
        });
    }
    Ok(())
}

/// A failed save leaves the document unsaved and says why; the last of the saves made meanwhile
/// that reached the server (`newest`) lets the app count the document as saved.
fn report(path: &str, outcome: &Outcome, newest: bool, inbox: &RemoteInbox) {
    let message = match outcome {
        Outcome::Conflict => CONFLICT.to_string(),
        other => match rw::failure_message(other) {
            Some(m) => m,
            None => {
                if newest {
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(RemoteEvent::Saved { path: path.to_string() });
                }
                return;
            }
        },
    };
    log::warn!("saving {path}: {message}");
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(RemoteEvent::SaveFailed { path: path.to_string(), message });
}

/// A token for one upload's `.part` file: 128 random bits as hex, so two saves of a path (this
/// tab or another client) never share a part file.
fn new_token() -> String {
    format!("{:016x}{:016x}", js_sys::Math::random().to_bits(), js_sys::Math::random().to_bits() ^ js_sys::Date::now().to_bits())
}

/// Upload `bytes` to this upload's own `.part` file, check the target, move the upload over it.
async fn upload(path: &str, bytes: &[u8], new_file: bool) -> Outcome {
    let (cfg, origin) = match cfg_and_origin() {
        Ok(v) => v,
        Err(e) => return Outcome::Failed(e),
    };
    let token = new_token();
    let outcome = upload_steps(&cfg, &origin, path, &token, bytes, new_file).await;
    if outcome != Outcome::Saved {
        // best effort: the .part file is of no use
        if let Ok(d) = rw::delete_part(&cfg, path, &token) {
            let _ = send(&d, None).await;
        }
    }
    outcome
}

/// The saved version when the server gave the moved target a new ETag: read the target back (at most
/// [`rw::MAX_UPLOAD`] bytes) and bind its ETag only if its bytes are the ones uploaded. Any failure
/// leaves the version unbound (the next save is then a conflict or Save As, never an overwrite).
async fn verify_by_content(cfg: &crate::remote_config::Config, path: &str, bytes: &[u8]) -> rw::SavedVersion {
    let Ok(get) = rw::get_target(cfg, path) else { return rw::SavedVersion::Unbound };
    let Ok(r) = send(&get, None).await else { return rw::SavedVersion::Unbound };
    let (status, etag) = (r.status(), header(&r, "etag"));
    let body = if status == 200 { read_body(&r, rw::MAX_UPLOAD).await.ok() } else { None };
    rw::verify_saved(bytes, body.as_deref().map(|b| (status, etag.as_deref(), b)))
}

/// The requests of one save, in order; the first that fails ends it.
async fn upload_steps(cfg: &crate::remote_config::Config, origin: &str, path: &str, token: &str, bytes: &[u8], new_file: bool) -> Outcome {
    let step = Outcome::Failed;
    let put = match rw::put_part(cfg, path, token) {
        Ok(r) => r,
        Err(e) => return step(e),
    };
    let mut part_etag = match send(&put, Some(bytes)).await {
        Ok(r) => {
            if let Err(e) = rw::classify_put(r.status()) {
                return step(e);
            }
            header(&r, "etag")
        }
        Err(e) => return step(e),
    };
    // the upload's own version, to tie the target's to it after the move
    if part_etag.is_none() {
        part_etag = match rw::head_part(cfg, path, token) {
            Ok(h) => send(&h, None).await.ok().filter(|r| r.status() == 200).and_then(|r| header(&r, "etag")),
            Err(e) => return step(e),
        };
    }
    let head = match rw::head_target(cfg, path) {
        Ok(r) => r,
        Err(e) => return step(e),
    };
    let overwrite = match send(&head, None).await {
        Ok(r) => match rw::precondition(&known(path, new_file), r.status(), header(&r, "etag").as_deref()) {
            Precondition::Proceed { overwrite } => overwrite,
            Precondition::Conflict => return Outcome::Conflict,
            Precondition::Exists => return Outcome::Exists,
            Precondition::Failed(e) => return step(e),
        },
        Err(e) => return step(e),
    };
    let mv = match rw::move_part(cfg, origin, path, token, overwrite) {
        Ok(r) => r,
        Err(e) => return step(e),
    };
    let (outcome, moved_etag) = match send(&mv, None).await {
        Ok(r) => (rw::classify_move(r.status()), header(&r, "etag")),
        Err(e) => return step(e),
    };
    if outcome == Outcome::Saved {
        // The version this save made: the move's own, else the target's when it is the upload's
        // (see `saved_version`). A follow-up HEAD alone could show another writer's version.
        // `None`: the HEAD failed (not recorded as "no ETag"); `Some(None)`: it answered without one.
        let after = match send(&head, None).await {
            Ok(r) if r.status() == 200 => Some(header(&r, "etag")),
            _ => None,
        };
        let mut version = rw::saved_version(moved_etag.as_deref(), part_etag.as_deref(), after.as_ref().map(Option::as_deref));
        if version == rw::SavedVersion::Verify {
            version = verify_by_content(cfg, path, bytes).await;
        }
        match version {
            rw::SavedVersion::Etag(e) => note_opened(path, Some(e)),
            rw::SavedVersion::NoEtag => note_opened(path, None),
            rw::SavedVersion::Unbound | rw::SavedVersion::Verify => forget_version(path),
        }
        crate::remote::forget(path);
    }
    outcome
}
