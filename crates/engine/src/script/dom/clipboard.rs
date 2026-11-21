//! `app.copy()`, `app.cut()` and `app.paste()`, as the help page
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Application/> describes them: Edit › Copy, Cut
//! and Paste on the active document, run as `edit.copy`, `edit.cut` and `edit.paste` in the run's
//! transaction.
//!
//! VectorCraft's own rules, which the help page does not state: with nothing selected, or no
//! document open, `copy` and `cut` do nothing; with the clipboard empty `paste` does nothing; and
//! `paste` with no document open throws `No active document`.

use serde_json::json;
use vectorcraft_js::{Data, HostError, HostMember};

use super::{Answer, Host, Link, NO_ACTIVE_DOCUMENT, command, method, named};

/// The clipboard functions of `Application`.
pub(super) fn app_members(link: &Link) -> Vec<(String, HostMember)> {
    named(vec![("copy", method(link, copy)), ("cut", method(link, cut)), ("paste", method(link, paste))])
}

/// Runs `id` when the active document has something it acts on.
fn on_selection(h: &Host, id: &str) -> Answer {
    let ready = crate::cmd::has_selection_guides_or_artboard_tool(&h.session.borrow()).is_ok();
    if ready {
        command(h, id, &json!({}))?;
    }
    Ok(Data::Undefined)
}

fn copy(h: &Host, _: &[Data]) -> Answer {
    on_selection(h, "edit.copy")
}

fn cut(h: &Host, _: &[Data]) -> Answer {
    on_selection(h, "edit.cut")
}

fn paste(h: &Host, _: &[Data]) -> Answer {
    let ready = {
        let s = h.session.borrow();
        if s.active().is_none() {
            return Err(HostError::Error(NO_ACTIVE_DOCUMENT.into()));
        }
        !s.clipboard.is_empty()
    };
    if ready {
        command(h, "edit.paste", &json!({}))?;
    }
    Ok(Data::Undefined)
}
