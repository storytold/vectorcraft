//! VectorCraft test helpers.
//!
//! - [`fixtures`]: sessions and documents in known states (empty, single/multi selection, a "rich"
//!   document touching most node kinds), plus a [`fixtures::DocBuilder`] for building documents
//!   directly (render tests).
//! - [`strategies`]: proptest strategies for geometry and for engine command sequences ([`strategies::Op`]), and
//!   junk-parameter generators for fuzzing the command registry.
//! - [`invariants`]: structural checks for documents and sessions, and round-trip checks
//!   (`.vectorcraft`, SVG, `document.inspect`).
//! - [`raster`]: rendering helpers and image comparison with a perceptual tolerance.
//! - [`geom`]: geometry assertions (approximate equality, curve sampling, Hausdorff distance).
//! - [`pdf`]: hand-written PDF files (page boxes, colour spaces, encryption) for import tests.
//! - [`ai`]: hand-written Illustrator editing data, and the EPS and `.ai` files that carry it.
//! - [`ase`]: hand-written swatch exchange (`.ase`) files for swatch library tests.
//! - [`tiff`]: hand-written TIFF files, chunky or planar, or compressed with CCITT Group 3, for
//!   image import tests.
//! - [`fonts`]: font files made from the bundled fonts (a renamed family) for font tests.
//!
//! This crate may only be used as a dev-dependency (enforced by `cargo xtask layers`).
// Test support only (a dev-dependency of every crate that uses it): a failed setup or assertion
// must panic, like `assert!`, so the shipped-code ban on panicking (AGENTS.md › Robustness) doesn't
// apply here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![forbid(unsafe_code)]

pub mod ai;
pub mod ase;
pub mod fixtures;
pub mod fonts;
pub mod geom;
pub mod invariants;
pub mod pdf;
pub mod raster;
pub mod strategies;
pub mod tiff;

pub use serde_json::{Value, json};
pub use vectorcraft_engine::{Session, doc::NodeId};
/// Re-exports so dependents without direct dependencies (e.g. app test crates) can use them.
pub use {vectorcraft_doc as doc, vectorcraft_format as format, vectorcraft_render as render, vectorcraft_svg as svg};

/// A per-process temporary directory for test output (created on first use). It is made with
/// `create_dir` (owner-only on Unix), and a directory that already exists is reused only when it is a
/// real directory, not a link, with no access for others and writable by this user: one pre-made in a shared temp folder is refused.
pub fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-testkit-{tag}-{}", std::process::id()));
    // Unix sets the mode on it; elsewhere it stays as made.
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    if let Err(e) = b.create(&dir) {
        let m = std::fs::symlink_metadata(&dir).expect("temp dir");
        assert!(e.kind() == std::io::ErrorKind::AlreadyExists && m.is_dir(), "create temp dir: {e}");
        #[cfg(unix)]
        assert!(std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o077 == 0, "{}: not private", dir.display());
        // A private directory made by another user passes the mode check but is closed to this one:
        // writing a probe file tells it apart from ours without needing the user ids.
        // Named for this call: tests running alongside probe the same directory.
        static PROBES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let probe = dir.join(format!(".probe-{}", PROBES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        assert!(std::fs::write(&probe, b"").and_then(|()| std::fs::remove_file(&probe)).is_ok(), "{}: not owned by this user", dir.display());
    }
    dir
}

/// How many shapes an SVG draws: its `<path>` elements and the basic shapes SVG export writes
/// simple shapes as (`<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, `<polygon>`).
pub fn svg_shapes(svg: &str) -> usize {
    ["<path", "<rect", "<circle", "<ellipse", "<line ", "<polyline", "<polygon"].iter().map(|t| svg.matches(t).count()).sum()
}

thread_local! {
    static QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f`, catching a panic without printing it (other threads' panics still print). Returns the
/// panic message on panic.
pub fn catch_quiet<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !QUIET.with(|q| q.get()) {
                prev(info);
            }
        }));
    });
    QUIET.with(|q| q.set(true));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    QUIET.with(|q| q.set(false));
    r.map_err(|p| p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into()))
}
