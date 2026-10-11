//! Owned Linux appearance watcher. Numeric/nested portal decoding is adapted from the
//! MIT OR Apache-2.0 implementation in https://github.com/storytold/vectorcraft/pull/928
//! at bad9fcca8df67a2bbe534ffe1f97e02c954d0a39. Cancellation follows the owned async
//! service design used by PDFCraft: cancel the entire task, then join on owner drop.

#[cfg(any(target_os = "linux", test))]
use std::sync::atomic::AtomicBool;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

struct Shared {
    value: AtomicU8,
    #[cfg(any(target_os = "linux", test))]
    published: AtomicBool,
    #[cfg(any(target_os = "linux", test))]
    closed: AtomicBool,
    #[cfg(any(target_os = "linux", test))]
    wake: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn get(&self) -> Option<egui::Theme> {
        decode(u32::from(self.value.load(Ordering::Acquire)))
    }

    #[cfg(any(target_os = "linux", test))]
    fn publish(&self, appearance: Option<egui::Theme>) {
        let value = match appearance {
            Some(egui::Theme::Dark) => 1,
            Some(egui::Theme::Light) => 2,
            None => 0,
        };
        // The cache is published before wakeup, including the initial unavailable result.
        let changed = self.value.swap(value, Ordering::AcqRel) != value;
        let first = !self.published.swap(true, Ordering::AcqRel);
        if (first || changed) && !self.closed.load(Ordering::Acquire) {
            (self.wake)();
        }
    }
}

/// Cached reads never perform I/O. The window owns cancellation and the worker's lifetime.
pub struct Reader {
    shared: Arc<Shared>,
    #[cfg(any(target_os = "linux", test))]
    stop: Option<async_channel::Sender<()>>,
    #[cfg(any(target_os = "linux", test))]
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Reader {
    pub fn get(&self) -> Option<egui::Theme> {
        self.shared.get()
    }

    #[cfg(any(target_os = "linux", test))]
    fn spawn<F, T>(wake: impl Fn() + Send + Sync + 'static, task: T) -> Option<Self>
    where
        F: std::future::Future<Output = ()> + 'static,
        T: FnOnce(Arc<Shared>) -> F + Send + 'static,
    {
        let shared =
            Arc::new(Shared { value: AtomicU8::new(0), published: AtomicBool::new(false), closed: AtomicBool::new(false), wake: Box::new(wake) });
        let worker_shared = Arc::clone(&shared);
        let (stop, stopped) = async_channel::bounded::<()>(1);
        let worker = std::thread::Builder::new()
            .name("appearance-portal".into())
            .spawn(move || {
                // Cancellation has priority and encompasses setup, Read, and idle signal reception.
                futures_lite::future::block_on(futures_lite::future::or(
                    async {
                        let _ = stopped.recv().await;
                    },
                    task(worker_shared),
                ));
            })
            .ok()?;
        Some(Self { shared, stop: Some(stop), worker: Some(worker) })
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        #[cfg(any(target_os = "linux", test))]
        {
            self.shared.closed.store(true, Ordering::Release);
            if let Some(stop) = self.stop.take() {
                stop.close();
            }
            if let Some(worker) = self.worker.take() {
                // A worker panic must not propagate into the window or its unsaved document.
                let _ = worker.join();
            }
        }
    }
}

fn decode(code: u32) -> Option<egui::Theme> {
    match code {
        1 => Some(egui::Theme::Dark),
        2 => Some(egui::Theme::Light),
        _ => None,
    }
}

pub fn service(ctx: &egui::Context) -> Option<Reader> {
    #[cfg(target_os = "linux")]
    {
        let ctx = ctx.clone();
        Reader::spawn(move || ctx.request_repaint(), |shared| portal::watch(shared, None))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ctx;
        None
    }
}

#[cfg(any(target_os = "linux", test))]
mod portal {
    use super::{Shared, decode};
    use futures_lite::StreamExt;
    use std::sync::Arc;

    pub(super) const DESTINATION: &str = "org.freedesktop.portal.Desktop";
    pub(super) const PATH: &str = "/org/freedesktop/portal/desktop";
    pub(super) const INTERFACE: &str = "org.freedesktop.portal.Settings";
    pub(super) const NAMESPACE: &str = "org.freedesktop.appearance";
    pub(super) const KEY: &str = "color-scheme";

    pub(super) fn code(value: zbus::zvariant::OwnedValue) -> Option<u32> {
        let mut value: zbus::zvariant::Value<'_> = value.into();
        // Settings.Read uses variants; retain support for the standard nested numeric reply.
        for _ in 0..4 {
            match value {
                zbus::zvariant::Value::Value(inner) => value = *inner,
                other => return u32::try_from(other).ok(),
            }
        }
        None
    }

    pub(super) async fn watch(shared: Arc<Shared>, address: Option<String>) {
        if watch_inner(&shared, address).await.is_err() {
            shared.publish(None);
        }
    }

    async fn watch_inner(shared: &Shared, address: Option<String>) -> zbus::Result<()> {
        // Build inside the cancellable future, so even authentication can be interrupted.
        let builder = match address.as_deref() {
            Some(address) => zbus::connection::Builder::address(address)?,
            None => zbus::connection::Builder::session()?,
        };
        let connection = builder.method_timeout(std::time::Duration::from_secs(2)).build().await?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(DESTINATION)?
            .path(PATH)?
            .interface(INTERFACE)?
            .member("SettingChanged")?
            .arg(0, NAMESPACE)?
            .arg(1, KEY)?
            .build();
        // Install the subscription before Read, including portals activated by that call.
        let mut signals = zbus::MessageStream::for_match_rule(rule, &connection, Some(64)).await?;
        let reply = connection.call_method(Some(DESTINATION), PATH, Some(INTERFACE), "Read", &(NAMESPACE, KEY)).await;
        let initial = reply.ok().and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedValue>().ok()).and_then(code).and_then(decode);
        shared.publish(initial);
        while let Some(message) = signals.next().await {
            let message = message?;
            let Ok((namespace, key, changed)) = message.body().deserialize::<(String, String, zbus::zvariant::OwnedValue)>() else { continue };
            if namespace == NAMESPACE
                && key == KEY
                && let Some(value) = code(changed)
            {
                shared.publish(decode(value));
            }
        }
        shared.publish(None);
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests_appearance.rs"]
mod tests;
