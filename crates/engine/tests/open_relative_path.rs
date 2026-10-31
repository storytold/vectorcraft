//! #1604: a document opened by a path relative to the working directory finds its relatively
//! linked images, as when opened by its absolute path. Its own test binary: it changes the
//! process's working directory.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_engine::Session;

#[test]
fn a_relative_path_finds_relative_links() {
    let dir = std::env::temp_dir().join(format!("vectorcraft-relative-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    std::fs::write(dir.join("ext.png"), &png).unwrap();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><image x="0" y="0" width="40" height="40" href="ext.png"/></svg>"#;
    std::fs::write(dir.join("linked.svg"), svg).unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"path": "linked.svg"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(r["warnings"], json!([]), "{r}");
    assert_eq!(r["missingLinks"], json!([]), "{r}");
}
