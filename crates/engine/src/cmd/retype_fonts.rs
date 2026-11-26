//! Retype's matching fonts (`type.retypeFonts`): the fonts whose glyphs match the selected outlined
//! text, ranked, for the Retype panel's list
//! (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/identify-fonts-using-retype.html>,
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/apply-retype-suggested-fonts-to-live-text.html>).
//! The panel lists ten fonts and five more on request. Each selected glyph shape is compared with
//! the glyphs of up to 96 loaded fonts, those of the document's live type first, by the same
//! matching as `type.retype` (exact matches only). Of two versions of one font, only the version its
//! family and style name gives is compared. The order is VectorCraft's own: a font ranks by how many
//! of the shapes it matches, then by how closely, on average, and then by name. The query changes
//! nothing.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_text::FontDb;
use vectorcraft_text::retype::{RetypeIndex, shape_key};

use super::super::edit::selected_roots;
use super::super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_selected_path};
use super::{candidate_faces, collect};

pub(in crate::cmd) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "type.retypeFonts",
        "Find Matching Fonts",
        [],
        None,
        "{count?: 1..96 (default 10)} the fonts whose glyphs match the selected outlined text, as the Retype panel lists them: each selected shape (paths and compound paths, inside groups too) is compared with the glyphs of up to 96 loaded fonts, those of the document's live type first, by type.retype's matching (exact matches only), a font whose glyphs match the shapes only as combining marks not listed; of two versions of a font loaded at once, only the version its family and style name gives is compared, the one choosing the font sets; the fonts rank by how many shapes each matches, then by the mean distance left after fitting, then by family and style name; the document does not change → {fonts: [{family, style, matched (shapes it matches)}] (the first count), shapes (shapes compared), more (more fonts match than count), truncated (shapes past the limits were left out)}; a count that is not a whole number from 1 to 96 is an error",
        has_selected_path,
        retype_fonts
    )]
}

/// The fonts listed when `count` is left out.
pub(super) const DEFAULT_COUNT: usize = 10;
/// The most fonts one query lists: every face the matching indexes.
pub(super) const MAX_COUNT: usize = vectorcraft_text::retype::MAX_FACES;

/// One font that matched some of the shapes.
struct Ranked {
    face: Arc<vectorcraft_text::FontFace>,
    /// How many shapes it matched.
    matched: usize,
    /// The sum of those matches' leftover distances.
    residual: f64,
    /// Whether some shape matched a glyph of it that is not a combining mark.
    character: bool,
}

impl Ranked {
    fn mean(&self) -> f64 {
        self.residual / self.matched.max(1) as f64
    }
}

pub(super) fn retype_fonts(s: &mut Session, p: &Value) -> Result<Value> {
    retype_fonts_in(s, p, FontDb::global())
}

/// [`retype_fonts`] with the fonts of `db`.
fn retype_fonts_in(s: &mut Session, p: &Value, db: &FontDb) -> Result<Value> {
    const C: &str = "type.retypeFonts";
    let count = match p.get("count") {
        None | Some(Value::Null) => DEFAULT_COUNT,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_COUNT).contains(n))
            .ok_or_else(|| bad(C, "`count` must be a whole number from 1 to 96"))?,
    };
    let roots = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let mut pieces = vec![];
    let (mut barrier, mut truncated) = (true, false);
    for r in &roots {
        if let Some(n) = d.node(*r) {
            collect(n, d.parent_of(*r), 0, &mut barrier, &mut truncated, &mut pieces);
        }
    }
    if pieces.is_empty() {
        return Err(EngineError::Other("Retype: select outlined text (paths or compound paths)".into()));
    }
    // Only glyphs with a selected shape's key (segments per contour) can match one.
    let wanted: BTreeSet<u64> = pieces.iter().filter_map(|pc| shape_key(&pc.path)).collect();
    // A font is chosen by its family and style, which set the version the name gives: of two
    // versions of one font, only that one is compared. The other versions do not take the places
    // of fonts loaded after them.
    let by_name = |f: &Arc<vectorcraft_text::FontFace>| db.face(&f.family, &f.style).is_some_and(|g| g.id() == f.id());
    let mut faces: Vec<_> = candidate_faces(db, d, &[]).into_iter().filter(by_name).collect();
    for f in db.loaded_faces() {
        if faces.len() >= vectorcraft_text::retype::MAX_FACES {
            break;
        }
        if by_name(&f) && !faces.iter().any(|k| k.id() == f.id()) {
            faces.push(f);
        }
    }
    // No shape has a key: no font is read.
    let index = if wanted.is_empty() { RetypeIndex::default() } else { RetypeIndex::build(db, faces, &wanted) };
    // A shape's paint does not change which glyph it is: shapes that would stay outlines count too.
    let mut ranked: Vec<Ranked> = vec![];
    for pc in &pieces {
        // A shape can fit several glyphs or poses of one font: it counts once for the font, at its
        // closest fit.
        let mut best: Vec<(Arc<vectorcraft_text::FontFace>, f64, bool)> = vec![];
        for m in index.all_matches(&pc.path) {
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
    }))
}

#[cfg(test)]
#[path = "retype_fonts_tests.rs"]
mod tests;
