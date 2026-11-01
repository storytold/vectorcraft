//! Panel thumbnails (the Navigator's view of the document, the Layers and Links rows) rendered
//! off the UI thread on native builds, so a heavy document never stalls a frame. The panels keep
//! showing their last image until the new one is back, and leave thumbnails alone while the user
//! drags (every pointer move makes a new document revision): they update once the drag ends and
//! the document has [`Settle`]d.
//!
//! On wasm (no threads) and in unit tests [`ThumbWorker::spawn`] gives `None` and the panels
//! render on the UI thread, with the same rules about drags and settling.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use vectorcraft_render::{Rendered, Renderer};

/// How long (seconds) the document must stay unchanged before a thumbnail that is already shown
/// is drawn again.
pub const SETTLE_SECS: f64 = 0.1;

/// A render a panel asks for: run on the worker's own [`Renderer`].
pub type JobFn = Box<dyn FnOnce(&mut Renderer) -> Option<Rendered> + Send>;

/// A finished job: the key it was submitted with, its image and how long it took.
pub struct Done<K> {
    pub key: K,
    pub img: Option<Rendered>,
    pub ms: f64,
}

struct Job<S, K> {
    slot: S,
    key: K,
    run: JobFn,
}

/// A render thread for one panel's thumbnails. Jobs go in by slot (the Navigator has one, the
/// Layers panel one per row): a job not started yet is replaced by a newer one for its slot, so
/// only the latest image of anything is drawn.
pub struct ThumbWorker<S, K> {
    tx: Sender<Job<S, K>>,
    rx: Receiver<Done<K>>,
}

impl<S: PartialEq + Send + 'static, K: Send + 'static> ThumbWorker<S, K> {
    /// Spawn the worker thread, `None` where rendering stays on the UI thread: on wasm, in unit
    /// tests (they look at the panel right after the frame that asks), with
    /// `VECTORCRAFT_SYNC_RENDER` set, or if the thread can't start.
    pub fn spawn(ctx: &egui::Context, name: &str) -> Option<Self> {
        if cfg!(target_arch = "wasm32") || cfg!(test) || std::env::var_os("VECTORCRAFT_SYNC_RENDER").is_some() {
            return None;
        }
        Self::start(ctx, name)
    }

    /// Spawn the thread.
    fn start(ctx: &egui::Context, name: &str) -> Option<Self> {
        let (tx, jobs) = channel::<Job<S, K>>();
        let (done_tx, rx) = channel::<Done<K>>();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name(format!("vectorcraft-{name}"))
            .spawn(move || {
                let mut r = Renderer::new();
                let mut queue: VecDeque<Job<S, K>> = VecDeque::new();
                let add = |queue: &mut VecDeque<Job<S, K>>, job: Job<S, K>| match queue.iter_mut().find(|j| j.slot == job.slot) {
                    Some(old) => *old = job,
                    None => queue.push_back(job),
                };
                loop {
                    if queue.is_empty() {
                        // The panel is gone (its sender dropped): end the thread.
                        let Ok(job) = jobs.recv() else { break };
                        add(&mut queue, job);
                    }
                    while let Ok(job) = jobs.try_recv() {
                        add(&mut queue, job);
                    }
                    let Some(job) = queue.pop_front() else { continue };
                    let t0 = crate::now_ms();
                    let img = (job.run)(&mut r);
                    if done_tx.send(Done { key: job.key, img, ms: crate::now_ms() - t0 }).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self { tx, rx })
    }

    /// Queue `run` for `slot` (replacing a job for the slot that hasn't started). False when the
    /// thread is gone: the caller renders on the UI thread from now on.
    #[must_use]
    pub fn submit(&self, slot: S, key: K, run: JobFn) -> bool {
        self.tx.send(Job { slot, key, run }).is_ok()
    }

    /// The jobs finished since the last call, oldest first; None when the thread is gone (the
    /// jobs it had will never finish).
    pub fn poll(&self) -> Option<Vec<Done<K>>> {
        let mut done = vec![];
        loop {
            match self.rx.try_recv() {
                Ok(d) => done.push(d),
                Err(TryRecvError::Empty) => return Some(done),
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

/// When the document last changed, for [`SETTLE_SECS`]: thumbnails already shown are drawn again
/// only once it has stayed the same that long.
#[derive(Default)]
pub struct Settle {
    seen: Option<(u64, u64)>,
    since: f64,
}

impl Settle {
    /// Note that document `uid` is at `revision` at time `now` (seconds, egui's input time) →
    /// whether it has stayed there [`SETTLE_SECS`]. Otherwise `ctx` is asked to draw again when
    /// it will have.
    pub fn settled(&mut self, ctx: &egui::Context, uid: u64, revision: u64, now: f64) -> bool {
        if self.seen != Some((uid, revision)) {
            self.seen = Some((uid, revision));
            self.since = now;
        }
        let waited = now - self.since;
        // The clock went back (a new egui context): don't wait for it to catch up.
        if !waited.is_nan() && !(0.0..SETTLE_SECS).contains(&waited) {
            return true;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((SETTLE_SECS - waited).max(0.0)));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_come_back_newest_last_per_slot() {
        let Some(w) = ThumbWorker::<u8, u32>::start(&egui::Context::default(), "test") else { return };
        let job = |n: u32| -> JobFn { Box::new(move |_| Some(Rendered { width: n, height: 1, pixels: vec![0; 4] })) };
        for k in 1..=3 {
            assert!(w.submit(0, k, job(k)));
        }
        assert!(w.submit(1, 10, job(10)));
        let mut got = vec![];
        let t0 = std::time::Instant::now();
        while !(got.contains(&3) && got.contains(&10)) && t0.elapsed().as_secs() < 30 {
            got.extend(w.poll().unwrap().into_iter().map(|d| {
                assert_eq!(d.img.map(|i| i.width), Some(d.key));
                d.key
            }));
            std::thread::yield_now();
        }
        let slot0: Vec<u32> = got.iter().copied().filter(|k| *k < 10).collect();
        assert_eq!(slot0.last(), Some(&3), "the newest job of a slot is the last done: {got:?}");
        assert!(slot0.windows(2).all(|p| p[0] < p[1]), "{got:?}");
        assert!(got.contains(&10));
        drop(w);
    }

    #[test]
    fn settles_after_the_document_stays_unchanged() {
        let ctx = egui::Context::default();
        let mut s = Settle::default();
        assert!(!s.settled(&ctx, 1, 1, 10.0), "a change starts the wait");
        assert!(!s.settled(&ctx, 1, 1, 10.05));
        assert!(s.settled(&ctx, 1, 1, 10.15));
        assert!(!s.settled(&ctx, 1, 2, 10.2), "a new revision starts it again");
        assert!(!s.settled(&ctx, 2, 2, 10.25), "so does another document");
        assert!(s.settled(&ctx, 2, 2, 1.0), "a clock that went back doesn't hold it up");
        assert!(!s.settled(&ctx, 2, 2, f64::NAN), "an unknown elapsed time is still unsettled");

        let mut s = Settle::default();
        assert!(!s.settled(&ctx, 1, 1, 0.0));
        for (now, expected) in [(f64::NEG_INFINITY, true), (-0.0, false), (0.0, false), (SETTLE_SECS, true), (f64::INFINITY, true), (f64::NAN, false)]
        {
            assert_eq!(s.settled(&ctx, 1, 1, now), expected, "clock boundary {now}");
        }
    }
}
