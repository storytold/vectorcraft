//! Eyedropper Options (double-click the Eyedropper tool): the raster sample size and the Picks Up /
//! Applies trees of attributes. OK runs `eyedropper.setOptions` (the options live in the
//! preferences).
//!
//! Fields: `sampleSize` (1, 3 or 5), `pickUp` and `apply` (`{appearance: {transparency, fill:
//! {color, transparency, overprint}, stroke: {color, transparency, overprint, weight, cap, join,
//! miter, dash}}, character, paragraph}`, each leaf a bool).

use craft::behavior::Mixed;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Outcome as _, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use super::{form, host};
use crate::VectorcraftApp;

/// The dialog kind of Eyedropper Options.
pub const KIND: &str = "eyedropperOptions";

/// The raster sample sizes: (pixels square, label).
const SAMPLE_SIZES: [(u64, &str); 3] = [(1, "Point Sample"), (3, "3 x 3 Average"), (5, "5 x 5 Average")];

/// The rows of a Picks Up / Applies tree: (JSON pointer into the tree, label, depth).
const TREE: &[(&str, &str, u8)] = &[
    ("/appearance", "Appearance", 0),
    ("/appearance/transparency", "Transparency", 1),
    ("/appearance/fill", "Focal Fill", 1),
    ("/appearance/fill/color", "Color", 2),
    ("/appearance/fill/transparency", "Transparency", 2),
    ("/appearance/fill/overprint", "Overprint", 2),
    ("/appearance/stroke", "Focal Stroke", 1),
    ("/appearance/stroke/color", "Color", 2),
    ("/appearance/stroke/transparency", "Transparency", 2),
    ("/appearance/stroke/overprint", "Overprint", 2),
    ("/appearance/stroke/weight", "Weight", 2),
    ("/appearance/stroke/cap", "Cap", 2),
    ("/appearance/stroke/join", "Join", 2),
    ("/appearance/stroke/miter", "Miter Limit", 2),
    ("/appearance/stroke/dash", "Dash Pattern", 2),
    ("/character", "Character Style", 0),
    ("/paragraph", "Paragraph Style", 0),
];

/// The rows: the sample size (its option ids are the sizes as text) and the two trees.
fn params() -> Vec<Param> {
    let sizes = SAMPLE_SIZES.iter().map(|(n, label)| (n.to_string(), Label::from(tl!(label)))).collect();
    vec![
        Param::new("sampleSize", tl!("Raster Sample Size:"), ParamKind::choice(sizes, Some("1".into()))),
        Param::custom("trees", "", trees).companions(["pickUp", "apply"]),
    ]
}

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    let params = params();
    let mut options = json!(app.session.prefs.eyedropper);
    if let Some(size) = options.get("sampleSize").and_then(Value::as_u64) {
        options["sampleSize"] = json!(size.to_string());
    }
    let fields = options.as_object().map(|m| form::fields_of(&params, m)).unwrap_or_default();
    let shown = params.clone();
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| {
        let mut values = form::json_of(&params, &result.fields);
        // The command takes the size as a number.
        if let Some(size) = values.get("sampleSize").and_then(Value::as_str).and_then(|s| s.parse::<u64>().ok()) {
            values.insert("sampleSize".into(), json!(size));
        }
        app.run("eyedropper.setOptions", Value::Object(values)).map(|_| ())
    };
    let opened = app.ui_ctx.clone().and_then(|ctx| {
        let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |f| {
            ParamDialog::new(KIND, tl!("Eyedropper Options"), shown.clone()).values(Bind::bound(f))
        });
        host::open_held(app, dialog, ok)
    });
    if opened.is_none() {
        app.status(tl!("No window to show the dialog in.").to_string());
    }
}

/// A flag or a branch of flags: `Some(on)` when every flag under it is `on`, `None` when mixed.
fn state(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Object(o) => {
            let mut s = o.values().map(state);
            let first = s.next()??;
            s.all(|x| x == Some(first)).then_some(first)
        }
        _ => Some(false),
    }
}

/// Set every flag under `v` to `on`.
fn set_all(v: &mut Value, on: bool) {
    match v {
        Value::Object(o) => o.values_mut().for_each(|c| set_all(c, on)),
        _ => *v = Value::Bool(on),
    }
}

/// The two trees side by side.
fn trees(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut CustomContext) -> egui::Response {
    Row::new()
        .show(ui, |ui, _| {
            for (key, title) in [("pickUp", tl!("Eyedropper Picks Up:")), ("apply", tl!("Eyedropper Applies:"))] {
                Stack::new().show(ui, |ui, _| {
                    Text::new(title).show(ui);
                    tree(ui, fields, key);
                });
            }
        })
        .output
}

/// One tree (`key`: `pickUp` or `apply`) of checkboxes; a branch's box sets everything under it.
fn tree(ui: &mut egui::Ui, fields: &mut DialogFields, key: &str) {
    let Some(FieldValue::Json(mut t)) = fields.get(key).cloned() else { return };
    let mut changed = false;
    for (path, label, depth) in TREE {
        let Some(node) = t.pointer_mut(path) else { continue };
        let s = state(node);
        Row::new().indent(*depth).show(ui, |ui, _| {
            let shown = s.map_or(Mixed::Mixed, Mixed::Same);
            let out = Checkbox::new(Bind::given(shown)).label(tl!(label)).show(ui);
            if clicked(out.response()) {
                set_all(node, s != Some(true));
                changed = true;
            }
        });
    }
    if changed {
        fields.set(key, FieldValue::Json(t));
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_rectangular_grid::{field_of, set_of, try_confirm};
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn the_trees_set_the_options() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        open(&mut app);
        // A branch's box sets the flags under it; a mixed branch shows a dash.
        let mut apply = field_of(&app, KIND, "apply");
        set_all(apply.pointer_mut("/appearance/stroke").unwrap(), false);
        assert_eq!(state(&apply["appearance"]["stroke"]), Some(false));
        assert_eq!(state(&apply["appearance"]), None);
        set_of(&mut app, KIND, "apply", apply).unwrap();
        set_of(&mut app, KIND, "sampleSize", json!("5")).unwrap();
        try_confirm(&mut app, KIND).unwrap();
        let o = app.session.prefs.eyedropper;
        assert_eq!(
            (o.sample_size, o.apply.appearance.stroke.dash, o.apply.appearance.fill.color, o.pick_up.appearance.stroke.dash),
            (5, false, true, true)
        );
    }
}
