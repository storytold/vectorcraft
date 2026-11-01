//! Create Swatch Legend (#4919): a grid of chips on the artboard, one per colour swatch, each
//! filled with its swatch and labelled with the swatch's name and colour values, as one group made
//! in one undo step.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint, cms::Model};
use vectorcraft_doc::{Appearance, Node, NodeKind, TextObject};
use vectorcraft_geom::{Point, Rect, shapes};

use super::{CommandSpec, Result, Session, bad, cmd, has_doc};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "swatch.legend",
        "Create Swatch Legend",
        ["Window", "Swatches"],
        None,
        "{names?: [swatch name…] (default: every colour swatch of the document, in panel order), columns?: 1..50 (4), size?: chip side in pt 8..1000 (72), gap?: pt 0..1000 (12), values?: [hex|rgb|cmyk|lab|gray…] (default: each swatch's own colour model; spot swatches also say spot), x?, y? (the legend's top-left; default: the first artboard's, inset by `gap`)} draw a legend of colour swatches: one square chip per swatch filled with it (linked to it), labelled inside with the swatch name and the values asked for, in black on light chips and white on dark ones, as one group in one undo step; the group is selected → {id, count} (#4919)",
        has_doc,
        legend
    )]
}

/// Most swatches one legend draws.
const MAX_CHIPS: usize = 2000;
const VALUES: [&str; 5] = ["hex", "rgb", "cmyk", "lab", "gray"];

fn number(p: &Value, key: &str, default: f64, range: std::ops::RangeInclusive<f64>, cmd: &str) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && range.contains(n))
            .ok_or_else(|| bad(cmd, format!("`{key}` must be a number {}..{}", range.start(), range.end()))),
    }
}

/// The label lines of colour `c`: the values `values` names, else those of its own model.
fn value_lines(c: Color, values: &[String], spot: bool) -> Vec<String> {
    let own = match c.model() {
        Model::Rgb => "rgb",
        Model::Cmyk => "cmyk",
        Model::Gray => "gray",
        Model::Lab => "lab",
    };
    let picked: Vec<&str> = if values.is_empty() { vec![own] } else { values.iter().map(String::as_str).collect() };
    let pct = |v: f32| (v * 100.0).round() as i32;
    let mut lines: Vec<String> = picked
        .into_iter()
        .map(|v| match v {
            "hex" => c.to_hex().to_uppercase(),
            "rgb" => {
                let [r, g, b] = c.to_rgb().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as i32);
                format!("R={r} G={g} B={b}")
            }
            "cmyk" => {
                let [cy, m, y, k] = c.to_cmyk().map(|v| pct(v.clamp(0.0, 1.0)));
                format!("C={cy} M={m} Y={y} K={k}")
            }
            "lab" => {
                let l = c.to_lab();
                format!("L={} a={} b={}", l.l.round() as i32, l.a.round() as i32, l.b.round() as i32)
            }
            _ => {
                let k = match c {
                    Color::Gray { k } => k,
                    _ => 1.0 - c.to_lab().l / 100.0,
                };
                format!("K={}%", pct(k.clamp(0.0, 1.0)))
            }
        })
        .collect();
    if spot {
        lines.insert(0, "Spot".into());
    }
    lines
}

fn legend(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.legend";
    let columns = number(p, "columns", 4.0, 1.0..=50.0, C)? as usize;
    let size = number(p, "size", 72.0, 8.0..=1000.0, C)?;
    let gap = number(p, "gap", 12.0, 0.0..=1000.0, C)?;
    let values: Vec<String> = match p.get("values") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str().filter(|v| VALUES.contains(v)).map(str::to_string).ok_or_else(|| bad(C, "`values` must name hex, rgb, cmyk, lab or gray"))
            })
            .collect::<Result<_>>()?,
        Some(_) => return Err(bad(C, "`values` must be a list")),
    };
    let st = s.doc()?;
    let doc = &st.doc;
    let colour = |w: &&vectorcraft_color::Swatch| w.paint.color().is_some();
    let swatches: Vec<vectorcraft_color::Swatch> = match p.get("names") {
        None | Some(Value::Null) => doc.swatches_iter().filter(colour).cloned().collect(),
        Some(Value::Array(a)) => {
            let mut out = vec![];
            for n in a {
                let n = n.as_str().ok_or_else(|| bad(C, "`names` must be a list of swatch names"))?;
                let w = doc.swatches_iter().find(|w| w.name == n).ok_or_else(|| bad(C, format!("no swatch named `{n}`")))?;
                if w.paint.color().is_none() {
                    return Err(bad(C, format!("`{n}` is not a colour swatch")));
                }
                out.push(w.clone());
            }
            out
        }
        Some(_) => return Err(bad(C, "`names` must be a list of swatch names")),
    };
    if swatches.is_empty() {
        return Err(bad(C, "there are no colour swatches to draw"));
    }
    if swatches.len() > MAX_CHIPS {
        return Err(bad(C, format!("a legend draws at most {MAX_CHIPS} swatches")));
    }
    let origin = doc.artboards.first().map_or(Point::ZERO, |a| a.rect.origin());
    let x0 = number(p, "x", origin.x + gap, -1e6..=1e6, C)?;
    let y0 = number(p, "y", origin.y + gap, -1e6..=1e6, C)?;
    let system_layout = doc.setup.system_layout;
    let parent = st.target_parent()?;
    let text_size = (size / 7.0).clamp(4.0, 12.0);
    let pad = (size / 12.0).clamp(1.0, 8.0);
    let style = super::create::new_type_style(s, &json!({ "size": text_size }));
    let count = swatches.len();
    let gid = s.edit("Create Swatch Legend", |d, sel| {
        let mut children = Vec::with_capacity(count * 2);
        for (i, w) in swatches.iter().enumerate() {
            let Some(c) = w.paint.color() else { continue };
            let (col, row) = (i % columns, i / columns);
            let (x, y) = (x0 + col as f64 * (size + gap), y0 + row as f64 * (size + gap));
            let mut chip = Node::path(
                d.alloc_id(),
                shapes::rectangle(Rect::new(x, y, x + size, y + size)),
                Appearance::basic(w.paint.clone(), Paint::None, 0.0),
            );
            chip.name = Some(w.name.clone());
            children.push(std::sync::Arc::new(chip));
            let mut lines = vec![w.name.clone()];
            lines.extend(value_lines(c, &values, w.spot));
            let mut label_style = style.clone();
            label_style.fill = Paint::solid(if c.to_lab().l < 50.0 { Color::WHITE } else { Color::BLACK });
            label_style.stroke = Paint::None;
            let mut t = TextObject::point(Point::new(x + pad, y + pad + text_size), &lines.join("\n"), label_style);
            t.para = super::create::new_type_para();
            t.system_layout = system_layout;
            super::typecmd::refresh_bounds(&mut t);
            children.push(std::sync::Arc::new(Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)))));
        }
        let gid = d.alloc_id();
        let mut g = Node::new(gid, NodeKind::Group { children, clip: false });
        g.name = Some("Swatch Legend".into());
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0, "count": count }))
}

#[cfg(test)]
mod tests;
