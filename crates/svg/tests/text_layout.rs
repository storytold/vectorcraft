//! Laid-out type in SVG: every line of area type is its own positioned `<tspan>`, rich text keeps
//! its leading, baseline shift, scaling and kerning, and a renderer with the same font draws the
//! export where the canvas draws the text.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{CharStyle, Document, Justify, Node, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

fn style(size: f64) -> CharStyle {
    CharStyle { size, fill: Paint::solid(Color::BLACK), ..CharStyle::default() }
}

/// Area type `w` wide at (20, 20) in a 300 × 200 document.
fn area_doc(runs: Vec<TextRun>, w: f64, justify: Justify) -> Document {
    let mut t = TextObject::point(Point::new(20.0, 20.0), "", style(12.0));
    t.runs = runs;
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, 160.0)) };
    t.para.justify = justify;
    let mut d = Document::new(300.0, 200.0);
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn run(text: &str, st: CharStyle) -> TextRun {
    TextRun { text: text.into(), style: st, inline: None }
}

/// The `<tspan …>` start tags.
fn tspans(svg: &str) -> Vec<&str> {
    svg.match_indices("<tspan").map(|(i, _)| &svg[i..i + svg[i..].find('>').unwrap()]).collect()
}

fn attr(tag: &str, name: &str) -> Option<f64> {
    let k = format!(" {name}=\"");
    let i = tag.find(&k)? + k.len();
    tag[i..i + tag[i..].find('"')?].parse().ok()
}

#[test]
fn wrapped_lines_get_one_tspan_each() {
    let d = area_doc(vec![run("alpha beta gamma delta epsilon zeta", style(12.0))], 70.0, Justify::Left);
    let svg = export(&d, &ExportOptions::default());
    let tags = tspans(&svg);
    let ys: Vec<f64> = tags.iter().map(|t| attr(t, "y").expect("each line is placed")).collect();
    assert!(ys.len() >= 3, "the text wraps: {svg}");
    assert!(ys.windows(2).all(|w| w[1] > w[0]), "baselines go down: {ys:?}");
    // Auto leading: 120 % of the size between baselines.
    assert!((ys[1] - ys[0] - 14.4).abs() < 0.01, "{ys:?}");
    assert!(tags.iter().all(|t| attr(t, "x") == Some(0.0)), "left aligned at the frame's edge: {svg}");
    // Reading it back gives a line per baseline, starting at the first baseline.
    let back = vectorcraft_svg::import(&svg).unwrap();
    let text = back.layers[0].children().unwrap().iter().find_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.clone()) } else { None }).unwrap();
    assert_eq!(text.plain_text().lines().count(), ys.len(), "{:?}", text.plain_text());
    assert!((text.xf.translation().y - (20.0 + ys[0])).abs() < 0.01);
    // Centred lines are anchored at the frame's centre, so they stay centred in any font.
    let svg = export(&area_doc(vec![run("alpha beta gamma delta", style(12.0))], 70.0, Justify::Center), &ExportOptions::default());
    assert!(svg.contains("text-anchor=\"middle\""), "{svg}");
    assert!(tspans(&svg).iter().all(|t| (attr(t, "x").unwrap() - 35.0).abs() < 0.01), "{svg}");
    // Left-aligned (and justified) lines are placed absolutely.
    assert!(!export(&d, &ExportOptions::default()).contains("text-anchor"));
}

#[test]
fn vertically_aligned_area_type_exports_where_it_is_drawn() {
    let top = area_doc(vec![run("alpha beta", style(12.0))], 200.0, Justify::Left);
    let mut centred = top.clone();
    let id = centred.layers[0].children().unwrap()[0].id;
    if let Some(NodeKind::Text(t)) = centred.node_mut(id).map(|n| &mut n.kind) {
        t.area.vertical_align = vectorcraft_doc::VerticalAlign::Center;
    }
    let y = |d: &Document| attr(tspans(&export(d, &ExportOptions::default()))[0], "y").unwrap();
    let (yt, yc) = (y(&top), y(&centred));
    // The 160 pt frame holds one 12 pt line: centring moves it down about (160 - 14.4) / 2.
    assert!(yc - yt > 60.0 && yc - yt < 80.0, "{yt} → {yc}");
}

#[test]
fn rich_text_writes_spacing_shift_and_scale() {
    let shifted = CharStyle { baseline_shift: 4.0, ..style(12.0) };
    let wide = CharStyle { h_scale: 150.0, ..style(12.0) };
    let kerned = CharStyle { tracking: 50.0, kerning: Some(-20.0), ..style(12.0) };
    let tall = CharStyle { v_scale: 200.0, h_scale: 200.0, leading: Some(30.0), ..style(12.0) };
    let d = area_doc(
        vec![run("Base ", style(12.0)), run("up ", shifted), run("wide ", wide), run("kern\n", kerned), run("tall", tall)],
        280.0,
        Justify::Left,
    );
    let svg = export(&d, &ExportOptions::default());
    let tags = tspans(&svg);
    assert_eq!(tags.len(), 5, "{svg}");
    assert_eq!(attr(tags[1], "dy"), Some(-4.0), "baseline shift: {}", tags[1]);
    assert!(attr(tags[2], "textLength").is_some() && tags[2].contains("lengthAdjust=\"spacingAndGlyphs\""), "{}", tags[2]);
    assert!(tags[3].contains("letter-spacing=\"0.36\"") && tags[3].contains("font-kerning:none"), "tracking + kerning: {}", tags[3]);
    assert!(tags[4].contains("font-size=\"24\"") && attr(tags[4], "textLength").is_none(), "uniform scale is the size: {}", tags[4]);
    assert!((attr(tags[4], "y").unwrap() - attr(tags[0], "y").unwrap() - 30.0).abs() < 0.01, "leading");
    // Segments on one baseline share its y; the shifted one is placed with dy.
    assert_eq!(attr(tags[1], "y"), attr(tags[0], "y"));
}

/// Proportional Metrics (#966) is written as the `palt` feature, beside the character's own.
#[test]
fn proportional_metrics_write_palt() {
    let proportional = CharStyle { proportional_metrics: true, ..style(12.0) };
    let both = CharStyle { proportional_metrics: true, features: vec!["dlig".into()], ..style(12.0) };
    let d = area_doc(vec![run("雅楽 ", style(12.0)), run("「あ」 ", proportional), run("ab", both)], 280.0, Justify::Left);
    let svg = export(&d, &ExportOptions::default());
    let tags = tspans(&svg);
    assert_eq!(tags.len(), 3, "{svg}");
    assert!(!tags[0].contains("palt"), "{}", tags[0]);
    assert!(tags[1].contains("font-feature-settings:&quot;palt&quot; 1"), "{}", tags[1]);
    assert!(tags[2].contains("dlig") && tags[2].contains("palt"), "{}", tags[2]);
}

/// Digits are written as drawn: the style's numeral system, not the digits that were typed (as
/// All Caps writes capitals).
#[test]
fn digits_are_written_as_drawn() {
    use vectorcraft_doc::Digits;
    let hindi = CharStyle { digits: Digits::Hindi, ..style(12.0) };
    let farsi = CharStyle { digits: Digits::Farsi, ..style(12.0) };
    let d = area_doc(vec![run("page 3, ", style(12.0)), run("page 4, ", hindi), run("page ٥", farsi)], 280.0, Justify::Left);
    let svg = export(&d, &ExportOptions::default());
    let text = svg.as_str();
    assert!(text.contains("page 3,"), "typed Western digits stay in Default: {svg}");
    assert!(text.contains("page ٤,"), "Hindi draws 4 as ٤: {svg}");
    assert!(text.contains("page ۵"), "Farsi draws ٥ as ۵: {svg}");
    assert!(!text.contains("page 4") && !text.contains("page ٥"), "{svg}");
}

#[test]
fn fewer_tspans_writes_one_per_line() {
    let bold = CharStyle { font_style: "Bold".into(), ..style(12.0) };
    let runs = vec![run("one ", style(12.0)), run("two", bold), run(" three\nfour", style(12.0))];
    let d = area_doc(runs, 280.0, Justify::Left);
    let full = export(&d, &ExportOptions::default());
    let fewer = export(&d, &ExportOptions { fewer_tspans: true, ..Default::default() });
    let placed = |s: &str| tspans(s).iter().filter(|t| attr(t, "y").is_some()).count();
    assert_eq!(placed(&full), 4, "{full}");
    assert_eq!(placed(&fewer), 2, "one positioned tspan per line: {fewer}");
    assert!(fewer.contains("font-weight=\"bold\""), "style changes stay: {fewer}");
    assert!(fewer.len() < full.len());
}

#[test]
fn justified_lines_place_each_word() {
    let d = area_doc(vec![run("aa bb cc dd ee ff gg hh ii jj kk ll", style(12.0))], 90.0, Justify::JustifyLeft);
    let svg = export(&d, &ExportOptions::default());
    let first_line_y = attr(tspans(&svg)[0], "y").unwrap();
    let words = tspans(&svg).iter().filter(|t| attr(t, "y") == Some(first_line_y)).count();
    assert!(words >= 3, "{svg}");
}

/// The only type in `d`.
fn the_text(d: &Document) -> TextObject {
    d.layers[0].children().unwrap().iter().find_map(|n| if let NodeKind::Text(t) = &n.kind { Some((**t).clone()) } else { None }).unwrap()
}

/// A drawn character: its text, where its first glyph is drawn, and the glyph's horizontal scale
/// (font units to user space).
type Drawn = (String, Point, f64);

/// Each character a usvg draws from `svg` (with the bundled font and the font files of `faces`)
/// and how it draws the character's first glyph, in SVG user space and text order; spaces and
/// tabs are left out.
fn viewer_glyph_positions(svg: &str, faces: &[Arc<vectorcraft_text::FontFace>]) -> Vec<Drawn> {
    let mut db = usvg::fontdb::Database::new();
    db.load_font_data(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap());
    for f in faces {
        db.load_font_data(f.file_data().to_vec());
    }
    let opt = usvg::Options { fontdb: Arc::new(db), ..Default::default() };
    let tree = usvg::Tree::from_str(svg, &opt).unwrap();
    fn walk(g: &usvg::Group, out: &mut Vec<Drawn>) {
        for n in g.children() {
            match n {
                usvg::Node::Group(g) => walk(g, out),
                usvg::Node::Text(t) => {
                    for g in t.layouted().iter().flat_map(|span| &span.positioned_glyphs) {
                        if g.text.trim().is_empty() {
                            continue;
                        }
                        let m = t.abs_transform().pre_concat(g.transform());
                        out.push((g.text.clone(), Point::new(m.tx as f64, m.ty as f64), m.sx as f64));
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    walk(tree.root(), &mut out);
    out
}

/// Each character the canvas draws of the only type in `d` and how it draws the character's
/// first glyph, in document space and text order (spaces and tabs left out), with the faces
/// it draws them in.
fn canvas_glyph_positions(d: &Document) -> (Vec<Drawn>, Vec<Arc<vectorcraft_text::FontFace>>) {
    let t = the_text(d);
    let db = vectorcraft_text::FontDb::global();
    let lay = vectorcraft_text::layout(db, &t);
    let plain = t.plain_text();
    let mut out: Vec<Drawn> = vec![];
    let mut faces: Vec<Arc<vectorcraft_text::FontFace>> = vec![];
    let mut last = None;
    for g in &lay.glyphs {
        let text = if g.len == 0 { "-" } else { &plain[g.byte..g.byte + g.len] };
        if text.trim().is_empty() || (g.len > 0 && last == Some(g.byte)) {
            continue;
        }
        last = Some(g.byte);
        if let Some(f) = db.face_by_id(g.font_id).filter(|f| !faces.iter().any(|o| o.id() == f.id())) {
            faces.push(f);
        }
        let m = t.xf * g.xf;
        out.push((text.to_string(), m * Point::ZERO, m.as_coeffs()[0]));
    }
    (out, faces)
}

/// Asserts that a viewer draws every character of `d`'s live SVG text where the canvas draws it,
/// as wide as the canvas draws it.
fn assert_drawn_in_place(d: &Document) {
    let svg = export(d, &ExportOptions::default());
    let (canvas, faces) = canvas_glyph_positions(d);
    let viewer = viewer_glyph_positions(&svg, &faces);
    let chars = |v: &[Drawn]| v.iter().map(|(s, _, _)| s.as_str()).collect::<String>();
    assert_eq!(chars(&viewer), chars(&canvas), "{svg}");
    for ((s, v, vs), (_, c, cs)) in viewer.iter().zip(&canvas) {
        assert!((v.x - c.x).abs() < 0.05 && (v.y - c.y).abs() < 0.05, "{s}: the viewer draws it at {v:?}, the canvas at {c:?}: {svg}");
        assert!((vs / cs - 1.0).abs() < 0.002, "{s}: the viewer scales it by {vs}, the canvas by {cs}: {svg}");
    }
}

/// A Justify All line without word spaces spreads its letters; live text keeps the gaps, with
/// tracking and manual kerning too.
#[test]
fn justify_all_lines_keep_their_letter_gaps() {
    let tracked = CharStyle { tracking: 80.0, ..style(12.0) };
    let kerned = CharStyle { kerning: Some(-40.0), ..style(12.0) };
    for runs in [vec![run("abcdefgh", style(12.0))], vec![run("abc", style(12.0)), run("def", tracked), run("ghi", kerned)]] {
        let d = area_doc(runs, 200.0, Justify::JustifyAll);
        assert_drawn_in_place(&d);
    }
}

/// Justified and Mojikumi lines whose characters a reader draws where the layout does are written
/// as before: a segment per style run, and per word on a justified line.
#[test]
fn lines_without_moved_characters_are_written_as_before() {
    let text = |d: &Document| {
        let svg = export(d, &ExportOptions::default());
        svg[svg.find("<text").unwrap()..svg.find("</text>").unwrap()].to_string()
    };
    let justified = area_doc(vec![run("aa bb cc dd ee ff", style(12.0))], 60.0, Justify::JustifyLeft);
    let mut composed = area_doc(vec![run("Latin words only", style(12.0))], 200.0, Justify::Center);
    let id = composed.layers[0].children().unwrap()[0].id;
    if let Some(NodeKind::Text(t)) = composed.node_mut(id).map(|n| &mut n.kind) {
        t.para.mojikumi = vectorcraft_doc::Mojikumi::LineEndHalf;
    }
    let open = |x: &str, anchor: &str| {
        format!(
            r##"<text transform="matrix(1 0 0 1 20 20)" x="{x}" y="12" xml:space="preserve" font-family="&apos;Source Sans 3&apos;" font-size="12" fill="#000000"{anchor}>"##
        )
    };
    let words =
        [("aa ", "0", "12"), ("bb ", "15.636", "12"), ("cc ", "32.448", "12"), ("dd", "46.68", "12"), ("ee ", "0", "26.4"), ("ff", "14.304", "26.4")];
    let words: String = words.iter().map(|(w, x, y)| format!(r#"<tspan x="{x}" y="{y}">{w}</tspan>"#)).collect();
    assert_eq!(text(&justified), format!("{}{words}", open("0", "")));
    let centered = format!(r#"{}<tspan x="100" y="12">Latin words only</tspan>"#, open("100", r#" text-anchor="middle""#));
    assert_eq!(text(&composed), centered);
}

/// Type scaled horizontally on a justified line is stretched to its own width, not to the
/// justification space after it, nor to the font's kerning against the letter after it when
/// the letters are spread (AV is a kerned pair).
#[test]
fn justified_lines_keep_scaled_type_at_its_width() {
    let wide = CharStyle { h_scale: 150.0, ..style(12.0) };
    for justify in [Justify::JustifyLeft, Justify::JustifyAll] {
        assert_drawn_in_place(&area_doc(vec![run("aa bb cc dd ee ff gg hh ii jj kk ll", wide.clone())], 120.0, justify));
        assert_drawn_in_place(&area_doc(vec![run("abcdefgh", wide.clone())], 200.0, justify));
    }
    assert_drawn_in_place(&area_doc(vec![run("AVAVAVAV", wide)], 200.0, Justify::JustifyAll));
}

/// On lines that aren't spread, the spacing a reader applies itself (tracking, manual kerning,
/// horizontal scale, baseline shift) puts each character where the canvas draws it.
#[test]
fn ragged_lines_keep_their_character_spacing() {
    let tracked = CharStyle { tracking: 80.0, ..style(12.0) };
    let kerned = CharStyle { kerning: Some(-40.0), ..style(12.0) };
    let wide = CharStyle { h_scale: 150.0, ..style(12.0) };
    let shifted = CharStyle { baseline_shift: 3.0, ..style(12.0) };
    for justify in [Justify::Left, Justify::Center, Justify::Right, Justify::JustifyLeft] {
        let mut runs =
            vec![run("plain ", style(12.0)), run("tracked ", tracked.clone()), run("kerned ", kerned.clone()), run("up ", shifted.clone())];
        // usvg anchors a centered or right-aligned line by its width before `textLength` stretches it.
        if matches!(justify, Justify::Left | Justify::JustifyLeft) {
            runs.push(run("wide ", wide.clone()));
        }
        runs.push(run("end", style(12.0)));
        assert_drawn_in_place(&area_doc(runs, 280.0, justify));
    }
}

/// Justified Japanese lines spread between their characters (#344); live text keeps the gaps.
#[test]
fn justified_japanese_lines_keep_the_gaps_between_their_characters() {
    let text = "日本語の文章は文字と文字の間で両端を揃えます。この段落は三行になります。";
    for justify in [Justify::JustifyLeft, Justify::JustifyAll] {
        assert_drawn_in_place(&area_doc(vec![run(text, style(12.0))], 100.0, justify));
    }
}

/// Area type that Shrink Text to Fit scales down to fit its frame is written at the size the
/// canvas draws it at: sizes, tracking, manual kerning and baseline shifts, on ragged, anchored
/// and justified lines.
#[test]
fn shrunk_area_type_is_written_at_the_size_it_draws_at() {
    let tracked = CharStyle { tracking: 80.0, ..style(12.0) };
    let kerned = CharStyle { kerning: Some(-40.0), ..style(12.0) };
    let wide = CharStyle { h_scale: 150.0, ..style(12.0) };
    let shifted = CharStyle { baseline_shift: 3.0, ..style(12.0) };
    for justify in [Justify::Left, Justify::Center, Justify::JustifyLeft, Justify::JustifyAll] {
        // Tracked and kerned letters are inside words: usvg also spaces a line's last letter, which
        // moves an anchored line ending in one.
        let mut runs = vec![
            run("plain words ", style(12.0)),
            run("trac", tracked.clone()),
            run("ked ", style(12.0)),
            run("ker", kerned.clone()),
            run("ned ", style(12.0)),
            run("up ", shifted.clone()),
            run("big ", style(18.0)),
        ];
        // usvg anchors a centered line by its width before `textLength` stretches it.
        if justify != Justify::Center {
            runs.push(run("wide ", wide.clone()));
        }
        runs.push(run("and more words at the end", style(12.0)));
        let mut d = area_doc(runs, 90.0, justify);
        let id = d.layers[0].children().unwrap()[0].id;
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 90.0, 40.0)) };
            t.area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 10.0 };
        }
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &the_text(&d));
        assert!(!lay.overflow && lay.fit_scale < 0.8, "{justify:?}: the text is shrunk to fit ({})", lay.fit_scale);
        assert_drawn_in_place(&d);
        // Fewer `<tspan>` Elements writes the shrunk size too.
        let fewer = export(&d, &ExportOptions { fewer_tspans: true, ..Default::default() });
        let size = attr(&fewer[fewer.find("<text").unwrap()..], "font-size").unwrap();
        assert!((size - 12.0 * lay.fit_scale).abs() < 1e-3, "{justify:?}: {fewer}");
    }
}

/// Japanese composition (Mojikumi) closes up consecutive punctuation and adds a quarter em between
/// Japanese and Latin letters; live text keeps that spacing, on anchored lines too.
#[test]
fn japanese_composition_keeps_its_spacing() {
    let text = "「かっこ」、「かな」。漢字とLatinの間。";
    for justify in [Justify::Left, Justify::Center, Justify::Right] {
        let mut d = area_doc(vec![run(text, style(12.0))], 280.0, justify);
        let id = d.layers[0].children().unwrap()[0].id;
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            t.para.mojikumi = vectorcraft_doc::Mojikumi::LineEndHalf;
        }
        assert_drawn_in_place(&d);
    }
}

/// Glyph outlines of every text a usvg (with the bundled font) lays out from `svg`, in SVG
/// user space.
fn viewer_outlines(svg: &str) -> Vec<BezPath> {
    let mut db = usvg::fontdb::Database::new();
    db.load_font_data(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap());
    let opt = usvg::Options { fontdb: Arc::new(db), ..Default::default() };
    let tree = usvg::Tree::from_str(svg, &opt).unwrap();
    fn walk(g: &usvg::Group, out: &mut Vec<BezPath>) {
        for n in g.children() {
            match n {
                usvg::Node::Group(g) => walk(g, out),
                usvg::Node::Text(t) => walk(t.flattened(), out),
                usvg::Node::Path(p) => {
                    let m = p.abs_transform();
                    let m = Affine::new([m.sx as f64, m.ky as f64, m.kx as f64, m.sy as f64, m.tx as f64, m.ty as f64]);
                    let mut bp = BezPath::new();
                    for s in p.data().segments() {
                        use usvg::tiny_skia_path::PathSegment as S;
                        let pt = |q: usvg::tiny_skia_path::Point| m * Point::new(q.x as f64, q.y as f64);
                        match s {
                            S::MoveTo(a) => bp.move_to(pt(a)),
                            S::LineTo(a) => bp.line_to(pt(a)),
                            S::QuadTo(a, b) => bp.quad_to(pt(a), pt(b)),
                            S::CubicTo(a, b, c) => bp.curve_to(pt(a), pt(b), pt(c)),
                            S::Close => bp.close_path(),
                        }
                    }
                    out.push(bp);
                }
                usvg::Node::Image(_) => {}
            }
        }
    }
    let mut out = vec![];
    walk(tree.root(), &mut out);
    out
}

#[test]
fn a_viewer_draws_the_text_where_the_canvas_does() {
    // With a tab every line is placed absolutely; without one, centred and right-aligned lines are
    // anchored, and a baseline shift mid-line is undone by the next run.
    for (sep, shift) in [("	", 0.0), (" ", 3.0)] {
        let second = |fill: Color| CharStyle { fill: Paint::solid(fill), baseline_shift: shift, ..style(14.0) };
        let runs = |second: CharStyle| {
            vec![
                run("The quick brown fox jumps over the lazy dog. ", style(14.0)),
                run(&format!("Pack my box{sep}with"), second),
                run(" five dozen jugs.", style(14.0)),
            ]
        };
        for justify in [Justify::Left, Justify::Center, Justify::Right] {
            for fewer_tspans in [false, true] {
                let o = ExportOptions { fewer_tspans, ..Default::default() };
                let svg = export(&area_doc(runs(second(Color::rgb(0.8, 0.0, 0.0))), 200.0, justify), &o);
                assert!(svg.contains("fill=\"#cc0000\""), "{svg}");
                // The viewer's glyphs as black shapes, against the canvas drawing the text all black.
                let mut viewer = Document::new(300.0, 200.0);
                let l = viewer.layers[0].id;
                let outlines = viewer_outlines(&svg);
                assert!(outlines.iter().map(|b| b.elements().len()).sum::<usize>() > 200, "the viewer laid the text out");
                for bp in outlines {
                    let ap = vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
                    let n = Node::path(viewer.alloc_id(), PathData::from_bezpath(&bp), ap);
                    viewer.insert(Some(l), usize::MAX, n).unwrap();
                }
                let canvas = area_doc(runs(second(Color::BLACK)), 200.0, justify);
                assert_similar(&render_artboard(&canvas), &render_artboard(&viewer), 40.0, 0.01);
            }
        }
    }
}

#[test]
fn paragraphs_aligned_differently_keep_their_alignment() {
    let mut t = TextObject::point(Point::new(20.0, 20.0), "Left words here\nCentred line\nRight line", style(14.0));
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 160.0)) };
    let para = |j| vectorcraft_doc::ParaStyle { justify: j, ..Default::default() };
    t.set_paragraph_styles(vec![para(Justify::Left), para(Justify::Center), para(Justify::Right)]);
    let mut d = Document::new(300.0, 200.0);
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    for fewer_tspans in [false, true] {
        let svg = export(&d, &ExportOptions { fewer_tspans, ..Default::default() });
        // Each anchored line carries its own anchor (the <text> has none).
        assert!(svg.contains("text-anchor=\"middle\"") && svg.contains("text-anchor=\"end\""), "{svg}");
        assert!(!svg.contains("<text text-anchor"), "{svg}");
        // A viewer draws each line where the canvas does.
        let mut viewer = Document::new(300.0, 200.0);
        let vl = viewer.layers[0].id;
        for bp in viewer_outlines(&svg) {
            let ap = vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
            let n = Node::path(viewer.alloc_id(), PathData::from_bezpath(&bp), ap);
            viewer.insert(Some(vl), usize::MAX, n).unwrap();
        }
        assert_similar(&render_artboard(&d), &render_artboard(&viewer), 40.0, 0.01);
        // Read back: one paragraph per line, each with its alignment, drawn where it was.
        let back = vectorcraft_svg::import(&svg).unwrap();
        let text =
            back.layers[0].children().unwrap().iter().find_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.clone()) } else { None }).unwrap();
        let js: Vec<Justify> = text.paragraph_styles().iter().map(|p| p.justify).collect();
        assert_eq!(js, [Justify::Left, Justify::Center, Justify::Right], "{svg}");
        assert_similar(&render_artboard(&d), &render_artboard(&back), 40.0, 0.01);
    }
}
