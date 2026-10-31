use super::drag::{PanelDrag, drag_source};
use super::{Color32, Paint, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, icons, pos2, vec2};

/// Draw a paint preview (swatch chip) into `rect`.
pub fn paint_chip(ui: &Ui, rect: Rect, paint: &Paint) {
    let p = ui.painter();
    match paint {
        Paint::None => {
            p.rect_filled(rect, 0.0, Color32::WHITE);
            p.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.6, Color32::from_rgb(0xe0, 0x20, 0x20)));
        }
        Paint::Solid { color, .. } => {
            let [r, g, b, _] = color.to_rgba8(1.0);
            p.rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
        }
        Paint::Gradient(gp) => gradient_chip(ui, rect, gp),
        Paint::Pattern { .. } => {
            p.rect_filled(rect, 0.0, Color32::from_gray(200));
            for i in 0..4 {
                let x = rect.left() + rect.width() * i as f32 / 4.0;
                p.line_segment([pos2(x, rect.bottom()), pos2(x + rect.width() / 4.0, rect.top())], Stroke::new(1.0, Color32::from_gray(90)));
            }
        }
    }
}

/// Draw a gradient preview into `rect`, laid out as the gradient would be on an object of that
/// shape: linear along its angle, radial from the centre, freeform from its colour points.
pub fn gradient_chip(ui: &Ui, rect: Rect, gradient: &vectorcraft_color::GradientPaint) {
    if !(rect.width() > 0.0 && rect.height() > 0.0) {
        return;
    }
    // Freeform sampling is costly per spot, so it gets a coarse grid (the mesh interpolates).
    let freeform = gradient.gradient.kind == vectorcraft_color::GradientKind::Freeform;
    let max = if freeform { 16 } else { 64 };
    let cols = ((rect.width() / 3.0).ceil() as usize).clamp(1, max);
    let rows = ((rect.height() / 3.0).ceil() as usize).clamp(1, max);
    let colors = gradient_chip_colors(gradient, rect.width(), rect.height(), cols, rows);
    let mut mesh = egui::Mesh::default();
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
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// The colours at the `(cols + 1) x (rows + 1)` grid points (row by row) of a `width` x `height`
/// chip showing `gp` as it would look on an object of that size.
pub(super) fn gradient_chip_colors(gp: &vectorcraft_color::GradientPaint, width: f32, height: f32, cols: usize, rows: usize) -> Vec<Color32> {
    use vectorcraft_color::{GradientGeom, GradientKind};
    let b = vectorcraft_geom::kurbo::Rect::new(0.0, 0.0, width as f64, height as f64);
    let rgb = |c: vectorcraft_color::Color| {
        let [r, g, b, _] = c.to_rgba8(1.0);
        Color32::from_rgb(r, g, b)
    };
    let at = |k: usize| {
        let (i, j) = (k % (cols + 1), k / (cols + 1));
        vectorcraft_geom::kurbo::Point::new(b.width() * i as f64 / cols as f64, b.height() * j as f64 / rows as f64)
    };
    let n = (cols + 1) * (rows + 1);
    if gp.gradient.kind == GradientKind::Freeform {
        let Some(painted) = vectorcraft_color::freeform::painted_box(b) else { return vec![Color32::TRANSPARENT; n] };
        let field = gp.freeform_on(painted).field(vectorcraft_color::freeform::spread_scale(painted));
        return (0..n)
            .map(|k| {
                let ([r, g, b], _) = field.sample(at(k));
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                Color32::from_rgb(q(r), q(g), q(b))
            })
            .collect();
    }
    // A placed gradient keeps its direction; a fresh one is fitted to the chip at its angle.
    let angle = gp.geom.map_or(gp.angle, |g| g.angle_deg());
    let geom = GradientGeom::fit(gp.gradient.kind, b, if angle.is_finite() { angle } else { 0.0 });
    (0..n)
        .map(|k| {
            let t = geom.param_at(gp.gradient.kind, at(k)).clamp(0.0, 1.0) as f32;
            rgb(gp.gradient.sample(t).0)
        })
        .collect()
}

/// A Fill chip of `paint` into `rect` or, with `ring`, a Stroke chip: a frame `ring` wide round a
/// panel-coloured hole (returned; the whole chip for fills). A selection whose paints differ
/// (`mixed`) shows a white chip with a "?" (in the hole for strokes).
pub fn proxy_chip(ui: &Ui, rect: Rect, paint: &Paint, mixed: bool, ring: Option<f32>) -> Rect {
    let t = Tokens::get(ui.ctx());
    if mixed {
        ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
    } else {
        paint_chip(ui, rect, paint);
    }
    let (hole, mark) = match ring {
        Some(w) => {
            let inner = rect.shrink(w);
            ui.painter().rect_filled(inner, 0.0, t.panel);
            (inner, t.text_strong)
        }
        None => (rect, Color32::BLACK),
    };
    if mixed {
        ui.painter().text(hole.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(hole.height() * 0.8), mark);
    }
    hole
}

/// A chip that opens something (the Control bar's and Properties' Fill/Stroke chips, the Control
/// bar's Style picker): `draw` paints the `size`-square chip, which gets a frame, and with
/// `chevron` a chevron follows it.
pub fn chip_button(ui: &mut Ui, size: f32, chevron: bool, draw: impl FnOnce(&Ui, Rect)) -> Response {
    chip_button_named(ui, size, chevron, "", draw)
}

/// [`chip_button`] with the tool tip `tip`, which screen readers also call it (none when empty).
pub fn chip_button_named(ui: &mut Ui, size: f32, chevron: bool, tip: &str, draw: impl FnOnce(&Ui, Rect)) -> Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(if chevron { size + 16.0 } else { size }, size), Sense::click());
    let chip = Rect::from_min_size(r.min, vec2(size, size)).shrink(2.0);
    draw(ui, chip);
    ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.input_border), StrokeKind::Outside);
    if chevron {
        icons::paint(ui, "chevron-down", Rect::from_center_size(pos2(chip.right() + 9.0, r.center().y), vec2(12.0, 12.0)), t.text_dim);
    }
    if tip.is_empty() {
        return resp;
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(tip)));
    resp.on_hover_text(tl!(tip))
}

/// What the user did on a [`fill_stroke_proxy`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxyClicks {
    pub fill: bool,
    pub stroke: bool,
    pub swap: bool,
    pub default: bool,
    /// Double-clicked a square: open the Color Picker for it (`true` = the stroke).
    pub pick: Option<bool>,
    /// A paint dragged from the other proxy (or a swatch) dropped on the fill square.
    pub dropped_fill: Option<serde_json::Value>,
    /// A paint dropped on the stroke square.
    pub dropped_stroke: Option<serde_json::Value>,
}

/// The Fill/Stroke proxy pair (two overlapping squares). `mixed` (fill, stroke) draws a "?" square
/// for a selection whose objects differ.
pub fn fill_stroke_proxy(ui: &mut Ui, fill: &Paint, stroke: &Paint, mixed: (bool, bool), fill_active: bool, size: f32) -> ProxyClicks {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let s = size * 0.62;
    let fill_r = Rect::from_min_size(rect.min + vec2(0.0, 0.0), Vec2::splat(s));
    let stroke_r = Rect::from_min_size(rect.max - Vec2::splat(s), Vec2::splat(s));
    let stroke_resp = ui.interact(stroke_r, ui.scope_id().with("stroke-proxy"), Sense::click_and_drag());
    let fill_resp = ui.interact(fill_r, ui.scope_id().with("fill-proxy"), Sense::click_and_drag());
    let dropped_fill = if mixed.0 { None } else { proxy_paint_drop(ui, &fill_resp) };
    let dropped_stroke = if mixed.1 { None } else { proxy_paint_drop(ui, &stroke_resp) };
    // A proxy drags its paint (onto the Swatches panel or art), unless it shows "?".
    for (resp, paint, mixed) in [(&fill_resp, fill, mixed.0), (&stroke_resp, stroke, mixed.1)] {
        if !mixed {
            drag_source(ui, resp, || PanelDrag::paint(paint.clone()));
        }
    }
    let draw_fill = |ui: &Ui| {
        proxy_chip(ui, fill_r, fill, mixed.0, None);
        ui.painter().rect_stroke(fill_r, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Inside);
        ui.painter().rect_stroke(fill_r.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Outside);
    };
    let draw_stroke = |ui: &Ui| {
        let inner = proxy_chip(ui, stroke_r, stroke, mixed.1, Some(s * 0.28));
        ui.painter().rect_stroke(stroke_r, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Inside);
        ui.painter().rect_stroke(inner, 0.0, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Outside);
        ui.painter().rect_stroke(stroke_r.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Outside);
    };
    if fill_active {
        draw_stroke(ui);
        draw_fill(ui);
    } else {
        draw_fill(ui);
        draw_stroke(ui);
    }
    // Swap arrow (top-right) and default (bottom-left) mini buttons.
    let swap_r = Rect::from_min_size(pos2(rect.right() - size * 0.3, rect.top()), Vec2::splat(size * 0.3));
    let def_r = Rect::from_min_size(pos2(rect.left(), rect.bottom() - size * 0.3), Vec2::splat(size * 0.3));
    let swap = ui.interact(swap_r, ui.scope_id().with("swap-proxy"), Sense::click());
    let def = ui.interact(def_r, ui.scope_id().with("default-proxy"), Sense::click());
    icons::paint(ui, "dc-swap", swap_r.shrink(1.0), if swap.hovered() { t.text } else { t.icon });
    let a = Rect::from_min_size(def_r.min + vec2(1.0, 1.0), Vec2::splat(size * 0.14));
    let b = Rect::from_min_size(def_r.min + vec2(size * 0.1, size * 0.1), Vec2::splat(size * 0.14));
    ui.painter().rect_filled(b, 0.0, Color32::BLACK);
    ui.painter().rect_stroke(b, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    ui.painter().rect_filled(a, 0.0, Color32::WHITE);
    ui.painter().rect_stroke(a, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
    let pick = if fill_resp.double_clicked() {
        Some(false)
    } else if stroke_resp.double_clicked() {
        Some(true)
    } else {
        None
    };
    let on = ui.is_enabled();
    fill_resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, on, fill_active, tl!("Fill (X), double-click for the Color Picker")));
    stroke_resp
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, on, !fill_active, tl!("Stroke (X), double-click for the Color Picker")));
    swap.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tl!("Swap Fill and Stroke (Shift+X)")));
    def.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tl!("Default Fill and Stroke (D)")));
    ProxyClicks {
        fill: fill_resp.on_hover_text(tl!("Fill (X), double-click for the Color Picker")).clicked(),
        stroke: stroke_resp.on_hover_text(tl!("Stroke (X), double-click for the Color Picker")).clicked(),
        swap: swap.on_hover_text(tl!("Swap Fill and Stroke (Shift+X)")).clicked(),
        default: def.on_hover_text(tl!("Default Fill and Stroke (D)")).clicked(),
        pick,
        dropped_fill,
        dropped_stroke,
    }
}

/// A [`PanelDrag::Paint`] released on a Fill/Stroke proxy square: highlight while held over it.
fn proxy_paint_drop(ui: &Ui, resp: &Response) -> Option<serde_json::Value> {
    if let Some(drag) = resp.dnd_hover_payload::<PanelDrag>()
        && let PanelDrag::Paint { params, .. } = &*drag
        && !params.is_null()
    {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_stroke(resp.rect, 0.0, Stroke::new(2.0, t.accent), StrokeKind::Outside);
    }
    resp.dnd_release_payload::<PanelDrag>().and_then(|d| {
        let PanelDrag::Paint { params, .. } = &*d else { return None };
        (!params.is_null()).then_some(params.clone())
    })
}

/// One swatch tile (Illustrator: 1 px dark frame, white inset on hover/selection).
pub fn swatch_tile(ui: &Ui, rect: Rect, paint: &Paint, selected: bool, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    paint_chip(ui, rect, paint);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    if selected || hovered {
        ui.painter().rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, if selected { t.accent } else { t.text }), StrokeKind::Outside);
    }
}
