//! Action sets another app saved as text (`actionSet.read`): actions, steps, the tokens left unread
//! and the caps. Every set here is written for the test, with invented names.

use serde_json::json;

use crate::cmd::actionset::{AiaStep, read_aia};
use crate::*;

/// `text` as a bracketed hexadecimal value.
fn hex(text: &str) -> String {
    let digits: String = text.bytes().map(|b| format!("{b:02x}")).collect();
    format!("[ {} {digits} ]", text.len())
}

/// A set written from scratch in the layout the module describes: two actions, the second with a
/// step left out of playback, a step with recorded values of each form, and names in several
/// scripts.
fn sample() -> String {
    format!(
        "/version 3\n/name {set}\n/isOpen 1\n/actionCount 2\n/action-1 {{\n\t/name {a1}\n\t/keyIndex 0\n\t/eventCount 1\n\t/event-1 {{\n\t\t/internalName (sample_join)\n\t\t/localizedName {join}\n\t\t/isOn 1\n\t\t/parameterCount 3\n\t\t/parameter-1 {{\n\t\t\t/key 1\n\t\t\t/type (unit real)\n\t\t\t/value 2.5\n\t\t}}\n\t\t/parameter-2 {{\n\t\t\t/type (ustring)\n\t\t\t/value {txt}\n\t\t}}\n\t\t/parameter-3 {{\n\t\t\t/type (raw)\n\t\t\t/value < 3\n\t\t\t\t0a7d0b\n\t\t\t>\n\t\t\t/size 3\n\t\t}}\n\t}}\n}}\r\n/action-2 {{\r\n\t/name {a2}\r\n\t/event-1 {{\r\n\t\t/internalName (sample\\)one)\r\n\t\t/isOn 0\r\n\t\t/parameterCount 0\r\n\t}}\r\n\t/event-2 {{\r\n\t\t/internalName (sample_two)\r\n\t\t/localizedName {two}\r\n\t}}\r\n}}\r\n",
        set = hex("Prep Sätze"),
        a1 = hex("First"),
        join = hex("Join"),
        txt = hex("a } brace"),
        a2 = hex("二番目"),
        two = hex("Second Step"),
    )
}

/// The set's name, its actions and their steps come in with the step's names, its on state and how
/// many values it was recorded with, and nothing is listed unread.
#[test]
fn a_set_reads_its_actions_and_steps() {
    let set = read_aia(sample().as_bytes()).unwrap();
    assert_eq!((set.version, set.name.as_deref(), set.truncated), (3, Some("Prep Sätze"), false));
    assert!(set.unread.is_empty(), "{:?}", set.unread);
    let names: Vec<&str> = set.actions.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["First", "二番目"]);
    let step = |a: usize, s: usize| set.actions[a].steps[s].clone();
    let join = AiaStep { name: "sample_join".into(), label: Some("Join".into()), on: Some(1), params: 3, declared: Some(3), line: 9 };
    assert_eq!(step(0, 0), join);
    assert_eq!((step(1, 0).name.as_str(), step(1, 0).label.clone(), step(1, 0).on), ("sample)one", None, Some(0)));
    assert_eq!((step(1, 1).label.as_deref(), step(1, 1).on, step(1, 1).line), (Some("Second Step"), None, 39));
}

/// Every recorded value of a step is counted. A hexadecimal `/value` with the wrong byte count marks
/// the set truncated; a long one or one that isn't UTF-8 does not.
#[test]
fn recorded_values_are_counted_and_checked() {
    let set = |values: &[String]| {
        let params: String = values.iter().enumerate().map(|(i, v)| format!("/parameter-{} {{ /key 1 {v} }} ", i + 1)).collect();
        read_aia(format!("/version 3 /action-1 {{ /event-1 {{ {params} }} }}").as_bytes()).unwrap()
    };
    let value = |t: &str| format!("/type (ustring) /value {}", hex(t));
    let read = set(&[value("one"), value("two"), value("three")]);
    assert_eq!((read.actions[0].steps[0].params, read.truncated), (3, false));
    let read = set(&[value(&"n".repeat(2000)), "/value [ 1 ff ]".to_string(), "/type (real) /value 4.5".to_string()]);
    assert!(!read.truncated, "a long value or one that isn't UTF-8 is not damage");
    assert!(set(&["/value [ 3 4142 ]".to_string()]).truncated);
    assert!(set(&[value("one"), "/value [ 1 zz ]".to_string()]).truncated, "a second value that isn't hexadecimal");
}

/// `actionSet.read` takes exactly one of `path`, `data` and `dataBase64`, and text for the text
/// params; any other value is an error.
#[test]
fn read_command_refuses_bad_params() {
    let mut s = Session::new();
    let text = "/version 3";
    let dir = std::env::temp_dir().join(format!("vc-actionset-params-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Mine.aia");
    std::fs::write(&file, text).unwrap();
    let path = file.to_string_lossy();
    assert!(s.execute("actionSet.read", &json!({ "path": path })).is_ok());
    let b64 = vectorcraft_format::base64_encode(text.as_bytes());
    for p in [
        json!({ "path": 5, "data": text }),
        json!({ "data": 5 }),
        json!({ "dataBase64": 5 }),
        json!({ "data": text, "name": 7 }),
        json!({ "path": path, "data": text }),
        json!({ "path": path, "dataBase64": b64 }),
        json!({ "data": text, "dataBase64": b64 }),
    ] {
        assert!(s.execute("actionSet.read", &p).is_err(), "{p}");
    }
    let r = s.execute("actionSet.read", &json!({ "data": text, "name": null })).unwrap();
    assert_eq!(r["set"], "", "no set name and no file name");
    let _ = std::fs::remove_dir_all(dir);
}

/// `actionSet.read` names the set after the file when it has no name, gives a step without a label,
/// or with a blank one, a null label, and refuses text that isn't an action set.
#[test]
fn read_command_answers_with_the_set() {
    let mut s = Session::new();
    let text = sample().replacen(&format!("/name {}", hex("Prep Sätze")), "", 1);
    let r = s.execute("actionSet.read", &json!({ "data": text, "name": "/sets/Mine.aia" })).unwrap();
    assert_eq!(r["set"], "Mine");
    assert_eq!(r["actions"][1]["steps"][0], json!({"name": "sample)one", "label": null, "on": false, "params": 0, "line": 34}));
    assert_eq!(r["actions"][0]["steps"][0]["on"], true);
    assert_eq!(r["actions"][0]["steps"][0]["label"], "Join");
    let blank = format!("/version 3 /action-1 {{ /event-1 {{ /internalName (x) /localizedName {} }} }}", hex(" "));
    assert_eq!(s.execute("actionSet.read", &json!({ "data": blank })).unwrap()["actions"][0]["steps"][0]["label"], json!(null));
    for text in ["/Menus { }", "", "version 3", "/version three"] {
        assert!(s.execute("actionSet.read", &json!({ "data": text })).is_err(), "{text}");
    }
    assert!(s.execute("actionSet.read", &json!({})).is_err());
}

/// `path` and `dataBase64` read the same set as `data`, and a set's name falls back to the path's.
#[test]
fn a_set_reads_from_a_path_and_from_base64() {
    let dir = std::env::temp_dir().join(format!("vc-actionset-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Mine.aia");
    let text = "/version 3 /action-1 { /name [ 1 41 ] }";
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    let r = s.execute("actionSet.read", &json!({ "path": path.to_string_lossy() })).unwrap();
    assert_eq!((r["set"].as_str(), r["actions"][0]["name"].as_str()), (Some("Mine"), Some("A")));
    let b64 = vectorcraft_format::base64_encode(text.as_bytes());
    assert_eq!(s.execute("actionSet.read", &json!({ "dataBase64": b64, "name": "B.aia" })).unwrap()["set"], "B");
    let _ = std::fs::remove_dir_all(dir);
}

/// Tokens out of place are listed with their lines, and a cap marks the set truncated.
#[test]
fn stray_tokens_are_listed_and_caps_truncate() {
    let set = read_aia(b"/version 3\n7 /name [ 1 zz ]\n/name [ 3 4142 ]\n/odd { /x 1 }\n}\n/action-1 { /name [ 1 41 ]").unwrap();
    let unread: Vec<&str> = set.unread.iter().map(|(_, t)| t.as_str()).collect();
    let bad = "[ … ] (not the hexadecimal bytes its count says)";
    assert_eq!(unread, ["7", bad, bad, "/odd {", "}", "blocks left open at the end: 1"]);
    let short = |text: &str| read_aia(text.as_bytes()).unwrap().truncated;
    assert!(short("/version 3 /actionCount 1 /odd { /action-1 { } }"), "the action in a block that isn't read");
    assert!(short("/version 3 /name [ 3 4142 ]"), "a set's name with the wrong byte count");
    assert!(short("/version 3 /action-1 { /name [ 1 zz ] }"), "an action's name that isn't hexadecimal");
    assert!(short("/version 3 /action-1 { /event-1 { /localizedName [ 3 4142 ] } }"), "a label with the wrong byte count");
    let cuts = ["/name [ 4 4142", "/name (ab", "/value < 2 0a", "/name", "/action-2 {"];
    let whole: Vec<&str> = cuts.into_iter().filter(|cut| !short(&format!("/version 3 /action-1 {{ }} {cut}"))).collect();
    assert!(whole.is_empty(), "{whole:?}");
    assert_eq!(set.name, None);
    assert_eq!(set.actions[0].name, "A");
    let many = format!("/version 3 {}", "/action-1 { } ".repeat(1001));
    assert!(read_aia(many.as_bytes()).unwrap().truncated);
    let deep = format!("/version 3 /action-1 {{ /event-1 {{ {} }} }}", "/parameter-1 { ".repeat(40));
    assert!(read_aia(deep.as_bytes()).unwrap().truncated);
    let long = format!("/version 3 /name {}", hex(&"n".repeat(2000)));
    assert!(read_aia(long.as_bytes()).unwrap().truncated);
}

/// A count that differs from the actions, steps or recorded values that follow, or that isn't a
/// number that fits an `i64`, marks the set truncated. A count is read only in its own block.
#[test]
fn counts_that_differ_truncate() {
    let short = |text: &str| read_aia(text.as_bytes()).unwrap().truncated;
    assert!(short("/version 3 /actionCount 2 /action-1 { }"), "one action of two");
    assert!(short("/version 3 /action-1 { /eventCount 2 /event-1 { } }"), "one step of two");
    assert!(!short("/version 3 /actionCount 1 /action-1 { /eventCount 1 /event-1 { } }"));
    assert!(short("/version 3 /actionCount 1 /action-1 { } /action-2 { }"), "two actions of one");
    assert!(short("/version 3 /action-1 { /eventCount 1 /event-1 { } /event-2 { } }"), "two steps of one");
    assert!(short("/version 3 /action-1 { /event-1 { /parameterCount 1 } }"), "no recorded value of one");
    assert!(short("/version 3 /action-1 { /event-1 { /parameterCount 0 /parameter-1 { } } }"), "a recorded value of none");
    assert!(!short("/version 3 /action-1 { /event-1 { /parameterCount 1 /parameter-1 { } } }"));
    assert!(short("/version 3 /actionCount -1"), "a negative count");
    assert!(short("/version 3 /action-1 { /eventCount -1 }"), "a negative step count");
    assert!(short("/version 3 /actionCount 99999999999999999999"), "a count too large for a number");
    for count in ["nope", "1.5", "(1)", "[ 1 31 ]", "< 1 31 >", "{ }"] {
        assert!(short(&format!("/version 3 /actionCount {count}")), "a count that isn't a number: {count}");
        assert!(short(&format!("/version 3 /action-1 {{ /eventCount {count} }}")), "a step count: {count}");
        assert!(short(&format!("/version 3 /action-1 {{ /event-1 {{ /parameterCount {count} }} }}")), "{count}");
    }
    assert!(short("/version 3 /actionCount /name [ 1 41 ]"), "a count followed by another field");
    assert!(short("/version 3 /action-1 { /eventCount }"), "a count at the end of its block");
    assert!(!short("/version 3 /action-1 { /event-1 { /parameter-1 { /actionCount nope } } }"), "a field inside a recorded value");
    assert!(!short("/version 3 /action-1 { /actionCount (x) }"), "a set's count in an action is not read");
}

#[test]
fn the_command_takes_only_the_keys_it_describes() {
    // The control channel, MCP and the CLI check params against the description (#1442).
    check_params("actionSet.read", &json!({"path": "/x/a.aia", "data": "", "dataBase64": "", "name": "a.aia"})).unwrap();
    assert!(check_params("actionSet.read", &json!({"pth": "/x/a.aia"})).is_err());
}
