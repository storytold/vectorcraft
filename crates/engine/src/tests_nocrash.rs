//! Never crash: every command, given hostile parameters and a stale selection, returns an error
//! instead of panicking.

use serde_json::{Value, json};

use super::*;

/// A document with a few kinds of object. `stale` 1 adds selected ids that don't exist; 2 selects
/// only such ids.
fn session(stale: u8) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 80, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("shape.line", &json!({"x1": 10, "y1": 100, "x2": 120, "y2": 140})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    if stale == 2 {
        s.execute("select.none", &json!({})).unwrap();
    }
    if stale > 0 {
        s.execute("select.add", &json!({"ids": [999_998, 999_999]})).unwrap();
    }
    s
}

fn hostile_params() -> Vec<Value> {
    vec![
        json!({}),
        json!({"id": 999_999, "ids": [999_999, 999_998]}),
        json!({"points": [], "index": 4_000_000_000u64, "count": -1, "rows": 0, "cols": 0, "scale": 0}),
        json!({"points": [[1, 1]], "index": -1, "width": -5, "height": 1e300, "radius": f64::MAX}),
        json!({"text": "", "start": 10, "end": 2, "name": "", "path": [], "events": []}),
    ]
}

/// Commands that touch the file system or the clock rather than the document.
fn skip(id: &str) -> bool {
    id.starts_with("file.open") || id.starts_with("file.save") || id.starts_with("file.export") || id.starts_with("file.place") || id == "file.revert"
}

#[test]
fn no_command_panics_on_hostile_input() {
    let ids: Vec<String> = Session::new().commands().into_iter().map(|c| c.id.to_string()).collect();
    let mut panics = vec![];
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for id in &ids {
        if skip(id) {
            continue;
        }
        for stale in [0, 1, 2] {
            for p in hostile_params() {
                let mut s = session(stale);
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = s.execute(id, &p);
                }));
                if r.is_err() {
                    panics.push(format!("{id} stale={stale} {p}"));
                }
            }
        }
    }
    std::panic::set_hook(hook);
    assert!(ids.len() > 300, "only {} commands", ids.len());
    assert!(panics.is_empty(), "{} panics:\n{}", panics.len(), panics.join("\n"));
}
