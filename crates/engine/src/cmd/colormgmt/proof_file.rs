//! View → Proof Setup → Customize…'s Save… and Load…: a custom proof setup (the device to simulate,
//! Preserve Numbers, the rendering intent, Simulate Paper Color and Simulate Black Ink) in a file,
//! and the setups saved in the `Proof Setups` folder next to the preferences, which View → Proof
//! Setup lists ([`Session::attach_proof_setups`]). `view.proofSetup.read` reads a file without
//! applying it, which is how Load… fills the dialog.
//!
//! The file format is VectorCraft's own: JSON with `format` [`FORMAT`], `version` 1, `target`,
//! `intent`, `simulatePaper`, `preserveNumbers` and `simulateBlack`, as `view.proofSetup` names
//! them, at most [`MAX_FILE`] bytes.

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, ProofSetup, ProofTarget};
use vectorcraft_render::proof;

use super::super::{EngineError, fileio};
use super::{Result, Session, bad, setup_params, store_view, text_param, view_json};

/// The extension of a proof setup file.
pub const EXT: &str = "vcproof";
/// The `format` a proof setup file names.
pub const FORMAT: &str = "vectorcraft-proof-setup";
/// The format version this build writes and reads.
const VERSION: u64 = 1;
/// The largest proof setup file `view.proofSetup.load` reads, bytes (VectorCraft's own limit).
pub const MAX_FILE: usize = 64 << 10;
/// The most setups `view.proofSetup.list` lists.
pub const MAX_LISTED: usize = 100;
/// The most folder entries `view.proofSetup.list` looks at, setup files or not.
const MAX_SCANNED: usize = 100_000;
/// The longest `name`, in characters.
const MAX_NAME: usize = 100;

impl Session {
    /// Keep the proof setups that `view.proofSetup.save` saves by name in the folder `dir` (`Proof
    /// Setups` next to the preferences), which `view.proofSetup.list` lists. The folder is made
    /// when a setup is first saved there.
    pub fn attach_proof_setups(&mut self, dir: std::path::PathBuf) {
        self.proof_setup_dir = Some(dir);
    }

    /// The saved proof setups' folder ([`Session::attach_proof_setups`]); none on the web.
    pub fn proof_setup_dir(&self) -> Option<&std::path::Path> {
        self.proof_setup_dir.as_deref().filter(|_| !cfg!(target_arch = "wasm32"))
    }

    /// How many proof setup files `view.proofSetup.save` wrote in this session; the menus list the
    /// Proof Setups folder again when it changes.
    pub fn proof_setup_saves(&self) -> u64 {
        self.proof_setup_saves
    }
}

/// The file `p` names for `cmd`: its `path`, or the setup `name` in the Proof Setups folder.
fn file_path(s: &Session, p: &Value, cmd: &str) -> Result<(String, bool)> {
    match (text_param(p, "path", cmd)?, text_param(p, "name", cmd)?) {
        (Some(path), None) if path.trim().is_empty() => Err(bad(cmd, "`path` is empty")),
        (Some(path), None) => Ok((path.to_string(), false)),
        (None, Some(name)) => {
            let name = setup_name(name, cmd)?;
            let dir = s.proof_setup_dir().ok_or_else(|| bad(cmd, "saved proof setups need the preferences folder, which this run doesn't have"))?;
            let path = dir.join(format!("{name}.{EXT}"));
            let path = path.to_str().ok_or_else(|| bad(cmd, format!("{}: the folder's path is not valid Unicode", dir.display())))?;
            Ok((path.to_string(), true))
        }
        (Some(_), Some(_)) => Err(bad(cmd, "give `path` or `name`, not both")),
        (None, None) => Err(bad(cmd, "missing `path` or `name`")),
    }
}

/// `name` trimmed, when it can name a file in the Proof Setups folder: not empty, at most
/// [`MAX_NAME`] characters, no path or control characters, and not starting with a dot.
fn setup_name<'a>(name: &'a str, cmd: &str) -> Result<&'a str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(bad(cmd, "`name` is empty"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(bad(cmd, format!("`name` is longer than {MAX_NAME} characters")));
    }
    if name.starts_with('.') || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err(bad(cmd, format!("`name` can't hold path characters: `{name}`")));
    }
    Ok(name)
}

/// `Ok` when `t` is a device a custom proof setup can simulate and its profile is loaded.
fn check_device(t: &ProofTarget) -> std::result::Result<(), String> {
    let loaded = |name: &str, kind| cms::profile(name).is_some_and(|k| k.kind == kind);
    match t {
        ProofTarget::WorkingCmyk => Ok(()),
        ProofTarget::Cmyk(name) if loaded(name, cms::ProfileKind::Cmyk) => Ok(()),
        ProofTarget::Rgb(name) if loaded(name, cms::ProfileKind::Rgb) => Ok(()),
        ProofTarget::Cmyk(name) => Err(format!("the device to simulate is the CMYK profile `{name}`, which is not loaded")),
        ProofTarget::Rgb(name) => Err(format!("the device to simulate is the RGB profile `{name}`, which is not loaded")),
        t => Err(format!("`{}` is not a device to simulate: a custom proof setup simulates workingCmyk, cmyk:<profile> or rgb:<profile>", t.id())),
    }
}

/// The settings of `setup`, as `view.proofSetup` names them.
fn setup_json(setup: &ProofSetup) -> serde_json::Map<String, Value> {
    let v = json!({
        "target": setup.target.id(),
        "intent": setup.intent.id(),
        "simulatePaper": setup.simulate_paper,
        "preserveNumbers": setup.preserve_numbers,
        "simulateBlack": setup.simulate_black,
    });
    match v {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// The proof setup in a file's `bytes`, or why it can't be read.
pub fn parse(bytes: &[u8]) -> std::result::Result<ProofSetup, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("not a proof setup file: {e}"))?;
    if v.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err("not a VectorCraft proof setup file".into());
    }
    match v.get("version").and_then(Value::as_u64) {
        Some(VERSION) => {}
        Some(n) if n > VERSION => return Err(format!("the file is version {n}, made by a newer VectorCraft")),
        _ => return Err("the file has no version VectorCraft reads".into()),
    }
    let text = |key: &str| v.get(key).and_then(Value::as_str);
    let target = text("target").ok_or("the file names no device to simulate")?;
    let target = ProofTarget::parse(target).ok_or_else(|| format!("unknown proof target `{target}`"))?;
    let intent = text("intent").ok_or("the file names no rendering intent")?;
    let intent = Intent::parse(intent).ok_or_else(|| format!("unknown intent `{intent}`"))?;
    let on = |key: &str| match v.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("`{key}` must be true or false")),
    };
    Ok(ProofSetup {
        target,
        intent,
        simulate_paper: on("simulatePaper")?,
        preserve_numbers: on("preserveNumbers")?,
        simulate_black: on("simulateBlack")?,
        separations: None,
    })
}

/// A proof setup file holding `setup`.
pub fn encode(setup: &ProofSetup) -> Vec<u8> {
    let mut m = serde_json::Map::new();
    m.insert("format".into(), json!(FORMAT));
    m.insert("version".into(), json!(VERSION));
    m.extend(setup_json(setup));
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(m)).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

pub(super) fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.proofSetup.save";
    let (path, named) = file_path(s, p, C)?;
    let mut setup = proof::view().setup;
    setup_params(p, &mut setup, C)?;
    check_device(&setup.target).map_err(|e| bad(C, e))?;
    // A file `view.proofSetup.load` would refuse is not written (a device with a very long name).
    let bytes = encode(&setup);
    if bytes.len() > MAX_FILE {
        return Err(bad(C, format!("{path}: larger than {MAX_FILE} bytes")));
    }
    if named
        && let Some(dir) = std::path::Path::new(&path).parent()
        && !dir.is_dir()
    {
        crate::file_access::check_write(&path).map_err(EngineError::Other)?;
        std::fs::create_dir_all(dir).map_err(|e| bad(C, format!("{}: {e}", dir.display())))?;
    }
    fileio::write_file(&path, &bytes)?;
    s.proof_setup_saves += 1;
    let mut r = setup_json(&setup);
    r.insert("path".into(), json!(path));
    Ok(Value::Object(r))
}

/// The file `p` names for `cmd` and the usable proof setup in it.
fn read_setup(s: &Session, p: &Value, cmd: &str) -> Result<(String, ProofSetup)> {
    let (path, _) = file_path(s, p, cmd)?;
    // One byte past the limit shows a file over it, which is not read further.
    let bytes = fileio::read_file_head(&path, MAX_FILE + 1)?;
    if bytes.len() > MAX_FILE {
        return Err(bad(cmd, format!("{path}: larger than {MAX_FILE} bytes")));
    }
    let setup = parse(&bytes).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    check_device(&setup.target).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    Ok((path, setup))
}

pub(super) fn read(s: &mut Session, p: &Value) -> Result<Value> {
    let (path, setup) = read_setup(s, p, "view.proofSetup.read")?;
    let mut r = setup_json(&setup);
    r.insert("path".into(), json!(path));
    Ok(Value::Object(r))
}

pub(super) fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let (path, setup) = read_setup(s, p, "view.proofSetup.load")?;
    let mut v = proof::view();
    // Separations Preview's plates are not part of a proof setup file.
    v.setup = ProofSetup { separations: v.setup.separations.take(), ..setup };
    v.proof_colors = true;
    store_view(s, v.clone());
    let mut r = view_json(&v);
    r["path"] = json!(path);
    Ok(r)
}

pub(super) fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let none = json!({"folder": null, "setups": [], "truncated": false});
    let Some(dir) = s.proof_setup_dir() else { return Ok(none) };
    listed(dir).map(|l| l.unwrap_or(none))
}

/// The web keeps no saved setups ([`Session::proof_setup_dir`]).
#[cfg(target_arch = "wasm32")]
pub fn listed(_: &std::path::Path) -> Result<Option<Value>> {
    Ok(None)
}

/// The listing of the Proof Setups folder `dir`, as `view.proofSetup.list` gives it; `None` for a
/// request that may not read it. The app calls it on another thread, without a session.
#[cfg(not(target_arch = "wasm32"))]
pub fn listed(dir: &std::path::Path) -> Result<Option<Value>> {
    const C: &str = "view.proofSetup.list";
    let folder = dir.to_string_lossy().into_owned();
    // A request that may not read the folder learns nothing of it, nor where it is.
    if crate::file_access::check_read(&folder).is_err() {
        return Ok(None);
    }
    // The listed paths are text: a folder whose path is not valid Unicode can't be named in them.
    if dir.to_str().is_none() {
        return Err(bad(C, format!("{folder}: the folder's path is not valid Unicode")));
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // No folder yet: nothing has been saved.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Some(json!({"folder": folder, "setups": [], "truncated": false}))),
        Err(e) => return Err(bad(C, format!("{folder}: {e}"))),
    };
    // The setups' names, the first MAX_LISTED by name; other entries are passed over before the
    // limit applies. A link is not followed. A file is listed only when it is named
    // `<name>.vcproof` and `name` takes that name unchanged, so each listed name loads its own
    // file. An entry that can't be read, or whose file name is not valid Unicode, leaves the
    // listing incomplete.
    let (mut names, mut truncated) = (Vec::<String>::new(), false);
    for (i, e) in entries.enumerate() {
        if i == MAX_SCANNED {
            truncated = true;
            break;
        }
        let Ok(e) = e else {
            truncated = true;
            continue;
        };
        let Ok(file) = e.file_name().into_string() else {
            truncated = true;
            continue;
        };
        let Some(name) = file.strip_suffix(EXT).and_then(|s| s.strip_suffix('.')) else { continue };
        if setup_name(name, C).ok() != Some(name) {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_file() => {}
            Ok(_) => continue,
            Err(_) => {
                truncated = true;
                continue;
            }
        }
        names.push(name.to_string());
        if names.len() > 2 * MAX_LISTED {
            names.sort();
            names.truncate(MAX_LISTED);
            truncated = true;
        }
    }
    names.sort();
    truncated |= names.len() > MAX_LISTED;
    names.truncate(MAX_LISTED);
    let setups: Vec<Value> = names.iter().map(|name| json!({"name": name, "path": dir.join(format!("{name}.{EXT}")).to_string_lossy()})).collect();
    Ok(Some(json!({"folder": folder, "setups": setups, "truncated": truncated})))
}
