use std::collections::BTreeSet;
use std::time::Duration;

use craft::behavior::CanvasView;
use craft::ui::primitives::textures::{CachedTexture, TextureState};
use egui::{Color32, Pos2, Shape, pos2, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::{Point, Rect};

use super::*;
use crate::menus::{Node, enabled};

/// An app with `n` new 400 × 300 pt documents, the last one current.
fn app_with(n: usize) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for _ in 0..n {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    }
    app
}

/// The command items of `nodes`, in order: each one's label, command id and params.
fn commands(nodes: &[Node]) -> Vec<(&str, &str, &serde_json::Value)> {
    nodes
        .iter()
        .filter_map(|n| match n {
            Node::Item(i) => Some((i.label.as_str(), i.command.id.as_str(), &i.command.params)),
            _ => None,
        })
        .collect()
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
    // A pane's picture is uploaded once, maybe in this frame: [`wait_ready`] finds it later.
    note_uploads(&mut out.textures_delta);
    let mut shapes = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut shapes));
    shapes
}

/// A texture's picture: its pixels and size.
type Picture = (Vec<Color32>, [usize; 2]);

thread_local! {
    /// The last picture each texture got in the frames [`frame_in`] and [`wait_ready`] ran, as pixels and size.
    static UPLOADED: std::cell::RefCell<std::collections::HashMap<egui::TextureId, Picture>> = Default::default();
}

/// Notes the last picture each texture got in `delta` in [`UPLOADED`], and handles the uploads.
fn note_uploads(delta: &mut egui::TexturesDelta) {
    for (id, deltas) in &delta.set {
        if let Some(d) = deltas.last() {
            let egui::ImageData::Color(img) = &d.image;
            UPLOADED.with(|u| u.borrow_mut().insert(*id, (img.pixels.clone(), img.size)));
        }
    }
    delta.clear();
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// The documents' indices in the panes the last frame drew, and the panes.
fn panes(app: &VectorcraftApp) -> (Vec<usize>, Vec<egui::Rect>) {
    app.canvas.panes.last.iter().map(|(w, r)| (w.0, *r)).unzip()
}

/// One headless frame of the canvas panel's body as [`frame`] runs it → the whole output.
fn run_out(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::FullOutput {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        max_texture_side: Some(8192),
        ..Default::default()
    };
    ctx.run_ui(raw, |ui| app.canvas_area(ui))
}

/// Window `w`'s view as the pane body shows it, in its art area.
fn pane_view(app: &VectorcraftApp, w: Win) -> CanvasView {
    let v = *crate::doc_windows::view(app, w).unwrap();
    v.canvas(app.ui_scaling(), v.rect)
}

/// Whether window `w`'s 400 × 300 artboard is fitted to its pane: the view's extent of it lies in
/// the art area and, with the fit margin on both sides, fills it along one axis.
fn fits(app: &VectorcraftApp, w: Win) -> bool {
    let Some(v) = crate::doc_windows::view(app, w) else { return false };
    let extent = pane_view(app, w).extent(Rect::new(0.0, 0.0, 400.0, 300.0));
    let margin = 2.0 * FIT_MARGIN as f32;
    let room = v.rect.size() - vec2(margin, margin);
    v.fitted
        && extent.x <= v.rect.width() + 0.01
        && extent.y <= v.rect.height() + 0.01
        && ((extent.x - room.x).abs() < 1.0 || (extent.y - room.y).abs() < 1.0)
}

/// The source of window `w`'s pane picture, as the pane body builds it.
fn pane_source(app: &mut VectorcraftApp, ctx: &egui::Context, w: Win) -> PaneSource {
    let view = pane_view(app, w);
    // Through the document's own color settings, as `pane` draws it.
    let cms = vectorcraft_engine::cmd::colormgmt::doc_cms(&app.session.documents()[w.0].doc);
    vectorcraft_color::cms::with_active(cms, || PaneSource::new(app, ctx, w, &view)).expect("the window's document is open")
}

/// What the thumbnails hold for document `uid`'s picture in the pane of `w`, asked without
/// starting a render.
fn picture_of(app: &mut VectorcraftApp, ctx: &egui::Context, w: Win, uid: u64) -> TextureState {
    let view = pane_view(app, w);
    let source = pane_source(app, ctx, w);
    let thumbs = thumbnails(ctx);
    thumbs.hold(true);
    let request = ThumbRequest::for_cell(view.rect.width().max(view.rect.height()), ctx.pixels_per_point());
    let state = thumbs.state(ctx, &source, &PaneKey { slot: (uid, w.1) }, &request, Priority::VISIBLE);
    thumbs.hold(false);
    state
}

/// Whether the thumbnails show something for document `uid`, asked through the pane of `w`.
fn has_picture(app: &mut VectorcraftApp, ctx: &egui::Context, w: Win, uid: u64) -> bool {
    picture_of(app, ctx, w, uid).shown().is_some()
}

/// Run frames with `run` until window `w`'s pane has its picture and the upload of it was seen,
/// starting from the uploads `seed` holds → the picture and its pixels and size as uploaded. The uploads are handled
/// once read.
fn wait_ready(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    w: Win,
    seed: Vec<egui::TexturesDelta>,
    run: &mut dyn FnMut(&mut VectorcraftApp) -> egui::FullOutput,
) -> (CachedTexture, (Vec<Color32>, [usize; 2])) {
    let uid = app.session.documents()[w.0].uid;
    let mut uploads = seed;
    for _ in 0..400 {
        if let TextureState::Ready(texture) = picture_of(app, ctx, w, uid)
            && (uploads.iter().flat_map(|d| d.set.iter()).any(|e| *e.0 == texture.id) || UPLOADED.with(|u| u.borrow().contains_key(&texture.id)))
        {
            let picture = uploaded(&uploads, texture.id);
            // Another pane's picture these frames uploaded stays for a later wait.
            uploads.iter_mut().for_each(note_uploads);
            return (texture, picture);
        }
        let mut out = run(app);
        uploads.push(std::mem::take(&mut out.textures_delta));
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the pane of window {w:?} never got its picture");
}

/// The picture `id` as `uploads` uploaded it, else as a frame of [`frame_in`] did: its pixels and size.
fn uploaded(uploads: &[egui::TexturesDelta], id: egui::TextureId) -> (Vec<Color32>, [usize; 2]) {
    let image = uploads.iter().rev().flat_map(|d| d.set.iter()).find(|e| e.0.eq(&id)).and_then(|e| e.1.last()).map(|d| d.image.clone());
    match image {
        Some(egui::ImageData::Color(img)) => (img.pixels.clone(), img.size),
        None => UPLOADED.with(|u| u.borrow().get(&id).cloned()).expect("the picture was uploaded"),
    }
}

/// Drop every picture and pane the panes hold, so the next frames make them again.
fn forget(app: &mut VectorcraftApp, ctx: &egui::Context) {
    app.canvas.panes.clear();
    thumbnails(ctx).retain(ctx, |_| false);
}

/// The RGB of the middle pixel of the picture window `(i, 1)`'s pane shows once it has arrived;
/// `seed` holds the uploads of frames already run and `run` runs the frames until then.
fn pane_middle(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    i: usize,
    seed: Vec<egui::TexturesDelta>,
    run: &mut dyn FnMut(&mut VectorcraftApp) -> egui::FullOutput,
) -> [u8; 3] {
    let (_, (pixels, [w, h])) = wait_ready(app, ctx, (i, 1), seed, run);
    let p = pixels[h / 2 * w + w / 2];
    [p.r(), p.g(), p.b()]
}

/// One headless frame that draws only window `w`'s pane, in a 600 × 400 cell → the whole output.
fn pane_out(app: &mut VectorcraftApp, ctx: &egui::Context, w: Win) -> egui::FullOutput {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        max_texture_side: Some(8192),
        ..Default::default()
    };
    ctx.run_ui(raw, |ui| {
        pane(app, ui, w, egui::Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0)));
    })
}

/// A document with a 50% red rectangle over most of its artboard and a second window of it, the first window
/// floating beside the current one → the app.
fn translucent_in_two_windows() -> VectorcraftApp {
    let mut app = app_with(1);
    let half = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 300, "height": 200})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [half], "color": "#ff0000"})).unwrap();
    app.run("transparency.set", json!({"ids": [half], "opacity": 50})).unwrap();
    app.run("select.none", json!({})).unwrap();
    crate::doc_windows::new_window(&mut app).unwrap();
    app
}

/// The distinct left and top edges of `c`: how many columns and rows its panes take.
fn columns_and_rows(c: &[egui::Rect]) -> (usize, usize) {
    let lefts: BTreeSet<u32> = c.iter().map(|r| r.left().to_bits()).collect();
    let tops: BTreeSet<u32> = c.iter().map(|r| r.top().to_bits()).collect();
    (lefts.len(), tops.len())
}

#[test]
fn n_up_shows_that_many_documents_and_keeps_the_rest_under_their_tabs() {
    let mut app = app_with(5);
    let ctx = crate::testing::ui_context();
    assert_eq!(app.run("window.arrange.tile", json!({"layout": "2up"})).unwrap(), json!({"tiled": true, "panes": 2, "layout": "2up"}));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 4], "the current document takes the last pane");
    app.run("document.activate", json!({"index": 1})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 1], "the first documents in tab order");
    let second = app.session.documents()[1].uid;
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    // The second document is rendered for its pane.
    wait_ready(&mut app, &ctx, (1, 1), vec![], &mut |a| run_out(a, &ctx));
    app.run("document.activate", json!({"index": 3})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 3], "a document chosen under its tab comes into the last pane");
    assert!(!has_picture(&mut app, &ctx, (0, 1), second), "a document under its tab keeps no render");
    assert!(fits(&app, (0, 1)));
    assert!(app.canvas_view().is_some_and(|c| rects[1].contains_rect(c.rect)), "the current document's pane is the canvas");
    // A document brought into the last pane leaves the first pane's zoom as it is.
    app.views[0].zoom *= 2.0;
    let zoomed = app.views[0].zoom;
    app.run("document.activate", json!({"index": 2})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 2]);
    assert_eq!(app.views[0].zoom, zoomed, "the document that stayed in its pane keeps its zoom");
    frame(&mut app, &ctx, vec![]);
    assert!(fits(&app, (2, 1)), "the document brought in fits its pane");
    assert_eq!(app.run("window.arrange.tile", json!({"layout": "4up"})).unwrap()["panes"], json!(4));
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!((docs, columns_and_rows(&rects)), (vec![0, 1, 2, 3], (2, 2)));
    assert_eq!(app.run("window.arrange.tile", json!({"layout": "6up"})).unwrap()["panes"], json!(5), "6-Up with five documents shows them all");
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!((docs, columns_and_rows(&rects)), (vec![0, 1, 2, 3, 4], (3, 2)));
    assert_eq!(app.run("window.arrange.tile", json!({"layout": "3up"})).unwrap()["panes"], json!(3));
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!((docs, columns_and_rows(&rects)), (vec![0, 1, 2], (3, 1)));
    app.run("document.activate", json!({"index": 4})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 1, 4]);
}

/// A document closed before the panes shifts the indices of the ones after it: the document that
/// moves into another pane fits it.
#[test]
fn closing_a_document_in_an_n_up_layout_fits_the_documents_that_change_panes() {
    let mut app = app_with(4);
    let ctx = crate::testing::ui_context();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.tile", json!({"layout": "2up"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("document.activate", json!({"index": 3})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 3]);
    let moving = app.session.documents()[1].uid;
    // The second document's view, under its tab, is zoomed away from its fit.
    app.views[1].zoom *= 3.0;
    app.run("file.close", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let (docs, _) = panes(&app);
    assert_eq!(docs, [0, 2], "the former second document is first now, the current one stays last");
    assert_eq!(app.session.documents()[0].uid, moving);
    assert!(fits(&app, (0, 1)), "it fits its new pane: {:?}", app.views[0]);
}

/// 6-Up keeps its three columns and two rows with five documents: closing the sixth leaves the
/// others in their panes with the zoom they had. Side by side, closing one widens every pane, and
/// each document fits its new pane.
#[test]
fn closing_a_document_keeps_the_zoom_of_the_documents_whose_panes_stay() {
    let mut app = app_with(6);
    let ctx = crate::testing::ui_context();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.tile", json!({"layout": "6up"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 1, 2, 3, 4, 5]);
    app.views[1].zoom *= 2.0;
    let zoomed = app.views[1].zoom;
    app.run("file.close", json!({"index": 5})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!((docs, columns_and_rows(&rects)), (vec![0, 1, 2, 3, 4], (3, 2)));
    assert_eq!(app.views[1].zoom, zoomed, "the document that kept its pane keeps its zoom");

    let mut app = app_with(3);
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.tile", json!({"layout": "vertical"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.views[1].zoom *= 2.0;
    app.run("file.close", json!({"index": 2})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let (docs, _) = panes(&app);
    assert_eq!(docs, [0, 1]);
    assert!(fits(&app, (1, 1)), "it fits its wider pane: {:?}", app.views[1]);
}

#[test]
fn tile_refuses_a_layout_it_does_not_have() {
    let mut app = app_with(3);
    for bad in [json!("5up"), json!("Vertical"), json!(3), json!(null), json!({})] {
        let err = app.run("window.arrange.tile", json!({"layout": bad})).unwrap_err();
        assert_eq!(err, "`layout` must be one of grid, vertical, horizontal, 2up, 3up, 4up, 6up", "{bad}");
        assert!(!app.ui.tiled && app.ui.tile_layout == Layout::Grid, "{bad}: the view stays as it is");
    }
    app.run("window.arrange.tile", json!({"layout": "vertical"})).unwrap();
    assert!(app.run("window.arrange.tile", json!({"layout": "diagonal"})).is_err());
    assert_eq!(app.ui.tile_layout, Layout::Vertical, "a refused layout leaves the tiled one");
    // With one document the layout is still checked, and nothing changes.
    let mut one = app_with(1);
    assert!(one.run("window.arrange.tile", json!({"layout": "7up"})).is_err());
    assert_eq!(one.run("window.arrange.tile", json!({"layout": "2up"})).unwrap(), json!({"tiled": false, "panes": 1, "layout": "grid"}));
}

#[test]
fn the_tile_items_check_the_layout_the_panes_show() {
    let mut app = app_with(3);
    let checked = |app: &VectorcraftApp, layout: Option<&str>| {
        let p = layout.map_or(serde_json::Value::Null, |l| json!({"layout": l}));
        crate::menus::checked(app, "window.arrange.tile", &p)
    };
    assert_eq!(checked(&app, None), Some(false), "not tiled");
    app.run("window.arrange.tile", json!({"layout": "4up"})).unwrap();
    assert_eq!(checked(&app, Some("4up")), Some(true));
    assert_eq!(checked(&app, None), Some(false));
    assert_eq!(checked(&app, Some("2up")), Some(false));
    app.run("window.arrange.tile", json!({})).unwrap();
    assert_eq!(checked(&app, None), Some(true), "Tile is the grid");
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    assert_eq!(checked(&app, None), Some(false));
    // Window › Arrange lists every layout, after Cascade and before Float in Window.
    let window = crate::menus::menu_tree().into_iter().find(|(t, _)| *t == "Window").map(|(_, items)| items).unwrap();
    let arrange = window
        .into_iter()
        .find_map(|i| match i {
            Node::Submenu { label, children, .. } if label == "Arrange" => Some(children),
            _ => None,
        })
        .unwrap();
    let labels: Vec<&str> = commands(&arrange).into_iter().map(|(l, ..)| l).collect();
    assert_eq!(
        labels,
        [
            "Cascade",
            "Tile",
            "Tile All Vertically",
            "Tile All Horizontally",
            "2-Up Layout",
            "3-Up Layout",
            "4-Up Layout",
            "6-Up Layout",
            "Float in Window",
            "Float All in Windows",
            "Consolidate All Windows"
        ]
    );
    // Each item's params run, as the menu sends them.
    for (_, id, p) in commands(&arrange) {
        if id == "window.arrange.tile" {
            let p = if p.is_null() { json!({}) } else { p.clone() };
            assert!(app.run(id, p.clone()).is_ok(), "{p}");
        }
    }
}

/// The Arrange Documents items tile and consolidate through the commands they name.
#[test]
fn the_arrange_documents_items_tile_and_consolidate() {
    let mut app = app_with(3);
    let items = crate::menus::arrange_documents_items();
    let items = commands(&items);
    let labels: Vec<&str> = items.iter().map(|(l, ..)| *l).collect();
    for item in [
        "Consolidate All Windows",
        "Tile",
        "Tile All Vertically",
        "Tile All Horizontally",
        "2-Up Layout",
        "3-Up Layout",
        "4-Up Layout",
        "6-Up Layout",
    ] {
        assert!(labels.contains(&item), "the items list {item}");
    }
    let choose = |app: &mut VectorcraftApp, label: &str| {
        let (_, id, p) = items.iter().find(|(l, ..)| *l == label).unwrap_or_else(|| panic!("{label} is listed"));
        app.run(id, if p.is_null() { json!({}) } else { (*p).clone() }).unwrap();
    };
    choose(&mut app, "Tile All Vertically");
    assert!(app.ui.tiled && app.ui.tile_layout == Layout::Vertical, "{}", app.ui.status);
    choose(&mut app, "Consolidate All Windows");
    assert!(!app.ui.tiled);
}

#[test]
fn tile_shows_a_pane_for_each_document_fitted_to_it() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty(), "documents open under their tabs");
    let tabbed = app.canvas_view().unwrap().rect;
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": true, "panes": 3, "layout": "grid"}));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 1, 2]);
    // Three documents: two columns and two rows, the lower-right cell empty.
    assert_eq!(rects[0].top(), rects[1].top());
    assert_eq!(rects[0].left(), rects[2].left());
    assert!(rects[2].top() > rects[0].top());
    let canvas = app.canvas_view().unwrap().rect;
    // The scroll bars are drawn inside the canvas (cv.S2), which fills its half-wide pane.
    assert!(rects[2].contains_rect(canvas) && canvas.width() <= tabbed.width() / 2.0, "the current document's pane is the canvas: {canvas:?}");
    let artboard = Rect::new(0.0, 0.0, 400.0, 300.0);
    for i in &docs {
        let v = app.views[*i];
        assert!(fits(&app, (*i, 1)), "document {i} fits its pane: {v:?}");
        assert_eq!(v.center, artboard.center());
    }
    // The other two documents are rendered for their panes.
    for i in [0, 1] {
        wait_ready(&mut app, &ctx, (i, 1), vec![], &mut |a| run_out(a, &ctx));
    }
    let inspected = crate::control::inspect(&app, &ctx);
    assert_eq!(inspected["tiled"], json!(true));
    let active: Vec<bool> = inspected["panes"].as_array().unwrap().iter().map(|p| p["active"].as_bool().unwrap()).collect();
    assert_eq!(active, [false, false, true]);
}

/// One headless frame at `time` seconds as the app runs it: the tabs and the panes draw.
fn timed_frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: Option<f64>, events: Vec<egui::Event>) -> egui::FullOutput {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        max_texture_side: Some(8192),
        time,
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| app.canvas_area(ui));
    out.textures_delta.clear();
    out
}

/// [`timed_frame`] without a time. → the cursor.
fn app_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::CursorIcon {
    timed_frame(app, ctx, None, events).platform_output.cursor_icon
}

/// Two 400 × 300 pt documents tiled side by side: the first, current, holds a selected 100 pt
/// square at 50, 50 and the second a 50 pt square at 300, 230. The Selection tool is chosen.
fn tiled_pair() -> (VectorcraftApp, egui::Context) {
    let mut app = app_with(0);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 300, "y": 230, "width": 50, "height": 50})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.run("window.arrange.tile", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    app_frame(&mut app, &ctx, vec![]);
    app_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

/// Press on document point `at` on the canvas and move the pointer 20 × 10 points, still on the
/// canvas: the Selection tool moves the art under it.
fn grab(app: &mut VectorcraftApp, ctx: &egui::Context, at: Point) {
    let p = app.canvas_view().unwrap().to_screen(at);
    app_frame(app, ctx, vec![egui::Event::PointerMoved(p), button(p, true)]);
    app_frame(app, ctx, vec![egui::Event::PointerMoved(p + vec2(20.0, 10.0))]);
    assert!(app.session.in_interaction(), "the Selection tool moves the art");
}

/// Window `w`'s pane: the screen point that shows document point `at` in it, and the pane.
fn in_pane(app: &VectorcraftApp, _ctx: &egui::Context, w: Win, at: Point) -> (Pos2, egui::Rect) {
    let (_, cell) = app.canvas.panes.last.iter().copied().find(|(o, _)| *o == w).unwrap();
    (pane_view(app, w).to_screen(at), cell)
}

/// How many objects document `i`'s first layer holds.
fn objects(app: &VectorcraftApp, i: usize) -> usize {
    app.session.documents()[i].doc.layers[0].children().map_or(0, Vec::len)
}

/// The bounds of the one object selected in the current document.
fn selected_bounds(app: &VectorcraftApp) -> Rect {
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects.len(), 1, "one object is selected");
    st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap()
}

#[test]
fn a_selection_drag_released_in_another_documents_pane_copies_the_art_there() {
    let (mut app, ctx) = tiled_pair();
    // The target's view differs from the source's: the grab point still lands under the pointer.
    app.views[1].zoom *= 0.6;
    app.views[1].rotation = 30.0;
    app_frame(&mut app, &ctx, vec![]);
    let source = app.session.documents()[0].doc.clone();
    let source_undo = app.session.documents()[0].history.undo.len();
    let target = app.session.documents()[1].doc.clone();
    let target_undo = app.session.documents()[1].history.undo.len();
    let clipboard = app.session.clipboard.clone();
    grab(&mut app, &ctx, Point::new(100.0, 100.0));
    let (drop, _) = in_pane(&app, &ctx, (1, 1), Point::new(170.0, 120.0));
    // The release comes in the first frame the pointer is over the pane.
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop), button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1), "the pane's document is the current one");
    let placed = selected_bounds(&app);
    assert!((placed.center().x - 170.0).abs() < 0.01 && (placed.center().y - 120.0).abs() < 0.01, "{placed:?}");
    assert_eq!(app.session.documents()[1].history.undo.len(), target_undo + 1, "one step to undo");
    assert_eq!(app.session.documents()[0].doc, source, "the source keeps its art where it was");
    assert_eq!(app.session.documents()[0].history.undo.len(), source_undo);
    assert_eq!(app.session.clipboard, clipboard, "the drag leaves the clipboard alone");
    assert!(!app.session.tool_busy());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.documents()[1].doc, target);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.documents()[1].history.undo.len(), target_undo + 1);
}

#[test]
fn art_held_over_another_pane_shows_a_copy_and_drags_back_the_other_way() {
    let (mut app, ctx) = tiled_pair();
    grab(&mut app, &ctx, Point::new(100.0, 100.0));
    let (over, _) = in_pane(&app, &ctx, (1, 1), Point::new(200.0, 150.0));
    let cursor = app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over)]);
    assert_eq!(cursor, egui::CursorIcon::Copy, "over another document's pane the art is copied");
    assert_eq!(app.session.active_index(), Some(0), "holding the art over a pane does not switch documents");
    let (drop, _) = in_pane(&app, &ctx, (1, 1), Point::new(120.0, 90.0));
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    app_frame(&mut app, &ctx, vec![button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(objects(&app, 1), 2, "the copy joins the target's square");
    // From the target, now the canvas, back into the first document's pane.
    app_frame(&mut app, &ctx, vec![]);
    grab(&mut app, &ctx, Point::new(325.0, 255.0));
    let (back, _) = in_pane(&app, &ctx, (0, 1), Point::new(300.0, 200.0));
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(back), button(back, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(0));
    let placed = selected_bounds(&app);
    assert!((placed.center().x - 300.0).abs() < 0.01 && (placed.center().y - 200.0).abs() < 0.01, "{placed:?}");
    assert_eq!(objects(&app, 0), 2);
}

#[test]
fn a_selection_drag_into_another_view_of_its_document_moves_the_art() {
    let (mut app, ctx) = tiled_pair();
    app.run("window.newWindow", json!({})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    app_frame(&mut app, &ctx, vec![]);
    app_frame(&mut app, &ctx, vec![]);
    let current = crate::doc_windows::current(&app).unwrap();
    let other = app.canvas.panes.last.iter().map(|(w, _)| *w).find(|w| w.0 == 0 && *w != current).unwrap();
    let undo = app.session.documents()[0].history.undo.len();
    let target = app.session.documents()[1].doc.clone();
    grab(&mut app, &ctx, Point::new(100.0, 100.0));
    let (drop, _) = in_pane(&app, &ctx, other, Point::new(250.0, 200.0));
    let cursor = app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    assert_eq!(cursor, egui::CursorIcon::Move, "another view of the same document moves the art");
    app_frame(&mut app, &ctx, vec![button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(crate::doc_windows::current(&app), Some(other), "the pane's view is the current window");
    assert_eq!(objects(&app, 0), 1, "the square moved, not copied");
    let moved = selected_bounds(&app);
    assert!((moved.center().x - 250.0).abs() < 0.01 && (moved.center().y - 200.0).abs() < 0.01, "{moved:?}");
    assert_eq!(app.session.documents()[0].history.undo.len(), undo + 1);
    assert_eq!(app.session.documents()[1].doc, target, "the other document is untouched");
}

#[test]
fn a_pane_drop_lands_where_the_button_went_up_and_copies_when_alt_was_held_then() {
    // A pointer move later in the frame of the release does not move the drop.
    let (mut app, ctx) = tiled_pair();
    grab(&mut app, &ctx, Point::new(100.0, 100.0));
    let (drop, _) = in_pane(&app, &ctx, (1, 1), Point::new(170.0, 120.0));
    let (later, _) = in_pane(&app, &ctx, (1, 1), Point::new(80.0, 60.0));
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop), button(drop, false), egui::Event::PointerMoved(later)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
    let placed = selected_bounds(&app);
    assert!((placed.center().x - 170.0).abs() < 0.01 && (placed.center().y - 120.0).abs() < 0.01, "{placed:?}");
    // Alt held at the release and let go later in that frame copies into another view of the source.
    let (mut app, ctx) = tiled_pair();
    app.run("window.newWindow", json!({})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    app_frame(&mut app, &ctx, vec![]);
    app_frame(&mut app, &ctx, vec![]);
    let current = crate::doc_windows::current(&app).unwrap();
    let other = app.canvas.panes.last.iter().map(|(w, _)| *w).find(|w| w.0 == 0 && *w != current).unwrap();
    grab(&mut app, &ctx, Point::new(100.0, 100.0));
    let (drop, _) = in_pane(&app, &ctx, other, Point::new(250.0, 200.0));
    let alt = egui::Modifiers::ALT;
    let cursor = app_frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(drop)]);
    assert_eq!(cursor, egui::CursorIcon::Copy, "Alt shows a copy");
    let up = egui::Event::PointerButton { pos: drop, button: egui::PointerButton::Primary, pressed: false, modifiers: alt };
    app_frame(&mut app, &ctx, vec![up, egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(crate::doc_windows::current(&app), Some(other));
    assert_eq!(objects(&app, 0), 2, "the square was copied, not moved");
}

#[test]
fn art_carried_through_a_tab_drops_into_a_third_documents_pane() {
    let (mut app, ctx) = tiled_pair();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    let source = app.session.documents()[0].doc.clone();
    let third = app.session.documents()[2].doc.clone();
    // A tab's title starts with its document's name (an inactive tab shows the name alone).
    let names: Vec<String> = app.session.documents().iter().map(|d| d.title()).collect();
    let titles = |out: egui::FullOutput| -> Vec<egui::Rect> {
        let mut t: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) if names.iter().any(|n| t.galley.text().starts_with(n.as_str())) => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                }
                _ => None,
            })
            .collect();
        t.sort_by(|a, b| a.left().total_cmp(&b.left()));
        t
    };
    timed_frame(&mut app, &ctx, Some(0.0), vec![]);
    let tabs = titles(timed_frame(&mut app, &ctx, Some(0.01), vec![]));
    assert_eq!(tabs.len(), 3);
    let p = app.canvas_view().unwrap().to_screen(Point::new(100.0, 100.0));
    timed_frame(&mut app, &ctx, Some(0.05), vec![egui::Event::PointerMoved(p), button(p, true)]);
    timed_frame(&mut app, &ctx, Some(0.1), vec![egui::Event::PointerMoved(p + vec2(20.0, 10.0))]);
    timed_frame(&mut app, &ctx, Some(0.2), vec![egui::Event::PointerMoved(tabs[2].center())]);
    timed_frame(&mut app, &ctx, Some(0.25), vec![]);
    // A tab under a drag becomes current after the pointer rests the tooltip delay (cv.F23).
    timed_frame(&mut app, &ctx, Some(0.3 + craft::ui::contract::tooltip::DELAY), vec![]);
    assert_eq!(app.session.active_index(), Some(2), "holding the art over the third tab makes that document current");
    timed_frame(&mut app, &ctx, Some(1.0), vec![]);
    let (drop, _) = in_pane(&app, &ctx, (1, 1), Point::new(150.0, 100.0));
    timed_frame(&mut app, &ctx, Some(1.05), vec![egui::Event::PointerMoved(drop)]);
    timed_frame(&mut app, &ctx, Some(1.15), vec![button(drop, false)]);
    timed_frame(&mut app, &ctx, Some(1.25), vec![]);
    assert_eq!(app.session.active_index(), Some(1), "the pane's document is the current one");
    assert_eq!(objects(&app, 1), 2, "the copy joins the second document's square");
    let placed = selected_bounds(&app);
    assert!((placed.center().x - 150.0).abs() < 0.01 && (placed.center().y - 100.0).abs() < 0.01, "{placed:?}");
    assert_eq!(app.session.documents()[0].doc, source);
    assert_eq!(app.session.documents()[2].doc, third, "the document the tab showed is untouched");
}

#[test]
fn a_drag_released_off_a_panes_art_area_or_after_escape_changes_nothing() {
    for (case, escape) in [("ruler", false), ("art area after Escape", true)] {
        let (mut app, ctx) = tiled_pair();
        // A pane's vertical ruler runs down its left side, outside its art area.
        app.ui.view.rulers = true;
        let docs: Vec<_> = app.session.documents().iter().map(|d| (d.doc.clone(), d.history.undo.len())).collect();
        grab(&mut app, &ctx, Point::new(100.0, 100.0));
        let (inside, cell) = in_pane(&app, &ctx, (1, 1), Point::new(200.0, 150.0));
        let at = if escape { inside } else { pos2(cell.left() + 2.0, cell.center().y) };
        app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        if escape {
            let key = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
            app_frame(&mut app, &ctx, vec![key]);
        }
        app_frame(&mut app, &ctx, vec![button(at, false)]);
        app_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.session.active_index(), Some(0), "{case}: the source stays current");
        let after: Vec<_> = app.session.documents().iter().map(|d| (d.doc.clone(), d.history.undo.len())).collect();
        assert!(after == docs, "{case}: neither document changes");
        assert!(!app.session.in_interaction(), "{case}: the move ended");
    }
}

#[test]
fn consolidate_all_windows_shows_one_document_again() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let tabbed = app.canvas_view().map(|v| v.rect);
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0.len(), 2);
    let uid = app.session.documents()[0].uid;
    wait_ready(&mut app, &ctx, (0, 1), vec![], &mut |a| run_out(a, &ctx));
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({})).unwrap(), json!({"tiled": false, "floating": []}));
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty());
    assert!(!has_picture(&mut app, &ctx, (1, 1), uid), "no pane keeps a picture");
    assert_eq!(app.canvas_view().map(|v| v.rect), tabbed, "the current document fills the document area again");
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
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(items(&app), (false, false), "every document closed");
    assert!(!app.ui.tiled);
}

#[test]
fn tile_with_one_document_leaves_the_next_one_under_its_tab() {
    let mut app = app_with(1);
    let ctx = crate::testing::ui_context();
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": false, "panes": 1, "layout": "grid"}));
    assert!(!app.ui.tiled);
    assert!(!enabled(&app, "window.arrange.consolidateAllWindows"));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty(), "the second document opens under its tab");
}

#[test]
fn closing_a_document_while_tiled_tiles_the_others() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 1, 2]);
    let first = app.session.documents()[0].uid;
    wait_ready(&mut app, &ctx, (0, 1), vec![], &mut |a| run_out(a, &ctx));
    // The first document, not the current one, closes.
    app.run("file.close", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 1]);
    assert_eq!(rects[0].height(), rects[1].height(), "two panes side by side");
    assert!(!has_picture(&mut app, &ctx, (0, 1), first), "the closed document's render is dropped");
    app.run("file.close", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty(), "one document fills the document area");
    assert!(enabled(&app, "window.arrange.consolidateAllWindows"));
}

#[test]
fn an_edit_shows_in_its_pane_once_another_document_is_current() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let before = pane_source(&mut app, &ctx, (1, 1)).frame.revision;
    assert_eq!(pane_source(&mut app, &ctx, (1, 1)).frame.revision, before, "without an edit the pane's picture stays");
    app.run("document.activate", json!({"index": 1})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 80})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_ne!(pane_source(&mut app, &ctx, (1, 1)).frame.revision, before, "the pane shows the document as edited");
}

#[test]
fn other_panes_draw_their_guides_and_selected_objects() {
    let mut app = app_with(2);
    app.run("document.activate", json!({"index": 0})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 300})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    // The pane hands its document's guides and selected objects to the overlays it shows.
    let st = &app.session.documents()[0];
    let mut tokens = None;
    crate::testing::ui_context()
        .run_ui(Default::default(), |ui| tokens = Some(craft::theme::Tokens::clone(Env::of(ui).tokens.tokens())))
        .textures_delta
        .clear();
    let guides = format!("{:?}", crate::canvas::guides::shown(&app, st, &tokens.unwrap()));
    assert!(guides.contains("300"), "the guide at 300 is shown in the pane: {guides}");
    assert!(!crate::canvas::selection::outlines(&app, st).is_empty(), "the selected rectangle is outlined in its pane");
}

#[test]
fn opening_a_document_while_tiled_fits_each_document_to_its_new_pane() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let side_by_side = app.views[0].zoom;
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let v = app.views[0];
    assert!(fits(&app, (0, 1)), "fitted to its new pane: {v:?}");
    assert_ne!(v.zoom, side_by_side, "the pane changed size");
}

#[test]
fn other_panes_show_pixel_preview() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.ui.view.pixel_preview = true;
    app.views[0].zoom = 4.0;
    frame(&mut app, &ctx, vec![]);
    let [x0, y0, x1, y1] = pane_source(&mut app, &ctx, (0, 1)).frame.cache.pixel.expect("the pane renders document pixels");
    assert!(x1 > x0 && y1 > y0);
}

/// A pane more device pixels across than the largest texture side the display supports renders
/// its document within that size, as the canvas does, and the frame covers the pane.
#[test]
fn other_panes_render_within_the_largest_texture_side() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    ctx.set_pixels_per_point(2.0);
    app.run("window.arrange.tile", json!({})).unwrap();
    let mut run = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
            max_texture_side: Some(1024),
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| app.canvas_area(ui))
    };
    // The first frames set the pixels a point and the panes' areas, the next draw at them.
    for _ in 0..3 {
        run(&mut app).textures_delta.clear();
    }
    let (texture, _) = wait_ready(&mut app, &ctx, (0, 1), vec![], &mut run);
    let (w, h) = (texture.size.width as f32, texture.size.height as f32);
    assert_eq!(w.max(h), 1024.0, "the pane's frame is {w} × {h} pixels");
    let rect = crate::doc_windows::view(&app, (0, 1)).unwrap().rect;
    assert!(rect.width().max(rect.height()) * ctx.pixels_per_point() > 1024.0, "the pane is {rect:?} at {} pixels a point", ctx.pixels_per_point());
    assert!((w / h - rect.width() / rect.height()).abs() < 0.02, "the frame covers the pane {rect:?}: {w} × {h} pixels");
}

/// A pane's render key takes its document's [`CacheKey::placed`] value ([`placed_key`]), which a
/// bitmap made for a test running beside this one does not move: the document holds no images.
#[test]
fn a_panes_key_takes_its_documents_placed_value() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let before = pane_source(&mut app, &ctx, (0, 1)).frame.revision;
    let uid = app.session.documents()[0].uid;
    let generation = vectorcraft_render::placed_document::generation();
    let marked = generation.wrapping_add(1000);
    app.canvas.placed_keys.insert(uid, crate::PlacedKey { checked: generation, key: marked, before: (marked, generation) });
    assert_ne!(pane_source(&mut app, &ctx, (0, 1)).frame.revision, before, "the picture follows the document's placed value");
}

#[test]
fn a_resized_window_fits_the_tiled_documents_again_unless_one_was_zoomed() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    app.views[1].zoom *= 2.0;
    let zoomed = app.views[1];
    let smaller = vec2(900.0, 600.0);
    frame_in(&mut app, &ctx, vec![], smaller);
    frame_in(&mut app, &ctx, vec![], smaller);
    frame_in(&mut app, &ctx, vec![], smaller);
    for i in [0, 2] {
        assert!(fits(&app, (i, 1)), "document {i} fits its smaller pane: {:?}", app.views[i]);
    }
    assert_eq!(app.views[1].zoom, zoomed.zoom, "the zoomed document keeps its zoom");
}

#[test]
fn other_panes_draw_through_their_documents_profiles() {
    const CHILD: &str = "VECTORCRAFT_TEST_PANE_PROFILES";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: assign profiles in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "canvas::panes::tests::other_panes_draw_through_their_documents_profiles", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated pane profile test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    use vectorcraft_color::cms::{self, Cms, ColorSettings};
    let mut app = app_with(2);
    app.session.prefs.monitor_profile_display = false;
    // Each document is covered by one RGB rectangle; the second is assigned Display P3.
    for i in 0..2 {
        app.run("document.activate", json!({"index": i})).unwrap();
        app.run("shape.rectangle", json!({"x": -50, "y": -50, "width": 500, "height": 400})).unwrap();
        app.run("paint.setFill", json!({"color": "#339966"})).unwrap();
    }
    app.run("edit.assignProfile", json!({"rgb": cms::DISPLAY_P3})).unwrap();
    // Its simulated paper shows in another color.
    app.run("document.setup", json!({"gridColors": ["#cc6633", "#ffffff"], "simulatePaper": true})).unwrap();
    let in_p3 = |r: u8, g: u8, b: u8| {
        let color = vectorcraft_color::Color::Rgb { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0 };
        let p3 = Cms::new(&ColorSettings { rgb: cms::DISPLAY_P3.into(), ..cms::working_settings() }).unwrap().display_rgb(&color);
        (color, p3.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
    };
    let (color, p3) = in_p3(0x33, 0x99, 0x66);
    let srgb = [0x33, 0x99, 0x66];
    assert_ne!(p3, srgb, "Display P3 draws the color differently");
    let (_, paper_p3) = in_p3(0xcc, 0x66, 0x33);
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1);
    // Whether `out` fills a shape with the RGB `rgb`, give or take one. The pasteboard draws the
    // paper as an untextured mesh.
    let fills = |out: &egui::FullOutput, rgb: [u8; 3]| {
        fn any(s: &Shape, f: &dyn Fn(Color32) -> bool) -> bool {
            match s {
                Shape::Vec(v) => v.iter().any(|s| any(s, f)),
                Shape::Path(p) => f(p.fill),
                Shape::Rect(r) => f(r.fill),
                Shape::Mesh(m) if m.texture_id == egui::TextureId::default() => m.vertices.iter().any(|v| f(v.color)),
                _ => false,
            }
        }
        out.shapes.iter().any(|c| any(&c.shape, &|c: Color32| c.a() == 255 && near([c.r(), c.g(), c.b()], rgb)))
    };
    let ctx = crate::testing::ui_context();
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        max_texture_side: Some(8192),
        ..Default::default()
    };
    app.run("window.arrange.tile", json!({})).unwrap();
    // The first document in front: the second one's pane draws through Display P3, its paper too.
    app.run("document.activate", json!({"index": 0})).unwrap();
    let mut out = ctx.run_ui(raw(), |ui| app.canvas_area(ui));
    assert!(fills(&out, paper_p3), "the back document's paper draws through its Display P3: {paper_p3:?}");
    assert!(!fills(&out, [0xcc, 0x66, 0x33]), "the back document's paper does not draw through the working space");
    let shown = pane_middle(&mut app, &ctx, 1, vec![std::mem::take(&mut out.textures_delta)], &mut |a| ctx.run_ui(raw(), |ui| a.canvas_area(ui)));
    assert!(near(shown, p3), "the back document draws through its Display P3: {shown:?}, not {p3:?}");
    // The same in a floating window.
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    let floating = |app: &mut VectorcraftApp| {
        ctx.run_ui(raw(), |ui| {
            app.canvas_area(ui);
            crate::doc_windows::show(app, ui);
        })
    };
    // The window takes its place in the first frame and fades in over the next few.
    for _ in 0..30 {
        floating(&mut app).textures_delta.clear();
    }
    forget(&mut app, &ctx);
    let mut out = floating(&mut app);
    assert!(fills(&out, paper_p3), "the floating document's paper draws through its Display P3: {paper_p3:?}");
    let shown = pane_middle(&mut app, &ctx, 1, vec![std::mem::take(&mut out.textures_delta)], &mut |a| floating(a));
    assert!(near(shown, p3), "the floating document draws through its Display P3: {shown:?}, not {p3:?}");
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    forget(&mut app, &ctx);
    // The second document in front: the first one's pane draws through the working sRGB.
    app.run("document.activate", json!({"index": 1})).unwrap();
    let mut out = ctx.run_ui(raw(), |ui| app.canvas_area(ui));
    let shown = pane_middle(&mut app, &ctx, 0, vec![std::mem::take(&mut out.textures_delta)], &mut |a| ctx.run_ui(raw(), |ui| a.canvas_area(ui)));
    assert!(near(shown, srgb), "the back document draws through the working space: {shown:?}, not {srgb:?}");
    // A CMYK document behind the front one draws through its own assigned CMYK profile.
    app.run("document.activate", json!({"index": 0})).unwrap();
    let steps = json!([
        {"command": "edit.assignProfile", "params": {"cmyk": cms::DEVICE_CMYK}},
        {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("paint.setFill", json!({"color": {"c": 60, "m": 10, "y": 80, "k": 0}})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    let ink = vectorcraft_color::Color::cmyk(0.6, 0.1, 0.8, 0.0);
    let to_u8 = |c: [f32; 3]| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let device = to_u8(Cms::new(&ColorSettings { cmyk: cms::DEVICE_CMYK.into(), ..cms::working_settings() }).unwrap().display_rgb(&ink));
    let front = to_u8(cms::active().display_rgb(&ink));
    let close = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3);
    assert!(!close(device, front), "the two CMYK profiles draw the color differently: {device:?}, {front:?}");
    forget(&mut app, &ctx);
    let shown = pane_middle(&mut app, &ctx, 0, vec![], &mut |a| ctx.run_ui(raw(), |ui| a.canvas_area(ui)));
    assert!(close(shown, device), "the back CMYK document draws through its Device CMYK: {shown:?}, not {device:?} (front: {front:?})");
    assert!(near(cms::active().display_rgb(&color).map(|v| (v * 255.0).round() as u8), p3), "the front document's profile stays active");
}

/// With a display profile, the tiled pane of a document behind the front one blends its
/// translucent art with that document's own simulated paper in sRGB, then converts the blend:
/// drawn over its converted paper, the pane shows the converted blend, not the art converted alone
/// and not a blend with the front document's paper. Opaque and empty pixels convert as before.
#[test]
fn tiled_documents_blend_translucent_art_over_their_own_paper_before_display_conversion() {
    use crate::display_profile::{Found, Reading};
    let mut app = app_with(2);
    let (front_paper, back_paper) = ([0x20, 0x40, 0xc0], [0xe0, 0xc0, 0x20]);
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("document.setup", json!({"gridColors": ["#2040c0", "#ffffff"], "simulatePaper": true})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("document.setup", json!({"gridColors": ["#e0c020", "#ffffff"], "simulatePaper": true})).unwrap();
    let half = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 300, "height": 200})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [half], "color": "#ff0000"})).unwrap();
    app.run("transparency.set", json!({"ids": [half], "opacity": 50})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    let ctx = crate::testing::ui_context();
    // The second document's pane: the picture it shows, and its width.
    let pane = |app: &mut VectorcraftApp| -> (Vec<Color32>, usize) {
        forget(app, &ctx);
        let (_, (pixels, [width, _])) = wait_ready(app, &ctx, (1, 1), vec![], &mut |a| run_out(a, &ctx));
        (pixels, width)
    };
    app.session.prefs.monitor_profile_display = false;
    let (srgb, width) = pane(&mut app);
    assert!(pane_source(&mut app, &ctx, (1, 1)).frame.cache.over().is_none(), "no display profile: nothing to blend");
    let p3 = crate::display_profile::tests::p3();
    app.display_profile.report(Reading { display: Some("DP-1".into()), found: Found::Profile { icc: p3, from: "test".into() } });
    app.session.prefs.monitor_profile_display = true;
    let t = app.display_profile.transform(true).expect("the reported profile gives a transform");
    let (shown, _) = pane(&mut app);
    let source = pane_source(&mut app, &ctx, (1, 1));
    let (backdrop, _) = source.frame.cache.over().expect("the pane's Preview blends with its paper");
    assert!(
        !backdrop.artboards.is_empty() && backdrop.artboards.iter().all(|(_, c)| *c == back_paper),
        "the pane blends with its own document's paper: {backdrop:?}"
    );
    assert_eq!(srgb.len(), shown.len());
    // What the canvas shows of pixel `c` over the converted paper `under`, as egui blends it.
    let over = |c: Color32, under: [u8; 3]| {
        let k = 255 - u32::from(c.a());
        [0, 1, 2].map(|i| u32::from(c.to_array()[i]) + (u32::from(under[i]) * k + 127) / 255)
    };
    let plain = |s: Color32| {
        let mut px = s.to_array();
        t.apply_premultiplied_rgba8(&mut px);
        Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
    };
    // One opaque sRGB color, converted.
    let apply8 = |[r, g, b]: [u8; 3]| {
        let c = plain(Color32::from_rgb(r, g, b));
        [c.r(), c.g(), c.b()]
    };
    // Pixel `s` over the paper `paper` in sRGB, converted.
    let blend = |s: Color32, paper: [u8; 3]| {
        let k = 255 - u32::from(s.a());
        apply8([0, 1, 2].map(|j| (u32::from(s.to_array()[j]) + (u32::from(paper[j]) * k + 127) / 255).min(255) as u8)).map(u32::from)
    };
    let differs = |a: [u32; 3], b: [u32; 3]| (0..3).any(|j| a[j].abs_diff(b[j]) > 2);
    let under = apply8(back_paper);
    for (s, c) in srgb.iter().zip(&shown) {
        if s.a() == 0 || s.a() == 255 {
            assert_eq!(*c, plain(*s), "opaque and empty pixels convert as before");
        }
    }
    // The middle of the pane is the middle of the document, inside the 50% fill.
    let at = srgb.len() / width / 2 * width + width / 2;
    let (s, c) = (srgb[at], shown[at]);
    assert!((50..=205).contains(&s.a()), "the 50% fill at the middle: {s:?}");
    assert_eq!(c.a(), s.a(), "the alpha is kept");
    let want = blend(s, back_paper);
    let got = over(c, under);
    assert!((0..3).all(|j| got[j].abs_diff(want[j]) <= 1), "{got:?} shown, {want:?} wanted");
    // The check tells the two papers and the plain conversion apart.
    assert!(differs(want, blend(s, front_paper)), "the front document's paper blends to another color");
    assert!(differs(got, over(plain(s), under)), "the art converted alone shows another color");
}

/// A pane draws its own view's Outline, whatever the current view shows: in Preview the middle of its picture shows
/// the rectangle's red fill, in Outline it does not, and its picture follows its own Outline, not the current view's.
#[test]
fn a_pane_draws_its_own_views_outline() {
    let mut app = translucent_in_two_windows();
    let ctx = crate::testing::ui_context();
    let other = (0, 1);
    let red = |[r, g, b]: [u8; 3]| r > g.saturating_add(60) && r > b.saturating_add(60);
    let shown = |app: &mut VectorcraftApp, pane_outline: bool, current_outline: bool| {
        crate::doc_windows::view_mut(app, other).unwrap().outline = pane_outline;
        app.view_mut().unwrap().outline = current_outline;
        forget(app, &ctx);
        let middle = pane_middle(app, &ctx, 0, vec![], &mut |a| pane_out(a, &ctx, other));
        let request = ThumbRequest::for_cell(600.0, ctx.pixels_per_point());
        let key = PaneKey { slot: (app.session.documents()[0].uid, other.1) };
        (middle, pane_source(app, &ctx, other).revision(&key, &request))
    };
    let (preview, preview_rev) = shown(&mut app, false, true);
    let (outline, outline_rev) = shown(&mut app, true, false);
    assert!(red(preview), "the pane's Preview shows the fill: {preview:?}");
    assert!(!red(outline), "the pane's Outline shows no fill: {outline:?}");
    assert_ne!(preview_rev, outline_rev, "the pane's picture follows its own Outline");
    let (_, current_rev) = shown(&mut app, true, true);
    assert_eq!(current_rev, outline_rev, "the current view's Outline does not reach the pane");
}

/// With a display profile, a floating window's pane blends its Preview with the paper before the conversion, as the
/// canvas does, and in its own Outline, while the current view stays in Preview, converts the art alone.
#[test]
fn a_pane_blends_translucent_art_before_the_display_profile() {
    use crate::display_profile::{Found, Reading};
    let mut app = translucent_in_two_windows();
    let p3 = crate::display_profile::tests::p3();
    app.display_profile.report(Reading { display: Some("DP-1".into()), found: Found::Profile { icc: p3, from: "test".into() } });
    app.session.prefs.monitor_profile_display = true;
    let ctx = crate::testing::ui_context();
    let other = (0, 1);
    pane_out(&mut app, &ctx, other).textures_delta.clear();
    assert!(pane_source(&mut app, &ctx, other).frame.cache.over().is_some(), "the pane's Preview blends with its paper");
    crate::doc_windows::view_mut(&mut app, other).unwrap().outline = true;
    app.view_mut().unwrap().outline = false;
    let cache = pane_source(&mut app, &ctx, other).frame.cache;
    assert!(cache.outline && cache.over().is_none(), "the pane's Outline has no paper to blend with");
}

/// Each view of a document has a pane of its own: an edit made in one view shows in the other's pane, at that view's
/// zoom.
#[test]
fn an_edit_in_one_view_shows_in_the_other() {
    let mut app = app_with(1);
    let ctx = crate::testing::ui_context();
    app.run("window.newWindow", json!({})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.canvas.panes.last.iter().map(|(w, _)| *w).collect::<Vec<_>>(), [(0, 1), (0, 2)], "one pane per view");
    crate::doc_windows::view_mut(&mut app, (0, 1)).unwrap().zoom = 0.5;
    app.view_mut().unwrap().zoom = 2.0;
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let revision = app.session.documents()[0].revision;
    let key = pane_source(&mut app, &ctx, (0, 1)).frame.cache;
    assert_eq!((key.revision, key.zoom), (revision, 0.5), "the other view's pane shows the edit at its own zoom");
    assert_eq!(app.view().unwrap().zoom, 2.0, "the view edited in keeps its zoom");
}

/// A pane draws its own view's Outline or Preview, not the current view's.
#[test]
fn each_pane_draws_its_own_views_outline_or_preview() {
    let mut app = app_with(1);
    let ctx = crate::testing::ui_context();
    app.run("window.newWindow", json!({})).unwrap();
    crate::doc_windows::show_window(&mut app, (0, 1));
    app.run("view.outline", json!({})).unwrap();
    crate::doc_windows::show_window(&mut app, (0, 2));
    assert!(!app.outline_view(), "the second view is in Preview");
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(pane_source(&mut app, &ctx, (0, 1)).frame.cache.outline, "the first view's pane is in Outline");
}
