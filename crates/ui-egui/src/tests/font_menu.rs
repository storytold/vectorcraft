//! The font picker's glue: previews, picks, recents and the saved preferences.

use craft::behavior::{EditEvent, GestureId, Mixed, Preview};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_text::FontDb;

use crate::VectorcraftApp;
use crate::font_menu::{apply_family, apply_preview, end_previews, family_pick, picker_settings, sample_text};

/// The owner every test preview is made by.
fn owner() -> egui::Id {
    egui::Id::unique("font-test")
}

/// A pick of `after` over `before`, as the picker raises it.
fn set(before: &str, after: &str) -> EditEvent<String> {
    EditEvent::set(Mixed::Same(before.to_string()), after.to_string(), "Font Family".into(), None)
}

/// The family of the first run of the type `id`.
fn font_of(app: &VectorcraftApp, id: u64) -> String {
    first_style(app, id).font_family
}

/// Recently used means applied, including an edited character range, never just hovered.
#[test]
fn recent_fonts_follow_committed_choices_and_the_count_preference() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Recent fonts", "font": "Inter"})).unwrap()["id"].clone();
    apply_preview(&mut app, owner(), Preview::Shows("Source Serif 4".into()));
    assert!(app.recent_fonts().is_empty());
    apply_preview(&mut app, owner(), Preview::Ended);
    assert!(app.recent_fonts().is_empty());
    apply_family(&mut app, set("Inter", "Source Serif 4"), None);
    app.run("text.setStyle", json!({"font": "Inter"})).unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 6, "font": "Source Serif 4"})).unwrap();
    assert_eq!(app.recent_fonts(), ["Source Serif 4", "Inter"]);
    assert_eq!(picker_settings(&app).recent, ["Source Serif 4", "Inter"]);
    // A failed command must leave history alone.
    assert!(app.run("text.setRangeStyle", json!({"id": 999999, "font": "Missing"})).is_err());
    assert_eq!(app.recent_fonts(), ["Source Serif 4", "Inter"]);
    app.run("prefs.set", json!({"values": {"recentFontsCount": 1}})).unwrap();
    assert_eq!(picker_settings(&app).recent, ["Source Serif 4"]);
    let restored: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&app.ui).unwrap()).unwrap();
    assert_eq!(restored.recent_fonts, ["Source Serif 4", "Inter"]);
}

/// Text the face cannot draw is shown as the generic sample, not as glyphs from another font.
#[test]
fn samples_show_the_text_in_the_family_or_a_generic_sample() {
    // PLAN: assumed `craft::ui::testing::Fixture` (`new(|ui| ..)`, `settle()`, `texts() -> Vec<String>`) and
    // `craft::ui::runtime::Env::of(ui)`; `ContentPainterCx::new(painter, env, ink)` is public (content.rs:43).
    use craft::ui::primitives::ContentPainterCx;
    use craft::ui::runtime::env::Env;
    use craft::ui::style::SurfaceKind;
    use craft::ui::testing::Fixture;
    let mut fixture = Fixture::new(|ui| {
        let env = Env::of(ui);
        let cx = ContentPainterCx::new(ui.painter().clone(), &env, SurfaceKind::Panel.ink());
        let at = ui.max_rect();
        let (top, bottom) = (
            egui::Rect::from_min_size(at.min, egui::vec2(160.0, 24.0)),
            egui::Rect::from_min_size(at.min + egui::vec2(0.0, 40.0), egui::vec2(160.0, 24.0)),
        );
        // Japanese text in a Latin font, and the generic sample itself.
        crate::font_menu::paint_family_sample(&cx, top, "Source Serif 4", "Regular", Some("雅楽"), false);
        crate::font_menu::paint_family_sample(&cx, bottom, "Source Serif 4", "Regular", None, false);
    });
    fixture.settle();
    let texts = fixture.texts();
    assert!(!texts.iter().any(|t| t.text == "雅楽"), "no glyphs from another font: {texts:?}");
    assert_eq!(texts.iter().filter(|t| t.text == "Sample").count(), 2, "both runs are the generic sample: {texts:?}");
}

#[test]
fn preview_changes_the_text_live_and_only_a_choice_is_a_step() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    assert_eq!(sample_text(&app).as_deref(), Some("Gagaku"));
    let before = steps(&app);
    apply_preview(&mut app, owner(), Preview::Shows("Source Serif 4".into()));
    assert_eq!(font_of(&app, id), "Source Serif 4", "previewed on the text");
    apply_preview(&mut app, owner(), Preview::Shows("Source Sans 3".into()));
    assert_eq!(font_of(&app, id), "Source Sans 3");
    apply_preview(&mut app, owner(), Preview::Ended);
    assert_eq!((font_of(&app, id), steps(&app)), ("Inter".to_string(), before), "the preview leaves nothing behind");
    apply_preview(&mut app, owner(), Preview::Shows("Source Serif 4".into()));
    apply_family(&mut app, set("Inter", "Source Serif 4"), None);
    assert_eq!((font_of(&app, id), steps(&app)), ("Source Serif 4".to_string(), before + 1), "one step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(font_of(&app, id), "Inter");
}

/// A preview whose picker isn't shown any more (the text was deselected, the panel closed) is undone.
#[test]
fn a_preview_left_open_by_a_menu_that_is_gone_is_undone() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let steps = app.session.active().unwrap().history.undo.len();
    apply_preview(&mut app, owner(), Preview::Shows("Source Serif 4".into()));
    assert!(app.session.in_interaction());
    let previewer = app.font_preview_owner.expect("the app records who previews");
    end_previews(&mut app, &[previewer]);
    assert!(!app.session.in_interaction(), "the picker is gone: the preview is undone");
    assert_eq!((font_of(&app, id).as_str(), app.session.active().unwrap().history.undo.len()), ("Inter", steps));
    // With nothing previewed it does nothing.
    end_previews(&mut app, &[previewer]);
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
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

/// #5253: a wheel run over the closed family field steps the selected type one family per notch down the
/// menu's order, and the whole run is one undo step.
#[test]
fn the_wheel_over_the_family_field_steps_the_selected_type_through_the_families() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    // Other tests add fonts to the process-wide database meanwhile: the two families below Inter as listed now.
    let list = FontDb::global().menu_family_list();
    let at = list.iter().position(|f| f == "Inter").expect("Inter is listed");
    let [one, two]: [String; 2] = list.iter().skip(at + 1).take(2).cloned().collect::<Vec<_>>().try_into().expect("two families below Inter");
    let before = steps(&app);
    let gesture = GestureId::new(1);
    apply_family(&mut app, EditEvent::begin(gesture, Mixed::Same("Inter".into()), "Font Family".into(), None), None);
    apply_family(&mut app, EditEvent::update(gesture, one.clone()), None);
    assert_eq!(font_of(&app, id), one, "a notch applies the next family at once");
    assert!(app.session.in_interaction(), "the run is still open");
    apply_family(&mut app, EditEvent::update(gesture, two.clone()), None);
    assert_eq!(font_of(&app, id), two);
    apply_family(&mut app, EditEvent::commit(gesture, Mixed::Same("Inter".into()), two.clone()), None);
    assert!(!app.session.in_interaction());
    assert_eq!((font_of(&app, id), steps(&app)), (two, before + 1), "one undo step for the run");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(font_of(&app, id), "Inter");
}

/// #5253: a family stepped to keeps the type's style when it has that style, else takes the closest it has.
#[test]
fn a_family_stepped_to_keeps_the_style_it_has() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let id = selected_inter(&mut app);
    let db = FontDb::global();
    let bold: Vec<String> = db.menu_family_list().iter().filter(|f| db.styles(f).iter().any(|s| s == "Bold")).take(2).cloned().collect();
    let [from, to] = bold.as_slice() else { panic!("two families with a Bold style: {bold:?}") };
    app.run("text.setStyle", json!({"font": from, "style": "Bold"})).unwrap();
    apply_family(&mut app, set(from, to), None);
    let s = first_style(&app, id);
    assert_eq!((&s.font_family, s.font_style.as_str()), (to, "Bold"));
}

/// The pick `pick_only` saw, kept for the test to read.
fn picked_id() -> egui::Id {
    egui::Id::unique("font-menu-test-pick")
}

/// A picker that only picks, as Find Font's Replace With and the Glyphs panel use it.
fn pick_only(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    if let Some(pick) = family_pick(app, ui, "Font", "Inter") {
        ui.ctx().data_mut(|d| d.insert_temp(picked_id(), pick));
    }
}

/// Find Font's Replace With and the Glyphs panel only pick a font: a choice is just returned, the document and its
/// history stay as they are, and a star is still toggled.
#[test]
fn menus_that_only_pick_a_font_leave_the_document_alone() {
    use super::panel_harness::Panel;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Gagaku", "font": "Inter"})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = steps(&app);
    let mut panel = Panel::new(app, pick_only);
    panel.type_into("Font", "Source Serif 4");
    let pick = panel.ctx().data_mut(|d| d.remove_temp::<(String, Option<String>)>(picked_id()));
    assert_eq!(pick.map(|(family, _)| family).as_deref(), Some("Source Serif 4"), "the pick comes back");
    {
        let app = panel.app();
        assert_eq!((font_of(&app, id), steps(&app)), ("Inter".to_string(), before), "nothing previewed or applied on the text");
    }
    assert!(panel.app().ui.favorite_fonts.is_empty());
    panel.click("Font");
    panel.click("Favorite Source Serif 4");
    assert_eq!(panel.app().ui.favorite_fonts, ["Source Serif 4"], "a star is still toggled");
    assert_eq!(steps(&panel.app()), before);
}

/// Show samples on white is a Type preference, off by default, saved with the other preferences
/// and passed to the font menus.
#[test]
fn show_samples_on_white_is_a_saved_preference_the_menus_follow() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    assert!(!picker_settings(&app).samples_on_white, "off by default");
    assert!(app.run("prefs.set", json!({"key": "fontSamplesOnWhite", "value": 1})).is_err(), "a value that is not a boolean is refused");
    assert!(!app.session.prefs.font_samples_on_white);
    app.run("prefs.set", json!({"key": "fontSamplesOnWhite", "value": true})).unwrap();
    assert!(picker_settings(&app).samples_on_white);
    let saved: vectorcraft_engine::Prefs = serde_json::from_str(&serde_json::to_string(&app.session.prefs).unwrap()).unwrap();
    assert!(saved.font_samples_on_white, "saved and read back");
    let older: vectorcraft_engine::Prefs = serde_json::from_str("{}").unwrap();
    assert!(!older.font_samples_on_white, "preferences saved before it read as off");
}
