//! Dialogs in a short or narrow window: a dialog too large for the window scrolls instead of
//! running off the screen. Every scroll bar here is drawn at rest (a solid bar style), so the tests
//! can find it.
use egui::{Pos2, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

type Open = fn(&mut VectorcraftApp);

/// What a dialog drew: the shapes, the dialog window's rectangle, and how it scrolls
/// ([`modal::scrolling`]).
struct Shown {
    shapes: Vec<egui::epaint::ClippedShape>,
    window: Rect,
    body: Option<f32>,
    whole: [bool; 2],
}

/// Open a dialog with `open` in a `size` window, with the pointer in the window's corner (away
/// from the dialog), and draw it until it settles. With `scroll`, then scroll far down and right
/// near the dialog's right edge (on the bar of a window that scrolls as a whole, beside the lists
/// inside it) and draw it until it settles again.
fn show_in(open: Open, size: egui::Vec2, scroll: bool) -> Shown {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 60})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    ctx.global_style_mut(|s| s.spacing.scroll = egui::style::ScrollStyle::solid());
    open(&mut app);
    let screen = Rect::from_min_size(Pos2::ZERO, size);
    let window = |ctx: &egui::Context| {
        ctx.memory(|m| {
            m.areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|l| l.order == egui::Order::Foreground)
                .filter_map(|l| Some((l.id, m.area_rect(l.id)?)))
                .max_by(|a, b| a.1.area().total_cmp(&b.1.area()))
        })
        .unwrap_or((egui::Id::NULL, Rect::NOTHING))
    };
    let mut time = 0.0;
    let mut frames = |app: &mut VectorcraftApp, first: Vec<egui::Event>| {
        let mut first = Some(first);
        let mut shapes = vec![];
        for _ in 0..12 {
            time += 1.0;
            let input =
                egui::RawInput { screen_rect: Some(screen), events: first.take().unwrap_or_default(), time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(app, ui.ctx()));
            out.textures_delta.clear();
            shapes = std::mem::take(&mut out.shapes);
        }
        shapes
    };
    let mut shapes = frames(&mut app, vec![egui::Event::PointerMoved(screen.max - vec2(2.0, 2.0))]);
    if scroll {
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(-5000.0, -5000.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, w) = window(&ctx);
        let before = Shown { shapes: std::mem::take(&mut shapes), window: w, body: None, whole: [false; 2] };
        let at = whole_bar(&before).map_or(egui::pos2(w.right() - 23.0, w.center().y), |r| r.center());
        shapes = frames(&mut app, vec![egui::Event::PointerMoved(at), wheel]);
    }
    let (id, window) = window(&ctx);
    let (body, whole) = modal::scrolling(&ctx, id);
    Shown { shapes, window, body, whole }
}

/// Filled bars 4 to 10 pt across and longer than 12 pt (scroll bars and their handles), upright or
/// (`sideways`) flat, inside the dialog window; with `seen`, only those inside their clip.
fn bars(shown: &Shown, sideways: bool, seen: bool) -> Vec<Rect> {
    fn walk(s: &Shape, clip: Rect, sideways: bool, seen: bool, out: &mut Vec<Rect>) {
        match s {
            Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT => {
                let (across, along) = if sideways { (r.rect.height(), r.rect.width()) } else { (r.rect.width(), r.rect.height()) };
                if (4.0..=10.0).contains(&across) && along > 12.0 && (!seen || clip.contains_rect(r.rect)) {
                    out.push(r.rect);
                }
            }
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, sideways, seen, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    shown.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, sideways, seen, &mut out));
    out.retain(|r| shown.window.expand(0.5).contains_rect(*r));
    out
}

/// The bar of a shared dialog's scrolling body: the rightmost upright bar as tall as the body is
/// drawn (a list inside the body draws its bar further left).
fn body_bar(shown: &Shown) -> Option<Rect> {
    let view = shown.body?;
    bars(shown, false, true).into_iter().filter(|r| (r.height() - view).abs() <= 1.0).max_by(|a, b| a.right().total_cmp(&b.right()))
}

/// The bar of a window that scrolls as a whole: an upright bar at the window's right edge that
/// runs most of the window's height (lists inside it are shorter).
fn whole_bar(shown: &Shown) -> Option<Rect> {
    let w = shown.window;
    bars(shown, false, true)
        .into_iter()
        .filter(|r| r.right() >= w.right() - 24.0 && r.height() >= 0.8 * w.height())
        .max_by(|a, b| a.height().total_cmp(&b.height()))
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
fn a_dialog_taller_than_the_window_scrolls_its_body_and_keeps_its_buttons_in_view() {
    let opens: [(&str, Open, &[&str]); 7] = [
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
        (
            "Print",
            |app| {
                print::open(app).unwrap();
            },
            &["Cancel", "Done"],
        ),
    ];
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 480.0));
    for (name, open, labels) in opens {
        let tall = show_in(open, vec2(1280.0, 900.0), false);
        assert!(tall.window.height() > screen.height(), "{name}: the dialog is taller than the short window ({:?})", tall.window);
        assert_eq!((tall.body, tall.whole), (None, [false; 2]), "{name}: in a tall window nothing scrolls");
        let shown = show_in(open, screen.size(), false);
        assert!(screen.contains_rect(shown.window), "{name}: the dialog stays on the screen: {:?}", shown.window);
        let found = buttons(&shown, screen, labels);
        assert_eq!(found.len(), labels.len(), "{name}: {found:?}");
        assert!(found.iter().all(|(_, seen)| *seen), "{name}: the buttons stay in view: {found:?}");
        assert_eq!(shown.whole, [false; 2], "{name}: the window does not scroll as a whole");
        let bar = body_bar(&shown);
        assert!(bar.is_some(), "{name}: the body scrolls ({:?}) and its bar is drawn: {:?}", shown.body, bars(&shown, false, true));
    }
}

#[test]
fn a_list_in_a_scrolling_body_is_no_taller_than_the_body() {
    // The settings of Save for Web and the sections of Print and Save PDF scroll in lists of their
    // own; in a short window those lists are no taller than the body shows, so each list and its
    // bar can be scrolled into view whole.
    let opens: [(&str, Open); 3] = [
        ("Save for Web", |app| {
            save_for_web::open(app).unwrap();
        }),
        ("Print", |app| {
            print::open(app).unwrap();
            if let Some(d) = app.ui.dialog.as_mut() {
                d.fields.insert("__section".into(), json!("Marks and Bleed"));
            }
        }),
        ("Save PDF", |app| {
            save_pdf::open(app, &json!({"path": "/tmp/x.pdf"})).unwrap();
            if let Some(d) = app.ui.dialog.as_mut() {
                d.fields.insert("__section".into(), json!("Marks and Bleeds"));
            }
        }),
    ];
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 400.0));
    let mut wrong = vec![];
    for (name, open) in opens {
        let shown = show_in(open, screen.size(), false);
        let Some(body) = body_bar(&shown) else {
            wrong.push(format!("{name}: the body does not scroll ({:?}): {:?}", shown.body, bars(&shown, false, false)));
            continue;
        };
        // The tracks of the lists' bars, seen or not: left of the body's bar and inside the window
        // (a list taller than the body runs past the window's bottom).
        let lists: Vec<Rect> = bars(&shown, false, false).into_iter().filter(|r| r.right() < body.left()).collect();
        let tallest = lists.iter().map(|r| r.height()).fold(0.0, f32::max);
        if tallest == 0.0 || tallest > body.height() + 0.5 {
            wrong.push(format!("{name}: no list bar no taller than the body ({}): {:?}", body.height(), bars(&shown, false, false)));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn save_for_web_in_the_smallest_window_keeps_its_settings_and_buttons_in_view() {
    let open: Open = |app| {
        save_for_web::open(app).unwrap();
    };
    // The app window's smallest size, 800 by 500 pixels, at a UI scale of 100%, 110% and 125%.
    let mut wrong = vec![];
    for size in [vec2(800.0, 500.0), vec2(727.0, 454.0), vec2(640.0, 400.0)] {
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        let shown = show_in(open, size, false);
        let labels = ["Format:", "Optimize to File Size…", "Cancel", "Done"];
        let found = buttons(&shown, screen, &labels);
        if !screen.contains_rect(shown.window) || found.len() != labels.len() || !found.iter().all(|(_, seen)| *seen) {
            wrong.push(format!("{size:?}: the dialog ({:?}) leaves its settings and buttons in view: {found:?}", shown.window));
        }
        if body_bar(&shown).is_none() || shown.whole != [false; 2] {
            wrong.push(format!("{size:?}: only the body scrolls ({:?}, {:?})", shown.body, shown.whole));
        }
        // The controls under the narrower panes stay in their column, left of the settings.
        let controls = text_rects(&shown, "Preview in Browser").first().map(|r| r.right());
        let settings = text_rects(&shown, "Format:").first().map(|r| r.left());
        if !matches!((controls, settings), (Some(c), Some(s)) if c < s) {
            wrong.push(format!("{size:?}: Preview in Browser ({controls:?}) is left of the settings ({settings:?})"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The rectangles of the texts `label` the dialog drew.
fn text_rects(shown: &Shown, label: &str) -> Vec<Rect> {
    fn walk(s: &Shape, label: &str, out: &mut Vec<Rect>) {
        match s {
            Shape::Text(t) if t.galley.text() == label => out.push(t.visual_bounding_rect()),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, label, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    shown.shapes.iter().for_each(|c| walk(&c.shape, label, &mut out));
    out
}

#[test]
fn a_window_too_large_for_the_screen_scrolls_as_a_whole() {
    let opens: [(&str, Open, &[&str]); 4] = [
        ("Preferences", |app| crate::prefs_dialog::open(app, None), &["OK", "Cancel"]),
        ("Keyboard Shortcuts", crate::shortcut_editor::open, &["OK", "Cancel"]),
        ("New Document", new_document::open, &["Create", "Close"]),
        ("Find Font", crate::find_font::open, &["Done"]),
    ];
    // The app window's smallest size.
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
    for (name, open, labels) in opens {
        let tall = show_in(open, vec2(1280.0, 900.0), false);
        assert!(
            tall.window.height() > screen.height() || tall.window.width() > screen.width(),
            "{name}: the dialog is larger than the small window ({:?})",
            tall.window
        );
        assert_eq!(tall.whole, [false; 2], "{name}: in a large window the dialog does not scroll as a whole");
        let shown = show_in(open, screen.size(), false);
        assert!(screen.contains_rect(shown.window), "{name}: the dialog stays on the screen: {:?}", shown.window);
        assert!(shown.whole[1], "{name}: the dialog scrolls as a whole");
        assert!(whole_bar(&shown).is_some(), "{name}: the window's bar is drawn: {:?}", bars(&shown, false, true));
        // The buttons can sit outside the part in view; each is in view at the top or scrolled to
        // the end.
        let scrolled = show_in(open, screen.size(), true);
        let (top, end) = (buttons(&shown, screen, labels), buttons(&scrolled, screen, labels));
        for label in labels {
            let seen = |found: &[(String, bool)]| found.iter().any(|(s, seen)| s == label && *seen);
            assert!(seen(&top) || seen(&end), "{name}: {label} can be scrolled into view: {top:?} {end:?}");
        }
    }
}

#[test]
fn keyboard_shortcuts_keeps_its_scroll_under_import_shortcut_set() {
    // Scrolled down in the smallest window, Keyboard Shortcuts is where it was left when Import
    // Shortcut Set, opened over it, is canceled; closed and opened again, it starts at its top.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
    let mut time = 0.0;
    // Draws 12 frames, the first with `events`; returns whether the heading can be seen.
    let mut heading_seen = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut events = Some(events);
        let mut seen = false;
        for _ in 0..12 {
            time += 1.0;
            let input =
                egui::RawInput { screen_rect: Some(screen), events: events.take().unwrap_or_default(), time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(app, ui.ctx()));
            out.textures_delta.clear();
            let shown = Shown { shapes: out.shapes, window: screen, body: None, whole: [false; 2] };
            seen = buttons(&shown, screen, &["Keyboard Shortcuts"]).iter().any(|(_, seen)| *seen);
        }
        seen
    };
    let wheel = |at: Pos2| {
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -5000.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(at), wheel]
    };
    crate::shortcut_editor::open(&mut app);
    assert!(heading_seen(&mut app, vec![]), "Keyboard Shortcuts opens at its top");
    // On the window's bar, inside its 20 pt margin.
    let window = ctx.memory(|m| m.area_rect(egui::Id::unique("dialog-shortcuts"))).unwrap_or(screen);
    assert!(!heading_seen(&mut app, wheel(egui::pos2(window.right() - 23.0, window.center().y))), "the wheel scrolls the heading away");
    let kys = "/Menus {\n /Acme\\ Copy { /Modifiers 11 /Key 902 }\n}\n";
    crate::io::open_bytes(&mut app, "Keys.kys", kys.as_bytes(), None).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(shortcut_import::KIND));
    heading_seen(&mut app, vec![]);
    cancel(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("shortcuts"), "Cancel shows Keyboard Shortcuts again");
    assert!(!heading_seen(&mut app, vec![]), "shown again, Keyboard Shortcuts is where it was scrolled to");
    cancel(&mut app);
    heading_seen(&mut app, vec![]);
    crate::shortcut_editor::open(&mut app);
    assert!(heading_seen(&mut app, vec![]), "opened again, Keyboard Shortcuts starts at its top");
}

#[test]
fn a_window_that_fits_itself_but_is_too_wide_scrolls_sideways() {
    // Import Shortcut Set sizes its lists from the window's height and is about 780 pt wide. An
    // 800 by 600 pixel window at a UI scale of 125% is 640 by 480 pt.
    let open: Open = |app| {
        let kys = "/Menus {\n /Acme\\ Copy { /Modifiers 11 /Key 902 }\n /Acme\\ Paste { /Modifiers 11 /Key 903 }\n}\n";
        crate::io::open_bytes(app, "Keys.kys", kys.as_bytes(), None).unwrap();
    };
    let wide = show_in(open, vec2(1280.0, 900.0), false);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0));
    assert!(wide.window.width() > screen.width(), "Import Shortcut Set is wider than the window: {:?}", wide.window);
    assert_eq!(wide.whole, [false; 2], "in a large window it does not scroll");
    let shown = show_in(open, screen.size(), false);
    assert!(screen.contains_rect(shown.window), "the dialog stays on the screen: {:?}", shown.window);
    assert_eq!(shown.whole, [true, false], "it scrolls sideways only");
    let sideways: Vec<Rect> = bars(&shown, true, true).into_iter().filter(|r| r.width() >= 0.5 * shown.window.width()).collect();
    assert!(!sideways.is_empty(), "its sideways bar is drawn: {:?}", bars(&shown, true, false));
}
