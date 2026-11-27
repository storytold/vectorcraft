//! `XMPFile` in a script: reading and writing a sidecar packet through the script's file places,
//! scanning another file for its packet, and the paths and packets it refuses.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

const LOAD: &str = "new ExternalObject('lib:AdobeXMPScript');\n";

const PACKET: &str = "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\
<dc:format>image/svg+xml</dc:format><dc:subject><rdf:Bag><rdf:li>a</rdf:li><rdf:li>b</rdf:li></rdf:Bag></dc:subject>\
</rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>";

#[test]
fn a_large_supported_packet_crosses_the_sidecar_write_transport() {
    let base = folder("large-write-transport");
    let payload = "x".repeat(6 << 20);
    let packet = PACKET.replace("image/svg+xml", &payload);
    let mut session = with_data(&base);
    let setup = logged(&mut session, &base, "vc.log(Folder.userData.fsName + '/large.xmp');");
    let path = PathBuf::from(setup[0].as_str().unwrap());
    write(&path, packet.as_bytes());
    let log = logged(
        &mut session,
        &base,
        "var p = Folder.userData.fsName + '/large.xmp'; var x = new XMPFile(p, XMPConst.FILE_XML, XMPConst.OPEN_FOR_UPDATE); var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'title', 'changed-for-write'); x.putXMP(m); x.closeFile(); vc.log(true);",
    );
    assert_eq!(log, json!([true]));
    let written = std::fs::read_to_string(path).unwrap();
    assert!(written.contains(&payload));
    assert!(written.contains("changed-for-write"));
}

struct Folder {
    _guard: vectorcraft_testkit::TempDir,
    path: PathBuf,
}

impl std::ops::Deref for Folder {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for Folder {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// A fresh folder for test `name`, with a `main` folder inside it for the script, by its real path.
fn folder(name: &str) -> Folder {
    let guard = vectorcraft_testkit::TempDir::new(&format!("script-xmp-{name}"));
    let base = std::fs::canonicalize(&*guard).unwrap();
    std::fs::create_dir_all(base.join("main")).unwrap();
    Folder { _guard: guard, path: base }
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

/// A session whose scripts keep their data folders in `base/settings/Script Data`.
fn with_data(base: &Path) -> Session {
    let mut s = Session::new();
    s.attach_script_data(base.join("settings/Script Data"));
    s
}

/// Runs `main/main.jsx` in `base` with the XMP library loaded and the text `text` in `s`.
fn run_in(s: &mut Session, base: &Path, text: &str) -> Result<Value> {
    let main = base.join("main/main.jsx");
    write(&main, format!("{LOAD}{text}").as_bytes());
    s.execute("script.run", &json!({"path": main.to_string_lossy()}))
}

/// The values the run of `text` logged.
fn logged(s: &mut Session, base: &Path, text: &str) -> Value {
    match run_in(s, base, text) {
        Ok(r) => r["log"].clone(),
        Err(e) => panic!("{text}\n{e}"),
    }
}

/// A sidecar in the script's data folder reads, takes a changed packet and holds it after
/// `closeFile`; a new `XMPFile` reads it back.
#[test]
fn a_sidecar_packet_reads_and_writes() {
    let base = folder("sidecar");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!(
            "var p = Folder.userData.fsName + '/a.xmp'; var f = new File(p); f.encoding = 'UTF-8'; f.open('w'); f.write({PACKET:?}); f.close();\n\
             var x = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
             var m = x.getXMP(); var info = x.getFileInfo();\n\
             var out = [m.getProperty(XMPConst.NS_DC, 'format').value, m.countArrayItems(XMPConst.NS_DC, 'subject'), info.format == XMPConst.FILE_XML, x.canPutXMP(m)];\n\
             m.setProperty(XMPConst.NS_DC, 'format', 'text/plain'); m.appendArrayItem(XMPConst.NS_DC, 'subject', 'c');\n\
             x.putXMP(m); x.closeFile();\n\
             var y = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); var n = y.getXMP();\n\
             out.push(n.getProperty(XMPConst.NS_DC, 'format').value, n.getArrayItem(XMPConst.NS_DC, 'subject', 3).value, y.getPacketInfo().offset, y.canPutXMP(n));\n\
             y.closeFile(); vc.log(out); vc.log(p);"
        ),
    );
    assert_eq!(log[0], json!(["image/svg+xml", 2, true, true, "text/plain", "c", 0, false]));
    let path = PathBuf::from(log[1].as_str().unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("<dc:format>text/plain</dc:format>"), "{text}");
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
    assert_eq!(leftovers.len(), 1, "no temporary file stays: {leftovers:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// A file that is not a sidecar is scanned for its packet, read only, and a file with none gives
/// `null`.
#[test]
fn another_file_is_scanned_for_its_packet() {
    let base = folder("scan");
    let mut bytes = b"\x89binary\x00head".to_vec();
    bytes.extend_from_slice(PACKET.as_bytes());
    bytes.extend_from_slice(b"\x00tail");
    write(&base.join("main/art.bin"), &bytes);
    write(&base.join("main/none.bin"), b"no packet here");
    let mut s = Session::new();
    let log = logged(
        &mut s,
        &base,
        "var x = new XMPFile(File('art.bin').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); var i = x.getPacketInfo();\n\
         var out = [x.getXMP().getProperty(XMPConst.NS_DC, 'format').value, i.offset, i.length, i.writeable, x.canPutXMP(x.getXMP())];\n\
         try { x.putXMP(x.getXMP()); out.push('put'); } catch (e) { out.push(e.message); }\n\
         out.push(new XMPFile(File('none.bin').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ).getXMP());\n\
         x.closeFile(); try { x.getXMP(); } catch (e) { out.push(e.message); } vc.log(out);",
    );
    assert_eq!(
        log[0],
        json!([
            "image/svg+xml",
            12,
            PACKET.len(),
            true,
            false,
            "XMPFile.putXMP: the file cannot be updated",
            null,
            "XMPFile.getXMP: the file is closed"
        ])
    );
    let _ = std::fs::remove_dir_all(base);
}

/// `getPacketInfo().padSize` gives the bytes of whitespace before the packet's trailer: twice the
/// characters in a UTF-16 packet, and 0 in a packet with none.
#[test]
fn packet_info_gives_the_padding() {
    let base = folder("pad");
    let padded = PACKET.replace("<?xpacket end=", " \n\t \n<?xpacket end=");
    write(&base.join("main/p.xmp"), padded.as_bytes());
    let mut le = vec![0xFF, 0xFE];
    le.extend(padded.encode_utf16().flat_map(u16::to_le_bytes));
    write(&base.join("main/le.xmp"), &le);
    write(&base.join("main/art.bin"), format!("\x00head{padded}\x00tail").as_bytes());
    write(&base.join("main/none.xmp"), PACKET.as_bytes());
    let mut s = Session::new();
    let log = logged(
        &mut s,
        &base,
        "vc.log(['p.xmp', 'le.xmp', 'art.bin', 'none.xmp'].map(function (n) {\n\
           var x = new XMPFile(File(n).fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); var p = x.getPacketInfo().padSize; x.closeFile(); return p;\n\
         }));",
    );
    assert_eq!(log[0], json!([5, 10, 5, 0]));
    let _ = std::fs::remove_dir_all(base);
}

/// A sidecar opened for update with packet scanning is read only, like any scanned file.
#[test]
fn a_scanned_sidecar_is_read_only() {
    let base = folder("scansidecar");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!(
            "var p = Folder.userData.fsName + '/s.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close();\n\
             var x = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE | XMPConst.OPEN_USE_PACKET_SCANNING); var m = x.getXMP();\n\
             var out = [m.getProperty(XMPConst.NS_DC, 'format').value, x.canPutXMP(m)];\n\
             try {{ x.putXMP(m); out.push('put'); }} catch (e) {{ out.push(e.message); }} x.closeFile(); vc.log(out);"
        ),
    );
    assert_eq!(log[0], json!(["image/svg+xml", false, "XMPFile.putXMP: the file cannot be updated"]));
    let _ = std::fs::remove_dir_all(base);
}

/// The state an `XMPFile` keeps cannot be changed by the script: a sidecar opened for reading is
/// not written by `closeFile` after the script assigns, defines or deletes the properties that
/// name the packet, the path and the open flags, or calls `closeFile` on another object that holds
/// properties of the same names, plain or made from `XMPFile.prototype`.
#[test]
fn a_script_cannot_change_the_state_of_an_xmp_file() {
    let base = folder("state");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!(
            "var p = Folder.userData.fsName + '/r.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close();\n\
             var x = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); var out = [];\n\
             x.__vcXmpPending = 'not a packet'; x.__vcXmpFlags = XMPConst.OPEN_FOR_UPDATE; x.__vcXmpPath = p + '.other';\n\
             try {{ Object.defineProperty(x, '__vcXmpPending', {{ value: 'not a packet' }}); out.push('defined'); }} catch (e) {{ out.push(e.name); }}\n\
             out.push(typeof x.__vcXmpPending, x.getFileInfo().openFlags == XMPConst.OPEN_FOR_READ);\n\
             [{{}}, Object.create(XMPFile.prototype)].forEach(function (fake) {{\n\
               fake.__vcXmpPath = p; fake.__vcXmpRun = x.__vcXmpRun; fake.__vcXmpOpen = true; fake.__vcXmpPending = 'not a packet';\n\
               try {{ XMPFile.prototype.closeFile.call(fake); out.push('closed'); }} catch (e) {{ out.push(e.name); }}\n\
             }});\n\
             x.closeFile(); out.push(delete x.__vcXmpPath); vc.log(out); vc.log(p);"
        ),
    );
    assert_eq!(log[0], json!(["defined", "string", true, "TypeError", "TypeError", true]));
    let text = std::fs::read_to_string(log[1].as_str().unwrap()).unwrap();
    assert_eq!(text, PACKET);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn xmp_file_buffers_are_not_script_properties() {
    let base = folder("native-packet");
    let mut session = with_data(&base);
    let log = logged(
        &mut session,
        &base,
        &format!(
            "var p = Folder.userData.fsName + '/native.xmp', file = new File(p);\n\
             file.open('w'); file.write({PACKET:?}); file.close();\n\
             var x = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE), m = x.getXMP();\n\
             m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m);\n\
             vc.log([typeof x.__vcXmpPacket, typeof x.__vcXmpPending, typeof x.__vcXmpPath,\n\
                     Object.getOwnPropertyNames(x).filter(function (name) {{ return name.indexOf('__vcXmp') === 0; }}).length,\n\
                     x.getXMP().getProperty(XMPConst.NS_DC, 'format').value]); x.closeFile();"
        ),
    );
    assert_eq!(log[0], json!(["undefined", "undefined", "undefined", 0, "changed"]));
}

/// A path outside the script's places, a missing file and a file opened for update where the run
/// may not write are refused with the file access errors, and a malformed or huge packet with an
/// error, never a fault.
#[test]
fn refused_paths_and_bad_packets_throw() {
    let base = folder("refused");
    let outside = folder("refused-outside");
    write(&outside.join("x.xmp"), PACKET.as_bytes());
    write(&base.join("main/bad.xmp"), b"<x:xmpmeta><rdf:RDF");
    write(&base.join("main/ok.xmp"), PACKET.as_bytes());
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!(
            "function t(f) {{ try {{ f(); return 'no error'; }} catch (e) {{ return e.name + ': ' + e.message; }} }}\n\
             vc.log([t(function () {{ new XMPFile({:?}, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); }}),\n\
             t(function () {{ new XMPFile(File('missing.xmp').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); }}),\n\
             t(function () {{ new XMPFile(File('ok.xmp').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE); }}),\n\
             t(function () {{ new XMPFile(File('bad.xmp').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ).getXMP(); }}),\n\
             t(function () {{ new XMPFile(File('ok.xmp').fsName, XMPConst.FILE_UNKNOWN, 0x100); }}),\n\
             t(function () {{ var u = new XMPFile(Folder.userData.fsName + '/n.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE); }})]);",
            outside.join("x.xmp").to_string_lossy()
        ),
    );
    assert_eq!(
        log[0],
        json!([
            "Error: Script file access denied",
            "Error: Script file does not exist",
            "Error: Script file access denied",
            "Error: XMPFile.getXMP: the XMP packet is not well-formed XMP",
            "RangeError: XMPFile: the value 0x100 is not available yet",
            "Error: Script file does not exist"
        ])
    );
    let _ = std::fs::remove_dir_all(base);
    let _ = std::fs::remove_dir_all(outside);
}

/// A sidecar whose write fails is left as it was and is not named in `written`.
#[test]
fn a_failed_write_is_not_listed_as_written() {
    let base = folder("unwritten");
    let mut s = with_data(&base);
    logged(&mut s, &base, &format!("var f = new File(Folder.userData.fsName + '/u.xmp'); f.open('w'); f.write({PACKET:?}); f.close();"));
    crate::script::files::XMP_WRITE_FAILS.with(|f| f.set(true));
    let r = run_in(
        &mut s,
        &base,
        "var x = new XMPFile(Folder.userData.fsName + '/u.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
         var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m);\n\
         try { x.closeFile(); } catch (e) { vc.log(e.message); }",
    );
    crate::script::files::XMP_WRITE_FAILS.with(|f| f.set(false));
    let r = r.unwrap();
    assert_eq!(r["log"], json!(["Script file could not be written"]));
    assert_eq!(r["written"], json!([]), "a file the run did not change is not listed");
    let _ = std::fs::remove_dir_all(base);
}

/// A sidecar that an open `File` has changed and not yet written is not written over: the `File`
/// keeps its changes and writes them when it closes.
#[test]
fn unwritten_file_changes_are_not_written_over() {
    let base = folder("pending");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!(
            "var p = Folder.userData.fsName + '/w.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close();\n\
             var x = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
             var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m);\n\
             f.open('e'); f.seek(0, 2); f.write('<!-- kept -->');\n\
             function text() {{ var g = new File(p); g.open('r'); var t = g.read(); g.close(); return t; }}\n\
             var out = []; try {{ x.closeFile(); }} catch (e) {{ out.push(e.message); }} f.close(); out.push(text().indexOf('kept') > 0);\n\
             x.closeFile(); out.push(text().indexOf('changed') > 0); vc.log(out);"
        ),
    );
    assert_eq!(log[0], json!(["Script file could not be written", true, true]), "the File's changes are written, then the packet");
    let _ = std::fs::remove_dir_all(base);
}

/// A file with the name of the temporary file is left alone: the packet goes through another name
/// and replaces only the sidecar.
#[test]
fn a_file_in_the_temporary_name_is_left_alone() {
    let base = folder("taken");
    let mut s = with_data(&base);
    let path = logged(
        &mut s,
        &base,
        &format!("var f = new File(Folder.userData.fsName + '/t.xmp'); f.open('w'); f.write({PACKET:?}); f.close(); vc.log(f.fsName);"),
    );
    let path = PathBuf::from(path[0].as_str().unwrap());
    let taken = path.with_file_name(format!(".t.xmp.{}.0.tmp", std::process::id()));
    write(&taken, b"someone else's");
    let log = logged(
        &mut s,
        &base,
        "var x = new XMPFile(Folder.userData.fsName + '/t.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
         var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m); x.closeFile(); vc.log('closed');",
    );
    assert_eq!(log, json!(["closed"]));
    assert_eq!(std::fs::read(&taken).unwrap(), b"someone else's");
    assert!(std::fs::read_to_string(&path).unwrap().contains("changed"));
    let names: Vec<String> =
        std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.iter().filter(|n| n.ends_with(".tmp")).count(), 1, "no temporary file is left behind: {names:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// Pumps the run until it waits for a file dialog (`None`) or ends.
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

/// Starts `main/main.jsx` in `base` with the XMP library loaded and the text `text`, in the app.
fn start_framed(s: &mut Session, base: &Path, text: &str) {
    s.script_frames = true;
    let main = base.join("main/main.jsx");
    write(&main, format!("{LOAD}{text}").as_bytes());
    assert_eq!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap()["running"], true);
}

/// With only the sidecar and the first temporary name granted, a taken first name does not make
/// the packet go to another name outside the grants: `closeFile()` throws and no file is made.
#[test]
fn temporary_names_outside_the_grants_are_not_made() {
    let base = folder("tempgrant");
    let (sidecar, taken) = (base.join("picked/t.xmp"), base.join(format!("picked/.t.xmp.{}.0.tmp", std::process::id())));
    write(&sidecar, PACKET.as_bytes());
    write(&taken, b"someone else's");
    let mut s = with_data(&base);
    start_framed(
        &mut s,
        &base,
        "var a = File.saveDialog('a'), b = File.saveDialog('b');\n\
         var x = new XMPFile(a.fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
         var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m);\n\
         try { x.closeFile(); vc.log('closed'); } catch (e) { vc.log(e.message); }",
    );
    for path in [&sidecar, &taken] {
        assert!(pump_to_picker(&mut s).is_none());
        assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![path.to_string_lossy().into_owned()], always: false }));
    }
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!(["Script file could not be written"]));
    assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), PACKET);
    assert_eq!(std::fs::read(&taken).unwrap(), b"someone else's");
    let names: Vec<String> = std::fs::read_dir(base.join("picked")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.len(), 2, "no other file is made: {names:?}");
    let _ = std::fs::remove_dir_all(base);
}

/// A `.xmp` link to another kind of file is scanned and takes no packet, and a link that leads to
/// another kind of file by the time of `closeFile()` is not written through.
#[cfg(unix)]
#[test]
fn a_link_to_another_kind_of_file_is_not_written() {
    let base = folder("link");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!(
            "var d = Folder.userData.fsName, l = d + '/l.xmp';\n\
             function put(n) {{ var f = new File(d + '/' + n); f.open('w'); f.write({PACKET:?}); f.close(); }}\n\
             function link(n) {{ var f = new File(l); if (f.exists) f.remove(); return !!new File(l).createAlias(d + '/' + n); }}\n\
             put('other.txt'); put('real.xmp'); var out = [link('other.txt')];\n\
             var a = new XMPFile(l, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE); out.push(a.canPutXMP(a.getXMP())); a.closeFile();\n\
             out.push(link('real.xmp'));\n\
             var x = new XMPFile(l, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE); var m = x.getXMP();\n\
             m.setProperty(XMPConst.NS_DC, 'format', 'changed'); out.push(x.canPutXMP(m)); x.putXMP(m);\n\
             out.push(link('other.txt'));\n\
             try {{ x.closeFile(); out.push('closed'); }} catch (e) {{ out.push(e.message); }}\n\
             vc.log(out); vc.log(d);"
        ),
    );
    assert_eq!(log[0], json!([true, false, true, true, true, "Script file access denied"]));
    let d = PathBuf::from(log[1].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(d.join("other.txt")).unwrap(), PACKET);
    assert_eq!(std::fs::read_to_string(d.join("real.xmp")).unwrap(), PACKET);
    let _ = std::fs::remove_dir_all(base);
}

/// A sidecar replaced through a temporary file keeps its permission bits.
#[cfg(unix)]
#[test]
fn a_replaced_sidecar_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let base = folder("mode");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!("var p = Folder.userData.fsName + '/a.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close(); vc.log(p);"),
    );
    let path = PathBuf::from(log[0].as_str().unwrap());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    logged(
        &mut s,
        &base,
        "var x = new XMPFile(Folder.userData.fsName + '/a.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
         var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m); x.closeFile();",
    );
    assert!(std::fs::read_to_string(&path).unwrap().contains("changed"));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let _ = std::fs::remove_dir_all(base);
}

/// A sidecar in UTF-16 is read by its byte order mark, and one that is not text is refused.
#[test]
fn a_utf16_sidecar_reads_by_its_byte_order_mark() {
    let base = folder("utf16");
    let mut le = vec![0xFF, 0xFE];
    le.extend(PACKET.encode_utf16().flat_map(u16::to_le_bytes));
    write(&base.join("main/le.xmp"), &le);
    write(&base.join("main/bad.xmp"), &[0xFF, 0xFE, 0x00]);
    let mut s = Session::new();
    let log = logged(
        &mut s,
        &base,
        "var x = new XMPFile(File('le.xmp').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ);\n\
         var out = [x.getXMP().getProperty(XMPConst.NS_DC, 'format').value, x.getPacketInfo().charForm];\n\
         try { new XMPFile(File('bad.xmp').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); } catch (e) { out.push(e.message); } vc.log(out);",
    );
    assert_eq!(log[0], json!(["image/svg+xml", 3, "Script file cannot be decoded"]));
    let _ = std::fs::remove_dir_all(base);
}

/// A `File` open on a UTF-16 sidecar reads the UTF-8 packet `closeFile()` wrote, from the start.
#[test]
fn an_open_utf16_file_reads_the_written_packet() {
    let base = folder("utf16open");
    let mut s = with_data(&base);
    let d = logged(&mut s, &base, "vc.log(Folder.userData.fsName);");
    let d = PathBuf::from(d[0].as_str().unwrap());
    let mut le = vec![0xFF, 0xFE];
    le.extend(PACKET.encode_utf16().flat_map(u16::to_le_bytes));
    write(&d.join("u.xmp"), &le);
    let log = logged(
        &mut s,
        &base,
        "var p = Folder.userData.fsName + '/u.xmp'; var f = new File(p); f.open('r'); var out = [f.encoding, f.read(5)];\n\
         var x = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE); var m = x.getXMP();\n\
         m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m); x.closeFile();\n\
         var t = f.read(), e = f.encoding; f.close(); out.push(e, t.slice(0, 9), t.indexOf('<dc:format>changed</dc:format>') > 0); vc.log(out);",
    );
    assert_eq!(log[0], json!(["UTF-16LE", "<?xpa", "UTF-8", "<?xpacket", true]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn writing_a_utf16_declared_packet_keeps_its_body_in_utf8() {
    let base = folder("utf16declaredwrite");
    let mut session = with_data(&base);
    let data = logged(&mut session, &base, "vc.log(Folder.userData.fsName);");
    let data = PathBuf::from(data[0].as_str().unwrap());
    let body = format!("<!-- retained prolog -->\n{PACKET}\n<!-- retained epilog -->");
    let packet = format!("<?xml version='1.0' encoding='UTF-16'?>{body}");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(packet.encode_utf16().flat_map(u16::to_le_bytes));
    write(&data.join("declared.xmp"), &bytes);
    logged(
        &mut session,
        &base,
        "var x = new XMPFile(Folder.userData.fsName + '/declared.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE); var packet = x.getPacketInfo().packet; x.putXMP(packet); x.closeFile();",
    );
    let written = std::fs::read_to_string(data.join("declared.xmp")).unwrap();
    assert_eq!(written, body);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn an_xmp_update_resets_bom_readers_and_keeps_unmarked_reader_positions() {
    for bom in [true, false] {
        let base = folder(if bom { "utf8bomopen" } else { "utf8open" });
        let mut session = with_data(&base);
        let data = logged(&mut session, &base, "vc.log(Folder.userData.fsName);");
        let data = PathBuf::from(data[0].as_str().unwrap());
        let mut bytes = if bom { vec![0xEF, 0xBB, 0xBF] } else { Vec::new() };
        bytes.extend_from_slice(PACKET.as_bytes());
        write(&data.join("u.xmp"), &bytes);
        let log = logged(
            &mut session,
            &base,
            &format!(
                "var p = Folder.userData.fsName + '/u.xmp', f = new File(p); f.open('r');\n\
                 if (!{bom}) f.read(3); var before = f.tell();\n\
                 var x = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE), m = x.getXMP();\n\
                 m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m); x.closeFile();\n\
                 var after = f.tell(), text = f.read(); f.close(); vc.log([before, after, text.slice(0, 9)]);"
            ),
        );
        assert_eq!(log[0], if bom { json!([3, 0, "<?xpacket"]) } else { json!([3, 3, "packet be"]) });
        let _ = std::fs::remove_dir_all(base);
    }
}

/// Opens that cannot take a packet need no write grant, even with `OPEN_FOR_UPDATE`: a scanned
/// sidecar and another kind of file in the script's own folder open read only.
#[test]
fn read_only_opens_need_no_write_grant() {
    let base = folder("readonly");
    write(&base.join("main/ok.xmp"), PACKET.as_bytes());
    write(&base.join("main/art.txt"), PACKET.as_bytes());
    let mut s = Session::new();
    let log = logged(
        &mut s,
        &base,
        "var u = XMPConst.OPEN_FOR_UPDATE, out = [];\n\
         [[File('ok.xmp').fsName, u | XMPConst.OPEN_USE_PACKET_SCANNING], [File('art.txt').fsName, u]].forEach(function (a) {\n\
           var x = new XMPFile(a[0], XMPConst.FILE_UNKNOWN, a[1]); var m = x.getXMP();\n\
           out.push(m.getProperty(XMPConst.NS_DC, 'format').value, x.canPutXMP(m)); x.closeFile();\n\
         }); vc.log(out);",
    );
    assert_eq!(log[0], json!(["image/svg+xml", false, "image/svg+xml", false]));
    let _ = std::fs::remove_dir_all(base);
}

/// A sidecar larger than a script may read is refused before it is read.
#[test]
fn a_huge_sidecar_is_refused_before_it_is_read() {
    let base = folder("huge");
    let file = std::fs::File::create(base.join("main/big.xmp")).unwrap();
    file.set_len(9 << 20).unwrap();
    drop(file);
    let mut s = Session::new();
    let log = logged(
        &mut s,
        &base,
        "try { new XMPFile(File('big.xmp').fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ); vc.log('read'); } catch (e) { vc.log(e.message); }",
    );
    assert_eq!(log, json!(["Script file exceeds 8 MiB"]));
    let _ = std::fs::remove_dir_all(base);
}

/// The file named `name` in `dir` or a folder inside it.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if entry.file_name() == name {
            return Some(path);
        }
    }
    None
}

/// A write that fails leaves the sidecar as it was and the `XMPFile` open with its packet, so a
/// later `closeFile` of the same object writes it.
#[test]
fn a_failed_write_keeps_the_packet_for_a_later_close() {
    let base = folder("fails");
    let mut s = with_data(&base);
    crate::script::files::XMP_WRITE_FAILS.with(|f| f.set(true));
    start_framed(
        &mut s,
        &base,
        &format!(
            "var p = Folder.userData.fsName + '/k.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close();\n\
             var kept = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
             var m = kept.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); kept.putXMP(m);\n\
             var out = []; try {{ kept.closeFile(); out.push('closed'); }} catch (e) {{ out.push(e.message, kept.getXMP().getProperty(XMPConst.NS_DC, 'format').value); }}\n\
             File.saveDialog('pause'); kept.closeFile(); out.push('closed'); vc.log(out); vc.log(p);"
        ),
    );
    let paused = pump_to_picker(&mut s);
    crate::script::files::XMP_WRITE_FAILS.with(|f| f.set(false));
    assert!(paused.is_none(), "the run waits at the dialog: {paused:?}");
    let path = find_file(&base.join("settings"), "k.xmp").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), PACKET, "the failed write leaves the file as it was");
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"][0], json!(["Script file could not be written", "changed", "closed"]));
    assert!(std::fs::read_to_string(&path).unwrap().contains("<dc:format>changed</dc:format>"), "the same XMPFile writes on its next close");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn a_directory_flush_failure_keeps_the_written_packet_and_allows_retry() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            crate::script::files::XMP_DIRECTORY_SYNC_FAILS.with(|flag| flag.set(false));
        }
    }
    let _reset = Reset;
    let base = folder("directory-flush");
    let mut session = with_data(&base);
    let dir = logged(&mut session, &base, "vc.log(Folder.userData.fsName);");
    let path = PathBuf::from(dir[0].as_str().unwrap()).join("packet.xmp");
    write(&path, format!("\u{FEFF}{PACKET}").as_bytes());
    crate::script::files::XMP_DIRECTORY_SYNC_FAILS.with(|flag| flag.set(true));
    start_framed(
        &mut session,
        &base,
        &format!(
            "Folder.userData; var p = {:?}, reader = new File(p); reader.open('r'); reader.read(7);\n\
             var kept = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
             var meta = kept.getXMP(); meta.setProperty(XMPConst.NS_DC, 'format', 'changed'); kept.putXMP(meta);\n\
             var out = []; try {{ kept.closeFile(); out.push('closed'); }} catch (e) {{ out.push(e.message, kept.getXMP().getProperty(XMPConst.NS_DC, 'format').value); }}\n\
             out.push(reader.tell() === 0, reader.read().indexOf('changed') >= 0); reader.close();\n\
             File.saveDialog('pause'); kept.closeFile(); out.push('closed'); vc.log(out);",
            path.to_string_lossy()
        ),
    );
    let paused = pump_to_picker(&mut session);
    crate::script::files::XMP_DIRECTORY_SYNC_FAILS.with(|flag| flag.set(false));
    assert!(paused.is_none(), "the run waits before retrying: {paused:?}");
    assert!(std::fs::read_to_string(&path).unwrap().contains("<dc:format>changed</dc:format>"), "the replacement already wrote the packet");
    assert!(session.script_pick(crate::script::files::PickAnswer::Deny));
    let report = pump_to_picker(&mut session).unwrap().unwrap();
    assert_eq!(report["log"][0], json!(["Script file could not be written", "changed", true, true, "closed"]));
    assert_eq!(report["written"], json!([path.to_string_lossy()]), "the written replacement is reported once");
}

/// An `XMPFile` from an earlier run of a named engine is closed.
#[test]
fn an_xmp_file_from_an_earlier_run_is_closed() {
    let base = folder("earlier");
    let mut s = with_data(&base);
    let engine = |text: &str| format!("#targetengine xmp\n{LOAD}{text}");
    let main = base.join("main/main.jsx");
    write(&main, engine(&format!("var f = new File(Folder.userData.fsName + '/k2.xmp'); f.open('w'); f.write({PACKET:?}); f.close();")).as_bytes());
    s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap();
    write(
        &main,
        engine("$.global.kept = new XMPFile(Folder.userData.fsName + '/k2.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);").as_bytes(),
    );
    s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap();
    write(&main, engine("try { kept.getXMP(); vc.log('open'); } catch (e) { vc.log(e.message); }").as_bytes());
    let r = s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap();
    assert_eq!(r["log"], json!(["XMPFile.getXMP: the file is closed"]), "an XMPFile from an earlier run is closed");
    let _ = std::fs::remove_dir_all(base);
}

/// The temporary file a sidecar is replaced through is readable by its owner alone while the
/// packet is written to it, and the sidecar keeps its own permission bits.
#[cfg(unix)]
#[test]
fn a_temporary_file_starts_private() {
    use std::os::unix::fs::PermissionsExt;
    let base = folder("private");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!("var p = Folder.userData.fsName + '/a.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close(); vc.log(p);"),
    );
    let path = PathBuf::from(log[0].as_str().unwrap());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    crate::script::files::XMP_TEMP_MODE.with(|m| m.set(None));
    logged(
        &mut s,
        &base,
        "var x = new XMPFile(Folder.userData.fsName + '/a.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
         var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m); x.closeFile();",
    );
    assert_eq!(crate::script::files::XMP_TEMP_MODE.with(std::cell::Cell::get).map(|m| m & 0o777), Some(0o600));
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);
    let _ = std::fs::remove_dir_all(base);
}

/// A replaced sidecar that had no access ACL takes none from its folder's default ACL, whose named
/// users the sidecar's group bits would let in. (Skipped where `setfacl` or ACLs are missing.)
#[cfg(target_os = "linux")]
#[test]
fn a_replaced_sidecar_takes_no_inherited_acl() {
    let base = folder("acl");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!("var p = Folder.userData.fsName + '/a.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close(); vc.log(p);"),
    );
    let path = PathBuf::from(log[0].as_str().unwrap());
    let set = std::process::Command::new("setfacl").args(["-d", "-m", "u:nobody:rw"]).arg(path.parent().unwrap()).status();
    if !set.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(base);
        return;
    }
    logged(
        &mut s,
        &base,
        "var x = new XMPFile(Folder.userData.fsName + '/a.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
         var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m); x.closeFile();",
    );
    assert!(std::fs::read_to_string(&path).unwrap().contains("<dc:format>changed</dc:format>"));
    let acl = std::process::Command::new("getfacl").args(["--omit-header", "--absolute-names"]).arg(&path).output().unwrap();
    let acl = String::from_utf8_lossy(&acl.stdout);
    assert!(!acl.contains("nobody"), "the replacement takes no ACL entry the sidecar lacked: {acl}");
    let _ = std::fs::remove_dir_all(base);
}

/// A packet argument whose conversion closes the `XMPFile` leaves it closed: `canPutXMP` and
/// `putXMP` throw that the file is closed, and nothing is written.
#[test]
fn converting_the_packet_cannot_close_the_file() {
    let base = folder("reentrant");
    let mut s = with_data(&base);
    let log = logged(
        &mut s,
        &base,
        &format!(
            "var p = Folder.userData.fsName + '/r.xmp'; var f = new File(p); f.open('w'); f.write({PACKET:?}); f.close();\n\
             function closing(x) {{ var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); var s = m.serialize();\n\
               return {{ toString: function () {{ x.closeFile(); return s; }} }}; }}\n\
             var out = [], a = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
             try {{ out.push(a.canPutXMP(closing(a))); }} catch (e) {{ out.push(e.message); }}\n\
             var b = new XMPFile(p, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
             try {{ b.putXMP(closing(b)); out.push('put'); }} catch (e) {{ out.push(e.message); }}\n\
             b.closeFile(); vc.log(out); vc.log(p);"
        ),
    );
    assert_eq!(log[0], json!(["XMPFile.canPutXMP: the file is closed", "XMPFile.putXMP: the file is closed"]));
    assert_eq!(std::fs::read_to_string(log[1].as_str().unwrap()).unwrap(), PACKET);
    let _ = std::fs::remove_dir_all(base);
}

/// With `CLOSE_UPDATE_SAFELY`, a sidecar whose folder takes no temporary file is left as it is and
/// the `XMPFile` keeps its packet; a later `closeFile()` without the flag writes the file itself.
#[test]
fn a_safe_close_writes_only_through_a_temporary_file() {
    let base = folder("safely");
    let sidecar = base.join("picked/s.xmp");
    write(&sidecar, PACKET.as_bytes());
    let mut s = with_data(&base);
    start_framed(
        &mut s,
        &base,
        "var a = File.saveDialog('a');\n\
         var x = new XMPFile(a.fsName, XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_UPDATE);\n\
         var m = x.getXMP(); m.setProperty(XMPConst.NS_DC, 'format', 'changed'); x.putXMP(m);\n\
         var out = []; try { x.closeFile(XMPConst.CLOSE_UPDATE_SAFELY); out.push('closed'); } catch (e) { out.push(e.message, x.getXMP().getProperty(XMPConst.NS_DC, 'format').value); }\n\
         File.saveDialog('pause'); x.closeFile(); out.push('written'); vc.log(out);",
    );
    assert!(pump_to_picker(&mut s).is_none());
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![sidecar.to_string_lossy().into_owned()], always: false }));
    assert!(pump_to_picker(&mut s).is_none());
    assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), PACKET, "the safe close leaves the file as it was");
    assert!(s.script_pick(crate::script::files::PickAnswer::Deny));
    let r = pump_to_picker(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([["Script file could not be written", "changed", "written"]]));
    assert!(std::fs::read_to_string(&sidecar).unwrap().contains("<dc:format>changed</dc:format>"));
    let names: Vec<String> = std::fs::read_dir(base.join("picked")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["s.xmp"], "no other file is made");
    let _ = std::fs::remove_dir_all(base);
}
