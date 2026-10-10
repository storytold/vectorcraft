//! Optional confinement of the MCP server's file access to chosen folders (`#832`).
//!
//! `vectorcraft-cli mcp --automation-read-root <dir> --automation-write-root <dir>` starts the
//! server confined: agent-supplied file paths must resolve beneath the matching root, with
//! symlinks resolved first. With neither flag the server keeps its legacy behaviour (any path
//! the user could use). With either flag set, confinement is on and the side without a root is
//! denied (fail closed), so passing one flag alone still confines the other direction.
//!
//! Checks run at the agent boundary (the MCP `dispatch` in [`crate::tools`]), before anything
//! reaches a backend, so they hold for headless and remote backends alike. What they cover:
//!
//! - the file tools' paths (`open_file`, `save_file`, `export`, `screenshot {path}`);
//! - every engine command run through `command_run` / `run_command` / `invoke_menu` /
//!   `command_batch` (including commands nested in an engine `command.batch`), per the policy
//!   table in [`check_command`]: reads (`document.open`, `file.place` and its info/queue,
//!   `links.relink`, library and preset loads, `plugin.install`, `color.loadProfile`, font
//!   searches) against the read root, writes (`document.save`, exports, `file.package`,
//!   `file.print`, `css.export`, library and preset saves) against the write root.
//!
//! Deliberately out of scope: paths the agent never supplies. A document the desktop user opened
//! themselves may live outside the roots, so saving it without a path, embedding its links, or
//! packaging its linked files can still touch outside files. When confined, pathless saves (and
//! other pathless variants that would write somewhere implicit, like a library save to the user
//! folder) are rejected outright; pass an explicit path under the write root instead. Variants
//! that provably return their bytes inline (`document.export` without a path, preset exports
//! without a path) stay allowed.

use std::path::{Path, PathBuf};

/// Canonicalized confinement roots for agent-supplied file paths. `None` per side; both `None`
/// means legacy unconstrained behaviour (no flag was given).
#[derive(Clone, Debug, Default)]
pub struct FileRoots {
    read: Option<PathBuf>,
    write: Option<PathBuf>,
}

impl FileRoots {
    /// No confinement (neither flag given): every check passes.
    pub fn unconstrained() -> Self {
        Self::default()
    }

    /// Canonicalize both roots (they must exist) for `--automation-read-root` /
    /// `--automation-write-root`. Either may be `None`; when one side is missing while the other
    /// is set, that side denies everything (fail closed).
    pub fn new(read_root: Option<&str>, write_root: Option<&str>) -> Result<Self, String> {
        let canonical =
            |what: &str, dir: &str| std::fs::canonicalize(dir).map_err(|e| format!("cannot use {what} `{dir}`: {e} (it must be an existing folder)"));
        Ok(Self {
            read: read_root.filter(|r| !r.trim().is_empty()).map(|r| canonical("--automation-read-root", r)).transpose()?,
            write: write_root.filter(|w| !w.trim().is_empty()).map(|w| canonical("--automation-write-root", w)).transpose()?,
        })
    }

    /// Whether either flag was given (any check can deny).
    pub fn confined(&self) -> bool {
        self.read.is_some() || self.write.is_some()
    }

    /// Agent-supplied `path` must resolve beneath the read root (`origin` names the tool or
    /// command for the error).
    pub fn check_read(&self, origin: &str, path: &str) -> Result<(), String> {
        if !self.confined() {
            return Ok(());
        }
        let Some(root) = &self.read else {
            return Err(format!("{origin}: reads are not allowed (no --automation-read-root was given)"));
        };
        let resolved =
            resolve(path).ok_or_else(|| format!("{origin}: cannot resolve `{path}` inside --automation-read-root `{}`", root.display()))?;
        if !resolved.starts_with(root) {
            return Err(format!("{origin}: `{path}` is outside --automation-read-root `{}`", root.display()));
        }
        Ok(())
    }

    /// Agent-supplied `path` must resolve beneath the write root.
    pub fn check_write(&self, origin: &str, path: &str) -> Result<(), String> {
        if !self.confined() {
            return Ok(());
        }
        let Some(root) = &self.write else {
            return Err(format!("{origin}: writes are not allowed (no --automation-write-root was given)"));
        };
        let resolved =
            resolve(path).ok_or_else(|| format!("{origin}: cannot resolve `{path}` inside --automation-write-root `{}`", root.display()))?;
        if !resolved.starts_with(root) {
            return Err(format!("{origin}: `{path}` is outside --automation-write-root `{}`", root.display()));
        }
        Ok(())
    }

    /// Reject a pathless file command whose no-path form would write somewhere implicit (the
    /// document's own file, the user library folder…), when confined. Variants that provably
    /// return bytes inline need no such gate.
    pub fn require_write_path(&self, origin: &str) -> Result<(), String> {
        if !self.confined() {
            return Ok(());
        }
        let Some(root) = &self.write else {
            return Err(format!("{origin}: writes are not allowed (no --automation-write-root was given)"));
        };
        Err(format!("{origin}: give an explicit `path` under --automation-write-root `{}`", root.display()))
    }
}

/// Resolve `path` with symlinks first: a path that exists canonicalizes whole; one that does
/// not (a file about to be written) canonicalizes through its nearest existing ancestor, the
/// missing tail re-appended verbatim. `None` when nothing anchors it (empty paths, absolute
/// paths with no existing ancestor, or an unresolvable working directory).
///
/// Soundness note: the ancestor is fully canonical (no symlinks left) and the tail does not
/// exist (so it holds no symlinks either), which makes applying the tail's `.` / `..`
/// lexically exact. This matters: re-appending a tail containing `..` verbatim would let
/// `<root>/../outside/x` pass a prefix check for `<root>` while living outside it.
fn resolve(path: &str) -> Option<PathBuf> {
    if path.trim().is_empty() {
        return None;
    }
    let full = Path::new(path);
    if let Ok(canonical) = std::fs::canonicalize(full) {
        return Some(canonical);
    }
    // Lexical ancestors, immediate parent first.
    let mut ancestors = vec![];
    let mut cursor = full;
    while let Some(parent) = cursor.parent().filter(|p| !p.as_os_str().is_empty()) {
        ancestors.push(parent);
        cursor = parent;
    }
    // The nearest existing ancestor anchors the resolution.
    for ancestor in ancestors {
        if let Ok(canonical) = std::fs::canonicalize(ancestor) {
            let tail = full.strip_prefix(ancestor).ok()?;
            return Some(apply_tail(canonical, tail));
        }
    }
    // Nothing above exists (absolute paths always keep `/`, so only relative paths land here):
    // anchor at the working directory.
    if full.is_absolute() {
        return None;
    }
    let cwd = std::fs::canonicalize(".").ok()?;
    Some(apply_tail(cwd, full))
}

/// Append `tail` to the canonical `base`, resolving `.` / `..` lexically (sound here: `base`
/// holds no symlinks and `tail` does not exist). Anything else fails closed.
fn apply_tail(mut base: PathBuf, tail: &Path) -> PathBuf {
    use std::path::Component::{CurDir, Normal, ParentDir, Prefix, RootDir};
    for component in tail.components() {
        match component {
            Prefix(_) | RootDir => {
                base = PathBuf::from("/");
            }
            CurDir => {}
            ParentDir => {
                base.pop();
            }
            Normal(part) => base.push(part),
        }
    }
    base
}

/// A non-empty string param, if present.
fn param(params: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    params.get(key).and_then(serde_json::Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string)
}

/// Every non-empty string of an array param.
fn param_list(params: &serde_json::Map<String, serde_json::Value>, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Gate one engine command's file params (`origin` names it for errors). Unknown commands pass:
/// only the file commands below touch the filesystem through agent-supplied paths. `path`-like
/// params of other commands are object ids or inline data, never host files.
///
/// `command.batch` nests further commands and is checked recursively, so a batch cannot smuggle
/// a file command past the gate.
pub fn check_command(roots: &FileRoots, id: &str, params: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    if !roots.confined() {
        return Ok(());
    }
    // (command, read-path params, read-path list params)
    const READS: &[(&str, &[&str], &[&str])] = &[
        ("document.open", &["path"], &[]),
        ("file.newFromTemplate", &["path"], &[]),
        ("document.pdfInfo", &["path"], &[]),
        ("file.place", &["path"], &[]),
        ("file.place.info", &["path"], &[]),
        ("file.place.queue", &[], &["paths"]),
        ("links.relink", &["path", "folder"], &[]),
        ("swatch.library.load", &["path"], &[]),
        ("graphicStyle.loadLibrary", &["path"], &[]),
        ("pdf.preset.import", &["path"], &[]),
        ("print.presets.import", &["path"], &[]),
        ("flattener.presets.import", &["path"], &[]),
        ("perspective.presets.import", &["path"], &[]),
        ("plugin.install", &["path"], &[]),
        ("plugin.reload", &["path"], &[]),
        ("color.loadProfile", &["path"], &[]),
        ("text.findFontFiles", &["folder"], &[]),
        ("text.addFontFiles", &[], &["files"]),
    ];
    // (command, write-path params, pathless form writes somewhere implicit and is denied)
    const WRITES: &[(&str, &[&str], bool)] = &[
        ("document.save", &["path"], true),
        ("file.saveAs", &["path"], false),
        ("file.saveCopy", &["path"], false),
        ("file.saveAsTemplate", &["path"], false),
        ("document.export", &["path"], false),
        ("document.exportSelection", &["path"], false),
        ("document.exportForOffice", &["path"], false),
        ("document.exportForScreens", &["folder"], false),
        ("document.exportForWeb", &["path"], false),
        ("file.package", &["folder"], false),
        ("file.print", &["path"], false),
        ("css.export", &["path"], false),
        ("swatch.library.save", &["path"], true),
        ("graphicStyle.saveLibrary", &["path"], true),
        ("pdf.preset.export", &["path"], false),
        ("print.presets.export", &["path"], false),
        ("flattener.presets.export", &["path"], false),
        ("perspective.presets.export", &["path"], false),
    ];
    if id == "command.batch" {
        let steps =
            params.get("commands").and_then(serde_json::Value::as_array).ok_or_else(|| "command.batch: `commands` must be an array".to_string())?;
        for (i, step) in steps.iter().enumerate() {
            let args = step.as_object().ok_or_else(|| format!("command.batch: step {i} must be an object"))?;
            let nested = args
                .get("command")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("command.batch: step {i} is missing `command`"))?;
            let nested_params = match args.get("params") {
                None | Some(serde_json::Value::Null) => serde_json::Map::new(),
                Some(serde_json::Value::Object(o)) => o.clone(),
                Some(_) => return Err(format!("command.batch: step {i} `params` must be an object")),
            };
            check_command(roots, nested, &nested_params).map_err(|e| format!("command.batch step {i} ({nested}): {e}"))?;
        }
        return Ok(());
    }
    if id == "prefs.set" {
        return check_prefs(roots, id, params);
    }
    // `plugin.reload` without a path loads the configured plug-ins folder (and executes what
    // it finds there): implicit and uncheckable, so denied when confined.
    if id == "plugin.reload" && param(params, "path").is_none() {
        return Err(format!("{id}: reloading the configured plug-ins folder is not allowed when confined; pass an explicit in-root `path`"));
    }
    for (cmd, paths, lists) in READS {
        if *cmd == id {
            for key in *paths {
                if let Some(p) = param(params, key) {
                    roots.check_read(id, &p)?;
                }
            }
            for key in *lists {
                for p in param_list(params, key) {
                    roots.check_read(id, &p)?;
                }
            }
            return Ok(());
        }
    }
    for (cmd, paths, deny_pathless) in WRITES {
        if *cmd == id {
            let mut given = false;
            for key in *paths {
                if let Some(p) = param(params, key) {
                    given = true;
                    roots.check_write(id, &p)?;
                }
            }
            if !given && *deny_pathless {
                roots.require_write_path(id)?;
            }
            return Ok(());
        }
    }
    Ok(())
}

/// `prefs.set` folder keys redirect the app's own file access (font scans, plug-in loads,
/// recovery copies, template files), so confined values must stay under the roots. Empty values
/// clear the preference and touch nothing.
fn check_prefs(roots: &FileRoots, id: &str, params: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let mut values: Vec<(&str, &str)> = vec![];
    if let Some(key) = params.get("key").and_then(serde_json::Value::as_str)
        && let Some(value) = params.get("value").and_then(serde_json::Value::as_str)
    {
        values.push((key, value));
    }
    if let Some(map) = params.get("values").and_then(serde_json::Value::as_object) {
        values.extend(map.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), s))));
    }
    for (key, value) in values {
        if value.trim().is_empty() {
            continue;
        }
        match key {
            "fontsFolder" | "pluginsFolder" | "templatesFolder" => roots.check_read(id, value)?,
            "recoveryFolder" => {
                roots.check_write(id, value)?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch tree: `base/inside` (the roots) and `base/outside`. Returns the base and roots
    /// confining both directions to `inside`.
    fn confined_tree(name: &str) -> (PathBuf, FileRoots) {
        let base = std::env::temp_dir().join(format!("vectorcraft-roots-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("inside")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        let inside = base.join("inside").to_string_lossy().to_string();
        let roots = FileRoots::new(Some(&inside), Some(&inside)).unwrap();
        (base, roots)
    }

    fn params(pairs: &[(&str, &str)]) -> serde_json::Map<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect()
    }

    #[test]
    fn unconstrained_roots_allow_everything() {
        let roots = FileRoots::unconstrained();
        assert!(!roots.confined());
        assert!(roots.check_read("t", "/etc/passwd").is_ok());
        assert!(roots.check_write("t", "/etc/passwd").is_ok());
        assert!(roots.require_write_path("t").is_ok());
        assert!(check_command(&roots, "document.open", &params(&[("path", "/etc/passwd")])).is_ok());
    }

    #[test]
    fn bad_roots_fail_at_startup() {
        assert!(FileRoots::new(Some("/nonexistent-dir-xyz"), None).is_err());
        assert!(FileRoots::new(None, Some("/nonexistent-dir-xyz")).is_err());
        // Empty flags behave as absent: no confinement at all.
        let roots = FileRoots::new(Some("  "), None).unwrap();
        assert!(!roots.confined());
        assert!(roots.check_read("t", "/x").is_ok());
    }

    #[test]
    fn one_sided_roots_fail_the_other_side_closed() {
        let (base, _) = confined_tree("one-sided");
        let inside = base.join("inside").to_string_lossy().to_string();
        let read_only = FileRoots::new(Some(&inside), None).unwrap();
        assert!(read_only.check_read("t", &format!("{inside}/a.svg")).is_ok());
        assert!(read_only.check_write("t", &format!("{inside}/a.svg")).is_err());
        let write_only = FileRoots::new(None, Some(&inside)).unwrap();
        assert!(write_only.check_write("t", &format!("{inside}/a.svg")).is_ok());
        assert!(write_only.check_read("t", &format!("{inside}/a.svg")).is_err());
    }

    #[test]
    fn inside_passes_and_outside_fails() {
        let (base, roots) = confined_tree("in-out");
        let (inside, outside) = (base.join("inside"), base.join("outside"));
        std::fs::write(inside.join("art.svg"), b"<svg/>").unwrap();
        let inp = inside.join("art.svg").to_string_lossy().to_string();
        let outp = outside.join("art.svg").to_string_lossy().to_string();
        assert!(roots.check_read("open_file", &inp).is_ok());
        assert!(roots.check_write("export", &inp).is_ok());
        // A new file inside the root resolves through its nearest existing ancestor.
        assert!(roots.check_write("export", &inside.join("new/file.svg").to_string_lossy()).is_ok());
        // …while `..` past the root resolves outside, wherever it is written lexically.
        let escape = inside.join("new").join("..").join("..").join("outside").join("x.svg");
        assert!(roots.check_write("export", &escape.to_string_lossy()).is_err());
        let err = roots.check_read("open_file", &outp).unwrap_err();
        assert!(err.contains("outside --automation-read-root"), "{err}");
        // Missing files outside fail closed (nothing to anchor a read to).
        assert!(roots.check_read("open_file", &outside.join("missing.svg").to_string_lossy()).is_err());
    }

    #[test]
    fn traversal_and_prefix_traps_fail() {
        let (base, roots) = confined_tree("traps");
        let inside = base.join("inside");
        // `..` escapes back out.
        let escape = inside.join("sub").join("..").join("..").join("outside").join("x.svg").to_string_lossy().to_string();
        assert!(roots.check_read("t", &escape).is_err());
        // String-prefix confusion (`inside2` starts with `inside`): component-wise, denied.
        let sibling = base.join("inside2");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("x.svg"), b"x").unwrap();
        assert!(roots.check_read("t", &sibling.join("x.svg").to_string_lossy()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;
        let (base, roots) = confined_tree("symlink");
        let (inside, outside) = (base.join("inside"), base.join("outside"));
        std::fs::write(outside.join("secret.svg"), b"<svg/>").unwrap();
        symlink(&outside, inside.join("link")).unwrap();
        assert!(roots.check_read("t", &inside.join("link/secret.svg").to_string_lossy()).is_err());
        assert!(roots.check_write("t", &inside.join("link/new.svg").to_string_lossy()).is_err());
        assert!(!outside.join("new.svg").exists());
    }

    #[test]
    fn engine_command_table_gates_reads_and_writes() {
        let (_, roots) = confined_tree("table");
        // Reads.
        for id in ["document.open", "file.place", "links.relink", "swatch.library.load", "pdf.preset.import", "plugin.install", "color.loadProfile"] {
            assert!(check_command(&roots, id, &params(&[("path", "/etc/passwd")])).is_err(), "{id}");
        }
        assert!(
            check_command(
                &roots,
                "file.place.queue",
                &serde_json::Map::from_iter([("paths".to_string(), serde_json::json!(["/etc/a.svg", "/etc/b.svg"]),)])
            )
            .is_err()
        );
        // Writes.
        for id in ["document.save", "document.export", "file.package", "swatch.library.save", "css.export"] {
            let key = if id == "file.package" { "folder" } else { "path" };
            assert!(check_command(&roots, id, &params(&[(key, "/etc/x")])).is_err(), "{id}");
        }
        // Inline data variants (no path) pass; the engine validates the rest.
        assert!(check_command(&roots, "document.open", &params(&[])).is_ok());
        assert!(check_command(&roots, "document.export", &params(&[])).is_ok());
        assert!(
            check_command(&roots, "swatch.library.load", &serde_json::Map::from_iter([("dataBase64".to_string(), serde_json::json!("e30="),)]))
                .is_ok()
        );
        // Pathless saves that would write somewhere implicit are denied.
        assert!(check_command(&roots, "document.save", &params(&[])).is_err());
        assert!(check_command(&roots, "swatch.library.save", &params(&[])).is_err());
        assert!(check_command(&roots, "plugin.reload", &params(&[])).is_err());
        // Unrelated commands (`path` as an object id, paint, …) pass through untouched.
        assert!(check_command(&roots, "text.createInPath", &params(&[("path", "12")])).is_ok());
        assert!(check_command(&roots, "object.group", &params(&[])).is_ok());
    }

    #[test]
    fn batch_nesting_cannot_smuggle_file_commands() {
        let (_, roots) = confined_tree("batch");
        let nested = serde_json::Map::from_iter([(
            "commands".to_string(),
            serde_json::json!([{"command": "shape.rectangle", "params": {}}, {"command": "document.open", "params": {"path": "/etc/passwd"}}]),
        )]);
        assert!(check_command(&roots, "command.batch", &nested).is_err());
        let clean = serde_json::Map::from_iter([("commands".to_string(), serde_json::json!([{"command": "shape.rectangle", "params": {}}]))]);
        assert!(check_command(&roots, "command.batch", &clean).is_ok());
    }

    #[test]
    fn prefs_folder_keys_stay_under_the_roots() {
        let (base, roots) = confined_tree("prefs");
        let inside = base.join("inside").to_string_lossy().to_string();
        let kv =
            |k: &str, v: &str| serde_json::Map::from_iter([("key".to_string(), serde_json::json!(k)), ("value".to_string(), serde_json::json!(v))]);
        assert!(check_command(&roots, "prefs.set", &kv("pluginsFolder", "/etc")).is_err());
        assert!(check_command(&roots, "prefs.set", &kv("pluginsFolder", &inside)).is_ok());
        assert!(check_command(&roots, "prefs.set", &kv("recoveryFolder", "/etc")).is_err());
        // Clearing a folder pref touches nothing.
        assert!(check_command(&roots, "prefs.set", &kv("pluginsFolder", "")).is_ok());
        // Unrelated prefs pass.
        assert!(check_command(&roots, "prefs.set", &kv("unitsGeneral", "Millimeters")).is_ok());
    }
}
