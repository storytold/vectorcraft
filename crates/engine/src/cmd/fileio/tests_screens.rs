//! Export for Screens: sizes (factor, width, height, resolution), presets and sub-folders, Full
//! Document, Include Bleed, per-format settings, ZIP downloads and the settings the document keeps.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::screens::ScreenSize;
use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A document of `artboards` 40×30 pt artboards side by side with a 20 pt square on the first.
fn session(artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 30, "artboards": artboards})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 20})).unwrap();
    s
}

fn export(s: &mut Session, p: Value) -> Value {
    s.execute("document.exportForScreens", &p).unwrap()
}

/// `(name, bytes)` of each returned file.
fn files(r: &Value) -> Vec<(String, Vec<u8>)> {
    r["files"].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_string(), b64(f))).collect()
}

fn names(r: &Value) -> Vec<String> {
    files(r).into_iter().map(|(n, _)| n).collect()
}

fn size(png: &[u8]) -> (u32, u32) {
    image::load_from_memory(png).unwrap().to_rgba8().dimensions()
}

#[test]
fn sizes_parse_as_factor_width_height_or_resolution() {
    assert_eq!(ScreenSize::parse("2x"), Some(ScreenSize::Scale(2.0)));
    assert_eq!(ScreenSize::parse(" 0.5 "), Some(ScreenSize::Scale(0.5)));
    assert_eq!(ScreenSize::parse("100W"), Some(ScreenSize::Width(100.0)));
    assert_eq!(ScreenSize::parse("64h"), Some(ScreenSize::Height(64.0)));
    assert_eq!(ScreenSize::parse("144ppi"), Some(ScreenSize::Ppi(144.0)));
    for bad in ["", "x", "-2x", "0w", "NaNx", "infh", "2y"] {
        assert_eq!(ScreenSize::parse(bad), None, "{bad}");
    }
    assert_eq!((ScreenSize::Scale(1.0).suffix(), ScreenSize::Scale(1.5).suffix()), (String::new(), "@1.5x".into()));
    assert_eq!(
        (ScreenSize::Width(100.0).suffix(), ScreenSize::Ppi(144.0).suffix(), ScreenSize::Ppi(144.0).label()),
        ("@100w".into(), "@2x".into(), "2x".into())
    );
}

#[test]
fn a_width_or_height_row_gives_that_many_pixels() {
    let mut s = session(1);
    let r = export(
        &mut s,
        json!({"formats": [{"format": "png", "scale": "100w"}, {"format": "png", "height": 60}, {"format": "jpg", "scale": "80ppi"}]}),
    );
    assert_eq!(names(&r), ["Artboard-1@100w.png", "Artboard-1@60h.png", "Artboard-1@1.111x.jpg"]);
    let f = files(&r);
    assert_eq!(size(&f[0].1), (100, 75));
    assert_eq!(size(&f[1].1), (80, 60));
    assert_eq!(size(&f[2].1), (44, 33));
    // Width or height win over scale, and a bad size is refused.
    let r = export(&mut s, json!({"formats": [{"format": "png", "scale": 3, "width": 20}]}));
    assert_eq!(size(&files(&r)[0].1), (20, 15));
    for bad in [json!({"scale": "big"}), json!({"width": -5}), json!({"height": "tall"})] {
        assert!(s.execute("document.exportForScreens", &json!({"formats": [bad]})).is_err(), "{bad}");
    }
    // Vector rows ignore sizes and their suffixes.
    let r = export(&mut s, json!({"formats": [{"format": "svg", "scale": "100w", "suffix": "@100w"}]}));
    assert_eq!(names(&r), ["Artboard-1.svg"]);
}

#[test]
fn density_buckets_go_into_sub_folders() {
    let mut s = session(2);
    let r = export(&mut s, json!({"preset": "density", "range": "1"}));
    assert_eq!(
        names(&r),
        [
            "ldpi/Artboard-1.png",
            "mdpi/Artboard-1.png",
            "hdpi/Artboard-1.png",
            "xhdpi/Artboard-1.png",
            "xxhdpi/Artboard-1.png",
            "xxxhdpi/Artboard-1.png"
        ]
    );
    let widths: Vec<u32> = files(&r).iter().map(|(_, b)| size(b).0).collect();
    assert_eq!(widths, [30, 40, 60, 80, 120, 160]);
    let r = export(&mut s, json!({"preset": "mobile", "artboards": [1]}));
    assert_eq!(names(&r), ["1x/Artboard-2.png", "2x/Artboard-2@2x.png", "3x/Artboard-2@3x.png"]);
    assert!(s.execute("document.exportForScreens", &json!({"preset": "mobile", "formats": [{}]})).is_err(), "a preset or formats");
    assert!(s.execute("document.exportForScreens", &json!({"preset": "tablet"})).is_err());
    // Sub-folders by size for raster rows, by format for vector rows, or the row's own (made safe).
    let r = export(
        &mut s,
        json!({"range": "2", "subfolders": true, "formats": [{"format": "png", "scale": 2}, {"format": "svg"}, {"format": "pdf", "folder": "../print"}]}),
    );
    assert_eq!(names(&r), ["2x/Artboard-2@2x.png", "SVG/Artboard-2.svg", "..-print/Artboard-2.pdf"]);
    // A prefix names files, not folders.
    let r = export(&mut s, json!({"range": "2", "prefix": "../a\\b:", "formats": [{"format": "svg"}]}));
    assert_eq!(names(&r), ["..-a-b-Artboard-2.svg"]);
    // So does a suffix: it can't open a folder or climb out of the export folder.
    let r = export(&mut s, json!({"range": "2", "formats": [{"format": "svg", "suffix": "/../x\\..\\y"}, {"format": "png", "suffix": "-ok"}]}));
    assert_eq!(names(&r), ["Artboard-2-..-x-..-y.svg", "Artboard-2-ok.png"]);
}

/// Windows does not allow `<`, `>`, `"`, `|`, `?`, `*` or control characters in a file name
/// (<https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file>): Export for Screens
/// writes them as `-` in a prefix and a suffix, on every platform.
#[test]
fn a_prefix_or_suffix_writes_the_characters_windows_does_not_allow_as_dashes() {
    let mut s = session(1);
    let r = export(&mut s, json!({"prefix": "a<b>c\"", "formats": [{"format": "svg", "suffix": "|d?e*f\u{1}\t"}]}));
    assert_eq!(names(&r), ["a-b-c-Artboard-1-d-e-f--.svg"]);
}

/// A file or folder named after a Windows device (`CON`, `NUL`, `COM1`…, alone or before an
/// extension) gets a `-` after the device name, and a folder loses a trailing period, on every
/// platform.
#[test]
fn device_names_and_trailing_periods_follow_windows_naming_rules() {
    let mut s = session(3);
    for (index, name) in ["CON", "CON-", "lpt1"].iter().enumerate() {
        s.execute("artboard.setProps", &json!({"index": index, "name": name})).unwrap();
    }
    let r = export(&mut s, json!({"formats": [{"format": "svg"}]}));
    assert_eq!(names(&r), ["CON-.svg", "CON--2.svg", "lpt1-.svg"]);
    let r =
        export(&mut s, json!({"range": "1", "subfolders": true, "formats": [{"format": "svg", "folder": "aux"}, {"format": "pdf", "folder": "x."}]}));
    assert_eq!(names(&r), ["aux-/CON-.svg", "x/CON-.pdf"]);
    let r = export(&mut s, json!({"range": "3", "folders": "item", "formats": [{"format": "svg"}]}));
    assert_eq!(names(&r), ["lpt1-/lpt1-.svg"]);
    // A name pattern, with an extension after the device name, and the document's own name.
    let r = export(&mut s, json!({"range": "1", "name": "<name>.tar", "formats": [{"format": "svg"}]}));
    assert_eq!(names(&r), ["CON-.tar.svg"]);
    s.execute("file.new", &json!({"width": 40, "height": 30, "name": "nul"})).unwrap();
    let r = export(&mut s, json!({"fullDocument": true, "folders": "item", "formats": [{"format": "pdf"}]}));
    assert_eq!(names(&r), ["nul-/nul-.pdf"]);
    let r = s.execute("document.exportForScreens", &json!({"zip": true, "formats": [{"format": "svg"}]})).unwrap();
    assert_eq!(r["name"], "nul-.zip");
}

/// `first_file` names the file the export writes first, for the same params: the item's file name,
/// a prefix and a suffix with characters Windows does not allow, a range, a Windows device name,
/// every kind of folder, presets, size suffixes, a name pattern with its counter and date, an
/// export preset, a ZIP, Full Document, Include Bleed, assets, layers and the selection; and it
/// refuses what the export refuses.
#[test]
fn the_first_file_is_the_file_the_export_writes_first() {
    let mut s = session(3);
    s.execute("artboard.setProps", &json!({"index": 1, "name": "CON"})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id.0;
    let square = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id.0;
    let asset = s.execute("assets.add", &json!({"ids": [square], "name": "Logo?"})).unwrap()["assets"][0].as_u64().unwrap();
    s.execute("shape.ellipse", &json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let written = |r: &Value| r["files"][0].as_str().or_else(|| r["files"][0]["name"].as_str()).unwrap_or_default().to_string();
    let cases = [
        json!({}),
        json!({"prefix": "a?b/", "formats": [{"format": "png", "suffix": "<x>"}]}),
        json!({"range": "2-3", "folders": "item"}),
        json!({"range": "2", "folders": "itemFormat", "formats": [{"format": "svg", "scale": "2x", "suffix": "@2x"}]}),
        json!({"preset": "mobile"}),
        json!({"preset": "density"}),
        json!({"subfolders": true, "formats": [{"format": "png", "scale": "144ppi", "folder": "a b"}]}),
        json!({"folders": "format", "formats": [{"format": "jpg", "quality": 50}]}),
        json!({
            "name": "<filename>-<name>-<scale>-<index>-###-YYYYMMDD",
            "counterStart": 7,
            "counterPer": "file",
            "date": "2026-10-17",
            "formats": [{"format": "png", "scale": "2x"}]
        }),
        json!({"name": "<suffix>", "formats": [{"format": "png", "suffix": ""}]}),
        json!({"exportPreset": "Logo Package"}),
        json!({"zip": true, "formats": [{"format": "svg"}, {"format": "png"}]}),
        json!({"fullDocument": true, "folders": "item", "formats": [{"format": "pdf"}]}),
        json!({"fullDocument": true, "formats": [{"format": "png"}]}),
        json!({"includeBleed": true}),
        json!({"assets": [asset], "formats": [{"format": "svg"}]}),
        json!({"layers": [layer], "folders": "item"}),
        json!({"selection": "one"}),
        json!({"selection": "each", "name": "<name>"}),
    ];
    for p in cases {
        let first = super::screens::first_file(&s, &p).unwrap();
        assert_eq!(first, written(&export(&mut s, p.clone())), "{p}");
    }
    for p in [json!({"layers": [layer], "assets": [asset]}), json!({"selection": "all"}), json!({"counterPer": "row"})] {
        assert!(super::screens::first_file(&s, &p).is_err() && s.execute("document.exportForScreens", &p).is_err(), "{p}");
    }
    // A document named after a Windows device.
    s.execute("file.new", &json!({"width": 40, "height": 30, "name": "nul"})).unwrap();
    for p in [json!({"fullDocument": true, "folders": "item", "formats": [{"format": "pdf"}]}), json!({"name": "<filename>"})] {
        let first = super::screens::first_file(&s, &p).unwrap();
        assert_eq!(first, written(&export(&mut s, p.clone())), "{p}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sub_folders_are_made_in_the_export_folder() {
    let mut s = session(1);
    let dir = std::env::temp_dir().join(format!("vc-screens-sub-{}", std::process::id()));
    let folder = dir.to_string_lossy().replace('\\', "/");
    let r = export(&mut s, json!({"folder": folder, "preset": "mobile"}));
    let written: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(written, [format!("{folder}/1x/Artboard-1.png"), format!("{folder}/2x/Artboard-1@2x.png"), format!("{folder}/3x/Artboard-1@3x.png")]);
    assert_eq!(size(&std::fs::read(dir.join("3x").join("Artboard-1@3x.png")).unwrap()), (120, 90));
    let _ = std::fs::remove_dir_all(dir);
}

/// #1713: whatever a suffix or prefix says (`..`, an absolute path, a drive, a share), every file
/// lands directly in the export folder.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_hostile_suffix_or_prefix_stays_in_the_export_folder() {
    let mut s = session(1);
    let top = std::env::temp_dir().join(format!("vc-screens-escape-{}", std::process::id()));
    let dir = top.join("out");
    let folder = dir.to_string_lossy().replace('\\', "/");
    for text in ["/../../up", "\\..\\up", "/etc/x", "C:\\Windows\\x", "\\\\server\\share\\x", "/..", ".."] {
        for key in ["suffix", "prefix"] {
            let mut p = json!({"folder": folder, "formats": [{"format": "svg"}]});
            if key == "suffix" {
                p["formats"][0][key] = json!(text);
            } else {
                p[key] = json!(text);
            }
            let r = export(&mut s, p);
            let written = std::path::PathBuf::from(r["files"][0].as_str().unwrap());
            assert!(written.is_file(), "{key} {text}");
            assert_eq!(written.parent().unwrap().canonicalize().unwrap(), dir.canonicalize().unwrap(), "{key} {text}");
        }
    }
    // Nothing beside the export folder, no folder inside it.
    assert_eq!(std::fs::read_dir(&top).unwrap().count(), 1);
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| e.unwrap().path().is_file()));
    let _ = std::fs::remove_dir_all(top);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_zip_is_written_into_the_folder() {
    let mut s = session(1);
    let dir = std::env::temp_dir().join(format!("vc-screens-zip-{}", std::process::id()));
    let folder = dir.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let r = export(&mut s, json!({"folder": folder, "zip": true, "formats": [{"format": "png"}, {"format": "svg"}]}));
    let zip = std::fs::read(r["path"].as_str().unwrap()).unwrap();
    assert_eq!(super::zip::entries(&zip).unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// The reply of a prepared export's [`super::ScreenExport::write`] and the files it gave the
/// writer, by path.
fn write_prepared(export: super::ScreenExport) -> (Value, BTreeMap<String, Vec<u8>>) {
    let mut files = BTreeMap::new();
    let mut write = |path: &str, bytes: &[u8]| {
        files.insert(path.to_string(), bytes.to_vec());
        Ok(())
    };
    let r = export.write(&mut write).unwrap_or_else(|e| panic!("{e}"));
    (r, files)
}

/// The files reply `r` names (its `files`, else the zip at its `path`) as they are on disk, by path.
fn on_disk(r: &Value) -> BTreeMap<String, Vec<u8>> {
    let paths: Vec<&str> = match r["path"].as_str() {
        Some(zip) => vec![zip],
        None => r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect(),
    };
    paths.into_iter().map(|p| (p.to_string(), std::fs::read(p).unwrap())).collect()
}

/// [`Session::prepare_screen_export`] runs `document.exportForScreens` and `assets.export` as
/// commands, journaled, and writes nothing; the export it gives writes the files the command
/// writes, with the same names (`-2` included), sub-folders, bytes and reply, into a folder, as a
/// zip in it and without a folder, through the writer it is given. Export for Screens remembers
/// its settings when it is prepared.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_prepared_export_writes_what_the_command_writes() {
    let mut s = session(2);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Icon"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "icon"})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-screens-prepared-{}", std::process::id()));
    let folder = dir.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let rows = json!([{"format": "png", "suffix": ""}, {"format": "png", "scale": "2x"}, {"format": "svg"}]);
    for zip in [false, true] {
        let p = json!({"folder": folder, "zip": zip, "folders": "rows", "name": "<name>", "formats": rows});
        let want = export(&mut s, p.clone());
        if !zip {
            let names = ["1x/Icon.png", "2x/Icon@2x.png", "SVG/Icon.svg", "1x/icon-2.png", "2x/icon@2x-2.png", "SVG/icon-2.svg"];
            assert_eq!(want["files"], json!(names.map(|n| format!("{folder}/{n}"))));
        }
        let disk = on_disk(&want);
        std::fs::remove_dir_all(&dir).unwrap();
        let journaled = s.journal.len();
        let prepared = s.prepare_screen_export("document.exportForScreens", &p).unwrap();
        assert_eq!((s.journal.len(), s.journal.last().map(|(id, _)| id.as_str())), (journaled + 1, Some("document.exportForScreens")));
        assert!(!dir.exists(), "nothing is written until the export writes");
        let (got, files) = write_prepared(prepared);
        assert_eq!(got, want, "zip: {zip}");
        assert_eq!(files, disk, "zip: {zip}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // Without a folder, the files come back in the reply and nothing is written.
    let p = json!({"name": "<name>", "formats": rows});
    let want = export(&mut s, p.clone());
    let (got, files) = write_prepared(s.prepare_screen_export("document.exportForScreens", &p).unwrap());
    assert_eq!((got, files.len()), (want, 0));
    // The settings are remembered when the export is prepared.
    s.prepare_screen_export("document.exportForScreens", &json!({"prefix": "pre-", "formats": rows})).unwrap();
    assert_eq!(s.doc().unwrap().doc.export_settings.get("prefix"), Some(&json!("pre-")));
    // The Asset Export panel's export.
    s.execute("select.all", &json!({})).unwrap();
    s.execute("assets.add", &json!({"multiple": false})).unwrap();
    let p = json!({"folder": folder, "formats": rows});
    let want = s.execute("assets.export", &p).unwrap();
    let disk = on_disk(&want);
    assert_eq!(disk.len(), 3, "{want}");
    std::fs::remove_dir_all(&dir).unwrap();
    let prepared = s.prepare_screen_export("assets.export", &p).unwrap();
    assert_eq!(s.journal.last().map(|(id, _)| id.as_str()), Some("assets.export"));
    assert_eq!(write_prepared(prepared), (want, disk));
    // An encoder's refusal names the command, as the command's own reply does.
    let p = json!({"folder": folder, "formats": [{"format": "bmp", "colorModel": "cmyk"}]});
    let want = s.execute("assets.export", &p).unwrap_err();
    let got = s.prepare_screen_export("assets.export", &p).unwrap().write(&mut |_: &str, _: &[u8]| Ok(())).unwrap_err();
    assert!(matches!(&got, crate::EngineError::BadParams { cmd, .. } if cmd == "assets.export"), "{got:?}");
    assert_eq!(got.to_string(), want.to_string());
    // The checks come first, as the command's do.
    let journaled = s.journal.len();
    assert!(s.prepare_screen_export("document.exportForScreens", &json!({"range": "9"})).is_err());
    assert_eq!(s.journal.len(), journaled);
    assert!(s.screens_hand_off.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_prepared_export_holds_the_document_as_it_was() {
    let mut s = session(1);
    let p = json!({"formats": [{"format": "svg"}]});
    let want = export(&mut s, p.clone());
    let prepared = s.prepare_screen_export("document.exportForScreens", &p).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert_ne!(export(&mut s, p), want, "the edit changes the file");
    assert_eq!(write_prepared(prepared).0, want);
}

/// Output All Blacks as Rich Black (Appearance of Black › Printing / Exporting) makes the RGB files
/// draw a CMYK black as RGB black, prepared or not.
#[test]
fn rich_black_reaches_the_files() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 30, "colorMode": "cmyk"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0, "m": 0, "y": 0, "k": 1}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    let p = json!({"formats": [{"format": "png"}]});
    let px = |r: &Value| image::load_from_memory(&files(r)[0].1).unwrap().to_rgba8().get_pixel(10, 10).0;
    let accurate = px(&export(&mut s, p.clone()));
    assert!(accurate[0] > 10, "accurate: dark grey, {accurate:?}");
    s.execute("prefs.set", &json!({"key": "blackOutput", "value": "rich"})).unwrap();
    assert_eq!(px(&export(&mut s, p.clone())), [0, 0, 0, 255]);
    let (prepared, _) = write_prepared(s.prepare_screen_export("document.exportForScreens", &p).unwrap());
    assert_eq!(px(&prepared), [0, 0, 0, 255]);
}

/// A cancel stops a prepared export before its next file; the file written before it stays.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_canceled_export_stops_before_its_next_file() {
    let mut s = session(3);
    let dir = std::env::temp_dir().join(format!("vc-screens-cancel-{}", std::process::id()));
    let folder = dir.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let prepared = s.prepare_screen_export("document.exportForScreens", &json!({"folder": folder, "formats": [{"format": "svg"}]})).unwrap();
    let progress = super::progress::Progress::new();
    let mut written = vec![];
    let r = super::progress::scope(&progress, || {
        prepared.write(&mut |path: &str, _: &[u8]| {
            written.push(path.to_string());
            progress.cancel();
            Ok(())
        })
    });
    assert!(matches!(r, Err(crate::EngineError::Cancelled)), "{r:?}");
    assert_eq!(written, [format!("{folder}/Artboard-1.svg")]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A prepared export reports one progress step per file, and the raster encoders' artboard steps
/// are not reported.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_prepared_export_counts_its_files() {
    let mut s = session(3);
    let dir = std::env::temp_dir().join(format!("vc-screens-steps-{}", std::process::id()));
    let folder = dir.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let rows = json!([{"format": "png"}, {"format": "png", "scale": "2x"}]);
    let prepared = s.prepare_screen_export("document.exportForScreens", &json!({"folder": folder, "formats": rows})).unwrap();
    let progress = super::progress::Progress::of_files();
    let mut seen = vec![];
    let r = super::progress::scope(&progress, || {
        prepared.write(&mut |_: &str, _: &[u8]| {
            seen.push(progress.counts());
            Ok(())
        })
    });
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(seen, (0..6).map(|n| (n, 6)).collect::<Vec<_>>());
    assert!(progress.counts_files());
    let _ = std::fs::remove_dir_all(dir);
}

/// The warnings in `r` that contain `text`.
fn warned<'a>(r: &'a Value, text: &str) -> Vec<&'a str> {
    r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains(text)).collect()
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_reply_says_once_when_an_image_could_not_be_decoded() {
    let mut s = session(1);
    let rows = json!([{"format": "png"}, {"format": "jpg", "scale": "2x"}]);
    assert_eq!(export(&mut s, json!({ "formats": rows }))["warnings"], json!([]));
    // The renderer can't decode the placed image and draws nothing in its place.
    crate::tests_menucmds::place_undecodable_png(&mut s);
    let dir = std::env::temp_dir().join(format!("vc-screens-undecoded-{}", std::process::id()));
    let folder = dir.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    for p in [
        json!({ "formats": rows }),
        json!({ "formats": rows, "zip": true }),
        json!({ "formats": rows, "folder": folder }),
        json!({ "formats": rows, "folder": folder, "zip": true }),
        json!({ "formats": rows, "fullDocument": true }),
    ] {
        let r = export(&mut s, p.clone());
        assert_eq!(warned(&r, "couldn't be decoded").len(), 1, "{p}: {}", r["warnings"]);
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3, "the files are still written");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn type_in_a_missing_family_is_named_once_for_every_format() {
    let mut s = session(2);
    s.execute("text.create", &json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let r = export(&mut s, json!({"formats": [{"format": "png"}, {"format": "pdf"}, {"format": "eps"}]}));
    let fallback = warned(&r, "fallback font");
    assert_eq!(fallback.len(), 1, "{}", r["warnings"]);
    assert!(fallback[0].contains("Nonexistent Sans Probe"), "{fallback:?}");
}

#[test]
fn full_document_gives_one_file_per_format() {
    let mut s = session(3);
    let r = export(
        &mut s,
        json!({"fullDocument": true, "range": "2", "prefix": "x-", "formats": [{"format": "pdf"}, {"format": "png", "scale": 2}, {"format": "svg"}]}),
    );
    let f = files(&r);
    let title = super::file_stem(&s.doc().unwrap().doc.title);
    assert_eq!(names(&r), [format!("x-{title}.pdf"), format!("x-{title}@2x.png"), format!("x-{title}.svg")]);
    assert_eq!(vectorcraft_pdf::import(&f[0].1).unwrap().artboards.len(), 3, "one page per artboard, whatever the range");
    // The bounds of the art: the square and its 1 pt stroke.
    assert_eq!(size(&f[1].1), (42, 42));
    // Nothing visible: nothing to export but the PDF.
    let mut empty = session(1);
    empty.execute("edit.undo", &json!({})).unwrap();
    assert!(empty.execute("document.exportForScreens", &json!({"fullDocument": true, "formats": [{"format": "png"}]})).is_err());
    assert_eq!(files(&export(&mut empty, json!({"fullDocument": true, "formats": [{"format": "pdf"}]}))).len(), 1);
}

#[test]
fn include_bleed_grows_each_artboard() {
    let mut s = session(1);
    s.execute("document.setup", &json!({"bleed": [2, 4, 6, 8]})).unwrap();
    let r = export(&mut s, json!({"includeBleed": true, "formats": [{"format": "png"}, {"format": "png", "scale": "108w", "suffix": "-w"}]}));
    let f = files(&r);
    assert_eq!(size(&f[0].1), (54, 36), "40 + 6 + 8 by 30 + 2 + 4");
    assert_eq!(size(&f[1].1), (108, 72), "the width counts the bleed");
    let r = export(&mut s, json!({"formats": [{"format": "png"}]}));
    assert_eq!(size(&files(&r)[0].1), (40, 30), "without it, the artboard alone");
}

#[test]
fn png_8_rows_write_indexed_colour() {
    let mut s = session(1);
    let r = export(&mut s, json!({"formats": [{"format": "png8"}, {"format": "png"}]}));
    let f = files(&r);
    // The IHDR colour type: 3 (indexed) for PNG 8, 6 (RGBA) for PNG; both are .png, the PNG 8 row
    // written first.
    assert_eq!(names(&r), ["Artboard-1.png"], "the same file name: the first row wins");
    assert_eq!(f[0].1[25], 3);
    let r = export(&mut s, json!({"formats": [{"format": "png8", "suffix": "-8"}, {"format": "png"}]}));
    let f = files(&r);
    assert_eq!((f[0].1[25], f[1].1[25]), (3, 6));
}

#[test]
fn format_settings_apply_to_their_rows_and_rows_win() {
    let mut s = session(1);
    let px = |png: &[u8]| image::load_from_memory(png).unwrap().to_rgba8().get_pixel(1, 1).0;
    let r = export(
        &mut s,
        json!({"settings": {"png": {"background": "black", "scale": 9, "artboard": 7}, "jpg": {"quality": 5}}, "formats": [{"format": "png"}, {"format": "png", "suffix": "-w", "background": "white"}, {"format": "jpg"}, {"format": "jpg", "quality": 100, "suffix": "-100"}]}),
    );
    let f = files(&r);
    assert_eq!(px(&f[0].1), [0, 0, 0, 255], "the PNG settings' background");
    assert_eq!(size(&f[0].1), (40, 30), "settings don't pick sizes or artboards");
    assert_eq!(px(&f[1].1), [255, 255, 255, 255], "the row's own wins");
    assert!(f[2].1.len() < f[3].1.len(), "quality 5 is smaller than quality 100");
}

/// The format ids `text` lists as `a|b|c` after `start`, up to `end`, sorted.
fn listed(text: &str, start: &str, end: char) -> Vec<String> {
    let (_, rest) = text.split_once(start).unwrap_or_else(|| panic!("no `{start}` in {text}"));
    let mut ids: Vec<String> = rest.split(end).next().unwrap_or_default().split('|').map(|f| f.trim().to_string()).collect();
    ids.sort();
    ids
}

/// The params text (a row's `format` and the keys of `settings`) and the refusal of a format a row
/// can't have list exactly the formats a row writes.
#[test]
fn the_params_and_the_refusal_list_every_format_a_row_writes() {
    let mut s = session(1);
    let (mut written, mut refusals) = (vec![], vec![]);
    for f in FORMATS.iter().filter(|f| f.write) {
        match s.execute("document.exportForScreens", &json!({"formats": [{"format": f.id}]})) {
            Ok(r) => {
                assert_eq!(files(&r).len(), 1, "{}", f.id);
                written.push(f.id.to_string());
            }
            Err(crate::EngineError::BadParams { msg, .. }) => refusals.push(msg),
            Err(e) => panic!("{}: {e}", f.id),
        }
    }
    written.sort();
    let params = crate::cmd::find_command("document.exportForScreens").unwrap().params;
    assert_eq!(listed(params, "[{format: ", ','), written, "a row's format");
    assert_eq!(listed(params, "settings?: {", ':'), written, "the keys of settings");
    assert!(!refusals.is_empty());
    for msg in refusals {
        let list = msg.strip_prefix("Export for Screens writes ").unwrap_or_else(|| panic!("{msg}"));
        let mut ids: Vec<String> = list.replace(" or ", ", ").split(", ").map(str::to_string).collect();
        ids.sort();
        assert_eq!(ids, written, "{msg}");
    }
}

#[test]
fn zip_downloads_round_trip() {
    let mut s = session(2);
    let p = json!({"subfolders": true, "formats": [{"format": "png"}, {"format": "png", "scale": 2}, {"format": "pdf"}]});
    let plain = files(&export(&mut s, p.clone()));
    let mut q = p;
    q["zip"] = json!(true);
    let r = export(&mut s, q);
    let title = super::file_stem(&s.doc().unwrap().doc.title);
    assert_eq!(r["name"], format!("{title}.zip"));
    let zip = b64(&r);
    assert_eq!(r["bytes"].as_u64(), Some(zip.len() as u64));
    let entries = super::zip::entries(&zip).unwrap();
    let names = |files: &[(String, Vec<u8>)]| files.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(&entries), names(&plain), "the same files, sub-folders in their names");
    for ((name, zipped), (_, returned)) in entries.iter().zip(&plain) {
        if name.ends_with(".pdf") {
            // A PDF carries the time it was written.
            assert_eq!(vectorcraft_pdf::import(zipped).unwrap().artboards.len(), 1, "{name}");
        } else {
            assert_eq!(zipped, returned, "{name}");
        }
    }
    assert_eq!(r["files"].as_array().unwrap().len(), 6);
}

#[test]
fn zip_archives_store_their_files() {
    let files = [("a.txt", b"hello".to_vec()), ("sub/ünï.png", vec![0u8, 1, 2, 255]), ("empty", vec![])];
    let zip = super::zip::store(&files).unwrap();
    let back = super::zip::entries(&zip).unwrap();
    assert_eq!(back.len(), 3);
    for ((n, b), (m, c)) in files.iter().zip(&back) {
        assert_eq!((*n, b), (m.as_str(), c));
    }
    assert_eq!(super::zip::entries(&super::zip::store::<&str, Vec<u8>>(&[]).unwrap()).unwrap(), vec![]);
    let mut broken = zip.clone();
    let at = broken.windows(5).position(|w| w == b"hello").unwrap();
    broken[at] = b'j';
    assert!(super::zip::entries(&broken).is_err(), "the CRC catches a changed byte");
}

#[test]
fn the_document_remembers_its_export_settings() {
    let mut s = session(2);
    assert_eq!(s.execute("document.exportSettings", &json!({})).unwrap()["settings"], json!({}));
    s.doc_mut().unwrap().mark_saved();
    let p = json!({"range": "2", "subfolders": true, "prefix": "p-", "zip": true, "openLocation": true, "formats": [{"format": "jpg", "quality": 80, "scale": "2x"}]});
    export(&mut s, p);
    let settings = s.execute("document.exportSettings", &json!({})).unwrap()["settings"].clone();
    assert_eq!(
        settings,
        json!({"range": "2", "subfolders": true, "prefix": "p-", "openLocation": true, "formats": [{"format": "jpg", "quality": 80, "scale": "2x"}]})
    );
    assert!(s.doc().unwrap().is_dirty(), "settings are document data");
    s.doc_mut().unwrap().mark_saved();
    export(
        &mut s,
        json!({"range": "2", "subfolders": true, "prefix": "p-", "openLocation": true, "formats": [{"format": "jpg", "quality": 80, "scale": "2x"}]}),
    );
    assert!(!s.doc().unwrap().is_dirty(), "the same settings change nothing");
    let undo_depth = s.doc().unwrap().history.undo.len();
    export(&mut s, json!({"range": "1"}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_depth, "not an undo step");
    // A failed export keeps the last settings.
    assert!(s.execute("document.exportForScreens", &json!({"range": "9"})).is_err());
    assert_eq!(s.execute("document.exportSettings", &json!({})).unwrap()["settings"], json!({"range": "1"}));
    // Saved with the document (native round trip); older files have none.
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(doc.export_settings, s.doc().unwrap().doc.export_settings);
    let plain = String::from_utf8(vectorcraft_format::save(&vectorcraft_doc::Document::new(5.0, 5.0), false)).unwrap();
    assert!(!plain.contains("export_settings") && !plain.contains("exportSettings"), "not written when empty");
    assert!(vectorcraft_format::load(plain.as_bytes()).unwrap().export_settings.is_empty());
}
