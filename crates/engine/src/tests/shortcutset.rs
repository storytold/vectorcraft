//! Shortcut sets another app saved as text (`shortcutSet.read`): entries, sections, codes, the
//! tokens left unread and the caps; and the matches kept from earlier imports (`importMatches.*`).
//! Every set here is written for the test, with invented names.

use serde_json::{Value, json};

use crate::cmd::importmatches;
use crate::cmd::shortcutset::{KysEntry, NOT_READABLE, read_kys};
use crate::*;

fn read(text: &str) -> Value {
    Session::new().execute("shortcutSet.read", &json!({ "data": text, "name": "My Keys.kys" })).unwrap()
}

#[test]
fn reads_an_entry_with_its_codes() {
    let text = "/Menus {\n  /Acme\\ Lasso\\ Tool {\n    /Represent 7\n    /Key 48\n    /Context 2\n    /Modifiers 0\n  }\n}\n";
    let set = read_kys(text.as_bytes()).unwrap();
    let entry = KysEntry {
        section: vec!["Menus".into()],
        name: "Acme Lasso Tool".into(),
        key: Some(48),
        modifiers: Some(0),
        context: Some(2),
        represent: Some(7),
        line: 2,
    };
    assert_eq!((set.entries, set.unread, set.truncated), (vec![entry], vec![], false));
}

#[test]
fn entries_keep_their_sections_and_the_command_counts_codes() {
    let text = "/Menus {\n /Edit {\n  /Acme\\ Undo { /Key 90 /Modifiers 64 }\n  /Acme\\ Redo { /Key 90 /Modifiers 80 }\n }\n /Acme\\ Zoom { /Key 61 /Modifiers 64 }\n /Group\\ Only { /Context 1 }\n}\n";
    let r = read(text);
    assert_eq!(r["set"], "My Keys");
    let names: Vec<(&str, Vec<&str>, usize)> = r["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap(),
                e["section"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect(),
                e["line"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(names, [("Acme Undo", vec!["Menus", "Edit"], 3), ("Acme Redo", vec!["Menus", "Edit"], 4), ("Acme Zoom", vec!["Menus"], 6)]);
    assert_eq!(r["entries"][0]["context"], Value::Null, "a code the entry doesn't set");
    assert_eq!(r["codes"]["keys"], json!([{"value": 90, "count": 2}, {"value": 61, "count": 1}]));
    assert_eq!(r["codes"]["modifiers"], json!([{"value": 64, "count": 2}, {"value": 80, "count": 1}]));
    assert_eq!((r["unread"].as_array().unwrap().len(), r["truncated"].as_bool()), (0, Some(false)), "{r}");
    // The same from a file, and from its bytes.
    let dir = vectorcraft_testkit::temp_dir("shortcutset-read");
    let path = dir.join("Studio Keys.kys");
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    let r = s.execute("shortcutSet.read", &json!({ "path": path.to_string_lossy() })).unwrap();
    assert_eq!((r["set"].as_str(), r["entries"].as_array().unwrap().len()), (Some("Studio Keys"), 3));
    let b64 = vectorcraft_format::base64_encode(text.as_bytes());
    assert_eq!(s.execute("shortcutSet.read", &json!({ "dataBase64": b64 })).unwrap()["entries"].as_array().unwrap().len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn refuses_xml_shortcut_files() {
    let e = read_kys(b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\n<shortcuts/>").unwrap_err();
    assert_eq!(e, NOT_READABLE);
    assert!(read_kys(b"").unwrap_err().starts_with("not a shortcut set"));
    assert!(read_kys(b"/Menus 3").unwrap_err().starts_with("not a shortcut set"));
    let e = Session::new().execute("shortcutSet.read", &json!({ "data": "<shortcuts/>" })).unwrap_err().to_string();
    assert!(e.contains(NOT_READABLE), "{e}");
    assert!(Session::new().execute("shortcutSet.read", &json!({})).is_err());
}

#[test]
fn lists_unknown_tokens_with_their_lines() {
    // CR, LF and CRLF each end a line.
    let text = "/Menus {\r/Acme\\ Tool {\r\n/Key 66 /Bogus 3 junk\n/Modifiers\n}\n/Dangling\n}\n} /Key 70";
    let set = read_kys(text.as_bytes()).unwrap();
    assert_eq!(set.entries.len(), 1);
    assert_eq!(set.entries[0].modifiers, None, "/Modifiers without a value");
    let unread: Vec<(usize, &str)> = set.unread.iter().map(|(l, t)| (*l, t.as_str())).collect();
    assert_eq!(unread, [(3, "/Bogus 3"), (3, "junk"), (4, "/Modifiers"), (6, "/Dangling"), (8, "}"), (8, "/Key 70")]);
    // A number too large, and a name ending in a backslash at the end of the text.
    let set = read_kys(b"/Menus { /A { /Key 99999999999999999999 } } /End\\").unwrap();
    assert!(set.entries.is_empty());
    assert_eq!(set.unread, [(1, "/Key (a number too large)".to_string()), (1, "/End".to_string())]);
    // A block left open.
    let set = read_kys(b"/Menus { /A { /Key 1 }").unwrap();
    assert_eq!((set.entries.len(), set.unread.clone()), (1, vec![(1, "/Menus { (not closed)".to_string())]));
}

#[test]
fn caps_nesting_entries_and_names() {
    // Blocks past 16 deep are skipped, and what follows them is read.
    let deep = format!("/Menus {{ {} /Inner {{ /Key 1 }} {} /After {{ /Key 2 }} }}", "/A { ".repeat(20), "} ".repeat(20));
    let set = read_kys(deep.as_bytes()).unwrap();
    assert!(set.truncated);
    assert_eq!(set.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["After"]);
    // At most 20,000 entries.
    let many = format!("/Menus {{ {} }}", "/E { /Key 1 } ".repeat(20_005));
    let set = read_kys(many.as_bytes()).unwrap();
    assert_eq!((set.entries.len(), set.truncated), (20_000, true));
    // A name is cut at 1,024 bytes, between characters.
    let long = format!("/Menus {{ /{} {{ /Key 1 }} }}", "é".repeat(700));
    let set = read_kys(long.as_bytes()).unwrap();
    assert_eq!((set.entries[0].name.chars().count(), set.truncated), (512, true));
    // At most 1,000 unread tokens.
    let junk = format!("/Menus {{ {} }}", "x ".repeat(1_500));
    assert_eq!(read_kys(junk.as_bytes()).unwrap().unread.len(), 1000);
    // At most 4 MiB of text.
    let big = format!("/Menus {{ /E {{ /Key 1 }} {} }}", " ".repeat(4 << 20));
    let set = read_kys(big.as_bytes()).unwrap();
    assert_eq!((set.entries.len(), set.truncated), (1, true));
    // Long section names repeated for every entry stop at 8 MiB of names, not hundreds of megabytes.
    let open: String = (0..15).map(|i| format!("/{}{i} {{ ", "s".repeat(1020))).collect();
    let wide = format!("{open}{}", "/E{/Key 1}".repeat(19_000));
    let set = read_kys(wide.as_bytes()).unwrap();
    let kept: usize = set.entries.iter().map(|e| e.name.len() + e.section.iter().map(String::len).sum::<usize>()).sum();
    assert!(set.truncated && kept <= 8 << 20 && set.entries.len() > 500, "{} entries, {kept} bytes", set.entries.len());
}

#[test]
fn big_files_are_read_up_to_the_cap() {
    let text = format!("/Menus {{ /E {{ /Key 1 }} {} }}", " ".repeat(5 << 20));
    let dir = vectorcraft_testkit::temp_dir("shortcutset-big");
    let path = dir.join("Big.kys");
    std::fs::write(&path, &text).unwrap();
    let b64 = vectorcraft_format::base64_encode(text.as_bytes());
    // Base64 broken into lines decodes as well.
    let lines: String = b64.as_bytes().chunks(76).map(|l| format!("{}\n", String::from_utf8_lossy(l))).collect();
    let mut s = Session::new();
    for p in [json!({ "path": path.to_string_lossy() }), json!({ "data": text }), json!({ "dataBase64": b64 }), json!({ "dataBase64": lines })] {
        let r = s.execute("shortcutSet.read", &p).unwrap();
        assert_eq!(
            (r["entries"].as_array().unwrap().len(), r["truncated"].as_bool()),
            (1, Some(true)),
            "{}",
            p.as_object().unwrap().keys().next().unwrap()
        );
    }
    // A byte order mark doesn't count toward the cap.
    let fits = format!("\u{feff}/Menus {{ /E {{ /Key 1 }} {}}}", " ".repeat((4 << 20) - 24));
    assert_eq!(fits.len(), (4 << 20) + 3);
    std::fs::write(&path, &fits).unwrap();
    assert_eq!(s.execute("shortcutSet.read", &json!({ "path": path.to_string_lossy() })).unwrap()["truncated"], false);
    let _ = std::fs::remove_dir_all(dir);
}

fn matched(s: &mut Session, p: Value) -> Vec<String> {
    let r = s.execute("importMatches.list", &p).unwrap();
    r.as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn import_matches_are_kept_replaced_and_forgotten() {
    let mut s = Session::new();
    let matches = json!([
        {"kind": "name", "name": "Acme Undo", "target": "edit.undo", "label": "Undo"},
        {"kind": "name", "name": "Acme Gadget", "target": ""},
        {"kind": "key", "name": "90", "target": "Z"},
        {"kind": "modifiers", "name": "64", "target": "Cmd"},
    ]);
    assert_eq!(s.execute("importMatches.set", &json!({ "matches": matches })).unwrap()["count"], 4);
    assert_eq!(matched(&mut s, json!({"kind": "name"})), ["Acme Undo", "Acme Gadget"]);
    assert_eq!(matched(&mut s, json!({"query": "UNDO"})), ["Acme Undo"], "the name, target or label, any case");
    assert_eq!(matched(&mut s, json!({"query": "cmd"})), ["64"]);
    // A later answer replaces the earlier one, and null forgets it.
    let later = json!([{"kind": "name", "name": "Acme Gadget", "target": "object.group"}, {"kind": "key", "name": "90", "target": null}]);
    assert_eq!(s.execute("importMatches.set", &json!({ "matches": later })).unwrap()["count"], 3);
    let targets = importmatches::targets(&s.prefs);
    assert_eq!((targets.get(&("name", "Acme Gadget")), targets.get(&("key", "90"))), (Some(&"object.group"), None));
    // A bad match fails the whole call.
    for bad in [
        json!([{"kind": "name", "name": "Fine", "target": "edit.copy"}, {"kind": "colour", "name": "Red", "target": ""}]),
        json!([{"kind": "name", "name": "x".repeat(1025), "target": ""}]),
        json!([{"kind": "name", "name": "", "target": ""}]),
        json!([{"kind": "name", "name": "Fine", "target": 3}]),
        json!((0..20_001).map(|i| json!({"kind": "key", "name": i.to_string(), "target": "K"})).collect::<Vec<_>>()),
    ] {
        assert!(s.execute("importMatches.set", &json!({ "matches": bad })).is_err());
    }
    assert!(s.execute("importMatches.list", &json!({"kind": "colour"})).is_err());
    assert_eq!(s.prefs.import_matches.len(), 3);
    // Forgetting one kind, then the rest.
    assert_eq!(s.execute("importMatches.reset", &json!({"kind": "modifiers"})).unwrap()["removed"], 1);
    assert_eq!(s.execute("importMatches.reset", &json!({})).unwrap()["removed"], 2);
}

#[test]
fn import_matches_survive_a_preferences_reset_and_a_restart() {
    let mut s = Session::new();
    s.execute("importMatches.set", &json!({"matches": [{"kind": "name", "name": "Acme Undo", "target": "edit.undo"}]})).unwrap();
    let saved = s.prefs.to_json();
    assert_eq!(saved["importMatches"][0]["target"], "edit.undo");
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.import_matches.len(), 1);
    let mut fresh = Session::new();
    fresh.apply_prefs(serde_json::from_value(saved).unwrap());
    assert_eq!(matched(&mut fresh, json!({})), ["Acme Undo"]);
    // Nothing is written while there are none, older preference files load with none, and a damaged
    // match doesn't stop the preferences loading.
    assert!(Prefs::default().to_json().get("importMatches").is_none());
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.import_matches.is_empty());
    let damaged: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2, "importMatches": [{"name": "Acme Undo"}]})).unwrap();
    assert_eq!((damaged.keyboard_increment, damaged.import_matches[0].target.as_str()), (2.0, ""));
}

#[test]
fn read_shows_what_earlier_imports_matched() {
    let mut s = Session::new();
    let matches = json!([
        {"kind": "name", "name": "Acme Undo", "target": "edit.undo"},
        {"kind": "key", "name": "90", "target": "Z"},
        {"kind": "key", "name": "61", "target": ""},
        {"kind": "modifiers", "name": "64", "target": "Cmd"},
        {"kind": "modifiers", "name": "0", "target": ""},
    ]);
    s.execute("importMatches.set", &json!({ "matches": matches })).unwrap();
    let text = "/Menus {\n /Acme\\ Undo { /Key 90 /Modifiers 64 }\n /Acme\\ Redo { /Key 90 /Modifiers 80 }\n /Acme\\ Zoom { /Key 61 /Modifiers 0 }\n /Acme\\ Tool { /Key 90 }\n}\n";
    let r = s.execute("shortcutSet.read", &json!({ "data": text })).unwrap();
    let e = &r["entries"];
    assert_eq!((e[0]["target"].as_str(), e[0]["chord"].as_str()), (Some("edit.undo"), Some("Cmd+Z")));
    assert_eq!((e[1].get("target"), e[1].get("chord")), (None, None), "modifiers 80 aren't matched yet");
    assert_eq!(e[2].get("chord"), None, "a key with no equivalent");
    assert_eq!(e[3]["chord"], "Z", "an entry without /Modifiers takes none");
    assert_eq!(r["codes"]["keys"], json!([{"value": 90, "count": 3, "key": "Z"}, {"value": 61, "count": 1, "key": ""}]));
    let mods = json!([{"value": 0, "count": 1, "mods": ""}, {"value": 64, "count": 1, "mods": "Cmd"}, {"value": 80, "count": 1}]);
    assert_eq!(r["codes"]["modifiers"], mods);
}

#[test]
fn the_commands_take_only_the_keys_they_describe() {
    // The control channel, MCP and the CLI check params against the description (#1442).
    for (id, every_key) in [
        ("shortcutSet.read", json!({"path": "/x/a.kys", "data": "", "dataBase64": "", "name": "a.kys"})),
        ("importMatches.list", json!({"kind": "name", "query": "undo"})),
        ("importMatches.set", json!({"matches": []})),
        ("importMatches.reset", json!({"kind": "key"})),
    ] {
        check_params(id, &every_key).unwrap();
        assert!(check_params(id, &json!({"qeury": "undo"})).is_err(), "{id}");
    }
}
