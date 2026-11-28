//! The tracing-settings rows the Image Trace panel, its preset editor and Tracing Presets share: Mode, Palette,
//! Library, Colors and the mode's count or threshold, then the Advanced settings. Each row writes the setting into
//! the `params` it is given and reports what the user did, so a host decides when to trace or save.

// PLAN: assumed the `craft` dependency of ui-egui enables the `color-ui` feature (`ColorPopover`, `PickedColor`);
// the manifest is outside this set's files.
use std::ops::RangeInclusive;

use craft::color_ui::ui::{ColorPopover, PickedColor};
use craft::theme::Srgba;
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::{Field, SliderField, SliderFieldLayout};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
// PLAN: assumed `IconId` is re-exported from `craft::ui::primitives`.
use craft::behavior::IconId;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES;

use super::{MODES, PALETTES, ignore_color, palette_label};
use crate::VectorcraftApp;
use crate::panels::swatches;

/// Where the rows are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowsHost {
    /// The Image Trace panel: a colour group the active document lacks shows as All.
    Panel,
    /// A dialog that edits a preset: a colour group stays by its name, whichever document is open.
    Dialog,
}

/// What the rows list: the libraries and the active document's colour groups.
#[derive(Clone)]
pub(crate) struct RowsData {
    /// Document Library's choices ([`swatches::library_entries`]).
    pub(crate) libraries: Vec<Entry<String>>,
    /// The active document's colour group names.
    pub(crate) groups: Vec<String>,
}

impl RowsData {
    /// The lists for the active document.
    pub(crate) fn of(app: &VectorcraftApp) -> Self {
        // PLAN: assumed `swatches::library_entries(app: &VectorcraftApp) -> Vec<Entry<String>>` (color-w10).
        RowsData { libraries: swatches::library_entries(app), groups: swatch_groups(app) }
    }
}

/// What the user did in the rows in one pass.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RowsOutput {
    /// A setting changed, a slider's proposals included.
    pub(crate) changed: bool,
    /// A setting was settled: a pick, a check, a segment or colour pick, or a slider's commit.
    pub(crate) committed: bool,
    /// The colour well of Ignore Color was clicked.
    pub(crate) color_opened: bool,
    /// The eyedropper button was toggled.
    pub(crate) sampler_toggled: bool,
}

impl RowsOutput {
    fn picked(&mut self) {
        self.changed = true;
        self.committed = true;
    }
}

/// The Ignore Color eyedropper button of a host that can sample the source image.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sampler {
    /// The button can be used: an image or an Image Trace object is the target.
    pub(crate) enabled: bool,
    /// The sampler is armed.
    pub(crate) armed: bool,
}

/// The names of the active document's colour groups.
pub(super) fn swatch_groups(app: &VectorcraftApp) -> Vec<String> {
    app.session.active().map(|d| d.doc.swatch_groups.iter().map(|g| g.name.clone()).collect()).unwrap_or_default()
}

/// `params`' colour group when `groups` has it, found as `imageTrace.make` finds it (named exactly, else ignoring
/// case); `None` is All. A group of another document, or a deleted one, is All.
pub(super) fn known_group<'a>(groups: &[String], params: &'a Value) -> Option<&'a str> {
    let name = params["colorGroup"].as_str().filter(|g| !g.is_empty())?;
    let lower = name.to_lowercase();
    groups.iter().any(|g| g.to_lowercase() == lower).then_some(name)
}

/// The tooltip and name of the eyedropper button.
fn sample_tip() -> &'static str {
    tl!("Eyedropper: click the source image to sample the ignored color")
}

fn text(params: &Value, key: &str, default: &str) -> String {
    params[key].as_str().unwrap_or(default).to_string()
}

/// A number as the engine stores the setting `key`: whole for counts, else rounded.
fn number(key: &str, v: f64) -> Value {
    if matches!(key, "threshold" | "colors" | "noise") { json!(v.round() as u64) } else { json!(v.round()) }
}

fn hex(color: Srgba) -> String {
    let [r, g, b] = color.to_rgb_array().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// A check box named `label`; whether this pass flipped it.
fn check(ui: &mut Ui, label: &'static str, on: bool, enabled: bool) -> bool {
    let mut value = on;
    Checkbox::new(Bind::bound(&mut value)).label(label).enabled(enabled).show(ui);
    value != on
}

/// A field named `label` over a select of `entries` showing `current`; the key the user picked this pass.
fn select_row(ui: &mut Ui, label: &'static str, entries: &[Entry<String>], current: &str, placeholder: Option<&str>) -> Option<String> {
    Field::new(label)
        .show(ui, |ui, _| {
            let mut select = Select::new(label, Bind::given(current.to_string()), entries);
            if let Some(p) = placeholder {
                select = select.placeholder(p.to_string());
            }
            select.show(ui).events.committed().cloned()
        })
        .inner
        .flatten()
}

/// One number setting: its key, name, range, unit and the value shown when the setting is absent.
struct Slider {
    key: &'static str,
    label: &'static str,
    range: RangeInclusive<f64>,
    unit: Option<&'static str>,
    default: f64,
}

fn slider(key: &'static str, label: &'static str, range: RangeInclusive<f64>, unit: Option<&'static str>) -> Slider {
    Slider { key, label, range, unit, default: 0.0 }
}

fn slider_row(ui: &mut Ui, params: &mut Value, spec: Slider, enabled: bool, out: &mut RowsOutput) {
    let Slider { key, label, range, unit, default } = spec;
    let value = params[key].as_f64().unwrap_or(default);
    let events = Field::new(label)
        .enabled(enabled)
        .show(ui, |ui, _| {
            let mut field = SliderField::new(label, Bind::given(value), range.clone()).layout(SliderFieldLayout::Inline).step(1.0).enabled(enabled);
            if let Some(unit) = unit {
                field = field.unit(unit);
            }
            field.show(ui).edit.events
        })
        .inner;
    let Some(events) = events else { return };
    if let Some(v) = events.value() {
        params[key] = number(key, *v);
        out.changed = true;
    }
    out.committed |= events.committed().is_some();
}

/// Mode, Palette (Color only), Library and Colors (Document Library only), and the mode's count or threshold.
pub(crate) fn main_rows(ui: &mut Ui, params: &mut Value, data: &RowsData, host: RowsHost) -> RowsOutput {
    let mut out = RowsOutput::default();
    let mode = text(params, "mode", "blackAndWhite");
    let modes: Vec<Entry<String>> = MODES.iter().map(|(id, label)| Entry::new((*id).to_string(), *label)).collect();
    if let Some(k) = select_row(ui, tl!("Mode:"), &modes, &mode, None) {
        // Only Color mode uses a color group.
        if k != "color" {
            params["colorGroup"] = Value::Null;
        }
        params["mode"] = json!(k);
        out.picked();
    }
    let palette = text(params, "palette", "limited");
    if mode == "color" {
        let palettes: Vec<Entry<String>> = PALETTES.iter().map(|id| Entry::new((*id).to_string(), palette_label(id))).collect();
        if let Some(k) = select_row(ui, tl!("Palette:"), &palettes, &palette, None) {
            // Every other palette refuses a color group.
            if k != "documentLibrary" {
                params["colorGroup"] = Value::Null;
            }
            params["palette"] = json!(k);
            out.picked();
        }
        if palette == "documentLibrary" {
            let key = text(params, "library", DOCUMENT_SWATCHES);
            if let Some(k) = select_row(ui, tl!("Library:"), &data.libraries, &key, Some(&key)) {
                if k != DOCUMENT_SWATCHES {
                    params["colorGroup"] = Value::Null;
                }
                params["library"] = json!(k);
                out.picked();
            }
            // The document's swatches: every one, or one color group's.
            if key == DOCUMENT_SWATCHES {
                let group = match host {
                    RowsHost::Panel => known_group(&data.groups, params),
                    RowsHost::Dialog => params["colorGroup"].as_str().filter(|g| !g.is_empty()),
                }
                .map(str::to_string);
                let mut groups = vec![Entry::new(String::new(), tl!("All"))];
                if !data.groups.is_empty() {
                    groups.push(Entry::separator());
                    groups.extend(data.groups.iter().map(|g| Entry::new(g.clone(), g.clone())));
                }
                let current = group.clone().unwrap_or_default();
                if let Some(k) = select_row(ui, tl!("Colors:"), &groups, &current, group.as_deref()) {
                    params["colorGroup"] = if k.is_empty() { Value::Null } else { json!(k) };
                    out.picked();
                }
            }
        }
    }
    // Full Tone and Automatic: how many colours (Less … More); else a count or the threshold.
    let count = match mode.as_str() {
        "blackAndWhite" => slider("threshold", tl!("Threshold"), 0.0..=255.0, None),
        "grayscale" => slider("colors", tl!("Grays"), 2.0..=256.0, None),
        // Flat Logo finds a handful of flat colours: more than ten is almost always noise.
        "logo" => slider("colors", tl!("Colors"), 2.0..=10.0, None),
        _ if matches!(palette.as_str(), "fullTone" | "automatic") => slider("colorDetail", tl!("Colors"), 0.0..=100.0, Some("%")),
        _ => slider("colors", tl!("Colors"), 2.0..=256.0, None),
    };
    slider_row(ui, params, count, true, &mut out);
    out
}

/// The Advanced settings: fidelity, method, Create, snapping, shapes, grouping, gradients and Ignore Color, with an
/// eyedropper for Ignore Color when the host passes a `sampler`. `_data` and `_host` are the arguments of
/// [`main_rows`], so a host passes the same set to both.
pub(crate) fn advanced_rows(ui: &mut Ui, params: &mut Value, _data: &RowsData, _host: RowsHost, sampler: Option<Sampler>) -> RowsOutput {
    let mut out = RowsOutput::default();
    let mode = text(params, "mode", "blackAndWhite");
    for spec in [
        slider("paths", tl!("Paths"), 0.0..=100.0, Some("%")),
        slider("corners", tl!("Corners"), 0.0..=100.0, Some("%")),
        slider("noise", tl!("Noise"), 1.0..=100.0, Some("px")),
    ] {
        slider_row(ui, params, spec, true, &mut out);
    }
    // Flat Logo always paints biggest shapes first with a hair of overlap, so there is no choice.
    if mode != "logo" {
        let methods = [Entry::new("abutting".to_string(), tl!("Abutting")), Entry::new("overlapping".to_string(), tl!("Overlapping"))];
        let method = text(params, "method", "");
        let picked = Field::new(tl!("Method:"))
            .show(ui, |ui, _| Segmented::new(tl!("Method:"), Bind::given(method.clone()), &methods).show(ui).events.committed().cloned())
            .inner
            .flatten();
        if let Some(m) = picked.filter(|m| *m != method) {
            params["method"] = json!(m);
            out.picked();
        }
        // Create: Fills and/or Strokes (one of them stays on), and the widest line stroked.
        let create = |p: &Value, key: &str| p[key].as_bool().unwrap_or(key == "fills");
        let (fills, strokes) = (create(params, "fills"), create(params, "strokes"));
        let mut flipped = None;
        Field::new(tl!("Create:")).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let mut hit = None;
                for (key, on, other, label) in [("fills", fills, strokes, tl!("Fills")), ("strokes", strokes, fills, tl!("Strokes"))] {
                    // Inside the Create field a check box's own label is not drawn, so each box takes a field of
                    // its own with the label after it.
                    let enabled = !on || other;
                    let mut value = on;
                    Field::new(label).layout(FieldLayout::After).enabled(enabled).show(ui, |ui, _| {
                        Checkbox::new(Bind::bound(&mut value)).enabled(enabled).show(ui);
                    });
                    if value != on {
                        hit = Some(key);
                    }
                }
                flipped = hit;
            });
        });
        if let Some(key) = flipped {
            params[key] = json!(!create(params, key));
            out.picked();
        }
        slider_row(ui, params, Slider { default: 10.0, ..slider("strokeWidth", tl!("Stroke"), 1.0..=100.0, Some("px")) }, strokes, &mut out);
    }
    // (Black and White has one colour and Flat Logo flat ones: no gradients to find.)
    let gradients = mode == "grayscale" || mode == "color";
    for (key, label, enabled) in [
        ("snapCurvesToLines", tl!("Snap Curves To Lines"), true),
        ("shapes", tl!("Shapes"), true),
        ("autoGrouping", tl!("Auto Grouping"), true),
        ("gradients", tl!("Gradients"), gradients),
    ] {
        let on = params[key].as_bool().unwrap_or(false);
        if check(ui, label, on, enabled) {
            params[key] = json!(!on);
            out.picked();
        }
    }
    // Ignore Color: white, unless another colour is picked.
    let on = params["ignoreWhite"].as_bool().unwrap_or(false);
    let (mut flipped, mut opened, mut toggled, mut color) = (false, false, false, None);
    Row::new().show(ui, |ui, _| {
        flipped = check(ui, tl!("Ignore Color"), on, true);
        (opened, toggled, color) = (false, false, None);
        if !on {
            return;
        }
        let [r, g, b] = ignore_color(params).map(|c| f32::from(c) / 255.0);
        let well = ColorPopover::new(tl!("Color"), Bind::given(PickedColor::rgb(Srgba::rgb(r, g, b)))).show(ui);
        opened = well.response.clicked();
        color = well.events.value().map(|p| (hex(p.color), well.events.committed().is_some()));
        if let Some(s) = sampler {
            let mut armed = s.armed;
            ToggleButton::new(sample_tip(), Bind::bound(&mut armed))
                .icon(IconId::new("pipette"))
                .icon_only()
                .tooltip(sample_tip())
                .enabled(s.enabled)
                .show(ui);
            toggled = armed != s.armed;
        }
    });
    if flipped {
        params["ignoreWhite"] = json!(!on);
        out.picked();
    }
    if let Some((color, settled)) = color {
        params["ignoreColor"] = json!(color);
        out.changed = true;
        out.committed |= settled;
    }
    out.color_opened = opened;
    out.sampler_toggled = toggled;
    out
}
