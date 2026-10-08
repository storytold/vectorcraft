//! File → New: category tabs (Recent, Saved and the built-in categories of `file.newPresets`)
//! over a grid of preset cards, and a Preset Details column (name and Save Preset, size and
//! units, orientation, artboards, bleed, background contents, Advanced Options, More Settings);
//! Close and Create. More Settings is the classic form of the same fields (artboard layout,
//! spacing and columns, profile and size lists), so both share one set of fields and widgets.
//!
//! Fields: `file.new`'s params (`preset`, `name`, `width`, `height` (pt), `units`, `artboards`,
//! `artboardLayout`, `bleed`, `backgroundContents`, `colorMode`, `rasterEffectsPpi`,
//! `previewMode`), plus `category` (the tab, also More Settings' Profile), `advanced` (Advanced
//! Options open), `bleedLinked` and `presetName` (Save Preset's name). Setting `preset` fills the
//! fields from that preset; changing any of them afterwards makes the settings custom.

use serde_json::{Map, Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::newdoc::{self, ArtboardLayout, DocSettings, MAX_ARTBOARDS, PreviewMode, RASTER_PPI};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "newDocument";
/// More Settings.
pub(super) const MORE: &str = "newDocumentMore";

pub(super) const SPEC: DialogSpec = DialogSpec::window(show, confirm);
pub(super) const MORE_SPEC: DialogSpec = DialogSpec::window(show_more, confirm);

/// The fields `file.new` takes.
const PARAMS: [&str; 12] = [
    "preset",
    "name",
    "width",
    "height",
    "units",
    "artboards",
    "artboardLayout",
    "bleed",
    "backgroundContents",
    "colorMode",
    "rasterEffectsPpi",
    "previewMode",
];
/// The preset the fields were last filled from (its fields as `apply` set them).
const APPLIED: &str = "__applied";
/// Save Preset's name field is showing.
const SAVING: &str = "__saving";
/// The units New Document offers (all ten).
const UNITS: [Unit; 10] = [
    Unit::Pixels,
    Unit::Points,
    Unit::Picas,
    Unit::Inches,
    Unit::Millimeters,
    Unit::Centimeters,
    Unit::Feet,
    Unit::Yards,
    Unit::Meters,
    Unit::FeetInches,
];
const BACKGROUNDS: [(&str, &str); 2] = [("transparent", "Transparent (Default)"), ("white", "White")];
const COLOR_MODES: [(&str, &str); 2] = [("rgb", "RGB Color"), ("cmyk", "CMYK Color")];
/// Width of the Preset Details column.
const DETAILS: f32 = 292.0;
/// Height of the dialog's content.
const HEIGHT: f32 = 600.0;

/// Open New Document on the most recent settings (or Letter), named after the next untitled
/// document.
pub fn open(app: &mut VectorcraftApp) {
    let category = if app.session.prefs.recent_new_docs.is_empty() { "Print" } else { "Recent" };
    let first = newdoc::category(&app.session, category).and_then(|v| v.into_iter().next()).unwrap_or_default();
    let mut d = Dialog::new(KIND, json!({ "category": category, "name": app.session.peek_untitled(), "advanced": false }));
    apply(&mut d, &first);
    app.ui.dialog = Some(d);
}

/// Fill the fields from a preset.
fn apply(d: &mut Dialog, s: &DocSettings) {
    let Value::Object(mut v) = s.to_json() else { return };
    for k in ["size", "orientation"] {
        v.remove(k);
    }
    if let Some(name) = v.remove("name") {
        v.insert("preset".into(), name);
    }
    d.fields.insert("bleedLinked".into(), json!(s.bleed.iter().all(|b| *b == s.bleed[0])));
    for (k, val) in &v {
        d.fields.insert(k.clone(), val.clone());
    }
    d.fields.insert(APPLIED.into(), Value::Object(v));
}

/// Keep the fields and the chosen preset in step: a new `preset` (also from `ui.dialog.set`) fills
/// the fields; a changed field makes the settings custom.
fn sync(app: &VectorcraftApp, d: &mut Dialog) {
    let preset = d.str("preset");
    match d.fields.get(APPLIED).and_then(Value::as_object) {
        Some(a) if a.get("preset").and_then(Value::as_str) == Some(preset.as_str()) => {
            // Unchanged fields keep the preset (falling through would apply it again).
            let edited = a.iter().any(|(k, v)| k != "preset" && d.fields.get(k) != Some(v));
            if edited {
                d.fields.insert("preset".into(), json!(""));
                d.fields.remove(APPLIED);
            }
        }
        _ if !preset.is_empty() => match newdoc::find(&app.session, &preset) {
            Some(s) => apply(d, &s),
            None => {
                d.fields.remove(APPLIED);
            }
        },
        _ => {}
    }
}

/// Create the document; the dialog stays open when that fails (bad size, …).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p: Map<String, Value> = PARAMS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    // Recent entries such as "Custom" aren't presets file.new knows.
    if newdoc::find(&app.session, &d.str("preset")).is_none() {
        p.remove("preset");
    }
    let r = app.run("file.new", Value::Object(p));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

/// The units the size fields show.
fn unit(d: &Dialog) -> Unit {
    Unit::named(&d.str("units")).unwrap_or_default()
}

/// The artboard layout fields.
fn layout(d: &Dialog) -> (ArtboardLayout, u64, f64, bool) {
    let l = d.fields.get("artboardLayout");
    let get = |k: &str| l.and_then(|l| l.get(k));
    (
        get("layout").and_then(Value::as_str).and_then(ArtboardLayout::parse).unwrap_or_default(),
        get("columns").and_then(Value::as_u64).unwrap_or(1),
        get("spacing").and_then(Value::as_f64).unwrap_or(20.0),
        get("rightToLeft").and_then(Value::as_bool).unwrap_or(false),
    )
}

fn set_layout(d: &mut Dialog, key: &str, v: Value) {
    let mut l = d.fields.get("artboardLayout").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    l[key] = v;
    d.fields.insert("artboardLayout".into(), l);
}

/// The dimmed backdrop and a centred dialog window in the shared dialog style.
fn window(ctx: &egui::Context, id: &str, margin: i8, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ctx);
    egui::Area::new(egui::Id::new("modal-dim")).order(egui::Order::Middle).fixed_pos(egui::pos2(0.0, 0.0)).show(ctx, |ui| {
        ui.allocate_rect(ctx.content_rect(), egui::Sense::click());
    });
    egui::Window::new(id)
        .id(egui::Id::new(("dialog", id)))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center() + egui::vec2(0.0, -20.0))
        .constrain(true)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(margin)))
        .show(ctx, add);
}

/// What the buttons of a window asked for.
#[derive(Default)]
struct Buttons {
    create: bool,
    close: bool,
}

impl Buttons {
    /// Act on the buttons (Enter also creates) once the window is drawn.
    fn finish(self, app: &mut VectorcraftApp, ctx: &egui::Context, d: Dialog) {
        let create = self.create || ctx.input(|i| i.key_pressed(egui::Key::Enter)) && !ctx.memory(|m| m.focused().is_some());
        app.ui.dialog = Some(d);
        if self.close {
            app.ui.dialog = None;
        } else if create && let Err(e) = super::confirm(app) {
            app.status(e);
        }
    }
}

fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    sync(app, &mut d);
    let t = Tokens::get(ctx);
    let mut b = Buttons::default();
    window(ctx, KIND, 0, |ui| {
        ui.horizontal_top(|ui| {
            egui::Frame::NONE.inner_margin(egui::Margin { left: 22, right: 14, top: 14, bottom: 18 }).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(640.0);
                    ui.set_min_height(HEIGHT - 32.0);
                    presets(app, ui, &mut d);
                });
            });
            egui::Frame::NONE.fill(t.panel_darker).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(DETAILS);
                    ui.set_min_height(HEIGHT);
                    details(app, ui, &mut d, &mut b);
                });
            });
        });
    });
    b.finish(app, ctx, d);
}

/// The category tabs and the chosen category's preset cards.
fn presets(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let names: Vec<&str> = newdoc::category_names().collect();
    let cat = names.iter().position(|n| n.eq_ignore_ascii_case(&d.str("category"))).unwrap_or(0);
    if let Some(i) = widgets::tab_bar(ui, &names, cat) {
        d.fields.insert("category".into(), json!(names[i]));
    }
    let list = names.get(cat).and_then(|c| newdoc::category(&app.session, c)).unwrap_or_default();
    ui.add_space(14.0);
    let heading = if cat == 0 {
        tl!("RECENT")
    } else if cat == 1 {
        tl!("SAVED")
    } else {
        tl!("BLANK DOCUMENT PRESETS")
    };
    ui.label(egui::RichText::new(format!("{heading} ({})", list.len())).font(theme::semibold(11.0)).color(t.text_dim));
    ui.add_space(10.0);
    if list.is_empty() {
        let hint = if cat == 1 { tl!("Presets you save with the Save Preset button appear here.") } else { tl!("Documents you create appear here.") };
        ui.label(egui::RichText::new(hint).color(t.text_dim));
        return;
    }
    egui::ScrollArea::vertical().id_salt(("newdoc-presets", cat)).max_height(HEIGHT - 110.0).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
            let (w, h, preset) = (d.f64("width", 0.0), d.f64("height", 0.0), d.str("preset"));
            for s in &list {
                let selected = d.fields.contains_key(APPLIED) && s.name == preset && s.width == w && s.height == h;
                if preset_card(ui, s, selected).clicked() {
                    apply(d, s);
                }
            }
        });
    });
}

/// Is `name` a built-in preset's? Those are ours (translated); Recent and Saved hold the names of
/// the user's documents and presets, shown as they are.
fn is_builtin_preset(name: &str) -> bool {
    newdoc::CATEGORIES.iter().any(|(_, _, _, presets)| presets.iter().any(|(n, ..)| *n == name))
}

/// A preset's name as shown: a built-in one translated, the user's as it is.
fn preset_name(name: &str) -> &str {
    if is_builtin_preset(name) { tl!(name) } else { name }
}

/// A preset card: a page-proportioned thumbnail, the name and the size (New Document, the Home
/// screen). Returns the card's response.
pub fn preset_card(ui: &mut egui::Ui, s: &DocSettings, selected: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let label = preset_name(&s.name);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(146.0, 150.0), egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(r, egui::CornerRadius::same(6), if resp.hovered() || selected { t.hover } else { t.panel_darker });
    if selected {
        p.rect_stroke(r, egui::CornerRadius::same(6), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    let k = (60.0 / s.width.max(s.height).max(1e-9)) as f32;
    let page = egui::Rect::from_center_size(
        r.center_top() + egui::vec2(0.0, 48.0),
        egui::vec2((s.width as f32 * k).max(4.0), (s.height as f32 * k).max(4.0)),
    );
    p.rect_filled(page.translate(egui::vec2(2.0, 2.0)), 0.0, egui::Color32::from_black_alpha(80));
    p.rect_filled(page, 0.0, egui::Color32::WHITE);
    // The name on up to two lines, then the size.
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat { font_id: theme::semibold(11.5), color: t.text_strong, ..Default::default() },
    );
    job.halign = egui::Align::Center;
    job.wrap = egui::text::TextWrapping { max_width: r.width() - 14.0, max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
    let name = ui.fonts_mut(|f| f.layout_job(job));
    let name_h = name.size().y;
    p.galley(egui::pos2(r.center().x, r.top() + 94.0), name, t.text_strong);
    p.text(egui::pos2(r.center().x, r.top() + 98.0 + name_h), egui::Align2::CENTER_TOP, s.size_label(), egui::FontId::proportional(11.0), t.text_dim);
    resp.on_hover_text(label)
}

/// The Preset Details column.
fn details(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, b: &mut Buttons) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
    let top = ui.cursor().top();
    ui.label(egui::RichText::new(tl!("PRESET DETAILS")).font(theme::semibold(11.0)).color(t.text_dim));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        form::text(ui, d, "name", DETAILS - 50.0);
        if widgets::icon_button(ui, "save", tl!("Save Preset"), d.bool(SAVING), 26.0).clicked() {
            let saving = !d.bool(SAVING);
            d.fields.insert(SAVING.into(), json!(saving));
            if saving && d.str("presetName").is_empty() {
                d.fields.insert("presetName".into(), json!(format!("Preset {}", app.session.prefs.new_doc_presets.len() + 1)));
            }
        }
    });
    if d.bool(SAVING) {
        save_row(app, ui, d);
    }
    ui.add_space(8.0);
    let unit = unit(d);
    form::caption(ui, tl!("Width"));
    ui.horizontal(|ui| {
        form::length(ui, d, "width", unit, DETAILS - 124.0);
        units_dropdown(ui, d, 112.0);
    });
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            form::caption(ui, tl!("Height"));
            form::length(ui, d, "height", unit, 104.0);
        });
        ui.add_space(6.0);
        ui.vertical(|ui| {
            form::caption(ui, tl!("Orientation"));
            orientation(ui, d);
        });
        ui.add_space(6.0);
        ui.vertical(|ui| {
            form::caption(ui, tl!("Artboards"));
            artboards(ui, d, 76.0);
        });
    });
    ui.add_space(8.0);
    form::caption(ui, tl!("Bleed"));
    form::bleed(ui, d, unit, 52.0);
    ui.add_space(8.0);
    form::caption(ui, tl!("Background Contents"));
    background(ui, d, DETAILS - 34.0);
    ui.add_space(8.0);
    advanced(ui, d, 110.0, DETAILS - 116.0);
    ui.add_space(12.0);
    if widgets::flat_button(ui, tl!("More Settings"), DETAILS).clicked() {
        d.kind = MORE.into();
    }
    // Close and Create at the bottom right.
    ui.add_space((HEIGHT - (ui.cursor().top() - top) - 28.0).max(12.0));
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 28.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        b.create = widgets::primary_button(ui, tl!("Create")).clicked();
        ui.add_space(8.0);
        b.close = widgets::secondary_button(ui, tl!("Close")).clicked();
    });
}

/// Save Preset: the preset's name, Save and Cancel.
fn save_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    ui.add_space(4.0);
    form::caption(ui, tl!("Preset Name"));
    ui.horizontal(|ui| {
        let enter = form::text_edit(ui, d, "presetName", DETAILS - 140.0).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if widgets::flat_button(ui, tl!("Save"), 54.0).clicked() || enter {
            let mut p: Map<String, Value> = PARAMS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
            p.insert("name".into(), json!(d.str("presetName")));
            p.remove("preset");
            match app.run("file.newPresets.save", Value::Object(p)) {
                Ok(r) => {
                    d.fields.insert(SAVING.into(), json!(false));
                    d.fields.insert("category".into(), json!("Saved"));
                    d.fields.insert("preset".into(), r["name"].clone());
                    d.fields.remove(APPLIED);
                }
                Err(e) => app.status(e),
            }
        }
        if widgets::flat_button(ui, tl!("Cancel"), 60.0).clicked() {
            d.fields.insert(SAVING.into(), json!(false));
        }
    });
}

fn units_dropdown(ui: &mut egui::Ui, d: &mut Dialog, width: f32) {
    let labels = UNITS.map(Unit::label);
    if let Some(u) = widgets::dropdown(ui, "newdoc-units", unit(d).label(), &labels, width).and_then(|i| UNITS.get(i)) {
        d.fields.insert("units".into(), json!(u.label()));
    }
}

/// Portrait and landscape: choosing the other one swaps width and height.
fn orientation(ui: &mut egui::Ui, d: &mut Dialog) {
    let (w, h) = (d.f64("width", 612.0), d.f64("height", 792.0));
    let landscape = w > h;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (is_landscape, tip) in [(false, tl!("Portrait")), (true, tl!("Landscape"))] {
            if widgets::orientation_button(ui, is_landscape, landscape == is_landscape, tip) && landscape != is_landscape {
                d.fields.insert("width".into(), json!(h));
                d.fields.insert("height".into(), json!(w));
            }
        }
    });
}

fn artboards(ui: &mut egui::Ui, d: &mut Dialog, width: f32) {
    let n = d.f64("artboards", 1.0);
    ui.horizontal(|ui| {
        if let Some(v) = widgets::spin_plain(ui, "newdoc-artboards", n, "", 0, width, 1.0, 1.0, &[]) {
            d.fields.insert("artboards".into(), json!(v.round().clamp(1.0, MAX_ARTBOARDS as f64) as u64));
        }
    });
}

/// Background Contents with its chip: white, or white slashed in red for transparent.
fn background(ui: &mut egui::Ui, d: &mut Dialog, width: f32) {
    let t = Tokens::get(ui.ctx());
    let cur = d.str("backgroundContents");
    let shown = BACKGROUNDS.iter().find(|(v, _)| *v == cur).map_or(BACKGROUNDS[0].1, |(_, l)| l);
    ui.horizontal(|ui| {
        if let Some((v, _)) = widgets::dropdown(ui, "newdoc-background", shown, &BACKGROUNDS.map(|b| b.1), width).and_then(|i| BACKGROUNDS.get(i)) {
            d.fields.insert("backgroundContents".into(), json!(v));
        }
        let (chip, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        ui.painter().rect_filled(chip, 2.0, egui::Color32::WHITE);
        if cur != "white" {
            ui.painter().line_segment([chip.left_bottom(), chip.right_top()], egui::Stroke::new(1.5, t.bleed));
        }
        ui.painter().rect_stroke(chip, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    });
}

/// A dropdown bound to `d.fields[key]` over (value, label) pairs.
fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, options: &[(Value, String)], width: f32) {
    let cur = d.fields.get(key).cloned().unwrap_or_default();
    let shown = options
        .iter()
        .find(|(v, _)| *v == cur || v.as_f64().is_some_and(|x| cur.as_f64() == Some(x)))
        .map_or_else(|| cur.to_string(), |(_, l)| l.clone());
    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
    if let Some((v, _)) = widgets::dropdown(ui, ("newdoc", key), &shown, &labels, width).and_then(|i| options.get(i)) {
        d.fields.insert(key.into(), v.clone());
    }
}

/// The collapsible Advanced Options: Color Mode, Raster Effects and Preview Mode.
fn advanced(ui: &mut egui::Ui, d: &mut Dialog, label: f32, field: f32) {
    let t = Tokens::get(ui.ctx());
    let open = d.bool("advanced");
    let resp = ui
        .horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::hover());
            crate::icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, r, t.icon);
            ui.label(egui::RichText::new(tl!("Advanced Options")).color(t.text));
        })
        .response
        .interact(egui::Sense::click());
    if resp.clicked() {
        d.fields.insert("advanced".into(), json!(!open));
    }
    if !open {
        return;
    }
    let colors: Vec<(Value, String)> = COLOR_MODES.iter().map(|(v, l)| (json!(v), l.to_string())).collect();
    let ppi: Vec<(Value, String)> = RASTER_PPI.iter().map(|(v, l)| (json!(v), l.to_string())).collect();
    let previews: Vec<(Value, String)> =
        PreviewMode::ALL.iter().map(|m| (json!(m.id()), form::humanize(m.id()).trim_end_matches(':').to_string())).collect();
    for (key, name, options) in [
        ("colorMode", tl!("Color Mode"), &colors),
        ("rasterEffectsPpi", tl!("Raster Effects"), &ppi),
        ("previewMode", tl!("Preview Mode"), &previews),
    ] {
        widgets::label_row(ui, name, label, |ui| choice(ui, d, key, options, field));
    }
}

/// More Settings: the classic form of the same fields.
fn show_more(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    sync(app, &mut d);
    let t = Tokens::get(ctx);
    let mut b = Buttons::default();
    let mut templates = false;
    const L: f32 = 150.0;
    window(ctx, MORE, 22, |ui| {
        ui.set_width(560.0);
        ui.label(egui::RichText::new(tl!("More Settings")).font(theme::semibold(16.0)).color(t.text));
        ui.add_space(12.0);
        widgets::label_row(ui, tl!("Name:"), L, |ui| {
            form::text(ui, &mut d, "name", 300.0);
        });
        // Profile: the preset categories (the tabs); Size: the profile's presets.
        let profiles: Vec<&str> = newdoc::CATEGORIES.iter().map(|c| c.0).collect();
        let profile = profiles.iter().find(|p| p.eq_ignore_ascii_case(&d.str("category"))).copied().unwrap_or("[Custom]");
        widgets::label_row(ui, tl!("Profile:"), L, |ui| {
            if let Some(p) = widgets::dropdown(ui, "newdoc-profile", profile, &profiles, 220.0).and_then(|i| profiles.get(i)) {
                d.fields.insert("category".into(), json!(p));
                if let Some(first) = newdoc::category(&app.session, p).and_then(|v| v.into_iter().next()) {
                    apply(&mut d, &first);
                }
            }
        });
        ui.add_space(6.0);
        let (lay, cols, spacing, rtl) = layout(&d);
        let n = d.f64("artboards", 1.0);
        let several = n > 1.0;
        widgets::label_row(ui, tl!("Number of Artboards:"), L, |ui| {
            artboards(ui, &mut d, 76.0);
            ui.add_space(10.0);
            for (l, icon) in [
                (ArtboardLayout::GridByRow, "dc-grid-view"),
                (ArtboardLayout::GridByColumn, "grid-3x3"),
                (ArtboardLayout::Row, "arrow-left-right"),
                (ArtboardLayout::Column, "arrow-up-down"),
            ] {
                if widgets::icon_button_enabled(ui, icon, l.label(), lay == l, several, 26.0).clicked() {
                    set_layout(&mut d, "layout", json!(l.id()));
                }
            }
            ui.add_space(6.0);
            if widgets::icon_button_enabled(ui, "chevrons-left", tl!("Change to Right-to-Left Layout"), rtl, several, 26.0).clicked() {
                set_layout(&mut d, "rightToLeft", json!(!rtl));
            }
        });
        ui.add_enabled_ui(several, |ui| {
            widgets::label_row(ui, tl!("Spacing:"), L, |ui| {
                if let Some(v) = widgets::num_field(ui, "newdoc-spacing", Some(spacing), unit(&d), 100.0) {
                    set_layout(&mut d, "spacing", json!(v.max(0.0)));
                }
                ui.add_space(20.0);
                ui.label(egui::RichText::new(tl!("Columns:")).color(t.text));
                let grid = matches!(lay, ArtboardLayout::GridByRow | ArtboardLayout::GridByColumn);
                ui.add_enabled_ui(grid, |ui| {
                    if let Some(v) = widgets::spin_plain(ui, "newdoc-columns", cols as f64, "", 0, 76.0, 1.0, 1.0, &[]) {
                        set_layout(&mut d, "columns", json!(v.round().clamp(1.0, n) as u64));
                    }
                });
            });
        });
        ui.add_space(6.0);
        let sizes = newdoc::category(&app.session, profile).unwrap_or_default();
        let size_names: Vec<&str> = sizes.iter().map(|s| s.name.as_str()).collect();
        let preset = d.str("preset");
        widgets::label_row(ui, tl!("Size:"), L, |ui| {
            // The profile's sizes are built in; the preset may be one the user saved.
            let shown = if preset.is_empty() { tl!("Custom") } else { preset_name(&preset) };
            if let Some(s) = super::mixed_dropdown(ui, "newdoc-size", shown, &size_names, 220.0, |_| true).and_then(|i| sizes.get(i)) {
                apply(&mut d, s);
            }
        });
        let unit = unit(&d);
        widgets::label_row(ui, tl!("Width:"), L, |ui| {
            form::length(ui, &mut d, "width", unit, 120.0);
            ui.add_space(20.0);
            ui.label(egui::RichText::new(tl!("Units:")).color(t.text));
            units_dropdown(ui, &mut d, 120.0);
        });
        widgets::label_row(ui, tl!("Height:"), L, |ui| {
            form::length(ui, &mut d, "height", unit, 120.0);
            ui.add_space(20.0);
            ui.label(egui::RichText::new(tl!("Orientation:")).color(t.text));
            orientation(ui, &mut d);
        });
        ui.add_space(4.0);
        widgets::label_row(ui, tl!("Bleed:"), L, |ui| form::bleed(ui, &mut d, unit, 64.0));
        ui.add_space(4.0);
        widgets::label_row(ui, tl!("Background Contents:"), L, |ui| background(ui, &mut d, 220.0));
        ui.add_space(8.0);
        advanced(ui, &mut d, L, 220.0);
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            templates = widgets::secondary_button(ui, tl!("Templates…")).on_hover_text(tl!("New from Template")).clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                b.create = widgets::primary_button(ui, tl!("Create Document")).clicked();
                ui.add_space(8.0);
                // Cancel goes back to New Document.
                if widgets::secondary_button(ui, tl!("Cancel")).clicked() {
                    d.kind = KIND.into();
                }
            });
        });
    });
    if templates {
        app.ui.dialog = None;
        if let Err(e) = app.run("file.newFromTemplate", json!({})) {
            app.status(e);
        }
        return;
    }
    b.finish(app, ctx, d);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    /// Cards translate the built-in presets' names only: Recent and Saved show the user's names.
    #[test]
    fn only_built_in_preset_names_are_translated() {
        assert!(super::is_builtin_preset("Letter") && super::is_builtin_preset("Phone 390×844"));
        assert!(!super::is_builtin_preset("Black") && !super::is_builtin_preset("Untitled-1"));
        assert_eq!(super::preset_name("Black"), "Black");
    }

    fn frame(app: &mut VectorcraftApp) -> egui::FullOutput {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1000.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        out
    }

    fn set(app: &mut VectorcraftApp, k: &str, v: serde_json::Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
    }

    fn field(app: &VectorcraftApp, k: &str) -> serde_json::Value {
        app.ui.dialog.as_ref().unwrap().fields.get(k).cloned().unwrap_or_default()
    }

    #[test]
    fn a_preset_fills_the_details_and_create_makes_the_document() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.newDialog", json!({})).unwrap();
        frame(&mut app);
        assert_eq!((field(&app, "category"), field(&app, "preset"), field(&app, "name")), (json!("Print"), json!("Letter"), json!("Untitled-1")));
        // Choosing a preset (as `ui.dialog.set` does) fills the fields.
        set(&mut app, "category", json!("Film & Video"));
        set(&mut app, "preset", json!("4K UHD"));
        frame(&mut app);
        assert_eq!((field(&app, "width"), field(&app, "units"), field(&app, "colorMode")), (json!(3840.0), json!("Pixels"), json!("rgb")));
        set(&mut app, "artboards", json!(4));
        set(&mut app, "artboardLayout", json!({"layout": "gridByRow", "columns": 2, "spacing": 10}));
        set(&mut app, "bleed", json!([0, 0, 0, 0]));
        set(&mut app, "previewMode", json!("pixel"));
        set(&mut app, "backgroundContents", json!("white"));
        frame(&mut app);
        assert_eq!(field(&app, "preset"), json!(""), "editing makes the settings custom");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        let rects: Vec<_> = st.doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0)).collect();
        assert_eq!(rects, [(0.0, 0.0), (3850.0, 0.0), (0.0, 2170.0), (3850.0, 2170.0)]);
        assert_eq!(st.doc.setup.background, vectorcraft_doc::Background::White);
        assert!(app.ui.view.pixel_preview, "Pixel preview mode");
        // Next time Recent opens on it.
        app.run("file.newDialog", json!({})).unwrap();
        assert_eq!((field(&app, "category"), field(&app, "name")), (json!("Recent"), json!("Untitled-2")));
        frame(&mut app);
    }

    #[test]
    fn every_category_and_more_settings_draw_and_save_preset_saves() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.newDialog", json!({})).unwrap();
        set(&mut app, "advanced", json!(true));
        for c in vectorcraft_engine::cmd::newdoc::category_names() {
            set(&mut app, "category", json!(c));
            let out = frame(&mut app);
            assert!(!out.shapes.is_empty());
        }
        // More Settings shares the fields; Cancel goes back.
        app.ui.dialog.as_mut().unwrap().kind = super::MORE.into();
        set(&mut app, "artboards", json!(3));
        frame(&mut app);
        assert_eq!(field(&app, "artboards"), json!(3));
        // Save Preset.
        set(&mut app, "category", json!("Print"));
        set(&mut app, "preset", json!("A5"));
        frame(&mut app);
        let p = app.run("file.newPresets.save", json!({"name": "Zine", "preset": "A5"}));
        assert!(p.is_ok());
        app.ui.dialog.as_mut().unwrap().kind = super::KIND.into();
        set(&mut app, "category", json!("Saved"));
        set(&mut app, "preset", json!("Zine"));
        frame(&mut app);
        assert!((field(&app, "width").as_f64().unwrap() - 148.0 * 72.0 / 25.4).abs() < 1e-6, "A5 wide");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.prefs.recent_new_docs[0].name, "Zine");
    }
}
