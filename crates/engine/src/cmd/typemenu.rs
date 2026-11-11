//! Type menu long tail (Change Case, Smart Punctuation, Convert To Area/Point Type, Type on a Path
//! Options, Fill with Placeholder Text, Insert Special/Whitespace/Break characters) and the text
//! commands of the Edit menu (Find and Replace, Find Next, Paste without Formatting).

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Document, Justify, LockedArtboards, Node, NodeId, NodeKind, Quotes, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::object::choice;
use super::typecmd::refresh_bounds;
use super::{
    CommandSpec, EngineError, Result, Session, bad, checked_ids_param, cmd, find_command, has_clipboard, has_doc, has_selection, number_or,
    switch_or, targets, text_param,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "type.changeCase",
            "Change Case",
            ["Type", "Change Case"],
            None,
            "{case: \"upper\"|\"lower\"|\"title\"|\"sentence\", ids?|id?} → {ids}",
            has_selection,
            change_case
        ),
        cmd!(
            "type.smartPunctuation",
            "Smart Punctuation…",
            ["Type"],
            None,
            "{quotes?: true (straight quotes become the Document Setup quotes), dashes?: true (-- → en dash, --- → em dash), ellipsis?: true (... → …), scope?: \"selection\"|\"document\"} → {changed}",
            has_doc,
            smart_punctuation
        ),
        cmd!(
            "type.convertToAreaType",
            "Convert To Area Type",
            ["Type"],
            None,
            "{ids?|id?} point type → area type framed by its current layout bounds → {ids}",
            has_selection,
            to_area
        ),
        cmd!(
            "type.convertToPointType",
            "Convert To Point Type",
            ["Type"],
            None,
            "{ids?|id?} area type → point type; soft line wraps become line breaks, and a frame of threaded text leaves its thread with the text it holds → {ids}",
            has_selection,
            to_point
        ),
        cmd!(
            "type.pathOptions",
            "Type on a Path Options…",
            ["Type", "Type on a Path"],
            None,
            "{ids?|id?, start?: 0..1, end?: 0..1|null, flip?: bool, effect?: rainbow|skew|3dRibbon|stairStep|gravity, alignToPath?: ascender|descender|center|baseline, spacing?: pt} set the selected type on a path's options: start and end place its start and end brackets as fractions of the path's length (end null: the end of the path, or once round a closed path), and flip then turns the type to the other side of its path (the path runs the other way, the brackets swap ends); alignToPath runs each character's em box top (ascender), font bounding box bottom (descender), half the height of its H's top above the baseline (center) or baseline along the path, by the character's own font, size and vertical scale (vertical type: half an em either way, or none for center); none given: query → the first object's {start, end, flip: false, effect, alignToPath, spacing}",
            has_selection,
            path_options
        ),
        cmd!(
            "type.pathCenter.top",
            "Center on Top",
            ["Type", "Type on a Path"],
            None,
            "{ids?|id?} slide the selected type on a closed path along it so that the middle of the span between its start and end brackets lands on the path's topmost point as the object is drawn (after its transform, so a rotated, scaled or skewed path is measured as it appears; on a level stretch such as a rectangle's top edge, its middle; with several separate topmost points, the middle of the shortest part of the path that holds them all), keeping the span's length (a full turn stays a full turn), and set the first paragraph's alignment to Center unless it is Center or Justify Center, so the type is centered there. Point type and area type in the selection are left as they are. Type on an open or compound path, type in a thread, and type on a path with no length or no height are refused with an error that names them, and nothing changes. When every object is already centered there, nothing changes and no undo step is recorded → {ids, start, end} (the first object's brackets)",
            has_closed_path_type,
            path_center_top
        ),
        cmd!(
            "type.pathCenter.bottom",
            "Center on Bottom",
            ["Type", "Type on a Path"],
            None,
            "{ids?|id?} as type.pathCenter.top, on the path's bottommost point as drawn. The type is not flipped: on the bottom of a path that runs clockwise it reads upside down until Type on a Path Options › Flip turns it → {ids, start, end} (the first object's brackets)",
            has_closed_path_type,
            path_center_bottom
        ),
        cmd!(
            "type.dynamicText",
            "Dynamic Text",
            ["Type"],
            None,
            "{ids?|id?, on?: bool} turn Dynamic Text on or off for the selected type on a path (on omitted: the opposite of the first object's setting). On, the layout scales the text by one factor (every character's size, leading and baseline shift; Type on a Path Options › Spacing stays) so that it fills the span between its start and end brackets, larger or smaller, and fits it again after every edit to the text, the path or the brackets; the first character's size moves in steps of 0.1 pt within 0.1 to 1,296 pt, and at 0.1 pt text that still runs past the end bracket overflows. The stored sizes keep their proportions. Off, the text keeps the sizes it was drawn at. Point type and area type in the selection are left as they are; a selection holding type on a path in a thread is refused (threading type turns Dynamic Text off, keeping its drawn sizes). When every object already has the setting, nothing changes and no undo step is recorded → {on, ids, fitScale (the first object's factor, 1 when off)}",
            has_free_path_type,
            dynamic_text
        ),
        cmd!(
            "type.fillPlaceholder",
            "Fill with Placeholder Text",
            ["Type"],
            None,
            "{ids?|id?} replace selected text with placeholder text (area type is filled to its frame) → {ids}",
            has_selection,
            fill_placeholder
        ),
        cmd!(
            "type.insert",
            "Insert Character",
            ["Type"],
            None,
            "{char: \"bullet\"|\"copyright\"|\"ellipsis\"|\"paragraph\"|\"registered\"|\"section\"|\"trademark\"|\"emDash\"|\"enDash\"|\"discretionaryHyphen\"|\"nonBreakingHyphen\"|\"doubleLeftQuote\"|\"doubleRightQuote\"|\"singleLeftQuote\"|\"singleRightQuote\"|\"emSpace\"|\"enSpace\"|\"hairSpace\"|\"sixthSpace\"|\"thinSpace\"|\"nonBreakingSpace\"|\"figureSpace\"|\"punctuationSpace\"|\"thirdSpace\"|\"quarterSpace\"|\"tab\"|\"forcedLineBreak\"|\"paragraphReturn\" | text: string | code: Windows Alt+0 code (0–255 in Windows-1252, a Unicode code point above: 176 → °, 151 → —) | unicode: hex code point (\"00B0\" or \"U+00B0\"), ids?|id?: object ids (default: the selection)} append to the selected text objects",
            has_selection,
            insert_char
        ),
        cmd!(
            "edit.findReplace",
            "Find and Replace…",
            ["Edit"],
            None,
            "{find, replace?: \"\", matchCase?: false, wholeWord?: false, ids?} replace in every text object (or ids) → {count}. Matches never span style runs.",
            has_doc,
            find_replace
        ),
        cmd!(
            "edit.findNext",
            "Find Next",
            ["Edit"],
            None,
            "{find, matchCase?, wholeWord?} select the next text object (after the selected one, wrapping) containing `find` → {id|null}",
            has_doc,
            find_next
        ),
        cmd!(
            "edit.pasteWithoutFormatting",
            "Paste without Formatting",
            ["Edit"],
            Some("Cmd+Alt+V"),
            "{center?, dx?, dy?, swatchConflict?} paste as edit.paste; pasted text takes the default character style (one run) and leaves its character and paragraph styles behind → {ids, added, merged, renamed}",
            has_clipboard,
            paste_plain
        ),
    ]
}

/// Text objects among `ids` and their descendants.
pub(crate) fn text_ids(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let paths = d.index_paths(ids.iter().copied());
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for id in ids {
        if let Some(n) = paths.get(id).and_then(|p| d.node_at(p)) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::Text(_)) && seen.insert(c.id) {
                    out.push(c.id);
                }
            });
        }
    }
    out
}

fn texts(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let ids = targets(s, p)?;
    let t = text_ids(&s.doc()?.doc, &ids);
    if t.is_empty() {
        return Err(bad(cmd, "no text objects selected"));
    }
    Ok(t)
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Rewrite every run of the given text objects with `f(run text, state)`. When no run changes, the
/// document and its undo history stay as they are.
fn edit_runs<S: Default>(s: &mut Session, label: &str, ids: &[NodeId], f: impl Fn(&str, &mut S) -> String) -> Result<usize> {
    super::overprint::edit_counted(s, label, |d| {
        let mut changed = 0;
        for id in ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let mut st = S::default();
            let mut any = false;
            // Inline graphics are art, not characters to rewrite.
            for r in t.runs.iter_mut().filter(|r| r.inline.is_none()) {
                let n = f(&r.text, &mut st);
                if n != r.text {
                    r.text = n;
                    any = true;
                }
            }
            if any {
                changed += 1;
                refresh_bounds(t);
            }
        }
        Ok(changed)
    })
}

// ---------- Change Case ----------

/// Does `c` take a capital at the start of a word or sentence? Georgian has no word capitals:
/// Mtavruli is a separate all-caps style, so title and sentence case leave Mkhedruli (and turn
/// Mtavruli back into) lower case. Upper case still gives Mtavruli, as Unicode defines it.
fn capitalizes(c: char) -> bool {
    !matches!(c, '\u{10D0}'..='\u{10FF}' | '\u{1C90}'..='\u{1CBF}')
}

#[derive(Default)]
struct CaseState {
    /// Previous character (across runs).
    prev: Option<char>,
    /// Sentence start pending (sentence case).
    sentence: bool,
    started: bool,
}

fn change_case(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.changeCase";
    let case = text_param(p, "case", C)?.ok_or_else(|| bad(C, "missing case"))?.to_ascii_lowercase();
    if !["upper", "uppercase", "lower", "lowercase", "title", "sentence"].contains(&case.as_str()) {
        return Err(bad(C, "case must be upper|lower|title|sentence"));
    }
    let ids = texts(s, p, C)?;
    edit_runs::<CaseState>(s, "Change Case", &ids, |text, st| {
        if !st.started {
            st.started = true;
            st.sentence = true;
        }
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match case.as_str() {
                "upper" | "uppercase" => out.extend(c.to_uppercase()),
                "lower" | "lowercase" => out.extend(c.to_lowercase()),
                "title" => {
                    if capitalizes(c) && st.prev.is_none_or(|p| p.is_whitespace() || matches!(p, '-' | '(' | '"' | '“' | '/')) {
                        out.extend(c.to_uppercase())
                    } else {
                        out.extend(c.to_lowercase())
                    }
                }
                _ => {
                    if c.is_alphabetic() && st.sentence {
                        if capitalizes(c) {
                            out.extend(c.to_uppercase());
                        } else {
                            out.extend(c.to_lowercase());
                        }
                        st.sentence = false;
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    if matches!(c, '.' | '!' | '?' | '\n') {
                        st.sentence = true;
                    }
                }
            }
            st.prev = Some(c);
        }
        out
    })?;
    Ok(ids_json(&ids))
}

// ---------- Smart Punctuation ----------

#[derive(Default)]
struct PunctState {
    prev: Option<char>,
}

/// Dashes (`--` en, `---` em), ellipses and, with `quotes`, typographer's quotes.
pub(crate) fn smarten(text: &str, prev: &mut Option<char>, quotes: Option<&Quotes>, dashes: bool, ellipsis: bool) -> String {
    let mut s = text.to_string();
    if dashes {
        s = s.replace("---", "—").replace("--", "–");
    }
    if ellipsis {
        s = s.replace("...", "…");
    }
    match quotes {
        Some(q) => q.apply(&s, prev),
        None => {
            *prev = s.chars().last().or(*prev);
            s
        }
    }
}

fn smart_punctuation(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.smartPunctuation";
    let (d, e) = (switch_or(p, "dashes", true, C)?, switch_or(p, "ellipsis", true, C)?);
    let quotes = switch_or(p, "quotes", true, C)?;
    let document = choice(p, C, "scope", &["selection", "document"])? == Some("document");
    let st = s.doc()?;
    // Quotes in the document's style (Document Setup → Type).
    let q = quotes.then_some(st.doc.setup.quotes);
    let ids = if document || st.selection.is_empty() {
        let mut v = vec![];
        st.doc.walk(|n| {
            if matches!(n.kind, NodeKind::Text(_)) {
                v.push(n.id)
            }
        });
        v
    } else {
        text_ids(&st.doc, &st.selection.objects)
    };
    let n = edit_runs::<PunctState>(s, "Smart Punctuation", &ids, |text, st| smarten(text, &mut st.prev, q.as_ref(), d, e))?;
    Ok(json!({ "changed": n }))
}

// ---------- Area / Point conversion ----------

fn to_area(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = texts(s, p, "type.convertToAreaType")?;
    // With no point type among the objects, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| matches!(d.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::Point))) {
        return Ok(ids_json(&ids));
    }
    s.edit("Convert To Area Type", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            if !matches!(t.kind, TextKind::Point) {
                continue;
            }
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let mut b = if lay.bounds.area() > 0.0 { lay.bounds } else { t.estimate_bounds() };
            // Across, the frame spans the lines as they are aligned: from their starts to their ends
            // without the tracking after their last characters (`LineInfo::trailing`), not to the
            // ink, which tracking can pull past an advance. Area type aligns them alike.
            let (x0, x1) = lay.lines.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, z), l| (a.min(l.x0), z.max(l.x1 - l.trailing)));
            if x0.is_finite() && x1.is_finite() && x0 <= x1 {
                (b.x0, b.x1) = (x0, x1);
            }
            // Line breaking counts that tracking: the slack holds the most of it.
            let trailing = lay.lines.iter().map(|l| l.trailing).filter(|t| t.is_finite()).fold(0.0, f64::max);
            let size = t.first_style().size;
            // A little slack on the right so the text doesn't rewrap.
            let top = lay.lines.first().map(|l| l.baseline - l.ascent).unwrap_or(b.y0).min(b.y0);
            let slack = size * 0.5 + trailing;
            // Slack goes where the alignment leaves room, so the text doesn't move or rewrap.
            let (sl, sr) = match t.para_at(0).justify {
                Justify::Center | Justify::JustifyCenter => (slack * 0.5, slack * 0.5),
                Justify::Right | Justify::JustifyRight => (slack, 0.0),
                Justify::Auto if lay.lines.first().is_some_and(|l| l.rtl) => (slack, 0.0),
                _ => (0.0, slack),
            };
            let frame = Rect::new(b.x0 - sl, top, b.x1 + sr, b.y1.max(top + 1.0));
            t.kind = TextKind::Area { frame: shapes::rectangle(frame) };
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(ids_json(&ids))
}

/// (run, byte within the run) for a byte offset into the plain text.
fn locate(runs: &[TextRun], byte: usize) -> Option<(usize, usize)> {
    let mut off = 0;
    for (i, r) in runs.iter().enumerate() {
        if byte < off + r.text.len() {
            return Some((i, byte - off));
        }
        off += r.text.len();
    }
    None
}

fn to_point(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = texts(s, p, "type.convertToPointType")?;
    // With no area type among the objects, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| matches!(d.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::Area { .. }))) {
        return Ok(ids_json(&ids));
    }
    s.edit("Convert To Point Type", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let TextKind::Area { frame } = &t.kind else { continue };
            let fb = frame.bounds().unwrap_or_default();
            // Point type is in no thread: its first paragraph starts in it.
            t.first_para_continues = false;
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let plain = t.plain_text();
            let baseline = lay.lines.first().map(|l| l.baseline).unwrap_or(fb.y0 + t.first_style().size * 0.8);
            // Soft wraps → explicit breaks (replace the space before the wrap when there is one).
            let mut breaks: Vec<usize> =
                lay.lines.iter().skip(1).map(|l| l.start).filter(|&b| b > 0 && plain.as_bytes().get(b - 1) != Some(&b'\n')).collect();
            breaks.sort_unstable();
            breaks.dedup();
            for b in breaks.into_iter().rev() {
                // Each wrapped line becomes a paragraph with its paragraph's attributes.
                if plain.as_bytes().get(b - 1) == Some(&b' ') {
                    if let Some((ri, bi)) = locate(&t.runs, b - 1) {
                        t.splice_paras(b - 1, b, "\n");
                        t.runs[ri].text.replace_range(bi..bi + 1, "\n");
                    }
                } else if let Some((ri, bi)) = locate(&t.runs, b) {
                    t.splice_paras(b, b, "\n");
                    // Never inside an inline graphic's run: at the end of the run before it, or a
                    // run of its own.
                    match t.runs.get(ri) {
                        Some(r) if r.inline.is_some() => match ri.checked_sub(1).and_then(|p| t.runs.get_mut(p)) {
                            Some(prev) if prev.inline.is_none() => prev.text.push('\n'),
                            _ => {
                                let style = t.runs.get(ri).map(|r| r.style.clone()).unwrap_or_default();
                                t.runs.insert(ri, TextRun::new("\n", style));
                            }
                        },
                        _ => t.runs[ri].text.insert(bi, '\n'),
                    }
                }
            }
            // The point origin sits where the alignment anchors the first line.
            let p0 = t.para_at(0);
            let (x0, x1) = lay
                .lines
                .first()
                .map_or((fb.x0, fb.x1), |l| (l.avail.0 - p0.laid_out_left_indent() - p0.first_line_indent, l.avail.1 + p0.right_indent));
            let ox = match p0.justify {
                Justify::Center | Justify::JustifyCenter => (x0 + x1) * 0.5,
                Justify::Right | Justify::JustifyRight => x1,
                Justify::Auto if lay.lines.first().is_some_and(|l| l.rtl) => x1,
                _ => x0,
            };
            t.kind = TextKind::Point;
            t.xf *= Affine::translate((ox, baseline));
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(ids_json(&ids))
}

/// Whether type on a path `t` runs along one closed path (no open or compound path).
fn on_one_closed_path(t: &TextObject) -> bool {
    matches!(&t.kind, TextKind::OnPath { path, .. } if path.subpaths.len() == 1 && path.is_closed())
}

/// Why `type.pathCenter.top` and `.bottom` refuse type on a path `ids`: those on an open or
/// compound path, then those in a thread; None when they take every one.
fn path_center_refusal(d: &Document, ids: &[NodeId]) -> Option<String> {
    let open = ids.iter().filter(|id| !path_text(d.node(**id)).is_some_and(on_one_closed_path)).map(|id| id.0.to_string()).collect::<Vec<_>>();
    if !open.is_empty() {
        let open = open.join(", ");
        return Some(format!("type on an open or compound path cannot be centered on its top or bottom: {open}"));
    }
    let frames = threaded_ids(d, ids);
    (!frames.is_empty()).then(|| format!("type on a path in a thread cannot be centered on its top or bottom: {frames}"))
}

/// `type.pathCenter.top` and `.bottom` are enabled for a selection holding type on a path, all of
/// it on one closed path and none of it in a thread.
fn has_closed_path_type(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
    let d = &st.doc;
    let selected: Vec<NodeId> = st.selection.objects.to_vec();
    let paths: Vec<NodeId> = text_ids(d, &selected).into_iter().filter(|id| path_text(d.node(*id)).is_some()).collect();
    if paths.is_empty() {
        return Err("select type on a closed path".into());
    }
    path_center_refusal(d, &paths).map_or(Ok(()), Err)
}

/// The brackets that center type on a path `t` on its path's topmost point (`bottom`: its
/// bottommost) as `t.xf` draws it, its span keeping its length; None for a path with no length or
/// no height.
fn centered_brackets(t: &TextObject, bottom: bool) -> Option<(f64, Option<f64>)> {
    let TextKind::OnPath { path, end, .. } = &t.kind else { return None };
    let (from, to) = t.kind.path_span()?;
    let arc = vectorcraft_geom::ArcPath::new(path);
    if arc.is_empty() {
        return None;
    }
    let flip = if bottom { Affine::FLIP_Y } else { Affine::IDENTITY };
    let middle = arc.topmost(flip * t.xf)? / arc.len();
    let run = to - from;
    let start = (middle - run * 0.5).rem_euclid(1.0).clamp(0.0, 1.0);
    // Without an end bracket the type runs once round; a full turn's end stays on its start.
    let end = end.map(|_| if run >= 1.0 - 1e-12 { start } else { (start + run).rem_euclid(1.0).clamp(0.0, 1.0) });
    Some((start, end))
}

/// Whether brackets `a` and `b`, fractions of a closed path's length, are at one place within
/// rounding (0 and 1 are one place).
fn same_bracket(a: f64, b: f64) -> bool {
    let d = (a - b).rem_euclid(1.0);
    d.min(1.0 - d) <= 1e-9
}

fn path_center_top(s: &mut Session, p: &Value) -> Result<Value> {
    path_center(s, p, false)
}

fn path_center_bottom(s: &mut Session, p: &Value) -> Result<Value> {
    path_center(s, p, true)
}

/// Type › Type on a Path › Center on Top and Center on Bottom: slide the selected type on closed
/// paths along them so the middle of each span lands on the path's top (`bottom`: its bottom) as
/// drawn, and center the first paragraph, in one undo step; none when nothing changes.
fn path_center(s: &mut Session, p: &Value, bottom: bool) -> Result<Value> {
    let c = if bottom { "type.pathCenter.bottom" } else { "type.pathCenter.top" };
    let centered = |j: Justify| matches!(j, Justify::Center | Justify::JustifyCenter);
    // Each object the command changes, with its new brackets when they move and whether its first
    // paragraph is set to Center.
    let (ids, changes) = {
        let d = &s.doc()?.doc;
        let ids: Vec<NodeId> = texts(s, p, c)?.into_iter().filter(|i| path_text(d.node(*i)).is_some()).collect();
        if ids.is_empty() {
            return Err(bad(c, "select type on a closed path"));
        }
        if let Some(why) = path_center_refusal(d, &ids) {
            return Err(bad(c, why));
        }
        let (mut changes, mut flat) = (vec![], vec![]);
        for id in &ids {
            let Some(t) = path_text(d.node(*id)) else { continue };
            let TextKind::OnPath { start, end, .. } = &t.kind else { continue };
            let Some((to_start, to_end)) = centered_brackets(t, bottom) else {
                flat.push(id.0.to_string());
                continue;
            };
            let stays = same_bracket(*start, to_start)
                && match (end, to_end) {
                    (None, None) => true,
                    (Some(a), Some(b)) => same_bracket(*a, b),
                    _ => false,
                };
            let align = !centered(t.para_at(0).justify);
            if !stays || align {
                changes.push((*id, (!stays).then_some((to_start, to_end)), align));
            }
        }
        if !flat.is_empty() {
            let flat = flat.join(", ");
            return Err(bad(c, format!("type on a path with no length or no height cannot be centered: {flat}")));
        }
        (ids, changes)
    };
    if !changes.is_empty() {
        s.edit(if bottom { "Center on Bottom" } else { "Center on Top" }, |d, _| {
            for (id, brackets, align) in &changes {
                let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
                if let (Some(b), TextKind::OnPath { start, end, .. }) = (brackets, &mut t.kind) {
                    (*start, *end) = *b;
                }
                if *align {
                    t.edit_paras(Some(0..1), |para| para.justify = Justify::Center);
                }
                refresh_bounds(t);
            }
            Ok(())
        })?;
    }
    let d = &s.doc()?.doc;
    let (start, end) = ids.first().and_then(|id| path_text(d.node(*id))).map_or((0.0, None), |t| match &t.kind {
        TextKind::OnPath { start, end, .. } => (*start, *end),
        _ => (0.0, None),
    });
    Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "start": start, "end": end}))
}

/// The options `type.pathOptions` sets; with none of them given it queries.
const PATH_OPTIONS: [&str; 6] = ["start", "end", "flip", "effect", "alignToPath", "spacing"];

/// Type on a path's text, if `n` is type on a path.
fn path_text(n: Option<&vectorcraft_doc::Node>) -> Option<&TextObject> {
    match n.map(|n| &n.kind) {
        Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::OnPath { .. }) => Some(t),
        _ => None,
    }
}

/// Type on a Path Options of `t`, as `type.pathOptions` takes them (`flip` is an action: false).
fn path_options_of(t: &TextObject) -> Value {
    let (start, end) = match &t.kind {
        TextKind::OnPath { start, end, .. } => (*start, *end),
        _ => (0.0, None),
    };
    json!({
        "start": start,
        "end": end,
        "flip": false,
        "effect": t.path_effect.id(),
        "alignToPath": t.path_align.id(),
        "spacing": t.path_spacing,
    })
}

fn path_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.pathOptions";
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        texts(s, p, C)?.into_iter().filter(|i| path_text(d.node(*i)).is_some()).collect()
    };
    let first = *ids.first().ok_or_else(|| bad(C, "select type on a path"))?;
    let options = |s: &Session| -> Result<Value> {
        let t = path_text(s.doc()?.doc.node(first)).ok_or_else(|| bad(C, "select type on a path"))?;
        Ok(path_options_of(t))
    };
    if !PATH_OPTIONS.iter().any(|k| p.get(k).is_some()) {
        return options(s);
    }
    // A bracket: a fraction of the path's length.
    let fraction = |k: &str| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|x| x.is_finite()).map(|x| Some(x.clamp(0.0, 1.0))).ok_or_else(|| bad(C, format!("`{k}` is a number 0..1"))),
        }
    };
    let start = fraction("start")?;
    // `end: null` puts the end bracket back at the end of the path.
    let end = p.get("end").map(|_| fraction("end")).transpose()?;
    let flip = switch_or(p, "flip", false, C)?;
    let effect = match text_param(p, "effect", C)? {
        Some(e) => Some(vectorcraft_doc::PathEffect::parse(e).ok_or_else(|| bad(C, format!("unknown effect `{e}`")))?),
        None => None,
    };
    let align = match text_param(p, "alignToPath", C)? {
        Some(a) => Some(vectorcraft_doc::PathAlign::parse(a).ok_or_else(|| bad(C, format!("unknown alignToPath `{a}`")))?),
        None => None,
    };
    let spacing = match p.get("spacing") {
        None | Some(Value::Null) => None,
        Some(_) => Some(number_or(p, "spacing", 0.0, C)?.clamp(-1000.0, 1000.0)),
    };
    // Without a flip, when every object already has every value given, the document and its undo
    // history stay as they are.
    let has_values = |t: &TextObject| match &t.kind {
        TextKind::OnPath { start: st, end: en, .. } => {
            start.is_none_or(|v| v == *st)
                && end.is_none_or(|v| v == *en)
                && effect.is_none_or(|e| t.path_effect == e)
                && align.is_none_or(|a| t.path_align == a)
                && spacing.is_none_or(|v| t.path_spacing == v)
        }
        _ => true,
    };
    let d = &s.doc()?.doc;
    if !flip && ids.iter().all(|id| path_text(d.node(*id)).is_none_or(has_values)) {
        return options(s);
    }
    s.edit("Type on a Path Options", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let TextKind::OnPath { start: st, end: en, .. } = &mut t.kind else { continue };
            if let Some(v) = start {
                *st = v;
            }
            if let Some(v) = end {
                *en = v;
            }
            // After the brackets: they are given on the path as it runs before the flip.
            if flip {
                t.flip_on_path();
            }
            if let Some(e) = effect {
                t.path_effect = e;
            }
            if let Some(a) = align {
                t.path_align = a;
            }
            if let Some(v) = spacing {
                t.path_spacing = v;
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    options(s)
}

/// Whether type on a path `id` is a frame of a thread.
fn threaded(d: &Document, id: NodeId) -> bool {
    !super::threads::story_of(d, id).is_empty()
}

/// `type.dynamicText` is enabled for a selection holding type on a path, none of it in a thread.
fn has_free_path_type(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
    let d = &st.doc;
    let selected: Vec<NodeId> = st.selection.objects.to_vec();
    let paths: Vec<NodeId> = text_ids(d, &selected).into_iter().filter(|id| path_text(d.node(*id)).is_some()).collect();
    if paths.is_empty() {
        return Err("select type on a path".into());
    }
    let frames = threaded_ids(d, &paths);
    if !frames.is_empty() {
        return Err(format!("type on a path in a thread cannot use Dynamic Text: {frames}"));
    }
    Ok(())
}

/// The ids among `ids` of frames of threads, as a list for a message; empty when there are none.
fn threaded_ids(d: &Document, ids: &[NodeId]) -> String {
    ids.iter().filter(|id| threaded(d, **id)).map(|id| id.0.to_string()).collect::<Vec<_>>().join(", ")
}

/// Type › Dynamic Text: set `on` (without it, the opposite of the first object's setting) on the
/// selected type on a path, in one undo step, and none when every object already has it. Turning
/// it off keeps the sizes the text was drawn at ([`bake_dynamic_text`]).
fn dynamic_text(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.dynamicText";
    let (ids, on, changing) = {
        let d = &s.doc()?.doc;
        let ids: Vec<NodeId> = texts(s, p, C)?.into_iter().filter(|i| path_text(d.node(*i)).is_some()).collect();
        let first = *ids.first().ok_or_else(|| bad(C, "select type on a path"))?;
        let frames = threaded_ids(d, &ids);
        if !frames.is_empty() {
            return Err(bad(C, format!("type on a path in a thread cannot use Dynamic Text: {frames}")));
        }
        let on = switch_or(p, "on", !path_text(d.node(first)).is_some_and(|t| t.dynamic_text), C)?;
        let changing: Vec<NodeId> = ids.iter().copied().filter(|id| path_text(d.node(*id)).is_some_and(|t| t.dynamic_text != on)).collect();
        (ids, on, changing)
    };
    if !changing.is_empty() {
        s.edit("Dynamic Text", |d, _| {
            for id in &changing {
                let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
                if on {
                    t.dynamic_text = true;
                    refresh_bounds(t);
                } else {
                    bake_dynamic_text(t);
                }
            }
            Ok(())
        })?;
    }
    let d = &s.doc()?.doc;
    let fit =
        ids.first().and_then(|id| path_text(d.node(*id))).map_or(1.0, |t| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).fit_scale);
    Ok(json!({"on": on, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "fitScale": fit}))
}

/// Turn Dynamic Text off on `t`, keeping the sizes its layout draws it at: every run's size,
/// leading and baseline shifts take the fit factor ([`vectorcraft_text::scaled`]). False when it
/// was already off.
pub(crate) fn bake_dynamic_text(t: &mut TextObject) -> bool {
    if !t.dynamic_text {
        return false;
    }
    let f = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).fit_scale;
    if f.is_finite() && f != 1.0 {
        *t = vectorcraft_text::scaled(t, f);
    }
    t.dynamic_text = false;
    refresh_bounds(t);
    true
}

// ---------- placeholder / insert ----------

/// VectorCraft's own placeholder copy (not the classical Latin text).
pub(crate) const PLACEHOLDER: &str = "Sample copy flows here while the layout takes shape. Swap these words for real text once the \
design is settled. Every line is only a stand-in that shows size, rhythm and colour. Headlines, captions and body text all \
start as rough drafts like this one. Keep going until the frame is full and the page feels balanced.";

fn fill_placeholder(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = texts(s, p, "type.fillPlaceholder")?;
    // When filling leaves every object as it was, the document and its undo history stay as they
    // are.
    super::overprint::edit_counted(s, "Fill with Placeholder Text", |d| {
        let mut changed = 0;
        for id in &ids {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                let before = t.clone();
                fill_with_placeholder(t);
                changed += usize::from(*t != before);
            }
        }
        Ok(changed)
    })?;
    Ok(ids_json(&ids))
}

/// Replace `t`'s text with placeholder text in its first style: area type is filled to its frame,
/// other type gets a sentence (Type › Fill with Placeholder Text, and new type with Preferences ›
/// Type › Fill New Type Objects With Placeholder Text).
pub(crate) fn fill_with_placeholder(t: &mut TextObject) {
    let style = t.first_style();
    let text = match &t.kind {
        TextKind::Area { frame } => {
            let b = frame.bounds().unwrap_or_default();
            let chars_per_line = (b.width() / (style.size * 0.5)).max(1.0);
            // The first paragraph's Auto Leading sets the leading of Auto type (a leading of 0
            // counts as a tenth of a point a line).
            let lines = (b.height() / t.para_at(0).effective_leading(&style).max(0.1)).max(1.0);
            let want = (chars_per_line * lines) as usize;
            let mut out = String::new();
            for (i, w) in PLACEHOLDER.split(' ').cycle().enumerate() {
                if out.len() >= want || i >= 10_000 {
                    break;
                }
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(w);
            }
            out
        }
        _ => PLACEHOLDER.split(". ").next().unwrap_or(PLACEHOLDER).to_string() + ".",
    };
    t.runs = vec![TextRun { text, style, inline: None }];
    refresh_bounds(t);
}

pub(crate) fn special_char(name: &str) -> Option<&'static str> {
    Some(match name {
        "bullet" => "•",
        "copyright" => "©",
        "ellipsis" => "…",
        "paragraph" => "¶",
        "registered" => "®",
        "section" => "§",
        "trademark" => "™",
        "emDash" => "—",
        "enDash" => "–",
        "discretionaryHyphen" => "\u{00AD}",
        "nonBreakingHyphen" => "\u{2011}",
        "doubleLeftQuote" => "“",
        "doubleRightQuote" => "”",
        "singleLeftQuote" => "‘",
        "singleRightQuote" => "’",
        "emSpace" => "\u{2003}",
        "enSpace" => "\u{2002}",
        "hairSpace" => "\u{200A}",
        "sixthSpace" => "\u{2006}",
        "thinSpace" => "\u{2009}",
        "nonBreakingSpace" => "\u{00A0}",
        "figureSpace" => "\u{2007}",
        "punctuationSpace" => "\u{2008}",
        "thirdSpace" => "\u{2004}",
        "quarterSpace" => "\u{2005}",
        "tab" => "\t",
        // Our text model breaks lines at paragraph returns only.
        "forcedLineBreak" | "paragraphReturn" => "\n",
        _ => return None,
    })
}

/// The character a Windows Alt + keypad code with a leading 0 types (`Alt+0176` → `°`), as
/// `type.insert {code}` takes it: 128–159 in the Windows-1252 code page (`0151` → `—`), the
/// Unicode code point otherwise (`8364` → `€`). Control characters type nothing.
/// <https://www.unicode.org/Public/MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1252.TXT>
pub(crate) fn alt_code_char(code: u64) -> Option<char> {
    const CP1252: [u16; 32] = [
        0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C,
        0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
    ];
    let cp = match code {
        128..=159 => u32::from(*CP1252.get(usize::try_from(code - 128).ok()?)?),
        _ => u32::try_from(code).ok()?,
    };
    char::from_u32(cp).filter(|c| !c.is_control() || *c == '\t')
}

/// The character `type.insert {unicode}` names: its code point in hex, as `00B0`, `U+00B0` or
/// `0x00B0`.
fn unicode_char(u: &str) -> Option<char> {
    let u = u.trim();
    let hex = u.strip_prefix("U+").or_else(|| u.strip_prefix("u+")).or_else(|| u.strip_prefix("0x")).unwrap_or(u);
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    char::from_u32(u32::from_str_radix(hex, 16).ok()?).filter(|c| !c.is_control() || *c == '\t')
}

fn insert_char(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.insert";
    // Every source is type-checked before one is chosen, so a bad value is refused even when a
    // source ahead of it is given.
    let ch = text_param(p, "char", C)?;
    let txt = text_param(p, "text", C)?;
    let code = match p.get("code").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(C, "`code` must be a whole number"))?),
        None => None,
    };
    let uni = text_param(p, "unicode", C)?;
    let text = match (ch, txt, code, uni) {
        (Some(c), ..) => special_char(c).ok_or_else(|| bad(C, format!("unknown character `{c}`")))?.to_string(),
        (None, Some(t), ..) => t.to_string(),
        (None, None, Some(code), _) => alt_code_char(code).ok_or_else(|| bad(C, format!("no character has code {code}")))?.to_string(),
        (None, None, None, Some(u)) => unicode_char(u).ok_or_else(|| bad(C, format!("`{u}` is no Unicode character")))?.to_string(),
        (None, None, None, None) => return Err(bad(C, "give `char`, `text`, `code` or `unicode`")),
    };
    let ids = texts(s, p, C)?;
    // When `text` is empty and every object ends with a run of text, the document and its undo
    // history stay as they are.
    let d = &s.doc()?.doc;
    let ends_with_text = |id: &NodeId| match d.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => t.runs.last().is_some_and(|r| r.inline.is_none()),
        _ => false,
    };
    if text.is_empty() && ids.iter().all(ends_with_text) {
        return Ok(ids_json(&ids));
    }
    s.edit("Typing", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            match t.runs.last_mut() {
                Some(r) if r.inline.is_none() => r.text.push_str(&text),
                // After an inline graphic: a run of its own, in its style.
                Some(r) => {
                    let style = r.style.clone();
                    t.runs.push(TextRun::new(text.clone(), style));
                }
                None => t.runs.push(TextRun { text: text.clone(), style: CharStyle::default(), inline: None }),
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(ids_json(&ids))
}

// ---------- Find and Replace ----------

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte ranges of `needle` in `hay`.
pub(crate) fn find_all(hay: &str, needle: &str, match_case: bool, whole_word: bool) -> Vec<(usize, usize)> {
    find_all_between(hay, needle, match_case, whole_word, None, None)
}

/// [`find_all`] in run `i` of `runs`: a whole word looks past the run's ends at the characters
/// next to it, so a change of style inside a word is not a word boundary (#5901). Matches stay
/// inside the run.
fn find_in_run(runs: &[TextRun], i: usize, needle: &str, match_case: bool, whole_word: bool) -> Vec<(usize, usize)> {
    let Some(run) = runs.get(i) else { return vec![] };
    let before = runs.get(..i).and_then(|r| r.iter().rev().find_map(|r| r.text.chars().next_back()));
    let after = runs.get(i + 1..).and_then(|r| r.iter().find_map(|r| r.text.chars().next()));
    find_all_between(&run.text, needle, match_case, whole_word, before, after)
}

/// [`find_all`], with `before` and `after` the characters on either side of `hay` (none at the
/// text's ends), which a whole word must not touch either.
fn find_all_between(hay: &str, needle: &str, match_case: bool, whole_word: bool, before: Option<char>, after: Option<char>) -> Vec<(usize, usize)> {
    let nc: Vec<char> = needle.chars().collect();
    if nc.is_empty() {
        return vec![];
    }
    let hc: Vec<(usize, char)> = hay.char_indices().collect();
    let eq = |a: char, b: char| if match_case { a == b } else { a == b || a.to_lowercase().eq(b.to_lowercase()) };
    let mut out = vec![];
    let mut i = 0;
    while i + nc.len() <= hc.len() {
        if (0..nc.len()).all(|k| eq(hc[i + k].1, nc[k])) {
            let prev = i.checked_sub(1).map_or(before, |j| hc.get(j).map(|x| x.1));
            let next = hc.get(i + nc.len()).map_or(after, |x| Some(x.1));
            if !whole_word || (!prev.is_some_and(is_word) && !next.is_some_and(is_word)) {
                let start = hc[i].0;
                let end = hc.get(i + nc.len()).map(|x| x.0).unwrap_or(hay.len());
                out.push((start, end));
                i += nc.len();
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The text objects Find and Replace and Find Next reach: on visible, unlocked layers, and not
/// inside a hidden or locked group, which hides or locks what it holds as a layer does (#5900).
/// The visible, unlocked type objects of visible, unlocked layers, off locked artboards, in paint
/// order: the text Find and Replace and Check Spelling go through.
pub(crate) fn all_texts(d: &Document) -> Vec<NodeId> {
    fn visit(n: &Node, locks: &LockedArtboards<'_>, v: &mut Vec<NodeId>) {
        if !n.visible || n.locked {
            return;
        }
        if matches!(n.kind, NodeKind::Text(_)) && !locks.holds_id(n.id) {
            v.push(n.id);
        }
        for c in n.children().into_iter().flatten() {
            visit(c, locks, v);
        }
    }
    let mut v = vec![];
    let locks = d.locked_artboards();
    for l in &d.layers {
        visit(l, &locks, &mut v);
    }
    v
}

fn find_replace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.findReplace";
    let find = text_param(p, "find", C)?.filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "missing `find`"))?.to_string();
    let rep = text_param(p, "replace", C)?.unwrap_or("").to_string();
    let (mc, ww) = (switch_or(p, "matchCase", false, C)?, switch_or(p, "wholeWord", false, C)?);
    let ids = match p.get("ids").filter(|v| !v.is_null()) {
        Some(_) => text_ids(&s.doc()?.doc, &checked_ids_param(s, p, "ids", C)?),
        None => all_texts(&s.doc()?.doc),
    };
    let st = s.doc()?;
    let total: usize = ids
        .iter()
        .filter_map(|id| match &st.doc.node(*id)?.kind {
            NodeKind::Text(t) => Some((0..t.runs.len()).map(|i| find_in_run(&t.runs, i, &find, mc, ww).len()).sum::<usize>()),
            _ => None,
        })
        .sum();
    if total == 0 {
        return Ok(json!({ "count": 0 }));
    }
    super::overprint::edit_counted(s, "Replace", |d| {
        let mut changed = 0;
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            // Matches as (run, start, end) with offsets inside the run. Inline graphics are art, not
            // characters to rewrite.
            let mut hits = vec![];
            let mut base = 0;
            for (ri, r) in t.runs.iter().enumerate() {
                if r.inline.is_none() {
                    for (a, b) in find_in_run(&t.runs, ri, &find, mc, ww) {
                        if r.text.get(a..b) != Some(rep.as_str()) {
                            hits.push((ri, base + a, base + b, a, b));
                        }
                    }
                }
                base += r.text.len();
            }
            if hits.is_empty() {
                continue;
            }
            // Last match first, so the offsets of earlier ones stay valid. The paragraph styles follow
            // each edit, as `text.editRange` does.
            for (ri, ta, tb, a, b) in hits.into_iter().rev() {
                t.splice_paras(ta, tb, &rep);
                if let Some(r) = t.runs.get_mut(ri) {
                    r.text.replace_range(a..b, &rep);
                }
            }
            changed += 1;
            refresh_bounds(t);
        }
        Ok(changed)
    })?;
    Ok(json!({ "count": total }))
}

fn find_next(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.findNext";
    let find = text_param(p, "find", C)?.filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "missing `find`"))?.to_string();
    let (mc, ww) = (switch_or(p, "matchCase", false, C)?, switch_or(p, "wholeWord", false, C)?);
    let st = s.doc()?;
    let all = all_texts(&st.doc);
    let cur = st.selection.objects.first().and_then(|id| all.iter().position(|x| x == id));
    let n = all.len();
    let start = cur.map(|c| c + 1).unwrap_or(0);
    let hit = (0..n).map(|k| all[(start + k) % n.max(1)]).find(|id| match st.doc.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => !find_all(&t.plain_text(), &find, mc, ww).is_empty(),
        _ => false,
    });
    match hit {
        Some(id) => {
            s.select(|_, sel| sel.set([id]))?;
            Ok(json!({ "id": id.0 }))
        }
        None => Ok(json!({ "id": null })),
    }
}

// ---------- Paste without Formatting ----------

fn strip_formatting(n: &mut vectorcraft_doc::Node) {
    if let NodeKind::Text(t) = &mut n.kind {
        let text = t.plain_text();
        t.runs = vec![TextRun { text, style: CharStyle::default(), inline: None }];
        t.set_all_paras(Default::default());
        refresh_bounds(t);
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            strip_formatting(std::sync::Arc::make_mut(c));
        }
    }
}

fn paste_plain(s: &mut Session, p: &Value) -> Result<Value> {
    let saved = s.clipboard.nodes.clone();
    for n in &mut s.clipboard.nodes {
        strip_formatting(n);
    }
    let paste = find_command("edit.paste").map(|c| c.run).ok_or_else(|| EngineError::Other("paste unavailable".into()))?;
    let r = paste(s, p);
    s.clipboard.nodes = saved;
    if r.is_ok()
        && let Some(e) = s.active_mut().and_then(|d| d.history.undo.last_mut())
    {
        e.label = "Paste without Formatting".into();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::unicode_char;

    #[test]
    fn unicode_char_rejects_sign_in_hex_digits() {
        assert_eq!(unicode_char("+B0"), None);
        assert_eq!(unicode_char("U++B0"), None);
        assert_eq!(unicode_char("U+"), None);
        assert_eq!(unicode_char("00B0"), Some('°'));
        assert_eq!(unicode_char("U+00B0"), Some('°'));
        assert_eq!(unicode_char("0x00B0"), Some('°'));
    }
}
