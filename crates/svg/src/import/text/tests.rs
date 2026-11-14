use super::*;

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
}
