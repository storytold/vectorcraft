//! Magic Wand panel: which attributes the Magic Wand tool compares, with a tolerance for each.
//! The settings live in the engine (`magicWand.set` / `magicWand.options`), so they survive tool
//! switches and agents can set them too.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::wand::color_tolerance_max;

use super::{part_hidden, set_part_hidden};
use crate::VectorcraftApp;
use crate::canvas::stroke_lengths;

const STROKE_KEY: &str = "wand-hide-stroke";
const TRANSPARENCY_KEY: &str = "wand-hide-transparency";

/// The panel menu's Hide / Show rows: (command id, hidden-part key, part name).
const PARTS: [(&str, &str, &str); 2] =
    [("ui.magicWand.strokeOptions", STROKE_KEY, "Stroke Options"), ("ui.magicWand.transparencyOptions", TRANSPARENCY_KEY, "Transparency Options")];

fn options(app: &mut VectorcraftApp) -> Value {
    app.session.execute("magicWand.options", &json!({})).unwrap_or_default()
}

/// The tolerance field of `key`, showing its value in `o`; the caller sets its range and unit.
fn tolerance(o: &Value, key: &str) -> NumberField<'static> {
    NumberField::new(tl!("Tolerance:"), Bind::given(Some(o[key].as_f64().unwrap_or(0.0)))).id_salt(key)
}

/// One row: checkbox + tolerance field (the key and what builds the field). The tolerance is editable while the
/// checkbox is on.
fn row(app: &mut VectorcraftApp, ui: &mut Ui, o: &Value, label: &'static str, flag: &str, tol: Option<(&str, &dyn Fn() -> NumberField<'static>)>) {
    let on = o[flag].as_bool().unwrap_or(false);
    Row::new().fill_width().align(RowAlign::SpaceBetween).wrap(true).show(ui, |ui, _| {
        if let Some(now) = Checkbox::new(Bind::given(on)).label(label).id_salt(flag).show(ui).value().copied() {
            app.run("magicWand.set", json!({ flag: now })).ok();
        }
        if let Some((key, field)) = tol {
            let caption = tl!("Tolerance:");
            Field::new(caption).id_salt(key).enabled(on).show(ui, |ui, _| {
                let name = format!("{caption} {label}");
                if let Some(n) = field().label(name).keeps_name(true).enabled(on).show(ui).value().copied().flatten() {
                    app.run("magicWand.set", json!({ key: n })).ok();
                }
            });
        }
    });
}

/// The largest color tolerance in the active document: 100 in a CMYK document, else 255.
fn color_max(app: &VectorcraftApp) -> f64 {
    color_tolerance_max(app.session.active().map(|d| d.doc.color_mode).unwrap_or_default())
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let o = options(app);
    let cmax = color_max(app);
    row(app, ui, &o, tl!("Fill Color"), "fillColor", Some(("fillTolerance", &|| tolerance(&o, "fillTolerance").range(0.0..=cmax))));
    if !part_hidden(app, STROKE_KEY) {
        Divider::horizontal().show(ui);
        row(app, ui, &o, tl!("Stroke Color"), "strokeColor", Some(("strokeTolerance", &|| tolerance(&o, "strokeTolerance").range(0.0..=cmax))));
        let lengths = stroke_lengths(app);
        let weight = || {
            NumberField::length(tl!("Tolerance:"), Bind::given(Some(o["weightTolerance"].as_f64().unwrap_or(0.0))), lengths)
                .id_salt("weightTolerance")
                .range(0.0..=1000.0)
        };
        row(app, ui, &o, tl!("Stroke Weight"), "strokeWeight", Some(("weightTolerance", &weight)));
    }
    if !part_hidden(app, TRANSPARENCY_KEY) {
        Divider::horizontal().show(ui);
        row(app, ui, &o, tl!("Opacity"), "opacity", Some(("opacityTolerance", &|| tolerance(&o, "opacityTolerance").range(0.0..=100.0).unit("%"))));
        row(app, ui, &o, tl!("Blending Mode"), "blendingMode", None);
    }
}

/// The panel menu: Hide / Show for each optional part, then Reset.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows: Vec<PanelRow> = PARTS.iter().map(|(id, ..)| PanelRow::Command(CommandRef::new(*id))).collect();
    rows.push(PanelRow::Separator);
    rows.push(PanelRow::Command(reset()));
    rows
}

/// The Reset row: `magicWand.set` with `reset`.
fn reset() -> CommandRef {
    CommandRef::with_params("magicWand.set", json!({"reset": true}))
}

/// Titles the Hide / Show rows by whether their part is hidden, and the Reset row.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for (id, key, name) in PARTS {
        let verb = if part_hidden(app, key) { tl!("Show") } else { tl!("Hide") };
        states.set(CommandRef::new(id), CommandState::new().title(format!("{verb} {}", tl!(name))));
    }
    states.set(reset(), CommandState::new().title(tl!("Reset")));
}

/// Runs a menu row of this panel; `None` for an id that is not one of its own (Reset runs the engine command).
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let (_, key, _) = PARTS.iter().find(|(part, ..)| *part == id)?;
    let hidden = part_hidden(app, key);
    set_part_hidden(app, key, !hidden);
    Some(Ok(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tolerance_field_has_its_own_accessible_name() {
        use craft::ui::contract::Role;
        use craft::ui::testing::Fixture;

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let mut fixture = Fixture::new(move |ui| show(&mut app, ui));
        fixture.settle();
        let names: Vec<_> = fixture.by_role(Role::SPIN_BUTTON).into_iter().filter_map(|field| fixture.tree().name_of(field.node)).collect();
        assert_eq!(names, ["Tolerance: Fill Color", "Tolerance: Stroke Color", "Tolerance: Stroke Weight", "Tolerance: Opacity"]);
        assert_eq!(fixture.texts().iter().filter(|run| run.text == "Tolerance:").count(), 4);
    }

    #[test]
    fn color_tolerance_fields_go_to_100_in_a_cmyk_document() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(color_max(&app), 255.0);
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        assert_eq!(color_max(&app), 255.0);
        app.session.execute("magicWand.set", &json!({"fillTolerance": 200})).unwrap();
        app.session.execute("file.new", &json!({"width": 100, "height": 100, "colorMode": "cmyk"})).unwrap();
        assert_eq!(color_max(&app), 100.0);
        assert_eq!(options(&mut app)["fillTolerance"], 100.0);
    }
}
