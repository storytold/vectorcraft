use super::{Color32, Pos2, Shape, VectorcraftApp, pos2};
// PLAN: assumed crate paths `craft_ui_canvas::canvas_controls::{CanvasGrid, GridLine}` and `craft_ui_canvas::style::DataColor`
use craft_ui_canvas::canvas_controls::{CanvasGrid, GridLine};
use craft_ui_canvas::style::DataColor;

/// The transparency grid's 2 × 2-cell tile in the document's colours (Document Setup), converted
/// into the display profile `display` when there is one, cached per look, and its cell size in
/// screen points.
pub(crate) fn checker_texture(
    ctx: &egui::Context,
    setup: &vectorcraft_doc::DocSetup,
    display: Option<&vectorcraft_color::cms::display::DisplayTransform>,
) -> (egui::TextureHandle, f32) {
    // A tile of its own with a display profile, so a preview drawn in sRGB beside the canvas
    // doesn't make the canvas's tile again each frame.
    let id = egui::Id::unique("transparency-grid-tile").with(display.is_some());
    (checker_tile(ctx, setup.grid_colors, display, id), setup.grid_size.cell())
}

/// The transparency grid's 2 × 2-cell tile in the colors `grid_colors` (the first in the top-left
/// and bottom-right cells), converted into the display profile `display` when there is one, and
/// cached under `id` until the colors change.
pub(crate) fn checker_tile(
    ctx: &egui::Context,
    grid_colors: [vectorcraft_color::Color; 2],
    display: Option<&vectorcraft_color::cms::display::DisplayTransform>,
    id: egui::Id,
) -> egui::TextureHandle {
    let colors = grid_colors.map(|c| crate::display_profile::color32(display, crate::panels::c32(&c)));
    let cached = ctx.data(|d| d.get_temp::<([Color32; 2], egui::TextureHandle)>(id)).filter(|(c, _)| *c == colors);
    cached.map(|(_, t)| t).unwrap_or_else(|| {
        let [a, b] = colors;
        let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
        let opts = egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::NEAREST };
        let t = ctx.load_texture(TRANSPARENCY_GRID, img, opts);
        ctx.data_mut(|d| d.insert_temp(id, (colors, t.clone())));
        t
    })
}

/// Name of the transparency grid's texture.
pub(crate) const TRANSPARENCY_GRID: &str = "transparency grid";

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
    let grid = CanvasGrid::new("Grid")
        .spacing((spacing, spacing).into())
        .subdivisions(st.doc.grid.subdivisions)
        .color(DataColor::new(color))
        .line(line);
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

/// The part of the segment from `a` to `b` inside `r`, none when the segment misses it.
pub(super) fn clip_segment(a: Pos2, b: Pos2, r: egui::Rect) -> Option<(Pos2, Pos2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    // Each edge as (the run toward it, the room before it).
    for (run, room) in [(-d.x, a.x - r.left()), (d.x, r.right() - a.x), (-d.y, a.y - r.top()), (d.y, r.bottom() - a.y)] {
        if run == 0.0 {
            if room < 0.0 {
                return None;
            }
        } else if run < 0.0 {
            t0 = t0.max(room / run);
        } else {
            t1 = t1.min(room / run);
        }
    }
    (t0 <= t1).then(|| (a + d * t0, a + d * t1))
}

/// A ruler label for `v` (in the ruler's unit) on a ruler whose labels are `step` apart: whole
/// numbers, or the fewest decimals that write `step` exactly, so a binary step keeps its value
/// (an inch ruler's 1/16 reads `0.0625`, not the `0.06` that rounding the step's magnitude gave).
pub(crate) fn ruler_label(v: f64, step: f64) -> String {
    let exact = |d: u32| (step * 10f64.powi(d as i32)).fract().abs() < 1e-9;
    let decimals = (0..=7).find(|d| exact(*d)).unwrap_or(7) as usize;
    let s = format!("{v:.decimals$}");
    // Trailing zeros go, as `Unit::number` drops them: a sixteenth-inch ruler labels the inch `1`,
    // not `1.0000`, which the vertical ruler would spell down six lines.
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { "0".into() } else { s.into() }
}
