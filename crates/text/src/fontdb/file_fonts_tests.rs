use super::*;
use crate::{FileFontInfo, FileGlyph, build_file_font, layout};
use kurbo::{Point, Rect, Shape};
use vectorcraft_doc::{CharStyle, Document, FILE_FONT_MIME, FileFont, FileFonts, ImageBlob, Node, NodeId, NodeKind, TextObject};

const FAMILY: &str = "Owned File Font Regression";

fn blob(width: f64) -> ImageBlob {
    let glyph = FileGlyph { ch: 'x', outline: Rect::new(0.0, 0.0, width, 700.0).to_path(0.1), advance: width + 100.0 };
    let info = FileFontInfo { family: FAMILY.into(), style: "Regular".into(), ..Default::default() };
    ImageBlob::new(FILE_FONT_MIME, build_file_font(&info, &[glyph]).unwrap())
}

fn text(key: &str) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, "x", CharStyle { font_family: FAMILY.into(), size: 20.0, ..Default::default() });
    t.file_fonts = Some(Box::new(FileFonts::new(vec![FileFont { family: FAMILY.into(), style: "Regular".into(), key: key.into() }], "x")));
    t
}

fn document(key: &str, blob: ImageBlob) -> Document {
    let mut doc = Document::new(100.0, 100.0);
    doc.images.insert(key.into(), blob);
    doc.layers.push(Arc::new(Node::new(NodeId(100), NodeKind::Text(Box::new(text(key))))));
    doc
}

fn first_text(doc: &Document) -> &TextObject {
    let NodeKind::Text(t) = &doc.layers.last().unwrap().kind else { panic!("text") };
    t
}

#[test]
fn chosen_keys_cannot_replace_another_documents_owned_font() {
    for reverse in [false, true] {
        let db = FontDb::with_font_dirs(vec![]);
        let (a, b) = (blob(200.0), blob(800.0));
        // A chooses the exact key an honest B derives from its different bytes.
        let key = b.content_key();
        let (mut da, mut dbb) = (document(&key, a.clone()), document(&key, b.clone()));
        let (first, second) = if reverse { (&mut dbb, &mut da) } else { (&mut da, &mut dbb) };
        assert!(db.bind_document_file_fonts(first).is_empty());
        assert!(db.bind_document_file_fonts(second).is_empty());
        for (doc, owned) in [(&da, &a), (&dbb, &b)] {
            let laid = layout(&db, first_text(doc));
            assert_eq!(laid.glyphs.len(), 1);
            let face = db.face_by_id(laid.glyphs[0].font_id).unwrap();
            assert_eq!(face.file_data(), owned.bytes.as_slice());
        }
        assert!(layout(&db, first_text(&dbb)).bounds.width() > layout(&db, first_text(&da)).bounds.width() * 2.0);
    }
}

#[test]
fn metadata_and_forged_internal_family_do_not_authorize_global_faces() {
    let db = FontDb::with_font_dirs(vec![]);
    let owned = blob(500.0);
    let face = db.keep_file_font(&owned).unwrap();
    let id = face.id();
    let mut missing = text(&owned.content_key());
    assert!(!db.bind_file_fonts(&mut missing, |_| None));
    assert!(db.file_faces(&missing).is_empty());
    assert!(layout(&db, &missing).glyphs.iter().all(|g| g.font_id != id));

    let mut json = serde_json::to_value(CharStyle::default()).unwrap();
    json["font_family"] = format!("\u{1}file font {id}").into();
    json["file_font_id"] = id.into();
    let forged: CharStyle = serde_json::from_value(json).unwrap();
    assert_eq!(forged.file_font_id, None);
    assert!(db.style_face(&forged).is_none_or(|face| face.id() != id));
    let forged = TextObject::point(Point::ZERO, "x", forged);
    assert!(layout(&db, &forged).glyphs.iter().all(|g| g.font_id != id));

    let mut owned_text = text("chosen");
    assert!(db.bind_file_fonts(&mut owned_text, |_| Some(&owned)));
    let serialized = serde_json::to_string(&owned_text).unwrap();
    let unowned: TextObject = serde_json::from_str(&serialized).unwrap();
    assert!(db.file_faces(&unowned).is_empty(), "runtime authorization never travels as metadata");
    let mut invalid = document("chosen", ImageBlob::new(FILE_FONT_MIME, b"not a font".to_vec()));
    let warnings = db.bind_document_file_fonts(&mut invalid);
    assert!(warnings.iter().any(|w| w.contains(FAMILY) && w.contains("substitute")));
    assert!(db.file_faces(first_text(&invalid)).is_empty());
}

#[test]
fn only_fonts_referenced_by_active_runs_consume_allowance() {
    let db = FontDb::with_font_dirs(vec![]);
    let mut doc = Document::new(100.0, 100.0);
    for i in 0..32 {
        doc.images.insert(format!("unused-{i}"), blob(100.0 + f64::from(i)));
    }
    let mut unused_reference = text("unused-0");
    unused_reference.runs[0].style.font_family = "Another Missing Family".into();
    doc.layers.push(Arc::new(Node::new(NodeId(100), NodeKind::Text(Box::new(unused_reference)))));
    assert!(db.bind_document_file_fonts(&mut doc).is_empty());
    assert!(db.read_file_faces().by_id.is_empty());
    let wanted = blob(900.0);
    let face = db.keep_file_font_up_to(&wanted, 1).unwrap();
    assert_eq!(face.file_data(), wanted.bytes.as_slice());
    assert_eq!(db.read_file_faces().by_id.len(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn removing_an_installed_exact_face_uses_the_owned_snapshot_fallback() {
    let dir = std::env::temp_dir().join(format!("vc-file-font-{}-installed-fallback", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let installed_path = dir.join("Installed.ttf");
    std::fs::write(&installed_path, blob(800.0).bytes.as_slice()).unwrap();
    let db = FontDb::with_font_dirs(vec![dir.clone()]);
    db.set_system_fallback(false);
    let installed_id = {
        let (face, matched) = db.resolve(FAMILY, "Regular").unwrap();
        assert_eq!(matched, super::super::traits::FontMatch::Exact);
        face.id()
    };
    let mut doc = document("carried", blob(200.0));
    assert!(db.bind_document_file_fonts(&mut doc).is_empty());
    assert!(db.file_faces(first_text(&doc)).is_empty(), "the installed exact face still wins");
    assert_eq!(layout(&db, first_text(&doc)).glyphs[0].font_id, installed_id);
    assert_eq!(db.read_file_faces().by_id.len(), 1, "the referenced carried resource is admitted while dormant");
    let (carried_id, weak) = {
        let face = first_text(&doc).file_fonts.as_ref().unwrap().face("carried").unwrap();
        assert_ne!(face.id(), installed_id);
        assert_eq!(face.bytes(), doc.images["carried"].bytes.as_slice());
        (face.id(), Arc::downgrade(&face))
    };
    let undo = doc.clone();
    let resource = doc.images["carried"].clone();
    let cached_text = first_text(&doc).clone();
    let replacement = blob(900.0);
    assert!(db.keep_file_font_up_to(&replacement, 1).is_none(), "dormant owned resources count toward the live allowance");

    std::fs::remove_file(&installed_path).unwrap();
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(db.load_system_fonts(), 0);
    assert!(db.face_by_id(installed_id).is_none());
    for retained in [&doc, &undo] {
        assert_eq!(layout(&db, first_text(retained)).glyphs[0].font_id, carried_id, "removal needs no reopen or rebind");
    }

    doc.images.clear();
    assert!(!db.bind_document_file_fonts(&mut doc).is_empty());
    assert!(db.file_faces(first_text(&doc)).is_empty());
    drop(doc);
    assert_eq!(layout(&db, first_text(&undo)).glyphs[0].font_id, carried_id, "current-document rebinding cannot revoke Undo ownership");
    drop(undo);
    assert_eq!(layout(&db, &cached_text).glyphs[0].font_id, carried_id, "a resource snapshot still owns the carried face");
    assert!(db.keep_file_font_up_to(&replacement, 1).is_none());
    drop(resource);
    assert!(weak.upgrade().is_none());
    assert!(db.face_by_id(carried_id).is_none());
    assert!(db.file_faces(&cached_text).is_empty(), "text bindings alone do not own admission");
    assert!(db.keep_file_font_up_to(&replacement, 1).is_some(), "closing the last resource owner releases allowance");
}

#[test]
fn document_snapshots_and_inflight_faces_keep_only_live_allowance() {
    let db = FontDb::with_font_dirs(vec![]);
    let (a, b) = (blob(200.0), blob(800.0));
    let mut doc = document("k", a.clone());
    assert!(db.bind_document_file_fonts(&mut doc).is_empty());
    let original = layout(&db, first_text(&doc));
    let id = original.glyphs[0].font_id;
    let inflight = db.face_by_id(id).unwrap();
    let weak = Arc::downgrade(&inflight);
    let open_document = Arc::new(doc);
    let undo_or_copy = open_document.clone();
    let resource_copy = (*open_document).clone();
    let cached_node = open_document.layers.last().unwrap().clone();
    let cached_text = first_text(&open_document).clone();
    let before_expiry = cached_text.clone();
    assert!(db.keep_file_font_up_to(&b, 1).is_none());
    // Identical content may share a live face even while the allowance is full.
    assert_eq!(db.keep_file_font_up_to(&a, 1).unwrap().id(), id);
    drop(open_document);
    assert_eq!(layout(&db, first_text(&undo_or_copy)).glyphs[0].font_id, id);
    assert!(db.keep_file_font_up_to(&b, 1).is_none());
    drop(undo_or_copy);
    assert!(db.keep_file_font_up_to(&b, 1).is_none(), "a copied document owns its image resources too");
    drop(resource_copy);
    drop(a);
    assert!(db.keep_file_font_up_to(&b, 1).is_none(), "an in-flight font Arc still owns its slot");
    drop(inflight);
    assert!(weak.upgrade().is_none());
    assert!(db.face_by_id(id).is_none());
    assert_eq!(cached_text, before_expiry, "weak expiry must not change cache identity");
    assert!(db.file_faces(&cached_text).is_empty());
    let NodeKind::Text(cached) = &cached_node.kind else { panic!("text") };
    assert!(db.file_faces(cached).is_empty(), "a cached node is not a resource owner");
    let recovered = db.keep_file_font_up_to(&b, 1).unwrap();
    assert_eq!(recovered.file_data(), b.bytes.as_slice());
    assert_eq!(db.read_file_faces().by_id.len(), 1, "dead slots were reaped, not accumulated");
    assert_eq!(db.read_file_faces().by_key.len(), 1);
    assert_eq!(db.read_file_faces().by_blob.len(), 1);
}

#[test]
fn cloned_and_mutated_blobs_cannot_reuse_an_old_runtime_face() {
    let db = FontDb::with_font_dirs(vec![]);
    let (a, b) = (blob(200.0), blob(800.0));
    let first = db.keep_file_font(&a).unwrap();
    let mut changed = a.clone();
    assert_eq!(changed, a, "runtime ownership does not change blob equality");
    changed.bytes = b.bytes.clone();
    // Cloning must not propagate a stale lease after public bytes were replaced.
    let copied = changed.clone();
    assert!(copied.file_font.lock().unwrap().is_none());
    assert!(changed.file_font.lock().unwrap().is_none());
    let second = db.keep_file_font(&copied).unwrap();
    assert_ne!(first.id(), second.id());
    assert_eq!(second.file_data(), b.bytes.as_slice());
    assert_eq!(first.file_data(), a.bytes.as_slice());
    let mut invalid = copied.clone();
    invalid.bytes = Arc::new(b"invalid replacement".to_vec());
    assert!(db.keep_file_font(&invalid).is_none());
    assert!(invalid.file_font.lock().unwrap().is_none());
}

#[test]
fn file_font_height_references_measure_the_owned_face() {
    use vectorcraft_doc::FontHeightReference;
    let db = FontDb::with_font_dirs(vec![]);
    let owned = blob(400.0);
    let mut t = text("k");
    assert!(db.bind_file_fonts(&mut t, |_| Some(&owned)));
    let face = db.file_faces(&t)[0].clone().unwrap();
    let height = face.font_height(FontHeightReference::CapHeight, false).unwrap();
    t.runs[0].style.font_height_reference = FontHeightReference::CapHeight;
    let mut em = t.clone();
    em.runs[0].style.font_height_reference = FontHeightReference::EmBox;
    em.runs[0].style.size /= height;
    assert!((layout(&db, &t).bounds.width() - layout(&db, &em).bounds.width()).abs() < 1e-6);
    assert_eq!(t.runs[0].style.file_font_id, None, "internal layout ids never enter source runs");
}
