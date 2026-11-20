//! Save errors retain their type without changing the other filesystem writers (#6146).

use std::io::{self, ErrorKind};

use serde_json::json;

use super::{SaveMode, access, save_plan, save_with, write_file_with, write_save_file_with};
use crate::{EngineError, READ_ONLY_LOCATION, Session};

#[test]
fn readonly_error_is_typed_and_other_writers_keep_their_error() {
    let dir = vectorcraft_testkit::TempDir::new("save-refusal");
    let path = dir.join("Poster.vectorcraft").to_string_lossy().into_owned();
    for kind in [ErrorKind::PermissionDenied, ErrorKind::ReadOnlyFilesystem, ErrorKind::StorageFull, ErrorKind::NotFound, ErrorKind::Other] {
        let typed = write_save_file_with(&path, &mut |_| Err(io::Error::new(kind, "injected"))).unwrap_err();
        let ordinary = write_file_with(&path, &mut |_| Err(io::Error::new(kind, "injected"))).unwrap_err();
        let reason = format!("{path}: injected");
        assert!(matches!(&ordinary, EngineError::Other(_)));
        assert_eq!(ordinary.to_string(), reason);
        if matches!(kind, ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem) {
            assert!(matches!(&typed, EngineError::ReadOnlyLocation(_)));
            assert_eq!(typed.to_string(), format!("{READ_ONLY_LOCATION}: {reason}"));
        } else {
            assert!(matches!(&typed, EngineError::Other(_)));
            assert_eq!(typed.to_string(), reason);
        }
        assert!(!std::path::Path::new(&path).exists());
    }
}

#[test]
fn readonly_failed_save_keeps_document_and_options() {
    let mut session = Session::new();
    session.execute("file.new", &json!({"width": 40, "height": 40})).unwrap();
    session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let st = session.doc_mut().unwrap();
    st.path = Some("Poster.svg".into());
    st.format = "svg";
    st.save_options = json!({"decimals": 3}).as_object().unwrap().clone();
    let before = serde_json::to_value(&*st.doc).unwrap();
    let options = st.save_options.clone();
    let plan = save_plan(&session, SaveMode::Save, &json!({"modified": null})).unwrap();
    let e = save_with(&mut session, plan, |path, _| Err(access::save_write_error(path, io::Error::from(ErrorKind::PermissionDenied)))).unwrap_err();
    assert!(matches!(e, EngineError::ReadOnlyLocation(_)));
    let st = session.doc().unwrap();
    assert_eq!(st.path.as_deref(), Some("Poster.svg"));
    assert_eq!(st.format, "svg");
    assert_eq!(st.save_options, options);
    assert!(st.is_dirty());
    assert_eq!(serde_json::to_value(&*st.doc).unwrap(), before);
}
