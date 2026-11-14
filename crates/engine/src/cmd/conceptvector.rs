//! Concept to Vector (`conceptVector.*`): vector variations of one selected image, made by a
//! provider ([`crate::concept_vector`]), compared, and inserted beside the image.
//!
//! `conceptVector.generate` asks the named provider for one variation per style and keeps them as a
//! job of this session; it changes nothing in the document. `conceptVector.insert` puts the chosen
//! variations to the right of the image as one undo step. A job is kept until it is canceled, its
//! document closes, or [`KEEP_JOBS`] newer ones replace it; it is never saved.

use std::collections::VecDeque;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer, LineCap, LineJoin, StrokeLayer};
use vectorcraft_doc::{Appearance, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect};

use super::buildcmds::TRACE_NAME;
use super::edit::selected_roots;
use super::pathops::shape_node;
use super::{CommandSpec, EngineError, Result, Session, always, bad, checked_id, cmd, has_doc, text_param};
use crate::concept_vector::{self as cv, MAX_STYLES, Output, Provider, ProviderError, Request, Variation};

/// The most jobs a session keeps; a new one drops the oldest.
pub const KEEP_JOBS: usize = 4;

/// The gap, in points, between the image and the first inserted variation, and between variations.
pub const GAP: f64 = 24.0;

/// A job's variations, or that it was canceled.
enum Outcome {
    Done(Vec<Variation>),
    Canceled,
}

/// What `conceptVector.generate` made, and what it was made of.
struct Job {
    id: u64,
    /// The document ([`crate::DocState::uid`]) and the object (an image or an Image Trace object).
    doc: u64,
    source: NodeId,
    image: ImageObject,
    bytes: Arc<Vec<u8>>,
    provider: String,
    outcome: Outcome,
}

/// The session's providers and jobs.
pub(crate) struct ConceptState {
    pub(crate) registry: cv::Registry,
    jobs: VecDeque<Job>,
    next: u64,
}

impl Default for ConceptState {
    fn default() -> Self {
        Self { registry: cv::Registry::default(), jobs: VecDeque::new(), next: 1 }
    }
}

impl Session {
    /// The Concept to Vector providers this session calls by id.
    pub fn concept_providers(&self) -> &cv::Registry {
        &self.concept.registry
    }

    /// The providers, to register another ([`cv::Registry::register`]).
    pub fn concept_providers_mut(&mut self) -> &mut cv::Registry {
        &mut self.concept.registry
    }

    /// Job `job`'s variations and the size in pixels of the image they were made of; `None` for a
    /// job this session does not keep, or one that was canceled.
    pub fn concept_variations(&self, job: u64) -> Option<(&[Variation], (u32, u32))> {
        let j = self.concept.jobs.iter().find(|j| j.id == job)?;
        match &j.outcome {
            Outcome::Done(v) => Some((v.as_slice(), (j.image.width, j.image.height))),
            Outcome::Canceled => None,
        }
    }

    /// Drops the jobs whose document is closed.
    fn prune_concept_jobs(&mut self) {
        let open: Vec<u64> = self.documents().iter().map(|d| d.uid).collect();
        self.concept.jobs.retain(|j| open.contains(&j.doc));
    }
}

/// The image `n` shows: its own, or the source image of an Image Trace object that wasn't expanded.
fn shown_image(n: &Node) -> Option<&ImageObject> {
    match &n.kind {
        NodeKind::Image(i) => Some(i),
        NodeKind::Group { clip: false, .. } if n.name.as_deref() == Some(TRACE_NAME) => match &n.children()?.first()?.kind {
            NodeKind::Image(i) => Some(i),
            _ => None,
        },
        _ => None,
    }
}

const NO_SOURCE: &str = "select one image or an unexpanded Image Trace object";

const LOCKED: &str = "the image is locked or hidden";

/// The object `id` names, else the one selected: an image or an unexpanded Image Trace object,
/// shown and unlocked with every layer and group around it, and not on a locked artboard → it, its
/// image and the image's bytes.
fn source(s: &Session, c: &str, p: &Value) -> Result<(NodeId, ImageObject, Arc<Vec<u8>>)> {
    let id = match p.get("id").filter(|v| !v.is_null()) {
        Some(v) => checked_id(s, v, c)?,
        None => match selected_roots(s)?.as_slice() {
            [one] => *one,
            _ => return Err(bad(c, NO_SOURCE)),
        },
    };
    let d = &s.doc()?.doc;
    let n = d.node(id).ok_or(EngineError::NoNode(id))?;
    let image = shown_image(n).ok_or_else(|| bad(c, NO_SOURCE))?;
    if !d.is_editable(id) {
        return Err(bad(c, LOCKED));
    }
    let blob = d.images.get(&image.key).ok_or_else(|| EngineError::Other(format!("image data `{}` is missing", image.key)))?;
    Ok((id, image.clone(), Arc::clone(&blob.bytes)))
}

/// A generation prepared from the session, which runs without it ([`ConceptRun::run`]), on any
/// thread, and is kept as a job with [`ConceptRun::finish`].
pub struct ConceptRun {
    provider: Arc<dyn Provider>,
    /// The provider's id, as it was registered.
    provider_id: &'static str,
    request: Request,
    doc: u64,
    source: NodeId,
    image: ImageObject,
}

impl ConceptRun {
    /// What `conceptVector.generate` with params `p` makes in the active document, checked as the
    /// command checks it.
    pub fn prepare(s: &Session, p: &Value) -> Result<ConceptRun> {
        const C: &str = "conceptVector.generate";
        let name = text_param(p, "provider", C)?.ok_or_else(|| bad(C, "missing `provider` (see conceptVector.providers)"))?;
        let found =
            s.concept.registry.get(name).ok_or_else(|| bad(C, format!("no Concept to Vector provider `{name}` (see conceptVector.providers)")))?;
        let (caps, pid, provider) = (&found.capabilities, found.id, Arc::clone(&found.provider));
        let styles_error = || bad(C, format!("`styles` must list 1 to 8 different style ids of `{pid}` (see conceptVector.providers)"));
        let styles: Vec<String> = match p.get("styles") {
            Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<_>>().ok_or_else(styles_error)?,
            _ => return Err(styles_error()),
        };
        // Checks the count before the scan for repeats.
        if styles.is_empty() || styles.len() > MAX_STYLES {
            return Err(styles_error());
        }
        let known = |id: &String| caps.styles.iter().any(|st| st.id == id);
        let unique = styles.iter().enumerate().all(|(i, id)| !styles.get(..i).unwrap_or_default().contains(id));
        if !unique || !styles.iter().all(known) {
            return Err(styles_error());
        }
        let prompt = text_param(p, "prompt", C)?.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        // Results are vector paths, inserted as vector art, whatever else a provider makes.
        let output = match text_param(p, "output", C)? {
            None | Some("vector") => Output::Vector,
            Some(_) => return Err(bad(C, "`output` must be \"vector\"")),
        };
        let unsupported = (prompt.is_some() && !caps.prompt) || !caps.vector_output;
        if unsupported || !caps.image_input {
            return Err(bad(C, format!("`{pid}` takes no prompt and makes vector art only")));
        }
        let detail = match p.get("detail") {
            None | Some(Value::Null) => 50,
            Some(v) => v.as_u64().filter(|d| *d <= 100).ok_or_else(|| bad(C, "`detail` must be a whole number from 0 to 100"))? as u8,
        };
        let (source, image, bytes) = source(s, C, p)?;
        let request = Request { image: bytes, width: image.width, height: image.height, prompt, styles, detail, output };
        Ok(ConceptRun { provider, provider_id: pid, request, doc: s.doc()?.uid, source, image })
    }

    /// The document it was prepared in ([`crate::DocState::uid`]).
    pub fn doc(&self) -> u64 {
        self.doc
    }

    /// The image or Image Trace object it makes variations of.
    pub fn source(&self) -> NodeId {
        self.source
    }

    /// Asks the provider for the variations and checks them ([`cv::check_result`]), telling `steps`
    /// how far it has got; it touches no document. A panic in the provider is an error.
    pub fn run(&self, steps: cv::Steps) -> std::result::Result<Vec<Variation>, ProviderError> {
        let made = crate::guard::catch_panic(|| self.provider.generate(&self.request, steps)).map_err(ProviderError::Internal)??;
        cv::check_result(&self.request, &made)?;
        Ok(made)
    }

    /// How many styles it makes.
    pub fn style_count(&self) -> usize {
        self.request.styles.len()
    }

    /// Does the next part of the run kept in `run`, for a caller with no thread for
    /// [`ConceptRun::run`] (the web build), which can run other work, or stop, between the parts
    /// → whether every style is made ([`PartRun::made`]). A provider that makes parts
    /// ([`cv::Provider::parts`]) does one of its own; the others make one style alone, and the
    /// styles made so far are checked together ([`cv::check_result`]) after each one, so the
    /// totals stop the run as soon as they are passed. A panic in the provider is an error.
    pub fn run_part(&self, run: &mut PartRun, steps: cv::Steps) -> std::result::Result<bool, ProviderError> {
        let guard = ProviderError::Internal;
        if run.parts.is_none() {
            run.parts = Some(crate::guard::catch_panic(|| self.provider.parts(&self.request)).map_err(guard)?);
        }
        if let Some(Some(parts)) = &mut run.parts {
            run.done = crate::guard::catch_panic(|| parts.styles_done()).map_err(guard)?;
            if !steps(run.done as u64, self.style_count() as u64) {
                return Err(ProviderError::Canceled);
            }
            let Some(made) = crate::guard::catch_panic(|| parts.step()).map_err(guard)?? else {
                run.done = crate::guard::catch_panic(|| parts.styles_done()).map_err(guard)?;
                return Ok(false);
            };
            cv::check_result(&self.request, &made)?;
            run.made = made;
            return Ok(true);
        }
        let Some(style) = self.request.styles.get(run.made.len()) else { return Ok(true) };
        let one = Request { styles: vec![style.clone()], ..self.request.clone() };
        let next = crate::guard::catch_panic(|| self.provider.generate(&one, steps)).map_err(guard)??;
        cv::check_result(&one, &next)?;
        run.made.extend(next);
        let so_far = Request { styles: self.request.styles.get(..run.made.len()).unwrap_or_default().to_vec(), ..one };
        cv::check_result(&so_far, &run.made)?;
        Ok(run.made.len() == self.request.styles.len())
    }

    /// Keeps `variations` as a new job of `s` → the job, as `conceptVector.results` gives it;
    /// `None`, keeping nothing, when the document it was prepared in has closed.
    pub fn finish(self, s: &mut Session, variations: Vec<Variation>) -> Option<Value> {
        s.prune_concept_jobs();
        if !s.documents().iter().any(|d| d.uid == self.doc) {
            return None;
        }
        let id = s.concept.next;
        s.concept.next += 1;
        let job = Job {
            id,
            doc: self.doc,
            source: self.source,
            image: self.image,
            bytes: self.request.image,
            provider: self.provider_id.to_string(),
            outcome: Outcome::Done(variations),
        };
        let out = job_json(&s.concept.registry, &job);
        s.concept.jobs.push_back(job);
        while s.concept.jobs.len() > KEEP_JOBS {
            s.concept.jobs.pop_front();
        }
        Some(out)
    }
}

/// How far a run made a part at a time ([`ConceptRun::run_part`]) has got.
#[derive(Default)]
pub struct PartRun {
    /// The provider's parts, once asked for (`None` inside: it makes one style at a time).
    parts: Option<Option<Box<dyn cv::Parts>>>,
    /// The styles the provider's parts had made when last asked, inside the panic guard.
    done: usize,
    made: Vec<Variation>,
}

impl PartRun {
    /// The styles made so far.
    pub fn styles_done(&self) -> usize {
        if self.made.is_empty() { self.done } else { self.made.len() }
    }

    /// The variations made: all of them once [`ConceptRun::run_part`] has said so.
    pub fn made(self) -> Vec<Variation> {
        self.made
    }
}

/// `e` as the engine's error.
pub fn provider_error(e: ProviderError) -> EngineError {
    match e {
        ProviderError::Canceled => EngineError::Cancelled,
        e => EngineError::Other(e.to_string()),
    }
}

/// The English name of style `style` of provider `provider`, else the id.
fn style_label(registry: &cv::Registry, provider: &str, style: &str) -> String {
    let caps = registry.get(provider).map(|p| &p.capabilities);
    caps.and_then(|c| c.styles.iter().find(|s| s.id == style).map(|s| s.label.to_string())).unwrap_or_else(|| style.to_string())
}

/// Where `image`'s pixels lie in the document.
fn image_rect(image: &ImageObject) -> Option<Rect> {
    let r = Rect::new(0.0, 0.0, f64::from(image.width.max(1)), f64::from(image.height.max(1)));
    let pts = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| image.xf * kurbo::Point::new(x, y));
    let mut out = Rect::from_points(pts[0], pts[0]);
    for q in &pts[1..] {
        out = out.union_pt(*q);
    }
    [out.x0, out.y0, out.x1, out.y1].iter().all(|v| v.is_finite()).then_some(out)
}

fn job_json(registry: &cv::Registry, j: &Job) -> Value {
    let (state, variations) = match &j.outcome {
        Outcome::Done(v) => ("done", v.as_slice()),
        Outcome::Canceled => ("canceled", &[][..]),
    };
    let variations: Vec<Value> = variations
        .iter()
        .map(|v| {
            let bounds = v.paths.iter().filter_map(|p| p.path.bounds()).reduce(|a, b| a.union(b));
            json!({
                "style": v.style,
                "label": style_label(registry, &j.provider, &v.style),
                "paths": v.paths.len(),
                "anchors": v.anchors(),
                "bounds": bounds.map(|b| [b.x0, b.y0, b.x1, b.y1]),
                "warnings": v.warnings,
            })
        })
        .collect();
    json!({ "job": j.id, "provider": j.provider, "source": j.source.0, "state": state, "variations": variations })
}

/// The `job` param of command `c`.
fn job_param(c: &str, p: &Value) -> Result<u64> {
    p.get("job").and_then(Value::as_u64).ok_or_else(|| bad(c, "`job` must be a job number from conceptVector.generate"))
}

/// The error of command `c` for a job the session does not keep.
fn no_job(c: &str, job: u64) -> EngineError {
    bad(c, format!("no Concept to Vector job {job}: it was never made, or was dropped (see conceptVector.results)"))
}

/// The index of job `job` among those `s` keeps.
fn job_index(s: &mut Session, c: &str, job: u64) -> Result<usize> {
    s.prune_concept_jobs();
    s.concept.jobs.iter().position(|j| j.id == job).ok_or_else(|| no_job(c, job))
}

fn providers(s: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = s
        .concept
        .registry
        .iter()
        .map(|p| {
            let c = &p.capabilities;
            json!({
                "id": p.id,
                "name": p.name,
                "available": true,
                "offline": c.offline,
                "imageInput": c.image_input,
                "vectorOutput": c.vector_output,
                "prompt": c.prompt,
                "rasterOutput": c.raster_output,
                "maxStyles": MAX_STYLES,
                "styles": c.styles,
            })
        })
        .collect();
    let selected = s.concept.registry.iter().next().map(|p| p.id);
    Ok(json!({ "providers": list, "selected": selected }))
}

fn generate(s: &mut Session, p: &Value) -> Result<Value> {
    let run = ConceptRun::prepare(s, p)?;
    let made = run.run(&|_, _| true).map_err(provider_error)?;
    run.finish(s, made).ok_or(EngineError::NoDocument)
}

fn results(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "conceptVector.results";
    let job = job_param(C, p)?;
    let at = job_index(s, C, job)?;
    let j = s.concept.jobs.get(at).ok_or_else(|| no_job(C, job))?;
    Ok(job_json(&s.concept.registry, j))
}

fn cancel(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "conceptVector.cancel";
    let job = job_param(C, p)?;
    let at = job_index(s, C, job)?;
    if let Some(j) = s.concept.jobs.get_mut(at) {
        j.outcome = Outcome::Canceled;
    }
    Ok(json!({ "job": job, "state": "canceled" }))
}

/// The appearance of variation path `p`, its widths scaled by `unit` (points per pixel).
fn appearance_of(p: &cv::VectorPath, unit: f64) -> Appearance {
    let solid = |c: [u8; 3]| Paint::solid(Color::rgb8(c[0], c[1], c[2]));
    let mut items = vec![];
    if let Some(c) = p.fill {
        items.push(AppearanceItem::Fill(FillLayer::new(solid(c))));
    }
    if let Some((c, w)) = p.stroke {
        let cap = if p.square_caps { LineCap::Square } else { LineCap::Round };
        items.push(AppearanceItem::Stroke(StrokeLayer { cap, join: LineJoin::Round, ..StrokeLayer::new(solid(c), w * unit) }));
    }
    Appearance { items, ..Default::default() }
}

fn insert(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "conceptVector.insert";
    let job = job_param(C, p)?;
    let at = job_index(s, C, job)?;
    let list_error = || bad(C, format!("`variations` must list different style ids of job {job}"));
    let chosen: Vec<String> = match p.get("variations") {
        Some(Value::Array(a)) if !a.is_empty() => a.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<_>>().ok_or_else(list_error)?,
        _ => return Err(list_error()),
    };
    let stale = || bad(C, format!("job {job} no longer applies: its document, image or image placement changed, so generate again"));
    let (doc, source, image, bytes, picked, label_of) = {
        let j = s.concept.jobs.get(at).ok_or_else(|| no_job(C, job))?;
        let Outcome::Done(all) = &j.outcome else { return Err(bad(C, format!("job {job} has no results to insert: it was canceled"))) };
        // Checks the count before the scan for repeats.
        if chosen.len() > all.len() {
            return Err(list_error());
        }
        let unique = chosen.iter().enumerate().all(|(i, id)| !chosen.get(..i).unwrap_or_default().contains(id));
        // In the provider's order, whatever order they were listed in.
        let picked: Vec<Variation> = all.iter().filter(|v| chosen.contains(&v.style)).cloned().collect();
        if !unique || picked.len() != chosen.len() {
            return Err(list_error());
        }
        let labels: Vec<String> = picked.iter().map(|v| style_label(&s.concept.registry, &j.provider, &v.style)).collect();
        (j.doc, j.source, j.image.clone(), Arc::clone(&j.bytes), picked, labels)
    };
    // The same document, the same object showing the same pixels where they were.
    let st = s.doc()?;
    let now = st.doc.node(source).and_then(shown_image);
    let same_pixels = || st.doc.images.get(&image.key).is_some_and(|b| Arc::ptr_eq(&b.bytes, &bytes) || *b.bytes == *bytes);
    let current = st.uid == doc && now.is_some_and(|i| i.key == image.key && i.xf == image.xf && i.width == image.width && i.height == image.height);
    if !current || !same_pixels() {
        return Err(stale());
    }
    if !st.doc.is_editable(source) {
        return Err(bad(C, LOCKED));
    }
    // The results go above the outermost group around the image that clips or masks it, so that
    // it hides none of them; a layer that clips or masks its art leaves them nowhere to go.
    let mut anchor = source;
    for &id in st.doc.ancestry(source).ok_or_else(stale)?.iter().rev().skip(1) {
        let n = st.doc.node(id).ok_or_else(stale)?;
        let masked = n.extras().mask.as_ref().is_some_and(|m| !m.disabled);
        if matches!(n.kind, NodeKind::Layer { clip: true, .. }) {
            return Err(bad(C, "the image is on a layer with a clipping mask, which would hide the results"));
        }
        if n.is_layer() && masked {
            return Err(bad(C, "the image is on a layer with an opacity mask, which would hide the results"));
        }
        if !n.is_layer() && (masked || matches!(n.kind, NodeKind::Group { clip: true, .. })) {
            anchor = id;
        }
    }
    let src = image_rect(&image).ok_or_else(stale)?;
    let unit = image.xf.determinant().abs().sqrt();
    // Each chosen variation's box over the image: the image's box and the variation's painted
    // bounds, which may reach past it. A square cap's corner lies half the width along the path
    // and half across it from the end, so up to √2 times half the width past it on an axis.
    let boxes: Vec<Rect> = picked
        .iter()
        .map(|v| {
            v.paths
                .iter()
                .filter_map(|vp| {
                    let reach = if vp.square_caps { std::f64::consts::SQRT_2 } else { 1.0 };
                    let half = vp.stroke.map_or(0.0, |(_, w)| w * unit / 2.0 * reach);
                    vp.path.transformed(image.xf).bounds().map(|b| b.inflate(half, half))
                })
                .fold(src, |a, b| a.union(b))
        })
        .collect();
    // How far each variation's box is moved past the first's: each box starts GAP to the right of
    // the one before it (`width + GAP` further right for boxes the image's size).
    let mut offsets = Vec::with_capacity(boxes.len());
    let mut next = 0.0;
    for (k, b) in boxes.iter().enumerate() {
        if let Some(prev) = k.checked_sub(1).and_then(|j| boxes.get(j)) {
            next += prev.x1 - b.x0 + GAP;
        }
        offsets.push(next);
    }
    // The first box's left edge, which may lie left of the image's where its variation paints there.
    let first = boxes.first().map_or(src.x0, |b| b.x0);
    // The boxes with the first one's left edge moved to `x0`, the others after it.
    let row = |x0: f64| -> Vec<Rect> {
        boxes
            .iter()
            .zip(&offsets)
            .map(|(b, off)| {
                let dx = x0 + off - first;
                Rect::new(b.x0 + dx, b.y0, b.x1 + dx, b.y1)
            })
            .collect()
    };
    // The painted bounds of every visible object (the source's too: an Image Trace object's traced
    // art and effects can reach past the image), and the image's own box. A layer, and a group that
    // paints nothing of its own, is measured by its members (an opacity mask only hides them); one
    // that clips, by its members after the clipping path, cut to what it paints.
    let mut obstacles: Vec<Rect> = vec![src];
    let mut stack: Vec<(&Arc<Node>, Option<Rect>)> = st.doc.layers.iter().map(|l| (l, None)).collect();
    while let Some((node, within)) = stack.pop() {
        if !node.visible {
            continue;
        }
        let cut = |b: Rect| match within {
            Some(w) => Some(b.intersect(w)).filter(|i| i.width() > 0.0 && i.height() > 0.0),
            None => Some(b),
        };
        let painted = || vectorcraft_render::painted_bounds(&st.doc, node).and_then(cut);
        if node.id == source {
            obstacles.extend(painted());
            continue;
        }
        let clips = matches!(node.kind, NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. });
        let by_members = node.is_layer()
            || (matches!(node.kind, NodeKind::Group { .. }) && node.appearance.items.is_empty() && node.appearance.effects.is_empty());
        if by_members {
            // A clipping path paints its fills and strokes when they are set (`Node::clip_paint`),
            // its strokes outside the clip too, whether or not it is shown.
            if clips && let Some(path) = node.children().into_iter().flatten().next() {
                let paint = path.clip_paint();
                for part in [paint.fill, paint.stroke].into_iter().flatten() {
                    if let Some(b) = vectorcraft_render::painted_bounds(&st.doc, &part).and_then(cut) {
                        obstacles.push(b);
                    }
                }
            }
            let within = if clips {
                // Nothing of a clipping group shows outside what it paints.
                let Some(shown) = painted() else { continue };
                Some(shown)
            } else {
                within
            };
            stack.extend(node.children().into_iter().flatten().skip(usize::from(clips)).map(|c| (c, within)));
        } else if let Some(b) = painted() {
            obstacles.push(b);
        }
    }
    let meets = |a: &Rect, b: &Rect| a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;
    // Only objects level with some box can be in the row's way.
    let top = boxes.iter().map(|b| b.y0).fold(src.y0, f64::min);
    let bottom = boxes.iter().map(|b| b.y1).fold(src.y1, f64::max);
    obstacles.retain(|o| o.y0 < bottom && top < o.y1);
    // By their left edges. The row only moves right, so only the objects between the row's left
    // and right edges are looked at: those it has passed are dropped, and the next ones join as
    // the row reaches them.
    obstacles.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    let mut joined = 0;
    let mut near: Vec<Rect> = vec![];
    let mut x0 = src.x1 + GAP;
    let mut placed = false;
    // Each move takes one box past one obstacle for good, as the row only moves right.
    for _ in 0..=obstacles.len() * boxes.len() {
        let at = row(x0);
        let left = at.iter().map(|r| r.x0).fold(f64::INFINITY, f64::min);
        let right = at.iter().map(|r| r.x1).fold(f64::NEG_INFINITY, f64::max);
        while let Some(o) = obstacles.get(joined).filter(|o| o.x0 < right) {
            near.push(*o);
            joined += 1;
        }
        near.retain(|o| o.x1 > left);
        // The first box in the way, moved past the rightmost object it meets.
        let shift = at.iter().find_map(|r| near.iter().filter(|o| meets(o, r)).map(|o| o.x1).reduce(f64::max).map(|right| right + GAP - r.x0));
        match shift {
            None => {
                placed = true;
                break;
            }
            Some(by) => x0 += by,
        }
    }
    let right = row(x0).iter().map(|r| r.x1).fold(f64::NEG_INFINITY, f64::max);
    let room = || bad(C, "there is no room for the results to the right of the image");
    if !placed || !x0.is_finite() || right > crate::MAX_COORD {
        return Err(room());
    }
    let ids = s.edit("Concept to Vector", |d, sel| {
        let mut groups = vec![];
        for ((v, label), off) in picked.into_iter().zip(label_of).zip(&offsets) {
            let dx = x0 + off - first;
            let xf = Affine::translate((dx, 0.0)) * image.xf;
            let mut members = Vec::with_capacity(v.paths.len());
            for vp in &v.paths {
                let path = vp.path.transformed(xf);
                if path.bounds().is_some_and(|b| [b.x0, b.y0, b.x1, b.y1].iter().any(|c| c.abs() > crate::MAX_COORD)) {
                    return Err(room());
                }
                let mut node = shape_node(d, path, None);
                node.appearance = appearance_of(vp, unit).into();
                members.push(Arc::new(node));
            }
            let mut g = Node::group(d.alloc_id(), members);
            g.name = Some(label);
            groups.push(g);
        }
        let ids: Vec<NodeId> = groups.iter().map(|g| g.id).collect();
        d.insert_beside(vec![(anchor, groups)], false)?;
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "conceptVector.providers",
            "Concept to Vector Providers",
            [],
            None,
            "{} → {providers: [{id, name, available, offline, imageInput, vectorOutput, prompt, rasterOutput, maxStyles: 8, styles: [{id, label}]}], selected: the id the panel uses} the providers conceptVector.generate can call by id; the built-in offline.traceStyles makes its styles with Image Trace on this computer (outline, inkLines, flatColor, detailedColor, grayTones), takes no prompt and makes vector art only",
            always,
            providers
        ),
        cmd!(
            query "conceptVector.generate",
            "Generate Concept to Vector Styles",
            [],
            None,
            "{id?: image or unexpanded Image Trace object (default: the one selected object), provider: id (conceptVector.providers; no other provider is used in its place), styles: [style id…] (1 to 8 different ids of that provider), detail?: 0-100 (50: how much of the image's detail each style keeps), prompt?: text (an error for a provider without prompts unless empty), output?: \"vector\" (the only output; any other value is an error)} one variation of the image for each style, kept by this session as a job and not put in the document; an object that is locked or hidden, itself or by a layer or group around it, or on a locked artboard, is refused → {job, provider, source: id, state: \"done\", variations: [{style, label, paths, anchors, bounds: [x0,y0,x1,y1] in the image's pixels, warnings: [text…]}]}",
            has_doc,
            generate
        ),
        cmd!(
            query "conceptVector.results",
            "Concept to Vector Results",
            [],
            None,
            "{job} a job conceptVector.generate made, as it returned it, or with state \"canceled\" and no variations; the session keeps the 4 newest jobs, until their document closes → {job, provider, source, state, variations}",
            has_doc,
            results
        ),
        cmd!(
            query "conceptVector.cancel",
            "Cancel Concept to Vector Job",
            [],
            None,
            "{job} drop a job's variations, so that it can't be inserted; canceling it again does the same → {job, state: \"canceled\"}",
            has_doc,
            cancel
        ),
        cmd!(
            "conceptVector.insert",
            "Insert Concept to Vector Styles",
            [],
            None,
            "{job, variations: [style id…] (different styles of the job)} put a group of each chosen variation, named after its style and in the provider's order, in a row to the right of the job's image, each the image's size with 24 pt between them, moved further right past other visible art in their way, where a variation reaches past its box too (the next one then starts 24 pt past what it paints); the groups go above the outermost clip group or opacity-masked group around the image; the image and other art stay as they are; one undo step; an error when the job's document is not the active one, the object, its image or its placement changed, the object is now locked or hidden, or it is on a layer with a clipping mask or an opacity mask → {ids: the groups, selected}",
            has_doc,
            insert
        ),
    ]
}
