//! The control channel's key (#3603): a per-launch 256-bit token the desktop app's loopback
//! control port requires as the first request of every connection, so a process that can reach
//! the port (another user on the machine, a sandboxed app, a web page) can't drive the app
//! without also being able to read the user's files.
//!
//! The app generates the token at launch and writes it to a file only its user can read (or takes
//! it from [`TOKEN_ENV`]); clients such as `vectorcraft-cli mcp` read it from the same file. The
//! first line of a connection is `{"method": "auth", "params": {"token": "<64 hex digits>"}}`.
//! See `docs/control-protocol.md`.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The method of the first request on every control connection.
pub const AUTH_METHOD: &str = "auth";
/// Supplies the token instead of a generated one (the app) or the token file (a client).
pub const TOKEN_ENV: &str = "VECTORCRAFT_CONTROL_TOKEN";
/// Where the app writes the token it generates, and where a client reads it, instead of the
/// default file next to the settings.
pub const TOKEN_FILE_ENV: &str = "VECTORCRAFT_CONTROL_TOKEN_FILE";
/// Random bytes in a token.
pub const TOKEN_BYTES: usize = 32;
/// Characters in a token: lowercase hexadecimal.
pub const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;
/// Longest token file read (a token and a newline, with room for stray whitespace).
const MAX_TOKEN_FILE: u64 = 4096;

/// The token for `bytes`, as lowercase hexadecimal.
pub fn token_from_bytes(bytes: &[u8; TOKEN_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|b| [HEX[usize::from(b >> 4)] as char, HEX[usize::from(b & 0x0f)] as char]).collect()
}

/// Accept only a token of [`TOKEN_HEX_LEN`] hexadecimal digits. Returns it lowercased.
pub fn validate_token(token: &str) -> Result<String, String> {
    let token = token.trim();
    if token.len() != TOKEN_HEX_LEN || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("a control token is {TOKEN_HEX_LEN} hexadecimal digits"));
    }
    Ok(token.to_ascii_lowercase())
}

/// Whether `supplied` is `expected`, compared without stopping at the first different byte, so the
/// time taken says nothing about how much of a guess was right. Hex letters match in either case.
pub fn token_matches(expected: &str, supplied: &str) -> bool {
    if expected.len() != TOKEN_HEX_LEN || supplied.len() != TOKEN_HEX_LEN {
        return false;
    }
    expected.bytes().zip(supplied.bytes()).fold(0u8, |diff, (a, b)| diff | (a.to_ascii_lowercase() ^ b.to_ascii_lowercase())) == 0
}

/// The first request a client sends on a connection.
pub fn auth_request(token: &str) -> Value {
    json!({"id": "auth", "method": AUTH_METHOD, "params": {"token": token}})
}

/// Whether the request `method` with `params` is an `auth` request carrying `expected`.
pub fn is_valid_auth(method: &str, params: &Value, expected: &str) -> bool {
    method == AUTH_METHOD && params.get("token").and_then(Value::as_str).is_some_and(|t| token_matches(expected, t))
}

/// Write `token` to `path` so only the current user can read it: a new file of mode 0600 (on
/// Windows the per-user settings folder is the user's alone) moved into place, so a reader never
/// sees half a token and a symbolic link at `path` is replaced, not followed.
pub fn write_token_file(path: &Path, token: &str) -> Result<(), String> {
    let shown = path.display();
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| format!("{shown}: no folder to keep the control token in"))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let name = path.file_name().ok_or_else(|| format!("{shown}: not a file name"))?;
    let mut temp_name = OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = parent.join(temp_name);
    // A leftover from a crashed start with the same process id.
    let _ = std::fs::remove_file(&temp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&temp).and_then(|mut f| {
        writeln!(f, "{token}")?;
        f.sync_all()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&temp, path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{shown}: {e}"));
    }
    Ok(())
}

/// The token in the file at `path`.
pub fn read_token_file(path: &Path) -> Result<String, String> {
    let shown = path.display();
    let file = std::fs::File::open(path).map_err(|e| format!("{shown}: {e}"))?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, MAX_TOKEN_FILE), &mut text).map_err(|e| format!("{shown}: {e}"))?;
    validate_token(&text).map_err(|e| format!("{shown}: {e}"))
}

/// The token a client sends: `env_token` ([`TOKEN_ENV`]) when set, else the one in the file
/// `env_file` ([`TOKEN_FILE_ENV`]) names, else the one in `default_file`, the file the app writes
/// next to its settings for the port. Empty variables count as unset.
pub fn client_token(env_token: Option<OsString>, env_file: Option<OsString>, default_file: Option<PathBuf>) -> Result<String, String> {
    if let Some(token) = env_token.filter(|v| !v.is_empty()) {
        let token = token.into_string().map_err(|_| format!("{TOKEN_ENV} is not text"))?;
        return validate_token(&token).map_err(|e| format!("{TOKEN_ENV}: {e}"));
    }
    let path = env_file
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or(default_file)
        .ok_or_else(|| format!("no control token: set {TOKEN_ENV} or {TOKEN_FILE_ENV} (the app writes its token next to its settings)"))?;
    read_token_file(&path).map_err(|e| format!("no control token ({e}); is the app running with --control on this port?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    #[test]
    fn tokens_are_64_hex_digits_compared_whole() {
        assert_eq!(token_from_bytes(&[0xab; TOKEN_BYTES]), "ab".repeat(TOKEN_BYTES));
        assert_eq!(validate_token(&format!("{}\n", T.to_uppercase())).unwrap(), T);
        for bad in ["", "abc", &T[1..], &format!("{T}0"), &format!("{}g", &T[1..])] {
            assert!(validate_token(bad).is_err(), "{bad}");
        }
        assert!(token_matches(T, T));
        assert!(token_matches(T, &T.to_uppercase()));
        assert!(!token_matches(T, &format!("{}0", &T[1..])));
        assert!(!token_matches(T, ""));
        assert!(!token_matches("", ""), "an unset token matches nothing");
    }

    #[test]
    fn only_an_auth_request_with_the_token_authenticates() {
        let req = auth_request(T);
        assert!(is_valid_auth(req["method"].as_str().unwrap(), &req["params"], T));
        assert!(!is_valid_auth("auth", &json!({"token": "0".repeat(64)}), T));
        assert!(!is_valid_auth("auth", &json!({}), T));
        assert!(!is_valid_auth("auth", &json!({"token": 5}), T));
        assert!(!is_valid_auth("ui.inspect", &json!({"token": T}), T));
    }

    #[test]
    fn a_token_file_is_private_and_read_back() {
        let dir = vectorcraft_testkit::temp_dir("control-token");
        let path = dir.join("sub").join("control-7979.token");
        write_token_file(&path, T).unwrap();
        assert_eq!(read_token_file(&path).unwrap(), T);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A new launch replaces it.
        let other = "f".repeat(64);
        write_token_file(&path, &other).unwrap();
        assert_eq!(read_token_file(&path).unwrap(), other);
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1, "no temporary file left behind");
    }

    #[test]
    fn a_client_takes_the_variable_then_the_named_file_then_the_default_file() {
        let dir = vectorcraft_testkit::temp_dir("control-client-token");
        let (named, default) = (dir.join("named.token"), dir.join("default.token"));
        write_token_file(&named, T).unwrap();
        write_token_file(&default, &"e".repeat(64)).unwrap();
        let other = "d".repeat(64);
        assert_eq!(client_token(Some(other.clone().into()), Some(named.clone().into()), Some(default.clone())).unwrap(), other);
        assert_eq!(client_token(Some("".into()), Some(named.clone().into()), Some(default.clone())).unwrap(), T);
        assert_eq!(client_token(None, Some("".into()), Some(default.clone())).unwrap(), "e".repeat(64));
        assert!(client_token(Some("short".into()), None, Some(default)).unwrap_err().contains(TOKEN_ENV));
        let missing = client_token(None, None, Some(dir.join("none.token"))).unwrap_err();
        assert!(missing.contains("--control"), "{missing}");
        assert!(client_token(None, None, None).is_err());
        std::fs::write(&named, "not a token").unwrap();
        assert!(client_token(None, Some(named.into()), None).is_err());
    }
}
