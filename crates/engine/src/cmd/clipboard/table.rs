//! Cells copied from a spreadsheet (Excel, Google Sheets, LibreOffice Calc, Numbers): the plain
//! text they put on the clipboard is tab-separated values, one row a line, every row with the
//! same number of cells. `clipboard.importText` pastes such text as a table: a group of point
//! type, one object for each cell that holds text, laid out in columns as wide as their widest
//! cell and rows as tall as their tallest, over gridlines around every cell.

use std::sync::Arc;

use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect};

/// Most rows a pasted table takes; more pastes as plain text.
pub(crate) const MAX_ROWS: usize = 1000;
/// Most columns a pasted table takes; more pastes as plain text.
pub(crate) const MAX_COLUMNS: usize = 256;
/// Most cells (rows × columns) a pasted table takes; more pastes as plain text.
pub(crate) const MAX_CELLS: usize = 20_000;
/// Space between a cell's text and its gridlines, in points.
const PAD: f64 = 4.0;
/// The gridlines' stroke weight, in points.
const GRIDLINE: f64 = 0.5;

/// The rows of `text` (line breaks already `\n`, trailing ones dropped) when it is a table copied
/// from a spreadsheet: tab-separated cells, at least two columns, every row with the same number
/// of cells, within the caps above. A cell in double quotes may hold tabs, line breaks and
/// doubled quotes (`""`), as the spreadsheets write them. `None` for any other text, which pastes
/// as it always has.
pub(crate) fn parse(text: &str) -> Option<Vec<Vec<String>>> {
    if !text.contains('\t') {
        return None;
    }
    let mut rows: Vec<Vec<String>> = vec![];
    let mut row: Vec<String> = vec![];
    let mut cell = String::new();
    let mut chars = text.chars().peekable();
    // At the start of a cell, a quote opens a quoted cell.
    let mut start = true;
    let mut quoted = false;
    let mut cells = 0usize;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' if start => {
                quoted = true;
                start = false;
            }
            '\t' | '\n' => {
                row.push(std::mem::take(&mut cell));
                cells += 1;
                if row.len() > MAX_COLUMNS || cells > MAX_CELLS {
                    return None;
                }
                if c == '\n' {
                    rows.push(std::mem::take(&mut row));
                    if rows.len() > MAX_ROWS {
                        return None;
                    }
                }
                start = true;
            }
            c => {
                cell.push(c);
                start = false;
            }
        }
    }
    // A quote left open is not a spreadsheet's.
    if quoted {
        return None;
    }
    row.push(cell);
    rows.push(row);
    if rows.iter().any(|r| r.len() > MAX_COLUMNS) || cells + 1 > MAX_CELLS {
        return None;
    }
    let columns = rows.first()?.len();
    let fits = columns >= 2 && rows.len() <= MAX_ROWS && rows.iter().all(|r| r.len() == columns);
    // The first and last columns hold something: text indented with tabs is not a table.
    let used = |k: usize| rows.iter().any(|r| r.get(k).is_some_and(|c| !c.trim().is_empty()));
    (fits && used(0) && used(columns - 1)).then_some(rows)
}

/// `rows` laid out as a table (see the module docs) with its top left at the origin: one group
/// holding a group of gridlines and the cells' type. `style` is the type's style; `layout`
/// refreshes a type object's bounds.
pub(crate) fn table(rows: &[Vec<String>], style: &CharStyle, layout: impl Fn(&mut TextObject)) -> Node {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut next = 0u64;
    let mut id = || {
        next += 1;
        NodeId(next)
    };
    // Each cell's type, at the origin, with its bounds.
    let mut cells: Vec<Vec<Option<(Node, Rect)>>> = vec![];
    for row in rows {
        let mut out = vec![];
        for text in row {
            if text.trim().is_empty() {
                out.push(None);
                continue;
            }
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            let mut t = TextObject::point(Point::ZERO, &text, style.clone());
            layout(&mut t);
            let n = Node::new(id(), NodeKind::Text(Box::new(t)));
            out.push(n.geometric_bounds().filter(|b| b.is_finite()).map(|b| (n, b)));
        }
        cells.push(out);
    }
    // A row with nothing in it is as tall as one line of the type.
    let line = style.size.max(1.0) * 1.2;
    let widths: Vec<f64> =
        (0..columns).map(|k| cells.iter().filter_map(|r| r.get(k)?.as_ref()).map(|(_, b)| b.width()).fold(line, f64::max) + 2.0 * PAD).collect();
    let heights: Vec<f64> = cells.iter().map(|r| r.iter().flatten().map(|(_, b)| b.height()).fold(line, f64::max) + 2.0 * PAD).collect();
    let xs: Vec<f64> = std::iter::once(0.0)
        .chain(widths.iter().scan(0.0, |x, w| {
            *x += w;
            Some(*x)
        }))
        .collect();
    let ys: Vec<f64> = std::iter::once(0.0)
        .chain(heights.iter().scan(0.0, |y, h| {
            *y += h;
            Some(*y)
        }))
        .collect();
    let (right, bottom) = (xs.last().copied().unwrap_or(0.0), ys.last().copied().unwrap_or(0.0));
    let stroke = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), GRIDLINE);
    let mut lines = vec![];
    for y in &ys {
        lines.push(segment(id(), (0.0, *y), (right, *y), &stroke));
    }
    for x in &xs {
        lines.push(segment(id(), (*x, 0.0), (*x, bottom), &stroke));
    }
    let mut grid = Node::group(id(), lines);
    grid.name = Some("Gridlines".into());
    let mut children = vec![Arc::new(grid)];
    for (r, row) in cells.into_iter().enumerate() {
        for (k, cell) in row.into_iter().enumerate() {
            let (Some((mut n, b)), Some(x), Some(y)) = (cell, xs.get(k), ys.get(r)) else { continue };
            // The text's top left a pad inside the cell's.
            n.transform(Affine::translate((x + PAD - b.x0, y + PAD - b.y0)), false);
            children.push(Arc::new(n));
        }
    }
    let mut group = Node::group(id(), children);
    group.name = Some("Table".into());
    group
}

/// A straight stroked line from `a` to `b`.
fn segment(id: NodeId, a: (f64, f64), b: (f64, f64), stroke: &Appearance) -> Arc<Node> {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    Arc::new(Node::path(id, PathData::from_bezpath(&p), stroke.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreadsheet_cells_are_told_from_other_text() {
        let rows = parse("Name\tQty\nApples\t3\nPears\t").unwrap();
        assert_eq!(rows, vec![vec!["Name", "Qty"], vec!["Apples", "3"], vec!["Pears", ""]]);
        // Quoted cells hold line breaks, tabs and quotes.
        let rows = parse("\"Two\nlines\"\t\"a\tb\"\n\"say \"\"hi\"\"\"\tx").unwrap();
        assert_eq!(rows, vec![vec!["Two\nlines", "a\tb"], vec!["say \"hi\"", "x"]]);
        // Not tables: no tabs, ragged rows, indented text, an unclosed quote.
        for t in ["plain words", "one\ntwo", "a\tb\nc", "\tindented\n\tcode", "a\t\"open", "x\t"] {
            assert_eq!(parse(t), None, "{t:?}");
        }
        // A quote inside a cell is just a character.
        assert_eq!(parse("5\" pipe\t2").unwrap(), vec![vec!["5\" pipe", "2"]]);
    }

    #[test]
    fn tables_past_the_caps_paste_as_text() {
        let wide = vec!["x"; MAX_COLUMNS + 1].join("\t");
        assert_eq!(parse(&wide), None);
        let tall = vec!["a\tb"; MAX_ROWS + 1].join("\n");
        assert_eq!(parse(&tall), None);
        let many = vec![vec!["c"; 100].join("\t"); MAX_CELLS / 100 + 1].join("\n");
        assert_eq!(parse(&many), None);
        let fits = vec!["a\tb"; MAX_ROWS].join("\n");
        assert_eq!(parse(&fits).map(|r| r.len()), Some(MAX_ROWS));
    }
}
