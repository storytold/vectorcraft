//! The font menu's samples, ↑/↓ highlight with live preview, and applying or undoing the preview.

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_text::FontDb;

use crate::VectorcraftApp;
use crate::font_menu::{FontPick, MenuLook, font_menu};

fn key(k: egui::Key) -> Vec<egui::Event> {
    [true, false].map(|pressed| egui::Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Default::default() }).to_vec()
}

/// Recently used means applied, including an edited character range, never just hovered.
#[test]
fn recent_fonts_follow_committed_choices_and_the_count_preference() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Recent fonts", "font": "Inter"})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    crate::font_menu::apply(&mut app, &ctx, FontPick::Preview("Source Serif 4".into(), None));
    assert!(app.recent_fonts().is_empty());
    crate::font_menu::apply(&mut app, &ctx, FontPick::EndPreview);
    assert!(app.recent_fonts().is_empty());
    crate::font_menu::apply(&mut app, &ctx, FontPick::Chosen("Source Serif 4".into(), None));
    app.run("text.setStyle", json!({"font": "Inter"})).unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 6, "font": "Source Serif 4"})).unwrap();
    assert_eq!(app.recent_fonts(), ["Source Serif 4", "Inter"]);
    // A failed command must leave history alone.
    assert!(app.run("text.setRangeStyle", json!({"id": 999999, "font": "Missing"})).is_err());
    assert_eq!(app.recent_fonts(), ["Source Serif 4", "Inter"]);
    app.run("prefs.set", json!({"values": {"recentFontsCount": 1}})).unwrap();
    assert_eq!(MenuLook::of(&app).recent, ["Source Serif 4"]);
    let restored: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&app.ui).unwrap()).unwrap();
    assert_eq!(restored.recent_fonts, ["Source Serif 4", "Inter"]);
}

#[test]
fn arrows_preview_the_next_family_and_enter_applies_it() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let (mut at, mut picked) = (egui::Pos2::ZERO, None);
        let mut out = ctx.run_ui(input, |ui| {
            at = ui.next_widget_position();
            picked = font_menu(ui, "test-menu", "Inter", 260.0, Some("Gagaku"), MenuLook::default());
        });
        out.textures_delta.clear();
        (at, picked)
    };
    let (at, _) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(true)]);
    frame(vec![button(false)]);
    for _ in 0..5 {
        frame(vec![]);
    }
    let families = FontDb::global().families();
    let inter = families.iter().position(|f| f == "Inter").unwrap();
    let next = families[inter + 1].clone();
    let (_, picked) = frame(key(egui::Key::ArrowDown));
    assert_eq!(picked, Some(FontPick::Preview(next.clone(), None)), "↓ previews the family below the current one");
    let (_, picked) = frame(key(egui::Key::ArrowUp));
    assert_eq!(picked, Some(FontPick::EndPreview), "back on the current family, the preview goes");
    frame(key(egui::Key::ArrowDown));
    let (_, picked) = frame(key(egui::Key::Enter));
    assert_eq!(picked, Some(FontPick::Chosen(next, None)));
}

#[test]
fn escape_closes_the_menu_and_ends_the_preview() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let (mut at, mut picked) = (egui::Pos2::ZERO, None);
        let mut out = ctx.run_ui(input, |ui| {
            at = ui.next_widget_position();
            picked = font_menu(ui, "test-esc", "Inter", 260.0, None, MenuLook::default());
        });
        out.textures_delta.clear();
        (at, picked)
    };
    let (at, _) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(true)]);
    frame(vec![button(false)]);
    for _ in 0..5 {
        frame(vec![]);
    }
    assert!(matches!(frame(key(egui::Key::ArrowDown)).1, Some(FontPick::Preview(..))));
    let mut ended = false;
    for events in [key(egui::Key::Escape), vec![], vec![], vec![]] {
        ended |= frame(events).1 == Some(FontPick::EndPreview);
    }
    assert!(ended, "closing without a choice ends the preview");
}

#[test]
fn samples_show_the_text_in_the_family_or_a_generic_sample() {
    let latin = crate::font_menu::render_sample_for_test("Source Serif 4", "Gagaku", 24).unwrap();
    assert_eq!(latin.size[1], 24);
    assert!(latin.pixels.iter().any(|p| p.a() > 0), "ink");
    // Japanese text in a Latin font: the generic sample, not glyphs from another font.
    let ja = crate::font_menu::render_sample_for_test("Source Serif 4", "雅楽", 24).unwrap();
    let sample = crate::font_menu::render_sample_for_test("Source Serif 4", "Sample", 24).unwrap();
    assert_eq!(ja.size, sample.size);
}

#[test]
fn preview_changes_the_text_live_and_only_a_choice_is_a_step() {
    let ctx = egui::Context::default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let font = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.first_style().font_family,
        _ => String::new(),
    };
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    assert_eq!(crate::font_menu::sample_text(&app).as_deref(), Some("Gagaku"));
    let before = steps(&app);
    crate::font_menu::apply(&mut app, &ctx, FontPick::Preview("Source Serif 4".into(), None));
    assert_eq!(font(&app), "Source Serif 4", "previewed on the text");
    crate::font_menu::apply(&mut app, &ctx, FontPick::Preview("Source Sans 3".into(), None));
    assert_eq!(font(&app), "Source Sans 3");
    crate::font_menu::apply(&mut app, &ctx, FontPick::EndPreview);
    assert_eq!((font(&app), steps(&app)), ("Inter".to_string(), before), "the preview leaves nothing behind");
    crate::font_menu::apply(&mut app, &ctx, FontPick::Preview("Source Serif 4".into(), None));
    crate::font_menu::apply(&mut app, &ctx, FontPick::Chosen("Source Serif 4".into(), None));
    assert_eq!((font(&app), steps(&app)), ("Source Serif 4".to_string(), before + 1), "one step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(font(&app), "Inter");
}

/// A preview whose menu isn't drawn any more (the text was deselected, the panel closed) is undone
/// at the start of the next frame, before the canvas sees its input.
#[test]
fn a_preview_left_open_by_a_menu_that_is_gone_is_undone() {
    let ctx = egui::Context::default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let steps = app.session.active().unwrap().history.undo.len();
    crate::font_menu::apply(&mut app, &ctx, FontPick::Preview("Source Serif 4".into(), None));
    assert!(app.session.in_interaction());
    crate::font_menu::end_stale_preview(&mut app, &ctx);
    assert!(!app.session.in_interaction(), "no menu drawn: the preview is undone");
    let font = match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.first_style().font_family,
        _ => String::new(),
    };
    assert_eq!((font.as_str(), app.session.active().unwrap().history.undo.len()), ("Inter", steps));
    // With nothing previewed it does nothing.
    crate::font_menu::end_stale_preview(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
}

/// A press outside the open menu ends the preview in that frame, before the canvas could act on
/// it inside the preview's interaction (the menu itself closes on the release).
#[test]
fn a_press_outside_the_menu_ends_the_preview_at_once() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let mut frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let mut at = egui::Pos2::ZERO;
        let mut out = ctx.run_ui(input, |ui| {
            crate::font_menu::end_stale_preview(&mut app, ui.ctx());
            at = ui.next_widget_position();
            if let Some(pick) = font_menu(ui, "test-press", "Inter", 260.0, None, MenuLook::default()) {
                crate::font_menu::apply(&mut app, ui.ctx(), pick);
            }
        });
        out.textures_delta.clear();
        (at, app.session.in_interaction())
    };
    let (at, _) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(click, true)]);
    frame(vec![button(click, false)]);
    for _ in 0..5 {
        frame(vec![]);
    }
    assert!(frame(key(egui::Key::ArrowDown)).1, "↓ previews");
    assert!(frame(vec![]).1, "the preview lasts while the menu is open");
    let outside = egui::pos2(580.0, 780.0);
    assert!(!frame(vec![egui::Event::PointerMoved(outside), button(outside, true)]).1, "the press ends the preview");
    assert!(!frame(vec![button(outside, false)]).1);
}

/// A document with point type in Inter, selected; its id.
fn selected_inter(app: &mut VectorcraftApp) -> u64 {
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    // These tests exercise font picking, including system fonts without Latin glyphs. Missing
    // glyph protection would intentionally keep the sample text in its previous font.
    app.run("prefs.set", json!({"key": "missingGlyphProtection", "value": false})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    if !crate::panel_docking::showing(&app.ui, "character") {
        app.run("window.panel", json!({"panel": "character"})).unwrap();
    }
    id
}

fn first_style(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::CharStyle {
    match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.first_style(),
        _ => panic!("not type"),
    }
}

fn wheel(unit: egui::MouseWheelUnit, dy: f32, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::MouseWheel { unit, delta: egui::vec2(0.0, dy), modifiers, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Unknown }
}

/// One frame of `draw` over `app`, as the shell runs it; the middle of the text naming `family`
/// lowest on the screen (the closed family field), if drawn, and whether the menu's search field
/// is drawn (the menu is open).
fn panel_frame(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    events: Vec<egui::Event>,
    family: &str,
    draw: &dyn Fn(&mut VectorcraftApp, &mut egui::Ui),
) -> (Option<egui::Pos2>, bool) {
    panel_frame_at(ctx, app, None, events, family, draw)
}

/// [`panel_frame`] with the input's clock set to `time` seconds.
fn panel_frame_at(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    time: Option<f64>,
    events: Vec<egui::Event>,
    family: &str,
    draw: &dyn Fn(&mut VectorcraftApp, &mut egui::Ui),
) -> (Option<egui::Pos2>, bool) {
    let mut input = egui::RawInput {
        time,
        events,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))),
        ..Default::default()
    };
    app.raw_input_hook(ctx, &mut input);
    let mut out = ctx.run_ui(input, |ui| {
        crate::font_menu::end_stale_preview(app, ui.ctx());
        draw(app, ui);
    });
    out.textures_delta.clear();
    let text = |label: &str| {
        out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + egui::vec2(4.0, t.galley.size().y * 0.5)),
            _ => None,
        })
    };
    (text(family), text("Search").is_some())
}

/// #5253: in the Character panel, each notch of the mouse wheel over the closed font family field
/// applies the next family down the menu (up it when turned up) to the selected type, each its own
/// undo step, with the menu closed. Leaving the field or Escape keeps it; Undo puts it back.
#[test]
fn the_wheel_over_the_family_field_steps_the_selected_type_through_the_families() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let font = |app: &VectorcraftApp| first_style(app, id).font_family;
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::panels::character::show(app, ui);
    let mut field = None;
    for _ in 0..3 {
        field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
    }
    let field = field.expect("the Character panel shows the family field");
    // Other tests add fonts to the process-wide database meanwhile: compare places, not neighbours.
    let place = |f: &str| FontDb::global().menu_family_list().iter().position(|x| x == f);
    let before = steps(&app);
    let line = egui::MouseWheelUnit::Line;
    let (_, open) = panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(field), wheel(line, -1.0, Default::default())], "Inter", &draw);
    let next = font(&app);
    assert!(place(&next) > place("Inter") && steps(&app) == before + 1, "a notch down applies a family below, one step: {next}");
    assert!(!open && !app.session.in_interaction(), "the menu stays closed and nothing is left previewed");
    // The family is now a recent one: the order stays as it was while the pointer stays on the field.
    panel_frame(&ctx, &mut app, vec![wheel(line, -1.0, Default::default())], &next, &draw);
    let after = font(&app);
    assert!(place(&after) > place(&next) && steps(&app) == before + 2, "{after}");
    panel_frame(&ctx, &mut app, vec![wheel(line, 1.0, Default::default())], &after, &draw);
    assert_eq!((font(&app), steps(&app)), (next.clone(), before + 3), "a notch up, back");
    // A turn of a fine wheel in points: a notch is `line_scroll_speed` points, the rest carries over.
    let speed = ctx.options(|o| o.input_options.line_scroll_speed);
    panel_frame(&ctx, &mut app, vec![wheel(egui::MouseWheelUnit::Point, speed * 0.5, Default::default())], &next, &draw);
    assert_eq!(font(&app), next, "less than a notch");
    panel_frame(&ctx, &mut app, vec![wheel(egui::MouseWheelUnit::Point, speed * 0.5, Default::default())], &next, &draw);
    assert_eq!((font(&app), steps(&app)), ("Inter".to_string(), before + 4), "the rest of the notch");
    // A turn with a modifier key is not the field's.
    panel_frame(&ctx, &mut app, vec![wheel(line, -1.0, egui::Modifiers::COMMAND)], "Inter", &draw);
    assert_eq!((font(&app), steps(&app)), ("Inter".to_string(), before + 4), "Ctrl/Cmd-wheel");
    // Two notches at once: two families, two steps.
    panel_frame(&ctx, &mut app, vec![wheel(line, -2.0, Default::default())], "Inter", &draw);
    assert_eq!((font(&app), steps(&app)), (after.clone(), before + 6));
    // Escape and leaving the field keep the family.
    panel_frame(&ctx, &mut app, key(egui::Key::Escape), &after, &draw);
    panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(egui::pos2(580.0, 780.0))], &after, &draw);
    panel_frame(&ctx, &mut app, vec![], &after, &draw);
    assert_eq!((font(&app), steps(&app)), (after, before + 6), "the family stays applied");
    // The wheel away from the field does not change the font.
    let shown = font(&app);
    panel_frame(&ctx, &mut app, vec![wheel(line, -1.0, Default::default())], &shown, &draw);
    assert_eq!(steps(&app), before + 6, "the wheel off the field");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(font(&app), next, "Undo takes back one notch");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(font(&app), "Inter");
}

/// Opposite wheel notches in one frame apply in order, each with its own undo step.
#[test]
fn opposite_wheel_notches_in_one_frame_keep_their_undo_steps() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::panels::character::show(app, ui);
    let mut field = None;
    for _ in 0..3 {
        field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
    }
    let field = field.expect("the Character panel shows the family field");
    let before = app.session.active().unwrap().history.undo.len();
    let line = egui::MouseWheelUnit::Line;
    panel_frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(field), wheel(line, -1.0, Default::default()), wheel(line, 1.0, Default::default())],
        "Inter",
        &draw,
    );
    assert_eq!(first_style(&app, id).font_family, "Inter");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 2);
    app.run("edit.undo", json!({})).unwrap();
    assert_ne!(first_style(&app, id).font_family, "Inter");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(first_style(&app, id).font_family, "Inter");
}

/// A turn before opening applies once; the popup keeps the later wheel and cancels to that font.
#[test]
fn a_wheel_before_opening_the_family_popup_keeps_its_committed_change() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::panels::character::show(app, ui);
    let mut field = None;
    for _ in 0..3 {
        field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
    }
    let field = field.unwrap();
    let before = app.session.active().unwrap().history.undo.len();
    let button = |pressed| egui::Event::PointerButton { pos: field, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let turn = || wheel(egui::MouseWheelUnit::Line, -1.0, Default::default());
    panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(field), turn(), button(true), button(false), turn()], "Inter", &draw);
    let applied = first_style(&app, id).font_family;
    assert_ne!(applied, "Inter");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    for _ in 0..3 {
        panel_frame(&ctx, &mut app, vec![], &applied, &draw);
    }
    panel_frame(&ctx, &mut app, key(egui::Key::Escape), &applied, &draw);
    assert_eq!(first_style(&app, id).font_family, applied);
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(first_style(&app, id).font_family, "Inter");
}

/// A later pointer move must not change which field owns an earlier wheel event.
#[test]
fn wheel_events_use_the_pointer_position_when_they_occurred() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::panels::character::show(app, ui);
    let mut field = None;
    for _ in 0..3 {
        field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
    }
    let field = field.expect("the Character panel shows the family field");
    let outside = egui::pos2(580.0, 780.0);
    panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(outside)], "Inter", &draw);
    let before = app.session.active().unwrap().history.undo.len();
    let line = egui::MouseWheelUnit::Line;
    panel_frame(&ctx, &mut app, vec![wheel(line, -1.0, Default::default()), egui::Event::PointerMoved(field)], "Inter", &draw);
    assert_eq!(first_style(&app, id).font_family, "Inter", "the wheel was outside the field");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
    assert!(ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. }))), "outside wheel input is left for its owner");
    let after_gap = |ctx: &egui::Context| Some(ctx.input(|i| i.time) + crate::widgets::WHEEL_STREAM_GAP);
    let later = after_gap(&ctx);
    panel_frame_at(&ctx, &mut app, later, vec![wheel(line, -1.0, Default::default()), egui::Event::PointerMoved(outside)], "Inter", &draw);
    assert_ne!(first_style(&app, id).font_family, "Inter", "the wheel was over the field before the pointer left");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(first_style(&app, id).font_family, "Inter");
    let button = |pos| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    let later = after_gap(&ctx);
    panel_frame_at(&ctx, &mut app, later, vec![button(field), wheel(line, -1.0, Default::default()), button(outside)], "Inter", &draw);
    assert_ne!(first_style(&app, id).font_family, "Inter", "button events also carry the pointer's position");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    let end = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::Vec2::ZERO,
        modifiers: Default::default(),
        phase: egui::TouchPhase::End,
        source: egui::MouseWheelSource::Unknown,
    };
    panel_frame(&ctx, &mut app, vec![end], "Inter", &draw);
    let later = after_gap(&ctx);
    panel_frame_at(
        &ctx,
        &mut app,
        later,
        vec![
            egui::Event::PointerMoved(field),
            wheel(line, -1.0, Default::default()),
            egui::Event::PointerMoved(outside),
            wheel(egui::MouseWheelUnit::Point, -2.0, Default::default()),
        ],
        "Inter",
        &draw,
    );
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    assert_eq!(ctx.input(|i| i.smooth_scroll_delta.y), -2.0, "only the outside wheel contributes to panel scrolling");
    // A fractional turn made before the pointer reached the field does not count toward its notches.
    let speed = ctx.options(|o| o.input_options.line_scroll_speed);
    let later = after_gap(&ctx);
    let early = wheel(egui::MouseWheelUnit::Point, -0.75 * speed, Default::default());
    panel_frame_at(&ctx, &mut app, later, vec![early, egui::Event::PointerMoved(field)], "Inter", &draw);
    let later = after_gap(&ctx);
    panel_frame_at(&ctx, &mut app, later, vec![wheel(line, -0.5, Default::default())], "Inter", &draw);
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1, "half a notch on the field");
    panel_frame(&ctx, &mut app, vec![wheel(line, -0.5, Default::default())], "Inter", &draw);
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 2, "a whole notch on the field");
}

/// A turn queued for one document cannot restyle another document opened before its UI pass.
#[test]
fn a_wheel_queued_for_another_document_is_discarded() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    selected_inter(&mut app);
    let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::panels::character::show(app, ui);
    let mut field = None;
    for _ in 0..3 {
        field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
    }
    let field = field.expect("the Character panel shows the family field");
    let mut raw = egui::RawInput {
        events: vec![egui::Event::PointerMoved(field), wheel(egui::MouseWheelUnit::Line, -1.0, Default::default())],
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))),
        ..Default::default()
    };
    app.raw_input_hook(&ctx, &mut raw);
    assert!(!raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
    let id = selected_inter(&mut app);
    let before = app.session.active().unwrap().history.undo.len();
    let mut out = ctx.run_ui(raw, |ui| draw(&mut app, ui));
    out.textures_delta.clear();
    assert_eq!(first_style(&app, id).font_family, "Inter");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
}

/// Presentation mode hides an Appearance row's Character popup even if its state is still open.
#[test]
fn presentation_mode_leaves_a_transient_font_fields_wheel_available() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    selected_inter(&mut app);
    app.run("window.panel.close", json!({"panel": "character"})).unwrap();
    app.run("window.panel.close", json!({"panel": "properties"})).unwrap();
    app.run("window.panel", json!({"panel": "appearance"})).unwrap();
    assert!(!crate::panel_docking::showing(&app.ui, "character"));
    assert!(!crate::panel_docking::showing(&app.ui, "properties"));
    assert!(crate::panel_docking::showing(&app.ui, "appearance"));
    app.ui.transient_panel = Some("character".into());
    let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::panels::character::show(app, ui);
    let mut field = None;
    for _ in 0..3 {
        field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
    }
    app.ui.screen_mode = 3;
    let mut raw = egui::RawInput {
        events: vec![egui::Event::PointerMoved(field.unwrap()), wheel(egui::MouseWheelUnit::Line, -1.0, Default::default())],
        ..Default::default()
    };
    app.raw_input_hook(&ctx, &mut raw);
    assert!(raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
}

/// #5253: the Properties panel and the Control bar have the same family field, and the wheel steps
/// the families over it too.
#[test]
fn the_wheel_steps_the_families_in_properties_and_the_control_bar() {
    type Draw = fn(&mut VectorcraftApp, &mut egui::Ui);
    let places: [(&str, Draw); 2] = [
        ("Properties", |app, ui| crate::panels::properties::type_sections(app, ui)),
        ("Control bar", |app, ui| crate::panels::character::control_bar(app, ui)),
    ];
    let at = |f: &str| FontDb::global().menu_family_list().iter().position(|x| x == f);
    for (place, draw) in places {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let id = selected_inter(&mut app);
        app.ui.control_bar = true;
        let mut field = None;
        for _ in 0..3 {
            field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
        }
        let field = field.unwrap_or_else(|| panic!("{place} shows the family field"));
        let steps = app.session.active().unwrap().history.undo.len();
        let notch = wheel(egui::MouseWheelUnit::Line, -1.0, Default::default());
        panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(field), notch], "Inter", &draw);
        let next = first_style(&app, id).font_family;
        assert!(at(&next) > at("Inter"), "{place}: {next}");
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "{place}");
    }
}

/// The Character panel in a scroll area, its family field near the bottom of the view.
fn scrolling_character_panel(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        ui.add_space(250.0);
        crate::panels::character::show(app, ui);
        ui.add_space(600.0);
    });
}

/// Scrolling a panel with a still pointer keeps going when the closed family field slides under
/// the pointer, and the font stays as it was.
#[test]
fn scrolling_a_panel_past_the_family_field_under_a_still_pointer_keeps_the_font() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let draw = scrolling_character_panel;
    let mut field = None;
    for _ in 0..3 {
        field = panel_frame(&ctx, &mut app, vec![], "Inter", &draw).0;
    }
    let field = field.expect("the panel shows the family field");
    let still = egui::pos2(field.x, field.y - 120.0);
    panel_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(still)], "Inter", &draw);
    let before = app.session.active().unwrap().history.undo.len();
    let mut passed = false;
    for _ in 0..240 {
        let at = panel_frame(&ctx, &mut app, vec![wheel(egui::MouseWheelUnit::Point, -6.0, Default::default())], "Inter", &draw).0;
        if at.is_some_and(|point| point.y < still.y - 30.0) {
            passed = true;
            break;
        }
    }
    assert!(passed, "the panel scrolled the field past the pointer");
    assert_eq!(first_style(&app, id).font_family, "Inter");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
}

/// A family field that a panel scrolled under a still pointer leaves later wheel input to the
/// panel, including after the stream ends, until the pointer really moves.
#[test]
fn a_family_field_scrolled_under_a_still_pointer_takes_the_wheel_after_the_pointer_moves() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let draw = scrolling_character_panel;
    let mut time = 100.0;
    let mut field = None;
    for _ in 0..3 {
        time += 0.02;
        field = panel_frame_at(&ctx, &mut app, Some(time), vec![], "Inter", &draw).0;
    }
    let field = field.expect("the panel shows the family field");
    let still = egui::pos2(field.x, field.y - 40.0);
    time += 0.02;
    panel_frame_at(&ctx, &mut app, Some(time), vec![egui::Event::PointerMoved(still)], "Inter", &draw);
    let before = app.session.active().unwrap().history.undo.len();
    let mut under = false;
    for _ in 0..80 {
        time += 0.02;
        panel_frame_at(&ctx, &mut app, Some(time), vec![wheel(egui::MouseWheelUnit::Point, -3.0, Default::default())], "Inter", &draw);
        let mut at = None;
        for _ in 0..12 {
            time += 0.02;
            at = panel_frame_at(&ctx, &mut app, Some(time), vec![], "Inter", &draw).0;
        }
        if at.is_some_and(|point| (point.y - still.y).abs() < 4.0) {
            under = true;
            break;
        }
    }
    assert!(under, "the panel scrolled the field under the pointer");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before);
    time += 5.0;
    let notch = || wheel(egui::MouseWheelUnit::Line, -1.0, Default::default());
    panel_frame_at(&ctx, &mut app, Some(time), vec![notch()], "Inter", &draw);
    assert_eq!(first_style(&app, id).font_family, "Inter", "the still pointer did not arm the field");
    let mut at = None;
    for _ in 0..12 {
        time += 0.02;
        at = panel_frame_at(&ctx, &mut app, Some(time), vec![], "Inter", &draw).0;
    }
    let at = at.expect("the field is still in view");
    time += 5.0;
    panel_frame_at(&ctx, &mut app, Some(time), vec![egui::Event::PointerMoved(at), notch()], "Inter", &draw);
    assert_ne!(first_style(&app, id).font_family, "Inter", "the pointer moved onto the field");
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
}

/// #5253: a family the wheel steps to keeps the type's style when it has that style, else takes the
/// closest it has, as a family picked from the menu does. The menus that only pick a font ignore
/// the wheel.
#[test]
fn a_family_stepped_to_keeps_the_style_it_has() {
    let ctx = egui::Context::default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let db = FontDb::global();
    let bold: Vec<String> = db.menu_family_list().iter().filter(|f| db.styles(f).iter().any(|s| s == "Bold")).take(2).cloned().collect();
    let [from, to] = bold.as_slice() else { panic!("two families with a Bold style: {bold:?}") };
    app.run("text.setStyle", json!({"font": from, "style": "Bold"})).unwrap();
    crate::font_menu::apply(&mut app, &ctx, FontPick::Wheel(vec![to.clone()]));
    let s = first_style(&app, id);
    assert_eq!((&s.font_family, s.font_style.as_str()), (to, "Bold"));
    assert_eq!(crate::font_menu::picked(&mut app, Some(FontPick::Wheel(vec![from.clone()]))), None);
    assert_eq!(&first_style(&app, id).font_family, to, "picking menus leave the document alone");
}

#[test]
fn bundled_families_are_classified() {
    use vectorcraft_text::FontClass;
    let db = FontDb::global();
    let class = |f: &str| db.family_traits(f).class;
    assert_eq!(class("JetBrains Mono"), FontClass::Monospaced);
    assert_eq!(class("Source Serif 4"), FontClass::Serif);
    assert_eq!(class("Source Sans 3"), FontClass::Sans);
    assert!(!db.family_traits("Inter").japanese);
}

#[test]
fn right_arrow_lists_a_familys_styles_and_they_preview() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let (mut at, mut picked) = (egui::Pos2::ZERO, None);
        let mut out = ctx.run_ui(input, |ui| {
            at = ui.next_widget_position();
            picked = font_menu(ui, "test-styles", "Source Sans 3", 260.0, None, MenuLook::default());
        });
        out.textures_delta.clear();
        (at, picked)
    };
    let (at, _) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(true)]);
    frame(vec![button(false)]);
    for _ in 0..5 {
        frame(vec![]);
    }
    let styles = FontDb::global().styles("Source Sans 3");
    assert!(styles.len() > 1);
    frame(key(egui::Key::ArrowRight));
    let (_, picked) = frame(key(egui::Key::ArrowDown));
    assert_eq!(picked, Some(FontPick::Preview("Source Sans 3".into(), Some(styles[0].clone()))), "the first style under the family");
    let (_, picked) = frame(key(egui::Key::ArrowDown));
    assert_eq!(picked, Some(FontPick::Preview("Source Sans 3".into(), Some(styles[1].clone()))));
    let (_, picked) = frame(key(egui::Key::Enter));
    assert_eq!(picked, Some(FontPick::Chosen("Source Sans 3".into(), Some(styles[1].clone()))));
}

/// #1116: styles listed above the highlight (a click on another family's triangle) leave it on its
/// family; it doesn't land on one of those styles and preview it.
#[test]
fn styles_listed_above_the_highlight_leave_it_on_its_family() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let (mut at, mut picked) = (egui::Pos2::ZERO, None);
        let mut out = ctx.run_ui(input, |ui| {
            at = ui.next_widget_position();
            picked = font_menu(ui, "test-rebuild", "Source Sans 3", 260.0, None, MenuLook::default());
        });
        out.textures_delta.clear();
        // The family's row: the lowest text naming it (the menu's button above names it too).
        let row = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Source Sans 3" => Some(t.pos + egui::vec2(0.0, t.galley.size().y * 0.5)),
                _ => None,
            })
            .max_by(|a, b| a.y.total_cmp(&b.y));
        (at, picked, row)
    };
    let (at, ..) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(click, true)]);
    frame(vec![button(click, false)]);
    let mut row = None;
    for _ in 0..5 {
        row = frame(vec![]).2;
    }
    // The family's disclosure triangle, left of its name: hovered, it highlights the family.
    let triangle = row.expect("the family is listed") - egui::vec2(7.0, 0.0);
    frame(vec![egui::Event::PointerMoved(triangle)]);
    let (_, picked, _) = frame(key(egui::Key::ArrowDown));
    let Some(FontPick::Preview(next, None)) = picked else { panic!("↓ previews the next family: {picked:?}") };
    // A click on the triangle, the pointer still: the styles are listed between the two families.
    frame(vec![button(triangle, true)]);
    frame(vec![button(triangle, false)]);
    for _ in 0..3 {
        let (_, picked, _) = frame(vec![]);
        assert_eq!(picked, None, "{next} stays highlighted and previewed");
    }
    let (_, picked, _) = frame(key(egui::Key::Enter));
    assert_eq!(picked, Some(FontPick::Chosen(next, None)));
}

#[test]
fn stars_mark_favourites_and_the_preferences_size_the_rows() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::font_menu::apply(&mut app, &ctx, FontPick::Favorite("Inter".into()));
    crate::font_menu::apply(&mut app, &ctx, FontPick::Favorite("Source Serif 4".into()));
    assert_eq!(app.ui.favorite_fonts, ["Inter", "Source Serif 4"]);
    crate::font_menu::apply(&mut app, &ctx, FontPick::Favorite("inter".into()));
    assert_eq!(app.ui.favorite_fonts, ["Source Serif 4"], "a second click takes the star off (any case)");
    let saved: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&app.ui).unwrap()).unwrap();
    assert_eq!(saved.favorite_fonts, ["Source Serif 4"], "favourites are kept with the UI state");
    let look = MenuLook::of(&app);
    assert!(look.samples && look.row == 24.0);
    app.run("prefs.set", json!({"values": {"fontPreviewSize": "large", "fontPreview": false}})).unwrap();
    let look = MenuLook::of(&app);
    assert!(!look.samples && look.row > 24.0, "Font Preview Size and Enable in-menu font previews");
}

/// Find Font's Replace With and the Glyphs panel only pick a font: their menu's highlight never
/// previews on the document, a choice is just returned, and a star is still toggled.
#[test]
fn menus_that_only_pick_a_font_leave_the_document_alone() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let font = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.first_style().font_family,
        _ => String::new(),
    };
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = steps(&app);
    use crate::font_menu::picked;
    assert_eq!(picked(&mut app, None), None);
    assert_eq!(picked(&mut app, Some(FontPick::Preview("Source Serif 4".into(), None))), None);
    assert_eq!(picked(&mut app, Some(FontPick::EndPreview)), None);
    assert_eq!(
        picked(&mut app, Some(FontPick::Chosen("Source Serif 4".into(), Some("Bold".into())))),
        Some(("Source Serif 4".to_string(), Some("Bold".to_string())))
    );
    assert_eq!((font(&app), steps(&app)), ("Inter".to_string(), before), "nothing previewed or applied on the text");
    assert_eq!(picked(&mut app, Some(FontPick::Favorite("Inter".into()))), None);
    assert_eq!(app.ui.favorite_fonts, ["Inter"], "a star is still toggled");
    assert_eq!(picked(&mut app, Some(FontPick::Favorite("inter".into()))), None);
    assert!(app.ui.favorite_fonts.is_empty());
}

/// #555: the menu's search field and filters stay at its top while the wheel scrolls the families,
/// past the list's end too: the list is the menu's one scrolling part.
#[test]
fn the_search_field_stays_put_while_the_list_scrolls() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 500.0))), ..Default::default() };
        let mut at = egui::Pos2::ZERO;
        let mut out = ctx.run_ui(input, |ui| {
            // Low in the window, as in a panel at the bottom of the dock: the menu opens upwards.
            ui.add_space(390.0);
            at = ui.next_widget_position();
            font_menu(ui, "test-scroll", "Inter", 260.0, None, MenuLook::default());
        });
        out.textures_delta.clear();
        let search = out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Search" => Some(t.pos),
            _ => None,
        });
        (at, search)
    };
    let (at, _) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(true)]);
    frame(vec![button(false)]);
    let mut field = None;
    for _ in 0..5 {
        field = frame(vec![]).1;
    }
    let field = field.expect("the search field shows");
    let over_list = egui::pos2(field.x + 40.0, field.y + 120.0);
    for dy in [-400.0, -4000.0, -4000.0] {
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            modifiers: Default::default(),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        };
        frame(vec![egui::Event::PointerMoved(over_list), wheel]);
        for _ in 0..10 {
            assert_eq!(frame(vec![]).1, Some(field), "the field stays where it was");
        }
    }
}

/// A search that matches nothing shrinks the menu to its search field and filters; clearing the
/// search lists every family again, and the menu grows back to its height before the search
/// (egui#5225). The search field stays where it was throughout.
#[test]
fn the_menu_grows_back_when_a_search_that_matched_nothing_is_cleared() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let mut at = egui::Pos2::ZERO;
        let mut out = ctx.run_ui(input, |ui| {
            at = ui.next_widget_position();
            font_menu(ui, "test-regrow", "Inter", 260.0, None, MenuLook { samples: false, ..Default::default() });
        });
        out.textures_delta.clear();
        let text = |label: &str| {
            out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos),
                _ => None,
            })
        };
        // The filter row under the search field: it shows whatever the field holds.
        let filters = text("Any Kind");
        let menu = filters.and_then(|p| ctx.layer_id_at(p)).and_then(|l| ctx.memory(|m| m.area_rect(l.id)));
        (at, filters, text("Search"), text("No matching fonts").is_some(), menu.map(|r| r.height()))
    };
    let settle = |events: Vec<egui::Event>| {
        let mut last = frame(events);
        for _ in 0..5 {
            last = frame(vec![]);
        }
        last
    };
    let (at, ..) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(true)]);
    let (_, filters, search, _, full) = settle(vec![button(false)]);
    let (filters, search, full) = (filters.expect("the menu is open"), search.expect("the search field shows"), full.expect("the menu's area"));
    let nothing = "qqzzxxqqzz";
    let (_, f, _, none, small) = settle(vec![egui::Event::Text(nothing.into())]);
    assert!(none, "the search matches no family");
    assert_eq!(f, Some(filters), "the search field stays put while nothing matches");
    let small = small.expect("the menu stays open");
    assert!(small + MenuLook::default().row < full, "the menu shrinks to its search field: {small} of {full}");
    for _ in nothing.chars() {
        frame(key(egui::Key::Backspace));
    }
    let (_, f, s, none, grown) = settle(vec![]);
    assert!(!none, "every family is listed again");
    assert_eq!((f, s), (Some(filters), Some(search)), "the search field is where it was");
    assert_eq!(grown, Some(full), "the menu grows back to its height before the search");
}

/// #563: a preview stays on the text while the menu stays on its family. The menu's caller passes
/// the text's font, which the preview changed; the menu keeps the font from before the preview as
/// the current one, so it doesn't take the previewed family for it and end the preview, then
/// preview it again, every frame (the text flickered).
#[test]
fn a_preview_stays_put_while_the_menu_is_still() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let font = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.first_style().font_family,
        _ => String::new(),
    };
    let revision = |app: &VectorcraftApp| app.session.active().unwrap().revision;
    // A panel's frame: the menu shows the text's font, and what it picks is applied.
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let current = font(app);
        let (mut at, mut picked) = (egui::Pos2::ZERO, None);
        let mut out = ctx.run_ui(input, |ui| {
            crate::font_menu::end_stale_preview(app, ui.ctx());
            at = ui.next_widget_position();
            picked = font_menu(ui, "test-steady", &current, 260.0, None, MenuLook::default());
            if let Some(p) = picked.clone() {
                crate::font_menu::apply(app, ui.ctx(), p);
            }
        });
        out.textures_delta.clear();
        (at, picked)
    };
    let (at, _) = frame(&mut app, vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(click), button(true)]);
    frame(&mut app, vec![button(false)]);
    for _ in 0..5 {
        frame(&mut app, vec![]);
    }
    let (_, picked) = frame(&mut app, key(egui::Key::ArrowDown));
    assert!(matches!(picked, Some(FontPick::Preview(..))), "{picked:?}");
    let previewed = font(&app);
    assert_ne!(previewed, "Inter");
    let rev = revision(&app);
    for _ in 0..6 {
        let (_, picked) = frame(&mut app, vec![]);
        assert_eq!(picked, None, "nothing more to apply");
    }
    assert_eq!((font(&app), revision(&app)), (previewed, rev), "the preview stays, untouched");
    // Back on the text's own family, the preview ends.
    let (_, picked) = frame(&mut app, key(egui::Key::ArrowUp));
    assert_eq!(picked, Some(FontPick::EndPreview));
    assert_eq!(font(&app), "Inter");
}

/// What an open font menu drew: each text with its rectangle and clip rectangle, the rectangles of
/// the samples, each sample's tint, the filled rectangles with their fill, each text's color, and
/// the menu's rectangle.
struct Drawn {
    texts: Vec<(String, egui::Rect, egui::Rect)>,
    samples: Vec<egui::Rect>,
    tints: Vec<egui::Color32>,
    fills: Vec<(egui::Rect, egui::Color32)>,
    text_colors: Vec<(String, egui::Color32)>,
    menu: egui::Rect,
}

impl Drawn {
    fn text(&self, label: &str) -> egui::Rect {
        self.texts.iter().find(|(t, ..)| t == label).map(|(_, r, _)| *r).unwrap_or_else(|| panic!("{label} is drawn"))
    }

    /// The filters' rows, each by the centers of its first and last chips, in the UI's language.
    fn filter_rows(&self) -> Vec<(f32, f32)> {
        use crate::i18n::{current, t, tr_ctx};
        let variable = tr_ctx(current(), "font-technology", "Variable");
        [(t("All Scripts"), t("Thai")), ("★", t("Rounded")), (t("Script / Brush"), t("Decorative")), (t("Font Technology"), variable)]
            .map(|(first, last)| (self.text(first).center().y, self.text(last).center().y))
            .to_vec()
    }

    /// The filters lie on their four rows, none wrapped.
    fn assert_filter_rows(&self) {
        let rows = self.filter_rows();
        for (i, (first, last)) in rows.iter().enumerate() {
            assert!((first - last).abs() < 1.0, "filter row {i} wraps: its first chip at {first}, its last at {last}");
        }
        assert!(rows.windows(2).all(|w| w[1].0 > w[0].0 + 4.0), "four rows of filters, in order: {rows:?}");
    }
}

/// Opens the font menu of a field `width` wide whose left edge is `left` points into a window of
/// size `screen`, draws it until it settles and its samples are rendered, and returns what the
/// last frame drew.
fn open_menu(id: &str, left: f32, width: f32, screen: egui::Vec2) -> Drawn {
    open_menu_with(id, left, width, screen, MenuLook::default(), None)
}

/// [`open_menu`] with the menu's look `look`, in the interface colors `tokens` when given.
fn open_menu_with(id: &str, left: f32, width: f32, screen: egui::Vec2, look: MenuLook, tokens: Option<crate::theme::Tokens>) -> Drawn {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    if let Some(t) = tokens {
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, t));
    }
    let field = egui::Rect::from_min_size(egui::pos2(left, 10.0), egui::vec2(width, 30.0));
    let frame = |events: Vec<egui::Event>| {
        let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                font_menu(ui, id, "Inter", width, None, look);
            });
        });
        out.textures_delta.clear();
        out.shapes
    };
    let click = field.min + egui::vec2(10.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![]);
    frame(vec![egui::Event::PointerMoved(click), button(true)]);
    frame(vec![button(false)]);
    // With interface colors given, enough frames for the menu to finish fading in, so the
    // samples' tints are whole.
    for _ in 0..if tokens.is_some() { 15 } else { 5 } {
        frame(vec![]);
    }
    // A frame starts only a few samples, and none while several are rendering: draw until a frame
    // leaves none rendering, so that frame drew every shown row's sample.
    let mut shapes = frame(vec![]);
    let mut frames = 0;
    while crate::font_menu::wait_for_samples(&ctx) {
        frames += 1;
        assert!(frames < 1000, "the menu's samples are still rendering after {frames} frames");
        shapes = frame(vec![]);
    }
    let mut texts = vec![];
    let mut samples = vec![];
    let mut tints = vec![];
    let mut fills = vec![];
    let mut text_colors = vec![];
    for s in &shapes {
        match &s.shape {
            egui::Shape::Text(t) => {
                texts.push((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()), s.clip_rect));
                text_colors.push((t.galley.text().to_string(), t.fallback_color));
            }
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                samples.push(m.calc_bounds());
                tints.push(m.vertices.first().map_or(egui::Color32::TRANSPARENT, |v| v.color));
            }
            egui::Shape::Rect(r) => fills.push((r.rect, r.fill)),
            _ => {}
        }
    }
    let mut drawn = Drawn { texts, samples, tints, fills, text_colors, menu: egui::Rect::NOTHING };
    let filter = drawn.text(crate::i18n::t("Any Kind")).center();
    drawn.menu = ctx.layer_id_at(filter).and_then(|l| ctx.memory(|m| m.area_rect(l.id))).expect("the menu is open");
    drawn
}

/// The field is drawn while other threads ask the context for repaints, as the threads that
/// render the samples do when they finish. A read of the context made while the menu already
/// holds one waits behind such a request, and a debug build of egui panics after ten seconds.
#[test]
fn the_field_draws_while_other_threads_request_repaints() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Stops the threads when the test ends, failed or not.
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let stop = Stop(Arc::default());
    let (started, running) = std::sync::mpsc::channel();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (ctx, stop, started) = (ctx.clone(), stop.0.clone(), started.clone());
            std::thread::spawn(move || {
                let _ = started.send(());
                while !stop.load(Ordering::Relaxed) {
                    ctx.request_repaint();
                }
            })
        })
        .collect();
    for _ in &threads {
        running.recv().expect("the thread started");
    }
    // The wheel on: the field also looks up its queued wheel input.
    let look = MenuLook { wheel: true, ..Default::default() };
    for _ in 0..2000 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            font_menu(ui, "test-repaints", "Inter", 200.0, None, look);
        });
        out.textures_delta.clear();
    }
    drop(stop);
    for thread in threads {
        thread.join().expect("the thread ended");
    }
}

/// A narrow field, such as the Control bar's, opens a menu wide enough for the filters on their
/// rows and for the families' names, each drawn whole beside its sample.
#[test]
fn a_narrow_field_opens_a_menu_wide_enough_for_the_filters_and_names() {
    let field = 115.0;
    let d = open_menu("test-narrow", 20.0, field, egui::vec2(1200.0, 800.0));
    d.assert_filter_rows();
    assert!(d.menu.width() > 2.0 * field, "the menu is {} wide", d.menu.width());
    let families = FontDb::global().menu_family_list();
    // The field shows the current family too: only the names in the menu's rows.
    let names: Vec<_> = d.texts.iter().filter(|(t, r, _)| r.top() > d.text("Variable").bottom() && families.iter().any(|f| f == t)).collect();
    assert!(names.iter().any(|(_, r, _)| r.width() > field / 2.0), "a name wider than half the field is listed: {names:?}");
    // The menu widens itself for names up to its limit; a longer name is cut.
    for (name, r, clip) in names.iter().filter(|(_, r, _)| r.width() <= crate::font_menu::NAME_MAX) {
        assert!(r.right() <= clip.right() + 0.5, "{name} ends at {} and is cut at {}", r.right(), clip.right());
    }
}

/// In every UI language the filters keep their four rows unwrapped when the window has room for
/// them: the longest translated labels set the menu's width.
#[test]
fn the_filter_rows_stay_whole_in_every_language() {
    for lang in crate::i18n::Lang::all() {
        crate::i18n::set_current(lang);
        let d = open_menu(&format!("test-lang-{}", lang.code()), 20.0, 115.0, egui::vec2(2400.0, 900.0));
        let rows = d.filter_rows();
        for (i, (first, last)) in rows.iter().enumerate() {
            assert!((first - last).abs() < 1.0, "{}: filter row {i} wraps in a menu {} wide", lang.code(), d.menu.width());
        }
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// A menu opened near the window's right edge is wider than its field and stays in the window.
#[test]
fn a_menu_wider_than_its_field_stays_in_the_window() {
    let screen = egui::vec2(700.0, 800.0);
    let d = open_menu("test-edge", 600.0, 90.0, screen);
    d.assert_filter_rows();
    assert!(d.menu.width() > 200.0, "the menu is {} wide", d.menu.width());
    assert!(d.menu.left() >= -0.5 && d.menu.right() <= screen.x + 0.5, "the menu at {:?} leaves the window", d.menu);
}

/// A docked panel's field is as wide as the panel: its menu takes that width, with the filters on
/// the same rows as a narrow field's menu.
#[test]
fn a_wide_fields_menu_takes_its_width_with_the_same_filter_rows() {
    let field = 480.0;
    let d = open_menu("test-docked", 20.0, field, egui::vec2(1200.0, 800.0));
    d.assert_filter_rows();
    // The field is drawn 4 points narrower than its width; the menu's frame adds its margins.
    assert!(d.menu.width() >= field - 4.0 && d.menu.width() < field + 40.0, "the menu is {} wide", d.menu.width());
}

/// Each sample takes its row's whole height.
#[test]
fn samples_fill_their_rows_height() {
    let d = open_menu("test-samples", 20.0, 260.0, egui::vec2(1200.0, 800.0));
    assert!(!d.samples.is_empty(), "samples are drawn");
    let row = MenuLook::default().row;
    for s in &d.samples {
        assert!((s.height() - row).abs() < 0.5, "a sample {} high in a row {row} high", s.height());
    }
}

/// Show samples on white: in every brightness each sample is drawn in the Light brightness's
/// text color on a white area, with the row's highlight above and below it, while the families'
/// names keep the interface's text color. Off
/// (the default), the samples take the interface's text color with nothing white behind them.
#[test]
fn show_samples_on_white_draws_dark_type_on_white_in_every_brightness() {
    use crate::theme::{Brightness, Tokens};
    let dark = Tokens::for_brightness(Brightness::Light).text;
    let families = FontDb::global().menu_family_list();
    for b in Brightness::ALL {
        let t = Tokens::for_brightness(b);
        for on_white in [false, true] {
            let look = MenuLook { on_white, ..Default::default() };
            let d = open_menu_with(&format!("test-on-white-{}-{on_white}", b.id()), 20.0, 260.0, egui::vec2(1200.0, 800.0), look, Some(t));
            assert!(!d.samples.is_empty(), "{}: samples are drawn", b.id());
            for (sample, tint) in d.samples.iter().zip(&d.tints) {
                let on_paper = d.fills.iter().any(|(r, fill)| *fill == egui::Color32::WHITE && r.expand(0.5).contains_rect(*sample));
                if on_white {
                    assert_eq!(*tint, dark, "{}: a sample on white is dark", b.id());
                    assert!(on_paper, "{}: the sample at {sample:?} is on white", b.id());
                } else {
                    assert_eq!(*tint, t.text, "{}: a sample takes the interface's text color", b.id());
                    assert!(!on_paper, "{}: nothing white behind the sample at {sample:?}", b.id());
                }
            }
            // The current family's row opens highlighted; on white, the highlight shows above and
            // below its paper.
            let lit = d.fills.iter().find(|(r, fill)| *fill == t.selection && d.menu.contains_rect(*r)).map(|(r, _)| *r);
            let lit = lit.unwrap_or_else(|| panic!("{}: a row is highlighted", b.id()));
            let paper = d.fills.iter().find(|(r, fill)| *fill == egui::Color32::WHITE && lit.contains_rect(*r)).map(|(r, _)| *r);
            if on_white {
                let paper = paper.unwrap_or_else(|| panic!("{}: the highlighted row has paper", b.id()));
                let inset = crate::font_menu::PAPER_INSET;
                assert!(
                    (paper.top() - lit.top() - inset).abs() < 0.01 && (lit.bottom() - paper.bottom() - inset).abs() < 0.01,
                    "{}: {paper:?} in {lit:?}",
                    b.id()
                );
            } else {
                assert_eq!(paper, None, "{}: no paper in the highlighted row", b.id());
            }
            // The rows' names, not the closed field's.
            let names: Vec<_> = d
                .text_colors
                .iter()
                .zip(&d.texts)
                .filter(|((name, _), (_, r, _))| d.menu.contains_rect(*r) && families.iter().any(|f| f == name))
                .map(|(named, _)| named)
                .collect();
            assert!(!names.is_empty(), "{}: families are listed", b.id());
            assert!(names.iter().all(|(_, c)| *c == t.text), "{}: the names keep the interface's text color: {names:?}", b.id());
        }
    }
}

/// Show samples on white is a Type preference, off by default, saved with the other preferences
/// and passed to the font menus.
#[test]
fn show_samples_on_white_is_a_saved_preference_the_menus_follow() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    assert!(!MenuLook::of(&app).on_white, "off by default");
    assert!(app.run("prefs.set", json!({"key": "fontSamplesOnWhite", "value": 1})).is_err(), "a value that is not a boolean is refused");
    assert!(!app.session.prefs.font_samples_on_white);
    app.run("prefs.set", json!({"key": "fontSamplesOnWhite", "value": true})).unwrap();
    assert!(MenuLook::of(&app).on_white);
    let saved: vectorcraft_engine::Prefs = serde_json::from_str(&serde_json::to_string(&app.session.prefs).unwrap()).unwrap();
    assert!(saved.font_samples_on_white, "saved and read back");
    let older: vectorcraft_engine::Prefs = serde_json::from_str("{}").unwrap();
    assert!(!older.font_samples_on_white, "preferences saved before it read as off");
}
