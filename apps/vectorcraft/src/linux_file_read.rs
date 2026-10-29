//! Linux desktop reads must not consume pipes/devices or allocate without a byte limit.

use rustix::fs::{Mode, OFlags};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Match the native format's 512 MiB unpacked limit for desktop input bytes.
const MAX_BYTES: u64 = 512 << 20;

/// Read a bounded regular file, following ordinary document symlinks.
pub fn read(path: &str) -> Result<Vec<u8>, String> {
    read_capped(Path::new(path), MAX_BYTES)
}

/// Check before opening to avoid device opens; check the descriptor again to close path races.
fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    if !std::fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("Only regular files can be opened; pipes, devices and directories are not supported".into());
    }
    // NONBLOCK prevents a regular-file-to-FIFO replacement from blocking open; NOCTTY avoids
    // acquiring a controlling terminal if a path is replaced with a terminal device.
    let fd =
        rustix::fs::open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC, Mode::empty()).map_err(|e| e.to_string())?;
    read_open_file(File::from(fd), max)
}

/// Validate the actual opened object and cap reads even when it grows or reports length zero.
fn read_open_file(file: File, max: u64) -> Result<Vec<u8>, String> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Only regular files can be opened; the path changed to a special file".into());
    }
    let too_large = || format!("File exceeds the desktop input limit of {max} bytes");
    if meta.len() > max {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A private fixture directory, removed after each test without touching user files.
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        /// Allocate a test-specific directory under the OS temporary folder.
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("vectorcraft-read-{}-{name}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        /// Remove only the files created inside this fixture directory.
        fn drop(&mut self) {
            for entry in std::fs::read_dir(&self.0).unwrap().flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    /// Exercise the actual desktop service, including symlinks to devices.
    #[test]
    fn desktop_service_rejects_devices_and_directories() {
        let dir = Fixture::new("devices");
        let link = dir.0.join("device.svg");
        symlink("/dev/zero", &link).unwrap();
        let services = crate::services(None, egui::Context::default());
        let read = services.read.unwrap();
        for path in [Path::new("/dev/null"), Path::new("/dev/zero"), &dir.0, &link] {
            assert!(read(path.to_str().unwrap()).unwrap_err().contains("regular files"));
        }
    }

    /// FIFOs and a descriptor swapped after the path check cannot enter a blocking read.
    #[test]
    fn fifo_and_opened_special_descriptor_are_rejected() {
        let dir = Fixture::new("fifo");
        let path = dir.0.join("pipe.svg");
        rustix::fs::mkfifoat(rustix::fs::CWD, &path, Mode::RUSR | Mode::WUSR).unwrap();
        assert!(read(path.to_str().unwrap()).unwrap_err().contains("regular files"));
        let fd = rustix::fs::open(&path, OFlags::RDONLY | OFlags::NONBLOCK, Mode::empty()).unwrap();
        assert!(read_open_file(File::from(fd), 16).unwrap_err().contains("regular files"));
    }

    /// Normal symlinks still work; sparse oversized files are refused before allocation.
    #[test]
    fn regular_files_symlinks_and_byte_limits() {
        let dir = Fixture::new("regular");
        let path = dir.0.join("日本語.svg");
        std::fs::write(&path, b"test").unwrap();
        let link = dir.0.join("linked.svg");
        symlink(&path, &link).unwrap();
        assert_eq!(read(link.to_str().unwrap()).unwrap(), b"test");
        assert_eq!(read_capped(&path, 4).unwrap(), b"test");
        assert!(read_capped(&path, 3).unwrap_err().contains("input limit"));
        File::options().write(true).open(&path).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert!(read(path.to_str().unwrap()).unwrap_err().contains("input limit"));
        // procfs reports zero length: the streaming cap must still apply.
        assert!(read_capped(Path::new("/proc/self/status"), 16).unwrap_err().contains("input limit"));
    }
}
