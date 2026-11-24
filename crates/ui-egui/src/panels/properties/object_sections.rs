//! The Properties panel's sections of the selected image, Image Trace object and Objects on Path groups.

use craft::behavior::{Anchor9, ColumnWidth, Mixed};
use craft::host::PickKind;
use craft::keymap::CommandRef;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Sizing};
use craft::ui::display::{Elide, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Grid;
use craft::ui::pickers::{ReferencePoint, ReferencePointLook};
use craft::ui::runtime::env::UnitKind;
use craft::ui::sections::Section;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, NumberPick};
use egui::Ui;
use serde_json::json;

use crate::VectorcraftApp;
use crate::panels::{links, transform};

/// Two equal columns: a pair of buttons side by side.
pub(super) const HALVES: [ColumnWidth; 2] = [ColumnWidth::fill(1.0); 2];

/// The one selected image: its file (or Embedded), colour mode and resolution, and the Links
/// panel's actions for it.
pub(super) fn image_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(i) = crate::place::selected_image_info(app) else { return };
    let (name, details) = crate::place::image_summary(i);
    let (Some(id), linked) = (i["id"].as_u64(), i["linked"] == true) else { return };
    Section::fixed(if linked { tl!("Linked File") } else { tl!("Embedded Image") }).show(ui, |ui, _| {
        Text::new(name.clone()).elide(Elide::Middle).role(TypeRole::BodyStrong).show(ui);
        Text::new(details.clone()).shade(Shade::Muted).show(ui);
        Grid::new(&HALVES).show(ui, |ui, _| {
            // PLAN: assumed `crate::place::LinkAction::{shown(linked) -> &'static [LinkAction], label(self), run(self, app, id, &name)}` (files-w14, fl.A18)
            for action in crate::place::LinkAction::shown(linked) {
                if Button::new(action.label()).sizing(Sizing::FillWidth).show(ui).clicked() {
                    action.run(app, id, &name);
                }
            }
            if crate::file_asks::can(app, PickKind::Open) && Button::new(tl!("Replace…")).sizing(Sizing::FillWidth).show(ui).clicked() {
                links::replace(app, vec![id]);
            }
            Button::command(CommandRef::with_params("window.panel.activate", json!({ "panel": links::ID })))
                .label(tl!("Links"))
                .sizing(Sizing::FillWidth)
                .show(ui);
        });
    });
}

/// The one selected Image Trace object: its preset (choosing another traces it again with that
/// one), view, a button to reveal the tracing controls, Expand and Release.
pub(super) fn trace_section(app: &mut VectorcraftApp, ui: &mut Ui, preset: &str, view: vectorcraft_doc::TraceView) {
    Section::fixed(tl!("Image Trace")).show(ui, |ui, _| {
        Field::new(tl!("Preset")).show(ui, |ui, _| crate::panels::image_trace::preset_dropdown(app, ui, preset));
        crate::panels::image_trace::view_row(app, ui, view);
        crate::panels::image_trace::split_row(app, ui);
        if Button::new(tl!("Image Trace")).sizing(Sizing::FillWidth).show(ui).clicked() {
            crate::panels::image_trace::open_settings(app);
        }
        Grid::new(&HALVES).show(ui, |ui, _| {
            for (label, id) in [(tl!("Expand"), "imageTrace.expand"), (tl!("Release"), "imageTrace.release")] {
                Button::command(CommandRef::new(id)).label(label).sizing(Sizing::FillWidth).show(ui);
            }
        });
    });
}

/// The Objects on Path groups selected with nothing else: their ids, the Pivot and Rotate they
/// share (`None` where they differ), and whether Detach can take an object off the path: one group,
/// neither hidden nor locked, with an object chosen with its Select/Move widget that is neither
/// hidden nor locked.
pub(super) struct ObjectsOnPathSelection {
    ids: Vec<u64>,
    pivot: Option<u8>,
    rotate: Option<f64>,
    detachable: bool,
}

/// The selection as Objects on Path groups ([`ObjectsOnPathSelection`]), when every selected
/// object is one.
pub(super) fn selected_objects_on_path(app: &VectorcraftApp) -> Option<ObjectsOnPathSelection> {
    let st = app.session.active()?;
    let mut out: Option<ObjectsOnPathSelection> = None;
    for id in &st.selection.objects {
        let rec = st.doc.node(*id)?.extras().objects_on_path.as_deref()?;
        match &mut out {
            None => {
                let chosen = st.selection.on_path_member.filter(|m| st.selection.objects.len() == 1 && rec.members.iter().any(|x| x.id == *m));
                let detachable = chosen.is_some_and(|m| st.doc.is_editable(*id) && st.doc.is_editable(m));
                out = Some(ObjectsOnPathSelection { ids: vec![id.0], pivot: Some(rec.pivot), rotate: Some(rec.rotate), detachable });
            }
            Some(s) => {
                s.ids.push(id.0);
                if s.pivot != Some(rec.pivot) {
                    s.pivot = None;
                }
                if s.rotate != Some(rec.rotate) {
                    s.rotate = None;
                }
            }
        }
    }
    out
}

/// The Objects on Path Options of the selected Objects on Path groups: Pivot, the point of each
/// object that attaches to the path, and Rotate, the angle each object tilts by from upright on the
/// path. A change goes to every selected group in one undo step (`object.objectsOnPath.options`).
/// Where the groups differ, no Pivot square is chosen and Rotate is blank. Detach takes the object
/// chosen with its Select/Move widget off the path (`object.objectsOnPath.detach`), and is dimmed
/// until one is chosen.
pub(super) fn objects_on_path_section(app: &mut VectorcraftApp, ui: &mut Ui, oop: &ObjectsOnPathSelection) {
    let set = |app: &mut VectorcraftApp, mut p: serde_json::Value| {
        p["ids"] = json!(oop.ids);
        if let Err(e) = app.run("object.objectsOnPath.options", p) {
            app.status(e);
        }
    };
    Section::fixed(tl!("Objects on Path Options")).show(ui, |ui, _| {
        Field::new(tl!("Pivot")).show(ui, |ui, _| {
            // Mixed while the groups' pivots differ: no square is chosen.
            let shown = oop.pivot.map_or(Mixed::Mixed, |p| Mixed::Same(Anchor9::ALL.get(usize::from(p)).copied()));
            let out = ReferencePoint::new(tl!("Pivot"), Bind::given(shown)).look(ReferencePointLook::Squares).show(ui);
            if let Some(i) = out.value().copied().flatten().and_then(|p| Anchor9::ALL.iter().position(|a| *a == p))
                && oop.pivot.is_none_or(|p| usize::from(p) != i)
            {
                set(app, json!({ "pivot": i }));
            }
        });
        Field::new(tl!("Rotate")).show(ui, |ui, _| {
            let presets: Vec<Entry<NumberPick>> =
                transform::ANGLE_PRESETS.iter().map(|v| Entry::new(NumberPick::Number(*v), format!("{v}°"))).collect();
            let out = NumberField::new(tl!("Rotate"), Bind::given(oop.rotate))
                .unit_kind(UnitKind::Degrees)
                .presets(&presets)
                .range(-360.0..=f64::INFINITY)
                .stepper(crate::fields::stops_stepper(15.0, &transform::ANGLE_PRESETS, |v| format!("{v:.2}")))
                .show(ui);
            crate::edits::apply_edits(app, &out.edit, crate::edits::EditPath::Scope, |app, v: &Option<f64>, _| {
                if let Some(a) = *v
                    && oop.rotate != Some(a)
                {
                    set(app, json!({ "rotate": a }));
                }
            });
        });
        let detach = Button::new(tl!("Detach")).enabled(oop.detachable).sizing(Sizing::FillWidth).show(ui);
        if detach.clicked()
            && let Err(e) = app.run("object.objectsOnPath.detach", json!({}))
        {
            app.status(e);
        }
    });
}
