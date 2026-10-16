//! Headless frames of the Stroke panel's reach: Units > Stroke in the panel, the Control bar and
//! the Properties panel, mixed values shown blank, Align Stroke for open paths and type (also
//! while the Type tool edits it), and the
//! weight presets.

use serde_json::json;

use super::tests_appearance::{app_with_rect, click, frame_events, run, text_rect};
use super::*;

type Texts = Vec<(String, egui::Rect)>;

fn texts(ctx: &egui::Context, app: &mut VectorcraftApp, f: impl FnMut(&mut VectorcraftApp, &mut Ui)) -> Texts {
    frame_events(ctx, app, vec![], f)
}

fn shows(t: &Texts, s: &str) -> bool {
    t.iter().any(|(x, _)| x == s)
}

/// The centre of the `n`th (0-based) 24 px button right of a Stroke panel row label.
fn row_button(ctx: &egui::Context, t: &Texts, label: &str, n: usize) -> egui::Pos2 {
    let r = text_rect(t, label);
    let sp = ctx.global_style().spacing.item_spacing.x;
    egui::pos2(r.right() + sp + n as f32 * (24.0 + sp) + 12.0, r.center().y)
}

#[test]
fn weights_and_dashes_show_in_the_stroke_unit_everywhere() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app_with_rect();
    app.session.prefs.units_stroke = "millimeters".into();
    run(&mut app, "stroke.set", json!({"dash": [12, 6]}));
    let mm = vectorcraft_doc::Unit::Millimeters.format(1.0);
    assert_eq!(mm, "0.3528 mm");
    let t = texts(&ctx, &mut app, stroke::show);
    assert!(shows(&t, &mm), "Stroke panel: {t:?}");
    assert!(shows(&t, "4.2333") && shows(&t, "2.1167"), "dash and gap in mm: {t:?}");
    assert!(shows(&texts(&ctx, &mut app, properties::show), &mm), "Properties panel");
    assert!(shows(&texts(&ctx, &mut app, crate::chrome::control_bar), &mm), "Control bar");
}

/// The Properties panel's Stroke row is the Control bar's: the Stroke link opens the Stroke panel
/// as a popover and the weight spinner lists the presets (Discord feedback).
#[test]
fn the_properties_stroke_link_and_weight_presets() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app_with_rect();
    let t = texts(&ctx, &mut app, properties::show);
    assert!(!shows(&t, "Cap:"), "closed: {t:?}");
    click(&ctx, &mut app, text_rect(&t, "Stroke").center(), properties::show);
    let t = texts(&ctx, &mut app, properties::show);
    assert!(shows(&t, "Cap:") && shows(&t, "Align Stroke:"), "the Stroke panel's popover: {t:?}");
    click(&ctx, &mut app, text_rect(&t, "Stroke").center(), properties::show);
    // The weight's dropdown: the 20 pt chevron cell right of the field showing "1 pt".
    let field = text_rect(&texts(&ctx, &mut app, properties::show), "1 pt");
    let chevron = ctx.viewport(|vp| {
        let w = vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).map(|w| w.rect);
        w.filter(|r| r.width() == 20.0 && r.left() > field.right() && r.y_range().contains(field.center().y))
            .min_by(|a, b| a.left().total_cmp(&b.left()))
    });
    click(&ctx, &mut app, chevron.expect("the weight's preset chevron").center(), properties::show);
    let t = texts(&ctx, &mut app, properties::show);
    assert!(!shows(&t, "Cap:") && shows(&t, "0.25 pt") && shows(&t, "100 pt"), "the presets: {t:?}");
    click(&ctx, &mut app, text_rect(&t, "3 pt").center(), properties::show);
    assert_eq!(app.session.shown_stroke().unwrap().width, 3.0);
}

#[test]
fn mixed_weights_show_blank() {
    let ctx = egui::Context::default();
    let mut app = app_with_rect();
    let a = app.session.active().unwrap().selection.objects[0];
    let b = run(&mut app, "shape.rectangle", json!({"x": 120, "y": 10, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut app, "stroke.set", json!({"ids": [b], "weight": 3}));
    run(&mut app, "select.set", json!({ "ids": [a.0, b] }));
    let t = texts(&ctx, &mut app, stroke::show);
    assert!(!shows(&t, "1 pt") && !shows(&t, "3 pt"), "{t:?}");
    run(&mut app, "select.set", json!({ "ids": [b] }));
    assert!(shows(&texts(&ctx, &mut app, stroke::show), "3 pt"), "the cache follows the selection");
}

#[test]
fn inside_and_outside_are_off_for_open_paths_and_on_for_type() {
    let ctx = egui::Context::default();
    let mut app = app_with_rect();
    let align = |app: &VectorcraftApp| current_stroke(app).unwrap().align;
    let at = row_button(&ctx, &texts(&ctx, &mut app, stroke::show), "Align Stroke:", 1);
    click(&ctx, &mut app, at, stroke::show);
    assert_eq!(align(&app), vectorcraft_doc::StrokeAlign::Inside, "a closed path takes an inside stroke");
    run(&mut app, "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    click(&ctx, &mut app, at, stroke::show);
    assert_eq!(align(&app), vectorcraft_doc::StrokeAlign::Center, "an open path doesn't");
    let id = run(&mut app, "text.create", json!({"x": 10, "y": 190, "text": "Hi"}))["id"].as_u64().unwrap();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    run(&mut app, "paint.setStroke", json!({"color": "#000000"}));
    click(&ctx, &mut app, at, stroke::show);
    assert_eq!(align(&app), vectorcraft_doc::StrokeAlign::Inside, "type's characters do, around their glyphs");
}

/// While the Type tool edits text, Align Stroke styles the selected characters' stroke.
#[test]
fn align_stroke_styles_the_characters_the_type_tool_selects() {
    let ctx = egui::Context::default();
    let mut app = app_with_rect();
    let at = row_button(&ctx, &texts(&ctx, &mut app, stroke::show), "Align Stroke:", 2);
    app.session.select_tool("type", Default::default()).unwrap();
    let id = run(&mut app, "text.create", json!({"x": 10, "y": 190, "text": "Hello"}))["id"].as_u64().unwrap();
    run(&mut app, "paint.setStroke", json!({"color": "#000000"}));
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    assert!(character::text_editing(&app).is_some());
    assert!(stroke_mixed(&app, &ctx).can_align, "the buttons are on");
    click(&ctx, &mut app, at, stroke::show);
    let vectorcraft_doc::NodeKind::Text(t) = &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else {
        panic!("not type")
    };
    let aligns: Vec<_> = t.runs.iter().map(|r| r.style.stroke_align).collect();
    assert_eq!(aligns, [vectorcraft_doc::StrokeAlign::Outside, vectorcraft_doc::StrokeAlign::Center], "only the selected characters");
}

#[test]
fn the_weight_presets_run_from_a_quarter_point_to_a_hundred() {
    let pt = stroke::weight_presets(vectorcraft_doc::Unit::Points);
    assert!(pt.contains(&0.25) && pt.contains(&100.0));
    let ctx = egui::Context::default();
    let mut app = app_with_rect();
    // The chevron at the right end of the 120 px weight spinner opens them.
    let t = texts(&ctx, &mut app, stroke::show);
    let r = text_rect(&t, "Weight:");
    let sp = ctx.global_style().spacing.item_spacing.x;
    click(&ctx, &mut app, egui::pos2(r.right() + sp + 110.0, r.center().y), stroke::show);
    let t = texts(&ctx, &mut app, stroke::show);
    assert!(shows(&t, "0.25 pt") && shows(&t, "100 pt"), "{t:?}");
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

/// #991: the spinner left of the Stroke Weight field steps ten with Shift on both arrows and a
/// tenth with Ctrl/Cmd, as ↑/↓ do in the focused field, while a plain click steps one point.
#[test]
fn stroke_weight_stepper_uses_ten_point_shift_steps() {
    use super::tests_appearance::frame_raw;

    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app_with_rect();
    run(&mut app, "stroke.set", json!({"weight": 14}));
    let t = texts(&ctx, &mut app, stroke::show);
    // The 16 px spinner sits just left of the field's text.
    let r = text_rect(&t, "14 pt");
    let x = r.left() - 14.0;
    let click_step = |app: &mut VectorcraftApp, y: f32, modifiers: egui::Modifiers| {
        let pos = egui::pos2(x, y);
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        for event in [egui::Event::PointerMoved(pos), button(true), button(false)] {
            let events = vec![egui::Event::ModifiersChanged(modifiers), event];
            frame_raw(&ctx, app, egui::RawInput { events, ..Default::default() }, stroke::show);
        }
    };

    click_step(&mut app, r.center().y - 5.0, egui::Modifiers::SHIFT);
    assert_eq!(app.session.shown_stroke().unwrap().width, 24.0);
    click_step(&mut app, r.center().y + 5.0, egui::Modifiers::NONE);
    assert_eq!(app.session.shown_stroke().unwrap().width, 23.0);
    click_step(&mut app, r.center().y + 5.0, egui::Modifiers::SHIFT);
    assert_eq!(app.session.shown_stroke().unwrap().width, 13.0);
    // Ctrl/Cmd steps a tenth, as ↑/↓ and the wheel do in the field.
    click_step(&mut app, r.center().y + 5.0, egui::Modifiers::COMMAND);
    assert!((app.session.shown_stroke().unwrap().width - 12.9).abs() < 1e-6);
}
