//! Pathfinder panel › Pathfinder Options…: Precision, Remove Redundant Points and Divide and Outline
//! Will Remove Unpainted Artwork. OK runs `object.pathfinder.options` (the options live in the
//! preferences).
//!
//! Fields: `precision` (pt, 0.001–10), `removeRedundantPoints` and `removeUnpainted`.

// PLAN: the framework import paths are assumed as in `path_ops`.
use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Map, Value, json};

use super::form;
use super::host::open_held;
use crate::VectorcraftApp;

const KEY: &str = "object.pathfinder.options";

/// The rows of Pathfinder Options, which the Pathfinder effect's dialog shows too.
pub(crate) fn params() -> Vec<Param> {
    let range = vectorcraft_engine::cmd::PATHFINDER_PRECISION_RANGE;
    vec![
        Param::number("precision", tl!("Precision:"), vectorcraft_engine::PathfinderOptions::default().precision, 0.001, 3)
            .bounds(Some(*range.start()), Some(*range.end()))
            .unit(UnitKind::Points),
        Param::new("removeRedundantPoints", tl!("Remove Redundant Points"), ParamKind::bool(false)),
        Param::new("removeUnpainted", tl!("Divide and Outline Will Remove Unpainted Artwork"), ParamKind::bool(false)),
    ]
}

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("There is no window to show the dialog in.");
        return;
    };
    let current = match json!(app.session.prefs.pathfinder) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let dialog = HostedParamDialog::new(&ctx, KEY, form::fields_of(&params(), &current), |f| {
        ParamDialog::new(KEY, tl!("Pathfinder Options"), params()).values(Bind::bound(f))
    });
    let ok = |app: &mut VectorcraftApp, result: &craft::ui::patterns::form_dialogs::ParamResult| {
        app.run(KEY, Value::Object(form::json_of(&params(), &result.fields))).map(drop)
    };
    if open_held(app, dialog, ok).is_none() {
        app.status("There is no window to show the dialog in.");
    }
}

/// The Pathfinder Options that `effect.apply` gives a new Pathfinder effect, as its params.
pub fn options_in_force(app: &VectorcraftApp) -> Value {
    vectorcraft_engine::cmd::pathfinder_effect_options(&app.session).to_params()
}

/// The params that an Effect › Pathfinder menu item with params `p` (`effect.apply`) runs with: its
/// `params` with the Pathfinder Options in force ([`options_in_force`]) added where they are
/// missing, as `effect.apply` journals them. Insert Menu Item stores the same params as the item's
/// step. Params of any other effect are returned unchanged.
pub fn menu_effect_params(app: &VectorcraftApp, mut p: Value) -> Value {
    if !p.get("effect").and_then(Value::as_str).is_some_and(vectorcraft_effects::takes_pathfinder_options) {
        return p;
    }
    let options = options_in_force(app);
    if let Some(o) = p.as_object_mut() {
        let params = o.entry("params").or_insert_with(|| json!({}));
        if let (Some(params), Some(options)) = (params.as_object_mut(), options.as_object()) {
            for (key, v) in options {
                params.entry(key.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    p
}
