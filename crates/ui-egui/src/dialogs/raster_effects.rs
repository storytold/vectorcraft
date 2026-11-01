//! Effect → Document Raster Effects Settings: resolution, colour model, background, and the
//! anti-alias, clipping mask, room around the art and spot colour options.
//!
//! Fields: what `document.rasterEffectsSettings` reports (and takes back). OK runs it with them:
//! one undo step. On an error the dialog stays open. [`effect_fields`] lays out the options of
//! Effect › Rasterize… with the same widgets.

use serde_json::{Value, json};
use vectorcraft_effects::RASTERIZE;

use super::DialogSpec;
use super::document_setup::{LABEL, check, choice};
use super::effect::own_choices;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "rasterEffectsSettings";
const CMD: &str = "document.rasterEffectsSettings";
/// The resolution presets: (ppi, label).
const RESOLUTIONS: [(f64, &str); 3] = [(72.0, "Screen (72 ppi)"), (150.0, "Medium (150 ppi)"), (300.0, "High (300 ppi)")];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Document Raster Effects Settings").into(),
    body,
    confirm,
    min_width: 440.0,
    max_width: Some(440.0),
    ..DialogSpec::FORM
};

/// Open the dialog on the active document's settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = app.session.execute(CMD, &json!({})).map_err(|e| e.to_string())?;
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let r = app.run(CMD, Value::Object(d.fields.clone()));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    // Resolution: a preset, or Other with its own value.
    let ppi = d.f64("resolution", 72.0);
    let preset = RESOLUTIONS.iter().position(|(p, _)| (*p - ppi).abs() < 1e-9);
    let mut labels: Vec<&str> = RESOLUTIONS.iter().map(|(_, l)| tl!(l)).collect();
    labels.push(tl!("Other"));
    widgets::label_row(ui, tl!("Resolution:"), LABEL, |ui| {
        if let Some(i) = widgets::dropdown(ui, "raster-resolution", labels[preset.unwrap_or(RESOLUTIONS.len())], &labels, 170.0) {
            // Other keeps the current value to edit.
            if let Some((p, _)) = RESOLUTIONS.get(i) {
                d.fields.insert("resolution".into(), json!(p));
            } else if preset.is_some() {
                d.fields.insert("resolution".into(), json!(ppi.round() + 1.0));
            }
        }
        if preset.is_none()
            && let Some(v) = widgets::plain_field(ui, "raster-ppi", ppi, " ppi", 0, 80.0)
        {
            d.fields.insert("resolution".into(), json!(v));
        }
    });
    ui.add_space(4.0);
    let own =
        if app.session.active().is_some_and(|s| s.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk) { ("cmyk", "CMYK") } else { ("rgb", "RGB") };
    choice(ui, d, "colorModel", tl!("Color Model:"), &[own, ("grayscale", tl!("Grayscale")), ("bitmap", tl!("Bitmap"))]);
    ui.add_space(6.0);
    widgets::label_row(ui, tl!("Background:"), LABEL, |ui| {
        let cur = d.str("background");
        for (id, label) in [("white", tl!("White")), ("transparent", tl!("Transparent"))] {
            if widgets::radio(ui, label, cur == id, true) {
                d.fields.insert("background".into(), json!(id));
            }
            ui.add_space(12.0);
        }
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Options"));
    check(ui, d, "antiAlias", tl!("Anti-alias"));
    check(ui, d, "clippingMask", tl!("Create Clipping Mask"));
    let unit = app.session.general_unit();
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Add")).color(t.text));
        if let Some(v) = widgets::num_field(ui, "raster-add-around", Some(d.f64("addAround", 0.0)), unit, 80.0) {
            d.fields.insert("addAround".into(), json!(v));
        }
        ui.label(egui::RichText::new(tl!("Around Object")).color(t.text));
    });
    check(ui, d, "preserveSpotColors", tl!("Preserve Spot Colors"));
    ui.add_space(8.0);
    super::form::caption(ui, tl!("Changing these settings changes how raster effects look when exported, flattened or expanded."));
    false
}

/// The options of Effect › Rasterize… in its effect dialog, with this dialog's widgets: Color
/// Model, Resolution (a preset, Use Document Raster Effects Resolution or Other) and Background,
/// then an Anti-aliasing menu, Create Clipping Mask and Add … Around Object.
pub(super) fn effect_fields(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let cmyk = app.session.active().is_some_and(|s| s.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk);
    let models: Vec<(&str, &str)> =
        own_choices(RASTERIZE, "colorModel", cmyk).unwrap_or_default().into_iter().map(|(label, value)| (value, label)).collect();
    choice(ui, d, "colorModel", tl!("Color Model:"), &models);
    ui.add_space(4.0);
    let (ppi, document) = (d.f64("resolution", 72.0), d.bool("useDocumentResolution"));
    let preset = RESOLUTIONS.iter().position(|(p, _)| (*p - ppi).abs() < 1e-9);
    let mut labels: Vec<&str> = RESOLUTIONS.iter().map(|(_, l)| tl!(l)).collect();
    labels.push(tl!("Use Document Raster Effects Resolution"));
    labels.push(tl!("Other"));
    let shown = if document { RESOLUTIONS.len() } else { preset.unwrap_or(RESOLUTIONS.len() + 1) };
    widgets::label_row(ui, tl!("Resolution:"), LABEL, |ui| {
        if let Some(i) = widgets::dropdown(ui, "rasterize-resolution", labels.get(shown).copied().unwrap_or_default(), &labels, 260.0) {
            d.fields.insert("useDocumentResolution".into(), json!(i == RESOLUTIONS.len()));
            if let Some((p, _)) = RESOLUTIONS.get(i) {
                d.fields.insert("resolution".into(), json!(p));
            } else if i > RESOLUTIONS.len() && preset.is_some() {
                // Other moves the value 1 ppi off the preset, so its own field shows.
                d.fields.insert("resolution".into(), json!(ppi.round() + 1.0));
            }
        }
        if !document
            && preset.is_none()
            && let Some(v) = widgets::plain_field(ui, "rasterize-ppi", ppi, " ppi", 0, 80.0)
        {
            d.fields.insert("resolution".into(), json!(v));
        }
    });
    ui.add_space(6.0);
    widgets::label_row(ui, tl!("Background:"), LABEL, |ui| {
        let cur = d.str("background");
        for (label, id) in own_choices(RASTERIZE, "background", false).unwrap_or_default() {
            if widgets::radio(ui, label, cur == id, true) {
                d.fields.insert("background".into(), json!(id));
            }
            ui.add_space(12.0);
        }
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Options"));
    widgets::label_row(ui, tl!("Anti-aliasing:"), LABEL, |ui| {
        use vectorcraft_render::AntiAlias;
        let aa = AntiAlias::from_id(&d.str("antiAlias")).unwrap_or(AntiAlias::Art);
        if let Some(i) = widgets::dropdown(ui, "rasterize-aa", aa.label(), &AntiAlias::ALL.map(AntiAlias::label), 170.0) {
            d.fields.insert("antiAlias".into(), json!(AntiAlias::ALL.get(i).copied().unwrap_or(AntiAlias::Art).id()));
        }
    });
    check(ui, d, "clippingMask", tl!("Create Clipping Mask"));
    let unit = app.session.general_unit();
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Add")).color(t.text));
        if let Some(v) = widgets::num_field(ui, "rasterize-add-around", Some(d.f64("addAround", 0.0)), unit, 80.0) {
            d.fields.insert("addAround".into(), json!(v));
        }
        ui.label(egui::RichText::new(tl!("Around Object")).color(t.text));
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn raster_effects_settings_draw_and_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        crate::menus::invoke(&mut app, "document.rasterEffectsSettings", json!({}));
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.kind.as_str(), d.fields["colorModel"].as_str()), (super::KIND, Some("rgb")));
        for (k, v) in [("resolution", json!(300)), ("background", json!("white")), ("addAround", json!(36)), ("colorModel", json!("grayscale"))] {
            d.fields.insert(k.into(), v);
        }
        frame(&mut app);
        // Other resolutions show their own field.
        app.ui.dialog.as_mut().unwrap().fields.insert("resolution".into(), json!(200));
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        let r = &st.doc.raster_effects;
        assert_eq!((st.doc.raster_effects_ppi, r.add_around, r.background), (200.0, 36.0, vectorcraft_doc::Background::White));
        assert_eq!(r.color_model, vectorcraft_doc::RasterColorModel::Grayscale);
        assert_eq!(st.history.undo.len(), 1);
        // A bad value keeps the dialog open.
        app.run("ui.rasterEffectsSettingsDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("addAround".into(), json!(-5));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
        crate::dialogs::cancel(&mut app);
        // Rasterize… starts from these settings.
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        crate::menus::invoke(&mut app, "object.rasterize", json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(
            (d.fields["ppi"].as_f64(), d.fields["background"].as_str(), d.fields["addAround"].as_f64()),
            (Some(200.0), Some("white"), Some(36.0))
        );
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        assert!(st.selection.objects.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Image(_)))));
    }

    #[test]
    fn rasterize_effect_dialog_previews_and_ok_applies_its_options_as_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let steps = app.session.active().unwrap().history.undo.len();
        crate::menus::invoke(&mut app, "effect.dialog", json!({"effect": "rasterize"}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.fields["useDocumentResolution"].as_bool()), ("effect", Some(true)));
        frame(&mut app);
        assert!(app.session.in_interaction(), "previews");
        let d = app.ui.dialog.as_mut().unwrap();
        for (k, v) in
            [("useDocumentResolution", json!(false)), ("resolution", json!(200)), ("background", json!("white")), ("antiAlias", json!("none"))]
        {
            d.fields.insert(k.into(), v);
        }
        // Other resolutions show their own field.
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        let (e, p) = (&n.appearance.effects[0], &n.appearance.effects[0].params);
        assert_eq!((e.id.as_str(), &p["resolution"], &p["background"], &p["antiAlias"]), ("rasterize", &json!(200), &json!("white"), &json!("none")));
        assert!(matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. }), "the rectangle stays a path");
        assert_eq!(st.history.undo.len(), steps + 1);
    }
}
