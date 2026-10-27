use super::*;
use crate::Services;
use vectorcraft_engine::Session;

#[test]
fn geometric_snapping_is_a_saved_smart_guides_preference() {
    let mut a = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    open(&mut a, Some("Smart Guides"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Geometric Guides") && text.contains("Snap to Perpendicular") && text.contains("Snap to Tangent"), "{text}");
    a.ui.dialog.as_mut().unwrap().fields.insert("snapPerpendicular".into(), json!(false));
    confirm(&mut a).unwrap();
    assert!(!a.session.prefs.snap_perpendicular && a.session.prefs.snap_tangent);
    let saved: Prefs = serde_json::from_value(a.ui.engine_prefs.clone()).unwrap();
    assert!(!saved.snap_perpendicular && saved.snap_tangent);
    assert!(a.run("view.snapping", json!({})).is_err());
}

/// A resting checkbox must have a visible outline in both states and every brightness.
#[test]
fn preference_checkboxes_draw_outlines_without_hover() {
    for brightness in Brightness::ALL {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        theme::apply(&ctx, brightness);
        for checked in [false, true] {
            let mut d = Dialog::new("preferences", json!({"showToolTips": checked}));
            let raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| bool_row(ui, &mut d, "showToolTips", "Show Tool Tips"));
            out.textures_delta.clear();
            let outlined = out.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Rect(r) => r.stroke.width > 0.0 && r.rect.width() <= 24.0 && r.rect.height() <= 24.0,
                _ => false,
            });
            assert!(outlined, "{brightness:?}, checked={checked}: no checkbox outline");
        }
    }
}

/// User Interface › Canvas Color (#663): White and the three greys paint the canvas around the
/// artboards; Match User Interface Brightness keeps the theme's.
#[test]
fn canvas_color_choices_paint_the_pasteboard() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    apply_runtime(&mut app, &ctx);
    let themed = Tokens::get(&ctx).pasteboard;
    for (key, want) in [
        ("white", egui::Color32::WHITE),
        ("lightGray", egui::Color32::from_gray(0xc8)),
        ("mediumGray", egui::Color32::from_gray(0x96)),
        ("darkGray", egui::Color32::from_gray(0x5a)),
        ("matchUi", themed),
    ] {
        app.run("prefs.set", serde_json::json!({"key": "canvasColor", "value": key})).unwrap();
        apply_runtime(&mut app, &ctx);
        assert_eq!(Tokens::get(&ctx).pasteboard, want, "{key}");
    }
    assert!(app.run("prefs.set", serde_json::json!({"key": "canvasColor", "value": "pink"})).is_err());
}

fn app() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Services::default())
}

#[test]
fn dialog_ok_applies_prefs_and_ui_fields() {
    let mut a = app();
    open(&mut a, Some("general"));
    let d = a.ui.dialog.as_mut().unwrap();
    assert_eq!(d.str("__category"), "General");
    d.fields.insert("keyboardIncrement".into(), json!(4));
    d.fields.insert("uiBrightness".into(), json!("light"));
    d.fields.insert("__smartGuides".into(), json!(false));
    confirm(&mut a).unwrap();
    assert!(a.ui.dialog.is_none());
    assert_eq!(a.session.prefs.keyboard_increment, 4.0);
    assert_eq!(a.session.prefs.ui_brightness, "light");
    assert!(!a.ui.view.smart_guides);
    assert_eq!(a.ui.engine_prefs["keyboardIncrement"], json!(4.0));
}

/// General › Show Tool Tips: off, a button held under the pointer shows no tool tip; back on,
/// it does again (#394).
#[test]
fn show_tool_tips_off_hides_tool_tips() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut a = app();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    // Come from elsewhere and hover the button for two seconds: did its tool tip show?
    let mut hover = |a: &mut VectorcraftApp| {
        let mut shown = vec![];
        for i in 0..20 {
            time += 0.1;
            let events = match i {
                0 => vec![egui::Event::PointerGone],
                1 => vec![egui::Event::PointerMoved(egui::pos2(20.0, 12.0))],
                _ => vec![],
            };
            let raw = egui::RawInput {
                events,
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                apply_runtime(a, ui.ctx());
                ui.button("Button").on_hover_text("The tool tip");
            });
            out.textures_delta.clear();
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
        }
        shown.iter().any(|t| t == "The tool tip")
    };
    assert!(hover(&mut a), "on by default");
    a.run("prefs.set", json!({"key": "showToolTips", "value": false})).unwrap();
    assert!(!hover(&mut a), "off: no tool tip");
    a.run("prefs.set", json!({"key": "showToolTips", "value": true})).unwrap();
    assert!(hover(&mut a), "on again");
}

#[test]
fn dialog_ok_with_invalid_value_keeps_dialog_and_prefs() {
    let mut a = app();
    open(&mut a, None);
    a.ui.dialog.as_mut().unwrap().fields.insert("anchorSize".into(), json!(42));
    assert!(confirm(&mut a).is_err());
    assert!(a.ui.dialog.is_some());
    assert_eq!(a.session.prefs, Prefs::default());
}

#[test]
fn prefs_persist_through_ui_state() {
    let mut a = app();
    a.run("prefs.set", json!({"key": "gridlineEvery", "value": 50})).unwrap();
    snapshot(&mut a);
    let saved = serde_json::to_vec(&a.ui).unwrap();
    let mut b = app();
    b.ui = serde_json::from_slice::<crate::UiState>(&saved).unwrap().sanitized();
    restore(&mut b);
    assert_eq!(b.session.prefs.gridline_every, 50.0);
}

#[test]
fn old_pref_files_keep_their_brightness() {
    let mut a = app();
    a.ui.brightness = Brightness::Light;
    a.ui.engine_prefs = Value::Null;
    restore(&mut a);
    assert_eq!(a.session.prefs.ui_brightness, "light");
}

/// Performance › Graphics Processor (#306, #502): shown with its restart note, applied by OK
/// and saved with the UI state under the key the desktop app reads before the window opens.
#[test]
fn graphics_processor_preference_shows_and_persists() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut a = app();
    open(&mut a, Some("Performance"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // The window sizes itself on the first frame and draws on the second.
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    assert!(shown.iter().any(|t| t.starts_with("Graphics Processor")), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Automatic"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Applies the next time VectorCraft starts."), "{shown:?}");
    a.ui.dialog.as_mut().unwrap().fields.insert("gpuPreference".into(), json!("highPerformance"));
    assert!(!a.take_prefs_changed(), "nothing to write before OK");
    confirm(&mut a).unwrap();
    assert!(a.take_prefs_changed(), "OK asks the host to write the preferences file");
    assert!(!a.take_prefs_changed());
    assert_eq!(a.session.prefs.gpu_preference, "highPerformance");
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&a.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["gpuPreference"], json!("highPerformance"));
}

/// User Interface › System Title Bar: shown with its next-launch note, applied by OK and
/// saved with the UI state under the key the desktop app reads before the window opens.
#[test]
fn system_title_bar_preference_shows_and_persists() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut a = app();
    open(&mut a, Some("User Interface"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // The window sizes itself on the first frame and draws on the second.
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    let supported = cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos")));
    assert_eq!(shown.iter().any(|t| t == "System Title Bar"), supported, "{shown:?}");
    assert_eq!(shown.iter().any(|t| t == "Applies the next time VectorCraft starts."), supported, "{shown:?}");
    a.ui.dialog.as_mut().unwrap().fields.insert("systemTitleBar".into(), json!(true));
    confirm(&mut a).unwrap();
    assert!(a.session.prefs.system_title_bar);
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&a.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["systemTitleBar"], json!(true));
}

/// Performance › Linux Display Server (#2028): on Linux it shows the drawing-tablet note, and
/// X11 (XWayland) When a Pen Is Attached is applied by OK and saved under the key the desktop
/// app reads before the window opens.
#[test]
fn linux_display_server_offers_the_pen_choice() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut a = app();
    open(&mut a, Some("Performance"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // The window sizes itself on the first frame and draws on the second.
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    let linux = cfg!(target_os = "linux");
    assert_eq!(shown.iter().any(|t| t.starts_with("Linux Display Server")), linux, "{shown:?}");
    let note = "If a drawing tablet's pen doesn't draw, choose an X11 (XWayland) option.";
    assert_eq!(shown.iter().any(|t| t == note), linux, "{shown:?}");
    a.ui.dialog.as_mut().unwrap().fields.insert("linuxDisplayServer".into(), json!("x11WithPen"));
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.linux_display_server, "x11WithPen");
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&a.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["linuxDisplayServer"], json!("x11WithPen"));
}

/// #1866: the category shown is highlighted in the list while the pointer is elsewhere.
#[test]
fn the_category_shown_is_highlighted() {
    fn rects(s: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
        match s {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| rects(s, out)),
            _ => {}
        }
    }
    let mut a = app();
    open(&mut a, Some("Units"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // The window sizes itself on the first frame and draws on the second.
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| rects(&c.shape, &mut shown));
    // The list's rows, top to bottom: only the fourth (Units) is filled.
    let rows: Vec<_> = shown.iter().filter(|(r, _)| r.width() == 184.0 && r.height() < 30.0).collect();
    assert_eq!(rows.len(), PREF_CATEGORIES.len(), "{shown:?}");
    let filled: Vec<usize> = rows.iter().enumerate().filter(|(_, (_, c))| c.a() > 0).map(|(i, _)| i).collect();
    assert_eq!(filled, vec![3], "{rows:?}");
}

/// Units › Numbers Without Units Are Points (#394): the preference reaches the fields when it
/// changes, and its checkbox is dimmed unless a unit is Picas.
#[test]
fn numbers_without_units_are_points_reaches_the_fields_and_dims_without_picas() {
    let mut a = app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |a: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            apply_runtime(a, ui.ctx());
            show(a, ui.ctx());
        });
        out.textures_delta.clear();
    };
    let read = |ctx: &egui::Context| widgets::typed_unit(ctx, vectorcraft_doc::Unit::Picas);
    frame(&mut a);
    assert_eq!(read(&ctx), vectorcraft_doc::Unit::Points, "on by default");
    a.run("prefs.set", json!({"key": "numbersWithoutUnitsArePoints", "value": false})).unwrap();
    frame(&mut a);
    assert_eq!(read(&ctx), vectorcraft_doc::Unit::Picas, "off");
    // The checkbox: dimmed with every unit in points, enabled once a unit is Picas.
    open(&mut a, Some("Units"));
    let d = a.ui.dialog.as_mut().unwrap();
    assert!(!picas_in_use(d), "dimmed in points");
    d.fields.insert("unitsStroke".into(), json!("picas"));
    assert!(picas_in_use(d), "enabled with a unit in picas");
}

/// 0.5.0 saved `powerSaving` for everyone (its default): it reads as Automatic, while the
/// choices this version offers are kept (#502).
#[test]
fn a_graphics_processor_this_version_does_not_offer_reads_as_automatic() {
    for (saved, read) in [("powerSaving", "automatic"), ("turbo", "automatic"), ("lowPower", "lowPower"), ("highPerformance", "highPerformance")] {
        let mut a = app();
        a.ui.engine_prefs = json!({"gpuPreference": saved});
        restore(&mut a);
        assert_eq!(a.session.prefs.gpu_preference, read, "{saved}");
    }
}

#[test]
fn every_category_renders_fields() {
    for c in PREF_CATEGORIES {
        assert!(PREF_SPECS.iter().any(|s| s.category == *c));
    }
}

/// Preferences › Type › Show Font Names in English (#394): OK on the dialog stores the flag the
/// font menus read (Character panel, Type → Font).
#[test]
fn show_font_names_in_english_dialog_ok_stores_the_flag() {
    let mut a = app();
    assert!(a.session.prefs.font_names_in_english);
    open(&mut a, Some("Type"));
    assert_eq!(a.ui.dialog.as_ref().unwrap().str("__category"), "Type");
    a.ui.dialog.as_mut().unwrap().fields.insert("fontNamesInEnglish".into(), json!(false));
    confirm(&mut a).unwrap();
    assert!(a.ui.dialog.is_none());
    assert!(!a.session.prefs.font_names_in_english);
    assert!(!crate::font_menu::MenuLook::of(&a).english_names);
}

/// Preferences › Hyphenation › Exceptions (#394): OK on the dialog pushes the list into the
/// hyphenator the same way `prefs.set` does (no need to drive the live Preferences window).
#[test]
fn hyphenation_exceptions_dialog_ok_reaches_the_hyphenator() {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            vectorcraft_text::set_hyphenation_exceptions("");
        }
    }
    let _clear = Clear;
    let mut a = app();
    open(&mut a, Some("Hyphenation"));
    let d = a.ui.dialog.as_mut().unwrap();
    assert_eq!(d.str("__category"), "Hyphenation");
    d.fields.insert("hyphenationExceptions".into(), json!("typography, hap-pen"));
    confirm(&mut a).unwrap();
    assert!(a.ui.dialog.is_none());
    assert_eq!(a.session.prefs.hyphenation_exceptions, "typography, hap-pen");
    assert_eq!(vectorcraft_text::hyphenation_exceptions(), "typography, hap-pen");
    assert!(vectorcraft_text::hyphen::hyphen_points("typography").is_empty());
    assert_eq!(vectorcraft_text::hyphen::hyphen_points("happen"), vec![3]);
}
