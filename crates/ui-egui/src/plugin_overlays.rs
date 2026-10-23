//! What live-effect plug-ins show on the canvas besides their geometry: the overlays of the frame
//! shown ([`crate::CanvasCache::shown`]), which `canvas::plugin_effect_overlay` draws over its art.
//!
//! They come from the plug-ins' cache as drawing that frame left it
//! ([`vectorcraft_render::plugin_overlays`]). Showing them never runs a plug-in, so a slow one
//! can't stall the interface, and a heavy document's overlays wait for the render worker's frame,
//! as its art does. The objects are those the frame drew with their plug-ins
//! ([`vectorcraft_render::plugin_effect_objects`]): Layer Options, Outline view and View › Hide
//! Template apply to overlays as they do to effects.
//!
//! An object whose results aren't in the cache (the view hasn't reached it, or the cache has let
//! them go) shows nothing, and is looked for again when the cache next changes
//! ([`vectorcraft_plugins::effect::cache_generation`]). An object that hasn't changed keeps its
//! overlays from frame to frame, whatever the cache lets go: drawing keeps such an object's
//! geometry, so it won't run its plug-ins again.
//!
//! Only the overlays drawn are kept: they stop at the most points the canvas draws.

use std::collections::BTreeMap;
use std::sync::Arc;

use vectorcraft_doc::{Document, Node};
use vectorcraft_plugins::overlay::{Line, Overlay};
use vectorcraft_render::RenderOptions;

/// The overlays of one frame.
pub struct PluginOverlays {
    /// The document the frame drew, and how.
    doc: Arc<Document>,
    opts: RenderOptions,
    /// The plug-ins installed then ([`Plugins::revision`]).
    plugins: u64,
    /// While an object's results weren't in the cache: the cache's generation then
    /// ([`Plugins::generation`]), which these overlays stand for.
    waiting: Option<u64>,
    /// The objects shown and their overlays, in stacking order.
    objects: Vec<(Arc<Node>, Shown)>,
}

/// What an object shows: its overlays, in the order drawing runs its plug-ins.
type Shown = Arc<[Arc<Overlay>]>;

/// What overlays depend on besides the frame: the plug-ins installed, and their cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plugins {
    /// [`vectorcraft_render::effects::plugin_revision`].
    revision: u64,
    /// [`vectorcraft_plugins::effect::cache_generation`].
    generation: u64,
}

impl Plugins {
    fn now() -> Self {
        Self { revision: vectorcraft_render::effects::plugin_revision(), generation: vectorcraft_plugins::effect::cache_generation() }
    }
}

/// What an object shows, from the plug-ins' cache: `None` while a result isn't there.
type Look<'a> = &'a dyn Fn(&Node) -> Option<Vec<Arc<Overlay>>>;

impl PluginOverlays {
    /// The overlays of the frame that drew `doc` with `opts`, until they reach `max_points` points:
    /// `old` while it still stands, else collected again, keeping `old`'s for the objects that
    /// haven't changed.
    pub fn of_frame(old: Option<Self>, doc: &Arc<Document>, opts: &RenderOptions, max_points: usize) -> Self {
        // Before looking: a result that comes in meanwhile has the objects looked for again.
        let now = Plugins::now();
        match old {
            Some(o) if o.stands(doc, opts, now) => o,
            old => Self::collect(old.as_ref(), doc, opts, max_points, now, &vectorcraft_render::plugin_overlays),
        }
    }

    /// Whether these are the overlays of the frame that drew `doc` with `opts`, with the plug-ins
    /// installed `now`, and nothing they waited for can have come into the cache.
    fn stands(&self, doc: &Arc<Document>, opts: &RenderOptions, now: Plugins) -> bool {
        Arc::ptr_eq(&self.doc, doc) && self.opts == *opts && self.plugins == now.revision && self.waiting.is_none_or(|g| g == now.generation)
    }

    /// The overlays of the frame that drew `doc` with `opts`, as `look` finds them, until they
    /// reach `max_points` points; `old`'s for the objects in it, unless other plug-ins were
    /// installed then.
    fn collect(old: Option<&Self>, doc: &Arc<Document>, opts: &RenderOptions, max_points: usize, now: Plugins, look: Look<'_>) -> Self {
        let kept: BTreeMap<*const Node, &Shown> = old
            .filter(|o| o.plugins == now.revision)
            .map(|o| o.objects.iter().map(|(n, shown)| (Arc::as_ptr(n), shown)).collect())
            .unwrap_or_default();
        let (mut objects, mut points, mut missing) = (Vec::new(), 0usize, false);
        for n in vectorcraft_render::plugin_effect_objects(doc, opts) {
            if points >= max_points {
                break;
            }
            let shown = match kept.get(&Arc::as_ptr(&n)) {
                Some(shown) => Arc::clone(shown),
                None => match look(&n) {
                    Some(shown) => Arc::from(shown),
                    None => {
                        missing = true;
                        continue;
                    }
                },
            };
            points = points.saturating_add(shown.iter().map(|o| o.points()).sum());
            objects.push((n, shown));
        }
        Self { doc: doc.clone(), opts: opts.clone(), plugins: now.revision, waiting: missing.then_some(now.generation), objects }
    }

    /// The lines to draw, in stacking order.
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.objects.iter().flat_map(|(_, shown)| shown.iter()).flat_map(|o| &o.lines)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Effect, NodeId};
    use vectorcraft_geom::{PathData, Point, SubPath};

    use super::*;

    const NOW: Plugins = Plugins { revision: 1, generation: 1 };

    /// A document of three squares, each with the effect of a live-effect plug-in that isn't
    /// installed: `look` stands in for the cache.
    fn three_squares() -> (Arc<Document>, [NodeId; 3]) {
        let mut d = Document::new(100.0, 100.0);
        let layer = d.layers[0].id;
        let mut ids = [NodeId(0); 3];
        for (i, x) in [0.0, 30.0, 60.0].into_iter().enumerate() {
            let corners = [Point::new(x, 0.0), Point::new(x + 20.0, 0.0), Point::new(x + 20.0, 20.0), Point::new(x, 20.0)];
            let mut n = Node::path(
                d.alloc_id(),
                PathData::single(SubPath::polyline(&corners, true)),
                Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
            );
            n.appearance.effects = vec![Effect { id: "plugin.test.ui.overlays.kept".into(), params: json!({}), visible: true }];
            ids[i] = n.id;
            d.insert(Some(layer), i, n).unwrap();
        }
        (Arc::new(d), ids)
    }

    /// One line at `x` with `points` points.
    fn line_at(x: f32, points: usize) -> Vec<Arc<Overlay>> {
        vec![Arc::new(Overlay { lines: vec![Line { color: [0, 0, 255, 255], width: 0.0, points: vec![[x, 0.0]; points] }] })]
    }

    /// Where each line drawn starts.
    fn starts(o: &PluginOverlays) -> Vec<f32> {
        o.lines().map(|l| l.points[0][0]).collect()
    }

    /// An object that hasn't changed keeps its overlays in the next frame, though the cache has let
    /// them go; while one that has changed isn't in the cache, the overlays wait for the cache to
    /// change, then look for it again.
    #[test]
    fn unchanged_objects_keep_their_overlays() {
        let (d1, [a, b, c]) = three_squares();
        let opts = RenderOptions::default();
        let cached = |n: &Node| Some(line_at(n.id.0 as f32, 2));
        let first = PluginOverlays::collect(None, &d1, &opts, 100, NOW, &cached);
        let ids = |ids: &[NodeId]| ids.iter().map(|id| id.0 as f32).collect::<Vec<_>>();
        assert_eq!((starts(&first), first.waiting), (ids(&[a, b, c]), None), "stacking order");
        let mut d2 = (*d1).clone();
        d2.node_mut(b).unwrap().appearance.effects[0].params = json!({"changed": true});
        let d2 = Arc::new(d2);
        assert!(std::ptr::eq(d1.node(a).unwrap(), d2.node(a).unwrap()), "the other squares are the same objects");
        let let_go = |_: &Node| None;
        let second = PluginOverlays::collect(Some(&first), &d2, &opts, 100, NOW, &let_go);
        assert_eq!((starts(&second), second.waiting), (ids(&[a, c]), Some(NOW.generation)));
        assert!(second.stands(&d2, &opts, NOW));
        let later = Plugins { generation: 2, ..NOW };
        assert!(!second.stands(&d2, &opts, later), "the cache has changed: look again");
        let third = PluginOverlays::collect(Some(&second), &d2, &opts, 100, later, &cached);
        assert_eq!((starts(&third), third.waiting), (ids(&[a, b, c]), None));
        // Other plug-ins: nothing is kept.
        let reinstalled = Plugins { revision: 2, ..later };
        assert!(PluginOverlays::collect(Some(&third), &d2, &opts, 100, reinstalled, &let_go).lines().next().is_none());
    }

    /// Collecting stops once the overlays reach the points drawn: those past them aren't kept, nor
    /// waited for.
    #[test]
    fn collecting_stops_at_the_points_drawn() {
        let (d, _) = three_squares();
        let o = PluginOverlays::collect(None, &d, &RenderOptions::default(), 100, NOW, &|_| Some(line_at(0.0, 60)));
        assert_eq!((o.objects.len(), o.waiting), (2, None), "60 points, then 120");
    }
}
