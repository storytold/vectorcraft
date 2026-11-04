//! Area type's vertical alignment in its frame (#3759): the Align Top, Align Center, Align Bottom
//! and Justify Vertically buttons of the Properties panel's Paragraph section and of the Control
//! panel. Per the public documentation they align area type with one row and one column that isn't
//! threaded ([Align text](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/align-text.html)),
//! so they are turned off when a selected frame has rows or columns or is threaded.

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{NodeKind, TextKind, VerticalAlign};

use crate::VectorcraftApp;
use crate::widgets;

/// The vertical alignment the selected area type shows, and whether the buttons can change it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shown {
    /// The alignment every selected frame shares; `None` when they differ.
    pub(crate) shared: Option<VerticalAlign>,
    /// Every selected frame has one row and one column and isn't threaded.
    pub(crate) enabled: bool,
}

/// The vertical alignment of the selected area type (and area type in selected groups, the frames
/// `text.areaOptions` changes); `None` when no area type is selected.
pub(crate) fn shown(app: &VectorcraftApp) -> Option<Shown> {
    let st = app.session.active()?;
    let threaded = |id| st.doc.text_threads.iter().any(|t| t.len() > 1 && t.contains(&id));
    let mut out: Option<Shown> = None;
    for n in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        n.walk(&mut |c| {
            let NodeKind::Text(t) = &c.kind else { return };
            if !matches!(t.kind, TextKind::Area { .. }) {
                return;
            }
            let a = t.area.vertical_align;
            let plain = t.area.rows <= 1 && t.area.columns <= 1 && !threaded(c.id);
            out = Some(match out {
                None => Shown { shared: Some(a), enabled: plain },
                Some(s) => Shown { shared: s.shared.filter(|b| *b == a), enabled: s.enabled && plain },
            });
        });
    }
    out
}

/// The four buttons for the selected area type: the shared alignment shows selected; a click sets
/// it on every selected frame in one undo step (`text.areaOptions {verticalAlign}`, as Area Type
/// Options does, each frame keeping its other options). Turned off for frames with rows or columns
/// and threaded ones; nothing shows without area type. Returns whether the buttons were drawn.
pub(crate) fn buttons(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) -> bool {
    let Some(s) = shown(app) else { return false };
    for (a, icon, tip, id) in [
        (VerticalAlign::Top, "dc-al-top", tl!("Align Top"), "top"),
        (VerticalAlign::Center, "dc-al-vcenter", tl!("Align Center"), "center"),
        (VerticalAlign::Bottom, "dc-al-bottom", tl!("Align Bottom"), "bottom"),
        (VerticalAlign::Justify, "dc-dist-vcenter", tl!("Justify Vertically"), "justify"),
    ] {
        if widgets::icon_button_enabled(ui, icon, tip, s.enabled && s.shared == Some(a), s.enabled, size).clicked()
            && let Err(e) = app.run("text.areaOptions", json!({ "verticalAlign": id }))
        {
            app.status(e);
        }
    }
    true
}
