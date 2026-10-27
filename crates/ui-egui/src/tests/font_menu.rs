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
