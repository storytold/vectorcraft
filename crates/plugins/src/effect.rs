//! Live-effect plug-ins: a plug-in of kind `effect` rewrites one object's geometry each time it is
//! drawn, from its appearance stack (an `Effect` record with id `plugin.<plug-in id>`).
//!
//! The plug-in is given the geometry and, read only, the object's fills and strokes as a filter
//! sees them ([`Paints`]): an effect can draw as wide as the stroke it sits on, or tell its fills
//! apart, without asking for them again as parameters.
//!
//! Drawing evaluates effects often (every frame, hit tests, bounds), so results are cached by
//! plug-in installation, parameters, input geometry and paints: the plug-in runs again only when
//! one of them changes. Failures are cached too (the geometry is then drawn unchanged) and
//! reported by [`last_error`].

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, AppearanceItem};
use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::objects::{decode, output_geometry, paint_lists};
use crate::{Error, Kind, Limits, Result, registry};

/// The effect id prefix of live-effect plug-ins: `plugin.<plug-in id>`.
pub const PREFIX: &str = crate::PREFIX;

/// Wall-clock budget of one live-effect run (drawing waits for it), in milliseconds.
pub const WALL_TIME_MS: u64 = 5_000;

/// Cached results kept before the cache starts over.
const CACHE_SIZE: usize = 512;

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

type Cache = HashMap<u64, (Key, Option<PathData>)>;

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
static ERRORS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// The last error of live-effect plug-in `id` (cleared when it next succeeds).
pub fn last_error(id: &str) -> Option<String> {
    ERRORS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(id).cloned()
}

fn note(id: &str, r: &Result<PathData>) {
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
    if let Some((k, out)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(&h))
        && *k == key
    {
        return out.clone();
    }
    let r = run(&plugin, resolved, path, bounds, rule, given);
    // A drawing that was cancelled (an export stopped half way) says nothing about the plug-in:
    // the next drawing runs it again.
    if matches!(r, Err(Error::Cancelled)) {
        return None;
    }
    note(id, &r);
    let out = r.ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() >= CACHE_SIZE {
        cache.clear();
    }
    cache.insert(h, (key, out.clone()));
    out
}

/// One run of `plugin` on `path`, with the object's paints as `given`.
fn run(plugin: &crate::Plugin, mut params: Value, path: &PathData, bounds: Rect, rule: FillRule, given: Given) -> Result<PathData> {
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
    Ok(output_geometry(decode(&plugin.run_limited(&input, &params, &limits)?)?))
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
}
