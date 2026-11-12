//! The answer to the color sampler's XDG desktop portal request
//! (<https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html>).
//! The wait ends with the request's `Response`, or with an error when the portal leaves the bus
//! first.

use std::io::Read as _;

use futures_lite::{StreamExt as _, future};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedObjectPath;

const DESKTOP: &str = "org.freedesktop.portal.Desktop";

/// A request token: `prefix`, then 128 bits from the operating system's random device, so the
/// token is unique and can't be guessed, as the Request documentation asks.
pub(super) fn token(prefix: &str) -> Result<String, String> {
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
pub(super) struct Responses {
    replies: MessageIterator,
    owners: MessageIterator,
}

impl Responses {
    pub(super) fn new(conn: &Connection) -> Result<Self, String> {
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
    /// An error when that connection leaves the bus first, or the bus connection closes.
    pub(super) fn wait(self, call: &zbus::Message) -> Result<zbus::Message, String> {
        enum Event {
            Reply(Option<zbus::Result<zbus::Message>>),
            Owner(Option<zbus::Result<zbus::Message>>),
        }
        let handle: OwnedObjectPath = call.body().deserialize().map_err(|e| e.to_string())?;
        let portal = call.header().sender().map(|s| s.as_str().to_string()).ok_or("the portal answered without a sender")?;
        let mut replies = self.replies.into_inner();
        let mut owners = self.owners.into_inner();
        future::block_on(async {
            loop {
                let event = future::or(async { Event::Reply(replies.next().await) }, async { Event::Owner(owners.next().await) }).await;
                match event {
                    Event::Reply(reply) => {
                        let reply = reply.ok_or("the portal closed without an answer")?.map_err(|e| e.to_string())?;
                        let header = reply.header();
                        if header.path().map(|p| p.as_str()) == Some(handle.as_str()) && header.sender().map(|s| s.as_str()) == Some(portal.as_str())
                        {
                            return Ok(reply);
                        }
                    }
                    Event::Owner(message) => {
                        let message = message.ok_or("the portal connection closed")?.map_err(|e| e.to_string())?;
                        if owner_disappeared(&message, &portal)? {
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
}
