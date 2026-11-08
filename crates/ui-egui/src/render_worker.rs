//! Background rendering (native): heavy documents render off the UI thread while the canvas shows
//! the previous frame reprojected to the current view. A second worker renders a low-resolution
//! frame for the parts that frame does not reach (`canvas::fill_uncovered`). Light documents
//! render synchronously so edits never lag behind overlays, after a first frame that renders here
//! too: the UI thread waits for it only briefly (`canvas::FIRST_FRAME_GRACE`).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use vectorcraft_doc::Document;
use vectorcraft_engine::file_access::ScopeHandle;
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Rendered, Renderer};

use crate::CacheKey;

pub struct Job {
    pub key: CacheKey,
    pub doc: Arc<Document>,
    pub w: u32,
    pub h: u32,
    pub view: Affine,
    pub opts: RenderOptions,
    /// The automation roots in force where the job was made: the render reads linked files under
    /// them, as it would on that thread.
    pub scope: ScopeHandle,
    /// The render may copy pixels from the worker's last frame ([`crate::canvas::render_job`]).
    /// Off: it renders the whole frame.
    pub may_copy: bool,
    /// The worker may keep the frame for its next frame to copy from
    /// ([`crate::canvas::render_job`]). Off: the worker holds no frame after this one.
    pub may_keep: bool,
}

pub struct Done {
    pub key: CacheKey,
    /// The document rendered, and the options it was rendered with.
    pub doc: Arc<Document>,
    pub opts: RenderOptions,
    pub img: Rendered,
    pub ms: f64,
    /// [`vectorcraft_render::FrameStats::moved`] of the frame.
    pub moved: usize,
    /// The frame holds pixels copied from the worker's last frame ([`crate::canvas::render_job`]),
    /// and `ms` is the time of the copy and of the parts rendered.
    pub reused: bool,
}

pub struct Worker {
    tx: Sender<Job>,
    rx: Receiver<Done>,
    pub pending: Option<CacheKey>,
}

impl Worker {
    /// Spawn the worker thread (None on wasm, where rendering stays synchronous).
    pub fn spawn(ctx: egui::Context) -> Option<Self> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        let (tx, jobs) = channel::<Job>();
        let (done_tx, rx) = channel::<Done>();
        std::thread::Builder::new()
            .name("vectorcraft-render".into())
            .spawn(move || {
                let mut r = Renderer::new();
                let mut kept = None;
                while let Ok(mut job) = jobs.recv() {
                    // Skip stale jobs: only the newest matters.
                    while let Ok(newer) = jobs.try_recv() {
                        job = newer;
                    }
                    let t0 = crate::now_ms();
                    let frame = job.scope.clone().run(|| crate::canvas::render_job(&mut r, &mut kept, &job));
                    let ms = crate::now_ms() - t0;
                    let done = Done { key: job.key, doc: job.doc, opts: job.opts, img: frame.img, ms, moved: frame.moved, reused: frame.reused };
                    if done_tx.send(done).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self { tx, rx, pending: None })
    }

    /// A controlled worker: tests receive jobs and decide when a frame completes.
    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, Receiver<Job>, Sender<Done>) {
        let (tx, jobs) = channel();
        let (done_tx, rx) = channel();
        (Self { tx, rx, pending: None }, jobs, done_tx)
    }

    /// Queue `job`, unless the job pending is the same frame. Whether it was queued.
    pub fn submit(&mut self, job: Job) -> bool {
        if self.pending.as_ref() == Some(&job.key) {
            return false;
        }
        self.pending = Some(job.key.clone());
        // A worker that has stopped renders nothing: the canvas keeps the frame it shows.
        self.tx.send(job).is_ok()
    }

    /// The newest finished frame, if any.
    pub fn poll(&mut self) -> Option<Done> {
        self.newest(None)
    }

    /// The newest finished frame, waiting up to `timeout` for one when none has finished yet.
    pub fn wait(&mut self, timeout: std::time::Duration) -> Option<Done> {
        let first = self.rx.try_recv().ok().or_else(|| self.rx.recv_timeout(timeout).ok())?;
        self.newest(Some(first))
    }

    /// The newest of `first` and the frames finished since, the pending job cleared when it is
    /// the one.
    fn newest(&mut self, first: Option<Done>) -> Option<Done> {
        let mut last = first;
        while let Ok(d) = self.rx.try_recv() {
            last = Some(d);
        }
        if let Some(d) = &last
            && self.pending.as_ref() == Some(&d.key)
        {
            self.pending = None;
        }
        last
    }
}
