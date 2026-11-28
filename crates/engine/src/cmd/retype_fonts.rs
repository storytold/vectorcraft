//! Retype's matching fonts (`type.retypeFonts`): the fonts whose glyphs match the selected outlined
//! text, ranked, for the Retype panel's list
//! (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/identify-fonts-using-retype.html>,
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/apply-retype-suggested-fonts-to-live-text.html>).
//! The panel lists ten fonts and five more on request. Each selected glyph shape is compared with
//! the glyphs of up to 96 loaded fonts, those of the document's live type first, and of the
//! installed fonts not loaded yet (`retype_installed.rs`), by the same matching as `type.retype`
//! (exact matches only). Of two versions of one font, only the version its
//! family and style name gives is compared. The order is VectorCraft's own: a font ranks by how many
//! of the shapes it matches, then by how closely, on average, and then by name. The query does not
//! change the document; the installed fonts that match are loaded.

use std::collections::BTreeSet;
use std::sync::Arc;

use kurbo::BezPath;
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_text::retype::{RetypeIndex, shape_key};
use vectorcraft_text::{FontDb, FontFace};

use super::super::{CommandSpec, Result, Session, bad, cmd, has_selected_path};
use super::installed::Installed;
use super::{candidate_faces, selected_pieces};

pub(in crate::cmd) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "type.retypeFonts",
        "Find Matching Fonts",
        [],
        None,
        "{count?: 1..96 (default 10), installed?: bool} the fonts whose glyphs match the selected outlined text, as the Retype panel lists them: each selected shape (paths and compound paths, inside groups too) is compared with the glyphs of up to 96 loaded fonts, those of the document's live type first, by type.retype's matching (exact matches only), a font whose glyphs match the shapes only as combining marks not listed, and, unless installed is false (true, omitted or null: compared), with the installed fonts not loaded yet, within type.retype's limits on reading them (every file within them is read, and the fonts that match are loaded while the list holds fewer than 96 fonts, loaded ones included); of two versions of a font, only the version its family and style name gives is compared, the one choosing the font sets; the fonts rank by how many shapes each matches, then by the mean distance left after fitting, then by family and style name; the document does not change → {fonts: [{family, style, matched (shapes it matches)}] (the first count), shapes (shapes compared), more (more fonts match than count), truncated (shapes past the limits were left out), installed: {read, skipped} (as type.retype reports them)}; a count that is not a whole number from 1 to 96, or an installed that is not true, false or null, is an error",
        has_selected_path,
        retype_fonts
    )]
}

/// The fonts listed when `count` is left out.
pub(super) const DEFAULT_COUNT: usize = 10;
/// The most fonts one query lists: every face the matching indexes.
pub(super) const MAX_COUNT: usize = vectorcraft_text::retype::MAX_FACES;

/// One font that matched some of the shapes.
pub(super) struct Ranked {
    face: Arc<vectorcraft_text::FontFace>,
    /// How many shapes it matched.
    matched: usize,
    /// The sum of those matches' leftover distances.
    residual: f64,
    /// Whether some shape matched a glyph of it that is not a combining mark.
    character: bool,
}

impl Ranked {
    /// The font.
    pub(super) fn face(&self) -> &Arc<FontFace> {
        &self.face
    }

    fn mean(&self) -> f64 {
        self.residual / self.matched.max(1) as f64
    }
}

/// The `count` param: how many fonts to list.
pub(super) fn count_param(p: &Value) -> Result<usize> {
    match p.get("count") {
        None | Some(Value::Null) => Ok(DEFAULT_COUNT),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_COUNT).contains(n))
            .ok_or_else(|| bad("type.retypeFonts", "`count` must be a whole number from 1 to 96")),
    }
}

/// Is `f` the version of its font that its family and style name gives? A font is chosen by its
/// family and style, which set that version: of two versions of one font, only that one is
/// compared, and the other versions do not take the places of fonts loaded after them.
fn by_name(db: &FontDb, f: &FontFace) -> bool {
    db.face(&f.family, &f.style).is_some_and(|g| g.id() == f.id())
}

/// The loaded faces the list compares: those of the document's live type first, then the other
/// loaded faces, each the version its name gives, at most [`MAX_COUNT`].
pub(super) fn listed_faces(db: &FontDb, d: &Document) -> Vec<Arc<FontFace>> {
    let mut faces: Vec<_> = candidate_faces(db, d, &[]).into_iter().filter(|f| by_name(db, f)).collect();
    for f in db.loaded_faces() {
        if faces.len() >= MAX_COUNT {
            break;
        }
        if by_name(db, &f) && !faces.iter().any(|k| k.id() == f.id()) {
            faces.push(f);
        }
    }
    faces
}

/// The fonts of `faces` whose glyphs match `shapes`, unranked, and with `search_installed` those of
/// the installed fonts not loaded yet, for the places left of [`MAX_COUNT`] (loading the ones that
/// match), with what that search read. With `handed` (what a search made off the UI thread found,
/// [`super::installed::RetypeFound`]) its faces are ranked instead, and the installed fonts are not
/// read again.
pub(super) fn rank_fonts(
    db: &FontDb,
    faces: Vec<Arc<FontFace>>,
    shapes: &[&BezPath],
    search_installed: bool,
    handed: Option<Installed>,
) -> Result<(Vec<Ranked>, Installed)> {
    // Only glyphs with a selected shape's key (segments per contour) can match one.
    let wanted: BTreeSet<u64> = shapes.iter().filter_map(|s| shape_key(s)).collect();
    // No shape has a key: no font is read.
    let index = if wanted.is_empty() { RetypeIndex::default() } else { RetypeIndex::build(db, faces, &wanted) };
    let mut ranked: Vec<Ranked> = vec![];
    rank(&index, shapes, &mut ranked);
    // The installed fonts not loaded yet: those whose glyphs match a shape, of the version their
    // family and style name gives, up to 96 fonts listed in all. A font that matches only as
    // combining marks is not listed, so it takes no place.
    let room = MAX_COUNT.saturating_sub(ranked.iter().filter(|r| r.character).count());
    let found = if let Some(handed) = handed {
        handed
    } else if !search_installed {
        Installed::default()
    } else if room == 0 {
        // The list is full: the installed faces a search would compare count as skipped, none
        // when no shape would start one.
        let skipped = if super::installed::starts_search(shapes) { db.unloaded_installed_faces().len() } else { 0 };
        Installed { skipped, ..Default::default() }
    } else {
        super::installed::search_listed(db, shapes, room, &|f| by_name(db, f))?
    };
    // A face found earlier (handed in) may be loaded and ranked already: it counts once.
    let more: Vec<_> = found.faces.iter().filter(|f| !ranked.iter().any(|r| r.face.id() == f.id())).cloned().collect();
    if !more.is_empty() {
        rank(&RetypeIndex::build(db, more, &wanted), shapes, &mut ranked);
    }
    Ok((ranked, found))
}

/// Adds to `ranked` the fonts of `index` that match `shapes`.
fn rank(index: &RetypeIndex, shapes: &[&BezPath], ranked: &mut Vec<Ranked>) {
    for shape in shapes {
        // A shape can fit several glyphs or poses of one font: it counts once for the font, at its
        // closest fit.
        let mut best: Vec<(Arc<FontFace>, f64, bool)> = vec![];
        for m in index.all_matches(shape) {
            let character = !vectorcraft_text::retype::is_combining_mark(m.ch);
            match best.iter_mut().find(|(f, _, _)| f.id() == m.face.id()) {
                Some(b) => {
                    b.1 = b.1.min(m.residual);
                    b.2 |= character;
                }
                None => best.push((m.face, m.residual, character)),
            }
        }
        for (face, residual, character) in best {
            match ranked.iter_mut().find(|r| r.face.id() == face.id()) {
                Some(r) => {
                    r.matched += 1;
                    r.residual += residual;
                    r.character |= character;
                }
                None => ranked.push(Ranked { face, matched: 1, residual, character }),
            }
        }
    }
}

pub(super) fn retype_fonts(s: &mut Session, p: &Value) -> Result<Value> {
    retype_fonts_in(s, p, FontDb::global())
}

/// [`retype_fonts`] with the fonts of `db`.
fn retype_fonts_in(s: &mut Session, p: &Value, db: &FontDb) -> Result<Value> {
    const C: &str = "type.retypeFonts";
    let count = count_param(p)?;
    let search_installed = super::installed::wanted(C, p)?;
    let handed = super::installed::handed(s);
    let (pieces, truncated) = selected_pieces(s)?;
    let d = &s.doc()?.doc;
    // A shape's paint does not change which glyph it is: shapes that would stay outlines count too.
    let shapes: Vec<&BezPath> = pieces.iter().map(|pc| &pc.path).collect();
    let (mut ranked, found) = rank_fonts(db, listed_faces(db, d), &shapes, search_installed, handed)?;
    // A font whose glyphs match the shapes only as combining marks would retype none of them, as
    // type.retype starts no line with a mark: it is not listed.
    ranked.retain(|r| r.character);
    ranked.sort_by(|a, b| {
        b.matched
            .cmp(&a.matched)
            .then(a.mean().total_cmp(&b.mean()))
            .then_with(|| a.face.family.to_lowercase().cmp(&b.face.family.to_lowercase()))
            .then_with(|| a.face.style.to_lowercase().cmp(&b.face.style.to_lowercase()))
            .then(a.face.id().cmp(&b.face.id()))
    });
    let more = ranked.len() > count;
    ranked.truncate(count);
    Ok(json!({
        "fonts": ranked.iter().map(|r| json!({"family": r.face.family, "style": r.face.style, "matched": r.matched})).collect::<Vec<_>>(),
        "shapes": pieces.len(),
        "more": more,
        "truncated": truncated,
        "installed": found.json(),
    }))
}

#[cfg(test)]
#[path = "retype_fonts_tests.rs"]
mod tests;
