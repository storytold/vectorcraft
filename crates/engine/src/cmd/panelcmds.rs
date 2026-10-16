//! Commands the panels need beyond the core set: artboard reorder, duplicate, copy and cut, and the advanced
//! Character/Paragraph attributes.

use serde_json::{Value, json};
use vectorcraft_doc::{Composer, NodeKind, Scaling};
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
            "{index?: n (default: the Artboard tool's artboard), art?: bool (default: the Artboard tool's moveArt option), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} copy placed right of the last artboard, with copies of the art fully inside it when `art` (locked and hidden art only with prefs moveLockedWithArtboard) → {index, ids: the art's copies}",
            has_doc,
            artboard_duplicate
        ),
        cmd!(
            "artboard.copy",
            "Copy Artboard",
            [],
            None,
            "{index?, art?} (defaults as artboard.duplicate) put the artboard, and the art fully inside it when `art`, on the clipboard: edit.paste adds a copy right of the last artboard, edit.pasteInPlace/InFront/InBack where it was, in this or another document (one undo step; edit.pasteOnAllArtboards pastes only the art); edit.copy runs this while the Artboard tool is chosen → {copied: objects}",
            has_doc,
            |s, p| artboard_copy(s, p, false)
        ),
        cmd!(
            "artboard.cut",
            "Cut Artboard",
            [],
            None,
            "{index?, art?} artboard.copy, then delete the artboard and that art in one undo step (not the only artboard); edit.cut runs this while the Artboard tool is chosen → {copied: objects}",
            has_doc,
            |s, p| artboard_copy(s, p, true)
        ),
        cmd!(
            "text.setFormat",
            "Character / Paragraph",
            [],
            None,
            "{ids?|id?, kerning?: 1/1000 em|\"auto\", baselineShift?: pt, hScale?: %, vScale?: %, rotation?: deg, underline?, strikethrough?, allCaps?, smallCaps?: bool, position?: \"normal\"|\"superscript\"|\"subscript\" (sizes from Document Setup), leftIndent?, rightIndent?, firstLineIndent?, spaceBefore?, spaceAfter?: pt (±1296), hyphenate?: bool, wordsLongerThan?: letters 1..25 (5), afterFirst?, beforeLast?: letters 1..15 (3), hyphenLimit?: lines 0..25 (0: no limit), hyphenationZone?: pt 0..1296 (0; Single-line Composer only), hyphenateCapitalized?: bool (true) (Paragraph panel menu › Hyphenation…: only words longer than wordsLongerThan break, keeping afterFirst letters before the hyphen and beforeLast after it; at most hyphenLimit lines in a row end with a hyphen; a line whose text already ends within hyphenationZone of its right edge breaks before the next word instead of hyphenating it; the counts are rounded, and every value is clamped to its range), mojikumi?: \"none\"|\"lineEndHalf\" (Japanese punctuation spacing), kinsoku?: \"none\"|\"hard\"|\"soft\" (Kinsoku Set: Soft lets 々, ー and small kana start a line), direction?: \"auto\"|\"leftToRight\"|\"rightToLeft\" (paragraph direction; auto: from each paragraph's first strong character), leadingModel?: \"romanBaseline\"|\"emBoxTop\" (leading measured baseline to baseline, or em box top to top), charAlign?: \"romanBaseline\"|\"emBoxTop\"|\"emBoxCenter\"|\"emBoxBottom\"|\"icfTop\"|\"icfBottom\" (where characters smaller than the largest on their line line up with it), proportionalMetrics?: bool (full-width glyphs on the font's proportional widths: `palt` in horizontal type, `vpal` in vertical type), digits?: \"default\"|\"arabic\"|\"hindi\"|\"farsi\" (Character panel menu › Digits: the numeral system every digit is drawn in, whatever was typed), burasagari?: \"none\"|\"standard\"|\"forced\" (Paragraph panel menu › Burasagari None/Regular/Force: an East Asian comma or full stop ending an area type line hangs outside it: when it doesn't fit, or always; new type: \"standard\"), composer?: \"singleLine\"|\"everyLine\" (line breaking, Every-line by default), start?: byte, end?: byte} (baselineShift is in document points and hScale includes the object transform; groups include their text descendants; with a range: the character attributes style that range and the paragraph attributes apply to the paragraphs it touches; without: all the text)",
            has_doc,
            set_format
        ),
    ]
}

// ---------- artboards ----------

fn artboard_reorder(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "artboard.reorder";
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing index"))? as usize;
    let to = p.get("to").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing to"))? as usize;
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

/// The artboard (`index`) and whether its art comes along (`art`) for the artboard copy
/// commands: by default the Artboard tool's artboard and its Move/Copy Artwork with Artboard option.
fn board_and_art(s: &Session, p: &Value) -> (usize, bool) {
    let t = s.tool_options_of("artboard");
    let i = p.get("index").and_then(Value::as_u64).or_else(|| t["active"].as_u64()).unwrap_or(0) as usize;
    (i, bool_or(p, "art", t["moveArt"].as_bool().unwrap_or(true)))
}

/// The art that goes with artboard `i` when `art`: the objects fully inside it (locked and hidden
/// ones only with prefs moveLockedWithArtboard), in paint order.
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

pub(crate) fn artboard_art(s: &Session, i: usize, art: bool) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let rect = d.artboards.get(i).map(|a| a.rect).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    Ok(if art { d.art_on_artboard(rect, s.prefs.move_locked_with_artboard) } else { vec![] })
}

/// How far a copy of `rect` moves to sit right of the last artboard (Duplicate Artboards, Paste).
pub(crate) fn beside_artboards(d: &vectorcraft_doc::Document, rect: Rect) -> Vec2 {
    let right = d.artboards.iter().map(|a| a.rect.x1).fold(f64::MIN, f64::max);
    Vec2::new(right + 20.0 - rect.x0, 0.0)
}

fn artboard_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let (i, art) = board_and_art(s, p);
    let art = artboard_art(s, i, art)?;
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &art)?, ..Scaling::default() };
    let (index, ids) = s.edit("Duplicate Artboard", |d, _| {
        let dv = d.artboards.get(i).map(|a| beside_artboards(d, a.rect)).unwrap_or_default();
        copy_artboard(d, i, dv, &art, sc)
    })?;
    Ok(json!({"index": index, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

/// Artboard Copy / Cut: the artboard and its art onto the clipboard (pasting brings both); a cut
/// then deletes them.
pub(crate) fn artboard_copy(s: &mut Session, p: &Value, cut: bool) -> Result<Value> {
    let (i, art) = board_and_art(s, p);
    let art = artboard_art(s, i, art)?;
    let st = s.doc()?;
    if cut && st.doc.artboards.len() <= 1 {
        return Err(EngineError::Other("a document needs at least one artboard".into()));
    }
    let board = st.doc.artboards.get(i).cloned().ok_or_else(|| EngineError::Other("no such artboard".into()))?;
    let clip = Clipboard { source_artboard: Some(board.rect), artboard: Some(board), ..Clipboard::copy(st, &art) };
    let copied = clip.nodes.len();
    if cut {
        s.edit("Cut Artboard", |d, sel| {
            if i >= d.artboards.len() {
                return Err(EngineError::Other("no such artboard".into()));
            }
            // Its guides go with it.
            let gone = d.artboards.remove(i).id;
            d.retain_guides(sel, |_, g| g.artboard != Some(gone));
            d.remove_many(&art)?;
            sel.prune(d);
            Ok(())
        })?;
        if s.tool_id() == "artboard" {
            s.set_tool_option("active", &json!(i.saturating_sub(1)));
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

/// Add a copy of artboard `src` at `rect`, with an id of its own and its name, or `<name> copy`
/// (`<name> copy 2`…) when that is taken → its index.
pub(crate) fn push_artboard_copy(d: &mut vectorcraft_doc::Document, src: &vectorcraft_doc::Artboard, rect: Rect) -> usize {
    let mut a = src.clone();
    a.id = d.next_artboard_id();
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
        "hyphenate",
        "wordsLongerThan",
        "afterFirst",
        "beforeLast",
        "hyphenLimit",
        "hyphenationZone",
        "hyphenateCapitalized",
        "mojikumi",
        "kinsoku",
        "direction",
        "leadingModel",
        "charAlign",
        "proportionalMetrics",
        "digits",
        "burasagari",
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
    let digits = super::textedit::digits_param(p, C)?;
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
        if let Some(v) = digits {
            st.digits = v;
        }
        if let Some(v) = flag("proportionalMetrics") {
            st.proportional_metrics = v;
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
                if let Some(v) = composer {
                    para.composer = v;
                }
            });
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
