use super::*;

/// Where new art goes (Illustrator's drawing modes, Shift+D cycles).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawMode {
    #[default]
    Normal,
    Behind,
    Inside,
}

/// Application preferences (Edit → Preferences). Every field is reachable through `prefs.get` /
/// `prefs.set {key, value}` (camelCase keys, validated against [`cmd::prefscmds::PREF_SPECS`]).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    // General
    /// Arrow-key nudge distance (General → Keyboard Increment), points.
    pub keyboard_increment: f64,
    pub constrain_angle: f64,
    pub corner_radius: f64,
    pub disable_auto_add_delete: bool,
    pub use_precise_cursors: bool,
    pub show_tool_tips: bool,
    pub anti_aliased_artwork: bool,
    pub select_same_tint_percent: bool,
    pub show_home_screen: bool,
    pub use_preview_bounds: bool,
    pub display_print_size: bool,
    pub double_click_to_isolate: bool,
    pub transform_pattern_tiles: bool,
    pub scale_corners: bool,
    /// Scale Strokes & Effects.
    pub scale_strokes: bool,
    pub zoom_with_mouse_wheel: bool,
    /// A horizontal drag on a numeric field or its label steps its value (#400).
    pub scrub_numeric_fields: bool,
    /// A two-finger rotation on a Mac trackpad turns the view about the pointer once it reaches 5°.
    pub trackpad_rotate_view: bool,
    /// Offset for Paste / duplicate (Illustrator pastes to the view centre; we offset by this).
    pub paste_offset: f64,
    // Selection & Anchor Display
    pub selection_tolerance: f64,
    pub object_selection_by_path_only: bool,
    pub snap_to_point_tolerance: f64,
    pub ctrl_click_selects_behind: bool,
    pub zoom_to_selection: bool,
    pub move_locked_with_artboard: bool,
    pub anchor_size: u32,
    pub handle_style: String,
    pub highlight_anchors_on_hover: bool,
    pub show_handles_multiple_anchors: bool,
    pub hide_corner_widget_above: f64,
    pub pen_rubber_band: bool,
    pub curvature_rubber_band: bool,
    // Type
    pub type_size_increment: f64,
    pub tracking_increment: f64,
    pub baseline_shift_increment: f64,
    pub show_east_asian_options: bool,
    pub show_indic_options: bool,
    pub type_selection_by_path_only: bool,
    pub font_names_in_english: bool,
    pub auto_size_area_type: bool,
    pub font_preview: bool,
    pub font_preview_size: String,
    pub recent_fonts_count: u32,
    pub missing_glyph_protection: bool,
    pub highlight_alternate_glyphs: bool,
    /// Type › Highlight Substituted Fonts: on screen, text drawn in a substitute for a missing
    /// font is highlighted, in every document (on by default).
    pub highlight_substituted_fonts: bool,
    pub placeholder_text: bool,
    // Units
    pub units_general: String,
    pub units_stroke: String,
    pub units_type: String,
    pub units_asian_type: String,
    /// Numbers Without Units Are Points: a number typed with no unit into a length field in picas
    /// is read in points (on by default; the reference app dims it unless a unit is Picas).
    pub numbers_without_units_are_points: bool,
    pub identify_objects_by: String,
    // Guides & Grid
    pub guide_color: String,
    pub guide_style: String,
    pub grid_color: String,
    pub grid_style: String,
    pub gridline_every: f64,
    pub grid_subdivisions: u32,
    pub grids_in_back: bool,
    pub show_pixel_grid: bool,
    // Smart Guides
    pub smart_guide_color: String,
    pub alignment_guides: bool,
    pub object_highlighting: bool,
    pub transform_tools_guides: bool,
    pub construction_guides: bool,
    pub construction_angles: String,
    pub anchor_path_labels: bool,
    pub measurement_labels: bool,
    pub spacing_guides: bool,
    pub distance_guides: bool,
    pub snapping_tolerance: f64,
    pub snap_tangent: bool,
    pub snap_perpendicular: bool,
    // Slices
    pub show_slice_numbers: bool,
    pub slice_line_color: String,
    // Hyphenation
    pub hyphenation_language: String,
    pub hyphenation_exceptions: String,
    /// Type › Options › Additional Fonts Folder: a folder (read with its subfolders) whose fonts
    /// are listed and used as if installed (#683); empty for none.
    pub fonts_folder: String,
    // Plug-ins & Scratch Disks
    pub plugins_folder: String,
    pub scratch_primary: String,
    pub scratch_secondary: String,
    // User Interface
    pub ui_brightness: String,
    /// User Interface › Accent Color: the colour of what is on, chosen or focused.
    pub ui_accent: String,
    /// User Interface › Color Scheme: "modern" (with the accent above) or "classic" (neutral
    /// greys and a blue accent).
    pub ui_color_scheme: String,
    pub canvas_color: String,
    pub auto_collapse_icon_panels: bool,
    /// User Interface › Show Tool Group Labels: the toolbar's group names (Select, Shapes, Draw…);
    /// off (as it ships, #1736), the tools run on from group to group with nothing between them.
    pub tool_group_labels: bool,
    pub open_documents_as_tabs: bool,
    pub large_tabs: bool,
    pub ui_scaling: f64,
    pub scale_cursor_with_ui: bool,
    /// UI language: `auto` (follow the system locale) or a language code such as `en`, `zh-hant`.
    /// The list of languages belongs to the shell (`ui-egui` i18n); an unknown code reads as `auto`.
    pub interface_language: String,
    /// Windows and Linux: use the system's title bar and window buttons instead of the app bar
    /// acting as the title bar (tiling window managers, desktops that draw their own decorations).
    /// Read when the app starts. macOS always uses the system's.
    pub system_title_bar: bool,
    // Performance
    pub gpu_performance: bool,
    pub animated_zoom: bool,
    /// Which graphics processor the desktop app asks for at startup (it takes effect after a
    /// restart): `automatic`, `lowPower` (the integrated GPU on hybrid-graphics machines) or
    /// `highPerformance` (the discrete one). The canvas is rasterized on the CPU and only
    /// composited on the GPU, so the integrated GPU is plenty; presenting from the discrete GPU
    /// through the integrated one made some hybrid laptops flicker (#306). Automatic is power
    /// saving on Windows and macOS and the system's default GPU elsewhere, the one the desktop
    /// runs on: a Wayland compositor may not show frames from another GPU (#502). Single-GPU
    /// machines are unaffected.
    pub gpu_preference: String,
    pub history_states: u32,
    pub real_time_drawing: bool,
    /// Rasterizer worker threads; -1 = automatic.
    pub render_threads: i32,
    /// Linux: `automatic` opens the window on the session's display server; `x11` opens it through
    /// XWayland on a Wayland session, where dropped files arrive (#1151); `x11WithPen` opens it
    /// through XWayland only when a pen is attached (#2028). Read when the app starts.
    pub linux_display_server: String,
    // File Handling
    pub background_save: bool,
    pub background_export: bool,
    pub autosave_recovery: bool,
    pub autosave_interval: u32,
    pub recovery_folder: String,
    pub recovery_off_for_complex: bool,
    pub recent_files_count: u32,
    pub low_res_proxy_eps: bool,
    pub anti_aliased_bitmaps: bool,
    pub update_links: String,
    // Clipboard Handling
    pub copy_as_svg: bool,
    pub copy_as_pdf: bool,
    pub copy_aicb: bool,
    pub aicb_mode: String,
    pub paste_text_formatting: String,
    // Appearance of Black
    pub black_on_screen: String,
    pub black_output: String,
    // Devices
    pub touch_workspace: bool,
    pub touch_gestures: bool,
    // Graphic Styles panel
    /// Override Character Color: a graphic style applied to type replaces its characters' fill
    /// and stroke with the style's fills and strokes.
    pub override_char_color: bool,
    // Pattern editing
    /// Object → Pattern → Tile Edge Color (`#rrggbb`): the tile edge and swatch bounds in pattern
    /// editing mode.
    pub pattern_tile_edge_color: String,
    /// Eyedropper Options (`eyedropper.setOptions`; a preference group, see
    /// [`cmd::prefscmds::PREF_GROUPS`]).
    pub eyedropper: EyedropperOptions,
    /// Pathfinder panel › Pathfinder Options (`object.pathfinder.options`; a preference group, see
    /// [`cmd::prefscmds::PREF_GROUPS`]).
    pub pathfinder: PathfinderOptions,
    /// Edit → Transparency Flattener Presets: the user's presets (the built-in ones aren't stored).
    /// Not a `prefs.set` key or group (resetting the preferences keeps them): `flattener.presets.*`
    /// edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flattener_presets: Vec<cmd::FlattenerPreset>,
    /// Width profiles saved to the Stroke panel's Profile list (`stroke.widthProfile.*`). Not a
    /// Preferences dialog field, so it has no [`cmd::prefscmds::PREF_SPECS`] row.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub width_profiles: Vec<vectorcraft_doc::SavedProfile>,
    /// General → Use Japanese Crop Marks: the style of Create Trim Marks and Effect → Crop Marks.
    pub japanese_crop_marks: bool,
    /// Appearance panel → New Art Has Basic Appearance (on): new art takes one fill and stroke;
    /// off, the whole appearance of the last selection (`appearance.setNewArtBasic`).
    pub new_art_basic: bool,
    /// The Color Themes panel's saved themes (`colorTheme.*`): a local library, not a Preferences
    /// dialog field, so it has no [`cmd::prefscmds::PREF_SPECS`] row and resetting keeps it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub color_themes: Vec<cmd::colortheme::ColorTheme>,
    /// New Document → Saved: the user's document presets (`file.newPresets.save`). A local
    /// library, not a Preferences dialog field: resetting the preferences keeps it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_doc_presets: Vec<cmd::newdoc::DocSettings>,
    /// New Document → Recent: the settings of the last documents made (newest first).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_new_docs: Vec<cmd::newdoc::DocSettings>,
    /// Edit → PDF Presets: the user's presets (the built-in ones aren't stored). A local library,
    /// not a Preferences dialog field: resetting the preferences keeps it; `pdf.preset.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pdf_presets: Vec<vectorcraft_pdf::PdfPreset>,
    /// File → Export for Screens: the user's batch export presets (`document.exportPresets.*`;
    /// the built-in ones aren't stored). A local library, not a Preferences dialog field.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub screen_export_presets: Vec<cmd::fileio::ScreenExportPreset>,
    // File Handling (continued)
    /// Where Save as Template and New from Template start ("" = `Documents/VectorCraft Templates`).
    pub templates_folder: String,
    /// Open older native files as "<name> [Converted]" so Save asks for a new name.
    pub append_converted: bool,
    /// File Handling → Use Compression: native saves are gzip-compressed (`document.save
    /// {compress}` overrides it).
    pub use_compression: bool,
    /// Save for Web: the user's presets (the built-in ones aren't stored). A local library, not a
    /// Preferences dialog field: resetting the preferences keeps it; `webExport.presets.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub web_export_presets: Vec<cmd::webexport::WebPreset>,
    /// Save for Web: the settings the dialog opens on (`webExport.settings`); none until set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_export_settings: Option<cmd::webexport::WebSettings>,
    /// Edit → Print Presets: the user's presets ([Default] isn't stored). A local library, not a
    /// Preferences dialog field: resetting the preferences keeps it; `print.presets.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub print_presets: Vec<cmd::printpresets::PrintPreset>,
    /// Constrain Width and Height Proportions: the link between W and H in the Transform panel,
    /// the Properties panel and the Control bar (their size fields pass `proportional` to
    /// `object.setBounds`).
    pub constrain_proportions: bool,
    /// One Corner Radius field sets every corner of a live rectangle (off: a field per corner).
    pub single_corner_radius: bool,
    /// The Transform panel menu's Transform Object Only / Transform Pattern Only / Transform Both
    /// (`object`, `pattern`, `both`): what the panel's fields, flips and menu flips transform
    /// (they pass `objects` and `patterns` to the transform commands). `default` (and any other
    /// value), until one is chosen: they pass neither, so patterns follow General › Transform
    /// Pattern Tiles (`transformPatternTiles`), as before the menu acted.
    pub transform_panel_patterns: String,
    /// The Transform panel menu's Use Registration Point for Symbol: with one symbol instance
    /// selected, the panel and the Properties panel's Transform section measure and transform it
    /// from its symbol's registration point instead of the reference point.
    pub use_registration_point_for_symbol: bool,
    /// The tools' persistent options by store (a tool id, or a store a family shares: `liquify`
    /// holds the Liquify tools' Global Brush Dimensions), see [`vectorcraft_tools::settings`]:
    /// kept across tool switches and saved with the preferences. Not a Preferences dialog field:
    /// resetting the preferences keeps them; `tool.setOption` edits them.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tool_settings: std::collections::BTreeMap<String, serde_json::Map<String, Value>>,
    /// View → Perspective Grid presets: the user's (the built-in ones aren't stored). A local
    /// library, not a Preferences dialog field: resetting the preferences keeps it;
    /// `perspective.presets.*` edit it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub perspective_presets: Vec<vectorcraft_tools::distort::perspective::GridDefinition>,
    /// Blend Options set with no blend selected: what new blends start with
    /// (`object.blend.options`; none: Smooth Color, Align to Page). A tool setting, not a
    /// Preferences dialog field: it has no [`cmd::prefscmds::PREF_SPECS`] row and resetting the
    /// preferences keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend_options: Option<vectorcraft_doc::live::BlendDefaults>,
    /// Perspective Grid Options (double-click the Perspective Grid tool): whether the Plane
    /// Switching Widget shows and where (`perspective.widget.options`).
    pub perspective_widget: vectorcraft_tools::distort::perspective::widget::WidgetOptions,
    /// What names and codes in other apps' shortcut sets stand for, as the user confirmed them on
    /// import (`importMatches.*`). A local library, not a Preferences dialog field: resetting the
    /// preferences keeps it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub import_matches: Vec<cmd::importmatches::ImportMatch>,
}

impl Default for Prefs {
    fn default() -> Self {
        let s = |v: &str| v.to_string();
        Self {
            keyboard_increment: 1.0,
            constrain_angle: 0.0,
            corner_radius: 12.0,
            disable_auto_add_delete: false,
            use_precise_cursors: false,
            show_tool_tips: true,
            anti_aliased_artwork: true,
            select_same_tint_percent: false,
            show_home_screen: true,
            use_preview_bounds: false,
            display_print_size: false,
            double_click_to_isolate: true,
            transform_pattern_tiles: false,
            scale_corners: false,
            scale_strokes: false,
            zoom_with_mouse_wheel: false,
            scrub_numeric_fields: true,
            trackpad_rotate_view: true,
            paste_offset: 10.0,
            selection_tolerance: 3.0,
            object_selection_by_path_only: false,
            snap_to_point_tolerance: 2.0,
            ctrl_click_selects_behind: true,
            zoom_to_selection: true,
            move_locked_with_artboard: false,
            anchor_size: 3,
            handle_style: s("solid"),
            highlight_anchors_on_hover: true,
            show_handles_multiple_anchors: true,
            hide_corner_widget_above: 177.0,
            pen_rubber_band: true,
            curvature_rubber_band: true,
            type_size_increment: 2.0,
            tracking_increment: 20.0,
            baseline_shift_increment: 2.0,
            show_east_asian_options: false,
            show_indic_options: false,
            type_selection_by_path_only: false,
            font_names_in_english: true,
            auto_size_area_type: false,
            font_preview: true,
            font_preview_size: s("medium"),
            recent_fonts_count: 10,
            missing_glyph_protection: true,
            highlight_alternate_glyphs: false,
            highlight_substituted_fonts: true,
            placeholder_text: true,
            units_general: s("points"),
            units_stroke: s("points"),
            units_type: s("points"),
            units_asian_type: s("points"),
            numbers_without_units_are_points: true,
            identify_objects_by: s("objectName"),
            guide_color: s("#4affff"),
            guide_style: s("lines"),
            grid_color: s("#c8c8c8"),
            grid_style: s("lines"),
            gridline_every: 72.0,
            grid_subdivisions: 8,
            grids_in_back: true,
            show_pixel_grid: true,
            smart_guide_color: s("#ff3dfc"),
            alignment_guides: true,
            object_highlighting: true,
            transform_tools_guides: true,
            construction_guides: true,
            construction_angles: s(vectorcraft_tools::guides::DEFAULT_CONSTRUCTION_ANGLES),
            anchor_path_labels: true,
            measurement_labels: true,
            spacing_guides: true,
            distance_guides: true,
            snapping_tolerance: 4.0,
            snap_tangent: true,
            snap_perpendicular: true,
            show_slice_numbers: true,
            slice_line_color: s("#ff3f3f"),
            hyphenation_language: s("English: USA"),
            hyphenation_exceptions: String::new(),
            fonts_folder: String::new(),
            plugins_folder: String::new(),
            scratch_primary: s("Startup"),
            scratch_secondary: s("None"),
            ui_brightness: s("mediumDark"),
            ui_accent: s("coral"),
            ui_color_scheme: s("modern"),
            canvas_color: s("matchUi"),
            auto_collapse_icon_panels: false,
            tool_group_labels: false,
            open_documents_as_tabs: true,
            large_tabs: false,
            ui_scaling: 1.0,
            scale_cursor_with_ui: false,
            interface_language: s("auto"),
            system_title_bar: false,
            gpu_performance: true,
            animated_zoom: true,
            gpu_preference: s("automatic"),
            history_states: 500,
            real_time_drawing: true,
            render_threads: -1,
            linux_display_server: s("automatic"),
            background_save: true,
            background_export: true,
            autosave_recovery: true,
            autosave_interval: 2,
            recovery_folder: String::new(),
            recovery_off_for_complex: false,
            recent_files_count: 20,
            low_res_proxy_eps: false,
            anti_aliased_bitmaps: false,
            update_links: s("askWhenModified"),
            copy_as_svg: true,
            copy_as_pdf: false,
            copy_aicb: false,
            aicb_mode: s("preserveAppearance"),
            paste_text_formatting: s("keep"),
            black_on_screen: s("accurate"),
            black_output: s("accurate"),
            touch_workspace: true,
            touch_gestures: true,
            override_char_color: true,
            pattern_tile_edge_color: {
                let [r, g, b] = vectorcraft_doc::LAYER_COLORS[0].1;
                Color::rgb8(r, g, b).to_hex()
            },
            eyedropper: Default::default(),
            pathfinder: Default::default(),
            flattener_presets: vec![],
            width_profiles: vec![],
            japanese_crop_marks: false,
            new_art_basic: true,
            color_themes: vec![],
            new_doc_presets: vec![],
            recent_new_docs: vec![],
            pdf_presets: vec![],
            screen_export_presets: vec![],
            templates_folder: String::new(),
            append_converted: true,
            use_compression: false,
            web_export_presets: vec![],
            web_export_settings: None,
            print_presets: vec![],
            constrain_proportions: false,
            single_corner_radius: true,
            transform_panel_patterns: "default".to_string(),
            use_registration_point_for_symbol: false,
            tool_settings: Default::default(),
            perspective_presets: vec![],
            blend_options: None,
            perspective_widget: Default::default(),
            import_matches: vec![],
        }
    }
}
