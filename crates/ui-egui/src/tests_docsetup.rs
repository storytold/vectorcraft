//! The canvas draws Document Setup: the red bleed outline and the transparency grid in the
//! document's colours and cell size, also in a rotated view; a white Background Contents hides
//! the grid and Simulate Colored Paper tints the artboard.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::theme::Tokens;
use crate::{VectorcraftApp, canvas};

/// One headless canvas frame: its shapes, the transparency grid tile uploaded in it (2 × 2
/// pixels: the grid colours) and the grid mesh, if any.
fn frame(app: &mut VectorcraftApp) -> (Vec<egui::Shape>, Option<Vec<egui::Color32>>, Option<egui::Mesh>) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    let tile = ctx.tex_manager().read().allocated().find(|(_, m)| m.name == canvas::TRANSPARENCY_GRID).map(|(id, _)| *id);
    let pixels = tile.and_then(|id| out.textures_delta.set.iter().find(|(t, _)| **t == id)).and_then(|(_, deltas)| match &deltas.first()?.image {
        egui::ImageData::Color(img) => Some(img.pixels.clone()),
    });
    out.textures_delta.clear();
    let shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
    let mut all = vec![];
    flatten(&shapes, &mut all);
    let mesh = all.into_iter().find_map(|s| match s {
        egui::Shape::Mesh(m) if Some(m.texture_id) == tile => Some((*m).clone()),
        _ => None,
    });
    (shapes, pixels, mesh)
}

fn flatten(shapes: &[egui::Shape], out: &mut Vec<egui::Shape>) {
    for s in shapes {
        match s {
            egui::Shape::Vec(v) => flatten(v, out),
            s => out.push(s.clone()),
        }
    }
}

/// The closed outlines drawn in `color`: their corner points.
fn outlines(shapes: &[egui::Shape], color: egui::Color32) -> Vec<Vec<egui::Pos2>> {
    let mut all = vec![];
    flatten(shapes, &mut all);
    all.into_iter()
        .filter_map(|s| match s {
            egui::Shape::Path(p) if p.closed && p.stroke.color == egui::epaint::ColorMode::Solid(color) => Some(p.points),
            _ => None,
        })
        .collect()
}

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app
}

#[test]
fn the_bleed_draws_as_a_red_outline_around_each_artboard() {
    let mut app = app();
    let red = Tokens::for_brightness(Default::default()).bleed;
    assert!(outlines(&frame(&mut app).0, red).is_empty(), "no bleed, no outline");
    app.run("document.setup", json!({"bleed": [10, 10, 20, 20]})).unwrap();
    let (shapes, _, _) = frame(&mut app);
    let found = outlines(&shapes, red);
    assert_eq!(found.len(), 1);
    // The artboard is 200 × 100 pt; the bleed grows it by 40 × 20 pt at the fitted zoom.
    let ab = outlines(&shapes, egui::Color32::from_gray(0)).into_iter().next().unwrap();
    let span = |q: &[egui::Pos2]| (q[2].x - q[0].x, q[2].y - q[0].y);
    let ((bw, bh), (aw, ah)) = (span(&found[0]), span(&ab));
    let zoom = aw / 200.0;
    assert!((bw - 240.0 * zoom).abs() < 0.5 && (bh - 120.0 * zoom).abs() < 0.5 && (ah - 100.0 * zoom).abs() < 0.5, "{found:?} {ab:?}");
    // A rotated view turns the outline with the artboard.
    app.view_mut().unwrap().rotation = 30.0;
    let q = &outlines(&frame(&mut app).0, red)[0];
    assert!((q[1].y - q[0].y).abs() > 10.0, "the top edge is no longer horizontal: {q:?}");
}

#[test]
fn the_grid_takes_the_document_colours_also_rotated() {
    let mut app = app();
    app.run("view.transparencyGrid", json!({"on": true})).unwrap();
    app.run("document.setup", json!({"gridColors": ["#ff0000", "#0000ff"], "gridSize": "large"})).unwrap();
    let (_, tile, _) = frame(&mut app);
    let (red, blue) = (egui::Color32::from_rgb(255, 0, 0), egui::Color32::from_rgb(0, 0, 255));
    assert_eq!(tile, Some(vec![red, blue, blue, red]));
    // Rotated: still one textured mesh over the artboard's turned corners.
    app.view_mut().unwrap().rotation = 45.0;
    let m = frame(&mut app).2.expect("the grid mesh");
    assert!((m.vertices[1].pos.y - m.vertices[0].pos.y).abs() > 10.0);
    // Large cells are 16 screen points: the 200 pt artboard spans 200 × zoom / 32 tile repeats.
    let zoom = app.view_mut().unwrap().zoom as f32;
    assert!((m.vertices[1].uv.x - 200.0 * zoom / 32.0).abs() < 1e-3, "{:?}", m.vertices[1].uv);
}

#[test]
fn white_background_hides_the_grid_and_paper_tints_the_artboard() {
    let mut app = app();
    app.run("view.transparencyGrid", json!({"on": true})).unwrap();
    app.run("document.setup", json!({"backgroundContents": "white"})).unwrap();
    assert!(frame(&mut app).2.is_none());
    app.run("view.transparencyGrid", json!({"on": false})).unwrap();
    app.run("document.setup", json!({"simulatePaper": true, "gridColors": ["#ffeecc", "#cccccc"]})).unwrap();
    let mut all = vec![];
    flatten(&frame(&mut app).0, &mut all);
    let paper = egui::Color32::from_rgb(0xff, 0xee, 0xcc);
    assert!(all.iter().any(|s| matches!(s, egui::Shape::Path(p) if p.fill == paper)), "the artboard shows the paper colour");
}

/// An artboard's name sits above the artboard's own top-left corner, also when a rotated view
/// turns that corner away from the corner of its box on screen (#1095).
#[test]
fn an_artboard_name_stays_at_its_corner_in_a_rotated_view() {
    let mut app = app();
    app.run("artboard.new", json!({})).unwrap();
    for rotation in [0.0, 60.0, -74.0] {
        app.view_mut().unwrap().rotation = rotation;
        let (shapes, _, _) = frame(&mut app);
        let corner = outlines(&shapes, egui::Color32::from_gray(0))[0][0];
        let mut all = vec![];
        flatten(&shapes, &mut all);
        let label = all
            .iter()
            .find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text().starts_with("01 - ") => Some(t.pos + egui::vec2(0.0, t.galley.size().y + 4.0)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{rotation}°: no name"));
        assert!((label - corner).length() < 1.0, "{rotation}°: the name ends at {label:?}, the corner is {corner:?}");
    }
}
