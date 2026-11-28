//! Holding the installed plug-ins and the deferred stages' results steady while a test counts what
//! a cache keyed on [`crate::registry::revision`] and [`crate::deferred::revision`] keeps: tests run
//! in parallel, and a count must not include a refresh that another test's install caused.

/// Who holds [`SteadyPlugins`] and who changes the revisions ([`plugin_change`]), guarded by
/// [`steadiness`] and signaled through [`STEADINESS_CHANGED`].
struct Steadiness {
    /// Threads that hold a [`SteadyPlugins`].
    holders: usize,
    /// Changes waiting to start.
    waiting: usize,
    /// Of `waiting`, those made on a thread that holds a [`SteadyPlugins`].
    holders_waiting: usize,
    /// Whether a change is under way.
    changing: bool,
}

static STEADINESS: std::sync::Mutex<Steadiness> = std::sync::Mutex::new(Steadiness { holders: 0, waiting: 0, holders_waiting: 0, changing: false });
static STEADINESS_CHANGED: std::sync::Condvar = std::sync::Condvar::new();

fn steadiness() -> std::sync::MutexGuard<'static, Steadiness> {
    STEADINESS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Wait for [`STEADINESS_CHANGED`].
fn await_steadiness(s: std::sync::MutexGuard<'static, Steadiness>) -> std::sync::MutexGuard<'static, Steadiness> {
    STEADINESS_CHANGED.wait(s).unwrap_or_else(std::sync::PoisonError::into_inner)
}

std::thread_local! {
    /// How many [`SteadyPlugins`] this thread holds.
    static STEADY: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Whether this thread holds the [`PluginChange`] under way.
    static CHANGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// While held, [`crate::registry::revision`] and [`crate::deferred::revision`] change only on this
/// thread: an install or a removal on another thread waits before it takes the plug-ins' lock, and
/// a deferred stage that gives its result on another thread waits before the revision moves. A
/// change on this thread waits until no other thread holds one, unless every other thread that
/// holds one is itself waiting to make a change. The guards one thread holds count as one, until
/// the last of them is dropped, in whatever order they are.
pub struct SteadyPlugins {
    /// Dropped on the thread that made it.
    _thread: std::marker::PhantomData<*const ()>,
}

impl Drop for SteadyPlugins {
    fn drop(&mut self) {
        let last = STEADY.with(|n| {
            n.set(n.get() - 1);
            n.get() == 0
        });
        if last {
            steadiness().holders -= 1;
            STEADINESS_CHANGED.notify_all();
        }
    }
}

/// [`SteadyPlugins`]. A thread's first guard waits for a change under way, and for a waiting change
/// that can start now. It does not wait for a change that is waiting for guards other threads hold:
/// the thread waiting for this guard may hold one of them.
pub fn steady_plugins() -> SteadyPlugins {
    if STEADY.with(std::cell::Cell::get) == 0 {
        let mut s = steadiness();
        let waits = |s: &Steadiness| !CHANGING.with(std::cell::Cell::get) && (s.changing || (s.waiting > 0 && s.holders == s.holders_waiting));
        while waits(&s) {
            s = await_steadiness(s);
        }
        s.holders += 1;
    }
    STEADY.with(|n| n.set(n.get() + 1));
    SteadyPlugins { _thread: std::marker::PhantomData }
}

/// Held while the plug-ins or the deferred stages' results change, from before the change is made
/// until its revision has moved ([`plugin_change`]).
pub(crate) struct PluginChange {
    /// Made while this thread already held one.
    nested: bool,
}

impl Drop for PluginChange {
    fn drop(&mut self) {
        if !self.nested {
            CHANGING.with(|c| c.set(false));
            steadiness().changing = false;
            STEADINESS_CHANGED.notify_all();
        }
    }
}

/// [`PluginChange`]: waits until no other change is under way and no other thread holds a
/// [`SteadyPlugins`] (on a thread that holds one: none that is not itself waiting to make a change).
/// Inside a change this thread is making, it does not wait.
pub(crate) fn plugin_change() -> PluginChange {
    if CHANGING.with(std::cell::Cell::get) {
        return PluginChange { nested: true };
    }
    let holder = STEADY.with(std::cell::Cell::get) > 0;
    let mut s = steadiness();
    #[cfg(test)]
    changes_asked().push(std::thread::current().id());
    s.waiting += 1;
    s.holders_waiting += usize::from(holder);
    let waits = |s: &Steadiness| s.changing || if holder { s.holders > s.holders_waiting } else { s.holders > 0 };
    while waits(&s) {
        s = await_steadiness(s);
    }
    s.waiting -= 1;
    s.holders_waiting -= usize::from(holder);
    s.changing = true;
    CHANGING.with(|c| c.set(true));
    PluginChange { nested: false }
}

/// The threads that have asked to make a change ([`plugin_change`]).
#[cfg(test)]
fn changes_asked() -> std::sync::MutexGuard<'static, Vec<std::thread::ThreadId>> {
    static ASKED: std::sync::Mutex<Vec<std::thread::ThreadId>> = std::sync::Mutex::new(Vec::new());
    ASKED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Waits until the thread `id` has asked to make a change, or `finished` says it ended.
#[cfg(test)]
pub(crate) fn asked(id: std::thread::ThreadId, finished: impl Fn() -> bool) {
    while !finished() && !changes_asked().contains(&id) {
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// While a thread holds [`steady_plugins`], a removal on another thread waits until the last of
    /// the thread's guards is dropped, whichever was made first.
    #[test]
    fn steady_plugins_hold_back_other_threads_changes() {
        let outer = steady_plugins();
        let inner = steady_plugins();
        std::thread::scope(|scope| {
            let removing = scope.spawn(|| crate::registry::remove("steady.test.none"));
            asked(removing.thread().id(), || removing.is_finished());
            assert!(!removing.is_finished(), "the other thread's change waits for the guard");
            drop(outer);
            assert!(!removing.is_finished(), "the inner guard is still held");
            drop(inner);
            assert!(!removing.join().unwrap_or(true), "nothing was installed under the id");
        });
    }
}
