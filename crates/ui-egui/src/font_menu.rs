//! The font family menu of the Character, Properties and Glyphs panels and Find Font's Replace
//! With: a search field over every family available, each row the family's name and a sample of
//! the selected text set in it. In the Character and Properties panels the row under the pointer
//! or reached with ↑/↓ previews its font on the selected text (one live interaction, nothing in
//! the history); a click or Enter applies it as one step, Escape or closing the menu puts the text
//! back. The Glyphs panel and Find Font only pick a font ([`picked`]): nothing is previewed on the
//! document. A star marks a favourite family, and the ★ filter lists only those. The Font
//! Technology filter's Variable lists only variable font families. Preferences › Type › Enable
//! in-menu font previews and Font Preview Size turn the samples off or size the rows. The menu lays
//! out its filters on the same four rows wherever it opens. However narrow its field, it opens at
//! least wide enough for those rows and for a family name up to 220 points wide with its sample
//! beside it, up to 60% of the window; each sample fills its row's height. Recently used families
//! appear first, in the order they were last applied, within the same search and filters as the
//! remaining families. In the Character and Properties panels and the Control bar,
//! the mouse wheel turned over the closed field applies the next family (or previous one) to the
//! selected text, one undo step per notch. The menu's order at the first turn stays fixed until the
//! pointer leaves the field or the text selection changes; steps stop at its first and last family.
//! The field takes the wheel only when the pointer was moved onto it after the field last moved or
//! was covered by a window or overlay, and the wheel stream began there.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{Color32, Sense, Ui, vec2};
use serde_json::json;

use vectorcraft_text::FontClass;

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;
use crate::widgets::{ArmedWheelField, WheelStream, WheelStreamOwner};

/// What happened in the menu this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum FontPick {
    /// A family (and one of its styles, when picked from the family's expanded list) was chosen.
    Chosen(String, Option<String>),
    /// The highlighted row changed: preview its family (and style).
    Preview(String, Option<String>),
    /// The menu closed (or came back to the current family) without a choice: undo the preview.
    EndPreview,
    /// The star of a family was clicked: add it to the favourites, or take it out.
    Favorite(String),
    /// The mouse wheel turned over the closed field: apply each of these families in turn, one
    /// undo step each.
    Wheel(Vec<String>),
}

/// The UI that contains a font family field.
#[derive(Clone, Copy, Debug)]
pub enum WheelOwner {
    Character,
    Properties,
    Control,
    /// The Character popover attached to this Control bar link.
    ControlPopover(egui::Id),
    /// The Contextual Task Bar's type controls.
    TaskBar,
}

impl WheelOwner {
    fn visible(self, app: &VectorcraftApp, ctx: &egui::Context) -> bool {
        if crate::panels::character::text_style(app).is_none() {
            return false;
        }
        let panel = match self {
            Self::Control | Self::ControlPopover(_) | Self::Properties if app.session.tool_id() == "artboard" => return false,
            Self::Control => return app.ui.control_bar && app.ui.screen_mode < 2,
            Self::ControlPopover(anchor) => {
                return app.ui.control_bar && app.ui.screen_mode < 2 && widgets::popover_is_open(ctx, anchor);
            }
            Self::TaskBar => {
                return app.ui.task_bar
                    && app.ui.screen_mode < 3
                    && crate::canvas::task_bar_shows_type(app)
                    && crate::canvas::task_bar_shows_fonts(ctx);
            }
            Self::Character => "character",
            Self::Properties => {
                let selected = app.session.active().and_then(|document| document.selection.objects.first().and_then(|id| document.doc.node(*id)));
                if !selected.is_some_and(|n| matches!(&n.kind, vectorcraft_doc::NodeKind::Text(_))) {
                    return false;
                }
                "properties"
            }
        };
        app.ui.dock
            && app.ui.screen_mode < 3
            && (crate::panel_docking::showing(&app.ui, panel)
                || (app.ui.transient_panel.as_deref() == Some(panel) && crate::panel_docking::showing(&app.ui, "appearance")))
    }
}

/// How the menu looks (from the preferences and the favourites) and whether its closed field
/// takes the mouse wheel.
#[derive(Clone, Copy, Debug)]
pub struct MenuLook<'a> {
    /// Show each family's sample (Preferences › Type › Enable in-menu font previews).
    pub samples: bool,
    /// Preferences › Type › Show samples on white: each sample is drawn in dark type on a white
    /// area, whatever the interface's brightness (off by default).
    pub on_white: bool,
    /// Preferences › Type › Show Font Names in English (on by default).
    pub english_names: bool,
    /// Row height in points (Font Preview Size).
    pub row: f32,
    /// The starred families.
    pub favorites: &'a [String],
    /// Recently applied families, newest first, limited by Number of Recent Fonts.
    pub recent: &'a [String],
    /// The mouse wheel turned over the closed field steps through the families in the menu's
    /// order ([`FontPick::Wheel`]): the Character and Properties panels and the Control bar.
    pub wheel: bool,
    /// Its owner, so a field hidden before the next layout cannot capture wheel input.
    pub wheel_owner: Option<WheelOwner>,
    /// The name screen readers give the closed field when no label stands beside it.
    pub name: Option<&'a str>,
}

impl Default for MenuLook<'static> {
    /// Samples on in the theme's colors, English names, medium rows, no favorites, no wheel.
    fn default() -> Self {
        Self {
            samples: true,
            on_white: false,
            english_names: true,
            row: ROW,
            favorites: &[],
            recent: &[],
            wheel: false,
            wheel_owner: None,
            name: None,
        }
    }
}

impl<'a> MenuLook<'a> {
    pub fn of(app: &'a VectorcraftApp) -> Self {
        let row = match app.session.prefs.font_preview_size.as_str() {
            "small" => 20.0,
            "large" => 32.0,
            _ => ROW,
        };
        Self {
            samples: app.session.prefs.font_preview,
            on_white: app.session.prefs.font_samples_on_white,
            english_names: app.session.prefs.font_names_in_english,
            row,
            favorites: &app.ui.favorite_fonts,
            recent: app.recent_fonts(),
            wheel: false,
            wheel_owner: None,
            name: None,
        }
    }
}

/// Height of a row, in points (Font Preview Size: Medium).
const ROW: f32 = 24.0;
/// Width of the star at the end of a family's row.
const STAR: f32 = 18.0;
/// The least share of a row the family's name takes when the row is too narrow for the widest
/// name and [`SAMPLE_W`]; the sample fills the rest.
const NAME_SHARE: f32 = 0.5;
/// Size of a family's name in its row.
const NAME_SIZE: f32 = 12.0;
/// The widest name the menu widens itself for. A longer name takes more of a row only in a menu
/// wider than that.
pub(crate) const NAME_MAX: f32 = 220.0;
/// Space between a family's name and its sample.
const NAME_GAP: f32 = 8.0;
/// How much wider than the sum of its characters' advances a name may lay out once shaped
/// (kerning, ligatures, contextual forms), as a share of that sum.
const SHAPING: f32 = 0.1;
/// The room the menu keeps for a sample beside the widest name.
const SAMPLE_W: f32 = 150.0;
/// The greatest share of the window's width the menu takes.
const WINDOW_SHARE: f32 = 0.6;
/// Size of the filters' labels.
const FILTER_SIZE: f32 = 11.0;
/// Width of the disclosure triangle of a family with several styles.
const DISCLOSURE: f32 = 14.0;
/// How far a family's style rows are indented past the triangle.
const STYLE_INDENT: f32 = 12.0;
/// The list's greatest height, under the search field and filters.
const LIST_HEIGHT: f32 = 600.0;
/// Samples rendered (or asked for) per frame, across every menu, so a long list doesn't stall the UI.
const SAMPLES_PER_FRAME: usize = 4;
/// Samples rendering at once (native).
#[cfg(not(target_arch = "wasm32"))]
const MAX_PENDING: usize = 8;
/// What a sample shows when the font lacks the selected text's characters (or there is none).
const FALLBACK_SAMPLE: &str = "Sample";
/// The script filter's choices: all, Japanese, Latin, Thai.
const SCRIPTS: usize = 4;

fn script_label(i: usize) -> &'static str {
    match i {
        // CJK families use the `japanese` font trait; Chinese UI calls this 汉字 (#2206).
        1 => crate::i18n::tr_ctx(crate::i18n::current(), "font-script", "Japanese"),
        2 => tl!("Latin"),
        3 => tl!("Thai"),
        _ => tl!("All Scripts"),
    }
}

/// The Font Technology filter's one choice: variable fonts. An adjective for fonts, not the
/// Variables panel's noun (`変数`), hence its context.
fn variable_label() -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "font-technology", "Variable")
}

/// Does a family of `traits` pass script filter `script` ([`SCRIPTS`])? Latin: neither Japanese
/// nor Thai.
fn script_matches(script: usize, traits: vectorcraft_text::FontTraits) -> bool {
    match script {
        1 => traits.japanese,
        2 => !traits.japanese && !traits.thai,
        3 => traits.thai,
        _ => true,
    }
}

fn class_label(c: FontClass) -> &'static str {
    match c {
        FontClass::Serif => tl!("Serif / Mincho"),
        FontClass::Sans => tl!("Sans Serif / Gothic"),
        FontClass::Rounded => tl!("Rounded"),
        FontClass::Script => tl!("Script / Brush"),
        FontClass::Monospaced => tl!("Monospaced"),
        FontClass::Decorative => tl!("Decorative"),
        FontClass::Other => tl!("Other"),
    }
}

/// A row of the list: a family, or one of the styles of an expanded family.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Heading(&'static str),
    Family(String),
    Style(String, String),
}

impl Row {
    fn family(&self) -> Option<&str> {
        match self {
            Row::Heading(_) => None,
            Row::Family(f) | Row::Style(f, _) => Some(f),
        }
    }
    fn style(&self) -> Option<&str> {
        match self {
            Row::Heading(_) | Row::Family(_) => None,
            Row::Style(_, s) => Some(s),
        }
    }
    fn pick(&self) -> Option<(String, Option<String>)> {
        Some((self.family()?.to_string(), self.style().map(str::to_string)))
    }
}

/// Recent families keep their application order, once each, and only while still available and
/// passing the filters. The other families keep the database's order. Headings share the virtual
/// list's row height, but never act as fonts or take keyboard focus.
fn grouped_rows(families: &[String], recent: &[String], expanded: &[String], styles: impl Fn(&str) -> Vec<String>) -> Vec<Row> {
    let mut first: Vec<&String> = vec![];
    for name in recent {
        if let Some(f) = families.iter().find(|f| f.eq_ignore_ascii_case(name))
            && !first.iter().any(|other| other.eq_ignore_ascii_case(f))
        {
            first.push(f);
        }
    }
    let mut rows = vec![];
    let append = |rows: &mut Vec<Row>, f: &String| {
        rows.push(Row::Family(f.clone()));
        if expanded.iter().any(|e| e.eq_ignore_ascii_case(f)) {
            rows.extend(styles(f).into_iter().map(|s| Row::Style(f.clone(), s)));
        }
    };
    // Build the family rows before adding headings, so the same expansion logic serves both groups.
    for f in &first {
        append(&mut rows, f);
    }
    let boundary = rows.len();
    for f in families.iter().filter(|f| !first.contains(f)) {
        append(&mut rows, f);
    }
    if boundary > 0 {
        if boundary < rows.len() {
            rows.insert(boundary, Row::Heading("Fonts"));
        }
        rows.insert(0, Row::Heading("Recent Fonts"));
    }
    rows
}

/// Keep keyboard/pointer focus on the same font after filtering, loading new fonts or
/// expanding a family's styles. A row index on its own can now name a different font.
fn remap_highlight(before: &[Row], selected: Option<usize>, after: &[Row]) -> Option<usize> {
    let row = before.get(selected?)?;
    after.iter().position(|candidate| candidate == row)
}

/// What the rows were listed for: the query, the script, kind, font technology (variable only) and
/// favourites filters, Show Font Names in English, the expanded families, favourites, recent
/// history and font list generation.
type RowsKey = (String, usize, Option<FontClass>, bool, bool, bool, Vec<String>, Vec<String>, Vec<String>, u64);

/// The menu's state between frames.
#[derive(Clone, Default)]
struct MenuState {
    /// The pass the list was last drawn in, and the pass it opened in.
    last: u64,
    opened: u64,
    /// The family the text had as the menu opened. A preview shows another one in the document
    /// (and the menu's caller), but this one stays the current family: hovering it ends the
    /// preview (#563).
    current: String,
    /// The highlighted row and the query and filters it belongs to.
    highlight: Option<usize>,
    query: String,
    /// The filters (kept from one opening to the next, and the same in every font menu:
    /// [`Filters`]): script (0..[`SCRIPTS`]), kind, font technology (variable fonts only) and
    /// favorites only.
    script: usize,
    class: Option<FontClass>,
    variable_only: bool,
    favorites_only: bool,
    /// Families whose styles are listed (lowercase).
    expanded: Vec<String>,
    /// What was previewed last (None: nothing is previewed).
    previewed: Option<(String, Option<String>)>,
    /// The rows listed, and what for: built again only when that changes, not every frame.
    rows: Arc<[Row]>,
    rows_key: Option<RowsKey>,
    /// The height of the search field and filters last frame.
    header: f32,
    /// The width of the widest row of filters without wrapping, last frame.
    filters: f32,
    /// The families the wheel steps through, listed at its first notch over the closed field and
    /// kept for that text selection while the pointer stays on it: each step makes its family a recent one, which would
    /// otherwise move it in the order.
    wheel_order: Option<Arc<[String]>>,
    wheel_target: Option<WheelTarget>,
}

/// The filters of [`MenuState`], one set for every font menu (the Character panel's, the Control
/// bar's, the Properties panel's…): a filter chosen in one menu is on in all of them. They last
/// for the session.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Filters {
    script: usize,
    class: Option<FontClass>,
    variable_only: bool,
    favorites_only: bool,
}

impl Filters {
    fn of(st: &MenuState) -> Self {
        Self { script: st.script, class: st.class, variable_only: st.variable_only, favorites_only: st.favorites_only }
    }

    fn apply(self, st: &mut MenuState) {
        st.script = self.script;
        st.class = self.class;
        st.variable_only = self.variable_only;
        st.favorites_only = self.favorites_only;
    }
}

/// Where the filters every font menu shares are kept ([`Filters`]).
fn filters_id() -> egui::Id {
    egui::Id::unique("font-menu-filters")
}

/// The font family menu showing `current`, `width` points wide, rows sampling `sample` (the
/// selected text; `None`: a generic sample).
pub fn font_menu(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    width: f32,
    sample: Option<&str>,
    look: MenuLook,
) -> Option<FontPick> {
    let state_id = ui.scope_id().with(&id).with("font-menu");
    let mut st: MenuState = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    ui.data(|d| d.get_temp::<Filters>(filters_id())).unwrap_or_default().apply(&mut st);
    let target = ui.data(|d| d.get_temp(wheel_fields_id(ui.ctx().viewport_id()).with("target")));
    let target_changed = st.wheel_target != target;
    if target_changed {
        st.wheel_order = None;
        st.wheel_target = target;
    }
    let height = st.header + LIST_HEIGHT;
    let state = &mut st;
    // The search field and filters stay in place over the rows, which scroll. The rows are all
    // one height (a family's styles too), so only those in view are laid out: there are thousands
    // of families.
    let (field, shown) = widgets::combo_menu_field_named(ui, &id, look.name, current, width, |menu| {
        menu.height(height).show_rows_with_header(
            |ui| {
                // Wider than a narrow field, so the filters keep their rows and long names show.
                ui.set_min_width(menu_width(ui, state, look));
                header(ui, state_id, state, current, look)
            },
            look.row,
            |h| h.rows.len(),
            |ui, range, h| rows(ui, range, h, sample, look),
        )
    });
    let drawn = shown.is_some();
    if target_changed {
        ui.data_mut(|d| d.remove::<f32>(field.id.with("wheel")));
    }
    let mut out = shown.flatten();
    if !drawn && out.is_none() && st.previewed.take().is_some() {
        // Closed (Escape, a click outside) with a preview showing.
        out = Some(FontPick::EndPreview);
    }
    let turns = remember_owned_wheel_field(ui, &field, look.wheel && !drawn && field.enabled(), state_id, look.wheel_owner);
    if !look.wheel || !field.enabled() {
        st.wheel_order = None;
        ui.data_mut(|d| d.remove::<f32>(field.id.with("wheel")));
    } else if out.is_none() || drawn {
        if !turns.is_empty() {
            let mut own = vectorcraft_text::FontDb::global().canonical(current, "").0;
            let mut steps = Vec::new();
            for turn in turns {
                match turn {
                    WheelInput::Leave => st.wheel_order = None,
                    WheelInput::Notches(notches) => {
                        if st.wheel_order.is_none() {
                            st.wheel_order = Some(wheel_order(&st, look).into());
                        }
                        if let Some(order) = st.wheel_order.as_deref() {
                            let next = wheel_steps(order, &own, notches);
                            if let Some(last) = next.last() {
                                own.clone_from(last);
                            }
                            steps.extend(next);
                        }
                    }
                }
            }
            if !steps.is_empty() {
                if drawn {
                    // These turns preceded the opener. The popup cancels back to the newly
                    // applied family, and any preview drawn this pass has not been applied.
                    st.current.clone_from(&own);
                    st.highlight = st.rows.iter().position(|row| matches!(row, Row::Family(f) if f.eq_ignore_ascii_case(&own)));
                    st.previewed = None;
                }
                out = Some(FontPick::Wheel(steps));
            }
        }
        let queued = ui.data(|d| {
            d.get_temp::<WheelFields>(wheel_fields_id(ui.ctx().viewport_id()))
                .is_some_and(|fields| fields.queued.iter().any(|event| event.field == field.id))
        });
        if drawn || (!field.contains_pointer() && !queued) {
            st.wheel_order = None;
            ui.data_mut(|d| d.remove::<f32>(field.id.with("wheel")));
        }
    }
    if matches!(out, Some(FontPick::Chosen(..))) {
        st.previewed = None;
    }
    ui.data_mut(|d| {
        d.insert_temp(filters_id(), Filters::of(&st));
        d.insert_temp(state_id, st);
    });
    out
}

/// Does the menu list family `f` for search `query` (lowercase) under the filters of `st`?
fn listed(db: &vectorcraft_text::FontDb, f: &str, query: &str, st: &MenuState, look: MenuLook) -> bool {
    let label = db.family_display_name(f, look.english_names);
    if !(query.is_empty() || f.to_lowercase().contains(query) || label.to_lowercase().contains(query)) {
        return false;
    }
    if st.favorites_only && !look.favorites.iter().any(|x| x.eq_ignore_ascii_case(f)) {
        return false;
    }
    if st.script == 0 && st.class.is_none() && !st.variable_only {
        return true;
    }
    let traits = db.family_traits(f);
    script_matches(st.script, traits) && st.class.is_none_or(|c| c == traits.class) && (!st.variable_only || traits.variable)
}

/// The families the closed field's wheel steps through, in the open menu's order with no search:
/// the recent families first, then the others, under the menu's filters.
fn wheel_order(st: &MenuState, look: MenuLook) -> Vec<String> {
    let db = vectorcraft_text::FontDb::global();
    let families: Vec<_> = db.menu_family_list().iter().filter(|f| listed(db, f, "", st, look)).cloned().collect();
    grouped_rows(&families, look.recent, &[], |_| vec![])
        .into_iter()
        .filter_map(|r| match r {
            Row::Family(f) => Some(f),
            Row::Heading(_) | Row::Style(..) => None,
        })
        .collect()
}

/// The families `notches` turns of the wheel step to from family `current` in `order`, one per
/// notch: forward (down the menu) when positive, back when negative. The steps stop at either end
/// of the order, and there are none when `current` is not in it.
fn wheel_steps(order: &[String], current: &str, notches: i32) -> Vec<String> {
    let Some(at) = order.iter().position(|f| f.eq_ignore_ascii_case(current)) else { return vec![] };
    let last = order.len().saturating_sub(1);
    let n = notches.unsigned_abs() as usize;
    let to = if notches >= 0 { at.saturating_add(n).min(last) } else { at.saturating_sub(n) };
    if to >= at { order.iter().skip(at + 1).take(to - at).cloned().collect() } else { order.iter().take(at).skip(to).rev().cloned().collect() }
}

#[derive(Clone)]
enum WheelInput {
    Leave,
    Notches(i32),
}

#[derive(Clone, Copy)]
struct WheelField {
    id: egui::Id,
    menu: egui::Id,
    owner: Option<WheelOwner>,
    rect: egui::Rect,
    layer: egui::LayerId,
}

#[derive(Clone)]
struct QueuedWheel {
    #[cfg(not(target_arch = "wasm32"))]
    frame: u64,
    field: egui::Id,
    #[cfg(not(target_arch = "wasm32"))]
    menu: egui::Id,
    #[cfg(not(target_arch = "wasm32"))]
    position: egui::Pos2,
    rect: egui::Rect,
    #[cfg(not(target_arch = "wasm32"))]
    event: egui::Event,
    #[cfg(not(target_arch = "wasm32"))]
    amount: f32,
    reset_after: bool,
    positions_after: Vec<egui::Pos2>,
}

#[derive(Clone)]
struct ReplayWheel {
    #[cfg(not(target_arch = "wasm32"))]
    target: egui::LayerId,
    position: egui::Pos2,
    event: egui::Event,
}

#[derive(Clone, Default)]
struct WheelFields {
    pass: u64,
    pending_frame: u64,
    activation_pass: u64,
    activating: Vec<egui::Id>,
    pointer: Option<egui::Pos2>,
    fields: Vec<WheelField>,
    pending: HashMap<egui::Id, Vec<WheelInput>>,
    queued: Vec<QueuedWheel>,
    replay: Vec<ReplayWheel>,
    armed: Option<ArmedWheelField>,
    stream: WheelStream,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct WheelLayers(Arc<dyn Fn(egui::Pos2) -> Option<egui::LayerId> + Send + Sync>);

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct WheelLayerSnapshot;

#[cfg(not(target_arch = "wasm32"))]
impl egui::Plugin for WheelLayerSnapshot {
    fn debug_name(&self) -> &'static str {
        "font wheel viewport layers"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        let viewport = ctx.viewport_id();
        if viewport == egui::ViewportId::ROOT {
            return;
        }
        let key = wheel_fields_id(viewport);
        let Some(mut fields) = ctx.data(|d| d.get_temp::<WheelFields>(key)) else { return };
        let frame = ctx.cumulative_frame_nr_for(viewport);
        let mut ready = Vec::new();
        fields.queued.retain(|event| {
            if event.frame < frame {
                ready.push(event.clone());
                false
            } else {
                true
            }
        });
        let replayed = fields.replay.len();
        for event in ready {
            let current = fields.fields.iter().find(|field| field.id == event.field);
            let layer = current.map(|field| field.layer);
            let opened_after_wheel = current.is_none()
                && ctx.data(|d| d.get_temp::<MenuState>(event.menu)).is_some_and(|menu| menu.opened > 0 && menu.opened == fields.pass);
            let active = opened_after_wheel
                || current.is_some_and(|field| field.rect.contains(event.position) && ctx.layer_id_at(event.position) == Some(field.layer));
            let left = !active || event.reset_after || event.positions_after.iter().any(|position| ctx.layer_id_at(*position) != layer);
            let id = event.field.with("wheel");
            if !active {
                ctx.data_mut(|d| d.remove::<f32>(id));
                fields.stream.release(event.field);
                if let Some(target) = ctx.layer_id_at(event.position) {
                    fields.replay.push(ReplayWheel { target, position: event.position, event: event.event });
                }
            } else {
                let notches = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0) + event.amount;
                let whole = notches.trunc();
                ctx.data_mut(|d| d.insert_temp(id, notches - whole));
                if whole != 0.0 {
                    fields.pending.entry(event.field).or_default().push(WheelInput::Notches(whole as i32));
                }
            }
            if left {
                ctx.data_mut(|d| d.remove::<f32>(id));
                fields.pending.entry(event.field).or_default().push(WheelInput::Leave);
            }
        }
        if fields.replay.len() > replayed {
            ctx.request_repaint();
        }
        ctx.data_mut(|d| d.insert_temp(key, fields));
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        let viewport = ctx.viewport_id();
        if viewport == egui::ViewportId::ROOT || ctx.data(|d| d.get_temp::<WheelFields>(wheel_fields_id(viewport))).is_none() {
            return;
        }
        let current_layers = ctx.viewport(|viewport| {
            viewport.this_pass.widgets.layers().filter_map(|(layer, widgets)| (!widgets.is_empty()).then_some(*layer)).collect::<Vec<_>>()
        });
        let layers = ctx.memory(|m| {
            m.layer_ids()
                .filter(|layer| current_layers.contains(layer))
                .filter(|layer| m.areas().is_visible(layer))
                .filter_map(|layer| m.area_rect(layer.id).map(|rect| (layer, rect)))
                .collect::<Vec<_>>()
        });
        let layers = layers
            .into_iter()
            .filter(|(layer, _)| egui::AreaState::load(ctx, layer.id).is_some_and(|state| state.interactable))
            .map(|(layer, rect)| (layer, ctx.layer_transform_to_global(layer).map_or(rect, |transform| transform * rect)))
            .collect::<Vec<_>>();
        ctx.data_mut(|d| {
            d.insert_temp(
                wheel_fields_id(viewport).with("layers"),
                WheelLayers(Arc::new(move |p| layers.iter().rev().find(|(_, rect)| rect.contains(p)).map(|(layer, _)| *layer))),
            )
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ensure_native_wheel_layers(ctx: &egui::Context) {
    if ctx.with_plugin(|_: &mut WheelLayerSnapshot| ()).is_none() {
        ctx.add_plugin(WheelLayerSnapshot);
    }
}

fn wheel_fields_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("font-wheel-fields").with(viewport)
}

fn mark_queued_wheel_leaves(fields: &mut WheelFields, point: Option<egui::Pos2>) {
    let mut marked = HashSet::new();
    for event in fields.queued.iter_mut().rev() {
        if marked.insert(event.field) {
            match point {
                Some(position) if event.rect.contains(position) => event.positions_after.push(position),
                _ => event.reset_after = true,
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn wheel_field_layer_for_test(ctx: &egui::Context, viewport: egui::ViewportId, point: egui::Pos2) -> Option<egui::LayerId> {
    ctx.data(|data| data.get_temp::<WheelFields>(wheel_fields_id(viewport)))
        .and_then(|fields| fields.fields.iter().find(|field| field.rect.contains(point)).map(|field| field.layer))
}

#[cfg(test)]
pub(crate) fn offset_wheel_fields_for_test(ctx: &egui::Context, viewport: egui::ViewportId, offset: egui::Vec2) {
    let key = wheel_fields_id(viewport);
    ctx.data_mut(|data| {
        let mut fields = data.get_temp::<WheelFields>(key).expect("registered wheel fields");
        for field in &mut fields.fields {
            field.rect = field.rect.translate(offset);
        }
        data.insert_temp(key, fields);
    });
}

#[derive(Clone, PartialEq)]
struct WheelTarget {
    input: crate::state::InputGeneration,
    workspace: u64,
    selection: Vec<vectorcraft_doc::NodeId>,
    range: Option<(vectorcraft_doc::NodeId, usize, usize)>,
}

/// Pending turns and field rectangles belong to their input target and visible workspace.
pub(crate) fn sync_wheel_target(app: &VectorcraftApp, ctx: &egui::Context, viewport: egui::ViewportId) {
    let target = WheelTarget {
        input: app.input_generation(),
        workspace: app.ui.docking_generation,
        selection: app.session.active().map_or_else(Vec::new, |document| document.selection.objects.clone()),
        range: crate::panels::character::text_editing(app),
    };
    let key = wheel_fields_id(viewport);
    let (previous, mut fields) = ctx.data(|d| (d.get_temp::<WheelTarget>(key.with("target")), d.get_temp::<WheelFields>(key)));
    if previous.is_some_and(|old| old != target) {
        // A new target keeps the armed field and the running wheel stream.
        fields = fields.map(|old| WheelFields { pass: old.pass, armed: old.armed, stream: old.stream, ..Default::default() });
    }
    let mut hidden = Vec::new();
    if let Some(fields) = &mut fields {
        fields.fields.retain(|field| {
            if field.owner.is_none_or(|owner| owner.visible(app, ctx)) {
                true
            } else {
                hidden.push(*field);
                false
            }
        });
        for field in &hidden {
            fields.pending.remove(&field.id);
        }
    }
    ctx.data_mut(|d| {
        if let Some(fields) = fields {
            d.insert_temp(key, fields);
        } else {
            d.remove::<WheelFields>(key);
        }
        for field in hidden {
            d.remove::<f32>(field.id.with("wheel"));
            if let Some(mut state) = d.get_temp::<MenuState>(field.menu) {
                state.wheel_order = None;
                d.insert_temp(field.menu, state);
            }
        }
        d.insert_temp(key.with("target"), target);
    });
}

/// Remember the visible closed fields for the next input batch and take this field's turns.
#[cfg(test)]
fn remember_wheel_field(ui: &Ui, resp: &egui::Response, enabled: bool) -> Vec<WheelInput> {
    remember_owned_wheel_field(ui, resp, enabled, resp.id, None)
}

fn remember_owned_wheel_field(ui: &Ui, resp: &egui::Response, enabled: bool, menu: egui::Id, owner: Option<WheelOwner>) -> Vec<WheelInput> {
    let key = wheel_fields_id(ui.ctx().viewport_id());
    let pass = ui.ctx().cumulative_pass_nr();
    // A window or overlay over the pointer covers the field even when no input arrives.
    let pointer = ui.data_mut(|d| d.get_temp_mut_or_default::<WheelFields>(key).pointer).or_else(|| ui.ctx().input(|i| i.pointer.latest_pos()));
    let covered = pointer.is_some_and(|p| resp.interact_rect.contains(p) && ui.ctx().layer_id_at(p) != Some(resp.layer_id));
    ui.data_mut(|d| {
        let mut fields: WheelFields = d.get_temp(key).unwrap_or_default();
        if fields.pass != pass {
            fields.pass = pass;
            fields.fields.clear();
        }
        fields.fields.retain(|f| f.id != resp.id);
        if let Some(armed) = &mut fields.armed
            && armed.id == resp.id
        {
            if armed.rect == resp.interact_rect && pass.saturating_sub(armed.pass) <= 1 && !covered {
                armed.pass = pass;
            } else {
                fields.armed = None;
                fields.stream.release(resp.id);
            }
        }
        if enabled {
            fields.fields.push(WheelField { id: resp.id, menu, owner, rect: resp.interact_rect, layer: resp.layer_id });
        }
        let turns = fields.pending.remove(&resp.id).unwrap_or_default();
        d.insert_temp(key, fields);
        turns
    })
}

/// Take the closed font fields' wheel events before egui smooths and combines them. Other
/// events stay in the input, so scrolling elsewhere keeps its own delta and event order. A field
/// takes a wheel stream only when the stream begins with the pointer over it and the pointer was
/// moved onto it after the field last moved or was covered. A stream that began anywhere else
/// stays there until it ends, and a stream that leaves its field does not return to it.
pub(crate) fn take_raw_wheel(ctx: &egui::Context, raw: &mut egui::RawInput) {
    let key = wheel_fields_id(raw.viewport_id);
    let Some(mut fields) = ctx.data(|d| d.get_temp::<WheelFields>(key)) else { return };
    if !ctx.input(|i| i.raw.viewports.contains_key(&raw.viewport_id)) {
        ctx.data_mut(|d| d.remove::<WheelFields>(key));
        #[cfg(not(target_arch = "wasm32"))]
        ctx.data_mut(|d| {
            d.remove::<WheelLayers>(key.with("layers"));
        });
        return;
    }
    let frame = ctx.cumulative_frame_nr_for(raw.viewport_id);
    let pass = ctx.cumulative_pass_nr_for(raw.viewport_id);
    if pass < fields.pass || pass.saturating_sub(fields.pass) > 1 {
        fields.fields.clear();
        fields.pointer = None;
        fields.armed = None;
        fields.stream.release_any();
    }
    let now = raw.time.unwrap_or_else(|| ctx.input_for(raw.viewport_id, |i| i.time) + f64::from(raw.predicted_dt));
    if fields.pending_frame != frame {
        fields.pending_frame = frame;
        fields.pending.clear();
    }
    let line = ctx.options(|o| o.input_options.line_scroll_speed).max(1.0);
    let mut pointer = fields.pointer.or_else(|| ctx.input_for(raw.viewport_id, |i| i.pointer.latest_pos()));
    #[cfg(not(target_arch = "wasm32"))]
    let native_layers = ctx.data(|d| d.get_temp::<WheelLayers>(key.with("layers")));
    let over = |field: &WheelField, point: Option<egui::Pos2>| {
        point.is_some_and(|p| {
            if !field.rect.contains(p) {
                return false;
            }
            if ctx.viewport_id() == raw.viewport_id {
                ctx.layer_id_at(p) == Some(field.layer)
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    native_layers.as_ref().is_some_and(|layers| (layers.0)(p) == Some(field.layer))
                }
                #[cfg(target_arch = "wasm32")]
                {
                    false
                }
            }
        })
    };
    // An activation batch belongs to the popup, including a release after a prior press.
    if fields.activation_pass != pass {
        fields.activation_pass = pass;
        fields.activating.clear();
        let pressed = ctx.input(|i| if i.pointer.button_down(egui::PointerButton::Primary) { i.pointer.press_origin() } else { None });
        if let Some(field) = fields.fields.iter().find(|f| over(f, pressed)) {
            fields.activating.push(field.id);
        }
    }
    let focused = ctx.memory(|m| m.focused());
    let mut captured = false;
    raw.events.retain(|event| {
        let opening = match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => {
                fields.fields.iter().rev().find(|f| over(f, Some(*pos))).map(|f| f.id)
            }
            egui::Event::Key { key: egui::Key::Enter | egui::Key::Space, pressed: true, .. } if raw.focused => {
                focused.filter(|id| fields.fields.iter().any(|f| f.id == *id))
            }
            _ => None,
        };
        if let Some(id) = opening
            && !fields.activating.contains(&id)
        {
            fields.activating.push(id);
        }
        if matches!(event, egui::Event::MouseWheel { .. }) {
            fields.stream.note_input(now);
        }
        match event {
            egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. } => {
                if pointer != Some(*p) {
                    let pass = fields.pass;
                    fields.armed = fields.fields.iter().rev().find(|f| over(f, Some(*p))).map(|f| ArmedWheelField { id: f.id, rect: f.rect, pass });
                    if let Some(WheelStreamOwner::Field(id)) = fields.stream.owner
                        && !fields.fields.iter().any(|f| f.id == id && over(f, Some(*p)))
                    {
                        fields.stream.release(id);
                    }
                }
                mark_queued_wheel_leaves(&mut fields, Some(*p));
                for field in &fields.fields {
                    if pointer.is_some_and(|point| field.rect.contains(point))
                        && !field.rect.contains(*p)
                        && !fields.queued.iter().any(|event| event.field == field.id)
                    {
                        ctx.data_mut(|d| d.remove::<f32>(field.id.with("wheel")));
                        fields.pending.entry(field.id).or_default().push(WheelInput::Leave);
                    }
                }
                pointer = Some(*p);
            }
            egui::Event::PointerGone => {
                pointer = None;
                fields.armed = None;
                fields.stream.release_any();
                mark_queued_wheel_leaves(&mut fields, None);
                for field in &fields.fields {
                    if !fields.queued.iter().any(|event| event.field == field.id) {
                        ctx.data_mut(|d| d.remove::<f32>(field.id.with("wheel")));
                        fields.pending.entry(field.id).or_default().push(WheelInput::Leave);
                    }
                }
            }
            egui::Event::MouseWheel { unit, delta, modifiers, source, .. } if modifiers.is_none() && delta.y != 0.0 => {
                let under = fields.fields.iter().rev().find(|f| !fields.activating.contains(&f.id) && over(f, pointer)).copied();
                let starts_on_field = under
                    .filter(|f| fields.armed.is_some_and(|armed| armed.id == f.id) && !matches!(source, egui::MouseWheelSource::Momentum))
                    .map(|f| f.id);
                let owner = *fields.stream.owner.get_or_insert(starts_on_field.map_or(WheelStreamOwner::Elsewhere, WheelStreamOwner::Field));
                if let Some(field) = under.filter(|f| owner == WheelStreamOwner::Field(f.id)) {
                    let amount = -match unit {
                        egui::MouseWheelUnit::Point => delta.y / line,
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                    };
                    if cfg!(not(target_arch = "wasm32")) && ctx.viewport_id() != raw.viewport_id {
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(position) = pointer {
                            fields.queued.push(QueuedWheel {
                                frame,
                                field: field.id,
                                menu: field.menu,
                                position,
                                rect: field.rect,
                                event: event.clone(),
                                amount,
                                reset_after: false,
                                positions_after: Vec::new(),
                            });
                        }
                    } else {
                        let id = field.id.with("wheel");
                        let notches = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0) + amount;
                        let whole = notches.trunc();
                        ctx.data_mut(|d| d.insert_temp(id, notches - whole));
                        if whole != 0.0 {
                            fields.pending.entry(field.id).or_default().push(WheelInput::Notches(whole as i32));
                        }
                    }
                    captured = true;
                    return false;
                }
                fields.stream.owner = Some(WheelStreamOwner::Elsewhere);
            }
            egui::Event::MouseWheel { delta, .. } if *delta != egui::Vec2::ZERO => {
                fields.stream.owner.get_or_insert(WheelStreamOwner::Elsewhere);
            }
            _ => {}
        }
        true
    });
    if !fields.replay.is_empty() {
        let mut replay = Vec::new();
        for wheel in std::mem::take(&mut fields.replay) {
            #[cfg(not(target_arch = "wasm32"))]
            let target_still_visible = native_layers.as_ref().is_some_and(|layers| (layers.0)(wheel.position) == Some(wheel.target));
            #[cfg(target_arch = "wasm32")]
            let target_still_visible = false;
            if target_still_visible {
                replay.push(egui::Event::PointerMoved(wheel.position));
                replay.push(wheel.event);
            }
        }
        if !replay.is_empty() {
            replay.push(pointer.map_or(egui::Event::PointerGone, egui::Event::PointerMoved));
            raw.events.splice(0..0, replay);
        }
    }
    fields.pointer = pointer;
    ctx.data_mut(|d| d.insert_temp(key, fields));
    if captured {
        ctx.data_mut(|d| d.insert_temp(widgets::taken_from_raw_id(raw.viewport_id), now));
        ctx.request_repaint();
    }
}

/// What the menu's header hands its rows.
struct Listed<'s> {
    st: &'s mut MenuState,
    /// The rows listed.
    rows: Arc<[Row]>,
    /// The current family by its own name.
    current: String,
    /// Enter was pressed; ↑/↓/←/→ moved the highlight; the menu is opening.
    enter: bool,
    keyed: bool,
    opening: bool,
}

/// The menu's contents outside a menu, for the unit tests: the header over its rows, as
/// [`craft_ui::combo::Menu::show_rows_with_header`] lays them out.
#[cfg(test)]
fn list(ui: &mut Ui, state_id: egui::Id, st: &mut MenuState, current: &str, sample: Option<&str>, look: MenuLook) -> Option<FontPick> {
    let h = header(ui, state_id, st, current, look);
    let total = h.rows.len();
    egui::ScrollArea::vertical().max_height(ui.available_height()).show_rows(ui, look.row, total, |ui, range| rows(ui, range, h, sample, look)).inner
}

/// The top of the open menu, which stays in place: the search field, the filters, and the rows
/// they list.
fn header<'s>(ui: &mut Ui, state_id: egui::Id, st: &'s mut MenuState, current: &str, look: MenuLook) -> Listed<'s> {
    let t = Tokens::get(ui.ctx());
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    // Each time the menu opens: no query, the current family highlighted and in view (the
    // filters stay as they were).
    let pass = ui.ctx().cumulative_pass_nr();
    let opening_now = st.last + 1 < pass || st.last == 0;
    if opening_now {
        st.opened = pass;
        st.highlight = None;
        st.query.clear();
        st.current = current.to_string();
    }
    st.last = pass;
    // For [`end_stale_preview`]: the menu is open, in this layer.
    ui.ctx().data_mut(|d| d.insert_temp(open_id(), (pass, ui.layer_id())));
    let field = state_id.with("query");
    if opening_now {
        ui.data_mut(|d| d.insert_temp(field, String::new()));
    }
    // A popup sizes itself invisibly in its first pass.
    let opening = pass <= st.opened + 1;
    // Arrow keys move the highlight and open or close a family's styles (taken before the
    // search field sees them).
    let key = |i: &mut egui::InputState, k| i.consume_key(egui::Modifiers::NONE, k);
    let (up, down, left, right) =
        ui.input_mut(|i| (key(i, egui::Key::ArrowUp), key(i, egui::Key::ArrowDown), key(i, egui::Key::ArrowLeft), key(i, egui::Key::ArrowRight)));
    if ui.memory(|m| m.focused().is_none()) {
        ui.memory_mut(|m| m.request_focus(field.with("edit")));
    }
    let query = widgets::search_field(ui, field, tl!("Search")).to_lowercase();
    let refilter = filters(ui, st, &t);
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let db = vectorcraft_text::FontDb::global();
    // The current family by its own name (a document may name it in Japanese: ヒラギノ角ゴシック),
    // as it was before any preview.
    let current = db.canonical(&st.current, "").0;
    let current = current.as_str();
    // The generation is read first: fonts that load meanwhile make the next frame list them.
    let key: RowsKey = (
        query.clone(),
        st.script,
        st.class,
        st.variable_only,
        st.favorites_only,
        look.english_names,
        st.expanded.clone(),
        look.favorites.to_vec(),
        look.recent.to_vec(),
        db.generation(),
    );
    if st.rows_key.as_ref() != Some(&key) {
        let families: Vec<_> = db.menu_family_list().iter().filter(|f| listed(db, f, &query, st, look)).cloned().collect();
        let rows = grouped_rows(&families, look.recent, &st.expanded, |f| db.styles(f));
        // Filter changes (including un-starring a favourite) may remove rows; expanding
        // another family or a background font scan can insert rows before the highlight.
        // Never preview an unrelated family because the old numeric index now points to it.
        st.highlight = remap_highlight(&st.rows, st.highlight, &rows);
        st.rows = rows.into();
        st.rows_key = Some(key);
    }
    let rows = st.rows.clone();
    if query != st.query || refilter {
        st.query = query.clone();
        st.highlight = None;
    }
    if st.highlight.is_none() && query.is_empty() {
        st.highlight = rows.iter().position(|r| matches!(r, Row::Family(f) if f.eq_ignore_ascii_case(current)));
    }
    let mut keyed = false;
    if (up || down) && !rows.is_empty() {
        let selectable = |(_, r): &(usize, &Row)| r.family().is_some();
        let next = match (st.highlight, down) {
            (None, _) => rows.iter().enumerate().find(selectable),
            (Some(i), true) => rows.iter().enumerate().skip(i.saturating_add(1)).find(selectable),
            (Some(i), false) => rows.iter().enumerate().take(i).rev().find(selectable),
        };
        st.highlight = next.map(|(i, _)| i).or(st.highlight);
        keyed = true;
    }
    // → opens the highlighted family's styles, ← closes them (from the family or one of them).
    if let Some(family) = st.highlight.and_then(|i| rows.get(i)).and_then(Row::family).filter(|_| left || right) {
        let f = family.to_lowercase();
        let open = st.expanded.contains(&f);
        if right && !open && db.styles(family).len() > 1 {
            st.expanded.push(f);
        } else if left && open {
            st.expanded.retain(|e| *e != f);
            st.highlight = rows.iter().position(|r| *r == Row::Family(family.to_string()));
        }
        keyed = true;
    }
    st.header = ui.min_rect().height();
    if rows.is_empty() {
        widgets::dim_label(ui, tl!("No matching fonts"));
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    // The scroll bar beside the rows, not over them: a floating one covered the stars at their
    // right ends, so a favourite was hard to click (#2277).
    ui.spacing_mut().scroll = widgets::list_scroll_style();
    Listed { st, rows, current: current.to_string(), enter, keyed, opening }
}

/// One of the filters' chips.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Chip {
    /// A script ([`SCRIPTS`]).
    Script(usize),
    Favorites,
    AnyKind,
    Class(FontClass),
    /// The Font Technology label, which filters nothing.
    Technology,
    Variable,
}

/// The filters, row by row, the same wherever the menu opens: the scripts; favorites, any kind
/// and the first three kinds; the other kinds; font technology.
fn filter_rows() -> [Vec<Chip>; 4] {
    let (first, rest) = FontClass::ALL.split_at(3);
    [
        (0..SCRIPTS).map(Chip::Script).collect(),
        [Chip::Favorites, Chip::AnyKind].into_iter().chain(first.iter().copied().map(Chip::Class)).collect(),
        rest.iter().copied().map(Chip::Class).collect(),
        vec![Chip::Technology, Chip::Variable],
    ]
}

/// The filters on their rows ([`filter_rows`]): a row wraps only where the menu is narrower than
/// it. Records in `st` the width of the widest row unwrapped; whether a filter changed.
fn filters(ui: &mut Ui, st: &mut MenuState, t: &Tokens) -> bool {
    let mut refilter = false;
    let mut widest = 0.0_f32;
    for row in filter_rows() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
            let gap = ui.spacing().item_spacing.x;
            let text = |s: &str| egui::RichText::new(s).size(FILTER_SIZE);
            let mut width = -gap;
            for chip in row {
                let resp = match chip {
                    Chip::Script(i) => ui.selectable_label(st.script == i, text(script_label(i))),
                    Chip::Favorites => ui.selectable_label(st.favorites_only, text("★")).on_hover_text(tl!("Favorites")),
                    Chip::AnyKind => ui.selectable_label(st.class.is_none(), text(tl!("Any Kind"))),
                    Chip::Class(c) => ui.selectable_label(st.class == Some(c), text(class_label(c))),
                    Chip::Technology => ui.label(text(tl!("Font Technology")).color(t.text_dim)),
                    Chip::Variable => ui.selectable_label(st.variable_only, text(variable_label())),
                };
                width += resp.rect.width() + gap;
                if !resp.clicked() {
                    continue;
                }
                match chip {
                    Chip::Script(i) => st.script = i,
                    Chip::Favorites => st.favorites_only = !st.favorites_only,
                    Chip::AnyKind => st.class = None,
                    Chip::Class(c) => st.class = if st.class == Some(c) { None } else { Some(c) },
                    Chip::Technology => continue,
                    Chip::Variable => st.variable_only = !st.variable_only,
                }
                refilter = true;
            }
            widest = widest.max(width);
        });
    }
    st.filters = widest;
    refilter
}

/// The width the open menu takes at least: the widest row of filters last frame, and the widest
/// name up to [`NAME_MAX`] with room for its sample, the star and the scroll bar beside it, within
/// [`WINDOW_SHARE`] of the window. The menu is never narrower than its field.
fn menu_width(ui: &Ui, st: &MenuState, look: MenuLook) -> f32 {
    let sample = if look.samples { SAMPLE_W } else { 0.0 };
    let names = name_width(ui, look.english_names, &st.rows).min(NAME_MAX);
    let row = DISCLOSURE + names + NAME_GAP + sample + STAR + 2.0 + widgets::list_scroll_style().allocated_width();
    // Rounding in the chips' widths must not wrap the widest row.
    let filters = if st.filters > 0.0 { st.filters.ceil() + 1.0 } else { 0.0 };
    filters.max(row).min(ui.ctx().content_rect().width() * WINDOW_SHARE)
}

/// The width of the widest name the menu lists after the disclosure triangle: the family names as
/// Show Font Names in English shows them, and the style names among `rows` after their indent.
fn name_width(ui: &Ui, english: bool, rows: &[Row]) -> f32 {
    let font = egui::FontId::proportional(NAME_SIZE);
    let measure = |name: &str| ui.painter().layout_no_wrap(name.to_string(), font.clone(), Color32::WHITE).size().x;
    let styles = rows.iter().filter_map(Row::style).map(|s| STYLE_INDENT + measure(s)).fold(0.0, f32::max);
    family_name_width(ui, english).max(styles)
}

/// What the widest family name was measured for: the font list's generation, Show Font Names in
/// English, and the number of fonts the interface draws with, which grows as faces for other
/// scripts are added to it.
fn family_name_key(ui: &Ui, english: bool) -> (u64, bool, usize) {
    let faces = ui.ctx().fonts(|f| f.definitions().font_data.len());
    (vectorcraft_text::FontDb::global().generation(), english, faces)
}

/// The id under which the widest family name's width is kept with its [`family_name_key`].
fn family_name_id() -> egui::Id {
    egui::Id::unique("font-menu-name-width")
}

/// The width of the widest family name the menu lists, labeled as Show Font Names in English
/// (`english`) labels it; worked out again only when its [`family_name_key`] changes.
fn family_name_width(ui: &Ui, english: bool) -> f32 {
    let key = family_name_key(ui, english);
    if let Some((k, w)) = ui.data(|d| d.get_temp::<((u64, bool, usize), f32)>(family_name_id()))
        && k == key
    {
        return w;
    }
    let db = vectorcraft_text::FontDb::global();
    let w = widest_label(ui, db.menu_family_list().iter().map(|f| db.family_display_name(f, english)).collect());
    ui.data_mut(|d| d.insert_temp(family_name_id(), (key, w)));
    w
}

/// The width of the widest of `labels` laid out at the names' size. Each label's characters'
/// advances are added up, and the labels are laid out widest sum first, until no sum left could
/// reach the widest layout, allowing [`SHAPING`] for shaping.
fn widest_label(ui: &Ui, labels: Vec<String>) -> f32 {
    let font = egui::FontId::proportional(NAME_SIZE);
    let mut sums: Vec<(f32, String)> =
        ui.ctx().fonts_mut(|f| labels.into_iter().map(|l| (l.chars().map(|c| f.glyph_width(&font, c)).sum(), l)).collect());
    sums.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut w = 0.0_f32;
    for (sum, label) in sums {
        if sum * (1.0 + SHAPING) + 1.0 <= w {
            break;
        }
        w = w.max(ui.painter().layout_no_wrap(label, font.clone(), Color32::WHITE).size().x);
    }
    w
}

/// Where a row `width` wide puts its sample, after the name column: room for the widest name
/// (`widest`) and a gap, as far as that leaves [`SAMPLE_W`] and the star for the sample or
/// [`NAME_SHARE`] of the row for the name. Without samples the name runs to the star.
fn name_column(width: f32, widest: f32, samples: bool) -> f32 {
    if !samples {
        return width - STAR;
    }
    (DISCLOSURE + widest + NAME_GAP).min((width - STAR - 2.0 - SAMPLE_W).max(width * NAME_SHARE))
}

/// The disclosure triangle of a family with several styles; whether the pointer is on it.
fn disclosure(p: &egui::Painter, resp: &egui::Response, tri: egui::Rect, open: bool, t: &Tokens) -> bool {
    p.text(tri.center(), egui::Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, egui::FontId::proportional(11.0), t.text_dim);
    resp.interact_pointer_pos().is_some_and(|pos| tri.contains(pos))
}

/// The star at the end of a family's row, shown when it is a favourite, hovered or highlighted; whether the pointer is
/// on it.
fn star_cell(p: &egui::Painter, resp: &egui::Response, star: egui::Rect, (on, highlighted): (bool, bool), t: &Tokens) -> bool {
    let hot = resp.hover_pos().is_some_and(|pos| star.contains(pos));
    if on || hot || highlighted {
        let color = if on { t.accent } else { t.text_dim };
        p.text(star.center(), egui::Align2::CENTER_CENTER, if on { "★" } else { "☆" }, egui::FontId::proportional(12.0), color);
    }
    resp.interact_pointer_pos().is_some_and(|pos| star.contains(pos))
}

/// A heading row (Recent Fonts, Fonts), which cannot preview or apply a font.
fn heading_row(ui: &mut Ui, row_h: f32, label: &str, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
    // Existing translated menu labels; the heading cannot preview or apply a font.
    let label = match label {
        "Recent Fonts" => tl!("Recent Fonts"),
        _ => tl!("Fonts"),
    };
    ui.painter_at(rect).text(rect.left_center() + vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(11.0), t.text_dim);
}

/// The rows of the open menu in `range`, the ones in view: the menu's one scrolling part, under
/// the search field and filters (#555).
fn rows(ui: &mut Ui, range: std::ops::Range<usize>, h: Listed<'_>, sample: Option<&str>, look: MenuLook) -> Option<FontPick> {
    let Listed { st, rows, current, enter, keyed, opening } = h;
    let current = current.as_str();
    let row_h = look.row;
    let t = Tokens::get(ui.ctx());
    let ink = sample_ink(look.on_white, t.text);
    let db = vectorcraft_text::FontDb::global();
    let favorite = |f: &str| look.favorites.iter().any(|x| x.eq_ignore_ascii_case(f));
    let widest = name_width(ui, look.english_names, &rows);
    if rows.is_empty() {
        // A filter or a font rescan can remove the row whose font was previewed.
        // Do not leave that font applied while the list has no candidate to highlight.
        return st.previewed.take().map(|_| FontPick::EndPreview);
    }
    // Scrolls the list to row `i`'s place, laid out or not: the rows in view start at
    // `range.start`.
    let (left, width) = (ui.max_rect().left(), ui.max_rect().width());
    let top = ui.max_rect().top() - range.start as f32 * row_h;
    let scroll_to = |ui: &Ui, i: usize, align| {
        let place = egui::Rect::from_min_size(egui::pos2(left, top + i as f32 * row_h), vec2(width, row_h));
        ui.scroll_to_rect_animation(place, align, egui::style::ScrollAnimation::none());
    };
    // Open at the recent section even when the current family is farther down the list.
    // Otherwise centre the current font; subsequent key presses keep the highlight in view.
    if opening && matches!(rows.first(), Some(Row::Heading("Recent Fonts"))) {
        scroll_to(ui, 0, Some(egui::Align::TOP));
    } else if let Some(i) = st.highlight
        && (opening || keyed)
    {
        scroll_to(ui, i, opening.then_some(egui::Align::Center));
    }
    let mut chosen: Option<(String, Option<String>)> = None;
    let mut hovered: Option<usize> = None;
    let mut toggled: Option<String> = None;
    let mut starred: Option<String> = None;
    for i in range {
        let Some(row) = rows.get(i) else { continue };
        if let Row::Heading(label) = row {
            heading_row(ui, row_h, label, &t);
            continue;
        }
        let Some(family) = row.family() else { continue };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
        if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO || inp.smooth_scroll_delta != egui::Vec2::ZERO) {
            hovered = Some(i);
        }
        let p = ui.painter_at(rect);
        if st.highlight == Some(i) {
            p.rect_filled(rect, 2.0, t.selection);
        } else if matches!(row, Row::Family(f) if f.eq_ignore_ascii_case(current)) {
            p.rect_filled(rect, 2.0, t.hover);
        }
        // The disclosure triangle of a family with several styles.
        let tri = egui::Rect::from_min_size(rect.min, vec2(DISCLOSURE, row_h));
        let mut on_triangle = false;
        if let Row::Family(f) = row
            && db.styles(f).len() > 1
        {
            let open = st.expanded.iter().any(|e| e.eq_ignore_ascii_case(f));
            on_triangle = disclosure(&p, &resp, tri, open, &t);
            if resp.clicked() && on_triangle {
                toggled = Some(f.to_lowercase());
            }
        }
        // The star at the end of a family's row.
        let star = egui::Rect::from_min_max(egui::pos2(rect.max.x - STAR, rect.min.y), rect.max);
        let mut on_star = false;
        if let Row::Family(f) = row {
            let on = favorite(f);
            on_star = star_cell(&p, &resp, star, (on, st.highlight == Some(i)), &t);
            if resp.clicked() && on_star {
                starred = Some(f.clone());
            }
        }
        let indent = if row.style().is_some() { DISCLOSURE + STYLE_INDENT } else { DISCLOSURE };
        let name_w = name_column(rect.width(), widest, look.samples);
        let name_rect = egui::Rect::from_min_max(rect.min + vec2(indent, 0.0), egui::pos2(rect.min.x + name_w - 4.0, rect.max.y));
        // Family rows follow Show Font Names in English; style rows keep the style name (#394).
        let name = match row {
            Row::Style(_, style) => style.clone(),
            Row::Family(f) => db.family_display_name(f, look.english_names),
            Row::Heading(_) => continue,
        };
        ui.painter_at(name_rect).text(name_rect.left_center(), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(NAME_SIZE), t.text);
        // The sample takes the row's whole height.
        let sample_rect = egui::Rect::from_min_max(rect.min + vec2(name_w, 0.0), rect.max - vec2(STAR + 2.0, 0.0));
        if look.samples {
            // The paper is drawn while the sample is still rendering, and for a family whose sample fails.
            let area = sample_area(ui, sample_rect, ink, PAPER_INSET);
            if let Some(tex) = sample_texture(ui.ctx(), family, row.style(), sample, area.height()) {
                paint_sample(ui, area, &tex, ink);
            }
        }
        if resp.clicked() && !on_triangle && !on_star {
            chosen = row.pick();
        }
    }
    if let Some(f) = toggled {
        if st.expanded.contains(&f) {
            st.expanded.retain(|e| *e != f);
        } else {
            st.expanded.push(f);
        }
        // The rows change next frame: what the highlight points at is decided then.
        return None;
    }
    if let Some(i) = hovered {
        st.highlight = Some(i);
    }
    if let Some(f) = starred {
        return Some(FontPick::Favorite(f));
    }
    if enter && chosen.is_none() {
        chosen = st.highlight.and_then(|i| rows.get(i)).and_then(Row::pick).or_else(|| rows.iter().find_map(Row::pick));
    }
    if let Some((f, s)) = chosen {
        ui.close();
        return Some(FontPick::Chosen(f, s));
    }
    // The highlighted row is previewed; back on the current family, the preview goes.
    let lit = st.highlight.and_then(|i| rows.get(i)).and_then(Row::pick);
    match lit {
        Some((f, None)) if f == current => st.previewed.take().map(|_| FontPick::EndPreview),
        Some(pick) if st.previewed.as_ref() != Some(&pick) => {
            st.previewed = Some(pick.clone());
            Some(FontPick::Preview(pick.0, pick.1))
        }
        None => st.previewed.take().map(|_| FontPick::EndPreview),
        _ => None,
    }
}

// ---------- samples ----------

/// Width of the sample drawn at the end of a Type → Font menu item.
/// The room a Type → Font menu item keeps for its sample, after the family's name.
pub(crate) const MENU_SAMPLE_SIZE: egui::Vec2 = vec2(90.0, 16.0);

/// A generic sample of `family` drawn in `slot` (the room a Type → Font item of the in-window
/// menu bar keeps for it; the native macOS menu shows names only).
/// With `on_white` (Show samples on white), the sample is drawn in dark type on a white slot.
pub(crate) fn menu_item_sample(ui: &Ui, slot: egui::Rect, family: &str, on_white: bool) {
    let ink = sample_ink(on_white, Tokens::get(ui.ctx()).text_dim);
    let area = sample_area(ui, slot, ink, 0.0);
    if let Some(tex) = sample_texture(ui.ctx(), family, None, None, area.height()) {
        paint_sample(ui, area, &tex, ink);
    }
}

/// What a sample is drawn on with Show samples on white.
pub(crate) const SAMPLE_PAPER: Color32 = Color32::WHITE;

/// How a sample is drawn: in `theme` (an interface text color) on the menu's own background, or,
/// with Show samples on white, in the Light brightness's text color on [`SAMPLE_PAPER`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SampleInk {
    pub ink: Color32,
    pub paper: Option<Color32>,
}

pub(crate) fn sample_ink(on_white: bool, theme: Color32) -> SampleInk {
    if on_white {
        SampleInk { ink: Tokens::for_brightness(crate::theme::Brightness::Light).text, paper: Some(SAMPLE_PAPER) }
    } else {
        SampleInk { ink: theme, paper: None }
    }
}

/// Space between the left edge of a sample's paper and its type.
const PAPER_PAD: f32 = 3.0;

/// Space above and below a menu row's paper, where the row's highlight color shows.
pub(crate) const PAPER_INSET: f32 = 2.0;

/// The part of `slot` a sample is drawn in: `slot` itself, or, when `ink` has paper, the paper
/// painted in `slot` with `inset` points left above and below it.
fn sample_area(ui: &Ui, slot: egui::Rect, ink: SampleInk, inset: f32) -> egui::Rect {
    let Some(paper) = ink.paper else { return slot };
    let area = slot.shrink2(vec2(0.0, inset));
    if area.is_positive() {
        ui.painter_at(slot).rect_filled(area, 2.0, paper);
    }
    area
}

/// Draw sample `tex` at the left of `area`, centered vertically, cut at its right; on paper,
/// [`PAPER_PAD`] in from its left edge.
fn paint_sample(ui: &Ui, area: egui::Rect, tex: &egui::TextureHandle, ink: SampleInk) {
    let painter = ui.painter_at(area);
    let mut start = area.left_center();
    if ink.paper.is_some() {
        start.x += PAPER_PAD;
    }
    let size = tex.size_vec2() / ui.ctx().pixels_per_point();
    let at = egui::Rect::from_min_size(start - vec2(0.0, size.y * 0.5), size);
    let shown = at.intersect(area);
    if !shown.is_positive() {
        return;
    }
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2((shown.width() / size.x).clamp(0.0, 1.0), 1.0));
    painter.image(tex.id(), shown, uv, ink.ink);
}

type Key = (String, String, String, u32);

/// Samples kept before the cache starts over.
const MAX_SAMPLES: usize = 256;

#[derive(Default)]
struct Samples {
    ready: HashMap<Key, Option<egui::TextureHandle>>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: HashMap<Key, std::sync::mpsc::Receiver<Option<egui::ColorImage>>>,
    /// The pass the budget is for, and the samples started in it.
    pass: u64,
    started: usize,
}

impl Samples {
    /// Take one sample from this frame's budget ([`SAMPLES_PER_FRAME`]); `false` when it is spent
    /// (another frame is asked for, to go on).
    fn take_budget(&mut self, ctx: &egui::Context) -> bool {
        let pass = ctx.cumulative_pass_nr();
        if self.pass != pass {
            (self.pass, self.started) = (pass, 0);
        }
        if self.started >= SAMPLES_PER_FRAME {
            ctx.request_repaint();
            return false;
        }
        self.started += 1;
        true
    }
}

thread_local! {
    static SAMPLES: crate::graphics::TexCache<Samples> = crate::graphics::TexCache::default();
}

/// The sample of `family` for a row `height` points high: the selected text set in it (else a
/// generic sample), white with coverage as alpha (tinted when drawn). Rendered off the UI thread
/// on native (the worker asks for a frame when it is done), a few per frame; `None` until it is
/// ready.
fn sample_texture(ctx: &egui::Context, family: &str, style: Option<&str>, sample: Option<&str>, height: f32) -> Option<egui::TextureHandle> {
    let px = (height * ctx.pixels_per_point()).round().clamp(8.0, 96.0) as u32;
    let key: Key = (family.to_string(), style.unwrap_or("Regular").to_string(), sample.unwrap_or_default().to_string(), px);
    let texture = |img: egui::ColorImage| ctx.load_texture(format!("font-sample-{family}-{px}"), img, egui::TextureOptions::LINEAR);
    SAMPLES.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(t) = s.ready.get(&key) {
            return t.clone();
        }
        // Up to ~370 KB of texture each (96 px high, 10:1): keep a few screens' worth.
        if s.ready.len() > MAX_SAMPLES {
            s.ready.clear();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(rx) = s.pending.get(&key) {
                let img = match rx.try_recv() {
                    Ok(img) => img,
                    Err(std::sync::mpsc::TryRecvError::Empty) => return None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
                };
                s.pending.remove(&key);
                let tex = img.map(texture);
                s.ready.insert(key, tex.clone());
                return tex;
            }
            if s.pending.len() >= MAX_PENDING || !s.take_budget(ctx) {
                return None;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let (f, style, text) = (key.0.clone(), key.1.clone(), key.2.clone());
            let repaint = ctx.clone();
            let spawned = std::thread::Builder::new().name("font-sample".into()).spawn(move || {
                // The receiver may be gone (the cache was cleared): nothing to do then.
                let _ = tx.send(render_sample(&f, &style, &text, px));
                repaint.request_repaint();
            });
            if spawned.is_ok() {
                s.pending.insert(key, rx);
            } else {
                s.ready.insert(key, None);
            }
            None
        }
        // The web has no threads: a few samples per frame, on this one.
        #[cfg(target_arch = "wasm32")]
        {
            if !s.take_budget(ctx) {
                return None;
            }
            let tex = render_sample(&key.0, &key.1, &key.2, px).map(texture);
            s.ready.insert(key, tex.clone());
            tex
        }
    })
}

/// Longest sample, in pixels per pixel of height (wider samples are cut).
const MAX_ASPECT: f32 = 10.0;

/// `text` (or [`FALLBACK_SAMPLE`] when the font lacks some of its characters) set in `family`,
/// `px` pixels high: a coverage mask as white with alpha.
fn render_sample(family: &str, style: &str, text: &str, px: u32) -> Option<egui::ColorImage> {
    let db = vectorcraft_text::FontDb::global();
    let face: Arc<vectorcraft_text::FontFace> = db.face(family, style)?;
    let covers = |s: &str| !s.trim().is_empty() && s.chars().filter(|c| !c.is_whitespace()).all(|c| face.covers(c));
    let text = if covers(text) { text } else { FALLBACK_SAMPLE };
    if !covers(text) && !face.family.eq_ignore_ascii_case(family) {
        return None;
    }
    let (asc, desc) = face.vertical_metrics();
    let em = (asc + desc).max(1.0) / face.units_per_em();
    // The em box fills the height with a little air.
    let size = px as f64 * 0.8 / em;
    let style = vectorcraft_doc::CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size, ..Default::default() };
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, text, style);
    let layout = vectorcraft_text::layout(db, &t);
    let mut path = layout.to_bezpath();
    let b = layout.bounds;
    let w = (b.width().ceil() as u32 + 2).clamp(1, (px as f32 * MAX_ASPECT) as u32) as usize;
    let h = px as usize;
    // Baseline where the ascent fits the top (with the air above it).
    let baseline = px as f64 * 0.1 + asc / face.units_per_em() * size;
    path.apply_affine(vectorcraft_geom::Affine::translate((1.0 - b.x0.min(0.0), baseline)));
    let mask = crate::panels::glyphs::rasterize(&path, w, h);
    Some(egui::ColorImage::new([w, h], mask.iter().map(|&a| Color32::from_rgba_premultiplied(a, a, a, a)).collect()))
}

#[cfg(test)]
pub(crate) fn render_sample_for_test(family: &str, text: &str, px: u32) -> Option<egui::ColorImage> {
    render_sample(family, "Regular", text, px)
}

/// Waits for the samples rendering on this thread's behalf and keeps them, so the next frame
/// draws them.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn wait_for_samples(ctx: &egui::Context) {
    SAMPLES.with(|s| {
        let mut s = s.borrow_mut();
        let pending: Vec<_> = s.pending.drain().collect();
        for (key, rx) in pending {
            let tex =
                rx.recv().ok().flatten().map(|img| ctx.load_texture(format!("font-sample-{}-{}", key.0, key.3), img, egui::TextureOptions::LINEAR));
            s.ready.insert(key, tex);
        }
    });
}

// ---------- applying a pick ----------

/// Is a font previewed on the text (an interaction is open for it)?
const PREVIEWING: &str = "font-previewing";

/// Where the open font menu was drawn last: (pass, its popup's layer).
fn open_id() -> egui::Id {
    egui::Id::unique("font-menu-open")
}

/// Undo a preview whose menu is gone (its panel wasn't drawn: the text was deselected, the panel
/// closed) or is closing (Escape, a press outside the menu), before anything else sees this
/// frame's input: the canvas must not act inside the preview's interaction (a drag would be
/// rolled back with the preview). The shell calls it at the start of each frame.
pub(crate) fn end_stale_preview(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !crate::panels::pstate::<bool>(ctx, PREVIEWING) {
        return;
    }
    let drawn = ctx.data(|d| d.get_temp::<(u64, egui::LayerId)>(open_id()));
    // Drawn in the last pass: still open.
    let Some((_, layer)) = drawn.filter(|(pass, _)| pass + 1 >= ctx.cumulative_pass_nr()) else {
        apply(app, ctx, FontPick::EndPreview);
        return;
    };
    // (`layer_id_at` outside `input`: egui's context lock isn't re-entrant.)
    let (escape, press) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.pointer.any_pressed().then(|| i.pointer.interact_pos())));
    let leaving = escape || press.is_some_and(|at| at.is_none_or(|p| ctx.layer_id_at(p) != Some(layer)));
    if leaving {
        apply(app, ctx, FontPick::EndPreview);
    }
}

/// What the font menus preview on and apply to: the Type tool's selected range while it edits
/// text (the whole text when nothing is selected), else the selected text objects.
pub(crate) fn apply(app: &mut VectorcraftApp, ctx: &egui::Context, pick: FontPick) {
    let previewing = crate::panels::pstate::<bool>(ctx, PREVIEWING);
    match pick {
        FontPick::Preview(f, style) => {
            if !previewing {
                crate::panels::character::end_typing(app);
                if app.session.begin_interaction("Font").is_err() {
                    return;
                }
                crate::panels::set_pstate(ctx, PREVIEWING, true);
            }
            let (cmd, params) = font_command(app, &f, style.as_deref());
            if let Err(e) = app.session.preview(cmd, &params) {
                app.ui.status = e.to_string();
            }
            app.sync_views();
        }
        FontPick::EndPreview => {
            if previewing {
                app.session.cancel_interaction().ok();
                crate::panels::set_pstate(ctx, PREVIEWING, false);
                app.sync_views();
            }
        }
        FontPick::Favorite(f) => toggle_favorite(app, f),
        FontPick::Wheel(families) => {
            if previewing {
                app.session.cancel_interaction().ok();
                crate::panels::set_pstate(ctx, PREVIEWING, false);
            }
            crate::panels::character::end_typing(app);
            // Each family is its own step, keeping the style when the family has it, else the
            // closest it has.
            for f in families {
                let (cmd, params) = font_command(app, &f, None);
                if app.run(cmd, params).is_err() {
                    break;
                }
            }
        }
        FontPick::Chosen(f, style) => {
            if previewing {
                app.session.cancel_interaction().ok();
                crate::panels::set_pstate(ctx, PREVIEWING, false);
            }
            crate::panels::character::end_typing(app);
            let (cmd, params) = font_command(app, &f, style.as_deref());
            app.run(cmd, params).ok();
        }
    }
}

/// Star `family`, or take its star off (any case).
fn toggle_favorite(app: &mut VectorcraftApp, family: String) {
    let favs = &mut app.ui.favorite_fonts;
    match favs.iter().position(|x| x.eq_ignore_ascii_case(&family)) {
        Some(i) => {
            favs.remove(i);
        }
        None => favs.push(family),
    }
}

/// For a font menu that only picks a font and never changes the document (Find Font's Replace
/// With, the Glyphs panel): the family (and style) chosen, if any. A star is toggled here; the
/// highlighted row is not previewed.
pub(crate) fn picked(app: &mut VectorcraftApp, pick: Option<FontPick>) -> Option<(String, Option<String>)> {
    match pick? {
        FontPick::Chosen(f, style) => Some((f, style)),
        FontPick::Favorite(f) => {
            toggle_favorite(app, f);
            None
        }
        FontPick::Preview(..) | FontPick::EndPreview | FontPick::Wheel(_) => None,
    }
}

/// The command that sets font `family` (and `style`, else the closest the family has) on what the
/// menus act on.
fn font_command(app: &VectorcraftApp, family: &str, style: Option<&str>) -> (&'static str, serde_json::Value) {
    let mut p = json!({"font": family});
    if let Some(s) = style {
        p["style"] = json!(s);
    }
    // After Select All in threaded text: every frame of the story.
    if let Some(first) = crate::panels::character::text_story(app).and_then(|story| story.first().copied()) {
        p["id"] = json!(first.0);
        p["story"] = json!(true);
        return ("text.setRangeStyle", p);
    }
    match crate::panels::character::text_editing(app) {
        Some((id, a, b)) => {
            p["id"] = json!(id.0);
            if b > a {
                p["start"] = json!(a);
                p["end"] = json!(b);
            }
            ("text.setRangeStyle", p)
        }
        None => ("text.setStyle", p),
    }
}

/// The text the menus sample: the Type tool's selected characters, else the first line of the
/// first selected text object, at most a dozen characters.
pub(crate) fn sample_text(app: &VectorcraftApp) -> Option<String> {
    let doc = &app.session.active()?.doc;
    let text = match crate::panels::character::text_editing(app) {
        Some((id, a, b)) if b > a => match &doc.node(id)?.kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text().get(a..b).map(str::to_string),
            _ => None,
        },
        _ => app.session.active()?.selection.objects.iter().find_map(|id| match &doc.node(*id)?.kind {
            vectorcraft_doc::NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        }),
    }?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(12).collect())
}

#[cfg(test)]
mod tests;
