use super::*;
use vectorcraft_doc::Document;

/// The texts [`prepare`] builds from `body` and how many times it read a `<textPath>`'s path.
fn prepared(body: &str) -> (usize, u64, Vec<String>) {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400">{body}</svg>"#);
    let xml = roxmltree::Document::parse(&svg).unwrap();
    let mut warnings = vec![];
    let before = TEXT_PATH_READS.get();
    let (_, slots) = prepare(&svg, &xml, None, 96.0, &mut warnings);
    (slots.texts.len(), TEXT_PATH_READS.get() - before, warnings)
}

/// Every `<use>` of a text on a path follows the path the text's own build read.
#[test]
fn uses_of_a_text_on_a_path_read_the_path_once() {
    let uses: String = (101..=150).map(|i| format!(r##"<use href="#t" font-size="{i}"/>"##)).collect();
    let (texts, reads, warnings) =
        prepared(&format!(r##"<defs><path id="p" d="M0 20L150 20"/><text id="t"><textPath href="#p">Path</textPath></text></defs>{uses}"##));
    assert_eq!(texts, 51, "the text and one per <use>: {warnings:?}");
    assert_eq!(reads, 1);
    // A path with no usable data is not read again either.
    let (texts, reads, _) = prepared(&format!(r##"<defs><text id="t"><textPath href="#none">Path</textPath></text></defs>{uses}"##));
    assert_eq!((texts, reads), (51, 1));
    // The path's pathLength is read with it.
    let (texts, reads, _) = prepared(&format!(
        r##"<defs><path id="p" d="M0 20L150 20" pathLength="10"/><text id="t"><textPath href="#p" startOffset="5">Path</textPath></text></defs>{uses}"##
    ));
    assert_eq!((texts, reads), (51, 1));
}

/// How many times [`prepare`] walked the document's elements to find the element a
/// `<textPath>` names.
fn id_walks(body: &str) -> u64 {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400">{body}</svg>"#);
    let xml = roxmltree::Document::parse(&svg).unwrap();
    let before = ID_WALKS.get();
    prepare(&svg, &xml, None, 96.0, &mut vec![]);
    ID_WALKS.get() - before
}

/// The texts on paths in one file find the elements they name through one walk of the document.
#[test]
fn text_paths_find_their_elements_in_one_walk() {
    let paths: String = (0..20).map(|i| format!(r#"<path id="p{i}" d="M0 {i}L150 {i}"/>"#)).collect();
    let texts: String = (0..20).map(|i| format!(r##"<text id="t{i}"><textPath href="#p{i}">Path</textPath></text>"##)).collect();
    assert_eq!(id_walks(&format!("<defs>{paths}</defs>{texts}")), 1);
    // Texts shown only through `<use>` elements share the walk, and a missing id needs no other.
    let uses: String = (0..20).map(|i| format!(r##"<use href="#t{i}" font-size="20"/>"##)).collect();
    let missing = r##"<text><textPath xlink:href="#none" xmlns:xlink="http://www.w3.org/1999/xlink">Path</textPath></text>"##;
    assert_eq!(id_walks(&format!("<defs>{paths}{texts}</defs>{uses}{missing}")), 1);
    // A file whose texts name no element is not walked.
    assert_eq!(id_walks(r#"<text>Plain</text><text><textPath path="M0 20L150 20">Inline</textPath></text>"#), 0);
    assert_eq!(id_walks(r#"<path id="p" d="M0 20L150 20"/><text><textPath href="p">No hash</textPath></text>"#), 0);
}

/// The document and warnings [`crate::import`] gives for `body`, with each `<textPath>`'s
/// element found through the map of ids or, with `linear`, by walking the document for each one.
fn imported(body: &str, linear: bool) -> (Document, Vec<String>) {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="400">{body}</svg>"#);
    LINEAR_IDS.set(linear);
    let out = crate::import::import(&svg, &crate::ImportOptions::default(), crate::MAX_NODES);
    LINEAR_IDS.set(false);
    out.unwrap_or_else(|e| panic!("{e}"))
}

/// The text objects of `d`, in paint order.
fn text_objects(d: &Document) -> Vec<TextObject> {
    let mut v = vec![];
    d.walk(|n| {
        if let vectorcraft_doc::NodeKind::Text(t) = &n.kind {
            v.push((**t).clone());
        }
    });
    v
}

/// The map of ids finds the element the document walk finds: the first element in document order
/// with the id, whatever it is.
#[test]
fn text_paths_find_the_element_a_document_walk_finds() {
    let cases = [
        // Several paths, one of them named twice, and shapes.
        r##"<defs><path id="a" d="M0 20L150 20"/><path id="b" d="M0 50C50 0 100 100 150 50" pathLength="10"/><path id="a" d="M0 80L150 80"/>
            <circle id="c" cx="200" cy="200" r="80"/><rect id="r" x="10" y="10" width="200" height="100" rx="5"/></defs>
            <text font-size="12"><textPath href="#a">First</textPath></text>
            <text font-size="12"><textPath xlink:href=" #b " startOffset="5">Second</textPath></text>
            <text font-size="12"><textPath href="#c" side="right">Circle</textPath></text>
            <text font-size="12"><textPath href="#r" startOffset="50%">Rect</textPath></text>"##,
        // A missing id, an empty one, and one without a `#`.
        r##"<path id="p" d="M0 20L150 20"/><text><textPath href="#none">Missing</textPath></text>
            <text><textPath href="#">Empty</textPath></text><text><textPath href="p">Bare</textPath></text>"##,
        // The first element with the id has no path; the later path is not used.
        r##"<g id="p"><path d="M0 20L150 20"/></g><path id="p" d="M0 80L150 80"/><text><textPath href="#p">Group</textPath></text>"##,
        // Ids on texts and groups, one of them named twice.
        r##"<g id="g"><text id="t"><textPath href="#t">Self</textPath></text></g><text id="g">Later</text><text><textPath href="#g">Group</textPath></text>"##,
        // Uses of texts on paths, and an inline path beside an href.
        r##"<defs><path id="p" d="M0 20L150 20"/><text id="t"><textPath href="#p">Used</textPath></text></defs>
            <use href="#t" font-size="30"/><use href="#t" y="40"/><text><textPath href="#p" path="M0 90L150 90">Inline</textPath></text>"##,
    ];
    for body in cases {
        let map = imported(body, false);
        assert_eq!(map, imported(body, true), "{body}");
        assert!(!text_objects(&map.0).is_empty(), "{body}");
    }
}

/// With two elements of one id, a `<textPath>` follows the first.
#[test]
fn a_text_path_follows_the_first_element_with_its_id() {
    let text = r##"<text font-size="12"><textPath href="#p">Path</textPath></text>"##;
    let first = text_objects(&imported(&format!(r#"<path id="p" d="M0 20L150 20"/><path id="p" d="M0 80L150 80"/>{text}"#), false).0);
    let only = text_objects(&imported(&format!(r#"<path id="p" d="M0 20L150 20"/><path id="q" d="M0 80L150 80"/>{text}"#), false).0);
    let second = text_objects(&imported(&format!(r#"<path id="q" d="M0 20L150 20"/><path id="p" d="M0 80L150 80"/>{text}"#), false).0);
    assert!(matches!(first[0].kind, TextKind::OnPath { .. }), "{:?}", first[0].kind);
    assert_eq!(first, only);
    assert_ne!(first, second);
    // A first element with no path leaves the text without one.
    let (doc, warnings) = imported(&format!(r#"<g id="p"/><path id="p" d="M0 80L150 80"/>{text}"#), false);
    assert!(!matches!(text_objects(&doc)[0].kind, TextKind::OnPath { .. }), "{warnings:?}");
}

/// Times importing a file with 20,000 paths and 2,000 texts on paths, finding each `<textPath>`'s
/// element through the map of ids and by walking the document for each one, after checking that
/// the two give the same document. Run it with
/// `cargo test --release -p vectorcraft-svg --lib text_path_lookup_timing -- --ignored --nocapture`.
#[test]
#[ignore = "timing; run by hand in a release build"]
fn text_path_lookup_timing() {
    let mut body = String::new();
    for i in 0..20_000 {
        let _ = write!(body, r#"<path id="p{i}" d="M0 {}L300 {}"/>"#, i % 400, i % 400);
        if i % 10 == 9 {
            let k = (i * 7919) % 20_000;
            let _ = write!(body, r##"<text font-size="8"><textPath href="#p{k}">L{i}</textPath></text>"##);
        }
    }
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400">{body}</svg>"#);
    let run = |linear: bool| {
        LINEAR_IDS.set(linear);
        let start = std::time::Instant::now();
        let out = crate::import::import(&svg, &crate::ImportOptions::default(), crate::MAX_NODES);
        let took = start.elapsed();
        LINEAR_IDS.set(false);
        (out.unwrap_or_else(|e| panic!("{e}")), took)
    };
    let (map, _) = run(false);
    let (walk, _) = run(true);
    assert_eq!(map, walk);
    assert_eq!(text_objects(&map.0).len(), 2_000);
    let (mut maps, mut walks) = (vec![], vec![]);
    for _ in 0..7 {
        walks.push(run(true).1);
        maps.push(run(false).1);
    }
    println!("walk per textPath: {walks:?}");
    println!("map of ids:        {maps:?}");
    walks.sort();
    maps.sort();
    println!("median: walk {:?}, map {:?}", walks[3], maps[3]);
}
