//! Window → CSS Properties: the CSS web pages style objects with ([`vectorcraft_svg::css_rules`],
//! which writes each property as SVG export writes it), for the selection or the whole document,
//! and the `.css` file with the pictures of the art it rasterizes.

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_svg::{CssOptions, CssSheet, css_rules};

use super::fileio::{default_name, encode_with_warnings, isolated, write_file};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "css.selection",
            "CSS Properties",
            [],
            None,
            "{ids?|id?: [id…] (default: the selection), units?: px|pt|mm|cm|in (default px; 1 px = 1 pt, as in SVG export), position?: false (position: absolute, left and top from the artboard's top-left), dimensions?: true (width, height), unnamed?: true (unnamed objects too, classes named after their kind; false: named objects only), rasterize?: false (art CSS can't describe as background-image: url(<class>.png), its picture, which css.export writes)} the CSS web pages style the objects with, one rule per object back to front (layers and plain groups stand for what they hold): a rectangle's or ellipse's fill as background-color or linear-/radial-gradient(), stroke as border, corners as border-radius; type's font, color, spacing and alignment; opacity, mix-blend-mode, shadows and glows (box-shadow, text-shadow), blur (filter) → {css, rules: [{id, selector, css, unsupported?: why CSS can't describe the art exactly (without rasterize it is written as its box), image?: the PNG it refers to}], skipped: n unnamed objects left out, warnings}",
            has_doc,
            selection
        ),
        cmd!(query "css.generate", "Generate CSS", [], None, "{…options as css.selection} css.selection of every object in the document", has_doc, generate),
        cmd!(
            "css.export",
            "Export CSS",
            [],
            None,
            "{path?, scope?: selection|all (default selection; ids? picks the objects), …options as css.selection, id?: object id (default: the selection)} write the CSS (css.selection, or css.generate for all) to path and, with rasterize, each picture its rules refer to next to it (PNG of the object's visual bounds, 1 px per point) → {path, bytes, rules: n, images: [path…], warnings}; no path → {data: the CSS, name: <document>.css, rules, images: [{name, dataBase64}], warnings}; warnings are css.selection's and the pictures' as document.export gives them (an image that couldn't be decoded, type in the fallback font or that doesn't fit its frame), each once",
            has_doc,
            export
        ),
    ]
}

/// The CSS options of `p` (its other params are the command's own).
fn options(p: &Value, cmd: &str) -> Result<CssOptions> {
    if !p.is_object() {
        return Ok(CssOptions::default());
    }
    CssOptions::deserialize(p).map_err(|e| bad(cmd, format!("CSS options: {e}")))
}

/// The CSS of objects `ids` (back to front) with the options of `p`.
fn sheet(s: &Session, p: &Value, ids: Vec<NodeId>, cmd: &str) -> Result<CssSheet> {
    let opts = options(p, cmd)?;
    let d = &s.doc()?.doc;
    Ok(css_rules(d, &d.paint_order(ids), &opts))
}

/// Every object of the active document: its layers.
fn everything(s: &Session) -> Result<Vec<NodeId>> {
    Ok(s.doc()?.doc.layers.iter().map(|l| l.id).collect())
}

fn result(sheet: &CssSheet) -> Value {
    let rules: Vec<Value> = sheet
        .rules
        .iter()
        .map(|r| {
            let mut v = json!({ "id": r.id.0, "selector": r.selector, "css": r.text() });
            if let Some(why) = r.unsupported {
                v["unsupported"] = json!(why);
            }
            if let Some(image) = &r.image {
                v["image"] = json!(image);
            }
            v
        })
        .collect();
    json!({ "css": sheet.text(), "rules": rules, "skipped": sheet.skipped, "warnings": sheet.warnings })
}

fn selection(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    Ok(result(&sheet(s, p, ids, "css.selection")?))
}

fn generate(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = everything(s)?;
    Ok(result(&sheet(s, p, ids, "css.generate")?))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "css.export";
    let ids = match str_param(p, "scope").unwrap_or("selection") {
        "selection" => targets(s, p)?,
        "all" => everything(s)?,
        other => return Err(bad(C, format!("scope must be selection or all (got {other})"))),
    };
    let sheet = sheet(s, p, ids, C)?;
    if sheet.rules.is_empty() {
        let why = if sheet.skipped > 0 {
            "the objects are unnamed: name them, or give unnamed: true"
        } else {
            "no objects to export: select some, or give scope: all"
        };
        return Err(bad(C, why));
    }
    let css = format!("{}\n", sheet.text());
    let doc = &s.doc()?.doc;
    // Encode the pictures of rasterized art, and add each of their warnings once.
    let mut images = Vec::new();
    let mut warnings = sheet.warnings.clone();
    for r in &sheet.rules {
        let Some(name) = &r.image else { continue };
        let (d, _) = isolated(doc, &[r.id], name).ok_or_else(|| bad(C, format!("{} paints nothing to rasterize", r.selector)))?;
        let (png, notes) = encode_with_warnings(&d, "png", &json!({}))?;
        images.push((name, png));
        for w in notes {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
    }
    let n = sheet.rules.len();
    match str_param(p, "path") {
        Some(path) => {
            write_file(path, css.as_bytes())?;
            let folder = std::path::Path::new(path).parent();
            let mut written = Vec::new();
            for (name, bytes) in &images {
                let at = folder.map_or_else(|| name.to_string(), |f| f.join(name).to_string_lossy().into_owned());
                write_file(&at, bytes)?;
                written.push(at);
            }
            Ok(json!({ "path": path, "bytes": css.len(), "rules": n, "images": written, "warnings": warnings }))
        }
        None => {
            let images: Vec<Value> =
                images.iter().map(|(name, bytes)| json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).collect();
            Ok(json!({ "data": css, "name": default_name(doc, "css"), "rules": n, "images": images, "warnings": warnings }))
        }
    }
}
