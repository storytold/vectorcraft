//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `curl`, `tar`) are
//! invoked through `std::process::Command`.

mod affinity_corpus;
mod assets;
mod brands;
mod bundle;
mod cursors;
mod ico;
mod layers;
mod pdf_deps;
mod pngsuite;
mod sha256;
mod stats;
mod version;
mod web_fonts;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  assets          check that every icon/image/font/asset is attributed in ASSETS.md
  brands          check that user-visible text (labels, params docs, MCP, docs, packaging) names no other vendor's products
  cursors         warn about Cursor values with no entry in crates/tools/src/cursors.toml (never fails)
  i18n-report     list the strings each language's catalog lacks, shown in English (fails only on a
                  broken scan; ci runs it too)
  pdf-deps        verify immutable shared PDF writer and patched Krilla sources
  layers          enforce the crate dependency layering (plan/architecture.md §3)
  wasm            cargo check --target wasm32-unknown-unknown for the wasm-safe crates and the web app
  test            the workspace tests (cargo nextest when installed, else cargo test), then the doctests
  ci              fmt --check, clippy -D warnings, test, doctests, no-bundled-fonts, pdf-deps, assets, brands, cursors (warnings only), layers, wasm (stops at first failure)
  corpus [--download] [--affinity]
                  show where test corpora live; --download fetches PngSuite into corpus/pngsuite,
                  --affinity the pinned public Affinity documents into corpus/affinity
  bundle          build dist/VectorCraft.app (macOS) with assets/app-icon/vectorcraft.icns
  ico <out.ico> <png>...
                  pack PNGs into a Windows .ico (used by packaging/icons.sh)
  stats [--exact] count tests and lines per crate (--exact: ask the test harness via `-- --list`)
  web-fonts       copy the craft-fonts faces in crates/text/web-fonts.txt into the web site (the Trunk
                  post_build hook; a no-op without CRAFT_FONTS_DIR)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("assets") => assets::run(&root()),
        Some("brands") => brands::run(&root()),
        Some("cursors") => cursors::run(&root()),
        Some("i18n-report") => cmd_i18n_report(),
        Some("layers") => cmd_layers(),
        Some("pdf-deps") => pdf_deps::run(&root()),
        Some("wasm") => cmd_wasm(),
        Some("test") => cmd_test().and_then(|()| cmd_doctests()),
        Some("ci") => cmd_ci(),
        Some("corpus") if rest.contains(&"--affinity") => affinity_corpus::fetch(),
        Some("corpus") => cmd_corpus(rest.contains(&"--download")),
        Some("bundle") => bundle::run(&root()),
        Some("ico") => ico::run(&rest),
        Some("version") => version::run(&root(), &rest),
        Some("stats") => stats::run(&root(), rest.contains(&"--exact")),
        Some("web-fonts") => web_fonts::run(&root()),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    // The repository root, as a clean path (`join("..")` would print as `…/xtask/..`).
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

/// Tracked and untracked (not ignored) files that exist, relative to `root`.
pub fn repo_files(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!("git ls-files failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|l| root.join(l).exists()).map(str::to_owned).collect())
}

/// A `cargo` command (the one running xtask, else `cargo` on the PATH) run in the workspace root.
pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

/// Prints `what` and runs `cmd`, which must exit successfully.
fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

/// `vectorcraft-ui-egui` and `vectorcraft-mcp` without their default features leave
/// vectorcraft-text's bundled fonts out: no crate they depend on turns them on (docs/development.md
/// › Leaving out the bundled fonts). The same tree with the default features must show them, so a
/// change in cargo tree's output can't make the check pass by finding nothing.
fn libraries_without_bundled_fonts() -> Result<(), String> {
    ["vectorcraft-ui-egui", "vectorcraft-mcp"].into_iter().try_for_each(without_bundled_fonts)
}

/// `package` shows vectorcraft-text's `bundled-fonts` in its feature tree with its default features
/// and nowhere without them, for any target.
fn without_bundled_fonts(package: &str) -> Result<(), String> {
    const FEATURE: &str = "vectorcraft-text feature \"bundled-fonts\"";
    let tree = |defaults: bool| -> Result<String, String> {
        let mut args = vec!["tree", "-p", package, "--target", "all", "-e", "features,normal", "-i", "vectorcraft-text"];
        if !defaults {
            args.insert(3, "--no-default-features");
        }
        let what = format!("cargo {}", args.join(" "));
        eprintln!("$ {what}");
        let out = cargo().args(&args).output().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!("{what}: exited with {}:\n{}", out.status, String::from_utf8_lossy(&out.stderr)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let with = tree(true)?;
    if !with.contains(FEATURE) {
        return Err(format!("{package} with its default features shows no {FEATURE} (has cargo tree's output changed?):\n{with}"));
    }
    let without = tree(false)?;
    if without.contains(FEATURE) {
        return Err(format!("{package} without default features still bundles the fonts:\n{without}"));
    }
    Ok(())
}

/// `cargo metadata --no-deps` for the workspace, as JSON.
pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (plan/architecture.md §3)\n");
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<28} {:<14} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Workspace packages that must build for wasm32: all L0–L5 crates, the egui
/// shell, and the web app (its wasm-only code is compiled by nothing else).
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5 || layers::short_name(&c.name) == "ui-egui",
            Some(layers::Class::Standalone) => true,
            _ => layers::short_name(&c.name) == "web",
        })
        .map(|c| c.name)
        .collect())
}

/// One `cargo check` over the whole wasm set: the crates share one build, and `--keep-going`
/// reports every crate that fails rather than only the first.
fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown", "--keep-going"]);
    for pkg in &set {
        c.args(["-p", pkg]);
    }
    run(c, &format!("cargo check --target wasm32-unknown-unknown --keep-going ({} crates: {})", set.len(), set.join(", ")))
}

/// Whether `cargo nextest` is installed.
fn has_nextest() -> bool {
    cargo().args(["nextest", "--version"]).output().is_ok_and(|o| o.status.success())
}

/// The workspace's unit and integration tests, without examples (clippy still checks those).
/// nextest runs each test in its own process, in parallel across all test binaries.
fn cmd_test() -> Result<(), String> {
    const TARGETS: [&str; 3] = ["--lib", "--bins", "--tests"];
    let mut c = cargo();
    if has_nextest() {
        c.args(["nextest", "run", "--workspace"]).args(TARGETS);
        run(c, "cargo nextest run --workspace --lib --bins --tests")
    } else {
        eprintln!("note: cargo-nextest is not installed; running cargo test (cargo install cargo-nextest for faster runs)");
        c.args(["test", "--workspace"]).args(TARGETS);
        run(c, "cargo test --workspace --lib --bins --tests")
    }
}

/// The doctests, which nextest doesn't run.
fn cmd_doctests() -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "--workspace", "--doc"]);
    run(c, "cargo test --workspace --doc")
}

/// `cargo xtask ci`: each step in turn, stopping at the first that fails; the summary names the
/// steps that passed and, after a failure, the one that failed and those skipped.
fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.args(["clippy", "--workspace", "--all-targets", "--keep-going", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --all-targets --keep-going -- -D warnings")
            }),
        ),
        ("test", Box::new(cmd_test)),
        ("doctests", Box::new(cmd_doctests)),
        (
            "no-bundled-fonts",
            Box::new(|| {
                // vectorcraft-text as embedders that bring their own fonts build it.
                let mut c = cargo();
                c.args(["test", "-p", "vectorcraft-text", "--no-default-features", "--test", "without_bundled_fonts"]);
                run(c, "cargo test -p vectorcraft-text --no-default-features --test without_bundled_fonts")?;
                // The egui panels and the MCP server as such an embedder builds them: nothing turns
                // the fonts back on.
                libraries_without_bundled_fonts()?;
                // The panels without their interface faces lint clean and still bind every family
                // they draw in.
                let mut c = cargo();
                c.args(["clippy", "-p", "vectorcraft-ui-egui", "--no-default-features", "--all-targets", "--", "-D", "warnings"]);
                run(c, "cargo clippy -p vectorcraft-ui-egui --no-default-features --all-targets -- -D warnings")?;
                let mut c = cargo();
                c.args(["test", "-p", "vectorcraft-ui-egui", "--no-default-features", "--test", "install_fonts"]);
                run(c, "cargo test -p vectorcraft-ui-egui --no-default-features --test install_fonts")
            }),
        ),
        ("pdf-deps", Box::new(|| pdf_deps::run(&root()))),
        ("assets", Box::new(|| assets::run(&root()))),
        ("brands", Box::new(|| brands::run(&root()))),
        ("cursors", Box::new(|| cursors::run(&root()))),
        ("i18n-report", Box::new(cmd_i18n_report)),
        ("layers", Box::new(cmd_layers)),
        ("wasm", Box::new(cmd_wasm)),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

fn cmd_corpus(download: bool) -> Result<(), String> {
    let corpus = root().join("corpus");
    println!(
        "Test corpora live under {} (git-ignored, never committed).
Tests that use a corpus skip cleanly when it is absent.

  corpus/pngsuite/   Original PngSuite PNGs (redistribution permitted), pinned and SHA-256
                     verified. Fetch: cargo xtask corpus --download
  corpus/psd/        PSD samples from MIT/BSD projects (ag-psd, psd-tools test data).
                     Copy files in manually; licences must be MIT/BSD/CC0.
  corpus/tiff/       libtiff pics (optional)
  corpus/exr/        OpenEXR sample images (optional)
  corpus/raw/        raw.pixls.us samples, CC0 (optional)
  corpus/affinity/   public Affinity documents (CC0/MIT/Apache-2.0) at pinned commits, sha256
                     verified: oracles for the Affinity reader. Fetch: cargo xtask corpus --affinity
",
        corpus.display()
    );
    if !download {
        return Ok(());
    }
    pngsuite::fetch()
}

/// Prints the strings each language's catalog has no row for (`i18n::tests::untranslated_report`).
/// The interface shows them in English, so a gap never fails it; only a scan that finds nothing does.
fn cmd_i18n_report() -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "-p", "vectorcraft-ui-egui", "--lib", "--", "i18n::tests::untranslated_report", "--exact", "--nocapture"]);
    run(c, "cargo test -p vectorcraft-ui-egui --lib -- i18n::tests::untranslated_report --exact --nocapture")
}
