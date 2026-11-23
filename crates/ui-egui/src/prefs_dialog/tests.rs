use super::*;
use crate::Services;
use vectorcraft_engine::Session;

/// The three marquee choices are available in Preferences and saved through the same path as
/// other selection preferences.
#[test]
fn marquee_selection_choices_are_saved_from_preferences() {
    let mut app = app();
    open(&mut app, Some("Selection & Anchor Display"));
    let text = crate::tests_labels::painted_text(&mut app, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Marquee Selection") && text.contains("Touching"), "{text}");
    app.ui.dialog.as_mut().unwrap().fields.insert("marqueeSelection".into(), json!("directional"));
    confirm(&mut app).unwrap();
    assert_eq!(app.session.prefs.marquee_selection, "directional");
    let saved: Prefs = serde_json::from_value(app.ui.engine_prefs.clone()).unwrap();
    assert_eq!(saved.marquee_selection, "directional");
    open(&mut app, Some("Selection & Anchor Display"));
    let text = crate::tests_labels::painted_text(&mut app, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Drag Direction"), "{text}");
}

/// Plug-ins & Scratch Disks › Plug-in Instruction Budget: shown with its choices, applied by OK,
/// saved with the UI state and read back from it when the app starts again.
#[test]
fn plugin_instruction_budget_shows_applies_and_restores() {
    let mut a = app();
    open(&mut a, Some("Plug-ins & Scratch Disks"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Plug-in Instruction Budget") && text.contains("Normal"), "{text}");
    a.ui.dialog.as_mut().unwrap().fields.insert("pluginInstructionBudget".into(), json!("large"));
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.plugin_instruction_budget, vectorcraft_engine::PluginBudget::Large);
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&a.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["pluginInstructionBudget"], json!("large"));
    let mut b = app();
    b.ui.engine_prefs = saved["engine_prefs"].clone();
    restore(&mut b);
    assert_eq!(b.session.prefs.plugin_instruction_budget, vectorcraft_engine::PluginBudget::Large);
    // A saved value this version doesn't offer reads as Normal, and the other preferences stay.
    b.ui.engine_prefs = json!({"keyboardIncrement": 4, "pluginInstructionBudget": 7});
    restore(&mut b);
    assert_eq!((b.session.prefs.plugin_instruction_budget, b.session.prefs.keyboard_increment), (vectorcraft_engine::PluginBudget::Normal, 4.0));
}

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

/// Type › Start Lists From Typed Markers is a checkbox in the Type panel, on by default; turning
/// it off is saved, and every bundled language translates its label.
#[test]
fn start_lists_from_typed_markers_is_a_saved_type_preference() {
    const LABEL: &str = "Start Lists From Typed Markers";
    let mut a = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(a.session.prefs.typed_lists);
    open(&mut a, Some("Type"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains(LABEL), "{text}");
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields.get("typedLists"), Some(&json!(true)));
    a.ui.dialog.as_mut().unwrap().fields.insert("typedLists".into(), json!(false));
    confirm(&mut a).unwrap();
    assert!(!a.session.prefs.typed_lists);
    let saved: Prefs = serde_json::from_value(a.ui.engine_prefs.clone()).unwrap();
    assert!(!saved.typed_lists);
    for lang in crate::i18n::Lang::all().filter(|l| l.code() != crate::i18n::Lang::EN.code()) {
        assert!(crate::i18n::has(lang, LABEL), "{}", lang.code());
    }
}

/// Type › Show Character Alternates is a checkbox in the Type panel, on by default; turning it
/// off is saved, and every bundled language translates its label and the row's two tooltips.
#[test]
fn show_character_alternates_is_a_saved_type_preference() {
    const LABEL: &str = "Show Character Alternates";
    let mut a = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(a.session.prefs.show_character_alternates);
    open(&mut a, Some("Type"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains(LABEL), "{text}");
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields.get("showCharacterAlternates"), Some(&json!(true)));
    a.ui.dialog.as_mut().unwrap().fields.insert("showCharacterAlternates".into(), json!(false));
    confirm(&mut a).unwrap();
    assert!(!a.session.prefs.show_character_alternates);
    let saved: Prefs = serde_json::from_value(a.ui.engine_prefs.clone()).unwrap();
    assert!(!saved.show_character_alternates);
    for lang in crate::i18n::Lang::all().filter(|l| l.code() != crate::i18n::Lang::EN.code()) {
        for source in [LABEL, "Click to replace", "Show all alternates in the Glyphs panel"] {
            assert!(crate::i18n::has(lang, source), "{}: {source}", lang.code());
        }
    }
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

/// #4837: the saved Color Scheme, Accent and Canvas Color are the ones the first frame shows, not
/// the defaults the first UI pass installs.
#[test]
fn saved_scheme_accent_and_canvas_color_show_from_the_first_frame() {
    let mut app = app();
    for (key, value) in [("uiColorScheme", "classic"), ("uiAccent", "pink"), ("canvasColor", "white")] {
        app.run("prefs.set", json!({"key": key, "value": value})).unwrap();
    }
    let ctx = egui::Context::default();
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx()));
        out.textures_delta.clear();
        let (t, classic) = (Tokens::get(&ctx), Tokens::for_scheme(Scheme::Classic, app.ui.brightness, Accent::Pink));
        assert_eq!((t.accent, t.app_bar, t.panel), (classic.accent, classic.app_bar, classic.panel));
        assert_eq!(t.pasteboard, egui::Color32::WHITE);
        assert_eq!(ctx.global_style().visuals.hyperlink_color, classic.accent);
    }
}

/// A Render Threads change made after the preferences were applied, and before the first UI pass
/// applies them again, still restarts the canvas renderer.
#[test]
fn reapply_restarts_the_renderer_after_a_render_threads_change() {
    let mut app = app();
    let ctx = egui::Context::default();
    apply_runtime(&mut app, &ctx);
    app.canvas.worker_started = true;
    // The automatic count as an explicit value: the process-wide count other tests read stays the same.
    // `VECTORCRAFT_RENDER_THREADS` can give more than the 64 the preference accepts.
    let threads = vectorcraft_render::default_threads().min(64);
    app.run("prefs.set", json!({"key": "renderThreads", "value": threads})).unwrap();
    reapply(&mut app, &ctx);
    let restarted = !app.canvas.worker_started;
    app.run("prefs.set", json!({"key": "renderThreads", "value": -1})).unwrap();
    assert!(restarted, "the canvas renderer kept its old thread count");
}

fn app() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Services::default())
}

/// #5941: a page whose settings run past the bottom of the list showed no scroll bar, so a setting
/// lower on the page was easy to miss. Every page that scrolls shows its bar inside the window, and
/// no label is cut off at the side.
#[test]
fn a_page_that_scrolls_shows_its_bar() {
    fn walk(shape: &egui::Shape, clip: egui::Rect, texts: &mut Vec<(String, egui::Rect, egui::Rect)>, bars: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(t) if !t.galley.text().trim().is_empty() => texts.push((t.galley.text().into(), t.visual_bounding_rect(), clip)),
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.width() <= 8.0 && r.rect.height() > 40.0 => bars.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, texts, bars)),
            _ => {}
        }
    }
    let mut scrolled = 0;
    for cat in PREF_CATEGORIES {
        let mut a = app();
        open(&mut a, Some(cat));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let (mut texts, mut bars) = (vec![], vec![]);
        for time in [0.0, 0.25, 0.5, 0.75] {
            let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0)));
            let raw = egui::RawInput { screen_rect, time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(&mut a, ui.ctx()));
            out.textures_delta.clear();
            (texts, bars) = (vec![], vec![]);
            out.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut texts, &mut bars));
        }
        let window = ctx.memory(|m| m.area_rect(egui::Id::unique("dialog-preferences"))).expect("the dialog window is shown");
        // A label below its clip: the page runs past the bottom of the list.
        let scrolls = texts.iter().any(|(_, r, clip)| window.contains_rect(*clip) && r.top() >= clip.bottom());
        if scrolls {
            scrolled += 1;
            assert!(bars.iter().any(|b| window.contains_rect(*b)), "{cat}: the page scrolls and its bar shows in {window:?}: {bars:?}");
        }
        let cut: Vec<_> = texts
            .iter()
            .filter(|(_, r, clip)| clip.y_range().contains(r.center().y) && (r.left() < clip.left() - 1.0 || r.right() > clip.right() + 1.0))
            .filter(|(_, r, _)| window.intersects(*r))
            .map(|(s, r, clip)| format!("{s:?} {r:?} in {clip:?}"))
            .collect();
        assert!(cut.is_empty(), "{cat}: labels cut off at the side: {cut:?}");
    }
    assert!(scrolled > 0, "at least one page runs past the bottom of its list");
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

/// Guides & Grid › Lock Guides by Default: shown in its category, applied by OK, saved with the
/// UI state and read back as locked guides. A file without the key leaves guides unlocked.
#[test]
fn lock_guides_by_default_shows_applies_and_restores() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    open(&mut a, Some("Guides & Grid"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Lock Guides by Default"), "{text}");
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields["lockGuidesByDefault"], json!(false));
    a.ui.dialog.as_mut().unwrap().fields.insert("lockGuidesByDefault".into(), json!(true));
    confirm(&mut a).unwrap();
    assert!(a.session.guides_locked());
    assert_eq!(crate::menus::dynamic_label(&a, "view.guides.lock", ""), "Unlock Guides");
    assert_eq!(a.ui.engine_prefs["lockGuidesByDefault"], json!(true));

    let saved = serde_json::to_vec(&a.ui).unwrap();
    let mut b = app();
    assert!(!b.session.guides_locked());
    b.ui = serde_json::from_slice::<crate::UiState>(&saved).unwrap().sanitized();
    restore(&mut b);
    assert!(b.session.guides_locked());

    let mut older = app();
    older.ui.engine_prefs = json!({"gridlineEvery": 50});
    restore(&mut older);
    assert!(!older.session.guides_locked());
}

/// OK with the option unchanged keeps the lock from the View menu. OK after turning the option
/// off, as Reset Preferences does in the dialog, unlocks guides.
#[test]
fn preferences_ok_changes_the_guide_lock_only_with_the_option() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("view.guides.lock", json!({"locked": true})).unwrap();
    open(&mut a, Some("Guides & Grid"));
    confirm(&mut a).unwrap();
    assert!(a.session.guides_locked(), "the option did not change");
    a.run("view.guides.lock", json!({"locked": false})).unwrap();
    a.run("prefs.set", json!({"key": "lockGuidesByDefault", "value": true})).unwrap();
    assert!(a.session.guides_locked());
    open(&mut a, Some("Guides & Grid"));
    a.ui.dialog.as_mut().unwrap().fields.insert("lockGuidesByDefault".into(), Prefs::default().to_json()["lockGuidesByDefault"].clone());
    confirm(&mut a).unwrap();
    assert!(!a.session.guides_locked(), "the option went off");
}

/// Click the Preferences dialog's button `label` with the pointer.
fn click_button(app: &mut VectorcraftApp, label: &str) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        // A window takes a sizing frame and fades in: each frame is a quarter second on.
        let time = ctx.input(|i| i.time) + 0.25;
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0)));
        let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        fn walk(s: &egui::Shape, v: &mut Vec<(String, egui::Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    };
    frame(app, vec![]);
    let texts = frame(app, vec![]);
    let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, vec![egui::Event::PointerMoved(at)]);
    frame(app, vec![press(true)]);
    frame(app, vec![press(false)]);
}

/// Click Reset Preferences and answer its question with OK: Preferences again, its working copy
/// at the defaults.
fn reset_preferences(app: &mut VectorcraftApp) {
    click_button(app, "Reset Preferences");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::confirm::RESET_PREFS));
    crate::dialogs::confirm(app).unwrap();
}

/// The Preferences dialog drawn in `lang` with the open dialog available: the label and screen
/// rectangle of each control, from the accessibility tree, and the dialog's content rectangle
/// (inside its 20 pt margin and frame stroke).
fn dialog_controls(lang: crate::i18n::Lang) -> (Vec<(String, egui::Rect)>, egui::Rect) {
    crate::i18n::set_current(lang);
    let mut a = app();
    a.services.pick_open = Some(Box::new(|_: &crate::FilePick| None));
    open(&mut a, Some("General"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let mut v = vec![];
    for time in [0.0, 0.25, 0.5] {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0)));
        let raw = egui::RawInput { screen_rect, time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(&mut a, ui.ctx()));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        v = nodes
            .iter()
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                Some((n.label()?.to_string(), egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))))
            })
            .collect();
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let window = ctx.memory(|m| m.area_rect(egui::Id::unique("dialog-preferences"))).expect("the dialog window is shown");
    let content = window.shrink(20.0 + ctx.global_style().visuals.window_stroke.width);
    (v, content)
}

/// Export Preferences…, Import Preferences… and Reset Preferences share one row above OK and
/// Cancel.
#[test]
fn export_import_and_reset_share_one_row() {
    let (controls, _) = dialog_controls(crate::i18n::Lang::EN);
    let top = |label: &str| controls.iter().find(|(t, _)| t == label).map(|(_, r)| r.top()).unwrap_or_else(|| panic!("no `{label}` in {controls:?}"));
    let row = top("Export Preferences…");
    assert!((top("Import Preferences…") - row).abs() < 0.5, "Import is on Export's row");
    assert!((top("Reset Preferences") - row).abs() < 0.5, "Reset is on Export's row");
    assert!(top("OK") > row + 20.0, "OK is on the row below");
}

/// In every language the dialog keeps its 760 pt content width, and the three buttons above OK
/// lie inside it and above the row of OK and Cancel.
#[test]
fn export_import_and_reset_fit_in_every_language() {
    for lang in crate::i18n::Lang::all() {
        let (controls, content) = dialog_controls(lang);
        assert!(content.width() <= 760.5, "{}: the dialog's content grew to {content:?}", lang.code());
        let find = |label: &str| {
            let label = crate::i18n::tr(lang, label);
            controls.iter().find(|(t, _)| t == label).map(|(_, r)| *r).unwrap_or_else(|| panic!("{}: no `{label}` in {controls:?}", lang.code()))
        };
        let ok = find("OK");
        for label in ["Export Preferences…", "Import Preferences…", "Reset Preferences"] {
            let r = find(label);
            assert!(
                r.right() <= content.right() + 0.5 && r.left() >= content.left() - 0.5,
                "{}: `{label}` at {r:?} is outside the dialog's content {content:?}",
                lang.code()
            );
            assert!(r.bottom() <= ok.top(), "{}: `{label}` at {r:?} reaches the OK row at {ok:?}", lang.code());
        }
    }
}

/// Reset Preferences then OK resets the preferences, General included, and leaves the open
/// document's units, with no undo step in the document.
#[test]
fn reset_preferences_then_ok_leaves_the_documents_units() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 5})).unwrap();
    let undo = a.session.active().unwrap().history.undo.len();
    open(&mut a, Some("General"));
    reset_preferences(&mut a);
    let d = a.ui.dialog.as_ref().expect("Reset leaves the dialog open");
    assert_eq!(d.fields["keyboardIncrement"], Prefs::default().to_json()["keyboardIncrement"], "the working copy holds the defaults");
    assert_eq!(d.fields["unitsGeneral"], json!("points"));
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.keyboard_increment, Prefs::default().keyboard_increment);
    assert_eq!(a.session.prefs.units_general, Prefs::default().units_general);
    let st = a.session.active().unwrap();
    assert_eq!(st.doc.units, vectorcraft_doc::Unit::Millimeters);
    assert_eq!(st.history.undo.len(), undo, "no undo step");
}

/// Reset Preferences returns every UI-only row (Smart Guides, Snap to Grid, Enable Contextual Task
/// Bar) to its default in the dialog, the value a new app shows. OK applies them; Cancel keeps the values in use.
#[test]
fn reset_preferences_resets_the_ui_only_rows() {
    let mut fresh = app();
    open(&mut fresh, None);
    let first = fresh.ui.dialog.take().unwrap().fields;
    let rows = |a: &VectorcraftApp| (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar);
    let default = rows(&fresh);
    let changed = (!default.0, !default.1, !default.2);
    let reset_changed = |a: &mut VectorcraftApp| {
        (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar) = changed;
        open(a, Some("General"));
        for (key, _, _) in UI_FIELDS {
            assert_ne!(a.ui.dialog.as_ref().unwrap().fields[*key], first[*key], "{key} starts changed");
        }
        reset_preferences(a);
    };
    let mut a = app();
    reset_changed(&mut a);
    for (key, _, _) in UI_FIELDS {
        assert_eq!(a.ui.dialog.as_ref().unwrap().fields[*key], first[*key], "{key} is at its default in the dialog");
    }
    assert_eq!(rows(&a), changed, "nothing applied before OK");
    confirm(&mut a).unwrap();
    assert_eq!(rows(&a), default, "OK applies the defaults");

    let mut b = app();
    reset_changed(&mut b);
    crate::dialogs::cancel(&mut b);
    assert!(b.ui.dialog.is_none());
    assert_eq!(rows(&b), changed, "Cancel keeps the values in use");
}

/// `prefs.reset` (the Command Palette's Reset Preferences) also returns the UI-only rows to their
/// defaults, as a batch step too; with a category, only that category's rows; with an
/// unknown or malformed category, none.
#[test]
fn prefs_reset_resets_the_ui_only_rows_of_its_category() {
    let fresh = app();
    let rows = |a: &VectorcraftApp| (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar);
    let default = rows(&fresh);
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar) = (!default.0, !default.1, !default.2);
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 7})).unwrap();
    for bad in [json!("Nowhere"), json!(["General"]), json!(5)] {
        assert!(a.run("prefs.reset", json!({"category": bad})).is_err(), "{bad}");
        assert_eq!(rows(&a), (!default.0, !default.1, !default.2), "{bad}: an unknown or malformed category resets nothing");
        assert_eq!(a.session.prefs.keyboard_increment, 7.0, "{bad}: the preferences are kept");
    }
    a.run("prefs.reset", json!({"category": "general"})).unwrap();
    assert_eq!(rows(&a), (!default.0, !default.1, default.2), "General: Enable Contextual Task Bar only");
    a.run("command.batch", json!({"commands": [{"command": "prefs.reset", "params": {"category": "Guides & Grid"}}]})).unwrap();
    assert_eq!(rows(&a), (!default.0, default.1, default.2), "Guides & Grid, as a batch step: Snap to Grid only");
    a.run("prefs.reset", json!({"category": "Smart Guides"})).unwrap();
    assert_eq!(rows(&a), default, "Smart Guides: Smart Guides");
    (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar) = (!default.0, !default.1, !default.2);
    a.run("prefs.reset", json!({})).unwrap();
    assert_eq!(rows(&a), default, "every category");
}

/// Reset Preferences and `prefs.reset` give Enable Contextual Task Bar the value the current
/// workspace was saved with, which for a workspace saved with the bar off differs from the default.
#[test]
fn reset_preferences_gives_the_task_bar_the_workspaces_value() {
    let off = crate::workspaces::builtins().into_iter().find(|w| !w.task_bar).unwrap();
    let in_workspace = |a: &mut VectorcraftApp| {
        crate::workspaces::switch(&mut a.ui, &off.name).unwrap();
        assert!(!a.ui.task_bar, "{} starts with the bar off", off.name);
        a.ui.task_bar = true;
    };
    let mut a = app();
    in_workspace(&mut a);
    open(&mut a, Some("General"));
    reset_preferences(&mut a);
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields["__taskBar"], json!(false), "the dialog shows the workspace's value");
    confirm(&mut a).unwrap();
    assert!(!a.ui.task_bar, "OK applies the workspace's value");

    let mut b = app();
    in_workspace(&mut b);
    b.run("prefs.reset", json!({"category": "General"})).unwrap();
    assert!(!b.ui.task_bar, "prefs.reset gives the workspace's value");
    let mut c = app();
    assert!(c.ui.task_bar, "the default workspace starts with the bar on");
    c.ui.task_bar = false;
    c.run("prefs.reset", json!({})).unwrap();
    assert!(c.ui.task_bar, "the default workspace's value");
}

/// On a touch screen, OK after Reset Preferences turns Enable Touch Workspace back on: the reset task
/// bar goes to the workspace left, and Touch keeps its own. With Touch entered automatically while
/// Preferences was open, clearing Enable Touch Workspace gives the row to the workspace returned to.
#[test]
fn ok_sets_the_task_bar_in_the_workspace_outside_touch() {
    let mut a = app();
    a.touch_screen = true;
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    crate::workspaces::switch(&mut a.ui, "Essentials Classic").unwrap();
    a.ui.task_bar = true;
    open(&mut a, Some("General"));
    reset_preferences(&mut a);
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields["touchWorkspace"], json!(true), "the reset turns it on");
    confirm(&mut a).unwrap();
    assert_eq!(a.ui.workspace, crate::workspaces::TOUCH);
    assert!(a.ui.task_bar, "Touch keeps its own task bar");
    let classic = a.ui.workspace_changes.iter().find(|w| w.name == "Essentials Classic").unwrap();
    assert!(!classic.task_bar, "the workspace left takes the reset value");

    let mut b = app();
    b.run("prefs.set", json!({"key": "touchWorkspace", "value": true})).unwrap();
    crate::workspaces::switch(&mut b.ui, "Essentials Classic").unwrap();
    open(&mut b, Some("General"));
    b.touch_screen = true;
    crate::workspaces::sync_touch_workspace(&mut b, true);
    assert_eq!(b.ui.workspace, crate::workspaces::TOUCH);
    b.ui.dialog.as_mut().unwrap().fields.insert("touchWorkspace".into(), json!(false));
    confirm(&mut b).unwrap();
    assert_eq!(b.ui.workspace, "Essentials Classic");
    assert!(!b.ui.task_bar, "the workspace returned to takes the row");
    let touch = b.ui.workspace_changes.iter().find(|w| w.name == crate::workspaces::TOUCH).unwrap();
    assert!(touch.task_bar, "Touch keeps its own task bar");
}

/// A Touch return marker left over from an earlier automatic switch (Window › Workspace does not
/// clear it) does not change where OK puts the task bar: OK enters Touch from the workspace it
/// leaves, so that workspace takes the row and Touch keeps its own.
#[test]
fn ok_sets_the_task_bar_in_the_workspace_left_with_a_stale_touch_marker() {
    let mut a = app();
    a.touch_screen = true;
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    a.ui.touch_workspace_restore = Some("Typography".into());
    crate::workspaces::switch(&mut a.ui, "Essentials Classic").unwrap();
    a.ui.task_bar = true;
    open(&mut a, Some("General"));
    reset_preferences(&mut a);
    confirm(&mut a).unwrap();
    assert_eq!(a.ui.workspace, crate::workspaces::TOUCH);
    assert!(a.ui.task_bar, "Touch keeps its own task bar");
    let classic = a.ui.workspace_changes.iter().find(|w| w.name == "Essentials Classic").unwrap();
    assert!(!classic.task_bar, "the workspace left takes the reset value");
}

/// A `prefs.reset` run straight on the session, outside `VectorcraftApp::run`, resets the UI-only
/// rows by the end of the next frame.
#[test]
fn prefs_reset_outside_run_resets_the_ui_only_rows_in_the_next_frame() {
    let mut a = app();
    let default = (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar);
    (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar) = (!default.0, !default.1, !default.2);
    a.session.execute("prefs.reset", &json!({})).unwrap();
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        a.logic(ui.ctx());
        a.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!((a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar), default);

    // Before the next command switches workspaces: the workspace left keeps the reset value.
    let mut b = app();
    crate::workspaces::switch(&mut b.ui, "Essentials Classic").unwrap();
    b.ui.task_bar = true;
    b.session.execute("prefs.reset", &json!({"category": "General"})).unwrap();
    b.run("window.workspace", json!({"name": crate::workspaces::ESSENTIALS})).unwrap();
    assert!(b.ui.task_bar, "Essentials keeps its own task bar");
    let classic = b.ui.workspace_changes.iter().find(|w| w.name == "Essentials Classic").unwrap();
    assert!(!classic.task_bar, "the workspace left keeps the reset value");
}

/// After Reset Preferences, choosing a General unit in the dialog still sets the document's units
/// on OK, Points included once another unit was chosen in between. With no document open, Reset
/// Preferences resets General to its default.
#[test]
fn reset_preferences_keeps_an_edit_of_general_units() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    open(&mut a, Some("General"));
    reset_preferences(&mut a);
    a.ui.dialog.as_mut().unwrap().fields.insert("unitsGeneral".into(), json!("inches"));
    confirm(&mut a).unwrap();
    assert_eq!(a.session.active().unwrap().doc.units, vectorcraft_doc::Unit::Inches);
    assert_eq!(a.session.prefs.units_general, "inches");

    // Between frames, as `ui.dialog.set` sets fields.
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    open(&mut a, Some("General"));
    reset_preferences(&mut a);
    for u in ["inches", "points"] {
        crate::dialogs::set_field(a.ui.dialog.as_mut().unwrap(), "unitsGeneral".into(), json!(u));
    }
    confirm(&mut a).unwrap();
    assert_eq!(a.session.active().unwrap().doc.units, vectorcraft_doc::Unit::Points);

    // Through a frame of the dialog.
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    open(&mut a, Some("General"));
    reset_preferences(&mut a);
    a.ui.dialog.as_mut().unwrap().fields.insert("unitsGeneral".into(), json!("inches"));
    click_button(&mut a, "General");
    a.ui.dialog.as_mut().unwrap().fields.insert("unitsGeneral".into(), json!("points"));
    confirm(&mut a).unwrap();
    assert_eq!(a.session.active().unwrap().doc.units, vectorcraft_doc::Unit::Points);

    let mut b = app();
    b.run("prefs.set", json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
    open(&mut b, Some("General"));
    reset_preferences(&mut b);
    confirm(&mut b).unwrap();
    assert_eq!(b.session.prefs.units_general, Prefs::default().units_general);
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

/// Performance › Low-Latency Drawing (#6013): shown off with its restart note, applied by OK,
/// saved with the UI state under the key the desktop app reads before the window opens, and every
/// bundled language translates its label.
#[test]
fn low_latency_drawing_preference_shows_and_persists() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut a = app();
    open(&mut a, Some("Performance"));
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields.get("lowLatencyDrawing"), Some(&json!(false)));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    let at = shown.iter().position(|t| t == "Low-Latency Drawing").unwrap_or_else(|| panic!("{shown:?}"));
    // The note is the row's own: it comes before the next row's label.
    let next = shown[at..].iter().position(|t| t.starts_with("History States")).map(|n| at + n).unwrap_or_else(|| panic!("{shown:?}"));
    assert!(shown[at..next].iter().any(|t| t == "Applies the next time VectorCraft starts."), "{shown:?}");
    a.ui.dialog.as_mut().unwrap().fields.insert("lowLatencyDrawing".into(), json!(true));
    confirm(&mut a).unwrap();
    assert!(a.take_prefs_changed(), "OK asks the host to write the preferences file");
    assert!(a.session.prefs.low_latency_drawing);
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&a.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["lowLatencyDrawing"], json!(true));
    for lang in crate::i18n::Lang::all().filter(|l| l.code() != crate::i18n::Lang::EN.code()) {
        assert!(crate::i18n::has(lang, "Low-Latency Drawing"), "{}", lang.code());
    }
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

/// User Interface › VectorCraft Title Bar on Every Desktop: shown on Linux and the BSDs only, under
/// System Title Bar with the next-launch note once, and saved under the key the desktop app reads
/// before the window opens.
#[test]
fn app_title_bar_preference_shows_and_persists() {
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
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
    out.textures_delta.clear();
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    let supported = cfg!(not(any(target_arch = "wasm32", target_os = "macos", target_os = "windows")));
    assert_eq!(shown.iter().any(|t| t == "VectorCraft Title Bar on Every Desktop"), supported, "{shown:?}");
    let notes = shown.iter().filter(|t| *t == "Applies the next time VectorCraft starts.").count();
    assert_eq!(notes, usize::from(cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos")))), "{shown:?}");
    a.ui.dialog.as_mut().unwrap().fields.insert("appTitleBar".into(), json!(true));
    confirm(&mut a).unwrap();
    assert!(a.session.prefs.app_title_bar);
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&a.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["appTitleBar"], json!(true));
}

/// VectorCraft Title Bar on Every Desktop does not take clicks while System Title Bar is on or
/// VECTORCRAFT_TITLEBAR is set. With the variable set its note names the variable in place of the
/// next-launch note, and with System Title Bar on and the variable unset a note says that System
/// Title Bar takes precedence.
#[test]
fn app_title_bar_is_dimmed_while_something_takes_precedence() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let note = "The VECTORCRAFT_TITLEBAR environment variable chooses the title bar while it is set.";
    let system_note = "System Title Bar takes precedence over this option while it is on.";
    let restart_note = "Applies the next time VectorCraft starts.";
    let supported = cfg!(not(any(target_arch = "wasm32", target_os = "macos", target_os = "windows")));
    for (from_env, system) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut a = app();
        a.titlebar_from_env = from_env;
        open(&mut a, Some("User Interface"));
        a.ui.dialog.as_mut().unwrap().fields.insert("systemTitleBar".into(), json!(system));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
        out.textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
        out.textures_delta.clear();
        let mut shown = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
        assert_eq!(shown.iter().any(|t| t == note), supported && from_env, "{shown:?}");
        assert_eq!(shown.iter().any(|t| t == system_note), supported && system && !from_env, "{shown:?}");
        // The next-launch note stays under this row on Linux and the BSDs, where it also covers
        // System Title Bar, and under System Title Bar on Windows.
        let restart_shown = if supported { !from_env } else { cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos"))) };
        assert_eq!(shown.iter().filter(|t| *t == restart_note).count(), usize::from(restart_shown), "{shown:?}");
        // A click on the checkbox's label turns the preference on only when nothing takes precedence.
        let label = out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "VectorCraft Title Bar on Every Desktop" => Some(t.visual_bounding_rect()),
            _ => None,
        });
        assert_eq!(label.is_some(), supported);
        if let Some(r) = label {
            for pressed in [true, false] {
                let input = egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
                    ],
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| show(&mut a, ui.ctx()));
                out.textures_delta.clear();
            }
            assert_eq!(a.ui.dialog.as_ref().unwrap().bool("appTitleBar"), !from_env && !system, "variable {from_env}, System Title Bar {system}");
        }
    }
}

/// While VECTORCRAFT_TITLEBAR chooses the frame, System Title Bar does not take clicks.
#[test]
fn system_title_bar_is_dimmed_while_the_environment_variable_chooses_the_frame() {
    let supported = cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos")));
    for from_env in [false, true] {
        let mut a = app();
        a.titlebar_from_env = from_env;
        open(&mut a, Some("User Interface"));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
        out.textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
        out.textures_delta.clear();
        let label = out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "System Title Bar" => Some(t.visual_bounding_rect()),
            _ => None,
        });
        assert_eq!(label.is_some(), supported);
        if let Some(r) = label {
            for pressed in [true, false] {
                let input = egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
                    ],
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| show(&mut a, ui.ctx()));
                out.textures_delta.clear();
            }
            assert_eq!(a.ui.dialog.as_ref().unwrap().bool("systemTitleBar"), !from_env, "clicked with the variable {from_env}");
        }
    }
}

/// Where the desktop draws the frame with System Title Bar on or off (KDE, Sway, X11, a window
/// opened through XWayland), a note under System Title Bar says so while VectorCraft Title Bar on
/// Every Desktop is off and VECTORCRAFT_TITLEBAR is unset. Linux and the BSDs only.
#[test]
fn system_title_bar_says_when_the_desktop_keeps_the_frame() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let note = "This desktop draws the window's title bar with this option on or off while VectorCraft Title Bar on Every Desktop is off.";
    let supported = cfg!(not(any(target_arch = "wasm32", target_os = "macos", target_os = "windows")));
    for keeps in [false, true] {
        for from_env in [false, true] {
            for (app_title_bar, system) in [(false, false), (false, true), (true, false), (true, true)] {
                let mut a = app();
                a.desktop_keeps_frame = keeps;
                a.titlebar_from_env = from_env;
                open(&mut a, Some("User Interface"));
                let fields = &mut a.ui.dialog.as_mut().unwrap().fields;
                fields.insert("appTitleBar".into(), json!(app_title_bar));
                fields.insert("systemTitleBar".into(), json!(system));
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
                out.textures_delta.clear();
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut a, ui.ctx()));
                out.textures_delta.clear();
                let mut shown = vec![];
                out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
                let expected = supported && keeps && !from_env && !app_title_bar;
                let label = format!("keeps {keeps}, variable {from_env}, app title bar {app_title_bar}, system {system}: {shown:?}");
                assert_eq!(shown.iter().any(|t| t == note), expected, "{label}");
            }
        }
    }
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

/// One frame of the Preferences dialog with `events`, keys first as the app takes them.
fn search_frame(a: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        crate::shortcuts::handle(a, ui.ctx());
        show(a, ui.ctx());
    });
    out.textures_delta.clear();
    out
}

/// A key pressed and let go in one frame.
fn key_press(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }).to_vec()
}

fn search_text(a: &VectorcraftApp) -> String {
    a.ui.dialog.as_ref().map(|d| d.str("__search")).unwrap_or_default()
}

/// Preferences' search finds a setting by its label, ignoring case; an empty search finds nothing.
#[test]
fn a_search_finds_a_setting_by_its_label() {
    let keys = |q: &str| search(q).iter().map(|s| s.key).collect::<Vec<_>>();
    assert!(keys("tool tips").contains(&"showToolTips"), "{:?}", keys("tool tips"));
    assert!(keys("SHOW TOOL TIPS").contains(&"showToolTips"));
    assert!(keys("snap to grid").contains(&"__snapToGrid"));
    let found = search("gridline every");
    assert_eq!(found.iter().map(|s| (s.key, s.category)).collect::<Vec<_>>(), [("gridlineEvery", "Guides & Grid")]);
    assert!(search("").is_empty() && search("   ").is_empty());
    assert!(search("no such setting at all").is_empty());
}

/// A translated label matches while the dialog is drawn in that language.
#[test]
fn a_translated_label_matches_in_the_interface_language() {
    let de = crate::i18n::Lang::from_code("de").expect("de registered");
    let label = crate::i18n::tr(de, "Show Tool Tips");
    assert_ne!(label, "Show Tool Tips", "German has a row for the label");
    crate::i18n::set_current(de);
    let found: Vec<_> = search(&label.to_uppercase()).iter().map(|s| s.key).collect();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert!(found.contains(&"showToolTips"), "{label}: {found:?}");
}

/// An empty search field shows the category list; a search that finds nothing says so.
#[test]
fn an_empty_search_lists_the_categories_and_no_match_says_so() {
    let mut a = app();
    open(&mut a, Some("General"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(PREF_CATEGORIES.iter().all(|c| text.contains(c)), "{text}");
    assert!(!text.contains("No settings found"), "{text}");
    a.ui.dialog.as_mut().unwrap().fields.insert("__search".into(), json!("no such setting at all"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("No settings found"), "{text}");
    assert!(!text.contains("Plug-ins & Scratch Disks"), "the list is hidden: {text}");
    a.ui.dialog.as_mut().unwrap().fields.insert("__search".into(), json!("  "));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(PREF_CATEGORIES.iter().all(|c| text.contains(c)), "{text}");
}

/// Choosing a setting the search found shows its category, clears the search and highlights the
/// setting's row.
#[test]
fn choosing_a_found_setting_goes_to_its_category() {
    let mut a = app();
    open(&mut a, Some("General"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    a.ui.dialog.as_mut().unwrap().fields.insert("__search".into(), json!("gridline"));
    search_frame(&mut a, &ctx, vec![]);
    let out = search_frame(&mut a, &ctx, vec![]);
    let mut at = None;
    fn find(s: &egui::Shape, label: &str, at: &mut Option<egui::Pos2>) {
        match s {
            egui::Shape::Text(t) if t.galley.text() == label => *at = Some(t.pos + t.galley.rect.center().to_vec2()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| find(s, label, at)),
            _ => {}
        }
    }
    out.shapes.iter().for_each(|c| find(&c.shape, "Gridline every", &mut at));
    let at = at.expect("the found setting is listed");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    search_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(at), button(true)]);
    search_frame(&mut a, &ctx, vec![button(false)]);
    let d = a.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("__category").as_str(), d.str("__found").as_str(), d.str("__search").as_str()), ("Guides & Grid", "gridlineEvery", ""));
    search_frame(&mut a, &ctx, vec![]);
    let out = search_frame(&mut a, &ctx, vec![]);
    fn rects(s: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
        match s {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| rects(s, out)),
            _ => {}
        }
    }
    let mut shown = vec![];
    out.shapes.iter().for_each(|c| rects(&c.shape, &mut shown));
    let mut label = None;
    out.shapes.iter().for_each(|c| find(&c.shape, "Gridline every:", &mut label));
    let label = label.expect("the setting's row is shown");
    // A tinted band across the fields column, behind the setting's label.
    let band = shown.iter().any(|(r, fill)| r.width() >= 500.0 && r.height() < 40.0 && fill.a() > 0 && r.contains(label));
    assert!(band, "the setting's row is highlighted: {shown:?}");
    assert!(!a.ui.dialog.as_ref().unwrap().bool("__scrollTo"), "the list scrolled to it");
    // Another category: the highlight goes.
    a.ui.dialog.as_mut().unwrap().fields.insert("__category".into(), json!("Units"));
    search_frame(&mut a, &ctx, vec![]);
    assert_eq!(a.ui.dialog.as_ref().unwrap().str("__found"), "");
}

/// The search field has the keyboard focus when the dialog opens, and Cmd/Ctrl+F gives it back.
#[test]
fn the_search_field_takes_the_focus_on_open_and_on_cmd_f() {
    let mut a = app();
    open(&mut a, None);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let focused = |ctx: &egui::Context| ctx.memory(|m| m.has_focus(search_edit_id()));
    search_frame(&mut a, &ctx, vec![]);
    search_frame(&mut a, &ctx, vec![]);
    assert!(focused(&ctx), "focused on open");
    ctx.memory_mut(|m| m.surrender_focus(search_edit_id()));
    search_frame(&mut a, &ctx, vec![]);
    assert!(!focused(&ctx));
    search_frame(&mut a, &ctx, key_press(egui::Key::F, egui::Modifiers::COMMAND));
    assert!(focused(&ctx), "focused on Cmd/Ctrl+F");
    assert!(a.ui.dialog.is_some());
}

/// Escape in the search field with text clears it and keeps the dialog open with the field
/// focused; Escape in the empty field closes the dialog.
#[test]
fn escape_clears_the_search_then_closes_the_dialog() {
    let mut a = app();
    open(&mut a, None);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    search_frame(&mut a, &ctx, vec![]);
    search_frame(&mut a, &ctx, vec![]);
    search_frame(&mut a, &ctx, vec![egui::Event::Text("grid".into())]);
    assert_eq!(search_text(&a), "grid");
    search_frame(&mut a, &ctx, key_press(egui::Key::Escape, egui::Modifiers::NONE));
    assert!(a.ui.dialog.is_some(), "the first Escape leaves the dialog open");
    assert_eq!(search_text(&a), "");
    search_frame(&mut a, &ctx, vec![]);
    assert!(ctx.memory(|m| m.has_focus(search_edit_id())), "the field keeps the focus");
    search_frame(&mut a, &ctx, key_press(egui::Key::Escape, egui::Modifiers::NONE));
    assert!(a.ui.dialog.is_none(), "Escape in the empty field closes the dialog");
}

/// One frame of the open dialog as the app draws it, keys first, a quarter second after the last
/// (a window takes a sizing frame and fades in). → The texts it painted, with their rectangles.
fn dialog_frame(a: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    let time = ctx.input(|i| i.time) + 0.25;
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0)));
    let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        crate::shortcuts::handle(a, ui.ctx());
        crate::dialogs::show(a, ui.ctx());
    });
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, v: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
            egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut v = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
    v
}

/// Click the text `label` in the open dialog with the pointer.
fn click_text(a: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    dialog_frame(a, ctx, vec![]);
    let texts = dialog_frame(a, ctx, vec![]);
    let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    dialog_frame(a, ctx, vec![egui::Event::PointerMoved(at)]);
    dialog_frame(a, ctx, vec![press(true)]);
    dialog_frame(a, ctx, vec![press(false)]);
}

/// Preferences with an edited Keyboard Increment, the Units category and a search, over a stored
/// Keyboard Increment of 5.
fn edited_preferences() -> VectorcraftApp {
    let mut a = app();
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 5})).unwrap();
    open(&mut a, Some("Units"));
    let d = a.ui.dialog.as_mut().unwrap();
    d.fields.insert("keyboardIncrement".into(), json!(7));
    d.fields.insert("__search".into(), json!("grid"));
    a
}

/// Reset Preferences asks first, over Preferences. Escape, or Cancel, shows Preferences again as it
/// was: its edits, category and search kept, and nothing applied.
#[test]
fn reset_preferences_asks_first_and_cancel_returns_to_preferences() {
    let mut a = edited_preferences();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for way_back in ["Escape", "Cancel"] {
        click_text(&mut a, &ctx, "Reset Preferences");
        let d = a.ui.dialog.clone().expect("a question is open");
        assert_eq!(d.kind, crate::dialogs::confirm::RESET_PREFS, "Reset Preferences asks first");
        let under = crate::dialogs::parent(&d).expect("the question is over Preferences");
        assert_eq!((under.kind.as_str(), under.fields["keyboardIncrement"].clone()), ("preferences", json!(7)), "Preferences is unchanged");
        dialog_frame(&mut a, &ctx, vec![]);
        let shown: Vec<String> = dialog_frame(&mut a, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
        assert!(shown.iter().any(|t| t == "Reset all preferences to their defaults?"), "{shown:?}");
        if way_back == "Escape" {
            dialog_frame(&mut a, &ctx, key_press(egui::Key::Escape, egui::Modifiers::NONE));
        } else {
            click_text(&mut a, &ctx, "Cancel");
        }
        let d = a.ui.dialog.as_ref().unwrap_or_else(|| panic!("{way_back}: Preferences is open again"));
        assert_eq!(d.kind, "preferences", "{way_back}");
        assert_eq!(d.fields["keyboardIncrement"], json!(7), "{way_back}: the edit is kept");
        assert_eq!((d.str("__category").as_str(), d.str("__search").as_str()), ("Units", "grid"), "{way_back}: category and search kept");
        assert_eq!(a.session.prefs.keyboard_increment, 5.0, "{way_back}: nothing applied");
    }
}

/// OK on the question shows Preferences again with every preference at its default, its category
/// and search kept; nothing is applied until OK in Preferences, and Preferences' Cancel keeps the
/// stored preferences.
#[test]
fn confirming_reset_fills_preferences_and_applies_nothing_until_ok() {
    let mut a = edited_preferences();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let default = Prefs::default().to_json()["keyboardIncrement"].clone();
    click_text(&mut a, &ctx, "Reset Preferences");
    click_text(&mut a, &ctx, "OK");
    let d = a.ui.dialog.as_ref().expect("Preferences is open again");
    assert_eq!(d.kind, "preferences");
    assert_eq!(d.fields["keyboardIncrement"], default, "the working copy holds the defaults");
    assert_eq!((d.str("__category").as_str(), d.str("__search").as_str()), ("Units", "grid"));
    assert_eq!(a.session.prefs.keyboard_increment, 5.0, "nothing applied yet");
    click_text(&mut a, &ctx, "Cancel");
    assert!(a.ui.dialog.is_none());
    assert_eq!(a.session.prefs.keyboard_increment, 5.0, "Cancel in Preferences keeps the preferences");

    // `ui.dialog.confirm` answers the question as OK does; OK in Preferences then applies the defaults.
    let mut a = edited_preferences();
    click_text(&mut a, &ctx, "Reset Preferences");
    crate::dialogs::confirm(&mut a).unwrap();
    assert_eq!(a.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("preferences"));
    crate::dialogs::confirm(&mut a).unwrap();
    assert!(a.ui.dialog.is_none());
    assert_eq!(a.session.prefs.keyboard_increment, Prefs::default().keyboard_increment);
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

/// General › Scroll Sideways with Cmd/Ctrl + Wheel: its checkbox is dimmed while Zoom with
/// Mouse Wheel is checked in the dialog, and a click on it then leaves the setting as it was.
#[test]
fn scroll_sideways_with_cmd_wheel_dims_with_zoom_with_mouse_wheel() {
    fn label(s: &egui::Shape, text: &str) -> Option<(egui::Color32, egui::Rect)> {
        match s {
            egui::Shape::Text(t) if t.galley.text() == text => t.galley.job.sections.first().map(|s| (s.format.color, t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().find_map(|s| label(s, text)),
            _ => None,
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut a = app();
    let mut frame = |d: &mut Dialog, events: Vec<egui::Event>| {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 900.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            category_fields(ui, &mut a, d, "General");
        });
        out.textures_delta.clear();
        out.shapes.iter().find_map(|c| label(&c.shape, "Scroll Sideways with Cmd/Ctrl + Wheel"))
    };
    let t = Tokens::get(&ctx);
    for (zoom, color, what) in [(false, t.text, "enabled while Zoom with Mouse Wheel is off"), (true, t.text_disabled, "dimmed while it is on")] {
        let mut d = Dialog::new("preferences", json!({"zoomWithMouseWheel": zoom, "ctrlWheelScrollsSideways": true}));
        let (shown, at) = frame(&mut d, vec![]).unwrap_or_else(|| panic!("no label ({what})"));
        assert_eq!(shown, color, "{what}");
        let press =
            |pressed| egui::Event::PointerButton { pos: at.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut d, vec![egui::Event::PointerMoved(at.center()), press(true)]);
        frame(&mut d, vec![press(false)]);
        // A click turns it off while it is enabled, and leaves it on while it is dimmed.
        assert_eq!(d.fields["ctrlWheelScrollsSideways"], json!(zoom), "after a click, {what}");
    }
}

/// General › Hold Shortcut Delay: its field is dimmed while Hold Shortcuts is unchecked in the
/// dialog, and a click on it then does not give it the keyboard.
#[test]
fn hold_shortcut_delay_dims_without_hold_shortcuts() {
    fn text(s: &egui::Shape, f: &dyn Fn(&str) -> bool) -> Option<egui::Rect> {
        match s {
            egui::Shape::Text(t) if f(t.galley.text()) => Some(t.visual_bounding_rect()),
            egui::Shape::Vec(v) => v.iter().find_map(|s| text(s, f)),
            _ => None,
        }
    }
    for on in [true, false] {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut a = app();
        let mut d = Dialog::new("preferences", json!({"holdShortcuts": on, "keyHoldDelay": 0.35}));
        let mut frame = |d: &mut Dialog, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 2400.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                category_fields(ui, &mut a, d, "General");
            });
            out.textures_delta.clear();
            let label = out.shapes.iter().find_map(|c| text(&c.shape, &|t| t.starts_with("Hold Shortcut Delay")));
            let field = out.shapes.iter().find_map(|c| text(&c.shape, &|t| t.starts_with("0.35")));
            (label, field)
        };
        let (label, field) = frame(&mut d, vec![]);
        assert!(label.is_some(), "no label (on {on})");
        let at = field.unwrap_or_else(|| panic!("no field (on {on})"));
        let press =
            |pressed| egui::Event::PointerButton { pos: at.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut d, vec![egui::Event::PointerMoved(at.center()), press(true)]);
        frame(&mut d, vec![press(false)]);
        assert_eq!(ctx.memory(|m| m.focused().is_some()), on, "a click gives the field the keyboard only while Hold Shortcuts is on ({on})");
    }
}

/// A restored Hold Shortcut Delay outside its range opens as the delay in use, and OK keeps it.
#[test]
fn a_restored_hold_shortcut_delay_outside_its_range_opens_as_the_delay_in_use() {
    for (saved, used) in [(5.0, 1.0), (0.05, 0.2)] {
        let mut a = app();
        a.session.prefs.key_hold_delay = saved;
        open(&mut a, Some("General"));
        assert_eq!(a.ui.dialog.as_ref().unwrap().fields["keyHoldDelay"], json!(used), "{saved}");
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.key_hold_delay, used, "{saved}");
    }
}

/// Type › Show samples on white: its checkbox is dimmed while Enable in-menu font previews is
/// unchecked in the dialog, and a click on it then leaves the setting as it was.
#[test]
fn show_samples_on_white_dims_without_in_menu_font_previews() {
    fn label(s: &egui::Shape, text: &str) -> Option<(egui::Color32, egui::Rect)> {
        match s {
            egui::Shape::Text(t) if t.galley.text() == text => t.galley.job.sections.first().map(|s| (s.format.color, t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().find_map(|s| label(s, text)),
            _ => None,
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut a = app();
    let mut frame = |d: &mut Dialog, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 1400.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            category_fields(ui, &mut a, d, "Type");
        });
        out.textures_delta.clear();
        out.shapes.iter().find_map(|c| label(&c.shape, "Show samples on white"))
    };
    let t = Tokens::get(&ctx);
    for (preview, color, what) in [(true, t.text, "enabled while previews are on"), (false, t.text_disabled, "dimmed while they are off")] {
        let mut d = Dialog::new("preferences", json!({"fontPreview": preview, "fontSamplesOnWhite": false}));
        let (shown, at) = frame(&mut d, vec![]).unwrap_or_else(|| panic!("no label ({what})"));
        assert_eq!(shown, color, "{what}");
        let press =
            |pressed| egui::Event::PointerButton { pos: at.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut d, vec![egui::Event::PointerMoved(at.center()), press(true)]);
        frame(&mut d, vec![press(false)]);
        // A click turns it on while it is enabled, and leaves it off while it is dimmed.
        assert_eq!(d.fields["fontSamplesOnWhite"], json!(preview), "after a click, {what}");
    }
}

/// General › Show Tool Cards: its checkbox is dimmed while Show Tool Tips is unchecked in the
/// dialog, and a click on it then leaves the setting as it was.
#[test]
fn show_tool_cards_dims_without_tool_tips() {
    fn label(s: &egui::Shape, text: &str) -> Option<(egui::Color32, egui::Rect)> {
        match s {
            egui::Shape::Text(t) if t.galley.text() == text => t.galley.job.sections.first().map(|s| (s.format.color, t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().find_map(|s| label(s, text)),
            _ => None,
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut a = app();
    let mut frame = |d: &mut Dialog, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 3000.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            category_fields(ui, &mut a, d, "General");
        });
        out.textures_delta.clear();
        out.shapes.iter().find_map(|c| label(&c.shape, "Show Tool Cards"))
    };
    let t = Tokens::get(&ctx);
    for (tips, color, what) in [(true, t.text, "enabled while tool tips are on"), (false, t.text_disabled, "dimmed while they are off")] {
        let mut d = Dialog::new("preferences", json!({"showToolTips": tips, "showToolCards": false}));
        let (shown, at) = frame(&mut d, vec![]).unwrap_or_else(|| panic!("no label ({what})"));
        assert_eq!(shown, color, "{what}");
        let press =
            |pressed| egui::Event::PointerButton { pos: at.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut d, vec![egui::Event::PointerMoved(at.center()), press(true)]);
        frame(&mut d, vec![press(false)]);
        // A click turns it on while it is enabled, and leaves it off while it is dimmed.
        assert_eq!(d.fields["showToolCards"], json!(tips), "after a click, {what}");
    }
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

/// #5236: File Handling › Use System Defaults for Edit Original is a checkbox, on by default, and
/// OK saves it off.
#[test]
fn use_system_defaults_for_edit_original_is_a_checkbox() {
    let mut a = VectorcraftApp::new(Session::new(), Default::default());
    assert!(a.session.prefs.edit_original_system_defaults, "on by default");
    open(&mut a, Some("File Handling"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Use System Defaults for Edit Original"), "{text}");
    assert!(a.ui.dialog.as_ref().unwrap().bool("editOriginalSystemDefaults"));
    a.ui.dialog.as_mut().unwrap().fields.insert("editOriginalSystemDefaults".into(), json!(false));
    confirm(&mut a).unwrap();
    assert!(!a.session.prefs.edit_original_system_defaults);
}

/// A new, empty folder of its own for test `name`.
fn temp_folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-ui-prefs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An app whose save and open dialogs answer `path`, and that writes and reads real files.
fn app_with_file_dialogs(path: &str) -> VectorcraftApp {
    let mut a = app();
    let (save, open) = (path.to_string(), path.to_string());
    a.services.pick_save = Some(Box::new(move |_: &crate::FilePick| Some(save.clone())));
    a.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(open.clone())));
    a.services.write = Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string())));
    a.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    a
}

/// Export Preferences… saves the applied preferences to a file, and Import Preferences… applies
/// them again: the dialog shows them, OK keeps them, and the open document's units stay as they
/// are.
#[test]
fn export_and_import_preferences_buttons_round_trip() {
    let dir = temp_folder("prefs-file");
    let path = dir.join("VectorCraft Preferences.json").to_string_lossy().into_owned();
    let mut a = app_with_file_dialogs(&path);
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    let values = json!({"keyboardIncrement": 5, "uiAccent": "green", "unitsGeneral": "inches"});
    a.run("prefs.set", json!({"values": values, "documentUnits": false})).unwrap();
    let saved = a.session.prefs.to_json();
    open(&mut a, Some("General"));
    click_button(&mut a, "Export Preferences…");
    let file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(file["format"], "vectorcraft-preferences");
    assert_eq!(file["prefs"]["uiAccent"], "green");

    a.ui.dialog = None;
    a.run("prefs.reset", json!({})).unwrap();
    let undo = a.session.active().unwrap().history.undo.len();
    open(&mut a, Some("General"));
    a.ui.dialog.as_mut().unwrap().fields.insert("keyboardIncrement".into(), json!(9));
    click_button(&mut a, "Import Preferences…");
    assert_eq!(a.ui.status, "Preferences imported.");
    assert_eq!(a.session.prefs.to_json(), saved, "the exported preferences are applied");
    assert_eq!(a.ui.engine_prefs, saved, "and saved with the UI state");
    let d = a.ui.dialog.as_ref().expect("the dialog stays open");
    assert_eq!((d.fields["keyboardIncrement"].clone(), d.fields["unitsGeneral"].clone()), (json!(5.0), json!("inches")), "the dialog shows them");
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.to_json(), saved);
    let st = a.session.active().unwrap();
    assert_eq!(st.doc.units, vectorcraft_doc::Unit::Millimeters, "the document's units stay");
    assert_eq!(st.history.undo.len(), undo, "no undo step");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file Import Preferences… can't use changes nothing, the dialog's edits included, and the
/// status bar says why.
#[test]
fn import_preferences_of_a_bad_file_changes_nothing() {
    let dir = temp_folder("prefs-bad-file");
    let path = dir.join("newer.json");
    std::fs::write(&path, r#"{"format": "vectorcraft-preferences", "version": 2, "prefs": {"uiAccent": "pink"}}"#).unwrap();
    let mut a = app_with_file_dialogs(&path.to_string_lossy());
    let before = a.session.prefs.to_json();
    open(&mut a, Some("General"));
    a.ui.dialog.as_mut().unwrap().fields.insert("keyboardIncrement".into(), json!(9));
    click_button(&mut a, "Import Preferences…");
    assert!(a.ui.status.contains("newer version of VectorCraft"), "{}", a.ui.status);
    assert_eq!(a.session.prefs.to_json(), before);
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields["keyboardIncrement"], json!(9), "the dialog keeps its edits");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `prefs.import` from a script or MCP while Preferences is open: the dialog shows the imported
/// values. Import Preferences… shows only where the host has an open dialog (not on the web).
#[test]
fn prefs_import_refreshes_an_open_preferences_dialog() {
    let mut a = app();
    open(&mut a, Some("General"));
    let data = json!({"format": "vectorcraft-preferences", "version": 1, "prefs": {"keyboardIncrement": 7}}).to_string();
    a.run("prefs.import", json!({ "data": data })).unwrap();
    assert_eq!(a.ui.dialog.as_ref().unwrap().fields["keyboardIncrement"], json!(7.0));
    assert_eq!(a.ui.engine_prefs["keyboardIncrement"], json!(7.0));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Export Preferences…") && !text.contains("Import Preferences…"), "{text}");
    a.services.pick_open = Some(Box::new(|_: &crate::FilePick| None));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Import Preferences…"), "{text}");
}

/// An import that turns Enable Touch Workspace on or off switches the workspace on a touch screen,
/// as OK in Preferences does.
#[test]
fn prefs_import_of_touch_workspace_switches_the_workspace() {
    let mut a = app();
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    a.touch_screen = true;
    a.ui.workspace = "Typography".into();
    let file = |on: bool| json!({"format": "vectorcraft-preferences", "version": 1, "prefs": {"touchWorkspace": on}}).to_string();
    a.run("prefs.import", json!({ "data": file(true) })).unwrap();
    assert_eq!(a.ui.workspace, crate::workspaces::TOUCH);
    a.run("prefs.import", json!({ "data": file(false) })).unwrap();
    assert_eq!(a.ui.workspace, "Typography");
}

/// A `prefs.reset` run straight on the session, then a touch screen that switches to Touch: the
/// workspace left keeps the reset value.
#[test]
fn a_pending_prefs_reset_applies_before_a_touch_switch() {
    let mut a = app();
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    crate::workspaces::switch(&mut a.ui, "Essentials Classic").unwrap();
    a.ui.task_bar = true;
    a.session.execute("prefs.reset", &json!({})).unwrap();
    assert!(a.session.prefs.touch_workspace, "the reset turns Enable Touch Workspace on");
    crate::workspaces::sync_touch_workspace(&mut a, true);
    assert_eq!(a.ui.workspace, crate::workspaces::TOUCH);
    assert!(a.ui.task_bar, "Touch keeps its own task bar");
    let classic = a.ui.workspace_changes.iter().find(|w| w.name == "Essentials Classic").unwrap();
    assert!(!classic.task_bar, "the workspace left keeps the reset value");
}

/// File Handling › Data Recovery › Folder has a Choose… button where the host has a folder
/// picker, and none where it has not (the web).
#[test]
fn recovery_folder_has_a_choose_button_with_a_folder_picker() {
    let mut a = VectorcraftApp::new(Session::new(), Default::default());
    a.services.pick_folder = Some(Box::new(|| None));
    open(&mut a, Some("File Handling"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Data Recovery") && text.contains("Choose…"), "{text}");
    a.services.pick_folder = None;
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Data Recovery") && !text.contains("Choose…"), "{text}");
}

/// Choose… puts the folder picked into the Folder field, and OK saves it; a canceled picker
/// leaves the field as it was.
#[test]
fn a_chosen_recovery_folder_fills_the_field_and_cancel_keeps_it() {
    let dir = temp_folder("recovery-chosen");
    let picked = dir.to_string_lossy().into_owned();
    let mut a = VectorcraftApp::new(Session::new(), Default::default());
    open(&mut a, Some("File Handling"));
    let answer = picked.clone();
    a.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
    let mut d = a.ui.dialog.clone().unwrap();
    choose_folder(&mut a, &mut d, "recoveryFolder");
    assert_eq!(d.str("recoveryFolder"), picked);
    assert!(a.ui.status.is_empty(), "{}", a.ui.status);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the write check leaves nothing behind");
    a.services.pick_folder = Some(Box::new(|| None));
    choose_folder(&mut a, &mut d, "recoveryFolder");
    assert_eq!(d.str("recoveryFolder"), picked, "cancel keeps the field");
    a.ui.dialog = Some(d);
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.recovery_folder, picked);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A picked folder that no folder can be created in is refused: the field keeps its value and the
/// status bar says why.
#[test]
fn a_recovery_folder_that_cannot_be_written_is_refused() {
    let dir = temp_folder("recovery-refused");
    let file = dir.join("not-a-folder.txt");
    std::fs::write(&file, b"x").unwrap();
    let mut a = VectorcraftApp::new(Session::new(), Default::default());
    open(&mut a, Some("File Handling"));
    let mut d = a.ui.dialog.clone().unwrap();
    d.fields.insert("recoveryFolder".into(), json!("kept"));
    for path in [file.clone(), dir.join("missing")] {
        let answer = path.to_string_lossy().into_owned();
        a.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
        a.ui.status.clear();
        choose_folder(&mut a, &mut d, "recoveryFolder");
        assert_eq!(d.str("recoveryFolder"), "kept");
        assert_eq!(a.ui.status, format!("{} is not a folder", path.to_string_lossy()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // An account that may write anyway (root) has nothing to refuse.
        if std::fs::create_dir(locked.join("probe")).is_err() {
            let answer = locked.to_string_lossy().into_owned();
            a.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
            a.ui.status.clear();
            choose_folder(&mut a, &mut d, "recoveryFolder");
            assert_eq!(d.str("recoveryFolder"), "kept");
            assert!(a.ui.status.starts_with("Data Recovery can't keep copies in "), "{}", a.ui.status);
        }
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        // A folder that can be written but not listed: the recovery store lists its folders.
        let unlisted = dir.join("unlisted");
        std::fs::create_dir(&unlisted).unwrap();
        std::fs::set_permissions(&unlisted, std::fs::Permissions::from_mode(0o333)).unwrap();
        // An account that may list anyway (root) has nothing to refuse.
        if std::fs::read_dir(&unlisted).is_err() {
            let answer = unlisted.to_string_lossy().into_owned();
            a.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
            a.ui.status.clear();
            choose_folder(&mut a, &mut d, "recoveryFolder");
            assert_eq!(d.str("recoveryFolder"), "kept");
            assert!(a.ui.status.starts_with("Data Recovery can't keep copies in "), "{}", a.ui.status);
        }
        let _ = std::fs::set_permissions(&unlisted, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The recovery folder is an app preference: a folder picked off the UI thread (Linux) fills the
/// open dialog's field even when the active document closed while the picker was open.
#[test]
fn a_chosen_recovery_folder_fills_the_field_after_the_active_document_closes() {
    let dir = temp_folder("recovery-closed");
    let picked = dir.to_string_lossy().into_owned();
    let mut a = VectorcraftApp::new(Session::new(), Default::default());
    let (shown, _) = crate::tests_picks::off_the_ui_thread(&mut a.services);
    a.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    open(&mut a, Some("File Handling"));
    let mut d = a.ui.dialog.take().unwrap();
    choose_folder(&mut a, &mut d, "recoveryFolder");
    a.ui.dialog = Some(d);
    assert!(matches!(shown.requests().as_slice(), [crate::picks::PickRequest::Folder]));
    a.session.execute("file.close", &json!({})).unwrap();
    a.sync_views();
    assert!(a.session.documents().is_empty());
    shown.answer(&[&picked]);
    crate::picks::poll(&mut a, &egui::Context::default());
    assert_eq!(a.ui.dialog.as_ref().unwrap().str("recoveryFolder"), picked, "{}", a.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Plug-ins & Scratch Disks › Additional Plug-ins Folder has a Choose… button where the host has a
/// folder picker, and none where it has not (the web).
#[test]
fn plugins_folder_has_a_choose_button_with_a_folder_picker() {
    let mut a = VectorcraftApp::new(Session::new(), Default::default());
    a.services.pick_folder = Some(Box::new(|| None));
    open(&mut a, Some("Plug-ins & Scratch Disks"));
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Additional Plug-ins Folder") && text.contains("Choose…"), "{text}");
    a.services.pick_folder = None;
    let text = crate::tests_labels::painted_text(&mut a, |a, ui| show(a, ui.ctx()));
    assert!(text.contains("Additional Plug-ins Folder") && !text.contains("Choose…"), "{text}");
}

/// Choose… beside Additional Plug-ins Folder puts the folder picked into the field, and OK saves
/// it; a canceled picker, a file and a missing folder leave the field as it was.
#[test]
fn a_chosen_plugins_folder_fills_the_field_and_others_keep_it() {
    let dir = temp_folder("plugins-chosen");
    let file = dir.join("not-a-folder.txt");
    std::fs::write(&file, b"x").unwrap();
    let mut a = VectorcraftApp::new(Session::new(), Default::default());
    open(&mut a, Some("Plug-ins & Scratch Disks"));
    let mut d = a.ui.dialog.clone().unwrap();
    for path in [file.clone(), dir.join("missing")] {
        let answer = path.to_string_lossy().into_owned();
        a.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
        a.ui.status.clear();
        choose_folder(&mut a, &mut d, "pluginsFolder");
        assert_eq!(d.str("pluginsFolder"), "");
        assert_eq!(a.ui.status, format!("{} is not a folder", path.to_string_lossy()));
    }
    let picked = dir.to_string_lossy().into_owned();
    let answer = picked.clone();
    a.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
    a.ui.status.clear();
    choose_folder(&mut a, &mut d, "pluginsFolder");
    assert_eq!(d.str("pluginsFolder"), picked);
    assert!(a.ui.status.is_empty(), "{}", a.ui.status);
    a.services.pick_folder = Some(Box::new(|| None));
    choose_folder(&mut a, &mut d, "pluginsFolder");
    assert_eq!(d.str("pluginsFolder"), picked, "cancel keeps the field");
    a.ui.dialog = Some(d);
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.plugins_folder, picked);
    let _ = std::fs::remove_dir_all(&dir);
}

/// OK checks a folder typed into Data Recovery › Folder or Additional Plug-ins Folder as Choose…
/// checks a picked one: one that fails keeps the dialog open and changes no preference. An empty
/// field and a folder the preference already holds are not checked.
#[test]
fn ok_refuses_a_typed_folder_that_choose_would_refuse() {
    let dir = temp_folder("typed-folders");
    let file = dir.join("not-a-folder.txt");
    std::fs::write(&file, b"x").unwrap();
    let missing = dir.join("missing").to_string_lossy().into_owned();
    for key in ["recoveryFolder", "pluginsFolder"] {
        let mut a = VectorcraftApp::new(Session::new(), Default::default());
        open(&mut a, Some("File Handling"));
        let before = a.session.prefs.to_json();
        for path in [file.to_string_lossy().into_owned(), missing.clone()] {
            let d = a.ui.dialog.as_mut().unwrap();
            d.fields.insert(key.into(), json!(path));
            d.fields.insert("showToolTips".into(), json!(!a.session.prefs.show_tool_tips));
            assert_eq!(confirm(&mut a), Err(format!("{path} is not a folder")), "{key}");
            assert!(a.ui.dialog.is_some(), "{key}: the dialog stays open");
            assert_eq!(a.session.prefs.to_json(), before, "{key}: no preference changes");
            assert!(!a.prefs_changed, "{key}: nothing to save");
        }
        // A typed folder that passes is saved, with the spaces around it as typed.
        let typed = format!(" {} ", dir.to_string_lossy());
        a.ui.dialog.as_mut().unwrap().fields.insert(key.into(), json!(typed));
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.to_json()[key], json!(typed), "{key}");
        // An empty field keeps the default folder.
        open(&mut a, Some("File Handling"));
        a.ui.dialog.as_mut().unwrap().fields.insert(key.into(), json!(""));
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.to_json()[key], json!(""), "{key}");
        // A folder already saved is not checked again, so OK works while it is unavailable.
        std::fs::create_dir(&missing).unwrap();
        a.run("prefs.set", json!({"key": key, "value": missing})).unwrap();
        std::fs::remove_dir(&missing).unwrap();
        open(&mut a, Some("File Handling"));
        a.ui.dialog.as_mut().unwrap().fields.insert("showToolTips".into(), json!(!a.session.prefs.show_tool_tips));
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.to_json()[key], json!(missing), "{key}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Preferences form rows associate each input with its visible label for accessibility tools.
#[test]
fn preference_inputs_expose_their_visible_labels() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    open(&mut app, Some("General"));
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_accessibility_check(false).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(3);
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Keyboard Increment:");
}

/// Render the real category without the dialog's scroll viewport clipping later rows.
fn preference_category_harness(category: &'static str) -> egui_kittest::Harness<'static, (VectorcraftApp, Dialog)> {
    let mut app = app();
    app.services.pick_folder = Some(Box::new(|| None));
    open(&mut app, Some(category));
    let dialog = app.ui.dialog.take().unwrap();
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 1400.0)).with_accessibility_check(false).build_ui_state(
        move |ui, (app, dialog): &mut (VectorcraftApp, Dialog)| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            category_fields(ui, app, dialog, category);
        },
        (app, dialog),
    );
    h.run_steps(3);
    h
}

#[test]
fn preference_colour_well_exposes_its_row_label_and_value() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let h = preference_category_harness("Guides & Grid");
    let expected = format!("{}ff", h.state().1.str("patternTileEdgeColor").to_lowercase());
    let colour = h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Tile Edge Color:");
    assert_eq!(colour.accesskit_node().value().as_deref(), Some(expected.as_str()));
}

#[test]
fn preference_scale_labels_both_controls_across_focus_changes() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = preference_category_harness("User Interface");
    let expected = h.state().1.f64("uiScaling", 1.0);
    for focus in [None, Some(Role::SpinButton), Some(Role::Slider)] {
        if let Some(role) = focus {
            h.get_by_role_and_label(role, "Scale:").focus();
            h.run_steps(3);
            assert!(h.get_by_role_and_label(role, "Scale:").accesskit_node().is_focused());
        }
        for role in [Role::Slider, Role::SpinButton] {
            let control = h.get_by_role_and_label(role, "Scale:");
            assert_eq!(control.accesskit_node().numeric_value(), Some(expected));
        }
    }
}

#[test]
fn preference_anchor_slider_exposes_its_row_label_and_value() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    let h = preference_category_harness("Selection & Anchor Display");
    let slider = h.get_by_role_and_label(Role::Slider, "Size:");
    assert_eq!(slider.accesskit_node().numeric_value(), Some(h.state().1.f64("anchorSize", 3.0)));
}

#[test]
fn preference_folder_label_preserves_the_choose_action_name() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let h = preference_category_harness("File Handling");
    let _ = h.get_by_role_and_label(Role::TextInput, "Folder:");
    let _ = h.get_by_role_and_label(Role::Button, "Choose…");
    assert!(h.query_by_role_and_label(Role::Button, "Folder:").is_none());
}

/// Under automation roots, OK refuses a typed folder outside them before it looks at the folder.
#[test]
fn ok_refuses_a_typed_folder_outside_the_automation_roots_before_looking_at_it() {
    use vectorcraft_engine::file_access::{self, Access};
    let dir = temp_folder("typed-roots");
    let inside = dir.join("inside");
    std::fs::create_dir_all(&inside).unwrap();
    let outside = dir.join("outside-missing").to_string_lossy().into_owned();
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[inside.as_path()]).unwrap().unwrap();
    for (key, access) in [("recoveryFolder", Access::Write), ("pluginsFolder", Access::Read)] {
        let mut a = VectorcraftApp::new(Session::new(), Default::default());
        open(&mut a, Some("File Handling"));
        a.ui.dialog.as_mut().unwrap().fields.insert(key.into(), json!(outside));
        let r = file_access::confine(Some(&roots), || confirm(&mut a));
        assert_eq!(r, Err(roots.check(&outside, access).unwrap_err()), "{key}");
        assert!(a.ui.dialog.is_some(), "{key}: the dialog stays open");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether a text painted in `rect` shows inside its `clip`: its center is inside the clip, so the
/// pointer on that center reaches the text's row.
fn shows_in_clip(rect: egui::Rect, clip: egui::Rect) -> bool {
    clip.contains(rect.center())
}

/// A text whose top or bottom edge the clip cuts off shows only while its center is inside the
/// clip: a row half scrolled out of the list is not yet one the pointer can be moved onto.
#[test]
fn a_text_shows_only_while_its_center_is_inside_its_clip() {
    let clip = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
    let text = |y: f32| egui::Rect::from_min_size(egui::pos2(10.0, y), egui::vec2(50.0, 20.0));
    assert!(shows_in_clip(text(40.0), clip));
    assert!(shows_in_clip(text(85.0), clip), "the center is 5 pt above the bottom");
    assert!(!shows_in_clip(text(95.0), clip), "the top is inside, the center below the bottom");
    assert!(!shows_in_clip(text(-15.0), clip), "the bottom is inside, the center above the top");
    assert!(!shows_in_clip(text(200.0), clip));
}

/// One frame of the open Preferences dialog with `events`, a quarter second after the last. → The
/// texts it painted where they show (a text whose center its clip rectangle hides is left out),
/// with their rectangles, and the description of the accessibility node with the keyboard focus, if
/// it has one.
fn description_frame(a: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> (Vec<(String, egui::Rect)>, Option<String>) {
    frame_texts(a, ctx, events, |clip, r| shows_in_clip(r, clip))
}

/// [`description_frame`], keeping only the texts whose whole rectangle is inside their clip
/// rectangle, so that the pointer can reach the center of each one.
fn whole_texts_frame(a: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    frame_texts(a, ctx, events, |clip, r| clip.contains_rect(r)).0
}

/// [`description_frame`] with the texts that `keep(clip rectangle, text rectangle)` accepts.
fn frame_texts(
    a: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    keep: fn(egui::Rect, egui::Rect) -> bool,
) -> (Vec<(String, egui::Rect)>, Option<String>) {
    let time = ctx.input(|i| i.time) + 0.25;
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0)));
    let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(a, ui.ctx()));
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, clip: egui::Rect, keep: fn(egui::Rect, egui::Rect) -> bool, v: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => {
                let r = egui::Rect::from_min_size(t.pos, t.galley.size());
                if keep(clip, r) {
                    v.push((t.galley.text().to_string(), r));
                }
            }
            egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, clip, keep, v)),
            _ => {}
        }
    }
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, keep, &mut texts));
    let focused = out.platform_output.accesskit_update.and_then(|u| {
        let focus = u.focus;
        u.nodes.into_iter().find(|(id, _)| *id == focus).and_then(|(_, n)| n.description().map(str::to_string))
    });
    (texts, focused)
}

/// The Preferences window's rectangle.
fn preferences_window(ctx: &egui::Context) -> egui::Rect {
    ctx.memory(|m| m.area_rect(egui::Id::unique("dialog-preferences"))).expect("the dialog window is shown")
}

/// Whether one of `texts` is `sentence`.
fn painted(texts: &[(String, egui::Rect)], sentence: &str) -> bool {
    texts.iter().any(|(t, _)| t == sentence)
}

/// Tab presses from where the focus is, at most 40, until each of `keys` has had its sentence shown.
/// → The keys in the order Tab first showed their sentences, each with the description the focused
/// control carried in that frame.
fn tab_order(a: &mut VectorcraftApp, ctx: &egui::Context, keys: &[&'static str]) -> Vec<(&'static str, Option<String>)> {
    let sentences: Vec<_> =
        keys.iter().map(|key| (*key, descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence")))).collect();
    let mut reached: Vec<(&'static str, Option<String>)> = vec![];
    for _ in 0..40 {
        if reached.len() == keys.len() {
            break;
        }
        let (texts, described) = description_frame(a, ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        for (key, sentence) in &sentences {
            if painted(&texts, sentence) && !reached.iter().any(|(k, _)| k == key) {
                reached.push((key, described.clone()));
            }
        }
    }
    reached
}

/// Tab from where the focus is reaches the controls of `keys` in that order, the first time round:
/// the line shows each one's sentence in turn, and a screen reader gets it as the focused control's
/// description.
fn assert_tab_order(a: &mut VectorcraftApp, ctx: &egui::Context, keys: &[&'static str]) {
    let reached = tab_order(a, ctx, keys);
    let order: Vec<_> = reached.iter().map(|(key, _)| *key).collect();
    assert_eq!(order, keys, "Tab first reaches the controls in this order");
    for (key, described) in &reached {
        let sentence = descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"));
        assert_eq!(described.as_deref(), Some(sentence), "the focused `{key}` control carries the sentence");
    }
}

/// The pointer over a Units row shows that row's sentence at the bottom of the dialog, and the
/// dialog keeps its size.
#[test]
fn hovering_a_units_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Units"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let size = preferences_window(&ctx).size();
    let general = descriptions::description("unitsGeneral").expect("Units › General has a sentence");
    let stroke = descriptions::description("unitsStroke").expect("Units › Stroke has a sentence");
    assert!(!painted(&texts, general), "nothing is described before the pointer comes");
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let at = row(&texts, "General:");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(at)]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&texts, general), "the General row's sentence shows: {texts:?}");
    assert!(!painted(&texts, stroke));
    let line = texts.iter().find(|(t, _)| t == general).map(|(_, r)| *r).unwrap();
    let ok = row(&texts, "OK");
    assert!(line.bottom() < ok.y, "the sentence is above OK: {line:?}, {ok:?}");
    assert_eq!(preferences_window(&ctx).size(), size, "the dialog keeps its size");
    let at = row(&texts, "Stroke:");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(at)]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&texts, stroke) && !painted(&texts, general), "the Stroke row's sentence replaces it");
    // Moving from row to row, across the spacing between them, never leaves the line empty.
    let (from, to) = (row(&texts, "General:"), row(&texts, "Stroke:"));
    let mut y = from.y;
    while y <= to.y {
        let (texts, _) = description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(egui::pos2(from.x, y))]);
        assert!(painted(&texts, general) || painted(&texts, stroke), "a sentence shows with the pointer at y = {y}");
        y += 1.0;
    }
    // Moving down onto the line keeps the sentence there to read.
    let line = description_line_rects(&ctx).line;
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(line.center())]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&texts, stroke), "the sentence stays while the pointer is on the line: {texts:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerGone]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(!painted(&texts, stroke), "the sentence goes with the pointer");
    assert_eq!(preferences_window(&ctx).size(), size);
}

/// With nothing under the pointer, the row whose control has the keyboard focus is described, and a
/// screen reader gets the same sentence as that control's description.
#[test]
fn keyboard_focus_on_a_units_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Units"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let general = descriptions::description("unitsGeneral").expect("Units › General has a sentence");
    assert!(ctx.memory(|m| m.has_focus(search_edit_id())), "the search field has the focus on open");
    assert!(!painted(&texts, general));
    // Tab from the search field through the category list to the first Units control.
    let mut shown = None;
    for _ in 0..40 {
        let (texts, described) = description_frame(&mut a, &ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if painted(&texts, general) {
            shown = Some(described);
            break;
        }
    }
    let described = shown.expect("Tab reaches the General row and its sentence shows");
    assert_eq!(described.as_deref(), Some(general), "the focused control carries the sentence");
}

/// The field list with the description line under it, and the description line, in the last frame.
struct DescriptionRects {
    column: egui::Rect,
    line: egui::Rect,
}

fn description_line_rects(ctx: &egui::Context) -> DescriptionRects {
    let (column, line) =
        ctx.data(|m| m.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::unique("prefs-description-rects"))).expect("the description line is shown");
    DescriptionRects { column, line }
}

/// Scrolls the field list with the wheel until the last text `label` in `texts` lies wholly inside
/// the list, above the spacing and the gap over the description line, and stays put there (the
/// wheel scrolls smoothly over several frames) → the texts of the last frame.
fn scroll_into_view(a: &mut VectorcraftApp, ctx: &egui::Context, mut texts: Vec<(String, egui::Rect)>, label: &str) -> Vec<(String, egui::Rect)> {
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -40.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    let at = |texts: &[(String, egui::Rect)]| texts.iter().rev().find(|(t, _)| t == label).map(|(_, r)| *r);
    let inside = |r: Option<egui::Rect>| {
        let DescriptionRects { column, line } = description_line_rects(ctx);
        let bottom = line.min.y - DESCRIPTION_GAP - ctx.global_style().spacing.item_spacing.y;
        let list = egui::Rect::from_min_max(column.min, egui::pos2(column.max.x, bottom));
        r.is_some_and(|r| list.contains_rect(r))
    };
    for _ in 0..80 {
        let shown = inside(at(&texts));
        let next = description_frame(a, ctx, if shown { vec![] } else { vec![wheel.clone()] }).0;
        let still = shown && at(&next) == at(&texts);
        texts = next;
        if still {
            break;
        }
    }
    assert!(inside(at(&texts)), "`{label}` lies wholly inside the field list: {texts:?}");
    texts
}

/// A row with no sentence leaves the description line empty, also after another category's row
/// was described, and the dialog keeps its size.
#[test]
fn a_row_without_a_description_leaves_the_line_empty() {
    let mut a = app();
    open(&mut a, Some("Units"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let at = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let general = descriptions::description("unitsGeneral").expect("Units › General has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(at(&texts, "General:"))]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&texts, general), "the General row's sentence shows: {texts:?}");
    // Go to the first other category that still has a row with no sentence (Slices when none is
    // left) and point at that row.
    let bare = settings().find(|s| s.category != "Units" && descriptions::description(s.key).is_none());
    let category = bare.map_or("Slices", |s| s.category);
    let button = at(&texts, category);
    let press = |pressed| egui::Event::PointerButton { pos: button, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(button), press(true), press(false)]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let size = preferences_window(&ctx).size();
    let line = description_line_rects(&ctx).line;
    let empty = |texts: &[(String, egui::Rect)], when: &str| {
        let drawn: Vec<_> = texts.iter().filter(|(_, r)| r.intersects(line)).collect();
        assert!(drawn.is_empty(), "{when}: nothing is drawn on the line: {drawn:?}");
    };
    // The Units sentence does not follow the pointer onto the line of another category.
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(line.center())]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    empty(&after, "on the line after changing category");
    if bare.is_some() {
        // A checkbox shows its label as it is; other rows show it with a colon.
        let (label, pos) = settings()
            .filter(|s| s.category == category && descriptions::description(s.key).is_none())
            .find_map(|s| texts.iter().find(|(t, _)| *t == s.label || *t == format!("{}:", s.label)).map(|(_, r)| (s.label, r.center())))
            .unwrap_or_else(|| panic!("no row of {category} without a sentence is drawn: {texts:?}"));
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(pos)]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        empty(&after, &format!("on {label}"));
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(line.center())]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        empty(&after, &format!("on the line after {label}"));
    }
    assert_eq!(preferences_window(&ctx).size(), size);
}

/// Every setting the General, Selection & Anchor Display, Guides & Grid, Smart Guides, Slices,
/// Hyphenation, Plug-ins & Scratch Disks, User Interface, Performance, File Handling, Clipboard
/// Handling, Appearance of Black and Devices categories show on this platform, the Smart Guides box
/// included, has exactly one sentence, and no key has two.
#[test]
fn every_setting_from_general_to_devices_has_one_description() {
    let mut bad = vec![];
    let categories = [
        "General",
        "Selection & Anchor Display",
        "Guides & Grid",
        "Smart Guides",
        "Slices",
        "Hyphenation",
        "Plug-ins & Scratch Disks",
        "User Interface",
        "Performance",
        "File Handling",
        "Clipboard Handling",
        "Appearance of Black",
        "Devices",
    ];
    for s in settings().filter(|s| categories.contains(&s.category)) {
        match descriptions::DESCRIPTIONS.iter().filter(|(k, _)| *k == s.key).count() {
            1 => {}
            n => bad.push(format!("`{}` ({}) has {n} sentences", s.key, s.label)),
        }
        if descriptions::description(s.key).is_some_and(|d| d.trim().is_empty()) {
            bad.push(format!("`{}` has an empty sentence", s.key));
        }
    }
    for (i, (key, _)) in descriptions::DESCRIPTIONS.iter().enumerate() {
        if descriptions::DESCRIPTIONS[..i].iter().any(|(k, _)| k == key) {
            bad.push(format!("`{key}` is listed twice"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Cmd/Ctrl-wheel scrolls sideways with Zoom with Mouse Wheel on as well as off
/// (`canvas::tests::zoom_with_mouse_wheel_preference`), so the sentence for Scroll Sideways with
/// Cmd/Ctrl + Wheel does not make the sideways scroll depend on that setting.
#[test]
fn the_sideways_scroll_sentence_does_not_depend_on_zoom_with_mouse_wheel() {
    let zoom = settings().find(|s| s.key == "zoomWithMouseWheel").expect("General has Zoom with Mouse Wheel").label;
    let sideways = descriptions::description("ctrlWheelScrollsSideways").expect("Scroll Sideways with Cmd/Ctrl + Wheel has a sentence");
    assert!(!sideways.contains(zoom.trim_end_matches(':')), "`{sideways}` names `{zoom}`");
    assert!(sideways.contains("Cmd/Ctrl") && sideways.contains("sideways"), "`{sideways}` describes the sideways scroll");
}

/// The pointer over General's Contextual Task Bar row and over its Keyboard Increment row shows
/// each row's sentence.
#[test]
fn hovering_a_general_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("General"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let task_bar = descriptions::description("__taskBar").expect("Enable Contextual Task Bar has a sentence");
    let increment = descriptions::description("keyboardIncrement").expect("Keyboard Increment has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Enable Contextual Task Bar"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, task_bar) && !painted(&after, increment), "the task bar row's sentence shows: {after:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Keyboard Increment:"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, increment) && !painted(&after, task_bar), "the Keyboard Increment row's sentence shows: {after:?}");
}

/// Tab from the search field reaches General's first control, the Contextual Task Bar box: the line
/// shows its sentence and a screen reader gets it as the control's description.
#[test]
fn keyboard_focus_on_a_general_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("General"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    let task_bar = descriptions::description("__taskBar").expect("Enable Contextual Task Bar has a sentence");
    let mut shown = None;
    for _ in 0..40 {
        let (texts, described) = description_frame(&mut a, &ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if painted(&texts, task_bar) {
            shown = Some(described);
            break;
        }
    }
    let described = shown.expect("Tab reaches the Contextual Task Bar row and its sentence shows");
    assert_eq!(described.as_deref(), Some(task_bar), "the focused control carries the sentence");
}

/// Every setting the Type category shows has exactly one sentence.
#[test]
fn every_type_setting_has_one_description() {
    let mut bad = vec![];
    let mut count = 0;
    for s in settings().filter(|s| s.category == "Type") {
        count += 1;
        match descriptions::DESCRIPTIONS.iter().filter(|(k, _)| *k == s.key).count() {
            1 => {}
            n => bad.push(format!("`{}` ({}) has {n} sentences", s.key, s.label)),
        }
        if descriptions::description(s.key).is_some_and(|d| d.trim().is_empty()) {
            bad.push(format!("`{}` has an empty sentence", s.key));
        }
    }
    assert!(count > 0, "the Type category shows settings");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The pointer over Type's Size/Leading row and over its Show East Asian Options box shows each
/// row's sentence.
#[test]
fn hovering_a_type_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Type"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let size = descriptions::description("typeSizeIncrement").expect("Size/Leading has a sentence");
    let east_asian = descriptions::description("showEastAsianOptions").expect("Show East Asian Options has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Size/Leading:"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, size) && !painted(&after, east_asian), "the Size/Leading row's sentence shows: {after:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Show East Asian Options"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, east_asian) && !painted(&after, size), "the East Asian row's sentence shows: {after:?}");
}

/// Tab from the search field reaches Type's first control, the Size/Leading field: the line shows
/// its sentence and a screen reader gets it as the control's description.
#[test]
fn keyboard_focus_on_a_type_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Type"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    let size = descriptions::description("typeSizeIncrement").expect("Size/Leading has a sentence");
    let mut shown = None;
    for _ in 0..40 {
        let (texts, described) = description_frame(&mut a, &ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if painted(&texts, size) {
            shown = Some(described);
            break;
        }
    }
    let described = shown.expect("Tab reaches the Size/Leading row and its sentence shows");
    assert_eq!(described.as_deref(), Some(size), "the focused control carries the sentence");
}

/// The pointer over Selection & Anchor Display's Tolerance row and over its Highlight anchors on
/// mouse over row shows each row's sentence.
#[test]
fn hovering_a_selection_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Selection & Anchor Display"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let tolerance = descriptions::description("selectionTolerance").expect("Tolerance has a sentence");
    let highlight = descriptions::description("highlightAnchorsOnHover").expect("Highlight anchors on mouse over has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Tolerance:"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, tolerance) && !painted(&after, highlight), "the Tolerance row's sentence shows: {after:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Highlight anchors on mouse over"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, highlight) && !painted(&after, tolerance), "the Highlight anchors row's sentence shows: {after:?}");
}

/// Tab from the search field reaches Selection & Anchor Display's first control, the Tolerance
/// field: the line shows its sentence and a screen reader gets it as the control's description.
#[test]
fn keyboard_focus_on_a_selection_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Selection & Anchor Display"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    let tolerance = descriptions::description("selectionTolerance").expect("Tolerance has a sentence");
    let mut shown = None;
    for _ in 0..40 {
        let (texts, described) = description_frame(&mut a, &ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if painted(&texts, tolerance) {
            shown = Some(described);
            break;
        }
    }
    let described = shown.expect("Tab reaches the Tolerance row and its sentence shows");
    assert!(described.iter().any(|d| d == tolerance), "the focused control carries the sentence: {described:?}");
}

/// The pointer over Guides & Grid's Snap to Grid row and over its Gridline every row shows each
/// row's sentence.
#[test]
fn hovering_a_guides_and_grid_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Guides & Grid"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let snap = descriptions::description("__snapToGrid").expect("Snap to Grid has a sentence");
    let every = descriptions::description("gridlineEvery").expect("Gridline every has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Snap to Grid"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, snap) && !painted(&after, every), "the Snap to Grid row's sentence shows: {after:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Gridline every:"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, every) && !painted(&after, snap), "the Gridline every row's sentence shows: {after:?}");
}

/// Tab from the search field reaches Guides & Grid's first control, the Snap to Grid box: the line
/// shows its sentence and a screen reader gets it as the control's description.
#[test]
fn keyboard_focus_on_a_guides_and_grid_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Guides & Grid"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    let snap = descriptions::description("__snapToGrid").expect("Snap to Grid has a sentence");
    let mut shown = None;
    for _ in 0..40 {
        let (texts, described) = description_frame(&mut a, &ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if painted(&texts, snap) {
            shown = Some(described);
            break;
        }
    }
    let described = shown.expect("Tab reaches the Snap to Grid row and its sentence shows");
    assert!(described.iter().any(|d| d == snap), "the focused control carries the sentence: {described:?}");
}

/// The pointer over Smart Guides' own Smart Guides row and over its Alignment Guides row shows each
/// row's sentence.
#[test]
fn hovering_a_smart_guides_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Smart Guides"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let smart = descriptions::description("__smartGuides").expect("Smart Guides has a sentence");
    let alignment = descriptions::description("alignmentGuides").expect("Alignment Guides has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Smart Guides (View → Smart Guides)"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, smart) && !painted(&after, alignment), "the Smart Guides row's sentence shows: {after:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Alignment Guides"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, alignment) && !painted(&after, smart), "the Alignment Guides row's sentence shows: {after:?}");
}

/// Tab from the search field reaches Smart Guides' first control, the Smart Guides box: the line
/// shows its sentence and a screen reader gets it as the control's description.
#[test]
fn keyboard_focus_on_a_smart_guides_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Smart Guides"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    let smart = descriptions::description("__smartGuides").expect("Smart Guides has a sentence");
    let mut shown = None;
    for _ in 0..40 {
        let (texts, described) = description_frame(&mut a, &ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if painted(&texts, smart) {
            shown = Some(described);
            break;
        }
    }
    let described = shown.expect("Tab reaches the Smart Guides row and its sentence shows");
    assert!(described.iter().any(|d| d == smart), "the focused control carries the sentence: {described:?}");
}

/// The pointer over Slices' Show Slice Numbers row and over its Line Color row shows each row's
/// sentence.
#[test]
fn hovering_a_slices_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Slices"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let numbers = descriptions::description("showSliceNumbers").expect("Show Slice Numbers has a sentence");
    let color = descriptions::description("sliceLineColor").expect("Line Color has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Show Slice Numbers"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, numbers) && !painted(&after, color), "the Show Slice Numbers row's sentence shows: {after:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Line Color:"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&after, color) && !painted(&after, numbers), "the Line Color row's sentence shows: {after:?}");
}

/// Tab from the search field reaches Slices' first control, the Show Slice Numbers box: the line
/// shows its sentence and a screen reader gets it as the control's description.
#[test]
fn keyboard_focus_on_a_slices_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Slices"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    let numbers = descriptions::description("showSliceNumbers").expect("Show Slice Numbers has a sentence");
    let mut shown = None;
    for _ in 0..40 {
        let (texts, described) = description_frame(&mut a, &ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if painted(&texts, numbers) {
            shown = Some(described);
            break;
        }
    }
    let described = shown.expect("Tab reaches the Show Slice Numbers row and its sentence shows");
    assert!(described.iter().any(|d| d == numbers), "the focused control carries the sentence: {described:?}");
}

/// Whether `sentence` is one of `texts` and is drawn on the description line of the frame that
/// painted them.
fn painted_on_line(ctx: &egui::Context, texts: &[(String, egui::Rect)], sentence: &str) -> bool {
    let line = description_line_rects(ctx).line;
    texts.iter().any(|(t, r)| t == sentence && r.intersects(line))
}

/// Tab from the search field, which must have the keyboard, until each of `sentences` has shown on
/// the description line. Other controls may come between them, but a sentence that shows before the
/// ones ahead of it in `sentences` fails the test. → For each sentence, the description of the focused
/// accessibility node and the focused widget when it first showed.
fn tab_to_each(a: &mut VectorcraftApp, ctx: &egui::Context, sentences: &[&str]) -> Vec<(Option<String>, Option<egui::Id>)> {
    assert!(ctx.memory(|m| m.has_focus(search_edit_id())), "the search field has the keyboard before the first Tab");
    let mut first = vec![];
    for _ in 0..40 {
        if first.len() == sentences.len() {
            break;
        }
        let (texts, described) = description_frame(a, ctx, key_press(egui::Key::Tab, egui::Modifiers::NONE));
        if let Some(i) = sentences.iter().position(|s| painted_on_line(ctx, &texts, s)) {
            assert!(i <= first.len(), "Tab reaches {:?} before {:?}", sentences[i], sentences[first.len()]);
            if i == first.len() {
                first.push((described, ctx.memory(|m| m.focused())));
            }
        }
    }
    if let Some(missing) = sentences.get(first.len()) {
        panic!("Tab does not reach the row of {missing:?}");
    }
    first
}

/// The pointer over Hyphenation's Default Language row and over its Exceptions row shows each
/// row's sentence.
#[test]
fn hovering_a_hyphenation_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Hyphenation"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t.starts_with(label)).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let language = descriptions::description("hyphenationLanguage").expect("Default Language has a sentence");
    let exceptions = descriptions::description("hyphenationExceptions").expect("Exceptions has a sentence");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Default Language:"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted_on_line(&ctx, &after, language) && !painted(&after, exceptions), "the Default Language row's sentence shows: {after:?}");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, "Exceptions"))]);
    let (after, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted_on_line(&ctx, &after, exceptions) && !painted(&after, language), "the Exceptions row's sentence shows: {after:?}");
}

/// Tab from the search field reaches Hyphenation's Default Language menu and then its Exceptions
/// field, in that order and before the focus comes back to the search field: the line shows each
/// one's sentence in turn, and a screen reader gets it as the focused control's description.
#[test]
fn keyboard_focus_on_a_hyphenation_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Hyphenation"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    assert_tab_order(&mut a, &ctx, &["hyphenationLanguage", "hyphenationExceptions"]);
}

/// The pointer over each row of Plug-ins & Scratch Disks, Additional Plug-ins Folder, Plug-in
/// Instruction Budget, Primary and Secondary, shows that row's sentence and none of the others.
#[test]
fn hovering_a_plugins_and_scratch_disks_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Plug-ins & Scratch Disks"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t.starts_with(label)).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let rows = [
        ("Additional Plug-ins Folder:", "pluginsFolder"),
        ("Plug-in Instruction", "pluginInstructionBudget"),
        ("Primary:", "scratchPrimary"),
        ("Secondary:", "scratchSecondary"),
    ];
    let sentences: Vec<_> = rows.iter().map(|(_, key)| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    for (i, (label, _)) in rows.iter().enumerate() {
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, label))]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        for (j, sentence) in sentences.iter().enumerate() {
            let shown = if i == j { painted_on_line(&ctx, &after, sentence) } else { painted(&after, sentence) };
            assert_eq!(shown, i == j, "pointer over `{label}`, sentence {j}: {after:?}");
        }
    }
}

/// Tab from the search field reaches the Additional Plug-ins Folder field, then Primary, then
/// Secondary: the line shows each one's sentence in turn, and a screen reader gets it as the focused
/// field's description. With a folder picker, Choose… shares the folder's row, and the folder's
/// text field still has the keyboard when its sentence first shows.
#[test]
fn keyboard_focus_on_a_plugins_and_scratch_disks_row_shows_its_description() {
    let keys = ["pluginsFolder", "scratchPrimary", "scratchSecondary"];
    let sentences: Vec<_> = keys.iter().map(|key| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    for picker in [false, true] {
        let mut a = app();
        if picker {
            a.services.pick_folder = Some(Box::new(|| None));
        }
        open(&mut a, Some("Plug-ins & Scratch Disks"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install_fonts(&ctx);
        description_frame(&mut a, &ctx, vec![]);
        let (texts, _) = description_frame(&mut a, &ctx, vec![]);
        assert_eq!(texts.iter().any(|(t, _)| t == "Choose…"), picker, "Choose… shows only with a folder picker: {texts:?}");
        let visits = tab_to_each(&mut a, &ctx, &sentences);
        for ((described, focused), (key, sentence)) in visits.iter().zip(keys.iter().zip(&sentences)) {
            let field = focused.is_some_and(|id| egui::text_edit::TextEditState::load(&ctx, id).is_some());
            assert!(field, "the `{key}` text field has the keyboard (picker {picker})");
            assert!(described.iter().any(|d| d == sentence), "the focused `{key}` field carries the sentence (picker {picker}): {described:?}");
        }
    }
}

/// Tab from the search field reaches the Additional Plug-ins Folder field, the Plug-in Instruction
/// Budget menu, then Primary, then Secondary: the line shows each one's sentence in turn, and a
/// screen reader gets it as the focused control's description.
#[test]
fn keyboard_focus_on_the_plugin_instruction_budget_menu_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Plug-ins & Scratch Disks"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    assert_tab_order(&mut a, &ctx, &["pluginsFolder", "pluginInstructionBudget", "scratchPrimary", "scratchSecondary"]);
}

/// System Title Bar and VectorCraft Title Bar on Every Desktop, which only some platforms show,
/// have a sentence on every platform.
#[test]
fn the_title_bar_settings_have_a_description_on_every_platform() {
    for key in ["systemTitleBar", "appTitleBar"] {
        assert!(PREF_SPECS.iter().any(|s| s.key == key && s.category == "User Interface"), "`{key}` is a User Interface setting");
        assert!(descriptions::description(key).is_some(), "`{key}` has a sentence");
    }
}

/// The pointer over each User Interface row that every platform shows, from Brightness to
/// Language, shows that row's sentence and no other row's.
#[test]
fn hovering_a_user_interface_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("User Interface"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    // Only rows the list shows whole: the pointer goes to the center of a row it can reach.
    let texts = whole_texts_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let rows = [
        ("Brightness:", "uiBrightness"),
        ("Color Scheme:", "uiColorScheme"),
        ("Accent Color:", "uiAccent"),
        ("Canvas Color:", "canvasColor"),
        ("Auto-Collapse Iconic Panels", "autoCollapseIconPanels"),
        ("Show Tool Group Labels", "toolGroupLabels"),
        ("Open Documents As Tabs", "openDocumentsAsTabs"),
        ("Large Tabs", "largeTabs"),
        ("Scale:", "uiScaling"),
        ("Scale Cursor Proportional to UI", "scaleCursorWithUi"),
        ("Language:", "interfaceLanguage"),
    ];
    let sentences: Vec<_> = rows.iter().map(|(_, key)| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    // The Language row is under the bottom of the list: the wheel scrolls it into view.
    let mut texts = texts;
    for (i, (label, _)) in rows.iter().enumerate() {
        texts = scroll_into_view(&mut a, &ctx, texts, label);
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, label))]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        for (j, sentence) in sentences.iter().enumerate() {
            assert_eq!(painted(&after, sentence), i == j, "pointer over `{label}`, sentence {j}: {after:?}");
        }
        texts = after;
    }
}

/// Tab from the search field reaches the Brightness menu, then Color Scheme, Accent Color, Canvas
/// Color and Auto-Collapse Iconic Panels: the line shows each one's sentence in turn, and a screen
/// reader gets it as the focused control's description.
#[test]
fn keyboard_focus_on_a_user_interface_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("User Interface"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    assert_tab_order(&mut a, &ctx, &["uiBrightness", "uiColorScheme", "uiAccent", "canvasColor", "autoCollapseIconPanels"]);
}

/// Linux Display Server, which only Linux shows, and Use Monitor Profile for Canvas, which the
/// web version does not show, have a sentence on every platform.
#[test]
fn the_display_server_and_monitor_profile_settings_have_a_description_on_every_platform() {
    for key in ["linuxDisplayServer", "monitorProfileDisplay"] {
        assert!(PREF_SPECS.iter().any(|s| s.key == key && s.category == "Performance"), "`{key}` is a Performance setting");
        assert!(descriptions::description(key).is_some(), "`{key}` has a sentence");
    }
}

/// The pointer over each Performance row this platform shows, from GPU Performance to Use Monitor
/// Profile for Canvas on the desktop, Low-Latency Drawing included there, shows that row's sentence
/// and no other row's.
#[test]
fn hovering_a_performance_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Performance"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    // The GPU Performance section's heading has the checkbox's label too: the checkbox comes after it.
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().rev().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let mut rows = vec![("GPU Performance", "gpuPerformance"), ("Animated Zoom", "animatedZoom"), ("Graphics Processor:", "gpuPreference")];
    if !cfg!(target_arch = "wasm32") {
        rows.push(("Low-Latency Drawing", "lowLatencyDrawing"));
    }
    rows.extend([
        ("History States:", "historyStates"),
        ("Real-time Drawing and Editing", "realTimeDrawing"),
        ("Render Threads (-1 = Automatic):", "renderThreads"),
    ]);
    if cfg!(target_os = "linux") {
        rows.push(("Linux Display Server:", "linuxDisplayServer"));
    }
    if !cfg!(target_arch = "wasm32") {
        rows.push(("Use Monitor Profile for Canvas", "monitorProfileDisplay"));
    }
    let sentences: Vec<_> = rows.iter().map(|(_, key)| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    // The last rows can be under the bottom of the list: the wheel scrolls them into view.
    let mut texts = texts;
    for (i, (label, _)) in rows.iter().enumerate() {
        texts = scroll_into_view(&mut a, &ctx, texts, label);
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, label))]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        for (j, sentence) in sentences.iter().enumerate() {
            assert_eq!(painted(&after, sentence), i == j, "pointer over `{label}`, sentence {j}: {after:?}");
        }
        texts = after;
    }
}

/// Tab from the search field reaches the GPU Performance checkbox, then Animated Zoom, the
/// Graphics Processor menu, the Low-Latency Drawing checkbox on the desktop and the History States
/// field: the line shows each one's sentence in turn, and a screen reader gets it as the focused
/// control's description.
#[test]
fn keyboard_focus_on_a_performance_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Performance"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    let mut keys = vec!["gpuPerformance", "animatedZoom", "gpuPreference"];
    if !cfg!(target_arch = "wasm32") {
        keys.push("lowLatencyDrawing");
    }
    keys.push("historyStates");
    assert_tab_order(&mut a, &ctx, &keys);
}

/// The pointer over each of the 15 File Handling rows, from Enable Background Save to Use
/// Compression, shows that row's sentence and no other row's.
#[test]
fn hovering_a_file_handling_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("File Handling"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let rows = [
        ("Enable Background Save", "backgroundSave"),
        ("Enable Background Export", "backgroundExport"),
        ("Automatically Save Recovery Data", "autosaveRecovery"),
        ("Every (minutes):", "autosaveInterval"),
        ("Folder:", "recoveryFolder"),
        ("Turn off Data Recovery for complex documents", "recoveryOffForComplex"),
        ("Number of Recent Files to Display:", "recentFilesCount"),
        ("Use Low Resolution Proxy for Linked EPS", "lowResProxyEps"),
        ("Display Bitmaps as Anti-aliased Images in Pixel Preview", "antiAliasedBitmaps"),
        ("Update Links:", "updateLinks"),
        ("PDF Text:", "pdfTextAs"),
        ("Templates Folder:", "templatesFolder"),
        ("Use System Defaults for Edit Original", "editOriginalSystemDefaults"),
        ("Mark Older Files as [Converted] When Opened", "appendConverted"),
        ("Use Compression", "useCompression"),
    ];
    let sentences: Vec<_> = rows.iter().map(|(_, key)| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    // The last rows can be under the bottom of the list: the wheel scrolls them into view.
    let mut texts = texts;
    for (i, (label, _)) in rows.iter().enumerate() {
        texts = scroll_into_view(&mut a, &ctx, texts, label);
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, label))]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        for (j, sentence) in sentences.iter().enumerate() {
            assert_eq!(painted(&after, sentence), i == j, "pointer over `{label}`, sentence {j}: {after:?}");
        }
        texts = after;
    }
}

/// Tab from the search field reaches the Enable Background Save and Enable Background Export
/// checkboxes, Automatically Save Recovery Data and the Every (minutes) field: the line shows each
/// one's sentence in turn, and a screen reader gets it as the focused control's description.
#[test]
fn keyboard_focus_on_a_file_handling_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("File Handling"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    assert_tab_order(&mut a, &ctx, &["backgroundSave", "backgroundExport", "autosaveRecovery", "autosaveInterval"]);
}

/// The pointer over each of the five Clipboard Handling rows, from Include SVG Code to When
/// pasting text, shows that row's sentence and no other row's.
#[test]
fn hovering_a_clipboard_handling_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Clipboard Handling"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let rows = [
        ("Include SVG Code", "copyAsSvg"),
        ("PDF", "copyAsPdf"),
        ("Legacy vector clipboard (no transparency)", "copyAicb"),
        ("Legacy vector clipboard:", "aicbMode"),
        ("When pasting text:", "pasteTextFormatting"),
    ];
    let sentences: Vec<_> = rows.iter().map(|(_, key)| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    for (i, (label, _)) in rows.iter().enumerate() {
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, label))]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        for (j, sentence) in sentences.iter().enumerate() {
            assert_eq!(painted(&after, sentence), i == j, "pointer over `{label}`, sentence {j}: {after:?}");
        }
    }
}

/// Tab from the search field reaches the Include SVG Code, PDF and Legacy vector clipboard (no
/// transparency) checkboxes, then the Legacy vector clipboard and When pasting text menus: the line
/// shows each one's sentence in turn, and a screen reader gets it as the focused control's
/// description.
#[test]
fn keyboard_focus_on_a_clipboard_handling_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Clipboard Handling"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    assert_tab_order(&mut a, &ctx, &["copyAsSvg", "copyAsPdf", "copyAicb", "aicbMode", "pasteTextFormatting"]);
}

/// The pointer over each of the two Appearance of Black rows, On Screen and Printing / Exporting,
/// shows that row's sentence and not the other row's.
#[test]
fn hovering_an_appearance_of_black_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Appearance of Black"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let rows = [("On Screen:", "blackOnScreen"), ("Printing / Exporting:", "blackOutput")];
    let sentences: Vec<_> = rows.iter().map(|(_, key)| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    for (i, (label, _)) in rows.iter().enumerate() {
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, label))]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        for (j, sentence) in sentences.iter().enumerate() {
            assert_eq!(painted(&after, sentence), i == j, "pointer over `{label}`, sentence {j}: {after:?}");
        }
    }
}

/// Tab from the search field reaches the On Screen and Printing / Exporting menus: the line shows
/// each one's sentence in turn, and a screen reader gets it as the focused control's description.
#[test]
fn keyboard_focus_on_an_appearance_of_black_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Appearance of Black"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    assert_tab_order(&mut a, &ctx, &["blackOnScreen", "blackOutput"]);
}

/// The pointer over each of the two Devices rows, Enable Touch Workspace and Enable Touch Gestures,
/// shows that row's sentence and not the other row's.
#[test]
fn hovering_a_devices_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Devices"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let row = |texts: &[(String, egui::Rect)], label: &str| {
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
    };
    let rows = [("Enable Touch Workspace", "touchWorkspace"), ("Enable Touch Gestures", "touchGestures")];
    let sentences: Vec<_> = rows.iter().map(|(_, key)| descriptions::description(key).unwrap_or_else(|| panic!("`{key}` has a sentence"))).collect();
    for (i, (label, _)) in rows.iter().enumerate() {
        description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(row(&texts, label))]);
        let (after, _) = description_frame(&mut a, &ctx, vec![]);
        for (j, sentence) in sentences.iter().enumerate() {
            assert_eq!(painted(&after, sentence), i == j, "pointer over `{label}`, sentence {j}: {after:?}");
        }
    }
}

/// Tab from the search field reaches the Enable Touch Workspace and Enable Touch Gestures boxes: the
/// line shows each one's sentence in turn, and a screen reader gets it as the focused control's
/// description.
#[test]
fn keyboard_focus_on_a_devices_row_shows_its_description() {
    let mut a = app();
    open(&mut a, Some("Devices"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    description_frame(&mut a, &ctx, vec![]);
    assert_tab_order(&mut a, &ctx, &["touchWorkspace", "touchGestures"]);
}

/// A sentence shown when the dialog closed does not come back when it opens again with the
/// pointer away from the rows.
#[test]
fn opening_the_dialog_again_starts_with_an_empty_line() {
    let mut a = app();
    open(&mut a, Some("Units"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    description_frame(&mut a, &ctx, vec![]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    let general = descriptions::description("unitsGeneral").expect("Units › General has a sentence");
    let at = texts.iter().find(|(t, _)| t == "General:").map(|(_, r)| r.center()).expect("the General row is shown");
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(at)]);
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(painted(&texts, general), "the General row's sentence shows: {texts:?}");
    let line = description_line_rects(&ctx).line;
    a.ui.dialog = None;
    description_frame(&mut a, &ctx, vec![egui::Event::PointerMoved(line.center())]);
    description_frame(&mut a, &ctx, vec![]);
    open(&mut a, Some("Units"));
    let (texts, _) = description_frame(&mut a, &ctx, vec![]);
    assert!(!painted(&texts, general), "the old sentence does not come back: {texts:?}");
}

/// The field list and the description line under it together take the height the field list
/// alone took before the line, in every category, so the dialog keeps its size.
#[test]
fn the_description_line_keeps_the_fields_column_height() {
    let mut a = app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for cat in PREF_CATEGORIES {
        open(&mut a, Some(*cat));
        description_frame(&mut a, &ctx, vec![]);
        description_frame(&mut a, &ctx, vec![]);
        let kept = description_line_rects(&ctx);
        assert!((kept.column.height() - 400.0).abs() < 0.01, "{cat}: the column is {} tall, not 400", kept.column.height());
        assert!((kept.line.height() - DESCRIPTION_HEIGHT).abs() < 0.01, "{cat}: the line is {} tall", kept.line.height());
    }
}

/// Each sentence describes a setting the dialog shows on this platform or, like System Title Bar,
/// on another one, and every complete catalog translates it into at most the three lines the
/// description line holds.
#[test]
fn every_description_names_a_setting_and_fits_in_every_language() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut bad = vec![];
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        for (key, sentence) in descriptions::DESCRIPTIONS {
            if !settings().any(|s| s.key == *key) && !PREF_SPECS.iter().any(|s| s.key == *key) {
                bad.push(format!("`{key}` is not a setting the dialog shows"));
            }
            for lang in crate::i18n::Lang::all() {
                let text = crate::i18n::tr(lang, sentence);
                if lang != crate::i18n::Lang::EN && text == *sentence {
                    bad.push(format!("{}: no row for {sentence:?}", lang.code()));
                }
                let mut job = egui::text::LayoutJob::simple(text.to_string(), egui::FontId::proportional(11.5), egui::Color32::WHITE, 530.0);
                job.wrap.max_rows = 3;
                let galley = ui.painter().layout_job(job);
                if galley.elided || galley.size().y > DESCRIPTION_HEIGHT {
                    bad.push(format!("{}: {text:?} takes more than three lines", lang.code()));
                }
            }
        }
    });
    out.textures_delta.clear();
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
