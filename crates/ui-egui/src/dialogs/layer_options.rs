//! Layer Options (double-click a layer row, or ≡ › Options for Selection…) and the same dialog for
//! a new layer or sublayer (≡ › New Layer…, Alt-click Create New Layer). Objects' rows get Name,
//! Show and Lock only.
//!
//! Fields: `ids` (the rows edited; none for a new layer), `mode` (`edit`, `new` or `newSublayer`),
//! `layer` (the rows are layers), `name`, `color` (a preset's name such as "Light Blue", or
//! `#rrggbb`), `template`, `locked`, `visible`, `printable`, `preview`, `dimImages` and
//! `dimPercent`. OK runs `layer.setProps`, `layer.new` or `layer.newSublayer`. For several rows
//! the dialog shows the first row's settings, and OK sends only the ones set in the dialog.

use std::cell::RefCell;

// PLAN: `craft::color_ui` and `craft::theme` need the `craft` facade (feature `color-ui`) in ui-egui's Cargo.toml, which has only `craft-ui`; the owner of the craft-libs pin adds it.
use craft::color_ui::ui::{ChosenColor, ColorChoice, NamedColor};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{FormDialog, ResetFooter, ResetPick};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::requests::ColorRequest;
use craft::ui::text::{NumberField, TextField};
use serde_json::{Value, json};
use vectorcraft_doc::{LAYER_COLORS, LayerColor, Node, NodeId, NodeKind};

use crate::VectorcraftApp;

/// The key of the Layer Options dialog.
const KEY: &str = "ui.layerOptions";

/// The preset a layer without a colour of its own shows.
const FALLBACK_COLOR: &str = "Light Blue";

/// The fields a dialog sets, as `ui.dialog.set` names them.
const FIELDS: [&str; 9] = ["name", "color", "template", "visible", "preview", "locked", "printable", "dimImages", "dimPercent"];

/// The first row's Template, Lock, Dim Images and percentage as the dialog opened, for several rows.
#[derive(Clone, Debug, PartialEq)]
struct Opened {
    template: bool,
    locked: bool,
    dim_images: bool,
    dim_percent: f64,
}

/// The settings the dialog edits, with the marks of the ones set in it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerOptionsValue {
    name: String,
    color: ChosenColor,
    /// The custom colour, once one was picked or the layer has one.
    custom: Option<Srgba>,
    template: bool,
    visible: bool,
    preview: bool,
    locked: bool,
    printable: bool,
    dim_images: bool,
    dim_percent: f64,
    /// The rows are layers (not objects).
    layer: bool,
    /// More than one row is edited.
    many: bool,
    /// `edit`, `new` or `newSublayer`.
    mode: &'static str,
    ids: Vec<u64>,
    /// The fields set in the dialog, by its controls or `ui.dialog.set`.
    set: Vec<&'static str>,
    opened: Option<Opened>,
}

impl Default for LayerOptionsValue {
    fn default() -> Self {
        LayerOptionsValue {
            name: String::new(),
            color: ChosenColor::Named(FALLBACK_COLOR.to_owned()),
            custom: None,
            template: false,
            visible: true,
            preview: true,
            locked: false,
            printable: true,
            dim_images: false,
            dim_percent: 50.0,
            layer: false,
            many: false,
            mode: "edit",
            ids: Vec::new(),
            set: Vec::new(),
            opened: None,
        }
    }
}

/// `rgb` as a colour.
pub(super) fn srgb([r, g, b]: [u8; 3]) -> Srgba {
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// `color` as 8-bit channels; alpha is dropped.
pub(super) fn rgb8(color: Srgba) -> [u8; 3] {
    color.to_rgb_array().map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `#rrggbb` for `color`, whatever its alpha.
fn hex_of(color: Srgba) -> String {
    let [r, g, b] = rgb8(color);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// A layer colour as the dialog holds it: a preset's name, else the custom colour.
fn chosen_of(c: LayerColor) -> (ChosenColor, Option<Srgba>) {
    match c {
        LayerColor::Preset(i) => (ChosenColor::Named(LAYER_COLORS.get(i as usize).map_or(FALLBACK_COLOR, |(n, _)| *n).to_owned()), None),
        LayerColor::Custom(rgb) => (ChosenColor::Custom, Some(srgb(rgb))),
    }
}

impl LayerOptionsValue {
    /// Records that `field` was set.
    fn mark(&mut self, field: &'static str) {
        if !self.is_set(field) {
            self.set.push(field);
        }
    }

    /// Whether `field` was set in the dialog.
    fn is_set(&self, field: &str) -> bool {
        self.set.contains(&field)
    }

    /// The check box `key` names.
    fn flag(&self, key: &str) -> bool {
        match key {
            "template" => self.template,
            "visible" => self.visible,
            "preview" => self.preview,
            "locked" => self.locked,
            "printable" => self.printable,
            "dimImages" => self.dim_images,
            _ => false,
        }
    }

    /// Sets the check box `key` as its control does, and records it. Template also sets Lock and Dim Images.
    fn set_flag(&mut self, key: &'static str, on: bool) {
        match key {
            "template" if on != self.template => return self.set_template(on),
            "template" => {}
            "visible" => self.visible = on,
            "preview" => self.preview = on,
            "locked" => self.locked = on,
            "printable" => self.printable = on,
            "dimImages" => self.dim_images = on,
            _ => return,
        }
        self.mark(key);
    }

    /// Turns Template on or off. A template is locked and dims its images, and turning Template off
    /// unlocks it and turns Dim Images off; a Dim Images that was set in the dialog keeps its value.
    /// For several rows, OK sends Lock and Dim Images only when they are set in the dialog. `layer.setProps` then locks each row that
    /// becomes a template and dims it to 50%, unless a percentage is set in the dialog, so the dialog shows 50%; it unlocks
    /// each row that stops being a template and stops dimming it.
    /// When Template is set back to the first row's value, Lock and Dim Images that were not set in
    /// the dialog show the first row's values again.
    fn set_template(&mut self, on: bool) {
        self.template = on;
        self.mark("template");
        self.locked = on;
        if !self.is_set("dimImages") {
            self.dim_images = on;
        }
        let Some(opened) = self.opened.clone() else { return };
        if opened.template == on {
            if !self.is_set("locked") {
                self.locked = opened.locked;
            }
            if !self.is_set("dimImages") {
                self.dim_images = opened.dim_images;
            }
            if !self.is_set("dimPercent") {
                self.dim_percent = opened.dim_percent;
            }
        } else if on && !self.is_set("dimPercent") {
            self.dim_percent = 50.0;
        }
    }

    /// The colour as `layer.setProps` takes it: a preset's name, else `#rrggbb`.
    fn color_text(&self) -> String {
        match (&self.color, self.custom) {
            (ChosenColor::Named(name), _) => name.clone(),
            (_, Some(custom)) => hex_of(custom),
            _ => FALLBACK_COLOR.to_owned(),
        }
    }

    /// Sets the colour from a preset's name (any case) or `#rrggbb`; `None` for anything else.
    fn set_color_text(&mut self, text: &str) -> Option<()> {
        if let Some((name, _)) = LAYER_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(text)) {
            self.color = ChosenColor::Named((*name).to_owned());
        } else {
            self.custom = Some(Srgba::from_hex(text)?);
            self.color = ChosenColor::Custom;
        }
        self.mark("color");
        Some(())
    }

    /// Whether the row of `key` shows: Name is for one row, the rest of the settings for layers.
    fn shows(&self, key: &str) -> bool {
        match key {
            "name" => !self.many,
            "visible" | "locked" => true,
            _ => self.layer,
        }
    }

    /// Whether the row of `key` takes input: a template is locked and printable as it is.
    fn takes_input(&self, key: &str) -> bool {
        !(self.layer && self.template && matches!(key, "locked" | "printable"))
    }

    fn heading(&self) -> &'static str {
        if self.layer || self.mode != "edit" { tl!("Layer Options") } else { tl!("Options") }
    }
}

/// The value for `n` (a layer or any other row).
/// `xml_id`: Preferences › Units › Identify Objects By › XML ID.
fn fields_of(n: &Node, xml_id: bool) -> LayerOptionsValue {
    let mut v = LayerOptionsValue { name: n.identify_label(xml_id), visible: n.visible, locked: n.locked, layer: n.is_layer(), ..Default::default() };
    if let NodeKind::Layer { color, template, printable, preview, dim_images, .. } = &n.kind {
        (v.color, v.custom) = chosen_of(*color);
        v.template = *template;
        v.printable = *printable;
        v.preview = *preview;
        v.dim_images = dim_images.is_some();
        v.dim_percent = dim_images.unwrap_or(50) as f64;
    }
    v
}

/// Opens the dialog on `value`.
fn open_value(app: &mut VectorcraftApp, value: LayerOptionsValue) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(LayerOptionsDialog::new(st.uid, st.reverts, value)));
    Ok(Value::Null)
}

/// `ui.layerOptions {ids?|id?}`: open Layer Options for rows (default: the highlighted rows, else
/// the current layer), filled in from the first.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    // Rows given are used as given: a malformed or unknown id fails rather than being skipped or
    // the dialog opening on the highlighted rows instead.
    let row = |v: &Value| -> Result<NodeId, String> {
        let id = NodeId(v.as_u64().ok_or_else(|| format!("object ids are non-negative integers, not {v}"))?);
        st.doc.node(id).map(|_| id).ok_or_else(|| "no such layer or object".to_string())
    };
    let ids: Vec<NodeId> = match (p.get("ids"), p.get("id")) {
        (Some(Value::Array(a)), _) => a.iter().map(row).collect::<Result<_, _>>()?,
        (Some(v), _) => return Err(format!("`ids` must be an array of object ids, not {v}")),
        (None, Some(v)) => vec![row(v)?],
        (None, None) => {
            let rows = st.highlighted_rows();
            if rows.is_empty() { st.current_layer().into_iter().collect() } else { rows }
        }
    };
    let first = ids.first().and_then(|id| st.doc.node(*id)).ok_or("no such layer or object")?;
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    let mut v = fields_of(first, xml_id);
    v.many = ids.len() > 1;
    if v.many {
        v.opened = Some(Opened { template: v.template, locked: v.locked, dim_images: v.dim_images, dim_percent: v.dim_percent });
    }
    v.ids = ids.iter().map(|i| i.0).collect();
    open_value(app, v)
}

/// `ui.newLayer {sublayer?}`: Layer Options for a new layer (or sublayer), with the name and
/// colour it would get. In isolation mode a layer that would be outside the isolated art, where the
/// Layers panel does not list it, is an error.
pub fn open_new(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let sublayer = crate::menus::opt_bool(p, "sublayer")?.unwrap_or(false);
    if crate::panels::layers::new_layer_unlisted(st, sublayer, false) {
        return Err("exit isolation mode first".into());
    }
    let mut layers = vec![];
    st.doc.walk(|n| {
        if n.is_layer() {
            layers.push(n.display_name());
        }
    });
    let mut i = layers.len() + 1;
    while layers.contains(&format!("Layer {i}")) {
        i += 1;
    }
    let name = format!("Layer {i}");
    let name = if app.session.prefs.identify_objects_by == "xmlId" { vectorcraft_doc::xml_id::from_object_name(&name) } else { name };
    let (color, custom) = chosen_of(LayerColor::Preset((layers.len() % LAYER_COLORS.len()) as u8));
    let v = LayerOptionsValue { name, color, custom, layer: true, mode: if sublayer { "newSublayer" } else { "new" }, ..Default::default() };
    open_value(app, v)
}

/// A check box named `text`; the new state in the pass the user changed it.
pub(super) fn check(ui: &mut egui::Ui, text: &'static str, on: bool, enabled: bool) -> Option<bool> {
    Checkbox::new(Bind::given(on)).label(text).id_salt(text).enabled(enabled).show(ui).value().copied()
}

/// The check box of `key`.
fn flag_row(ui: &mut egui::Ui, v: &mut LayerOptionsValue, key: &'static str, text: &'static str) {
    if let Some(on) = check(ui, text, v.flag(key), v.takes_input(key)) {
        v.set_flag(key, on);
    }
}

/// The dialog's rows; the colour request of Custom… goes to `requested`.
fn body(ui: &mut egui::Ui, v: &mut LayerOptionsValue, requested: &RefCell<Option<ColorRequest>>) {
    if v.shows("name") {
        Field::new(tl!("Name:")).id_salt("name").show(ui, |ui, _| {
            TextField::new(tl!("Name:"), Bind::bound(&mut v.name)).show(ui);
        });
    }
    if v.layer {
        Field::new(tl!("Color:")).id_salt("color").show(ui, |ui, _| {
            let named = LAYER_COLORS.iter().map(|&(n, c)| NamedColor::new(n, tl!(n), Some(srgb(c))));
            let out = ColorChoice::new(tl!("Color:"), &v.color).named(named).custom(v.custom).show(ui);
            if let Some(chosen) = out.chosen {
                v.color = chosen;
                v.mark("color");
            }
            // A second pass of the frame has no press: only a request keeps what the first pass asked for.
            if let Some(request) = out.pick_color {
                *requested.borrow_mut() = Some(request);
            }
        });
    }
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            if v.layer {
                flag_row(ui, v, "template", tl!("Template"));
            }
            flag_row(ui, v, "visible", tl!("Show"));
            if v.layer {
                flag_row(ui, v, "preview", tl!("Preview"));
                Row::new().show(ui, |ui, _| {
                    flag_row(ui, v, "dimImages", tl!("Dim Images to:"));
                    let field = NumberField::new(tl!("Dim Images to:"), Bind::given(Some(v.dim_percent)))
                        .range(0.0..=100.0)
                        .unit_kind(UnitKind::Percent)
                        .enabled(v.dim_images)
                        .show(ui);
                    if let Some(Some(percent)) = field.edit.value().copied() {
                        v.dim_percent = percent;
                        v.mark("dimPercent");
                    }
                });
            }
        });
        Stack::new().show(ui, |ui, _| {
            flag_row(ui, v, "locked", tl!("Lock"));
            if v.layer {
                flag_row(ui, v, "printable", tl!("Print"));
            }
        });
    });
}

/// The command and params OK runs. For several rows only the settings set in the dialog are sent,
/// so each row keeps its own values for the rest.
pub(crate) fn params(v: &LayerOptionsValue) -> (&'static str, Value) {
    let mut p = serde_json::Map::new();
    if !v.many {
        p.insert("name".into(), json!(v.name));
    }
    p.insert("visible".into(), json!(v.visible));
    p.insert("locked".into(), json!(v.locked));
    if v.layer {
        p.insert("color".into(), json!(v.color_text()));
        p.insert("template".into(), json!(v.template));
        p.insert("printable".into(), json!(v.printable));
        p.insert("preview".into(), json!(v.preview));
        p.insert("dimImages".into(), if v.dim_images { json!(v.dim_percent.clamp(0.0, 100.0)) } else { json!(false) });
    }
    if v.many {
        // A percentage set in the dialog is sent whenever Dim Images is on, also when Dim Images
        // was turned on by Template.
        let percent = v.dim_images && v.is_set("dimPercent");
        p.retain(|k, _| v.is_set(k) || (k == "dimImages" && percent));
    }
    match v.mode {
        "new" => ("layer.new", Value::Object(p)),
        "newSublayer" => ("layer.newSublayer", Value::Object(p)),
        _ => {
            p.insert("ids".into(), json!(v.ids));
            ("layer.setProps", Value::Object(p))
        }
    }
}

/// The Layer Options dialog. OK files the value for [`pump`], which runs the command and answers it: a refusal keeps
/// the dialog open with the reason under its rows.
pub(crate) struct LayerOptionsDialog {
    id: DialogId,
    document: u64,
    reverts: u64,
    open: bool,
    value: LayerOptionsValue,
    held: HeldAnswer<LayerOptionsValue>,
    /// The value the held OK was refused for: the reason shows until it changes.
    refused_at: Option<LayerOptionsValue>,
    /// The Custom… press, until the app takes it.
    color_request: Option<ColorRequest>,
}

impl LayerOptionsDialog {
    /// The dialog over `value`.
    fn new(document: u64, reverts: u64, value: LayerOptionsValue) -> Self {
        LayerOptionsDialog {
            id: dialog_id(KEY),
            document,
            reverts,
            open: true,
            value,
            held: HeldAnswer::new(),
            refused_at: None,
            color_request: None,
        }
    }

    /// The OK filed by the user, once.
    fn take_filed(&mut self) -> Option<LayerOptionsValue> {
        self.held.take()
    }

    /// The app's answer to the filed OK: `Ok` closes the dialog, `Err` keeps it open with the reason.
    fn answer(&mut self, answer: Result<(), Label>) {
        if answer.is_err() {
            self.refused_at = Some(self.value.clone());
        }
        self.held.answer(answer);
    }
}

impl StackDialog for LayerOptionsDialog {
    type Result = LayerOptionsValue;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<LayerOptionsValue>> {
        let finish = self.held.is_accepted();
        let refusal = self.held.refusal().map(|(reason, _)| reason.clone());
        let requested = RefCell::new(None);
        let latest = RefCell::new(None);
        let footer = ResetFooter::new(ui, None, true);
        let shown = FormDialog::new(self.value.heading(), Bind::bound(&mut self.value))
            .opened(Bind::bound(&mut self.open))
            .accept_stays_open(true)
            .ok_enabled(!self.held.is_waiting())
            .show(ui, |ui, form, v| {
                // A measuring run closes nothing: the real run of the same pass makes the close.
                if finish && !craft::ui::runtime::measure::active(ui.ctx()) {
                    form.request_close_as(CloseReason::Activated);
                }
                body(ui, v, &requested);
                if let Some(reason) = &refusal {
                    Text::new(reason.clone()).tone(Tone::Bad).show(ui);
                }
                *latest.borrow_mut() = Some(v.clone());
                true
            });
        if let Some(request) = requested.into_inner() {
            self.color_request = Some(request);
        }
        if self.refused_at.as_ref().is_some_and(|at| *at != self.value) {
            self.refused_at = None;
            self.held.clear_refusal();
        }
        if finish && matches!(shown.closed, Some(CloseReason::Activated)) {
            return Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        // OK (the button, or Enter) files the value; the dialog stays open until the app answers.
        let pressed = footer.pick(shown.picked) == Some(ResetPick::Accept);
        if let Some(value) = shown.accepted.or_else(|| latest.into_inner().filter(|_| pressed)) {
            if !self.held.is_waiting() {
                self.held.file(value);
            }
            self.open = true;
            return None;
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<LayerOptionsValue> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if !self.held.is_waiting() {
            self.held.file(self.value.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let v = &self.value;
        let mut f = serde_json::Map::new();
        for key in FIELDS.into_iter().filter(|key| v.shows(key)) {
            let value = match key {
                "name" => json!(v.name),
                "color" => json!(v.color_text()),
                "dimPercent" => json!(v.dim_percent),
                _ => json!(v.flag(key)),
            };
            f.insert(key.into(), value);
        }
        // The name is a field of one row; several rows show none but keep it in their fields.
        f.entry("name").or_insert_with(|| json!(v.name));
        f.insert("ids".into(), json!(v.ids));
        f.insert("mode".into(), json!(v.mode));
        f.insert("layer".into(), json!(v.layer));
        Some(Value::Object(f))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if matches!(field, "ids" | "mode" | "layer") {
            return Err(FieldRefusal::ReadOnly.message(field));
        }
        let Some(key) = FIELDS.into_iter().find(|key| *key == field) else {
            return Err(FieldRefusal::Unknown.message(field));
        };
        let v = &mut self.value;
        if !v.shows(key) || !v.takes_input(key) {
            return Err(FieldRefusal::Inactive.message(field));
        }
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        match key {
            "name" => v.name = value.as_str().ok_or_else(rejects)?.to_owned(),
            "color" => v.set_color_text(value.as_str().ok_or_else(rejects)?).ok_or_else(rejects)?,
            "dimPercent" => {
                v.dim_percent = value.as_f64().filter(|p| p.is_finite()).ok_or_else(rejects)?;
                v.mark("dimPercent");
            }
            _ => v.set_flag(key, value.as_bool().ok_or_else(rejects)?),
        }
        Ok(())
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.color_request.take()
    }

    fn answer_color(&mut self, key: &str, color: Srgba) {
        if key == ColorChoice::CUSTOM_KEY {
            self.value.custom = Some(color);
            self.value.color = ChosenColor::Custom;
            self.value.mark("color");
        }
    }
}

/// Runs the OK Layer Options filed: the command, once the isolation check passes. A refusal keeps the dialog open; any
/// other command error goes to the status line and closes it.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    // The closed dialog's answer was run when OK was filed.
    let _ = dialogs.with(|host| host.take::<LayerOptionsDialog>());
    let Some((document, reverts, value)) = dialogs.with(|host| {
        let dialog = host.get_mut::<LayerOptionsDialog>()?;
        Some((dialog.document, dialog.reverts, dialog.take_filed()?))
    }) else {
        return;
    };
    let (cmd, p) = params(&value);
    let sublayer = match cmd {
        "layer.new" => Some(false),
        "layer.newSublayer" => Some(true),
        _ => None,
    };
    // Isolation mode can start while the dialog is open.
    let answer = match app.session.active().filter(|st| st.uid == document) {
        None => Err(Label::from(tl!("Layer Options can only change the document it was opened for."))),
        Some(st) if st.reverts != reverts => Err(Label::from(tl!("The document was reverted. Reopen this dialog to continue."))),
        Some(st) if sublayer.is_some_and(|sublayer| crate::panels::layers::new_layer_unlisted(st, sublayer, false)) => {
            Err(Label::from("exit isolation mode first"))
        }
        _ => {
            if let Err(e) = app.run(cmd, p) {
                app.status(e);
            }
            Ok(())
        }
    };
    dialogs.with(|host| {
        if let Some(dialog) = host.get_mut::<LayerOptionsDialog>() {
            dialog.answer(answer);
        }
    });
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app
    }

    /// The open dialog's id.
    fn open_id(app: &VectorcraftApp) -> DialogId {
        app.dialogs().unwrap().with(|host| host.open_dialogs().last().map(|d| d.id)).unwrap()
    }

    /// The open dialog's fields.
    fn fields(app: &VectorcraftApp) -> Value {
        app.dialogs().unwrap().with(|host| host.open_dialogs().last().and_then(|d| d.fields.clone())).unwrap()
    }

    /// Sets a field as `ui.dialog.set` does.
    fn set(app: &mut VectorcraftApp, field: &str, value: Value) {
        try_set(app, field, value).unwrap();
    }

    fn try_set(app: &mut VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = app.ui_ctx.clone().unwrap();
        let id = open_id(app);
        app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, field, value))
    }

    /// The command and params OK would run for the open dialog.
    fn current_params(app: &VectorcraftApp) -> Value {
        app.dialogs().unwrap().with(|host| params(&host.get::<LayerOptionsDialog>().unwrap().value).1)
    }

    /// Presses OK and runs the host pump, which runs the command; the second OK closes an accepted dialog.
    fn ok(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        let id = open_id(app);
        let dialogs = app.dialogs().unwrap();
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
    }

    /// Explicit object ids must not silently disappear or fall back to the highlighted row.
    #[test]
    fn invalid_layer_options_targets_do_not_open_a_partial_dialog() {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        for params in
            [json!({"ids": [id, "invalid"]}), json!({"ids": [id, 999999]}), json!({"ids": "invalid"}), json!({"id": "invalid"}), json!({"id": null})]
        {
            assert!(app.run("ui.layerOptions", params.clone()).is_err(), "accepted {params}");
            assert!(!app.any_dialog_open(), "opened partial options for {params}");
        }
        app.run("ui.layerOptions", json!({"ids": [id]})).unwrap();
        assert_eq!(fields(&app)["ids"], json!([id]));
    }

    #[test]
    fn layer_options_edit_the_layer_and_ok_is_one_undo_step() {
        let mut app = app();
        let layer = app.session.doc().unwrap().doc.layers[0].id;
        app.run("ui.layerOptions", json!({})).unwrap();
        let f = fields(&app);
        assert_eq!((f["name"].as_str(), f["color"].as_str()), (Some("Layer 1"), Some("Light Blue")));
        set(&mut app, "name", json!("Sky"));
        set(&mut app, "color", json!("#00ff00"));
        set(&mut app, "dimImages", json!(true));
        set(&mut app, "dimPercent", json!(30));
        set(&mut app, "preview", json!(false));
        let undo = app.session.doc().unwrap().history.undo.len();
        ok(&mut app);
        assert!(!app.any_dialog_open());
        let n = app.session.doc().unwrap().doc.node(layer).unwrap().clone();
        assert_eq!(n.display_name(), "Sky");
        assert!(matches!(n.kind, NodeKind::Layer { color: LayerColor::Custom([0, 255, 0]), dim_images: Some(30), preview: false, .. }));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    }

    #[test]
    fn layer_options_and_new_layers_follow_their_document() {
        for (command, arguments) in [("ui.layerOptions", json!({})), ("ui.newLayer", json!({})), ("ui.newLayer", json!({"sublayer": true}))] {
            let mut app = app();
            let source_id = app.session.active().unwrap().doc.layers[0].id;
            app.run("file.new", json!({"width": 160, "height": 160})).unwrap();
            let destination_id = app.session.active().unwrap().doc.layers[0].id;
            assert_eq!(destination_id, source_id, "the two documents reuse a layer ID");
            let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
            activate(&mut app, 0);
            app.run(command, arguments).unwrap();
            set(&mut app, "name", json!("Source Draft"));
            let ctx = app.ui_ctx.clone().unwrap();
            let dialog = open_id(&app);
            app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, dialog));

            let layers = |app: &VectorcraftApp| {
                let mut names = vec![];
                app.session.active().unwrap().doc.walk(|node| {
                    if node.is_layer() {
                        names.push(node.display_name());
                    }
                });
                names
            };
            activate(&mut app, 1);
            let before = layers(&app);
            let b_undo = app.session.active().unwrap().history.undo.len();
            pump(&mut app, &ctx);
            assert_eq!(layers(&app), before, "the destination's layers stay unchanged for {command}");
            assert_eq!(app.session.active().unwrap().history.undo.len(), b_undo);
            assert!(matches!(app.dialogs().unwrap().with(|host| host.held_state(dialog)), Some(HeldOk::Refused(_))));

            activate(&mut app, 0);
            let a_undo = app.session.active().unwrap().history.undo.len();
            ok(&mut app);
            assert!(!app.any_dialog_open());
            assert!(layers(&app).iter().any(|name| name == "Source Draft"));
            assert_eq!(app.session.active().unwrap().history.undo.len(), a_undo + 1);
        }
    }

    /// Two layers with different options, the first plain and the second green, locked, in
    /// outline view and dimmed to 30%.
    fn two_layers(app: &mut VectorcraftApp) -> (NodeId, NodeId) {
        let first = app.session.doc().unwrap().doc.layers[0].id;
        let p = json!({"color": "#00ff00", "locked": true, "preview": false, "dimImages": 30});
        let second = NodeId(app.session.execute("layer.new", &p).unwrap()["id"].as_u64().unwrap());
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        (first, second)
    }

    fn layer(app: &VectorcraftApp, id: NodeId) -> Node {
        app.session.doc().unwrap().doc.node(id).unwrap().clone()
    }

    /// A setting set to the first layer's own value still goes to every layer.
    #[test]
    fn layer_options_for_several_layers_send_only_the_settings_set() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        set(&mut app, "printable", json!(false));
        set(&mut app, "color", json!("Light Blue"));
        assert_eq!(current_params(&app), json!({"printable": false, "color": "Light Blue", "ids": [first.0, second.0]}));
        ok(&mut app);
        let (a, b) = (layer(&app, first), layer(&app, second));
        assert!(matches!(a.kind, NodeKind::Layer { color: LayerColor::Preset(0), printable: false, preview: true, dim_images: None, .. }));
        assert!(!a.locked);
        let NodeKind::Layer { color, printable, preview, dim_images, .. } = b.kind else { panic!("not a layer") };
        assert_eq!((color, printable, preview, dim_images, b.locked), (LayerColor::Preset(0), false, false, Some(30), true));
    }

    /// Template locks and dims to 50% each layer whose Template it changes; a Dim Images
    /// percentage set as well goes to every layer.
    #[test]
    fn template_for_several_layers_leaves_lock_and_dimming_to_each_layer() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [second.0], "template": true, "dimImages": 30})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "template", json!(true));
        assert_eq!(current_params(&app), json!({"template": true, "ids": [first.0, second.0]}));
        ok(&mut app);
        let (a, b) = (layer(&app, first), layer(&app, second));
        assert!(a.is_template() && a.locked && matches!(a.kind, NodeKind::Layer { dim_images: Some(50), .. }));
        assert!(b.is_template() && b.locked && matches!(b.kind, NodeKind::Layer { dim_images: Some(30), .. }));
        // The first layer is dimmed to 50% now; 50% set again goes to both.
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "dimPercent", json!(50));
        assert_eq!(current_params(&app), json!({"dimImages": 50.0, "ids": [first.0, second.0]}));
        ok(&mut app);
        assert!(matches!(layer(&app, second).kind, NodeKind::Layer { dim_images: Some(50), .. }));
        // A percentage set while Dim Images is off is sent once Dim Images is on.
        app.run("layer.setProps", json!({"ids": [first.0], "template": false, "dimImages": false})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "dimPercent", json!(20));
        assert_eq!(current_params(&app), json!({"ids": [first.0, second.0]}));
        set(&mut app, "dimImages", json!(true));
        assert_eq!(current_params(&app), json!({"dimImages": 20.0, "ids": [first.0, second.0]}));
    }

    /// A percentage set while Dim Images is off is sent when Template turns Dim Images on.
    #[test]
    fn a_percentage_set_before_template_turns_dimming_on_goes_to_every_layer() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        set(&mut app, "dimPercent", json!(20));
        set(&mut app, "template", json!(true));
        assert_eq!(current_params(&app), json!({"template": true, "dimImages": 20.0, "ids": [first.0, second.0]}));
        ok(&mut app);
        for id in [first, second] {
            assert!(matches!(layer(&app, id).kind, NodeKind::Layer { template: true, dim_images: Some(20), .. }));
        }
    }

    /// Template set through `ui.dialog.set` locks and dims as its checkbox does, and set back it
    /// shows the first layer's Lock and Dim Images again and sends neither.
    #[test]
    fn template_set_and_set_back_leaves_lock_and_dimming_alone() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [first.0], "locked": true})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "template", json!(true));
        let f = fields(&app);
        assert!(f["locked"] == json!(true) && f["dimImages"] == json!(true));
        assert_eq!(current_params(&app), json!({"template": true, "ids": [first.0, second.0]}));
        set(&mut app, "template", json!(false));
        let f = fields(&app);
        assert!(f["locked"] == json!(true) && f["dimImages"] == json!(false));
        assert_eq!(current_params(&app), json!({"template": false, "ids": [first.0, second.0]}));
        ok(&mut app);
        assert!(layer(&app, first).locked && layer(&app, second).locked);
        assert!(matches!(layer(&app, second).kind, NodeKind::Layer { dim_images: Some(30), .. }));
    }

    /// Template turned on shows the 50% that `layer.setProps` dims each new template to, and
    /// turned back off shows the first layer's own percentage again.
    #[test]
    fn template_shows_the_percentage_it_dims_to() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [first.0], "dimImages": 30})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "template", json!(true));
        assert_eq!(fields(&app)["dimPercent"].as_f64(), Some(50.0));
        set(&mut app, "template", json!(false));
        let f = fields(&app);
        assert_eq!(f["dimImages"], json!(true));
        assert_eq!(f["dimPercent"].as_f64(), Some(30.0));
        assert_eq!(current_params(&app), json!({"template": false, "ids": [first.0, second.0]}));
        set(&mut app, "template", json!(true));
        ok(&mut app);
        assert!(matches!(layer(&app, first).kind, NodeKind::Layer { template: true, dim_images: Some(50), .. }));
    }

    /// A percentage set while Dim Images is on stays sent when Template is then set on and off,
    /// since Template leaves Dim Images on.
    #[test]
    fn a_percentage_typed_before_template_goes_to_every_layer() {
        let mut app = app();
        let (first, second) = two_layers(&mut app);
        app.run("layer.setProps", json!({"ids": [first.0], "dimImages": 30})).unwrap();
        app.run("ui.layerOptions", json!({"ids": [first.0, second.0]})).unwrap();
        set(&mut app, "dimPercent", json!(40));
        set(&mut app, "template", json!(true));
        assert_eq!(current_params(&app), json!({"template": true, "dimImages": 40.0, "ids": [first.0, second.0]}));
        // Template off again shows the first layer's Dim Images, on, and the percentage still
        // goes to every layer.
        set(&mut app, "template", json!(false));
        assert_eq!(current_params(&app), json!({"template": false, "dimImages": 40.0, "ids": [first.0, second.0]}));
    }

    /// A Dim Images set in the dialog stays as set when Template is set after it, so
    /// `ui.dialog.set` gives the same result in either order. Template still locks the layers.
    #[test]
    fn template_keeps_dim_images_that_was_set() {
        for dim_first in [true, false] {
            let mut app = app();
            let (first, second) = two_layers(&mut app);
            if dim_first {
                set(&mut app, "dimImages", json!(false));
                set(&mut app, "template", json!(true));
            } else {
                set(&mut app, "template", json!(true));
                set(&mut app, "dimImages", json!(false));
            }
            let f = fields(&app);
            assert!(f["template"] == json!(true) && f["locked"] == json!(true) && f["dimImages"] == json!(false), "dim first: {dim_first}");
            assert_eq!(current_params(&app), json!({"template": true, "dimImages": false, "ids": [first.0, second.0]}), "dim first: {dim_first}");
            ok(&mut app);
            for id in [first, second] {
                let n = layer(&app, id);
                assert!(n.locked && matches!(n.kind, NodeKind::Layer { template: true, dim_images: None, .. }), "{id:?}, dim first: {dim_first}");
            }
        }
    }

    /// OK in New Layer Options makes no layer outside the isolated art when isolation mode started
    /// while the dialog was open; the dialog stays open with the reason.
    #[test]
    fn new_layer_options_ok_checks_isolation_again() {
        let mut app = app();
        let id = |v: Value| v["id"].as_u64().unwrap();
        let rect = id(app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
        app.session.execute("select.set", &json!({"ids": [rect]})).unwrap();
        let group = id(app.session.execute("object.group", &json!({})).unwrap());
        app.run("ui.newLayer", json!({})).unwrap();
        app.session.execute("object.isolate", &json!({"id": group})).unwrap();
        // OK once and the pump's answer; a second OK would file the refused value again.
        let (ctx, dialog) = (app.ui_ctx.clone().unwrap(), open_id(&app));
        app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, dialog));
        crate::dialogs::host::pump(&mut app, &ctx);
        let state = app.dialogs().unwrap().with(|host| host.held_state(dialog));
        assert!(matches!(state, Some(HeldOk::Refused(_))), "{state:?}");
        assert_eq!(app.session.doc().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn new_layer_options_make_a_layer_and_objects_get_name_show_and_lock() {
        let mut app = app();
        app.run("ui.newLayer", json!({})).unwrap();
        assert_eq!(fields(&app)["name"], json!("Layer 2"));
        set(&mut app, "name", json!("Ink"));
        // Template locks the layer and takes Lock out of the dialog's reach.
        set(&mut app, "locked", json!(true));
        set(&mut app, "template", json!(true));
        assert!(try_set(&mut app, "locked", json!(false)).is_err());
        ok(&mut app);
        let d = &app.session.doc().unwrap().doc;
        let top = d.layers.last().unwrap();
        assert_eq!(top.display_name(), "Ink");
        assert!(top.is_template() && top.locked);
        // An object's row: Name, Show and Lock.
        app.session.execute("layer.setCurrent", &json!({"id": app.session.doc().unwrap().doc.layers[0].id.0})).unwrap();
        let rect = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.layerOptions", json!({"id": rect})).unwrap();
        assert_eq!(fields(&app)["layer"], json!(false));
        assert!(try_set(&mut app, "template", json!(true)).is_err(), "an object has no Template");
        set(&mut app, "name", json!("Box"));
        set(&mut app, "locked", json!(true));
        assert_eq!(current_params(&app)["template"], Value::Null);
        ok(&mut app);
        let n = app.session.doc().unwrap().doc.node(NodeId(rect)).unwrap().clone();
        assert_eq!((n.display_name(), n.locked), ("Box".to_string(), true));
    }

    #[test]
    fn layer_options_refuses_a_stale_draft_after_confirmed_revert() {
        let file = std::env::temp_dir().join(format!("vectorcraft-layer-options-revert-{}.vectorcraft", std::process::id()));
        let path = file.to_string_lossy();
        let mut app = app();
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|err| err.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|err| err.to_string())));
        app.run("file.saveAs", json!({"path": path})).unwrap();
        let original = app.session.active().unwrap();
        let (uid, reverts, layer) = (original.uid, original.reverts, original.doc.layers[0].id);
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();

        app.run("ui.layerOptions", json!({"id": layer.0})).unwrap();
        set(&mut app, "name", json!("Stale Draft"));
        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let _ = std::fs::remove_file(&file);
        let reverted = app.session.active().unwrap();
        assert_eq!((reverted.uid, reverted.reverts, reverted.doc.layers[0].id), (uid, reverts + 1, layer));
        let undo = reverted.history.undo.len();

        let ctx = app.ui_ctx.clone().unwrap();
        let id = open_id(&app);
        let dialogs = app.dialogs().unwrap();
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        let reverted = app.session.active().unwrap();
        assert_eq!(reverted.doc.node(layer).unwrap().display_name(), "Layer 1");
        assert_eq!(reverted.history.undo.len(), undo);
        assert!(app.any_dialog_open(), "the refused draft stays available to inspect or cancel");
        assert_eq!(fields(&app)["name"], "Stale Draft");
        assert_eq!(
            dialogs.with(|host| host.held_state(id)),
            Some(HeldOk::Refused("The document was reverted. Reopen this dialog to continue.".into()))
        );
        dialogs.with(|host| host.cancel_by_id(&ctx, id));
        app.run("ui.layerOptions", json!({"id": layer.0})).unwrap();
        set(&mut app, "name", json!("Fresh Draft"));
        ok(&mut app);
        assert_eq!(app.session.active().unwrap().doc.node(layer).unwrap().display_name(), "Fresh Draft");
    }
}
