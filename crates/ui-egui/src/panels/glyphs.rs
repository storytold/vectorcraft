//! Glyphs panel: every character the current font maps (its cmap), drawn from the font's own
//! outlines; Show › Entire Font adds the glyphs no character maps, by glyph id. Double-click
//! inserts the character at the Type tool's caret (or appends it to the selected text objects).
//! The character selected with the Type tool is highlighted in the grid.
//! A character whose glyph has alternates has a triangle in its cell's lower right corner, and a
//! click on it opens a pop-up of those alternates ([`alternates_popup`]); picking one inserts the
//! character drawn with it ([`insert_alternate`]). In Entire Font a glyph no character maps has the
//! triangle too, and a pick goes on the selected character as a double-click on its cell does.
//! Show › Alternates for Current Selection lists that character's alternate glyphs instead (as it
//! stands in its word), and a double-click applies the OpenType feature that draws one to that
//! character alone, so the text stays real text that joins its neighbors. A double-click on a
//! glyph no character maps in Entire Font does the same when the selected character's font gives
//! that glyph as one of its alternates in its word. A glyph no feature reaches whose name does
//! not belong to another letter is applied as a glyph choice, which may not join. Other glyphs
//! are drawn dimmed, and their tooltips say why they cannot be applied to the selection.
//! With Show East Asian Options, the panel menu sets the selected characters' East Asian glyph
//! form ([`glyph_form_items`]).
//! After its Unicode ranges and Alternates for Current Selection, the Show menu lists the OpenType
//! categories the browsed font has ([`vectorcraft_text::FontFace::glyph_categories`]): each shows
//! the glyphs its features give, as Entire Font shows them.

use std::collections::{HashMap, HashSet};

use egui::{Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;
use vectorcraft_doc::{CharStyle, GlyphForm, NodeId, NodeKind};
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

/// The Show menu's choice after [`SUBSETS`] (its index is `SUBSETS.len()`): the alternate glyphs
/// of the Type tool's selected character. Public behavior: [Replace characters with alternate
/// glyphs](https://helpx.adobe.com/illustrator/desktop/design-with-text/special-characters-glyphs/replace-characters-with-alternate-glyphs.html)
/// (select Alternates For Current Selection from the Show menu; highlight a character, then
/// double-click the glyph to replace it).
const ALTERNATES: &str = "Alternates for Current Selection";

/// The Show menu's choice of an OpenType category of the browsed font, after [`ALTERNATES`]; the
/// panel state `gl-category` holds the category's id ([`vectorcraft_text::GlyphCategoryKind::id`]).
/// Public behavior: [Glyphs panel overview](https://helpx.adobe.com/illustrator/desktop/design-with-text/special-characters-glyphs/glyphs-panel-overview.html)
/// (with an OpenType font, a category chosen from the Show menu restricts the kinds of glyphs the
/// panel displays). Which categories are offered, their names and their order are ours.
const CATEGORY: usize = SUBSETS.len() + 1;

/// The Show menu label of `category`: a stylistic set by the font's name for it, else `Set N`, as
/// the OpenType panel names it.
fn category_label(category: &vectorcraft_text::GlyphCategory) -> String {
    use vectorcraft_text::GlyphCategoryKind as K;
    match category.kind {
        K::Ligatures => tl!("Ligatures").to_string(),
        K::Swashes => tl!("Swash").to_string(),
        K::StylisticAlternates => tl!("Stylistic Alternates").to_string(),
        K::StylisticSet(number) => super::opentype::set_label(&vectorcraft_text::StylisticSet { number, name: category.name.clone() }),
        K::SmallCaps => tl!("Small Caps").to_string(),
        K::Ornaments => tl!("Ornaments").to_string(),
        K::Fractions => tl!("Fractions").to_string(),
        K::Figures => tl!("Figures").to_string(),
    }
}

/// The ids and Show menu labels of `face`'s OpenType categories, in the menu's order.
fn categories(face: Option<&vectorcraft_text::FontFace>) -> Vec<(String, String)> {
    face.map(|f| f.glyph_categories().iter().map(|c| (c.kind.id(), category_label(c))).collect()).unwrap_or_default()
}

/// The Show menu's choice: the entry index (a Unicode range, [`ALTERNATES`] or [`CATEGORY`]) and,
/// for a category, its place in `cats`. A category the browsed font does not have is Entire Font,
/// and so is an index past the entries.
fn shown(ctx: &egui::Context, cats: &[(String, String)]) -> (usize, Option<usize>) {
    let subset: usize = pstate(ctx, "gl-subset");
    match subset {
        CATEGORY => {
            let id: String = pstate(ctx, "gl-category");
            match cats.iter().position(|c| c.0 == id) {
                Some(i) => (CATEGORY, Some(i)),
                None => (0, None),
            }
        }
        s if s > CATEGORY => (0, None),
        s => (s, None),
    }
}

/// Show category `id` of the browsed font.
fn show_category(ctx: &egui::Context, id: &str) {
    set_pstate(ctx, "gl-category", id.to_string());
    set_pstate(ctx, "gl-subset", CATEGORY);
}

/// The face the panel browses: the font picked in the panel, else the current text's font, else
/// the fallback family, with that family and style. The same family and style can have a different
/// face id at the selection's version or axes; that instance is used when the Type tool's selected
/// character is set in the font.
fn browsed_face(app: &VectorcraftApp, ctx: &egui::Context) -> (String, String, Option<std::sync::Arc<vectorcraft_text::FontFace>>) {
    let db = FontDb::global();
    let current = super::character::text_style(app).map(|(s, _)| (s.font_family, s.font_style));
    // The panel browses the current text's font unless another font was picked here.
    let picked: Option<(String, String)> = pstate(ctx, "gl-font");
    let (family, style) = picked.or(current).unwrap_or_else(|| (vectorcraft_text::FALLBACK_FAMILY.to_string(), "Regular".to_string()));
    let face = selected_target(app)
        .filter(|t| t.style.font_family.eq_ignore_ascii_case(&family) && t.style.font_style.eq_ignore_ascii_case(&style))
        .and_then(|t| db.style_face(t.style))
        .or_else(|| db.face(&family, &style));
    (family, style, face)
}

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
    selected_target(app).map(|t| t.ch)
}

/// The character [`selected_char`] picks, where it is and the style of its run.
struct Target<'a> {
    id: NodeId,
    /// The character's first byte in the text.
    byte: usize,
    ch: char,
    style: &'a CharStyle,
}

fn selected_target(app: &VectorcraftApp) -> Option<Target<'_>> {
    let (id, a, b) = super::character::text_editing(app)?;
    let NodeKind::Text(t) = &app.session.active()?.doc.node(id)?.kind else { return None };
    let (at, range) = (a.min(b), a != b);
    let mut start = 0;
    for run in &t.runs {
        let end = start + run.text.len();
        let byte = if range && (start..end).contains(&at) {
            at
        } else if !range && at > start && at <= end {
            at - run.text.get(..at - start)?.chars().next_back()?.len_utf8()
        } else {
            start = end;
            continue;
        };
        let ch = run.text.get(byte - start..)?.chars().next()?;
        return Some(Target { id, byte, ch, style: &run.style });
    }
    None
}

/// Insert `ch` at the Type tool's caret, else append it to the selected text; whether it succeeded.
fn insert(app: &mut VectorcraftApp, ctx: &egui::Context, ch: char) -> bool {
    let s = ch.to_string();
    if app.session.tool_wants_text() {
        let v = app.view_info();
        let target_before = crate::ime::target(&mut app.session, v);
        let before = crate::ime::stamp(&mut app.session, v);
        let r = app.session.tool_text(&s, v);
        let inserted = r.is_ok();
        match &r {
            Err(e) => app.status(e.to_string()),
            Ok(_) => crate::ime::note_application_edit(app, ctx, v, target_before, before),
        }
        app.sync_views();
        inserted
    } else if super::character::text_style(app).is_some() {
        app.run("type.insert", json!({"text": s})).is_ok()
    } else {
        app.ui.status = "Select text or click into text with the Type tool to insert glyphs".into();
        false
    }
}

/// Browse and insert glyphs from the chosen font, with font controls below the glyph grid.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The same family and style can have a different face id at the selection's version or axes.
    // That instance is used for both the previews and Fit's applicability check.
    let (family, style, face) = browsed_face(app, ui.ctx());
    let cats = categories(face.as_deref());
    let (subset, category) = shown(ui.ctx(), &cats);
    let (_, lo, hi) = SUBSETS[subset.min(SUBSETS.len() - 1)];
    let w = ui.available_width();
    // A stylistic set's name is the font's, shown as it is: the built-in labels are translated here.
    let labels: Vec<String> =
        SUBSETS.iter().map(|s| tl!(s.0).to_string()).chain([tl!(ALTERNATES).to_string()]).chain(cats.iter().map(|c| c.1.clone())).collect();
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    let current = match category {
        Some(i) => CATEGORY + i,
        None => subset,
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Show:")).color(t.text_dim));
        if let Some(i) = widgets::dropdown_names(ui, "gl-subset", names[current.min(names.len() - 1)], &names, (w - 50.0).max(80.0)) {
            match i.checked_sub(CATEGORY).and_then(|k| cats.get(k)) {
                Some((id, _)) => show_category(ui.ctx(), id),
                None => set_pstate(ui.ctx(), "gl-subset", i),
            }
        }
    });
    if subset == SUBSETS.len() {
        alternates(app, ui, w);
        font_row(app, ui, &family, &style, w);
        return;
    }
    let Some(face) = face else {
        ui.label(egui::RichText::new(tl!("No fonts are available.")).color(t.text_dim));
        return;
    };
    let (chars, unencoded): (Vec<(char, u32)>, Vec<u32>) = match category.and_then(|i| face.glyph_categories().get(i)) {
        // A category lists its glyphs that a character maps by that character, and the others by
        // glyph id after them.
        Some(c) => {
            let wanted: HashSet<u32> = c.glyphs.iter().copied().collect();
            // A glyph several characters map is listed once, by the first of them; a glyph only a
            // space or control character maps is listed by glyph id, as no character it can insert.
            let mut mapped: HashSet<u32> = HashSet::new();
            let chars: Vec<(char, u32)> =
                face.chars().into_iter().filter(|(c, g)| !c.is_control() && !c.is_whitespace() && wanted.contains(g) && mapped.insert(*g)).collect();
            (chars, c.glyphs.iter().copied().filter(|g| !mapped.contains(g)).collect())
        }
        None => {
            let chars = face.chars().into_iter().filter(|(c, _)| (lo..=hi).contains(&(*c as u32)) && !c.is_control() && !c.is_whitespace()).collect();
            // Entire Font also lists the glyphs no character maps (positional forms, alternates,
            // ligatures) by glyph id, after the characters.
            (chars, if subset == 0 { face.unencoded_glyphs().to_vec() } else { vec![] })
        }
    };
    let total = chars.len() + unencoded.len();
    let cell = CELL_STEPS[cell_index(app)];
    let cols = ((w / cell).floor() as usize).max(1);
    let rows = total.div_ceil(cols);
    let px = (cell * ui.ctx().pixels_per_point()).round().clamp(8.0, 256.0) as u32;
    let list_h = widgets::panel_list_height(ui, (ui.available_height() - 64.0).max(cell * 3.0), 33.0);
    let hover_id = egui::Id::unique("gl-hover");
    let selected = selected_char(app);
    // What a double-click on an unencoded glyph can do to the selected character.
    let fit = (!unencoded.is_empty()).then(|| Fit::of(app, ui, &face));
    // The selected character and the rows shown last frame: the grid scrolls to the selected glyph
    // only when the selection moves to another character, so it still scrolls freely meanwhile.
    let follow_id = egui::Id::unique("gl-follow");
    let (followed, shown): (Option<char>, std::ops::Range<usize>) = ui.ctx().data(|d| d.get_temp(follow_id)).unwrap_or_default();
    let mut area = egui::ScrollArea::vertical().max_height(list_h).min_scrolled_height(list_h.min(64.0)).auto_shrink([false, false]);
    if selected != followed
        && let Some(c) = selected
        && let Ok(i) = chars.binary_search_by_key(&c, |x| x.0)
        && !shown.contains(&(i / cols))
    {
        let row_h = cell + ui.spacing().item_spacing.y;
        area = area.vertical_scroll_offset((((i / cols) as f32 + 0.5) * row_h - list_h * 0.5).max(0.0));
    }
    // A pick in a cell's pop-up of alternates: the character (none for a glyph no character maps),
    // the glyph of its cell, the glyph picked.
    let mut popup_pick: Option<(Option<char>, u32, u32)> = None;
    let shown =
        area.show_rows(ui, cell, rows, |ui, range| {
            for r in range.clone() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    for k in r * cols..((r + 1) * cols).min(total) {
                        let (c, gid) = match chars.get(k) {
                            Some(&(c, gid)) => (Some(c), gid),
                            None => (None, unencoded.get(k - chars.len()).copied().unwrap_or_default()),
                        };
                        let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
                        let chosen = c.is_some() && selected == c;
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 2, t.hover);
                            ui.ctx().data_mut(|d| d.insert_temp(hover_id, (c, gid)));
                        } else if chosen {
                            ui.painter().rect_filled(rect, 2, t.row_selected);
                        }
                        let edge = if chosen { Stroke::new(1.5, t.accent) } else { Stroke::new(0.5, t.border) };
                        ui.painter().rect_stroke(rect.shrink(0.5), 0, edge, StrokeKind::Inside);
                        // A glyph no character maps is dimmed when it cannot go on the selection.
                        let pick = match (c, &fit) {
                            (None, Some(fit)) => Some(fit.pick(&face, gid)),
                            _ => None,
                        };
                        let ink = if matches!(pick, Some(None)) { t.text_disabled } else { t.text_strong };
                        let tex = glyph_texture(ui.ctx(), &face, gid, px);
                        ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), ink);
                        // Only the cells drawn ask whether their glyph has alternates.
                        if face.has_alternates(gid)
                            && let Some(picked) = corner_popup(ui, &face, c, gid, rect, cell)
                        {
                            popup_pick = Some((c, gid, picked));
                        }
                        // The tooltip is made when it is shown: working out why a glyph cannot be
                        // applied reads the font, which hundreds of cells cannot do each frame.
                        let resp = resp.on_hover_ui(|ui| {
                        let tip = match (c, &pick) {
                            (Some(c), _) => encoded_tip(&face, c, gid),
                            (None, Some(Some(Pick::Glyph(_)))) => format!(
                                "{}\n{}\n{}",
                                unencoded_tip(&face, gid),
                                tl!("Double-click to replace"),
                                tl!("No OpenType feature draws this glyph here, so it replaces the character's glyph and may not join its neighbors.")
                            ),
                            (None, Some(Some(_))) => format!("{}\n{}", unencoded_tip(&face, gid), tl!("Double-click to replace")),
                            (None, Some(None)) => {
                                let why = fit.as_ref().map(|f| f.why(&face, gid)).unwrap_or_default();
                                format!("{}\n{why}", unencoded_tip(&face, gid))
                            }
                            (None, None) => unencoded_tip(&face, gid),
                        };
                        ui.label(tip);
                    });
                        if resp.double_clicked() {
                            match (c, pick) {
                                (Some(c), _) => insert_character(app, ui.ctx(), &face, c, gid),
                                (None, Some(Some(pick))) => {
                                    if let Some(fit) = &fit {
                                        fit.apply(app, pick);
                                    }
                                }
                                (None, Some(None)) => {
                                    if let Some(fit) = &fit {
                                        app.status(fit.why(&face, gid));
                                    }
                                }
                                (None, None) => {}
                            }
                        }
                    }
                });
            }
            range
        });
    ui.ctx().data_mut(|d| d.insert_temp(follow_id, (selected, shown.inner)));
    match (popup_pick, &fit) {
        (Some((Some(c), cell_gid, picked)), _) => insert_alternate(app, ui.ctx(), &face, c, cell_gid, picked),
        // A glyph no character maps goes on the selected character as a double-click on its cell
        // in Entire Font puts it.
        (Some((None, _, picked)), Some(fit)) => match fit.pick(&face, picked) {
            Some(pick) => fit.apply(app, pick),
            None => app.status(fit.why(&face, picked)),
        },
        _ => {}
    }
    ui.add_space(4.0);
    let info = ui.ctx().data(|d| d.get_temp::<(Option<char>, u32)>(hover_id));
    let count = crate::i18n::tn(total as u64, "{n} glyph", "{n} glyphs");
    ui.label(
        egui::RichText::new(match info {
            Some((c, g)) => {
                let glyph = crate::i18n::fmt(tl!("glyph {id}"), &[("id", &g.to_string())]);
                match c {
                    Some(c) => format!("U+{:04X}  {glyph}  ·  {count}", c as u32),
                    None => format!("{glyph}  ·  {count}"),
                }
            }
            None => count,
        })
        .size(11.0)
        .color(t.text_dim),
    );
    font_row(app, ui, &family, &style, w);
}

/// The family and style menus of the font the panel browses, and Zoom Out and Zoom In.
fn font_row(app: &mut VectorcraftApp, ui: &mut Ui, family: &str, style: &str, w: f32) {
    let db = FontDb::global();
    widgets::panel_footer_space(ui, 24.0);
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
        let pick = crate::font_menu::font_menu(ui, "gl-family", family, family_w, sample.as_deref(), look);
        if let Some((f, style)) = crate::font_menu::picked(app, pick) {
            let st = style.unwrap_or_else(|| db.face(&f, "Regular").map_or_else(|| "Regular".into(), |face| face.style.clone()));
            set_pstate(ui.ctx(), "gl-font", Some((f, st)));
        }
        let styles = db.styles(family);
        let snames: Vec<&str> = styles.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown_names(ui, "gl-style", style, &snames, style_w)
            && let Some(name) = snames.get(i)
        {
            set_pstate(ui.ctx(), "gl-font", Some((family.to_string(), (*name).to_string())));
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

/// One cell of Show › Alternates for Current Selection: a glyph, the feature that lists it, and the
/// feature tag that draws it on the selected character in its word (`salt=2`), when one does.
struct Variant {
    gid: u32,
    feature: [u8; 4],
    apply: Option<String>,
    /// The glyph the shaper makes for the character with none of its own alternate features.
    default: bool,
}

/// What a double-click on a variant does to the selected character.
enum Pick {
    /// Back to the default glyph: its alternate features off, a glyph choice taken away.
    Default,
    /// A feature that draws the variant: the text stays real Unicode and its word stays joined.
    Feature(String),
    /// No feature draws it: it replaces the character's glyph (`text.setGlyphAlternate`), which is
    /// shaped apart from its neighbors.
    Glyph(u32),
}

/// The grid of Show › Alternates for Current Selection: the alternate glyphs of the Type tool's
/// selected character as it stands in its word ([`vectorcraft_text::alternates_in_context`]: a
/// medial letter lists its medial form's alternates), in the face the text is set in, whatever font
/// the panel browses, with that face's name under the grid. The glyph the shaper makes for the
/// character with none of its own alternate features comes first, and the glyph the character is
/// drawn with is highlighted. A double-click applies the OpenType feature that draws the glyph to
/// that character alone (`text.setRangeStyle`, `salt=2`), as one undo step, so the text stays real
/// text that joins its neighbors. A glyph no feature draws there replaces the character's glyph
/// instead (`text.setGlyphAlternate`), and its tooltip says that it may not join.
fn alternates(app: &mut VectorcraftApp, ui: &mut Ui, w: f32) {
    let t = Tokens::get(ui.ctx());
    let db = FontDb::global();
    let note = |ui: &mut Ui, text: &str| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(text).color(t.text_dim));
    };
    let Some(target) = selected_target(app) else {
        return note(ui, tl!("Select a character with the Type tool to see its alternates."));
    };
    let face = db.style_face(target.style).filter(|_| db.has_family(&target.style.font_family));
    let Some(face) = face else {
        return note(ui, tl!("The selected character's font is not available."));
    };
    let (id, byte, ch) = (target.id, target.byte, target.ch);
    let (features, has_choice) = (target.style.features.clone(), target.style.glyph_alternate.is_some());
    let found = alternates_found(app, ui, id, byte, face.id());
    let font = format!("{} {}", face.family, face.style);
    let Some(found) = found.filter(|f| has_choice || !f.alternates.is_empty()) else {
        note(ui, tl!("No alternates for the selected character."));
        ui.label(egui::RichText::new(font).size(11.0).color(t.text_dim));
        return;
    };
    // The glyphs are drawn from the face that supplied the character's glyph (a fallback face for
    // a character the text's font lacks).
    let face = db.face_by_id(found.font_id).unwrap_or(face);
    let drawn = Some(found.current);
    let mut list = vec![Variant { gid: found.default, feature: [0; 4], apply: None, default: true }];
    list.extend(found.alternates.iter().filter(|a| a.gid != found.default).map(|a| Variant {
        gid: a.gid,
        feature: a.feature,
        apply: a.apply.clone(),
        default: false,
    }));
    let cell = CELL_STEPS[cell_index(app)];
    let cols = ((w / cell).floor() as usize).max(1);
    let rows = list.len().div_ceil(cols);
    let px = (cell * ui.ctx().pixels_per_point()).round().clamp(8.0, 256.0) as u32;
    let list_h = widgets::panel_list_height(ui, (ui.available_height() - 64.0).max(cell * 3.0), 33.0);
    let mut pick = None;
    // A scroll position of its own for each character, so the grid starts at the top for another.
    let area = egui::ScrollArea::vertical()
        .id_salt(("gl-alternates", id.0, byte))
        .max_height(list_h)
        .min_scrolled_height(list_h.min(64.0))
        .auto_shrink([false, false]);
    area.show_rows(ui, cell, rows, |ui, range| {
        for r in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for a in list.iter().skip(r * cols).take(cols) {
                    let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
                    let current = drawn == Some(a.gid);
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 2, t.hover);
                    } else if current {
                        ui.painter().rect_filled(rect, 2, t.row_selected);
                    }
                    let edge = if current { Stroke::new(1.5, t.accent) } else { Stroke::new(0.5, t.border) };
                    ui.painter().rect_stroke(rect.shrink(0.5), 0, edge, StrokeKind::Inside);
                    let tex = glyph_texture(ui.ctx(), &face, a.gid, px);
                    ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), t.text_strong);
                    let glyph = crate::i18n::fmt(tl!("glyph {id}"), &[("id", &a.gid.to_string())]);
                    let glyph = match (&a.apply, a.default) {
                        (_, true) => glyph,
                        (Some(tag), _) => format!("{glyph}  ({tag})"),
                        (None, _) => format!("{glyph}  ({})", String::from_utf8_lossy(&a.feature)),
                    };
                    let mut tip = format!("{glyph}\n{}", tl!("Double-click to replace"));
                    if !a.default && a.apply.is_none() {
                        tip.push('\n');
                        tip.push_str(tl!(
                            "No OpenType feature draws this glyph here, so it replaces the character's glyph and may not join its neighbors."
                        ));
                    }
                    let resp = resp.on_hover_text(tip);
                    if resp.double_clicked() {
                        pick = Some(match (&a.apply, a.default) {
                            (_, true) => Pick::Default,
                            (Some(tag), _) => Pick::Feature(tag.clone()),
                            (None, _) => Pick::Glyph(a.gid),
                        });
                    }
                }
            });
        }
    });
    ui.add_space(4.0);
    let count = crate::i18n::tn(list.len() as u64, "{n} glyph", "{n} glyphs");
    ui.label(egui::RichText::new(format!("{font}  ·  {count}")).size(11.0).color(t.text_dim));
    if let Some(pick) = pick {
        super::character::end_typing(app);
        apply_alternate(app, id, (byte, byte + ch.len_utf8()), &features, has_choice, pick);
    }
}

/// Do what a double-click on a variant says to the character at `bytes` of text `id`, whose style
/// has `features` and, when `has_choice`, a glyph choice: one command, so one undo step. A feature
/// goes through `text.setRangeStyle` on exactly the character (its other alternate features are
/// turned off, a glyph choice is taken away); a glyph no feature draws through
/// `text.setGlyphAlternate`. The default glyph, or a variant the character already has, changes
/// nothing. Whether the command ran or nothing needed to change; a failed command's error goes to
/// the status.
fn apply_alternate(app: &mut VectorcraftApp, id: NodeId, bytes: (usize, usize), features: &[String], has_choice: bool, pick: Pick) -> bool {
    let Some((cmd, params)) = alternate_command(id, bytes, features, has_choice, pick) else { return true };
    match app.run(cmd, params) {
        Ok(_) => true,
        Err(e) => {
            app.status(e.to_string());
            false
        }
    }
}

/// The command and parameters [`apply_alternate`] runs; `None` when nothing needs to change.
fn alternate_command(
    id: NodeId,
    bytes: (usize, usize),
    features: &[String],
    has_choice: bool,
    pick: Pick,
) -> Option<(&'static str, serde_json::Value)> {
    Some(match pick {
        Pick::Glyph(gid) => ("text.setGlyphAlternate", json!({"id": id.0, "start": bytes.0, "gid": gid})),
        Pick::Default | Pick::Feature(_) => {
            let tag = if let Pick::Feature(tag) = &pick { Some(tag.as_str()) } else { None };
            let changes = vectorcraft_text::alternate_changes(features, tag);
            if changes.is_empty() && !has_choice {
                return None;
            }
            let mut params = json!({"id": id.0, "start": bytes.0, "end": bytes.1});
            if !changes.is_empty() {
                params["featureChanges"] = json!(changes);
            }
            if has_choice {
                params["glyphAlternate"] = serde_json::Value::Null;
            }
            ("text.setRangeStyle", params)
        }
    })
}

/// The most alternate glyphs the canvas shows beside a selected character. Public behavior: at
/// most five alternates show on the canvas, and an icon to their right opens the Glyphs panel when
/// the font has more ("Replace a character with on-canvas alternate glyphs",
/// <https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf#page=479>).
pub(crate) const ON_CANVAS_ALTERNATES: usize = 5;

/// The alternate glyphs the canvas offers beside the Type tool's one selected character
/// ([`on_canvas`]).
pub(crate) struct OnCanvas {
    /// The text the character is in.
    pub id: NodeId,
    /// The character's bytes in the text, start and end.
    pub bytes: (usize, usize),
    /// The face that draws the character's glyph.
    face: std::sync::Arc<vectorcraft_text::FontFace>,
    /// Up to [`ON_CANVAS_ALTERNATES`] glyphs, none of them the glyph the character is drawn with.
    cells: Vec<Variant>,
    /// Whether the character has alternates that [`Self::cells`] leaves out.
    pub more: bool,
    features: Vec<String>,
    has_choice: bool,
}

impl OnCanvas {
    /// How many glyphs are offered: 1 to [`ON_CANVAS_ALTERNATES`].
    pub fn count(&self) -> usize {
        self.cells.len()
    }

    /// The id of glyph `i`.
    #[cfg(test)]
    pub fn gid(&self, i: usize) -> Option<u32> {
        self.cells.get(i).map(|c| c.gid)
    }

    /// The preview of glyph `i` in a square of `px` pixels.
    pub fn texture(&self, ctx: &egui::Context, i: usize, px: u32) -> Option<egui::TextureHandle> {
        self.cells.get(i).map(|c| glyph_texture(ctx, &self.face, c.gid, px))
    }

    /// The tooltip of glyph `i`: its id, the feature that draws it, and what a click does.
    pub fn tip(&self, i: usize) -> String {
        let Some(c) = self.cells.get(i) else { return String::new() };
        let glyph = crate::i18n::fmt(tl!("glyph {id}"), &[("id", &c.gid.to_string())]);
        let glyph = match (&c.apply, c.default) {
            (_, true) => glyph,
            (Some(tag), _) => format!("{glyph}  ({tag})"),
            (None, _) => format!("{glyph}  ({})", String::from_utf8_lossy(&c.feature)),
        };
        let mut tip = format!("{glyph}\n{}", tl!("Click to replace"));
        if !c.default && c.apply.is_none() {
            tip.push('\n');
            tip.push_str(tl!("No OpenType feature draws this glyph here, so it replaces the character's glyph and may not join its neighbors."));
        }
        tip
    }

    /// Draw the character with glyph `i`, as a double-click on that glyph in Show › Alternates for
    /// Current Selection does: one command, so one undo step.
    pub fn pick(&self, app: &mut VectorcraftApp, i: usize) {
        let Some(c) = self.cells.get(i) else { return };
        let pick = match (&c.apply, c.default) {
            (_, true) => Pick::Default,
            (Some(tag), _) => Pick::Feature(tag.clone()),
            (None, _) => Pick::Glyph(c.gid),
        };
        super::character::end_typing(app);
        apply_alternate(app, self.id, self.bytes, &self.features, self.has_choice, pick);
    }
}

/// What the canvas offers beside the Type tool's selection: the alternates of the selected
/// character as it stands in its word ([`alternates_found`], found once for each document
/// revision), in the font's order and without the glyph the character is drawn with. A character
/// drawn with an alternate gets its default glyph first. `None` unless the Type tool's selection
/// is exactly one character of one type object whose font is available and gives it another glyph.
pub(crate) fn on_canvas(app: &VectorcraftApp, ui: &Ui) -> Option<OnCanvas> {
    let (_, a, b) = super::character::text_editing(app)?;
    if a == b || super::character::text_story(app).is_some() {
        return None;
    }
    let target = selected_target(app)?;
    let (start, end) = (a.min(b), a.max(b));
    if target.byte != start || start + target.ch.len_utf8() != end {
        return None;
    }
    let db = FontDb::global();
    let face = db.style_face(target.style).filter(|_| db.has_family(&target.style.font_family))?;
    let found = alternates_found(app, ui, target.id, start, face.id())?;
    let face = db.face_by_id(found.font_id).unwrap_or(face);
    let default = (found.default != found.current).then_some(Variant { gid: found.default, feature: [0; 4], apply: None, default: true });
    let others = found.alternates.iter().filter(|a| a.gid != found.default && a.gid != found.current).map(|a| Variant {
        gid: a.gid,
        feature: a.feature,
        apply: a.apply.clone(),
        default: false,
    });
    let mut cells: Vec<Variant> = default.into_iter().chain(others).collect();
    if cells.is_empty() {
        return None;
    }
    let more = cells.len() > ON_CANVAS_ALTERNATES;
    cells.truncate(ON_CANVAS_ALTERNATES);
    Some(OnCanvas {
        id: target.id,
        bytes: (start, end),
        face,
        cells,
        more,
        features: target.style.features.clone(),
        has_choice: target.style.glyph_alternate.is_some(),
    })
}

/// Show the Glyphs panel on Show › Alternates for Current Selection, for the canvas's control that
/// stands for the alternates it leaves out. A panel that is already shown stays where it is.
pub(crate) fn show_alternates(app: &mut VectorcraftApp, ctx: &egui::Context) {
    set_pstate(ctx, "gl-subset", SUBSETS.len());
    if let Err(e) = crate::panel_docking::reveal_panel(app, "glyphs") {
        app.status(e);
    }
}

/// The Type tool's selected character, and what its word offers, for a double-click on a glyph no
/// character maps in Show › Entire Font ([`Fit::pick`]).
struct Fit {
    /// The selected character: its text, where it is and how it is styled. `None` without one.
    target: Option<FitTarget>,
    /// The id of the face the panel browses; its glyph ids are what is applied.
    face_id: u32,
}

struct FitTarget {
    id: NodeId,
    byte: usize,
    ch: char,
    features: Vec<String>,
    has_choice: bool,
    /// The family the selected text is set in, and whether the app has it.
    family: String,
    available: bool,
    /// The alternates of the character in its word, from the face of its text.
    found: Option<vectorcraft_text::ContextAlternates>,
}

impl Fit {
    fn of(app: &VectorcraftApp, ui: &Ui, face: &vectorcraft_text::FontFace) -> Self {
        let db = FontDb::global();
        let target = selected_target(app).map(|t| {
            let text_face = db.style_face(t.style).filter(|_| db.has_family(&t.style.font_family));
            let found = text_face.as_ref().and_then(|f| alternates_found(app, ui, t.id, t.byte, f.id()));
            FitTarget {
                id: t.id,
                byte: t.byte,
                ch: t.ch,
                features: t.style.features.clone(),
                has_choice: t.style.glyph_alternate.is_some(),
                family: t.style.font_family.clone(),
                available: text_face.is_some(),
                found,
            }
        });
        Fit { target, face_id: face.id() }
    }

    /// What a double-click on glyph `gid` does to the selected character: the way to draw it with
    /// that glyph, when it is the character's default glyph or one of its alternates as it stands
    /// in its word, in the face the selected text is set in; or a glyph no feature reaches that
    /// the font's naming does not give to another letter (`A.salt` for `A`, `s.ss1` in a font
    /// with no `ss01`), which is drawn as a glyph choice. `None` when it is not.
    fn pick(&self, face: &vectorcraft_text::FontFace, gid: u32) -> Option<Pick> {
        let t = self.target.as_ref().filter(|t| t.available)?;
        let found = t.found.as_ref().filter(|f| f.font_id == self.face_id)?;
        if gid == found.default {
            return Some(Pick::Default);
        }
        if let Some(a) = found.alternates.iter().find(|a| a.gid == gid) {
            return Some(a.apply.clone().map_or(Pick::Glyph(gid), Pick::Feature));
        }
        face.is_unlisted_alternate(t.ch, gid).then_some(Pick::Glyph(gid))
    }

    /// Why [`Self::pick`] finds nothing for glyph `gid` of `face`, in the user's words: what the
    /// glyph belongs to (its letter and place in a word, from the font's own substitutions) and what
    /// the selection is, or that nothing is selected.
    fn why(&self, face: &vectorcraft_text::FontFace, gid: u32) -> String {
        let Some(t) = &self.target else { return tl!("Select one character with the Type tool first.").to_string() };
        if !t.available {
            return tl!("The selected character's font is not available.").to_string();
        }
        let name = glyph_label(face, gid);
        if let Some(found) = t.found.as_ref().filter(|f| f.font_id != self.face_id) {
            let other = FontDb::global().face_by_id(found.font_id).map_or_else(|| t.family.clone(), |f| format!("{} {}", f.family, f.style));
            return crate::i18n::fmt(
                tl!("{glyph} is a glyph of {font}, not of the font the selected text is set in ({text_font})."),
                &[("glyph", &name), ("font", &format!("{} {}", face.family, face.style)), ("text_font", &other)],
            );
        }
        let (sel, sel_code) = (t.ch.to_string(), format!("{:04X}", t.ch as u32));
        let not_alternate = || {
            crate::i18n::fmt(
                tl!("{glyph} is not an alternate of {sel} (U+{code}) as it stands in its word."),
                &[("glyph", &name), ("sel", &sel), ("code", &sel_code)],
            )
        };
        let Some(origin) = face.glyph_origin(gid) else { return not_alternate() };
        let place = match origin.position() {
            Some(vectorcraft_text::JoiningPosition::Initial) => Some(tl!("at the start of a word")),
            Some(vectorcraft_text::JoiningPosition::Medial) => Some(tl!("in the middle of a word")),
            Some(vectorcraft_text::JoiningPosition::Final) => Some(tl!("at the end of a word")),
            Some(vectorcraft_text::JoiningPosition::Isolated) => Some(tl!("standing alone")),
            None => None,
        };
        let (Some(place), true) = (place, origin.ch != t.ch) else { return not_alternate() };
        crate::i18n::fmt(
            tl!("{glyph} belongs to {base} (U+{base_code}) {place}; your selection is {sel} (U+{code})."),
            &[
                ("glyph", &name),
                ("base", &origin.ch.to_string()),
                ("base_code", &format!("{:04X}", origin.ch as u32)),
                ("place", place),
                ("sel", &sel),
                ("code", &sel_code),
            ],
        )
    }

    /// Apply `pick` to the selected character, as a double-click on a variant of Show › Alternates
    /// for Current Selection does.
    fn apply(&self, app: &mut VectorcraftApp, pick: Pick) {
        let Some(t) = &self.target else { return };
        super::character::end_typing(app);
        apply_alternate(app, t.id, (t.byte, t.byte + t.ch.len_utf8()), &t.features, t.has_choice, pick);
    }
}

/// `name` of glyph `gid`, or `gid N`.
fn glyph_label(face: &vectorcraft_text::FontFace, gid: u32) -> String {
    face.glyph_name(gid).unwrap_or_else(|| format!("gid {gid}"))
}

/// The letter to insert in place of character `c`, an Arabic presentation form: the letter whose
/// glyph the font substitutes with `init`, `medi`, `fina` or `isol` to get `c`'s glyph. The
/// presentation form itself is a fixed shape that neither joins its neighbors nor is joined by
/// them; the letter takes the form its place in the word asks for, so the word stays joined.
/// `None` when `c` is not such a form in this font.
fn base_letter(face: &vectorcraft_text::FontFace, c: char, gid: u32) -> Option<char> {
    if !matches!(c, '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}') {
        return None;
    }
    let origin = face.glyph_origin(gid)?;
    origin.position()?;
    (origin.ch != c).then_some(origin.ch)
}

/// The tooltip of an encoded glyph (a character): its code point and glyph, and what a
/// double-click inserts.
fn encoded_tip(face: &vectorcraft_text::FontFace, c: char, gid: u32) -> String {
    let head = format!("U+{:04X}  {c}  (glyph {gid})", c as u32);
    match base_letter(face, c, gid) {
        Some(base) => format!(
            "{head}\n{}",
            crate::i18n::fmt(
                tl!("Double-click to insert {base} (U+{code}) instead: a presentation form does not join, the letter does."),
                &[("base", &base.to_string()), ("code", &format!("{:04X}", base as u32))]
            )
        ),
        None => format!("{head}\n{}", tl!("Double-click to insert")),
    }
}

/// Double-click on character `c` of the grid: insert it, or its letter when it is an Arabic
/// presentation form ([`base_letter`]), which the status says.
fn insert_character(app: &mut VectorcraftApp, ctx: &egui::Context, face: &vectorcraft_text::FontFace, c: char, gid: u32) {
    match base_letter(face, c, gid) {
        Some(base) => {
            if insert(app, ctx, base) {
                app.status(crate::i18n::fmt(
                    tl!("Inserted {base} (U+{code}) instead of a presentation form: the font joins it to its neighbors by position."),
                    &[("base", &base.to_string()), ("code", &format!("{:04X}", base as u32))],
                ));
            }
        }
        None => {
            insert(app, ctx, c);
        }
    }
}

/// The side of the triangle in the lower right corner of a grid cell `cell` points wide whose glyph
/// has alternates: a quarter of the cell, from 5 to 10 points (VectorCraft's own size).
fn corner_side(cell: f32) -> f32 {
    (cell * 0.25).clamp(5.0, 10.0)
}

/// The most columns, and the most rows before it scrolls, of a cell's pop-up of alternates
/// (VectorCraft's own numbers).
const POPUP_COLUMNS: usize = 8;
const POPUP_ROWS: usize = 6;

/// The triangle in the lower right corner of the grid cell `rect` of character `c` (none for a
/// glyph no character maps), whose glyph `gid` has alternates, and the pop-up a click on it opens
/// ([`alternates_popup`]). Public
/// behavior: a pop-up of alternate glyphs opens from the triangle in the lower right corner of a
/// glyph's box where the glyph has alternates
/// ([CS6 reference, Glyphs panel overview](https://web.archive.org/web/20220911143322/https://helpx.adobe.com/pdf/cs6/illustrator_reference.pdf#page=373)).
/// Returns the glyph picked in the pop-up, in the frame it is picked.
fn corner_popup(ui: &mut Ui, face: &vectorcraft_text::FontFace, c: Option<char>, gid: u32, rect: egui::Rect, cell: f32) -> Option<u32> {
    let t = Tokens::get(ui.ctx());
    let side = corner_side(cell);
    let corner = egui::Rect::from_min_max(rect.max - vec2(side + 1.0, side + 1.0), rect.max - vec2(1.0, 1.0));
    let resp = ui.interact(corner, egui::Id::unique(("gl-alternates-corner", face.id(), c, gid)), Sense::click());
    let fill = if resp.hovered() { t.accent } else { t.text_dim };
    ui.painter().add(egui::Shape::convex_polygon(vec![corner.right_top(), corner.right_bottom(), corner.left_bottom()], fill, Stroke::NONE));
    let resp = resp.on_hover_text(tl!("Show Character Alternates"));
    let mut picked = None;
    egui::Popup::menu(&resp).show(|ui| picked = alternates_popup(ui, face, c, gid, cell.max(CELL_STEPS[CELL_DEFAULT])));
    picked
}

/// The pop-up of the alternates of glyph `gid`: that glyph first, then the alternates
/// [`vectorcraft_text::FontFace::alternates`] lists for character `c`, or for a glyph no character
/// maps the glyphs [`vectorcraft_text::FontFace::substitutions`] gives it, in the font's order, in
/// cells `cell` points wide, [`POPUP_COLUMNS`] to a row and [`POPUP_ROWS`] rows before it scrolls.
/// The glyph clicked.
fn alternates_popup(ui: &mut Ui, face: &vectorcraft_text::FontFace, c: Option<char>, gid: u32, cell: f32) -> Option<u32> {
    let t = Tokens::get(ui.ctx());
    let mut list: Vec<(u32, Option<[u8; 4]>)> = vec![(gid, None)];
    match c {
        Some(c) => list.extend(face.alternates(c).into_iter().map(|a| (a.gid, Some(a.feature)))),
        None => list.extend(face.substitutions(gid).into_iter().flat_map(|s| s.glyphs.into_iter().map(move |g| (g, Some(s.feature))))),
    }
    let mut seen = std::collections::HashSet::new();
    list.retain(|(g, _)| seen.insert(*g));
    let cols = list.len().clamp(1, POPUP_COLUMNS);
    let rows = list.len().div_ceil(cols);
    let px = (cell * ui.ctx().pixels_per_point()).round().clamp(8.0, 256.0) as u32;
    let mut picked = None;
    let area = egui::ScrollArea::vertical().id_salt(("gl-alternates-popup", face.id(), c, gid)).max_height(cell * POPUP_ROWS as f32);
    area.show_rows(ui, cell, rows, |ui, range| {
        for r in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for &(g, feature) in list.iter().skip(r * cols).take(cols) {
                    let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 2, t.hover);
                    }
                    ui.painter().rect_stroke(rect.shrink(0.5), 0, Stroke::new(0.5, t.border), StrokeKind::Inside);
                    let tex = glyph_texture(ui.ctx(), face, g, px);
                    ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), t.text_strong);
                    let glyph = crate::i18n::fmt(tl!("glyph {id}"), &[("id", &g.to_string())]);
                    let tip = match feature {
                        Some(f) => format!("{glyph}  ({})", String::from_utf8_lossy(&f)),
                        None => glyph,
                    };
                    if resp.on_hover_text(tip).clicked() {
                        picked = Some(g);
                    }
                }
            });
        }
    });
    picked
}

/// A pick in the pop-up of character `c`'s cell, whose glyph is `gid` ([`alternates_popup`]).
/// The cell's own glyph inserts the character as a double-click on the cell does
/// ([`insert_character`]). An alternate needs the Type tool's caret or selection in text set in
/// `face`: when the selection is exactly one `c` in one type object, that character is drawn with
/// the glyph, and the selection follows it to the frame the text flows it to; else `c` goes in at the caret, in place of the selection (with the rest of a
/// threaded story that Select All selected), and is drawn with it, the two as one undo step. The
/// glyph goes on as a double-click in Show › Alternates for Current Selection puts it
/// ([`draw_selected_with`]). An Arabic presentation form goes in as its base letter, as a
/// double-click on its cell puts it, drawn with the glyph where the font gives it that glyph in its
/// word. Without a caret, with text in another font, or with a glyph the character cannot be drawn
/// with there, the status says why and nothing changes.
fn insert_alternate(app: &mut VectorcraftApp, ctx: &egui::Context, face: &vectorcraft_text::FontFace, c: char, gid: u32, picked: u32) {
    if picked == gid {
        return insert_character(app, ctx, face, c, gid);
    }
    let db = FontDb::global();
    let Some((editing, a, b)) = super::character::text_editing(app) else {
        app.ui.status = "Select text or click into text with the Type tool to insert glyphs".into();
        return;
    };
    let style = super::character::text_style(app).map(|(s, _)| s);
    let text_face = style.as_ref().filter(|s| db.has_family(&s.font_family)).and_then(|s| db.style_face(s));
    if text_face.as_ref().map(|f| f.id()) != Some(face.id()) {
        let text_font = match &text_face {
            Some(f) => format!("{} {}", f.family, f.style),
            None => style.map(|s| s.font_family).unwrap_or_default(),
        };
        app.status(crate::i18n::fmt(
            tl!("{glyph} is a glyph of {font}, not of the font the selected text is set in ({text_font})."),
            &[("glyph", &glyph_label(face, picked)), ("font", &format!("{} {}", face.family, face.style)), ("text_font", &text_font)],
        ));
        return;
    }
    let c = base_letter(face, c, gid).unwrap_or(c);
    // Typing in progress stays a step of its own.
    super::character::end_typing(app);
    // After Select All in a threaded story, the other frames' text goes too, as typing removes it,
    // even when the edited frame holds only `c`.
    let story = super::character::text_story(app).unwrap_or_default();
    let one = story.is_empty() && selected_target(app).is_some_and(|t| t.ch == c && (a.min(b), a.max(b)) == (t.byte, t.byte + c.len_utf8()));
    if one {
        let Some(at) = app.session.active().map(|st| story_start(&st.doc, editing, &[]) + a.min(b)) else { return };
        if draw_selected_with(app, face, c, picked) {
            // The glyph can change the character's width and flow it on to the next frame: the
            // selection follows it there.
            select_in_story(app, (editing, at), c, (0, c.len_utf8()));
        }
        return;
    }
    // Inside an undo group that is already open, the insertion and the glyph are part of that
    // group's step; else they are one step of their own.
    let open = app.session.active().is_some_and(|st| st.undo_group.is_some());
    if !open {
        app.session.begin_named_undo_group(None);
    }
    put_alternate(app, face, c, picked, editing, (a.min(b), a.max(b)), &story);
    if !open && let Some(st) = app.session.active_mut() {
        st.undo_group = None;
    }
}

/// The insertion of [`insert_alternate`]: `c` takes the place of the Type tool's selection `lo..hi`
/// of text `id` (with the text of the other frames of `story` when the Type tool has the whole
/// threaded story selected), in the style of the selection's first character, and is drawn with
/// glyph `picked`. The edits are tried on a copy of the document first, and a pick the copy refuses
/// changes nothing, the undo history included.
fn put_alternate(
    app: &mut VectorcraftApp,
    face: &vectorcraft_text::FontFace,
    c: char,
    picked: u32,
    id: NodeId,
    (lo, hi): (usize, usize),
    story: &[NodeId],
) {
    let style = if lo < hi {
        match app.run("text.getRange", json!({"id": id.0, "start": lo, "end": hi})) {
            Ok(v) => v.pointer("/runs/0/style").cloned(),
            Err(e) => {
                app.status(e.to_string());
                return;
            }
        }
    } else {
        None
    };
    let clear: Vec<NodeId> = story.iter().copied().filter(|f| *f != id).collect();
    let mut edit = json!({"id": id.0, "start": lo, "end": hi, "clear": clear.iter().map(|f| f.0).collect::<Vec<_>>()});
    match style {
        Some(style) => edit["runs"] = json!([{"text": c.to_string(), "style": style}]),
        None => edit["insert"] = json!(c.to_string()),
    }
    // The character's place in the story: a full frame of threaded text flows it on to the next.
    let Some(at) = app.session.active().map(|st| story_start(&st.doc, id, &clear) + lo) else { return };
    if let Err(e) = try_on_a_copy(app, face, c, picked, (id, at), &edit) {
        app.status(e);
        return;
    }
    if let Err(e) = app.run("text.editRange", edit) {
        app.status(e.to_string());
        return;
    }
    let n = c.len_utf8();
    if select_in_story(app, (id, at), c, (0, n)) && draw_selected_with(app, face, c, picked) {
        // The glyph can change the character's width and flow it on again: find it again.
        select_in_story(app, (id, at), c, (n, n));
    }
}

/// The Type tool's selection becomes bytes `start..end` from character `c` at byte `at` of the
/// story of text `id`, in the frame that holds the character; whether it is there. When that
/// frame is not the one the Type tool edits, the tool edits it, and it is selected, as a click in
/// it selects it (`move_editing_selection` in the engine's `panelcmds.rs`).
fn select_in_story(app: &mut VectorcraftApp, (id, at): (NodeId, usize), c: char, (start, end): (usize, usize)) -> bool {
    let Some((frame, byte)) = app.session.active().and_then(|st| story_place(&st.doc, id, at, c)) else { return false };
    if super::character::text_editing(app).is_some_and(|(editing, ..)| editing == frame) {
        app.session.set_tool_option("select", &json!({"start": byte + start, "end": byte + end}));
        return true;
    }
    app.session.set_tool_option("textSelection", &json!({"editing": frame.0, "anchor": byte + start, "caret": byte + end}));
    let selected =
        app.session.active().is_some_and(|st| std::iter::successors(Some(frame), |f| st.doc.parent_of(*f)).any(|f| st.selection.contains(f)));
    if !selected && let Err(e) = app.session.select(|_, sel| sel.set([frame])) {
        app.status(e.to_string());
        return false;
    }
    true
}

/// The bytes of the text of the frames before frame `id` in its story ([`Document::story_of`]),
/// without the frames of `clear`, whose text an edit removes.
///
/// [`Document::story_of`]: vectorcraft_doc::Document::story_of
fn story_start(doc: &vectorcraft_doc::Document, id: NodeId, clear: &[NodeId]) -> usize {
    doc.story_of(id)
        .into_iter()
        .take_while(|f| *f != id)
        .filter(|f| !clear.contains(f))
        .filter_map(|f| match doc.node(f).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => Some(t.plain_text().len()),
            _ => None,
        })
        .sum()
}

/// The frame of the story of text `id` that holds the character at byte `at` of the story's text,
/// and that character's byte in the frame; `None` unless that character is `c`.
fn story_place(doc: &vectorcraft_doc::Document, id: NodeId, at: usize, c: char) -> Option<(NodeId, usize)> {
    let mut start = 0;
    for f in doc.story_of(id) {
        let Some(NodeKind::Text(t)) = doc.node(f).map(|n| &n.kind) else { continue };
        let text = t.plain_text();
        if at < start + text.len() {
            return text.get(at - start..)?.starts_with(c).then_some((f, at - start));
        }
        start += text.len();
    }
    None
}

/// The edits of [`put_alternate`] on a copy of the active document in a session of its own: `edit`
/// (`text.editRange`) puts `c` in at byte `at` of the story of text `id` (in whichever frame the
/// text flows it to), and glyph `picked` of `face` goes on it. The error of the step that fails; an
/// error too when the character is not where the edit put it.
fn try_on_a_copy(
    app: &VectorcraftApp,
    face: &vectorcraft_text::FontFace,
    c: char,
    picked: u32,
    (id, at): (NodeId, usize),
    edit: &serde_json::Value,
) -> Result<(), String> {
    let Some(st) = app.session.active() else { return Ok(()) };
    let mut copy = vectorcraft_engine::Session::new();
    copy.add_document(vectorcraft_doc::Document::clone(&st.doc), None);
    copy.execute("text.editRange", edit).map_err(|e| e.to_string())?;
    let n = c.len_utf8();
    let doc = copy.active().map(|s| &s.doc);
    let step = match doc.and_then(|d| story_place(d, id, at, c).and_then(|(f, byte)| Some((f, byte, d.node(f)?)))) {
        Some((f, byte, node)) => match &node.kind {
            NodeKind::Text(text) => style_at(text, byte).and_then(|style| {
                alternate_command(f, (byte, byte + n), &style.features, style.glyph_alternate.is_some(), pick_for(text, byte, face, picked))
            }),
            _ => return Err("Select text or click into text with the Type tool to insert glyphs".into()),
        },
        None => return Err("Select text or click into text with the Type tool to insert glyphs".into()),
    };
    if let Some((cmd, params)) = step {
        copy.execute(cmd, &params).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Draw the Type tool's selected character, `c` (a range's first character, else the one before
/// the caret), with glyph `picked` of `face`: through the OpenType feature that draws that glyph
/// on it in its word when one does, else as a glyph choice ([`apply_alternate`]). Whether it
/// worked; nothing changes when the character already has that glyph.
fn draw_selected_with(app: &mut VectorcraftApp, face: &vectorcraft_text::FontFace, c: char, picked: u32) -> bool {
    let Some(t) = selected_target(app).filter(|t| t.ch == c) else { return false };
    let (id, byte, features, has_choice) = (t.id, t.byte, t.style.features.clone(), t.style.glyph_alternate.is_some());
    let pick = match app.session.active().and_then(|s| s.doc.node(id)).map(|node| &node.kind) {
        Some(NodeKind::Text(text)) => pick_for(text, byte, face, picked),
        _ => Pick::Glyph(picked),
    };
    super::character::end_typing(app);
    apply_alternate(app, id, (byte, byte + c.len_utf8()), &features, has_choice, pick)
}

/// How glyph `picked` of `face` goes on the character at `byte` of `text`: the OpenType feature
/// that draws it there in its word, the default glyph, else a glyph choice.
fn pick_for(text: &vectorcraft_text::TextObject, byte: usize, face: &vectorcraft_text::FontFace, picked: u32) -> Pick {
    match vectorcraft_text::alternates_in_context(FontDb::global(), text, byte).filter(|f| f.font_id == face.id()) {
        Some(f) if f.default == picked => Pick::Default,
        Some(f) => f.alternates.iter().find(|x| x.gid == picked).and_then(|x| x.apply.clone()).map_or(Pick::Glyph(picked), Pick::Feature),
        None => Pick::Glyph(picked),
    }
}

/// The style of the character at `byte` of `text`.
fn style_at(text: &vectorcraft_text::TextObject, byte: usize) -> Option<&CharStyle> {
    let mut start = 0;
    text.runs
        .iter()
        .find(|run| {
            let end = start + run.text.len();
            let found = (start..end).contains(&byte);
            start = end;
            found
        })
        .map(|run| &run.style)
}

/// The tooltip of a glyph no character maps (Show › Entire Font): its name from the font, `gid N`
/// when it has none, its id, and the feature that gives it, if one does.
fn unencoded_tip(face: &vectorcraft_text::FontFace, gid: u32) -> String {
    let name = face.glyph_name(gid).unwrap_or_else(|| format!("gid {gid}"));
    let glyph = crate::i18n::fmt(tl!("glyph {id}"), &[("id", &gid.to_string())]);
    match face.feature_reaching(gid) {
        Some(feature) => format!("{name}  ({glyph})  ·  {}", String::from_utf8_lossy(&feature)),
        None => format!("{name}  ({glyph})"),
    }
}

/// The alternates of the character at `byte` of text `id` in its word, and the glyph it is drawn
/// with ([`vectorcraft_text::alternates_in_context`]). Kept in egui's temporary data until the
/// document, the character or the face changes, as [`super::character`] keeps the kerning at the
/// caret.
fn alternates_found(app: &VectorcraftApp, ui: &Ui, id: NodeId, byte: usize, face_id: u32) -> Option<vectorcraft_text::ContextAlternates> {
    type Found = Option<vectorcraft_text::ContextAlternates>;
    let state = app.session.active();
    let key = state.map(|s| (s.uid, s.revision, id, byte, face_id));
    let slot = ui.scope_id().with("gl-alternates-found");
    if let Some(key) = key
        && let Some((cached_key, found)) = ui.ctx().data(|d| d.get_temp::<((u64, u64, NodeId, usize, u32), Found)>(slot))
        && cached_key == key
    {
        return found;
    }
    let found = state.and_then(|s| s.doc.node(id)).and_then(|node| match &node.kind {
        NodeKind::Text(text) => vectorcraft_text::alternates_in_context(FontDb::global(), text, byte),
        _ => None,
    });
    if let Some(key) = key {
        ui.ctx().data_mut(|d| d.insert_temp(slot, (key, found.clone())));
    }
    found
}

/// A square [ − ] or [ + ] beside the font menus. `label` is the mark itself (not translated).
fn zoom_button(ui: &mut Ui, id: &str, label: &str, tip: &str, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, ui.scope_id().with(("gl-zoom", id)), sense);
    let bg = if enabled && resp.hovered() { t.hover } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, 2, bg);
    let border = if enabled { t.button_border } else { t.border };
    ui.painter().rect_stroke(rect.shrink(0.5), 2, Stroke::new(1.0, border), StrokeKind::Inside);
    let color = if enabled { t.text_strong } else { t.text_disabled };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(14.0), color);
    resp.on_hover_text(tl!(tip)).clicked() && enabled
}

/// The menu label of East Asian glyph form `form` ([`GlyphForm::ALL`]).
fn glyph_form_label(form: GlyphForm) -> &'static str {
    match form {
        GlyphForm::Traditional => "Traditional Forms",
        GlyphForm::Expert => "Expert Forms",
        GlyphForm::Jis04 => "JIS 04 Forms",
        GlyphForm::Jis90 => "JIS 90 Forms",
        GlyphForm::Jis78 => "JIS 78 Forms",
        GlyphForm::Jis83 => "JIS 83 Forms",
        GlyphForm::FullWidth => "Monospaced Full-Width Forms",
        GlyphForm::HalfWidth => "Monospaced Half-Width Forms",
        GlyphForm::ThirdWidth => "Monospaced Third-Width Forms",
        GlyphForm::QuarterWidth => "Monospaced Quarter-Width Forms",
        GlyphForm::Default => "Revert To Default Forms",
    }
}

/// The ids of the faces that draw the characters [`glyph_form_items`] acts on: the faces the
/// fonts of their styles name, and the faces layout draws them with, fallback faces included:
/// the Type tool's selected range (every frame of its story after Select All in threaded text,
/// the whole text object at a caret), else every frame of the selected stories. Laid out once for
/// each document revision, selection and set of fonts while the menu is open, overflow text
/// included.
fn selection_faces(app: &VectorcraftApp, ui: &Ui) -> Vec<u32> {
    type Key = (u64, u64, u64, Option<(NodeId, usize, usize)>, Vec<NodeId>);
    let editing = super::character::text_editing(app).map(|(id, a, b)| (id, a.min(b), a.max(b)));
    let Some(state) = app.session.active() else { return vec![] };
    let story = super::character::text_story(app);
    let stories = match editing {
        Some(_) => story.clone().unwrap_or_default(),
        None => super::character::selected_stories(app),
    };
    let db = FontDb::global();
    let key: Key = (state.uid, state.revision, db.generation(), editing, stories);
    let slot = ui.scope_id().with("gl-form-faces");
    if let Some((cached, ids)) = ui.ctx().data(|d| d.get_temp::<(Key, Vec<u32>)>(slot))
        && cached == key
    {
        return ids;
    }
    let mut ids: Vec<u32> =
        super::character::selected_fonts(app).iter().filter_map(|(family, style)| db.face(family, style)).map(|f| f.id()).collect();
    let mut add = |t: &vectorcraft_doc::TextObject, range: Option<(usize, usize)>| {
        // Laid out as point type, so that text overflowing its area or path counts too.
        let mut all = t.clone();
        all.kind = vectorcraft_doc::TextKind::Point;
        for g in vectorcraft_text::layout(db, &all).glyphs {
            // A glyph counts when its characters overlap the range (a ligature may start before it).
            if range.is_none_or(|(a, b)| g.byte < b && g.byte + g.len.max(1) > a) && !ids.contains(&g.font_id) {
                ids.push(g.font_id);
            }
        }
    };
    let text = |id: NodeId| match state.doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    };
    match (editing, story) {
        // After Select All in threaded text: every frame of the story.
        (Some(_), Some(frames)) => frames.into_iter().filter_map(text).for_each(|t| add(t, None)),
        (Some((id, a, b)), None) => {
            if let Some(t) = text(id) {
                // A caret applies the form to all the text of the object.
                add(t, (a < b).then_some((a, b)));
            }
        }
        (None, _) => super::character::selected_story_texts(app).into_iter().for_each(|t| add(t, None)),
    }
    ui.ctx().data_mut(|d| d.insert_temp(slot, (key, ids.clone())));
    ids
}

/// The value `f` gives every character [`glyph_form_items`] acts on: the Type tool's selected
/// range, all the text of the object at a caret (the menu formats all of it there), else every
/// frame of the selected stories. `None` where they differ, or with no type.
fn selection_shared<T: PartialEq>(app: &VectorcraftApp, f: impl Fn(&CharStyle) -> T) -> Option<T> {
    match super::character::text_editing(app) {
        Some((id, a, b)) if b <= a && super::character::text_story(app).is_none() => {
            let NodeKind::Text(t) = &app.session.active()?.doc.node(id)?.kind else { return None };
            let mut acc = super::character::Shared::None;
            for r in &t.runs {
                acc.add(f(&r.style));
            }
            acc.value()
        }
        _ => super::character::shared(app, f),
    }
}

/// The East Asian glyph form every character [`glyph_form_items`] acts on has ([`selection_shared`]).
fn selection_glyph_form(app: &VectorcraftApp) -> Option<GlyphForm> {
    selection_shared(app, |s| s.glyph_form)
}

/// Does choosing `form` change the characters [`glyph_form_items`] acts on? Not when they all have
/// it already, unless one of them has a glyph choice, which a form other than the default takes
/// away.
fn glyph_form_changes(app: &VectorcraftApp, form: GlyphForm) -> bool {
    selection_glyph_form(app) != Some(form) || (form != GlyphForm::Default && selection_shared(app, |s| s.glyph_alternate.is_none()) != Some(true))
}

/// The East Asian glyph forms of the panel menu, with Show East Asian Options: each draws the
/// selected characters (all the text of the selected type objects without a Type tool range) in
/// one form of their font, and is checked when they all have it; a form none of the faces that
/// draw the selected text has a feature for ([`selection_faces`]) is disabled. Revert To Default
/// Forms draws the font's default glyphs.
/// Public behavior: [Format East Asian characters](https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html)
/// (Replace Asian characters with a different glyph form).
fn glyph_form_items(app: &mut VectorcraftApp, ui: &mut Ui) {
    let has = super::character::text_style(app).is_some();
    let db = FontDb::global();
    let faces: Vec<_> = selection_faces(app, ui).into_iter().filter_map(|id| db.face_by_id(id)).collect();
    let current = selection_glyph_form(app);
    for form in GlyphForm::ALL {
        // A width form may be drawn with other widths for the same glyphs (`GPOS`) instead of
        // other glyphs.
        let enabled = match form.feature() {
            Some(tag) => faces.iter().any(|f| f.has_substitution_feature(*tag) || (form.is_width() && f.has_positioning_feature(*tag))),
            None => has,
        };
        let checked = form != GlyphForm::Default && current == Some(form);
        if form == GlyphForm::Default {
            ui.separator();
        }
        // Choosing the form every selected character already has changes nothing.
        if menu_item(ui, tl!(glyph_form_label(form)), enabled, checked) && glyph_form_changes(app, form) {
            super::character::format(app, json!({"glyphForm": form.id()}));
        }
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if menu_item(ui, tl!("Use the Selected Text's Font"), true, false) {
        set_pstate::<Option<(String, String)>>(ui.ctx(), "gl-font", None);
    }
    ui.separator();
    if app.session.prefs.show_east_asian_options {
        glyph_form_items(app, ui);
        ui.separator();
    }
    let (_, _, face) = browsed_face(app, ui.ctx());
    let cats = categories(face.as_deref());
    let (cur, category) = shown(ui.ctx(), &cats);
    for (i, (name, _, _)) in SUBSETS.iter().enumerate() {
        if menu_item(ui, tl!(name), true, cur == i) {
            set_pstate(ui.ctx(), "gl-subset", i);
        }
    }
    if menu_item(ui, tl!("Alternates for Current Selection"), true, cur == SUBSETS.len()) {
        set_pstate(ui.ctx(), "gl-subset", SUBSETS.len());
    }
    if !cats.is_empty() {
        ui.separator();
    }
    for (i, (id, label)) in cats.iter().enumerate() {
        if menu_item(ui, label, true, category == Some(i)) {
            show_category(ui.ctx(), id);
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
        squares(shapes).iter().map(|r| r.width()).collect()
    }

    /// The square glyph previews, in the order they are drawn.
    fn squares(shapes: &[Shape]) -> Vec<Rect> {
        fn walk(s: &Shape, out: &mut Vec<Rect>) {
            match s {
                Shape::Mesh(m) => {
                    let r = m.calc_bounds();
                    if (r.width() - r.height()).abs() < 1.0 && r.width() >= 14.0 {
                        out.push(r);
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

    /// The panel menu's texts and where each is drawn.
    fn menu_texts(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, Pos2)> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 420.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| menu(app, ui));
        out.textures_delta.clear();
        text_anchors(&out.shapes.into_iter().map(|c| c.shape).collect::<Vec<_>>())
    }

    /// Choose `label` in the panel menu (its items start with a check mark or spaces).
    fn choose(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) {
        let items = menu_texts(ctx, app, vec![]);
        let item = items.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == label);
        let at = item.unwrap_or_else(|| panic!("{label} missing in {items:?}")).1 + vec2(16.0, 4.0);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        menu_texts(ctx, app, vec![egui::Event::PointerMoved(at), button(true)]);
        menu_texts(ctx, app, vec![button(false)]);
    }

    fn texts(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<String> {
        text_anchors(&shapes(ctx, app, vec![])).into_iter().map(|(t, _)| t).collect()
    }

    /// With Show East Asian Options, the panel menu lists the East Asian glyph forms; choosing one
    /// draws the Type tool's selected characters in it, checks it, and Revert To Default Forms
    /// takes it away. A form the font has no feature for is disabled. The second half needs
    /// craft-fonts' BIZ UDMincho, which has JIS 78 forms and no JIS 04 forms (skipped without it).
    #[test]
    fn east_asian_glyph_forms_in_the_panel_menu_set_the_selected_characters_form() {
        let (mut app, id) = editing("亜唖");
        let ctx = egui::Context::default();
        let labels = |app: &mut VectorcraftApp| menu_texts(&ctx, app, vec![]).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
        let plain = |l: &String| l.trim_start_matches(['✓', ' ']).to_string();
        app.session.prefs.show_east_asian_options = false;
        assert!(!labels(&mut app).iter().map(plain).any(|l| l == "JIS 78 Forms"), "hidden without Show East Asian Options");
        app.session.prefs.show_east_asian_options = true;
        let shown: Vec<String> = labels(&mut app).iter().map(plain).collect();
        for form in GlyphForm::ALL {
            assert!(shown.iter().any(|l| l == glyph_form_label(form)), "{form:?} in {shown:?}");
        }
        const FAMILY: &str = "BIZ UDMincho";
        if FontDb::global().face(FAMILY, "Regular").is_none_or(|f| f.family != FAMILY) {
            eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
            return;
        }
        app.run("text.setRangeStyle", json!({"id": id, "font": FAMILY})).unwrap();
        let forms = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap_or(0))).unwrap().kind {
            NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.glyph_form)).collect::<Vec<_>>(),
            _ => panic!("text"),
        };
        select(&mut app, 3, 6);
        choose(&ctx, &mut app, "JIS 78 Forms");
        assert_eq!(forms(&app), [("亜".to_string(), GlyphForm::Default), ("唖".to_string(), GlyphForm::Jis78)]);
        assert!(labels(&mut app).iter().any(|l| l.starts_with('✓') && l.ends_with("JIS 78 Forms")), "checked");
        // A selection whose first font has no JIS 83 forms, and whose second one does, takes them.
        let fallback = vectorcraft_text::FALLBACK_FAMILY;
        if FontDb::global().face(fallback, "Regular").is_some_and(|f| !f.has_substitution_feature(*b"jp83")) {
            app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 3, "font": fallback})).unwrap();
            select(&mut app, 0, 6);
            choose(&ctx, &mut app, "JIS 83 Forms");
            assert!(forms(&app).iter().all(|(_, f)| *f == GlyphForm::Jis83), "{:?}", forms(&app));
            app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 3, "font": FAMILY})).unwrap();
            app.run("text.setRangeStyle", json!({"id": id, "glyphForm": "default"})).unwrap();
            select(&mut app, 3, 6);
            choose(&ctx, &mut app, "JIS 78 Forms");
        }
        // Choosing it again changes nothing, and the font has no JIS 04 forms: that item is disabled.
        let n = app.session.active().unwrap().history.undo.len();
        choose(&ctx, &mut app, "JIS 78 Forms");
        choose(&ctx, &mut app, "JIS 04 Forms");
        assert_eq!(app.session.active().unwrap().history.undo.len(), n);
        assert_eq!(forms(&app)[1].1, GlyphForm::Jis78);
        choose(&ctx, &mut app, "Revert To Default Forms");
        assert!(forms(&app).iter().all(|(_, f)| *f == GlyphForm::Default), "{:?}", forms(&app));
    }

    /// At a caret the glyph form items act on all the text of the object, so they are checked, and
    /// skipped as changing nothing, by every character's form, not the caret's: with the caret in
    /// text of the default form and other text in JIS 78 forms, Revert To Default Forms reverts
    /// that text.
    #[test]
    fn glyph_form_items_at_a_caret_follow_all_the_text() {
        let (mut app, id) = editing("亜唖");
        app.session.prefs.show_east_asian_options = true;
        let ctx = egui::Context::default();
        let node = NodeId(id.as_u64().unwrap_or(0));
        let forms = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(node).unwrap().kind {
            NodeKind::Text(t) => t.runs.iter().map(|r| r.style.glyph_form).collect::<Vec<_>>(),
            _ => panic!("text"),
        };
        app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 3, "glyphForm": "jis78"})).unwrap();
        select(&mut app, 6, 6);
        assert_eq!(selection_glyph_form(&app), None, "the caret's form is not every character's");
        choose(&ctx, &mut app, "Revert To Default Forms");
        assert!(forms(&app).iter().all(|f| *f == GlyphForm::Default), "{:?}", forms(&app));
        app.run("text.setRangeStyle", json!({"id": id, "glyphForm": "jis78"})).unwrap();
        select(&mut app, 6, 6);
        assert_eq!(selection_glyph_form(&app), Some(GlyphForm::Jis78));
    }

    /// Choosing the form every character already has is skipped as changing nothing, unless a
    /// character has a glyph choice: a form other than the default takes it away, and Revert To
    /// Default Forms keeps it.
    #[test]
    fn a_glyph_choice_lets_the_current_form_be_chosen_again() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
        FontDb::global().add_font(alternates_font(false).expect("the test font builds"));
        let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
        let (mut app, id) = editing("xay");
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(false), "glyphForm": "jis78"})).unwrap();
        select(&mut app, 3, 3);
        assert!(!glyph_form_changes(&app, GlyphForm::Jis78), "every character has it");
        assert!(glyph_form_changes(&app, GlyphForm::Default));
        app.run("text.setGlyphAlternate", json!({"id": id, "start": 1, "gid": u1})).unwrap();
        select(&mut app, 3, 3);
        assert!(glyph_form_changes(&app, GlyphForm::Jis78), "it takes away the glyph choice");
        app.run("text.setRangeStyle", json!({"id": id, "glyphForm": "default"})).unwrap();
        select(&mut app, 3, 3);
        assert!(!glyph_form_changes(&app, GlyphForm::Default), "Revert To Default Forms keeps the glyph choice");
    }

    /// Show › Alternates for Current Selection lists the selected character's alternates from the
    /// font of its text, whatever font the panel browses; a double-click draws the character with
    /// one. A character without alternates, no selected character and a missing font each show a
    /// note, and Show stays on the alternates.
    #[test]
    fn alternates_for_current_selection_lists_and_replaces_the_selected_character() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
        let (mut app, id) = editing("xay");
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        select(&mut app, 2, 2);
        let ctx = egui::Context::default();
        let t = Tokens::get(&ctx);
        let entire = squares(&shapes(&ctx, &mut app, vec![])).len();
        choose(&ctx, &mut app, "Alternates for Current Selection");
        assert_eq!(pstate::<usize>(&ctx, "gl-subset"), SUBSETS.len());
        let all = texts(&ctx, &mut app);
        assert!(all.iter().any(|s| s == "Alternates for Current Selection"), "the Show menu names it: {all:?}");
        assert!(all.iter().any(|s| s == &format!("{} Regular  ·  7 glyphs", alternates_family(true))), "the font and the count: {all:?}");
        let found = shapes(&ctx, &mut app, vec![]);
        let cells = squares(&found);
        assert_eq!(cells.len(), 7, "a, its six alternates and nothing else (the whole font drew {entire})");
        assert_eq!(highlighted(&found, &t), [cells[0].center()], "the default glyph is the one drawn");
        app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "allCaps": true})).unwrap();
        let caps = shapes(&ctx, &mut app, vec![]);
        assert!(highlighted(&caps, &t).is_empty(), "All Caps draws a different glyph, so no alternate of a is highlighted");
        app.run("text.setRangeStyle", json!({"id": id, "start": 1, "end": 2, "allCaps": false})).unwrap();
        let found = shapes(&ctx, &mut app, vec![]);
        assert_eq!(highlighted(&found, &t), [squares(&found)[0].center()]);

        // The panel's own font does not change what is listed.
        set_pstate(&ctx, "gl-font", Some(("Source Serif 4".to_string(), "Regular".to_string())));
        assert_eq!(squares(&shapes(&ctx, &mut app, vec![])).len(), 7);
        set_pstate(&ctx, "gl-font", Some(("Unavailable Panel Font".to_string(), "Regular".to_string())));
        assert_eq!(squares(&shapes(&ctx, &mut app, vec![])).len(), 7, "the selected text's face still supplies alternates");

        // A double-click on the second cell draws the a with that glyph, as one undo step.
        let at = cells[1].center();
        let undo = app.session.active().unwrap().history.undo.len();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut time = 10.0;
        let mut timed = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.05;
            let screen = Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 420.0)));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: screen, time: Some(time), events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        timed(&mut app, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            timed(&mut app, vec![button(true)]);
            timed(&mut app, vec![button(false)]);
        }
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "status: {}", app.ui.status);
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().kind else {
            panic!("text")
        };
        assert_eq!(text.plain_text(), "xay");
        let gids: Vec<u32> = vectorcraft_text::layout(FontDb::global(), text).glyphs.iter().map(|g| g.gid).collect();
        assert_eq!(gids[1], u1, "the a is drawn with the chosen glyph");
        shapes(&ctx, &mut app, vec![egui::Event::PointerMoved(Pos2::ZERO)]);
        let found = shapes(&ctx, &mut app, vec![]);
        assert_eq!(highlighted(&found, &t), [squares(&found)[1].center()], "the chosen glyph is highlighted");

        select(&mut app, 1, 1);
        let all = texts(&ctx, &mut app);
        assert!(all.iter().any(|s| s == "No alternates for the selected character."), "{all:?}");
        assert_eq!(pstate::<usize>(&ctx, "gl-subset"), SUBSETS.len(), "Show stays on the alternates");
        assert!(squares(&shapes(&ctx, &mut app, vec![])).is_empty());

        app.run("text.setRangeStyle", json!({"id": id, "font": "No Such Font Family"})).unwrap();
        select(&mut app, 2, 2);
        assert!(texts(&ctx, &mut app).iter().any(|s| s == "The selected character's font is not available."));

        app.session.select_tool("select", Default::default()).unwrap();
        assert!(texts(&ctx, &mut app).iter().any(|s| s == "Select a character with the Type tool to see its alternates."));

        // Back to Entire Font: the font's characters again.
        choose(&ctx, &mut app, "Entire Font");
        assert_eq!(pstate::<usize>(&ctx, "gl-subset"), 0);
        assert!(squares(&shapes(&ctx, &mut app, vec![])).len() > 7);
    }

    /// The alternates and the glyph drawn are found once per document revision: a change that does
    /// not count as an edit leaves the grid as it was, and the next revision finds them again.
    #[test]
    fn alternates_are_found_once_per_revision() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
        let (mut app, id) = editing("xay");
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        select(&mut app, 2, 2);
        let ctx = egui::Context::default();
        let t = Tokens::get(&ctx);
        choose(&ctx, &mut app, "Alternates for Current Selection");
        let found = shapes(&ctx, &mut app, vec![]);
        let cells = squares(&found);
        assert_eq!(highlighted(&found, &t), [cells[0].center()], "the default glyph is drawn");
        let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
        let node = vectorcraft_doc::NodeId(id.as_u64().unwrap());
        let state = app.session.active_mut().unwrap();
        let Some(NodeKind::Text(text)) = std::sync::Arc::make_mut(&mut state.doc).node_mut(node).map(|n| &mut n.kind) else { panic!("text") };
        vectorcraft_text::edit::style_range(&mut text.runs, 1, 2, |st| st.glyph_alternate = Some(face.choice('a', u1)));
        let found = shapes(&ctx, &mut app, vec![]);
        assert_eq!(highlighted(&found, &t), [cells[0].center()], "the same revision keeps what was found");
        app.session.active_mut().unwrap().revision += 1;
        let found = shapes(&ctx, &mut app, vec![]);
        assert_eq!(highlighted(&found, &t), [cells[1].center()], "the next revision finds the chosen glyph");
    }

    /// A font whose `aalt` does not list the default glyph (here no `aalt` at all) still shows it,
    /// first in the grid, so a chosen alternate can be put back.
    #[test]
    fn the_default_glyph_is_listed_when_the_font_leaves_it_out() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
        FontDb::global().add_font(alternates_font(false).expect("the test font builds"));
        let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
        let (mut app, id) = editing("xay");
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(false)})).unwrap();
        select(&mut app, 2, 2);
        let ctx = egui::Context::default();
        let t = Tokens::get(&ctx);
        choose(&ctx, &mut app, "Alternates for Current Selection");
        let found = shapes(&ctx, &mut app, vec![]);
        let cells = squares(&found);
        assert_eq!(cells.len(), 7, "the default glyph and the six alternates");
        assert_eq!(highlighted(&found, &t), [cells[0].center()], "the default glyph is first and drawn");
        app.run("text.setGlyphAlternate", json!({"id": id, "start": 1, "gid": u1})).unwrap();
        let found = shapes(&ctx, &mut app, vec![]);
        assert_eq!(squares(&found).len(), 7);
        assert_eq!(highlighted(&found, &t), [squares(&found)[5].center()], "the chosen glyph, after b, u2, u4 and u3");
    }

    /// Double-click the glyph cell at `at`.
    fn double_click(ctx: &egui::Context, app: &mut VectorcraftApp, at: Pos2) {
        double_click_in(ctx, app, at, vec2(280.0, 420.0));
    }

    /// Double-click the glyph cell at `at` in a panel `size` large.
    fn double_click_in(ctx: &egui::Context, app: &mut VectorcraftApp, at: Pos2, size: egui::Vec2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        // Time only moves forward in one egui context.
        let mut time = ctx.input(|i| i.time) + 10.0;
        let mut timed = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.05;
            let screen = Some(Rect::from_min_size(Pos2::ZERO, size));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: screen, time: Some(time), events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        timed(app, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            timed(app, vec![button(true)]);
            timed(app, vec![button(false)]);
        }
    }

    /// The glyph ids of the text `id`, in the order of its characters.
    fn glyph_ids(app: &VectorcraftApp, id: &serde_json::Value) -> Vec<u32> {
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        let mut glyphs = vectorcraft_text::layout(FontDb::global(), text).glyphs;
        glyphs.sort_by_key(|g| g.byte);
        glyphs.iter().map(|g| g.gid).collect()
    }

    /// For the selected medial letter of an Arabic word the grid lists that form's alternates, and
    /// a double-click applies the feature that draws one to that letter alone, as one undo step:
    /// the word keeps its joining forms and the text its characters; undo takes it back.
    #[test]
    fn a_double_click_applies_a_feature_value_to_one_letter_of_a_joined_word() {
        use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
        FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
        let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
        let (mut app, id) = editing("\u{628}\u{62A}\u{646}");
        app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
        assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
        select(&mut app, 2, 4);
        let ctx = egui::Context::default();
        let t = Tokens::get(&ctx);
        choose(&ctx, &mut app, "Alternates for Current Selection");
        let found = shapes(&ctx, &mut app, vec![]);
        let cells = squares(&found);
        assert_eq!(cells.len(), 4, "the medial teh and its three alternates, not the isolated form's");
        assert_eq!(highlighted(&found, &t), [cells[0].center()], "the form the letter has is first and drawn");
        let undo = app.session.active().unwrap().history.undo.len();
        double_click(&ctx, &mut app, cells[2].center());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step; status: {}", app.ui.status);
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        assert_eq!(text.plain_text(), "\u{628}\u{62A}\u{646}", "the text keeps its characters");
        let features: Vec<(String, Vec<String>)> = text.runs.iter().map(|r| (r.text.clone(), r.style.features.clone())).collect();
        assert_eq!(
            features,
            [("\u{628}".to_string(), vec![]), ("\u{62A}".to_string(), vec!["salt=2".to_string()]), ("\u{646}".to_string(), vec![])],
            "the feature is on exactly that letter"
        );
        assert!(text.runs.iter().all(|r| r.style.glyph_alternate.is_none()));
        assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')], "the neighbours keep their initial and final forms");
        // The pointer leaves the cell: a hovered cell is painted as hovered, not as the drawn one.
        shapes(&ctx, &mut app, vec![egui::Event::PointerMoved(Pos2::ZERO)]);
        let found = shapes(&ctx, &mut app, vec![]);
        assert_eq!(highlighted(&found, &t), [squares(&found)[2].center()], "the applied variant is the one drawn");
        // The default glyph takes the feature off again, and undo takes each step back.
        double_click(&ctx, &mut app, squares(&found)[0].center());
        assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(glyph_ids(&app, &id), [g('B'), g('u'), g('M')]);
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        assert!(text.runs.iter().all(|r| r.style.features.is_empty()), "undo restores the features");
    }

    /// A variant no feature draws is still enabled: it replaces the glyph (`text.setGlyphAlternate`).
    #[test]
    fn a_variant_no_feature_draws_replaces_the_glyph() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let (mut app, id) = editing("xay");
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        select(&mut app, 2, 2);
        let ctx = egui::Context::default();
        choose(&ctx, &mut app, "Alternates for Current Selection");
        let cells = squares(&shapes(&ctx, &mut app, vec![]));
        let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
        // The last cell is cv01's glyph c, which the style has no tag for.
        double_click(&ctx, &mut app, cells[cells.len() - 1].center());
        assert_eq!(glyph_ids(&app, &id)[1], face.glyph_for('c'));
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        assert!(text.runs.iter().any(|r| r.style.glyph_alternate.is_some()));
        // A feature variant afterwards takes the choice away in the same step.
        let found = shapes(&ctx, &mut app, vec![]);
        let undo = app.session.active().unwrap().history.undo.len();
        double_click(&ctx, &mut app, squares(&found)[2].center());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        assert!(text.runs.iter().all(|r| r.style.glyph_alternate.is_none()), "the choice is gone");
    }

    /// A positional alternate without a supported feature still goes through the engine's glyph
    /// choice validation, even though the isolated character has different alternates.
    #[test]
    fn a_positional_variant_no_feature_draws_replaces_the_glyph() {
        use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
        FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
        let face = FontDb::global().face(JOINING_ALTERNATES_FAMILY, "Regular").expect("the test face");
        let gid = u32::from(source_sans_gid('d').expect("a glyph"));
        assert!(face.alternates('\u{646}').is_empty(), "the isolated noon has no alternates");
        assert!(!face.is_unlisted_alternate('\u{646}', gid), "cv01 reaches the glyph");
        for view in ["Alternates for Current Selection", "Entire Font"] {
            let (mut app, id) = editing("\u{628}\u{646}\u{62A}");
            app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
            set_cell_index(&mut app, 0);
            let plain = glyph_ids(&app, &id);
            select(&mut app, 2, 4);
            let ctx = egui::Context::default();
            choose(&ctx, &mut app, view);
            let at = if view == "Entire Font" {
                entire_font_cell(&ctx, &mut app, &face, gid).0
            } else {
                let cells = squares(&shapes(&ctx, &mut app, vec![]));
                assert_eq!(cells.len(), 2, "the medial noon and its cv01 alternate");
                cells[1].center()
            };
            let undo = app.session.active().unwrap().history.undo.len();
            double_click_in(&ctx, &mut app, at, if view == "Entire Font" { TALL } else { vec2(280.0, 420.0) });
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "{view}: {}", app.ui.status);
            assert_eq!(glyph_ids(&app, &id)[1], gid, "{view}");
            let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
            assert_eq!(text.plain_text(), "\u{628}\u{646}\u{62A}", "the characters stay unchanged");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(glyph_ids(&app, &id), plain, "{view}: undo restores the word");
        }
    }

    /// The corner triangles of the grid cells whose glyph has alternates: right-angled, with the
    /// right angle at the cell's lower right corner.
    fn corner_triangles(shapes: &[Shape]) -> Vec<Rect> {
        fn walk(s: &Shape, out: &mut Vec<Rect>) {
            match s {
                Shape::Path(p) if p.points.len() == 3 && p.points[0].x == p.points[1].x && p.points[1].y == p.points[2].y => {
                    out.push(Rect::from_points(&p.points));
                }
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = vec![];
        shapes.iter().for_each(|s| walk(s, &mut out));
        out
    }

    /// The Type tool editing `text` set in the alternates test font, the panel on Basic Latin, and
    /// the egui context it is drawn in.
    fn basic_latin_of_the_alternates_font(text: &str) -> (VectorcraftApp, serde_json::Value, egui::Context) {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let (mut app, id) = editing(text);
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        let ctx = egui::Context::default();
        set_pstate(&ctx, "gl-subset", 1usize);
        (app, id, ctx)
    }

    /// Click the one corner triangle of the grid, and return the glyph cells of the pop-up it
    /// opens (drawn after the panel, on top of it).
    fn open_corner_popup(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<Rect> {
        let before = squares(&shapes(ctx, app, vec![]));
        let corners = corner_triangles(&shapes(ctx, app, vec![]));
        assert_eq!(corners.len(), 1, "only a has alternates among Basic Latin: {corners:?}");
        click(ctx, app, corners[0].center() + vec2(1.0, 1.0));
        let after = squares(&shapes(ctx, app, vec![]));
        assert!(after.len() > before.len(), "the pop-up drew its glyphs: {} then {}", before.len(), after.len());
        after[after.len() - (after.len() - before.len())..].to_vec()
    }

    /// A cell whose glyph has alternates has a corner triangle; a click on it opens a pop-up of the
    /// glyph and its alternates in the font's order, and picking an alternate inserts the
    /// character at the caret drawn with it, as one undo step.
    #[test]
    fn a_cells_corner_triangle_opens_its_alternates_and_a_pick_inserts_one() {
        use vectorcraft_text::test_fonts::{alternates_family, unencoded_glyphs};
        let [u1, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
        let (mut app, id, ctx) = basic_latin_of_the_alternates_font("xy");
        let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
        let a = face.glyph_for('a');
        assert!(face.has_alternates(a));
        let drawn = shapes(&ctx, &mut app, vec![]);
        let (cells, triangle) = (squares(&drawn), corner_triangles(&drawn));
        assert_eq!(triangle.len(), 1);
        assert!(
            cells.iter().any(|c| c.contains(triangle[0].center()) && (c.max - triangle[0].max).length() < 2.0),
            "in a cell's lower right corner: {triangle:?} among {cells:?}"
        );
        select(&mut app, 1, 1);
        let popup = open_corner_popup(&ctx, &mut app);
        assert_eq!(popup.len(), 7, "a and its six alternates");
        let plain = glyph_ids(&app, &id);
        let undo = app.session.active().unwrap().history.undo.len();
        // a, u1, b, u2: the fourth cell is ss01's u2.
        click(&ctx, &mut app, popup[3].center());
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        assert_eq!(text.plain_text(), "xay", "status: {}", app.ui.status);
        assert_eq!(glyph_ids(&app, &id), [plain[0], u2, plain[1]], "the inserted a is drawn with the pick");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(glyph_ids(&app, &id), plain, "undo takes the character back out");
        // The cell's own glyph inserts the plain character, as a double-click does.
        select(&mut app, 1, 1);
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[0].center());
        assert_eq!(glyph_ids(&app, &id), [plain[0], a, plain[1]]);
        // An alternate with no feature that draws it here (u1 is listed by aalt) still goes on.
        select(&mut app, 2, 2);
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[1].center());
        assert_eq!(glyph_ids(&app, &id), [plain[0], a, u1, plain[1]], "status: {}", app.ui.status);
    }

    /// With the Type tool's selection exactly the cell's character, a pick draws that character
    /// with the glyph and keeps the text; picking the glyph it already has adds no undo step.
    /// Without the Type tool, and with text in another font, the status says why and nothing
    /// changes.
    #[test]
    fn a_pick_replaces_the_selected_character_and_says_why_when_it_cannot() {
        use vectorcraft_text::test_fonts::{alternates_family, unencoded_glyphs};
        let [_, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
        let (mut app, id, ctx) = basic_latin_of_the_alternates_font("xay");
        let plain = glyph_ids(&app, &id);
        select(&mut app, 1, 2);
        let undo = app.session.active().unwrap().history.undo.len();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        assert_eq!(text.plain_text(), "xay", "the selected a stays: status {}", app.ui.status);
        assert_eq!(glyph_ids(&app, &id), [plain[0], u2, plain[2]]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        select(&mut app, 1, 2);
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "the glyph it already has changes nothing");
        assert_eq!(glyph_ids(&app, &id), [plain[0], u2, plain[2]]);

        // Text set in another font than the one the panel browses.
        let family = alternates_family(true);
        set_pstate(&ctx, "gl-font", Some((family.to_string(), "Regular".to_string())));
        app.run("text.setRangeStyle", json!({"id": id, "font": vectorcraft_text::FALLBACK_FAMILY})).unwrap();
        select(&mut app, 3, 3);
        let undo = app.session.active().unwrap().history.undo.len();
        let before = glyph_ids(&app, &id);
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        assert_eq!(glyph_ids(&app, &id), before);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(app.ui.status.contains("not of the font the selected text is set in"), "{}", app.ui.status);

        // No Type tool caret.
        app.session.select_tool("select", Default::default()).unwrap();
        app.ui.status.clear();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        assert_eq!(glyph_ids(&app, &id), before);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(app.ui.status, "Select text or click into text with the Type tool to insert glyphs");
    }

    /// A pick the inserted character cannot be drawn with (All Caps keeps a glyph choice from
    /// being drawn) takes the character back out and keeps the status that says why: on its own,
    /// and inside an undo group that is already open, which stays open with its earlier edit and
    /// takes a pick that can be drawn in place of the selection.
    #[test]
    fn a_pick_that_cannot_be_drawn_leaves_the_text_as_it_was() {
        let [_, u2, ..] = vectorcraft_text::test_fonts::unencoded_glyphs().expect("unencoded glyphs");
        let (mut app, id, ctx) = basic_latin_of_the_alternates_font("xy");
        let text = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("text"),
        };
        app.run("text.setFormat", json!({"ids": [id], "allCaps": true})).unwrap();
        // A step to redo, which the failed pick keeps.
        app.run("text.setFormat", json!({"ids": [id], "underline": true})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        select(&mut app, 1, 1);
        app.ui.status.clear();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[1].center());
        assert_eq!(text(&app), "xy");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "no undo step is left");
        assert_eq!(app.session.active().unwrap().history.redo.len(), 1, "the redo step stays");
        assert!(!app.ui.status.is_empty(), "the status says why");

        // In an open group with no edit yet, the failed pick leaves the group with nothing to undo.
        app.run("edit.undoGroup.begin", json!({})).unwrap();
        select(&mut app, 0, 1);
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[1].center());
        assert_eq!(text(&app), "xy");
        assert!(!app.ui.status.is_empty(), "the status says why");
        app.run("edit.undoGroup.end", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "the empty group adds no step");

        app.run("edit.undoGroup.begin", json!({})).unwrap();
        app.run("text.setFormat", json!({"ids": [id], "underline": true})).unwrap();
        select(&mut app, 0, 1);
        app.ui.status.clear();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[1].center());
        assert_eq!(text(&app), "xy", "the replaced x is back: status {}", app.ui.status);
        assert!(!app.ui.status.is_empty(), "the status says why");
        assert!(app.session.active().unwrap().undo_group.is_some(), "the group stays open");
        // A pick that can be drawn replaces the selection inside the group.
        app.run("text.setFormat", json!({"ids": [id], "allCaps": false})).unwrap();
        select(&mut app, 0, 1);
        let plain = glyph_ids(&app, &id);
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        assert_eq!(text(&app), "ay", "status {}", app.ui.status);
        assert_eq!(glyph_ids(&app, &id), [u2, plain[1]]);
        assert!(app.session.active().unwrap().undo_group.is_some(), "the group stays open");
        app.run("edit.undoGroup.end", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "the group is one step");
    }

    /// After Select All in a threaded story, a pick replaces the whole story's text, as typing
    /// does: the character drawn with the glyph stays, in one undo step.
    #[test]
    fn a_pick_replaces_a_threaded_story_that_select_all_selected() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
        let [_, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        let story = "a bad cab ".repeat(30);
        let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        let c = app.run("text.thread.add", json!({"from": b, "rect": [400, 10, 150, 200]})).unwrap()["id"].as_u64().unwrap();
        for id in [a, b, c] {
            app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        }
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
        app.run("select.all", json!({})).unwrap();
        assert_eq!(super::super::character::text_story(&app).map(|s| s.len()), Some(3), "the whole story is selected");
        let ctx = egui::Context::default();
        set_pstate(&ctx, "gl-subset", 1usize);
        let undo = app.session.active().unwrap().history.undo.len();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        let plain = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("text"),
        };
        let texts: String = [a, b, c].iter().map(|id| plain(&app, *id)).collect();
        assert_eq!(texts, "a", "status: {}", app.ui.status);
        let first = [a, b, c].into_iter().find(|id| !plain(&app, *id).is_empty()).expect("a frame with the character");
        assert_eq!(glyph_ids(&app, &json!(first)), [u2]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// After Select All in a threaded story whose first frame holds only the picked character, a
    /// pick still replaces the whole story's text: the other frames' text goes too.
    #[test]
    fn a_pick_replaces_a_threaded_story_whose_first_frame_holds_one_character() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
        let [_, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        // The first frame is wide enough for the "a" only; the ideographs flow on to the second.
        let area = json!({"x": 10, "y": 10, "text": "a中中中", "size": 12, "area": {"width": 16, "height": 16}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        for id in [a, b] {
            app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        }
        let plain = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("text"),
        };
        assert_eq!((plain(&app, a), plain(&app, b)), ("a".to_string(), "中中中".to_string()));
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
        app.run("select.all", json!({})).unwrap();
        assert_eq!(super::super::character::text_editing(&app), Some((NodeId(a), 0, 1)), "the selection is the one character");
        assert_eq!(super::super::character::text_story(&app).map(|s| s.len()), Some(2), "the whole story is selected");
        let ctx = egui::Context::default();
        set_pstate(&ctx, "gl-subset", 1usize);
        let undo = app.session.active().unwrap().history.undo.len();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        let texts: String = [a, b].iter().map(|id| plain(&app, *id)).collect();
        assert_eq!(texts, "a", "status: {}", app.ui.status);
        assert_eq!(glyph_ids(&app, &json!(a)), [u2]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// A pick at the caret at the end of a full frame of threaded text puts the character in, and
    /// the text flows it on to the next frame: it is drawn with the glyph there, and the Type tool
    /// edits that frame with the caret after it.
    #[test]
    fn a_pick_at_the_end_of_a_full_frame_follows_the_character_to_the_next_frame() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
        let [_, u2, ..] = unencoded_glyphs().expect("unencoded glyphs");
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
        let gid = |c| face.chars().into_iter().find(|(x, _)| *x == c).map(|(_, g)| g).expect("a glyph");
        let pt = |c| face.advance(gid(c)) * 12.0 / face.units_per_em();
        // The first frame holds the line "a xx " and not the next word. Every character is in the
        // test font, so no fallback face measures the line.
        let width = pt('a') + 2.0 * pt(' ') + 2.0 * pt('x') + pt('x') / 2.0;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        let area = json!({"x": 10, "y": 10, "text": "a xx xx", "size": 12, "area": {"width": width, "height": 16}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        for id in [a, b] {
            app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        }
        let plain = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("text"),
        };
        assert_eq!((plain(&app, a), plain(&app, b)), ("a xx ".to_string(), "xx".to_string()));
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let end = "a xx ".len();
        app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": end, "caret": end}));
        let ctx = egui::Context::default();
        set_pstate(&ctx, "gl-subset", 1usize);
        let undo = app.session.active().unwrap().history.undo.len();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[3].center());
        assert_eq!((plain(&app, a), plain(&app, b)), ("a xx ".to_string(), "axx".to_string()), "status: {}", app.ui.status);
        assert_eq!(glyph_ids(&app, &json!(b))[0], u2);
        assert_eq!(super::super::character::text_editing(&app), Some((NodeId(b), 1, 1)), "the caret is after the character");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// A pick on the selected last character of a full frame of threaded text, with a glyph wider
    /// than the one it had, flows the character on to the next frame: the Type tool edits that
    /// frame with the character still selected.
    #[test]
    fn a_wider_pick_on_the_selected_character_moves_the_selection_with_it() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
        let gid = |c| face.chars().into_iter().find(|(x, _)| *x == c).map(|(_, g)| g).expect("a glyph");
        let plain_a = gid('a');
        // The pop-up's glyphs in its order: the cell's own glyph, then the alternates.
        let mut list = vec![plain_a];
        for alt in face.alternates('a') {
            if !list.contains(&alt.gid) {
                list.push(alt.gid);
            }
        }
        let (index, wide) =
            list.iter().copied().enumerate().max_by(|x, y| face.advance(x.1).total_cmp(&face.advance(y.1))).expect("the pop-up lists glyphs");
        assert!(face.advance(wide) > face.advance(plain_a), "an alternate wider than the a");
        // The first frame holds "x a" with the plain a, and not with the wider glyph.
        let pt = |g: u32| face.advance(g) * 12.0 / face.units_per_em();
        let width = pt(gid('x')) + pt(gid(' ')) + (pt(plain_a) + pt(wide)) / 2.0;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        let area = json!({"x": 10, "y": 10, "text": "x a", "size": 12, "area": {"width": width, "height": 16}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        for id in [a, b] {
            app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        }
        let plain = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("text"),
        };
        assert_eq!((plain(&app, a), plain(&app, b)), ("x a".to_string(), String::new()));
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 2, "caret": 3}));
        let ctx = egui::Context::default();
        set_pstate(&ctx, "gl-subset", 1usize);
        let undo = app.session.active().unwrap().history.undo.len();
        let popup = open_corner_popup(&ctx, &mut app);
        click(&ctx, &mut app, popup[index].center());
        assert_eq!((plain(&app, a), plain(&app, b)), ("x ".to_string(), "a".to_string()), "status: {}", app.ui.status);
        assert_eq!(glyph_ids(&app, &json!(b)), [wide]);
        assert_eq!(super::super::character::text_editing(&app), Some((NodeId(b), 0, 1)), "the character stays selected");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// In Entire Font, a glyph no character maps whose substitutions give other glyphs has the
    /// corner triangle too: its pop-up lists it and those glyphs, and a pick goes on the selected
    /// character as a double-click on that glyph's cell would put it.
    #[test]
    fn an_unencoded_glyphs_corner_triangle_applies_its_alternates_to_the_selection() {
        use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font, source_sans_gid};
        FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
        let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
        let (mut app, id) = editing("\u{628}\u{62A}\u{646}");
        app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
        select(&mut app, 2, 4);
        let ctx = egui::Context::default();
        choose(&ctx, &mut app, "Entire Font");
        let face = FontDb::global().face(JOINING_ALTERNATES_FAMILY, "Regular").expect("the test face");
        let medial = g('u');
        assert!(face.unencoded_glyphs().contains(&medial) && face.has_alternates(medial), "the medial teh is unencoded and has alternates");
        // A screen tall enough for every cell of the font.
        let run = |ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 200_000.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes.into_iter().map(|c| c.shape).collect::<Vec<Shape>>()
        };
        let click_at = |ctx: &egui::Context, app: &mut VectorcraftApp, at: Pos2| {
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            run(ctx, app, vec![egui::Event::PointerMoved(at), button(true)]);
            run(ctx, app, vec![button(false)]);
        };
        let drawn = run(&ctx, &mut app, vec![]);
        let (cells, corners) = (squares(&drawn), corner_triangles(&drawn));
        let chars = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).count();
        let unencoded = face.unencoded_glyphs();
        assert_eq!(cells.len(), chars + unencoded.len(), "every glyph has a cell");
        let cell = cells[chars + unencoded.iter().position(|g| *g == medial).expect("an unencoded glyph")];
        let corner = corners.iter().find(|t| cell.contains(t.center())).expect("its corner triangle");
        let undo = app.session.active().unwrap().history.undo.len();
        click_at(&ctx, &mut app, corner.center() + vec2(1.0, 1.0));
        let after = squares(&run(&ctx, &mut app, vec![]));
        let popup = &after[cells.len()..];
        assert_eq!(popup.len(), 4, "the medial teh and its three alternates");
        click_at(&ctx, &mut app, popup[2].center());
        assert_eq!(glyph_ids(&app, &id), [g('B'), g('x'), g('M')], "status: {}", app.ui.status);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// Show › Entire Font also lists the glyphs no character maps.
    #[test]
    fn entire_font_lists_the_unencoded_glyphs() {
        use vectorcraft_text::test_fonts::{JOINING_ALTERNATES_FAMILY, joining_alternates_font};
        FontDb::global().add_font(joining_alternates_font().expect("the test font builds"));
        let (mut app, id) = editing("\u{628}");
        app.run("text.setRangeStyle", json!({"id": id, "font": JOINING_ALTERNATES_FAMILY})).unwrap();
        let face = FontDb::global().face(JOINING_ALTERNATES_FAMILY, "Regular").expect("the test face");
        let encoded = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).count();
        let unencoded = face.unencoded_glyphs();
        assert!(unencoded.len() > 1000 && encoded == 4, "{} unencoded, {encoded} encoded", unencoded.len());
        let ctx = egui::Context::default();
        let all = texts(&ctx, &mut app);
        assert!(all.iter().any(|s| s == &format!("{} glyphs", encoded + unencoded.len())), "{all:?}");
        assert!(face.glyph_name(unencoded[0]).is_some(), "the font names its glyphs");
    }

    /// The panel menu lists the browsed font's OpenType categories after Alternates for Current
    /// Selection, a stylistic set by the font's name for it; choosing one shows the glyphs its
    /// features give and names it in Show. A font without that category shows Entire Font.
    #[test]
    fn the_show_menu_lists_the_fonts_opentype_categories() {
        use vectorcraft_text::test_fonts::{CATEGORIES_FAMILY, HALF_FORM_FAMILY, categories_font, half_form_font};
        FontDb::global().add_font(categories_font().expect("the test font builds"));
        FontDb::global().add_font(half_form_font(false).expect("the test font builds"));
        let (mut app, _) = editing("a");
        let ctx = egui::Context::default();
        set_pstate(&ctx, "gl-font", Some((CATEGORIES_FAMILY.to_string(), "Regular".to_string())));
        let items: Vec<String> = menu_texts(&ctx, &mut app, vec![]).into_iter().map(|(t, _)| t.trim_start_matches(['✓', ' ']).to_string()).collect();
        let at = items.iter().position(|t| t == "Alternates for Current Selection").expect("the alternates item");
        assert_eq!(items[at + 1..], ["Ligatures", "Round Dots", "Small Caps", "Figures"]);
        choose(&ctx, &mut app, "Figures");
        assert_eq!(pstate::<String>(&ctx, "gl-category"), "figures");
        let all = texts(&ctx, &mut app);
        assert!(all.iter().any(|s| s == "2 glyphs") && all.iter().any(|s| s == "Figures"), "{all:?}");
        choose(&ctx, &mut app, "Round Dots");
        let all = texts(&ctx, &mut app);
        assert!(all.iter().any(|s| s == "1 glyph") && all.iter().any(|s| s == "Round Dots"), "{all:?}");
        set_pstate(&ctx, "gl-font", Some((HALF_FORM_FAMILY.to_string(), "Regular".to_string())));
        let all = texts(&ctx, &mut app);
        assert!(all.iter().any(|s| s == "Entire Font") && !all.iter().any(|s| s == "Round Dots"), "{all:?}");
        let items: Vec<String> = menu_texts(&ctx, &mut app, vec![]).into_iter().map(|(t, _)| t.trim_start_matches(['✓', ' ']).to_string()).collect();
        assert_eq!(items.last().map(String::as_str), Some("Alternates for Current Selection"), "{items:?}");
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
    /// The panel tall enough to draw every row of the grid, so each glyph has a cell.
    const TALL: egui::Vec2 = vec2(280.0, 8000.0);

    fn tall_shapes(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<Shape> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, TALL)), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    /// Each glyph preview with the colour it is drawn in, in the order of the grid.
    fn inks(shapes: &[Shape]) -> Vec<(Rect, Color32)> {
        fn walk(s: &Shape, out: &mut Vec<(Rect, Color32)>) {
            match s {
                Shape::Mesh(m) => {
                    let r = m.calc_bounds();
                    if (r.width() - r.height()).abs() < 1.0 && r.width() >= 14.0 {
                        out.push((r, m.vertices.first().map_or(Color32::TRANSPARENT, |v| v.color)));
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

    /// What a double-click on glyph `gid` of `face` would do to the selection ([`Fit::pick`], [`Fit::why`]).
    fn verdict(ctx: &egui::Context, app: &VectorcraftApp, face: &vectorcraft_text::FontFace, gid: u32) -> Result<Pick, String> {
        let mut out = Err(String::new());
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, TALL)), ..Default::default() };
        let mut frame = ctx.run_ui(raw, |ui| {
            let fit = Fit::of(app, ui, face);
            out = fit.pick(face, gid).ok_or_else(|| fit.why(face, gid));
        });
        frame.textures_delta.clear();
        out
    }

    /// The cell of glyph `gid` in Show › Entire Font of `face`: the grid lists the characters, then
    /// the glyphs no character maps. Its centre, and the colour its glyph is drawn in.
    fn entire_font_cell(ctx: &egui::Context, app: &mut VectorcraftApp, face: &vectorcraft_text::FontFace, gid: u32) -> (Pos2, Color32) {
        let chars = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).count();
        let unencoded = face.unencoded_glyphs();
        let cells = inks(&tall_shapes(ctx, app));
        assert_eq!(cells.len(), chars + unencoded.len(), "every glyph has a cell");
        let at = chars + unencoded.iter().position(|g| *g == gid).expect("an unencoded glyph");
        (cells[at].0.center(), cells[at].1)
    }

    /// The owner's Kufi Murabba PIIIX Heavy (`~/Library/Fonts`), in the app's font database; `None`
    /// when it is not installed. A file that is there but gives no face is a failure.
    fn kufi_heavy() -> Option<std::sync::Arc<vectorcraft_text::FontFace>> {
        let path = std::path::Path::new(&std::env::var_os("HOME")?).join("Library/Fonts/KufiMurabbaPIIIX-Heavy.ttf");
        if !path.exists() {
            eprintln!("skipped: {} is not installed", path.display());
            return None;
        }
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{} exists but cannot be read: {e}", path.display()));
        FontDb::global().add_font(bytes);
        let face =
            FontDb::global().face("Kufi Murabba PIIIX", "Heavy").unwrap_or_else(|| panic!("{} exists but its face is missing", path.display()));
        assert_eq!((face.family.as_str(), face.style.as_str()), ("Kufi Murabba PIIIX", "Heavy"), "the installed font, not a fallback");
        Some(face)
    }

    fn gid_named(face: &vectorcraft_text::FontFace, name: &str) -> u32 {
        (0..face.glyph_count()).find(|g| face.glyph_name(*g).as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} missing"))
    }

    /// The word al-Khwarizmi ending in `letter`, in Kufi Murabba PIIIX Heavy.
    fn khwarizmi_in_heavy(letter: char) -> (VectorcraftApp, serde_json::Value, usize) {
        let word = format!("\u{627}\u{644}\u{62E}\u{648}\u{627}\u{631}\u{632}\u{645}{letter}");
        let (mut app, id) = editing(&word);
        app.run("text.setRangeStyle", json!({"id": id, "font": "Kufi Murabba PIIIX", "style": "Heavy"})).unwrap();
        (app, id, word.len() - letter.len_utf8())
    }

    /// Applying each alternate of the final letter of a word in Kufi Murabba PIIIX Heavy, with the
    /// letter selected as the Type tool selects it (the UTF-8 byte range of the character, the
    /// last one of a right-to-left word), by a double-click on its cell in Alternates for Current
    /// Selection and in Entire Font: the same glyph, one undo step, the rest of the word as it was.
    /// Persian yeh, Arabic yeh and alef maksura each have their own `.fina.alt1` to `.fina.alt6`.
    #[test]
    fn a_final_letters_alternates_apply_from_alternates_and_from_entire_font() {
        let Some(face) = kufi_heavy() else { return };
        for (letter, base) in [('\u{06CC}', "yehfarsi"), ('\u{064A}', "yeh"), ('\u{0649}', "alefmaksura")] {
            let (mut app, id, last) = khwarizmi_in_heavy(letter);
            let plain = glyph_ids(&app, &id);
            let ends = plain.len() - 1;
            assert_eq!(plain[ends], gid_named(&face, &format!("{base}.fina")), "{base}: the final form");
            select(&mut app, last, last + letter.len_utf8());
            assert_eq!(selected_char(&app), Some(letter), "{base}: the selection is the last letter");
            let alternates: Vec<u32> = (1..=6).map(|n| gid_named(&face, &format!("{base}.fina.alt{n}"))).collect();
            let ctx = egui::Context::default();
            for view in ["Alternates for Current Selection", "Entire Font"] {
                for (n, alt) in alternates.iter().enumerate() {
                    choose(&ctx, &mut app, view);
                    let undo = app.session.active().unwrap().history.undo.len();
                    let at = if view == "Entire Font" {
                        let (at, ink) = entire_font_cell(&ctx, &mut app, &face, *alt);
                        assert_eq!(ink, Tokens::get(&ctx).text_strong, "{base} {view}: {alt} is not dimmed for its own letter");
                        at
                    } else {
                        let cells = squares(&shapes(&ctx, &mut app, vec![]));
                        assert_eq!(cells.len(), 7, "{base}: the final form and its six alternates");
                        cells[n + 1].center()
                    };
                    double_click_in(&ctx, &mut app, at, TALL);
                    let after = glyph_ids(&app, &id);
                    let mut want = plain.clone();
                    want[ends] = *alt;
                    assert_eq!(after, want, "{base} {view} alt{}: only the last letter changes; status: {}", n + 1, app.ui.status);
                    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "{base} {view} alt{}: one undo step", n + 1);
                    app.run("edit.undo", json!({})).unwrap();
                    assert_eq!(glyph_ids(&app, &id), plain, "{base} {view}: undo");
                }
            }
        }
    }

    /// A glyph of another letter is dimmed in Entire Font and a double-click changes nothing: the
    /// tooltip's reason names the glyph's letter, where in a word it stands and the selected letter.
    #[test]
    fn a_glyph_that_cannot_go_on_the_selection_is_dimmed_and_says_why() {
        let Some(face) = kufi_heavy() else { return };
        let glyph = gid_named(&face, "yehfarsi.fina.alt2");
        assert_eq!(glyph, 820, "the owner's glyph");
        for (letter, selected) in [('\u{064A}', "ي (U+064A)"), ('\u{0649}', "ى (U+0649)")] {
            let (mut app, id, last) = khwarizmi_in_heavy(letter);
            let plain = glyph_ids(&app, &id);
            select(&mut app, last, last + letter.len_utf8());
            let ctx = egui::Context::default();
            let t = Tokens::get(&ctx);
            let why = verdict(&ctx, &app, &face, glyph).err().expect("not applicable");
            assert_eq!(why, format!("yehfarsi.fina.alt2 belongs to ی (U+06CC) at the end of a word; your selection is {selected}."));
            let (at, ink) = entire_font_cell(&ctx, &mut app, &face, glyph);
            assert_eq!(ink, t.text_disabled, "{letter}: dimmed");
            let undo = app.session.active().unwrap().history.undo.len();
            double_click_in(&ctx, &mut app, at, TALL);
            assert_eq!(glyph_ids(&app, &id), plain, "{letter}: nothing changes");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
            assert_eq!(app.ui.status, why, "the status says why");
        }
        // The same glyph on Persian yeh applies.
        let (mut app, id, last) = khwarizmi_in_heavy('\u{06CC}');
        select(&mut app, last, last + 2);
        let ctx = egui::Context::default();
        let (at, ink) = entire_font_cell(&ctx, &mut app, &face, glyph);
        assert_eq!(ink, Tokens::get(&ctx).text_strong);
        double_click_in(&ctx, &mut app, at, TALL);
        assert_eq!(glyph_ids(&app, &id).last(), Some(&glyph));
        // Nothing selected, and a variant of the letter's other form.
        select(&mut app, 0, 0);
        assert_eq!(verdict(&ctx, &app, &face, glyph).err().as_deref(), Some("Select one character with the Type tool first."));
        let (_, ink) = entire_font_cell(&ctx, &mut app, &face, glyph);
        assert_eq!(ink, Tokens::get(&ctx).text_disabled, "dimmed without a selection");
        select(&mut app, last, last + 2);
        let medial = gid_named(&face, "yehfarsi.medi.alt1");
        assert_eq!(
            verdict(&ctx, &app, &face, medial).err().as_deref(),
            Some("yehfarsi.medi.alt1 is not an alternate of ی (U+06CC) as it stands in its word."),
            "its own letter, another place in a word"
        );
    }

    /// A glyph no feature reaches (`a.a`, `a.b` of a font with no feature for them) is applied by
    /// Entire Font to the selected character as a glyph choice, not dimmed; one the font's naming
    /// gives to another letter (`a.a` on `y`) stays dimmed, and so does every glyph with no
    /// character selected.
    #[test]
    fn entire_font_applies_a_glyph_no_feature_reaches_to_its_letter() {
        use vectorcraft_text::test_fonts::{alternates_family, alternates_font};
        FontDb::global().add_font(alternates_font(true).expect("the test font builds"));
        let face = FontDb::global().face(alternates_family(true), "Regular").expect("the test face");
        let glyph = gid_named(&face, "a.a");
        assert!(face.feature_reaching(glyph).is_none() && face.glyph_origin(glyph).is_none(), "no feature reaches it");
        let (mut app, id) = editing("xay");
        app.run("text.setRangeStyle", json!({"id": id, "font": alternates_family(true)})).unwrap();
        let ctx = egui::Context::default();
        let t = Tokens::get(&ctx);
        let plain = glyph_ids(&app, &id);
        // The cell of the glyph and its colour: this font's grid is longer than the panel is tall,
        // but the glyph is in the first rows.
        let cell_of = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let chars = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).count();
            let at = chars + face.unencoded_glyphs().iter().position(|g| *g == glyph).expect("an unencoded glyph");
            let cells = inks(&tall_shapes(ctx, app));
            (cells[at].0.center(), cells[at].1)
        };
        // Another letter, and nothing selected: dimmed, and a double-click changes nothing.
        for (start, end) in [(2, 3), (0, 0)] {
            select(&mut app, start, end);
            assert!(verdict(&ctx, &app, &face, glyph).is_err());
            let (at, ink) = cell_of(&ctx, &mut app);
            assert_eq!(ink, t.text_disabled, "{start}..{end}: dimmed");
            double_click_in(&ctx, &mut app, at, TALL);
            assert_eq!(glyph_ids(&app, &id), plain);
        }
        // The letter itself: enabled, one undo step, drawn with that glyph.
        select(&mut app, 1, 2);
        assert!(matches!(verdict(&ctx, &app, &face, glyph), Ok(Pick::Glyph(g)) if g == glyph));
        let (at, ink) = cell_of(&ctx, &mut app);
        assert_eq!(ink, t.text_strong);
        let undo = app.session.active().unwrap().history.undo.len();
        double_click_in(&ctx, &mut app, at, TALL);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        assert_eq!(glyph_ids(&app, &id), [plain[0], glyph, plain[2]]);
    }

    /// A presentation form picked in Entire Font (beh medial, U+FE92) with the caret before the last
    /// letter would be a fixed shape that does not join the word: the letter (beh) is inserted, so
    /// the word is shaped as if it were typed, and the status says so.
    #[test]
    fn a_presentation_form_inserts_its_letter_so_the_word_joins() {
        let Some(face) = kufi_heavy() else { return };
        let (mut app, id, last) = khwarizmi_in_heavy('\u{06CC}');
        let typed = {
            let word = "\u{627}\u{644}\u{62E}\u{648}\u{627}\u{631}\u{632}\u{645}\u{628}\u{6CC}";
            let (mut other, other_id) = editing(word);
            other.run("text.setRangeStyle", json!({"id": other_id, "font": "Kufi Murabba PIIIX", "style": "Heavy"})).unwrap();
            glyph_ids(&other, &other_id)
        };
        let gid_of = |c| face.chars().into_iter().find(|(ch, _)| *ch == c).expect("the character is in the font").1;
        // Typed as the presentation form itself, the same word is shaped differently: it does not join.
        let literal = {
            let word = "\u{627}\u{644}\u{62E}\u{648}\u{627}\u{631}\u{632}\u{645}\u{FE92}\u{6CC}";
            let (mut other, other_id) = editing(word);
            other.run("text.setRangeStyle", json!({"id": other_id, "font": "Kufi Murabba PIIIX", "style": "Heavy"})).unwrap();
            glyph_ids(&other, &other_id)
        };
        eprintln!("typed with beh {typed:?}\ntyped with U+FE92 {literal:?}");
        assert_ne!(literal, typed, "the presentation form is not shaped like the letter");
        assert_eq!(base_letter(&face, '\u{FE92}', gid_of('\u{FE92}')), Some('\u{0628}'), "medial beh is a form of beh");
        assert_eq!(base_letter(&face, '\u{0628}', gid_of('\u{0628}')), None, "beh itself inserts as it is");
        let tip = encoded_tip(&face, '\u{FE92}', gid_of('\u{FE92}'));
        assert!(tip.contains("(U+0628)"), "{tip}");
        select(&mut app, last, last);
        let ctx = egui::Context::default();
        let chars = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).collect::<Vec<_>>();
        let cells = inks(&tall_shapes(&ctx, &mut app));
        let at = cells[chars.iter().position(|(c, _)| *c == '\u{FE92}').expect("the presentation form is in the font")].0.center();
        double_click_in(&ctx, &mut app, at, TALL);
        let NodeKind::Text(text) = &app.session.active().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
        assert!(text.plain_text().ends_with("\u{645}\u{628}\u{6CC}"), "the letter is in the text: {}", text.plain_text());
        assert_eq!(glyph_ids(&app, &id), typed, "shaped as the word typed with beh");
        assert!(app.ui.status.starts_with("Inserted ب (U+0628)"), "{}", app.ui.status);
    }

    #[test]
    fn an_unlisted_choice_without_gsub_can_be_reset_from_the_default_cell() {
        use vectorcraft_text::test_fonts::{UNLISTED_VARIABLE_FAMILY, unlisted_variable_font};
        let db = FontDb::global();
        db.add_font(unlisted_variable_font().expect("the test font builds"));
        let face = db.face(UNLISTED_VARIABLE_FAMILY, "Regular").expect("the test face");
        let glyph = gid_named(&face, "a.a");
        let (mut app, id) = editing("a");
        app.run("text.setRangeStyle", json!({"id": id, "font": UNLISTED_VARIABLE_FAMILY})).unwrap();
        select(&mut app, 0, 1);
        let plain = glyph_ids(&app, &id);
        app.run("text.setGlyphAlternate", json!({"id": id, "start": 0, "gid": glyph})).unwrap();
        assert_eq!(glyph_ids(&app, &id), [glyph]);
        let ctx = egui::Context::default();
        choose(&ctx, &mut app, "Alternates for Current Selection");
        let cells = squares(&shapes(&ctx, &mut app, vec![]));
        assert_eq!(cells.len(), 1, "the default remains available without any GSUB alternates");
        let undo = app.session.active().unwrap().history.undo.len();
        double_click(&ctx, &mut app, cells[0].center());
        assert_eq!(glyph_ids(&app, &id), plain);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(glyph_ids(&app, &id), [glyph]);
    }

    #[test]
    fn entire_font_applies_an_unlisted_glyph_at_the_selections_variable_axes() {
        use vectorcraft_text::test_fonts::{UNLISTED_VARIABLE_FAMILY, unlisted_variable_font};
        let db = FontDb::global();
        db.add_font(unlisted_variable_font().expect("the test font builds"));
        let base = db.face(UNLISTED_VARIABLE_FAMILY, "Regular").expect("the test face");
        let glyph = gid_named(&base, "a.a");
        let (mut app, id) = editing("a");
        app.run("text.setRangeStyle", json!({"id": id, "font": UNLISTED_VARIABLE_FAMILY, "fontAxes": {"wght": 650}})).unwrap();
        select(&mut app, 0, 1);
        let target = selected_target(&app).expect("the selected character");
        let face = db.style_face(target.style).expect("the variable instance");
        assert_ne!(face.id(), base.id(), "a non-default setting has its own face id");
        let ctx = egui::Context::default();
        let chars = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).count();
        let at = chars + face.unencoded_glyphs().iter().position(|g| *g == glyph).expect("the unencoded glyph");
        let cells = inks(&tall_shapes(&ctx, &mut app));
        assert_eq!(cells[at].1, Tokens::get(&ctx).text_strong, "the instance's alternate is enabled");
        double_click_in(&ctx, &mut app, cells[at].0.center(), TALL);
        assert_eq!(glyph_ids(&app, &id), [glyph]);
        assert_eq!(selected_target(&app).unwrap().style.font_axes["wght"], 650.0);
        app.run("edit.undo", json!({})).unwrap();
        assert_ne!(glyph_ids(&app, &id), [glyph]);
    }

    #[test]
    fn encoded_characters_normalize_only_arabic_presentation_forms() {
        use vectorcraft_text::test_fonts::{ALIASED_JOINING_FAMILY, aliased_joining_font};
        let db = FontDb::global();
        db.add_font(aliased_joining_font().expect("the test font builds"));
        let face = db.face(ALIASED_JOINING_FAMILY, "Regular").expect("the test face");
        let chars = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).collect::<Vec<_>>();
        for (c, expected) in [('B', "B"), ('\u{FE91}', "\u{0628}")] {
            let (mut app, id) = editing("a");
            app.run("text.setRangeStyle", json!({"id": id, "font": ALIASED_JOINING_FAMILY})).unwrap();
            select(&mut app, 0, 1);
            let at = chars.iter().position(|(ch, _)| *ch == c).expect("an encoded character");
            let origin = face.glyph_origin(chars[at].1).expect("the shared positional glyph");
            assert_eq!(origin.ch, '\u{0628}');
            assert!(origin.position().is_some());
            let ctx = egui::Context::default();
            set_pstate(&ctx, "gl-font", Some((ALIASED_JOINING_FAMILY.to_string(), "Regular".to_string())));
            let cells = inks(&tall_shapes(&ctx, &mut app));
            double_click_in(&ctx, &mut app, cells[at].0.center(), TALL);
            let state = app.session.active().unwrap();
            let NodeKind::Text(text) = &state.doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
            assert_eq!(text.plain_text(), expected, "the encoded {c} cell inserts the intended character");
            app.run("edit.undo", json!({})).unwrap();
            let state = app.session.active().unwrap();
            let NodeKind::Text(text) = &state.doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!("text") };
            assert_eq!(text.plain_text(), "a");
        }
    }

    #[test]
    fn a_normalized_glyph_without_a_text_target_preserves_the_insertion_error() {
        use vectorcraft_text::test_fonts::{ALIASED_JOINING_FAMILY, aliased_joining_font};
        let db = FontDb::global();
        db.add_font(aliased_joining_font().expect("the test font builds"));
        let face = db.face(ALIASED_JOINING_FAMILY, "Regular").expect("the test face");
        let chars = face.chars().into_iter().filter(|(c, _)| !c.is_control() && !c.is_whitespace()).collect::<Vec<_>>();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        set_pstate(&ctx, "gl-font", Some((ALIASED_JOINING_FAMILY.to_string(), "Regular".to_string())));
        let cells = inks(&tall_shapes(&ctx, &mut app));
        let at = |c| chars.iter().position(|(ch, _)| *ch == c).expect("the encoded glyph");
        double_click_in(&ctx, &mut app, cells[at('B')].0.center(), TALL);
        let failed = app.ui.status.clone();
        assert!(!failed.is_empty(), "the ordinary character reports the missing text target");
        double_click_in(&ctx, &mut app, cells[at('\u{FE91}')].0.center(), TALL);
        assert_eq!(app.ui.status, failed, "normalization must not overwrite the insertion failure with a success");
        let state = app.session.active().unwrap();
        assert_eq!(state.history.undo.len(), undo);
        let mut texts = 0;
        state.doc.walk(|n| texts += usize::from(matches!(n.kind, NodeKind::Text(_))));
        assert_eq!(texts, 0, "neither glyph created or changed text");
    }
}
