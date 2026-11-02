//! OpenType panel: per-character OpenType features (ligatures, alternates, swashes, small caps,
//! ordinals, fractions, figure style) for the selected text or the Type tool's selected characters.

use egui::Ui;
use serde_json::json;
use vectorcraft_text::{EXTRA_FEATURES, OtFeatures};

use super::character::{text_editing, text_style};
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

/// Apply `f` to the selection's features (the Type tool range, or whole objects).
fn set(app: &mut VectorcraftApp, f: OtFeatures) {
    let tags = f.to_tags();
    let r = match text_editing(app) {
        Some((id, a, b)) if b > a => app.run("text.setRangeStyle", json!({ "id": id.0, "start": a, "end": b, "features": tags })),
        Some((id, _, _)) => app.run("text.setRangeStyle", json!({ "id": id.0, "features": tags })),
        None => app.run("text.setStyle", json!({ "features": tags })),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((st, _)) = text_style(app) else {
        widgets::dim_label(ui, tl!("Select text to set its OpenType features."));
        return;
    };
    let cur = OtFeatures::from_tags(st.features.iter().map(String::as_str));
    let mut next = cur;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Figure:"));
        let fig = figure(&cur);
        if let Some(i) = widgets::dropdown(ui, "ot-figure", FIGURES[fig], &FIGURES, 170.0) {
            set_figure(&mut next, i);
        }
    });
    widgets::divider(ui);
    for (label, flag) in rows() {
        let on = flag.get(&next);
        if widgets::check(ui, label, on, true) {
            flag.put(&mut next, !on);
        }
    }
    if cur.ligatures && vectorcraft_text::ligatures_suppressed_by(st.tracking) && !vectorcraft_text::explicit_ligatures(&st.features) {
        widgets::dim_label(ui, tl!("Ligatures are off while the text is tracked."));
    }
    if next != cur {
        set(app, next);
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
}
