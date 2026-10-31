use super::{VectorcraftApp, io};

/// The active document's perspective grid (View → Perspective Grid toggles).
pub(super) fn perspective_grid(app: &VectorcraftApp) -> Option<vectorcraft_tools::distort::perspective::PerspectiveGrid> {
    app.session.active().map(|d| vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&d.doc))
}

/// File → Open Recent Files slots (Preferences → File Handling shows 0–30 of them).
pub(super) const RECENT_IDS: [&str; io::MAX_RECENT_FILES] = [
    "file.openRecent1",
    "file.openRecent2",
    "file.openRecent3",
    "file.openRecent4",
    "file.openRecent5",
    "file.openRecent6",
    "file.openRecent7",
    "file.openRecent8",
    "file.openRecent9",
    "file.openRecent10",
    "file.openRecent11",
    "file.openRecent12",
    "file.openRecent13",
    "file.openRecent14",
    "file.openRecent15",
    "file.openRecent16",
    "file.openRecent17",
    "file.openRecent18",
    "file.openRecent19",
    "file.openRecent20",
    "file.openRecent21",
    "file.openRecent22",
    "file.openRecent23",
    "file.openRecent24",
    "file.openRecent25",
    "file.openRecent26",
    "file.openRecent27",
    "file.openRecent28",
    "file.openRecent29",
    "file.openRecent30",
];

/// The saved selection a `select.recallN` slot names (none past the document's last).
pub(super) fn saved_selection_name(app: &VectorcraftApp, id: &str) -> Option<String> {
    let n = saved_selection_slot(id)?.checked_sub(1)?;
    app.session.active()?.doc.saved_selections.get(n).map(|x| x.name.clone())
}

/// The recent file a `file.openRecentN` slot names (none past the preference's count).
pub(super) fn recent_slot<'a>(app: &'a VectorcraftApp, id: &str) -> Option<&'a String> {
    let n: usize = id.strip_prefix("file.openRecent")?.parse().ok()?;
    io::recent_files(app).get(n.checked_sub(1)?)
}

/// View → Perspective Grid → One/Two/Three Point Perspective: the saved-preset slots of each type.
pub(super) const PERSPECTIVE_SLOTS: [[&str; crate::dialogs::perspective_presets::SLOTS]; 3] = [
    [
        "ui.perspectiveUserPreset1.1",
        "ui.perspectiveUserPreset1.2",
        "ui.perspectiveUserPreset1.3",
        "ui.perspectiveUserPreset1.4",
        "ui.perspectiveUserPreset1.5",
    ],
    [
        "ui.perspectiveUserPreset2.1",
        "ui.perspectiveUserPreset2.2",
        "ui.perspectiveUserPreset2.3",
        "ui.perspectiveUserPreset2.4",
        "ui.perspectiveUserPreset2.5",
    ],
    [
        "ui.perspectiveUserPreset3.1",
        "ui.perspectiveUserPreset3.2",
        "ui.perspectiveUserPreset3.3",
        "ui.perspectiveUserPreset3.4",
        "ui.perspectiveUserPreset3.5",
    ],
];

/// Select → saved selections: the n-th saved selection of the active document.
pub(super) const SAVED_SELECTION_IDS: [&str; vectorcraft_engine::doc::SavedSelection::MAX] = [
    "select.recall1",
    "select.recall2",
    "select.recall3",
    "select.recall4",
    "select.recall5",
    "select.recall6",
    "select.recall7",
    "select.recall8",
    "select.recall9",
    "select.recall10",
    "select.recall11",
    "select.recall12",
    "select.recall13",
    "select.recall14",
    "select.recall15",
    "select.recall16",
    "select.recall17",
    "select.recall18",
    "select.recall19",
    "select.recall20",
    "select.recall21",
    "select.recall22",
    "select.recall23",
    "select.recall24",
    "select.recall25",
];

/// View → saved views: the n-th saved view of the active document.
pub(super) const SAVED_VIEW_IDS: [&str; vectorcraft_engine::doc::SavedView::MAX] = [
    "view.goto1",
    "view.goto2",
    "view.goto3",
    "view.goto4",
    "view.goto5",
    "view.goto6",
    "view.goto7",
    "view.goto8",
    "view.goto9",
    "view.goto10",
    "view.goto11",
    "view.goto12",
    "view.goto13",
    "view.goto14",
    "view.goto15",
    "view.goto16",
    "view.goto17",
    "view.goto18",
    "view.goto19",
    "view.goto20",
    "view.goto21",
    "view.goto22",
    "view.goto23",
    "view.goto24",
    "view.goto25",
];

/// The 1-based slot a `view.gotoN` id stands for.
pub(super) fn saved_view_slot(id: &str) -> Option<usize> {
    id.strip_prefix("view.goto")?.parse().ok()
}

/// The saved view a `view.gotoN` slot names (none past the document's last).
pub(super) fn saved_view<'a>(app: &'a VectorcraftApp, id: &str) -> Option<&'a vectorcraft_engine::doc::SavedView> {
    app.session.active()?.doc.views.get(saved_view_slot(id)?.checked_sub(1)?)
}

/// The 1-based slot a `select.recallN` id stands for.
pub(super) fn saved_selection_slot(id: &str) -> Option<usize> {
    id.strip_prefix("select.recall")?.parse().ok()
}

/// Type → Recent Fonts slots (Preferences › Type › Number of Recent Fonts shows up to 15).
pub(super) const RECENT_FONT_IDS: [&str; crate::MAX_RECENT_FONTS] = [
    "type.recentFont1",
    "type.recentFont2",
    "type.recentFont3",
    "type.recentFont4",
    "type.recentFont5",
    "type.recentFont6",
    "type.recentFont7",
    "type.recentFont8",
    "type.recentFont9",
    "type.recentFont10",
    "type.recentFont11",
    "type.recentFont12",
    "type.recentFont13",
    "type.recentFont14",
    "type.recentFont15",
];
