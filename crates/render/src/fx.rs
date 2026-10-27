//! Live effects in the renderer.
//!
//! Geometry effects (object-level, then per fill/stroke) rewrite the path before painting via
//! `vectorcraft-effects`. Raster effects use vello_cpu filter layers. Those only work in
//! single-threaded contexts, so on the multithreaded pipeline each filter layer is rendered
//! offscreen on the calling thread (cropped to its reach) and composited back with the same blend
//! mode and opacity ([`Renderer::with_filters`]):
//!
//! - Drop Shadow / Outer Glow: the object's silhouette in a `DropShadowOnly` filter layer, painted
//!   below the object with the effect's blend mode and opacity;
//! - Gaussian Blur: the object painted inside a Gaussian filter layer;
//! - Feather: like Gaussian Blur but clipped to the shape, so the edge fades inward;
//! - Inner Glow: a blurred inverse silhouette (Edge) or the blurred silhouette (Center), clipped
//!   to the shape and painted above it;
//! - Photoshop-style filters (Radial Blur, Smart Blur, Unsharp Mask…): the content is rendered
//!   offscreen and its pixels run through the content-changing effects in stack order (those
//!   filters, Gaussian Blur and Feather, see [`effects::pixel`]), then drawn back
//!   ([`Renderer::pixel_content`]). The result is cached per object while it and the zoom don't
//!   change.
//! - Rasterize: the content drawn again on the image's own grid, as Object › Rasterize draws it
//!   ([`Renderer::rasterize_image`]), then into the content's raster
//!   ([`effects::Rasterize::compose`]; an image too large for that is made from the content's
//!   raster, [`effects::Rasterize::apply`]), clipped to its outline with Create Clipping Mask. The
//!   effects before it in the stack, and the shadows and glows after it, are drawn around it by
//!   splitting the stack there ([`Renderer::raster_fx`]).
//!
//! While [`crate::RenderOptions::interacting`], an object whose content only moved draws its
//! cached shadows, glows and filtered content at its new place instead of running the effects
//! again ([`Renderer::move_shadow`], [`Renderer::move_pixel_content`]).
//!
//! While [`crate::RenderOptions::dragging`] is set, the renderer draws filtered content that runs
//! its effects within [`DRAG_PIXEL_BUDGET`] pixels, at a lower resolution where it would need more,
//! and keeps no raster made that way. On the multithreaded pipeline it does the same for shadows
//! and glows. The frame after the drag renders them as a still frame does.
//!
//! A cached raster is made again when something its content reads from outside the object
//! changes ([`Inputs`]): the pattern definitions, symbols, images and brushes it uses, the
//! document's raster resolution and color mode, the substitution highlights, the knockout its
//! object inherits, the color settings and fonts, the plug-ins its plug-in effects run, and the options
//! and layer settings it is drawn with.
//!
//! The same applies to one fill or stroke's own raster effects (around that item alone) and to
//! type, images, symbol instances and live objects ([`Renderer::draw_object_fx`]): their art is
//! drawn offscreen in the filter layers, reshaped first by their geometry effects. Groups and
//! layers with an appearance of their own take the same path with their evaluated art
//! ([`effects::evaluate_container`]: their fills and strokes painting the members, their geometry
//! effects reshaping them), so their raster effects apply to the composite (one combined shadow).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Weak};

use vectorcraft_color::cms::Cms;
use vectorcraft_doc::{AppearanceItem, Document, Effect, Node, NodeId, NodeKind, SymbolBoxes};
use vectorcraft_effects::pixel::{Channels, PixelFx};
use vectorcraft_effects::{self as effects, GeomContext, RasterFx};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathEl, Point, Rect, Shape, Size, Vec2};
use vello_common::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::RenderContext;
use vello_cpu::peniko;

use crate::ink::Ink;
use crate::{Frame, Renderer, blend_mode, fill_rule, paint};

mod bounds;
mod entries;
mod pixels;
mod predicates;
mod raster;
mod renderer;

#[cfg(test)]
pub(crate) use bounds::visual_bounds;
pub(crate) use bounds::{cull_bounds, drawn_outline, painted_reach};
#[cfg(test)]
pub(crate) use entries::Drawn;
pub(crate) use entries::{DocInputs, FxGeom, Reads, ShadowEntry};
pub(crate) use predicates::{has_fx, has_object_fx};
pub(crate) use raster::PixelEntry;
#[cfg(test)]
pub(crate) use raster::RASTERIZED;
#[cfg(test)]
use raster::{DRAG_PIXEL_BUDGET, PIXEL_BUDGET, drag_scale, filter_region, filter_region_within, pan_margin};

#[cfg(test)]
mod tests;
