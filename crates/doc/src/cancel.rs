//! Cooperative cancellation and progress for long-running work (exports, flattening, baking).
//!
//! A host that wants to be able to stop a call (the MCP server, on `notifications/cancelled`)
//! runs it inside [`scope`] with a [`CancelToken`]. Code deep below, which has no handle on the
//! host, calls [`check`] (or [`is_cancelled`] where it can't return an error) at points where
//! stopping is safe, and returns [`Cancelled`] up through the same `Result`s it already has. The
//! token is tied to the thread that entered the scope, so concurrent sessions on other threads
//! (and tests running in parallel) never see each other's cancellation, and code outside any
//! scope is never cancelled.
//!
//! Work that stops early leaves nothing behind: callers drop whatever was half built and report
//! the error, so a cancelled command changes nothing.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A flag one thread raises and another polls. Cloning shares the flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask for the work holding this token to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The error of work that stopped because it was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Receives progress: `(done, total, what is being done)`. `total` is 0 when it isn't known.
pub type ProgressSink = Rc<dyn Fn(u64, u64, &str)>;

struct Scope {
    token: CancelToken,
    sink: Option<ProgressSink>,
}

thread_local! {
    static CURRENT: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// Puts the previous scope back, also when the work inside unwinds.
struct Restore(Option<Scope>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        // `try_with`: during thread teardown the slot may be gone; nothing to restore then.
        let _ = CURRENT.try_with(|c| *c.borrow_mut() = previous);
    }
}

/// Run `f` with `token` (and an optional progress `sink`) in effect on this thread.
pub fn scope<T>(token: &CancelToken, sink: Option<ProgressSink>, f: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|c| c.borrow_mut().replace(Scope { token: token.clone(), sink }));
    let _restore = Restore(previous);
    f()
}

/// Has the work running on this thread been cancelled? Always false outside a [`scope`].
pub fn is_cancelled() -> bool {
    CURRENT.with(|c| c.borrow().as_ref().is_some_and(|s| s.token.is_cancelled()))
}

/// Is this thread running inside a [`scope`], so that a cancel can reach it?
pub fn is_cancellable() -> bool {
    CURRENT.with(|c| c.borrow().is_some())
}

/// `Err(Cancelled)` when the work running on this thread has been cancelled.
pub fn check() -> Result<(), Cancelled> {
    if is_cancelled() { Err(Cancelled) } else { Ok(()) }
}

/// Run `f` with this thread's token still in effect and its progress reports dropped: for work
/// that reports its own steps around calls that report theirs (each file of a batch export).
pub fn quiet<T>(f: impl FnOnce() -> T) -> T {
    match CURRENT.with(|c| c.borrow().as_ref().map(|s| s.token.clone())) {
        Some(token) => scope(&token, None, f),
        None => f(),
    }
}

/// Report that `done` of `total` steps (0: unknown) are finished. A no-op outside a [`scope`]
/// that has a sink.
pub fn progress(done: u64, total: u64, what: &str) {
    // The sink runs outside the borrow, so it may itself call back into this module.
    let sink = CURRENT.with(|c| c.borrow().as_ref().and_then(|s| s.sink.clone()));
    if let Some(sink) = sink {
        sink(done, total, what);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_work_inside_the_scope_is_cancelled() {
        let t = CancelToken::new();
        assert_eq!(check(), Ok(()));
        scope(&t, None, || {
            assert_eq!(check(), Ok(()));
            t.cancel();
            assert_eq!(check(), Err(Cancelled));
            // A nested scope with its own token shadows, then restores.
            scope(&CancelToken::new(), None, || assert_eq!(check(), Ok(())));
            assert!(is_cancelled());
        });
        assert!(!is_cancelled(), "the scope ended");
    }

    #[test]
    fn progress_reaches_the_sink_and_the_scope_survives_a_panic() {
        let seen = Rc::new(RefCell::new(vec![]));
        let sink: ProgressSink = {
            let seen = seen.clone();
            Rc::new(move |d, t, w| seen.borrow_mut().push((d, t, w.to_string())))
        };
        let t = CancelToken::new();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scope(&t, Some(sink), || {
                progress(1, 3, "page");
                panic!("boom");
            })
        }));
        assert!(r.is_err());
        progress(2, 3, "after");
        assert_eq!(*seen.borrow(), vec![(1, 3, "page".to_string())]);
    }

    #[test]
    fn quiet_work_reports_nothing_and_can_still_be_cancelled() {
        let seen = Rc::new(RefCell::new(vec![]));
        let sink: ProgressSink = {
            let seen = seen.clone();
            Rc::new(move |d, t, w| seen.borrow_mut().push((d, t, w.to_string())))
        };
        let t = CancelToken::new();
        scope(&t, Some(sink), || {
            progress(0, 2, "file");
            quiet(|| {
                progress(0, 1, "artboard");
                t.cancel();
                assert_eq!(check(), Err(Cancelled));
            });
            progress(1, 2, "file");
        });
        assert_eq!(*seen.borrow(), vec![(0, 2, "file".to_string()), (1, 2, "file".to_string())]);
        assert_eq!(quiet(check), Ok(()), "outside a scope nothing is cancelled");
    }
}
