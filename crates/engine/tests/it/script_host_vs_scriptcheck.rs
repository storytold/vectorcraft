//! Every global and member the script host (`script.run`) adds to the JavaScript engine is in one of
//! the tables below, and each name that reaches outside the document has the `outside` id
//! `script.check` (`vectorcraft-scriptcheck`) gives it. The names are read from a run: the own
//! properties of the global object, of each object or function it holds and of each function's
//! `prototype`, with a setter listed again as `name=`. The names a plain `vectorcraft_js::Realm` also
//! has are the language's, which reach files and other apps only through the host; `eval` and
//! `Function` still have rows, since they run code the checker can't read. A name the host adds
//! without a row fails the test.

use std::collections::BTreeSet;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_js::{DirectiveName, JsValue, Realm};
use vectorcraft_scriptcheck::{Limits, check};

/// The names that reach outside the document: each with a script that uses it and the `outside` id
/// the checker gives that script.
const OUTSIDE: &[(&str, &str, &str)] = &[
    ("eval", "eval(s);", "unseenCode"),
    ("Function", "var g = new Function(s);", "unseenCode"),
    ("$.evalFile", "$.evalFile(p);", "unseenCode"),
    ("$.getenv", "var h = $.getenv(n);", "environment"),
    ("$.setenv", "$.setenv(n, v);", "environment"),
    ("File.prototype.open", "var f = new File(p); f.open('w');", "filesWritten"),
    ("File.prototype.write", "var f = new File(p); f.write(s);", "filesWritten"),
    ("File.prototype.writeln", "var f = new File(p); f.writeln(s);", "filesWritten"),
    ("File.prototype.copy", "var f = new File(p); f.copy(q);", "filesWritten"),
    ("File.prototype.createAlias", "var f = new File(p); f.createAlias(q);", "filesWritten"),
    ("File.prototype.remove", "var f = new File(p); f.remove();", "filesWritten"),
    ("File.prototype.rename", "var f = new File(p); f.rename(q);", "filesWritten"),
    ("File.prototype.length=", "var f = new File(p); f.length = 0;", "filesWritten"),
    ("File.prototype.hidden=", "var f = new File(p); f.hidden = true;", "filesWritten"),
    ("File.prototype.readonly=", "var f = new File(p); f.readonly = true;", "filesWritten"),
    ("File.prototype.execute", "var f = new File(p); f.execute();", "filesOpened"),
    ("Folder.prototype.create", "var d = new Folder(p); d.create();", "filesWritten"),
    ("Folder.prototype.remove", "var d = new Folder(p); d.remove();", "filesWritten"),
    ("Folder.prototype.rename", "var d = new Folder(p); d.rename(q);", "filesWritten"),
    ("Folder.prototype.execute", "var d = new Folder(p); d.execute();", "filesOpened"),
];

/// The names the host adds that stay inside the document and the run, by the object that has them.
/// Reading a file or a folder, a file dialog, a message to the person and a ScriptUI dialog do not
/// change anything outside it.
const INSIDE: &[(&str, &str)] = &[
    ("", "$ File Folder UnitValue Window alert confirm localize prompt toSource"),
    ("Window.", "alert confirm length prompt prototype"),
    ("Window.prototype.", "add addEventListener center close constructor findElement hide remove removeEventListener show"),
    ("Object.prototype.", "toSource"),
    ("UnitValue.", "baseUnit baseUnit= length prototype"),
    ("UnitValue.prototype.", "as baseUnit baseUnit= constructor convert toString type type= value value="),
    ("localize.", "length"),
    ("toSource.", "length"),
    (
        "$.",
        "about appEncoding appEncoding= bp build buildDate colorPicker decimalPoint engineName error fileName flags \
         flags= gc global hiresTimer includePath level line locale locale= localize localize= memCache memCache= os screens \
         sleep stack strict version write writeln",
    ),
    ("File.", "decode encode fs isEncodingAvailable openDialog saveDialog length prototype"),
    (
        "File.prototype.",
        "absoluteURI alias changePath close constructor created creator displayName encoding encoding= eof error error= \
         exists fsName fullName getRelativeURI hidden length lineFeed lineFeed= localizedName modified name openDlg parent path read \
         readch readln readonly relativeURI resolve saveDlg seek tell toString type",
    ),
    (
        "Folder.",
        "appData appPackage commonFiles current current= decode desktop encode fs isEncodingAvailable length myDocuments \
         prototype selectDialog startup system temp trash userData",
    ),
    (
        "Folder.prototype.",
        "absoluteURI alias changePath constructor created displayName error error= exists fsName fullName getFiles \
         getRelativeURI localizedName modified name parent path relativeURI resolve selectDlg toString",
    ),
];

/// The name the checker does not read, which also stands for its members: `vc` is VectorCraft's
/// own API, and what `vc.run` does depends on the command it runs.
const NOT_CHECKED: &str = "vc";

/// An expression whose value is the list of names, one to a line.
const LIST: &str = r#"(function () {
    var out = [];
    function add(o, prefix) {
        var own = Object.getOwnPropertyNames(o);
        for (var i = 0; i < own.length; i++) {
            out.push(prefix + own[i]);
            var d = Object.getOwnPropertyDescriptor(o, own[i]);
            if (d && d.set) out.push(prefix + own[i] + "=");
        }
    }
    var g = (function () { return this; })();
    var globals = Object.getOwnPropertyNames(g);
    add(g, "");
    for (var i = 0; i < globals.length; i++) {
        var v = g[globals[i]];
        if (v !== null && (typeof v == "object" || typeof v == "function")) {
            add(v, globals[i] + ".");
            if (typeof v == "function" && v.prototype !== null && typeof v.prototype == "object") add(v.prototype, globals[i] + ".prototype.");
        }
    }
    return out.join("\n");
})()"#;

/// The names a script run by the host has.
fn host_names() -> BTreeSet<String> {
    let mut s = Session::new();
    let r = s.execute("script.run", &json!({"source": format!("vc.log({LIST});"), "name": "list.js"})).unwrap();
    let text = r["log"][0].as_str().unwrap_or_else(|| panic!("no list in {r}"));
    text.lines().map(str::to_string).collect()
}

/// The names a plain JavaScript engine has.
fn language_names() -> BTreeSet<String> {
    match Realm::new().run(LIST) {
        Ok(JsValue::String(text)) => text.lines().map(str::to_string).collect(),
        other => panic!("no list: {other:?}"),
    }
}

fn table_names() -> BTreeSet<String> {
    let inside = INSIDE.iter().flat_map(|(prefix, names)| names.split_whitespace().map(move |n| format!("{prefix}{n}")));
    OUTSIDE.iter().map(|(n, ..)| n.to_string()).chain(inside).chain([NOT_CHECKED.to_string()]).collect()
}

#[test]
fn every_name_the_host_adds_has_a_row() {
    let host = host_names();
    let language = language_names();
    let table = table_names();
    let not_checked = |n: &&String| n.starts_with(&format!("{NOT_CHECKED}."));
    let missing: Vec<_> = host.difference(&language).filter(|n| !table.contains(*n) && !not_checked(n)).collect();
    assert!(missing.is_empty(), "the host adds these names, which no table here lists: {missing:?}");
    let gone: Vec<_> = table.difference(&host).collect();
    assert!(gone.is_empty(), "the tables list these names, which a script run by the host does not have: {gone:?}");
}

#[test]
fn every_name_that_reaches_outside_has_its_checker_id() {
    for (name, script, id) in OUTSIDE {
        let r = check("probe.jsx", script.as_bytes(), None, &Limits::default());
        let ids: Vec<_> = r.outside.iter().map(|o| o.id).collect();
        assert!(ids.contains(id), "{name}: the checker gives {ids:?} for {script:?}, not {id}");
    }
}

/// A line with the directive `d`. The match has no catch-all, so a new directive needs a row, and
/// [`DirectiveName::from_name`] reads only the directives in [`DirectiveName::ALL`].
fn directive_line(d: DirectiveName) -> &'static str {
    match d {
        DirectiveName::Include => "#include \"lib.jsxinc\"",
        DirectiveName::IncludePath => "#includepath \"lib\"",
        DirectiveName::Script => "#script Tidy",
        DirectiveName::Strict => "#strict on",
        DirectiveName::Target => "#target vectorcraft",
        DirectiveName::TargetEngine => "#targetengine session",
    }
}

#[test]
fn the_checker_lists_every_directive_the_host_reads() {
    for d in DirectiveName::ALL {
        let line = directive_line(d);
        let r = check("probe.jsx", format!("{line}\nvar a = 1;\n").as_bytes(), None, &Limits::default());
        let listed: Vec<_> = r.directives.iter().map(|d| d.name.to_string()).collect();
        assert_eq!(listed, [d.as_str()], "{line}");
    }
}
