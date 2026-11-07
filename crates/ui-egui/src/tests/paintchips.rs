//! The Fill/Stroke proxies and chips (toolbar, Control bar, Properties), their swatch and mixer
//! popovers, the Recolor quick action and the panel shortcuts (M3.51).

use egui::{Color32, Event, Modifiers, PointerButton, Pos2, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, chrome, panels, shortcut_editor, shortcuts, toolbar};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// One headless frame of `draw` with `events` (and `modifiers` held); returns the painted shapes.
fn frame(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: Vec<Event>,
    modifiers: Modifiers,
    draw: fn(&mut VectorcraftApp, &mut egui::Ui),
) -> Vec<Shape> {
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 1000.0)));
    let mut out = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| draw(app, ui));
    out.textures_delta.clear();
    out.shapes.into_iter().map(|c| c.shape).collect()
}

/// Click at `at` with `modifiers` held (press and release in two frames), then settle a frame.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, modifiers: Modifiers, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    frame(app, ctx, vec![Event::PointerMoved(at), button(true)], modifiers, draw);
    frame(app, ctx, vec![button(false)], modifiers, draw);
    frame(app, ctx, vec![], Modifiers::NONE, draw);
}

/// Whether `shapes` paint a rectangle filled with `color`.
fn paints_rect(shapes: &[Shape], color: Color32) -> bool {
    shapes.iter().any(|s| match s {
        Shape::Rect(r) => r.fill == color,
        Shape::Vec(v) => paints_rect(v, color),
        _ => false,
    })
}

/// The Fill and Stroke chips (22 pt squares with a chevron): left to right in the Control bar,
/// top to bottom in the Properties panel.
fn chips(ctx: &egui::Context) -> (Rect, Rect) {
    let mut r: Vec<Rect> = ctx.viewport(|vp| {
        vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(38.0, 22.0)).map(|w| w.rect).collect()
    });
    r.sort_by(|a, b| (a.left() + a.top()).total_cmp(&(b.left() + b.top())));
    (r[0], r[1])
}

fn control_bar(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    chrome::control_bar(app, ui);
}

fn fill_hex(app: &VectorcraftApp) -> String {
    panels::current_paints(app).0.color().map(|c| c.to_hex()).unwrap_or_default()
}

#[test]
fn red_text_shows_a_red_toolbar_fill() {
    let mut app = app();
    app.run("text.create", json!({"x": 20, "y": 60, "text": "Red"})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let ctx = context();
    frame(&mut app, &ctx, vec![], Modifiers::NONE, toolbar::show);
    let shapes = frame(&mut app, &ctx, vec![], Modifiers::NONE, toolbar::show);
    assert!(paints_rect(&shapes, Color32::from_rgb(255, 0, 0)), "the toolbar's Fill proxy shows the type's red");
}

#[test]
fn the_chips_show_a_question_mark_for_mixed_fills() {
    let mut app = app();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let b = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
    let text = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, control_bar);
    assert!(!text(&mut app).lines().any(|l| l == "?"));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert!(text(&mut app).lines().any(|l| l == "?"), "the Fill chip shows ?");
    let props = crate::tests_labels::painted_text(&mut app, panels::properties::show);
    assert!(props.lines().any(|l| l == "?"), "so does the Properties panel's");
}

#[test]
fn a_popover_tile_click_sets_the_fill() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.toggleActive", json!({"fill": false})).unwrap();
    let ctx = context();
    frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    let (fill, stroke) = chips(&ctx);
    click(&mut app, &ctx, fill.center(), Modifiers::NONE, control_bar);
    assert!(app.session.fill_active, "the Fill chip brings its proxy forward");
    let tile = ctx.read_response(egui::Id::unique(("swatch-pop-tile", "Red"))).expect("the popover shows the swatches").rect;
    click(&mut app, &ctx, tile.center(), Modifiers::NONE, control_bar);
    assert_eq!(fill_hex(&app), "#ed1c24");
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Fill Color", "through paint.setFill");
    // The Stroke chip: its proxy comes forward; Shift-click shows the mixer instead of swatches.
    click(&mut app, &ctx, Pos2::new(1300.0, 900.0), Modifiers::NONE, control_bar);
    click(&mut app, &ctx, stroke.center(), Modifiers::SHIFT, control_bar);
    assert!(!app.session.fill_active);
    assert!(ctx.read_response(egui::Id::unique(("swatch-pop-tile", "Red"))).is_none(), "no swatches in the mixer");
    let text = {
        let mut t = String::new();
        for s in frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar) {
            if let Shape::Text(g) = s {
                t.push_str(g.galley.text());
                t.push('\n');
            }
        }
        t
    };
    assert!(text.contains("Recent Colors"), "the Color panel's body: {text}");
}

/// The Properties panel has the Control bar's Fill and Stroke chips and stroke weight, with
/// nothing selected too, where they set up the next object drawn (Discord feedback).
#[test]
fn the_properties_panel_has_the_control_bars_fill_and_stroke() {
    let mut app = app();
    let props = panels::properties::show;
    let text = crate::tests_labels::painted_text(&mut app, props);
    for s in ["Document", "Appearance", "Fill", "Stroke", "1 pt"] {
        assert!(text.lines().any(|l| l == s), "{s}: {text}");
    }
    let ctx = context();
    frame(&mut app, &ctx, vec![], Modifiers::NONE, props);
    let (fill, stroke) = chips(&ctx);
    assert!(stroke.top() > fill.bottom(), "Fill above Stroke");
    click(&mut app, &ctx, fill.center(), Modifiers::NONE, props);
    let tile = ctx.read_response(egui::Id::unique(("swatch-pop-tile", "Red"))).expect("the popover shows the swatches").rect;
    click(&mut app, &ctx, tile.center(), Modifiers::NONE, props);
    assert_eq!(fill_hex(&app), "#ed1c24", "the next object's fill");
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    assert_eq!(fill_hex(&app), "#ed1c24", "the new rectangle (selected) has it");
    // With the rectangle selected: the same chips, under the Transform section.
    click(&mut app, &ctx, Pos2::new(1300.0, 900.0), Modifiers::NONE, props);
    frame(&mut app, &ctx, vec![], Modifiers::NONE, props);
    let (fill, _) = chips(&ctx);
    click(&mut app, &ctx, fill.center(), Modifiers::NONE, props);
    let tile = ctx.read_response(egui::Id::unique(("swatch-pop-tile", "Black"))).expect("the popover shows the swatches").rect;
    click(&mut app, &ctx, tile.center(), Modifiers::NONE, props);
    assert_eq!(fill_hex(&app), "#000000");
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Fill Color");
}

#[test]
fn recolor_is_a_quick_action_for_multicolour_selections() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    let text = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, panels::properties::show);
    assert!(!text(&mut app).contains("Recolor"), "one colour");
    app.run("paint.setStroke", json!({"color": "#ff0000"})).unwrap();
    assert!(text(&mut app).contains("Recolor"), "white and red");
}

#[test]
fn the_shortcut_table_maps_f6_to_color() {
    assert_eq!(shortcut_editor::panel_shortcut("color"), Some("F6"));
    assert_eq!(shortcut_editor::panel_shortcut("transparency"), Some("Cmd+Shift+F10"));
    let all = shortcuts::all_shortcuts();
    let f6 = all.iter().find(|(sc, ..)| sc.logical_key == egui::Key::F6 && sc.modifiers == Modifiers::NONE).expect("F6");
    assert_eq!((f6.1, &f6.2), ("window.panel", &json!({"panel": "color"})));
    // The keys toggle their panels; Shift+F6 isn't taken by F6.
    let mut app = app();
    let ctx = context();
    let key =
        |key, modifiers| vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
    let press = |app: &mut VectorcraftApp, k, m| {
        let mut out = ctx.run_ui(egui::RawInput { events: key(k, m), ..Default::default() }, |ui| shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    };
    press(&mut app, egui::Key::F6, Modifiers::NONE);
    assert_eq!(app.ui.open_panel.as_deref(), Some("color"));
    press(&mut app, egui::Key::F6, Modifiers::SHIFT);
    assert_eq!(app.ui.open_panel.as_deref(), Some("appearance"));
    press(&mut app, egui::Key::F9, Modifiers::COMMAND);
    assert_eq!(app.ui.open_panel.as_deref(), Some("gradient"));
    // The Window menu lists them.
    let color = crate::menus::menu_entries(&app).into_iter().find(|e| e.path == ["Window"] && e.label == "Color").unwrap();
    assert_eq!(color.shortcut, "F6");
}

/// Illustrator's other Window menu panel keys (Discord feedback: Shift+F7 for Align), the dock's
/// Layers tab (F7) among them; the menu shows them and the Keyboard Shortcuts editor lists every
/// panel, so they can be changed.
#[test]
fn the_window_menu_panel_keys() {
    let mut app = app();
    let ctx = context();
    let press = |app: &mut VectorcraftApp, key, modifiers| {
        let events = vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| shortcuts::handle(app, ui.ctx())).textures_delta.clear();
    };
    let (cmd, shift, alt) = (Modifiers::COMMAND, Modifiers::SHIFT, Modifiers::ALT);
    for (key, m, panel) in [
        (egui::Key::F5, Modifiers::NONE, "brushes"),
        (egui::Key::F7, shift, "align"),
        (egui::Key::F8, shift, "transform"),
        (egui::Key::F8, cmd, "info"),
        (egui::Key::F9, cmd | shift, "pathfinder"),
        (egui::Key::F11, cmd | shift, "symbols"),
        (egui::Key::T, cmd, "character"),
        (egui::Key::T, cmd | alt, "paragraph"),
        (egui::Key::T, cmd | shift, "tabs"),
        (egui::Key::T, cmd | alt | shift, "openType"),
    ] {
        press(&mut app, key, m);
        assert!(crate::panel_docking::showing(&app.ui, panel), "{m:?}+{key:?}");
        press(&mut app, key, m);
        assert!(!crate::panel_docking::showing(&app.ui, panel), "{m:?}+{key:?} again hides it");
    }
    press(&mut app, egui::Key::F7, Modifiers::NONE);
    assert_eq!(app.ui.dock_tab, crate::state::DockTab::Layers);
    let entries = crate::menus::menu_entries(&app);
    let shortcut =
        |label: &str| entries.iter().find(|e| e.path.first().map(String::as_str) == Some("Window") && e.label == label).unwrap().shortcut.clone();
    assert_eq!((shortcut("Align"), shortcut("Layers"), shortcut("Paragraph")), ("Shift+F7".into(), "F7".into(), "Cmd+Alt+T".into()));
    assert!(shortcut_editor::entry("panel:layers").is_some(), "the dock's tabs can be given keys too");
    // Changed in the editor, the old key is free.
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "panel:align", "Cmd+Alt+Shift+F7");
    shortcut_editor::sync(&app.ui);
    press(&mut app, egui::Key::F7, shift);
    assert_eq!(app.ui.open_panel, None);
    press(&mut app, egui::Key::F7, cmd | alt | shift);
    assert_eq!(app.ui.open_panel.as_deref(), Some("align"));
    app.ui.shortcut_overrides.clear();
    shortcut_editor::sync(&app.ui);
}

/// Where `shapes` paint the text `s`.
fn text_rect(shapes: &[Shape], s: &str) -> Option<Rect> {
    shapes.iter().find_map(|sh| match sh {
        Shape::Text(t) if t.galley.text() == s => Some(t.visual_bounding_rect()),
        Shape::Vec(v) => text_rect(v, s),
        _ => None,
    })
}

#[test]
fn the_transform_link_opens_the_transform_panel() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let ctx = context();
    frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    let shapes = frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    let link = text_rect(&shapes, "Transform").expect("the Control bar shows the Transform link");
    assert!(text_rect(&shapes, "Scale Corners").is_none());
    click(&mut app, &ctx, link.center(), Modifiers::NONE, control_bar);
    let shapes = frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    for s in ["Scale Corners", "Scale Strokes & Effects", "0°"] {
        assert!(text_rect(&shapes, s).is_some(), "the popover shows {s}");
    }
    // A click outside closes it.
    click(&mut app, &ctx, Pos2::new(1300.0, 900.0), Modifiers::NONE, control_bar);
    assert!(text_rect(&frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar), "Scale Corners").is_none());
}

/// The 24 pt icon buttons of the popover's bottom bar (its Swatch Libraries Menu first), left to
/// right.
fn popover_bottom_buttons(ctx: &egui::Context) -> Vec<Rect> {
    let mut r: Vec<Rect> = ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .layers()
            .filter(|(l, _)| l.order != egui::Order::Background)
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == vec2(24.0, 24.0))
            .map(|w| w.rect)
            .collect()
    });
    let bottom = r.iter().map(|r| r.top()).fold(f32::MIN, f32::max);
    r.retain(|w| (w.top() - bottom).abs() < 1.0);
    r.sort_by(|a, b| a.left().total_cmp(&b.left()));
    r
}

/// A menu row's text (menu labels leave room for a check mark before them).
fn menu_item_rect(shapes: &[Shape], s: &str) -> Option<Rect> {
    shapes.iter().find_map(|sh| match sh {
        Shape::Text(t) if t.galley.text().trim() == s => Some(t.visual_bounding_rect()),
        Shape::Vec(v) => menu_item_rect(v, s),
        _ => None,
    })
}

/// #536: the Swatch Libraries Menu button at the foot of a Fill chip's popover opens the
/// libraries menu (it used to close the popover and show nothing), and a library chosen there
/// opens in the library panel.
#[test]
fn the_popover_swatch_libraries_button_opens_the_libraries_menu() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let ctx = context();
    frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    let (fill, _) = chips(&ctx);
    click(&mut app, &ctx, fill.center(), Modifiers::NONE, control_bar);
    let libraries = *popover_bottom_buttons(&ctx).first().expect("the popover has a bottom bar");
    click(&mut app, &ctx, libraries.center(), Modifiers::NONE, control_bar);
    let shapes = frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    assert!(text_rect(&shapes, "Swatch Tiles").is_some(), "the popover stays open");
    let defaults = menu_item_rect(&shapes, "Default Swatches").expect("the libraries menu is open");
    assert!(menu_item_rect(&shapes, "Other Library…").is_some());
    let earth = menu_item_rect(&shapes, "Earth Tones").expect("the menu lists the built-in libraries");
    assert!(earth.top() > defaults.top());
    click(&mut app, &ctx, earth.center(), Modifiers::NONE, control_bar);
    let open = crate::panels::library_panel::last_open(&app.ui).map(|o| (o.kind.clone(), o.id.clone()));
    assert_eq!(open, Some(("swatches".into(), "earth-tones".into())));
}

/// #4249: the Color Mixer a Shift-click on a Fill chip opens has the Color panel's menu, whose
/// color modes (Grayscale, RGB, HSB, CMYK, Web Safe RGB) switch the color it shows and edits.
#[test]
fn the_mixer_popover_has_the_color_panels_menu() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let ctx = context();
    frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    let (fill, _) = chips(&ctx);
    click(&mut app, &ctx, fill.center(), Modifiers::SHIFT, control_bar);
    let menu = ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .layers()
            .filter(|(l, _)| l.order != egui::Order::Background)
            .flat_map(|(_, w)| w.iter())
            .find_map(|w| (w.rect.size() == vec2(18.0, 18.0)).then_some(w.rect))
    });
    let menu = menu.expect("the mixer has a panel menu button");
    click(&mut app, &ctx, menu.center(), Modifiers::NONE, control_bar);
    let shapes = frame(&mut app, &ctx, vec![], Modifiers::NONE, control_bar);
    // The fill is RGB, so RGB is checked.
    assert!(menu_item_rect(&shapes, "✓ RGB").is_some(), "RGB is the checked mode");
    for mode in ["Grayscale", "HSB", "Web Safe RGB", "Invert", "Complement"] {
        assert!(menu_item_rect(&shapes, mode).is_some(), "{mode} in the mixer's menu");
    }
    let cmyk = menu_item_rect(&shapes, "CMYK").expect("CMYK in the mixer's menu");
    click(&mut app, &ctx, cmyk.center(), Modifiers::NONE, control_bar);
    let fill = panels::current_paints(&app).0.color();
    assert!(matches!(fill, Some(vectorcraft_color::Color::Cmyk { .. })), "the fill is CMYK now: {fill:?}");
}
