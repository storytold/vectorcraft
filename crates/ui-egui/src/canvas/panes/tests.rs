use std::collections::BTreeSet;

use egui::{Color32, Pos2, Shape};
use vectorcraft_engine::Session;
use vectorcraft_geom::Rect;

use super::*;
use crate::menus::enabled;

/// An app with `n` new 400 × 300 pt documents, the last one current.
fn app_with(n: usize) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for _ in 0..n {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    }
    app
}

/// One headless frame of the canvas panel's body (the tabs and the canvas or panes) on a
/// 1200 × 800 window → the shapes it drew, `Shape::Vec`s flattened.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<Shape> {
    frame_in(app, ctx, events, vec2(1200.0, 800.0))
}

/// [`frame`] on a window `size` points large.
fn frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, size: egui::Vec2) -> Vec<Shape> {
    fn flat(s: Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)), max_texture_side: Some(8192), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| app.canvas_area(ui));
    out.textures_delta.clear();
    let mut shapes = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut shapes));
    shapes
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// The documents' indices in the panes the last frame drew, and the panes.
fn panes(app: &VectorcraftApp) -> (Vec<usize>, Vec<egui::Rect>) {
    app.canvas.panes.last.iter().copied().unzip()
}

#[test]
fn cells_tile_the_area_edge_to_edge() {
    let area = egui::Rect::from_min_size(pos2(10.0, 35.0), vec2(1001.0, 703.0));
    assert!(cells(0, area).is_empty());
    for (n, cols, rows) in [(1, 1, 1), (2, 2, 1), (3, 2, 2), (4, 2, 2), (5, 3, 2), (6, 3, 2), (7, 3, 3), (10, 4, 3)] {
        let c = cells(n, area);
        assert_eq!(c.len(), n);
        let lefts: BTreeSet<u32> = c.iter().map(|r| r.left().to_bits()).collect();
        let tops: BTreeSet<u32> = c.iter().map(|r| r.top().to_bits()).collect();
        assert_eq!((lefts.len(), tops.len()), (cols, rows), "{n} panes: columns and rows");
        assert_eq!((c[0].left(), c[0].top()), (area.left(), area.top()), "{n} panes start at the corner");
        for (i, r) in c.iter().enumerate() {
            assert!(r.width() > 0.0 && r.height() > 0.0 && area.contains_rect(*r), "{n} panes: {r:?}");
            let last_column = i % cols == cols - 1;
            match c.get(i + 1).filter(|_| !last_column) {
                Some(next) => {
                    assert_eq!((r.right(), r.top(), r.bottom()), (next.left(), next.top(), next.bottom()), "{n} panes: {i} meets its neighbor")
                }
                None if last_column => assert_eq!(r.right(), area.right(), "{n} panes: {i} reaches the right edge"),
                None => {}
            }
            match c.get(i + cols) {
                Some(below) => assert_eq!(r.bottom(), below.top(), "{n} panes: {i} meets the pane below"),
                None if i / cols == rows - 1 => assert_eq!(r.bottom(), area.bottom(), "{n} panes: {i} reaches the bottom"),
                None => {}
            }
            for o in &c[i + 1..] {
                assert!(r.intersect(*o).area() <= 0.0, "{n} panes overlap: {r:?} {o:?}");
            }
        }
        let covered: f32 = c.iter().map(egui::Rect::area).sum();
        let whole = area.area() * n as f32 / (cols * rows) as f32;
        assert!((covered - whole).abs() < 1.0, "{n} panes cover {covered}, not {whole}");
    }
}

#[test]
fn tile_shows_a_pane_for_each_document_fitted_to_it() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty(), "documents open under their tabs");
    let tabbed = app.canvas_rect.unwrap();
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": true, "panes": 3}));
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 1, 2]);
    // Three documents: two columns and two rows, the lower-right cell empty.
    assert_eq!(rects[0].top(), rects[1].top());
    assert_eq!(rects[0].left(), rects[2].left());
    assert!(rects[2].top() > rects[0].top());
    let canvas = app.canvas_rect.unwrap();
    assert!(rects[2].contains_rect(canvas) && canvas.width() < tabbed.width() / 2.0, "the current document's pane is the canvas: {canvas:?}");
    let artboard = Rect::new(0.0, 0.0, 400.0, 300.0);
    for (i, r) in docs.iter().zip(&rects) {
        let v = app.views[*i];
        let shown = if *i == 2 { canvas } else { view_rects(&app, *r, ctx.pixels_per_point()).1 };
        assert!(v.fitted && (v.zoom - fit_zoom(shown, artboard, 0.0)).abs() < 1e-9, "document {i} fits its pane: {v:?}");
        assert_eq!(v.center, artboard.center());
    }
    assert_eq!(app.canvas.panes.shown.len(), 2, "the other two documents are rendered for their panes");
    let inspected = crate::control::inspect(&app, &ctx);
    assert_eq!(inspected["tiled"], json!(true));
    let active: Vec<bool> = inspected["panes"].as_array().unwrap().iter().map(|p| p["active"].as_bool().unwrap()).collect();
    assert_eq!(active, [false, false, true]);
}

#[test]
fn a_press_in_another_pane_makes_its_document_current_without_drawing() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("rectangle");
    let pane = panes(&app).1[0];
    let (from, to) = (pane.center() - vec2(30.0, 20.0), pane.center() + vec2(30.0, 20.0));
    let drag = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
        frame(app, &ctx, vec![button(from, true)]);
        for k in 1..=4 {
            frame(app, &ctx, vec![egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
        }
        frame(app, &ctx, vec![button(to, false)]);
        frame(app, &ctx, vec![]);
    };
    drag(&mut app);
    assert_eq!(app.session.active_index(), Some(0), "the press made the first document current");
    assert!(app.session.documents()[0].doc.art_bounds().is_none(), "the press that switched documents drew nothing");
    assert!(app.canvas_rect.is_some_and(|c| pane.contains_rect(c)), "its pane is the canvas now");
    drag(&mut app);
    assert!(app.session.documents()[0].doc.art_bounds().is_some(), "a drag in the current pane draws");
    assert!(app.session.documents()[1..].iter().all(|d| d.doc.art_bounds().is_none()));
}

#[test]
fn consolidate_all_windows_shows_one_document_again() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let tabbed = app.canvas_rect.unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0.len(), 2);
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({})).unwrap(), json!({"tiled": false}));
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty() && app.canvas.panes.shown.is_empty());
    assert_eq!(app.canvas_rect, Some(tabbed), "the current document fills the document area again");
}

#[test]
fn arrange_items_follow_the_open_documents() {
    let mut app = app_with(0);
    let items = |app: &VectorcraftApp| (enabled(app, "window.arrange.tile"), enabled(app, "window.arrange.consolidateAllWindows"));
    assert_eq!(items(&app), (false, false), "no document");
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(items(&app), (false, false), "one document");
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(items(&app), (true, false), "two documents under their tabs");
    app.run("window.arrange.tile", json!({})).unwrap();
    assert_eq!(items(&app), (true, true), "tiled");
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    assert_eq!(items(&app), (true, false), "consolidated");
    app.run("window.arrange.tile", json!({})).unwrap();
    app.run("file.close", json!({})).unwrap();
    app.run("file.close", json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(items(&app), (false, false), "every document closed");
    assert!(!app.ui.tiled);
}

#[test]
fn closing_a_document_while_tiled_tiles_the_others() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 1, 2]);
    let first = app.session.documents()[0].uid as usize;
    assert!(app.canvas.panes.shown.contains_key(&first));
    // The first document, not the current one, closes.
    app.run("file.close", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 1]);
    assert_eq!(rects[0].height(), rects[1].height(), "two panes side by side");
    assert!(!app.canvas.panes.shown.contains_key(&first), "the closed document's render is dropped");
    app.run("file.close", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty(), "one document fills the document area");
    assert!(enabled(&app, "window.arrange.consolidateAllWindows"));
}

#[test]
fn an_edit_shows_in_its_pane_once_another_document_is_current() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let uid = app.session.documents()[1].uid as usize;
    let before = app.canvas.panes.shown.get(&uid).map(|(k, _)| k.revision);
    assert_eq!(before, Some(app.session.documents()[1].revision), "the second document's pane shows it");
    app.run("document.activate", json!({"index": 1})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 80})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let edited = &app.session.documents()[1];
    assert_ne!(Some(edited.revision), before);
    let shown = app.canvas.panes.shown.get(&uid).map(|(k, _)| k.revision);
    assert_eq!(shown, Some(edited.revision), "the pane shows the document as edited");
}

#[test]
fn other_panes_draw_their_guides_and_selected_objects() {
    let mut app = app_with(2);
    app.run("document.activate", json!({"index": 0})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 300})).unwrap();
    let st = &app.session.documents()[0];
    let layer = super::c32(st.doc.layer_color(vectorcraft_doc::NodeId(id)));
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    let pane = panes(&app).1[0];
    let xf = Xf::new(view_rects(&app, pane, ctx.pixels_per_point()).1, &app.views[0]);
    let outline: Vec<Pos2> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(layer) => Some(p.points.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let corner = xf.to_screen(Point::new(50.0, 50.0));
    assert!(outline.iter().any(|p| p.distance(corner) < 1.0), "the selected rectangle is outlined in its pane: {outline:?}");
    let guide_x = xf.to_screen(Point::new(300.0, 0.0)).x;
    let guide = LineLook::guides(&app.session.prefs, &Tokens::get(&ctx)).color;
    let guides = shapes.iter().any(|s| {
        matches!(s, Shape::LineSegment { points: [a, b], stroke } if stroke.color == guide && (a.x - guide_x).abs() < 0.5 && (b.x - guide_x).abs() < 0.5 && pane.contains(*a))
    });
    assert!(guides, "the guide is drawn in its pane");
    assert_ne!(layer, Color32::TRANSPARENT);
}

#[test]
fn opening_a_document_while_tiled_fits_each_document_to_its_new_pane() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let side_by_side = app.views[0].zoom;
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let pane = panes(&app).1[0];
    let rect = view_rects(&app, pane, ctx.pixels_per_point()).1;
    let v = app.views[0];
    assert!((v.zoom - fit_zoom(rect, Rect::new(0.0, 0.0, 400.0, 300.0), 0.0)).abs() < 1e-9, "fitted to its new pane: {v:?}");
    assert_ne!(v.zoom, side_by_side, "the pane changed size");
}

#[test]
fn other_panes_show_pixel_preview() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.ui.view.pixel_preview = true;
    app.views[0].zoom = 4.0;
    frame(&mut app, &ctx, vec![]);
    let uid = app.session.documents()[0].uid as usize;
    let key = app.canvas.panes.shown.get(&uid).map(|(k, _)| k.clone()).unwrap();
    let [x0, y0, x1, y1] = key.pixel.expect("the pane renders document pixels");
    assert!(x1 > x0 && y1 > y0);
}

#[test]
fn a_resized_window_fits_the_tiled_documents_again_unless_one_was_zoomed() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    app.views[1].zoom *= 2.0;
    let zoomed = app.views[1];
    let smaller = vec2(900.0, 600.0);
    frame_in(&mut app, &ctx, vec![], smaller);
    let artboard = Rect::new(0.0, 0.0, 400.0, 300.0);
    let (_, rects) = panes(&app);
    for i in [0, 2] {
        let area = if i == 2 { app.canvas_rect.unwrap() } else { view_rects(&app, rects[i], ctx.pixels_per_point()).1 };
        assert!((app.views[i].zoom - fit_zoom(area, artboard, 0.0)).abs() < 1e-9, "document {i} fits its smaller pane: {:?}", app.views[i]);
    }
    assert_eq!(app.views[1].zoom, zoomed.zoom, "the zoomed document keeps its zoom");
}
