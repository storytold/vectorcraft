//! A file whose import left something out isn't written over silently: its hidden text, art or
//! layers would be gone for good.

use serde_json::json;
use vectorcraft_engine::Session;

fn ascii85(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(4) {
        let mut x = chunk.iter().enumerate().fold(0u32, |v, (i, b)| v | u32::from(*b) << (24 - 8 * i));
        let mut d = [0u8; 5];
        for c in d.iter_mut().rev() {
            *c = (x % 85) as u8 + b'!';
            x /= 85;
        }
        out.extend(d.iter().take(chunk.len() + 1).map(|c| char::from(*c)));
    }
    out.push_str("~>");
    out
}

/// A square drawn with an operator the layers' reader doesn't know and flat on the page: the file
/// comes in as its page, and its hidden layer is left out.
fn lossy_eps() -> Vec<u8> {
    let layer = |name: &str, visible: u8, body: &str| {
        format!("%AI5_BeginLayer\n{visible} 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n({name}) Ln\n{body}\nLB\n%AI5_EndLayer--\n")
    };
    let editing = format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n{}{}%%Trailer\n",
        layer("Art", 1, "1 0 0 0 1 0 Zq\n0 0 1 0 k\n10 10 m\n14 10 L\n14 14 L\n10 14 L\nf"),
        layer("Hidden", 0, "0 0 1 0 k\n50 50 m\n54 50 L\n54 54 L\n50 54 L\nf"),
    );
    let packed = ruzstd::encoding::compress_to_vec(editing.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest);
    let lines: Vec<String> = ascii85(&packed).as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 1 0 setcmykcolor 10 10 moveto 14 10 lineto 14 14 lineto 10 14 lineto closepath fill\nshowpage\n%%EOF\n%AI9_PrivateDataBegin\n%AI24_DataStream\n{}\n%AI9_PrivateDataEnd\n",
        lines.join("\n")
    )
    .into_bytes()
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-lossy-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_file_the_import_left_things_out_of_is_not_written_over_unasked() {
    let d = dir("a");
    let src = d.join("art.eps");
    let original = lossy_eps();
    std::fs::write(&src, &original).unwrap();
    let mut s = Session::new();
    let opened = s.execute("document.open", &json!({ "path": src.to_string_lossy() })).unwrap();
    assert!(opened["warnings"].to_string().contains("layers weren't read"), "{opened}");

    // Over the same file: refused, and the file is as it was.
    let e = s.execute("document.export", &json!({ "path": src.to_string_lossy(), "format": "eps" })).unwrap_err().to_string();
    assert!(e.contains("left things out") && e.contains("acknowledgeLoss"), "{e}");
    assert_eq!(std::fs::read(&src).unwrap(), original);
    // However the path is spelled.
    let spelled = d.join(".").join("art.eps");
    assert!(s.execute("document.export", &json!({ "path": spelled.to_string_lossy(), "format": "eps" })).is_err());

    // Another name is fine, and so is replacing the file when asked to.
    let other = d.join("art copy.eps");
    s.execute("document.export", &json!({ "path": other.to_string_lossy(), "format": "eps" })).unwrap();
    assert!(other.exists());
    s.execute("document.export", &json!({ "path": src.to_string_lossy(), "format": "eps", "acknowledgeLoss": true })).unwrap();
    assert_ne!(std::fs::read(&src).unwrap(), original);
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn a_file_the_import_read_whole_is_written_over_as_before() {
    let d = dir("b");
    let src = d.join("plain.eps");
    std::fs::write(&src, "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 1 0 setcmykcolor 10 10 moveto 14 10 lineto 14 14 lineto closepath fill\nshowpage\n%%EOF\n").unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({ "path": src.to_string_lossy() })).unwrap();
    s.execute("document.export", &json!({ "path": src.to_string_lossy(), "format": "eps" })).unwrap();
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn the_other_ways_of_writing_a_file_ask_too() {
    let d = dir("c");
    let src = d.join("art.eps");
    let original = lossy_eps();
    std::fs::write(&src, &original).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({ "path": src.to_string_lossy() })).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    for (cmd, params) in [
        ("document.exportPdf", json!({})),
        ("document.exportSelection", json!({ "format": "eps" })),
        ("document.exportForOffice", json!({})),
        ("file.print", json!({ "format": "pdf" })),
    ] {
        let mut p = params;
        p["path"] = json!(src.to_string_lossy());
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains("left things out"), "{cmd}: {e}");
        assert_eq!(std::fs::read(&src).unwrap(), original, "{cmd}");
    }
    std::fs::remove_dir_all(d).ok();
}
