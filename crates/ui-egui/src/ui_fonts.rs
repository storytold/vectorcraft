//! Fallback fonts for the app's own text: layer and file names (and anything else the UI shows) in
//! Chinese, Japanese, Korean or other scripts the UI fonts lack are drawn in an installed font that
//! covers them. Builds made with craft-fonts (`CRAFT_FONTS_DIR`, all releases) carry Japanese fonts,
//! which `theme::install_fonts` adds after the UI fonts, so Japanese never gets here; nothing else
//! is bundled for them: the first time a frame paints a character the UI fonts
//! don't have, an installed font covering it is looked for (in the background on native) and added
//! to egui's families as their last fallback, on the baseline of each family's first face, from
//! the next frame on. Text the UI fonts cover costs no font loading or memory. On the web, which
//! has no system fonts, the shell is told which characters no loaded font covers
//! ([`on_missing_glyphs`]) and fetches a face for them (Korean, Devanagari) when it has one; until
//! a face arrives they stay missing-glyph boxes (Japanese too, unless built with craft-fonts).
//! Characters no font covered are looked for again whenever the font database changes.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use egui::epaint::Shape;
use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{FontData, FontDefinitions, FontFamily, FontId, FontTweak, LayerId};
use vectorcraft_text::{FontDb, FontFace};

/// Called with the painted characters no loaded or installed font covers.
static MISSING_GLYPHS: OnceLock<fn(&[char])> = OnceLock::new();

/// Have `f` called (on the UI thread, after a frame) with the characters the interface painted
/// that no loaded or installed font covers, so it can load a font for them: the web shell fetches
/// its `demand` craft-fonts faces. A character is reported again only after the fonts change and it
/// is still not covered. The first call sets it; later calls are ignored.
pub fn on_missing_glyphs(f: fn(&[char])) {
    let _ = MISSING_GLYPHS.set(f);
}

/// What one search for fallback faces found.
struct Found {
    /// The faces covering the characters looked for, each once.
    faces: Vec<Arc<FontFace>>,
    /// The characters no face covers.
    uncovered: Vec<char>,
    /// [`FontDb::generation`] when the search ended.
    generation: u64,
}

#[derive(Default)]
pub(crate) struct UiFonts {
    /// Characters looked at already: the UI fonts have them, a fallback was added (or is being
    /// looked for), or no font has them.
    seen: HashSet<char>,
    /// Characters no font covered at the last search: looked for again once the font database's
    /// generation is no longer [`UiFonts::searched`].
    uncovered: HashSet<char>,
    /// The font database's generation when the last search ended.
    searched: u64,
    /// The fonts added to egui, by name.
    added: HashSet<String>,
    /// The faces covering the characters being looked for (native: looked for on a thread).
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<std::sync::mpsc::Receiver<Found>>,
}

impl UiFonts {
    /// After a frame is laid out: add the fallback fonts found since the last frame, and look for
    /// fonts covering the characters this frame painted that the UI fonts lack.
    pub(crate) fn frame(&mut self, ctx: &egui::Context) {
        self.frame_in(ctx, FontDb::global(), MISSING_GLYPHS.get().copied().unwrap_or(|_| {}));
    }

    /// [`UiFonts::frame`] looking through `db`, reporting uncovered characters to `report`.
    fn frame_in(&mut self, ctx: &egui::Context, db: &'static FontDb, report: fn(&[char])) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(found) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            self.found(ctx, found, report);
        }
        let mut missing = self.missing(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        let idle = self.pending.is_none();
        #[cfg(target_arch = "wasm32")]
        let idle = true;
        // A font added since (a web face fetched for them) may cover them now.
        if idle && !self.uncovered.is_empty() && db.generation() != self.searched {
            missing.extend(self.uncovered.drain());
            missing.sort_unstable();
            missing.dedup();
        }
        if missing.is_empty() {
            return;
        }
        let find = move || {
            let (mut faces, mut uncovered): (Vec<Arc<FontFace>>, Vec<char>) = (vec![], vec![]);
            for c in missing {
                match db.face_covering(c) {
                    Some(f) if !faces.iter().any(|x| x.id() == f.id()) => faces.push(f),
                    Some(_) => {}
                    None => uncovered.push(c),
                }
            }
            Found { faces, uncovered, generation: db.generation() }
        };
        // Native: the installed fonts are read off the UI thread. Characters met meanwhile wait for
        // the next search (they aren't marked seen until then).
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let repaint = ctx.clone();
            let spawned = std::thread::Builder::new().name("ui-font-fallback".into()).spawn(move || {
                // The receiver is gone only when the app is.
                let _ = tx.send(find());
                repaint.request_repaint();
            });
            if spawned.is_ok() {
                self.pending = Some(rx);
            }
        }
        // The web has only the loaded fonts to look through: quick, on this thread.
        #[cfg(target_arch = "wasm32")]
        self.found(ctx, find(), report);
    }

    /// Add the faces a search found, and keep (and report) the characters it found no face for.
    fn found(&mut self, ctx: &egui::Context, found: Found, report: fn(&[char])) {
        self.install(ctx, &found.faces);
        self.searched = found.generation;
        if !found.uncovered.is_empty() {
            report(&found.uncovered);
            self.uncovered.extend(found.uncovered);
        }
    }

    /// Characters painted this frame that the UI fonts lack and haven't been looked for, now
    /// marked seen. None while a search is running.
    fn missing(&mut self, ctx: &egui::Context) -> Vec<char> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.pending.is_some() {
            return vec![];
        }
        let mut layers: Vec<LayerId> = ctx.memory(|m| m.layer_ids().collect());
        if !layers.contains(&LayerId::background()) {
            layers.push(LayerId::background());
        }
        let mut chars: Vec<char> = vec![];
        let seen = &self.seen;
        let mut note = |text: &str| {
            // Most UI text is ASCII, which the UI fonts cover.
            if !text.is_ascii() {
                chars.extend(text.chars().filter(|c| !c.is_ascii() && !c.is_whitespace() && !c.is_control() && !seen.contains(c)));
            }
        };
        ctx.graphics(|g| {
            for l in &layers {
                for s in g.get(*l).into_iter().flat_map(|list| list.all_entries()) {
                    painted_text(&s.shape, &mut note);
                }
            }
        });
        if chars.is_empty() {
            return chars;
        }
        chars.sort_unstable();
        chars.dedup();
        let font = FontId::new(13.0, FontFamily::Proportional);
        let missing: Vec<char> = ctx.fonts_mut(|f| chars.iter().copied().filter(|c| !f.has_glyph(&font, *c)).collect());
        self.seen.extend(chars);
        missing
    }

    /// Add `faces` to every egui family, after the fonts it has, on the baseline of the family's
    /// first face.
    fn install(&mut self, ctx: &egui::Context, faces: &[Arc<FontFace>]) {
        for face in faces {
            let name = format!("{} {} #{}", face.family, face.style, face.face_index());
            if !self.added.insert(name.clone()) {
                continue;
            }
            // The face's file data, shared with the process-wide font database (which keeps every
            // face it loads for the session) instead of copied: one handle is leaked per font added.
            let face: &'static FontFace = Box::leak(Box::new(face.clone()));
            let data = FontData { index: face.face_index(), ..FontData::from_static(face.file_data()) };
            for (name, data, families) in ctx.fonts(|f| baseline_copies(f.definitions(), &name, &data)) {
                let families = families.into_iter().map(|family| InsertFontFamily { family, priority: FontPriority::Lowest }).collect();
                ctx.add_font(FontInsert { name, data, families });
            }
        }
        if !faces.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Wait for the font search running, if any, and add what it found (tests).
    #[cfg(test)]
    pub(crate) fn finish(&mut self, ctx: &egui::Context) {
        self.finish_with(ctx, |_| {});
    }

    /// [`UiFonts::finish`], reporting uncovered characters to `report` (tests).
    #[cfg(test)]
    fn finish_with(&mut self, ctx: &egui::Context, report: fn(&[char])) {
        if let Some(found) = self.pending.take().and_then(|rx| rx.recv().ok()) {
            self.found(ctx, found, report);
        }
    }
}

/// The copies of the fallback face `data` that `defs`' families take, one for each baseline they
/// need: each copy is `data` with the `y_offset_factor` that puts its glyphs on the baseline of the
/// first face of the families listed with it. Families whose first faces need the same shift share
/// a copy. The first copy (the Proportional family's, when `defs` has it) is named `name`; each
/// other copy adds the name of its families' first face.
pub(crate) fn baseline_copies(defs: &FontDefinitions, name: &str, data: &FontData) -> Vec<(String, FontData, Vec<FontFamily>)> {
    let mut copies: Vec<(String, FontData, Vec<FontFamily>)> = vec![];
    for (family, stack) in &defs.families {
        let head = stack.first().and_then(|head| Some((head.as_str(), defs.font_data.get(head)?)));
        let shift = head.and_then(|(_, data)| row_metrics(data)).map_or(0.0, |row| baseline_shift(row, (*data.font).as_ref(), data.index));
        if let Some((_, _, families)) = copies.iter_mut().find(|(_, copy, _)| copy.tweak.y_offset_factor.to_bits() == shift.to_bits()) {
            families.push(family.clone());
            continue;
        }
        let copy_name = if copies.is_empty() { name.to_owned() } else { format!("{name} / {}", head.map_or("", |(head, _)| head)) };
        let tweak = FontTweak { y_offset_factor: shift, ..data.tweak.clone() };
        copies.push((copy_name, FontData { tweak, ..data.clone() }, vec![family.clone()]));
    }
    copies
}

/// The ascent, row height and vertical offset of a family's first face `data`, in ems of the font
/// size, with that face's `FontTweak` scale applied, as egui lays out its rows. `None` when the
/// face can't be read.
fn row_metrics(data: &FontData) -> Option<(f32, f32, f32)> {
    let (ascent, height) = em_metrics((*data.font).as_ref(), data.index)?;
    let scale = data.tweak.scale;
    Some((ascent * scale, height * scale, data.tweak.y_offset_factor * scale))
}

/// The ascent and row height (ascent, descent and line gap) of the face at `index` of `data`, in ems.
fn em_metrics(data: &[u8], index: u32) -> Option<(f32, f32)> {
    let (upem, ascent, descent, gap) = vectorcraft_text::line_metrics(data, index)?;
    Some((ascent / upem, (ascent - descent + gap) / upem))
}

/// The `y_offset_factor` that puts the glyphs of the face at `index` of `face` on the baseline of
/// the row's first face, whose ascent, row height and offset in ems are `row`. egui draws a
/// fallback glyph's baseline at its face's ascent plus half the difference between the two row
/// heights, so a face with a taller ascent or descent sits off the baseline by the shift computed
/// here, in ems of the font size, limited to half an em either way.
fn baseline_shift(row: (f32, f32, f32), face: &[u8], index: u32) -> f32 {
    let (row_ascent, row_height, row_offset) = row;
    let shift = em_metrics(face, index).map_or(0.0, |(ascent, height)| row_ascent + row_offset - (ascent + 0.5 * (row_height - height)));
    if shift.is_finite() { shift.clamp(-0.5, 0.5) } else { 0.0 }
}

/// Call `f` with the text of `shape` and the shapes it holds.
fn painted_text(shape: &Shape, f: &mut impl FnMut(&str)) {
    match shape {
        Shape::Text(t) => f(t.galley.text()),
        Shape::Vec(v) => v.iter().for_each(|s| painted_text(s, f)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a frame painting `text`, then the fallback search after it.
    fn frame(ctx: &egui::Context, fonts: &mut UiFonts, text: &str) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(text);
            fonts.frame(ui.ctx());
        });
        out.textures_delta.clear();
    }

    fn has_glyph(ctx: &egui::Context, c: char) -> bool {
        ctx.fonts_mut(|f| f.has_glyph(&FontId::new(13.0, FontFamily::Proportional), c))
    }

    #[test]
    fn latin_text_loads_no_fonts() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut fonts = UiFonts::default();
        frame(&ctx, &mut fonts, "Layer 1 — Café");
        assert!(fonts.pending.is_none() && fonts.added.is_empty());
    }

    #[test]
    fn hindi_ui_loads_a_devanagari_fallback_in_every_family() {
        if FontDb::global().face_covering('ह').is_none() {
            eprintln!("skipped: install a Devanagari font to check the native Hindi UI fallback");
            return;
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut fonts = UiFonts::default();
        // Conjuncts, pre-base matras, reph, nukta and mixed Latin/numeric labels.
        let text = "हिन्दी क्षि त्रि क्रि प्रिंट फ़ाइल लेयर RGB 123";
        frame(&ctx, &mut fonts, text);
        fonts.finish(&ctx);
        frame(&ctx, &mut fonts, text);
        let families: Vec<_> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
        for family in families {
            let id = FontId::new(13.0, family.clone());
            for c in text.chars().filter(|c| !c.is_whitespace()) {
                assert!(ctx.fonts_mut(|f| f.has_glyph(&id, c)), "{family:?}: missing {c}");
            }
            let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.into(), id, egui::Color32::WHITE));
            assert!(galley.rect.width().is_finite() && galley.rect.width() > 0.0);
        }
    }

    #[test]
    fn japanese_renders_with_the_craft_fonts_without_installed_fonts() {
        if !vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.is_japanese()) {
            eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
            return;
        }
        let text = "日本語の文字";
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut fonts = UiFonts::default();
        frame(&ctx, &mut fonts, text);
        // Real glyphs (no tofu) in every family from the first frame, with no installed font looked for.
        assert!(fonts.pending.is_none() && fonts.added.is_empty(), "no system-font search");
        let families: Vec<FontFamily> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
        for family in families {
            let stack = ctx.fonts(|f| f.definitions().families.get(&family).cloned().unwrap_or_default());
            assert!(stack.last().is_some_and(|n| n.starts_with("craft-fonts ")), "{family:?}: craft-fonts last: {stack:?}");
            for c in text.chars() {
                assert!(ctx.fonts_mut(|f| f.has_glyph(&FontId::new(13.0, family.clone()), c)), "{family:?} draws {c}");
            }
        }
        // The UI prefers BIZ UDPGothic.
        let stack = ctx.fonts(|f| f.definitions().families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
        let first = stack.iter().find(|n| n.starts_with("craft-fonts ")).cloned().unwrap_or_default();
        assert_eq!(first, "craft-fonts BIZ UDPGothic Regular", "{stack:?}");
    }

    #[test]
    fn the_ui_works_without_the_craft_fonts() {
        // Built either way, the fonts install and Japanese text lays out (as installed-font
        // fallbacks or missing-glyph boxes when no font has it).
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut fonts = UiFonts::default();
        frame(&ctx, &mut fonts, "日本語の文字 Layer 1");
        fonts.finish(&ctx);
        frame(&ctx, &mut fonts, "日本語の文字 Layer 1");
        if vectorcraft_text::CRAFT_FONTS.is_empty() {
            let names: Vec<String> = ctx.fonts(|f| f.definitions().font_data.keys().cloned().collect());
            assert!(names.iter().all(|n| !n.starts_with("craft-fonts")), "{names:?}");
        }
        assert!(has_glyph(&ctx, 'L'));
    }

    /// An installed face with a taller ascent and descent than the UI faces (a CJK system face's
    /// proportions) draws on the baseline of each family's first face, the monospace families'
    /// included: `H` and the test face's 编 (the same `H` outline, which stands on the baseline) end
    /// at the same height.
    #[test]
    fn fallback_faces_sit_on_the_ui_baseline() {
        use vectorcraft_text::test_fonts::{TALL_IDEOGRAPH_FAMILY, tall_ideograph_font};
        let db = FontDb::with_font_dirs(vec![]);
        db.add_font(tall_ideograph_font().expect("the test face builds"));
        let face = db.face(TALL_IDEOGRAPH_FAMILY, "Regular").expect("the test face loads");
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        crate::theme::install_fonts(&ctx);
        let mut fonts = UiFonts::default();
        frame(&ctx, &mut fonts, "H");
        assert!(!has_glyph(&ctx, '编'), "no UI face has 编");
        fonts.install(&ctx, &[face]);
        frame(&ctx, &mut fonts, "H");
        assert!(has_glyph(&ctx, '编'), "the test face draws 编");
        let bottom = |g: &egui::epaint::text::Glyph| g.pos.y + g.uv_rect.offset.y + g.uv_rect.size.y;
        let families: Vec<FontFamily> = ctx.fonts(|f| f.definitions().families.keys().cloned().collect());
        for family in families {
            for size in [13.0_f32, 24.0, 40.0] {
                let galley = ctx.fonts_mut(|f| f.layout_no_wrap("H编".into(), FontId::new(size, family.clone()), egui::Color32::WHITE));
                let row = &galley.rows[0].glyphs;
                assert_eq!(row.len(), 2, "H编 lays out as two glyphs");
                // Half a point: one device pixel at 2 pixels per point, for each glyph's rounding.
                let drift = bottom(&row[1]) - bottom(&row[0]);
                assert!(drift.abs() <= 0.5, "{family:?} at {size} pt: 编 ends {drift:.2} pt below H (negative: above)");
            }
        }
    }

    /// The bundled craft-fonts faces draw on the baseline of each family's first face: a glyph of
    /// each ends as far below `H`'s baseline as it does below its own face's baseline when that face
    /// heads a family.
    #[test]
    fn craft_fonts_sit_on_the_baseline_of_each_family() {
        use vectorcraft_text::CraftFont;
        use vectorcraft_text::craft_fonts::{georgian_ui_fonts, japanese_ui_fonts};
        // The face each family draws a script in, with a character of it: (regular face, semibold
        // family's face, character).
        let first = |faces: Vec<&'static CraftFont>| faces.first().copied();
        let deva = vectorcraft_text::CRAFT_FONTS.iter().find(|f| f.scripts.contains(&"Deva"));
        let probes: Vec<(&CraftFont, &CraftFont, char)> = [
            first(japanese_ui_fonts(false)).zip(first(japanese_ui_fonts(true))).map(|(regular, bold)| (regular, bold, '編')),
            first(georgian_ui_fonts()).map(|f| (f, f, 'ა')),
            deva.map(|f| (f, f, 'क')),
        ]
        .into_iter()
        .flatten()
        .collect();
        if probes.is_empty() {
            eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
            return;
        }
        let mut defs = crate::theme::interface_fonts();
        let families: Vec<FontFamily> = defs.families.keys().cloned().collect();
        let probe_name = |f: &CraftFont| format!("probe {}", crate::theme::craft_font_name(f));
        let probe = |f: &CraftFont| FontFamily::Name(probe_name(f).into());
        for &(regular, bold, _) in &probes {
            for f in [regular, bold] {
                defs.font_data.insert(probe_name(f), Arc::new(FontData::from_static(f.bytes)));
                defs.families.insert(probe(f), vec![probe_name(f)]);
            }
        }
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        ctx.set_fonts(defs);
        frame(&ctx, &mut UiFonts::default(), "H");
        let bottom = |g: &egui::epaint::text::Glyph| g.pos.y + g.uv_rect.offset.y + g.uv_rect.size.y;
        let glyphs = |text: &str, font: FontId| ctx.fonts_mut(|f| f.layout_no_wrap(text.into(), font, egui::Color32::WHITE)).rows[0].glyphs.clone();
        for family in families {
            let semibold = family == FontFamily::Name(crate::theme::FONT_UI_SEMIBOLD.into());
            for &(regular, bold, c) in &probes {
                let face = if semibold { bold } else { regular };
                for size in [13.0_f32, 24.0, 40.0] {
                    let own = glyphs(&c.to_string(), FontId::new(size, probe(face)));
                    let row = glyphs(&format!("H{c}"), FontId::new(size, family.clone()));
                    assert_eq!((own.len(), row.len()), (1, 2), "{c} lays out as one glyph");
                    // Half a point: one device pixel at 2 pixels per point, for each glyph's rounding.
                    let drift = (bottom(&row[1]) - row[0].pos.y) - (bottom(&own[0]) - own[0].pos.y);
                    assert!(drift.abs() <= 0.5, "{family:?} at {size} pt: {c} ({}) ends {drift:.2} pt below its baseline", face.family);
                }
            }
        }
    }

    #[test]
    fn cjk_names_get_an_installed_fallback_font() {
        let text = "Logo 标志 ロゴ 로고";
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut fonts = UiFonts::default();
        frame(&ctx, &mut fonts, text);
        assert!(!has_glyph(&ctx, '标'), "the UI fonts have no CJK");
        fonts.finish(&ctx);
        // The fonts arrive with the next frame.
        frame(&ctx, &mut fonts, text);
        let installed: Vec<char> = text.chars().filter(|c| !c.is_ascii() && FontDb::global().face_covering(*c).is_some()).collect();
        if installed.is_empty() {
            eprintln!("no installed font covers {text}: nothing to check");
            return;
        }
        for c in installed {
            assert!(has_glyph(&ctx, c), "{c} has a glyph");
        }
        // Looked for once: a later frame starts no search, unless characters no font covered are
        // looked for again because the fonts changed (other tests add fonts meanwhile).
        let searched = fonts.searched;
        frame(&ctx, &mut fonts, text);
        assert!(fonts.pending.is_none() || FontDb::global().generation() != searched, "a search with the fonts unchanged");
    }

    thread_local! {
        static REPORTED: std::cell::RefCell<Vec<char>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn report(chars: &[char]) {
        REPORTED.with(|r| r.borrow_mut().extend_from_slice(chars));
    }

    /// Run a frame painting `text` with fallbacks looked for in `db`, then wait for the search.
    fn frame_in(ctx: &egui::Context, fonts: &mut UiFonts, text: &str, db: &'static FontDb) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(text);
            fonts.frame_in(ui.ctx(), db, report);
        });
        out.textures_delta.clear();
        fonts.finish_with(ctx, report);
    }

    /// A character painted while no font covers it is reported once, and draws once a font that
    /// covers it is added (as the web fetches one), with no further search while the fonts stay
    /// the same. The character is a private-use one, which no craft-fonts face covers either (the
    /// database holds them when the build embeds craft-fonts).
    #[test]
    fn a_font_added_later_draws_characters_painted_before_it() {
        use vectorcraft_text::test_fonts::{PRIVATE_USE_CHAR, PRIVATE_USE_FAMILY, private_use_font};
        let db: &'static FontDb = Box::leak(Box::new(FontDb::with_font_dirs(vec![])));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut fonts = UiFonts::default();
        let text = format!("Layer {PRIVATE_USE_CHAR}");
        frame_in(&ctx, &mut fonts, &text, db);
        assert!(!has_glyph(&ctx, PRIVATE_USE_CHAR), "no font covers the character yet");
        assert_eq!(REPORTED.with(|r| r.borrow().clone()), [PRIVATE_USE_CHAR], "reported once");
        frame_in(&ctx, &mut fonts, &text, db);
        assert!(fonts.pending.is_none() && REPORTED.with(|r| r.borrow().len()) == 1, "no new search while the fonts are the same");
        db.add_font(private_use_font().expect("the test face builds"));
        assert!(db.face(PRIVATE_USE_FAMILY, "Regular").is_some());
        // The next frame looks again and adds the face; the one after draws with it.
        frame_in(&ctx, &mut fonts, &text, db);
        frame_in(&ctx, &mut fonts, &text, db);
        assert!(has_glyph(&ctx, PRIVATE_USE_CHAR), "the added face draws the character");
        assert!(fonts.uncovered.is_empty());
        assert_eq!(REPORTED.with(|r| r.borrow().len()), 1, "a covered character is not reported again");
    }
}
