//! Type menu: Create Outlines, and the Character/Paragraph panel setters (`text.setText`,
//! `text.setStyle`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{AreaFit, Document, Justify, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, PathData, Vec2};

use super::edit::selected_roots;
use super::pathops::{len_param, num_param, shape_node};
use super::*;

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
            "{} convert the selected text to groups of compound paths (one per glyph) → {ids}",
            has_selection,
            create_outlines
        ),
        cmd!(
            "text.setText",
            "Set Text",
            [],
            None,
            "{id?|ids?, text} replace the contents of text objects (keeps the first run's style)",
            has_doc,
            set_text
        ),
        cmd!(
            "text.areaOptions",
            "Area Type Options…",
            ["Type"],
            None,
            "{ids?, width?: pt, height?: pt, rows?, columns?, gutter?: pt, inset?: pt, firstBaseline?: ascent|capHeight|xHeight|leading|fixed, firstBaselineMin?: pt, verticalAlign?: top|center|bottom|justify, fit?: none|autoHeight|shrinkText, fitMinPercent?: 10..100} set the selected area type's options; width and height size the type area from its top-left corner, along the type's own axes, and the text reflows at its size (with fit autoHeight, Auto Size, the frame's height follows the text after every edit and height is ignored; setting the height by hand turns it off); fit shrinkText scales overflowing text down (size, leading, baseline shift) by the largest factor down to fitMinPercent % (default 50) that makes it fit (none given: query) → the first object's options, plus overflow (the text doesn't fit its frame) and fitScale (Shrink Text's factor, 1 unshrunk)",
            has_selection,
            area_options
        ),
        cmd!(
            "text.reshapeArea",
            "Reshape Type Area",
            [],
            None,
            "{id, anchors: [[subpath, anchor]…], dx, dy} move anchors of area type's frame (the type area) by dx, dy points, with their handles; the text reflows at its size, through its thread too (what Direct Selection does dragging a frame corner or edge)",
            has_doc,
            reshape_area
        ),
        cmd!(
            "text.fitHeadline",
            "Fit Headline",
            ["Type"],
            None,
            "{ids?} track the first line of area type so it fills the frame width → {tracking}",
            has_selection,
            fit_headline
        ),
        cmd!(
            "text.setStyle",
            "Character",
            [],
            None,
            "{ids?|id?, font?, style?, size?: pt, leading?: pt|\"auto\", tracking?: 1/1000 em, justify?: \"auto\" (the start of each paragraph's direction)|\"left\"|\"center\"|\"right\"|\"justifyAll\", fill?: colour, features?: [\"dlig\", \"-liga\", …] OpenType, start?: byte, end?: byte} (with a range: the character attributes style that range and justify the paragraphs it touches; without: all the text)",
            has_doc,
            set_style
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
            with_size(probe, b.width(), h);
            !vectorcraft_text::layout(db, probe).overflow
        };
        if !fits(&mut probe, h) {
            return false;
        }
        let (mut lo, mut hi) = (1.0, h);
        for _ in 0..24 {
            if hi - lo <= 0.01 {
                break;
            }
            let mid = (lo + hi) * 0.5;
            if fits(&mut probe, mid) { hi = mid } else { lo = mid }
        }
        h = hi;
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

fn create_outlines(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let ids = s.edit("Create Outlines", |d, sel| {
        let texts = text_ids(d, &roots);
        if texts.is_empty() {
            return Err(EngineError::Other("Create Outlines: select text objects".into()));
        }
        let mut new_sel: Vec<NodeId> = roots.iter().copied().filter(|r| !texts.contains(r)).collect();
        let mut out = vec![];
        for tid in texts {
            let Some(n) = d.node(tid).cloned() else { continue };
            let NodeKind::Text(t) = &n.kind else { continue };
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let mut children = vec![];
            for (gi, g) in lay.glyphs.iter().enumerate() {
                // Inline graphics become instances of their symbols, in place.
                if let Some(ig) = lay.inlines.iter().find(|i| i.glyph == gi)
                    && let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref())
                {
                    let id = d.alloc_id();
                    let xf = t.xf * ig.xf * d.symbol_natural_xf(&art.symbol);
                    children.push(Arc::new(Node::new(id, NodeKind::SymbolInstance { symbol: art.symbol.clone(), xf })));
                    continue;
                }
                let path = PathData::from_bezpath(&g.outline).transformed(t.xf);
                if path.is_empty() {
                    continue;
                }
                let st = t.runs.get(g.run).map(|r| r.style.clone()).unwrap_or_else(|| t.first_style());
                let mut node = shape_node(d, path, None);
                node.appearance = st.appearance();
                children.push(Arc::new(node));
            }
            // Underline and strikethrough bars become paths of their run's paint too (#847).
            for (run, bar) in vectorcraft_text::decorations(&lay, vectorcraft_text::FontDb::global(), t) {
                let st = t.runs.get(run).map(|r| r.style.clone()).unwrap_or_else(|| t.first_style());
                let mut node = shape_node(d, PathData::from_bezpath(&bar).transformed(t.xf), None);
                node.appearance = st.appearance();
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
                Some(v) => {
                    v.as_u64().map(|n| Some(usize::try_from(n).unwrap_or(usize::MAX))).ok_or_else(|| bad(cmd, format!("`{k}` must be a byte offset")))
                }
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
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn justify_param(v: &str) -> Option<Justify> {
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
    let fill = match p.get("fill") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) if n.eq_ignore_ascii_case("none") => Some(vectorcraft_color::Paint::None),
        Some(v) => Some(vectorcraft_color::Paint::solid(color_value(v).ok_or_else(|| bad(C, "bad fill colour"))?)),
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
            let before = protect.then(|| t.runs.clone());
            style_chars(t, range, |st| {
                if let Some(f) = &font {
                    st.font_family = f.clone();
                }
                if let Some(f) = &style {
                    st.font_style = f.clone();
                }
                // Another font: none of the old one's versions.
                if font.is_some() || style.is_some() {
                    st.font_version = None;
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
            if let Some(before) = before {
                super::textedit::protect_missing_glyphs(&before, &mut t.runs);
            }
            if let Some(j) = justify {
                let span = para_span(range, t);
                t.edit_paras(span, |pa| pa.justify = j);
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
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
    opts.rows = opts.rows.clamp(1, 100);
    opts.columns = opts.columns.clamp(1, 100);
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
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let mut refs = super::select::parse_refs(p.get("anchors"));
    refs.sort_unstable();
    refs.dedup();
    if refs.is_empty() {
        return Err(bad(C, "missing anchors [[subpath, anchor]…]"));
    }
    let d = Vec2::new(f64_req(p, "dx", C)?, f64_req(p, "dy", C)?);
    let node = s.doc()?.doc.node(id);
    if area_text(node).is_none() || node.is_some_and(|n| n.perspective.is_some()) {
        return Err(bad(C, "not area type (text in a frame) outside perspective"));
    }
    s.edit("Reshape Type Area", |doc, _| {
        let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) else { return Err(EngineError::NoNode(id)) };
        if !reshape_area_with(t, |t| t.move_area_anchors(&refs, d)) {
            return Err(bad(C, "no such frame anchor, or the frame would reach past the canvas"));
        }
        Ok(())
    })?;
    ok()
}

#[cfg(test)]
mod area_tests {
    use super::*;

    #[test]
    fn kinsoku_set_is_set_per_paragraph_and_saved_only_when_not_hard() {
        use vectorcraft_doc::Kinsoku;
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "あカッ", "area": {"width": 40, "height": 100}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.para.clone(),
            _ => panic!("text"),
        };
        // New type: Hard, today's set.
        assert_eq!(para(&s).kinsoku, Kinsoku::Hard);
        assert!(serde_json::to_value(para(&s)).unwrap().get("kinsoku").is_none());
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(s.execute("text.setFormat", &json!({"kinsoku": "strict"})).is_err());
        s.execute("text.setFormat", &json!({"kinsoku": "soft"})).unwrap();
        assert_eq!(para(&s).kinsoku, Kinsoku::Soft);
        assert_eq!(serde_json::to_value(para(&s)).unwrap()["kinsoku"], "soft");
        s.execute("text.setFormat", &json!({"kinsoku": "none"})).unwrap();
        assert_eq!(para(&s).kinsoku, Kinsoku::None);
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(para(&s).kinsoku, Kinsoku::Hard);
        // Documents from before it read as Hard.
        let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left"})).unwrap();
        assert_eq!(old.kinsoku, Kinsoku::Hard);
    }

    #[test]
    fn burasagari_is_set_per_paragraph_and_saved_only_when_on() {
        use vectorcraft_doc::Burasagari;
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "雅楽、笙。", "area": {"width": 200, "height": 100}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.para.clone(),
            _ => panic!("text"),
        };
        // New type: Standard.
        assert_eq!(para(&s).burasagari, Burasagari::Standard);
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(s.execute("text.setFormat", &json!({"burasagari": "strong"})).is_err());
        s.execute("text.setFormat", &json!({"burasagari": "forced"})).unwrap();
        assert_eq!(para(&s).burasagari, Burasagari::Forced);
        assert_eq!(serde_json::to_value(para(&s)).unwrap()["burasagari"], "forced");
        s.execute("text.setFormat", &json!({"burasagari": "none"})).unwrap();
        assert_eq!(para(&s).burasagari, Burasagari::None);
        // Saved only when on; documents from before it read as None.
        assert!(serde_json::to_value(para(&s)).unwrap().get("burasagari").is_none());
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(para(&s).burasagari, Burasagari::Standard);
        let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left"})).unwrap();
        assert_eq!(old.burasagari, Burasagari::None);
    }

    /// #432: while the interface is in Japanese, new type (text.create, text.createInPath) starts
    /// with em box top-to-top leading and em box centre alignment, and new styles made from nothing
    /// carry them; params override; the values are journaled; imported text keeps the Roman
    /// baseline.
    #[test]
    fn new_type_takes_the_japanese_defaults_while_the_interface_is_japanese() {
        use vectorcraft_doc::{CharAlign, LeadingModel};
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let made = |s: &mut Session, cmd: &str, p: Value| -> (LeadingModel, CharAlign) {
            let id = s.execute(cmd, &p).unwrap()["id"].as_u64().unwrap();
            match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
                NodeKind::Text(t) => (t.para.leading_model, t.runs[0].style.char_align),
                _ => panic!("text"),
            }
        };
        let roman = (LeadingModel::RomanBaseline, CharAlign::RomanBaseline);
        let japanese = (LeadingModel::EmBoxTop, CharAlign::EmBoxCenter);
        let point = json!({"x": 10, "y": 50, "text": "雅楽"});
        assert!(!s.japanese_interface());
        assert_eq!(made(&mut s, "text.create", point.clone()), roman, "headless, `auto`: the Roman defaults");
        s.ui_language = Some("ja".into());
        assert_eq!(made(&mut s, "text.create", point.clone()), japanese);
        assert_eq!(made(&mut s, "text.create", json!({"x": 10, "y": 90, "text": "笙", "area": {"width": 100, "height": 50}})), japanese);
        // Journaled: a replay in another language does the same.
        let (_, logged) = s.journal.last().unwrap().clone();
        assert_eq!((logged["leadingModel"].as_str(), logged["charAlign"].as_str()), (Some("emBoxTop"), Some("emBoxCenter")));
        // Params override.
        assert_eq!(
            made(&mut s, "text.create", json!({"x": 10, "y": 120, "text": "a", "leadingModel": "romanBaseline", "charAlign": "romanBaseline"})),
            roman
        );
        assert!(s.execute("text.create", &json!({"x": 0, "y": 0, "text": "a", "leadingModel": "middle"})).is_err());
        // Area type in a path.
        let path = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 100, "height": 80})).unwrap()["id"].clone();
        assert_eq!(made(&mut s, "text.createInPath", json!({"path": path, "mode": "area", "text": "篳篥"})), japanese);
        // A preference set to Japanese counts without a UI; another language doesn't.
        s.ui_language = None;
        s.prefs.interface_language = "ja".into();
        assert_eq!(made(&mut s, "text.create", point.clone()), japanese);
        s.ui_language = Some("en".into());
        assert_eq!(made(&mut s, "text.create", point.clone()), roman);
        // New styles made from nothing.
        s.execute("select.set", &json!({"ids": []})).unwrap();
        s.ui_language = Some("ja".into());
        let style = |s: &mut Session, kind: &str| {
            let name = s.execute(&format!("{kind}.new"), &json!({})).unwrap()["name"].as_str().unwrap().to_string();
            let list = s.execute(&format!("{kind}.list"), &json!({})).unwrap();
            list["styles"].as_array().unwrap().iter().find(|st| st["name"] == name.as_str()).unwrap()["attrs"].clone()
        };
        assert_eq!(style(&mut s, "paraStyle"), json!({"leading_model": "emBoxTop"}));
        assert_eq!(style(&mut s, "charStyle"), json!({"charAlign": "emBoxCenter"}));
        s.ui_language = Some("en".into());
        assert_eq!(style(&mut s, "paraStyle"), json!({}));
        // Imported text keeps the Roman baseline.
        s.ui_language = Some("ja".into());
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><text x="10" y="50">雅楽</text></svg>"#;
        s.execute("document.open", &json!({"name": "t.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
        let mut found = vec![];
        s.doc().unwrap().doc.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                found.push((t.para.leading_model, t.runs[0].style.char_align));
            }
        });
        assert_eq!(found, [roman]);
    }

    #[test]
    fn leading_model_is_set_per_paragraph_and_saved_only_when_top_to_top() {
        use vectorcraft_doc::LeadingModel;
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        // Exercise ink bounds with bundled glyphs; Japanese outlines require optional craft-fonts.
        let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "A\nB", "area": {"width": 200, "height": 100}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let text = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => (**t).clone(),
            _ => panic!("text"),
        };
        assert_eq!(text(&s).para.leading_model, LeadingModel::RomanBaseline);
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(s.execute("text.setFormat", &json!({"leadingModel": "middle"})).is_err());
        let before = text(&s).cached_bounds;
        s.execute("text.setFormat", &json!({"leadingModel": "emBoxTop"})).unwrap();
        let t = text(&s);
        assert_eq!(t.para.leading_model, LeadingModel::EmBoxTop);
        // The fallback used without craft-fonts has no glyph metrics to move the first line.
        if !vectorcraft_text::CRAFT_FONTS.is_empty() {
            assert_ne!(t.cached_bounds, before, "the first line moves up to the frame's top");
        }
        assert_eq!(serde_json::to_value(&t.para).unwrap()["leading_model"], "emBoxTop");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(serde_json::to_value(&text(&s).para).unwrap().get("leading_model").is_none());
    }

    #[test]
    fn character_alignment_is_a_character_attribute_saved_only_when_set() {
        use vectorcraft_doc::CharAlign;
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "雅楽"})).unwrap()["id"].as_u64().unwrap();
        let runs = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.runs.clone(),
            _ => panic!("text"),
        };
        assert_eq!(runs(&s)[0].style.char_align, CharAlign::RomanBaseline);
        assert!(serde_json::to_value(&runs(&s)[0].style).unwrap().get("charAlign").is_none());
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(s.execute("text.setFormat", &json!({"charAlign": "middle"})).is_err());
        s.execute("text.setFormat", &json!({"charAlign": "emBoxCenter"})).unwrap();
        assert!(runs(&s).iter().all(|r| r.style.char_align == CharAlign::EmBoxCenter));
        assert_eq!(serde_json::to_value(&runs(&s)[0].style).unwrap()["charAlign"], "emBoxCenter");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(runs(&s)[0].style.char_align, CharAlign::RomanBaseline);
        // Characters selected with the Type tool: only the range takes it (雅 is 3 bytes).
        assert!(s.execute("text.setRangeStyle", &json!({"id": id, "start": 3, "end": 6, "charAlign": "top"})).is_err());
        s.execute("text.setRangeStyle", &json!({"id": id, "start": 3, "end": 6, "charAlign": "emBoxTop"})).unwrap();
        let aligns: Vec<_> = runs(&s).iter().map(|r| (r.text.clone(), r.style.char_align)).collect();
        assert_eq!(aligns, [("雅".to_string(), CharAlign::RomanBaseline), ("楽".to_string(), CharAlign::EmBoxTop)]);
        // The ideographic character face's top and bottom.
        for (key, want) in [("icfTop", CharAlign::IcfTop), ("icfBottom", CharAlign::IcfBottom)] {
            s.execute("text.setFormat", &json!({"charAlign": key})).unwrap();
            assert!(runs(&s).iter().all(|r| r.style.char_align == want));
            assert_eq!(serde_json::to_value(&runs(&s)[0].style).unwrap()["charAlign"], key);
        }
    }

    #[test]
    fn new_type_is_composed_with_line_end_half_width_punctuation() {
        use vectorcraft_doc::Mojikumi;
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "雅楽。"})).unwrap()["id"].as_u64().unwrap();
        let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.para.clone(),
            _ => panic!("text"),
        };
        assert_eq!(para(&s).mojikumi, Mojikumi::LineEndHalf);
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(s.execute("text.setFormat", &json!({"mojikumi": "everything"})).is_err());
        s.execute("text.setFormat", &json!({"mojikumi": "none"})).unwrap();
        assert_eq!(para(&s).mojikumi, Mojikumi::None);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(para(&s).mojikumi, Mojikumi::LineEndHalf);
        // Saved only when set; documents from before it read as None.
        let json = serde_json::to_value(para(&s)).unwrap();
        assert_eq!(json["mojikumi"], "lineEndHalf");
        let old: vectorcraft_doc::ParaStyle = serde_json::from_value(json!({"justify": "Left"})).unwrap();
        assert_eq!(old.mojikumi, Mojikumi::None);
        assert!(serde_json::to_value(&old).unwrap().get("mojikumi").is_none());
    }

    #[test]
    fn vertical_point_type_keeps_its_anchor_on_the_column_centre_line() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let id = s.execute("text.create", &json!({"x": 100, "y": 50, "text": "§§§§ abc"})).unwrap()["id"].as_u64().unwrap();
        let bounds = |s: &Session| s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
        let wide = bounds(&s);
        assert!(wide.width() > wide.height());
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        s.execute("type.orientation.vertical", &json!({})).unwrap();
        let tall = bounds(&s);
        assert!(tall.height() > tall.width(), "{tall:?}");
        assert!((tall.center().x - 100.0).abs() < 6.0, "the anchor stays on the column's centre line: {tall:?}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(bounds(&s), wide);
    }

    #[test]
    fn area_options_columns_change_layout() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        let text = "word ".repeat(80);
        let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": text, "area": {"width": 300, "height": 120}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let q = s.execute("text.areaOptions", &json!({})).unwrap();
        assert_eq!((q["rows"].as_u64(), q["columns"].as_u64()), (Some(1), Some(1)));
        let lay = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t),
            _ => panic!(),
        };
        assert_eq!(lay(&s).frames.len(), 1);
        let r = s.execute("text.areaOptions", &json!({"columns": 3, "gutter": 12, "inset": 4, "firstBaseline": "capHeight"})).unwrap();
        assert_eq!(r["firstBaseline"], "capHeight");
        let l = lay(&s);
        assert_eq!(l.frames.len(), 3, "three column cells");
        // Glyphs land in more than one column.
        let xs: Vec<f64> = l.glyphs.iter().map(|g| g.origin.x).collect();
        assert!(xs.iter().any(|x| *x > 110.0 + 10.0) && xs.iter().any(|x| *x < 100.0));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(lay(&s).frames.len(), 1);
        assert!(s.execute("text.areaOptions", &json!({"firstBaseline": "nope"})).is_err());
    }
}

/// Width of the first line and of the space it can fill, with `tracking` on the first paragraph.
fn headline_fit(t: &TextObject, tracking: f64) -> Option<(f64, f64, usize)> {
    let mut probe = t.clone();
    let para_end = probe.plain_text().find('\n').unwrap_or(usize::MAX);
    vectorcraft_text::edit::style_range(&mut probe.runs, 0, para_end.min(vectorcraft_text::edit::runs_len(&t.runs)), |st| st.tracking = tracking);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &probe);
    let cell = lay.frames.first()?;
    let line = lay.lines.first()?;
    let para = t.para_at(0);
    let avail = cell.width() - 2.0 * t.area.inset - para.left_indent - para.right_indent;
    Some((line.x1 - line.x0, avail, lay.lines.iter().filter(|l| l.start < para_end).count()))
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
        // Largest tracking that keeps the first paragraph on one line (bisection).
        let one_line = |tr: f64| headline_fit(t, tr).is_some_and(|(_, _, lines)| lines == 1);
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
        let (w, avail, lines) = headline_fit(t, tr).unwrap();
        assert_eq!(lines, 1);
        assert!(avail - w < 30.0, "{w} of {avail}");
        assert_eq!(t.runs.last().unwrap().style.tracking, 0.0, "the body keeps its tracking");
    }
}
