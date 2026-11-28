//! A dialog a script builds with ScriptUI's `new Window("dialog")` and shows with `show()`
//! (kind `scriptWindow`). It draws the controls the running script's dialog holds, at the places
//! the engine laid them out, and gives the script each click and change.
//!
//! Its fields are `title` and, for `ui.dialog.set` and `ui.dialog.confirm`, `event` (`click`,
//! `change` or `close`), `control` (a control's index in the dialog's list of elements, or its
//! name or text) and `value` (an edit field's text, a slider's number, or a list's item index or
//! null). `ui.dialog.confirm` gives the script that event and leaves the dialog open, giving the
//! text still typed in its fields first when the event is a click or a close; an event the dialog
//! cannot take is an error. An edit field gives its text when it loses the keyboard focus, when
//! Enter is pressed in a one-line field, or before a click on another control or a close. Events
//! given in one frame reach the script one at a time, in order. Enter outside a field clicks the
//! default button, and Escape or `ui.dialog.cancel` clicks the cancel button, or closes the dialog
//! when it has none. Stop ends the run and puts back everything it did.
//!
//! The dialog lives on the window's dialog host. Its body has no app, so it keeps what the person
//! did and [`pump`] gives it to the run, and [`sync`] pushes the dialog the run waits at into it.

use std::collections::{BTreeMap, VecDeque};

use craft::automation::protocol::Params;
use craft::behavior::{Selection, SelectionChange, keyed};
use craft::theme::Variant;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Scroll, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::display::{Progress, Text};
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow};
use craft::ui::pickers::Select;
use craft::ui::runtime::focus::Initial;
use craft::ui::runtime::keys::take_submit;
use craft::ui::runtime::overlay;
use craft::ui::sections::{DialogRole, FooterButton, GroupBox};
use craft::ui::text::TextField;
use craft::ui::value_drag::Slider;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::script::dialogs::{cut_to, shown_text};
use vectorcraft_engine::script::scriptui::{MAX_TEXT_BYTES, UiNode, check_event, list_index};
use vectorcraft_engine::script::{ScriptUiDialog, UiEvent, UiKind, UiValue};

use super::script_run::with_host;
use crate::VectorcraftApp;

/// The dialog kind of a script's ScriptUI dialog.
pub const KIND: &str = "scriptWindow";

/// The most of the dialog's content shown at once; a larger dialog scrolls.
const MAX_SHOWN: egui::Vec2 = egui::vec2(960.0, 640.0);

/// The height of a list box row in the engine's layout, and the room the layout adds around the
/// rows.
const LIST_ROW: f32 = 18.0;
const LIST_PAD: f32 = 6.0;

/// How many rows a list box `height` tall shows; it scrolls the rest, inside the dialog's own
/// scroll.
fn list_rows(height: f32) -> u16 {
    let rows = ((height - LIST_PAD) / LIST_ROW).floor();
    if rows.is_finite() { rows.clamp(1.0, f32::from(u16::MAX)) as u16 } else { 1 }
}

/// An event the script has not had yet, with its control by its id ([`UiNode::id`]), which stays
/// with the control while a handler adds or removes controls before it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Queued {
    Click(u64),
    /// A change, and for a list's, the list's items as the person saw them and how many times the
    /// script had replaced them.
    Change(u64, UiValue, Option<(Vec<String>, u64)>),
    Close,
    /// Escape or `ui.dialog.cancel` while a handler ran: the click on the cancel button of the
    /// dialog the run waits at next, or its close box.
    Cancel,
}

/// The ScriptUI dialog on the window's dialog host; at most one is open.
pub(crate) struct ScriptWindow {
    /// The dialog the run waits at, or while one of its handlers runs, the one it last waited at.
    window: ScriptUiDialog,
    /// The run waits at the dialog; while a handler runs, every control is disabled.
    waiting: bool,
    /// The text typed in each edit field that the script has not had yet, by the field's id.
    drafts: BTreeMap<u64, String>,
    /// The events the person gave that the script has not had yet; the script takes one each time
    /// it waits.
    queue: VecDeque<Queued>,
    /// The event `ui.dialog.set` set: `event`, `control` and `value`.
    event: Option<String>,
    control: Option<Value>,
    value: Option<Value>,
    open: bool,
    /// Stop was clicked.
    stop: bool,
    /// Escape or the close box asked for the dialog's cancel.
    cancel: bool,
    /// An error for the status bar.
    status: Option<String>,
    /// The overlay layer the dialog's body was last drawn in.
    layer: Option<egui::Id>,
}

impl ScriptWindow {
    fn new(window: ScriptUiDialog) -> Self {
        ScriptWindow {
            window,
            waiting: true,
            drafts: BTreeMap::new(),
            queue: VecDeque::new(),
            event: None,
            control: None,
            value: None,
            open: true,
            stop: false,
            cancel: false,
            status: None,
            layer: None,
        }
    }

    /// Which `show()` of the run the dialog is for.
    pub(crate) fn shown(&self) -> u64 {
        self.window.shown
    }

    /// The text typed in edit field `i` that the script has not had yet.
    #[cfg(test)]
    pub(crate) fn typed(&self, i: usize) -> Option<&str> {
        self.window.nodes.get(i).and_then(|n| self.drafts.get(&n.id)).map(String::as_str)
    }

    /// Types `text` in edit field `i`, as the person does before the field loses the focus.
    #[cfg(test)]
    pub(crate) fn type_text(&mut self, i: usize, text: &str) {
        if let Some(n) = self.window.nodes.get(i) {
            self.drafts.insert(n.id, text.to_string());
        }
    }

    /// The events the script has not had yet.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> &VecDeque<Queued> {
        &self.queue
    }

    /// Adds `event` after the events the script has not had yet.
    pub(crate) fn queue(&mut self, event: &UiEvent) {
        if let Some(q) = queued(&self.window, event) {
            self.queue.push_back(q);
        }
    }

    /// The dialog the run waits at now (`None` while one of its handlers runs). A typed text whose
    /// field the dialog no longer has is dropped.
    fn sync(&mut self, waiting: Option<ScriptUiDialog>) {
        self.waiting = waiting.is_some();
        if let Some(w) = waiting {
            self.window = w;
            let fields: Vec<u64> = self.window.nodes.iter().skip(1).filter(|n| n.kind == UiKind::EditText).map(|n| n.id).collect();
            self.drafts.retain(|id, _| fields.contains(id));
        }
    }

    /// Before a click or a close, gives the script the text typed in each edit field that has not
    /// left the field yet, so the handler of a button that closes the dialog, or `onClose`, reads
    /// it. A disabled or hidden field keeps its text for later. Each text is checked as the script
    /// in `w` would check it first; when one is refused, every field keeps its text, `events` stays
    /// as it was and the error is returned.
    pub(crate) fn commit_drafts(&mut self, w: &ScriptUiDialog, events: &mut Vec<UiEvent>) -> Result<(), String> {
        let mut given = Vec::new();
        let mut changes = Vec::new();
        for (i, n) in w.nodes.iter().enumerate() {
            if n.kind != UiKind::EditText
                || n.readonly
                || !n.enabled
                || !n.visible
                || events.iter().any(|e| matches!(e, UiEvent::Change(c, _) if *c == i))
            {
                continue;
            }
            if let Some(text) = self.drafts.get(&n.id) {
                if *text != n.text {
                    changes.push(check_event(w, UiEvent::Change(i, UiValue::Text(text.clone())))?);
                }
                given.push(n.id);
            }
        }
        for id in given {
            self.drafts.remove(&id);
        }
        let closing = |e: &UiEvent| matches!(e, UiEvent::Click(_) | UiEvent::Close);
        events.sort_by_key(closing);
        let at = events.iter().position(closing).unwrap_or(events.len());
        events.splice(at..at, changes);
        Ok(())
    }

    /// Takes out of `events` each text an edit field gave as it lost the focus that the script
    /// would refuse, and puts it back in its field, so the person can fix it. → the error of the
    /// last one.
    pub(crate) fn keep_refused_texts(&mut self, events: &mut Vec<UiEvent>) -> Option<String> {
        let mut refused = None;
        let (w, drafts) = (&self.window, &mut self.drafts);
        events.retain(|e| {
            let UiEvent::Change(i, UiValue::Text(text)) = e else { return true };
            match check_event(w, e.clone()) {
                Ok(_) => true,
                Err(error) => {
                    if let Some(n) = w.nodes.get(*i) {
                        drafts.insert(n.id, text.clone());
                    }
                    refused = Some(error);
                    false
                }
            }
        });
        refused
    }

    /// The next queued event the dialog `w` can still take, with its control's index in `w` now.
    /// An event whose control or list items a handler removed since it was given is dropped.
    fn next_event(&mut self, w: &ScriptUiDialog) -> Option<UiEvent> {
        while let Some(next) = self.queue.pop_front() {
            if let Some(event) = resolved(w, next) {
                return Some(event);
            }
        }
        None
    }

    /// The events the person gave in this pass, after the ones before them: the typed texts the
    /// script would refuse go back in their fields, and a click gives the typed texts first, or
    /// waits while one of them is refused.
    fn take_events(&mut self, mut events: Vec<UiEvent>) {
        if let Some(e) = self.keep_refused_texts(&mut events) {
            self.status = Some(e);
        }
        if events.iter().any(|e| matches!(e, UiEvent::Click(_))) {
            let w = self.window.clone();
            if let Err(e) = self.commit_drafts(&w, &mut events) {
                // A refused text stays in its field, and the click waits for the person to fix it.
                events.retain(|e| !matches!(e, UiEvent::Click(_)));
                self.status = Some(e);
            }
        }
        for e in &events {
            self.queue(e);
        }
    }
}

/// `event` in the dialog `w`, with its control by its id.
fn queued(w: &ScriptUiDialog, event: &UiEvent) -> Option<Queued> {
    let id = |i: usize| w.nodes.get(i).map(|n| n.id);
    Some(match event {
        UiEvent::Click(i) => Queued::Click(id(*i)?),
        UiEvent::Change(i, v) => {
            let items = matches!(v, UiValue::Index(_)).then(|| w.nodes.get(*i).map(|n| (n.items.clone(), n.items_revision))).flatten();
            Queued::Change(id(*i)?, v.clone(), items)
        }
        UiEvent::Close => Queued::Close,
    })
}

/// The queued event `q` in the dialog `w`, with `control` the index its control has in `w` now.
/// `None` when `w` no longer has the control, or for a list's change, when the list's items are no
/// longer those the person chose from.
fn resolved(w: &ScriptUiDialog, q: Queued) -> Option<UiEvent> {
    match q {
        Queued::Click(id) => Some(UiEvent::Click(index_of(w, id)?)),
        Queued::Change(id, v, items) => {
            let i = index_of(w, id)?;
            if items.is_some_and(|(items, revision)| items != w.nodes[i].items || revision != w.nodes[i].items_revision) {
                return None;
            }
            Some(UiEvent::Change(i, v))
        }
        Queued::Close => Some(UiEvent::Close),
        Queued::Cancel => Some(cancel_event(w)),
    }
}

/// The dialog `w`'s cancel: a click on its cancel button, or its close box when it has none or the
/// button cannot be clicked.
fn cancel_event(w: &ScriptUiDialog) -> UiEvent {
    w.cancel_element.map(UiEvent::Click).filter(|e| check_event(w, e.clone()).is_ok()).unwrap_or(UiEvent::Close)
}

/// The index of the control whose id ([`UiNode::id`]) is `id` in the dialog `w`, if it has one.
fn index_of(w: &ScriptUiDialog, id: u64) -> Option<usize> {
    w.nodes.iter().position(|n| n.id == id).filter(|i| *i > 0)
}

impl StackDialog for ScriptWindow {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<()>> {
        let top = stack.is_top(self.id());
        let heading = match self.window.title() {
            "" => tl!("Script Alert").to_string(),
            title => shown_text(title, false),
        };
        let buttons = [FooterButton::new(tl!("Stop"), DialogRole::Other).stays_open(true)];
        let ScriptWindow { window, waiting, drafts, open, .. } = self;
        let (window, waiting, drafts) = (&*window, *waiting, &*drafts);
        // The dialog opens with the focus on its default button, so Enter clicks it until the
        // person moves the focus into a field; without one that can be clicked, on its first control.
        let focus_default = window.default_element.and_then(|i| window.nodes.get(i)).is_some_and(|n| n.visible && n.enabled);
        let initial = if focus_default { Initial::Default } else { Initial::First };
        let dialog = Dialog::new(heading).buttons(buttons).initial_focus(initial).opened(Bind::bound(open));
        // The body can run twice in a pass (measure, then paint), so it only reports what the
        // person did; the dialog applies the last run's report after the container.
        let shown = dialog.show(ui, |ui, _| {
            let (width, height) = window.size();
            let size = egui::vec2(width.max(1.0), height.max(1.0));
            let mut pass = Pass { layer: overlay::drawing_parent(ui.ctx()), ..Pass::default() };
            ui.scope(|ui| {
                ui.set_max_size(MAX_SHOWN);
                Stack::new().scroll(Scroll::both()).show(ui, |ui, _| {
                    let origin = ui.cursor().min;
                    ui.set_min_size(size);
                    // Each container comes before the controls it holds.
                    let mut containers: Vec<Option<egui::Id>> = vec![None; window.nodes.len()];
                    for (i, n) in window.nodes.iter().enumerate().skip(1) {
                        if !n.visible {
                            continue;
                        }
                        let parent = containers.get(n.parent).copied().flatten();
                        let place = Place { origin, enabled: waiting && n.enabled, default: window.default_element == Some(i), parent };
                        let shown = control(ui, drafts, &place, (i, n, prompt_of(window, i)));
                        if let Some(slot) = containers.get_mut(i) {
                            *slot = shown.container;
                        }
                        pass.typing |= shown.typing;
                        pass.events.extend(shown.event);
                        pass.drafts.extend(shown.draft.map(|d| (n.id, d)));
                    }
                });
            });
            pass
        });
        let pass = shown.inner.unwrap_or_default();
        if pass.layer.is_some() {
            self.layer = pass.layer;
        }
        for (id, draft) in pass.drafts {
            match draft {
                Some(text) => self.drafts.insert(id, text),
                None => self.drafts.remove(&id),
            };
        }
        let (mut events, typing) = (pass.events, pass.typing);
        // An Enter the focused control left untaken (a slider, a check box, a button) is the
        // dialog's, unless a question is open over it or a field has the focus.
        let submit = top && !typing && take_submit(ui.ctx());
        if shown.output.picked == Some(0) {
            self.stop = true;
            return None;
        }
        // Escape and the close box click the cancel button: the dialog stays open until the script
        // closes it.
        if shown.output.closed.is_some() {
            self.open = true;
            self.cancel = true;
            return None;
        }
        // Enter in a field belongs to the field; with the focus on any other control, it clicks the
        // default button.
        if events.is_empty() && self.waiting && submit {
            events.extend(self.window.default_element.map(UiEvent::Click));
        }
        if !events.is_empty() {
            self.take_events(events);
        }
        None
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({
            "title": self.window.title(),
            "event": self.event,
            "control": self.control,
            "value": self.value,
        }))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "event" => {
                let event = match &value {
                    Value::Null => None,
                    Value::String(e) if matches!(e.as_str(), "click" | "change" | "close") => Some(e.clone()),
                    _ => return Err("`event` must be click, change or close".into()),
                };
                self.event = event;
            }
            "control" => {
                if !matches!(value, Value::Null | Value::Number(_) | Value::String(_)) {
                    return Err("`control` must be a control's index, name or text".into());
                }
                self.control = Some(value).filter(|v| !v.is_null());
            }
            "value" => self.value = Some(value),
            "title" => return Err(FieldRefusal::ReadOnly.message(field)),
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// Where a control is drawn: from `origin`, enabled or not, whether it is the default button, and
/// the accessibility node of the group or panel that holds it.
struct Place {
    origin: egui::Pos2,
    enabled: bool,
    default: bool,
    parent: Option<egui::Id>,
}

/// What drawing a control gave: the event it gives, whether an edit field has the keyboard focus,
/// a group's or a panel's accessibility node, and an edit field's typed text to keep (`Some`) or
/// forget (`None`).
#[derive(Default)]
struct Shown {
    event: Option<UiEvent>,
    typing: bool,
    container: Option<egui::Id>,
    draft: Option<Option<String>>,
}

/// What one run of the dialog's body gave: the events, whether an edit field has the keyboard
/// focus, the typed texts to keep or forget, by field id, and the overlay layer it was drawn in.
#[derive(Default)]
struct Pass {
    events: Vec<UiEvent>,
    typing: bool,
    drafts: Vec<(u64, Option<String>)>,
    layer: Option<egui::Id>,
}

/// The text of the visible static text just before control `i` in its container: the visible
/// prompt of an edit field, a list box or a drop-down list, which screen readers call one without a
/// name by. Empty when there is none.
pub(crate) fn prompt_of(w: &ScriptUiDialog, i: usize) -> &str {
    let Some(parent) = w.nodes.get(i).map(|n| n.parent) else { return "" };
    match w.nodes[..i].iter().skip(1).rev().find(|n| n.parent == parent && n.visible) {
        Some(n) if n.kind == UiKind::StaticText => &n.text,
        _ => "",
    }
}

/// Draws control `i` (`n`, with the prompt [`prompt_of`] gives) at its place.
fn control(ui: &mut Ui, drafts: &BTreeMap<u64, String>, place: &Place, (i, n, prompt): (usize, &UiNode, &str)) -> Shown {
    let rect = egui::Rect::from_min_size(place.origin + egui::vec2(n.rect.x, n.rect.y), egui::vec2(n.rect.w, n.rect.h));
    let mut shown = Shown::default();
    let enabled = place.enabled;
    let salt = (KIND, n.id);
    // A control in a group or a panel is that container's child in the accessibility tree.
    let builder = egui::UiBuilder::new().id_salt(n.id).max_rect(rect);
    let builder = match place.parent {
        Some(parent) => builder.accessibility_parent(parent),
        None => builder,
    };
    ui.scope_builder(builder, |ui| match n.kind {
        UiKind::Dialog => {}
        UiKind::Group => shown.container = Some(group(ui, rect, n, enabled)),
        UiKind::Panel => shown.container = Some(panel(ui, rect, n, enabled)),
        UiKind::StaticText => {
            Text::new(n.text.clone()).show(ui);
        }
        UiKind::Button => {
            let mut button = Button::new(n.text.clone()).id_salt(salt).enabled(enabled);
            if place.default {
                button = button.variant(Variant::PRIMARY).default_stop();
            }
            if clicked(&button.show(ui)) {
                shown.event = Some(UiEvent::Click(i));
            }
        }
        UiKind::CheckBox => {
            let out = Checkbox::new(Bind::given(n.checked)).label(n.text.clone()).id_salt(salt).enabled(enabled).show(ui);
            if out.events.committed().is_some() {
                shown.event = Some(UiEvent::Click(i));
            }
        }
        UiKind::RadioButton => {
            let mut picked = false;
            let out = RadioGroup::new(n.text.clone(), Bind::given(n.checked)).id_salt(salt).enabled(enabled).show(ui, |ui, _| {
                picked = clicked(&Radio::new(true).label(n.text.clone()).enabled(enabled).show(ui));
            });
            if picked || out.events.committed().is_some() {
                shown.event = Some(UiEvent::Click(i));
            }
        }
        UiKind::EditText => shown = edit(ui, drafts, enabled, (i, n, prompt)),
        UiKind::DropDownList => {
            let entries: Vec<Entry<Option<usize>>> = n.items.iter().enumerate().map(|(k, item)| Entry::new(Some(k), item.clone())).collect();
            let out = Select::new(list_name(n, prompt), Bind::given(n.selection), &entries).id_salt(salt).enabled(enabled).show(ui);
            if let Some(k) = out.events.committed().copied().filter(|k| *k != n.selection) {
                shown.event = Some(UiEvent::Change(i, UiValue::Index(k)));
            }
        }
        UiKind::ListBox => {
            let keys: Vec<usize> = (0..n.items.len()).collect();
            let source = keyed(&keys, |k: &usize| *k);
            let selection = n.selection.map(Selection::single).unwrap_or_default();
            // Screen readers read the box as a list box, by its name or the static text before
            // it, with the selected item as its value. The box has no opener to keep the keyboard
            // focus, so a press on a row gives it the focus and the arrow keys go on from that row.
            let out = List::<usize>::options(list_name(n, prompt))
                .focus_on_press(true)
                .id_salt(salt)
                .enabled(enabled)
                .max_rows(list_rows(n.rect.h))
                .selection(Bind::given(selection))
                .show_items(ui, &source, &mut |k, _| ListRow::new(n.items.get(*k).cloned().unwrap_or_default()));
            // A click activates a row; the arrow keys and typed letters select one.
            for event in out.events {
                let picked = match event {
                    CollectionEvent::Activate(k) => Some(k),
                    CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => keys.last().copied(),
                    _ => None,
                };
                if let Some(k) = picked.filter(|k| n.selection != Some(*k)) {
                    shown.event = Some(UiEvent::Change(i, UiValue::Index(Some(k))));
                }
            }
        }
        UiKind::Slider => {
            let out = Slider::new(Bind::given(n.value), n.min..=n.max).label(screen_reader_name(n, prompt)).id_salt(salt).enabled(enabled).show(ui);
            if let Some(v) = out.events.committed().copied().filter(|v| *v != n.value) {
                shown.event = Some(UiEvent::Change(i, UiValue::Number(v)));
            }
        }
        UiKind::ProgressBar => {
            Progress::new(n.fraction()).label(screen_reader_name(n, prompt)).id_salt(salt).show(ui);
        }
    });
    shown
}

/// A panel: a group box titled by its text over the panel's place; screen readers read it as a
/// group, disabled when the panel is. → the id of its node in the accessibility tree.
fn panel(ui: &mut Ui, rect: egui::Rect, n: &UiNode, enabled: bool) -> egui::Id {
    let shown = GroupBox::new(n.text.clone(), None).id_salt((KIND, n.id)).enabled(enabled).show_at(ui, rect, |_, _| ());
    shown.output.response.id
}

/// A group, which draws nothing; screen readers read it as a group called by the name the script
/// gave it, disabled when the group is.
/// → the id of its node in the accessibility tree.
fn group(ui: &mut Ui, rect: egui::Rect, n: &UiNode, enabled: bool) -> egui::Id {
    let (enabled, name) = (enabled && ui.is_enabled(), n.name.clone());
    let r = ui.interact(rect, ui.scope_id().with("group"), egui::Sense::hover());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, enabled, &name));
    r.id
}

/// The name screen readers call a list box or a drop-down list `n` by: the visible static text
/// just before it, or else the name the script gave it, or else "Value".
fn list_name(n: &UiNode, prompt: &str) -> String {
    match (prompt, n.name.as_str()) {
        ("", "") => tl!("Value").to_string(),
        ("", name) => name.to_string(),
        (prompt, _) => prompt.to_string(),
    }
}

/// The name screen readers call a control with no visible label of its own (an edit field, a
/// slider or a progress bar) by: the visible static text just before it in its container, or else
/// the name the script gave it, or else a slider's or progress bar's text, or else "Text" for an
/// edit field and "Value" for the others.
pub(crate) fn screen_reader_name(n: &UiNode, prompt: &str) -> String {
    let name = if !prompt.is_empty() {
        prompt
    } else if !n.name.is_empty() || n.kind == UiKind::EditText {
        &n.name
    } else {
        &n.text
    };
    match name {
        "" if n.kind == UiKind::EditText => tl!("Text").to_string(),
        "" => tl!("Value").to_string(),
        name => name.to_string(),
    }
}

/// The most characters an edit field takes while it is typed: twice the longest text a script
/// accepts, so that a text that is too long can still be typed, refused and shortened.
pub(crate) const DRAFT_CHARS: usize = 2 * MAX_TEXT_BYTES;

/// An edit field: its text is kept in the dialog while it is typed and given to the script when
/// the field loses the focus, or at Enter in a one-line field.
fn edit(ui: &mut Ui, drafts: &BTreeMap<u64, String>, enabled: bool, (i, n, prompt): (usize, &UiNode, &str)) -> Shown {
    let mut text = drafts.get(&n.id).cloned().unwrap_or_else(|| n.text.clone());
    let mut field = TextField::new(screen_reader_name(n, prompt), Bind::bound(&mut text))
        .limit(DRAFT_CHARS)
        .id_salt((KIND, n.id))
        .enabled(enabled)
        .read_only(n.readonly);
    if n.multiline {
        field = field.lines(3);
    }
    let out = field.show(ui);
    let focused = out.response.has_focus();
    let mut draft = (out.events.value().is_some() || text != n.text).then(|| Some(text.clone()));
    let committed = out.submitted || out.events.committed().is_some() || out.response.lost_focus();
    if committed && text != n.text && !n.readonly {
        return Shown { event: Some(UiEvent::Change(i, UiValue::Text(text))), typing: true, container: None, draft: Some(None) };
    }
    // A text the script refused stays in the field until the person changes it.
    if !focused && text == n.text {
        draft = Some(None);
    }
    Shown { event: None, typing: focused || committed, container: None, draft }
}

/// The event in the dialog's fields, for the dialog `w`.
fn event_of(w: &ScriptUiDialog, d: &ScriptWindow) -> Result<UiEvent, String> {
    // A close names no control, so a `control` left from an earlier event does not stop it.
    if d.event.as_deref() == Some("close") {
        return Ok(UiEvent::Close);
    }
    let control = match &d.control {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("`control` must be a control's index, name or text")?,
        Some(Value::String(s)) => w.find(s).ok_or_else(|| format!("the dialog has no control named {}", cut_to(s, 64)))?,
        _ => 0,
    };
    let list = w.nodes.get(control).is_some_and(|n| matches!(n.kind, UiKind::DropDownList | UiKind::ListBox));
    match (d.event.as_deref(), &d.value) {
        (Some("click"), _) => Ok(UiEvent::Click(control)),
        (Some("change"), Some(Value::String(t))) => Ok(UiEvent::Change(control, UiValue::Text(t.clone()))),
        (Some("change"), Some(Value::Number(v))) if list => match list_index(v) {
            Some(k) => Ok(UiEvent::Change(control, UiValue::Index(Some(k)))),
            None => Err("a list's `value` must be an item index or null".into()),
        },
        (Some("change"), Some(Value::Number(v))) => Ok(UiEvent::Change(control, UiValue::Number(v.as_f64().unwrap_or(f64::NAN)))),
        (Some("change"), Some(Value::Null) | None) if list => Ok(UiEvent::Change(control, UiValue::Index(None))),
        (Some("change"), _) => Err("a change's `value` must be a string, a number or null".into()),
        _ => Err("`event` must be click, change or close".into()),
    }
}

/// Runs `f` on the open ScriptUI dialog; `None` when none is open.
pub(crate) fn with_window<R>(app: &VectorcraftApp, f: impl FnOnce(&mut ScriptWindow) -> R) -> Option<R> {
    with_host(app, |host, _| host.get_mut::<ScriptWindow>().map(f)).flatten()
}

/// Whether the ScriptUI dialog is open and no other dialog is open over it.
pub(crate) fn is_top(app: &VectorcraftApp) -> bool {
    with_host(app, |host, _| host.get::<ScriptWindow>().is_some_and(|d| host.stack().is_top(d.id()))).unwrap_or(false)
}

/// Whether the ScriptUI dialog is the top dialog and no popup or modal layer, its own drop-down
/// lists' included, is open over it.
pub(crate) fn uncovered(app: &VectorcraftApp) -> bool {
    let Some(layer) = with_window(app, |d| d.layer) else { return false };
    let top = app.ui_ctx.as_ref().and_then(overlay::top);
    is_top(app) && top.is_none_or(|top| Some(top) == layer)
}

/// Opens, keeps or closes the ScriptUI dialog as the run's `show()` stands: it opens when a
/// `show()` waits, follows the dialog while the run waits at it (its heading follows a title a
/// handler changed), stays open while one of its handlers runs and closes when `show()` ends. A
/// dialog the script showed after closing the one that is open replaces it.
pub(crate) fn sync(app: &mut VectorcraftApp) {
    let Some(showing) = app.session.script_window_showing() else {
        super::script_run::forget::<ScriptWindow>(app);
        return;
    };
    let open_for = with_window(app, |d| d.shown());
    if open_for.is_some_and(|shown| shown != showing.shown) {
        super::script_run::forget::<ScriptWindow>(app);
    }
    let waiting = app.session.script_window();
    if open_for == Some(showing.shown) {
        with_window(app, |d| d.sync(waiting));
    } else if let Some(w) = waiting
        && !super::script_run::busy(app)
    {
        with_host(app, |host, _| host.open(ScriptWindow::new(w)));
    }
}

/// Takes what the person did in the dialog, once per frame, and gives the run its next event while
/// it waits at the dialog.
pub(crate) fn pump(app: &mut VectorcraftApp) {
    let Some((stop, cancel_asked, status)) = with_window(app, |d| (std::mem::take(&mut d.stop), std::mem::take(&mut d.cancel), d.status.take()))
    else {
        return;
    };
    if let Some(e) = status {
        app.status(e);
    }
    if stop {
        app.session.script_request_stop();
        super::script_run::forget::<ScriptWindow>(app);
        return;
    }
    if cancel_asked && let Err(e) = cancel(app) {
        app.status(e);
    }
    // Events wait in the queue while another dialog, such as the quit question, is open over it.
    if !is_top(app) {
        return;
    }
    let Some(w) = app.session.script_window() else { return };
    if with_window(app, |d| d.shown()) != Some(w.shown) {
        return;
    }
    if let Some(event) = with_window(app, |d| d.next_event(&w)).flatten() {
        // A typed text the script refuses (an earlier handler disabled its field, say) goes back
        // in its field, so the person does not have to type it again.
        let typed = match &event {
            UiEvent::Change(i, UiValue::Text(t)) => w.nodes.get(*i).map(|n| (n.id, t.clone())),
            _ => None,
        };
        if let Err(e) = give(app, &w, event) {
            if let Some((id, text)) = typed {
                with_window(app, |d| d.drafts.insert(id, text));
            }
            app.status(e);
        }
    }
}

/// Gives the run `event` in the dialog `w` it waits at. Before a click or a close, the script is
/// given the text still typed in the dialog's fields first, and the click or close waits for its
/// next wait.
fn give(app: &mut VectorcraftApp, w: &ScriptUiDialog, event: UiEvent) -> Result<(), String> {
    let mut events = vec![check_event(w, event)?];
    if matches!(events[0], UiEvent::Click(_) | UiEvent::Close) {
        with_window(app, |d| d.commit_drafts(w, &mut events)).unwrap_or(Ok(()))?;
    }
    let mut events = events.into_iter();
    if let Some(first) = events.next() {
        app.session.script_window_answer(first)?;
    }
    let later: Vec<Queued> = events.filter_map(|e| queued(w, &e)).collect();
    with_window(app, |d| {
        for q in later.into_iter().rev() {
            d.queue.push_front(q);
        }
    });
    Ok(())
}

/// `ui.dialog.confirm`: gives the running script the event in the dialog's fields. The dialog stays
/// open for the script's next wait; an event it cannot take is an error.
pub(crate) fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(w) = app.session.script_window() else { return Err("no script dialog waits".into()) };
    let event = with_window(app, |d| event_of(&w, d)).ok_or("no script dialog waits")??;
    if with_window(app, |d| d.queue.is_empty()).unwrap_or(true) {
        give(app, &w, event)?;
    } else {
        // Events the script has not had yet come first; this one, and the typed texts before a
        // click or a close, go after them.
        let mut events = vec![check_event(&w, event)?];
        if matches!(events[0], UiEvent::Click(_) | UiEvent::Close) {
            with_window(app, |d| d.commit_drafts(&w, &mut events)).unwrap_or(Ok(()))?;
        }
        let later: Vec<Queued> = events.iter().filter_map(|e| queued(&w, e)).collect();
        with_window(app, |d| d.queue.extend(later));
    }
    with_window(app, |d| (d.event, d.control, d.value) = (None, None, None));
    Ok(Value::Null)
}

/// Escape or `ui.dialog.cancel`: a click on the cancel button, or the close box when the dialog
/// has none or it cannot be clicked, after the text still typed in its fields. Events the script
/// has not had yet go after the ones before them. A typed text the script would refuse is an
/// error, and the dialog is left as it was. While a handler runs, the cancel waits for the dialog
/// the run waits at next.
pub(crate) fn cancel(app: &mut VectorcraftApp) -> Result<(), String> {
    let Some(w) = app.session.script_window() else {
        return with_window(app, |d| d.queue.push_back(Queued::Cancel)).ok_or_else(|| "no script dialog waits".to_string());
    };
    let mut events = vec![cancel_event(&w)];
    with_window(app, |d| d.commit_drafts(&w, &mut events)).unwrap_or(Ok(()))?;
    let empty = with_window(app, |d| d.queue.is_empty()).unwrap_or(true);
    // A cancel after other events goes last, and is a click on the cancel button of the dialog the
    // run waits at when its turn comes, or its close box.
    let deferred = !empty || events.len() > 1;
    if deferred {
        events.pop();
    }
    let mut events = events.into_iter();
    if empty && let Some(first) = events.next() {
        app.session.script_window_answer(first)?;
    }
    let mut later: Vec<Queued> = events.filter_map(|e| queued(&w, &e)).collect();
    if deferred {
        later.push(Queued::Cancel);
    }
    with_window(app, |d| d.queue.extend(later));
    Ok(())
}

/// `ui.dialog.set`, `ui.dialog.confirm` and `ui.dialog.cancel` (`method`, with `params` naming the
/// dialog) when they name the ScriptUI dialog; `None` for any other dialog. `ui.dialog.set` sets
/// the fields as the host does, then answers with its `action`.
pub(crate) fn dialog_method(app: &mut VectorcraftApp, method: &str, params: &Value) -> Option<Result<Value, String>> {
    let ours = with_host(app, |host, _| craft::automation::dialogs::dialog_id(host, params).is_ok_and(|id| id == dialog_id(KIND)))?;
    if !ours {
        return None;
    }
    // The parameters each method takes, checked before anything is set or given.
    let keys: &[&str] = if method == "ui.dialog.set" { &["dialog", "field", "value", "fields", "action"] } else { &["dialog"] };
    if let Err(e) = Params(params).only(None, keys) {
        return Some(Err(e));
    }
    let mut params = params.clone();
    let action = if method == "ui.dialog.set" { params.as_object_mut().and_then(|o| o.remove("action")) } else { None };
    let answer = match (method, action.as_ref().map(Value::as_str)) {
        ("ui.dialog.confirm", _) | (_, Some(Some("confirm"))) => Some(true),
        ("ui.dialog.cancel", _) | (_, Some(Some("cancel"))) => Some(false),
        (_, None) => None,
        (_, Some(_)) => return Some(Err("`action` must be confirm or cancel".into())),
    };
    // A dialog open over it, such as the quit question, takes its answer first.
    if answer == Some(true) && with_host(app, |host, _| host.stack().has_child(dialog_id(KIND))).unwrap_or(false) {
        return Some(Err("the dialog refuses OK while a dialog open over it waits for its answer".into()));
    }
    // A set without an action sets a field, which the host's setter asks for when none is given.
    if method == "ui.dialog.set" && (answer.is_none() || params.as_object().is_some_and(|o| o.keys().any(|k| k != "dialog"))) {
        let set = with_host(app, |host, ctx| craft::automation::dialogs::dialog_method(host, ctx, method, &params)).flatten();
        if let Some(Err(e)) = set {
            return Some(Err(e));
        }
    }
    Some(match answer {
        Some(true) => confirm(app),
        Some(false) => cancel(app).map(|()| Value::Null),
        // A set answers with the dialog it set, `{id, parent, key, fields}`, as the other dialogs' sets do.
        None => Ok(with_host(app, |host, _| {
            let id = dialog_id(KIND).value().to_string();
            craft::automation::dialogs::inspect(host).as_array().and_then(|open| open.iter().find(|d| d["id"] == id.as_str()).cloned())
        })
        .flatten()
        .unwrap_or(Value::Null)),
    })
}
