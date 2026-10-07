//! Recover Documents (shown at launch when the last session left recovery copies behind):
//! Restore / Discard / Cancel (ask again next launch). The flow lives in [`crate::recovery`]; this
//! is its face in the shared dialog frame.

use serde_json::Value;

use super::DialogSpec;
use crate::theme::Tokens;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Recover Documents").into(),
    body: |_, ui, d| {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("Vector W3K2 didn't quit normally last time. These documents had unsaved changes:")).color(t.text_dim));
        ui.add_space(8.0);
        let copies = d.fields.get("copies").and_then(Value::as_array);
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for c in copies.into_iter().flatten() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(c["title"].as_str().unwrap_or_default()).color(t.text));
                    if let Some(at) = c["saved"].as_i64().and_then(|s| u64::try_from(s).ok()) {
                        ui.label(egui::RichText::new(crate::panels::links::date_label(at.saturating_mul(1000))).color(t.text_dim));
                    }
                });
            }
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tl!("Restored documents open unsaved: save them to keep them.")).color(t.text_dim));
        false
    },
    confirm: crate::recovery::confirm,
    ok: Some("Restore"),
    discard: Some("Discard"),
    min_width: 380.0,
    max_width: Some(460.0),
    ..DialogSpec::FORM
};
