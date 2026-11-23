use craft::ui::canvas_controls::{CanvasGrid, GridLine};
use craft::ui::style::DataColor;
use egui::{Color32, Pos2, Shape, pos2};

use super::VectorcraftApp;

/// One quad drawn with `tex`: its corners `quad` (from the top left, clockwise) show the texture
/// from (0, 0) to (`u`, `v`).
pub(super) fn textured_quad(p: &egui::Painter, tex: egui::TextureId, quad: &[Pos2], (u, v): (f32, f32)) {
    let mut mesh = egui::Mesh::with_texture(tex);
    for (pos, uv) in quad.iter().zip([pos2(0.0, 0.0), pos2(u, 0.0), pos2(u, v), pos2(0.0, v)]) {
        mesh.vertices.push(egui::epaint::Vertex { pos: *pos, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    p.add(Shape::mesh(mesh));
}

/// The zoom the pixel grid shows from in Pixel Preview (600%), and its colour: a translucent grey,
/// so the art's pixels read through it.
pub(super) const PIXEL_GRID_ZOOM: f64 = 6.0;

pub(super) const PIXEL_GRID: Color32 = Color32::from_rgba_premultiplied(64, 64, 64, 96);

/// The grid (View → Show Grid) as a canvas control, with whether it lies behind the art (Guides &
/// Grid › Grids In Back): the host passes `CanvasGrid::picture()` for it and orders it before or
/// after the picture. `None` while the grid is hidden or presenting.
pub(super) fn document_grid(app: &VectorcraftApp, st: &vectorcraft_engine::DocState) -> Option<(CanvasGrid, bool)> {
    if !app.ui.view.grid_shown() || app.ui.screen_mode >= 3 {
        return None;
    }
    let prefs = &app.session.prefs;
    let (line, color) = grid_look(prefs);
    let spacing = st.doc.grid.spacing;
    let grid =
        CanvasGrid::new("Grid").spacing((spacing, spacing).into()).subdivisions(st.doc.grid.subdivisions).color(DataColor::new(color)).line(line);
    Some((grid, prefs.grids_in_back))
}

/// How the grid's lines are drawn and their colour (Preferences › Guides & Grid): the Dots style
/// puts a dot at the crossings.
pub(super) fn grid_look(prefs: &vectorcraft_engine::Prefs) -> (GridLine, Color32) {
    let line = if prefs.grid_style == "dots" { GridLine::Crossings } else { GridLine::Solid };
    (line, pref_color(&prefs.grid_color, Color32::from_gray(200)))
}

/// The pixel grid (Guides & Grid › Show Pixel Grid (Above 600% Zoom)): a line at every document
/// pixel in Pixel Preview, which the control draws from [`PIXEL_GRID_ZOOM`] on (its `min_zoom` is
/// the device zoom, as the view's).
pub(super) fn pixel_grid(app: &VectorcraftApp) -> Option<CanvasGrid> {
    if app.ui.screen_mode >= 3 || !app.ui.view.pixel_preview || !app.session.prefs.show_pixel_grid {
        return None;
    }
    Some(CanvasGrid::new("Pixel grid").spacing((1.0, 1.0).into()).subdivisions(1).color(DataColor::new(PIXEL_GRID)).min_zoom(PIXEL_GRID_ZOOM))
}

/// A `#rrggbb` colour preference, `fallback` when it doesn't parse.
pub(super) fn pref_color(hex: &str, fallback: Color32) -> Color32 {
    vectorcraft_color::Color::from_hex(hex).map_or(fallback, |c| crate::panels::c32(&c))
}

