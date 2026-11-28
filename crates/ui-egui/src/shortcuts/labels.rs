//! Copy and Cut while text is selected in a label, such as a Document Info value or an Info panel readout. The key
//! stage leaves the Copy or Cut event in egui's input then, for the labels to copy their text. When no label of the
//! window copies anything in that pass (the selected text is in another window, or its label is not drawn), the
//! command runs in the next pass, on the selection it was given.

use craft::ui::runtime::windows::WindowHost;
use egui::text_selection::LabelSelectionState;
use serde_json::json;
use vectorcraft_doc::{NodeId, Selection};

use crate::VectorcraftApp;

/// A Copy or Cut command left to the labels, with the active document (its `uid`) and the
/// selection it was given then.
#[derive(Clone, Default)]
struct LeftToLabels {
    id: &'static str,
    given: Option<(u64, Selection)>,
}

/// The Copy or Cut command left to the labels of viewport `viewport` in this pass.
fn waiting_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique(("copy-waits-for-labels", viewport))
}

/// How many texts viewport `viewport` had copied in this pass before its labels could copy theirs.
fn copies_before_labels_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique(("copies-before-labels", viewport))
}

/// The Copy or Cut command to run in the next pass: the labels it was left to copied nothing.
fn after_labels_id() -> egui::Id {
    egui::Id::unique("copy-after-labels")
}

/// The texts copied in this pass of the current viewport so far.
fn copies(ctx: &egui::Context) -> usize {
    ctx.output(|o| o.commands.iter().filter(|c| matches!(c, egui::OutputCommand::CopyText(_))).count())
}

/// First in a pass: the Copy or Cut an earlier pass left to labels that copied nothing runs now, on the selection it
/// was given ([`run_after_labels`]). Watches the labels' copies from the next pass on: egui runs a plugin only from
/// the pass after the one that adds it.
pub(crate) fn run_left_by_labels(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if ctx.plugin_opt::<CopyAfterLabels>().is_none() {
        ctx.add_plugin(CopyAfterLabels);
    }
    if let Some(c) = ctx.data_mut(|d| d.remove_temp::<LeftToLabels>(after_labels_id())) {
        run_after_labels(app, c);
    }
}

/// After the pass's input, once the text it copied has gone out: a Copy or Cut event still in the input while text
/// is selected in a label, and no text field has the keys, is the labels'. The texts copied so far are noted, so a
/// copy at the end of the pass beyond them is the labels' ([`CopyAfterLabels`]). The labels' text, or the art copied
/// in the next pass when they copy nothing, replaces what an earlier Copy or Cut has not written yet. Every window
/// calls it in its own pass, the OS windows of floating panels and documents included: the key stage of each window
/// leaves its Copy or Cut to the labels while text is selected in a label of any window.
pub(crate) fn leave_to_labels(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    let n = copies(ctx);
    ctx.data_mut(|d| d.insert_temp(copies_before_labels_id(viewport), n));
    let command = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Copy => Some("edit.copy"),
            egui::Event::Cut => Some("edit.cut"),
            _ => None,
        })
    });
    let selected = ctx.with_plugin(|s: &mut LabelSelectionState| s.has_selection()).unwrap_or(false);
    let Some(command) = command.filter(|_| selected && !WindowHost::of(ctx).typing() && ctx.plugin_opt::<CopyAfterLabels>().is_some()) else {
        return;
    };
    let given = active_selection(app);
    ctx.data_mut(|d| d.insert_temp(waiting_id(viewport), LeftToLabels { id: command, given }));
    app.drop_unwritten_formats();
}

/// The `uid` of `app`'s active document and its selection.
fn active_selection(app: &VectorcraftApp) -> Option<(u64, Selection)> {
    app.session.active().map(|st| (st.uid, st.selection.clone()))
}

/// Run Copy or Cut `c`, which labels left to this pass, on the selection it was given. A selection
/// made in the same frame after it (a Select All) comes back after it, without the objects a Cut
/// removed and with the pieces it left of a path it split. With another document active by now,
/// or none, it does not run. The text it copies goes out now, so it is not taken for a label's copy.
fn run_after_labels(app: &mut VectorcraftApp, c: LeftToLabels) {
    let Some((doc, given)) = c.given else {
        crate::menus::invoke(app, c.id, json!({}));
        app.flush_clipboard_out();
        return;
    };
    match active_selection(app) {
        None => {}
        Some((active, _)) if active != doc => {}
        Some((_, now)) if now != given => {
            if let Err(e) = app.session.select(|_, sel| *sel = given.clone()) {
                app.status(e.to_string());
                return;
            }
            let before = app.session.active().map(|st| st.doc.clone());
            crate::menus::invoke(app, c.id, json!({}));
            // The pieces a Cut left of a path it split are new objects: they join the later
            // selection when it held an object the Cut acted on.
            let acted_on = given.objects.iter().any(|id| now.contains(*id));
            let pieces: Vec<NodeId> = match (before, app.session.active()) {
                (Some(before), Some(st)) if acted_on => st.selection.objects.iter().copied().filter(|id| before.node(*id).is_none()).collect(),
                _ => vec![],
            };
            if let Err(e) = app.session.select(|_, sel| {
                *sel = now;
                sel.add_all(pieces);
            }) {
                app.status(e.to_string());
            }
        }
        _ => crate::menus::invoke(app, c.id, json!({})),
    }
    app.flush_clipboard_out();
}

/// Runs after egui's label text selection at the end of each pass, when the labels have copied
/// their selected text.
struct CopyAfterLabels;

impl egui::Plugin for CopyAfterLabels {
    fn debug_name(&self) -> &'static str {
        "CopyAfterLabels"
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        let viewport = ctx.viewport_id();
        // The art a Copy or Cut copied earlier in this pass is not the labels' copy.
        let before = ctx.data_mut(|d| d.remove_temp::<usize>(copies_before_labels_id(viewport))).unwrap_or(0);
        let Some(command) = ctx.data_mut(|d| d.remove_temp::<LeftToLabels>(waiting_id(viewport))) else {
            return;
        };
        // In a pass egui discards too: its rerun gets no input, so nothing leaves the command to the labels again.
        if copies(ctx) == before {
            ctx.data_mut(|d| d.insert_temp(after_labels_id(), command));
            ctx.request_repaint();
        }
    }
}
