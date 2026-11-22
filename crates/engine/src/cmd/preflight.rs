//! Production preflight: the existing output-aware type walk, linked-file probes and image
//! resolution calculation, returned as findings that the interface can locate and recheck.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_text::{FontDb, FontMatch};

use super::{CommandSpec, Result, Session, bad, cmd, has_doc, number_or, text_param};

const COMMAND: &str = "document.preflight";
const KINDS: [&str; 4] = ["fonts", "text", "links", "resolution"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "document.preflight",
        "Production Preflight",
        [],
        None,
        "{output?: pdf|print|screen (pdf), minPpi?: 1..9600 (300, or 96 for screen), artboards?: [index, …] (0-based whole numbers; default all)} check the selected output without writing files or editing the document; print reuses the saved print settings with these artboards, pdf and screen check visible artwork on these artboards. Reuses output-aware font and overflow checks, Links file status and image.info effective resolution. Resource findings locate their first visible canvas owner; resolution inside resource art is reported as not checked because its placed scale can differ. → {output, minPpi, artboards, checks: [{kind: fonts|text|links|resolution, status: passed|failed|notChecked, count, notChecked}], findings: [{kind, code, ids (canvas objects), name, detail?, sourceId?, effectivePpi?, threshold?}], outputWarnings, limitations}; a pass covers only these checks, not certification of a produced file",
        has_doc,
        check
    )]
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let output = text_param(p, "output", COMMAND)?.unwrap_or("pdf");
    if !matches!(output, "pdf" | "print" | "screen") {
        return Err(bad(COMMAND, "output must be pdf, print or screen"));
    }
    let min_ppi = number_or(p, "minPpi", if output == "screen" { 96.0 } else { 300.0 }, COMMAND)?;
    if !min_ppi.is_finite() || !(1.0..=9600.0).contains(&min_ppi) {
        return Err(bad(COMMAND, "minPpi must be from 1 to 9600"));
    }
    let indices = match p.get("artboards").filter(|v| !v.is_null()) {
        Some(v) => super::panelcmds::listed_artboards(&s.doc()?.doc, v, "artboards", COMMAND)?,
        None => (0..s.doc()?.doc.artboards.len()).collect(),
    };
    let context = if output == "print" {
        super::print::preflight_context(s, &indices)?
    } else {
        let doc = &s.doc()?.doc;
        let pages = indices.iter().filter_map(|i| doc.artboards.get(*i).map(|a| (a.rect, vectorcraft_geom::Affine::IDENTITY))).collect();
        super::print::PreflightContext { layers: vectorcraft_pdf::PrintLayers::Visible, pages, warnings: vec![] }
    };
    let areas = context.pages.iter().map(|(r, _)| *r).collect::<Vec<_>>();
    let st = s.doc()?;
    let doc = st.doc.with_placed_art();
    let mut layers = doc.layers.clone();
    vectorcraft_pdf::keep_layers(&mut layers, context.layers);
    let typed = super::fonts::preflight_type(&doc, &layers, &areas);
    let statuses = super::links::statuses(st);
    let mut findings = vec![];
    let db = FontDb::global();
    for (font, ids) in typed.fonts {
        let code = match db.resolve(&font.0, &font.1).map(|(_, m)| m) {
            None | Some(FontMatch::Missing) => "missingFont",
            Some(FontMatch::Style) => "substitutedFont",
            _ => continue,
        };
        findings.push(
            json!({ "kind": "fonts", "code": code, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "name": super::fonts::font_label(&font) }),
        );
    }
    for id in typed.overflow {
        findings.push(finding("text", "overflow", id, &doc));
    }
    let mut resolution_unchecked = 0usize;
    for (source, owner) in typed.images {
        if let Some((status, _, _)) = statuses.get(&source) {
            let code = match status.as_str() {
                "missing" => Some("missingLink"),
                "modified" => Some("modifiedLink"),
                _ => None,
            };
            if let Some(code) = code {
                let mut f = finding("links", code, owner, &doc);
                f["sourceId"] = json!(source.0);
                findings.push(f);
            }
        }
        // A vector placed document has no single bitmap resolution. Its embedded artwork, and
        // resources with their own coordinate systems, must never be certified at a local scale.
        if doc.node(source).is_none() || source != owner {
            resolution_unchecked += 1;
            continue;
        }
        let Some(vectorcraft_doc::NodeKind::Image(im)) = doc.node(source).map(|n| &n.kind) else {
            resolution_unchecked += 1;
            continue;
        };
        let bounds = doc.node(source).and_then(|n| vectorcraft_render::painted_bounds(&doc, n));
        // A distorted image is measured at its middle, as Link Info measures it.
        let xf = doc.node(source).and_then(vectorcraft_doc::image_quad::nearest_affine).unwrap_or(im.xf);
        let ppi = context
            .pages
            .iter()
            .filter(|(area, _)| bounds.is_some_and(|b| area.intersect(b).area() > 0.0))
            .map(|(_, to_page)| super::place::effective_ppi(*to_page * xf))
            .reduce(|[a, b], [c, d]| [a.min(c), b.min(d)]);
        let Some(ppi) = ppi else {
            resolution_unchecked += 1;
            continue;
        };
        // Inverting the placed-size transform can round an exact threshold slightly down.
        if ppi.iter().any(|n| !n.is_finite() || *n <= 0.0) {
            resolution_unchecked += 1;
        } else if ppi.iter().any(|n| *n < min_ppi * (1.0 - 1e-9)) {
            let mut f = finding("resolution", "lowResolution", owner, &doc);
            f["effectivePpi"] = json!(ppi);
            f["threshold"] = json!(min_ppi);
            findings.push(f);
        }
    }
    let checks = KINDS.map(|kind| {
        let count = findings.iter().filter(|f| f["kind"] == kind).count();
        let not_checked = if kind == "resolution" { resolution_unchecked } else { 0 };
        let status = if count > 0 {
            "failed"
        } else if not_checked > 0 {
            "notChecked"
        } else {
            "passed"
        };
        json!({ "kind": kind, "status": status, "count": count, "notChecked": not_checked })
    });
    Ok(json!({
        "output": output, "minPpi": min_ppi, "artboards": indices, "checks": checks,
        "findings": findings, "outputWarnings": context.warnings,
        "limitations": ["These checks do not certify an exported file.", "Resolution inside symbols, patterns, brushes, opacity masks and placed documents is not checked."]
    }))
}

fn finding(kind: &str, code: &str, id: NodeId, doc: &vectorcraft_doc::Document) -> Value {
    json!({ "kind": kind, "code": code, "ids": [id.0], "name": doc.node(id).map(|n| n.display_name()).unwrap_or_else(|| id.0.to_string()) })
}
