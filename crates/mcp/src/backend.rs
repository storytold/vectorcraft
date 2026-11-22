//! Where MCP tool calls end up: a control-channel method call.

use std::net::{SocketAddr, ToSocketAddrs};

use craft::automation::args::{ControlOptions, EnvNames};
use craft::automation::client::ControlClient;
use serde_json::Value;

/// Something that answers control-channel methods (`engine.execute`, `document.inspect`,
/// `ui.pointer`, `ui.render`, `app.export`, …). See `vectorcraft_ui_egui::control` for the list.
pub trait Backend {
    /// Call one method. `Ok` carries the `result`, `Err` the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// True when a real UI is attached (`ui.*` methods like `ui.key` / `ui.inspect` work).
    fn has_ui(&self) -> bool;
    /// Short human description ("headless", "remote 127.0.0.1:7979").
    fn describe(&self) -> String;
}

/// The control token a running app was started with: `VECTORCRAFT_CONTROL_TOKEN`, else the
/// trimmed content of the file `VECTORCRAFT_CONTROL_TOKEN_FILE` names. One place reads it, for
/// MCP and the CLI alike.
pub fn control_token() -> Result<String, String> {
    let names = EnvNames::for_app("VECTORCRAFT");
    let options = ControlOptions::from_env(&names).map_err(|e| e.to_string())?;
    let missing = || format!("no control token: set {} (or {} to a file that holds it)", names.token, names.token_file);
    if let Some(token) = options.token {
        return Ok(token);
    }
    let file = options.token_file.ok_or_else(missing)?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("cannot read the control token file {}: {e}", file.display()))?;
    match text.trim() {
        "" => Err(missing()),
        token => Ok(token.to_owned()),
    }
}

/// A running VectorCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    sock: SocketAddr,
    token: String,
    client: Option<ControlClient>,
}

impl Remote {
    /// Connect to `addr` (e.g. `127.0.0.1:7979`) with the app's control `token`, failing fast when
    /// nothing is listening.
    pub fn connect(addr: &str, token: &str) -> std::io::Result<Self> {
        let sock =
            addr.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {addr}")))?;
        let client = ControlClient::connect(sock, token).map_err(std::io::Error::other)?;
        Ok(Self { addr: addr.to_string(), sock, token: token.to_string(), client: Some(client) })
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    #[cfg(test)]
    pub(crate) fn is_usable(&self) -> bool {
        self.client.as_ref().is_some_and(ControlClient::is_usable)
    }

    /// The open connection; a new one when the app closed the last (an idle connection, a restart)
    /// or it holds a line nobody asked for.
    fn open_client(&mut self, method: &str) -> Result<&mut ControlClient, String> {
        if !self.client.as_ref().is_some_and(ControlClient::is_usable) {
            self.client = None;
            let client = ControlClient::connect(self.sock, &self.token)
                .map_err(|e| format!("VectorCraft app at {} is not reachable: {e}; `{method}`: nothing was sent", self.addr))?;
            self.client = Some(client);
        }
        self.client.as_mut().ok_or_else(|| format!("VectorCraft app at {} is not reachable; `{method}`: nothing was sent", self.addr))
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let addr = self.addr.clone();
        let client = self.open_client(method)?;
        // The app can't stop what it runs, but a cancelled call stops waiting for it.
        let cancellable = vectorcraft_engine::cancel::is_cancellable();
        let result =
            if cancellable { client.call_until(method, params, &vectorcraft_engine::cancel::is_cancelled) } else { client.call(method, params) };
        match result {
            Ok(value) => Ok(value),
            // The connection survived: the app answered with this error.
            Err(e) if !client.is_broken() => Err(e),
            Err(_) if cancellable && vectorcraft_engine::cancel::is_cancelled() => Err(format!(
                "VectorCraft app at {addr}: `{method}` cancelled; the app may still finish what it was asked, which can't be stopped from here"
            )),
            Err(e) => Err(format!("VectorCraft app at {addr}: `{method}` failed: {e}; operation may have completed; inspect state before retrying")),
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("remote {}", self.addr)
    }
}
