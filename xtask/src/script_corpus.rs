//! `cargo xtask corpus --scripts` and `cargo xtask scripts --pass-rate`: public scripts from MIT and
//! Apache-2.0 repositories at pinned commits, and a report of how many of them run headless in the
//! script host.
//!
//! `xtask/script-corpus.txt` lists each repository with its commit and license, and each file with
//! its size and SHA-256: the repository's license file, the scripts that are run, the files they
//! include, and the candidates left out (a script over [`MAX_SCRIPT_BYTES`]). No script text is
//! committed.
//!
//! The fetch downloads each listed file from raw.githubusercontent.com at its commit into
//! `corpus/scripts/<owner>_<repo>/<path>` (gitignored, never committed), with the license file
//! beside the scripts. A download stops past its listed size, is checked against its size and
//! SHA-256 in a staging folder and only then moved into place. A copy already there that matches is
//! kept, and a copy that does not match stays until a verified one replaces it. One fetch runs at a
//! time: the fetch holds `corpus/scripts/.fetch.lock` while it runs.
//!
//! The pass-rate run never downloads. Without `corpus/scripts` it prints that it skipped and exits
//! successfully. Otherwise it checks every listed file against the manifest, builds the engine's
//! `script_corpus` test in release mode and runs each script in a process of its own: a new session
//! with one new document, `script.run` with [`FUEL`] instructions and no dialog answers, reads only
//! in the script's repository folder and the run's own folder, and writes only in the run's own
//! folder, which holds the run's temporary folder and data folder and is deleted after the script.
//! A process still running after [`TIMEOUT`] is stopped. Script failures never fail the command;
//! a manifest, a corpus file or a report path that is wrong does. The report is JSON, with a
//! summary on the console; it names each script by its repository and path and holds no script
//! text.
//!
//! The limits, the fuel, the timeout and the report's outcomes are VectorCraft's own.
//!
//! Moving a pin: change the repository's `source` line, list its files with their new sizes and
//! SHA-256, and review the manifest diff.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::{cargo, root, run, sha256};

/// The manifest, relative to the repository root.
pub const MANIFEST: &str = "xtask/script-corpus.txt";
/// The largest manifest read.
const MAX_MANIFEST_BYTES: u64 = 1 << 20;
/// The largest script or included file fetched.
pub const MAX_SCRIPT_BYTES: u64 = 1 << 20;
/// The largest license file fetched.
pub const MAX_LICENSE_BYTES: u64 = 1 << 20;
/// The most bytes of scripts and included files in the corpus.
pub const MAX_SCRIPTS_TOTAL: u64 = 64 << 20;
/// The most bytes one fetch downloads.
pub const MAX_TRANSFER: u64 = 128 << 20;
/// The instructions each script may run (`script.run`'s `maxFuel`).
pub const FUEL: u64 = 5_000_000;
/// How long one script's process may run before it is stopped.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// The most bytes of a script process's output read.
const MAX_ROW_OUTPUT: u64 = 1 << 20;
/// The licenses a repository in the manifest may have (SPDX ids).
const LICENSES: &[&str] = &["MIT", "Apache-2.0"];
/// Windows device names, which no name in a path may be.
const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL"];
/// What a script process prints before its result.
const ROW_PREFIX: &str = "script-corpus-row ";
/// Names that show a dialog or a window when a call follows them (spaces and line breaks may come
/// before the `(`), and names of file dialogs wherever they appear. A script that has none of them
/// is in the low-interaction subset.
const UI_CALLS: &[&str] = &["alert", "confirm", "prompt", "Window"];
const UI_NAMES: &[&str] = &["openDialog", "saveDialog", "selectDialog", "openDlg", "saveDlg", "selectDlg"];
/// The script host's errors for a file dialog with nobody to answer it, and for file access it
/// refuses (`crates/engine/src/script/files.rs`, `crates/engine/src/file_access.rs`).
const NO_PICKER: &str = "Script file dialogs need a person to answer them";
const DENIED: &[&str] = &["Script file access denied", "automation path rejected", "automation filesystem access is not granted"];

const SKIPPED: &str = "skipped: script corpus absent; run cargo xtask corpus --scripts";

pub const SCRIPTS_USAGE: &str = "usage: cargo xtask scripts --pass-rate [--report <path>]";

/// What a manifest entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The repository's license file.
    License,
    /// A script that is run.
    Script,
    /// A file that scripts include, not run on its own.
    Include,
    /// A candidate that is neither fetched nor run.
    Excluded,
}

#[derive(Debug)]
pub struct Entry {
    pub kind: Kind,
    pub bytes: u64,
    pub sha256: String,
    pub path: String,
}

#[derive(Debug)]
pub struct Source {
    pub repo: String,
    pub commit: String,
    pub license: String,
    pub entries: Vec<Entry>,
}

impl Source {
    /// The folder under `corpus/scripts` that holds its files.
    pub fn dir(&self) -> String {
        self.repo.replace('/', "_")
    }

    fn fetched(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind != Kind::Excluded)
    }

    fn scripts(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == Kind::Script)
    }
}

/// Reads and checks the manifest.
fn load(root: &Path) -> Result<(Vec<Source>, String), String> {
    let path = root.join(MANIFEST);
    let len = std::fs::metadata(&path).map_err(|e| format!("{MANIFEST}: {e}"))?.len();
    if len > MAX_MANIFEST_BYTES {
        return Err(format!("{MANIFEST} is {len} bytes, over the {MAX_MANIFEST_BYTES}-byte limit"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| format!("{MANIFEST} is not UTF-8"))?;
    Ok((parse(text)?, sha256::hex(&bytes)))
}

/// The sources the manifest text lists, checked line by line and as a whole.
pub fn parse(text: &str) -> Result<Vec<Source>, String> {
    let mut sources: Vec<Source> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let at = |m: String| format!("{MANIFEST} line {}: {m}", i + 1);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("source ") {
            sources.push(parse_source(rest).map_err(at)?);
            continue;
        }
        let entry = parse_entry(line).map_err(at)?;
        let Some(source) = sources.last_mut() else { return Err(at("an entry comes before any source line".into())) };
        source.entries.push(entry);
    }
    check_whole(&sources)?;
    Ok(sources)
}

fn parse_source(rest: &str) -> Result<Source, String> {
    let fields: Vec<&str> = rest.split(' ').collect();
    let [repo, commit, license] = fields[..] else { return Err("a source line is `source <owner/repo> <commit> <SPDX id>`".into()) };
    let names: Vec<&str> = repo.split('/').collect();
    let named = |n: &str| !n.is_empty() && !n.starts_with('.') && n.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if names.len() != 2 || !names.iter().all(|n| named(n)) {
        return Err(format!("`{repo}` is not an owner/repository name"));
    }
    if !is_hex(commit, 40) {
        return Err(format!("`{commit}` is not a full commit id (40 lowercase hex digits)"));
    }
    if !LICENSES.contains(&license) {
        return Err(format!("license `{license}` is not one of {}", LICENSES.join(", ")));
    }
    Ok(Source { repo: repo.into(), commit: commit.into(), license: license.into(), entries: Vec::new() })
}

fn parse_entry(line: &str) -> Result<Entry, String> {
    let mut fields = line.splitn(4, ' ');
    let (Some(kind), Some(bytes), Some(sha), Some(path)) = (fields.next(), fields.next(), fields.next(), fields.next()) else {
        return Err("an entry line is `<kind> <bytes> <sha256> <path>`".into());
    };
    let kind = match kind {
        "license" => Kind::License,
        "script" => Kind::Script,
        "include" => Kind::Include,
        "excluded" => Kind::Excluded,
        other => return Err(format!("unknown kind `{other}` (license, script, include or excluded)")),
    };
    let bytes: u64 =
        bytes.parse().ok().filter(|_| bytes.bytes().all(|b| b.is_ascii_digit())).ok_or_else(|| format!("`{bytes}` is not a byte count"))?;
    if !is_hex(sha, 64) {
        return Err(format!("`{sha}` is not a SHA-256 (64 lowercase hex digits)"));
    }
    check_path(path)?;
    let limit = match kind {
        Kind::License => MAX_LICENSE_BYTES,
        Kind::Script | Kind::Include => MAX_SCRIPT_BYTES,
        Kind::Excluded => u64::MAX,
    };
    if bytes > limit {
        return Err(format!("{path} is {bytes} bytes, over the {limit}-byte limit"));
    }
    if kind == Kind::Script && !is_script(path) {
        return Err(format!("{path}: a script must be a .js or .jsx file"));
    }
    Ok(Entry { kind, bytes, sha256: sha.into(), path: path.into() })
}

/// Checks that hold for the manifest as a whole: one license and at least one script in each
/// repository, no two files whose paths differ only in letter case, no file where another needs a
/// folder, and the totals.
fn check_whole(sources: &[Source]) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    let mut folders = HashSet::new();
    let (mut scripts, mut transfer) = (0u64, 0u64);
    for s in sources {
        if !seen.insert(s.dir().to_lowercase()) {
            return Err(format!("{MANIFEST}: {} is listed twice, or its folder name differs from another's only in letter case", s.repo));
        }
        if s.entries.iter().filter(|e| e.kind == Kind::License).count() != 1 {
            return Err(format!("{MANIFEST}: {} needs exactly one license entry", s.repo));
        }
        if s.scripts().next().is_none() {
            return Err(format!("{MANIFEST}: {} lists no script", s.repo));
        }
        for e in s.fetched() {
            let full = format!("{}/{}", s.dir(), e.path).to_lowercase();
            if !seen.insert(full.clone()) {
                return Err(format!("{MANIFEST}: {} {} is listed twice, or differs from another path only in letter case", s.repo, e.path));
            }
            let mut rest = full.as_str();
            while let Some((parent, _)) = rest.rsplit_once('/') {
                folders.insert(parent.to_owned());
                rest = parent;
            }
            files.push((full, s, e));
            transfer += e.bytes;
            if e.kind != Kind::License {
                scripts += e.bytes;
            }
        }
    }
    if let Some((_, s, e)) = files.iter().find(|(full, _, _)| folders.contains(full)) {
        return Err(format!("{MANIFEST}: {} {} is a file where another entry needs a folder", s.repo, e.path));
    }
    if scripts > MAX_SCRIPTS_TOTAL {
        return Err(format!("{MANIFEST}: the scripts add up to {scripts} bytes, over the {MAX_SCRIPTS_TOTAL}-byte limit"));
    }
    if transfer > MAX_TRANSFER {
        return Err(format!("{MANIFEST}: the files add up to {transfer} bytes, over the {MAX_TRANSFER}-byte limit"));
    }
    Ok(())
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".js") || lower.ends_with(".jsx")
}

/// A relative path with `/` between its names that is the same file on every platform: no empty,
/// `.` or `..` name, no character Windows refuses, no name that ends in a space or a dot, and no
/// device name.
fn check_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 1024 {
        return Err("a path has 1 to 1024 bytes".into());
    }
    if path.starts_with('/') {
        return Err(format!("`{path}` is an absolute path"));
    }
    if let Some(c) = path.chars().find(|c| c.is_control() || matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')) {
        return Err(format!("`{path}` has the character {c:?}, which a path here may not have"));
    }
    for name in path.split('/') {
        if name.is_empty() || name == "." || name == ".." {
            return Err(format!("`{path}` has an empty, `.` or `..` name"));
        }
        if name.starts_with(' ') || name.ends_with(' ') || name.ends_with('.') {
            return Err(format!("`{path}` has a name that starts or ends with a space or ends with a dot"));
        }
        let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
        let numbered = stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit());
        if RESERVED.contains(&stem.as_str()) || numbered {
            return Err(format!("`{path}` has the device name {name}"));
        }
    }
    Ok(())
}

/// `cargo xtask corpus --scripts`.
pub fn fetch() -> Result<(), String> {
    let root = root();
    let (sources, _) = load(&root)?;
    let dest = root.join("corpus").join("scripts");
    std::fs::create_dir_all(&dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    let _lock = Lock::take(&dest.join(".fetch.lock"))?;
    let staging = dest.join(format!(".staging-{}", std::process::id()));
    let result = fetch_into(&sources, &dest, &staging, MAX_TRANSFER, &mut download);
    let _ = std::fs::remove_dir_all(&staging);
    let done = result?;
    std::fs::write(dest.join("SOURCES.md"), sources_md(&sources)).map_err(|e| format!("SOURCES.md: {e}"))?;
    let scripts: usize = sources.iter().map(|s| s.scripts().count()).sum();
    println!(
        "scripts: {scripts} from {} repositories verified in {} ({} files kept, {} downloaded, {} bytes)",
        sources.len(),
        dest.display(),
        done.kept,
        done.fetched,
        done.bytes
    );
    Ok(())
}

/// A downloader: fetch the URL into the file, stopping past the byte count.
type Download<'a> = dyn FnMut(&str, &Path, u64) -> Result<(), String> + 'a;

#[derive(Debug, Default, PartialEq, Eq)]
struct Fetched {
    kept: usize,
    fetched: usize,
    bytes: u64,
}

/// Puts every fetched file of `sources` in `dest`, downloading at most `limit` bytes in all through
/// `staging` with `get`.
fn fetch_into(sources: &[Source], dest: &Path, staging: &Path, limit: u64, get: &mut Download) -> Result<Fetched, String> {
    std::fs::create_dir_all(staging).map_err(|e| format!("create {}: {e}", staging.display()))?;
    let mut done = Fetched::default();
    for s in sources {
        for e in s.fetched() {
            let out = dest.join(s.dir()).join(&e.path);
            if verified(&out, e)? {
                done.kept += 1;
                continue;
            }
            if done.bytes + e.bytes > limit {
                return Err(format!("{} {}: the fetch would download more than {limit} bytes", s.repo, e.path));
            }
            let part = staging.join(format!("{}.part", done.kept + done.fetched));
            let got = get(&raw_url(s, e), &part, e.bytes).and_then(|()| check_file(&part, e));
            if let Err(err) = got {
                let _ = std::fs::remove_file(&part);
                return Err(format!("{} {}: {err}", s.repo, e.path));
            }
            place(dest, &s.dir(), e, &part)?;
            done.fetched += 1;
            done.bytes += e.bytes;
        }
    }
    Ok(done)
}

/// Is the file at `path` there with the entry's size and SHA-256? Its size is read before any of
/// it; a link or a folder there is an error.
fn verified(path: &Path, e: &Entry) -> Result<bool, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    if !meta.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() != e.bytes {
        return Ok(false);
    }
    let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(sha256::hex(&bytes) == e.sha256)
}

/// A downloaded file has the entry's size and SHA-256.
fn check_file(path: &Path, e: &Entry) -> Result<(), String> {
    let len = std::fs::metadata(path).map_err(|err| format!("{}: {err}", path.display()))?.len();
    if len != e.bytes {
        return Err(format!("the download is {len} bytes, and {MANIFEST} lists {}", e.bytes));
    }
    let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let got = sha256::hex(&bytes);
    if got != e.sha256 {
        return Err(format!("sha256 {got} does not match {MANIFEST}"));
    }
    Ok(())
}

/// Moves the verified `part` to its place under `dest`, making its folders; a link or a file on the
/// way is an error.
fn place(dest: &Path, dir: &str, e: &Entry, part: &Path) -> Result<(), String> {
    let mut folder = dest.to_path_buf();
    let names: Vec<&str> = std::iter::once(dir).chain(e.path.split('/')).collect();
    let (file, folders) = names.split_last().ok_or("an empty path")?;
    for name in folders {
        folder.push(name);
        match std::fs::symlink_metadata(&folder) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => return Err(format!("{} is not a folder", folder.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&folder).map_err(|err| format!("create {}: {err}", folder.display()))?;
            }
            Err(err) => return Err(format!("{}: {err}", folder.display())),
        }
    }
    let out = folder.join(file);
    std::fs::rename(part, &out).map_err(|err| format!("move into {}: {err}", out.display()))
}

/// The file's address at its pinned commit, each byte outside letters, digits and `-._~/`
/// percent-encoded.
fn raw_url(s: &Source, e: &Entry) -> String {
    let mut path = String::new();
    for b in e.path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            path.push(char::from(b));
        } else {
            path.push_str(&format!("%{b:02X}"));
        }
    }
    format!("https://raw.githubusercontent.com/{}/{}/{path}", s.repo, s.commit)
}

/// Downloads `url` into `out` with curl: HTTPS only, no redirects, at most `max` bytes.
fn download(url: &str, out: &Path, max: u64) -> Result<(), String> {
    let mut curl = Command::new("curl");
    curl.args([
        "-fsS",
        "--proto",
        "=https",
        "--max-redirs",
        "0",
        "--retry",
        "3",
        "-A",
        "VectorCraft-dev",
        "--max-filesize",
        &max.max(1).to_string(),
        "-o",
    ])
    .arg(out)
    .arg(url);
    run(curl, &format!("curl {url}"))
}

fn sources_md(sources: &[Source]) -> String {
    let mut md = format!(
        "# Public scripts\n\nFetched by `cargo xtask corpus --scripts` and checked against `{MANIFEST}`.\nGitignored; never commit these files. Each folder holds its repository's license file.\n\n| Repository | Commit | License | Scripts |\n|---|---|---|---:|\n"
    );
    for s in sources {
        md.push_str(&format!("| https://github.com/{} | {} | {} | {} |\n", s.repo, s.commit, s.license, s.scripts().count()));
    }
    md
}

/// A file made with `create_new` and deleted when dropped.
struct Lock(PathBuf);

impl Lock {
    fn take(path: &Path) -> Result<Self, String> {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(_) => Ok(Self(path.to_path_buf())),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
                "{} exists: another `cargo xtask corpus --scripts` is running, or one stopped before it finished (delete the file when none is running)",
                path.display()
            )),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What `cargo xtask scripts` was asked for.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    report: Option<PathBuf>,
}

fn parse_args(args: &[&str]) -> Result<Args, String> {
    let (mut pass_rate, mut report) = (false, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--pass-rate" if !pass_rate => pass_rate = true,
            "--report" if report.is_none() => match it.next() {
                Some(p) if !p.is_empty() && !p.starts_with("--") => report = Some(PathBuf::from(p)),
                _ => return Err(format!("--report takes a file path\n\n{SCRIPTS_USAGE}")),
            },
            other => return Err(format!("unknown or repeated argument `{other}`\n\n{SCRIPTS_USAGE}")),
        }
    }
    if !pass_rate {
        return Err(format!("give --pass-rate\n\n{SCRIPTS_USAGE}"));
    }
    Ok(Args { report })
}

/// A report file made for this run: an existing file is an error.
fn create_report(path: &Path) -> Result<(PathBuf, std::fs::File), String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(format!("report {}: the folder {} does not exist", path.display(), parent.display()));
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(f) => Ok((path.to_path_buf(), f)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(format!("report {} already exists; give another --report path", path.display()))
        }
        Err(e) => Err(format!("report {}: {e}", path.display())),
    }
}

fn write_report(path: &Path, mut file: std::fs::File, report: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(report).map_err(|e| format!("report: {e}"))?;
    file.write_all(text.as_bytes()).and_then(|()| file.write_all(b"\n")).map_err(|e| format!("report {}: {e}", path.display()))?;
    println!("report: {}", path.display());
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `cargo xtask scripts --pass-rate [--report <path>]`.
pub fn scripts(args: &[&str]) -> Result<(), String> {
    let args = parse_args(args)?;
    let root = root();
    let (sources, manifest_sha) = load(&root)?;
    let dest = root.join("corpus").join("scripts");
    let given = args.report.as_deref().map(create_report).transpose()?;
    if !dest.is_dir() {
        println!("{SKIPPED}");
        if let Some((path, file)) = given {
            write_report(&path, file, &json!({"status": "skipped", "reason": SKIPPED}))?;
        }
        return Ok(());
    }
    let report = match check_corpus(&sources, &dest).and_then(|()| run_corpus(&root, &sources, &dest, manifest_sha)) {
        Ok(r) => r,
        Err(e) => {
            // The report file this command made is removed when it gets no report.
            if let Some((path, file)) = given {
                drop(file);
                let _ = std::fs::remove_file(path);
            }
            return Err(e);
        }
    };
    print_summary(&report);
    let (path, file) = match given {
        Some(r) => r,
        None => {
            let dir = root.join("target").join("script-corpus");
            std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
            create_report(&dir.join(format!("report-{}-{}.json", unix_seconds(), std::process::id())))?
        }
    };
    write_report(&path, file, &report)
}

/// Builds the runner, runs every script and gives the report.
fn run_corpus(root: &Path, sources: &[Source], dest: &Path, manifest_sha: String) -> Result<Value, String> {
    let exe = build_runner()?;
    let work = root.join("target").join("script-corpus").join(format!("run-{}-{}", unix_seconds(), std::process::id()));
    let rows = run_rows(sources, dest, &work, &exe);
    let _ = std::fs::remove_dir_all(&work);
    let base = Command::new("git").current_dir(root).args(["rev-parse", "HEAD"]).output().ok().filter(|o| o.status.success());
    let meta = json!({
        "base": base.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()),
        "manifestSha256": manifest_sha,
        "timestamp": unix_seconds(),
        "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        "fuelPerScript": FUEL,
        "timeoutSeconds": TIMEOUT.as_secs(),
    });
    Ok(build_report(meta, sources, &rows?))
}

/// Every fetched file is in the corpus with its listed size and SHA-256.
fn check_corpus(sources: &[Source], dest: &Path) -> Result<(), String> {
    for s in sources {
        for e in s.fetched() {
            if !verified(&dest.join(s.dir()).join(&e.path), e)? {
                return Err(format!("{} {} is missing or does not match {MANIFEST}; run cargo xtask corpus --scripts", s.repo, e.path));
            }
        }
    }
    Ok(())
}

/// Builds the engine's `script_corpus` test and returns its program.
fn build_runner() -> Result<PathBuf, String> {
    let what = "cargo test -p vectorcraft-engine --release --test script_corpus --no-run --message-format=json";
    eprintln!("$ {what}");
    let mut c = cargo();
    c.args(["test", "-p", "vectorcraft-engine", "--release", "--test", "script_corpus", "--no-run", "--message-format=json"])
        .stderr(Stdio::inherit());
    let out = c.output().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!("{what}: exited with {}", out.status));
    }
    runner_path(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| format!("{what} named no script_corpus test program"))
}

/// The `script_corpus` test program that cargo's JSON messages name.
fn runner_path(messages: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["reason"] == "compiler-artifact" && v["target"]["name"] == "script_corpus")
        .find_map(|v| v["executable"].as_str().map(PathBuf::from))
}

/// One script's result.
#[derive(Debug)]
pub struct Row {
    pub repo: String,
    pub path: String,
    pub low_interaction: bool,
    pub outcome: Outcome,
    /// What the script process gave (`Value::Null` when it gave nothing).
    pub detail: Value,
}

/// What became of a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Syntax,
    UnimplementedMember,
    Thrown,
    Headless,
    Denied,
    Limit,
    Timeout,
    Internal,
    Infrastructure,
}

impl Outcome {
    pub const ALL: [Outcome; 10] = [
        Outcome::Passed,
        Outcome::Syntax,
        Outcome::UnimplementedMember,
        Outcome::Thrown,
        Outcome::Headless,
        Outcome::Denied,
        Outcome::Limit,
        Outcome::Timeout,
        Outcome::Internal,
        Outcome::Infrastructure,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Syntax => "syntax",
            Outcome::UnimplementedMember => "unimplemented-member",
            Outcome::Thrown => "uncaught-throw",
            Outcome::Headless => "needs-answer-or-ui",
            Outcome::Denied => "denied-file-or-external",
            Outcome::Limit => "limit",
            Outcome::Timeout => "timeout",
            Outcome::Internal => "internal",
            Outcome::Infrastructure => "infrastructure",
        }
    }
}

/// The outcome of a script process's result. A run that ended well passes. A dialog with no answer
/// or a file dialog with nobody to answer it, and file access the host refused, come before the
/// failure's kind. A script `script.run` refused before it ran (text that is not UTF-8, a compiled
/// script) counts as a syntax failure.
pub fn classify(v: &Value) -> Outcome {
    if v["ok"] == true {
        return Outcome::Passed;
    }
    let message = v["message"].as_str().unwrap_or("");
    if v["missingAnswer"] == true || message.contains(NO_PICKER) {
        return Outcome::Headless;
    }
    if DENIED.iter().any(|d| message.contains(d)) {
        return Outcome::Denied;
    }
    if v["started"] != true {
        return Outcome::Syntax;
    }
    match v["kind"].as_str() {
        Some("syntax") => Outcome::Syntax,
        Some("thrown") => Outcome::Thrown,
        Some("limit" | "stopped") => Outcome::Limit,
        _ => Outcome::Internal,
    }
}

/// Does the script parse? Only a result that names its kind (or passed) says so.
fn parsed(row: &Row) -> bool {
    row.detail["started"] == true && row.detail["kind"] != "syntax"
}

/// Is the script free of dialogs and windows ([`UI_CALLS`], [`UI_NAMES`])? Names are matched as
/// whole identifiers.
fn low_interaction(text: &[u8]) -> bool {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut i = 0;
    while i < text.len() {
        if !word(text[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < text.len() && word(text[i]) {
            i += 1;
        }
        let name = &text[start..i];
        if UI_NAMES.iter().any(|n| n.as_bytes() == name) {
            return false;
        }
        if UI_CALLS.iter().any(|n| n.as_bytes() == name) && text[i..].iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'(') {
            return false;
        }
    }
    true
}

/// Runs every script of `sources` with the runner `exe`, one process each, in folders under `work`.
fn run_rows(sources: &[Source], dest: &Path, work: &Path, exe: &Path) -> Result<Vec<Row>, String> {
    std::fs::create_dir_all(work).map_err(|e| format!("create {}: {e}", work.display()))?;
    let redact = Redact::new(dest, work);
    let mut rows = Vec::new();
    for s in sources {
        let folder = dest.join(s.dir());
        for e in s.scripts() {
            let script = folder.join(&e.path);
            let text = std::fs::read(&script).unwrap_or_default();
            let row_dir = work.join(format!("row-{}", rows.len()));
            let (outcome, detail) = match run_child(exe, &script, &folder, &row_dir) {
                Ok(Ran::Reported(v)) => (classify(&v), redact.value(v)),
                Ok(Ran::TimedOut) => (Outcome::Timeout, Value::Null),
                Ok(Ran::NoResult(why)) => (Outcome::Internal, json!({"message": redact.text(&why)})),
                Err(why) => (Outcome::Infrastructure, json!({"message": redact.text(&why)})),
            };
            let _ = std::fs::remove_dir_all(&row_dir);
            eprintln!("{} {}: {}", s.repo, e.path, outcome.id());
            rows.push(Row { repo: s.repo.clone(), path: e.path.clone(), low_interaction: low_interaction(&text), outcome, detail });
        }
    }
    Ok(rows)
}

enum Ran {
    Reported(Value),
    TimedOut,
    NoResult(String),
}

/// Runs one script in the runner's `row` test, stopping the process after [`TIMEOUT`].
fn run_child(exe: &Path, script: &Path, folder: &Path, row_dir: &Path) -> Result<Ran, String> {
    let tmp = row_dir.join("tmp");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
    let child = Command::new(exe)
        .args(["row", "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env("VECTORCRAFT_SCRIPT_CORPUS_ROW", script)
        .env("VECTORCRAFT_SCRIPT_CORPUS_SOURCE", folder)
        .env("VECTORCRAFT_SCRIPT_CORPUS_WORK", row_dir)
        .env("TMPDIR", &tmp)
        .env("TMP", &tmp)
        .env("TEMP", &tmp)
        .current_dir(row_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("start {}: {e}", exe.display()))?;
    let start = Instant::now();
    let Some((status, out)) = wait_bounded(child, &mut || start.elapsed() >= TIMEOUT)? else { return Ok(Ran::TimedOut) };
    Ok(match row_result(&String::from_utf8_lossy(&out)) {
        Some(v) => Ran::Reported(v),
        None => Ran::NoResult(format!("the runner exited with {status} without a result")),
    })
}

/// The JSON after [`ROW_PREFIX`] in the runner's output. libtest writes `test row ... ` before
/// the test's own output on the same line, so the prefix can start anywhere in a line.
fn row_result(text: &str) -> Option<Value> {
    text.lines().find_map(|l| l.find(ROW_PREFIX).map(|i| &l[i + ROW_PREFIX.len()..])).and_then(|j| serde_json::from_str(j).ok())
}

/// Waits for `child` and reads its output (at most [`MAX_ROW_OUTPUT`] bytes) while it runs; once
/// `expired` gives true the child is stopped and the result is `None`.
fn wait_bounded(mut child: std::process::Child, expired: &mut dyn FnMut() -> bool) -> Result<Option<(std::process::ExitStatus, Vec<u8>)>, String> {
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(pipe) = stdout {
            let _ = pipe.take(MAX_ROW_OUTPUT).read_to_end(&mut out);
        }
        out
    });
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("wait for the script process: {e}"))? {
            break Some(status);
        }
        if expired() {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let out = reader.join().unwrap_or_default();
    Ok(status.map(|s| (s, out)))
}

/// Takes the corpus folder, the run's folder and the home folder out of messages.
struct Redact(Vec<(String, &'static str)>);

impl Redact {
    fn new(dest: &Path, work: &Path) -> Self {
        let mut pairs = Vec::new();
        for (path, shown) in [(Some(dest.to_path_buf()), "<corpus>"), (Some(work.to_path_buf()), "<work>"), (home(), "~")] {
            let Some(path) = path else { continue };
            let text = path.display().to_string();
            if text.len() > 1 {
                pairs.push((text.replace('\\', "/"), shown));
                pairs.push((text, shown));
            }
        }
        Self(pairs)
    }

    fn text(&self, s: &str) -> String {
        self.0.iter().fold(s.to_owned(), |s, (from, to)| s.replace(from.as_str(), to))
    }

    fn value(&self, mut v: Value) -> Value {
        if let Some(m) = v.get("message").and_then(Value::as_str).map(|m| self.text(m)) {
            v["message"] = Value::String(m);
        }
        v
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// `passed` of `total` as the report shows it; an empty set has no rate.
pub fn rate(passed: usize, total: usize) -> String {
    if total == 0 {
        return "0/0; rate unavailable".into();
    }
    format!("{passed}/{total} ({:.1}%)", passed as f64 * 100.0 / total as f64)
}

/// The report of `rows`, sorted by repository and path.
pub fn build_report(meta: Value, sources: &[Source], rows: &[Row]) -> Value {
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by(|a, b| (a.repo.as_str(), a.path.as_str()).cmp(&(b.repo.as_str(), b.path.as_str())));
    let count = |f: &dyn Fn(&Row) -> bool| sorted.iter().filter(|r| f(r)).count();
    let passed = |r: &Row| r.outcome == Outcome::Passed;
    let per_source: Vec<Value> = sources
        .iter()
        .map(|s| {
            let mine = |r: &Row| r.repo == s.repo;
            json!({
                "repository": s.repo,
                "commit": s.commit,
                "license": s.license,
                "scripts": count(&mine),
                "passed": count(&|r| mine(r) && passed(r)),
                "parsed": count(&|r| mine(r) && parsed(r)),
            })
        })
        .collect();
    let outcomes: serde_json::Map<String, Value> =
        Outcome::ALL.iter().map(|o| (o.id().to_owned(), json!(count(&|r: &Row| r.outcome == *o)))).collect();
    let excluded: Vec<Value> = sources
        .iter()
        .flat_map(|s| s.entries.iter().filter(|e| e.kind == Kind::Excluded).map(move |e| (s, e)))
        .map(|(s, e)| {
            let reason = if e.bytes > MAX_SCRIPT_BYTES { "over the 1 MiB script limit" } else { "excluded" };
            json!({"repository": s.repo, "path": e.path, "bytes": e.bytes, "reason": reason})
        })
        .collect();
    let includes = sources.iter().flat_map(|s| &s.entries).filter(|e| e.kind == Kind::Include).count();
    let low = |r: &Row| r.low_interaction;
    let scripts: Vec<Value> = sorted
        .iter()
        .map(|r| {
            let d = &r.detail;
            json!({
                "repository": r.repo,
                "path": r.path,
                "outcome": r.outcome.id(),
                "lowInteraction": r.low_interaction,
                "kind": d["kind"],
                "message": d["message"],
                "line": d["line"],
                "column": d["column"],
                "fuelUsed": d["fuelUsed"],
                "hostCalls": d["hostCalls"],
                "fileExecutes": d["fileExecutes"],
            })
        })
        .collect();
    json!({
        "status": "ran",
        "meta": meta,
        "candidates": sorted.len() + includes + excluded.len(),
        "includes": includes,
        "excluded": excluded,
        "scriptsRun": sorted.len(),
        "passed": count(&passed),
        "rate": rate(count(&passed), sorted.len()),
        "parsed": count(&parsed),
        "lowInteraction": {"scripts": count(&low), "passed": count(&|r| low(r) && passed(r)), "rate": rate(count(&|r| low(r) && passed(r)), count(&low))},
        "outcomes": outcomes,
        "unimplementedMembers": "unavailable",
        "sources": per_source,
        "scripts": scripts,
    })
}

fn print_summary(r: &Value) {
    println!(
        "script corpus: {} scripts run from {} repositories ({} candidates, {} included files, {} excluded)",
        r["scriptsRun"],
        r["sources"].as_array().map_or(0, Vec::len),
        r["candidates"],
        r["includes"],
        r["excluded"].as_array().map_or(0, Vec::len)
    );
    println!("passed: {}; parsed: {}", r["rate"].as_str().unwrap_or(""), r["parsed"]);
    println!("low-interaction subset: {}", r["lowInteraction"]["rate"].as_str().unwrap_or(""));
    let outcomes: Vec<String> = Outcome::ALL.iter().map(|o| format!("{} {}", o.id(), r["outcomes"][o.id()])).collect();
    println!("outcomes: {}", outcomes.join(", "));
    println!("unimplemented-member ranking unavailable");
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn sha(b: &[u8]) -> String {
        sha256::hex(b)
    }

    fn manifest(lines: &[&str]) -> String {
        let mut text = String::from("# test\nsource owner/repo 0123456789abcdef0123456789abcdef01234567 MIT\n");
        for l in lines {
            text.push_str(l);
            text.push('\n');
        }
        text
    }

    fn entry(kind: &str, body: &[u8], path: &str) -> String {
        format!("{kind} {} {} {path}", body.len(), sha(body))
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-script-corpus-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn manifest_reads_sources_and_entries() {
        let text = manifest(&[&entry("license", b"L", "LICENSE"), &entry("script", b"s", "a b/c (1).jsx"), &format!("excluded 2000000 {A} big.jsx")]);
        let sources = parse(&text).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].dir(), "owner_repo");
        assert_eq!(sources[0].entries[1].path, "a b/c (1).jsx");
        assert_eq!(sources[0].entries[2].kind, Kind::Excluded);
    }

    #[test]
    fn manifest_refuses_bad_paths() {
        let lic = entry("license", b"L", "LICENSE");
        for path in ["../x.js", "a/../x.js", "/x.js", "a//x.js", "a\\x.js", "c:x.js", "CON.js", "lpt1.jsx", "a /x.js", "a./x.js", "./x.js"] {
            let e = parse(&manifest(&[&lic, &entry("script", b"s", path)])).unwrap_err();
            assert!(e.contains("line 4"), "{path}: {e}");
        }
    }

    #[test]
    fn manifest_refuses_duplicates_and_case_collisions() {
        let lic = entry("license", b"L", "LICENSE");
        let e = parse(&manifest(&[&lic, &entry("script", b"s", "a.js"), &entry("script", b"t", "a.js")])).unwrap_err();
        assert!(e.contains("listed twice"), "{e}");
        let e = parse(&manifest(&[&lic, &entry("script", b"s", "A.js"), &entry("script", b"t", "a.JS")])).unwrap_err();
        assert!(e.contains("letter case"), "{e}");
        let e = parse(&manifest(&[&lic, &entry("script", b"s", "a"), &entry("script", b"t", "a/b.js")]));
        assert!(e.is_err());
        let e = parse(&manifest(&[&lic, &entry("include", b"s", "a"), &entry("script", b"t", "a/b.js")])).unwrap_err();
        assert!(e.contains("needs a folder"), "{e}");
    }

    #[test]
    fn manifest_refuses_bad_sha_sizes_and_sources() {
        let lic = entry("license", b"L", "LICENSE");
        let e = parse(&manifest(&[&lic, "script 1 ABCDEF x.js"])).unwrap_err();
        assert!(e.contains("SHA-256"), "{e}");
        let e = parse(&manifest(&[&lic, &format!("script 1 {} x.js", "A".repeat(64))])).unwrap_err();
        assert!(e.contains("SHA-256"), "{e}");
        let e = parse(&manifest(&[&lic, &format!("script +1 {A} x.js")])).unwrap_err();
        assert!(e.contains("byte count"), "{e}");
        let e = parse(&manifest(&[&lic, &format!("script {} {A} x.js", MAX_SCRIPT_BYTES + 1)])).unwrap_err();
        assert!(e.contains("limit"), "{e}");
        let e = parse(&manifest(&[&lic, &format!("script 1 {A} x.txt")])).unwrap_err();
        assert!(e.contains(".js or .jsx"), "{e}");
        let e = parse(&manifest(&[&format!("script 1 {A} x.js")])).unwrap_err();
        assert!(e.contains("needs exactly one license"), "{e}");
        let e = parse(&manifest(&[&lic])).unwrap_err();
        assert!(e.contains("lists no script"), "{e}");
        let e = parse(&format!("script 1 {A} x.js\n")).unwrap_err();
        assert!(e.contains("before any source"), "{e}");
        let e = parse("source owner/repo 0123 MIT\n").unwrap_err();
        assert!(e.contains("commit"), "{e}");
        let e = parse("source owner/repo 0123456789abcdef0123456789abcdef01234567 GPL-3.0\n").unwrap_err();
        assert!(e.contains("license"), "{e}");
        let e = parse("source ../repo 0123456789abcdef0123456789abcdef01234567 MIT\n").unwrap_err();
        assert!(e.contains("owner/repository"), "{e}");
        let e = parse(&manifest(&[&lic, &format!("binary 1 {A} x.js")])).unwrap_err();
        assert!(e.contains("unknown kind"), "{e}");
    }

    #[test]
    fn manifest_refuses_totals_over_the_limits() {
        let mut lines = vec![entry("license", b"L", "LICENSE")];
        for i in 0..65 {
            lines.push(format!("script {MAX_SCRIPT_BYTES} {A} s{i}.js"));
        }
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let e = parse(&manifest(&refs)).unwrap_err();
        assert!(e.contains("add up to"), "{e}");
    }

    #[test]
    fn an_empty_manifest_lists_nothing() {
        assert!(parse("# nothing yet\n").unwrap().is_empty());
    }

    /// A downloader that serves `files` (URL suffix, body) and counts its calls.
    fn server<'a>(files: &'a [(&'a str, &'a [u8])], calls: &'a std::cell::Cell<usize>) -> impl FnMut(&str, &Path, u64) -> Result<(), String> + 'a {
        move |url, out, max| {
            calls.set(calls.get() + 1);
            let body = files.iter().find(|(name, _)| url.ends_with(name)).map(|(_, b)| *b).ok_or("404")?;
            if body.len() as u64 > max.max(1) {
                return Err("curl: maximum file size exceeded".into());
            }
            std::fs::write(out, body).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn fetch_verifies_places_and_keeps_matching_copies() {
        let dir = temp("fetch");
        let text = manifest(&[&entry("license", b"MIT text", "LICENSE"), &entry("script", b"var a = 1;", "dir one/x (1).jsx")]);
        let sources = parse(&text).unwrap();
        let calls = std::cell::Cell::new(0);
        let files: &[(&str, &[u8])] = &[("/LICENSE", b"MIT text"), ("/dir%20one/x%20%281%29.jsx", b"var a = 1;")];
        let done = fetch_into(&sources, &dir, &dir.join(".staging"), MAX_TRANSFER, &mut server(files, &calls)).unwrap();
        assert_eq!(done, Fetched { kept: 0, fetched: 2, bytes: 18 });
        assert_eq!(std::fs::read(dir.join("owner_repo/dir one/x (1).jsx")).unwrap(), b"var a = 1;");
        assert_eq!(std::fs::read(dir.join("owner_repo/LICENSE")).unwrap(), b"MIT text");
        let done = fetch_into(&sources, &dir, &dir.join(".staging"), MAX_TRANSFER, &mut server(files, &calls)).unwrap();
        assert_eq!(done, Fetched { kept: 2, fetched: 0, bytes: 0 });
        assert_eq!(calls.get(), 2);
        assert!(check_corpus(&sources, &dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_upstream_file_is_refused_and_the_verified_copy_stays() {
        let dir = temp("changed");
        let text = manifest(&[&entry("license", b"MIT text", "LICENSE"), &entry("script", b"var a = 1;", "x.js")]);
        let sources = parse(&text).unwrap();
        let calls = std::cell::Cell::new(0);
        let good: &[(&str, &[u8])] = &[("/LICENSE", b"MIT text"), ("/x.js", b"var a = 1;")];
        fetch_into(&sources, &dir, &dir.join(".staging"), MAX_TRANSFER, &mut server(good, &calls)).unwrap();
        // The cached copy was damaged, and upstream now serves other bytes of the same size.
        std::fs::write(dir.join("owner_repo/x.js"), b"var a = 2;").unwrap();
        let changed: &[(&str, &[u8])] = &[("/LICENSE", b"MIT text"), ("/x.js", b"var b = 1;")];
        let e = fetch_into(&sources, &dir, &dir.join(".staging"), MAX_TRANSFER, &mut server(changed, &calls)).unwrap_err();
        assert!(e.contains("does not match"), "{e}");
        assert_eq!(std::fs::read(dir.join("owner_repo/x.js")).unwrap(), b"var a = 2;");
        assert_eq!(std::fs::read_dir(dir.join(".staging")).unwrap().count(), 0);
        let e = check_corpus(&sources, &dir).unwrap_err();
        assert!(e.contains("missing or does not match"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_oversize_or_short_download_is_refused() {
        let dir = temp("oversize");
        let text = manifest(&[&entry("license", b"MIT text", "LICENSE"), &entry("script", b"var a = 1;", "x.js")]);
        let sources = parse(&text).unwrap();
        let calls = std::cell::Cell::new(0);
        let big: &[(&str, &[u8])] = &[("/LICENSE", b"MIT text"), ("/x.js", b"var a = 1; var b = 2;")];
        let e = fetch_into(&sources, &dir, &dir.join(".staging"), MAX_TRANSFER, &mut server(big, &calls)).unwrap_err();
        assert!(e.contains("maximum file size"), "{e}");
        assert!(!dir.join("owner_repo/x.js").exists());
        let short: &[(&str, &[u8])] = &[("/LICENSE", b"MIT text"), ("/x.js", b"var a")];
        let e = fetch_into(&sources, &dir, &dir.join(".staging"), MAX_TRANSFER, &mut server(short, &calls)).unwrap_err();
        assert!(e.contains("the download is 5 bytes"), "{e}");
        assert!(!dir.join("owner_repo/x.js").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fetch_stops_at_the_transfer_limit() {
        let dir = temp("transfer");
        let text =
            manifest(&[&entry("license", b"MIT text", "LICENSE"), &entry("script", b"var a = 1;", "x.js"), &entry("script", b"var b;", "y.js")]);
        let sources = parse(&text).unwrap();
        let calls = std::cell::Cell::new(0);
        let files: &[(&str, &[u8])] = &[("/LICENSE", b"MIT text"), ("/x.js", b"var a = 1;"), ("/y.js", b"var b;")];
        // 8 + 10 bytes fit in 20; the next 6 do not, and nothing is asked for them.
        let e = fetch_into(&sources, &dir, &dir.join(".staging"), 20, &mut server(files, &calls)).unwrap_err();
        assert!(e.contains("y.js: the fetch would download more than 20 bytes"), "{e}");
        assert_eq!(calls.get(), 2);
        assert!(!dir.join("owner_repo/y.js").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_fetch_is_refused_while_the_lock_is_held() {
        let dir = temp("lock");
        let lock = Lock::take(&dir.join(".fetch.lock")).unwrap();
        let e = Lock::take(&dir.join(".fetch.lock")).err().unwrap();
        assert!(e.contains("another `cargo xtask corpus --scripts` is running"), "{e}");
        drop(lock);
        assert!(Lock::take(&dir.join(".fetch.lock")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn urls_escape_every_byte_outside_the_safe_set() {
        let sources = parse(&manifest(&[&entry("license", b"L", "LICENSE"), &entry("script", b"s", "etc/new(&L) é.jsx")])).unwrap();
        assert_eq!(
            raw_url(&sources[0], &sources[0].entries[1]),
            "https://raw.githubusercontent.com/owner/repo/0123456789abcdef0123456789abcdef01234567/etc/new%28%26L%29%20%C3%A9.jsx"
        );
    }

    #[test]
    fn arguments_are_checked() {
        assert_eq!(parse_args(&["--pass-rate"]).unwrap(), Args { report: None });
        assert_eq!(parse_args(&["--report", "r.json", "--pass-rate"]).unwrap(), Args { report: Some("r.json".into()) });
        for bad in
            [&[][..], &["--pass-rate", "--pass-rate"], &["--pass-rate", "--report"], &["--pass-rate", "--report", "--x"], &["--pass-rate", "--fast"]]
        {
            assert!(parse_args(bad).unwrap_err().contains(SCRIPTS_USAGE), "{bad:?}");
        }
    }

    #[test]
    fn a_report_path_that_exists_or_has_no_folder_is_refused() {
        let dir = temp("report");
        let path = dir.join("r.json");
        let (_, file) = create_report(&path).unwrap();
        drop(file);
        assert!(create_report(&path).unwrap_err().contains("already exists"));
        assert!(create_report(&dir.join("missing").join("r.json")).unwrap_err().contains("does not exist"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outcomes_are_sorted_into_their_categories() {
        let cases = [
            (json!({"started": true, "ok": true}), Outcome::Passed),
            (json!({"started": true, "ok": false, "kind": "syntax", "message": "SyntaxError: x"}), Outcome::Syntax),
            (json!({"started": true, "ok": false, "kind": "thrown", "message": "ReferenceError: app is not defined"}), Outcome::Thrown),
            (json!({"started": true, "ok": false, "kind": "limit", "message": "fuel"}), Outcome::Limit),
            (json!({"started": true, "ok": false, "kind": "stopped", "message": "stopped"}), Outcome::Limit),
            (json!({"started": true, "ok": false, "kind": "internal", "message": "x"}), Outcome::Internal),
            (json!({"started": true, "ok": false, "kind": "thrown", "message": "alert", "missingAnswer": true}), Outcome::Headless),
            (json!({"started": true, "ok": false, "kind": "thrown", "message": format!("Error: {NO_PICKER}")}), Outcome::Headless),
            (json!({"started": true, "ok": false, "kind": "thrown", "message": "Error: Script file access denied"}), Outcome::Denied),
            (json!({"started": true, "ok": false, "kind": "thrown", "message": "automation path rejected: outside the write root"}), Outcome::Denied),
            (json!({"started": false, "ok": false, "message": "the script is not UTF-8 text"}), Outcome::Syntax),
        ];
        for (v, want) in cases {
            assert_eq!(classify(&v), want, "{v}");
        }
    }

    fn row(repo: &str, path: &str, outcome: Outcome, low: bool, detail: Value) -> Row {
        Row { repo: repo.into(), path: path.into(), low_interaction: low, outcome, detail }
    }

    #[test]
    fn the_report_counts_every_row_and_sorts_them() {
        let text = format!(
            "source b/two 0123456789abcdef0123456789abcdef01234567 MIT\n{}\n{}\n{}\nsource a/one 0123456789abcdef0123456789abcdef01234567 Apache-2.0\n{}\n{}\n{}\nexcluded {} {A} big.jsx\n",
            entry("license", b"L", "LICENSE"),
            entry("script", b"1", "z.js"),
            entry("script", b"2", "a.js"),
            entry("license", b"L", "LICENSE"),
            entry("script", b"3", "m.jsx"),
            entry("include", b"4", "m.jsxinc"),
            MAX_SCRIPT_BYTES + 1
        );
        let sources = parse(&text).unwrap();
        let rows = [
            row("b/two", "z.js", Outcome::Passed, true, json!({"started": true, "ok": true, "fuelUsed": 10})),
            row("b/two", "a.js", Outcome::Syntax, true, json!({"started": true, "kind": "syntax"})),
            row("a/one", "m.jsx", Outcome::Timeout, false, Value::Null),
        ];
        let r = build_report(json!({}), &sources, &rows);
        assert_eq!(r["scriptsRun"], 3);
        assert_eq!(r["candidates"], 5);
        assert_eq!(r["includes"], 1);
        assert_eq!(r["excluded"][0]["reason"], "over the 1 MiB script limit");
        assert_eq!(r["passed"], 1);
        assert_eq!(r["rate"], "1/3 (33.3%)");
        assert_eq!(r["parsed"], 1);
        assert_eq!(r["lowInteraction"]["rate"], "1/2 (50.0%)");
        assert_eq!(r["outcomes"]["timeout"], 1);
        assert_eq!(r["outcomes"]["unimplemented-member"], 0);
        assert_eq!(r["unimplementedMembers"], "unavailable");
        let order: Vec<(&str, &str)> =
            r["scripts"].as_array().unwrap().iter().map(|s| (s["repository"].as_str().unwrap(), s["path"].as_str().unwrap())).collect();
        assert_eq!(order, [("a/one", "m.jsx"), ("b/two", "a.js"), ("b/two", "z.js")]);
        assert_eq!(r["sources"][0]["repository"], "b/two");
        assert_eq!(r["sources"][0]["passed"], 1);
        assert_eq!(r["sources"][1]["scripts"], 1);
    }

    #[test]
    fn no_rows_has_no_rate() {
        assert_eq!(rate(0, 0), "0/0; rate unavailable");
        let r = build_report(json!({}), &[], &[]);
        assert_eq!(r["rate"], "0/0; rate unavailable");
        assert_eq!(r["lowInteraction"]["rate"], "0/0; rate unavailable");
    }

    #[test]
    fn messages_lose_local_folders() {
        let redact = Redact::new(Path::new("/home/me/vc/corpus/scripts"), Path::new("/home/me/vc/target/run-1"));
        let v = redact.value(json!({"message": "Error: /home/me/vc/corpus/scripts/a_b/x.jsx and /home/me/vc/target/run-1/tmp/y"}));
        assert_eq!(v["message"], "Error: <corpus>/a_b/x.jsx and <work>/tmp/y");
    }

    #[test]
    fn scripts_with_dialogs_or_windows_are_not_low_interaction() {
        assert!(low_interaction(b"var d = app.activeDocument;"));
        assert!(!low_interaction(b"alert('x')"));
        assert!(!low_interaction(b"var w = new Window('dialog');"));
        assert!(!low_interaction(b"var f = File.openDialog();"));
        assert!(!low_interaction(b"alert ('Continue?')"));
        assert!(!low_interaction(b"var w = new Window\n\t('dialog');"));
        assert!(!low_interaction(b"if (confirm('x')) {}"));
        assert!(low_interaction(b"var alerts = 1; myalert(2); var Window2 = prompt;"));
        assert!(low_interaction(b""));
    }

    #[test]
    fn the_runner_is_found_in_cargo_messages() {
        let messages = concat!(
            "{\"reason\":\"compiler-artifact\",\"target\":{\"name\":\"other\"},\"executable\":\"/t/other\"}\n",
            "not json\n",
            "{\"reason\":\"compiler-artifact\",\"target\":{\"name\":\"script_corpus\"},\"executable\":\"/t/script_corpus-1\"}\n",
            "{\"reason\":\"build-finished\",\"success\":true}\n"
        );
        assert_eq!(runner_path(messages), Some(PathBuf::from("/t/script_corpus-1")));
        assert_eq!(runner_path("{\"reason\":\"build-finished\"}"), None);
    }

    #[test]
    fn the_checked_in_manifest_reads() {
        let (sources, _) = load(&root()).unwrap();
        assert!(!sources.is_empty());
        assert!(sources.iter().all(|s| s.entries.iter().any(|e| e.kind == Kind::License)));
    }

    #[test]
    fn the_row_result_is_read_after_libtests_own_text() {
        let out = "\nrunning 1 test\ntest row ... script-corpus-row {\"ok\":true,\"kind\":null}\nok\n\ntest result: ok. 1 passed\n";
        assert_eq!(row_result(out), Some(json!({"ok": true, "kind": null})));
        assert_eq!(row_result("script-corpus-row {\"ok\":false}\n"), Some(json!({"ok": false})));
        assert_eq!(row_result("test row ... FAILED\n"), None);
        assert_eq!(row_result("script-corpus-row {not json\n"), None);
    }

    #[test]
    fn a_process_still_running_when_its_time_is_up_is_stopped() {
        // `sleep` stands in for a script process that hangs; the time is up at the first check.
        if !cfg!(unix) {
            return;
        }
        let child = Command::new("sleep").arg("600").stdout(Stdio::piped()).spawn().unwrap();
        let pid = child.id();
        assert!(wait_bounded(child, &mut || true).unwrap().is_none());
        let alive = Command::new("kill").args(["-0", &pid.to_string()]).status().unwrap();
        assert!(!alive.success(), "the process {pid} still runs");
    }

    #[test]
    fn a_finished_process_gives_its_status_and_output() {
        if !cfg!(unix) {
            return;
        }
        let child = Command::new("sh").args(["-c", "echo script-corpus-row '{\"ok\":true}'"]).stdout(Stdio::piped()).spawn().unwrap();
        let (status, out) = wait_bounded(child, &mut || false).unwrap().unwrap();
        assert!(status.success());
        assert_eq!(out, b"script-corpus-row {\"ok\":true}\n");
    }
}
