//! The default SVG filters: the filters Effect › SVG Filters and Apply SVG Filter list in every
//! document. The set, the names and the values are VectorCraft's own, written with the filter
//! primitives of SVG 1.1 (<https://www.w3.org/TR/SVG11/filters.html>). A default is not part of a
//! document until it is applied or edited there: the document then holds a copy under the
//! default's name, and that copy stands in the default's place in the lists. Every default computes
//! in sRGB (`color-interpolation-filters="sRGB"`), so art a filter passes through keeps its colors.

use vectorcraft_doc::SvgFilter;

/// A default SVG filter: its name (the `<filter>` element's `id`) and the element's XML.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultFilter {
    pub name: &'static str,
    pub xml: &'static str,
}

/// The default filters, in the order the lists show them.
pub const DEFAULT_FILTERS: [DefaultFilter; 14] = [
    DefaultFilter {
        name: "Soft Blur",
        xml: r#"<filter id="Soft Blur" x="-20%" y="-20%" width="140%" height="140%" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="2"/></filter>"#,
    },
    DefaultFilter {
        name: "Heavy Blur",
        xml: r#"<filter id="Heavy Blur" x="-50%" y="-50%" width="200%" height="200%" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="6"/></filter>"#,
    },
    DefaultFilter {
        name: "Soft Shadow",
        xml: r#"<filter id="Soft Shadow" x="-30%" y="-30%" width="170%" height="170%" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceAlpha" stdDeviation="3"/><feOffset dx="4" dy="4"/><feComponentTransfer><feFuncA type="linear" slope="0.5"/></feComponentTransfer><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>"#,
    },
    DefaultFilter {
        name: "Hard Shadow",
        xml: r#"<filter id="Hard Shadow" x="-20%" y="-20%" width="150%" height="150%" color-interpolation-filters="sRGB"><feOffset in="SourceAlpha" dx="3" dy="3"/><feComponentTransfer><feFuncA type="linear" slope="0.6"/></feComponentTransfer><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>"#,
    },
    DefaultFilter {
        name: "Glow",
        xml: r##"<filter id="Glow" x="-40%" y="-40%" width="180%" height="180%" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceAlpha" stdDeviation="4" result="spread"/><feFlood flood-color="#ffd24a"/><feComposite in2="spread" operator="in"/><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>"##,
    },
    DefaultFilter {
        name: "Bevel",
        xml: r##"<filter id="Bevel" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceAlpha" stdDeviation="2" result="height"/><feSpecularLighting in="height" surfaceScale="4" specularConstant="0.8" specularExponent="16" lighting-color="#ffffff" result="light"><feDistantLight azimuth="225" elevation="45"/></feSpecularLighting><feComposite in="light" in2="SourceAlpha" operator="in" result="lit"/><feComposite in="SourceGraphic" in2="lit" operator="arithmetic" k1="0" k2="1" k3="1" k4="0"/></filter>"##,
    },
    DefaultFilter {
        name: "Emboss",
        xml: r#"<filter id="Emboss" color-interpolation-filters="sRGB"><feConvolveMatrix order="3" kernelMatrix="-2 -1 0 -1 1 1 0 1 2" preserveAlpha="true"/></filter>"#,
    },
    DefaultFilter {
        name: "Thicken",
        xml: r#"<filter id="Thicken" x="-20%" y="-20%" width="140%" height="140%" color-interpolation-filters="sRGB"><feMorphology operator="dilate" radius="1.5"/></filter>"#,
    },
    DefaultFilter {
        name: "Thin",
        xml: r#"<filter id="Thin" color-interpolation-filters="sRGB"><feMorphology operator="erode" radius="1.5"/></filter>"#,
    },
    DefaultFilter {
        name: "Rough Edges",
        xml: r#"<filter id="Rough Edges" x="-20%" y="-20%" width="140%" height="140%" color-interpolation-filters="sRGB"><feTurbulence type="turbulence" baseFrequency="0.05" numOctaves="2" result="waves"/><feDisplacementMap in="SourceGraphic" in2="waves" scale="6" xChannelSelector="R" yChannelSelector="G"/></filter>"#,
    },
    DefaultFilter {
        name: "Grain",
        xml: r#"<filter id="Grain" color-interpolation-filters="sRGB"><feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2"/><feColorMatrix type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 0.35 0"/><feComposite in2="SourceAlpha" operator="in" result="specks"/><feMerge><feMergeNode in="SourceGraphic"/><feMergeNode in="specks"/></feMerge></filter>"#,
    },
    DefaultFilter {
        name: "Grayscale",
        xml: r#"<filter id="Grayscale" color-interpolation-filters="sRGB"><feColorMatrix type="saturate" values="0"/></filter>"#,
    },
    DefaultFilter {
        name: "Invert",
        xml: r#"<filter id="Invert" color-interpolation-filters="sRGB"><feComponentTransfer><feFuncR type="table" tableValues="1 0"/><feFuncG type="table" tableValues="1 0"/><feFuncB type="table" tableValues="1 0"/></feComponentTransfer></filter>"#,
    },
    DefaultFilter {
        name: "Hue Shift",
        xml: r#"<filter id="Hue Shift" color-interpolation-filters="sRGB"><feColorMatrix type="hueRotate" values="180"/></filter>"#,
    },
];

/// The default filter named `name`.
pub fn default_filter(name: &str) -> Option<DefaultFilter> {
    DEFAULT_FILTERS.iter().copied().find(|f| f.name == name)
}

impl DefaultFilter {
    /// The filter as a document holds it once it is applied or edited there.
    pub fn to_filter(self) -> SvgFilter {
        SvgFilter { name: self.name.to_string(), xml: self.xml.to_string() }
    }
}

/// A filter of the lists ([`listed`]), and whether it is a default the document does not hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub filter: SvgFilter,
    /// A default the document has no filter named for: applying or editing it adds a copy.
    pub default: bool,
}

/// The filters Effect › SVG Filters and Apply SVG Filter list for a document that holds `held`,
/// and how many of them come first as the defaults: each default in [`DEFAULT_FILTERS`]' order (the
/// document's filter of that name in its place), then the document's other filters in their order.
pub fn listed(held: &[SvgFilter]) -> (Vec<Listed>, usize) {
    let mut out: Vec<Listed> = DEFAULT_FILTERS
        .iter()
        .map(|d| match held.iter().find(|f| f.name == d.name) {
            Some(f) => Listed { filter: f.clone(), default: false },
            None => Listed { filter: d.to_filter(), default: true },
        })
        .collect();
    let defaults = out.len();
    out.extend(held.iter().filter(|f| default_filter(&f.name).is_none()).map(|f| Listed { filter: f.clone(), default: false }));
    (out, defaults)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::Effect;
    use vectorcraft_geom::{Affine, Point};

    use super::*;
    use crate::pixel::{Channels, PixelSpace};
    use crate::svg_filter::{SVG_FILTER, SvgFilterFx, check, with_id};
    use crate::{PixelFx, RasterFx, raster_effects};

    /// A `w` × `w` raster whose pixels are one point wide.
    fn space(w: usize) -> PixelSpace {
        let wd = w as f64;
        PixelSpace {
            to_doc: Affine::scale(1.0),
            px: 1.0,
            center: Point::new(wd / 2.0, wd / 2.0),
            size: vectorcraft_geom::Vec2::new(wd, wd),
            channels: Channels::Rgb,
            line: 1.0,
            extent: 0.5 * wd.hypot(wd),
        }
    }

    /// The default named `name` as the renderer reads it.
    fn fx(name: &str) -> SvgFilterFx {
        let d = default_filter(name).unwrap_or_else(|| panic!("no default {name}"));
        let e = Effect { id: SVG_FILTER.into(), params: json!({"name": d.name, "xml": d.xml}), visible: true };
        match raster_effects(&[e]).first() {
            Some(RasterFx::Pixel(PixelFx::Svg(f))) => f.clone(),
            other => panic!("{name}: {other:?}"),
        }
    }

    /// A 40 × 40 raster with an opaque square of `rgb` from 10 to 30, run through default `name`.
    fn run(name: &str, rgb: [u8; 3]) -> Vec<u8> {
        let mut data: Vec<u8> = (0..40usize)
            .flat_map(|y| (0..40usize).map(move |x| (x, y)))
            .flat_map(|(x, y)| if (10..30).contains(&x) && (10..30).contains(&y) { [rgb[0], rgb[1], rgb[2], 255] } else { [0; 4] })
            .collect();
        fx(name).apply(&mut data, 40, 40, &space(40));
        data
    }

    fn at(d: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * 40 + x) * 4;
        [d[i], d[i + 1], d[i + 2], d[i + 3]]
    }

    #[test]
    fn every_default_is_a_filter_the_check_takes_under_its_own_name() {
        let mut seen = std::collections::BTreeSet::new();
        for d in DEFAULT_FILTERS {
            assert_eq!(check(d.xml), Ok(d.name.to_string()), "{}", d.name);
            assert!(seen.insert(d.name), "{} is listed twice", d.name);
            assert_eq!(d.name, d.name.trim());
            // Edit SVG Filter keeps the name by writing it as the id: a default's code comes back as it is.
            assert_eq!(with_id(d.xml, d.name).as_deref(), Some(d.xml), "{}", d.name);
            assert_eq!(default_filter(d.name), Some(d));
            assert!(d.xml.contains(r#" color-interpolation-filters="sRGB""#), "{} computes in sRGB", d.name);
        }
        assert!(DEFAULT_FILTERS.len() <= SvgFilter::MAX);
        assert_eq!(default_filter("soft blur"), None, "names match exactly");
    }

    #[test]
    fn every_default_draws_and_is_written_to_svg_as_a_filter() {
        let plain = {
            let mut d = vec![0u8; 40 * 40 * 4];
            for y in 10..30 {
                for x in 10..30 {
                    d[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4].copy_from_slice(&[200, 60, 20, 255]);
                }
            }
            d
        };
        for d in DEFAULT_FILTERS {
            assert_ne!(run(d.name, [200, 60, 20]), plain, "{} changes what is drawn", d.name);
            assert!(fx(d.name).writes_as_svg11(), "{} is written as SVG 1.1", d.name);
        }
    }

    /// The color defaults against SVG 1.1's own arithmetic, in sRGB as each filter asks.
    #[test]
    fn the_color_defaults_compute_what_svg_1_1_defines() {
        let near = |got: [u8; 4], want: [u8; 4]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1);
        // feComponentTransfer, type="table" with values 1 0: C' = 1 - C (§15.11).
        let inv = at(&run("Invert", [200, 60, 20]), 20, 20);
        assert!(near(inv, [55, 195, 235, 255]), "{inv:?}");
        // feColorMatrix type="saturate" values="0": every channel is 0.213 R + 0.715 G + 0.072 B (§15.10).
        let grey = at(&run("Grayscale", [200, 60, 20]), 20, 20);
        let l = (0.213 * 200.0 + 0.715 * 60.0 + 0.072 * 20.0f64).round() as u8;
        assert!(near(grey, [l, l, l, 255]), "{grey:?} for {l}");
        // feColorMatrix type="hueRotate" values="180" (§15.10): cos = -1 and sin = 0 leave
        // R' = (2·0.213 - 1) R + 2·0.715 G + 2·0.072 B, and likewise for G' and B'.
        let hue = at(&run("Hue Shift", [200, 60, 20]), 20, 20);
        let lum2 = 2.0 * (0.213 * 200.0 + 0.715 * 60.0 + 0.072 * 20.0f64);
        let turned = [lum2 - 200.0, lum2 - 60.0, lum2 - 20.0].map(|v| v.round().clamp(0.0, 255.0) as u8);
        assert!(near(hue, [turned[0], turned[1], turned[2], 255]), "{hue:?} for {turned:?}");
    }

    /// The shape defaults against what their primitives define: an offset copy of the alpha behind
    /// the art (feOffset, §15.21), and a shape grown or shrunk by its radius (feMorphology, §15.20).
    #[test]
    fn the_shape_defaults_move_and_resize_the_alpha() {
        let shadow = run("Hard Shadow", [200, 60, 20]);
        assert_eq!(at(&shadow, 20, 20), [200, 60, 20, 255], "the art is on top");
        // Right of and below the art by up to 3: black at 60% alpha (premultiplied 153).
        assert_eq!(at(&shadow, 31, 31), [0, 0, 0, 153]);
        assert_eq!(at(&shadow, 34, 34)[3], 0, "past the offset");
        assert_eq!(at(&shadow, 8, 8)[3], 0, "nothing up and left");
        let thick = run("Thicken", [200, 60, 20]);
        assert_eq!((at(&thick, 9, 20)[3], at(&thick, 7, 20)[3]), (255, 0), "grown by its radius");
        let thin = run("Thin", [200, 60, 20]);
        assert_eq!((at(&thin, 10, 20)[3], at(&thin, 13, 20)[3]), (0, 255), "shrunk by its radius");
        let soft = run("Soft Shadow", [200, 60, 20]);
        assert_eq!(at(&soft, 20, 20), [200, 60, 20, 255]);
        let below = at(&soft, 33, 33);
        assert!(below[3] > 0 && below[3] < 128 && below[0] == 0, "a blurred, half-strength shadow: {below:?}");
    }

    #[test]
    fn the_lists_show_the_defaults_first_with_the_documents_copy_in_place() {
        let (all, defaults) = listed(&[]);
        assert_eq!((all.len(), defaults), (DEFAULT_FILTERS.len(), DEFAULT_FILTERS.len()));
        assert!(all.iter().all(|l| l.default));
        let mine = SvgFilter { name: "Mine".into(), xml: "<filter id=\"Mine\"><feOffset/></filter>".into() };
        let glow = SvgFilter { name: "Glow".into(), xml: "<filter id=\"Glow\"><feOffset dx=\"1\"/></filter>".into() };
        let (all, defaults) = listed(&[mine.clone(), glow.clone()]);
        assert_eq!((all.len(), defaults), (DEFAULT_FILTERS.len() + 1, DEFAULT_FILTERS.len()));
        let at = DEFAULT_FILTERS.iter().position(|d| d.name == "Glow").unwrap();
        assert_eq!(all[at], Listed { filter: glow, default: false }, "the document's Glow where the default was");
        assert_eq!(all.last(), Some(&Listed { filter: mine, default: false }));
        assert_eq!(all.iter().filter(|l| l.default).count(), DEFAULT_FILTERS.len() - 1);
    }
}
