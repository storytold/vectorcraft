//! Effect → Document Raster Effects Settings: resolution, colour model, background, and the
//! anti-alias, clipping mask, room around the art and spot colour options.
//!
//! Fields: what `document.rasterEffectsSettings` reports (and takes back), with the resolution as
//! a preset (`resolutionPreset`). OK runs the command with them: one undo step. On an error the
//! dialog stays open. [`params`] lays out the options of Effect › Rasterize… with the same rows.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Map, Value, json};
use vectorcraft_effects::RASTERIZE;
use vectorcraft_render::AntiAlias;

use super::effect_params::{EffectCx, own_choices};
use super::form::{fields_of, json_of};
use super::halftone::options;
use super::host::open_held;
use crate::VectorcraftApp;

const CMD: &str = "document.rasterEffectsSettings";
/// The resolution presets: (ppi, label).
const RESOLUTIONS: [(f64, &str); 3] = [(72.0, "Screen (72 ppi)"), (150.0, "Medium (150 ppi)"), (300.0, "High (300 ppi)")];

/// Sets `resolutionPreset` from `resolution` and `useDocumentResolution`: Use Document Raster
/// Effects Resolution, the preset whose ppi the resolution is, or Other.
// PLAN: effects-w4's `effect::open` calls this when the Rasterize effect opens.
pub(crate) fn preset_of(params: &mut Map<String, Value>) {
    let id = if params.get("useDocumentResolution").and_then(Value::as_bool) == Some(true) {
        "document".to_string()
    } else {
        let ppi = params.get("resolution").and_then(Value::as_f64).unwrap_or(72.0);
        RESOLUTIONS.iter().find(|(p, _)| (*p - ppi).abs() < 1e-9).map_or("other".to_string(), |(p, _)| p.to_string())
    };
    params.insert("resolutionPreset".into(), Value::String(id));
}

/// Turns `resolutionPreset` back into `resolution` and `useDocumentResolution`, and removes it.
/// `useDocumentResolution` is written only for the Rasterize effect, whose values carry it.
// PLAN: effects-w4's `effect::command` calls this when the Rasterize effect's params are built.
pub(crate) fn resolution_of(params: &mut Map<String, Value>) {
    let Some(Value::String(preset)) = params.remove("resolutionPreset") else { return };
    let document = preset == "document";
    if let Some((ppi, _)) = RESOLUTIONS.iter().find(|(p, _)| p.to_string() == preset) {
        params.insert("resolution".into(), json!(ppi));
    }
    if document || params.contains_key("useDocumentResolution") {
        params.insert("useDocumentResolution".into(), json!(document));
    }
}

/// The rows of the settings dialog, or with `effect` those of Effect › Rasterize…: Resolution (a
/// preset, with Use Document Raster Effects Resolution for the effect, or Other), Color Model,
/// Background, then the options.
pub(crate) fn params(cx: &EffectCx, effect: bool) -> Vec<Param> {
    let mut presets: Vec<(String, &str)> = RESOLUTIONS.iter().map(|(p, label)| (p.to_string(), *label)).collect();
    if effect {
        presets.push(("document".into(), "Use Document Raster Effects Resolution"));
    }
    presets.push(("other".into(), "Other"));
    let models: Vec<(&str, &str)> = if effect {
        own_choices(RASTERIZE, "colorModel", cx.cmyk).unwrap_or_default().into_iter().map(|(label, id)| (id, label)).collect()
    } else {
        vec![if cx.cmyk { ("cmyk", "CMYK") } else { ("rgb", "RGB") }, ("grayscale", "Grayscale"), ("bitmap", "Bitmap")]
    };
    let backgrounds: Vec<(&str, &str)> =
        own_choices(RASTERIZE, "background", false).unwrap_or_default().into_iter().map(|(label, id)| (id, label)).collect();
    let mut rows = vec![
        Param::new("resolutionPreset", tl!("Resolution:"), ParamKind::choice(options(&presets), Some(if effect { "document" } else { "72" }.into()))),
        Param::new("resolution", tl!("Resolution:"), ParamKind::int(Some(1), None, 72))
            .visible_when("resolutionPreset", "other")
            .unit(UnitKind::PixelsPerInch),
        Param::new("colorModel", tl!("Color Model:"), ParamKind::choice(options(&models), models.first().map(|(id, _)| id.to_string()))),
        Param::new("background", tl!("Background:"), ParamKind::radio(options(&backgrounds), Some("white".into()))),
        Param::section(tl!("Options")),
    ];
    rows.push(if effect {
        let aa = AntiAlias::ALL.map(|a| (a.id(), a.label()));
        Param::new("antiAlias", tl!("Anti-aliasing:"), ParamKind::choice(options(&aa), Some(AntiAlias::Art.id().into())))
    } else {
        Param::new("antiAlias", tl!("Anti-alias"), ParamKind::bool(true))
    });
    rows.push(Param::new("clippingMask", tl!("Create Clipping Mask"), ParamKind::bool(false)));
    rows.push(Param::number("addAround", tl!("Add Around Object"), 0.0, 1.0, 2).bounds(Some(0.0), None).length(cx.lengths));
    if !effect {
        rows.push(Param::new("preserveSpotColors", tl!("Preserve Spot Colors"), ParamKind::bool(false)));
    }
    rows
}

/// Open the dialog on the active document's settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let values = app.session.execute(CMD, &json!({})).map_err(|e| e.to_string())?;
    let mut values = values.as_object().cloned().unwrap_or_default();
    preset_of(&mut values);
    let rows = params(&EffectCx::new(app, CMD, false), false);
    let initial = fields_of(&rows, &values);
    let (shown, answered) = (rows.clone(), rows);
    let dialog = HostedParamDialog::command(&ctx, CMD, initial, move |fields| {
        ParamDialog::command(CMD, tl!("Document Raster Effects Settings"), shown.clone())
            .footer_note(tl!("Changing these settings changes how raster effects look when exported, flattened or expanded."))
            .values(Bind::bound(fields))
    });
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| {
        let mut params = json_of(&answered, &result.fields);
        resolution_of(&mut params);
        app.run(CMD, Value::Object(params)).map(|_| ())
    };
    open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::{DialogId, StackDialog};
    use craft::ui::patterns::form_dialogs::HostedParamDialog;
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    /// An app with a window and a 200 × 200 document.
    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app
    }

    fn pump(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        crate::dialogs::host::pump(app, &ctx);
    }

    /// The text of field `key` of the open dialog `id`.
    fn text(app: &VectorcraftApp, id: DialogId, key: &str) -> Option<String> {
        app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().text(key).map(str::to_string)))
    }

    fn set(app: &mut VectorcraftApp, dialog: Option<&str>, field: &str, value: serde_json::Value) {
        let mut params = json!({"field": field, "value": value});
        if let Some(dialog) = dialog {
            params["dialog"] = json!(dialog);
        }
        // A control channel method, not an engine command.
        let ctx = app.ui_ctx.clone().unwrap();
        let (req, _) = craft::automation::ControlRequest::new("ui.dialog.set", params);
        let craft::automation::Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("ui.dialog.set answers at once") };
        assert_eq!(r["ok"], true, "{field}: {r}");
    }

    /// OK as the button does it, the pump that answers the held OK, and the close the accepted answer waits for.
    fn ok(app: &mut VectorcraftApp, id: DialogId) {
        let ctx = app.ui_ctx.clone().unwrap();
        app.dialogs().unwrap().with(|h| h.confirm_by_id(&ctx, id));
        pump(app);
        app.dialogs().unwrap().with(|h| h.confirm_by_id(&ctx, id));
        pump(app);
    }

    #[test]
    fn raster_effects_settings_draw_and_ok_is_one_undo_step() {
        let mut app = app();
        crate::menus::invoke(&mut app, "document.rasterEffectsSettings", json!({}));
        let id = app.dialogs().unwrap().with(|h| h.get::<HostedParamDialog>().map(StackDialog::id)).unwrap();
        assert_eq!(text(&app, id, "colorModel").as_deref(), Some("rgb"));
        // `ui.dialog.set` names the dialog by the id `ui.inspect` lists.
        let listed = id.value().to_string();
        for (k, v) in
            [("resolutionPreset", json!("300")), ("background", json!("white")), ("addAround", json!(36)), ("colorModel", json!("grayscale"))]
        {
            set(&mut app, Some(&listed), k, v);
        }
        // Other resolutions show their own field.
        set(&mut app, Some(&listed), "resolutionPreset", json!("other"));
        set(&mut app, Some(&listed), "resolution", json!(200));
        ok(&mut app, id);
        assert!(app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).is_none()), "OK closes the dialog");
        let st = app.session.active().unwrap();
        let r = &st.doc.raster_effects;
        assert_eq!((st.doc.raster_effects_ppi, r.add_around, r.background), (200.0, 36.0, vectorcraft_doc::Background::White));
        assert_eq!(r.color_model, vectorcraft_doc::RasterColorModel::Grayscale);
        assert_eq!(st.history.undo.len(), 1);
        // Rasterize… starts from these settings.
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        crate::menus::invoke(&mut app, "object.rasterize", json!({}));
        let rasterize = crate::tests::picks::open_dialogs(&app).pop().expect("Rasterize… is open");
        let fields = rasterize.fields.unwrap_or_default();
        assert_eq!((fields["ppi"].as_f64(), fields["background"].as_str(), fields["addAround"].as_f64()), (Some(200.0), Some("white"), Some(36.0)));
        ok(&mut app, rasterize.id);
        let st = app.session.active().unwrap();
        assert!(st.selection.objects.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Image(_)))));
    }

    #[test]
    fn rasterize_effect_dialog_previews_and_ok_applies_its_options_as_one_undo_step() {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let steps = app.session.active().unwrap().history.undo.len();
        crate::menus::invoke(&mut app, "effect.dialog", json!({"effect": "rasterize"}));
        let dialog = app.effect_dialogs.effect.as_ref().unwrap().id;
        assert_eq!(text(&app, dialog, "resolutionPreset").as_deref(), Some("document"));
        pump(&mut app);
        assert!(app.session.in_interaction(), "previews");
        set(&mut app, None, "resolutionPreset", json!("other"));
        for (k, v) in [("resolution", json!(200)), ("background", json!("white")), ("antiAlias", json!("none"))] {
            set(&mut app, None, k, v);
        }
        pump(&mut app);
        ok(&mut app, dialog);
        let st = app.session.active().unwrap();
        let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        let (e, p) = (&n.appearance.effects[0], &n.appearance.effects[0].params);
        assert_eq!((e.id.as_str(), &p["resolution"], &p["background"], &p["antiAlias"]), ("rasterize", &json!(200), &json!("white"), &json!("none")));
        assert_eq!(p["useDocumentResolution"], json!(false));
        assert!(p.get("resolutionPreset").is_none());
        assert!(matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. }), "the rectangle stays a path");
        assert_eq!(st.history.undo.len(), steps + 1);
    }
}
