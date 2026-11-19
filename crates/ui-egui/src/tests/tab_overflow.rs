//! The » list of the document tab strip with more documents than fit in the window (#4017).

use egui::{Event, PointerButton, Pos2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, chrome};

const SIZE: egui::Vec2 = egui::vec2(1000.0, 700.0);

/// Run a tab-strip frame and return the visible texts with their screen rects.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<(String, egui::Rect)> {
    frame_sized(app, ctx, SIZE, time, events)
}

/// [`frame`] in a window `size` large.
fn frame_sized(app: &mut VectorcraftApp, ctx: &egui::Context, size: egui::Vec2, time: f64, events: Vec<Event>) -> Vec<(String, egui::Rect)> {
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)), time: Some(time), events, ..Default::default() },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| chrome::doc_tabs(app, ui));
        },
    );
    out.textures_delta.clear();
    let screen = egui::Rect::from_min_size(Pos2::ZERO, size);
    // Texts nested in shape lists too, kept only where their clip rect shows them whole, as their
    // glyphs draw them (a title shortened to fit ends with its …).
    fn walk(s: &egui::Shape, clip: egui::Rect, v: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => {
                let r = egui::Rect::from_min_size(t.pos, t.galley.size());
                if clip.contains_rect(r) {
                    v.push((t.galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|g| g.chr)).collect(), r));
                }
            }
            egui::Shape::Vec(list) => list.iter().for_each(|s| walk(s, clip, v)),
            _ => {}
        }
    }
    let mut texts = vec![];
    for c in &out.shapes {
        walk(&c.shape, c.clip_rect.intersect(screen), &mut texts);
    }
    texts
}

fn click(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// The tab that doesn't fit whole fills the room left before the » button with its title shortened,
/// instead of the strip leaving that room empty (#8584), and its tooltip shows the whole title.
#[test]
fn the_tab_that_does_not_fit_whole_fills_the_rest_of_the_strip() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The active tab and two inactive ones fit whole; the fourth, with a much longer name, doesn't.
    for i in 0..8 {
        let name = if i == 3 { format!("Tab test {i} {}", "with a much longer name ".repeat(4)) } else { format!("Tab test {i}") };
        app.run("file.new", json!({"width": 100, "height": 100, "name": name})).unwrap();
    }
    app.session.set_active(0);
    app.sync_views();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, 0.0, vec![]);
    let mut tabs: Vec<(String, egui::Rect)> = texts.into_iter().filter(|(t, _)| t.starts_with("Tab test")).collect();
    tabs.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    let whole = tabs.iter().filter(|(t, _)| !t.ends_with('…')).count();
    let (short, r) = tabs.last().cloned().unwrap_or_else(|| panic!("tabs are drawn: {tabs:?}"));
    assert_eq!((whole, tabs.len()), (3, 4), "3 tabs fit whole and one more is shortened: {tabs:?}");
    assert!(short.ends_with('…'), "a tab with a shortened title follows the whole ones: {tabs:?}");
    // The strip ends 8 pt from the window's edge, and the » button takes its last 30 pt; the tab's
    // padding (50 pt with the ×) and less than one character are left over.
    let more_left = SIZE.x - 8.0 - 30.0;
    assert!(r.right() > more_left - 50.0 - 16.0 && r.right() <= more_left, "the shortened title reaches the » button: {r:?}");
    // The whole title shows in the shortened tab's tooltip.
    let name = short.trim_end_matches('…');
    let mut texts = frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(r.center())]);
    for i in 1..20 {
        texts = frame(&mut app, &ctx, 1.0 + f64::from(i) * 0.1, vec![]);
    }
    assert!(
        texts.iter().any(|(t, _)| t.starts_with(name) && !t.ends_with('…') && t.contains("(RGB/Preview)")),
        "the tooltip shows the whole title: {texts:?}"
    );
}

/// An inactive tab shows the document's name alone, without the zoom and the color mode and view,
/// so more tabs fit in the strip (#8582): in a 1000 pt window, 7 tabs fit whole where 3 did with
/// whole titles. The active tab keeps its whole title, and an inactive tab's tooltip shows it.
#[test]
fn inactive_tabs_show_the_name_alone_so_more_tabs_fit() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for i in 0..12 {
        app.run("file.new", json!({"width": 100, "height": 100, "name": format!("Tab test {i}")})).unwrap();
    }
    // The second document has unsaved changes, which its tab still marks.
    app.session.set_active(1);
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.session.set_active(0);
    app.sync_views();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, 0.0, vec![]);
    let mut tabs: Vec<(String, egui::Rect)> = texts.into_iter().filter(|(t, _)| t.starts_with("Tab test")).collect();
    tabs.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    let whole = tabs.iter().filter(|(t, _)| !t.ends_with('…')).count();
    assert_eq!(whole, 7, "7 tabs fit whole: {tabs:?}");
    assert!(tabs.first().is_some_and(|(t, _)| t == "Tab test 0 @ 100 % (RGB/Preview)"), "the active tab keeps its whole title: {tabs:?}");
    assert!(tabs.get(1).is_some_and(|(t, _)| t == "Tab test 1*"), "an inactive tab shows the name and the unsaved mark: {tabs:?}");
    assert!(tabs.iter().skip(2).all(|(t, _)| !t.contains('@') && !t.contains("RGB")), "inactive tabs show no zoom or mode: {tabs:?}");
    // The whole title shows in an inactive tab's tooltip.
    let r = tabs.get(2).map(|(_, r)| *r).unwrap_or_else(|| panic!("{tabs:?}"));
    let mut texts = frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(r.center())]);
    for i in 1..20 {
        texts = frame(&mut app, &ctx, 1.0 + f64::from(i) * 0.1, vec![]);
    }
    assert!(texts.iter().any(|(t, _)| t == "Tab test 2 @ 100 % (RGB/Preview)"), "the tooltip shows the whole title: {texts:?}");
}

/// With the last document active, the tabs from it to the end leave room before the » button, and
/// the tab before them fills that room with its title shortened instead of the strip ending short
/// of the » button (#8584).
#[test]
fn with_the_last_document_active_the_tab_before_the_shown_ones_fills_the_room() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The last three tabs fit whole; the one before them, with a much longer name, doesn't.
    for i in 0..8 {
        let name = if i <= 4 { format!("Tab test {i} {}", "with a much longer name ".repeat(4)) } else { format!("Tab test {i}") };
        app.run("file.new", json!({"width": 100, "height": 100, "name": name})).unwrap();
    }
    app.session.set_active(7);
    app.sync_views();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, 0.0, vec![]);
    let mut tabs: Vec<(String, egui::Rect)> = texts.into_iter().filter(|(t, _)| t.starts_with("Tab test")).collect();
    tabs.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    let whole = tabs.iter().filter(|(t, _)| !t.ends_with('…')).count();
    assert_eq!((whole, tabs.len()), (3, 4), "3 tabs fit whole and one more is shortened: {tabs:?}");
    let (lead, r) = tabs.first().cloned().unwrap_or_else(|| panic!("tabs are drawn: {tabs:?}"));
    assert!(lead.starts_with("Tab test 4") && lead.ends_with('…'), "the tab before the shown ones comes first, shortened: {tabs:?}");
    // The strip starts 8 pt from the window's edge, and the title 14 pt into the tab (32 pt on
    // macOS, after the ×).
    assert!(r.left() <= 8.0 + 32.0 + 1.0, "the shortened tab starts the strip: {r:?}");
    assert!(tabs.last().is_some_and(|(t, _)| t.starts_with("Tab test 7")), "the active tab ends the strip: {tabs:?}");
}

/// The active tab with a title wider than the whole strip is shortened to end before the » button,
/// so its × stays on screen, and its tooltip shows the whole title.
#[test]
fn an_active_tab_wider_than_the_strip_is_shortened_to_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100, "name": "Tab test short"})).unwrap();
    let long = format!("Tab test {}", "long name ".repeat(20));
    app.run("file.new", json!({"width": 100, "height": 100, "name": long})).unwrap();
    app.session.set_active(1);
    app.sync_views();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, 0.0, vec![]);
    let tabs: Vec<(String, egui::Rect)> = texts.into_iter().filter(|(t, _)| t.starts_with("Tab test")).collect();
    let [(short, r)] = tabs.as_slice() else { panic!("only the active tab shows: {tabs:?}") };
    assert!(short.starts_with("Tab test long") && short.ends_with('…'), "{tabs:?}");
    let more_left = SIZE.x - 8.0 - 30.0;
    assert!(r.right() <= more_left, "the title ends before the » button: {r:?}");
    let name = short.trim_end_matches('…');
    let mut texts = frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(r.center())]);
    for i in 1..20 {
        texts = frame(&mut app, &ctx, 1.0 + f64::from(i) * 0.1, vec![]);
    }
    assert!(texts.iter().any(|(t, _)| t.starts_with(name) && t.contains("(RGB/Preview)")), "the tooltip shows the whole title: {texts:?}");
    // In a window narrower than a tooltip's usual width, the tooltip wraps the title to fit.
    let narrow = egui::vec2(300.0, 400.0);
    let texts = frame_sized(&mut app, &ctx, narrow, 3.0, vec![]);
    let r = texts.iter().find(|(t, _)| t.starts_with("Tab test long")).map(|(_, r)| *r).unwrap_or_else(|| panic!("{texts:?}"));
    let mut texts = frame_sized(&mut app, &ctx, narrow, 4.0, vec![Event::PointerMoved(r.center())]);
    for i in 1..20 {
        texts = frame_sized(&mut app, &ctx, narrow, 4.0 + f64::from(i) * 0.1, vec![]);
    }
    let tip = texts.iter().find(|(t, _)| t.contains("(RGB/Preview)")).unwrap_or_else(|| panic!("the tooltip shows in the window: {texts:?}"));
    assert!(tip.1.right() <= narrow.x, "{tip:?}");
}

/// In a strip too narrow for any of the active tab's title, the title, "…" included, is not drawn
/// over the ×; in one too narrow for the × as well, a click on the tab does not close it.
#[test]
fn an_active_tab_in_a_very_narrow_strip_keeps_clear_of_its_close_box() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100, "name": "Tab test short"})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100, "name": "Tab test active"})).unwrap();
    app.session.set_active(1);
    app.sync_views();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // The strip is 70 pt wide: the » button takes 30 pt and the active tab the other 40 pt.
    let texts = frame_sized(&mut app, &ctx, egui::vec2(86.0, 400.0), 0.0, vec![]);
    assert!(!texts.iter().any(|(t, r)| r.left() < 48.0 && (t.starts_with("Tab test") || t.contains('…'))), "no title is drawn: {texts:?}");
    // The strip is 50 pt wide, leaving the tab 20 pt: a click on it makes no × close it.
    let narrow = egui::vec2(66.0, 400.0);
    let at = Pos2::new(18.0, 18.0);
    frame_sized(&mut app, &ctx, narrow, 1.0, vec![Event::PointerMoved(at)]);
    frame_sized(&mut app, &ctx, narrow, 1.1, vec![click(at, true)]);
    frame_sized(&mut app, &ctx, narrow, 1.2, vec![click(at, false)]);
    frame_sized(&mut app, &ctx, narrow, 1.3, vec![]);
    assert_eq!(app.session.documents().len(), 2, "the click does not close the tab's document");
}

/// Every document of a large session can be picked from the » list: it scrolls instead of running
/// off the bottom of the window.
#[test]
fn the_tab_overflow_list_scrolls_to_its_last_document() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for i in 0..60 {
        app.run("file.new", json!({"width": 100, "height": 100, "name": format!("Doc {i:02}")})).unwrap();
    }
    app.session.set_active(0);
    app.sync_views();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, 0.0, vec![]);
    let tab = texts.iter().find(|(t, _)| t.starts_with("Doc 00")).map(|(_, r)| *r).unwrap();
    // The » button sits at the right end of the strip, level with the tabs.
    let more = egui::pos2(SIZE.x - 8.0 - 15.0, tab.center().y);
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(more)]);
    frame(&mut app, &ctx, 0.15, vec![click(more, true)]);
    frame(&mut app, &ctx, 0.2, vec![click(more, false)]);
    frame(&mut app, &ctx, 0.25, vec![]);
    let texts = frame(&mut app, &ctx, 0.3, vec![]);
    // The list's rows put a check column before each title; the tabs don't.
    let row = |texts: &[(String, egui::Rect)], name: &str| texts.iter().find(|(t, _)| !t.starts_with("Doc") && t.contains(name)).map(|(_, r)| *r);
    let first = row(&texts, "Doc 00").unwrap_or_else(|| panic!("the » list is open: {texts:?}"));
    assert!(row(&texts, "Doc 59").is_none(), "the last document starts below the bottom of the window");
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -20000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    frame(&mut app, &ctx, 0.35, vec![Event::PointerMoved(first.center()), wheel]);
    let mut texts = vec![];
    for i in 4..40 {
        texts = frame(&mut app, &ctx, f64::from(i) * 0.1, vec![]);
    }
    let last = row(&texts, "Doc 59").unwrap_or_else(|| panic!("the wheel scrolls the list to its last document: {texts:?}")).center();
    frame(&mut app, &ctx, 4.0, vec![Event::PointerMoved(last)]);
    frame(&mut app, &ctx, 4.1, vec![click(last, true)]);
    frame(&mut app, &ctx, 4.2, vec![click(last, false)]);
    assert_eq!(app.session.active_index(), Some(59), "the last document is picked from the list");
}
