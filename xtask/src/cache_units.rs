//! The build units a CI build used, for the dependency cache entry the gate saves.
//!
//! In a gate leg that saves the dependency cache, craft-common-actions' `rust-cache` action names a
//! file in `CRAFT_CACHE_UNITS`, and its `cache-trim` action deletes every build unit of the target
//! directory that the file does not name before the entry is saved. Cargo prints a JSON message for
//! every unit a build compiled or found fresh. The builds of `cargo xtask ci warm` append those
//! messages to the file, so the saved entry holds this build and no older builds of updated
//! dependencies. Without `CRAFT_CACHE_UNITS` the commands run as they always have.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Cargo prints its messages as JSON on stdout and the diagnostics as text on stderr.
const MESSAGE_FORMAT: [&str; 2] = ["--message-format", "json-render-diagnostics"];

/// The record file, when this leg saves the cache.
fn file() -> Option<PathBuf> {
    std::env::var_os("CRAFT_CACHE_UNITS").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Runs `cmd`, a cargo build shown as `what`, which must exit successfully. When this leg saves the
/// cache, it adds `--message-format json-render-diagnostics` and appends Cargo's messages to the
/// record; every other stdout line (test output) still goes to stdout.
pub fn run(cmd: Command, what: &str) -> Result<(), String> {
    match file() {
        Some(file) => run_recorded(cmd, what, &file),
        None => crate::run(cmd, what),
    }
}

/// [`run`] with the record in `file`.
fn run_recorded(mut cmd: Command, what: &str, file: &Path) -> Result<(), String> {
    // After `--` the flag would go to rustc or the test binaries instead of cargo.
    if cmd.get_args().any(|a| a == "--") {
        return Err(format!("{what}: a recorded build takes no `--` arguments"));
    }
    cmd.args(MESSAGE_FORMAT);
    eprintln!("$ {what} {}", MESSAGE_FORMAT.join(" "));
    let mut record = OpenOptions::new().create(true).append(true).open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut child = cmd.stdout(Stdio::piped()).spawn().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    let routed = match child.stdout.take() {
        Some(stdout) => route(BufReader::new(stdout), &mut record, &mut std::io::stdout().lock()),
        None => Err(std::io::Error::other("no stdout pipe")),
    };
    let status = child.wait().map_err(|e| format!("{what}: {e}"))?;
    routed.map_err(|e| format!("{what}: recording the build units in {}: {e}", file.display()))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

/// Records xtask's own units. `cargo xtask` builds xtask before xtask can record anything, and the
/// trim deletes an unrecorded unit from the entry, so the next run would build xtask again. The
/// build is fresh by now, so this only prints the messages.
pub fn record_xtask() -> Result<(), String> {
    if file().is_none() {
        return Ok(());
    }
    let mut c = crate::cargo();
    c.args(XTASK_BUILD);
    run(c, &format!("cargo {}", XTASK_BUILD.join(" ")))
}

/// The build `cargo xtask` (the `run --package xtask` alias in `.cargo/config.toml`) runs first.
const XTASK_BUILD: [&str; 3] = ["build", "-p", "xtask"];

/// Lines that are Cargo's JSON messages go to `record`, every other line to `out`.
fn route(mut input: impl BufRead, record: &mut impl Write, out: &mut impl Write) -> std::io::Result<()> {
    let mut line = Vec::new();
    while input.read_until(b'\n', &mut line)? != 0 {
        if line.starts_with(b"{\"reason\":") {
            record.write_all(&line)?;
            if !line.ends_with(b"\n") {
                record.write_all(b"\n")?;
            }
        } else {
            out.write_all(&line)?;
        }
        line.clear();
    }
    record.flush()?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_messages_are_recorded_and_test_output_passes_through() {
        let input =
            "{\"reason\":\"compiler-artifact\",\"fresh\":true}\nrunning 3 tests\ntest a ... ok\n{\"reason\":\"build-finished\",\"success\":true}";
        let (mut record, mut out) = (Vec::new(), Vec::new());
        route(input.as_bytes(), &mut record, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(record).unwrap(),
            "{\"reason\":\"compiler-artifact\",\"fresh\":true}\n{\"reason\":\"build-finished\",\"success\":true}\n"
        );
        assert_eq!(String::from_utf8(out).unwrap(), "running 3 tests\ntest a ... ok\n");
    }

    #[test]
    fn a_recorded_build_takes_no_separator() {
        // A program that does not exist: the check comes before anything is spawned.
        let mut c = Command::new("vectorcraft-no-such-cargo");
        c.args(["clippy", "--", "-D", "warnings"]);
        let file = std::env::temp_dir().join(format!("xtask-cache-units-separator-{}.jsonl", std::process::id()));
        let err = run_recorded(c, "cargo clippy -- -D warnings", &file).unwrap_err();
        assert!(err.contains("no `--` arguments"), "{err}");
        assert!(!file.exists());
    }

    #[test]
    fn xtask_record_build_matches_the_alias() {
        let config = std::fs::read_to_string(crate::root().join(".cargo/config.toml")).unwrap();
        assert!(config.lines().any(|l| l.trim() == "xtask = \"run --package xtask --\""), "{config}");
    }

    #[test]
    fn a_real_build_appends_its_units_fresh_on_the_second_run() {
        let dir = std::env::temp_dir().join(format!("xtask-cache-units-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"cache-unit-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n")
            .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn one() -> u32 { 1 }\n").unwrap();
        let record = dir.join("units.jsonl");
        for expected_fresh in [false, true] {
            let mut c = crate::cargo();
            c.current_dir(&dir)
                .env("CARGO_TARGET_DIR", dir.join("target"))
                .env("CARGO_BUILD_BUILD_DIR", dir.join("target"))
                .env("CARGO_NET_OFFLINE", "true")
                .args(["build", "--lib"]);
            run_recorded(c, "cargo build --lib", &record).unwrap();
            let messages: Vec<serde_json::Value> =
                std::fs::read_to_string(&record).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
            let artifacts: Vec<&serde_json::Value> =
                messages.iter().filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "cache_unit_fixture").collect();
            // The record is appended to: the first build's message is still there after the second.
            assert_eq!(artifacts.len(), if expected_fresh { 2 } else { 1 });
            assert_eq!(artifacts.last().unwrap()["fresh"].as_bool(), Some(expected_fresh));
            assert!(artifacts.iter().all(|m| !m["filenames"].as_array().unwrap().is_empty()));
            assert_eq!(messages.last().unwrap()["reason"], "build-finished");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
