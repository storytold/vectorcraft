//! OpenType panel: per-character OpenType features (ligatures, alternates, swashes, small caps,
//! ordinals, fractions, figure style, position) for the selected text or the Type tool's selected
//! characters.

use egui::Ui;
use serde_json::json;
use vectorcraft_text::{EXTRA_FEATURES, OtFeatures};

use super::character::{shared, text_editing, text_style};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

/// The feature a checkbox row toggles: a named flag, or a tag of [`EXTRA_FEATURES`].
#[derive(Clone, Copy)]
enum Flag {
    Field(fn(&mut OtFeatures) -> &mut bool),
    Extra(&'static str),
}

/// A checkbox row: label and the feature it toggles.
type Toggle = (&'static str, Flag);

const FIGURES: [&str; 5] = ["Default Figure", "Tabular Lining", "Proportional Lining", "Proportional Oldstyle", "Tabular Oldstyle"];

const POSITIONS: [&str; 5] = ["Default Position", "Superscript", "Subscript", "Numerator", "Denominator"];

/// The features the Position menu's entries after Default Position turn on, in menu order.
const POSITION_TAGS: [&str; 4] = ["sups", "subs", "numr", "dnom"];

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
            Flag::Extra(tag) if on => f.extra |= extra_bit(tag),
            Flag::Extra(tag) => f.extra &= !extra_bit(tag),
        }
    }
}

/// The Figure menu entry `f` shows: Default Figure, Tabular Lining, Proportional Lining
/// (`pnum` and `lnum`), Proportional Oldstyle or Tabular Oldstyle.
fn figure(f: &OtFeatures) -> usize {
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
    f.extra &= !match i {
        1 => extra_bit("pnum"),
        3 => extra_bit("lnum"),
        _ => extra_bit("pnum") | extra_bit("lnum"),
    };
    if i == 2 {
        f.extra |= extra_bit("pnum") | extra_bit("lnum");
    }
}

/// The Position menu entry `f` shows: Superscript (`sups`), Subscript (`subs`), Numerator
/// (`numr`) or Denominator (`dnom`) when `f` turns that one on, Default Position when it turns
/// none of them on, and `None` when it turns on more than one.
fn position(f: &OtFeatures) -> Option<usize> {
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
        if k + 1 == i {
            f.extra |= extra_bit(tag);
        } else {
            f.extra &= !extra_bit(tag);
        }
    }
}

/// The Position menu entry for the characters the panel acts on, or `None` when they differ or
/// one of them turns on more than one position.
fn shown_position(app: &VectorcraftApp) -> Option<usize> {
    shared(app, |s| position(&OtFeatures::from_tags(s.features.iter().map(String::as_str)))).flatten()
}

/// The Position menu's text for entry `shown`: Mixed when the characters differ.
fn position_label(shown: Option<usize>) -> &'static str {
    shown.and_then(|i| POSITIONS.get(i).copied()).unwrap_or("Mixed")
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
}

impl Change {
    fn apply(self, f: &mut OtFeatures) {
        match self {
            Change::Figure(i) => set_figure(f, i),
            Change::Position(i) => set_position(f, i),
            Change::Toggle(flag, on) => flag.put(f, on),
        }
    }
}

/// Make `change` to the features of each character the panel acts on, from the character's own
/// features (`featureChanges`), unless it changes none of them.
fn edit(app: &mut VectorcraftApp, change: Change) {
    let unchanged = shared(app, |s| {
        let before = OtFeatures::from_tags(s.features.iter().map(String::as_str));
        let mut after = before;
        change.apply(&mut after);
        after == before
    });
    if unchanged != Some(true) {
        run_features(app, "featureChanges", OtFeatures::changes(|f| change.apply(f)));
    }
}

/// Apply `f` to the selection's features (the Type tool range, or whole objects).
fn set(app: &mut VectorcraftApp, f: OtFeatures) {
    run_features(app, "features", f.to_tags());
}

/// Send `tags` as the `key` param (`features` or `featureChanges`) for the Type tool's range or
/// caret, or else for the selected type.
fn run_features(app: &mut VectorcraftApp, key: &str, tags: Vec<String>) {
    let (cmd, mut p) = match text_editing(app) {
        Some((id, a, b)) if b > a => ("text.setRangeStyle", json!({ "id": id.0, "start": a, "end": b })),
        Some((id, _, _)) => ("text.setRangeStyle", json!({ "id": id.0 })),
        None => ("text.setStyle", json!({})),
    };
    p[key] = json!(tags);
    if let Err(e) = app.run(cmd, p) {
        app.ui.status = e;
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((st, _)) = text_style(app) else {
        widgets::dim_label(ui, tl!("Select text to set its OpenType features."));
        return;
    };
    let cur = OtFeatures::from_tags(st.features.iter().map(String::as_str));
    let mut change = None;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Figure:"));
        let fig = figure(&cur);
        if let Some(i) = widgets::dropdown(ui, "ot-figure", FIGURES[fig], &FIGURES, 170.0) {
            change = Some(Change::Figure(i));
        }
    });
    let shown = shown_position(app);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Position:"));
        if let Some(i) = widgets::dropdown(ui, "ot-position", position_label(shown), &POSITIONS, 170.0) {
            change = Some(Change::Position(i));
        }
    });
    widgets::divider(ui);
    let mut next = cur;
    if let Some(c) = change {
        c.apply(&mut next);
    }
    for (label, flag) in rows() {
        let on = flag.get(&next);
        if widgets::check(ui, label, on, true) {
            change = Some(Change::Toggle(flag, !on));
        }
    }
    if cur.ligatures && vectorcraft_text::ligatures_suppressed_by(st.tracking) && !vectorcraft_text::explicit_ligatures(&st.features) {
        widgets::dim_label(ui, tl!("Ligatures are off while the text is tracked."));
    }
    if let Some(c) = change {
        edit(app, c);
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

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if menu_item(ui, tl!("Reset OpenType Features"), text_style(app).is_some(), false) {
        set(app, OtFeatures::default());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn toggles_reach_the_text() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "fi 1/2"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui);
            menu(&mut app, ui);
        });
        out.textures_delta.clear();
        set(&mut app, OtFeatures { ligatures: false, fractions: true, ..Default::default() });
        assert_eq!(text_style(&app).unwrap().0.features, ["-liga", "frac"]);
    }

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

    /// The Stylistic Alternates and Titling Alternates rows turn `salt` and `titl` on and off
    /// on the selected type.
    #[test]
    fn alternates_rows_set_and_clear_their_features() {
        for (label, tag) in [("Stylistic Alternates", "salt"), ("Titling Alternates", "titl")] {
            let mut app = selected_text();
            let Some((_, flag)) = rows().into_iter().find(|(l, _)| *l == label) else {
                panic!("no {label} row");
            };
            let mut f = features(&app);
            assert!(!flag.get(&f), "{label} starts off");
            flag.put(&mut f, true);
            set(&mut app, f);
            assert_eq!(text_style(&app).unwrap().0.features, [tag], "{label} on");
            assert!(flag.get(&features(&app)));
            let mut f = features(&app);
            flag.put(&mut f, false);
            set(&mut app, f);
            assert!(text_style(&app).unwrap().0.features.is_empty(), "{label} off");
        }
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

    /// The Position menu lists its five entries in order; Superscript, Subscript, Numerator and
    /// Denominator each set their feature and clear the other three, and Default Position clears
    /// all four, on the selected type.
    #[test]
    fn position_menu_sets_one_position() {
        assert_eq!(POSITIONS, ["Default Position", "Superscript", "Subscript", "Numerator", "Denominator"]);
        let mut app = selected_text();
        assert_eq!(shown_position(&app), Some(0));
        set(&mut app, OtFeatures::from_tags(["sups", "subs", "numr", "dnom", "smcp"]));
        for (i, tag) in [(1, "sups"), (2, "subs"), (3, "numr"), (4, "dnom")] {
            edit(&mut app, Change::Position(i));
            assert_eq!(text_style(&app).unwrap().0.features, ["smcp", tag], "{}", POSITIONS[i]);
            assert_eq!(position_label(shown_position(&app)), POSITIONS[i]);
        }
        edit(&mut app, Change::Position(0));
        assert_eq!(text_style(&app).unwrap().0.features, ["smcp"]);
        assert_eq!(shown_position(&app), Some(0));
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
        assert_eq!(position_label(shown_position(&app)), "Mixed");
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
        out.textures_delta.clear();
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

    /// Type with two positions turned on shows the Position menu as mixed, and the entry of one
    /// of them turns the other off.
    #[test]
    fn two_positions_show_mixed_and_one_is_chosen() {
        let mut app = selected_text();
        set(&mut app, OtFeatures::from_tags(["sups", "numr"]));
        assert_eq!(shown_position(&app), None);
        edit(&mut app, Change::Position(1));
        assert_eq!(text_style(&app).unwrap().0.features, ["sups"]);
        assert_eq!(shown_position(&app), Some(1));
    }

    /// The runs of `id`'s characters `0..end`, each as its text and its features.
    fn run_features(app: &mut VectorcraftApp, id: &Value, end: usize) -> Vec<(String, Value)> {
        let runs = app.session.execute("text.getRange", &json!({"id": id, "start": 0, "end": end})).unwrap()["runs"].clone();
        runs.as_array().unwrap().iter().map(|r| (r["text"].as_str().unwrap_or("").to_string(), r["style"]["features"].clone())).collect()
    }

    /// The Type tool's selected characters show mixed when only some are numerators; Denominator
    /// then applies to those characters alone, each run keeping its other features, and the
    /// Figure menu and the checkboxes also keep each run's other features.
    #[test]
    fn mixed_range_takes_the_choice() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "1/23"})).unwrap()["id"].clone();
        app.session.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 1, "features": ["numr"]})).unwrap();
        app.session.execute("text.setRangeStyle", &json!({"id": id, "start": 1, "end": 3, "features": ["smcp"]})).unwrap();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 0, "end": 3}));
        assert_eq!(text_editing(&app).map(|(_, a, b)| (a, b)), Some((0, 3)));
        assert_eq!(shown_position(&app), None);
        let undo = undo_steps(&app);
        edit(&mut app, Change::Position(4));
        assert_eq!(undo_steps(&app), undo + 1);
        assert_eq!(shown_position(&app), Some(4));
        let none = Value::Null;
        assert_eq!(
            run_features(&mut app, &id, 4),
            [("1".to_string(), json!(["dnom"])), ("/2".to_string(), json!(["smcp", "dnom"])), ("3".to_string(), none.clone())]
        );
        edit(&mut app, Change::Figure(4));
        edit(&mut app, Change::Toggle(Flag::Field(|f| &mut f.swash), true));
        assert_eq!(
            run_features(&mut app, &id, 4),
            [
                ("1".to_string(), json!(["dnom", "onum", "tnum", "swsh"])),
                ("/2".to_string(), json!(["smcp", "dnom", "onum", "tnum", "swsh"])),
                ("3".to_string(), none)
            ]
        );
    }
}
