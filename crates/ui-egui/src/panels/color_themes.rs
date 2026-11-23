//! Color Themes panel (local only): the Create tab makes a five-colour theme on a harmony wheel (a
//! harmony rule from the selected colour, or Custom: each colour moves on its own; with a rule the
//! colours move together; the selected color is also edited in its hex field or with CMYK, RGB, Lab
//! or HSB sliders), the My Themes tab lists the themes saved in the preferences (the panel
//! menu's Show Find Field adds a field that lists only the themes whose names hold its text). Themes are
//! saved, edited, deleted and added to Swatches as colour groups through `colorTheme.*`; the theme
//! being made is panel state (`ThemesUi`).

// PLAN: the module paths of the craft_ui / craft_color_ui / craft_theme re-exports below follow the
// framework crates' layout (racheck-fw); none of them is imported anywhere else in the app yet.
use std::borrow::Cow;

use craft::behavior::{DisplayOrder, IconId, ItemSource, Selection, SelectionChange};
use craft::color_ui::ui::{ChannelRow, ColorPanelMode, HarmonyWheel};
use craft::theme::variant::Tone;
use craft::theme::{ModelValues, Rgba};
use craft::ui::buttons::IconButton;
use craft::ui::choice::{SwatchSpacing, SwatchTable};
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _};
use craft::ui::list::{List, ListRow, ThumbSize};
use craft::ui::menu::PanelRow;
use craft::ui::notices::Notice;
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::Label;
use craft::ui::sections::{FooterAction, FooterBar};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::{HexField, TextField};
use egui::{Rect, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::harmony::{self, Harmony, THEME_SIZE, move_on_wheel};
use vectorcraft_engine::cmd::colortheme::ColorTheme;

use super::color::{color_of_values, working_rgb};
use super::{active_paint, c32, color_json, proxy_cmd, srgba};
use crate::VectorcraftApp;

/// The rule a new theme starts with.
const DEFAULT_RULE: Harmony = Harmony::Analogous2;
/// The base colour before one is picked: a teal.
const DEFAULT_BASE: Color = Color::Rgb { r: 0.1, g: 0.55, b: 0.6 };
/// The color modes of the selected color's sliders, in the order the Color Mode menu lists them.
const SLIDER_MODES: [ColorPanelMode; 4] =
    [ColorPanelMode::CmykSliders, ColorPanelMode::RgbSliders, ColorPanelMode::LabSliders, ColorPanelMode::HsbSliders];
/// Colours a slider's track shows between its ends.
const TRACK_STOPS: usize = 7;

/// The tabs, by their index in `ThemesUi::tab`.
const CREATE: usize = 0;
const MY_THEMES: usize = 1;

/// The panel menu's and footers' commands.
const SAVE: &str = "ui.colorThemes.save";
const ADD_TO_SWATCHES: &str = "ui.colorThemes.addToSwatches";
const EDIT: &str = "ui.colorThemes.edit";
const DELETE: &str = "ui.colorThemes.delete";
const SHOW_FIND: &str = "ui.colorThemes.showFind";
/// Starts a new theme from the current colour (a footer button of the Create tab only).
const NEW_THEME: &str = "ui.colorThemes.new";

/// The panel's state: the theme being made, the pick on My Themes, the tab, the find field and the
/// sliders' mode and numbers. Lives in `UiState`, so it outlasts the panel's frames.
#[derive(Clone, Debug, Default)]
pub struct ThemesUi {
    pub(crate) draft: Option<Draft>,
    /// The saved theme picked on My Themes; see `picked`.
    pub(crate) picked: Option<String>,
    /// The tab shown: `CREATE` or `MY_THEMES`.
    pub(crate) tab: usize,
    /// Whether the panel menu's Show Find Field shows the find field (off in a new session).
    pub(crate) show_find: bool,
    /// The find field's text, kept while the panel menu hides the field.
    pub(crate) find: String,
    /// The mode the sliders were last switched to; `None` until then.
    pub(crate) mode: Option<ColorPanelMode>,
    pub(crate) held: Option<Held>,
}

impl ThemesUi {
    /// The find field's text, trimmed and lowercased; empty while the field is hidden.
    fn query(&self) -> String {
        if self.show_find { self.find.trim().to_lowercase() } else { String::new() }
    }

    /// Load saved theme `t` into the Create tab to edit it.
    fn edit(&mut self, t: &ColorTheme) {
        let rule = t.rule.as_deref().and_then(harmony::parse);
        self.draft = Some(Draft { colors: t.colors.clone(), rule, sel: 0, name: t.name.clone(), editing: Some(t.name.clone()) });
        self.tab = CREATE;
    }
}

/// The theme being made on the Create tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Draft {
    pub(crate) colors: Vec<Color>,
    /// The harmony rule (`None`: Custom).
    pub(crate) rule: Option<Harmony>,
    /// The selected colour (the base a rule starts from).
    pub(crate) sel: usize,
    pub(crate) name: String,
    /// The saved theme being edited: Save overwrites it.
    pub(crate) editing: Option<String>,
}

impl Draft {
    /// A new theme from `base` by `rule`.
    fn new(base: Color, rule: Option<Harmony>) -> Self {
        let colors = harmony::theme(rule.unwrap_or(DEFAULT_RULE), base);
        Self { colors, rule, ..Default::default() }
    }
    /// Set the selected colour to `c`: with a rule the theme follows from it as the new base.
    fn set_selected(&mut self, c: Color) {
        match self.rule {
            Some(r) => {
                self.colors = harmony::theme(r, c);
                self.sel = 0;
            }
            None => {
                if let Some(x) = self.colors.get_mut(self.sel) {
                    *x = c;
                }
            }
        }
    }
    /// `colorTheme.save` parameters.
    fn save_params(&self) -> Value {
        let mut p = json!({ "colors": colors_json(&self.colors) });
        if let Some(r) = self.rule {
            p["rule"] = json!(r.id());
        }
        if !self.name.trim().is_empty() {
            p["name"] = json!(self.name.trim());
        }
        if let Some(e) = &self.editing {
            p["replace"] = json!(e);
        }
        p
    }
}

/// Colours as command params, exact in their models.
fn colors_json(colors: &[Color]) -> Vec<Value> {
    colors.iter().map(color_json).collect()
}

/// The theme being made: from the current colour when the panel first shows.
fn draft_mut(app: &mut VectorcraftApp) -> &mut Draft {
    if app.ui.color_themes_panel.draft.is_none() {
        let d = Draft::new(active_paint(app).color().unwrap_or(DEFAULT_BASE), Some(DEFAULT_RULE));
        app.ui.color_themes_panel.draft = Some(d);
    }
    app.ui.color_themes_panel.draft.get_or_insert_with(Draft::default)
}

fn draft(app: &mut VectorcraftApp) -> Draft {
    draft_mut(app).clone()
}

fn set_draft(app: &mut VectorcraftApp, d: Draft) {
    app.ui.color_themes_panel.draft = Some(d);
}

/// Whether the find field lists theme `name` for `query` (trimmed and lowercased): its name holds
/// the text anywhere, in any letter case. An empty query lists every theme.
fn matches(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
}

/// The saved themes the My Themes list shows, in their saved order: the ones the find field keeps.
fn shown_themes(app: &VectorcraftApp) -> impl Iterator<Item = &ColorTheme> {
    let q = app.ui.color_themes_panel.query();
    app.session.prefs.color_themes.iter().filter(move |t| matches(&t.name, &q))
}

/// The saved theme picked on the My Themes tab, while the list shows it: a theme that the find
/// field hides stays picked, but nothing acts on it until the list shows it again.
fn picked(app: &VectorcraftApp) -> Option<String> {
    let name = app.ui.color_themes_panel.picked.as_ref()?;
    shown_themes(app).any(|t| t.name == *name).then(|| name.clone())
}

/// The mode of the sliders: the one last picked from their Color Mode menu while the app runs, or
/// else the selected color's own model (HSB for a gray). Picking a mode changes no color.
fn slider_mode(picked: Option<ColorPanelMode>, c: &Color) -> ColorPanelMode {
    picked.unwrap_or(match c {
        Color::Cmyk { .. } => ColorPanelMode::CmykSliders,
        Color::Rgb { .. } => ColorPanelMode::RgbSliders,
        Color::Lab { .. } => ColorPanelMode::LabSliders,
        _ => ColorPanelMode::HsbSliders,
    })
}

/// The slider values a slider edit last set: the selected place and the color they made. While those
/// still hold, the sliders show `values` as they were set, so an edit of one slider leaves the
/// others' numbers as they were instead of converting them back from the color.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Held {
    pub(crate) sel: usize,
    pub(crate) color: Color,
    pub(crate) values: ModelValues,
}

/// The channels of `mode`: the row's letter and range, in order.
fn channels(mode: ColorPanelMode) -> &'static [(&'static str, f64, f64)] {
    match mode {
        ColorPanelMode::CmykSliders => &[("C", 0.0, 100.0), ("M", 0.0, 100.0), ("Y", 0.0, 100.0), ("K", 0.0, 100.0)],
        ColorPanelMode::RgbSliders => &[("R", 0.0, 255.0), ("G", 0.0, 255.0), ("B", 0.0, 255.0)],
        ColorPanelMode::LabSliders => &[("L", 0.0, 100.0), ("a", -128.0, 127.0), ("b", -128.0, 127.0)],
        _ => &[("H", 0.0, 360.0), ("S", 0.0, 100.0), ("B", 0.0, 100.0)],
    }
}

/// The model the sliders of `mode` edit in, as [`ModelValues::model`] names it.
fn model_of(mode: ColorPanelMode) -> &'static str {
    match mode {
        ColorPanelMode::CmykSliders => "cmyk",
        ColorPanelMode::RgbSliders => "rgb",
        ColorPanelMode::LabSliders => "lab",
        _ => "hsb",
    }
}

/// The numbers the sliders of `mode` show for `c`, in the channels' units ([`channels`]).
fn values_in(c: &Color, mode: ColorPanelMode) -> ModelValues {
    let rgb = || working_rgb(c).map(|v| v.clamp(0.0, 1.0));
    let values = match mode {
        ColorPanelMode::CmykSliders => c.to_cmyk().map(|v| v * 100.0).to_vec(),
        ColorPanelMode::RgbSliders => rgb().map(|v| v * 255.0).to_vec(),
        ColorPanelMode::LabSliders => {
            let lab = c.to_lab();
            vec![lab.l, lab.a, lab.b]
        }
        _ => {
            let [r, g, b] = rgb();
            let [h, s, v] = Color::rgb(r, g, b).to_hsb();
            vec![h, s * 100.0, v * 100.0]
        }
    };
    ModelValues::new(model_of(mode), &values)
}

fn mode_label(mode: ColorPanelMode) -> &'static str {
    match mode {
        ColorPanelMode::CmykSliders => tl!("CMYK"),
        ColorPanelMode::RgbSliders => tl!("RGB"),
        ColorPanelMode::LabSliders => tl!("Lab"),
        _ => tl!("HSB"),
    }
}

/// The titles of a strip of tabs or the rows of a list, keyed by position.
struct Names<'a>(&'a [String]);

impl DisplayOrder<usize> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|name| Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for Names<'_> {}

/// A single-choice `Select` over `labels`: the index picked this pass.
fn select(ui: &mut Ui, label: &'static str, current: usize, labels: &[&'static str]) -> Option<usize> {
    let entries: Vec<Entry<usize>> = labels.iter().enumerate().map(|(i, l)| Entry::new(i, Label::from(*l))).collect();
    Select::new(label, Bind::given(current), &entries).show(ui).value().copied()
}

/// The color mode menu (CMYK, RGB, Lab and HSB) and its rows, each a rail and a number field, for the
/// selected color of `d`. An edit sets the selected color in the menu's mode (an RGB or HSB edit makes
/// it RGB, a CMYK edit CMYK and a Lab edit Lab) as the hex field does: with a rule the theme follows
/// from it as the new base.
fn sliders(ui: &mut Ui, d: &mut Draft, mode_pick: &mut Option<ColorPanelMode>, held: &mut Option<Held>) {
    let Some(color) = d.colors.get(d.sel).cloned() else { return };
    let current = slider_mode(*mode_pick, &color);
    let labels: Vec<&'static str> = SLIDER_MODES.iter().map(|m| mode_label(*m)).collect();
    let at = SLIDER_MODES.iter().position(|m| *m == current).unwrap_or(0);
    if let Some(&m) = select(ui, tl!("Color Mode:"), at, &labels).and_then(|i| SLIDER_MODES.get(i)) {
        *mode_pick = Some(m);
    }
    let mode = slider_mode(*mode_pick, &color);
    let shown = held
        .as_ref()
        .filter(|h| h.sel == d.sel && h.color == color && h.values.model == model_of(mode))
        .map_or_else(|| values_in(&color, mode), |h| h.values);
    let mut set: Option<ModelValues> = None;
    for (n, &(letter, lo, hi)) in channels(mode).iter().enumerate() {
        let value = shown.values.get(n).copied().map_or(0.0, f64::from);
        let track: Vec<_> = (0..TRACK_STOPS)
            .map(|i| {
                let mut probe = shown;
                if let Some(x) = probe.values.get_mut(n) {
                    *x = (lo + (hi - lo) * i as f64 / (TRACK_STOPS - 1) as f64) as f32;
                }
                srgba(&color_of_values(&probe))
            })
            .collect();
        let rule = super::whole_steps(lo..=hi);
        let Some(row) = ChannelRow::new(letter, Bind::given(value), rule).short(letter).track(track).show(ui) else { continue };
        for event in row.events {
            if let Some(&nv) = event.value().filter(|v| v.is_finite()) {
                let mut next = set.take().unwrap_or(shown);
                if let Some(x) = next.values.get_mut(n) {
                    *x = nv.clamp(lo, hi) as f32;
                }
                set = Some(next);
            }
        }
    }
    if let Some(values) = set {
        d.set_selected(color_of_values(&values));
        if let Some(&color) = d.colors.get(d.sel) {
            *held = Some(Held { sel: d.sel, color, values });
        }
    }
}

/// A colour as the hex field's value.
fn hex_rgba(c: &Color) -> Rgba {
    let h = c.to_hex();
    let byte = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0);
    Rgba::rgb(byte(1), byte(3), byte(5))
}

/// Save the theme being made (overwriting the theme it edits); it then edits the saved theme.
fn save(app: &mut VectorcraftApp) {
    let params = draft_mut(app).save_params();
    if let Ok(r) = app.run("colorTheme.save", params) {
        let name = r["name"].as_str().unwrap_or_default().to_string();
        let st = &mut app.ui.color_themes_panel;
        st.picked = Some(name.clone());
        if let Some(d) = st.draft.as_mut() {
            d.name = name.clone();
            d.editing = Some(name);
        }
    }
}

/// Add the theme being made to Swatches as a colour group (one undo step).
fn add_draft_to_swatches(app: &mut VectorcraftApp) {
    let d = draft(app);
    let name = if d.name.trim().is_empty() { "Color Theme" } else { d.name.trim() };
    app.run("swatch.newGroup", json!({ "name": name, "colors": colors_json(&d.colors), "keepModel": true })).ok();
}

/// Paint the active proxy (Fill or Stroke) with the selected colour of the theme being made, in its
/// own model: the selection and the new-art default take it in one paint command, so one undo
/// step, and no step when the art already has it. With characters selected with the Type tool,
/// only those characters take it and the new-art default stays as it was, as with a swatch click.
/// The theme stays as it is.
fn set_active_color(app: &mut VectorcraftApp, d: &Draft) {
    if let Some(c) = d.colors.get(d.sel) {
        app.run(proxy_cmd(app, false), json!({ "color": color_json(c), "keepModel": true })).ok();
    }
}

/// Edit saved theme `name` on the Create tab.
fn edit_saved(app: &mut VectorcraftApp, name: &str) {
    if let Some(th) = app.session.prefs.color_themes.iter().find(|t| t.name == name) {
        app.ui.color_themes_panel.edit(th);
    }
}

/// Delete saved theme `name`; the theme being made no longer edits it.
fn delete(app: &mut VectorcraftApp, name: &str) {
    if app.run("colorTheme.delete", json!({ "name": name })).is_ok()
        && let Some(d) = app.ui.color_themes_panel.draft.as_mut().filter(|d| d.editing.as_deref() == Some(name))
    {
        d.editing = None;
    }
}

/// Whether command `id` can run now: Edit, Delete and (on My Themes) Add to Swatches need a shown pick,
/// Add to Swatches a document. The menu rows and the footer buttons read this one rule.
fn enabled(app: &VectorcraftApp, id: &str) -> bool {
    let pick = picked(app).is_some();
    match id {
        ADD_TO_SWATCHES => app.session.active().is_some() && (app.ui.color_themes_panel.tab == CREATE || pick),
        EDIT | DELETE => pick,
        _ => true,
    }
}

/// The rows of the panel menu: Save Theme and Add to Swatches on Create; Edit Theme, Add to Swatches,
/// Delete Theme and Show Find Field on My Themes.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let row = |id: &str| PanelRow::Command(CommandRef::new(id));
    if app.ui.color_themes_panel.tab == MY_THEMES {
        vec![row(EDIT), row(ADD_TO_SWATCHES), row(DELETE), PanelRow::Separator, row(SHOW_FIND)]
    } else {
        vec![row(SAVE), row(ADD_TO_SWATCHES)]
    }
}

/// The enabled and checked state of the panel menu's rows.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for id in [SAVE, ADD_TO_SWATCHES, EDIT, DELETE] {
        states.set(CommandRef::new(id), CommandState::new().enabled(enabled(app, id)));
    }
    states.set(CommandRef::new(SHOW_FIND), CommandState::new().checked(app.ui.color_themes_panel.show_find));
}

/// Run panel command `id`; `None` when it is not this panel's. A command that is off does nothing.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let known = [SAVE, ADD_TO_SWATCHES, EDIT, DELETE, SHOW_FIND, NEW_THEME];
    if !known.contains(&id) {
        return None;
    }
    if enabled(app, id) {
        let pick = picked(app);
        match (id, pick) {
            (SAVE, _) => save(app),
            (ADD_TO_SWATCHES, _) if app.ui.color_themes_panel.tab == CREATE => add_draft_to_swatches(app),
            (ADD_TO_SWATCHES, Some(n)) => {
                app.run("colorTheme.addToSwatches", json!({ "name": n })).ok();
            }
            (EDIT, Some(n)) => edit_saved(app, &n),
            (DELETE, Some(n)) => delete(app, &n),
            (SHOW_FIND, _) => app.ui.color_themes_panel.show_find ^= true,
            (NEW_THEME, _) => {
                let rule = draft_mut(app).rule.or(Some(DEFAULT_RULE));
                set_draft(app, Draft::new(active_paint(app).color().unwrap_or(DEFAULT_BASE), rule));
            }
            _ => {}
        }
    }
    Some(Ok(Value::Null))
}

fn icon(name: &'static str) -> IconSpec<'static> {
    IconSpec::svg(IconId::new(name))
}

/// The footer under a tab: one action per command of `ids`, from the end edge, with `icons`; a click runs
/// the command the same way the panel menu does.
fn footer(app: &mut VectorcraftApp, ui: &mut Ui, ids: &[&str], icons: &[&'static str], labels: Vec<Label>) {
    let actions: Vec<_> =
        ids.iter().zip(icons).zip(labels).map(|((id, name), label)| FooterAction::new(icon(name), label).enabled(enabled(app, id))).collect();
    let out = FooterBar::actions(actions).show_footer(ui);
    // The footer runs twice per pass: the sizing pass measures it and must report nothing.
    if ui.is_sizing_pass() {
        return;
    }
    if let Some(&id) = out.picked.and_then(|i| ids.get(i)) {
        run(app, id, &Value::Null);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let titles = [tl!("Create").to_string(), tl!("My Themes").to_string()];
    let cur = app.ui.color_themes_panel.tab;
    let strip = TabStrip::new(tl!("Color Themes"))
        .kind(TabKind::Text)
        .shown(cur)
        .show_items(ui, &Names(&titles), &mut |key, _| Tab::new(titles.get(*key).cloned().unwrap_or_default()).id_salt(("ct-tab", *key)));
    for event in strip.response.events {
        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
            && let Some(&tab) = keys.last()
        {
            app.ui.color_themes_panel.tab = tab;
        }
    }
    if app.ui.color_themes_panel.tab == MY_THEMES { my_themes(app, ui) } else { create(app, ui) }
}

fn create(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut d = draft(app);
    let before = d.clone();
    let (mut mode, mut held) = (app.ui.color_themes_panel.mode, app.ui.color_themes_panel.held.clone());
    let labels: Vec<&str> = std::iter::once("Custom").chain(Harmony::ALL.into_iter().map(harmony::label)).collect();
    let at = d.rule.and_then(|r| Harmony::ALL.iter().position(|h| *h == r)).map_or(0, |i| i + 1);
    // A rule makes the theme from the selected colour; Custom frees the colours.
    if let Some(i) = select(ui, tl!("Rule:"), at, &labels) {
        d.rule = i.checked_sub(1).and_then(|i| Harmony::ALL.get(i).copied());
        if let (Some(r), Some(&base)) = (d.rule, d.colors.get(d.sel)) {
            d.colors = harmony::theme(r, base);
            d.sel = 0;
        }
    }
    let shown: Vec<_> = d.colors.iter().map(srgba).collect();
    let wheel = HarmonyWheel::new(tl!("Harmony wheel"), &shown).selected_marker(Some(d.sel)).show(ui);
    if let Some(k) = wheel.pressed {
        d.sel = k;
    }
    for (k, event) in wheel.edits {
        if let Some(set) = event.value()
            && let Some(cur) = d.colors.get(k).map(Color::to_hsb)
        {
            let hsb = [0, 1, 2].map(|n| set[n].unwrap_or(cur[n]));
            move_on_wheel(&mut d.colors, k, hsb, d.rule.is_some());
        }
    }
    // The theme's colours: click one to select it.
    let chips: Vec<_> = d.colors.iter().map(|c| (Label::from(c.to_hex()), srgba(c))).collect();
    let mut chosen = Selection::single(d.sel);
    // The selection comes back through `chosen`.
    let _ = SwatchTable::new(tl!("Theme colors"), &chips)
        .per_row(THEME_SIZE)
        .spacing(SwatchSpacing::Spaced)
        .selection(Bind::bound(&mut chosen))
        .show_all(ui);
    if let Some(&i) = chosen.items.last() {
        d.sel = i;
    }
    sliders(ui, &mut d, &mut mode, &mut held);
    ui.horizontal(|ui| {
        let hex = d.colors.get(d.sel).map(hex_rgba).unwrap_or(Rgba::rgb(0, 0, 0));
        let mut edited = hex;
        HexField::hex(tl!("Color"), Bind::bound(&mut edited)).show(ui);
        // PLAN: assumed `Rgba::to_hex() -> String` ("#rrggbb"), as `Srgba::to_hex`.
        if edited != hex
            && let Some(c) = Color::from_hex(&edited.to_hex())
        {
            d.set_selected(c);
        }
        let can_set = app.session.active().is_some() && d.colors.get(d.sel).is_some();
        if IconButton::new(icon("paint-bucket")).label(tl!("Set the current fill or stroke to the selected color")).enabled(can_set).show(ui).clicks
            == 1
        {
            set_active_color(app, &d);
        }
        let current = active_paint(app).color();
        if IconButton::new(icon("pipette")).label(tl!("Set the selected color to the current color")).enabled(current.is_some()).show(ui).clicks == 1
            && let Some(c) = current
        {
            d.set_selected(c);
        }
    });
    TextField::new(tl!("Name:"), Bind::bound(&mut d.name)).show(ui);
    let st = &mut app.ui.color_themes_panel;
    st.mode = mode;
    st.held = held;
    if d != before {
        set_draft(app, d.clone());
    }
    // The label is translated around the theme's name, not looked up whole.
    let save_label = match &d.editing {
        Some(n) => crate::i18n::fmt(tl!("Save changes to {name}"), &[("name", n)]).to_string(),
        None => tl!("Save theme to My Themes").to_string(),
    };
    let labels = vec![Label::from(tl!("New theme from the current color")), Label::from(tl!("Add to Swatches")), Label::from(save_label)];
    footer(app, ui, &[NEW_THEME, ADD_TO_SWATCHES, SAVE], &["file-plus", "dc-folder", "save"], labels);
}

fn my_themes(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.ui.color_themes_panel.show_find {
        let mut find = app.ui.color_themes_panel.find.clone();
        TextField::search(tl!("Find"), Bind::bound(&mut find)).show(ui);
        app.ui.color_themes_panel.find = find;
    }
    let rows: Vec<(String, Vec<Color>)> = shown_themes(app).map(|t| (t.name.clone(), t.colors.clone())).collect();
    if app.session.prefs.color_themes.is_empty() {
        Notice::new(Tone::Neutral, tl!("No saved themes")).lines([tl!("Make a theme on Create and save it.")]).show(ui);
    } else if rows.is_empty() {
        Notice::new(Tone::Neutral, tl!("No matches.")).show(ui);
    } else {
        let names: Vec<String> = rows.iter().map(|(n, _)| n.clone()).collect();
        let mut selection = app
            .ui
            .color_themes_panel
            .picked
            .as_ref()
            .and_then(|p| names.iter().position(|n| n == p))
            .map_or_else(Selection::default, Selection::single);
        let out = List::new(tl!("My Themes")).select(craft::ui::contract::SelectMode::Single).selection(Bind::bound(&mut selection)).show_items(
            ui,
            &Names(&names),
            &mut |key, _| {
                let colors = rows.get(*key).map(|(_, c)| c.clone()).unwrap_or_default();
                let name = names.get(*key).cloned().unwrap_or_default();
                ListRow::new(name).thumb(ThumbSize::Small, move |cx, rect| {
                    let w = rect.width() / colors.len().max(1) as f32;
                    cx.data_shapes(colors.iter().enumerate().map(|(i, c)| {
                        egui::Shape::rect_filled(Rect::from_min_size(rect.min + vec2(i as f32 * w, 0.0), vec2(w, rect.height())), 0.0, c32(c))
                    }));
                })
            },
        );
        for event in out.events {
            match event {
                CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                    if let Some(name) = keys.last().and_then(|k| names.get(*k)) {
                        app.ui.color_themes_panel.picked = Some(name.clone());
                    }
                }
                // Enter and a double click edit the theme.
                CollectionEvent::Activate(key) => {
                    if let Some(name) = names.get(key) {
                        edit_saved(app, name);
                    }
                }
                _ => {}
            }
        }
    }
    let labels = vec![Label::from(tl!("Delete theme")), Label::from(tl!("Add to Swatches")), Label::from(tl!("Edit theme"))];
    footer(app, ui, &[DELETE, ADD_TO_SWATCHES, EDIT], &["trash-2", "dc-folder", "pencil"], labels);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use craft::ui::contract::Role;
    use craft::ui::testing::Fixture;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app
    }

    fn themes(app: &VectorcraftApp) -> &[ColorTheme] {
        &app.session.prefs.color_themes
    }

    /// The panel over `app` in a fixture, settled; the app stays reachable through the `Rc`.
    fn fixture(app: &Rc<RefCell<VectorcraftApp>>) -> Fixture {
        let shared = Rc::clone(app);
        let mut fx = Fixture::new(move |ui| show(&mut shared.borrow_mut(), ui));
        fx.room(vec2(300.0, 700.0)).settle();
        fx
    }

    /// The slider rails, top to bottom (the wheel's Brightness rail first). The wheel's colour markers are
    /// sliders too, but square: a rail is wider than tall.
    fn rails(fx: &Fixture) -> Vec<craft::ui::testing::Target> {
        let mut rails: Vec<_> = fx.by_role(Role::SLIDER).into_iter().filter(|t| t.rect.width() > 2.0 * t.rect.height()).collect();
        rails.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
        rails
    }

    /// Three saved themes and the find field holding `text`, on My Themes.
    fn three_themes(text: &str) -> VectorcraftApp {
        let mut app = app();
        for (name, color) in [("Harbor", "#2266aa"), ("Harvest", "#cc8822"), ("Forest", "#227744")] {
            app.run("colorTheme.save", json!({"color": color, "rule": "complementary", "name": name})).unwrap();
        }
        let st = &mut app.ui.color_themes_panel;
        (st.tab, st.show_find, st.find) = (MY_THEMES, true, text.to_string());
        app
    }

    fn shown_names(app: &VectorcraftApp) -> Vec<&str> {
        shown_themes(app).map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn create_save_edit_and_resave_a_theme() {
        let mut app = app();
        app.run("paint.setFill", json!({"color": "#cc4422"})).unwrap();
        let d = draft(&mut app);
        assert_eq!(d.colors, harmony::theme(DEFAULT_RULE, Color::from_hex("#cc4422").unwrap()), "a theme from the current colour");
        save(&mut app);
        assert_eq!(themes(&app).len(), 1);
        assert_eq!((themes(&app)[0].name.as_str(), &themes(&app)[0].colors), ("Theme 1", &d.colors));
        assert_eq!(themes(&app)[0].rule.as_deref(), Some("analogous2"));
        assert_eq!(draft(&mut app).editing.as_deref(), Some("Theme 1"), "it now edits the saved theme");
        // Another rule from the selected colour, then save: the same theme changes in place.
        let mut d = draft(&mut app);
        d.sel = 2;
        let base = d.colors[2];
        d.rule = Some(Harmony::Triad);
        d.set_selected(base);
        assert_eq!((d.colors.clone(), d.sel), (harmony::theme(Harmony::Triad, base), 0));
        set_draft(&mut app, d.clone());
        save(&mut app);
        assert_eq!(themes(&app).len(), 1, "saving an edited theme overwrites it");
        assert_eq!(themes(&app)[0].colors, d.colors);
        // Custom: only the selected colour changes; a new name saves a new theme.
        let mut d = Draft { rule: None, sel: 1, editing: None, name: "Mine".into(), ..d };
        d.set_selected(Color::from_hex("#00ff00").unwrap());
        assert_eq!(d.colors[1].to_hex(), "#00ff00");
        assert_eq!(d.colors[0], themes(&app)[0].colors[0]);
        set_draft(&mut app, d);
        save(&mut app);
        assert_eq!(themes(&app).iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Theme 1", "Mine"]);
        assert!(themes(&app)[1].rule.is_none());
        assert_eq!(picked(&app).as_deref(), Some("Mine"), "the theme just saved is picked");
    }

    #[test]
    fn my_themes_edit_add_to_swatches_and_delete() {
        let mut app = app();
        app.ui.color_themes_panel.tab = MY_THEMES;
        app.run("colorTheme.save", json!({"color": "#2266aa", "rule": "complementary", "name": "Harbor"})).unwrap();
        app.ui.color_themes_panel.picked = Some("Harbor".to_string());
        edit_saved(&mut app, "Harbor");
        assert_eq!(app.ui.color_themes_panel.tab, CREATE, "editing shows the Create tab");
        let d = draft(&mut app);
        assert_eq!((d.rule, d.editing.as_deref(), d.colors.len()), (Some(Harmony::Complementary), Some("Harbor"), 5));
        // Add to Swatches (the picked saved theme), one colour group in one undo step.
        let undo = undo_len(&app);
        app.run("colorTheme.addToSwatches", json!({"name": "Harbor"})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        assert_eq!(st.doc.swatch_groups.last().map(|g| (g.name.as_str(), g.swatches.len())), Some(("Harbor", 5)));
        // The theme being made goes to Swatches as it is.
        add_draft_to_swatches(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.swatch_groups.last().map(|g| g.name.as_str()), Some("Harbor 2"));
        // Deleting it frees the theme being made from editing it.
        delete(&mut app, "Harbor");
        assert!(themes(&app).is_empty());
        assert_eq!(draft(&mut app).editing, None);
        assert_eq!(picked(&app), None);
    }

    /// The find field lists the themes whose names hold its text, in any letter case, anywhere in
    /// the name, in their saved order; it reads names only, and its text is plain text.
    #[test]
    fn the_find_field_lists_themes_whose_names_hold_its_text() {
        let mut app = three_themes("  HAR ");
        assert_eq!(shown_names(&app), ["Harbor", "Harvest"]);
        app.ui.color_themes_panel.find = "vest".into();
        assert_eq!(shown_names(&app), ["Harvest"], "a match in the middle of a name");
        // Colours are not searched: no name holds "2266aa".
        app.ui.color_themes_panel.find = "2266aa".into();
        assert!(shown_names(&app).is_empty());
        assert_eq!(themes(&app).len(), 3, "the saved themes stay");
        // Pattern characters are plain text, and letters outside ASCII match in either case.
        app.run("colorTheme.save", json!({"color": "#aa2266", "rule": "complementary", "name": "Été a.b*"})).unwrap();
        app.ui.color_themes_panel.find = "ÉTÉ A.B*".into();
        assert_eq!(shown_names(&app), ["Été a.b*"]);
        app.ui.color_themes_panel.find = ".*".into();
        assert!(shown_names(&app).is_empty());
        // Blank text, or the field turned off, lists every theme; turned on again the text applies.
        app.ui.color_themes_panel.find = "   ".into();
        assert_eq!(shown_names(&app).len(), 4);
        app.ui.color_themes_panel.find = "har".into();
        app.ui.color_themes_panel.show_find = false;
        assert_eq!(shown_names(&app).len(), 4);
        app.ui.color_themes_panel.show_find = true;
        assert_eq!(shown_names(&app), ["Harbor", "Harvest"]);
    }

    /// A picked theme that the find field hides stays picked, but neither the menu nor the footer
    /// acts on it until the list shows it again.
    #[test]
    fn a_theme_the_find_field_hides_is_not_acted_on() {
        let mut app = three_themes("har");
        app.ui.color_themes_panel.picked = Some("Forest".to_string());
        assert_eq!(picked(&app), None);
        let _ = run(&mut app, DELETE, &Value::Null);
        assert_eq!(themes(&app).len(), 3, "Delete Theme is off");
        let _ = run(&mut app, EDIT, &Value::Null);
        assert_eq!(app.ui.color_themes_panel.tab, MY_THEMES, "Edit Theme is off");
        let undo = undo_len(&app);
        let _ = run(&mut app, ADD_TO_SWATCHES, &Value::Null);
        assert_eq!(undo_len(&app), undo, "Add to Swatches is off");
        // Cleared, the field lists Forest again and it is picked as before.
        app.ui.color_themes_panel.find.clear();
        assert_eq!(picked(&app).as_deref(), Some("Forest"));
        app.ui.color_themes_panel.find = "har".into();
        app.ui.color_themes_panel.show_find = false;
        assert_eq!(picked(&app).as_deref(), Some("Forest"), "the field turned off lists it too");
        app.ui.color_themes_panel.show_find = true;
        // A theme the field shows is acted on.
        app.ui.color_themes_panel.picked = Some("Harvest".to_string());
        let _ = run(&mut app, DELETE, &Value::Null);
        assert_eq!(themes(&app).iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Harbor", "Forest"]);
    }

    /// The menu rows and the footer buttons are off for a picked theme that the find field hides.
    #[test]
    fn footer_buttons_are_off_for_a_hidden_pick() {
        let mut app = three_themes("har");
        app.ui.color_themes_panel.picked = Some("Forest".to_string());
        let on = |app: &VectorcraftApp, id: &str| {
            let mut states = CommandStates::new();
            menu_states(app, &mut states);
            // PLAN: assumed `CommandState::is_enabled()` (the Fixture-free read of a state).
            states.get(&CommandRef::new(id)).is_some_and(|s| s.is_enabled())
        };
        for id in [EDIT, ADD_TO_SWATCHES, DELETE] {
            assert!(!on(&app, id), "{id} with the pick hidden");
        }
        app.ui.color_themes_panel.find = "for".into();
        for id in [EDIT, ADD_TO_SWATCHES, DELETE] {
            assert!(on(&app, id), "{id} with the pick shown");
        }
    }

    /// A Custom theme of five different colours with the third selected.
    fn five_colors(app: &mut VectorcraftApp) -> Draft {
        let colors = ["#cc2200", "#22aa44", "#3355dd", "#ddbb11", "#884499"].map(|h| Color::from_hex(h).unwrap()).to_vec();
        let d = Draft { colors, rule: None, sel: 2, name: "Mine".into(), editing: None };
        set_draft(app, d.clone());
        d
    }

    /// The fill and stroke paints of object `id`.
    fn paints(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> (vectorcraft_color::Paint, vectorcraft_color::Paint) {
        let n = app.session.doc().unwrap().doc.node(id).unwrap();
        (n.appearance.fill().unwrap().paint.clone(), n.appearance.stroke().unwrap().paint.clone())
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// Set Active Color paints the active proxy of the selection with the selected theme colour,
    /// one undo step a click, none when the art already has it, and leaves the theme as it is.
    #[test]
    fn set_active_color_paints_the_active_proxy_with_the_selected_color() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = app.session.doc().unwrap().selection.objects[0];
        let d = five_colors(&mut app);
        let (_, stroke) = paints(&app, id);
        let undo = undo_len(&app);
        set_active_color(&mut app, &d);
        assert_eq!(paints(&app, id), (vectorcraft_color::Paint::solid(d.colors[2]), stroke.clone()), "the selected colour, not the first");
        assert_eq!(undo_len(&app), undo + 1, "one undo step");
        set_active_color(&mut app, &d);
        assert_eq!(undo_len(&app), undo + 1, "no step when the fill already has the colour");
        // With the Stroke proxy in front, the stroke takes the colour and the fill stays.
        app.run("paint.toggleActive", json!({"fill": false})).unwrap();
        let d3 = Draft { sel: 3, ..d.clone() };
        set_draft(&mut app, d3.clone());
        set_active_color(&mut app, &d3);
        let solid = |i: usize| vectorcraft_color::Paint::solid(d.colors[i]);
        assert_eq!(paints(&app, id), (solid(2), solid(3)));
        assert_eq!(undo_len(&app), undo + 2);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(paints(&app, id), (solid(2), stroke.clone()), "one undo takes the stroke back");
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(paints(&app, id), (solid(2), solid(3)));
        // The theme being made and the saved themes stay as they were.
        assert_eq!(draft(&mut app), d3);
        assert!(themes(&app).is_empty());
    }

    /// With nothing selected, Set Active Color sets the new-art default alone, without an undo step,
    /// and new art takes it.
    #[test]
    fn set_active_color_with_nothing_selected_sets_the_default_for_new_art() {
        let mut app = app();
        let d = five_colors(&mut app);
        let undo = undo_len(&app);
        set_active_color(&mut app, &d);
        assert_eq!(app.session.paint.fill, vectorcraft_color::Paint::solid(d.colors[2]));
        assert_eq!(undo_len(&app), undo, "no art changed");
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = app.session.doc().unwrap().selection.objects[0];
        assert_eq!(paints(&app, id).0, vectorcraft_color::Paint::solid(d.colors[2]));
    }

    /// In a CMYK document Set Active Color keeps an RGB theme colour in RGB.
    #[test]
    fn set_active_color_keeps_the_model_in_a_cmyk_document() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = app.session.doc().unwrap().selection.objects[0];
        let d = five_colors(&mut app);
        set_active_color(&mut app, &d);
        assert_eq!(paints(&app, id).0.color(), Some(d.colors[2]));
        assert!(matches!(d.colors[2], Color::Rgb { .. }));
    }

    /// With characters selected with the Type tool, Set Active Color paints those characters as a
    /// swatch click does: the type and the undo history come out as they do from `paint.setFill` with
    /// the same color, and the new-art default stays as it was.
    #[test]
    fn set_active_color_paints_selected_characters_as_a_swatch_click_does() {
        use vectorcraft_doc::{NodeId, NodeKind};
        // The editing handshake of text_styles::tests::select_type_range.
        let typed = || {
            let mut app = app();
            let id = NodeId(app.run("text.create", json!({"x": 10, "y": 50, "text": "ABcdef", "size": 18})).unwrap()["id"].as_u64().unwrap());
            app.session.select_tool("type", Default::default()).unwrap();
            app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
            app.session.set_tool_option("select", &json!({"start": 2, "end": 4}));
            super::super::character::end_typing(&mut app);
            assert_eq!(super::super::character::text_editing(&app), Some((id, 2, 4)));
            (app, id)
        };
        let runs = |app: &VectorcraftApp, id: NodeId| match &app.session.doc().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Text(t) => t.runs.clone(),
            _ => panic!("not type"),
        };
        let (mut by_button, id) = typed();
        let d = five_colors(&mut by_button);
        let undo = undo_len(&by_button);
        let default = (by_button.session.paint.fill.clone(), by_button.session.paint.stroke.clone());
        assert_ne!(default.0, vectorcraft_color::Paint::solid(d.colors[2]));
        set_active_color(&mut by_button, &d);
        let (mut by_command, _) = typed();
        by_command.run("paint.setFill", json!({"color": color_json(&d.colors[2]), "keepModel": true})).unwrap();
        let painted = runs(&by_button, id);
        assert!(painted.iter().any(|r| r.style.fill == vectorcraft_color::Paint::solid(d.colors[2])), "the characters take the color");
        assert!(painted.iter().any(|r| r.style.fill != vectorcraft_color::Paint::solid(d.colors[2])), "the other characters keep theirs");
        assert_eq!(painted, runs(&by_command, id));
        assert_eq!((&by_button.session.paint.fill, &by_button.session.paint.stroke), (&default.0, &default.1), "the default stays");
        assert_eq!(
            (&by_button.session.paint.fill, &by_button.session.paint.stroke),
            (&by_command.session.paint.fill, &by_command.session.paint.stroke)
        );
        assert_eq!((undo_len(&by_button), undo_len(&by_command)), (undo + 1, undo + 1));
    }

    /// In a CMYK document the theme being made goes to Swatches with its colors in their own model.
    #[test]
    fn the_theme_being_made_keeps_its_model_in_a_cmyk_document() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("colorTheme.save", json!({"color": "#2266aa", "rule": "complementary", "name": "Harbor"})).unwrap();
        edit_saved(&mut app, "Harbor");
        let colors = draft(&mut app).colors;
        assert!(colors.iter().all(|c| matches!(c, Color::Rgb { .. })), "{colors:?}");
        add_draft_to_swatches(&mut app);
        let g = app.session.doc().unwrap().doc.swatch_groups.last().expect("the group");
        assert_eq!(g.swatches.iter().filter_map(|w| w.paint.color()).collect::<Vec<_>>(), colors);
    }

    /// In Custom, a channel row's edit changes the selected color alone, in the mode's model, and the
    /// selection stays; the other numbers stay as they were set.
    #[test]
    fn a_slider_edit_changes_only_the_selected_custom_color() {
        let app = Rc::new(RefCell::new(app()));
        let d = five_colors(&mut app.borrow_mut());
        app.borrow_mut().ui.color_themes_panel.mode = Some(ColorPanelMode::RgbSliders);
        let mut fx = fixture(&app);
        let g_rail = rails(&fx)[2];
        fx.click(&g_rail).settle();
        let now = draft(&mut app.borrow_mut());
        assert_ne!(now.colors[2], d.colors[2], "the selected color changed");
        assert_eq!((now.sel, now.rule), (2, None));
        for i in [0, 1, 3, 4] {
            assert_eq!(now.colors[i], d.colors[i], "color {i} stays");
        }
        let held = app.borrow().ui.color_themes_panel.held.clone().expect("held");
        assert_eq!(held.values.model, "rgb");
        // The arrow keys step the focused rail by one; the other numbers stay as set.
        fx.key(egui::Key::ArrowRight, egui::Modifiers::NONE).settle();
        let after = app.borrow().ui.color_themes_panel.held.clone().expect("held");
        assert_eq!(after.values.values[1], held.values.values[1] + 1.0);
        assert_eq!((after.values.values[0], after.values.values[2]), (held.values.values[0], held.values.values[2]));
    }

    /// The sliders keep the numbers they set: an HSB color taken to no saturation keeps its hue on
    /// the H row, and S back up brings that hue back.
    #[test]
    fn the_sliders_keep_the_numbers_they_set() {
        let app = Rc::new(RefCell::new(app()));
        five_colors(&mut app.borrow_mut());
        app.borrow_mut().ui.color_themes_panel.mode = Some(ColorPanelMode::HsbSliders);
        let mut fx = fixture(&app);
        let (h_rail, s_rail) = (rails(&fx)[1], rails(&fx)[2]);
        fx.click(&h_rail).settle();
        let hue = app.borrow().ui.color_themes_panel.held.clone().expect("held").values.values[0];
        fx.click(&s_rail).settle();
        fx.key(egui::Key::Home, egui::Modifiers::NONE).settle();
        let held = app.borrow().ui.color_themes_panel.held.clone().expect("held");
        assert_eq!((held.values.values[1], held.values.values[0]), (0.0, hue), "a gray keeps the hue it was set to");
        fx.key(egui::Key::ArrowRight, egui::Modifiers::NONE).settle();
        let [h, s, _] = draft(&mut app.borrow_mut()).colors[2].to_hsb();
        assert!((h - hue).abs() < 0.5 && s > 0.0, "{h} {s}");
    }

    /// With a rule, a slider edit makes the edited color the theme's new base, as a hex edit does,
    /// and selects it.
    #[test]
    fn with_a_rule_a_slider_edit_rebuilds_the_theme_from_the_edited_color() {
        let app = Rc::new(RefCell::new(app()));
        let d = Draft { rule: Some(Harmony::Triad), ..five_colors(&mut app.borrow_mut()) };
        set_draft(&mut app.borrow_mut(), d);
        app.borrow_mut().ui.color_themes_panel.mode = Some(ColorPanelMode::HsbSliders);
        let mut fx = fixture(&app);
        let h_rail = rails(&fx)[1];
        fx.click(&h_rail).settle();
        let held = app.borrow().ui.color_themes_panel.held.clone().expect("held");
        let base = color_of_values(&held.values);
        let now = draft(&mut app.borrow_mut());
        assert_eq!((now.colors, now.sel), (harmony::theme(Harmony::Triad, base), 0));
    }

    /// A CMYK edit keeps the color CMYK, through Save and Add to Swatches in a CMYK document.
    #[test]
    fn cmyk_and_lab_slider_edits_keep_their_models() {
        let app = Rc::new(RefCell::new(app()));
        app.borrow_mut().run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        five_colors(&mut app.borrow_mut());
        app.borrow_mut().ui.color_themes_panel.mode = Some(ColorPanelMode::CmykSliders);
        let mut fx = fixture(&app);
        let c_rail = rails(&fx)[1];
        fx.click(&c_rail).settle();
        let mut app = app.borrow_mut();
        let colors = draft(&mut app).colors;
        assert!(matches!(colors[2], Color::Cmyk { .. }), "the edited color is CMYK");
        save(&mut app);
        assert_eq!(themes(&app)[0].colors, colors, "saved in their models");
        add_draft_to_swatches(&mut app);
        let g = app.session.doc().unwrap().doc.swatch_groups.last().expect("the group");
        assert_eq!(g.swatches.iter().filter_map(|w| w.paint.color()).collect::<Vec<_>>(), colors);
    }
}
