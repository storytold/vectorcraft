//! Stored System mode, OS reports, native inheritance and independent document/canvas colours.
use egui::{Context, RawInput, Theme, ThemePreference, ViewportCommand};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::theme::{Brightness, Tokens};
use crate::{VectorcraftApp, prefs_dialog};

fn frame(app: &mut VectorcraftApp, ctx: &Context, os: Option<Theme>) -> egui::FullOutput {
    let mut out = ctx.run_ui(RawInput { system_theme: os, ..Default::default() }, |ui| app.logic(ui.ctx()));
    out.textures_delta.clear();
    out
}

#[test]
fn system_theme_tracks_reports_without_pinning_or_replacing_saved_choice() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("window.brightness", json!({"brightness": "System"})).unwrap();
    app.run("prefs.set", json!({"key": "canvasColor", "value": "white"})).unwrap();
    let document = serde_json::to_value(&app.session.active().unwrap().doc).unwrap();
    let ctx = Context::default();
    for os in [Some(Theme::Light), Some(Theme::Dark), Some(Theme::Light), None] {
        let out = frame(&mut app, &ctx, os);
        let expected = Tokens::for_brightness(if os == Some(Theme::Light) { Brightness::Light } else { Brightness::MediumDark });
        let tokens = Tokens::get(&ctx);
        assert_eq!(tokens.panel, expected.panel);
        assert_eq!(tokens.app_bar, expected.app_bar);
        assert_eq!(tokens.tab_strip, expected.tab_strip);
        assert_eq!(tokens.pasteboard, egui::Color32::WHITE);
        assert_eq!(app.ui.brightness, Brightness::System);
        assert_eq!(app.session.prefs.ui_brightness, "system");
        assert_eq!(serde_json::to_value(&app.session.active().unwrap().doc).unwrap(), document);
        assert_eq!(ctx.options(|o| o.theme_preference), ThemePreference::System);
        for theme in [Theme::Light, Theme::Dark] {
            assert_eq!(ctx.style_of(theme).visuals.panel_fill, expected.panel);
            assert_eq!(ctx.style_of(theme).visuals.window_fill, expected.panel);
        }
        let commands: Vec<_> = out
            .viewport_output
            .values()
            .flat_map(|v| &v.commands)
            .filter_map(|c| match c {
                ViewportCommand::SetTheme(t) => Some(*t),
                _ => None,
            })
            .collect();
        assert!(commands.iter().all(|t| *t == egui::SystemTheme::SystemDefault), "{commands:?}");
    }
    prefs_dialog::snapshot(&mut app);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let mut restarted = VectorcraftApp::new(Session::new(), Default::default());
    restarted.ui = serde_json::from_str(&saved).unwrap();
    prefs_dialog::restore(&mut restarted);
    frame(&mut restarted, &Context::default(), Some(Theme::Light));
    assert_eq!(restarted.ui.brightness, Brightness::System);
    assert_eq!(restarted.session.prefs.ui_brightness, "system");
    assert_eq!(restarted.session.prefs.canvas_color, "white");
}

#[test]
fn manual_themes_and_legacy_preferences_keep_their_palette() {
    assert_eq!(Brightness::default(), Brightness::MediumDark);
    for mode in Brightness::ALL.into_iter().filter(|b| *b != Brightness::System) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui = serde_json::from_value(json!({"brightness": mode})).unwrap();
        prefs_dialog::restore(&mut app);
        let ctx = Context::default();
        for os in [Some(Theme::Light), Some(Theme::Dark), None] {
            frame(&mut app, &ctx, os);
            let expected = Tokens::for_brightness(mode);
            assert_eq!(Tokens::get(&ctx).panel, expected.panel);
            assert_eq!(Tokens::get(&ctx).pasteboard, expected.pasteboard);
            assert_eq!(app.session.prefs.ui_brightness, mode.id());
            for theme in [Theme::Light, Theme::Dark] {
                assert_eq!(ctx.style_of(theme).visuals.panel_fill, expected.panel);
            }
        }
    }
}

#[test]
fn system_theme_is_selectable_in_preferences_and_native_appearance() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    prefs_dialog::open(&mut app, Some("User Interface"));
    app.ui.dialog.as_mut().unwrap().fields.insert("uiBrightness".into(), json!("system"));
    prefs_dialog::confirm(&mut app).unwrap();
    frame(&mut app, &Context::default(), Some(Theme::Light));
    let menu = crate::native_menu::mac_layout(&app, &crate::native_menu::from_tree(&app, &crate::menus::menu_tree()), crate::i18n::Lang::EN).bar;
    let items = menu.items();
    let appearance: Vec<_> = items.iter().filter(|it| it.command == Some("window.brightness")).collect();
    assert_eq!(appearance.len(), 5);
    assert_eq!(appearance.iter().filter(|it| it.checked == Some(true)).count(), 1);
    let system = appearance.iter().find(|it| it.params["brightness"] == "system").unwrap();
    assert_eq!(system.checked, Some(true));
    let spec = vectorcraft_engine::cmd::prefscmds::spec("uiBrightness").unwrap();
    let vectorcraft_engine::cmd::prefscmds::PrefKind::Choice(choices) = spec.kind else { panic!("brightness choices") };
    assert!(choices.contains(&("system", "System")));
}
