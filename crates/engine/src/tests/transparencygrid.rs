//! View → Show Transparency Grid is engine view state of each open document (M3.88).

use serde_json::json;

use crate::*;

#[test]
fn transparency_grid_is_per_document_view_state() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let grid = |s: &Session| s.doc().unwrap().transparency_grid;
    assert!(!grid(&s));
    assert_eq!(s.execute("view.transparencyGrid", &json!({})).unwrap(), json!({"on": true}));
    assert!(grid(&s));
    // Not an edit: nothing to undo, the document stays clean, the journal skips it.
    let st = s.doc().unwrap();
    assert!(st.history.undo.is_empty() && !st.is_dirty());
    // Another document has its own setting.
    s.execute("file.new", &json!({})).unwrap();
    assert!(!grid(&s));
    assert_eq!(s.execute("view.transparencyGrid", &json!({"on": false})).unwrap(), json!({"on": false}));
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert!(grid(&s));
    assert_eq!(s.execute("view.transparencyGrid", &json!({})).unwrap(), json!({"on": false}));
    assert_eq!(s.execute("view.transparencyGrid", &json!({"on": true})).unwrap(), json!({"on": true}));
    s.execute("document.activate", &json!({"index": 1})).unwrap();
    assert!(!grid(&s));
}
