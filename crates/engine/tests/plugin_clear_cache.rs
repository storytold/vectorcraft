//! `plugin.clearCache` without an id empties the cache of every installed plug-in and leaves their
//! settings.
//!
//! A test binary of its own: installed plug-ins are process-wide, so clearing every plug-in's cache
//! in the engine's other tests would empty the caches those tests check.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

/// Sets "zoom" to "hello" and writes "hel" to the cache entry "tile".
const WRITES: &str = "(if (call $vc_settings_set (i32.const 0x2000) (i32.const 4) (i32.const 0x2100) (i32.const 5)) (then unreachable))
    (if (call $vc_cache_write (i32.const 0x2004) (i32.const 4) (i32.const 0x2100) (i32.const 3)) (then unreachable))";

/// Traps unless "zoom" is "hello".
const READS: &str =
    "(if (i32.ne (call $vc_settings_get (i32.const 0x2000) (i32.const 4) (i32.const 0x3000) (i32.const 16)) (i32.const 5)) (then unreachable))
    (if (i32.ne (i32.load (i32.const 0x3000)) (i32.load (i32.const 0x2100))) (then unreachable))";

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// Installs a filter `id` of API level 2 that lists `settings` and `cache` and whose `vc_run` runs
/// `first`, in place of any module installed with its id (`replace: true`).
fn install_storing(s: &mut Session, id: &str, first: &str) {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["settings","cache"]}}"#);
    let data = format!(r#"{} (data (i32.const 0x2000) "zoomtile") (data (i32.const 0x2100) "hello")"#, tpl::STORAGE_IMPORTS);
    let wasm = wat::parse_str(tpl::with_imports(&manifest, &data, first, r#"{"objects":[]}"#)).unwrap();
    let data = vectorcraft_format::base64_encode(&wasm);
    run(s, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"], "replace": true}));
}

#[test]
fn clearing_every_cache_empties_each_plugins_cache_and_keeps_its_settings() {
    let folder = std::env::temp_dir().join(format!("vc-plugin-clear-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let ids = ["test.clearcache.a", "test.clearcache.b"];
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s.attach_plugin_store(folder.clone());
    for id in ids {
        install_storing(&mut s, id, WRITES);
        run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
        assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3));
    }

    assert_eq!(run(&mut s, "plugin.clearCache", json!({})), json!({"cleared": ids, "bytesFreed": 6}));
    for id in ids {
        assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(0));
        // The settings stay: a module that reads them runs.
        install_storing(&mut s, id, READS);
        run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    }
    // Nothing is left to clear.
    assert_eq!(run(&mut s, "plugin.clearCache", json!({})), json!({"cleared": ids, "bytesFreed": 0}));

    for id in ids {
        run(&mut s, "plugin.remove", json!({"id": id}));
    }
    let _ = std::fs::remove_dir_all(&folder);
}
