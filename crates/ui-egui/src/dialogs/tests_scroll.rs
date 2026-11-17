//! Dialogs in a short or narrow window: options that run past their list show its scroll bar at
//! rest, and a dialog too large for the window scrolls instead of running off the screen.
use egui::{Pos2, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

type Open = fn(&mut VectorcraftApp);

/// What a dialog drew: the shapes, and the dialog window's rectangle.
struct Shown {
    shapes: Vec<egui::epaint::ClippedShape>,
    window: Rect,
}

/// Open a dialog with `open` in a `size` window, with the pointer in the window's corner (away
/// from the dialog), and draw it until it settles.
fn show_in(open: Open, size: egui::Vec2) -> Shown {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 60})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    open(&mut app);
    let screen = Rect::from_min_size(Pos2::ZERO, size);
    let mut shapes = vec![];
    for i in 0..12 {
        let events = if i == 0 { vec![egui::Event::PointerMoved(screen.max - vec2(2.0, 2.0))] } else { vec![] };
        let input = egui::RawInput { screen_rect: Some(screen), events, time: Some(f64::from(i)), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    let window = ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .into_iter()
            .filter(|l| l.order == egui::Order::Foreground)
            .filter_map(|l| m.area_rect(l.id))
            .max_by(|a, b| a.area().total_cmp(&b.area()))
    });
    Shown { shapes, window: window.unwrap_or(Rect::NOTHING) }
}

/// Filled upright bars 4 to 10 pt wide and taller than 12 pt (vertical scroll bars and their
/// handles) inside the dialog window.
fn bars(shown: &Shown) -> Vec<Rect> {
    fn walk(s: &Shape, clip: Rect, out: &mut Vec<Rect>) {
        match s {
            Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT && (4.0..=10.0).contains(&r.rect.width()) && r.rect.height() > 12.0 => {
                if clip.contains_rect(r.rect) {
                    out.push(r.rect);
                }
            }
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    shown.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut out));
    out.retain(|r| shown.window.expand(0.5).contains_rect(*r));
    out
}

/// The texts `labels` the dialog drew, each with whether it can be seen (inside the screen and
/// its clip).
fn buttons(shown: &Shown, screen: Rect, labels: &[&str]) -> Vec<(String, bool)> {
    fn walk(s: &Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect)>) {
        match s {
            Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect(), clip)),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut texts = vec![];
    shown.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut texts));
    texts
        .into_iter()
        .filter(|(s, ..)| labels.contains(&s.as_str()))
        .map(|(s, r, clip)| (s, screen.contains_rect(r) && clip.contains_rect(r)))
        .collect()
}

#[test]
fn the_export_options_that_run_past_their_list_show_its_bar_at_rest() {
    let opens: [(&str, Open); 2] = [
        ("Save for Web", |app| {
            save_for_web::open(app).unwrap();
        }),
        ("Print", |app| {
            print::open(app).unwrap();
        }),
    ];
    for (name, open) in opens {
        let shown = show_in(open, vec2(1280.0, 900.0));
        assert!(!bars(&shown).is_empty(), "{name}: the settings scroll and their bar is drawn at rest");
    }
}

#[test]
fn a_dialog_taller_than_the_window_scrolls_its_body_and_keeps_its_buttons_in_view() {
    let opens: [(&str, Open, &[&str]); 6] = [
        ("SVG Options", |app| svg_options::open(app, svg_options::Mode::Export, Some("/tmp/x.svg")), &["OK", "Cancel"]),
        ("EPS Options", |app| eps_options::open(app, &json!({"format": "eps", "path": "/tmp/x.eps"})), &["Export", "Cancel"]),
        ("DXF Options", |app| dxf_options::open(app, &json!({"format": "dxf", "path": "/tmp/x.dxf"})), &["Export", "Cancel"]),
        (
            "Save PDF",
            |app| {
                save_pdf::open(app, &json!({"path": "/tmp/x.pdf"})).unwrap();
            },
            &["Cancel"],
        ),
        (
            "Save for Web",
            |app| {
                save_for_web::open(app).unwrap();
            },
            &["Cancel", "Done"],
        ),
        ("Export for Screens", export_for_screens::open, &["Cancel"]),
    ];
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 480.0));
    for (name, open, labels) in opens {
        let tall = show_in(open, vec2(1280.0, 900.0));
        assert!(tall.window.height() > screen.height(), "{name}: the dialog is taller than the short window ({:?})", tall.window);
        let shown = show_in(open, screen.size());
        assert!(screen.contains_rect(shown.window), "{name}: the dialog stays on the screen: {:?}", shown.window);
        let found = buttons(&shown, screen, labels);
        assert_eq!(found.len(), labels.len(), "{name}: {found:?}");
        assert!(found.iter().all(|(_, seen)| *seen), "{name}: the buttons stay in view: {found:?}");
        assert!(!bars(&shown).is_empty(), "{name}: the body scrolls and its bar is drawn");
    }
}

#[test]
fn save_for_web_in_the_smallest_window_keeps_its_settings_and_buttons_in_view() {
    let open: Open = |app| {
        save_for_web::open(app).unwrap();
    };
    // The app window's smallest size.
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
    let shown = show_in(open, screen.size());
    assert!(screen.contains_rect(shown.window), "the dialog stays on the screen: {:?}", shown.window);
    let found = buttons(&shown, screen, &["Optimize to File Size…", "Cancel", "Done"]);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.iter().all(|(_, seen)| *seen), "the settings and the buttons stay in view: {found:?}");
    assert!(!bars(&shown).is_empty(), "the settings scroll and their bar is drawn");
    // A sideways bar: flat, and wider than any slider in the dialog.
    fn sideways(s: &Shape, out: &mut Vec<Rect>) {
        match s {
            Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT && (4.0..=10.0).contains(&r.rect.height()) && r.rect.width() > 300.0 => {
                out.push(r.rect)
            }
            Shape::Vec(v) => v.iter().for_each(|s| sideways(s, out)),
            _ => {}
        }
    }
    let mut found = vec![];
    shown.shapes.iter().for_each(|c| sideways(&c.shape, &mut found));
    assert!(found.is_empty(), "the dialog fits the window's width and does not scroll sideways: {found:?}");
}

#[test]
fn a_window_too_large_for_the_screen_scrolls_as_a_whole() {
    let opens: [(&str, Open); 4] = [
        ("Preferences", |app| crate::prefs_dialog::open(app, None)),
        ("Keyboard Shortcuts", crate::shortcut_editor::open),
        ("New Document", new_document::open),
        ("Find Font", crate::find_font::open),
    ];
    // The app window's smallest size.
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
    for (name, open) in opens {
        let tall = show_in(open, vec2(1280.0, 900.0));
        assert!(
            tall.window.height() > screen.height() || tall.window.width() > screen.width(),
            "{name}: the dialog is larger than the small window ({:?})",
            tall.window
        );
        let shown = show_in(open, screen.size());
        assert!(screen.contains_rect(shown.window), "{name}: the dialog stays on the screen: {:?}", shown.window);
        assert!(!bars(&shown).is_empty(), "{name}: the dialog scrolls and its bar is drawn");
    }
}
