//! EPS Options (Export As → EPS): the preview (none, TIFF black and white or colour, transparent
//! or opaque), overprints and the flattener preset transparency is flattened with, fonts, the
//! options (linked files, thumbnails, CMYK PostScript, compatible gradients) and the PostScript
//! language level.
//!
//! Fields: `document.export` params (`format: eps`, `path?`, the artboard choice) and the EPS
//! options of `document.exportEps`. OK remembers the options for next time and writes the file
//! (asking where when there is no path); bad options keep the dialog open.
//!
//! Custom… beside the flattener preset opens Custom Transparency Flattener Options over the
//! dialog: its OK sets every flattener option as `flattener` (the preset then shows [Custom]) and
//! goes back to EPS Options; Cancel goes back with the options as they were. Choosing a preset
//! drops the custom options.

use std::sync::Arc;

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Show as _, TooltipSpec};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::{FlattenOptions, FlattenerPreset, fileio};
use vectorcraft_eps::{Level, Overprint, Preview};

use super::dxf_options::{check_param, choice_param};
use super::export_host::{self, After, Export, Open};
use super::{flatten, form};
use crate::{VectorcraftApp, io};

pub(super) const KIND: &str = "epsOptions";

/// Custom Transparency Flattener Options, opened over EPS Options by Custom….
pub(super) const CUSTOM_KIND: &str = "epsFlattenerOptions";

/// The flattener rows the format fixes, and why.
const FIXED: [&str; 2] = ["preserveAlpha", "preserveOverprints"];
const FIXED_REASON: &str = "EPS has no alpha transparency, and Overprints sets whether overprints are kept.";

/// Fields that are not remembered between exports (they belong to one export).
const PER_EXPORT: [&str; 7] = ["format", "path", "useArtboards", "range", "artboard", "artboards", "selectedOnly"];

/// The options a first EPS Options dialog starts from (the export's defaults).
fn defaults() -> Map<String, Value> {
    let medium = FlattenOptions::preset_label(FlattenOptions::PRESETS[1]).unwrap_or_default();
    let mut m = Map::new();
    for (k, v) in [
        ("previewFormat", json!(Preview::default().id())),
        ("transparentPreview", json!(true)),
        ("overprints", json!(Overprint::default().id())),
        ("flattenerPreset", json!(medium)),
        ("includeLinkedFiles", json!(false)),
        ("thumbnails", json!(true)),
        ("cmykPostScript", json!(true)),
        ("compatibleGradients", json!(false)),
        ("level", json!(Level::default().id())),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// The rows, with `presets` ([`flatten::preset_names`]) in the flattener picker.
fn rows(presets: &Arc<[(String, String)]>) -> Vec<Param> {
    let presets = Arc::clone(presets);
    // PLAN: assumed ids "true" and "false" for the two-id radio that holds a JSON bool (wg.S12).
    let transparency = Param::new(
        "transparentPreview",
        "",
        ParamKind::radio(vec![("true".into(), tl!("Transparent").into()), ("false".into(), tl!("Opaque").into())], None),
    );
    vec![
        Param::section(tl!("Preview")),
        choice_param("previewFormat", tl!("Format:"), Preview::ALL.iter().map(|p| (p.id(), p.label()))),
        // Only the colour preview can keep transparency.
        transparency.enabled_when("previewFormat", FieldValue::Text(Preview::TiffColor.id().into())),
        Param::section(tl!("Transparency")),
        choice_param("overprints", tl!("Overprints:"), Overprint::ALL.iter().map(|o| (o.id(), o.label()))),
        Param::custom("flattenerPreset", tl!("Preset:"), move |ui, fields, cx| {
            ui.horizontal(|ui| {
                let custom = fields.contains("flattener");
                flatten::preset_picker(ui, fields, "flattenerPreset", &presets, custom);
                if Button::new(tl!("Custom…")).show(ui).clicks > 0 {
                    cx.request_action("custom");
                }
            })
            .response
        })
        .resets(["flattener"]),
        Param::section(tl!("Fonts")),
        // Type is written as glyph outlines, which need no fonts.
        check_param("__embedFonts", tl!("Embed Fonts (for other applications)"))
            .opens_with(true)
            .enabled_when_in("__embedFonts", std::iter::empty::<FieldValue>())
            .tooltip(
                TooltipSpec::new(tl!("Type is written as glyph outlines, which need no fonts."))
                    .disabled_text(tl!("Type is written as glyph outlines, which need no fonts.")),
            ),
        Param::section(tl!("Options")),
        check_param("includeLinkedFiles", tl!("Include Linked Files")),
        check_param("thumbnails", tl!("Include Document Thumbnails")),
        check_param("cmykPostScript", tl!("Include CMYK PostScript in RGB Files")),
        check_param("compatibleGradients", tl!("Compatible Gradient and Gradient Mesh Printing")),
        choice_param("level", tl!("PostScript:"), Level::ALL.iter().map(|l| (l.id(), l.label()))),
    ]
}

/// The rows as far as their kinds go, for reading values to and from JSON.
fn kinds() -> Vec<Param> {
    rows(&Arc::from(Vec::new()))
}

/// Open EPS Options for an export whose params (path, artboard choice, any EPS option) are
/// `params`, over the options used last.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    open_with(app, params, false);
}

/// [`open`] for Export Selection when `selection` is set: the options then write the selected art alone.
pub fn open_with(app: &mut VectorcraftApp, params: &Value, selection: bool) {
    let mut fields = defaults();
    for given in [&app.ui.eps_options, params] {
        if let Some(o) = given.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    // A preset asked for without options of its own replaces the custom options used last.
    if params.get("flattenerPreset").is_some() && params.get("flattener").is_none() {
        fields.remove("flattener");
    }
    fields.insert("format".into(), json!("eps"));
    let mut values = form::fields_of(&kinds(), &fields);
    builtin_label(&mut values);
    let presets: Arc<[(String, String)]> = flatten::preset_names(app).into();
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let dialog = HostedParamDialog::new(&ctx, KIND, values, move |fields| {
        builtin_label(fields);
        normalise_level(fields);
        ParamDialog::new(KIND, tl!("EPS Options"), rows(&presets)).values(Bind::bound(fields)).ok_label(tl!("Export").into()).below(|ui, fields| {
            ui.weak(if fields.text("level").and_then(Level::from_id) == Some(Level::Two) || fields.bool("compatibleGradients") == Some(true) {
                tl!("Transparency is flattened with the preset. Type is written as outlines; gradients as bands of colour.")
            } else {
                tl!("Transparency is flattened with the preset. Type is written as outlines; gradients stay smooth.")
            });
        })
    });
    export_host::open(app, Export::Eps, selection, dialog);
}

/// A level set as a number (`level: 2`) reads as its id.
fn normalise_level(fields: &mut DialogFields) {
    let id = match fields.get("level") {
        Some(FieldValue::Int(n)) => n.to_string(),
        Some(FieldValue::Number(n)) if n.is_finite() => format!("{n}"),
        _ => return,
    };
    fields.set("level", id);
}

/// A built-in preset named by id (`medium`) in `fields` shows as its name. It runs where the values are built and
/// where the build runs each pass, so a preset set from outside shows as its name too.
fn builtin_label(fields: &mut DialogFields) {
    let label = fields.text("flattenerPreset").and_then(|p| FlattenOptions::preset_label(&p.to_ascii_lowercase()).map(|l| l.to_string()));
    if let Some(label) = label {
        fields.set("flattenerPreset", label);
    }
}

/// The custom options `fields` hold, as JSON.
fn custom_value(fields: &DialogFields) -> Option<Value> {
    match fields.get("flattener")? {
        FieldValue::Json(v) => Some(v.clone()),
        other => serde_json::to_value(other).ok(),
    }
}

/// The flattener options EPS Options `fields` export with: its preset, with the Custom… options over it.
fn flattener(fields: &DialogFields, saved: &[FlattenerPreset]) -> Result<FlattenOptions, String> {
    FlattenOptions::for_export(fields.text("flattenerPreset"), custom_value(fields).as_ref(), saved)
}

/// The options Custom Transparency Flattener Options `fields` hold.
fn custom_options(fields: &DialogFields) -> Result<FlattenOptions, String> {
    let rows = flatten::option_params(&FIXED, FIXED_REASON);
    FlattenOptions::from_params_with(&Value::Object(form::json_of(&rows, fields)), &[])
}

/// Custom Transparency Flattener Options over EPS Options `parent` (whose values are `fields`), on the options it
/// exports with (the default preset's while those are not valid), without alpha: EPS has none.
pub(super) fn custom_dialog(app: &VectorcraftApp, parent: DialogId, fields: &DialogFields) -> HostedParamDialog {
    let _ = parent; // the host keeps the parent; `Export::EpsCustom` names it
    let mut o = flattener(fields, &app.session.prefs.flattener_presets).unwrap_or_default();
    o.preserve_alpha = false;
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    HostedParamDialog::new(&ctx, CUSTOM_KIND, flatten::option_fields(&o), |fields| {
        ParamDialog::new(CUSTOM_KIND, tl!("Custom Transparency Flattener Options"), flatten::option_params(&FIXED, FIXED_REASON))
            .values(Bind::bound(fields))
            .below(|ui, _| {
                ui.weak(tl!("EPS has no alpha transparency, and Overprints sets whether overprints are kept."));
            })
    })
}

/// OK of the custom options: EPS Options `parent` again, exporting with these options (a built-in preset under them,
/// so that deleting a saved preset leaves them whole); bad options keep the dialog open.
pub(super) fn custom_ok(_app: &VectorcraftApp, parent: &mut DialogFields, fields: &DialogFields) -> Result<(), String> {
    let mut o = custom_options(fields)?;
    o.preserve_alpha = false;
    if FlattenOptions::preset(parent.text("flattenerPreset").unwrap_or_default()).is_none() {
        let medium = FlattenOptions::preset_label(FlattenOptions::PRESETS[1]).unwrap_or_default();
        parent.set("flattenerPreset", medium.to_string());
    }
    parent.set("flattener", FieldValue::Json(serde_json::to_value(&o).map_err(|e| e.to_string())?));
    Ok(())
}

/// Check the options, remember them, and write the file(s).
pub(super) fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    let map = form::json_of(&kinds(), fields);
    let params = fileio::eps::resolve(&Value::Object(map.clone()), &app.session.prefs.flattener_presets)?;
    // Nothing selected keeps the dialog open too.
    if let Some(st) = app.session.active() {
        fileio::export_source(st, &params).map_err(|e| e.to_string())?;
    }
    app.ui.eps_options = Value::Object(map.into_iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).collect());
    let path = fields.text("path").map(str::to_string);
    io::export(app, Some("eps"), path, &super::export_as::exported(open.selection, params)).map(|_| None)
}
