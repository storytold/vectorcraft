//! Linux and the BSDs: the answer to an XDG desktop portal request
//! (<https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html>),
//! for the save dialog (`portal_save`) and, on Linux, the screen color sampler
//! (`linux_screen_color`). The wait ends with the request's `Response`, or with an error when the
//! portal leaves the bus first.

use futures_lite::{StreamExt as _, future};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedObjectPath;

const DESKTOP: &str = "org.freedesktop.portal.Desktop";

/// A request token: `prefix`, then 128 bits from the operating system's random device, so the
/// token is unique and can't be guessed, as the Request documentation asks.
#[cfg(target_os = "linux")]
pub(crate) fn token(prefix: &str) -> Result<String, String> {
    use std::io::Read as _;

    let mut bytes = [0; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| format!("could not create a portal request token: {e}"))?;
    let random: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{prefix}_{random}"))
}

/// Subscriptions made before a portal method call, on every request path: a portal older than
/// version 0.9 ignores `handle_token` and answers on the request path it returns, and its answer
/// can come before the call's reply. Each stream keeps every signal from the time it is made.
pub(crate) struct Responses {
    replies: MessageIterator,
    owners: MessageIterator,
}

impl Responses {
    pub(crate) fn new(conn: &Connection) -> Result<Self, String> {
        let reply_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(DESKTOP)
            .and_then(|b| b.interface("org.freedesktop.portal.Request"))
            .and_then(|b| b.member("Response"))
            .map_err(|e| e.to_string())?
            .build();
        let owner_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .and_then(|b| b.path("/org/freedesktop/DBus"))
            .and_then(|b| b.interface("org.freedesktop.DBus"))
            .and_then(|b| b.member("NameOwnerChanged"))
            .and_then(|b| b.arg(0, DESKTOP))
            .map_err(|e| e.to_string())?
            .build();
        Ok(Self {
            replies: MessageIterator::for_match_rule(reply_rule, conn, None).map_err(|e| e.to_string())?,
            owners: MessageIterator::for_match_rule(owner_rule, conn, None).map_err(|e| e.to_string())?,
        })
    }

    /// The `Response` on the request path `call` returned, from the connection that returned it.
    /// An error when that connection leaves the bus first, or the bus connection closes. A
    /// `Response` the portal sent before it left is still the answer.
    pub(crate) fn wait(self, call: &zbus::Message) -> Result<zbus::Message, String> {
        enum Event {
            Reply(Option<zbus::Result<zbus::Message>>),
            Owner(Option<zbus::Result<zbus::Message>>),
        }
        let handle: OwnedObjectPath = call.body().deserialize().map_err(|e| e.to_string())?;
        let portal = call.header().sender().map(|s| s.as_str().to_string()).ok_or("the portal answered without a sender")?;
        let answers = |reply: &zbus::Message| {
            let header = reply.header();
            header.path().map(|p| p.as_str()) == Some(handle.as_str()) && header.sender().map(|s| s.as_str()) == Some(portal.as_str())
        };
        let mut replies = self.replies.into_inner();
        let mut owners = self.owners.into_inner();
        future::block_on(async {
            loop {
                let event = future::or(async { Event::Owner(owners.next().await) }, async { Event::Reply(replies.next().await) }).await;
                match event {
                    Event::Reply(reply) => {
                        let reply = reply.ok_or("the portal closed without an answer")?.map_err(|e| e.to_string())?;
                        if answers(&reply) {
                            return Ok(reply);
                        }
                    }
                    Event::Owner(message) => {
                        let message = message.ok_or("the portal connection closed")?.map_err(|e| e.to_string())?;
                        if owner_disappeared(&message, &portal)? {
                            // zbus puts each message in every matching stream before it reads the
                            // next one, so a `Response` that came before the bus's report is
                            // already queued: it is still the answer.
                            while let Some(Some(reply)) = future::poll_once(replies.next()).await {
                                if let Ok(reply) = reply
                                    && answers(&reply)
                                {
                                    return Ok(reply);
                                }
                            }
                            return Err("the desktop portal disappeared while waiting for its answer".into());
                        }
                    }
                }
            }
        })
    }
}

/// Whether `message` is the bus's `NameOwnerChanged` saying that the connection `portal` no longer
/// owns the portal's name. The bus itself must send it: another connection can't end the wait.
fn owner_disappeared(message: &zbus::Message, portal: &str) -> Result<bool, String> {
    if message.header().sender().map(|s| s.as_str()) != Some("org.freedesktop.DBus") {
        return Ok(false);
    }
    let (name, old, new): (String, String, String) = message.body().deserialize().map_err(|e| e.to_string())?;
    Ok(name == DESKTOP && old == portal && new != portal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_bus_can_report_portal_owner_loss() {
        let changed = |sender, name, old, new| {
            zbus::Message::signal("/org/freedesktop/DBus", "org.freedesktop.DBus", "NameOwnerChanged")
                .unwrap()
                .sender(sender)
                .unwrap()
                .build(&(name, old, new))
                .unwrap()
        };
        let forged = changed(":1.42", DESKTOP, ":1.7", "");
        assert!(!owner_disappeared(&forged, ":1.7").unwrap());
        let real = changed("org.freedesktop.DBus", DESKTOP, ":1.7", "");
        assert!(owner_disappeared(&real, ":1.7").unwrap());
        let other = changed("org.freedesktop.DBus", "org.example.Other", ":1.7", "");
        assert!(!owner_disappeared(&other, ":1.7").unwrap());
        let old = changed("org.freedesktop.DBus", DESKTOP, ":1.8", "");
        assert!(!owner_disappeared(&old, ":1.7").unwrap());
        let acquired = changed("org.freedesktop.DBus", DESKTOP, "", ":1.7");
        assert!(!owner_disappeared(&acquired, ":1.7").unwrap());
    }

    /// Through a real D-Bus: when the bus has reported the portal gone, a `Response` the portal sent
    /// before it left is still the answer; without one the wait ends with an error.
    #[test]
    fn a_queued_answer_wins_over_the_portal_leaving() {
        use std::collections::HashMap;

        use zbus::blocking::connection::Builder;
        use zbus::zvariant::OwnedValue;

        for answered in [true, false] {
            let Some((_daemon, address)) = test_bus::private_bus() else { return };
            let address = address.as_str();
            let portal = Builder::address(address).unwrap().name(DESKTOP).unwrap().build().unwrap();
            let unique = portal.unique_name().unwrap().to_string();
            let client = Builder::address(address).unwrap().build().unwrap();
            let responses = Responses::new(&client).unwrap();
            // zbus puts each message in every matching stream before it reads the next one
            // (`src/connection/socket_reader.rs`), so once this stream has the bus's report,
            // `responses` holds the `Response` sent before it.
            let gone_rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.freedesktop.DBus")
                .unwrap()
                .member("NameOwnerChanged")
                .unwrap()
                .arg(0, DESKTOP)
                .unwrap()
                .build();
            let gone = MessageIterator::for_match_rule(gone_rule, &client, None).unwrap();
            let path = "/org/freedesktop/portal/desktop/request/test/t1";
            if answered {
                let results = HashMap::<String, OwnedValue>::new();
                portal.emit_signal(None::<&str>, path, "org.freedesktop.portal.Request", "Response", &(0_u32, results)).unwrap();
            }
            portal.close().unwrap();
            // On a thread of its own, so a report that never comes fails the test instead of
            // hanging it.
            let (seen, report) = std::sync::mpsc::channel();
            let left = unique.clone();
            std::thread::spawn(move || {
                let mut gone = gone;
                let _ = seen.send(gone.any(|m| m.is_ok_and(|m| owner_disappeared(&m, &left).unwrap_or(false))));
            });
            assert_eq!(report.recv_timeout(std::time::Duration::from_secs(30)), Ok(true), "the bus did not report the portal gone");
            // The call's reply as `wait` reads it: the request path, from the portal's connection.
            let call = zbus::Message::signal("/", "org.example.Test", "Reply")
                .unwrap()
                .sender(unique.as_str())
                .unwrap()
                .build(&OwnedObjectPath::try_from(path).unwrap())
                .unwrap();
            let waited = responses.wait(&call);
            assert_eq!(waited.is_ok(), answered, "{waited:?}");
        }
    }
}

/// The private session bus that the D-Bus tests of the portal requests run on.
#[cfg(test)]
pub(crate) mod test_bus {
    /// A session bus of its own (`dbus-daemon`), `None` when there is none to run.
    pub(crate) fn private_bus() -> Option<(Daemon, String)> {
        use std::io::BufRead as _;
        let spawning = crate::spawning();
        let daemon = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn();
        drop(spawning);
        let Ok(daemon) = daemon else {
            eprintln!("no dbus-daemon: skipped");
            return None;
        };
        let mut daemon = Daemon(daemon);
        let mut address = String::new();
        std::io::BufReader::new(daemon.0.stdout.take().unwrap()).read_line(&mut address).unwrap();
        Some((daemon, address.trim().to_string()))
    }

    /// Stops the private bus when the test ends, unwinding included.
    pub(crate) struct Daemon(std::process::Child);

    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
