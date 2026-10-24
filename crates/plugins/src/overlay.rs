//! Overlays: lines a live-effect plug-in shows on the canvas besides the geometry it returns, such
//! as a preview of how a machine will work the object (a pen's path, a cutter's passes) or
//! construction lines. They are for the eyes only: the canvas draws them above the artwork, and
//! nothing exports, prints, measures or hit-tests them. The document doesn't hold them either: they
//! are kept with the effect's cached result and shown again from it.
//!
//! A live effect returns one as `"overlay": {"format": "f32-polylines", "data": "<base64>"}` beside
//! (or instead of) its `objects`. The data is little-endian binary, 4 bytes a coordinate rather than
//! the ten or more of JSON numbers:
//!
//! ```text
//! u32 count of lines
//! each line: u8 r, u8 g, u8 b, u8 a   colour, not premultiplied
//!            f32 width                in points; 0 draws one pixel wide at any zoom
//!            u32 n                    points, at least 2
//!            n × (f32 x, f32 y)       in the coordinates of the geometry
//! ```

use serde_json::Value;

use crate::objects::{MAX_COORD, MAX_STROKE_WIDTH};
use crate::{Error, Result};

/// The format id of [`Overlay`] data.
pub const FORMAT: &str = "f32-polylines";

/// Most lines one overlay holds.
pub const MAX_LINES: usize = 100_000;

/// Most points one overlay holds, all its lines together.
pub const MAX_POINTS: usize = 1_000_000;

/// What a live effect shows on the canvas besides its geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    pub lines: Vec<Line>,
}

/// One line of an [`Overlay`]: a polyline of at least two points.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Red, green, blue and alpha, not premultiplied.
    pub color: [u8; 4],
    /// Width in points; 0 is one pixel wide at any zoom.
    pub width: f32,
    pub points: Vec<[f32; 2]>,
}

/// Reads little-endian values off a byte slice, failing (never panicking) at its end.
struct Reader<'a> {
    rest: &'a [u8],
}

impl Reader<'_> {
    /// The next `N` bytes, or an error at the data's end.
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let (head, rest) = self.rest.split_first_chunk::<N>().ok_or_else(|| err("the data ends too soon"))?;
        self.rest = rest;
        Ok(*head)
    }
    /// The next little-endian `u32`, as a count.
    fn u32(&mut self) -> Result<usize> {
        Ok(u32::from_le_bytes(self.take()?) as usize)
    }
    /// The next little-endian `f32`.
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take()?))
    }
}

/// The output error of a bad overlay, saying `why`.
fn err(why: impl std::fmt::Display) -> Error {
    Error::Output(format!("`overlay`: {why}"))
}

impl Overlay {
    /// The overlay in `f32-polylines` `data`, checked: whole lines of finite points within the
    /// coordinate range, widths of 0–1,000 pt, at most [`MAX_LINES`] lines and [`MAX_POINTS`]
    /// points, and no bytes after the last line.
    pub fn decode(data: &[u8]) -> Result<Overlay> {
        let mut r = Reader { rest: data };
        let count = r.u32()?;
        if count > MAX_LINES {
            return Err(err(format!("{count} lines (limit {MAX_LINES})")));
        }
        let mut lines = Vec::with_capacity(count.min(1024));
        let mut points = 0usize;
        for i in 0..count {
            let color = r.take::<4>()?;
            let width = r.f32()?;
            if !(width.is_finite() && (0.0..=MAX_STROKE_WIDTH as f32).contains(&width)) {
                return Err(err(format!("line {i}: the width must be 0 to {MAX_STROKE_WIDTH} pt")));
            }
            let n = r.u32()?;
            points = points.saturating_add(n);
            if points > MAX_POINTS {
                return Err(err(format!("more than {MAX_POINTS} points")));
            }
            if n < 2 {
                return Err(err(format!("line {i}: a line has at least 2 points")));
            }
            // Room for the points the data can still hold, whatever `n` promises.
            let mut line = Vec::with_capacity(n.min(r.rest.len() / 8));
            for _ in 0..n {
                let p = [r.f32()?, r.f32()?];
                if !p.iter().all(|c| c.is_finite() && f64::from(c.abs()) <= MAX_COORD) {
                    return Err(err(format!("line {i}: coordinate out of range (limit ±{MAX_COORD} pt)")));
                }
                line.push(p);
            }
            lines.push(Line { color, width, points: line });
        }
        if !r.rest.is_empty() {
            return Err(err(format!("{} bytes after the last line", r.rest.len())));
        }
        Ok(Overlay { lines })
    }

    /// The points of all its lines.
    pub fn points(&self) -> usize {
        self.lines.iter().map(|l| l.points.len()).sum()
    }

    /// The overlay as `f32-polylines` data: what [`Overlay::decode`] reads.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((self.lines.len() as u32).to_le_bytes());
        for l in &self.lines {
            out.extend(l.color);
            out.extend(l.width.to_le_bytes());
            out.extend((l.points.len() as u32).to_le_bytes());
            for p in &l.points {
                out.extend(p[0].to_le_bytes());
                out.extend(p[1].to_le_bytes());
            }
        }
        out
    }

    /// The overlay a plug-in's output `v` holds: `None` without one, else its `overlay`, which must
    /// be `{"format": "f32-polylines", "data": "<base64>"}`.
    pub(crate) fn from_output(v: &Value) -> Result<Option<Overlay>> {
        let Some(o) = v.get("overlay").filter(|o| !o.is_null()) else { return Ok(None) };
        match o.get("format").and_then(Value::as_str) {
            Some(FORMAT) => {}
            Some(other) => return Err(err(format!("format {other:?} isn't one this host reads (\"{FORMAT}\")"))),
            None => return Err(err(format!("expected {{\"format\": \"{FORMAT}\", \"data\": \"<base64>\"}}"))),
        }
        let data = o.get("data").and_then(Value::as_str).ok_or_else(|| err("`data` must be a base64 string"))?;
        let bytes = vectorcraft_doc::base64::decode(data).ok_or_else(|| err("`data` is not valid base64"))?;
        Overlay::decode(&bytes).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Two lines: one a pixel wide and half transparent, one 2.5 pt wide through three points.
    fn sample() -> Overlay {
        Overlay {
            lines: vec![
                Line { color: [255, 0, 0, 128], width: 0.0, points: vec![[0.0, 0.0], [10.0, 5.5]] },
                Line { color: [0, 0, 255, 255], width: 2.5, points: vec![[1.0, 2.0], [3.0, 4.0], [-5.0, 6.0]] },
            ],
        }
    }

    /// Encoding and decoding agree, at 4 bytes a coordinate, and a plug-in's output gives the same
    /// overlay; output without one gives none.
    #[test]
    fn round_trips_its_lines() {
        let o = sample();
        let data = o.encode();
        assert_eq!(data.len(), 4 + (12 + 2 * 8) + (12 + 3 * 8), "4 bytes a coordinate");
        assert_eq!(Overlay::decode(&data).unwrap(), o);
        assert_eq!(o.points(), 5);
        assert_eq!(Overlay::decode(&0u32.to_le_bytes()).unwrap(), Overlay::default(), "no lines");
        let v = json!({"overlay": {"format": FORMAT, "data": vectorcraft_doc::base64::encode(&data)}});
        assert_eq!(Overlay::from_output(&v).unwrap(), Some(o));
        assert_eq!(Overlay::from_output(&json!({"objects": []})).unwrap(), None);
        assert_eq!(Overlay::from_output(&json!({"overlay": null})).unwrap(), None);
    }

    /// Plug-in output is untrusted: whatever its bytes, it is read or refused, never more.
    #[test]
    fn refuses_what_it_cannot_draw() {
        let good = sample().encode();
        let with = |f: &dyn Fn(&mut Overlay)| {
            let mut o = sample();
            f(&mut o);
            o.encode()
        };
        let mut long = good.clone();
        long.push(0);
        let too_many = (MAX_LINES as u32 + 1).to_le_bytes().to_vec();
        let bad: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("cut short", good.get(..good.len() - 1).unwrap().to_vec()),
            ("trailing bytes", long),
            ("too many lines", too_many),
            ("one point", with(&|o| o.lines[0].points.truncate(1))),
            ("NaN", with(&|o| o.lines[1].points[0][0] = f32::NAN)),
            ("far away", with(&|o| o.lines[1].points[2][1] = 1e9)),
            ("negative width", with(&|o| o.lines[0].width = -1.0)),
            ("infinite width", with(&|o| o.lines[0].width = f32::INFINITY)),
            ("too many points", with(&|o| o.lines[0].points = vec![[0.0, 0.0]; MAX_POINTS + 1])),
        ];
        for (what, data) in bad {
            assert!(matches!(Overlay::decode(&data), Err(Error::Output(_))), "{what}");
        }
        // A line count that promises more than the data holds fails at the data's end, without
        // reserving room for it all.
        let mut promise = 50_000u32.to_le_bytes().to_vec();
        promise.extend([0; 12]);
        assert!(Overlay::decode(&promise).is_err());
        for v in [
            json!({"overlay": {"format": "svg", "data": ""}}),
            json!({"overlay": {"data": ""}}),
            json!({"overlay": {"format": FORMAT}}),
            json!({"overlay": {"format": FORMAT, "data": "not base64!"}}),
            // Padding inside the data: read past, it would be the four zero bytes of no lines.
            json!({"overlay": {"format": FORMAT, "data": "AAA=AAA="}}),
            json!({"overlay": 3}),
        ] {
            assert!(Overlay::from_output(&v).is_err(), "{v}");
        }
    }
}
