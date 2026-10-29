//! Type menu: Create Outlines, and the Character/Paragraph panel setters (`text.setText`,
//! `text.setStyle`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{AreaFit, Document, Justify, Node, NodeId, NodeKind, RubyAlign, RubyPosition, RubySpan, TextObject};
use vectorcraft_geom::{Affine, PathData, Vec2};

use super::edit::selected_roots;
use super::pathops::{len_param, num_param, shape_node};
use super::{
    CommandSpec, EngineError, Result, Session, bad, check_numbers, cmd, color_value, f64_req, has_doc, has_selection, id_param, index, number_or, ok,
    str_param, switch_or, targets,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("type.orientation.vertical", "Vertical", ["Type", "Orientation"], None, "{ids?|id?} set vertical writing", has_selection, |s, p| {
            orientation(s, p, true)
        }),
        cmd!(
            "type.orientation.horizontal",
            "Horizontal",
            ["Type", "Orientation"],
            None,
            "{ids?|id?} set horizontal writing",
            has_selection,
            |s, p| orientation(s, p, false)
        ),
        cmd!(
            "type.createOutlines",
            "Create Outlines",
            ["Type"],
            Some("Cmd+Shift+O"),
            "{} convert the selected text to groups of compound paths (one per glyph); a frame of threaded text leaves its thread with the text it shows → {ids}",
            has_selection,
            create_outlines
        ),
        cmd!(
            "type.scaleTextAttributes",
            "Scale Text Attributes",
            ["Type"],
            None,
            "{ids?|id?} bake each selected type object's transform scale into its character sizes (leading, baseline shift and horizontal scale included), leaving rotation and position in the affine → {ids, changed}",
            scale_text_attributes_enabled,
            scale_text_attributes
        ),
        cmd!(
            "type.opticalMarginAlignment",
            "Optical Margin Alignment",
            ["Type"],
            None,
            "{ids?|id?, on?: bool} turn Optical Margin Alignment on or off for the selected type and every frame of its threads (on omitted: the opposite of the first object's setting): in every paragraph of point and area type, quotation marks, hyphens, full stops and commas at a line's start or end hang wholly outside the margin there; asterisks, tildes, ellipses, en and em dashes, colons and semicolons hang by half; and A, V, W, Y, v, w and y hang by a tenth of their advance; a left-aligned line hangs at its left margin, a right-aligned line hangs at its right margin, and centered and justified lines hang at both; Auto aligns a right-to-left line at its right; type on a path hangs at the start bracket when its line is placed there, hangs at the end bracket when the line is placed there, and hangs at both when it is centered → {on, ids}",
            has_selection,
            optical_margin_alignment
        ),
        cmd!(
            "type.systemLayout",
            "System Layout",
            [],
            None,
            "{on?: bool} turn System Layout on or off for the document (on omitted: the opposite of the document's setting) and every text object in it. On, each character that takes any space advances by a whole number of points, at least one, so small type does not run together; a character with no width stays at none. Off (the default, and what a document from before it reads as), spacing uses fractions of a point. New point, area and path type inherits the document's setting → {on}",
            has_doc,
            system_layout
        ),
        cmd!(
            "type.fractionalWidths",
            "Fractional Widths",
            [],
            None,
            "{ids?|id?, on?: bool} turn Fractional Widths on or off for the selected type and every frame of its threads (on omitted: the opposite of the first object's setting). On (the default, and what a document from before it reads as), spacing between characters uses fractions of a point. Off, each character that takes any space advances by a whole number of points, at least one, so small type does not run together; a character with no width stays at none. The setting is the whole type object, not a range of characters → {on, ids}",
            has_selection,
            fractional_widths
        ),
        cmd!(
            "text.setText",
            "Set Text",
            [],
            None,
            "{id?|ids?, text} replace the contents of text objects (keeps the first run's style) → {ids, overflow?: [id…] (area type and type on a path whose text no longer fits; left out when all fits)}",
            has_doc,
            set_text
        ),
        cmd!(
            "text.areaOptions",
            "Area Type Options…",
            ["Type"],
            None,
            "{ids?|id?, width?: pt, height?: pt, rows?, columns?, gutter?: pt, inset?: pt, firstBaseline?: ascent|capHeight|xHeight|leading|fixed, firstBaselineMin?: pt, verticalAlign?: top|center|bottom|justify, fit?: none|autoHeight|shrinkText, fitMinPercent?: 10..100, …the keys of its own reply (it can be passed back; the values it only reports are ignored)} set the selected area type's options; width and height size the type area from its top-left corner, along the type's own axes, and the text reflows at its size (with fit autoHeight, Auto Size, the frame's height follows the text after every edit and height is ignored; setting the height by hand turns it off); fit shrinkText scales overflowing text down (size, leading, baseline shift) by the largest factor down to fitMinPercent % (default 50) that makes it fit (none given: query) → the first object's options, plus overflow (the text doesn't fit its frame) and fitScale (Shrink Text's factor, 1 unshrunk)",
            has_selection,
            area_options
        ),
        cmd!(
            "text.reshapeArea",
            "Reshape Type Area",
            [],
            None,
            "{id, anchors: [[subpath, anchor]…], dx, dy} or {items: [{id, anchors}…], dx, dy} move anchors of area type's frame (the type area) by dx, dy points, with their handles; the text reflows at its size, through its thread too (what Direct Selection does dragging a frame corner or edge, or the frame anchors its marquee selected, and the arrow keys nudging those); with items, every listed frame moves in one step",
            has_doc,
            reshape_area
        ),
        cmd!(
            "text.reshapePath",
            "Reshape Type Path",
            [],
            None,
            "{id, anchors: [[subpath, anchor]…], dx, dy} move anchors of type on a path's path by dx, dy points, with their handles; the start and end brackets keep their fractions of the path and the text reflows along it (what Direct Selection does dragging an anchor of the path)",
            has_doc,
            reshape_path
        ),
        cmd!(
            "text.fitHeadline",
            "Fit Headline",
            ["Type"],
            None,
            "{ids?|id?} track the first line of area type so it fills the frame width → {tracking}",
            has_selection,
            fit_headline
        ),
        cmd!(
            "text.setStyle",
            "Character",
            [],
            None,
            "{ids?|id?, font?, style? (font alone: the style is kept when the new family has it, else its closest), size?: pt, leading?: pt|\"auto\", tracking?: 1/1000 em, justify?: \"auto\" (the start of each paragraph's direction)|\"left\"|\"center\"|\"right\"|\"justifyAll\", fill?: colour, keepModel?: bool (as paint.setFill), features?: [\"dlig\", \"-liga\", …] OpenType, start?: byte, end?: byte} (size/leading are document points after object scaling; with a range: the character attributes style that range and justify the paragraphs it touches; without: all the text) → {ids, overflow?: [id…] (area type and type on a path whose text no longer fits; left out when all fits)}",
            has_doc,
            set_style
        ),
        cmd!(
            "type.ruby",
            "Ruby…",
            [],
            None,
            "{ids?|id?, start?: byte, end?: byte, text?: reading, position?: above|below (vertical type: right|left), align?: start|center|end, size?: 1–100 (percent of the base text's size, default 50), overhang?: bool (default true), remove?: bool} set the ruby (furigana) over the selected text's bytes in `start..end` (without them: all its text): the reading over the base characters, above them in horizontal type and right of them in vertical type; `remove` takes the ruby off the range instead → {ids, ruby: [{start, end, text, position, align, sizePercent, overhang}…] (the first object's ruby)}",
            has_selection,
            ruby
        ),
    ]
}

fn orientation(s: &mut Session, p: &Value, vertical: bool) -> Result<Value> {
    let ids = text_targets(s, p, "Text Orientation")?;
    s.edit("Text Orientation", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else {
                return Err(EngineError::NoNode(*id));
            };
            t.vertical = vertical;
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({"vertical": vertical, "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}))
}

/// Type › Optical Margin Alignment: set `on` (without it, the opposite of the first object's
/// setting) on every selected text object and every frame of their threads, in one undo step.
fn optical_margin_alignment(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = super::threads::with_threads(&s.doc()?.doc, &text_targets(s, p, "Optical Margin Alignment")?);
    let first = ids.first().and_then(|id| match s.doc().ok()?.doc.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t.optical_margin),
        _ => None,
    });
    let on = switch_or(p, "on", !first.unwrap_or(false), "type.opticalMarginAlignment")?;
    s.edit("Optical Margin Alignment", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else {
                return Err(EngineError::NoNode(*id));
            };
            t.optical_margin = on;
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({"on": on, "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}))
}

/// Character panel menu › System Layout: set `on` (without it, the opposite of the document's
/// setting) on the document and every text object, in one undo step. Off is the default.
fn system_layout(s: &mut Session, p: &Value) -> Result<Value> {
    let current = s.doc()?.doc.setup.system_layout;
    let on = switch_or(p, "on", !current, "type.systemLayout")?;
    s.edit("System Layout", |d, _| {
        d.setup.system_layout = on;
        let mut ids = vec![];
        d.walk(|n| {
            if matches!(n.kind, NodeKind::Text(_)) {
                ids.push(n.id);
            }
        });
        for id in ids {
            let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else {
                continue;
            };
            t.system_layout = on;
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "on": on }))
}

/// Character panel menu › Fractional Widths: set `on` (without it, the opposite of the first
/// object's setting) on every selected text object and every frame of their threads, in one undo
/// step. On is the default.
fn fractional_widths(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = super::threads::with_threads(&s.doc()?.doc, &text_targets(s, p, "Fractional Widths")?);
    let first = ids.first().and_then(|id| match s.doc().ok()?.doc.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t.fractional_widths),
        _ => None,
    });
    let on = switch_or(p, "on", !first.unwrap_or(true), "type.fractionalWidths")?;
    s.edit("Fractional Widths", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else {
                return Err(EngineError::NoNode(*id));
            };
            t.fractional_widths = on;
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({"on": on, "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}))
}

/// Type › Ruby: set or remove the ruby (furigana) over a byte range of the selected type in one
/// undo step. Without `start`/`end` the range is all the text; a new reading replaces the ruby
/// over the range. The reply lists the first object's ruby (start/end in bytes of its plain text).
fn ruby(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.ruby";
    let ids = text_targets(s, p, C)?;
    let range = TextRange::parse(p, C)?;
    let remove = switch_or(p, "remove", false, C)?;
    let reading = str_param(p, "text").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if !remove && reading.is_none() {
        return Err(bad(C, "give `text` (the reading) or `remove`"));
    }
    let position = match str_param(p, "position").map(str::to_ascii_lowercase).as_deref() {
        None | Some("above") | Some("right") => RubyPosition::Above,
        Some("below") | Some("left") => RubyPosition::Below,
        Some(_) => return Err(bad(C, "`position` is above or below")),
    };
    let align = match str_param(p, "align").map(str::to_ascii_lowercase).as_deref() {
        None | Some("center") | Some("centre") => RubyAlign::Center,
        Some("start") | Some("left") | Some("top") => RubyAlign::Start,
        Some("end") | Some("right") | Some("bottom") => RubyAlign::End,
        Some(_) => return Err(bad(C, "`align` is start, center or end")),
    };
    let size = number_or(p, "size", 50.0, C)?;
    if !size.is_finite() || !(1.0..=100.0).contains(&size) {
        return Err(bad(C, "`size` is a percentage of the base text's size, 1 to 100"));
    }
    let overhang = switch_or(p, "overhang", true, C)?;
    let mut first: Option<Value> = None;
    s.edit("Ruby", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else {
                return Err(EngineError::NoNode(*id));
            };
            let (a, b) = match range {
                Some(r) => r.bytes(t),
                None => (0, vectorcraft_text::edit::runs_len(&t.runs)),
            };
            // The ruby over the range goes; a new reading takes its place.
            t.ruby.retain(|r| r.end <= a || r.start >= b);
            if let Some(text) = &reading
                && a < b
            {
                let mut span = RubySpan::new(a, b, text.clone());
                span.position = position;
                span.align = align;
                span.size_percent = size;
                span.overhang = overhang;
                t.ruby.push(span);
            }
            t.normalize_ruby();
            refresh_bounds(t);
            if first.is_none() {
                first = Some(serde_json::to_value(&t.ruby).unwrap_or(Value::Null));
            }
        }
        Ok(())
    })?;
    Ok(json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>(), "ruby": first.unwrap_or(Value::Null)}))
}

/// Recompute the layout caches (bounds and baselines) after a text edit. Every engine text edit ends here (typing
/// through `text.editRange` too), inside its `Session::edit`, so this is where Auto Size area type
/// ([`vectorcraft_doc::AreaFit::AutoHeight`]) fits its frame to the text: part of the same undo
/// step as the edit.
pub(crate) fn refresh_bounds(t: &mut TextObject) {
    // Edits that changed the paragraph count keep one paragraph style per paragraph.
    t.normalize_paras();
    auto_height(t);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    t.cached_bounds = Some(lay.bounds);
    t.cached_baselines = lay.baselines();
}

/// The text-space bounds of area type's frame, when Auto Size can fit it: a rectangle (one
/// subpath, as large as its bounds) of horizontal type in one row, with some height.
fn auto_height_frame(t: &TextObject) -> Option<vectorcraft_geom::Rect> {
    use vectorcraft_geom::Shape;
    let vectorcraft_doc::TextKind::Area { frame } = &t.kind else { return None };
    if t.area.fit != vectorcraft_doc::AreaFit::AutoHeight || t.vertical || t.area.rows > 1 || frame.subpaths.len() != 1 {
        return None;
    }
    let b = frame.bounds()?;
    let area = frame.to_bezpath().area().abs();
    let finite = [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite());
    (finite && b.height() > 1e-6 && b.width() > 1e-6 && (area - b.area()).abs() <= b.area() * 1e-6).then_some(b)
}

#[cfg(test)]
thread_local! {
    /// Tests: how many times [`auto_height`] laid out text in its columns on this thread.
    pub(crate) static COLUMN_LAYOUTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Tests: when set, [`auto_height`] lays out the columns at every step of its bisection on
    /// this thread.
    pub(crate) static FULL_SEARCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Auto Size: move the bottom of a rectangular area type frame to just below its last line (plus
/// the inset), so the frame's height follows the text. Text in several columns gets the least
/// height (to 0.01 pt) at which it fits. True when the frame changed.
fn auto_height(t: &mut TextObject) -> bool {
    use vectorcraft_geom::Shape;
    let Some(b) = auto_height_frame(t) else { return false };
    let db = vectorcraft_text::FontDb::global();
    // Lay out in a frame as tall as the canvas allows: everything the width lets through flows.
    let tall = (crate::MAX_COORD - b.y0).min(1.0e5);
    if tall <= 1.0 {
        return false;
    }
    let mut probe = t.clone();
    let with_size = |probe: &mut TextObject, w: f64, h: f64| {
        if let vectorcraft_doc::TextKind::Area { frame } = &mut probe.kind {
            *frame = PathData::from_bezpath(&vectorcraft_geom::Rect::new(b.x0, b.y0, b.x0 + w, b.y0 + h).to_path(0.1));
        }
    };
    // Columns: all the text in one column as wide as each of them is surely tall enough.
    let cols = t.area.columns.max(1);
    let col_w = if cols > 1 { ((b.width() - t.area.gutter.max(0.0) * (cols - 1) as f64) / cols as f64).max(1.0) } else { b.width() };
    with_size(&mut probe, col_w, tall);
    probe.area.columns = 1;
    // Measured from the top: aligned in the tall probe the lines would sit far below.
    probe.area.vertical_align = vectorcraft_doc::VerticalAlign::Top;
    let lay = vectorcraft_text::layout(db, &probe);
    let inset = t.area.inset.max(0.0);
    let Some(bottom) = lay.lines.iter().map(|l| l.baseline + l.descent).filter(|y| y.is_finite()).reduce(f64::max) else { return false };
    // The layout's fit test is `baseline + descent <= bottom + 0.01`: the last line fits exactly.
    let mut h = (bottom + inset - b.y0).clamp(1.0, tall);
    if cols > 1 {
        // The least height (to 0.01 pt) at which the columns hold the text.
        probe.area.columns = cols;
        let fits = |probe: &mut TextObject, h: f64| {
            #[cfg(test)]
            COLUMN_LAYOUTS.with(|c| c.set(c.get() + 1));
            with_size(probe, b.width(), h);
            !vectorcraft_text::layout(db, probe).overflow
        };
        // The bisection runs on the one column's lines as `columns_hold` places them in the columns.
        // The columns are then laid out at the upper height it ends on and, when they hold the text
        // there, at the lower one, unless that is the 1 pt it starts from. When the one column
        // overflows, when text wraps around other objects, or when the columns don't hold the text
        // at the upper height or do hold it at the lower one, the columns are laid out at every step
        // of the bisection.
        let quick = !lay.overflow && t.wrap.is_empty();
        #[cfg(test)]
        let quick = quick && !FULL_SEARCH.with(std::cell::Cell::get);
        let ends = quick.then(|| least_height(1.0, h, |x| columns_hold(&lay.lines, cols, b.y0, inset, x)));
        h = match ends {
            Some((lo, hi)) if fits(&mut probe, hi) && (lo <= 1.0 || !fits(&mut probe, lo)) => hi,
            _ => {
                if !fits(&mut probe, h) {
                    return false;
                }
                least_height(1.0, h, |x| fits(&mut probe, x)).1
            }
        };
    }
    if (h - b.height()).abs() <= 1e-6 {
        return false;
    }
    // Scale the frame about its top edge, keeping its anchors (and their order) as they are.
    let a = Affine::translate((0.0, b.y0)) * Affine::scale_non_uniform(1.0, h / b.height()) * Affine::translate((0.0, -b.y0));
    let vectorcraft_doc::TextKind::Area { frame } = &mut t.kind else { return false };
    let mut next = frame.clone();
    next.transform(a);
    if next.bounds().is_none_or(|nb| ![nb.x0, nb.y0, nb.x1, nb.y1].iter().all(|v| v.is_finite() && v.abs() <= crate::MAX_COORD)) {
        return false;
    }
    *frame = next;
    true
}

/// The bisection [`auto_height`] runs for text in columns, from `lo` (taken not to fit) and `hi`
/// (where it fits): at most 24 halvings, until they are at most 0.01 pt apart → (lo, hi).
fn least_height(mut lo: f64, mut hi: f64, mut fits: impl FnMut(f64) -> bool) -> (f64, f64) {
    for _ in 0..24 {
        if hi - lo <= 0.01 {
            break;
        }
        let mid = (lo + hi) * 0.5;
        if fits(mid) { hi = mid } else { lo = mid }
    }
    (lo, hi)
}

/// Do `cols` columns of a frame `h` tall (its top at `y0`, the text `inset` inside it) hold
/// `lines`, the text's lines laid out in one column as wide as each of them from the same top?
/// Each column takes the next lines while they end within its bottom, as layout tests them
/// (`baseline + descent <= bottom + 0.01`). A column's first line sits its
/// [`first_baseline`](vectorcraft_text::LineInfo::first_baseline) below `y0 + inset`, and the
/// lines after it keep their distances from it. A last line with no characters (after a return
/// that ends the text) may be left out, as layout leaves it out without overflowing.
fn columns_hold(lines: &[vectorcraft_text::LineInfo], cols: usize, y0: f64, inset: f64, h: f64) -> bool {
    let Some(first) = lines.first() else { return true };
    let bottom = y0 + h - inset + 0.01;
    let mut next = 0;
    for _ in 0..cols {
        let Some(start) = lines.get(next) else { break };
        let up = (start.baseline - start.first_baseline) - (first.baseline - first.first_baseline);
        next += lines.iter().skip(next).take_while(|l| l.baseline + l.descent - up <= bottom).count();
    }
    next >= lines.len() || (next + 1 == lines.len() && lines.last().is_some_and(|l| l.start == l.end))
}

/// Recompute the layout bounds cache for the text among `ids` (and their descendants) that lacks
/// it. Pasted or imported type arrives without the cache (not serialized), so selection boxes, the
/// Transform panel and hit testing would fall back to [`TextObject::estimate_bounds`] (a rough
/// 0.55 em per character) until the text is edited — the box comes up short and alignment looks off.
pub(crate) fn refresh_bounds_of(d: &mut Document, ids: &[NodeId]) {
    for id in text_ids(d, ids) {
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind)
            && t.cached_bounds.is_none()
        {
            refresh_bounds(t);
        }
    }
}

/// `result` with `overflow: [id…]`: the area type and type on a path among `ids` whose text
/// doesn't fit (the red overflow "+"; its lines past the frame aren't drawn). Left out when all of
/// it fits. The commands that make type or change what it holds answer with it, so the call that
/// pushed text out of its frame says so itself.
pub(crate) fn with_overflow(s: &Session, ids: &[NodeId], mut result: Value) -> Value {
    let Ok(st) = s.doc() else { return result };
    let db = vectorcraft_text::FontDb::global();
    let fits = |t: &TextObject| matches!(t.kind, vectorcraft_doc::TextKind::Point) || !vectorcraft_text::layout(db, t).overflow;
    let over = |id: &&NodeId| matches!(st.doc.node(**id).map(|n| &n.kind), Some(NodeKind::Text(t)) if !fits(t));
    let over: Vec<u64> = ids.iter().filter(over).map(|i| i.0).collect();
    if !over.is_empty() {
        result["overflow"] = json!(over);
    }
    result
}

/// Adds `bounds` to the result of a command that made the type object `id`: the same
/// `{x, y, width, height}` box `document.inspect` reports for it (after `justify`, so a script can
/// place the new type without a helper object); `null` when it has none.
pub(crate) fn with_bounds(s: &Session, id: Option<NodeId>, mut result: Value) -> Value {
    let Ok(st) = s.doc() else { return result };
    let symbols = st.doc.symbol_boxes();
    let b = id.and_then(|i| st.doc.node(i)).and_then(|n| n.geometric_bounds_with(&symbols));
    result["bounds"] = crate::inspect::rect_json(b);
    result
}

/// The warning that Print gives for `ids`, area type and type on a path whose text doesn't fit
/// (as [`with_overflow`] finds it): the text past the end of their frames doesn't print. It names
/// the first 20 ids and counts the rest; `None` when `ids` is empty.
pub(crate) fn overflow_warning(ids: &[NodeId]) -> Option<String> {
    overflow_note(ids, "doesn't print")
}

/// [`overflow_warning`] as an export that draws type gives it: the text past the end of the frames
/// isn't drawn in the export.
pub(crate) fn export_overflow_warning(ids: &[NodeId]) -> Option<String> {
    overflow_note(ids, "isn't drawn in the export")
}

/// [`overflow_warning`], saying that the text past the end of the frames `left_out`.
fn overflow_note(ids: &[NodeId], left_out: &str) -> Option<String> {
    const NAMED: usize = 20;
    let (named, rest) = ids.split_at(ids.len().min(NAMED));
    let mut list = named.iter().map(|i| i.0.to_string()).collect::<Vec<_>>().join(", ");
    if !rest.is_empty() {
        list = format!("{list} and {} more", rest.len());
    }
    match ids {
        [] => None,
        [_] => Some(format!("1 text object doesn't fit its frame: the text past the end of the frame {left_out} (id {list})")),
        _ => Some(format!("{} text objects don't fit their frames: the text past the end of each frame {left_out} (ids {list})", ids.len())),
    }
}

/// Text objects among `ids` and their descendants.
fn text_ids(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for id in ids {
        if let Some(n) = d.node(*id) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::Text(_)) && !out.contains(&c.id) {
                    out.push(c.id);
                }
            });
        }
    }
    out
}

fn scale_text_attributes_enabled(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
    let ids = text_ids(&st.doc, &st.selection.objects);
    if ids.iter().any(|id| {
        st.doc.node(*id).is_some_and(|n| {
            let NodeKind::Text(t) = &n.kind else { return false };
            t.style_scale().is_some_and(|sc| (sc.points - 1.0).abs() > 1e-12 || (sc.horizontal - 1.0).abs() > 1e-12)
        })
    }) {
        Ok(())
    } else {
        Err("Scale Text Attributes: selected type has no scale to bake".into())
    }
}

fn scale_text_attributes(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.scaleTextAttributes";
    let ids = text_targets(s, p, C)?;
    let changed = s.edit("Scale Text Attributes", |d, _| {
        let mut n = 0usize;
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|node| &mut node.kind) else { continue };
            if t.scale_text_attributes() {
                refresh_bounds(t);
                n += 1;
            }
        }
        if n == 0 {
            return Err(EngineError::Other("Scale Text Attributes: selected type has no scale to bake".into()));
        }
        Ok(n)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "changed": changed }))
}

fn create_outlines(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let ids = s.edit("Create Outlines", |d, sel| {
        let texts = text_ids(d, &roots);
        if texts.is_empty() {
            return Err(EngineError::Other("Create Outlines: select text objects".into()));
        }
        let mut new_sel: Vec<NodeId> = roots.iter().copied().filter(|r| !texts.contains(r)).collect();
        let mut out = vec![];
        let threaded = super::threads::frames(d);
        for tid in texts {
            let Some(n) = d.node(tid).cloned() else { continue };
            let NodeKind::Text(t) = &n.kind else { continue };
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let mut children = vec![];
            // Where each run's glyphs end among `children`: the canvas paints a run's ruby with the
            // run, before the runs after it, so its outlines go there too.
            let mut run_end: std::collections::BTreeMap<usize, usize> = Default::default();
            for (gi, g) in lay.glyphs.iter().enumerate() {
                'glyph: {
                    // Inline graphics become instances of their symbols, in place.
                    if let Some(ig) = lay.inlines.iter().find(|i| i.glyph == gi)
                        && let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref())
                    {
                        let id = d.alloc_id();
                        let xf = t.xf * ig.xf * d.symbol_natural_xf(&art.symbol);
                        children.push(Arc::new(Node::new(id, NodeKind::SymbolInstance { symbol: art.symbol.clone(), xf })));
                        break 'glyph;
                    }
                    // Colour bitmap glyphs (emoji) become embedded images, in place, their pixels
                    // taken into the document: glyphs that show the same image share them.
                    if let Some(image) = g.placed_image_node(t.xf) {
                        children.push(Arc::new(d.reid(&image)));
                        break 'glyph;
                    }
                    // COLR glyphs become groups of their layers, in place, with ids of the document's.
                    if let Some(art) = g.placed_colour(t.xf) {
                        children.push(Arc::new(d.reid(&art)));
                        break 'glyph;
                    }
                    let path = PathData::from_bezpath(&g.outline).transformed(t.xf);
                    if path.is_empty() {
                        break 'glyph;
                    }
                    let st = t.runs.get(g.run).map(|r| r.style.clone()).unwrap_or_else(|| t.first_style());
                    let mut node = shape_node(d, path, None);
                    node.appearance = st.appearance().into();
                    children.push(Arc::new(node));
                }
                run_end.insert(g.run, children.len());
            }
            // Ruby (furigana) glyphs, where the layout put them, in their base text's run's paint,
            // right after that run's glyphs, as the canvas draws them (colour glyphs in ruby draw
            // nothing there either).
            let mut ruby: Vec<(usize, Arc<Node>)> = vec![];
            for g in &lay.ruby {
                let path = PathData::from_bezpath(&g.outline).transformed(t.xf);
                if path.is_empty() {
                    continue;
                }
                let st = t.runs.get(g.run).map(|r| r.style.clone()).unwrap_or_else(|| t.first_style());
                let mut node = shape_node(d, path, None);
                node.appearance = st.appearance().into();
                // After the glyphs of its run and of the runs before it.
                let at = run_end.range(..=g.run).map(|(_, end)| *end).max().unwrap_or(0);
                ruby.push((at, Arc::new(node)));
            }
            ruby.sort_by_key(|(at, _)| *at);
            let mut ruby = ruby.into_iter().peekable();
            let mut placed = Vec::with_capacity(children.len() + ruby.len());
            for (i, c) in children.into_iter().enumerate() {
                while let Some((_, r)) = ruby.next_if(|(at, _)| *at == i) {
                    placed.push(r);
                }
                placed.push(c);
            }
            placed.extend(ruby.map(|(_, r)| r));
            let mut children = placed;
            // Underline and strikethrough bars become paths of their run's paint too (#847).
            for (run, bar) in vectorcraft_text::decorations(&lay, vectorcraft_text::FontDb::global(), t) {
                let st = t.runs.get(run).map(|r| r.style.clone()).unwrap_or_else(|| t.first_style());
                let mut node = shape_node(d, PathData::from_bezpath(&bar).transformed(t.xf), None);
                node.appearance = st.appearance().into();
                children.push(Arc::new(node));
            }
            let (par, idx, _) = d.position(tid).ok_or(EngineError::NoNode(tid))?;
            d.remove(tid)?;
            if children.is_empty() {
                continue;
            }
            let gid = d.alloc_id();
            let mut g = Node::group(gid, children);
            g.opacity = n.opacity;
            g.blend = n.blend;
            g.name = n.name.clone();
            d.insert(par, idx, g)?;
            out.push(gid);
            if roots.contains(&tid) {
                new_sel.push(gid);
            }
        }
        // Outlined frames of threaded text leave their threads with the text they show.
        super::threads::leave_threads(d, &threaded);
        sel.set(new_sel);
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

pub(crate) fn text_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let ids = targets(s, p)?;
    let t = text_ids(&s.doc()?.doc, &ids);
    if t.is_empty() {
        return Err(bad(cmd, "no text objects selected"));
    }
    Ok(t)
}

/// Clamp in the units shown to the user, then compensate the text's affine. Clamping
/// the local result would make small sizes impossible after enlarging an object.
pub(crate) fn local_type_value(t: &TextObject, value: f64, limits: (f64, f64), horizontal: bool, cmd: &str) -> Result<f64> {
    if !value.is_finite() {
        return Err(bad(cmd, "type dimension must be finite"));
    }
    let scale = t.style_scale().ok_or_else(|| bad(cmd, "text transform is collapsed or unrepresentable"))?;
    let local = value.clamp(limits.0, limits.1) / if horizontal { scale.horizontal } else { scale.points };
    // Bound local dimensions too: an extreme inverse scale can produce a finite size
    // whose leading, glyph advances or layout bounds overflow before the edit's geometry check.
    if !local.is_finite() || local.abs() > crate::MAX_COORD || (value != 0.0 && local == 0.0) {
        return Err(bad(cmd, "type dimension cannot be represented with this text transform"));
    }
    Ok(local)
}

/// Optional `start`/`end` byte offsets of a text command: the character range it styles and the
/// paragraphs it touches. Without either, the command applies to all the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextRange {
    start: Option<usize>,
    end: Option<usize>,
}

impl TextRange {
    /// `start`/`end` of `p`, if either is given (non-negative integers).
    pub(crate) fn parse(p: &Value, cmd: &str) -> Result<Option<Self>> {
        let get = |k: &str| -> Result<Option<usize>> {
            match p.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_u64().map(|n| Some(index(n))).ok_or_else(|| bad(cmd, format!("`{k}` must be a byte offset"))),
            }
        };
        let (start, end) = (get("start")?, get("end")?);
        Ok((start.is_some() || end.is_some()).then_some(Self { start, end }))
    }
    /// The byte range in `t`, clamped and ordered.
    pub(crate) fn bytes(&self, t: &TextObject) -> (usize, usize) {
        let len = vectorcraft_text::edit::runs_len(&t.runs);
        let a = self.start.unwrap_or(0).min(len);
        let b = self.end.unwrap_or(len).min(len);
        (a.min(b), a.max(b))
    }
    /// The paragraphs of `t` the range touches.
    pub(crate) fn paras(&self, t: &TextObject) -> std::ops::Range<usize> {
        let (a, b) = self.bytes(t);
        t.paragraphs_in(a, b)
    }
}

/// Paragraphs of `t` an optional range touches (None: every paragraph).
pub(crate) fn para_span(range: Option<TextRange>, t: &TextObject) -> Option<std::ops::Range<usize>> {
    range.map(|r| r.paras(t))
}

/// Apply `f` to the character styles of `range` of `t` (None: every run).
pub(crate) fn style_chars(t: &mut TextObject, range: Option<TextRange>, mut f: impl FnMut(&mut vectorcraft_doc::CharStyle)) {
    match range {
        None => t.runs.iter_mut().for_each(|r| f(&mut r.style)),
        Some(r) => {
            let (a, b) = r.bytes(t);
            if a < b {
                vectorcraft_text::edit::style_range(&mut t.runs, a, b, f);
            }
        }
    }
}

fn set_text(s: &mut Session, p: &Value) -> Result<Value> {
    let text = str_param(p, "text").ok_or_else(|| bad("text.setText", "missing `text`"))?.to_string();
    let ids = text_targets(s, p, "text.setText")?;
    s.edit("Typing", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let style = t.first_style();
            // Every paragraph of the new text takes the first paragraph's attributes.
            t.splice_paras(0, vectorcraft_text::edit::runs_len(&t.runs), &text);
            t.runs = vec![vectorcraft_doc::TextRun { text: text.clone(), style, inline: None }];
            // The text is new: the ruby of the old one is gone.
            t.ruby.clear();
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(with_overflow(s, &ids, json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })))
}

pub(crate) fn justify_param(v: &str) -> Option<Justify> {
    Some(match v.to_ascii_lowercase().as_str() {
        "auto" => Justify::Auto,
        "left" => Justify::Left,
        "center" => Justify::Center,
        "right" => Justify::Right,
        "justifyleft" => Justify::JustifyLeft,
        "justifycenter" => Justify::JustifyCenter,
        "justifyright" => Justify::JustifyRight,
        "justifyall" | "justify" => Justify::JustifyAll,
        _ => return None,
    })
}

fn set_style(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.setStyle";
    let font = str_param(p, "font").map(str::to_string);
    let style = str_param(p, "style").map(str::to_string);
    let size = len_param(p, "size");
    if size.is_some_and(|v| v <= 0.0) {
        return Err(bad(C, "size must be positive"));
    }
    let leading = match p.get("leading") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) if a.eq_ignore_ascii_case("auto") => Some(None),
        Some(_) => Some(Some(len_param(p, "leading").ok_or_else(|| bad(C, "leading must be a number or \"auto\""))?.clamp(0.1, 5000.0))),
    };
    let tracking = num_param(p, "tracking");
    let justify = match str_param(p, "justify") {
        Some(j) => Some(justify_param(j).ok_or_else(|| bad(C, "justify must be auto|left|center|right|justifyAll"))?),
        None => None,
    };
    super::paint::check_keep_model(p, C)?;
    let fill = match p.get("fill") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) if n.eq_ignore_ascii_case("none") => Some(vectorcraft_color::Paint::None),
        Some(v) => Some(vectorcraft_color::Paint::solid(super::paint::new_color(s, p, color_value(v).ok_or_else(|| bad(C, "bad fill colour"))?))),
    };
    let features = super::textedit::features_param(p, C)?;
    if font.is_none()
        && style.is_none()
        && size.is_none()
        && leading.is_none()
        && tracking.is_none()
        && justify.is_none()
        && fill.is_none()
        && features.is_none()
    {
        return Err(bad(C, "nothing to change"));
    }
    let range = TextRange::parse(p, C)?;
    let ids = text_targets(s, p, C)?;
    let protect = s.prefs.missing_glyph_protection && (font.is_some() || style.is_some());
    s.edit("Character", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let size = size.map(|v| local_type_value(t, v, (0.1, 1296.0), false, C)).transpose()?;
            let leading = leading.map(|v| v.map(|v| local_type_value(t, v, (0.1, 5000.0), false, C)).transpose()).transpose()?;
            let before = protect.then(|| t.runs.clone());
            style_chars(t, range, |st| {
                if let Some(f) = &font {
                    st.font_family = f.clone();
                    // Keep the style when the new family has it, else its closest match (#4128).
                    if style.is_none() {
                        super::textedit::fit_font_style(st);
                    }
                }
                if let Some(f) = &style {
                    st.font_style = f.clone();
                }
                // Another font: none of the old one's versions or Variable Font settings.
                if font.is_some() || style.is_some() {
                    st.font_version = None;
                    st.font_axes.clear();
                }
                if let Some(v) = size {
                    st.size = v;
                }
                if let Some(l) = leading {
                    st.leading = l;
                }
                if let Some(v) = tracking {
                    st.tracking = v.clamp(-1000.0, 10000.0);
                }
                if let Some(f) = &fill {
                    st.fill = f.clone();
                }
                if let Some(f) = &features {
                    st.features = f.clone();
                }
            });
            if let Some(before) = before {
                super::textedit::protect_missing_glyphs(&before, &mut t.runs);
            }
            if let Some(j) = justify {
                let span = para_span(range, t);
                t.edit_paras(span, |pa| pa.justify = j);
            }
            refresh_bounds(t);
        }
        // Graph labels keep a change made to all of their text through the graph being drawn again (sizes as given,
        // not in a scaled label's own units).
        if range.is_none() {
            super::graph::capture_label_styles(d, &ids, &|st| {
                if let Some(f) = &font {
                    st.font_family = f.clone();
                    st.font_version = None;
                    st.font_axes.clear();
                    if style.is_none() {
                        super::textedit::fit_font_style(st);
                    }
                }
                if let Some(f) = &style {
                    st.font_style = f.clone();
                    st.font_version = None;
                    st.font_axes.clear();
                }
                if let Some(v) = size {
                    st.size = v.clamp(0.1, 1296.0);
                }
                if let Some(l) = leading {
                    st.leading = l;
                }
                if let Some(v) = tracking {
                    st.tracking = v.clamp(-1000.0, 10000.0);
                }
                if let Some(f) = &fill {
                    st.fill = f.clone();
                }
                if let Some(f) = &features {
                    st.features = f.clone();
                }
            });
        }
        Ok(())
    })?;
    Ok(with_overflow(s, &ids, json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })))
}

/// Reshape area type's frame with `f` ([`TextObject::transform_area`],
/// [`TextObject::move_area_anchors`]) and lay its text out again. False, changing nothing, when
/// `f` fails or the frame would reach past [`crate::MAX_COORD`] (the text flows over its height).
pub(crate) fn reshape_area_with(t: &mut TextObject, f: impl FnOnce(&mut TextObject) -> bool) -> bool {
    let before = t.kind.clone();
    let within = |t: &TextObject| match &t.kind {
        vectorcraft_doc::TextKind::Area { frame } => {
            frame.bounds().is_some_and(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.abs() <= crate::MAX_COORD))
        }
        _ => false,
    };
    let height = |t: &TextObject| auto_height_frame(t).map(|b| b.height());
    let was = height(t);
    if !(f(t) && within(t)) {
        t.kind = before;
        return false;
    }
    // Setting the height by hand turns Auto Size off (as in Illustrator); a new width keeps it.
    if let (Some(a), Some(b)) = (was, height(t))
        && (a - b).abs() > 1e-6
    {
        t.area.fit = vectorcraft_doc::AreaFit::None;
    }
    refresh_bounds(t);
    true
}

/// Area type's text, if `n` is area type.
fn area_text(n: Option<&Node>) -> Option<&TextObject> {
    match n.map(|n| &n.kind) {
        Some(NodeKind::Text(t)) if matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }) => Some(t),
        _ => None,
    }
}

/// The width and height of area type's frame in points, along the type's own axes, and the frame's
/// bounds in text space.
fn area_size(t: &TextObject) -> Option<(f64, f64, vectorcraft_geom::Rect)> {
    let vectorcraft_doc::TextKind::Area { frame } = &t.kind else { return None };
    let b = frame.bounds()?;
    let [a, bb, c, d, _, _] = t.xf.as_coeffs();
    Some((b.width() * a.hypot(bb), b.height() * c.hypot(d), b))
}

/// Size area type's frame to `w` × `h` points (None: keep that side) from its top-left corner,
/// and lay its text out again. False when the frame doesn't change.
fn size_area(t: &mut TextObject, w: Option<f64>, h: Option<f64>) -> bool {
    let Some((cw, ch, b)) = area_size(t) else { return false };
    let factor = |to: Option<f64>, cur: f64| match to {
        Some(to) if cur > 1e-9 && (to - cur).abs() > 1e-9 => to / cur,
        _ => 1.0,
    };
    let (sx, sy) = (factor(w, cw), factor(h, ch));
    if sx == 1.0 && sy == 1.0 {
        return false;
    }
    let o = Affine::translate(b.origin().to_vec2());
    let a = t.xf * o * Affine::scale_non_uniform(sx, sy) * o.inverse() * t.xf.inverse();
    reshape_area_with(t, |t| t.transform_area(a))
}

/// The fit `p` asks for (`fit`: none|autoHeight|shrinkText or `{"shrinkText": {"minPercent"}}`,
/// `fitMinPercent`), starting from `cur`. None when `p` sets neither (or only `fitMinPercent`
/// while the fit isn't Shrink Text).
pub(crate) fn fit_param(p: &Value, cur: AreaFit, c: &str) -> Result<Option<AreaFit>> {
    check_numbers(p, &["fitMinPercent"], c)?;
    let min = p.get("fitMinPercent").and_then(Value::as_f64);
    let cur_min = match cur {
        AreaFit::ShrinkText { min_percent } => Some(min_percent),
        _ => None,
    };
    let fit = match p.get("fit") {
        None | Some(Value::Null) => match (cur, min) {
            (AreaFit::ShrinkText { .. }, Some(m)) => AreaFit::ShrinkText { min_percent: m },
            _ => return Ok(None),
        },
        Some(Value::String(id)) => {
            AreaFit::parse(id, min.or(cur_min)).ok_or_else(|| bad(c, format!("fit must be none|autoHeight|shrinkText, got {id}")))?
        }
        Some(v) => match serde_json::from_value::<AreaFit>(v.clone()).map_err(|e| bad(c, format!("bad fit: {e}")))? {
            AreaFit::ShrinkText { min_percent } => AreaFit::ShrinkText { min_percent: min.unwrap_or(min_percent) },
            f => f,
        },
    };
    Ok(Some(match fit {
        AreaFit::ShrinkText { min_percent } => AreaFit::ShrinkText { min_percent: AreaFit::clamp_percent(min_percent) },
        f => f,
    }))
}

/// Area type's options as `text.areaOptions` reports them: the Area Type Options with `fit` as
/// its id and `fitMinPercent` beside it, the frame's `width` and `height`, and whether the text
/// `overflow`s its frame at `fitScale` (Shrink Text to Fit's factor; 1 unshrunk).
pub(crate) fn area_options_json(t: &TextObject) -> Result<Value> {
    let mut v = serde_json::to_value(&t.area).map_err(|e| EngineError::Other(e.to_string()))?;
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    if let Some(o) = v.as_object_mut() {
        o.insert("fit".into(), json!(t.area.fit.id()));
        let min = match t.area.fit {
            AreaFit::ShrinkText { min_percent } => min_percent,
            _ => AreaFit::DEFAULT_MIN_PERCENT,
        };
        o.insert("fitMinPercent".into(), json!(min));
        if let Some((w, h, _)) = area_size(t) {
            o.insert("width".into(), json!(w));
            o.insert("height".into(), json!(h));
        }
        o.insert("overflow".into(), json!(lay.overflow));
        o.insert("fitScale".into(), json!(lay.fit_scale));
    }
    Ok(v)
}

/// The [`vectorcraft_doc::AreaOptions`] keys `text.areaOptions` merges as they are.
const AREA_KEYS: [&str; 7] = ["rows", "columns", "gutter", "inset", "firstBaseline", "firstBaselineMin", "verticalAlign"];

fn area_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.areaOptions";
    check_numbers(p, &["width", "height"], C)?;
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        text_targets(s, p, C)?.into_iter().filter(|i| area_text(d.node(*i)).is_some()).collect()
    };
    let first = *ids.first().ok_or_else(|| bad(C, "select area type (text in a frame)"))?;
    let options = |s: &Session| -> Result<Value> {
        let t = area_text(s.doc()?.doc.node(first)).ok_or_else(|| bad(C, "select area type (text in a frame)"))?;
        area_options_json(t)
    };
    let cur = area_text(s.doc()?.doc.node(first)).map(|t| t.area.clone()).unwrap_or_default();
    let fit = fit_param(p, cur.fit, C)?;
    let mut v = serde_json::to_value(&cur).map_err(|e| EngineError::Other(e.to_string()))?;
    let size = |k: &str| p.get(k).and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| x.clamp(1.0, 100_000.0));
    let (w, mut h) = (size("width"), size("height"));
    let mut changed = w.is_some() || h.is_some() || fit.is_some();
    if let (Some(o), Some(src)) = (v.as_object_mut(), p.as_object()) {
        for (k, val) in src.iter().filter(|(k, _)| AREA_KEYS.contains(&k.as_str())) {
            o.insert(k.clone(), val.clone());
            changed = true;
        }
    }
    if !changed {
        return options(s);
    }
    let mut opts: vectorcraft_doc::AreaOptions = serde_json::from_value(v).map_err(|e| bad(C, e.to_string()))?;
    opts.rows = opts.rows.clamp(1, vectorcraft_doc::AreaOptions::MAX_ROWS_COLUMNS);
    opts.columns = opts.columns.clamp(1, vectorcraft_doc::AreaOptions::MAX_ROWS_COLUMNS);
    let finite = |x: f64| if x.is_finite() { x } else { 0.0 };
    opts.gutter = finite(opts.gutter).clamp(0.0, 10_000.0);
    opts.inset = finite(opts.inset).clamp(0.0, 10_000.0);
    opts.first_baseline_min = finite(opts.first_baseline_min).clamp(0.0, 10_000.0);
    opts.fit = fit.unwrap_or(cur.fit);
    if opts.fit == AreaFit::AutoHeight {
        // Auto Size sets the height (Illustrator's dialog turns the Height field off).
        h = None;
    }
    s.edit("Area Type Options", |d, _| {
        for id in &ids {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                t.area = opts.clone();
                // A resized frame has laid its text out again; otherwise the new options do here.
                if !size_area(t, w, h) {
                    refresh_bounds(t);
                }
            }
        }
        Ok(())
    })?;
    options(s)
}

fn reshape_area(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.reshapeArea";
    // One area type (`id`, `anchors`), or several (`items`: [{id, anchors}…]).
    let raw: Vec<&Value> = match p.get("items") {
        Some(items) => {
            if p.get("id").is_some() || p.get("anchors").is_some() {
                return Err(bad(C, "give `id` and `anchors`, or `items`, not both"));
            }
            let items = items.as_array().ok_or_else(|| bad(C, "`items` must be an array of {id, anchors}"))?;
            if items.is_empty() {
                return Err(bad(C, "`items` is empty"));
            }
            items.iter().collect()
        }
        None => vec![p],
    };
    let mut items = Vec::with_capacity(raw.len());
    for it in raw {
        let id = id_param(it, "id").ok_or_else(|| bad(C, "missing id"))?;
        let mut refs = super::select::checked_refs(it.get("anchors"), C)?;
        refs.sort_unstable();
        refs.dedup();
        if refs.is_empty() {
            return Err(bad(C, "missing anchors [[subpath, anchor]…]"));
        }
        if items.iter().any(|(other, _)| *other == id) {
            return Err(bad(C, format!("area type {} is named twice", id.0)));
        }
        let node = s.doc()?.doc.node(id);
        if area_text(node).is_none() || node.is_some_and(|n| n.extras().perspective.is_some()) {
            return Err(bad(C, "not area type (text in a frame) outside perspective"));
        }
        items.push((id, refs));
    }
    let d = Vec2::new(f64_req(p, "dx", C)?, f64_req(p, "dy", C)?);
    s.edit("Reshape Type Area", |doc, _| {
        for (id, refs) in &items {
            let Some(NodeKind::Text(t)) = doc.node_mut(*id).map(|n| &mut n.kind) else { return Err(EngineError::NoNode(*id)) };
            if !reshape_area_with(t, |t| t.move_area_anchors(refs, d)) {
                return Err(bad(C, "no such frame anchor, or the frame would reach past the canvas"));
            }
        }
        Ok(())
    })?;
    ok()
}

fn reshape_path(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.reshapePath";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let mut refs = super::select::checked_refs(p.get("anchors"), C)?;
    refs.sort_unstable();
    refs.dedup();
    if refs.is_empty() {
        return Err(bad(C, "missing anchors [[subpath, anchor]…]"));
    }
    let d = Vec2::new(f64_req(p, "dx", C)?, f64_req(p, "dy", C)?);
    let node = s.doc()?.doc.node(id);
    let on_path = matches!(node.map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, vectorcraft_doc::TextKind::OnPath { .. }));
    if !on_path || node.is_some_and(|n| n.extras().perspective.is_some()) {
        return Err(bad(C, "not type on a path outside perspective"));
    }
    s.edit("Reshape Type Path", |doc, _| {
        let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) else { return Err(EngineError::NoNode(id)) };
        let before = t.kind.clone();
        let within = |t: &TextObject| {
            t.type_path().and_then(|p| p.bounds()).is_some_and(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.abs() <= crate::MAX_COORD))
        };
        if !(t.move_path_anchors(&refs, d) && within(t)) {
            t.kind = before;
            return Err(bad(C, "no such path anchor, or the path would reach past the canvas"));
        }
        refresh_bounds(t);
        Ok(())
    })?;
    ok()
}

#[cfg(test)]
mod area_tests;

/// Width of the first line and of the space it can fill, with `tracking` on the first paragraph.
/// The third value is where the first line ends in the plain text.
fn headline_fit(t: &TextObject, tracking: f64) -> Option<(f64, f64, usize)> {
    let mut probe = t.clone();
    let para_end = probe.plain_text().find('\n').unwrap_or(usize::MAX);
    vectorcraft_text::edit::style_range(&mut probe.runs, 0, para_end.min(vectorcraft_text::edit::runs_len(&t.runs)), |st| st.tracking = tracking);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &probe);
    let cell = lay.frames.first()?;
    let line = lay.lines.first()?;
    let para = t.para_at(0);
    let avail = cell.width() - 2.0 * t.area.inset - para.left_indent - para.right_indent;
    Some((line.x1 - line.x0, avail, line.end))
}

fn fit_headline(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.fitHeadline";
    let ids = text_targets(s, p, C)?;
    let d = &s.doc()?.doc;
    let mut plans = vec![];
    for id in &ids {
        let Some(NodeKind::Text(t)) = d.node(*id).map(|n| &n.kind) else { continue };
        if !matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }) {
            continue;
        }
        // Largest tracking that keeps the first paragraph on one line (bisection). The layout
        // drops lines that overflow the frame, so a frame one line tall hides the wrapped rest:
        // "one line" means the first line reaches the end of the first paragraph.
        let para_end = t.plain_text().find('\n').unwrap_or(t.plain_text().len());
        let one_line = |tr: f64| headline_fit(t, tr).is_some_and(|(_, _, end)| end >= para_end);
        let (mut lo, mut hi) = (-200.0, 2000.0);
        if !one_line(lo) {
            continue;
        }
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if one_line(mid) { lo = mid } else { hi = mid }
        }
        plans.push((*id, (lo * 10.0).floor() / 10.0));
    }
    let first = plans.first().map(|p| p.1).ok_or_else(|| bad(C, "select area type whose first line can fit its frame"))?;
    s.edit("Fit Headline", |d, _| {
        for (id, tr) in &plans {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                let end = t.plain_text().find('\n').unwrap_or(usize::MAX).min(vectorcraft_text::edit::runs_len(&t.runs));
                vectorcraft_text::edit::style_range(&mut t.runs, 0, end, |st| st.tracking = *tr);
                refresh_bounds(t);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "tracking": first }))
}

#[cfg(test)]
mod headline_tests {
    use super::*;

    #[test]
    fn fit_headline_fills_the_first_line() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s
            .execute("text.create", &json!({"x": 10, "y": 10, "text": "HEADLINE\nbody text", "size": 24, "area": {"width": 300, "height": 120}}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let tr = s.execute("text.fitHeadline", &json!({})).unwrap()["tracking"].as_f64().unwrap();
        assert!(tr > 100.0, "a short word spreads out: {tr}");
        let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!() };
        let (w, avail, end) = headline_fit(t, tr).unwrap();
        assert_eq!(end, 8);
        assert!(avail - w < 30.0, "{w} of {avail}");
        assert_eq!(t.runs.last().unwrap().style.tracking, 0.0, "the body keeps its tracking");
    }

    #[test]
    fn fit_headline_fills_even_when_the_frame_holds_one_line() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s
            .execute("text.create", &json!({"x": 10, "y": 10, "text": "NEON DRIVE", "size": 36, "area": {"width": 240, "height": 50}}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let tr = s.execute("text.fitHeadline", &json!({})).unwrap()["tracking"].as_f64().unwrap();
        assert!(tr < 1500.0, "a frame one line tall must not push the tracking to the ceiling: {tr}");
        let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!() };
        let (w, avail, end) = headline_fit(t, tr).unwrap();
        assert_eq!(end, 10);
        assert!(avail - w < 5.0, "{w} of {avail}");
    }
}

#[cfg(test)]
mod ruby_tests {
    use super::*;

    fn session_with_text() -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "漢字です", "size": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        (s, id)
    }

    fn text_of(s: &Session, id: u64) -> &TextObject {
        let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!("text object") };
        t
    }

    #[test]
    fn ruby_command_sets_reports_removes_and_survives_undo() {
        let (mut s, id) = session_with_text();
        let r = s.execute("type.ruby", &json!({"text": "かんじ", "start": 0, "end": 6})).unwrap();
        assert_eq!(r["ruby"][0]["start"], 0);
        assert_eq!(r["ruby"][0]["end"], 6);
        assert_eq!(r["ruby"][0]["text"], "かんじ");
        let t = text_of(&s, id);
        assert_eq!(t.ruby.len(), 1);
        assert_eq!(t.ruby[0].position, RubyPosition::Above);
        assert_eq!(t.ruby[0].align, RubyAlign::Center);
        assert!(t.ruby[0].overhang);
        assert!(t.cached_bounds.unwrap().height() > 20.0, "bounds cover the ruby");
        // Options, and a new reading over the range replaces the old one.
        s.execute("type.ruby", &json!({"text": "かん", "start": 0, "end": 3, "position": "below", "align": "end", "size": 80, "overhang": false}))
            .unwrap();
        let t = text_of(&s, id);
        assert_eq!(t.ruby.len(), 1);
        assert_eq!(t.ruby[0].position, RubyPosition::Below);
        assert_eq!(t.ruby[0].align, RubyAlign::End);
        assert!((t.ruby[0].size_percent - 80.0).abs() < 1e-9 && !t.ruby[0].overhang);
        // Remove takes the ruby off the range; Undo brings it back.
        let r = s.execute("type.ruby", &json!({"remove": true, "start": 0, "end": 6})).unwrap();
        assert_eq!(r["ruby"].as_array().map(Vec::len), Some(0));
        assert!(text_of(&s, id).ruby.is_empty());
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(text_of(&s, id).ruby.len(), 1);
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(text_of(&s, id).ruby.is_empty());
        // Bad input changes nothing.
        for p in [
            json!({"start": 0, "end": 6}),
            json!({"text": "あ", "size": 0}),
            json!({"text": "あ", "size": 101}),
            json!({"text": "あ", "position": "sideways"}),
            json!({"text": "あ", "align": "lean"}),
        ] {
            assert!(s.execute("type.ruby", &p).is_err(), "{p}");
        }
        assert!(text_of(&s, id).ruby.is_empty());
    }

    #[test]
    fn typed_text_before_a_ruby_moves_it() {
        let (mut s, id) = session_with_text();
        s.execute("type.ruby", &json!({"text": "かんじ", "start": 0, "end": 6})).unwrap();
        s.execute("text.editRange", &json!({"id": id, "start": 0, "end": 0, "insert": "あ"})).unwrap();
        let t = text_of(&s, id);
        assert_eq!((t.ruby[0].start, t.ruby[0].end), (3, 9), "the ruby follows the text");
        assert_eq!(t.plain_text(), "あ漢字です");
        // Setting all the text drops the old ruby (it isn't the same text any more).
        s.execute("text.setText", &json!({"id": id, "text": "新しい"})).unwrap();
        assert!(text_of(&s, id).ruby.is_empty());
    }

    /// Create Outlines keeps the ruby: one path per inked glyph of the base text and of the
    /// reading, each where the layout (and so the canvas) puts it.
    #[test]
    fn create_outlines_keeps_the_ruby_where_it_is_laid_out() {
        use kurbo::Shape;
        let (mut s, id) = session_with_text();
        s.execute("type.ruby", &json!({"text": "かんじ", "start": 0, "end": 6})).unwrap();
        let t = text_of(&s, id).clone();
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        let inked = |gs: &[vectorcraft_text::PositionedGlyph]| -> Vec<kurbo::Rect> {
            gs.iter().filter(|g| !g.outline.elements().is_empty()).map(|g| t.xf.transform_rect_bbox(g.outline.bounding_box())).collect()
        };
        let (base, ruby) = (inked(&lay.glyphs), inked(&lay.ruby));
        assert_eq!(ruby.len(), 3, "the reading's three kana have ink");
        let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let d = &s.doc().unwrap().doc;
        let paths: Vec<kurbo::Rect> = d.node(NodeId(g)).unwrap().children().unwrap().iter().filter_map(|c| c.geometric_bounds()).collect();
        assert_eq!(paths.len(), base.len() + ruby.len(), "a path per glyph, the reading's included: {paths:?}");
        let near = |a: &kurbo::Rect, b: &kurbo::Rect| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 0.05);
        for r in &ruby {
            assert!(paths.iter().any(|p| near(p, r)), "the kana at {r:?} among {paths:?}");
        }
        // Undo brings the type back with its ruby.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(text_of(&s, id).ruby.len(), 1);
    }

    /// The outlines stack as the canvas paints type, run by run (`render`'s `TextGeom::run`): a
    /// run's reading goes right after its base glyphs, below the glyphs of the runs after it. Here
    /// the red 漢's wide reading hangs over the big blue 字.
    #[test]
    fn create_outlines_stacks_each_reading_with_its_run() {
        use kurbo::Shape;
        let (mut s, id) = session_with_text();
        s.execute("text.setText", &json!({"id": id, "text": "漢字"})).unwrap();
        s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 3, "fill": "#ff0000"})).unwrap();
        s.execute("text.setRangeStyle", &json!({"id": id, "start": 3, "end": 6, "size": 60, "fill": "#0000ff"})).unwrap();
        s.execute("type.ruby", &json!({"text": "かんじかんじ", "start": 0, "end": 3, "size": 100})).unwrap();
        let t = text_of(&s, id).clone();
        assert_eq!(t.runs.len(), 2, "{:?}", t.runs);
        let (red, blue) = (t.runs[0].style.fill.clone(), t.runs[1].style.fill.clone());
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        let ink = |gs: &mut dyn Iterator<Item = &vectorcraft_text::PositionedGlyph>| {
            gs.filter(|g| !g.outline.elements().is_empty()).map(|g| g.outline.bounding_box()).reduce(|a, b| a.union(b)).unwrap()
        };
        let (reading, big) = (ink(&mut lay.ruby.iter()), ink(&mut lay.glyphs.iter().filter(|g| g.run == 1)));
        assert!(reading.intersect(big).area() > 0.0, "the reading {reading:?} hangs over 字 {big:?}");
        let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let d = &s.doc().unwrap().doc;
        let paints: Vec<String> = d
            .node(NodeId(g))
            .unwrap()
            .children()
            .unwrap()
            .iter()
            .map(|c| match c.appearance.fill().map(|f| &f.paint) {
                Some(p) if *p == red => "red".into(),
                Some(p) if *p == blue => "blue".into(),
                p => format!("{p:?}"),
            })
            .collect();
        // 漢 and its six kana, then 字 over them.
        let want: Vec<String> = ["red"; 7].into_iter().chain(["blue"]).map(String::from).collect();
        assert_eq!(paints, want);
    }

    /// Ruby is written to SVG and PDF with its text: SVG outlines the type where ruby is present
    /// (a reader's `<text>` can't place the reading), PDF differs when the ruby is there.
    #[test]
    fn ruby_reaches_svg_and_pdf_exports() {
        let (mut s, _) = session_with_text();
        let export = |s: &mut Session, format: &str| -> Vec<u8> {
            let r = s.execute("document.export", &json!({"format": format})).unwrap();
            vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
        };
        s.execute("type.ruby", &json!({"text": "かんじ", "start": 0, "end": 6})).unwrap();
        let svg_ruby = String::from_utf8(export(&mut s, "svg")).unwrap();
        let pdf_ruby = export(&mut s, "pdf");
        assert!(!svg_ruby.contains("<text"), "ruby type is outlined: {svg_ruby}");
        assert!(!pdf_ruby.is_empty());
        s.execute("type.ruby", &json!({"remove": true})).unwrap();
        let svg_plain = String::from_utf8(export(&mut s, "svg")).unwrap();
        let pdf_plain = export(&mut s, "pdf");
        assert!(svg_plain.contains("<text"), "without ruby the type stays live text: {svg_plain}");
        assert_ne!(svg_ruby, svg_plain, "the SVG carries the ruby");
        assert_ne!(pdf_ruby, pdf_plain, "the PDF carries the ruby");
    }

    #[test]
    fn ruby_survives_a_native_round_trip() {
        let (mut s, id) = session_with_text();
        s.execute("type.ruby", &json!({"text": "かんじ", "start": 0, "end": 6, "position": "below", "size": 40})).unwrap();
        let doc = s.doc().unwrap().doc.clone();
        let bytes = vectorcraft_format::save(&doc, false);
        let back = vectorcraft_format::load(&bytes).unwrap();
        let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
        assert_eq!(t.ruby.len(), 1);
        assert_eq!((t.ruby[0].start, t.ruby[0].end), (0, 6));
        assert_eq!(t.ruby[0].text, "かんじ");
        assert_eq!(t.ruby[0].position, RubyPosition::Below);
        assert!((t.ruby[0].size_percent - 40.0).abs() < 1e-9);
        // The loaded type lays the reading out too.
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        assert!(!lay.ruby.is_empty());
    }
}
