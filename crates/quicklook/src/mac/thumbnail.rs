//! The thumbnail extension's principal class: Finder asks it for a file's thumbnail, and it
//! answers with the file's embedded preview ([`crate::thumbnail`]) drawn into Quick Look's
//! context, or with an error (Finder then shows the document icon).

use std::ptr::NonNull;

use block2::{DynBlock, RcBlock};
use objc2::define_class;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject};
use objc2_core_foundation::{CFData, CFRetained, CGFloat, CGSize};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGImage, CGImageAlphaInfo, CGInterpolationQuality,
    kCGColorSpaceSRGB,
};
use objc2_foundation::{NSError, NSObjectProtocol};
use objc2_quick_look_thumbnailing::{QLFileThumbnailRequest, QLThumbnailProvider, QLThumbnailReply};

use super::{file_path, guarded, ns_error};
use crate::{MAX_FILE, Picture, QuickLookError, Result, read_file};

define_class!(
    /// `NSExtensionPrincipalClass` of `VectorCraftThumbnail.appex`.
    #[unsafe(super(QLThumbnailProvider, NSObject))]
    #[name = "VCThumbnailProvider"]
    pub struct ThumbnailProvider;

    unsafe impl NSObjectProtocol for ThumbnailProvider {}

    impl ThumbnailProvider {
        #[unsafe(method(provideThumbnailForFileRequest:completionHandler:))]
        fn provide_thumbnail(&self, request: &QLFileThumbnailRequest, handler: &DynBlock<dyn Fn(*mut QLThumbnailReply, *mut NSError)>) {
            match reply(request) {
                Ok(reply) => handler.call((Retained::as_ptr(&reply).cast_mut(), std::ptr::null_mut())),
                Err(e) => {
                    let error = ns_error(&e);
                    handler.call((std::ptr::null_mut(), Retained::as_ptr(&error).cast_mut()));
                }
            }
        }
    }
);

/// Finder's scale factor (2 on Retina screens), kept to a sane range.
fn scale_of(request: &QLFileThumbnailRequest) -> CGFloat {
    // SAFETY: a plain property read.
    let s = unsafe { request.scale() };
    if s.is_finite() && s > 0.0 { s.clamp(1.0, 4.0) } else { 1.0 }
}

/// The reply for `request`: the thumbnail as large as Finder asks (in pixels: its maximum size in
/// points times its scale).
fn reply(request: &QLFileThumbnailRequest) -> Result<Retained<QLThumbnailReply>> {
    // SAFETY: plain property reads.
    let (url, max) = unsafe { (request.fileURL(), request.maximumSize()) };
    let path = file_path(&url)?;
    let scale = scale_of(request);
    let picture = guarded(|| crate::thumbnail(&read_file(&path, MAX_FILE)?, max.width * scale, max.height * scale))?;
    let image = cg_image(&picture).ok_or_else(|| QuickLookError::Render("Core Graphics refused the image".into()))?;
    let size = CGSize::new(f64::from(picture.width) / scale, f64::from(picture.height) / scale);
    let draw = RcBlock::new(move |cx: NonNull<CGContext>| -> Bool {
        // SAFETY: Quick Look passes the bitmap context it draws the thumbnail from.
        let cx = unsafe { cx.as_ref() };
        // The whole context, in its own coordinates (whatever scale Quick Look set up).
        let rect = CGContext::clip_bounding_box(Some(cx));
        CGContext::set_interpolation_quality(Some(cx), CGInterpolationQuality::High);
        CGContext::draw_image(Some(cx), rect, Some(&image));
        Bool::YES
    });
    // SAFETY: the block keeps the image it draws alive for as long as Quick Look keeps the block.
    Ok(unsafe { QLThumbnailReply::replyWithContextSize_drawingBlock(size, &draw) })
}

/// `p` as an sRGB Core Graphics image (straight alpha, last).
pub(crate) fn cg_image(p: &Picture) -> Option<CFRetained<CGImage>> {
    let (w, h) = (usize::try_from(p.width).ok()?, usize::try_from(p.height).ok()?);
    if p.rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let data = CFData::from_bytes(&p.rgba);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    // SAFETY: kCGColorSpaceSRGB is a constant CFString Core Graphics exports.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    let info = CGBitmapInfo(CGImageAlphaInfo::Last.0);
    // SAFETY: the provider holds w × h pixels of 4 bytes, rows of w × 4 bytes; no decode array.
    unsafe {
        CGImage::new(w, h, 8, 32, w * 4, Some(&space), info, Some(&provider), std::ptr::null(), true, CGColorRenderingIntent::RenderingIntentDefault)
    }
}
