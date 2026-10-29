//! Affinity → VectorCraft mapping of pages, master pages, tables, paragraph formats, text scale,
//! layer effects and embedded documents, on original synthetic documents built with the reader's
//! `synth` feature (no Affinity software or files were used). The fixtures follow DesignCraft's
//! Affinity import tests, whose reader VectorCraft shares.

use crate::affinity_docs;
use crate::text_fixture;

use std::sync::Arc;

use affinity_docs::*;

use kurbo::Rect;
use text_fixture::{array_f64, array_i32, attrs, block, run, utf8};
use vectorcraft_affinity::synth::{F, tag};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, StrokeAlign};
use vectorcraft_doc::text::{TextKind, TextObject, VerticalAlign};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_engine::cmd::fileio::{Loaded, load};

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn open(name: &str, bytes: &[u8]) -> Loaded {
    let loaded = load(name, bytes).unwrap();
    assert!(!loaded.preview_only, "{:?}", loaded.warnings);
    loaded
}

/// Every node of the document, depth first.
fn all(d: &Document) -> Vec<&Node> {
    fn walk<'a>(nodes: &'a [Arc<Node>], out: &mut Vec<&'a Node>) {
        for n in nodes {
            out.push(n);
            if let Some(c) = n.children() {
                walk(c, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&d.layers, &mut out);
    out
}

fn named<'a>(d: &'a Document, name: &str) -> &'a Node {
    all(d).into_iter().find(|n| n.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no node {name}"))
}

fn texts(d: &Document) -> Vec<&TextObject> {
    all(d).into_iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.as_ref()) } else { None }).collect()
}

fn content(t: &TextObject) -> String {
    t.runs.iter().map(|r| r.text.as_str()).collect()
}

/// An area text's frame in document space.
fn frame(t: &TextObject) -> Rect {
    let TextKind::Area { frame } = &t.kind else { panic!("area type: {:?}", t.kind) };
    t.xf.transform_rect_bbox(frame.bounds().unwrap())
}

fn rect_near(a: Rect, b: Rect) -> bool {
    near(a.x0, b.x0) && near(a.y0, b.y0) && near(a.x1, b.x1) && near(a.y1, b.y1)
}

fn layer_names(d: &Document) -> Vec<&str> {
    d.layers.iter().map(|l| l.name.as_deref().unwrap_or("")).collect()
}

#[test]
fn publisher_pages_become_artboards_in_points() {
    let d = open("brochure.afpub", &publisher()).doc;
    let boards: Vec<_> = d.artboards.iter().map(|b| (b.name.as_str(), b.rect)).collect();
    assert_eq!(boards.len(), 2);
    assert_eq!((boards[0].0, boards[1].0), ("Page 1", "Page 2"));
    assert!(rect_near(boards[0].1, Rect::new(0.0, 0.0, 2480.0 * PT, 3508.0 * PT)), "{boards:?}");
    assert!(rect_near(boards[1].1, Rect::new(2480.0 * PT, 0.0, 4960.0 * PT, 3508.0 * PT)), "{boards:?}");
    assert!(layer_names(&d).contains(&"Background"));
    let rect = named(&d, "Magenta box");
    assert!(rect_near(rect.geometric_bounds().unwrap(), Rect::new(100.0 * PT, 100.0 * PT, 200.0 * PT, 300.0 * PT)));
    let Some(AppearanceItem::Fill(fill)) = rect.appearance.items.first() else { panic!("{:?}", rect.appearance) };
    assert_eq!(fill.paint, Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)), "CMYK stays CMYK");
}

/// Several spreads of several pages: every page is an artboard, the spreads side by side.
#[test]
fn every_page_of_every_spread_is_an_artboard() {
    let file = publisher_with(vec![
        spread(2, 1, vec![rectangle(10, (10.0, 10.0), (20.0, 20.0))]),
        spread(3, 2, vec![rectangle(20, (2500.0, 10.0), (20.0, 20.0))]),
        spread(4, 2, vec![]),
    ]);
    let d = open("book.afpub", &file).doc;
    let names: Vec<_> = d.artboards.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Page 1", "Page 2", "Page 3", "Page 4", "Page 5"]);
    assert_eq!(d.artboards.iter().map(|b| b.id).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    let (w, h) = (2480.0 * PT, 3508.0 * PT);
    for b in &d.artboards {
        assert!(near(b.rect.width(), w) && near(b.rect.height(), h), "{b:?}");
    }
    // The two pages of a spread touch; spreads are 36 pt apart.
    let x: Vec<f64> = d.artboards.iter().map(|b| b.rect.x0).collect();
    assert!(near(x[1], w + 36.0) && near(x[2], 2.0 * w + 36.0) && near(x[3], 3.0 * w + 72.0), "{x:?}");
    // The second spread's box is on its second page.
    let boxes: Vec<Rect> = all(&d).into_iter().filter(|n| n.name.as_deref() == Some("Magenta box")).map(|n| n.geometric_bounds().unwrap()).collect();
    assert_eq!(boxes.len(), 2);
    assert!(d.artboards[2].rect.contains(boxes[1].center()), "{boxes:?} {:?}", d.artboards[2]);
    let reopened = vectorcraft_format::load_file(&vectorcraft_format::save_file(&d)).unwrap();
    assert_eq!(reopened.doc.artboards, d.artboards);
}

/// Artboards on one canvas keep their names, sizes, places and own content; objects beside them
/// stay on the canvas.
#[test]
fn several_artboards_keep_their_own_content() {
    let file = canvas_with(
        vec![
            board(10, "Icon", (0.0, 0.0), (64.0, 64.0), vec![named_rect(11, "Icon art", (8.0, 8.0), (48.0, 48.0))]),
            named_rect(20, "Loose", (80.0, 0.0), (10.0, 10.0)),
            board(30, "Banner", (100.0, 0.0), (300.0, 100.0), vec![named_rect(31, "Banner art", (110.0, 10.0), (280.0, 80.0))]),
            board(40, "Card", (0.0, 200.0), (85.0, 55.0), vec![]),
        ],
        None,
    );
    let d = open("set.af", &file).doc;
    let boards: Vec<_> = d.artboards.iter().map(|b| (b.name.as_str(), b.rect)).collect();
    assert_eq!(
        boards,
        [("Icon", Rect::new(0.0, 0.0, 64.0, 64.0)), ("Banner", Rect::new(100.0, 0.0, 400.0, 100.0)), ("Card", Rect::new(0.0, 200.0, 85.0, 255.0))]
    );
    // Each artboard is a layer holding its clip group, its art inside.
    for (board, art) in [("Icon", "Icon art"), ("Banner", "Banner art")] {
        let layer = d.layers.iter().find(|l| l.name.as_deref() == Some(board)).unwrap();
        let group = &layer.children().unwrap()[0];
        assert!(matches!(group.kind, NodeKind::Group { clip: true, .. }));
        assert!(group.children().unwrap().iter().any(|c| c.name.as_deref() == Some(art)), "{board}");
    }
    assert!(rect_near(named(&d, "Loose").geometric_bounds().unwrap(), Rect::new(80.0, 0.0, 90.0, 10.0)));
}

#[test]
fn artistic_text_keeps_its_paragraphs_size_and_leading() {
    let d = open("brochure.afpub", &publisher()).doc;
    let t = texts(&d)[0];
    assert_eq!(content(t), "Hello\nWorld", "paragraph marks become paragraphs; the final one goes");
    assert!(near(t.runs[0].style.size, 100.0 * PT));
    assert_eq!(t.runs[0].style.font_style, "Bold");
    assert_eq!(t.para.justify, vectorcraft_doc::text::Justify::Center);
    assert!(matches!(t.kind, TextKind::Point));
    // Centred on the anchor (x = 1000 + 200), first baseline 80 px below the frame top (y = 500).
    let origin = t.xf * kurbo::Point::ZERO;
    assert!(near(origin.x, 1200.0 * PT) && near(origin.y, 580.0 * PT), "{origin:?}");
    // Lines are the font size plus the font's line gap apart (VectorCraft's default is 120 %).
    let face = vectorcraft_text::FontDb::global().face(&t.runs[0].style.font_family, &t.runs[0].style.font_style).unwrap();
    let expected = 100.0 * (1.0 + face.line_gap() / face.units_per_em());
    assert!(near(t.para.justification.auto_leading, expected), "{} vs {expected}", t.para.justification.auto_leading);
}

#[test]
fn tables_become_cell_text_and_edge_lines() {
    // Stretched to twice its width by its transform: the grid follows, the type doesn't.
    let loaded = open("table.afpub", &page_with(table_node(30, [2.0, 0.0, 400.0, 0.0, 1.0, 600.0])));
    let d = &loaded.doc;
    let table = named(d, "Table");
    let cells: Vec<&TextObject> =
        table.children().unwrap().iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.as_ref()) } else { None }).collect();
    assert_eq!(cells.iter().map(|t| content(t)).collect::<Vec<_>>(), ["A", "B", "C", "D", "E"]);
    let a = cells[0];
    assert!(near(a.runs[0].style.size, 40.0 * PT), "type keeps its size");
    assert!(near(a.runs[0].style.h_scale, 100.0), "type isn't stretched with the grid");
    assert_eq!(a.area.vertical_align, VerticalAlign::Center);
    // Insets (5, 6, 7, 8 px) inside the cell, scaled with the grid; the text centred in them as
    // Affinity centres it.
    let f = frame(a);
    assert!(near(f.x0, (400.0 + 10.0) * PT) && near(f.width(), 176.0 * PT), "{f:?}");
    let (top, last) = cap_to_last_baseline(a);
    assert!(((top + last) / 2.0 - (600.0 + 24.0) * PT).abs() < 1e-6, "{top} {last}");
    // The merged cell spans two columns.
    assert!(near(frame(cells[3]).width(), (400.0 - 24.0) * PT), "{:?}", frame(cells[3]));
    // 8 vertical edges less the one inside the merged cell, and 9 horizontal ones.
    let lines: Vec<&Arc<Node>> = table.children().unwrap().iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).collect();
    assert_eq!(lines.len(), 16);
    let Some(AppearanceItem::Stroke(s)) = lines[0].appearance.items.first() else { panic!() };
    assert_eq!(s.paint, Paint::solid(Color::cmyk(1.0, 0.5, 0.0, 0.0)));
    assert!(s.width > 0.0);
}

/// Where a cell's text goes in the document: its first line's capitals' tops and its last
/// baseline (y).
fn cap_to_last_baseline(t: &TextObject) -> (f64, f64) {
    let db = vectorcraft_text::FontDb::global();
    // Hung from the frame's top by its cap height, the first baseline is that far down.
    let mut top = t.clone();
    top.area.vertical_align = VerticalAlign::Top;
    let cap = vectorcraft_text::layout(db, &top).lines[0].baseline;
    let lines = vectorcraft_text::layout(db, t).lines;
    let y = t.xf.translation().y;
    (y + lines[0].baseline - cap, y + lines.last().unwrap().baseline)
}

#[test]
fn cell_text_taller_than_its_cell_is_centred_on_it_and_shows() {
    // "A" over "a" (a line break inside the paragraph) in a cell 36 px tall inside its insets.
    let loaded = open("cells.afpub", &page_with(table_with(30, [1.0, 0.0, 400.0, 0.0, 1.0, 600.0], ["A\u{2028}a", "B", "C", "D", "E"])));
    assert!(!loaded.warnings.iter().any(|w| w.contains("don't fit")), "{:?}", loaded.warnings);
    let a = texts(&loaded.doc)[0];
    assert_eq!(content(a), "A\na");
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), a);
    assert_eq!(lay.lines.len(), 2, "both lines show");
    assert!(!lay.overflow);
    assert!(frame(a).height() > 36.0 * PT, "the frame grew: {:?}", frame(a));
    let (top, last) = cap_to_last_baseline(a);
    assert!(top < (600.0 + 6.0) * PT, "reaching out of the cell");
    assert!(((top + last) / 2.0 - (600.0 + 24.0) * PT).abs() < 1e-6, "centred: {top} {last}");
}

#[test]
fn line_breaks_inside_a_paragraph_get_no_paragraph_spacing() {
    // Doub slots: 4 first line, 5 space before, 6 after (see the test below).
    let attrs_para = F::Obj(tag(b"PAtt"), vec![(tag(b"Doub"), array_f64(b"Doub", &[1.0, 0.0, 0.0, 0.0, 25.0, 9.0, 17.0]))]);
    let blk = F::Obj(
        tag(b"StBl"),
        vec![
            (tag(b"Glyp"), utf8("One\u{2028}two\u{2029}Three\u{2029}")),
            (
                tag(b"GAtt"),
                F::Obj(
                    tag(b"GlAS"),
                    vec![(tag(b"Runs"), F::Objs(tag(b"GlAR"), vec![run(14, Some(attrs("Inter", "Inter-Regular", 400, false, 30.0)))]))],
                ),
            ),
            (
                tag(b"PAtt"),
                F::Obj(tag(b"PaAS"), vec![(tag(b"Runs"), F::Objs(tag(b"PaAR"), vec![vec![(tag(b"Indx"), F::I32(14)), (tag(b"Item"), attrs_para)]]))]),
            ),
        ],
    );
    let node = F::Def(
        40,
        vec![tag(b"TxtF")],
        vec![
            (tag(b"StSt"), F::Obj(tag(b"Stry"), vec![(tag(b"Blok"), F::Shared(vec![blk]))])),
            (tag(b"TxtH"), F::Obj(tag(b"FrFr"), vec![(tag(b"FrmB"), F::F64s(vec![0.0, 0.0, 500.0, 300.0]))])),
        ],
    );
    let t = texts(&open("breaks.afpub", &page_with(node)).doc)[0].clone();
    assert_eq!(content(&t), "One\ntwo\nThree");
    let p = &t.paras;
    let spacing: Vec<(f64, f64, f64)> = p.iter().map(|p| (p.space_before / PT, p.space_after / PT, p.first_line_indent / PT)).collect();
    let r = |v: (f64, f64, f64)| ((v.0 * 1e6).round() / 1e6, (v.1 * 1e6).round() / 1e6, (v.2 * 1e6).round() / 1e6);
    assert_eq!(
        spacing.into_iter().map(r).collect::<Vec<_>>(),
        [(9.0, 0.0, 25.0), (0.0, 17.0, 0.0), (0.0, 17.0, 25.0)],
        "space before the paragraph and after it, the first-line indent on its first line"
    );
}

#[test]
fn each_paragraph_keeps_its_alignment_indents_and_spacing() {
    // Doub slots: 1 leading (Ints slot 1 = 2), 2 left, 3 right, 4 first line, 5 space before, 6 after.
    let para = |end: i32, align: i32| {
        vec![
            (tag(b"Indx"), F::I32(end)),
            (
                tag(b"Item"),
                F::Obj(
                    tag(b"PAtt"),
                    vec![
                        (tag(b"Doub"), array_f64(b"Doub", &[1.0, 5.0, 40.0, 3.0, 25.0, 9.0, 17.0])),
                        (tag(b"Ints"), array_i32(b"Ints", &[align, 2])),
                    ],
                ),
            ),
        ]
    };
    let blk = F::Obj(
        tag(b"StBl"),
        vec![
            (tag(b"Glyp"), utf8("One\u{2029}Two\u{2029}")),
            (
                tag(b"GAtt"),
                F::Obj(
                    tag(b"GlAS"),
                    vec![(tag(b"Runs"), F::Objs(tag(b"GlAR"), vec![run(8, Some(auto_leading(attrs("Inter", "Inter-Regular", 400, false, 30.0))))]))],
                ),
            ),
            (tag(b"PAtt"), F::Obj(tag(b"PaAS"), vec![(tag(b"Runs"), F::Objs(tag(b"PaAR"), vec![para(4, 0), para(8, 2)]))])),
        ],
    );
    let node = F::Def(
        40,
        vec![tag(b"TxtF")],
        vec![
            (tag(b"StSt"), F::Obj(tag(b"Stry"), vec![(tag(b"Blok"), F::Shared(vec![blk]))])),
            (tag(b"TxtH"), F::Obj(tag(b"FrFr"), vec![(tag(b"FrmB"), F::F64s(vec![0.0, 0.0, 500.0, 300.0]))])),
        ],
    );
    let loaded = open("paragraphs.afpub", &page_with(node));
    assert!(!loaded.warnings.iter().any(|w| w.contains("paragraph")), "{:?}", loaded.warnings);
    let t = texts(&loaded.doc)[0];
    assert_eq!(content(t), "One\nTwo");
    assert_eq!(t.paras.len(), 2, "{:?}", t.paras);
    let p = &t.paras;
    use vectorcraft_doc::text::Justify;
    assert_eq!((p[0].justify, p[1].justify), (Justify::Left, Justify::Right));
    assert!(near(p[0].left_indent, 40.0 * PT) && near(p[0].first_line_indent, -15.0 * PT) && near(p[0].right_indent, 3.0 * PT), "{:?}", p[0]);
    assert!(near(p[0].space_before, 9.0 * PT) && near(p[0].space_after, 17.0 * PT));
    // The larger of 17 after "One" and 9 before "Two": nothing more before "Two".
    assert!(near(p[1].space_before, 0.0) && near(p[1].space_after, 17.0 * PT));
    assert!(t.runs.iter().all(|r| r.style.leading.is_some_and(|l| near(l, 5.0 * PT))), "the paragraph's fixed leading");
}

#[test]
fn frame_type_follows_the_frame_text_scale_not_the_frame_transform() {
    // The same frame scaled 3× by its transform, without and with a text scale (FTxS) of 3.
    let frame_node = |ftxs: bool| {
        let story = block(utf8("Hi"), vec![run(2, Some(attrs("Inter", "Inter-Regular", 400, false, 10.0)))], &[]);
        let mut fields = vec![
            (tag(b"StSt"), F::Obj(tag(b"Stry"), vec![(tag(b"Blok"), F::Shared(vec![story]))])),
            (tag(b"TxtH"), F::Obj(tag(b"FrFr"), vec![(tag(b"FrmB"), F::F64s(vec![0.0, 0.0, 100.0, 50.0]))])),
            (tag(b"Xfrm"), F::F64s(vec![3.0, 0.0, 0.0, 0.0, 3.0, 0.0])),
        ];
        if ftxs {
            fields.push((tag(b"FTxS"), F::F64s(vec![3.0, 0.0, 0.0, 0.0, 3.0, 0.0])));
        }
        F::Def(40, vec![tag(b"TxtF")], fields)
    };
    for (ftxs, size) in [(false, 10.0), (true, 30.0)] {
        let d = open("frame.afpub", &page_with(frame_node(ftxs))).doc;
        let t = texts(&d)[0];
        assert!(near(frame(t).width(), 300.0 * PT), "the frame scales either way: {:?}", frame(t));
        assert!(near(t.runs[0].style.size, size * PT), "FTxS {ftxs}: {}", t.runs[0].style.size);
        assert!(near(t.xf.determinant(), 1.0), "the type isn't scaled by the placement");
    }
}

#[test]
fn embedded_affinity_documents_come_in_as_their_content() {
    let inner = canvas_with(vec![rectangle(10, (0.0, 0.0), (100.0, 50.0))], None);
    let outer = canvas_with(vec![embedded_node(20)], Some(&inner));
    let loaded = open("logo.af", &outer);
    let rect = named(&loaded.doc, "Magenta box");
    // Content bounds (0,0)-(100,50), centre (50,25) → origin, then 2× at (500,300).
    assert!(rect_near(rect.geometric_bounds().unwrap(), Rect::new(400.0, 250.0, 600.0, 350.0)), "{:?}", rect.geometric_bounds());
    assert!(!loaded.warnings.iter().any(|w| w.contains("cached picture")), "{:?}", loaded.warnings);
}

/// An embedded document's artboard frames its content but is no artboard of the document it is in.
#[test]
fn artboards_of_embedded_documents_are_not_artboards_here() {
    let inner = canvas_with(vec![board(10, "Logo board", (0.0, 0.0), (100.0, 50.0), vec![rectangle(11, (10.0, 10.0), (80.0, 30.0))])], None);
    let outer = canvas_with(vec![board(30, "Poster", (0.0, 0.0), (1000.0, 800.0), vec![embedded_node(20)])], Some(&inner));
    let d = open("poster.af", &outer).doc;
    assert_eq!(d.artboards.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Poster"]);
    // The logo still comes in, clipped by its own board.
    let logo = named(&d, "Logo board");
    assert!(matches!(logo.kind, NodeKind::Group { clip: true, .. }));
    assert!(rect_near(named(&d, "Magenta box").geometric_bounds().unwrap(), Rect::new(420.0, 270.0, 580.0, 330.0)));
}

#[test]
fn embedded_documents_nest_only_so_deep() {
    let mut doc = canvas_with(vec![rectangle(10, (0.0, 0.0), (100.0, 50.0))], None);
    for _ in 0..6 {
        doc = canvas_with(vec![embedded_node(20)], Some(&doc));
    }
    let loaded = open("deep.af", &doc);
    assert!(loaded.warnings.iter().any(|w| w.contains("nested more than four deep")), "{:?}", loaded.warnings);
}

/// An embedded document's master pages aren't kept, so its instances of them are left out: in the
/// document it is embedded in they would show that document's masters.
#[test]
fn master_pages_of_embedded_documents_are_left_out() {
    use vectorcraft_affinity::model::{Kind, Node as AfNode};
    fn instances(nodes: &[AfNode]) -> usize {
        nodes.iter().map(|n| usize::from(matches!(n.kind, Kind::MasterInstance { .. })) + instances(&n.children)).sum()
    }
    let outer = canvas_with(vec![embedded_node(20)], Some(&with_master(1)));
    let d = vectorcraft_affinity::read(&outer, vectorcraft_affinity::Limits::default()).unwrap();
    assert_eq!(instances(&d.spreads[0].nodes), 0);
    assert!(d.warnings.iter().any(|w| w.contains("master pages of embedded documents")), "{:?}", d.warnings);
    // Its own objects still come in.
    let loaded = open("poster.af", &outer);
    named(&loaded.doc, "Page box");
}

/// A dash pattern has a few lengths; a crafted one of thousands would be copied with every stroke.
#[test]
fn dash_patterns_of_more_than_64_lengths_import_as_solid_lines() {
    use vectorcraft_affinity::model::Kind;
    for (lengths, dashed) in [(64, true), (65, false)] {
        let dash: Vec<f64> = (0..lengths).map(|i| 1.0 + f64::from(i)).collect();
        let d = vectorcraft_affinity::read(&canvas_with(vec![stroked(10, 2, &dash)], None), vectorcraft_affinity::Limits::default()).unwrap();
        let Kind::Shape { strokes, .. } = &d.spreads[0].nodes[0].kind else { panic!("{:?}", d.spreads[0].nodes[0].kind) };
        assert_eq!(strokes[0].dash.as_ref().map(|(d, _)| d.len()), dashed.then_some(64));
        assert_eq!(d.warnings.iter().any(|w| w.contains("dash patterns of more than 64")), !dashed, "{:?}", d.warnings);
    }
}

#[test]
fn master_pages_show_on_the_pages_that_use_them() {
    let loaded = open("master.afpub", &with_master(1));
    let d = &loaded.doc;
    assert!(!loaded.warnings.iter().any(|w| w.contains("master")), "{:?}", loaded.warnings);
    assert_eq!(d.artboards.len(), 1, "a master spread is no artboard");
    // Behind the page's own objects, on a locked layer of its own.
    let master = &d.layers[0];
    assert_eq!(master.name.as_deref(), Some("Master A"));
    assert!(master.locked);
    // Only the second master page's box, moved onto the page.
    assert!(all(d).iter().all(|n| n.name.as_deref() != Some("Left master box")));
    let b = named(d, "Right master box").geometric_bounds().unwrap();
    assert!(rect_near(b, Rect::new(10.0, 10.0, 40.0, 50.0)), "{b:?}");
    assert!(named(d, "Page box").geometric_bounds().is_some());
}

#[test]
fn a_master_page_shows_on_every_page_it_covers() {
    // Two pages show master pages 2 and 3 (the master spread has two: the last repeats).
    let d = open("master2.afpub", &with_master(2)).doc;
    assert_eq!(d.artboards.len(), 2);
    let boxes: Vec<Rect> =
        all(&d).into_iter().filter(|n| n.name.as_deref() == Some("Right master box")).map(|n| n.geometric_bounds().unwrap()).collect();
    assert_eq!(boxes.len(), 2, "{boxes:?}");
    for (b, board) in boxes.iter().zip(&d.artboards) {
        assert!(board.rect.contains(b.center()), "{b:?} on {:?}", board.rect);
    }
}

fn pixel(d: &Document, x: usize, y: usize) -> [u8; 4] {
    let img = vectorcraft_render::Renderer::new().render_region(d, d.artboards[0].rect, 1.0, false);
    img.to_straight()[(y * img.width as usize + x) * 4..][..4].try_into().unwrap()
}

#[test]
fn shadows_go_where_affinity_puts_them() {
    // A blue shadow 10 px straight below (angle π/2), blurred 2 px, at half opacity.
    let shadow = F::Obj(
        tag(b"Shad"),
        vec![
            (tag(b"Enab"), F::Bool(true)),
            (tag(b"Opac"), F::F64(0.5)),
            (tag(b"Radi"), F::F64(2.0)),
            (tag(b"Offs"), F::F64(10.0)),
            (tag(b"Angl"), F::F64(std::f64::consts::FRAC_PI_2)),
            (tag(b"Colr"), rgba(0.0, 0.0, 1.0)),
        ],
    );
    let bevel = F::Obj(tag(b"BevE"), vec![(tag(b"Enab"), F::Bool(true))]);
    let loaded = open("shadow.af", &canvas_with(vec![with_effects(rectangle(10, (0.0, 0.0), (50.0, 50.0)), vec![shadow, bevel])], None));
    let d = &loaded.doc;
    let rect = named(d, "Magenta box");
    let fx = &rect.appearance.effects;
    assert_eq!(fx.len(), 1, "{fx:?}");
    assert_eq!(fx[0].id, "stylize.dropShadow");
    let p = &fx[0].params;
    assert!(near(p["x"].as_f64().unwrap(), 0.0) && near(p["y"].as_f64().unwrap(), 10.0), "{p}");
    assert_eq!((p["blur"].as_f64(), p["opacity"].as_f64()), (Some(2.0), Some(50.0)));
    // A bevel without a radius draws nothing, in Affinity as here.
    assert!(!loaded.warnings.iter().any(|w| w.contains("bevel")), "{:?}", loaded.warnings);
    // Drawn: blue at half strength below the box, the box itself magenta.
    let [r, g, b, a] = pixel(d, 25, 57);
    assert!(r < 10 && g < 10 && b > 245 && (100..160).contains(&a), "{:?}", [r, g, b, a]);
    assert_eq!(pixel(d, 25, 25)[3], 255);
}

#[test]
fn glows_inner_shadows_and_bevels_are_kept_or_reported() {
    let effect = |class: &[u8; 4], radius: f64| {
        F::Obj(
            tag(class),
            vec![(tag(b"Enab"), F::Bool(true)), (tag(b"Opac"), F::F64(0.8)), (tag(b"Radi"), F::F64(radius)), (tag(b"Colr"), rgba(1.0, 1.0, 0.0))],
        )
    };
    let node = with_effects(rectangle(10, (0.0, 0.0), (50.0, 50.0)), vec![effect(b"OutG", 6.0), effect(b"InnS", 3.0), effect(b"BevE", 4.0)]);
    let loaded = open("glow.af", &canvas_with(vec![node], None));
    let fx = &named(&loaded.doc, "Magenta box").appearance.effects;
    assert_eq!(fx.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.outerGlow", "stylize.innerGlow"]);
    assert_eq!((fx[0].params["blur"].as_f64(), fx[0].params["opacity"].as_f64()), (Some(6.0), Some(80.0)));
    for w in ["inner shadows (imported as inner glows", "bevel and emboss effects (left out)"] {
        assert!(loaded.warnings.iter().any(|x| x.contains(w)), "{w}: {:?}", loaded.warnings);
    }
}

#[test]
fn outline_effects_become_strokes() {
    let outline = |align: u16| {
        F::Obj(
            tag(b"Strk"),
            vec![(tag(b"Enab"), F::Bool(true)), (tag(b"Radi"), F::F64(3.0)), (tag(b"Alig"), F::Enum(align, 0)), (tag(b"Colr"), rgba(1.0, 0.0, 0.0))],
        )
    };
    let shape = with_effects(rectangle(10, (0.0, 0.0), (50.0, 50.0)), vec![outline(0)]);
    let text = with_effects(artistic_text(20, "Hi", 40.0), vec![outline(2)]);
    let d = open("outline.af", &canvas_with(vec![shape, text], None)).doc;
    let rect = named(&d, "Magenta box");
    let Some(AppearanceItem::Stroke(s)) = rect.appearance.items.iter().find(|i| matches!(i, AppearanceItem::Stroke(_))) else { panic!() };
    assert_eq!((s.width, s.align), (3.0, StrokeAlign::Outside));
    assert_eq!(s.paint, Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let t = texts(&d)[0];
    assert_eq!((t.runs[0].style.stroke.clone(), t.runs[0].style.stroke_width), (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 3.0));
}

#[test]
fn colour_overlays_are_baked_into_the_paint() {
    let overlay = F::Obj(tag(b"ColO"), vec![(tag(b"Enab"), F::Bool(true)), (tag(b"Opac"), F::F64(1.0)), (tag(b"Colr"), rgba(0.0, 1.0, 0.0))]);
    let d = open("overlay.af", &canvas_with(vec![with_effects(rectangle(10, (0.0, 0.0), (50.0, 50.0)), vec![overlay])], None)).doc;
    let rect = named(&d, "Magenta box");
    assert!(rect.appearance.effects.is_empty());
    let [r, g, b, a] = pixel(&d, 25, 25);
    assert!(r < 5 && g > 250 && b < 5 && a == 255, "{:?}", [r, g, b, a]);
}

#[test]
fn outline_effects_on_stroked_shapes_go_around_their_stroke() {
    let outline = |align: u16, width: f64| {
        F::Obj(
            tag(b"Strk"),
            vec![
                (tag(b"Enab"), F::Bool(true)),
                (tag(b"Radi"), F::F64(width)),
                (tag(b"Alig"), F::Enum(align, 0)),
                (tag(b"Colr"), rgba(1.0, 0.0, 0.0)),
            ],
        )
    };
    // A black stroke 2 px wide, centred on the path: it reaches 1 px out of the shape.
    let stroked = |id: u32| {
        let mut node = rectangle(id, (0.0, 0.0), (50.0, 50.0));
        if let F::Def(_, _, fields) = &mut node {
            fields.push((tag(b"PFil"), cmyk(0.0, 0.0, 0.0, 1.0)));
            fields.push((tag(b"LSty"), F::Obj(tag(b"LSty"), vec![(tag(b"Wght"), F::F64(2.0))])));
        }
        node
    };
    let loaded = open("outline.af", &canvas_with(vec![with_effects(stroked(10), vec![outline(2, 4.0), outline(0, 3.0)])], None));
    assert!(!loaded.warnings.iter().any(|w| w.contains("outline")), "{:?}", loaded.warnings);
    let strokes: Vec<(f64, StrokeAlign, Paint)> = named(&loaded.doc, "Magenta box")
        .appearance
        .items
        .iter()
        .filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some((s.width, s.align, s.paint.clone())) } else { None })
        .collect();
    let (red, black) = (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    assert_eq!(
        strokes,
        [
            // Outside: from the path to 3 px past the stroke, under it.
            (4.0, StrokeAlign::Outside, red.clone()),
            (2.0, StrokeAlign::Center, black),
            // Centred on the stroke's outer edge, over it (as far in as out).
            (6.0, StrokeAlign::Center, red),
        ]
    );
}

#[test]
fn artistic_text_aligns_its_lines_within_the_longest() {
    // (first, second alignment) → where "Mid" goes in the span of "Wide line" (0 its left end,
    // 0.5 centred, 1 its right end).
    for (first, second, at) in [(0, 1, 0.5), (0, 2, 1.0), (1, 0, 0.0), (2, 1, 0.5), (1, 2, 1.0)] {
        let d = open("mixed.af", &canvas_with(vec![mixed_alignment(first, second)], None)).doc;
        let t = texts(&d)[0];
        assert!(matches!(t.kind, TextKind::Point));
        let lines = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).lines;
        let (wide, mid) = (&lines[0], &lines[1]);
        let w = wide.x1 - wide.x0;
        assert!(w > 0.0);
        // The block stays where the first paragraph's alignment puts it on the anchor.
        let start = [0.0, -0.5, -1.0][first as usize] * w;
        assert!((wide.x0 - start).abs() < 1e-6, "{first}/{second}: {} vs {start}", wide.x0);
        let expected = wide.x0 + (w - (mid.x1 - mid.x0)) * at;
        assert!((mid.x0 - expected).abs() < 1e-6, "{first}/{second}: {} vs {expected}", mid.x0);
        // Each paragraph keeps its own alignment.
        use vectorcraft_doc::text::Justify;
        let j = |a: i32| [Justify::Left, Justify::Center, Justify::Right][a as usize];
        assert_eq!((t.paras[0].justify, t.paras[1].justify), (j(first), j(second)));
    }
    // One alignment throughout: no indents.
    let d = open("same.af", &canvas_with(vec![mixed_alignment(1, 1)], None)).doc;
    assert!(texts(&d)[0].paras.iter().all(|p| p.left_indent == 0.0 && p.right_indent == 0.0));
}

#[test]
fn frame_text_hangs_from_cap_height_and_shows_lines_whose_baseline_fits() {
    use vectorcraft_doc::text::FirstBaseline;
    let db = vectorcraft_text::FontDb::global();
    // Room for both lines: where they go.
    let loaded = open("tall.af", &canvas_with(vec![two_line_frame(1000.0)], None));
    let t = texts(&loaded.doc)[0];
    assert_eq!(t.area.first_baseline, FirstBaseline::CapHeight);
    let lines = vectorcraft_text::layout(db, t).lines;
    assert_eq!(lines.len(), 2);
    // Higher than VectorCraft's default, the font's ascent.
    let ascent = vectorcraft_text::layout(db, &TextObject { area: Default::default(), ..t.clone() }).lines[0].baseline;
    assert!(lines[0].baseline < ascent - 1.0, "first baseline {} vs ascent {ascent}", lines[0].baseline);
    let (baseline, bottom) = (lines[1].baseline, lines[1].baseline + lines[1].descent);
    // A frame the second line's baseline fits in but not its descenders: Affinity shows the line.
    let height = (baseline + bottom) / 2.0;
    let loaded = open("short.af", &canvas_with(vec![two_line_frame(height)], None));
    assert!(loaded.warnings.iter().any(|w| w.contains("made taller so the lines show")), "{:?}", loaded.warnings);
    let t = texts(&loaded.doc)[0];
    let lay = vectorcraft_text::layout(db, t);
    assert_eq!(lay.lines.len(), 2);
    assert!(!lay.overflow);
    assert!(frame(t).height() > height && frame(t).height() < bottom + 1.0, "{:?}", frame(t));
    // One whose second baseline is outside it too: that line stays out, as in Affinity.
    let loaded = open("shorter.af", &canvas_with(vec![two_line_frame(baseline - 5.0)], None));
    let t = texts(&loaded.doc)[0];
    assert_eq!(vectorcraft_text::layout(db, t).lines.len(), 1);
    assert!((frame(t).height() - (baseline - 5.0)).abs() < 1e-6);
}

#[test]
fn underlined_and_struck_through_characters_stay_so() {
    let decorated = |ints: &[i32]| {
        let mut a = attrs("Inter", "Inter-Regular", 400, false, 40.0);
        if let F::Obj(_, fields) = &mut a {
            for (t, v) in fields.iter_mut() {
                if *t == tag(b"Ints") {
                    *v = array_i32(b"Ints", ints);
                }
            }
        }
        a
    };
    let blk = block(
        utf8("Plain under struck"),
        vec![
            run(6, Some(attrs("Inter", "Inter-Regular", 400, false, 40.0))),
            run(12, Some(decorated(&[1, 0, 0, 0, 1]))),
            run(18, Some(decorated(&[0, 1, 0, 0, 1]))),
        ],
        &[0],
    );
    let node = F::Def(
        70,
        vec![tag(b"TxtA")],
        vec![
            (tag(b"StSt"), F::Obj(tag(b"Stry"), vec![(tag(b"Blok"), F::Shared(vec![blk]))])),
            (tag(b"TxtH"), F::Obj(tag(b"ArFr"), vec![(tag(b"FrmB"), F::F64s(vec![0.0, 0.0, 400.0, 100.0])), (tag(b"ArtV"), F::F64(80.0))])),
        ],
    );
    let loaded = open("underline.af", &canvas_with(vec![node], None));
    assert!(!loaded.warnings.iter().any(|w| w.contains("decoration")), "{:?}", loaded.warnings);
    let t = texts(&loaded.doc)[0];
    assert_eq!(
        t.runs.iter().map(|r| (r.text.as_str(), r.style.underline, r.style.strikethrough)).collect::<Vec<_>>(),
        [("Plain ", false, false), ("under ", true, false), ("struck", false, true)]
    );
}

/// The image of the node named "Photo", and the blob it shows.
fn photo(d: &Document) -> &vectorcraft_doc::ImageBlob {
    let NodeKind::Image(img) = &named(d, "Photo").kind else { panic!("an image") };
    &d.images[&img.key]
}

#[test]
fn cmyk_pixel_layers_show_through_the_documents_cmyk_profile() {
    use vectorcraft_color::cms::{self, IccProfile, Intent};
    // A CMYK profile written from a built-in space, embedded as the document's.
    let icc = cms::icc_bytes(cms::GENERIC_CMYK).unwrap();
    let loaded = open("photo.af", &cmyk_layer(false, Some(&icc)));
    let blob = photo(&loaded.doc);
    assert!(!blob.is_cmyk());
    let px = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
    let profile = IccProfile::from_bytes(None, &icc).unwrap();
    let srgb = cms::icc::builtin_rgb(cms::SRGB).unwrap();
    let want = profile.to_rgb_of(&srgb, &[1.0, 0.0, 0.0, 0.0], Intent::RelativeColorimetric, true).unwrap().map(|v| (v * 255.0).round() as i32);
    let got = px.get_pixel(8, 8).0;
    assert!((0..3).all(|i| (i32::from(got[i]) - want[i]).abs() <= 2) && got[3] == 255, "{got:?} vs {want:?}");
    // Not the profile-free formula's pure cyan.
    assert_ne!(&got[..3], &[0, 255, 255]);
    assert!(!loaded.warnings.iter().any(|w| w.contains("CMYK")), "{:?}", loaded.warnings);
}

#[test]
fn cmyk_pixel_layers_without_a_profile_keep_their_inks_or_use_the_working_profile() {
    // Opaque: a CMYK image with the layer's inks.
    let loaded = open("photo.af", &cmyk_layer(false, None));
    let inks = photo(&loaded.doc).cmyk().expect("a CMYK image");
    assert_eq!((inks.width, inks.height, &inks.data[..4]), (16, 16, &[255, 0, 0, 0][..]));
    // With transparency: RGBA through the working CMYK profile, and said.
    let loaded = open("photo.af", &cmyk_layer(true, None));
    let blob = photo(&loaded.doc);
    assert!(!blob.is_cmyk());
    assert_eq!(image::load_from_memory(&blob.bytes).unwrap().to_rgba8().get_pixel(8, 8).0[3], 0);
    assert!(loaded.warnings.iter().any(|w| w.contains("CMYK pixel layers with transparency")), "{:?}", loaded.warnings);
}

#[test]
fn objects_beyond_the_document_range_are_left_out() {
    // 5 million points from the origin, past `MAX_COORD`.
    let far = named_rect(11, "Far box", (5.0e6, 0.0), (10.0, 10.0));
    let loaded = open("far.af", &canvas_with(vec![rectangle(10, (0.0, 0.0), (50.0, 50.0)), far], None));
    let names: Vec<_> = all(&loaded.doc).iter().filter_map(|n| n.name.clone()).collect();
    assert!(names.iter().any(|n| n == "Magenta box") && !names.iter().any(|n| n == "Far box"), "{names:?}");
    assert!(loaded.warnings.iter().any(|w| w.contains("beyond the document range")), "{:?}", loaded.warnings);
}
