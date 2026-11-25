//! Object › Step and Repeat: lays out copies of the selection on a grid or fills a sheet, for
//! print-and-cut work (stickers, labels).

use serde_json::{Value, json};
use vectorcraft_geom::{Rect, Vec2, rotate_deg};

use super::edit::{copy_above, selected_roots};
use super::object::group_nodes;
use super::{Affine, CommandSpec, NodeId, Result, Session, bad, check_numbers, check_switches, cmd, has_selection, switch_or};
use crate::EngineError;
use vectorcraft_doc::Scaling;

const CMD: &str = "object.stepAndRepeat";
const MAX_CELLS: usize = 10_000;
const EPS: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "object.stepAndRepeat",
        "Step and Repeat…",
        ["Object", "Transform"],
        None,
        "{rows?: n (default 1), columns?: n (default 1), fill?: bool (rows and columns are as many as fit the sheet, ignoring `rows` and `columns`; the first copy goes to the sheet's top-left corner plus `margin`), sheet?: {x, y, width, height} (pt; default the artboard that holds the selection's center, else the first), margin?: pt (fill only, default 0), gap?: pt (between neighbors, both axes), gapX?: pt, gapY?: pt (default `gap`, default 0; measured between the bounds of `cutPath`, else of the selection, not between centers), cutPath?: id (a selected object whose geometric bounds measure the spacing and the 180° turn; default the selection), rotate?: \"none\"|\"columns\"|\"rows\" (turn every other column or row by 180° about its cell's center, the same clockwise or counter-clockwise; default none), patterns?: bool} copy the selection into a grid of cells, left to right and top to bottom, the first cell being the selection itself (moved to the sheet's corner with fill); a selection of several objects is grouped first, so that a cut contour stays with its artwork in every copy; one undo step; the copies are selected → {ids: [id…], rows, columns, count}",
        has_selection,
        step_and_repeat
    )]
}

/// Which cells of the grid are turned by 180°.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alternate {
    None,
    Columns,
    Rows,
}

/// One cell of a layout: its place in the grid, the offset of its top-left corner from the first
/// cell's, and whether its copy is turned by 180° about the cell's center.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cell {
    pub row: usize,
    pub col: usize,
    pub dx: f64,
    pub dy: f64,
    pub turned: bool,
}

/// How many items of `size` with `gap` between them fit in `room` (all at least 0).
pub fn fit_count(room: f64, size: f64, gap: f64) -> usize {
    if !(room.is_finite() && size.is_finite() && gap.is_finite()) || size <= 0.0 || room < size - EPS {
        return 0;
    }
    (((room + gap) / (size + gap)) + EPS).floor().max(0.0) as usize
}

/// The `(rows, columns)` of items `w` × `h` with gaps `gx`, `gy` that fit a sheet `sw` × `sh`
/// inside `margin` on every side.
pub fn fill_counts(sw: f64, sh: f64, margin: f64, w: f64, h: f64, gx: f64, gy: f64) -> (usize, usize) {
    let m = margin.max(0.0);
    (fit_count(sh - 2.0 * m, h, gy), fit_count(sw - 2.0 * m, w, gx))
}

/// The cells of a `rows` × `cols` grid of items `w` × `h` with gaps `gx` between columns and `gy`
/// between rows, in reading order.
pub fn layout(w: f64, h: f64, gx: f64, gy: f64, rows: usize, cols: usize, alt: Alternate) -> Vec<Cell> {
    (0..rows)
        .flat_map(|row| (0..cols).map(move |col| (row, col)))
        .map(|(row, col)| Cell {
            row,
            col,
            dx: col as f64 * (w + gx),
            dy: row as f64 * (h + gy),
            turned: match alt {
                Alternate::None => false,
                Alternate::Columns => col % 2 == 1,
                Alternate::Rows => row % 2 == 1,
            },
        })
        .collect()
}

fn count_param(p: &Value, key: &str) -> Result<usize> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(1),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=MAX_CELLS as u64).contains(n))
            .map(|n| n as usize)
            .ok_or_else(|| bad(CMD, format!("`{key}` must be a whole number from 1 to {MAX_CELLS}"))),
    }
}

fn sheet_param(p: &Value) -> Result<Option<Rect>> {
    let Some(v) = p.get("sheet").filter(|v| !v.is_null()) else { return Ok(None) };
    let n = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite()).ok_or_else(|| bad(CMD, format!("`sheet.{k}` must be a number")));
    let (x, y, w, h) = (n("x")?, n("y")?, n("width")?, n("height")?);
    if w <= 0.0 || h <= 0.0 {
        return Err(bad(CMD, "`sheet` needs a positive width and height"));
    }
    Ok(Some(Rect::new(x, y, x + w, y + h)))
}

fn step_and_repeat(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["gap", "gapX", "gapY", "margin"], CMD)?;
    check_switches(p, &["patterns"], CMD)?;
    let fill = switch_or(p, "fill", false, CMD)?;
    let alt = match p.get("rotate").filter(|v| !v.is_null()) {
        None => Alternate::None,
        Some(v) => match v.as_str() {
            Some("none") => Alternate::None,
            Some("columns") => Alternate::Columns,
            Some("rows") => Alternate::Rows,
            _ => return Err(bad(CMD, "`rotate` must be \"none\", \"columns\" or \"rows\"")),
        },
    };
    let gap = super::f64_or(p, "gap", 0.0);
    let (gx, gy) = (super::f64_or(p, "gapX", gap), super::f64_or(p, "gapY", gap));
    let margin = super::f64_or(p, "margin", 0.0);
    if !(gx.is_finite() && gy.is_finite() && margin.is_finite()) {
        return Err(bad(CMD, "gaps and margin must be finite"));
    }
    let (rows, cols) = (count_param(p, "rows")?, count_param(p, "columns")?);
    let sheet = sheet_param(p)?;
    let mut roots = selected_roots(s)?;
    if roots.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let d = &s.doc()?.doc;
    let cut = match p.get("cutPath").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => {
            let id = NodeId(v.as_u64().ok_or_else(|| bad(CMD, "`cutPath` must be an object id"))?);
            let held = roots.iter().filter_map(|r| d.node(*r)).any(|n| {
                let mut found = false;
                n.walk(&mut |c| found |= c.id == id);
                found
            });
            if !held {
                return Err(bad(CMD, "`cutPath` must be part of the selection"));
            }
            Some(id)
        }
    };
    let measured = cut.map_or_else(|| roots.clone(), |id| vec![id]);
    let b = d.bounds_of(&measured, false);
    let b = b.ok_or_else(|| EngineError::Other("the selection has no bounds".into()))?;
    let (w, h) = (b.width(), b.height());
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return Err(EngineError::Other("the selection has no area to repeat".into()));
    }
    let anchor = if fill {
        let sheet = sheet.or_else(|| {
            let c = b.center();
            d.artboards.iter().find(|a| a.rect.contains(c)).or_else(|| d.artboards.first()).map(|a| a.rect)
        });
        let sheet = sheet.ok_or_else(|| EngineError::Other("no sheet to fill".into()))?;
        Some((sheet, margin.max(0.0)))
    } else {
        None
    };
    let (rows, cols) = match anchor {
        Some((sh, m)) => fill_counts(sh.width(), sh.height(), m, w, h, gx, gy),
        None => (rows, cols),
    };
    if rows == 0 || cols == 0 {
        return Err(EngineError::Other("the selection does not fit the sheet".into()));
    }
    if rows.saturating_mul(cols) > MAX_CELLS {
        return Err(bad(CMD, format!("more than {MAX_CELLS} copies")));
    }
    let origin = anchor.map_or(Vec2::ZERO, |(sh, m)| Vec2::new(sh.x0 + m - b.x0, sh.y0 + m - b.y0));
    let cells = layout(w, h, gx, gy, rows, cols, alt);
    let center = b.center().to_vec2();
    let place = |c: &Cell| -> Affine {
        let t = Affine::translate(origin + Vec2::new(c.dx, c.dy));
        if c.turned { t * Affine::translate(center) * rotate_deg(180.0) * Affine::translate(-center) } else { t }
    };
    let sc = Scaling { patterns: super::object::transform_patterns(s, p, &roots)?, ..Scaling::default() };
    let ids = s.edit("Step and Repeat", |d, sel| {
        if roots.len() > 1 {
            roots = vec![group_nodes(d, &roots)?];
        }
        let mut copied: Vec<NodeId> = vec![];
        let mut first = Affine::IDENTITY;
        for c in &cells {
            let m = place(c);
            if c.row == 0 && c.col == 0 {
                first = m;
                continue;
            }
            let copies = copy_above(d, &roots, |_, _, n| {
                n.transform(m, sc);
                super::object::keep_pixel_aligned(n, sc);
            })?;
            copied.extend(copies);
        }
        if first != Affine::IDENTITY {
            for id in &roots {
                if let Some(n) = d.node_mut(*id) {
                    n.transform(first, sc);
                    super::object::keep_pixel_aligned(n, sc);
                }
            }
        }
        let mut out: Vec<NodeId> = roots.clone();
        out.extend(copied);
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "rows": rows, "columns": cols, "count": ids.len() }))
}

#[cfg(test)]
mod tests;
