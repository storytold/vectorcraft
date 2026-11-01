use super::{
    Color, Color32, Entry, GradientKind, Paint, Rect, Response, Sense, Shape, Stroke, StrokeKind, Tokens, Ui, VectorcraftApp, View, icons, pos2, vec2,
};

/// A pattern swatch drawn as a rendered tile (cached by the definition's identity and size).
pub(super) fn pattern_thumb(app: &VectorcraftApp, ui: &Ui, r: Rect, paint: &Paint) {
    use std::collections::HashMap;
    type Key = (String, Vec<usize>, String, u32);
    thread_local! {
        static CACHE: crate::graphics::TexCache<HashMap<Key, Option<egui::TextureHandle>>> = crate::graphics::TexCache::default();
    }
    let Paint::Pattern { pattern, .. } = paint else { return };
    let Some(st) = app.session.active() else { return };
    let Some(def) = st.doc.pattern(pattern) else { return };
    let px = (r.width() * ui.ctx().pixels_per_point()).round().max(4.0) as u32;
    let key = (
        pattern.clone(),
        def.art.iter().map(|a| std::sync::Arc::as_ptr(a) as usize).collect(),
        format!("{:?}{:?}{:?}", def.tile, def.tile_type, def.overlap),
        px,
    );
    let tex = CACHE.with(|c| c.borrow().get(&key).cloned()).unwrap_or_else(|| {
        let tex = vectorcraft_render::render_pattern_swatch(&st.doc, pattern, px).map(|img| {
            let color = crate::canvas::color_image(&img);
            ui.ctx().load_texture(format!("pattern-swatch-{pattern}-{px}"), color, egui::TextureOptions::LINEAR)
        });
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 256 {
                c.clear();
            }
            c.insert(key, tex.clone());
        });
        tex
    });
    if let Some(tex) = tex {
        ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
        ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

pub(super) fn draw_registration(ui: &Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    let c = r.center();
    let rad = r.width() * 0.28;
    let s = Stroke::new(1.0, egui::Color32::BLACK);
    ui.painter().circle_stroke(c, rad, s);
    ui.painter().line_segment([pos2(r.left() + 2.0, c.y), pos2(r.right() - 2.0, c.y)], s);
    ui.painter().line_segment([pos2(c.x, r.top() + 2.0), pos2(c.x, r.bottom() - 2.0)], s);
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
}

pub(crate) fn draw_folder(ui: &Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    icons::paint(ui, "dc-folder", r.expand(1.0), t.icon);
}

/// The ink colours of the CMYK glyph (cyan, magenta, yellow, black) and the RGB glyph's bars.
const CMYK_INKS: [Color32; 4] = [Color32::from_rgb(0, 174, 239), Color32::from_rgb(236, 0, 140), Color32::from_rgb(255, 242, 0), Color32::BLACK];

const RGB_BARS: [Color32; 3] = [Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 200, 0), Color32::from_rgb(0, 0, 255)];

/// Lab's opponent axes: green | red (a) over blue | yellow (b).
const LAB_AXES: [Color32; 4] =
    [Color32::from_rgb(0, 166, 81), Color32::from_rgb(230, 30, 60), Color32::from_rgb(0, 90, 220), Color32::from_rgb(250, 205, 0)];

/// A swatch's kind in words, for list tooltips ("Global Process Color, CMYK").
pub(crate) fn describe(paint: &Paint, global: bool, spot: bool) -> String {
    match paint {
        Paint::Solid { color, .. } => {
            let kind = if spot {
                tl!("Spot Color")
            } else if global {
                tl!("Global Process Color")
            } else {
                tl!("Process Color")
            };
            crate::i18n::fmt(tl!("{kind}, {model}"), &[("kind", kind), ("model", color.model_name())])
        }
        Paint::Gradient(g) => crate::i18n::fmt(
            tl!("{kind} Gradient"),
            &[("kind", crate::i18n::tr_ctx(crate::i18n::current(), "gradient-kind", g.gradient.kind.label()))],
        ),
        Paint::Pattern { .. } => tl!("Pattern").into(),
        Paint::None => tl!("None").into(),
    }
}

/// The kind and colour-mode icons at the right of list row `r`: spot (a dot in a ring), global (a
/// square with a filled corner) or process colours (a square), gradients and patterns; then a
/// colour's model (CMYK as four ink quarters, RGB as three bars, Gray as a grey square, Lab as its
/// two opponent axes).
pub(crate) fn list_icons(ui: &Ui, r: Rect, paint: &Paint, global: bool, spot: bool) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let mode = Rect::from_center_size(r.right_center() - vec2(12.0, 0.0), vec2(10.0, 10.0));
    let kind = mode.translate(vec2(-16.0, 0.0));
    let line = Stroke::new(1.0, t.icon);
    match paint {
        Paint::Solid { color, .. } => {
            if spot {
                p.circle_stroke(kind.center(), 4.5, line);
                p.circle_filled(kind.center(), 1.8, t.icon);
            } else {
                p.rect_stroke(kind, 0.0, line, StrokeKind::Inside);
                if global {
                    let c = kind.right_bottom();
                    p.add(Shape::convex_polygon(vec![c, c - vec2(6.0, 0.0), c - vec2(0.0, 6.0)], t.icon, Stroke::NONE));
                }
            }
            let quarters = |colors: [Color32; 4]| {
                let q = mode.size() / 2.0;
                for (i, c) in colors.into_iter().enumerate() {
                    p.rect_filled(Rect::from_min_size(mode.min + vec2((i % 2) as f32 * q.x, (i / 2) as f32 * q.y), q), 0.0, c);
                }
            };
            match color {
                Color::Cmyk { .. } => quarters(CMYK_INKS),
                Color::Lab { .. } => quarters(LAB_AXES),
                Color::Rgb { .. } => {
                    let w = mode.width() / 3.0;
                    for (i, bar) in RGB_BARS.into_iter().enumerate() {
                        p.rect_filled(Rect::from_min_size(mode.min + vec2(i as f32 * w, 0.0), vec2(w, mode.height())), 0.0, bar);
                    }
                }
                Color::Gray { .. } => {
                    p.rect_filled(mode, 0.0, Color32::from_gray(128));
                }
            }
            p.rect_stroke(mode, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
        }
        Paint::Gradient(g) => {
            let icon = match g.gradient.kind {
                GradientKind::Linear => "dc-grad-linear",
                GradientKind::Radial => "dc-grad-radial",
                GradientKind::Freeform => "dc-grad-freeform",
            };
            icons::paint(ui, icon, kind.expand(1.0), t.icon);
        }
        Paint::Pattern { .. } => icons::paint(ui, "grid-3x3", kind.expand(1.0), t.icon),
        Paint::None => {}
    }
}

/// The thumbnail cells of `items` for `view` in the full available width, row by row; a folder
/// (`is_folder`) starts a new row.
pub(crate) fn tile_grid<'a, T>(ui: &mut Ui, items: &'a [T], view: View, is_folder: impl Fn(&T) -> bool) -> Vec<(&'a T, Rect)> {
    let (tile, pitch) = view.tile();
    let w = ui.available_width();
    let per_row = ((w - 2.0) / pitch).floor().max(1.0) as usize;
    let mut rows: Vec<Vec<&T>> = vec![vec![]];
    for e in items {
        match rows.last_mut() {
            Some(row) if row.len() < per_row && (row.is_empty() || !is_folder(e)) => row.push(e),
            _ => rows.push(vec![e]),
        }
    }
    let mut out = Vec::with_capacity(items.len());
    for row in rows {
        let (r, _) = ui.allocate_exact_size(vec2(w, pitch), Sense::hover());
        let cell = |c: usize| Rect::from_min_size(r.min + vec2(1.0 + c as f32 * pitch, (pitch - tile) / 2.0), vec2(tile, tile));
        out.extend(row.into_iter().enumerate().map(|(c, e)| (e, cell(c))));
    }
    out
}

/// A list-view row for `view` taking clicks and drags as `id`, shaded when `selected` or hovered
/// → (the row, its response, the chip at its left).
pub(crate) fn list_row(ui: &mut Ui, id: egui::Id, view: View, selected: bool) -> (Rect, Response, Rect) {
    let t = Tokens::get(ui.ctx());
    let (tile, pitch) = view.tile();
    let (_, r) = ui.allocate_space(vec2(ui.available_width(), pitch));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    if selected {
        ui.painter().rect_filled(r, 0.0, t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover);
    }
    (r, resp, Rect::from_min_size(r.left_center() + vec2(4.0, -tile / 2.0), vec2(tile, tile)))
}

/// A list row's name, right of its `chip`.
pub(crate) fn list_row_name(ui: &Ui, r: Rect, chip: Rect, name: &str) {
    let t = Tokens::get(ui.ctx());
    ui.painter().text(pos2(chip.right() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), t.text);
}

/// The white corner of a global colour's tile; a spot colour has a dot in it.
pub(crate) fn global_mark(ui: &Ui, cell: Rect, spot: bool) {
    let k = cell.shrink(1.0).right_bottom();
    let corner = vec![k, k - vec2(5.0, 0.0), k - vec2(0.0, 5.0)];
    ui.painter().add(Shape::convex_polygon(corner, Color32::WHITE, Stroke::NONE));
    if spot {
        ui.painter().circle_filled(k - vec2(1.6, 1.6), 1.0, Color32::BLACK);
    }
}

/// A colour group's folder in the thumbnail view, outlined when `selected`.
pub(crate) fn folder_tile(ui: &Ui, cell: Rect, selected: bool) {
    draw_folder(ui, cell);
    if selected {
        ui.painter().rect_stroke(cell.expand(1.0), 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Outside);
    }
}

/// The interaction id of a tile or list row (stable per swatch or group name and host `salt`).
pub(super) fn tile_id(salt: &'static str, e: &Entry) -> egui::Id {
    egui::Id::unique((salt, e.name()))
}
