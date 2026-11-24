//! Synthetic regression verifying locked group and locked object import in AI fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_pdf::import;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};

#[test]
fn ai_fixture_with_locked_group_and_locked_object_imports_with_locks_preserved() {
    let (a, b, c, d) = (first_extra(1), first_extra(1) + 1, first_extra(1) + 2, first_extra(1) + 3);
    let order_obj = first_extra(1) + 4;
    let objs = [
        "<< /Type /OCG /Name (Layer1) >>".to_string(),
        "<< /Type /OCG /Name (LockedGroup) /Locked true >>".to_string(),
        "<< /Type /OCG /Name (UnlockedGroup) >>".to_string(),
        "<< /Type /OCG /Name (LockedObject) /Locked true >>".to_string(),
    ];
    let catalog = format!("/OCProperties << /OCGs [{a} 0 R {b} 0 R {c} 0 R {d} 0 R] /D << /Order {order_obj} 0 R >> >> ");
    let order_body = format!("[{a} 0 R [{b} 0 R {c} 0 R]]");
    let content = "/OC /MC0 BDC 1 0 0 rg 10 10 20 20 re f EMC \
         /OC /MC1 BDC 0 1 0 rg 30 30 20 20 re f EMC \
         /OC /MC2 BDC 0 0 1 rg 50 50 20 20 re f EMC \
         /OC /MC3 BDC 1 1 0 rg 70 70 20 20 re f EMC \
         0 0 0 rg 0 0 10 10 re f";
    let page = PdfPage {
        resources: format!("/Properties << /MC0 {a} 0 R /MC1 {b} 0 R /MC2 {c} 0 R /MC3 {d} 0 R >>"),
        entries: "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into(),
        ..PdfPage::new(100.0, 100.0, content)
    };
    let bytes = pdf_with_catalog(&[page], &[&objs[0], &objs[1], &objs[2], &objs[3], &order_body], &catalog, None);
    let doc = import(&bytes).unwrap();

    let layer1 = doc.layers.iter().find(|l| l.name.as_deref() == Some("Layer1")).expect("Layer1 exists");
    assert!(!layer1.locked, "Layer1 is not locked");

    let children = layer1.children().expect("Layer1 has children");
    let locked_group = children.iter().find(|n| n.name.as_deref() == Some("LockedGroup")).expect("LockedGroup exists");
    assert!(locked_group.locked, "LockedGroup has locked: true");

    let unlocked_group = children.iter().find(|n| n.name.as_deref() == Some("UnlockedGroup")).expect("UnlockedGroup exists");
    assert!(!unlocked_group.locked, "UnlockedGroup has locked: false");

    let locked_obj = doc.layers.iter().find(|l| l.name.as_deref() == Some("LockedObject")).expect("LockedObject exists");
    assert!(locked_obj.locked, "LockedObject has locked: true");

    if let Some(page_layer) = doc.layers.iter().find(|l| l.name.as_deref() == Some("Page 1")) {
        assert!(!page_layer.locked, "Page layer is not locked");
        if let Some(kids) = page_layer.children() {
            for kid in kids {
                assert!(!kid.locked, "Non-locked object in page layer remains unlocked");
            }
        }
    }
}

#[test]
fn catalog_level_piece_info_with_aipdfprivatedata_detects_ai_data_and_preserves_locks() {
    let (a, b) = (first_extra(1), first_extra(1) + 1);
    let order_obj = first_extra(1) + 2;
    let objs = ["<< /Type /OCG /Name (Layer1) >>".to_string(), "<< /Type /OCG /Name (LockedGroup) /Locked true >>".to_string()];
    let catalog = format!(
        "/OCProperties << /OCGs [{a} 0 R {b} 0 R] /D << /Order {order_obj} 0 R >> >> \
         /PieceInfo << /Illustrator << /Private << /AIPDFPrivateData1 7 /NumBlock 1 >> >> >>"
    );
    let order_body = format!("[{a} 0 R [{b} 0 R]]");
    let content = "/OC /MC0 BDC 1 0 0 rg 10 10 20 20 re f EMC \
         /OC /MC1 BDC 0 1 0 rg 30 30 20 20 re f EMC \
         0 0 0 rg 0 0 10 10 re f";
    let page = PdfPage { resources: format!("/Properties << /MC0 {a} 0 R /MC1 {b} 0 R >>"), ..PdfPage::new(100.0, 100.0, content) };
    let bytes = pdf_with_catalog(&[page], &[&objs[0], &objs[1], &order_body], &catalog, None);
    let doc = import(&bytes).unwrap();

    let layer1 = doc.layers.iter().find(|l| l.name.as_deref() == Some("Layer1")).expect("Layer1 exists");
    assert!(!layer1.locked, "Layer1 is not locked");

    let children = layer1.children().expect("Layer1 has children");
    let locked_group = children.iter().find(|n| n.name.as_deref() == Some("LockedGroup")).expect("LockedGroup exists");
    assert!(locked_group.locked, "LockedGroup has locked: true");
}
