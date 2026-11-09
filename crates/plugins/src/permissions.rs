//! What a plug-in may do: the permissions its manifest lists (`permissions`), which the user
//! approves when it is installed ([`Grant`], kept in [`crate::registry`]), and which the host
//! enforces on every run.
//!
//! `doc.read`, `doc.write`, `state` and `progress` are the names of the plug-in capability model in
//! storytold/photocraft#662; the other names are VectorCraft's own. A manifest without
//! `permissions` gets the ones its kind implies ([`Permission::implied`]): what every plug-in of
//! that kind could do before manifests listed them. `progress`, `settings` and `cache`, which let a
//! plug-in use the host imports of API level 2 ([`crate::host`]), are never implied: a manifest
//! lists them.

use serde::{Deserialize, Serialize};

use crate::{Error, Kind, Result};

/// The plug-in API level this host implements. A manifest's `api` (1 when left out) may be at most
/// this.
pub const API_LEVEL: u32 = 2;

/// The API level that adds the host imports ([`crate::host::IMPORTS`]) and the `progress`,
/// `settings` and `cache` permissions they need.
pub const PROGRESS_API: u32 = 2;

/// Most permissions a manifest may list.
pub const MAX_PERMISSIONS: usize = 16;

/// Longest plug-in name a prompt shows, in characters.
pub const MAX_SHOWN_NAME: usize = 64;

/// Longest version or id a prompt shows, in characters.
pub const MAX_SHOWN_FIELD: usize = 64;

/// Names kept for host calls a later plug-in API adds. This host grants none of them, so a
/// manifest that lists one is refused when it is installed.
pub const RESERVED: &[&str] = &["ui.panel", "ui.tool"];

/// One thing a plug-in may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Read the objects the host hands it: a filter's selected paths, the object a live effect is
    /// on, the document an exporter writes.
    #[serde(rename = "doc.read")]
    DocRead,
    /// Change, add and delete objects (a filter's output), as one undo step.
    #[serde(rename = "doc.write")]
    DocWrite,
    /// Read and write the data it keeps with the document (a filter's `documentData`).
    #[serde(rename = "state")]
    State,
    /// Change how the objects its live effect is applied to are drawn.
    #[serde(rename = "effect.render")]
    EffectRender,
    /// Write the document as a file in its own format (an exporter).
    #[serde(rename = "export.write")]
    ExportWrite,
    /// Report how far a run has got, see that the user asked it to stop and write log lines: the
    /// host imports of API level 2 ([`crate::host::IMPORTS`]).
    #[serde(rename = "progress")]
    Progress,
    /// Keep settings of its own between runs and sessions ([`crate::storage`]).
    #[serde(rename = "settings")]
    Settings,
    /// Keep a cache of its own, which the user can clear ([`crate::storage`]).
    #[serde(rename = "cache")]
    Cache,
}

/// Every permission this host grants, in the order prompts list them.
pub const ALL: [Permission; 8] = [
    Permission::DocRead,
    Permission::DocWrite,
    Permission::State,
    Permission::EffectRender,
    Permission::ExportWrite,
    Permission::Progress,
    Permission::Settings,
    Permission::Cache,
];

/// The permissions of the host imports, which any kind may list and none implies; each needs plug-in
/// API level [`PROGRESS_API`].
pub const HOST: [Permission; 3] = [Permission::Progress, Permission::Settings, Permission::Cache];

impl Permission {
    /// Its name in a manifest and in `plugin.install`'s `grant`.
    pub fn name(self) -> &'static str {
        match self {
            Permission::DocRead => "doc.read",
            Permission::DocWrite => "doc.write",
            Permission::State => "state",
            Permission::EffectRender => "effect.render",
            Permission::ExportWrite => "export.write",
            Permission::Progress => "progress",
            Permission::Settings => "settings",
            Permission::Cache => "cache",
        }
    }

    /// The permission named `name`.
    pub fn parse(name: &str) -> Option<Permission> {
        ALL.into_iter().find(|p| p.name() == name)
    }

    /// What it lets a plug-in do, in plain words (English; the app shows its own translation).
    pub fn describe(self) -> &'static str {
        match self {
            Permission::DocRead => "Read the objects it is given",
            Permission::DocWrite => "Change, add and delete objects in the document",
            Permission::State => "Keep data of its own with the document",
            Permission::EffectRender => "Change how the objects its effect is applied to are drawn",
            Permission::ExportWrite => "Write the document as a file in its own format",
            Permission::Progress => "Show its progress, see Cancel and write log lines",
            Permission::Settings => "Keep settings of its own",
            Permission::Cache => "Keep a cache of its own, which you can clear",
        }
    }

    /// May a plug-in of `kind` that declares `commands` or not list it: one [`Permission::allowed`]
    /// gives it, or one of the host imports' ([`HOST`]), which any kind may list.
    pub fn applies(self, kind: Kind, commands: bool) -> bool {
        HOST.contains(&self) || Permission::allowed(kind, commands).contains(&self)
    }

    /// The permissions a manifest of `kind` without `permissions` gets. A manifest may also list
    /// the host imports' ([`HOST`], [`Permission::applies`]).
    pub fn implied(kind: Kind) -> &'static [Permission] {
        match kind {
            Kind::Filter => &[Permission::DocRead, Permission::DocWrite, Permission::State],
            Kind::Effect => &[Permission::DocRead, Permission::EffectRender],
            Kind::Exporter => &[Permission::DocRead, Permission::ExportWrite],
        }
    }

    /// The permissions a plug-in of `kind` that declares `commands` or not may list besides
    /// `progress`, which are also the ones its manifest gets without `permissions`: those `kind`
    /// implies, and for a live effect with commands also `doc.write` and `state`, which only its
    /// commands use (drawing the effect changes no object and keeps no data).
    pub fn allowed(kind: Kind, commands: bool) -> &'static [Permission] {
        match kind {
            Kind::Effect if commands => &[Permission::DocRead, Permission::DocWrite, Permission::State, Permission::EffectRender],
            _ => Permission::implied(kind),
        }
    }

    /// The permissions a plug-in of `kind` can't work without, which its list must hold: a live
    /// effect and an exporter are always handed objects, and only draw or write. A filter may
    /// leave any of its permissions out.
    fn required(kind: Kind) -> &'static [Permission] {
        match kind {
            Kind::Filter => &[],
            Kind::Effect | Kind::Exporter => Permission::implied(kind),
        }
    }
}

/// Checks the `permissions` a manifest of `kind` lists → them, in the order [`ALL`] has; `commands`
/// is whether it declares commands ([`Permission::allowed`]). A name that is not a permission, a
/// reserved one, one that doesn't apply to the kind, one listed twice and a list without what the
/// kind needs are errors that name it.
pub(crate) fn validate(kind: Kind, commands: bool, names: &[String]) -> Result<Vec<Permission>> {
    if names.len() > MAX_PERMISSIONS {
        return Err(Error::Manifest(format!("at most {MAX_PERMISSIONS} `permissions`")));
    }
    let mut out: Vec<Permission> = Vec::with_capacity(names.len());
    for name in names {
        let shown = display_text(name, MAX_SHOWN_FIELD);
        let Some(p) = Permission::parse(name) else {
            if RESERVED.contains(&name.as_str()) {
                return Err(Error::Manifest(format!("permission `{shown}` is reserved for a later plug-in API; this host doesn't grant it")));
            }
            return Err(Error::Manifest(format!("unknown permission `{shown}`")));
        };
        if !p.applies(kind, commands) {
            return Err(Error::Manifest(format!("permission `{shown}` doesn't apply to a plug-in of kind `{}`", kind.name())));
        }
        if out.contains(&p) {
            return Err(Error::Manifest(format!("permission `{shown}` is listed twice")));
        }
        out.push(p);
    }
    if let Some(missing) = Permission::required(kind).iter().find(|p| !out.contains(p)) {
        return Err(Error::Manifest(format!("a plug-in of kind `{}` needs the permission `{}`", kind.name(), missing.name())));
    }
    out.sort();
    Ok(out)
}

/// The names of `permissions`, as a manifest and `grant` list them.
pub fn names(permissions: &[Permission]) -> Vec<&'static str> {
    permissions.iter().map(|p| p.name()).collect()
}

/// Is `c` a character that changes the direction text is laid out in, or a line or paragraph
/// separator, which a prompt must not let a manifest string use to disguise itself?
fn is_bidi_or_separator(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}')
}

/// `s` as a prompt shows a string from a manifest: control, bidirectional formatting and line
/// separator characters written as `\u{202E}` escapes, and at most `max` characters, the last of
/// them `…` when it was cut.
pub fn display_text(s: &str, max: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    // Characters written so far, and the byte length of the longest prefix of whole pieces
    // shorter than `max` characters, where the ellipsis goes when the text is cut.
    let mut count = 0;
    let mut cut = 0;
    for c in s.chars() {
        let start = out.len();
        let n = if c.is_control() || is_bidi_or_separator(c) {
            let _ = write!(out, "\\u{{{:04X}}}", c as u32);
            out.len() - start
        } else {
            out.push(c);
            1
        };
        if count + n > max {
            out.truncate(cut);
            out.push('…');
            return out;
        }
        count += n;
        if count < max {
            cut = out.len();
        }
    }
    out
}

/// The SHA-256 of `bytes`, in lower-case hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Is `s` a SHA-256 as [`sha256_hex`] writes it: 64 lower-case hexadecimal digits?
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// What the user approved for a plug-in: its id, the SHA-256 of the module they approved
/// ([`sha256_hex`]) and the permissions it was granted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub hash: String,
    pub permissions: Vec<Permission>,
}

impl Grant {
    /// Does it grant every one of `wanted` to the module with this `id` and SHA-256 `hash`? A grant
    /// whose own hash is not a valid SHA-256 ([`sha256_hex`]) covers nothing.
    pub fn covers(&self, id: &str, hash: &str, wanted: &[Permission]) -> bool {
        self.id == id && is_sha256_hex(&self.hash) && self.hash == hash && wanted.iter().all(|w| self.permissions.contains(w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_serialize_as_names() {
        for p in ALL {
            assert_eq!(Permission::parse(p.name()), Some(p));
            assert_eq!(serde_json::to_value(p).unwrap(), serde_json::json!(p.name()));
        }
        assert_eq!(Permission::parse("document.read"), None);
        assert!(RESERVED.iter().all(|r| Permission::parse(r).is_none()), "reserved names are not granted");
    }

    #[test]
    fn lists_are_checked_against_the_kind() {
        let list = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(validate(Kind::Filter, false, &list(&["doc.write", "doc.read"])).unwrap(), [Permission::DocRead, Permission::DocWrite]);
        assert_eq!(validate(Kind::Filter, false, &[]).unwrap(), []);
        assert!(validate(Kind::Effect, false, &list(&["doc.read", "effect.render"])).is_ok());
        assert!(validate(Kind::Exporter, false, &list(&["export.write", "doc.read"])).is_ok());
        // Any kind may list `progress`, which no kind implies.
        assert_eq!(validate(Kind::Filter, false, &list(&["progress", "doc.read"])).unwrap(), [Permission::DocRead, Permission::Progress]);
        assert!(validate(Kind::Effect, false, &list(&["doc.read", "effect.render", "progress"])).is_ok());
        assert!(validate(Kind::Effect, true, &list(&["doc.read", "effect.render", "progress"])).is_ok());
        assert!(validate(Kind::Exporter, false, &list(&["progress", "export.write", "doc.read"])).is_ok());
        assert!([Kind::Filter, Kind::Effect, Kind::Exporter].iter().all(|k| !Permission::implied(*k).contains(&Permission::Progress)));
        // So may `settings` and `cache`, which no kind implies either.
        assert_eq!(
            validate(Kind::Effect, false, &list(&["cache", "settings", "doc.read", "effect.render"])).unwrap(),
            [Permission::DocRead, Permission::EffectRender, Permission::Settings, Permission::Cache]
        );
        assert!(validate(Kind::Exporter, false, &list(&["settings", "export.write", "doc.read"])).is_ok());
        assert_eq!(validate(Kind::Filter, false, &list(&["cache"])).unwrap(), [Permission::Cache]);
        for kind in [Kind::Filter, Kind::Effect, Kind::Exporter] {
            for commands in [false, true] {
                assert!(HOST.iter().all(|p| p.applies(kind, commands) && !Permission::implied(kind).contains(p)));
            }
        }
        // A live effect with commands may also change the document and keep data with it.
        assert_eq!(
            validate(Kind::Effect, true, &list(&["state", "effect.render", "doc.write", "doc.read"])).unwrap(),
            [Permission::DocRead, Permission::DocWrite, Permission::State, Permission::EffectRender]
        );
        assert_eq!(Permission::allowed(Kind::Effect, false), Permission::implied(Kind::Effect));
        assert_eq!(Permission::allowed(Kind::Filter, true), Permission::implied(Kind::Filter));
        for (kind, commands, names, says) in [
            (Kind::Filter, false, &["doc.read", "clipboard"][..], "unknown permission `clipboard`"),
            (Kind::Filter, false, &["ui.panel"], "`ui.panel` is reserved"),
            (Kind::Filter, false, &["progress", "progress"], "`progress` is listed twice"),
            (Kind::Filter, false, &["export.write"], "doesn't apply to a plug-in of kind `filter`"),
            (Kind::Exporter, false, &["doc.read", "export.write", "doc.read"], "`doc.read` is listed twice"),
            (Kind::Effect, false, &["doc.read"], "needs the permission `effect.render`"),
            (Kind::Effect, false, &["doc.read", "effect.render", "doc.write"], "`doc.write` doesn't apply to a plug-in of kind `effect`"),
            (Kind::Effect, true, &["doc.read", "doc.write"], "needs the permission `effect.render`"),
            (Kind::Effect, true, &["doc.read", "effect.render", "export.write"], "`export.write` doesn't apply"),
            (Kind::Exporter, true, &["doc.read", "export.write", "doc.write"], "`doc.write` doesn't apply"),
            (Kind::Exporter, false, &["export.write"], "needs the permission `doc.read`"),
            (Kind::Filter, false, &["cache", "cache"], "`cache` is listed twice"),
            (Kind::Filter, false, &["storage"], "unknown permission `storage`"),
        ] {
            let e = validate(kind, commands, &list(names)).unwrap_err().to_string();
            assert!(e.contains(says), "{names:?}: {e}");
        }
        let many = list(&["doc.read"; MAX_PERMISSIONS + 1]);
        assert!(validate(Kind::Filter, false, &many).unwrap_err().to_string().contains("at most"));
    }

    /// Control, bidirectional override and separator characters show as escapes, so a name can't
    /// reverse or hide the text after it; long text is cut with an ellipsis.
    #[test]
    fn prompt_text_escapes_hidden_characters_and_is_capped() {
        assert_eq!(display_text("Tidy\u{202E}fdp.exe", 64), "Tidy\\u{202E}fdp.exe");
        assert_eq!(display_text("a\u{0007}b\nc\u{2066}d\u{200F}", 64), "a\\u{0007}b\\u{000A}c\\u{2066}d\\u{200F}");
        assert_eq!(display_text("plain", 64), "plain");
        assert_eq!(display_text("abcdef", 6), "abcdef");
        assert_eq!(display_text("abcdefg", 6), "abcde…");
        assert_eq!(display_text(&"x".repeat(500), MAX_SHOWN_NAME).chars().count(), MAX_SHOWN_NAME);
        assert_eq!(display_text("ab\u{202E}", 6), "ab…", "an escape that doesn't fit is cut whole");
        assert_eq!(display_text("ab\u{0007}", 10), "ab\\u{0007}", "an escape that ends exactly at the cap is kept");
        let long = display_text(&"\u{202E}y".repeat(500_000), MAX_SHOWN_NAME);
        assert_eq!(long.chars().count(), MAX_SHOWN_NAME);
        assert!(long.starts_with("\\u{202E}y\\u{202E}y") && long.ends_with('…'), "{long}");
    }

    #[test]
    fn hashes_are_sha256_in_hex() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_grant_covers_its_permissions_and_no_more() {
        let hash = sha256_hex(b"module");
        let g = Grant { id: "a".into(), hash: hash.clone(), permissions: vec![Permission::DocRead, Permission::DocWrite] };
        assert!(g.covers("a", &hash, &[Permission::DocRead]));
        assert!(g.covers("a", &hash, &[]));
        assert!(!g.covers("a", &hash, &[Permission::State]));
        assert!(!g.covers("a", &hash, &[Permission::DocRead, Permission::State]));
    }

    #[test]
    fn a_grant_covers_only_the_module_that_was_approved() {
        let hash = sha256_hex(b"module");
        let wanted = [Permission::DocRead];
        let g = Grant { id: "a".into(), hash: hash.clone(), permissions: vec![Permission::DocRead] };
        assert!(g.covers("a", &hash, &wanted));
        assert!(!g.covers("b", &hash, &wanted), "another id");
        assert!(!g.covers("a", &sha256_hex(b"other"), &wanted), "other bytes");
        assert!(!g.covers("a", &sha256_hex(b"other"), &[]), "other bytes, even when nothing is wanted");
    }

    /// Damaged data can't stand for every module, even when the hash asked about is just as damaged.
    #[test]
    fn a_grant_with_an_invalid_hash_covers_nothing() {
        let upper = sha256_hex(b"module").to_uppercase();
        let short = "ab".repeat(31);
        let long = "ab".repeat(33);
        let not_hex = "g".repeat(64);
        let wide = "é".repeat(32);
        for hash in ["", "00", upper.as_str(), short.as_str(), long.as_str(), not_hex.as_str(), wide.as_str()] {
            let g = Grant { id: "a".into(), hash: hash.into(), permissions: vec![Permission::DocRead] };
            assert!(!g.covers("a", hash, &[Permission::DocRead]), "{hash:?}");
            assert!(!g.covers("a", hash, &[]), "{hash:?}");
        }
    }
}
