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
    dataset_file_naming: &'a str,
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
        dataset_file_naming: p.get("datasetFileNaming").map_or("fileDataSetName", |v| v.as_str().unwrap_or("")),
    };
    match source {
        "folder" => batch_folder(s, C, p, override_open, &run),
        "datasets" => {
            let one_file = batch_overwrites_one_file(s, p);
            let mut r = batch_datasets(s, C, &run)?;
            if one_file {
                r["warning"] = json!(ONE_FILE_WARNING);
            }
            Ok(r)
        }
        _ => Err(bad(C, "`source` must be `folder` or `datasets`")),
    }
}

/// The `warning` of a Data Sets batch for which [`batch_overwrites_one_file`] holds.
#[cfg(not(target_arch = "wasm32"))]
const ONE_FILE_WARNING: &str = "each data set saves over the same file, which keeps only the last data set";

/// Whether an `action.batch` with params `p` plays on data sets of the open document in `s` and
/// writes each data set to the same file. Save and Close saves the open document over its own file
/// after every data set, unless a Save step that Override Save points into `destinationFolder`
/// first makes each data set's file the document's own. A step that the batch does not point into `destinationFolder` writes the
/// same file every time when it writes one: a Save, Save As, Save a Copy, Save as Template, Export,
/// Export Selection, PDF, EPS, DXF, Save for Web or Save for Office Documents step that gives a
/// `path`; a Save step without a `path` when the document has a file of its own (saved, and not
/// converted from an older version or restored by Data Recovery) and the step gives no `format` or
/// the document's own; an Export for Screens step whose `folder`, given or from its
/// `exportPreset`, names a folder, whatever the overrides.
/// Other steps return their data instead of writing a file.
pub fn batch_overwrites_one_file(s: &Session, p: &Value) -> bool {
    if p.get("source").and_then(Value::as_str) != Some("datasets") {
        return false;
    }
    let folder = p.get("destinationFolder").and_then(Value::as_str).is_some();
    let redirected = |key: &str| folder && p.get(key).and_then(Value::as_bool).unwrap_or(true);
    let (save, export) = (redirected("overrideSave"), redirected("overrideExport"));
    let steps = p.get("commands").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    if p.get("destination").and_then(Value::as_str) == Some("saveAndClose") && !saves_move_to_each_file(steps, save) {
        return true;
    }
    // The format of the open document's own file, which a Save step without a `path` writes.
    let own_format = s.active().filter(|st| st.path.is_some() && !st.converted && !st.recovered).map(|st| st.format);
    let names = |v: Option<&Value>| v.and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    steps.iter().any(|c| {
        let params = c.get("params").cloned().unwrap_or_else(|| Value::Object(Default::default()));
        let path = names(params.get("path"));
        let own_file = || match params.get("format").and_then(Value::as_str).filter(|f| !f.is_empty()) {
            None => own_format.is_some(),
            Some(f) => own_format.is_some_and(|own| super::format(f).is_some_and(|f| f.id == own)),
        };
        match c.get("command").and_then(Value::as_str).unwrap_or("") {
            "document.save" | "file.save" => !save && (path || own_file()),
            "file.saveAs" => !save && path,
            "file.saveCopy" | "file.saveAsTemplate" => path,
            "document.export" | "document.exportSelection" => !export && path,
            // The batch does not point these exports into `destinationFolder`.
            "document.exportPdf" | "document.exportEps" | "document.exportDxf" | "document.exportForWeb" | "document.exportForOffice" => path,
            // Override Export sets `path`, which Export for Screens does not read: its `folder` stays.
            "document.exportForScreens" => super::screens::with_preset(s, &params).is_ok_and(|p| names(p.get("folder"))),
            _ => false,
        }
    })
}

/// Whether `steps` hold a Save, Save As or `document.save` step that Override Save points at each
/// data set's own file in the destination folder (`redirected`). Such a step makes that file the
/// document's own, so Save and Close then saves each data set to its own file.
fn saves_move_to_each_file(steps: &[Value], redirected: bool) -> bool {
    redirected && steps.iter().any(|c| retargeting_save(c, true))
}

/// Whether step `c` is a Save, Save As or `document.save` that writes a file (its own `path`, or one
/// in the destination folder when `redirected`) and makes that file the document's own: a save in
/// the native template format writes a copy and leaves the document's file as it was.
fn retargeting_save(c: &Value, redirected: bool) -> bool {
    if !matches!(c.get("command").and_then(Value::as_str), Some("document.save" | "file.save" | "file.saveAs")) {
        return false;
    }
    let params = c.get("params");
    let path = params.and_then(|p| p.get("path")).and_then(Value::as_str).filter(|p| !p.is_empty());
    let format_ok = match params.and_then(|p| p.get("format")).and_then(Value::as_str).filter(|f| !f.is_empty()) {
        Some(f) => super::format(f).is_some_and(|f| f.id != "template"),
        // Override Save gives the step a native file name; else its own path names the format.
        None if redirected => true,
        None => path.and_then(super::format_for_name).is_none_or(|f| f.id != "template"),
    };
    format_ok && (redirected || path.is_some())
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
    let st = s.doc()?;
    let names: Vec<String> = st.doc.variables.datasets.iter().map(|d| d.name.clone()).collect();
    if names.is_empty() {
        return Err(bad(cmd, "no data sets"));
    }
    if names.len() > MAX_BATCH_FILES {
        return Err(bad(cmd, format!("more than {MAX_BATCH_FILES} data sets")));
    }
    let redirected = run.override_save && run.dest_folder.is_some();
    // A recorded save with a file to write gives the document a file of its own before Save and Close.
    if run.destination == "saveAndClose" && !run.steps.iter().any(|c| retargeting_save(c, redirected)) {
        if st.path.is_none() {
            return Err(bad(cmd, "the document has never been saved"));
        }
        // `save_plan` writes no file for a document converted from an older version or restored by
        // Data Recovery.
        if st.converted || st.recovered {
            return Err(bad(cmd, "save the document with Save As before Save and Close"));
        }
    }
    let doc_stem = st.path.as_deref().and_then(|p| std::path::Path::new(p).file_stem().and_then(|s| s.to_str())).unwrap_or("Untitled").to_string();
    let stems = dataset_stems(&names, &doc_stem, run.dataset_file_naming, run, &st.doc)?;
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
            let out = output_path(folder, stem, out_ext).ok_or_else(|| bad("action.batch", "unsafe destination file name"))?;
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

/// Builds the three data set naming forms, replaces unsafe name characters, and adds `-2`, `-3`
/// suffixes when a stem was used earlier or its output already exists in a folder the batch may
/// read and write.
#[cfg(not(target_arch = "wasm32"))]
fn dataset_stems(names: &[String], doc_stem: &str, naming: &str, run: &Run<'_>, doc: &vectorcraft_doc::Document) -> Result<Vec<String>> {
    use super::screens::{file_part, safe_file_name};

    if !matches!(naming, "fileNumber" | "fileDataSetName" | "dataSetName") {
        return Err(bad("action.batch", "`datasetFileNaming` must be `fileNumber`, `fileDataSetName` or `dataSetName`"));
    }
    let doc_stem = safe_file_name(doc_stem);
    let doc_stem = if doc_stem.is_empty() || matches!(doc_stem.as_str(), "." | "..") { "Untitled" } else { &doc_stem };
    let bases = names.iter().enumerate().map(|(i, name)| {
        let safe = file_part(name);
        let safe_data_set = safe_file_name(&safe);
        let ordinal = format!("{doc_stem}{:03}", i + 1);
        match naming {
            "fileNumber" => ordinal,
            "dataSetName" if !safe_data_set.is_empty() => safe_data_set,
            "dataSetName" => ordinal,
            _ => format!("{doc_stem}_{safe}"),
        }
    });
    let outputs = dataset_output_extensions(run, doc);
    let mut taken = std::collections::HashSet::new();
    let mut reserved = std::collections::HashSet::new();
    let mut stems = Vec::with_capacity(bases.len());
    for base in bases {
        let mut candidate = base.clone();
        let mut suffix = 2;
        while taken.contains(&candidate.to_lowercase())
            || dataset_output_paths(run.dest_folder, &candidate, &outputs, doc)
                .iter()
                .any(|path| reserved.contains(&path.to_string_lossy().to_lowercase()))
            || dataset_output_exists(run.dest_folder, &candidate, &outputs, doc)
        {
            if suffix > 10_000 {
                return Err(bad("action.batch", "no free data set file name in destination folder"));
            }
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        taken.insert(candidate.to_lowercase());
        reserved
            .extend(dataset_output_paths(run.dest_folder, &candidate, &outputs, doc).into_iter().map(|path| path.to_string_lossy().to_lowercase()));
        stems.push(candidate);
    }
    Ok(stems)
}

#[cfg(not(target_arch = "wasm32"))]
struct DatasetOutput {
    ext: String,
    artboards: bool,
    joiner: &'static str,
}

/// Lists step output extensions and reserves artboard file names for saves with any artboard or
/// exports with multiple artboards. Per-artboard names count for vectorcraft, template, AI and AIT
/// saves and SVG, SVGZ, EPS, raster, DXF, EMF, WMF and AI exports.
#[cfg(not(target_arch = "wasm32"))]
fn dataset_output_extensions(run: &Run<'_>, doc: &vectorcraft_doc::Document) -> Vec<DatasetOutput> {
    let mut outputs = Vec::new();
    if run.destination == "folder" {
        outputs.push(DatasetOutput { ext: "vectorcraft".into(), artboards: false, joiner: "-" });
    }
    for step in run.steps {
        let id = step.get("command").and_then(Value::as_str).unwrap_or("");
        let writes = (run.override_save && matches!(id, "document.save" | "file.save" | "file.saveAs"))
            || (run.override_export && matches!(id, "document.export" | "document.exportForScreens" | "document.exportSelection"));
        if writes {
            let format = step.get("params").and_then(|p| p.get("format")).and_then(Value::as_str).filter(|f| !f.is_empty());
            let ext = match format {
                Some(format) if super::format(format).is_some() => format_extension(format),
                Some(_) => continue,
                None => "vectorcraft".to_string(),
            };
            if safe_extension(&ext) {
                let format_id = format.and_then(super::format).map(|f| f.id).unwrap_or("vectorcraft");
                let is_save = matches!(id, "document.save" | "file.save" | "file.saveAs");
                let artboards = if is_save {
                    !doc.artboards.is_empty() && matches!(format_id, "vectorcraft" | "template" | "ai" | "ait")
                } else {
                    doc.artboards.len() > 1
                        && id == "document.export"
                        && matches!(
                            format_id,
                            "svg"
                                | "svgz"
                                | "eps"
                                | "png"
                                | "jpg"
                                | "webp"
                                | "gif"
                                | "png8"
                                | "tiff"
                                | "bmp"
                                | "tga"
                                | "psd"
                                | "dxf"
                                | "emf"
                                | "wmf"
                                | "ai"
                        )
                };
                outputs.push(DatasetOutput { ext, artboards, joiner: if format_id == "eps" { "_" } else { "-" } });
            }
        }
    }
    outputs
}

#[cfg(not(target_arch = "wasm32"))]
fn dataset_output_exists(folder: Option<&str>, stem: &str, outputs: &[DatasetOutput], doc: &vectorcraft_doc::Document) -> bool {
    let Some(folder) = folder else { return false };
    if super::may_read(folder).is_err() || super::may_write(folder).is_err() {
        return false;
    }
    dataset_output_paths(Some(folder), stem, outputs, doc).iter().any(|path| {
        let path_text = path.to_string_lossy();
        super::may_read(&path_text).is_ok() && super::may_write(&path_text).is_ok() && path.exists()
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn dataset_output_paths(folder: Option<&str>, stem: &str, outputs: &[DatasetOutput], doc: &vectorcraft_doc::Document) -> Vec<std::path::PathBuf> {
    let Some(folder) = folder else { return Vec::new() };
    let boards: Vec<_> = (0..doc.artboards.len()).collect();
    let mut parts = super::artboard_file_names(doc, &boards);
    for board in boards {
        parts.extend(super::artboard_file_names(doc, &[board]));
    }
    let mut paths = Vec::new();
    for output in outputs {
        if !safe_extension(&output.ext) || super::format(&output.ext).is_none() {
            continue;
        }
        paths.extend(output_path(folder, stem, &output.ext));
        if output.artboards {
            for part in &parts {
                paths.extend(output_path(folder, &format!("{stem}{}{part}", output.joiner), &output.ext));
            }
        }
    }
    paths
}

/// Returns whether an extension is safe to use in an output file name.
#[cfg(not(target_arch = "wasm32"))]
fn safe_extension(ext: &str) -> bool {
    !ext.is_empty() && !ext.contains(['/', '\\', ':']) && !ext.contains("..")
}

/// Returns a path inside the destination folder for a safe stem and extension.
#[cfg(not(target_arch = "wasm32"))]
fn output_path(folder: &str, stem: &str, ext: &str) -> Option<std::path::PathBuf> {
    use std::path::Path;
    if stem.is_empty() || matches!(stem, "." | "..") || stem.contains(std::path::is_separator) || !safe_extension(ext) {
        return None;
    }
    let name = format!("{stem}.{ext}");
    if name.contains(std::path::is_separator) {
        return None;
    }
    let path = Path::new(folder).join(name);
    (path.parent() == Some(Path::new(folder))).then_some(path)
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
/// extension of the step's `format` ([`format_extension`]), else `default_ext`.
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
            let ext = match params.get("format").and_then(Value::as_str).filter(|f| !f.is_empty()) {
                Some(f) => format_extension(f),
                None => default_ext.to_string(),
            };
            if params.get("format").and_then(Value::as_str).filter(|f| !f.is_empty()).is_some_and(|f| super::format(f).is_none()) {
                return c;
            }
            let Some(file) = output_path(folder, stem, &ext) else { return c };
            params.insert("path".into(), json!(file.to_string_lossy()));
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

    #[cfg(unix)]
    #[test]
    fn batch_folder_keeps_backslash_in_source_file_name() {
        let src = tmp("backslash-src");
        let dest = tmp("backslash-dest");
        write_svg(&src, "a\\b.svg", "#f00");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "folder",
                    "folder": src.to_string_lossy(),
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "stopOnError": true,
                    "commands": [{"command": "object.unlockAll", "params": {}}]
                }),
            )
            .unwrap();
        assert_eq!(r["count"], 1, "{r}");
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(r["stopped"], false, "{r}");
        assert!(dest.join("a\\b.svg").is_file(), "{r}");
    }

    #[cfg(unix)]
    #[test]
    fn batch_folder_override_export_keeps_backslash_in_source_file_name() {
        let src = tmp("backslash-export-src");
        let dest = tmp("backslash-export-dest");
        write_svg(&src, "a\\b.svg", "#f00");
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "folder",
                    "folder": src.to_string_lossy(),
                    "destination": "folder",
                    "destinationFolder": dest.to_string_lossy(),
                    "stopOnError": true,
                    "commands": [{"command": "document.export", "params": {"format": "svg"}}]
                }),
            )
            .unwrap();
        assert_eq!(r["count"], 1, "{r}");
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(r["stopped"], false, "{r}");
        assert_eq!(file_names(&dest), ["a\\b.svg"], "{r}");
    }

    #[test]
    fn output_path_accepts_only_platform_file_name_separators() {
        let folder = tmp("output-path");
        let folder = folder.to_string_lossy();
        let backslash = output_path(&folder, "a\\b", "svg");
        #[cfg(windows)]
        assert!(backslash.is_none());
        #[cfg(not(windows))]
        assert_eq!(backslash, Some(std::path::Path::new(folder.as_ref()).join("a\\b.svg")));
        for stem in ["a/b", ".", "..", ""] {
            assert!(output_path(&folder, stem, "svg").is_none(), "{stem:?}");
        }
        #[cfg(windows)]
        assert_eq!(std::path::Path::new("a\\b.svg").file_stem(), Some(std::ffi::OsStr::new("b")));
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

    fn named_batch(s: &mut Session, dest: &std::path::Path, naming: Option<&str>) -> Value {
        let mut p = json!({
            "source": "datasets", "destination": "folder", "destinationFolder": dest.to_string_lossy(),
            "commands": [{"command": "object.unlockAll", "params": {}}]
        });
        if let Some(naming) = naming {
            p["datasetFileNaming"] = json!(naming);
        }
        s.execute("action.batch", &p).unwrap()
    }

    #[test]
    fn batch_data_set_naming_choices_and_omitted_default() {
        for (i, (naming, want)) in [
            (Some("fileNumber"), ["source001.vectorcraft", "source002.vectorcraft"]),
            (Some("fileDataSetName"), ["source_Alpha.vectorcraft", "source_Beta.vectorcraft"]),
            (Some("dataSetName"), ["Alpha.vectorcraft", "Beta.vectorcraft"]),
            (None, ["source_Alpha.vectorcraft", "source_Beta.vectorcraft"]),
        ]
        .into_iter()
        .enumerate()
        {
            let dest = tmp(&format!("choice-{i}"));
            let mut s = with_data_sets(&["Alpha", "Beta"]);
            let saved = tmp(&format!("saved-{i}")).join("source.vectorcraft");
            s.execute("document.save", &json!({"path": saved.to_string_lossy()})).unwrap();
            let r = named_batch(&mut s, &dest, naming);
            assert_eq!(r["failed"], 0, "{r}");
            assert_eq!(file_names(&dest), want, "{naming:?}: {r}");
        }
    }

    #[test]
    fn batch_file_number_uses_three_digits_through_500() {
        let names: Vec<String> = (1..=500).map(|n| format!("Set {n}")).collect();
        let run = Run {
            label: "Test",
            steps: &[],
            stop_on_error: true,
            override_save: true,
            override_export: true,
            destination: "none",
            dest_folder: None,
            dataset_file_naming: "fileDataSetName",
        };
        let stems = dataset_stems(&names, "source", "fileNumber", &run, &vectorcraft_doc::Document::new(100.0, 80.0)).unwrap();
        for n in [1, 2, 10, 100, 500] {
            assert_eq!(stems[n - 1], format!("source{n:03}"));
        }
    }

    #[test]
    fn batch_unsaved_data_set_names_and_empty_fallback() {
        let dest = tmp("unsaved-number");
        let mut s = with_data_sets(&["Alpha"]);
        assert_eq!(named_batch(&mut s, &dest, Some("fileNumber"))["failed"], 0);
        assert_eq!(file_names(&dest), ["Untitled001.vectorcraft"]);
        let dest = tmp("unsaved-name");
        assert_eq!(named_batch(&mut s, &dest, Some("fileDataSetName"))["failed"], 0);
        assert_eq!(file_names(&dest), ["Untitled_Alpha.vectorcraft"]);
        let run = Run {
            label: "Test",
            steps: &[],
            stop_on_error: true,
            override_save: true,
            override_export: true,
            destination: "none",
            dest_folder: None,
            dataset_file_naming: "fileDataSetName",
        };
        assert_eq!(dataset_stems(&[String::new()], "Untitled", "dataSetName", &run, &s.doc().unwrap().doc).unwrap(), ["Untitled001"]);
        assert_eq!(dataset_stems(&["   ".to_string()], "Untitled", "dataSetName", &run, &s.doc().unwrap().doc).unwrap(), ["Untitled001"]);
    }

    #[test]
    fn batch_data_set_name_collisions_and_existing_files() {
        let clear = tmp("choice-collision-clear");
        let mut s = with_data_sets(&["a:b", "a-b"]);
        let r = named_batch(&mut s, &clear, Some("dataSetName"));
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&clear), ["a-b-2.vectorcraft", "a-b.vectorcraft"]);
        let dest = tmp("choice-collision");
        std::fs::write(dest.join("a-b.vectorcraft"), b"keep").unwrap();
        let mut s = with_data_sets(&["a:b", "a-b"]);
        let r = named_batch(&mut s, &dest, Some("dataSetName"));
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(std::fs::read(dest.join("a-b.vectorcraft")).unwrap(), b"keep");
        assert_eq!(file_names(&dest), ["a-b-2.vectorcraft", "a-b-3.vectorcraft", "a-b.vectorcraft"]);
    }

    #[test]
    fn batch_data_set_name_avoids_windows_device_files() {
        let dest = tmp("choice-device");
        let mut s = with_data_sets(&["CON"]);
        let r = named_batch(&mut s, &dest, Some("dataSetName"));
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&dest), ["CON-.vectorcraft"]);
    }

    #[test]
    fn batch_override_export_keeps_existing_output() {
        let dest = tmp("choice-existing-export");
        std::fs::write(dest.join("Alpha.svg"), b"keep").unwrap();
        let mut s = with_data_sets(&["Alpha"]);
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets", "datasetFileNaming": "dataSetName",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.export", "params": {"format": "svg"}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(std::fs::read(dest.join("Alpha.svg")).unwrap(), b"keep");
        assert_eq!(file_names(&dest), ["Alpha-2.svg", "Alpha.svg"]);
    }

    #[test]
    fn batch_artboard_export_keeps_existing_output() {
        let dest = tmp("choice-existing-artboard-export");
        let existing = dest.join("Alpha-Artboard-1.png");
        std::fs::write(&existing, b"keep").unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20, "artboards": 2})).unwrap();
        s.execute("dataset.new", &json!({"name": "Alpha"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets", "datasetFileNaming": "dataSetName",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.export", "params": {"format": "png", "useArtboards": true}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(std::fs::read(existing).unwrap(), b"keep");
        assert_eq!(file_names(&dest), ["Alpha-2-Artboard-1.png", "Alpha-2-Artboard-2.png", "Alpha-Artboard-1.png"]);
    }

    #[test]
    fn batch_remembered_separate_artboards_keeps_existing_output() {
        let source = tmp("choice-remembered-separate-source");
        let dest = tmp("choice-remembered-separate-dest");
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20, "artboards": 2})).unwrap();
        s.execute("document.save", &json!({"path": source.join("source.vectorcraft").to_string_lossy(), "separateArtboards": true})).unwrap();
        assert!(source.join("source-Artboard-1.vectorcraft").exists());
        let existing = dest.join("Alpha-Artboard-1.vectorcraft");
        std::fs::write(&existing, b"keep").unwrap();
        s.execute("dataset.new", &json!({"name": "Alpha"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets", "datasetFileNaming": "dataSetName",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.save"}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(std::fs::read(existing).unwrap(), b"keep");
        assert_eq!(
            file_names(&dest),
            ["Alpha-2-Artboard-1.vectorcraft", "Alpha-2-Artboard-2.vectorcraft", "Alpha-2.vectorcraft", "Alpha-Artboard-1.vectorcraft",]
        );
    }

    #[test]
    fn batch_single_artboard_save_keeps_existing_artboard_file() {
        let source = tmp("choice-single-artboard-source");
        let dest = tmp("choice-single-artboard-dest");
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20, "artboards": 1})).unwrap();
        s.execute(
            "document.save",
            &json!({
                "path": source.join("source.vectorcraft").to_string_lossy(),
                "separateArtboards": true
            }),
        )
        .unwrap();
        assert!(source.join("source-Artboard-1.vectorcraft").exists());

        let existing = dest.join("Alpha-Artboard-1.vectorcraft");
        std::fs::write(&existing, b"keep").unwrap();
        s.execute("dataset.new", &json!({"name": "Alpha"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets", "datasetFileNaming": "dataSetName",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.save", "params": {"separateArtboards": true}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(std::fs::read(existing).unwrap(), b"keep");
        assert_eq!(file_names(&dest), ["Alpha-2-Artboard-1.vectorcraft", "Alpha-2.vectorcraft", "Alpha-Artboard-1.vectorcraft"]);
    }

    #[test]
    fn batch_pdf_export_ignores_artboard_named_file() {
        let dest = tmp("choice-pdf-artboard-name");
        let existing = dest.join("Alpha-Artboard-1.pdf");
        std::fs::write(&existing, b"keep").unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20, "artboards": 2})).unwrap();
        s.execute("dataset.new", &json!({"name": "Alpha"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets", "datasetFileNaming": "dataSetName",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.export", "params": {"format": "pdf"}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(std::fs::read(existing).unwrap(), b"keep");
        assert_eq!(file_names(&dest), ["Alpha-Artboard-1.pdf", "Alpha.pdf"]);
    }

    #[test]
    fn batch_separate_artboards_ignores_unrelated_extension() {
        let dest = tmp("choice-separate-unrelated-extension");
        let unrelated = dest.join("Alpha-photo.png");
        std::fs::write(&unrelated, b"keep").unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20, "artboards": 2})).unwrap();
        s.execute("dataset.new", &json!({"name": "Alpha"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets", "datasetFileNaming": "dataSetName",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.save", "params": {"separateArtboards": true}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
        assert_eq!(file_names(&dest), ["Alpha-Artboard-1.vectorcraft", "Alpha-Artboard-2.vectorcraft", "Alpha-photo.png", "Alpha.vectorcraft"]);
    }

    #[test]
    fn batch_separate_artboards_reserves_prior_output_names() {
        let dest = tmp("choice-separate-reserved");
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20, "artboards": 2})).unwrap();
        s.execute("dataset.new", &json!({"name": "Alpha"})).unwrap();
        s.execute("dataset.new", &json!({"name": "Alpha-Artboard-1"})).unwrap();
        let r = s
            .execute(
                "action.batch",
                &json!({
                    "source": "datasets", "datasetFileNaming": "dataSetName",
                    "destinationFolder": dest.to_string_lossy(),
                    "commands": [{"command": "document.save", "params": {"separateArtboards": true}}]
                }),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert!(dest.join("Alpha-Artboard-1.vectorcraft").exists());
        assert!(dest.join("Alpha-Artboard-1-2.vectorcraft").exists());
    }

    #[test]
    fn batch_does_not_probe_a_denied_destination() {
        let allowed = tmp("probe-allowed");
        let denied = tmp("probe-denied");
        std::fs::write(denied.join("Alpha.svg"), b"private").unwrap();
        let roots = crate::file_access::AutomationRoots::new(&[allowed.as_path()], &[allowed.as_path()]).unwrap().unwrap();
        let steps = [json!({"command": "document.export", "params": {"format": "svg"}})];
        let folder = denied.to_string_lossy().into_owned();
        let run = Run {
            label: "Test",
            steps: &steps,
            stop_on_error: true,
            override_save: true,
            override_export: true,
            destination: "none",
            dest_folder: Some(&folder),
            dataset_file_naming: "fileDataSetName",
        };
        crate::file_access::confine(Some(&roots), || {
            assert_eq!(
                dataset_stems(&["Alpha".into()], "source", "dataSetName", &run, &vectorcraft_doc::Document::new(100.0, 80.0)).unwrap(),
                ["Alpha"]
            );
            let mut s = with_data_sets(&["Alpha"]);
            let result = s
                .execute(
                    "action.batch",
                    &json!({
                        "source": "datasets", "datasetFileNaming": "dataSetName", "destination": "none",
                        "destinationFolder": folder, "overrideExport": true,
                        "commands": steps,
                    }),
                )
                .unwrap();
            assert!(!result.to_string().contains("Alpha-2"), "{result}");
        });
    }

    #[test]
    fn batch_does_not_probe_unknown_format_paths() {
        let root = tmp("probe-format");
        let folder = root.join("out");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(root.join("outside"), b"private").unwrap();
        for format in ["../outside", "x/../../outside", "..\\..\\outside"] {
            let steps = [json!({"command": "document.export", "params": {"format": format}})];
            let folder = folder.to_string_lossy().into_owned();
            let run = Run {
                label: "Test",
                steps: &steps,
                stop_on_error: true,
                override_save: true,
                override_export: true,
                destination: "none",
                dest_folder: Some(&folder),
                dataset_file_naming: "fileDataSetName",
            };
            assert_eq!(
                dataset_stems(&["Alpha".into()], "source", "dataSetName", &run, &vectorcraft_doc::Document::new(100.0, 80.0))
                    .unwrap_or_else(|e| panic!("{format}: {e}")),
                ["Alpha"],
                "{format}"
            );
            assert_eq!(rewrite_paths(&steps, false, true, Some(&folder), "Alpha", "vectorcraft"), steps, "{format}");
        }
    }

    #[test]
    fn batch_probe_and_dot_names_stay_in_destination() {
        let root = tmp("probe-parent");
        let folder = root.join("out");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(root.join("outside.svg"), b"private").unwrap();
        let folder = folder.to_string_lossy().into_owned();
        let run = Run {
            label: "Test",
            steps: &[],
            stop_on_error: true,
            override_save: true,
            override_export: true,
            destination: "none",
            dest_folder: Some(&folder),
            dataset_file_naming: "fileDataSetName",
        };
        assert_eq!(
            dataset_stems(&[".".into(), "..".into()], ".", "dataSetName", &run, &vectorcraft_doc::Document::new(100.0, 80.0)).unwrap(),
            ["Untitled001", "Untitled002"]
        );
        assert_eq!(
            dataset_stems(&["Alpha".into()], "..", "fileNumber", &run, &vectorcraft_doc::Document::new(100.0, 80.0)).unwrap(),
            ["Untitled001"]
        );
        let output = root.join("named");
        let mut s = with_data_sets(&[".", ".."]);
        let r = named_batch(&mut s, &output, Some("dataSetName"));
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(file_names(&output), ["Untitled001.vectorcraft", "Untitled002.vectorcraft"]);
    }

    #[test]
    fn batch_probe_rejects_parent_escape() {
        let root = tmp("probe-parent-only");
        let folder = root.join("out");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(root.join("outside.svg"), b"private").unwrap();
        let folder = folder.to_string_lossy().into_owned();
        assert!(!dataset_output_exists(
            Some(&folder),
            "../outside",
            &[DatasetOutput { ext: "svg".into(), artboards: false, joiner: "-" }],
            &vectorcraft_doc::Document::new(100.0, 80.0),
        ));
    }

    #[test]
    fn batch_rejects_unknown_data_set_naming_but_ignores_it_for_folders() {
        let mut s = with_data_sets(&["Alpha"]);
        let e = s
            .execute("action.batch", &json!({"source": "datasets", "datasetFileNaming": "other", "commands": [{"command": "object.unlockAll"}]}))
            .unwrap_err();
        let msg = e.to_string();
        for value in ["fileNumber", "fileDataSetName", "dataSetName"] {
            assert!(msg.contains(value), "{msg}");
        }
        assert!(
            s.execute("action.batch", &json!({"source": "datasets", "datasetFileNaming": 7, "commands": [{"command": "object.unlockAll"}]})).is_err()
        );
        let src = tmp("folder-ignore-src");
        write_svg(&src, "one.svg", "#f00");
        let r = s.execute("action.batch", &json!({"source": "folder", "folder": src.to_string_lossy(), "datasetFileNaming": "other", "commands": [{"command": "object.unlockAll"}]})).unwrap();
        assert_eq!(r["failed"], 0, "{r}");
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

    /// A saved document with a text object bound to `Name` and two data sets, `A` and `B`.
    fn saved_with_two_data_sets(path: &std::path::Path) -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 40, "text": "A"})).unwrap()["id"].as_u64().unwrap();
        s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
        s.execute("variable.bind", &json!({"variable": "Name", "ids": [id]})).unwrap();
        s.execute("dataset.capture", &json!({"name": "A"})).unwrap();
        s.execute("text.setText", &json!({"ids": [id], "text": "B"})).unwrap();
        s.execute("dataset.capture", &json!({"name": "B"})).unwrap();
        s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
        s
    }

    fn save_and_close_over_data_sets(s: &mut Session) -> Result<Value> {
        s.execute(
            "action.batch",
            &json!({"source": "datasets", "destination": "saveAndClose", "commands": [{"command": "object.unlockAll", "params": {}}]}),
        )
    }

    #[test]
    fn batch_save_and_close_over_data_sets_saves_each_over_the_template_and_warns() {
        let path = tmp("save-close-sets").join("template.vectorcraft");
        let mut s = saved_with_two_data_sets(&path);
        std::fs::write(&path, b"stale").unwrap();
        let r = save_and_close_over_data_sets(&mut s).unwrap();
        assert_eq!((r["count"].as_u64(), r["failed"].as_u64()), (Some(2), Some(0)), "{r}");
        assert_eq!(r["warning"], ONE_FILE_WARNING, "{r}");
        assert_ne!(std::fs::read(&path).unwrap(), b"stale", "the template's file is saved over");
        assert_eq!(file_names(path.parent().unwrap()), ["template.vectorcraft"], "{r}");
        let st = s.doc().unwrap();
        assert_eq!(st.doc.variables.active_dataset.as_deref(), Some("B"), "the last data set is applied");
        assert_eq!(st.path.as_deref(), Some(path.to_string_lossy().as_ref()), "the template stays open");
        assert!(!st.is_dirty(), "the last data set is saved");
    }

    #[test]
    fn batch_warns_when_a_recorded_save_writes_one_file_for_every_data_set() {
        let path = tmp("recorded-save-sets").join("template.vectorcraft");
        let mut s = saved_with_two_data_sets(&path);
        let save = json!([{"command": "document.save", "params": {}}]);
        let r = s.execute("action.batch", &json!({"source": "datasets", "commands": save})).unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(r["warning"], ONE_FILE_WARNING, "{r}");
        let dest = tmp("recorded-save-dest");
        let p = json!({"source": "datasets", "destination": "folder", "destinationFolder": dest.to_string_lossy(), "commands": save});
        let r = s.execute("action.batch", &p).unwrap();
        assert!(r.get("warning").is_none(), "Override Save points each save into the folder: {r}");
        let unlock = json!({"source": "datasets", "commands": [{"command": "object.unlockAll", "params": {}}]});
        let r = s.execute("action.batch", &unlock).unwrap();
        assert!(r.get("warning").is_none(), "{r}");
    }

    #[test]
    fn batch_save_and_close_after_a_redirected_save_writes_each_data_set_to_its_own_file() {
        let save_close = |dest: &std::path::Path| {
            json!({
                "source": "datasets",
                "destination": "saveAndClose",
                "destinationFolder": dest.to_string_lossy(),
                "commands": [{"command": "document.save", "params": {}}]
            })
        };
        let path = tmp("redirected-save-close").join("template.vectorcraft");
        let mut s = saved_with_two_data_sets(&path);
        let bytes = std::fs::read(&path).unwrap();
        let dest = tmp("redirected-save-close-dest");
        let p = save_close(&dest);
        let mut kept = p.clone();
        kept["overrideSave"] = json!(false);
        assert!(batch_overwrites_one_file(&s, &kept), "without Override Save the save stays on the template");
        let r = s.execute("action.batch", &p).unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert!(r.get("warning").is_none(), "{r}");
        assert_eq!(file_names(&dest), ["template_A.vectorcraft", "template_B.vectorcraft"], "{r}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "the template's file stays as it was");
        // A document never saved gets its files from the redirected save too.
        let mut unsaved = with_data_sets(&["a", "b"]);
        let dest = tmp("redirected-save-close-unsaved");
        let r = unsaved.execute("action.batch", &save_close(&dest)).unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert!(r.get("warning").is_none(), "{r}");
        assert_eq!(file_names(&dest), ["Untitled_a.vectorcraft", "Untitled_b.vectorcraft"], "{r}");
        // A template save writes a copy: Save and Close still saves over the document's own file.
        let mut template = save_close(&dest);
        template["commands"] = json!([{"command": "document.save", "params": {"format": "template"}}]);
        assert!(batch_overwrites_one_file(&s, &template), "{template}");
        let mut never = with_data_sets(&["a", "b"]);
        let msg = never.execute("action.batch", &template).unwrap_err().to_string();
        assert!(msg.ends_with("the document has never been saved"), "{msg}");
        // A recorded Save As with its own path gives a never-saved document its file, the same one
        // for every data set.
        let one = tmp("save-as-then-close").join("one.vectorcraft");
        let p = json!({
            "source": "datasets",
            "destination": "saveAndClose",
            "commands": [{"command": "file.saveAs", "params": {"path": one.to_string_lossy()}}]
        });
        let r = never.execute("action.batch", &p).unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_eq!(r["warning"], ONE_FILE_WARNING, "{r}");
        assert!(one.exists(), "{r}");
    }

    #[test]
    fn one_file_rule_covers_save_and_close_and_steps_left_in_place() {
        let saved = saved_with_two_data_sets(&tmp("one-file-rule").join("template.vectorcraft"));
        let unsaved = with_data_sets(&["a"]);
        let with = |extra: Value| {
            let mut p = json!({"source": "datasets", "commands": [{"command": "document.export", "params": {"path": "/x/a.png"}}]});
            for (k, v) in extra.as_object().unwrap() {
                p[k] = v.clone();
            }
            batch_overwrites_one_file(&saved, &p)
        };
        assert!(with(json!({})), "an export with its own path");
        assert!(!with(json!({"destination": "folder", "destinationFolder": "/out"})));
        assert!(with(json!({"destination": "folder", "destinationFolder": "/out", "overrideExport": false})));
        assert!(!with(json!({"destination": "folder", "destinationFolder": "/out", "overrideSave": false})));
        assert!(!with(json!({"source": "folder"})), "a folder source opens a file per pass");
        assert!(with(json!({"destination": "saveAndClose", "commands": [{"command": "object.unlockAll"}]})));
        let saved_as = json!({"commands": [{"command": "file.saveAs", "params": {"path": "/x/a.svg"}}]});
        assert!(with(saved_as.clone()));
        assert!(!with(json!({"destination": "folder", "destinationFolder": "/out", "commands": saved_as["commands"]})));
        assert!(!with(json!({"commands": [{"command": "file.saveAs", "params": {}}]})), "no path: the data comes back");
        assert!(!with(json!({"commands": [{"command": "document.export", "params": {"format": "png"}}]})), "no path: the data comes back");
        let copy = json!([{"command": "file.saveCopy", "params": {"path": "/x/a.vectorcraft"}}]);
        assert!(with(json!({"destination": "folder", "destinationFolder": "/out", "commands": copy})), "Override Save leaves copies alone");
        let save = |format: Option<&str>| {
            let params = format.map_or(json!({}), |f| json!({"format": f}));
            json!({"source": "datasets", "commands": [{"command": "document.save", "params": params}]})
        };
        assert!(batch_overwrites_one_file(&saved, &save(None)), "the document's own file");
        assert!(batch_overwrites_one_file(&saved, &save(Some("vectorcraft"))), "the document's own format");
        assert!(!batch_overwrites_one_file(&unsaved, &save(None)), "a document never saved has no file to write");
        assert!(!batch_overwrites_one_file(&saved, &save(Some("svg"))), "another format without a path: the data comes back");
        assert!(!with(json!({"commands": [{"command": "file.saveCopy", "params": {"folder": "/x"}}]})), "a folder alone: the data comes back");
        let screens = json!([{"command": "document.exportForScreens", "params": {"folder": "/x"}}]);
        assert!(with(json!({"destination": "folder", "destinationFolder": "/out", "commands": screens})), "Override Export leaves its folder");
        assert!(!with(json!({"commands": [{"command": "document.exportForScreens", "params": {}}]})));
        let preset = json!([{"command": "document.exportForScreens", "params": {"exportPreset": "Logo Package"}}]);
        assert!(!with(json!({"commands": preset})), "a preset without a folder: the data comes back");
        for id in ["document.exportPdf", "document.exportEps", "document.exportDxf", "document.exportForWeb", "document.exportForOffice"] {
            let step = json!([{"command": id, "params": {"path": "/x/a.out"}}]);
            assert!(with(json!({"destination": "folder", "destinationFolder": "/out", "commands": step})), "{id} keeps its path");
            assert!(!with(json!({"commands": [{"command": id, "params": {}}]})), "{id} without a path: the data comes back");
        }
    }

    #[test]
    fn batch_refuses_save_and_close_over_data_sets_of_a_converted_or_recovered_document() {
        for converted in [true, false] {
            let path = tmp(if converted { "converted-sets" } else { "recovered-sets" }).join("template.vectorcraft");
            let mut s = saved_with_two_data_sets(&path);
            let bytes = std::fs::read(&path).unwrap();
            let (active, revision) = {
                let st = s.doc_mut().unwrap();
                if converted {
                    st.converted = true;
                } else {
                    st.recovered = true;
                }
                (st.doc.variables.active_dataset.clone(), st.revision)
            };
            let msg = save_and_close_over_data_sets(&mut s).unwrap_err().to_string();
            assert!(msg.ends_with("save the document with Save As before Save and Close"), "{msg}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file is not written");
            let st = s.doc().unwrap();
            assert_eq!(st.doc.variables.active_dataset, active, "no data set is selected");
            assert_eq!(st.revision, revision, "no step runs");
        }
    }

    #[test]
    fn batch_refuses_save_and_close_over_data_sets_of_an_unsaved_document() {
        let mut s = with_data_sets(&["a", "b"]);
        let (active, revision) = {
            let st = s.doc().unwrap();
            (st.doc.variables.active_dataset.clone(), st.revision)
        };
        let msg = save_and_close_over_data_sets(&mut s).unwrap_err().to_string();
        assert!(msg.ends_with("the document has never been saved"), "{msg}");
        let st = s.doc().unwrap();
        assert_eq!(st.doc.variables.active_dataset, active, "no data set is selected");
        assert_eq!(st.revision, revision, "no step runs");
        assert!(st.path.is_none());
    }

    #[test]
    fn batch_save_and_close_saves_and_closes_each_source_file() {
        let src = tmp("save-close-src");
        let a = write_svg(&src, "a.svg", "#f00");
        let before = std::fs::read(&a).unwrap();
        let mut s = Session::new();
        let r = s
            .execute(
                "action.batch",
                &json!({"folder": src.to_string_lossy(), "destination": "saveAndClose", "commands": [{"command": "object.unlockAll", "params": {}}]}),
            )
            .unwrap();
        assert_eq!(r["failed"], 0, "{r}");
        assert_ne!(std::fs::read(&a).unwrap(), before, "the source file is saved over");
        assert_eq!(file_names(&src), ["a.svg"], "{r}");
        assert!(s.doc().is_err(), "the source file is closed");
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
