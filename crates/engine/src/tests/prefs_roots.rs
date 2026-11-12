//! The preferences commands under automation roots: a confined request sees a folder preference and
//! an export preset's folder only inside its read roots.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cmd::fileio::ScreenExportPreset;
use crate::file_access::{AutomationRoots, confine};
use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn text(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// A fresh scratch folder for one test, with the folders `in` and `out`.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-prefs-roots-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["in", "out"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    dir
}

/// The folder preferences that name the folder outside the read root.
const OUTSIDE: [&str; 3] = ["pluginsFolder", "scratchPrimary", "templatesFolder"];

/// A session whose folder preferences name `inside` (fonts, and the Data Recovery folder with
/// spaces around it) and `outside` (the [`OUTSIDE`] ones, the Templates folder with spaces around
/// it), with a scratch disk choice and an export preset kept with each folder.
fn session(inside: &str, outside: &str) -> Session {
    let mut s = Session::new();
    s.prefs.fonts_folder = inside.to_string();
    s.prefs.recovery_folder = format!(" {inside} ");
    s.prefs.plugins_folder = outside.to_string();
    s.prefs.scratch_primary = outside.to_string();
    s.prefs.scratch_secondary = "None".to_string();
    s.prefs.templates_folder = format!(" {outside} ");
    for (name, folder) in [("Inside", inside), ("Outside", outside)] {
        let settings = json!({"formats": [{"format": "svg"}], "folder": folder});
        s.prefs.screen_export_presets.push(ScreenExportPreset { name: name.into(), settings: settings.as_object().cloned().unwrap_or_default() });
    }
    s
}

/// Does a string anywhere in `v` hold `path`?
fn holds(v: &Value, path: &str) -> bool {
    v.to_string().contains(&path.replace('\\', "\\\\"))
}

#[test]
fn prefs_commands_leave_out_folders_outside_the_read_root() {
    let base = folder("get");
    let (inside, outside) = (text(&base.join("in").join("fonts")), text(&base.join("out")));
    let roots = AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
    let mut s = session(&inside, &outside);

    let all = confine(roots.as_ref(), || run(&mut s, "prefs.get", json!({})));
    let prefs = &all["prefs"];
    assert_eq!(prefs["fontsFolder"], json!(inside), "a folder inside the root is shown");
    assert_eq!(prefs["recoveryFolder"], json!(format!(" {inside} ")), "checked without the spaces, shown as kept");
    assert_eq!(prefs["scratchSecondary"], json!("None"), "a scratch disk choice is shown");
    for key in OUTSIDE {
        assert_eq!(prefs[key], Value::Null, "{key}: {all}");
    }
    let presets = prefs["screenExportPresets"].as_array().unwrap();
    assert_eq!(presets[0]["settings"]["folder"], json!(inside));
    assert_eq!(presets[1]["name"], json!("Outside"), "the preset stays");
    assert!(presets[1]["settings"].get("folder").is_none(), "{all}");
    assert!(!holds(&all, &outside), "{all}");

    for key in OUTSIDE {
        let one = confine(roots.as_ref(), || run(&mut s, "prefs.get", json!({"key": key})));
        assert_eq!(one, Value::Null, "{key}");
    }
    assert_eq!(confine(roots.as_ref(), || run(&mut s, "prefs.get", json!({"key": "fontsFolder"}))), json!(inside));

    let list = confine(roots.as_ref(), || run(&mut s, "prefs.list", json!({})));
    assert!(!holds(&list, &outside), "{list}");
    let row = |key: &str| list.as_array().unwrap().iter().find(|r| r["key"] == key).cloned().unwrap();
    assert_eq!(row("fontsFolder")["value"], json!(inside));
    assert_eq!(row("pluginsFolder")["value"], Value::Null);
    assert_eq!(row("pluginsFolder")["kind"], json!("text"), "the row stays");

    // A request that sets another preference, or resets a category without folders.
    let set = confine(roots.as_ref(), || run(&mut s, "prefs.set", json!({"values": {"keyboardIncrement": 2, "pluginsFolder": outside}})));
    assert_eq!(set, json!({"keyboardIncrement": 2.0, "pluginsFolder": null}), "a folder kept as it was is left out too");
    let reset = confine(roots.as_ref(), || run(&mut s, "prefs.reset", json!({"category": "General"})));
    assert!(!holds(&reset, &outside), "{reset}");
    assert_eq!(reset["prefs"]["fontsFolder"], json!(inside));

    // Unconfined and with only a write root.
    let all = run(&mut s, "prefs.get", json!({}));
    assert_eq!(all["prefs"]["pluginsFolder"], json!(outside), "unconfined, the folder is shown");
    assert_eq!(all["prefs"]["screenExportPresets"][1]["settings"]["folder"], json!(outside));
    let write_only = AutomationRoots::new(&[], &[&base.join("in")]).unwrap();
    let all = confine(write_only.as_ref(), || run(&mut s, "prefs.get", json!({})));
    assert_eq!(all["prefs"]["fontsFolder"], Value::Null, "without a read root no folder is shown");
    assert_eq!(all["prefs"]["recoveryFolder"], Value::Null);
    assert_eq!(s.prefs.plugins_folder, outside, "the preferences are unchanged");
    let _ = std::fs::remove_dir_all(&base);
}
