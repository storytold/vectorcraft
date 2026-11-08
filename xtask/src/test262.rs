//! `cargo xtask test262`: the official ECMAScript conformance suite (https://github.com/tc39/test262,
//! BSD-3-Clause) at a pinned commit, fetched as GitHub's archive of that commit, checked against
//! its SHA-256 and size, and unpacked into `corpus/test262/` (gitignored, never committed). It then
//! runs `vectorcraft-js`'s `test262` tests, which parse the ES5 parse subset, run the ES5
//! `test/language` tests and the tests of the ES5 built-ins under `test/built-ins` and
//! `test/annexB/built-ins` that need only the interpreter's built-ins, and print each one's counts.
//!
//! It also fetches `json2.js`, Douglas Crockford's public-domain JSON polyfill
//! (https://github.com/douglascrockford/JSON-js), at a pinned commit, checks it against its SHA-256
//! and size, saves it in `corpus/json2/` (gitignored, never committed), and runs `vectorcraft-js`'s
//! `json2` test, which runs the polyfill and compares what it writes and reads with the built-in
//! `JSON`.
//!
//! Moving the pin: change [`COMMIT`] (or [`JSON2_COMMIT`]), run the command once with the new
//! commit to see the file's SHA-256 in the error, check the file, and update [`SHA256`] (or
//! [`JSON2_SHA256`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{cargo, root, run, sha256};

/// The douglascrockford/JSON-js commit `json2.js` is read at.
const JSON2_COMMIT: &str = "7e83f38a2312429fd4933169c1f6a27fd65e889c";

/// The SHA-256 of `json2.js` at [`JSON2_COMMIT`].
const JSON2_SHA256: &str = "86df14b56572e68a7e10b18e71804cb78c6890a7d6e534324822c23d12e858a2";

/// The largest `json2.js` accepted (the pinned one is 18,876 bytes).
const JSON2_MAX_BYTES: u64 = 1 << 20;

/// The tc39/test262 commit the suite is read at.
const COMMIT: &str = "d53ebc8b1931d9f1541e43d59650776e4b766248";

/// The SHA-256 of GitHub's `tar.gz` archive of [`COMMIT`].
const SHA256: &str = "6db8566044d1a3f5a68aac87806e09b2fe5d21f5869f3a74cfd8b7236ebaf5c8";

/// The largest archive accepted (the pinned one is 9.6 MB).
const MAX_BYTES: u64 = 64 << 20;

pub fn run_suite() -> Result<(), String> {
    let dir = fetch()?;
    let mut c = cargo();
    c.env("VECTORCRAFT_TEST262_DIR", &dir).args(["test", "-p", "vectorcraft-js", "--release", "--test", "test262", "--", "--nocapture"]);
    run(c, &format!("cargo test -p vectorcraft-js --release --test test262 -- --nocapture (VECTORCRAFT_TEST262_DIR={})", dir.display()))?;
    let json2 = fetch_json2()?;
    let mut c = cargo();
    c.env("VECTORCRAFT_JSON2", &json2).args(["test", "-p", "vectorcraft-js", "--release", "--test", "json2", "--", "--nocapture"]);
    run(c, &format!("cargo test -p vectorcraft-js --release --test json2 -- --nocapture (VECTORCRAFT_JSON2={})", json2.display()))
}

/// Fetches `json2.js` unless a copy with the pinned SHA-256 is there already, and returns its path.
fn fetch_json2() -> Result<PathBuf, String> {
    let base = root().join("corpus").join("json2").join(JSON2_COMMIT);
    let file = base.join("json2.js");
    let matches = |p: &Path| -> Result<bool, String> {
        let Ok(meta) = std::fs::metadata(p) else { return Ok(false) };
        if meta.len() > JSON2_MAX_BYTES {
            return Err(format!("{} is {} bytes, over the {JSON2_MAX_BYTES}-byte limit", p.display(), meta.len()));
        }
        Ok(std::fs::read(p).is_ok_and(|b| sha256::hex(&b) == JSON2_SHA256))
    };
    if matches(&file)? {
        println!("json2.js {JSON2_COMMIT} is in {}", file.display());
        return Ok(file);
    }
    std::fs::create_dir_all(&base).map_err(|e| format!("create {}: {e}", base.display()))?;
    let url = format!("https://raw.githubusercontent.com/douglascrockford/JSON-js/{JSON2_COMMIT}/json2.js");
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "--retry", "3", "-A", "VectorCraft-dev", "--max-filesize", &JSON2_MAX_BYTES.to_string(), "-o"]).arg(&file).arg(&url);
    run(curl, &format!("curl {url}"))?;
    if !matches(&file)? {
        let got = std::fs::read(&file).map(|b| sha256::hex(&b)).unwrap_or_default();
        let _ = std::fs::remove_file(&file);
        return Err(format!("json2.js: sha256 {got} does not match the pinned {JSON2_SHA256}"));
    }
    println!("json2.js {JSON2_COMMIT} saved in {}", file.display());
    Ok(file)
}

/// Fetches and unpacks the suite unless it is there already, and returns its folder.
fn fetch() -> Result<PathBuf, String> {
    let base = root().join("corpus").join("test262");
    let dir = base.join(format!("test262-{COMMIT}"));
    let marker = dir.join(".vectorcraft-sha256");
    if std::fs::read_to_string(&marker).is_ok_and(|s| s.trim() == SHA256) {
        println!("test262 {COMMIT} is in {}", dir.display());
        return Ok(dir);
    }
    std::fs::create_dir_all(&base).map_err(|e| format!("create {}: {e}", base.display()))?;
    let archive = base.join(format!("test262-{COMMIT}.tar.gz"));
    if !verified(&archive)? {
        let url = format!("https://codeload.github.com/tc39/test262/tar.gz/{COMMIT}");
        let mut curl = Command::new("curl");
        curl.args(["-fsSL", "--retry", "3", "-A", "VectorCraft-dev", "--max-filesize", &MAX_BYTES.to_string(), "-o"]).arg(&archive).arg(&url);
        run(curl, &format!("curl {url}"))?;
        if !verified(&archive)? {
            let got = std::fs::read(&archive).map(|b| sha256::hex(&b)).unwrap_or_default();
            let _ = std::fs::remove_file(&archive);
            return Err(format!("test262 archive: sha256 {got} does not match the pinned {SHA256}"));
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
    }
    let mut tar = Command::new("tar");
    tar.arg("-xzf").arg(&archive).arg("-C").arg(&base);
    run(tar, &format!("tar -xzf {}", archive.display()))?;
    if !dir.join("test").join("language").is_dir() {
        return Err(format!("{} has no test/language folder after unpacking", dir.display()));
    }
    std::fs::write(&marker, SHA256).map_err(|e| format!("{}: {e}", marker.display()))?;
    println!("test262 {COMMIT} unpacked into {}", dir.display());
    Ok(dir)
}

/// The archive is there, within the size limit, with the pinned SHA-256.
fn verified(path: &Path) -> Result<bool, String> {
    let Ok(meta) = std::fs::metadata(path) else { return Ok(false) };
    if meta.len() > MAX_BYTES {
        return Err(format!("{} is {} bytes, over the {MAX_BYTES}-byte limit", path.display(), meta.len()));
    }
    Ok(std::fs::read(path).is_ok_and(|b| sha256::hex(&b) == SHA256))
}
