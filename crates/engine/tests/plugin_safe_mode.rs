//! Safe Mode and disabled plug-ins through the engine: `plugin.safeMode`, `plugin.setEnabled`, the
//! plug-in folder skipping disabled plug-ins at the next launch, and the `plugin.*` commands in Safe
//! Mode.
//!
//! A test binary of its own: Safe Mode and installed plug-ins are process-wide, so these tests run
//! one at a time and no other test sees them.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::safe_mode;
use vectorcraft_plugins::wat as tpl;

static LOCK: Mutex<()> = Mutex::new(());

/// Safe Mode off, nothing disabled or installed.
fn reset() {
    safe_mode::set_active(None);
    safe_mode::set_disabled(vec![]);
    for p in vectorcraft_plugins::registry::list() {
        vectorcraft_plugins::registry::remove(p.id());
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

fn filter(id: &str) -> Vec<u8> {
    wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-plugin-safe-mode-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn installed(id: &str) -> bool {
    vectorcraft_plugins::registry::get(id).is_some()
}

/// A new launch: nothing installed, the plug-in folder `dir` attached.
fn launch(dir: &std::path::Path) -> Session {
    for p in vectorcraft_plugins::registry::list() {
        vectorcraft_plugins::registry::remove(p.id());
    }
    let mut s = Session::new();
    s.attach_plugin_store(dir.to_path_buf());
    s
}

#[test]
fn a_disabled_plug_in_stays_installed_and_loads_again_once_enabled_at_the_next_launch() {
    let _f = fresh();
    let dir = tmp_dir("enable");
    let mut s = launch(&dir);
    for id in ["test.sm.x", "test.sm.y"] {
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter(id))})).unwrap();
    }
    let r = s.execute("plugin.setEnabled", &json!({"id": "test.sm.x", "enabled": false})).unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(r["disabled"][0]["id"], "test.sm.x");
    assert!(installed("test.sm.x"), "disabling waits for the next launch");
    let again = s.execute("plugin.setEnabled", &json!({"id": "test.sm.x", "enabled": false})).unwrap();
    assert_eq!(again["changed"], false, "already disabled: nothing changes");

    let mut s = launch(&dir);
    assert!(!installed("test.sm.x") && installed("test.sm.y"), "only the disabled one is skipped");
    assert!(dir.read_dir().unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "wasm")).count() == 2, "nothing uninstalled");
    let list = s.execute("plugin.list", &json!({})).unwrap();
    assert_eq!((list["safeMode"].clone(), list["disabled"][0]["id"].clone()), (json!(false), json!("test.sm.x")));
    let state = s.execute("plugin.safeMode", &json!({})).unwrap();
    assert_eq!(state, json!({"active": false, "source": null, "pendingItem": null, "disabled": state["disabled"].clone()}));
    assert_eq!(state["disabled"].as_array().map(Vec::len), Some(1));

    s.execute("plugin.setEnabled", &json!({"id": "test.sm.x", "enabled": true})).unwrap();
    assert!(!installed("test.sm.x"), "enabling waits for the next launch");
    launch(&dir);
    assert!(installed("test.sm.x") && installed("test.sm.y"));
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two windows share a plug-in folder: a plug-in the other one disabled after this one started stays
/// disabled when this one saves its own changes.
#[test]
fn saving_the_disabled_list_keeps_what_another_window_saved_meanwhile() {
    let _f = fresh();
    let dir = tmp_dir("two-windows");
    let mut s = launch(&dir);
    s.execute("plugin.setEnabled", &json!({"id": "test.sm.a", "enabled": false})).unwrap();
    // The other window disabled b and saved its list.
    let other = |ids: &[&str]| {
        let items: Vec<Value> = ids.iter().map(|id| json!({"id": id})).collect();
        let file = json!({"format": "vectorcraft-disabled-plugins", "version": 1, "items": items});
        std::fs::write(dir.join(vectorcraft_engine::store::DISABLED_FILE), serde_json::to_vec(&file).unwrap()).unwrap();
    };
    other(&["test.sm.a", "test.sm.b"]);
    s.execute("plugin.setEnabled", &json!({"id": "test.sm.c", "enabled": false})).unwrap();
    s.execute("plugin.setEnabled", &json!({"id": "test.sm.a", "enabled": true})).unwrap();
    let saved = |s: &mut Session| -> Vec<String> {
        let state = s.execute("plugin.safeMode", &json!({})).unwrap();
        state["disabled"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(saved(&mut s), ["test.sm.b", "test.sm.c"], "b is kept, c added, and a turned on again");
    // The other window turns b back on; this window's next change keeps that too.
    other(&["test.sm.c"]);
    s.execute("plugin.setEnabled", &json!({"id": "test.sm.d", "enabled": false})).unwrap();
    assert_eq!(saved(&mut s), ["test.sm.c", "test.sm.d"]);
    let mut next = launch(&dir);
    assert_eq!(saved(&mut next), ["test.sm.c", "test.sm.d"], "the file holds the merged list");
    // While the lock file can't be locked (here a folder stands in its place), nothing is written;
    // the next change that is written carries this one too.
    let lock = dir.join(vectorcraft_engine::store::DISABLED_LOCK);
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir_all(&lock).unwrap();
    let in_file = || -> Vec<String> {
        let bytes = std::fs::read(dir.join(vectorcraft_engine::store::DISABLED_FILE)).unwrap();
        safe_mode::parse_disabled_file(&bytes).into_iter().filter_map(|i| i.id).collect()
    };
    next.execute("plugin.setEnabled", &json!({"id": "test.sm.e", "enabled": false})).unwrap();
    assert_eq!(in_file(), ["test.sm.c", "test.sm.d"]);
    std::fs::remove_dir(&lock).unwrap();
    next.execute("plugin.setEnabled", &json!({"id": "test.sm.f", "enabled": false})).unwrap();
    assert_eq!(in_file(), ["test.sm.c", "test.sm.d", "test.sm.e", "test.sm.f"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_full_list_of_long_disabled_items_is_read_back_at_the_next_launch() {
    let _f = fresh();
    let dir = tmp_dir("full-list");
    let field = |i: usize, tag: &str| format!("{tag}{i:04}{}", "\u{1}".repeat(1024));
    let items: Vec<safe_mode::Item> = (0..safe_mode::MAX_DISABLED)
        .map(|i| safe_mode::Item { id: Some(field(i, "i")), file: Some(field(i, "f")), name: Some(field(i, "n")) })
        .collect();
    safe_mode::set_disabled(items);
    let saved = safe_mode::disabled();
    std::fs::write(dir.join(vectorcraft_engine::store::DISABLED_FILE), safe_mode::disabled_file()).unwrap();
    safe_mode::set_disabled(vec![]);
    launch(&dir);
    assert_eq!(safe_mode::disabled(), saved, "every disabled item stays disabled");
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_plug_in_disabled_by_id_is_saved_with_its_file() {
    let _f = fresh();
    let dir = tmp_dir("by-id");
    let extra = tmp_dir("by-id-extra");
    let f = extra.join("f.wasm");
    std::fs::write(&f, filter("test.sm.f")).unwrap();
    let mut s = launch(&dir);
    s.execute("plugin.setEnabled", &json!({"id": "test.sm.f", "enabled": false})).unwrap();
    let mut s = launch(&dir);
    let r = s.execute("plugin.reload", &json!({"path": extra.to_string_lossy()})).unwrap();
    assert_eq!(r["failed"][0]["error"], vectorcraft_plugins::registry::DISABLED);
    safe_mode::set_disabled(vec![]);
    launch(&dir);
    assert_eq!(safe_mode::disabled()[0].file.as_deref(), Some(f.display().to_string().as_str()), "the next launch skips the file before it loads");
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&extra);
}

#[test]
fn installing_a_disabled_plug_in_again_turns_it_on() {
    let _f = fresh();
    let dir = tmp_dir("reinstall");
    let mut s = launch(&dir);
    s.execute("plugin.setEnabled", &json!({"id": "test.sm.r", "enabled": false})).unwrap();
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter("test.sm.r"))})).unwrap();
    assert!(safe_mode::disabled().is_empty());
    launch(&dir);
    assert!(installed("test.sm.r"));
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// When the grant of an update can't be stored, the store keeps the old module, and it stays disabled.
#[test]
fn a_disabled_plug_in_whose_new_grant_cant_be_stored_stays_disabled() {
    let _f = fresh();
    let dir = tmp_dir("reinstall-no-grant");
    let mut s = launch(&dir);
    s.allow_plugin_grants_from_requests();
    let manifest = r#"{"id":"test.sm.g","name":"Test","version":"1","kind":"filter","permissions":["doc.read"]}"#;
    let module = |out: &str| vectorcraft_format::base64_encode(&wat::parse_str(tpl::returning(manifest, out)).unwrap());
    s.execute("plugin.install", &json!({"dataBase64": module(r#"{"objects":[]}"#), "grant": ["doc.read"]})).unwrap();
    s.execute("plugin.setEnabled", &json!({"id": "test.sm.g", "enabled": false})).unwrap();
    let grants = dir.join("grants.json");
    std::fs::remove_file(&grants).unwrap();
    std::fs::create_dir_all(grants.join("block")).unwrap();
    s.execute("plugin.install", &json!({"dataBase64": module(r#"{"objects":[],"error":null}"#), "grant": ["doc.read"], "replace": true})).unwrap();
    assert_eq!(safe_mode::disabled().len(), 1, "the module the update could not replace stays disabled");
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stored module that lists no permissions and isn't read, because it is disabled or Safe Mode is
/// on, leaves the grant for its id in the store. The load that reads it forgets the grant.
#[test]
fn a_stored_module_that_is_not_read_keeps_the_grant_for_its_id() {
    let _f = fresh();
    let dir = tmp_dir("unread-grant");
    let id = "test.sm.unread";
    let mut s = launch(&dir);
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter(id))})).unwrap();
    let grants = dir.join("grants.json");
    let entry = serde_json::to_vec(&json!([{"id": id, "hash": "0".repeat(64), "permissions": ["doc.read"]}])).unwrap();
    std::fs::write(&grants, &entry).unwrap();

    s.execute("plugin.setEnabled", &json!({"id": id, "enabled": false})).unwrap();
    let mut s = launch(&dir);
    assert!(!installed(id), "the disabled module is skipped");
    assert_eq!(std::fs::read(&grants).unwrap(), entry, "a disabled module keeps the grant");

    s.execute("plugin.setEnabled", &json!({"id": id, "enabled": true})).unwrap();
    vectorcraft_engine::cmd::plugin::start_in_safe_mode();
    let mut s = launch(&dir);
    assert!(!installed(id), "Safe Mode loads nothing");
    assert_eq!(std::fs::read(&grants).unwrap(), entry, "Safe Mode keeps the grant");

    safe_mode::set_active(None);
    vectorcraft_engine::cmd::plugin::load_prefs_folder(&mut s);
    assert!(installed(id));
    assert!(!grants.exists(), "the load that reads the module forgets the grant");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A launch where the stored plug-in `id` stopped while loading: it is installed, then disabled by
/// its stored file alone (as Disable and Continue saves it), and the next launch skips it.
fn stored_plug_in_disabled_by_file(dir: &std::path::Path, id: &str) -> Session {
    let mut s = launch(dir);
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter(id))})).unwrap();
    let stored = dir.read_dir().unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "wasm")).unwrap();
    s.execute("plugin.setEnabled", &json!({"file": stored.display().to_string(), "enabled": false})).unwrap();
    let s = launch(dir);
    assert!(!installed(id), "the next launch skips the file");
    assert_eq!(safe_mode::disabled()[0].id, None);
    s
}

fn nothing_disabled(s: &mut Session, dir: &std::path::Path) {
    assert_eq!(s.execute("plugin.safeMode", &json!({})).unwrap()["disabled"], json!([]));
    let saved = std::fs::read(dir.join(vectorcraft_engine::store::DISABLED_FILE)).unwrap();
    assert!(safe_mode::parse_disabled_file(&saved).is_empty(), "nothing disabled is saved");
}

#[test]
fn installing_a_stored_plug_in_disabled_by_its_file_turns_it_on() {
    let _f = fresh();
    let dir = tmp_dir("reinstall-file");
    let mut s = stored_plug_in_disabled_by_file(&dir, "test.sm.p");
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter("test.sm.p"))})).unwrap();
    nothing_disabled(&mut s, &dir);
    launch(&dir);
    assert!(installed("test.sm.p"));
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stored_plug_in_disabled_by_its_file_stays_disabled_when_its_update_cannot_be_saved() {
    let _f = fresh();
    let dir = tmp_dir("reinstall-unsaved");
    let mut s = stored_plug_in_disabled_by_file(&dir, "test.sm.u");
    let stored = PathBuf::from(safe_mode::disabled()[0].file.clone().unwrap());
    std::fs::remove_file(&stored).unwrap();
    std::fs::create_dir_all(stored.join("block")).unwrap();
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter("test.sm.u"))})).unwrap();
    assert_eq!(safe_mode::disabled().len(), 1, "the file the update could not replace stays disabled");
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn enabling_a_stored_plug_in_disabled_by_its_file_by_its_id_turns_it_on() {
    let _f = fresh();
    let dir = tmp_dir("enable-file");
    let mut s = stored_plug_in_disabled_by_file(&dir, "test.sm.q");
    let r = s.execute("plugin.setEnabled", &json!({"id": "test.sm.q", "enabled": true})).unwrap();
    assert_eq!(r["changed"], true);
    nothing_disabled(&mut s, &dir);
    launch(&dir);
    assert!(installed("test.sm.q"));
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_enabled_rejects_bad_parameters() {
    let _f = fresh();
    safe_mode::set_disabled(vec![]);
    let mut s = Session::new();
    let err = |s: &mut Session, p: Value| s.execute("plugin.setEnabled", &p).unwrap_err().to_string();
    assert!(err(&mut s, json!({"id": "a"})).contains("`enabled` must be true or false"));
    assert!(err(&mut s, json!({"id": "a", "enabled": "yes"})).contains("`enabled` must be true or false"));
    assert!(err(&mut s, json!({"id": 3, "enabled": false})).contains("`id` must be a string"));
    assert!(err(&mut s, json!({"enabled": false})).contains("give `id` or `file`"));
    assert!(err(&mut s, json!({"id": "  ", "enabled": false})).contains("give `id` or `file`"));
    assert!(err(&mut s, json!({"id": "a", "enabled": true})).contains("isn't disabled"));
    assert!(safe_mode::disabled().is_empty(), "nothing changed");
    let r = s.execute("plugin.setEnabled", &json!({"file": "/p/a.wasm", "enabled": false})).unwrap();
    assert_eq!(
        (r["changed"].clone(), r["disabled"][0]["file"].clone(), r["disabled"][0]["label"].clone()),
        (json!(true), json!("/p/a.wasm"), json!("a.wasm"))
    );
    assert_eq!(r["saved"], false, "a session without a plug-in folder can't keep the change for the next launch");
    safe_mode::set_disabled(vec![]);
}

/// A disabled list too large to read, or one of a later version, stays as it is: the changes wait
/// for a save that can merge them. A file that isn't JSON holds nothing to keep and is replaced.
#[test]
fn a_disabled_list_this_version_cannot_read_is_kept() {
    let _f = fresh();
    let dir = tmp_dir("kept");
    let file = dir.join(vectorcraft_engine::store::DISABLED_FILE);
    let mut s = launch(&dir);
    let later = serde_json::to_vec(&json!({"format": "vectorcraft-disabled-plugins", "version": 2, "items": [{"id": "test.sm.k"}]})).unwrap();
    let large = vec![b' '; safe_mode::MAX_DISABLED_FILE_BYTES as usize + 1];
    for (kept, id) in [(later, "test.sm.a"), (large, "test.sm.b")] {
        std::fs::write(&file, &kept).unwrap();
        let r = s.execute("plugin.setEnabled", &json!({"id": id, "enabled": false})).unwrap();
        assert_eq!((r["changed"].clone(), r["saved"].clone()), (json!(true), json!(false)), "{id}");
        assert!(std::fs::read(&file).unwrap() == kept, "{id}: the file is kept");
    }
    std::fs::write(&file, b"{\"format\": \"vectorcraft-disab").unwrap();
    let r = s.execute("plugin.setEnabled", &json!({"id": "test.sm.c", "enabled": false})).unwrap();
    assert_eq!(r["saved"], true);
    let ids: Vec<String> = safe_mode::parse_disabled_file(&std::fs::read(&file).unwrap()).into_iter().filter_map(|i| i.id).collect();
    assert_eq!(ids, ["test.sm.a", "test.sm.b", "test.sm.c"], "the waiting changes are saved too");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Another window filled the list after this one read it: a plug-in turned off here has no room in
/// the file, and `saved` says so.
#[test]
fn a_disable_that_finds_the_saved_list_full_is_not_saved() {
    let _f = fresh();
    let dir = tmp_dir("full-merge");
    let mut s = launch(&dir);
    let items: Vec<Value> = (0..safe_mode::MAX_DISABLED).map(|i| json!({"id": format!("test.sm.full{i}")})).collect();
    let file = json!({"format": "vectorcraft-disabled-plugins", "version": 1, "items": items});
    std::fs::write(dir.join(vectorcraft_engine::store::DISABLED_FILE), serde_json::to_vec(&file).unwrap()).unwrap();
    let r = s.execute("plugin.setEnabled", &json!({"id": "test.sm.late", "enabled": false})).unwrap();
    assert_eq!((r["changed"].clone(), r["saved"].clone()), (json!(true), json!(false)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_safe_mode_the_plug_in_folder_and_commands_load_and_run_nothing() {
    let _f = fresh();
    let dir = tmp_dir("safe");
    let mut s = launch(&dir);
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter("test.sm.s"))})).unwrap();
    let extra = tmp_dir("safe-extra");
    std::fs::write(extra.join("e.wasm"), filter("test.sm.e")).unwrap();

    vectorcraft_engine::cmd::plugin::start_in_safe_mode();
    let mut s = launch(&dir);
    assert!(!installed("test.sm.s"), "stored plug-ins don't load");
    let safe = "Safe Mode is on: plug-ins don't load or run until VectorCraft starts normally";
    let install = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter("test.sm.t"))}));
    assert!(install.unwrap_err().to_string().contains(safe));
    let reload = s.execute("plugin.reload", &json!({"path": extra.to_string_lossy()}));
    assert!(reload.unwrap_err().to_string().contains(safe));
    assert!(!installed("test.sm.e") && !installed("test.sm.t"));
    let state = s.execute("plugin.safeMode", &json!({})).unwrap();
    assert_eq!((state["active"].clone(), state["source"].clone()), (json!(true), json!("flag")));
    assert_eq!(s.execute("plugin.list", &json!({})).unwrap()["safeMode"], true);

    // Leaving the choice after a launch that stopped with Start Normally loads them.
    safe_mode::set_active(None);
    vectorcraft_engine::cmd::plugin::load_prefs_folder(&mut s);
    assert!(installed("test.sm.s"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&extra);
}

#[test]
fn a_live_effect_skipped_in_safe_mode_stays_on_its_object_and_says_so() {
    let _f = fresh();
    let mut s = Session::new();
    let fx = wat::parse_str(tpl::returning(&tpl::manifest("test.sm.fx", "effect", "{}"), r#"{"objects":[]}"#)).unwrap();
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&fx)})).unwrap();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("effect.apply", &json!({"effect": "plugin.test.sm.fx"})).unwrap();
    let effect = |s: &mut Session| s.execute("effect.list", &json!({})).unwrap()["applied"][0]["effects"][0].clone();
    assert_eq!((effect(&mut s)["id"].clone(), effect(&mut s).get("skipped").cloned()), (json!("plugin.test.sm.fx"), None));

    safe_mode::set_active(Some(safe_mode::Source::Flag));
    let skipped = effect(&mut s);
    assert_eq!((skipped["id"].clone(), skipped["skipped"].clone()), (json!("plugin.test.sm.fx"), json!(true)), "the object keeps it");

    safe_mode::set_active(None);
    assert_eq!(effect(&mut s).get("skipped"), None);
}

/// In Safe Mode, Expand Appearance refuses an object whose plug-in effect it would drop, its own or
/// in a symbol's art it places, and changes nothing.
#[test]
fn expand_appearance_keeps_a_plug_in_effect_safe_mode_skips() {
    let _f = fresh();
    let mut s = Session::new();
    let fx = wat::parse_str(tpl::returning(&tpl::manifest("test.sm.xp", "effect", "{}"), r#"{"objects":[]}"#)).unwrap();
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&fx)})).unwrap();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("effect.apply", &json!({"effect": "plugin.test.sm.xp"})).unwrap();
    let doc = |s: &mut Session| s.execute("document.inspect", &json!({})).unwrap();
    let refused = |s: &mut Session| {
        let before = doc(s);
        let err = s.execute("effect.expandAppearance", &json!({})).unwrap_err().to_string();
        assert!(err.contains("a plug-in effect in the selection can't be expanded in Safe Mode"), "{err}");
        assert_eq!(doc(s), before, "nothing changes");
    };
    safe_mode::set_active(Some(safe_mode::Source::Flag));
    refused(&mut s);
    // An instance, with an effect of its own, of a symbol whose art holds the plug-in effect.
    safe_mode::set_active(None);
    s.execute("symbol.new", &json!({"name": "fx"})).unwrap();
    s.execute("effect.apply", &json!({"effect": "distort.puckerBloat"})).unwrap();
    safe_mode::set_active(Some(safe_mode::Source::Flag));
    refused(&mut s);
    safe_mode::set_active(None);
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    // A plain instance of that symbol doesn't expand: selected with an object that does, they expand.
    let plain = s.execute("symbol.place", &json!({"name": "fx", "x": 150, "y": 150})).unwrap()["id"].clone();
    let other = s.execute("shape.rectangle", &json!({"x": 10, "y": 120, "width": 30, "height": 30})).unwrap()["id"].clone();
    s.execute("effect.apply", &json!({"effect": "distort.puckerBloat"})).unwrap();
    safe_mode::set_active(Some(safe_mode::Source::Flag));
    s.execute("effect.expandAppearance", &json!({"ids": [plain, other]})).unwrap();
    safe_mode::set_active(None);
    // A plug-in effect on a hidden fill: Expand Appearance drops that fill, so it expands.
    s.execute("shape.rectangle", &json!({"x": 100, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("effect.apply", &json!({"effect": "plugin.test.sm.xp", "item": 0})).unwrap();
    s.execute("appearance.setItem", &json!({"index": 0, "visible": false})).unwrap();
    safe_mode::set_active(Some(safe_mode::Source::Flag));
    s.execute("effect.expandAppearance", &json!({})).unwrap();
}

/// In Safe Mode, a plug-in effect on a dimension selected with the art it measures does not stop
/// Expand Appearance: the dimension is left as it is, its effect included, and the art expands.
#[test]
fn expand_appearance_in_safe_mode_leaves_a_dimension_selected_with_its_art() {
    let _f = fresh();
    let mut s = Session::new();
    let fx = wat::parse_str(tpl::returning(&tpl::manifest("test.sm.dim", "effect", "{}"), r#"{"objects":[]}"#)).unwrap();
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&fx)})).unwrap();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let art = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 72, "height": 72})).unwrap()["id"].clone();
    s.execute("paint.setStroke", &json!({"ids": [art], "color": "#000000"})).unwrap();
    s.execute("appearance.addStroke", &json!({})).unwrap();
    let source = json!({"kind": "segment", "edge": {"node": art, "subpath": 0, "segment": 0}});
    let dim = s.execute("dimension.create", &json!({"source": source})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [dim]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "plugin.test.sm.dim"})).unwrap();
    s.execute("select.set", &json!({"ids": [art, dim]})).unwrap();
    let node = |s: &mut Session, id: &Value| {
        let inspect = s.execute("document.inspect", &json!({})).unwrap();
        let mut found = Value::Null;
        let mut stack = vec![inspect["layers"].clone()];
        while let Some(v) = stack.pop() {
            match v {
                Value::Array(a) => stack.extend(a),
                Value::Object(o) if o.get("id") == Some(id) => found = Value::Object(o),
                Value::Object(o) => stack.extend(o.into_iter().map(|(_, v)| v)),
                _ => {}
            }
        }
        found
    };
    let kept = node(&mut s, &dim);
    let measured = node(&mut s, &art);
    safe_mode::set_active(Some(safe_mode::Source::Flag));
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    safe_mode::set_active(None);
    assert_ne!(node(&mut s, &art), measured, "the art expands");
    assert_eq!(node(&mut s, &dim), kept, "the dimension and its effect stay as they are");
}
