//! File → New: the framework's New Document dialog over VectorCraft's settings. The cards (Recent,
//! Saved and the built-in categories of `file.newPresets`) and the Preset Details rows (size and
//! units, orientation, artboards, bleed, background contents, Advanced Options, More Settings and
//! Manage Profiles) are this file's; the tabs, the card grid, the name field with Save Preset and
//! Create and Close are the framework's. Create runs `file.new` through the held OK, so a refused
//! size keeps the dialog open with the reason; saved-preset edits and the Manage Profiles buttons
//! reach the app through [`pump`].
//!
//! The dialog's fields, as `ui.dialog.set` reads and writes them, are `file.new`'s params
//! (`name` is the document's name, `width`, `height` (pt), `units`, `artboards`, `artboardLayout`,
//! `bleed`, `backgroundContents`, `colorMode`, `rasterEffectsPpi`, `previewMode`,
//! `alignNewObjectsToPixelGrid`, `profile`), plus `basedOn`, the preset the fields were filled from.
//! The dialog opens in Preferences › Units › General, also on a Recent entry made in other units
//! (one in pixels, or a saved preset in the units it was saved in, keeps its units); units chosen in
//! the Units list or set with `ui.dialog.set` stay when a card is picked.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use vectorcraft_doc::{Background, ColorMode, Unit};
use vectorcraft_engine::cmd::newdoc::{self, ArtboardLayout, DocSettings, MAX_ARTBOARDS, PreviewMode, RASTER_PPI};

use craft::behavior::{IconId, Mixed};
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{
    CardCx, CardSource, DetailsCx, DocumentCard, DocumentSettings, HostedNewDocument, NewDocumentDialog, Orientation, PresetCategory,
};
use craft::ui::patterns::presets::{GroupPath, NamedPresetEvent, PresetLibrary};
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::sections::Section;
use craft::ui::text::{LinkMode, LinkedFields, LinkedLayout, NumberField};

use super::host::open_held;
use crate::VectorcraftApp;

/// The fields `file.new` takes.
const PARAMS: [&str; 14] = [
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
    "alignNewObjectsToPixelGrid",
    "profile",
];
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
const BACKGROUNDS: [(Background, &str); 2] = [(Background::Transparent, "Transparent (Default)"), (Background::White, "White")];
const COLOR_MODES: [(ColorMode, &str); 2] = [(ColorMode::Rgb, "RGB Color"), (ColorMode::Cmyk, "CMYK Color")];
/// The name settings take once a field of Preset Details was edited: no preset any more.
const CUSTOM: &str = "Custom";
/// The ids of the Manage Profiles buttons' actions, followed by `:` and the profile's name.
const RESET_PROFILE: &str = "profile.reset";
const EDIT_PROFILE: &str = "profile.edit";
/// The size fields' range, in points: the engine accepts a side up to the canvas less both bleeds.
// PLAN: `newdoc::MAX_SIDE` and `MAX_SPACING` are private to the engine; the two constants repeat their values.
const MAX_SIDE: f64 = vectorcraft_doc::MAX_COORD - 2.0 * vectorcraft_doc::setup::MAX_BLEED;
const MIN_SIDE: f64 = 0.01;
const MAX_SPACING: f64 = 1.0e5;

/// What New Document edits: the document's name over the engine's settings, whose own `name` is the
/// preset they were filled from.
#[derive(Clone, Debug, Default)]
pub(crate) struct VcNewDocument {
    /// The new document's name.
    title: String,
    /// The settings; `name` is the preset they came from, `Custom` once edited.
    settings: DocSettings,
    /// The orientation automation set, which [`DocumentSettings::note_set`] turns the page to: read back from the
    /// fields, the width and height carry the orientation, so the one listed with them is not applied.
    turn: Option<Value>,
}

/// The same document whatever orientation a set asked for: `turn` is spent by the set that gave it.
impl PartialEq for VcNewDocument {
    fn eq(&self, other: &Self) -> bool {
        (&self.title, &self.settings) == (&other.title, &other.settings)
    }
}

impl VcNewDocument {
    /// `settings` as they are, with an empty document name (a card's value).
    fn of(settings: DocSettings) -> Self {
        Self { title: String::new(), settings, turn: None }
    }

    /// The fields as `ui.dialog.set` and `file.new` name them.
    fn value(&self) -> Value {
        let mut v = self.settings.to_json();
        if let Value::Object(m) = &mut v {
            m.insert("name".into(), json!(self.title));
            m.insert("basedOn".into(), json!(self.settings.name));
        }
        v
    }

    /// The settings the fields `value` describe; a missing field keeps its default and a bad one is
    /// refused with the engine's own message.
    fn from_value(value: &Value) -> Result<Self, String> {
        let mut sizes = value.clone();
        let turn = sizes.as_object_mut().and_then(|m| m.remove("orientation"));
        let read = |p: &Value| DocSettings::default().with_params(p, "ui.dialog.set").map_err(|e| e.to_string());
        // A bad orientation is refused with the engine's own message.
        if let Some(o) = &turn {
            read(&json!({ "orientation": o }))?;
        }
        let mut settings = read(&sizes)?;
        settings.name = value.get("basedOn").and_then(Value::as_str).unwrap_or(CUSTOM).to_owned();
        let title = value.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
        Ok(Self { title, settings, turn })
    }
}

impl Serialize for VcNewDocument {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for VcNewDocument {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(&Value::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// What the dialog is set up with besides the cards.
#[derive(Clone, Default)]
pub(crate) struct Setup {
    /// The name of the next untitled document.
    untitled: String,
    /// The New Document profiles, each with whether it was edited.
    profiles: Vec<(&'static str, bool)>,
    /// Whether profiles can be edited (the web build and runs without preferences have no folder).
    has_profile_folder: bool,
    /// Numbers Without Units Are Points.
    bare_points: bool,
}

impl Setup {
    /// The setup the app's session gives now.
    fn of(app: &VectorcraftApp) -> Self {
        Self {
            untitled: app.session.peek_untitled().to_string(),
            profiles: newdoc::profile_names().map(|name| (name, app.session.new_doc_profile_customized(name))).collect(),
            has_profile_folder: app.session.new_doc_profile_dir().is_some(),
            bare_points: app.session.prefs.numbers_without_units_are_points,
        }
    }
}

/// What the dialog's session keeps for the Preset Details rows.
// PLAN: the sheet's `card_units` and `Setup::general_units` are not kept: a card carries its own units and the
// first settings are filled before the dialog opens, so nothing reads them.
#[derive(Clone, Default)]
pub(crate) struct Memory {
    /// More Settings' pop-up is open.
    more_open: bool,
    /// Manage Profiles' pop-up is open.
    profiles_open: bool,
    /// The units were chosen (in the Units list or with `ui.dialog.set`): a card picked after keeps them.
    units_chosen: bool,
    /// The four bleed fields were unlinked.
    bleed_split: bool,
}

impl DocumentSettings for VcNewDocument {
    type Preset = DocSettings;
    type Setup = Setup;
    type Memory = Memory;

    fn name(&self) -> &str {
        &self.title
    }

    fn set_name(&mut self, name: String) {
        self.title = name;
    }

    fn page(&self) -> (f64, f64) {
        (self.settings.width, self.settings.height)
    }

    /// One entry per size (orientation included), units and colour set-up: a landscape and a portrait
    /// A3 are two documents (#2860).
    fn same_document(&self, other: &Self) -> bool {
        let (a, b) = (&self.settings, &other.settings);
        (a.width, a.height, a.units, a.color_mode, a.background) == (b.width, b.height, b.units, b.color_mode, b.background)
    }

    fn fits(&self) -> bool {
        Self::from_value(&self.value()).is_ok()
    }

    fn note_set(&mut self, field: &str, memory: &mut Memory) {
        if field == "units" {
            memory.units_chosen = true;
        }
        if let Some(o) = self.turn.take().filter(|_| field == "orientation") {
            let name = self.settings.name.clone();
            if let Ok(turned) = self.settings.clone().with_params(&json!({ "orientation": o }), "ui.dialog.set") {
                self.settings = DocSettings { name, ..turned };
            }
        }
    }

    fn open(&mut self, setup: &Setup, _untitled: &str) {
        self.title.clone_from(&setup.untitled);
    }

    fn preset_card(preset: &DocSettings, cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        built_in_card(preset, cx.values, cx.orientation)
    }

    fn recent_cards(cx: &CardCx<'_, Self>) -> Vec<DocumentCard<Self>> {
        let now = &cx.values.settings;
        cx.recents
            .iter()
            .map(|recent| {
                let s = &recent.settings;
                // The entry the fields were filled from shows the units chosen in Preset Details, which
                // the new document will have (#2078); the others keep their own.
                let chosen = (s.name.as_str(), s.width, s.height) == (now.name.as_str(), now.width, now.height);
                let shown = DocSettings { units: if chosen { now.units } else { s.units }, ..s.clone() };
                DocumentCard::new(text(preset_name(&s.name)), shown.size_label(), recent.clone(), CardSource::Recent)
            })
            .collect()
    }

    fn saved_card(name: &str, saved: &Self, _cx: &CardCx<'_, Self>) -> DocumentCard<Self> {
        // PLAN: `CardSource::Saved` takes a `PresetId` this hook is not given; the framework sets the source of a saved row.
        DocumentCard::new(text(name), saved.settings.size_label(), saved.clone(), CardSource::Recent)
    }

    /// A card sets everything but the document's name, and keeps the units chosen in Preset Details.
    fn apply_card(&mut self, card: &DocumentCard<Self>, _setup: &Setup, memory: &mut Memory) {
        let units = self.settings.units;
        self.settings = card.settings.settings.clone();
        if memory.units_chosen {
            self.settings.units = units;
        }
        memory.bleed_split = !self.settings.bleed.iter().all(|b| *b == self.settings.bleed[0]);
    }

    fn details(ui: &mut egui::Ui, values: &mut Self, cx: &mut DetailsCx<'_, Self>) {
        let words = cx.words();
        let (width, height, orientation, artboards, background, portrait, landscape) = (
            words.width.clone(),
            words.height.clone(),
            words.orientation.clone(),
            words.artboards.clone(),
            words.background.clone(),
            words.portrait.clone(),
            words.landscape.clone(),
        );
        let setup = cx.setup().clone();
        let mut memory = cx.memory().clone();
        let before = values.settings.clone();
        let unit = before.units;
        let lengths = crate::fields::bare_points(LengthUnits::new(UnitKind::Points, unit_kind(unit), pixels_per_inch()), setup.bare_points);
        let current = if before.width > before.height { Orientation::Landscape } else { Orientation::Portrait };
        let (mut turned, mut picked, mut action) = (false, None, None);

        let s = &mut values.settings;
        let units: Vec<Entry<Unit>> = UNITS.iter().map(|u| Entry::new(*u, Label::from(u.label()))).collect();
        Field::new(width.clone()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                size_field(ui, &width, &mut s.width, lengths);
                if let Some(u) = Select::new(text(tl!("Units")), Bind::given(unit), &units).show(ui).value().cloned() {
                    s.units = u;
                    memory.units_chosen = true;
                }
            });
        });
        // Host viewports below the desktop main window's 800-point minimum need this row to wrap.
        Row::new().wrap(ui.ctx().content_rect().width() < 800.0).show(ui, |ui, _| {
            Field::new(height.clone()).show(ui, |ui, _| size_field(ui, &height, &mut s.height, lengths));
            Field::new(orientation.clone()).show(ui, |ui, _| {
                let entries = [
                    Entry::new(Orientation::Portrait, portrait.clone()).icon(IconId::new("orientation_portrait")),
                    Entry::new(Orientation::Landscape, landscape.clone()).icon(IconId::new("orientation_landscape")),
                ];
                picked = Segmented::new(orientation.clone(), Bind::given(current), &entries).show(ui).value().cloned();
            });
            Field::new(artboards.clone()).show(ui, |ui, _| {
                let field = NumberField::new(artboards.clone(), Bind::given(Some(f64::from(s.artboards))))
                    .range(1.0..=f64::from(MAX_ARTBOARDS))
                    .step(1.0)
                    .commit_decimals(0);
                if let Some(n) = field.show(ui).value().copied().flatten().filter(|n| n.is_finite()) {
                    s.artboards = n.round().clamp(1.0, f64::from(MAX_ARTBOARDS)) as u32;
                }
            });
        });
        if let Some(o) = picked
            && o != current
        {
            std::mem::swap(&mut s.width, &mut s.height);
            turned = true;
        }

        let bleed = text(tl!("Bleed"));
        Field::new(bleed.clone()).show(ui, |ui, _| {
            let mut linked = !memory.bleed_split;
            let parts = s.bleed.map(|b| Mixed::Same(Some(b)));
            let captions = [text(tl!("Top")), text(tl!("Bottom")), text(tl!("Left")), text(tl!("Right"))];
            let out = LinkedFields::<4>::new(bleed.clone(), Bind::given(parts), Bind::bound(&mut linked))
                .mode(LinkMode::Equal)
                .layout(LinkedLayout::Columns)
                .captions(captions)
                .range(0.0..=vectorcraft_doc::setup::MAX_BLEED)
                .lengths(lengths)
                .show(ui);
            if let Some(edited) = out.edit.value().copied() {
                for (side, part) in s.bleed.iter_mut().zip(edited) {
                    if let Mixed::Same(Some(b)) = part {
                        *side = b.clamp(0.0, vectorcraft_doc::setup::MAX_BLEED);
                    }
                }
            }
            memory.bleed_split = !linked;
        });

        Field::new(background.clone()).show(ui, |ui, _| {
            let entries: Vec<Entry<Background>> = BACKGROUNDS.iter().map(|(b, name)| Entry::new(*b, text(tl!(name)))).collect();
            if let Some(b) = Select::new(background.clone(), Bind::given(s.background), &entries).show(ui).value().copied() {
                s.background = b;
            }
        });

        Section::new(text(tl!("Advanced Options"))).initially_open(false).show(ui, |ui, _| advanced(ui, s));

        let more = Button::new(text(tl!("More Settings"))).id_salt("more-settings").show(ui);
        let mut more_open = memory.more_open != clicked(&more);
        Popover::new(text(tl!("More Settings")), Bind::bound(&mut more_open)).opened_by(&more).show(ui, |ui, _| more_settings(ui, s, lengths));
        memory.more_open = more_open;

        let manage = Button::new(text(tl!("Manage Profiles"))).id_salt("manage-profiles").show(ui);
        let mut profiles_open = memory.profiles_open != clicked(&manage);
        Popover::new(text(tl!("Manage Profiles")), Bind::bound(&mut profiles_open))
            .opened_by(&manage)
            .show(ui, |ui, _| manage_profiles(ui, &setup, &mut action));
        memory.profiles_open = profiles_open;

        // Units and the orientation only change how the page shows; any other edit ends the preset.
        let mut after = values.settings.clone();
        after.units = before.units;
        if !turned && after != before {
            values.settings.name = CUSTOM.into();
            cx.drop_card();
        }
        if let Some(o) = picked {
            cx.set_orientation(o);
        }
        if let Some(id) = action {
            cx.request_action(id);
        }
        *cx.memory() = memory;
    }
}

/// The card of the built-in `preset`, in the units chosen in Preset Details and turned to the picked
/// `orientation` (a square has none).
fn built_in_card(preset: &DocSettings, values: &VcNewDocument, orientation: Option<Orientation>) -> DocumentCard<VcNewDocument> {
    let mut s = DocSettings { units: values.settings.units, ..preset.clone() };
    let landscape = orientation.map(|o| o == Orientation::Landscape);
    if let Some(landscape) = landscape
        && s.width != s.height
        && (s.width > s.height) != landscape
    {
        std::mem::swap(&mut s.width, &mut s.height);
    }
    let line = s.size_label();
    let value = VcNewDocument { title: values.title.clone(), settings: s, turn: None };
    DocumentCard::new(text(preset_name(&preset.name)), line, value, CardSource::preset(None))
}

/// A label with the text `s`, already translated.
fn text(s: &str) -> Label {
    Label::from(s.to_owned())
}

/// The framework's unit for `unit`; Feet and Inches counts in feet.
fn unit_kind(unit: Unit) -> UnitKind {
    match unit {
        Unit::Pixels => UnitKind::Pixels,
        Unit::Picas => UnitKind::Picas,
        Unit::Inches => UnitKind::Inches,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Meters => UnitKind::Metres,
        Unit::Yards => UnitKind::Yards,
        Unit::Feet | Unit::FeetInches => UnitKind::Feet,
        _ => UnitKind::Points,
    }
}

/// Pixels to the inch, as the engine sizes a pixel.
fn pixels_per_inch() -> f64 {
    72.0 / Unit::Pixels.to_pt(1.0)
}

/// A side of the page in points, shown in the chosen units.
fn size_field(ui: &mut egui::Ui, name: &Label, points: &mut f64, lengths: LengthUnits) {
    let field = NumberField::length(name.clone(), Bind::given(Some(*points)), lengths).range(MIN_SIDE..=MAX_SIDE);
    if let Some(v) = field.show(ui).value().copied().flatten().filter(|v| v.is_finite()) {
        *points = v;
    }
}

/// The rows of Advanced Options: Color Mode, Raster Effects, Preview Mode and Align New Objects to
/// Pixel Grid.
fn advanced(ui: &mut egui::Ui, s: &mut DocSettings) {
    let modes: Vec<Entry<ColorMode>> = COLOR_MODES.iter().map(|(m, name)| Entry::new(*m, text(tl!(name)))).collect();
    let ppi: Vec<Entry<u32>> = RASTER_PPI.iter().map(|(v, name)| Entry::new(*v as u32, text(tl!(name)))).collect();
    let previews: Vec<Entry<PreviewMode>> = PreviewMode::ALL
        .iter()
        .map(|m| {
            let name = match m {
                PreviewMode::Default => tl!("Default"),
                PreviewMode::Pixel => tl!("Pixel"),
                PreviewMode::Overprint => tl!("Overprint"),
            };
            Entry::new(*m, text(name))
        })
        .collect();
    let (mode, effects, preview) = (text(tl!("Color Mode")), text(tl!("Raster Effects")), text(tl!("Preview Mode")));
    Field::new(mode.clone()).show(ui, |ui, _| {
        if let Some(m) = Select::new(mode.clone(), Bind::given(s.color_mode), &modes).show(ui).value().copied() {
            s.color_mode = m;
        }
    });
    Field::new(effects.clone()).show(ui, |ui, _| {
        let current = s.raster_effects_ppi.clamp(1.0, 2400.0).round() as u32;
        if let Some(v) = Select::new(effects.clone(), Bind::given(current), &ppi).show(ui).value().copied() {
            s.raster_effects_ppi = f64::from(v);
        }
    });
    Field::new(preview.clone()).show(ui, |ui, _| {
        if let Some(m) = Select::new(preview.clone(), Bind::given(s.preview_mode), &previews).show(ui).value().copied() {
            s.preview_mode = m;
        }
    });
    let align = Checkbox::new(Bind::given(s.align_new_objects_to_pixel_grid)).label(text(tl!("Align New Objects to Pixel Grid"))).show(ui);
    if let Some(on) = align.value().copied() {
        s.align_new_objects_to_pixel_grid = on;
    }
}

/// More Settings' pop-up: the artboard layout, spacing and columns. All three are off with one
/// artboard, Columns also for the row and column layouts; columns stay within the artboards.
fn more_settings(ui: &mut egui::Ui, s: &mut DocSettings, lengths: LengthUnits) {
    let several = s.artboards > 1;
    let grid = matches!(s.layout, ArtboardLayout::GridByRow | ArtboardLayout::GridByColumn);
    let (layout, spacing, columns, right_to_left) =
        (text(tl!("Artboard Layout")), text(tl!("Spacing")), text(tl!("Columns")), text(tl!("Change to Right-to-Left Layout")));
    Field::new(layout.clone()).show(ui, |ui, _| {
        let entries: Vec<Entry<ArtboardLayout>> = ArtboardLayout::ALL.iter().map(|l| Entry::new(*l, text(tl!(l.label())))).collect();
        if let Some(l) = Segmented::new(layout.clone(), Bind::given(s.layout), &entries).enabled(several).show(ui).value().copied() {
            choose_layout(s, l);
        }
    });
    let rtl = Checkbox::new(Bind::given(s.right_to_left)).label(right_to_left).enabled(several).show(ui);
    if let Some(on) = rtl.value().copied() {
        s.right_to_left = on;
    }
    Field::new(spacing.clone()).show(ui, |ui, _| {
        let field = NumberField::length(spacing.clone(), Bind::given(Some(s.spacing)), lengths).range(0.0..=MAX_SPACING).enabled(several);
        if let Some(v) = field.show(ui).value().copied().flatten().filter(|v| v.is_finite()) {
            s.spacing = v;
        }
    });
    Field::new(columns.clone()).show(ui, |ui, _| {
        let n = f64::from(s.artboards.max(1));
        let shown = f64::from(shown_columns(s));
        let field = NumberField::new(columns.clone(), Bind::given(Some(shown))).range(1.0..=n).step(1.0).commit_decimals(0).enabled(several && grid);
        if let Some(v) = field.show(ui).value().copied().flatten().filter(|v| v.is_finite()) {
            set_columns(s, v.round().clamp(1.0, n) as u32);
        }
    });
}

/// The columns More Settings shows: for a grid by column made with `file.new`'s `rows`, the columns
/// those rows give the current number of artboards, as `file.new` lays them out; otherwise the
/// stored count. At most one for each artboard.
fn shown_columns(s: &DocSettings) -> u32 {
    let n = s.artboards.max(1);
    if s.layout == ArtboardLayout::GridByColumn && s.rows > 0 { u32::try_from(s.grid_columns()).unwrap_or(n) } else { s.columns.max(1).min(n) }
}

/// Chooses a layout in More Settings. Another layout replaces a preset's rows and keeps the column
/// count those rows showed; choosing the layout that is already selected keeps the rows.
fn choose_layout(s: &mut DocSettings, l: ArtboardLayout) {
    if l == s.layout {
        return;
    }
    if s.rows > 0 {
        s.columns = shown_columns(s);
        s.rows = 0;
    }
    s.layout = l;
}

/// A column count entered in More Settings replaces the rows of a preset made with `file.new`'s
/// `rows`; the same count as shown changes nothing.
fn set_columns(s: &mut DocSettings, columns: u32) {
    if columns != shown_columns(s) {
        s.columns = columns;
        s.rows = 0;
    }
}

/// Manage Profiles' pop-up: a row per New Document profile with Edit, which opens the profile as a
/// document, and Reset, which brings back the shipped profile. The button pressed is put in `action`.
fn manage_profiles(ui: &mut egui::Ui, setup: &Setup, action: &mut Option<String>) {
    Text::new(text(tl!("NEW DOCUMENT PROFILES"))).show(ui);
    for (name, customized) in &setup.profiles {
        ui.push_id(name, |ui| {
            Row::new().show(ui, |ui, _| {
                Text::new(text(tl!(name))).show(ui);
                if clicked(&Button::new(text(tl!("Edit"))).id_salt("edit").enabled(setup.has_profile_folder).show(ui)) {
                    *action = Some(format!("{EDIT_PROFILE}:{name}"));
                }
                if clicked(&Button::new(text(tl!("Reset"))).id_salt("reset").enabled(*customized).show(ui)) {
                    *action = Some(format!("{RESET_PROFILE}:{name}"));
                }
            });
        });
    }
}

/// The category of the settings New Document opens on: the most recent, else Print.
fn first_category(app: &VectorcraftApp) -> &'static str {
    if app.session.prefs.recent_new_docs.is_empty() { "Print" } else { "Recent" }
}

/// The settings of the first preset of `category`, as opening the dialog starts on them; None when
/// the category has none. The most recent entry keeps its size and takes Preferences › Units ›
/// General, as the built-in presets do, unless it is in pixels or it is a saved preset in the units
/// it was saved in.
fn first_settings(app: &VectorcraftApp, category: &str) -> Option<VcNewDocument> {
    let first = newdoc::category(&app.session, category)?.into_iter().next()?;
    // `file.new` reads a named preset in its own units, and names the entry Custom in others.
    let as_saved = newdoc::find(&app.session, &first.name).is_some_and(|p| p.units == first.units);
    let own = !category.eq_ignore_ascii_case("Recent") || first.units == Unit::Pixels || as_saved;
    let units = if own { first.units } else { app.session.default_units() };
    Some(VcNewDocument { title: app.session.peek_untitled().to_string(), settings: DocSettings { units, ..first }, turn: None })
}

/// The settings New Document opens on.
fn offered(app: &VectorcraftApp) -> VcNewDocument {
    first_settings(app, first_category(app)).unwrap_or_else(|| VcNewDocument {
        title: app.session.peek_untitled().to_string(),
        settings: DocSettings { units: app.session.default_units(), ..DocSettings::default() },
        turn: None,
    })
}

/// Open New Document on the most recent settings (or Letter), named after the next untitled
/// document, in Preferences › Units › General.
pub fn open(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status(tl!("There is no window to show New Document in."));
        return;
    };
    let first = offered(app);
    let categories: Vec<PresetCategory<DocSettings>> = newdoc::category_names()
        .skip(2)
        .filter_map(|name| Some(PresetCategory::new(name, text(tl!(name)), newdoc::category(&app.session, name)?)))
        .collect();
    let setup = Setup::of(app);
    let mut saved = PresetLibrary::new();
    for preset in &app.session.prefs.new_doc_presets {
        // The engine keeps names the library refuses (an empty one); such a preset is not offered here.
        saved.add(&GroupPath::root(), &preset.name, VcNewDocument { title: preset.name.clone(), settings: preset.clone(), turn: None }).ok();
    }
    let recent: Vec<VcNewDocument> = app.session.prefs.recent_new_docs.iter().cloned().map(VcNewDocument::of).collect();
    let mut dialog = HostedNewDocument::<VcNewDocument>::new(&ctx, first, move |options| {
        NewDocumentDialog::over(Bind::bound(options)).categories(categories.clone()).setup(setup.clone())
    })
    .saved(saved)
    .recent(recent);
    if app.session.prefs.recent_new_docs.is_empty() {
        dialog = dialog.tab("Print");
    }
    open_held(app, dialog, |app, answer| {
        let r = app.run("file.new", Value::Object(params(app, &answer.options)))?;
        report_profile(app, &r);
        Ok(())
    });
}

/// Run what the open New Document dialog asked of the app this frame: saved-preset edits
/// (`file.newPresets.save` and `.delete`) and the Manage Profiles buttons (`file.newProfiles.reset`
/// and `.edit`; an Edit that succeeds closes the dialog). A frame's second pass finds nothing: the
/// dialog hands each request over once.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let (presets, actions) = dialogs.with(|host| match host.get_mut::<HostedNewDocument<VcNewDocument>>() {
        Some(d) => {
            let events = d.take_preset_events();
            let presets: Vec<PresetEdit> = events
                .into_iter()
                .filter_map(|event| match event {
                    NamedPresetEvent::Create { name, .. } => {
                        let value = d.saved_presets().items().into_iter().find(|item| item.name == name).map(|item| item.value.clone())?;
                        Some(PresetEdit::Save(name, value))
                    }
                    NamedPresetEvent::Delete(name) => Some(PresetEdit::Delete(name)),
                    _ => None,
                })
                .collect();
            (presets, d.take_actions())
        }
        None => (Vec::new(), Vec::new()),
    });
    for edit in presets {
        let done = match edit {
            PresetEdit::Save(name, doc) => save_preset(app, &name, &doc),
            PresetEdit::Delete(name) => app.run("file.newPresets.delete", json!({ "name": name })),
        };
        if let Err(e) = done {
            app.status(e);
        }
    }
    if actions.is_empty() {
        return;
    }
    let mut close = false;
    for id in &actions {
        close |= profile_action(app, id);
    }
    if close {
        dialogs.with(|host| host.cancel::<HostedNewDocument<VcNewDocument>>(ctx));
    } else {
        let setup = Setup::of(app);
        dialogs.with(|host| {
            if let Some(d) = host.get_mut::<HostedNewDocument<VcNewDocument>>() {
                d.set_setup(setup);
            }
        });
    }
}

/// A saved-preset edit of the dialog.
enum PresetEdit {
    /// A preset made of the dialog's settings under this name.
    Save(String, VcNewDocument),
    /// The preset of this name was deleted.
    Delete(String),
}

/// Store `doc` as the preset `name`.
fn save_preset(app: &mut VectorcraftApp, name: &str, doc: &VcNewDocument) -> Result<Value, String> {
    let mut p = params(app, doc);
    p.insert("name".into(), json!(name));
    p.remove("preset");
    app.run("file.newPresets.save", Value::Object(p))
}

/// Run the Manage Profiles action `id` (`profile.reset:<name>` or `profile.edit:<name>`); true when
/// it was an Edit that opened the profile, which closes New Document.
fn profile_action(app: &mut VectorcraftApp, id: &str) -> bool {
    let Some((verb, name)) = id.split_once(':') else { return false };
    let command = match verb {
        RESET_PROFILE => "file.newProfiles.reset",
        EDIT_PROFILE => "file.newProfiles.edit",
        _ => return false,
    };
    match app.run(command, json!({ "name": name })) {
        Ok(_) => verb == EDIT_PROFILE,
        Err(e) => {
            app.status(e);
            false
        }
    }
}

/// Say in the status bar when `file.new`'s result `r` reports a profile it could not read, or
/// color profiles its file carries that can't be used.
fn report_profile(app: &mut VectorcraftApp, r: &Value) {
    if let Some(e) = r.get("profileError") {
        let (name, reason) = (e["profile"].as_str().unwrap_or_default(), e["reason"].as_str().unwrap_or_default());
        app.status(format!("The {name} profile could not be read, so the new document has the default settings: {reason}"));
    } else if let Some(w) = r["warnings"].as_array().filter(|w| !w.is_empty()) {
        app.status(w.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "));
    }
}

/// New Without Dialog (`file.newWithoutDialog`, Cmd+Alt+N): create the document [`open`] would
/// offer, as its Create would, without showing the dialog. A dialog already open stays open.
pub fn create_without_dialog(app: &mut VectorcraftApp) -> Result<Value, String> {
    let p = params(app, &offered(app));
    let r = app.run("file.new", Value::Object(p))?;
    report_profile(app, &r);
    Ok(r)
}

/// The `file.new` params of `doc`.
fn params(app: &VectorcraftApp, doc: &VcNewDocument) -> Map<String, Value> {
    let Value::Object(fields) = doc.value() else { return Map::new() };
    let mut p: Map<String, Value> = PARAMS.iter().filter_map(|k| Some((k.to_string(), fields.get(*k)?.clone()))).collect();
    // Only a preset `file.new` knows keeps its name; Custom and Recent entries made from none don't.
    if newdoc::find(&app.session, &doc.settings.name).is_some() {
        p.insert("preset".into(), json!(doc.settings.name));
    }
    p
}

/// Is `name` a built-in preset's? Those are ours (translated); Recent and Saved hold the names of
/// the user's documents and presets, shown as they are.
fn is_builtin_preset(name: &str) -> bool {
    newdoc::CATEGORIES.iter().any(|(_, _, _, presets)| presets.iter().any(|(n, ..)| *n == name))
}

/// A preset's name as shown: a built-in one translated, the user's as it is.
pub(crate) fn preset_name(name: &str) -> &str {
    if is_builtin_preset(name) { tl!(name) } else { name }
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::StackDialog;
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    type Hosted = HostedNewDocument<VcNewDocument>;

    /// An app with a window context, so New Document has a host to open on.
    fn app() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().unwrap()
    }

    /// The open dialog's fields, as `ui.dialog.get` lists them.
    fn fields(app: &VectorcraftApp) -> Value {
        app.dialogs().unwrap().with(|h| h.get::<Hosted>().unwrap().json_fields().unwrap())
    }

    /// Set a field of the open dialog, as `ui.dialog.set` does.
    fn set(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = ctx(app);
        app.dialogs().unwrap().with(|h| {
            let id = h.get::<Hosted>().unwrap().id();
            h.set_json_field(&ctx, id, field, value)
        })
    }

    /// Press Create: the dialog files its answer and the pump runs `file.new` with it.
    fn create(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        app.dialogs().unwrap().with(|h| h.confirm::<Hosted>(&ctx));
        crate::dialogs::host::pump(app, &ctx);
        // A held OK closes at the dialog's next pass.
        crate::tests_synthetic::key_frame(app, &ctx, vec![]);
    }

    /// Create, then the new document's units.
    fn create_units(app: &mut VectorcraftApp) -> Unit {
        create(app);
        assert!(app.dialogs().unwrap().with(|h| !h.is_open::<Hosted>()), "{:?}", app.ui.status);
        app.session.active().unwrap().doc.units
    }

    fn general_units(app: &mut VectorcraftApp, units: &str) {
        app.run("prefs.set", json!({"key": "unitsGeneral", "value": units, "documentUnits": false})).unwrap();
    }

    /// The window's frames with the dialog open, laid out; the last frame's output.
    fn laid_out(app: &mut VectorcraftApp) -> egui::FullOutput {
        let ctx = ctx(app);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        let mut frame = |i: u32| {
            let input = egui::RawInput { time: Some(0.1 * f64::from(i)), screen_rect: Some(screen), ..Default::default() };
            crate::testing::part_frame(app, &ctx, input, |app, ui| app.ui(ui))
        };
        (0..3).for_each(|i| drop(frame(i)));
        frame(3)
    }

    /// Orientation shows a page icon for Portrait and for Landscape, named for screen readers, as production's buttons
    /// did; before, it showed the words, and Landscape, cut short, ran into Artboards.
    #[test]
    fn orientation_shows_two_page_icons_beside_artboards() {
        fn texts(s: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect().center())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut app = app();
        open(&mut app);
        let out = laid_out(&mut app);
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        let bounds = |label: &str, role: egui::accesskit::Role| {
            let (_, n) = nodes.iter().find(|(_, n)| n.label() == Some(label) && n.role() == role).unwrap_or_else(|| panic!("no {role:?} {label}"));
            n.bounds().expect("bounds")
        };
        let landscape = bounds("Landscape", egui::accesskit::Role::RadioButton);
        let artboards = bounds("Artboards", egui::accesskit::Role::SpinButton);
        assert!(landscape.x1 <= artboards.x0, "Landscape {landscape:?} stays left of Artboards {artboards:?}");
        let group = bounds("Orientation", egui::accesskit::Role::RadioGroup);
        let mut drawn = Vec::new();
        out.shapes.iter().for_each(|s| texts(&s.shape, &mut drawn));
        let inside: Vec<_> = drawn
            .iter()
            .filter(|(_, at)| (group.x0..=group.x1).contains(&f64::from(at.x)) && (group.y0..=group.y1).contains(&f64::from(at.y)))
            .collect();
        assert!(inside.is_empty(), "the segments draw icons, not words: {inside:?}");
    }

    /// Recent shows each entry as it was made: a portrait and a landscape A3 are two documents, and
    /// the built-in cards turn to the picked orientation (#2860).
    #[test]
    fn recent_cards_keep_the_orientation_they_were_made_in() {
        let app = VectorcraftApp::new(Session::new(), Default::default());
        let a3 = newdoc::find(&app.session, "A3").unwrap();
        let landscape = VcNewDocument::of(DocSettings { width: a3.height, height: a3.width, ..a3.clone() });
        let portrait = VcNewDocument::of(a3.clone());
        assert!(!landscape.same_document(&portrait));
        assert!(portrait.same_document(&portrait.clone()));
        for preset in newdoc::category(&app.session, "Print").unwrap() {
            let card = built_in_card(&preset, &portrait, Some(Orientation::Portrait));
            let (w, h) = card.settings.page();
            assert!(h >= w, "{} turns portrait", preset.name);
        }
    }

    #[test]
    fn a_preset_fills_the_details_and_create_makes_the_document() {
        let mut app = app();
        open(&mut app);
        let f = fields(&app);
        assert_eq!((&f["name"], &f["basedOn"], &f["colorMode"]), (&json!("Untitled-1"), &json!("Letter"), &json!("cmyk")));
        for (k, v) in [
            ("units", json!("Pixels")),
            ("width", json!(3840.0)),
            ("height", json!(2160.0)),
            ("colorMode", json!("rgb")),
            ("artboards", json!(4)),
            ("artboardLayout", json!({"layout": "gridByRow", "columns": 2, "spacing": 10})),
            ("bleed", json!([0, 0, 0, 0])),
            ("previewMode", json!("pixel")),
            ("backgroundContents", json!("white")),
        ] {
            set(&app, k, v).unwrap();
        }
        create(&mut app);
        let st = app.session.active().unwrap();
        let rects: Vec<_> = st.doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0)).collect();
        assert_eq!(rects, [(0.0, 0.0), (3850.0, 0.0), (0.0, 2170.0), (3850.0, 2170.0)]);
        assert_eq!(st.doc.setup.background, vectorcraft_doc::Background::White);
        assert!(app.ui.view.pixel_preview, "Pixel preview mode");
        // Next time it opens on the document just made, named after the next untitled one.
        open(&mut app);
        assert_eq!(fields(&app)["name"], json!("Untitled-2"));
    }

    /// The four Bleed fields stay inside the dialog in a 1280 by 900 window; in one row, Right ran past the dialog's
    /// right edge.
    #[test]
    fn bleed_fields_stay_inside_the_dialog() {
        let mut app = app();
        open(&mut app);
        let ctx = ctx(&app);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        // Each frame gives the app's input once, as the hosts do.
        let frame = |app: &mut VectorcraftApp, i: u32| {
            let mut raw = egui::RawInput { time: Some(0.1 * f64::from(i)), screen_rect: Some(screen), ..Default::default() };
            app.adopt_context(&ctx);
            app.raw_input(&ctx, &mut raw);
            craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
            let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
            out.textures_delta.clear();
            out
        };
        (0..3).for_each(|i| drop(frame(&mut app, i)));
        let out = frame(&mut app, 3);
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        let bounds = |label: Option<&str>, role: egui::accesskit::Role| {
            let found = nodes.iter().find(|(_, n)| n.role() == role && label.is_none_or(|l| n.label() == Some(l)));
            let (_, n) = found.unwrap_or_else(|| panic!("no {role:?} {label:?}"));
            n.bounds().expect("bounds")
        };
        let dialog = bounds(None, egui::accesskit::Role::Dialog);
        for side in ["Top", "Bottom", "Left", "Right"] {
            let field = bounds(Some(side), egui::accesskit::Role::SpinButton);
            assert!(dialog.x0 <= field.x0 && field.x1 <= dialog.x1, "Bleed {side} {field:?} stays inside the dialog {dialog:?}");
        }
    }

    /// More Settings edits only the artboard layout, spacing and columns, and creates nothing.
    #[test]
    fn more_settings_is_a_popup_holding_only_the_artboard_layout() {
        let mut app = app();
        open(&mut app);
        set(&app, "artboards", json!(3)).unwrap();
        set(&app, "artboardLayout", json!({"layout": "column", "columns": 2, "spacing": 25, "rightToLeft": true})).unwrap();
        let layout = fields(&app)["artboardLayout"].clone();
        assert_eq!((&layout["layout"], &layout["spacing"], &layout["rightToLeft"]), (&json!("column"), &json!(25.0), &json!(true)));
        let ctx = ctx(&app);
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.documents().is_empty(), "no document was made");
    }

    /// A new column count replaces a preset's rows; the count already shown changes nothing.
    #[test]
    fn a_new_column_count_replaces_a_presets_rows() {
        let rows = DocSettings { artboards: 9, layout: ArtboardLayout::GridByColumn, columns: 2, rows: 4, ..DocSettings::default() };
        let mut s = rows.clone();
        set_columns(&mut s, 3);
        assert_eq!(s, rows, "3 is what the rows show");
        set_columns(&mut s, 2);
        assert_eq!((s.layout, s.columns, s.rows), (ArtboardLayout::GridByColumn, 2, 0));
        assert_eq!(s.grid_columns(), 2);
    }

    /// The preset keeps the artboard layout, spacing and columns More Settings set (#4893); an empty
    /// name is refused.
    #[test]
    fn save_preset_saves_what_more_settings_set() {
        let mut app = app();
        open(&mut app);
        set(&app, "artboards", json!(4)).unwrap();
        set(&app, "artboardLayout", json!({"layout": "column", "spacing": 30, "rightToLeft": true})).unwrap();
        let doc: VcNewDocument = serde_json::from_value(fields(&app)).unwrap();
        assert!(save_preset(&mut app, "   ", &doc).is_err(), "an empty name is refused");
        save_preset(&mut app, "Strip", &doc).unwrap();
        let saved = app.session.prefs.new_doc_presets.iter().find(|p| p.name == "Strip").expect("saved");
        assert_eq!(saved.artboards, 4);
        assert!(saved.right_to_left);
        assert_eq!((saved.layout, saved.spacing), (ArtboardLayout::Column, 30.0));
    }

    /// A grid by column with a preset's rows.
    fn by_rows(artboards: u32, rows: u32) -> DocSettings {
        DocSettings { artboards, layout: ArtboardLayout::GridByColumn, columns: 2, rows, ..DocSettings::default() }
    }

    /// Columns shows what a preset's rows give the current number of artboards, and the stored
    /// count without rows.
    #[test]
    fn columns_follow_a_presets_rows_as_artboards_change() {
        for (artboards, shown) in [(5, 2), (9, 3), (2, 1)] {
            assert_eq!(shown_columns(&by_rows(artboards, 4)), shown, "{artboards} artboards");
        }
        assert_eq!(shown_columns(&by_rows(9, 0)), 2);
    }

    /// Another layout replaces a preset's rows and keeps the columns they showed, so Create lays out
    /// what the dialog showed; the layout already selected keeps the rows.
    #[test]
    fn a_new_layout_keeps_the_columns_a_presets_rows_showed() {
        for to in [ArtboardLayout::GridByRow, ArtboardLayout::Row, ArtboardLayout::Column] {
            let mut s = by_rows(9, 4);
            choose_layout(&mut s, to);
            assert_eq!((s.layout, s.columns, s.rows), (to, 3, 0), "{to:?}");
        }
        let mut s = by_rows(9, 4);
        choose_layout(&mut s, ArtboardLayout::GridByColumn);
        assert_eq!(s, by_rows(9, 4));
        let mut s = by_rows(9, 0);
        choose_layout(&mut s, ArtboardLayout::GridByRow);
        assert_eq!((s.columns, s.rows), (2, 0));
    }

    /// A preset's rows reach the dialog's fields and Create puts that many artboards in each column.
    #[test]
    fn a_presets_rows_reach_the_document_create_makes() {
        let mut app = app();
        open(&mut app);
        set(&app, "artboards", json!(5)).unwrap();
        set(&app, "artboardLayout", json!({"layout": "gridByColumn", "rows": 3.6, "spacing": 10})).unwrap();
        assert_eq!(fields(&app)["artboardLayout"]["rows"], json!(4));
        create(&mut app);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.artboards.len(), 5);
        let first: Vec<_> = st.doc.artboards.iter().filter(|a| a.rect.x0 == 0.0).map(|a| a.rect.y0).collect();
        assert_eq!(first.len(), 4, "four artboards in the first column: {first:?}");
    }

    /// Align New Objects to Pixel Grid reaches a saved preset and the document Create makes.
    #[test]
    fn align_new_objects_checkbox_follows_the_preset_and_reaches_the_document() {
        const KEY: &str = "alignNewObjectsToPixelGrid";
        let mut app = app();
        open(&mut app);
        assert_eq!(fields(&app)[KEY], json!(false));
        set(&app, KEY, json!(true)).unwrap();
        let doc: VcNewDocument = serde_json::from_value(fields(&app)).unwrap();
        save_preset(&mut app, "Pixel Letter", &doc).unwrap();
        assert!(app.session.prefs.new_doc_presets.iter().find(|p| p.name == "Pixel Letter").expect("saved").align_new_objects_to_pixel_grid);
        create(&mut app);
        assert!(app.session.active().unwrap().doc.setup.align_new_objects_to_pixel_grid);
    }

    /// With Preferences › Units › General in millimeters, New Document opens on the most recent
    /// entry in millimeters even though that entry was made in points, and New Without Dialog makes
    /// the same document. An entry in pixels stays in pixels, as the screen presets do.
    #[test]
    fn the_dialog_opens_in_the_general_units_over_a_recent_entry() {
        let mut app = app();
        app.run("file.new", json!({"units": "Points"})).unwrap();
        general_units(&mut app, "Millimeters");
        assert_eq!(app.session.prefs.recent_new_docs[0].units, Unit::Points);
        open(&mut app);
        assert_eq!(fields(&app)["units"], json!("Millimeters"));
        assert_eq!(create_units(&mut app), Unit::Millimeters, "the dialog's Create");
        app.run("file.new", json!({"units": "Points"})).unwrap();
        create_without_dialog(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().doc.units, Unit::Millimeters, "New Without Dialog");
        app.run("file.new", json!({"preset": "Web 1920×1080"})).unwrap();
        open(&mut app);
        assert_eq!(fields(&app)["units"], json!("Pixels"));
        assert_eq!(create_units(&mut app), Unit::Pixels);
    }

    /// A Recent entry that is a saved preset in the units it was saved in opens in those units, so
    /// Create keeps the preset's name in Recent; the General unit is millimeters.
    #[test]
    fn a_recent_saved_preset_keeps_its_units_and_name() {
        let mut app = app();
        app.run("file.newPresets.save", json!({"name": "Zine", "width": "5.5 in", "height": "8.5 in", "units": "Inches"})).unwrap();
        app.run("file.new", json!({"preset": "Zine"})).unwrap();
        general_units(&mut app, "Millimeters");
        open(&mut app);
        assert_eq!((fields(&app)["basedOn"].clone(), fields(&app)["units"].clone()), (json!("Zine"), json!("Inches")));
        assert_eq!(create_units(&mut app), Unit::Inches);
        let recent = &app.session.prefs.recent_new_docs;
        assert_eq!((recent.len(), recent[0].name.as_str()), (1, "Zine"), "{recent:?}");
    }

    /// Millimeters chosen in the Units list stay chosen when a preset is picked after them, and reach
    /// the new document.
    #[test]
    fn units_chosen_in_the_menu_survive_tabs_and_presets() {
        let mut app = app();
        app.run("file.newPresets.save", json!({"name": "Zine", "width": "5.5 in", "height": "8.5 in", "units": "Inches"})).unwrap();
        open(&mut app);
        assert_eq!(fields(&app)["units"], json!("Points"));
        set(&app, "units", json!("Millimeters")).unwrap();
        set(&app, "preset", json!("Zine")).unwrap();
        let f = fields(&app);
        assert_eq!((f["units"].clone(), f["width"].clone()), (json!("Millimeters"), json!(Unit::Inches.to_pt(5.5))));
        assert_eq!(create_units(&mut app), Unit::Millimeters);
    }

    /// The units rules of picking a card: the default follows Preferences, a Recent entry keeps its
    /// own units unless in pixels, chosen units stay, a saved preset keeps its own, entries of one
    /// size in two units stay apart and the cards follow the chosen units.
    #[test]
    fn a_card_pick_keeps_the_chosen_units() {
        let mut app = app();
        general_units(&mut app, "Millimeters");
        assert_eq!(offered(&app).settings.units, Unit::Millimeters, "the default follows the general units");
        let entry = |units| DocSettings { units, name: "Custom".into(), ..DocSettings::default() };
        app.session.prefs.recent_new_docs = vec![entry(Unit::Points)];
        assert_eq!(first_settings(&app, "Recent").unwrap().settings.units, Unit::Millimeters, "a Recent entry takes the general units");
        app.session.prefs.recent_new_docs = vec![entry(Unit::Pixels)];
        assert_eq!(first_settings(&app, "Recent").unwrap().settings.units, Unit::Pixels, "unless it is in pixels");

        let card = |units| DocumentCard::new("Card", "line", VcNewDocument::of(entry(units)), CardSource::Recent);
        let setup = Setup::default();
        let mut doc =
            VcNewDocument { title: "Mine".into(), settings: DocSettings { units: Unit::Millimeters, ..DocSettings::default() }, turn: None };
        let mut memory = Memory::default();
        doc.apply_card(&card(Unit::Inches), &setup, &mut memory);
        assert_eq!((doc.settings.units, doc.title.as_str()), (Unit::Inches, "Mine"), "a saved preset keeps its own units, the name stays");
        doc.settings.units = Unit::Millimeters;
        doc.note_set("units", &mut memory);
        doc.apply_card(&card(Unit::Inches), &setup, &mut memory);
        assert_eq!(doc.settings.units, Unit::Millimeters, "chosen units stay when a card is picked");

        let (inches, points) = (VcNewDocument::of(entry(Unit::Inches)), VcNewDocument::of(entry(Unit::Points)));
        assert!(!inches.same_document(&points), "two entries of one size in two units stay apart");
        let built_in = newdoc::find(&app.session, "A4").unwrap();
        let shown = built_in_card(&built_in, &doc, None);
        assert_eq!(shown.settings.settings.units, Unit::Millimeters, "the cards follow the chosen units");
    }

    /// Edit opens a profile as a document and closes the dialog, Reset does not; the rows list every
    /// profile and say which were edited.
    #[test]
    fn manage_profiles_edits_and_resets_the_profiles() {
        let dir = std::env::temp_dir().join(format!("vc-ui-newdoc-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = app();
        assert!(!Setup::of(&app).has_profile_folder, "without a folder there is nothing to edit");
        app.session.attach_new_doc_profiles(dir.clone());
        let setup = Setup::of(&app);
        assert!(setup.has_profile_folder);
        assert_eq!(setup.profiles.len(), newdoc::profile_names().count());
        assert!(setup.profiles.iter().all(|(_, customized)| !customized));
        assert!(profile_action(&mut app, "profile.edit:Print"), "Edit closes the dialog");
        assert_eq!(app.session.active().unwrap().doc.title, "Print");
        assert!(dir.join("Print.vectorcraft").is_file());
        assert!(Setup::of(&app).profiles.iter().any(|(name, customized)| *name == "Print" && *customized));
        assert!(!profile_action(&mut app, "profile.reset:Print"), "Reset leaves the dialog open");
        assert!(!dir.join("Print.vectorcraft").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn orientation_remains_clickable_in_compact_web_windows() {
        use crate::i18n::LangExt;
        use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        for code in ["en", "de", "fr", "el"] {
            for width in [800.0, 640.0, 540.0, 799.0, 801.0] {
                let services = crate::Services { download: Some(Box::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
                let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
                app.session.prefs.interface_language = code.into();
                let lang = crate::i18n::Lang::from_code(code).unwrap();
                crate::i18n::set_current(lang);
                open(&mut app);
                set(&app, "width", json!(400)).unwrap();
                set(&app, "height", json!(600)).unwrap();
                set(&app, "units", json!("pt")).unwrap();
                let context = ctx(&app);
                let size = vec2(width, 500.0);
                let screen = Rect::from_min_size(Pos2::ZERO, size);
                let mut time = 0.0;
                let mut draw = |app: &mut VectorcraftApp, events| {
                    time += 0.25;
                    let out = crate::testing::app_frame(
                        app,
                        &context,
                        RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() },
                    );
                    assert_eq!(context.content_rect().size(), size, "the web host keeps the requested viewport");
                    out
                };
                for _ in 0..6 {
                    draw(&mut app, vec![]);
                }
                let original = fields(&app);
                let out = draw(&mut app, vec![]);
                let nodes = out.platform_output.accesskit_update.unwrap().nodes;
                let bounds = nodes
                    .iter()
                    .find(|(_, node)| node.role() == egui::accesskit::Role::RadioButton && node.label() == Some(crate::i18n::tr(lang, "Landscape")))
                    .and_then(|(_, node)| node.bounds())
                    .expect("Landscape is shown");
                let at = pos2(((bounds.x0 + bounds.x1) * 0.5) as f32, ((bounds.y0 + bounds.y1) * 0.5) as f32);
                draw(&mut app, vec![Event::PointerMoved(at)]);
                for pressed in [true, false] {
                    draw(&mut app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
                }
                draw(&mut app, vec![]);
                let turned = fields(&app);
                assert_eq!(turned["width"], original["height"], "{code} {width}: Landscape swaps the page width");
                assert_eq!(turned["height"], original["width"], "{code} {width}: Landscape swaps the page height");
                for _ in 0..3 {
                    draw(&mut app, vec![]);
                }
                for idle in 0..3 {
                    let output = draw(&mut app, vec![]);
                    assert_eq!(output.platform_output.num_completed_passes, 1, "{code} {width}: idle frame {idle} repeats layout");
                }
            }
        }
        crate::i18n::set_current(crate::i18n::Lang::EN);
    }
}
