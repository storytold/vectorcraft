//! OpenType panel: per-character OpenType features (ligatures, alternates, swashes, small caps,
//! ordinals, fractions, figure style, position, stylistic sets from the panel or its menu, and with
//! the East Asian options the menu's Proportional Metrics, H or V Style and Roman Italics) for the
//! selected text (every frame of each selected threaded story) or the Type tool's selected
//! characters.

use craft::behavior::Mixed;
use craft::keymap::CommandRef;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::Field;
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, NodeKind};
use vectorcraft_text::{EXTRA_FEATURES, FontDb, OtFeatures, StylisticSet};

use super::character::{any_style, shared, text_editing, text_style};
use crate::VectorcraftApp;

/// The panel menu's Reset OpenType Features command.
const RESET: &str = "ui.openType.reset";
/// The panel menu's feature toggles, run with `{tag}`.
const TOGGLE: &str = "ui.openType.toggle";
/// The panel menu's Proportional Metrics item, with Show East Asian Options.
const METRICS: &str = "ui.openType.proportionalMetrics";

/// The feature a checkbox row toggles: a named flag, or a tag of [`EXTRA_FEATURES`].
#[derive(Clone, Copy)]
enum Flag {
    Field(fn(&mut OtFeatures) -> &mut bool),
    Extra(&'static str),
}

/// A checkbox row: label and the feature it toggles.
type Toggle = (&'static str, Flag);

pub(crate) const FIGURES: [&str; 5] = ["Default Figure", "Tabular Lining", "Proportional Lining", "Proportional Oldstyle", "Tabular Oldstyle"];

pub(crate) const POSITIONS: [&str; 5] = ["Default Position", "Superscript", "Subscript", "Numerator", "Denominator"];

/// The features the Position menu's entries after Default Position turn on, in menu order.
pub(crate) const POSITION_TAGS: [&str; 4] = ["sups", "subs", "numr", "dnom"];

/// The bit of [`OtFeatures::extra`] that turns `tag` on (0 for a tag outside [`EXTRA_FEATURES`]).
fn extra_bit(tag: &str) -> u32 {
    EXTRA_FEATURES.iter().position(|t| *t == tag).map_or(0, |i| 1 << i)
}

impl Flag {
    fn get(self, f: &OtFeatures) -> bool {
        match self {
            Flag::Field(field) => *field(&mut f.clone()),
            Flag::Extra(tag) => f.extra & extra_bit(tag) != 0,
        }
    }

    fn put(self, f: &mut OtFeatures, on: bool) {
        match self {
            Flag::Field(field) => *field(f) = on,
            Flag::Extra(tag) => f.set_extra(extra_bit(tag), on),
        }
    }

    /// The tag a menu command names this flag by; named flags have none.
    fn tag(self) -> Option<&'static str> {
        match self {
            Flag::Extra(tag) => Some(tag),
            Flag::Field(_) => None,
        }
    }
}

/// The Figure menu entry `f` shows: Default Figure, Tabular Lining, Proportional Lining
/// (`pnum` and `lnum`), Proportional Oldstyle or Tabular Oldstyle.
pub(crate) fn figure(f: &OtFeatures) -> usize {
    let proportional_lining = extra_bit("pnum") | extra_bit("lnum");
    match (f.tabular_figures, f.oldstyle_figures) {
        (false, false) if f.extra & proportional_lining == proportional_lining => 2,
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 3,
        (true, true) => 4,
    }
}

/// `f` with Figure menu entry `i` chosen: the entry's features on, the other figure features off.
/// Tabular Lining keeps an `lnum` the type already has and clears `pnum`; Proportional Oldstyle
/// keeps a `pnum` and clears `lnum`; Default Figure and Tabular Oldstyle clear both.
fn set_figure(f: &mut OtFeatures, i: usize) {
    f.tabular_figures = i == 1 || i == 4;
    f.oldstyle_figures = i >= 3;
    f.set_extra(
        match i {
            1 => extra_bit("pnum"),
            3 => extra_bit("lnum"),
            _ => extra_bit("pnum") | extra_bit("lnum"),
        },
        false,
    );
    if i == 2 {
        f.set_extra(extra_bit("pnum") | extra_bit("lnum"), true);
    }
}

/// The Position menu entry `f` shows: Superscript (`sups`), Subscript (`subs`), Numerator
/// (`numr`) or Denominator (`dnom`) when `f` turns that one on, Default Position when it turns
/// none of them on, and `None` when it turns on more than one.
pub(crate) fn position(f: &OtFeatures) -> Option<usize> {
    let mut on = POSITION_TAGS.iter().enumerate().filter(|(_, t)| f.extra & extra_bit(t) != 0).map(|(i, _)| i + 1);
    match (on.next(), on.next()) {
        (None, _) => Some(0),
        (Some(i), None) => Some(i),
        (Some(_), Some(_)) => None,
    }
}

/// `f` with Position menu entry `i` chosen: the entry's feature on and the other three off.
/// Default Position turns all four off.
fn set_position(f: &mut OtFeatures, i: usize) {
    for (k, tag) in POSITION_TAGS.iter().enumerate() {
        f.set_extra(extra_bit(tag), k + 1 == i);
    }
}

/// The Position menu entry for the characters the panel acts on, or `None` when they differ or
/// one of them turns on more than one position.
fn shown_position(app: &VectorcraftApp) -> Option<usize> {
    shared(app, |s| position(&OtFeatures::from_tags(s.features.iter().map(String::as_str)))).flatten()
}

/// The Figure menu entry for the characters the panel acts on, or `None` when they differ.
fn shown_figure(app: &VectorcraftApp) -> Option<usize> {
    shared(app, |s| figure(&OtFeatures::from_tags(s.features.iter().map(String::as_str))))
}

/// Whether `flag` is on for every character a toggle changes (`Some(true)`), for none of them
/// (`Some(false)`), or only for some (`None`). At a Type tool caret, and after Select All in a
/// threaded story, the command styles all the text ([`run_features`]), so every character of it
/// counts, as in [`edit`]. There, and with the Selection tool, a run with no text (an empty
/// frame's) counts only when no run has text.
fn shown_flag(app: &VectorcraftApp, flag: Flag) -> Option<bool> {
    shown(app, |s| flag.get(&OtFeatures::from_tags(s.features.iter().map(String::as_str))))
}

/// Whether `on` holds for every character a menu item changes (`Some(true)`), for none of them
/// (`Some(false)`), or only for some (`None`), read as [`shown_flag`] reads a feature.
fn shown(app: &VectorcraftApp, on: impl Fn(&CharStyle) -> bool) -> Option<bool> {
    let runs = match whole_text_runs(app) {
        Some(runs) => Some(runs),
        // With the Selection tool: every run of every frame of the selected stories.
        None if text_editing(app).is_none() => Some(super::character::selected_story_texts(app).into_iter().flat_map(|t| t.runs.iter()).collect()),
        None => None,
    };
    if let Some(mut runs) = runs {
        // A run with no text, such as an empty frame's, counts only when no run has text.
        if runs.iter().any(|r| !r.text.is_empty()) {
            runs.retain(|r| !r.text.is_empty());
        }
        let Some(first) = runs.first() else {
            return shared(app, &on);
        };
        let v = on(&first.style);
        return runs.iter().all(|r| on(&r.style) == v).then_some(v);
    }
    shared(app, &on)
}

/// The runs the panel's commands style as a whole while the Type tool edits text: those of every
/// frame of the story after Select All in threaded text, else at a caret (an empty selection)
/// those of the text object. `None` for a range in one text object, or with no Type tool editing.
fn whole_text_runs(app: &VectorcraftApp) -> Option<Vec<&vectorcraft_doc::TextRun>> {
    let (id, a, b) = text_editing(app)?;
    let st = app.session.active()?;
    let ids = match super::character::text_story(app) {
        Some(story) => story,
        None if b <= a => vec![id],
        None => return None,
    };
    let runs = ids.into_iter().filter_map(|id| match &st.doc.node(id)?.kind {
        NodeKind::Text(t) => Some(t.runs.iter()),
        _ => None,
    });
    Some(runs.flatten().collect())
}

/// A change one of the panel's controls makes to the features of each character it acts on.
#[derive(Clone, Copy)]
enum Change {
    /// The Figure menu entry chosen.
    Figure(usize),
    /// The Position menu entry chosen.
    Position(usize),
    /// A checkbox row turned on or off.
    Toggle(Flag, bool),
    /// A row menu's Reset OpenType Features: the panel menu's default.
    Reset,
}

impl Change {
    fn apply(self, f: &mut OtFeatures) {
        match self {
            Change::Figure(i) => set_figure(f, i),
            Change::Position(i) => set_position(f, i),
            Change::Toggle(flag, on) => flag.put(f, on),
            Change::Reset => *f = OtFeatures::default(),
        }
    }
}

/// Make `change` to the features of each character the panel acts on, from the character's own
/// features (`featureChanges`), unless it changes none of them. At a Type tool caret, and after
/// Select All in a threaded story, the command styles all the text, so every character of it is
/// checked ([`whole_text_runs`]).
fn edit(app: &mut VectorcraftApp, change: Change) {
    let same = |s: &CharStyle| {
        let before = OtFeatures::from_tags(s.features.iter().map(String::as_str));
        let mut after = before;
        change.apply(&mut after);
        after == before
    };
    let unchanged = match whole_text_runs(app) {
        Some(runs) => Some(runs.iter().all(|r| same(&r.style))),
        None => shared(app, same),
    };
    if unchanged != Some(true) {
        run_features(app, "featureChanges", OtFeatures::changes(|f| change.apply(f)));
    }
}

/// Apply `f` to the selection's features (the Type tool range, or whole objects).
fn set(app: &mut VectorcraftApp, f: OtFeatures) {
    run_features(app, "features", f.to_tags());
}

/// Send `tags` as the `key` param (`features` or `featureChanges`) for the Type tool's range or
/// caret, or else for every frame of the selected stories
/// ([`super::character::selected_stories`]).
fn run_features(app: &mut VectorcraftApp, key: &str, tags: Vec<String>) {
    // After Select All in threaded text: every frame of the story.
    let story = super::character::text_story(app).and_then(|story| story.first().copied());
    let (cmd, mut p) = match (text_editing(app), story) {
        (Some(_), Some(first)) => ("text.setRangeStyle", json!({ "id": first.0, "story": true })),
        (Some((id, a, b)), None) if b > a => ("text.setRangeStyle", json!({ "id": id.0, "start": a, "end": b })),
        (Some((id, _, _)), None) => ("text.setRangeStyle", json!({ "id": id.0 })),
        (None, _) => {
            let mut p = json!({});
            super::character::target_selected_stories(app, &mut p);
            ("text.setStyle", p)
        }
    };
    p[key] = json!(tags);
    if let Err(e) = app.run(cmd, p) {
        app.ui.status = e;
    }
}

/// A shown value for a binding: the shared value, or mixed where the characters differ.
fn mixed<T>(shown: Option<T>) -> Mixed<T> {
    shown.map_or(Mixed::Mixed, Mixed::Same)
}

/// A labelled select over `names` that shows entry `shown` (mixed for `None`) → the entry chosen
/// in this pass.
fn choose(ui: &mut Ui, label: &'static str, names: &[&'static str], shown: Option<usize>) -> Option<usize> {
    let entries: Vec<Entry<u8>> = names.iter().zip(0u8..).map(|(name, i)| Entry::new(i, *name)).collect();
    let shown = shown.and_then(|i| u8::try_from(i).ok());
    Field::new(label)
        .show(ui, |ui, _| Select::new(label, Bind::given(mixed(shown)), &entries).show(ui))
        .inner
        .and_then(|r| r.events.committed().map(|i| usize::from(*i)))
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((st, _)) = text_style(app) else {
        Text::new(tl!("Select text to set its OpenType features.")).shade(Shade::Muted).show(ui);
        return;
    };
    let mut change = None;
    if let Some(i) = choose(ui, tl!("Figure:"), &FIGURES, shown_figure(app)) {
        change = Some(Change::Figure(i));
    }
    if let Some(i) = choose(ui, tl!("Position:"), &POSITIONS, shown_position(app)) {
        change = Some(Change::Position(i));
    }
    Divider::horizontal().show(ui);
    // A row whose characters differ shows a dash, and a click turns the feature on for all of them.
    // A secondary click keeps the text selection and opens that row's menu.
    for (label, flag) in rows() {
        let shown = shown_flag(app, flag);
        let out = Checkbox::new(Bind::given(mixed(shown))).label(label).show(ui);
        if let Some(on) = out.events.committed() {
            change = Some(Change::Toggle(flag, *on));
        }
        // The row menu's choice is applied with the rest at the end of the pass, so the rows drawn
        // after it still read the state the pass started with.
        if let Some(c) = Menu::context(label, &out.response).show(ui, |ui, _| feature_row_menu(app, ui, flag, shown)).inner.flatten() {
            change = Some(c);
        }
    }
    let sets = set_entries(app, &st);
    let opener = Button::new(tl!("Stylistic Sets")).enabled(!sets.is_empty()).show(ui);
    Menu::new(tl!("Stylistic Sets")).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
        for (set, flag, on) in &sets {
            if MenuItem::new(set_label(set)).check(*on).keep_open(true).show(ui).ran {
                change = Some(Change::Toggle(*flag, !on));
            }
        }
    });
    if tracking_drops_ligatures(app) {
        Text::new(tl!("Ligatures are off while the text is tracked.")).shade(Shade::Muted).show(ui);
    }
    match change {
        // Reset writes the default as the whole feature list, as the panel menu's entry does.
        Some(Change::Reset) => set(app, OtFeatures::default()),
        Some(c) => edit(app, c),
        None => {}
    }
}

/// The menu a secondary click on one feature row opens. Turn Off when the row is on
/// (`shown` is `Some(true)`); Turn On when it is off or mixed, through the same toggle a click
/// uses, so a mixed row turns the feature on for every character the panel acts on. Reset
/// OpenType Features applies the panel menu's default and is enabled on the same condition.
fn feature_row_menu(app: &VectorcraftApp, ui: &mut Ui, flag: Flag, shown: Option<bool>) -> Option<Change> {
    let on = shown == Some(true);
    let mut chosen = None;
    if MenuItem::new(if on { tl!("Turn Off") } else { tl!("Turn On") }).show(ui).ran {
        chosen = Some(Change::Toggle(flag, !on));
    }
    if MenuItem::new(tl!("Reset OpenType Features")).enabled(text_style(app).is_some()).show(ui).ran {
        chosen = Some(Change::Reset);
    }
    chosen
}

/// Whether tracking turns off the standard ligatures of any character the panel acts on: one with
/// Standard Ligatures on, tracked past [`vectorcraft_text::ligatures_suppressed_by`]'s limit and
/// without an explicit `liga` or `clig` tag ([`vectorcraft_text::explicit_ligatures`]).
fn tracking_drops_ligatures(app: &VectorcraftApp) -> bool {
    any_style(app, |s| {
        OtFeatures::from_tags(s.features.iter().map(String::as_str)).ligatures
            && vectorcraft_text::ligatures_suppressed_by(s.tracking)
            && !vectorcraft_text::explicit_ligatures(&s.features)
    })
}

/// The feature tag stylistic set `set` turns on.
fn set_tag(set: &StylisticSet) -> Option<&'static str> {
    EXTRA_FEATURES.get(usize::from(set.number).checked_sub(1)?).copied()
}

/// The Stylistic Sets entries of the panel's button and of the panel menu's submenu: each set the
/// font `st` names offers, the feature it turns on and off, and whether it is checked. The font,
/// not the text, decides which numbered sets can be offered. Each set is an independent feature,
/// so the menu permits several at once. A font's own label wins over the numbered fallback. See
/// the public Stylistic Sets help page. A set is checked when every character has it.
fn set_entries(app: &VectorcraftApp, st: &CharStyle) -> Vec<(StylisticSet, Flag, bool)> {
    offered_sets(st)
        .into_iter()
        .filter_map(|set| {
            let flag = Flag::Extra(set_tag(&set)?);
            let on = shown_flag(app, flag) == Some(true);
            Some((set, flag, on))
        })
        .collect()
}

/// The stylistic sets of the font `st` names. A font that is not available offers none, rather than
/// the sets of the face drawn in its place.
fn offered_sets(st: &vectorcraft_doc::CharStyle) -> Vec<StylisticSet> {
    let db = FontDb::global();
    let (family, _) = db.canonical(&st.font_family, &st.font_style);
    if !db.has_family(&family) {
        return Vec::new();
    }
    db.style_face(st).map(|face| face.stylistic_sets()).unwrap_or_default()
}

/// A Stylistic Sets menu entry: the font's own name for the set, shown as the font gives it, or
/// `Set 1` through `Set 20` in the UI language. The Glyphs panel's Show menu names its stylistic
/// set entries the same way.
pub(super) fn set_label(set: &StylisticSet) -> String {
    match &set.name {
        Some(name) => name.clone(),
        None => crate::i18n::fmt(tl!("Set {n}"), &[("n", &set.number.to_string())]),
    }
}

/// The panel's checkbox rows, in the order they are drawn.
fn rows() -> [Toggle; 9] {
    [
        (tl!("Standard Ligatures"), Flag::Field(|f| &mut f.ligatures)),
        (tl!("Contextual Alternates"), Flag::Field(|f| &mut f.contextual)),
        (tl!("Discretionary Ligatures"), Flag::Field(|f| &mut f.discretionary_ligatures)),
        (tl!("Swash"), Flag::Field(|f| &mut f.swash)),
        (tl!("Stylistic Alternates"), Flag::Extra("salt")),
        (tl!("Titling Alternates"), Flag::Extra("titl")),
        (tl!("Small Caps"), Flag::Field(|f| &mut f.small_caps)),
        (tl!("Ordinals"), Flag::Field(|f| &mut f.ordinals)),
        (tl!("Fractions"), Flag::Field(|f| &mut f.fractions)),
    ]
}

/// The panel menu's East Asian items, with Show East Asian Options: label and the feature each
/// turns on and off. H or V Style sets kana in the forms the font draws for the writing direction
/// (`hkna`, shaped as `vkna` in vertical type), and Roman Italics sets half-width alphanumerics
/// in the font's italic forms (`ital`).
fn east_asian_items() -> [Toggle; 2] {
    [(tl!("H or V Style"), Flag::Extra("hkna")), (tl!("Roman Italics"), Flag::Extra("ital"))]
}

/// The menu command that toggles the feature `tag`.
fn toggle_ref(tag: &str) -> CommandRef {
    CommandRef::with_params(TOGGLE, json!({ "tag": tag }))
}

/// The panel menu's rows: Reset, the Stylistic Sets submenu while the font offers sets, and with
/// Show East Asian Options Proportional Metrics and the two East Asian items. `PanelRow::Submenu`
/// has no disabled state, so the submenu is absent while the font offers no sets.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = vec![PanelRow::Command(CommandRef::new(RESET))];
    let sets: Vec<PanelRow> = text_style(app)
        .map(|(st, _)| offered_sets(&st))
        .unwrap_or_default()
        .iter()
        .filter_map(set_tag)
        .map(|tag| PanelRow::Command(toggle_ref(tag)))
        .collect();
    if !sets.is_empty() {
        rows.push(PanelRow::Submenu(tl!("Stylistic Sets").into(), sets));
    }
    if app.session.prefs.show_east_asian_options {
        rows.push(PanelRow::Separator);
        rows.push(PanelRow::Command(CommandRef::new(METRICS)));
        rows.extend(east_asian_items().into_iter().filter_map(|(_, flag)| flag.tag()).map(|tag| PanelRow::Command(toggle_ref(tag))));
    }
    rows
}

/// The titles, checks and enabled states of the rows [`menu_rows`] lists. A set or item is checked
/// when every character it changes has it; chosen then, it turns off.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let style = text_style(app);
    let has = style.is_some();
    states.set(CommandRef::new(RESET), CommandState::new().enabled(has));
    let sets = style.map(|(st, _)| set_entries(app, &st)).unwrap_or_default();
    for (set, flag, on) in &sets {
        if let Some(tag) = flag.tag() {
            states.set(toggle_ref(tag), CommandState::new().title(set_label(set)).checked(*on).enabled(true));
        }
    }
    if app.session.prefs.show_east_asian_options {
        // Proportional Metrics: full-width glyphs on the font's proportional widths (`palt`, `vpal`),
        // the character attribute the Character panel menu's item sets.
        let on = shown(app, |s| s.proportional_metrics) == Some(true);
        states.set(CommandRef::new(METRICS), CommandState::new().title(tl!("Proportional Metrics")).checked(on).enabled(has));
        for (label, flag) in east_asian_items() {
            if let Some(tag) = flag.tag() {
                let on = shown_flag(app, flag) == Some(true);
                states.set(toggle_ref(tag), CommandState::new().title(label).checked(on).enabled(has));
            }
        }
    }
}

/// The panel menu's commands: `ui.openType.reset`, `ui.openType.toggle {tag}` and
/// `ui.openType.proportionalMetrics`. A set or item chosen while only some characters have it
/// turns it on for all of them.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        RESET => {
            set(app, OtFeatures::default());
            Ok(json!({}))
        }
        TOGGLE => toggle(app, p),
        METRICS => proportional_metrics(app),
        _ => return None,
    })
}

/// `ui.openType.proportionalMetrics`: turn the `proportionalMetrics` character attribute off when
/// every character the item changes has it, else on, through the Character panel's format route
/// ([`super::character::run_format`]). With no type it changes nothing.
fn proportional_metrics(app: &mut VectorcraftApp) -> Result<Value, String> {
    // The typing Real-time Drawing off left unapplied is applied before the text is read.
    super::character::text_editing_kept(app)?;
    if text_style(app).is_none() {
        return Err(format!("{METRICS} is not available now"));
    }
    let on = shown(app, |s| s.proportional_metrics) == Some(true);
    super::character::run_format(app, json!({ "proportionalMetrics": !on }))?;
    Ok(json!({ "on": !on }))
}

/// `ui.openType.toggle {tag}`: turn the feature off when every character has it, else on.
fn toggle(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let tag = p.get("tag").and_then(Value::as_str).ok_or("`tag` must be an OpenType feature tag")?;
    let tag = EXTRA_FEATURES.iter().copied().find(|t| *t == tag).ok_or_else(|| format!("`{tag}` is not an OpenType feature the panel sets"))?;
    let flag = Flag::Extra(tag);
    // The typing Real-time Drawing off left unapplied is applied before the text is read.
    super::character::text_editing_kept(app)?;
    let on = shown_flag(app, flag) == Some(true);
    edit(app, Change::Toggle(flag, !on));
    Ok(json!({ "tag": tag, "on": !on }))
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::NodeId;

    use super::*;
    use crate::tests::panel_harness::Panel;

    fn selected_text() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Ag 12"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        app
    }

    fn features(app: &VectorcraftApp) -> OtFeatures {
        OtFeatures::from_tags(text_style(app).unwrap().0.features.iter().map(String::as_str))
    }

    /// Every row of the panel, and every Figure and Position entry, changes the selected text's
    /// features as the engine stores them: a checkbox row flips and flips back, the alternates
    /// rows leave exactly their tag, a Figure entry shows as itself, and a Position entry turns
    /// one position on and the other three off while the rest stays.
    #[test]
    fn a_panel_row_sets_its_feature_on_the_selected_text() {
        for (label, flag) in rows() {
            let mut app = selected_text();
            let was = flag.get(&features(&app));
            edit(&mut app, Change::Toggle(flag, !was));
            assert_eq!(flag.get(&features(&app)), !was, "{label} flips");
            edit(&mut app, Change::Toggle(flag, was));
            assert_eq!(flag.get(&features(&app)), was, "{label} flips back");
        }
        for (label, tag) in [("Stylistic Alternates", "salt"), ("Titling Alternates", "titl")] {
            let mut app = selected_text();
            let Some((_, flag)) = rows().into_iter().find(|(l, _)| *l == label) else {
                panic!("no {label} row");
            };
            edit(&mut app, Change::Toggle(flag, true));
            assert_eq!(text_style(&app).unwrap().0.features, [tag], "{label} on");
            edit(&mut app, Change::Toggle(flag, false));
            assert!(text_style(&app).unwrap().0.features.is_empty(), "{label} off");
        }
        let mut app = selected_text();
        for (i, name) in FIGURES.iter().enumerate() {
            edit(&mut app, Change::Figure(i));
            assert_eq!(figure(&features(&app)), i, "{name}");
            assert_eq!(shown_figure(&app), Some(i));
        }
        let mut app = selected_text();
        assert_eq!(shown_position(&app), Some(0));
        set(&mut app, OtFeatures::from_tags(["sups", "subs", "numr", "dnom", "smcp"]));
        for (i, tag) in [(1, "sups"), (2, "subs"), (3, "numr"), (4, "dnom")] {
            edit(&mut app, Change::Position(i));
            assert_eq!(text_style(&app).unwrap().0.features, ["smcp", tag], "{}", POSITIONS[i]);
            assert_eq!(shown_position(&app), Some(i));
        }
        edit(&mut app, Change::Position(0));
        assert_eq!(text_style(&app).unwrap().0.features, ["smcp"]);
        assert_eq!(shown_position(&app), Some(0));
    }

    /// Stylistic sets are independent: selecting a second set keeps the first, and clearing one
    /// keeps the other. The command reaches the selected text's stored OpenType feature tags.
    #[test]
    fn multiple_stylistic_sets_stay_independent() {
        let mut app = selected_text();
        edit(&mut app, Change::Toggle(Flag::Extra("ss01"), true));
        edit(&mut app, Change::Toggle(Flag::Extra("ss03"), true));
        assert_eq!(text_style(&app).unwrap().0.features, ["ss01", "ss03"]);
        edit(&mut app, Change::Toggle(Flag::Extra("ss01"), false));
        assert_eq!(text_style(&app).unwrap().0.features, ["ss03"]);
    }

    /// A font that is not available offers no sets, even though the face drawn in its place has
    /// some.
    #[test]
    fn a_missing_font_offers_no_stylistic_sets() {
        let fallback = vectorcraft_doc::CharStyle { font_family: vectorcraft_text::FALLBACK_FAMILY.into(), ..Default::default() };
        // Builds without the bundled fonts have no fallback face to draw in a missing font's place.
        if FontDb::global().has_family(vectorcraft_text::FALLBACK_FAMILY) {
            assert!(!offered_sets(&fallback).is_empty(), "the fallback face lists its sets");
        }
        let missing = vectorcraft_doc::CharStyle { font_family: "No Such Family Zq".into(), ..fallback };
        assert!(offered_sets(&missing).is_empty());
    }

    /// The Figure menu lists its five entries in order; Proportional Lining sets `pnum` and
    /// `lnum` and clears `tnum` and `onum`, Tabular Lining clears `pnum`, Proportional Oldstyle
    /// clears `lnum`, and Default Figure and Tabular Oldstyle clear `pnum` and `lnum`.
    #[test]
    fn proportional_lining_figures() {
        assert_eq!(FIGURES, ["Default Figure", "Tabular Lining", "Proportional Lining", "Proportional Oldstyle", "Tabular Oldstyle"]);
        let mut app = selected_text();
        let mut f = features(&app);
        set_figure(&mut f, 4);
        set(&mut app, f);
        assert_eq!(text_style(&app).unwrap().0.features, ["onum", "tnum"]);
        let mut f = features(&app);
        set_figure(&mut f, 2);
        set(&mut app, f);
        assert_eq!(text_style(&app).unwrap().0.features, ["pnum", "lnum"]);
        assert_eq!(figure(&features(&app)), 2);
        for (i, want) in [(0, vec![]), (1, vec!["tnum", "lnum"]), (3, vec!["onum", "pnum"]), (4, vec!["onum", "tnum"])] {
            let mut f = features(&app);
            set_figure(&mut f, 2);
            set_figure(&mut f, i);
            set(&mut app, f);
            assert_eq!(text_style(&app).unwrap().0.features, want, "{}", FIGURES[i]);
            assert_eq!(figure(&features(&app)), i);
        }
        // `pnum` alone is not Proportional Lining.
        assert_eq!(figure(&OtFeatures::from_tags(["pnum"])), 0);
    }

    /// Choosing Proportional Oldstyle on type with `pnum` and `onum` keeps `pnum`.
    #[test]
    fn proportional_oldstyle_keeps_pnum() {
        let mut app = selected_text();
        set(&mut app, OtFeatures::from_tags(["pnum", "onum"]));
        let mut f = features(&app);
        assert_eq!(figure(&f), 3);
        set_figure(&mut f, 3);
        set(&mut app, f);
        let tags = text_style(&app).unwrap().0.features;
        assert!(tags.iter().any(|t| t == "pnum"), "{tags:?}");
        assert!(tags.iter().any(|t| t == "onum"), "{tags:?}");
        assert_eq!(figure(&features(&app)), 3);
    }

    /// Choosing Tabular Lining on type with `lnum`, or with `tnum` and `lnum`, keeps `lnum`.
    #[test]
    fn tabular_lining_keeps_lnum() {
        for start in [&["lnum"][..], &["tnum", "lnum"][..]] {
            let mut app = selected_text();
            set(&mut app, OtFeatures::from_tags(start.iter().copied()));
            let mut f = features(&app);
            set_figure(&mut f, 1);
            set(&mut app, f);
            assert_eq!(text_style(&app).unwrap().0.features, ["tnum", "lnum"], "{start:?}");
            assert_eq!(figure(&features(&app)), 1);
        }
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// Selected objects whose positions differ show the Position menu as mixed; an entry chosen
    /// then applies to every one, the one the panel reads its features from included, in one undo
    /// step, and each keeps its other features.
    #[test]
    fn mixed_positions_show_mixed_and_take_the_choice() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let a = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "x2"})).unwrap()["id"].clone();
        let b = app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "y2"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        set(&mut app, OtFeatures::from_tags(["sups"]));
        app.session.execute("select.set", &json!({"ids": [b]})).unwrap();
        set(&mut app, OtFeatures::from_tags(["smcp"]));
        app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        assert_eq!(shown_position(&app), None);
        assert_eq!(position(&features(&app)), Some(1), "the panel reads the first object");
        let undo = undo_steps(&app);
        edit(&mut app, Change::Position(1));
        assert_eq!(undo_steps(&app), undo + 1);
        assert_eq!(shown_position(&app), Some(1));
        for (id, want) in [(&a, vec!["sups"]), (&b, vec!["smcp", "sups"])] {
            app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
            assert_eq!(text_style(&app).unwrap().0.features, want);
        }
    }

    /// The features of every run of `id`, each as its tags.
    fn frame_features(app: &VectorcraftApp, id: u64) -> Vec<Vec<String>> {
        match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.runs.iter().map(|r| r.style.features.clone()).collect(),
            _ => panic!("not text"),
        }
    }

    /// With the Selection tool a selected frame of threaded text stands for its whole story: the
    /// Figure and Position menus show Mixed when a locked or hidden frame differs, and a choice
    /// reaches every frame in one undo step, each keeping its other features, while other type keeps
    /// its own. A story selected through two frames and a group is changed once.
    #[test]
    fn selection_tool_features_reach_every_frame_of_the_story() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        let story = "The quick brown fox jumps over the lazy dog 1234.\n".repeat(12);
        let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        let c = app.run("text.thread.add", json!({"from": b, "rect": [400, 10, 150, 200]})).unwrap()["id"].as_u64().unwrap();
        let other = app.run("text.create", json!({"x": 10, "y": 250, "text": "Other 12", "size": 12})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [b]})).unwrap();
        app.run("object.lock", json!({})).unwrap();
        app.run("select.set", json!({"ids": [c]})).unwrap();
        app.run("object.hide", json!({})).unwrap();
        app.run("text.setStyle", json!({"ids": [c], "features": ["onum", "smcp", "sups", "-liga"]})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        for id in [a, b, c] {
            assert!(!frame_features(&app, id).is_empty(), "frame {id} holds text");
        }
        assert_eq!(shown_figure(&app), None, "the hidden frame's figures differ");
        assert_eq!(shown_position(&app), None, "the hidden frame's position differs");
        let hidden = frame_features(&app, c);
        let has = |app: &VectorcraftApp, id: u64, tag: &str| frame_features(app, id).iter().all(|f| f.iter().any(|t| t == tag));

        let undo = undo_steps(&app);
        edit(&mut app, Change::Figure(1));
        assert_eq!(undo_steps(&app), undo + 1, "one undo step");
        assert_eq!(shown_figure(&app), Some(1));
        for id in [a, b] {
            assert!(frame_features(&app, id).iter().all(|f| *f == ["tnum"]), "frame {id}: {:?}", frame_features(&app, id));
        }
        assert!(
            ["smcp", "sups", "-liga", "tnum"].iter().all(|t| has(&app, c, t))
                && frame_features(&app, c).iter().all(|f| !f.iter().any(|t| t == "onum")),
            "{:?}",
            frame_features(&app, c)
        );
        assert!(frame_features(&app, other).iter().all(Vec::is_empty), "other type keeps its features");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(frame_features(&app, c), hidden, "undo restores the hidden frame");

        // Standard Ligatures is off in the hidden frame only: its row shows a dash, and turning it
        // on reaches every frame.
        let liga = rows().into_iter().find(|(l, _)| *l == "Standard Ligatures").map(|(_, f)| f);
        let Some(liga) = liga else { panic!("no Standard Ligatures row") };
        assert_eq!(shown_flag(&app, liga), None);
        let undo = undo_steps(&app);
        edit(&mut app, Change::Toggle(liga, true));
        assert_eq!(undo_steps(&app), undo + 1, "one undo step");
        assert_eq!(shown_flag(&app, liga), Some(true));
        for id in [a, b, c] {
            assert!(frame_features(&app, id).iter().all(|f| !f.iter().any(|t| t == "-liga")), "frame {id} has ligatures");
        }
        assert!(has(&app, c, "smcp"), "the hidden frame keeps its other features");

        // Two frames of the story and the other type in a group: each story once.
        app.run("object.unlockAll", json!({})).unwrap();
        app.run("select.set", json!({"ids": [a, other]})).unwrap();
        let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [group, b]})).unwrap();
        edit(&mut app, Change::Position(1));
        for id in [a, b, c, other] {
            assert!(frame_features(&app, id).iter().all(|f| f.iter().filter(|t| *t == "sups").count() == 1), "frame {id} has sups once");
        }
    }

    /// The runs of `id`'s characters `0..end`, each as its text and its features.
    fn run_features(app: &mut VectorcraftApp, id: &Value, end: usize) -> Vec<(String, Value)> {
        let runs = app.session.execute("text.getRange", &json!({"id": id, "start": 0, "end": end})).unwrap()["runs"].clone();
        runs.as_array().unwrap().iter().map(|r| (r["text"].as_str().unwrap_or("").to_string(), r["style"]["features"].clone())).collect()
    }

    /// With the Selection tool on the first frame of a threaded story, the tracked-ligatures note
    /// reads every frame: it shows when a later, locked frame is tracked with Standard Ligatures
    /// on, and not once that frame's ligatures are explicit, off, or its tracking is back within
    /// the limit.
    #[test]
    fn ligature_note_reads_every_frame_of_the_story() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 900, "height": 300})).unwrap();
        let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame. ".repeat(6);
        let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
        let a = app.session.execute("text.create", &area).unwrap()["id"].clone();
        let b = app.session.execute("text.thread.add", &json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].clone();
        assert!(!run_features(&mut app, &b, 4).is_empty(), "the second frame holds text");
        app.session.execute("text.setStyle", &json!({"ids": [b], "tracking": 100})).unwrap();
        app.session.execute("select.set", &json!({"ids": [b]})).unwrap();
        app.session.execute("object.lock", &json!({})).unwrap();
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        assert_eq!(text_editing(&app), None);
        assert_eq!(text_style(&app).unwrap().0.tracking, 0.0, "the panel reads its first run from the first frame");
        assert!(tracking_drops_ligatures(&app), "the tracked second frame shows the note");
        // Commands write `liga` as the default and leave it out; an opened file can keep it.
        let b_id = NodeId(b.as_u64().unwrap());
        for tags in [&["liga"][..], &["clig"][..], &["-liga"][..]] {
            let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
            let Some(NodeKind::Text(t)) = doc.node_mut(b_id).map(|n| &mut n.kind) else { panic!("no second frame") };
            for r in &mut t.runs {
                r.style.features = tags.iter().map(|s| s.to_string()).collect();
            }
            assert!(!tracking_drops_ligatures(&app), "{tags:?}");
        }
        app.session.execute("text.setStyle", &json!({"ids": [b], "features": [], "tracking": 20})).unwrap();
        assert!(!tracking_drops_ligatures(&app), "tracking within the limit");
        app.session.execute("text.setStyle", &json!({"ids": [b], "tracking": -100})).unwrap();
        assert!(tracking_drops_ligatures(&app), "tight tracking shows the note too");
    }

    /// At a caret whose insertion style is already a denominator, Denominator still reaches the
    /// characters the command styles at a caret (all of them) in one undo step; chosen again once
    /// every character has it, it adds no undo step.
    #[test]
    fn caret_with_the_same_position_still_applies_it() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "1/23"})).unwrap()["id"].clone();
        app.session.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 1, "features": ["dnom"]})).unwrap();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
        assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((1, 1)));
        assert_eq!(shown_position(&app), Some(4));
        let undo = undo_steps(&app);
        edit(&mut app, Change::Position(4));
        assert_eq!(undo_steps(&app), undo + 1);
        assert_eq!(run_features(&mut app, &id, 4), [("1/23".to_string(), json!(["dnom"]))]);
        edit(&mut app, Change::Position(4));
        assert_eq!(undo_steps(&app), undo + 1);
    }

    /// The state the panel menu gives the command `r`.
    fn state_of(app: &VectorcraftApp, r: &CommandRef) -> Option<CommandState> {
        let mut states = CommandStates::new();
        menu_states(app, &mut states);
        states.get(r).cloned()
    }

    /// Whether the rows, or a submenu's, list the command `r`.
    fn has_row(rows: &[PanelRow], r: &CommandRef) -> bool {
        rows.iter().any(|row| match row {
            PanelRow::Command(c) => c == r,
            PanelRow::Submenu(_, rows) => has_row(rows, r),
            _ => false,
        })
    }

    /// The state of a toggle row titled `label`, checked or not.
    fn toggled(label: &str, on: bool) -> CommandState {
        CommandState::new().title(label).checked(on).enabled(true)
    }

    /// The panel menu's Stylistic Sets submenu lists the font's sets with the labels of the panel's
    /// Stylistic Sets button. Run, a set turns on for the selected type in one undo step and
    /// shows checked, keeping the other features; run again, it turns off. With no text selected
    /// the submenu is left out and Reset is disabled.
    #[test]
    fn the_panel_menu_stylistic_sets_submenu_turns_sets_on_and_off() {
        let mut app = selected_text();
        let Some((st, _)) = text_style(&app) else { panic!("no text style") };
        let sets = set_entries(&app, &st);
        // Builds without the bundled fonts have no face that offers sets.
        let Some((first, flag, _)) = sets.first() else { return };
        let (label, tag) = (set_label(first), flag.tag().unwrap());
        set(&mut app, OtFeatures { small_caps: true, ..Default::default() });
        let r = toggle_ref(tag);
        assert!(menu_rows(&app).iter().any(|row| matches!(row, PanelRow::Submenu(_, sub) if has_row(sub, &r))));
        assert_eq!(state_of(&app, &r), Some(toggled(&label, false)));
        let undo = undo_steps(&app);
        assert_eq!(run(&mut app, TOGGLE, &json!({"tag": tag})), Some(Ok(json!({"tag": tag, "on": true}))));
        assert_eq!(undo_steps(&app), undo + 1);
        let on = text_style(&app).unwrap().0.features;
        assert!(on.iter().any(|f| f == tag) && on.iter().any(|f| f == "smcp"), "{on:?}");
        assert_eq!(state_of(&app, &r), Some(toggled(&label, true)));
        run(&mut app, TOGGLE, &json!({"tag": tag})).unwrap().unwrap();
        assert_eq!(text_style(&app).unwrap().0.features, ["smcp"]);
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        assert!(!menu_rows(&app).iter().any(|row| matches!(row, PanelRow::Submenu(..))), "the submenu is left out with no text selected");
        assert_eq!(state_of(&app, &CommandRef::new(RESET)), Some(CommandState::new().enabled(false)));
        assert_eq!(run(&mut app, "ui.openType.nothing", &json!({})), None);
    }

    /// H or V Style and Roman Italics are in the panel menu with the East Asian options only.
    #[test]
    fn kana_style_and_roman_italics_show_with_the_east_asian_options() {
        let mut app = selected_text();
        let shown = |app: &VectorcraftApp| ["hkna", "ital"].map(|tag| has_row(&menu_rows(app), &toggle_ref(tag)));
        assert_eq!(shown(&app), [false, false]);
        app.session.prefs.show_east_asian_options = true;
        assert_eq!(shown(&app), [true, true]);
    }

    /// With the Type tool's characters only partly set, H or V Style and Roman Italics show
    /// unchecked; run, each turns its feature on for every selected character in one undo step,
    /// each run keeping its other features, and shows checked; run again, it turns it off.
    /// Reset OpenType Features clears both.
    #[test]
    fn kana_style_and_roman_italics_turn_on_and_off() {
        for (label, tag) in [("H or V Style", "hkna"), ("Roman Italics", "ital")] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.session.prefs.show_east_asian_options = true;
            app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
            app.session.select_tool("type", Default::default()).unwrap();
            let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "かなAB"})).unwrap()["id"].clone();
            app.session.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 3, "features": [tag, "smcp"]})).unwrap();
            app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
            // "かなAB" is 8 bytes.
            app.session.set_tool_option("select", &json!({"start": 0, "end": 8}));
            let r = toggle_ref(tag);
            let has = |runs: &[(String, Value)], t: &str| runs.iter().all(|(_, f)| f.as_array().is_some_and(|f| f.iter().any(|x| x == t)));
            assert_eq!(state_of(&app, &r), Some(toggled(label, false)), "{label}");
            let undo = undo_steps(&app);
            run(&mut app, TOGGLE, &json!({"tag": tag})).unwrap().unwrap();
            assert_eq!(undo_steps(&app), undo + 1, "{label}");
            let runs = run_features(&mut app, &id, 8);
            assert!(has(&runs, tag), "{label}: {runs:?}");
            assert!(has(&runs[..1], "smcp") && runs[0].0 == "か", "{label} keeps the other features: {runs:?}");
            assert_eq!(state_of(&app, &r), Some(toggled(label, true)), "{label}");
            run(&mut app, TOGGLE, &json!({"tag": tag})).unwrap().unwrap();
            assert_eq!(run_features(&mut app, &id, 8), [("か".to_string(), json!(["smcp"])), ("なAB".to_string(), Value::Null)], "{label} off");
            app.session.execute("text.setRangeStyle", &json!({"id": id, "features": ["hkna", "ital", "dlig"]})).unwrap();
            run(&mut app, RESET, &json!({})).unwrap().unwrap();
            assert_eq!(run_features(&mut app, &id, 8), [("かなAB".to_string(), Value::Null)], "{label}: reset");
        }
    }

    /// At a Type tool caret after a character that has H or V Style or Roman Italics while the
    /// rest of the text does not, the item and the checkbox rows read every character the command
    /// styles at a caret (all of them): the item shows unchecked, a Small Caps row set the same way
    /// shows a dash, and running the item turns its feature on for the whole text.
    #[test]
    fn caret_items_read_every_character_they_change() {
        for (label, tag) in [("H or V Style", "hkna"), ("Roman Italics", "ital")] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.session.prefs.show_east_asian_options = true;
            app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
            app.session.select_tool("type", Default::default()).unwrap();
            let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "かなAB"})).unwrap()["id"].clone();
            app.session.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 3, "features": [tag, "smcp"]})).unwrap();
            app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
            // The caret after "か", whose style the next typed character takes.
            app.session.set_tool_option("select", &json!({"start": 3, "end": 3}));
            assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((3, 3)));
            assert_eq!(shown_flag(&app, Flag::Field(|f| &mut f.small_caps)), None, "{label}: Small Caps row");
            assert_eq!(state_of(&app, &toggle_ref(tag)), Some(toggled(label, false)), "{label}");
            run(&mut app, TOGGLE, &json!({"tag": tag})).unwrap().unwrap();
            let runs = run_features(&mut app, &id, 8);
            assert!(runs.iter().all(|(_, f)| f.as_array().is_some_and(|f| f.iter().any(|x| x == tag))), "{label}: {runs:?}");
        }
    }

    /// After Select All in a threaded story whose edited frame is empty, the item reads the text
    /// of every frame the command styles: with the story's text set with H or V Style or Roman
    /// Italics it shows checked, and running it turns the feature off for that text.
    #[test]
    fn select_all_from_an_empty_frame_reads_the_whole_story() {
        for (label, tag) in [("H or V Style", "hkna"), ("Roman Italics", "ital")] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.session.prefs.show_east_asian_options = true;
            app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
            let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame. ".repeat(4);
            let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
            let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
            let b = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
            app.run("select.set", json!({"ids": [a, b]})).unwrap();
            app.run("text.thread.create", json!({})).unwrap();
            app.session.select_tool("type", Default::default()).unwrap();
            app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
            app.run("select.all", json!({})).unwrap();
            // A frame threaded in before the first takes the whole text, leaving the edited frame empty.
            let c = app.run("text.thread.add", json!({"from": a, "before": true, "rect": [700, 10, 400, 600]})).unwrap()["id"].as_u64().unwrap();
            app.run("text.setRangeStyle", json!({"id": c, "features": [tag]})).unwrap();
            assert_eq!(text_editing(&app).map(|(id, a, b)| (id.0, a, b)), Some((a, 0, 0)), "{label}");
            assert_eq!(super::super::character::text_story(&app).map(|frames| frames.len()), Some(3), "{label}");
            // The empty frames' runs, which hold no character, without the feature.
            for id in [a, b] {
                let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
                let Some(NodeKind::Text(t)) = doc.node_mut(NodeId(id)).map(|n| &mut n.kind) else { panic!("no frame {id}") };
                for r in &mut t.runs {
                    r.style.features.clear();
                }
            }
            assert_eq!(state_of(&app, &toggle_ref(tag)), Some(toggled(label, true)), "{label}");
            run(&mut app, TOGGLE, &json!({"tag": tag})).unwrap().unwrap();
            let runs = run_features(&mut app, &json!(c), story.len());
            assert!(runs.iter().all(|(_, f)| !f.as_array().is_some_and(|f| f.iter().any(|x| x == tag))), "{label}: {runs:?}");
        }
    }

    /// The text, Proportional Metrics and features of each run of `id`.
    fn metrics_runs(app: &VectorcraftApp, id: &Value) -> Vec<(String, bool, Vec<String>)> {
        let Some(id) = id.as_u64() else { panic!("no id: {id}") };
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.proportional_metrics, r.style.features.clone())).collect(),
            _ => panic!("not text"),
        }
    }

    /// Where the panel menu's top-level rows list the command `r`.
    fn row_index(rows: &[PanelRow], r: &CommandRef) -> Option<usize> {
        rows.iter().position(|row| matches!(row, PanelRow::Command(c) if c == r))
    }

    /// Proportional Metrics is in the panel menu with the East Asian options only, before H or V
    /// Style, and with no type selected it is disabled: run, it changes nothing.
    #[test]
    fn proportional_metrics_shows_with_the_east_asian_options() {
        let mut app = selected_text();
        let metrics = CommandRef::new(METRICS);
        assert_eq!(row_index(&menu_rows(&app), &metrics), None);
        app.session.prefs.show_east_asian_options = true;
        let rows = menu_rows(&app);
        let (Some(at), Some(kana)) = (row_index(&rows, &metrics), row_index(&rows, &toggle_ref("hkna"))) else { panic!("rows missing") };
        assert!(at < kana, "Proportional Metrics comes before H or V Style");
        assert_eq!(state_of(&app, &metrics), Some(toggled("Proportional Metrics", false)));
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        assert_eq!(state_of(&app, &metrics), Some(CommandState::new().title("Proportional Metrics").checked(false).enabled(false)));
        let undo = undo_steps(&app);
        assert!(matches!(run(&mut app, METRICS, &json!({})), Some(Err(_))));
        assert_eq!(undo_steps(&app), undo, "no text: nothing changes");
    }

    /// With the Type tool's characters only partly set, Proportional Metrics shows unchecked; run,
    /// it turns on for every selected character in one undo step, each run keeping its features,
    /// and shows checked; run again, it turns off in one undo step.
    #[test]
    fn proportional_metrics_turns_on_and_off_for_a_mixed_range() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.prefs.show_east_asian_options = true;
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "かなAB"})).unwrap()["id"].clone();
        let first = json!({"id": id, "start": 0, "end": 3, "features": ["smcp"], "proportionalMetrics": true});
        app.session.execute("text.setRangeStyle", &first).unwrap();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        // "かなAB" is 8 bytes.
        app.session.set_tool_option("select", &json!({"start": 0, "end": 8}));
        let metrics = CommandRef::new(METRICS);
        assert_eq!(state_of(&app, &metrics), Some(toggled("Proportional Metrics", false)));
        let undo = undo_steps(&app);
        assert_eq!(run(&mut app, METRICS, &json!({})), Some(Ok(json!({"on": true}))));
        assert_eq!(undo_steps(&app), undo + 1);
        let runs = metrics_runs(&app, &id);
        assert!(runs.iter().all(|(_, on, _)| *on), "{runs:?}");
        assert_eq!(runs.iter().map(|(t, _, f)| (t.as_str(), f.clone())).collect::<Vec<_>>(), [("か", vec!["smcp".to_string()]), ("なAB", vec![])]);
        assert_eq!(state_of(&app, &metrics), Some(toggled("Proportional Metrics", true)));
        assert_eq!(run(&mut app, METRICS, &json!({})), Some(Ok(json!({"on": false}))));
        assert_eq!(undo_steps(&app), undo + 2);
        let runs = metrics_runs(&app, &id);
        assert!(runs.iter().all(|(_, on, f)| !*on && (f.is_empty() || *f == ["smcp"])), "{runs:?}");
    }

    /// With the Selection tool, Proportional Metrics turns on for the whole selected object. At a
    /// Type tool caret after a character that has it while the rest of the text does not, the item
    /// reads every character the command styles at a caret (all of them): it shows unchecked, and
    /// running it turns it on for the whole text.
    #[test]
    fn proportional_metrics_reaches_a_selected_object_and_the_text_at_a_caret() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.prefs.show_east_asian_options = true;
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "かなAB"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        run(&mut app, METRICS, &json!({})).unwrap().unwrap();
        assert!(metrics_runs(&app, &id).iter().all(|(_, on, _)| *on), "{:?}", metrics_runs(&app, &id));

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.prefs.show_east_asian_options = true;
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "かなAB"})).unwrap()["id"].clone();
        app.session.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 3, "proportionalMetrics": true})).unwrap();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        // The caret after "か", whose style the next typed character takes.
        app.session.set_tool_option("select", &json!({"start": 3, "end": 3}));
        assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((3, 3)));
        assert_eq!(state_of(&app, &CommandRef::new(METRICS)), Some(toggled("Proportional Metrics", false)));
        run(&mut app, METRICS, &json!({})).unwrap().unwrap();
        let runs = metrics_runs(&app, &id);
        assert!(runs.iter().all(|(_, on, _)| *on), "{runs:?}");
    }

    /// With Real-time Drawing off, Proportional Metrics and H or V Style decide from the text with
    /// the typing applied: after a Backspace not yet applied takes out the one character without
    /// the setting, the rest has it, so running the item turns it off.
    #[test]
    fn east_asian_items_decide_from_the_text_with_the_typing_applied() {
        type Get = fn(&CharStyle) -> bool;
        let items: [(&str, &str, Value, Value, Get); 2] = [
            ("Proportional Metrics", METRICS, json!({}), json!({"proportionalMetrics": true}), |s| s.proportional_metrics),
            ("H or V Style", TOGGLE, json!({"tag": "hkna"}), json!({"features": ["hkna"]}), |s| s.features.iter().any(|f| f == "hkna")),
        ];
        for (label, cmd, params, set, get) in items {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.session.prefs.show_east_asian_options = true;
            app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
            app.session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
            let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "aか", "size": 12})).unwrap()["id"].clone();
            let mut range = set.clone();
            range["id"] = id.clone();
            range["start"] = json!(1);
            range["end"] = json!(4);
            app.run("text.setRangeStyle", range).unwrap();
            app.run("select.set", json!({"ids": [id.clone()]})).unwrap();
            app.select_tool("type");
            app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
            app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
            let mut app = crate::testing::windowed(app);
            let ctx = app.ui_ctx.clone().unwrap();
            let backspace =
                |pressed| egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
            // The keymap builds in the first pass.
            crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
            crate::tests::synthetic::key_frame(&mut app, &ctx, vec![backspace(true), backspace(false)]);
            assert_eq!(app.session.tool_options()["typing"], json!(true), "{label}");
            let node = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
                NodeKind::Text(t) => (t.plain_text(), t.runs.iter().map(|r| get(&r.style)).collect::<Vec<_>>()),
                _ => panic!("text"),
            };
            assert_eq!(node(&app).0, "aか", "{label}: the Backspace is not applied yet");
            run(&mut app, cmd, &params).unwrap().unwrap();
            assert_eq!(node(&app), ("か".to_string(), vec![false]), "{label}");
        }
    }

    /// With the Selection tool on a threaded story whose second frame is empty, the East Asian
    /// items read only the characters: with the text set and the empty frame's run not, each item
    /// shows checked, and running it turns the setting off for the text.
    #[test]
    fn east_asian_items_skip_an_empty_frame_of_a_selected_story() {
        type Set = fn(&mut CharStyle, bool);
        type Get = fn(&CharStyle) -> bool;
        let items: [(&str, CommandRef, &str, Value, Set, Get); 3] = [
            ("Proportional Metrics", CommandRef::new(METRICS), METRICS, json!({}), |s, on| s.proportional_metrics = on, |s| s.proportional_metrics),
            (
                "H or V Style",
                toggle_ref("hkna"),
                TOGGLE,
                json!({"tag": "hkna"}),
                |s, on| s.features = if on { vec!["hkna".into()] } else { vec![] },
                |s| s.features.iter().any(|f| f == "hkna"),
            ),
            (
                "Roman Italics",
                toggle_ref("ital"),
                TOGGLE,
                json!({"tag": "ital"}),
                |s, on| s.features = if on { vec!["ital".into()] } else { vec![] },
                |s| s.features.iter().any(|f| f == "ital"),
            ),
        ];
        for (label, item, cmd, params, set, get) in items {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.session.prefs.show_east_asian_options = true;
            app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
            let area = json!({"x": 10, "y": 10, "text": "かなAB", "size": 12, "area": {"width": 150, "height": 60}});
            let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
            let b = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
            app.run("select.set", json!({"ids": [a, b]})).unwrap();
            app.run("text.thread.create", json!({})).unwrap();
            app.run("select.set", json!({"ids": [a]})).unwrap();
            assert_eq!(super::super::character::selected_story_texts(&app).len(), 2, "{label}");
            for (id, on) in [(a, true), (b, false)] {
                let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
                let Some(NodeKind::Text(t)) = doc.node_mut(NodeId(id)).map(|n| &mut n.kind) else { panic!("no frame {id}") };
                for r in &mut t.runs {
                    set(&mut r.style, on);
                }
            }
            let empty = match &app.session.active().unwrap().doc.node(NodeId(b)).unwrap().kind {
                NodeKind::Text(t) => t.runs.iter().filter(|r| r.text.is_empty()).count(),
                _ => panic!("{label}: not text"),
            };
            assert!(empty > 0, "{label}: the empty frame has no run");
            assert_eq!(state_of(&app, &item), Some(toggled(label, true)), "{label}");
            run(&mut app, cmd, &params).unwrap().unwrap();
            match &app.session.active().unwrap().doc.node(NodeId(a)).unwrap().kind {
                NodeKind::Text(t) => assert!(t.runs.iter().all(|r| !get(&r.style)), "{label}"),
                _ => panic!("{label}: not text"),
            }
        }
    }

    fn row_flag(label: &str) -> Flag {
        rows().into_iter().find(|(l, _)| *l == label).unwrap().1
    }

    #[test]
    fn right_clicking_an_opentype_row_offers_turn_on_or_turn_off() {
        let mut panel = Panel::new(selected_text(), show);
        let ids = panel.app().session.active().unwrap().selection.objects.clone();
        let before = features(&panel.app()).to_tags();
        let undo = undo_steps(&panel.app());
        panel.secondary_click("Standard Ligatures");
        assert!(panel.menu_item_enabled("Turn Off"));
        assert!(!panel.shows("Turn On"));
        assert!(panel.menu_item_enabled("Reset OpenType Features"));
        assert_eq!(panel.app().session.active().unwrap().selection.objects, ids);
        assert_eq!(features(&panel.app()).to_tags(), before);
        assert_eq!(undo_steps(&panel.app()), undo);
        panel.click_menu_item("Turn Off");
        panel.secondary_click("Standard Ligatures");
        assert!(panel.menu_item_enabled("Turn On"));
        assert!(!panel.shows("Turn Off"));
        assert_eq!(panel.app().session.active().unwrap().selection.objects, ids);
    }

    #[test]
    fn clicking_turn_on_sets_that_feature_in_one_undo_step() {
        let off = OtFeatures { ligatures: false, swash: true, ..OtFeatures::default() };
        let mut app = selected_text();
        set(&mut app, off);
        let mut by_check = Panel::new(app, show);
        by_check.click("Standard Ligatures");
        let mut app = selected_text();
        set(&mut app, off);
        let before = features(&app).to_tags();
        let undo = undo_steps(&app);
        let mut panel = Panel::new(app, show);
        panel.secondary_click("Standard Ligatures");
        panel.click_menu_item("Turn On");
        assert_eq!(undo_steps(&panel.app()), undo + 1);
        assert_eq!(features(&panel.app()).to_tags(), features(&by_check.app()).to_tags());
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(features(&panel.app()).to_tags(), before);
    }

    #[test]
    fn turn_on_preserves_the_mixed_character_range_and_undo_restores_its_runs() {
        let mut app = selected_text();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.run("text.create", json!({"x": 0, "y": 20, "text": "abcd"})).unwrap()["id"].clone();
        app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "features": ["frac"]})).unwrap();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 1, "end": 3}));
        let range = Some((NodeId(id.as_u64().unwrap()), 1, 3));
        assert_eq!(text_editing(&app), range);
        assert_eq!(shown_flag(&app, row_flag("Fractions")), None);
        let before = run_features(&mut app, &id, 4);
        let undo = undo_steps(&app);
        let mut panel = Panel::new(app, show);
        panel.secondary_click("Fractions");
        assert_eq!(text_editing(&panel.app()), range);
        assert_eq!(run_features(&mut panel.app(), &id, 4), before);
        panel.click_menu_item("Turn On");
        assert_eq!(run_features(&mut panel.app(), &id, 4), [("a".into(), Value::Null), ("bc".into(), json!(["frac"])), ("d".into(), Value::Null)]);
        assert_eq!(undo_steps(&panel.app()), undo + 1);
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(run_features(&mut panel.app(), &id, 4), before);
    }

    #[test]
    fn clicking_reset_opentype_features_clears_the_rows() {
        let custom = OtFeatures { ligatures: false, fractions: true, swash: true, ..OtFeatures::default() };
        let mut by_flyout = selected_text();
        set(&mut by_flyout, custom);
        run(&mut by_flyout, RESET, &json!({})).unwrap().unwrap();
        let mut app = selected_text();
        set(&mut app, custom);
        let before = features(&app).to_tags();
        let undo = undo_steps(&app);
        let mut panel = Panel::new(app, show);
        panel.secondary_click("Fractions");
        panel.click_menu_item("Reset OpenType Features");
        assert_eq!(undo_steps(&panel.app()), undo + 1);
        assert_eq!(features(&panel.app()).to_tags(), features(&by_flyout).to_tags());
        assert_eq!(shown_flag(&panel.app(), row_flag("Standard Ligatures")), Some(true));
        assert_eq!(shown_flag(&panel.app(), row_flag("Fractions")), Some(false));
        assert_eq!(shown_flag(&panel.app(), row_flag("Swash")), Some(false));
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(features(&panel.app()).to_tags(), before);
    }
}
