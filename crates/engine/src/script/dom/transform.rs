//! The transform methods of a page item, `translate`, `rotate`, `resize` and `transform`, as the
//! help page <https://ai-scripting.docsforadobe.dev/jsobjref/PageItem/> describes them. Each one
//! runs `object.transform` on the object, with its selection kept, so it joins the run's one undo
//! step for the document; a call that changes nothing runs no command.
//!
//! VectorCraft's own rules, which the help page does not state:
//! - Offsets, angles and matrices are taken with Y up, as `Matrix` records are, whatever
//!   `app.coordinateSystem` is: `translate(0, 10)` moves the object up, and a positive angle turns
//!   it counterclockwise.
//! - `rotate`, `resize` and `transform` act about the center of the object's geometric bounds,
//!   unless a `Transformation` value names another point of those bounds; `DOCUMENTORIGIN` is the
//!   bottom-left corner of the first artboard. A matrix's own translation is added after it acts
//!   about that point.
//! - Positions, pattern fills and pattern strokes, and gradients each follow their argument:
//!   positions and gradients default to `true`, and patterns to the Transform Pattern Tiles
//!   preference. Fill and stroke patterns transform together, and gradients with the object, so a
//!   call whose values differ, counting the defaults of the arguments it leaves out, throws an error
//!   that says so.
//! - `changeLineWidths` is 100 (stroke weights stay, also the default) or the transformation's own
//!   scale in percent, the square root of the area it scales by (stroke weights and effects scale
//!   with it, as Scale Strokes & Effects does); any other value throws an error that says so.
//! - A locked object, one inside a locked layer or group, a group that holds one, and an object on
//!   a locked artboard are refused with nothing changed, and so are a scale of 0, a matrix whose
//!   determinant is 0 and a transformation that would take the object past the canvas.

use serde_json::json;
use vectorcraft_geom::{Affine, Point, Rect};
use vectorcraft_js::{Data, HostError};

use super::matrix::{self, Matrix};
use super::{Answer, Coords, Host, Item, arg, coordinate, edit_item, frame, number, this_item, with_doc};

/// The error for an object on a locked artboard.
const ON_LOCKED_ARTBOARD: &str = "cannot transform objects on a locked artboard";

/// The error for a locked object.
const LOCKED: &str = "cannot transform locked objects";

/// What a transform method changes besides the object's geometry.
struct Options {
    /// Positions and orientations (`changePositions`, `transformObjects`).
    objects: bool,
    /// Pattern fills and strokes.
    patterns: bool,
    /// Stroke weights and effects (`changeLineWidths`).
    strokes: bool,
}

/// An optional `true` or `false` argument.
fn flag(d: &Data, what: &str) -> std::result::Result<Option<bool>, HostError> {
    match d {
        Data::Undefined => Ok(None),
        Data::Bool(b) => Ok(Some(*b)),
        _ => Err(HostError::Type(format!("{what} must be true or false"))),
    }
}

/// The positions, fill patterns, fill gradients and stroke patterns arguments at `at` and after,
/// with the names the method gives them.
fn options(h: &Host, a: &[Data], at: usize, names: [&str; 4]) -> std::result::Result<Options, HostError> {
    let tiles = h.session.borrow().prefs.transform_pattern_tiles;
    let objects = flag(arg(a, at), names[0])?.unwrap_or(true);
    let fill = flag(arg(a, at + 1), names[1])?.unwrap_or(tiles);
    let gradients = flag(arg(a, at + 2), names[2])?.unwrap_or(true);
    let stroke = flag(arg(a, at + 3), names[3])?.unwrap_or(tiles);
    if fill != stroke {
        return Err(HostError::Error(format!("{} and {} with different values are not available yet", names[1], names[3])));
    }
    if gradients != objects {
        return Err(HostError::Error(format!("{} different from {} is not available yet", names[2], names[0])));
    }
    Ok(Options { objects, patterns: fill, strokes: false })
}

/// Whether `changeLineWidths` (`d`) scales stroke weights for the linear part `m` of a matrix.
fn line_widths(d: &Data, m: Matrix) -> std::result::Result<bool, HostError> {
    let lw = match d {
        Data::Undefined => return Ok(false),
        d => number(d, "changeLineWidths")?,
    };
    let own = (m[0] * m[3] - m[1] * m[2]).abs().sqrt() * 100.0;
    if (lw - own).abs() <= 1e-9 * own.max(1.0) {
        return Ok((own - 100.0).abs() > 1e-9);
    }
    if (lw - 100.0).abs() <= 1e-9 {
        return Ok(false);
    }
    Err(HostError::Error(format!("changeLineWidths other than 100 or the scale itself ({own}) is not available yet")))
}

/// The point of `r` (document coordinates, Y down) that the `Transformation` value `d` names.
fn anchor(h: &Host, item: Item, r: Rect, d: &Data) -> std::result::Result<Point, HostError> {
    let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
    let name = match d {
        Data::Undefined => return Ok(Point::new(cx, cy)),
        Data::String(s) => s.strip_prefix("Transformation.").unwrap_or(""),
        _ => "",
    };
    Ok(match name {
        "CENTER" => Point::new(cx, cy),
        "TOPLEFT" => Point::new(r.x0, r.y0),
        "TOP" => Point::new(cx, r.y0),
        "TOPRIGHT" => Point::new(r.x1, r.y0),
        "LEFT" => Point::new(r.x0, cy),
        "RIGHT" => Point::new(r.x1, cy),
        "BOTTOMLEFT" => Point::new(r.x0, r.y1),
        "BOTTOM" => Point::new(cx, r.y1),
        "BOTTOMRIGHT" => Point::new(r.x1, r.y1),
        "DOCUMENTORIGIN" => with_doc(h, item.doc, |_, st| frame(st).map(|f| f.to_document(Coords::Document, [0.0, 0.0])))?,
        _ => return Err(HostError::Type("the point to transform about must be a Transformation value".into())),
    })
}

/// The matrix `m` (Y up) as it acts on the page (Y down), about `o`.
fn on_page(m: Matrix, o: Point) -> Affine {
    let [a, b, c, d, tx, ty] = m;
    let page = Affine::new([a, 0.0 - b, 0.0 - c, d, tx, 0.0 - ty]);
    Affine::translate(o.to_vec2()) * page * Affine::translate(-o.to_vec2())
}

/// Applies `m` (Y up) to the item `this` names, about the point `about` names (`None`: the matrix
/// acts as a move, which needs no point), with `opts`.
fn apply(h: &Host, a: &[Data], m: Matrix, about: Option<&Data>, opts: &Options) -> Answer {
    let item = this_item(h, a)?;
    if m.iter().any(|v| !v.is_finite()) {
        return Err(HostError::Range("the transformation has a value too large for a number".into()));
    }
    if matrix::singular(m) {
        return Err(HostError::Error("the transformation is singular and would flatten the object".into()));
    }
    let (locked, on_locked, bounds, uses_pattern) = with_doc(h, item.doc, |_, st| {
        let d = &st.doc;
        let mut locked = d.ancestry(item.id).is_none_or(|a| a.iter().any(|i| d.node(*i).is_none_or(|n| n.locked)));
        let mut uses = false;
        if let Some(n) = d.node(item.id) {
            n.walk(&mut |c| {
                locked |= c.locked;
                uses |= vectorcraft_doc::pattern::uses_pattern(c, None);
            });
        }
        (locked, d.on_locked_artboard(item.id), d.bounds_of(&[item.id], false), uses)
    });
    if locked {
        return Err(HostError::Error(format!("{LOCKED}: {}", item.id.0)));
    }
    if on_locked {
        return Err(HostError::Error(format!("{ON_LOCKED_ARTBOARD}: {}", item.id.0)));
    }
    let o = match (about, bounds) {
        (Some(d), Some(r)) => anchor(h, item, r, d)?,
        (Some(_), None) => return Err(HostError::Error("the object has no bounds".into())),
        (None, _) => Point::ORIGIN,
    };
    let xf = on_page(m, o);
    if let Some(r) = bounds.filter(|_| opts.objects) {
        let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)];
        if corners.iter().map(|p| xf * *p).any(|p| !(p.x.abs() <= crate::MAX_COORD && p.y.abs() <= crate::MAX_COORD)) {
            return Err(HostError::Range("the transformation would take the object past the canvas".into()));
        }
    }
    let c = xf.as_coeffs();
    let identity = c.iter().zip([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).all(|(v, i)| (v - i).abs() <= 1e-12);
    let changes = if opts.objects { !identity } else { opts.patterns && uses_pattern && !identity };
    if changes {
        let p = json!({"ids": [item.id.0], "matrix": c, "objects": opts.objects, "patterns": opts.patterns, "strokes": opts.strokes});
        edit_item(h, item, "object.transform", &p)?;
    }
    Ok(Data::Undefined)
}

/// `translate(deltaX?, deltaY?, transformObjects?, transformFillPatterns?, transformFillGradients?,
/// transformStrokePatterns?)`: moves the object by `deltaX` and `deltaY` points (0 when left out).
pub(super) fn translate(h: &Host, a: &[Data]) -> Answer {
    let delta = |i: usize, what: &str| match arg(a, i) {
        Data::Undefined => Ok(0.0),
        d => coordinate(d, what),
    };
    let (dx, dy) = (delta(1, "deltaX")?, delta(2, "deltaY")?);
    let names = ["transformObjects", "transformFillPatterns", "transformFillGradients", "transformStrokePatterns"];
    let opts = options(h, a, 3, names)?;
    apply(h, a, matrix::translate(dx, dy), None, &opts)
}

const CHANGE_NAMES: [&str; 4] = ["changePositions", "changeFillPatterns", "changeFillGradients", "changeStrokePattern"];

/// `rotate(angle, changePositions?, changeFillPatterns?, changeFillGradients?, changeStrokePattern?,
/// rotateAbout?)`: turns the object by `angle` degrees, counterclockwise when it is positive.
pub(super) fn rotate(h: &Host, a: &[Data]) -> Answer {
    let angle = number(arg(a, 1), "angle")?;
    let opts = options(h, a, 2, CHANGE_NAMES)?;
    apply(h, a, matrix::rotate(angle), Some(arg(a, 6)), &opts)
}

/// `resize(scaleX, scaleY, changePositions?, changeFillPatterns?, changeFillGradients?,
/// changeStrokePattern?, changeLineWidths?, scaleAbout?)`: scales the object by `scaleX` and
/// `scaleY` percent along the page's axes.
pub(super) fn resize(h: &Host, a: &[Data]) -> Answer {
    let (sx, sy) = (number(arg(a, 1), "scaleX")?, number(arg(a, 2), "scaleY")?);
    if sx == 0.0 || sy == 0.0 {
        return Err(HostError::Range("scaleX and scaleY must not be 0".into()));
    }
    let m = matrix::scaled(sx, sy);
    let mut opts = options(h, a, 3, CHANGE_NAMES)?;
    opts.strokes = line_widths(arg(a, 7), m)?;
    apply(h, a, m, Some(arg(a, 8)), &opts)
}

/// `transform(transformationMatrix, changePositions?, changeFillPatterns?, changeFillGradients?,
/// changeStrokePattern?, changeLineWidths?, transformAbout?)`: applies a `Matrix` record to the
/// object.
pub(super) fn transform(h: &Host, a: &[Data]) -> Answer {
    let m = matrix::matrix(arg(a, 1), "transformationMatrix")?;
    let mut opts = options(h, a, 2, CHANGE_NAMES)?;
    opts.strokes = line_widths(arg(a, 6), m)?;
    apply(h, a, m, Some(arg(a, 7)), &opts)
}
