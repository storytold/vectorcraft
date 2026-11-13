//! Joined-text stroke regression probes: original Beh text, actual CPU pixels, no external fixture assets.
//! W3C Arabic & Persian Layout Requirements 4.3.7.3 describes borders of joined letters:
//! https://www.w3.org/TR/alreq/ (Group Draft Note, not an Adobe execution oracle).
//! The four cases use optional Craft Fonts Arabic input and explicitly skip when it is absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, Document, FillLayer, LineJoin, Node, NodeKind, StrokeLayer, TextObject};
use vectorcraft_geom::{Affine, FillRule, PathData, Point};
use vectorcraft_pathops::try_unite_all_with;
use vectorcraft_render::{RenderOptions, Rendered, Renderer};
use vectorcraft_text::{FontDb, FontFace, PositionedGlyph, TextLayout, layout};

const WIDTH: u32 = 900;
const HEIGHT: u32 = 360;
const WEIGHT: f64 = 3.0;
const JOINED: &str = "\u{0628}\u{0628}\u{0628}";
const SEPARATED: &str = "\u{0628}\u{200c}\u{0628}\u{200c}\u{0628}";

#[derive(Clone, Copy, Debug)]
enum StrokeRoute {
    None,
    Characters,
    ObjectAboveCharacters,
}

/// Use the optional authored Craft face, never an installed same-named substitute.
/// A supplied but invalid face must fail the assertions rather than become a skip.
fn craft_arabic_face(test: &str) -> Option<Arc<FontFace>> {
    let Some(font) = vectorcraft_text::CRAFT_FONTS.iter().find(|font| font.family == "Noto Sans Arabic" && font.style == "Regular") else {
        eprintln!("SKIPPED: {test}; optional Craft Noto Sans Arabic Regular is absent (set CRAFT_FONTS_DIR)");
        return None;
    };
    assert!(font.is_arabic(), "the supplied Craft face must be marked for Arabic");
    let face = FontDb::global().face(font.family, font.style).expect("the supplied Craft Arabic face must be registered");
    assert!(face.file_data() == font.bytes, "the selected face must use the optional Craft font bytes");
    assert!(face.covers('\u{0628}'), "the supplied Craft face must cover the authored Beh text");
    eprintln!(
        "ASSERTIONS RUN: {test}; Craft Arabic {} {} bytes={} crc32={:08x}",
        face.family,
        face.style,
        font.bytes.len(),
        crc32fast::hash(font.bytes)
    );
    Some(face)
}

fn black() -> Paint {
    Paint::solid(Color::BLACK)
}

fn red() -> Paint {
    Paint::solid(Color::rgb8(230, 0, 0))
}

fn border() -> StrokeLayer {
    let mut stroke = StrokeLayer::new(red(), WEIGHT);
    stroke.join = LineJoin::Round;
    stroke
}

fn render_node(mut node: Node) -> Rendered {
    let mut doc = Document::new(f64::from(WIDTH), f64::from(HEIGHT));
    node.id = doc.alloc_id();
    doc.insert(Some(doc.layers[0].id), 0, node).unwrap();
    let mut renderer = Renderer::new();
    renderer.threads = 0;
    renderer.render(
        &doc,
        WIDTH,
        HEIGHT,
        Affine::IDENTITY,
        &RenderOptions { background: Some([255, 255, 255, 255]), artboards: false, ..Default::default() },
    )
}

fn render_text(text: &TextObject, route: StrokeRoute) -> Rendered {
    let mut text = text.clone();
    let mut appearance = Appearance::default();
    match route {
        StrokeRoute::None => {}
        StrokeRoute::Characters => {
            // The normal whole-type paint/stroke commands edit these character attributes.
            text.runs[0].style.stroke = red();
            text.runs[0].style.stroke_width = WEIGHT;
            text.runs[0].style.stroke_join = LineJoin::Round;
        }
        StrokeRoute::ObjectAboveCharacters => {
            // Explicit object Appearance row, distinct from the default character-stroke route.
            appearance.items.push(AppearanceItem::Stroke(border()));
            appearance.set_contents_at(0);
        }
    }
    assert_eq!(
        geometry(&layout(FontDb::global(), &text)),
        geometry(&layout(FontDb::global(), &plain(text.clone()))),
        "paint must not move or reshape glyphs"
    );
    let mut node = Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text)));
    node.appearance = appearance.into();
    render_node(node)
}

fn plain(mut text: TextObject) -> TextObject {
    text.runs[0].style.stroke = Paint::None;
    text.runs[0].style.stroke_width = 0.0;
    text
}

type Geometry = (u32, u32, usize, usize, [u64; 2], [u64; 6], u64);

fn geometry(text: &TextLayout) -> Vec<Geometry> {
    text.glyphs
        .iter()
        .map(|g| {
            (g.font_id, g.gid, g.byte, g.len, [g.origin.x.to_bits(), g.origin.y.to_bits()], g.xf.as_coeffs().map(f64::to_bits), g.advance.to_bits())
        })
        .collect()
}

/// Authored letters own clusters, not a fixed number of shaped glyphs. Noto may
/// decompose each Beh into an advancing body and a zero-advance letter dot.
fn beh_clusters(source: &str, face: &FontFace, laid_out: &TextLayout) -> Vec<Vec<u32>> {
    assert!(source.chars().all(|ch| matches!(ch, '\u{0628}' | '\u{200c}')));
    let letters: Vec<_> = source.char_indices().filter_map(|(byte, ch)| (ch == '\u{0628}').then_some(byte)).collect();
    assert_eq!(letters.len(), 3, "HOLD: the authored input must contain exactly three Beh letters");
    assert!(laid_out.ruby.is_empty() && laid_out.inlines.is_empty() && laid_out.markers.is_empty());
    let mut clusters: Vec<Vec<&PositionedGlyph>> = vec![Vec::new(); letters.len()];
    for glyph in &laid_out.glyphs {
        let end = glyph.byte.checked_add(glyph.len).expect("HOLD: source cluster overflow");
        let span = source.get(glyph.byte..end).expect("HOLD: glyph cluster is outside a UTF-8 source span");
        assert!(glyph.font_id == face.id() && glyph.gid != 0 && glyph.rtl, "HOLD: wrong face, missing glyph or absent RTL shaping");
        assert!(glyph.advance.is_finite() && glyph.advance >= 0.0);
        assert!(glyph.origin.x.is_finite() && glyph.origin.y.is_finite() && glyph.xf.as_coeffs().into_iter().all(f64::is_finite));
        if span == "\u{200c}" {
            assert!(glyph.outline.elements().is_empty() && glyph.advance == 0.0, "HOLD: ZWNJ must not add ink or advance");
        } else {
            assert_eq!(span, "\u{0628}", "HOLD: an ink glyph must belong to one authored Beh cluster");
            let index = letters.iter().position(|byte| *byte == glyph.byte).expect("HOLD: unowned letter cluster");
            assert!(!glyph.outline.elements().is_empty(), "HOLD: every shaped Beh component must contain real ink");
            clusters[index].push(glyph);
        }
    }
    clusters
        .into_iter()
        .map(|glyphs| {
            assert!(!glyphs.is_empty(), "HOLD: a letter cluster was dropped");
            assert_eq!(
                glyphs.iter().filter(|glyph| glyph.advance > 0.0).count(),
                1,
                "HOLD: a Beh cluster needs one advancing body, without duplication"
            );
            let mut ids: Vec<_> = glyphs.iter().map(|glyph| glyph.gid).collect();
            ids.sort_unstable();
            ids
        })
        .collect()
}

fn assert_joining(joined: &[Vec<u32>], separated: &[Vec<u32>]) {
    assert_eq!(joined.len(), 3);
    assert_eq!(separated.len(), 3);
    assert!(
        joined.iter().zip(separated).filter(|(a, b)| a != b).count() >= 2,
        "HOLD: no contextual joining-form substitution against the same-font ZWNJ control"
    );
}

fn assert_joined_probe(face: &FontFace, text: &TextObject, laid_out: &TextLayout) {
    let joined = beh_clusters(JOINED, face, laid_out);
    let control = TextObject::point(Point::ZERO, SEPARATED, text.runs[0].style.clone());
    let separated = beh_clusters(SEPARATED, face, &layout(FontDb::global(), &control));
    assert_joining(&joined, &separated);
}

/// Layout bounds include the face's metrics; place the actual drawn ink instead.
fn place_ink(text: &mut TextObject, laid_out: &TextLayout) {
    let bounds = laid_out
        .glyphs
        .iter()
        .filter(|glyph| !glyph.outline.elements().is_empty())
        .filter_map(|glyph| PathData::from_bezpath(&glyph.outline).bounds())
        .reduce(|a, b| a.union(b))
        .expect("HOLD: no actual glyph ink bounds");
    text.xf = Affine::translate((40.0 - bounds.x0, 40.0 - bounds.y0));
}

fn silhouette(text: &TextObject, laid_out: &TextLayout, face: &FontFace) -> PathData {
    let source: String = text.runs.iter().map(|run| run.text.as_str()).collect();
    beh_clusters(&source, face, laid_out);

    // Public laid-out outlines are already in text space. Union their FILLED regions: merely
    // appending their contours would retain the internal borders this fixture is meant to catch.
    let paths: Vec<_> = laid_out
        .glyphs
        .iter()
        .filter(|g| !g.outline.elements().is_empty())
        .map(|g| {
            let mut outline = g.outline.clone();
            outline.apply_affine(text.xf);
            PathData::from_bezpath(&outline)
        })
        .collect();
    assert!(!paths.is_empty(), "HOLD: the three validated source clusters must contain outline ink");
    let inputs: Vec<_> = paths.iter().map(|p| (p, FillRule::NonZero)).collect();
    let united = try_unite_all_with(&inputs, 0.001).expect("HOLD: independent filled-silhouette union failed");
    let b = united.bounds().expect("HOLD: no filled glyph silhouette");
    assert!(
        b.x0 > WEIGHT && b.y0 > WEIGHT && b.x1 < f64::from(WIDTH) - WEIGHT && b.y1 < f64::from(HEIGHT) - WEIGHT,
        "HOLD: fixture would clip: {b:?}"
    );
    united
}

fn render_silhouette(path: &PathData, stroked: bool) -> Rendered {
    let mut appearance = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(black()))], ..Default::default() };
    if stroked {
        appearance.items.push(AppearanceItem::Stroke(border()));
    }
    render_node(Node::path(vectorcraft_doc::NodeId(0), path.clone(), appearance))
}

fn is_black([r, g, b, a]: [u8; 4]) -> bool {
    a == 255 && r < 30 && g < 30 && b < 30
}

fn is_red([r, g, b, a]: [u8; 4]) -> bool {
    a == 255 && r > 160 && g < 70 && b < 70
}

fn save_evidence(name: &str, image: &Rendered) {
    let path = std::env::temp_dir().join(format!("6706-{name}.png"));
    std::fs::write(&path, image.to_png().expect("HOLD: PNG encoding failed")).expect("HOLD: PNG evidence write failed");
    eprintln!("#6706 CPU image: {}", path.display());
}

fn black_mask(image: &Rendered) -> Vec<bool> {
    (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| is_black(image.pixel(x, y)))).collect()
}

fn components(mut mask: Vec<bool>) -> usize {
    let mut count = 0;
    for start in 0..mask.len() {
        if !mask[start] {
            continue;
        }
        count += 1;
        let mut pending = vec![start];
        mask[start] = false;
        while let Some(i) = pending.pop() {
            let (x, y) = (i % WIDTH as usize, i / WIDTH as usize);
            for next in [
                (x > 0).then(|| i - 1),
                (x + 1 < WIDTH as usize).then_some(i + 1),
                (y > 0).then(|| i - WIDTH as usize),
                (y + 1 < HEIGHT as usize).then_some(i + WIDTH as usize),
            ]
            .into_iter()
            .flatten()
            {
                if mask[next] {
                    mask[next] = false;
                    pending.push(next);
                }
            }
        }
    }
    count
}

/// Pixels a full pixel inside the reference black ink, away from antialiasing/refit edges.
fn interior_counts(actual: &Rendered, reference: &Rendered) -> (usize, usize, usize) {
    let (mut eligible, mut red_inside, mut missing_black) = (0, 0, 0);
    for y in 1..HEIGHT - 1 {
        for x in 1..WIDTH - 1 {
            if (y - 1..=y + 1).all(|v| (x - 1..=x + 1).all(|u| is_black(reference.pixel(u, v)))) {
                eligible += 1;
                red_inside += usize::from(is_red(actual.pixel(x, y)));
                missing_black += usize::from(!is_black(actual.pixel(x, y)));
            }
        }
    }
    (eligible, red_inside, missing_black)
}

#[test]
fn arabic_joining_strokes_do_not_paint_internal_borders() {
    let db = FontDb::global();
    let Some(face) = craft_arabic_face("arabic_joining_strokes_do_not_paint_internal_borders") else { return };
    let mut observed = Vec::new();
    let mut component_counts = Vec::new();
    let mut cluster_shapes = Vec::new();
    for (name, source) in [("joined Beh", JOINED), ("ZWNJ-separated Beh", SEPARATED)] {
        let style = CharStyle {
            font_family: face.family.clone(),
            font_style: face.style.clone(),
            size: 192.0,
            fill: black(),
            stroke: Paint::None,
            stroke_width: 0.0,
            stroke_join: LineJoin::Round,
            ..Default::default()
        };
        let mut text = TextObject::point(Point::ZERO, source, style);
        let laid_out = layout(db, &text);
        cluster_shapes.push(beh_clusters(source, &face, &laid_out));
        place_ink(&mut text, &laid_out);
        let united = silhouette(&text, &laid_out, &face);
        let reference_plain = render_silhouette(&united, false);
        let reference_border = render_silhouette(&united, true);
        save_evidence(&format!("{name}-reference-plain"), &reference_plain);
        save_evidence(&format!("{name}-reference-border"), &reference_border);
        for route in [StrokeRoute::None, StrokeRoute::Characters, StrokeRoute::ObjectAboveCharacters] {
            let actual = render_text(&text, route);
            save_evidence(&format!("{name}-{route:?}"), &actual);
            let black_pixels = black_mask(&actual).into_iter().filter(|on| *on).count();
            assert!(black_pixels > 100, "HOLD: no meaningful visible text: {name} {route:?}");
            let reference = if matches!(route, StrokeRoute::None) { &reference_plain } else { &reference_border };
            let (eligible, interior_red, missing_black) = interior_counts(&actual, reference);
            assert!(eligible > 100, "HOLD: no meaningful reference interior: {name} {route:?}");
            if matches!(route, StrokeRoute::None) {
                component_counts.push(components(black_mask(&actual)));
            } else {
                let red_pixels = (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).filter(|&(x, y)| is_red(actual.pixel(x, y))).count();
                assert!(red_pixels > 10, "HOLD: stroke route was not exercised: {name} {route:?}");
            }
            eprintln!("#6706 {name} {route:?}: black={black_pixels} interior={eligible} red_inside={interior_red} missing_black={missing_black}");
            observed.push((name, route, interior_red, missing_black));
        }
    }
    assert_eq!(component_counts.len(), 2);
    assert_eq!(cluster_shapes.len(), 2);
    assert_joining(&cluster_shapes[0], &cluster_shapes[1]);
    eprintln!("#6706 unstroked ink components, joined/ZWNJ: {component_counts:?}");
    assert!(
        component_counts[0] > 0 && component_counts[0] < component_counts[1],
        "HOLD: joined text must form fewer ink components than its ZWNJ control: {component_counts:?}"
    );
    // Observe both real stroke routes and the controls before reporting an internal-border defect.
    for (name, route, interior_red, missing_black) in observed {
        assert_eq!(interior_red, 0, "#6706: {name} {route:?} paints a stroke through the joined silhouette");
        assert_eq!(missing_black, 0, "{name} {route:?} loses the reference silhouette's interior ink");
    }
}

fn render_styled_text(text: &TextObject) -> Rendered {
    let mut node = Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(text.clone())));
    node.appearance = Appearance::default().into();
    render_node(node)
}

/// Keep the individual fills as the color oracle; the united black border only defines the
/// interior mask. An implementation that fixes the border by flattening the fills must fail too.
fn mixed_interior_counts(actual: &Rendered, no_stroke: &Rendered, border_reference: &Rendered) -> (usize, usize, usize, usize, usize) {
    let (mut eligible, mut red_inside, mut changed_fill, mut black_fill, mut blue_fill) = (0, 0, 0, 0, 0);
    for y in 1..HEIGHT - 1 {
        for x in 1..WIDTH - 1 {
            if (y - 1..=y + 1).all(|v| (x - 1..=x + 1).all(|u| is_black(border_reference.pixel(u, v)))) {
                eligible += 1;
                let (before, after) = (no_stroke.pixel(x, y), actual.pixel(x, y));
                red_inside += usize::from(is_red(after));
                changed_fill += usize::from(before.iter().zip(after).any(|(a, b)| a.abs_diff(b) > 2));
                black_fill += usize::from(is_black(before));
                blue_fill += usize::from(before[0] < 70 && before[1] < 120 && before[2] > 120 && before[3] == 255);
            }
        }
    }
    (eligible, red_inside, changed_fill, black_fill, blue_fill)
}

#[test]
fn arabic_joining_border_across_equal_stroke_runs() {
    use vectorcraft_doc::text::TypeAntiAlias;

    let db = FontDb::global();
    let Some(face) = craft_arabic_face("arabic_joining_border_across_equal_stroke_runs") else { return };
    let style = CharStyle {
        font_family: face.family.clone(),
        font_style: face.style.clone(),
        size: 192.0,
        fill: black(),
        stroke: Paint::None,
        stroke_width: 0.0,
        stroke_join: LineJoin::Round,
        ..Default::default()
    };
    let mut text = TextObject::point(Point::ZERO, JOINED, style);
    let uniform = layout(db, &text);
    assert_joined_probe(&face, &text, &uniform);
    place_ink(&mut text, &uniform);
    // First Beh black; the following two blue. Only fill changes at this two-byte boundary.
    vectorcraft_text::edit::style_range(&mut text.runs, 2, JOINED.len(), |s| s.fill = Paint::solid(Color::rgb8(0, 70, 180)));
    assert_eq!(text.runs.len(), 2);
    assert_ne!(text.runs[0].style.fill, text.runs[1].style.fill);
    assert!(text.runs.iter().all(|r| !r.style.has_stroke()));
    let mixed_layout = layout(db, &text);
    assert_eq!(geometry(&mixed_layout), geometry(&uniform), "HOLD: changing fill must preserve joining and glyph geometry");
    let united = silhouette(&text, &mixed_layout, &face);
    let border_reference = render_silhouette(&united, true);
    let mut observed = Vec::new();
    for method in [TypeAntiAlias::Sharp, TypeAntiAlias::Crisp] {
        let mut no_stroke = text.clone();
        no_stroke.anti_alias = method;
        let mut equal = no_stroke.clone();
        for run in &mut equal.runs {
            run.style.set_stroke_layer(&border());
        }
        assert_eq!(
            equal.runs[0].style.stroke_layer(),
            equal.runs[1].style.stroke_layer(),
            "complete stroke attributes must match across the fill boundary"
        );
        let mut different = equal.clone();
        different.runs[1].style.stroke_width = WEIGHT * 2.0;
        assert_ne!(different.runs[0].style.stroke_layer(), different.runs[1].style.stroke_layer());
        for candidate in [&no_stroke, &equal, &different] {
            assert_eq!(
                geometry(&layout(db, candidate)),
                geometry(&mixed_layout),
                "stroke/antialias changes must preserve glyph IDs, origins, transforms and advances"
            );
        }
        let plain_image = render_styled_text(&no_stroke);
        let equal_image = render_styled_text(&equal);
        let different_image = render_styled_text(&different);
        save_evidence(&format!("mixed-{method:?}-no-stroke"), &plain_image);
        save_evidence(&format!("mixed-{method:?}-equal-strokes"), &equal_image);
        save_evidence(&format!("mixed-{method:?}-different-strokes"), &different_image);
        save_evidence(&format!("mixed-{method:?}-reference-border-mask"), &border_reference);
        let (eligible, red_inside, changed_fill, black_fill, blue_fill) = mixed_interior_counts(&equal_image, &plain_image, &border_reference);
        assert!(
            eligible > 100 && black_fill > 20 && blue_fill > 20,
            "HOLD: both fills must have observable interior coverage: {method:?} eligible={eligible} black={black_fill} blue={blue_fill}"
        );
        let (different_red, different_changed, wider_border) =
            (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).fold((0, 0, 0), |(red_count, changed, growth), (x, y)| {
                let (a, b) = (equal_image.pixel(x, y), different_image.pixel(x, y));
                (red_count + usize::from(is_red(b)), changed + usize::from(a != b), growth + usize::from(is_red(b) && !is_red(a)))
            });
        eprintln!(
            "#6706 mixed {method:?}: interior={eligible} black={black_fill} blue={blue_fill} equal_red_inside={red_inside} equal_changed_fill={changed_fill}; different-stroke control red={different_red} changed={different_changed} extra_red={wider_border}"
        );
        // Different weights are a legitimate boundary: require its visible effect, not one united
        // border or preservation of all of its previous interior pixels.
        observed.push((method, red_inside, changed_fill, different_red, different_changed, wider_border));
    }
    // All images and both antialias paths are observed before reporting the coupled-style defect.
    for (method, red_inside, changed_fill, different_red, different_changed, wider_border) in observed {
        assert!(
            different_red > 10 && different_changed > 10 && wider_border > 10,
            "HOLD: the different-weight boundary must remain effective: {method:?}"
        );
        assert_eq!(red_inside, 0, "#6706: {method:?} equal strokes create an interior border at a fill-only run boundary");
        assert_eq!(changed_fill, 0, "#6706: {method:?} equal strokes replace the unstroked text's interior fill colors");
    }
}

/// A genuine joining font and visible original text, shared by the output/cache probes.
fn fresh_joined_probe(face: &FontFace) -> (TextObject, TextLayout) {
    let db = FontDb::global();
    let style = CharStyle {
        font_family: face.family.clone(),
        font_style: face.style.clone(),
        size: 192.0,
        fill: black(),
        stroke: Paint::None,
        stroke_width: 0.0,
        stroke_join: LineJoin::Round,
        ..Default::default()
    };
    let mut text = TextObject::point(Point::ZERO, JOINED, style);
    let laid_out = layout(db, &text);
    assert_joined_probe(face, &text, &laid_out);
    place_ink(&mut text, &laid_out);
    (text, laid_out)
}

// Reuse one Renderer through fill-only, appearance-only stroke, and unchanged frames.
// Both antialiasing routes keep the original genuine-font geometry and reference border.
#[test]
fn arabic_joining_stroke_survives_object_cache_transition() {
    use vectorcraft_doc::text::TypeAntiAlias;
    use vectorcraft_render::AntiAlias;

    let Some(face) = craft_arabic_face("arabic_joining_stroke_survives_object_cache_transition") else { return };

    let mut observed = Vec::new();
    for mode in [AntiAlias::Art, AntiAlias::Type] {
        let (mut text, original_layout) = fresh_joined_probe(&face);
        text.anti_alias = TypeAntiAlias::Sharp;
        assert!(text.runs.iter().all(|r| !r.style.has_stroke() && r.inline.is_none()));
        // Translation leaves the text-space geometry unchanged, but guarantees an exercised
        // pixel-snap row rather than an accidentally already aligned layout.
        text.xf = Affine::translate((0.375, 0.25)) * text.xf;
        assert_eq!(geometry(&layout(FontDb::global(), &text)), geometry(&original_layout));
        let mut expected_layout = layout(FontDb::global(), &text);
        let snapped = if mode == AntiAlias::Type { expected_layout.snap_to_pixels(text.xf) } else { false };
        assert!(mode != AntiAlias::Type || snapped, "HOLD: Type-optimized row must actually snap the fixture");
        let united = silhouette(&text, &expected_layout, &face);
        let reference_plain = render_silhouette(&united, false);
        let reference_border = render_silhouette(&united, true);
        let font_generation = FontDb::global().generation();

        let mut doc = Document::new(f64::from(WIDTH), f64::from(HEIGHT));
        let id = doc.alloc_id();
        let mut node = Node::new(id, NodeKind::Text(Box::new(text.clone())));
        node.appearance = Appearance::default().into();
        doc.insert(Some(doc.layers[0].id), 0, node).unwrap();
        // Holding the old Arc makes the appearance edit's COW identity transition explicit.
        let before = doc.layers[0].children().unwrap().first().unwrap().clone();
        assert!(std::ptr::eq(before.as_ref(), doc.node(id).unwrap()));
        let NodeKind::Text(initial) = &doc.node(id).unwrap().kind else { panic!("text missing") };
        assert!(matches!(doc.inline_resolved(initial), std::borrow::Cow::Borrowed(_)), "HOLD: inline resolution must keep the node's own TextObject");

        let options = RenderOptions { anti_alias: mode, background: Some([255, 255, 255, 255]), artboards: false, ..Default::default() };
        let mut renderer = Renderer::new();
        renderer.threads = 0;
        let fill_only = renderer.render(&doc, WIDTH, HEIGHT, Affine::IDENTITY, &options);
        assert!(renderer.stats.drawn > 0);
        assert_eq!(renderer.stats.type_edges, 0, "the fixture uses Sharp type, not offscreen non-Sharp text pixels");
        eprintln!(
            "#6706 cache {mode:?} fill-only frame: drawn={} type_edges={} font_generation={font_generation} snapped={snapped}",
            renderer.stats.drawn, renderer.stats.type_edges
        );

        {
            let node = doc.node_mut(id).unwrap();
            node.appearance.items.push(AppearanceItem::Stroke(border()));
            node.appearance.set_contents_at(0);
        }
        assert!(!std::ptr::eq(before.as_ref(), doc.node(id).unwrap()), "HOLD: the edit must create the new node Arc used by same-id cache reuse");
        assert!(before.appearance.items.is_empty(), "the retained fill-only node must remain unchanged");
        let NodeKind::Text(current) = &doc.node(id).unwrap().kind else { panic!("text missing") };
        assert_eq!(current.as_ref(), &text, "appearance-only edit must leave the complete TextObject unchanged");
        assert_eq!(geometry(&layout(FontDb::global(), current)), geometry(&original_layout));
        assert!(matches!(doc.inline_resolved(current), std::borrow::Cow::Borrowed(_)));

        let stroked = renderer.render(&doc, WIDTH, HEIGHT, Affine::IDENTITY, &options);
        assert!(renderer.stats.drawn > 0);
        eprintln!("#6706 cache {mode:?} appearance-only stroke frame: drawn={} type_edges={}", renderer.stats.drawn, renderer.stats.type_edges);
        let repeated = renderer.render(&doc, WIDTH, HEIGHT, Affine::IDENTITY, &options);
        assert_eq!(FontDb::global().generation(), font_generation, "HOLD: a font-generation change would invalidate the intended cache transition");
        save_evidence(&format!("cache-{mode:?}-fill-only"), &fill_only);
        save_evidence(&format!("cache-{mode:?}-appearance-stroke"), &stroked);
        save_evidence(&format!("cache-{mode:?}-repeat"), &repeated);
        save_evidence(&format!("cache-{mode:?}-reference-border"), &reference_border);
        let repeat_equal = stroked.pixels == repeated.pixels;
        for (stage, actual, reference) in [
            ("fill-only", &fill_only, &reference_plain),
            ("appearance-stroke", &stroked, &reference_border),
            ("repeat", &repeated, &reference_border),
        ] {
            let (eligible, red_inside, missing_black) = interior_counts(actual, reference);
            let red_pixels = (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).filter(|&(x, y)| is_red(actual.pixel(x, y))).count();
            eprintln!(
                "#6706 cache {mode:?} {stage}: interior={eligible} red_inside={red_inside} missing_black={missing_black} visible_red={red_pixels} repeat_equal={repeat_equal}"
            );
            observed.push((mode, stage, eligible, red_inside, missing_black, red_pixels, repeat_equal));
        }
    }
    // Both routes and every actual image are observed before the pixel-fidelity assertions.
    for (mode, stage, eligible, red_inside, missing_black, red_pixels, repeat_equal) in observed {
        assert!(eligible > 100, "HOLD: {mode:?} {stage} did not exercise meaningful reference ink");
        assert_eq!(red_inside, 0, "#6706: {mode:?} {stage} reintroduced an interior glyph border");
        assert_eq!(missing_black, 0, "#6706: {mode:?} {stage} lost the original fill interior");
        if stage == "fill-only" {
            assert_eq!(red_pixels, 0);
        } else {
            assert!(red_pixels > 10, "HOLD: the object Appearance border must actually draw");
        }
        assert!(repeat_equal, "#6706: unchanged repeated frame must keep the actual stroked pixels");
    }
}

fn assert_unstroked_conversion(fill_name: &str, consumer: &str, mixed: bool, converted_plain: &Rendered, no_stroke: &Rendered, reference: &Rendered) {
    save_evidence(&format!("outputs-{fill_name}-{consumer}-no-stroke-control"), converted_plain);
    let (eligible, red_inside, baseline_changed, _, _) = mixed_interior_counts(converted_plain, no_stroke, reference);
    let (mut solid, mut changed_solid, mut black, mut blue) = (0, 0, 0, 0);
    for y in 1..HEIGHT - 1 {
        for x in 1..WIDTH - 1 {
            let before = no_stroke.pixel(x, y);
            let is_black_fill = before == [0, 0, 0, 255];
            let is_blue_fill = before == [0, 70, 180, 255];
            // This independent ownership control stays away from BOTH the exterior stroke
            // and the antialiased boundary between the two original fill colors.
            if !(is_black_fill || is_blue_fill)
                || !(y - 1..=y + 1).all(|v| (x - 1..=x + 1).all(|u| is_black(reference.pixel(u, v)) && no_stroke.pixel(u, v) == before))
            {
                continue;
            }
            solid += 1;
            black += usize::from(is_black_fill);
            blue += usize::from(is_blue_fill);
            changed_solid += usize::from(before.iter().zip(converted_plain.pixel(x, y)).any(|(a, b)| a.abs_diff(b) > 2));
        }
    }
    eprintln!(
        "#6706 unstroked conversion {fill_name} {consumer}: interior={eligible} conversion_vs_direct_changed={baseline_changed} red_inside={red_inside} solid={solid} changed_solid={changed_solid} black={black} blue={blue}"
    );
    assert!(eligible > 100 && solid > 100 && black > 20, "HOLD: the unstroked control must have meaningful fill ownership");
    if mixed {
        assert!(blue > 20, "HOLD: the unstroked conversion must exercise the original blue fill too");
    }
    assert_eq!(red_inside, 0, "the unstroked conversion cannot create a red stroke");
    assert_eq!(changed_solid, 0, "the conversion cannot move or flatten solid original fill ownership: {fill_name} {consumer}");
}

// Genuine-font output coverage preserves the original font and joining assertions.
// Four inputs x three actual output consumers = twelve CPU output observations.
// Four stroked inputs x three output consumers, plus matching unstroked conversion controls.
#[test]
fn arabic_joining_strokes_match_outline_and_svg_outputs() {
    use std::collections::HashSet;

    let Some(face) = craft_arabic_face("arabic_joining_strokes_match_outline_and_svg_outputs") else { return };

    let paint_document = |doc: &Document| {
        let mut ids = HashSet::new();
        doc.walk(|node| assert!(ids.insert(node.id), "HOLD: output document contains duplicate node IDs"));
        let mut renderer = Renderer::new();
        renderer.threads = 0;
        renderer.render(
            doc,
            WIDTH,
            HEIGHT,
            Affine::IDENTITY,
            &RenderOptions { background: Some([255, 255, 255, 255]), artboards: false, ..Default::default() },
        )
    };
    let source_document = |text: TextObject, object_stroke: bool| {
        let mut source = Document::new(f64::from(WIDTH), f64::from(HEIGHT));
        let mut node = Node::new(source.alloc_id(), NodeKind::Text(Box::new(text)));
        if object_stroke {
            let mut appearance = Appearance { items: vec![AppearanceItem::Stroke(border())], ..Default::default() };
            appearance.set_contents_at(0);
            node.appearance = appearance.into();
        } else {
            node.appearance = Appearance::default().into();
        }
        source.insert(Some(source.layers[0].id), 0, node.clone()).expect("HOLD: source text insertion failed");
        (source, node)
    };
    // The SAME real conversion is used for the unstroked control and both stroke routes.
    let converted_outputs = |source: &Document, node: &Node| {
        let mut outlined = vectorcraft_render::effects::outline_text(node).expect("HOLD: the actual text outliner returned no art");
        assert!(outlined.extras().type_outlines, "the actual conversion must preserve its type-outline marker");
        let mut outline_doc = Document::new(f64::from(WIDTH), f64::from(HEIGHT));
        outlined.id = outline_doc.alloc_id();
        let mut seen = HashSet::new();
        outline_doc.walk(|node| {
            seen.insert(node.id);
        });
        vectorcraft_render::effects::fresh_ids(&mut outline_doc, &mut outlined, &mut seen);
        outline_doc.insert(Some(outline_doc.layers[0].id), 0, outlined).expect("HOLD: outlined art insertion failed");
        let mut outputs = vec![("effects-outline", paint_document(&outline_doc))];
        for outline_text in [false, true] {
            let svg = vectorcraft_svg::export(source, &vectorcraft_svg::ExportOptions { outline_text, ..Default::default() });
            let imported = vectorcraft_svg::import(&svg).expect("HOLD: actual exported SVG did not import");
            let (mut paths, mut live_text) = (0, 0);
            imported.walk(|node| match &node.kind {
                NodeKind::Path { .. } | NodeKind::Compound { .. } => paths += 1,
                NodeKind::Text(_) => live_text += 1,
                _ => {}
            });
            assert!(paths > 0 && live_text == 0, "HOLD: the genuine Arabic output must exercise the existing outlined SVG route");
            outputs.push((if outline_text { "svg-explicit-outline" } else { "svg-natural-rtl-outline" }, paint_document(&imported)));
        }
        outputs
    };
    let (base, base_layout) = fresh_joined_probe(&face);
    assert!(base.runs.iter().all(|run| !run.style.has_stroke()), "HOLD: the source helper must start unstroked");
    let reference = render_silhouette(&silhouette(&base, &base_layout, &face), true);
    save_evidence("outputs-united-border-reference", &reference);
    let mut observed = Vec::new();
    for mixed in [false, true] {
        let mut plain = base.clone();
        if mixed {
            vectorcraft_text::edit::style_range(&mut plain.runs, 2, JOINED.len(), |style| {
                style.fill = Paint::solid(Color::rgb8(0, 70, 180));
            });
            assert_eq!(plain.runs.len(), 2, "HOLD: the mixed source must expose its authored fill boundary");
            assert_ne!(plain.runs[0].style.fill, plain.runs[1].style.fill);
        }
        let plain_layout = layout(FontDb::global(), &plain);
        assert_eq!(geometry(&plain_layout), geometry(&base_layout), "fill changes must not reshape or move the genuine joined glyphs");
        assert_eq!(plain_layout.bounds, base_layout.bounds, "fill changes must not change layout bounds");
        let no_stroke = render_text(&plain, StrokeRoute::None);
        let fill_name = if mixed { "mixed" } else { "uniform" };
        save_evidence(&format!("outputs-{fill_name}-no-stroke-oracle"), &no_stroke);
        let (plain_source, plain_node) = source_document(plain.clone(), false);
        let plain_outputs = converted_outputs(&plain_source, &plain_node);
        assert_eq!(plain_outputs.len(), 3);
        for (consumer, converted_plain) in &plain_outputs {
            assert_unstroked_conversion(fill_name, consumer, mixed, converted_plain, &no_stroke, &reference);
        }
        for object_stroke in [false, true] {
            let mut text = plain.clone();
            if !object_stroke {
                for run in &mut text.runs {
                    run.style.set_stroke_layer(&border());
                }
                assert!(text.runs.iter().all(|run| run.style.stroke_layer() == border()));
            }
            let painted_layout = layout(FontDb::global(), &text);
            assert_eq!(geometry(&painted_layout), geometry(&base_layout), "strokes must preserve shaping, positioning and advances");
            assert_eq!(painted_layout.bounds, base_layout.bounds, "strokes must preserve layout bounds");
            assert_eq!(text.xf, base.xf, "the output inputs retain the original translated frame");
            let stroke_name = if object_stroke { "object-above" } else { "characters" };
            let name = format!("{fill_name}-{stroke_name}");
            let (source, node) = source_document(text, object_stroke);
            let outputs = converted_outputs(&source, &node);
            for (consumer, image) in outputs {
                let matched_plain = &plain_outputs.iter().find(|(name, _)| *name == consumer).expect("HOLD: matching conversion control missing").1;
                save_evidence(&format!("outputs-{name}-{consumer}"), &image);
                let red_pixels = (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).filter(|&(x, y)| is_red(image.pixel(x, y))).count();
                let (eligible, red_inside, changed_fill, black_fill, blue_fill) = if mixed {
                    mixed_interior_counts(&image, matched_plain, &reference)
                } else {
                    let (eligible, red_inside, missing_black) = interior_counts(&image, &reference);
                    (eligible, red_inside, missing_black, eligible, 0)
                };
                eprintln!(
                    "#6706 output {name} {consumer}: stroke_red={red_pixels} interior={eligible} red_inside={red_inside} changed_fill={changed_fill} black={black_fill} blue={blue_fill}"
                );
                assert!(
                    red_pixels > 10 && eligible > 100,
                    "HOLD: the actual output must contain both visible stroke and meaningful ink: {name} {consumer}"
                );
                if mixed {
                    assert!(black_fill > 20 && blue_fill > 20, "HOLD: both authored fills must have observable ownership: {name} {consumer}");
                }
                observed.push((name.clone(), consumer, red_inside, changed_fill));
            }
        }
    }
    assert_eq!(observed.len(), 12, "all four inputs must exercise all three actual output consumers");
    for (input, consumer, red_inside, changed_fill) in observed {
        assert_eq!(red_inside, 0, "#6706 {input} {consumer}: an internal border remains in the actual painted output");
        assert_eq!(changed_fill, 0, "#6706 {input} {consumer}: the actual output changed or removed its original interior fill");
    }
}
