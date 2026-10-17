//! `action.batch`: play an action's commands on every file in a folder, or on every data set
//! of the open document (the Actions panel menu's Batch…).
//!
//! Public options follow the Batch dialog: source Folder or Data Sets; destination None,
//! Save and Close, or Folder; Override Open / Save / Export. Cited:
//! <https://helpx.adobe.com/illustrator/desktop/automate-visualize-data/automate-actions/batch-options.html>

use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use super::super::*;

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
            return Err(bad(C, format!("commands[{i}]: unknown command `{id}`")));
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
    let steps = filter_steps(run.steps, override_open, false);
    let run = Run { steps: &steps, ..*run };
    let mut processed = Vec::new();
    let mut failed = 0usize;
    for path in &files {
        let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("file");
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
    use super::distinct_names;
    use super::screens::file_part;

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
    // destination uses the same name). `file_part` writes the characters Windows does not allow in a
    // file name (`/`, `\`, `:`, `<`, `>`, `"`, `|`, `?`, `*` and control characters) in the data set's
    // name as `-`, and `distinct_names` adds `-2`, `-3`… to a name an earlier data set used.
    let stems = distinct_names(names.iter().map(|name| format!("{doc_stem}_{}", file_part(name))));
    let steps = filter_steps(run.steps, true, false);
    let run = Run { steps: &steps, ..*run };
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

/// Point Save / Export steps at `dest_folder/{stem}.{ext}` when overrides are on.
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
                || (override_export && matches!(id, "document.export" | "document.exportForScreens" | "document.exportSelection"));
            if !rewrite {
                return c;
            }
            let params = c.get("params").cloned().unwrap_or(json!({}));
            let mut params = params.as_object().cloned().unwrap_or_default();
            let ext = params.get("format").and_then(Value::as_str).unwrap_or(default_ext);
            let ext = if ext.is_empty() { default_ext } else { ext };
            let file = std::path::Path::new(folder).join(format!("{stem}.{ext}"));
            params.insert("path".into(), json!(file.to_string_lossy()));
            if let Some(o) = c.as_object_mut() {
                o.insert("params".into(), Value::Object(params));
            }
            c
        })
        .collect()
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
