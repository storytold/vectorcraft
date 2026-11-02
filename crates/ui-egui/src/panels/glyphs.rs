//! Glyphs panel: every character the current font maps (its cmap), drawn from the font's own
//! outlines. Double-click inserts the character at the Type tool's caret (or appends it to the
//! selected text objects). The character selected with the Type tool is highlighted in the grid.

use std::collections::HashMap;

use egui::{Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{Affine, BezPath, PathEl};
use vectorcraft_text::FontDb;

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

/// Preview cell sizes for the glyph grid, smallest to largest. The third step (30 pt) is the size
/// the grid used before Zoom Out and Zoom In. Public behaviour: [Glyphs panel overview](https://helpx.adobe.com/illustrator/desktop/design-with-text/special-characters-glyphs/glyphs-panel-overview.html)
/// (Zoom Out reduces the preview, Zoom In increases it). The step sizes themselves are ours.
const CELL_STEPS: [f32; 7] = [16.0, 22.0, 30.0, 42.0, 56.0, 72.0, 96.0];
const CELL_DEFAULT: usize = 2;

/// The grid step in [`crate::state::UiState::glyphs_preview`]. `0` means [`CELL_DEFAULT`]; a
/// stored step is `index + 1`. A value past the steps (a hand-edited preferences file) is the
/// default too.
fn cell_index(app: &VectorcraftApp) -> usize {
    match usize::from(app.ui.glyphs_preview) {
        0 => CELL_DEFAULT,
        n if n <= CELL_STEPS.len() => n - 1,
        _ => CELL_DEFAULT,
    }
}

fn set_cell_index(app: &mut VectorcraftApp, index: usize) {
    let index = index.min(CELL_STEPS.len() - 1);
    // `index + 1` is at most `CELL_STEPS.len()` (7), which fits in a `u8`.
    app.ui.glyphs_preview = (index + 1) as u8;
}

/// One step toward a smaller (`-1`) or larger (`1`) preview, staying inside [`CELL_STEPS`].
fn nudge_cell(index: usize, delta: i32) -> usize {
    let last = CELL_STEPS.len() as i32 - 1;
    (index as i32 + delta).clamp(0, last) as usize
}

/// Character subsets offered by the Show menu.
const SUBSETS: [(&str, u32, u32); 7] = [
    ("Entire Font", 0, 0x10FFFF),
    ("Basic Latin", 0x20, 0x7F),
    ("Latin-1 Supplement", 0xA0, 0xFF),
    ("Latin Extended", 0x100, 0x24F),
    ("Punctuation", 0x2000, 0x206F),
    ("Currency & Symbols", 0x20A0, 0x2BFF),
    ("Greek & Cyrillic", 0x370, 0x52F),
];

thread_local! {
    static TEX: crate::graphics::TexCache<HashMap<(u32, u32, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
}

/// Antialiased coverage mask (nonzero winding) of `path` in a `w`×`h` pixel grid.
pub(crate) fn rasterize(path: &BezPath, w: usize, h: usize) -> Vec<u8> {
    let mut edges: Vec<(f64, f64, f64, f64)> = vec![];
    let (mut start, mut last) = (None, (0.0, 0.0));
    vectorcraft_geom::kurbo::flatten(path, 0.1, |el| match el {
        PathEl::MoveTo(p) => {
            if let Some(s) = start.replace((p.x, p.y)) {
                edges.push((last.0, last.1, s.0, s.1));
            }
            last = (p.x, p.y);
        }
        PathEl::LineTo(p) => {
            edges.push((last.0, last.1, p.x, p.y));
            last = (p.x, p.y);
        }
        PathEl::ClosePath => {
            if let Some(s) = start.take() {
                edges.push((last.0, last.1, s.0, s.1));
                last = s;
            }
        }
        _ => {}
    });
    if let Some(s) = start {
        edges.push((last.0, last.1, s.0, s.1));
    }
    const SUB: usize = 4;
    let mut acc = vec![0f32; w * h];
    let mut xs: Vec<(f64, i32)> = vec![];
    for row in 0..h {
        for sy in 0..SUB {
            let y = row as f64 + (sy as f64 + 0.5) / SUB as f64;
            xs.clear();
            for &(x0, y0, x1, y1) in &edges {
                if (y0 <= y) != (y1 <= y) {
                    xs.push((x0 + (y - y0) / (y1 - y0) * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for k in 0..xs.len() {
                wind += xs[k].1;
                if wind == 0 || k + 1 >= xs.len() {
                    continue;
                }
                let (a, b) = (xs[k].0.max(0.0), xs[k + 1].0.min(w as f64));
                if b <= a {
                    continue;
                }
                for px in (a.floor() as usize)..(b.ceil() as usize).min(w) {
                    let cov = (b.min(px as f64 + 1.0) - a.max(px as f64)).max(0.0);
                    acc[row * w + px] += (cov / SUB as f64) as f32;
                }
            }
        }
    }
    acc.into_iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

fn glyph_texture(ctx: &egui::Context, face: &vectorcraft_text::FontFace, gid: u32, px: u32) -> egui::TextureHandle {
    let key = (face.id(), gid, px);
    if let Some(t) = TEX.with(|c| c.borrow().get(&key).cloned()) {
        return t;
    }
    let (asc, desc) = face.vertical_metrics();
    let em = (asc + desc).max(1.0);
    let s = px as f64 * 0.8 / em;
    let adv = face.advance(gid);
    let mut path = (*FontDb::global().outline(face, gid)).clone();
    // Font units (y down, baseline at 0) → pixels, centred on the advance.
    let ox = (px as f64 - adv * s) * 0.5;
    let oy = px as f64 * 0.1 + asc * s;
    path.apply_affine(Affine::translate((ox, oy)) * Affine::scale(s));
    let mask = rasterize(&path, px as usize, px as usize);
    let img = egui::ColorImage::new([px as usize, px as usize], mask.iter().map(|&a| Color32::from_rgba_premultiplied(a, a, a, a)).collect());
    let tex = ctx.load_texture(format!("glyph-{}-{gid}-{px}", face.id()), img, egui::TextureOptions::LINEAR);
    TEX.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4000 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    tex
}

/// The character the grid highlights: the first one of the Type tool's selected range, else the
/// one before its caret. Public behaviour: the Glyphs panel shows the selected character's glyph
/// (the overview page cited at [`CELL_STEPS`]). Walks the runs, not the characters, so it stays
/// cheap on every frame.
fn selected_char(app: &VectorcraftApp) -> Option<char> {
    let (id, a, b) = super::character::text_editing(app)?;
    let NodeKind::Text(t) = &app.session.active()?.doc.node(id)?.kind else { return None };
    let (at, range) = (a.min(b), a != b);
    let mut start = 0;
    for run in &t.runs {
        let end = start + run.text.len();
        if range && (start..end).contains(&at) {
            return run.text.get(at - start..)?.chars().next();
        }
        if !range && at > start && at <= end {
            return run.text.get(..at - start)?.chars().next_back();
        }
        start = end;
    }
    None
}

/// Insert `ch` at the Type tool's caret, else append it to the selected text.
fn insert(app: &mut VectorcraftApp, ctx: &egui::Context, ch: char) {
    let s = ch.to_string();
    if app.session.tool_wants_text() {
        let v = app.view_info();
        let target_before = crate::ime::target(&mut app.session, v);
        let before = crate::ime::stamp(&mut app.session, v);
        let r = app.session.tool_text(&s, v);
        match &r {
            Err(e) => app.status(e.to_string()),
            Ok(_) => crate::ime::note_application_edit(app, ctx, v, target_before, before),
        }
        app.sync_views();
    } else if super::character::text_style(app).is_some() {
        app.run("type.insert", json!({"text": s})).ok();
    } else {
        app.ui.status = "Select text or click into text with the Type tool to insert glyphs".into();
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let db = FontDb::global();
    let current = super::character::text_style(app).map(|(s, _)| (s.font_family, s.font_style));
    // The panel browses the current text's font unless another font was picked here.
    let picked: Option<(String, String)> = pstate(ui.ctx(), "gl-font");
    let (family, style) = picked.or(current).unwrap_or_else(|| (vectorcraft_text::FALLBACK_FAMILY.to_string(), "Regular".to_string()));
    let Some(face) = db.face(&family, &style) else {
        ui.label(egui::RichText::new(tl!("No fonts are available.")).color(t.text_dim));
        return;
    };
    let subset: usize = pstate(ui.ctx(), "gl-subset");
    let (_, lo, hi) = SUBSETS[subset.min(SUBSETS.len() - 1)];
    let w = ui.available_width();
    let names: Vec<&str> = SUBSETS.iter().map(|s| s.0).collect();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Show:")).color(t.text_dim));
        if let Some(i) = widgets::dropdown(ui, "gl-subset", names[subset.min(names.len() - 1)], &names, (w - 50.0).max(80.0)) {
            set_pstate(ui.ctx(), "gl-subset", i);
        }
    });
    let chars: Vec<(char, u32)> =
        face.chars().into_iter().filter(|(c, _)| (lo..=hi).contains(&(*c as u32)) && !c.is_control() && !c.is_whitespace()).collect();
    let cell = CELL_STEPS[cell_index(app)];
    let cols = ((w / cell).floor() as usize).max(1);
    let rows = chars.len().div_ceil(cols);
    let px = (cell * ui.ctx().pixels_per_point()).round().clamp(8.0, 256.0) as u32;
    let list_h = (ui.available_height() - 64.0).max(cell * 3.0);
    let hover_id = egui::Id::unique("gl-hover");
    let selected = selected_char(app);
    // The selected character and the rows shown last frame: the grid scrolls to the selected glyph
    // only when the selection moves to another character, so it still scrolls freely meanwhile.
    let follow_id = egui::Id::unique("gl-follow");
    let (followed, shown): (Option<char>, std::ops::Range<usize>) = ui.ctx().data(|d| d.get_temp(follow_id)).unwrap_or_default();
    let mut area = egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]);
    if selected != followed
        && let Some(c) = selected
        && let Ok(i) = chars.binary_search_by_key(&c, |x| x.0)
        && !shown.contains(&(i / cols))
    {
        let row_h = cell + ui.spacing().item_spacing.y;
        area = area.vertical_scroll_offset(((i / cols) as f32 + 0.5) * row_h - list_h * 0.5);
    }
    let shown = area.show_rows(ui, cell, rows, |ui, range| {
        for r in range.clone() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for &(c, gid) in chars.iter().skip(r * cols).take(cols) {
                    let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
                    let chosen = selected == Some(c);
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, t.cr(), t.hover);
                        ui.ctx().data_mut(|d| d.insert_temp(hover_id, (c, gid)));
                    } else if chosen {
                        ui.painter().rect_filled(rect, 2, t.row_selected);
                    }
                    let edge = if chosen { Stroke::new(1.5, t.accent) } else { Stroke::new(0.5, t.border) };
                    ui.painter().rect_stroke(rect.shrink(0.5), 0, edge, StrokeKind::Inside);
                    let tex = glyph_texture(ui.ctx(), &face, gid, px);
                    ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), t.text_strong);
                    let resp = resp.on_hover_text(format!("U+{:04X}  {c}  (glyph {gid})\n{}", c as u32, tl!("Double-click to insert")));
                    if resp.double_clicked() {
                        insert(app, ui.ctx(), c);
                    }
                }
            });
        }
        range
    });
    ui.ctx().data_mut(|d| d.insert_temp(follow_id, (selected, shown.inner)));
    ui.add_space(4.0);
    let info = ui.ctx().data(|d| d.get_temp::<(char, u32)>(hover_id));
    ui.label(
        egui::RichText::new(match info {
            Some((c, g)) => format!(
                "U+{:04X}  {}  ·  {}",
                c as u32,
                crate::i18n::fmt(tl!("glyph {id}"), &[("id", &g.to_string())]),
                crate::i18n::tn(chars.len() as u64, "{n} glyph", "{n} glyphs")
            ),
            None => crate::i18n::tn(chars.len() as u64, "{n} glyph", "{n} glyphs"),
        })
        .size(11.0)
        .color(t.text_dim),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        // Picks the font the panel browses only: nothing is previewed on the document.
        // Zoom Out and Zoom In sit to the right of the family and style menus.
        let btn = 22.0;
        let menus = (w - btn * 2.0 - 12.0).max(88.0);
        let family_w = (menus * 0.62).clamp(48.0, (menus - 40.0).max(48.0));
        let style_w = (menus - family_w).max(40.0);
        let sample = crate::font_menu::sample_text(app);
        let look = crate::font_menu::MenuLook::of(app);
        let pick = crate::font_menu::font_menu(ui, "gl-family", &family, family_w, sample.as_deref(), look);
        if let Some((f, style)) = crate::font_menu::picked(app, pick) {
            let st = style.unwrap_or_else(|| db.face(&f, "Regular").map_or_else(|| "Regular".into(), |face| face.style.clone()));
            set_pstate(ui.ctx(), "gl-font", Some((f, st)));
        }
        let styles = db.styles(&family);
        let snames: Vec<&str> = styles.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown_names(ui, "gl-style", &style, &snames, style_w)
            && let Some(name) = snames.get(i)
        {
            set_pstate(ui.ctx(), "gl-font", Some((family.clone(), (*name).to_string())));
        }
        let step = cell_index(app);
        if zoom_button(ui, "out", "−", "Zoom Out", step > 0) {
            set_cell_index(app, nudge_cell(step, -1));
        }
        if zoom_button(ui, "in", "+", "Zoom In", step + 1 < CELL_STEPS.len()) {
            set_cell_index(app, nudge_cell(step, 1));
        }
    });
}

/// A square [ − ] or [ + ] beside the font menus. `label` is the mark itself (not translated).
fn zoom_button(ui: &mut Ui, id: &str, label: &str, tip: &str, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, ui.scope_id().with(("gl-zoom", id)), sense);
    let bg = if enabled && resp.hovered() { t.hover } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, t.cr(), bg);
    let border = if enabled { t.button_border } else { t.border };
    ui.painter().rect_stroke(rect.shrink(0.5), t.cr(), Stroke::new(1.0, border), StrokeKind::Inside);
    let color = if enabled { t.text_strong } else { t.text_disabled };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(14.0), color);
    resp.on_hover_text(tl!(tip)).clicked() && enabled
}

pub fn menu(_app: &mut VectorcraftApp, ui: &mut Ui) {
    if menu_item(ui, tl!("Use the Selected Text's Font"), true, false) {
        set_pstate::<Option<(String, String)>>(ui.ctx(), "gl-font", None);
    }
    ui.separator();
    for (i, (name, _, _)) in SUBSETS.iter().enumerate() {
        let cur: usize = pstate(ui.ctx(), "gl-subset");
        if menu_item(ui, tl!(name), true, cur == i) {
            set_pstate(ui.ctx(), "gl-subset", i);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, Shape};

    use super::*;

    fn shapes(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<Shape> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 420.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> (Vec<(String, Pos2)>, Vec<f32>) {
        let shapes = shapes(ctx, app, events);
        (text_anchors(&shapes), square_sides(&shapes))
    }

    /// The Type tool editing `text` in a new document.
    fn editing(text: &str) -> (VectorcraftApp, serde_json::Value) {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 50, "text": text})).unwrap()["id"].clone();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        (app, id)
    }

    fn select(app: &mut VectorcraftApp, start: usize, end: usize) {
        app.session.set_tool_option("select", &json!({"start": start, "end": end}));
    }

    /// Centres of the highlighted (selected-character) cells.
    fn highlighted(shapes: &[Shape], t: &Tokens) -> Vec<Pos2> {
        fn walk(s: &Shape, fill: Color32, out: &mut Vec<Pos2>) {
            match s {
                Shape::Rect(r) if r.fill == fill => out.push(r.rect.center()),
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, fill, out)),
                _ => {}
            }
        }
        let mut out = vec![];
        shapes.iter().for_each(|s| walk(s, t.row_selected, &mut out));
        out
    }

    fn text_anchors(shapes: &[Shape]) -> Vec<(String, Pos2)> {
        fn walk(s: &Shape, out: &mut Vec<(String, Pos2)>) {
            match s {
                Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos)),
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = vec![];
        shapes.iter().for_each(|s| walk(s, &mut out));
        out
    }

    /// Sides of the square glyph previews (font-menu icons are smaller).
    fn square_sides(shapes: &[Shape]) -> Vec<f32> {
        fn walk(s: &Shape, out: &mut Vec<f32>) {
            match s {
                Shape::Mesh(m) => {
                    let r = m.calc_bounds();
                    if (r.width() - r.height()).abs() < 1.0 && r.width() >= 14.0 {
                        out.push(r.width());
                    }
                }
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = vec![];
        shapes.iter().for_each(|s| walk(s, &mut out));
        out
    }

    fn mark(texts: &[(String, Pos2)], label: &str) -> Pos2 {
        texts
            .iter()
            .filter(|(t, _)| t == label)
            .map(|(_, p)| *p)
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap_or_else(|| panic!("{label} missing in {texts:?}"))
    }

    fn click(ctx: &egui::Context, app: &mut VectorcraftApp, at: Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, app, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(ctx, app, vec![button(false)]);
    }

    #[test]
    fn the_selected_character_is_the_range_start_or_the_one_before_the_caret() {
        let (mut app, id) = editing("aHé");
        select(&mut app, 1, 2);
        assert_eq!(selected_char(&app), Some('H'));
        select(&mut app, 2, 2);
        assert_eq!(selected_char(&app), Some('H'));
        select(&mut app, 4, 1);
        assert_eq!(selected_char(&app), Some('H'), "a backward range starts at its lower end");
        select(&mut app, 0, 0);
        assert_eq!(selected_char(&app), None, "nothing before the caret");
        // Across runs: a style on `H` alone splits the text into three runs.
        app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "kerning": 50})).unwrap();
        select(&mut app, 1, 1);
        assert_eq!(selected_char(&app), Some('a'));
        select(&mut app, 2, 2);
        assert_eq!(selected_char(&app), Some('H'));
        select(&mut app, 2, 4);
        assert_eq!(selected_char(&app), Some('é'));
        app.session.select_tool("select", Default::default()).unwrap();
        assert_eq!(selected_char(&app), None, "only the Type tool's selection counts");
    }

    #[test]
    fn the_selected_glyph_is_highlighted_and_scrolled_into_view() {
        let (mut app, _) = editing("A");
        let ctx = egui::Context::default();
        let t = Tokens::get(&ctx);
        let (texts, _) = frame(&ctx, &mut app, vec![]);
        if texts.iter().any(|(t, _)| t.contains("No fonts are available")) {
            eprintln!("glyphs highlight: no fonts installed, grid not drawn");
            return;
        }
        let (family, style) = super::super::character::text_style(&app).map(|(s, _)| (s.font_family, s.font_style)).unwrap();
        let face = FontDb::global().face(&family, &style).unwrap();
        let chars: Vec<char> = face.chars().into_iter().map(|(c, _)| c).filter(|c| !c.is_control() && !c.is_whitespace()).collect();
        let (Some(&first), Some(&last)) = (chars.first(), chars.last()) else { return };
        if chars.len() < 200 {
            eprintln!("glyphs highlight: font too small to need scrolling");
            return;
        }
        let (mut app, _) = editing(&format!("{first}{last}"));
        let end = first.len_utf8() + last.len_utf8();
        select(&mut app, end, end);
        let ctx = egui::Context::default();
        // The caret sits after the last glyph of the font, far below the first screen of the grid.
        let lit = highlighted(&shapes(&ctx, &mut app, vec![]), &t);
        assert_eq!(lit.len(), 1, "the grid scrolled to the selected glyph and highlighted it: {lit:?}");
        let lit = highlighted(&shapes(&ctx, &mut app, vec![]), &t);
        assert_eq!(lit.len(), 1, "it stays highlighted");
        // Highlighting the first character scrolls back up to it.
        select(&mut app, 0, 1);
        shapes(&ctx, &mut app, vec![]);
        let top = highlighted(&shapes(&ctx, &mut app, vec![]), &t);
        assert_eq!(top.len(), 1, "{top:?}");
        assert!(top[0].y < lit[0].y, "the first glyph sits at the top of the grid: {top:?} vs {lit:?}");
        // The caret at the start selects nothing.
        select(&mut app, 0, 0);
        assert!(highlighted(&shapes(&ctx, &mut app, vec![]), &t).is_empty());
    }

    #[test]
    fn zoom_stops_at_the_ends() {
        assert_eq!(nudge_cell(CELL_DEFAULT, 1), CELL_DEFAULT + 1);
        assert_eq!(nudge_cell(CELL_DEFAULT, -1), CELL_DEFAULT - 1);
        assert_eq!(nudge_cell(0, -1), 0);
        assert_eq!(nudge_cell(CELL_STEPS.len() - 1, 1), CELL_STEPS.len() - 1);
        assert_eq!(CELL_STEPS[CELL_DEFAULT], 30.0);
    }

    #[test]
    fn zoom_buttons_resize_the_glyph_grid() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let (texts, sides) = frame(&ctx, &mut app, vec![]);
        if texts.iter().any(|(t, _)| t.contains("No fonts are available")) {
            eprintln!("glyphs zoom: no fonts installed, grid not drawn");
            return;
        }
        let (minus, plus) = (mark(&texts, "−"), mark(&texts, "+"));
        assert!(minus.x + 8.0 < plus.x, "− then + on the right: {minus:?} {plus:?}");
        let family_x = texts.iter().map(|(_, p)| p.x).fold(f32::MAX, f32::min);
        assert!(minus.x > family_x + 40.0, "the buttons sit to the right of the font menus");
        assert!(sides.iter().all(|s| (*s - 30.0).abs() < 1.0), "default cells are 30 pt: {sides:?}");
        assert!(!sides.is_empty(), "the grid drew glyphs");
        click(&ctx, &mut app, plus);
        let (_texts, sides) = frame(&ctx, &mut app, vec![]);
        assert_eq!(cell_index(&app), CELL_DEFAULT + 1);
        assert!(sides.iter().all(|s| (*s - 42.0).abs() < 1.0), "Zoom In grows the cells: {sides:?}");
        click(&ctx, &mut app, minus);
        let (_texts, sides) = frame(&ctx, &mut app, vec![]);
        assert_eq!(cell_index(&app), CELL_DEFAULT);
        assert!(sides.iter().all(|s| (*s - 30.0).abs() < 1.0), "Zoom Out restores 30 pt: {sides:?}");
        // Hold Zoom Out until the smallest step, then once more: it stays there.
        for _ in 0..CELL_STEPS.len() {
            let (texts, _) = frame(&ctx, &mut app, vec![]);
            click(&ctx, &mut app, mark(&texts, "−"));
        }
        assert_eq!(cell_index(&app), 0);
        let (texts, sides) = frame(&ctx, &mut app, vec![]);
        assert!(sides.iter().all(|s| (*s - CELL_STEPS[0]).abs() < 1.0), "{sides:?}");
        click(&ctx, &mut app, mark(&texts, "−"));
        assert_eq!(cell_index(&app), 0);
    }

    #[test]
    fn the_preview_step_is_kept_in_the_ui_preferences() {
        let mut ui = crate::state::UiState::default();
        assert_eq!(ui.glyphs_preview, 0);
        assert!(serde_json::to_value(&ui).unwrap().get("glyphs_preview").is_none(), "the default adds nothing to an older preferences file");
        ui.glyphs_preview = (CELL_DEFAULT + 2) as u8;
        let saved = serde_json::to_value(&ui).unwrap();
        assert_eq!(saved["glyphs_preview"], serde_json::json!(CELL_DEFAULT + 2));
        let back: crate::state::UiState = serde_json::from_value(saved).unwrap();
        assert_eq!(back.glyphs_preview, (CELL_DEFAULT + 2) as u8);
        let old: crate::state::UiState = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(old.glyphs_preview, 0);
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.ui.glyphs_preview = back.glyphs_preview;
        assert_eq!(cell_index(&app), CELL_DEFAULT + 1);
        app.ui.glyphs_preview = 99;
        assert_eq!(cell_index(&app), CELL_DEFAULT);
    }
}
