//! Events plug-ins react to ([`vectorcraft_plugins::events`]): the session notices a changed
//! selection, current layer or set of layers by comparing each open document with how it was at the
//! last check, and hears of each successful open and save ([`Session::note_plugin_event`]). It then
//! calls the `vc_event` handler of each installed plug-in whose manifest names the event, after the
//! change has finished and outside any undo step.
//!
//! - A check runs after each top-level command, or when a frontend that sets
//!   [`Session::plugin_event_frames`] delivers ([`Session::deliver_plugin_events`]), by the frame.
//!   Changes between two checks are one event: a burst of selection changes is one
//!   `selection.changed` with the final selection. No check or delivery runs while an interaction
//!   is open (a drag being previewed); waiting events wait for it to end. A document opened or
//!   created since the last check is compared with how it was when the command that added it ended
//!   ([`Session::plugin_events_see_new_documents`]).
//! - At most [`MAX_PENDING`] events wait; past that the oldest is dropped. An event raised while a
//!   handler runs waits for the next delivery: handlers never run inside one another.
//! - A handler only reads. The host applies nothing it returns, and it may call only the commands
//!   in [`super::cmd::plugin_call::EVENT_READS`]; any other call is refused with an error that names
//!   the event, and what a call changes is put back ([`super::cmd::plugin_call::event_call`]).
//! - A handler that fails is reported once a streak; after
//!   [`vectorcraft_plugins::events::MAX_EVENT_FAILURES`] failures in a row the plug-in's events stop
//!   until the session ends, or until another module is installed under its id.
//! - When no installed plug-in reacts to events, a check returns before comparing anything.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, Selection};
use vectorcraft_plugins::events::{Event, MAX_EVENT_FAILURES, MAX_EVENT_IDS};
use vectorcraft_plugins::permissions::Permission;
use vectorcraft_plugins::{LogLevel, Plugin, registry};

use crate::Session;

/// Most events that wait for delivery.
pub const MAX_PENDING: usize = 32;

/// Most notices kept for the frontend to show; older ones are dropped.
pub const MAX_NOTICES: usize = 64;

/// The reasons a `layer.changed` event gives, in the order it lists them.
pub const LAYER_REASONS: [&str; 5] = ["current", "added", "deleted", "reordered", "options"];

/// A document as the last check saw it.
struct Seen {
    uid: u64,
    doc: Arc<Document>,
    selection: Selection,
    layer: Option<NodeId>,
}

/// A plug-in an event is for: its id, the hash of its module and its installation
/// ([`Plugin::serial`]), so a module removed and installed again is another recipient.
type Recipient = (String, String, u64);

/// The recipient `p` is.
fn recipient(p: &Plugin) -> Recipient {
    (p.id().to_string(), p.hash().to_string(), p.serial())
}

/// An event waiting for delivery: what changed, in which document, and the plug-ins it is for
/// (those installed when it happened), each with the reasons of a `layer.changed` it was given.
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    event: Event,
    document: u64,
    to: Vec<(Recipient, Vec<&'static str>)>,
    /// When it was queued ([`PluginEvents::queued`] then).
    seq: u64,
    /// Queued by a command of the running script, whose run has not ended yet
    /// ([`Session::plugin_events_script_ended`]).
    by_script: bool,
}

impl Pending {
    /// Whether this event is for `r`.
    fn is_for(&self, r: &Recipient) -> bool {
        self.to.iter().any(|(x, _)| x == r)
    }

    /// The reasons of a `layer.changed` that `r` is given.
    fn reasons_for(&self, r: &Recipient) -> &[&'static str] {
        self.to.iter().find(|(x, _)| x == r).map_or(&[], |(_, reasons)| reasons.as_slice())
    }
}

/// One plug-in's failed handlers in a row, for the module with `hash`.
#[derive(Clone, Debug, Default)]
struct Failures {
    hash: String,
    count: u32,
    disabled: bool,
    last_error: Option<String>,
}

/// A line a handler wrote with `vc_log`, or a handler's failure, for the frontend to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// The plug-in's id and name.
    pub plugin: String,
    pub name: String,
    /// The event its handler was given.
    pub event: &'static str,
    pub level: LogLevel,
    /// The text, in English for a failure.
    pub text: String,
}

/// The session's plug-in event state.
#[derive(Default)]
pub struct PluginEvents {
    seen: Vec<Seen>,
    /// The plug-ins that reacted to events when `seen` was taken.
    seen_by: Vec<Recipient>,
    pending: VecDeque<Pending>,
    /// How many events have been queued as their own entry.
    queued: u64,
    dispatching: bool,
    failures: BTreeMap<String, Failures>,
    notices: VecDeque<Notice>,
    /// The installed plug-ins that react to events, as of a registry revision.
    subscribers: Option<(u64, Vec<Arc<Plugin>>)>,
}

impl PluginEvents {
    /// The installed plug-ins that react to events (none in Safe Mode).
    fn subscribers(&mut self) -> Vec<Arc<Plugin>> {
        if vectorcraft_plugins::safe_mode::active() {
            return Vec::new();
        }
        let revision = registry::revision();
        if let Some((at, subs)) = &self.subscribers
            && *at == revision
        {
            return subs.clone();
        }
        let subs: Vec<Arc<Plugin>> = registry::list().into_iter().filter(|p| !p.manifest().events.is_empty()).collect();
        self.subscribers = Some((revision, subs.clone()));
        subs
    }

    /// Queues `event` for document `uid`, for the plug-ins `to`, with the reasons of a
    /// `layer.changed`. A `selection.changed` or `layer.changed` already waiting for that document
    /// takes it in: each plug-in in `to` is added, or adds the new reasons to the ones it was given,
    /// so a plug-in is given only the reasons of the changes it was given. Past [`MAX_PENDING`] the
    /// oldest event is dropped.
    fn queue(&mut self, event: Event, uid: u64, reasons: Vec<&'static str>, to: Vec<Recipient>) {
        if matches!(event, Event::SelectionChanged | Event::LayerChanged)
            && let Some(p) = self.pending.iter_mut().find(|p| p.event == event && p.document == uid)
        {
            for r in to {
                let i = match p.to.iter().position(|(x, _)| *x == r) {
                    Some(i) => i,
                    None => {
                        p.to.push((r, Vec::new()));
                        p.to.len() - 1
                    }
                };
                let given = &mut p.to[i].1;
                for reason in &reasons {
                    if !given.contains(reason) {
                        given.push(*reason);
                    }
                }
                given.sort_by_key(|r| LAYER_REASONS.iter().position(|x| x == r));
            }
            return;
        }
        if self.pending.len() >= MAX_PENDING
            && let Some(dropped) = self.pending.pop_front()
        {
            log::warn!("plug-in events: dropped a waiting {} event, past {MAX_PENDING} waiting", dropped.event.name());
        }
        let to = to.into_iter().map(|r| (r, reasons.clone())).collect();
        self.pending.push_back(Pending { event, document: uid, to, seq: self.queued, by_script: false });
        self.queued += 1;
    }

    fn notice(&mut self, n: Notice) {
        if self.notices.len() >= MAX_NOTICES {
            self.notices.pop_front();
        }
        self.notices.push_back(n);
    }

    /// The failures of plug-in `p`, started again when another module was installed under its id.
    fn failures_of(&mut self, p: &Plugin) -> &mut Failures {
        let f = self.failures.entry(p.id().to_string()).or_default();
        if f.hash != p.hash() {
            *f = Failures { hash: p.hash().to_string(), ..Failures::default() };
        }
        f
    }
}

/// A layer or sublayer: its id, its parent (`None` at the top), the layer itself without its
/// contents (for comparing its options) and the ids of the rows directly in it, in order.
type LayerRow = (NodeId, Option<NodeId>, vectorcraft_doc::Node, Vec<NodeId>);

/// Each layer and sublayer of `doc`, in order.
fn layer_rows(doc: &Document) -> Vec<LayerRow> {
    let mut out = Vec::new();
    let mut stack: Vec<(&Arc<vectorcraft_doc::Node>, Option<NodeId>)> = doc.layers.iter().rev().map(|l| (l, None)).collect();
    while let Some((n, parent)) = stack.pop() {
        let NodeKind::Layer { children, .. } = &n.kind else { continue };
        stack.extend(children.iter().rev().filter(|c| c.is_layer()).map(|c| (c, Some(n.id))));
        let rows = children.iter().map(|c| c.id).collect();
        let mut bare = (**n).clone();
        if let Some(c) = bare.children_mut() {
            c.clear();
        }
        out.push((n.id, parent, bare, rows));
    }
    out
}

/// Whether a layer's place among the rows beside it differs between `old` (of `before`) and `new`
/// (of `after`), counting only the rows on both sides, so an object added or deleted next to a
/// sublayer is not a move.
fn layers_moved_among_rows(before: &Document, old: &[LayerRow], after: &Document, new: &[LayerRow]) -> bool {
    use std::collections::{HashMap, HashSet};
    let layers: HashSet<NodeId> = old.iter().map(|r| r.0).collect();
    let lists = |doc: &Document, rows: &[LayerRow]| {
        let mut m: HashMap<Option<NodeId>, Vec<NodeId>> = rows.iter().map(|r| (Some(r.0), r.3.clone())).collect();
        m.insert(None, doc.layers.iter().map(|l| l.id).collect());
        m
    };
    let (a, b) = (lists(before, old), lists(after, new));
    a.iter().any(|(parent, xs)| {
        let Some(ys) = b.get(parent) else { return false };
        let (in_x, in_y): (HashSet<NodeId>, HashSet<NodeId>) = (xs.iter().copied().collect(), ys.iter().copied().collect());
        let places = |list: &[NodeId], other: &HashSet<NodeId>| {
            list.iter().filter(|id| other.contains(id)).enumerate().filter(|(_, id)| layers.contains(id)).map(|(i, id)| (i, *id)).collect::<Vec<_>>()
        };
        places(xs, &in_y) != places(ys, &in_x)
    })
}

/// Why the layers of `after` (with current layer `layer_after`) differ from those of `before`
/// (with `layer_before`), in [`LAYER_REASONS`] order; empty when they don't.
fn layer_reasons(before: &Arc<Document>, layer_before: Option<NodeId>, after: &Arc<Document>, layer_after: Option<NodeId>) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if layer_before != layer_after {
        reasons.push("current");
    }
    if Arc::ptr_eq(before, after) {
        return reasons;
    }
    let (old, new) = (layer_rows(before), layer_rows(after));
    if new.iter().any(|(id, ..)| !old.iter().any(|(o, ..)| o == id)) {
        reasons.push("added");
    }
    if old.iter().any(|(id, ..)| !new.iter().any(|(n, ..)| n == id)) {
        reasons.push("deleted");
    }
    let kept =
        |rows: &[LayerRow], other: &[LayerRow]| rows.iter().filter(|r| other.iter().any(|o| o.0 == r.0)).map(|r| (r.0, r.1)).collect::<Vec<_>>();
    if kept(&old, &new) != kept(&new, &old) || layers_moved_among_rows(before, &old, after, &new) {
        reasons.push("reordered");
    }
    if old.iter().any(|r| new.iter().any(|o| o.0 == r.0 && o.2 != r.2)) {
        reasons.push("options");
    }
    reasons
}

impl Session {
    /// Hears that `event` (`document.opened` or `document.saved`) happened to document `uid`, which
    /// the next delivery hands the plug-ins that react to it. Nothing is kept when none does.
    pub(crate) fn note_plugin_event(&mut self, event: Event, uid: u64) {
        let to: Vec<Recipient> = self.plugin_events.subscribers().iter().filter(|p| p.manifest().reacts_to(event)).map(|p| recipient(p)).collect();
        if to.is_empty() {
            return;
        }
        self.plugin_events.queue(event, uid, Vec::new(), to);
        // An open or a save is queued as its own entry, the newest.
        if self.script_in_run
            && let Some(p) = self.plugin_events.pending.back_mut()
        {
            p.by_script = true;
        }
    }

    /// At the end of a script's run. When the run failed (`kept` false), drops the waiting
    /// `document.opened` events its commands queued, since the documents they opened were discarded
    /// with its working copy. Its saves stay, since the files they wrote stay written, and so do the
    /// events of documents a frontend opened while the run was paused.
    pub(crate) fn plugin_events_script_ended(&mut self, kept: bool) {
        if !kept {
            self.plugin_events.pending.retain(|p| !(p.by_script && p.event == Event::DocumentOpened));
        }
        for p in &mut self.plugin_events.pending {
            p.by_script = false;
        }
    }

    /// A mark for [`Session::forget_plugin_opens_since`]: the events queued so far.
    pub(crate) fn plugin_events_mark(&self) -> u64 {
        self.plugin_events.queued
    }

    /// Drops the waiting `document.opened` events queued since `mark`, for opens that a rollback
    /// undid (a failed `command.batch`, or a failed command of a script or a plug-in call). The
    /// saves stay, since the files they wrote stay written.
    pub(crate) fn forget_plugin_opens_since(&mut self, mark: u64) {
        self.plugin_events.pending.retain(|p| p.event != Event::DocumentOpened || p.seq < mark);
    }

    /// Compares each open document with how the last check saw it and queues the events its changes
    /// give the installed plug-ins: `selection.changed` and `layer.changed`. A document neither the
    /// last check nor [`Session::plugin_events_see_new_documents`] saw gives none, and a plug-in
    /// installed since the last check is given none: the
    /// change may have come before it. A waiting event is no longer for a plug-in that is not
    /// installed now, so a module removed and installed again is not given it. Nothing is compared
    /// while an interaction is open, nor when no plug-in reacts to events.
    fn check_plugin_events(&mut self) {
        let subs = self.plugin_events.subscribers();
        if subs.is_empty() {
            self.plugin_events.seen.clear();
            self.plugin_events.seen_by.clear();
            self.plugin_events.pending.clear();
            return;
        }
        let installed: Vec<Recipient> = subs.iter().map(|p| recipient(p)).collect();
        self.plugin_events.pending.retain_mut(|p| {
            p.to.retain(|(r, _)| installed.contains(r));
            !p.to.is_empty()
        });
        if self.docs.iter().any(|d| d.interaction.is_some()) {
            return;
        }
        let mut events = std::mem::take(&mut self.plugin_events);
        let before = std::mem::take(&mut events.seen);
        let before_by = std::mem::take(&mut events.seen_by);
        let to = |e: Event| -> Vec<Recipient> {
            subs.iter().filter(|p| p.manifest().reacts_to(e)).map(|p| recipient(p)).filter(|r| before_by.contains(r)).collect()
        };
        let (selection, layers) = (to(Event::SelectionChanged), to(Event::LayerChanged));
        let mut seen = Vec::with_capacity(self.docs.len());
        for st in &self.docs {
            if let Some(old) = before.iter().find(|s| s.uid == st.uid) {
                if !selection.is_empty() && old.selection != st.selection {
                    events.queue(Event::SelectionChanged, st.uid, Vec::new(), selection.clone());
                }
                if !layers.is_empty() {
                    let reasons = layer_reasons(&old.doc, old.layer, &st.doc, st.active_layer);
                    if !reasons.is_empty() {
                        events.queue(Event::LayerChanged, st.uid, reasons, layers.clone());
                    }
                }
            }
            seen.push(Seen { uid: st.uid, doc: st.doc.clone(), selection: st.selection.clone(), layer: st.active_layer });
        }
        events.seen = seen;
        events.seen_by = subs.iter().map(|p| recipient(p)).collect();
        events.pending.retain(|p| p.event.outlives_document() || self.docs.iter().any(|d| d.uid == p.document));
        self.plugin_events = events;
    }

    /// Checks the open documents for changes ([`Session::check_plugin_events`]) and calls the
    /// handlers of the events waiting, oldest first, one handler at a time. While a handler runs,
    /// this returns at once: what the handler's calls change waits for the next delivery. While an
    /// interaction is open, the events wait for the first delivery after it ends.
    pub fn deliver_plugin_events(&mut self) {
        if self.plugin_events.dispatching {
            return;
        }
        self.check_plugin_events();
        if self.plugin_events.pending.is_empty() || self.docs.iter().any(|d| d.interaction.is_some()) {
            return;
        }
        let subs = self.plugin_events.subscribers();
        self.plugin_events.dispatching = true;
        // The commands a handler calls run nested, never journaled; a panic in one leaves the depth
        // as it was found.
        let depth = self.depth;
        self.depth += 1;
        while let Some(p) = self.plugin_events.pending.pop_front() {
            if !p.event.outlives_document() && !self.docs.iter().any(|d| d.uid == p.document) {
                continue;
            }
            for plugin in subs.iter().filter(|s| s.manifest().reacts_to(p.event) && p.is_for(&recipient(s))) {
                if self.plugin_events.failures_of(plugin).disabled {
                    continue;
                }
                self.run_handler(plugin, &p);
            }
        }
        self.depth = depth;
        self.plugin_events.dispatching = false;
    }

    /// The JSON event `p` as plug-in `plugin` is given it: the selected object ids of a
    /// `selection.changed` only with `doc.read`.
    fn event_json(&self, plugin: &Plugin, p: &Pending) -> Value {
        let mut v = json!({ "type": p.event.name(), "document": p.document });
        let Some(st) = self.docs.iter().find(|d| d.uid == p.document) else { return v };
        match p.event {
            Event::SelectionChanged => {
                let objects = &st.selection.objects;
                v["count"] = json!(objects.len());
                v["truncated"] = json!(false);
                v["ids"] = Value::Null;
                if plugin.manifest().permissions().contains(&Permission::DocRead) {
                    v["ids"] = json!(objects.iter().take(MAX_EVENT_IDS).map(|i| i.0).collect::<Vec<_>>());
                    v["truncated"] = json!(objects.len() > MAX_EVENT_IDS);
                }
            }
            Event::LayerChanged => {
                v["layer"] = json!(st.active_layer.map(|l| l.0));
                v["reasons"] = json!(p.reasons_for(&recipient(plugin)));
            }
            Event::DocumentOpened | Event::DocumentSaved => {}
        }
        v
    }

    /// Calls `plugin`'s handler with event `p` and records what it wrote and whether it failed.
    fn run_handler(&mut self, plugin: &Arc<Plugin>, p: &Pending) {
        let event = p.event.name();
        let text = serde_json::to_vec(&self.event_json(plugin, p)).unwrap_or_default();
        let m = plugin.manifest();
        let may = m.permissions();
        let (r, report) = if may.contains(&Permission::Commands) && !m.command_allowlist.is_empty() {
            let calls = crate::cmd::plugin_call::Calls::of(self, may.contains(&Permission::DocRead), false);
            let mut bridge = |id: &str, args: &Value| crate::cmd::plugin_call::event_call(self, &calls, event, id, args);
            plugin.run_event(&text, Some(&mut bridge))
        } else {
            plugin.run_event(&text, None)
        };
        let (id, name) = (plugin.id().to_string(), m.name.clone());
        for line in report.lines {
            self.plugin_events.notice(Notice { plugin: id.clone(), name: name.clone(), event, level: line.level, text: line.text });
        }
        let failures = self.plugin_events.failures_of(plugin);
        let e = match r {
            Ok(()) => {
                failures.count = 0;
                return;
            }
            Err(e) => e.to_string(),
        };
        failures.count += 1;
        failures.last_error = Some(e.clone());
        let (count, first) = (failures.count, failures.count == 1);
        log::warn!("plug-in {id}: its {event} handler failed: {e}");
        if first {
            let text = format!("its {event} handler failed: {e}");
            self.plugin_events.notice(Notice { plugin: id.clone(), name: name.clone(), event, level: LogLevel::Error, text });
        }
        if count >= MAX_EVENT_FAILURES {
            self.plugin_events.failures_of(plugin).disabled = true;
            let text = format!("its events are off until VectorCraft starts again, after {count} failed handlers in a row");
            self.plugin_events.notice(Notice { plugin: id, name, event, level: LogLevel::Error, text });
        }
    }

    /// Takes each open document the last check didn't see, one opened or created since, as it is
    /// now, so the next check compares it with this state: a change made to it before the next
    /// delivery gives its event. Called after each top-level command and when a document is added
    /// outside one. Nothing is kept when no plug-in reacted to events at the last check, nor for a
    /// document in an interaction.
    pub(crate) fn plugin_events_see_new_documents(&mut self) {
        if self.plugin_events.seen_by.is_empty() {
            return;
        }
        for st in &self.docs {
            if st.interaction.is_none() && !self.plugin_events.seen.iter().any(|s| s.uid == st.uid) {
                self.plugin_events.seen.push(Seen { uid: st.uid, doc: st.doc.clone(), selection: st.selection.clone(), layer: st.active_layer });
            }
        }
    }

    /// Delivers the events of a top-level command that just ended, unless the frontend delivers
    /// them once a frame ([`Session::plugin_event_frames`]) or a script is running.
    pub(crate) fn plugin_events_after_command(&mut self) {
        self.plugin_events_see_new_documents();
        if !self.plugin_event_frames && !self.script_in_run {
            self.deliver_plugin_events();
        }
    }

    /// After a plug-in was installed or removed, compares the open documents with the last check
    /// ([`Session::check_plugin_events`]): what changed before is given only to the plug-ins that
    /// were installed then, and what changes from now on to every plug-in installed now, also
    /// before the next delivery.
    pub(crate) fn plugins_changed_for_events(&mut self) {
        if !self.plugin_events.dispatching {
            self.check_plugin_events();
        }
    }

    /// The handlers' log lines and failures since the last call, oldest first.
    pub fn take_plugin_event_notices(&mut self) -> Vec<Notice> {
        self.plugin_events.notices.drain(..).collect()
    }

    /// Whether this session stopped delivering events to plug-in `p` after its handlers failed, and
    /// the last failure's error.
    pub fn plugin_event_status(&self, p: &Plugin) -> (bool, Option<String>) {
        match self.plugin_events.failures.get(p.id()).filter(|f| f.hash == p.hash()) {
            Some(f) => (f.disabled, f.last_error.clone()),
            None => (false, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Waiting selection and layer events of a document merge; each plug-in keeps the reasons of
    /// the changes it was given.
    #[test]
    fn waiting_selection_and_layer_events_of_a_document_merge() {
        let (a, b): (Recipient, Recipient) = (("a".into(), "1".into(), 1), ("b".into(), "2".into(), 2));
        let mut e = PluginEvents::default();
        e.queue(Event::SelectionChanged, 1, vec![], vec![a.clone()]);
        e.queue(Event::LayerChanged, 1, vec!["options"], vec![a.clone()]);
        e.queue(Event::SelectionChanged, 1, vec![], vec![a.clone()]);
        e.queue(Event::LayerChanged, 1, vec!["current", "options"], vec![a.clone()]);
        e.queue(Event::LayerChanged, 1, vec!["deleted", "current"], vec![a.clone(), b.clone()]);
        e.queue(Event::SelectionChanged, 2, vec![], vec![a.clone()]);
        e.queue(Event::DocumentSaved, 1, vec![], vec![a.clone()]);
        e.queue(Event::DocumentSaved, 1, vec![], vec![a.clone()]);
        let got: Vec<(Event, u64, Vec<&str>)> = e.pending.iter().map(|p| (p.event, p.document, p.reasons_for(&a).to_vec())).collect();
        assert_eq!(
            got,
            [
                (Event::SelectionChanged, 1, vec![]),
                (Event::LayerChanged, 1, vec!["current", "deleted", "options"]),
                (Event::SelectionChanged, 2, vec![]),
                (Event::DocumentSaved, 1, vec![]),
                (Event::DocumentSaved, 1, vec![]),
            ]
        );
        assert_eq!(e.pending[1].reasons_for(&b), ["current", "deleted"], "only the change it was given");
        assert!(!e.pending[0].is_for(&b));
    }

    /// While a handler runs, a delivery returns at once; the change waits for the next one, which
    /// hands it over once.
    #[test]
    fn an_event_raised_while_a_handler_runs_waits_for_the_next_delivery() {
        use serde_json::json;
        use vectorcraft_plugins::wat as tpl;
        let id = "test.events.nested";
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.allow_plugin_grants_from_requests();
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap()["id"].clone();
        s.execute("select.none", &json!({})).unwrap();
        let manifest =
            format!(r#"{{"id":"{id}","name":"Test","kind":"filter","api":2,"permissions":["progress","events"],"events":["selection.changed"]}}"#);
        let wasm = wat::parse_str(tpl::reacting(&manifest, tpl::HOST_IMPORTS, tpl::LOG_EVENT)).unwrap();
        let grant = json!(["progress", "events"]);
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true})).unwrap();
        s.plugin_event_frames = true;
        let given = |s: &mut Session| s.take_plugin_event_notices().into_iter().filter(|n| n.plugin == id).count();
        s.plugin_events.dispatching = true;
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.deliver_plugin_events();
        assert_eq!(given(&mut s), 0, "not inside the running handler");
        s.plugin_events.dispatching = false;
        s.deliver_plugin_events();
        assert_eq!(given(&mut s), 1);
        s.deliver_plugin_events();
        assert_eq!(given(&mut s), 0, "once");
        s.execute("plugin.remove", &json!({"id": id})).unwrap();
    }

    #[test]
    fn at_most_32_events_wait() {
        let mut e = PluginEvents::default();
        for uid in 0..(MAX_PENDING as u64 + 5) {
            e.queue(Event::DocumentOpened, uid, vec![], vec![]);
        }
        assert_eq!(e.pending.len(), MAX_PENDING);
        assert_eq!(e.pending.front().map(|p| p.document), Some(5), "the oldest are dropped");
    }
}
