//! Paint previews: the gradient face a paint well draws, and the paint a well shows.
//!
//! The framework's well draws the checkerboard under every face, so a face paints only its own
//! colours.

use std::hash::{Hash, Hasher};

use craft::color_ui::ui::{self as color_ui, GradientPreview, ImagePreview};
use craft::theme::Srgba;
use craft::ui::primitives::{CachedTexture, ContentPainterCx, FacePaint};
use egui::emath::GuiRounding;
use egui::{Color32, Mesh, Rect, Shape, Stroke, pos2};
use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};

/// A face drawing `gradient` as it would look on an object of the well's shape: linear along its
/// angle, radial from the centre, freeform from its colour points. The rect is rounded to whole
/// pixels, as filled rectangles are when drawn, so the face lines up with a colour well beside it
/// (#3826).
pub fn paint_face(gradient: &GradientPaint) -> FacePaint {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // The serialised form holds every field the mesh reads.
    serde_json::to_string(gradient).unwrap_or_default().hash(&mut hasher);
    let gradient = gradient.clone();
    FacePaint::new(hasher.finish(), move |cx, rect| gradient_mesh(cx, rect, &gradient))
}

fn gradient_mesh(cx: &ContentPainterCx<'_>, rect: Rect, gradient: &GradientPaint) {
    let rect = rect.round_to_pixels(cx.painter.pixels_per_point());
    if !(rect.width() > 0.0 && rect.height() > 0.0) {
        return;
    }
    let (cols, rows) = chip_grid(&gradient.gradient, rect.width(), rect.height());
    let colors = gradient_chip_colors(gradient, rect.width(), rect.height(), cols, rows);
    let mut mesh = Mesh::default();
    for (k, color) in colors.into_iter().enumerate() {
        let (i, j) = (k % (cols + 1), k / (cols + 1));
        let at = pos2(rect.left() + rect.width() * i as f32 / cols as f32, rect.top() + rect.height() * j as f32 / rows as f32);
        mesh.colored_vertex(at, color);
    }
    for j in 0..rows as u32 {
        for i in 0..cols as u32 {
            let v = j * (cols as u32 + 1) + i;
            mesh.add_triangle(v, v + 1, v + cols as u32 + 1);
            mesh.add_triangle(v + 1, v + cols as u32 + 2, v + cols as u32 + 1);
        }
    }
    cx.data_shapes([Shape::mesh(mesh)]);
}

/// The solid colour of `color` as the framework's colour type.
fn srgba(color: &vectorcraft_color::Color) -> Srgba {
    let [r, g, b, _] = color.to_rgba8(1.0);
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// `paint` as a paint well shows it. `tile` gives the texture of the pattern named to it; `None`
/// shows the field fill until the host has one.
pub fn well_paint(paint: &Paint, tile: &dyn Fn(&str) -> Option<CachedTexture>) -> color_ui::Paint {
    match paint {
        Paint::None => color_ui::Paint::None,
        Paint::Solid { color, .. } => color_ui::Paint::Solid(srgba(color)),
        Paint::Gradient(g) => {
            let stops = g.gradient.stops.iter().map(|s| (s.offset, srgba(&s.color).with_alpha(s.opacity))).collect();
            color_ui::Paint::Gradient(GradientPreview::new(stops).face(paint_face(g)))
        }
        Paint::Pattern { pattern, .. } => color_ui::Paint::Pattern(ImagePreview::new(pattern.clone(), tile(pattern))),
    }
}

/// A face painting `paint` with [`paint_rect`], keyed by every field of the paint.
pub fn rect_face(paint: &Paint) -> FacePaint {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(paint).unwrap_or_default().hash(&mut hasher);
    let paint = paint.clone();
    FacePaint::new(hasher.finish(), move |cx, rect| paint_rect(cx, rect, &paint))
}

/// Paints `paint` into `rect` of a content painter (a `Picture::painter` body). Nothing is drawn
/// for no paint: the picture's own surface shows.
pub fn paint_rect(cx: &ContentPainterCx<'_>, rect: Rect, paint: &Paint) {
    match paint {
        Paint::None => {}
        Paint::Solid { color, .. } => {
            let [r, g, b, _] = color.to_rgba8(1.0);
            cx.data_shapes([Shape::rect_filled(rect, 0.0, Color32::from_rgb(r, g, b))]);
        }
        Paint::Gradient(g) => paint_face(g).paint(cx, rect),
        Paint::Pattern { .. } => {
            let mut shapes = vec![Shape::rect_filled(rect, 0.0, Color32::from_gray(200))];
            for i in 0..4 {
                let x = rect.left() + rect.width() * i as f32 / 4.0;
                shapes.push(Shape::line_segment(
                    [pos2(x, rect.bottom()), pos2(x + rect.width() / 4.0, rect.top())],
                    Stroke::new(1.0, Color32::from_gray(90)),
                ));
            }
            cx.data_shapes(shapes);
        }
    }
}

/// Most grid cells across (and down) a chip of a gradient that repeats or reflects.
const SPREAD_CHIP_CELLS: usize = 128;

/// The grid (columns, rows) a `width` x `height` chip of gradient `g` is sampled on: a cell about
/// every 3 points (at most 64 a side, 16 for a freeform gradient, whose sampling is costly per
/// spot; the mesh interpolates between them), and for a gradient that repeats or reflects at
/// least 4 cells a run along each side (at most one a point, and [`SPREAD_CHIP_CELLS`]), so the
/// grid's points do not all fall at the same place in a run and paint the chip one color. For
/// such a gradient a side gets a cell or two more when its points would fall at fewer than three
/// places in a run.
pub fn chip_grid(g: &Gradient, width: f32, height: f32) -> (usize, usize) {
    let freeform = g.kind == GradientKind::Freeform;
    let max = if freeform { 16 } else { 64 };
    let runs = if g.is_spread() { g.repeat_count() as usize } else { 1 };
    let least = if runs > 1 { (4 * runs).min(SPREAD_CHIP_CELLS) } else { 1 };
    let gcd = |mut a: usize, mut b: usize| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let side = |len: f32| {
        let mut cells = ((len / 3.0).ceil() as usize).clamp(1, max).max(least.min(len.ceil() as usize));
        // Points `i / cells` of the way along fall at `cells / gcd(cells, runs)` places in a run.
        while runs > 1 && cells / gcd(cells, runs) < 3 {
            cells += 1;
        }
        cells
    };
    (side(width), side(height))
}

/// The colours at the `(cols + 1) x (rows + 1)` grid points (row by row) of a `width` x `height`
/// chip showing `gp` as it would look on an object of that size, with the gradient's opacity as
/// their alpha.
pub fn gradient_chip_colors(gp: &GradientPaint, width: f32, height: f32, cols: usize, rows: usize) -> Vec<Color32> {
    let b = vectorcraft_geom::kurbo::Rect::new(0.0, 0.0, width as f64, height as f64);
    let rgba = |(c, a): (vectorcraft_color::Color, f32)| {
        let [r, g, b, a] = c.to_rgba8(if a.is_finite() { a } else { 1.0 });
        Color32::from_rgba_unmultiplied(r, g, b, a)
    };
    let at = |k: usize| {
        let (i, j) = (k % (cols + 1), k / (cols + 1));
        vectorcraft_geom::kurbo::Point::new(b.width() * i as f64 / cols as f64, b.height() * j as f64 / rows as f64)
    };
    let n = (cols + 1) * (rows + 1);
    if gp.gradient.kind == GradientKind::Freeform {
        let Some(painted) = vectorcraft_color::freeform::painted_box(b) else { return vec![Color32::TRANSPARENT; n] };
        let field = gp.freeform_on(painted).model_field(vectorcraft_color::freeform::spread_scale(painted));
        return (0..n)
            .map(|k| {
                let ([r, g, b], a) = field.display_sample(at(k));
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                Color32::from_rgba_unmultiplied(q(r), q(g), q(b), q(if a.is_finite() { a } else { 1.0 }))
            })
            .collect();
    }
    // A placed gradient keeps its direction; a fresh one is fitted to the chip at its angle.
    let angle = gp.geom.map_or(gp.angle, |g| g.angle_deg());
    let mut geom = GradientGeom::fit(gp.gradient.kind, b, if angle.is_finite() { angle } else { 0.0 });
    // A mirrored conic's chip turns the way the gradient does on its object.
    if gp.gradient.kind == GradientKind::Conic && gp.geom.is_some_and(|g| g.is_mirrored()) {
        geom.aspect = -1.0;
    }
    (0..n)
        .map(|k| {
            let t = gp.gradient.spread_param(geom.param_at(gp.gradient.kind, at(k))).clamp(0.0, 1.0) as f32;
            rgba(gp.gradient.sample(t))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::Color;

    fn paint(kind: GradientKind, angle: f64) -> GradientPaint {
        let mut gp = GradientPaint::new(Gradient { kind, ..Gradient::default() });
        gp.angle = angle;
        gp
    }

    #[test]
    fn linear_chip_follows_the_angle() {
        // 5 x 5 grid points on a 20 x 20 chip; White (start) to Black (end).
        let at = |colors: &[Color32], i: usize, j: usize| colors[j * 5 + i];
        let side = gradient_chip_colors(&paint(GradientKind::Linear, 0.0), 20.0, 20.0, 4, 4);
        assert_eq!(at(&side, 0, 2), Color32::WHITE);
        assert_eq!(at(&side, 4, 2), Color32::BLACK);
        assert_eq!(at(&side, 2, 0), at(&side, 2, 4));
        // 90 degrees runs bottom to top: constant along rows, white at the bottom.
        let up = gradient_chip_colors(&paint(GradientKind::Linear, 90.0), 20.0, 20.0, 4, 4);
        assert_eq!(at(&up, 0, 4), Color32::WHITE);
        assert_eq!(at(&up, 0, 0), Color32::BLACK);
        assert_eq!(at(&up, 0, 2), at(&up, 4, 2));
    }

    /// A conic chip sweeps clockwise from its angle, or counterclockwise when the gradient is
    /// mirrored.
    #[test]
    fn conic_chip_sweeps_the_way_the_gradient_turns() {
        let at = |colors: &[Color32], i: usize, j: usize| colors[j * 5 + i].r();
        let mut gp = paint(GradientKind::Conic, 0.0);
        let chip = gradient_chip_colors(&gp, 20.0, 20.0, 4, 4);
        // From the right: a quarter turn on is below the center, halfway around left of it.
        assert!(at(&chip, 2, 4).abs_diff(191) <= 2 && at(&chip, 2, 0).abs_diff(64) <= 2, "{chip:?}");
        assert!(at(&chip, 0, 2).abs_diff(128) <= 2, "{chip:?}");
        let c = vectorcraft_geom::kurbo::Point::new(50.0, 50.0);
        gp.geom =
            Some(vectorcraft_color::GradientGeom { start: c, end: c + vectorcraft_geom::kurbo::Vec2::new(10.0, 0.0), aspect: -1.0, focal: None });
        let mirrored = gradient_chip_colors(&gp, 20.0, 20.0, 4, 4);
        assert!(at(&mirrored, 2, 4).abs_diff(64) <= 2 && at(&mirrored, 2, 0).abs_diff(191) <= 2, "{mirrored:?}");
    }

    /// A diamond chip runs from the center out to a square whose edges pass through the chip's
    /// corners: halfway along an edge and halfway to a corner are the same color.
    #[test]
    fn diamond_chip_runs_out_to_the_corners() {
        let at = |colors: &[Color32], i: usize, j: usize| colors[j * 5 + i].r();
        let chip = gradient_chip_colors(&paint(GradientKind::Diamond, 0.0), 20.0, 20.0, 4, 4);
        assert_eq!((at(&chip, 2, 2), at(&chip, 0, 0), at(&chip, 4, 4)), (255, 0, 0), "{chip:?}");
        for (i, j) in [(2, 0), (0, 2), (1, 1), (3, 3)] {
            assert!(at(&chip, i, j).abs_diff(128) <= 2, "({i}, {j}): {chip:?}");
        }
    }

    /// A gradient that repeats or reflects as many times as a chip has cells (8 on a 24-point chip)
    /// still shows its runs: the grid gets more cells than runs, so its points do not all sample
    /// the same place in a run.
    #[test]
    fn a_chip_repeated_once_a_cell_is_not_one_color() {
        for spread in [vectorcraft_color::GradientSpread::Repeat, vectorcraft_color::GradientSpread::Reflect] {
            let mut gp = paint(GradientKind::Linear, 0.0);
            gp.gradient.spread = spread;
            gp.gradient.repeats = 8;
            let (cols, rows) = chip_grid(&gp.gradient, 24.0, 24.0);
            assert!(cols > 8 && rows > 8, "{spread:?}: {cols} x {rows}");
            let chip = gradient_chip_colors(&gp, 24.0, 24.0, cols, rows);
            let row: std::collections::HashSet<Color32> = chip.iter().take(cols + 1).copied().collect();
            assert!(row.len() > 2, "{spread:?}: {row:?}");
        }
        // A gradient that pads keeps a cell about every 3 points.
        assert_eq!(chip_grid(&Gradient::default(), 24.0, 24.0), (8, 8));
        // Many runs give a small chip at most a cell a point.
        let mut many = paint(GradientKind::Linear, 0.0).gradient;
        many.spread = vectorcraft_color::GradientSpread::Repeat;
        many.repeats = 100;
        assert_eq!(chip_grid(&many, 24.0, 12.0), (24, 12));
        assert_eq!(chip_grid(&many, 400.0, 400.0), (128, 128));
        // A count that matches a chip's points still shows its runs.
        let mut gp = paint(GradientKind::Linear, 0.0);
        gp.gradient.spread = vectorcraft_color::GradientSpread::Repeat;
        gp.gradient.repeats = 23;
        let (cols, rows) = chip_grid(&gp.gradient, 23.0, 23.0);
        let chip = gradient_chip_colors(&gp, 23.0, 23.0, cols, rows);
        let row: std::collections::HashSet<Color32> = chip.iter().take(cols + 1).copied().collect();
        assert!(row.len() > 2, "{cols}: {row:?}");
    }

    #[test]
    fn radial_chip_runs_from_the_centre_out() {
        let colors = gradient_chip_colors(&paint(GradientKind::Radial, 0.0), 20.0, 20.0, 4, 4);
        assert_eq!(colors[2 * 5 + 2], Color32::WHITE);
        assert_eq!(colors[0], Color32::BLACK);
    }

    /// A stop at opacity 0 is clear on the chip of every kind, and one at 50% half clear.
    #[test]
    fn chip_shows_the_stops_opacity() {
        use vectorcraft_color::GradientStop;
        for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Conic] {
            let mut gp = paint(kind, 0.0);
            gp.gradient.stops = vec![
                GradientStop { opacity: 0.5, ..GradientStop::new(0.0, Color::BLACK) },
                GradientStop { opacity: 0.0, ..GradientStop::new(1.0, Color::BLACK) },
            ];
            let chip = gradient_chip_colors(&gp, 20.0, 20.0, 4, 4);
            let (lo, hi) = chip.iter().fold((u8::MAX, 0), |(lo, hi), c| (lo.min(c.a()), hi.max(c.a())));
            // No grid point of a conic chip lands exactly at the end of its sweep.
            assert!(lo <= if kind == GradientKind::Conic { 12 } else { 0 }, "{kind:?}: {chip:?}");
            assert!(hi.abs_diff(128) <= 1, "{kind:?}: {chip:?}");
        }
        let at = |colors: &[Color32], i: usize, j: usize| colors[j * 5 + i];
        let mut gp = paint(GradientKind::Linear, 0.0);
        gp.gradient.stops[1].opacity = 0.0;
        let chip = gradient_chip_colors(&gp, 20.0, 20.0, 4, 4);
        assert_eq!(at(&chip, 0, 2), Color32::WHITE);
        assert_eq!(at(&chip, 4, 2).a(), 0);
        // Radial: an opaque edge around a clear center.
        let mut gp = paint(GradientKind::Radial, 0.0);
        gp.gradient.stops[0].opacity = 0.0;
        let chip = gradient_chip_colors(&gp, 20.0, 20.0, 4, 4);
        assert_eq!(at(&chip, 2, 2).a(), 0);
        assert_eq!(chip[0], Color32::BLACK);
    }

    #[test]
    fn freeform_chip_shows_the_points_opacity() {
        use vectorcraft_color::{Freeform, FreeformPoint};
        use vectorcraft_geom::kurbo::Point;
        let mut gp = paint(GradientKind::Freeform, 0.0);
        let clear = FreeformPoint { opacity: 0.0, ..FreeformPoint::new(Point::new(10.0, 10.0), Color::BLACK) };
        gp.freeform = Some(Freeform { points: vec![clear], ..Default::default() });
        let chip = gradient_chip_colors(&gp, 20.0, 20.0, 4, 4);
        assert!(chip.iter().all(|c| c.a() == 0), "{chip:?}");
    }

    /// A Holographic glow's chip is clear at the corners and nearly opaque at the center; a foil's
    /// chip is opaque everywhere.
    #[test]
    fn holographic_chips_show_a_glow_clear_at_its_edge_and_a_foil_opaque() {
        let lib = vectorcraft_color::libraries::builtin_library("holographic").unwrap();
        let chip = |name: &str| match &lib.swatch(name).unwrap().paint {
            Paint::Gradient(g) => gradient_chip_colors(g, 20.0, 20.0, 4, 4),
            p => panic!("{name}: {p:?}"),
        };
        let glow = chip("Mint Halo");
        assert!([0, 4, 20, 24].iter().all(|&i| glow[i].a() == 0), "{glow:?}");
        assert!(glow[2 * 5 + 2].a() >= 240, "{glow:?}");
        let foil = chip("Prism Veil");
        assert!(foil.iter().all(|c| c.a() == 255), "{foil:?}");
    }

    #[test]
    fn freeform_chip_mixes_points_in_the_model_they_share() {
        use vectorcraft_color::{Freeform, FreeformPoint};
        use vectorcraft_geom::kurbo::Point;
        let chip = |a: Color, b: Color| {
            let p = |x, c| FreeformPoint::new(Point::new(x, 10.0), c);
            let mut gp = paint(GradientKind::Freeform, 0.0);
            gp.freeform = Some(Freeform { points: vec![p(0.0, a), p(20.0, b)], ..Default::default() });
            gradient_chip_colors(&gp, 20.0, 20.0, 4, 4)[2 * 5 + 2]
        };
        let near = |got: Color32, want: [u8; 4]| [got.r(), got.g(), got.b()].iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 3);
        let got = chip(Color::cmyk(1.0, 0.0, 0.0, 0.0), Color::cmyk(0.0, 0.0, 0.0, 1.0));
        let want = Color::cmyk(0.5, 0.0, 0.0, 0.5).to_rgba8(1.0);
        assert!(near(got, want), "CMYK: {got:?} vs {want:?}");
        let got = chip(Color::lab(90.0, -60.0, 40.0), Color::lab(30.0, 60.0, -40.0));
        let want = Color::lab(60.0, 0.0, 0.0).to_rgba8(1.0);
        assert!(near(got, want), "Lab: {got:?} vs {want:?}");
    }

    /// At a display scale that puts the picture between pixels, the face's mesh edges fall on whole
    /// pixels, as a filled rectangle's do (#3826): the toolbar's Gradient button sat up to half a
    /// pixel off its neighbours.
    #[test]
    fn gradient_face_lines_up_with_whole_pixels() {
        use craft::ui::contract::Show as _;
        use craft::ui::display::Picture;
        use craft::ui::list::ThumbAspect;
        use craft::ui::testing::Fixture;
        let gp = paint(GradientKind::Linear, 0.0);
        let mut fixture = Fixture::new(move |ui| {
            let face = paint_face(&gp);
            Picture::painter("", ThumbAspect::new(1, 1), move |cx, rect| face.paint(cx, rect)).show(ui);
        });
        let ppp = 1.5;
        fixture.ctx().set_pixels_per_point(ppp);
        fixture.ctx().tessellation_options_mut(|o| o.feathering = false);
        fixture.frame(vec![]);
        let shapes = fixture.frame_shapes();
        fn meshes<'a>(shape: &'a Shape, out: &mut Vec<&'a Mesh>) {
            match shape {
                Shape::Mesh(mesh) => out.push(mesh),
                Shape::Vec(inner) => inner.iter().for_each(|s| meshes(s, out)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        shapes.iter().for_each(|s| meshes(s, &mut found));
        let mesh = found.last().expect("the face paints a mesh");
        let whole = |v: f32| ((v * ppp).round() - v * ppp).abs() < 1e-3;
        assert!(mesh.vertices.iter().all(|v| whole(v.pos.x) && whole(v.pos.y)), "{:?}", mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>());
    }
}
