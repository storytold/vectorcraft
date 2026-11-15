//! The shadow of the 3D and Materials effects (Extrude & Bevel, Inflate, Rotate and Revolve): the
//! `shadow` switch with `shadowPosition`, `shadowDistance` and `shadowBounds`, and the shape it
//! draws behind the solids.
use std::ops::Range;
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, FillLayer, Node, NodeId};
use vectorcraft_three_d::{Shadow, ShadowCaster, ShadowPlane, ShadowPosition};

use super::Scope;
use crate::util::{flag, num, text};

/// The shadow's paint: black at this opacity. VectorCraft's own values.
const SHADOW_OPACITY: f32 = 0.5;

/// The shadow `p` asks for, or `None` with the switch off or when the first light lights nothing
/// (Shading is off or its intensity is 0), so that no shadow is built.
pub(crate) fn options(p: &Value) -> Option<Shadow> {
    let intensity = crate::lighting::lights(p).and_then(|l| l.iter().next().map(|l| l.intensity)).unwrap_or_else(|| num(p, "lightIntensity", 80.0));
    let lit = flag(p, "shade", true) && intensity > 0.0;
    (lit && flag(p, "shadow", false)).then(|| Shadow {
        position: ShadowPosition::parse(text(p, "shadowPosition", "behind")).unwrap_or_default(),
        distance: num(p, "shadowDistance", 0.0) / 100.0,
        bounds: num(p, "shadowBounds", 100.0) / 100.0,
    })
}

/// Check the shadow settings in `p`: an error for a value out of its range or of the wrong kind.
pub(crate) fn validate(p: &Value) -> Result<(), String> {
    if p.get("shadow").is_some_and(|v| !v.is_boolean()) {
        return Err("shadow must be true or false".into());
    }
    if p.get("shadowPosition").is_some_and(|v| v.as_str().and_then(ShadowPosition::parse).is_none()) {
        return Err("shadowPosition must be behind or below".into());
    }
    for (key, min, max) in [("shadowDistance", 0.0, 100.0), ("shadowBounds", 10.0, 400.0)] {
        if p.get(key).is_some_and(|v| !v.as_f64().is_some_and(|n| n.is_finite() && (min..=max).contains(&n))) {
            return Err(format!("{key} must be a number from {min} to {max}"));
        }
    }
    Ok(())
}

/// The opacity and blending mode of the fill or stroke (and member) that casts part of a shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ink {
    pub(crate) opacity: f32,
    pub(crate) blend: BlendMode,
}

/// The shadow of the faces in `caster` as a path of object `id`, cast by art in `ink`: filled
/// black at [`SHADOW_OPACITY`] times its opacity, in its blending mode, with the nonzero rule;
/// `None` when it casts nothing.
pub(crate) fn art(id: NodeId, caster: &ShadowCaster, shadow: Shadow, ink: Ink) -> Option<Node> {
    let all = 0..caster.len();
    let all = std::slice::from_ref(&all);
    piece(id, caster, &caster.shadow_plane(shadow, all)?, all, ink)
}

/// [`art`] of the faces in the ranges `faces` of `caster` only, on `plane`.
fn piece(id: NodeId, caster: &ShadowCaster, plane: &ShadowPlane, faces: &[Range<usize>], ink: Ink) -> Option<Node> {
    let path = caster.path_on(plane, faces)?;
    let mut fill = FillLayer::new(Paint::solid(Color::BLACK));
    fill.opacity = SHADOW_OPACITY * ink.opacity.clamp(0.0, 1.0);
    fill.blend = ink.blend;
    Some(Node::path(id, path, Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() }))
}

/// Whether `a` and `b` are the same objects' scopes (each member holds its own copies).
fn same_scopes(a: &[Scope], b: &[Scope]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.key == b.key)
}

/// The faces one solid added to a caster, with the scopes of its member and the ink of its art.
pub(crate) type Cast<'a> = (&'a [Scope], Ink, Range<usize>);

/// The faces of the solids that cast one piece of a shadow, with their scopes and ink.
type Run<'a> = (&'a [Scope], Ink, Vec<Range<usize>>);

/// The shadow `cast` casts on one plane: the faces of `caster` that the solids of visible art
/// added, each range with the scopes of the member that cast it and the ink of the art, in order.
/// Each run of solids in the same ink casts a piece in that ink ([`art`]); with `masks` (a member's
/// opacity mask or blending mode applies to its solids) the solids of a run also share their scopes,
/// and each piece is drawn in their opacity masks and blending modes ([`super::masked`]).
pub(crate) fn scoped_art(id: NodeId, caster: &ShadowCaster, shadow: Shadow, cast: Vec<Cast<'_>>, masks: bool) -> Vec<Arc<Node>> {
    let placing: Vec<Range<usize>> = cast.iter().map(|(.., faces)| faces.clone()).collect();
    let Some(plane) = caster.shadow_plane(shadow, &placing) else { return vec![] };
    // Consecutive solids in the same ink (and with masks, the same scopes) share a piece; the pieces
    // keep the solids' order.
    let mut runs: Vec<Run> = vec![];
    for (scopes, ink, faces) in cast {
        match runs.last_mut() {
            Some(run) if run.1 == ink && (!masks || same_scopes(run.0, scopes)) => run.2.push(faces),
            _ => runs.push((scopes, ink, vec![faces])),
        }
    }
    let pieces = runs.into_iter().filter_map(|(scopes, ink, faces)| Some((scopes, Arc::new(piece(id, caster, &plane, &faces, ink)?))));
    if masks { super::masked(id, pieces.collect()) } else { pieces.map(|(_, piece)| piece).collect() }
}
