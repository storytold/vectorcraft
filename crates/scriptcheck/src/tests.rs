//! Tests on synthetic scripts, each written here from the language rules with invented names.

use proptest::prelude::*;

use crate::catalog::FeatureId;
use crate::lex::{Lexer, T};
use crate::*;

fn report(src: &str) -> Report {
    check("test.jsx", src.as_bytes(), None, &Limits::default())
}

/// The kinds and texts of the tokens of `src`, blanks and line breaks left out.
fn tokens(src: &str) -> Vec<(T, &str)> {
    Lexer::new(src, Limits::default().nesting).filter(|t| !matches!(t.kind, T::Space | T::Newline)).map(|t| (t.kind, t.text)).collect()
}

fn feature<'r>(r: &'r Report, id: &str) -> Option<&'r Feature> {
    r.features.iter().find(|f| f.id == id)
}

fn problems(r: &Report) -> Vec<(&'static str, Option<usize>, Option<&str>)> {
    r.problems.iter().map(|p| (p.code, p.line, p.detail.as_deref())).collect()
}

/// The script `src` with one feature, and one without it: `(id, with, without)`.
const FEATURE_CASES: &[(&str, &str, &str)] = &[
    ("actions", "app.doScript('Set', 'Group');", "doScript('Set', 'Group');"),
    ("actions", "var text = '/version 3 /actionCount 1';", "var text = '/version 3';"),
    ("bridgeTalk", "var bt = new BridgeTalk();", "var bt = new Bridge();"),
    ("directive.include", "#include 'a.jsxinc'", "var include = 'a.jsxinc';"),
    ("directive.include", "#includepath 'lib'", "// #includepath 'lib'"),
    ("directive.other", "#strict on", "var strict = 'on';"),
    ("directive.target", "#target acme", "// #target acme"),
    ("directive.target", "  //@target acme", "x(); //@target acme"),
    ("directive.targetengine", "#targetengine session", "var targetengine = 'session';"),
    ("dollar", "$.writeln('hi');", "log.writeln('hi');"),
    ("dollar.env", "var home = $.getenv('HOME');", "var home = env.getenv('HOME');"),
    ("dollar.evalFile", "$.evalFile(f);", "loader.evalFile(f);"),
    ("dynamicCode", "eval(code);", "safe.eval(code);"),
    ("dynamicCode", "var f = new Function('a', 'return a');", "var f = function (a) { return a; };"),
    ("e4x", "var x = <root><item id=\"1\"/>text {v}</root>;", "var x = a < b;"),
    ("e4x", "var id = node.@id;", "var id = node.id;"),
    ("e4x", "var items = node..item;", "var items = node.item;"),
    ("e4x", "for each (var v in list) {}", "for (var k in list) {}"),
    ("e4x", "default xml namespace = ns;", "var namespace = ns;"),
    ("externalObject", "var lib = new ExternalObject('lib:' + name);", "var lib = new External(name);"),
    ("file.dialog", "var f = File.openDialog('Pick');", "var f = picker.openDialog('Pick');"),
    ("file.dialog", "var d = Folder.selectDialog();", "var d = Folder.select();"),
    ("file.dialog", "var f = target.saveDlg('Save');", "var f = target.save('Save');"),
    ("file.execute", "out.execute();", "out.executeAll();"),
    ("file.objects", "var f = new File(path);", "var f = new Filer(path);"),
    ("file.places", "var t = Folder.temp;", "var t = cache.temp;"),
    ("file.read", "var f = new File(p); f.open('r');", "var f = new File(p);"),
    ("file.read", "var f = new File(p); var s = f.read();", "var s = cache.read();"),
    ("file.read", "var all = folder.getFiles();", "var all = folder.files;"),
    ("file.read", "var s = File(p).read();", "var s = Cache(p).read();"),
    ("file.write", "var f = new File(p); f.open('w');", "var f = new File(p); f.open('r');"),
    ("file.write", "var f = File.saveDialog(); f.write(s);", "var f = picker(); f.write(s);"),
    ("file.write", "new File(p).remove();", "new Thing(p).remove();"),
    ("file.write", "var a = new File(p); var b = a; b.remove();", "var a = new File(p); var b = a.parent; b.remove();"),
    ("file.write", "var f = new File(p); f.remove();", "var f = new File(p); var o = {f: {remove: function () {}}}; o.f.remove();"),
    ("file.write", "var f = new File(p); f.open('\\167');", "var f = new File(p); f.open('\\162');"),
    ("file.write", "new File(p).open('a');", "var cache = {open: function (m) {}}; cache.open('w');"),
    ("file.write", "new Folder(p).create();", "new Folder(p).name;"),
    ("file.write", "File.saveDialog('Save').open('w');", "picker.saveDialog('Save').open('w');"),
    ("file.write", "Folder.selectDialog().create();", "Folder.select().create();"),
    ("file.write", "Folder.temp.remove();", "cache.temp.remove();"),
    ("hostCheck", "if (parseInt(app.version) < 20) {}", "if (parseInt(appVersion) < 20) {}"),
    ("localize", "var s = localize(text);", "var s = text.localize;"),
    ("localize", "var s = $.localize(text);", "var s = obj.localize(text);"),
    ("menuCommand", "app.executeMenuCommand('acmecopyall');", "executeMenuCommand('acmecopyall');"),
    ("notify", "alert('done');", "panel.alert('done');"),
    ("notify", "if (confirm('Go?')) {}", "function confirm(m) {}"),
    ("operatorOverloading", "Vec.prototype['+'] = function (o) { return o; };", "Vec.prototype['plus'] = function (o) { return o; };"),
    ("preferences", "app.preferences.setAcmeFlag('k', 1);", "prefs.setAcmeFlag('k', 1);"),
    ("reflection", "var p = obj.reflect.properties;", "var p = obj.reflected;"),
    ("scriptui.dialog", "var w = new Window('dialog', 'Title');", "var w = new Window('palette');"),
    ("scriptui.dialog", "var w = new Window(\"dialog { text: 'T' }\");", "var w = new Window('window');"),
    ("scriptui.drawing", "g.onDraw = paint;", "g.onDrawn = paint;"),
    ("scriptui.drawing", "var pen = g.graphics.newPen(t, c, 1);", "var pen = g.graphics.pen;"),
    ("scriptui.palette", "var w = new Window('palette');", "var w = new Window('dialog');"),
    ("scriptui.window", "var w = new Window('window');", "var w = new Window('dialog');"),
    ("scriptui.window", "var w = new Window(kind);", "var w = new Panel(kind);"),
    ("selectTool", "app.selectTool('acme');", "selectTool('acme');"),
    ("socket", "var s = new Socket();", "var s = new Sock();"),
    ("unitValue", "var u = new UnitValue(1, 'in');", "var u = units.UnitValue;"),
    ("xml", "var x = new XML(s);", "var x = new Xml(s);"),
    ("xmp", "var m = new XMPMeta();", "var m = new Meta();"),
];

#[test]
fn each_feature_is_found_where_it_is_and_nowhere_else() {
    for (id, with, without) in FEATURE_CASES {
        let r = report(with);
        assert_eq!(r.kind, Kind::Script, "{with}");
        let f = feature(&r, id).unwrap_or_else(|| panic!("{id} not found in {with:?}: {:?}", r.features));
        assert_eq!(f.lines, [1], "{with}");
        let need = FeatureId::ALL.iter().find(|f| f.id() == *id).map(|f| f.need()).unwrap_or_else(|| panic!("{id} sets no need"));
        assert!(r.needs.contains(&need), "{with}: {:?}", r.needs);
        let r = report(without);
        assert!(feature(&r, id).is_none(), "{id} found in {without:?}");
    }
}

#[test]
fn features_are_listed_in_alphabetical_order_of_their_ids() {
    let ids: Vec<&str> = FeatureId::ALL.iter().map(|f| f.id()).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
    assert!(FeatureId::ALL.windows(2).all(|w| w[0] < w[1]), "the enum is declared in the order of its ids");
}

#[test]
fn every_feature_id_has_a_case() {
    for f in FeatureId::ALL {
        let id = f.id();
        assert!(FEATURE_CASES.iter().any(|(case, ..)| *case == id), "{id} has no case");
    }
    for (id, ..) in FEATURE_CASES {
        assert!(FeatureId::ALL.iter().any(|f| f.id() == *id), "{id} isn't a FeatureId");
    }
    let unique = |ids: &[&str]| ids.iter().collect::<std::collections::BTreeSet<_>>().len() == ids.len();
    assert!(unique(PROBLEMS) && unique(OUTSIDE));
}

#[test]
fn compiled_scripts_are_reported_and_never_read() {
    let limits = Limits::default();
    for (name, bytes) in [
        ("x.jsxbin", b"#target acme\nalert(1);".as_slice()),
        ("x.JSXBIN", b"".as_slice()),
        ("x.jsx", b"@JSXBIN@ES@2.0@AcmeBody".as_slice()),
        ("x.jsx", b"\xEF\xBB\xBF \r\n\t@JSXBIN@ES@2.0@".as_slice()),
    ] {
        let r = check(name, bytes, None, &limits);
        assert_eq!(r.kind, Kind::Compiled, "{name} {bytes:?}");
        assert_eq!(r.steps, 0, "a compiled script isn't read");
        assert!(r.directives.is_empty() && r.needs.is_empty());
    }
    // The prefix elsewhere is text in a script.
    assert_eq!(report("var s = '@JSXBIN@ES@2.0@';").kind, Kind::Script);
}

#[test]
fn modules_are_not_scripts() {
    for src in ["import Acme from 'acme';\nexport default function App() { return <div/>; }\n", "var a = 1;\nexport { a };\n", "export const b = 2;"]
    {
        let r = report(src);
        assert_eq!(r.kind, Kind::Module, "{src}");
        assert!(r.features.is_empty() && r.problems.is_empty() && r.needs.is_empty(), "{r:?}");
    }
    let r = report("var o = { import: 1 };\no.export(2);\nimport('x');\nexport.y = 3;\n");
    assert_eq!(r.kind, Kind::Script);
}

#[test]
fn binary_and_oversized_files_are_not_checked() {
    let limits = Limits::default();
    assert_eq!(check("x.jsx", b"var a = 1;\0", None, &limits).kind, Kind::NotText);
    assert_eq!(check("x.jsx", &[1, 2, 3, 4, 5, b'a', b'b'], None, &limits).kind, Kind::NotText);
    assert_eq!(check("x.jsx", b"var a = '\x07';\nvar b = 2;\n", None, &limits).kind, Kind::Script, "a stray control character");
    let small = Limits { file_bytes: 16, ..Limits::default() };
    assert_eq!(check("x.jsx", &[b'a'; 17], None, &small).kind, Kind::TooLarge);
    assert_eq!(check("x.jsx", &[b'a'; 16], None, &small).kind, Kind::Script);
}

#[test]
fn directives_are_read_at_the_start_of_a_line_in_both_forms() {
    let r = report(concat!(
        "#target \"acme_2-26.1-en_US\"\n",
        "  //@targetengine session\n",
        "#include 'lib/a.jsxinc'; // helpers\n",
        "#includepath \"lib;shared\"\n",
        "#script \"Tidy\"\n",
        "#strict on\n",
        "var x = 1; //@target other\n",
        "#pragma once\n",
        "#!\n",
        "#TARGET second\n",
    ));
    let directives: Vec<_> = r.directives.iter().map(|d| (d.name, d.value.as_str(), d.line, d.form)).collect();
    assert_eq!(
        directives,
        [
            ("target", "acme_2-26.1-en_US", 1, "#"),
            ("targetengine", "session", 2, "//@"),
            ("include", "lib/a.jsxinc", 3, "#"),
            ("includepath", "lib;shared", 4, "#"),
            ("script", "Tidy", 5, "#"),
            ("strict", "on", 6, "#"),
            ("target", "second", 10, "#"),
        ]
    );
    let t = &r.targets[0];
    assert_eq!(
        (t.value.as_str(), t.app.as_str(), t.instance.as_deref(), t.version.as_deref(), t.locale.as_deref(), t.line),
        ("acme_2-26.1-en_US", "acme", Some("2"), Some("26.1"), Some("en_US"), 1)
    );
    assert_eq!((r.targets[1].app.as_str(), r.targets[1].instance.as_deref(), r.targets[1].version.as_deref()), ("second", None, None));
    let includes: Vec<_> = r.includes.iter().map(|i| (i.value.as_str(), i.line, i.status)).collect();
    assert_eq!(includes, [("lib/a.jsxinc", 3, IncludeStatus::NotRead)]);
    assert_eq!(problems(&r), [("unknownDirective", Some(8), Some("#pragma")), ("unknownDirective", Some(9), Some("#!"))]);
    assert_eq!(r.needs, [Need::Language, Need::HostOnly]);
}

#[test]
fn a_byte_order_mark_and_every_line_break_are_read() {
    // U+2028 ends line 2.
    let r = check("t.jsx", b"\xEF\xBB\xBF#target acme\nvar a = 1;\xE2\x80\xA8#target b\r\n", None, &Limits::default());
    let lines: Vec<usize> = r.directives.iter().map(|d| d.line).collect();
    assert_eq!(lines, [1, 3]);
    assert_eq!(r.lines, 3);
    assert_eq!(problems(&r), [("mixedLineEndings", None, None)]);
}

#[test]
fn invalid_utf8_is_noted_and_the_rest_checked() {
    let r = check("t.jsx", b"var b = '\xff';\nalert(b);\n", None, &Limits::default());
    assert_eq!(problems(&r), [("notUtf8", None, None)]);
    assert!(feature(&r, "notify").is_some());
    assert_eq!(r.lines, 2);
}

#[test]
fn member_chains_are_listed_as_written_with_variables_replaced() {
    let r = report(concat!(
        "var doc = app.activeDocument;\n",
        "var shape = doc.acmeItems.add();\n",
        "shape.filled = true;\n",
        "for (var i = 0; i < doc.acmeLayers.length; i++) {\n",
        "  doc.acmeLayers[i].shown = false;\n",
        "}\n",
        "var n = selection.length;\n",
        "var first = documents[0];\n",
    ));
    let members: Vec<(&str, usize)> = r.members.iter().map(|m| (m.name.as_str(), m.count)).collect();
    assert_eq!(
        members,
        [
            ("app.activeDocument", 1),
            ("app.activeDocument.acmeItems.add()", 1),
            ("app.activeDocument.acmeItems.add().filled", 1),
            ("app.activeDocument.acmeLayers.length", 1),
            ("app.activeDocument.acmeLayers[].shown", 1),
            ("documents[]", 1),
            ("selection.length", 1),
        ]
    );
    assert_eq!(r.needs, [Need::Dom]);
}

#[test]
fn names_written_with_escapes_are_read_as_the_names_they_spell() {
    let r = report("\\u0061pp.activeDocument.acmeName;\n\\u0065val(code);\nvar s = '\\u0061pp.x', \\u002e;\n");
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument.acmeName"]);
    assert!(feature(&r, "dynamicCode").is_some());
    assert_eq!(r.outside.iter().map(|o| o.id).collect::<Vec<_>>(), ["unseenCode"]);
    assert_eq!(r.lines, 3);
}

#[test]
fn the_scripts_own_platform_names_are_not_the_platforms() {
    let r = report(concat!(
        "var Socket = 0;\n",
        "function alert(m) { return m; }\n",
        "alert('x');\n",
        "function f(File) { return new File(p); }\n",
        "var g = new File(q); g.remove();\n",
    ));
    assert!(feature(&r, "socket").is_none());
    assert!(feature(&r, "notify").is_none());
    assert_eq!(feature(&r, "file.objects").map(|f| f.lines.as_slice()), Some(&[5][..]));
    assert_eq!(r.outside.iter().map(|o| o.id).collect::<Vec<_>>(), ["filesWritten"]);
}

#[test]
fn a_function_after_a_line_that_ends_a_statement_is_a_declaration() {
    let r = report("var n = 1\nfunction File() {}\nFile('x');\nvar m = 2 /* a\n */ function Socket() {}\nnew Socket();\n");
    assert!(feature(&r, "file.objects").is_none(), "{:?}", r.features);
    assert!(feature(&r, "socket").is_none(), "{:?}", r.features);
}

#[test]
fn object_keys_are_not_platform_names() {
    let r = report("var options = {Socket: false, ExternalObject: false, Window: 1};\nf(a, Socket);\n");
    assert_eq!(feature(&r, "socket").map(|f| f.lines.as_slice()), Some(&[2][..]));
    assert!(feature(&r, "externalObject").is_none());
    assert!(feature(&r, "scriptui.window").is_none());
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("network", &[2][..])]);
}

#[test]
fn the_scripts_own_variables_hide_the_roots() {
    let r = report(concat!(
        "var selection = [1, 2];\n",
        "selection.push(3);\n",
        "function documents() { return []; }\n",
        "documents().length;\n",
        "var o = { activeDocument: 1 };\n",
        "foo.app.x = 1;\n",
        "var doc = app.activeDocument;\n",
        "doc = 5;\n",
        "doc.acmeItems;\n",
    ));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument"], "only the line that reads the document");
}

#[test]
fn only_the_commas_of_a_var_statement_declare_names() {
    let r = report(concat!(
        "var x = f(0, app.activeDocument), y = [1, selection];\n",
        "app.activeDocument.acmeLayers;\n",
        "selection.length;\n",
        "var a = 1, b, documents = [];\n",
        "documents.length;\n",
    ));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers", "selection", "selection.length"]);
}

/// In `for (var name in …)` the names after `in` are the loop's object, also after a comma (ECMA-262
/// 3rd edition, section 12.6.4): the statement declares none of them, in a function's body or outside.
#[test]
fn the_names_after_the_in_of_a_for_var_are_not_declared() {
    let r = report(concat!(
        "function run() {\n",
        "  new File(p).remove();\n",
        "  for (var key in obj, File) {}\n",
        "  for each (var item in list, alert) {}\n",
        "  alert('a');\n",
        "}\n",
        "for (var k in obj, Folder) {}\n",
    ));
    assert_eq!(feature(&r, "file.objects").map(|f| f.lines.as_slice()), Some(&[2, 3, 7][..]));
    assert_eq!(feature(&r, "notify").map(|f| f.lines.as_slice()), Some(&[5][..]));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[2][..])]);
}

#[test]
fn a_var_without_a_value_keeps_what_the_name_holds() {
    let r = report(concat!(
        "var f = new File(p); var f; f.remove();\n",
        "var doc = app.activeDocument;\n",
        "var doc;\n",
        "doc.acmeName;\n",
        "function g() { var f; f.remove(); var app; app.acmeLocal; }\n",
    ));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[1][..])]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeName"]);
}

#[test]
fn a_block_comment_with_a_line_break_ends_a_line() {
    let r = report("var d = app.activeDocument/*\n*/d.acmeLayers;\n");
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers"]);
}

#[test]
fn a_var_statement_goes_on_after_a_function_with_a_var_statement_of_its_own() {
    let r = report("var helper = function () { var x = 0; }, f = new File(p);\nf.remove();\n");
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[2][..])]);
}

#[test]
fn a_var_statement_goes_on_across_the_line_breaks_inside_it() {
    let r = report(concat!(
        "var first =\n  0, doc = app.activeDocument;\n",
        "doc.acmeLayers;\n",
        "var opts = {\n  a: 1\n}, sel = selection;\n",
        "sel.length;\n",
        "var count = 1\n  , layer = app.activeDocument;\n",
        "layer.acmeName;\n",
        "var n = 1\nvar documents = 2;\n",
        "documents.length;\n",
    ));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers", "app.activeDocument.acmeName", "selection", "selection.length"]);
}

#[test]
fn a_catch_blocks_parameter_is_its_own() {
    let r = report(concat!(
        "var f = new File('out');\n",
        "try {} catch (f) { f = null; }\n",
        "f.remove();\n",
        "var doc = app.activeDocument;\n",
        "try { x(); } catch (doc) { doc.acmeName; }\n",
        "doc.acmeLayers;\n",
        "try { x(); } catch (e) { g = new File(p); }\n",
        "g.remove();\n",
        "try { x(); } catch (e) { var d2 = app.activeDocument; }\n",
        "d2.acmeMode;\n",
    ));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[3, 8][..])]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers", "app.activeDocument.acmeMode"]);
    // In a function, the parameter is not the function's own name, assigned in the block or not.
    let r = report(concat!(
        "var f = new File(p);\n",
        "function g() { try {} catch (f) {} var f; f.remove(); }\n",
        "function h() { try {} catch (f) { f = null; } }\n",
        "f.remove();\n",
        "function k() { try { throw 0; } catch (e) { var e = new File(p); var e; e.remove(); } }\n",
        "function m() { try {} catch (f) { var f; } f.remove(); }\n",
        "f.remove();\n",
        "function n() { try {} catch (f) { try {} catch (f) { var f; } f.remove(); } f.remove(); }\n",
        "f.remove();\n",
    ));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[4, 5, 7, 9][..])]);
}

#[test]
fn assignments_inside_expressions_are_followed() {
    let r = report(concat!(
        "var f;\n",
        "(f = new File('out.txt'));\n",
        "f.open('w');\n",
        "if ((g = File.openDialog('Pick')) != null) { g.remove(); }\n",
        "switch (k) { case 1: doc = app.activeDocument; break; }\n",
        "doc.acmeLayers;\n",
        "if (!ok) x = 1; else h = new File(p);\n",
        "h.remove();\n",
        "o.e = new File(p);\n",
        "e.remove();\n",
        "var c; ok ? c = new File(p) : 0;\n",
        "c.remove();\n",
        "ok ? d2 = app.activeDocument : 0;\n",
        "d2.acmeName;\n",
    ));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[3, 4, 8, 12][..])]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers", "app.activeDocument.acmeName"]);
}

/// `name += …` and the other compound assignments, `name++`, `name--`, `++name` and `--name` give the
/// name a number or a string (ECMA-262 3rd edition, sections 11.3, 11.4 and 11.13.2), so it no longer
/// holds a member chain or a File object. A compound assignment's name holds its old value while the
/// value on the right is read, until that value ends: the `:` of a conditional inside the value
/// doesn't end it, and the `:` of one around it does. A `++` after a line break is the next name's,
/// and a property changed so leaves the variable as it is.
#[test]
fn compound_assignments_and_increments_empty_a_name() {
    let r = report(concat!(
        "var d = app.activeDocument; d += ''; d.acmeA;\n",
        "var f = new File(p); f += '.txt'; f.remove();\n",
        "var e = app.activeDocument; e >>>= 1; e.acmeB;\n",
        "var g = app.activeDocument; g++; g.acmeC;\n",
        "var h = app.activeDocument; --h; h.acmeD;\n",
        "var k = app.activeDocument; x = y + ++k; k.acmeE;\n",
        "var m = app.activeDocument; m\n",
        "++n; m.acmeF;\n",
        "var o = app.activeDocument; o.count += 1; ++o.total; o.acmeG;\n",
        "var q = new File(p); w.q += 1; q.remove();\n",
        "var r = new File(p); r += r.remove();\n",
        "r.remove();\n",
        "var s = app.activeDocument; s += s.acmeH; s.acmeI;\n",
        "var u = app.activeDocument; u += 'x'\n",
        "u.acmeJ;\n",
        "var v = new File(p); v += ok ? '' : v.remove();\n",
        "v.remove();\n",
        "var z; ok ? z += 1 : z = new File(p); z.remove();\n",
        "var y = new File(p); y += a ? b ? 1 : 2 : y.remove();\n",
    ));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[10, 11, 16, 18, 19][..])]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        members,
        [
            "app.activeDocument",
            "app.activeDocument.acmeF",
            "app.activeDocument.acmeG",
            "app.activeDocument.acmeH",
            "app.activeDocument.count",
            "app.activeDocument.total"
        ]
    );
}

/// The lines of the `filesWritten` use in `r`, empty when it has none, and the member names, for a
/// script whose other uses are not under test.
fn written_lines_and_members(r: &Report) -> (Vec<usize>, Vec<&str>) {
    let lines = r.outside.iter().flat_map(|o| {
        assert_eq!(o.id, "filesWritten");
        o.lines.iter().copied()
    });
    (lines.collect(), r.members.iter().map(|m| m.name.as_str()).collect())
}

/// `(name, script, lines of the files written, members)`: what a name holds after a value that goes
/// on past its parentheses or past a call, or that is a comma expression or a grouped value.
const HELD_VALUE_CASES: &[(&str, &str, &[usize], &[&str])] = &[
    (
        "a_value_that_goes_on_on_the_next_line_is_not_what_the_name_holds",
        concat!(
            "var a = new File(p);\n",
            "var b = a\n  .name;\n",
            "b.remove();\n",
            "var d = app.activeDocument\n  + 'x';\n",
            "d.acmeName;\n",
            "var c = a\nc.remove();\n",
            "var e = app.activeDocument\ne.acmeLayers;\n",
        ),
        &[9],
        &["app.activeDocument", "app.activeDocument.acmeLayers"],
    ),
    // A comma expression in parentheses is the value of its last operand (ECMA-262 3rd edition,
    // section 11.14).
    (
        "a_comma_expression_in_parentheses_is_its_last_operand",
        concat!(
            "var d = (0, app.activeDocument);\n",
            "d.acmeName;\n",
            "var f = (x, (new File(p)));\n",
            "f.remove();\n",
            "var g = (new File(p), 0);\n",
            "g.remove();\n",
            "var e = ((app.activeDocument), 0);\n",
            "e.acmeLayers;\n",
            "var k = (a = 1, f);\n",
            "k.remove();\n",
        ),
        &[4, 10],
        &["app.activeDocument", "app.activeDocument.acmeName"],
    ),
    // Parentheses that hold only the start of the value, or a value that goes on after them, leave the
    // name holding nothing the check follows, as a call's argument list does.
    (
        "a_value_that_only_begins_in_parentheses_is_not_what_the_name_holds",
        concat!(
            "var f = new File(p);\n",
            "var x = (app.activeDocument).name;\n",
            "x.acmeLayers;\n",
            "var y = (f).name;\n",
            "y.remove();\n",
            "var doc = ((app.activeDocument) + 1);\n",
            "doc.acmeName;\n",
            "var g = (new File(p).name);\n",
            "g.remove();\n",
            "var h = (new File(p))\n  .name;\n",
            "h.remove();\n",
            "var k = (File.name);\nnew File(p);\n",
            "k.remove();\n",
            "var flag = (File === File(p));\n",
            "flag.remove();\n",
            "var c = (f)(1), z = foo(new File(p));\n",
            "c.remove(); z.remove();\n",
            "var u = (x;\ny, new File(p));\n",
            "u.remove();\n",
        ),
        &[],
        &["app.activeDocument"],
    ),
    // The name holds a grouped value from the token after its last `)`: the next statement reads it, a
    // function's own variable holds it only inside the function, and grouped values inside a function in
    // parentheses don't change what the outer name holds.
    (
        "a_grouped_value_is_held_from_the_token_after_its_parentheses",
        concat!(
            "var doc = (app.activeDocument)\n",
            "doc.acmeName;\n",
            "function test() { var d2 = (app.activeDocument) }\n",
            "d2.acmeLayers;\n",
            "var f = (new File(File(p).fsName));\n",
            "f.remove();\n",
            "var lib = (function () { var sel = (selection); return sel.length; })();\n",
            "var w = new File(p);\n",
            "w = (function () { var inner = (q); return inner; });\n",
            "w.remove();\n",
        ),
        &[6],
        &["app.activeDocument", "app.activeDocument.acmeName", "selection", "selection.length"],
    ),
    // A value without parentheses that starts with `File` or `Folder` is what the name holds only when
    // nothing follows the call, the dialog or the place, as in parentheses.
    (
        "a_file_value_that_goes_on_is_not_what_the_name_holds",
        concat!(
            "var a = File(p) && {remove: function () {}};\n",
            "a.remove();\n",
            "var b = new File(p).name;\n",
            "b.remove();\n",
            "var c = new File(p)\n  .parent;\n",
            "c.remove();\n",
            "var d = Folder.temp + '/x', e = File.openDialog('Pick') || other;\n",
            "d.remove(); e.remove();\n",
            "var g = File(p) == null, h = File, k = File.saveDialog;\n",
            "g.remove(); h.remove(); k.remove();\n",
        ),
        &[],
        &[],
    ),
];

const HELD_VALUE_CASE_COUNT: usize = 5;

#[test]
fn a_name_holds_what_the_value_it_was_given_ends_as() {
    assert_eq!(HELD_VALUE_CASES.len(), HELD_VALUE_CASE_COUNT);
    for &(name, src, lines, members) in HELD_VALUE_CASES {
        let r = report(src);
        let (got_lines, got_members) = written_lines_and_members(&r);
        assert_eq!((got_lines.as_slice(), got_members.as_slice()), (lines, members), "{name}");
    }
}

/// A value in grouping parentheses, nested to any depth, is what the name holds (ECMA-262 3rd
/// edition, section 11.1.6).
#[test]
fn a_value_in_grouping_parentheses_is_what_the_name_holds() {
    let deep = format!("var deep = {}new File(p){};\ndeep.remove();\n", "(".repeat(1000), ")".repeat(1000));
    let src = concat!(
        "var f = (new File(p));\n",
        "f.open('w');\n",
        "g = ((new File(p)));\n",
        "g.remove();\n",
        "var d = (app.activeDocument);\n",
        "d.acmeLayers;\n",
        "var b = (f), n = (new Folder);\n",
        "b.remove();\n",
        "n.create();\n",
        "var s = (File.saveDialog()), t = (Folder.temp);\n",
        "s.write(x);\n",
        "t.remove();\n",
        "if ((h = (new File(p))) != null) { h.rename(q); }\n",
        "var sel = ((selection));\n",
        "sel.length;\n",
    )
    .to_string();
    let r = report(&(src + &deep));
    let (lines, members) = written_lines_and_members(&r);
    assert_eq!(lines, [2, 4, 8, 9, 11, 12, 13, 17]);
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers", "selection", "selection.length"]);
}

/// Grouping parentheses, nested or after a comma, leave a name holding what the value without them
/// gives it.
#[test]
fn a_value_holds_the_same_with_or_without_grouping_parentheses() {
    let start = "var f = new File(p), doc = app.activeDocument;\n";
    let uses = "x.remove();\nx.acmeName;\n";
    for value in [
        "new File(p)",
        "File(p)",
        "new Folder(p)",
        "new Folder",
        "Folder.temp",
        "File.openDialog('Pick')",
        "f",
        "app.activeDocument",
        "app.documents[0]",
        "doc.acmeLayers.add()",
        "selection",
        "1",
        "pick(f)",
    ] {
        let bare = report(&format!("{start}var x = {value};\n{uses}"));
        for grouped in [format!("({value})"), format!("(({value}))"), format!("(0, {value})")] {
            let r = report(&format!("{start}var x = {grouped};\n{uses}"));
            assert_eq!((&r.outside, &r.members), (&bare.outside, &bare.members), "{grouped}");
        }
    }
}

/// A call on a File or Folder object in grouping parentheses is a call on the object (ECMA-262 3rd
/// edition, section 11.1.6). A call on parentheses that hold more than the object, on the result of a
/// call, or on an E4X filter is not a call on the object.
#[test]
fn a_call_on_a_file_in_grouping_parentheses_is_followed() {
    let r = report(concat!(
        "var f = new File(p);\n",
        "(f).remove();\n",
        "(new File(p)).remove();\n",
        "((f)).open('w');\n",
        "(0, f).rename(q);\n",
        "(File.saveDialog('Save')).write(s);\n",
        "(Folder.temp).remove();\n",
        "(new File).create();\n",
        "if (ok) (f).remove();\n",
        "{} (new Folder(p)).create();\n",
        "var s = (f).read();\n",
        "(f.parent).remove();\n",
        "(x + f).remove();\n",
        "(f).name.remove();\n",
        "foo(f).remove();\n",
        "var g = function () {}\n(f).remove();\n",
        "a\n(f).remove();\n",
        "o.return(f).remove();\n",
        "list.(f).remove();\n",
    ));
    let outside: Vec<(&str, &[usize], &[&str])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice(), o.modes.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[2, 3, 4, 5, 6, 7, 8, 9, 10][..], &["w"][..])]);
    assert_eq!(feature(&r, "file.read").map(|f| f.lines.as_slice()), Some(&[11][..]));
}

/// The name holds a value without parentheses from the token that ends it: the next statement reads
/// it, the value itself reads what the name held before, a function's own variable holds it only
/// inside the function, a preprocessor line or the end of a file ends it, and a line break after a
/// `.` does not.
#[test]
fn a_file_value_is_held_from_the_token_that_ends_it() {
    let mut m = mem(&[("lib.jsxinc", "var out = new File(p)")]);
    let src = concat!(
        "var a = new File(p)\n",
        "a.remove();\n",
        "var doc = app.activeDocument;\n",
        "doc = new File(doc.fullName);\n",
        "doc.remove();\n",
        "function test() { var w = new File(p) }\n",
        "function test2() { var v = a }\n",
        "w.remove(); v.remove();\n",
        "var t = Folder.temp\n",
        "#target acme\n",
        "t.remove();\n",
        "#include 'lib.jsxinc'\n",
        "out.remove();\n",
        "var s = File.\n  saveDialog('Save');\n",
        "s.remove();\n",
    );
    let r = check("main.jsx", src.as_bytes(), Some(&mut m), &Limits::default());
    let (lines, members) = written_lines_and_members(&r);
    assert_eq!(lines, [2, 5, 11, 13, 16]);
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.fullName"]);
}

#[test]
fn a_functions_own_variables_hide_the_roots_only_inside_it() {
    let r = report(concat!(
        "function helper() { var app = {}; app.acmeLocal = 1; }\n",
        "app.activeDocument.acmeLayers;\n",
        "function pick() { var selection = []; return selection.length; }\n",
        "selection.length;\n",
        "var doc = app.activeDocument;\n",
        "var other = function () { var doc = 5; doc.acmeName; };\n",
        "doc.acmeName;\n",
        "function outer() { function documents() {} documents().length; }\n",
        "documents.length;\n",
    ));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        members,
        ["app.activeDocument", "app.activeDocument.acmeLayers", "app.activeDocument.acmeName", "documents.length", "selection.length"]
    );
}

#[test]
fn a_name_a_function_body_empties_is_empty_only_inside_it() {
    let r = report(concat!(
        "var f = new File(p);\n",
        "function unused() { f = null; app = 1; }\n",
        "f.remove();\n",
        "var doc;\n",
        "function main() { doc = app.activeDocument; }\n",
        "doc.acmeLayers;\n",
    ));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[3][..])]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers"]);
}

#[test]
fn a_function_expressions_name_hides_a_root_only_inside_it() {
    let r = report(concat!(
        "var f = function app() { return app.length; };\n",
        "app.activeDocument.acmeLayers;\n",
        "function selection() {}\n",
        "selection.length;\n",
    ));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument.acmeLayers"]);
}

#[test]
fn a_functions_parameters_hide_the_roots_and_variables_only_inside_it() {
    assert!(report("function count(selection) { return selection.length; }\n").needs.is_empty());
    let r = report(concat!(
        "function count(selection) { return selection.length; }\n",
        "var doc = app.activeDocument;\n",
        "function f(doc, app) { doc.acmeLocal; app.acmeLocal; }\n",
        "var g = function (documents) { return documents.length; };\n",
        "doc.acmeName;\n",
        "selection.length;\n",
    ));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeName", "selection.length"]);
}

/// A function declaration later in a function's body makes the name the function's own in all of the
/// body (ECMA-262 3rd edition, section 10.1.3), and nowhere else. A function expression's name is the
/// expression's own (section 13), and a function declared inside a block is the function's own from
/// where it stands.
#[test]
fn a_function_declared_later_in_a_body_is_its_own_in_all_of_it() {
    let r = report(concat!(
        "function run() {\n",
        "  alert('a');\n",
        "  var f = new File(p);\n",
        "  f.remove();\n",
        "  app.activeDocument.acmeName;\n",
        "  confirm('b');\n",
        "  if (ok) { prompt('c'); function prompt() {} }\n",
        "  function alert(m) {}\n",
        "  function File(p) {}\n",
        "  function app() {}\n",
        "  var g = function confirm() {};\n",
        "}\n",
        "alert('d');\n",
        "var h = new File(q);\n",
        "h.remove();\n",
    ));
    assert_eq!(feature(&r, "notify").map(|f| f.lines.as_slice()), Some(&[6, 7, 13][..]));
    assert_eq!(feature(&r, "file.objects").map(|f| f.lines.as_slice()), Some(&[14][..]));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[15][..])]);
    assert!(r.members.is_empty(), "{:?}", r.members);
}

/// A `var` statement later in a function's body, in a block or a `for` included, makes the name the
/// function's own in all of the body: neither the outer variable nor the platform's object or root of
/// that name is what the name stands for there. A `var` statement goes on after a function in it, and
/// a line break ends it as in the analysis.
#[test]
fn a_var_later_in_a_body_is_its_own_in_all_of_it() {
    let r = report(concat!(
        "var doc = app.activeDocument, out = new File(p);\n",
        "function tidy() {\n",
        "  doc.acmeName;\n",
        "  out.remove();\n",
        "  confirm('a');\n",
        "  selection.length;\n",
        "  prompt('b');\n",
        "  for (var i = 0, doc; i < 1; i++) {}\n",
        "  if (ok) { var out = 1, confirm = ask; }\n",
        "  var helper = function () { var x; }, prompt = ask\n",
        "  x, alert('c');\n",
        "  var selection;\n",
        "}\n",
        "doc.acmeLayers;\n",
        "out.remove();\n",
        "confirm('d');\n",
    ));
    assert_eq!(feature(&r, "notify").map(|f| f.lines.as_slice()), Some(&[11, 16][..]));
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[15][..])]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers"]);
}

/// When a function's body declares `Folder`, `$` or `Socket` later on, each is the body's own in all
/// of the body, as `alert` and `File` are, and the platform's object again after the body.
#[test]
fn each_kind_of_platform_name_a_body_declares_later_is_its_own() {
    let r = report(concat!(
        "function run() {\n",
        "  var t = Folder.temp; t.remove();\n",
        "  $.writeln('a');\n",
        "  new Socket();\n",
        "  var Folder = {}, $ = {}, Socket = Object;\n",
        "}\n",
        "Folder.temp.remove();\n",
        "$.writeln('b');\n",
        "new Socket();\n",
    ));
    for (id, line) in [("file.objects", 7), ("file.places", 7), ("dollar", 8), ("socket", 9)] {
        assert_eq!(feature(&r, id).map(|f| f.lines.as_slice()), Some(&[line][..]), "{id}");
    }
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[7][..]), ("network", &[9][..])]);
}

/// A function's parameters are its own, and not the code's around it. A `catch` block's parameter
/// is the block's own (ECMA-262 3rd edition, section 12.14), while a `var` statement in the block
/// declares a name of the function's own.
#[test]
fn parameters_and_catch_parameters_are_own_only_where_they_are_declared() {
    let r = report(concat!(
        "alert('a');\n",
        "function show(alert, File) { alert('b'); new File(p).remove(); }\n",
        "function run() {\n",
        "  confirm('c');\n",
        "  try { x(); } catch (confirm) { confirm('d'); }\n",
        "  confirm('e');\n",
        "}\n",
        "function load() {\n",
        "  prompt('f');\n",
        "  try { x(); } catch (prompt) { var prompt = 1; prompt('g'); }\n",
        "  prompt('h');\n",
        "}\n",
    ));
    assert_eq!(feature(&r, "notify").map(|f| f.lines.as_slice()), Some(&[1, 4, 6][..]));
    assert!(r.outside.is_empty(), "{:?}", r.outside);
}

/// A nested function reads the names the functions around it declare later in their bodies, and a
/// name a function declares holds neither in the code around it nor in another function.
#[test]
fn a_nested_function_reads_what_the_functions_around_it_declare_later() {
    let r = report(concat!(
        "var doc = app.activeDocument;\n",
        "function outer() {\n",
        "  function inner() { alert('a'); doc.acmeName; }\n",
        "  var alert = log, doc = 1;\n",
        "}\n",
        "function first() { var confirm; function nested() { var prompt; } prompt('b'); }\n",
        "function second() { confirm('c'); doc.acmeLayers; }\n",
        "alert('d');\n",
    ));
    assert_eq!(feature(&r, "notify").map(|f| f.lines.as_slice()), Some(&[6, 7, 8][..]));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers"]);
}

/// The global code's function declarations make their names the script's own from the start of the
/// file, and a member chain or a File object that another file's variable of that name holds stays.
/// A `var` statement in the global code leaves the host's object of that name as it is until a value
/// is assigned (ECMA-262 3rd edition, sections 10.1.3, 10.1.5 and 10.2.1), and a function declared
/// inside a block is the script's own from where it stands.
#[test]
fn a_function_declared_later_in_the_global_code_is_the_scripts_own_from_the_start() {
    let r = report(concat!(
        "alert('a');\n",
        "app.activeDocument.acmeName;\n",
        "var f = new File(p); f.remove();\n",
        "confirm('b');\n",
        "var confirm = ask;\n",
        "confirm('c');\n",
        "if (ok) { prompt('d'); function prompt() {} }\n",
        "prompt('e');\n",
        "function alert(m) {}\n",
        "function app() {}\n",
        "function File(p) {}\n",
    ));
    assert_eq!(feature(&r, "notify").map(|f| f.lines.as_slice()), Some(&[4, 7][..]));
    assert!(feature(&r, "file.objects").is_none(), "{:?}", r.features);
    assert!(r.outside.is_empty(), "{:?}", r.outside);
    assert!(r.members.is_empty(), "{:?}", r.members);
    let mut m = mem(&[("lib.jsxinc", "var out = new File(p);\n")]);
    let r = check("main.jsx", b"#include 'lib.jsxinc'\nout.remove();\nfunction out() {}\n", Some(&mut m), &Limits::default());
    let outside: Vec<(&str, &[usize])> = r.outside.iter().map(|o| (o.id, o.lines.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", &[2][..])]);
}

/// A function declaration is not evaluated where it stands (ECMA-262 3rd edition, section 14): a
/// member chain assigned to the function's name before the declaration stays.
#[test]
fn a_chain_assigned_before_a_function_declaration_stays() {
    let r = report(concat!(
        "doc = app.activeDocument;\n",
        "function doc() {}\n",
        "doc.acmeName;\n",
        "function run() { sel = selection; function sel() {} sel.length; }\n",
    ));
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeName", "selection", "selection.length"]);
}

/// A name a function body or the global code declares past [`Limits::entries`], or in a function
/// nested past [`Limits::nesting`], is the script's own only from where it is declared, and a name
/// left out marks the report truncated.
#[test]
fn declarations_past_a_limit_hold_from_where_they_stand() {
    let notify = |src: &str, limits: &Limits| {
        let r = check("test.jsx", src.as_bytes(), None, limits);
        (feature(&r, "notify").map(|f| f.lines.clone()).unwrap_or_default(), r.truncated)
    };
    let entries = Limits { entries: 3, ..Limits::default() };
    assert_eq!(notify("function g() {\n  alert('a');\n  var b, c, alert;\n}\n", &entries), (vec![], false));
    assert_eq!(notify("function g() {\n  alert('a');\n  var b, c, d, alert;\n}\n", &entries), (vec![2], true));
    assert_eq!(notify("function g(b, c) {\n  alert('a');\n  var alert;\n}\n", &entries), (vec![], false));
    assert_eq!(notify("function g(b, c, d) {\n  alert('a');\n  var alert;\n}\n", &entries), (vec![2], true));
    assert_eq!(notify("alert('a');\nfunction b() {}\nfunction c() {}\nfunction alert() {}\n", &entries), (vec![], false));
    assert_eq!(notify("alert('a');\nfunction b() {}\nfunction c() {}\nfunction d() {}\nfunction alert() {}\n", &entries), (vec![1], true));
    let nesting = Limits { nesting: 4, ..Limits::default() };
    let four = "function a() { function b() { function c() { function d() {\n  alert('a');\n  var alert;\n} } } }\n";
    let five = "function a() { function b() { function c() { function d() { function e() {\n  alert('a');\n  var alert;\n} } } } }\n";
    assert_eq!(notify(four, &nesting).0, Vec::<usize>::new());
    assert_eq!(notify(five, &nesting).0, [2]);
    assert_eq!(notify(five, &Limits::default()).0, Vec::<usize>::new());
}

#[test]
fn application_methods_are_features_not_members() {
    let r = report("app.executeMenuCommand('acmecopyall');\napp.selectTool('acme');\n");
    assert!(r.members.is_empty(), "{:?}", r.members);
    assert_eq!(r.needs, [Need::MenuCommands, Need::HostOnly]);
}

#[test]
fn type_names_compared_or_created_are_listed() {
    let r = report(concat!(
        "if (item.typename == 'AcmeShape') {}\n",
        "var c = new AcmeColor();\n",
        "function Helper() {}\n",
        "var h = new Helper();\n",
        "var d = new Date();\n",
        "var Maker = function () {};\n",
        "var m = new Maker();\n",
    ));
    let types: Vec<&str> = r.types.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(types, ["AcmeColor", "AcmeShape"]);
    assert_eq!(r.needs, [Need::Dom]);
}

#[test]
fn menu_commands_are_counted_with_their_lines() {
    let r = report(concat!(
        "app.executeMenuCommand('acmecopyall');\n",
        "app.executeMenuCommand(\"acmecopyall\");\n",
        "app.executeMenuCommand('acme\\x41');\n",
        "app.executeMenuCommand(name);\n",
        "app.executeMenuCommand('acme' + n);\n",
        "app.executeMenuCommand('\\141cmecopyall');\n",
    ));
    let strings: Vec<(&str, usize, &[usize])> = r.menu_strings.iter().map(|s| (s.value.as_str(), s.count, s.lines.as_slice())).collect();
    assert_eq!(strings, [("acmecopyall", 3, &[1, 2, 6][..]), ("acmeA", 1, &[3][..])]);
    assert_eq!(r.menu_dynamic, 2);
    assert_eq!(feature(&r, "menuCommand").map(|f| f.count), Some(6));
}

#[test]
fn effects_outside_the_document_are_listed_in_order() {
    let r = report(concat!(
        "var log = new File(p);\n",
        "log.open('a');\n",
        "log.open('w');\n",
        "log.remove();\n",
        "var s = new Socket();\n",
        "var lib = new ExternalObject('lib:x');\n",
        "var bt = new BridgeTalk();\n",
        "$.setenv('K', 'V');\n",
        "eval(code);\n",
        "out.execute();\n",
        "app.preferences.acmeFlag = true;\n",
        "var v = app.preferences.acmeFlag;\n",
        "app.preferences.setAcmeFlag('k', 1);\n",
        "app.preferences.removeAcmeFlag('k');\n",
        "var w = app.preferences.getAcmeFlag('k');\n",
    ));
    let outside: Vec<(&str, usize, &[usize], &[&str])> = r.outside.iter().map(|o| (o.id, o.count, o.lines.as_slice(), o.modes.as_slice())).collect();
    assert_eq!(
        outside,
        [
            ("filesWritten", 3, &[2, 3, 4][..], &["a", "w"][..]),
            ("filesOpened", 1, &[10][..], &[][..]),
            ("network", 1, &[5][..], &[][..]),
            ("nativeCode", 1, &[6][..], &[][..]),
            ("otherApps", 1, &[7][..], &[][..]),
            ("environment", 1, &[8][..], &[][..]),
            ("unseenCode", 1, &[9][..], &[][..]),
            ("preferences", 3, &[11, 13, 14][..], &[][..]),
        ]
    );
    assert_eq!(feature(&r, "preferences").map(|f| f.count), Some(5), "reading a preference is no change");
}

#[test]
fn problems_are_listed_with_their_lines() {
    let r = report(concat!(
        "var a = 'open;\n",
        "var b = /open;\n",
        "var c = (1;\n",
        "var d = 2];\n",
        "#pragma once\n",
        "var e = `tpl`;\n",
        "var x = <a><b></a>;\n",
        "/* open\n",
    ));
    assert_eq!(
        problems(&r),
        [
            ("unterminatedString", Some(1), None),
            ("unterminatedRegExp", Some(2), None),
            ("unexpectedBracket", Some(4), Some("]")),
            ("unknownDirective", Some(5), Some("#pragma")),
            ("newerSyntax", Some(6), Some("`")),
            ("unterminatedXml", Some(7), None),
            ("unclosedBracket", Some(3), Some("(")),
        ]
    );
    assert_eq!(problems(&report("/* open")), [("unterminatedComment", Some(1), None)]);
    assert_eq!(problems(&report("var t = `open")), [("unterminatedTemplate", Some(1), None), ("newerSyntax", Some(1), Some("`"))]);
}

#[test]
fn a_regular_expression_may_start_a_statement_after_a_condition() {
    for src in [
        "if (ok) /[)]/.test(s);\n",
        "while (more(s)) /[(]/.exec(s);\n",
        "for (var i = 0; i < n; i++) /[)]/.test(list[i]);\n",
        "for each (var v in list) /[)]/.test(v);\n",
        "with (o) /a[)]b/.test(s);\n",
    ] {
        assert_eq!(problems(&report(src)), [], "{src}");
    }
    assert_eq!(tokens("if (f(a)) /[)]/.test(s)").get(7), Some(&(T::RegExp, "/[)]/")));
    // After the `)` of a call or a grouping, `/` divides.
    for src in ["f(a) / 2 / b", "(a) / 2 / b", "if (f(a) / 2 / b) x();", "if (a) (b) / 2 / c;"] {
        assert!(tokens(src).iter().all(|(kind, _)| *kind != T::RegExp), "{src}: {:?}", tokens(src));
    }
}

#[test]
fn a_slash_divides_after_an_object_literal_and_starts_a_regular_expression_after_a_block() {
    for src in [
        "var n = {} / 2; alert(n);\n",
        "var o = {a: {b: 1}} / 2; alert(o);\n",
        "f({} / 2); alert(1);\n",
        "var x = <x>{ {a: 1} / 2 }</x>; alert(x);\n",
        "var n = function () {} / 2; alert(n);\n",
        "x = [function () { return {}; } / 2]; alert(x);\n",
        "var n = ok ? 1 : function () {} / 2; alert(n);\n",
        "var m = ok ? {a: 1} : function () {} / 2; alert(m);\n",
    ] {
        let r = report(src);
        assert_eq!(problems(&r), [], "{src}");
        assert!(feature(&r, "notify").is_some(), "{src}");
    }
    for src in [
        "if (a) { b(); } /[)]/.test(s);\n",
        "function f() {} /[)]/.test(s);\n",
        "{ function g() {} /[)]/.test(s); }\n",
        "{ } /[)]/.test(s);\n",
        "switch (a) { case 1: {} /[)]/.test(s); }\n",
        "switch (a) { case b ? 1 : 2: {} /[)]/.test(s); }\n",
        "function f() { return\n{}\n/[)]/.test(s); }\n",
        "while (a) { break\n{}\n/[)]/.test(s); }\n",
    ] {
        assert_eq!(problems(&report(src)), [], "{src}");
    }
}

#[test]
fn a_keyword_after_a_dot_is_a_property_name() {
    for src in [
        "var n = obj.return / 2; alert(n);\n",
        "var n = obj.typeof / 2; alert(n);\n",
        "var n = obj.if (a) / 2; alert(n);\n",
        "var n = obj.else / 2; alert(n);\n",
        "var n = obj.throw\n/ 2; alert(n);\n",
        "var n = obj.function / 2; {} /[)]/.test(s); alert(n);\n",
    ] {
        let r = report(src);
        assert_eq!(problems(&r), [], "{src}");
        assert!(feature(&r, "notify").is_some(), "{src}");
    }
}

#[test]
fn braces_in_the_strings_comments_and_regular_expressions_of_xml_code_are_text() {
    for src in [
        "var x = <x>{\"{\"}</x>;\n",
        "var x = <x a={'}'}>{f(/[{]/)}</x>;\n",
        "var x = <x>{a /* { */ + b}</x>;\n",
        "var x = <x>{a // {\n}</x>;\n",
        "var x = <x>{n / 2 + m}</x>;\n",
        "var x = <x>{<y>{'}'}</y>}</x>;\n",
    ] {
        let r = report(&format!("{src}app.executeMenuCommand('acmecopyall');\n"));
        assert_eq!(problems(&r), [], "{src}");
        let strings: Vec<&str> = r.menu_strings.iter().map(|s| s.value.as_str()).collect();
        assert_eq!(strings, ["acmecopyall"], "{src}");
    }
}

#[test]
fn the_code_in_the_braces_of_an_xml_literal_is_checked() {
    let r = report(concat!(
        "var x = <x a={app.acmeMode}>{$.evalFile('other.jsx')}</x>;\n",
        "var y = <list>{items.map(function (i) { return <item>{app.executeMenuCommand('acmecopyall')}</item>; })}</list>;\n",
    ));
    assert_eq!(problems(&r), []);
    // Three XML literals, one of them in another's code.
    assert_eq!(feature(&r, "e4x").map(|f| (f.count, f.lines.as_slice())), Some((3, &[1, 2][..])));
    assert_eq!(feature(&r, "dollar.evalFile").map(|f| f.lines.as_slice()), Some(&[1][..]));
    assert_eq!(r.outside.iter().map(|o| o.id).collect::<Vec<_>>(), ["unseenCode"]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.acmeMode"]);
    let strings: Vec<&str> = r.menu_strings.iter().map(|s| s.value.as_str()).collect();
    assert_eq!(strings, ["acmecopyall"]);
    // An XML literal the text ends in is one problem, at the line it starts on.
    assert_eq!(problems(&report("var x = <x>{\n  a + b;\n")), [("unterminatedXml", Some(1), None)]);
}

#[test]
fn later_syntax_is_reported_once_per_construct_and_line() {
    let r = report(concat!(
        "var f = (a) => a;\n",
        "var g = (a) => (b) => b;\n",
        "class Shape {}\n",
        "var all = [...list];\n",
        "var v = a?.b ?? c;\n",
        "var p = 2 ** 3;\n",
        "var n = [0b101, 0o17, 1_000, 10n];\n",
        "var q = this.#hidden;\n",
        "async function go() {}\n",
        "var ok = cond ?.5 : 1;\n",
        "let count = 1;\n",
        "for (let i = 0; i < 2; i++) {}\n",
        "var let = 1; let = 2; let.x = 3; if (let in o) {}\n",
    ));
    let found: Vec<(usize, &str)> = r.problems.iter().filter_map(|p| Some((p.line?, p.detail.as_deref()?))).collect();
    assert_eq!(
        found,
        [
            (1, "=>"),
            (2, "=>"),
            (3, "class"),
            (4, "..."),
            (5, "?."),
            (5, "??"),
            (6, "**"),
            (7, "0b101"),
            (7, "0o17"),
            (7, "1_000"),
            (7, "10n"),
            (8, "#hidden"),
            (9, "async"),
            (11, "let"),
            (12, "let"),
        ]
    );
}

#[test]
fn deep_nesting_is_a_problem_not_a_stack_overflow() {
    let inputs: Vec<String> = vec![
        "(".repeat(100_000),
        "[".repeat(100_000),
        "{".repeat(100_000),
        "x = ".repeat(100_000),
        "if (x) ".repeat(100_000) + ";",
        "!".repeat(100_000) + "x",
        "a ? ".repeat(100_000),
        "var x = ".to_string() + &"<a>".repeat(100_000),
        "app.x(".repeat(50_000),
        "app[".repeat(50_000),
        "x = {a: ".repeat(50_000),
        "var x = ".to_string() + &"<a>{".repeat(50_000),
        "x = (".repeat(50_000),
        "x = new File(".repeat(50_000),
        "function f() { var alert; ".repeat(50_000),
        "x += a ? ".repeat(50_000),
    ];
    // A small stack: anything that recursed once per level would overflow it and end the process.
    let checked = std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(move || {
            inputs.iter().map(|s| report(s)).map(|r| (r.problems.iter().any(|p| p.code == "tooDeep"), r.problems.len())).collect::<Vec<_>>()
        })
        .unwrap()
        .join()
        .unwrap();
    let deep: Vec<bool> = checked.iter().map(|c| c.0).collect();
    assert_eq!(deep, [true, true, true, false, false, false, false, false, true, true, true, true, true, true, true, false]);
    assert!(checked.iter().all(|c| c.1 <= Limits::default().entries));
    // Deeper than the cap: one problem, and the check goes on.
    let small = Limits { nesting: 8, ..Limits::default() };
    let r = check("t.jsx", format!("{}alert(1);{}", "(".repeat(20), ")".repeat(20)).as_bytes(), None, &small);
    assert_eq!(problems(&r), [("tooDeep", Some(1), None)]);
    assert!(feature(&r, "notify").is_some());
    // Past the cap, a value in grouping parentheses is not followed, nor is one without them.
    let grouped = b"var a = (b = (c = (new File(p))));\nc.remove();\n";
    assert_eq!(check("t.jsx", grouped, None, &Limits::default()).outside.len(), 1);
    assert!(check("t.jsx", grouped, None, &Limits { nesting: 2, ..Limits::default() }).outside.is_empty());
    let bare = b"var a = new File(b = new File(c = new File(p)));\nc.remove();\n";
    assert_eq!(check("t.jsx", bare, None, &Limits::default()).outside.len(), 1);
    assert!(check("t.jsx", bare, None, &Limits { nesting: 2, ..Limits::default() }).outside.is_empty());
    // Past the cap, a call on a File in grouping parentheses is not followed.
    let receiver = b"var f = new File(p);\n(((f))).remove();\n";
    assert_eq!(check("t.jsx", receiver, None, &Limits::default()).outside.len(), 1);
    assert!(check("t.jsx", receiver, None, &Limits { nesting: 2, ..Limits::default() }).outside.is_empty());
    // An XML literal in the code of an XML literal takes two levels: its markup and the code. The
    // literal ends where a level past the cap would start.
    let nested = b"var x = <a>{<b>{<c>{<d>{1}</d>}</c>}</b>}</a>;\nalert(1);\n";
    assert_eq!(problems(&check("t.jsx", nested, None, &small)), []);
    let r = check("t.jsx", nested, None, &Limits { nesting: 6, ..Limits::default() });
    assert_eq!(problems(&r).first(), Some(&("tooDeep", Some(1), None)));
    assert_eq!(r.problems.iter().filter(|p| p.code == "tooDeep").count(), 1, "{:?}", r.problems);
    assert!(feature(&r, "notify").is_some());
}

/// The inputs that take the most steps for their size: brackets never closed, parameters and the
/// names a function declares, and grouped values, each after a name written with an escape. The
/// text is read three times (for the escapes, the names declared and the analysis), and the work
/// stays within the bound the properties below check.
#[test]
fn the_costliest_inputs_stay_within_five_steps_a_byte() {
    let names: Vec<String> = (0..600).map(|i| format!("n{i}")).collect();
    for src in [
        format!("\\u0061;{}", "{".repeat(2_000)),
        format!("\\u0061;function f({}) {{ var {}; }}", names.join(","), names.join(",")),
        format!("\\u0061;function f({}) {{}}", vec!["a"; 900].join(",")),
        format!("\\u0061;{}", "x=(".repeat(600)),
    ] {
        let r = report(&src);
        assert!(r.steps <= 5 * src.len() + 16, "{} steps for {} bytes", r.steps, src.len());
    }
}

#[test]
fn lists_stop_at_their_limits_and_echoes_are_cut() {
    let mut s = String::new();
    for i in 0..20_000 {
        s += &format!("app.executeMenuCommand('acme{i}');\nfoo = app.m{i};\n");
    }
    let r = report(&s);
    let limits = Limits::default();
    assert_eq!((r.menu_strings.len(), r.members.len()), (limits.entries, limits.entries));
    assert!(r.truncated);
    assert_eq!(feature(&r, "menuCommand").map(|f| (f.count, f.lines.len())), Some((20_000, limits.lines)));
    let long = "x".repeat(500);
    let r = report(&format!("app.executeMenuCommand('{long}');\n#target {long}\napp.{long}.y;\nnew {long}();\n"));
    let echoed = r
        .menu_strings
        .iter()
        .map(|s| &s.value)
        .chain(r.members.iter().map(|m| &m.name))
        .chain(r.directives.iter().map(|d| &d.value))
        .chain([&r.targets[0].app]);
    for e in echoed {
        assert_eq!(e.chars().count(), limits.echo, "{e}");
        assert!(e.ends_with('…'));
    }
    assert!(!r.truncated);
}

#[test]
fn variables_past_the_limit_mark_the_report_truncated() {
    let limits = Limits { entries: 3, ..Limits::default() };
    for (src, truncated) in [
        ("var f1 = new File(p), f2 = new File(p), f3 = new File(p);\nf3.remove();\n", false),
        ("var f1 = new File(p), f2 = new File(p), f3 = new File(p), f4 = new File(p);\nf4.remove();\n", true),
        ("var f1 = new File(p), f2 = new File(p), f3 = new File(p);\nvar f4 = f1;\n", true),
        ("var f1 = new File(p), f2 = new File(p), f3 = new File(p);\nvar f4 = f1", true),
        ("var f1 = (new File(p)), f2 = (new File(p)), f3 = (new File(p)), f4 = (new File(p));\nf4.remove();\n", true),
        ("var d1 = app.activeDocument, d2 = app.activeDocument, d3 = app.activeDocument;\n", false),
        ("var d1 = app.activeDocument, d2 = app.activeDocument, d3 = app.activeDocument, d4 = app.activeDocument;\n", true),
        ("function g(a, b, c) {}\n", false),
        ("function g(a, b, c, d) {}\n", true),
        ("function g() { var a = 1, b = 2, c = 3; }\n", false),
        ("function g() { var a = 1, b = 2, c = 3, e = 4; }\n", true),
    ] {
        let r = check("test.jsx", src.as_bytes(), None, &limits);
        assert_eq!(r.truncated, truncated, "{src}");
        if src.contains("remove") {
            assert_eq!(r.outside.is_empty(), truncated, "{src}");
        }
    }
    // The script and the file it includes hold four File variables between them.
    let mut m = mem(&[("lib.jsxinc", "var f3 = new File(p), f4 = new File(p);\n")]);
    let r = check("main.jsx", b"#include 'lib.jsxinc'\nvar f1 = new File(p), f2 = new File(p);\n", Some(&mut m), &limits);
    assert!(r.truncated);
}

/// Files in memory, with every path asked for and the bytes handed out.
struct Mem {
    files: Vec<(String, Vec<u8>)>,
    refused: Vec<&'static str>,
    asked: Vec<String>,
    served: usize,
}

fn mem(files: &[(&str, &str)]) -> Mem {
    Mem { files: files.iter().map(|(p, s)| (p.to_string(), s.as_bytes().to_vec())).collect(), refused: vec![], asked: vec![], served: 0 }
}

impl Loader for Mem {
    fn load(&mut self, path: &str, max: usize) -> Loaded {
        self.asked.push(path.to_string());
        if self.refused.contains(&path) {
            return Loaded::Refused;
        }
        match self.files.iter().find(|(p, _)| p == path) {
            Some((_, b)) => {
                let head = b[..b.len().min(max)].to_vec();
                self.served += head.len();
                Loaded::Bytes(head)
            }
            None => Loaded::Missing,
        }
    }
}

/// An include as `(value, line, file, status, path)`.
type IncludeRow<'r> = (&'r str, usize, Option<&'r str>, IncludeStatus, Option<&'r str>);

fn includes(r: &Report) -> Vec<IncludeRow<'_>> {
    r.includes.iter().map(|i| (i.value.as_str(), i.line, i.file.as_deref(), i.status, i.path.as_deref())).collect()
}

#[test]
fn includes_are_followed_inside_the_scripts_folder() {
    let mut m = mem(&[
        ("lib/a.jsxinc", "#include 'b.jsxinc'\n$.writeln(1);\nvar s = 'open;\n"),
        ("lib/b.jsxinc", "alert('b');\n#include '../main.jsx'\n#include 'b.jsxinc'\n"),
        ("shared/c.jsxinc", "var u = new UnitValue(1, 'in');\n"),
        ("bin.jsxinc", "@JSXBIN@ES@2.0@AcmeBody"),
        ("secret.jsxinc", "alert(1);"),
    ]);
    m.refused = vec!["secret.jsxinc"];
    let main = concat!(
        "#include 'lib/a.jsxinc'\n",
        "#include 'lib/a.jsxinc'\n",
        "#include 'nope.jsxinc'\n",
        "#include '/etc/x.jsx'\n",
        "#include '../up.jsx'\n",
        "#includepath 'shared'\n",
        "#include 'c.jsxinc'\n",
        "#include 'bin.jsxinc'\n",
        "#include 'secret.jsxinc'\n",
    );
    let r = check("folder/main.jsx", main.as_bytes(), Some(&mut m), &Limits::default());
    use IncludeStatus::*;
    // main.jsx includes lib/a, lib/a includes lib/b and lib/b includes main.jsx: each of those lines
    // is part of a cycle, as is lib/b's line that includes lib/b.
    assert_eq!(
        includes(&r),
        [
            ("lib/a.jsxinc", 1, None, Cycle, Some("lib/a.jsxinc")),
            ("lib/a.jsxinc", 2, None, Cycle, Some("lib/a.jsxinc")),
            ("nope.jsxinc", 3, None, Missing, None),
            ("/etc/x.jsx", 4, None, NotFollowed, None),
            ("../up.jsx", 5, None, NotFollowed, None),
            ("c.jsxinc", 7, None, Checked, Some("shared/c.jsxinc")),
            ("bin.jsxinc", 8, None, Unreadable, Some("bin.jsxinc")),
            ("secret.jsxinc", 9, None, NotFollowed, None),
            ("b.jsxinc", 1, Some("lib/a.jsxinc"), Cycle, Some("lib/b.jsxinc")),
            ("../main.jsx", 2, Some("lib/b.jsxinc"), Cycle, Some("main.jsx")),
            ("b.jsxinc", 3, Some("lib/b.jsxinc"), Cycle, Some("lib/b.jsxinc")),
        ]
    );
    // A refused file is looked for in the include path's folder next.
    assert_eq!(
        m.asked,
        ["lib/a.jsxinc", "nope.jsxinc", "c.jsxinc", "shared/c.jsxinc", "bin.jsxinc", "secret.jsxinc", "shared/secret.jsxinc", "lib/b.jsxinc"],
        "each file is read once"
    );
    // What the included files use counts, without their lines.
    for id in ["dollar", "notify", "unitValue"] {
        assert_eq!(feature(&r, id).map(|f| (f.count, f.lines.len())), Some((1, 0)), "{id}");
    }
    let p = &r.problems[0];
    assert_eq!((p.code, p.line, p.file.as_deref()), ("unterminatedString", Some(3), Some("lib/a.jsxinc")));
}

#[test]
fn files_that_include_each_other_are_cycles_wherever_they_were_found() {
    // The script includes a and b, and a and b include each other.
    let mut m = mem(&[("a.jsxinc", "#include 'b.jsxinc'\n"), ("b.jsxinc", "#include 'a.jsxinc'\n")]);
    let r = check("main.jsx", b"#include 'a.jsxinc'\n#include 'b.jsxinc'\n", Some(&mut m), &Limits::default());
    use IncludeStatus::*;
    assert_eq!(
        includes(&r),
        [
            ("a.jsxinc", 1, None, Checked, Some("a.jsxinc")),
            ("b.jsxinc", 2, None, Checked, Some("b.jsxinc")),
            ("b.jsxinc", 1, Some("a.jsxinc"), Cycle, Some("b.jsxinc")),
            ("a.jsxinc", 1, Some("b.jsxinc"), Cycle, Some("a.jsxinc")),
        ]
    );
    assert_eq!(m.asked, ["a.jsxinc", "b.jsxinc"], "each file is read once");
}

#[test]
fn include_lines_share_the_folders_of_the_include_path_before_them() {
    let folders: Vec<String> = (0..64).map(|i| format!("{}{i}", "d".repeat(4000))).collect();
    let mut src = format!("#includepath '{}'\n", folders.join(";"));
    for i in 0..1000 {
        src += &format!("#include 'f{i}.jsxinc'\n");
    }
    let limits = Limits::default();
    let mut acc = report::Acc::new(&limits);
    let declared = analyze::declarations(&src, &mut acc);
    let scan = analyze::scan(&src, &declared, None, std::rc::Rc::from([]), &analyze::Known::default(), &mut acc);
    let (first, rest) = scan.includes.split_first().unwrap();
    assert_eq!((first.paths.len(), rest.len()), (64, 999));
    assert!(rest.iter().all(|i| std::rc::Rc::ptr_eq(&i.paths, &first.paths)), "one list of folders, not a copy for each line");
}

#[test]
fn names_and_folders_longer_than_a_path_are_not_followed() {
    let long = "d".repeat(4097);
    let mut m = mem(&[]);
    let src = format!("#includepath '{long};lib'\n#include 'a.jsxinc'\n#include '{long}'\n");
    let r = check("main.jsx", src.as_bytes(), Some(&mut m), &Limits::default());
    assert_eq!(m.asked, ["a.jsxinc", "lib/a.jsxinc"]);
    let statuses: Vec<IncludeStatus> = r.includes.iter().map(|i| i.status).collect();
    assert_eq!(statuses, [IncludeStatus::Missing, IncludeStatus::NotFollowed]);
}

#[test]
fn what_a_variable_holds_is_known_in_every_file_of_the_check() {
    let mut m =
        mem(&[("write.jsxinc", "f.open('w');\nf.write('data');\n"), ("lib.jsxinc", "var doc = app.activeDocument;\nvar out = new File(p);\n")]);
    let main = "var f = new File('out.txt');\n#include 'write.jsxinc'\n#include 'lib.jsxinc'\ndoc.acmeLayers;\nout.remove();\n";
    let r = check("main.jsx", main.as_bytes(), Some(&mut m), &Limits::default());
    // The script's `f` in an included file, and an included file's `doc` and `out` in the script.
    let outside: Vec<(&str, usize, &[usize], &[&str])> = r.outside.iter().map(|o| (o.id, o.count, o.lines.as_slice(), o.modes.as_slice())).collect();
    assert_eq!(outside, [("filesWritten", 3, &[5][..], &["w"][..])]);
    let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayers"]);
    assert_eq!(m.asked, ["write.jsxinc", "lib.jsxinc"], "each file is read once");
}

#[test]
fn what_a_variable_holds_is_known_also_when_another_files_variable_gives_it() {
    for order in [["a.jsxinc", "b.jsxinc"], ["b.jsxinc", "a.jsxinc"]] {
        let mut m = mem(&[("a.jsxinc", "var doc = app.activeDocument;\n"), ("b.jsxinc", "var layer = doc.acmeLayer;\n")]);
        let main = format!("#include '{}'\n#include '{}'\nlayer.remove();\n", order[0], order[1]);
        let r = check("main.jsx", main.as_bytes(), Some(&mut m), &Limits::default());
        let members: Vec<&str> = r.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(members, ["app.activeDocument", "app.activeDocument.acmeLayer", "app.activeDocument.acmeLayer.remove()"], "{order:?}");
    }
}

#[test]
fn a_file_two_branches_include_is_followed_from_the_shallower_one() {
    let mut m = mem(&[
        ("a.jsxinc", "#include 'shared.jsxinc'\n"),
        ("b.jsxinc", "#include 'c.jsxinc'\n"),
        ("c.jsxinc", "#include 'shared.jsxinc'\n"),
        ("shared.jsxinc", "#include 'dep.jsxinc'\n"),
        ("dep.jsxinc", "var s = new Socket();\n"),
    ]);
    let limits = Limits { include_depth: 3, ..Limits::default() };
    let r = check("main.jsx", b"#include 'a.jsxinc'\n#include 'b.jsxinc'\n", Some(&mut m), &limits);
    assert!(r.includes.iter().all(|i| i.status == IncludeStatus::Checked), "{:?}", includes(&r));
    assert_eq!(r.outside.iter().map(|o| o.id).collect::<Vec<_>>(), ["network"]);
}

#[test]
fn include_limits_stop_the_following() {
    // A chain of files, each including the next.
    let files: Vec<(String, String)> = (0..20).map(|i| (format!("f{i}.jsxinc"), format!("#include 'f{}.jsxinc'\n", i + 1))).collect();
    let pairs: Vec<(&str, &str)> = files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
    // The script is 20 bytes, and each file 21.
    let statuses = |limits: &Limits| {
        let mut m = mem(&pairs);
        let r = check("main.jsx", b"#include 'f0.jsxinc'", Some(&mut m), limits);
        r.includes.iter().map(|i| i.status).collect::<Vec<_>>()
    };
    let mut want = vec![IncludeStatus::Checked; 16];
    want.push(IncludeStatus::Limit);
    assert_eq!(statuses(&Limits::default()), want, "followed 16 deep");
    let three = Limits { includes: 3, ..Limits::default() };
    assert_eq!(statuses(&three), [IncludeStatus::Checked, IncludeStatus::Checked, IncludeStatus::Checked, IncludeStatus::Limit]);
    // The second file would take the total past 50 bytes.
    let total = Limits { total_bytes: 50, ..Limits::default() };
    assert_eq!(statuses(&total), [IncludeStatus::Checked, IncludeStatus::Limit]);
    let one_file = Limits { file_bytes: 20, ..Limits::default() };
    assert_eq!(statuses(&one_file), [IncludeStatus::Limit]);
    // Each file is found at the first path looked up.
    let two_lookups = Limits { lookups: 2, ..Limits::default() };
    assert_eq!(statuses(&two_lookups), [IncludeStatus::Checked, IncludeStatus::Checked, IncludeStatus::Limit]);
}

#[test]
fn included_files_not_checked_count_toward_the_limits() {
    use IncludeStatus::*;
    let big = "a".repeat(2000);
    let mut pairs: Vec<(String, String)> = (0..20).map(|i| (format!("big{i}.jsxinc"), big.clone())).collect();
    pairs.extend((0..10).map(|i| (format!("bin{i}.jsxinc"), "@JSXBIN@ES@2.0@AcmeBody".to_string())));
    let files: Vec<(&str, &str)> = pairs.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
    // Files too large to check (over 1,000 bytes): the reads stop at the total, 1,500 bytes past the
    // script's own, with one byte more that shows a file is larger.
    let script: String = (0..20).map(|i| format!("#include 'big{i}.jsxinc'\n")).collect();
    let mut m = mem(&files);
    let r = check("main.jsx", script.as_bytes(), Some(&mut m), &Limits { file_bytes: 1000, total_bytes: script.len() + 1500, ..Limits::default() });
    assert_eq!(m.served, 1501);
    assert!(r.includes.iter().all(|i| i.status == Limit), "{:?}", includes(&r));
    // Compiled files count toward the files read.
    let script: String = (0..10).map(|i| format!("#include 'bin{i}.jsxinc'\n")).collect();
    let mut m = mem(&files);
    let r = check("main.jsx", script.as_bytes(), Some(&mut m), &Limits { includes: 3, ..Limits::default() });
    let statuses: Vec<IncludeStatus> = r.includes.iter().map(|i| i.status).collect();
    assert_eq!(statuses[..4], [Unreadable, Unreadable, Unreadable, Limit]);
    assert_eq!(m.asked.len(), 3);
}

#[test]
fn missing_includes_stop_at_the_lookup_limit() {
    // Each line names a missing file, looked up as given and in each of 64 include path folders:
    // 65 paths a line, so the first 15 lines take 975 of the 1,024 lookups.
    let folders: Vec<String> = (0..64).map(|i| format!("d{i}")).collect();
    let mut script = format!("#includepath '{}'\n", folders.join(";"));
    for i in 0..1000 {
        script += &format!("#include 'f{i}.jsxinc'\n");
    }
    let mut m = mem(&[]);
    let r = check("main.jsx", script.as_bytes(), Some(&mut m), &Limits::default());
    assert_eq!(m.asked.len(), Limits::default().lookups);
    let statuses: Vec<IncludeStatus> = r.includes.iter().map(|i| i.status).collect();
    assert_eq!(statuses.len(), 1000);
    assert!(statuses.iter().take(15).all(|s| *s == IncludeStatus::Missing), "{statuses:?}");
    assert!(statuses.iter().skip(15).all(|s| *s == IncludeStatus::Limit), "{statuses:?}");
}

#[test]
fn without_a_loader_includes_are_listed_unread() {
    let r = report("//@include 'a.jsxinc'\n");
    assert_eq!(includes(&r), [("a.jsxinc", 1, None, IncludeStatus::NotRead, None)]);
}

/// Text built from pieces of scripts: directives, strings, regular expressions, XML, later syntax,
/// every line break, a byte order mark and replaced bytes.
fn script_text() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "#target acme",
        "//@include \"a.jsxinc\"",
        "#includepath 'lib;..'",
        "#pragma",
        "#",
        "var",
        " x",
        "=",
        "app",
        "\\u0061pp",
        ".activeDocument",
        ".",
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        ";",
        ",",
        "'str'",
        "\"str",
        "'",
        "\\",
        "/re[/]x/g",
        "/",
        "/=",
        "<a b=\"1\">",
        "</a>",
        "<>",
        "</>",
        "<!--",
        "-->",
        "<![CDATA[",
        "{v}",
        ".@",
        "..",
        "::",
        "`tpl",
        "`",
        "=>",
        "?.",
        "??",
        "\n",
        "\r\n",
        "\r",
        "\u{2028}",
        "\u{feff}",
        "\u{fffd}",
        "é",
        "0x1F",
        "1e5",
        "1_000",
        "10n",
        "#priv",
        "/*",
        "*/",
        "//",
        "@JSXBIN@",
        "executeMenuCommand",
        "File",
        "Window",
        "'dialog'",
        "new",
        "function",
        "import",
        " ",
    ]);
    prop::collection::vec(pieces, 0..80).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The tokens cover the text exactly once, in order, each at least one character long.
    #[test]
    fn tokens_cover_the_text_exactly_once(text in script_text()) {
        let mut joined = String::new();
        for t in Lexer::new(&text, Limits::default().nesting) {
            prop_assert!(!t.text.is_empty());
            joined.push_str(t.text);
        }
        prop_assert_eq!(joined, text);
    }

    /// Any bytes are checked without a panic, every list within its limit, with work linear in the
    /// size.
    #[test]
    fn any_bytes_are_checked_in_linear_work(bytes in prop::collection::vec(any::<u8>(), 0..2_000), jsxbin in any::<bool>()) {
        let r = check(if jsxbin { "a.jsxbin" } else { "a.jsx" }, &bytes, None, &Limits::default());
        prop_assert!(r.steps <= 5 * bytes.len() + 16, "{} steps for {} bytes", r.steps, bytes.len());
    }

    #[test]
    fn script_like_text_is_checked_in_linear_work(text in script_text()) {
        let r = report(&text);
        prop_assert!(r.steps <= 5 * text.len() + 16, "{} steps for {} bytes", r.steps, text.len());
        let limits = Limits::default();
        for list in [r.targets.len(), r.directives.len(), r.includes.len(), r.members.len(), r.types.len(), r.menu_strings.len(), r.problems.len()] {
            prop_assert!(list <= limits.entries);
        }
    }

    /// Compiled input is never read, whatever follows its prefix.
    #[test]
    fn compiled_scripts_are_never_lexed(rest in prop::collection::vec(any::<u8>(), 0..500), blanks in "[ \t\r\n]{0,4}", bom in any::<bool>()) {
        let mut b = if bom { b"\xEF\xBB\xBF".to_vec() } else { vec![] };
        b.extend(blanks.as_bytes());
        b.extend(b"@JSXBIN@");
        b.extend(rest);
        let r = check("x.jsx", &b, None, &Limits::default());
        prop_assert_eq!(r.kind, Kind::Compiled);
        prop_assert_eq!(r.steps, 0);
    }

    /// Include graphs over files in memory, cycles and missing files among them: each file is read
    /// at most once, and no more files are read or paths looked up than the limits allow.
    #[test]
    fn include_graphs_respect_every_cap(
        edges in prop::collection::vec((0usize..10, 0usize..12), 0..40),
        most in 1usize..6,
        depth in 1usize..5,
        lookups in 1usize..12,
    ) {
        // Files f0 to f9; f10 and f11 are missing.
        let files: Vec<(String, String)> = (0..10)
            .map(|i| (format!("f{i}.jsxinc"), edges.iter().filter(|e| e.0 == i).map(|e| format!("#include 'f{}.jsxinc'\n", e.1)).collect()))
            .collect();
        let pairs: Vec<(&str, &str)> = files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
        let mut m = mem(&pairs);
        let limits = Limits { includes: most, include_depth: depth, lookups, ..Limits::default() };
        let r = check("main.jsx", b"#include 'f0.jsxinc'\n#include 'f1.jsxinc'\n", Some(&mut m), &limits);
        let read: Vec<&String> = m.asked.iter().filter(|p| files.iter().any(|(f, _)| f == *p)).collect();
        let unique: std::collections::BTreeSet<&&String> = read.iter().collect();
        prop_assert_eq!(unique.len(), read.len(), "read twice: {:?}", m.asked);
        prop_assert!(read.len() <= most);
        prop_assert!(m.asked.len() <= lookups, "{} paths looked up: {:?}", m.asked.len(), m.asked);
        prop_assert!(r.includes.iter().all(|i| i.status != IncludeStatus::NotRead));
    }
}

/// A function that writes to a variable the global code assigns a File object after it: the
/// variable declaration is hoisted, so the function writes a file, with a loader and without one.
#[test]
fn a_variable_assigned_after_the_function_using_it_holds_its_value() {
    let src = "function g(){ f.write('x'); }  var f = new File('a');";
    let without = report(src);
    let with = check("test.jsx", src.as_bytes(), Some(&mut mem(&[])), &Limits::default());
    assert!(feature(&without, "file.write").is_some(), "{:?}", without.features);
    assert!(feature(&with, "file.write").is_some(), "{:?}", with.features);
}

/// Script-like text with functions that use variables the code after them assigns, a chain of them
/// taking more than one scan to settle.
fn hoisting_text() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "function g(){ ",
        "function h(){ ",
        "}",
        "f.write('x'); ",
        "b.remove(); ",
        "e.acmeItems.add(); ",
        "b = a; ",
        "var a = f; ",
        "var f = new File('a'); ",
        "var d = app.activeDocument; ",
        "var e = d; ",
        "e = d.acmeLayers; ",
        "f = d; ",
    ]);
    (prop::collection::vec(pieces, 0..30), script_text()).prop_map(|(v, tail)| v.concat() + &tail)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// A script that includes nothing gets the same report with a loader and without one.
    #[test]
    fn a_script_without_includes_reports_the_same_with_and_without_a_loader(text in hoisting_text()) {
        let mut without = report(&text);
        prop_assume!(without.includes.is_empty());
        let mut with = check("test.jsx", text.as_bytes(), Some(&mut mem(&[])), &Limits::default());
        without.steps = 0;
        with.steps = 0;
        prop_assert_eq!(without, with);
    }
}

/// Chains of variables a function assigns before the code after it assigns their source take a
/// scan each to settle: the script is read at most seven times.
#[test]
fn variables_that_settle_over_several_scans_stay_linear() {
    let mut src = String::new();
    for i in 0..150 {
        src += &format!("function a{i}(){{ e{i} = d{i}; }} function b{i}(){{ d{i} = c{i}; }} function c{i}(){{ c{i} = b{i}; }} ");
        src += &format!("function d{i}(){{ b{i} = a{i}; }} var a{i} = new File('x');\n");
    }
    let r = report(&src);
    assert!(r.steps <= 9 * src.len() + 16, "{} steps for {} bytes", r.steps, src.len());
}
