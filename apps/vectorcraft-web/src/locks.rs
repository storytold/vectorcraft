//! The browser's Web Locks (`navigator.locks`), which tell Data Recovery whether another tab still
//! runs ([`vectorcraft_engine::cmd::recovery::RecoveryStore::announce`]). A tab holds a lock named
//! after its recovery area for as long as it lives; the browser releases it only when the tab is
//! gone (closed, crashed or reloaded), not while its timers are paused in the background, which a
//! heartbeat can't tell apart.
//!
//! web-sys has Web Locks only as an unstable API, so they are reached through `js_sys::Reflect`.
//! Pages that aren't secure (plain `http://` other than localhost) and older browsers have none:
//! heartbeats alone decide there.

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use js_sys::{Function, Promise, Reflect};
#[cfg(target_arch = "wasm32")]
use vectorcraft_engine::cmd::recovery::Hold;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast as _;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;

/// How often the snapshot of the locks held is refreshed (ms).
#[cfg(target_arch = "wasm32")]
const REFRESH_MS: i32 = 10_000;
/// How old a snapshot may be and still be trusted (ms). A lock asked for after the snapshot was
/// taken comes with a heartbeat, which stays fresh for at least three minutes, so anything under
/// that is safe; an older snapshot counts every area as held.
const FRESH_MS: f64 = 60_000.0;

/// Whether a snapshot asked for at `asked` can still be trusted at `now`, both in ms on
/// [`vectorcraft_ui_egui::now_ms`]'s clock: it is at most [`FRESH_MS`] old, and not stamped later
/// than `now`.
fn trusted(asked: f64, now: f64) -> bool {
    (0.0..=FRESH_MS).contains(&(now - asked))
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// What releases each lock this tab holds, by name: it resolves the promise the lock is held
    /// by (wasm runs on one thread).
    static RELEASE: RefCell<BTreeMap<String, Function>> = const { RefCell::new(BTreeMap::new()) };
}

/// The locks of the page, named `<prefix><area>`.
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct WebLocks {
    prefix: &'static str,
    /// The areas whose locks were held at the last snapshot, and when it was asked for (ms on
    /// [`vectorcraft_ui_egui::now_ms`]'s monotonic clock, which a change to the system clock does
    /// not move). `navigator.locks.query()` is asynchronous, so the snapshot is refreshed every
    /// [`REFRESH_MS`] and read from here.
    seen: Arc<Mutex<(f64, BTreeSet<String>)>>,
}

/// `navigator.locks`, if the browser has it here.
#[cfg(target_arch = "wasm32")]
fn manager() -> Option<JsValue> {
    let locks = Reflect::get(&web_sys::window()?.navigator(), &"locks".into()).ok()?;
    locks.is_object().then_some(locks)
}

/// Method `name` of `obj`.
#[cfg(target_arch = "wasm32")]
fn method(obj: &JsValue, name: &str) -> Option<Function> {
    Reflect::get(obj, &name.into()).ok()?.dyn_into().ok()
}

#[cfg(target_arch = "wasm32")]
impl WebLocks {
    /// The page's Web Locks with a first snapshot of them, kept fresh from then on; `None` when
    /// the browser has none or won't say which are held (heartbeats alone decide then).
    pub async fn start(prefix: &'static str) -> Option<Self> {
        let locks = Self { prefix, seen: Arc::new(Mutex::new((f64::NEG_INFINITY, BTreeSet::new()))) };
        locks.refresh().await?;
        let again = locks.clone();
        let tick = Closure::<dyn FnMut()>::new(move || {
            let locks = again.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // A failed refresh leaves the snapshot to age, and every area counts as held.
                let _ = locks.refresh().await;
            });
        });
        web_sys::window()?.set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), REFRESH_MS).ok()?;
        // The timer lives as long as the page.
        tick.forget();
        Some(locks)
    }

    /// Take a new snapshot of the locks held.
    async fn refresh(&self) -> Option<()> {
        let m = manager()?;
        let asked = vectorcraft_ui_egui::now_ms();
        let query: Promise = method(&m, "query")?.call0(&m).ok()?.dyn_into().ok()?;
        let snapshot = wasm_bindgen_futures::JsFuture::from(query).await.ok()?;
        let held = js_sys::Array::from(&Reflect::get(&snapshot, &"held".into()).ok()?);
        let areas = held
            .iter()
            .filter_map(|l| Reflect::get(&l, &"name".into()).ok()?.as_string())
            .filter_map(|n| n.strip_prefix(self.prefix).map(str::to_string))
            .collect();
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = (asked, areas);
        Some(())
    }

    /// Does a tab hold `area`'s lock? Yes when the last snapshot is too old to tell.
    pub fn held(&self, area: &str) -> bool {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        !trusted(seen.0, vectorcraft_ui_egui::now_ms()) || seen.1.contains(area)
    }

    /// Hold `area`'s lock until the returned hold is dropped (or the page goes).
    pub fn hold(&self, area: &str) -> Option<Hold> {
        let m = manager()?;
        let name = format!("{}{area}", self.prefix);
        // The lock is held until this promise resolves: when the hold is dropped.
        let mut release = None;
        let until = Promise::new(&mut |resolve, _| release = Some(resolve));
        let release = release?;
        let granted = Closure::once_into_js(move |_lock: JsValue| until);
        method(&m, "request")?.call2(&m, &name.clone().into(), &granted).ok()?;
        RELEASE.with(|r| r.borrow_mut().insert(name.clone(), release));
        Some(Box::new(LockHold(name)))
    }
}

/// A lock this tab holds (by name), released when dropped.
#[cfg(target_arch = "wasm32")]
struct LockHold(String);

#[cfg(target_arch = "wasm32")]
impl Drop for LockHold {
    fn drop(&mut self) {
        if let Some(resolve) = RELEASE.with(|r| r.borrow_mut().remove(&self.0)) {
            // Resolving can't fail in a way worth reporting: the lock goes with the page anyway.
            let _ = resolve.call0(&JsValue::NULL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_is_trusted_for_a_minute() {
        assert!(trusted(1_000.0, 1_000.0));
        assert!(trusted(1_000.0, 1_000.0 + FRESH_MS));
        assert!(!trusted(1_000.0, 1_001.0 + FRESH_MS));
        // Before the first snapshot every area counts as held.
        assert!(!trusted(f64::NEG_INFINITY, 0.0));
    }

    #[test]
    fn a_snapshot_stamped_later_than_now_is_not_trusted() {
        // A clock that went back reads an old snapshot as taken in the future.
        assert!(!trusted(5_000_000.0, 1_000.0));
        assert!(!trusted(1_001.0, 1_000.0));
    }
}
