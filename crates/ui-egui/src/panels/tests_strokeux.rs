//! Headless frames of the Stroke panel's reach: Units > Stroke in the panel, the Control bar and
//! the Properties panel, mixed values shown blank, Align Stroke for open paths and type (also
//! while the Type tool edits it), and the weight presets.

use craft::behavior::PartKey;
use craft::ui::text::NumberField;
use egui::Modifiers;
use serde_json::json;

use super::tests_appearance::{app_with_rect, run};
use super::*;
use crate::tests::panel_harness::{Panel, ROOM};

type Show = fn(&mut VectorcraftApp, &mut Ui);

#[test]
fn stroke_style_choices_cross_the_editor_bridge_in_both_directions() {
    use craft::draw::ui::stroke::{self as draw, StrokeField};
    use vectorcraft_doc::{LineCap, LineJoin, StrokeAlign};

    let mut app = app_with_rect();
    run(&mut app, "paint.setStroke", json!({"color": "#222222"}));
    let mut document = current_stroke(&app).unwrap();
    let rows = [
        (LineCap::Butt, draw::LineCap::Butt, "butt"),
        (LineCap::Round, draw::LineCap::Round, "round"),
        (LineCap::Square, draw::LineCap::Square, "square"),
    ];
    for (choice, shown, keyword) in rows {
        document.cap = choice;
        let style = stroke::style_of(Some(&document));
        assert_eq!(style.cap, shown);
        let params = stroke::params_of(&draw::StrokeStyle::default(), &style, StrokeField::Cap);
        run(&mut app, "stroke.set", params.clone());
        assert_eq!(params, json!({"cap": keyword}));
        assert_eq!(current_stroke(&app).unwrap().cap, choice);
    }
    let rows = [
        (LineJoin::Miter, draw::LineJoin::Miter, "miter"),
        (LineJoin::Round, draw::LineJoin::Round, "round"),
        (LineJoin::Bevel, draw::LineJoin::Bevel, "bevel"),
    ];
    for (choice, shown, keyword) in rows {
        document.join = choice;
        let style = stroke::style_of(Some(&document));
        assert_eq!(style.join, shown);
        let params = stroke::params_of(&draw::StrokeStyle::default(), &style, StrokeField::Join);
        run(&mut app, "stroke.set", params.clone());
        assert_eq!(params, json!({"join": keyword}));
        assert_eq!(current_stroke(&app).unwrap().join, choice);
    }
    let rows = [
        (StrokeAlign::Center, draw::StrokeAlign::Center, "center"),
        (StrokeAlign::Inside, draw::StrokeAlign::Inside, "inside"),
        (StrokeAlign::Outside, draw::StrokeAlign::Outside, "outside"),
    ];
    for (choice, shown, keyword) in rows {
        document.align = choice;
        let style = stroke::style_of(Some(&document));
        assert_eq!(style.align, shown);
        let params = stroke::params_of(&draw::StrokeStyle::default(), &style, StrokeField::Align);
        run(&mut app, "stroke.set", params.clone());
        assert_eq!(params, json!({"align": keyword}));
        assert_eq!(current_stroke(&app).unwrap().align, choice);
    }
}

#[test]
fn compact_stroke_miter_limit_only_applies_to_miter_joins() {
    use craft::ui::contract::Role;
    use egui::{Key, Pos2, Rect};
    use vectorcraft_doc::LineJoin;

    for (join, width) in [("round", crate::panel_docking::PANEL_MIN_WIDTH), ("bevel", 320.0)] {
        let mut app = app_with_rect();
        app.session.prefs.interface_language = "en".into();
        run(&mut app, "paint.setStroke", json!({"color": "#222222"}));
        run(&mut app, "stroke.set", json!({"join": join, "miterLimit": 4}));
        let original = current_stroke(&app).unwrap();
        let room = egui::vec2(width, 500.0);
        let mut panel = Panel::new(app, stroke::show);
        panel.room(room).frame();
        let baseline = panel.app().session.active().unwrap().history.undo.len();
        let edit = |panel: &mut Panel| {
            let field = panel.rect_role("Miter limit", Role::SPIN_BUTTON);
            assert!(Rect::from_min_size(Pos2::ZERO, room).contains_rect(field));
            let value = panel.app().host.i18n().unwrap().format().number(8.5, 1);
            panel.click_at(field.center());
            panel.key(Key::A, Modifiers::COMMAND);
            panel.type_text(&value);
            panel.key(Key::Enter, Modifiers::NONE);
        };
        edit(&mut panel);
        assert_eq!(current_stroke(&panel.app()).unwrap(), original);
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), baseline);
        panel.click("Miter");
        let miter = current_stroke(&panel.app()).unwrap();
        assert_eq!(miter.join, LineJoin::Miter);
        edit(&mut panel);
        let mut expected = miter.clone();
        expected.miter_limit = 8.5;
        assert_eq!(current_stroke(&panel.app()).unwrap(), expected);
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), baseline + 2);
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(current_stroke(&panel.app()).unwrap(), miter);
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(current_stroke(&panel.app()).unwrap(), original);
    }
}

#[test]
fn weights_and_dashes_show_in_the_stroke_unit_everywhere() {
    let mut app = app_with_rect();
    app.session.prefs.units_stroke = "millimeters".into();
    // The Control bar spans the window.
    let surfaces: [(Show, egui::Vec2); 3] = [(stroke::show, ROOM), (properties::show, ROOM), (crate::chrome::control_bar, egui::vec2(1600.0, 200.0))];
    let panel = Panel::new(app, stroke::show);
    for (show, room) in surfaces {
        let mut p =
            Panel::new(std::mem::replace(&mut *panel.app(), VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default())), show);
        p.room(room).frame();
        assert!(super::tests_units::reads(p.value("Weight"), vectorcraft_doc::Unit::Millimeters, 1.0), "{:?}", p.value("Weight"));
        *panel.app() = std::mem::replace(&mut *p.app(), VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
    }
}

#[test]
fn mixed_weights_show_blank() {
    let mut app = app_with_rect();
    let a = app.session.active().unwrap().selection.objects[0];
    let b = run(&mut app, "shape.rectangle", json!({"x": 120, "y": 10, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut app, "stroke.set", json!({"ids": [b], "weight": 3}));
    run(&mut app, "select.set", json!({ "ids": [a.0, b] }));
    let mut panel = Panel::new(app, stroke::show);
    panel.frame();
    assert_eq!(panel.value("Weight").unwrap_or_default(), "");
    run(&mut panel.app(), "select.set", json!({ "ids": [b] }));
    panel.frame();
    assert_eq!(panel.value("Weight"), Some(vectorcraft_doc::Unit::Points.format(3.0)), "the cache follows the selection");
}

#[test]
fn inside_and_outside_are_off_for_open_paths_and_on_for_type() {
    let mut panel = Panel::new(app_with_rect(), stroke::show);
    let align = |p: &Panel| current_stroke(&p.app()).unwrap().align;
    panel.click("Inside");
    assert_eq!(align(&panel), vectorcraft_doc::StrokeAlign::Inside, "a closed path takes an inside stroke");
    run(&mut panel.app(), "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    panel.frame();
    assert!(!panel.enabled("Inside"));
    assert_eq!(align(&panel), vectorcraft_doc::StrokeAlign::Center, "an open path doesn't");
    let id = run(&mut panel.app(), "text.create", json!({"x": 10, "y": 190, "text": "Hi"}))["id"].as_u64().unwrap();
    run(&mut panel.app(), "select.set", json!({ "ids": [id] }));
    run(&mut panel.app(), "paint.setStroke", json!({"color": "#000000"}));
    panel.frame();
    panel.click("Inside");
    assert_eq!(align(&panel), vectorcraft_doc::StrokeAlign::Inside, "type's characters do, around their glyphs");
}

/// While the Type tool edits text, Align Stroke styles the selected characters' stroke.
#[test]
fn align_stroke_styles_the_characters_the_type_tool_selects() {
    let mut app = app_with_rect();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = run(&mut app, "text.create", json!({"x": 10, "y": 190, "text": "Hello"}))["id"].as_u64().unwrap();
    run(&mut app, "paint.setStroke", json!({"color": "#000000"}));
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    assert!(character::text_editing(&app).is_some());
    assert!(stroke_mixed(&app, &crate::testing::ui_context()).can_align, "the buttons are on");
    let mut panel = Panel::new(app, stroke::show);
    panel.click("Outside");
    let app = panel.app();
    let vectorcraft_doc::NodeKind::Text(t) = &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else {
        panic!("not type")
    };
    let aligns: Vec<_> = t.runs.iter().map(|r| r.style.stroke_align).collect();
    assert_eq!(aligns, [vectorcraft_doc::StrokeAlign::Outside, vectorcraft_doc::StrokeAlign::Center], "only the selected characters");
}

/// While the Live Paint Bucket is active, the Stroke panel shows the one stroke the bucket paints
/// with, even when the selected objects' strokes differ, as it does with nothing selected.
#[test]
fn the_stroke_panel_shows_the_live_paint_bucket_stroke_as_one_stroke() {
    let mut app = app_with_rect();
    let a = app.session.active().unwrap().selection.objects[0].0;
    let b = run(&mut app, "shape.rectangle", json!({"x": 200, "y": 200, "width": 40, "height": 40}))["id"].as_u64().unwrap();
    run(&mut app, "stroke.set", json!({"weight": 1, "ids": [a]}));
    run(&mut app, "stroke.set", json!({"weight": 7, "ids": [b]}));
    run(&mut app, "select.set", json!({ "ids": [] }));
    let none_selected = stroke_mixed(&app, &crate::testing::ui_context());
    assert!(none_selected.can_align, "Align Stroke applies with nothing selected");
    run(&mut app, "select.set", json!({ "ids": [a, b] }));
    let mixed = stroke_mixed(&app, &crate::testing::ui_context());
    assert!(mixed.weight, "the selection's weights differ");
    app.session.select_tool("livePaintBucket", Default::default()).unwrap();
    let mixed = stroke_mixed(&app, &crate::testing::ui_context());
    assert_eq!(mixed, none_selected, "one stroke that Align Stroke applies to, as with nothing selected");
    assert_eq!(stroke::shown_weight(&app, current_stroke(&app).as_ref(), &mixed), Some(app.session.paint.stroke_width));
}

#[test]
fn weight_presets_display_cleanly_in_their_native_unit() {
    use vectorcraft_doc::Unit;
    // Round-tripping a native-unit value through to_pt → from_pt → number() must give the same
    // string the ladder was authored with: no "0.353 mm" surprises in the dropdown.
    for (unit, want) in [
        (Unit::Millimeters, ["0.1", "0.25", "0.35", "0.5", "0.75", "1", "30"].as_slice()),
        (Unit::Centimeters, ["0.01", "0.05", "0.1", "0.5", "1", "5"].as_slice()),
        (Unit::Pixels, ["1", "10", "20", "40"].as_slice()),
        (Unit::Inches, ["0.0078", "0.0313", "0.375", "1", "5"].as_slice()),
        (Unit::Points, ["0.25", "0.5", "1", "50", "100"].as_slice()),
    ] {
        let presets = stroke::weight_presets(unit);
        let shown: Vec<String> = presets.iter().map(|pt| unit.number(*pt)).collect();
        for w in want {
            assert!(shown.iter().any(|s| s == w), "{unit:?} preset {w:?} missing from {shown:?}");
        }
    }
    // Picas are exact pt weights (0p1 = 1 pt, 1p = 12 pt, 5p = 60 pt).
    let pc = stroke::weight_presets(Unit::Picas);
    assert!([1.0, 12.0, 60.0].iter().all(|w| pc.contains(w)), "{pc:?}");
    // Units no stroke is measured in keep the pt ladder, in points.
    assert_eq!(stroke::weight_presets(Unit::Meters), stroke::weight_presets(Unit::Points));
}

/// A click on the weight's step arrow in `show`'s surface, drawn in `room`: the stroke's weight afterwards.
fn step(app: VectorcraftApp, show: Show, room: egui::Vec2, part: PartKey) -> f64 {
    let mut panel = Panel::new(app, show);
    panel.room(room).frame();
    panel.click_part("Weight", part, Modifiers::NONE);

    panel.app().session.shown_stroke().unwrap().width
}

/// The weight spinner in the Stroke panel, the Control bar and the Properties panel steps from a
/// weight between the presets to the next preset: 0.52 pt went down to 0 pt.
#[test]
fn every_weight_spinner_steps_to_the_next_preset() {
    // The Control bar spans the window.
    let bar = egui::vec2(1600.0, 200.0);
    let surfaces: [(&str, Show, egui::Vec2); 3] =
        [("Stroke panel", stroke::show, ROOM), ("Control bar", crate::chrome::control_bar, bar), ("Properties panel", properties::show, ROOM)];
    for (surface, show, room) in surfaces {
        for (part, after) in [(NumberField::STEP_DOWN, 0.5), (NumberField::STEP_UP, 0.75)] {
            let mut app = app_with_rect();
            run(&mut app, "stroke.set", json!({"weight": 0.52}));
            assert_eq!(step(app, show, room, part), after, "{surface}, {}", part.name());
        }
    }
}

/// A weight the field shows with a preset's text has reached that preset: one click down on the
/// spinner from 2.004 pt, shown as `0.167 p` like the 2 pt preset, goes on to 1 pt.
#[test]
fn a_spinner_step_from_a_weight_shown_as_a_preset_moves_past_it() {
    let mut app = app_with_rect();
    app.session.prefs.units_stroke = "picas".into();
    run(&mut app, "stroke.set", json!({"weight": 2.004}));
    assert_eq!(step(app, stroke::show, ROOM, NumberField::STEP_DOWN), 1.0);
}

/// With objects of different weights selected the Weight field is blank, and its spinner steps
/// from 0.25 pt (#5349): up gives 0.5 pt and down gives 0 pt, for all of them, whatever the
/// Units > Stroke.
#[test]
fn the_spinner_of_a_blank_weight_steps_from_a_quarter_point() {
    for (unit, part, after) in [
        ("points", NumberField::STEP_UP, 0.5),
        ("points", NumberField::STEP_DOWN, 0.0),
        ("millimeters", NumberField::STEP_UP, 0.5),
        ("millimeters", NumberField::STEP_DOWN, 0.0),
    ] {
        let mut app = app_with_rect();
        app.session.prefs.units_stroke = unit.into();
        let a = app.session.active().unwrap().selection.objects[0];
        let b = run(&mut app, "shape.rectangle", json!({"x": 120, "y": 10, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        run(&mut app, "stroke.set", json!({"ids": [b], "weight": 3}));
        run(&mut app, "select.set", json!({ "ids": [a.0, b] }));
        let mut panel = Panel::new(app, stroke::show);
        panel.frame();
        panel.click_part("Weight", part, Modifiers::NONE);
        for id in [a.0, b] {
            run(&mut panel.app(), "select.set", json!({ "ids": [id] }));
            assert_eq!(panel.app().session.shown_stroke().unwrap().width, after, "object {id}, {}, {unit}", part.name());
        }
    }
}
