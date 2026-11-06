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
use std::sync::{Arc, Mutex};

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

/// Which slots' jobs [`ThumbWorker::discard`] or [`ThumbWorker::forget`] drops.
type SlotTest<S> = Box<dyn Fn(&S) -> bool + Send>;

/// The slots [`ThumbWorker::forget`] picked since the worker was last idle.
type Forgotten<S> = Arc<Mutex<Vec<SlotTest<S>>>>;

/// What a panel sends its worker: a job, or the slots whose jobs not started yet are dropped.
enum Msg<S, K> {
    Job(Job<S, K>),
    Discard(SlotTest<S>),
}

/// A render thread for one panel's thumbnails. Jobs go in by slot (the Navigator has one, the
/// Layers panel one per row): a job not started yet is replaced by a newer one for its slot, so
/// only the latest image of anything is drawn.
pub struct ThumbWorker<S, K> {
    tx: Sender<Msg<S, K>>,
    rx: Receiver<Done<K>>,
    forgotten: Forgotten<S>,
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
    pub(crate) fn start(ctx: &egui::Context, name: &str) -> Option<Self> {
        let (tx, jobs) = channel::<Msg<S, K>>();
        let (done_tx, rx) = channel::<Done<K>>();
        let forgotten: Forgotten<S> = Arc::default();
        let gone = forgotten.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name(format!("vectorcraft-{name}"))
            .spawn(move || {
                let mut r = Renderer::new();
                let mut queue: VecDeque<Job<S, K>> = VecDeque::new();
                let add = |queue: &mut VecDeque<Job<S, K>>, msg: Msg<S, K>| match msg {
                    Msg::Job(job) => match queue.iter_mut().find(|j| j.slot == job.slot) {
                        Some(old) => *old = job,
                        None => queue.push_back(job),
                    },
                    Msg::Discard(which) => queue.retain(|j| !which(&j.slot)),
                };
                loop {
                    if queue.is_empty() {
                        // With no job here or on its way (checked under the lock, since a panel
                        // sends a job before it forgets that job's slot), what `forget` picked
                        // has no job left to drop.
                        let waiting = match gone.lock() {
                            Ok(mut f) => match jobs.try_recv() {
                                Ok(msg) => Some(msg),
                                Err(_) => {
                                    f.clear();
                                    None
                                }
                            },
                            Err(_) => break,
                        };
                        let msg = match waiting {
                            Some(msg) => msg,
                            // The panel is gone (its sender dropped): end the thread.
                            None => match jobs.recv() {
                                Ok(msg) => msg,
                                Err(_) => break,
                            },
                        };
                        add(&mut queue, msg);
                    }
                    while let Ok(job) = jobs.try_recv() {
                        add(&mut queue, job);
                    }
                    let Some(job) = queue.pop_front() else { continue };
                    let Ok(skip) = gone.lock().map(|f| f.iter().any(|which| which(&job.slot))) else { break };
                    if skip {
                        continue;
                    }
                    let t0 = crate::now_ms();
                    let img = (job.run)(&mut r);
                    let done = Done { key: job.key, img, ms: crate::now_ms() - t0 };
                    // Sent under the lock, so a `forget` either drops it here or comes after it
                    // and finds it in the next `poll`.
                    let Ok(f) = gone.lock() else { break };
                    if f.iter().any(|which| which(&job.slot)) {
                        continue;
                    }
                    if done_tx.send(done).is_err() {
                        break;
                    }
                    drop(f);
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self { tx, rx, forgotten })
    }

    /// Queue `run` for `slot` (replacing a job for the slot that hasn't started). False when the
    /// thread is gone: the caller renders on the UI thread from now on.
    #[must_use]
    pub fn submit(&self, slot: S, key: K, run: JobFn) -> bool {
        self.tx.send(Msg::Job(Job { slot, key, run })).is_ok()
    }

    /// Drop the jobs not started yet whose slot `which` picks, so they don't hold up the jobs
    /// submitted after them; they never come back from [`ThumbWorker::poll`]. A job already
    /// running still finishes. False when the thread is gone, as for [`ThumbWorker::submit`].
    #[must_use]
    pub fn discard(&self, which: impl Fn(&S) -> bool + Send + 'static) -> bool {
        self.tx.send(Msg::Discard(Box::new(which))).is_ok()
    }

    /// Drop the jobs whose slot `which` picks, queued or running: their images never come back
    /// from [`ThumbWorker::poll`], except those finished before this call, which the next poll
    /// returns. The panel must not submit jobs for those slots again.
    pub fn forget(&self, which: impl Fn(&S) -> bool + Send + 'static) {
        if let Ok(mut f) = self.forgotten.lock() {
            f.push(Box::new(which));
        }
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

    /// The next finished job, waiting for it; None when the thread is gone.
    #[cfg(test)]
    pub(crate) fn wait(&self) -> Option<Done<K>> {
        self.rx.recv().ok()
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
    fn discarded_jobs_do_not_run_and_the_running_one_finishes() {
        let Some(w) = ThumbWorker::<u8, u32>::start(&egui::Context::default(), "test") else { return };
        let job = |n: u32| -> JobFn { Box::new(move |_| Some(Rendered { width: n, height: 1, pixels: vec![0; 4] })) };
        // A job that holds the thread until it is let go, so the others queue behind it.
        let (started, running) = channel::<()>();
        let (release, held) = channel::<()>();
        let blocker: JobFn = Box::new(move |_| {
            started.send(()).ok();
            held.recv().ok();
            None
        });
        assert!(w.submit(0, 0, blocker));
        running.recv().expect("the first job starts");
        assert!(w.submit(1, 1, job(1)));
        assert!(w.submit(2, 2, job(2)));
        assert!(w.submit(1, 3, job(3)));
        assert!(w.discard(|slot| *slot == 0 || *slot == 1));
        assert!(w.submit(3, 4, job(4)));
        release.send(()).expect("the first job is waiting");
        assert_eq!(w.wait().map(|d| d.key), Some(0), "the running job finishes");
        assert_eq!(w.wait().map(|d| d.key), Some(2), "slot 1's jobs were dropped");
        assert_eq!(w.wait().map(|d| d.key), Some(4), "a job submitted after the discard runs");
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
