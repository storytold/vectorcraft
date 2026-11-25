use super::*;

/// A hotspot is data a typo moves off the glyph: every painted tool cursor's lies in its icon's box.
#[test]
fn every_tool_cursors_hotspot_lies_inside_its_icon_box() {
    for c in GLYPHS {
        if let cb::Cursor::Painted(PaintedCursor::Tool { hotspot, .. }) = tool_cursor(c) {
            assert!((0.0..=24.0).contains(&hotspot.x) && (0.0..=24.0).contains(&hotspot.y), "{c:?}: {hotspot:?}");
        }
    }
}

/// A `cursor-` icon name a cursor uses must be in the set the host registers.
#[test]
fn every_cursor_icon_the_app_names_is_in_its_set() {
    for c in GLYPHS {
        let cb::Cursor::Painted(PaintedCursor::Tool { icon, .. } | PaintedCursor::PointerTool { icon, .. }) = tool_cursor(c) else { continue };
        if icon.name().starts_with("cursor-") {
            assert!(CURSOR_ICONS.icons.iter().any(|(name, _)| *name == icon.name()), "{c:?} names {}", icon.name());
        }
    }
}
