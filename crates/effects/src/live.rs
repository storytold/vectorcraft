//! Live objects (blends, envelopes, gradient meshes, repeats) evaluated alike for the renderer and
//! every exporter, with what the pure evaluation in [`vectorcraft_doc::live`] can't do itself
//! ([`Hooks`]): type inside an envelope distorts as its glyph outlines, run by run
//! ([`outline_text`]); symbol instances as their symbol's art; with Distort Appearance strokes and
//! geometry effects bend with the art ([`bake_appearance`]); with Distort Pattern Fills the pattern
//! tiles do.

use std::sync::Arc;

use vectorcraft_doc::live::{self, Hooks, Outliner};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};

use crate::{bake_appearance, outline_text};

/// The text outliner live evaluation uses: type as its glyph outlines, one path per run in the
/// run's paint ([`outline_text`]).
pub fn text_outliner() -> Outliner<'static> {
    Some(&outline_text)
}

/// Type outlined, or a symbol instance as its symbol's art (from `doc`) placed by the instance.
fn outline(doc: Option<&Document>, n: &Node) -> Option<Node> {
    match &n.kind {
        NodeKind::SymbolInstance { symbol, xf } => {
            let sym = doc?.symbols.iter().find(|s| s.name == *symbol)?;
            let mut art = (*sym.art).clone();
            art.transform(*xf, false);
            Some(Node::group(n.id, vec![Arc::new(art)]))
        }
        _ => outline_text(n),
    }
}

/// Run `f` with the hooks live evaluation needs; symbols and patterns come from `doc` (without a
/// document, symbol instances and pattern tiles stay as they are).
fn with_hooks<R>(doc: Option<&Document>, f: impl FnOnce(Hooks) -> R) -> R {
    let outline = |n: &Node| outline(doc, n);
    let tiles = |name: &str, xf: Affine, region: Rect| Some(doc?.pattern(name)?.instances_in(xf, region));
    f(Hooks { outline: Some(&outline), appearance: Some(&bake_appearance), pattern: Some(&tiles), symbols: doc })
}

/// One level of evaluation of live object `n` ([`live::expand_live_hooks`]).
pub fn expand_live(doc: Option<&Document>, n: &Node) -> Vec<Node> {
    with_hooks(doc, |h| live::expand_live_hooks(n, h))
}

/// Live object `n` evaluated into a plain group ([`live::expanded_group_hooks`]): Expand.
pub fn expanded_live_group(doc: Option<&Document>, n: &Node) -> Node {
    with_hooks(doc, |h| live::expanded_group_hooks(n, h))
}

/// `n` with every live object in it replaced by its evaluated art ([`live::expand_deep_hooks`]):
/// what the exporters write for live objects.
pub fn expand_live_deep(doc: Option<&Document>, n: &Node) -> Node {
    with_hooks(doc, |h| live::expand_deep_hooks(n, h))
}
