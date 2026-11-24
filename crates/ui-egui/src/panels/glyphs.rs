//! Glyphs panel: every character the current font maps (its cmap), drawn from the font's own
//! outlines; Show › Entire Font adds the glyphs no character maps, by glyph id. Double-click
//! inserts the character at the Type tool's caret (or appends it to the selected text objects).
//! The character selected with the Type tool is marked in the grid.
//! The grid, the font pickers, the category list and the zoom are the framework's
//! [`GlyphsPanel`]; this module is its source ([`VcGlyphs`]) and what its outputs do.
//! A character whose glyph has alternates has a triangle in its cell's lower right corner that opens
//! a pop-up of those alternates; picking one inserts the character drawn with it
//! ([`insert_alternate`]). In Entire Font a glyph no character maps has the triangle too, and a
//! pick goes on the selected character as a double-click on its cell does.
//! Show › Alternates for Selection lists that character's alternate glyphs instead (as it stands in
//! its word), and a double-click applies the OpenType feature that draws one to that character
//! alone, so the text stays real text that joins its neighbors. A double-click on a glyph no
//! character maps in Entire Font does the same when the selected character's font gives that glyph
//! as one of its alternates in its word. A glyph no feature reaches whose name does not belong to
//! another letter is applied as a glyph choice, which may not join. Other glyphs are drawn dimmed,
//! and their notes say why they cannot be applied to the selection.
//! With Show East Asian Options, the panel menu sets the selected characters' East Asian glyph
//! form ([`menu_rows`]).
//! After the framework's categories, the Show menu lists the OpenType categories the browsed font
//! has ([`vectorcraft_text::FontFace::glyph_categories`]): each shows the glyphs its features give,
//! as Entire Font shows them.

use std::collections::HashSet;
use std::sync::Arc;

use craft::font_manager::glyphs::{CATEGORIES, CellInfo, GlyphCell, GlyphSource, GlyphsPanel, GlyphsWords};
use craft::keymap::CommandRef;
use craft::ui::contract::Bind;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Env;
use egui::{Color32, ColorImage, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, GlyphForm, NodeId, NodeKind};
use vectorcraft_geom::{Affine, BezPath, PathEl};
use vectorcraft_text::{ContextAlternates, FontDb, FontFace};

use crate::VectorcraftApp;

/// The category that lists every character and the glyphs no character maps.
const ENTIRE_FONT: &str = "Entire Font";

/// The category of the Type tool's selected character's alternate glyphs. Public behavior: [Replace
/// characters with alternate glyphs](https://helpx.adobe.com/illustrator/desktop/design-with-text/special-characters-glyphs/replace-characters-with-alternate-glyphs.html)
/// (select Alternates For Current Selection from the Show menu; highlight a character, then
/// double-click the glyph to replace it).
const ALTERNATES: &str = "Alternates for Selection";

/// The category of the glyphs used last; the panel keeps that list itself.
const RECENTLY_USED: &str = "Recently Used";

/// The alternates of the character at a place in a text, kept until the document, the character or
/// the face changes: document uid, revision, text, byte and face id.
pub(crate) type FoundKey = (u64, u64, NodeId, usize, u32);

/// [`alternates_found`]'s cache slot in [`crate::caches::CanvasCache`].
pub(crate) type Found = Option<(FoundKey, Option<ContextAlternates>)>;

/// What the faces that draw the selected text were read for: document uid and revision, the font
/// database's generation, the Type tool's range and the selected stories.
pub(crate) type FacesKey = (u64, u64, u64, Option<(NodeId, usize, usize)>, Vec<NodeId>);

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
fn categories(face: Option<&FontFace>) -> Vec<(String, String)> {
    face.map(|f| f.glyph_categories().iter().map(|c| (c.kind.id(), category_label(c))).collect()).unwrap_or_default()
}

/// The category the panel shows: the chosen one, or Entire Font when it is not a framework category
/// and the browsed font does not have it.
fn current_category(app: &VectorcraftApp, cats: &[(String, String)]) -> String {
    let chosen = &app.ui.glyphs_panel.category;
    if CATEGORIES.contains(&chosen.as_str()) || cats.iter().any(|c| &c.0 == chosen) { chosen.clone() } else { ENTIRE_FONT.to_string() }
}

/// The face the panel browses: the font picked in the panel, else the current text's font, else
/// the fallback family, with that family and style. The same family and style can have a different
/// face id at the selection's version or axes; that instance is used when the Type tool's selected
/// character is set in the font.
fn browsed_face(app: &VectorcraftApp) -> (String, String, Option<Arc<FontFace>>) {
    let db = FontDb::global();
    let current = super::character::text_style(app).map(|(s, _)| (s.font_family, s.font_style));
    // The panel browses the current text's font unless another font was picked here.
    let (family, style) =
        app.ui.glyphs_font.clone().or(current).unwrap_or_else(|| (vectorcraft_text::FALLBACK_FAMILY.to_string(), "Regular".to_string()));
    let face = selected_target(app)
        .filter(|t| t.style.font_family.eq_ignore_ascii_case(&family) && t.style.font_style.eq_ignore_ascii_case(&style))
        .and_then(|t| db.style_face(t.style))
        .or_else(|| db.face(&family, &style));
    (family, style, face)
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

/// Glyph `gid` of `face` in a square of `px` pixels: white, with its coverage in the alpha channel.
fn glyph_image(face: &FontFace, gid: u32, px: u32) -> ColorImage {
    let px = px.clamp(1, 1024);
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
    ColorImage::new([px as usize, px as usize], mask.iter().map(|&a| Color32::from_rgba_premultiplied(a, a, a, a)).collect())
}

/// The glyph `face` draws for character `c`.
fn gid_of(face: &FontFace, c: char) -> Option<u32> {
    let all = face.mapped_chars();
    let at = all.binary_search_by_key(&c, |x| x.0).ok()?;
    all.get(at).map(|x| x.1)
}

/// The glyph of `cell` in `face`.
fn gid_of_cell(face: &FontFace, cell: GlyphCell) -> Option<u32> {
    match cell {
        GlyphCell::Char(c) => gid_of(face, c),
        GlyphCell::Glyph(g) => Some(g),
        _ => None,
    }
}

/// Whether a character is listed as a cell: control and white space characters are not.
fn listable(c: char) -> bool {
    !c.is_control() && !c.is_whitespace()
}

/// Whether `c` is in the Unicode-block category `category` (one of [`CATEGORIES`]); false for the
/// others.
fn in_block(c: char, category: &str) -> bool {
    let code = c as u32;
    match category {
        "Punctuation" => c.is_ascii_punctuation() && !"$+<=>^`|~".contains(c) || "¡¿«»·".contains(c) || (0x2010..=0x2027).contains(&code),
        "Numbers" => c.is_numeric(),
        "Currency" => "$¢£¤¥".contains(c) || (0x20A0..=0x20CF).contains(&code),
        "Symbols" => "©®°§¶†‡•".contains(c) || (0x2100..=0x2BFF).contains(&code) && !(0x2190..=0x22FF).contains(&code),
        "Math Symbols" => "+−×÷±<=>¬".contains(c) || (0x2200..=0x22FF).contains(&code),
        "Arrows" => (0x2190..=0x21FF).contains(&code),
        "Latin" => c.is_alphabetic() && code < 0x250,
        "Greek" => (0x370..=0x3FF).contains(&code),
        "Cyrillic" => (0x400..=0x52F).contains(&code),
        "Letterlike Symbols" => (0x2100..=0x214F).contains(&code),
        "Superscripts and Subscripts" => "¹²³".contains(c) || (0x2070..=0x209F).contains(&code),
        "Ornaments" => (0x2766..=0x2767).contains(&code) || code == 0x2042,
        _ => false,
    }
}

/// The character the grid marks: the first one of the Type tool's selected range, else the
/// one before its caret. Walks the runs, not the characters, so it stays cheap on every frame.
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

/// The cell the grid marks: the selected character, or in Alternates for Selection the glyph the
/// character is drawn with.
fn marked_cell(app: &mut VectorcraftApp) -> Option<GlyphCell> {
    if app.ui.glyphs_panel.category == ALTERNATES {
        return alternates_listing(app).ok().map(|l| GlyphCell::Glyph(l.drawn));
    }
    selected_char(app).map(GlyphCell::Char)
}

/// Whether the app has a place to insert a glyph: the Type tool's caret or a selected text.
fn can_insert(app: &VectorcraftApp) -> bool {
    app.session.tool_wants_text() || super::character::text_style(app).is_some()
}

/// Insert `ch` at the Type tool's caret, else append it to the selected text; whether it succeeded.
fn insert(app: &mut VectorcraftApp, ch: char) -> bool {
    let s = ch.to_string();
    if app.session.tool_wants_text() {
        let v = app.view_info();
        let r = app.session.tool_text(&s, v);
        if let Err(e) = &r {
            app.status(e.to_string());
        }
        app.sync_views();
        r.is_ok()
    } else if super::character::text_style(app).is_some() {
        app.run("type.insert", json!({"text": s})).is_ok()
    } else {
        app.ui.status = "Select text or click into text with the Type tool to insert glyphs".into();
        false
    }
}

/// Where the Glyphs panel gets its cells: the faces of the app's text engine. It holds the face the
/// panel shows, what a glyph can do to the selected character and the alternates of that
/// character, all read when it is made, so it borrows nothing from the app. Every method uses the
/// held face whatever family and style it is given: the face of the canvas row or of the selected
/// text can be an instance of variable axes that no family and style name reaches.
pub(crate) struct VcGlyphs<'a> {
    db: &'a FontDb,
    family: String,
    style: String,
    face: Option<Arc<FontFace>>,
    /// What a glyph no character maps can do to the selected character; none for a face without one
    /// and in Alternates for Selection.
    fit: Option<Fit>,
    /// The alternates of the selected character, in Alternates for Selection.
    listing: Option<Listing>,
    /// Shown in place of the grid when there is nothing to list.
    empty: Option<String>,
}

impl VcGlyphs<'static> {
    /// The source of the panel: the browsed face, or the face of the selected character's alternates
    /// in Alternates for Selection.
    pub(crate) fn of(app: &mut VectorcraftApp) -> Self {
        let (family, style, face) = browsed_face(app);
        if app.ui.glyphs_panel.category == ALTERNATES {
            return match alternates_listing(app) {
                Ok(listing) => VcGlyphs {
                    db: FontDb::global(),
                    family: listing.face.family.clone(),
                    style: listing.face.style.clone(),
                    face: Some(listing.face.clone()),
                    fit: None,
                    listing: Some(listing),
                    empty: None,
                },
                Err(note) => VcGlyphs { db: FontDb::global(), family, style, face: None, fit: None, listing: None, empty: Some(note) },
            };
        }
        let fit = face.as_ref().map(|f| Fit::of(app, f));
        let empty = face.is_none().then(|| tl!("No fonts are available.").to_string());
        VcGlyphs { db: FontDb::global(), family, style, face, fit, listing: None, empty }
    }

    /// The source of `face` alone: it draws glyphs and describes nothing about the selection.
    fn bare(face: Arc<FontFace>) -> Self {
        VcGlyphs {
            db: FontDb::global(),
            family: face.family.clone(),
            style: face.style.clone(),
            face: Some(face),
            fit: None,
            listing: None,
            empty: None,
        }
    }
}

impl VcGlyphs<'_> {
    /// The family and style the panel shows.
    fn font(&self) -> (&str, &str) {
        (&self.family, &self.style)
    }
}

impl GlyphSource for VcGlyphs<'_> {
    fn cells(&self, _family: &str, _style: &str, category: &str) -> Vec<GlyphCell> {
        let Some(face) = &self.face else { return vec![] };
        if category == ALTERNATES {
            return self.listing.iter().flat_map(|l| l.variants.iter().map(|v| GlyphCell::Glyph(v.gid))).collect();
        }
        if category == RECENTLY_USED {
            return vec![];
        }
        let all = face.mapped_chars();
        if let Some(found) = face.glyph_categories().iter().find(|c| c.kind.id() == category) {
            let wanted: HashSet<u32> = found.glyphs.iter().copied().collect();
            // A glyph several characters map is listed once, by the first of them; a glyph only a
            // space or control character maps is listed by glyph id, as no character it can insert.
            let mut mapped: HashSet<u32> = HashSet::new();
            let mut cells: Vec<GlyphCell> = all
                .iter()
                .copied()
                .filter(|(c, g)| listable(*c) && wanted.contains(g) && mapped.insert(*g))
                .map(|(c, _)| GlyphCell::Char(c))
                .collect();
            cells.extend(found.glyphs.iter().copied().filter(|g| !mapped.contains(g)).map(GlyphCell::Glyph));
            return cells;
        }
        if CATEGORIES.contains(&category) && category != ENTIRE_FONT {
            return all.iter().map(|x| x.0).filter(|c| listable(*c) && in_block(*c, category)).map(GlyphCell::Char).collect();
        }
        // Entire Font lists the glyphs no character maps (positional forms, alternates,
        // ligatures) by glyph id, after the characters.
        let mut cells: Vec<GlyphCell> = all.iter().map(|x| x.0).filter(|c| listable(*c)).map(GlyphCell::Char).collect();
        cells.extend(face.unencoded_glyphs().iter().copied().map(GlyphCell::Glyph));
        cells
    }

    fn glyph(&self, _family: &str, _style: &str, cell: GlyphCell, px: u32) -> Option<ColorImage> {
        let face = self.face.as_ref()?;
        Some(glyph_image(face, gid_of_cell(face, cell)?, px))
    }

    fn revision(&self) -> u64 {
        self.db.generation()
    }

    fn categories(&self, _family: &str, _style: &str) -> Vec<(String, String)> {
        categories(self.face.as_deref())
    }

    fn describe(&self, _family: &str, _style: &str, cell: GlyphCell) -> CellInfo {
        let mut info = CellInfo::default();
        let Some(face) = &self.face else { return info };
        let Some(gid) = gid_of_cell(face, cell) else { return info };
        info.gid = Some(gid);
        info.has_alternates = face.has_alternates(gid);
        match cell {
            GlyphCell::Char(c) => info.note = Some(encoded_tip(face, c, gid)),
            GlyphCell::Glyph(g) => {
                info.name = Some(glyph_label(face, g));
                info.note = match (&self.listing, &self.fit) {
                    (Some(listing), _) => listing.variants.iter().find(|v| v.gid == g).map(|v| v.tip(tl!("Double-click to replace"))),
                    (None, Some(fit)) => {
                        let mut note = unencoded_tip(face, g);
                        match fit.pick(face, g) {
                            Some(Pick::Glyph(_)) => note.push_str(&format!("\n{}\n{}", tl!("Double-click to replace"), no_feature_note())),
                            Some(_) => note.push_str(&format!("\n{}", tl!("Double-click to replace"))),
                            None => {
                                info.dimmed = true;
                                note.push_str(&format!("\n{}", fit.why(face, g)));
                            }
                        }
                        Some(note)
                    }
                    (None, None) => Some(unencoded_tip(face, g)),
                };
            }
            _ => {}
        }
        info
    }

    fn alternates(&self, _family: &str, _style: &str, cell: GlyphCell) -> Vec<GlyphCell> {
        let Some(face) = &self.face else { return vec![] };
        let Some(gid) = gid_of_cell(face, cell) else { return vec![] };
        let mut list = vec![gid];
        match cell {
            GlyphCell::Char(c) => list.extend(face.alternates(c).into_iter().map(|a| a.gid)),
            GlyphCell::Glyph(g) => list.extend(face.substitutions(g).into_iter().flat_map(|s| s.glyphs)),
            _ => {}
        }
        let mut seen = HashSet::new();
        list.retain(|g| seen.insert(*g));
        list.into_iter().map(GlyphCell::Glyph).collect()
    }
}

/// Browse and insert glyphs from the chosen font, with font controls below the glyph grid.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let db = FontDb::global();
    let marked = marked_cell(app);
    // The grid scrolls to the marked cell only in the pass it moves to another cell, so it still
    // scrolls freely meanwhile.
    let reveal = (marked != app.ui.glyphs_marked).then_some(marked).flatten();
    app.ui.glyphs_marked = marked;
    if app.session.prefs.show_east_asian_options {
        refresh_form_faces(app);
    }
    let source = VcGlyphs::of(app);
    let (mut family, mut style) = {
        let (family, style) = source.font();
        (family.to_string(), style.to_string())
    };
    let (families, styles) = (db.families(), db.styles(&family));
    let can_insert = can_insert(app);
    let mut panel =
        GlyphsPanel::new(tl!("Glyphs"), &source, Bind::bound(&mut family), Bind::bound(&mut style), Bind::bound(&mut app.ui.glyphs_panel))
            .families(&families)
            .styles(&styles)
            .can_insert(can_insert);
    if let Some(cell) = marked {
        panel = panel.marked(cell);
    }
    if let Some(cell) = reveal {
        panel = panel.reveal(cell);
    }
    if let Some(note) = &source.empty {
        panel = panel.empty(note.clone());
    }
    let out = panel.show(ui);
    if out.font_changed {
        app.ui.glyphs_font = Some((family, style));
    }
    if let Some(cell) = out.activated {
        activate(app, cell);
    }
    if let Some(pick) = out.alternate {
        pick_alternate(app, pick.cell, pick.glyph);
    }
}

/// A double-click or Enter on `cell`: insert the character (its letter for an Arabic presentation
/// form), or put the glyph on the selected character.
pub(crate) fn activate(app: &mut VectorcraftApp, cell: GlyphCell) {
    if app.ui.glyphs_panel.category == ALTERNATES {
        match (alternates_listing(app), cell) {
            (Ok(listing), GlyphCell::Glyph(gid)) => listing.apply(app, gid),
            (Err(note), _) => app.status(note),
            _ => {}
        }
        return;
    }
    let (_, _, Some(face)) = browsed_face(app) else { return };
    match cell {
        GlyphCell::Char(c) => {
            if let Some(gid) = gid_of(&face, c) {
                insert_character(app, &face, c, gid);
            }
        }
        GlyphCell::Glyph(gid) => put_on_selection(app, &face, gid),
        _ => {}
    }
}

/// A pick in the alternates pop-up of a cell of the browsed face.
pub(crate) fn pick_alternate(app: &mut VectorcraftApp, cell: GlyphCell, glyph: GlyphCell) {
    let (_, _, Some(face)) = browsed_face(app) else { return };
    let (Some(gid), Some(picked)) = (gid_of_cell(&face, cell), gid_of_cell(&face, glyph)) else { return };
    match cell {
        GlyphCell::Char(c) => insert_alternate(app, &face, c, gid, picked),
        // A glyph no character maps goes on the selected character as a double-click on its cell
        // in Entire Font puts it.
        _ => put_on_selection(app, &face, picked),
    }
}

/// Put glyph `gid` of `face` on the selected character, or say why it cannot go there.
fn put_on_selection(app: &mut VectorcraftApp, face: &FontFace, gid: u32) {
    let fit = Fit::of(app, face);
    match fit.pick(face, gid) {
        Some(pick) => fit.apply(app, pick),
        None => app.status(fit.why(face, gid)),
    }
}

/// One cell of Show › Alternates for Selection: a glyph, the feature that lists it, and the
/// feature tag that draws it on the selected character in its word (`salt=2`), when one does.
struct Variant {
    gid: u32,
    feature: [u8; 4],
    apply: Option<String>,
    /// The glyph the shaper makes for the character with none of its own alternate features.
    default: bool,
}

impl Variant {
    /// What a double-click on the variant does to the selected character.
    fn pick(&self) -> Pick {
        match (&self.apply, self.default) {
            (_, true) => Pick::Default,
            (Some(tag), _) => Pick::Feature(tag.clone()),
            (None, _) => Pick::Glyph(self.gid),
        }
    }

    /// The tooltip: its id, the feature that draws it, and what `action` does.
    fn tip(&self, action: &str) -> String {
        let glyph = crate::i18n::fmt(tl!("glyph {id}"), &[("id", &self.gid.to_string())]);
        let glyph = match (&self.apply, self.default) {
            (_, true) => glyph,
            (Some(tag), _) => format!("{glyph}  ({tag})"),
            (None, _) => format!("{glyph}  ({})", String::from_utf8_lossy(&self.feature)),
        };
        let mut tip = format!("{glyph}\n{action}");
        if !self.default && self.apply.is_none() {
            tip.push('\n');
            tip.push_str(&no_feature_note());
        }
        tip
    }
}

/// Why a glyph no feature draws may not join its neighbors.
fn no_feature_note() -> String {
    tl!("No OpenType feature draws this glyph here, so it replaces the character's glyph and may not join its neighbors.").to_string()
}

/// The variants of a character in its word: the glyph the shaper makes with none of its own
/// alternate features first, then the font's alternates in its order, without the glyph `omit`.
fn variant_list(found: &ContextAlternates, omit: Option<u32>) -> Vec<Variant> {
    let default = (Some(found.default) != omit).then_some(Variant { gid: found.default, feature: [0; 4], apply: None, default: true });
    let others = found.alternates.iter().filter(|a| a.gid != found.default && Some(a.gid) != omit).map(|a| Variant {
        gid: a.gid,
        feature: a.feature,
        apply: a.apply.clone(),
        default: false,
    });
    default.into_iter().chain(others).collect()
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

/// The cells of Show › Alternates for Selection: the alternate glyphs of the Type tool's selected
/// character as it stands in its word ([`vectorcraft_text::alternates_in_context`]: a medial letter
/// lists its medial form's alternates), in the face the text is set in, whatever font the panel
/// browses. The glyph the shaper makes for the character with none of its own alternate features
/// comes first, and the glyph the character is drawn with is marked.
struct Listing {
    id: NodeId,
    /// The character's bytes in the text, start and end.
    bytes: (usize, usize),
    features: Vec<String>,
    has_choice: bool,
    /// The face that supplied the character's glyph (a fallback face for a character the text's
    /// font lacks).
    face: Arc<FontFace>,
    variants: Vec<Variant>,
    /// The glyph the character is drawn with.
    drawn: u32,
}

impl Listing {
    /// Draw the character with glyph `gid` of the listing, as one undo step.
    fn apply(&self, app: &mut VectorcraftApp, gid: u32) {
        if let Some(variant) = self.variants.iter().find(|v| v.gid == gid) {
            replace_glyph(app, self.id, self.bytes, &self.features, self.has_choice, variant.pick());
        }
    }
}

/// The listing for the selected character, or the note that says why there is none.
fn alternates_listing(app: &mut VectorcraftApp) -> Result<Listing, String> {
    let db = FontDb::global();
    let (id, byte, ch, style) = selected_target(app)
        .map(|t| (t.id, t.byte, t.ch, t.style.clone()))
        .ok_or_else(|| tl!("Select a character with the Type tool to see its alternates.").to_string())?;
    let face = db
        .style_face(&style)
        .filter(|_| db.has_family(&style.font_family))
        .ok_or_else(|| tl!("The selected character's font is not available.").to_string())?;
    let has_choice = style.glyph_alternate.is_some();
    let found = alternates_found(app, id, byte, face.id())
        .filter(|f| has_choice || !f.alternates.is_empty())
        .ok_or_else(|| tl!("No alternates for the selected character.").to_string())?;
    let face = db.face_by_id(found.font_id).unwrap_or(face);
    Ok(Listing {
        id,
        bytes: (byte, byte + ch.len_utf8()),
        features: style.features,
        has_choice,
        face,
        variants: variant_list(&found, None),
        drawn: found.current,
    })
}

/// Draw the character at `bytes` of text `id` with `pick`, as one undo step ([`apply_alternate`]);
/// typing in progress stays a step of its own. Whether it worked.
fn replace_glyph(app: &mut VectorcraftApp, id: NodeId, bytes: (usize, usize), features: &[String], has_choice: bool, pick: Pick) -> bool {
    super::character::end_typing(app);
    apply_alternate(app, id, bytes, features, has_choice, pick)
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
    face: Arc<FontFace>,
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
    pub fn gid(&self, i: usize) -> Option<u32> {
        self.cells.get(i).map(|c| c.gid)
    }

    /// The face that draws the offered glyphs.
    pub fn face(&self) -> &Arc<FontFace> {
        &self.face
    }

    /// The source that draws the offered glyphs ([`Self::face`]).
    pub fn source(&self) -> VcGlyphs<'static> {
        VcGlyphs::bare(self.face.clone())
    }

    /// The tooltip of glyph `i`: its id, the feature that draws it, and what a click does.
    pub fn tip(&self, i: usize) -> String {
        self.cells.get(i).map(|c| c.tip(tl!("Click to replace"))).unwrap_or_default()
    }

    /// Draw the character with glyph `i`, as a double-click on that glyph in Show › Alternates for
    /// Selection does: one command, so one undo step.
    pub fn pick(&self, app: &mut VectorcraftApp, i: usize) {
        if let Some(c) = self.cells.get(i) {
            replace_glyph(app, self.id, self.bytes, &self.features, self.has_choice, c.pick());
        }
    }
}

/// What the canvas offers beside the Type tool's selection: the alternates of the selected
/// character as it stands in its word ([`alternates_found`], found once for each document
/// revision), in the font's order and without the glyph the character is drawn with. A character
/// drawn with an alternate gets its default glyph first. `None` unless the Type tool's selection
/// is exactly one character of one type object whose font is available and gives it another glyph,
/// and while Preferences › Type › Show Character Alternates is on.
pub(crate) fn on_canvas(app: &mut VectorcraftApp) -> Option<OnCanvas> {
    if !app.session.prefs.show_character_alternates {
        return None;
    }
    let (_, a, b) = super::character::text_editing(app)?;
    if a == b || super::character::text_story(app).is_some() {
        return None;
    }
    let (id, byte, ch, style) = selected_target(app).map(|t| (t.id, t.byte, t.ch, t.style.clone()))?;
    let (start, end) = (a.min(b), a.max(b));
    if byte != start || start + ch.len_utf8() != end {
        return None;
    }
    let db = FontDb::global();
    let face = db.style_face(&style).filter(|_| db.has_family(&style.font_family))?;
    let found = alternates_found(app, id, start, face.id())?;
    let face = db.face_by_id(found.font_id).unwrap_or(face);
    let mut cells = variant_list(&found, Some(found.current));
    if cells.is_empty() {
        return None;
    }
    let more = cells.len() > ON_CANVAS_ALTERNATES;
    cells.truncate(ON_CANVAS_ALTERNATES);
    Some(OnCanvas { id, bytes: (start, end), face, cells, more, features: style.features, has_choice: style.glyph_alternate.is_some() })
}

/// Show the Glyphs panel on Show › Alternates for Selection, for the canvas's control that
/// stands for the alternates it leaves out. A panel that is already shown stays where it is.
pub(crate) fn show_alternates(app: &mut VectorcraftApp) {
    app.ui.glyphs_panel.category = ALTERNATES.to_string();
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
    found: Option<ContextAlternates>,
}

impl Fit {
    fn of(app: &mut VectorcraftApp, face: &FontFace) -> Self {
        let db = FontDb::global();
        let target = selected_target(app).map(|t| (t.id, t.byte, t.ch, t.style.clone())).map(|(id, byte, ch, style)| {
            let text_face = db.style_face(&style).filter(|_| db.has_family(&style.font_family));
            let found = text_face.as_ref().and_then(|f| alternates_found(app, id, byte, f.id()));
            FitTarget {
                id,
                byte,
                ch,
                has_choice: style.glyph_alternate.is_some(),
                features: style.features,
                family: style.font_family,
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
    fn pick(&self, face: &FontFace, gid: u32) -> Option<Pick> {
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
    fn why(&self, face: &FontFace, gid: u32) -> String {
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
    /// for Selection does.
    fn apply(&self, app: &mut VectorcraftApp, pick: Pick) {
        let Some(t) = &self.target else { return };
        replace_glyph(app, t.id, (t.byte, t.byte + t.ch.len_utf8()), &t.features, t.has_choice, pick);
    }
}

/// `name` of glyph `gid`, or `gid N`.
fn glyph_label(face: &FontFace, gid: u32) -> String {
    face.glyph_name(gid).unwrap_or_else(|| format!("gid {gid}"))
}

/// The letter to insert in place of character `c`, an Arabic presentation form: the letter whose
/// glyph the font substitutes with `init`, `medi`, `fina` or `isol` to get `c`'s glyph. The
/// presentation form itself is a fixed shape that neither joins its neighbors nor is joined by
/// them; the letter takes the form its place in the word asks for, so the word stays joined.
/// `None` when `c` is not such a form in this font.
fn base_letter(face: &FontFace, c: char, gid: u32) -> Option<char> {
    if !matches!(c, '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}') {
        return None;
    }
    let origin = face.glyph_origin(gid)?;
    origin.position()?;
    (origin.ch != c).then_some(origin.ch)
}

/// The tooltip of an encoded glyph (a character): its code point and glyph, and what a
/// double-click inserts.
fn encoded_tip(face: &FontFace, c: char, gid: u32) -> String {
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
fn insert_character(app: &mut VectorcraftApp, face: &FontFace, c: char, gid: u32) {
    match base_letter(face, c, gid) {
        Some(base) => {
            if insert(app, base) {
                app.status(crate::i18n::fmt(
                    tl!("Inserted {base} (U+{code}) instead of a presentation form: the font joins it to its neighbors by position."),
                    &[("base", &base.to_string()), ("code", &format!("{:04X}", base as u32))],
                ));
            }
        }
        None => {
            insert(app, c);
        }
    }
}

/// A pick in the pop-up of character `c`'s cell, whose glyph is `gid`. The cell's own glyph
/// inserts the character as a double-click on the cell does ([`insert_character`]). An alternate
/// needs the Type tool's caret or selection in text set in `face`: when the selection is exactly
/// one `c` in one type object, that character is drawn with the glyph, and the selection follows
/// it to the frame the text flows it to; else `c` goes in at the caret, in place of the selection
/// (with the rest of a threaded story that Select All selected), and is drawn with it, the two as
/// one undo step. The glyph goes on as a double-click in Show › Alternates for Selection puts it
/// ([`draw_selected_with`]). An Arabic presentation form goes in as its base letter, as a
/// double-click on its cell puts it, drawn with the glyph where the font gives it that glyph in its
/// word. Without a caret, with text in another font, or with a glyph the character cannot be drawn
/// with there, the status says why and nothing changes.
fn insert_alternate(app: &mut VectorcraftApp, face: &FontFace, c: char, gid: u32, picked: u32) {
    if picked == gid {
        return insert_character(app, face, c, gid);
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
fn put_alternate(app: &mut VectorcraftApp, face: &FontFace, c: char, picked: u32, id: NodeId, (lo, hi): (usize, usize), story: &[NodeId]) {
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
    face: &FontFace,
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
fn draw_selected_with(app: &mut VectorcraftApp, face: &FontFace, c: char, picked: u32) -> bool {
    let Some(t) = selected_target(app).filter(|t| t.ch == c) else { return false };
    let (id, byte, features, has_choice) = (t.id, t.byte, t.style.features.clone(), t.style.glyph_alternate.is_some());
    let pick = match app.session.active().and_then(|s| s.doc.node(id)).map(|node| &node.kind) {
        Some(NodeKind::Text(text)) => pick_for(text, byte, face, picked),
        _ => Pick::Glyph(picked),
    };
    replace_glyph(app, id, (byte, byte + c.len_utf8()), &features, has_choice, pick)
}

/// How glyph `picked` of `face` goes on the character at `byte` of `text`: the OpenType feature
/// that draws it there in its word, the default glyph, else a glyph choice.
fn pick_for(text: &vectorcraft_text::TextObject, byte: usize, face: &FontFace, picked: u32) -> Pick {
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
fn unencoded_tip(face: &FontFace, gid: u32) -> String {
    let name = face.glyph_name(gid).unwrap_or_else(|| format!("gid {gid}"));
    let glyph = crate::i18n::fmt(tl!("glyph {id}"), &[("id", &gid.to_string())]);
    match face.feature_reaching(gid) {
        Some(feature) => format!("{name}  ({glyph})  ·  {}", String::from_utf8_lossy(&feature)),
        None => format!("{name}  ({glyph})"),
    }
}

/// The alternates of the character at `byte` of text `id` in its word, and the glyph it is drawn
/// with ([`vectorcraft_text::alternates_in_context`]). Kept in the canvas cache until the
/// document, the character or the face changes.
fn alternates_found(app: &mut VectorcraftApp, id: NodeId, byte: usize, face_id: u32) -> Option<ContextAlternates> {
    let state = app.session.active()?;
    let key: FoundKey = (state.uid, state.revision, id, byte, face_id);
    if let Some((cached, found)) = &app.canvas.glyphs_found
        && *cached == key
    {
        return found.clone();
    }
    let found = state.doc.node(id).and_then(|node| match &node.kind {
        NodeKind::Text(text) => vectorcraft_text::alternates_in_context(FontDb::global(), text, byte),
        _ => None,
    });
    app.canvas.glyphs_found = Some((key, found.clone()));
    found
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

/// What [`form_face_ids`] was read for: the key of [`crate::caches::CanvasCache::glyph_form_faces`].
fn faces_key(app: &VectorcraftApp) -> Option<FacesKey> {
    let editing = super::character::text_editing(app).map(|(id, a, b)| (id, a.min(b), a.max(b)));
    let state = app.session.active()?;
    let stories = match editing {
        Some(_) => super::character::text_story(app).unwrap_or_default(),
        None => super::character::selected_stories(app),
    };
    Some((state.uid, state.revision, FontDb::global().generation(), editing, stories))
}

/// The ids of the faces that draw the characters the glyph form rows act on: the faces the
/// fonts of their styles name, and the faces layout draws them with, fallback faces included:
/// the Type tool's selected range (every frame of its story after Select All in threaded text,
/// the whole text object at a caret), else every frame of the selected stories. Overflow text is
/// included.
fn form_face_ids(app: &VectorcraftApp) -> Vec<u32> {
    let editing = super::character::text_editing(app).map(|(id, a, b)| (id, a.min(b), a.max(b)));
    let Some(state) = app.session.active() else { return vec![] };
    let story = super::character::text_story(app);
    let db = FontDb::global();
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
    ids
}

/// Read the faces of the selection again when the document, the selection or the fonts changed:
/// laying the text out is too much to do for each menu state. The shown panel does it, and a panel
/// menu opens only from a shown panel.
fn refresh_form_faces(app: &mut VectorcraftApp) {
    let Some(key) = faces_key(app) else {
        app.canvas.glyph_form_faces = None;
        return;
    };
    if app.canvas.glyph_form_faces.as_ref().is_some_and(|(cached, _)| *cached == key) {
        return;
    }
    let ids = form_face_ids(app);
    app.canvas.glyph_form_faces = Some((key, ids));
}

/// The faces [`refresh_form_faces`] read, when they are those of the selection now.
fn form_faces(app: &VectorcraftApp) -> Option<Vec<Arc<FontFace>>> {
    let key = faces_key(app)?;
    let (cached, ids) = app.canvas.glyph_form_faces.as_ref()?;
    (*cached == key).then(|| ids.iter().filter_map(|id| FontDb::global().face_by_id(*id)).collect())
}

/// The value `f` gives every character the glyph form rows act on: the Type tool's selected
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

/// The East Asian glyph form every character the glyph form rows act on has ([`selection_shared`]).
fn selection_glyph_form(app: &VectorcraftApp) -> Option<GlyphForm> {
    selection_shared(app, |s| s.glyph_form)
}

/// Does choosing `form` change the characters the glyph form rows act on? Not when they all have
/// it already, unless one of them has a glyph choice, which a form other than the default takes
/// away.
fn glyph_form_changes(app: &VectorcraftApp, form: GlyphForm) -> bool {
    selection_glyph_form(app) != Some(form) || (form != GlyphForm::Default && selection_shared(app, |s| s.glyph_alternate.is_none()) != Some(true))
}

/// The command that shows category `id`.
fn show_ref(id: &str) -> CommandRef {
    CommandRef::with_params("ui.glyphs.show", json!({"category": id}))
}

/// The command that sets East Asian glyph form `form`.
fn form_ref(form: GlyphForm) -> CommandRef {
    CommandRef::with_params("ui.glyphs.glyphForm", json!({"form": form.id()}))
}

/// The rows of the panel menu: the text's font, with Show East Asian Options the East Asian glyph
/// forms (each draws the selected characters, all the text of the selected type objects without a
/// Type tool range, in one form of their font; Revert To Default Forms draws the font's default
/// glyphs), then the categories of Show and the OpenType categories the browsed font has.
/// Public behavior: [Format East Asian characters](https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html)
/// (Replace Asian characters with a different glyph form).
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = vec![PanelRow::Command(CommandRef::new("ui.glyphs.useTextFont")), PanelRow::Separator];
    if app.session.prefs.show_east_asian_options {
        for form in GlyphForm::ALL {
            if form == GlyphForm::Default {
                rows.push(PanelRow::Separator);
            }
            rows.push(PanelRow::Command(form_ref(form)));
        }
        rows.push(PanelRow::Separator);
    }
    rows.extend(CATEGORIES.iter().map(|id| PanelRow::Command(show_ref(id))));
    let (_, _, face) = browsed_face(app);
    let cats = categories(face.as_deref());
    if !cats.is_empty() {
        rows.push(PanelRow::Separator);
        rows.extend(cats.iter().map(|(id, _)| PanelRow::Command(show_ref(id))));
    }
    rows
}

/// The row titles of a category, which are the panel's words, with the check on the shown one. A
/// form a face of the selection has no feature for is disabled, once the shown panel has read the
/// faces ([`refresh_form_faces`]).
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let (_, _, face) = browsed_face(app);
    let cats = categories(face.as_deref());
    let current = current_category(app, &cats);
    let words = app.ui_ctx.as_ref().map(|ctx| Env::of_root(ctx).words::<GlyphsWords>());
    for id in CATEGORIES {
        let title = words.as_ref().map_or(*id, |w| w.category_name(id)).to_string();
        states.set(show_ref(id), CommandState::new().title(title).checked(current == *id));
    }
    for (id, label) in &cats {
        states.set(show_ref(id), CommandState::new().title(label.clone()).checked(current == *id));
    }
    if !app.session.prefs.show_east_asian_options {
        return;
    }
    let has = super::character::text_style(app).is_some();
    let faces = form_faces(app);
    let shared = selection_glyph_form(app);
    for form in GlyphForm::ALL {
        // A width form may be drawn with other widths for the same glyphs (`GPOS`) instead of
        // other glyphs.
        let enabled = match (form.feature(), &faces) {
            (Some(tag), Some(faces)) => {
                faces.iter().any(|f| f.has_substitution_feature(*tag) || (form.is_width() && f.has_positioning_feature(*tag)))
            }
            (Some(_), None) => true,
            (None, _) => has,
        };
        let checked = form != GlyphForm::Default && shared == Some(form);
        states.set(form_ref(form), CommandState::new().title(tl!(glyph_form_label(form))).enabled(enabled).checked(checked));
    }
}

/// Run a Glyphs panel menu command: `ui.glyphs.useTextFont`, `ui.glyphs.show {category}` and
/// `ui.glyphs.glyphForm {form}`.
// PLAN: assumed signature `run(app, id, params) -> Result<(), String>`, called by hub-w2's menus/run.rs
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<(), String> {
    match id {
        "ui.glyphs.useTextFont" => app.ui.glyphs_font = None,
        "ui.glyphs.show" => {
            let category = params.get("category").and_then(Value::as_str).ok_or("ui.glyphs.show needs a category")?;
            let (_, _, face) = browsed_face(app);
            // A category the browsed font lacks is Entire Font.
            let known = CATEGORIES.contains(&category) || categories(face.as_deref()).iter().any(|c| c.0 == category);
            app.ui.glyphs_panel.category = if known { category } else { ENTIRE_FONT }.to_string();
        }
        "ui.glyphs.glyphForm" => {
            let wanted = params.get("form").and_then(Value::as_str).ok_or("ui.glyphs.glyphForm needs a form")?;
            let form = GlyphForm::ALL.into_iter().find(|f| f.id() == wanted).ok_or_else(|| format!("unknown glyph form `{wanted}`"))?;
            // Choosing the form every selected character already has changes nothing.
            if glyph_form_changes(app, form) {
                super::character::format(app, json!({"glyphForm": form.id()}));
            }
        }
        _ => return Err(format!("unknown command `{id}`")),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
