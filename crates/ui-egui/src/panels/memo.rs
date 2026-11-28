//! What panels work out once per document version or library and keep while it lasts: brush
//! definitions and stroke-preview keys, symbol thumbnail keys.

use std::rc::Rc;
use std::sync::{Arc, Weak};

/// How many documents and libraries a [`Memo`] remembers (the oldest goes first).
const KEPT: usize = 8;

/// Values worked out from an `Arc<A>` (a document version, a library), each kept while that
/// allocation lives: one is known by its address, which a held [`Weak`] keeps from being reused,
/// and by `stamp` (a document's revision, which also changes when it is edited in place).
pub(crate) struct Memo<A, T> {
    entries: Vec<(Weak<A>, u64, Rc<T>)>,
}

impl<A, T> Default for Memo<A, T> {
    fn default() -> Self {
        Self { entries: vec![] }
    }
}

impl<A, T> Memo<A, T> {
    /// The value for `of` at `stamp`, built by `build` the first time it is asked for.
    pub(crate) fn get(&mut self, of: &Arc<A>, stamp: u64, build: impl FnOnce() -> T) -> Rc<T> {
        let at = Arc::as_ptr(of);
        if let Some(i) = self.entries.iter().position(|(w, s, _)| w.as_ptr() == at && *s == stamp) {
            let hit = self.entries.remove(i);
            let value = hit.2.clone();
            self.entries.push(hit);
            return value;
        }
        self.entries.retain(|(w, ..)| w.strong_count() > 0 && w.as_ptr() != at);
        if self.entries.len() >= KEPT {
            self.entries.remove(0);
        }
        let value = Rc::new(build());
        self.entries.push((Arc::downgrade(of), stamp, value.clone()));
        value
    }
}
