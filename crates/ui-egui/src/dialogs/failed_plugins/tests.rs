use serde_json::json;
use vectorcraft_plugins::wat as tpl;

use super::super::brush_delete::tests::{answer, app};
use super::*;

/// A plug-in store holding a file that isn't a plug-in yet, at `<store>/<id>.wasm`.
fn store_with_broken(tag: &str, id: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("vectorcraft-ui-failed-plugins-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{id}.wasm"));
    std::fs::write(&file, b"\0asm?").unwrap();
    (dir, file)
}

/// Plug-ins That Didn't Load… is disabled until a stored plug-in fails to load; it then lists the
/// file with why, and Retry All installs it once it loads, after which the item is disabled again.
#[test]
fn the_failed_plugins_list_names_each_file_and_retry_all_loads_it() {
    let id = "test.ui.failedretry";
    let (dir, file) = store_with_broken("retry", id);
    let mut app = app();
    assert!(!crate::menus::enabled(&app, COMMAND), "nothing failed yet");
    app.session.attach_plugin_store(dir.clone());
    assert!(crate::menus::enabled(&app, COMMAND), "a stored file failed to load");

    let r = app.run(COMMAND, json!({})).unwrap();
    assert_eq!((r["pending"].clone(), r["failed"].clone()), (json!(KIND), json!(1)), "{r}");
    let named = format!("{id}.wasm: ");
    let words = &app.host;
    let lines = app.dialogs().unwrap().with(|host| host.get::<FailedPluginsList>().expect("the list is open").0.shown_paragraphs(words));
    assert!(lines.iter().any(|l| l.starts_with(&named) && l.len() > named.len()), "the file and why: {lines:?}");

    // Close keeps it listed.
    answer::<FailedPluginsList>(&mut app, None, pump);
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(crate::menus::enabled(&app, COMMAND));

    std::fs::write(&file, wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()).unwrap();
    app.run(COMMAND, json!({})).unwrap();
    answer::<FailedPluginsList>(&mut app, Some("proceed"), pump);
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "Retry All installed it");
    assert!(!crate::menus::enabled(&app, COMMAND), "nothing is left that failed");
    assert!(app.run(COMMAND, json!({})).is_err(), "the list doesn't open with nothing in it");
    vectorcraft_plugins::registry::remove(id);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Retry All of a list opened under automation read roots runs under them, even when it is
/// answered in a later frame: a file outside them isn't read.
#[test]
fn retry_all_runs_under_the_read_roots_the_list_opened_under() {
    let id = "test.ui.failedroots";
    let (dir, file) = store_with_broken("roots", id);
    let mut app = app();
    app.session.attach_plugin_store(dir.clone());
    std::fs::write(&file, wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()).unwrap();
    let elsewhere = std::env::temp_dir().join(format!("vectorcraft-ui-failed-plugins-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(&elsewhere).unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[elsewhere.as_path()], &[]).unwrap();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || app.run(COMMAND, json!({}))).unwrap();
    answer::<FailedPluginsList>(&mut app, Some("proceed"), pump);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "not loaded outside the roots");
    assert!(crate::menus::enabled(&app, COMMAND), "still listed");

    // Without roots, Retry All loads it.
    app.run(COMMAND, json!({})).unwrap();
    answer::<FailedPluginsList>(&mut app, Some("proceed"), pump);
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    vectorcraft_plugins::registry::remove(id);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&elsewhere);
}
