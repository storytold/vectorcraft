//! Command params written by tools and panels: the inverse of the engine's paint parsers, so a
//! paint read from the document and sent back through a command arrives unchanged.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Freeform, FreeformPoint, GradientPaint, GradientStop};

/// A colour as a `color` param in its own model: `[r,g,b]`, `{c,m,y,k}`, `{gray}` or `{l,a,b}`.
pub fn color_json(c: &Color) -> Value {
    match *c {
        Color::Rgb { r, g, b } => json!([r, g, b]),
        Color::Cmyk { c, m, y, k } => json!({"c": c, "m": m, "y": y, "k": k}),
        Color::Gray { k } => json!({"gray": k}),
        Color::Lab { l, a, b } => json!({"l": l, "a": a, "b": b}),
    }
}

/// Gradient stops as the `stops` param of `paint.editGradient` / a `gradient` paint (a linked
/// stop also carries its `swatch` and `tint` %).
pub fn stops_json(stops: &[GradientStop]) -> Value {
    Value::Array(
        stops
            .iter()
            .map(|s| {
                let mut v = json!({"offset": s.offset, "color": color_json(&s.color), "opacity": s.opacity, "midpoint": s.midpoint});
                if let Some(n) = &s.swatch {
                    v["swatch"] = json!(n);
                    v["tint"] = json!(s.tint * 100.0);
                }
                v
            })
            .collect(),
    )
}

/// A freeform point as its `paint.freeform.addPoint` / `freeform` param fields.
pub fn freeform_point_json(p: &FreeformPoint) -> Value {
    json!({"at": [p.at.x, p.at.y], "color": color_json(&p.color), "opacity": p.opacity, "spread": p.spread})
}

/// A freeform gradient as the `freeform` param of a `gradient` paint: points, lines and Draw mode.
pub fn freeform_json(f: &Freeform) -> Value {
    json!({
        "points": f.points.iter().map(freeform_point_json).collect::<Vec<_>>(),
        "lines": f.lines,
        "mode": f.mode.label().to_lowercase(),
    })
}

/// A gradient paint as the `gradient` param of `paint.setFill` / `swatch.new`: kind, stops (with
/// opacity and midpoint), angle, the linked swatch and, once placed, the vector, aspect (%) and
/// focal point, and the freeform points.
pub fn gradient_params(g: &GradientPaint) -> Value {
    let mut v = json!({
        "kind": g.gradient.kind.label().to_lowercase(),
        "angle": g.angle,
        "stops": stops_json(&g.gradient.stops),
    });
    if let Some(geom) = g.geom {
        v["start"] = json!([geom.start.x, geom.start.y]);
        v["end"] = json!([geom.end.x, geom.end.y]);
        v["aspect"] = json!(geom.aspect * 100.0);
        if let Some(f) = geom.focal {
            v["focal"] = json!([f.x, f.y]);
        }
    }
    if let Some(s) = &g.swatch {
        v["swatch"] = json!(s);
    }
    if g.gradient.interpolation != vectorcraft_color::GradientInterpolation::Linear {
        v["interpolation"] = json!(g.gradient.interpolation.label().to_lowercase());
    }
    if g.gradient.dither {
        v["dither"] = json!(true);
    }
    if let Some(f) = &g.freeform {
        v["freeform"] = freeform_json(f);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::{Gradient, GradientGeom, GradientKind};
    use vectorcraft_geom::Point;

    #[test]
    fn gradient_params_carry_placement_only_when_placed() {
        let mut g = GradientPaint::new(Gradient { kind: GradientKind::Radial, ..Default::default() });
        let v = gradient_params(&g);
        assert_eq!(v["kind"], "radial");
        assert!(v.get("start").is_none() && v.get("aspect").is_none());
        assert_eq!(v["stops"][0]["midpoint"], json!(0.5));
        g.geom = Some(GradientGeom { start: Point::new(1.0, 2.0), end: Point::new(3.0, 4.0), aspect: 0.5, focal: None });
        g.swatch = Some("Sky".into());
        let v = gradient_params(&g);
        assert_eq!((v["start"].clone(), v["end"].clone(), v["aspect"].clone()), (json!([1.0, 2.0]), json!([3.0, 4.0]), json!(50.0)));
        assert_eq!(v["swatch"], "Sky");
        assert!(v["stops"][0].get("swatch").is_none(), "unlinked stops carry no link");
        g.gradient.stops[1].swatch = Some("Ink".into());
        g.gradient.stops[1].tint = 0.25;
        let v = gradient_params(&g);
        assert_eq!((v["stops"][1]["swatch"].clone(), v["stops"][1]["tint"].clone()), (json!("Ink"), json!(25.0)));
        assert_eq!(color_json(&Color::gray(0.25)), json!({"gray": 0.25}));
    }
}
