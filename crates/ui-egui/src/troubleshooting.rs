//! Help › Copy Troubleshooting Info (`help.copyTroubleshootingInfo`): puts the text the host
//! gives ([`crate::Services::troubleshooting`]) on the clipboard for a bug report. The desktop app
//! gives the platform, the graphics processor, recent commands and frame times, and the hang reports
//! of this session and the one before (its `hang_watch` module); without the service the text is
//! the version only.

use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The status message after the text was put on the clipboard.
pub const COPIED: &str = "Copied the troubleshooting info";

/// The status message at launch when the last session stopped responding and was closed before it
/// answered again.
pub const LAST_SESSION_STOPPED: &str =
    "The last session of VectorCraft stopped responding before it closed. Help › Copy Troubleshooting Info copies the details for a bug report";

/// The text the command copies.
pub fn text(app: &VectorcraftApp) -> String {
    match &app.services.troubleshooting {
        Some(text) => text(),
        None => format!("VectorCraft {}\n", env!("CARGO_PKG_VERSION")),
    }
}

/// `help.copyTroubleshootingInfo`: put [`text`] on the clipboard. → `{chars}`, the text's length in
/// characters.
pub fn copy(app: &mut VectorcraftApp) -> Result<Value, String> {
    let text = text(app);
    let chars = text.chars().count();
    app.clipboard_out = Some(text);
    app.status(COPIED);
    Ok(json!({ "chars": chars }))
}

/// Tell the user, at launch, that the last session stopped responding and was closed.
pub fn last_session_stopped(app: &mut VectorcraftApp) {
    app.status(LAST_SESSION_STOPPED);
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;

    #[test]
    fn the_command_copies_the_hosts_text_and_says_so() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.troubleshooting = Some(Box::new(|| "platform: test\n".to_string()));
        let r = app.run("help.copyTroubleshootingInfo", json!({})).unwrap();
        assert_eq!(r["chars"], 15);
        assert_eq!(app.clipboard_out.as_deref(), Some("platform: test\n"));
        assert_eq!(app.ui.status, COPIED);
    }

    #[test]
    fn without_the_service_the_version_is_copied() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("help.copyTroubleshootingInfo", json!({})).unwrap();
        assert_eq!(app.clipboard_out, Some(format!("VectorCraft {}\n", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn running_a_command_marks_it_for_the_hang_report() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.beacon = Box::leak(Box::new(crate::hang_beacon::Beacon::new()));
        app.run("help.copyTroubleshootingInfo", json!({})).unwrap();
        assert_eq!(app.beacon.recent_commands(), Some(vec!["help.copyTroubleshootingInfo".to_string()]));
    }
}
