//! Shared helpers for the MCP integration tests.

use std::path::{Path, PathBuf};

/// A test's own folder, made by [`vectorcraft_testkit::temp_dir`] (with its checks against a
/// folder made by someone else) and removed with everything in it when dropped, also when the
/// test panics. A removal that fails (a file still open) is ignored. Tests that run at the same
/// time use different tags.
pub struct TempFolder(PathBuf);

impl TempFolder {
    pub fn new(tag: &str) -> Self {
        Self(vectorcraft_testkit::temp_dir(tag))
    }
}

impl std::ops::Deref for TempFolder {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
