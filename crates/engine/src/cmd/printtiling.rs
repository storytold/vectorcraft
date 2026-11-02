//! View → Show Print Tiling (`view.printTiling`) and the Print Tiling tool (`print.tiling.set`):
//! the pages of the document's print settings drawn on the canvas, from `print.preview`'s layout
//! (`print.tiling`), and where the tool put them ([`vectorcraft_pdf::TileOrigin`], saved with
//! the print settings).

use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_geom::{Point, Rect};
use vectorcraft_pdf::{TileOrigin, TilingPage};

use super::fileio::pdf::pdf_error;
use super::print::{settings, to_json};
use super::{CommandSpec, Result, Session, bad, bool_or, cmd, has_doc, index};

const SET: &str = "print.tiling.set";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "view.printTiling",
            "Show Print Tiling",
            ["View"],
            None,
            "{on?: bool (default: toggle)} show the pages of the document's print settings on the canvas (each document has its own setting; the Print Tiling tool shows them too) → {on}",
            has_doc,
            show
        ),
        cmd!(
            "print.tiling.set",
            "Print Tiling",
            [],
            None,
            "{origin: [x, y] (document point: the top-left corner of the first page's imageable area), artboard? (0-based: the artboard it is measured from; default the printed artboard containing origin, its edges included, else the first; ignored with artboards: ignore)} | {reset: true} place the printed pages as the Print Tiling tool does, saved with the print settings (tileOrigin: {placed, x, y}, from the artboard's top-left corner; it replaces the placement, and tiles start there), as one undo step; reset → the placement places them again → {tileOrigin}",
            has_doc,
            set_origin
        ),
        cmd!(
            query "print.tiling",
            "Print Tiling Pages",
            [],
            None,
            "{settings?: {…settings of print.setup} (over the document's)} the pages as View → Show Print Tiling draws them, from print.preview's layout → {tileOrigin, pages: [{artboard (0-based; null with artboards ignored), number (1-based tile across then down, or page), printed (tiles outside tileRange don't), page: [x0, y0, x1, y1] (the paper's edge), imageable: [x0, y0, x1, y1] (inside the unprintable margin)}] (document space; every tile when tiling, else each page once)}",
            has_doc,
            tiling
        ),
    ]
}

fn show(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    st.print_tiling = p.get("on").and_then(Value::as_bool).unwrap_or(!st.print_tiling);
    Ok(json!({ "on": st.print_tiling }))
}

/// The pages of `doc`'s print settings in document space (what View → Show Print Tiling draws);
/// an error when they can't print.
pub fn pages(doc: &Document) -> Result<Vec<TilingPage>> {
    pages_with(doc, &json!({}))
}

fn pages_with(doc: &Document, p: &Value) -> Result<Vec<TilingPage>> {
    const C: &str = "print.tiling";
    let set = settings(C, doc, p)?;
    let pv = vectorcraft_pdf::preview(doc, &set).map_err(|e| pdf_error(C, e))?;
    Ok(vectorcraft_pdf::tiling(&pv, &set))
}

fn tiling(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let origin = settings("print.tiling", doc, p)?.tile_origin;
    Ok(json!({ "tileOrigin": origin, "pages": pages_with(doc, p)? }))
}

/// `[x, y]` at `key` of `p`.
fn point(p: &Value, key: &str) -> Option<Point> {
    let a = p.get(key)?.as_array()?;
    Some(Point::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?))
}

/// The first of the print `regions` holding `at`: one that holds it inside or on its left or top
/// edge, else one that holds it on its right or bottom edge or within a millionth of a point
/// outside an edge (the edges `Document::art_on_artboard` counts).
fn region_holding(regions: &[(Option<usize>, Rect)], at: Point) -> Option<&(Option<usize>, Rect)> {
    regions.iter().find(|(_, r)| r.contains(at)).or_else(|| regions.iter().find(|(_, r)| vectorcraft_doc::artboard_holds(*r, at)))
}

fn set_origin(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let current = settings(SET, doc, &json!({}))?;
    let mut set = current.clone();
    set.tile_origin = if bool_or(p, "reset", false) {
        TileOrigin::default()
    } else {
        let at = point(p, "origin").filter(|q| q.x.is_finite() && q.y.is_finite());
        let at = at.ok_or_else(|| bad(SET, "give origin: [x, y], or reset: true"))?;
        let regions = vectorcraft_pdf::print_regions(doc, &set).map_err(|e| pdf_error(SET, e))?;
        let picked = match p.get("artboard").filter(|v| !v.is_null()) {
            Some(v) => {
                let i = v.as_u64().map(index).ok_or_else(|| bad(SET, "artboard must be a 0-based index"))?;
                regions.iter().find(|(a, _)| a.is_none() || *a == Some(i)).ok_or_else(|| bad(SET, format!("artboard {i} doesn't print")))?
            }
            None => region_holding(&regions, at).or(regions.first()).ok_or_else(|| bad(SET, "nothing prints"))?,
        };
        let corner = picked.1.origin();
        TileOrigin { placed: true, x: at.x - corner.x, y: at.y - corner.y }
    };
    set.check().map_err(|e| pdf_error(SET, e))?;
    if set != current {
        let stored = to_json(&set)?;
        s.edit("Print Tiling", |d, _| {
            d.print_setup = Some(stored);
            Ok(())
        })?;
    }
    Ok(json!({ "tileOrigin": set.tile_origin }))
}
