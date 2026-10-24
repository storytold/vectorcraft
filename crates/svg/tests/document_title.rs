//! Document title read from SVG on import (#3626).
#![allow(clippy::unwrap_used)]

#[test]
fn root_title_and_metadata_dc_title() {
    let svg = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"><title>Access paths</title></svg>"#;
    assert_eq!(vectorcraft_svg::document_title(svg).as_deref(), Some("Access paths"));
    let meta = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>From metadata</dc:title></metadata></svg>"#;
    assert_eq!(vectorcraft_svg::document_title(meta).as_deref(), Some("From metadata"));
    let both = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" xmlns:dc="http://purl.org/dc/elements/1.1/"><title>Root wins</title><metadata><dc:title>Other</dc:title></metadata></svg>"#;
    assert_eq!(vectorcraft_svg::document_title(both).as_deref(), Some("Root wins"));
}
