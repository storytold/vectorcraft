//! Align panel: Align Objects, Distribute Objects and Distribute Spacing (both with the spacing
//! value when aligning to a key object) and Align To (selection / key object / artboard).

use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};

use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

/// What Align aligns to. The Align To choice kept with the UI state ([`crate::UiState::align_to`])
/// is Selection or Artboard: Align to Key Object stores no choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
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

/// Serde reader for the saved Align To choice ([`crate::UiState::align_to`]): `"artboard"` is
/// Artboard, and any other value, `"key"` included, is Selection.
pub fn deserialize_align_to<'de, D: serde::Deserializer<'de>>(d: D) -> Result<AlignTo, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(if v == "artboard" { AlignTo::Artboard } else { AlignTo::Selection })
}

/// What Align aligns to with `n_selected` objects selected: the artboard for a single object, else
/// `to`.
fn aligns_to(to: AlignTo, n_selected: usize) -> AlignTo {
    if n_selected == 1 { AlignTo::Artboard } else { to }
}

/// `object.align` params: a single object aligns to the artboard (Illustrator does the same).
pub fn align_params(base: Value, to: AlignTo, n_selected: usize) -> Value {
    let mut p = base;
    p["to"] = json!(aligns_to(to, n_selected).param());
    p
}

/// `object.align` params `base` with the `to` that the align buttons of the Align panel, the Control
/// bar and the Properties panel send: the artboard for a single selected object, else the key object
/// while there is one, else the Align To choice. Object → Align sends it too while every selected
/// object is selected whole ([`crate::menus::invoke`]).
pub fn with_align_to(app: &VectorcraftApp, base: Value) -> Value {
    align_params(base, align_to(app), objects_aligned(app))
}

/// The selected objects that Align aligns as objects: none while anchor points are selected
/// (Direct Selection), since the points align and a single path's points must not go to the
/// artboard as a single object does (#1636).
fn objects_aligned(app: &VectorcraftApp) -> usize {
    if has_anchors(app) { 0 } else { selection_len(app) }
}

/// Whether anchor points are selected (Direct Selection): Align and Distribute then act on the points.
fn has_anchors(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| !st.selection.anchors.is_empty())
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
            app.run("object.align", with_align_to(app, json!({"horizontal": "center", "vertical": "center"}))).ok();
        }
        if widgets::icon_button(ui, icon, tl!(tip), false, size).clicked() {
            app.run("object.align", with_align_to(app, json!({axis: v}))).ok();
        }
    }
}

/// Whether the selection has a key object.
fn has_key(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| st.selection.key.is_some())
}

/// The topmost selected object, the last in paint order.
fn topmost(app: &VectorcraftApp) -> Option<vectorcraft_doc::NodeId> {
    app.session.active().and_then(|st| st.selection.in_paint_order(&st.doc).last().copied())
}

/// What Align aligns to: the key object while there is one (a click on an object of the
/// selection makes it the key, and Align to Key Object makes the topmost one the key), else the
/// stored Align To choice, Selection or Artboard. The Distribute Objects and Distribute Spacing
/// buttons send it as their `to`.
fn align_to(app: &VectorcraftApp) -> AlignTo {
    if has_key(app) { AlignTo::Key } else { app.ui.align_to }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = selection_len(app);
    let to = align_to(app);
    // `spacing` is the Distribute Spacing value. The Distribute Objects and Distribute Spacing
    // buttons send it while there is a key object.
    let spacing: f64 = pstate::<Option<f64>>(ui.ctx(), "align-spacing").unwrap_or(0.0);
    let key = to == AlignTo::Key;
    let sp = if key { Some(spacing) } else { None };
    // Two or more objects, or anchor points, which may all be on one path.
    let distributes = n >= 2 || has_anchors(app);
    widgets::subheader(ui, tl!("Align Objects:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (i, (icon, tip, axis, v)) in ALIGN.iter().enumerate() {
            if i == 3 {
                ui.add_space(6.0);
                if widgets::icon_button_enabled(ui, "dc-al-center", tl!("Horizontal & Vertical Align Center"), false, n >= 1, 32.0).clicked() {
                    app.run("object.align", with_align_to(app, json!({"horizontal": "center", "vertical": "center"}))).ok();
                }
                ui.add_space(6.0);
            }
            if widgets::icon_button_enabled(ui, icon, tl!(tip), false, n >= 1, 32.0).clicked() {
                app.run("object.align", with_align_to(app, json!({*axis: v}))).ok();
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
            if widgets::icon_button_enabled(ui, icon, tl!(tip), false, distributes, 32.0).clicked() {
                let mut p = json!({*axis: v, "to": to.param()});
                if let Some(g) = sp {
                    p["spacing"] = json!(g);
                }
                app.run("object.distribute", p).ok();
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
            ui.horizontal(|ui| {
                let on = distributes;
                if widgets::icon_button_enabled(ui, "dc-dist-vspace", tl!("Vertical Distribute Space"), false, on, 28.0).clicked() {
                    app.run("object.distributeSpacing", json!({"axis": "vertical", "spacing": sp, "to": to.param()})).ok();
                }
                if widgets::icon_button_enabled(ui, "dc-dist-hspace", tl!("Horizontal Distribute Space"), false, on, 28.0).clicked() {
                    app.run("object.distributeSpacing", json!({"axis": "horizontal", "spacing": sp, "to": to.param()})).ok();
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
            // With one object selected the buttons align it to the artboard, and Align To shows Artboard.
            let shown = aligns_to(to, objects_aligned(app));
            ui.horizontal(|ui| {
                for (v, icon, tip) in [
                    (AlignTo::Selection, "dc-alignto-selection", tl!("Align to Selection")),
                    (AlignTo::Key, "dc-alignto-key", tl!("Align to Key Object")),
                    (AlignTo::Artboard, "dc-alignto-artboard", tl!("Align to Artboard")),
                ] {
                    // A key object is one of two or more selected objects.
                    let on = v != AlignTo::Key || n >= 2;
                    if widgets::icon_button_enabled(ui, icon, tl!(tip), shown == v, on, 28.0).clicked() {
                        if v == AlignTo::Key {
                            // The topmost selected object becomes the key object, unless
                            // there is one already. The Align To choice stays as it was, and
                            // Align To shows it again once the key is cleared.
                            if !has_key(app)
                                && let Some(id) = topmost(app)
                            {
                                app.run("select.key", json!({"id": id.0})).ok();
                            }
                        } else {
                            app.ui.align_to = v;
                            // Aligning to the selection or the artboard lets go of the key object.
                            if has_key(app) {
                                app.run("select.key", json!({})).ok();
                            }
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
    // Cancel Key Object clears the key, and Align To shows its stored choice again.
    if menu_item(ui, tl!("Cancel Key Object"), align_to(app) == AlignTo::Key, false) {
        app.run("select.key", json!({})).ok();
    }
    menu_item(ui, tl!("Align to Glyph Bounds"), false, false);
}

#[cfg(test)]
mod tests {
    use super::super::tests_appearance::{click, frame_events, text_rect};
    use super::*;
    use crate::toolbar::tests::widget_rects;

    #[test]
    fn single_object_aligns_to_artboard() {
        let p = align_params(json!({"horizontal": "left"}), AlignTo::Selection, 1);
        assert_eq!(p["to"], "artboard");
        let p = align_params(json!({"vertical": "top"}), AlignTo::Selection, 3);
        assert_eq!(p["to"], "selection");
        assert_eq!(align_params(json!({}), AlignTo::Key, 1)["to"], "artboard");
    }

    /// A Shift-click that leaves the key object selected on its own lets the key go, and the align
    /// buttons of the Control bar and the Properties panel, and then the Align panel's, align that
    /// object to the artboard.
    #[test]
    fn a_lone_object_aligns_to_the_artboard() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
        let ctx = egui::Context::default();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("select.key", json!({"id": b})).unwrap();
        app.run("select.toggle", json!({"id": a})).unwrap();
        let id = vectorcraft_doc::NodeId(b.as_u64().unwrap());
        let bounds = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
        // `align_buttons` (the Control bar and the Properties panel): one frame finds the first
        // button, and the next clicks Horizontal Align Right, the third.
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
        assert_eq!(bounds(&app).x1, 200.0, "the artboard's right edge");
        // The Align panel's Horizontal Align Left.
        let p = with_align_to(&app, json!({"horizontal": "left", "bounds": "geometric"}));
        app.run("object.align", p).unwrap();
        assert_eq!(bounds(&app).x0, 0.0, "the artboard's left edge");
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
        assert_eq!(align_to(&app), AlignTo::Selection);
        app.run("select.key", json!({"id": b})).unwrap();
        assert_eq!(align_to(&app), AlignTo::Key);
        app.run("select.key", json!({})).unwrap();
        assert_eq!(align_to(&app), AlignTo::Selection, "no key: the choice again");
    }

    /// Two rectangles, the back one at x 10 and the front one at x 80, selected front first →
    /// (the app, the back one's id, the front one's id).
    fn back_and_front() -> (VectorcraftApp, Value, Value) {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let back = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        let front = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [front, back]})).unwrap();
        (app, back, front)
    }

    /// One frame of the Align panel → its Align To buttons (Selection, Key Object, Artboard) and
    /// its Horizontal Align Left button.
    fn panel_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> ([egui::Rect; 3], egui::Rect) {
        frame_events(ctx, app, vec![], show);
        // The 28-point buttons are Distribute Spacing's two and then Align To's three.
        let mut small = widget_rects(ctx, egui::vec2(28.0, 28.0));
        small.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [.., selection, key, artboard] = small[..] else { panic!("no Align To buttons in {small:?}") };
        let objects = widget_rects(ctx, egui::vec2(32.0, 32.0));
        let left = objects.iter().copied().filter(|r| r.top() == objects[0].top()).min_by(|a, b| a.left().total_cmp(&b.left())).unwrap();
        ([selection, key, artboard], left)
    }

    /// The left edge of object `id`.
    fn left_edge(app: &VectorcraftApp, id: &Value) -> f64 {
        let id = vectorcraft_doc::NodeId(id.as_u64().unwrap());
        app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0
    }

    /// With two objects selected and no key object, Align to Key Object makes the topmost one the
    /// key, whatever order they were selected in, and Align then aligns to it. A key already set
    /// stays the key.
    #[test]
    fn align_to_key_object_makes_the_topmost_object_the_key() {
        let (mut app, back, front) = back_and_front();
        let ctx = egui::Context::default();
        let ([_, key, _], left) = panel_buttons(&mut app, &ctx);
        click(&ctx, &mut app, key.center(), show);
        let key_object = |app: &VectorcraftApp| app.session.active().unwrap().selection.key.map(|k| k.0);
        assert_eq!(key_object(&app), front.as_u64(), "the front rectangle, selected first");
        app.run("select.set", json!({"ids": [back, front]})).unwrap();
        click(&ctx, &mut app, key.center(), show);
        assert_eq!(key_object(&app), front.as_u64(), "the front rectangle, selected last");
        assert_eq!(align_to(&app), AlignTo::Key);
        click(&ctx, &mut app, left.center(), show);
        assert_eq!((left_edge(&app, &back), left_edge(&app, &front)), (80.0, 80.0), "to the key's left edge: {}", app.ui.status);
        app.run("select.key", json!({"id": back})).unwrap();
        click(&ctx, &mut app, key.center(), show);
        assert_eq!(key_object(&app), back.as_u64(), "a key already set");
    }

    /// Align to Key Object stores no Align To choice: once the key object is cleared, by Cancel
    /// Key Object, by a click on the key or by another selection, Align To shows the stored
    /// choice again.
    #[test]
    fn align_to_shows_its_choice_again_once_the_key_is_cleared() {
        let (mut app, back, front) = back_and_front();
        let ctx = egui::Context::default();
        let ([_, key, artboard], _) = panel_buttons(&mut app, &ctx);
        click(&ctx, &mut app, artboard.center(), show);
        click(&ctx, &mut app, key.center(), show);
        assert_eq!(align_to(&app), AlignTo::Key);
        let cancel = text_rect(&frame_events(&ctx, &mut app, vec![], menu), "   Cancel Key Object");
        click(&ctx, &mut app, cancel.center(), menu);
        assert!(!has_key(&app));
        assert_eq!(align_to(&app), AlignTo::Artboard, "Cancel Key Object");
        // The Selection tool's click on the key object runs `select.key {}`.
        click(&ctx, &mut app, key.center(), show);
        assert!(has_key(&app));
        app.run("select.key", json!({})).unwrap();
        assert_eq!(align_to(&app), AlignTo::Artboard, "a click on the key");
        click(&ctx, &mut app, key.center(), show);
        assert!(has_key(&app));
        app.run("select.set", json!({"ids": [back, front]})).unwrap();
        assert_eq!(align_to(&app), AlignTo::Artboard, "another selection");
    }

    /// The Align To choice is kept with the UI state, which the app saves with its preferences and
    /// `ui.inspect` reports. A saved value other than "artboard" reads as Selection, and the rest of
    /// the state still loads.
    #[test]
    fn the_align_to_choice_is_kept_with_the_ui_state() {
        let (mut app, _, _) = back_and_front();
        let ctx = egui::Context::default();
        let ([_, _, artboard], _) = panel_buttons(&mut app, &ctx);
        click(&ctx, &mut app, artboard.center(), show);
        assert_eq!(crate::control::inspect(&app, &ctx)["ui"]["align_to"], "artboard");
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(back.align_to, AlignTo::Artboard);
        for v in [json!("key"), json!("Artboard"), json!(3), Value::Null] {
            let ui: crate::UiState = serde_json::from_value(json!({"align_to": v, "place_link": false})).unwrap();
            assert_eq!((ui.align_to, ui.place_link), (AlignTo::Selection, false), "{v}");
        }
        let old: crate::UiState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old.align_to, AlignTo::Selection, "preferences saved without it");
    }

    /// With one object selected, Align to Key Object is disabled: a click sets no key and keeps the
    /// Align To choice, and Horizontal Align Left moves the object to the artboard's left edge.
    #[test]
    fn align_to_key_object_needs_two_selected_objects() {
        let (mut app, _, front) = back_and_front();
        app.run("select.set", json!({"ids": [front]})).unwrap();
        let ctx = egui::Context::default();
        let ([_, key, _], left) = panel_buttons(&mut app, &ctx);
        let clickable = ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).any(|w| w.rect == key && w.sense.senses_click()));
        assert!(!clickable, "disabled");
        click(&ctx, &mut app, key.center(), show);
        assert_eq!(app.session.active().unwrap().selection.key, None);
        assert_eq!(align_to(&app), AlignTo::Selection);
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(left_edge(&app, &front), 0.0, "the artboard's left edge");
    }

    /// One frame of the Align panel → the rects of the Align To buttons drawn as chosen.
    fn chosen_align_to(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<egui::Rect> {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        let fill = crate::theme::Tokens::get(ctx).tool_active;
        let chosen = |s: &egui::Shape| match s {
            egui::Shape::Rect(r) if r.fill == fill && r.rect.size() == egui::vec2(28.0, 28.0) => Some(r.rect),
            _ => None,
        };
        out.shapes.iter().filter_map(|c| chosen(&c.shape)).collect()
    }

    /// With one object selected the align buttons align it to the artboard, and Align To shows
    /// Artboard; with two, it shows the stored choice.
    #[test]
    fn align_to_shows_artboard_while_one_object_is_selected() {
        let (mut app, _, front) = back_and_front();
        let ctx = egui::Context::default();
        let ([selection, _, artboard], _) = panel_buttons(&mut app, &ctx);
        assert_eq!(chosen_align_to(&mut app, &ctx), [selection], "two objects");
        app.run("select.set", json!({"ids": [front]})).unwrap();
        assert_eq!(chosen_align_to(&mut app, &ctx), [artboard], "one object");
        assert_eq!(align_to(&app), AlignTo::Selection, "the stored choice");
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
        app.ui.align_to = AlignTo::Artboard;
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

    /// One frame of the Align panel → its Horizontal Distribute Left and Horizontal Distribute
    /// Space buttons.
    fn distribute_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> (egui::Rect, egui::Rect) {
        frame_events(ctx, app, vec![], show);
        // The lower row of 32-point buttons is Distribute Objects': Vertical Distribute Top, Center
        // and Bottom, then Horizontal Distribute Left, Center and Right.
        let objects = widget_rects(ctx, egui::vec2(32.0, 32.0));
        let bottom = objects.last().unwrap().top();
        let mut row: Vec<egui::Rect> = objects.into_iter().filter(|r| r.top() == bottom).collect();
        row.sort_by(|a, b| a.left().total_cmp(&b.left()));
        // The 28-point buttons are Distribute Spacing's two (Vertical, then Horizontal) and then
        // Align To's three.
        let mut small = widget_rects(ctx, egui::vec2(28.0, 28.0));
        small.sort_by(|a, b| a.left().total_cmp(&b.left()));
        (row[3], small[1])
    }

    /// With Align to Artboard chosen, the Align panel's Horizontal Distribute Left puts the
    /// outermost of three objects (20, 40 and 20 pt wide) on the active artboard's left and right
    /// edges and spaces their left edges evenly, and Horizontal Distribute Space makes the gaps
    /// between them equal. With Align to Selection chosen, the outermost objects stay put.
    #[test]
    fn the_distribute_buttons_follow_align_to_artboard() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200, "artboards": 2})).unwrap();
        let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        let ids: Vec<Value> = [(10, 20), (50, 40), (120, 20)]
            .into_iter()
            .map(|(x, w)| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": w, "height": 10})).unwrap()["id"].clone())
            .collect();
        app.run("select.set", json!({"ids": ids})).unwrap();
        let lefts = |app: &VectorcraftApp| ids.iter().map(|id| left_edge(app, id)).collect::<Vec<_>>();
        let ctx = egui::Context::default();
        let (left, space) = distribute_buttons(&mut app, &ctx);
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(lefts(&app), [10.0, 65.0, 120.0], "Align to Selection: {}", app.ui.status);
        app.run("edit.undo", json!({})).unwrap();
        // The objects are on artboard 0, and artboard 1 is the active one.
        app.ui.align_to = AlignTo::Artboard;
        app.view_mut().unwrap().artboard = 1;
        let (x0, x1) = (boards[1].x0, boards[1].x1);
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(lefts(&app), [x0, x0 + 90.0, x1 - 20.0], "the active artboard's edges: {}", app.ui.status);
        app.run("edit.undo", json!({})).unwrap();
        click(&ctx, &mut app, space.center(), show);
        assert_eq!(lefts(&app), [x0, x0 + 80.0, x1 - 20.0], "60 pt gaps: {}", app.ui.status);
    }

    /// With a key object, the Align panel's Distribute Objects buttons send the Distribute Spacing
    /// value: Horizontal Distribute Left puts the others' left edges on the key's left edge with
    /// the default value of 0, and puts them 15 pt apart from it with 15. Without a key object
    /// they send no spacing.
    #[test]
    fn the_distribute_objects_buttons_send_the_spacing_with_a_key_object() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let ids: Vec<Value> = [(10, 20), (50, 40), (120, 20)]
            .into_iter()
            .map(|(x, w)| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": w, "height": 10})).unwrap()["id"].clone())
            .collect();
        app.run("select.set", json!({"ids": ids})).unwrap();
        let lefts = |app: &VectorcraftApp| ids.iter().map(|id| left_edge(app, id)).collect::<Vec<_>>();
        // The command and its `to` and `spacing` params (the app adds the active artboard).
        let sent = |app: &VectorcraftApp| {
            let (id, p) = app.session.journal.last().unwrap();
            (id.clone(), p["to"].clone(), p.get("spacing").cloned())
        };
        let ctx = egui::Context::default();
        let (left, _) = distribute_buttons(&mut app, &ctx);
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(sent(&app), ("object.distribute".to_string(), json!("selection"), None), "no key object");
        assert_eq!(lefts(&app), [10.0, 65.0, 120.0], "{}", app.ui.status);
        app.run("edit.undo", json!({})).unwrap();
        app.run("select.key", json!({"id": ids[1]})).unwrap();
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(sent(&app), ("object.distribute".to_string(), json!("key"), Some(json!(0.0))));
        assert_eq!(lefts(&app), [50.0; 3], "on the key's left edge: {}", app.ui.status);
        app.run("edit.undo", json!({})).unwrap();
        set_pstate(&ctx, "align-spacing", Some(15.0));
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(lefts(&app), [35.0, 50.0, 65.0], "15 pt apart: {}", app.ui.status);
    }

    /// Runs the Object → Align item `label` as a click on it does.
    fn object_align(app: &mut VectorcraftApp, label: &str) {
        let entry = crate::menus::menu_entries(app).into_iter().find(|e| e.path == ["Object", "Align"] && e.label == label).expect("menu item");
        let (id, p) = crate::menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
        crate::menus::invoke(app, &id, p);
    }

    /// Object → Align aligns as the align buttons do: it aligns one object to the active artboard
    /// whatever Align To says, and with Align to Artboard chosen it aligns two objects to the active
    /// artboard too.
    #[test]
    fn object_align_follows_align_to() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200, "artboards": 2})).unwrap();
        let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
        let bounds = |app: &VectorcraftApp, id: &Value| {
            app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap()
        };
        app.run("select.set", json!({"ids": [b]})).unwrap();
        object_align(&mut app, "Horizontal Align Left");
        assert_eq!(bounds(&app, &b).x0, 0.0, "one object, Align to Selection: the artboard's left edge");
        app.ui.align_to = AlignTo::Artboard;
        app.view_mut().unwrap().artboard = 1;
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        object_align(&mut app, "Horizontal Align Right");
        assert_eq!((bounds(&app, &a).x1, bounds(&app, &b).x1), (boards[1].x1, boards[1].x1), "the active artboard's right edge");
    }

    /// With anchor points selected, Object → Align sends its params unchanged: two anchor points of
    /// one path align to their own bounds with Align to Selection and with Align to Artboard chosen.
    #[test]
    fn object_align_aligns_anchor_points_to_each_other() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("path.create", json!({"d": "M100 40 L200 60 L150 160 Z"})).unwrap()["id"].as_u64().unwrap();
        let anchors = |app: &VectorcraftApp| -> Vec<(f64, f64)> {
            let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
            node.path_data().unwrap().anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect()
        };
        for to in [AlignTo::Selection, AlignTo::Artboard] {
            app.ui.align_to = to;
            app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
            object_align(&mut app, "Horizontal Align Left");
            assert_eq!(anchors(&app), [(100.0, 40.0), (100.0, 60.0), (150.0, 160.0)], "{to:?}: {}", app.ui.status);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(anchors(&app)[1], (200.0, 60.0), "undone");
        }
    }

    /// #1636: with two anchor points of one path selected, the Align panel's buttons align the
    /// points to each other, not to the artboard as they align a lone object, and Align To shows
    /// the stored choice. With Align to Artboard chosen the points go to the artboard.
    #[test]
    fn anchor_points_of_one_path_align_to_each_other() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("path.create", json!({"d": "M100 40 L200 60 L150 160 Z"})).unwrap()["id"].as_u64().unwrap();
        let xs = |app: &VectorcraftApp| -> Vec<f64> {
            let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
            node.path_data().unwrap().anchors().map(|(_, _, a)| a.p.x).collect()
        };
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
        let ctx = egui::Context::default();
        let ([selection, _, artboard], left) = panel_buttons(&mut app, &ctx);
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(xs(&app), [100.0, 100.0, 150.0], "to the points' left edge: {}", app.ui.status);
        assert_eq!(chosen_align_to(&mut app, &ctx), [selection], "the stored choice");
        app.run("edit.undo", json!({})).unwrap();
        click(&ctx, &mut app, artboard.center(), show);
        click(&ctx, &mut app, left.center(), show);
        assert_eq!(xs(&app), [0.0, 0.0, 150.0], "to the artboard's left edge: {}", app.ui.status);
    }

    /// #1636: the Distribute buttons work on the anchor points of one path (one object selected).
    #[test]
    fn distribute_buttons_space_the_anchor_points_of_one_path() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("path.create", json!({"d": "M10 10 L20 50 L100 30"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1], [0, 2]], "mode": "set"})).unwrap();
        let ctx = egui::Context::default();
        frame_events(&ctx, &mut app, vec![], show);
        // The 32-point buttons are the Align row and, below it, the Distribute row, whose fifth
        // button is Horizontal Distribute Center.
        let all = widget_rects(&ctx, egui::vec2(32.0, 32.0));
        let top = all.iter().map(|r| r.top()).fold(f32::MIN, f32::max);
        let mut row: Vec<_> = all.into_iter().filter(|r| r.top() == top).collect();
        row.sort_by(|a, b| a.left().total_cmp(&b.left()));
        assert_eq!(row.len(), 6, "the Distribute row");
        let hit = row[4].center();
        click(&ctx, &mut app, hit, show);
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        let xs: Vec<f64> = node.path_data().unwrap().anchors().map(|(_, _, a)| a.p.x).collect();
        assert_eq!(xs, [10.0, 55.0, 100.0], "{}", app.ui.status);
    }
}
