//! `File` and `Folder` in a script: their classes and names, reading inside the script's own folder,
//! the paths refused before anything is looked up, the read limits, writing in the run's temporary
//! folder, the script's data folder and what a person allowed in a file dialog, and the grants kept
//! for a script.

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
    for i in 0..5 {
        write(&base.join(format!("main/seven{i}.bin")), &vec![b'a'; 7 << 20]);
    }
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    write(&main, b"for (var i = 0; i < 5; i++) { new File('seven' + i + '.bin').open('r'); vc.log(i); }");
    let e = s.execute("script.run", &json!({"path": main.to_string_lossy()}));
    assert!(e.is_err(), "the fifth open passes 32 MiB");
    let report = s.script_report().unwrap();
    assert_eq!(report["log"], json!([0, 1, 2, 3]));
    assert!(report["message"].as_str().unwrap().contains("32 MiB"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

/// A file opened again once its objects closed counts only once toward the 32 MiB a run reads, and
/// a file open in two objects at once counts for each.
#[test]
fn script_file_opened_again_counts_once_toward_the_run_limit() {
    let base = folder("reopencount");
    write(&base.join("main/seven.bin"), &vec![b'a'; 7 << 20]);
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    write(&main, b"var f = new File('seven.bin'); for (var i = 0; i < 8; i++) { f.open('r'); f.close(); } vc.log('again');");
    let r = s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap();
    assert_eq!(r["log"], json!(["again"]));
    write(&main, b"for (var i = 0; i < 5; i++) { new File('seven.bin').open('r'); vc.log(i); }");
    assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err(), "five objects hold five copies");
    assert_eq!(s.script_report().unwrap()["log"], json!([0, 1, 2, 3]));
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
    // Where case does not matter to the file system, the mask matches without regard to case.
    let jsx = if cfg!(any(windows, target_os = "macos")) { json!(["C.JSX", "a.jsx", "main.jsx"]) } else { json!(["a.jsx", "main.jsx"]) };
    assert_eq!(log, json!([jsx, ["sub"], [["b.txt"], 5, null, DENIED], "stop", [null, true]]));
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

/// A script's own folder is only read: writing there, and anywhere it was not given, is refused,
/// and with nobody to ask its file dialogs give `null`. Aliases and `execute` are not available.
#[test]
fn script_file_writes_outside_its_places_and_headless_dialogs_are_refused() {
    let base = folder("writes");
    write(&base.join("main/keep.txt"), b"keep");
    let outside = base.join("outside.txt").to_string_lossy().into_owned();
    let r = run(
        &base,
        &format!(
            "var f = new File('keep.txt'); var n = new File('new.txt'); var o = new File({outside:?});\n\
             var out = [n.open('w'), n.error, f.open('a'), f.open('e'), f.open('x'), f.error, f.remove(), f.copy('c.txt'), f.rename('r.txt'), o.open('w')];\n\
             out.push(new Folder('made').create(), f.openDlg(), f.error, File.openDialog(), Folder.selectDialog(), f.execute(), f.error);\n\
             f.hidden = true; out.push(f.error, f.hidden, f.createAlias('l'), f.error); f.error = ''; out.push(f.error);\n\
             vc.log(out);"
        ),
    );
    assert_eq!(
        r["log"],
        json!([[
            false,
            DENIED,
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
            "Script file dialogs need a person to answer them",
            null,
            null,
            false,
            "Script files cannot be opened in another application",
            "Script file hidden setting cannot be changed",
            false,
            false,
            "Script files cannot make aliases",
            ""
        ]])
    );
    assert_eq!(r["hostCallCounts"]["fileExecutes"], 1);
    assert_eq!(r["written"], json!([]));
    assert_eq!(std::fs::read(base.join("main/keep.txt")).unwrap(), b"keep");
    assert!(!base.join("main/new.txt").exists() && !base.join("main/made").exists() && !base.join("main/c.txt").exists());
    assert!(!Path::new(&outside).exists());
    let _ = std::fs::remove_dir_all(base);
}

/// A session whose scripts keep their data folders in `base/settings/Script Data`.
fn with_data(base: &Path) -> Session {
    let mut s = Session::new();
    s.attach_script_data(base.join("settings/Script Data"));
    s
}

/// Runs `main/main.jsx` in `base` with the text `text` in `s`.
fn run_in(s: &mut Session, base: &Path, text: &str) -> Result<Value> {
    let main = base.join("main/main.jsx");
    write(&main, text.as_bytes());
    s.execute("script.run", &json!({"path": main.to_string_lossy()}))
}

/// `Folder.userData` is the script's own folder, kept between runs, and `Folder.temp` the run's,
/// deleted when it ends. `write` joins its values, `writeln` adds the `lineFeed`'s line end, and a
/// byte order mark is written as the script writes it.
#[test]
fn script_files_write_in_the_temporary_and_data_folders() {
    let base = folder("userdata");
    let mut s = with_data(&base);
    let r = run_in(
        &mut s,
        &base,
        "var out = [], u = new File(Folder.userData.fsName + '/notes.txt');\n\
         out.push(u.open('w'), u.write('a', 1, true), u.writeln('b'), u.close());\n\
         u.lineFeed = 'Windows'; out.push(u.open('a'), u.writeln('c'), u.close());\n\
         u.open('r'); out.push(u.read()); u.close();\n\
         var t = new File(Folder.temp.fsName + '/t.txt'); out.push(t.open('w'), t.write('\\uFEFFx'), t.close(), t.length);\n\
         var m = new File(); out.push(m.open('w'), m.parent.fsName === Folder.temp.fsName);\n\
         vc.log(out); vc.log(u.fsName); vc.log(Folder.temp.fsName);",
    )
    .unwrap();
    assert_eq!(r["log"][0], json!([true, true, true, true, true, true, true, "a1trueb\nc\r\n", true, true, true, 4, true, true]));
    let notes = PathBuf::from(r["log"][1].as_str().unwrap());
    let temp = PathBuf::from(r["log"][2].as_str().unwrap());
    assert_eq!(std::fs::read(&notes).unwrap(), b"a1trueb\nc\r\n");
    assert!(notes.starts_with(std::fs::canonicalize(base.join("settings/Script Data")).unwrap()), "{notes:?}");
    assert!(!temp.exists(), "the run's temporary folder is deleted when it ends");
    let written: Vec<&str> = r["written"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(written.contains(&notes.to_string_lossy().as_ref()), "{written:?}");
    let again = run_in(&mut s, &base, "var u = new File(Folder.userData.fsName + '/notes.txt'); u.open('r'); vc.log(u.read());").unwrap();
    assert_eq!(again["log"], json!(["a1trueb\nc\r\n"]), "a script's data folder is kept between runs");
    let _ = std::fs::remove_dir_all(base);
}

/// `copy` reads its source under the rules for reading: a source that a link to a file outside the
/// script's places replaced after it was looked up is refused, and nothing is copied.
#[cfg(unix)]
#[test]
fn a_copy_whose_source_becomes_a_link_outside_copies_nothing() {
    let base = folder("copy-link");
    let mut s = with_data(&base);
    let secret = base.join("secret.txt");
    write(&secret, b"secret");
    crate::script::files::BEFORE_COPY_READ.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_: &Path, source: &Path| {
            std::fs::remove_file(source).unwrap();
            std::os::unix::fs::symlink(&secret, source).unwrap();
        }));
    });
    let r = run_in(
        &mut s,
        &base,
        "var d = Folder.userData.fsName, a = new File(d + '/a.txt'); a.open('w'); a.write('mine'); a.close();\n\
         vc.log([a.copy(d + '/b.txt'), a.error, new File(d + '/b.txt').exists]);",
    )
    .unwrap();
    crate::script::files::BEFORE_COPY_READ.with(|h| h.borrow_mut().take());
    assert_eq!(r["log"], json!([[false, "Script file access denied", false]]));
    let _ = std::fs::remove_dir_all(base);
}

/// `copy` of a link whose target changes after it was looked up is refused, also when both files
/// are in the script's places.
#[cfg(unix)]
#[test]
fn a_copy_of_a_link_that_leads_elsewhere_after_it_was_looked_up_copies_nothing() {
    let base = folder("copy-relink");
    let mut s = with_data(&base);
    let first = run_in(
        &mut s,
        &base,
        "var d = Folder.userData.fsName;\n\
         ['a', 'c'].forEach(function (n) { var f = new File(d + '/' + n + '.txt'); f.open('w'); f.write(n); f.close(); });\n\
         vc.log(d);",
    )
    .unwrap();
    let data = PathBuf::from(first["log"][0].as_str().unwrap());
    std::os::unix::fs::symlink(data.join("a.txt"), data.join("link.txt")).unwrap();
    let other = data.join("c.txt");
    crate::script::files::BEFORE_COPY_READ.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |given: &Path, _: &Path| {
            std::fs::remove_file(given).unwrap();
            std::os::unix::fs::symlink(&other, given).unwrap();
        }));
    });
    let r = run_in(
        &mut s,
        &base,
        "var d = Folder.userData.fsName, l = new File(d + '/link.txt');\n\
         vc.log([l.copy(d + '/b.txt'), l.error, new File(d + '/b.txt').exists]);",
    )
    .unwrap();
    crate::script::files::BEFORE_COPY_READ.with(|h| h.borrow_mut().take());
    assert_eq!(r["log"], json!([[false, "Script file access denied", false]]));
    let _ = std::fs::remove_dir_all(base);
}

/// The grants file is written through a temporary file of its own, so a temporary file another
/// process left beside it, or one an earlier write left under the name this write takes first,
/// does not stop a change and is left where it is.
#[test]
fn a_grants_change_does_not_use_another_writers_temporary_file() {
    let base = folder("grants-part");
    let data = base.join("settings/Script Data");
    let grant = json!({"script": "/s.jsx", "sha256": "0", "code": [], "path": "/p", "write": false, "folder": false});
    write(&base.join("settings/Script Grants.json"), json!({"version": 1, "grants": [grant]}).to_string().as_bytes());
    std::fs::create_dir_all(base.join("settings/Script Grants.json.part/held")).unwrap();
    let next = crate::script::files::NEXT_PART.load(std::sync::atomic::Ordering::Relaxed);
    let left_behind: Vec<PathBuf> =
        (next..next + 8).map(|n| base.join(format!("settings/Script Grants.json.{}-{n}.part", std::process::id()))).collect();
    for p in &left_behind {
        write(p, b"earlier");
    }
    assert_eq!(crate::script::files::revoke_grants(&data, None, None).unwrap(), 1);
    let left: Value = serde_json::from_slice(&std::fs::read(base.join("settings/Script Grants.json")).unwrap()).unwrap();
    assert_eq!(left["grants"], json!([]));
    assert!(left_behind.iter().all(|p| std::fs::read(p).is_ok_and(|b| b == b"earlier")), "files this write did not make stay");
    let _ = std::fs::remove_dir_all(base);
}

/// A change to the grants file is not made when its lock cannot be taken.
#[test]
fn a_grants_change_without_its_lock_changes_nothing() {
    let base = folder("grants-lock");
    let data = base.join("settings/Script Data");
    let grant = json!({"script": "/s.jsx", "sha256": "0", "code": [], "path": "/p", "write": false, "folder": false});
    let text = json!({"version": 1, "grants": [grant]}).to_string();
    write(&base.join("settings/Script Grants.json"), text.as_bytes());
    std::fs::create_dir_all(base.join("settings/Script Grants.lock/held")).unwrap();
    assert!(crate::script::files::revoke_grants(&data, None, None).is_err());
    assert_eq!(std::fs::read_to_string(base.join("settings/Script Grants.json")).unwrap(), text);
    let _ = std::fs::remove_dir_all(base);
}

/// `rename` of a file an earlier run wrote names both its old and its new path in `written`.
#[test]
fn a_renamed_file_names_its_old_and_new_paths_in_written() {
    let base = folder("renamed");
    let mut s = with_data(&base);
    let first = run_in(&mut s, &base, "var u = new File(Folder.userData.fsName + '/old.txt'); u.open('w'); u.write('x'); vc.log(u.close());");
    assert_eq!(first.unwrap()["log"], json!([true]));
    let r = run_in(
        &mut s,
        &base,
        "var u = new File(Folder.userData.fsName + '/old.txt'); var before = u.fsName;\n\
         vc.log(u.rename('new.txt')); vc.log(before); vc.log(u.fsName);",
    )
    .unwrap();
    assert_eq!(r["log"][0], json!(true));
    let written: Vec<&str> = r["written"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    for at in [1, 2] {
        let path = r["log"][at].as_str().unwrap();
        assert!(written.contains(&path), "{path} in {written:?}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// The open modes, the encodings that cannot hold a character, `copy` over a file, `remove` of a
/// folder only when it is empty, `rename` refusing a name in use, `create`, and setting `length` and
/// `readonly`.
#[test]
fn script_file_modes_encodings_and_changes() {
    let base = folder("changes");
    let r = run(
        &base,
        "var d = Folder.temp.fsName, out = [];\n\
         var e = new File(d + '/missing.txt'); out.push(e.open('e'), e.error);\n\
         var a = new File(d + '/a.txt'); a.encoding = 'ASCII'; out.push(a.open('w'), a.write('caf\\u00e9'), a.error, a.write('cafe'), a.close(), a.length);\n\
         var b = new File(d + '/b.bin'); b.encoding = 'BINARY'; b.open('w'); b.write('\\u0141B'); b.close();\n\
         b.encoding = 'BINARY'; b.open('r'); out.push(b.read()); b.close();\n\
         out.push(a.copy(d + '/b.bin'), new File(d + '/b.bin').length);\n\
         var f = new Folder(d + '/x/y'); out.push(f.create(), f.exists);\n\
         var x = new Folder(d + '/x'); out.push(x.remove(), x.error, f.remove(), f.exists);\n\
         out.push(a.rename('b.bin'), a.error, a.rename('c.txt'), a.name, a.exists, new File(d + '/a.txt').exists);\n\
         a.length = 2; out.push(a.length); a.length = 6; out.push(a.length);\n\
         a.readonly = true; out.push(a.readonly); a.readonly = false; out.push(a.readonly);\n\
         out.push(a.remove(), a.exists);\n\
         vc.log(out);",
    );
    assert_eq!(
        r["log"],
        json!([[
            false,
            "Script file does not exist",
            true,
            false,
            "Script file cannot encode the text",
            true,
            true,
            4,
            "AB",
            true,
            4,
            true,
            true,
            false,
            "Script folder is not empty",
            true,
            false,
            false,
            "Script file name is already in use",
            true,
            "c.txt",
            true,
            false,
            2,
            6,
            true,
            false,
            true,
            false
        ]])
    );
    let _ = std::fs::remove_dir_all(base);
}

/// Setting `length` while the file is open changes what the open file holds, so closing it keeps
/// the new length.
#[test]
fn script_file_length_changes_reach_an_open_file() {
    let base = folder("openlength");
    let r = run(
        &base,
        "var o = new File(Folder.temp.fsName + '/l.txt'); o.open('w'); o.write('abc');\n\
         new File(o.fsName).length = 1; o.write('d'); o.close();\n\
         o.open('r'); vc.log([o.read(), o.length]); o.close();",
    );
    assert_eq!(r["log"], json!([["ad", 2]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A file is kept as it was when `open('w')` is given an encoding that is not one and when it is
/// copied onto itself, and the changes an open file holds stay to be written when writing them fails.
#[test]
fn script_file_writes_keep_what_they_cannot_change() {
    let base = folder("keeps");
    let r = run(
        &base,
        "var d = Folder.temp.fsName, out = [];\n\
         var k = new File(d + '/k.txt'); k.open('w'); k.write('keep'); k.close();\n\
         k.encoding = 'nope'; out.push(k.open('w'), k.error);\n\
         k.encoding = 'UTF-8'; k.open('r'); out.push(k.read()); k.close();\n\
         out.push(k.copy(k.fsName), k.length);\n\
         var w = new File(d + '/w.txt'); w.open('w'); w.write('abc'); new File(w.fsName).readonly = true;\n\
         var q = new File(w.fsName); q.length = 1; out.push(q.error);\n\
         new File(w.fsName).readonly = false; out.push(w.close()); w.open('r'); out.push(w.read()); w.close();\n\
         vc.log(out);",
    );
    assert_eq!(r["log"], json!([[false, "Script file cannot be decoded", "keep", true, 4, "Script file could not be written", true, "abc"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A file whose `close` could not write it stays to be written when the run ends.
#[test]
fn script_file_close_that_cannot_write_keeps_the_changes() {
    let base = folder("closefail");
    let mut s = with_data(&base);
    let r = run_in(
        &mut s,
        &base,
        "var u = new File(Folder.userData.fsName + '/c.txt'); u.open('w'); u.write('kept');\n\
         new File(u.fsName).readonly = true; var out = [u.close(), u.error]; new File(u.fsName).readonly = false;\n\
         vc.log(out); vc.log(u.fsName);",
    )
    .unwrap();
    assert_eq!(r["log"][0], json!([false, "Script file could not be written"]));
    assert_eq!(std::fs::read(r["log"][1].as_str().unwrap()).unwrap(), b"kept");
    let _ = std::fs::remove_dir_all(base);
}

/// `changePath` on an object whose changes could not be written gives false with `error` set, and
/// the object keeps its path.
#[test]
fn script_file_change_path_that_cannot_write_keeps_the_path() {
    let base = folder("changepathfail");
    let mut s = with_data(&base);
    let r = run_in(
        &mut s,
        &base,
        "var u = new File(Folder.userData.fsName + '/p.txt'); u.open('w'); u.write('kept'); var was = u.fsName;\n\
         new File(was).readonly = true; var out = [u.changePath(Folder.userData.fsName + '/q.txt'), u.error, u.fsName == was];\n\
         new File(was).readonly = false; vc.log(out);",
    )
    .unwrap();
    assert_eq!(r["log"][0], json!([false, "Script file could not be written", true]));
    let _ = std::fs::remove_dir_all(base);
}

/// A file opened again after a `close` that could not write it keeps what was written after it was
/// opened again, and the earlier changes do not replace it when the run ends.
#[test]
fn script_file_opened_again_after_a_failed_close_keeps_the_later_text() {
    let base = folder("reopen");
    let mut s = with_data(&base);
    let r = run_in(
        &mut s,
        &base,
        "var u = new File(Folder.userData.fsName + '/r.txt'); u.open('w'); u.write('old');\n\
         new File(u.fsName).readonly = true; var out = [u.close()]; new File(u.fsName).readonly = false;\n\
         out.push(u.open('w'), u.write('new'), u.close()); vc.log(out); vc.log(u.fsName);",
    )
    .unwrap();
    assert_eq!(r["log"][0], json!([false, true, true, true]));
    assert_eq!(std::fs::read(r["log"][1].as_str().unwrap()).unwrap(), b"new");
    let _ = std::fs::remove_dir_all(base);
}

/// A write that would pass the run's 32 MiB of writing ends the run, and its text is not written.
#[test]
fn script_file_write_past_the_run_limit_is_not_written() {
    let base = folder("runlimit");
    let mut s = with_data(&base);
    let r = run_in(
        &mut s,
        &base,
        "var t = 'x'; while (t.length < 4194304) t += t;\n\
         var f = new File(Folder.userData.fsName + '/f.bin'); f.encoding = 'UTF-16';\n\
         for (var k = 0; k < 4; k++) { f.open('w'); f.write(t); f.close(); }\n\
         var g = new File(Folder.userData.fsName + '/g.txt'); vc.log(g.fsName); g.open('w'); g.write('over');",
    );
    assert!(r.is_err());
    let report = s.script_report().unwrap();
    assert!(report["message"].as_str().unwrap().contains("32 MiB"), "{report}");
    assert_eq!(std::fs::read(report["log"][0].as_str().unwrap()).unwrap(), b"", "{report}");
    let _ = std::fs::remove_dir_all(base);
}

/// `remove` and `rename` of a file that is open elsewhere: a removed file is not written again when
/// the run ends, a renamed one is written under its new name, and an object whose own changes could
/// not be written does not remove or rename its file.
#[test]
fn script_file_remove_and_rename_of_an_open_file() {
    let base = folder("openmove");
    let mut s = with_data(&base);
    let r = run_in(
        &mut s,
        &base,
        "var d = Folder.userData.fsName, out = [];\n\
         var a = new File(d + '/a.txt'); a.open('w'); a.write('a'); out.push(new File(a.fsName).remove());\n\
         var b = new File(d + '/b.txt'); b.open('w'); b.write('b'); var c = new File(b.fsName); out.push(c.rename('c.txt'), b.close());\n\
         var e = new File(d + '/e.txt'); e.open('w'); e.write('e'); new File(e.fsName).readonly = true;\n\
         out.push(e.remove(), e.error, e.exists); new File(e.fsName).readonly = false;\n\
         vc.log(out); vc.log(d);",
    )
    .unwrap();
    assert_eq!(r["log"][0], json!([true, true, true, false, "Script file could not be written", true]));
    let d = PathBuf::from(r["log"][1].as_str().unwrap());
    assert!(!d.join("a.txt").exists() && !d.join("b.txt").exists());
    assert_eq!(std::fs::read(d.join("c.txt")).unwrap(), b"b");
    assert_eq!(std::fs::read(d.join("e.txt")).unwrap(), b"e", "the run's end writes the changes the failed close kept");
    let _ = std::fs::remove_dir_all(base);
}

/// `copy` of a file that is open copies what the open file holds, and an object that has the target
/// open then holds the copy.
#[test]
fn script_file_copy_of_an_open_file_has_its_changes() {
    let base = folder("copyopen");
    let r = run(
        &base,
        "var d = Folder.temp.fsName, f = new File(d + '/f.txt'); f.open('w'); f.write('new');\n\
         var out = [f.copy(d + '/g.txt')]; var g = new File(d + '/g.txt'); g.open('r'); out.push(g.read()); g.close(); vc.log(out);",
    );
    assert_eq!(r["log"], json!([[true, "new"]]));
    let r = run(
        &base,
        "var d = Folder.temp.fsName, s = new File(d + '/s.txt'); s.open('w'); s.write('copied'); s.close();\n\
         var t = new File(d + '/t.txt'); t.open('w'); t.write('old'); var out = [s.copy(t.fsName), t.close()];\n\
         t.open('r'); out.push(t.read()); t.close(); vc.log(out);",
    );
    assert_eq!(r["log"], json!([[true, true, "copied"]]), "an open target holds the copy");
    let _ = std::fs::remove_dir_all(base);
}

/// A file whose `close` could not write it stays open, so a later `close` writes it.
#[test]
fn script_file_close_that_fails_can_be_tried_again() {
    let base = folder("closeagain");
    let r = run(
        &base,
        "var d = Folder.temp.fsName, f = new File(d + '/f.txt'); f.open('w'); f.write('kept');\n\
         new File(f.fsName).readonly = true; var out = [f.close(), f.error];\n\
         new File(f.fsName).readonly = false; out.push(f.close());\n\
         var g = new File(f.fsName); g.open('r'); out.push(g.read()); g.close(); vc.log(out);",
    );
    assert_eq!(r["log"], json!([[false, "Script file could not be written", true, "kept"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// `copy` gives the copy the source's permissions: the copy of a read-only file is read-only.
#[test]
fn script_file_copy_keeps_the_source_read_only() {
    let base = folder("copyreadonly");
    let r = run(
        &base,
        "var d = Folder.temp.fsName, s = new File(d + '/s.txt'); s.open('w'); s.write('x'); s.close(); s.readonly = true;\n\
         var out = [s.copy(d + '/t.txt'), new File(d + '/t.txt').readonly, new File(d + '/t.txt').length];\n\
         s.readonly = false; new File(d + '/t.txt').readonly = false; vc.log(out);",
    );
    assert_eq!(r["log"], json!([[true, true, 1]]));
    let _ = std::fs::remove_dir_all(base);
}

/// `write` with several values joins them before making them text, so a surrogate pair split
/// between two values is written as one character.
#[test]
fn script_file_write_joins_a_surrogate_pair_split_between_values() {
    let base = folder("surrogates");
    let r = run(
        &base,
        "var f = new File(Folder.temp.fsName + '/s.txt'); f.encoding = 'UTF-8'; f.open('w'); f.write('\\uD83D', '\\uDE00'); f.close();\n\
         f.open('r'); var t = f.read(); f.close(); vc.log([t === '\\uD83D\\uDE00', t.length]);",
    );
    assert_eq!(r["log"], json!([[true, 2]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A grant that would make the grants file larger than it may be read is not kept, and the grants
/// file keeps the grants it had.
#[test]
fn script_file_grants_file_stays_within_its_size_limit() {
    let base = folder("bigstore");
    let out = base.join("picked/out.txt");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let store = base.join("settings/Script Grants.json");
    let grants =
        |path: String| json!({"version": 1, "grants": [{"script": "x", "sha256": "y", "code": [], "path": path, "write": false, "folder": false}]});
    let empty = serde_json::to_vec_pretty(&grants(String::new())).unwrap().len();
    let full = serde_json::to_vec_pretty(&grants("p".repeat((4 << 20) - 64 - empty))).unwrap();
    write(&store, &full);
    let mut s = with_data(&base);
    assert_eq!(s.execute("script.grants", &json!({})).unwrap()["grants"].as_array().unwrap().len(), 1);
    start_framed(&mut s, &base, "var f = File.saveDialog('p'); vc.log(f === null ? null : f.fsName);");
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
    let _ = pump_to_picker(&mut s).unwrap();
    assert_eq!(std::fs::read(&store).unwrap(), full, "the grants file is left as it was");
    assert_eq!(s.execute("script.grants", &json!({})).unwrap()["grants"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(base);
}

/// A grants file that is there but cannot be read (another version, not JSON, a grant that is not
/// well formed, or no list of grants) is not written over: a grant picked with Always is not kept, and a revoke fails, so the grants it holds stay.
#[test]
fn script_file_grants_file_that_cannot_be_read_is_left_as_it_is() {
    let cases = [
        ("newer", br#"{"version": 2, "grants": [{"script": "x"}]}"#.as_slice()),
        ("broken", b"{ not json".as_slice()),
        ("malformed", br#"{"version": 1, "grants": [{"script": "x"}]}"#.as_slice()),
        ("nolist", br#"{"version": 1, "grants": {}}"#.as_slice()),
    ];
    for (name, held) in cases {
        let base = folder(&format!("store-{name}"));
        let out = base.join("picked/out.txt");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        let store = base.join("settings/Script Grants.json");
        write(&store, held);
        let mut s = with_data(&base);
        start_framed(&mut s, &base, "var f = File.saveDialog('p'); vc.log(f === null ? null : f.fsName);");
        assert!(pump_to_picker(&mut s).is_none());
        assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
        let _ = pump_to_picker(&mut s).unwrap();
        assert_eq!(std::fs::read(&store).unwrap(), held, "{name}: the grants file is left as it was");
        assert!(s.execute("script.grants.revoke", &json!({"all": true})).is_err(), "{name}");
        assert_eq!(std::fs::read(&store).unwrap(), held, "{name}: a revoke leaves it too");
        let _ = std::fs::remove_dir_all(base);
    }
}

/// `copy` of a file over 8 MiB is refused, as opening one is.
#[test]
fn script_file_copy_is_held_to_the_file_limit() {
    let base = folder("bigcopy");
    write(&base.join("main/big.bin"), &vec![0u8; 8 * 1024 * 1024 + 1]);
    let r = run(&base, "var b = new File('big.bin'); vc.log([b.copy(Folder.temp.fsName + '/b.bin'), b.error]);");
    assert_eq!(r["log"], json!([[false, "Script file exceeds 8 MiB"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// The run's 32 MiB of writes counts what setting `length` adds to each object the file is open in,
/// as each holds its own bytes.
#[test]
fn script_file_length_counts_the_growth_of_every_open_object() {
    let base = folder("lengthheld");
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    write(
        &main,
        b"var p = Folder.temp.fsName + '/l.bin'; var w = new File(p); w.open('w'); w.close();\n\
          var all = []; for (var i = 0; i < 5; i++) { var f = new File(p); f.open('e'); all.push(f); }\n\
          vc.log('open'); new File(p).length = 7 * 1024 * 1024; vc.log('grown');",
    );
    assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err(), "five objects grow by 7 MiB each");
    let report = s.script_report().unwrap();
    assert_eq!(report["log"], json!(["open"]));
    assert!(report["message"].as_str().unwrap().contains("32 MiB"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

/// `copy` onto another name of the same file, a hard link, keeps the file's bytes, and what the
/// source's open object holds is in the file when `copy` returns.
#[test]
fn script_file_copy_onto_a_hard_link_of_itself_keeps_the_file() {
    let base = folder("copylink");
    let picked = base.join("picked");
    write(&picked.join("a.txt"), b"keep");
    std::fs::hard_link(picked.join("a.txt"), picked.join("b.txt")).unwrap();
    let mut s = Session::new();
    start_framed(
        &mut s,
        &base,
        "var p = Folder.selectDialog('p'); var a = new File(p.fsName + '/a.txt'), b = new File(p.fsName + '/b.txt');\n\
         a.open('w'); a.write('new'); var out = [a.copy(b.fsName)]; b.open('r'); out.push(b.read()); b.close(); a.close(); vc.log(out);",
    );
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![picked.to_string_lossy().into_owned()], always: false }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[true, "new"]]));
    assert_eq!(std::fs::read(picked.join("a.txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(picked.join("b.txt")).unwrap(), b"new");
    let _ = std::fs::remove_dir_all(base);
}

/// The run's 32 MiB of writes counts the copy each open object of `copy`'s target takes.
#[test]
fn script_file_copy_counts_the_bytes_each_open_object_of_the_target_takes() {
    let base = folder("copyheld");
    write(&base.join("main/seven.bin"), &vec![b'a'; 7 << 20]);
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    write(
        &main,
        b"var t = Folder.temp.fsName + '/t.bin'; var w = new File(t); w.open('w'); w.close();\n\
          var all = []; for (var i = 0; i < 5; i++) { var f = new File(t); f.open('e'); all.push(f); }\n\
          vc.log('open'); vc.log(new File('seven.bin').copy(t));",
    );
    assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err(), "five objects take 7 MiB each");
    let report = s.script_report().unwrap();
    assert_eq!(report["log"], json!(["open"]));
    assert!(report["message"].as_str().unwrap().contains("32 MiB"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

/// `write` joins its values only for an open file, and stops joining once the text is longer than
/// any file a script writes: the values after that are not turned into text.
#[test]
fn script_file_write_stops_joining_past_the_file_limit() {
    let base = folder("writejoin");
    let r = run(
        &base,
        "var seen = 0, late = { toString: function () { seen++; return 'x'; } };\n\
         var f = new File(Folder.temp.fsName + '/j.txt'); var closed = [f.write(late), seen];\n\
         var s = new Array((1 << 20) + 1).join('x'), a = []; for (var k = 0; k < 17; k++) a.push(s); a.push(late);\n\
         f.open('w'); var big = f.write.apply(f, a); vc.log([closed, big, f.error, seen, f.write('ok'), f.close(), f.length]);",
    );
    assert_eq!(r["log"], json!([[[false, 0], false, "Script file exceeds 8 MiB", 0, true, true, 2]]));
    let _ = std::fs::remove_dir_all(base);
}

/// The report names at most 4,096 files a run wrote, each once, and says when it wrote more.
#[test]
fn script_reports_name_at_most_4096_written_files() {
    let base = folder("manywritten");
    let r = run(
        &base,
        "var d = Folder.temp.fsName; for (var k = 0; k < 4100; k++) { var f = new File(d + '/' + k + '.txt'); f.open('w'); f.close(); f.open('w'); f.close(); }",
    );
    let written = r["written"].as_array().unwrap();
    assert_eq!(written.len(), crate::script::MAX_WRITTEN, "{}", written.len());
    assert_eq!(written.iter().filter_map(Value::as_str).collect::<std::collections::HashSet<_>>().len(), written.len());
    assert_eq!(r["writtenMore"], true);
    let few = run(&base, "var f = new File(Folder.temp.fsName + '/one.txt'); f.open('w'); f.close();");
    assert_eq!((few["written"].as_array().map(Vec::len), few.get("writtenMore")), (Some(1), None));
    let _ = std::fs::remove_dir_all(base);
}

/// A kept grant stays on the real path it was kept for: when a link to another folder takes the
/// granted folder's place, the next run is not granted the folder the link leads to.
#[cfg(unix)]
#[test]
fn script_file_grants_do_not_follow_a_link_put_in_the_granted_place() {
    let base = folder("grantlink");
    let granted = base.join("picked/granted");
    write(&granted.join("s.txt"), b"mine");
    write(&base.join("away/s.txt"), b"secret");
    let text = format!(
        "var t = new File({:?}), u = new File({:?});\n\
         if (t.open('r')) vc.log(['kept', t.read()]); else if (u.open('r')) vc.log(['away', u.read()]);\n\
         else vc.log(Folder.selectDialog('p') ? 'asked' : null);",
        granted.join("s.txt").to_string_lossy(),
        base.join("away/s.txt").to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![granted.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!(["asked"]));
    start_framed(&mut s, &base, &text);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["kept", "mine"]]));
    std::fs::rename(&granted, base.join("picked/moved")).unwrap();
    std::os::unix::fs::symlink(base.join("away"), &granted).unwrap();
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none(), "the run asks again instead of reading where the link leads");
    s.script_pick(crate::script::files::PickAnswer::Deny);
    let _ = pump_to_picker(&mut s);
    let _ = std::fs::remove_dir_all(base);
}

/// A folder granted in a file dialog cannot have a folder in it renamed when that folder holds the
/// settings folder.
#[test]
fn script_file_rename_does_not_move_the_settings_folder() {
    let base = folder("moveup");
    let mut s = Session::new();
    s.attach_script_data(base.join("cfg/settings/Script Data"));
    std::fs::create_dir_all(base.join("cfg/settings")).unwrap();
    start_framed(&mut s, &base, "var p = Folder.selectDialog('p'); var c = new Folder(p.fsName + '/cfg'); vc.log([c.rename('cfg2'), c.error]);");
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![base.to_string_lossy().into_owned()], always: false }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[false, DENIED]]));
    assert!(base.join("cfg/settings").is_dir());
    let _ = std::fs::remove_dir_all(base);
}

/// Where case does not matter, a folder named in another case is still the folder that holds the
/// settings folder, or the settings folder itself: it is not renamed or removed.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn script_file_rename_does_not_move_the_settings_folder_named_in_another_case() {
    let base = folder("moveupcase");
    let mut s = Session::new();
    s.attach_script_data(base.join("cfg/settings/Script Data"));
    std::fs::create_dir_all(base.join("cfg/settings")).unwrap();
    start_framed(
        &mut s,
        &base,
        "var p = Folder.selectDialog('p'); var c = new Folder(p.fsName + '/CFG'), d = new Folder(p.fsName + '/cfg/SETTINGS');\n\
         vc.log([c.rename('cfg2'), c.error, d.rename('s2'), d.error, d.remove(), d.error]);",
    );
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![base.to_string_lossy().into_owned()], always: false }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[false, DENIED, false, DENIED, false, DENIED]]));
    assert!(base.join("cfg/settings").is_dir());
    let _ = std::fs::remove_dir_all(base);
}

/// Where case matters, a rename to a name that differs in case alone does not replace the other file.
#[cfg(target_os = "linux")]
#[test]
fn script_file_rename_keeps_a_file_that_differs_in_case() {
    let base = folder("casename");
    let r = run(
        &base,
        "var d = Folder.temp.fsName, a = new File(d + '/a.txt'), b = new File(d + '/A.txt');\n\
         a.open('w'); a.write('a'); a.close(); b.open('w'); b.write('B'); b.close();\n\
         var out = [a.rename('A.txt'), a.error]; b.open('r'); out.push(b.read()); b.close(); vc.log(out);",
    );
    assert_eq!(r["log"], json!([[false, "Script file name is already in use", "B"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A script's data folder that is a link is not used: the folder it leads to is not granted.
#[cfg(unix)]
#[test]
fn script_data_folder_that_is_a_link_is_not_granted() {
    let base = folder("datalink");
    let mut s = with_data(&base);
    let r = run_in(&mut s, &base, "vc.log(Folder.userData.fsName);").unwrap();
    let data = PathBuf::from(r["log"][0].as_str().unwrap());
    std::fs::remove_dir(&data).unwrap();
    std::fs::create_dir_all(base.join("away")).unwrap();
    std::os::unix::fs::symlink(base.join("away"), &data).unwrap();
    let text = format!(
        "var d = Folder.userData; var w = new File({:?}); vc.log([d === null, w.open('w'), w.error]);",
        base.join("away/x.txt").to_string_lossy()
    );
    let r = run_in(&mut s, &base, &text).unwrap();
    assert_eq!(r["log"], json!([[true, false, DENIED]]));
    assert!(!base.join("away/x.txt").exists());
    let _ = std::fs::remove_dir_all(base);
}

/// A data folder that is a link is not used: no script's `Folder.userData` is made in the folder it
/// leads to.
#[cfg(unix)]
#[test]
fn script_data_folders_in_a_data_folder_that_is_a_link_are_not_granted() {
    let base = folder("datadirlink");
    let mut s = with_data(&base);
    let r = run_in(&mut s, &base, "vc.log(Folder.userData.fsName);").unwrap();
    let key = PathBuf::from(r["log"][0].as_str().unwrap()).file_name().unwrap().to_owned();
    let data = base.join("settings/Script Data");
    std::fs::remove_dir_all(&data).unwrap();
    std::fs::create_dir_all(base.join("away")).unwrap();
    std::os::unix::fs::symlink(base.join("away"), &data).unwrap();
    let text = format!(
        "var d = Folder.userData; var w = new File({:?}); vc.log([d === null, w.open('w'), w.error]);",
        base.join("away").join(&key).join("x.txt").to_string_lossy()
    );
    let r = run_in(&mut s, &base, &text).unwrap();
    assert_eq!(r["log"], json!([[true, false, DENIED]]));
    assert!(!base.join("away").join(&key).exists());
    let _ = std::fs::remove_dir_all(base);
}

/// An open file whose folder was replaced by a link before it was written is not written through
/// the link.
#[cfg(unix)]
#[test]
fn script_files_are_not_written_through_a_link_put_in_place_of_their_folder() {
    let base = folder("parentlink");
    let mut s = with_data(&base);
    std::fs::create_dir_all(base.join("away")).unwrap();
    // The run opens and changes `sub/f.txt` in its data folder and waits on a file dialog, while
    // another program puts a link to `away` in place of `sub`.
    start_framed(
        &mut s,
        &base,
        "var d = Folder.userData.fsName, f = new File(d + '/sub/f.txt');\n\
         var before = [new Folder(d + '/sub').create(), f.open('w'), f.write('x')];\n\
         File.openDialog('p'); vc.log([before, f.close(), f.error]);",
    );
    assert!(pump_to_picker(&mut s).is_none());
    let data = std::fs::read_dir(base.join("settings/Script Data")).unwrap().next().unwrap().unwrap().path();
    std::fs::rename(data.join("sub"), data.join("old")).unwrap();
    std::os::unix::fs::symlink(base.join("away"), data.join("sub")).unwrap();
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    // The file stays unwritten, so the run's end fails the run as for any file it could not write.
    let e = pump_to_picker(&mut s).unwrap().unwrap_err().to_string();
    assert!(e.contains("left open files that could not be written"), "{e}");
    assert!(!base.join("away/f.txt").exists());
    let _ = std::fs::remove_dir_all(base);
}

/// On Unix, `readonly` clears the write bits and clearing it gives the file the bits it had.
#[cfg(unix)]
#[test]
fn script_file_readonly_keeps_the_other_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let base = folder("modes");
    let mut s = with_data(&base);
    let r = run_in(&mut s, &base, "var u = new File(Folder.userData.fsName + '/m.txt'); u.open('w'); u.close(); vc.log(u.fsName);").unwrap();
    let path = PathBuf::from(r["log"][0].as_str().unwrap());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let text = format!(
        "Folder.userData; var u = new File({:?}); u.readonly = true; var a = u.readonly; u.readonly = false; vc.log([a, u.readonly]);",
        path.to_string_lossy()
    );
    let r = run_in(&mut s, &base, &text).unwrap();
    assert_eq!(r["log"], json!([[true, false]]));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    let _ = std::fs::remove_dir_all(base);
}

/// Clearing `readonly` gives back every write bit that setting it cleared in the run, also after a
/// rename, and only the owner's write bit for a file that was read-only before the run.
#[cfg(unix)]
#[test]
fn script_file_readonly_gives_back_the_write_bits_it_cleared() {
    use std::os::unix::fs::PermissionsExt;
    let base = folder("modesback");
    let mut s = with_data(&base);
    let r = run_in(&mut s, &base, "var u = new File(Folder.userData.fsName + '/m.txt'); u.open('w'); u.close(); vc.log(u.fsName);").unwrap();
    let path = PathBuf::from(r["log"][0].as_str().unwrap());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o662)).unwrap();
    let toggle = format!(
        "Folder.userData; var u = new File({:?}); u.readonly = true; var a = u.readonly; var b = u.rename('n.txt');\n\
         u.readonly = false; vc.log([a, b, u.readonly]); u.rename('m.txt');",
        path.to_string_lossy()
    );
    let r = run_in(&mut s, &base, &toggle).unwrap();
    assert_eq!(r["log"], json!([[true, true, false]]));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o662);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let clear = format!("Folder.userData; var u = new File({:?}); u.readonly = false;", path.to_string_lossy());
    run_in(&mut s, &base, &clear).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);
    let _ = std::fs::remove_dir_all(base);
}

/// Opening a file to append or edit reads it, so the automation read roots apply as well as the
/// write roots.
#[test]
fn script_file_append_and_edit_need_the_read_roots() {
    let base = folder("appendroots");
    let mut s = with_data(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    let roots = AutomationRoots::new(&[&base.join("main")], &[&base]).unwrap();
    let r = confine(roots.as_ref(), || {
        run_in(
            &mut s,
            &base,
            "var u = new File(Folder.userData.fsName + '/w.txt'), out = [u.open('w'), u.write('kept'), u.close()];\n\
             out.push(u.open('a'), u.error, u.open('e'), u.error); vc.log(out);",
        )
    })
    .unwrap();
    assert_eq!(r["log"], json!([[true, true, true, false, DENIED, false, DENIED]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A script run from a folder that holds the settings folder reads its own files there, but no file
/// in the settings folder.
#[test]
fn script_files_in_a_folder_above_the_settings_folder_do_not_reach_it() {
    let base = folder("above");
    let mut s = Session::new();
    s.attach_script_data(base.join("main/settings/Script Data"));
    write(&base.join("main/settings/Script Grants.json"), b"{\"version\": 1, \"grants\": []}");
    write(&base.join("main/own.txt"), b"own");
    let r = run_in(
        &mut s,
        &base,
        "var a = new File('own.txt'), g = new File('settings/Script Grants.json');\n\
         var out = [a.open('r'), a.read()]; a.close(); out.push(g.open('r'), g.error, g.exists); vc.log(out);",
    )
    .unwrap();
    assert_eq!(r["log"], json!([[true, "own", false, DENIED, false]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A script run from the settings folder itself reads its own file there, but no other file in the
/// settings folder.
#[test]
fn script_files_in_the_settings_folder_do_not_reach_it() {
    let base = folder("insettings");
    let mut s = Session::new();
    s.attach_script_data(base.join("main/Script Data"));
    write(&base.join("main/Script Grants.json"), b"{\"version\": 1, \"grants\": []}");
    let r = run_in(
        &mut s,
        &base,
        "var a = new File('main.jsx'), g = new File('Script Grants.json');\n\
         var out = [a.open('r'), a.read().length > 0]; a.close(); out.push(g.open('r'), g.error, g.exists); vc.log(out);",
    )
    .unwrap();
    assert_eq!(r["log"], json!([[true, true, false, DENIED, false]]));
    let _ = std::fs::remove_dir_all(base);
}

/// The settings folder is kept out of every grant also before it has been made.
#[test]
fn script_file_grants_never_reach_a_settings_folder_not_made_yet() {
    let base = folder("unmade");
    let mut s = with_data(&base);
    assert!(!base.join("settings").exists());
    start_framed(&mut s, &base, "var o = new Folder(Folder.userData.parent.parent.fsName); var f = o.selectDlg('p'); vc.log([f, o.error]);");
    assert!(pump_to_picker(&mut s).is_none());
    let settings = base.join("settings").to_string_lossy().into_owned();
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![settings], always: true }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[null, "Script file access to the app's own folders is not allowed"]]));
    assert_eq!(s.execute("script.grants", &json!({})).unwrap()["grants"], json!([]));
    let _ = std::fs::remove_dir_all(base);
}

/// Files left open are written when the run ends in the order they were opened, so of two objects
/// that hold the same file, the one opened last decides what it holds.
#[test]
fn script_files_left_open_are_written_in_the_order_they_were_opened() {
    let base = folder("flushorder");
    let mut s = with_data(&base);
    let text = "var p = Folder.userData.fsName + '/two.txt'; var a = new File(p), b = new File(p);\n\
                a.open('w'); b.open('w'); a.write('first'); b.write('second'); vc.log(p);";
    for _ in 0..16 {
        let r = run_in(&mut s, &base, text).unwrap();
        let path = PathBuf::from(r["log"][0].as_str().unwrap_or_default());
        assert_eq!(std::fs::read(&path).unwrap(), b"second", "{r}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// Removing a file leaves the other objects that have it open without the file: they take no more
/// writes, and closing them does not make it again.
#[test]
fn script_file_remove_detaches_the_other_open_objects() {
    let base = folder("removeopen");
    let mut s = with_data(&base);
    let text = "var p = Folder.userData.fsName + '/gone.txt'; var a = new File(p), b = new File(p);\n\
                a.open('w'); a.write('x'); a.close(); b.open('e');\n\
                var removed = a.remove(); var w = b.write('y'); var e = b.error; b.close();\n\
                vc.log([removed, w, e, new File(p).exists]);";
    let r = run_in(&mut s, &base, text).unwrap();
    assert_eq!(r["log"], json!([[true, false, "Script file is not open for writing", false]]), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A run that ends with a changed file open that cannot be written fails and names the file.
#[test]
fn script_files_left_open_that_cannot_be_written_fail_the_run() {
    let base = folder("unwritten");
    let mut s = with_data(&base);
    let r = run_in(
        &mut s,
        &base,
        "var u = new File(Folder.userData.fsName + '/ro.txt'); u.open('w'); u.write('x'); new File(u.fsName).readonly = true; vc.log(u.fsName);",
    );
    let report = s.script_report().unwrap();
    let path = PathBuf::from(report["log"][0].as_str().unwrap_or_default());
    if let Ok(m) = std::fs::metadata(&path) {
        let mut p = m.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        let _ = std::fs::set_permissions(&path, p);
    }
    assert!(r.is_err(), "{report}");
    let message = report["message"].as_str().unwrap();
    assert!(message.contains("the script left open files that could not be written") && message.contains("ro.txt"), "{report}");
    assert_eq!(std::fs::read(&path).unwrap(), b"", "{report}");
    let _ = std::fs::remove_dir_all(base);
}

/// The automation write roots apply to the run's own folders too: outside them `Folder.temp` and
/// `Folder.userData` are `null` and are not made.
#[test]
fn script_files_write_only_inside_the_automation_write_roots() {
    let base = folder("writeroots");
    std::fs::create_dir_all(base.join("out")).unwrap();
    let roots = AutomationRoots::new(&[&base], &[&base.join("out")]).unwrap();
    let mut s = with_data(&base);
    let r = confine(roots.as_ref(), || run_in(&mut s, &base, "vc.log([Folder.temp === null, Folder.userData === null]);")).unwrap();
    assert_eq!(r["log"], json!([[true, true]]));
    assert!(!base.join("settings/Script Data").exists(), "the data folder is not made");
    let _ = std::fs::remove_dir_all(base);
}

/// The run's temporary folder is checked against the write roots by its own name: a write root named
/// `VectorCraft Script` in the temporary folder does not take the run's folder beside it.
#[test]
fn script_temporary_folder_is_checked_by_its_own_name() {
    let base = folder("tempname");
    let named = std::env::temp_dir().join("VectorCraft Script");
    let made = std::fs::create_dir(&named).is_ok();
    let roots = AutomationRoots::new(&[&base], &[&named]).unwrap();
    let mut s = with_data(&base);
    let r = confine(roots.as_ref(), || run_in(&mut s, &base, "vc.log(Folder.temp === null);"));
    if made {
        let _ = std::fs::remove_dir(&named);
    }
    assert_eq!(r.unwrap()["log"], json!([true]));
    let _ = std::fs::remove_dir_all(base);
}

/// The folders made above a script's data folder are inside the write roots too: with a write root
/// that names the data folder after the folders above it were removed, none of them is made again.
#[test]
fn script_data_folder_makes_no_folder_outside_the_write_roots() {
    let base = folder("dataparent");
    let mut s = with_data(&base);
    let r = run_in(&mut s, &base, "vc.log(Folder.userData.fsName);").unwrap();
    let data = PathBuf::from(r["log"][0].as_str().unwrap());
    let roots = AutomationRoots::new(&[&base], &[&data]).unwrap();
    std::fs::remove_dir_all(base.join("settings")).unwrap();
    let r = confine(roots.as_ref(), || run_in(&mut s, &base, "vc.log(Folder.userData === null);")).unwrap();
    assert_eq!(r["log"], json!([true]));
    assert!(!base.join("settings").exists(), "no folder above the write root is made");
    let _ = std::fs::remove_dir_all(base);
}

/// `create` makes no folder outside the places the script may write: when the folders above its data
/// folder were removed during the run, a folder in the data folder is not made, and neither are they.
#[test]
fn script_folder_create_makes_no_folder_outside_the_grants() {
    let base = folder("createabove");
    let mut s = with_data(&base);
    start_framed(&mut s, &base, "var d = Folder.userData.fsName; File.openDialog('p'); var f = new Folder(d + '/a'); vc.log([f.create(), f.error]);");
    assert!(pump_to_picker(&mut s).is_none());
    std::fs::remove_dir_all(base.join("settings")).unwrap();
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[false, DENIED]]));
    assert!(!base.join("settings").exists(), "no folder above the data folder is made");
    let _ = std::fs::remove_dir_all(base);
}

/// A file a script wrote stays written after its run fails, and the report names it.
#[test]
fn script_files_written_before_a_failure_are_named() {
    let base = folder("failwrite");
    let mut s = with_data(&base);
    let r = run_in(&mut s, &base, "var u = new File(Folder.userData.fsName + '/left.txt'); u.open('w'); u.write('left'); throw new Error('late');");
    assert!(r.is_err());
    let report = s.script_report().unwrap();
    let written: Vec<PathBuf> = report["written"].as_array().unwrap().iter().filter_map(Value::as_str).map(PathBuf::from).collect();
    assert_eq!(written.len(), 1, "{report}");
    assert_eq!(std::fs::read(&written[0]).unwrap(), b"left", "the open file is written when the run ends");
    let _ = std::fs::remove_dir_all(base);
}

/// Pumps the run in the app until it ends or waits at a file dialog (`None`).
fn pump_to_picker(s: &mut Session) -> Option<Result<Value>> {
    for _ in 0..10_000 {
        if s.script_picker().is_some() {
            return None;
        }
        if let Some(r) = s.script_pump() {
            return Some(r);
        }
    }
    panic!("the run neither ended nor waited");
}

/// Starts `main/main.jsx` in `base` with the text `text`, in the app.
fn start_framed(s: &mut Session, base: &Path, text: &str) {
    s.script_frames = true;
    let main = base.join("main/main.jsx");
    write(&main, text.as_bytes());
    assert_eq!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap()["running"], true);
}

/// The filter of an open or save dialog gives the file types the dialog shows; an open dialog with
/// `multiSelect` takes every file picked and gives them as a list.
#[test]
fn script_file_dialogs_show_the_filters_file_types_and_open_several_files() {
    use crate::script::files::dialog_filters;
    let f = |types: &[(&str, &[&str])]| -> Vec<(String, Vec<String>)> {
        types.iter().map(|(n, e)| (n.to_string(), e.iter().map(|e| e.to_string()).collect())).collect()
    };
    assert_eq!(dialog_filters("JavaScript files:*.jsx;All files:*.*"), f(&[("JavaScript files", &["jsx"]), ("All files", &["*"])]));
    assert_eq!(dialog_filters("Images:*.png,*.JPG , Text:*.txt"), f(&[("Images", &["png", "JPG"]), ("Text", &["txt"])]));
    assert_eq!(dialog_filters("*.svg;*.ai"), f(&[("*.svg;*.ai", &["svg", "ai"])]));
    assert_eq!(dialog_filters("All files:*.*"), f(&[]), "every file");
    assert_eq!(dialog_filters("Odd:a*b.txt;Bad:*.a/b;Text:*.txt"), f(&[("Text", &["txt"])]), "patterns that are not *.ext are left out");
    assert_eq!(dialog_filters(""), f(&[]));

    let base = folder("pickfilter");
    for name in ["a.txt", "b.txt"] {
        write(&base.join("picked").join(name), b"x");
    }
    let mut s = with_data(&base);
    start_framed(&mut s, &base, "var f = File.openDialog('Pick', 'Text:*.txt;All files:*.*', true); vc.log([f.length, f[0].name, f[1].name]);");
    assert!(pump_to_picker(&mut s).is_none());
    let picker = s.script_picker().unwrap();
    assert!(picker.multiple);
    assert_eq!(picker.filters, f(&[("Text", &["txt"]), ("All files", &["*"])]));
    let paths = ["a.txt", "b.txt"].iter().map(|n| base.join("picked").join(n).to_string_lossy().into_owned()).collect();
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths, always: false }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[2, "a.txt", "b.txt"]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A dialog's `filter` that is a function is called with each file picked, and the files it gives a
/// false value for are left out: a list keeps the rest, and one file, or a list left empty, gives
/// `null`. The dialog shows every file. A save dialog takes the same filter.
#[test]
fn script_file_dialogs_leave_out_the_files_a_filter_function_refuses() {
    let base = folder("pickfilterfn");
    for name in ["a.txt", "b.svg", "c.txt"] {
        write(&base.join("picked").join(name), b"x");
    }
    let paths = |names: &[&str]| -> Vec<String> { names.iter().map(|n| base.join("picked").join(n).to_string_lossy().into_owned()).collect() };
    let text = "var seen = [];\n\
                function txt(f) { seen.push(f instanceof File ? f.name : null); return f.name.slice(-4) === '.txt'; }\n\
                var many = File.openDialog('Pick', txt, true);\n\
                vc.log([many.length, many[0].name, many[1].name]);\n\
                vc.log(File.openDialog('Pick one', txt));\n\
                vc.log(File.openDialog('Pick svg', txt, true));\n\
                var saved = File.saveDialog('Save', txt);\n\
                vc.log(saved && saved.name);\n\
                vc.log(new File('x.txt').saveDlg('Save svg', txt));\n\
                vc.log(seen);";
    let mut s = with_data(&base);
    start_framed(&mut s, &base, text);
    for picked in [&["a.txt", "b.svg", "c.txt"][..], &["b.svg"], &["b.svg"], &["c.txt"], &["b.svg"]] {
        assert!(pump_to_picker(&mut s).is_none());
        assert_eq!(s.script_picker().unwrap().filters, Vec::<(String, Vec<String>)>::new(), "every file is shown");
        assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: paths(picked), always: false }));
    }
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    let seen = ["a.txt", "b.svg", "c.txt", "b.svg", "b.svg", "c.txt", "b.svg"];
    assert_eq!(r["log"], json!([[2, "a.txt", "c.txt"], null, null, "c.txt", null, seen]));
    let _ = std::fs::remove_dir_all(base);
}

/// A file a person allows always is kept for that script with that text: a later run reads it
/// without asking, and a run of changed text asks again. A file allowed once is not kept.
#[test]
fn script_file_dialog_grants_are_kept_for_the_same_script_text() {
    let base = folder("grants");
    let out = base.join("picked/out.txt");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let text = format!(
        "var target = new File({:?});\n\
         if (target.open('r')) {{ vc.log(['kept', target.read()]); target.close(); }}\n\
         else {{ vc.log(['asked', target.error]); var f = File.saveDialog('Save it'); if (f) {{ f.open('w'); f.write('hi'); vc.log(f.close()); }} else vc.log(null); }}",
        out.to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    let picker = s.script_picker().unwrap();
    assert_eq!((picker.kind, picker.prompt.as_str(), picker.can_always), (crate::script::files::PickerKind::Save, "Save it", true));
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([["asked", DENIED], true]));
    assert_eq!(std::fs::read(&out).unwrap(), b"hi");
    let grants = s.execute("script.grants", &json!({})).unwrap();
    assert_eq!(grants["grants"].as_array().unwrap().len(), 1, "{grants}");
    assert_eq!(grants["grants"][0]["write"], true);
    // Under automation read roots, a path outside them is named by its file name alone.
    let roots = AutomationRoots::new(&[&base.join("main")], &[]).unwrap();
    let hidden = confine(roots.as_ref(), || s.execute("script.grants", &json!({}))).unwrap();
    assert_eq!(hidden["grants"][0]["path"], "out.txt", "{hidden}");
    assert_eq!(hidden["grants"][0]["script"], grants["grants"][0]["script"], "the script inside the roots is named whole: {hidden}");
    assert_eq!(hidden["file"], "Script Grants.json", "{hidden}");

    start_framed(&mut s, &base, &text);
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([["kept", "hi"]]), "the same script text keeps its grant");

    start_framed(&mut s, &base, &format!("{text}\n// changed"));
    assert!(pump_to_picker(&mut s).is_none(), "changed text has no grant, so it asks");
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([["asked", DENIED], null]));

    let once = "var g = File.openDialog('Read it'); vc.log(g.fsName === null ? null : g.open('r') && g.read());";
    start_framed(&mut s, &base, once);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: false }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!(["hi"]));
    assert_eq!(s.execute("script.grants", &json!({})).unwrap()["grants"].as_array().unwrap().len(), 1, "a file allowed once is not kept");

    // A run that waits at a dialog keeps the grants it started with, so they are not revoked under it.
    start_framed(&mut s, &base, once);
    assert!(pump_to_picker(&mut s).is_none());
    let busy = s.execute("script.grants.revoke", &json!({"all": true}));
    assert!(matches!(busy, Err(crate::EngineError::Busy)), "{busy:?}");
    assert_eq!(s.execute("script.grants", &json!({})).unwrap()["grants"].as_array().unwrap().len(), 1);
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    assert!(pump_to_picker(&mut s).is_some(), "the run ends");

    // Under automation read roots, a path outside them is not looked up: a link there to the
    // granted file does not name it.
    #[cfg(unix)]
    {
        let link = base.join("link.txt");
        std::os::unix::fs::symlink(&out, &link).unwrap();
        let roots = AutomationRoots::new(&[&base.join("main")], &[]).unwrap();
        let r = confine(roots.as_ref(), || s.execute("script.grants.revoke", &json!({"path": link.to_string_lossy()}))).unwrap();
        assert_eq!(r["removed"], 0, "{r}");
        let r = s.execute("script.grants.revoke", &json!({"path": link.to_string_lossy()})).unwrap();
        assert_eq!(r["removed"], 1, "without roots the link is followed: {r}");
    }
    #[cfg(not(unix))]
    {
        let removed = s.execute("script.grants.revoke", &json!({"path": out.to_string_lossy()})).unwrap();
        assert_eq!(removed["removed"], 1);
    }
    assert_eq!(s.execute("script.grants", &json!({})).unwrap()["grants"], json!([]));
    assert!(s.execute("script.grants.revoke", &json!({})).is_err());
    assert!(!base.join("settings/Script Grants.json.part").exists());
    let _ = std::fs::remove_dir_all(base);
}

/// A grant is kept for the script's text with the files it includes: a changed included file asks
/// again.
#[test]
fn script_file_grants_are_kept_for_the_included_files_too() {
    let base = folder("grantinclude");
    let out = base.join("picked/out.txt");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    write(&out, b"hi");
    write(&base.join("main/lib.jsxinc"), b"var lib = 1;\n");
    let text = format!(
        "#include \"lib.jsxinc\"\nvar t = new File({:?});\nif (t.open('r')) vc.log('kept'); else vc.log(File.openDialog('p') ? 'asked' : null);",
        out.to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!(["asked"]));
    start_framed(&mut s, &base, &text);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!(["kept"]));
    write(&base.join("main/lib.jsxinc"), b"var lib = 2;\n");
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none(), "a changed included file has no grant, so it asks");
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([null]));
    let _ = std::fs::remove_dir_all(base);
}

/// A kept grant is in force only while the run evaluates code the run that kept it evaluated: a
/// file `$.evalFile` runs, or a string given to `eval` or `Function`, that differs withdraws it
/// before that code runs.
#[test]
fn script_file_grants_are_kept_for_the_code_the_script_evaluates() {
    let base = folder("granteval");
    let out = base.join("picked/out.txt");
    write(&out, b"hi");
    write(&base.join("main/lib.jsxinc"), b"var lib = 1;");
    write(&base.join("main/code.txt"), b"new File(p).open('r')");
    let text = format!(
        "var p = {:?};\n$.evalFile('lib.jsxinc');\n\
         var c = new File('code.txt'); c.open('r'); var src = c.read(); c.close();\n\
         var one = Function('return 1')();\n\
         var before = new File(p).open('r');\n\
         var during = eval(src) && one === 1;\n\
         if (before) vc.log(['kept', during]); else vc.log(['asked', !!File.openDialog('p')]);",
        out.to_string_lossy()
    );
    let mut s = with_data(&base);
    let picked = || crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true };
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(picked()));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["asked", true]]));
    start_framed(&mut s, &base, &text);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["kept", true]]), "the same code keeps the grant");
    write(&base.join("main/code.txt"), b"new File(p).open('r') && true");
    start_framed(&mut s, &base, &text);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["kept", false]]), "changed eval code runs without the grant");
    write(&base.join("main/code.txt"), b"new File(p).open('r')");
    write(&base.join("main/lib.jsxinc"), b"var lib = 2;");
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none(), "a changed $.evalFile file has no grant, so it asks");
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["asked", false]]));
    write(&base.join("main/lib.jsxinc"), b"var lib = 1;");
    start_framed(&mut s, &base, &text);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["kept", true]]));
    let grants = s.execute("script.grants", &json!({})).unwrap();
    assert_eq!(grants["grants"][0]["code"].as_array().map(Vec::len), Some(3), "{grants}");
    let _ = std::fs::remove_dir_all(base);
}

/// A file's grant covers that path only: a folder the script makes in the file's place, in the run
/// that was allowed the file and in later runs that keep its grant, is not read or written inside.
#[test]
fn script_file_grants_do_not_reach_into_a_folder_in_the_files_place() {
    let base = folder("grantfileplace");
    let out = base.join("picked/out.txt");
    write(&out, b"hi");
    let text = "var p = File.saveDialog('p').fsName;\n\
                var gone = new File(p).remove(); var made = new Folder(p).create();\n\
                var w = new File(p + '/x.txt'); vc.log([gone, made, w.open('w'), w.error]);";
    let mut s = with_data(&base);
    start_framed(&mut s, &base, text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([[true, true, false, DENIED]]));
    assert!(out.is_dir() && !out.join("x.txt").exists());
    let grants = s.execute("script.grants", &json!({})).unwrap();
    assert_eq!(grants["grants"][0]["folder"], false, "{grants}");

    let later = format!("var w = new File({:?}); vc.log([w.open('w'), w.error]);", out.join("y.txt").to_string_lossy());
    let main = base.join("main/main.jsx");
    write(&main, later.as_bytes());
    let real = std::fs::canonicalize(&main).unwrap();
    let hash = vectorcraft_plugins::permissions::sha256_hex(later.as_bytes());
    let out_real = crate::cmd::findfiles::plain(std::fs::canonicalize(&out).unwrap());
    let kept = json!({"version": 1, "grants": [{"script": crate::cmd::findfiles::plain(real).to_string_lossy(), "sha256": hash, "code": [], "path": out_real.to_string_lossy(), "write": true, "folder": false}]});
    write(&base.join("settings/Script Grants.json"), kept.to_string().as_bytes());
    start_framed(&mut s, &base, &later);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([[false, DENIED]]), "a kept file grant does not reach inside either");
    assert!(!out.join("y.txt").exists());
    let _ = std::fs::remove_dir_all(base);
}

/// The code a run evaluates after a person allowed it a file always is kept with the grant when
/// the run ends, so the next run of the same script that evaluates it keeps the grant.
#[test]
fn script_file_grants_keep_the_code_evaluated_after_the_dialog() {
    let base = folder("grantlater");
    let out = base.join("picked/out.txt");
    write(&out, b"hi");
    let text = format!(
        "var t = new File({:?}); var kept = t.open('r');\n\
         if (!kept) File.openDialog('p');\n\
         vc.log([kept, eval('new File(t.fsName).open(\"r\")')]);",
        out.to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([[false, true]]));
    start_framed(&mut s, &base, &text);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([[true, true]]));
    let _ = std::fs::remove_dir_all(base);
}

/// Code another script left in a named engine withdraws a script's kept grants in that engine.
#[test]
fn script_file_grants_are_withdrawn_by_code_another_script_left_in_the_engine() {
    let base = folder("grantengine");
    let out = base.join("picked/out.txt");
    write(&out, b"hi");
    let text = format!(
        "#targetengine shared\nvar t = new File({:?});\nif (t.open('r')) vc.log('kept'); else vc.log(File.openDialog('p') ? 'asked' : null);",
        out.to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!(["asked"]));
    start_framed(&mut s, &base, &text);
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!(["kept"]), "the engine holds only this script's code");
    let other = base.join("main/other.jsx");
    write(&other, b"#targetengine shared\nvar otherWasHere = 1;");
    s.script_frames = false;
    s.execute("script.run", &json!({"path": other.to_string_lossy()})).unwrap();
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none(), "another script's code is in the engine, so it asks");
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([null]));
    let _ = std::fs::remove_dir_all(base);
}

/// Code a script given as text left in a named engine withdraws a script's kept grants in that
/// engine, as a file script's code does.
#[test]
fn script_file_grants_are_withdrawn_by_code_a_text_script_left_in_the_engine() {
    let base = folder("grantenginetext");
    let out = base.join("picked/out.txt");
    write(&out, b"hi");
    let text = format!(
        "#targetengine shared\nvar t = new File({:?});\nif (t.open('r')) vc.log('kept'); else vc.log(File.openDialog('p') ? 'asked' : null);",
        out.to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![out.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!(["asked"]));
    s.script_frames = false;
    s.execute("script.run", &json!({"source": "#targetengine shared\nvar textWasHere = 1;"})).unwrap();
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none(), "a text script's code is in the engine, so it asks");
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([null]));
    let _ = std::fs::remove_dir_all(base);
}

/// A file opened for writing under a kept grant that new code withdraws takes no more writes, and
/// changes that cannot be written then fail the run when it ends.
#[test]
fn script_files_open_under_a_withdrawn_grant_keep_their_unwritten_changes() {
    let base = folder("grantwithdrawn");
    let dir = base.join("picked");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out.txt");
    let text = format!(
        "var f = new File({:?});\n\
         if (!f.open('w')) vc.log(['asked', !!Folder.selectDialog('p')]);\n\
         else {{ f.write('a'); new File(f.fsName).readonly = true; eval('1'); vc.log(['wrote', f.write('b'), f.error]); }}",
        out.to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![dir.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["asked", true]]));
    start_framed(&mut s, &base, &text);
    let r = pump_to_picker(&mut s).unwrap();
    let report = s.script_report().unwrap();
    if let Ok(m) = std::fs::metadata(&out) {
        let mut p = m.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        let _ = std::fs::set_permissions(&out, p);
    }
    assert!(r.is_err(), "{report}");
    assert_eq!(report["log"], json!([["wrote", false, "Script file is not open for writing"]]), "{report}");
    let message = report["message"].as_str().unwrap();
    assert!(message.contains("the script left open files that could not be written") && message.contains("out.txt"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

/// A file opened for reading under a kept grant that new code withdraws gives no more of what it
/// holds.
#[test]
fn script_files_open_under_a_withdrawn_grant_are_not_read() {
    let base = folder("grantwithdrawnread");
    let dir = base.join("picked");
    write(&dir.join("in.txt"), b"secret");
    let text = format!(
        "var f = new File({:?});\n\
         if (!f.open('r')) vc.log(['asked', !!Folder.selectDialog('p')]);\n\
         else {{ var a = f.readch(); eval('1'); var r = f.read(), e = f.error; f.error = ''; vc.log([a, r, e, f.eof, f.seek(0, 2), f.error, f.tell()]); }}",
        dir.join("in.txt").to_string_lossy()
    );
    let mut s = with_data(&base);
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![dir.to_string_lossy().into_owned()], always: true }));
    assert_eq!(pump_to_picker(&mut s).unwrap().unwrap()["log"], json!([["asked", true]]));
    start_framed(&mut s, &base, &text);
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([["s", "", DENIED, true, false, DENIED, 1]]));
    let _ = std::fs::remove_dir_all(base);
}

/// The folder that holds the settings is kept out of every grant: a pick there gives `null` with the
/// error, a grant written into the grants file for it is not taken, and the script cannot write the
/// grants file through its data folder.
#[test]
fn script_file_grants_never_reach_the_settings_folder() {
    let base = folder("protected");
    let mut s = with_data(&base);
    std::fs::create_dir_all(base.join("settings/Scripts")).unwrap();
    let inside = base.join("settings/Scripts/x.txt");
    write(&inside, b"secret");
    let text = format!(
        "var o = new File('x.txt'); var f = o.saveDlg('p'); vc.log([f, o.error]);\n\
         var g = new File(Folder.userData.parent.parent.fsName + '/Script Grants.json'); vc.log([g.open('w'), g.error]);\n\
         var h = new File({:?}); vc.log([h.open('r'), h.error]);",
        inside.to_string_lossy()
    );
    let main = base.join("main/main.jsx");
    write(&main, text.as_bytes());
    let real = std::fs::canonicalize(&main).unwrap();
    let hash = vectorcraft_plugins::permissions::sha256_hex(text.as_bytes());
    let settings = crate::cmd::findfiles::plain(std::fs::canonicalize(base.join("settings")).unwrap());
    let forged = json!({"version": 1, "grants": [{"script": crate::cmd::findfiles::plain(real).to_string_lossy(), "sha256": hash, "code": [], "path": settings.to_string_lossy(), "write": true, "folder": true}]});
    write(&base.join("settings/Script Grants.json"), forged.to_string().as_bytes());
    start_framed(&mut s, &base, &text);
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![inside.to_string_lossy().into_owned()], always: true }));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    let protected = "Script file access to the app's own folders is not allowed";
    assert_eq!(r["log"], json!([[null, protected], [false, DENIED], [false, DENIED]]));
    assert_eq!(std::fs::read(&inside).unwrap(), b"secret");
    let _ = std::fs::remove_dir_all(base);
}

/// A script in the Scripts folder inside the settings folder reads the files beside it, while the
/// rest of the settings folder stays closed to it.
#[test]
fn a_script_in_the_settings_scripts_folder_reads_its_own_folder() {
    let base = folder("installed");
    let mut s = with_data(&base);
    write(&base.join("settings/Scripts/x.txt"), b"beside");
    write(&base.join("settings/other.txt"), b"settings");
    let main = base.join("settings/Scripts/main.jsx");
    write(&main, b"var f = new File('x.txt'), g = new File(f.parent.parent.fsName + '/other.txt'); vc.log([f.open('r'), f.read(), g.open('r')]);");
    let r = s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap();
    assert_eq!(r["log"], json!([[true, "beside", false]]));
    let _ = std::fs::remove_dir_all(base);
}

/// A file dialog inside a built-in's callback throws in the app, where it cannot wait.
#[test]
fn script_file_dialog_in_a_callback_throws() {
    let base = folder("callback");
    let mut s = Session::new();
    start_framed(&mut s, &base, "[1].forEach(function () { File.openDialog(); });");
    let r = pump_to_picker(&mut s).unwrap();
    assert!(r.is_err());
    let report = s.script_report().unwrap();
    assert!(report["message"].as_str().unwrap().contains("File.openDialog: a file dialog cannot open inside a callback"), "{report}");
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
