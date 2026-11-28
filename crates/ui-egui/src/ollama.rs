//! Shared Ollama transport for desktop AI dialogs.
//!
//! Plain `http://` is accepted only for a loopback host. Every other server must be `https://`,
//! and `curl` is then limited to HTTPS so a remote preview cannot leave in cleartext.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use serde_json::{Value, json};

pub(crate) const DEFAULT_ENDPOINT: &str = "http://localhost:11434/api/chat";
pub(crate) const DEFAULT_MODEL: &str = "qwen2.5vl:3b";
pub(crate) const DEFAULT_LANGUAGE: &str = "English";

/// Server, vision model and name language. Missing keys use the shared defaults.
pub(crate) fn connection(settings: &Value) -> (String, String, String) {
    let pick = |key: &str, fallback: &str| settings.get(key).and_then(Value::as_str).unwrap_or(fallback).to_string();
    (pick("endpoint", DEFAULT_ENDPOINT), pick("model", DEFAULT_MODEL), pick("language", DEFAULT_LANGUAGE))
}

/// Remember the connection for every Ollama dialog.
pub(crate) fn store_connection(settings: &mut Value, endpoint: &str, model: &str, language: &str) {
    *settings = json!({"endpoint": endpoint, "model": model, "language": language});
}

#[derive(Debug)]
pub(crate) enum EndpointKind {
    Loopback,
    RemoteHttps,
}

/// Accept loopback `http://` or `https://`, and require `https://` for every other host.
pub(crate) fn endpoint_kind(endpoint: &str) -> Result<EndpointKind, String> {
    if endpoint.chars().any(char::is_control) {
        return Err("Server URL must start with http:// or https://".into());
    }
    let (https, rest) = if let Some(rest) = endpoint.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err("Server URL must start with http:// or https://".into());
    };
    let Some(host) = host_of(rest) else {
        return Err("Server URL must start with http:// or https://".into());
    };
    if is_loopback(&host) {
        Ok(EndpointKind::Loopback)
    } else if https {
        Ok(EndpointKind::RemoteHttps)
    } else {
        Err("Plain http:// is only allowed for localhost; use https:// for a remote server".into())
    }
}

/// curl supplies OS TLS support without coupling the engine or wasm build to networking.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn post(endpoint: &str, request: &Value) -> Result<Value, String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let args = curl_args(endpoint)?;
    let mut child = Command::new("curl")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot run curl (required for Ollama): {e}"))?;
    let data = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "No request pipe".to_string())
        .and_then(|mut stdin| stdin.write_all(&data).map_err(|e| format!("Cannot send request: {e}")));
    let mut bytes = Vec::new();
    let read_result = child
        .stdout
        .take()
        .ok_or_else(|| "No response pipe".to_string())
        .and_then(|stdout| stdout.take(1_048_577).read_to_end(&mut bytes).map_err(|e| e.to_string()));
    if bytes.len() > 1_048_576 || write_result.is_err() || read_result.is_err() {
        // Close both pipes and reap the child on errors or oversized responses.
        let _ = child.kill();
        let _ = child.wait();
        return Err("Ollama request failed or response exceeded 1 MiB".into());
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Ollama request failed. Check the server URL, installed vision model and curl; request timeout is 120 seconds.".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid Ollama response: {e}"))
}

/// Arguments for one Ollama POST. Loopback skips any configured proxy so the preview stays on this machine.
#[cfg(not(target_arch = "wasm32"))]
fn curl_args(endpoint: &str) -> Result<Vec<String>, String> {
    let kind = endpoint_kind(endpoint)?;
    let proto = match kind {
        EndpointKind::Loopback => "=http,https",
        EndpointKind::RemoteHttps => "=https",
    };
    let mut args = vec![
        "--disable".into(),
        "--silent".into(),
        "--show-error".into(),
        "--fail".into(),
        "--max-time".into(),
        "120".into(),
        "--connect-timeout".into(),
        "10".into(),
    ];
    if matches!(kind, EndpointKind::Loopback) {
        args.extend(["--noproxy".into(), "*".into()]);
    }
    args.extend([
        "--proto".into(),
        proto.into(),
        "--header".into(),
        "Content-Type: application/json".into(),
        "--data-binary".into(),
        "@-".into(),
        "--url".into(),
        endpoint.into(),
    ]);
    Ok(args)
}

/// Host of an http(s) URL, rejecting userinfo tricks and a non-numeric port.
fn host_of(rest: &str) -> Option<String> {
    if rest.is_empty() {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = if let Some(v6) = hostport.strip_prefix('[') {
        let (host, tail) = v6.split_once(']')?;
        if !tail.is_empty() {
            let port = tail.strip_prefix(':')?;
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
        }
        host
    } else {
        if hostport.contains('[') || hostport.contains(']') {
            return None;
        }
        match hostport.split_once(':') {
            Some((host, port)) => {
                if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                host
            }
            None => hostport,
        }
    };
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':') {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Whether the host is localhost or an IPv4 or IPv6 loopback address.
fn is_loopback(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return ip.is_loopback();
    }
    if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        return ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// One connection supplies the model.
    fn one_connection_supplies_the_model() {
        let (endpoint, model, language) = connection(&serde_json::Value::Null);
        assert_eq!((endpoint.as_str(), model.as_str(), language.as_str()), (DEFAULT_ENDPOINT, DEFAULT_MODEL, DEFAULT_LANGUAGE));
        let mut saved = serde_json::Value::Null;
        store_connection(&mut saved, DEFAULT_ENDPOINT, "other:7b", "French");
        assert_eq!(connection(&saved).1, "other:7b");
    }

    #[test]
    /// Plain http is loopback only.
    fn plain_http_is_loopback_only() {
        assert!(matches!(endpoint_kind("http://localhost:11434/api/chat"), Ok(EndpointKind::Loopback)));
        assert!(matches!(endpoint_kind("http://127.0.0.1:9/api/chat"), Ok(EndpointKind::Loopback)));
        assert!(matches!(endpoint_kind("http://[::1]:11434/api/chat"), Ok(EndpointKind::Loopback)));
        assert!(matches!(endpoint_kind("http://[::ffff:127.0.0.1]/api/chat"), Ok(EndpointKind::Loopback)));
        assert!(matches!(endpoint_kind("https://models.example/api/chat"), Ok(EndpointKind::RemoteHttps)));
        assert_eq!(
            endpoint_kind("http://models.example/api/chat").unwrap_err(),
            "Plain http:// is only allowed for localhost; use https:// for a remote server"
        );
        assert_eq!(
            endpoint_kind("http://10.0.0.5/api/chat").unwrap_err(),
            "Plain http:// is only allowed for localhost; use https:// for a remote server"
        );
        assert_eq!(
            endpoint_kind("http://127.0.0.1@models.example/api/chat").unwrap_err(),
            "Plain http:// is only allowed for localhost; use https:// for a remote server"
        );
        assert_eq!(endpoint_kind("ftp://localhost/api/chat").unwrap_err(), "Server URL must start with http:// or https://");
        assert_eq!(endpoint_kind("http://localhost\n/api/chat").unwrap_err(), "Server URL must start with http:// or https://");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    /// Loopback bypasses the proxy.
    fn loopback_bypasses_the_proxy() {
        let local = curl_args("http://127.0.0.1:11434/api/chat").unwrap();
        let remote = curl_args("https://models.example/api/chat").unwrap();
        assert!(local.windows(2).any(|pair| pair[0] == "--noproxy" && pair[1] == "*"));
        assert!(remote.windows(2).all(|pair| pair[0] != "--noproxy"));
        assert!(local.windows(2).any(|pair| pair[0] == "--proto" && pair[1] == "=http,https"));
        assert!(remote.windows(2).any(|pair| pair[0] == "--proto" && pair[1] == "=https"));
    }
}
