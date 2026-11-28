//! The Twist effect's angle in saved documents. A positive angle turns clockwise on the page in
//! VectorCraft 0.21.0 and later; earlier releases turned it counterclockwise. Documents of version
//! 2 ([`TWIST_CLOCKWISE_SINCE`]) turn clockwise. Version 1 documents written before 0.21.0 have
//! their Twist angles negated when they open, so they draw as they were saved
//! ([`Document::migrate_twist_angles`], [`written_clockwise`]), and a save for an earlier app
//! negates them back ([`Document::store_legacy_twist_angles`]). Graphic style library files
//! (`.vcstyles`) of version 1 have their Twist angles negated when they are read
//! ([`crate::style_libs::read`]); [`crate::style_libs::write`] writes version 2.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::style_libs::StyleLibrary;
use crate::{Appearance, Document, Effect, Node};

/// The Twist effect's id.
pub const TWIST: &str = "distort.twist";

/// The first document version whose Twist angles turn clockwise for a positive value.
pub const TWIST_CLOCKWISE_SINCE: u32 = 2;

/// The first release (major, minor) whose Twist turned clockwise for a positive angle.
const CLOCKWISE_RELEASE: (u64, u64) = (0, 21);

/// Did the app named by a native file's `generator` (`"VectorCraft 0.21.0"`) turn Twist effects
/// clockwise for a positive angle? It did from 0.21.0. Any other value, or none, is an earlier
/// app (DrawCraft included).
pub fn written_clockwise(generator: Option<&str>) -> bool {
    let Some(version) = generator.and_then(|g| g.strip_prefix("VectorCraft ")) else { return false };
    let mut parts = version.split('.').map(|p| p.parse::<u64>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) >= CLOCKWISE_RELEASE,
        _ => false,
    }
}

/// The angle a Twist effect without a readable one draws with (`vectorcraft-effects` reads a
/// missing or unreadable `angle` as this).
const DEFAULT_ANGLE: f64 = 10.0;

/// The key of `Document::unknown` that holds the brush definitions (`vectorcraft-brush`'s
/// `DOC_KEY`), whose art can carry effects.
const BRUSHES_KEY: &str = "brushes";

/// The angle a Twist effect with `params` draws with, read as `vectorcraft-effects` reads it: a
/// number, the number a string starts with, or a boolean as 1 or 0; anything else (or a value
/// that is not finite) is [`DEFAULT_ANGLE`].
fn angle(params: &Value) -> f64 {
    let v = match params.get("angle") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => {
            let t: String = s.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')).collect();
            t.parse().ok()
        }
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    };
    v.filter(|v| v.is_finite()).unwrap_or(DEFAULT_ANGLE)
}

/// Write the negated angle a Twist effect with `params` draws with into `params`, as a number.
fn negate_params(params: &mut Value) {
    // `0.0 - a` rather than `-a`: an angle of 0 stays 0, not -0.
    let turned = Value::from(0.0 - angle(params));
    match params {
        Value::Object(m) => {
            m.insert("angle".into(), turned);
        }
        p => *p = json!({ "angle": turned }),
    }
}

fn has_twist(fx: &[Effect]) -> bool {
    fx.iter().any(|e| e.id == TWIST)
}

fn appearance_has_twist(a: &Appearance) -> bool {
    has_twist(&a.effects) || a.items.iter().any(|i| has_twist(i.effects()))
}

fn negate_appearance(a: &mut Appearance) {
    for e in a.effects.iter_mut().chain(a.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut())) {
        if e.id == TWIST {
            negate_params(&mut e.params);
        }
    }
}

/// The appearances a Shaper Group's face and edge edits paint with.
fn shaper_appearances(n: &Node) -> impl Iterator<Item = &Appearance> {
    let s = n.extras().shaper.as_deref();
    let faces = s.into_iter().flat_map(|s| s.faces.iter().filter_map(|f| f.paint.as_ref()));
    let edges = s.into_iter().flat_map(|s| s.edges.iter().filter_map(|e| e.paint.as_ref()));
    faces.chain(edges)
}

/// Does `n`'s subtree (opacity-mask art and Shaper edits included) hold a Twist effect?
fn node_has_twist(n: &Node) -> bool {
    appearance_has_twist(&n.appearance)
        || n.extras().mask.as_ref().is_some_and(|m| node_has_twist(&m.art))
        || shaper_appearances(n).any(appearance_has_twist)
        || n.children().is_some_and(|c| c.iter().any(|c| node_has_twist(c)))
}

/// Negate the Twist angles of `a`'s subtree, copying only the nodes on the way to them.
fn negate_node(a: &mut Arc<Node>) {
    if !node_has_twist(a) {
        return;
    }
    let n = Arc::make_mut(a);
    if appearance_has_twist(&n.appearance) {
        negate_appearance(&mut n.appearance);
    }
    if let Some(x) = n.extras_if_any() {
        if let Some(m) = x.mask.as_mut() {
            negate_node(&mut m.art);
        }
        if let Some(s) = x.shaper.as_mut() {
            let faces = s.faces.iter_mut().filter_map(|f| f.paint.as_mut());
            for p in faces.chain(s.edges.iter_mut().filter_map(|e| e.paint.as_mut())) {
                negate_appearance(p);
            }
        }
    }
    for c in n.children_mut().into_iter().flatten() {
        negate_node(c);
    }
}

fn is_twist_json(m: &Map<String, Value>) -> bool {
    m.get("id").and_then(Value::as_str) == Some(TWIST)
}

/// Does JSON `v` (brush definitions) hold a Twist effect?
fn json_has_twist(v: &Value) -> bool {
    match v {
        Value::Object(m) => is_twist_json(m) || m.values().any(json_has_twist),
        Value::Array(a) => a.iter().any(json_has_twist),
        _ => false,
    }
}

/// Negate the Twist angles in JSON `v`: every object whose `id` is [`TWIST`] is a serialized
/// [`Effect`] (object ids are numbers).
fn negate_json(v: &mut Value) {
    match v {
        Value::Object(m) if is_twist_json(m) => negate_params(m.entry("params").or_insert(Value::Null)),
        Value::Object(m) => m.values_mut().for_each(negate_json),
        Value::Array(a) => a.iter_mut().for_each(negate_json),
        _ => {}
    }
}

/// Negate every Twist angle of a graphic style library: its styles' and its patterns' art.
pub(crate) fn negate_library(lib: &mut StyleLibrary) {
    for g in &mut lib.styles {
        negate_appearance(&mut g.appearance);
    }
    for p in &mut lib.patterns {
        p.art.iter_mut().for_each(negate_node);
    }
}

impl Document {
    /// A document before version [`TWIST_CLOCKWISE_SINCE`] that holds a Twist effect: when
    /// `written_clockwise` (asked only then; see [`written_clockwise`]) says the app that saved it
    /// turned Twist counterclockwise, negate its Twist angles, so it draws as it was saved. A later
    /// version is left as it is, and the document's version is not changed. It recurses through
    /// the art, once per level of nesting.
    pub fn migrate_twist_angles(&mut self, written_clockwise: impl FnOnce() -> bool) {
        if self.version >= TWIST_CLOCKWISE_SINCE || !self.has_twist() || written_clockwise() {
            return;
        }
        self.negate_twist_angles();
    }

    /// The reverse of [`Document::migrate_twist_angles`], for saving for an app that turned Twist
    /// counterclockwise: negate the Twist angles and mark the document version 1. Call it on a
    /// document whose angles turn clockwise, as every opened or new document's do, whatever
    /// version another conversion for that save has marked it already.
    pub fn store_legacy_twist_angles(&mut self) {
        self.version = TWIST_CLOCKWISE_SINCE - 1;
        if self.has_twist() {
            self.negate_twist_angles();
        }
    }

    /// Does any Twist effect sit where [`Document::negate_twist_angles`] negates them?
    fn has_twist(&self) -> bool {
        let mut roots = self
            .layers
            .iter()
            .chain(self.symbols.iter().map(|s| &s.art))
            .chain(self.graph_designs.iter().map(|g| &g.art))
            .chain(self.patterns.iter().flat_map(|p| p.art.iter()))
            .chain(self.pattern_edit.iter().filter_map(|pe| pe.original.as_ref()).flat_map(|p| p.art.iter()));
        roots.any(|r| node_has_twist(r))
            || self.graphic_styles.iter().any(|g| appearance_has_twist(&g.appearance))
            || self.unknown.get(BRUSHES_KEY).is_some_and(json_has_twist)
    }

    /// Negate every Twist angle: in the art of the layers, symbols, graph designs, pattern
    /// swatches and the pattern definition that pattern editing's Cancel puts back (opacity-mask
    /// art and Shaper edits included), in the graphic styles and in the brush definitions.
    fn negate_twist_angles(&mut self) {
        let roots = self
            .layers
            .iter_mut()
            .chain(self.symbols.iter_mut().map(|s| &mut s.art))
            .chain(self.graph_designs.iter_mut().map(|g| &mut g.art))
            .chain(self.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
            .chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()).flat_map(|p| p.art.iter_mut()));
        for root in roots {
            negate_node(root);
        }
        if self.graphic_styles.iter().any(|g| appearance_has_twist(&g.appearance)) {
            for g in self.graphic_styles.iter_mut() {
                negate_appearance(&mut g.appearance);
            }
        }
        if self.unknown.get(BRUSHES_KEY).is_some_and(json_has_twist)
            && let Some(b) = self.unknown.get_mut(BRUSHES_KEY)
        {
            negate_json(b);
        }
    }
}

#[cfg(test)]
mod tests;
