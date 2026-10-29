//! Remote storage in the browser: files on a same-origin WebDAV server appear as mounts
//! (`./vectorcraft-config.json`, see [`crate::remote_config`]). Listings are PROPFIND requests,
//! files are read with `fetch`, saves are uploaded by [`crate::remote_upload`]. The pure parts live
//! in [`crate::remote_config`], [`crate::remote_dav`] and [`crate::remote_write`]; this module is
//! the browser side, and hands the app a [`RemoteStorage`] (`Services::remote`).
//!
//! The app's callbacks are synchronous, so a listing starts on the first look and answers
//! `WouldBlock` ("Loading…") until it arrives; the dialog asks again on the next frames.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;

use vectorcraft_ui_egui::remote::{Purpose, RemoteEntry, RemoteEvent, RemoteInbox, RemoteMount, RemoteStorage};

use crate::remote_config::{self, Config};
use crate::remote_dav::{self, Child};
use crate::remote_http::{fetch, fetch_within, header, read_body};
use crate::remote_util::{self as util, ListingView};

/// A failed listing is not asked for again for this long (seconds).
const ERROR_RETRY_S: f64 = 5.0;
/// Largest file opened or placed from the server (bytes).
const MAX_FILE_BYTES: usize = 512 << 20;
/// Largest config file read.
const CONFIG_MAX: usize = 64 * 1024;

#[derive(Default)]
struct DirState {
    entries: Option<Vec<RemoteEntry>>,
    fetched: f64,
    /// Whether the listing was handed out since it was fetched (a TTL of 0 refetches after that).
    shown: bool,
    /// When the request in flight started.
    loading: Option<f64>,
    error: Option<(String, f64)>,
}

#[derive(Default)]
struct State {
    cfg: Option<Config>,
    origin: String,
    dirs: HashMap<String, DirState>,
    ctx: Option<egui::Context>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Seconds since the page started (monotonic).
fn now_s() -> f64 {
    js_sys::Date::now() / 1000.0
}

/// Ask the UI for a frame (something arrived).
fn repaint() {
    if let Some(ctx) = STATE.with(|s| s.borrow().ctx.clone()) {
        ctx.request_repaint();
    }
}

/// The remote config and the page's origin, or why remote storage is off.
pub(crate) fn cfg_and_origin() -> Result<(Config, String), String> {
    STATE.with(|s| {
        let s = s.borrow();
        s.cfg.clone().map(|c| (c, s.origin.clone())).ok_or_else(|| "remote storage is not configured".to_string())
    })
}

/// Read `./vectorcraft-config.json`; remote storage stays off when it is missing or invalid.
pub async fn load_config() {
    let Some(loc) = web_sys::window().map(|w| w.location()) else { return };
    let (origin, path) = (loc.origin().unwrap_or_default(), loc.pathname().unwrap_or_default());
    let base_dir = path.get(..=path.rfind('/').unwrap_or(0)).unwrap_or("/").to_string();
    // A server that does not answer must not hold the app back: the time limit turns it into "off".
    let bytes = match fetch_within("GET", "vectorcraft-config.json", &[], None, util::CONFIG_TIMEOUT).await {
        Ok(r) if r.status() == 200 => read_body(&r, CONFIG_MAX).await,
        Ok(r) => Err(format!("status {}", r.status())),
        Err(e) => Err(e),
    };
    match bytes.and_then(|b| remote_config::parse(&b, &origin, &base_dir)) {
        Ok(cfg) => {
            log::info!("remote storage: {} mount(s)", cfg.mounts.len());
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.cfg = Some(cfg);
                s.origin = origin;
            });
        }
        Err(e) => log::info!("remote storage off: {e}"),
    }
}

/// PROPFIND `dir` (Depth 1) and map the answer to virtual paths.
async fn propfind(dir: &str) -> Result<Vec<Child>, String> {
    let (cfg, origin) = cfg_and_origin()?;
    let mount = cfg.mount_for(dir).ok_or_else(|| format!("{dir}: not on a remote mount"))?.clone();
    let url = cfg.url_for(&format!("{}/", dir.trim_end_matches('/')))?;
    let r = fetch("PROPFIND", &url, &[("Depth", "1"), ("Content-Type", "application/xml")], Some(remote_dav::PROPFIND_BODY.as_bytes())).await?;
    if r.status() != 207 {
        return Err(format!("{dir}: the server answered {} to a listing", r.status()));
    }
    let entries = remote_dav::parse_multistatus(&read_body(&r, remote_dav::MAX_XML_BYTES).await?)?;
    Ok(remote_dav::children_of(&cfg, &mount, dir, &origin, entries))
}

/// Store a listing's answer (entries, or the error) for `dir` and end its loading state.
fn record_dir(dir: &str, res: Result<Vec<Child>, String>) {
    let now = now_s();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.dirs.entry(dir.to_string()).or_default();
        st.loading = None;
        match res {
            Ok(children) => {
                st.error = None;
                st.fetched = now;
                st.shown = false;
                st.entries =
                    Some(children.iter().map(|c| RemoteEntry { name: c.name.clone(), is_dir: c.entry.is_dir, size: c.entry.size }).collect());
            }
            Err(e) => st.error = Some((e, now)),
        }
    });
}

/// Ask the UI for a frame after `d`.
fn repaint_after(d: std::time::Duration) {
    if let Some(ctx) = STATE.with(|s| s.borrow().ctx.clone()) {
        ctx.request_repaint_after(d);
    }
}

/// Start the PROPFIND of `dir` in the background; the Media Browser sees it on a later frame.
fn start_listing(dir: String) {
    // Look again when the last error may give way to "Loading…".
    repaint_after(std::time::Duration::from_secs_f64(util::LOADING_GRACE_S + 0.05));
    wasm_bindgen_futures::spawn_local(async move {
        let res = propfind(&dir).await;
        record_dir(&dir, res);
        repaint();
    });
}

/// The listing of a folder: cached, else started and answered `WouldBlock` until it arrives. An
/// expired one is shown while it refreshes.
fn list(dir: &str) -> io::Result<Vec<RemoteEntry>> {
    let dir = dir.trim_end_matches('/').to_string();
    if !is_remote(&dir) {
        return Err(io::Error::other(format!("{dir} is not on remote storage")));
    }
    let now = now_s();
    let (result, start) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let ttl = s.cfg.as_ref().map_or(remote_config::DEFAULT_LIST_CACHE_S, |c| c.list_cache_s);
        let st = s.dirs.entry(dir.clone()).or_default();
        // A listing that was not shown yet is always fresh, so a TTL of 0 does not refetch in a loop.
        let fresh = st.entries.is_some() && (!st.shown || now - st.fetched < ttl);
        let blocked = st.loading.is_some() || st.error.as_ref().is_some_and(|(_, at)| now - at < ERROR_RETRY_S);
        let start = !fresh && !blocked;
        if start {
            st.loading = Some(now);
        }
        st.shown |= st.entries.is_some();
        // The last error stays up until the retry has run for a moment (no error/Loading flicker).
        let result = match (util::listing_view(st.entries.is_some(), st.error.is_some(), st.loading.map(|t| now - t)), &st.entries, &st.error) {
            (ListingView::Entries, Some(e), _) => Ok(e.clone()),
            (ListingView::Error, _, Some((msg, _))) => Err(io::Error::other(msg.clone())),
            _ => Err(io::Error::new(io::ErrorKind::WouldBlock, "Loading…")),
        };
        (result, start)
    });
    if start {
        start_listing(dir);
    }
    result
}

/// Is `path` on a remote mount?
fn is_remote(path: &str) -> bool {
    STATE.with(|s| s.borrow().cfg.as_ref().is_some_and(|c| c.mount_for(path).is_some()))
}

/// Drop the cached listing of the folder of `path` (after it was written): the next look asks again.
pub(crate) fn forget(path: &str) {
    STATE.with(|s| {
        if let Some(p) = remote_config::parent(path) {
            s.borrow_mut().dirs.remove(p);
        }
    });
}

/// GET a whole file. A file opened (not one placed) has its ETag remembered for the save that
/// follows, once its body was read in full.
async fn get_file(path: &str, purpose: Purpose) -> Result<Vec<u8>, String> {
    let (cfg, _) = cfg_and_origin()?;
    let r = fetch("GET", &cfg.url_for(path)?, &[], None).await?;
    if r.status() != 200 {
        return Err(format!("the server answered {}", r.status()));
    }
    let etag = header(&r, "etag");
    let bytes = read_body(&r, MAX_FILE_BYTES).await?;
    if purpose == Purpose::Open {
        crate::remote_upload::note_opened(path, etag);
    }
    Ok(bytes)
}

/// Fetch `path` in the background; the result is a [`RemoteEvent::Fetched`] in the inbox.
fn start_fetch(path: &str, purpose: Purpose, inbox: &RemoteInbox) {
    let (path, inbox) = (path.to_string(), inbox.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let result = get_file(&path, purpose).await;
        inbox.lock().unwrap_or_else(|e| e.into_inner()).push(RemoteEvent::Fetched { path, purpose, result });
        repaint();
    });
}

/// The app's remote storage; `None` when the page has no (valid) config. `ctx` is woken when
/// something arrives.
pub fn storage(ctx: &egui::Context, inbox: &RemoteInbox) -> Option<RemoteStorage> {
    let (cfg, _) = cfg_and_origin().ok()?;
    STATE.with(|s| s.borrow_mut().ctx = Some(ctx.clone()));
    let mounts = cfg.mounts.iter().map(|m| RemoteMount { name: m.name.clone(), path: m.path.clone(), writable: m.writable }).collect();
    let fetch_inbox = inbox.clone();
    let write_inbox = inbox.clone();
    Some(RemoteStorage {
        mounts,
        is_remote: Box::new(is_remote),
        list: Box::new(list),
        fetch: Box::new(move |path, purpose| start_fetch(path, purpose, &fetch_inbox)),
        write: Box::new(move |path, data, new_file| crate::remote_upload::write(path, data, new_file, &write_inbox)),
        inbox: inbox.clone(),
        new_files: Default::default(),
    })
}
