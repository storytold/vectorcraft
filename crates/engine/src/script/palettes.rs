//! ScriptUI palettes (`new Window("palette")`, [`vectorcraft_js::scriptui`]): windows a run in a
//! named engine (`#targetengine`) shows, which stay open after the run ends and take clicks and
//! changes while the person goes on working.
//!
//! A palette needs a named engine, which keeps its controls and handlers between runs: in a script
//! without `#targetengine`, or with `main`, `new Window("palette")` throws. `show()` returns at once.
//! Each click, change or close the person gives a palette waits here, and then runs as a run of its
//! own in the palette's engine, which takes the mutation lease like any other run: one event at a
//! time, oldest first, each starting only once no run is going. While an event waits, another click
//! on the same control, another change of it or another close of the same palette takes its place
//! in line, with the newer value, instead of waiting as well.
//!
//! A palette closes when its script calls `close()` or the person closes it, unless `onClose`
//! returns `false`; when its engine is dropped or ends with an internal error; and with the session.
//! Hiding or closing a palette drops the events that wait for it. An error in a handler ends that
//! handler's run like any failed run, with its report, and leaves the palette open.
//!
//! The limits here are VectorCraft's own: [`MAX_PALETTES`] and [`MAX_WAITING`].

use std::collections::VecDeque;

use serde_json::{Map, Value, json};

use super::scriptui::{self, ScriptUiDialog, UiEvent, UiKind, UiValue};
use crate::{EngineError, Result, Session};

/// The most palettes a session keeps, shown or hidden.
pub const MAX_PALETTES: usize = 16;
/// The most events that wait for a run, for every palette together.
pub const MAX_WAITING: usize = 64;

/// A palette a run showed.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptPalette {
    /// Its number in the session, which no other palette of the session has had.
    pub id: u64,
    /// The named engine its handlers run in.
    pub engine: String,
    /// The script file of the run that first showed it, whose folder its handlers read in; `None`
    /// for a script given as text.
    pub file: Option<String>,
    /// Its controls, as the last `show()`, `add()` or handler left them.
    pub dialog: ScriptUiDialog,
    pub visible: bool,
    /// The SHA-256 the file grants of the script that first showed it are kept for, which its
    /// handlers' runs keep using: the grants allowed with Always. A grant allowed once is the
    /// showing run's alone, as for any later run of the script.
    pub(crate) grants: String,
}

/// What became of an event given to a palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// It waits at the end of the line.
    Added,
    /// It took the place of a waiting event for the same control.
    Merged,
}

/// An event given to a palette, waiting for its run or handed to it.
#[derive(Clone, Debug)]
pub(crate) struct Queued {
    pub(crate) palette: u64,
    pub(crate) event: UiEvent,
    /// Where the control of a click or a change sits in the palette ([`ScriptUiDialog::place_of`]),
    /// by which the event finds its control again when a run adds controls before it; empty for a
    /// close.
    pub(crate) place: Vec<usize>,
    /// For a list's change, the list's items as the palette showed them when the event was given and
    /// the list's revision then ([`super::scriptui::UiNode::revision`]), which the run checks the list
    /// still has.
    pub(crate) list: Option<(Vec<String>, f64)>,
    /// Once its run starts, the SHA-256 the file grants of the palette's script are kept for, which
    /// the run takes as its own ([`super::files::Setup::own`]).
    pub(crate) grants: Option<String>,
}

/// The palettes of a session, and the events that wait for a run.
#[derive(Debug, Default)]
pub(crate) struct Palettes {
    list: Vec<ScriptPalette>,
    waiting: VecDeque<Queued>,
    last_id: u64,
    /// The event the run that starts next handles ([`Session::script_palette_start`]).
    pub(crate) starting: Option<Queued>,
}

/// Does `event`, for the control at `place`, share the waiting `q`'s place in line: a click on the
/// same control, a change of it, or a close?
fn same_place(q: &Queued, event: &UiEvent, place: &[usize]) -> bool {
    let same_kind = matches!(
        (&q.event, event),
        (UiEvent::Click(_), UiEvent::Click(_)) | (UiEvent::Change(..), UiEvent::Change(..)) | (UiEvent::Close, UiEvent::Close)
    );
    same_kind && q.place == place
}

/// The control of a click or a change.
fn control_of(event: &mut UiEvent) -> Option<&mut usize> {
    match event {
        UiEvent::Click(c) | UiEvent::Change(c, _) => Some(c),
        UiEvent::Close => None,
    }
}

impl Palettes {
    fn find(&mut self, id: u64, engine: &str) -> Option<&mut ScriptPalette> {
        self.list.iter_mut().find(|p| p.id == id && p.engine == engine)
    }

    /// `show()`: shows the palette `id` of `engine` with `dialog`, or a new one without `id`. → its id.
    pub(crate) fn show(
        &mut self,
        id: Option<u64>,
        engine: &str,
        (file, grants): (Option<String>, String),
        dialog: ScriptUiDialog,
    ) -> std::result::Result<u64, String> {
        if let Some(p) = id.and_then(|id| self.find(id, engine)) {
            p.dialog = dialog;
            p.visible = true;
            return Ok(p.id);
        }
        if self.list.len() >= MAX_PALETTES {
            return Err(format!("show: a session keeps at most {MAX_PALETTES} script palettes; close one first"));
        }
        self.last_id += 1;
        self.list.push(ScriptPalette { id: self.last_id, engine: engine.to_string(), file, dialog, visible: true, grants });
        Ok(self.last_id)
    }

    /// The palette's controls as a handler or `add()` left them. → false when it is closed.
    pub(crate) fn update(&mut self, id: u64, engine: &str, dialog: ScriptUiDialog) -> bool {
        let Some(p) = self.find(id, engine) else { return false };
        p.dialog = dialog;
        true
    }

    /// `hide()`. → false when it is closed.
    pub(crate) fn hide(&mut self, id: u64, engine: &str) -> bool {
        let Some(p) = self.find(id, engine) else { return false };
        p.visible = false;
        self.waiting.retain(|q| q.palette != id);
        true
    }

    /// `close()`, once `onClose` allowed it. → false when it was closed already.
    pub(crate) fn close(&mut self, id: u64, engine: &str) -> bool {
        let before = self.list.len();
        self.list.retain(|p| !(p.id == id && p.engine == engine));
        self.waiting.retain(|q| q.palette != id);
        self.list.len() != before
    }

    /// Closes the palettes of the engines `keep` does not keep, with their waiting events.
    fn keep_engines(&mut self, keep: impl Fn(&str) -> bool) {
        self.list.retain(|p| keep(&p.engine));
        let open: Vec<u64> = self.list.iter().map(|p| p.id).collect();
        self.waiting.retain(|q| open.contains(&q.palette));
    }

    /// Puts `event` for the shown palette `id` in line, once it is checked against the palette.
    fn wait(&mut self, id: u64, event: UiEvent) -> std::result::Result<Waiting, String> {
        let Some(p) = self.list.iter().find(|p| p.id == id) else { return Err(format!("there is no script palette {id}")) };
        if !p.visible {
            return Err(format!("the script palette {id} is hidden"));
        }
        let mut event = scriptui::check_event(&p.dialog, event)?;
        let list = match &event {
            UiEvent::Change(c, UiValue::Index(_)) => p.dialog.nodes.get(*c).map(|n| (n.items.clone(), n.revision)),
            _ => None,
        };
        let place = control_of(&mut event).map_or_else(Vec::new, |c| p.dialog.place_of(*c));
        if let Some(slot) = self.waiting.iter_mut().find(|q| q.palette == id && same_place(q, &event, &place)) {
            slot.event = event;
            slot.list = list;
            return Ok(Waiting::Merged);
        }
        if self.waiting.len() >= MAX_WAITING {
            return Err(format!("{MAX_WAITING} script palette events already wait; this one was not kept"));
        }
        self.waiting.push_back(Queued { palette: id, event, place, list, grants: None });
        Ok(Waiting::Added)
    }

    /// The oldest waiting event whose palette is still shown, given to the control at its place in
    /// the palette as the runs since left it, and that palette. An event whose control the palette
    /// no longer has, or that the control can no longer take ([`scriptui::check_event`]), for
    /// example because a handler disabled or hid it, is dropped.
    fn next(&mut self) -> Option<(ScriptPalette, Queued)> {
        while let Some(mut q) = self.waiting.pop_front() {
            let Some(p) = self.list.iter().find(|p| p.id == q.palette && p.visible) else { continue };
            if let Some(c) = control_of(&mut q.event) {
                let Some(at) = p.dialog.control_at(&q.place) else { continue };
                *c = at;
            }
            // A list's change whose list a run changed since goes to its run, which ends with the
            // error that says so, when the list can still take a change: the choice is checked
            // without its item, which the list may no longer have.
            let list_changed = match (&q.event, &q.list) {
                (UiEvent::Change(c, UiValue::Index(_)), Some((_, revision))) => {
                    p.dialog.nodes.get(*c).is_some_and(|n| n.revision != *revision).then_some(*c)
                }
                _ => None,
            };
            if let Some(c) = list_changed {
                if scriptui::check_event(&p.dialog, UiEvent::Change(c, UiValue::Index(None))).is_err() {
                    continue;
                }
            } else {
                let Ok(event) = scriptui::check_event(&p.dialog, q.event) else { continue };
                q.event = event;
            }
            return Some((p.clone(), q));
        }
        None
    }
}

/// `name` in a `#targetengine` line, quoted with a quote it does not hold.
fn engine_line(name: &str) -> String {
    let quote = if name.contains('"') { '\'' } else { '"' };
    format!("#targetengine {quote}{name}{quote}")
}

/// The source of a palette's handler run: its engine, and the call that hands the event to the
/// palette, which takes it from the run's host.
fn handler_source(engine: &str) -> String {
    format!("{}\nWindow.__vcPaletteEvent();\n", engine_line(engine))
}

/// The event `event` (`click`, `change` or `close`) for `dialog`, from `script.palette.event`'s
/// `control` (a control's index, or its name or text) and `value` (an edit field's text, a slider's
/// number, or a list's item index or null).
pub(crate) fn parse_event(
    dialog: &ScriptUiDialog,
    event: &str,
    control: Option<&Value>,
    value: Option<&Value>,
) -> std::result::Result<UiEvent, String> {
    if event == "close" {
        if control.is_some() || value.is_some() {
            return Err("a close takes no `control` and no `value`".into());
        }
        return Ok(UiEvent::Close);
    }
    let id = match control {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("`control` must be a control's index, name or text")?,
        Some(Value::String(s)) => dialog.find(s).ok_or_else(|| format!("the palette has no control named {}", super::dialogs::cut_to(s, 64)))?,
        _ => return Err("`control` must be a control's index, name or text".into()),
    };
    let list = dialog.nodes.get(id).is_some_and(|n| matches!(n.kind, UiKind::DropDownList | UiKind::ListBox));
    let event = match (event, value) {
        ("click", None) => UiEvent::Click(id),
        ("click", Some(_)) => return Err("a click takes no `value`".into()),
        ("change", Some(Value::String(t))) => UiEvent::Change(id, UiValue::Text(t.clone())),
        ("change", Some(Value::Number(v))) if list => match scriptui::list_index(v) {
            Some(k) => UiEvent::Change(id, UiValue::Index(Some(k))),
            None => return Err("a list's `value` must be an item index or null".into()),
        },
        ("change", Some(Value::Number(v))) => UiEvent::Change(id, UiValue::Number(v.as_f64().unwrap_or(f64::NAN))),
        ("change", Some(Value::Null)) if list => UiEvent::Change(id, UiValue::Index(None)),
        ("change", _) => return Err("a change's `value` must be a string, a number, or for a list an item index or null".into()),
        _ => return Err("`event` must be click, change or close".into()),
    };
    scriptui::check_event(dialog, event)
}

/// A palette as `script.palettes` lists it.
pub(crate) fn palette_json(p: &ScriptPalette) -> Value {
    let controls: Vec<Value> = p
        .dialog
        .nodes
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, n)| {
            let mut c = Map::new();
            c.insert("index".into(), json!(i));
            c.insert("kind".into(), json!(kind_id(n.kind)));
            c.insert("name".into(), json!(n.name));
            c.insert("text".into(), json!(n.text));
            c.insert("enabled".into(), json!(n.enabled));
            c.insert("visible".into(), json!(n.visible));
            match n.kind {
                UiKind::CheckBox | UiKind::RadioButton => {
                    c.insert("value".into(), json!(n.checked));
                }
                UiKind::Slider | UiKind::ProgressBar => {
                    c.insert("value".into(), json!(n.value));
                }
                UiKind::DropDownList | UiKind::ListBox => {
                    c.insert("items".into(), json!(n.items));
                    c.insert("selection".into(), json!(n.selection));
                }
                _ => {}
            }
            Value::Object(c)
        })
        .collect();
    json!({"id": p.id, "engine": p.engine, "title": p.dialog.title(), "visible": p.visible, "controls": controls})
}

fn kind_id(kind: UiKind) -> &'static str {
    match kind {
        UiKind::Dialog => "palette",
        UiKind::Group => "group",
        UiKind::Panel => "panel",
        UiKind::StaticText => "statictext",
        UiKind::EditText => "edittext",
        UiKind::Button => "button",
        UiKind::CheckBox => "checkbox",
        UiKind::RadioButton => "radiobutton",
        UiKind::DropDownList => "dropdownlist",
        UiKind::ListBox => "listbox",
        UiKind::Slider => "slider",
        UiKind::ProgressBar => "progressbar",
    }
}

impl Session {
    /// The palettes the named engines' runs showed and have not closed, shown or hidden, in the
    /// order they were first shown.
    pub fn script_palettes(&self) -> &[ScriptPalette] {
        &self.script_palettes.list
    }

    /// How many palette events wait for a run.
    pub fn script_palette_events_waiting(&self) -> usize {
        self.script_palettes.waiting.len()
    }

    /// Whether [`Session::script_palette_start`] can start a waiting event's run now: an event
    /// waits, no run is going, and no undo group is open, which a run cannot start in. A drag in
    /// progress is not counted, since starting the run settles it.
    pub fn script_palette_can_start(&self) -> bool {
        !self.script_palettes.waiting.is_empty() && !self.script_running() && !self.undo_group_open()
    }

    /// Whether an undo group is open, the session's or a document's, which a run cannot start in.
    pub(crate) fn undo_group_open(&self) -> bool {
        self.explicit_undo_group.is_some() || self.docs.iter().any(|d| d.undo_group.is_some())
    }

    /// Puts `event` for the shown palette `id` in line, to run in its engine once no run is going
    /// ([`Session::script_palette_start`]). → an error when there is no such shown palette, it
    /// cannot take the event ([`scriptui::check_event`]), or [`MAX_WAITING`] events already wait.
    pub fn script_palette_event(&mut self, id: u64, event: UiEvent) -> std::result::Result<Waiting, String> {
        self.script_palettes.wait(id, event)
    }

    /// Starts the run of the oldest waiting palette event, when no run is going. → `None` when a
    /// run is going or no event waits; otherwise the run's name, or the error that ended it before
    /// it began.
    pub fn script_palette_start(&mut self) -> Option<Result<String>> {
        // An event waits while a drag or an undo group is open, which a run cannot start in.
        if self.script_running() || self.script_palettes.waiting.is_empty() || self.settle_for_script().is_err() {
            return None;
        }
        let (palette, mut queued) = self.script_palettes.next()?;
        queued.grants = Some(palette.grants.clone());
        let title = palette.dialog.title();
        let name = if title.is_empty() { "palette".to_string() } else { super::dialogs::cut_to(title, super::dialogs::MAX_TITLE_BYTES).to_string() };
        let locale = super::dollar::ui_locale(self.ui_language.as_deref().unwrap_or(&self.prefs.interface_language));
        let request = super::Request {
            name,
            name_given: true,
            file: palette.file.clone(),
            checked_real: None,
            source: handler_source(&palette.engine),
            args: Map::new(),
            fuel: super::DEFAULT_FUEL,
            locale,
            answers: Vec::new(),
        };
        self.script_palettes.starting = Some(queued);
        let r = self.script_start(request);
        self.script_palettes.starting = None;
        Some(r)
    }

    /// Closes the palettes whose engine the session no longer keeps.
    pub(crate) fn keep_engines_palettes(&mut self) {
        let names: Vec<String> = self.script_engines().into_iter().map(|(name, ..)| name).collect();
        self.script_palettes.keep_engines(|e| names.iter().any(|n| n == e));
    }

    /// `script.palette.event` with no run going and nobody to ask: runs the waiting events in turn,
    /// the one just given last. → that one's report. An event whose run could not begin, for
    /// example because the recovery copies could not be saved, ends this with its error, and the
    /// events after it keep waiting.
    pub(crate) fn script_palette_run_waiting(&mut self) -> Result<Value> {
        let mut last = Err(EngineError::Other("no script palette event waits".into()));
        while let Some(started) = self.script_palette_start() {
            started?;
            last = self.script_run_to_end();
        }
        last
    }
}
