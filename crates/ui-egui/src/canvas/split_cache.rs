//! Coherent, coalesced two-mode canvas frames on one persistent thumbnail worker.

use egui::{TextureHandle, TextureOptions};
use vectorcraft_render::{RenderOptions, Rendered, Renderer};

use crate::CacheKey;
use crate::render_worker::Job;
use crate::thumb_worker::{Done, JobFn, ThumbWorker};

const DRAIN_SLOT: u8 = 2;

pub(super) struct Pair {
    pub(super) keys: [CacheKey; 2],
    pub(super) textures: [TextureHandle; 2],
    pub(super) ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompletionKey {
    epoch: u64,
    sequence: u64,
    side: u8,
}

#[derive(Clone, PartialEq)]
struct Identity {
    keys: [CacheKey; 2],
    options: [RenderOptions; 2],
}

impl Identity {
    fn of(jobs: &[Job; 2]) -> Self {
        Self { keys: jobs.each_ref().map(|job| job.key.clone()), options: jobs.each_ref().map(|job| job.opts.clone()) }
    }

    fn same_context(&self, other: &Self) -> bool {
        self.keys.iter().zip(&other.keys).all(|(a, b)| {
            let mut a = a.clone();
            a.revision = b.revision;
            a.interacting = b.interacting;
            a.dragging = b.dragging;
            a.zoom = b.zoom;
            a.cx = b.cx;
            a.cy = b.cy;
            a.w = b.w;
            a.h = b.h;
            a.ppp = b.ppp;
            a.rot = b.rot;
            a.placed = b.placed;
            a.plugins = b.plugins;
            a.pixel = b.pixel;
            a.backdrop = b.backdrop.clone();
            a == *b
        }) && self.options.iter().zip(&other.options).all(|(a, b)| {
            let mut a = a.clone();
            a.interacting = b.interacting;
            a.dragging = b.dragging;
            a == *b
        })
    }
}

struct Request {
    identity: Identity,
    jobs: [Job; 2],
    placed_generation: u64,
}

impl Request {
    fn new(mut jobs: [Job; 2]) -> Option<Self> {
        let [a, b] = &jobs;
        let x = &a.key;
        let y = &b.key;
        let common = x.doc == y.doc
            && x.revision == y.revision
            && (x.zoom, x.cx, x.cy, x.w, x.h, x.ppp, x.rot) == (y.zoom, y.cx, y.cy, y.w, y.h, y.ppp, y.rot)
            && std::sync::Arc::ptr_eq(&a.doc, &b.doc);
        if !common {
            return None;
        }
        for job in &mut jobs {
            job.may_copy = false;
            job.may_keep = false;
        }
        Some(Self { identity: Identity::of(&jobs), jobs, placed_generation: vectorcraft_render::placed_document::generation() })
    }
}

struct Batch {
    epoch: u64,
    sequence: u64,
    request: Request,
    images: [Option<Rendered>; 2],
    ms: f64,
}

/// One shown pair, one submitted pair and one coalesced desired snapshot. Divider geometry is
/// absent: callers submit the same keys/options when only its position or orientation changes.
#[derive(Default)]
pub(crate) struct SplitCache {
    worker: Option<ThumbWorker<u8, CompletionKey>>,
    started: bool,
    renderer: Option<Renderer>,
    context: Option<Identity>,
    pending: Option<Batch>,
    latest: Option<Request>,
    shown: Option<Pair>,
    shown_identity: Option<Identity>,
    shown_sequence: u64,
    epoch: u64,
    sequence: u64,
    // A completion independent of the current epoch: repeated clears must not invalidate its
    // acknowledgement or queue more closures while the obsolete render still owns the worker.
    draining: Option<CompletionKey>,
    #[cfg(test)]
    controlled: Option<tests::Controlled>,
}

impl SplitCache {
    pub(super) fn shown(&self) -> Option<&Pair> {
        self.shown.as_ref()
    }

    pub(super) fn update(&mut self, ctx: &egui::Context, jobs: [Job; 2]) {
        let Some(request) = Request::new(jobs) else {
            self.clear();
            return;
        };
        if self.context.as_ref().is_some_and(|old| !old.same_context(&request.identity)) {
            self.clear();
        }
        self.context = Some(request.identity.clone());
        if !self.started {
            self.started = true;
            self.worker = ThumbWorker::spawn(ctx, "split-canvas");
        }
        // Check the new context before polling: a completed old-mode frame must never be uploaded.
        let requested = self.pending.as_ref().map(|batch| &batch.request.identity).or(self.shown_identity.as_ref());
        self.latest = (requested != Some(&request.identity)).then_some(request);
        self.poll(ctx);
        self.start_latest(ctx);
    }

    pub(super) fn clear(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.context = None;
        self.latest = None;
        self.shown = None;
        self.shown_identity = None;
        self.shown_sequence = 0;
        self.renderer = None;
        let busy = self.pending.take().is_some();
        if busy && self.draining.is_none() && self.has_worker() {
            let key = CompletionKey { epoch: self.epoch, sequence: self.next_sequence(), side: DRAIN_SLOT };
            self.draining = Some(key);
            // Discard is asynchronous. One following barrier, not a new pair on each toggle,
            // bounds Doc-owning closures even while an obsolete render is blocked.
            if !self.discard_sides() || !self.submit(DRAIN_SLOT, key, Box::new(|_| None)) {
                self.disable_worker();
            }
        }
        // Also drain while disabled/no document; never retain completed obsolete image buffers.
        self.poll_cleared();
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    fn start_latest(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.draining.is_some() {
            return;
        }
        let Some(mut request) = self.latest.take() else { return };
        if !self.has_worker() {
            self.render_sync(ctx, request);
            return;
        }
        let sequence = self.next_sequence();
        let keys = [0, 1].map(|side| CompletionKey { epoch: self.epoch, sequence, side });
        // Capture the raw generation before either side can read the shared placed cache.
        request.placed_generation = vectorcraft_render::placed_document::generation();
        let sent = request.jobs.iter().zip(keys).all(|(job, key)| {
            let job = clone_job(job);
            self.submit(key.side, key, Box::new(move |renderer| Some(render(renderer, &job))))
        });
        if sent {
            self.pending = Some(Batch { epoch: self.epoch, sequence, request, images: [None, None], ms: 0.0 });
        } else {
            self.disable_worker();
            self.render_sync(ctx, request);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(completed) = self.completions() else {
            self.fallback(ctx);
            return;
        };
        for done in completed {
            if self.draining == Some(done.key) {
                self.draining = None;
                continue;
            }
            if !self.accept(done) {
                self.fallback(ctx);
                break;
            }
        }
        if self.pending.as_ref().is_some_and(|batch| batch.images.iter().all(Option::is_some))
            && let Some(batch) = self.pending.take()
            && let [Some(a), Some(b)] = batch.images
        {
            self.publish(ctx, batch.request, [a, b], batch.ms, batch.sequence);
        }
    }

    // False is a current-batch render failure. Old epochs, duplicate sides and invalid keys are
    // ignored; they must not turn an unrelated current request into a synchronous fallback.
    fn accept(&mut self, done: Done<CompletionKey>) -> bool {
        let Some(batch) = self.pending.as_mut() else { return true };
        if done.key.epoch != self.epoch || done.key.epoch != batch.epoch || done.key.sequence != batch.sequence {
            return true;
        }
        let Some(image) = batch.images.get_mut(usize::from(done.key.side)) else { return true };
        if image.is_some() {
            return true;
        }
        let Some(img) = done.img else { return false };
        *image = Some(img);
        batch.ms += done.ms;
        true
    }

    fn poll_cleared(&mut self) {
        match self.completions() {
            Some(completed) => {
                if completed.iter().any(|done| self.draining == Some(done.key)) {
                    self.draining = None;
                }
            }
            None => self.disable_worker(),
        }
    }

    fn fallback(&mut self, ctx: &egui::Context) {
        let pending = self.pending.take().map(|batch| batch.request);
        self.disable_worker();
        if let Some(request) = self.latest.take().or(pending) {
            self.render_sync(ctx, request);
        }
    }

    fn disable_worker(&mut self) {
        self.worker = None;
        self.draining = None;
        #[cfg(test)]
        {
            self.controlled = None;
        }
        // `started` stays true: a disconnect never creates another worker thread.
    }

    fn render_sync(&mut self, ctx: &egui::Context, mut request: Request) {
        let sequence = self.next_sequence();
        let renderer = self.renderer.get_or_insert_with(Renderer::new);
        let t0 = crate::now_ms();
        request.placed_generation = vectorcraft_render::placed_document::generation();
        let images = request.jobs.each_ref().map(|job| render(renderer, job));
        self.publish(ctx, request, images, crate::now_ms() - t0, sequence);
    }

    fn publish(&mut self, ctx: &egui::Context, request: Request, images: [Rendered; 2], ms: f64, sequence: u64) {
        if sequence <= self.shown_sequence || !self.context.as_ref().is_some_and(|current| current.same_context(&request.identity)) {
            return;
        }
        if super::placed_changed(
            &request.jobs[0].doc,
            request.placed_generation,
            vectorcraft_render::placed_document::generation(),
            vectorcraft_render::placed_document::made_between,
        ) {
            // Neither side is uploaded. Keep the last coherent pair and ask for a fresh request.
            ctx.request_repaint();
            return;
        }
        let identity = request.identity;
        let options = identity.keys.each_ref().map(|key| if key.pixel.is_some() { TextureOptions::NEAREST } else { TextureOptions::LINEAR });
        let images = images.map(super::color_image_owned);
        let textures = match self.shown.take() {
            Some(mut pair) => {
                for ((texture, image), options) in pair.textures.iter_mut().zip(images).zip(options) {
                    texture.set(image, options);
                }
                pair.textures
            }
            None => {
                let [main, comparison] = images;
                let [a, b] = options;
                [ctx.load_texture("canvas-split-main", main, a), ctx.load_texture("canvas-split-comparison", comparison, b)]
            }
        };
        self.shown_sequence = sequence;
        self.shown = Some(Pair { keys: identity.keys.clone(), textures, ms });
        self.shown_identity = Some(identity);
    }

    fn has_worker(&self) -> bool {
        #[cfg(test)]
        if self.controlled.is_some() {
            return true;
        }
        self.worker.is_some()
    }

    fn completions(&self) -> Option<Vec<Done<CompletionKey>>> {
        #[cfg(test)]
        if let Some(controlled) = &self.controlled {
            return controlled.poll();
        }
        self.worker.as_ref().map_or_else(|| Some(vec![]), ThumbWorker::poll)
    }

    fn submit(&self, slot: u8, key: CompletionKey, run: JobFn) -> bool {
        #[cfg(test)]
        if let Some(controlled) = &self.controlled {
            return controlled.submit(slot, key, run);
        }
        self.worker.as_ref().is_some_and(|worker| worker.submit(slot, key, run))
    }

    fn discard_sides(&self) -> bool {
        #[cfg(test)]
        if let Some(controlled) = &self.controlled {
            return controlled.discard();
        }
        self.worker.as_ref().is_some_and(|worker| worker.discard(|slot| *slot < DRAIN_SLOT))
    }
}

fn render(renderer: &mut Renderer, job: &Job) -> Rendered {
    job.scope.clone().run(|| job.effects.view(|| super::render_job(renderer, &mut None, job))).img
}

fn clone_job(job: &Job) -> Job {
    Job {
        key: job.key.clone(),
        doc: job.doc.clone(),
        w: job.w,
        h: job.h,
        view: job.view,
        opts: job.opts.clone(),
        scope: job.scope.clone(),
        effects: job.effects.clone(),
        may_copy: false,
        may_keep: false,
        display: job.display.clone(),
    }
}

#[cfg(test)]
mod tests;
