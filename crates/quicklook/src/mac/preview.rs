//! The preview extension's principal class: the Space bar asks it for a file's preview, and it
//! answers with a PNG of the first artboard ([`crate::preview`]) as a data-based
//! `QLPreviewReply`, or with an error (Quick Look then shows the file's icon and details).

use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{AnyThread as _, define_class};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSError, NSObjectProtocol};
use objc2_quick_look_ui::{QLFilePreviewRequest, QLPreviewProvider, QLPreviewReply, QLPreviewingController};
use objc2_uniform_type_identifiers::UTTypePNG;

use super::{file_path, guarded, ns_error};
use crate::{Limits, MAX_FILE, Result, read_file};

define_class!(
    /// `NSExtensionPrincipalClass` of `VectorCraftPreview.appex`.
    #[unsafe(super(QLPreviewProvider, NSObject))]
    #[name = "VCPreviewProvider"]
    pub struct PreviewProvider;

    unsafe impl NSObjectProtocol for PreviewProvider {}

    unsafe impl QLPreviewingController for PreviewProvider {
        #[unsafe(method(providePreviewForFileRequest:completionHandler:))]
        fn provide_preview(&self, request: &QLFilePreviewRequest, handler: &DynBlock<dyn Fn(*mut QLPreviewReply, *mut NSError)>) {
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

/// The reply for `request`: the preview PNG, shown at the artboard's size in points. It is made
/// before replying, so a file without anything to show is an error Quick Look can act on.
fn reply(request: &QLFilePreviewRequest) -> Result<Retained<QLPreviewReply>> {
    // SAFETY: a plain property read.
    let url = unsafe { request.fileURL() };
    let path = file_path(&url)?;
    // One set for the process, so renders that ran out of time are counted across requests.
    static LIMITS: OnceLock<Limits> = OnceLock::new();
    let limits = LIMITS.get_or_init(Limits::default);
    let preview = guarded(|| crate::preview(read_file(&path, MAX_FILE)?, limits))?;
    let data = NSData::with_bytes(&preview.png);
    let make = RcBlock::new(move |_reply: NonNull<QLPreviewReply>, _error: *mut *mut NSError| -> *mut NSData {
        // Returned autoreleased, as a block's object result is.
        Retained::autorelease_return(data.clone())
    });
    let size = CGSize::new(preview.points.0, preview.points.1);
    // SAFETY: UTTypePNG is a constant UTType the framework exports; the block returns valid data.
    Ok(unsafe { QLPreviewReply::initWithDataOfContentType_contentSize_dataCreationBlock(QLPreviewReply::alloc(), UTTypePNG, size, &make) })
}
