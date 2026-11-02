//! CSS classes on imported elements come back on export, beside the generated `cls-N` classes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use usvg::roxmltree;
use vectorcraft_svg::{ExportOptions, Styling, export, import};

const SRC: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><g id="grp" class="wrap  big"><rect id="r" class="hero hero cls-7" width="20" height="20" fill="red"/><circle id="c" r="5" fill="blue"/></g></svg>"#;

fn class_of(svg: &str, id: &str) -> Option<String> {
    let doc = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).unwrap();
    let n = doc.descendants().find(|n| n.attribute("id") == Some(id))?;
    n.attribute("class").map(str::to_string)
}

#[test]
fn imported_classes_survive_every_styling_with_generated_ones() {
    let doc = import(SRC).unwrap();
    for styling in [Styling::PresentationAttributes, Styling::InlineStyle, Styling::InternalCss, Styling::StyleEntities] {
        let out = export(&doc, &ExportOptions { styling, ..ExportOptions::default() });
        assert_eq!(class_of(&out, "grp").as_deref(), Some("wrap big"), "{styling:?}");
        let rect = class_of(&out, "r").unwrap();
        if styling == Styling::InternalCss {
            assert!(rect.starts_with("hero ") && rect.contains("cls-") && !rect.contains("cls-7"), "{rect}");
        } else {
            assert_eq!(rect, "hero");
        }
        assert!(class_of(&out, "c").is_none_or(|c| c.starts_with("cls-")));
        // A second round trip changes nothing.
        let again = export(&import(&out).unwrap(), &ExportOptions { styling, ..ExportOptions::default() });
        assert_eq!(class_of(&again, "r"), Some(rect), "{styling:?}");
    }
}

#[test]
fn class_names_with_colons_slashes_and_no_break_spaces_are_kept() {
    let src = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect id=\"r\" class=\"hover:bg-blue w-1/2 a\u{a0}b\" width=\"5\" height=\"5\" fill=\"red\"/></svg>";
    let out = export(&import(src).unwrap(), &ExportOptions::default());
    assert_eq!(class_of(&out, "r").as_deref(), Some("hover:bg-blue w-1/2 a\u{a0}b"));
}
