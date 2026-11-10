//! Live-effect plug-ins: a plug-in of kind `effect` rewrites one object's geometry each time it is
//! drawn, from its appearance stack (an `Effect` record with id `plugin.<plug-in id>`).
//!
//! The plug-in is given the geometry and, read only, the object's fills and strokes as a filter
//! sees them ([`Paints`]): an effect can draw as wide as the stroke it sits on, or tell its fills
//! apart, without asking for them again as parameters.
//!
//! An effect whose manifest declares `"paintOutput": true` returns painted pieces instead
//! ([`apply_painted`], [`crate::painted`]): each is drawn by itself with paints of its own.
//!
//! Drawing evaluates effects often (every frame, hit tests, bounds), so results are cached by
//! plug-in installation, parameters, input geometry and paints: the plug-in runs again only when
//! one of them changes. Failures are cached too (the geometry is then drawn unchanged) and
//! reported by [`last_error`]. The swatches and patterns a painted result names are checked
//! against the document each time it is used.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, AppearanceItem, Document};
use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::objects::{decode, output_geometry, paint_lists};
use crate::painted::{self, Piece, Scope};
use crate::{Error, Kind, Limits, Result, registry};

/// The effect id prefix of live-effect plug-ins: `plugin.<plug-in id>`.
pub const PREFIX: &str = crate::PREFIX;

/// Wall-clock budget of one live-effect run (drawing waits for it), in milliseconds.
pub const WALL_TIME_MS: u64 = 5_000;

/// Cached results kept before the cache starts over.
const CACHE_SIZE: usize = 512;
/// Anchors the cached inputs and results may hold together before the cache starts over (about
/// 50 MB). A plug-in can return far more than it was given, so a count of results alone let a few
/// hundred small inputs keep gigabytes. A result past a quarter of this isn't kept.
const CACHE_ANCHORS: usize = 1 << 20;

/// The plug-in id of effect id `id` (`plugin.<id>`).
pub fn plugin_id(effect_id: &str) -> Option<&str> {
    effect_id.strip_prefix(PREFIX)
}

/// The effect id of plug-in `id`.
pub fn effect_id(plugin_id: &str) -> String {
    format!("{PREFIX}{plugin_id}")
}

/// Does Safe Mode skip effect `effect_id` (a plug-in effect while Safe Mode is on)? Its object then
/// draws its geometry without it, and keeps the effect.
pub fn skipped(effect_id: &str) -> bool {
    plugin_id(effect_id).is_some() && crate::safe_mode::active()
}

/// An installed live-effect plug-in by its effect id.
pub fn installed(effect_id: &str) -> Option<std::sync::Arc<crate::Plugin>> {
    registry::get(plugin_id(effect_id)?).filter(|p| p.manifest().kind == Kind::Effect)
}

/// What a live effect is given besides its geometry: the object's appearance and where in it the
/// effect sits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Paints<'a> {
    /// The object's fills and strokes (`None`: an object without, given as empty lists).
    pub appearance: Option<&'a Appearance>,
    /// The index in `appearance.items` of the fill or stroke whose own effects hold this one;
    /// `None`: one of the object's effects.
    pub item: Option<usize>,
}

/// [`Paints`] as the plug-in is given them: the input object's `fills` and `strokes`, and
/// `_context.item`.
struct Given {
    fills: Vec<Value>,
    strokes: Vec<Value>,
    item: Value,
}

impl Paints<'_> {
    /// What the plug-in is given: `item` is `"object"`, or `{"fill": i}` / `{"stroke": i}` with `i`
    /// counting the object's fills or strokes from the bottom.
    fn given(&self) -> Given {
        let Some(ap) = self.appearance else { return Given { fills: Vec::new(), strokes: Vec::new(), item: json!("object") } };
        let (fills, strokes) = paint_lists(ap);
        let at = self.item.and_then(|i| Some((ap.items.get(i)?, ap.items.get(..i)?)));
        let item = match at {
            Some((AppearanceItem::Fill(_), below)) => json!({"fill": below.iter().filter(|i| i.is_fill()).count()}),
            Some((AppearanceItem::Stroke(_), below)) => json!({"stroke": below.iter().filter(|i| !i.is_fill()).count()}),
            None => json!("object"),
        };
        Given { fills, strokes, item }
    }

    /// Where the effect sits: what the pieces of an effect that declares `paintOutput` may paint.
    fn scope(&self) -> Scope {
        match self.appearance.zip(self.item).and_then(|(ap, i)| ap.items.get(i)) {
            Some(AppearanceItem::Fill(_)) => Scope::Fill,
            Some(AppearanceItem::Stroke(_)) => Scope::Stroke,
            None => Scope::Object,
        }
    }
}

#[derive(PartialEq)]
struct Key {
    serial: u64,
    params: String,
    /// The paints and `_context.item`, as the plug-in is given them.
    paints: String,
    bounds: [u64; 4],
    rule: FillRule,
    path: PathData,
}

impl Key {
    fn hash64(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.serial, &self.params, &self.paints, self.bounds, self.rule).hash(&mut h);
        for sp in &self.path.subpaths {
            sp.closed.hash(&mut h);
            for a in &sp.anchors {
                for p in [a.p, a.h_in, a.h_out] {
                    (p.x.to_bits(), p.y.to_bits()).hash(&mut h);
                }
            }
        }
        h.finish()
    }
}

/// What one run of a live effect gave: geometry, or the painted pieces of an effect that declares
/// `paintOutput` ([`crate::painted`]).
#[derive(Clone, Debug, PartialEq)]
enum Out {
    Path(PathData),
    Pieces(Arc<Vec<Piece>>),
}

impl Out {
    /// The geometry: the path, or every piece's path in order.
    fn geometry(&self) -> PathData {
        match self {
            Out::Path(p) => p.clone(),
            Out::Pieces(pieces) => PathData::new(pieces.iter().flat_map(|p| p.path.subpaths.iter().cloned()).collect()),
        }
    }
}

#[derive(Default)]
struct Cache {
    entries: HashMap<u64, (Key, Option<Out>)>,
    /// The anchors of the entries' inputs and results ([`CACHE_ANCHORS`]).
    anchors: usize,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Cache units a path takes: its anchors, and one for each subpath's own allocation.
fn anchors(p: &PathData) -> usize {
    p.subpaths.iter().map(|sp| sp.anchors.len() + 1).sum()
}

/// Cache units an entry takes: its input and result paths, the serialized parameters and paints its
/// key owns (one unit per [`CACHE_ANCHORS`]'s 50 bytes an anchor stands for), and one for each
/// paint, gradient stop and freeform point a painted result holds.
fn weight(key: &Key, out: Option<&Out>) -> usize {
    let out = match out {
        None => 0,
        Some(Out::Path(p)) => anchors(p),
        Some(Out::Pieces(pieces)) => pieces.iter().map(|p| anchors(&p.path) + 1).sum::<usize>() + painted::paint_count(pieces),
    };
    anchors(&key.path) + out + (key.params.len() + key.paints.len()) / 50 + 1
}
static ERRORS: Mutex<Option<HashMap<String, Failures>>> = Mutex::new(None);

/// The errors recorded for a live-effect plug-in, one for each kind of outcome that can clear it.
#[derive(Default)]
struct Failures {
    /// From a run of the plug-in.
    run: Option<String>,
    /// From checking the swatches and patterns a painted result names against the document.
    references: Option<String>,
    /// Was the latest failure a refused reference?
    references_latest: bool,
}

/// Which of a plug-in's [`Failures`] an outcome replaces.
#[derive(Clone, Copy)]
enum Origin {
    Run,
    References,
}

/// Where an installation ([`crate::Plugin::serial`]) stands as a live effect in this process.
enum Runs {
    /// Its first run is in progress, or a run that started while a guarded run was in progress:
    /// how many of them are running, whether one has finished (wasn't cancelled), and the launch
    /// marker's entry naming the plug-in until none is running.
    Guarded { running: usize, finished: bool, loading: crate::safe_mode::Loading },
    /// A guarded run finished and none is running: later runs aren't guarded.
    Done,
}

static RUNS: Mutex<Option<HashMap<u64, Runs>>> = Mutex::new(None);

/// A run of `plugin` starts → is it guarded (the launch marker names the plug-in while it runs)?
/// The first run in this process is, and so is every run that starts while a guarded one is still
/// running. Give a guarded run to [`end_run`] when it ends.
fn begin_run(plugin: &crate::Plugin) -> bool {
    let mut runs = RUNS.lock().unwrap_or_else(|e| e.into_inner());
    let runs = runs.get_or_insert_with(HashMap::new);
    match runs.get_mut(&plugin.serial()) {
        Some(Runs::Done) => false,
        Some(Runs::Guarded { running, .. }) => {
            *running += 1;
            true
        }
        None => {
            let loading = crate::safe_mode::loading(&crate::safe_mode::Item::of(plugin));
            runs.insert(plugin.serial(), Runs::Guarded { running: 1, finished: false, loading });
            true
        }
    }
}

/// A guarded run of installation `serial` ended; `cancelled`: a drawing stopped it (an export
/// stopped half way), which says nothing about the plug-in. When it was the last guarded run still
/// running, the launch marker stops naming the plug-in, and when none of the guarded runs finished,
/// the next run is guarded as the first.
fn end_run(serial: u64, cancelled: bool) {
    let mut runs = RUNS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(runs) = runs.as_mut() else { return };
    let Some(Runs::Guarded { running, finished, .. }) = runs.get_mut(&serial) else { return };
    *running = running.saturating_sub(1);
    *finished |= !cancelled;
    if *running > 0 {
        return;
    }
    let ended = if *finished { runs.insert(serial, Runs::Done) } else { runs.remove(&serial) };
    if let Some(Runs::Guarded { loading, .. }) = ended {
        crate::safe_mode::loaded(loading);
    }
}

/// The last error of live-effect plug-in `id`: the latest failure that still stands. A run that
/// succeeds clears a failed run's error, and a reference check that passes clears a refused
/// reference's.
pub fn last_error(id: &str) -> Option<String> {
    let errors = ERRORS.lock().unwrap_or_else(|e| e.into_inner());
    let failures = errors.as_ref()?.get(id)?;
    let (latest, other) = if failures.references_latest { (&failures.references, &failures.run) } else { (&failures.run, &failures.references) };
    latest.clone().or_else(|| other.clone())
}

/// Records how a run of plug-in `id`, or a check of what its result names, went: a failure as that
/// origin's error (a painted effect's named after the plug-in, [`painted::named`]), a success
/// clearing that origin's error. A run of an effect that paints nothing clears a refused
/// reference's error too.
fn note(id: &str, plugin: &crate::Plugin, origin: Origin, r: std::result::Result<(), &Error>) {
    let message = r.err().map(|e| if plugin.manifest().paint_output { painted::named(&plugin.manifest().name, e) } else { e.to_string() });
    let mut errors = ERRORS.lock().unwrap_or_else(|e| e.into_inner());
    let errors = errors.get_or_insert_with(HashMap::new);
    let failures = errors.entry(id.to_string()).or_default();
    if message.is_some() {
        failures.references_latest = matches!(origin, Origin::References);
    }
    match origin {
        Origin::Run => {
            failures.run = message;
            // Nothing checks the references of an effect that paints nothing, such as one that
            // replaced a painted effect under this id.
            if !plugin.manifest().paint_output {
                failures.references = None;
            }
        }
        Origin::References => failures.references = message,
    }
    if failures.run.is_none() && failures.references.is_none() {
        errors.remove(id);
    }
}

/// Does live-effect plug-in `id` return painted pieces (its manifest declares `paintOutput`)?
pub fn paints_output(id: &str) -> bool {
    registry::get(id).is_some_and(|p| p.manifest().kind == Kind::Effect && p.manifest().paint_output)
}

/// Runs live-effect plug-in `id` on `path` (whose reference box is `bounds`), of an object painted
/// with `paints`: the new geometry, or `None` when the plug-in isn't installed, isn't an effect,
/// fails, or Safe Mode is on (the caller then keeps the geometry, and the object keeps its effect).
/// For an effect that returns painted pieces, the geometry is every piece's path in order, and
/// `None` when the swatches and patterns they name are refused as [`apply_painted`] refuses them.
pub fn apply(id: &str, params: &Value, path: &PathData, bounds: Rect, rule: FillRule, paints: Paints, doc: Option<&Document>) -> Option<PathData> {
    let (plugin, out) = evaluate(id, params, path, bounds, rule, paints)?;
    if let Out::Pieces(pieces) = &out {
        references_found(id, &plugin, pieces, doc)?;
    }
    Some(out.geometry())
}

/// Runs live-effect plug-in `id`, which declares `paintOutput`, as [`apply`] does: its painted
/// pieces, each to draw by itself in order with the paints it gives ([`painted::appearance`],
/// [`painted::items`]). Every swatch and pattern the pieces name must be in `doc`; without a
/// document, pieces that name one are refused ([`painted::check_references`]). A paint linked to a
/// global swatch has that swatch's current color. `None` when the
/// plug-in isn't installed, doesn't declare `paintOutput`, fails, or Safe Mode is on; the caller then
/// draws its input unchanged, and [`last_error`] says why a run failed (a refusal for want of a
/// document is not recorded).
pub fn apply_painted(
    id: &str,
    params: &Value,
    path: &PathData,
    bounds: Rect,
    rule: FillRule,
    paints: Paints,
    doc: Option<&Document>,
) -> Option<Arc<Vec<Piece>>> {
    let (plugin, out) = evaluate(id, params, path, bounds, rule, paints)?;
    let Out::Pieces(pieces) = out else { return None };
    references_found(id, &plugin, &pieces, doc)?;
    Some(match doc {
        Some(d) => painted::with_swatch_colors(pieces, d),
        None => pieces,
    })
}

/// `Some` when every swatch and pattern `pieces` name is in `doc` ([`painted::check_references`]),
/// recording the outcome for [`last_error`] when there is a document. The document's swatches and
/// patterns can change while a result stays cached, so each use checks them.
fn references_found(id: &str, plugin: &crate::Plugin, pieces: &[Piece], doc: Option<&Document>) -> Option<()> {
    let checked = painted::check_references(pieces, doc);
    if doc.is_some() {
        note(id, plugin, Origin::References, checked.as_ref().map(|_| ()));
    }
    checked.ok()
}

/// Runs plug-in `id` as [`apply`] says, or takes its result from the cache: the plug-in and what
/// it gave. The launch marker names the plug-in during its first run in this process, and until
/// every run that started while it ran has ended too ([`crate::safe_mode::loading`]).
fn evaluate(id: &str, params: &Value, path: &PathData, bounds: Rect, rule: FillRule, paints: Paints) -> Option<(Arc<crate::Plugin>, Out)> {
    if crate::safe_mode::active() {
        return None;
    }
    let plugin = registry::get(id).filter(|p| p.manifest().kind == Kind::Effect)?;
    let resolved = Value::Object(plugin.manifest().resolve_params(params).ok()?);
    let given = paints.given();
    let key = Key {
        serial: plugin.serial(),
        params: resolved.to_string(),
        paints: json!([given.fills, given.strokes, given.item]).to_string(),
        bounds: [bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(f64::to_bits),
        rule,
        path: path.clone(),
    };
    let h = key.hash64();
    if let Some((k, out)) = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|c| c.entries.get(&h))
        && *k == key
    {
        return out.clone().map(|o| (plugin, o));
    }
    let scope = paints.scope();
    let guarded = begin_run(&plugin);
    let r = run(&plugin, resolved, path, bounds, rule, given, scope);
    let cancelled = matches!(r, Err(Error::Cancelled));
    if guarded {
        end_run(plugin.serial(), cancelled);
    }
    // A drawing that was cancelled says nothing about the plug-in: the next drawing runs it again.
    if cancelled {
        return None;
    }
    note(id, &plugin, Origin::Run, r.as_ref().map(|_| ()));
    let out = r.ok();
    let size = weight(&key, out.as_ref());
    if size > CACHE_ANCHORS / 4 {
        return out.map(|o| (plugin, o));
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(Cache::default);
    if cache.entries.len() >= CACHE_SIZE || cache.anchors + size > CACHE_ANCHORS {
        *cache = Cache::default();
    }
    if let Some((old, old_out)) = cache.entries.insert(h, (key, out.clone())) {
        cache.anchors = cache.anchors.saturating_sub(weight(&old, old_out.as_ref()));
    }
    cache.anchors += size;
    out.map(|o| (plugin, o))
}

/// One run of `plugin` on `path`, with the object's paints as `given`: geometry, or for an effect
/// that declares `paintOutput` its pieces, checked for an effect in `scope`.
fn run(plugin: &crate::Plugin, mut params: Value, path: &PathData, bounds: Rect, rule: FillRule, given: Given, scope: Scope) -> Result<Out> {
    let b = json!([bounds.x0, bounds.y0, bounds.x1, bounds.y1]);
    params["_context"] = json!({"mode": "effect", "bounds": b, "item": given.item});
    let rule_name = match rule {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    };
    let input =
        json!({"objects": [{"type": "path", "path": path, "fillRule": rule_name, "bounds": b, "fills": given.fills, "strokes": given.strokes}]});
    let (input, params) = (serde_json::to_vec(&input), serde_json::to_vec(&params));
    let (Ok(input), Ok(params)) = (input, params) else { return Err(Error::Params("can't encode the input".into())) };
    let limits = Limits { wall_time_ms: WALL_TIME_MS, ..plugin.limits().clone() };
    let output = plugin.run_limited(&input, &params, &limits)?;
    if plugin.manifest().paint_output {
        return Ok(Out::Pieces(Arc::new(painted::decode(&output, scope, rule)?)));
    }
    Ok(Out::Path(output_geometry(decode(&output)?)))
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{FillLayer, StrokeLayer};
    use vectorcraft_geom::{Point, SubPath};

    use super::*;

    /// Fills and strokes come in paint order, and the item an effect sits on is counted among the
    /// fills or among the strokes.
    #[test]
    fn the_paints_and_the_item_given() {
        let fill = |c| AppearanceItem::Fill(FillLayer::new(Paint::solid(c)));
        let stroke = |w| AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), w));
        let ap = Appearance { items: vec![fill(Color::WHITE), stroke(2.0), fill(Color::BLACK), stroke(5.0)], ..Appearance::default() };
        let given = |item| Paints { appearance: Some(&ap), item }.given();
        let g = given(None);
        assert_eq!((g.fills.len(), g.strokes.len(), &g.item), (2, 2, &json!("object")));
        assert_eq!(g.strokes.iter().map(|s| s["width"].clone()).collect::<Vec<_>>(), [json!(2.0), json!(5.0)]);
        assert_eq!(g.fills[0], serde_json::to_value(Paint::solid(Color::WHITE)).unwrap());
        assert_eq!(given(Some(3)).item, json!({"stroke": 1}));
        assert_eq!(given(Some(2)).item, json!({"fill": 1}));
        assert_eq!(given(Some(0)).item, json!({"fill": 0}));
        assert_eq!(given(Some(9)).item, json!("object"), "no such item");
        let none = Paints::default().given();
        assert_eq!((none.fills, none.strokes, none.item), (vec![], vec![], json!("object")));
    }

    /// A result taken from the cache passes its reference check without clearing the error of a
    /// failed run, and the next run that succeeds clears that error.
    #[test]
    fn a_passed_reference_check_keeps_a_failed_run_s_error() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        const ID: &str = "test.effect.lasterror";
        let manifest = format!(r#"{{"id":"{ID}","name":"Test","version":"1","kind":"effect","api":2,"paintOutput":true,"params":{{}}}}"#);
        // Traps on a long input.
        let first = "(if (i32.gt_u (local.get $il) (i32.const 4096)) (then unreachable))";
        let src = crate::wat::with_imports(&manifest, "", first, r#"{"objects":[]}"#);
        registry::install(crate::Plugin::load(&wat::parse_str(&src).unwrap(), Limits::default()).unwrap());
        let doc = Document::new(100.0, 100.0);
        let line = |n: usize| PathData::single(SubPath::polyline(&(0..n).map(|i| Point::new(i as f64, 0.0)).collect::<Vec<_>>(), false));
        let draw =
            |path: &PathData| apply_painted(ID, &json!({}), path, Rect::new(0.0, 0.0, 1.0, 1.0), FillRule::NonZero, Paints::default(), Some(&doc));
        let short = line(2);
        assert!(draw(&short).is_some());
        assert_eq!(last_error(ID), None);
        assert!(draw(&line(1000)).is_none());
        let failed = last_error(ID).unwrap();
        assert!(draw(&short).is_some(), "from the cache");
        assert_eq!(last_error(ID), Some(failed));
        assert!(draw(&line(3)).is_some(), "a new run");
        assert_eq!(last_error(ID), None);
        registry::remove(ID);
    }

    /// With a refused reference and a failed run both standing, the latest of the two is reported.
    #[test]
    fn the_latest_of_a_refused_reference_and_a_failed_run_is_reported() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        const ID: &str = "test.effect.latesterror";
        let manifest = format!(r#"{{"id":"{ID}","name":"Test","version":"1","kind":"effect","api":2,"paintOutput":true,"params":{{}}}}"#);
        let mut fill = serde_json::to_value(Paint::solid(Color::WHITE)).unwrap();
        fill["swatch"] = json!("Missing");
        let square = PathData::single(SubPath::polyline(&[(0.0, 0.0), (9.0, 0.0), (9.0, 9.0)].map(|(x, y)| Point::new(x, y)), true));
        let output = json!({"objects": [{"path": square, "fills": [fill]}]}).to_string();
        // Traps on a long input.
        let first = "(if (i32.gt_u (local.get $il) (i32.const 4096)) (then unreachable))";
        let src = crate::wat::with_imports(&manifest, "", first, &output);
        registry::install(crate::Plugin::load(&wat::parse_str(&src).unwrap(), Limits::default()).unwrap());
        let doc = Document::new(100.0, 100.0);
        let line = |n: usize| PathData::single(SubPath::polyline(&(0..n).map(|i| Point::new(i as f64, 0.0)).collect::<Vec<_>>(), false));
        let draw =
            |path: &PathData| apply_painted(ID, &json!({}), path, Rect::new(0.0, 0.0, 1.0, 1.0), FillRule::NonZero, Paints::default(), Some(&doc));
        let short = line(2);
        assert!(draw(&short).is_none());
        assert!(last_error(ID).unwrap().contains("swatch"));
        assert!(draw(&line(1000)).is_none());
        assert!(last_error(ID).unwrap().contains("trapped"));
        assert!(draw(&short).is_none(), "from the cache, refused again");
        assert!(last_error(ID).unwrap().contains("swatch"));
        registry::remove(ID);
    }

    /// A first run that is cancelled leaves the next run to be the first: the launch marker names
    /// the plug-in during that one.
    #[test]
    fn a_cancelled_first_run_leaves_the_next_run_first() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let src = crate::wat::spinning(&crate::wat::manifest("test.effect.cancelled", "effect", "{}"), 1_000_000_000, r#"{"objects":[]}"#);
        let plugin = registry::install(crate::Plugin::load(&wat::parse_str(&src).unwrap(), Limits::default()).unwrap());
        let ran_first =
            || RUNS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|r| matches!(r.get(&plugin.serial()), Some(Runs::Done)));
        let draw = || {
            apply(
                "test.effect.cancelled",
                &json!({}),
                &PathData::default(),
                Rect::new(0.0, 0.0, 1.0, 1.0),
                FillRule::NonZero,
                Paints::default(),
                None,
            )
        };
        let token = vectorcraft_doc::cancel::CancelToken::new();
        token.cancel();
        assert_eq!(vectorcraft_doc::cancel::scope(&token, None, draw), None);
        assert!(!ran_first(), "the cancelled run doesn't count as the first");
        // Runs to its instruction budget and fails: that run was the first.
        assert_eq!(draw(), None);
        assert!(ran_first());
        registry::remove("test.effect.cancelled");
    }

    /// Two runs overlap during a plug-in's first run in this process: when the first ends while the
    /// second is still running, the launch marker keeps naming the plug-in until the second ends.
    #[test]
    fn the_marker_stays_while_an_overlapping_run_is_running() {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::safe_mode::set_active(None);
        let dir = std::env::temp_dir().join(format!("vc-effect-overlap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(crate::safe_mode::begin_launch(&dir), None);
        let marker = || {
            let files = std::fs::read_dir(&dir).unwrap().filter_map(|e| Some(e.ok()?.path()));
            let json = files.filter(|p| p.extension().is_some_and(|x| x == "json")).collect::<Vec<_>>();
            assert_eq!(json.len(), 1, "one marker for this launch");
            crate::safe_mode::parse_marker(&std::fs::read(&json[0]).unwrap())
        };
        let src = crate::wat::returning(&crate::wat::manifest("test.effect.overlap", "effect", "{}"), r#"{"objects":[]}"#);
        let plugin = registry::install(crate::Plugin::load(&wat::parse_str(&src).unwrap(), Limits::default()).unwrap());
        let named = Some(crate::safe_mode::Item::of(&plugin));
        assert_eq!(marker(), None);

        assert!(begin_run(&plugin), "the first run is guarded");
        assert!(begin_run(&plugin), "a run that starts while the first is running is guarded");
        assert_eq!(marker(), named);
        end_run(plugin.serial(), false);
        assert_eq!(marker(), named, "the second run is still running");
        // A third run starts before the second ends, and the second is cancelled.
        assert!(begin_run(&plugin));
        end_run(plugin.serial(), true);
        assert_eq!(marker(), named, "the third run is still running");
        end_run(plugin.serial(), false);
        assert_eq!(marker(), None, "no run is running");
        assert!(!begin_run(&plugin), "later runs aren't guarded");
        assert_eq!(marker(), None);

        crate::safe_mode::end_launch();
        crate::safe_mode::forget_previous();
        registry::remove("test.effect.overlap");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
