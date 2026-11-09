//! Safe Mode and disabled plug-ins: in Safe Mode no plug-in loads or runs (one installed before it
//! was turned on included), and loading a folder skips the plug-ins the user disabled. Safe Mode is
//! process-wide, so these tests run in their own test binary, one at a time.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Mutex;

use serde_json::json;
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_plugins::safe_mode::{self, Item, Source};
use vectorcraft_plugins::{Error, Limits, Plugin, effect, registry, wat as tpl};

static LOCK: Mutex<()> = Mutex::new(());

/// Safe Mode off, nothing disabled or installed.
fn reset() {
    safe_mode::set_active(None);
    safe_mode::set_disabled(vec![]);
    for p in registry::list() {
        registry::remove(p.id());
    }
}

/// Holds the lock for one test and resets the process-wide state when the test starts and when it
/// ends, so a test that fails leaves the next one as it would find a new process.
struct Fresh {
    _guard: std::sync::MutexGuard<'static, ()>,
}

fn fresh() -> Fresh {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    Fresh { _guard: guard }
}

impl Drop for Fresh {
    fn drop(&mut self) {
        reset();
    }
}

fn build(src: &str) -> Vec<u8> {
    wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{src}"))
}

fn filter(id: &str) -> Vec<u8> {
    build(&tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#))
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-safe-mode-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn in_safe_mode_nothing_loads_or_runs() {
    let _f = fresh();
    // Installed before Safe Mode was turned on.
    let before = registry::install(Plugin::load(&filter("test.safe.before"), Limits::default()).unwrap());
    let fx = build(&tpl::returning(&tpl::manifest("test.safe.effect", "effect", "{}"), r#"{"objects":[]}"#));
    registry::install(Plugin::load(&fx, Limits::default()).unwrap());
    let dir = temp_dir("gate");
    std::fs::write(dir.join("a.wasm"), filter("test.safe.folder")).unwrap();

    safe_mode::set_active(Some(Source::Flag));
    assert_eq!((safe_mode::active(), safe_mode::source()), (true, Some(Source::Flag)));
    assert_eq!(Plugin::load(&filter("test.safe.new"), Limits::default()).err(), Some(Error::SafeMode));
    assert_eq!(before.run(br#"{"objects":[]}"#, b"{}"), Err(Error::SafeMode), "an installed plug-in can't run either");
    assert_eq!(registry::load_folder(&dir), Err(Error::SafeMode));
    assert!(registry::get("test.safe.folder").is_none());
    let path = PathData::default();
    let drawn =
        effect::apply("test.safe.effect", &json!({}), &path, Rect::new(0.0, 0.0, 1.0, 1.0), FillRule::NonZero, effect::Paints::default(), None);
    assert_eq!(drawn, None, "the geometry is drawn unchanged");
    assert_eq!(effect::last_error("test.safe.effect"), None, "not a failure of the plug-in");
    assert_eq!(Error::SafeMode.to_string(), "Safe Mode is on: plug-ins don't load or run until VectorCraft starts normally");

    safe_mode::set_active(None);
    assert!(before.run(br#"{"objects":[]}"#, b"{}").is_ok(), "nothing was uninstalled");
    assert_eq!(registry::load_folder(&dir).unwrap().loaded, ["test.safe.folder"]);
    registry::remove("test.safe.before");
    registry::remove("test.safe.effect");
    registry::remove("test.safe.folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loading_a_folder_skips_disabled_plug_ins_by_file_or_id() {
    let _f = fresh();
    let dir = temp_dir("disabled");
    let (x, y, z) = (dir.join("x.wasm"), dir.join("y.wasm"), dir.join("z.wasm"));
    std::fs::write(&x, filter("test.disabled.x")).unwrap();
    std::fs::write(&y, filter("test.disabled.y")).unwrap();
    std::fs::write(&z, filter("test.disabled.z")).unwrap();
    safe_mode::set_disabled(vec![Item::file(x.display().to_string()), Item { id: Some("test.disabled.z".into()), ..Item::default() }]);
    let report = registry::load_folder(&dir).unwrap();
    assert_eq!(report.loaded, ["test.disabled.y"]);
    let failed: Vec<(&str, &str)> = report.failed.iter().map(|(f, e)| (f.as_str(), e.as_str())).collect();
    assert_eq!(failed, [(x.display().to_string().as_str(), registry::DISABLED), (z.display().to_string().as_str(), registry::DISABLED)]);
    assert!(registry::get("test.disabled.x").is_none() && registry::get("test.disabled.z").is_none());
    safe_mode::set_disabled(vec![]);
    registry::remove("test.disabled.y");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_module_disabled_by_id_alone_runs_no_code_on_the_next_folder_load() {
    let _f = fresh();
    let dir = temp_dir("by-id");
    let w = dir.join("w.wasm");
    std::fs::write(&w, filter("test.byid.w")).unwrap();
    let by_id = Item { id: Some("test.byid.w".into()), name: Some("W".into()), ..Item::default() };
    safe_mode::set_disabled(vec![by_id]);
    let first = registry::load_folder(&dir).unwrap();
    assert_eq!(first.failed, [(w.display().to_string(), registry::DISABLED.to_string())]);
    assert!(first.disabled_changed, "the app saves the list with the file");
    assert_eq!(safe_mode::disabled().len(), 1, "the entry takes the file; no second entry");
    assert_eq!(safe_mode::disabled()[0].file.as_deref(), Some(w.display().to_string().as_str()));
    // A module that fails as soon as it loads: the file is skipped before any of it runs.
    std::fs::write(&w, b"not a module").unwrap();
    let second = registry::load_folder(&dir).unwrap();
    assert_eq!(second.failed, [(w.display().to_string(), registry::DISABLED.to_string())]);
    assert!(!second.disabled_changed);
    // Turned back on for the next launch: loading doesn't disable its file again for that launch.
    std::fs::write(&w, filter("test.byid.w")).unwrap();
    safe_mode::set_disabled(vec![Item { id: Some("test.byid.w".into()), ..Item::default() }]);
    assert!(safe_mode::set_enabled(&Item { id: Some("test.byid.w".into()), ..Item::default() }, true));
    let third = registry::load_folder(&dir).unwrap();
    assert!(!third.disabled_changed && safe_mode::disabled().is_empty());
    assert!(safe_mode::skipped(&Item::file(w.display().to_string())), "this launch still skips it");
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}
