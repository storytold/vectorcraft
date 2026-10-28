#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_engine::cmd::fileio::{LoadOptions, detect, load, load_with};

#[test]
fn test_cdr_detection() {
    assert!(detect("logo.cdr", b"RIFF\x00\x00\x00\x00CDR9").is_some());
    let fmt = detect("logo.cdr", b"RIFF\x00\x00\x00\x00CDR9").unwrap();
    assert_eq!(fmt.id, "cdr");
    assert_eq!(fmt.label, "CorelDRAW");
}

#[test]
fn test_cdr_load_real_file() {
    let path = std::path::Path::new("/home/deck/Pictures/cdr/2010_KAL.cdr");
    if let Ok(bytes) = std::fs::read(path) {
        let res = load("2010_KAL.cdr", &bytes);
        assert!(res.is_ok(), "failed to load 2010_KAL.cdr: {:?}", res.err());
        let loaded = res.unwrap();
        assert!(!loaded.doc.layers.is_empty());
        let desktop_layer = loaded.doc.layers.iter().find(|l| l.name.as_deref() == Some("Desktop"));
        assert!(desktop_layer.is_some(), "Desktop layer should be present");
        let children = desktop_layer.unwrap().children().unwrap();
        assert!(!children.is_empty(), "Desktop layer should contain imported objects");
        assert!(children.iter().any(|c| matches!(&c.kind, vectorcraft_doc::NodeKind::Text(_))));
        assert!(children.iter().any(|c| matches!(&c.kind, vectorcraft_doc::NodeKind::Path { .. })));

        // Also test TextAs::Outlines
        let outlines_opts = LoadOptions { text_as: vectorcraft_pdf::TextAs::Outlines, ..Default::default() };
        let outlined = load_with("2010_KAL.cdr", &bytes, &outlines_opts).expect("load 2010_KAL.cdr as outlines");
        let outlined_children = outlined.doc.layers.iter().find(|l| l.name.as_deref() == Some("Desktop")).unwrap().children().unwrap();
        assert!(
            !outlined_children.iter().any(|c| matches!(&c.kind, vectorcraft_doc::NodeKind::Text(_))),
            "TextAs::Outlines should convert all Text nodes into vector shapes"
        );
    }

    let zap_path = std::path::Path::new("/home/deck/Pictures/cdr/spiewnik_CMC/ZAPROSZENIE_CMC.cdr");
    if let Ok(bytes) = std::fs::read(zap_path) {
        let loaded = load("ZAPROSZENIE_CMC.cdr", &bytes).expect("load ZAPROSZENIE_CMC.cdr");
        assert_eq!(loaded.doc.artboards.len(), 2, "ZAPROSZENIE_CMC.cdr should have 2 artboards");
        assert!(!loaded.doc.images.is_empty(), "ZAPROSZENIE_CMC.cdr should have embedded bitmaps");
    }

    let spiewnik_path = std::path::Path::new("/home/deck/Pictures/cdr/spiewnik_2010.cdr");
    if let Ok(bytes) = std::fs::read(spiewnik_path) {
        let loaded = load("spiewnik_2010.cdr", &bytes).expect("load spiewnik_2010.cdr");
        assert_eq!(loaded.doc.artboards.len(), 33, "spiewnik_2010.cdr should have 33 artboards");
        if let Some(desktop) = loaded.doc.layers.first()
            && let Some(children) = desktop.children()
        {
            for c in children {
                if let vectorcraft_doc::NodeKind::Text(to) = &c.kind {
                    let text = to.runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("");
                    let y = to.xf.as_coeffs()[5];
                    if (y - 507.0).abs() < 8.0 {
                        eprintln!(
                            "LINE 507 OBJ: text={text:?} x={:?} y={y:?} size={:?}",
                            to.xf.as_coeffs()[4],
                            to.runs.first().map(|r| r.style.size)
                        );
                    }
                }
            }
        }
    }
}
