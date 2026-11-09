//! Small native invalidation stamps over the shared bounded command bus (#431).
//! No operation replay, document serialization, custom queue or disk access.
use craft_command::{Cause, Change, ChangeBus};
use serde_json::{Value, json};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq)]
pub struct Stamp {
    pub document: u64,
    pub revision: u64,
}

#[derive(Default)]
pub struct DocumentChanges {
    bus: ChangeBus<Stamp>,
    sequence: Mutex<u64>,
    wake: Condvar,
}

impl DocumentChanges {
    pub fn latest(&self) -> u64 {
        *self.sequence.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn publish(&self, document: u64, revision: u64, command: &str) {
        let mut sequence = self.sequence.lock().unwrap_or_else(PoisonError::into_inner);
        *sequence = sequence.saturating_add(1);
        let cause = match command {
            "edit.undo" => Cause::Undo,
            "edit.redo" => Cause::Redo,
            _ => Cause::Untracked,
        };
        self.bus.publish(Change { revision: *sequence, cause, merged: false, snapshot: Stamp { document, revision }, prev: None });
        drop(sequence);
        self.wake.notify_all();
    }

    /// Coalesced invalidation, not native history. Gaps/restarts request reinspection.
    /// The temporary subscriber is bounded by the existing native connection cap.
    pub fn poll(&self, after: u64, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout.min(Duration::from_secs(30));
        let subscriber = self.bus.subscribe();
        let mut sequence = self.sequence.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let (changes, lost) = self.bus.drain_with_lost(subscriber);
            if *sequence != after || !changes.is_empty() || Instant::now() >= deadline {
                let cursor = *sequence;
                let gap = changes.first().is_none_or(|c| c.revision > after.saturating_add(1));
                let reset = after != cursor && gap || lost > 0 || after > cursor;
                let events: Vec<_> = changes
                    .into_iter()
                    .map(|c| {
                        json!({
                            "seq":c.revision,"kind":"document.changed",
                            "data":{"documentId":c.snapshot.document,"revision":c.snapshot.revision}
                        })
                    })
                    .collect();
                self.bus.unsubscribe(subscriber);
                return json!({"events":events,"cursor":cursor,"reset":reset,"dropped":lost});
            }
            sequence = match self.wake.wait_timeout(sequence, deadline.saturating_duration_since(Instant::now())) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    pub fn request(&self, params: &Value) -> Value {
        let after = params.get("after").map_or(Some(0), Value::as_u64);
        let timeout = params.get("timeoutMs").map_or(Some(20000), Value::as_u64);
        match (after, timeout) {
            (Some(after), Some(timeout)) => json!({"ok":true,"result":self.poll(after, Duration::from_millis(timeout.min(30000)))}),
            _ => json!({"ok":false,"error":"after and timeoutMs must be non-negative integers"}),
        }
    }
}
