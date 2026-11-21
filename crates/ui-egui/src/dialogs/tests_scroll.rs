//! Dialogs in a short or narrow window: a dialog too large for the window scrolls instead of
//! running off the screen. Every scroll bar here is drawn at rest (a solid bar style), so the tests
//! can find it.

use egui::{Pos2, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

type Open = fn(&mut VectorcraftApp);

/// What a dialog drew: the shapes, the dialog window's rectangle, how it scrolls
/// ([`modal::scrolling`]), and whether it scrolled upright as a whole in any frame since it was
/// opened or the window last changed size.
struct Shown {
    shapes: Vec<egui::epaint::ClippedShape>,
    window: Rect,
    body: Option<f32>,
    whole: [bool; 2],
    tall_once: bool,
}

/// Open a dialog with `open` in a `size` window, with the pointer in the window's corner (away
/// from the dialog), and draw it until it settles. With `scroll`, then scroll far down and right
/// near the dialog's right edge (on the bar of a window that scrolls as a whole, beside the lists
/// inside it) and draw it until it settles again.
fn show_in(open: Open, size: egui::Vec2, scroll: bool) -> Shown {
    show_resized(open, None, size, scroll)
}

/// [`show_in`], with the dialog first drawn until it settles in a window `first_size` in size when
/// that is given.
fn show_resized(open: Open, first_size: Option<egui::Vec2>, size: egui::Vec2, scroll: bool) -> Shown {
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
    let mut frames = |app: &mut VectorcraftApp, first: Vec<egui::Event>, screen: Rect| {
        let mut first = Some(first);
        let (mut shapes, mut tall_once) = (vec![], false);
        for _ in 0..12 {
            time += 1.0;
            let input =
                egui::RawInput { screen_rect: Some(screen), events: first.take().unwrap_or_default(), time: Some(time), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(app, ui.ctx()));
            out.textures_delta.clear();
            shapes = std::mem::take(&mut out.shapes);
            tall_once |= modal::scrolling(&ctx, window(&ctx).0).1[1];
        }
        (shapes, tall_once)
    };
    if let Some(first_size) = first_size {
        let first_screen = Rect::from_min_size(Pos2::ZERO, first_size);
        frames(&mut app, vec![egui::Event::PointerMoved(first_screen.max - vec2(2.0, 2.0))], first_screen);
    }
    let (mut shapes, tall_once) = frames(&mut app, vec![egui::Event::PointerMoved(screen.max - vec2(2.0, 2.0))], screen);
    if scroll {
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(-5000.0, -5000.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        let (id, w) = window(&ctx);
        let (body, whole) = modal::scrolling(&ctx, id);
        let before = Shown { shapes: std::mem::take(&mut shapes), window: w, body, whole, tall_once };
        // On the window's upright bar, else on its body's bar, else on its sideways bar, else at
        // its right edge.
        let sideways = bars(&before, true, true).into_iter().find(|r| r.width() >= 0.5 * w.width());
        let at = whole_bar(&before).or(body_bar(&before)).or(sideways).map_or(egui::pos2(w.right() - 23.0, w.center().y), |r| r.center());
        shapes = frames(&mut app, vec![egui::Event::PointerMoved(at), wheel], screen).0;
    }
    let (id, window) = window(&ctx);
    let (body, whole) = modal::scrolling(&ctx, id);
    Shown { shapes, window, body, whole, tall_once }
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
/// runs most of the window's height, or all of it but the margins of a short window (lists inside
/// it are shorter).
fn whole_bar(shown: &Shown) -> Option<Rect> {
    let w = shown.window;
    bars(shown, false, true)
        .into_iter()
        .filter(|r| r.right() >= w.right() - 24.0 && r.height() >= (0.8 * w.height()).min(w.height() - 48.0))
        .max_by(|a, b| a.height().total_cmp(&b.height()))
}

/// The texts `labels` the dialog drew, each with whether it can be seen (inside the screen and
/// its clip).
fn buttons(shown: &Shown, screen: Rect, labels: &[&str]) -> Vec<(String, bool)> {
    texts(shown)
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
        // Preview in Browser is right of the download speed list's frame or on a row below it, not
        // over it.
        let button = text_rects(&shown, "Preview in Browser").first().copied();
        let speed = text_rects(&shown, "56.6 Kbps").first().and_then(|r| frame_around(&shown, *r));
        if !matches!((button, speed), (Some(b), Some(s)) if b.left() > s.right() + 16.0 || b.top() > s.bottom() + 8.0) {
            wrong.push(format!("{size:?}: Preview in Browser ({button:?}) clears the download speed list ({speed:?})"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The frame of the control that drew the text at `inner`: the widest rectangle shape around it
/// that is at most 240 by 40 pt.
fn frame_around(shown: &Shown, inner: Rect) -> Option<Rect> {
    fn walk(s: &Shape, inner: Rect, out: &mut Vec<Rect>) {
        match s {
            Shape::Rect(r) if r.rect.contains_rect(inner) && r.rect.width() <= 240.0 && r.rect.height() <= 40.0 => out.push(r.rect),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, inner, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    shown.shapes.iter().for_each(|c| walk(&c.shape, inner, &mut out));
    out.into_iter().max_by(|a, b| a.width().total_cmp(&b.width()))
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
            let shown = Shown { shapes: out.shapes, window: screen, body: None, whole: [false; 2], tall_once: false };
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

/// Open Import Shortcut Set on a set of two shortcuts.
fn open_shortcut_import(app: &mut VectorcraftApp) {
    let kys = "/Menus {\n /Acme\\ Copy { /Modifiers 11 /Key 902 }\n /Acme\\ Paste { /Modifiers 11 /Key 903 }\n}\n";
    crate::io::open_bytes(app, "Keys.kys", kys.as_bytes(), None).unwrap();
}

#[test]
fn a_window_that_fits_itself_but_is_too_wide_scrolls_sideways() {
    // Import Shortcut Set sizes its lists from the window's height and is about 780 pt wide. An
    // 800 by 600 pixel window at a UI scale of 125% is 640 by 480 pt.
    let wide = show_in(open_shortcut_import, vec2(1280.0, 900.0), false);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0));
    assert!(wide.window.width() > screen.width(), "Import Shortcut Set is wider than the window: {:?}", wide.window);
    assert_eq!(wide.whole, [false; 2], "in a large window it does not scroll");
    let shown = show_in(open_shortcut_import, screen.size(), false);
    assert!(screen.contains_rect(shown.window), "the dialog stays on the screen: {:?}", shown.window);
    assert_eq!(shown.whole, [true, false], "it scrolls sideways only");
    let sideways: Vec<Rect> = bars(&shown, true, true).into_iter().filter(|r| r.width() >= 0.5 * shown.window.width()).collect();
    assert!(!sideways.is_empty(), "its sideways bar is drawn: {:?}", bars(&shown, true, false));
}

#[test]
fn a_window_that_fits_itself_scrolls_as_a_whole_once_its_lists_are_at_their_smallest() {
    // These dialogs size their own lists from the window's height. In a window that holds them
    // with shorter lists, they fit without scrolling; in one too short for them with their lists at
    // their smallest, they scroll as a whole instead of running off the screen.
    // A dialog, how it opens, its buttons, text in its lists or rows, and window heights with
    // whether it scrolls in each and whether that text is then seen at the top or the end of the
    // window. Import Shortcut Set's lists at their smallest are shorter than one of their rows, and
    // they scroll inside themselves; in a 90 pt window Transform Each's first row is in view only
    // part way down.
    type Case = (&'static str, Open, &'static [&'static str], &'static [&'static str], &'static [(f32, bool, bool)]);
    let opens: [Case; 2] = [
        (
            "Import Shortcut Set",
            open_shortcut_import,
            &["Apply", "Cancel"],
            &["/Key 902"],
            &[(600.0, false, true), (400.0, true, false), (300.0, true, false)],
        ),
        (
            "Transform Each",
            |app| {
                app.run("ui.menuDialog", json!({"command": "object.transformEach"})).unwrap();
            },
            &["OK", "Cancel"],
            &["Scale"],
            &[(200.0, false, true), (150.0, true, true), (120.0, true, true), (90.0, true, false)],
        ),
    ];
    let mut wrong = vec![];
    for (name, open, labels, rows, heights) in opens {
        let tall = show_in(open, vec2(1280.0, 900.0), false);
        if tall.whole != [false; 2] {
            wrong.push(format!("{name}: in a large window it scrolls ({:?})", tall.whole));
        }
        // The Keys list of Import Shortcut Set is open, so both of its lists shrink.
        if !texts(&tall).iter().any(|(s, ..)| rows.contains(&s.as_str())) {
            wrong.push(format!("{name}: its lists or rows are not drawn in a large window"));
        }
        for &(height, scrolls, rows_seen) in heights {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, height));
            let shown = show_in(open, screen.size(), false);
            let scrolled = show_in(open, screen.size(), true);
            if !screen.contains_rect(shown.window) || shown.whole != [false, scrolls] || whole_bar(&shown).is_some() != scrolls {
                wrong.push(format!(
                    "{name} at {height}: the window ({:?}) stays on the screen and scrolls as a whole ({:?}) with its bar ({:?}) only when {scrolls}",
                    shown.window,
                    shown.whole,
                    bars(&shown, false, true)
                ));
            }
            if !scrolls && shown.window.height() >= tall.window.height() {
                wrong.push(format!("{name} at {height}: its lists are not shorter than in a large window ({:?})", shown.window));
            }
            if !scrolls && shown.tall_once {
                wrong.push(format!("{name} at {height}: it scrolled as a whole while its lists were fitting to the window"));
            }
            let (top, end) = (buttons(&shown, screen, labels), buttons(&scrolled, screen, labels));
            for label in labels {
                let seen = |found: &[(String, bool)]| found.iter().any(|(s, seen)| s == label && *seen);
                if !seen(&top) && !seen(&end) {
                    wrong.push(format!("{name} at {height}: {label} cannot be scrolled into view: {top:?} {end:?}"));
                }
            }
            let (top, end) = (buttons(&shown, screen, rows), buttons(&scrolled, screen, rows));
            if rows_seen && !top.iter().chain(&end).any(|(_, seen)| *seen) {
                wrong.push(format!("{name} at {height}: its lists or rows cannot be scrolled into view: {top:?} {end:?}"));
            }
        }
        // Grown from the shortest window to the tallest that holds it, it stops scrolling and is
        // as tall as when opened there.
        let (fits, short) = (heights[0].0, heights[heights.len() - 1].0);
        let opened = show_in(open, vec2(1280.0, fits), false);
        let grown = show_resized(open, Some(vec2(1280.0, short)), vec2(1280.0, fits), false);
        if grown.whole != [false; 2] || (grown.window.height() - opened.window.height()).abs() > 1.0 {
            wrong.push(format!(
                "{name} grown from {short} to {fits}: it scrolls ({:?}) or its height ({:?}) differs from opened there ({:?})",
                grown.whole, grown.window, opened.window
            ));
        }
    }
    // The same in a window too narrow for Import Shortcut Set, where it goes on scrolling sideways.
    let opened = show_in(open_shortcut_import, vec2(640.0, 600.0), false);
    let grown = show_resized(open_shortcut_import, Some(vec2(640.0, 300.0)), vec2(640.0, 600.0), false);
    if opened.whole != [true, false] || grown.whole != [true, false] || (grown.window.height() - opened.window.height()).abs() > 1.0 {
        wrong.push(format!(
            "Import Shortcut Set grown from 640 by 300 to 640 by 600 ({:?}, {:?}) scrolls sideways only and is as tall as opened there ({:?}, {:?})",
            grown.whole, grown.window, opened.whole, opened.window
        ));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_window_wider_than_the_screen_scrolls_sideways_to_its_right_edge() {
    // In a window 70% as wide as the dialog, the dialog scrolls sideways, and scrolled to its right
    // edge, the text drawn furthest right in a large window can be seen.
    let opens: [(&str, Open); 3] = [
        ("Import Shortcut Set", open_shortcut_import),
        ("Keyboard Shortcuts", crate::shortcut_editor::open),
        ("Preferences", |app| crate::prefs_dialog::open(app, None)),
    ];
    let mut wrong = vec![];
    for (name, open) in opens {
        let wide = show_in(open, vec2(1600.0, 1000.0), false);
        let Some((right, _)) = texts(&wide)
            .into_iter()
            .filter(|(_, r, _)| wide.window.contains_rect(*r))
            .map(|(s, r, _)| (s, r.right()))
            .max_by(|a, b| a.1.total_cmp(&b.1))
        else {
            wrong.push(format!("{name}: no text in the large window"));
            continue;
        };
        let screen = Rect::from_min_size(Pos2::ZERO, vec2((0.7 * wide.window.width()).round(), 1000.0));
        let shown = show_in(open, screen.size(), false);
        let scrolled = show_in(open, screen.size(), true);
        // Within 1 pt: a text's drawn box can reach a fraction of a point past the content's edge.
        let seen =
            |shown: &Shown| texts(shown).iter().any(|(s, r, clip)| *s == right && screen.contains_rect(*r) && clip.expand(1.0).contains_rect(*r));
        let sideways = bars(&shown, true, true).into_iter().any(|r| r.width() >= 0.5 * shown.window.width());
        if !screen.contains_rect(shown.window) || !shown.whole[0] || !sideways || seen(&shown) || !seen(&scrolled) {
            let at = |shown: &Shown| texts(shown).into_iter().filter(|(s, ..)| *s == right).collect::<Vec<_>>();
            wrong.push(format!(
                "{name} in {:?}: window {:?}, scrolls {:?}, sideways bar {sideways}, {right:?} seen at first {} {:?}, scrolled {} {:?}, bars {:?}",
                screen.size(),
                shown.window,
                shown.whole,
                seen(&shown),
                at(&shown),
                seen(&scrolled),
                at(&scrolled),
                bars(&shown, true, true)
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Every text the dialog drew, with its rectangle and its clip.
fn texts(shown: &Shown) -> Vec<(String, Rect, Rect)> {
    fn walk(s: &Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect)>) {
        match s {
            Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect(), clip)),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    shown.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut out));
    out
}

#[test]
fn a_dialog_with_buttons_in_its_body_scrolls_them_with_the_body() {
    // Check Spelling and a stop's message box draw their buttons inside their body: in a window too
    // short for the dialog, the heading stays in view and the buttons scroll with the body.
    let opens: [(&str, Open, &str); 2] = [
        (
            "Check Spelling",
            |app| {
                check_spelling::open(app).unwrap();
            },
            "Cancel",
        ),
        ("Action 1", |app| action_stop::open_message(app, "Action 1", &"Check the art. ".repeat(40), true), "Stop"),
    ];
    let mut wrong = vec![];
    for (heading, open, button) in opens {
        let tall = show_in(open, vec2(1280.0, 900.0), false);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1024.0, (0.6 * tall.window.height()).round()));
        let shown = show_in(open, screen.size(), false);
        let scrolled = show_in(open, screen.size(), true);
        let seen = |shown: &Shown, label: &str| buttons(shown, screen, &[label]).iter().any(|(_, seen)| *seen);
        if !screen.contains_rect(shown.window) {
            wrong.push(format!("{heading}: the dialog leaves the screen: {:?}", shown.window));
        }
        if body_bar(&shown).is_none() || shown.whole != [false; 2] {
            wrong.push(format!("{heading}: not only the body scrolls ({:?}, {:?})", shown.body, shown.whole));
        }
        if !seen(&shown, heading) || !seen(&scrolled, heading) {
            wrong.push(format!("{heading}: the heading leaves the view"));
        }
        if seen(&shown, button) || !seen(&scrolled, button) {
            wrong.push(format!("{heading}: {button} is not out of view until the body is scrolled down"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
