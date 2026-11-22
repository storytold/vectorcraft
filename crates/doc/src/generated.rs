//! Generated art objects: a group whose children a generator plug-in drew from its parameters
//! (`plugin.generate`). The group keeps the plug-in's id and name, the parameters it was drawn with
//! and the transform its art has had since, so `plugin.editGenerated` can run the plug-in again and
//! replace the art. Its children are ordinary objects for every renderer and exporter, so the art
//! draws, prints and exports whether or not the plug-in is installed.
//!
//! A file holds the fields beside a SHA-256 of the art as it was written ([`art_hash`]). Reading
//! never runs a plug-in. A field that can't be read (an id that is not a plug-in id, parameters
//! that are not an object or are past [`MAX_PARAMS_BYTES`], a transform that is not one) marks the
//! object [`State::Damaged`], and art that doesn't match the hash marks it [`State::Changed`]: the
//! art stays as it was read either way. A damaged object is saved with `damaged: true`, so it stays
//! damaged when the file is opened again.

use serde::Serialize;
use serde_json::{Map, Value};
use vectorcraft_geom::Affine;

use crate::Effect;
use crate::node::{Node, NodeKind, Scaling};

/// Most bytes a generated art object's parameters take, written as JSON.
pub const MAX_PARAMS_BYTES: usize = 64 << 10;

/// Most bytes the art of a generated art object takes when a generator draws it, written as JSON.
pub const MAX_ART_BYTES: usize = 16 << 20;

/// Longest plug-in id, in bytes, as plug-in manifests limit it.
pub const MAX_ID: usize = 64;

/// Longest plug-in name kept, in characters, as plug-in manifests limit it.
pub const MAX_NAME: usize = 64;

/// How the fields of a generated art object were read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// Made in this session, or read with every field and the art's hash intact.
    #[default]
    Intact,
    /// A field could not be read: the plug-in id is not one, or the parameters or the transform
    /// are not usable. The art can't be drawn again.
    Damaged,
    /// The fields were read, but the art doesn't match the hash the file gave for it.
    Changed,
}

/// The settings of a generated art object ([`crate::NodeExtras::generated`]).
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedArt {
    /// The id of the plug-in that drew it.
    pub plugin: String,
    /// The plug-in's name when it drew it, to name the plug-in while it is not installed.
    pub name: String,
    /// The plug-in's parameters it was drawn with, each one given.
    pub params: Map<String, Value>,
    /// The transform its art has had since it was drawn (moves, scales, rotations, reflections
    /// and shears).
    pub transform: Affine,
    /// What its art's stroke weights and dashes have been scaled by since it was drawn (the
    /// transforms made with Scale Strokes & Effects on).
    pub stroke_scale: f64,
    /// What its art's effect distances have been scaled by since it was drawn.
    pub effect_scale: f64,
    /// The transform its art's pattern tiles have had since it was drawn (the transforms made with
    /// Transform Patterns on).
    pub pattern_transform: Affine,
    /// How it was read; not written to files.
    pub state: State,
}

impl GeneratedArt {
    /// The settings of art drawn by plug-in `plugin` (named `name`) with `params`, not transformed
    /// since.
    pub fn new(plugin: &str, name: &str, params: Map<String, Value>) -> Self {
        Self {
            plugin: plugin.into(),
            name: name.chars().take(MAX_NAME).collect(),
            params,
            transform: Affine::IDENTITY,
            stroke_scale: 1.0,
            effect_scale: 1.0,
            pattern_transform: Affine::IDENTITY,
            state: State::Intact,
        }
    }

    /// Its art transformed by `a`, with what `sc` says scales besides its geometry.
    pub fn transform(&mut self, a: Affine, sc: &Scaling) {
        self.transform = a * self.transform;
        if let Some(k) = Scaling::factor(a)
            && sc.strokes
        {
            self.stroke_scale *= k;
            if sc.effects.is_some() {
                self.effect_scale *= k;
            }
        }
        if sc.patterns {
            self.pattern_transform = a * self.pattern_transform;
        }
    }

    /// Places `art`, drawn by the generator as it is drawn untransformed, as the object's art has
    /// been transformed since it was drawn: its geometry, stroke weights, effect distances and
    /// pattern tiles. `scale` scales an effect's distance parameters (the effects catalogue's).
    pub fn place(&self, art: &mut Node, scale: fn(&mut Effect, f64)) {
        if self.transform != Affine::IDENTITY {
            art.transform(self.transform, false);
        }
        self.restyle(art, scale);
    }

    /// Scales the strokes and effects of `n` and its descendants, and transforms their pattern
    /// tiles, as [`Self::place`] says.
    fn restyle(&self, n: &mut Node, scale: fn(&mut Effect, f64)) {
        if self.stroke_scale != 1.0 {
            n.appearance.scale_strokes(self.stroke_scale);
        }
        if self.effect_scale != 1.0 {
            n.appearance.scale_effects(self.effect_scale, scale);
        }
        if self.pattern_transform != Affine::IDENTITY {
            crate::pattern::transform_pattern_paints(n, self.pattern_transform);
        }
        if let Some(ch) = n.children_mut() {
            for c in ch.iter_mut() {
                self.restyle(std::sync::Arc::make_mut(c), scale);
            }
        }
    }

    /// Can its art be drawn again: are its fields intact or only its art changed?
    pub fn redrawable(&self) -> bool {
        self.state != State::Damaged
    }
}

/// Is `id` shaped as a plug-in id: 1 to [`MAX_ID`] characters from `A-Z a-z 0-9 . _ -`?
pub fn valid_id(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The SHA-256 of `children` written as JSON, in lower-case hexadecimal: what a file holds as the
/// hash of a generated art object's art.
pub fn art_hash(children: &[std::sync::Arc<Node>]) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(children).unwrap_or_default();
    Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes `children` take written as JSON ([`MAX_ART_BYTES`]).
pub fn art_bytes(children: &[std::sync::Arc<Node>]) -> usize {
    serde_json::to_vec(children).map_or(usize::MAX, |b| b.len())
}

/// How a file holds a generated art object's settings, borrowed from the node.
#[derive(Serialize)]
pub(crate) struct Wire<'a> {
    plugin: &'a str,
    name: &'a str,
    params: &'a Map<String, Value>,
    #[serde(skip_serializing_if = "is_identity")]
    transform: &'a Affine,
    #[serde(rename = "strokeScale", skip_serializing_if = "is_one")]
    stroke_scale: f64,
    #[serde(rename = "effectScale", skip_serializing_if = "is_one")]
    effect_scale: f64,
    #[serde(rename = "patternTransform", skip_serializing_if = "is_identity")]
    pattern_transform: &'a Affine,
    hash: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    damaged: bool,
}

fn is_identity(a: &&Affine) -> bool {
    **a == Affine::IDENTITY
}

fn is_one(k: &f64) -> bool {
    *k == 1.0
}

impl<'a> Wire<'a> {
    /// The settings `g` of node `n`, with the hash of its art.
    pub(crate) fn of(g: &'a GeneratedArt, n: &Node) -> Self {
        let hash = art_hash(n.children().map_or(&[][..], Vec::as_slice));
        Wire {
            plugin: &g.plugin,
            name: &g.name,
            params: &g.params,
            transform: &g.transform,
            stroke_scale: g.stroke_scale,
            effect_scale: g.effect_scale,
            pattern_transform: &g.pattern_transform,
            hash,
            damaged: g.state == State::Damaged,
        }
    }
}

/// A saved transform `t` (absent: none): `None` when it is not six finite numbers.
fn read_affine(t: Option<Value>) -> Option<Affine> {
    match t {
        None => Some(Affine::IDENTITY),
        Some(t) => serde_json::from_value::<Affine>(t).ok().filter(|a| a.as_coeffs().iter().all(|c| c.is_finite())),
    }
}

/// A saved scale `k` (absent: 1): `None` when it is not a finite number above 0.
fn read_scale(k: Option<Value>) -> Option<f64> {
    match k {
        None => Some(1.0),
        Some(k) => k.as_f64().filter(|k| k.is_finite() && *k > 0.0),
    }
}

/// The settings a file gives (`v`, its `generated` field) for a node of `kind`: `None` for anything
/// but a group, else what could be read, marked [`State::Damaged`] or [`State::Changed`] when a
/// field or the art's hash doesn't hold up. A field that can't be read is kept empty: an id is
/// kept only when it is a plug-in id, a name at most [`MAX_NAME`] characters long.
pub(crate) fn read(v: Value, kind: &NodeKind) -> Option<Box<GeneratedArt>> {
    let NodeKind::Group { children, .. } = kind else { return None };
    let Value::Object(mut o) = v else {
        return Some(Box::new(GeneratedArt { state: State::Damaged, ..GeneratedArt::new("", "", Map::new()) }));
    };
    let mut damaged = !matches!(o.remove("damaged"), None | Some(Value::Bool(false)));
    let plugin = match o.remove("plugin") {
        Some(Value::String(id)) if valid_id(&id) => id,
        _ => {
            damaged = true;
            String::new()
        }
    };
    let name: String = match o.remove("name") {
        Some(Value::String(n)) => n.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect(),
        _ => String::new(),
    };
    let params = match o.remove("params") {
        Some(Value::Object(p)) if serde_json::to_vec(&p).is_ok_and(|b| b.len() <= MAX_PARAMS_BYTES) => p,
        _ => {
            damaged = true;
            Map::new()
        }
    };
    let mut affine = |key: &str| {
        read_affine(o.remove(key)).unwrap_or_else(|| {
            damaged = true;
            Affine::IDENTITY
        })
    };
    let transform = affine("transform");
    let pattern_transform = affine("patternTransform");
    let mut scale = |key: &str| {
        read_scale(o.remove(key)).unwrap_or_else(|| {
            damaged = true;
            1.0
        })
    };
    let stroke_scale = scale("strokeScale");
    let effect_scale = scale("effectScale");
    let intact = matches!(o.get("hash"), Some(Value::String(h)) if *h == art_hash(children));
    let state = if damaged {
        State::Damaged
    } else if intact {
        State::Intact
    } else {
        State::Changed
    };
    Some(Box::new(GeneratedArt { plugin, name, params, transform, stroke_scale, effect_scale, pattern_transform, state }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeId;
    use serde_json::json;
    use std::sync::Arc;
    use vectorcraft_geom::{Rect, shapes};

    fn art() -> Node {
        let square = Node::path(NodeId(2), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Default::default());
        let mut g = Node::group(NodeId(1), vec![Arc::new(square)]);
        let mut params = Map::new();
        params.insert("size".into(), json!(10));
        g.extras_mut().generated = Some(Box::new(GeneratedArt::new("org.example.gen", "Squares", params)));
        g
    }

    fn reread(v: &Value) -> Node {
        serde_json::from_value(v.clone()).unwrap()
    }

    /// A generated art object writes its plug-in, name, parameters and the hash of its art, and
    /// reads back the same, intact.
    #[test]
    fn a_generated_art_object_round_trips_with_the_hash_of_its_art() {
        let g = art();
        let v = serde_json::to_value(&g).unwrap();
        let saved = &v["generated"];
        assert_eq!((&saved["plugin"], &saved["name"], &saved["params"]), (&json!("org.example.gen"), &json!("Squares"), &json!({"size": 10})));
        assert_eq!(saved["hash"], json!(art_hash(g.children().unwrap())));
        assert!(saved.get("transform").is_none());
        let back = reread(&v);
        assert_eq!(back, g);
        assert_eq!(back.extras().generated.as_ref().map(|a| a.state), Some(State::Intact));
        // A moved object keeps its transform.
        let mut moved = g.clone();
        moved.transform(Affine::translate((5.0, 0.0)), false);
        let back = reread(&serde_json::to_value(&moved).unwrap());
        assert_eq!(back.extras().generated.as_ref().map(|a| (a.transform, a.state)), Some((Affine::translate((5.0, 0.0)), State::Intact)));
        // A scale with Scale Strokes & Effects and Transform Patterns keeps what it scaled.
        let mut scaled = g.clone();
        let sc = Scaling { strokes: true, patterns: true, ..Scaling::default() };
        scaled.transform(Affine::scale(2.0), sc);
        scaled.transform(Affine::scale(3.0), false);
        let back = reread(&serde_json::to_value(&scaled).unwrap());
        let a = back.extras().generated.as_deref().unwrap();
        assert_eq!((a.stroke_scale, a.effect_scale, a.pattern_transform, a.state), (2.0, 1.0, Affine::scale(2.0), State::Intact));
        assert_eq!(a.transform, Affine::scale(6.0));
    }

    /// Art that doesn't match the file's hash, and fields that can't be read, keep the art and say
    /// how the object was read; a garbled id is never kept.
    #[test]
    fn damaged_fields_and_changed_art_keep_the_art() {
        let g = art();
        let v = serde_json::to_value(&g).unwrap();
        let state = |v: &Value| reread(v).extras().generated.as_ref().map(|a| (a.plugin.clone(), a.state));
        // The art edited, with the hash of the art before.
        let mut edited = g.clone();
        if let Some(c) = edited.children_mut().and_then(|c| c.first_mut()) {
            Arc::make_mut(c).transform(Affine::translate((1.0, 0.0)), false);
        }
        let mut changed = serde_json::to_value(&edited).unwrap();
        changed["generated"]["hash"] = v["generated"]["hash"].clone();
        assert_eq!(state(&changed), Some(("org.example.gen".into(), State::Changed)));
        let mut no_hash = v.clone();
        no_hash["generated"].as_object_mut().unwrap().remove("hash");
        assert_eq!(state(&no_hash), Some(("org.example.gen".into(), State::Changed)));
        for (field, bad) in [
            ("plugin", json!("../evil")),
            ("plugin", json!("x".repeat(65))),
            ("plugin", json!("")),
            ("plugin", json!("a\u{0}b")),
            ("plugin", json!(7)),
            ("params", json!([1, 2])),
            ("params", json!({"blob": "x".repeat(MAX_PARAMS_BYTES)})),
            ("transform", json!("sideways")),
            ("transform", Value::Null),
            ("strokeScale", json!(-2.0)),
            ("strokeScale", Value::Null),
            ("strokeScale", json!("big")),
            ("effectScale", json!(0.0)),
            ("patternTransform", json!([1, 0, 0, 1, 0])),
            ("patternTransform", Value::Null),
            ("effectScale", Value::Null),
            ("damaged", json!(true)),
            ("damaged", json!("no")),
        ] {
            let mut w = v.clone();
            w["generated"][field] = bad;
            let back = reread(&w);
            let a = back.extras().generated.as_ref().unwrap();
            assert_eq!(a.state, State::Damaged, "{field}");
            assert!(a.plugin.is_empty() || a.plugin == "org.example.gen", "{field}: {}", a.plugin);
            assert_eq!(back.children(), g.children(), "{field}");
            // Saved again and read back, it stays damaged.
            let again = reread(&serde_json::to_value(&back).unwrap());
            assert_eq!(again.extras().generated.as_ref().map(|a| a.state), Some(State::Damaged), "{field} saved again");
        }
        // Not an object at all.
        let mut w = v.clone();
        w["generated"] = json!("art");
        assert_eq!(state(&w), Some((String::new(), State::Damaged)));
        // On a path the field is dropped, and the path stays as it was.
        let path = g.children().unwrap()[0].clone();
        let mut p = serde_json::to_value(&*path).unwrap();
        p["generated"] = v["generated"].clone();
        let back = reread(&p);
        assert!(back.extras().generated.is_none());
        assert_eq!(back, *path);
        // A name is cut to its limit and loses control characters.
        let mut w = v.clone();
        w["generated"]["name"] = json!(format!("A\u{202e}\n{}", "n".repeat(100)));
        let back = reread(&w);
        let name = &back.extras().generated.as_ref().unwrap().name;
        assert_eq!(name.chars().count(), MAX_NAME);
        assert!(!name.chars().any(char::is_control));
    }
}
