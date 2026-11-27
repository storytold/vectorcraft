//! Language packs (#4936, RFC-0011): the spelling dictionary, hyphenation patterns and licence
//! texts of a language other than English, published by storytold/craft-i18n and installed into the
//! Craft apps' shared font store (craft-libs `craft-font-store`, item kind `language-pack`), where
//! craft-proof reads them (`craft_proof::packs`). A pack installed by another Craft app is used
//! here too, since the apps share the store.
//!
//! The store downloads only while its download setting is on, which only the person using an app
//! turns on, with their own click. Nothing here turns it on: [`use_font_store`] opens the store as
//! an agent's, so `proof.installPack` from the CLI, MCP or a script installs only after the person
//! allowed downloads.
//!
//! The packs state is the process's, like craft-proof's: one backend ([`set_backend`]), one list of
//! installs running and one error per locale. In the desktop app installs run on a thread of their
//! own ([`run_in_background`]) and [`crate::Session::poll_language_packs`] lays out the documents
//! again when one ends; elsewhere they return once installed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

pub use craft_font_store::Cancel;

/// One pack the pack index lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pack {
    /// Lowercase BCP 47 tag (`nl`, `pt-br`), also the installed pack's folder name.
    pub locale: String,
    /// The language's own name (`Nederlands`).
    pub name: String,
    /// The download's size in bytes.
    pub size: u64,
    /// SPDX licence expression of the pack's data.
    pub licence: String,
    /// The licence class: `Embeddable` or `Download-only`.
    pub class: String,
    /// Whether it has a spelling dictionary.
    pub spelling: bool,
    /// Whether it has hyphenation patterns.
    pub hyphenation: bool,
}

/// Where packs come from and go to: the font store ([`use_font_store`]), or a stand-in in tests.
pub trait Backend: Send + Sync {
    /// The installed-packs folder craft-proof reads.
    fn packs_dir(&self) -> PathBuf;
    /// Whether the person allowed downloads.
    fn downloads_on(&self) -> bool;
    /// The packs of the index fetched last, with no request; empty before the first fetch.
    fn cached_index(&self) -> Vec<Pack>;
    /// Fetches the pinned pack index (needs downloads on).
    fn fetch_index(&self, cancel: &Cancel) -> Result<Vec<Pack>, String>;
    /// Downloads, checks and installs the pack for `locale`, which the index lists (needs downloads
    /// on), calling `progress(done, total)` in bytes.
    fn install(&self, locale: &str, cancel: &Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String>;
    /// Removes the installed pack for `locale`.
    fn remove(&self, locale: &str) -> Result<(), String>;
    /// The test index used instead of the pinned one (`CRAFT_LANGUAGE_PACK_INDEX`), if any.
    fn index_override(&self) -> Option<String> {
        None
    }
}

/// An install running.
struct Job {
    done: u64,
    total: u64,
    cancel: Cancel,
}

#[derive(Default)]
struct Jobs {
    installing: BTreeMap<String, Job>,
    /// A fetch of the index running.
    refreshing: Option<Cancel>,
    /// The last failure of each locale's install or removal, and of the index fetch (key `""`).
    errors: BTreeMap<String, String>,
}

static BACKEND: RwLock<Option<Arc<dyn Backend>>> = RwLock::new(None);
static JOBS: Mutex<Jobs> = Mutex::new(Jobs { installing: BTreeMap::new(), refreshing: None, errors: BTreeMap::new() });
static BACKGROUND: AtomicBool = AtomicBool::new(false);
/// Goes up each time the installed packs change.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn jobs() -> MutexGuard<'static, Jobs> {
    JOBS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Use `backend` for packs (`None`: none), and have craft-proof read its installed packs.
pub fn set_backend(backend: Option<Arc<dyn Backend>>) {
    let dir = backend.as_ref().map(|b| b.packs_dir());
    *BACKEND.write().unwrap_or_else(PoisonError::into_inner) = backend;
    craft_proof::packs::set_packs_dir(dir);
    changed();
}

/// The backend [`set_backend`] set.
pub fn backend() -> Option<Arc<dyn Backend>> {
    BACKEND.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Use the Craft apps' shared font store for packs, as an agent's store (it can't turn downloads
/// on): the desktop app and `vectorcraft-cli` call it at startup. Without a per-user folder there
/// are no packs.
pub fn use_font_store() {
    let env = craft_font_catalog::FontEnv::from_process();
    set_backend(craft_font_store::FontStore::for_agent(&env).map(|store| Arc::new(FontStoreBackend::new(store)) as Arc<dyn Backend>));
}

/// Run installs and index fetches on a thread of their own (the desktop app, so its window keeps
/// working) instead of returning once they are done.
pub fn run_in_background(on: bool) {
    BACKGROUND.store(on, Ordering::Relaxed);
}

/// Goes up each time the installed packs change ([`crate::Session::poll_language_packs`]).
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Read the packs folder again: spelling and hyphenation in its languages change.
fn changed() {
    craft_proof::packs::reload();
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Whether an install or an index fetch is running.
pub fn busy() -> bool {
    let j = jobs();
    !j.installing.is_empty() || j.refreshing.is_some()
}

/// `raw` as a pack locale: trimmed, lower case, `_` read as `-` (`pt_BR` is `pt-br`); a language
/// tag of letters and digits, at most 35 bytes, in parts of at most 8.
pub fn parse_locale(raw: &str) -> Result<String, String> {
    let l = raw.trim().to_ascii_lowercase().replace('_', "-");
    let ok = !l.is_empty()
        && l.len() <= 35
        && l.split('-').all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    if ok { Ok(l) } else { Err(format!("{:?} is not a language tag like nl or pt-br", raw.chars().take(40).collect::<String>())) }
}

fn no_backend() -> String {
    "Language packs aren't available: there is no font store folder for this user".into()
}

fn downloads_off() -> String {
    "downloading is turned off; only the person using a Craft app can turn it on, by installing a Language pack there".into()
}

/// The packs the index fetched last lists.
pub fn available() -> Vec<Pack> {
    backend().map(|b| b.cached_index()).unwrap_or_default()
}

/// What `proof.packs` reports about the jobs: the installs running as (locale, done, total), whether
/// the index is being fetched, and the last error of each locale (`""`: the index).
pub fn status() -> (Vec<(String, u64, u64)>, bool, BTreeMap<String, String>) {
    let j = jobs();
    (j.installing.iter().map(|(l, job)| (l.clone(), job.done, job.total)).collect(), j.refreshing.is_some(), j.errors.clone())
}

/// What an install or a fetch did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Done.
    Done,
    /// Running in the background ([`run_in_background`]).
    Started,
}

/// Runs `f` on a thread of its own in the background, else here.
fn run(f: impl FnOnce() + Send + 'static) -> Result<Outcome, String> {
    if !BACKGROUND.load(Ordering::Relaxed) {
        f();
        return Ok(Outcome::Done);
    }
    std::thread::Builder::new().name("language-pack".into()).spawn(f).map(|_| Outcome::Started).map_err(|e| format!("couldn't start the download: {e}"))
}

/// Fetches the pinned pack index again. Needs downloads on.
pub fn refresh() -> Result<Outcome, String> {
    let b = backend().ok_or_else(no_backend)?;
    if !b.downloads_on() {
        return Err(downloads_off());
    }
    let cancel = {
        let mut j = jobs();
        if j.refreshing.is_some() {
            return Ok(Outcome::Started);
        }
        let cancel = Cancel::new();
        j.refreshing = Some(cancel.clone());
        j.errors.remove("");
        cancel
    };
    let outcome = run(move || {
        let r = b.fetch_index(&cancel);
        let mut j = jobs();
        j.refreshing = None;
        if let Err(e) = r {
            j.errors.insert(String::new(), e);
        }
    });
    if outcome.is_err() {
        jobs().refreshing = None;
    }
    let outcome = outcome?;
    match jobs().errors.get("") {
        Some(e) if outcome == Outcome::Done => Err(e.clone()),
        _ => Ok(outcome),
    }
}

/// Installs the pack for `locale` (fetching the index first when it doesn't list it). Needs
/// downloads on. Spelling and hyphenation in that language use it once it is installed.
pub fn install(locale: &str) -> Result<Outcome, String> {
    let b = backend().ok_or_else(no_backend)?;
    if !b.downloads_on() {
        return Err(downloads_off());
    }
    let locale = locale.to_string();
    let cancel = {
        let mut j = jobs();
        if j.installing.contains_key(&locale) {
            return Ok(Outcome::Started);
        }
        let cancel = Cancel::new();
        j.installing.insert(locale.clone(), Job { done: 0, total: 0, cancel: cancel.clone() });
        j.errors.remove(&locale);
        cancel
    };
    let l = locale.clone();
    let outcome = run(move || {
        let r = install_now(&*b, &l, &cancel);
        if r.is_ok() {
            changed();
        }
        let mut j = jobs();
        j.installing.remove(&l);
        if let Err(e) = r {
            j.errors.insert(l, e);
        }
    });
    if outcome.is_err() {
        jobs().installing.remove(&locale);
    }
    let outcome = outcome?;
    match jobs().errors.get(&locale) {
        Some(e) if outcome == Outcome::Done => Err(e.clone()),
        _ => Ok(outcome),
    }
}

fn install_now(b: &dyn Backend, locale: &str, cancel: &Cancel) -> Result<(), String> {
    let listed = |packs: &[Pack]| packs.iter().any(|p| p.locale == locale);
    if !listed(&b.cached_index()) && !listed(&b.fetch_index(cancel)?) {
        return Err(format!("there is no Language pack for {locale}"));
    }
    b.install(locale, cancel, &mut |done, total| {
        if let Some(job) = jobs().installing.get_mut(locale) {
            (job.done, job.total) = (done, total);
        }
    })
}

/// Stops the install of `locale` (or the index fetch, for `""`) if one is running.
pub fn cancel(locale: &str) {
    let j = jobs();
    let c = if locale.is_empty() { j.refreshing.as_ref() } else { j.installing.get(locale).map(|job| &job.cancel) };
    if let Some(c) = c {
        c.cancel();
    }
}

/// Removes the installed pack for `locale`; not while it is installing.
pub fn remove(locale: &str) -> Result<(), String> {
    let b = backend().ok_or_else(no_backend)?;
    if jobs().installing.contains_key(locale) {
        return Err(format!("the {locale} pack is still installing"));
    }
    let r = b.remove(locale);
    changed();
    let mut j = jobs();
    match &r {
        Ok(()) => j.errors.remove(locale),
        Err(e) => j.errors.insert(locale.to_string(), e.clone()),
    };
    r
}

/// The pack index this build trusts: craft-i18n's Language pack release `language-packs-2` (75
/// packs), pinned by the sha256 of its `index.json`. The font store reads
/// `CRAFT_LANGUAGE_PACK_INDEX` itself, to try packs from a test index (testing only).
pub const PACK_INDEX: craft_font_store::PackIndexPin = craft_font_store::PackIndexPin {
    url: "https://github.com/storytold/craft-i18n/releases/download/language-packs-2/index.json",
    sha256: "b35c7aef0591177f18233b551da16035dcd615549ff874e169ffd19f16fea74b",
};

/// The font store as the packs [`Backend`].
struct FontStoreBackend {
    store: craft_font_store::FontStore,
    /// The index read or fetched last: an install takes its entry (URL, size, sha256) from it.
    index: Mutex<Option<craft_font_store::PackIndex>>,
}

impl FontStoreBackend {
    fn new(store: craft_font_store::FontStore) -> Self {
        Self { store, index: Mutex::new(None) }
    }

    fn keep(&self, index: craft_font_store::PackIndex) -> Vec<Pack> {
        let packs = index
            .packs
            .iter()
            .map(|e| Pack {
                locale: e.locale.clone(),
                name: e.name.clone(),
                size: e.size,
                licence: e.licence.clone(),
                class: e.class.clone(),
                spelling: e.spelling,
                hyphenation: e.hyphenation,
            })
            .collect();
        *self.index.lock().unwrap_or_else(PoisonError::into_inner) = Some(index);
        packs
    }
}

impl Backend for FontStoreBackend {
    fn packs_dir(&self) -> PathBuf {
        self.store.language_packs_dir()
    }

    fn downloads_on(&self) -> bool {
        self.store.network() == craft_font_store::Network::On
    }

    fn cached_index(&self) -> Vec<Pack> {
        self.store.cached_language_pack_index(&PACK_INDEX).map(|i| self.keep(i)).unwrap_or_default()
    }

    fn fetch_index(&self, cancel: &Cancel) -> Result<Vec<Pack>, String> {
        let index = self.store.fetch_language_pack_index(&PACK_INDEX, cancel).map_err(|e| e.to_string())?;
        Ok(self.keep(index))
    }

    fn install(&self, locale: &str, cancel: &Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        let held = self.index.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let index = held.or_else(|| self.store.cached_language_pack_index(&PACK_INDEX)).ok_or("the list of Language packs hasn't been fetched")?;
        let entry = index.pack(locale).ok_or_else(|| format!("there is no Language pack for {locale}"))?.clone();
        self.store.install_language_pack(&index, &entry, cancel, &mut |p| progress(p.done, p.total)).map(|_| ()).map_err(|e| e.to_string())
    }

    fn remove(&self, locale: &str) -> Result<(), String> {
        self.store.remove_language_pack(locale).map_err(|e| e.to_string())
    }

    fn index_override(&self) -> Option<String> {
        craft_font_store::language_pack_index_overridden()
    }
}

/// The font store backend in `dir`, for tests that need the real store without a request.
#[cfg(test)]
pub(crate) fn font_store_in(dir: PathBuf) -> Arc<dyn Backend> {
    Arc::new(FontStoreBackend::new(craft_font_store::FontStore::new(dir, craft_font_store::Caller::Agent)))
}
