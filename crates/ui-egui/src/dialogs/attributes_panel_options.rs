//! Attributes panel › Panel Options…: how many recent URLs the panel's URL menu lists.
//!
//! Field: `count` (1–30). OK runs `ui.attributesPanelOptions` with it (kept with the UI state).

use serde_json::{Value, json};

use super::DialogSpec;
use super::swatch_options::{grid, label};
use crate::panels::attributes::DEFAULT_RECENT_URLS;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Attributes Panel Options.
pub const KIND: &str = "attributesPanelOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Attributes Panel Options").into(), body, confirm, min_width: 280.0, ..DialogSpec::FORM };

/// `ui.attributesPanelOptions` without options: open the dialog on the panel's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let count = app.ui.attributes_panel.shown();
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "count": count })));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Number of Recent URLs:"));
        let v = d.f64("count", f64::from(DEFAULT_RECENT_URLS));
        let max = vectorcraft_engine::cmd::RECENT_URLS as f64;
        if let Some(v) = widgets::range_field(ui, "attributes-recent-urls", v, 1.0..=max, "", 0, 60.0) {
            d.fields.insert("count".into(), json!(v));
        }
        ui.end_row();
    });
    false
}

/// OK: set the count; a count that is refused leaves the dialog open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = json!({ "count": d.fields.get("count").cloned().unwrap_or(Value::Null) });
    let r = app.run("ui.attributesPanelOptions", params)?;
    app.ui.dialog = None;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// The dialog opens on the panel's count (10 by default), OK sets it, kept with the UI state,
    /// and a count out of range is refused, leaving the dialog open and the count as it was.
    #[test]
    fn panel_options_set_the_recent_url_count() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("ui.attributesPanelOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.f64("count", 0.0)), (KIND, 10.0));
        // Drawn headlessly in the shared frame.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let set = |app: &mut VectorcraftApp, count: Value| {
            app.ui.dialog.as_mut().unwrap().fields.insert("count".into(), count);
            super::super::confirm(app)
        };
        assert_eq!(set(&mut app, json!(25.0)).unwrap(), json!({"count": 25}));
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.ui.attributes_panel.recent_urls, 25);
        app.run("ui.attributesPanelOptions", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("count", 0.0), 25.0, "opens on the current count");
        assert!(set(&mut app, json!(31)).is_err());
        assert!(app.ui.dialog.is_some() && app.ui.attributes_panel.recent_urls == 25, "a refused count changes nothing");
        // Cancel closes the dialog without applying what was typed.
        app.ui.dialog.as_mut().unwrap().fields.insert("count".into(), json!(3));
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.ui.attributes_panel.recent_urls, 25);
    }
}
