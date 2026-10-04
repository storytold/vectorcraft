//! Type menu: Create Outlines, and the Character/Paragraph panel setters (`text.setText`,
//! `text.setStyle`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Justify, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::PathData;

use super::edit::selected_roots;
use super::pathops::{num_param, shape_node};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
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
            "{ids?, rows?, columns?, gutter?: pt, inset?: pt, firstBaseline?: ascent|capHeight|xHeight|leading|fixed, firstBaselineMin?: pt} set the selected area type's options (none given: query) → the first object's options",
            has_selection,
            area_options
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
            "{ids?|id?, font?, style?, size?: pt, leading?: pt|\"auto\", tracking?: 1/1000 em, justify?: \"left\"|\"center\"|\"right\"|\"justifyAll\", fill?: colour, features?: [\"dlig\", \"-liga\", …] OpenType}",
            has_doc,
            set_style
        ),
    ]
}

/// Recompute the layout bounds cache after a text edit.
pub(crate) fn refresh_bounds(t: &mut TextObject) {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    t.cached_bounds = Some(lay.bounds);
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
            for g in &lay.glyphs {
                let path = PathData::from_bezpath(&g.outline).transformed(t.xf);
                if path.is_empty() {
                    continue;
                }
                let st = t.runs.get(g.run).map(|r| r.style.clone()).unwrap_or_else(|| t.first_style());
                let mut node = shape_node(d, path, None);
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

fn set_text(s: &mut Session, p: &Value) -> Result<Value> {
    let text = str_param(p, "text").ok_or_else(|| bad("text.setText", "missing `text`"))?.to_string();
    let ids = text_targets(s, p, "text.setText")?;
    s.edit("Typing", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let style = t.first_style();
            t.runs = vec![vectorcraft_doc::TextRun { text: text.clone(), style }];
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn justify_param(v: &str) -> Option<Justify> {
    Some(match v.to_ascii_lowercase().as_str() {
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
    let size = num_param(p, "size");
    if size.is_some_and(|v| v <= 0.0) {
        return Err(bad(C, "size must be positive"));
    }
    let leading = match p.get("leading") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) if a.eq_ignore_ascii_case("auto") => Some(None),
        Some(_) => Some(Some(num_param(p, "leading").ok_or_else(|| bad(C, "leading must be a number or \"auto\""))?.clamp(0.1, 5000.0))),
    };
    let tracking = num_param(p, "tracking");
    let justify = match str_param(p, "justify") {
        Some(j) => Some(justify_param(j).ok_or_else(|| bad(C, "justify must be left|center|right|justifyAll"))?),
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
    let ids = text_targets(s, p, C)?;
    s.edit("Character", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            for r in &mut t.runs {
                let st = &mut r.style;
                if let Some(f) = &font {
                    st.font_family = f.clone();
                }
                if let Some(f) = &style {
                    st.font_style = f.clone();
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
            }
            if let Some(j) = justify {
                t.para.justify = j;
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn area_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.areaOptions";
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        text_targets(s, p, C)?
            .into_iter()
            .filter(|i| matches!(d.node(*i).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, vectorcraft_doc::TextKind::Area { .. })))
            .collect()
    };
    let first = ids.first().ok_or_else(|| bad(C, "select area type (text in a frame)"))?;
    let current = match &s.doc()?.doc.node(*first).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => t.area.clone(),
        _ => return Err(bad(C, "select area type (text in a frame)")),
    };
    let mut v = serde_json::to_value(&current).map_err(|e| EngineError::Other(e.to_string()))?;
    let mut changed = false;
    if let (Some(o), Some(src)) = (v.as_object_mut(), p.as_object()) {
        for (k, val) in src {
            if o.contains_key(k) && k != "ids" {
                o.insert(k.clone(), val.clone());
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(v);
    }
    let mut opts: vectorcraft_doc::AreaOptions = serde_json::from_value(v).map_err(|e| bad(C, e.to_string()))?;
    opts.rows = opts.rows.clamp(1, 100);
    opts.columns = opts.columns.clamp(1, 100);
    opts.gutter = opts.gutter.clamp(0.0, 10_000.0);
    opts.inset = opts.inset.clamp(0.0, 10_000.0);
    opts.first_baseline_min = opts.first_baseline_min.clamp(0.0, 10_000.0);
    let out = serde_json::to_value(&opts).map_err(|e| EngineError::Other(e.to_string()))?;
    s.edit("Area Type Options", |d, _| {
        for id in &ids {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                t.area = opts.clone();
                refresh_bounds(t);
            }
        }
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod area_tests {
    use super::*;

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
    let avail = cell.width() - 2.0 * t.area.inset - t.para.left_indent - t.para.right_indent;
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
