//! DXF Import Options: opening or placing a DXF drawing asks which layout to import (model space
//! or a paper layout, previewed), its scale (fitted to the artboard, or 1 unit of the art = N
//! drawing units: by default the drawing at 1:1 in its own unit), whether lineweights scale with
//! it, whether it is centred, and whether its layers merge into one.
//!
//! Fields: `mode` (`open` | `place`), `layouts` (model space first), `units` and `version` (what
//! the drawing declares), and the `dxf` options of `document.open`: `layout`, `fit`, `unit` (a
//! unit name), `scale`, `scaleLineweights`, `center`, `mergeLayers`. The file, its name and path
//! and the other `file.place` params stay with the dialog's OK, which opens or places the drawing
//! and remembers the fit, lineweight, centring and layer choices for the next one; errors keep
//! the dialog open.

use std::sync::Arc;

use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbSize;
use craft::ui::pickers::Select;
use craft::ui::primitives::textures::Revision;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::text::NumberField;
use egui::{Color32, ColorImage, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::fileio;

use super::host::open_held;
use super::import_pdf::{DialogFile, other_place_params, place_source, render_preview, with_file};
use crate::{VectorcraftApp, io};

pub const KIND: &str = "dxfImport";

/// The choices the next drawing starts from (the scale belongs to each drawing's units).
const REMEMBERED: [&str; 4] = ["fit", "scaleLineweights", "center", "mergeLayers"];

/// What DXF Import Options answers with.
#[derive(Clone, PartialEq)]
pub struct DxfChoices {
    /// The layout to import: `Model` or a paper layout's name.
    pub layout: String,
    /// Scale to Fit Artboard.
    pub fit: bool,
    /// The unit of the art in `1 unit = scale drawing units`.
    pub unit: Unit,
    /// The drawing units one unit of the art stands for.
    pub scale: f64,
    /// Whether lineweights scale with the drawing.
    pub scale_lineweights: bool,
    /// Whether the art is centred.
    pub center: bool,
    /// Whether the layers merge into one.
    pub merge_layers: bool,
}

impl Default for DxfChoices {
    fn default() -> Self {
        DxfChoices {
            layout: "Model".into(),
            fit: false,
            unit: Unit::Millimeters,
            scale: 1.0,
            scale_lineweights: false,
            center: true,
            merge_layers: false,
        }
    }
}

/// What the dialog's OK needs besides its choices.
struct Source {
    /// The `file.place` params the drawing is placed with; `None` opens it.
    place: Option<Value>,
    name: String,
    path: Option<String>,
    file: Arc<DialogFile>,
}

/// The DXF Import Options dialog: the layout preview and what the drawing declares on the left, the
/// choices on the right. OK files the choices for the app, which imports and answers; an error stays
/// under the choices until one changes.
pub struct DxfImportDialog {
    id: DialogId,
    placing: bool,
    file: Arc<DialogFile>,
    layouts: Vec<String>,
    units: String,
    version: String,
    choices: DxfChoices,
    opening: DxfChoices,
    /// The picture of the layout the pump rendered last, with the layout's revision.
    preview: Option<(Revision, Arc<ColorImage>)>,
    held: HeldAnswer<DxfChoices>,
}

impl DxfImportDialog {
    /// The dialog for `file`'s `info`, starting from `choices`.
    fn new(placing: bool, file: Arc<DialogFile>, info: &vectorcraft_cad::DxfInfo, choices: DxfChoices) -> Self {
        let text = |v: Value| match v {
            Value::String(s) => s,
            other => other.to_string(),
        };
        DxfImportDialog {
            id: dialog_id(KIND),
            placing,
            file,
            layouts: info.layouts.clone(),
            units: text(json!(info.units)),
            version: text(json!(info.version)),
            opening: choices.clone(),
            choices,
            preview: None,
            held: HeldAnswer::new(),
        }
    }

    /// The revision of the shown layout while its picture has not been handed over ([`set_preview`](Self::set_preview)).
    pub fn wanted_preview(&self) -> Option<Revision> {
        let wanted = Revision::of(&self.choices.layout);
        self.preview.as_ref().is_none_or(|(shown, _)| *shown != wanted).then_some(wanted)
    }

    /// Hands over the picture of the layout whose revision is `key`.
    pub fn set_preview(&mut self, key: Revision, image: Arc<ColorImage>) {
        self.preview = Some((key, image));
    }
}

impl StackDialog for DxfImportDialog {
    type Result = DxfChoices;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<DxfChoices>> {
        // The app accepted the held OK: this pass closes the dialog with it. A measuring run closes nothing.
        let finish = self.held.is_accepted() && !craft::ui::runtime::measure::active(ui.ctx());
        let refusal = self.held.refusal().map(|(reason, refused)| (reason.clone(), refused.clone()));
        let title = if self.placing { tl!("Place DXF") } else { tl!("DXF Import Options") };
        let layouts: Vec<Entry<String>> = self.layouts.iter().map(|l| Entry::new(l.clone(), l.clone())).collect();
        let units: Vec<Entry<Unit>> = Unit::ALL.iter().filter(|u| **u != Unit::FeetInches).map(|u| Entry::new(*u, u.label())).collect();
        let (revision, image) = match &self.preview {
            Some((revision, image)) => (*revision, Some(image.clone())),
            None => (Revision::default(), None),
        };
        let picture = || image.as_ref().map_or_else(|| ColorImage::filled([1, 1], Color32::TRANSPARENT), |i| ColorImage::clone(i));
        let declared = format!("DXF {} · {}", self.version, self.units);
        let (token, scale_range) = (self.file.token, *vectorcraft_cad::SCALE_RANGE.start()..=*vectorcraft_cad::SCALE_RANGE.end());
        let shown = craft::ui::patterns::form_dialogs::FormDialog::new(title, Bind::bound(&mut self.choices))
            .opening(self.opening.clone())
            .accept_stays_open(true)
            .ok_enabled(!self.held.is_waiting())
            .show(ui, |ui, form, c| {
                Row::new().show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        Picture::new(tl!("Preview"), token, &picture).revision(revision).extent(PictureSize::Thumb(ThumbSize::Large)).show(ui);
                        Text::new(declared.clone()).show(ui);
                    });
                    Stack::new().show(ui, |ui, _| {
                        Field::new(tl!("Layout")).show(ui, |ui, _| {
                            if let Some(layout) =
                                Select::new(tl!("Layout"), Bind::given(c.layout.clone()), &layouts).show(ui).events.landed().cloned()
                            {
                                c.layout = layout;
                            }
                        });
                        FieldGroup::new(tl!("Scaling")).show(ui, |ui, _| {
                            if let Some(fit) =
                                craft::ui::choice::Checkbox::new(Bind::given(c.fit)).label(tl!("Scale to Fit Artboard")).show(ui).value().copied()
                            {
                                c.fit = fit;
                            }
                            Field::new(tl!("Scale")).show(ui, |ui, _| {
                                Row::new().show(ui, |ui, _| {
                                    Text::new("1").show(ui);
                                    if let Some(unit) =
                                        Select::new(tl!("Unit"), Bind::given(c.unit), &units).enabled(!c.fit).show(ui).events.landed().copied()
                                    {
                                        c.unit = unit;
                                    }
                                    Text::new("=").show(ui);
                                    let scale = NumberField::new(tl!("Scale"), Bind::given(Some(c.scale)))
                                        .range(scale_range.clone())
                                        .enabled(!c.fit)
                                        .show(ui)
                                        .edit
                                        .events
                                        .landed()
                                        .copied()
                                        .flatten();
                                    if let Some(scale) = scale {
                                        c.scale = scale;
                                    }
                                    Text::new(tl!("Units")).show(ui);
                                });
                            });
                            if let Some(on) = craft::ui::choice::Checkbox::new(Bind::given(c.scale_lineweights))
                                .label(tl!("Scale Lineweights"))
                                .show(ui)
                                .value()
                                .copied()
                            {
                                c.scale_lineweights = on;
                            }
                        });
                        FieldGroup::new(tl!("Options")).show(ui, |ui, _| {
                            if let Some(on) =
                                craft::ui::choice::Checkbox::new(Bind::given(c.center)).label(tl!("Center Artwork")).show(ui).value().copied()
                            {
                                c.center = on;
                            }
                            if let Some(on) =
                                craft::ui::choice::Checkbox::new(Bind::given(c.merge_layers)).label(tl!("Merge Layers")).show(ui).value().copied()
                            {
                                c.merge_layers = on;
                            }
                        });
                    });
                });
                if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| refused == c) {
                    Text::new(reason.clone()).show(ui);
                }
                if finish {
                    form.request_close_as(CloseReason::Activated);
                }
                true
            });
        if self.held.refusal().is_some_and(|(_, refused)| *refused != self.choices) {
            self.held.clear_refusal();
        }
        // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if shown.picked == Some(0) && !self.held.is_waiting() {
            self.held.file(self.choices.clone());
        }
        if finish && shown.closed == Some(CloseReason::Activated) {
            return Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    /// OK as the button: files the choices and answers none, and answers the held choices once the app
    /// accepted them.
    fn confirm(&mut self, _: &egui::Context) -> Option<DxfChoices> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() {
            return None;
        }
        self.held.file(self.choices.clone());
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let c = &self.choices;
        Some(json!({
            "mode": if self.placing { "place" } else { "open" },
            "layouts": self.layouts,
            "layout": c.layout,
            "units": self.units,
            "version": self.version,
            "fit": c.fit,
            "unit": c.unit.label(),
            "scale": c.scale,
            "scaleLineweights": c.scale_lineweights,
            "center": c.center,
            "mergeLayers": c.merge_layers,
        }))
    }

    /// The layout is kept as typed: a layout the file lacks is refused when OK imports, with the
    /// dialog still open.
    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejected = || FieldRefusal::Rejects(&value).message(field);
        let flag = || value.as_bool().ok_or_else(rejected);
        let c = &mut self.choices;
        match field {
            "layout" => c.layout = value.as_str().ok_or_else(rejected)?.to_owned(),
            "unit" => c.unit = value.as_str().and_then(Unit::named).ok_or_else(rejected)?,
            "scale" => {
                c.scale = value.as_f64().ok_or_else(rejected)?.clamp(*vectorcraft_cad::SCALE_RANGE.start(), *vectorcraft_cad::SCALE_RANGE.end())
            }
            "fit" => c.fit = flag()?,
            "scaleLineweights" => c.scale_lineweights = flag()?,
            "center" => c.center = flag()?,
            "mergeLayers" => c.merge_layers = flag()?,
            _ => return Err(format!("unknown field `{field}`")),
        }
        ctx.request_repaint();
        Ok(())
    }
}

impl HoldsOk for DxfImportDialog {
    type Answer = DxfChoices;

    /// OK always files its choices for the app to judge.
    fn held_ok(self) -> Self {
        self
    }

    fn take_ok(&mut self) -> Option<DxfChoices> {
        self.held.take()
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }
}

/// Open `bytes` (or place them with the `file.place` params `place`) through the dialog when they
/// are a DXF drawing; false when they aren't (or can't be read: the loader then says why).
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    if fileio::detect(name, bytes).is_none_or(|f| f.id != "dxf") {
        return false;
    }
    let Ok(info) = vectorcraft_cad::info(bytes) else { return false };
    let mut choices = DxfChoices {
        layout: info.layouts.first().cloned().unwrap_or_else(|| "Model".into()),
        unit: info.unit,
        scale: info.scale,
        ..DxfChoices::default()
    };
    if let Some(last) = app.ui.dxf_import.as_object() {
        let slots = [&mut choices.fit, &mut choices.scale_lineweights, &mut choices.center, &mut choices.merge_layers];
        for (key, slot) in REMEMBERED.into_iter().zip(slots) {
            if let Some(v) = last.get(key).and_then(Value::as_bool) {
                *slot = v;
            }
        }
    }
    let file = DialogFile::new(bytes.to_vec());
    let source = Source { place: place.map(|p| other_place_params(p, &["dxf"])), name: name.to_string(), path, file: file.clone() };
    let dialog = DxfImportDialog::new(place.is_some(), file, &info, choices);
    open_held(app, dialog, move |app, choices| confirm(app, &source, choices)).is_some()
}

/// `file.place` with params `p` asks first (true) when they name a `.dxf` drawing and no `dxf`
/// options.
pub fn offer_place(app: &mut VectorcraftApp, p: &Value) -> bool {
    let named = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).map(fileio::extension);
    if p.get("dxf").is_some() || named.as_deref() != Some("dxf") {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| ext == "dxf") else { return false };
    offer(app, &name, &bytes, path, Some(p))
}

/// The `dxf` options a drawing `file.place` params `p` name is placed with when Show Import Options
/// is off: the choices confirmed last (fit, lineweights, centring, layers), the drawing at 1:1 in
/// its own unit. `None` when `p` names no `.dxf` drawing or already gives `dxf` options.
pub fn last_options(app: &VectorcraftApp, p: &Value) -> Option<Value> {
    let named = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).map(fileio::extension);
    if p.get("dxf").is_some() || named.as_deref() != Some("dxf") {
        return None;
    }
    Some(remembered(app))
}

/// The `dxf` options of the choices confirmed last (fit, lineweights, centring, layers; `{}` when
/// none was).
pub fn remembered(app: &VectorcraftApp) -> Value {
    let last = app.ui.dxf_import.as_object();
    Value::Object(REMEMBERED.iter().filter_map(|k| last?.get(*k).filter(|v| v.is_boolean()).map(|v| (k.to_string(), v.clone()))).collect())
}

/// The `dxf` options of `document.open` and `file.place` the choices set.
fn options(c: &DxfChoices) -> Value {
    let mut o = json!({
        "layout": c.layout,
        "fit": c.fit,
        "scaleLineweights": c.scale_lineweights,
        "center": c.center,
        "mergeLayers": c.merge_layers,
    });
    if !c.fit {
        o["unit"] = json!(c.unit.label());
        o["scale"] = json!(c.scale);
    }
    o
}

/// OK: open or place the drawing with the choices; an error keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, source: &Source, choices: &DxfChoices) -> Result<(), String> {
    let dxf = options(choices);
    app.ui.dxf_import = Value::Object(REMEMBERED.iter().map(|k| (k.to_string(), dxf[*k].clone())).collect());
    match &source.place {
        Some(place) => {
            let mut p = with_file(place, source.path.as_deref(), &source.name, &source.file);
            p["dxf"] = dxf;
            crate::place::run_for_user(app, &p).map(|_| ())
        }
        None => io::open_document(app, &source.name, &source.file.bytes, source.path.clone(), &json!({ "dxf": dxf })).map(|_| ()),
    }
}

/// Renders the picture of the layout the open dialog wants, once per layout. A layout that can't be
/// read gets an empty picture, so it isn't read again.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(windows) = app.dialogs() else { return };
    let wanted = windows.with(|h| {
        let dialog = h.get::<DxfImportDialog>()?;
        Some((dialog.wanted_preview()?, dialog.file.clone(), dialog.choices.layout.clone()))
    });
    let Some((key, file, layout)) = wanted else { return };
    let o = vectorcraft_cad::ImportOptions { layout: Some(layout), fit: true, ..Default::default() };
    let image = vectorcraft_cad::import(&file.bytes, &o)
        .ok()
        .and_then(|imported| render_preview(app, ctx, &imported.document))
        .unwrap_or_else(|| Arc::new(ColorImage::filled([1, 1], Color32::TRANSPARENT)));
    windows.with(|h| {
        if let Some(dialog) = h.get_mut::<DxfImportDialog>() {
            dialog.set_preview(key, image);
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use egui::{Key, Modifiers};
    use vectorcraft_engine::doc::NodeKind;

    use super::super::text_import::test_support as host;
    use super::*;

    /// A millimetre drawing: a line in model space and a circle on a paper layout.
    fn drawing() -> Vec<u8> {
        let pairs: &[(i32, &str)] = &[
            (0, "SECTION"),
            (2, "HEADER"),
            (9, "$INSUNITS"),
            (70, "4"),
            (0, "ENDSEC"),
            (0, "SECTION"),
            (2, "ENTITIES"),
            (0, "LINE"),
            (8, "Walls"),
            (10, "0"),
            (20, "0"),
            (11, "100"),
            (21, "50"),
            (0, "CIRCLE"),
            (67, "1"),
            (10, "0"),
            (20, "0"),
            (40, "5"),
            (0, "ENDSEC"),
            (0, "EOF"),
        ];
        pairs.iter().map(|(c, v)| format!("{c:>3}\n{v}\n")).collect::<String>().into_bytes()
    }

    fn set(app: &VectorcraftApp, k: &str, v: Value) {
        host::set(app, KIND, k, v);
    }

    fn actual_dialog() -> Panel {
        let mut panel = Panel::new(host::app(), crate::tests::picks::dialogs);
        panel.room(egui::vec2(1000.0, 800.0));
        io::open_bytes(&mut panel.app(), "plan.dxf", &drawing(), None).unwrap();
        panel.frame();
        panel
    }

    fn preview_key(panel: &Panel) -> Option<Revision> {
        panel.app().dialogs().unwrap().with(|host| host.get::<DxfImportDialog>().unwrap().preview.as_ref().map(|(key, _)| *key))
    }

    fn accept_and_check_width(panel: &mut Panel, unit: Unit, width: f64) {
        panel.click("OK");
        panel.frame();
        assert!(!host::is_open(&panel.app(), KIND));
        let app = panel.app();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.units, unit);
        assert!((doc.artboards[0].rect.width() - width).abs() < 1e-6, "the imported drawing uses the accepted layout, unit and scale");
    }

    fn canceled_choice_keeps_the_original_import(field: &str) {
        let mut panel = actual_dialog();
        let original = host::fields(&panel.app(), KIND);
        assert_eq!(preview_key(&panel), Some(Revision::of(&"Model".to_owned())));
        panel.click_role(field, Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        panel.frame();
        let changed = host::fields(&panel.app(), KIND);
        if field == "Layout" {
            assert_eq!(changed["layout"], "Layout1");
            assert_eq!(preview_key(&panel), Some(Revision::of(&"Layout1".to_owned())), "the proposed layout is previewed");
        } else {
            assert_eq!(changed["unit"], "Centimeters");
        }
        panel.key(Key::Escape, Modifiers::NONE);
        panel.frame();
        assert_eq!(host::fields(&panel.app(), KIND), original, "Escape restores the choices normal OK will import");
        assert_eq!(preview_key(&panel), Some(Revision::of(&"Model".to_owned())));
        accept_and_check_width(&mut panel, Unit::Millimeters, 100.0 * Unit::Millimeters.points());
    }

    #[test]
    fn actual_layout_popup_escape_restores_the_preview_and_import() {
        canceled_choice_keeps_the_original_import("Layout");
    }

    #[test]
    fn actual_unit_popup_escape_restores_the_import_unit() {
        canceled_choice_keeps_the_original_import("Unit");
    }

    #[test]
    fn escape_while_typing_a_scale_cancels_the_import() {
        let mut panel = actual_dialog();
        let count = panel.app().session.documents().len();
        panel.click("Scale");
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text("2");
        assert_eq!(host::fields(&panel.app(), KIND)["scale"], 2.0);
        panel.key(Key::Escape, Modifiers::NONE);
        assert!(!host::is_open(&panel.app(), KIND));
        assert_eq!(panel.app().session.documents().len(), count);
        io::open_bytes(&mut panel.app(), "plan.dxf", &drawing(), None).unwrap();
        panel.frame();
        assert_eq!(host::fields(&panel.app(), KIND)["scale"], 1.0);
    }

    #[test]
    fn actual_layout_and_unit_enter_and_scale_blur_import_the_accepted_choices() {
        let mut panel = actual_dialog();
        for field in ["Layout", "Unit"] {
            panel.click_role(field, Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(Key::Enter, Modifiers::NONE);
        }
        panel.click("Scale");
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text("2");
        panel.key(Key::Tab, Modifiers::NONE);
        let choices = host::fields(&panel.app(), KIND);
        assert_eq!(choices["layout"], "Layout1");
        assert_eq!(choices["unit"], "Centimeters");
        assert_eq!(choices["scale"], 2.0);
        accept_and_check_width(&mut panel, Unit::Centimeters, 10.0 * Unit::Centimeters.points() / 2.0);
    }

    #[test]
    fn opening_a_drawing_asks_for_the_import_options() {
        let mut app = host::app();
        io::open_bytes(&mut app, "plan.dxf", &drawing(), None).unwrap();
        let f = host::fields(&app, KIND);
        assert_eq!((&f["mode"], &f["layout"], &f["unit"]), (&json!("open"), &json!("Model"), &json!("Millimeters")));
        assert_eq!(f["layouts"], json!(["Model", "Layout1"]));
        // A paper layout with its layers merged opens.
        set(&app, "fit", json!(true));
        set(&app, "layout", json!("Layout1"));
        set(&app, "mergeLayers", json!(true));
        host::confirm(&mut app, KIND).unwrap();
        assert!(!host::is_open(&app, KIND));
        let st = app.session.doc().unwrap();
        let mut circles = 0;
        st.doc.walk(|n| circles += usize::from(matches!(n.kind, NodeKind::Path { .. })));
        assert_eq!(circles, 1, "the paper layout's circle");
        assert_eq!(st.doc.layers.len(), 1);
        // The next drawing starts from these choices, its own scale.
        io::open_bytes(&mut app, "plan.dxf", &drawing(), None).unwrap();
        let f = host::fields(&app, KIND);
        assert!(f["fit"] == true && f["mergeLayers"] == true);
        assert_eq!(f["layout"], "Model");
    }

    #[test]
    fn bad_options_keep_the_dialog_open() {
        let mut app = host::app();
        io::open_bytes(&mut app, "plan.dxf", &drawing(), None).unwrap();
        set(&app, "layout", json!("Sheet 9"));
        assert!(host::confirm(&mut app, KIND).is_err());
        assert!(host::is_open(&app, KIND));
        assert_eq!(host::fields(&app, KIND)["layout"], "Sheet 9", "the typed values stay");
        set(&app, "layout", json!("Model"));
        set(&app, "unit", json!("Points"));
        set(&app, "scale", json!(1));
        host::confirm(&mut app, KIND).unwrap();
        let w = app.session.doc().unwrap().doc.artboards[0].rect.width();
        assert_eq!(w, 100.0, "1 pt = 1 unit");
    }

    #[test]
    fn placing_a_drawing_asks_then_places_one_group() {
        let mut app = host::app();
        app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
        let p = json!({"name": "plan.dxf", "dataBase64": vectorcraft_format::base64_encode(&drawing()), "template": false});
        crate::place::run(&mut app, &p).unwrap();
        assert_eq!(host::fields(&app, KIND)["mode"], "place");
        set(&app, "fit", json!(true));
        let undo = app.session.doc().unwrap().history.undo.len();
        host::confirm(&mut app, KIND).unwrap();
        assert!(!host::is_open(&app, KIND));
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let id = st.selection.objects[0];
        let b = st.doc.node(id).unwrap().geometric_bounds().unwrap();
        assert!((b.width() - 600.0).abs() < 1.0, "fitted to the artboard: {b:?}");
        // Params with dxf options place without asking.
        let mut p = p.clone();
        p["dxf"] = json!({});
        crate::place::run(&mut app, &p).unwrap();
        assert!(!host::is_open(&app, KIND));
    }
}
