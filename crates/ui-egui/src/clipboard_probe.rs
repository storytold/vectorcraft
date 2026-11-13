//! Checks the system clipboard for Paste off the UI thread. `has()` can block for seconds when the
//! clipboard owner never answers (an unresponsive X11 client), so it must not run on the thread that
//! draws the window: a worker owns a second clipboard handle, sends its verdict when it changes, and
//! the UI only reads the latest one. Without a worker the check stays in line, as before (the web,
//! and hosts that don't install a factory).

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;

use crate::sysclip::ClipboardProbeFactory;

/// How often the worker looks at the clipboard while the UI wants to know.
const PROBE_INTERVAL: Duration = Duration::from_millis(250);

/// How long dropping the probe (when the app quits) waits for the worker to let go of its clipboard
/// handle (VectorCraft's own bound). arboard's X11 reads wait up to 4 s for each text format they
/// ask for; when a look is still waiting after this bound, the app quits without waiting longer,
/// and what it copied may not reach the clipboard manager.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// The background clipboard check ([`crate::VectorcraftApp`]'s `clipboard_probe`).
pub(crate) struct Probe {
    /// Tells the worker whether Paste depends on the system clipboard; dropped to stop it.
    wants: Option<Sender<bool>>,
    wanted: bool,
    verdicts: Receiver<bool>,
    /// The worker's last verdict.
    last: bool,
}

impl Probe {
    /// Start the worker. `None` on wasm, or when the thread can't be spawned: the caller then falls
    /// back to the in-line check.
    pub(crate) fn start(make: ClipboardProbeFactory, ctx: egui::Context) -> Option<Self> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        let (wants, wanted_rx) = std::sync::mpsc::channel();
        let (tx, verdicts) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("vectorcraft-clip".into())
            .spawn(move || {
                let mut clipboard = make();
                let (mut wanted, mut sent) = (false, None);
                loop {
                    if wanted {
                        // This thread may wait here for a slow owner; the UI's never does.
                        let verdict = clipboard.has(&PASTE_ORDER);
                        if sent != Some(verdict) {
                            if tx.send(verdict).is_err() {
                                break;
                            }
                            sent = Some(verdict);
                            ctx.request_repaint();
                        }
                    }
                    // Sleep until the next look (no looks while unwanted) or the UI's next word.
                    let next =
                        if wanted { wanted_rx.recv_timeout(PROBE_INTERVAL) } else { wanted_rx.recv().map_err(|_| RecvTimeoutError::Disconnected) };
                    match next {
                        Ok(w) => wanted = w,
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                // The handle goes before the app stops waiting (`Drop`): on X11 the last one alive
                // hands what the app copied to the clipboard manager, which must finish before exit.
                drop(clipboard);
                drop(tx);
            })
            .ok()?;
        Some(Self { wants: Some(wants), wanted: false, verdicts, last: false })
    }

    /// Paste can take from the system clipboard: `wanted` (nothing copied in the app, a document
    /// open) and the worker's latest verdict says so. Never blocks. `None` once the worker is gone.
    pub(crate) fn pasteable(&mut self, wanted: bool) -> Option<bool> {
        if wanted != self.wanted {
            self.wants.as_ref()?.send(wanted).ok()?;
            self.wanted = wanted;
        }
        loop {
            match self.verdicts.try_recv() {
                Ok(verdict) => self.last = verdict,
                Err(TryRecvError::Empty) => return Some(wanted && self.last),
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

impl Drop for Probe {
    /// Wake the worker to stop and wait ([`STOP_WAIT`] at most: it may be stuck on an owner that
    /// never answers) until it has let go of its clipboard handle.
    fn drop(&mut self) {
        self.wants = None;
        let end = Instant::now() + STOP_WAIT;
        while self.verdicts.recv_timeout(end.saturating_duration_since(Instant::now())).is_ok() {}
    }
}
