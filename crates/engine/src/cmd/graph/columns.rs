use super::data::fmt_value;
use super::designs::design_names;
use super::edit::{regenerate, spec_of, target};
use super::generate::generate_with;
use super::markers::{design_frame, fit_affine};
use super::series::{picked_series, reselect_series};
use super::*;

/// Most objects a graph's column designs may add (each column, repeated copy and legend swatch draws a copy).
const COLUMN_BUDGET: usize = 200_000;

/// A series' column design (Object › Graph › Column): its options; its art without guide lines and without the totals
/// taken out of it ([`without_totals`]); the bounds of those guides; those totals; its frame (the art's without guides,
/// totals included, [`design_frame`]); whether its art still holds totals (inside groups that do more than group, or
/// inside other objects that hold objects), which are filled where they are; and its art without any total for the
/// legend swatch, `None` when that draws nothing.
pub(super) struct ColumnArt {
    options: ColumnDesign,
    art: Node,
    guides: Vec<Rect>,
    totals: Vec<Node>,
    frame: Option<Rect>,
    in_place: bool,
    pub(super) legend: Option<Node>,
}

impl ColumnArt {
    /// A copy of `art` with the design's frame fitted to `cell`, as [`fit_design`] fits it. Fresh ids.
    pub(super) fn fit(&self, d: &mut Document, art: &Node, cell: Rect) -> Option<Arc<Node>> {
        let art = crate::cmd::place::transformed(art.clone(), fit_affine(self.frame?, cell));
        Some(Arc::new(d.reid(&art)))
    }
}

/// The column design totals in `s` (Add totals to a column design): each `%` followed by two digits, as its byte offset,
/// the places before the decimal point (the first digit) and the places after it (the second), left to right. A `%` not
/// followed by two digits is plain text.
fn total_spans(s: &str) -> Vec<(usize, usize, usize)> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while let Some(w) = b.get(i..i + 3) {
        match *w {
            [b'%', x, y] if x.is_ascii_digit() && y.is_ascii_digit() => {
                out.push((i, usize::from(x - b'0'), usize::from(y - b'0')));
                i += 3;
            }
            _ => i += 1,
        }
    }
    out
}

/// Value `v` as a column design total shows it: rounded half away from zero, from its shortest decimal form, to `after`
/// places after the decimal point, zeros added as needed; and with at least `before` places before the point, zeros
/// added in front (0: as many as the value needs). The places before the point are never cut, and a value that rounds
/// to 0 has no minus sign.
pub(super) fn total_text(v: f64, before: usize, after: usize) -> String {
    if !v.is_finite() {
        return fmt_value(v);
    }
    let s = format!("{}", v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((s.as_str(), ""));
    // The digits kept, the point `after` places from the end.
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes().chain(std::iter::repeat(b'0')).take(after)).collect();
    if frac.as_bytes().get(after).is_some_and(|d| *d >= b'5') {
        let mut carry = true;
        for d in digits.iter_mut().rev() {
            if *d == b'9' {
                *d = b'0';
            } else {
                *d += 1;
                carry = false;
                break;
            }
        }
        if carry {
            digits.insert(0, b'1');
        }
    }
    let (whole, places) = digits.split_at(digits.len().saturating_sub(after));
    let mut out = String::new();
    if v < 0.0 && digits.iter().any(|d| *d != b'0') {
        out.push('-');
    }
    out.extend(std::iter::repeat_n('0', before.saturating_sub(whole.len())));
    out.extend(whole.iter().map(|d| char::from(*d)));
    if after > 0 {
        out.push('.');
        out.extend(places.iter().map(|d| char::from(*d)));
    }
    out
}

/// Type `t` with each total in it ([`total_spans`]) replaced by value `v` ([`total_text`]), styled as its `%` was; true
/// when it held one.
pub(super) fn fill_totals(t: &mut TextObject, v: f64) -> bool {
    let spans = total_spans(&t.plain_text());
    // Last first, so the offsets of earlier ones stay valid.
    for &(at, before, after) in spans.iter().rev() {
        let s = total_text(v, before, after);
        t.splice_paras(at, at + 3, &s);
        vectorcraft_text::edit::replace_range(&mut t.runs, at, at + 3, &s);
    }
    if !spans.is_empty() {
        refresh_bounds(t);
    }
    !spans.is_empty()
}

/// Is `n` type that holds a total ([`total_spans`])?
fn holds_total(n: &Node) -> bool {
    matches!(&n.kind, NodeKind::Text(t) if !total_spans(&t.plain_text()).is_empty())
}

/// Does group `n` only group its children: no clipping, hiding, opacity, blend mode, isolation, knockout, appearance of
/// its own or opacity mask, so that type taken out of it draws as it did inside it?
fn only_groups(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.visible
        && n.opacity == 1.0
        && n.blend == vectorcraft_color::BlendMode::Normal
        && !n.isolate
        && n.knockout == vectorcraft_doc::Knockout::default()
        && n.appearance.items.is_empty()
        && n.appearance.effects.is_empty()
        && n.extras().mask.is_none()
}

/// Is `n` a clipping group whose clipping path is type that holds a total?
fn clipped_by_total(n: &Node) -> bool {
    matches!(&n.kind, NodeKind::Group { clip: true, children } if children.first().is_some_and(|c| holds_total(c)))
}

/// `art` without the type in it that holds a total, inside groups at any depth that only group ([`only_groups`]), and
/// that type, pushed to `totals` in paint order (Add totals to a column design); with `all`, without every total inside
/// any group or other object that holds objects (a blend, compound shape, envelope, repeat or intertwine), and with
/// each group that such type clips left empty. A design that is such type alone leaves an empty group.
fn without_totals(art: &Node, all: bool, totals: &mut Vec<Node>) -> Node {
    if holds_total(art) || (all && clipped_by_total(art)) {
        totals.push(art.clone());
        return Node::group(art.id, vec![]);
    }
    let mut n = art.clone();
    let take = all || only_groups(&n);
    if take && let Some(children) = n.children_mut() {
        for c in std::mem::take(children) {
            if holds_total(&c) {
                totals.push((*c).clone());
            } else if c.children().is_some() {
                children.push(Arc::new(without_totals(&c, all, totals)));
            } else {
                children.push(c);
            }
        }
    }
    n
}

/// `n` with the totals in it at any depth, in groups and the other objects that hold objects, showing value `v` where
/// they are ([`fill_totals`]).
fn fill_in_place(n: &mut Node, v: f64) {
    if let NodeKind::Text(t) = &mut n.kind {
        fill_totals(t, v);
    } else if let Some(children) = n.children_mut() {
        for c in children.iter_mut() {
            fill_in_place(Arc::make_mut(c), v);
        }
    }
}

/// `art` without the paths made into guides, at any depth.
fn without_guides(art: &Node) -> Node {
    let mut n = art.clone();
    if let Some(ch) = n.children_mut() {
        let old = std::mem::take(ch);
        *ch = old.iter().filter(|c| !matches!(c.kind, NodeKind::Path { guide: true, .. })).map(|c| Arc::new(without_guides(c))).collect();
    }
    n
}

/// Each series' column design, when the document has the design.
pub(super) fn column_designs(d: &Document, g: &GraphSpec) -> Vec<Option<ColumnArt>> {
    let nser = g.rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, vectorcraft_doc::MAX_GRAPH_SERIES);
    (0..nser)
        .map(|s| {
            let options = g.series_columns.get(s)?.as_ref()?;
            let art = &d.graph_designs.iter().find(|x| x.name == options.design)?.art;
            let mut guides = vec![];
            art.walk(&mut |n| {
                if matches!(n.kind, NodeKind::Path { guide: true, .. })
                    && let Some(b) = n.geometric_bounds()
                {
                    guides.push(b);
                }
            });
            let art = without_guides(art);
            let frame = design_frame(&art);
            let legend = without_totals(&art, true, &mut vec![]);
            let mut draws = false;
            legend.walk(&mut |n| draws |= n.children().is_none());
            let mut totals = vec![];
            let art = without_totals(&art, false, &mut totals);
            let mut in_place = false;
            art.walk(&mut |n| in_place |= holds_total(n));
            Some(ColumnArt { options: options.clone(), art, guides, totals, frame, in_place, legend: draws.then_some(legend) })
        })
        .collect()
}

/// The value one copy of a repeating design stands for, when the design repeats and the value is usable.
fn repeat_each(c: &ColumnDesign) -> Option<f64> {
    (c.column_type == ColumnType::Repeating && c.each.is_finite() && c.each > 0.0).then_some(c.each)
}

/// The objects a graph's column designs may still add (out of [`COLUMN_BUDGET`]); `short` once a copy needed more than
/// was left, and the graph is then drawn with plain columns.
pub(super) struct Allowance {
    left: usize,
    pub(super) short: bool,
}

impl Allowance {
    pub(super) fn new() -> Self {
        Self { left: COLUMN_BUDGET, short: false }
    }
    /// Spend `n` objects, when that many are left.
    pub(super) fn take(&mut self, n: usize) -> bool {
        match self.left.checked_sub(n) {
            Some(rest) => {
                self.left = rest;
                true
            }
            None => {
                self.short = true;
                false
            }
        }
    }
}

/// Column or bar `rect` of value `value` drawn with column design `c` (Object › Graph › Column), as a group of fresh art,
/// its copies spent from `left`. `far_low`: the value end of the column is at the rect's lower coordinate (the top of a
/// column above zero, the left end of a bar below zero), so repeating copies stack from the other end. `per_unit`: plot
/// points per unit of value. The design's totals show `value` and come last: each keeps its size and is moved to where
/// the design's copy puts the point it is set from; in a repeating design they are drawn once, where a whole copy ending
/// at the value end puts them, and a sliding design moves each with the part of the design on its side of the guide.
/// Totals left in the art ([`ColumnArt`]) show `value` in each copy, drawn with it. `None` for art with no area, for a
/// column of no length and when `left` runs out, which are drawn plain.
pub(super) fn column_art(
    d: &mut Document,
    c: &ColumnArt,
    (rect, value): (Rect, f64),
    (horizontal, far_low): (bool, bool),
    per_unit: f64,
    left: &mut Allowance,
) -> Option<Arc<Node>> {
    let frame = c.frame?;
    let length = |r: Rect| if horizontal { r.width() } else { r.height() };
    let thickness = |r: Rect| if horizontal { r.height() } else { r.width() };
    if length(rect) <= 1e-9 || !length(rect).is_finite() {
        return None;
    }
    let cost = c.art.count_with_masks();
    // A copy of the design moved by `a`, the totals left in it showing `value`; a copy fitted to `cell`; and a copy moved
    // by `a` and clipped to what it paints, cut at `at` (an x when `along_x`, else a y) with the side below `at` kept when
    // `keep_low` (two objects more).
    let copy = |d: &mut Document, a: Affine| {
        let mut art = d.reid(&crate::cmd::place::transformed(c.art.clone(), a));
        if c.in_place {
            fill_in_place(&mut art, value);
        }
        art
    };
    let fit = |d: &mut Document, cell: Rect, left: &mut Allowance| left.take(cost).then(|| Arc::new(copy(d, fit_affine(frame, cell))));
    let clipped = |d: &mut Document, a: Affine, (along_x, keep_low, at): (bool, bool, f64), left: &mut Allowance| {
        let art = left.take(cost + 2).then(|| copy(d, a))?;
        // The box of the copy and of each object in it that isn't a group or layer, as the brush art past a brushed
        // stroke counts only in its own object's box.
        let mut b = vectorcraft_render::painted_bounds(d, &art);
        art.walk(&mut |n| {
            if !matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) {
                b = vectorcraft_geom::union_opt(b, vectorcraft_render::painted_bounds(d, n));
            }
        });
        // A copy that draws nothing (a design of totals alone) needs no clip.
        let Some(b) = b.or_else(|| art.geometric_bounds()) else { return Some(Arc::new(art)) };
        let keep = match (along_x, keep_low) {
            (true, true) => Rect::new(b.x0, b.y0, at.max(b.x0), b.y1),
            (true, false) => Rect::new(at.min(b.x1), b.y0, b.x1, b.y1),
            (false, true) => Rect::new(b.x0, b.y0, b.x1, at.max(b.y0)),
            (false, false) => Rect::new(b.x0, at.min(b.y1), b.x1, b.y1),
        };
        Some(Arc::new(crate::cmd::rasterfx::clip_group(d, shapes::rectangle(keep), FillRule::NonZero, art)))
    };
    // The part of the column from `a` to `b` points out from its zero end.
    let piece = |a: f64, b: f64| match (horizontal, far_low) {
        (true, true) => Rect::new(rect.x1 - b, rect.y0, rect.x1 - a, rect.y1),
        (true, false) => Rect::new(rect.x0 + a, rect.y0, rect.x0 + b, rect.y1),
        (false, true) => Rect::new(rect.x0, rect.y1 - b, rect.x1, rect.y1 - a),
        (false, false) => Rect::new(rect.x0, rect.y0 + a, rect.x1, rect.y0 + b),
    };
    // The column's group: `parts`, then the totals, each showing `value` at its own size, moved as `a` moves the point it
    // is set from; with `sliding` (the kept part's map, the stretched part's map and the guide, as [`slide`] gives them),
    // as the map of the part on its side of the guide moves it. One object each.
    let group = |d: &mut Document, mut parts: Vec<Arc<Node>>, a: Affine, sliding: Option<(Affine, Affine, f64)>, left: &mut Allowance| {
        for t in &c.totals {
            if !left.take(1) {
                return None;
            }
            let mut n = t.clone();
            let mut by = Affine::IDENTITY;
            if let NodeKind::Text(text) = &mut n.kind {
                // A move only, so the type keeps its size; pattern paints move with it.
                let p = text.xf * Point::ZERO;
                let a = match sliding {
                    Some((kept, stretched, at)) => {
                        let q = kept * p;
                        if (horizontal && q.x >= at) || (!horizontal && q.y <= at) { kept } else { stretched }
                    }
                    None => a,
                };
                fill_totals(text, value);
                by = Affine::translate(a * p - p);
            }
            parts.push(Arc::new(d.reid(&crate::cmd::place::transformed(n, by))));
        }
        Some(Arc::new(Node::group(d.alloc_id(), parts)))
    };
    let mut parts = vec![];
    let (mut label, mut sliding) = (fit_affine(frame, rect), None);
    match c.options.column_type {
        ColumnType::Uniform => {
            // Scaled both ways to the column's length, centered across it.
            let t = thickness(frame) * length(rect) / length(frame) / 2.0;
            let cell = if horizontal {
                Rect::new(rect.x0, rect.center().y - t, rect.x1, rect.center().y + t)
            } else {
                Rect::new(rect.center().x - t, rect.y0, rect.center().x + t, rect.y1)
            };
            parts.push(fit(d, cell, left)?);
            label = fit_affine(frame, cell);
        }
        ColumnType::Repeating => {
            // When the value per copy is unusable, too small to draw or makes one copy longer than the canvas, the design
            // is drawn vertically scaled.
            let Some(unit) = repeat_each(&c.options).map(|e| e * per_unit).filter(|u| u.is_finite() && *u > 1e-9 && *u <= crate::MAX_COORD) else {
                parts.push(fit(d, rect, left)?);
                return group(d, parts, label, None, left);
            };
            // The totals go where a whole copy ending at the value end puts them.
            label = fit_affine(frame, piece(length(rect) - unit, length(rect)));
            // The whole copies, and the points of the column left for the last one (none under a millionth of a point).
            let whole = ((length(rect) + 1e-6) / unit).floor();
            let rest = length(rect) - whole * unit;
            // Each copy is spent from `left`, which stops the loop.
            for i in 0..whole as usize {
                parts.push(fit(d, piece(i as f64 * unit, (i + 1) as f64 * unit), left)?);
            }
            if rest > 1e-6 {
                let start = whole * unit;
                match c.options.fractions {
                    Fractions::Scale => parts.push(fit(d, piece(start, start + rest), left)?),
                    Fractions::Chop => {
                        // The last copy cut off where the column ends, and nowhere else.
                        let cut = match (horizontal, far_low) {
                            (true, true) => (true, false, rect.x0),
                            (true, false) => (true, true, rect.x1),
                            (false, true) => (false, false, rect.y0),
                            (false, false) => (false, true, rect.y1),
                        };
                        parts.push(clipped(d, fit_affine(frame, piece(start, start + unit)), cut, left)?);
                    }
                }
            }
        }
        ColumnType::Sliding => match slide(frame, &c.guides, rect, horizontal) {
            Some((kept, stretched, at)) => {
                // Each copy cut at the guide, on its own side.
                parts.push(clipped(d, kept, (horizontal, !horizontal, at), left)?);
                parts.push(clipped(d, stretched, (horizontal, horizontal, at), left)?);
                sliding = Some((kept, stretched, at));
            }
            None => parts.push(fit(d, rect, left)?),
        },
        ColumnType::Vertical => parts.push(fit(d, rect, left)?),
    }
    group(d, parts, label, sliding, left)
}

/// Sliding design with frame `frame` in column `rect`, at the column's width: the map of the part above the first
/// horizontal guide in `guides` inside the frame (in a bar, right of the first vertical one), scaled in proportion and
/// put at the rect's top (in a bar, its right end), the value end for values above zero; the map of the part below (left
/// of) it, stretched or compressed to fill the rest; and where the guide lands (y, or x in a bar). Both maps put the guide there. `None` without such a guide, or when the column is too
/// short for the part kept in proportion.
fn slide(frame: Rect, guides: &[Rect], rect: Rect, horizontal: bool) -> Option<(Affine, Affine, f64)> {
    // A horizontal line (in a bar, a vertical one): longer than it is across, and at most a hundredth of the frame across.
    let inside = |g: &&Rect| {
        if horizontal {
            g.width() < g.height() && g.width() <= frame.width() / 100.0 && g.center().x > frame.x0 && g.center().x < frame.x1
        } else {
            g.height() < g.width() && g.height() <= frame.height() / 100.0 && g.center().y > frame.y0 && g.center().y < frame.y1
        }
    };
    let g = guides.iter().find(inside)?.center();
    if horizontal {
        let s = rect.height() / frame.height();
        let (kept, stretched) = (frame.x1 - g.x, g.x - frame.x0);
        let rest = rect.width() - kept * s;
        if !(rest.is_finite() && rest > 1e-9) {
            return None;
        }
        let k = rest / stretched;
        let kept_map = Affine::new([s, 0.0, 0.0, s, rect.x1 - frame.x1 * s, rect.y0 - frame.y0 * s]);
        let stretched_map = Affine::new([k, 0.0, 0.0, s, rect.x0 - frame.x0 * k, rect.y0 - frame.y0 * s]);
        Some((kept_map, stretched_map, rect.x1 - kept * s))
    } else {
        let s = rect.width() / frame.width();
        let (kept, stretched) = (g.y - frame.y0, frame.y1 - g.y);
        let rest = rect.height() - kept * s;
        if !(rest.is_finite() && rest > 1e-9) {
            return None;
        }
        let k = rest / stretched;
        let kept_map = Affine::new([s, 0.0, 0.0, s, rect.x0 - frame.x0 * s, rect.y0 - frame.y0 * s]);
        let stretched_map = Affine::new([s, 0.0, 0.0, k, rect.x0 - frame.x0 * s, rect.y1 - frame.y1 * k]);
        Some((kept_map, stretched_map, rect.y0 + kept * s))
    }
}

pub(super) fn set_column(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.column";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).min(vectorcraft_doc::MAX_GRAPH_SERIES);
    let picked = picked_series(s, p, id, count, C)?;
    let names = design_names(&s.doc()?.doc);
    let all: Vec<usize> = if picked.is_empty() { (0..count).collect() } else { picked.clone() };
    // The column design the picked series (all of them with none picked) share; a design deleted since doesn't count,
    // as those series draw plain columns.
    let used: Vec<Option<&ColumnDesign>> =
        all.iter().map(|i| spec.series_columns.get(*i).and_then(Option::as_ref).filter(|c| names.contains(&c.design))).collect();
    let shared = used.first().copied().flatten().filter(|c| used.iter().all(|x| *x == Some(*c))).cloned();
    let Some(v) = p.get("design") else {
        let options = shared.clone().unwrap_or_default();
        return Ok(json!({
            "designs": names,
            "design": shared.map(|c| c.design),
            "columnType": options.column_type.id(),
            "each": options.each,
            "fractions": options.fractions.id(),
        }));
    };
    let design = match v {
        Value::Null => None,
        Value::String(n) if names.contains(n) => Some(n.clone()),
        Value::String(n) => return Err(bad(C, format!("no design named `{n}`"))),
        _ => return Err(bad(C, "`design` is a design name, or null for plain columns")),
    };
    let mut options = shared.unwrap_or_default();
    if let Some(t) = p.get("columnType") {
        options.column_type =
            t.as_str().and_then(ColumnType::parse).ok_or_else(|| bad(C, "`columnType` is vertical, uniform, repeating or sliding"))?;
    }
    if let Some(e) = p.get("each") {
        options.each =
            e.as_f64().filter(|e| e.is_finite() && *e > 0.0).ok_or_else(|| bad(C, "`each` is the value one design stands for, more than 0"))?;
    }
    if let Some(f) = p.get("fractions") {
        options.fractions = f.as_str().and_then(Fractions::parse).ok_or_else(|| bad(C, "`fractions` is chop or scale"))?;
    }
    let reselect: Vec<u32> = if p.get("seriesIndexes").is_none() { picked.iter().filter_map(|i| u32::try_from(*i).ok()).collect() } else { vec![] };
    spec.series_columns.truncate(count);
    spec.series_columns.resize(count, None);
    for i in all {
        if let Some(c) = spec.series_columns.get_mut(i) {
            *c = design.clone().map(|design| ColumnDesign { design, ..options.clone() });
        }
    }
    while spec.series_columns.last().is_some_and(Option::is_none) {
        spec.series_columns.pop();
    }
    // Drawn once on a copy of the document, to refuse designs past the budget before the edit.
    let mut probe = (*s.doc()?.doc).clone();
    let looks = column_designs(&probe, &spec);
    if generate_with(&mut probe, &spec, &looks).is_none() {
        return Err(bad(C, "the design is too big to draw in every column of this graph"));
    }
    s.edit("Graph Column", |d, sel| {
        regenerate(d, id, spec)?;
        if !reselect.is_empty() {
            reselect_series(d, sel, id, &reselect);
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}
