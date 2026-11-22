//! The ScriptUI palettes a script shows with `new Window("palette")` in a named engine
//! ([`vectorcraft_engine::script::palettes`]): each shown palette is a window beside the documents
//! that stays open after the run, with the controls the engine laid out. A click, a change or the
//! window's close box waits in the session's line and runs as a script run of its own once no run
//! is going ([`crate::script_runs::poll`]); the palette stays usable while a run goes on, and its
//! events wait for it. A palette does not take the keyboard focus when it opens or changes. An edit
//! field's text is given when it loses the focus, at Enter in a one-line field, or before a click
//! or a close; a text the script would refuse stays in its field with the error in the status bar.

use serde_json::json;
use vectorcraft_engine::script::{ScriptPalette, UiEvent};

use crate::VectorcraftApp;
use crate::dialogs::script_window;
use crate::state::Dialog;

/// The largest part of a palette's content shown at once; a larger palette scrolls.
const MAX_SHOWN: egui::Vec2 = egui::vec2(960.0, 640.0);

/// Where the text typed in a palette's fields is kept between frames.
fn drafts_id(id: u64) -> egui::Id {
    egui::Id::unique(("script-palette-drafts", id))
}

/// Draws each shown palette, and puts the events the person gives in line.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let shown: Vec<ScriptPalette> = app.session.script_palettes().iter().filter(|p| p.visible).cloned().collect();
    for p in shown {
        let mut drafts = ctx.data(|m| m.get_temp::<Dialog>(drafts_id(p.id))).unwrap_or_else(|| Dialog::new(script_window::KIND, json!({})));
        let mut events = Vec::new();
        let mut open = true;
        let title = p.dialog.title();
        let title = if title.is_empty() { tl!("Script Alert").to_string() } else { vectorcraft_engine::script::dialogs::shown_text(title, false) };
        let (width, height) = p.dialog.size();
        let size = egui::vec2(width.max(1.0), height.max(1.0));
        egui::Window::new(title).id(egui::Id::unique(("script-palette", p.id))).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::ScrollArea::both().max_width(MAX_SHOWN.x).max_height(MAX_SHOWN.y).auto_shrink([true, true]).show(ui, |ui| {
                let (area, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                for (i, n) in p.dialog.nodes.iter().enumerate().skip(1) {
                    if n.visible
                        && let Some(e) = script_window::control(ui, &mut drafts, area.min, (i, n, script_window::prompt_of(&p.dialog, i)), false)
                    {
                        events.push(e);
                    }
                }
            });
        });
        if !open {
            events.push(UiEvent::Close);
        }
        drafted(app, &p, &mut drafts, &mut events);
        ctx.data_mut(|m| m.insert_temp(drafts_id(p.id), drafts));
        give(app, ctx, p.id, events);
    }
}

/// Puts the text still typed in palette `p`'s fields (`drafts`) before a click or a close in
/// `events`. A text the script would refuse stays in its field with its error in the status bar,
/// and a click or a close waits for the person to fix it.
pub(crate) fn drafted(app: &mut VectorcraftApp, p: &ScriptPalette, drafts: &mut Dialog, events: &mut Vec<UiEvent>) {
    if let Some(e) = script_window::keep_refused_texts(&p.dialog, drafts, events) {
        app.status(e);
    }
    if events.iter().any(|e| matches!(e, UiEvent::Click(_) | UiEvent::Close))
        && let Err(e) = script_window::commit_drafts(&p.dialog, drafts, events)
    {
        events.retain(|e| !matches!(e, UiEvent::Click(_) | UiEvent::Close));
        app.status(e);
    }
}

/// Puts the events the person gave the palette `id` in line, or shows why one was refused.
pub(crate) fn give(app: &mut VectorcraftApp, ctx: &egui::Context, id: u64, events: Vec<UiEvent>) {
    for e in events {
        match app.session.script_palette_event(id, e) {
            // The next frame starts its run.
            Ok(_) => ctx.request_repaint(),
            Err(e) => app.status(e),
        }
    }
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
