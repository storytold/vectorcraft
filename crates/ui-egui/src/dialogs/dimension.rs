//! Dimension Tool Options, the tool's control strip, and the Properties panel's Dimension section.
//! Dialog values are drafts until Save, which sets the style of new dimensions.
use crate::chrome::rank;
use crate::edits::{EditPath, apply_edits};
use crate::font_menu::{picker_settings, store_picker_settings};
use crate::panels::{live_run, stroke};
use crate::{VectorcraftApp, i18n};
use craft::color_ui::ui::{ColorPickerDialog, ColorPickerResult};
use craft::font_manager::picker::{FontFilters, FontPicker, FontPickerSettings};
use craft::theme::variant::Tone;
use craft::theme::{Rgba, Srgba};
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented, Swatch};
use craft::ui::contract::overlay::{Dismiss, Outside};
use craft::ui::contract::{Bind, EditResponse, Entry, FieldLabelRole, Overlay, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::{
    DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id, serde_fields, set_serde_field,
};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{FormCx, FormDialog, FormOutput, HostRequests, ResetFooter, ResetPick};
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::measure;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::requests::{ColorRequest, HostAnswer};
use craft::ui::sections::GroupBox;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, TextField};
use egui::{Rect, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vectorcraft_doc::dimension::{ArrowStyle, DimensionOptions};
use vectorcraft_doc::{Arrowhead, NodeId};
use vectorcraft_text::FontDb;

/// The key of the Tool Options dialog: its id on the host and the name `ui.dialog.set` and the control channel know it by.
pub const KIND: &str = "dimensionOptions";

/// (value, label) of each choice.
type Choices = &'static [(&'static str, &'static str)];
const UNITS: Choices = &[
    ("in", "Inches"),
    ("mm", "Millimeters"),
    ("pt", "Points"),
    ("pc", "Picas"),
    ("px", "Pixels"),
    ("cm", "Centimeters"),
    ("m", "Meters"),
    ("ft", "Feet"),
    ("yd", "Yards"),
];
const LINE_TYPES: Choices = &[("solid", "Solid"), ("dashed", "Dashed"), ("dotted", "Dotted")];
/// One Arrow Style entry: the `arrowStyle` and `arrowHead` values it sets, and its name.
struct ArrowChoice {
    style: ArrowStyle,
    head: Option<Arrowhead>,
    name: &'static str,
}
impl ArrowChoice {
    /// The arrowhead the dimension line draws with this entry.
    fn drawn(&self) -> Option<Arrowhead> {
        self.head.clone().or_else(|| self.style.arrowhead())
    }
}
/// The Arrow Style entries: Filled, Open, Tick and None, then the other built-in stroke arrowheads
/// in the Stroke panel's order. A stroke arrowhead keeps `filled` as its `arrowStyle`, the head that
/// versions which do not read `arrowHead` draw when they draw the dimension again.
fn arrow_choices() -> Vec<ArrowChoice> {
    let named = [(ArrowStyle::Filled, "Filled"), (ArrowStyle::Open, "Open"), (ArrowStyle::Tick, "Tick"), (ArrowStyle::None, "None")];
    let heads = Arrowhead::ALL.into_iter().filter(|h| !named.iter().any(|(s, _)| s.arrowhead().as_ref() == Some(h)));
    named
        .into_iter()
        .map(|(style, name)| ArrowChoice { style, head: None, name })
        .chain(heads.map(|h| ArrowChoice { style: ArrowStyle::Filled, name: h.label(), head: Some(h) }))
        .collect()
}
const POSITIONS: Choices = &[("away", "Away from the Line"), ("between", "Between Line and Object")];
const ALIGNMENTS: Choices = &[("left", "Left"), ("center", "Center"), ("right", "Right")];
const SCALES: [&str; 11] = ["1:1", "1:2", "1:5", "1:10", "1:20", "1:50", "1:100", "1:200", "2:1", "5:1", "10:1"];
/// The kinds of dimension the strip switches between: (`mode` value, label).
const MODES: [(&str, &str); 3] = [("linear", "Linear"), ("angular", "Angular"), ("radial", "Radial")];
/// Where the floating strip sits from the canvas corner.
const STRIP_OFFSET: egui::Vec2 = egui::vec2(24.0, 24.0);

/// The option a number field edits.
type NumSlot = fn(&mut DimensionOptions) -> &mut f64;

/// A number field of the options: its labels, the number it edits and the range and unit it shows.
#[derive(Clone, Copy)]
struct Num {
    /// The label beside the field.
    label: &'static str,
    /// The field's accessible name, which tells apart the fields that share a label.
    name: &'static str,
    slot: NumSlot,
    min: f64,
    max: f64,
    unit: &'static str,
}
const LINE_WEIGHT: Num = Num { label: "Line Weight", name: "Line Weight", slot: |o| &mut o.line_weight, min: 0.1, max: 100.0, unit: "pt" };
const ARROW_SCALE: Num = Num { label: "Arrow Scale", name: "Arrow Scale", slot: |o| &mut o.arrow_scale, min: 1.0, max: 1000.0, unit: "%" };
const EXTENSION_WEIGHT: Num =
    Num { label: "Line Weight", name: "Extension Line Weight", slot: |o| &mut o.extension_weight, min: 0.1, max: 100.0, unit: "pt" };
const EXTENSION_OFFSET: Num =
    Num { label: "Offset", name: "Extension Offset", slot: |o| &mut o.extension_offset, min: 0.0, max: 10000.0, unit: "pt" };
const EXTENSION_LENGTH: Num =
    Num { label: "Extension", name: "Extension Length", slot: |o| &mut o.extension_length, min: 0.0, max: 10000.0, unit: "pt" };
const FONT_SIZE: Num = Num { label: "Font Size", name: "Font Size", slot: |o| &mut o.font_size, min: 1.0, max: 1000.0, unit: "pt" };
const TEXT_OFFSET: Num = Num { label: "Offset", name: "Text Offset", slot: |o| &mut o.text_offset, min: 0.0, max: 10000.0, unit: "pt" };

/// A colour of the options: the swatch's name, the key its picker request carries and the colour it edits.
#[derive(Clone, Copy)]
struct Tint {
    name: &'static str,
    key: &'static str,
    slot: fn(&mut DimensionOptions) -> &mut [u8; 3],
}
const LINE_TINT: Tint = Tint { name: "Line Color", key: "lineColor", slot: |o| &mut o.line_color };
const EXTENSION_TINT: Tint = Tint { name: "Extension Line Color", key: "extensionColor", slot: |o| &mut o.extension_color };
const TEXT_TINT: Tint = Tint { name: "Text Color", key: "textColor", slot: |o| &mut o.text_color };
const TINTS: [Tint; 3] = [LINE_TINT, EXTENSION_TINT, TEXT_TINT];

/// Which of the two kinds of line a row edits.
#[derive(Clone, Copy)]
enum Line {
    Dimension,
    Extension,
}

/// What the rows report to their caller.
#[derive(Default)]
struct Edited {
    /// A control other than a number field changed a value.
    set: bool,
    /// The number fields' edits of the pass, with the number each one edits.
    numbers: Vec<(NumSlot, EditResponse<Option<f64>>)>,
    /// A swatch asked for a colour picker.
    pick: Option<ColorRequest>,
}

/// The Properties section's colour picker and the swatch, document and dimension it edits.
#[derive(Clone, Debug)]
pub(crate) struct Pick {
    /// The open colour picker.
    pub id: DialogId,
    /// The key of the swatch that asked.
    pub key: String,
    /// The document and loaded contents that contain the dimension.
    pub document: (u64, u64),
    /// The dimension the colour goes to.
    pub target: NodeId,
}

/// The Tool Options dialog: a draft of the style that Save files for the app to set on the tool. Its body takes
/// nothing from the app, so everything it draws is taken when it opens.
pub(crate) struct DimensionDialog {
    id: DialogId,
    draft: DimensionOptions,
    /// The style the dialog opened with, which the footer's Reset restores.
    opened: DimensionOptions,
    font: FontPickerSettings,
    /// The settings the app holds, to tell when the picker changed them.
    font_stored: FontPickerSettings,
    filters: FontFilters,
    requests: HostRequests,
    held: HeldAnswer<DimensionOptions>,
}

impl DimensionDialog {
    /// The dialog over `style`, with the family picker's settings as the app holds them.
    fn new(style: DimensionOptions, font: FontPickerSettings) -> Self {
        DimensionDialog {
            id: dialog_id(KIND),
            draft: style.clone(),
            opened: style,
            font_stored: font.clone(),
            font,
            filters: FontFilters::default(),
            requests: HostRequests::new(),
            held: HeldAnswer::new(),
        }
    }

    /// The picker settings as (the app's, the picker's) when the picker changed them since the last call.
    fn font_change(&mut self) -> Option<(FontPickerSettings, FontPickerSettings)> {
        if self.font == self.font_stored {
            return None;
        }
        let before = std::mem::replace(&mut self.font_stored, self.font.clone());
        Some((before, self.font.clone()))
    }
}

impl StackDialog for DimensionDialog {
    type Result = DimensionOptions;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    /// Files the draft as Save does, and closes with it once `pump` accepted it.
    fn confirm(&mut self, _: &egui::Context) -> Option<DimensionOptions> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() || read(&self.draft).is_err() {
            return None;
        }
        self.held.file(self.draft.clone());
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        serde_fields(&self.draft)
    }

    /// For `ui.dialog.set`, as the Arrow Style menu sets them: an `arrowStyle` set alone draws that style's own head,
    /// and a built-in head set as `arrowHead` sets both values of its Arrow Style entry (`tick` with no `arrowHead` for
    /// `Slash`, `filled` with `Barbed` for `Barbed`). The default style has no `arrowHead` field and still takes one.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "arrowStyle" => {
                let mut draft = self.draft.clone();
                draft.arrow_head = None;
                set_serde_field(&mut draft, field, value)?;
                self.draft = draft;
            }
            "arrowHead" if value.is_null() => self.draft.arrow_head = None,
            "arrowHead" => {
                let head = built_in_head(&value);
                let choice = arrow_choices().into_iter().find(|ch| head.is_some() && ch.drawn() == head);
                let choice = choice.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                self.draft.arrow_style = choice.style;
                self.draft.arrow_head = choice.head;
            }
            _ => set_serde_field(&mut self.draft, field, value)?,
        }
        Ok(())
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    fn answer_color(&mut self, key: &str, color: Srgba) {
        self.requests.answer_color(key, color);
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.requests.cancel();
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<DimensionOptions>> {
        let DimensionDialog { draft, opened, font, filters, requests, held, .. } = self;
        let finish = held.is_accepted();
        let reset = ResetFooter::new(ui, None, true);
        let refused = held.refusal().map(|(reason, at)| (reason.clone(), at.clone()));
        let invalid = read(draft).err();
        let mut dialog = FormDialog::new(tl!("Dimension Tool Options"), Bind::bound(&mut *draft))
            .ok_label(tl!("Save"))
            .ok_enabled(invalid.is_none() && !held.is_waiting())
            .accept_stays_open(true)
            .opening(opened.clone())
            .answers(requests.take_answers());
        if let Some(reason) = invalid {
            dialog = dialog.ok_reason(reason);
        }
        let shown = dialog.show(ui, |ui, form, o| {
            // A measuring run closes nothing: the real run of the pass makes the close.
            if finish && !measure::active(ui.ctx()) {
                form.request_close_as(CloseReason::Activated);
            }
            dialog_rows(ui, form, o, font, filters, refused.as_ref())
        });
        let FormOutput { closed, picked, requests: filed, .. } = shown;
        requests.extend(filed);
        if reset.pick(picked) == Some(ResetPick::Accept) && read(draft).is_ok() {
            held.file(draft.clone());
        }
        // The refusal shows until the values it refused change.
        if held.refusal().is_some_and(|(_, at)| *at != *draft) {
            held.clear_refusal();
        }
        match closed {
            None => None,
            Some(CloseReason::Activated) if finish => Some(held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
            Some(_) => {
                requests.cancel();
                Some(DialogResult::Cancel)
            }
        }
    }
}

/// Opens the Tool Options on the tool's current style.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.tool_id() == "dimension" && app.session.tool_busy() {
        app.run("tool.select", json!({"tool":"selection"}))?;
        app.run("tool.select", json!({"tool":"dimension"}))?;
    }
    let style = app.session.tool_options_of("dimension")["style"].clone();
    let o: DimensionOptions = serde_json::from_value(style).unwrap_or_default();
    let dialog = DimensionDialog::new(o, picker_settings(app));
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(dialog));
    Ok(json!({"dialog":KIND}))
}

/// Runs once a frame: sets the tool's style from a Save the dialog filed and answers it, keeps the family picker's
/// settings, and applies the colour the Properties section's picker answered.
pub(crate) fn pump(app: &mut VectorcraftApp, _: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KIND);
    let (filed, font) = dialogs.with(|host| match host.get_mut_by_id::<DimensionDialog>(id) {
        Some(dialog) => (dialog.held.take(), dialog.font_change()),
        None => (None, None),
    });
    if let Some((before, after)) = font {
        store_picker_settings(app, &before, after);
    }
    if let Some(style) = filed {
        let answer = app.run("tool.setOption", json!({"tool":"dimension","values":{"style":style}})).map(drop).map_err(Label::from);
        dialogs.with(|host| {
            if let Some(dialog) = host.get_mut_by_id::<DimensionDialog>(id) {
                dialog.held.answer(answer);
            }
        });
    }
    let Some(pick) = app.object_dialogs.dimension.clone() else { return };
    let Some(finished) = dialogs.with(|host| host.take_by_id::<ColorPickerDialog>(pick.id)) else { return };
    app.object_dialogs.dimension = None;
    let Some(ColorPickerResult { picked, .. }) = finished.ok() else { return };
    let Some(source) = app.session.active().filter(|st| st.uid == pick.document.0) else {
        app.status("The active document changed while the Color Picker was open");
        return;
    };
    if source.reverts != pick.document.1 {
        app.status(tl!("The document was reverted. Reopen this dialog to continue."));
        return;
    }
    if let Some(mut o) = options_of(app, pick.target)
        && set_color(&mut o, &pick.key, picked.color)
    {
        update(app, pick.target.0, &o);
    }
}

/// The built-in stroke arrowhead `value` names. Only a short name is read: a draft may hold anything.
fn built_in_head(value: &Value) -> Option<Arrowhead> {
    let name = value.as_str().filter(|name| name.len() <= 64)?;
    serde_json::from_value::<Arrowhead>(Value::String(name.into())).ok().filter(|h| !matches!(h, Arrowhead::Custom(_)))
}

fn read(o: &DimensionOptions) -> Result<(), String> {
    o.validate()
}

/// Writes `color` into the colour the swatch `key` edits; whether `key` names one.
fn set_color(o: &mut DimensionOptions, key: &str, color: Srgba) -> bool {
    let Some(tint) = TINTS.iter().find(|tint| tint.key == key) else { return false };
    let Rgba([r, g, b, _]) = color.to_rgba();
    *(tint.slot)(o) = [r, g, b];
    true
}

/// Runs `dimension.update` with the whole style, or reports why it is not one.
fn update(app: &mut VectorcraftApp, id: u64, o: &DimensionOptions) {
    let ran = read(o).and_then(|()| app.run("dimension.update", json!({"ids":[id],"options":o})).map(drop));
    if let Err(e) = ran {
        app.ui.status = e;
    }
}

/// The rows of the dialog's body, in the order the sections have always shown them. Returns whether the draft can be
/// saved.
fn dialog_rows(
    ui: &mut Ui,
    form: &mut FormCx<'_, '_>,
    o: &mut DimensionOptions,
    font: &mut FontPickerSettings,
    filters: &mut FontFilters,
    refused: Option<&(Label, DimensionOptions)>,
) -> bool {
    for answer in form.take_answers() {
        if let HostAnswer::Color { key, color, .. } = answer {
            set_color(o, &key, color);
        }
    }
    let mut edited = Edited::default();
    let e = &mut edited;
    GroupBox::new(tl!("Units & Scale"), None).show(ui, |ui, _| {
        Row::new().wrap(true).show(ui, |ui, _| {
            units_row(ui, o, e);
            precision_row(ui, o, e);
        });
        scale_row(ui, o);
        Text::new(tl!("Drawing : real size — 1:10 labels 1 unit on the page as 10.")).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
    });
    GroupBox::new(tl!("Dimension Lines"), None).show(ui, |ui, _| {
        line_row(ui, o, e, Line::Dimension, true);
        Row::new().wrap(true).show(ui, |ui, _| {
            arrow_style_row(ui, o, e);
            number_row(ui, o, e, ARROW_SCALE);
        });
    });
    GroupBox::new(tl!("Extension Lines"), None).show(ui, |ui, _| {
        line_row(ui, o, e, Line::Extension, true);
        Row::new().wrap(true).show(ui, |ui, _| {
            number_row(ui, o, e, EXTENSION_OFFSET);
            number_row(ui, o, e, EXTENSION_LENGTH);
        });
        e.set |= check_row(ui, "Hide Extension Lines", &mut o.hide_extensions);
    });
    GroupBox::new(tl!("Dimension Text"), None).show(ui, |ui, _| {
        font_rows(ui, o, font, filters);
        text_rows(ui, o, e, true);
    });
    if let Some(request) = edited.pick {
        form.requests().request_color(request);
    }
    match read(o) {
        Ok(()) => {
            let sample = o.label(72.0, false).unwrap_or_default();
            Text::new(format!("{} {sample}", i18n::colon(tl!("Preview (1 inch on the page)")))).show(ui);
        }
        Err(reason) => {
            Text::new(reason).show(ui);
        }
    }
    if Button::new(tl!("Reset")).show(ui).clicked() {
        *o = DimensionOptions::default();
    }
    // The host's refusal of the held Save stays under the rows until the values change.
    if let Some((reason, at)) = refused
        && *at == *o
    {
        Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
    }
    read(o).is_ok()
}

/// A labelled dropdown over `choices`, whose values are the serde names of `slot`'s type; whether it changed `slot`.
fn choose<T: Serialize + DeserializeOwned>(ui: &mut Ui, label: &'static str, choices: Choices, slot: &mut T) -> bool {
    let entries: Vec<Entry<String>> = choices.iter().map(|&(value, name)| Entry::new(value.to_owned(), tl!(name).to_owned())).collect();
    let current = serde_json::to_value(&*slot).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    let mut picked = None;
    Field::new(tl!(label)).show(ui, |ui, _| picked = Select::new(tl!(label), Bind::given(current.clone()), &entries).show(ui).value().cloned());
    match picked.and_then(|name| serde_json::from_value(Value::String(name)).ok()) {
        Some(value) => {
            *slot = value;
            true
        }
        None => false,
    }
}

fn check_row(ui: &mut Ui, label: &'static str, value: &mut bool) -> bool {
    let picked = Checkbox::new(Bind::given(*value)).label(tl!(label)).show(ui).value().copied();
    if let Some(on) = picked {
        *value = on;
    }
    picked.is_some()
}

/// A labelled number field over the number `num` names. The value goes into `o` as it is edited and the field's
/// edits go to `e`, for a caller that applies gestures itself.
fn number_row(ui: &mut Ui, o: &mut DimensionOptions, e: &mut Edited, num: Num) {
    let value = *(num.slot)(o);
    let mut edit = None;
    // A field whose visible label repeats another's carries its line in its name, so the label only describes it.
    let role = if num.name == num.label { FieldLabelRole::Names } else { FieldLabelRole::Describes };
    Field::new(tl!(num.label)).side(SideLabel::new().role(role)).show(ui, |ui, _| {
        let field = NumberField::new(tl!(num.name), Bind::given(Some(value))).range(num.min..=num.max).unit(num.unit).fixed_decimals(2);
        edit = Some(field.show(ui).edit);
    });
    if let Some(edit) = edit {
        if let Some(Some(v)) = edit.value().copied() {
            *(num.slot)(o) = v;
        }
        e.numbers.push((num.slot, edit));
    }
}

/// A colour swatch over the colour `tint` names; a click asks for a picker through `e`.
fn color_row(ui: &mut Ui, o: &mut DimensionOptions, e: &mut Edited, tint: Tint) {
    let [r, g, b] = *(tint.slot)(o);
    let swatch = Swatch::new(tl!(tint.name), Srgba::from(Rgba::rgb(r, g, b))).picker(tint.key).show(ui);
    if let Some(request) = swatch.pick_color {
        e.pick = Some(request);
    }
}

fn units_row(ui: &mut Ui, o: &mut DimensionOptions, e: &mut Edited) {
    e.set |= choose(ui, "Units", UNITS, &mut o.units);
}

fn precision_row(ui: &mut Ui, o: &mut DimensionOptions, e: &mut Edited) {
    let entries: Vec<Entry<usize>> = (0..=6).map(|n| Entry::new(n, n.to_string())).collect();
    let mut picked = None;
    Field::new(tl!("Precision"))
        .show(ui, |ui, _| picked = Select::new(tl!("Precision"), Bind::given(o.precision), &entries).show(ui).value().copied());
    if let Some(precision) = picked {
        o.precision = precision;
        e.set = true;
    }
}

/// The drawing:real scale as text with the presets beside it, and Hide Units.
fn scale_row(ui: &mut Ui, o: &mut DimensionOptions) {
    let presets: Vec<Entry<String>> = SCALES.iter().map(|&scale| Entry::new(scale.to_owned(), scale)).collect();
    Row::new().wrap(true).show(ui, |ui, _| {
        Field::new(tl!("Scale")).show(ui, |ui, _| {
            TextField::new(tl!("Scale"), Bind::bound(&mut o.scale)).show(ui);
        });
        if let Some(scale) =
            Select::new(tl!("Scale Presets"), Bind::given(o.scale.clone()), &presets).placeholder(tl!("Presets")).show(ui).value().cloned()
        {
            o.scale = scale;
        }
        check_row(ui, "Hide Units", &mut o.hide_units);
    });
}

/// The weight and colour of `line`, and its type when `full`.
fn line_row(ui: &mut Ui, o: &mut DimensionOptions, e: &mut Edited, line: Line, full: bool) {
    let (weight, tint) = match line {
        Line::Dimension => (LINE_WEIGHT, LINE_TINT),
        Line::Extension => (EXTENSION_WEIGHT, EXTENSION_TINT),
    };
    Row::new().wrap(true).show(ui, |ui, _| {
        number_row(ui, o, e, weight);
        if full {
            e.set |= match line {
                Line::Dimension => choose(ui, "Line Type", LINE_TYPES, &mut o.line_type),
                Line::Extension => choose(ui, "Line Type", LINE_TYPES, &mut o.extension_type),
            };
        }
        color_row(ui, o, e, tint);
    });
}

/// The Arrow Style dropdown: Filled, Open, Tick and None, then the other built-in stroke arrowheads. Each style
/// shows a drawn preview of its arrowhead before its name, in the open list and in the closed field.
fn arrow_style_row(ui: &mut Ui, o: &mut DimensionOptions, e: &mut Edited) {
    let choices = arrow_choices();
    let entries: Vec<Entry<Option<usize>>> =
        choices.iter().enumerate().map(|(at, ch)| Entry::new(Some(at), stroke::builtin_arrow_name(ch.name).to_owned())).collect();
    let drawn = o.arrowhead();
    let shown = choices.iter().position(|ch| ch.drawn() == drawn);
    let mut picked = None;
    Field::new(tl!("Arrow Style")).show(ui, |ui, _| {
        picked = Select::new(tl!("Arrow Style"), Bind::given(shown), &entries)
            .picture(|at, cx, rect| {
                if let Some(ch) = (*at).and_then(|at| choices.get(at)) {
                    stroke::arrow_thumb(cx, rect, ch.drawn().as_ref(), false);
                }
            })
            .show(ui)
            .value()
            .copied();
    });
    if let Some(ch) = picked.flatten().and_then(|at| choices.get(at)) {
        o.arrow_style = ch.style;
        o.arrow_head = ch.head.clone();
        e.set = true;
    }
}

/// The dimension text's family and style.
fn font_rows(ui: &mut Ui, o: &mut DimensionOptions, font: &mut FontPickerSettings, filters: &mut FontFilters) {
    let catalog = FontDb::global().catalog();
    Field::new(tl!("Font Family")).show(ui, |ui, _| {
        let before = o.font_family.clone();
        let picked = FontPicker::new(tl!("Font Family"), Bind::bound(&mut o.font_family), catalog)
            .settings(Bind::bound(&mut *font))
            .filters(Bind::bound(&mut *filters))
            .preview_text("123.45")
            .show(ui)
            .picked_style;
        if o.font_family != before || picked.is_some() {
            o.font_style = picked.unwrap_or_else(|| "Regular".to_owned());
        }
    });
    let styles = FontDb::global().styles(&o.font_family);
    let entries: Vec<Entry<String>> = styles.iter().map(|style| Entry::new(style.clone(), style.clone())).collect();
    Field::new(tl!("Font Style")).show(ui, |ui, _| {
        if let Some(style) = Select::new(tl!("Font Style"), Bind::given(o.font_style.clone()), &entries).show(ui).value().cloned() {
            o.font_style = style;
        }
    });
}

/// The text's size and colour, its position (with its offset when `full`) and its alignment.
fn text_rows(ui: &mut Ui, o: &mut DimensionOptions, e: &mut Edited, full: bool) {
    Row::new().wrap(true).show(ui, |ui, _| {
        number_row(ui, o, e, FONT_SIZE);
        color_row(ui, o, e, TEXT_TINT);
    });
    Row::new().wrap(true).show(ui, |ui, _| {
        e.set |= choose(ui, "Position", POSITIONS, &mut o.text_position);
        if full {
            number_row(ui, o, e, TEXT_OFFSET);
        }
    });
    e.set |= choose(ui, "Alignment", ALIGNMENTS, &mut o.text_alignment);
}

/// The kind of dimension the tool makes, as a row of segments; a change cancels previews and unfinished picks, which
/// reselecting the tool does. The current kind reports nothing when clicked.
fn mode_control(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mode = app.session.tool_options()["mode"].as_str().unwrap_or("linear").to_owned();
    let entries: Vec<Entry<String>> = MODES.iter().map(|&(id, label)| Entry::new(id.to_owned(), tl!(label).to_owned())).collect();
    if let Some(picked) = Segmented::new(tl!("Dimension Type"), Bind::given(mode), &entries).show(ui).value().cloned()
        && let Err(e) = select_mode(app, &picked)
    {
        app.ui.status = e;
    }
}

fn select_mode(app: &mut VectorcraftApp, mode: &str) -> Result<(), String> {
    app.run("tool.select", json!({"tool":"selection"}))?;
    app.run("tool.select", json!({"tool":"dimension"}))?;
    app.run("tool.setOption", json!({"tool":"dimension","values":{"mode":mode}}))?;
    Ok(())
}

fn options_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    if Button::new(tl!("Tool Options…")).show(ui).clicked()
        && let Err(e) = open(app)
    {
        app.ui.status = e;
    }
}

/// The Dimension tool's strip in the Control bar: the kind of dimension, and Tool Options. Tool Options leaves the
/// row first when room is short.
pub fn controls(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    Divider::vertical().show(ui);
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| mode_control(app, ui));
    Row::new().fit_rank(rank::DETAIL).show(ui, |ui, _| options_button(app, ui));
    true
}

/// Always available while measuring, including workspaces with the main Control bar hidden. It floats over the canvas
/// `area` without taking the focus from it.
pub fn floating(app: &mut VectorcraftApp, ui: &mut Ui, area: Rect) {
    if app.session.tool_id() != "dimension" || app.any_dialog_open() {
        return;
    }
    Popover::new(tl!("Dimension Tool"), Bind::given(true))
        .overlay(|_| Overlay::floating().modeless().bounds(area).placed_at(area.min + STRIP_OFFSET))
        .dismiss(Dismiss::new().outside(Outside::Keep))
        .escape(false)
        .show(ui, |ui, _| {
            Text::new(tl!("Dimension Tool")).show(ui);
            Row::new().show(ui, |ui, _| {
                mode_control(app, ui);
                options_button(app, ui);
            });
            let hint = match app.session.tool_options()["mode"].as_str().unwrap_or("linear") {
                "angular" => "Drag out from a corner, or click two straight lines, then click to place.",
                "radial" | "radius" | "diameter" => "Drag out from a circle or arc for its radius, inwards for its diameter.",
                _ => "Drag out from a segment, or click two points, then click to place.",
            };
            Text::new(tl!(hint)).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
        });
}

/// The style of the dimension `id`, if it is one.
fn options_of(app: &VectorcraftApp, id: NodeId) -> Option<DimensionOptions> {
    let dimension = app.session.active()?.doc.node(id)?.extras().dimension.as_deref()?;
    Some(dimension.options.clone())
}

/// The selected dimension, if one is selected alone: its id and style.
pub fn selected(app: &VectorcraftApp) -> Option<(u64, DimensionOptions)> {
    let st = app.session.active()?;
    let [id] = st.selection.objects.as_slice() else { return None };
    Some((id.0, options_of(app, *id)?))
}

/// The Properties panel's Dimension section: the selected dimension's style, Apply to All and Set as Default. Its
/// scale is set in the Tool Options.
pub fn properties_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((id, o)) = selected(app) else { return };
    let mut next = o.clone();
    let mut edited = Edited::default();
    let e = &mut edited;
    crate::panels::section(tl!("Dimension")).show(ui, |ui, _| {
        units_row(ui, &mut next, e);
        precision_row(ui, &mut next, e);
        line_row(ui, &mut next, e, Line::Dimension, false);
        arrow_style_row(ui, &mut next, e);
        crate::panels::section(tl!("Extension Lines")).show(ui, |ui, _| line_row(ui, &mut next, e, Line::Extension, false));
        crate::panels::section(tl!("Dimension Text")).show(ui, |ui, _| text_rows(ui, &mut next, e, false));
    });
    if edited.set {
        update(app, id, &next);
    }
    // A scrub or a wheel run is one undo step: the whole style runs with the number it edits.
    for (slot, response) in &edited.numbers {
        // PLAN: the sheet names `EditPath::Preview { label, cmd }`; widgets-w13's `EditPath::Preview` has no fields, so
        // the closure names the command through `live_run`.
        apply_edits(app, response, EditPath::Preview, |app, value: &Option<f64>, live| {
            let Some(value) = *value else { return };
            let mut style = o.clone();
            *slot(&mut style) = value;
            match read(&style) {
                Ok(()) => live_run(app, "Update Dimensions", "dimension.update", json!({"ids":[id],"options":style}), live),
                Err(e) => app.ui.status = e,
            }
        });
    }
    if let Some(request) = edited.pick
        && let Some(document) = app.session.active().map(|st| (st.uid, st.reverts))
        && let Some(dialogs) = app.dialogs()
    {
        let key = request.key.clone();
        let picker = ColorPickerDialog::from_request(request).swatch_view(super::color_picker::swatch_view(app));
        let picker = dialogs.with(|host| host.open(picker));
        app.object_dialogs.dimension = Some(Pick { id: picker, key, document, target: NodeId(id) });
    }
    Row::new().show(ui, |ui, _| {
        if Button::new(tl!("Apply to All")).show(ui).clicked()
            && let Err(e) = app.run("dimension.applyToAll", json!({"id":id}))
        {
            app.ui.status = e;
        }
        if Button::new(tl!("Set as Default")).show(ui).clicked()
            && let Err(e) = app.run("tool.setOption", json!({"tool":"dimension","values":{"style":o}}))
        {
            app.ui.status = e;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::runtime::env::WindowKey;
    use craft::ui::shell::WindowDialogs;
    use craft::ui::testing::Fixture;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The app in a `Fixture`, after its first frame: the window's dialog host exists from then on.
    fn driven() -> (Rc<RefCell<VectorcraftApp>>, Fixture) {
        let app = Rc::new(RefCell::new(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default())));
        let shown = Rc::clone(&app);
        let mut fx = Fixture::new(move |ui| {
            let mut app = shown.borrow_mut();
            app.tick(ui.ctx());
            app.ui(ui);
        });
        fx.frame(vec![]);
        (app, fx)
    }

    /// Opens the dialog through the tool's command and returns its id.
    fn reopen(app: &Rc<RefCell<VectorcraftApp>>, host: &WindowDialogs) -> DialogId {
        app.borrow_mut().run("tool.options", json!({"tool":"dimension"})).unwrap();
        host.with(|h| h.open_dialogs()).into_iter().find(|d| d.key.as_deref() == Some(KIND)).unwrap().id
    }

    /// Save as the user takes it: the dialog files its answer, the pump sets the style, the dialog closes.
    fn save(app: &Rc<RefCell<VectorcraftApp>>, host: &WindowDialogs, ctx: &egui::Context, id: DialogId) -> bool {
        host.with(|h| h.confirm_by_id(ctx, id));
        pump(&mut app.borrow_mut(), ctx);
        host.with(|h| h.confirm_by_id(ctx, id))
    }

    fn saved_style(app: &Rc<RefCell<VectorcraftApp>>) -> DimensionOptions {
        serde_json::from_value(app.borrow().session.tool_options_of("dimension")["style"].clone()).unwrap()
    }

    #[test]
    fn dimension_properties_picker_keeps_its_source_document() {
        use crate::tests::panel_harness::Panel;

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let mut add_dimension = |line_color: [u8; 3]| {
            app.run("file.new", json!({"width":200,"height":200})).unwrap();
            let created = app.run("dimension.create", json!({"a":[0,0],"b":[72,0],"options":{"lineColor":line_color}})).unwrap();
            let id = created["id"].as_u64().unwrap();
            app.run("select.set", json!({"ids":[id]})).unwrap();
            NodeId(id)
        };
        let source = add_dimension([17, 34, 51]);
        let other = add_dimension([51, 34, 17]);
        assert_eq!(source, other, "each drawing allocates its own object ids");
        app.run("document.activate", json!({"index":0})).unwrap();

        let mut panel = Panel::new(app, properties_section);
        let ctx = panel.ctx().clone();
        let host = WindowDialogs::of(&ctx, &WindowKey::main());
        panel.click("Line Color");
        let picker = panel.app().object_dialogs.dimension.as_ref().unwrap().id;
        host.with(|h| h.set_json_field(&ctx, picker, "color", json!("#123456"))).unwrap();
        panel.app().run("document.activate", json!({"index":1})).unwrap();
        assert!(host.with(|h| h.confirm_by_id(&ctx, picker)));
        pump(&mut panel.app(), &ctx);
        assert_eq!(options_of(&panel.app(), other).unwrap().line_color, [51, 34, 17]);

        panel.app().run("document.activate", json!({"index":0})).unwrap();
        assert_eq!(options_of(&panel.app(), source).unwrap().line_color, [17, 34, 51]);
        panel.click("Line Color");
        let picker = panel.app().object_dialogs.dimension.as_ref().unwrap().id;
        host.with(|h| h.set_json_field(&ctx, picker, "color", json!("#123456"))).unwrap();
        assert!(host.with(|h| h.confirm_by_id(&ctx, picker)));
        pump(&mut panel.app(), &ctx);
        assert_eq!(options_of(&panel.app(), source).unwrap().line_color, [18, 52, 86]);
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(options_of(&panel.app(), source).unwrap().line_color, [17, 34, 51]);
    }

    #[test]
    fn dimension_properties_picker_cannot_edit_a_dimension_restored_by_file_revert() {
        use crate::tests::panel_harness::Panel;

        let dir = vectorcraft_testkit::TempDir::new("dimension-color-revert");
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let made = app.run("dimension.create", json!({"a": [0, 0], "b": [72, 0], "options": {"lineColor": [17, 34, 51]}})).unwrap();
        let dimension = NodeId(made["id"].as_u64().unwrap());
        app.run("file.saveAs", json!({"path": dir.join("Dimension.vectorcraft")})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("select.set", json!({"ids": [dimension.0]})).unwrap();
        let state = app.session.active().unwrap();
        let opening = (state.uid, state.reverts);
        let mut panel = Panel::new(app, properties_section);
        let ctx = panel.ctx().clone();
        let host = WindowDialogs::of(&ctx, &WindowKey::main());
        let choose = |panel: &mut Panel| {
            panel.click("Line Color");
            let id = panel.app().object_dialogs.dimension.as_ref().unwrap().id;
            host.with(|h| h.set_json_field(&ctx, id, "color", json!("#123456"))).unwrap();
            id
        };
        let stale = choose(&mut panel);
        panel.app().run("file.revert", json!({"confirmed": true})).unwrap();
        let (drawing, undo) = {
            let app = panel.app();
            let state = app.session.active().unwrap();
            ((state.uid, state.reverts), state.history.undo.len())
        };
        assert_eq!(drawing, (opening.0, opening.1 + 1));
        assert!(host.with(|h| h.confirm_by_id(&ctx, stale)));
        pump(&mut panel.app(), &ctx);
        assert_eq!(options_of(&panel.app(), dimension).unwrap().line_color, [17, 34, 51]);
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);

        panel.app().run("select.set", json!({"ids": [dimension.0]})).unwrap();
        let fresh = choose(&mut panel);
        assert!(host.with(|h| h.confirm_by_id(&ctx, fresh)));
        pump(&mut panel.app(), &ctx);
        assert_eq!(options_of(&panel.app(), dimension).unwrap().line_color, [18, 52, 86]);
    }

    /// The Arrow Style rule of `ui.dialog.set` and the saved style: a head clears with `arrowStyle`, a built-in head
    /// sets its entry's two values, a name that is no built-in head is refused, and a draft that does not validate
    /// cannot be saved.
    #[test]
    fn dimension_dialog_fields_save_and_cancel() {
        let (app, fx) = driven();
        let ctx = fx.ctx().clone();
        let host = WindowDialogs::of(&ctx, &WindowKey::main());
        let set = |id: DialogId, field: &str, value: Value| host.with(|h| h.set_json_field(&ctx, id, field, value));
        let fields = |id: DialogId| host.with(|h| h.open_dialogs()).into_iter().find(|d| d.id == id).and_then(|d| d.fields).unwrap();

        let id = reopen(&app, &host);
        // The default style has no `arrowHead`, and the dialog still takes one.
        assert!(fields(id).get("arrowHead").is_none());
        set(id, "scale", json!("1:10")).unwrap();
        set(id, "units", json!("pc")).unwrap();
        set(id, "arrowHead", json!("Barbed")).unwrap();
        assert!(save(&app, &host, &ctx, id));
        let style = saved_style(&app);
        assert_eq!((style.arrowhead(), style.scale.as_str(), style.units.suffix()), (Some(Arrowhead::Barbed), "1:10", "pc"));

        let id = reopen(&app, &host);
        set(id, "scale", json!("1:0")).unwrap();
        assert!(!save(&app, &host, &ctx, id), "an invalid scale keeps Save disabled");
        assert!(host.with(|h| h.cancel_by_id(&ctx, id)));
        assert_eq!(saved_style(&app).scale, "1:10");

        // An arrow style set by `ui.dialog.set` replaces the Barbed head, as the menu does.
        let id = reopen(&app, &host);
        set(id, "arrowStyle", json!("none")).unwrap();
        assert!(save(&app, &host, &ctx, id));
        let style = saved_style(&app);
        assert_eq!((style.arrow_style, style.arrowhead()), (ArrowStyle::None, None));

        // A stroke head keeps Filled as the arrow style, and a name that is no built-in head is refused and leaves
        // the arrow style as it was.
        let id = reopen(&app, &host);
        set(id, "arrowStyle", json!("tick")).unwrap();
        assert!(set(id, "arrowHead", json!("Nope")).is_err());
        assert_eq!(fields(id)["arrowStyle"], "tick");
        set(id, "arrowHead", json!("Barbed")).unwrap();
        assert!(save(&app, &host, &ctx, id));
        let style = saved_style(&app);
        assert_eq!((style.arrow_style, style.arrowhead()), (ArrowStyle::Filled, Some(Arrowhead::Barbed)));

        // The head of an older style sets that style and no `arrowHead`, as its menu entry does.
        let id = reopen(&app, &host);
        set(id, "arrowHead", json!("Slash")).unwrap();
        assert_eq!(fields(id)["arrowStyle"], "tick");
        assert!(fields(id).get("arrowHead").is_none());
    }

    /// The Arrow Style list names each arrowhead once, with the four older styles first, and each
    /// stroke arrowhead keeps Filled as its `arrowStyle`.
    #[test]
    fn arrow_style_list_names_each_style_once() {
        let choices = arrow_choices();
        let names: Vec<&str> = choices.iter().map(|c| c.name).collect();
        assert_eq!(names[..4], ["Filled", "Open", "Tick", "None"]);
        for name in ["Triangle", "Triangle (open)", "Slash"] {
            assert!(!names.contains(&name), "{name} is listed as Filled, Open or Tick");
        }
        assert_eq!(names.len(), Arrowhead::ALL.len() + 1, "None and each built-in head: {names:?}");
        let drawn: Vec<_> = choices.iter().map(|c| format!("{:?}", c.drawn())).collect();
        assert_eq!(drawn.iter().collect::<std::collections::HashSet<_>>().len(), drawn.len(), "{drawn:?}");
        assert!(choices[4..].iter().all(|c| c.style == ArrowStyle::Filled && c.head.is_some()));
    }
}
