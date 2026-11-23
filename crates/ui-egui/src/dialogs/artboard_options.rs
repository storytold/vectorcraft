//! Artboard Options (double-click with the Artboard tool, double-click the tool, or the Artboards
//! panel's options icon or menu): the artboard's fields, sizes parsed with units, its position
//! from a reference point, its background color (`color`: a color as the paint commands take
//! it, or null for none), its guide label (`guideLabel`: the color its own guides are drawn in, a
//! layer color's name or `#rrggbb`, or null for none), Show Center Mark and Show Cross Hairs (`showCenterMark`,
//! `showCrossHairs`), the display marks the canvas draws on the artboard
//! (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/modify-display-settings-of-artboards.html),
//! and its video display options, Show Video Safe Areas and Video Ruler Pixel Aspect Ratio
//! (`showVideoSafeAreas`, `videoRulerPixelAspectRatio`)
//! (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/set-video-display-options-for-artboards.html).
//! Under Global, Fade Region Outside Artboard and Update While Dragging set the application
//! preferences `fadeRegionOutsideArtboard` and `artboardUpdateWhileDragging` for every document
//! (https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf, Artboard options).
//! Sizing choices also go to the other selected artboards; fitting changes only the active
//! artboard around stationary artwork.
//!
//! The dialog lives on the window's dialog host. OK files the value as a held answer; [`pump`] runs
//! the artboard command and answers it, so a refused command keeps the dialog open with the reason.

use std::cell::Cell;

use craft::color_ui::ui::{ChosenColor, ColorChoice, NamedColor};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::patterns::form_dialogs::HostRequests; // PLAN: assumed re-export of the host requests the form output carries
use craft::ui::patterns::form_dialogs::{FormCx, FormDialog, ResetFooter, ResetPick};
use craft::ui::pickers::{Anchor9, ReferencePoint, ReferencePointLook};
use craft::ui::runtime::env::{Label, LengthUnits};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::requests::ColorRequest;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::{LAYER_COLORS, LayerColor, VIDEO_PIXEL_ASPECT_RANGE};
use vectorcraft_engine::cmd::color_value;
use vectorcraft_engine::cmd::newdoc::DocSettings;
use vectorcraft_geom::{Point, Rect};

use crate::VectorcraftApp;
use crate::panels::artboards::{FILLS, fill_colors};
// PLAN: op.A10 must derive `Debug, PartialEq` on `ArtboardSizeChoice` (the value is compared by the form).
use crate::panels::properties::{ArtboardSizeChoice, artboard_presets, artboard_size_controls};
use crate::panels::transform::artboard_rect;

/// The field of the guide label, as `artboard.setProps` and `artboard.new` take it: a layer
/// color's name, `#rrggbb`, or null for none.
const GUIDE_LABEL: &str = "guideLabel";
/// The key of the Background Color row's custom-color request.
const BACKGROUND: &str = "background";
/// The preference and field of Fade Region Outside Artboard: while the Artboard tool is active,
/// the canvas draws everything outside the artboards darker.
const FADE: &str = "fadeRegionOutsideArtboard";
/// The preference and field of Update While Dragging: the area outside the artboards stays darker
/// while the Artboard tool moves or resizes an artboard.
const UPDATE_WHILE_DRAGGING: &str = "artboardUpdateWhileDragging";
/// The field of the Video Ruler Pixel Aspect Ratio, as `artboard.setProps` takes it.
const PIXEL_ASPECT: &str = "videoRulerPixelAspectRatio";

/// The dialog kind of Artboard Options, the value of the `dialog` key of the reply to opening it.
pub const KIND: &str = "artboardOptions";

/// What the dialog edits: the artboard's fields, the draft of the size choice and the reference point
/// the last Width or Height change used. It is also the dialog's answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtboardOptionsValue {
    /// Whether this is the dialog of a new artboard (`ui.newArtboard`).
    new: bool,
    /// The artboard being edited.
    index: usize,
    name: String,
    /// The artboard's top-left corner and size, in document points.
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    /// The background color as the paint commands take it; null for none.
    color: Value,
    /// The last custom background color the dialog knows, for the list's Custom row.
    custom_background: Option<Srgba>,
    /// The guide label's text (a layer color's name or `#rrggbb`); `None` for none.
    guide_label: Option<String>,
    /// The last custom guide color the dialog knows, for the list's Custom row.
    custom_guide: Option<Srgba>,
    show_center_mark: bool,
    show_cross_hairs: bool,
    show_video_safe_areas: bool,
    pixel_aspect: f64,
    fade: bool,
    update_while_dragging: bool,
    /// The reference point, an index into the nine points row by row.
    reference: usize,
    /// The column and row of the reference point of the last Width or Height change, which the other
    /// selected artboards resize about.
    ref_x: usize,
    ref_y: usize,
    /// A Fit was chosen: OK then moves and scales no art.
    fit: bool,
    /// The preset or orientation chosen, which applies to each selected artboard in its own orientation.
    size_choice: Option<ArtboardSizeChoice>,
}

impl ArtboardOptionsValue {
    /// Where the fields put the artboard.
    fn placed(&self) -> Rect {
        Rect::new(self.x, self.y, self.x + self.width, self.y + self.height)
    }

    /// Stores document-coordinate draft bounds, retaining an editable minimum for zero-area art.
    fn set_rect(&mut self, r: Rect) {
        (self.x, self.y, self.width, self.height) = (r.x0, r.y0, r.width().max(1.0), r.height().max(1.0));
    }

    /// Remembers the artboard's own custom colors for the lists' Custom rows: a background that is no
    /// entry of `fills` and a guide label that is a `#rrggbb` color.
    fn seed_customs(&mut self, fills: &[Option<Color>; 3]) {
        self.custom_background = color_value(&self.color).filter(|_| fill_index(fills, &self.color).is_none()).map(|c| swatch(&c));
        self.custom_guide = match self.guide_label.as_deref().and_then(LayerColor::from_label_text) {
            Some(LayerColor::Custom(rgb)) => Some(srgba(rgb)),
            _ => None,
        };
    }

    /// The parameters of `artboard.setProps` (and the fields of `artboard.new`) for the fields.
    fn params(&self) -> Value {
        json!({
            "index": self.index, "name": self.name, "x": self.x, "y": self.y, "width": self.width, "height": self.height,
            "color": self.color, GUIDE_LABEL: self.guide_label,
            "showCenterMark": self.show_center_mark, "showCrossHairs": self.show_cross_hairs,
            "showVideoSafeAreas": self.show_video_safe_areas, PIXEL_ASPECT: self.pixel_aspect,
        })
    }
}

/// What the dialog shows but does not edit; read from the app when it opens, since a modal dialog's
/// document, presets and preferences cannot change while it is open.
struct Setup {
    /// The lengths the fields keep and show.
    lengths: LengthUnits,
    /// The ruler origin X and Y count from.
    origin: Point,
    /// The size presets of the Preset menu.
    presets: Vec<(&'static str, Vec<DocSettings>)>,
    /// The background colors [`FILLS`] stand for, in the document's color mode.
    fills: [Option<Color>; 3],
    /// The bounds Fit to Artwork Bounds and Fit to Selected Art fit to; `None` without art or selection.
    fit_bounds: [Option<Rect>; 2],
}

/// Artboard Options on the dialog host: the artboard's value, the held OK and the color requests.
pub struct ArtboardOptionsDialog {
    id: DialogId,
    value: ArtboardOptionsValue,
    setup: Setup,
    held: HeldAnswer<ArtboardOptionsValue>,
    requests: HostRequests,
}

impl ArtboardOptionsDialog {
    fn new(value: ArtboardOptionsValue, setup: Setup) -> Self {
        ArtboardOptionsDialog { id: dialog_id(KIND), value, setup, held: HeldAnswer::new(), requests: HostRequests::new() }
    }
}

impl StackDialog for ArtboardOptionsDialog {
    type Result = ArtboardOptionsValue;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ArtboardOptionsValue>> {
        let finish = self.held.is_accepted();
        let footer = ResetFooter::new(ui, None, true);
        let refusal = self.held.refusal().map(|(reason, at)| (reason.as_str().to_string(), at.clone()));
        let setup = &self.setup;
        let latest = Cell::new(None);
        let shown = FormDialog::new(tl!("Artboard Options"), Bind::bound(&mut self.value))
            .accept_stays_open(true)
            .ok_enabled(!self.held.is_waiting())
            .show(ui, |ui, cx, v| {
                // A measuring run closes nothing: the real run of the pass makes the close.
                if finish && !craft::ui::runtime::measure::active(ui.ctx()) {
                    cx.request_close_as(CloseReason::Activated);
                }
                body(ui, cx, v, setup);
                if let Some((reason, at)) = refusal.as_ref().filter(|(_, at)| at == v) {
                    let _ = at;
                    Text::new(reason.clone()).tone(Tone::Bad).show(ui);
                }
                latest.set(Some(v.clone()));
                true
            });
        self.requests.extend(shown.requests);
        if matches!(footer.pick(shown.picked), Some(ResetPick::Accept))
            && let Some(value) = latest.take()
        {
            self.held.file(value);
        }
        if self.held.refusal().is_some_and(|(_, at)| *at != self.value) {
            self.held.clear_refusal();
        }
        match shown.closed {
            Some(CloseReason::Activated) if finish => Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
            Some(_) => Some(DialogResult::Cancel),
            None => None,
        }
    }

    /// Files the value as OK does; once [`pump`] has run the command and the app accepted it, answers it.
    fn confirm(&mut self, _: &egui::Context) -> Option<ArtboardOptionsValue> {
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

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    /// A custom color is RGB, and becomes the row's choice.
    fn answer_color(&mut self, key: &str, color: Srgba) {
        let v = &mut self.value;
        match key {
            BACKGROUND => {
                v.custom_background = Some(color);
                v.color = custom_background_json(color);
            }
            GUIDE_LABEL => {
                v.custom_guide = Some(color);
                v.guide_label = Some(LayerColor::Custom(rgb8(color)).label_text());
            }
            _ => {}
        }
    }

    fn json_fields(&self) -> Option<Value> {
        let v = &self.value;
        let mut fields = v.params();
        for (key, on) in [(FADE, v.fade), (UPDATE_WHILE_DRAGGING, v.update_while_dragging), ("new", v.new)] {
            fields[key] = json!(on);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let number = || value.as_f64().filter(|n| n.is_finite()).ok_or_else(rejects);
        let on = || value.as_bool().ok_or_else(rejects);
        let v = &mut self.value;
        match field {
            "name" => v.name = value.as_str().ok_or_else(rejects)?.to_string(),
            "x" => v.x = number()?,
            "y" => v.y = number()?,
            "width" => v.width = number()?,
            "height" => v.height = number()?,
            "color" => {
                if !value.is_null() && color_value(&value).is_none() {
                    return Err(rejects());
                }
                v.color = value.clone();
            }
            GUIDE_LABEL => v.guide_label = if value.is_null() { None } else { Some(value.as_str().ok_or_else(rejects)?.to_string()) },
            "showCenterMark" => v.show_center_mark = on()?,
            "showCrossHairs" => v.show_cross_hairs = on()?,
            "showVideoSafeAreas" => v.show_video_safe_areas = on()?,
            PIXEL_ASPECT => {
                let n = number()?;
                if !VIDEO_PIXEL_ASPECT_RANGE.contains(&n) {
                    return Err(rejects());
                }
                v.pixel_aspect = n;
            }
            FADE => v.fade = on()?,
            UPDATE_WHILE_DRAGGING => v.update_while_dragging = on()?,
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// The index into `fills` of the entry that is the background `color` (null for none).
fn fill_index(fills: &[Option<Color>; 3], color: &Value) -> Option<usize> {
    fills.iter().position(|fill| fill_json(fill) == *color)
}

/// A background color as the fields keep it.
fn fill_json(fill: &Option<Color>) -> Value {
    fill.as_ref().map_or(Value::Null, crate::panels::color_json)
}

/// The Srgba of an 8-bit color.
fn srgba([r, g, b]: [u8; 3]) -> Srgba {
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// The 8-bit color nearest `c`.
fn rgb8(c: Srgba) -> [u8; 3] {
    [c.r, c.g, c.b].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The swatch a color shows in a list.
fn swatch(c: &Color) -> Srgba {
    let [r, g, b, _] = c.to_rgba8(1.0);
    srgba([r, g, b])
}

/// A picked custom color as the background field keeps it.
fn custom_background_json(c: Srgba) -> Value {
    crate::panels::color_json(&Color::rgb(c.r, c.g, c.b))
}

/// The nine points are row-major from the top left, as `Anchor9::ALL` and the reference index are.
fn anchor(index: usize) -> Anchor9 {
    Anchor9::ALL.get(index).copied().unwrap_or(Anchor9::Centre)
}

/// The reference index of `anchor`.
fn reference_of(anchor: Anchor9) -> usize {
    let (column, row) = anchor.cell();
    row * 3 + column
}

/// A length row: the value in the unit shown; `Some` with the typed length when it was edited.
fn length_row(ui: &mut Ui, label: impl Into<Label>, salt: &str, value: f64, lengths: LengthUnits) -> Option<f64> {
    let label = label.into();
    let mut typed = None;
    Field::new(label.clone()).id_salt(salt).show(ui, |ui, _| {
        typed = NumberField::length(label.clone(), Bind::given(Some(value)), lengths).id_salt(salt).show(ui).value().copied().flatten();
    });
    typed
}

/// A check box row.
fn check(ui: &mut Ui, label: impl Into<Label>, salt: &str, on: &mut bool) {
    if let Some(now) = Checkbox::new(Bind::given(*on)).label(label).id_salt(salt).show(ui).value().copied() {
        *on = now;
    }
}

/// A color list row: `named` entries and the custom color; returns the row chosen this pass. "Select
/// Custom Color…" files a request under `key` for the app's color picker.
fn color_row(
    ui: &mut Ui,
    cx: &mut FormCx<'_, '_>,
    label: impl Into<Label>,
    key: &'static str,
    chosen: &ChosenColor,
    named: &[NamedColor],
    custom: Option<Srgba>,
) -> Option<ChosenColor> {
    let label = label.into();
    let mut picked = None;
    Field::new(label.clone()).id_salt(key).show(ui, |ui, _| {
        let out = ColorChoice::new(label.clone(), chosen).named(named.iter().cloned()).custom(custom).show(ui);
        picked = out.chosen;
        if let Some(mut request) = out.pick_color {
            request.key = key.into();
            cx.requests().request_color(request);
        }
    });
    picked
}

/// The form: Name, Width and Height, the reference point with X and Y, Background Color, Guide Label,
/// the marks, the Video Ruler Pixel Aspect Ratio and, under Global, the two preferences. The reference
/// point is the one the Transform panel shares (#1876); X and Y are where it lies on the rulers, counted
/// from the ruler origin (#1212), and a new Width or Height keeps it where it was.
fn body(ui: &mut Ui, cx: &mut FormCx<'_, '_>, v: &mut ArtboardOptionsValue, s: &Setup) {
    let lengths = s.lengths;
    let (was, w0, h0) = (v.placed(), v.width, v.height);
    let (choice, fit) = artboard_size_controls(ui, &s.presets, w0, h0, Some(s.fit_bounds.map(|b| b.is_some())));
    let mut refi = v.reference;
    let picked = choice.is_some();
    if let Some(choice) = choice {
        if let Some((w, h)) = choice.size_for(w0, h0) {
            v.set_rect(artboard_rect(was, refi, None, None, Some(w), Some(h), false));
        }
        v.size_choice = Some(choice);
        (v.ref_x, v.ref_y, v.fit) = (refi % 3, refi / 3, false);
    }
    ui.add_space(8.0);
    let name = Label::from(tl!("Name:"));
    Field::new(name.clone()).id_salt("name").show(ui, |ui, _| {
        TextField::new(name.clone(), Bind::bound(&mut v.name)).show(ui);
    });
    if let Some(w) = length_row(ui, tl!("Width:"), "width", v.width, lengths) {
        v.width = w;
    }
    if let Some(h) = length_row(ui, tl!("Height:"), "height", v.height, lengths) {
        v.height = h;
    }
    let (w, h) = (v.width, v.height);
    if !picked && (w != w0 || h != h0) {
        v.size_choice = None;
    }
    // Only the side that changed moves, so an untouched coordinate stays exactly as it was.
    let n = artboard_rect(was, refi, None, None, Some(w), Some(h), false);
    if w != w0 {
        (v.x, v.ref_x) = (n.x0, refi % 3);
    }
    if h != h0 {
        (v.y, v.ref_y) = (n.y0, refi / 3);
    }
    let reference = Label::from(tl!("Reference Point:"));
    Field::new(reference.clone()).id_salt("reference").show(ui, |ui, _| {
        let pick = ReferencePoint::new(reference.clone(), Bind::given(Some(anchor(refi)))).look(ReferencePointLook::Squares).show(ui);
        if let Some(a) = pick.value().copied().flatten() {
            refi = reference_of(a);
        }
    });
    v.reference = refi;
    let (r, at) = (v.placed(), vectorcraft_geom::reference_point(v.placed(), refi));
    if let Some(typed) = length_row(ui, tl!("X:"), "x", at.x - s.origin.x, lengths) {
        v.x = artboard_rect(r, refi, Some(typed + s.origin.x), None, None, None, false).x0;
    }
    if let Some(typed) = length_row(ui, tl!("Y:"), "y", at.y - s.origin.y, lengths) {
        v.y = artboard_rect(r, refi, None, Some(typed + s.origin.y), None, None, false).y0;
    }
    // Apply a Fit after the ordinary size and reference-point rows: they must not replace the chosen
    // artwork's position with one derived from the previous artboard rectangle.
    if let Some(selection) = fit
        && let Some(bounds) = s.fit_bounds[usize::from(selection)]
    {
        v.set_rect(bounds);
        (v.fit, v.size_choice) = (true, None);
    }
    ui.add_space(8.0);
    background_row(ui, cx, v, s);
    guide_label_row(ui, cx, v);
    ui.add_space(8.0);
    check(ui, tl!("Show Center Mark"), "showCenterMark", &mut v.show_center_mark);
    check(ui, tl!("Show Cross Hairs"), "showCrossHairs", &mut v.show_cross_hairs);
    check(ui, tl!("Show Video Safe Areas"), "showVideoSafeAreas", &mut v.show_video_safe_areas);
    let ratio = Label::from(tl!("Video Ruler Pixel Aspect Ratio:"));
    Field::new(ratio.clone()).id_salt(PIXEL_ASPECT).show(ui, |ui, _| {
        let typed = NumberField::new(ratio.clone(), Bind::given(Some(v.pixel_aspect)))
            .range(VIDEO_PIXEL_ASPECT_RANGE)
            .fixed_decimals(4)
            .id_salt(PIXEL_ASPECT)
            .show(ui)
            .value()
            .copied()
            .flatten();
        if let Some(n) = typed {
            v.pixel_aspect = n;
        }
    });
    ui.add_space(8.0);
    Text::new(tl!("Global")).show(ui);
    check(ui, tl!("Fade Region Outside Artboard"), FADE, &mut v.fade);
    check(ui, tl!("Update While Dragging"), UPDATE_WHILE_DRAGGING, &mut v.update_while_dragging);
}

/// Background Color: None, White and Black (in the document's color mode), Custom and Select Custom Color….
fn background_row(ui: &mut Ui, cx: &mut FormCx<'_, '_>, v: &mut ArtboardOptionsValue, s: &Setup) {
    let named: Vec<NamedColor> = FILLS
        .iter()
        .zip(&s.fills)
        .enumerate()
        .map(|(k, (name, fill))| NamedColor::new(k.to_string(), tl!(*name), fill.as_ref().map(swatch)))
        .collect();
    let chosen = fill_index(&s.fills, &v.color).map_or(ChosenColor::Custom, |k| ChosenColor::Named(k.to_string()));
    match color_row(ui, cx, tl!("Background Color:"), BACKGROUND, &chosen, &named, v.custom_background) {
        Some(ChosenColor::Named(id)) => {
            if let Some(fill) = id.parse::<usize>().ok().and_then(|k| s.fills.get(k)) {
                v.color = fill_json(fill);
            }
        }
        Some(ChosenColor::Custom) => {
            if let Some(c) = v.custom_background {
                v.color = custom_background_json(c);
            }
        }
        _ => {}
    }
}

/// Guide Label: None, the layer colors, Custom and Select Custom Color….
fn guide_label_row(ui: &mut Ui, cx: &mut FormCx<'_, '_>, v: &mut ArtboardOptionsValue) {
    let named: Vec<NamedColor> = std::iter::once(NamedColor::new("none", tl!("None"), None))
        .chain(LAYER_COLORS.iter().enumerate().map(|(i, (name, rgb))| NamedColor::new(i.to_string(), tl!(*name), Some(srgba(*rgb)))))
        .collect();
    let chosen = match v.guide_label.as_deref().and_then(LayerColor::from_label_text) {
        None => ChosenColor::Named("none".into()),
        Some(LayerColor::Preset(i)) => ChosenColor::Named(i.to_string()),
        Some(LayerColor::Custom(_)) => ChosenColor::Custom,
    };
    match color_row(ui, cx, tl!("Guide Label:"), GUIDE_LABEL, &chosen, &named, v.custom_guide) {
        Some(ChosenColor::Named(id)) => v.guide_label = id.parse::<u8>().ok().map(|i| LayerColor::Preset(i).label_text()),
        Some(ChosenColor::Custom) => {
            if let Some(c) = v.custom_guide {
                v.guide_label = Some(LayerColor::Custom(rgb8(c)).label_text());
            }
        }
        _ => {}
    }
}

/// `fields` with the Global checkboxes' values added.
fn global_fields(app: &VectorcraftApp) -> [(&'static str, bool); 2] {
    let prefs = &app.session.prefs;
    [(FADE, prefs.fade_region_outside_artboard), (UPDATE_WHILE_DRAGGING, prefs.artboard_update_while_dragging)]
}

/// The Global checkboxes' preferences that the form changed.
fn global_changes(app: &VectorcraftApp, v: &ArtboardOptionsValue) -> serde_json::Map<String, Value> {
    let shown = [(FADE, v.fade), (UPDATE_WHILE_DRAGGING, v.update_while_dragging)];
    global_fields(app).into_iter().zip(shown).filter(|((_, now), (_, on))| now != on).map(|(_, (key, on))| (key.to_string(), json!(on))).collect()
}

/// OK sets the Global checkboxes' preferences that the form changed ([`global_changes`]), in one
/// `prefs.set`, and leaves the preferences alone when none changed.
fn set_globals(app: &mut VectorcraftApp, values: serde_json::Map<String, Value>) -> Result<(), String> {
    if values.is_empty() {
        return Ok(());
    }
    app.run("prefs.set", json!({ "values": values })).map(drop)
}

/// The active artboard's index: the Artboard tool's while it is in use, else the window's.
pub(crate) fn active_index(app: &VectorcraftApp) -> usize {
    if app.session.tool_id() == "artboard" {
        app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0)
    } else {
        app.view().map_or(0, |v| v.artboard)
    }
}

/// Opens the dialog over `value`, with the preferences, the shared reference point and what the dialog
/// shows read from the app.
fn present(app: &mut VectorcraftApp, mut value: ArtboardOptionsValue) -> Result<Value, String> {
    let host = app.dialogs().ok_or("no window")?;
    let [(_, fade), (_, update_while_dragging)] = global_fields(app);
    (value.fade, value.update_while_dragging, value.reference) = (fade, update_while_dragging, app.ui.transform_panel.reference_point);
    let fills = fill_colors(app);
    value.seed_customs(&fills);
    let artwork = app.session.active().and_then(|st| vectorcraft_render::encode::art_bounds(&st.doc));
    let setup = Setup {
        lengths: crate::canvas::rulers::general_lengths(app),
        origin: app.ruler_origin(),
        presets: artboard_presets(app),
        fills,
        fit_bounds: [artwork, app.selection_bounds()],
    };
    host.with(|h| h.open(ArtboardOptionsDialog::new(value, setup)));
    Ok(json!({ "dialog": KIND }))
}

/// Open Artboard Options for the active artboard ([`active_index`]).
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let index = active_index(app);
    let st = app.session.active().ok_or("no document")?;
    let a = st.doc.artboards.get(index).ok_or("no artboard")?;
    let r = a.rect;
    let value = ArtboardOptionsValue {
        index,
        name: a.name.clone(),
        x: r.x0,
        y: r.y0,
        width: r.width(),
        height: r.height(),
        color: a.color.as_ref().map_or(Value::Null, crate::panels::color_json),
        guide_label: a.guide_label.map(LayerColor::label_text),
        show_center_mark: a.show_center_mark,
        show_cross_hairs: a.show_cross_hairs,
        show_video_safe_areas: a.show_video_safe_areas,
        pixel_aspect: a.video_ruler_pixel_aspect_ratio,
        ..Default::default()
    };
    present(app, value)
}

/// `ui.newArtboard` (Alt-click on the Artboards panel's New Artboard): Artboard Options for a new
/// artboard, filled in as `artboard.new {}` would make it: named Artboard N (N one more than the
/// number of artboards), as large as the last artboard and 20 pt to the right of it.
pub fn open_new(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let abs = &st.doc.artboards;
    let last = abs.last().map_or(Rect::new(0.0, 0.0, 612.0, 792.0), |a| a.rect);
    let value = ArtboardOptionsValue {
        new: true,
        name: format!("Artboard {}", abs.len() + 1),
        x: last.x1 + 20.0,
        y: last.y0,
        width: last.width(),
        height: last.height(),
        pixel_aspect: 1.0,
        ..Default::default()
    };
    present(app, value)
}

/// Runs once per frame after the shell pass: runs the artboard command of a filed OK and answers it
/// (a refusal keeps the dialog open with its reason), and gives the Transform panel the reference
/// point picked in the dialog.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(host) = app.dialogs() else { return };
    let Some((held, reference)) = host.with(|h| h.get_mut::<ArtboardOptionsDialog>().map(|d| (d.held.take(), d.value.reference))) else { return };
    if app.ui.transform_panel.reference_point != reference {
        app.ui.transform_panel.reference_point = reference;
    }
    if let Some(value) = held {
        let answer = confirm(app, &value).map(drop).map_err(Label::from);
        host.with(|h| {
            if let Some(d) = h.get_mut::<ArtboardOptionsDialog>() {
                d.held.answer(answer);
            }
        });
    }
}

/// OK sets the artboard's fields. A preset or orientation choice applies separately to each
/// selected artboard; a typed Width or Height that differs also goes to the others (when this
/// one is among them), each resized about the reference point the form's last Width or Height change
/// used (its column and row), all in one undo step. Fitting applies only to this artboard and never
/// moves its artwork. The Global checkboxes' preferences are set only after the artboard command
/// succeeds, so an error leaves them as they were.
fn confirm(app: &mut VectorcraftApp, v: &ArtboardOptionsValue) -> Result<Value, String> {
    let globals = global_changes(app, v);
    if v.new {
        let r = confirm_new(app, v)?;
        set_globals(app, globals)?;
        return Ok(r);
    }
    let p = v.params();
    let was = app.session.active().and_then(|st| st.doc.artboards.get(v.index)).map(|a| a.rect);
    let changed = |k: &str, old: Option<f64>| p[k].as_f64().filter(|n| old.is_some_and(|old| (n - old).abs() > 1e-6));
    let (w, h) = (changed("width", was.map(|r| r.width())), changed("height", was.map(|r| r.height())));
    let refi = v.ref_y.min(2) * 3 + v.ref_x.min(2);
    let mut entries = vec![];
    if !v.fit && (v.size_choice.is_some() || w.is_some() || h.is_some()) {
        for i in crate::panels::artboards::resized_with(app, v.index).into_iter().filter(|i| *i != v.index) {
            if let Some(r) = app.session.active().and_then(|st| st.doc.artboards.get(i)).map(|a| a.rect) {
                let (w, h) = if let Some(choice) = v.size_choice {
                    let Some((w, h)) = choice.size_for(r.width(), r.height()) else { continue };
                    (Some(w), Some(h))
                } else {
                    (w, h)
                };
                let n = artboard_rect(r, refi, None, None, w, h, false);
                entries.push(json!({"index": i, "x": n.x0, "y": n.y0, "width": n.width(), "height": n.height()}));
            }
        }
    }
    entries.insert(0, p);
    // Fitting changes the artboard around existing art; it must neither move nor scale that art.
    let common = json!({
        "scaleArt": !v.fit && crate::panels::artboards::scale_art(app),
        "moveArt": !v.fit && crate::panels::artboards::move_art(app),
    });
    let p = crate::panels::artboards::set_props_params(common, entries).ok_or("no artboard")?;
    let r = app.run("artboard.setProps", p)?;
    set_globals(app, globals)?;
    Ok(r)
}

/// OK for a new artboard: `artboard.new` with the fields (an empty name gives the default one),
/// the background color and guide label picked, the display marks ticked and the Video Ruler Pixel Aspect Ratio
/// typed, and the new artboard becomes the active one.
fn confirm_new(app: &mut VectorcraftApp, v: &ArtboardOptionsValue) -> Result<Value, String> {
    let mut p = json!({"x": v.x, "y": v.y, "width": v.width, "height": v.height, PIXEL_ASPECT: v.pixel_aspect});
    if !v.name.trim().is_empty() {
        p["name"] = json!(v.name);
    }
    if !v.color.is_null() {
        p["color"] = v.color.clone();
    }
    if let Some(label) = &v.guide_label {
        p[GUIDE_LABEL] = json!(label);
    }
    for (key, on) in [("showCenterMark", v.show_center_mark), ("showCrossHairs", v.show_cross_hairs), ("showVideoSafeAreas", v.show_video_safe_areas)]
    {
        if on {
            p[key] = json!(true);
        }
    }
    let r = app.run("artboard.new", p)?;
    if let Some(i) = r["index"].as_u64().and_then(|i| usize::try_from(i).ok()) {
        crate::panels::artboards::select(app, i);
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// An app with one 200 × 150 artboard on a window with a dialog host.
    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app
    }

    fn id() -> DialogId {
        dialog_id(KIND)
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|h| h.get::<ArtboardOptionsDialog>().is_some())
    }

    /// Sets fields as `ui.dialog.set` does, all or none.
    fn set(app: &VectorcraftApp, fields: &[(&str, Value)]) -> Result<(), String> {
        let fields: Vec<(String, Value)> = fields.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect();
        app.dialogs().unwrap().with(|h| h.set_json_fields(&crate::testing::ui_context(), id(), &fields))
    }

    /// Changes the value as the form's controls do (the parts no field names).
    fn edit(app: &VectorcraftApp, change: impl FnOnce(&mut ArtboardOptionsValue)) {
        app.dialogs().unwrap().with(|h| change(&mut h.get_mut::<ArtboardOptionsDialog>().unwrap().value));
    }

    /// Presses OK: the host files the value, `pump` runs the command, and the host closes the dialog
    /// when the command succeeded.
    fn ok(app: &mut VectorcraftApp) -> Result<(), String> {
        let (ctx, host) = (crate::testing::ui_context(), app.dialogs().unwrap());
        host.with(|h| h.confirm_by_id(&ctx, id()));
        pump(app, &ctx);
        match host.with(|h| h.held_state(id())) {
            Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_string()),
            _ => {
                host.with(|h| h.confirm_by_id(&ctx, id()));
                Ok(())
            }
        }
    }

    fn boards(app: &VectorcraftApp) -> Vec<Rect> {
        app.session.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect()
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// A new X moves the art only with Move Artwork with Artboard on, and Undo restores the document.
    #[test]
    fn position_moves_art_only_with_the_tool_option_on() {
        for (tool_in_use, move_art) in [(true, false), (true, true), (false, false), (false, true)] {
            let mut app = app();
            let id = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
            app.run("tool.setOption", json!({"tool": "artboard", "key": "moveArt", "value": move_art})).unwrap();
            if tool_in_use {
                app.select_tool("artboard");
            }
            open(&mut app).unwrap();
            set(&app, &[("x", json!(50))]).unwrap();
            let before = app.session.doc().unwrap().doc.clone();
            ok(&mut app).unwrap();
            assert_eq!(boards(&app)[0].x0, 50.0);
            let art = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0;
            assert_eq!(art, if move_art { 60.0 } else { 10.0 }, "tool in use {tool_in_use}");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(*app.session.doc().unwrap().doc, *before);
        }
    }

    /// Each field family reaches `artboard.setProps` in one undo step; a bad guide label name is
    /// refused with the dialog kept open.
    #[test]
    fn fields_reach_their_params() {
        let mut app = app();
        let board = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards[0].clone();
        app.run("artboard.setProps", json!({"index": 0, "color": "#336699"})).unwrap();
        open(&mut app).unwrap();
        set(&app, &[("name", json!("Card"))]).unwrap();
        ok(&mut app).unwrap();
        let ab = board(&app);
        assert_eq!((ab.name.as_str(), ab.color.map(|c| c.to_hex()).as_deref()), ("Card", Some("#336699")), "the color is kept");
        open(&mut app).unwrap();
        set(&app, &[("color", Value::Null)]).unwrap();
        ok(&mut app).unwrap();
        assert!(board(&app).color.is_none());
        for (text, want) in [("Orange", "Orange"), ("#123456", "#123456")] {
            let undo = undo_len(&app);
            open(&mut app).unwrap();
            set(&app, &[(GUIDE_LABEL, json!(text))]).unwrap();
            ok(&mut app).unwrap();
            assert_eq!((board(&app).guide_label.map(LayerColor::label_text).as_deref(), undo_len(&app)), (Some(want), undo + 1));
        }
        open(&mut app).unwrap();
        set(&app, &[(GUIDE_LABEL, json!("purple"))]).unwrap();
        assert!(ok(&mut app).is_err() && is_open(&app), "another letter case is refused and the dialog stays");
        assert_eq!(board(&app).guide_label.map(LayerColor::label_text).as_deref(), Some("#123456"));
        set(&app, &[(GUIDE_LABEL, Value::Null)]).unwrap();
        ok(&mut app).unwrap();
        assert!(board(&app).guide_label.is_none());
        open(&mut app).unwrap();
        set(&app, &[("showCenterMark", json!(true)), ("showCrossHairs", json!(true))]).unwrap();
        ok(&mut app).unwrap();
        let ab = board(&app);
        assert_eq!((ab.show_center_mark, ab.show_cross_hairs), (true, true));
    }

    /// A name that is no field, and a value of the wrong type, are refused and change nothing.
    #[test]
    fn unknown_fields_and_wrong_types_are_refused() {
        let mut app = app();
        open(&mut app).unwrap();
        assert!(set(&app, &[("__fit", json!(true))]).is_err());
        assert!(set(&app, &[("name", json!("A")), ("width", json!("wide"))]).is_err());
        assert_eq!(app.dialogs().unwrap().with(|h| h.get::<ArtboardOptionsDialog>().unwrap().value.name.clone()), "Artboard 1");
    }

    /// A new artboard takes the color, guide label and marks set, or the defaults.
    #[test]
    fn new_artboard_takes_the_values_set() {
        let mut app = app();
        for (fields, color, label, marks) in [
            (vec![("color", json!("#336699")), (GUIDE_LABEL, json!("Teal")), ("showCenterMark", json!(true))], Some("#336699"), Some("Teal"), true),
            (vec![], None, None, false),
        ] {
            open_new(&mut app).unwrap();
            set(&app, &fields).unwrap();
            ok(&mut app).unwrap();
            let ab = app.session.doc().unwrap().doc.artboards.last().cloned().unwrap();
            assert_eq!(ab.color.map(|c| c.to_hex()).as_deref(), color);
            assert_eq!(ab.guide_label.map(LayerColor::label_text).as_deref(), label);
            assert_eq!(ab.show_center_mark, marks);
            assert_eq!(app.view().unwrap().artboard, app.session.doc().unwrap().doc.artboards.len() - 1, "the new artboard is active");
        }
    }

    /// OK writes the preferences that changed, and an error leaves them as they were.
    #[test]
    fn globals_set_the_preferences_and_an_error_leaves_them() {
        let mut app = app();
        let prefs = |app: &VectorcraftApp| (app.session.prefs.fade_region_outside_artboard, app.session.prefs.artboard_update_while_dragging);
        assert_eq!(prefs(&app), (true, true));
        open(&mut app).unwrap();
        set(&app, &[(FADE, json!(false)), (GUIDE_LABEL, json!("purple"))]).unwrap();
        assert!(ok(&mut app).is_err() && is_open(&app));
        assert_eq!(prefs(&app), (true, true), "a refused artboard command leaves the preferences");
        set(&app, &[(GUIDE_LABEL, Value::Null)]).unwrap();
        ok(&mut app).unwrap();
        assert_eq!(prefs(&app), (false, true));
        open_new(&mut app).unwrap();
        set(&app, &[(UPDATE_WHILE_DRAGGING, json!(false))]).unwrap();
        ok(&mut app).unwrap();
        assert_eq!(prefs(&app), (false, false));
    }

    /// The video options reach the artboard in one undo step; a ratio out of range is refused; the
    /// Artboard tool's double click opens with the stored values; a new artboard takes the values set.
    #[test]
    fn video_options_apply_refuse_a_ratio_out_of_range_and_reach_a_new_artboard() {
        let mut app = app();
        let board = |app: &VectorcraftApp, i: usize| app.session.doc().unwrap().doc.artboards[i].clone();
        open(&mut app).unwrap();
        assert!(set(&app, &[(PIXEL_ASPECT, json!(1e9))]).is_err(), "out of range");
        let undo = undo_len(&app);
        set(&app, &[("showVideoSafeAreas", json!(true)), (PIXEL_ASPECT, json!(1.5))]).unwrap();
        ok(&mut app).unwrap();
        let ab = board(&app, 0);
        assert_eq!((ab.show_video_safe_areas, ab.video_ruler_pixel_aspect_ratio, undo_len(&app)), (true, 1.5, undo + 1));
        app.select_tool("artboard");
        crate::dialogs::open_tool_dialog(
            &mut app,
            "artboardOptions",
            json!({"index": 0, "name": "Artboard 1", "x": 0, "y": 0, "width": 200, "height": 150,
            "showVideoSafeAreas": true, "videoRulerPixelAspectRatio": 1.5}),
        );
        let v = app.dialogs().unwrap().with(|h| h.get::<ArtboardOptionsDialog>().unwrap().value.clone());
        assert_eq!((v.show_video_safe_areas, v.pixel_aspect), (true, 1.5));
        open_new(&mut app).unwrap();
        set(&app, &[(PIXEL_ASPECT, json!(2.0))]).unwrap();
        ok(&mut app).unwrap();
        assert_eq!(board(&app, 1).video_ruler_pixel_aspect_ratio, 2.0);
    }

    /// Two selected artboards: a new Width goes to both, each keeping its top-left corner and
    /// height, in one undo step; the name goes only to the active one. About another reference point
    /// the others resize about it.
    #[test]
    fn width_resizes_every_selected_artboard() {
        for (column, want) in [(0, Rect::new(400.0, 0.0, 700.0, 50.0)), (2, Rect::new(200.0, 0.0, 500.0, 50.0))] {
            let mut app = app();
            app.run("artboard.new", json!({"x": 400, "y": 0, "width": 100, "height": 50})).unwrap();
            app.select_tool("artboard");
            app.session.set_tool_option("selected", &json!([0, 1]));
            open(&mut app).unwrap();
            let before = app.session.doc().unwrap().doc.clone();
            set(&app, &[("width", json!(300)), ("name", json!("Wide"))]).unwrap();
            edit(&app, |v| v.ref_x = column);
            ok(&mut app).unwrap();
            let names: Vec<_> = app.session.doc().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
            assert_eq!(boards(&app)[1], want, "column {column}");
            assert_eq!(
                (boards(&app)[0].width(), boards(&app)[0].height(), names[0].as_str(), names[1].as_str()),
                (300.0, 150.0, "Wide", "Artboard 2")
            );
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(*app.session.doc().unwrap().doc, *before, "one undo step");
        }
        // A name alone resizes nothing.
        let mut app = app();
        app.run("artboard.new", json!({"x": 400, "y": 0, "width": 100, "height": 50})).unwrap();
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([0, 1]));
        open(&mut app).unwrap();
        set(&app, &[("name", json!("Only"))]).unwrap();
        ok(&mut app).unwrap();
        assert_eq!(boards(&app)[1], Rect::new(400.0, 0.0, 500.0, 50.0));
    }

    /// A preset applies to each selected artboard in its own orientation; an orientation swaps only
    /// the artboards that are not already in it.
    #[test]
    fn a_size_choice_applies_to_the_other_selected_artboards() {
        let mut app = app();
        app.run("artboard.new", json!({"x": 400, "y": 0, "width": 50, "height": 100})).unwrap();
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([0, 1]));
        open(&mut app).unwrap();
        edit(&app, |v| v.size_choice = Some(ArtboardSizeChoice::Preset(300.0, 100.0)));
        ok(&mut app).unwrap();
        assert_eq!(boards(&app)[1], Rect::new(400.0, 0.0, 500.0, 300.0), "the portrait artboard gets the preset turned");
        open(&mut app).unwrap();
        edit(&app, |v| v.size_choice = Some(ArtboardSizeChoice::Orientation(false)));
        ok(&mut app).unwrap();
        assert_eq!((boards(&app)[1].width(), boards(&app)[1].height()), (100.0, 300.0), "already portrait: unchanged");
    }

    /// A Fit sets the artboard to the bounds and leaves the art even with Move Artwork on; the bounds
    /// measure all art and the selection; Undo restores the document.
    #[test]
    fn fit_sets_the_artboard_and_leaves_the_art() {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 300, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.select_tool("artboard");
        app.session.set_tool_option("moveArt", &json!(true));
        open(&mut app).unwrap();
        let bounds = app.dialogs().unwrap().with(|h| h.get::<ArtboardOptionsDialog>().unwrap().setup.fit_bounds);
        let art = bounds[0].expect("art off every board counts");
        assert_eq!(bounds[1], app.selection_bounds());
        let before = app.session.doc().unwrap().doc.clone();
        edit(&app, |v| {
            v.set_rect(art);
            v.fit = true;
        });
        ok(&mut app).unwrap();
        assert_eq!(boards(&app)[0], art);
        let moved = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0;
        assert_eq!(moved, 300.0, "the art stays");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.doc().unwrap().doc, *before);
    }

    /// Without art or selection both Fit entries are off, and Cancel leaves the document.
    #[test]
    fn fit_is_off_without_art_and_cancel_changes_nothing() {
        let mut app = app();
        open(&mut app).unwrap();
        let bounds = app.dialogs().unwrap().with(|h| h.get::<ArtboardOptionsDialog>().unwrap().setup.fit_bounds);
        assert_eq!(bounds, [None, None]);
        let before = app.session.doc().unwrap().doc.clone();
        set(&app, &[("width", json!(10))]).unwrap();
        let ctx = crate::testing::ui_context();
        app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, id()));
        assert!(!is_open(&app));
        assert_eq!(*app.session.doc().unwrap().doc, *before);
    }
}
