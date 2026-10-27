//! The » list of the document tab strip with more documents than fit in the window (#4017).

use egui::{Event, PointerButton, Pos2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, chrome};

const SIZE: egui::Vec2 = egui::vec2(1000.0, 700.0);

/// Run a tab-strip frame and return the visible texts with their screen rects.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<(String, egui::Rect)> {
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, SIZE)), time: Some(time), events, ..Default::default() },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| chrome::doc_tabs(app, ui));
        },
    );
    out.textures_delta.clear();
    let screen = egui::Rect::from_min_size(Pos2::ZERO, SIZE);
    // Texts nested in shape lists too, kept only where their clip rect shows them whole.
    fn walk(s: &egui::Shape, clip: egui::Rect, v: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => {
                let r = egui::Rect::from_min_size(t.pos, t.galley.size());
                if clip.contains_rect(r) {
                    v.push((t.galley.text().to_owned(), r));
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
