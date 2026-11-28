//! Ruler guides copied alone and the system clipboard (#3226 review): a guides-only Copy empties
//! the system clipboard, as any Copy replaces what it held, so the next Paste takes the guides and
//! not what another app copied before. What another app copies after it still wins.

use std::sync::{Arc, Mutex};

use craft::platform::clipboard::{EventClipboard, Flavor};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::{PNG, TEXT};

use crate::tests_sysclip::{UNDECODED, app, app_with_busy_worker, copy_elsewhere, copy_rect, count, cut_short_png, frame, pasted, run, written};
use crate::{Services, VectorcraftApp, background};

fn guides(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().doc.ruler_guides().len()
}

/// Add a vertical guide at `pos`, select only it and copy it.
fn copy_guide(app: &mut VectorcraftApp, pos: f64) {
    run(app, "guide.add", json!({"vertical": true, "pos": pos}));
    let last = guides(app) - 1;
    run(app, "select.none", json!({}));
    run(app, "guide.select", json!({"indexes": [last]}));
    run(app, "edit.copy", json!({}));
}

#[test]
fn a_guides_only_copy_pastes_the_guides_not_what_another_app_copied_before() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec()), (PNG, vec![1, 2, 3])]);
    copy_guide(&mut app, 120.0);
    assert!(written(&board).is_empty(), "the Copy emptied the system clipboard: {:?}", written(&board));
    run(&mut app, "edit.paste", json!({}));
    assert_eq!(guides(&app), 2, "the copied guide was pasted");
    assert_eq!(count(&app), 0, "and not the other app's text");
    let st = app.session.active().unwrap();
    assert_eq!(vectorcraft_engine::cmd::guide_indexes(&st.doc, &st.selection.guides), [1]);
    // A second Paste takes the guides again.
    run(&mut app, "edit.paste", json!({}));
    assert_eq!((guides(&app), count(&app)), (3, 0));
}

/// A guides-only Copy has no notes: it clears the message an earlier Copy's notes left.
#[test]
fn a_guides_only_copy_clears_an_earlier_copys_notes() {
    let (mut app, _) = app();
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &cut_short_png(), json!({}));
    run(&mut app, "edit.copy", json!({}));
    background::wait_all(&mut app);
    assert_eq!(app.ui.status, format!("Copied with 1 note: {UNDECODED}"));
    copy_guide(&mut app, 120.0);
    assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), ("", &None));
}

#[test]
fn what_another_app_copies_after_a_guides_only_copy_is_pasted() {
    let (mut app, board) = app();
    copy_guide(&mut app, 120.0);
    copy_elsewhere(&board, vec![(TEXT, b"Copied later".to_vec())]);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("not type") };
    assert_eq!(t.plain_text(), "Copied later");
    assert_eq!(guides(&app), 1, "no guide pasted");
}

#[test]
fn objects_copied_after_guides_paste_as_objects() {
    let (mut app, board) = app();
    copy_guide(&mut app, 120.0);
    run(&mut app, "select.none", json!({}));
    copy_rect(&mut app);
    assert!(!written(&board).is_empty(), "objects go to the system clipboard");
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }));
    assert_eq!(guides(&app), 1, "the earlier guides-only copy is replaced");
}

/// A guides-only Copy right after an object Copy whose formats are still being made: those formats
/// are dropped, so they don't land on the system clipboard after it and replace the guides.
#[test]
fn a_guides_only_copy_drops_an_earlier_copys_formats_not_written_yet() {
    let (mut app, board, hold) = app_with_busy_worker("");
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
    copy_rect(&mut app);
    copy_guide(&mut app, 120.0);
    drop(hold);
    background::wait_all(&mut app);
    assert!(written(&board).is_empty(), "{:?}", written(&board));
    let objects = count(&app);
    run(&mut app, "edit.paste", json!({}));
    assert_eq!((guides(&app), count(&app)), (2, objects), "the guide, not the rectangle or the text");
}

/// The web (a clipboard read only inside a paste event): a guides-only Copy puts empty text on the
/// clipboard, in place of the SVG an object Copy puts there, so another app's SVG copied before is
/// not pasted instead of the guides.
#[test]
fn on_the_web_a_guides_only_copy_empties_the_clipboard_text() {
    let sent = Arc::new(Mutex::new(Vec::<Flavor>::new()));
    let out = sent.clone();
    let clipboard = EventClipboard::new(
        || 0,
        move |flavors| {
            out.lock().unwrap().extend_from_slice(flavors);
            Ok(0)
        },
    );
    let mut app = VectorcraftApp::new(Session::new(), Services { clipboard: Some(Box::new(clipboard)), ..Default::default() });
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    copy_guide(&mut app, 120.0);
    frame(&mut app, vec![]);
    let texts: Vec<Vec<u8>> = sent.lock().unwrap().iter().filter(|f| f.mime == TEXT).map(|f| f.data.clone()).collect();
    assert_eq!(texts, [Vec::<u8>::new()], "empty text replaces the browser's clipboard");
    run(&mut app, "edit.paste", json!({}));
    assert_eq!((guides(&app), count(&app)), (2, 0), "the guide, not the other app's SVG");
}
