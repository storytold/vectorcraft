//! `ui.screenshot`'s queue, admission, settling, staleness and deadline are `craft_automation`'s
//! `Captures`, tried in that crate's `tests/it/screenshot.rs`; what stays here is the app's PNG
//! writer and its reply limit (`control::save_screenshot`).

use serde_json::json;
use vectorcraft_engine::Session;

use crate::{Services, VectorcraftApp};

/// An app with a write service and a 100 by 100 document, and a temporary directory.
fn app(tag: &str) -> (VectorcraftApp, std::path::PathBuf) {
    let dir = vectorcraft_testkit::temp_dir(tag);
    let services =
        Services { write: Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    (app, dir)
}

/// An oversized claimed image is refused from the reply-encoding budget before any copy or encode.
#[test]
fn a_data_reply_over_the_encoding_budget_is_refused_before_encoding() {
    let (mut app, _dir) = app("screenshot-encoding");
    // A huge claimed image with no pixels: the guard must look at the checked size before copying
    // or encoding anything. (A real 100 000 × 100 000 ColorImage would already be impossible.)
    let image = egui::ColorImage { size: [100_000, 100_000], source_size: egui::Vec2::ZERO, pixels: vec![] };
    let answer = crate::control::save_screenshot(&mut app, &image, None, true);
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("reply-encoding limit"), "{answer}");
}

/// A path plus data capture writes the file and omits an oversized inline PNG; without a path the
/// same request fails.
#[test]
fn a_path_plus_data_capture_writes_the_file_and_omits_an_oversized_inline_png() {
    let (mut app, dir) = app("screenshot-path-data");
    let image = egui::ColorImage::filled([2, 2], egui::Color32::RED);
    let path = dir.join("shot.png");
    let path_text = path.to_string_lossy().into_owned();
    let answer = crate::control::save_screenshot_with_limit(&mut app, &image, Some(&path_text), true, 1);
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["path"], json!(path_text), "{answer}");
    assert_eq!(answer["result"]["width"], json!(2), "{answer}");
    assert_eq!(answer["result"]["height"], json!(2), "{answer}");
    assert!(answer["result"].get("pngBase64").is_none(), "the oversized inline copy is downgraded, not the file: {answer}");
    assert!(path.is_file(), "the named file is still written");

    // Without a path there is no delivery channel left, so that request still fails before it
    // copies or encodes the image.
    let answer = crate::control::save_screenshot_with_limit(&mut app, &image, None, true, 1);
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("reply-encoding limit"), "{answer}");
    let _ = std::fs::remove_dir_all(dir);
}
