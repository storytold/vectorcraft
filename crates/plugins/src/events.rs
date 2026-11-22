//! Events a plug-in may react to, from plug-in API level 2 on: its manifest's `events` names them
//! ([`Event`]) and lists the `events` permission, and the module exports
//! `vc_event(event_ptr: i32, event_len: i32) -> i32`. After a change has finished, the host calls
//! `vc_event` in a fresh instance with a small JSON object that names the event and the document
//! ([`crate::Plugin::run_event`]). A handler only reads: the host applies nothing it returns, and
//! a command call that would change a document is refused ([`crate::host`]).
//!
//! The limits are VectorCraft's own values: [`EVENT_FUEL`] instructions and
//! [`EVENT_WALL_TIME_MS`] for one handler, an event of at most [`MAX_EVENT_BYTES`], and
//! [`MAX_EVENT_IDS`] selected ids in a `selection.changed` event.

/// The plug-in API level that adds `events`, the `events` permission and `vc_event`.
pub const EVENTS_API: u32 = 2;

/// Instructions one handler may use, or the plug-in's own budget when that is smaller.
pub const EVENT_FUEL: u64 = 5_000_000;

/// Wall-clock budget of one handler, in milliseconds, or the plug-in's own when that is shorter.
pub const EVENT_WALL_TIME_MS: u64 = 200;

/// Largest event JSON the host hands a handler, in bytes.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

/// Most selected object ids a `selection.changed` event lists; a larger selection sends the first
/// ones with `truncated: true` and its whole `count`. That many ids of the largest value still fit
/// in [`MAX_EVENT_BYTES`].
pub const MAX_EVENT_IDS: usize = 2048;

/// Failed handlers in a row after which the host stops calling a plug-in's handler until the app
/// starts again.
pub const MAX_EVENT_FAILURES: u32 = 3;

/// One thing a plug-in may react to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    /// The selection in a document changed: other objects, anchors or segments, or nothing.
    SelectionChanged,
    /// A file opened as a document (File › Open, a recent file, New from Template).
    DocumentOpened,
    /// A document was saved to its own file (Save, Save As).
    DocumentSaved,
    /// The current layer changed, or a layer was added, deleted, moved or given other options.
    LayerChanged,
}

impl Event {
    /// Every event, in the order the docs list them.
    pub const ALL: [Event; 4] = [Event::SelectionChanged, Event::DocumentOpened, Event::DocumentSaved, Event::LayerChanged];

    /// Its name in a manifest's `events`, which is also the event's `type`.
    pub fn name(self) -> &'static str {
        match self {
            Event::SelectionChanged => "selection.changed",
            Event::DocumentOpened => "document.opened",
            Event::DocumentSaved => "document.saved",
            Event::LayerChanged => "layer.changed",
        }
    }

    /// The event named `name`.
    pub fn parse(name: &str) -> Option<Event> {
        Event::ALL.into_iter().find(|e| e.name() == name)
    }

    /// Whether the event is still delivered when its document has closed before the delivery:
    /// an open or a save is, a selection or layer change is not.
    pub fn outlives_document(self) -> bool {
        matches!(self, Event::DocumentOpened | Event::DocumentSaved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for e in Event::ALL {
            assert_eq!(Event::parse(e.name()), Some(e));
        }
        assert_eq!(Event::parse("selection.change"), None);
        assert_eq!(Event::parse(""), None);
    }

    /// A `selection.changed` event that lists the most ids, each of the largest value, fits in the
    /// largest event a handler is given.
    #[test]
    fn the_largest_selection_event_fits() {
        let ids = vec![u64::MAX; MAX_EVENT_IDS];
        let event = serde_json::json!({"type": "selection.changed", "document": u64::MAX, "count": u64::MAX, "ids": ids, "truncated": true});
        let len = serde_json::to_vec(&event).unwrap().len();
        assert!(len <= MAX_EVENT_BYTES, "{len} bytes");
    }
}
