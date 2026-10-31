//! The font folders fontconfig is set up with (Linux and the other Unix desktops): the `<dir>`
//! entries of its configuration, `/etc/fonts/fonts.conf` and what that includes (`conf.d`, the
//! user's own files). Distributions and packages add folders outside the usual ones there: TeX
//! fonts under `/usr/share/texmf/fonts`, say, which `fc-list` and other apps then show (#1529).
//!
//! The format is fontconfig's documented one
//! (<https://www.freedesktop.org/software/fontconfig/fontconfig-user.html>, "Configuration file
//! format"): `<dir prefix="…">` and `<include prefix="…">` are all that is read here. The files are
//! read as they are: fontconfig itself (a library, or `fc-list`) isn't asked.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The most configuration files read, and the largest one (they are a few kilobytes each).
const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 1 << 20;
/// The most folders taken, and how deep files may include files.
const MAX_DIRS: usize = 256;
const MAX_DEPTH: usize = 8;

/// A `<dir>` (a font folder) or an `<include>` (another configuration file, or a folder of them)
/// with its `prefix` attribute and its text.
#[derive(Debug, PartialEq)]
struct Entry {
    include: bool,
    prefix: String,
    path: String,
}

/// The `<dir>` and `<include>` elements of a configuration file, in order. Anything that isn't
/// one of them, comments included, is skipped; a file that isn't well formed gives what was read
/// before the damage.
fn entries(xml: &str) -> Vec<Entry> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(at) = rest.find('<') {
        rest = rest.get(at + 1..).unwrap_or_default();
        if let Some(comment) = rest.strip_prefix("!--") {
            rest = comment.split_once("-->").map_or("", |(_, after)| after);
            continue;
        }
        let Some((tag, after)) = rest.split_once('>') else { break };
        let name = tag.split_whitespace().next().unwrap_or_default();
        if !matches!(name, "dir" | "include") || tag.ends_with('/') {
            continue;
        }
        let Some((text, after)) = after.split_once("</") else { break };
        rest = after;
        // No element is inside these: text with a tag in it isn't a path.
        if text.contains('<') {
            continue;
        }
        let path = text.trim().replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
        if !path.is_empty() {
            out.push(Entry { include: name == "include", prefix: attribute(tag, "prefix").unwrap_or_default().to_string(), path });
        }
    }
    out
}

/// The value of attribute `name` in a start tag's text.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before_ok = rest.get(..at).is_some_and(|b| b.ends_with(char::is_whitespace));
        rest = rest.get(at + name.len()..)?;
        let Some(value) = rest.trim_start().strip_prefix('=') else { continue };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|q| matches!(q, '"' | '\''))?;
        let (value, _) = value.get(1..)?.split_once(quote)?;
        if before_ok {
            return Some(value);
        }
    }
    None
}

/// Where the user's files are: the home folder and the XDG data and configuration folders.
#[derive(Clone, Debug, Default)]
pub(crate) struct Places {
    pub home: Option<PathBuf>,
    pub data_home: Option<PathBuf>,
    pub config_home: Option<PathBuf>,
}

impl Places {
    /// The path an entry of the file `from` names: `~` is the home folder, `prefix="xdg"` the XDG
    /// data folder (a `<dir>`) or configuration folder (an `<include>`), `prefix="relative"` the
    /// file's own folder, as is an `<include>` that isn't a full path. A `<dir>` relative to the
    /// folder the app was started in is none.
    fn resolve(&self, e: &Entry, from: &Path) -> Option<PathBuf> {
        // `~` alone or before a `/`: another user's home (`~name`) isn't looked up.
        if let Some(rest) = e.path.strip_prefix('~').filter(|rest| rest.is_empty() || rest.starts_with('/')) {
            return Some(self.home.as_ref()?.join(rest.trim_start_matches('/')));
        }
        let path = Path::new(&e.path);
        let base = match e.prefix.as_str() {
            "xdg" if e.include => self.config_home.as_deref(),
            "xdg" => self.data_home.as_deref(),
            _ if path.is_absolute() => return Some(path.to_path_buf()),
            "relative" => from.parent(),
            _ if e.include => from.parent(),
            _ => None,
        };
        Some(base?.join(path)).filter(|p| p.is_absolute())
    }
}

/// The font folders the configuration file `root` and the files it includes list, in order, each
/// once. Files that are missing or can't be read list none.
pub(crate) fn font_dirs(root: &Path, places: &Places) -> Vec<PathBuf> {
    let mut read = Reader { places, files: HashSet::new(), dirs: vec![] };
    read.file(root, 0);
    read.dirs
}

struct Reader<'a> {
    places: &'a Places,
    /// The files read so far (a file including itself is read once).
    files: HashSet<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Reader<'_> {
    fn file(&mut self, path: &Path, depth: usize) {
        if depth > MAX_DEPTH || self.files.len() >= MAX_FILES || !self.files.insert(path.to_path_buf()) {
            return;
        }
        // A folder of configuration files: those named as fontconfig reads them (a digit first,
        // `.conf` last), in name order.
        if let Ok(list) = std::fs::read_dir(path) {
            let mut files: Vec<PathBuf> = list
                .flatten()
                .take(MAX_FILES)
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with(|c: char| c.is_ascii_digit()) && name.ends_with(".conf")
                })
                .map(|e| e.path())
                .collect();
            files.sort();
            for f in files {
                self.file(&f, depth + 1);
            }
            return;
        }
        // A file that is missing, too large or not text lists nothing.
        if !std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_BYTES) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else { return };
        for e in entries(&text) {
            let Some(to) = self.places.resolve(&e, path) else { continue };
            if e.include {
                self.file(&to, depth + 1);
            } else if self.dirs.len() < MAX_DIRS && !self.dirs.contains(&to) {
                self.dirs.push(to);
            }
        }
    }
}

/// The configuration file fontconfig starts from: `FONTCONFIG_FILE`, else `fonts.conf` in
/// `FONTCONFIG_PATH`, else `/etc/fonts/fonts.conf`.
pub(crate) fn root() -> PathBuf {
    let var = |name| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    var("FONTCONFIG_FILE").or_else(|| var("FONTCONFIG_PATH").map(|d| d.join("fonts.conf"))).unwrap_or_else(|| "/etc/fonts/fonts.conf".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(include: bool, prefix: &str, path: &str) -> Entry {
        Entry { include, prefix: prefix.into(), path: path.into() }
    }

    #[test]
    fn dirs_and_includes_are_read_and_the_rest_is_skipped() {
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <description>Default configuration file</description>
  <!-- <dir>/commented/out</dir> -->
  <dir>/usr/share/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <dir prefix = 'relative' salt="x"> more &amp; more </dir>
  <dir>~/.fonts</dir>
  <dir></dir>
  <cachedir>/var/cache/fontconfig</cachedir>
  <match target="pattern"><test qual="any" name="family"><string>mono</string></test></match>
  <include ignore_missing="yes">conf.d</include>
  <include ignore_missing="yes" prefix="xdg">fontconfig/fonts.conf</include>
  <dir>/cut/off"#;
        assert_eq!(
            entries(xml),
            [
                entry(false, "", "/usr/share/fonts"),
                entry(false, "xdg", "fonts"),
                entry(false, "relative", "more & more"),
                entry(false, "", "~/.fonts"),
                entry(true, "", "conf.d"),
                entry(true, "xdg", "fontconfig/fonts.conf"),
            ]
        );
        // Damaged files and files that aren't XML read as nothing, without a panic.
        for junk in
            ["", "<", "<dir", "<dir>", "<dir><dir></dir>", "</dir>", "<!--", "<dir prefix=>x</dir", "\u{0}<dir prefix=\"", "<dir prefix='é>ü</dir>"]
        {
            let _ = entries(junk);
        }
        assert_eq!(attribute("dir notprefix=\"a\" prefix=\"b\"", "prefix"), Some("b"));
    }

    /// A configuration as distributions lay it out: the main file lists the usual folders and
    /// includes `conf.d`, where a package's file adds its own folder (TeX fonts, #1529) and
    /// another includes the user's files.
    #[test]
    fn folders_of_included_files_are_listed_once_in_order() {
        let tmp = std::env::temp_dir().join(format!("vc-fontconfig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (etc, home) = (tmp.join("etc/fonts"), tmp.join("home"));
        let write = |path: PathBuf, body: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("<fontconfig>{body}</fontconfig>")).unwrap();
        };
        let texmf = tmp.join("usr/share/texmf/fonts/opentype");
        write(
            etc.join("fonts.conf"),
            &format!(
                "<dir>{}</dir><dir prefix=\"xdg\">fonts</dir><dir>~/.fonts</dir><dir>fonts-of-the-working-folder</dir>\
                 <include ignore_missing=\"yes\">conf.d</include><include>{}</include><include>fonts.conf</include>",
                tmp.join("usr/share/fonts").display(),
                etc.join("missing.conf").display()
            ),
        );
        write(etc.join("conf.d/09-texlive.conf"), &format!("<dir>{}</dir>", texmf.display()));
        write(etc.join("conf.d/50-user.conf"), "<include ignore_missing=\"yes\" prefix=\"xdg\">fontconfig/fonts.conf</include>");
        write(etc.join("conf.d/README"), "<dir>/not/a/conf/file</dir>");
        write(etc.join("conf.d/user.conf"), "<dir>/no/digit/first</dir>");
        write(home.join(".config/fontconfig/fonts.conf"), &format!("<dir prefix=\"relative\">mine</dir><dir>{}</dir>", texmf.display()));
        let places = Places { home: Some(home.clone()), data_home: Some(home.join(".local/share")), config_home: Some(home.join(".config")) };
        assert_eq!(
            font_dirs(&etc.join("fonts.conf"), &places),
            [tmp.join("usr/share/fonts"), home.join(".local/share/fonts"), home.join(".fonts"), texmf, home.join(".config/fontconfig/mine")]
        );
        // Without a home folder, what needs one is left out; a missing file lists nothing.
        assert_eq!(font_dirs(&etc.join("fonts.conf"), &Places::default()).len(), 2);
        assert!(font_dirs(&tmp.join("none.conf"), &places).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
