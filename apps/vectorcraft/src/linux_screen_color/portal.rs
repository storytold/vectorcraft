//! The XDG desktop portal's `Screenshot.PickColor`
//! (<https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Screenshot.html>):
//! the call returns a request path at once, and the request's `Response` signal later carries the
//! color the person picked, as sRGB components from 0 to 1, or a code for a cancel or another end.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

/// How long the portal has to return a request path for `PickColor`. VectorCraft's own limit,
/// 25 seconds as libdbus's default for a method call. The portal returns the path before it shows
/// the sampler, so the person's time with the sampler does not count.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

/// Why no color came back, as the status bar shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    /// No session bus, no portal, or a portal without the Screenshot interface's `PickColor`.
    Unavailable(String),
    /// The portal did not return a request path in time, or it left the bus before it answered.
    NoAnswer(String),
    /// The portal ended the request without a color, or its answer can't be read.
    Failed(String),
}

impl Failure {
    /// The status bar's message (English; the status bar translates it).
    pub(super) fn message(&self) -> &'static str {
        match self {
            Failure::Unavailable(_) => "this system has no screen color sampler",
            Failure::NoAnswer(_) => "the screen color sampler did not answer",
            Failure::Failed(_) => "the screen color sampler failed",
        }
    }

    /// What went wrong, for the log.
    pub(super) fn detail(&self) -> &str {
        match self {
            Failure::Unavailable(d) | Failure::NoAnswer(d) | Failure::Failed(d) => d,
        }
    }
}

/// D-Bus errors that mean the session has no portal, or none that answers `PickColor`.
const UNAVAILABLE: &[&str] = &[
    "org.freedesktop.DBus.Error.ServiceUnknown",
    "org.freedesktop.DBus.Error.NameHasNoOwner",
    "org.freedesktop.DBus.Error.UnknownMethod",
    "org.freedesktop.DBus.Error.UnknownInterface",
    "org.freedesktop.DBus.Error.UnknownObject",
];

/// The connection `builder` makes, with [`CALL_TIMEOUT`] for its method calls.
pub(super) fn connect(builder: zbus::Result<zbus::blocking::connection::Builder<'_>>) -> Result<Connection, Failure> {
    builder.and_then(|b| b.method_timeout(CALL_TIMEOUT).build()).map_err(|e| Failure::Unavailable(e.to_string()))
}

/// Shows the portal's color sampler over the window `parent` names and waits for the color, `None`
/// when the person canceled it. `conn`'s method timeout bounds the call; the wait for the person
/// after it has no bound, and ends when the portal answers or leaves the bus.
pub(super) fn pick(conn: &Connection, parent: &str) -> Result<Option<[f32; 3]>, Failure> {
    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    let token = crate::portal_request::token("vectorcraft_color").map_err(Failure::Failed)?;
    // Made before the call: a portal can send its answer before the call returns.
    let replies = crate::portal_request::Responses::new(conn).map_err(Failure::Failed)?;
    let proxy = Proxy::new(conn, DESKTOP, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Screenshot")
        .map_err(|e| Failure::Failed(e.to_string()))?;
    let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
    let call = proxy.call_method("PickColor", &(parent, options)).map_err(|e| call_failure(&e))?;
    let reply = replies.wait(&call).map_err(Failure::NoAnswer)?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(|e| Failure::Failed(e.to_string()))?;
    answer(code, &results)
}

/// The [`Failure`] for an error the `PickColor` call returned. zbus reports a call that outlives
/// the connection's method timeout as an `InputOutput` error of kind `TimedOut`.
fn call_failure(e: &zbus::Error) -> Failure {
    match e {
        zbus::Error::MethodError(name, ..) if UNAVAILABLE.contains(&name.as_str()) => Failure::Unavailable(e.to_string()),
        zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => Failure::NoAnswer(e.to_string()),
        _ => Failure::Failed(e.to_string()),
    }
}

/// The color in a `Response` with `code` and `results`: response 0 carries it, 1 is a cancel, and
/// any other code ends the request without one. The components are passed on as given;
/// `screen_color::sampled` clamps them to 0 to 1, as for the other systems' samplers.
fn answer(code: u32, results: &HashMap<String, OwnedValue>) -> Result<Option<[f32; 3]>, Failure> {
    match code {
        0 => {}
        1 => return Ok(None),
        _ => return Err(Failure::Failed(format!("the portal ended the request (response {code})"))),
    }
    let color = results.get("color").ok_or_else(|| Failure::Failed("no color in the portal's answer".into()))?;
    // Matched field by field: zvariant's tuple conversion panics on a structure with fewer fields.
    match &**color {
        Value::Structure(s) => match s.fields() {
            [Value::F64(r), Value::F64(g), Value::F64(b)] => Ok(Some([*r, *g, *b].map(|v| v as f32))),
            _ => Err(Failure::Failed(format!("the portal's color is not three numbers: {color:?}"))),
        },
        _ => Err(Failure::Failed(format!("the portal's color is not three numbers: {color:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_request::test_bus::private_bus;

    fn color(rgb: (f64, f64, f64)) -> HashMap<String, OwnedValue> {
        HashMap::from([("color".into(), OwnedValue::try_from(Value::from(rgb)).unwrap())])
    }

    fn failed<T: std::fmt::Debug>(r: Result<T, Failure>) -> bool {
        matches!(r, Err(Failure::Failed(_)))
    }

    #[test]
    fn a_picked_color_keeps_its_srgb_values() {
        assert_eq!(answer(0, &color((0.25, 0.5, 0.75))), Ok(Some([0.25, 0.5, 0.75])));
        assert_eq!(answer(0, &color((0.0, 1.0, 0.0))), Ok(Some([0.0, 1.0, 0.0])));
        // Outside 0 to 1: passed on for `screen_color::sampled` to clamp.
        assert_eq!(answer(0, &color((-0.5, 1.5, 1.0))), Ok(Some([-0.5, 1.5, 1.0])));
    }

    #[test]
    fn a_cancel_needs_no_color() {
        assert_eq!(answer(1, &HashMap::new()), Ok(None));
        assert_eq!(answer(1, &color((0.25, 0.5, 0.75))), Ok(None));
    }

    #[test]
    fn other_response_codes_are_failures() {
        for code in [2, 3, u32::MAX] {
            assert!(failed(answer(code, &color((0.25, 0.5, 0.75)))), "{code}");
        }
    }

    #[test]
    fn a_success_without_a_valid_color_is_a_failure() {
        assert!(failed(answer(0, &HashMap::new())));
        let wrong = HashMap::from([("color".into(), OwnedValue::from(1_u32))]);
        assert!(failed(answer(0, &wrong)));
        // Fewer fields, more fields, and fields of another type: an error, not a panic.
        for wrong in [Value::from((0.5, 0.5)), Value::from((0.5, 0.5, 0.5, 0.5)), Value::from((1_u32, 0.5, 0.5)), Value::from(vec![0.5, 0.5, 0.5])] {
            let wrong = HashMap::from([("color".into(), OwnedValue::try_from(wrong).unwrap())]);
            assert!(failed(answer(0, &wrong)));
        }
    }

    #[test]
    fn call_errors_say_whether_the_system_has_a_sampler() {
        let reply = |name: &str| {
            let call = zbus::Message::method_call("/org/freedesktop/portal/desktop", "PickColor").unwrap().build(&()).unwrap();
            let error = zbus::Message::error(&call.header(), name).unwrap().build(&("detail",)).unwrap();
            zbus::Error::from(error)
        };
        for name in UNAVAILABLE {
            assert!(matches!(call_failure(&reply(name)), Failure::Unavailable(_)), "{name}");
        }
        assert!(matches!(call_failure(&reply("org.freedesktop.portal.Error.NotAllowed")), Failure::Failed(_)));
        let timed_out = zbus::Error::InputOutput(std::sync::Arc::new(std::io::ErrorKind::TimedOut.into()));
        assert!(matches!(call_failure(&timed_out), Failure::NoAnswer(_)));
        let broken = zbus::Error::InputOutput(std::sync::Arc::new(std::io::ErrorKind::BrokenPipe.into()));
        assert!(matches!(call_failure(&broken), Failure::Failed(_)));
        let messages = [Failure::Unavailable(String::new()), Failure::NoAnswer(String::new()), Failure::Failed(String::new())].map(|f| f.message());
        assert_eq!(
            messages,
            ["this system has no screen color sampler", "the screen color sampler did not answer", "the screen color sampler failed"]
        );
        assert_eq!(call_failure(&broken).detail(), broken.to_string());
    }

    /// Every language that translates the sampler's other messages translates these too. The
    /// i18n tests of `vectorcraft-ui-egui` do not read this crate's messages.
    #[test]
    fn the_messages_are_translated_where_the_sampler_is() {
        use vectorcraft_ui_egui::i18n::{Lang, message};
        const OPEN: &str = "the screen color sampler is already open";
        let langs: Vec<Lang> = Lang::all().filter(|&lang| message(lang, OPEN) != OPEN).collect();
        assert!(!langs.is_empty(), "no language translates {OPEN:?}");
        for lang in langs {
            for failure in [Failure::Unavailable(String::new()), Failure::NoAnswer(String::new()), Failure::Failed(String::new())] {
                assert_ne!(message(lang, failure.message()), failure.message(), "{}", lang.code());
            }
        }
    }

    /// The client's connection to the private bus at `address`, made as the sampler makes its own.
    fn client(address: &str) -> Connection {
        connect(zbus::blocking::connection::Builder::address(address)).unwrap()
    }

    /// The portal's name, owned on the private bus at `address` before any call, so the bus never
    /// starts an installed portal in its place.
    fn portal(address: &str) -> Connection {
        zbus::blocking::connection::Builder::address(address).unwrap().name("org.freedesktop.portal.Desktop").unwrap().build().unwrap()
    }

    /// Through a real D-Bus: an answer that comes before the call returns is kept; answers on
    /// other request paths or from other connections are ignored; a cancel gives no color; and a
    /// portal that leaves the bus while the person picks ends the wait.
    #[test]
    fn the_answer_is_the_portals_response_on_the_returned_path() {
        use zbus::blocking::MessageIterator;
        use zbus::zvariant::OwnedObjectPath;

        let Some((_daemon, address)) = private_bus() else { return };
        let address = address.as_str();
        let server = portal(address);
        let impostor = zbus::blocking::connection::Builder::address(address).unwrap().build().unwrap();
        let incoming = MessageIterator::from(&server);
        let worker = std::thread::spawn(move || {
            let mut calls = Vec::new();
            for msg in incoming {
                let msg = msg.unwrap();
                let header = msg.header();
                if header.message_type() != zbus::message::Type::MethodCall || header.member().map(|m| m.as_str()) != Some("PickColor") {
                    continue;
                }
                let (parent, options): (String, HashMap<String, OwnedValue>) = msg.body().deserialize().unwrap();
                let token = String::try_from(options.get("handle_token").unwrap().try_clone().unwrap()).unwrap();
                assert!(token.starts_with("vectorcraft_color_"));
                assert!(token.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'));
                assert!(!calls.iter().any(|(_, old)| old == &token));
                calls.push((parent, token));
                // A portal older than version 0.9 answers on a path of its own, not the token's.
                let path = format!("/org/freedesktop/portal/desktop/request/independent/pick_{}", calls.len());
                let handle = OwnedObjectPath::try_from(path.as_str()).unwrap();
                if calls.len() == 4 {
                    server.reply(&header, &handle).unwrap();
                    assert!(server.release_name("org.freedesktop.portal.Desktop").unwrap());
                    break;
                }
                let to = header.sender().cloned().map(zbus::names::BusName::from);
                // Another connection can't end the wait by claiming the portal left.
                impostor
                    .emit_signal(
                        to.clone(),
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "NameOwnerChanged",
                        &("org.freedesktop.portal.Desktop", server.unique_name().unwrap().as_str(), ""),
                    )
                    .unwrap();
                let wrong = color((1.0, 0.0, 0.0));
                impostor.emit_signal(to.clone(), path.as_str(), "org.freedesktop.portal.Request", "Response", &(0_u32, &wrong)).unwrap();
                let unrelated = "/org/freedesktop/portal/desktop/request/unrelated";
                server.emit_signal(to.clone(), unrelated, "org.freedesktop.portal.Request", "Response", &(0_u32, &wrong)).unwrap();
                let code = if calls.len() == 2 { 1_u32 } else { 0_u32 };
                server.emit_signal(to, path.as_str(), "org.freedesktop.portal.Request", "Response", &(code, &color((0.25, 0.5, 0.75)))).unwrap();
                // The reply that names the path goes last: every answer above came before it.
                server.reply(&header, &handle).unwrap();
            }
            calls
        });
        let client = client(address);
        assert_eq!(pick(&client, "x11:12"), Ok(Some([0.25, 0.5, 0.75])));
        assert_eq!(pick(&client, "wayland:example"), Ok(None));
        assert_eq!(pick(&client, ""), Ok(Some([0.25, 0.5, 0.75])));
        assert!(matches!(pick(&client, ""), Err(Failure::NoAnswer(_))));
        let calls = worker.join().unwrap();
        assert_eq!(calls.iter().map(|(parent, _)| parent.as_str()).collect::<Vec<_>>(), ["x11:12", "wayland:example", "", ""]);
    }

    /// Through a real D-Bus: a portal without `PickColor` ends the request as a system without a
    /// sampler, and the sampler's connection gives the call [`CALL_TIMEOUT`].
    #[test]
    fn a_portal_without_pick_color_ends_the_request() {
        let Some((_daemon, address)) = private_bus() else { return };
        let address = address.as_str();
        let server = portal(address);
        let incoming = zbus::blocking::MessageIterator::from(&server);
        let worker = std::thread::spawn(move || {
            for msg in incoming {
                let msg = msg.unwrap();
                let header = msg.header();
                if header.member().map(|m| m.as_str()) == Some("PickColor") {
                    server.reply_error(&header, "org.freedesktop.DBus.Error.UnknownMethod", &("no PickColor",)).unwrap();
                    break;
                }
            }
        });
        let client = client(address);
        assert_eq!(client.method_timeout(), Some(CALL_TIMEOUT));
        let failure = pick(&client, "");
        assert!(matches!(failure, Err(Failure::Unavailable(_))), "{failure:?}");
        worker.join().unwrap();
    }
}
