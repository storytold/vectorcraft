//! Keep form XObjects in page space. Quartz resolves shading patterns inside a form against the
//! page's initial space, whereas other readers also apply the transform at the form's invocation.
//! Moving that transform inside the form makes both interpretations agree (including gradients
//! in opacity groups, masks, layers and overprinting art), without rasterizing the art.
//!
//! krilla's page surface has an implicit y flip which `ctm()` does not include. Remember that
//! root separately; a form's surface has an identity root. A group still counts as one push/pop
//! for the drawing code, even though moving its transform takes three krilla pushes.

use std::ops::{Deref, DerefMut};

use craft_pdf::write::backend as krilla;
use krilla::blend::BlendMode;
use krilla::geom::{Path, Transform};
use krilla::mask::Mask;
use krilla::num::NormalizedF32;
use krilla::paint::FillRule;
use vectorcraft_geom::Affine;

pub(crate) fn xf(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32)
}

pub(crate) fn affine(t: Transform) -> Affine {
    Affine::new([t.sx(), t.ky(), t.kx(), t.sy(), t.tx(), t.ty()].map(f64::from))
}

struct Push {
    count: usize,
    root: Affine,
}

/// A drawing surface whose form groups keep gradients and masks in the same page space.
pub(crate) struct Surface<'a> {
    inner: krilla::surface::Surface<'a>,
    root: Affine,
    pushes: Vec<Push>,
}

impl<'a> Surface<'a> {
    /// Wrap a page while recording the writer's implicit Y flip outside its reported CTM.
    pub fn page(inner: krilla::surface::Surface<'a>, height: f32) -> Self {
        Self { inner, root: Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, height.into()]), pushes: vec![] }
    }

    /// Wrap a form or pattern stream, which starts without a page's implicit Y flip.
    pub fn stream(inner: krilla::surface::Surface<'a>) -> Self {
        Self { inner, root: Affine::IDENTITY, pushes: vec![] }
    }

    /// Current drawing space → page space, for both a form's art and its mask. A singular
    /// enclosing transform draws no area: keep that form local rather than trying to invert it.
    pub fn group_transform(&self) -> Transform {
        let t = xf(self.root * affine(self.inner.ctm()));
        if t.invert().is_some() { t } else { Transform::identity() }
    }

    /// Move the enclosing transform inside a form and record its three pushes as one group.
    fn group(&mut self, push: impl FnOnce(&mut krilla::surface::Surface)) {
        let t = self.group_transform();
        self.inner.push_transform(&t.invert().unwrap_or_default());
        push(&mut self.inner);
        self.inner.push_transform(&t);
        self.pushes.push(Push { count: 3, root: self.root });
        self.root = Affine::IDENTITY;
    }

    /// Apply opacity in page space when it creates a form; full opacity is a plain push.
    pub fn push_opacity(&mut self, opacity: NormalizedF32) {
        if opacity == NormalizedF32::ONE {
            // krilla does not create a form for full opacity.
            self.inner.push_opacity(opacity);
            self.plain_push();
        } else {
            self.group(|s| s.push_opacity(opacity));
        }
    }

    pub fn push_isolated(&mut self) {
        self.group(|s| s.push_isolated());
    }

    /// A transparency group with `options` (isolated, knockout), as [`Self::push_isolated`].
    pub fn push_group(&mut self, options: krilla::blend::GroupOptions) {
        self.group(|s| s.push_group(options));
    }

    /// The mask's stream is already in [`Self::group_transform`]'s space, like the group's art.
    pub fn push_mask(&mut self, mask: Mask) {
        self.group(|s| s.push_mask(mask));
    }

    fn plain_push(&mut self) {
        self.pushes.push(Push { count: 1, root: self.root });
    }

    pub fn push_transform(&mut self, transform: &Transform) {
        self.inner.push_transform(transform);
        self.plain_push();
    }

    pub fn push_clip_path(&mut self, path: &Path, rule: &FillRule) {
        self.inner.push_clip_path(path, rule);
        self.plain_push();
    }

    pub fn push_blend_mode(&mut self, mode: BlendMode) {
        self.inner.push_blend_mode(mode);
        self.plain_push();
    }

    /// Restore the enclosing coordinate space, undoing every writer push for the last group.
    /// An unmatched pop is logged rather than forwarded to the writer's stack.
    pub fn pop(&mut self) {
        let Some(push) = self.pushes.pop() else {
            log::error!("PDF surface pop without a matching push");
            return;
        };
        for _ in 0..push.count {
            self.inner.pop();
        }
        self.root = push.root;
    }

    pub fn finish(self) {
        self.inner.finish();
    }
}

// Drawing operations use krilla directly; every push/pop above keeps our coordinate state in
// step. Stream builders are wrapped with `Surface::stream` when their surfaces are created.
impl<'a> Deref for Surface<'a> {
    type Target = krilla::surface::Surface<'a>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Surface<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
