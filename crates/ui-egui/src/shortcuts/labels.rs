use egui::text_selection::LabelSelectionState;

/// The Copy or Cut command left to the labels of viewport `viewport` in this pass.
fn waiting_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique(("copy-waits-for-labels", viewport))
}

/// The Copy or Cut command to run in the next pass: the labels it was left to copied nothing.
fn after_labels_id() -> egui::Id {
    egui::Id::unique("copy-after-labels")
}

/// Set when a label in a window of its own (a floating panel's) copied its selected text: the
/// window then sends the Copy or Cut on to the main window ([`crate::panel_docking::forward_keys`]).
/// It lasts until the main window's next pass starts.
fn copied_in_window_id() -> egui::Id {
    egui::Id::unique("labels-copied-in-window")
}

/// Watch the labels' copies from the next pass on ([`leave_to_labels`]). egui runs a plugin only
/// from the pass after the one that adds it.
pub(super) fn watch_label_copies(ctx: &egui::Context) {
    if ctx.plugin_opt::<CopyAfterLabels>().is_none() {
        ctx.add_plugin(CopyAfterLabels);
    }
}

/// Copy or Cut `command` (`edit.copy`, `edit.cut`) while text is selected in a label, such as a
/// Document Info value or an Info panel readout: egui copies that text when the Copy or Cut event
/// reaches the labels, so the event is left in the input for them (→ true). When no label of this
/// viewport copies anything in this pass (the selected text is in another window, or its label is
/// not drawn), `command` runs in the next pass ([`take_command_after_labels`]).
pub(crate) fn leave_to_labels(ctx: &egui::Context, command: &'static str) -> bool {
    let selected = ctx.with_plugin(|s: &mut LabelSelectionState| s.has_selection()).unwrap_or(false);
    if !selected || ctx.plugin_opt::<CopyAfterLabels>().is_none() {
        return false;
    }
    let id = waiting_id(ctx.viewport_id());
    ctx.data_mut(|d| d.insert_temp(id, command));
    true
}

/// The Copy or Cut command left to this viewport's labels in this pass, if any.
pub(super) fn left_to_labels(ctx: &egui::Context) -> Option<&'static str> {
    let id = waiting_id(ctx.viewport_id());
    ctx.data(|d| d.get_temp::<&'static str>(id))
}

/// The Copy or Cut command an earlier pass left to labels that copied nothing.
pub(super) fn take_command_after_labels(ctx: &egui::Context) -> Option<&'static str> {
    ctx.data_mut(|d| d.remove_temp::<&'static str>(after_labels_id()))
}

/// Note that a floating panel's window copied its label text, as [`CopyAfterLabels`] does.
#[cfg(test)]
pub(crate) fn note_copied_in_window(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(copied_in_window_id(), true));
}

/// Did a label in another window copy its selected text since the last call? The Copy or Cut that
/// window sends on is then its own and copies nothing more.
pub(crate) fn take_copied_in_window(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(copied_in_window_id())).unwrap_or(false)
}

/// Runs after egui's label text selection at the end of each pass, when the labels have copied
/// their selected text.
struct CopyAfterLabels;

impl egui::Plugin for CopyAfterLabels {
    fn debug_name(&self) -> &'static str {
        "CopyAfterLabels"
    }

    /// The windows of their own run their passes inside the main window's pass, and their Copy or
    /// Cut is sent on in that pass: a copy noted before it is not theirs.
    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
            take_copied_in_window(ui.ctx());
        }
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        let copied = ctx.output(|o| o.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
        // A field with the keyboard copies its own text, and its window sends nothing on.
        if ctx.viewport_id() != egui::ViewportId::ROOT
            && copied
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)))
        {
            ctx.data_mut(|d| d.insert_temp(copied_in_window_id(), true));
        }
        let id = waiting_id(ctx.viewport_id());
        let Some(command) = ctx.data_mut(|d| d.remove_temp::<&'static str>(id)) else {
            return;
        };
        if !copied {
            ctx.data_mut(|d| d.insert_temp(after_labels_id(), command));
            ctx.request_repaint();
        }
    }
}
