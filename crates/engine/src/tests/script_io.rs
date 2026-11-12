//! `File` and `Folder` in a script: their classes and names, reading inside the script's own folder,
//! the paths refused before anything is looked up, the read limits, and the parts that write, which
//! are not available.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::file_access::{AutomationRoots, confine, script_probes};
use crate::*;

const DENIED: &str = "Script file access denied";

/// A fresh folder for test `name`, with a `main` folder inside it for the script.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-io-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    base
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

/// Runs `main/main.jsx` in `base`, with the text `text`.
fn run(base: &Path, text: &str) -> Value {
    let main = base.join("main/main.jsx");
    write(&main, text.as_bytes());
    Session::new().execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap()
}

/// The values the run of `text` logged.
fn logged(base: &Path, text: &str) -> Value {
    run(base, text)["log"].clone()
}

#[test]
fn file_constructor_identity_and_folder_path() {
    let base = folder("identity");
    write(&base.join("main/data.txt"), b"x");
    std::fs::create_dir_all(base.join("main/sub")).unwrap();
    let log = logged(
        &base,
        "var a = new File('data.txt'), b = new File('data.txt');\n\
         vc.log([a === b, a instanceof File, a instanceof Folder, File('sub') instanceof Folder, new File('sub') instanceof File,\n\
         Folder('data.txt') instanceof File, new Folder('data.txt') instanceof Folder, File('nope') instanceof File,\n\
         a.parent instanceof Folder, a.name, a.parent.fsName === Folder.current.fsName, String(a) === a.absoluteURI,\n\
         a.exists, new File('nope').exists, a.length, a.created instanceof Date, Folder('sub').exists, new File().exists]);",
    );
    assert_eq!(log, json!([[false, true, false, true, true, true, true, true, true, "data.txt", true, true, true, false, 1, true, true, false]]));
    let _ = std::fs::remove_dir_all(base);
}

/// `exists` is true only when the path names an entry of the object's own kind.
#[test]
fn script_file_exists_checks_the_kind() {
    let base = folder("kind");
    write(&base.join("main/data.txt"), b"x");
    std::fs::create_dir_all(base.join("main/sub")).unwrap();
    let log =
        logged(&base, "vc.log([new File('sub').exists, new Folder('data.txt').exists, new File('data.txt').exists, new Folder('sub').exists]);");
    assert_eq!(log, json!([[false, false, true, true]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A property a script puts on `Object.prototype` is not read as part of what the app returns.
#[test]
fn script_file_ignores_inherited_result_fields() {
    let base = folder("inherited");
    write(&base.join("main/data.txt"), b"data");
    let log = logged(
        &base,
        "Object.prototype.error = 'inherited'; Object.prototype.exists = true;\n\
         var f = new File('data.txt'); var opened = f.open('r');\n\
         vc.log([f.exists, new File('nope').exists, opened, f.error, f.read()]);",
    );
    assert_eq!(log, json!([[true, false, true, "", "data"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// `File.encode`, `File.decode`, `File.isEncodingAvailable` and the `Folder` locations are counted as
/// calls to the app.
#[test]
fn script_file_class_functions_are_counted() {
    let source = "File.encode('x'); File.decode('y'); Folder.isEncodingAvailable('UTF-8'); Folder.temp;";
    let r = Session::new().execute("script.run", &json!({"source": source})).unwrap();
    assert_eq!(r["hostCallCounts"]["fileCalls"], 4, "{r}");
}

#[test]
fn file_names_are_uri_encoded_and_current_relative() {
    let base = folder("names");
    std::fs::create_dir_all(base.join("main/sub")).unwrap();
    let log = logged(
        &base,
        "var f = new File('caf%C3%A9 x.txt');\n\
         var sep = File.fs === 'Windows' ? '\\\\' : '/';\n\
         vc.log([f.name, f.displayName, f.relativeURI, f.fsName === Folder.current.fsName + sep + 'caf\\u00e9 x.txt',\n\
         File.encode('\\u00e9 /'), File.decode('%C3%A9'), new File('sub/../sub/./y.txt').relativeURI,\n\
         new File('~/z').fsName === new Folder('~').fsName + sep + 'z', new File('a/b').path === new Folder('a').absoluteURI]);\n\
         Folder.current = new Folder('sub');\n\
         vc.log([new File('w').parent.name, new File('w').relativeURI, new File('../w').getRelativeURI(Folder.current.parent)]);\n\
         var l = new File('literal%2520name.txt'); Folder.current = new Folder('p%2541');\n\
         vc.log([l.displayName, l.name, l.absoluteURI.slice(-20), l.fsName === new File(l).fsName, Folder.current.displayName, new File('x').parent.displayName]);",
    );
    assert_eq!(
        log,
        json!([
            ["caf%C3%A9%20x.txt", "caf\u{e9} x.txt", "caf%C3%A9%20x.txt", true, "%C3%A9%20/", "\u{e9}", "sub/y.txt", true, true],
            ["sub", "w", "w"],
            ["literal%20name.txt", "literal%2520name.txt", "literal%2520name.txt", true, "p%41", "p%41"]
        ])
    );
    if cfg!(windows) {
        let log = logged(&base, "vc.log([new File('/c/a b/x.txt').fsName, new File('C:/a/x.txt').absoluteURI, new File('d:\\\\q').fsName]);");
        assert_eq!(log, json!([["C:\\a b\\x.txt", "/c/a/x.txt", "D:\\q"]]));
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_file_without_scope_cannot_probe_home() {
    let base = folder("home");
    let source = "var f = File('~/.ssh/id_test'); var g = new Folder('~');\n\
                  vc.log([f instanceof File, f.exists, f.open('r'), f.error, g.getFiles(), g.error, f.modified, f.length]);";
    let denied = json!([[true, false, false, "Script file access denied", null, "Script file access denied", null, 0]]);
    let before = script_probes();
    let r = Session::new().execute("script.run", &json!({"source": source})).unwrap();
    assert_eq!(r["log"], denied, "a script given as text reads nothing");
    assert_eq!(script_probes(), before, "nothing was looked up");
    // A script run from a file is held to its folder.
    let before = script_probes();
    assert_eq!(logged(&base, source), denied);
    assert_eq!(script_probes(), before, "the home folder lies outside the script's folder by its names alone");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_file_unc_rejected_before_lookup() {
    let base = folder("unc");
    let before = script_probes();
    let log = logged(
        &base,
        "var out = [];\n\
         var paths = ['\\\\\\\\host\\\\s\\\\x.txt', '//host/s/x.txt', '\\\\\\\\?\\\\C:\\\\x.txt', '\\\\\\\\.\\\\pipe\\\\x'];\n\
         for (var i = 0; i < paths.length; i++) { var f = new File(paths[i]); out.push(f.exists, f.open('r'), new Folder(paths[i]).getFiles()); }\n\
         vc.log(out);",
    );
    assert_eq!(log, json!([[false, false, null, false, false, null, false, false, null, false, false, null]]));
    if cfg!(windows) {
        assert_eq!(script_probes(), before, "no network or device path was looked up");
    } else {
        // Outside Windows the backslash forms are relative file names in the script's folder, and
        // `//host/s/x.txt` is an absolute path outside it.
        let before = script_probes();
        let log = logged(&base, "var f = new File('//host/s/x.txt'); vc.log([f.exists, f.open('r'), new Folder('//host/s/x.txt').getFiles()]);");
        assert_eq!(log, json!([[false, false, null]]));
        assert_eq!(script_probes(), before, "a path outside the script's folder was not looked up");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_file_link_escape_refused() {
    let base = folder("links");
    write(&base.join("outside/secret.txt"), b"secret");
    write(&base.join("main/own.txt"), b"own");
    std::fs::create_dir_all(base.join("main/inner")).unwrap();
    // A folder link: a symbolic link, or on Windows a junction, which needs no privilege.
    let link_folder = |target: &Path, link: &Path| {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap();
            assert!(made.status.success(), "mklink /J: {}", String::from_utf8_lossy(&made.stderr));
        }
    };
    link_folder(&base.join("outside"), &base.join("main").join("away"));
    link_folder(&base.join("main").join("inner"), &base.join("main").join("near"));
    let log = logged(
        &base,
        "var away = new Folder('away'); var s = new File('away/secret.txt');\n\
         vc.log([away.exists, away.getFiles(), away.error, away.resolve(), s.exists, s.open('r'), s.error,\n\
         Folder.current.getFiles().map(function (f) { return f.name; }), new Folder('near').alias, new Folder('near').resolve().name]);",
    );
    assert_eq!(log, json!([[false, null, DENIED, null, false, false, DENIED, ["inner", "main.jsx", "near", "own.txt"], true, "inner"]]));
    // A missing file behind the link is refused like an existing one, so nothing tells them apart.
    let log = logged(&base, "var n = new File('away/nope.txt'); vc.log([n.exists, n.open('r'), n.error]);");
    assert_eq!(log, json!([[false, false, DENIED]]));
    // A folder link to an outside folder that does not exist is refused the same way, so a script
    // cannot tell whether the folder it leads to exists.
    link_folder(&base.join("gone"), &base.join("main").join("dangling"));
    let log = logged(
        &base,
        "var d = new File('dangling/nope.txt'); var e = new File('dangling/a/b.txt');\n\
         vc.log([d.exists, d.open('r'), d.error, e.open('r'), e.error]);",
    );
    assert_eq!(log, json!([[false, false, DENIED, false, DENIED]]));
    let dangling = base.join("main").join("dangling");
    std::fs::remove_dir(&dangling).or_else(|_| std::fs::remove_file(&dangling)).unwrap();
    #[cfg(unix)]
    let made = |target: &Path, link: &Path| std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    let made = |target: &Path, link: &Path| std::os::windows::fs::symlink_file(target, link).is_ok();
    #[cfg(not(any(unix, windows)))]
    let made = |_: &Path, _: &Path| false;
    // Windows needs Developer Mode or elevation to make a link to a file.
    if !made(&base.join("outside/secret.txt"), &base.join("main/leak.txt")) {
        eprintln!("skipped: links to files can't be made here");
        let _ = std::fs::remove_dir_all(base);
        return;
    }
    assert!(made(&base.join("main/own.txt"), &base.join("main/alias.txt")));
    let log = logged(
        &base,
        "var leak = new File('leak.txt');\n\
         vc.log([leak.exists, leak.open('r'), leak.error, leak.resolve(), new File('away/secret.txt').exists, new Folder('away').getFiles(),\n\
         Folder.current.getFiles().map(function (f) { return f.name; }), new File('alias.txt').alias, new File('alias.txt').resolve().name]);",
    );
    assert_eq!(log, json!([[false, false, DENIED, null, false, null, ["alias.txt", "inner", "main.jsx", "near", "own.txt"], true, "own.txt"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A path whose names differ from the script's folder only in case is that folder where case does
/// not matter, and outside it where case matters, whether or not the file is there.
#[test]
fn script_file_case_variant_folder() {
    let base = crate::cmd::findfiles::plain(folder("case").canonicalize().unwrap());
    write(&base.join("main/own.txt"), b"own");
    let upper = base.join("MAIN");
    let source = |name: &str| {
        let p = format!("{:?}", upper.join(name).to_string_lossy());
        format!("var f = new File({p}); vc.log([f.exists, f.open('r'), f.error]);")
    };
    if upper.exists() {
        // Case does not matter here: `MAIN` is the script's folder.
        assert_eq!(logged(&base, &source("own.txt")), json!([[true, true, ""]]));
        assert_eq!(logged(&base, &source("nope.txt")), json!([[false, false, "Script file does not exist"]]));
    } else {
        let denied = json!([[false, false, DENIED]]);
        assert_eq!(logged(&base, &source("nope.txt")), denied, "MAIN does not exist");
        write(&upper.join("secret.txt"), b"secret");
        assert_eq!(logged(&base, &source("secret.txt")), denied);
        assert_eq!(logged(&base, &source("nope.txt")), denied, "MAIN exists beside main");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_file_oversize_refused_before_read() {
    let base = folder("oversize");
    let big = std::fs::File::create(base.join("main/big.bin")).unwrap();
    big.set_len(9 << 20).unwrap();
    drop(big);
    write(&base.join("main/small.txt"), b"ok");
    let r =
        run(&base, "var f = new File('big.bin'); var s = new File('small.txt'); vc.log([f.length, f.open('r'), f.error, s.open('r'), s.read()]);");
    assert_eq!(r["log"], json!([[9437184, false, "Script file exceeds 8 MiB", true, "ok"]]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_files_read_at_most_32_mib_in_a_run() {
    let base = folder("total");
    write(&base.join("main/seven.bin"), &vec![b'a'; 7 << 20]);
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    write(&main, b"var f = new File('seven.bin'); for (var i = 0; i < 5; i++) { f.open('r'); vc.log(i); }");
    let e = s.execute("script.run", &json!({"path": main.to_string_lossy()}));
    assert!(e.is_err(), "the fifth open passes 32 MiB");
    let report = s.script_report().unwrap();
    assert_eq!(report["log"], json!([0, 1, 2, 3]));
    assert!(report["message"].as_str().unwrap().contains("32 MiB"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_files_have_at_most_256_open() {
    let base = folder("open");
    write(&base.join("main/empty.txt"), b"");
    let log = logged(
        &base,
        "var all = [], fails = 0, error = '';\n\
         for (var i = 0; i < 300; i++) { var f = new File('empty.txt'); if (f.open('r')) all.push(f); else { fails++; error = f.error; } }\n\
         var closed = all[0].close(); var again = new File('empty.txt');\n\
         vc.log([all.length, fails, error, closed, again.open('r'), all[1].open('r'), new File('empty.txt').open('r')]);",
    );
    assert_eq!(log, json!([[256, 44, "Script has 256 files open", true, true, true, false]]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_folder_getfiles_stays_inside_scope() {
    let base = folder("list");
    for name in ["a.jsx", "b.txt", "C.JSX"] {
        write(&base.join("main").join(name), b"");
    }
    std::fs::create_dir_all(base.join("main/sub")).unwrap();
    write(&base.join("outside.jsx"), b"");
    let log = logged(
        &base,
        "var d = Folder.current; var names = function (l) { var o = []; for (var i = 0; i < l.length; i++) o.push(l[i].name); return o; };\n\
         vc.log(names(d.getFiles('*.jsx')));\n\
         vc.log(names(d.getFiles(function (f) { return f instanceof Folder; })));\n\
         var up = d.parent; vc.log([names(d.getFiles('?.txt')), d.getFiles().length, up.getFiles(), up.error]);\n\
         try { d.getFiles(function () { throw new Error('stop'); }); } catch (e) { vc.log(e.message); }\n\
         vc.log([new Folder('nope').getFiles(), d.getFiles('*.jsx')[0] instanceof File]);",
    );
    assert_eq!(log, json!([["C.JSX", "a.jsx", "main.jsx"], ["sub"], [["b.txt"], 5, null, DENIED], "stop", [null, true]]));
    let many = base.join("main/many");
    for i in 0..=crate::script::files::MAX_ENTRIES {
        write(&many.join(format!("{i:04}.txt")), b"");
    }
    let log = logged(&base, "var m = new Folder('many'); vc.log([m.getFiles(), m.error, m.getFiles('0001.*').length]);");
    assert_eq!(log, json!([[null, "Script folder has more than 2,048 entries", 1]]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_file_bom_and_line_read() {
    let base = folder("lines");
    write(&base.join("main/lines.txt"), b"\xEF\xBB\xBFone\r\ntwo\n\rthree\rfour\nfin");
    write(&base.join("main/e.txt"), "\u{e9}t\u{e9}".as_bytes());
    let log = logged(
        &base,
        "var f = new File('lines.txt'); var out = [f.eof, f.open('r'), f.encoding, f.tell()];\n\
         var lines = []; while (!f.eof) lines.push(f.readln()); out.push(lines, f.tell());\n\
         out.push(f.seek(3), f.read(3), f.seek(-3, 2), f.read(), f.seek(1, 1), f.seek(100), f.seek(-1), f.tell());\n\
         f.seek(6); out.push(f.readch(), f.readch(), f.tell(), f.close(), f.read(), f.error, f.eof);\n\
         var e = new File('e.txt'); e.open(); out.push(e.read(1), e.tell(), e.read(5), e.tell());\n\
         vc.log(out);",
    );
    assert_eq!(
        log,
        json!([[
            true,
            true,
            "UTF-8",
            3,
            ["one", "two", "three", "four", "fin"],
            27,
            true,
            "one",
            true,
            "fin",
            false,
            false,
            false,
            27,
            "\n",
            "t",
            9,
            true,
            "",
            "Script file is not open",
            true,
            "\u{e9}",
            2,
            "t\u{e9}",
            5
        ]])
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_file_encoding_binary_ascii() {
    let base = folder("encodings");
    write(&base.join("main/bytes.bin"), &[0xE8, 0x41]);
    write(&base.join("main/euro.txt"), &[0x80]);
    let log = logged(
        &base,
        "var b = new File('bytes.bin'); b.encoding = 'BINARY'; b.open('r'); var t = b.read();\n\
         var a = new File('bytes.bin'); a.encoding = 'ASCII'; a.open('r');\n\
         var out = [t.charCodeAt(0), t.charAt(1), b.encoding, a.read(), a.error, a.tell()];\n\
         a.encoding = 'binary'; out.push(a.encoding, a.read().length);\n\
         var c = new File('euro.txt'); c.open('r'); out.push(c.encoding, c.read().charCodeAt(0));\n\
         var k = new File('bytes.bin'); k.encoding = 'KLINGON'; out.push(k.open('r'), k.error);\n\
         out.push(File.isEncodingAvailable('utf-8'), Folder.isEncodingAvailable('BINARY'), File.isEncodingAvailable('KLINGON'));\n\
         vc.log(out);\n\
         var m = new File('bytes.bin'); m.encoding = 'BINARY'; m.open('r'); m.encoding = 'KLINGON'; vc.log([m.error, m.encoding, m.read().length]);\n\
         var r = new File('bytes.bin'); r.open('r'); r.encoding = 'ASCII'; r.close(); vc.log([r.encoding, r.open('r'), r.read(), r.error]);",
    );
    let undecodable = "Script file cannot be decoded";
    assert_eq!(log[0], json!([232, "A", "BINARY", "", undecodable, 0, "BINARY", 2, "CP1252", 0x20AC, false, undecodable, true, true, false]));
    assert_eq!(log[1], json!([undecodable, "BINARY", 2]), "a name that is not an encoding leaves the open file's encoding");
    assert_eq!(log[2], json!(["ASCII", true, "", undecodable]), "an encoding set while open is kept after close");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_file_writes_dialogs_and_execute_are_not_available() {
    let base = folder("writes");
    write(&base.join("main/keep.txt"), b"keep");
    let r = run(
        &base,
        "var f = new File('keep.txt'); var n = new File('new.txt');\n\
         var out = [n.open('w'), n.error, f.open('a'), f.open('e'), f.open('x'), f.error, f.write('x'), f.remove(), f.copy('c.txt'), f.rename('r.txt')];\n\
         out.push(new Folder('made').create(), f.openDlg(), f.error, File.openDialog(), Folder.selectDialog(), f.execute(), f.error);\n\
         f.hidden = true; out.push(f.error, f.hidden); f.error = ''; out.push(f.error);\n\
         vc.log(out);",
    );
    assert_eq!(
        r["log"],
        json!([[
            false,
            "Script file writing is not available",
            false,
            false,
            false,
            "Script file open mode must be r, w, e or a",
            false,
            false,
            false,
            false,
            false,
            null,
            "Script file dialogs are not available",
            null,
            null,
            false,
            "Script files cannot be opened in another application",
            "Script file writing is not available",
            false,
            ""
        ]])
    );
    assert_eq!(r["hostCallCounts"]["fileExecutes"], 1);
    assert!(r["hostCallCounts"]["fileCalls"].as_u64().unwrap() > 0);
    assert_eq!(std::fs::read(base.join("main/keep.txt")).unwrap(), b"keep");
    assert!(!base.join("main/new.txt").exists() && !base.join("main/made").exists() && !base.join("main/c.txt").exists());
    let _ = std::fs::remove_dir_all(base);
}

/// A run with automation roots in force reads inside its folder and the read roots, and a run given
/// as text reads nothing even where the roots allow it.
#[test]
fn script_files_read_inside_the_automation_read_roots() {
    let base = folder("roots");
    write(&base.join("main/data.txt"), b"data");
    let source = "var f = new File('data.txt'); vc.log([f.exists, f.open('r'), f.error, f.read()]);";
    let roots = AutomationRoots::new(&[&base.join("main")], &[]).unwrap();
    assert_eq!(confine(roots.as_ref(), || logged(&base, source)), json!([[true, true, "", "data"]]));
    let text = format!("var f = new File({:?}); vc.log([f.exists, f.error]);", base.join("main/data.txt").to_string_lossy());
    let r = confine(roots.as_ref(), || Session::new().execute("script.run", &json!({"source": text})).unwrap());
    assert_eq!(r["log"], json!([[false, "Script file access denied"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A path longer than 32,767 characters is refused before it is split into names.
#[test]
fn script_file_path_length_is_bounded() {
    let base = folder("longpath");
    let log = logged(
        &base,
        "var s = 'a/'; while (s.length < 40000) s += s;\n\
         var out = []; try { new File(s); } catch (e) { out.push(e.name, e.message); }\n\
         try { new Folder('x').getRelativeURI(s); } catch (e) { out.push(e.name); }\n\
         out.push(new File(s.slice(0, 30000)).exists); vc.log(out);",
    );
    assert_eq!(log, json!([["RangeError", "Script file path exceeds 32,767 characters", "RangeError", false]]));
    let _ = std::fs::remove_dir_all(base);
}

/// An error `File` or `Folder` throws is reported at the script's line that called it.
#[test]
fn script_file_errors_are_at_the_calling_line() {
    let base = folder("errors");
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    write(&main, b"\n\n\nnew File('x').read(-1);");
    assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err());
    let report = s.script_report().unwrap();
    assert!(report["message"].as_str().unwrap().contains("RangeError: File.read: the count must be a finite number"), "{report}");
    assert_eq!(report["line"].as_u64(), Some(4), "{report}");
    let _ = std::fs::remove_dir_all(base);
}
