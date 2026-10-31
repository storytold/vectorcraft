//! Pinned PngSuite files: verify bounded HTTPS downloads before publishing, without extracting archives.
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{root, run, sha256};

const SOURCE: &str = "https://raw.githubusercontent.com/lunapaint/pngsuite/8cd768dd0d0063195174d0d01cacbd5a7d1e5605";
const MANIFEST: &str = include_str!("../pngsuite.sha256");
const MAX_FILE: u64 = 1024 * 1024;
const MAX_TOTAL: u64 = 16 * MAX_FILE;
const MAX_FILES: usize = 256;

#[derive(Debug)]
struct Entry<'a> {
    sha: &'a str,
    bytes: u64,
    name: &'a str,
}

fn entries(manifest: &str) -> Result<Vec<Entry<'_>>, String> {
    let mut result = Vec::new();
    let mut names = HashSet::new();
    let mut total = 0u64;
    for line in manifest.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let fields: Vec<_> = line.split_whitespace().collect();
        let [sha, size, name] = fields.as_slice() else { return Err("invalid PngSuite manifest row".into()) };
        let png = name.strip_suffix(".png").is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
        if !png && !matches!(*name, "LICENSE" | "PngSuite.README") {
            return Err(format!("unexpected PngSuite filename: {name}"));
        }
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(format!("invalid PngSuite sha256: {name}"));
        }
        let bytes = size.parse::<u64>().map_err(|_| format!("invalid PngSuite size: {name}"))?;
        if bytes == 0 || bytes > MAX_FILE || bytes > MAX_TOTAL.saturating_sub(total) || result.len() >= MAX_FILES || !names.insert(*name) {
            return Err(format!("PngSuite manifest budget or duplicate filename: {name}"));
        }
        total += bytes;
        result.push(Entry { sha, bytes, name });
    }
    if !result.iter().any(|e| e.name.ends_with(".png")) {
        return Err("PngSuite manifest has no PNG files".into());
    }
    Ok(result)
}

fn verified(path: &Path, entry: &Entry<'_>) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if !metadata.file_type().is_file() {
        return Err(format!("{}: expected a regular corpus file", path.display()));
    }
    if metadata.len() != entry.bytes {
        return Ok(false);
    }
    let mut data = Vec::new();
    File::open(path).and_then(|f| f.take(entry.bytes + 1).read_to_end(&mut data)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(data.len() as u64 == entry.bytes && sha256::hex(&data) == entry.sha)
}

struct Download(PathBuf);
impl Drop for Download {
    fn drop(&mut self) {
        // Best-effort removal of our disposable download on success, rejection or network failure.
        let _ = fs::remove_file(&self.0);
    }
}

fn fetch_into(dest: &Path, manifest: &str, mut download: impl FnMut(&str, &Path, u64) -> Result<(), String>) -> Result<usize, String> {
    let entries = entries(manifest)?;
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    if !fs::symlink_metadata(dest).map_err(|e| e.to_string())?.file_type().is_dir() {
        return Err("PngSuite destination must be a regular directory".into());
    }
    let mut count = 0;
    for entry in entries {
        let out = dest.join(entry.name);
        if !verified(&out, &entry)? {
            let temporary = dest.join(format!(".{}-{}.download", entry.name, std::process::id()));
            // Reserve a new regular file, never reuse somebody else's stale download or link.
            OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|e| format!("{}: {e}", temporary.display()))?;
            let temporary = Download(temporary);
            let folder = if entry.name.ends_with(".png") { "/png" } else { "" };
            download(&format!("{SOURCE}{folder}/{}", entry.name), &temporary.0, entry.bytes)?;
            if !verified(&temporary.0, &entry)? {
                return Err(format!("{}: size or sha256 does not match xtask/pngsuite.sha256", entry.name));
            }
            fs::rename(&temporary.0, &out).map_err(|e| format!("publish {}: {e}", out.display()))?;
        }
        count += usize::from(entry.name.ends_with(".png"));
    }
    Ok(count)
}

fn supported_curl(version: &str) -> bool {
    let Some(version) = version.strip_prefix("curl ").and_then(|s| s.split_whitespace().next()) else { return false };
    let mut parts = version.split('.');
    let major = parts.next().and_then(|s| s.parse::<u64>().ok());
    let minor = parts.next().and_then(|s| s.parse::<u64>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if (major, minor) >= (8, 4))
}

pub fn fetch() -> Result<(), String> {
    let dest = root().join("corpus/pngsuite");
    let mut checked_curl = false;
    let count = fetch_into(&dest, MANIFEST, |url, path, bytes| {
        if !checked_curl {
            // Older curl only bounds responses whose length is known before downloading.
            let version = Command::new("curl").arg("--version").output().map_err(|e| format!("curl --version: {e}"))?;
            if !version.status.success() || !supported_curl(&String::from_utf8_lossy(&version.stdout)) {
                return Err("PngSuite downloading requires curl 8.4 or newer to bound unknown-length responses".into());
            }
            checked_curl = true;
        }
        let mut curl = Command::new("curl");
        curl.args([
            "-fsSL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "--retry",
            "2",
            "--max-filesize",
        ])
        .arg(bytes.to_string())
        .arg("-o")
        .arg(path)
        .arg(url);
        run(curl, &format!("curl {url}"))
    })?;
    println!("PngSuite: {count} PNG files verified in {}", dest.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("vectorcraft-pngsuite-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn manifest(data: &[u8]) -> String {
        format!("{} {} test.png\n", sha256::hex(data), data.len())
    }

    #[test]
    fn substituted_and_oversized_downloads_never_replace_corpus_files() {
        let temp = Temp::new();
        let out = temp.0.join("test.png");
        let good = b"original PNG oracle";
        for replacement in [b"substitutePNGoracle".as_slice(), &[0; 100]] {
            fs::write(&out, b"old corpus data").unwrap();
            let error = fetch_into(&temp.0, &manifest(good), |_, path, limit| {
                assert_eq!(limit, good.len() as u64);
                fs::write(path, replacement).unwrap();
                Ok(())
            })
            .unwrap_err();
            assert!(error.contains("size or sha256"));
            assert_eq!(fs::read(&out).unwrap(), b"old corpus data");
            assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
        }
    }

    #[test]
    fn verified_cache_is_reused_and_corrupt_cache_is_repaired() {
        let temp = Temp::new();
        let data = b"verified oracle";
        let m = manifest(data);
        let mut requests = 0;
        for _ in 0..2 {
            assert_eq!(
                fetch_into(&temp.0, &m, |url, path, _| {
                    requests += 1;
                    assert!(url.starts_with(SOURCE) && url.ends_with("/png/test.png"));
                    fs::write(path, data).unwrap();
                    Ok(())
                })
                .unwrap(),
                1
            );
        }
        assert_eq!(requests, 1);
        fs::write(temp.0.join("test.png"), b"wrong oracle!!!").unwrap();
        fetch_into(&temp.0, &m, |_, path, _| {
            fs::write(path, data).unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(temp.0.join("test.png")).unwrap(), data);
    }

    #[test]
    fn network_failures_clean_staging_and_nonregular_targets_are_rejected() {
        let temp = Temp::new();
        let m = manifest(b"png");
        assert!(
            fetch_into(&temp.0, &m, |_, path, _| {
                fs::write(path, b"partial").unwrap();
                Err("network failed".into())
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 0);
        fs::create_dir(temp.0.join("test.png")).unwrap();
        let e = fetch_into(&temp.0, &m, |_, _, _| panic!("must not download over a nonregular file")).unwrap_err();
        assert!(e.contains("regular corpus file"));
    }

    #[test]
    fn stale_download_is_never_overwritten() {
        let temp = Temp::new();
        let stale = temp.0.join(format!(".test.png-{}.download", std::process::id()));
        fs::write(&stale, b"keep this earlier download").unwrap();
        assert!(fetch_into(&temp.0, &manifest(b"png"), |_, _, _| panic!("must not reuse an existing staging file")).is_err());
        assert_eq!(fs::read(stale).unwrap(), b"keep this earlier download");
        assert!(!temp.0.join("test.png").exists());
    }

    #[test]
    fn manifest_rejects_unexpected_names_duplicates_and_resource_budgets() {
        let sha = sha256::hex(b"png");
        for name in ["../test.png", "a/b.png", "a\\b.png", "a:stream.png", "script.exe", "a..png"] {
            assert!(entries(&format!("{sha} 3 {name}\n")).is_err());
        }
        for m in [format!("{sha} 0 a.png"), format!("{sha} {} a.png", MAX_FILE + 1), format!("{sha} 3 a.png\n{sha} 3 a.png"), "bad 3 a.png".into()] {
            assert!(entries(&m).is_err());
        }
        let m: String = (0..17).map(|i| format!("{sha} {MAX_FILE} a{i}.png\n")).collect();
        assert!(entries(&m).is_err());
        let m: String = (0..257).map(|i| format!("{sha} 1 a{i}.png\n")).collect();
        assert!(entries(&m).is_err());
    }

    #[test]
    fn pinned_manifest_contains_the_original_175_pngs_and_redistribution_notice() {
        let e = entries(MANIFEST).unwrap();
        assert_eq!(e.iter().filter(|e| e.name.ends_with(".png")).count(), 175);
        assert!(e.iter().any(|e| e.name == "LICENSE"));
        assert!(e.iter().any(|e| e.name == "PngSuite.README"));
    }

    #[test]
    fn old_curl_without_unknown_length_download_limits_is_rejected() {
        for version in ["curl 7.88.1", "curl 8.3.0", "curl invalid", "not curl"] {
            assert!(!supported_curl(version));
        }
        for version in ["curl 8.4.0 libcurl/8.4.0", "curl 8.21.0 (Windows)", "curl 9.0.0"] {
            assert!(supported_curl(version));
        }
    }
}
