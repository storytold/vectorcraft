//! Linked files that change while their document is open (another app saved them: PhotoCraft,
//! LightCraft, Photoshop, a placed VectorCraft document): Preferences › File Handling › Update
//! Links decides, as it does when a document opens. Automatically reads them again
//! (`links.update`, an undo step), Ask When Modified offers to, Manually leaves them modified in
//! the Links panel.
//!
//! The active document's linked files are compared by size and modification time with what they
//! were when last seen. The desktop app takes the stamps every two seconds and when its window
//! comes back to the front, on a worker thread so a file on a slow network share never holds up
//! the interface ([`Session::start_link_scan`], [`Session::poll_link_scan`]);
//! `links.updateChanged` takes them on the spot and reads the changed files again (CLI, agents).
//!
//! Stamps are kept per document: one in the background is looked at when it comes to the front.
//! A file seen for the first time is only remembered (one modified while its document was closed
//! is handled when the document opens); one that can't be read yet (another app is still writing
//! it) keeps its old stamp, so the next look tries again. A change is acted on once: undoing an
//! update, or answering No, leaves the link modified until the file changes again.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use crate::{Result, Session};

/// A file's size and modification time (nanoseconds since 1970).
pub type Stamp = (u64, u128);

/// Stamps of a document's linked files taken on a worker thread: (document uid, [(file,
/// object ids, stamp)]).
pub type LinkScan = Arc<Mutex<Option<(u64, Vec<(String, Vec<u64>, Option<Stamp>)>)>>>;

#[cfg(not(target_arch = "wasm32"))]
fn stamp(path: &str) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos()))
}

#[cfg(target_arch = "wasm32")]
fn stamp(_: &str) -> Option<Stamp> {
    None
}

/// The linked files of `d` with the ids of the objects showing them.
pub fn linked_files(d: &vectorcraft_doc::Document) -> Vec<(String, Vec<u64>)> {
    crate::cmd::links::linked_groups(d)
}

/// `links.updateChanged`: stamp the active document's linked files now and read the changed
/// ones again, whatever Update Links says.
pub(crate) fn update_changed(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let (uid, files) = (st.uid, crate::cmd::links::linked_groups(&st.doc));
    let now = files.into_iter().map(|(p, ids)| (p.clone(), ids, stamp(&p))).collect();
    s.apply_link_stamps(uid, now, true)
}

impl Session {
    /// Start stamping the active document's linked files on a worker thread, unless a look is
    /// still running (or the build has no files to look at: the web app).
    pub fn start_link_scan(&mut self) {
        if self.link_scan.is_some() || cfg!(target_arch = "wasm32") {
            return;
        }
        let Some(st) = self.active() else { return };
        let (uid, files) = (st.uid, crate::cmd::links::linked_groups(&st.doc));
        if files.is_empty() {
            return;
        }
        let slot: LinkScan = Arc::default();
        let out = slot.clone();
        let spawned = std::thread::Builder::new().name("link-watch".into()).spawn(move || {
            let stamps = files.into_iter().map(|(p, ids)| (p.clone(), ids, stamp(&p))).collect();
            *out.lock().unwrap_or_else(PoisonError::into_inner) = Some((uid, stamps));
        });
        if spawned.is_ok() {
            self.link_scan = Some(slot);
        }
    }

    /// Apply a finished look (`None` while there is none, or it is still running): see
    /// [`Session::apply_link_stamps`].
    pub fn poll_link_scan(&mut self) -> Option<Result<Value>> {
        let (uid, stamps) = self.link_scan.as_ref()?.lock().unwrap_or_else(PoisonError::into_inner).take()?;
        self.link_scan = None;
        Some(self.apply_link_stamps(uid, stamps, false))
    }

    /// Compare the stamps of document `uid`'s linked files with the ones last seen and act on
    /// the changed ones as Update Links says (`update`: read them again regardless). Returns
    /// `{updated: [ids], ask: [ids], modified: [ids]}`: read again, to offer (Ask When Modified),
    /// or left modified (Manually).
    pub fn apply_link_stamps(&mut self, uid: u64, now: Vec<(String, Vec<u64>, Option<Stamp>)>, update: bool) -> Result<Value> {
        let none = || json!({"updated": [], "ask": [], "modified": []});
        // The document went to the back meanwhile: it's looked at when it's in front again.
        if self.active().map(|d| d.uid) != Some(uid) {
            return Ok(none());
        }
        // This document's last stamps; those of closed documents are dropped.
        let open: HashSet<u64> = self.documents().iter().map(|d| d.uid).collect();
        let mut before: HashMap<String, Stamp> = HashMap::new();
        self.link_stamps.retain(|(u, p), s| {
            if *u == uid {
                before.insert(p.clone(), *s);
                false
            } else {
                open.contains(u)
            }
        });
        let mut changed: Vec<(String, Vec<u64>)> = Vec::new();
        for (path, ids, stamp) in now {
            match (before.get(&path), stamp) {
                // Not there right now (an app saving by delete and rename): keep the last stamp.
                (Some(old), None) => {
                    self.link_stamps.insert((uid, path), *old);
                }
                (old, Some(new)) => {
                    if old.is_some_and(|o| *o != new) {
                        changed.push((path.clone(), ids));
                    }
                    self.link_stamps.insert((uid, path), new);
                }
                (None, None) => {}
            }
        }
        if changed.is_empty() {
            return Ok(none());
        }
        let ids: Vec<u64> = changed.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
        let mode = if update { "automatically" } else { self.prefs.update_links.as_str() };
        match mode {
            "automatically" => {
                let r = self.execute("links.update", &json!({"ids": ids}))?;
                // Not readable yet (still being written): the next look tries again.
                let unread: BTreeSet<u64> = r["missing"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                for (path, ids) in &changed {
                    if ids.iter().any(|i| unread.contains(i))
                        && let Some(old) = before.get(path)
                    {
                        self.link_stamps.insert((uid, path.clone()), *old);
                    }
                }
                Ok(json!({"updated": r["updated"], "ask": [], "modified": []}))
            }
            "askWhenModified" => Ok(json!({"updated": [], "ask": ids, "modified": []})),
            _ => Ok(json!({"updated": [], "ask": [], "modified": ids})),
        }
    }
}
