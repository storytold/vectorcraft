//! Concept to Vector providers: what makes vector variations of a selected image.
//!
//! A provider ([`Provider`]) takes one image's encoded pixels and up to [`MAX_STYLES`] of its own
//! style ids ([`Request`]), and returns one [`Variation`] per style, in the order asked, as paths
//! in the image's pixel space with explicit paints. It sees nothing else of the session or the
//! document. The session keeps the providers in a [`Registry`] and calls one only by its id: an
//! unknown or incapable provider is an error, never a substitute.
//!
//! The host checks every result again ([`check_result`]) before it keeps it, whatever provider
//! made it: the number and order of the variations, finite coordinates and widths within bounds,
//! and the totals [`MAX_ANCHORS`] and [`MAX_PATHS`] across all styles.
//!
//! The one built-in provider ([`offline::ID`]) makes its styles with Image Trace, on this
//! computer: it takes no prompt and makes vector art only.

pub mod offline;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use vectorcraft_geom::{PathData, Point};

/// The most styles one request may ask for.
pub const MAX_STYLES: usize = 8;

/// The most anchor points all the variations of one request may hold together.
pub const MAX_ANCHORS: usize = 1_000_000;

/// The most paths all the variations of one request may hold together.
pub const MAX_PATHS: usize = 250_000;

/// The most warnings a variation may carry, and the most characters in each.
pub const MAX_WARNINGS: usize = 16;
const MAX_WARNING_CHARS: usize = 500;

/// One of a provider's styles: its stable id and its English name.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Style {
    pub id: &'static str,
    pub label: &'static str,
}

/// What a provider takes and makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// It takes an image as its input.
    pub image_input: bool,
    /// It makes editable vector paths.
    pub vector_output: bool,
    /// It takes a text prompt.
    pub prompt: bool,
    /// It makes raster images.
    pub raster_output: bool,
    /// It runs on this computer without a network.
    pub offline: bool,
    /// Its styles, in the order it lists them.
    pub styles: Vec<Style>,
}

/// The kind of result a request asks for. `conceptVector.generate` asks only for
/// [`Output::Vector`]: a [`Variation`] holds vector paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Vector,
    Raster,
}

/// One request: the encoded pixels of an image, and how to make its variations.
#[derive(Clone, Debug)]
pub struct Request {
    /// The image's encoded bytes (PNG, JPEG, WebP or GIF), as the document holds them.
    pub image: Arc<Vec<u8>>,
    /// The image's size in pixels: the space the variations' paths are in.
    pub width: u32,
    pub height: u32,
    /// The prompt, trimmed; `None` when there is none.
    pub prompt: Option<String>,
    /// The provider's style ids, 1 to [`MAX_STYLES`] of them, each once.
    pub styles: Vec<String>,
    /// How much of the image's detail to keep, 0 (least) to 100 (most).
    pub detail: u8,
    pub output: Output,
}

/// One path of a variation, in the image's pixel space (`(0, 0)` top left, `(width, height)` bottom
/// right).
#[derive(Clone, Debug, PartialEq)]
pub struct VectorPath {
    pub path: PathData,
    /// The fill color (sRGB), if it is filled.
    pub fill: Option<[u8; 3]>,
    /// The stroke color (sRGB) and width in pixels, if it is stroked.
    pub stroke: Option<([u8; 3], f64)>,
    /// Stroked: the stroke's ends are square (cut straight half the width past the path's end);
    /// otherwise round.
    pub square_caps: bool,
}

/// The variation a provider made for one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Variation {
    /// The style id it was made for.
    pub style: String,
    /// Its paths, bottom-most first.
    pub paths: Vec<VectorPath>,
    /// Notes for the person who asked (a scaled-down image, nothing traced).
    pub warnings: Vec<String>,
}

impl Variation {
    /// Its anchor points, every path counted.
    pub fn anchors(&self) -> usize {
        self.paths.iter().map(|p| p.path.anchor_count()).sum()
    }
}

/// Why a provider made nothing.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ProviderError {
    /// The image or the request cannot be used.
    #[error("{0}")]
    InvalidInput(String),
    /// The provider does not do what the request asks.
    #[error("{0}")]
    Unsupported(String),
    /// The provider cannot run now.
    #[error("{0}")]
    Unavailable(String),
    /// The caller stopped it.
    #[error("cancelled; nothing was changed")]
    Canceled,
    /// The results would pass a limit.
    #[error("the results would have more than {0} anchor points: lower Detail or choose fewer styles")]
    ResourceLimit(usize),
    /// The provider's results do not match the request, or are out of bounds.
    #[error("the provider's results do not match the request or are malformed")]
    InvalidResult,
    /// The provider stopped on a fault of its own.
    #[error("the provider stopped with an internal error: {0}")]
    Internal(String),
}

/// How far a provider has got: `steps(done, total)`; it stops with [`ProviderError::Canceled`]
/// once this answers `false`.
pub type Steps<'a> = &'a (dyn Fn(u64, u64) -> bool + Sync);

/// Something that makes vector variations of an image.
pub trait Provider: Send + Sync {
    /// Its stable id, such as `offline.traceStyles`.
    fn id(&self) -> &'static str;
    /// Its English name.
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    /// One variation for each of `request.styles`, in that order.
    fn generate(&self, request: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError>;
    /// What [`Provider::generate`] makes, made a part at a time ([`Parts`]), for a caller with no
    /// thread to wait on; `None` (the default) when it makes no parts smaller than one style.
    fn parts(&self, _request: &Request) -> Option<Box<dyn Parts>> {
        None
    }
}

/// A request's variations made a part at a time: each call of [`Parts::step`] does a part and
/// returns, so that the caller can run other work, or stop, between parts.
pub trait Parts: Send {
    /// Does the next part → the variations once the last part is done, `None` before it.
    fn step(&mut self) -> Result<Option<Vec<Variation>>, ProviderError>;
    /// The styles made so far.
    fn styles_done(&self) -> usize;
}

/// The providers a session can call, by id.
#[derive(Clone)]
pub struct Registry {
    providers: Vec<Registered>,
}

/// A provider in a [`Registry`], with the id, name and capabilities it gave when it was
/// registered.
#[derive(Clone)]
pub struct Registered {
    pub id: &'static str,
    pub name: &'static str,
    pub capabilities: Capabilities,
    pub provider: Arc<dyn Provider>,
}

impl Default for Registry {
    /// The built-in provider only.
    fn default() -> Self {
        let p = offline::TraceStyles;
        Self { providers: vec![Registered { id: p.id(), name: p.name(), capabilities: p.capabilities(), provider: Arc::new(p) }] }
    }
}

impl Registry {
    /// A registry with no provider.
    pub fn empty() -> Self {
        Self { providers: vec![] }
    }

    /// Add `provider`, reading its id, name and capabilities once; an error when one with its id
    /// is registered already, or when the provider panics while giving them.
    pub fn register(&mut self, provider: Arc<dyn Provider>) -> Result<(), String> {
        let (id, name, capabilities) = crate::guard::catch_panic(|| (provider.id(), provider.name(), provider.capabilities()))
            .map_err(|e| ProviderError::Internal(e).to_string())?;
        if self.get(id).is_some() {
            return Err(format!("a provider with the id `{id}` is registered already"));
        }
        self.providers.push(Registered { id, name, capabilities, provider });
        Ok(())
    }

    /// The provider `id` (exactly so spelled).
    pub fn get(&self, id: &str) -> Option<&Registered> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// The providers in the order they were registered.
    pub fn iter(&self) -> impl Iterator<Item = &Registered> {
        self.providers.iter()
    }
}

/// Whether `p` is a finite point within `limit` of the image's pixel space.
fn within(p: Point, limit: f64, size: (f64, f64)) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x >= -limit && p.y >= -limit && p.x <= size.0 + limit && p.y <= size.1 + limit
}

/// Checks the variations a provider returned for `request`: one per style in the order asked, at
/// most [`MAX_PATHS`] paths and [`MAX_ANCHORS`] anchors together, each path with an anchor and a
/// fill or a stroke, every anchor and handle finite and within one image side of the image, every
/// stroke width finite, over 0 and at most the image's larger side, and at most [`MAX_WARNINGS`]
/// warnings of at most 500 characters on each variation.
pub fn check_result(request: &Request, variations: &[Variation]) -> Result<(), ProviderError> {
    if variations.len() != request.styles.len() || variations.iter().zip(&request.styles).any(|(v, s)| v.style != *s) {
        return Err(ProviderError::InvalidResult);
    }
    let size = (f64::from(request.width.max(1)), f64::from(request.height.max(1)));
    let side = size.0.max(size.1);
    let (mut paths, mut anchors) = (0usize, 0usize);
    for v in variations {
        paths += v.paths.len();
        anchors += v.anchors();
        if anchors > MAX_ANCHORS {
            return Err(ProviderError::ResourceLimit(MAX_ANCHORS));
        }
        if paths > MAX_PATHS || v.warnings.len() > MAX_WARNINGS || v.warnings.iter().any(|w| w.chars().count() > MAX_WARNING_CHARS) {
            return Err(ProviderError::InvalidResult);
        }
        for p in &v.paths {
            let width_ok = p.stroke.is_none_or(|(_, w)| w.is_finite() && w > 0.0 && w <= side);
            let painted = p.fill.is_some() || p.stroke.is_some();
            let points_ok = p.path.anchors().all(|(_, _, a)| [a.p, a.h_in, a.h_out].into_iter().all(|q| within(q, side, size)));
            if !width_ok || !painted || p.path.anchor_count() == 0 || !points_ok {
                return Err(ProviderError::InvalidResult);
            }
        }
    }
    Ok(())
}
