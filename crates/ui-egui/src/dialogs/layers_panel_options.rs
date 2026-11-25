//! Layers panel › Panel Options…: Show Layers Only, Locate Selected Objects, the row size and
//! which rows show thumbnails.
//!
//! Fields: `layersOnly`, `locateSelected`, `rowSize` (`small`, `medium` or `large`), `thumbLayers`,
//! `thumbGroups` and `thumbObjects`. OK applies them to the panel (kept with the UI state).

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use serde_json::Value;

use super::host::open_held;
use crate::VectorcraftApp;
use crate::panels::layers::{PanelOptions, RowSize};

/// The command Layers Panel Options answers for; the key of its dialog.
const CMD: &str = "ui.layersPanelOptions";

/// The row sizes the dialog offers: the field's value and the size it stands for.
const SIZES: [(&str, RowSize); 3] = [("small", RowSize::Small), ("medium", RowSize::Medium), ("large", RowSize::Large)];

/// The dialog's rows.
// PLAN: the sheet names `Param::bool`/`Param::choice`; the framework has `Param::new(key, label, ParamKind::..)` only.
fn rows() -> Vec<Param> {
    let sizes = [("small", tl!("Small")), ("medium", tl!("Medium")), ("large", tl!("Large"))];
    let sizes = sizes.into_iter().map(|(id, label)| (id.to_owned(), label.into())).collect();
    vec![
        Param::new("layersOnly", tl!("Show Layers Only"), ParamKind::bool(false)),
        Param::new("locateSelected", tl!("Locate Selected Objects"), ParamKind::bool(false)),
        Param::new("rowSize", tl!("Row Size:"), ParamKind::choice(sizes, Some("medium".to_owned()))),
        Param::section(tl!("Thumbnails:")),
        Param::new("thumbLayers", tl!("Layers"), ParamKind::bool(true)),
        Param::new("thumbGroups", tl!("Group"), ParamKind::bool(true)),
        Param::new("thumbObjects", tl!("Object"), ParamKind::bool(true)),
    ]
}

/// `ui.layersPanelOptions`: open the dialog on the panel's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let o = &app.ui.layers_panel;
    let mut fields = DialogFields::new();
    fields.set("layersOnly", o.layers_only);
    fields.set("locateSelected", o.locate_selected);
    fields.set("rowSize", SIZES.iter().find(|s| s.1 == o.row_size).map_or("medium", |s| s.0));
    fields.set("thumbLayers", o.thumb_layers);
    fields.set("thumbGroups", o.thumb_groups);
    fields.set("thumbObjects", o.thumb_objects);
    let dialog = HostedParamDialog::command(&ctx, CMD, fields, |fields| {
        ParamDialog::command(CMD, tl!("Layers Panel Options"), rows()).values(Bind::bound(fields))
    });
    let ok = |app: &mut VectorcraftApp, result: &ParamResult| {
        let f = &result.fields;
        let flag = |key: &str| f.bool(key).unwrap_or_default();
        app.ui.layers_panel = PanelOptions {
            layers_only: flag("layersOnly"),
            locate_selected: flag("locateSelected"),
            row_size: SIZES.iter().find(|s| Some(s.0) == f.text("rowSize")).map_or(RowSize::Medium, |s| s.1),
            thumb_layers: flag("thumbLayers"),
            thumb_groups: flag("thumbGroups"),
            thumb_objects: flag("thumbObjects"),
        };
        Ok(())
    };
    open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app
    }

    /// The open dialog's id and fields.
    fn open_dialog(app: &VectorcraftApp) -> (craft::ui::dialog::DialogId, Value) {
        let open = app.dialogs().unwrap().with(|host| host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(CMD))).unwrap();
        (open.id, open.fields.unwrap())
    }

    /// Sets `edits` in the open dialog, presses OK and runs the host pump, which runs the held OK.
    fn apply(app: &mut VectorcraftApp, edits: &[(&str, Value)]) {
        let ctx = app.ui_ctx.clone().unwrap();
        let (id, _) = open_dialog(app);
        let dialogs = app.dialogs().unwrap();
        for (field, value) in edits {
            dialogs.with(|host| host.set_json_field(&ctx, id, field, value.clone())).unwrap();
        }
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
    }

    #[test]
    fn panel_options_round_trip_and_apply() {
        let mut app = app();
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        assert_eq!(open_dialog(&app).1["rowSize"], json!("medium"));
        apply(&mut app, &[("layersOnly", json!(true)), ("rowSize", json!("large")), ("thumbObjects", json!(false))]);
        let o = &app.ui.layers_panel;
        assert!(o.layers_only && !o.thumb_objects && o.thumb_layers);
        assert_eq!(o.row_size, RowSize::Large);
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        assert_eq!(open_dialog(&app).1["rowSize"], json!("large"), "opens on the saved size");
        // Options saved with a row height in points load as the nearest size.
        let old: crate::UiState = serde_json::from_value(json!({"layers_panel": {"rowSize": 500}})).unwrap();
        assert_eq!(old.layers_panel.row_size, RowSize::Large);
    }

    /// Locate Selected Objects is off by default and in options saved without it; OK turns it on
    /// and keeps it with the UI state, and the dialog opens on it again.
    #[test]
    fn locate_selected_objects_is_off_by_default_and_kept_with_the_ui_state() {
        let mut app = app();
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        assert_eq!(open_dialog(&app).1["locateSelected"], json!(false));
        apply(&mut app, &[("locateSelected", json!(true))]);
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(back.layers_panel.locate_selected, "kept with the UI state");
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        assert_eq!(open_dialog(&app).1["locateSelected"], json!(true));
        let old: crate::UiState = serde_json::from_value(json!({"layers_panel": {"layersOnly": true}})).unwrap();
        assert!(old.layers_panel.layers_only && !old.layers_panel.locate_selected, "options saved without it");
    }
}
