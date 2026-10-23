//! Live-effect plug-ins: a plug-in of kind `effect` rewrites one object's geometry each time it is
//! drawn, from its appearance stack (an `Effect` record with id `plugin.<plug-in id>`).
//!
//! The plug-in is given the geometry and, read only, the object's fills and strokes as a filter
//! sees them ([`Paints`]): an effect can draw as wide as the stroke it sits on, or tell its fills
//! apart, without asking for them again as parameters.
//!
//! Besides geometry, or instead of it, a live effect may return an [`Overlay`]: lines the canvas
//! shows above the artwork and nothing else draws. The canvas shows those of the runs drawing
//! made, from the cache ([`cached`]), and never runs a plug-in for them.
//!
//! Drawing evaluates effects often (every frame, hit tests, bounds), so results are cached by
//! plug-in installation, parameters, input geometry and paints: the plug-in runs again only when
//! one of them changes. Failures are cached too (the geometry is then drawn unchanged) and
//! reported by [`last_error`]. The cache starts over when it holds a few hundred results, or
//! overlays of a few million points in all ([`cache_generation`] tells when it changes).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, AppearanceItem};
use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::objects::{declined, decode_objects, output_geometry, paint_lists};
pub use crate::overlay::Overlay;
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

/// Overlay points the cached results hold in all before the cache starts over: four overlays as
/// large as one may be.
const CACHE_POINTS: usize = 4 * crate::overlay::MAX_POINTS;

/// The plug-in id of effect id `id` (`plugin.<id>`).
pub fn plugin_id(effect_id: &str) -> Option<&str> {
    effect_id.strip_prefix(PREFIX)
}

/// The effect id of plug-in `id`.
pub fn effect_id(plugin_id: &str) -> String {
    format!("{PREFIX}{plugin_id}")
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

/// What one run of a live effect made: its geometry, and the overlay it showed with it.
#[derive(Clone, Debug)]
pub struct Ran {
    pub path: PathData,
    pub overlay: Option<Arc<Overlay>>,
}

impl Ran {
    /// The points of its overlay.
    fn points(&self) -> usize {
        self.overlay.as_ref().map_or(0, |o| o.points())
    }
}

/// The results of live-effect runs by their key's hash, and what they hold in all: the anchors of
/// their inputs and geometry, and the points of their overlays.
#[derive(Default)]
struct Cache {
    results: HashMap<u64, (Key, Option<Ran>)>,
    /// The anchors of the results' inputs and geometry ([`CACHE_ANCHORS`]).
    anchors: usize,
    /// The points of the results' overlays ([`CACHE_POINTS`]).
    points: usize,
}

impl Cache {
    /// The result of `key` (whose hash is `h`): `None` when it isn't kept, `Some(None)` for a run
    /// that failed.
    fn get(&self, h: u64, key: &Key) -> Option<Option<Ran>> {
        self.results.get(&h).filter(|(k, _)| k == key).map(|(_, out)| out.clone())
    }

    /// Keeps `out`, the result of `key` (whose hash is `h`), first starting over when the cache
    /// holds [`CACHE_SIZE`] results, or `out` would take their anchors past `max_anchors` or their
    /// overlays' points past `max_points`. A result whose anchors, its input's with its geometry's,
    /// are more than a quarter of `max_anchors` isn't kept.
    fn keep(&mut self, h: u64, key: Key, out: Option<Ran>, max_anchors: usize, max_points: usize) {
        let size = held(&key, out.as_ref());
        if size > max_anchors / 4 {
            return;
        }
        let points = out.as_ref().map_or(0, Ran::points);
        if self.results.len() >= CACHE_SIZE || self.anchors.saturating_add(size) > max_anchors || self.points.saturating_add(points) > max_points {
            *self = Cache::default();
        }
        if let Some((old, old_out)) = self.results.insert(h, (key, out)) {
            self.anchors = self.anchors.saturating_sub(held(&old, old_out.as_ref()));
            self.points = self.points.saturating_sub(old_out.as_ref().map_or(0, Ran::points));
        }
        self.anchors = self.anchors.saturating_add(size);
        self.points = self.points.saturating_add(points);
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn anchors(p: &PathData) -> usize {
    p.subpaths.iter().map(|sp| sp.anchors.len()).sum()
}

/// The anchors a cached result holds: its input's, and its geometry's.
fn held(key: &Key, out: Option<&Ran>) -> usize {
    anchors(&key.path).saturating_add(out.map_or(0, |r| anchors(&r.path)))
}

/// Counts the results the cache has kept ([`cache_generation`]).
static GENERATION: AtomicU64 = AtomicU64::new(0);
static ERRORS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Changes whenever the cache keeps a result, or starts over: what [`cached`] didn't find may be
/// there now.
pub fn cache_generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// The last error of live-effect plug-in `id` (cleared when it next succeeds).
pub fn last_error(id: &str) -> Option<String> {
    ERRORS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(id).cloned()
}

/// Records how a run of plug-in `id` went (`r`) for [`last_error`]: a failure replaces its last
/// error, a success clears it.
fn note(id: &str, r: &Result<Ran>) {
    let mut errors = ERRORS.lock().unwrap_or_else(|e| e.into_inner());
    let errors = errors.get_or_insert_with(HashMap::new);
    match r {
        Ok(_) => errors.remove(id),
        Err(e) => errors.insert(id.to_string(), e.to_string()),
    };
}

/// Runs live-effect plug-in `id` on `path` (whose reference box is `bounds`), of an object painted
/// with `paints`: the new geometry, or `None` when the plug-in isn't installed, isn't an effect, or
/// fails (the caller then keeps the geometry).
pub fn apply(id: &str, params: &Value, path: &PathData, bounds: Rect, rule: FillRule, paints: Paints) -> Option<PathData> {
    ran(id, params, path, bounds, rule, paints).map(|r| r.path)
}

/// What drawing made of `path` with live-effect plug-in `id` (the run [`apply`] used), from the
/// cache alone: this never runs the plug-in. One that isn't installed, isn't an effect, refuses
/// `params` or failed leaves the geometry as it is and shows nothing. `None` when it hasn't run on
/// these inputs, or the cache has let the result go since.
pub fn cached(id: &str, params: &Value, path: &PathData, bounds: Rect, rule: FillRule, paints: Paints) -> Option<Ran> {
    let out = match Inputs::of(id, params, path, bounds, rule, paints) {
        Some(inputs) => inputs.cached()?,
        None => None,
    };
    Some(out.unwrap_or_else(|| Ran { path: path.clone(), overlay: None }))
}

/// The run of [`apply`], from the cache when it can.
fn ran(id: &str, params: &Value, path: &PathData, bounds: Rect, rule: FillRule, paints: Paints) -> Option<Ran> {
    let inputs = Inputs::of(id, params, path, bounds, rule, paints)?;
    if let Some(out) = inputs.cached() {
        return out;
    }
    let Inputs { plugin, params, given, key, hash } = inputs;
    let r = run(&plugin, params, path, bounds, rule, given);
    // A drawing that was cancelled (an export stopped half way) says nothing about the plug-in:
    // the next drawing runs it again.
    if matches!(r, Err(Error::Cancelled)) {
        return None;
    }
    note(id, &r);
    let out = r.ok();
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Cache::default).keep(hash, key, out.clone(), CACHE_ANCHORS, CACHE_POINTS);
    out
}

/// A run of a live effect as the cache knows it: the plug-in, the parameters and paints it is
/// given, and its key, hashed.
struct Inputs {
    plugin: Arc<crate::Plugin>,
    params: Value,
    given: Given,
    key: Key,
    hash: u64,
}

impl Inputs {
    /// The run of live-effect plug-in `id` on `path` (whose reference box is `bounds`), of an
    /// object painted with `paints`: `None` when the plug-in isn't installed, isn't an effect, or
    /// refuses `params`.
    fn of(id: &str, params: &Value, path: &PathData, bounds: Rect, rule: FillRule, paints: Paints) -> Option<Self> {
        let plugin = registry::get(id).filter(|p| p.manifest().kind == Kind::Effect)?;
        let params = Value::Object(plugin.manifest().resolve_params(params).ok()?);
        let given = paints.given();
        let key = Key {
            serial: plugin.serial(),
            params: params.to_string(),
            paints: json!([given.fills, given.strokes, given.item]).to_string(),
            bounds: [bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(f64::to_bits),
            rule,
            path: path.clone(),
        };
        let hash = key.hash64();
        Some(Self { plugin, params, given, key, hash })
    }

    /// Its result in the cache: `None` when it isn't there, `Some(None)` for a run that failed.
    fn cached(&self) -> Option<Option<Ran>> {
        CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(self.hash, &self.key)
    }
}

/// One run of `plugin` on `path`, with the object's paints as `given`.
fn run(plugin: &crate::Plugin, mut params: Value, path: &PathData, bounds: Rect, rule: FillRule, given: Given) -> Result<Ran> {
    let b = json!([bounds.x0, bounds.y0, bounds.x1, bounds.y1]);
    params["_context"] = json!({"mode": "effect", "bounds": b, "item": given.item});
    let rule = match rule {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    };
    let input = json!({"objects": [{"type": "path", "path": path, "fillRule": rule, "bounds": b, "fills": given.fills, "strokes": given.strokes}]});
    let (input, params) = (serde_json::to_vec(&input), serde_json::to_vec(&params));
    let (Ok(input), Ok(params)) = (input, params) else { return Err(Error::Params("can't encode the input".into())) };
    let limits = Limits { wall_time_ms: WALL_TIME_MS, ..plugin.limits().clone() };
    read(&plugin.run_limited(&input, &params, &limits)?, path)
}

/// A live effect's output: `{"objects": [...], "overlay"?: …}`, or an overlay alone, which leaves
/// the geometry (`input`) as it is.
fn read(bytes: &[u8], input: &PathData) -> Result<Ran> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    if let Some(e) = declined(&v) {
        return Err(e);
    }
    let overlay = Overlay::from_output(&v)?.map(Arc::new);
    let path = match (v.get("objects").filter(|o| !o.is_null()), &overlay) {
        (None, Some(_)) => input.clone(),
        _ => output_geometry(decode_objects(&v)?),
    };
    Ok(Ran { path, overlay })
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{FillLayer, StrokeLayer};

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

    /// The cached overlays hold at most so many points: a result whose overlay would take them
    /// further starts the cache over, a result kept again under its key gives back the points of
    /// the one it replaces, and each result kept changes the generation.
    #[test]
    fn the_cache_holds_overlays_of_so_many_points() {
        let key =
            |serial| Key { serial, params: String::new(), paints: String::new(), bounds: [0; 4], rule: FillRule::NonZero, path: PathData::default() };
        let ran = |points| {
            let line = crate::overlay::Line { color: [0; 4], width: 0.0, points: vec![[0.0, 0.0]; points] };
            Some(Ran { path: PathData::default(), overlay: Some(Arc::new(Overlay { lines: vec![line] })) })
        };
        let mut cache = Cache::default();
        cache.keep(1, key(1), ran(60), CACHE_ANCHORS, 100);
        cache.keep(2, key(2), ran(30), CACHE_ANCHORS, 100);
        assert_eq!((cache.results.len(), cache.points), (2, 90));
        cache.keep(3, key(3), ran(20), CACHE_ANCHORS, 100);
        assert_eq!((cache.results.len(), cache.points), (1, 20), "past 100 points it starts over");
        assert!(cache.get(1, &key(1)).is_none() && cache.get(3, &key(3)).is_some());
        cache.keep(3, key(3), ran(5), CACHE_ANCHORS, 100);
        assert_eq!((cache.results.len(), cache.points), (1, 5), "the same key kept again");
        let before = cache_generation();
        cache.keep(4, key(4), None, CACHE_ANCHORS, 100);
        assert_eq!(cache.points, 5, "a failed run holds no points");
        assert!(cache_generation() > before);
    }

    /// The cached inputs and results hold at most so many anchors: a result that would take them
    /// further starts the cache over, one of more than a quarter of them isn't kept, and a result
    /// kept again under its key gives back the anchors of the one it replaces.
    #[test]
    fn the_cache_holds_inputs_and_results_of_so_many_anchors() {
        let path = |n| {
            PathData::new(vec![vectorcraft_geom::SubPath::new(
                vec![vectorcraft_geom::Anchor::corner(vectorcraft_geom::Point::new(0.0, 0.0)); n],
                false,
            )])
        };
        let key = |serial, n| Key { serial, params: String::new(), paints: String::new(), bounds: [0; 4], rule: FillRule::NonZero, path: path(n) };
        let ran = |n| Some(Ran { path: path(n), overlay: None });
        let mut cache = Cache::default();
        cache.keep(1, key(1, 10), ran(15), 100, CACHE_POINTS);
        cache.keep(2, key(2, 20), None, 100, CACHE_POINTS);
        assert_eq!((cache.results.len(), cache.anchors), (2, 45), "an input's anchors and its geometry's");
        cache.keep(3, key(3, 26), None, 100, CACHE_POINTS);
        assert_eq!((cache.results.len(), cache.anchors), (2, 45), "more than a quarter of the anchors isn't kept");
        assert!(cache.get(3, &key(3, 26)).is_none());
        cache.keep(4, key(4, 20), ran(5), 100, CACHE_POINTS);
        cache.keep(5, key(5, 25), None, 100, CACHE_POINTS);
        assert_eq!((cache.results.len(), cache.anchors), (4, 95));
        cache.keep(6, key(6, 3), ran(3), 100, CACHE_POINTS);
        assert_eq!((cache.results.len(), cache.anchors), (1, 6), "past 100 anchors it starts over");
        cache.keep(6, key(6, 3), None, 100, CACHE_POINTS);
        assert_eq!((cache.results.len(), cache.anchors), (1, 3), "the same key kept again");
    }
}
