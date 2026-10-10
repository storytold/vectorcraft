//! Align panel: Align Objects, Distribute Objects, Distribute Spacing (with a spacing value when
//! aligning to a key object) and Align To (selection / key object / artboard).

use egui::Ui;
use serde_json::{Value, json};

use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignTo {
    #[default]
    Selection,
    Key,
    Artboard,
}

impl AlignTo {
    pub fn param(self) -> &'static str {
        match self {
            AlignTo::Selection => "selection",
            AlignTo::Key => "key",
            AlignTo::Artboard => "artboard",
        }
    }
}

/// `object.align` params: a single object aligns to the artboard (Illustrator does the same).
pub fn align_params(base: Value, to: AlignTo, n_selected: usize) -> Value {
    let mut p = base;
    let to = if n_selected == 1 && to == AlignTo::Selection { AlignTo::Artboard } else { to };
    p["to"] = json!(to.param());
    p
}

pub const ALIGN: [(&str, &str, &str, &str); 6] = [
    ("dc-al-left", "Horizontal Align Left", "horizontal", "left"),
    ("dc-al-hcenter", "Horizontal Align Center", "horizontal", "center"),
    ("dc-al-right", "Horizontal Align Right", "horizontal", "right"),
    ("dc-al-top", "Vertical Align Top", "vertical", "top"),
    ("dc-al-vcenter", "Vertical Align Center", "vertical", "center"),
    ("dc-al-bottom", "Vertical Align Bottom", "vertical", "bottom"),
];

pub const DISTRIBUTE: [(&str, &str, &str, &str); 6] = [
    ("dc-dist-top", "Vertical Distribute Top", "vertical", "top"),
    ("dc-dist-vcenter", "Vertical Distribute Center", "vertical", "center"),
    ("dc-dist-bottom", "Vertical Distribute Bottom", "vertical", "bottom"),
    ("dc-dist-left", "Horizontal Distribute Left", "horizontal", "left"),
    ("dc-dist-hcenter", "Horizontal Distribute Center", "horizontal", "center"),
    ("dc-dist-right", "Horizontal Distribute Right", "horizontal", "right"),
];

/// The six align buttons of the Control bar and the Properties panel, `size` points square.
/// They align to what the Align panel's Align To says (the key object while there is one), as
/// its own buttons do (#789).
pub fn align_buttons(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) {
    let n = selection_len(app);
    let to = align_to(app, ui.ctx());
    for (i, (icon, tip, axis, v)) in [
        ("align-start-vertical", "Horizontal Align Left", "horizontal", "left"),
        ("align-center-vertical", "Horizontal Align Center", "horizontal", "center"),
        ("align-end-vertical", "Horizontal Align Right", "horizontal", "right"),
        ("align-start-horizontal", "Vertical Align Top", "vertical", "top"),
        ("align-center-horizontal", "Vertical Align Center", "vertical", "center"),
        ("align-end-horizontal", "Vertical Align Bottom", "vertical", "bottom"),
    ]
    .into_iter()
    .enumerate()
    {
        if i == 3 && widgets::icon_button(ui, "dc-al-center", tl!("Horizontal & Vertical Align Center"), false, size).clicked() {
            app.run("object.align", align_params(json!({"horizontal": "center", "vertical": "center"}), to, n)).ok();
        }
        if widgets::icon_button(ui, icon, tl!(tip), false, size).clicked() {
            app.run("object.align", align_params(json!({axis: v}), to, n)).ok();
        }
    }
}

/// Whether the selection has a key object.
fn has_key(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| st.selection.key.is_some())
}

/// What Align aligns to: the key object while there is one (a click on an object of the
/// selection makes it the key), else the Align To choice.
fn align_to(app: &VectorcraftApp, ctx: &egui::Context) -> AlignTo {
    if has_key(app) { AlignTo::Key } else { pstate(ctx, "align-to") }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = selection_len(app);
    let to = align_to(app, ui.ctx());
    widgets::subheader(ui, tl!("Align Objects:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (i, (icon, tip, axis, v)) in ALIGN.iter().enumerate() {
            if i == 3 {
                ui.add_space(6.0);
                if widgets::icon_button_enabled(ui, "dc-al-center", tl!("Horizontal & Vertical Align Center"), false, n >= 1, 32.0).clicked() {
                    app.run("object.align", align_params(json!({"horizontal": "center", "vertical": "center"}), to, n)).ok();
                }
                ui.add_space(6.0);
            }
            if widgets::icon_button_enabled(ui, icon, tl!(tip), false, n >= 1, 32.0).clicked() {
                app.run("object.align", align_params(json!({*axis: v}), to, n)).ok();
            }
        }
    });
    widgets::divider(ui);
    widgets::subheader(ui, tl!("Distribute Objects:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (i, (icon, tip, axis, v)) in DISTRIBUTE.iter().enumerate() {
            if i == 3 {
                ui.add_space(6.0);
            }
            if widgets::icon_button_enabled(ui, icon, tl!(tip), false, n >= 2, 32.0).clicked() {
                app.run("object.distribute", json!({*axis: v})).ok();
            }
        }
    });
    if pstate::<bool>(ui.ctx(), "align-hide-options") {
        return;
    }
    widgets::divider(ui);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Distribute Spacing:"));
            let spacing: f64 = pstate::<Option<f64>>(ui.ctx(), "align-spacing").unwrap_or(0.0);
            let key = to == AlignTo::Key;
            ui.horizontal(|ui| {
                let on = n >= 2;
                let sp = if key { Some(spacing) } else { None };
                if widgets::icon_button_enabled(ui, "dc-dist-vspace", tl!("Vertical Distribute Space"), false, on, 28.0).clicked() {
                    app.run("object.distributeSpacing", json!({"axis": "vertical", "spacing": sp})).ok();
                }
                if widgets::icon_button_enabled(ui, "dc-dist-hspace", tl!("Horizontal Distribute Space"), false, on, 28.0).clicked() {
                    app.run("object.distributeSpacing", json!({"axis": "horizontal", "spacing": sp})).ok();
                }
                ui.add_enabled_ui(key, |ui| {
                    if let Some(v) = widgets::spin_field(ui, "align-spacing", Some(spacing), app.session.general_unit(), 70.0, 1.0, 0.0, &[]) {
                        set_pstate(ui.ctx(), "align-spacing", Some(v));
                    }
                });
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Align To:"));
            ui.horizontal(|ui| {
                for (v, icon, tip) in [
                    (AlignTo::Selection, "dc-alignto-selection", tl!("Align to Selection")),
                    (AlignTo::Key, "dc-alignto-key", tl!("Align to Key Object")),
                    (AlignTo::Artboard, "dc-alignto-artboard", tl!("Align to Artboard")),
                ] {
                    if widgets::icon_button(ui, icon, tl!(tip), to == v, 28.0).clicked() {
                        set_pstate(ui.ctx(), "align-to", v);
                        // Aligning to the selection or the artboard lets go of the key object.
                        if v != AlignTo::Key && has_key(app) {
                            app.run("select.key", json!({})).ok();
                        }
                    }
                }
            });
        });
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "align-hide-options");
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "align-hide-options", !hidden);
    }
    let pb = app.session.prefs.use_preview_bounds;
    if menu_item(ui, tl!("Use Preview Bounds"), true, pb) {
        super::transform::set_pref(app, "usePreviewBounds", !pb);
    }
    if menu_item(ui, tl!("Cancel Key Object"), align_to(app, ui.ctx()) == AlignTo::Key, false) {
        set_pstate(ui.ctx(), "align-to", AlignTo::Selection);
        app.run("select.key", json!({})).ok();
    }
    menu_item(ui, tl!("Align to Glyph Bounds"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_object_aligns_to_artboard() {
        let p = align_params(json!({"horizontal": "left"}), AlignTo::Selection, 1);
        assert_eq!(p["to"], "artboard");
        let p = align_params(json!({"vertical": "top"}), AlignTo::Selection, 3);
        assert_eq!(p["to"], "selection");
        assert_eq!(align_params(json!({}), AlignTo::Key, 1)["to"], "key");
    }

    #[test]
    fn align_both_centers_params() {
        let p = align_params(json!({"horizontal": "center", "vertical": "center"}), AlignTo::Selection, 2);
        assert_eq!(p["horizontal"], "center");
        assert_eq!(p["vertical"], "center");
        assert_eq!(p["to"], "selection");
    }

    /// #541: while the selection has a key object Align aligns to it, whatever Align To says;
    /// choosing Align to Selection lets the key go.
    #[test]
    fn a_key_object_makes_align_align_to_it() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let ctx = egui::Context::default();
        assert_eq!(align_to(&app, &ctx), AlignTo::Selection);
        app.run("select.key", json!({"id": b})).unwrap();
        assert_eq!(align_to(&app, &ctx), AlignTo::Key);
        app.run("select.key", json!({})).unwrap();
        assert_eq!(align_to(&app, &ctx), AlignTo::Selection, "no key: the choice again");
    }

    /// #789: the Control bar's and the Properties panel's align buttons follow Align To too.
    #[test]
    fn the_other_align_buttons_align_to_the_align_to_choice() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let ctx = egui::Context::default();
        set_pstate(&ctx, "align-to", AlignTo::Artboard);
        // One frame finds the first button (Horizontal Align Right is third); the next clicks it.
        let at = std::cell::Cell::new(egui::Pos2::ZERO);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                ui.horizontal(|ui| {
                    at.set(ui.cursor().min);
                    align_buttons(app, ui, 24.0);
                });
            });
            out.textures_delta.clear();
        };
        frame(&mut app, vec![]);
        let right = at.get() + egui::vec2(2.0 * (24.0 + ctx.global_style().spacing.item_spacing.x) + 12.0, 12.0);
        let press = |pressed| egui::Event::PointerButton { pos: right, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(right), press(true)]);
        frame(&mut app, vec![press(false)]);
        let st = app.session.active().unwrap();
        let x1 = |id: &serde_json::Value| st.doc.bounds_of(&[vectorcraft_doc::NodeId(id.as_u64().unwrap())], false).unwrap().x1;
        assert_eq!((x1(&a), x1(&b)), (200.0, 200.0), "both to the artboard's right edge");
    }
}
