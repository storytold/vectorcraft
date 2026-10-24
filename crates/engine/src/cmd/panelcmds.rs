//! Commands the panels need beyond the core set: artboard reorder, duplicate, copy and cut, and the advanced
//! Character/Paragraph attributes.

use serde_json::{Value, json};
use vectorcraft_doc::{Composer, JustificationOptions, NodeKind, Scaling, SingleWordJustify};
use vectorcraft_geom::{Rect, Vec2};

use super::*;
use crate::{Clipboard, EngineError};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("artboard.reorder", "Move Artboard Up/Down", [], None, "{index, to} change an artboard's number", has_doc, artboard_reorder),
        cmd!(
            "artboard.duplicate",
            "Duplicate Artboard",
            ["Window", "Artboards"],
            None,
            "{index?: n | indices?: [n…] (default: the artboards the Artboard tool has selected), art?: bool (default: the Artboard tool's moveArt option), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} a copy of each artboard, the copies placed together right of the last artboard as they lay, with copies of the art fully inside each when `art` (locked and hidden art only with prefs moveLockedWithArtboard), in one undo step → {index: the first copy's index, indices: the copies' indices, ids: the art's copies}",
            has_doc,
            artboard_duplicate
        ),
        cmd!(
            "artboard.copy",
            "Copy Artboard",
            [],
            None,
            "{index? | indices?, art?} (defaults as artboard.duplicate) put the artboards, and the art fully inside each when `art`, on the clipboard: edit.paste adds a copy of each, together right of the last artboard as they lay, edit.pasteInPlace/InFront/InBack where they were, in this or another document (one undo step; edit.pasteOnAllArtboards pastes only the art); edit.copy runs this while the Artboard tool is chosen → {copied: objects}",
            has_doc,
            |s, p| artboard_copy(s, p, false)
        ),
        cmd!(
            "artboard.cut",
            "Cut Artboard",
            [],
            None,
            "{index? | indices?, art?} artboard.copy, then delete the artboards and that art in one undo step (one artboard is always left: the cut fails when it names every artboard, or a locked one; art also on another locked artboard stays, and is left off the clipboard, unless prefs moveLockedWithArtboard); edit.cut runs this while the Artboard tool is chosen → {copied: objects}",
            has_doc,
            |s, p| artboard_copy(s, p, true)
        ),
        cmd!(
            "text.setFormat",
            "Character / Paragraph",
            [],
            None,
            "{ids?|id?, kerning?: 1/1000 em|\"auto\", baselineShift?: pt, hScale?: %, vScale?: %, rotation?: deg, underline?, strikethrough?, allCaps?, smallCaps?: bool, position?: \"normal\"|\"superscript\"|\"subscript\" (sizes from Document Setup), leftIndent?, rightIndent?, firstLineIndent?, spaceBefore?, spaceAfter?: pt (±1296), list?: \"none\"|\"bullet\"|\"number\", bullet?: \"none\"|\"dot\"|\"dash\"|\"circle\"|\"square\", number?: \"none\"|\"arabic\"|\"upperLetters\"|\"lowerLetters\"|\"upperRoman\"|\"lowerRoman\"|\"twoDigits\"|\"threeDigits\" (Paragraph panel › Bullets and Numbered List: each paragraph of a list starts with its marker, drawn in the font, size and color of the paragraph's first character, and its text starts at the next tab stop after the marker, the left indent counting as a stop; a bulleted list's marker is its bullet, and a numbered list's is the paragraph's number and a period: 1, 2, 3; A to Z, then AA, BB; a to z, then aa, bb; I, II, III; i, ii, iii; 01 to 99, then 100; 001 to 999, then 1000; letters and roman numerals past 3999 in Arabic digits; the numbers count the story's numbered paragraphs, its threaded frames included, from 1 after a paragraph in no list, and an empty or bulleted paragraph is not counted; list \"bullet\" keeps a list's bullet, else a dot, and list \"number\" keeps a numbered list's style, else arabic; without list, a number style makes a numbered list and a bullet makes a bulleted one; \"none\" in any of the three turns the list off; a paragraph that joins a list with left and first-line indents of 0 gets 18 and -18 unless the call sets an indent; a horizontal right-to-left list paragraph has its indents mirrored, measured from the right), hyphenate?: bool, wordsLongerThan?: letters 1..25 (5), afterFirst?, beforeLast?: letters 1..15 (3), hyphenLimit?: lines 0..25 (0: no limit), hyphenationZone?: pt 0..1296 (0; Single-line Composer only), hyphenateCapitalized?: bool (true) (Paragraph panel menu › Hyphenation…: only words longer than wordsLongerThan break, keeping afterFirst letters before the hyphen and beforeLast after it; at most hyphenLimit lines in a row end with a hyphen; a line whose text already ends within hyphenationZone of its right edge breaks before the next word instead of hyphenating it; the counts are rounded, and every value is clamped to its range), wordSpacing?: % 0..1000 (80, 100, 133), letterSpacing?: % of a word space -100..500 (0, 0, 0), glyphScaling?: % 50..200 (100, 100, 100): each [min, desired, max] or {min?, desired?, max?} (Paragraph panel menu › Justification…: Desired applies to every line, Minimum and Maximum bound what justified lines may use; values are clamped to their range, and Minimum ≤ Desired ≤ Maximum must hold afterwards or nothing changes), autoLeading?: % of the size 0..500 (120; the leading of characters whose leading is Auto), singleWordJustification?: \"fullJustify\"|\"alignLeft\"|\"alignCenter\"|\"alignRight\" (a justified line holding one word), mojikumi?: \"none\"|\"lineEndHalf\" (Japanese punctuation spacing), kinsoku?: \"none\"|\"hard\"|\"soft\" (Kinsoku Set: Soft lets 々, ー and small kana start a line), direction?: \"auto\"|\"leftToRight\"|\"rightToLeft\" (paragraph direction; auto: from each paragraph's first strong character), leadingModel?: \"romanBaseline\"|\"emBoxTop\" (leading measured baseline to baseline, or em box top to top), charAlign?: \"romanBaseline\"|\"emBoxTop\"|\"emBoxCenter\"|\"emBoxBottom\"|\"icfTop\"|\"icfBottom\" (where characters smaller than the largest on their line line up with it), proportionalMetrics?: bool (full-width glyphs on the font's proportional widths: `palt` in horizontal type, `vpal` in vertical type), tateChuYoko?: bool (Character panel menu › Tate-chu-yoko, with Show East Asian Options: in vertical type those characters stand side by side across one em of the column; kept on the characters and ignored in horizontal type), noBreak?: bool (Character panel menu › No Break: these characters stay on one line, including across a space inside the run), charDirection?: \"default\"|\"leftToRight\"|\"rightToLeft\" (Character panel menu › Character Direction: the characters are set left to right or right to left inside their paragraph, as a Unicode bidi embedding; default: as the bidi algorithm sets them), digits?: \"default\"|\"arabic\"|\"hindi\"|\"farsi\" (Character panel menu › Digits: the numeral system every digit is drawn in, whatever was typed), fontAxes?: {\"wght\": 650, …} (Character panel › Variable Font, as text.setRangeStyle: each axis named set in its units and clamped to its range, null back to the style's own setting), burasagari?: \"none\"|\"standard\"|\"forced\" (Paragraph panel menu › Burasagari None/Regular/Force: an East Asian comma or full stop ending an area type line hangs outside it: when it doesn't fit, or always; new type: \"standard\"), romanHangingPunctuation?: bool (Paragraph panel menu › Roman Hanging Punctuation: quotation marks, hyphens, full stops and commas starting or ending a line hang outside the margin, asterisks, tildes, ellipses, dashes, colons and semicolons by half; off by default), kashida?: \"none\"|\"short\"|\"medium\"|\"long\"|\"stylistic\" (Paragraph panel › Insert Kashidas: justified lines of horizontal area type lengthen each Arabic word with a kashida between its last pair of joined letters, at most a quarter, half or whole em long, or whole tatweels up to an em for stylistic, before they widen their word spaces; none by default), composer?: \"singleLine\"|\"everyLine\" (line breaking, Every-line by default), start?: byte, end?: byte} (baselineShift is in document points and hScale includes the object transform; groups include their text descendants; with a range: the character attributes style that range and the paragraph attributes apply to the paragraphs it touches; without: all the text)",
            has_doc,
            set_format
        ),
    ]
}

// ---------- artboards ----------

fn artboard_reorder(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.reorder";
    let i = artboard_index(&s.doc()?.doc, p, C)?;
    let to = p.get("to").and_then(Value::as_u64).map(index).ok_or_else(|| bad(C, "missing to"))?;
    s.edit("Reorder Artboards", |d, _| {
        if i >= d.artboards.len() {
            return Err(EngineError::Other("no such artboard".into()));
        }
        let a = d.artboards.remove(i);
        let to = to.min(d.artboards.len());
        d.artboards.insert(to, a);
        Ok(())
    })?;
    ok()
}

/// The artboards (`index` or `indices`) and whether their art comes along (`art`) for the artboard
/// copy commands: by default the artboards the Artboard tool has selected (its active one when none
/// of them is in the document) and its Move/Copy Artwork with Artboard option.
fn boards_and_art(s: &Session, p: &Value, cmd: &str) -> Result<(Vec<usize>, bool)> {
    check_switches(p, &["art"], cmd)?;
    let t = s.tool_options_of("artboard");
    let d = &s.doc()?.doc;
    let indices = if p.get("index").is_some() || p.get("indices").is_some() {
        artboard_indices(d, p, cmd)?
    } else {
        let mut picked: Vec<u64> = t["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .filter(|i| usize::try_from(*i).is_ok_and(|i| i < d.artboards.len()))
            .collect();
        if picked.is_empty() {
            picked.push(t["active"].as_u64().unwrap_or(0));
        }
        artboard_indices(d, &json!({ "indices": picked }), cmd)?
    };
    Ok((indices, bool_or(p, "art", t["moveArt"].as_bool().unwrap_or(true))))
}

/// The one artboard `cmd` acts on: `index`, an artboard of `d`, with the errors of
/// [`artboard_indices`] (`missing index`, `no such artboard`).
pub(crate) fn artboard_index(d: &vectorcraft_doc::Document, p: &Value, cmd: &str) -> Result<usize> {
    let i = p.get("index").ok_or_else(|| bad(cmd, "missing index"))?;
    artboard_indices(d, &json!({ "index": i }), cmd)?.into_iter().next().ok_or_else(|| EngineError::Other("no such artboard".into()))
}

/// The artboards `cmd` acts on: `indices` (several), else `index` (one), each an artboard of `d`,
/// in order and once each.
pub(crate) fn artboard_indices(d: &vectorcraft_doc::Document, p: &Value, cmd: &str) -> Result<Vec<usize>> {
    let listed = match (p.get("indices"), p.get("index")) {
        (Some(v), _) => point_indices(v),
        (None, Some(i)) => point_indices(&json!([i])),
        (None, None) => return Err(bad(cmd, "missing index")),
    };
    let mut indices =
        listed.filter(|v| !v.is_empty() && v.iter().all(|i| *i < d.artboards.len())).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

/// An error when one of artboards `indices` of `d` is locked: a locked artboard isn't moved,
/// resized or deleted.
pub(crate) fn refuse_locked(d: &vectorcraft_doc::Document, indices: &[usize]) -> Result<()> {
    match indices.iter().any(|i| d.artboards.get(*i).is_some_and(|a| a.locked)) {
        true => Err(EngineError::Other("the artboard is locked".into())),
        false => Ok(()),
    }
}

/// The art that goes with artboard `i` when `art`: the objects fully inside it (locked and hidden
/// ones only with prefs moveLockedWithArtboard), in paint order. With `moving` (a move or a cut),
/// the art also on another locked artboard stays unless prefs moveLockedWithArtboard
/// ([`vectorcraft_doc::Document::art_moving_with`]); a copy takes it.
pub(crate) fn artboard_art(s: &Session, i: usize, art: bool, moving: bool) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let rect = d.artboards.get(i).map(|a| a.rect).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    let all = s.prefs.move_locked_with_artboard;
    Ok(match (art, moving) {
        (false, _) => vec![],
        (true, false) => d.art_on_artboard(rect, all),
        (true, true) => d.art_moving_with(i, all),
    })
}

/// Each of `indices` with the art that goes with it when `art` ([`artboard_art`]), an object
/// counted once, with the first of them it is on (artboards can overlap).
pub(crate) fn artboards_art(s: &Session, indices: &[usize], art: bool, moving: bool) -> Result<Vec<(usize, Vec<NodeId>)>> {
    let mut seen = std::collections::HashSet::new();
    let mut arts = vec![];
    for &i in indices {
        let art = artboard_art(s, i, art, moving)?.into_iter().filter(|id| seen.insert(*id)).collect();
        arts.push((i, art));
    }
    Ok(arts)
}

/// How far a copy of `rect` moves to sit right of the last artboard (Duplicate Artboards, Paste).
pub(crate) fn beside_artboards(d: &vectorcraft_doc::Document, rect: Rect) -> Vec2 {
    let right = d.artboards.iter().map(|a| a.rect.x1).fold(f64::MIN, f64::max);
    Vec2::new(right + 20.0 - rect.x0, 0.0)
}

/// Duplicate Artboards: a copy of each artboard with its art, in one undo step. The copies keep
/// their places relative to each other, together right of the last artboard.
fn artboard_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &["patterns"], "artboard.duplicate")?;
    let (indices, art) = boards_and_art(s, p, "artboard.duplicate")?;
    let arts = artboards_art(s, &indices, art, false)?;
    let all: Vec<NodeId> = arts.iter().flat_map(|(_, a)| a.iter().copied()).collect();
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &all)?, ..Scaling::default() };
    let (index, ids) = s.edit("Duplicate Artboard", |d, _| {
        let dv = arts
            .iter()
            .filter_map(|(i, _)| d.artboards.get(*i))
            .map(|a| a.rect)
            .reduce(|a, b| a.union(b))
            .map(|r| beside_artboards(d, r))
            .unwrap_or_default();
        let (mut index, mut ids) = (vec![], vec![]);
        for (i, art) in &arts {
            let (at, copies) = copy_artboard(d, *i, dv, art, sc)?;
            index.push(at);
            ids.extend(copies);
        }
        Ok((index, ids))
    })?;
    Ok(json!({"index": index.first(), "indices": index, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

/// Artboard Copy / Cut: the artboards and their art onto the clipboard (pasting brings them all);
/// a cut then deletes them.
pub(crate) fn artboard_copy(s: &mut Session, p: &Value, cut: bool) -> Result<Value> {
    let (indices, art) = boards_and_art(s, p, if cut { "artboard.cut" } else { "artboard.copy" })?;
    let arts = artboards_art(s, &indices, art, cut)?;
    let st = s.doc()?;
    if cut && st.doc.artboards.len() <= indices.len() {
        return Err(EngineError::Other("a document needs at least one artboard".into()));
    }
    if cut {
        refuse_locked(&st.doc, &indices)?;
    }
    let art = st.doc.paint_order(arts.into_iter().flat_map(|(_, a)| a));
    let boards: Vec<_> = indices.iter().filter_map(|i| st.doc.artboards.get(*i).cloned()).collect();
    let clip = Clipboard { source_artboard: boards.first().map(|a| a.rect), artboards: boards, ..Clipboard::copy(st, &art) };
    let copied = clip.nodes.len();
    if cut {
        s.edit("Cut Artboard", |d, sel| {
            if indices.iter().any(|i| *i >= d.artboards.len()) {
                return Err(EngineError::Other("no such artboard".into()));
            }
            // The last first, so the others keep their places; their guides go with them.
            for &i in indices.iter().rev() {
                let gone = d.artboards.remove(i).id;
                d.retire_artboard_id(gone);
                d.retain_guides(sel, |_, g| g.artboard != Some(gone));
            }
            d.remove_many(&art)?;
            sel.prune(d);
            Ok(())
        })?;
        if s.tool_id() == "artboard" {
            s.set_tool_option("active", &json!(indices.first().map_or(0, |i| i.saturating_sub(1))));
        }
    }
    s.clipboard = clip;
    Ok(json!({"copied": copied}))
}

/// Add a copy of artboard `i` moved by `dv`, with copies of its guides and of its `art` (each just
/// above its original) moved along → (the copy's index, the art's copies). When `sc.patterns`, the
/// pattern tiles of the art's copies move with them. Duplicate Artboards and Alt-drag.
pub(crate) fn copy_artboard(d: &mut vectorcraft_doc::Document, i: usize, dv: Vec2, art: &[NodeId], sc: Scaling) -> Result<(usize, Vec<NodeId>)> {
    let src = d.artboards.get(i).cloned().ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    let index = push_artboard_copy(d, &src, src.rect + dv);
    // Its guides come along.
    if let Some(id) = d.artboards.get(index).map(|a| a.id) {
        d.copy_artboard_guides(src.id, id, dv);
    }
    let copies = super::edit::copy_above(d, art, |_, _, c| c.transform(Affine::translate(dv), sc))?;
    Ok((index, copies))
}

/// Add a copy of artboard `src` at `rect`, unlocked, with an id of its own and its name, or
/// `<name> copy` (`<name> copy 2`…) when that is taken → its index.
pub(crate) fn push_artboard_copy(d: &mut vectorcraft_doc::Document, src: &vectorcraft_doc::Artboard, rect: Rect) -> usize {
    let mut a = src.clone();
    a.id = d.next_artboard_id();
    a.locked = false;
    let taken = |name: &str| d.artboards.iter().any(|a| a.name == name);
    a.name = [src.name.clone(), format!("{} copy", src.name)]
        .into_iter()
        .chain((2u64..).map(|i| format!("{} copy {i}", src.name)))
        .find(|name| !taken(name))
        .unwrap_or_default();
    a.rect = rect;
    d.artboards.push(a);
    d.artboards.len() - 1
}

// ---------- text ----------

/// Largest indent or paragraph spacing (points; Illustrator's limit).
const MAX_PARA: f64 = 1296.0;

/// `text.setFormat`'s `bullet` values and the bullets they pick.
const BULLETS: [(&str, vectorcraft_doc::text::Bullet); 4] = [
    ("dot", vectorcraft_doc::text::Bullet::Dot),
    ("dash", vectorcraft_doc::text::Bullet::Dash),
    ("circle", vectorcraft_doc::text::Bullet::Circle),
    ("square", vectorcraft_doc::text::Bullet::Square),
];

/// `text.setFormat`'s `number` values and the number styles they pick.
const NUMBERS: [(&str, vectorcraft_doc::text::NumberStyle); 7] = [
    ("arabic", vectorcraft_doc::text::NumberStyle::Arabic),
    ("upperLetters", vectorcraft_doc::text::NumberStyle::UpperLetters),
    ("lowerLetters", vectorcraft_doc::text::NumberStyle::LowerLetters),
    ("upperRoman", vectorcraft_doc::text::NumberStyle::UpperRoman),
    ("lowerRoman", vectorcraft_doc::text::NumberStyle::LowerRoman),
    ("twoDigits", vectorcraft_doc::text::NumberStyle::TwoDigits),
    ("threeDigits", vectorcraft_doc::text::NumberStyle::ThreeDigits),
];

/// A justification spacing of `text.setFormat` (`wordSpacing`, `letterSpacing`, `glyphScaling`):
/// `[min, desired, max]` or `{min?, desired?, max?}`, in percent. The values not given are kept.
fn spacing_param(p: &Value, key: &str, c: &str) -> Result<Option<[Option<f64>; 3]>> {
    let number = |v: &Value| v.as_f64().filter(|v| v.is_finite());
    let err = || bad(c, format!("`{key}` must be [min, desired, max] or {{min?, desired?, max?}}, in percent"));
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() == 3 => {
            let mut out = [None; 3];
            for (o, v) in out.iter_mut().zip(a) {
                *o = Some(number(v).ok_or_else(err)?);
            }
            Ok(Some(out))
        }
        Some(Value::Object(o)) => {
            let mut out = [None; 3];
            for (k, v) in o {
                let i = ["min", "desired", "max"].iter().position(|n| n == k).ok_or_else(err)?;
                if let Some(slot) = out.get_mut(i) {
                    *slot = Some(number(v).ok_or_else(err)?);
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(err()),
    }
}

fn set_format(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.setFormat";
    let ids = super::typecmd::text_targets(s, p, C)?;
    let num = |k: &str| p.get(k).and_then(Value::as_f64);
    let flag = |k: &str| p.get(k).and_then(Value::as_bool);
    let kerning = match p.get("kerning") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) if a.eq_ignore_ascii_case("auto") => Some(None),
        Some(v) => Some(Some(v.as_f64().ok_or_else(|| bad(C, "kerning must be a number or \"auto\""))?.clamp(-1000.0, 10000.0))),
    };
    let keys = [
        "kerning",
        "baselineShift",
        "hScale",
        "vScale",
        "rotation",
        "underline",
        "strikethrough",
        "allCaps",
        "smallCaps",
        "position",
        "leftIndent",
        "rightIndent",
        "firstLineIndent",
        "spaceBefore",
        "spaceAfter",
        "list",
        "bullet",
        "number",
        "hyphenate",
        "wordsLongerThan",
        "afterFirst",
        "beforeLast",
        "hyphenLimit",
        "hyphenationZone",
        "hyphenateCapitalized",
        "wordSpacing",
        "letterSpacing",
        "glyphScaling",
        "autoLeading",
        "singleWordJustification",
        "mojikumi",
        "kinsoku",
        "direction",
        "leadingModel",
        "charAlign",
        "charDirection",
        "proportionalMetrics",
        "tateChuYoko",
        "noBreak",
        "digits",
        "fontAxes",
        "burasagari",
        "romanHangingPunctuation",
        "kashida",
        "composer",
    ];
    if !keys.iter().any(|k| p.get(*k).is_some()) {
        return Err(bad(C, "nothing to change"));
    }
    let composer = match p.get("composer") {
        None | Some(Value::Null) => None,
        Some(v) => Some(match v.as_str() {
            Some("singleLine") => Composer::SingleLine,
            Some("everyLine") => Composer::EveryLine,
            _ => return Err(bad(C, "composer must be \"singleLine\" or \"everyLine\"")),
        }),
    };
    let (position, small_caps) = super::docsetup::script_params(p, &s.doc()?.doc.setup, C)?;
    let char_align = super::textedit::char_align_param(p, C)?;
    let char_direction = super::textedit::char_direction_param(p, C)?;
    let digits = super::textedit::digits_param(p, C)?;
    let font_axes = super::textedit::font_axes_param(p, C)?;
    let mojikumi = match p.get("mojikumi") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("none") => vectorcraft_doc::Mojikumi::None,
            Some("lineEndHalf") => vectorcraft_doc::Mojikumi::LineEndHalf,
            _ => return Err(bad(C, "`mojikumi` must be \"none\" or \"lineEndHalf\"")),
        }),
    };
    let kinsoku = match p.get("kinsoku") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("none") => vectorcraft_doc::Kinsoku::None,
            Some("hard") => vectorcraft_doc::Kinsoku::Hard,
            Some("soft") => vectorcraft_doc::Kinsoku::Soft,
            _ => return Err(bad(C, "`kinsoku` must be \"none\", \"hard\" or \"soft\"")),
        }),
    };
    let direction = match p.get("direction") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("auto") => None,
            Some("leftToRight") => Some(vectorcraft_doc::ParaDirection::LeftToRight),
            Some("rightToLeft") => Some(vectorcraft_doc::ParaDirection::RightToLeft),
            _ => return Err(bad(C, "`direction` must be \"auto\", \"leftToRight\" or \"rightToLeft\"")),
        }),
    };
    let leading_model = match p.get("leadingModel") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("romanBaseline") => vectorcraft_doc::LeadingModel::RomanBaseline,
            Some("emBoxTop") => vectorcraft_doc::LeadingModel::EmBoxTop,
            _ => return Err(bad(C, "`leadingModel` must be \"romanBaseline\" or \"emBoxTop\"")),
        }),
    };
    let burasagari = match p.get("burasagari") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("none") => vectorcraft_doc::Burasagari::None,
            Some("standard") => vectorcraft_doc::Burasagari::Standard,
            Some("forced") => vectorcraft_doc::Burasagari::Forced,
            _ => return Err(bad(C, "`burasagari` must be \"none\", \"standard\" or \"forced\"")),
        }),
    };
    let kashida = match p.get("kashida") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("none") => vectorcraft_doc::text::Kashida::None,
            Some("short") => vectorcraft_doc::text::Kashida::Short,
            Some("medium") => vectorcraft_doc::text::Kashida::Medium,
            Some("long") => vectorcraft_doc::text::Kashida::Long,
            Some("stylistic") => vectorcraft_doc::text::Kashida::Stylistic,
            _ => return Err(bad(C, "`kashida` must be \"none\", \"short\", \"medium\", \"long\" or \"stylistic\"")),
        }),
    };
    // Bullets and numbers: `list` turns a bulleted or numbered list on or off, `bullet` picks the
    // bullet and `number` the number style; "none" in any of them turns the list off.
    let list = super::object::choice(p, C, "list", &["none", "bullet", "number"])?;
    let bullet = super::object::choice(p, C, "bullet", &["none", "dot", "dash", "circle", "square"])?
        .map(|b| BULLETS.iter().find(|(k, _)| *k == b).map(|&(_, bullet)| bullet));
    let number = super::object::choice(
        p,
        C,
        "number",
        &["none", "arabic", "upperLetters", "lowerLetters", "upperRoman", "lowerRoman", "twoDigits", "threeDigits"],
    )?
    .map(|n| NUMBERS.iter().find(|(k, _)| *k == n).map(|&(_, style)| style));
    let indents_given = num("leftIndent").is_some() || num("firstLineIndent").is_some();
    let spacing = |k: &str| spacing_param(p, k, C);
    let (word_spacing, letter_spacing, glyph_scaling) = (spacing("wordSpacing")?, spacing("letterSpacing")?, spacing("glyphScaling")?);
    let single_word = match p.get("singleWordJustification") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("fullJustify") => SingleWordJustify::FullJustify,
            Some("alignLeft") => SingleWordJustify::AlignLeft,
            Some("alignCenter") => SingleWordJustify::AlignCenter,
            Some("alignRight") => SingleWordJustify::AlignRight,
            _ => return Err(bad(C, "`singleWordJustification` must be \"fullJustify\", \"alignLeft\", \"alignCenter\" or \"alignRight\"")),
        }),
    };
    let auto_leading = match p.get("autoLeading") {
        None => None,
        Some(v) => Some(v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(C, "`autoLeading` must be a number"))?),
    };
    // A paragraph whose Minimum, Desired and Maximum would no longer be in order.
    let mut out_of_order: Option<&str> = None;
    let range = super::typecmd::TextRange::parse(p, C)?;
    // The character attributes, given the baseline shift and horizontal scale in the type's own units.
    let chars = |st: &mut vectorcraft_doc::CharStyle, baseline_shift: Option<f64>, h_scale: Option<f64>| {
        if let Some(k) = kerning {
            st.kerning = k;
        }
        if let Some(v) = baseline_shift {
            st.baseline_shift = v;
        }
        if let Some(v) = h_scale {
            st.h_scale = v;
        }
        if let Some(v) = num("vScale") {
            st.v_scale = v.clamp(1.0, 10000.0);
        }
        if let Some(v) = num("rotation") {
            st.rotation = ((v + 180.0).rem_euclid(360.0)) - 180.0;
        }
        if let Some(v) = flag("underline") {
            st.underline = v;
        }
        if let Some(v) = flag("strikethrough") {
            st.strikethrough = v;
        }
        if let Some(v) = flag("allCaps") {
            st.all_caps = v;
        }
        if let Some(v) = position {
            st.position = v;
        }
        if let Some(v) = small_caps {
            st.small_caps = v;
        }
        if let Some(v) = char_align {
            st.char_align = v;
        }
        if let Some(v) = char_direction {
            st.char_direction = v;
        }
        if let Some(v) = digits {
            st.digits = v;
        }
        if let Some(v) = flag("proportionalMetrics") {
            st.proportional_metrics = v;
        }
        if let Some(v) = flag("tateChuYoko") {
            st.tate_chu_yoko = v;
        }
        if let Some(v) = flag("noBreak") {
            st.no_break = v;
        }
        if let Some(v) = &font_axes {
            super::textedit::change_font_axes(st, v);
        }
    };
    s.edit("Character", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let baseline_shift = num("baselineShift").map(|v| super::typecmd::local_type_value(t, v, (-1296.0, 1296.0), false, C)).transpose()?;
            let h_scale = num("hScale").map(|v| super::typecmd::local_type_value(t, v, (1.0, 10000.0), true, C)).transpose()?;
            super::typecmd::style_chars(t, range, |st| chars(st, baseline_shift, h_scale));
            let span = super::typecmd::para_span(range, t);
            t.edit_paras(span, |para| {
                if let Some(v) = num("leftIndent") {
                    para.left_indent = v.clamp(-MAX_PARA, MAX_PARA);
                }
                if let Some(v) = num("rightIndent") {
                    para.right_indent = v.clamp(-MAX_PARA, MAX_PARA);
                }
                if let Some(v) = num("firstLineIndent") {
                    para.first_line_indent = v.clamp(-MAX_PARA, MAX_PARA);
                }
                if let Some(v) = num("spaceBefore") {
                    para.space_before = v.clamp(-MAX_PARA, MAX_PARA);
                }
                if let Some(v) = num("spaceAfter") {
                    para.space_after = v.clamp(-MAX_PARA, MAX_PARA);
                }
                if list.is_some() || bullet.is_some() || number.is_some() {
                    let on = list != Some("none") && bullet != Some(None) && number != Some(None);
                    let had = para.list;
                    // A numbered list for list "number", or without `list` for a number style.
                    let numbered = list == Some("number") || (list.is_none() && number.is_some());
                    para.list = on.then(|| vectorcraft_doc::text::ParaList {
                        bullet: bullet.flatten().or(had.map(|l| l.bullet)).unwrap_or_default(),
                        number: numbered.then(|| number.flatten().or(had.and_then(|l| l.number)).unwrap_or_default()),
                    });
                    // A paragraph that joins a list with no left or first-line indent gets a
                    // hanging indent, unless the call sets an indent.
                    if on && had.is_none() && !indents_given && para.left_indent == 0.0 && para.first_line_indent == 0.0 {
                        para.left_indent = vectorcraft_doc::text::LIST_HANGING_INDENT;
                        para.first_line_indent = -vectorcraft_doc::text::LIST_HANGING_INDENT;
                    }
                }
                if let Some(v) = flag("hyphenate") {
                    para.hyphenate = v;
                }
                let letters = |k: &str, lo: f64, hi: f64| num(k).map(|v| v.round().clamp(lo, hi) as u32);
                if let Some(v) = letters("wordsLongerThan", 1.0, 25.0) {
                    para.hyphenation.words_longer_than = v;
                }
                if let Some(v) = letters("afterFirst", 1.0, 15.0) {
                    para.hyphenation.after_first = v;
                }
                if let Some(v) = letters("beforeLast", 1.0, 15.0) {
                    para.hyphenation.before_last = v;
                }
                if let Some(v) = letters("hyphenLimit", 0.0, 25.0) {
                    para.hyphenation.hyphen_limit = v;
                }
                if let Some(v) = num("hyphenationZone") {
                    para.hyphenation.zone = v.clamp(0.0, MAX_PARA);
                }
                if let Some(v) = flag("hyphenateCapitalized") {
                    para.hyphenation.capitalized = v;
                }
                let j = &mut para.justification;
                for (name, given, value, limits) in [
                    ("wordSpacing", &word_spacing, &mut j.word_spacing, JustificationOptions::WORD_SPACING),
                    ("letterSpacing", &letter_spacing, &mut j.letter_spacing, JustificationOptions::LETTER_SPACING),
                    ("glyphScaling", &glyph_scaling, &mut j.glyph_scaling, JustificationOptions::GLYPH_SCALING),
                ] {
                    let Some(given) = given else { continue };
                    for (v, g) in value.iter_mut().zip(given) {
                        if let Some(g) = g {
                            *v = g.clamp(limits.0, limits.1);
                        }
                    }
                    if !(value[0] <= value[1] && value[1] <= value[2]) {
                        out_of_order.get_or_insert(name);
                    }
                }
                if let Some(v) = auto_leading {
                    j.auto_leading = v.clamp(JustificationOptions::AUTO_LEADING.0, JustificationOptions::AUTO_LEADING.1);
                }
                if let Some(v) = single_word {
                    j.single_word = v;
                }
                if let Some(v) = mojikumi {
                    para.mojikumi = v;
                }
                if let Some(v) = kinsoku {
                    para.kinsoku = v;
                }
                if let Some(v) = direction {
                    para.direction = v;
                }
                if let Some(v) = leading_model {
                    para.leading_model = v;
                }
                if let Some(v) = burasagari {
                    para.burasagari = v;
                }
                if let Some(v) = flag("romanHangingPunctuation") {
                    para.roman_hanging = v;
                }
                if let Some(v) = kashida {
                    para.kashida = v;
                }
                if let Some(v) = composer {
                    para.composer = v;
                }
            });
            if let Some(name) = out_of_order {
                return Err(bad(C, format!("`{name}`: Minimum must not exceed Desired, nor Desired Maximum")));
            }
            super::typecmd::refresh_bounds(t);
        }
        // Graph labels keep a change made to all of their text through the graph being drawn again.
        if range.is_none() {
            let shift = num("baselineShift").map(|v| v.clamp(-1296.0, 1296.0));
            let scale = num("hScale").map(|v| v.clamp(1.0, 10000.0));
            super::graph::capture_label_styles(d, &ids, &|st| chars(st, shift, scale));
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}
