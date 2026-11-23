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

/// Tile's grid of `n` panes in `area`.
fn cells(n: usize, area: egui::Rect) -> Vec<egui::Rect> {
    layout_cells(Layout::Grid, n, area)
}

/// The documents' indices in the panes the last frame drew, and the panes.
fn panes(app: &VectorcraftApp) -> (Vec<usize>, Vec<egui::Rect>) {
    app.canvas.panes.last.iter().map(|(w, r)| (w.0, *r)).unzip()
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

/// The distinct left and top edges of `c`: how many columns and rows its panes take.
fn columns_and_rows(c: &[egui::Rect]) -> (usize, usize) {
    let lefts: BTreeSet<u32> = c.iter().map(|r| r.left().to_bits()).collect();
    let tops: BTreeSet<u32> = c.iter().map(|r| r.top().to_bits()).collect();
    (lefts.len(), tops.len())
}

#[test]
fn each_tile_layout_places_its_panes_edge_to_edge() {
    let area = egui::Rect::from_min_size(pos2(10.0, 35.0), vec2(1001.0, 703.0));
    let cases = [
        (Layout::Vertical, 3, (3, 1)),
        (Layout::Vertical, 7, (7, 1)),
        (Layout::Horizontal, 3, (1, 3)),
        (Layout::Up(2), 2, (2, 1)),
        (Layout::Up(3), 3, (3, 1)),
        (Layout::Up(3), 2, (2, 1)),
        (Layout::Up(4), 4, (2, 2)),
        (Layout::Up(4), 3, (2, 2)),
        (Layout::Up(6), 6, (3, 2)),
        (Layout::Up(6), 4, (3, 2)),
        (Layout::Up(6), 2, (2, 1)),
    ];
    for (layout, n, shape) in cases {
        let c = layout_cells(layout, n, area);
        assert_eq!(c.len(), n);
        assert_eq!(columns_and_rows(&c), shape, "{layout:?} with {n} panes: columns and rows");
        assert_eq!(c[0].min, area.min, "{layout:?}: the first pane starts at the corner");
        let right = c.iter().map(|r| r.right()).fold(f32::MIN, f32::max);
        let bottom = c.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
        assert_eq!((right, bottom), (area.right(), area.bottom()), "{layout:?} with {n} panes reaches the far edges");
        for (i, r) in c.iter().enumerate() {
            assert!(r.width() > 0.0 && r.height() > 0.0 && area.contains_rect(*r), "{layout:?}: {r:?}");
            for o in &c[i + 1..] {
                assert!(r.intersect(*o).area() <= 0.0, "{layout:?} panes overlap: {r:?} {o:?}");
            }
        }
    }
    assert!(layout_cells(Layout::Up(2), 0, area).is_empty());
}

#[test]
fn tile_layouts_line_the_documents_up_side_by_side_or_stacked() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    let reply = app.run("window.arrange.tile", json!({"layout": "vertical"})).unwrap();
    assert_eq!(reply, json!({"tiled": true, "panes": 3, "layout": "vertical"}));
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 1, 2]);
    assert!(rects.windows(2).all(|p| p[0].top() == p[1].top() && p[0].height() == p[1].height() && p[0].right() == p[1].left()), "{rects:?}");
    let reply = app.run("window.arrange.tile", json!({"layout": "horizontal"})).unwrap();
    assert_eq!(reply, json!({"tiled": true, "panes": 3, "layout": "horizontal"}));
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 1, 2]);
    assert!(rects.windows(2).all(|p| p[0].left() == p[1].left() && p[0].width() == p[1].width() && p[0].bottom() == p[1].top()), "{rects:?}");
    // Each document fits its new pane.
    let artboard = Rect::new(0.0, 0.0, 400.0, 300.0);
    let canvas = app.canvas_rect.unwrap();
    for (i, r) in docs.iter().zip(&rects) {
        let shown = if *i == 2 { canvas } else { view_rects(&app, *r, ctx.pixels_per_point()).1 };
        let v = app.views[*i];
        assert!(v.fitted && (v.zoom - fit_zoom(shown, artboard, 0.0, 1.0)).abs() < 1e-9, "document {i} fits its pane: {v:?}");
    }
    assert_eq!(crate::control::inspect(&app, &ctx)["tileLayout"], json!("horizontal"));
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap()["layout"], json!("grid"), "Tile without a layout is the grid");
}

#[test]
fn n_up_shows_that_many_documents_and_keeps_the_rest_under_their_tabs() {
    let mut app = app_with(5);
    let ctx = egui::Context::default();
    assert_eq!(app.run("window.arrange.tile", json!({"layout": "2up"})).unwrap(), json!({"tiled": true, "panes": 2, "layout": "2up"}));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 4], "the current document takes the last pane");
    app.run("document.activate", json!({"index": 1})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 1], "the first documents in tab order");
    let second = (app.session.documents()[1].uid, 1);
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(app.canvas.panes.shown.contains_key(&second), "the second document is rendered for its pane");
    app.run("document.activate", json!({"index": 3})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 3], "a document chosen under its tab comes into the last pane");
    assert!(!app.canvas.panes.shown.contains_key(&second), "a document under its tab keeps no render");
    let artboard = Rect::new(0.0, 0.0, 400.0, 300.0);
    let shown = view_rects(&app, rects[0], ctx.pixels_per_point()).1;
    assert!((app.views[0].zoom - fit_zoom(shown, artboard, 0.0, 1.0)).abs() < 1e-9);
    assert!(app.canvas_rect.is_some_and(|c| rects[1].contains_rect(c)), "the current document's pane is the canvas");
    // A document brought into the last pane leaves the first pane's zoom as it is.
    app.views[0].zoom *= 2.0;
    let zoomed = app.views[0].zoom;
    app.run("document.activate", json!({"index": 2})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 2]);
    assert_eq!(app.views[0].zoom, zoomed, "the document that stayed in its pane keeps its zoom");
    let last = app.canvas_rect.unwrap();
    assert!((app.views[2].zoom - fit_zoom(last, artboard, 0.0, 1.0)).abs() < 1e-9, "the document brought in fits its pane");
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
    let ctx = egui::Context::default();
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
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 2], "the former second document is first now, the current one stays last");
    assert_eq!(app.session.documents()[0].uid, moving);
    let area = view_rects(&app, rects[0], ctx.pixels_per_point()).1;
    let v = app.views[0];
    assert!(v.fitted && (v.zoom - fit_zoom(area, Rect::new(0.0, 0.0, 400.0, 300.0), 0.0, 1.0)).abs() < 1e-9, "it fits its new pane: {v:?}");
}

/// 6-Up keeps its three columns and two rows with five documents: closing the sixth leaves the
/// others in their panes with the zoom they had. Side by side, closing one widens every pane, and
/// each document fits its new pane.
#[test]
fn closing_a_document_keeps_the_zoom_of_the_documents_whose_panes_stay() {
    let mut app = app_with(6);
    let ctx = egui::Context::default();
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
    let (docs, rects) = panes(&app);
    assert_eq!(docs, [0, 1]);
    let area = view_rects(&app, rects[1], ctx.pixels_per_point()).1;
    let v = app.views[1];
    assert!(v.fitted && (v.zoom - fit_zoom(area, Rect::new(0.0, 0.0, 400.0, 300.0), 0.0, 1.0)).abs() < 1e-9, "it fits its wider pane: {v:?}");
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
            crate::menus::Item::Sub("Arrange", items) => Some(items),
            _ => None,
        })
        .unwrap();
    let labels: Vec<&str> = arrange
        .iter()
        .filter_map(|i| match i {
            crate::menus::Item::Cmd(l, ..) => Some(*l),
            _ => None,
        })
        .collect();
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
    for item in &arrange {
        if let crate::menus::Item::Cmd(_, "window.arrange.tile", p) = item {
            let p = if p.is_null() { json!({}) } else { p.clone() };
            assert!(app.run("window.arrange.tile", p.clone()).is_ok(), "{p}");
        }
    }
}

/// One frame of the application bar with screen-reader nodes on → the nodes.
fn app_bar_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::accesskit::Node> {
    app_bar_frame_in(app, ctx, events, 1400.0)
}

/// [`app_bar_frame`] on a window `width` points wide.
fn app_bar_frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, width: f32) -> Vec<egui::accesskit::Node> {
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(width, 800.0));
    let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| crate::chrome::app_bar(app, ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
}

/// The center of the application bar's node whose label, without its leading spaces, is `name`
/// (or starts with it and a shortcut in parentheses).
fn app_bar_node(nodes: &[egui::accesskit::Node], name: &str) -> Option<Pos2> {
    let named = |l: &str| l.trim_start().strip_prefix(name).is_some_and(|rest| rest.is_empty() || rest.trim_start().starts_with('('));
    let b = nodes.iter().find(|n| n.label().is_some_and(named))?.bounds()?;
    Some(pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// Click the application bar's node labeled `name`.
fn click_app_bar(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
    let at = app_bar_node(&app_bar_frame(app, ctx, vec![]), name).unwrap_or_else(|| panic!("{name} is drawn"));
    app_bar_frame(app, ctx, vec![egui::Event::PointerMoved(at), button(at, true)]);
    app_bar_frame(app, ctx, vec![button(at, false)]);
}

#[test]
fn the_arrange_documents_button_tiles_and_consolidates() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    app_bar_frame(&mut app, &ctx, vec![]);
    let nodes = app_bar_frame(&mut app, &ctx, vec![]);
    let (arrange, search) = (app_bar_node(&nodes, "Arrange Documents"), app_bar_node(&nodes, "Search commands"));
    let (arrange, search) = (arrange.expect("the application bar has Arrange Documents"), search.expect("the search shows"));
    assert!(arrange.x > search.x && (arrange.y - search.y).abs() < 1.0, "right of the search: {arrange:?}, {search:?}");
    click_app_bar(&mut app, &ctx, "Arrange Documents");
    let menu = app_bar_frame(&mut app, &ctx, vec![]);
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
        assert!(app_bar_node(&menu, item).is_some(), "the menu lists {item}");
    }
    click_app_bar(&mut app, &ctx, "Tile All Vertically");
    assert!(app.ui.tiled && app.ui.tile_layout == Layout::Vertical, "{}", app.ui.status);
    click_app_bar(&mut app, &ctx, "Arrange Documents");
    click_app_bar(&mut app, &ctx, "Consolidate All Windows");
    assert!(!app.ui.tiled);
}

/// As the application bar narrows, Arrange Documents goes before the search.
#[test]
fn a_narrow_application_bar_keeps_search_before_arrange_documents() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let mut search_alone = false;
    for width in (200..=1400).rev().step_by(4) {
        app_bar_frame_in(&mut app, &ctx, vec![], width as f32);
        let nodes = app_bar_frame_in(&mut app, &ctx, vec![], width as f32);
        let (arrange, search) = (app_bar_node(&nodes, "Arrange Documents").is_some(), app_bar_node(&nodes, "Search commands").is_some());
        assert!(search || !arrange, "{width} pt: Arrange Documents shows without the search");
        search_alone |= search && !arrange;
    }
    assert!(search_alone, "some width shows the search without Arrange Documents");
}

#[test]
fn tile_shows_a_pane_for_each_document_fitted_to_it() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert!(panes(&app).0.is_empty(), "documents open under their tabs");
    let tabbed = app.canvas_rect.unwrap();
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": true, "panes": 3, "layout": "grid"}));
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
        assert!(v.fitted && (v.zoom - fit_zoom(shown, artboard, 0.0, 1.0)).abs() < 1e-9, "document {i} fits its pane: {v:?}");
        assert_eq!(v.center, artboard.center());
    }
    assert_eq!(app.canvas.panes.shown.len(), 2, "the other two documents are rendered for their panes");
    let inspected = crate::control::inspect(&app, &ctx);
    assert_eq!(inspected["tiled"], json!(true));
    let active: Vec<bool> = inspected["panes"].as_array().unwrap().iter().map(|p| p["active"].as_bool().unwrap()).collect();
    assert_eq!(active, [false, false, true]);
}

/// With the interface zoomed by Preferences › User Interface › UI Scaling, each tiled document
/// fits its pane with 30 interface points to spare: its zoom is the zoom that fits it in an
/// unscaled interface times the UI Scaling.
#[test]
fn tiled_documents_fit_their_panes_at_every_ui_scaling() {
    for ui_zoom in [0.75_f32, 1.5, 2.0] {
        let mut app = app_with(3);
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(ui_zoom);
        let size = vec2(1200.0, 800.0) / ui_zoom;
        frame_in(&mut app, &ctx, vec![], size);
        app.run("window.arrange.tile", json!({})).unwrap();
        frame_in(&mut app, &ctx, vec![], size);
        let (docs, rects) = panes(&app);
        for (i, r) in docs.iter().zip(&rects).filter(|(i, _)| **i != 2) {
            let shown = view_rects(&app, *r, ctx.pixels_per_point()).1;
            let unscaled = ((f64::from(shown.width()) - 60.0) / 400.0).min((f64::from(shown.height()) - 60.0) / 300.0);
            let v = app.views[*i];
            assert!(v.fitted && (v.zoom - unscaled * f64::from(ui_zoom)).abs() < 1e-9, "UI Scaling {ui_zoom}×: document {i} fits its pane: {v:?}");
        }
    }
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

/// One headless frame at `time` seconds as the app runs it: a Selection move released off the
/// canvas ends first ([`crate::canvas::release_over_panel`]), then the tabs and the panes draw.
fn timed_frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: Option<f64>, events: Vec<egui::Event>) -> egui::FullOutput {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        max_texture_side: Some(8192),
        time,
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        crate::canvas::release_over_panel(app, ui.ctx());
        app.canvas_area(ui);
    });
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
    let ctx = egui::Context::default();
    app_frame(&mut app, &ctx, vec![]);
    app_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

/// Press on document point `at` on the canvas and move the pointer 20 × 10 points, still on the
/// canvas: the Selection tool moves the art under it.
fn grab(app: &mut VectorcraftApp, ctx: &egui::Context, at: Point) {
    let p = Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), 1.0).to_screen(at);
    app_frame(app, ctx, vec![egui::Event::PointerMoved(p), button(p, true)]);
    app_frame(app, ctx, vec![egui::Event::PointerMoved(p + vec2(20.0, 10.0))]);
    assert!(app.session.in_interaction(), "the Selection tool moves the art");
}

/// Window `w`'s pane: the screen point that shows document point `at` in it, and the pane.
fn in_pane(app: &VectorcraftApp, ctx: &egui::Context, w: Win, at: Point) -> (Pos2, egui::Rect) {
    let (_, cell) = app.canvas.panes.last.iter().copied().find(|(o, _)| *o == w).unwrap();
    let area = view_rects(app, cell, ctx.pixels_per_point()).1;
    (Xf::at(area, crate::doc_windows::view(app, w).unwrap(), 1.0).to_screen(at), cell)
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
    let p = Xf::at(app.canvas_rect.unwrap(), app.view().unwrap(), 1.0).to_screen(Point::new(100.0, 100.0));
    timed_frame(&mut app, &ctx, Some(0.05), vec![egui::Event::PointerMoved(p), button(p, true)]);
    timed_frame(&mut app, &ctx, Some(0.1), vec![egui::Event::PointerMoved(p + vec2(20.0, 10.0))]);
    timed_frame(&mut app, &ctx, Some(0.2), vec![egui::Event::PointerMoved(tabs[2].center())]);
    timed_frame(&mut app, &ctx, Some(0.25), vec![]);
    timed_frame(&mut app, &ctx, Some(0.8), vec![]);
    assert_eq!(app.session.active_index(), Some(2), "holding the art over the third tab makes that document current");
    timed_frame(&mut app, &ctx, Some(0.85), vec![]);
    let (drop, _) = in_pane(&app, &ctx, (1, 1), Point::new(150.0, 100.0));
    timed_frame(&mut app, &ctx, Some(0.9), vec![egui::Event::PointerMoved(drop)]);
    timed_frame(&mut app, &ctx, Some(1.0), vec![button(drop, false)]);
    timed_frame(&mut app, &ctx, Some(1.1), vec![]);
    assert_eq!(app.session.active_index(), Some(1), "the pane's document is the current one");
    assert_eq!(objects(&app, 1), 2, "the copy joins the second document's square");
    let placed = selected_bounds(&app);
    assert!((placed.center().x - 150.0).abs() < 0.01 && (placed.center().y - 100.0).abs() < 0.01, "{placed:?}");
    assert_eq!(app.session.documents()[0].doc, source);
    assert_eq!(app.session.documents()[2].doc, third, "the document the tab showed is untouched");
}

#[test]
fn a_drag_released_off_a_panes_art_area_or_after_escape_changes_nothing() {
    for (case, escape) in [("scroll bar", false), ("art area after Escape", true)] {
        let (mut app, ctx) = tiled_pair();
        let docs: Vec<_> = app.session.documents().iter().map(|d| (d.doc.clone(), d.history.undo.len())).collect();
        grab(&mut app, &ctx, Point::new(100.0, 100.0));
        let (inside, cell) = in_pane(&app, &ctx, (1, 1), Point::new(200.0, 150.0));
        let at = if escape { inside } else { pos2(cell.right() - 2.0, cell.center().y) };
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
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let tabbed = app.canvas_rect.unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0.len(), 2);
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({})).unwrap(), json!({"tiled": false, "floating": []}));
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
fn tile_with_one_document_leaves_the_next_one_under_its_tab() {
    let mut app = app_with(1);
    let ctx = egui::Context::default();
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
    let ctx = egui::Context::default();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(panes(&app).0, [0, 1, 2]);
    let first = (app.session.documents()[0].uid, 1);
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
    let uid = (app.session.documents()[1].uid, 1);
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

/// The selected objects' outlines as the panes made them on every frame before they kept them,
/// to hold the kept outlines against.
fn outlines_made_each_frame(st: &vectorcraft_engine::DocState, xf: &Xf) -> Vec<Shape> {
    let mut shapes = vec![];
    let symbols = st.doc.symbol_boxes();
    let nodes: Vec<(vectorcraft_doc::NodeId, &vectorcraft_doc::Node)> =
        st.selection.objects.iter().filter_map(|id| Some((*id, st.doc.node(*id)?))).collect();
    let color = |id: vectorcraft_doc::NodeId| Stroke::new(1.0, c32(st.doc.layer_color(id)));
    let mut stroke_path = |bp: &BezPath, s: Stroke| {
        for line in to_screen_path(bp, xf) {
            shapes.push(Shape::line(line, s));
        }
    };
    if too_many_marks(nodes.iter().map(|(_, n)| *n)) {
        let bounds = nodes.iter().filter_map(|(_, n)| n.geometric_bounds_with(&symbols)).reduce(|a, b| a.union(b));
        if let (Some(b), Some((id, _))) = (bounds, nodes.first()) {
            stroke_path(&vectorcraft_geom::shapes::rectangle(b).to_bezpath(), color(*id));
        }
        return shapes;
    }
    for (id, n) in nodes {
        stroke_path(&node_outline(n, &symbols), color(id));
        stroke_path(&linked_crosses(n), color(id));
    }
    shapes
}

/// The first document's pane in a tiled pair: the outlines it keeps, after checking that they
/// equal the outlines made afresh through its view and that the frame `drawn` holds them in order.
fn kept_outlines<'a>(app: &'a VectorcraftApp, ctx: &egui::Context, drawn: &[Shape]) -> &'a [Shape] {
    let pane = panes(app).1[0];
    let xf = Xf::new(view_rects(app, pane, ctx.pixels_per_point()).1, &app.views[0]);
    let slot = (app.session.documents()[0].uid, 1);
    let kept = &app.canvas.panes.outlines.get(&slot).expect("the pane keeps its outlines").shapes;
    assert_eq!(kept, &outlines_made_each_frame(&app.session.documents()[0], &xf), "the kept outlines are the ones made afresh");
    assert!(kept.is_empty() || drawn.windows(kept.len()).any(|w| w == kept.as_slice()), "the pane draws the kept outlines");
    kept
}

/// Run `cmd` on the first document of a tiled pair, then make the second current again.
fn in_first(app: &mut VectorcraftApp, cmd: &str, args: serde_json::Value) -> serde_json::Value {
    app.run("document.activate", json!({"index": 0})).unwrap();
    let out = app.session.execute(cmd, &args).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    out
}

#[test]
fn kept_outlines_do_not_show_for_a_new_document_that_takes_a_freed_documents_address() {
    let affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let st = vectorcraft_engine::DocState::new(vectorcraft_doc::Document::new(400.0, 300.0), None);
    let kept = Outlines { revision: st.revision, doc: Arc::downgrade(&st.doc), selection: vec![], affine, shapes: vec![] };
    assert!(kept.shows(&st, &affine), "the outlines show the document they were made for");
    let freed = Arc::as_ptr(&st.doc);
    drop(st);
    let mut reused = false;
    for _ in 0..64 {
        let other = vectorcraft_engine::DocState::new(vectorcraft_doc::Document::new(400.0, 300.0), None);
        reused |= Arc::as_ptr(&other.doc) == freed;
        assert!(!kept.shows(&other, &affine), "a different document with the same revision, selection and mapping is not the one shown");
    }
    assert!(!reused, "the weak reference keeps the freed document's address from being taken");
}

/// Another pane crosses a selected linked image from corner to corner, as the current pane does,
/// and outlines a selected embedded image alone (#9080).
#[test]
fn other_panes_cross_a_selected_linked_image() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    let dir = vectorcraft_testkit::temp_dir("panes-linked-image");
    let png = dir.join("red.png");
    let image = Image::from_raw(40, 40, ChannelLayout::Rgba, SampleType::U8, [200u8, 40, 40, 255].repeat(40 * 40)).unwrap();
    std::fs::write(&png, craft_image_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()).unwrap();
    let mut app = app_with(2);
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    let ctx = egui::Context::default();
    let linked = in_first(&mut app, "file.place", json!({"path": png, "link": true, "at": [100, 100]}))["ids"][0].clone();
    let embedded = in_first(&mut app, "file.place", json!({"path": png, "link": false, "at": [250, 100]}))["ids"][0].clone();
    let lines = |app: &mut VectorcraftApp, id: &serde_json::Value| {
        in_first(app, "select.set", json!({"ids": [id]}));
        frame(app, &ctx, vec![]);
        let drawn = frame(app, &ctx, vec![]);
        kept_outlines(app, &ctx, &drawn).len()
    };
    let (linked, embedded) = (lines(&mut app, &linked), lines(&mut app, &embedded));
    assert_eq!(linked, embedded + 2, "the outline of a linked image and its two crossing lines; an embedded image's outline alone");
}

/// Another pane outlines a selected group's visible members only: hiding a member draws the same
/// outlines as deleting it.
#[test]
fn other_panes_leave_out_a_selected_groups_hidden_members() {
    let mut app = app_with(2);
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    let ctx = egui::Context::default();
    let shown = in_first(&mut app, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 80}))["id"].clone();
    let hidden = in_first(&mut app, "shape.ellipse", json!({"x": 200, "y": 100, "width": 90, "height": 60}))["id"].clone();
    in_first(&mut app, "select.set", json!({"ids": [shown, hidden]}));
    let group = in_first(&mut app, "object.group", json!({}))["id"].clone();
    let outlines = |app: &mut VectorcraftApp| {
        in_first(app, "select.set", json!({"ids": [group]}));
        frame(app, &ctx, vec![]);
        let drawn = frame(app, &ctx, vec![]);
        kept_outlines(app, &ctx, &drawn).to_vec()
    };
    let both = outlines(&mut app);
    in_first(&mut app, "select.set", json!({"ids": [hidden]}));
    in_first(&mut app, "edit.clear", json!({}));
    let without = outlines(&mut app);
    in_first(&mut app, "edit.undo", json!({}));
    in_first(&mut app, "select.set", json!({"ids": [hidden]}));
    in_first(&mut app, "object.hide", json!({}));
    let with_hidden = outlines(&mut app);
    assert_ne!(both, without, "the outlines show both members");
    assert_eq!(with_hidden, without, "a hidden member is not outlined");
}

#[test]
fn other_panes_keep_their_selected_outlines_until_the_document_selection_or_view_changes() {
    let mut app = app_with(2);
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    let ctx = egui::Context::default();
    let rect = in_first(&mut app, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 80}))["id"].clone();
    let ellipse = in_first(&mut app, "shape.ellipse", json!({"x": 200, "y": 100, "width": 90, "height": 60}))["id"].clone();
    in_first(&mut app, "select.set", json!({"ids": [rect, ellipse]}));
    frame(&mut app, &ctx, vec![]);
    let drawn = frame(&mut app, &ctx, vec![]);
    assert!(kept_outlines(&app, &ctx, &drawn).len() >= 2, "the rectangle and the ellipse are outlined");
    let made = app.canvas.panes.outlines_made;
    let drawn = frame(&mut app, &ctx, vec![]);
    kept_outlines(&app, &ctx, &drawn);
    assert_eq!(app.canvas.panes.outlines_made, made, "an unchanged frame draws the kept outlines");
    let changed = |app: &mut VectorcraftApp, what: &str| {
        let made = app.canvas.panes.outlines_made;
        let drawn = frame(app, &ctx, vec![]);
        kept_outlines(app, &ctx, &drawn);
        assert_eq!(app.canvas.panes.outlines_made, made + 1, "{what} makes the outlines again, once");
        let drawn = frame(app, &ctx, vec![]);
        kept_outlines(app, &ctx, &drawn);
        assert_eq!(app.canvas.panes.outlines_made, made + 1, "the frame after {what} draws the kept outlines");
    };
    in_first(&mut app, "select.set", json!({"ids": [ellipse]}));
    changed(&mut app, "a new selection");
    in_first(&mut app, "object.move", json!({"dx": 30, "dy": -10}));
    changed(&mut app, "a move");
    let layer = app.session.documents()[0].doc.layers[0].id.0;
    in_first(&mut app, "layer.setProps", json!({"id": layer, "color": "#20a040"}));
    changed(&mut app, "a new layer color");
    app.views[0].center.x += 25.0;
    changed(&mut app, "a pan");
    app.views[0].zoom *= 1.5;
    app.views[0].rotation = 20.0;
    changed(&mut app, "a zoom and a turn");
    app.ui.view.edges = false;
    frame(&mut app, &ctx, vec![]);
    assert!(app.canvas.panes.outlines.is_empty(), "with Edges off no outlines are kept");
    app.ui.view.edges = true;
    changed(&mut app, "Edges on again");
    in_first(&mut app, "select.none", json!({}));
    changed(&mut app, "an empty selection");
    let drawn = frame(&mut app, &ctx, vec![]);
    assert!(kept_outlines(&app, &ctx, &drawn).is_empty());
    // 30,000 rectangles have more points than the outlines are made for: the pane keeps the bounds.
    let st = &app.session.documents()[0];
    let rect = st.doc.node(vectorcraft_doc::NodeId(rect.as_u64().unwrap())).cloned().unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.session
        .edit("Fill", |d, _| {
            let layer = d.layers[0].id;
            for _ in 0..30_000 {
                let mut n = rect.clone();
                n.id = d.alloc_id();
                d.insert(Some(layer), usize::MAX, n)?;
            }
            Ok(())
        })
        .unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    changed(&mut app, "selecting more points than are outlined");
    let drawn = frame(&mut app, &ctx, vec![]);
    assert_eq!(kept_outlines(&app, &ctx, &drawn).len(), 1, "one rectangle around the bounds");
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(app.canvas.panes.outlines.is_empty(), "Consolidate All Windows forgets the outlines");
    // Panes that stop showing before their render arrives keep no outlines either.
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.canvas.panes.outlines.len(), 1);
    app.ui.tiled = false;
    app.canvas.panes.shown.clear();
    app.canvas.panes.last.clear();
    frame(&mut app, &ctx, vec![]);
    assert!(app.canvas.panes.outlines.is_empty(), "no pane shows, so no outlines are kept");
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
    assert!((v.zoom - fit_zoom(rect, Rect::new(0.0, 0.0, 400.0, 300.0), 0.0, 1.0)).abs() < 1e-9, "fitted to its new pane: {v:?}");
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
    let uid = (app.session.documents()[0].uid, 1);
    let key = app.canvas.panes.shown.get(&uid).map(|(k, _)| k.clone()).unwrap();
    let [x0, y0, x1, y1] = key.pixel.expect("the pane renders document pixels");
    assert!(x1 > x0 && y1 > y0);
}

/// A pane more device pixels across than the largest texture side the display supports renders
/// its document within that size, as the canvas does, and the frame covers the pane.
#[test]
fn other_panes_render_within_the_largest_texture_side() {
    let mut app = app_with(2);
    app.canvas.panes.worker_started = true; // every pane renders on this thread
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(2.0);
    app.run("window.arrange.tile", json!({})).unwrap();
    // The first frame sets the pixels a point, the second draws at them.
    for _ in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
            max_texture_side: Some(1024),
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| app.canvas_area(ui)).textures_delta.clear();
    }
    let uid = (app.session.documents()[0].uid, 1);
    let (key, tex) = app.canvas.panes.shown.get(&uid).map(|(k, t)| (k.clone(), t.size())).expect("the other document's pane rendered");
    assert_eq!(tex[0].max(tex[1]), 1024, "the pane's frame is {tex:?} pixels");
    let (docs, cells) = panes(&app);
    let pane = docs.iter().zip(cells).find(|(i, _)| **i == 0).map(|(_, r)| r).unwrap();
    let rect = view_rects(&app, pane, ctx.pixels_per_point()).1;
    assert!(rect.width().max(rect.height()) * ctx.pixels_per_point() > 1024.0, "the pane is {rect:?} at {} pixels a point", ctx.pixels_per_point());
    assert!(
        (key.w as f32 / key.ppp - rect.width()).abs() < 1.0 && (key.h as f32 / key.ppp - rect.height()).abs() < 1.0,
        "the frame covers the pane {rect:?}: {} × {} pixels at {} a point",
        key.w,
        key.h,
        key.ppp
    );
}

/// A pane's render key takes its document's [`CacheKey::placed`] value ([`placed_key`]), which a
/// bitmap made for a test running beside this one does not move: the document holds no images.
#[test]
fn a_panes_key_takes_its_documents_placed_value() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    app.run("window.arrange.tile", json!({})).unwrap();
    let uid = app.session.documents()[0].uid;
    let generation = vectorcraft_render::placed_document::generation();
    let marked = generation.wrapping_add(1000);
    app.canvas.placed_keys.insert(uid, crate::PlacedKey { checked: generation, key: marked, before: (marked, generation) });
    frame(&mut app, &ctx, vec![]);
    let key = app.canvas.panes.shown.get(&(uid, 1)).map(|(k, _)| k.placed);
    assert_eq!(key, Some(marked));
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
        assert!((app.views[i].zoom - fit_zoom(area, artboard, 0.0, 1.0)).abs() < 1e-9, "document {i} fits its smaller pane: {:?}", app.views[i]);
    }
    assert_eq!(app.views[1].zoom, zoomed.zoom, "the zoomed document keeps its zoom");
}

/// The RGB of the middle pixel of the render that document `i`'s pane uploaded in `out`.
fn pane_middle(app: &VectorcraftApp, out: &egui::FullOutput, i: usize) -> [u8; 3] {
    let uid = app.session.documents()[i].uid;
    let id = app.canvas.panes.shown.iter().find(|(slot, _)| slot.0 == uid).map(|(_, (_, tex))| tex.id()).unwrap();
    let image = out.textures_delta.set.iter().find(|(t, _)| **t == id).and_then(|(_, d)| d.last()).map(|d| d.image.clone()).unwrap();
    let egui::ImageData::Color(img) = image;
    let p = img.pixels[img.size[1] / 2 * img.size[0] + img.size[0] / 2];
    [p.r(), p.g(), p.b()]
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
    // Whether `out` fills a shape with the RGB `rgb`, give or take one.
    let fills = |out: &egui::FullOutput, rgb: [u8; 3]| {
        fn any(s: &Shape, f: &dyn Fn(Color32) -> bool) -> bool {
            match s {
                Shape::Vec(v) => v.iter().any(|s| any(s, f)),
                Shape::Path(p) => f(p.fill),
                _ => false,
            }
        }
        out.shapes.iter().any(|c| any(&c.shape, &|c: Color32| c.a() == 255 && near([c.r(), c.g(), c.b()], rgb)))
    };
    let ctx = egui::Context::default();
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        max_texture_side: Some(8192),
        ..Default::default()
    };
    app.run("window.arrange.tile", json!({})).unwrap();
    // The first document in front: the second one's pane draws through Display P3, its paper too.
    app.run("document.activate", json!({"index": 0})).unwrap();
    let mut out = ctx.run_ui(raw(), |ui| app.canvas_area(ui));
    let shown = pane_middle(&app, &out, 1);
    assert!(fills(&out, paper_p3), "the back document's paper draws through its Display P3: {paper_p3:?}");
    assert!(!fills(&out, [0xcc, 0x66, 0x33]), "the back document's paper does not draw through the working space");
    out.textures_delta.clear();
    assert!(near(shown, p3), "the back document draws through its Display P3: {shown:?}, not {p3:?}");
    // The same in a floating window.
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    let floating = |app: &mut VectorcraftApp| {
        ctx.run_ui(raw(), |ui| {
            app.canvas_area(ui);
            crate::doc_windows::show(app, ui.ctx());
        })
    };
    // The window takes its place in the first frame and fades in over the next few.
    for _ in 0..30 {
        floating(&mut app).textures_delta.clear();
    }
    app.canvas.panes.clear();
    let mut out = floating(&mut app);
    let shown = pane_middle(&app, &out, 1);
    assert!(fills(&out, paper_p3), "the floating document's paper draws through its Display P3: {paper_p3:?}");
    out.textures_delta.clear();
    assert!(near(shown, p3), "the floating document draws through its Display P3: {shown:?}, not {p3:?}");
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    app.canvas.panes.clear();
    // The second document in front: the first one's pane draws through the working sRGB.
    app.run("document.activate", json!({"index": 1})).unwrap();
    let mut out = ctx.run_ui(raw(), |ui| app.canvas_area(ui));
    let shown = pane_middle(&app, &out, 0);
    out.textures_delta.clear();
    assert!(near(shown, srgb), "the back document draws through the working space: {shown:?}, not {srgb:?}");
    // The same off the UI thread.
    app.canvas.panes.clear();
    app.canvas.panes.worker = crate::thumb_worker::ThumbWorker::start(&ctx, "test-panes");
    app.canvas.panes.worker_started = true;
    ctx.run_ui(raw(), |ui| app.canvas_area(ui)).textures_delta.clear();
    let done = app.canvas.panes.worker.as_ref().unwrap().wait().unwrap();
    assert_eq!(done.key.0.0, app.session.documents()[0].uid);
    let img = done.img.unwrap();
    let at = ((img.height / 2 * img.width + img.width / 2) * 4) as usize;
    let shown = [img.pixels[at], img.pixels[at + 1], img.pixels[at + 2]];
    assert!(near(shown, srgb), "the worker draws the back document through the working space: {shown:?}, not {srgb:?}");
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
    app.canvas.panes.clear();
    ctx.run_ui(raw(), |ui| app.canvas_area(ui)).textures_delta.clear();
    let done = app.canvas.panes.worker.as_ref().unwrap().wait().unwrap();
    assert_eq!(done.key.0.0, app.session.documents()[0].uid);
    let img = done.img.unwrap();
    let at = ((img.height / 2 * img.width + img.width / 2) * 4) as usize;
    let shown = [img.pixels[at], img.pixels[at + 1], img.pixels[at + 2]];
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
    app.canvas.panes.worker_started = true; // every pane renders on this thread
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
    let uid = app.session.documents()[1].uid;
    let ctx = egui::Context::default();
    // The second document's pane: the raster one frame uploads, its width, and the key it was rendered for.
    let pane = |app: &mut VectorcraftApp| -> (Vec<Color32>, usize, CacheKey) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
            max_texture_side: Some(8192),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| app.canvas_area(ui));
        let (key, tex) = app.canvas.panes.shown.iter().find(|(slot, _)| slot.0 == uid).map(|(_, s)| s).expect("the second document's pane rendered");
        let id = tex.id();
        let up = out.textures_delta.set.iter().find(|e| e.0.eq(&id)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            (img.pixels.clone(), img.width())
        });
        let key = key.clone();
        out.textures_delta.clear();
        let (pixels, width) = up.expect("the pane uploads");
        (pixels, width, key)
    };
    app.session.prefs.monitor_profile_display = false;
    let (srgb, width, key) = pane(&mut app);
    assert!(key.backdrop.is_none(), "no display profile: nothing to blend");
    let p3 = crate::display_profile::tests::p3();
    let t = vectorcraft_color::cms::display::DisplayTransform::from_icc(&p3).unwrap();
    app.display_profile.report(Reading { display: Some("DP-1".into()), found: Found::Profile { icc: p3, from: "test".into() } });
    app.session.prefs.monitor_profile_display = true;
    let (shown, _, key) = pane(&mut app);
    let backdrop = key.backdrop.as_deref().expect("the pane's Preview blends with its paper");
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
        t.apply_premultiplied(&mut px);
        Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
    };
    // Pixel `s` over the paper `paper` in sRGB, converted.
    let blend = |s: Color32, paper: [u8; 3]| {
        let k = 255 - u32::from(s.a());
        t.apply8([0, 1, 2].map(|j| (u32::from(s.to_array()[j]) + (u32::from(paper[j]) * k + 127) / 255).min(255) as u8)).map(u32::from)
    };
    let differs = |a: [u32; 3], b: [u32; 3]| (0..3).any(|j| a[j].abs_diff(b[j]) > 2);
    let under = t.apply8(back_paper);
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

fn thread_cpu_ns() -> Option<u128> {
    std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
}

/// Time a pane's selected-object outlines with every object of a document selected: 2,000 and
/// 20,000 rectangles, 20,000 ellipses, and 150,000 rectangles (more points than are outlined, so
/// the pane draws their bounds). Each frame is a whole egui pass that draws the outlines and the
/// tessellation of what it drew, made afresh on every frame ([`outlines_made_each_frame`]) or
/// drawn from the kept outlines ([`selected_objects`]). The sides alternate, and the median time
/// per frame of 7 runs is printed.
/// Run with `cargo test --release -p vectorcraft-ui-egui --lib pane_outlines_timing -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; performance depends on machine load"]
fn pane_outlines_timing() {
    const FRAMES: u32 = 20;
    let clock = if thread_cpu_ns().is_some() { "thread CPU time" } else { "wall-clock time" };
    eprintln!("{clock} per frame, median of 7 runs of {FRAMES} frames");
    for (count, shape) in [(2_000, "shape.rectangle"), (20_000, "shape.rectangle"), (20_000, "shape.ellipse"), (150_000, "shape.rectangle")] {
        let mut app = app_with(1);
        let id = app.run(shape, json!({"x": 20, "y": 20, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap();
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().unwrap();
        app.session
            .edit("Fill", |d, _| {
                let layer = d.layers[0].id;
                for _ in 1..count {
                    let mut n = node.clone();
                    n.id = d.alloc_id();
                    d.insert(Some(layer), usize::MAX, n)?;
                }
                Ok(())
            })
            .unwrap();
        app.run("select.all", json!({})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.selection.objects.len(), count);
        let ctx = egui::Context::default();
        let area = egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0));
        let xf = Xf { rect: area, zoom: 4.0, center: Point::new(50.0, 40.0), rot: 0.0 };
        let mut kept = Panes::default();
        let slot = (st.uid, 1);
        let mut pass = |fresh: bool| {
            let raw = egui::RawInput { screen_rect: Some(area), ..Default::default() };
            let out = ctx.run_ui(raw, |ui| {
                let painter = ui.painter_at(area);
                if fresh {
                    for s in outlines_made_each_frame(st, &xf) {
                        painter.add(s);
                    }
                } else {
                    selected_objects(&mut kept, slot, st, &painter, &xf);
                }
            });
            ctx.tessellate(out.shapes, out.pixels_per_point).len()
        };
        assert_eq!(pass(true), pass(false), "both sides draw as much");
        let mut time = |fresh: bool| {
            let (cpu, wall) = (thread_cpu_ns(), std::time::Instant::now());
            for _ in 0..FRAMES {
                pass(fresh);
            }
            let ns = thread_cpu_ns().zip(cpu).map_or_else(|| wall.elapsed().as_nanos(), |(b, a)| b - a);
            ns as f64 / f64::from(FRAMES) / 1e6
        };
        let (mut fresh, mut cached) = (vec![], vec![]);
        for _ in 0..7 {
            fresh.push(time(true));
            cached.push(time(false));
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let shapes = kept.outlines.get(&slot).map_or(0, |o| o.shapes.len());
        eprintln!("{count} x {shape} ({shapes} lines drawn)");
        eprintln!("  made every frame: {:>9.3} ms  {fresh:.3?}", median(&mut fresh));
        eprintln!("  kept:             {:>9.3} ms  {cached:.3?}", median(&mut cached));
    }
}
