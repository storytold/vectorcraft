//! Optional content groups become layers: name, visibility (the default configuration's or the
//! view state's), print state, lock and nesting; art that is off comes in as a hidden layer, and
//! art past what is read of a huge page never shows or prints a hidden layer.

use vectorcraft_color::Color;
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};

use crate::tests_import_fidelity::stream;
use crate::*;

/// Groups "Shapes" and "Hidden" (not printed) as the first extra objects of a file of `pages`
/// pages, the catalog entries listing them with `config` (where `{A}` and `{B}` stand for them)
/// as the default configuration, and their object numbers.
fn groups(pages: usize, config: &str) -> ([String; 2], String, [usize; 2]) {
    let (a, b) = (first_extra(pages), first_extra(pages) + 1);
    let objs = [
        "<< /Type /OCG /Name (Shapes) >>".to_string(),
        "<< /Type /OCG /Name <FEFF00480069006400640065006E> /Usage << /Print << /PrintState /OFF >> >> >>".to_string(),
    ];
    let catalog = format!(
        "/OCProperties << /OCGs [{a} 0 R {b} 0 R] /D << /Order [{a} 0 R {b} 0 R] {} >> >> ",
        config.replace("{A}", &format!("{a} 0 R")).replace("{B}", &format!("{b} 0 R"))
    );
    (objs, catalog, [a, b])
}

/// A page drawing red in "Shapes", blue in "Hidden" and green outside any group.
fn page(ids: [usize; 2]) -> PdfPage {
    PdfPage {
        resources: format!("/Properties << /MC0 {} 0 R /MC1 {} 0 R >>", ids[0], ids[1]),
        ..PdfPage::new(100.0, 100.0, "/OC /MC0 BDC 1 0 0 rg 10 10 30 30 re f EMC /OC /MC1 BDC 0 0 1 rg 50 50 30 30 re f EMC 0 1 0 rg 0 0 5 5 re f")
    }
}

fn file(config: &str) -> Vec<u8> {
    let (objs, catalog, ids) = groups(1, config);
    pdf_with_catalog(&[page(ids)], &[&objs[0], &objs[1]], &catalog, None)
}

fn names(d: &Document) -> Vec<String> {
    d.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect()
}

/// The fill colours of a layer's art.
fn colors(l: &Node) -> Vec<Color> {
    l.children().unwrap().iter().filter_map(|n| n.appearance.fill().and_then(|f| f.paint.color())).collect()
}

fn printable(l: &Node) -> bool {
    matches!(l.kind, NodeKind::Layer { printable: true, .. })
}

#[test]
fn optional_content_groups_become_layers_and_off_art_a_hidden_layer() {
    let d = import(&file("/OFF [{B}]")).unwrap();
    // In paint order (bottom first): the groups, then the art outside them.
    assert_eq!(names(&d), ["Shapes", "Hidden", "Page 1"]);
    let [shapes, hidden, rest] = [0, 1, 2].map(|i| d.layers[i].clone());
    assert!(shapes.visible && printable(&shapes) && !shapes.locked);
    assert_eq!(colors(&shapes), [Color::rgb(1.0, 0.0, 0.0)]);
    // The group that is off is a hidden layer that still has its art; it doesn't print.
    assert!(!hidden.visible && !printable(&hidden));
    assert_eq!(colors(&hidden), [Color::rgb(0.0, 0.0, 1.0)]);
    assert_eq!(colors(&rest), [Color::rgb(0.0, 1.0, 0.0)]);
}

#[test]
fn base_state_on_lists_and_locks_are_read() {
    let d = import(&file("/BaseState /OFF /ON [{A}] /Locked [{A}]")).unwrap();
    assert_eq!(names(&d), ["Shapes", "Hidden", "Page 1"]);
    assert!(d.layers[0].visible && d.layers[0].locked);
    assert!(!d.layers[1].visible && !d.layers[1].locked);
}

#[test]
fn without_layers_each_page_is_one_layer_of_what_shows() {
    let r = import_with_report(&file("/OFF [{B}]"), &ImportOptions { layers: false, ..Default::default() }).unwrap();
    assert_eq!(names(&r.document), ["Page 1"]);
    assert_eq!(colors(&r.document.layers[0]), [Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 1.0, 0.0)]);
}

#[test]
fn pages_share_the_layers_of_their_groups() {
    let (objs, catalog, ids) = groups(2, "/OFF [{B}]");
    let bytes = pdf_with_catalog(&[page(ids), page(ids)], &[&objs[0], &objs[1]], &catalog, None);
    let d = import(&bytes).unwrap();
    assert_eq!(names(&d), ["Shapes", "Hidden", "Page 1", "Page 2"]);
    assert_eq!(colors(&d.layers[0]).len(), 2, "both pages' art");
    assert_eq!(colors(&d.layers[1]).len(), 2);
    // The second page's art sits on its artboard.
    let x = d.layers[0].children().unwrap()[1].geometric_bounds().unwrap().x0;
    assert!(x > d.artboards[1].rect.x0, "{x}");
}

#[test]
fn groups_marked_inside_forms_and_clips_reach_their_layers() {
    let (objs, catalog, ids) = groups(1, "/OFF [{B}]");
    let form_no = first_extra(1) + 2;
    let form = stream(
        &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Properties << /MC1 {} 0 R >> >>", ids[1]),
        "/OC /MC1 BDC 0 0 1 rg 50 50 30 30 re f EMC",
    );
    let page = PdfPage {
        resources: format!("/Properties << /MC0 {} 0 R >> /XObject << /X0 {form_no} 0 R >>", ids[0]),
        ..PdfPage::new(100.0, 100.0, "q 0 0 25 25 re W n /OC /MC0 BDC 1 0 0 rg 10 10 30 30 re f EMC Q /X0 Do")
    };
    let d = import(&pdf_with_catalog(&[page], &[&objs[0], &objs[1], &form], &catalog, None)).unwrap();
    assert_eq!(names(&d), ["Shapes", "Hidden"]);
    let shapes = &d.layers[0].children().unwrap()[0];
    assert!(matches!(shapes.kind, NodeKind::Group { clip: true, .. }), "the clip group goes with its art");
    assert!(!d.layers[1].visible);
    assert_eq!(colors(&d.layers[1]), [Color::rgb(0.0, 0.0, 1.0)]);
}

#[test]
fn a_form_hidden_by_its_own_group_stays_hidden() {
    let (objs, catalog, ids) = groups(1, "/OFF [{B}]");
    let form_no = first_extra(1) + 2;
    let form = stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /OC {} 0 R", ids[1]), "0 0 1 rg 50 50 30 30 re f");
    let page = PdfPage { resources: format!("/XObject << /X0 {form_no} 0 R >>"), ..PdfPage::new(100.0, 100.0, "1 0 0 rg 10 10 30 30 re f /X0 Do") };
    let r = import_with_report(&pdf_with_catalog(&[page], &[&objs[0], &objs[1], &form], &catalog, None), &ImportOptions::default()).unwrap();
    assert_eq!(names(&r.document), ["Page 1"]);
    assert_eq!(colors(&r.document.layers[0]), [Color::rgb(1.0, 0.0, 0.0)], "the hidden form isn't shown");
    assert!(r.warnings.iter().any(|w| w.contains("hidden layers")), "{:?}", r.warnings);
}

#[test]
fn an_encrypted_file_keeps_its_hidden_layers() {
    let (objs, catalog, ids) = groups(1, "/OFF [{B}]");
    let bytes = pdf_with_catalog(&[page(ids)], &[&objs[0], &objs[1]], &catalog, Some("pw"));
    let r = import_with_report(&bytes, &ImportOptions { password: Some("pw".into()), ..Default::default() }).unwrap();
    let d = r.document;
    assert_eq!(d.layers.len(), 3, "{:?}", r.warnings);
    assert!(d.layers[0].visible && !d.layers[1].visible);
    assert_eq!(colors(&d.layers[1]), [Color::rgb(0.0, 0.0, 1.0)]);
}

/// A page drawing `before`, white and red squares in "Shapes", then `after`; `ai`: with an
/// editor's private data (a PDF-compatible `.ai`).
fn page_fill(before: &str, after: &str, ai: bool) -> Document {
    let (objs, catalog, ids) = groups(1, "");
    let page = PdfPage {
        resources: format!(
            "/Properties << /MC0 {} 0 R >> /ExtGState << /GS0 << /ca 0.5 >> >> /ColorSpace << /W [/Separation /White /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 0 0] /N 1 >>] >>",
            ids[0]
        ),
        entries: if ai { "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into() } else { String::new() },
        ..PdfPage::new(200.0, 100.0, &format!("{before} /OC /MC0 BDC 1 1 1 rg 20 20 60 60 re f 1 0 0 rg 120 20 60 60 re f EMC {after}"))
    };
    import(&pdf_with_catalog(&[page], &[&objs[0], &objs[1]], &catalog, None)).unwrap()
}

/// A file with an editor's private data says that art off its artboards isn't in the part read
/// (#472); a plain PDF has nothing to say.
#[test]
fn an_ai_file_notes_that_its_art_off_the_artboards_is_not_read() {
    for ai in [false, true] {
        let page = PdfPage {
            entries: if ai { "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into() } else { String::new() },
            ..PdfPage::new(100.0, 100.0, "1 0 0 rg 10 10 30 30 re f")
        };
        let r = import_with_report(&pdf_with_catalog(&[page], &[], "", None), &ImportOptions::default()).unwrap();
        assert_eq!(r.warnings, if ai { vec![crate::import::OFF_ARTBOARD_NOTE.to_string()] } else { vec![] });
    }
}

fn path_count(d: &Document) -> usize {
    let mut n = 0;
    d.walk(|c| n += usize::from(matches!(c.kind, NodeKind::Path { .. })));
    n
}

#[test]
fn an_ai_files_white_page_outside_its_layers_is_not_art() {
    // RGB, grey and CMYK white, the rectangle drawn either way round.
    for fill in ["1 1 1 rg 0 0 200 100 re f", "1 g 0 100 200 -100 re f", "0 0 0 0 k 0 0 200 100 re f"] {
        let d = page_fill(fill, "", true);
        assert_eq!(names(&d), ["Shapes"], "{fill}");
        assert_eq!(path_count(&d), 2, "{fill}");
        assert_eq!(colors(&d.layers[0]), [Color::rgb(1.0, 1.0, 1.0), Color::rgb(1.0, 0.0, 0.0)], "{fill}");
    }
}

#[test]
fn a_page_fill_that_is_art_stays() {
    // A plain PDF's white background is art, as are an .ai's fills that aren't a plain white
    // page under its layers: grey, smaller or larger than the page, stroked, translucent or a
    // white ink.
    let page = "1 1 1 rg 0 0 200 100 re f";
    let kept = [
        (page, false),
        ("0.9 g 0 0 200 100 re f", true),
        ("1 g 10 10 180 80 re f", true),
        ("1 g -20 -20 240 140 re f", true),
        ("1 g 0 G 0 0 200 100 re B", true),
        ("/GS0 gs 1 g 0 0 200 100 re f", true),
        ("/W cs 1 scn 0 0 200 100 re f", true),
    ];
    for (fill, ai) in kept {
        let d = page_fill(fill, "", ai);
        assert_eq!(names(&d), ["Page 1", "Shapes"], "{fill}");
        assert_eq!(path_count(&d), 3, "{fill}");
    }
    // Painted over the layers.
    let d = page_fill("", page, true);
    assert_eq!(names(&d), ["Shapes", "Page 1"]);
    assert_eq!(path_count(&d), 3);
}

/// A page drawing a red, a blue and a green square in groups `defs` (as `/MC0`–`/MC2`), the
/// catalog listing them with default configuration `config`, plus `more` objects; in `config` and
/// `more`, `{0}`–`{2}` stand for the groups.
fn three_groups(defs: [&str; 3], more: &[&str], config: &str) -> Vec<u8> {
    let refs = [0, 1, 2].map(|i| format!("{} 0 R", first_extra(1) + i));
    let fill = |s: &str| (0..3).fold(s.to_string(), |s, i| s.replace(&format!("{{{i}}}"), &refs[i]));
    let content = "/OC /MC0 BDC 1 0 0 rg 10 10 20 20 re f EMC /OC /MC1 BDC 0 0 1 rg 40 40 20 20 re f EMC /OC /MC2 BDC 0 1 0 rg 70 70 20 20 re f EMC";
    let page = PdfPage { resources: fill("/Properties << /MC0 {0} /MC1 {1} /MC2 {2} >>"), ..PdfPage::new(100.0, 100.0, content) };
    let catalog = format!("/OCProperties << /OCGs [{}] /D << {} >> >> ", refs.join(" "), fill(config));
    let more: Vec<String> = more.iter().map(|m| fill(m)).collect();
    let objs: Vec<&str> = defs.iter().copied().chain(more.iter().map(String::as_str)).collect();
    pdf_with_catalog(&[page], &objs, &catalog, None)
}

fn ocg(name: &str, usage: &str) -> String {
    format!("<< /Type /OCG /Name ({name}) {usage} >>")
}

fn visibility(layers: &[std::sync::Arc<Node>]) -> Vec<bool> {
    layers.iter().map(|l| l.visible).collect()
}

#[test]
fn a_view_state_that_is_off_hides_the_layer_with_its_art() {
    let off = ocg("Off", "/Usage << /View << /ViewState /OFF >> >>");
    // With an /AS entry for viewing, the view state wins over the default configuration.
    let on = ocg("On", "/Usage << /View << /ViewState /ON >> >>");
    let bytes = three_groups([&ocg("Plain", ""), &off, &on], &[], "/OFF [{2}] /AS [<< /Event /View /Category [/View] /OCGs [{1} {2}] >>]");
    let d = import(&bytes).unwrap();
    assert_eq!(names(&d), ["Plain", "Off", "On"]);
    assert_eq!(visibility(&d.layers), [true, false, true]);
    assert_eq!(colors(&d.layers[1]), [Color::rgb(0.0, 0.0, 1.0)]);
    assert_eq!(colors(&d.layers[2]), [Color::rgb(0.0, 1.0, 0.0)], "the art the default configuration hides is read");
    // Without one, a view state that is off still hides the layer.
    let d = import(&three_groups([&ocg("Plain", ""), &off, &ocg("Other", "")], &[], "")).unwrap();
    assert_eq!(visibility(&d.layers), [true, false, true]);
    assert_eq!(colors(&d.layers[1]), [Color::rgb(0.0, 0.0, 1.0)]);
}

#[test]
fn nested_groups_become_sublayers_and_hidden_ones_keep_their_art() {
    // The order as an object of its own (as Illustrator writes it): "Top" holds "Sub", which is
    // off; a label's groups stay at the level of the label.
    let order = first_extra(1) + 3;
    let bytes = three_groups(
        [&ocg("Top", ""), &ocg("Sub", ""), &ocg("Other", "")],
        &["[{0} [{1}] [(Label) {2}]]"],
        &format!("/OFF [{{1}}] /Order {order} 0 R"),
    );
    let d = import(&bytes).unwrap();
    assert_eq!(names(&d), ["Top", "Other"]);
    assert_eq!(visibility(&d.layers), [true, true]);
    let top = d.layers[0].children().unwrap();
    assert_eq!(top.len(), 2, "its art, then its sublayer");
    assert_eq!(top[0].appearance.fill().and_then(|f| f.paint.color()), Some(Color::rgb(1.0, 0.0, 0.0)));
    let sub = &top[1];
    assert!(matches!(sub.kind, NodeKind::Layer { .. }) && sub.name.as_deref() == Some("Sub"));
    assert!(!sub.visible, "the sublayer that is off is hidden");
    assert_eq!(colors(sub), [Color::rgb(0.0, 0.0, 1.0)], "with its art");
    assert_eq!(d.layer_color(sub.id), d.layer_color(d.layers[0].id), "a sublayer takes its layer's colour");
    assert_eq!(colors(&d.layers[1]), [Color::rgb(0.0, 1.0, 0.0)]);
}

#[test]
fn a_sublayer_that_paints_first_brings_its_parent_layer() {
    // "Sub" paints first; "Top", its parent, paints its own art last.
    let bytes = three_groups([&ocg("Sub", ""), &ocg("Other", ""), &ocg("Top", "")], &[], "/Order [{2} [{0}] {1}]");
    let d = import(&bytes).unwrap();
    assert_eq!(names(&d), ["Top", "Other"]);
    let top = d.layers[0].children().unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].name.as_deref(), Some("Sub"));
    assert_eq!(colors(&top[0]), [Color::rgb(1.0, 0.0, 0.0)]);
    assert_eq!(top[1].appearance.fill().and_then(|f| f.paint.color()), Some(Color::rgb(0.0, 1.0, 0.0)));
}

#[test]
fn hostile_layer_orders_stay_bounded() {
    // A group listed twice, under itself, and an order nested far deeper than layers go.
    let deep = format!("{}{{2}}{}", "[".repeat(100), "]".repeat(100));
    let order = format!("[{{0}} [{{0}} {{1}}] {{1}} [{{0}}] {deep}]");
    let d = import(&three_groups([&ocg("A", ""), &ocg("B", ""), &ocg("C", "")], &[], &format!("/Order {order}"))).unwrap();
    assert_eq!(names(&d), ["A", "C"], "C, listed too deep, stays a top-level layer");
    assert_eq!(d.layers[0].children().unwrap()[1].name.as_deref(), Some("B"));
    let mut count = 0;
    d.walk(|n| count += usize::from(matches!(n.kind, NodeKind::Layer { .. })));
    assert_eq!(count, 3, "each group is one layer");
}

#[test]
fn thousands_of_layers_hidden_ones_included_all_come_in() {
    const N: usize = 3000;
    let first = first_extra(1);
    let objs: Vec<String> = (0..N).map(|i| ocg(&format!("L{i}"), "")).collect();
    let refs: Vec<String> = (0..N).map(|i| format!("{} 0 R", first + i)).collect();
    let props: String = refs.iter().enumerate().map(|(i, r)| format!("/M{i} {r} ")).collect();
    let content: String = (0..N).map(|i| format!("/OC /M{i} BDC {} {} 1 1 re f EMC ", i % 100, i / 100)).collect();
    let off: Vec<&str> = refs.iter().step_by(2).map(String::as_str).collect();
    let catalog = format!("/OCProperties << /OCGs [{}] /D << /OFF [{}] >> >> ", refs.join(" "), off.join(" "));
    let page = PdfPage { resources: format!("/Properties << {props}>>"), ..PdfPage::new(100.0, 100.0, &content) };
    let objs: Vec<&str> = objs.iter().map(String::as_str).collect();
    let d = import(&pdf_with_catalog(&[page], &objs, &catalog, None)).unwrap();
    assert_eq!(d.layers.len(), N);
    assert!(d.layers.iter().enumerate().all(|(i, l)| l.visible == (i % 2 == 1) && l.children().is_some_and(|c| c.len() == 1)));
}

/// Import `bytes` with the scan's operator limit lowered to `n`.
fn import_with_max_ops(bytes: &[u8], n: usize) -> ImportReport {
    let old = crate::import_scan::TEST_MAX_OPS.replace(n);
    let r = import_with_report(bytes, &ImportOptions::default());
    crate::import_scan::TEST_MAX_OPS.set(old);
    r.unwrap()
}

#[test]
fn art_past_the_scan_limit_never_shows_a_hidden_layer() {
    // "Shapes" draws 100 operators before its square, then "Hidden" (off, not printed) its own.
    let (objs, catalog, ids) = groups(1, "/OFF [{B}]");
    let filler = "q Q ".repeat(50);
    let page = PdfPage {
        resources: format!("/Properties << /MC0 {} 0 R /MC1 {} 0 R >>", ids[0], ids[1]),
        ..PdfPage::new(
            100.0,
            100.0,
            &format!("/OC /MC0 BDC {filler}1 0 0 rg 10 10 30 30 re f EMC /OC /MC1 BDC 0 0 1 rg 50 50 30 30 re f EMC 0 1 0 rg 0 0 5 5 re f"),
        )
    };
    let bytes = pdf_with_catalog(&[page], &[&objs[0], &objs[1]], &catalog, None);
    // Within the limit, every group is told apart.
    assert_eq!(names(&import(&bytes).unwrap()), ["Shapes", "Hidden", "Page 1"]);
    let r = import_with_max_ops(&bytes, 60);
    let d = &r.document;
    assert_eq!(names(d), ["Shapes", "Page 1 (unsorted)", "Page 1"], "{:?}", r.warnings);
    // The art drawn past the limit in a group already open stays in it.
    assert!(d.layers[0].visible);
    assert_eq!(colors(&d.layers[0]), [Color::rgb(1.0, 0.0, 0.0)]);
    // The hidden group's art, past it, can't be told apart: it neither shows nor prints.
    let unsorted = &d.layers[1];
    assert!(!unsorted.visible && !printable(unsorted));
    assert_eq!(colors(unsorted), [Color::rgb(0.0, 0.0, 1.0)]);
    // Art outside any group is the page's, as before.
    assert!(d.layers[2].visible && printable(&d.layers[2]));
    assert_eq!(colors(&d.layers[2]), [Color::rgb(0.0, 1.0, 0.0)]);
    assert!(r.warnings.iter().any(|w| w.contains("more than can be read") && w.contains("hidden, non-printing")), "{:?}", r.warnings);
}

#[test]
fn art_past_the_scan_limit_of_shown_layers_stays_shown() {
    // Every group shows and prints: what can't be told apart goes to the page's layer, shown.
    let bytes = three_groups([&ocg("A", ""), &ocg("B", ""), &ocg("C", "")], &[], "");
    let r = import_with_max_ops(&bytes, 4);
    let d = &r.document;
    assert_eq!(names(d), ["A", "Page 1"], "{:?}", r.warnings);
    assert_eq!(colors(&d.layers[0]), [Color::rgb(1.0, 0.0, 0.0)]);
    assert!(d.layers[1].visible && printable(&d.layers[1]));
    assert_eq!(colors(&d.layers[1]), [Color::rgb(0.0, 0.0, 1.0), Color::rgb(0.0, 1.0, 0.0)]);
    assert!(r.warnings.iter().any(|w| w.contains("went to its page's layer")), "{:?}", r.warnings);
}

/// Art reaching past the page keeps a clip to it in a plain PDF (as viewers show it), but not in
/// a PDF-compatible `.ai`, whose art crossing the artboard edge opens whole and editable (#646).
#[test]
fn art_past_the_page_is_clipped_in_a_pdf_but_not_in_an_ai_file() {
    for ai in [false, true] {
        let page = PdfPage {
            entries: if ai { "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into() } else { String::new() },
            ..PdfPage::new(100.0, 100.0, "0 0 100 100 re W n 1 0 0 rg 50 50 200 200 re f")
        };
        let r = import_with_report(&pdf_with_catalog(&[page], &[], "", None), &ImportOptions::default()).unwrap();
        let mut clips = 0;
        r.document.walk(|n| clips += usize::from(matches!(n.kind, NodeKind::Group { clip: true, .. })));
        let past = r.warnings.iter().any(|w| w == crate::import::PAST_PAGE_NOTE);
        assert_eq!((clips, past), if ai { (0, false) } else { (1, true) }, "ai: {ai}: {:?}", r.warnings);
        let art = r.document.art_bounds().unwrap();
        assert_eq!(art.x1 > 100.5, ai, "ai: {ai}: {art:?}");
    }
}

/// The editor's private data of a `.ai` is its `AIPrivateData` streams, joined in order.
#[test]
fn an_ai_files_private_data_is_its_streams_joined() {
    let (a, b) = (first_extra(1), first_extra(1) + 1);
    let page = |ai: bool| PdfPage {
        entries: if ai {
            format!("/PieceInfo << /Illustrator << /Private << /AIPrivateData1 {a} 0 R /AIPrivateData2 {b} 0 R /NumBlock 2 >> >> >>")
        } else {
            String::new()
        },
        ..PdfPage::new(100.0, 100.0, "1 0 0 rg 10 10 30 30 re f")
    };
    let (one, two) = (stream("", "%AI24_ZStandard_Data first "), stream("", "second"));
    let ai = pdf_with_catalog(&[page(true)], &[&one, &two], "", None);
    assert_eq!(illustrator_data(&ai, None).unwrap(), b"%AI24_ZStandard_Data first second");
    // A PDF without them has none; neither has bytes that aren't a PDF.
    assert!(illustrator_data(&pdf_with_catalog(&[page(false)], &[&one, &two], "", None), None).is_none());
    assert!(illustrator_data(b"not a pdf", None).is_none());
    // Only a plain or a deflated stream is read: a chain of filters is refused, not run unbounded.
    let chained = stream("/Filter [/ASCIIHexDecode /ASCIIHexDecode]", "61>");
    assert!(illustrator_data(&pdf_with_catalog(&[page(true)], &[&chained, &two], "", None), None).is_none());
}
