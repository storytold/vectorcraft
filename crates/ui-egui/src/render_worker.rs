//! Background rendering (native): heavy documents render off the UI thread while the canvas shows
//! the previous frame reprojected to the current view. Light documents render synchronously so
//! edits never lag behind overlays.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use vectorcraft_doc::Document;
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
                while let Ok(mut job) = jobs.recv() {
                    // Skip stale jobs: only the newest matters.
                    while let Ok(newer) = jobs.try_recv() {
                        job = newer;
                    }
                    let t0 = crate::now_ms();
                    let img = r.render(&job.doc, job.w, job.h, job.view, &job.opts);
                    if done_tx.send(Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: crate::now_ms() - t0, moved: r.stats.moved }).is_err()
                    {
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

    pub fn submit(&mut self, job: Job) {
        if self.pending.as_ref() == Some(&job.key) {
            return;
        }
        self.pending = Some(job.key.clone());
        let _ = self.tx.send(job);
    }

    pub fn poll(&mut self) -> Option<Done> {
        let mut last = None;
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
