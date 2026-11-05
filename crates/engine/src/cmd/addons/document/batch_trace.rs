//! Batch Trace Folder (`addon.document.batchTraceFolder`, #4921): each image in a folder is opened,
//! traced with a preset (and expanded when asked), saved into a destination folder under its own
//! name, and closed again, whether it traced or not.

use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

#[cfg(not(target_arch = "wasm32"))]
use super::super::super::fileio;
use super::super::super::{Result, Session, bad};
#[cfg(not(target_arch = "wasm32"))]
use super::super::super::{bool_or, str_param};

/// The command id.
pub(super) const ID: &str = "addon.document.batchTraceFolder";

#[cfg(target_arch = "wasm32")]
pub(super) fn batch_trace_folder(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad(ID, "no file system here"))
}

#[cfg(not(target_arch = "wasm32"))]
/// The image files a batch traces, by extension.
const IMAGE_EXTS: [&str; 10] = ["png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif", "webp", "psd", "psb"];
#[cfg(not(target_arch = "wasm32"))]
/// Caps on a batch (folders and MCP payloads are untrusted).
pub(super) const MAX_FILES: usize = 500;
#[cfg(not(target_arch = "wasm32"))]
const MAX_DEPTH: usize = 8;
#[cfg(not(target_arch = "wasm32"))]
const FORMATS: [&str; 5] = ["vectorcraft", "svg", "pdf", "ai", "eps"];

#[cfg(not(target_arch = "wasm32"))]
/// The image files in `folder` (and its subfolders with `deep`), sorted.
fn images(folder: &str, deep: bool, depth: usize, out: &mut Vec<String>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(bad(ID, format!("folders deeper than {MAX_DEPTH} levels")));
    }
    let max = MAX_FILES.saturating_mul(2);
    let (entries, truncated) = fileio::list_dir(folder, max)?;
    // The listing is cut before it is sorted, so a cut one could skip any image: refuse it.
    if truncated {
        return Err(bad(ID, format!("{folder}: more than {max} files and folders")));
    }
    let mut subdirs = vec![];
    for e in entries {
        let name = e.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let path = std::path::Path::new(folder).join(name).to_string_lossy().into_owned();
        match e.get("kind").and_then(Value::as_str) {
            Some("file") if IMAGE_EXTS.contains(&fileio::extension(&path).as_str()) => out.push(path),
            Some("folder") if deep => subdirs.push(path),
            _ => {}
        }
        if out.len() > MAX_FILES {
            return Err(bad(ID, format!("more than {MAX_FILES} images")));
        }
    }
    for d in subdirs {
        images(&d, deep, depth + 1, out)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn batch_trace_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let folder = str_param(p, "folder").map(fileio::absolute_path).ok_or_else(|| bad(ID, "missing `folder`"))?;
    let dest = str_param(p, "destinationFolder").map(fileio::absolute_path).ok_or_else(|| bad(ID, "missing `destinationFolder`"))?;
    let format = str_param(p, "format").unwrap_or("vectorcraft");
    if !FORMATS.contains(&format) {
        return Err(bad(ID, format!("`format` must be one of {}", FORMATS.join(", "))));
    }
    let preset = str_param(p, "preset").unwrap_or("Default").to_string();
    let presets = s.execute("imageTrace.presets", &json!({}))?;
    if !presets["presets"].as_array().into_iter().flatten().any(|q| q["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(&preset))) {
        return Err(bad(ID, format!("no tracing preset `{preset}` (see imageTrace.presets)")));
    }
    let trace = if bool_or(p, "expand", false) { "imageTrace.makeAndExpand" } else { "imageTrace.make" };
    fileio::may_read(&folder)?;
    fileio::may_write(&dest)?;
    let mut files = vec![];
    images(&folder, bool_or(p, "includeSubdirs", false), 0, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Err(bad(ID, "no images in the folder"));
    }
    std::fs::create_dir_all(&dest).map_err(|e| bad(ID, format!("{dest}: {e}")))?;
    let ext = fileio::save_format(Some(format), None).map_err(|e| bad(ID, e))?.extensions.first().copied().unwrap_or(format);
    let active = s.active_index();
    let (mut traced, mut failed, mut taken) = (vec![], vec![], Vec::<String>::new());
    for path in &files {
        let opened_before = s.documents().len();
        let one = |s: &mut Session| -> Result<String> {
            s.execute("document.open", &json!({ "path": path }))?;
            s.execute("select.all", &json!({}))?;
            s.execute(trace, &json!({ "preset": preset }))?;
            let stem = fileio::file_stem(path);
            // Names this batch already took count as taken, as do files on disk.
            let free = super::free_name(&dest, &stem, ext, |o| taken.iter().any(|t| t == o) || std::path::Path::new(o).exists())
                .ok_or_else(|| bad(ID, "every numbered name for the result is taken"))?;
            s.execute("document.save", &json!({ "path": free, "format": format }))?;
            Ok(free)
        };
        let result = one(s);
        // The image's document closes whatever happened, so a failure leaves no tab behind.
        if s.documents().len() > opened_before
            && let Some(i) = s.active_index()
        {
            // A just-opened image has no work to lose; if closing fails, one extra tab stays open.
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
