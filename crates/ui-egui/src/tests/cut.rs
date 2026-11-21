//! The cutting tools in the UI: toolbar entries and icons.

// PLAN: assumed `IconId` is reached as `craft::behavior::IconId`
use craft::behavior::IconId;

use crate::icons;

#[test]
fn the_cut_tools_have_their_own_icons_and_toolbar_slot() {
    for (tool, icon) in [("mirrorCut", "dc-mirror-cut"), ("lineCut", "dc-line-cut"), ("rectCut", "dc-rect-cut")] {
        let info = vectorcraft_tools::tool_info(tool).unwrap();
        assert_eq!(icons::tool_icon(info.icon), icon);
        assert!(craft::ui::primitives::has_icon(&IconId::new(icon)), "{icon}");
        assert!(crate::toolbar::BASIC.iter().flat_map(|(_, slots)| slots.iter()).any(|s| s.contains(&tool) && s.contains(&"knife")));
    }
}
