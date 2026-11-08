use serde_json::json;
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Affine, Point, Rect};

use super::*;
use crate::pixel::Channels;
use crate::{PixelFx, RasterFx, raster_effects};

/// A `w` × `h` raster whose pixels are `px` points wide (origin top-left), at a raster effects
/// resolution whose pixels are `line` points wide.
fn space(w: usize, h: usize, px: f64, line: f64) -> PixelSpace {
    let (wd, hd) = (w as f64 * px, h as f64 * px);
    PixelSpace { to_doc: Affine::scale(px), px, center: Point::new(wd / 2.0, hd / 2.0), channels: Channels::Rgb, line, extent: 0.5 * wd.hypot(hd) }
}

/// `w` × `h` premultiplied pixels: `f(x, y)` each.
fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| f(x, y)).collect()
}

fn at(d: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

/// The effect `svg.filter` with `params`, as the renderer reads it.
fn filter(params: serde_json::Value) -> SvgFilterFx {
    let e = Effect { id: SVG_FILTER.into(), params, visible: true };
    match raster_effects(&[e]).first() {
        Some(RasterFx::Pixel(PixelFx::Svg(f))) => f.clone(),
        other => panic!("{other:?}"),
    }
}

/// A blue square of `size` pixels inset by `inset` in a transparent raster `w` wide.
fn blue_square(w: usize, inset: usize) -> Vec<u8> {
    image(w, w, |x, y| if (inset..w - inset).contains(&x) && (inset..w - inset).contains(&y) { [0, 0, 255, 255] } else { [0; 4] })
}

const RED_FLOOD: &str = r##"<filter id="Red"><feFlood flood-color="#ff0000"/><feComposite in2="SourceGraphic" operator="in"/></filter>"##;

#[test]
fn check_names_a_filter_by_its_id() {
    assert_eq!(check(RED_FLOOD), Ok("Red".into()));
    assert_eq!(check(&format!("  {RED_FLOOD}\n")), Ok("Red".into()), "spaces around the element");
    let linked = r#"<filter id=" Shadow 1 "><feImage xlink:href="data:image/png;base64,AAAA"/></filter>"#;
    assert_eq!(check(linked), Ok("Shadow 1".into()), "the xlink prefix is declared for it");
}

#[test]
fn check_refuses_what_is_not_one_drawable_filter() {
    for (xml, why) in [
        ("", "give one <filter> element"),
        ("<filter id=\"a\">", "isn't valid XML"),
        ("<g id=\"a\"><feFlood/></g>", "give one <filter> element"),
        ("<filter id=\"a\"><feFlood/></filter><filter id=\"b\"><feFlood/></filter>", "give one <filter> element"),
        ("text<filter id=\"a\"><feFlood/></filter>", "give one <filter> element"),
        ("<filter><feFlood/></filter>", "needs an id"),
        ("<filter id=\"  \"><feFlood/></filter>", "needs an id"),
        ("<filter id=\"a\"></filter>", "no primitives"),
        ("<filter id=\"a\"><feTurbulence numOctaves=\"11\"/></filter>", "numOctaves is at most 10"),
        ("<filter id=\"a\"><feConvolveMatrix order=\"3 12\" kernelMatrix=\"1\"/></filter>", "order is at most 9"),
        ("<filter id=\"a\"><feConvolveMatrix order=\"65536+65536\" kernelMatrix=\"1\"/></filter>", "order is at most 9"),
        ("<filter id=\"a\"><feConvolveMatrix order=\"3 x\" kernelMatrix=\"1\"/></filter>", "order is at most 9"),
        ("<filter id=\"a\"><notAPrimitive/></filter>", "<notAPrimitive> isn't an element of an SVG filter"),
        ("<filter id=\"a\"><feOffset/><script>alert(1)</script></filter>", "<script> isn't an element"),
        ("<filter id=\"a\"><feMerge><feMergeNode/><a/></feMerge></filter>", "<a> isn't an element"),
        ("<filter id=\"a\" onload=\"x()\"><feOffset/></filter>", "no event attributes such as onload"),
        ("<filter id=\"a\"><feOffset ONCLICK=\"x()\"/></filter>", "no event attributes such as ONCLICK"),
        ("<filter id=\"a\"><feImage xlink:href=\"javascript:x()\"/></filter>", "WebP data: URLs"),
        ("<filter id=\"a\"><feImage href=\"photo.png\"/></filter>", "WebP data: URLs"),
        ("<filter id=\"a\"><feImage href=\"data:image/svg+xml;base64,PHN2Zy8+\"/></filter>", "WebP data: URLs"),
        ("<filter id=\"a\"><feImage href=\"data:application/gzip;base64,H4sI\"/></filter>", "WebP data: URLs"),
        ("<filter id=\"a\"><feImage href=\"data:;base64,AAAA\"/></filter>", "WebP data: URLs"),
        ("<filter id=\"a\" width=\"0\"><feOffset/></filter>", "can't be drawn"),
    ] {
        let e = check(xml).expect_err(xml);
        assert!(e.contains(why), "{xml}: {e}");
    }
    let many = format!("<filter id=\"a\">{}</filter>", "<feOffset/>".repeat(MAX_PRIMITIVES + 1));
    assert!(check(&many).is_err_and(|e| e.contains("at most 64 primitives")));
    let titled = format!("<filter id=\"a\"><title>T</title><desc>D</desc>{}</filter>", "<feOffset/>".repeat(MAX_PRIMITIVES));
    assert_eq!(check(&titled), Ok("a".into()), "a title and a description aren't primitives");
    assert!(check("<filter id=\"a\"><title>T</title></filter>").is_err_and(|e| e.contains("no primitives")));
    let long = format!("<filter id=\"{}\"><feOffset/></filter>", "x".repeat(MAX_NAME + 1));
    assert!(check(&long).is_err_and(|e| e.contains("at most 255 characters")));
    let big = format!("<filter id=\"a\"><feOffset/><!--{}--></filter>", " ".repeat(SvgFilter::MAX_XML));
    assert!(check(&big).is_err_and(|e| e.contains("at most 64 KB")));
}

#[test]
fn with_id_renames_only_the_filter() {
    let xml = r#"<filter id="Old" x="0"><feOffset result="Old"/></filter>"#;
    assert_eq!(with_id(xml, "New").as_deref(), Some(r#"<filter id="New" x="0"><feOffset result="Old"/></filter>"#));
    assert_eq!(with_id("<filter><feOffset/></filter>", "New"), None);
}

#[test]
fn filters_in_lists_the_documents_filters_it_takes() {
    let svg = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\"><svg xmlns=\"http://www.w3.org/2000/svg\"><defs>{RED_FLOOD}<filter id=\"Empty\"/><filter id=\"Off\"><feOffset dx=\"2\"/></filter></defs></svg>"
    );
    let (found, skipped) = filters_in(&svg);
    assert_eq!(found.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Red", "Off"]);
    assert_eq!(found[0].1, RED_FLOOD, "each filter's XML as it is written");
    assert_eq!(skipped, 1, "the filter without primitives");
}

/// [`filters_of`] on `svg` read as SVG import reads it (with DTDs).
fn filters_in(svg: &str) -> (Vec<(String, String)>, usize) {
    let doc = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).unwrap();
    filters_of(&doc, svg, usize::MAX, |_| false)
}

#[test]
fn filters_of_stops_at_the_room_left_and_skips_taken_and_styled_filters() {
    let many = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
        (0..200).map(|i| format!("<filter id=\"f{i}\"><feOffset/></filter>")).collect::<String>()
    );
    let doc = roxmltree::Document::parse(&many).unwrap();
    let (found, skipped) = filters_of(&doc, &many, 3, |name| name == "f1");
    assert_eq!(found.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["f0", "f2", "f3"]);
    assert_eq!(skipped, 197);
    // A filter whose look comes from the document's styles isn't taken.
    for styled in [
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>.r{flood-color:red}</style><filter id=\"a\"><feFlood class=\"r\"/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><filter id=\"a\"><feFlood flood-color=\"currentColor\"/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><filter id=\"a\"><feFlood flood-color=\"CURRENTcolor\"/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><defs color-interpolation-filters=\"sRGB\"><filter id=\"a\"><feGaussianBlur stdDeviation=\"2\"/></filter></defs></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>@media screen { feFlood { flood-color: red } }</style><filter id=\"a\"><feFlood/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>#a feOffset{dx:2}</style><filter id=\"a\"><feOffset/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>.s { content: \"{ /*\" } feFlood { flood-color: red }</style><filter id=\"a\"><feFlood/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>.outer { fill: blue; &amp; #a { color: red } }</style><filter id=\"a\"><feFlood/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>@scope (#a) { :scope { color: red } }</style><filter id=\"a\"><feFlood/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>svg { color-interpolation-filters: sRGB }</style><filter id=\"a\"><feGaussianBlur stdDeviation=\"2\"/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>feFlood[data-t=\\;] { flood-color: red }</style><filter id=\"a\"><feFlood/></filter></svg>",
    ] {
        let doc = roxmltree::Document::parse(styled).unwrap();
        assert_eq!(filters_of(&doc, styled, usize::MAX, |_| false), (vec![], 1), "{styled}");
    }
    // A style sheet for the shapes alone (as CSS export writes), applying the filter to them
    // included, doesn't reach the filter.
    for shapes in [
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>.cls-1{fill:#f00}</style><filter id=\"a\"><feOffset/></filter></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>/* #a */ .shape { filter: url(#a) }</style><filter id=\"a\"><feOffset/></filter></svg>",
        // An identifier named currentColor takes no color.
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><filter id=\"currentColor\"><feOffset result=\"currentColor\"/></filter></svg>",
    ] {
        let doc = roxmltree::Document::parse(shapes).unwrap();
        assert_eq!(filters_of(&doc, shapes, usize::MAX, |_| false).0.len(), 1, "{shapes}");
    }
}

#[test]
fn deeply_nested_filters_are_refused_before_they_are_parsed() {
    let deep = format!("<filter id=\"a\"><feMerge>{}<feMergeNode/>{}</feMerge></filter>", "<g>".repeat(20), "</g>".repeat(20));
    assert!(check(&deep).is_err_and(|e| e.contains("nest at most 16 deep")));
    assert_eq!(with_id(&deep, "b"), None);
    let e = Effect { id: SVG_FILTER.into(), params: json!({ "xml": deep }), visible: true };
    assert!(raster_effects(&[e]).is_empty(), "drawn without it");
    // Markup in comments, CDATA and attribute values doesn't count.
    let quoted =
        format!("<filter id=\"a\" x=\"{}\"><!--{}--><feOffset/><![CDATA[{}]]></filter>", "&lt;g>".repeat(20), "<g>".repeat(20), "<g>".repeat(20));
    assert_eq!(check(&quoted), Ok("a".into()));
    let gt = format!("<filter id=\"a\" title='{}'>{}<feOffset/></filter>", "x>".repeat(20), "<!-- > -->".repeat(3));
    assert_eq!(check(&gt), Ok("a".into()));
}

#[test]
fn a_filter_replaces_the_pixels_with_its_result() {
    let w = 20;
    let mut data = blue_square(w, 5);
    filter(json!({ "xml": RED_FLOOD })).apply(&mut data, w, w, &space(w, w, 1.0, 1.0));
    assert_eq!(at(&data, w, 10, 10), [255, 0, 0, 255], "inside: the flood, cut to the source");
    assert_eq!(at(&data, w, 1, 1), [0; 4], "outside the source");
}

#[test]
fn lengths_are_in_points_times_scale() {
    let w = 40;
    let shift = r#"<filter id="Shift" x="-1" y="-1" width="3" height="3"><feOffset dx="4"/></filter>"#;
    let run = |px: f64, scale: f64| {
        let n = (w as f64 / px) as usize;
        let mut data = image(n, n, |x, y| {
            let (x, y) = (x as f64 * px, y as f64 * px);
            if (10.0..20.0).contains(&x) && (10.0..20.0).contains(&y) { [0, 0, 255, 255] } else { [0; 4] }
        });
        filter(json!({ "xml": shift, "scale": scale })).apply(&mut data, n, n, &space(n, n, px, px));
        let column = |x: f64| at(&data, n, (x / px) as usize, (15.0 / px) as usize)[3];
        (column(9.5), column(21.0), column(23.5), column(25.0))
    };
    // The square spans 10..20 pt: moved by 4 pt, 14..24.
    assert_eq!(run(1.0, 1.0), (0, 255, 255, 0));
    assert_eq!(run(0.5, 1.0), (0, 255, 255, 0), "the same in points at another zoom");
    // One unit of the filter is half a point: moved by 2 pt, 12..22.
    assert_eq!(run(1.0, 0.5), (0, 255, 0, 0));
}

/// A user-space region is placed from the effect's origin.
#[test]
fn user_space_starts_at_the_origin() {
    let w = 40;
    // Flood the square, cut to the region x 0..20 of the filter's user space.
    let half = r##"<filter id="Half" filterUnits="userSpaceOnUse" x="0" y="0" width="20" height="40"><feFlood flood-color="#ff0000"/><feComposite in2="SourceGraphic" operator="in"/></filter>"##;
    let run = |origin: [f64; 2]| {
        let mut data = image(w, w, |_, _| [0, 0, 255, 255]);
        filter(json!({ "xml": half, "origin": origin })).apply(&mut data, w, w, &space(w, w, 1.0, 1.0));
        (at(&data, w, 5, 20)[3], at(&data, w, 25, 20)[3])
    };
    assert_eq!(run([0.0, 0.0]), (255, 0));
    assert_eq!(run([20.0, 0.0]), (0, 255), "the region 20 pt to the right");
    let b = Rect::new(0.0, 0.0, 40.0, 40.0);
    assert!((filter(json!({ "xml": half, "origin": [-10.0, 0.0] })).outset(b) - 10.0).abs() < 1e-9);
}

#[test]
fn runs_no_finer_than_the_raster_effects_resolution() {
    let w = 16;
    // Columns one view pixel wide, alternately opaque and clear.
    let stripes = image(w, w, |x, _| if x % 2 == 0 { [0, 0, 255, 255] } else { [0; 4] });
    let keep = r#"<filter id="Keep" x="0" y="0" width="1" height="1"><feOffset/></filter>"#;
    let mut fine = stripes.clone();
    filter(json!({ "xml": keep })).apply(&mut fine, w, w, &space(w, w, 0.25, 0.25));
    assert_eq!(at(&fine, w, 4, 4)[3], 255);
    assert_eq!(at(&fine, w, 5, 4)[3], 0, "at the view's resolution the stripes stay");
    let mut coarse = stripes.clone();
    filter(json!({ "xml": keep })).apply(&mut coarse, w, w, &space(w, w, 0.25, 1.0));
    let (a, b) = (at(&coarse, w, 4, 4)[3], at(&coarse, w, 5, 4)[3]);
    assert!(a < 255 && b > 0, "four view pixels to a raster pixel: the stripes blend ({a}, {b})");
}

#[test]
fn draws_nothing_new_without_content_or_a_readable_filter() {
    let w = 8;
    let mut empty = vec![0; w * w * 4];
    filter(json!({ "xml": RED_FLOOD })).apply(&mut empty, w, w, &space(w, w, 1.0, 1.0));
    assert!(empty.iter().all(|v| *v == 0), "a flood cut to a source that draws nothing");
    // A filter that doesn't read the source draws on art that draws no pixels: the raster is its box.
    let mut clear = vec![0; w * w * 4];
    filter(json!({ "xml": "<filter id=\"G\"><feFlood flood-color=\"#00ff00\"/></filter>" })).apply(&mut clear, w, w, &space(w, w, 1.0, 1.0));
    assert_eq!(at(&clear, w, 4, 4), [0, 255, 0, 255]);
    let e = Effect { id: SVG_FILTER.into(), params: json!({ "xml": "<g/>" }), visible: true };
    assert!(raster_effects(&[e]).is_empty(), "not a filter: the object is drawn without it");
    let e = Effect { id: SVG_FILTER.into(), params: json!({ "xml": "<filter id=\"x\" width=\"0\"><feOffset/></filter>" }), visible: true };
    assert!(raster_effects(&[e]).is_empty(), "a filter resvg can't draw: the object is drawn without it");
}

#[test]
fn outset_is_how_far_the_region_reaches() {
    let b = Rect::new(0.0, 0.0, 100.0, 50.0);
    assert!((filter(json!({ "xml": RED_FLOOD })).outset(b) - 10.0).abs() < 1e-9, "the default region: 10% of the width");
    let wide = r#"<filter id="w" x="-50%" y="0" width="200%" height="100%"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": wide })).outset(b) - 50.0).abs() < 1e-9);
    let user = r#"<filter id="u" filterUnits="userSpaceOnUse" x="-10" y="0" width="60" height="25"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": user, "scale": 2.0 })).outset(b) - 20.0).abs() < 1e-9, "user units times scale");
    let inside = r#"<filter id="i" x="0" y="0" width="1" height="1"><feOffset/></filter>"#;
    assert_eq!(filter(json!({ "xml": inside })).outset(b), 0.0);
    // User-space lengths in points: -30 pt is -40 user units.
    let points = r#"<filter id="p" filterUnits="userSpaceOnUse" x="-30pt" y="0" width="100" height="50"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": points })).outset(b) - 40.0).abs() < 1e-9);
    // In user space, a percentage is a share of the object's box; em, ex and a missing length
    // take the default region's share of it.
    let viewport = r#"<filter id="v" filterUnits="userSpaceOnUse" x="-50%" y="0" width="10" height="10"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": viewport })).outset(b) - 50.0).abs() < 1e-9);
    let far = r#"<filter id="r" filterUnits="userSpaceOnUse" x="-1000%" y="0" width="3000%" height="10"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": far })).outset(b) - 1900.0).abs() < 1e-9, "however far it reaches");
    let font = r#"<filter id="f" filterUnits="userSpaceOnUse" x="-100em" y="0" width="10" height="10"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": font })).outset(b) - 10.0).abs() < 1e-9, "relative to the font");
    let default = r#"<filter id="d" filterUnits="userSpaceOnUse"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": default })).outset(b) - 10.0).abs() < 1e-9);
    // In the bounding box, a length with a unit is invalid: the default.
    let unit = r#"<filter id="u" x="-5mm"><feOffset/></filter>"#;
    assert!((filter(json!({ "xml": unit })).outset(b) - 10.0).abs() < 1e-9);
}

#[test]
fn a_filter_keeps_the_prefixes_an_ancestor_declares() {
    let svg = r#"<svg:svg xmlns:svg="http://www.w3.org/2000/svg" xmlns:x="urn:other"><svg:defs><svg:filter id="Pre"><svg:feOffset dx="1"/></svg:filter></svg:defs></svg:svg>"#;
    let (found, skipped) = filters_in(svg);
    assert_eq!((found.len(), skipped), (1, 0));
    assert_eq!(
        found[0].1, r#"<svg:filter xmlns:svg="http://www.w3.org/2000/svg" id="Pre"><svg:feOffset dx="1"/></svg:filter>"#,
        "only the prefix it uses"
    );
    // A declaration on the filter itself, with spaces around its `=`, is kept as it is.
    let own = r#"<svg xmlns="http://www.w3.org/2000/svg"><defs><s:filter xmlns:s = "http://www.w3.org/2000/svg" id="Own"><s:feOffset/></s:filter></defs></svg>"#;
    let (found, skipped) = filters_in(own);
    assert_eq!((found.len(), skipped), (1, 0));
    assert_eq!(found[0].1, r#"<s:filter xmlns:s = "http://www.w3.org/2000/svg" id="Own"><s:feOffset/></s:filter>"#);
    // The same declaration again on the filter, and text like one inside an attribute's value.
    let again = r#"<s:svg xmlns:s="http://www.w3.org/2000/svg"><s:filter xmlns:s="http://www.w3.org/2000/svg" id="A"><s:feOffset/></s:filter><s:filter id="B" color-profile=" xmlns:s = x"><s:feOffset/></s:filter></s:svg>"#;
    let (found, skipped) = filters_in(again);
    assert_eq!((found.len(), skipped), (2, 0), "{found:?}");
    assert!(found[1].1.starts_with(r#"<s:filter xmlns:s="http://www.w3.org/2000/svg" id="B""#), "{found:?}");
}

#[test]
fn primitive_forms_tell_filters_with_the_same_primitives_apart() {
    let a = primitive_forms(r#"<filter id="a"><feOffset dx="1"/></filter>"#).unwrap();
    let b = primitive_forms(r#"<filter id="b"><feOffset dx="2"/></filter>"#).unwrap();
    assert_eq!(a.len(), 1);
    assert_ne!(a, b);
    assert_eq!(primitive_forms(r#"<filter id="c"><feOffset dx="1"/></filter>"#), Some(a), "the id doesn't count");
}

#[test]
fn costly_parameters_are_capped() {
    for (xml, why) in [
        ("<filter id=\"a\"><feMorphology radius=\"101\"/></filter>", "radius is at most 100"),
        ("<filter id=\"a\"><feMorphology radius=\"2 1e9\"/></filter>", "radius is at most 100"),
        ("<filter id=\"a\"><feMorphology radius=\"50+500\"/></filter>", "radius is at most 100"),
        ("<filter id=\"a\"><feGaussianBlur stdDeviation=\"501\"/></filter>", "stdDeviation is at most 500"),
        ("<filter id=\"a\"><feDropShadow stdDeviation=\"1 9e9\"/></filter>", "stdDeviation is at most 500"),
        (
            "<filter id=\"a\"><feDiffuseLighting kernelUnitLength=\"101\"><feDistantLight/></feDiffuseLighting></filter>",
            "kernelUnitLength is at most 100",
        ),
    ] {
        let e = check(xml).expect_err(xml);
        assert!(e.contains(why), "{xml}: {e}");
    }
    assert_eq!(check("<filter id=\"a\"><feMorphology radius=\"100\"/></filter>"), Ok("a".into()));
    // usvg also reads attributes in the SVG, XLink and XML namespaces by their local names.
    for prefixed in [
        "<filter id=\"a\" xmlns:s=\"http://www.w3.org/2000/svg\"><feMorphology s:radius=\"1000\"/></filter>",
        "<filter id=\"a\" xmlns:s=\"http://www.w3.org/2000/svg\"><feGaussianBlur s:stdDeviation=\"1e6\"/></filter>",
    ] {
        assert!(check(prefixed).is_err(), "{prefixed}");
    }
}

/// A filter is drawn and written rebuilt from its filter elements and their filter attributes:
/// no comments, text, descriptions, other attributes or prefixes.
#[test]
fn the_filter_is_drawn_and_written_from_its_filter_parts_alone() {
    let xml = concat!(
        "<filter id=\"a\" xmlns:s=\"http://www.w3.org/2000/svg\" data-x=\"1\"><!-- note --><desc>&lt;script&gt;</desc><title>T</title>",
        "<feFlood flood-color=\"blue\" style=\"flood-color: red; fill: url(#x); flood-opacity: url(#y)\" foo=\"&quot;\"/>",
        "<feMorphology s:radius=\"2\"/><feImage xlink:href=\"data:image/png;base64,AAAA\"/></filter>",
    );
    let f = filter(json!({ "xml": xml }));
    assert_eq!(
        f.xml(),
        "<filter id=\"a\"><feFlood flood-color=\"red\"/><feMorphology radius=\"2\"/><feImage href=\"data:image/png;base64,AAAA\"/></filter>"
    );
    // An image keeps how it is scaled, as a style declaration.
    for image in [
        "<filter id=\"i\"><feImage image-rendering=\"pixelated\" href=\"data:image/png;base64,AAAA\"/></filter>",
        "<filter id=\"i\"><feImage style=\"image-rendering: pixelated; fill: red\" href=\"data:image/png;base64,AAAA\"/></filter>",
    ] {
        assert_eq!(
            filter(json!({ "xml": image })).xml(),
            "<filter id=\"i\"><feImage href=\"data:image/png;base64,AAAA\" style=\"image-rendering:pixelated\"/></filter>",
            "{image}"
        );
    }
    // `currentColor` keeps the color it names.
    let colored = filter(json!({ "xml": "<filter id=\"c\" color=\"red\"><feFlood flood-color=\"currentColor\" style=\"color: blue\"/></filter>" }));
    assert_eq!(colored.xml(), "<filter id=\"c\" color=\"red\"><feFlood flood-color=\"currentColor\" color=\"blue\"/></filter>");
}

/// An image of a filter is a PNG, JPEG, GIF or WebP, by its bytes, within the pixel cap by its header.
#[test]
fn filter_images_are_small_raster_images() {
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![];
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut out),
            &vec![0; (w * h * 4) as usize],
            w,
            h,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        out
    }
    let raster_image = |data| raster_image_sized(data).map(|(kind, _)| kind);
    assert!(matches!(raster_image(Arc::new(png(2, 2))), Some(usvg::ImageKind::PNG(_))));
    assert_eq!(raster_image_sized(Arc::new(png(3, 2))).map(|(_, n)| n), Some(6), "its pixel count");
    // A header that claims more pixels than the cap: the IHDR's width, with its CRC fixed.
    let mut big = png(2, 2);
    big[16..20].copy_from_slice(&5000u32.to_be_bytes());
    big[20..24].copy_from_slice(&5000u32.to_be_bytes());
    let crc = crc32(&big[12..29]);
    big[29..33].copy_from_slice(&crc.to_be_bytes());
    assert!(raster_image(Arc::new(big)).is_none(), "past the pixel cap");
    assert!(raster_image(Arc::new(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec())).is_none(), "SVG");
    assert!(raster_image(Arc::new(vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0])).is_none(), "compressed SVG");
}

/// A 2 × 2 PNG whose header claims `w` × `h` pixels (its IHDR rewritten, with the CRC fixed).
fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    image::ImageEncoder::write_image(image::codecs::png::PngEncoder::new(&mut out), &[0; 16], 2, 2, image::ExtendedColorType::Rgba8).unwrap();
    out[16..20].copy_from_slice(&w.to_be_bytes());
    out[20..24].copy_from_slice(&h.to_be_bytes());
    let crc = crc32(&out[12..29]);
    out[29..33].copy_from_slice(&crc.to_be_bytes());
    out
}

/// A filter's images share the pixel cap: images that are each within it but pass it together
/// are refused.
#[test]
fn a_filters_images_share_the_pixel_cap() {
    let img = |w: u32, h: u32| format!("<feImage href=\"data:image/png;base64,{}\"/>", base64(&png_claiming(w, h)));
    let half = img(4096, 2048);
    let err = check(&format!("<filter id=\"a\">{half}{}</filter>", img(4096, 2049))).unwrap_err();
    assert!(err.contains("pixels in all"), "{err}");
    let err = check(&format!("<filter id=\"a\">{half}{half}{}</filter>", img(1, 1))).unwrap_err();
    assert!(err.contains("pixels in all"), "{err}");
    assert!(check(&format!("<filter id=\"a\">{half}{half}</filter>")).is_ok(), "two halves fill the cap exactly");
}

/// The image limit is for a filter's own images: the object it runs on is its source graphic
/// whatever its size.
#[test]
fn a_large_object_is_still_the_source_graphic() {
    let w = 4097;
    let mut d = blue_square(w, 0);
    filter(json!({ "xml": RED_FLOOD })).apply(&mut d, w, w, &space(w, w, 1.0, 1.0));
    assert_eq!(at(&d, w, w / 2, w / 2), [255, 0, 0, 255]);
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in bytes {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// A user-space region in percentages is written in user units, measured against the object's box.
#[test]
fn a_region_measured_against_the_box_is_written_out() {
    let far = r#"<filter id="r" filterUnits="userSpaceOnUse" x="-50%" y="0" width="200%" height="10"><feOffset/></filter>"#;
    let f = filter(json!({ "xml": far, "origin": [10.0, 0.0] }));
    assert_eq!(
        f.written_xml(Rect::new(10.0, 0.0, 30.0, 10.0)).as_deref(),
        Some(r#"<filter id="r" filterUnits="userSpaceOnUse" x="-10" y="0" width="40" height="10"><feOffset/></filter>"#)
    );
    let plain = filter(json!({ "xml": RED_FLOOD }));
    assert_eq!(plain.written_xml(Rect::new(0.0, 0.0, 1.0, 1.0)).as_deref(), Some(plain.xml()));
}

/// A filter at the size limit whose region is measured against the box still draws: written in
/// user units, the region can make the drawn form longer than the limit.
#[test]
fn a_filter_at_the_size_limit_draws_with_its_measured_region() {
    let open = r#"<filter id="r" filterUnits="userSpaceOnUse" x="-1%" y="-1%" width="1%" height="1%"><feOffset result=""#;
    let close = r#""/><feOffset/></filter>"#;
    let xml = format!("{open}{}{close}", "a".repeat(SvgFilter::MAX_XML - open.len() - close.len()));
    assert_eq!(xml.len(), SvgFilter::MAX_XML);
    assert_eq!(check(&xml), Ok("r".into()));
    let f = filter(json!({ "xml": xml }));
    let written = f.written_xml(Rect::new(0.123456789, 0.123456789, 1000.987654321, 1000.987654321)).unwrap();
    assert!(written.len() > SvgFilter::MAX_XML, "the region written out is longer: {} bytes", written.len());
    assert!(with_id(&written, DRAWN_ID).is_some(), "the drawn form is read");
}

/// Only a type selector naming a filter or one of its elements, or `*`, lets a style sheet reach
/// filters: class, id, attribute and pseudo-class names that contain "filter" or "fe" don't.
#[test]
fn style_sheets_reach_filters_only_through_their_type_selectors() {
    let reaches = |sheet: &str| {
        let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><style>{sheet}</style></svg>");
        Styles::of(&roxmltree::Document::parse(&svg).unwrap()).reach_filters
    };
    for sheet in
        [".filter-button { color: red }", "#feature, .fe-icon { fill: blue }", "[data-filter] { fill: red }", "a:hover, rect.filter { stroke: red }"]
    {
        assert!(!reaches(sheet), "{sheet}");
    }
    for sheet in
        ["filter { x: 0 }", "FeFlood { flood-color: red }", "svg|feBlend { mode: multiply }", "* { color: red }", "g :is(feGaussianBlur) { x: 0 }"]
    {
        assert!(reaches(sheet), "{sheet}");
    }
}

/// An attribute selector styles a filter whose elements carry that attribute, whatever its value
/// or namespace prefix; a filter without it is not affected.
#[test]
fn an_attribute_selector_styles_a_filter_with_that_attribute() {
    let depends = |sheet: &str, filter: &str| {
        let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><style>{sheet}</style><defs>{filter}</defs></svg>");
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let f = doc.descendants().find(|n| n.tag_name().name() == "filter").unwrap();
        depends_on_styles(f, &Styles::of(&doc))
    };
    let tagged = r#"<filter id="a"><feFlood data-tone="warm"/></filter>"#;
    let plain = r#"<filter id="a"><feFlood/></filter>"#;
    for sheet in ["[data-tone] { flood-color: red }", "[data-tone=\"warm\"] { flood-color: red }", "[svg|data-tone ~= warm] { x: 0 }"] {
        assert!(depends(sheet, tagged), "{sheet}");
        assert!(!depends(sheet, plain), "{sheet}");
    }
}

#[test]
fn filters_of_reads_at_most_256_filters() {
    let many = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
        (0..300).map(|i| format!("<filter id=\"f{i}\"><feOffset/></filter>")).collect::<String>()
    );
    let doc = roxmltree::Document::parse(&many).unwrap();
    let (found, skipped) = filters_of(&doc, &many, usize::MAX, |_| false);
    assert_eq!((found.len(), skipped), (MAX_FILTERS_READ, 300 - MAX_FILTERS_READ));
}

/// However large `scale` makes them, a morphology radius spans at most 32 pixels of the raster the
/// filter runs on, and a standard deviation at most 512.
#[test]
fn costly_lengths_are_clamped_in_pixels() {
    let sp = space(10, 10, 0.5, 0.5);
    let morph = filter(json!({ "xml": "<filter id=\"m\"><feMorphology radius=\"100\"/></filter>", "scale": 1000.0 }));
    let s = morph.resolution(&sp, 10.0);
    assert!(100.0 * 1000.0 / 0.5 * s <= 32.0 + 1e-6, "{s}");
    let blur = filter(json!({ "xml": "<filter id=\"b\"><feGaussianBlur stdDeviation=\"500\"/></filter>", "scale": 1000.0 }));
    let s = blur.resolution(&sp, 10.0);
    assert!(500.0 * 1000.0 / 0.5 * s <= 512.0 + 1e-6, "{s}");
    // At a fine raster too (1200 ppi): the cap holds.
    let fine = space(10, 10, 0.06, 0.06);
    let s = morph.resolution(&fine, 10.0);
    assert!(100.0 * 1000.0 / 0.06 * s <= 32.0 + 1e-6, "{s}");
    let small = filter(json!({ "xml": "<filter id=\"s\"><feMorphology radius=\"1\"/></filter>" }));
    assert_eq!(small.resolution(&sp, 10.0), 1.0, "a small radius runs at the raster's own pixels");
    // In the object's bounding box, a radius is a fraction of its box: half a 4096-pixel box.
    let boxed = filter(json!({ "xml": "<filter id=\"o\" primitiveUnits=\"objectBoundingBox\"><feMorphology radius=\"0.5\"/></filter>" }));
    let s = boxed.resolution(&sp, 4096.0);
    assert!(0.5 * 4096.0 * s <= 32.0 + 1e-6, "{s}");
}

/// A filter that `check` refuses is never drawn, whatever path put it in an effect.
#[test]
fn an_unchecked_filter_is_not_drawn() {
    for xml in [
        "<filter id=\"a\"><feOffset/><script>alert(1)</script></filter>",
        "<filter id=\"a\"><feMorphology radius=\"1e6\"/></filter>",
        "<filter id=\"a\" onload=\"x()\"><feOffset/></filter>",
    ] {
        let e = Effect { id: SVG_FILTER.into(), params: json!({ "xml": xml }), visible: true };
        assert!(raster_effects(&[e]).is_empty(), "{xml}");
    }
}

/// Scale Strokes & Effects scales the filter's lengths with the art: the points a unit measures.
#[test]
fn scaling_the_effect_scales_its_units() {
    let mut e = Effect { id: SVG_FILTER.into(), params: json!({ "xml": RED_FLOOD }), visible: true };
    crate::scale_effect(&mut e, 2.0);
    assert_eq!(e.params["scale"], json!(2.0));
    crate::scale_effect(&mut e, 0.25);
    assert_eq!(e.params["scale"], json!(0.5));
}

/// The form SVG output writes, for a filter that fills a 10 × 10 point box.
fn written(xml: &str) -> Option<String> {
    filter(json!({ "xml": xml })).written_xml(Rect::new(0.0, 0.0, 10.0, 10.0))
}

/// SVG 1.1 output writes an image's reference as `xlink:href`; the filter as it is drawn and
/// listed keeps `href`.
#[test]
fn an_image_is_written_with_an_xlink_reference() {
    let data = format!("data:image/png;base64,{}", base64(&png_claiming(2, 2)));
    let f = filter(json!({ "xml": format!(r#"<filter id="i"><feImage href="{data}"/></filter>"#) }));
    assert_eq!(f.xml(), format!(r#"<filter id="i"><feImage href="{data}"/></filter>"#));
    let out = f.written_xml(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    assert_eq!(out, format!(r#"<filter id="i"><feImage xlink:href="{data}"/></filter>"#));
    assert!(f.writes_as_svg11());
    assert_eq!(check(&out), Ok("i".into()), "the written form reads back");
}

/// SVG 1.1 has no `feDropShadow`: it is written as the primitives Filter Effects gives as its
/// equivalent, with its defaults, input, result and subregion, and results no other primitive uses.
#[test]
fn a_drop_shadow_is_written_as_svg_1_1_primitives() {
    assert_eq!(
        written(r#"<filter id="s"><feDropShadow/></filter>"#).as_deref(),
        Some(concat!(
            r#"<filter id="s"><feGaussianBlur in="SourceGraphic" stdDeviation="2"/><feOffset dx="2" dy="2" result="vcShadow0"/><feFlood/>"#,
            r#"<feComposite in2="vcShadow0" operator="in" result="vcShadow1"/><feMerge><feMergeNode in="vcShadow1"/><feMergeNode in="SourceGraphic"/></feMerge></filter>"#,
        ))
    );
    // Its input, result, subregion, color space and flood; a name the filter uses is not taken.
    let given = r#"<filter id="s"><feOffset dx="1" result="vcShadow0"/><feDropShadow in="vcShadow0" result="r" x="1" width="5" dx="3" dy="-1" stdDeviation="1 2" flood-color="rgba(255,0,0,0.5)" flood-opacity="50%" color-interpolation-filters="sRGB"/></filter>"#;
    assert_eq!(
        written(given).as_deref(),
        Some(concat!(
            r#"<filter id="s"><feOffset dx="1" result="vcShadow0"/>"#,
            r#"<feGaussianBlur in="vcShadow0" stdDeviation="1 2" color-interpolation-filters="sRGB"/><feOffset dx="3" dy="-1" result="vcShadow1" color-interpolation-filters="sRGB"/>"#,
            r##"<feFlood flood-color="#ff0000" flood-opacity="0.25098" color-interpolation-filters="sRGB"/>"##,
            r#"<feComposite in2="vcShadow1" operator="in" result="vcShadow2" color-interpolation-filters="sRGB"/>"#,
            r#"<feMerge x="1" width="5" result="r" color-interpolation-filters="sRGB"><feMergeNode in="vcShadow2"/><feMergeNode in="vcShadow0"/></feMerge></filter>"#,
        ))
    );
    // An implicit input, or a result no earlier primitive has, is the previous result, named.
    for shadow in [r#"<feDropShadow dx="1"/>"#, r#"<feDropShadow in="nowhere" dx="1"/>"#] {
        let out = written(&format!(r#"<filter id="s"><feFlood flood-color="blue"/>{shadow}</filter>"#)).unwrap();
        assert!(out.starts_with(r##"<filter id="s"><feFlood flood-color="#0000ff" result="vcShadow0"/><feGaussianBlur in="vcShadow0""##), "{out}");
        assert!(out.ends_with(r#"<feMergeNode in="vcShadow0"/></feMerge></filter>"#), "{out}");
        assert_eq!(check(&out), Ok("s".into()), "the written form reads back");
    }
    // What SVG 1.1 can't hold leaves the filter to be written as other pixel effects are.
    for xml in [
        r#"<filter id="s"><feDropShadow flood-color="currentColor"/></filter>"#,
        r#"<filter id="s" primitiveUnits="objectBoundingBox"><feDropShadow/></filter>"#,
    ] {
        let f = filter(json!({ "xml": xml }));
        assert!(!f.writes_as_svg11() && f.written_xml(Rect::new(0.0, 0.0, 10.0, 10.0)).is_none(), "{xml}");
    }
    // An offset that isn't a number is the shadow's default, which an `feOffset` doesn't share.
    let out = written(r#"<filter id="s"><feDropShadow dx="bad" dy="-3"/></filter>"#).unwrap();
    assert!(out.contains(r#"<feOffset dx="2" dy="-3""#), "{out}");
    // Expanded shadows past the primitive cap.
    let many = format!(r#"<filter id="s">{}</filter>"#, "<feDropShadow/>".repeat(13));
    assert!(!filter(json!({ "xml": many })).writes_as_svg11());
}

/// Each input is written as the primitive it reads when drawn: a result name only usvg gives (to
/// a primitive without a `result`, here the shadow), and the inputs usvg draws as `SourceGraphic`.
#[test]
fn inputs_are_written_as_the_primitives_they_read() {
    let xml = r#"<filter id="s"><feOffset dx="1"/><feDropShadow in="BackgroundImage"/><feOffset dx="2"/><feMerge><feMergeNode in="result2"/><feMergeNode in="FillPaint"/></feMerge><feComposite in="result1" in2="StrokePaint"/></filter>"#;
    let out = written(xml).unwrap();
    assert_eq!(
        out,
        concat!(
            r#"<filter id="s"><feOffset dx="1" result="vcShadow1"/>"#,
            r#"<feGaussianBlur in="SourceGraphic" stdDeviation="2"/><feOffset dx="2" dy="2" result="vcShadow2"/><feFlood/>"#,
            r#"<feComposite in2="vcShadow2" operator="in" result="vcShadow3"/><feMerge result="vcShadow0"><feMergeNode in="vcShadow3"/><feMergeNode in="SourceGraphic"/></feMerge>"#,
            r#"<feOffset dx="2"/><feMerge><feMergeNode in="vcShadow0"/><feMergeNode in="SourceGraphic"/></feMerge>"#,
            r#"<feComposite in="vcShadow1" in2="SourceGraphic"/></filter>"#,
        )
    );
    assert_eq!(check(&out), Ok("s".into()), "the written form reads back");
}

/// Other values SVG 1.1 lacks: SVG 1.1 values that draw alike, or no SVG 1.1 form.
#[test]
fn values_svg_1_1_lacks_are_written_as_resvg_draws_them() {
    // `lighter` (Filter Effects) is drawn as the default `over`, and a blur has no `edgeMode`.
    assert_eq!(
        written(r#"<filter id="c"><feComposite in2="SourceAlpha" operator="lighter"/><feGaussianBlur stdDeviation="1" edgeMode="wrap"/></filter>"#)
            .as_deref(),
        Some(r#"<filter id="c"><feComposite in2="SourceAlpha"/><feGaussianBlur stdDeviation="1"/></filter>"#)
    );
    assert_eq!(
        written(r#"<filter id="c"><feBlend in2="SourceAlpha" mode="multiply"/></filter>"#).as_deref(),
        Some(r#"<filter id="c"><feBlend in2="SourceAlpha" mode="multiply"/></filter>"#)
    );
    // CSS colors as `#rrggbb`, a flood color's alpha in its opacity, a lighting color's dropped.
    assert_eq!(
        written(r#"<filter id="c" color="hsl(120, 100%, 50%)"><feFlood flood-color="transparent"/><feDiffuseLighting lighting-color="rgba(0,0,255,0.5)"><feDistantLight/></feDiffuseLighting></filter>"#)
            .as_deref(),
        Some(
            r##"<filter id="c" color="#00ff00"><feFlood flood-color="#000000" flood-opacity="0"/><feDiffuseLighting lighting-color="#0000ff"><feDistantLight/></feDiffuseLighting></filter>"##
        )
    );
    // Image scaling as an SVG 1.1 keyword.
    let data = format!("data:image/png;base64,{}", base64(&png_claiming(2, 2)));
    for (given, kept) in
        [("pixelated", "optimizeSpeed"), ("crisp-edges", "optimizeSpeed"), ("smooth", "optimizeQuality"), ("auto", "optimizeQuality")]
    {
        let out = written(&format!(r#"<filter id="i"><feImage style="image-rendering:{given}" href="{data}"/></filter>"#)).unwrap();
        assert!(out.contains(&format!(r#"style="image-rendering:{kept}""#)), "{given}: {out}");
    }
    // The blend modes SVG 1.1 lacks have no SVG 1.1 form.
    for mode in LATER_BLEND_MODES {
        let f = filter(json!({ "xml": format!(r#"<filter id="b"><feBlend in2="SourceAlpha" mode="{mode}"/></filter>"#) }));
        assert!(!f.writes_as_svg11(), "{mode}");
    }
}

#[test]
fn base64_pads_as_rfc_4648_does() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
}
