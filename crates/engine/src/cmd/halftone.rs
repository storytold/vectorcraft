//! Object → Vector Halftone: the selected art (vectors, type, images) as vector dots, lines or
//! shapes sized by its tone, as one undo step.
//!
//! The screens, dots and caps are Color Effects' Vector Halftone's
//! ([`vectorcraft_render::effects::addons::halftone`]), which draws the same dots live; the art is
//! sampled as [`vectorcraft_render::addons::halftone::sample`] draws it.

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::Node;
use vectorcraft_render::effects::addons::halftone::{DotShape, Halftone, art_outline, dot_nodes};

use super::edit::selected_roots;
use super::{CommandSpec, EngineError, Result, Session, bad, bool_or, cmd, color_value, f64_or, has_selection, str_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "object.vectorHalftone",
        "Vector Halftone…",
        ["Object"],
        None,
        "{shape?: \"circle\"|\"ellipse\"|\"square\"|\"diamond\"|\"line\" (circle), frequency?: lines per inch 1–300 (20), angle?: deg (45; CMYK: the black screen's, cyan −30°, magenta +30°, yellow −45° from it), mode?: \"mono\"|\"cmyk\" (mono), color?: mono dot colour (\"#000000\"), invert?: bool (dots for light tones), clip?: true (clip the dots to the art's outline), keepOriginal?: false} the selection as vector halftone dots sized by its tone, a group of one path per screen (CMYK screens multiply), as one undo step → {id, dots, warning? (an image in the art couldn't be decoded, as object.rasterize says)}",
        has_selection,
        halftone
    )]
}

fn halftone(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.vectorHalftone";
    let shape = match str_param(p, "shape") {
        Some(v) => DotShape::parse(v).ok_or_else(|| bad(C, format!("unknown shape `{v}` (circle|ellipse|square|diamond|line)")))?,
        None => DotShape::Circle,
    };
    let frequency = f64_or(p, "frequency", 20.0);
    if !(1.0..=300.0).contains(&frequency) {
        return Err(bad(C, "frequency must be between 1 and 300 lines per inch"));
    }
    let angle = f64_or(p, "angle", 45.0);
    if !angle.is_finite() {
        return Err(bad(C, "invalid angle"));
    }
    let mono = match str_param(p, "mode").unwrap_or("mono") {
        "mono" => true,
        "cmyk" => false,
        other => return Err(bad(C, format!("unknown mode `{other}` (mono|cmyk)"))),
    };
    let color = match p.get("color") {
        Some(v) => color_value(v).ok_or_else(|| bad(C, "invalid color"))?,
        None => Color::BLACK,
    };
    let (invert, clip, keep) = (bool_or(p, "invert", false), bool_or(p, "clip", true), bool_or(p, "keepOriginal", false));
    let ht = Halftone { shape, frequency, angle, mono, color, from_art: false, invert, clip };

    let roots = selected_roots(s)?;
    let st = s.doc()?;
    let b = st.doc.bounds_of(&roots, true).ok_or_else(|| bad(C, "the selection has no bounds"))?;
    if b.width() <= 0.0 || b.height() <= 0.0 || !b.area().is_finite() {
        return Err(bad(C, "the selection has no area"));
    }
    let (region, scale) = ht.plan(b).ok_or_else(|| bad(C, "too many dots for this size: lower the frequency"))?;
    let nodes: Vec<Node> = roots
        .iter()
        .filter_map(|id| st.doc.node(*id).cloned())
        .map(|mut n| {
            n.visible = true;
            n
        })
        .collect();
    let outline = if clip { art_outline(&nodes) } else { None };
    let failed = vectorcraft_render::decode_failures();
    let samples = vectorcraft_render::addons::halftone::sample(&st.doc, nodes, region, scale, mono, invert);
    let warning = super::rasterfx::undecoded_warning(failed);
    let samples = samples.ok_or_else(|| EngineError::Other("the art could not be sampled".into()))?;
    let (layers, dots) = ht.layers(&samples, b, outline.as_ref());
    if layers.is_empty() {
        return Err(EngineError::Other("the art is too light for any dots".into()));
    }
    let top = *roots.last().ok_or_else(|| bad(C, "nothing selected"))?;
    let id = s.edit("Vector Halftone", |d, sel| {
        let children = dot_nodes(layers, mono, &mut || d.alloc_id());
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = Some("Vector Halftone".into());
        if keep {
            let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
            d.insert(par, idx + 1, g)?;
        } else {
            d.wrap(&roots, |_| g)?;
        }
        sel.set([gid]);
        Ok(gid)
    })?;
    let mut out = json!({ "id": id.0, "dots": dots });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}
