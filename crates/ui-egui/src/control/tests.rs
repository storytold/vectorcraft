use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::{ControlRequest, MAX_DIALOG_FIELDS, Outcome, handle};
use crate::state::Dialog;
use crate::{Services, VectorcraftApp};

fn input_request(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
    let (req, _) = ControlRequest::new(method, params);
    let Outcome::Done(r) = handle(app, &egui::Context::default(), &req) else { panic!("expected immediate reply") };
    r
}

/// A reply hides a hidden library's id and path at every occurrence, in an error that wraps them in
/// spaces, brackets or quotes, follows them with more path text, or quotes the path with `{:?}`.
#[test]
fn a_reply_hides_a_hidden_library_whatever_surrounds_it() {
    let path = std::env::temp_dir().join("vc-named-reply").join("b.svg").to_string_lossy().into_owned();
    let id = format!("loaded/{path}");
    let hidden = hiding(&[(&id, "loaded/b.svg")], &[]);
    let mut reply = json!({"ok": false, "error": format!("can't add ( {id} ) or '{id}', from {path}; also {id}.bak and {path:?}")});
    hide_under_a_root("named-reply", &mut reply, &hidden);
    assert_eq!(reply["error"], "can't add ( loaded/b.svg ) or 'loaded/b.svg', from b.svg; also loaded/b.svg.bak and \"b.svg\"", "{reply}");
}

/// A [`super::Hiding`] with `hidden` ids, each with its shown id, and `readable` ids.
fn hiding(hidden: &[(&str, &str)], readable: &[&str]) -> super::Hiding {
    let mut hidden: Vec<(String, String)> = hidden.iter().map(|(id, shown)| (id.to_string(), shown.to_string())).collect();
    hidden.sort_by_key(|(id, _)| std::cmp::Reverse(id.len()));
    super::Hiding { hidden, readable: readable.iter().map(|id| id.to_string()).collect(), sent: Vec::new() }
}

/// [`super::hide_library_ids`] on `reply`, under a read root made for `tag`.
fn hide_under_a_root(tag: &str, reply: &mut Value, hidden: &super::Hiding) {
    let root = crate::tests_folder::Folder::new(&format!("{tag}-root"));
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[root.as_path()], &[]).unwrap();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || super::hide_library_ids(reply, hidden));
}

/// Encoded file data in a reply, a base64 string under `dataBase64`, `pngBase64`, `thumbnailBase64`
/// or `thumbnail` (a library item's or a PDF page's PNG), stays as it is when it holds a hidden
/// library's path. Text that is not base64 under such a key, an object or a list under it, and any
/// other key ending in `Base64` are searched.
#[test]
fn a_reply_keeps_encoded_data_that_holds_a_hidden_path() {
    let hidden = hiding(&[("loaded/assets/icon", "loaded/icon")], &[]);
    let data = "QUJ+assets/iconREVGxQQ==";
    let mut reply = json!({"ok": true, "result": {
        "graphics": [{"name": "a", "thumbnail": data}, {"name": "b", "thumbnail": "assets/icon.png"}],
        "files": [{"name": "a.png", "dataBase64": data, "pngBase64": "assets/icon.png", "thumbnailBase64": data}],
        "status": "from assets/icon",
        "dataBase64": {"note": "from assets/icon"},
        "pngBase64": ["from assets/icon"],
        "otherBase64": data,
    }});
    hide_under_a_root("base64", &mut reply, &hidden);
    let files = &reply["result"]["files"][0];
    assert_eq!((&files["dataBase64"], &files["thumbnailBase64"]), (&json!(data), &json!(data)), "{reply}");
    assert_eq!(files["pngBase64"], "icon.png", "{reply}");
    let graphics = &reply["result"]["graphics"];
    assert_eq!((&graphics[0]["thumbnail"], &graphics[1]["thumbnail"]), (&json!(data), &json!("icon.png")), "{reply}");
    assert_eq!(reply["result"]["status"], "from icon", "{reply}");
    assert_eq!(reply["result"]["dataBase64"]["note"], "from icon", "{reply}");
    assert_eq!(reply["result"]["pngBase64"][0], "from icon", "{reply}");
    assert_eq!(reply["result"]["otherBase64"], "QUJ+iconREVGxQQ==", "{reply}");
}

/// A hidden path followed by more text stays only when the longer text is another loaded library's
/// id: text after the path that the request may read, a `..` part and the part of the path before a
/// space do not keep it.
#[test]
fn a_reply_keeps_a_hidden_path_only_inside_a_known_library_id() {
    let base = crate::tests_folder::Folder::new("continuation");
    let root = base.join("in");
    std::fs::create_dir_all(&root).unwrap();
    let hidden_path = base.join("in x").join("b.svg").to_string_lossy().into_owned();
    let other = base.join("outside").join("b.svg").to_string_lossy().into_owned();
    let known = format!("loaded/{other}{}sub.svg", std::path::MAIN_SEPARATOR);
    let hidden = hiding(&[(&format!("loaded/{hidden_path}"), "loaded/b.svg"), (&format!("loaded/{other}"), "loaded/b.svg (2)")], &[&known]);
    let up = ["", "..", "..", "in", "a.svg"].join(std::path::MAIN_SEPARATOR_STR);
    let mut reply = json!({"error": format!("kept {hidden_path}.bak, {other}{up} and {known}")});
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || super::hide_library_ids(&mut reply, &hidden));
    assert_eq!(reply["error"], json!(format!("kept b.svg.bak, b.svg (2){up} and {known}")), "{reply}");
    // A readable library's id with a space after the hidden part stays whole.
    // A readable id with a `..` part does not keep the hidden part.
    let hidden = hiding(&[("loaded//art/in", "loaded/in")], &["loaded//art/in work/a.svg", "loaded//art/in/../ok/c.svg"]);
    let mut reply = json!({"error": "x loaded//art/in work/a.svg, /art/in work/a.svg and /art/in work; /art/in/../ok/c.svg"});
    hide_under_a_root("space-continuation", &mut reply, &hidden);
    assert_eq!(reply["error"], "x loaded//art/in work/a.svg, /art/in work/a.svg and in work; in/../ok/c.svg", "{reply}");
}

/// A readable library's id that holds a hidden absolute path after its start stays whole: with and
/// without its `loaded/`, in a `file:` URL and with `%XX` escapes, in any case with matching in any
/// case. The hidden path on its own, and followed by other path text, is still hidden.
#[test]
fn a_reply_keeps_a_readable_id_that_holds_a_hidden_path_inside() {
    let hidden = hiding(&[("loaded//libs/Kit.gpl", "loaded/Kit.gpl")], &["loaded//project/libs/Kit.gpl"]);
    let text = "loaded//project/libs/Kit.gpl, /project/libs/Kit.gpl, file:///project/libs/Kit.gpl and file:///project%2Flibs/Kit.gpl; /libs/Kit.gpl and /libs/Kit.gpl.bak";
    for fold in [false, true] {
        let mut reply = json!({"error": text});
        super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&hidden), hiding: &hidden, fold });
        let kept = "loaded//project/libs/Kit.gpl, /project/libs/Kit.gpl, file:///project/libs/Kit.gpl and file:///project%2Flibs/Kit.gpl";
        assert_eq!(reply["error"], json!(format!("{kept}; Kit.gpl and Kit.gpl.bak")), "fold {fold}: {reply}");
    }
    let mut reply = json!({"error": "LOADED//PROJECT/LIBS/KIT.GPL and file://LocalHost/project/libs/Kit.gpl"});
    super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&hidden), hiding: &hidden, fold: true });
    assert_eq!(reply["error"], "LOADED//PROJECT/LIBS/KIT.GPL and file://LocalHost/project/libs/Kit.gpl", "{reply}");
    // The readable id's path after a drive or in a URL that names another host is another path.
    for text in ["C:/project/libs/Kit.gpl", "file://remote/project/libs/Kit.gpl"] {
        let mut reply = json!({"error": text});
        super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&hidden), hiding: &hidden, fold: false });
        assert!(!reply["error"].as_str().unwrap_or_default().contains("/libs/Kit.gpl"), "{text}: {reply}");
    }
    // The readable id's path after a character that can continue a path name ends another path.
    for text in
        ["/a+/project/libs/Kit.gpl", "/a@/project/libs/Kit.gpl", "a=/project/libs/Kit.gpl", "/a!/project/libs/Kit.gpl", "/a%2B/project/libs/Kit.gpl"]
    {
        for fold in [false, true] {
            let mut reply = json!({"error": text});
            super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&hidden), hiding: &hidden, fold });
            assert!(!reply["error"].as_str().unwrap_or_default().contains("/libs/Kit.gpl"), "{text}, fold {fold}: {reply}");
        }
    }
    // Written with its `loaded/`, the readable id stays after `=` too.
    let mut reply = json!({"error": "key=loaded//project/libs/Kit.gpl"});
    super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&hidden), hiding: &hidden, fold: false });
    assert_eq!(reply["error"], "key=loaded//project/libs/Kit.gpl", "{reply}");
}

/// With matching in any case (as on Windows), a hidden path written in other letter cases is hidden
/// too; a path in a `file:` URL with `%XX` escapes is hidden on every platform, the whole URL shown as
/// the file name.
#[test]
fn a_reply_hides_a_path_in_any_case_and_escaped() {
    let hidden = hiding(&[("loaded/C:/Libs/My Swatches/b.svg", "loaded/b.svg")], &[]);
    let mut reply = json!({"error": "from c:/LIBS/my swatches/B.SVG and file:///C:/Libs/My%20Swatches/b.svg"});
    let pairs = super::hidden_text(&hidden);
    let rewrite = super::Rewrite { pairs, hiding: &hidden, fold: true };
    super::replace_in_value(&mut reply, &rewrite);
    assert_eq!(reply["error"], "from b.svg and b.svg", "{reply}");
    // Two libraries whose paths differ only in letter case keep their own ids.
    let two = hiding(&[("loaded//libs/b.svg", "loaded/b.svg"), ("loaded//libs/B.svg", "loaded/B.svg")], &[]);
    let mut reply = json!({"error": "x /libs/B.svg y /libs/b.svg z /LIBS/b.svg"});
    super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&two), hiding: &two, fold: true });
    assert_eq!(reply["error"], "x B.svg y b.svg z b.svg", "{reply}");
    // A readable id that begins with a hidden one stays whole in any case, `loaded/` included.
    let nested = hiding(&[("loaded//libs/in", "loaded/in")], &["loaded//libs/in/a.svg"]);
    let mut reply = json!({"error": "LOADED//LIBS/IN/A.SVG"});
    super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&nested), hiding: &nested, fold: true });
    assert_eq!(reply["error"], "LOADED//LIBS/IN/A.SVG", "{reply}");
    let mut reply = json!({"error": "file:///C:/Libs/My%20Swatches/b.svg"});
    hide_under_a_root("escaped", &mut reply, &hidden);
    assert_eq!(reply["error"], "b.svg", "{reply}");
    let hidden = hiding(&[("loaded//libs/caf\u{e9}/b.svg", "loaded/b.svg")], &[]);
    let mut reply = json!({"error": "file:///libs/caf%C3%a9/b.svg and file:///libs%2Fcaf%c3%A9%2Fb.svg?next=%2Fdocs&x=%FF at 100%25"});
    hide_under_a_root("escaped-mixed", &mut reply, &hidden);
    assert_eq!(reply["error"], "b.svg and b.svg?next=%2Fdocs&x=%FF at 100%25", "{reply}");
    // A `file:` URL without escapes is replaced whole too, with `localhost` or another host.
    let mut reply = json!({"error": "file:///libs/caf\u{e9}/b.svg and FILE://localhost/libs/caf%C3%A9/b.svg"});
    hide_under_a_root("plain-url", &mut reply, &hidden);
    assert_eq!(reply["error"], "b.svg and b.svg", "{reply}");
    let mut reply = json!({"error": "file://remote/libs/caf\u{e9}/b.svg; file:/libs/caf\u{e9}/b.svg; file: x/libs/caf\u{e9}/b.svg"});
    hide_under_a_root("other-host", &mut reply, &hidden);
    assert_eq!(reply["error"], "b.svg; b.svg; file: xb.svg", "{reply}");
    let hidden = hiding(&[("loaded///host/share/c.svg", "loaded/c.svg")], &[]);
    let mut reply = json!({"error": "file://host/share/c.svg, file:////host/share/c.svg, file://localhost//host/share/c.svg"});
    hide_under_a_root("host-url", &mut reply, &hidden);
    assert_eq!(reply["error"], "c.svg, c.svg, c.svg", "{reply}");
}

/// Only a string with a `%XX` escape is decoded before it is searched: a `%` that begins no escape
/// leaves the text as it is, and the reply is the same either way.
#[test]
fn only_a_percent_escape_is_decoded() {
    for (text, escaped) in [("width=\"100%\"", false), ("50% off", false), ("%zz%4", false), ("%", false), ("a%2Fb", true), ("%fF", true)] {
        assert_eq!(super::has_percent_escape(text), escaped, "{text}");
        let decoded = super::decoded_with_spans(text).map(|(decoded, _)| decoded);
        assert_eq!(decoded.as_deref() != Some(text), escaped, "{text}: an escape alone changes the decoded text");
    }
    let hidden = hiding(&[("loaded//libs/b.svg", "loaded/b.svg")], &[]);
    let mut reply = json!({"error": "100% of /libs/b.svg and file:///libs%2Fb.svg"});
    super::replace_in_value(&mut reply, &super::Rewrite { pairs: super::hidden_text(&hidden), hiding: &hidden, fold: false });
    assert_eq!(reply["error"], "100% of b.svg and b.svg", "{reply}");
}

/// A path a failed request sent takes the place of a hidden library's id for the same path, written
/// with the other separator, and in another letter case when matching in any case (as on Windows).
#[test]
fn a_sent_path_replaces_a_hidden_id_for_the_same_path() {
    let mut hidden = hiding(&[(r"loaded/C:\Libs\b.svg", "loaded/b.svg (2)"), ("loaded//other/b.svg", "loaded/b.svg")], &[]);
    hidden.set_hidden(vec![("loaded/c:/libs/B.svg".to_string(), "loaded/B.svg".to_string())], true);
    let shown: Vec<&str> = hidden.hidden.iter().map(|(_, shown)| shown.as_str()).collect();
    assert_eq!(shown, ["loaded/B.svg", "loaded/b.svg"], "{hidden:?}");
    assert_eq!(hidden.sent, [("loaded/c:/libs/B.svg".to_string(), "loaded/B.svg".to_string())]);
    hidden.set_hidden(vec![("loaded//OTHER/b.svg".to_string(), "loaded/x.svg".to_string())], false);
    assert_eq!((hidden.hidden[1].1.as_str(), hidden.sent.len()), ("loaded/b.svg", 2), "{hidden:?}");
    // A library loaded by a relative path, and the absolute path it reads as, sent: the sent path's
    // file name is shown for both, not the library's numbered id.
    let mut hidden = hiding(&[("loaded/data/icon.svg", "loaded/icon.svg (2)")], &[]);
    let absolute = std::path::absolute("data/icon.svg").unwrap().to_string_lossy().into_owned();
    hidden.set_hidden(vec![(format!("loaded/{absolute}"), "loaded/icon.svg".to_string())], false);
    let mut reply = json!({"error": format!("can't read \"data/icon.svg\" or {absolute}")});
    hide_under_a_root("sent-absolute", &mut reply, &hidden);
    assert_eq!(reply["error"], "can't read \"icon.svg\" or icon.svg", "{reply}");
}

/// Outside Windows, a `\` in a native path is part of a name: a sent path that differs from a hidden
/// library's path only by `/` in place of that `\` names another file, so the library keeps its id
/// and a reply does not read either path as a spelling of the other.
#[cfg(not(windows))]
#[test]
fn a_backslash_in_a_native_path_keeps_two_paths_apart() {
    let mut hidden = hiding(&[(r"loaded//out/weird\name.svg", r"loaded/weird\name.svg")], &[]);
    hidden.set_hidden(vec![("loaded//out/weird/name.svg".to_string(), "loaded/name.svg".to_string())], false);
    assert_eq!(hidden.hidden[0].1, r"loaded/weird\name.svg", "{hidden:?}");
    let mut reply = json!({"error": r"no /out/weird\name.svg, /out/weird/name.svg or \out\weird\name.svg"});
    hide_under_a_root("native-backslash", &mut reply, &hidden);
    assert_eq!(reply["error"], r"no weird\name.svg, name.svg or \out\weird\name.svg", "{reply}");
}

/// A sent path whose text overlaps part of a hidden library's path does not keep that library's path
/// from being hidden: the hidden libraries are matched first.
#[test]
fn a_sent_path_does_not_stop_a_hidden_library_path_from_being_hidden() {
    let mut hidden = hiding(&[("loaded//home/secret/dir/lib", "loaded/lib")], &[]);
    hidden.set_hidden(vec![("loaded//dir/lib/sent-folder/longer".to_string(), "loaded/longer".to_string())], false);
    let mut reply = json!({"error": "can't read /home/secret/dir/lib/sent-folder/longer"});
    hide_under_a_root("sent-overlap", &mut reply, &hidden);
    assert_eq!(reply["error"], "can't read lib/sent-folder/longer", "{reply}");
}

/// A hidden relative path is also hidden where a reply shows it as the absolute path it reads as from
/// the working folder.
#[test]
fn a_reply_hides_a_hidden_relative_path_written_as_absolute() {
    let hidden = hiding(&[("loaded/data/icon.svg", "loaded/icon.svg")], &[]);
    let absolute = std::path::absolute("data/icon.svg").unwrap().to_string_lossy().into_owned();
    let mut reply = json!({"error": format!("can't read {absolute}")});
    hide_under_a_root("relative-absolute", &mut reply, &hidden);
    assert_eq!(reply["error"], "can't read icon.svg", "{reply}");
}

/// The paths outside the read root that a request sends, as a path, a `file:` URL (also one that names
/// another host), under `~` or starting with `../`, or as a relative path in a path param, are hidden
/// by their file names; plain text and a path inside the root are not.
#[test]
fn a_request_sends_paths_that_an_error_hides() {
    let base = crate::tests_folder::Folder::new("sent-paths");
    let root = base.join("in");
    std::fs::create_dir_all(&root).unwrap();
    let inside = root.join("a.svg").to_string_lossy().into_owned();
    let keep = json!(["a/b", inside, "file://host/g.svg"]);
    let params = json!({"path": "/x/my lib/b.svg", "file": "outside/h.svg", "list": ["file:///y/c%20d.svg", "~/e.svg", "../f.svg"], "other": keep});
    let mut sent = Vec::new();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || super::sent_paths(&params, &mut sent));
    let pair = |path: &str, name: &str| (format!("loaded/{path}"), format!("loaded/{name}"));
    let want = [
        pair("outside/h.svg", "h.svg"),
        pair("/y/c d.svg", "c d.svg"),
        pair("~/e.svg", "e.svg"),
        pair("../f.svg", "f.svg"),
        pair("//host/g.svg", "g.svg"),
        pair("/x/my lib/b.svg", "b.svg"),
    ];
    assert_eq!(sent, want);
}

/// Under a read root, an absolute path or a `file:` URL outside it is unreadable; an empty string, a
/// relative name and a path inside the root are not.
#[test]
fn an_absolute_path_outside_the_read_root_is_unreadable() {
    let base = crate::tests_folder::Folder::new("unreadable-absolute");
    std::fs::create_dir_all(base.join("in")).unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
    let inside = base.join("in").join("a.svg").to_string_lossy().into_owned();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        assert!(super::unreadable_absolute("/outside/root/x"));
        assert!(super::unreadable_absolute("file:///outside/root/x"));
        assert!(super::unreadable_absolute("FILE:///outside/root/x"));
        assert!(super::unreadable_absolute("file://other-host/share/x"));
        let url = if inside.starts_with('/') { format!("File://{inside}") } else { format!("File:///{}", inside.replace('\\', "/")) };
        assert!(!super::unreadable_absolute(&url), "{url}");
        assert!(super::readable_value(&url), "{url}");
        let escaped = url.replacen("/in/a.svg", "/%69n/a.svg", 1);
        assert!(!super::unreadable_absolute(&escaped) && super::readable_value(&escaped), "{escaped}");
        assert!(super::unreadable_absolute("file:///outside%2Froot/x"));
        assert!(!super::unreadable_absolute(""));
        assert!(!super::unreadable_absolute("keep"));
        assert!(!super::unreadable_absolute(&inside));
    });
}

/// Under a read root, an absolute path or a `file:` URL outside it is written as a path also with
/// whitespace around it, and a recorded step's text param loses it; a path inside the root stays.
#[test]
fn a_spaced_absolute_path_outside_the_read_root_is_left_out() {
    let base = crate::tests_folder::Folder::new("unreadable-spaced");
    std::fs::create_dir_all(base.join("in")).unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
    let inside = format!(" {} ", base.join("in").join("a.svg").to_string_lossy());
    let mut params = json!({"target": " /outside/root/x", "name": "\tfile:///outside/root/x ", "keep": inside, "list": [" /outside/root/y"]});
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        assert!(super::unreadable_path_like(" /outside/root/x"));
        assert!(super::unreadable_path_like("\tfile:///outside/root/x "));
        assert!(!super::unreadable_path_like(&inside), "{inside}");
        super::leave_out_unreadable_params(params.as_object_mut().unwrap(), vectorcraft_engine::cmd::ParamsPlace::top(None));
    });
    assert_eq!(params, json!({"keep": inside, "list": []}));
}

/// Under a read root, an empty or blank value names no path, so a recorded step keeps it under a path
/// param and in a path param's list, a folder preference reset in a `prefs.set` step's `values`
/// included; a path outside the root is still left out.
#[test]
fn an_empty_path_param_stays_under_a_read_root() {
    let base = crate::tests_folder::Folder::new("unreadable-empty");
    std::fs::create_dir_all(base.join("in")).unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&base.join("in")], &[]).unwrap();
    let mut params = json!({
        "values": {"fontsFolder": "", "templatesFolder": " ", "recoveryFolder": "/outside/root/x"},
        "path": "", "destinationFolder": " ", "files": ["", "/outside/root/y"],
    });
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        super::leave_out_unreadable_step("prefs.set", params.as_object_mut().unwrap());
    });
    assert_eq!(params, json!({"values": {"fontsFolder": "", "templatesFolder": " "}, "path": "", "destinationFolder": " ", "files": [""]}));
}

/// Without a read root, a `file:` URL that names another host may be read, and a recorded step keeps
/// it.
#[test]
fn a_url_naming_another_host_stays_without_a_read_root() {
    assert!(!super::unreadable_absolute("file://host/share/a.svg"));
    assert!(super::readable_value("file://host/share/a.svg"));
    let mut params = json!({"path": "file://host/share/a.svg", "list": ["file://host/share/b.svg"]});
    let fields = params.as_object_mut().unwrap();
    super::leave_out_unreadable_params(fields, vectorcraft_engine::cmd::ParamsPlace::top(None));
    assert_eq!(params, json!({"path": "file://host/share/a.svg", "list": ["file://host/share/b.svg"]}));
}

/// Under a read root, a recorded step loses a relative path outside it under a key that names a file
/// or a folder as its command reads it (also inside a `command.batch` step), and a `file:` URL outside
/// it under any key. A web link, a choice or a switch whose name ends like a path key's, and a key
/// that names part of the document in its command, keep their values; the same key in a command that
/// doesn't take it as a param names a file.
#[test]
fn recorded_steps_filter_only_unreadable_locations() {
    use vectorcraft_engine::cmd::ParamsPlace;
    let root = std::env::temp_dir().join(format!("vc-recorded-locations-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap();
    let steps = [
        ("document.compare", json!({"diffPath": "elsewhere/a.svg", "directory": "elsewhere", "keep": 1})),
        ("attributes.set", json!({"url": "https://example.com/elsewhere/a.html", "note": "kept"})),
        ("object.slice.options", json!({"url": "file:///elsewhere/a.html"})),
        ("object.path.simplify", json!({"wholePaths": "elsewhere/a", "alignToPath": "elsewhere/b", "linksFolder": "elsewhere/c"})),
        ("document.exportPresets.save", json!({"settings": {"folders": "item/format", "formats": [{"folder": "elsewhere/png"}]}})),
        ("text.createInPath", json!({"path": "elsewhere/shape"})),
        ("object.setLiveShape", json!({"path": "elsewhere/shape"})),
        ("command.batch", json!({"commands": [{"command": "document.save", "params": {"path": "elsewhere/b.svg"}}]})),
    ];
    let filtered: Vec<Value> = vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        steps
            .iter()
            .map(|(id, params)| {
                let mut params = params.clone();
                super::leave_out_unreadable_params(params.as_object_mut().unwrap(), ParamsPlace::of(id));
                params
            })
            .collect()
    });
    let _ = std::fs::remove_dir_all(root);
    let want = [
        json!({"keep": 1}),
        json!({"url": "https://example.com/elsewhere/a.html", "note": "kept"}),
        json!({}),
        json!({"wholePaths": "elsewhere/a", "alignToPath": "elsewhere/b", "linksFolder": "elsewhere/c"}),
        json!({"settings": {"folders": "item/format", "formats": [{}]}}),
        json!({"path": "elsewhere/shape"}),
        json!({}),
        json!({"commands": [{"command": "document.save", "params": {}}]}),
    ];
    assert_eq!(filtered, want);
}

/// A request's relative path outside the read root is hidden in a reply where the command it calls
/// reads it as a file, and a web link or a key that names part of the document there is not.
#[test]
fn a_request_sends_paths_as_its_command_reads_them() {
    let root = std::env::temp_dir().join(format!("vc-sent-command-paths-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap();
    let mut sent = Vec::new();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        super::sent_paths(&json!({"command": "attributes.set", "params": {"url": "https://example.com/a/page.html"}}), &mut sent);
        super::sent_paths(&json!({"command": "object.path.simplify", "params": {"wholePaths": "out/w.svg"}}), &mut sent);
        super::sent_paths(&json!({"command": "document.compare", "params": {"diffPath": "out/d.svg"}}), &mut sent);
        super::sent_paths(&json!({"command": "file.place", "params": {"directory": "out/folder"}}), &mut sent);
    });
    let _ = std::fs::remove_dir_all(root);
    let pair = |path: &str, name: &str| (format!("loaded/{path}"), format!("loaded/{name}"));
    assert_eq!(sent, [pair("out/d.svg", "d.svg"), pair("out/folder", "folder")]);
}

/// A hidden path quoted with `{:?}` is hidden with its quotes and control characters escaped.
#[test]
fn a_reply_hides_a_quoted_path_with_escaped_quotes() {
    let hidden = hiding(&[("loaded//outside/a\"b\t.svg", "loaded/a\"b\t.svg")], &[]);
    let mut reply = json!({"error": format!("can't read {:?}", "/outside/a\"b\t.svg")});
    hide_under_a_root("quoted", &mut reply, &hidden);
    assert_eq!(reply["error"], "can't read \"a\\\"b\\t.svg\"", "{reply}");
}

/// Each replacement is chosen from the text as given: a hidden path inside the id shown for another
/// hidden library is not replaced again, and a readable id written with `%XX` escapes keeps the hidden
/// path it begins with.
#[test]
fn a_reply_replaces_each_hidden_text_once() {
    let hidden = hiding(&[("loaded//b.svg", "loaded/b.svg")], &[]);
    let mut reply = json!({"error": "no loaded//b.svg"});
    hide_under_a_root("root-file", &mut reply, &hidden);
    assert_eq!(reply["error"], "no loaded/b.svg", "{reply}");
    let hidden = hiding(&[("loaded//art/in", "loaded/in")], &["loaded//art/in work/a.svg"]);
    let mut reply = json!({"error": "file:///art/in%20work/a.svg"});
    hide_under_a_root("escaped-readable", &mut reply, &hidden);
    assert_eq!(reply["error"], "file:///art/in%20work/a.svg", "{reply}");
    // A URL whose scheme another hidden path took already is cut at its path.
    let hidden = hiding(&[("loaded//a", "loaded/a"), ("loaded/file", "loaded/file")], &[]);
    let mut reply = json!({"error": "file:///a"});
    hide_under_a_root("scheme-taken", &mut reply, &hidden);
    assert_eq!(reply["error"], "file://a", "{reply}");
    // Keys that come to read the same keep every field; a key that is not rewritten keeps its name.
    let hidden = hiding(&[("loaded//out/a.svg", "loaded/a.svg")], &[]);
    let mut reply = json!({"result": {"a.svg": 1, "file:///out/a.svg": 2, "/out/a.svg": 3}});
    hide_under_a_root("same-keys", &mut reply, &hidden);
    assert_eq!(reply["result"]["a.svg"], 1, "{reply}");
    let mut values: Vec<_> = reply["result"].as_object().map(|o| o.values().filter_map(Value::as_u64).collect()).unwrap_or_default();
    values.sort_unstable();
    assert_eq!(values, [1, 2, 3], "{reply}");
}

/// A library saved with another platform's absolute path is hidden, also when the working folder is
/// inside the read root.
#[test]
fn a_library_saved_with_another_platforms_path_is_hidden() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.ui.color_guide_limit = r"loaded/C:\Users\someone\b.svg".to_string();
    let here = std::env::current_dir().unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&here], &[]).unwrap();
    let hiding = vectorcraft_engine::file_access::confine(roots.as_ref(), || super::unreadable_library_ids(&app));
    assert_eq!(hiding.hidden, [(r"loaded/C:\Users\someone\b.svg".to_string(), "loaded/b.svg".to_string())], "{hiding:?}");
}

/// Another platform's absolute path is outside the read root, also when the working folder is inside
/// it: a request that sends one sees it by its file name.
#[test]
fn another_platforms_absolute_path_is_unreadable() {
    let here = std::env::current_dir().unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&here], &[]).unwrap();
    let path = r"C:\Users\someone\private.svg";
    let mut sent = Vec::new();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        assert!(!super::readable(path) && super::unreadable_absolute(path));
        super::sent_paths(&json!({"list": [path]}), &mut sent);
    });
    assert_eq!(sent, [(format!("loaded/{path}"), "loaded/private.svg".to_string())]);
}

/// A hidden relative path is hidden where it stands alone or after leading `./` and `../` parts, and
/// stays where it ends a longer path, also one with a `.` or `..` part before it.
#[test]
fn a_reply_keeps_a_longer_path_that_ends_with_a_hidden_relative_path() {
    let hidden = hiding(&[("loaded/data/icon.svg", "loaded/icon.svg")], &[]);
    let mut reply = json!({"result": "kept /work/data/icon.svg and xdata/icon.svg, hid ./data/icon.svg, \"data/icon.svg\" and loaded/data/icon.svg"});
    hide_under_a_root("relative", &mut reply, &hidden);
    assert_eq!(reply["result"], "kept /work/data/icon.svg and xdata/icon.svg, hid ./icon.svg, \"icon.svg\" and loaded/icon.svg", "{reply}");
    let mut reply = json!({"result": "kept /allowed/./data/icon.svg, /allowed/../data/icon.svg and a./data/icon.svg, hid ../data/icon.svg and './../data/icon.svg'"});
    hide_under_a_root("relative-dots", &mut reply, &hidden);
    assert_eq!(
        reply["result"], "kept /allowed/./data/icon.svg, /allowed/../data/icon.svg and a./data/icon.svg, hid ../icon.svg and './../icon.svg'",
        "{reply}"
    );
}

/// Outside Windows a `\` is part of a file name: a sent path outside the read root is shown by its
/// whole file name.
#[cfg(not(windows))]
#[test]
fn a_sent_path_keeps_a_backslash_in_its_file_name() {
    let root = crate::tests_folder::Folder::new("sent-backslash");
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[root.as_path()], &[]).unwrap();
    let mut sent = Vec::new();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || super::sent_paths(&json!({"path": "/out/weird\\name.svg"}), &mut sent));
    assert_eq!(sent, [("loaded//out/weird\\name.svg".to_string(), "loaded/weird\\name.svg".to_string())]);
}

/// `ui.inspect` and `ui.docking` show a hidden library whose file is at the top of the file system
/// (`/b.svg`) as `loaded/b.svg`, with its separator.
#[test]
fn a_hidden_library_at_the_top_of_the_file_system_keeps_its_shown_id() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let panel = "library:swatches:loaded//b.svg".to_string();
    app.ui.color_guide_limit = "loaded//b.svg".to_string();
    app.ui.open_panel = Some(panel);
    let root = crate::tests_folder::Folder::new("top-level-library");
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[root.as_path()], &[]).unwrap();
    let (inspected, docking) = vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        (input_request(&mut app, "ui.inspect", json!({})), input_request(&mut app, "ui.docking", json!({})))
    });
    assert_eq!(inspected["result"]["ui"]["color_guide_limit"], "loaded/b.svg", "{inspected}");
    assert_eq!(inspected["result"]["ui"]["open_panel"], "library:swatches:loaded/b.svg", "{inspected}");
    assert_eq!(docking["result"]["flyout"], "library:swatches:loaded/b.svg", "{docking}");
}

/// `ui.inspect` shows a recovery folder outside the read root that the status text names (a failed
/// recovery copy) by its folder name, also in the paths under it: the folder in use, and one a
/// failure named before the preference moved on. A readable one stays.
#[test]
fn a_recovery_folder_outside_the_read_root_is_hidden_in_the_status() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.session.recovery.set_default_folder(Some("/outside/recovery".into()));
    let root = crate::tests_folder::Folder::new("recovery-status");
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[root.as_path()], &[]).unwrap();
    let inspect = |app: &mut VectorcraftApp, status: String| {
        app.ui.status = status;
        let reply = vectorcraft_engine::file_access::confine(roots.as_ref(), || input_request(app, "ui.inspect", json!({})));
        reply["result"]["ui"]["status"].clone()
    };
    let status = "Couldn't save recovery data: /outside/recovery/a-1 is not a folder. Copies go to /outside/recovery".to_string();
    assert_eq!(inspect(&mut app, status), "Couldn't save recovery data: recovery/a-1 is not a folder. Copies go to recovery");
    let inside = root.as_path().join("copies").to_string_lossy().into_owned();
    app.session.prefs.recovery_folder = inside.clone();
    app.recovery.named = vec!["/outside/earlier".into()];
    assert_eq!(inspect(&mut app, "Copies go to /outside/earlier".into()), "Copies go to earlier");
    assert_eq!(inspect(&mut app, format!("Copies go to {inside}")), json!(format!("Copies go to {inside}")));
}

/// A path outside the read root that a request sent is shown by its file name in a reply that succeeded
/// as in one that failed.
#[test]
fn a_sent_path_reads_the_same_whatever_the_outcome() {
    let app = VectorcraftApp::new(Session::new(), Services::default());
    let root = crate::tests_folder::Folder::new("sent-outcome");
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[root.as_path()], &[]).unwrap();
    let sent = json!({"path": "/outside/b.svg"});
    let mut done = json!({"ok": true, "result": {"path": "/outside/b.svg"}});
    let mut failed = json!({"ok": false, "error": "can't read /outside/b.svg"});
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        super::hide_in_reply(&app, &mut done, &sent, super::Hiding::default());
        super::hide_in_reply(&app, &mut failed, &sent, super::Hiding::default());
    });
    assert_eq!(done["result"]["path"], "b.svg", "{done}");
    assert_eq!(failed["error"], "can't read b.svg", "{failed}");
}

#[test]
fn deferred_pointer_reply_uses_the_gesture_document_after_file_new() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let (reply, received) = std::sync::mpsc::channel();
    let at = app.session.input_count();
    app.pointer_replies.push(super::PendingPointerReply { from: at, to: at, document: app.session.active().map(|d| d.uid), reply });

    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
    app.answer_pointer_replies(&egui::Context::default());

    let result = received.try_recv().unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["result"]["selection"], json!([id]));
    assert_eq!(result["result"]["key"], Value::Null);
    assert_eq!(result["result"]["tool"], "rectangle");
}

#[test]
fn control_lists_and_selects_optional_addon_tools() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let listed = input_request(&mut app, "ui.tool.list", json!({}));
    assert_eq!(listed["ok"], true);
    let groups = listed["result"].as_array().unwrap();
    for tool in vectorcraft_tools::addons::GROUPS.iter().flat_map(|group| group.iter()) {
        let matching: Vec<_> = groups.iter().flat_map(|group| group.as_array().unwrap()).filter(|entry| entry["id"] == tool.id).collect();
        assert_eq!(matching.len(), 1, "{}", tool.id);
        assert_eq!(matching[0]["label"], tool.label);
        assert!(matching[0]["shortcut"].is_null());
        let selected = input_request(&mut app, "ui.tool.select", json!({"tool": tool.id}));
        assert_eq!(selected["ok"], true, "{selected}");
        assert_eq!(selected["result"]["tool"], tool.id);
    }
}

#[test]
fn ui_set_turns_the_image_trace_preview_off_and_on() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(app.ui.image_trace_preview);
    for on in [false, true] {
        let r = input_request(&mut app, "ui.set", json!({"imageTracePreview": on}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.ui.image_trace_preview, on);
    }
}

/// #3797: `ui.set` `outline` sets the active document's view; without a document it is an error
/// and sets nothing else either.
#[test]
fn ui_set_outline_sets_the_active_documents_view() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    for params in [json!({"outline": false}), json!({"outline": true, "grid": true})] {
        let r = input_request(&mut app, "ui.set", params);
        assert_eq!(r["ok"], false, "{r}");
    }
    assert!(!app.ui.view.grid, "nothing is set");
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = input_request(&mut app, "ui.set", json!({"outline": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(app.outline_view());
    assert_eq!(app.views.iter().map(|v| v.outline).collect::<Vec<_>>(), [false, true], "the other document stays in Preview");
}

#[test]
fn the_image_trace_preview_is_a_command() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(crate::menus::UI_COMMANDS.iter().any(|c| c.0 == "imageTrace.preview"));
    assert_eq!(app.run("imageTrace.preview", json!({})), Ok(json!(false)), "toggles");
    assert!(!app.ui.image_trace_preview);
    assert_eq!(app.run("imageTrace.preview", json!({"on": false})), Ok(json!(false)), "stays off");
    assert_eq!(app.run("imageTrace.preview", json!({"on": true})), Ok(json!(true)));
    assert!(app.ui.image_trace_preview);
    assert!(app.run("imageTrace.preview", json!({"on": "yes"})).is_err());
    assert!(app.ui.image_trace_preview, "a bad value leaves it");
    let r = input_request(&mut app, "engine.execute", json!({"command": "imageTrace.preview", "params": {"on": false}}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!app.ui.image_trace_preview, "the control channel reaches it");
}

#[test]
fn control_input_rejects_excessive_counts_before_mutation() {
    // Only modest over-limit values on the baseline: never exercise an unbounded loop.
    for (method, params) in [("ui.click", json!({"count": 17})), ("ui.drag", json!({"steps": 257})), ("ui.drag", json!({"steps": 2, "count": 17}))] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.synthetic.push(egui::Event::Text("already queued".into()));
        let before = format!("{:?}", app.synthetic);
        let r = input_request(&mut app, method, params);
        assert_eq!(r["ok"], false, "oversized input must be rejected: {r}");
        assert_eq!(format!("{:?}", app.synthetic), before, "rejection must preserve the entire queue");
    }
}

#[test]
fn control_input_respects_existing_queue_budget() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.synthetic = vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)); 1023];
    let before = format!("{:?}", app.synthetic);
    let r = input_request(&mut app, "ui.click", json!({}));
    assert_eq!(r["ok"], false, "a whole click must fit before any part is appended");
    assert_eq!(format!("{:?}", app.synthetic), before);
}

#[test]
fn control_input_yields_control_channel_after_frame_budget() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx);
    let mut replies = vec![];
    for _ in 0..65 {
        let (req, reply) = ControlRequest::new("ui.move", json!({"x": 1, "y": 2}));
        tx.send(req).unwrap();
        replies.push(reply);
    }
    app.drain_control(&egui::Context::default());
    assert_eq!(replies.iter().filter(|r| r.try_recv().is_ok()).count(), 64);
    assert_eq!(app.synthetic.len(), 64);
    app.drain_control(&egui::Context::default());
    assert_eq!(replies.last().unwrap().try_recv().unwrap()["ok"], true);
    assert_eq!(app.synthetic.len(), 65);
}

#[test]
fn control_input_rejects_invalid_and_extreme_counts() {
    for value in [json!(u64::MAX), json!(-1), json!(1.5), json!("2"), Value::Null] {
        for (method, key) in [("ui.click", "count"), ("ui.drag", "count"), ("ui.drag", "steps")] {
            let mut app = VectorcraftApp::new(Session::new(), Services::default());
            app.synthetic.push(egui::Event::Text("pending".into()));
            let before = format!("{:?}", app.synthetic);
            let mut params = json!({});
            params[key] = value.clone();
            let r = input_request(&mut app, method, params);
            assert_eq!(r["ok"], false, "{method}/{key}: {r}");
            assert!(r["error"].as_str().unwrap().contains(key));
            assert_eq!(format!("{:?}", app.synthetic), before);
        }
    }
}

#[test]
fn control_input_accepts_boundary_gestures_and_preserves_event_order() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let r = input_request(
        &mut app,
        "ui.drag",
        json!({"x": 2, "y": 3, "toX": 20, "toY": 30,
        "steps": 256, "count": 16, "button": "right", "shift": true}),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(app.synthetic.len(), 289);
    assert!(matches!(app.synthetic.first(), Some(egui::Event::PointerMoved(p)) if *p == egui::pos2(2.0, 3.0)));
    assert!(
        matches!(app.synthetic.get(1), Some(egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Secondary, modifiers, .. }) if modifiers.shift)
    );
    assert!(matches!(app.synthetic.get(257), Some(egui::Event::PointerMoved(p)) if *p == egui::pos2(20.0, 30.0)));
    assert!(matches!(app.synthetic.get(258), Some(egui::Event::PointerButton { pressed: false, .. })));
    assert!(matches!(app.synthetic.last(), Some(egui::Event::PointerButton { pressed: false, pos, .. }) if *pos == egui::pos2(20.0, 30.0)));
    for (method, params, events) in [
        ("ui.click", json!({}), 3),
        ("ui.click", json!({"count": 2}), 5),
        ("ui.drag", json!({}), 11),
        ("ui.drag", json!({"steps": 0, "count": 0}), 4),
    ] {
        app.synthetic.clear();
        assert_eq!(input_request(&mut app, method, params)["ok"], true);
        assert_eq!(app.synthetic.len(), events);
    }
    app.synthetic = vec![egui::Event::PointerMoved(egui::pos2(0.0, 0.0)); 1021];
    assert_eq!(input_request(&mut app, "ui.click", json!({}))["ok"], true);
    assert_eq!(app.synthetic.len(), 1024, "the exact queue boundary is permitted");
}

#[test]
fn control_input_budget_covers_every_synthetic_method() {
    for (method, params) in [
        ("ui.key", json!({"key": "Enter", "text": "x"})),
        ("ui.move", json!({})),
        ("ui.wheel", json!({"dy": 1})),
        ("ui.text", json!({"text": "hello"})),
        ("ui.click", json!({})),
        ("ui.drag", json!({"steps": 1})),
    ] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.synthetic = vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)); 1024];
        let before = format!("{:?}", app.synthetic);
        let r = input_request(&mut app, method, params);
        assert_eq!(r["ok"], false, "{method}: {r}");
        assert_eq!(format!("{:?}", app.synthetic), before);
        app.synthetic.clear();
        assert_eq!(input_request(&mut app, method, json!({"key": "Enter", "steps": 1}))["ok"], true, "queue space becomes usable after draining");
    }
}

/// `ui.dialog.set {fields}` stores the whole list in order, or nothing when an item is refused.
#[test]
fn dialog_set_stores_a_field_list_whole_or_not_at_all() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.ui.dialog = Some(crate::state::Dialog::new("move", json!({"dx": "0 pt", "dy": "0 pt"})));
    let fields = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| serde_json::to_value(&d.fields).unwrap());
    let before = fields(&app);
    for params in [
        json!({"fields": [{"field": "copy", "value": true}, {"value": 1}]}),
        json!({"fields": [{"field": "copy", "value": true}, 7]}),
        json!({"fields": {"field": "copy"}}),
        json!({"field": "dx", "fields": [{"field": "copy", "value": true}]}),
        json!({"fields": [{"field": "copy", "value": true}, {"field": "note"}]}),
    ] {
        let r = input_request(&mut app, "ui.dialog.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(fields(&app), before, "{params}: a refused list stores no field");
    }
    let list = json!([{"field": "copy", "value": true}, {"field": "dx", "value": "3 pt"}, {"field": "dx", "value": "5 pt"}, {"field": "dy"}]);
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": list}));
    assert_eq!(r["ok"], true, "{r}");
    let after = fields(&app).unwrap();
    assert_eq!(after, json!({"copy": true, "dx": "5 pt", "dy": null}));
    assert_eq!(r["result"]["fields"], after);
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "dy", "value": "2 pt"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(fields(&app).unwrap()["dy"], "2 pt");
    app.ui.dialog = None;
    assert_eq!(input_request(&mut app, "ui.dialog.set", json!({"fields": []}))["error"], "no dialog open");
}

/// `ui.dialog.set {fields}` stores at most `MAX_DIALOG_FIELDS` items; a longer list stores none.
#[test]
fn dialog_set_refuses_a_field_list_over_the_limit() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.ui.dialog = Some(crate::state::Dialog::new("move", json!({"dx": "0 pt"})));
    let list = |n: usize| Value::Array((0..n).map(|i| json!({"field": "dx", "value": format!("{i} pt")})).collect());
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": list(MAX_DIALOG_FIELDS + 1)}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"], format!("`fields` holds {} items; at most {MAX_DIALOG_FIELDS}", MAX_DIALOG_FIELDS + 1));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["dx"].clone()), Some(json!("0 pt")), "a refused list stores no field");
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": list(MAX_DIALOG_FIELDS)}));
    assert_eq!(r["ok"], true, "{r}");
    let last = format!("{} pt", MAX_DIALOG_FIELDS - 1);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["dx"].clone()), Some(json!(last)), "the whole list is stored, in order");
}

/// `ui.dialog.set {fields, action}` confirms or cancels the dialog it set the fields in, within the
/// same request, and replies `{dialog, result}`. A bad `action` or a refused field sets nothing and
/// leaves the dialog open.
#[test]
fn dialog_set_with_an_action_answers_the_same_dialog() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let open = |app: &mut VectorcraftApp| app.ui.dialog = Some(crate::state::Dialog::new("note", json!({"text": "a"})));
    let text = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.fields["text"].clone());
    open(&mut app);
    for params in [
        json!({"fields": [{"field": "text", "value": "b"}], "action": "ok"}),
        json!({"fields": [{"field": "text", "value": "b"}], "action": true}),
        json!({"fields": [{"field": "text", "value": "b"}], "action": null}),
        json!({"fields": [{"field": "text", "value": "b"}, {"field": "nope", "value": 1}], "action": "confirm"}),
    ] {
        let r = input_request(&mut app, "ui.dialog.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(text(&app), Some(json!("a")), "{params}: the dialog stays open as it was");
    }
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": [{"field": "text", "value": "b"}], "action": "confirm"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"], json!({"dialog": {"kind": "note", "fields": {"text": "b"}}, "result": null}));
    assert!(app.ui.dialog.is_none(), "confirmed in the same request");
    open(&mut app);
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": [{"field": "text", "value": "c"}], "action": "cancel"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"], json!({"dialog": {"kind": "note", "fields": {"text": "c"}}, "result": null}));
    assert!(app.ui.dialog.is_none(), "canceled in the same request");
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": [], "action": "confirm"}));
    assert_eq!(r["error"], "no dialog open");
}

/// #1004: `ui.render` (the MCP `screenshot` of a connected app) renders the artboard asked
/// for, at the scale asked for; the first without one.
#[test]
fn render_takes_the_artboard_asked_for() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx);
    let mut request = |method: &str, params: Value| {
        let (req, reply) = ControlRequest::new(method, params);
        tx.send(req).unwrap();
        app.drain_control(&egui::Context::default());
        reply.try_recv().unwrap()
    };
    let new = json!({"width": 64, "height": 64, "units": "Points", "artboards": 2});
    request("engine.execute", json!({"command": "file.new", "params": new}));
    let board = json!({"index": 1, "x": 100, "y": 0, "width": 96, "height": 48});
    let r = request("engine.execute", json!({"command": "artboard.setProps", "params": board}));
    assert_eq!(r["ok"], true, "{r}");
    for (params, size) in [
        (json!({}), [64, 64]),
        (json!({"artboard": 0}), [64, 64]),
        (json!({"artboard": 1}), [96, 48]),
        (json!({"artboard": 1, "scale": 0.5}), [48, 24]),
    ] {
        let r = request("ui.render", params.clone());
        let got = [&r["result"]["width"], &r["result"]["height"]].map(Value::as_u64);
        assert_eq!(got, size.map(Some), "{params}: {r}");
    }
    assert!(request("ui.render", json!({"artboard": 2}))["error"].is_string());
    for (params, key) in [
        (json!({"artboard": "1"}), "artboard"),
        (json!({"artboard": -1}), "artboard"),
        (json!({"scale": "0.5"}), "scale"),
        (json!({"traceViews": 1}), "traceViews"),
        (json!({"data": "yes"}), "data"),
    ] {
        let r = request("ui.render", params.clone());
        assert!(r["error"].as_str().is_some_and(|e| e.contains(&format!("`{key}`"))), "{params}: {r}");
    }
    let r = request("ui.render", json!({"artboard": null, "scale": null}));
    assert_eq!([&r["result"]["width"], &r["result"]["height"]].map(Value::as_u64), [Some(64), Some(64)], "null keeps the default: {r}");
}

/// The control channel's input methods refuse a number or switch of the wrong type before they
/// queue any event; omitting it keeps its default.
#[test]
fn control_input_refuses_params_of_the_wrong_type() {
    let move_to = |mods: Value| json!({"events": [{"kind": "move", "x": 5, "y": 5}, {"kind": "move", "x": 6, "y": 6, "mods": mods}]});
    for (method, params, key) in [
        ("ui.move", json!({"x": "10"}), "x"),
        ("ui.move", json!({"y": true}), "y"),
        ("ui.wheel", json!({"dy": "1"}), "dy"),
        ("ui.wheel", json!({"dy": 1, "shift": "yes"}), "shift"),
        ("ui.click", json!({"x": [1]}), "x"),
        ("ui.click", json!({"cmd": 1}), "cmd"),
        ("ui.drag", json!({"toX": "5"}), "toX"),
        ("ui.key", json!({"key": "Enter", "alt": "true"}), "alt"),
        ("ui.resize", json!({"width": "800"}), "width"),
        ("ui.screenshot", json!({"focus": "no"}), "focus"),
        ("ui.screenshot", json!({"data": 1}), "data"),
        ("ui.set", json!({"rulers": "true"}), "rulers"),
        ("ui.set", json!({"grid": true, "controlBar": 0}), "controlBar"),
        ("ui.pointer", json!({"events": [{"kind": "move", "x": "5", "y": 0}]}), "x"),
        ("ui.pointer", move_to(json!({"shift": "yes"})), "mods"),
        ("ui.pointer", move_to(json!([true])), "mods"),
        ("ui.pointer", json!({"mods": "shift", "events": [{"kind": "move", "x": 1, "y": 1}]}), "mods"),
    ] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.synthetic.push(egui::Event::Text("pending".into()));
        let before = (format!("{:?}", app.synthetic), app.ui.view.rulers, app.ui.view.grid, app.ui.control_bar);
        let r = input_request(&mut app, method, params.clone());
        assert_eq!(r["ok"], false, "{method} {params}: {r}");
        assert!(r["error"].as_str().is_some_and(|e| e.contains(&format!("`{key}`"))), "{method} {params}: {r}");
        let after = (format!("{:?}", app.synthetic), app.ui.view.rulers, app.ui.view.grid, app.ui.control_bar);
        assert_eq!(after, before, "{method} {params}: nothing is queued or set");
    }
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert_eq!(input_request(&mut app, "ui.move", json!({"x": null, "y": 4}))["ok"], true);
    assert!(matches!(app.synthetic.first(), Some(egui::Event::PointerMoved(p)) if *p == egui::pos2(0.0, 4.0)));
}

/// The app's own commands refuse a param of the wrong type and change nothing; omitting it or
/// giving null keeps its default.
#[test]
fn ui_commands_refuse_params_of_the_wrong_type() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("view.setZoom", json!({"zoom": 300, "center": [5, 5]})).unwrap();
    let view = |app: &VectorcraftApp| app.view().map(|v| (v.zoom, v.center.x, v.center.y));
    let before = (view(&app), app.ui.screen_mode, app.session.documents().len(), app.ui.dialog.is_some(), app.ui.open_panel.clone());
    for (id, params, key) in [
        ("view.setZoom", json!({"zoom": "200"}), "zoom"),
        ("view.setZoom", json!({"zoom": 200, "center": [1]}), "center"),
        ("view.setZoom", json!({"zoom": 200, "center": "0,0"}), "center"),
        ("view.screenMode", json!({"mode": "2"}), "mode"),
        ("view.screenMode", json!({"mode": 1.5}), "mode"),
        ("window.panel", json!({"panel": "layers", "toggle": "true"}), "toggle"),
        ("window.panel", json!({"panel": "layers", "toggle": true, "toggleGroup": 1}), "toggleGroup"),
        ("file.revert", json!({"confirmed": "yes"}), "confirmed"),
        ("file.close", json!({"index": "0"}), "index"),
        ("ui.newSwatch", json!({"spot": "true"}), "spot"),
        ("ui.newSwatch", json!({"global": 1}), "global"),
        ("ui.symbolOptions", json!({"edit": 1}), "edit"),
        ("ui.batchDialog", json!({"set": "1"}), "set"),
        ("ui.batchDialog", json!({"action": -1}), "action"),
        ("file.exportSelection", json!({"multiple": "true"}), "multiple"),
    ] {
        let e = app.run(id, params.clone()).unwrap_err();
        assert!(e.contains(&format!("`{key}`")), "{id} {params}: {e}");
        let after = (view(&app), app.ui.screen_mode, app.session.documents().len(), app.ui.dialog.is_some(), app.ui.open_panel.clone());
        assert_eq!(after, before, "{id} {params}: nothing changes");
    }
    app.run("view.setZoom", json!({"zoom": null, "center": null})).unwrap();
    assert_eq!(app.view().map(|v| v.zoom), Some(crate::state::zoom_from_display_percent(100.0, app.session.prefs.display_print_size)));
}

#[test]
fn synthetic_pointer_backquote_keeps_solid_object_in_place() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let tile = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("object.pattern.make", json!({"ids": [tile], "name": "Dots", "width": 20, "height": 20})).unwrap();
    app.run("object.pattern.done", json!({})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
    let solid = app.run("shape.rectangle", json!({"x": 220, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [solid], "color": "#3366cc"})).unwrap();
    app.run("select.set", json!({"ids": [id, solid]})).unwrap();
    let before = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let before_solid = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(solid)).unwrap().clone();
    let r = input_request(
        &mut app,
        "ui.pointer",
        json!({"events": [
        {"kind": "down", "x": 125, "y": 125},
        {"kind": "drag", "x": 145, "y": 125},
        {"kind": "up", "x": 145, "y": 125}
    ], "backquote": true}),
    );
    assert_eq!(r["ok"], true, "{r}");
    let after = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    assert_eq!(after.geometric_bounds(), before.geometric_bounds());
    assert_eq!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(solid)).unwrap(), &before_solid);
    let vectorcraft_color::Paint::Pattern { xf, .. } = after.appearance.fill_paint() else { panic!("pattern fill expected") };
    assert_eq!(xf.as_coeffs()[4], 20.0, "synthetic backquote drag changes pattern paint");
}

/// `ui.pointer` rejects a `backquote` that is not a boolean, given for the request or for any
/// event, before any of the events runs.
#[test]
fn synthetic_pointer_rejects_a_backquote_that_is_not_a_boolean() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(input_request(&mut app, "ui.tool.select", json!({"tool": "selection"}))["ok"], true);
    let before = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let down = json!({"kind": "down", "x": 125, "y": 125});
    let drag = json!({"kind": "drag", "x": 145, "y": 125});
    for params in [
        json!({"events": [down, drag, {"kind": "up", "x": 145, "y": 125}], "backquote": "true"}),
        json!({"events": [down, drag, {"kind": "up", "x": 145, "y": 125, "backquote": 1}]}),
    ] {
        let r = input_request(&mut app, "ui.pointer", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert!(r["error"].as_str().is_some_and(|e| e.contains("`backquote` must be a boolean")), "{r}");
        assert_eq!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap(), &before, "{params}");
    }
}

/// `ui.pointer` refuses an unknown event kind, a pen value or `space` of the wrong type, or a
/// screen-space event with no canvas, before any of the packet's events runs.
#[test]
fn synthetic_pointer_rejects_an_unknown_kind_before_any_event_runs() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(input_request(&mut app, "ui.tool.select", json!({"tool": "selection"}))["ok"], true);
    let before = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let moves = [json!({"kind": "down", "x": 125, "y": 125}), json!({"kind": "drag", "x": 145, "y": 125}), json!({"kind": "up", "x": 145, "y": 125})];
    for (last, error) in [
        (json!({"kind": "wiggle", "x": 145, "y": 125}), "unknown pointer kind `wiggle`"),
        (json!({"kind": "move", "x": 145, "y": 125, "space": "screen"}), "no canvas yet"),
        (json!({"kind": "move", "x": 145, "y": 125, "pressure": "bad"}), "`pressure` must be a number"),
        (json!({"kind": "move", "x": 145, "y": 125, "holdMs": "800"}), "`holdMs` must be a number"),
        (json!({"kind": "move", "x": 145, "y": 125, "tiltY": true}), "`tiltY` must be a number"),
        (json!({"kind": "move", "x": 145, "y": 125, "rotation": "90"}), "`rotation` must be a number"),
        (json!({"kind": "move", "x": 145, "y": 125, "space": "screeen"}), "`space` must be doc or screen"),
        (json!({"kind": "move", "x": 145, "y": 125, "space": true}), "`space` must be doc or screen"),
    ] {
        let events = moves.iter().cloned().chain([last]).collect::<Vec<_>>();
        let r = input_request(&mut app, "ui.pointer", json!({"events": events}));
        assert_eq!(r["ok"], false, "{error}: {r}");
        assert!(r["error"].as_str().is_some_and(|e| e.contains(error)), "{r}");
        assert_eq!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap(), &before, "{error}: the drag does not run");
    }
}

/// `ui.pointer` replies with the key object: a click on one of two selected objects makes it
/// the key, and a click on the key again lets it go.
#[test]
fn pointer_replies_name_the_key_object() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let ctx = egui::Context::default();
    let click = |app: &mut VectorcraftApp| {
        let events = json!([{"kind": "down", "x": 110, "y": 20}, {"kind": "up", "x": 110, "y": 20}]);
        let (req, _) = ControlRequest::new("ui.pointer", json!({"events": events}));
        let Outcome::Done(r) = handle(app, &ctx, &req) else { panic!("not done") };
        assert_eq!(r["ok"], true, "{r}");
        r["result"].get("key").cloned()
    };
    assert_eq!(click(&mut app), Some(b), "a click on a selected object makes it the key");
    assert_eq!(click(&mut app), Some(Value::Null), "a click on the key again lets it go");
}

/// A Shape Builder click on a large selection merges on another thread: the frames go on, and
/// the `ui.pointer` reply comes once the merge is applied, with the selection after it.
#[test]
fn pointer_reply_waits_for_a_merge_on_another_thread() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx);
    app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
    for k in 0..40 {
        app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
    let ctx = egui::Context::default();
    let send = |app: &mut VectorcraftApp, events: Value| {
        let (req, reply) = ControlRequest::new("ui.pointer", json!({"events": events}));
        tx.send(req).unwrap();
        app.drain_control(&ctx);
        reply
    };
    let hover = send(&mut app, json!([{"kind": "move", "x": 15, "y": 10}]));
    assert_eq!(hover.try_recv().unwrap()["ok"], true, "a hover is answered at once");
    app.session.tool_poll(app.view_info(), true).unwrap();
    let revision = app.session.active().unwrap().revision;
    let click = send(&mut app, json!([{"kind": "down", "x": 15, "y": 10}, {"kind": "up", "x": 15, "y": 10}]));
    assert!(click.try_recv().is_err() && app.session.tool_editing(), "the click is not answered before the merge is applied");
    // A hover sent meanwhile is answered at once; a click is made after the merge, and its
    // reply waits until then.
    let hover = send(&mut app, json!([{"kind": "move", "x": 35, "y": 10}]));
    assert_eq!(hover.try_recv().map(|r| r["ok"].clone()), Ok(json!(true)), "a hover during the merge");
    let cmd = json!({"cmd": true});
    let press = send(&mut app, json!([{"kind": "down", "x": 500, "y": 150, "mods": cmd}, {"kind": "up", "x": 500, "y": 150, "mods": cmd}]));
    assert!(press.try_recv().is_err(), "a Cmd-click during the merge waits for it");
    let hover = send(&mut app, json!([{"kind": "move", "x": 45, "y": 10}]));
    assert_eq!(hover.try_recv().map(|r| r["ok"].clone()), Ok(json!(true)), "a hover after the held Cmd-click");
    let start = std::time::Instant::now();
    let reply = loop {
        if let Ok(r) = click.try_recv() {
            break r;
        }
        assert!(app.session.tool_editing(), "the merge is being computed until the reply goes");
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "no reply");
        std::thread::sleep(std::time::Duration::from_millis(2));
        // A frame: the canvas checks on the tool, then the replies are answered.
        let r = app.session.tool_poll(app.view_info(), false);
        crate::canvas::apply_requests(&mut app, r);
        app.answer_pointer_replies(&ctx);
    };
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["selection"].as_array().map(Vec::len), Some(41), "{reply}: the selection after the merge");
    assert_ne!(app.session.active().unwrap().revision, revision);
    let press = loop {
        if let Ok(r) = press.try_recv() {
            break r;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "no reply to the Cmd-click");
        let r = app.session.tool_poll(app.view_info(), false);
        crate::canvas::apply_requests(&mut app, r);
        app.answer_pointer_replies(&ctx);
    };
    assert_eq!(press["result"]["selection"].as_array().map(Vec::len), Some(0), "{press}: the lent selection tool deselected after the merge");
    assert_eq!(press["result"]["tool"], "shapeBuilder", "{press}");
}

/// A Cmd-click held while a Shape Builder merge is computed on another thread makes the artboard
/// it lands in active once it is made, as a Cmd-click made at once does: by the frames' checks,
/// by choosing another tool first, or before a command that acts on the active artboard takes
/// it.
#[test]
fn a_held_cmd_click_makes_its_artboard_active() {
    for then in ["frames", "another tool", "All on Active Artboard", "another document"] {
        held_cmd_click(then);
    }
}

fn held_cmd_click(then: &str) {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    if then == "another document" {
        // A document with an artboard where the press is, whose view keeps its own.
        app.run("file.new", json!({"width": 1400, "height": 200})).unwrap();
        app.run("artboard.new", json!({"x": 1000, "y": 0, "width": 300, "height": 200})).unwrap();
        app.view_mut().unwrap().artboard = 0;
    }
    app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
    app.run("artboard.new", json!({"x": 1000, "y": 0, "width": 300, "height": 200})).unwrap();
    for k in 0..40 {
        app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
    app.view_mut().unwrap().artboard = 0;
    let view = app.view_info();
    let at = |kind, x, mods| PointerEvent { kind, pos: vectorcraft_geom::Point::new(x, 10.0), mods, pressure: 1.0, backquote: false, pen: None };
    crate::canvas::dispatch(&mut app, &at(PointerKind::Move, 15.0, Mods::default()), view);
    app.session.tool_poll(view, true).unwrap();
    crate::canvas::dispatch(&mut app, &at(PointerKind::Down, 15.0, Mods::default()), view);
    crate::canvas::dispatch(&mut app, &at(PointerKind::Up, 15.0, Mods::default()), view);
    let cmd = Mods { cmd: true, ..Default::default() };
    crate::canvas::dispatch(&mut app, &at(PointerKind::Down, 1100.0, cmd), view);
    crate::canvas::dispatch(&mut app, &at(PointerKind::Up, 1100.0, cmd), view);
    assert_eq!(app.view().unwrap().artboard, 0, "{then}: the Cmd-click is held");
    if then == "another tool" {
        app.select_tool("rectangle");
        assert!(!app.session.tool_editing(), "{then}");
    }
    if then == "All on Active Artboard" {
        app.run("select.allOnArtboard", json!({})).unwrap();
        assert!(app.session.doc().unwrap().selection.is_empty(), "{then}: the second artboard, which holds no art");
    }
    let doc = app.session.active_index().unwrap();
    if then == "another document" {
        // A document tab chosen.
        app.session.set_active(0);
    }
    while app.session.tool_waiting() {
        let r = app.session.tool_poll(view, true);
        crate::canvas::apply_requests(&mut app, r);
    }
    app.sync_views();
    assert_eq!(app.views[doc].artboard, 1, "{then}: the Cmd-click made the artboard it landed in active");
    if then == "another document" {
        assert_eq!(app.views[0].artboard, 0, "{then}: the other document's view keeps its artboard");
    }
}

/// A dialog chosen from the menu, or with `ui.menuDialog`, while a Shape Builder merge is
/// computed on another thread opens after the merge is applied, on the merged art. So do the
/// app's own dialogs (Recolor Artwork), from the menu or a panel's button.
#[test]
fn menu_dialogs_open_after_a_merge_on_another_thread() {
    for route in ["menu", "ui.menuDialog", "app dialog from the menu", "app dialog from a panel"] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
        let view = app.view_info();
        let at = |kind| PointerEvent {
            kind,
            pos: vectorcraft_geom::Point::new(15.0, 10.0),
            mods: Mods::default(),
            pressure: 1.0,
            backquote: false,
            pen: None,
        };
        crate::canvas::dispatch(&mut app, &at(PointerKind::Move), view);
        app.session.tool_poll(view, true).unwrap();
        crate::canvas::dispatch(&mut app, &at(PointerKind::Down), view);
        crate::canvas::dispatch(&mut app, &at(PointerKind::Up), view);
        assert!(app.session.tool_editing(), "{route}");
        match route {
            "menu" => crate::menus::invoke(&mut app, "object.move", json!({})),
            "ui.menuDialog" => {
                app.run("ui.menuDialog", json!({"command": "object.move"})).unwrap();
            }
            "app dialog from the menu" => crate::menus::invoke(&mut app, "ui.recolorDialog", json!({})),
            _ => {
                app.run("ui.recolorDialog", json!({})).unwrap();
            }
        }
        assert!(app.ui.dialog.is_some(), "{route}: the dialog is open");
        assert!(!app.session.tool_editing(), "{route}");
        let merged = app.session.doc().unwrap().history.undo.last().map(|e| e.label.clone());
        assert_eq!(merged.as_deref(), Some("Shape Builder"), "{route}");
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 41, "{route}: the dialog acts on the merged art");
    }
}

/// The Separations panel's checkboxes and a preference set while a Shape Builder merge is
/// computed on another thread apply the merge first, and the document keeps it. The checkboxes
/// leave the view off, so they apply it by command id without giving the documents new revisions.
#[test]
fn panels_that_redraw_apply_a_merge_on_another_thread_first() {
    let cases = [
        ("view.separationsPreview", json!({"on": false})),
        ("view.overprintPreview", json!({"on": false})),
        ("prefs.set", json!({"values": {"gridlineEvery": 90}})),
    ];
    for (id, params) in cases {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
        let view = app.view_info();
        let at = |kind| PointerEvent {
            kind,
            pos: vectorcraft_geom::Point::new(15.0, 10.0),
            mods: Mods::default(),
            pressure: 1.0,
            backquote: false,
            pen: None,
        };
        crate::canvas::dispatch(&mut app, &at(PointerKind::Move), view);
        app.session.tool_poll(view, true).unwrap();
        crate::canvas::dispatch(&mut app, &at(PointerKind::Down), view);
        crate::canvas::dispatch(&mut app, &at(PointerKind::Up), view);
        assert!(app.session.tool_editing(), "{id}");
        app.run(id, params).unwrap();
        assert!(!app.session.tool_editing(), "{id}");
        for _ in 0..3 {
            app.session.tool_poll(view, false).unwrap();
        }
        let merged = app.session.doc().unwrap().history.undo.last().map(|e| e.label.clone());
        assert_eq!(merged.as_deref(), Some("Shape Builder"), "{id}");
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 41, "{id}");
    }
}

/// Saving, exporting or closing while a Shape Builder merge is computed on another thread
/// applies the merge first: the file holds it, and closing asks to save it.
#[test]
fn save_and_close_apply_a_merge_on_another_thread_first() {
    let dir = crate::tests_folder::Folder::adopt(vectorcraft_testkit::temp_dir("control-merge-save"));
    for then in ["file.saveAs", "file.exportAs", "file.close"] {
        let services = Services { write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))), ..Default::default() };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
        // As if just saved: only the merge modifies it.
        app.session.doc_mut().unwrap().mark_saved();
        let view = app.view_info();
        let at = |kind| PointerEvent {
            kind,
            pos: vectorcraft_geom::Point::new(15.0, 10.0),
            mods: Mods::default(),
            pressure: 1.0,
            backquote: false,
            pen: None,
        };
        crate::canvas::dispatch(&mut app, &at(PointerKind::Move), view);
        app.session.tool_poll(view, true).unwrap();
        crate::canvas::dispatch(&mut app, &at(PointerKind::Down), view);
        crate::canvas::dispatch(&mut app, &at(PointerKind::Up), view);
        assert!(app.session.tool_editing() && !app.session.doc().unwrap().is_dirty(), "{then}");
        let path = dir.join(if then == "file.exportAs" { "a.svg" } else { "a.vectorcraft" });
        let p = if then == "file.close" { json!({}) } else { json!({"path": path.to_string_lossy()}) };
        let r = app.run(then, p).unwrap();
        assert!(!app.session.tool_editing(), "{then}");
        let merged = app.session.doc().unwrap().history.undo.last().map(|e| e.label.clone());
        assert_eq!(merged.as_deref(), Some("Shape Builder"), "{then}");
        match then {
            "file.saveAs" => assert!(!app.session.doc().unwrap().is_dirty(), "the file holds the merge"),
            "file.exportAs" => assert!(app.session.doc().unwrap().is_dirty(), "an export leaves the document modified"),
            _ => assert_eq!(r["pending"], "saveChanges", "closing asks to save the merge"),
        }
    }
}

/// `ui.dialog.set` of a field the open dialog does not have is an error that lists its fields,
/// and the dialog stays as it was. Fields a dialog may not hold yet are still set: `discard`
/// with a discard button, `preview` with a live preview and the spec's `inputs`. A `__` field that
/// is not one of the controls of the dialog's kind is refused.
#[test]
fn dialog_set_refuses_a_field_the_dialog_does_not_have() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let fields = json!({"index": 0, "uid": 1, "name": "Untitled-1", "then": "close"});
    app.ui.dialog = Some(Dialog::new(crate::unsaved::KIND, fields.clone()));
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "discrad", "value": true}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"], "the `saveChanges` dialog has no field `discrad`; its fields: discard, index, name, then, uid");
    assert_eq!(app.ui.dialog.as_ref().map(|d| Value::Object(d.fields.clone())), Some(fields.clone()));
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "__note", "value": true}));
    assert_eq!(r["error"], "the `saveChanges` dialog cannot set protected field `__note`", "{r}");
    assert_eq!(app.ui.dialog.as_ref().map(|d| Value::Object(d.fields.clone())), Some(fields));
    for field in ["name", "discard"] {
        let r = input_request(&mut app, "ui.dialog.set", json!({"field": field, "value": true}));
        assert_eq!(r["ok"], true, "{field}: {r}");
    }
    // Move opens without Copy, which its checkbox adds once clicked; Scale's options and the
    // reference point belong to the other transform dialogs.
    let set = |app: &mut VectorcraftApp, kind: &str, fields: Value, cases: &[(&str, bool)]| {
        app.ui.dialog = Some(Dialog::new(kind, fields));
        for (field, ok) in cases {
            let r = input_request(app, "ui.dialog.set", json!({"field": field, "value": true}));
            assert_eq!(r["ok"], *ok, "{kind} {field}: {r}");
        }
    };
    let moved = [("copy", true), ("preview", true), ("distance", true), ("discard", false), ("copies", false), ("corners", false), ("origin", false)];
    set(&mut app, "move", json!({"dx": "0 pt", "dy": "0 pt"}), &moved);
    set(
        &mut app,
        "scale",
        json!({"sx": 100, "sy": 100, "uniform": true}),
        &[("corners", true), ("strokes", true), ("origin", true), ("distance", false)],
    );
    set(&mut app, "rotate", json!({"angle": 0}), &[("origin", true), ("objects", true), ("corners", false)]);
    set(&mut app, "rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}), &[("radius", true), ("sides", false)]);
    set(&mut app, "ellipse", json!({"x": 0, "y": 0, "width": 10, "height": 10}), &[("radius", false)]);
    // Corners that differ in kind and radius open without either.
    let mixed = json!({"id": 1, "corners": [0, 1, 2, 3], "relativeRadii": false});
    set(&mut app, vectorcraft_tools::corners::DIALOG, mixed, &[("kind", true), ("radius", true), ("radii", false)]);
    set(&mut app, "saveForOffice", json!({"ppi": 150, "transparent": false, "artboard": 0}), &[("path", true)]);
    set(&mut app, "brushOptions", json!({"name": "Dots"}), &[("tips", true), ("tip", false)]);
    set(&mut app, "justification", json!({"wordDesired": 100}), &[("wordMin", true), ("singleWord", true), ("wordMid", false)]);
    set(&mut app, "hyphenation", json!({"hyphenate": true}), &[("afterFirst", true), ("hyphenationZone", true), ("afterfirst", false)]);
    set(&mut app, "effectGallery", json!({"stack": []}), &[("browseEffect", true), ("browseParams", true), ("browse", false)]);
    // A new artboard's dialog opens without a background color or display marks.
    let new_board = json!({"new": true, "name": "Artboard 2", "x": 0, "y": 0, "width": 10, "height": 10});
    let marks = [
        ("color", true),
        ("showCenterMark", true),
        ("showCrossHairs", true),
        ("showVideoSafeAreas", true),
        ("videoRulerPixelAspectRatio", true),
        ("showCenterMarks", false),
    ];
    set(&mut app, "artboardOptions", new_board, &marks);
    // Glass and Texturizer take the texture Load Texture… reads; other effects don't.
    set(&mut app, "effect", json!({"__effect": "distort.glass"}), &[("textureImage", true)]);
    set(&mut app, "effect", json!({"__effect": "blur.gaussian"}), &[("textureImage", false)]);
    // The generic dialog takes every parameter of its command.
    set(&mut app, "command", json!({"__command": "shape.rectangle", "x": 0, "y": 0}), &[("width", true), ("widht", false)]);
    // Transparency Flattener Presets holds the selected preset's options before its first frame.
    crate::dialogs::flattener_presets::open(&mut app, None);
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "textToOutlines", "value": true}));
    assert_eq!(r["ok"], true, "{r}");
    // Edit Selection takes `deleted<i>` for each of its rows only.
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    for name in ["A", "B"] {
        app.run("select.save", json!({ "name": name })).unwrap();
    }
    crate::dialogs::edit_selection::open(&mut app).unwrap();
    for (field, ok) in [("deleted1", true), ("deleted", false), ("deletedx", false), ("deleted01", false), ("deleted2", false)] {
        let r = input_request(&mut app, "ui.dialog.set", json!({"field": field, "value": true}));
        assert_eq!(r["ok"], ok, "{field}: {r}");
    }
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "name", "value": "A"}));
    assert_eq!(
        r["error"],
        "the `editSelection` dialog has no field `name`; its fields: count, deleted0, deleted1, name0, name1, orig0, orig1, selected"
    );
    app.ui.dialog = None;
    assert_eq!(input_request(&mut app, "ui.dialog.set", json!({"field": "name", "value": "A"}))["error"], "no dialog open");
}
