//! The font family menu of the Character, Properties and Glyphs panels and Find Font's Replace
//! With: a search field over every family available, each row the family's name and a sample of
//! the selected text set in it. In the Character and Properties panels the row under the pointer
//! or reached with ↑/↓ previews its font on the selected text (one live interaction, nothing in
//! the history); a click or Enter applies it as one step, Escape or closing the menu puts the text
//! back. The Glyphs panel and Find Font only pick a font ([`picked`]): nothing is previewed on the
//! document. A star marks a favourite family, and the ★ filter lists only those. Preferences ›
//! Type › Enable in-menu font previews and Font Preview Size turn the samples off or size the
//! rows.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Sense, Ui, vec2};
use serde_json::json;

use vectorcraft_text::FontClass;

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// What happened in the menu this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum FontPick {
    /// A family (and one of its styles, when picked from the family's expanded list) was chosen.
    Chosen(String, Option<String>),
    /// The highlighted row changed: preview its family (and style).
    Preview(String, Option<String>),
    /// The menu closed (or came back to the current family) without a choice: undo the preview.
    EndPreview,
    /// The star of a family was clicked: add it to the favourites, or take it out.
    Favorite(String),
}

/// How the menu looks: from the preferences and the favourites.
#[derive(Clone, Copy, Debug)]
pub struct MenuLook<'a> {
    /// Show each family's sample (Preferences › Type › Enable in-menu font previews).
    pub samples: bool,
    /// Preferences › Type › Show Font Names in English (on by default).
    pub english_names: bool,
    /// Row height in points (Font Preview Size).
    pub row: f32,
    /// The starred families.
    pub favorites: &'a [String],
}

impl Default for MenuLook<'static> {
    /// Samples on, English names, medium rows, no favourites.
    fn default() -> Self {
        Self { samples: true, english_names: true, row: ROW, favorites: &[] }
    }
}

impl<'a> MenuLook<'a> {
    pub fn of(app: &'a VectorcraftApp) -> Self {
        let row = match app.session.prefs.font_preview_size.as_str() {
            "small" => 20.0,
            "large" => 32.0,
            _ => ROW,
        };
        Self {
            samples: app.session.prefs.font_preview,
            english_names: app.session.prefs.font_names_in_english,
            row,
            favorites: &app.ui.favorite_fonts,
        }
    }
}

/// Height of a row, in points (Font Preview Size: Medium).
const ROW: f32 = 24.0;
/// Width of the star at the end of a family's row.
const STAR: f32 = 18.0;
/// The share of a row the family's name takes; the sample fills the rest.
const NAME_SHARE: f32 = 0.5;
/// Width of the disclosure triangle of a family with several styles.
const DISCLOSURE: f32 = 14.0;
/// Samples rendered (or asked for) per frame, across every menu, so a long list doesn't stall the UI.
const SAMPLES_PER_FRAME: usize = 4;
/// Samples rendering at once (native).
#[cfg(not(target_arch = "wasm32"))]
const MAX_PENDING: usize = 8;
/// What a sample shows when the font lacks the selected text's characters (or there is none).
const FALLBACK_SAMPLE: &str = "Sample";
/// The script filter's choices: all, Japanese, Latin.
const SCRIPTS: usize = 3;

fn script_label(i: usize) -> &'static str {
    match i {
        1 => tl!("Japanese"),
        2 => tl!("Latin"),
        _ => tl!("All Scripts"),
    }
}

fn class_label(c: FontClass) -> &'static str {
    match c {
        FontClass::Serif => tl!("Serif / Mincho"),
        FontClass::Sans => tl!("Sans Serif / Gothic"),
        FontClass::Rounded => tl!("Rounded"),
        FontClass::Script => tl!("Script / Brush"),
        FontClass::Monospaced => tl!("Monospaced"),
        FontClass::Decorative => tl!("Decorative"),
        FontClass::Other => tl!("Other"),
    }
}

/// A row of the list: a family, or one of the styles of an expanded family.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Family(String),
    Style(String, String),
}

impl Row {
    fn family(&self) -> &str {
        match self {
            Row::Family(f) | Row::Style(f, _) => f,
        }
    }
    fn style(&self) -> Option<&str> {
        match self {
            Row::Family(_) => None,
            Row::Style(_, s) => Some(s),
        }
    }
    fn pick(&self) -> (String, Option<String>) {
        (self.family().to_string(), self.style().map(str::to_string))
    }
}

/// Keep keyboard/pointer focus on the same font after filtering, loading new fonts or
/// expanding a family's styles. A row index on its own can now name a different font.
fn remap_highlight(before: &[Row], selected: Option<usize>, after: &[Row]) -> Option<usize> {
    let row = before.get(selected?)?;
    after.iter().position(|candidate| candidate == row)
}

/// What the rows were listed for: the query, the script, kind and favourites filters, Show Font
/// Names in English, the expanded families, the favourites and the font list's generation.
type RowsKey = (String, usize, Option<FontClass>, bool, bool, Vec<String>, Vec<String>, u64);

/// The menu's state between frames.
#[derive(Clone, Default)]
struct MenuState {
    /// The pass the list was last drawn in, and the pass it opened in.
    last: u64,
    opened: u64,
    /// The family the text had as the menu opened. A preview shows another one in the document
    /// (and the menu's caller), but this one stays the current family: hovering it ends the
    /// preview (#563).
    current: String,
    /// The highlighted row and the query and filters it belongs to.
    highlight: Option<usize>,
    query: String,
    /// The filters (kept from one opening to the next): script (0..[`SCRIPTS`]), kind, and
    /// favourites only.
    script: usize,
    class: Option<FontClass>,
    favorites_only: bool,
    /// Families whose styles are listed (lowercase).
    expanded: Vec<String>,
    /// What was previewed last (None: nothing is previewed).
    previewed: Option<(String, Option<String>)>,
    /// The rows listed, and what for: built again only when that changes, not every frame.
    rows: Arc<[Row]>,
    rows_key: Option<RowsKey>,
    /// The list's scroll offset last frame.
    scrolled: f32,
}

/// The font family menu showing `current`, `width` points wide, rows sampling `sample` (the
/// selected text; `None`: a generic sample).
pub fn font_menu(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    width: f32,
    sample: Option<&str>,
    look: MenuLook,
) -> Option<FontPick> {
    let state_id = ui.id().with(&id).with("font-menu");
    let mut st: MenuState = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let mut drawn = false;
    let picked = widgets::combo(ui, &id, current, width, true, |ui| {
        drawn = true;
        list(ui, state_id, &mut st, current, sample, look)
    });
    let mut out = picked;
    if !drawn && out.is_none() && st.previewed.take().is_some() {
        // Closed (Escape, a click outside) with a preview showing.
        out = Some(FontPick::EndPreview);
    }
    if matches!(out, Some(FontPick::Chosen(..))) {
        st.previewed = None;
    }
    ui.data_mut(|d| d.insert_temp(state_id, st));
    out
}

/// The list inside the open menu.
fn list(ui: &mut Ui, state_id: egui::Id, st: &mut MenuState, current: &str, sample: Option<&str>, look: MenuLook) -> Option<FontPick> {
    let row_h = look.row;
    let t = Tokens::get(ui.ctx());
    // Each time the menu opens: no query, the current family highlighted and in view (the
    // filters stay as they were).
    let pass = ui.ctx().cumulative_pass_nr();
    let opening_now = st.last + 1 < pass || st.last == 0;
    if opening_now {
        st.opened = pass;
        st.highlight = None;
        st.query.clear();
        st.current = current.to_string();
    }
    st.last = pass;
    // For [`end_stale_preview`]: the menu is open, in this layer.
    ui.ctx().data_mut(|d| d.insert_temp(open_id(), (pass, ui.layer_id())));
    let field = state_id.with("query");
    if opening_now {
        ui.data_mut(|d| d.insert_temp(field, String::new()));
    }
    // A popup sizes itself invisibly in its first pass.
    let opening = pass <= st.opened + 1;
    // Arrow keys move the highlight and open or close a family's styles (taken before the
    // search field sees them).
    let key = |i: &mut egui::InputState, k| i.consume_key(egui::Modifiers::NONE, k);
    let (up, down, left, right) =
        ui.input_mut(|i| (key(i, egui::Key::ArrowUp), key(i, egui::Key::ArrowDown), key(i, egui::Key::ArrowLeft), key(i, egui::Key::ArrowRight)));
    if ui.memory(|m| m.focused().is_none()) {
        ui.memory_mut(|m| m.request_focus(field.with("edit")));
    }
    let query = widgets::search_field(ui, field, tl!("Search")).to_lowercase();
    // The filters: script, then kind.
    let mut refilter = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
        for i in 0..SCRIPTS {
            if ui.selectable_label(st.script == i, egui::RichText::new(script_label(i)).size(11.0)).clicked() {
                st.script = i;
                refilter = true;
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
        if ui.selectable_label(st.favorites_only, egui::RichText::new("★").size(11.0)).on_hover_text(tl!("Favorites")).clicked() {
            st.favorites_only = !st.favorites_only;
            refilter = true;
        }
        if ui.selectable_label(st.class.is_none(), egui::RichText::new(tl!("Any Kind")).size(11.0)).clicked() {
            st.class = None;
            refilter = true;
        }
        for c in FontClass::ALL {
            if ui.selectable_label(st.class == Some(c), egui::RichText::new(class_label(c)).size(11.0)).clicked() {
                st.class = if st.class == Some(c) { None } else { Some(c) };
                refilter = true;
            }
        }
    });
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let db = vectorcraft_text::FontDb::global();
    // The current family by its own name (a document may name it in Japanese: ヒラギノ角ゴシック),
    // as it was before any preview.
    let current = db.canonical(&st.current, "").0;
    let current = current.as_str();
    let favorite = |f: &str| look.favorites.iter().any(|x| x.eq_ignore_ascii_case(f));
    // The generation is read first: fonts that load meanwhile make the next frame list them.
    let key: RowsKey =
        (query.clone(), st.script, st.class, st.favorites_only, look.english_names, st.expanded.clone(), look.favorites.to_vec(), db.generation());
    if st.rows_key.as_ref() != Some(&key) {
        let wanted = |f: &str| {
            let label = db.family_display_name(f, look.english_names);
            if !(query.is_empty() || f.to_lowercase().contains(&query) || label.to_lowercase().contains(&query)) {
                return false;
            }
            if st.favorites_only && !favorite(f) {
                return false;
            }
            if st.script == 0 && st.class.is_none() {
                return true;
            }
            let traits = db.family_traits(f);
            (st.script == 0 || (st.script == 1) == traits.japanese) && st.class.is_none_or(|c| c == traits.class)
        };
        let mut rows: Vec<Row> = vec![];
        for f in db.menu_family_list().iter().filter(|f| wanted(f)) {
            rows.push(Row::Family(f.clone()));
            if st.expanded.iter().any(|e| e.eq_ignore_ascii_case(f)) {
                rows.extend(db.styles(f).into_iter().map(|s| Row::Style(f.clone(), s)));
            }
        }
        // Filter changes (including un-starring a favourite) may remove rows; expanding
        // another family or a background font scan can insert rows before the highlight.
        // Never preview an unrelated family because the old numeric index now points to it.
        st.highlight = remap_highlight(&st.rows, st.highlight, &rows);
        st.rows = rows.into();
        st.rows_key = Some(key);
    }
    let rows = st.rows.clone();
    if query != st.query || refilter {
        st.query = query.clone();
        st.highlight = None;
    }
    if st.highlight.is_none() && query.is_empty() {
        st.highlight = rows.iter().position(|r| matches!(r, Row::Family(f) if f.eq_ignore_ascii_case(current)));
    }
    let mut keyed = false;
    if (up || down) && !rows.is_empty() {
        let last = rows.len() - 1;
        st.highlight = Some(match (st.highlight, down) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(last),
            (Some(i), false) => i.saturating_sub(1),
        });
        keyed = true;
    }
    // → opens the highlighted family's styles, ← closes them (from the family or one of them).
    if let Some(row) = st.highlight.and_then(|i| rows.get(i)).cloned().filter(|_| left || right) {
        let f = row.family().to_lowercase();
        let open = st.expanded.contains(&f);
        if right && !open && db.styles(row.family()).len() > 1 {
            st.expanded.push(f);
        } else if left && open {
            st.expanded.retain(|e| *e != f);
            st.highlight = rows.iter().position(|r| *r == Row::Family(row.family().to_string()));
        }
        keyed = true;
    }
    if rows.is_empty() {
        widgets::dim_label(ui, tl!("No matching fonts"));
        return None;
    }
    // The rows: only those in view are laid out (there are thousands of families). The list is
    // the menu's one scrolling part, under the search field and filters (#555): it fills the
    // menu's room (its max rect) below them.
    let room = (ui.max_rect().bottom() - ui.next_widget_position().y).clamp(120.0, 600.0);
    let mut area = egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room.min(row_h * rows.len() as f32));
    // The highlight in view: centred as the menu opens, scrolled just enough as the keys move it.
    if let Some(top) = st.highlight.map(|i| i as f32 * row_h) {
        let target = if opening {
            Some(top - (room - row_h) * 0.5)
        } else if keyed && top < st.scrolled {
            Some(top)
        } else if keyed && top + row_h > st.scrolled + room {
            Some(top + row_h - room)
        } else {
            None
        };
        if let Some(y) = target {
            area = area.vertical_scroll_offset(y.max(0.0));
        }
    }
    let mut chosen: Option<(String, Option<String>)> = None;
    let mut hovered: Option<usize> = None;
    let mut toggled: Option<String> = None;
    let mut starred: Option<String> = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    let shown = area.show_rows(ui, row_h, rows.len(), |ui, range| {
        for i in range {
            let Some(row) = rows.get(i) else { continue };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO || inp.smooth_scroll_delta != egui::Vec2::ZERO) {
                hovered = Some(i);
            }
            let p = ui.painter_at(rect);
            if st.highlight == Some(i) {
                p.rect_filled(rect, 2.0, t.selection);
            } else if matches!(row, Row::Family(f) if f.eq_ignore_ascii_case(current)) {
                p.rect_filled(rect, 2.0, t.hover);
            }
            // The disclosure triangle of a family with several styles.
            let tri = egui::Rect::from_min_size(rect.min, vec2(DISCLOSURE, row_h));
            let mut on_triangle = false;
            if let Row::Family(f) = row
                && db.styles(f).len() > 1
            {
                let open = st.expanded.iter().any(|e| e.eq_ignore_ascii_case(f));
                p.text(tri.center(), egui::Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, egui::FontId::proportional(11.0), t.text_dim);
                on_triangle = resp.interact_pointer_pos().is_some_and(|pos| tri.contains(pos));
                if resp.clicked() && on_triangle {
                    toggled = Some(f.to_lowercase());
                }
            }
            // The star at the end of a family's row.
            let star = egui::Rect::from_min_max(egui::pos2(rect.max.x - STAR, rect.min.y), rect.max);
            let mut on_star = false;
            if let Row::Family(f) = row {
                let on = favorite(f);
                let hot = resp.hover_pos().is_some_and(|pos| star.contains(pos));
                if on || hot || st.highlight == Some(i) {
                    let color = if on { t.accent } else { t.text_dim };
                    p.text(star.center(), egui::Align2::CENTER_CENTER, if on { "★" } else { "☆" }, egui::FontId::proportional(12.0), color);
                }
                on_star = resp.interact_pointer_pos().is_some_and(|pos| star.contains(pos));
                if resp.clicked() && on_star {
                    starred = Some(f.clone());
                }
            }
            let indent = if row.style().is_some() { DISCLOSURE + 12.0 } else { DISCLOSURE };
            let name_w = rect.width() * NAME_SHARE;
            let name_rect = egui::Rect::from_min_max(rect.min + vec2(indent, 0.0), egui::pos2(rect.min.x + name_w - 4.0, rect.max.y));
            // Family rows follow Show Font Names in English; style rows keep the style name (#394).
            let name = match row {
                Row::Style(_, style) => style.clone(),
                Row::Family(f) => db.family_display_name(f, look.english_names),
            };
            ui.painter_at(name_rect).text(name_rect.left_center(), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), t.text);
            let sample_rect = egui::Rect::from_min_max(rect.min + vec2(name_w, 2.0), rect.max - vec2(STAR + 2.0, 2.0));
            if !look.samples {
            } else if let Some(tex) = sample_texture(ui.ctx(), row.family(), row.style(), sample, sample_rect.height()) {
                paint_sample(ui, sample_rect, &tex, t.text);
            }
            if resp.clicked() && !on_triangle && !on_star {
                chosen = Some(row.pick());
            }
        }
    });
    st.scrolled = shown.state.offset.y;
    if let Some(f) = toggled {
        if st.expanded.contains(&f) {
            st.expanded.retain(|e| *e != f);
        } else {
            st.expanded.push(f);
        }
        // The rows change next frame: what the highlight points at is decided then.
        return None;
    }
    if let Some(i) = hovered {
        st.highlight = Some(i);
    }
    if let Some(f) = starred {
        return Some(FontPick::Favorite(f));
    }
    if enter && chosen.is_none() {
        chosen = st.highlight.and_then(|i| rows.get(i)).or(rows.first()).map(Row::pick);
    }
    if let Some((f, s)) = chosen {
        ui.close();
        return Some(FontPick::Chosen(f, s));
    }
    // The highlighted row is previewed; back on the current family, the preview goes.
    let lit = st.highlight.and_then(|i| rows.get(i)).map(Row::pick);
    match lit {
        Some((f, None)) if f == current => st.previewed.take().map(|_| FontPick::EndPreview),
        Some(pick) if st.previewed.as_ref() != Some(&pick) => {
            st.previewed = Some(pick.clone());
            Some(FontPick::Preview(pick.0, pick.1))
        }
        _ => None,
    }
}

// ---------- samples ----------

/// Width of the sample drawn at the end of a Type → Font menu item.
/// The room a Type → Font menu item keeps for its sample, after the family's name.
pub(crate) const MENU_SAMPLE_SIZE: egui::Vec2 = vec2(90.0, 16.0);

/// A generic sample of `family` drawn in `slot` (the room a Type → Font item of the in-window
/// menu bar keeps for it; the native macOS menu shows names only).
pub(crate) fn menu_item_sample(ui: &Ui, slot: egui::Rect, family: &str) {
    if let Some(tex) = sample_texture(ui.ctx(), family, None, None, slot.height()) {
        paint_sample(ui, slot, &tex, Tokens::get(ui.ctx()).text_dim);
    }
}

/// Draw sample `tex` (tinted `tint`) at the left of `area`, centred vertically, cut at its right.
fn paint_sample(ui: &Ui, area: egui::Rect, tex: &egui::TextureHandle, tint: Color32) {
    let size = tex.size_vec2() / ui.ctx().pixels_per_point();
    let at = egui::Rect::from_min_size(area.left_center() - vec2(0.0, size.y * 0.5), size);
    let shown = at.intersect(area);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2((shown.width() / size.x).clamp(0.0, 1.0), 1.0));
    ui.painter_at(area).image(tex.id(), shown, uv, tint);
}

type Key = (String, String, String, u32);

/// Samples kept before the cache starts over.
const MAX_SAMPLES: usize = 256;

#[derive(Default)]
struct Samples {
    ready: HashMap<Key, Option<egui::TextureHandle>>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: HashMap<Key, std::sync::mpsc::Receiver<Option<egui::ColorImage>>>,
    /// The pass the budget is for, and the samples started in it.
    pass: u64,
    started: usize,
}

impl Samples {
    /// Take one sample from this frame's budget ([`SAMPLES_PER_FRAME`]); `false` when it is spent
    /// (another frame is asked for, to go on).
    fn take_budget(&mut self, ctx: &egui::Context) -> bool {
        let pass = ctx.cumulative_pass_nr();
        if self.pass != pass {
            (self.pass, self.started) = (pass, 0);
        }
        if self.started >= SAMPLES_PER_FRAME {
            ctx.request_repaint();
            return false;
        }
        self.started += 1;
        true
    }
}

thread_local! {
    static SAMPLES: crate::graphics::TexCache<Samples> = crate::graphics::TexCache::default();
}

/// The sample of `family` for a row `height` points high: the selected text set in it (else a
/// generic sample), white with coverage as alpha (tinted when drawn). Rendered off the UI thread
/// on native (the worker asks for a frame when it is done), a few per frame; `None` until it is
/// ready.
fn sample_texture(ctx: &egui::Context, family: &str, style: Option<&str>, sample: Option<&str>, height: f32) -> Option<egui::TextureHandle> {
    let px = (height * ctx.pixels_per_point()).round().clamp(8.0, 96.0) as u32;
    let key: Key = (family.to_string(), style.unwrap_or("Regular").to_string(), sample.unwrap_or_default().to_string(), px);
    let texture = |img: egui::ColorImage| ctx.load_texture(format!("font-sample-{family}-{px}"), img, egui::TextureOptions::LINEAR);
    SAMPLES.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(t) = s.ready.get(&key) {
            return t.clone();
        }
        // Up to ~370 KB of texture each (96 px high, 10:1): keep a few screens' worth.
        if s.ready.len() > MAX_SAMPLES {
            s.ready.clear();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(rx) = s.pending.get(&key) {
                let img = match rx.try_recv() {
                    Ok(img) => img,
                    Err(std::sync::mpsc::TryRecvError::Empty) => return None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
                };
                s.pending.remove(&key);
                let tex = img.map(texture);
                s.ready.insert(key, tex.clone());
                return tex;
            }
            if s.pending.len() >= MAX_PENDING || !s.take_budget(ctx) {
                return None;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let (f, style, text) = (key.0.clone(), key.1.clone(), key.2.clone());
            let repaint = ctx.clone();
            let spawned = std::thread::Builder::new().name("font-sample".into()).spawn(move || {
                // The receiver may be gone (the cache was cleared): nothing to do then.
                let _ = tx.send(render_sample(&f, &style, &text, px));
                repaint.request_repaint();
            });
            if spawned.is_ok() {
                s.pending.insert(key, rx);
            } else {
                s.ready.insert(key, None);
            }
            None
        }
        // The web has no threads: a few samples per frame, on this one.
        #[cfg(target_arch = "wasm32")]
        {
            if !s.take_budget(ctx) {
                return None;
            }
            let tex = render_sample(&key.0, &key.1, &key.2, px).map(texture);
            s.ready.insert(key, tex.clone());
            tex
        }
    })
}

/// Longest sample, in pixels per pixel of height (wider samples are cut).
const MAX_ASPECT: f32 = 10.0;

/// `text` (or [`FALLBACK_SAMPLE`] when the font lacks some of its characters) set in `family`,
/// `px` pixels high: a coverage mask as white with alpha.
fn render_sample(family: &str, style: &str, text: &str, px: u32) -> Option<egui::ColorImage> {
    let db = vectorcraft_text::FontDb::global();
    let face: Arc<vectorcraft_text::FontFace> = db.face(family, style)?;
    let covers = |s: &str| !s.trim().is_empty() && s.chars().filter(|c| !c.is_whitespace()).all(|c| face.covers(c));
    let text = if covers(text) { text } else { FALLBACK_SAMPLE };
    if !covers(text) && !face.family.eq_ignore_ascii_case(family) {
        return None;
    }
    let (asc, desc) = face.vertical_metrics();
    let em = (asc + desc).max(1.0) / face.units_per_em();
    // The em box fills the height with a little air.
    let size = px as f64 * 0.8 / em;
    let style = vectorcraft_doc::CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size, ..Default::default() };
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, text, style);
    let layout = vectorcraft_text::layout(db, &t);
    let mut path = layout.to_bezpath();
    let b = layout.bounds;
    let w = (b.width().ceil() as u32 + 2).clamp(1, (px as f32 * MAX_ASPECT) as u32) as usize;
    let h = px as usize;
    // Baseline where the ascent fits the top (with the air above it).
    let baseline = px as f64 * 0.1 + asc / face.units_per_em() * size;
    path.apply_affine(vectorcraft_geom::Affine::translate((1.0 - b.x0.min(0.0), baseline)));
    let mask = crate::panels::glyphs::rasterize(&path, w, h);
    Some(egui::ColorImage::new([w, h], mask.iter().map(|&a| Color32::from_rgba_premultiplied(a, a, a, a)).collect()))
}

#[cfg(test)]
pub(crate) fn render_sample_for_test(family: &str, text: &str, px: u32) -> Option<egui::ColorImage> {
    render_sample(family, "Regular", text, px)
}

// ---------- applying a pick ----------

/// Is a font previewed on the text (an interaction is open for it)?
const PREVIEWING: &str = "font-previewing";

/// Where the open font menu was drawn last: (pass, its popup's layer).
fn open_id() -> egui::Id {
    egui::Id::new("font-menu-open")
}

/// Undo a preview whose menu is gone (its panel wasn't drawn: the text was deselected, the panel
/// closed) or is closing (Escape, a press outside the menu), before anything else sees this
/// frame's input: the canvas must not act inside the preview's interaction (a drag would be
/// rolled back with the preview). The shell calls it at the start of each frame.
pub(crate) fn end_stale_preview(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !crate::panels::pstate::<bool>(ctx, PREVIEWING) {
        return;
    }
    let drawn = ctx.data(|d| d.get_temp::<(u64, egui::LayerId)>(open_id()));
    // Drawn in the last pass: still open.
    let Some((_, layer)) = drawn.filter(|(pass, _)| pass + 1 >= ctx.cumulative_pass_nr()) else {
        apply(app, ctx, FontPick::EndPreview);
        return;
    };
    // (`layer_id_at` outside `input`: egui's context lock isn't re-entrant.)
    let (escape, press) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.pointer.any_pressed().then(|| i.pointer.interact_pos())));
    let leaving = escape || press.is_some_and(|at| at.is_none_or(|p| ctx.layer_id_at(p) != Some(layer)));
    if leaving {
        apply(app, ctx, FontPick::EndPreview);
    }
}

/// What the font menus preview on and apply to: the Type tool's selected range while it edits
/// text (the whole text when nothing is selected), else the selected text objects.
pub(crate) fn apply(app: &mut VectorcraftApp, ctx: &egui::Context, pick: FontPick) {
    let previewing = crate::panels::pstate::<bool>(ctx, PREVIEWING);
    match pick {
        FontPick::Preview(f, style) => {
            if !previewing {
                crate::panels::character::end_typing(app);
                if app.session.begin_interaction("Font").is_err() {
                    return;
                }
                crate::panels::set_pstate(ctx, PREVIEWING, true);
            }
            let (cmd, params) = font_command(app, &f, style.as_deref());
            if let Err(e) = app.session.preview(cmd, &params) {
                app.ui.status = e.to_string();
            }
            app.sync_views();
        }
        FontPick::EndPreview => {
            if previewing {
                app.session.cancel_interaction().ok();
                crate::panels::set_pstate(ctx, PREVIEWING, false);
                app.sync_views();
            }
        }
        FontPick::Favorite(f) => toggle_favorite(app, f),
        FontPick::Chosen(f, style) => {
            if previewing {
                app.session.cancel_interaction().ok();
                crate::panels::set_pstate(ctx, PREVIEWING, false);
            }
            crate::panels::character::end_typing(app);
            let (cmd, params) = font_command(app, &f, style.as_deref());
            app.run(cmd, params).ok();
        }
    }
}

/// Star `family`, or take its star off (any case).
fn toggle_favorite(app: &mut VectorcraftApp, family: String) {
    let favs = &mut app.ui.favorite_fonts;
    match favs.iter().position(|x| x.eq_ignore_ascii_case(&family)) {
        Some(i) => {
            favs.remove(i);
        }
        None => favs.push(family),
    }
}

/// For a font menu that only picks a font and never changes the document (Find Font's Replace
/// With, the Glyphs panel): the family (and style) chosen, if any. A star is toggled here; the
/// highlighted row is not previewed.
pub(crate) fn picked(app: &mut VectorcraftApp, pick: Option<FontPick>) -> Option<(String, Option<String>)> {
    match pick? {
        FontPick::Chosen(f, style) => Some((f, style)),
        FontPick::Favorite(f) => {
            toggle_favorite(app, f);
            None
        }
        FontPick::Preview(..) | FontPick::EndPreview => None,
    }
}

/// The command that sets font `family` (and `style`, else the closest the family has) on what the
/// menus act on.
fn font_command(app: &VectorcraftApp, family: &str, style: Option<&str>) -> (&'static str, serde_json::Value) {
    let mut p = json!({"font": family});
    if let Some(s) = style {
        p["style"] = json!(s);
    }
    match crate::panels::character::text_editing(app) {
        Some((id, a, b)) => {
            p["id"] = json!(id.0);
            if b > a {
                p["start"] = json!(a);
                p["end"] = json!(b);
            }
            ("text.setRangeStyle", p)
        }
        None => ("text.setStyle", p),
    }
}

/// The text the menus sample: the Type tool's selected characters, else the first line of the
/// first selected text object, at most a dozen characters.
pub(crate) fn sample_text(app: &VectorcraftApp) -> Option<String> {
    let doc = &app.session.active()?.doc;
    let text = match crate::panels::character::text_editing(app) {
        Some((id, a, b)) if b > a => match &doc.node(id)?.kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text().get(a..b).map(str::to_string),
            _ => None,
        },
        _ => app.session.active()?.selection.objects.iter().find_map(|id| match &doc.node(*id)?.kind {
            vectorcraft_doc::NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        }),
    }?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(12).collect())
}

#[cfg(test)]
mod highlight_rebuild_tests {
    use super::{Row, remap_highlight};

    fn family(name: &str) -> Row {
        Row::Family(name.to_string())
    }

    #[test]
    fn keeps_the_same_family_when_other_rows_are_inserted_or_removed() {
        let before = [family("Alpha"), family("Bravo"), family("Charlie")];
        assert_eq!(remap_highlight(&before, Some(2), &[family("Bravo"), family("Charlie")]), Some(1));
        assert_eq!(remap_highlight(&before, Some(2), &[family("Alpha"), family("New"), family("Bravo"), family("Charlie")]), Some(3));
    }

    #[test]
    fn clears_the_highlight_when_its_family_is_filtered_out() {
        let before = [family("Alpha"), family("Bravo"), family("Charlie")];
        assert_eq!(remap_highlight(&before, Some(1), &[family("Alpha"), family("Charlie")]), None);
        assert_eq!(remap_highlight(&before, Some(10), &before), None);
        assert_eq!(remap_highlight(&before, None, &before), None);
    }

    #[test]
    fn expanded_style_rows_do_not_retarget_the_highlight() {
        let before = [family("Alpha"), family("Bravo")];
        let after = [family("Alpha"), Row::Style("Alpha".into(), "Bold".into()), family("Bravo")];
        assert_eq!(remap_highlight(&before, Some(1), &after), Some(2));
        assert_eq!(remap_highlight(&after, Some(1), &before), None);
    }
}
