//! `action.batch`: play an action's commands on every file in a folder, or on every data set
//! of the open document (the Actions panel menu's Batch…).
//!
//! Public options follow the Batch dialog: source Folder or Data Sets; destination None,
//! Save and Close, or Folder; Override Open / Save / Export. Cited:
//! <https://helpx.adobe.com/illustrator/desktop/automate-visualize-data/automate-actions/batch-options.html>

use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use super::super::{Result, Session, bad};
#[cfg(not(target_arch = "wasm32"))]
use super::super::{bool_or, find_command, str_param};

#[cfg(target_arch = "wasm32")]
pub(super) fn action_batch(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("action.batch", "no file system here"))
}

/// Caps on a batch (folders and MCP payloads are untrusted).
#[cfg(not(target_arch = "wasm32"))]
const MAX_BATCH_FILES: usize = 500;
#[cfg(not(target_arch = "wasm32"))]
const MAX_BATCH_DEPTH: usize = 8;

/// Shared Batch options for one run.
#[cfg(not(target_arch = "wasm32"))]
struct Run<'a> {
    label: &'a str,
    steps: &'a [Value],
    stop_on_error: bool,
    override_save: bool,
    override_export: bool,
    destination: &'a str,
    dest_folder: Option<&'a str>,
}

/// Play `commands` on each source file or data set.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn action_batch(s: &mut Session, p: &Value) -> Result<Value> {
    use super::{absolute_path, may_write};

    const C: &str = "action.batch";
    let cmds = p.get("commands").and_then(Value::as_array).ok_or_else(|| bad(C, "missing `commands`"))?;
    if cmds.is_empty() {
        return Err(bad(C, "`commands` must not be empty"));
    }
    if cmds.len() > 256 {
        return Err(bad(C, "more than 256 commands"));
    }
    for (i, c) in cmds.iter().enumerate() {
        let id = c.get("command").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            return Err(bad(C, format!("commands[{i}]: missing `command`")));
        }
        if id == "command.batch" || id == "action.batch" {
            return Err(bad(C, "batches cannot nest"));
        }
        if find_command(id).is_none() {
            return Err(bad(
                C,
                match super::super::params::nearest_command(id) {
                    Some(near) => format!("commands[{i}]: unknown command `{id}`; did you mean `{near}`?"),
                    None => format!("commands[{i}]: unknown command `{id}`"),
                },
            ));
        }
    }
    let source = str_param(p, "source").unwrap_or("folder");
    let destination = str_param(p, "destination").unwrap_or("none");
    if !matches!(destination, "none" | "saveAndClose" | "folder") {
        return Err(bad(C, "`destination` must be `none`, `saveAndClose` or `folder`"));
    }
    let dest_folder = str_param(p, "destinationFolder").map(absolute_path);
    if destination == "folder" {
        let Some(ref folder) = dest_folder else {
            return Err(bad(C, "`destinationFolder` is required when destination is `folder`"));
        };
        may_write(folder)?;
        std::fs::create_dir_all(folder).map_err(|e| bad(C, format!("{folder}: {e}")))?;
    }
    let label = str_param(p, "label").unwrap_or("Batch").to_string();
    let override_open = bool_or(p, "overrideOpen", true);
    let run = Run {
        label: &label,
        steps: cmds,
        stop_on_error: bool_or(p, "stopOnError", true),
        override_save: bool_or(p, "overrideSave", true),
        override_export: bool_or(p, "overrideExport", true),
        destination,
        dest_folder: dest_folder.as_deref(),
    };
    match source {
        "folder" => batch_folder(s, C, p, override_open, &run),
        "datasets" => batch_datasets(s, C, &run),
        _ => Err(bad(C, "`source` must be `folder` or `datasets`")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn batch_folder(s: &mut Session, cmd: &str, p: &Value, override_open: bool, run: &Run<'_>) -> Result<Value> {
    use super::screens::safe_file_name;
    use super::{absolute_path, may_read};

    let folder = str_param(p, "folder").map(absolute_path).ok_or_else(|| bad(cmd, "missing `folder`"))?;
    may_read(&folder)?;
    let include_subdirs = bool_or(p, "includeSubdirs", false);
    let files = collect_files(&folder, include_subdirs, 0)?;
    if files.len() > MAX_BATCH_FILES {
        return Err(bad(cmd, format!("more than {MAX_BATCH_FILES} files")));
    }
    if files.is_empty() {
        return Err(bad(cmd, "no readable files in the folder"));
    }
    // Each file's name without its extension, through `safe_file_name` (`a..svg` gives `a`, the
    // folder Windows makes for `a.`), with `-2`, `-3`… after a name an earlier file used (`a.svg` and
    // `sub/a.svg`, or `a.svg` and `a.ai`) or after a name whose files or folder an earlier file
    // wrote ([`distinct_stems`]), as data set names are numbered. No output takes the name of a
    // source file in the destination folder, other than that file's own output, or of a subfolder
    // of the destination folder that holds source files.
    let steps = filter_steps(run.steps, override_open, false);
    let run = Run { steps: &steps, ..*run };
    let in_dest: Vec<Option<(String, bool)>> = files.iter().map(|path| name_in_folder(path, run.dest_folder)).collect();
    let reserved = in_dest.iter().flatten().map(|(name, _)| name.clone()).collect();
    let stems = distinct_stems(
        files.iter().zip(&in_dest).map(|(path, place)| {
            let stem = safe_file_name(std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(""));
            let (exts, makes_folder) = outputs(&run, &super::extension(path));
            let own = place.as_ref().filter(|(_, own)| *own).map(|(name, _)| name.clone());
            (if stem.is_empty() { "file".to_string() } else { stem }, exts, makes_folder, own)
        }),
        reserved,
    );
    let mut processed = Vec::new();
    let mut failed = 0usize;
    for (path, stem) in files.iter().zip(&stems) {
        let ext = super::extension(path);
        match process_one_file(s, &run, path, stem, &ext) {
            Ok(row) => processed.push(row),
            Err(e) => {
                failed += 1;
                processed.push(json!({"path": path, "error": e.to_string()}));
                if run.stop_on_error {
                    return Ok(json!({"processed": processed, "count": processed.len(), "failed": failed, "stopped": true}));
                }
            }
        }
    }
    Ok(json!({"processed": processed, "count": processed.len(), "failed": failed, "stopped": false}))
}

#[cfg(not(target_arch = "wasm32"))]
fn batch_datasets(s: &mut Session, cmd: &str, run: &Run<'_>) -> Result<Value> {
    use super::screens::safe_file_name;

    let st = s.doc()?;
    let names: Vec<String> = st.doc.variables.datasets.iter().map(|d| d.name.clone()).collect();
    if names.is_empty() {
        return Err(bad(cmd, "no data sets"));
    }
    if names.len() > MAX_BATCH_FILES {
        return Err(bad(cmd, format!("more than {MAX_BATCH_FILES} data sets")));
    }
    let doc_stem = st.path.as_deref().and_then(|p| std::path::Path::new(p).file_stem().and_then(|s| s.to_str())).unwrap_or("Untitled").to_string();
    // Data set files are named File + Data Set Name (helpx, for Override Save/Export; the folder
    // destination uses the same name). `safe_file_name` writes the characters Windows does not allow
    // in a file name (`/`, `\`, `:`, `<`, `>`, `"`, `|`, `?`, `*` and control characters) as `-` and
    // drops trailing periods and spaces, which Windows drops from a folder name (`a.` and `a` give
    // one folder), and `distinct_stems` adds `-2`, `-3`… to a name an earlier data set used.
    let steps = filter_steps(run.steps, true, false);
    let run = Run { steps: &steps, ..*run };
    let (exts, folder) = outputs(&run, "vectorcraft");
    let stems = distinct_stems(
        names.iter().map(|name| (safe_file_name(&format!("{doc_stem}_{name}")), exts.clone(), folder, None)),
        std::collections::HashSet::new(),
    );
    let mut processed = Vec::new();
    let mut failed = 0usize;
    for (name, stem) in names.iter().zip(&stems) {
        match process_one_dataset(s, &run, name, stem) {
            Ok(row) => processed.push(row),
            Err(e) => {
                failed += 1;
                processed.push(json!({"dataset": name, "error": e.to_string()}));
                if run.stop_on_error {
                    return Ok(json!({"processed": processed, "count": processed.len(), "failed": failed, "stopped": true}));
                }
            }
        }
    }
    Ok(json!({"processed": processed, "count": processed.len(), "failed": failed, "stopped": false}))
}

/// What one source writes into the destination folder: the extension of each file a Save or
/// Export step or the Folder destination writes there as `{stem}.{ext}`, and whether an Export for
/// Screens or Export Assets step makes the folder `{stem}` there. Follows [`rewrite_paths`] and
/// [`finish_destination`].
#[cfg(not(target_arch = "wasm32"))]
fn outputs(run: &Run<'_>, default_ext: &str) -> (Vec<String>, bool) {
    let mut exts = Vec::new();
    let mut folder = false;
    if run.dest_folder.is_some() {
        for c in run.steps {
            let id = c.get("command").and_then(Value::as_str).unwrap_or("");
            if run.override_export && matches!(id, "document.exportForScreens" | "assets.export") {
                folder = true;
            } else if (run.override_save && matches!(id, "document.save" | "file.save" | "file.saveAs"))
                || (run.override_export && matches!(id, "document.export" | "document.exportSelection"))
            {
                let format = c.get("params").and_then(|p| p.get("format")).and_then(Value::as_str).filter(|f| !f.is_empty());
                exts.push(format.map_or_else(|| default_ext.to_string(), format_extension));
            }
        }
    }
    if run.destination == "folder" {
        exts.push(if default_ext.is_empty() { "vectorcraft".to_string() } else { default_ext.to_string() });
    }
    (exts, folder)
}

/// When the file at `path` is inside `folder`: the name in lower case of the file, when it is
/// directly inside, or of the subfolder of `folder` that holds it, and whether it is the file's own.
#[cfg(not(target_arch = "wasm32"))]
fn name_in_folder(path: &str, folder: Option<&str>) -> Option<(String, bool)> {
    let folder = std::fs::canonicalize(folder?).ok()?;
    let path = std::fs::canonicalize(path).ok()?;
    let mut parts = path.strip_prefix(&folder).ok()?.components();
    let first = parts.next()?.as_os_str().to_str()?.to_lowercase();
    Some((first, parts.next().is_none()))
}

/// The stem of each source, given as (name, extensions, folder, own file) with [`outputs`]: the
/// name, with `-2`, `-3`… after it when an earlier source used it or when one of the files or the
/// folder it writes has the name of a file or folder an earlier source wrote (`a.svg` writes the
/// file `a.svg`, so the Export for Screens folder of `a.svg.svg` is `a.svg-2`) or a name in
/// `reserved` other than its own file's. Names are compared without case.
#[cfg(not(target_arch = "wasm32"))]
fn distinct_stems(
    sources: impl IntoIterator<Item = (String, Vec<String>, bool, Option<String>)>,
    reserved: std::collections::HashSet<String>,
) -> Vec<String> {
    let mut stems = std::collections::HashSet::new();
    let mut written = reserved;
    sources
        .into_iter()
        .map(|(base, exts, folder, own)| {
            let mut name = base.clone();
            let mut n = 1usize;
            loop {
                let lower = name.to_lowercase();
                let names: Vec<String> = exts.iter().map(|e| format!("{lower}.{}", e.to_lowercase())).chain(folder.then(|| lower.clone())).collect();
                if !stems.contains(&lower) && names.iter().all(|f| !written.contains(f) || own.as_ref() == Some(f)) {
                    stems.insert(lower);
                    written.extend(names);
                    return name;
                }
                n += 1;
                name = format!("{base}-{n}");
            }
        })
        .collect()
}

/// Drop Open (and optionally Close) steps when the batch supplies the file itself.
#[cfg(not(target_arch = "wasm32"))]
fn filter_steps(cmds: &[Value], drop_open: bool, drop_close: bool) -> Vec<Value> {
    cmds.iter()
        .filter(|c| {
            let id = c.get("command").and_then(Value::as_str).unwrap_or("");
            if drop_open && matches!(id, "document.open" | "file.open") {
                return false;
            }
            if drop_close && matches!(id, "file.close" | "document.close") {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn process_one_file(s: &mut Session, run: &Run<'_>, path: &str, stem: &str, ext: &str) -> Result<Value> {
    s.execute("document.open", &json!({"path": path}))?;
    let rewritten = rewrite_paths(run.steps, run.override_save, run.override_export, run.dest_folder, stem, ext);
    let batch = s.execute("command.batch", &json!({"label": run.label, "commands": rewritten}))?;
    // Folder sources open a file per pass: close after Save and Close / Folder.
    finish_destination(s, run, stem, ext, path, true)?;
    Ok(json!({"path": path, "result": batch}))
}

#[cfg(not(target_arch = "wasm32"))]
fn process_one_dataset(s: &mut Session, run: &Run<'_>, name: &str, stem: &str) -> Result<Value> {
    s.execute("dataset.select", &json!({"name": name}))?;
    let rewritten = rewrite_paths(run.steps, run.override_save, run.override_export, run.dest_folder, stem, "vectorcraft");
    let batch = s.execute("command.batch", &json!({"label": run.label, "commands": rewritten}))?;
    // Data sets share one open document: write each row out but leave it open for the next.
    finish_destination(s, run, stem, "vectorcraft", "", false)?;
    Ok(json!({"dataset": name, "result": batch}))
}

#[cfg(not(target_arch = "wasm32"))]
fn finish_destination(s: &mut Session, run: &Run<'_>, stem: &str, ext: &str, source_path: &str, close: bool) -> Result<()> {
    match run.destination {
        "none" => Ok(()),
        "saveAndClose" => {
            if !source_path.is_empty() {
                s.execute("document.save", &json!({"path": source_path}))?;
            } else {
                s.execute("document.save", &json!({}))?;
            }
            if close {
                s.execute("file.close", &json!({}))?;
            }
            Ok(())
        }
        "folder" => {
            let folder = run.dest_folder.ok_or_else(|| bad("action.batch", "missing destination folder"))?;
            let out_ext = if ext.is_empty() { "vectorcraft" } else { ext };
            let out = std::path::Path::new(folder).join(format!("{stem}.{out_ext}"));
            let out = out.to_string_lossy().into_owned();
            // When `close` is false (data sets), write a copy so the template keeps its path.
            write_result(s, &out, out_ext, !close)?;
            if close {
                s.execute("file.close", &json!({}))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Save when the extension is a Save format; otherwise export (PNG, JPEG…).
/// `as_copy`: `file.saveCopy` so the open document keeps its path (data-set batches).
#[cfg(not(target_arch = "wasm32"))]
fn write_result(s: &mut Session, path: &str, ext: &str, as_copy: bool) -> Result<()> {
    use super::SAVE_FORMATS;
    let id = if ext == "drawcraft" { "vectorcraft" } else { ext };
    if SAVE_FORMATS.contains(&id) {
        let cmd = if as_copy { "file.saveCopy" } else { "document.save" };
        s.execute(cmd, &json!({"path": path}))?;
    } else {
        s.execute("document.export", &json!({"path": path, "format": id}))?;
    }
    Ok(())
}

/// Point Save / Export steps at `dest_folder/{stem}.{ext}` when overrides are on: `ext` is the
/// extension of the step's `format` ([`format_extension`]), else `default_ext`. Export for Screens
/// and Export Assets steps get `dest_folder/{stem}` as their `folder`, one folder for each source
/// file or data set, and name the files in it themselves.
#[cfg(not(target_arch = "wasm32"))]
fn rewrite_paths(
    steps: &[Value],
    override_save: bool,
    override_export: bool,
    dest_folder: Option<&str>,
    stem: &str,
    default_ext: &str,
) -> Vec<Value> {
    let Some(folder) = dest_folder else {
        return steps.to_vec();
    };
    steps
        .iter()
        .map(|c| {
            let id = c.get("command").and_then(Value::as_str).unwrap_or("");
            let mut c = c.clone();
            let rewrite = (override_save && matches!(id, "document.save" | "file.save" | "file.saveAs"))
                || (override_export && matches!(id, "document.export" | "document.exportForScreens" | "document.exportSelection" | "assets.export"));
            if !rewrite {
                return c;
            }
            let params = c.get("params").cloned().unwrap_or(json!({}));
            let mut params = params.as_object().cloned().unwrap_or_default();
            if matches!(id, "document.exportForScreens" | "assets.export") {
                params.insert("folder".into(), json!(std::path::Path::new(folder).join(stem).to_string_lossy()));
            } else {
                let ext = match params.get("format").and_then(Value::as_str).filter(|f| !f.is_empty()) {
                    Some(f) => format_extension(f),
                    None => default_ext.to_string(),
                };
                let file = std::path::Path::new(folder).join(format!("{stem}.{ext}"));
                params.insert("path".into(), json!(file.to_string_lossy()));
            }
            if let Some(o) = c.as_object_mut() {
                o.insert("params".into(), Value::Object(params));
            }
            c
        })
        .collect()
}

/// The extension of the file a step writes in `format` (an id or extension, as [`super::format`]
/// reads it: any case, a leading dot allowed): the extension as written, in lower case and without
/// the dot, when it is one of the format's (`SVG` → `svg`, `.JPEG` → `jpeg`), else the format's
/// first (`png8` → `png`, `template` → `vctemplate`). An unknown format (`xyz`) keeps its spelling,
/// and the step fails with its command's error.
#[cfg(not(target_arch = "wasm32"))]
fn format_extension(format: &str) -> String {
    let written = format.trim_start_matches('.').to_ascii_lowercase();
    match super::format(format) {
        Some(f) if f.extensions.contains(&written.as_str()) => written,
        Some(f) => f.extensions.first().map_or(written, |e| (*e).to_string()),
        None => format.to_string(),
    }
}

/// Readable files under `folder` (and optionally its subfolders), capped.
#[cfg(not(target_arch = "wasm32"))]
fn collect_files(folder: &str, include_subdirs: bool, depth: usize) -> Result<Vec<String>> {
    use std::path::PathBuf;

    use super::list_dir;

    if depth > MAX_BATCH_DEPTH {
        return Err(bad("action.batch", format!("folders deeper than {MAX_BATCH_DEPTH} levels")));
    }
    let (entries, _) = list_dir(folder, MAX_BATCH_FILES.saturating_mul(2))?;
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    for e in entries {
        let name = e.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let kind = e.get("kind").and_then(Value::as_str).unwrap_or("");
        let path = PathBuf::from(folder).join(name);
        let path = path.to_string_lossy().into_owned();
        match kind {
            "file" if is_openable(&path) => files.push(path),
            "folder" if include_subdirs => subdirs.push(path),
            _ => {}
        }
    }
    files.sort();
    for sub in subdirs {
        let mut nested = collect_files(&sub, true, depth + 1)?;
        files.append(&mut nested);
        if files.len() > MAX_BATCH_FILES {
            break;
        }
    }
    Ok(files)
}

#[cfg(not(target_arch = "wasm32"))]
fn is_openable(path: &str) -> bool {
    use super::{FORMATS, OPEN_EXTS, extension};
    let ext = extension(path);
    if ext.is_empty() {
        return false;
    }
    OPEN_EXTS.iter().any(|e| *e == ext) || FORMATS.iter().any(|f| f.read && f.extensions.iter().any(|e| *e == ext))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("vc-action-batch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_svg(dir: &std::path::Path, name: &str, fill: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(
            &p,
            format!(
                r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="100" height="80">
                <rect x="10" y="10" width="40" height="30" fill="{fill}"/></svg>"#
            ),
        )
        .unwrap();
        p
    }

    #[test]
    fn batch_folder_runs_action_and_saves_to_destination() {
        let src = tmp("src");
        let dest = tmp("dest");
        write_svg(&src, "a.svg", "#f00");
        write_svg(&src, "b.svg", "#0f0");
        let mut s = Session::new();
        // Need a document so command.batch's has_doc passes after the first open leaves one.
        s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "object.unlockAll", "params": {}}],
                    "label": "Unlock"
                }),
            )
            .unwrap();
        assert_eq!(r["count"], 2);
        assert_eq!(r["failed"], 0);
        assert!(dest.join("a.svg").is_file(), "a.svg written");
        assert!(dest.join("b.svg").is_file(), "b.svg written");
    }

    #[test]
    fn batch_skips_open_steps_when_override_open() {
        let src = tmp("open");
        let dest = tmp("open-out");
        write_svg(&src, "only.svg", "#00f");
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "overrideOpen": true,
                    "commands": [
                        {"command": "document.open", "params": {"path": "/no/such/file.svg"}},
                        {"command": "object.unlockAll", "params": {}}
                    ]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert!(dest.join("only.svg").is_file());
    }

    /// Override Export names each file with the extension of the format the step names, however
    /// the step spells it; a format given by one of its extensions keeps that extension.
    #[test]
    fn batch_override_export_names_files_with_the_format_s_extension() {
        let src = tmp("ext-src");
        write_svg(&src, "a.svg", "#f00");
        let cases = [(".svg", "a.svg"), ("SVG", "a.svg"), ("svg", "a.svg"), ("png8", "a.png"), ("template", "a.vctemplate"), (".JPEG", "a.jpeg")];
        for (i, (format, want)) in cases.into_iter().enumerate() {
            let dest = tmp(&format!("ext-out-{i}"));
            let mut s = Session::new();
            let r = s
                .execute(
                    "action.batch",
                    &json!({
                        "folder": src.to_string_lossy(),
                        "destinationFolder": dest.to_string_lossy(),
                        "commands": [{"command": "document.export", "params": {"format": format}}]
                    }),
                )
                .unwrap();
            assert_eq!(r["failed"], 0, "{format}: {r}");
            assert_eq!(file_names(&dest), [want], "{format}: {r}");
        }
        // An unknown format keeps its spelling, and the step fails with the export's error.
        let dest = tmp("ext-out-unknown");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.export", "params": {"format": "xyz"}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 1, "{r}");
        assert!(r["processed"][0]["error"].as_str().is_some_and(|e| e.contains("unknown format `xyz`")), "{r}");
        assert!(file_names(&dest).is_empty(), "{r}");
    }

    /// Override Export writes an Export for Screens step's files into a folder for each source file
    /// inside the destination folder, under the names the export gives them, whether the step names
    /// a folder of its own, takes one from an export preset or names none. With Override Export off,
    /// the files go to the step's folder.
    #[test]
    fn batch_override_export_sends_export_for_screens_files_to_the_destination_folder() {
        let src = tmp("screens-src");
        write_svg(&src, "a.svg", "#f00");
        write_svg(&src, "b.svg", "#0f0");
        let own = tmp("screens-own");
        let svg = json!([{"format": "svg"}]);
        let preset = json!({"name": "Own Folder", "settings": {"formats": svg, "folder": own.to_string_lossy()}});
        let steps = [json!({"folder": own.to_string_lossy(), "formats": svg}), json!({"exportPreset": "Own Folder"}), json!({"formats": svg})];
        for (i, params) in steps.into_iter().enumerate() {
            let dest = tmp(&format!("screens-out-{i}"));
            let mut s = Session::new();
            s.execute("document.exportPresets.save", &preset).unwrap();
            let r = s
                .execute(
                    "action.batch",
                    &json!({
                        "folder": src.to_string_lossy(),
                        "destinationFolder": dest.to_string_lossy(),
                        "commands": [{"command": "document.exportForScreens", "params": params}]
                    }),
                )
                .unwrap();
            assert_eq!(r["failed"], 0, "{params}: {r}");
            assert_eq!(file_names(&dest), ["a", "b"], "{params}: {r}");
            for stem in ["a", "b"] {
                assert_eq!(file_names(&dest.join(stem)), ["Artboard-1.svg"], "{params}: {r}");
            }
            assert!(file_names(&own).is_empty(), "{params}: {r}");
        }
        let dest = tmp("screens-out-off");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "destinationFolder": dest.to_string_lossy(),
                    "overrideExport": false,
                    "commands": [{"command": "document.exportForScreens", "params": {"folder": own.to_string_lossy(), "formats": svg}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&own), ["Artboard-1.svg"], "{r}");
        assert!(file_names(&dest).is_empty(), "{r}");
    }

    /// Two source files with the same name in different subfolders get Export for Screens folders of
    /// their own: the second one's folder is numbered, as a repeated data set name is.
    #[test]
    fn batch_override_export_numbers_export_for_screens_folders_of_files_with_the_same_name() {
        let src = tmp("screens-same-src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        write_svg(&src, "a.svg", "#f00");
        write_svg(&src.join("sub"), "a.svg", "#0f0");
        let dest = tmp("screens-same-out");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "includeSubdirs": true,
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.exportForScreens", "params": {"formats": [{"format": "svg"}]}}]
                }),
            )
            .unwrap();
        for folder in file_names(&dest) {
            println!("{folder}/: {:?}", file_names(&dest.join(&folder)));
        }
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["a", "a-2"], "{r}");
        for (folder, fill) in [("a", "#ff0000"), ("a-2", "#00ff00")] {
            assert_eq!(file_names(&dest.join(folder)), ["Artboard-1.svg"], "{r}");
            let svg = std::fs::read_to_string(dest.join(folder).join("Artboard-1.svg")).unwrap();
            assert!(svg.contains(fill), "{folder}: {svg}");
        }
    }

    /// A source file whose name without its extension ends in a period (`a..svg`) gets the folder
    /// name without it, numbered after the folder of `a.svg`: Windows drops the period from a folder
    /// name, so `a.` and `a` would be one folder there.
    #[test]
    fn batch_override_export_numbers_folders_that_differ_by_a_trailing_period() {
        let src = tmp("screens-period-src");
        write_svg(&src, "a.svg", "#f00");
        write_svg(&src, "a..svg", "#0f0");
        let dest = tmp("screens-period-out");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.exportForScreens", "params": {"formats": [{"format": "svg"}]}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["a", "a-2"], "{r}");
        for folder in ["a", "a-2"] {
            assert_eq!(file_names(&dest.join(folder)), ["Artboard-1.svg"], "{r}");
        }
    }

    /// A source file's Export for Screens folder does not take the name of a file the Folder
    /// destination saved for an earlier source: `a.svg` is saved as `a.svg`, so the folder of
    /// `a.svg.svg` is numbered.
    #[test]
    fn batch_override_export_numbers_folders_named_like_a_saved_file() {
        let src = tmp("screens-file-src");
        write_svg(&src, "a.svg", "#f00");
        write_svg(&src, "a.svg.svg", "#0f0");
        let dest = tmp("screens-file-out");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.exportForScreens", "params": {"formats": [{"format": "svg"}]}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["a", "a.svg", "a.svg-2", "a.svg-2.svg"], "{r}");
        for folder in ["a", "a.svg-2"] {
            assert_eq!(file_names(&dest.join(folder)), ["Artboard-1.svg"], "{r}");
        }
    }

    /// With the destination folder the same as the source folder, a file's output does not replace
    /// another source file before Batch opens it: `a..svg` would be saved as `a.svg`, so it is saved
    /// as `a-2.svg`, and `a.svg` is saved over itself.
    #[test]
    fn batch_does_not_save_over_another_source_file_in_the_destination_folder() {
        let src = tmp("in-place");
        write_svg(&src, "a..svg", "#f00");
        write_svg(&src, "a.svg", "#0f0");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "destination": "folder",
                    "destinationFolder": src.to_string_lossy(),
                    "commands": [{"command": "object.unlockAll", "params": {}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&src), ["a-2.svg", "a..svg", "a.svg"], "{r}");
        for (file, fill) in [("a-2.svg", "#ff0000"), ("a.svg", "#00ff00")] {
            let svg = std::fs::read_to_string(src.join(file)).unwrap();
            assert!(svg.contains(fill), "{file}: {svg}");
        }
    }

    /// With the destination folder the same as the source folder and `includeSubdirs`, an Export for
    /// Screens folder does not take the name of a subfolder that holds source files: the folder of
    /// `a.svg` is `a-2`, so `a/Artboard-1.svg` is opened as it was.
    #[test]
    fn batch_does_not_export_into_a_source_subfolder() {
        let src = tmp("in-place-sub");
        std::fs::create_dir_all(src.join("a")).unwrap();
        write_svg(&src, "a.svg", "#f00");
        write_svg(&src.join("a"), "Artboard-1.svg", "#0f0");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "folder": src.to_string_lossy(),
                    "includeSubdirs": true,
                    "destinationFolder": src.to_string_lossy(),
                    "commands": [{"command": "document.exportForScreens", "params": {"formats": [{"format": "svg"}]}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        let source = std::fs::read_to_string(src.join("a").join("Artboard-1.svg")).unwrap();
        assert!(source.contains("fill=\"#0f0\""), "{r}: {source}");
        let export = std::fs::read_to_string(src.join("a-2").join("Artboard-1.svg")).unwrap();
        assert!(export.contains("#ff0000"), "{r}: {export}");
    }

    /// Data sets whose names differ only by a trailing period (`a` and `a.`) get Export Assets
    /// folders of their own: the second one's folder is numbered.
    #[test]
    fn batch_override_export_numbers_data_set_folders_that_differ_by_a_trailing_period() {
        let dest = tmp("assets-period-out");
        let mut s = with_data_sets(&["a", "a."]);
        let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("assets.add", &json!({"ids": [id], "name": "Logo"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "assets.export", "params": {"formats": [{"format": "svg"}]}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["Untitled_a", "Untitled_a-2"], "{r}");
        for folder in ["Untitled_a", "Untitled_a-2"] {
            assert_eq!(file_names(&dest.join(folder)), ["Logo.svg"], "{r}");
        }
    }

    /// Two source files with the same name in different subfolders are saved under names of their
    /// own, by Override Save and by the Folder destination: the second one's name is numbered.
    #[test]
    fn batch_numbers_saved_files_of_source_files_with_the_same_name() {
        let src = tmp("save-same-src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        write_svg(&src, "a.svg", "#f00");
        write_svg(&src.join("sub"), "a.svg", "#0f0");
        let save = json!([{"command": "file.saveAs", "params": {"format": "svg"}}]);
        let unlock = json!([{"command": "object.unlockAll", "params": {}}]);
        for (tag, destination, commands) in [("save", "none", save), ("folder", "folder", unlock)] {
            let dest = tmp(&format!("save-same-{tag}"));
            let mut s = Session::new();
            let r = s
                .execute(
                    "action.batch",
                    &json!({
                        "folder": src.to_string_lossy(),
                        "includeSubdirs": true,
                        "destination": destination,
                        "destinationFolder": dest.to_string_lossy(),
                        "commands": commands
                    }),
                )
                .unwrap();
            assert_eq!(r["failed"], 0, "{tag}: {r}");
            assert_eq!(file_names(&dest), ["a-2.svg", "a.svg"], "{tag}: {r}");
            for (file, fill) in [("a.svg", "#ff0000"), ("a-2.svg", "#00ff00")] {
                let svg = std::fs::read_to_string(dest.join(file)).unwrap();
                assert!(svg.contains(fill), "{tag}: {file}: {svg}");
            }
        }
    }

    /// Override Export writes an Export Assets step's files into a folder for each data set inside
    /// the destination folder, under the assets' names.
    #[test]
    fn batch_override_export_sends_asset_export_files_to_the_destination_folder() {
        let own = tmp("assets-own");
        let dest = tmp("assets-out");
        let mut s = with_data_sets(&["a", "b"]);
        let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("assets.add", &json!({"ids": [id], "name": "Logo"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "assets.export", "params": {"folder": own.to_string_lossy(), "formats": [{"format": "svg"}]}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["Untitled_a", "Untitled_b"], "{r}");
        for stem in ["Untitled_a", "Untitled_b"] {
            assert_eq!(file_names(&dest.join(stem)), ["Logo.svg"], "{r}");
        }
        assert!(file_names(&own).is_empty(), "{r}");
    }

    /// Override Save names each file with the extension of the format the step names, also over
    /// data sets.
    #[test]
    fn batch_override_save_names_files_with_the_format_s_extension() {
        for (i, format) in [".svg", "SVG", ".PDF"].into_iter().enumerate() {
            let dest = tmp(&format!("ext-save-{i}"));
            let mut s = with_data_sets(&["a"]);
            let r = s
                .execute(
                    "action.batch",
                    &json!({
                        "source": "datasets",
                        "destinationFolder": dest.to_string_lossy(),
                        "commands": [{"command": "file.saveAs", "params": {"format": format}}]
                    }),
                )
                .unwrap();
            assert_eq!(r["failed"], 0, "{format}: {r}");
            let want = format!("Untitled_a.{}", format.trim_start_matches('.').to_ascii_lowercase());
            assert_eq!(file_names(&dest), [want], "{format}: {r}");
        }
    }

    #[test]
    fn batch_datasets_applies_each_row() {
        let dest = tmp("sets");
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 40, "text": "A"})).unwrap()["id"].as_u64().unwrap();
        s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
        s.execute("variable.bind", &json!({"variable": "Name", "ids": [id]})).unwrap();
        s.execute("dataset.capture", &json!({})).unwrap();
        s.execute("text.setText", &json!({"ids": [id], "text": "B"})).unwrap();
        s.execute("dataset.capture", &json!({})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "object.unlockAll", "params": {}}],
                    "label": "Per set"
                }),
            )
            .unwrap();
        assert_eq!(r["count"], 2, "{r}");
        assert_eq!(r["failed"], 0, "{r}");
        let written: Vec<_> = std::fs::read_dir(&dest).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        assert_eq!(written.len(), 2, "{written:?}");
    }

    /// The names of the files in `dir`, sorted.
    fn file_names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// A new document with an empty data set for each of `names`.
    fn with_data_sets(names: &[&str]) -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
        for name in names {
            s.execute("dataset.new", &json!({"name": name})).unwrap();
        }
        s
    }

    #[test]
    fn batch_keeps_data_set_files_inside_the_destination_folder() {
        let root = tmp("set-names-root");
        let dest = root.join("out");
        // The folder that `a/../../x` would pass through on its way out of `out`.
        std::fs::create_dir_all(dest.join("Untitled_a")).unwrap();
        let mut s = with_data_sets(&["a/../../x", "../y"]);
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "stopOnError": false,
                    "commands": [{"command": "object.unlockAll", "params": {}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["Untitled_..-y.vectorcraft", "Untitled_a", "Untitled_a-..-..-x.vectorcraft"], "{r}");
        assert_eq!(file_names(&root), ["out"], "nothing is written outside the destination folder");
    }

    #[test]
    fn batch_saves_data_sets_named_with_separators() {
        let dest = tmp("set-names-sep");
        let exports = tmp("set-names-sep-export");
        let mut s = with_data_sets(&["a/b", "c\\d", "e:f"]);
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "stopOnError": false,
                    "commands": [{"command": "object.unlockAll", "params": {}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["Untitled_a-b.vectorcraft", "Untitled_c-d.vectorcraft", "Untitled_e-f.vectorcraft"], "{r}");
        // Override Export points each data set's export at the destination folder under the same name.
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destinationFolder": exports.to_string_lossy(),
                    "stopOnError": false,
                    "commands": [{"command": "document.export", "params": {"format": "svg"}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&exports), ["Untitled_a-b.svg", "Untitled_c-d.svg", "Untitled_e-f.svg"], "{r}");
    }

    /// Windows does not allow `<`, `>`, `"`, `|`, `?`, `*` or control characters in a file name:
    /// Batch writes them as `-` in a data set's name, on every platform.
    #[test]
    fn batch_saves_data_sets_named_with_characters_windows_does_not_allow() {
        let dest = tmp("set-names-win");
        let mut s = with_data_sets(&["a?b", "c*d<e>", "f\"g|h", "i\u{7}j"]);
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "stopOnError": false,
                    "commands": [{"command": "object.unlockAll", "params": {}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(
            file_names(&dest),
            ["Untitled_a-b.vectorcraft", "Untitled_c-d-e-.vectorcraft", "Untitled_f-g-h.vectorcraft", "Untitled_i-j.vectorcraft"],
            "{r}"
        );
    }

    #[test]
    fn batch_gives_each_data_set_a_file_of_its_own() {
        let dest = tmp("set-names-same");
        let mut s = with_data_sets(&["a:b", "a-b", "A-B"]);
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets",
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "object.unlockAll", "params": {}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["Untitled_A-B-3.vectorcraft", "Untitled_a-b-2.vectorcraft", "Untitled_a-b.vectorcraft"], "{r}");
    }

    #[test]
    fn batch_refuses_nesting_and_empty_commands() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
        assert!(s.execute("action.batch", &json!({"commands": []})).is_err());
        assert!(
            s.execute(
                "action.batch",
                &json!({
                    "folder": ".",
                    "commands": [{"command": "command.batch", "params": {"commands": []}}]
                })
            )
            .is_err()
        );
    }
}
