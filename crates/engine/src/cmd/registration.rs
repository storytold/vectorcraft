//! File → Registration: cutter registration marks built in (no plug-in), each system's marks on
//! a layer of their own above the artwork so they can be hidden, locked or printed on their own.
//!
//! - **Summa OPOS** (`registration.summa`): solid black squares in a row below the art (the
//!   origin mark at its lower left) and a row above it, at regular X intervals, on the layer
//!   "Regmark" (the layer Summa's own plug-in makes); OPOS XY adds a 3 mm bar along the bottom
//!   row between the marks (the cutter measures bowing along it), XY2 bars along both rows,
//!   Random XY bars along both rows and up the outer columns.
//! - **Zünd** (`registration.zund`): solid black dots at the four corners around the art plus a
//!   fifth on the bottom edge, off-centre so the camera can tell the job's orientation.
//!
//! The marks surround the selection (all artwork when nothing is selected, the marks layers left
//! out) with a gap. Running a command again replaces its layer's marks. Sizes are in points.
//!
//! Numbers from the makers' manuals (Summa S Class / DC5 user manuals, OPOS chapter): mark a
//! black square, 1.5 to 10 mm (3 mm advised), no outline; white space around each mark 3 to 4
//! times its size; X distance about 400 mm recommended, 1300 mm at most; the XY line within 20 mm
//! of the marks' centres and 10 mm clear of them. Checked against a file made by Summa's
//! Illustrator plug-in on Alex's PC: layer "Regmark", 3 mm squares of 100% K with no stroke, a
//! bottom bar between the marks as high as a mark and about 10 mm short of each.
//! Zünd: black dots on a layer named "Register" (Zünd Cut Center's register layer name), 1/4 in
//! dots in common use. The Random XY layout and the fifth dot's place are this app's choices.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Point, Rect, shapes};

use super::*;

/// Points per millimetre.
pub const MM: f64 = 72.0 / 25.4;

/// Summa OPOS mark size limits and default, in mm (the manual: 1.5 to 10 mm; 3 mm advised).
pub const SUMMA_SIZE_MM: (f64, f64, f64) = (1.5, 10.0, 3.0);
/// The white space a Summa mark needs around it, as a multiple of its size (the manual: 3 to 4).
pub const SUMMA_CLEAR: f64 = 3.0;
/// Summa X distance: recommended (the default largest step between marks) and the maximum, mm.
pub const SUMMA_X_DISTANCE_MM: (f64, f64) = (400.0, 1300.0);
/// The OPOS XY line: thickness and the gap to the marks at its ends, mm (Summa Cutter Control's
/// profile has a 3 mm XY width; its plug-in stops the line about 10 mm short of each mark).
pub const SUMMA_LINE_MM: (f64, f64) = (3.0, 10.0);

/// Zünd dot diameter limits and default, in mm. Alex asked for 0.2 to 0.4; read as inches
/// (5.08 to 10.16 mm), which takes in the 1/4 in dots Zünd shops use. Change the unit here.
pub const ZUND_DOT_MM: (f64, f64, f64) = (0.2 * 25.4, 0.4 * 25.4, 0.25 * 25.4);
/// The default gap between the artwork and the Zünd dots' edges, mm.
pub const ZUND_GAP_MM: f64 = 10.0;
/// Where the fifth Zünd dot sits on the bottom edge: this fraction of the way from the
/// bottom-right dot toward the bottom-left one.
pub const ZUND_FIFTH: f64 = 0.25;

/// The layer each system's marks go on.
pub const SUMMA_LAYER: &str = "Regmark";
pub const ZUND_LAYER: &str = "Register";

/// The Summa OPOS methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OposMode {
    /// Marks only (OPOS X).
    Opos,
    /// A line between the front marks.
    Xy,
    /// Lines between the front and the rear marks.
    Xy2,
    /// A line between every column of marks.
    RandomXy,
}

impl OposMode {
    pub const ALL: [OposMode; 4] = [OposMode::Opos, OposMode::Xy, OposMode::Xy2, OposMode::RandomXy];
    pub fn id(self) -> &'static str {
        match self {
            OposMode::Opos => "opos",
            OposMode::Xy => "oposXY",
            OposMode::Xy2 => "oposXY2",
            OposMode::RandomXy => "oposRandomXY",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            OposMode::Opos => "OPOS Marks",
            OposMode::Xy => "OPOS XY Marks",
            OposMode::Xy2 => "OPOS XY 2 Marks",
            OposMode::RandomXy => "OPOS Random XY",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(s))
    }
}

/// Where the Summa marks go (document space, points).
#[derive(Clone, Debug, PartialEq)]
pub struct SummaLayout {
    /// The marks: the origin first (lower left), then the bottom row left to right, then the top row.
    /// The XY lines follow in `lines`.
    pub marks: Vec<Rect>,
    /// The XY lines.
    pub lines: Vec<Rect>,
    /// From one mark's corner to the next along X, and between the rows.
    pub x_distance: f64,
    pub y_distance: f64,
}

/// The Summa layout around `art`: marks `size` square, `gap` clear of the art, at most `max_step`
/// apart along X.
pub fn summa_layout(art: Rect, mode: OposMode, size: f64, gap: f64, max_step: f64) -> std::result::Result<SummaLayout, String> {
    if ![art.x0, art.y0, art.x1, art.y1, size, gap, max_step].iter().all(|v| v.is_finite()) || size <= 0.0 || max_step <= 0.0 {
        return Err("the sizes must be positive numbers".into());
    }
    let left = art.x0 - gap - size;
    let right = art.x1 + gap;
    let top = art.y0 - gap - size;
    let bottom = art.y1 + gap;
    let span = right - left;
    // Capped so a huge document can't ask for millions of marks.
    let steps = (span / max_step).ceil().clamp(1.0, 1000.0) as usize;
    let step = span / steps as f64;
    let xs: Vec<f64> = (0..=steps).map(|i| left + step * i as f64).collect();
    let square = |x: f64, y: f64| Rect::new(x, y, x + size, y + size);
    let mut marks: Vec<Rect> = xs.iter().map(|x| square(*x, bottom)).collect();
    marks.extend(xs.iter().map(|x| square(*x, top)));
    let (thick, clear) = (SUMMA_LINE_MM.0 * MM, SUMMA_LINE_MM.1 * MM);
    let n = xs.len();
    let (bottom_row, top_row) = (marks.get(..n).unwrap_or_default(), marks.get(n..).unwrap_or_default());
    // A bar along a row, between each pair of neighbouring marks, `clear` short of both.
    let row_lines = |row: &[Rect]| -> Vec<Rect> {
        row.windows(2)
            .filter_map(|w| match w {
                [a, b] => Some(Rect::new(a.x1 + clear, a.center().y - thick / 2.0, b.x0 - clear, a.center().y + thick / 2.0)),
                _ => None,
            })
            .collect()
    };
    // A bar up a column, between its bottom and top marks.
    let column_line = |x: f64| Rect::new(x + (size - thick) / 2.0, top + size + clear, x + (size + thick) / 2.0, bottom - clear);
    let mut lines = match mode {
        OposMode::Opos => vec![],
        OposMode::Xy => row_lines(bottom_row),
        OposMode::Xy2 | OposMode::RandomXy => [row_lines(bottom_row), row_lines(top_row)].concat(),
    };
    if mode == OposMode::RandomXy {
        lines.extend(xs.first().into_iter().chain(xs.last()).map(|x| column_line(*x)));
    }
    if lines.iter().any(|l| l.width() < thick - 1e-9 || l.height() < thick - 1e-9) {
        return Err(format!(
            "the marks are too close for the {} lines: they need at least {:.0} mm between them",
            mode.label(),
            (2.0 * clear + thick) / MM
        ));
    }
    Ok(SummaLayout { marks, lines, x_distance: step, y_distance: bottom - top })
}

/// The Zünd dot centres around `art`: the four corners (top left, top right, bottom right, bottom
/// left), then the fifth on the bottom edge, `fifth` of the way from the bottom-right dot toward the
/// bottom-left one.
pub fn zund_layout(art: Rect, diameter: f64, gap: f64, fifth: f64) -> Vec<Point> {
    let r = diameter / 2.0;
    let (l, rt) = (art.x0 - gap - r, art.x1 + gap + r);
    let (t, b) = (art.y0 - gap - r, art.y1 + gap + r);
    vec![Point::new(l, t), Point::new(rt, t), Point::new(rt, b), Point::new(l, b), Point::new(rt - fifth * (rt - l), b)]
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "registration.summa",
            "Summa OPOS Marks",
            [],
            None,
            "{mode?: opos|oposXY|oposXY2|oposRandomXY (opos), size?: pt (mark side, 1.5 to 10 mm; 3 mm), gap?: pt (art to marks; at least 3× the size, default 4×), xDistance?: pt (largest step between marks along X, up to 1300 mm; 400 mm), ids?} add Summa OPOS marks around the selection (all art when nothing is selected) on the layer \"Regmark\" (its old marks are replaced), as one undo step → {layer, marks, lines, xDistance, yDistance, origin: [x, y] (the origin mark's lower-left corner)}",
            has_doc,
            summa
        ),
        cmd!(
            "registration.zund",
            "Zünd Registration Dots",
            [],
            None,
            "{diameter?: pt (0.2 to 0.4 in; 0.25 in), gap?: pt (art to the dots' edges; 10 mm), fifth?: 0.05..0.95 (the fifth dot's place on the bottom edge, from the bottom-right dot toward the bottom-left; 0.25), ids?} add five Zünd registration dots around the selection (all art when nothing is selected) on the layer \"Register\" (its old dots are replaced), as one undo step → {layer, dots: [[x, y]] (centres)}",
            has_doc,
            zund
        ),
    ]
}

/// Is `id` on one of the registration layers?
fn on_marks_layer(doc: &Document, id: NodeId) -> bool {
    doc.layer_of(id).and_then(|l| doc.node(l)).is_some_and(|l| matches!(l.name.as_deref(), Some(SUMMA_LAYER | ZUND_LAYER)))
}

/// The bounds the marks go around: the targeted objects', else all art but the marks layers'.
fn art_rect(s: &Session, p: &Value, cmd: &str) -> Result<Rect> {
    let st = s.doc()?;
    let d = &st.doc;
    let ids: Vec<NodeId> = targets(s, p)?.into_iter().filter(|id| !on_marks_layer(d, *id)).collect();
    let b = if ids.is_empty() {
        d.layers
            .iter()
            .filter(|l| !matches!(l.name.as_deref(), Some(SUMMA_LAYER | ZUND_LAYER)))
            .fold(None, |acc, l| vectorcraft_geom::union_opt(acc, l.visual_bounds()))
    } else {
        d.bounds_of(&ids, true)
    };
    b.filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())).ok_or_else(|| bad(cmd, "there is no artwork to put marks around"))
}

/// A length param in points, checked to lie in `[lo, hi]` (mm).
fn length(p: &Value, key: &str, cmd: &str, default_mm: f64, (lo, hi): (f64, f64)) -> Result<f64> {
    let v = match p.get(key) {
        None | Some(Value::Null) => return Ok(default_mm * MM),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("{key} must be a number of points")))?,
    };
    // A hair of slack so a value typed in mm doesn't fail on rounding.
    if v < lo * MM - 1e-6 || v > hi * MM + 1e-6 {
        return Err(bad(cmd, format!("{key} must be {lo:.2} to {hi:.2} mm (it is {:.2} mm)", v / MM)));
    }
    Ok(v.clamp(lo * MM, hi * MM))
}

/// Solid 100% black, no outline (an outline changes a mark's size).
fn mark_look() -> Appearance {
    Appearance::basic(Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)), Paint::None, 0.0)
}

/// Put `paths` (name, outline) on the top-level layer `name`, replacing what it held (a new layer
/// at the top when there is none), as one undo step. Returns the layer.
fn fill_layer(s: &mut Session, label: &str, name: &str, paths: Vec<(String, vectorcraft_geom::PathData)>) -> Result<NodeId> {
    let lid = s.edit(label, |d, sel| {
        let existing = d.layers.iter().find(|l| l.name.as_deref() == Some(name)).map(|l| l.id);
        let lid = match existing {
            Some(lid) => {
                let old: Vec<NodeId> = d.node(lid).and_then(|n| n.children()).map(|c| c.iter().map(|n| n.id).collect()).unwrap_or_default();
                for id in old {
                    d.remove(id)?;
                }
                if let Some(l) = d.node_mut(lid) {
                    l.visible = true;
                    l.locked = false;
                }
                lid
            }
            None => d.add_layer(Some(name)),
        };
        for (nm, path) in paths {
            let id = d.alloc_id();
            let mut n = Node::path(id, path, mark_look());
            n.name = Some(nm);
            d.insert(Some(lid), usize::MAX, n)?;
        }
        sel.prune(d);
        Ok(lid)
    })?;
    Ok(lid)
}

fn summa(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "registration.summa";
    let mode = match str_param(p, "mode") {
        None => OposMode::Opos,
        Some(m) => OposMode::parse(m).ok_or_else(|| bad(C, format!("unknown mode `{m}` (opos, oposXY, oposXY2, oposRandomXY)")))?,
    };
    let (lo, hi, def) = SUMMA_SIZE_MM;
    let size = length(p, "size", C, def, (lo, hi))?;
    let size_mm = size / MM;
    let gap = length(p, "gap", C, size_mm * 4.0, (size_mm * SUMMA_CLEAR, 1000.0))?;
    let step = length(p, "xDistance", C, SUMMA_X_DISTANCE_MM.0, (size_mm * (1.0 + SUMMA_CLEAR), SUMMA_X_DISTANCE_MM.1))?;
    let art = art_rect(s, p, C)?;
    let lay = summa_layout(art, mode, size, gap, step).map_err(|e| bad(C, e))?;
    let mut paths: Vec<(String, _)> = lay.marks.iter().enumerate().map(|(i, r)| (format!("OPOS Mark {}", i + 1), shapes::rectangle(*r))).collect();
    paths.extend(lay.lines.iter().enumerate().map(|(i, r)| (format!("OPOS XY Line {}", i + 1), shapes::rectangle(*r))));
    let lid = fill_layer(s, mode.label(), SUMMA_LAYER, paths)?;
    let origin = lay.marks.first().map(|r| [r.x0, r.y1]).unwrap_or_default();
    Ok(json!({
        "layer": lid.0,
        "marks": lay.marks.len(),
        "lines": lay.lines.len(),
        "xDistance": lay.x_distance,
        "yDistance": lay.y_distance,
        "origin": origin,
    }))
}

fn zund(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "registration.zund";
    let (lo, hi, def) = ZUND_DOT_MM;
    let diameter = length(p, "diameter", C, def, (lo, hi))?;
    let gap = length(p, "gap", C, ZUND_GAP_MM, (0.0, 1000.0))?;
    let fifth = match p.get("fifth") {
        None | Some(Value::Null) => ZUND_FIFTH,
        Some(v) => v.as_f64().filter(|f| (0.05..=0.95).contains(f)).ok_or_else(|| bad(C, "fifth must be 0.05 to 0.95"))?,
    };
    let art = art_rect(s, p, C)?;
    let dots = zund_layout(art, diameter, gap, fifth);
    let paths = dots
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("Register Dot {}", i + 1), shapes::ellipse(Rect::from_center_size(*c, (diameter, diameter)))))
        .collect();
    let lid = fill_layer(s, "Zünd Registration Dots", ZUND_LAYER, paths)?;
    Ok(json!({ "layer": lid.0, "dots": dots.iter().map(|c| [c.x, c.y]).collect::<Vec<_>>() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_box() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 2000, "height": 2000})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 600, "height": 300})).unwrap();
        s
    }

    fn layer_children(s: &Session, name: &str) -> Vec<Node> {
        let d = &s.doc().unwrap().doc;
        let l = d.layers.iter().find(|l| l.name.as_deref() == Some(name)).expect("marks layer");
        l.children().unwrap().iter().map(|n| (**n).clone()).collect()
    }

    #[test]
    fn summa_rows_follow_the_manual() {
        let art = Rect::new(0.0, 0.0, 1000.0 * MM, 300.0 * MM);
        let size = 3.0 * MM;
        let lay = summa_layout(art, OposMode::Opos, size, 12.0 * MM, 400.0 * MM).unwrap();
        // 1030 mm between the outer marks' corners → 3 steps of ~343 mm, 4 marks a row.
        assert_eq!(lay.marks.len(), 8);
        assert!(lay.x_distance <= 400.0 * MM + 1e-9);
        // Origin mark: lower left, clear of the art by the gap.
        let o = lay.marks[0];
        assert!((o.x1 - (art.x0 - 12.0 * MM)).abs() < 1e-9 && (o.y0 - (art.y1 + 12.0 * MM)).abs() < 1e-9);
        assert!((o.width() - size).abs() < 1e-9 && (o.height() - size).abs() < 1e-9);
        // Rows level, columns aligned.
        assert!(lay.marks[..4].iter().all(|m| m.y0 == o.y0));
        assert!(lay.marks[..4].iter().zip(&lay.marks[4..]).all(|(a, b)| a.x0 == b.x0));
        assert!(lay.lines.is_empty());
    }

    #[test]
    fn summa_xy_lines() {
        let art = Rect::new(0.0, 0.0, 300.0 * MM, 200.0 * MM);
        let size = 3.0 * MM;
        let xy = summa_layout(art, OposMode::Xy, size, 12.0 * MM, 400.0 * MM).unwrap();
        assert_eq!((xy.marks.len(), xy.lines.len()), (4, 1));
        let (line, a, b) = (xy.lines[0], xy.marks[0], xy.marks[1]);
        // 3 mm high, centred on the bottom row, 10 mm short of both marks (as Summa's plug-in).
        assert!((line.height() - 3.0 * MM).abs() < 1e-9);
        assert!((line.center().y - a.center().y).abs() < 1e-9);
        assert!((line.x0 - a.x1 - 10.0 * MM).abs() < 1e-9 && (b.x0 - line.x1 - 10.0 * MM).abs() < 1e-9);
        let xy2 = summa_layout(art, OposMode::Xy2, size, 12.0 * MM, 400.0 * MM).unwrap();
        assert_eq!(xy2.lines.len(), 2);
        assert!((xy2.lines[1].center().y - xy2.marks[2].center().y).abs() < 1e-9, "the second bar runs along the top row");
        let random = summa_layout(art, OposMode::RandomXy, size, 12.0 * MM, 200.0 * MM).unwrap();
        assert_eq!(random.marks.len(), 6);
        assert_eq!(random.lines.len(), 2 + 2 + 2);
        // Too close for a bar.
        assert!(summa_layout(Rect::new(0.0, 0.0, 1.0, 100.0), OposMode::Xy, size, 9.0 * MM, 400.0 * MM).is_err());
    }

    #[test]
    fn summa_command_puts_black_marks_on_their_own_layer_and_replaces_them() {
        let mut s = session_with_box();
        let r = s.execute("registration.summa", &json!({"mode": "oposXY2"})).unwrap();
        assert_eq!(r["lines"], 2);
        let kids = layer_children(&s, SUMMA_LAYER);
        assert_eq!(kids.len(), r["marks"].as_u64().unwrap() as usize + 2);
        let fill = kids[0].appearance.fill().unwrap();
        assert_eq!(fill.paint, Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
        // Again: replaced, not doubled; one undo step each.
        s.execute("registration.summa", &json!({})).unwrap();
        assert_eq!(layer_children(&s, SUMMA_LAYER).len(), r["marks"].as_u64().unwrap() as usize);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(layer_children(&s, SUMMA_LAYER).len(), r["marks"].as_u64().unwrap() as usize + 2);
        // The marks don't count as artwork the next time.
        let again = s.execute("registration.summa", &json!({"mode": "oposXY2"})).unwrap();
        assert_eq!(again["origin"], r["origin"]);
    }

    #[test]
    fn summa_rejects_sizes_outside_the_manual() {
        let mut s = session_with_box();
        assert!(s.execute("registration.summa", &json!({"size": 1.0 * MM})).is_err());
        assert!(s.execute("registration.summa", &json!({"size": 11.0 * MM})).is_err());
        assert!(s.execute("registration.summa", &json!({"size": 5.0 * MM, "gap": 10.0 * MM})).is_err(), "gap under 3× the size");
        assert!(s.execute("registration.summa", &json!({"xDistance": 1400.0 * MM})).is_err());
        assert!(s.execute("registration.summa", &json!({"mode": "nope"})).is_err());
    }

    #[test]
    fn zund_five_dots_with_the_fifth_off_centre() {
        let art = Rect::new(0.0, 0.0, 400.0, 200.0);
        let d = zund_layout(art, 18.0, 20.0, 0.25);
        assert_eq!(d.len(), 5);
        let (l, r) = (d[3].x, d[2].x);
        assert_eq!(d[4].y, d[2].y);
        assert!((d[4].x - (r - 0.25 * (r - l))).abs() < 1e-9);
        // Clear of the art.
        assert!(d[0].x + 9.0 <= art.x0 - 20.0 + 1e-9 && d[2].y - 9.0 >= art.y1 + 20.0 - 1e-9);
    }

    #[test]
    fn zund_command_enforces_the_dot_range() {
        let mut s = session_with_box();
        let r = s.execute("registration.zund", &json!({})).unwrap();
        assert_eq!(r["dots"].as_array().unwrap().len(), 5);
        let kids = layer_children(&s, ZUND_LAYER);
        assert_eq!(kids.len(), 5);
        let b = kids[0].geometric_bounds().unwrap();
        assert!((b.width() - 6.35 * MM).abs() < 1e-6, "default dot is 1/4 in");
        assert!(s.execute("registration.zund", &json!({"diameter": 0.19 * 72.0})).is_err());
        assert!(s.execute("registration.zund", &json!({"diameter": 0.41 * 72.0})).is_err());
        assert!(s.execute("registration.zund", &json!({"diameter": 0.4 * 72.0})).is_ok());
        assert!(s.execute("registration.zund", &json!({"fifth": 1.0})).is_err());
        assert_eq!(layer_children(&s, ZUND_LAYER).len(), 5);
    }

    #[test]
    fn marks_go_around_the_selection() {
        let mut s = session_with_box();
        s.execute("shape.rectangle", &json!({"x": 1000, "y": 1000, "width": 50, "height": 50})).unwrap();
        let r = s.execute("registration.zund", &json!({})).unwrap();
        let tl = &r["dots"][0];
        // Around the selected small box only.
        assert!(tl[0].as_f64().unwrap() > 900.0 && tl[1].as_f64().unwrap() > 900.0);
    }

    #[test]
    fn no_art_is_an_error() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        assert!(s.execute("registration.zund", &json!({})).is_err());
    }
}
