//! How VectorCraft draws its cursors today, written out as SVG for nibcraft
//! (https://github.com/bobtherobot/nibcraft): a designer starts a cursor from it.
//! `vectorcraft --cursor-seeds DIR` writes them; nibcraft's `pull` runs it and keeps the seeds of
//! the cursors that have no drawing there.
//!
//! Every cursor with a look of its own is written: one drawn from a designer's SVG
//! ([`glyphs::GENERATED`], written back from its data, the inverse of nibcraft's reading) or one
//! drawn in this crate's code. Not those that borrow another's look ([`Cursor::stand_in`]), nor
//! system cursors. The white strokes in the hand-drawn glyphs are their halos, which nibcraft adds
//! by itself, so they're left out; white fills (hollow shapes) stay.

use egui::epaint::{CircleShape, ColorMode, PathShape};
use egui::{Color32, Pos2, Rect, Shape, pos2, vec2};
use vectorcraft_tools::Cursor;

use super::glyphs::{Glyph, GlyphPart};
use super::{GLYPHS, HALO, INK, glyph, glyphs};

/// The side of nibcraft's canvas (`viewBox="0 0 32 32"`), in points.
const CANVAS: f32 = 32.0;

/// Every cursor with a look of its own, as `(id, svg)`: the id is the cursor's name in
/// `crates/tools/src/cursors.toml` (its `Cursor` value). A cursor whose shapes can't be written
/// (a kind of shape or a colour this module doesn't know) is left out; the tests catch that.
pub fn seed_svgs() -> Vec<(String, String)> {
    // A designer can draw a cursor that only borrows a look in code (ScissorsNotAllowed), so it
    // isn't in GLYPHS.
    let drawn_only = glyphs::GENERATED.iter().map(|&(c, _)| c).filter(|c| !GLYPHS.contains(c));
    GLYPHS
        .iter()
        .copied()
        .chain(drawn_only)
        .filter_map(|c| {
            let elements = match glyphs::generated(c) {
                Some(g) => drawn(g),
                None if c.stand_in().is_none() => shape_elements(&glyph(c, Pos2::ZERO)?)?,
                None => return None,
            };
            let id = id(c);
            Some((id.clone(), svg(&id, &elements)?))
        })
        .collect()
}

/// A generated glyph's parts as elements: each as the designer drew it.
fn drawn(g: &Glyph) -> Vec<Element> {
    let colour = |white: bool| if white { HALO } else { INK };
    let pts = |points: &[(f32, f32)]| points.iter().map(|&(x, y)| pos2(x, y)).collect::<Vec<_>>();
    g.parts
        .iter()
        .filter_map(|part| match part {
            GlyphPart::Fill { points, white, .. } if points.len() >= 3 => {
                Some(Element::Polygon { points: pts(points), closed: true, fill: Some(colour(*white)), stroke: None })
            }
            GlyphPart::Stroke { points, closed, width, white } if points.len() >= 2 => {
                Some(Element::Polygon { points: pts(points), closed: *closed, fill: None, stroke: Some((*width, colour(*white))) })
            }
            _ => None,
        })
        .collect()
}

/// Hand-drawn shapes as elements; `None` if one can't be written.
fn shape_elements(shapes: &[Shape]) -> Option<Vec<Element>> {
    let parts = shapes.iter().map(element).collect::<Result<Vec<_>, ()>>().ok()?;
    Some(merge_outlines(parts.into_iter().flatten().collect()))
}

/// The cursor's id: its `Cursor` value without a payload (`RotateToward(1)` is `RotateToward`).
fn id(c: Cursor) -> String {
    let name = format!("{c:?}");
    name.split('(').next().unwrap_or_default().to_string()
}

/// One element to write: a filled or stroked outline, in points relative to the hotspot.
enum Element {
    Polygon { points: Vec<Pos2>, closed: bool, fill: Option<Color32>, stroke: Option<(f32, Color32)> },
    Circle { centre: Pos2, radius: f32, fill: Option<Color32>, stroke: Option<(f32, Color32)> },
}

impl Element {
    /// The area it covers, strokes included.
    fn bounds(&self) -> Rect {
        match self {
            Element::Polygon { points, stroke, .. } => Rect::from_points(points).expand(stroke.map_or(0.0, |(w, _)| w / 2.0)),
            Element::Circle { centre, radius, stroke, .. } => {
                Rect::from_center_size(*centre, vec2(2.0 * radius, 2.0 * radius)).expand(stroke.map_or(0.0, |(w, _)| w / 2.0))
            }
        }
    }
}

/// The ink of a shape: `None` for halo strokes and invisible parts. Shapes this module doesn't
/// know give `Err`, so a new kind of hand-drawn part is noticed (by the tests) instead of dropped.
fn element(s: &Shape) -> Result<Option<Element>, ()> {
    let ink = |c: Color32| c != Color32::TRANSPARENT;
    // Cursors are black and white only: another colour would be written wrongly, so refuse it.
    if shape_colours(s).unwrap_or_default().iter().any(|&c| ink(c) && c != INK && c != HALO) {
        return Err(());
    }
    // A white stroke is a halo: nibcraft draws its own.
    let stroke = |w: f32, c: Color32| (w > 0.0 && ink(c) && c != HALO).then_some((w, c));
    Ok(match s {
        Shape::Noop => None,
        Shape::LineSegment { points, stroke: st } => {
            stroke(st.width, st.color).map(|st| Element::Polygon { points: points.to_vec(), closed: false, fill: None, stroke: Some(st) })
        }
        Shape::Path(PathShape { points, .. }) if points.len() < 2 => None,
        Shape::Path(PathShape { points, closed, fill, stroke: st }) => {
            let ColorMode::Solid(colour) = st.color else { return Err(()) };
            let fill = (*closed && ink(*fill)).then_some(*fill);
            let st = stroke(st.width, colour);
            (fill.is_some() || st.is_some()).then(|| Element::Polygon { points: points.clone(), closed: *closed, fill, stroke: st })
        }
        Shape::Circle(CircleShape { center, radius, fill, stroke: st }) => {
            let fill = ink(*fill).then_some(*fill);
            let st = stroke(st.width, st.color);
            (fill.is_some() || st.is_some()).then_some(Element::Circle { centre: *center, radius: *radius, fill, stroke: st })
        }
        _ => return Err(()),
    })
}

/// The colours a shape paints with, `None` for a shape [`element`] doesn't know.
fn shape_colours(s: &Shape) -> Option<Vec<Color32>> {
    match s {
        Shape::Noop => Some(vec![]),
        Shape::LineSegment { stroke, .. } => Some(vec![stroke.color]),
        Shape::Path(PathShape { fill, stroke, .. }) => match stroke.color {
            ColorMode::Solid(c) => Some(vec![*fill, c]),
            ColorMode::UV(_) => None,
        },
        Shape::Circle(CircleShape { fill, stroke, .. }) => Some(vec![*fill, stroke.color]),
        _ => None,
    }
}

/// A fill followed by the outline of the same shape (how [`super::poly`] draws) as one element,
/// so the designer edits one path.
fn merge_outlines(elements: Vec<Element>) -> Vec<Element> {
    let mut out: Vec<Element> = Vec::with_capacity(elements.len());
    for e in elements {
        if let (
            Some(Element::Polygon { points: filled, fill: Some(_), stroke: outline @ None, .. }),
            Element::Polygon { points, closed: true, fill: None, stroke: Some(stroke) },
        ) = (out.last_mut(), &e)
            && filled == points
        {
            *outline = Some(*stroke);
            continue;
        }
        out.push(e);
    }
    out
}

/// A number for the SVG: at most three decimals, no trailing zeros.
fn num(x: f32) -> String {
    let s = format!("{:.3}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn colour(c: Color32) -> &'static str {
    if c == HALO { "#ffffff" } else { "#000000" }
}

/// Cursor `id`'s elements as an SVG on nibcraft's canvas, its hotspot marked; `None` when
/// there's nothing to draw.
fn svg(id: &str, elements: &[Element]) -> Option<String> {
    let bounds = elements.iter().map(Element::bounds).reduce(|a, b| a.union(b))?;
    // The glyph centred on the canvas, the hotspot on a half point.
    let hotspot = ((pos2(CANVAS, CANVAS) / 2.0 - bounds.center().to_vec2()) * 2.0).round() / 2.0;
    let at = |p: Pos2| p + hotspot.to_vec2();
    let paint = |fill: Option<Color32>, stroke: Option<(f32, Color32)>| {
        let fill = fill.map_or("none", colour);
        match stroke {
            Some((w, c)) => format!(r#"fill="{fill}" stroke="{}" stroke-width="{}""#, colour(c), num(w)),
            None => format!(r#"fill="{fill}""#),
        }
    };
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"32\" viewBox=\"0 0 32 32\">\n  \
         <title>{id}</title>\n  \
         <!-- How VectorCraft draws its {id} cursor today. Without the white halos: nibcraft adds those. -->\n"
    );
    for e in elements {
        match e {
            Element::Polygon { points, closed, fill, stroke } => {
                let mut d = String::new();
                for (i, p) in points.iter().map(|&p| at(p)).enumerate() {
                    d.push_str(&format!("{}{} {} ", if i == 0 { "M" } else { "L" }, num(p.x), num(p.y)));
                }
                if *closed {
                    d.push('Z');
                }
                out.push_str(&format!("  <path d=\"{}\" {}/>\n", d.trim_end(), paint(*fill, *stroke)));
            }
            Element::Circle { centre, radius, fill, stroke } => {
                let c = at(*centre);
                out.push_str(&format!("  <circle cx=\"{}\" cy=\"{}\" r=\"{}\" {}/>\n", num(c.x), num(c.y), num(*radius), paint(*fill, *stroke)));
            }
        }
    }
    out.push_str(&format!("  <circle id=\"hotspot\" cx=\"{}\" cy=\"{}\" r=\"0.25\" fill=\"#ff0000\"/>\n</svg>\n", num(hotspot.x), num(hotspot.y)));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursors::Ink;

    fn seed(id: &str) -> String {
        seed_svgs().into_iter().find(|(i, _)| i == id).map(|(_, s)| s).unwrap_or_else(|| panic!("no seed for {id}"))
    }

    /// The SVG of a cursor drawn in code, whether or not a designer's drawing replaces it today.
    fn hand_drawn(id: &str, draw: impl FnOnce(&mut Ink)) -> String {
        let ink = &mut Ink::default();
        draw(ink);
        svg(id, &shape_elements(&ink.0).unwrap()).unwrap()
    }

    #[test]
    fn the_seeds_are_every_cursor_with_a_look_of_its_own() {
        let ids: Vec<String> = seed_svgs().into_iter().map(|(id, _)| id).collect();
        for c in GLYPHS {
            let own = glyphs::generated(c).is_some() || (c.stand_in().is_none() && glyph(c, Pos2::ZERO).is_some());
            assert_eq!(ids.contains(&id(c)), own, "{c:?}");
        }
        // Borrowed or a system cursor: no seed.
        assert!(!ids.contains(&"NotAllowed".to_string()), "{ids:?}");
        assert!(ids.contains(&"Pen".to_string()) && ids.contains(&"Crosshair".to_string()), "{ids:?}");
    }

    #[test]
    fn a_designers_glyph_is_written_back_part_for_part() {
        for &(c, g) in glyphs::GENERATED {
            let svg = seed(&id(c));
            assert_eq!(svg.matches("<path ").count(), g.parts.len(), "{c:?}\n{svg}");
            let white = g.parts.iter().any(|p| matches!(p, GlyphPart::Fill { white: true, .. } | GlyphPart::Stroke { white: true, .. }));
            assert_eq!(svg.contains("#ffffff"), white, "{c:?}\n{svg}");
        }
    }

    #[test]
    fn the_hand_drawn_crosshair_is_its_four_black_arms_round_the_hotspot() {
        let crosshair = hand_drawn("Crosshair", |ink| super::super::crosshair(ink, Pos2::ZERO));
        assert!(crosshair.contains(r#"viewBox="0 0 32 32""#));
        assert!(crosshair.contains(r##"<circle id="hotspot" cx="16" cy="16" r="0.25" fill="#ff0000"/>"##), "{crosshair}");
        // The halos are left out: only the 1.2-point black arms.
        assert!(!crosshair.contains("#ffffff"), "{crosshair}");
        assert_eq!(crosshair.matches("<path ").count(), 4, "{crosshair}");
        assert!(crosshair.contains(r##"<path d="M7 16 L14 16" fill="none" stroke="#000000" stroke-width="1.2"/>"##), "{crosshair}");
    }

    #[test]
    fn hollow_parts_keep_their_white_fill() {
        // Away from art the Blend tool's square is hollow: a white fill in a black outline.
        let blend = hand_drawn("Blend", |ink| super::super::blend(ink, Pos2::ZERO, Cursor::Blend));
        assert!(blend.contains(r##"fill="#ffffff" stroke="#000000" stroke-width="1""##), "{blend}");
        // Over an object it's filled.
        assert!(!hand_drawn("BlendObject", |ink| super::super::blend(ink, Pos2::ZERO, Cursor::BlendObject)).contains("#ffffff"));
    }

    #[test]
    fn every_seed_fits_the_canvas_with_its_hotspot_inside() {
        for (id, svg) in seed_svgs() {
            for n in svg.split(['"', ' ']).filter_map(|w| w.trim_start_matches(['M', 'L']).parse::<f32>().ok()) {
                assert!((-1.0..=33.0).contains(&n), "{id}: {n} outside the canvas\n{svg}");
            }
            assert_eq!(svg.matches(r#"id="hotspot""#).count(), 1, "{id}");
        }
    }

    #[test]
    fn shapes_it_cant_write_are_refused_not_drawn_wrong() {
        let red = Shape::line_segment([Pos2::ZERO, pos2(4.0, 0.0)], egui::Stroke::new(1.0, Color32::RED));
        assert!(element(&red).is_err(), "a colour cursors don't use");
        assert!(shape_elements(&[red]).is_none());
        let empty = Shape::Path(PathShape::closed_line(vec![], egui::Stroke::new(1.0, INK)));
        assert!(matches!(element(&empty), Ok(None)));
        assert!(svg("X", &shape_elements(&[empty]).unwrap()).is_none(), "nothing to draw");
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(7.0), "7");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(1.25), "1.25");
        assert_eq!(num(1.23456), "1.235");
    }
}
