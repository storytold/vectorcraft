//! `imageTrace.batchFolder` (#4921): Image Trace › Batch Trace Folder. Each image in a folder is
//! opened, traced with a preset (and expanded when asked), saved into a destination folder under
//! its own name, and closed again, whether it traced or not.

use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use super::super::{Result, Session, bad};
#[cfg(not(target_arch = "wasm32"))]
use super::super::{bool_or, str_param};

#[cfg(target_arch = "wasm32")]
pub(super) fn batch_folder(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("imageTrace.batchFolder", "no file system here"))
}

#[cfg(not(target_arch = "wasm32"))]
/// The image files a batch traces, by extension.
const IMAGE_EXTS: [&str; 10] = ["png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif", "webp", "psd", "psb"];
#[cfg(not(target_arch = "wasm32"))]
/// Caps on a batch (folders and MCP payloads are untrusted).
const MAX_FILES: usize = 500;
#[cfg(not(target_arch = "wasm32"))]
const MAX_DEPTH: usize = 8;
#[cfg(not(target_arch = "wasm32"))]
const FORMATS: [&str; 5] = ["vectorcraft", "svg", "pdf", "ai", "eps"];

#[cfg(not(target_arch = "wasm32"))]
/// The image files in `folder` (and its subfolders with `deep`), sorted.
fn images(folder: &str, deep: bool, depth: usize, out: &mut Vec<String>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(bad("imageTrace.batchFolder", format!("folders deeper than {MAX_DEPTH} levels")));
    }
    let max = MAX_FILES.saturating_mul(2);
    let (entries, truncated) = super::list_dir(folder, max)?;
    // The listing is cut before it is sorted, so a cut one could skip any image: refuse it.
    if truncated {
        return Err(bad("imageTrace.batchFolder", format!("{folder}: more than {max} files and folders")));
    }
    let mut subdirs = vec![];
    for e in entries {
        let name = e.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let path = std::path::Path::new(folder).join(name).to_string_lossy().into_owned();
        match e.get("kind").and_then(Value::as_str) {
            Some("file") if IMAGE_EXTS.contains(&super::extension(&path).as_str()) => out.push(path),
            Some("folder") if deep => subdirs.push(path),
            _ => {}
        }
        if out.len() > MAX_FILES {
            return Err(bad("imageTrace.batchFolder", format!("more than {MAX_FILES} images")));
        }
    }
    for d in subdirs {
        images(&d, deep, depth + 1, out)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn batch_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "imageTrace.batchFolder";
    let folder = str_param(p, "folder").map(super::absolute_path).ok_or_else(|| bad(C, "missing `folder`"))?;
    let dest = str_param(p, "destinationFolder").map(super::absolute_path).ok_or_else(|| bad(C, "missing `destinationFolder`"))?;
    let format = str_param(p, "format").unwrap_or("vectorcraft");
    if !FORMATS.contains(&format) {
        return Err(bad(C, format!("`format` must be one of {}", FORMATS.join(", "))));
    }
    let preset = str_param(p, "preset").unwrap_or("Default").to_string();
    let presets = s.execute("imageTrace.presets", &json!({}))?;
    if !presets["presets"].as_array().into_iter().flatten().any(|q| q["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(&preset))) {
        return Err(bad(C, format!("no tracing preset `{preset}` (see imageTrace.presets)")));
    }
    let trace = if bool_or(p, "expand", false) { "imageTrace.makeAndExpand" } else { "imageTrace.make" };
    super::may_read(&folder)?;
    super::may_write(&dest)?;
    let mut files = vec![];
    images(&folder, bool_or(p, "includeSubdirs", false), 0, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Err(bad(C, "no images in the folder"));
    }
    std::fs::create_dir_all(&dest).map_err(|e| bad(C, format!("{dest}: {e}")))?;
    let ext = super::save_format(Some(format), None).map_err(|e| bad(C, e))?.extensions.first().copied().unwrap_or(format);
    let active = s.active_index();
    let (mut traced, mut failed, mut taken) = (vec![], vec![], Vec::<String>::new());
    for path in &files {
        let opened_before = s.documents().len();
        let one = |s: &mut Session| -> Result<String> {
            s.execute("document.open", &json!({ "path": path }))?;
            s.execute("select.all", &json!({}))?;
            s.execute(trace, &json!({ "preset": preset }))?;
            let stem = super::file_stem(path);
            let free = (1..=999u32)
                .map(|i| if i == 1 { format!("{stem}.{ext}") } else { format!("{stem}-{i}.{ext}") })
                .map(|n| std::path::Path::new(&dest).join(n).to_string_lossy().into_owned())
                .find(|o| !taken.contains(o) && !std::path::Path::new(o).exists())
                .ok_or_else(|| bad(C, "every numbered name for the result is taken"))?;
            s.execute("document.save", &json!({ "path": free, "format": format }))?;
            Ok(free)
        };
        let result = one(s);
        // The image's document closes whatever happened, so a failure leaves no tab behind.
        if s.documents().len() > opened_before
            && let Some(i) = s.active_index()
        {
            let _ = s.execute("file.close", &json!({ "index": i }));
        }
        match result {
            Ok(out) => {
                taken.push(out.clone());
                traced.push(json!({ "path": path, "output": out }));
            }
            Err(e) => failed.push(json!({ "path": path, "error": e.to_string() })),
        }
    }
    if let Some(i) = active.filter(|i| *i < s.documents().len()) {
        s.set_active(i);
    }
    Ok(json!({ "traced": traced, "failed": failed, "format": format }))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn png(path: &std::path::Path, dark: bool) {
        let mut img = image::RgbaImage::from_pixel(24, 24, image::Rgba([255, 255, 255, 255]));
        for y in 6..18 {
            for x in 6..18 {
                img.put_pixel(x, y, if dark { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([200, 30, 30, 255]) });
            }
        }
        img.save(path).unwrap();
    }

    /// #4921: each image of the folder is traced and saved under its own name; a file that isn't
    /// an image is left alone, one that can't be read is reported, and every document the batch
    /// opened is closed again, the active one staying active.
    #[test]
    fn batch_trace_folder_traces_and_saves_each_image() {
        let dir = std::env::temp_dir().join(format!("vc-batch-trace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (src, out) = (dir.join("in"), dir.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        png(&src.join("logo.png"), true);
        png(&src.join("mark.png"), false);
        std::fs::write(src.join("notes.txt"), "not an image").unwrap();
        std::fs::write(src.join("broken.png"), b"not a png").unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let docs = s.documents().len();
        let r = s.execute("imageTrace.batchFolder", &json!({"folder": src, "destinationFolder": out, "format": "svg", "expand": true})).unwrap();
        assert_eq!(r["traced"].as_array().unwrap().len(), 2, "{r}");
        assert_eq!(r["failed"].as_array().unwrap().len(), 1, "{r}");
        assert!(r["failed"][0]["path"].as_str().unwrap().ends_with("broken.png"));
        let mut names: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["logo.svg", "mark.svg"]);
        let svg = std::fs::read_to_string(out.join("logo.svg")).unwrap();
        assert!(svg.contains("<path") && !svg.contains("<image"), "traced and expanded, not the image");
        assert_eq!(s.documents().len(), docs, "the opened images are closed");
        assert_eq!(s.active_index(), Some(0));
        // Again: the taken names are numbered, nothing is replaced.
        s.execute("imageTrace.batchFolder", &json!({"folder": src, "destinationFolder": out, "format": "svg"})).unwrap();
        assert!(out.join("logo-2.svg").exists() && out.join("mark-2.svg").exists());
        // Refused: no folders, an unknown preset or format, a folder without images.
        for bad in [
            json!({"destinationFolder": out}),
            json!({"folder": src}),
            json!({"folder": src, "destinationFolder": out, "preset": "No Such Preset"}),
            json!({"folder": src, "destinationFolder": out, "format": "docx"}),
            json!({"folder": out.join("empty"), "destinationFolder": out}),
        ] {
            assert!(s.execute("imageTrace.batchFolder", &bad).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder too big to list in full is refused rather than traced in part.
    #[test]
    fn batch_trace_folder_refuses_a_folder_it_cannot_list_in_full() {
        let dir = std::env::temp_dir().join(format!("vc-batch-trace-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("in");
        std::fs::create_dir_all(&src).unwrap();
        for i in 0..=super::MAX_FILES * 2 {
            std::fs::write(src.join(format!("note-{i}.txt")), "").unwrap();
        }
        png(&src.join("logo.png"), true);
        let mut s = Session::new();
        let e = s.execute("imageTrace.batchFolder", &json!({"folder": src, "destinationFolder": dir.join("out")})).unwrap_err();
        assert!(e.to_string().contains("more than"), "{e}");
        assert!(!dir.join("out").exists(), "nothing traced");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
