//! A test's temporary folder, removed with everything in it when the test ends.

use std::path::PathBuf;

/// A test's folder, removed with everything in it when dropped, also when the test panics, as
/// the engine's find-files tests remove their trees. A removal that fails (a file still held open)
/// is ignored.
pub(crate) struct Folder(PathBuf);

impl Folder {
    /// A new empty folder `vectorcraft-<name>-<process id>` in the system temporary folder. One
    /// left there by an earlier process with the same id is removed first.
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vectorcraft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// The folder at `dir`, made by the caller, removed when dropped.
    pub(crate) fn adopt(dir: PathBuf) -> Self {
        Self(dir)
    }

    /// `bytes` written to a file `name` in the folder → its path.
    pub(crate) fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }
}

impl std::ops::Deref for Folder {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_folder_is_removed_when_its_test_ends() {
    let folder = Folder::new("ui-folder-ends");
    let path = folder.to_path_buf();
    std::fs::create_dir_all(path.join("nested")).unwrap();
    folder.file("nested/a.txt", b"a");
    assert!(path.join("nested").join("a.txt").is_file());
    drop(folder);
    assert!(!path.exists(), "{} is left", path.display());
}

#[test]
fn a_folder_is_removed_when_its_test_panics() {
    let path = std::env::temp_dir().join(format!("vectorcraft-ui-folder-panics-{}", std::process::id()));
    let r = vectorcraft_testkit::catch_quiet::<()>(|| {
        let folder = Folder::new("ui-folder-panics");
        folder.file("a.txt", b"a");
        panic!("the test fails");
    });
    assert_eq!(r.err().as_deref(), Some("the test fails"));
    assert!(!path.exists(), "{} is left", path.display());
}

#[test]
fn an_adopted_test_kit_folder_is_removed_when_its_test_ends() {
    let folder = Folder::adopt(vectorcraft_testkit::temp_dir("ui-folder-adopted"));
    let path = folder.to_path_buf();
    folder.file("a.txt", b"a");
    drop(folder);
    assert!(!path.exists(), "{} is left", path.display());
}
