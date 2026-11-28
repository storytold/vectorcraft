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

/// A click lands where the glyph points: in every painted tool cursor's image, the icon's own
/// mark color (not only its halo) covers the hotspot pixel or one next to it.
#[test]
fn every_tool_cursors_hotspot_is_on_its_glyph() {
    use craft::ui::primitives::{painted_image, register_app_icons};
    use craft::ui::runtime::env::Env;
    use craft::ui::style::rgba_color;
    register_app_icons(&crate::icons::APP_ICONS);
    register_app_icons(&CURSOR_ICONS);
    let mut missed = Vec::new();
    crate::testing::ui_context()
        .run_ui(Default::default(), |ui| {
            let env = Env::of(ui);
            let ink = rgba_color(env.tokens.tokens().color.overlay.cursor);
            let near_ink = |px: &[u8]| px[3] == 255 && [ink.r(), ink.g(), ink.b()].iter().zip(px).all(|(want, got)| want.abs_diff(*got) <= 8);
            for c in GLYPHS {
                let cb::Cursor::Painted(painted @ PaintedCursor::Tool { .. }) = tool_cursor(c) else { continue };
                let Some(image) = painted_image(&env, &painted, 2.0) else {
                    missed.push(format!("{c:?}: no image"));
                    continue;
                };
                let ([w, h], [hx, hy]) = (image.size.map(usize::from), image.hotspot.map(usize::from));
                let on_glyph = (hy.saturating_sub(1)..=(hy + 1).min(h - 1))
                    .flat_map(|y| (hx.saturating_sub(1)..=(hx + 1).min(w - 1)).map(move |x| (y * w + x) * 4))
                    .any(|i| near_ink(&image.rgba[i..i + 4]));
                if !on_glyph {
                    missed.push(format!("{c:?}"));
                }
            }
        })
        .textures_delta
        .clear();
    assert!(missed.is_empty(), "hotspots off their glyphs: {missed:?}");
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
