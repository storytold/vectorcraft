use super::drag::mods;
use super::*;

/// The transparency grid's 2 × 2-cell tile in the document's colours (Document Setup), cached per
/// look, and its cell size in screen points.
pub(crate) fn checker_texture(ctx: &egui::Context, setup: &vectorcraft_doc::DocSetup) -> (egui::TextureHandle, f32) {
    let colors = setup.grid_colors.map(|c| crate::panels::c32(&c));
    let id = egui::Id::unique("transparency-grid-tile");
    let cached = ctx.data(|d| d.get_temp::<([Color32; 2], egui::TextureHandle)>(id)).filter(|(c, _)| *c == colors);
    let tex = cached.map(|(_, t)| t).unwrap_or_else(|| {
        let [a, b] = colors;
        let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
        let opts = egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::NEAREST };
        let t = ctx.load_texture(TRANSPARENCY_GRID, img, opts);
        ctx.data_mut(|d| d.insert_temp(id, (colors, t.clone())));
        t
    });
    (tex, setup.grid_size.cell())
}

/// Name of the transparency grid's texture.
pub(crate) const TRANSPARENCY_GRID: &str = "transparency grid";

/// The transparency grid over the artboard `ab` (screen corners `quad`): one textured quad whose
/// cells stay `cell` screen points at every zoom and turn with a rotated view.
pub(super) fn checker(p: &egui::Painter, quad: &[Pos2], ab: Rect, zoom: f64, tex: egui::TextureId, cell: f32) {
    // Two cells per texture repeat.
    let per = (2.0 * cell as f64 / zoom).max(1e-9);
    textured_quad(p, tex, quad, ((ab.width() / per) as f32, (ab.height() / per) as f32));
}

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

/// The look of the grid or of the guides (Preferences › Guides & Grid › Color and Style).
#[derive(Clone, Copy)]
pub(super) struct LineLook {
    pub(super) color: Color32,
    pub(super) dots: bool,
}

impl LineLook {
    pub(super) fn grid(prefs: &vectorcraft_engine::Prefs) -> Self {
        Self { color: pref_color(&prefs.grid_color, Color32::from_gray(200)), dots: prefs.grid_style == "dots" }
    }

    pub(super) fn guides(prefs: &vectorcraft_engine::Prefs, t: &Tokens) -> Self {
        Self { color: pref_color(&prefs.guide_color, t.guide), dots: prefs.guide_style == "dots" }
    }

    /// A guide from `a` to `b`: a line, or dots.
    pub(super) fn guide(self, p: &egui::Painter, a: Pos2, b: Pos2) {
        if self.dots {
            p.extend(Shape::dotted_line(&[a, b], self.color, 4.0, 0.75));
        } else {
            p.line_segment([a, b], Stroke::new(1.0, self.color));
        }
    }
}

/// A `#rrggbb` colour preference, `fallback` when it doesn't parse.
pub(super) fn pref_color(hex: &str, fallback: Color32) -> Color32 {
    vectorcraft_color::Color::from_hex(hex).map_or(fallback, |c| crate::panels::c32(&c))
}

/// The view in document space: the box round the canvas's corners (the view may be rotated).
pub(super) fn view_bounds(xf: &Xf) -> Rect {
    let r = xf.rect;
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|c| xf.to_doc(c));
    corners.iter().fold(Rect::from_points(corners[0], corners[0]), |b, c| b.union_pt(*c))
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

/// The grid (View → Show Grid): a gridline every `spacing` and `subdiv` lighter lines between
/// (left out when they would crowd). In the Dots style, a dot where gridlines cross the
/// subdivisions instead.
pub(super) fn grid(p: &egui::Painter, xf: &Xf, spacing: f64, subdiv: u32, look: LineLook) {
    // The painter clips to the canvas.
    let view = view_bounds(xf);
    let sub = spacing / subdiv.max(1) as f64;
    let step = if sub * xf.zoom >= 6.0 { sub } else { spacing };
    // Dots crowd sooner than lines.
    if step * xf.zoom < 4.0 || (look.dots && spacing * xf.zoom < 8.0) {
        return;
    }
    let major = |v: f64| (v / spacing - (v / spacing).round()).abs() < 1e-6;
    // The lines across the view (where along their axis), at least 4 px apart (and capped).
    let lines = |from: f64, to: f64| {
        let first = (from / step).floor();
        (0..10_000u32).map(move |i| (first + f64::from(i)) * step).take_while(move |v| *v <= to)
    };
    if look.dots {
        let mut mesh = egui::Mesh::default();
        for x in lines(view.x0, view.x1) {
            for y in lines(view.y0, view.y1).filter(|y| major(x) || major(*y)) {
                mesh.add_colored_rect(egui::Rect::from_center_size(xf.to_screen(Point::new(x, y)), vec2(1.5, 1.5)), look.color);
            }
        }
        p.add(Shape::mesh(mesh));
        return;
    }
    let minor = look.color.gamma_multiply(0.4);
    let stroke = |major: bool| Stroke::new(1.0, if major { look.color } else { minor });
    for x in lines(view.x0, view.x1) {
        p.line_segment([xf.to_screen(Point::new(x, view.y0)), xf.to_screen(Point::new(x, view.y1))], stroke(major(x)));
    }
    for y in lines(view.y0, view.y1) {
        p.line_segment([xf.to_screen(Point::new(view.x0, y)), xf.to_screen(Point::new(view.x1, y))], stroke(major(y)));
    }
}

/// A ruler label for `v` (in the ruler's unit) on a ruler whose labels are `step` apart: whole
/// numbers, or the fewest decimals that write `step` exactly, so a binary step keeps its value
/// (an inch ruler's 1/16 reads `0.0625`, not the `0.06` that rounding the step's magnitude gave).
/// How far down the left ruler a label of `chars` characters reaches, counting from its top: it
/// is spelled one character per line, [`RULER_CHAR`] apart, and the last one is a line tall.
pub(crate) fn vertical_label_height(chars: usize) -> f32 {
    chars.saturating_sub(1) as f32 * RULER_CHAR + RULER_LINE
}

/// A ruler label's line spacing and line height on the left ruler.
const RULER_CHAR: f32 = 8.5;

const RULER_LINE: f32 = 9.5;

pub(crate) fn ruler_label(v: f64, step: f64) -> String {
    let exact = |d: u32| (step * 10f64.powi(d as i32)).fract().abs() < 1e-9;
    let decimals = (0..=7).find(|d| exact(*d)).unwrap_or(7) as usize;
    let s = format!("{v:.decimals$}");
    // Trailing zeros go, as `Unit::number` drops them: a sixteenth-inch ruler labels the inch `1`,
    // not `1.0000`, which the vertical ruler would spell down six lines.
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { "0".into() } else { s.into() }
}

/// The rulers, numbered in `unit` (the General unit).
/// The top ruler, the left ruler and the box where they meet.
pub(super) fn ruler_rects(full: egui::Rect) -> [egui::Rect; 3] {
    let top = egui::Rect::from_min_max(pos2(full.left() + RULER, full.top()), pos2(full.right(), full.top() + RULER));
    let left = egui::Rect::from_min_max(pos2(full.left(), full.top() + RULER), pos2(full.left() + RULER, full.bottom()));
    [top, left, egui::Rect::from_min_size(full.min, vec2(RULER, RULER))]
}

/// A drag from a ruler onto the canvas makes a guide where the button is released (the engine's
/// `Session::ruler_guide`): a horizontal one from the top ruler, a vertical one from the left
/// ruler, snapped as a moved guide is (with Shift to the ruler's ticks). Released anywhere else,
/// it makes none. The corner drags the rulers' origin or resets it on a double-click.
pub(super) fn ruler_input(app: &mut VectorcraftApp, ui: &Ui, full: egui::Rect, canvas: egui::Rect, xf: &Xf) -> Option<Point> {
    let [top, left, corner] = ruler_rects(full);
    // Right-click on a ruler or the origin box: the document units, to swap between them as
    // Preferences ▸ Units ▸ General does ([`crate::menus::ruler_menu_body`]).
    let mut unit_clicked = None;
    for (r, vertical, id) in [(top, false, "ruler-top"), (left, true, "ruler-left")] {
        // Click-and-drag so the same widget drags out guides (left) and opens the unit menu (right).
        let resp = ui.interact(r, egui::Id::unique(id), Sense::click_and_drag());
        resp.context_menu(|ui| crate::menus::ruler_menu_body(app, ui, &mut unit_clicked));
        let kind = if resp.drag_stopped() {
            PointerKind::Up
        } else if resp.dragged() {
            PointerKind::Drag
        } else {
            continue;
        };
        let p = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
        let on_canvas = p.is_some_and(|p| canvas.contains(p));
        if on_canvas {
            // The new guide shows even if guides were hidden.
            app.ui.view.guides = true;
        }
        let ev = PointerEvent { kind, pos: p.map_or(Point::ZERO, |p| xf.to_doc(p)), mods: mods(ui.input(|i| i.modifiers), false), pressure: 1.0 };
        if let Err(e) = app.session.ruler_guide(vertical, &ev, on_canvas, app.view_info()) {
            app.status(e.to_string());
        }
    }
    let corner = ui.interact(corner, egui::Id::unique("ruler-corner"), Sense::click_and_drag());
    corner.context_menu(|ui| crate::menus::ruler_menu_body(app, ui, &mut unit_clicked));
    if let Some((id, p)) = unit_clicked {
        crate::menus::invoke(app, &id, p);
    }
    if corner.double_clicked_by(egui::PointerButton::Primary) {
        let artboard = app.view().map_or(0, |v| v.artboard);
        if let Err(e) = app.run("document.resetRulerOrigin", json!({"artboard": artboard})) {
            app.status(e.to_string());
        }
    }
    // Keep previews in the UI: only a release on the canvas edits the document, once. Cancelling
    // never adds an undo step or interferes with the drawing tool's interaction.
    let drag_id = ruler_origin_drag_id();
    let uid = app.session.active().map(|d| d.uid);
    if corner.drag_started_by(egui::PointerButton::Primary) {
        ui.data_mut(|d| d.insert_temp(drag_id, (uid, false)));
    }
    let mut preview = None;
    if let Some((started_in, mut cancelled)) = ui.data(|d| d.get_temp::<(Option<u64>, bool)>(drag_id)) {
        cancelled |= started_in != uid || ui.input(|i| !i.focused || i.key_pressed(egui::Key::Escape));
        let at = corner.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos())).filter(|p| canvas.contains(*p)).map(|p| xf.to_doc(p));
        if corner.drag_stopped_by(egui::PointerButton::Primary) {
            ui.data_mut(|d| d.remove::<(Option<u64>, bool)>(drag_id));
            if !cancelled
                && let Some(at) = at
                && let Err(e) = app.run("document.setRulerOrigin", json!({"x": at.x, "y": at.y, "artboard": app.view().map_or(0, |v| v.artboard)}))
            {
                app.status(e.to_string());
            }
        } else if !ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|d| d.remove::<(Option<u64>, bool)>(drag_id));
        } else {
            ui.data_mut(|d| d.insert_temp(drag_id, (started_in, cancelled)));
            if !cancelled {
                preview = at;
            }
        }
    }
    preview
}

/// The key of the ruler origin drag's state in egui's temporary data.
fn ruler_origin_drag_id() -> egui::Id {
    egui::Id::unique("ruler-origin-drag")
}

/// Is a drag from the rulers' corner in progress? It stays true after Escape cancels the move, until
/// the button is released.
pub(crate) fn ruler_origin_dragging(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<(Option<u64>, bool)>(ruler_origin_drag_id()).is_some())
}

/// What the rulers count, and from where: `unit`, from `origin` (document points), with each
/// unit stretched `stretch` times along the document's x and y, which a video ruler's pixel aspect
/// ratio makes differ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerScale {
    pub(crate) unit: Unit,
    pub(crate) origin: Point,
    pub(crate) stretch: [f64; 2],
}

impl RulerScale {
    /// The rulers in `unit` (the General unit), from `origin` (the ruler origin).
    pub(crate) fn document(unit: Unit, origin: Point) -> Self {
        Self { unit, origin, stretch: [1.0, 1.0] }
    }

    /// View › Rulers › Show Video Rulers on artboard `ab`: pixels from its top-left corner, each
    /// pixel as many points wide as its Video Ruler Pixel Aspect Ratio and one point tall, so the
    /// rulers read the artboard's size in video pixels
    /// (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/set-video-display-options-for-artboards.html).
    pub(crate) fn video(ab: &vectorcraft_doc::Artboard) -> Self {
        Self { unit: Unit::Pixels, origin: Point::new(ab.rect.x0, ab.rect.y0), stretch: [ab.video_ruler_pixel_aspect_ratio, 1.0] }
    }

    /// How far document point `p` lies from the origin along `axis` (a unit direction in the
    /// document), in points of this scale: each document coordinate divided by its stretch.
    /// Divided by the unit's points, it is the value the ruler reads there.
    pub(crate) fn along(&self, p: Point, axis: vectorcraft_geom::Vec2) -> f64 {
        let d = p - self.origin;
        d.x / self.stretch[0] * axis.x + d.y / self.stretch[1] * axis.y
    }

    /// The screen points one point of this scale spans along `axis` at `zoom` (screen points per
    /// document point): `zoom` itself when nothing is stretched.
    pub(crate) fn zoom_along(&self, zoom: f64, axis: vectorcraft_geom::Vec2) -> f64 {
        zoom / (axis.x * axis.x / self.stretch[0] + axis.y * axis.y / self.stretch[1])
    }
}

pub(super) fn rulers(ui: &Ui, full: egui::Rect, xf: &Xf, hover: Option<Point>, scale: RulerScale, t: &Tokens) {
    let p = ui.painter();
    let [top, left, corner] = ruler_rects(full);
    for r in [top, left, corner] {
        p.rect_filled(r, 0.0, t.ruler);
    }
    p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, t.border));
    p.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.border));
    // Crosshair in the origin box.
    p.line_segment([corner.center() - vec2(4.0, 0.0), corner.center() + vec2(4.0, 0.0)], Stroke::new(1.0, t.ruler_tick));
    p.line_segment([corner.center() - vec2(0.0, 4.0), corner.center() + vec2(0.0, 4.0)], Stroke::new(1.0, t.ruler_tick));
    let font = egui::FontId::proportional(9.5);
    // Each ruler measures along its own edge, in the canvas's coordinates turned with the view
    // (#826): at 0° the top one reads x and the left one y. `draw` gets each tick's distance
    // (screen points) from `from` along the ruler, its length, and the label of a labelled one.
    let ticks = |from: Pos2, to: Pos2, axis: vectorcraft_geom::Vec2, draw: &mut dyn FnMut(f32, f32, Option<String>)| {
        // Pick a label step (in the unit) that gives ≥ 50 px between labels on this ruler, at its
        // own scale; positions below are in the unit (`per` points of the scale each).
        let zoom = scale.zoom_along(xf.zoom, axis);
        let (per, step) = (scale.unit.points(), scale.unit.ruler_step(zoom));
        let minor = scale.unit.ruler_minor(step);
        let near = |v: f64, every: f64| ((v / every).round() * every - v).abs() < minor * 0.01;
        let (a, b) = (scale.along(xf.to_doc(from), axis), scale.along(xf.to_doc(to), axis));
        let mut v = (a / per / minor).floor() * minor;
        while v * per <= b {
            // A tick's length: the labelled ones longest, the halves between them shorter.
            let (len, label) = if near(v, step) {
                (RULER, Some(ruler_label(v, step)))
            } else if near(v, step / 2.0) {
                (7.0, None)
            } else {
                (4.0, None)
            };
            draw(((v * per - a) * zoom) as f32, len, label);
            v += minor;
        }
    };
    // The canvas's directions along the screen's x and y.
    let (sn, cs) = xf.rot.sin_cos();
    let clip_top = p.with_clip_rect(top);
    ticks(top.left_top(), top.right_top(), vectorcraft_geom::Vec2::new(cs, -sn), &mut |d, len, label| {
        let sx = top.left() + d;
        clip_top.line_segment([pos2(sx, top.bottom() - len), pos2(sx, top.bottom())], Stroke::new(1.0, t.ruler_tick));
        if let Some(text) = label {
            clip_top.text(pos2(sx + 2.0, top.top() + 1.0), egui::Align2::LEFT_TOP, text, font.clone(), t.ruler_tick);
        }
    });
    let clip_left = p.with_clip_rect(left);
    // A label spelled down the ruler is tall, and a long one can reach the next label's place:
    // "-0.0625" is seven characters, 60 pt of ruler, where a sixteenth-inch step leaves 54 pt at
    // zoom 12. The ticks all stay; a label that would run into the one above it is left out.
    let mut filled_to = f32::NEG_INFINITY;
    ticks(left.left_top(), left.left_bottom(), vectorcraft_geom::Vec2::new(sn, cs), &mut |d, len, label| {
        let sy = left.top() + d;
        clip_left.line_segment([pos2(left.right() - len, sy), pos2(left.right(), sy)], Stroke::new(1.0, t.ruler_tick));
        if let Some(text) = label {
            // Vertical labels read top-to-bottom, one digit per line like Illustrator.
            let top = sy + 2.0;
            if top < filled_to {
                return;
            }
            filled_to = top + vertical_label_height(text.chars().count());
            for (k, ch) in text.chars().enumerate() {
                clip_left.text(
                    pos2(left.left() + 4.0, top + k as f32 * RULER_CHAR),
                    egui::Align2::LEFT_TOP,
                    ch.to_string(),
                    font.clone(),
                    t.ruler_tick,
                );
            }
        }
    });
    if let Some(h) = hover {
        let s = xf.to_screen(h);
        clip_top.line_segment([pos2(s.x, top.top()), pos2(s.x, top.bottom())], Stroke::new(1.0, t.text));
        clip_left.line_segment([pos2(left.left(), s.y), pos2(left.right(), s.y)], Stroke::new(1.0, t.text));
    }
}
