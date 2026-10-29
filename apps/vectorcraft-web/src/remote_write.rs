//! Saving to a writable remote mount: the requests of one save and what their answers mean.
//!
//! A save never overwrites the target in place. The bytes are uploaded to `<name>.<token>.part`
//! beside it (a token of its own for every upload, so two saves of one path never share a part
//! file) and then moved over the target, so a failed upload leaves the old file whole. The target is
//! only replaced when it is still what this session last saw: the ETag from when the project was
//! opened (or last saved) is compared with the server's before the move, so a change by someone
//! else, such as the headless MCP editing the same project, is never overwritten silently.
//!
//! The check is a `HEAD` before the `MOVE`, not an `If-Match` header on the `MOVE`: a conditional
//! header on a `MOVE` is evaluated against the source (the `.part` file), where it can never match.
//! A change in the moments between the `HEAD` and the `MOVE` is not caught (WebDAV has no atomic
//! compare-and-move). A move that overwrites nothing sends `Overwrite: F`, so the server refuses it
//! (`412`) when the target appeared meanwhile.
//!
//! Pure (no browser types), so it runs natively in `tests/it`.

use crate::remote_config::Config;

/// Longest upload accepted (bytes): a project file, not media.
pub const MAX_UPLOAD: usize = 64 << 20;

/// One request to send; the body (an upload) is the caller's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Req {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
}

/// What this session knows about the target's version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Known {
    /// The ETag seen when the file was opened or last saved.
    Etag(String),
    /// The file was opened from a server that sent no ETag: a change cannot be detected.
    NoEtag,
    /// Not opened or saved here: the target should not exist yet.
    Unknown,
}

/// What to do after the `HEAD` of the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Precondition {
    /// Move the upload over the target (`overwrite`: the move replaces an existing file).
    Proceed {
        overwrite: bool,
    },
    /// The target changed on the server since this session saw it.
    Conflict,
    /// The target exists but this session never opened or saved it (a Save As onto a file).
    Exists,
    Failed(String),
}

/// How a save ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Saved,
    /// The server refused to replace a file that changed: the project stays unsaved.
    Conflict,
    /// The target exists and this session never opened or saved it: nothing was overwritten.
    Exists,
    Failed(String),
}

/// A token that is safe in a file name: 1 to 64 ASCII letters or digits.
pub fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 64 && token.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `/data/a/x.vectorcraft` + `7f3a` → `/data/a/x.vectorcraft.7f3a.part`, in the same folder.
pub fn part_path(path: &str, token: &str) -> Result<String, String> {
    if path.ends_with('/') || path.rsplit('/').next().is_none_or(str::is_empty) {
        return Err(format!("{path}: not a file path"));
    }
    if !valid_token(token) {
        return Err("invalid upload token".into());
    }
    Ok(format!("{path}.{token}.part"))
}

/// Only a mount configured `"writable": true` takes writes.
pub fn check_writable(cfg: &Config, path: &str) -> Result<(), String> {
    let m = cfg.mount_for(path).ok_or_else(|| format!("{path}: not on a remote mount"))?;
    if m.writable { Ok(()) } else { Err(format!("{} is read-only: use Save As to save somewhere else", m.name)) }
}

/// `PUT` of the upload to the `.part` file of upload `token`.
pub fn put_part(cfg: &Config, path: &str, token: &str) -> Result<Req, String> {
    check_writable(cfg, path)?;
    Ok(Req { method: "PUT", url: cfg.url_for(&part_path(path, token)?)?, headers: vec![("Content-Type", "application/octet-stream".into())] })
}

/// `HEAD` of the target: its current ETag.
pub fn head_target(cfg: &Config, path: &str) -> Result<Req, String> {
    Ok(Req { method: "HEAD", url: cfg.url_for(path)?, headers: Vec::new() })
}

/// `MOVE` of upload `token`'s `.part` file over the target. `Destination` is an absolute URL on `origin`.
pub fn move_part(cfg: &Config, origin: &str, path: &str, token: &str, overwrite: bool) -> Result<Req, String> {
    check_writable(cfg, path)?;
    let dest = format!("{origin}{}", cfg.url_for(path)?);
    Ok(Req {
        method: "MOVE",
        url: cfg.url_for(&part_path(path, token)?)?,
        headers: vec![("Destination", dest), ("Overwrite", if overwrite { "T" } else { "F" }.into())],
    })
}

/// `DELETE` of upload `token`'s `.part` file (best effort after a failed save).
pub fn delete_part(cfg: &Config, path: &str, token: &str) -> Result<Req, String> {
    check_writable(cfg, path)?;
    Ok(Req { method: "DELETE", url: cfg.url_for(&part_path(path, token)?)?, headers: Vec::new() })
}

/// Strong comparison: both ETags are strong (no `W/` prefix) and equal, ignoring surrounding
/// blanks. A weak ETag can stay the same while the bytes change, so it never proves the target is
/// still what this session saw.
fn same_etag(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    !a.starts_with("W/") && !b.starts_with("W/") && a == b
}

/// `HEAD` of upload `token`'s `.part` file: the ETag its bytes have before the move.
pub fn head_part(cfg: &Config, path: &str, token: &str) -> Result<Req, String> {
    Ok(Req { method: "HEAD", url: cfg.url_for(&part_path(path, token)?)?, headers: Vec::new() })
}

/// The version of the target a finished save leaves in this session's record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavedVersion {
    /// The ETag of what this save wrote.
    Etag(String),
    /// The server sends no ETags at all: a change cannot be detected (as for a file opened so).
    NoEtag,
    /// The server's ETag cannot be tied to this save (someone may have written in between): the
    /// record is dropped, so the next save is a conflict or Save As, never a silent overwrite.
    Unbound,
    /// The target has an ETag that differs from the upload's (some servers, such as rclone's WebDAV,
    /// derive it from the modification time and so give the moved file a new one): only its bytes can
    /// tell whether it is this save's. See [`verify_saved`].
    Verify,
}

/// Which version a save made. `moved` is the `MOVE` answer's ETag, `part` the upload's ETag
/// before the move (its `PUT` answer or a `HEAD`), `head` the target's `HEAD` after the move (`None`:
/// that request failed; `Some(None)`: it answered without an ETag). Only a strong ETag counts: the
/// `MOVE`'s own, or the target's when it equals the part's (the move kept the bytes, so a writer in
/// between would have changed it). A target ETag that differs (or a part ETag that is unknown or weak)
/// is [`SavedVersion::Verify`]: the caller compares the target's bytes with the upload. A failed `HEAD`
/// shows nothing about the target: not tied.
pub fn saved_version(moved: Option<&str>, part: Option<&str>, head: Option<Option<&str>>) -> SavedVersion {
    let strong = |e: Option<&str>| e.map(str::trim).filter(|e| !e.is_empty() && !e.starts_with("W/")).map(str::to_string);
    if let Some(m) = strong(moved) {
        return SavedVersion::Etag(m);
    }
    let Some(head) = head else { return SavedVersion::Unbound };
    match (head.map(str::trim).filter(|h| !h.is_empty()), strong(part)) {
        (None, _) => SavedVersion::NoEtag,
        (Some(h), Some(p)) if p == h => SavedVersion::Etag(p),
        _ => SavedVersion::Verify,
    }
}

/// `GET` of the whole target, to compare its bytes with an upload ([`verify_saved`]).
pub fn get_target(cfg: &Config, path: &str) -> Result<Req, String> {
    Ok(Req { method: "GET", url: cfg.url_for(path)?, headers: Vec::new() })
}

/// The version a save made when the target's ETag differs from the upload's. `got` is the target's
/// `GET` answer: its status, ETag (from the same response as the bytes) and bytes (`None`: the request
/// failed or the body was too large). The ETag is bound only when the target holds exactly the bytes
/// uploaded: another writer's file (even of the same size) is never bound, so the next save stays a
/// conflict instead of overwriting it. A weak ETag is fine here: the bytes were just checked.
pub fn verify_saved(uploaded: &[u8], got: Option<(u16, Option<&str>, &[u8])>) -> SavedVersion {
    match got {
        Some((200, etag, body)) if body == uploaded => match etag.map(str::trim).filter(|e| !e.is_empty()) {
            Some(e) => SavedVersion::Etag(e.to_string()),
            None => SavedVersion::NoEtag,
        },
        _ => SavedVersion::Unbound,
    }
}

/// Decide from the target's `HEAD` answer whether the move may go ahead.
pub fn precondition(known: &Known, status: u16, etag: Option<&str>) -> Precondition {
    match (known, status) {
        // an answer without an ETag does not show the target is still what this session saw
        (Known::Etag(was), 200) => match etag {
            Some(now) if same_etag(was, now) => Precondition::Proceed { overwrite: true },
            _ => Precondition::Conflict,
        },
        (Known::NoEtag, 200) => Precondition::Proceed { overwrite: true },
        // never opened or saved here: it must not exist yet, so it is not replaced
        (Known::Unknown, 200) => Precondition::Exists,
        // gone meanwhile (or never there): it is created again
        (_, 404) => Precondition::Proceed { overwrite: false },
        (_, s) => Precondition::Failed(format!("the server answered {s} when checking the file")),
    }
}

/// The answer to the upload.
pub fn classify_put(status: u16) -> Result<(), String> {
    if matches!(status, 200 | 201 | 204) { Ok(()) } else { Err(format!("the server answered {status} to the upload")) }
}

/// The answer to the move: `412` is a conflict, never retried or forced.
pub fn classify_move(status: u16) -> Outcome {
    match status {
        201 | 204 => Outcome::Saved,
        412 => Outcome::Conflict,
        s => Outcome::Failed(format!("the server answered {s} to the move")),
    }
}

/// The status text for an outcome that left the project unsaved.
pub fn failure_message(o: &Outcome) -> Option<String> {
    match o {
        Outcome::Saved => None,
        Outcome::Conflict => Some("Project changed on the server; use Save As".into()),
        Outcome::Exists => Some("A file with that name already exists on the server; open it first or save under another name".into()),
        Outcome::Failed(e) => Some(format!("Could not save to the server: {e}")),
    }
}
