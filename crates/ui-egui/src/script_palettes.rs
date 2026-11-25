//! The ScriptUI palettes a script shows with `new Window("palette")` in a named engine
//! ([`vectorcraft_engine::script::palettes`]): each shown palette is a modeless dialog on the
//! window's dialog host (kind `scriptPalette`) that stays open after the run, with the controls the
//! engine laid out. A click, a change or the close box waits in the session's line and runs as a
//! script run of its own once no run is going ([`crate::script_runs::poll`]); the palette stays
//! usable while a run goes on, and its events wait for it. A palette does not take the keyboard
//! focus when it opens or changes. An edit field's text is given when it loses the focus, at Enter
//! in a one-line field, or before a click or a close; a text the script would refuse, or that the
//! full line refuses, stays in its field with the error in the status bar, and the click or close
//! after it is not given.
//!
//! The dialog's body has no app, so it keeps what the person did, and [`show`] follows the
//! session's palettes into the host each frame and puts the events in line.

use std::collections::BTreeMap;

use craft::ui::contract::{Bind, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::runtime::focus::Initial;
use egui::Ui;
use vectorcraft_engine::script::{ScriptPalette, ScriptUiDialog, UiEvent, UiValue};

use crate::VectorcraftApp;
use crate::dialogs::script_run::with_host;
use crate::dialogs::script_window;

/// The dialog kind of a ScriptUI palette.
pub const KIND: &str = "scriptPalette";

/// The dialog id of the palette `id` ([`ScriptPalette::id`]).
pub(crate) fn palette_id(id: u64) -> DialogId {
    dialog_id((KIND, id))
}

/// A shown palette on the window's dialog host.
pub(crate) struct PaletteWindow {
    /// The palette as the session has it, from the last frame.
    palette: ScriptPalette,
    /// The text typed in each edit field that the script has not had yet, by the field's id.
    drafts: BTreeMap<u64, String>,
    /// The events the person gave since the app last took them.
    given: Vec<UiEvent>,
    open: bool,
}

impl PaletteWindow {
    fn new(palette: ScriptPalette) -> Self {
        PaletteWindow { palette, drafts: BTreeMap::new(), given: Vec::new(), open: true }
    }

    /// The palette as the session has it now. A typed text whose field the palette no longer has is
    /// dropped.
    fn sync(&mut self, palette: ScriptPalette) {
        self.palette = palette;
        script_window::keep_fields(&self.palette.dialog, &mut self.drafts);
    }

    /// The text typed in edit field `i` that the script has not had yet.
    #[cfg(test)]
    pub(crate) fn typed(&self, i: usize) -> Option<&str> {
        self.palette.dialog.nodes.get(i).and_then(|n| self.drafts.get(&n.id)).map(String::as_str)
    }

    /// Types `text` in edit field `i`, as the person does before the field loses the focus.
    #[cfg(test)]
    pub(crate) fn type_text(&mut self, i: usize, text: &str) {
        if let Some(n) = self.palette.dialog.nodes.get(i) {
            self.drafts.insert(n.id, text.to_string());
        }
    }

    /// Gives `event`, as the person does in the palette.
    #[cfg(test)]
    pub(crate) fn give(&mut self, event: UiEvent) {
        self.given.push(event);
    }
}

impl StackDialog for PaletteWindow {
    type Result = ();

    fn id(&self) -> DialogId {
        palette_id(self.palette.id)
    }

    fn instance(&self) -> Option<DialogId> {
        Some(self.id())
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn modeless(&self) -> bool {
        true
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let heading = script_window::heading(&self.palette.dialog);
        let PaletteWindow { palette, drafts, given, open } = self;
        let mut events = Vec::new();
        let shown = Dialog::new(heading).modeless().initial_focus(Initial::Keep).opened(Bind::bound(open)).show(ui, |ui, _| {
            events = script_window::controls(ui, &palette.dialog, drafts, true).events;
        });
        given.extend(events);
        // The close box gives the script the close; the palette stays open until the script closes
        // it.
        if shown.output.closed.is_some() {
            self.open = true;
            self.given.push(UiEvent::Close);
        }
        None
    }
}

/// Follows the session's shown palettes into the dialog host: opens a palette that shows, updates
/// one that changed and closes one that was hidden or closed. Then puts the events the person gave
/// each palette in line.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let shown: Vec<ScriptPalette> = app.session.script_palettes().iter().filter(|p| p.visible).cloned().collect();
    let taken = with_host(app, |host, ctx| {
        let ids: Vec<DialogId> = shown.iter().map(|p| palette_id(p.id)).collect();
        for d in host.open_dialogs() {
            if d.key.as_deref() == Some(KIND) && !ids.contains(&d.id) {
                host.cancel_by_id(ctx, d.id);
                let _ = host.take_by_id::<PaletteWindow>(d.id);
            }
        }
        let mut taken = Vec::new();
        for p in &shown {
            match host.get_mut_by_id::<PaletteWindow>(palette_id(p.id)) {
                Some(w) => {
                    w.sync(p.clone());
                    taken.push((p.id, std::mem::take(&mut w.drafts), std::mem::take(&mut w.given)));
                }
                None => {
                    host.open(PaletteWindow::new(p.clone()));
                }
            }
        }
        taken
    })
    .unwrap_or_default();
    for (id, mut drafts, mut events) in taken {
        let Some(p) = shown.iter().find(|p| p.id == id) else { continue };
        drafted(app, &p.dialog, &mut drafts, &mut events);
        give(app, ctx, p, &mut drafts, events);
        with_host(app, |host, _| {
            if let Some(w) = host.get_mut_by_id::<PaletteWindow>(palette_id(id)) {
                w.drafts = drafts;
            }
        });
    }
}

/// Puts the text still typed in the fields of palette `w` (`drafts`) before a click or a close in
/// `events`. A text the script would refuse stays in its field with its error in the status bar,
/// and a click or a close waits for the person to fix it.
fn drafted(app: &mut VectorcraftApp, w: &ScriptUiDialog, drafts: &mut BTreeMap<u64, String>, events: &mut Vec<UiEvent>) {
    if let Some(e) = script_window::keep_refused(w, drafts, events) {
        app.status(e);
    }
    if events.iter().any(|e| matches!(e, UiEvent::Click(_) | UiEvent::Close))
        && let Err(e) = script_window::commit_texts(w, drafts, events)
    {
        events.retain(|e| !matches!(e, UiEvent::Click(_) | UiEvent::Close));
        app.status(e);
    }
}

/// Puts the events the person gave the palette `p` in line, or shows why one was refused. A
/// refused text goes back in its field (`drafts`), and once a change is refused, the clicks and
/// closes after it are not given, so no handler runs without the change before it.
fn give(app: &mut VectorcraftApp, ctx: &egui::Context, p: &ScriptPalette, drafts: &mut BTreeMap<u64, String>, events: Vec<UiEvent>) {
    let mut change_refused = false;
    for e in events {
        if change_refused && matches!(e, UiEvent::Click(_) | UiEvent::Close) {
            continue;
        }
        let change = match &e {
            UiEvent::Change(i, value) => Some((*i, if let UiValue::Text(t) = value { Some(t.clone()) } else { None })),
            _ => None,
        };
        match app.session.script_palette_event(p.id, e) {
            // The next frame starts its run.
            Ok(_) => ctx.request_repaint(),
            Err(error) => {
                if let Some((i, text)) = change {
                    if let Some((n, text)) = p.dialog.nodes.get(i).zip(text) {
                        drafts.insert(n.id, text);
                    }
                    change_refused = true;
                }
                app.status(error);
            }
        }
    }
}

/// Runs `f` on palette `id`'s dialog; `None` when it is not open.
#[cfg(test)]
pub(crate) fn with_palette<R>(app: &VectorcraftApp, id: u64, f: impl FnOnce(&mut PaletteWindow) -> R) -> Option<R> {
    with_host(app, |host, _| host.get_mut_by_id::<PaletteWindow>(palette_id(id)).map(f)).flatten()
}

/// Starts the run of the oldest waiting palette event when no run is going. → whether one started.
pub(crate) fn start_next(app: &mut VectorcraftApp) -> bool {
    match app.session.script_palette_start() {
        None => false,
        Some(Ok(_)) => true,
        Some(Err(e)) => {
            crate::script_runs::failed(app, e.to_string());
            false
        }
    }
}
