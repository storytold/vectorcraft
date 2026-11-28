use std::path::Path;

use super::*;

const MIME: &str = include_str!("../../../packaging/linux/ai.storyteller.vectorcraft.mime.xml");

/// A PNG of `width`×`height` pixels with `data` as its samples.
fn png_of(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    out
}

/// An empty folder `vectorcraft-thumbnail-handler-<pid>-<name>` in the temp folder.
#[cfg(unix)]
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-thumbnail-handler-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The names of the files in `dir`.
#[cfg(unix)]
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn the_class_id_string_and_number_are_the_same_id() {
    let hex = format!("{CLSID_VALUE:032X}");
    let grouped = format!("{{{}-{}-{}-{}-{}}}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]);
    assert_eq!(grouped, CLSID);
}

#[test]
fn registration_names_the_dll_and_the_handler_of_every_extension() {
    let values = registry_values("C:\\Program Files\\VectorCraft\\vectorcraft_thumbnail_handler.dll");
    let class = format!("CLSID\\{CLSID}");
    let server = format!("{class}\\InprocServer32");
    let want = vec![
        RegistryValue { key: class, name: None, data: DESCRIPTION.into() },
        RegistryValue { key: server.clone(), name: None, data: "C:\\Program Files\\VectorCraft\\vectorcraft_thumbnail_handler.dll".into() },
        RegistryValue { key: server, name: Some("ThreadingModel"), data: "Apartment".into() },
        RegistryValue { key: format!(".vectorcraft\\ShellEx\\{THUMBNAIL_SHELL_EX}"), name: None, data: CLSID.into() },
        RegistryValue { key: format!(".drawcraft\\ShellEx\\{THUMBNAIL_SHELL_EX}"), name: None, data: CLSID.into() },
        RegistryValue { key: format!(".vctemplate\\ShellEx\\{THUMBNAIL_SHELL_EX}"), name: None, data: CLSID.into() },
    ];
    assert_eq!(values, want);
    assert_eq!(THUMBNAIL_SHELL_EX.to_ascii_uppercase(), "{E357FCCD-A995-4576-B01F-234630154E96}");
}

#[test]
fn the_extensions_are_the_globs_of_the_document_and_template_types() {
    let mut globs = Vec::new();
    let mut in_type = false;
    for line in MIME.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("<mime-type type=\"") {
            in_type = matches!(rest.split('"').next(), Some("application/x-vectorcraft" | "application/x-vectorcraft-template"));
        } else if in_type && let Some(rest) = line.strip_prefix("<glob pattern=\"*") {
            globs.push(rest.split('"').next().unwrap().to_owned());
        }
    }
    assert_eq!(globs, EXTENSIONS);
}

#[test]
fn only_this_class_id_names_this_handler() {
    assert!(names_this_handler(CLSID));
    assert!(names_this_handler(&CLSID.to_ascii_lowercase()));
    assert!(!names_this_handler("{C7657C4A-9F68-40FA-A4DF-96BC08EB3551}"));
    assert!(!names_this_handler(""));
}

#[test]
fn a_stream_larger_than_the_limit_is_refused() {
    assert_eq!(read_bounded(&[7u8; 10][..], 10).unwrap(), vec![7; 10]);
    assert_eq!(read_bounded(&[][..], 10).unwrap(), Vec::<u8>::new());
    assert_eq!(read_bounded(&[7u8; 11][..], 10).unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert!(read_bounded(&[7u8; 11][..], u64::MAX).is_ok());
}

#[test]
fn a_second_request_gets_the_same_document() {
    let mut document = Document::new(io::Cursor::new(vec![1u8, 2, 3]));
    assert_eq!(document.bytes(10).unwrap(), [1, 2, 3]);
    assert_eq!(document.bytes(10).unwrap(), [1, 2, 3]);

    let mut large = Document::new(&[7u8; 11][..]);
    assert_eq!(large.bytes(10).unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert!(large.bytes(10).is_err());
}

#[test]
fn the_size_asked_of_the_command_is_at_most_its_limit() {
    assert_eq!(request_size(0), None);
    assert_eq!(request_size(1), Some(1));
    assert_eq!(request_size(256), Some(256));
    assert_eq!(request_size(MAX_SIDE), Some(MAX_SIDE));
    assert_eq!(request_size(2048), Some(MAX_SIDE));
}

#[test]
fn the_command_line_is_thumbnail_size_input_output() {
    let args = arguments(96, Path::new("in.vectorcraft"), Path::new("out.png"));
    assert_eq!(args, ["thumbnail", "--size", "96", "in.vectorcraft", "out.png"]);
}

#[test]
fn an_rgba_png_becomes_top_down_premultiplied_bgra() {
    // Two rows: opaque red and half-transparent blue, then 20% white and a clear white pixel.
    let rgba = [255, 0, 0, 255, 0, 0, 255, 128, 255, 255, 255, 51, 255, 255, 255, 0];
    let picture = picture(&png_of(2, 2, png::ColorType::Rgba, &rgba), 2).unwrap();
    assert_eq!((picture.width, picture.height), (2, 2));
    assert_eq!(picture.bgra, [0, 0, 255, 255, 128, 0, 0, 128, 51, 51, 51, 51, 0, 0, 0, 0]);
}

#[test]
fn a_png_that_is_not_an_rgba_thumbnail_is_refused() {
    assert!(picture(&png_of(1, 1, png::ColorType::Rgb, &[1, 2, 3]), 8).is_err());
    assert!(picture(&png_of(3, 1, png::ColorType::Rgba, &[0; 12]), 2).is_err(), "wider than asked");
    assert!(picture(&png_of(1, 3, png::ColorType::Rgba, &[0; 12]), 2).is_err(), "taller than asked");
    assert!(picture(b"not a picture", 8).is_err());
    let png = png_of(2, 2, png::ColorType::Rgba, &[9; 16]);
    assert!(picture(&png[..png.len() - 20], 8).is_err(), "cut short");
}

/// `render` runs the program with the thumbnail arguments, returns what it writes to OUT, gives
/// an error for a failed or overlong run, and leaves no file behind. The stand-ins are shell
/// scripts, all written before the first one runs.
#[cfg(unix)]
#[test]
fn render_runs_the_command_and_removes_its_files() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    let dir = folder("render");
    let temp = dir.join("temp");
    std::fs::create_dir(&temp).unwrap();
    let png = png_of(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4]);
    std::fs::write(dir.join("picture.png"), &png).unwrap();
    std::fs::write(dir.join("document"), b"document bytes").unwrap();
    let script = |name: &str, body: &str| {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let d = dir.display();
    let writes = script(
        "writes",
        &format!(
            "[ \"$1 $2 $3\" = \"thumbnail --size 64\" ] || exit 4\ncase \"$4\" in *.vectorcraft) ;; *) exit 5 ;; esac\n\
             cmp -s \"$4\" '{d}/document' || exit 3\ncp '{d}/picture.png' \"$5\""
        ),
    );
    let fails = script("fails", "echo broken >&2\nexit 1");
    let silent = script("silent", "exit 0");
    let hangs = script("hangs", "exec sleep 30");

    assert_eq!(render(&writes, b"document bytes", 64, &temp, DEADLINE).unwrap(), png);
    assert!(render(&fails, b"x", 64, &temp, DEADLINE).unwrap_err().contains("exited with"));
    assert!(render(&silent, b"x", 64, &temp, DEADLINE).is_err(), "no OUT written");
    assert!(render(&hangs, b"x", 64, &temp, Duration::ZERO).unwrap_err().contains("ran longer than"));
    assert!(render(&dir.join("missing"), b"x", 64, &temp, DEADLINE).is_err());
    assert_eq!(files_in(&temp), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}
