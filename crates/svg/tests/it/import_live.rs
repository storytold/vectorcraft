//! SVG import keeps what the canvas edits live: `<pattern>` becomes a pattern swatch, `<symbol>`
//! with `<use>` a symbol and its instances, blurs and drop-shadow chains live effects; and
//! export → import keeps them.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeKind};
use vectorcraft_effects::RasterFx;
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export, import, import_with_report};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="120">{body}</svg>"#)
}

fn open(body: &str) -> (Document, Vec<String>) {
    import_with_report(&svg(body)).unwrap()
}

/// All non-layer nodes in paint order.
fn art(d: &Document) -> Vec<&Node> {
    let mut v = Vec::new();
    d.walk(|n| {
        if !n.is_layer() {
            v.push(n)
        }
    });
    v
}

/// The SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

/// The effects of the only object.
fn effects(d: &Document) -> Vec<Effect> {
    let a = art(d);
    assert_eq!(a.len(), 1, "{a:?}");
    a[0].appearance.effects.clone()
}

#[test]
fn a_checker_pattern_becomes_a_pattern_swatch() {
    let (d, w) = open(
        r##"<defs><pattern id="Checker" width="20" height="20" patternUnits="userSpaceOnUse">
          <rect width="10" height="10" fill="#000"/><rect x="10" y="10" width="10" height="10" fill="#000"/>
        </pattern></defs>
        <rect width="200" height="120" fill="url(#Checker)"/>"##,
    );
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(d.patterns.len(), 1);
    let p = &d.patterns[0];
    assert_eq!((p.name.as_str(), p.tile), ("Checker", Rect::new(0.0, 0.0, 20.0, 20.0)));
    assert!(matches!(art(&d)[0].appearance.fill_paint(), Paint::Pattern { pattern, .. } if pattern == "Checker"));
    assert_similar(
        &render_artboard(&d),
        &resvg_render(
            &svg(
                r##"<defs><pattern id="Checker" width="20" height="20" patternUnits="userSpaceOnUse"><rect width="10" height="10" fill="#000"/><rect x="10" y="10" width="10" height="10" fill="#000"/></pattern></defs><rect width="200" height="120" fill="url(#Checker)"/>"##,
            ),
            200,
            120,
        ),
        24.0,
        0.01,
    );
}

const SYMBOL: &str = r##"<defs><symbol id="Star" data-name="Gold Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/><circle cx="10" cy="10" r="4" fill="#fff"/></symbol></defs>"##;

#[test]
fn two_uses_give_one_symbol_and_two_instances() {
    let body = format!(r##"{SYMBOL}<use href="#Star" x="20" y="30"/><use xlink:href="#Star" transform="translate(100 50) rotate(30)"/>"##);
    let (d, w) = open(&body);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(d.symbols.len(), 1);
    assert_eq!(d.symbols[0].name, "Gold Star");
    assert!(matches!(&d.symbols[0].art.kind, NodeKind::Group { children, .. } if children.len() == 2));
    let instances: Vec<_> = art(&d)
        .into_iter()
        .filter_map(|n| match &n.kind {
            NodeKind::SymbolInstance { symbol, xf } => Some((symbol.clone(), *xf)),
            _ => None,
        })
        .collect();
    assert_eq!(instances.len(), 2, "{:?}", art(&d));
    assert!(instances.iter().all(|(s, _)| s == "Gold Star"));
    assert!((instances[0].1 * Point::ORIGIN).distance(Point::new(20.0, 30.0)) < 1e-9);
    assert!((instances[1].1 * Point::ORIGIN).distance(Point::new(100.0, 50.0)) < 1e-9);
    assert_similar(&render_artboard(&d), &resvg_render(&svg(&body), 200, 120), 24.0, 0.003);
}

#[test]
fn an_instance_is_measured_by_its_symbols_art() {
    let (d, _) = open(&format!(r##"{SYMBOL}<use href="#Star" x="20" y="30"/>"##));
    let id = art(&d)[0].id;
    // The art, (0, 0) to (20, 20), drawn from (20, 30): not the 20 × 20 square around the origin,
    // which the same transform places from (10, 20) to (30, 40).
    assert_eq!(d.bounds_of(&[id], false), Some(Rect::new(20.0, 30.0, 40.0, 50.0)));
    assert_eq!(d.bounds_of(&[id], true), Some(Rect::new(20.0, 30.0, 40.0, 50.0)));
}

#[test]
fn uses_the_symbol_cant_stand_for_stay_art() {
    // Inherited paint: the second use shows the symbol in another colour. Scaled: the canvas keeps
    // stroke weights as they are.
    let body = r##"<defs><symbol id="Dot"><circle cx="5" cy="5" r="4" stroke="#000" stroke-width="2"/></symbol></defs>
        <use href="#Dot" fill="red"/><use href="#Dot" x="40" fill="blue"/><use href="#Dot" x="80" fill="red"/>
        <use href="#Dot" fill="red" transform="translate(120 0) scale(3)"/>"##;
    let (d, _) = open(body);
    assert_eq!(d.symbols.len(), 1);
    let a = art(&d);
    let instances = a.iter().filter(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })).count();
    assert_eq!(instances, 2, "the red ones not scaled: {a:?}");
    assert_similar(&render_artboard(&d), &resvg_render(&svg(body), 200, 120), 24.0, 0.003);
}

#[test]
fn symbols_round_trip_through_export() {
    let (d, _) = open(&format!(r##"{SYMBOL}<use href="#Star" x="20" y="30"/><use href="#Star" x="120" y="60"/>"##));
    let back = import(&export(&d, &ExportOptions::default())).unwrap();
    assert_eq!(back.symbols.len(), 1);
    assert_eq!(back.symbols[0].name, "Gold Star");
    assert_eq!(art(&back).iter().filter(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })).count(), 2);
    assert_similar(&render_artboard(&d), &render_artboard(&back), 8.0, 0.001);
}

/// `name="…"` of the first element `tag` in `svg`, as a number.
fn number_attr(svg: &str, tag: &str, name: &str) -> f64 {
    let at = svg.find(&format!("<{tag} ")).unwrap_or_else(|| panic!("no <{tag}> in {svg}"));
    let rest = &svg[at..];
    let from = rest.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
    rest[from..from + rest[from..].find('"').unwrap()].parse().unwrap()
}

#[test]
fn an_instances_shadow_filter_and_the_art_export_cover_its_symbols_art() {
    let (mut d, _) = open(&format!(r##"{SYMBOL}<use href="#Star" x="20" y="30"/>"##));
    let id = art(&d)[0].id;
    let shadow = Effect { id: "stylize.dropShadow".into(), params: json!({"x": 0, "y": 0, "blur": 0}), visible: true };
    d.node_mut(id).unwrap().appearance.effects.push(shadow);
    // The filter region holds the art, (20, 30) to (40, 50).
    let svg = export(&d, &ExportOptions::default());
    let (x, y) = (number_attr(&svg, "filter", "x"), number_attr(&svg, "filter", "y"));
    let (w, h) = (number_attr(&svg, "filter", "width"), number_attr(&svg, "filter", "height"));
    assert!(x <= 20.0 && y <= 30.0 && x + w >= 40.0 && y + h >= 50.0, "({x}, {y}) {w} × {h}");
    // Exported without an artboard, the file is the art's size and the instance draws the art from
    // the file's top left corner.
    d.node_mut(id).unwrap().appearance.effects.clear();
    let svg = export(&d, &ExportOptions { artboard: None, ..ExportOptions::default() });
    assert_eq!((number_attr(&svg, "svg", "width"), number_attr(&svg, "svg", "height")), (20.0, 20.0), "{svg}");
    let back = import(&svg).unwrap();
    let NodeKind::SymbolInstance { xf, .. } = &art(&back)[0].kind else { panic!("an instance: {svg}") };
    assert!((*xf * Point::ORIGIN).distance(Point::ORIGIN) < 1e-9, "{svg}");
}

#[test]
fn a_figma_blurred_shape_filter_becomes_gaussian_blur() {
    let (d, w) = open(
        r##"<defs><filter id="b"><feFlood flood-opacity="0" result="BackgroundImageFix"/><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"/><feGaussianBlur result="effect1_foregroundBlur" stdDeviation="3"/></filter></defs><rect x="50" y="30" width="60" height="40" fill="#08f" filter="url(#b)"/>"##,
    );
    assert!(w.is_empty(), "{w:?}");
    let fx = effects(&d);
    assert_eq!(fx.len(), 1);
    assert_eq!((fx[0].id.as_str(), &fx[0].params), ("blur.gaussian", &json!({"radius": 6.0})));
}

#[test]
fn figma_blurred_shapes_inside_mask_import_with_effects_and_no_warnings() {
    let body = r##"<mask id="m"><circle cx="50" cy="50" r="40" fill="#fff"/></mask>
        <g mask="url(#m)">
            <g filter="url(#b1)"><circle cx="30" cy="30" r="20" fill="#ff0"/></g>
            <g filter="url(#b2)"><circle cx="70" cy="70" r="20" fill="#00f"/></g>
        </g>
        <defs>
            <filter id="b1"><feFlood flood-opacity="0" result="BackgroundImageFix"/><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"/><feGaussianBlur result="effect1_foregroundBlur" stdDeviation="2"/></filter>
            <filter id="b2"><feFlood flood-opacity="0" result="BackgroundImageFix"/><feBlend in="SourceGraphic" in2="BackgroundImageFix" result="shape"/><feGaussianBlur result="effect1_foregroundBlur" stdDeviation="4"/></filter>
        </defs>"##;
    let (d, w) = open(body);
    assert!(w.is_empty(), "{w:?}");
    let mut blurs = vec![];
    d.walk(|n| {
        for e in &n.appearance.effects {
            if e.id == "blur.gaussian" {
                blurs.push(e.params["radius"].as_f64().unwrap());
            }
        }
    });
    assert_eq!(blurs, vec![4.0, 8.0]);
}

#[test]
fn a_gaussian_blur_becomes_the_effect_with_its_radius() {
    let (d, w) = open(
        r##"<defs><filter id="b"><feGaussianBlur stdDeviation="3"/></filter></defs><rect x="50" y="30" width="60" height="40" fill="#08f" filter="url(#b)"/>"##,
    );
    assert!(w.is_empty(), "{w:?}");
    let fx = effects(&d);
    assert_eq!(fx.len(), 1);
    assert_eq!((fx[0].id.as_str(), &fx[0].params), ("blur.gaussian", &json!({"radius": 6.0})));
    // In a scaled group, the radius scales too.
    let (d, _) = open(
        r##"<defs><filter id="b"><feGaussianBlur stdDeviation="3"/></filter></defs><g transform="scale(2)"><rect width="30" height="20" filter="url(#b)"/></g>"##,
    );
    assert_eq!(effects(&d)[0].params["radius"], json!(12.0));
}

/// The one drop shadow of the only object: (dx, dy, blur, colour, opacity %).
fn shadow(d: &Document) -> (f64, f64, f64, Color, f32) {
    let fx = vectorcraft_effects::raster_effects(&effects(d));
    match fx.as_slice() {
        [RasterFx::DropShadow { dx, dy, blur, color, opacity, mode, darkness }] => {
            assert_eq!(*mode, BlendMode::Normal, "filters composite normally");
            assert_eq!(*darkness, None);
            (*dx, *dy, *blur, *color, (opacity * 100.0).round())
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn drop_shadow_chains_become_drop_shadows() {
    let rect = r##"<rect x="50" y="30" width="60" height="40" fill="#08f" filter="url(#s)"/>"##;
    let chains = [
        // feDropShadow.
        r##"<feDropShadow dx="4" dy="5" stdDeviation="2" flood-color="#ff0000" flood-opacity="0.5"/>"##,
        // Offset, blur, flood, composite, merge.
        r##"<feOffset in="SourceAlpha" dx="4" dy="5"/><feGaussianBlur stdDeviation="2" result="b"/><feFlood flood-color="#ff0000" flood-opacity="0.5"/><feComposite in2="b" operator="in"/><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge>"##,
        // Flood in the source, blur, offset, the source over it.
        r##"<feFlood flood-color="#ff0000" flood-opacity="0.5" result="f"/><feComposite in="f" in2="SourceGraphic" operator="in"/><feGaussianBlur stdDeviation="2"/><feOffset dx="4" dy="5" result="o"/><feComposite in="SourceGraphic" in2="o" operator="over"/>"##,
        // A colour matrix paints the blurred, moved silhouette.
        r##"<feGaussianBlur in="SourceAlpha" stdDeviation="2"/><feOffset dx="4" dy="5"/><feColorMatrix values="0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 0.5 0" result="c"/><feMerge><feMergeNode in="c"/><feMergeNode in="SourceGraphic"/></feMerge>"##,
    ];
    for chain in chains {
        let (d, w) = open(&format!(r#"<defs><filter id="s">{chain}</filter></defs>{rect}"#));
        assert!(w.is_empty(), "{chain}: {w:?}");
        let (dx, dy, blur, color, opacity) = shadow(&d);
        assert_eq!((dx, dy, blur, opacity), (4.0, 5.0, 4.0, 50.0), "{chain}");
        assert_eq!(color.to_hex(), "#ff0000", "{chain}");
    }
}

/// A proportional RGB filter is not additive Darkness. A fully black shadow remains
/// recognizable, while other proportional transforms are reported rather than misrepresented.
#[test]
fn proportional_filters_are_not_misread_as_additive_darkness() {
    let chain = |space: &str, transfer: &str, slope: f64| {
        format!(
            r##"<defs><filter id="s"{space}><feGaussianBlur in="SourceGraphic" stdDeviation="3" color-interpolation-filters="linearRGB" result="unused"/><feGaussianBlur in="SourceGraphic" stdDeviation="2"/><feOffset dx="4" dy="5"/><feComponentTransfer result="p"{transfer}><feFuncR type="linear" slope="{slope}"/><feFuncG type="linear" slope="{slope}"/><feFuncB type="linear" slope="{slope}"/><feFuncA type="linear" slope="0.6"/></feComponentTransfer><feMerge><feMergeNode in="p"/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs><rect x="50" y="30" width="60" height="40" fill="#f00" filter="url(#s)"/>"##
        )
    };
    let srgb = r#" color-interpolation-filters="sRGB""#;
    // The scale 0 makes black whatever values it works on.
    for (space, slope, darkness) in [(srgb, 0.0, 100.0), ("", 0.0, 100.0)] {
        let (d, w) = open(&chain(space, "", slope));
        assert!(w.is_empty(), "{w:?}");
        let fx = effects(&d);
        let p = &fx[0].params;
        assert_eq!(
            (fx[0].id.as_str(), &p["shade"], &p["darkness"], &p["opacity"]),
            ("stylize.dropShadow", &json!("darkness"), &json!(darkness), &json!(60.0))
        );
        assert_eq!((&p["x"], &p["y"], &p["blur"]), (&json!(4.0), &json!(5.0), &json!(4.0)));
    }
    // Scaled on linear values, or blurred on them before an sRGB scale.
    for (space, transfer) in [(srgb, ""), ("", ""), ("", srgb)] {
        let (d, w) = open(&chain(space, transfer, 0.5));
        assert!(w.iter().any(|w| w.contains("filter")), "{transfer}: {w:?}");
        assert!(effects(&d).is_empty());
    }
}

#[test]
fn other_filters_are_reported() {
    let (d, w) =
        open(r##"<defs><filter id="t"><feTurbulence baseFrequency="0.05"/></filter></defs><rect width="60" height="40" filter="url(#t)"/>"##);
    assert!(w.iter().any(|w| w.contains("filter")), "{w:?}");
    assert!(effects(&d).is_empty());
}

#[test]
fn a_filter_and_a_clip_on_one_object_filter_first() {
    let body = r##"<defs><filter id="b"><feGaussianBlur stdDeviation="4"/></filter><clipPath id="c"><rect x="60" y="40" width="40" height="40"/></clipPath></defs>
        <rect x="50" y="30" width="60" height="60" fill="#08f" filter="url(#b)" clip-path="url(#c)"/>"##;
    let (d, w) = open(body);
    assert!(w.is_empty(), "{w:?}");
    let a = art(&d);
    assert!(matches!(a[0].kind, NodeKind::Group { clip: true, .. }) && a[0].appearance.effects.is_empty());
    assert!(a.iter().any(|n| n.appearance.effects.iter().any(|e| e.id == "blur.gaussian")));
    assert_similar(&render_artboard(&d), &resvg_render(&svg(body), 200, 120), 40.0, 0.02);
}

#[test]
fn exported_effects_import_as_the_same_effects() {
    let fx = |id: &str, p: serde_json::Value| Effect { id: id.into(), params: p, visible: true };
    let cases = [
        vec![fx("stylize.dropShadow", json!({"x": 7.0, "y": -3.0, "blur": 5.0, "opacity": 60.0, "color": "#203040"}))],
        vec![fx("stylize.dropShadow", json!({"x": 7.0, "y": -3.0, "blur": 5.0, "opacity": 60.0, "shade": "darkness", "darkness": 40.0}))],
        vec![fx("stylize.outerGlow", json!({"blur": 8.0, "color": "#ffcc00", "opacity": 80.0}))],
        vec![fx("stylize.innerGlow", json!({"blur": 6.0, "color": "#ffffff", "source": "edge", "mode": "normal"}))],
        vec![fx("stylize.innerGlow", json!({"blur": 6.0, "color": "#00ff00", "source": "center", "mode": "multiply"}))],
        vec![fx("stylize.feather", json!({"radius": 4.0}))],
        // Stacked: the first innermost.
        vec![fx("blur.gaussian", json!({"radius": 3.0})), fx("stylize.dropShadow", json!({"x": 2.0, "y": 2.0, "blur": 1.0}))],
    ];
    for effects in cases {
        let mut d = Document::new(200.0, 120.0);
        let mut n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(50.0, 30.0, 110.0, 90.0)), Appearance::default_art());
        n.appearance.effects = effects.clone();
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        let (back, w) = import_with_report(&export(&d, &ExportOptions::default())).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let got = vectorcraft_effects::raster_effects(&art(&back)[0].appearance.effects);
        assert_eq!(got, vectorcraft_effects::raster_effects(&effects), "{effects:?}");
    }
}

#[test]
fn a_drop_shadow_with_darkness_exports_as_the_canvas_draws_it() {
    let mut d = Document::new(200.0, 120.0);
    let mut n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(40.0, 20.0, 100.0, 70.0)),
        Appearance::basic(Paint::solid(Color::rgb(0.8, 0.4, 0.2)), Paint::None, 0.0),
    );
    let shadow = json!({"x": 30.0, "y": 30.0, "blur": 0.0, "opacity": 100.0, "mode": "normal", "shade": "darkness", "darkness": 25.0});
    n.appearance.effects = vec![Effect { id: "stylize.dropShadow".into(), params: shadow, visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let svg = export(&d, &ExportOptions::default());
    let (canvas, exported) = (render_artboard(&d), resvg_render(&svg, 200, 120));
    // An intermediate color distinguishes subtraction from proportional darkening.
    for img in [&canvas, &exported] {
        let [r, g, b] = img.over_white(115, 90);
        assert!(r.abs_diff(140) <= 2 && g.abs_diff(38) <= 2 && b <= 2, "{:?}", [r, g, b]);
    }
    assert_similar(&canvas, &exported, 40.0, 0.02);
}

#[test]
fn an_exported_outer_glow_round_faint_art_draws_as_the_canvas_does() {
    // GUARDS: the export's glow blurs a solid silhouette like the renderer, not the art's faint alpha.
    let mut d = Document::new(200.0, 120.0);
    let mut n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(70.0, 40.0, 130.0, 80.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    n.appearance.fill_mut().unwrap().opacity = 0.2;
    n.appearance.effects = vec![Effect {
        id: "stylize.outerGlow".into(),
        params: json!({"blur": 12.0, "color": "#ff0000", "opacity": 100.0, "mode": "normal"}),
        visible: true,
    }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let svg = export(&d, &ExportOptions::default());
    assert_similar(&render_artboard(&d), &resvg_render(&svg, 200, 120), 40.0, 0.02);
}

#[test]
fn symbols_and_effects_export_import_export_is_stable() {
    // A doc with a symbol instance carrying a shadow: export → import → export is stable.
    let mut d = Document::new(200.0, 120.0);
    let dot = Node::path(
        d.alloc_id(),
        shapes::ellipse(Rect::new(0.0, 0.0, 20.0, 20.0)),
        Appearance::basic(Paint::solid(Color::rgb8(200, 30, 30)), Paint::None, 0.0),
    );
    d.symbols.push(vectorcraft_doc::Symbol { name: "Dot".into(), art: Arc::new(dot) });
    let l = d.layers[0].id;
    for x in [10.0, 60.0] {
        let id = d.alloc_id();
        d.insert(
            Some(l),
            usize::MAX,
            Node::new(id, NodeKind::SymbolInstance { symbol: "Dot".into(), xf: vectorcraft_geom::Affine::translate((x, 40.0)) }),
        )
        .unwrap();
    }
    let mut blurred = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(120.0, 30.0, 180.0, 90.0)), Appearance::default_art());
    blurred.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: json!({"x": 3.0, "y": 3.0, "blur": 2.0}), visible: true });
    d.insert(Some(l), usize::MAX, blurred).unwrap();
    let opts = ExportOptions { object_ids: vectorcraft_svg::ObjectIds::Minimal, ..Default::default() };
    let first = export(&d, &opts);
    let second = export(&import(&first).unwrap(), &opts);
    let strip = |s: &str| s.lines().filter(|l| !l.contains("<title>")).collect::<Vec<_>>().join("\n");
    assert_eq!(strip(&first), strip(&second));
}
