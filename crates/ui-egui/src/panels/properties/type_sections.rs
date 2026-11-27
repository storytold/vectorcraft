//! The Properties panel's Character and Paragraph sections for the selected text.

use craft::text::ui::character::CharacterRows;
use craft::ui::contract::{LabelScheme, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use egui::Ui;
use vectorcraft_doc::NodeKind;

use super::{dynamic_text, more_options, vertical_align};
use crate::VectorcraftApp;
use crate::panels::{character, first_selected, paragraph};

/// The Character and Paragraph sections of the selected text; a note while no text is selected.
pub fn type_sections(app: &mut VectorcraftApp, ui: &mut Ui) {
    if !matches!(first_selected(app).as_deref().map(|n| &n.kind), Some(NodeKind::Text(_))) {
        Text::new(tl!("Select a text object")).shade(Shade::Muted).show(ui);
        return;
    }
    // The style of the Type tool's selection (or caret) when it edits text, else of the selected text's first run,
    // as the Character panel shows it (#8189).
    if character::text_style(app).is_none() {
        return;
    }
    // The Character panel's controls as that panel lays them out: the font pickers and the metrics fill the row.
    super::section(tl!("Character")).show(ui, |ui, _| {
        Stack::new().fill_width().labels(LabelScheme::Inspector).show(ui, |ui, _| character::controls(app, ui, CharacterRows::PROPERTIES));
    });
    super::section(tl!("Paragraph")).show(ui, |ui, _| {
        // As in the Paragraph panel: the seven alignments and Hyphenate (#775), and area type's vertical alignment
        // (#3759); the rest is there.
        Row::new().show(ui, |ui, _| {
            paragraph::alignment_toggles(app, ui, 7);
            more_options(ui, "paragraph", tl!("More Paragraph options"));
        });
        if vertical_align::shown(app).is_some() {
            Row::new().show(ui, |ui, _| {
                vertical_align::buttons(app, ui);
            });
        }
        paragraph::hyphenate_check(app, ui);
        // Area type: Dynamic Text's Block and its gaps.
        dynamic_text::rows(app, ui);
    });
}
