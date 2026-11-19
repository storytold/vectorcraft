use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Weak};

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Affine;

use super::*;

pub(super) struct Controlled {
    tx: Sender<Command>,
    rx: Receiver<Done<CompletionKey>>,
}

enum Command {
    Submit(Submission),
    Discard,
}

struct Submission {
    slot: u8,
    key: CompletionKey,
    run: JobFn,
}

struct Probe {
    rx: Receiver<Command>,
    tx: Sender<Done<CompletionKey>>,
}

impl Controlled {
    pub(super) fn submit(&self, slot: u8, key: CompletionKey, run: JobFn) -> bool {
        self.tx.send(Command::Submit(Submission { slot, key, run })).is_ok()
    }

    pub(super) fn discard(&self) -> bool {
        self.tx.send(Command::Discard).is_ok()
    }

    pub(super) fn poll(&self) -> Option<Vec<Done<CompletionKey>>> {
        let mut done = vec![];
        loop {
            match self.rx.try_recv() {
                Ok(value) => done.push(value),
                Err(TryRecvError::Empty) => return Some(done),
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

fn controlled() -> (SplitCache, Probe) {
    let (tx, rx) = channel();
    let (done, output) = channel();
    let worker = Controlled { tx, rx: output };
    (SplitCache { started: true, controlled: Some(worker), ..Default::default() }, Probe { rx, tx: done })
}

fn key(revision: u64) -> CacheKey {
    CacheKey {
        doc: 1,
        revision,
        interacting: false,
        dragging: false,
        zoom: 1.0,
        cx: 4.0,
        cy: 4.0,
        w: 8,
        h: 8,
        outline: false,
        trim: false,
        templates: true,
        guides: true,
        ppp: 1.0,
        hidden: vec![],
        rot: 0.0,
        anti_alias: true,
        highlight_alternate_glyphs: false,
        highlight_substituted_fonts: true,
        placed: 0,
        plugins: 0,
        pixel: None,
        smooth_images: true,
        isolated: None,
        rich_black: false,
        guide: ([20, 30, 40], false),
        trace_source: None,
        display: 0,
        backdrop: None,
    }
}

fn jobs(revision: u64) -> [Job; 2] {
    let mut session = Session::new();
    session.execute("file.new", &json!({"width": 8, "height": 8})).unwrap();
    let doc = session.active().unwrap().doc.clone();
    [false, true].map(|outline| Job {
        key: CacheKey { outline, ..key(revision) },
        doc: doc.clone(),
        w: 8,
        h: 8,
        view: Affine::IDENTITY,
        opts: RenderOptions { outline, ..Default::default() },
        scope: vectorcraft_engine::file_access::ScopeHandle::capture(),
        effects: Default::default(),
        may_copy: true,
        may_keep: true,
        display: None,
    })
}

fn image(red: u8) -> Rendered {
    Rendered { width: 8, height: 8, pixels: [red, 0, 0, 255].repeat(64) }
}

fn uploaded_colors(ctx: &egui::Context, pair: &Pair) -> [u8; 2] {
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    let colors = pair.textures.each_ref().map(|texture| {
        let delta = out.textures_delta.set.iter().find(|(id, _)| **id == texture.id()).unwrap().1.last().unwrap();
        let egui::ImageData::Color(image) = &delta.image;
        image.pixels[0].r()
    });
    // The assertions consume these uploads; this controller fixture has no painter.
    out.textures_delta.clear();
    colors
}

impl Probe {
    fn pair(&self) -> [Submission; 2] {
        let read = || match self.rx.try_recv().unwrap() {
            Command::Submit(job) => job,
            Command::Discard => panic!("expected the current pair, not a discard"),
        };
        let pair = [read(), read()];
        assert_eq!((pair[0].slot, pair[1].slot), (0, 1));
        assert_eq!(pair[0].key.sequence, pair[1].key.sequence);
        assert_eq!(pair[0].key.epoch, pair[1].key.epoch);
        pair
    }

    fn barrier(&self) -> Submission {
        assert!(matches!(self.rx.try_recv().unwrap(), Command::Discard));
        let Command::Submit(barrier) = self.rx.try_recv().unwrap() else { panic!("one drain barrier") };
        assert_eq!(barrier.slot, DRAIN_SLOT);
        barrier
    }

    fn empty(&self) {
        assert!(matches!(self.rx.try_recv(), Err(TryRecvError::Empty)));
    }

    fn finish(&self, key: CompletionKey, red: Option<u8>) {
        self.tx.send(Done { key, img: red.map(image), ms: 3.0 }).unwrap_or_else(|_| panic!("current completion receiver"));
    }

    fn render(&self, job: Submission) {
        let img = (job.run)(&mut Renderer::new());
        assert!(img.is_some());
        self.tx.send(Done { key: job.key, img, ms: 3.0 }).unwrap_or_else(|_| panic!("current completion receiver"));
    }
}

#[test]
fn split_cache_publishes_only_complete_pairs_and_identical_requests_submit_nothing() {
    let ctx = egui::Context::default();
    let (mut cache, probe) = controlled();
    cache.update(&ctx, jobs(1));
    let [a, b] = probe.pair();
    probe.finish(b.key, Some(20));
    cache.update(&ctx, jobs(1));
    assert!(cache.shown().is_none());
    probe.empty();
    // An invalid side and a duplicate valid side cannot finish or replace the pair.
    probe.finish(CompletionKey { side: 9, ..a.key }, None);
    probe.finish(CompletionKey { sequence: 0, ..a.key }, Some(99));
    probe.finish(b.key, Some(90));
    probe.finish(a.key, Some(10));
    cache.update(&ctx, jobs(1));
    let shown = cache.shown().unwrap();
    assert_eq!(shown.keys.each_ref().map(|key| key.revision), [1, 1]);
    assert_eq!(shown.ms, 6.0);
    assert_eq!(uploaded_colors(&ctx, shown), [10, 20], "side order and first completion survive reverse delivery/duplicates");
    let ids = shown.textures.each_ref().map(TextureHandle::id);
    cache.update(&ctx, jobs(1));
    assert_eq!(cache.shown().unwrap().textures.each_ref().map(TextureHandle::id), ids);
    probe.empty();
    cache.update(&ctx, jobs(2));
    let [a, b] = probe.pair();
    probe.render(a);
    probe.render(b);
    cache.update(&ctx, jobs(2));
    assert_eq!(cache.shown().unwrap().textures.each_ref().map(TextureHandle::id), ids, "complete replacement reuses both handles");
}

#[test]
fn split_cache_coalesces_continuous_edits_but_presents_finished_compatible_pairs() {
    let ctx = egui::Context::default();
    let (mut cache, probe) = controlled();
    cache.update(&ctx, jobs(1));
    let first = probe.pair();
    cache.update(&ctx, jobs(2));
    cache.update(&ctx, jobs(3));
    probe.empty();
    for job in first.into_iter().rev() {
        probe.finish(job.key, Some(1));
    }
    cache.update(&ctx, jobs(3));
    assert_eq!(cache.shown().unwrap().keys[0].revision, 1, "continuous edits cannot starve all presentation");
    let next = probe.pair();
    assert_eq!(cache.pending.as_ref().unwrap().request.identity.keys[0].revision, 3);
    for job in next {
        probe.finish(job.key, Some(3));
    }
    cache.update(&ctx, jobs(3));
    assert_eq!(cache.shown().unwrap().keys.each_ref().map(|key| key.revision), [3, 3]);
    probe.empty();
}

#[test]
fn split_cache_context_changes_reject_completed_old_mode_and_document_frames() {
    let ctx = egui::Context::default();
    let (mut cache, probe) = controlled();
    cache.update(&ctx, jobs(1));
    let old = probe.pair();
    let mut changed = jobs(2);
    changed[1].key.outline = false;
    changed[1].opts.outline = false;
    cache.update(&ctx, changed);
    let barrier = probe.barrier();
    for job in old {
        probe.finish(job.key, Some(1));
    }
    probe.finish(barrier.key, None);
    let mut changed = jobs(2);
    changed[1].key.outline = false;
    changed[1].opts.outline = false;
    cache.update(&ctx, changed);
    assert!(cache.shown().is_none());
    let current = probe.pair();
    for job in current {
        probe.finish(job.key, Some(2));
    }
    let mut another_document = jobs(3);
    for job in &mut another_document {
        job.key.doc = 2;
    }
    cache.update(&ctx, another_document);
    assert!(cache.shown().is_none(), "check the incoming doc before polling a completed old pair");
    let barrier = probe.barrier();
    probe.finish(barrier.key, None);
    let mut another_document = jobs(3);
    for job in &mut another_document {
        job.key.doc = 2;
    }
    cache.update(&ctx, another_document);
    assert!(cache.shown().is_none());
    let current = probe.pair();
    for job in current {
        probe.finish(job.key, Some(3));
    }
    let mut another_document = jobs(3);
    for job in &mut another_document {
        job.key.doc = 2;
    }
    cache.update(&ctx, another_document);
    assert_eq!(cache.shown().unwrap().keys[0].doc, 2);
}

#[test]
fn split_cache_rapid_disable_and_enable_keep_one_barrier_and_only_latest_snapshot() {
    let ctx = egui::Context::default();
    let (mut cache, probe) = controlled();
    cache.update(&ctx, jobs(1));
    let old = probe.pair();
    cache.clear();
    let barrier = probe.barrier();
    let mut released: Vec<Weak<vectorcraft_doc::Document>> = vec![];
    for revision in 2..8 {
        let pair = jobs(revision);
        released.push(Arc::downgrade(&pair[0].doc));
        cache.update(&ctx, pair);
        cache.clear();
        assert!(cache.shown().is_none());
        assert!(cache.pending.is_none());
        assert_eq!(cache.draining, Some(barrier.key), "clear does not invalidate or multiply the drain acknowledgement");
        probe.empty();
    }
    assert!(released.iter().all(|doc| doc.upgrade().is_none()), "intermediate UI snapshots own no queued closures");
    cache.update(&ctx, jobs(8));
    probe.empty();
    for job in old {
        probe.finish(job.key, Some(1));
    }
    probe.finish(barrier.key, None);
    cache.update(&ctx, jobs(8));
    assert!(cache.shown().is_none());
    let current = probe.pair();
    assert_eq!(cache.pending.as_ref().unwrap().request.identity.keys[0].revision, 8);
    for job in current {
        probe.finish(job.key, Some(8));
    }
    cache.clear();
    let barrier = probe.barrier();
    probe.finish(barrier.key, None);
    cache.clear();
    assert!(cache.draining.is_none());
    assert!(cache.latest.is_none());
    assert!(cache.pending.is_none());
    probe.empty();
}

#[test]
fn split_cache_explicit_mode_stamp_clear_drains_even_when_render_keys_would_match() {
    let ctx = egui::Context::default();
    let (mut cache, probe) = controlled();
    cache.update(&ctx, jobs(1));
    let obsolete = probe.pair();
    // Root's mode stamp calls clear for Preview/Pixel Preview transitions even below the
    // pixel-render threshold, where neither side has a different pixel region/options yet.
    cache.clear();
    let barrier = probe.barrier();
    cache.update(&ctx, jobs(1));
    probe.empty();
    for job in obsolete {
        probe.finish(job.key, Some(1));
    }
    probe.finish(barrier.key, None);
    cache.update(&ctx, jobs(1));
    assert!(cache.shown().is_none(), "the prior mode cannot upload despite exact matching render keys");
    let current = probe.pair();
    for job in current {
        probe.finish(job.key, Some(2));
    }
    cache.update(&ctx, jobs(1));
    assert_eq!(uploaded_colors(&ctx, cache.shown().unwrap()), [2, 2]);
}

#[test]
fn split_cache_context_ignores_only_the_frozen_motion_and_content_fields() {
    let a = Identity::of(&jobs(1));
    let mut b = a.clone();
    for key in &mut b.keys {
        key.revision = 9;
        key.interacting = true;
        key.dragging = true;
        key.zoom = 2.0;
        key.cx = 15.0;
        key.cy = 16.0;
        key.w = 9;
        key.h = 10;
        key.ppp = 2.0;
        key.rot = 0.5;
        key.placed = 3;
        key.plugins = 4;
        key.pixel = Some([0, 0, 8, 8]);
    }
    for options in &mut b.options {
        options.interacting = true;
        options.dragging = true;
    }
    assert!(a.same_context(&b));
    assert!(a != b, "exact request equality still distinguishes all those snapshots");
    for changed in [
        {
            let mut b = b.clone();
            b.keys[0].display = 7;
            b
        },
        {
            let mut b = b.clone();
            b.keys[0].hidden.push(5);
            b
        },
        {
            let mut b = b.clone();
            b.options[1].overprint_preview = true;
            b
        },
    ] {
        assert!(!a.same_context(&changed));
    }
}

#[test]
fn split_cache_rejects_mixed_snapshots_but_allows_pixel_side_render_dimensions() {
    let mut pair = jobs(1);
    pair[1].key.pixel = Some([0, 0, 4, 4]);
    pair[1].w = 4;
    pair[1].h = 4;
    pair[1].view = Affine::scale(0.5);
    assert!(Request::new(pair).is_some());
    let mut pair = jobs(1);
    pair[1].key.revision = 2;
    assert!(Request::new(pair).is_none());
    let mut pair = jobs(1);
    pair[1].key.cx = 9.0;
    assert!(Request::new(pair).is_none());
    let mut pair = jobs(1);
    pair[1].doc = jobs(1)[1].doc.clone();
    assert!(Request::new(pair).is_none(), "equal caller keys do not make two different Doc snapshots coherent");
}

#[test]
fn split_cache_render_error_and_disconnection_fall_back_to_the_current_request() {
    let ctx = egui::Context::default();
    let (mut cache, probe) = controlled();
    cache.update(&ctx, jobs(1));
    let old = probe.pair();
    cache.update(&ctx, jobs(2));
    probe.finish(old[0].key, None);
    cache.update(&ctx, jobs(2));
    assert_eq!(cache.shown().unwrap().keys[0].revision, 2);
    assert!(cache.renderer.is_some(), "the fallback used the actual scoped render path");
    assert!(!cache.has_worker());
    assert!(cache.started);
    assert!(cache.pending.is_none());
    cache.update(&ctx, jobs(3));
    assert_eq!(cache.shown().unwrap().keys[0].revision, 3);
    assert!(!cache.has_worker(), "do not respawn after failure");

    let (mut cache, probe) = controlled();
    cache.update(&ctx, jobs(1));
    let _pair = probe.pair();
    drop(probe.tx);
    cache.update(&ctx, jobs(2));
    assert_eq!(cache.shown().unwrap().keys[0].revision, 2);
    assert!(cache.pending.is_none());
    assert!(!cache.has_worker());

    let (mut cache, probe) = controlled();
    drop(probe.rx);
    cache.update(&ctx, jobs(1));
    assert!(cache.shown().is_some(), "a failed submit also falls back finitely");
    assert!(!cache.has_worker());
}

#[test]
fn split_cache_synchronous_default_is_lazy_and_releases_textures_on_clear() {
    let ctx = egui::Context::default();
    let mut cache = SplitCache::default();
    assert!(cache.renderer.is_none());
    cache.update(&ctx, jobs(1));
    assert!(cache.renderer.is_some());
    assert_eq!(cache.shown().unwrap().textures[0].size(), [8, 8]);
    assert!(cache.pending.is_none());
    cache.clear();
    assert!(cache.shown().is_none());
    assert!(cache.renderer.is_none());
    cache.update(&ctx, jobs(1));
    assert!(cache.shown().is_some());
    assert!(cache.worker.is_none(), "unit-test fallback never starts a worker");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn split_cache_real_worker_drains_before_reusing_side_slots() {
    let ctx = egui::Context::default();
    let worker = ThumbWorker::start(&ctx, "split-pair-test").expect("one test worker");
    let (began, begun) = channel();
    let (release, held) = channel();
    let request = Request::new(jobs(1)).unwrap();
    let mut cache =
        SplitCache { worker: Some(worker), started: true, epoch: 1, sequence: 1, context: Some(request.identity.clone()), ..Default::default() };
    let a = CompletionKey { epoch: 1, sequence: 1, side: 0 };
    let b = CompletionKey { side: 1, ..a };
    let first = clone_job(&request.jobs[0]);
    let second = clone_job(&request.jobs[1]);
    assert!(cache.submit(
        0,
        a,
        Box::new(move |renderer| {
            began.send(()).unwrap();
            held.recv().unwrap();
            Some(render(renderer, &first))
        })
    ));
    assert!(cache.submit(1, b, Box::new(move |renderer| Some(render(renderer, &second)))));
    cache.pending = Some(Batch { epoch: 1, sequence: 1, request, images: [None, None], ms: 0.0 });
    begun.recv().unwrap(); // actual first render is held, no sleep-based ordering
    cache.clear();
    let barrier = cache.draining.unwrap();
    for revision in 2..6 {
        cache.update(&ctx, jobs(revision));
        assert_eq!(cache.draining, Some(barrier));
        assert!(cache.pending.is_none());
    }
    release.send(()).unwrap();
    let done = cache.worker.as_ref().unwrap().wait().unwrap();
    assert_eq!(done.key, a, "only the running obsolete side finishes");
    let done = cache.worker.as_ref().unwrap().wait().unwrap();
    assert_eq!(done.key, barrier, "queued obsolete side was discarded before this acknowledgement");
    assert!(done.img.is_none(), "a barrier acknowledgement is not a render failure");
    cache.draining = None;
    cache.start_latest(&ctx);
    let first = cache.worker.as_ref().unwrap().wait().unwrap();
    let second = cache.worker.as_ref().unwrap().wait().unwrap();
    assert_eq!((first.key.side, second.key.side), (0, 1));
    assert_eq!(first.key.sequence, second.key.sequence);
    assert!(cache.accept(first));
    assert!(cache.accept(second));
    cache.poll(&ctx);
    assert_eq!(cache.shown().unwrap().keys.each_ref().map(|key| key.revision), [5, 5]);
    cache.clear();
}

fn review_597943_camera(jobs: &[Job; 2], x: f64) -> [Job; 2] {
    jobs.each_ref().map(|job| {
        let mut job = clone_job(job);
        job.key.cx += x;
        job.view = Affine::translate((-x, 0.0)) * job.view;
        job
    })
}

fn review_597943_step(cache: &mut SplitCache, ctx: &egui::Context, time: f64, jobs: [Job; 2]) {
    let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |_| {});
    out.textures_delta.clear();
    cache.update(ctx, jobs);
}

fn review_597943_held(cache: &SplitCache, keys: &[CacheKey; 2], sequence: (u64, u64), textures: [egui::TextureId; 2]) {
    let pair = cache.shown().unwrap();
    assert!(pair.keys == *keys, "the complete old pair remains available for reprojection");
    assert_eq!((cache.sequence, cache.shown_sequence), sequence, "waiting must neither render nor allocate a sequence");
    assert_eq!(pair.textures.each_ref().map(|t| t.id()), textures);
    assert!(cache.pending.is_none());
    assert!(cache.latest.is_some(), "one newest desired pair stays queued during the delay");
}

fn review_597943_immediate_requests(base: &[Job; 2]) -> Vec<(&'static str, [Job; 2])> {
    let mut edit = review_597943_camera(base, 4.0);
    for job in &mut edit {
        job.key.revision += 1;
    }
    let mut document = review_597943_camera(base, 4.0);
    for job in &mut document {
        job.key.doc += 1;
    }
    let mut placed = review_597943_camera(base, 4.0);
    for job in &mut placed {
        job.key.placed += 1;
    }
    let mut plugins = review_597943_camera(base, 4.0);
    // The first side is camera-only: the second side must independently block deferral.
    plugins[1].key.plugins += 1;
    let mut mode = review_597943_camera(base, 4.0);
    mode[1].key.outline = false;
    mode[1].opts.outline = false;
    let mut proof = review_597943_camera(base, 4.0);
    for job in &mut proof {
        job.opts.proof = Some(Default::default());
    }
    let mut options = review_597943_camera(base, 4.0);
    // Context compatibility ignores this flag, but rendering options must still match exactly.
    options[1].opts.interacting = true;
    vec![
        ("revision", edit),
        ("document", document),
        ("placed", placed),
        ("second-side plugins", plugins),
        ("mode", mode),
        ("proof", proof),
        ("options", options),
    ]
}

fn review_597943_disconnected_pair_waits(ctx: &egui::Context, base: &[Job; 2]) {
    let (mut cache, probe) = controlled();
    review_597943_step(&mut cache, ctx, 50.0, review_597943_camera(base, 0.0));
    for submission in probe.pair() {
        probe.render(submission);
    }
    review_597943_step(&mut cache, ctx, 50.01, review_597943_camera(base, 0.0));
    cache.shown.as_mut().unwrap().ms = 9.0;
    let keys = cache.shown().unwrap().keys.clone();
    let sequence = (cache.sequence, cache.shown_sequence);
    let textures = cache.shown().unwrap().textures.each_ref().map(|t| t.id());
    drop(probe);
    review_597943_step(&mut cache, ctx, 50.02, review_597943_camera(base, 1.0));
    assert!(!cache.has_worker(), "the real completion-channel failure enters synchronous fallback");
    review_597943_held(&cache, &keys, sequence, textures);
    review_597943_step(&mut cache, ctx, 50.23, review_597943_camera(base, 1.0));
    assert_eq!(cache.shown().unwrap().keys[0].cx, base[0].key.cx + 1.0);
    assert!(cache.latest.is_none());
}

#[test]
fn review_597943_synchronous_pairs_wait_only_for_slow_camera_changes() {
    let ctx = egui::Context::default();
    let base = jobs(1);
    let mut cache = SplitCache::default();
    review_597943_step(&mut cache, &ctx, 10.0, review_597943_camera(&base, 0.0));
    assert_eq!((cache.sequence, cache.shown_sequence), (1, 1), "the first real tiny pair renders at once");
    assert!(cache.worker.is_none() && cache.pending.is_none());
    cache.shown.as_mut().unwrap().ms = 8.0;
    review_597943_step(&mut cache, &ctx, 10.01, review_597943_camera(&base, 1.0));
    assert_eq!((cache.sequence, cache.shown_sequence), (2, 2), "the fast/8ms boundary stays immediate");
    // A declared observed duration tests the policy threshold, not elapsed benchmark performance.
    cache.shown.as_mut().unwrap().ms = 9.0;
    let keys = cache.shown().unwrap().keys.clone();
    let sequence = (cache.sequence, cache.shown_sequence);
    let textures = cache.shown().unwrap().textures.each_ref().map(|t| t.id());
    for (time, camera) in [(11.0, 2.0), (11.1, 3.0), (11.29, 3.0)] {
        review_597943_step(&mut cache, &ctx, time, review_597943_camera(&base, camera));
        review_597943_held(&cache, &keys, sequence, textures);
        assert_eq!(cache.latest.as_ref().unwrap().identity.keys[0].cx, base[0].key.cx + camera);
    }
    review_597943_step(&mut cache, &ctx, 11.31, review_597943_camera(&base, 3.0));
    assert_eq!(cache.shown().unwrap().keys[0].cx, base[0].key.cx + 3.0, "settling renders only the newest desired camera");
    assert_eq!((cache.sequence, cache.shown_sequence), (sequence.0 + 1, sequence.1 + 1));
    assert!(cache.latest.is_none());

    cache.shown.as_mut().unwrap().ms = 9.0;
    let shown = cache.shown().unwrap().keys.clone();
    let sequence = (cache.sequence, cache.shown_sequence);
    let textures = cache.shown().unwrap().textures.each_ref().map(|t| t.id());
    review_597943_step(&mut cache, &ctx, 12.0, review_597943_camera(&base, 4.0));
    review_597943_held(&cache, &shown, sequence, textures);
    review_597943_step(&mut cache, &ctx, 12.1, review_597943_camera(&base, 3.0));
    assert!(cache.latest.is_none(), "returning to the shown camera withdraws the pending request");
    review_597943_step(&mut cache, &ctx, 12.3, review_597943_camera(&base, 4.0));
    review_597943_held(&cache, &shown, sequence, textures);
    review_597943_step(&mut cache, &ctx, 12.51, review_597943_camera(&base, 4.0));
    assert_eq!(cache.shown().unwrap().keys[0].cx, base[0].key.cx + 4.0, "a revisited camera gets a fresh stability interval");

    for (label, changed) in review_597943_immediate_requests(&base) {
        let mut cache = SplitCache::default();
        review_597943_step(&mut cache, &ctx, 20.0, review_597943_camera(&base, 0.0));
        cache.shown.as_mut().unwrap().ms = 9.0;
        let sequence = cache.sequence;
        let expected = Identity::of(&changed);
        review_597943_step(&mut cache, &ctx, 20.01, changed);
        assert!(cache.shown_identity.as_ref() == Some(&expected), "{label} changes render immediately despite camera motion");
        assert_eq!(cache.sequence, sequence + 1);
        assert!(cache.latest.is_none());
    }
    cache.shown.as_mut().unwrap().ms = 9.0;
    review_597943_step(&mut cache, &ctx, 30.0, review_597943_camera(&base, 5.0));
    assert!(cache.latest.is_some());
    let sequence = cache.sequence;
    review_597943_step(&mut cache, &ctx, 29.0, review_597943_camera(&base, 5.0));
    assert_eq!(cache.sequence, sequence + 1, "a backward clock does not wait for its previous timestamp");
    cache.shown.as_mut().unwrap().ms = 9.0;
    review_597943_step(&mut cache, &ctx, 31.0, review_597943_camera(&base, 6.0));
    assert!(cache.latest.is_some());
    cache.clear();
    assert!(cache.latest.is_none() && cache.shown().is_none());
    review_597943_step(&mut cache, &ctx, 31.01, review_597943_camera(&base, 6.0));
    assert!(cache.shown().is_some() && cache.latest.is_none(), "clear restores first-pair immediate rendering");
    review_597943_disconnected_pair_waits(&ctx, &base);
}
