use craft::behavior::dock::Action;
use craft::ui::canvas::{ViewCommands, ViewRequest};
use craft::ui::runtime::frame::{ChromeToggle, request_chrome_toggle};

use super::invoke::open_menu_dialog;
use super::model::home_key;
use super::slots::{recent_readable, recent_slot, saved_selection_name, saved_selection_slot, saved_view, saved_view_slot};
use super::state::{checked, selected_image};
use super::ui_commands::UI_COMMANDS;
use super::{Brightness, Face, SaveMode, Value, VectorcraftApp, io, json, next_zoom};
use crate::UiState;
use crate::i18n::LangExt as _;

/// Canonical panel id for `window.panel`: a dock tab's or an icon panel's id or display label,
/// matched case-insensitively (`"Layers"`, `"swatches"`, `"Color Guide"`), or a library panel's id
/// as it is (`library:swatches:pastels`).
pub(crate) fn normalize_panel(input: &str) -> Option<&str> {
    let name = input.trim();
    crate::state::all_panels()
        .find(|(id, label)| name.eq_ignore_ascii_case(id) || name.eq_ignore_ascii_case(label))
        .map(|(id, _)| id)
        .or_else(|| crate::panels::library_panel::parse(input).map(|_| input))
}

/// The dock panel of the `toolbar` param: a toolbar made with New Toolbar, or the Tools panel.
pub(super) fn toolbar_panel(ui: &UiState, p: &Value) -> Result<String, String> {
    let name = crate::custom_toolbars::target(ui, p)?;
    Ok(name
        .and_then(|n| crate::custom_toolbars::find(ui, &n))
        .map_or_else(|| crate::toolbar::TOOLS.to_owned(), |b| crate::custom_toolbars::panel_id(b.id)))
}

/// Whether the toolbar in dock panel `panel` lays its tools out in two columns.
pub(super) fn toolbar_double(ui: &UiState, panel: &str) -> bool {
    crate::panel_docking::column_of(crate::panel_docking::layout_of(ui), panel).is_some_and(|(_, column)| column.steps == Some(2))
}

/// An optional `{key?: bool}` param: none when it is omitted or null (the commands then toggle).
pub(crate) fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// An optional `{key?: number}` param: none when it is omitted or null.
pub(crate) fn opt_number(p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("`{key}` must be a number")),
    }
}

/// An optional `{key?: index}` param, a whole number of 0 or more: none when it is omitted or null.
pub(crate) fn opt_index(p: &Value, key: &str) -> Result<Option<usize>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).map(Some).ok_or_else(|| format!("`{key}` must be a whole number of 0 or more")),
    }
}

/// An optional `{key?: [x, y]}` param: none when it is omitted or null.
fn opt_point(p: &Value, key: &str) -> Result<Option<vectorcraft_geom::Point>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_array().map(Vec::as_slice) {
            Some([x, y]) => x.as_f64().zip(y.as_f64()).map(|(x, y)| Some(vectorcraft_geom::Point::new(x, y))),
            _ => None,
        }
        .ok_or_else(|| format!("`{key}` must be [x, y] numbers")),
    }
}

/// Handle a UI command. `None` = not a UI command (the engine handles it).
/// `params` has only keys the params description of engine command `id` names, or the parameter
/// error naming the one it doesn't (#1442). The UI's own commands aren't checked yet: their
/// descriptions haven't been audited against what they read.
/// `file.save` and `file.exportForScreens` take the params of the engine command they run.
pub fn check_params(session: &vectorcraft_engine::Session, id: &str, params: &Value) -> Result<(), String> {
    let id = match id {
        "file.save" => "document.save",
        "file.exportForScreens" => "document.exportForScreens",
        id => id,
    };
    if UI_COMMANDS.iter().any(|c| c.0 == id) {
        return Ok(());
    }
    session.check_params(id, params).map_err(|e| e.to_string())
}

/// Runs `id` when the UI runs it itself: one of [`UI_COMMANDS`], or an engine command the UI
/// handles its own way (a Window menu panel item that toggles, …) → `None` to leave it to the
/// engine.
pub fn run_ui_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    // Window menu items and the named panel function keys toggle against the checked state;
    // other commands that open panels show them.
    let toggles = if id == "window.panel" { opt_bool(p, "toggle").and_then(|t| Ok((t, opt_bool(p, "toggleGroup")?))) } else { Ok((None, None)) };
    let (toggle, toggle_group) = match toggles {
        Ok(flags) => flags,
        Err(e) => return Some(Err(e)),
    };
    if toggle == Some(true) && checked(app, id, p) == Some(true) {
        let panel = p.get("panel").and_then(Value::as_str).and_then(normalize_panel).unwrap_or_default();
        if app.ui.open_panel.as_deref() == Some(panel) {
            app.ui.open_panel = None;
            return Some(Ok(json!({"open": null})));
        }
        if toggle_group == Some(true) {
            return Some(crate::panel_docking::hide_group(app, panel));
        }
        return Some(app.run("window.panel.close", json!({"panel": panel})));
    }
    if let Some(result) = crate::panel_docking::command(app, id, p) {
        return Some(result);
    }
    if id == "app.language" {
        let lang = p.get("lang").and_then(Value::as_str).unwrap_or("");
        let value = if lang.eq_ignore_ascii_case("auto") {
            "auto".to_string()
        } else if let Some(l) = crate::i18n::Lang::from_code(lang) {
            l.code().to_string()
        } else {
            let codes: Vec<&str> = std::iter::once("auto").chain(crate::i18n::Lang::all().map(|l| l.code())).collect();
            return Some(Err(format!("lang must be one of {}", codes.join(", "))));
        };
        return Some(app.run("prefs.set", json!({"key": "interfaceLanguage", "value": value})).map(|_| json!(value)));
    }
    if let Some(r) = crate::panels::character::intercept_text_command(app, id) {
        return Some(r);
    }
    if let Some(r) = crate::shortcut_editor::run_command(app, id, p)
        .or_else(|| crate::workspaces::run_command(app, id, p))
        .or_else(|| crate::edit_toolbar::command(app, id, p))
        .or_else(|| crate::custom_toolbars::command(app, id, p))
        .or_else(|| crate::panels::learn::command(app, id, p))
        .or_else(|| crate::panels::discover::command(app, id, p))
        .or_else(|| crate::panels::run_menu_command(app, id, p))
    {
        return Some(r);
    }
    run_ui_part1(app, id, p)
        .or_else(|| run_ui_part2(app, id, p))
        .or_else(|| run_ui_part3(app, id, p))
        .or_else(|| run_ui_part4(app, id, p))
        .or_else(|| run_ui_part5(app, id, p))
        .or_else(|| run_ui_part6(app, id, p))
        .or_else(|| run_ui_part7(app, id, p))
        .or_else(|| run_ui_part8(app, id, p))
        .or_else(|| run_ui_part9(app, id, p))
        .or_else(|| run_ui_part10(app, id, p))
        .or_else(|| run_ui_part11(app, id, p))
        .or_else(|| run_ui_part12(app, id, p))
        .or_else(|| run_ui_part13(app, id, p))
}

/// Why Show/Hide Guides and Show/Hide Grid refuse to run while Trim View is on.
pub(crate) const TRIM_VIEW_HIDES: &str = "Trim View hides the guides and the grid; turn it off first";

/// Drops the selection when it holds ruler guides or guide paths that no longer draw (Hide Guides,
/// Trim View): their selection overlay would otherwise reveal the geometry just hidden.
fn deselect_hidden_guides(app: &mut VectorcraftApp) {
    let hidden_selection = !app.ui.view.guides_shown()
        && app.session.active().is_some_and(|d| {
            !d.selection.guides.is_empty()
                || d.selection
                    .objects
                    .iter()
                    .any(|id| matches!(d.doc.node(*id).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Path { guide: true, .. })))
        });
    if hidden_selection {
        // Deselecting a document that is open cannot fail.
        let _ = app.run("select.none", json!({}));
    }
}

/// UI commands, part 1.
fn run_ui_part1(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "file.newDialog" => {
            crate::dialogs::open_new_document(app);
            Ok(Value::Null)
        }
        "file.newWithoutDialog" => crate::dialogs::new_document_without_dialog(app),
        "app.home" => {
            app.ui.home = Some(home_key(app));
            Ok(Value::Null)
        }
        "file.open" => match s("path") {
            Some(path) => match p.get("params").filter(|v| !v.is_null()) {
                Some(params) => io::open_path_with(app, &path, params),
                None => io::open_path(app, &path),
            },
            None => io::open_dialog(app),
        },
        "file.openWith" => io::open_with(app, p),
        // Saves write through the app (save panel, download); with no path known the panel and the
        // format's options dialog ask first.
        // A confined request sees the document's own file, and the folder a save suggests, only
        // inside its read roots.
        "file.save" | "file.saveAs" | "file.saveCopy" | "file.saveAsTemplate" => {
            let mode = match id {
                "file.save" => SaveMode::Save,
                "file.saveAs" => SaveMode::SaveAs,
                "file.saveCopy" => SaveMode::Copy,
                _ => SaveMode::Template,
            };
            let saved = if mode == SaveMode::Save { io::save_current(app, p) } else { io::save(app, mode, p, true) };
            saved.map(|mut r| {
                vectorcraft_engine::cmd::fileio::leave_out_unreadable_save_paths(&mut r, p.get("path").and_then(Value::as_str));
                r
            })
        }
        // Without params the save panel picks the file, then its format's options dialog follows.
        "document.exportSelection" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::export_selection(app),
        "document.exportSelection" => {
            io::save_command_output(app, id, "png", if p.is_object() { p.clone() } else { json!({}) }, |_, path| Ok(json!({ "path": path })))
        }
        // Bytes sent by an agent go straight to the engine; a path or nothing opens it here.
        "file.newFromTemplate" if p.get("dataBase64").is_none() => io::new_from_template(app, s("path")),
        "file.revert" if opt_bool(p, "confirmed") != Ok(Some(true)) => opt_bool(p, "confirmed").and_then(|_| io::ask_revert(app)),
        "file.reveal" => io::reveal(app),
        "file.cancelExport" => Ok(io::cancel_exports(app)),
        "file.exportInBackground" => Ok(io::hide_exports(app)),
        "file.cancelSave" => Ok(io::cancel_saves(app)),
        "file.saveInBackground" => Ok(io::hide_saves(app)),
        "file.cancelOpen" => Ok(crate::opening::cancel_all(app)),
        "file.openInBackground" => Ok(crate::opening::hide_all(app)),
        "file.cancelPlace" => Ok(crate::placing::cancel_all(app)),
        id if id.starts_with("file.openRecent") => match recent_slot(app, id).filter(|path| recent_readable(path)).cloned() {
            Some(path) => io::open_reporting(app, &path),
            None => Err("no such recent file".into()),
        },
        "type.findFont" => {
            crate::find_font::open(app);
            Ok(Value::Null)
        }
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 2.
fn run_ui_part2(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let flag = |b: &mut bool| {
        *b = !*b;
        Ok(json!(*b))
    };
    let r = match id {
        "ui.missingFontsDialog" => crate::dialogs::missing_fonts::open_command(app, s("folder")),
        "ui.findFontsInFolder" => crate::dialogs::missing_fonts::find_in_folder_command(app, s("folder")),
        "file.clearRecent" => {
            app.ui.recent_files.clear();
            Ok(Value::Null)
        }
        "file.recentFiles" => Ok(json!(io::recent_files(app).iter().filter(|path| recent_readable(path)).collect::<Vec<_>>())),
        "file.place" => crate::place::run(app, p),
        "file.place.queue" => crate::place::queue(app, p),
        "file.export.svg" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::svg_options::open(app, crate::dialogs::svg_options::Mode::Export, None);
            Ok(Value::Null)
        }
        "file.export.svg" => io::export(app, Some("svg"), s("path"), p),
        "file.export.svg.browser" => crate::dialogs::svg_options::browser_preview(app, p),
        "file.exportForScreens" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::open_export_for_screens(app);
            Ok(Value::Null)
        }
        "file.exportForScreens" => io::export_for_screens(app, p.clone()),
        "file.export.png" if s("path").is_none() => {
            let mut params = if p.is_object() { p.clone() } else { json!({}) };
            io::target_path(app, None, "png", move |app, path| {
                let f = vectorcraft_engine::cmd::fileio::format("png").ok_or("no PNG encoder")?;
                params["format"] = json!("png");
                params["path"] = json!(path);
                // No artboard named: the artboard in use rather than the first (#5502), or each one the
                // Artboard tool has selected, one file each.
                let named =
                    ["useArtboards", "rect"].iter().chain(vectorcraft_engine::cmd::fileio::ARTBOARD_PARAMS.iter()).any(|k| !params[*k].is_null());
                let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
                if !named && n > 0 {
                    match crate::panels::artboards::picked(app, n).as_slice() {
                        [i] => params["artboard"] = json!(i),
                        several => {
                            params["artboards"] = json!(several);
                            params["useArtboards"] = json!(true);
                        }
                    }
                }
                crate::dialogs::open_raster_options(app, f, params);
                Ok(Value::Null)
            })
        }
        "file.export.png" => io::export(app, Some("png"), s("path"), p),
        "file.documentSetup" => crate::dialogs::open_document_setup(app),
        "edit.preferences" => {
            crate::prefs_dialog::open(app, s("category").as_deref());
            Ok(Value::Null)
        }
        // Edit → Color Settings… / Assign Profile… (no params): show the colour-management panel
        // wherever it is, as Window › Separations Preview does (it isn't in an icon column, so a
        // flyout for it never opened: #4542), scrolled to its Color Settings section, which a short
        // window shows only after scrolling past the plates and Proof Setup (#6419).
        "edit.colorSettings" | "edit.assignProfile" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::panel_docking::reveal_panel(app, "separations").and_then(|_| {
                app.ui.color_settings_scroll = true;
                app.session.execute(id, p).map_err(|e| e.to_string())
            })
        }
        // Edit › Refresh Monitor Profile: the desktop app's next frame reads the display's profile.
        "view.refreshMonitorProfile" => app.display_profile.request_read(app.session.prefs.monitor_profile_display).map(|()| {
            if let Some(ctx) = &app.ui_ctx {
                ctx.request_repaint();
            }
            json!({ "requested": true })
        }),
        // Outline or Preview is a setting of the active document's view.
        "view.outline" => match app.view_mut() {
            Some(v) => flag(&mut v.outline),
            None => Err("no document".into()),
        },
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 3.
fn run_ui_part3(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let flag = |b: &mut bool| {
        *b = !*b;
        Ok(json!(*b))
    };
    let r = match id {
        "view.pixelPreview" => flag(&mut app.ui.view.pixel_preview),
        // Trim View hides the guides while it is on (#8363), and hidden guides cannot stay selected.
        "view.trimView" => {
            let r = flag(&mut app.ui.view.trim_view);
            deselect_hidden_guides(app);
            r
        }
        "view.cornerWidget" => flag(&mut app.ui.view.corner_widgets),
        // Hide/Show Shape Widgets: `on` sets the state, and without it the state flips.
        "view.shapeWidgets" => opt_bool(p, "on").map(|on| {
            let v = &mut app.ui.view.shape_widgets;
            *v = on.unwrap_or(!*v);
            json!(*v)
        }),
        // New View: the engine stores the current zoom, centre and rotation under the given name.
        "view.saved.new" if p.get("zoom").is_none() => {
            let Some(v) = app.view().copied() else { return Some(Err("no document".into())) };
            let mut params = if p.is_object() { p.clone() } else { json!({}) };
            params["center"] = json!([v.center.x, v.center.y]);
            params["zoom"] = json!(v.zoom);
            params["rotation"] = json!(v.rotation);
            app.session.execute(id, &params).map_err(|e| e.to_string())
        }
        // A saved view: its zoom, centre and rotation, in the current window.
        id if saved_view_slot(id).is_some() => {
            let saved = saved_view(app, id).and_then(|sv| crate::state::View::saved(sv).map(|to| (sv.name.clone(), to)));
            match (saved, app.view_mut()) {
                (Some((name, to)), Some(v)) => {
                    (v.center, v.zoom, v.rotation) = (to.center, to.zoom, to.rotation);
                    Ok(json!({ "name": name }))
                }
                _ => Err("no such view".into()),
            }
        }
        id if saved_selection_slot(id).is_some() => match saved_selection_name(app, id) {
            Some(name) => app.run("select.recall", json!({ "name": name })),
            None => Err("no such saved selection".into()),
        },
        "view.snapToPixel" => flag(&mut app.ui.view.snap_to_pixel),
        "view.snapToGlyph" => flag(&mut app.ui.view.snap_to_glyph),
        // The preference itself (General › Use Precise Cursors), so the command, the dialog and the saved
        // preferences share one value.
        "view.preciseCursors" => {
            let on = !app.session.prefs.use_precise_cursors;
            app.run("prefs.set", json!({"key": "usePreciseCursors", "value": on})).map(|_| {
                crate::prefs_dialog::snapshot(app);
                json!(on)
            })
        }
        "view.textThreads" => flag(&mut app.ui.view.text_threads),
        "type.hiddenCharacters" => flag(&mut app.ui.view.hidden_chars),
        "edit.autoSpellCheck" => flag(&mut app.ui.view.auto_spell_check),
        "type.bold" => crate::panels::character::toggle_face(app, Face::Bold),
        "type.italic" => crate::panels::character::toggle_face(app, Face::Italic),
        "type.highlightFontName" => crate::panels::character::highlight(app, crate::panels::character::Field::FontName),
        "type.highlightFontSize" => crate::panels::character::highlight(app, crate::panels::character::Field::FontSize),
        "type.highlightKerning" => crate::panels::character::highlight(app, crate::panels::character::Field::Kerning),
        "type.highlightTracking" => crate::panels::character::highlight(app, crate::panels::character::Field::Tracking),
        "type.underline" => crate::panels::character::toggle_underline(app),
        "type.superscript" => crate::panels::character::toggle_position(app, "superscript"),
        "type.subscript" => crate::panels::character::toggle_position(app, "subscript"),
        "type.lineComposer" => crate::panels::paragraph::toggle_composer(app),
        "type.hyphenate" => crate::panels::paragraph::toggle_hyphenate(app),
        id if let Some((_, justify)) = crate::panels::paragraph::ALIGN_COMMANDS.iter().find(|c| c.0 == id) => {
            crate::panels::paragraph::align(app, justify)
        }
        "type.applyBullets" => crate::panels::paragraph::apply_bullets(app, p),
        "type.applyNumbers" => crate::panels::paragraph::apply_numbers(app, p),
        "ui.bulletsAndNumbering" => crate::dialogs::bullets_numbering::open(app, p),
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 4.
fn run_ui_part4(app: &mut VectorcraftApp, id: &str, _p: &Value) -> Option<Result<Value, String>> {
    let flag = |b: &mut bool| {
        *b = !*b;
        Ok(json!(*b))
    };
    let r = match id {
        "type.convertListToText" => crate::panels::paragraph::convert_list_to_text(app),
        id if let Some(b) = crate::panels::align::button(id) => crate::panels::align::run_button(app, b),
        id if let Some(v) = crate::panels::align::align_to_command(id) => crate::panels::align::run_align_to(app, v),
        "view.gradientAnnotator" => flag(&mut app.ui.view.gradient_annotator),
        "view.livePaintGaps" => flag(&mut app.ui.view.live_paint_gaps),
        "effect.last" => match app.last_effect.clone() {
            Some((e, params)) => crate::dialogs::effect::open_last(app, &e, params),
            None => Err("no effect applied yet".into()),
        },
        id if id.starts_with("type.recentFont") => {
            let n: usize = id["type.recentFont".len()..].parse().unwrap_or(0);
            match n.checked_sub(1).and_then(|i| app.recent_fonts().get(i)).cloned() {
                Some(font) => app.run("text.setStyle", json!({ "font": font })),
                None => Err("no such recent font".into()),
            }
        }
        "view.edges" => flag(&mut app.ui.view.edges),
        "view.artboards" => flag(&mut app.ui.view.artboards),
        "view.template" => flag(&mut app.ui.view.templates),
        "view.rulers" => flag(&mut app.ui.view.rulers),
        "view.videoRulers" => {
            let v = &mut app.ui.view;
            let show = !(v.video_rulers && v.rulers);
            v.video_rulers = show;
            v.rulers |= show;
            Ok(json!(show))
        }
        "view.boundingBox" => flag(&mut app.ui.view.bounding_box),
        // Trim View hides the guides and the grid while it is on, and their Show/Hide items wait
        // until it is off (#8363).
        "view.guides" | "view.grid" if app.ui.view.trim_view => Err(TRIM_VIEW_HIDES.into()),
        "view.guides" => {
            let r = flag(&mut app.ui.view.guides);
            deselect_hidden_guides(app);
            r
        }
        "view.smartGuides" => flag(&mut app.ui.view.smart_guides),
        "view.grid" => flag(&mut app.ui.view.grid),
        "view.snapToGrid" => flag(&mut app.ui.view.snap_to_grid),
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 5.
fn run_ui_part5(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let flag = |b: &mut bool| {
        *b = !*b;
        Ok(json!(*b))
    };
    let r = match id {
        "view.snapToPoint" => flag(&mut app.ui.view.snap_to_point),
        "view.zoomIn" | "view.zoomOut" => {
            let up = id == "view.zoomIn";
            let print = app.print_ppi();
            // Selection & Anchor Display › Zoom to Selection: the selection comes to the middle.
            let focus = if app.session.prefs.zoom_to_selection { app.selection_box().map(|b| b.center()) } else { None };
            match app.view_mut() {
                Some(v) => {
                    v.zoom = next_zoom(v.zoom, up, print);
                    v.fit_zoom = None;
                    if let Some(c) = focus {
                        v.center = c;
                    }
                    Ok(json!({"zoom": crate::state::zoom_display_percent(v.zoom, print)}))
                }
                None => Err("no document".into()),
            }
        }
        "view.setZoom" => {
            let print = app.print_ppi();
            let (zoom, center) = match opt_number(p, "zoom").and_then(|z| Ok((z, opt_point(p, "center")?))) {
                Ok(given) => given,
                Err(e) => return Some(Err(e)),
            };
            let z = crate::state::zoom_from_display_percent(zoom.unwrap_or(100.0), print);
            match app.view_mut() {
                Some(v) => {
                    v.zoom = z.clamp(0.0313, 640.0);
                    // A percentage, not a fit: the first reading of the display keeps it.
                    v.fit_zoom = None;
                    v.fitted = true;
                    if let Some(c) = center {
                        v.center = c;
                    }
                    Ok(Value::Null)
                }
                None => Err("no document".into()),
            }
        }
        "view.fitArtboard" | "view.fitAll" | "view.actualSize" => {
            let command = match id {
                "view.fitArtboard" => ViewCommands::FitArtboard,
                "view.fitAll" => ViewCommands::FitOnScreen,
                _ => ViewCommands::ActualPixels,
            };
            app.request_view(ViewRequest::Command(command), None);
            Ok(Value::Null)
        }
        "view.goToArtboard" => go_to_artboard(app, p),
        "view.statusBarShow" => crate::chrome::set_status_bar_show(app, p),
        // The navigator's four buttons as commands of their own, so Edit › Keyboard Shortcuts lists
        // them (view.goToArtboard needs an index, so a key alone can't run it).
        "view.firstArtboard" => go_to_artboard(app, &json!({ "index": "first" })),
        "view.previousArtboard" => go_to_artboard(app, &json!({ "index": "previous" })),
        "view.nextArtboard" => go_to_artboard(app, &json!({ "index": "next" })),
        "view.lastArtboard" => go_to_artboard(app, &json!({ "index": "last" })),
        "view.presentation" => {
            app.ui.screen_mode = if app.ui.screen_mode == 3 { 0 } else { 3 };
            Ok(json!(app.ui.screen_mode))
        }
        "view.screenMode" => {
            let mode = match opt_index(p, "mode") {
                Ok(mode) => mode,
                Err(e) => return Some(Err(e)),
            };
            app.ui.screen_mode = match mode {
                Some(m) => m.min(3) as u8,
                // F cycles the three screen modes. Without a mode, the command also goes from
                // Presentation Mode (not in the cycle) back to Normal.
                None if app.ui.screen_mode >= 2 => 0,
                None => app.ui.screen_mode + 1,
            };
            Ok(json!(app.ui.screen_mode))
        }
        // View › Rotate View and the status bar's angle menu: the view turns about the center of the
        // canvas to the angle, normalized as a Rotate View drag normalizes it.
        "view.rotate" => {
            let key = "angle";
            let Some(angle) = p.get(key).and_then(Value::as_f64).filter(|a| a.is_finite()) else {
                return Some(Err(format!("`{key}` must be a number")));
            };
            match app.view_mut() {
                Some(v) => {
                    v.rotation = vectorcraft_geom::normalize_deg(angle);
                    Ok(json!({ "angle": v.rotation }))
                }
                None => Err("no document".into()),
            }
        }
        // View › Rotate View to Selection: the view turns about the center of the canvas to the angle
        // of the selection's bounding box, so the box is square to the window.
        "view.rotateToSelection" => {
            let Some(st) = app.session.active() else { return Some(Err("no document".into())) };
            if st.selection.objects.is_empty() {
                return Some(Err("nothing selected".into()));
            }
            let angle = vectorcraft_geom::normalize_deg(st.doc.bbox_angle(&st.selection.objects));
            match app.view_mut() {
                Some(v) => {
                    v.rotation = angle;
                    Ok(json!({ "angle": angle }))
                }
                None => Err("no document".into()),
            }
        }
        "view.rotateReset" => {
            if let Some(v) = app.view_mut() {
                v.rotation = 0.0;
            }
            Ok(Value::Null)
        }
        "window.control" => flag(&mut app.ui.control_bar),
        "window.helpBar" => flag(&mut app.ui.help_bar),
        // The Control bar's panel menu and gripper: the edge of the window the bar docks to, or
        // floating over the window at the position it last floated at.
        "window.control.dock" => {
            let key = "edge";
            let edges = crate::chrome::CONTROL_BAR_EDGES;
            match edges.iter().position(|e| s(key).as_deref() == Some(*e)) {
                Some(i) => {
                    app.ui.control_bar_floating = i == 2;
                    if i < 2 {
                        app.ui.control_bar_bottom = i == 1;
                    }
                    Ok(json!({ key: edges[i] }))
                }
                None => Err(format!("`{key}` must be one of {}", edges.join(", "))),
            }
        }
        // The Control bar's panel menu: a group of the bar's options on or off.
        "window.control.options" => {
            let groups = crate::chrome::CONTROL_BAR_GROUPS;
            match (groups.iter().find(|g| s("group").as_deref() == Some(**g)), opt_bool(p, "visible")) {
                (None, _) => Err(format!("`group` must be one of {}", groups.join(", "))),
                (_, Err(e)) => Err(e),
                (Some(&group), Ok(visible)) => {
                    let hidden = &mut app.ui.control_bar_hidden_groups;
                    let visible = visible.unwrap_or_else(|| hidden.contains(group));
                    if visible {
                        hidden.remove(group);
                    } else {
                        hidden.insert(group.to_string());
                    }
                    Ok(json!({ "group": group, "visible": visible }))
                }
            }
        }
        "window.toolbar" => {
            let on = !crate::toolbar::shown(&app.ui);
            crate::toolbar::set_shown(app, on);
            Ok(json!(on))
        }
        "window.toolbarCompact" => opt_bool(p, "on").map(|on| {
            let on = on.unwrap_or(!app.ui.toolbar_compact);
            app.ui.toolbar_compact = on;
            json!(on)
        }),
        "window.toolbarColumns" => {
            let panel = match toolbar_panel(&app.ui, p) {
                Ok(panel) => panel,
                Err(e) => return Some(Err(e)),
            };
            opt_bool(p, "double").and_then(|on| {
                let on = on.unwrap_or(!toolbar_double(&app.ui, &panel));
                let ctx = app.ui_ctx.clone();
                crate::panel_docking::apply(app, ctx.as_ref(), vec![Action::set_column_steps(panel, if on { 2 } else { 1 })])?;
                Ok(json!(on))
            })
        }
        "window.toolbarAdvanced" => opt_bool(p, "advanced").and_then(|advanced| match advanced {
            Some(advanced) => crate::edit_toolbar::pick_layout(app, advanced).map(|()| json!(advanced)),
            None => flag(&mut app.ui.toolbar_advanced),
        }),
        "imageTrace.preview" => opt_bool(p, "on").map(|on| {
            let on = on.unwrap_or(!app.ui.image_trace_preview);
            app.ui.image_trace_preview = on;
            json!(on)
        }),
        "window.floatTools" => {
            let floating = match opt_bool(p, "floating") {
                Ok(on) => on,
                Err(e) => return Some(Err(e)),
            };
            let Some(tool) = s("tool") else { return Some(Err("tool (a tool id) is required".into())) };
            let toolbar = match crate::custom_toolbars::target(&app.ui, p) {
                Ok(t) => t,
                Err(e) => return Some(Err(e)),
            };
            crate::toolbar::float_group(app, &tool, floating, toolbar.as_deref()).map(Value::Bool)
        }
        _ => return None,
    };
    Some(r)
}

/// Pins or unpins a canvas bar (`{pinned?}`, none toggles). The bar settles it on its next pass: pinned where it
/// shows, or unpinned following its anchor from there.
fn pin_bar(place: &mut crate::canvas::bar_place::KeyedPlace, p: &Value) -> Result<Value, String> {
    opt_bool(p, "pinned").map(|pinned| {
        let pinned = pinned.unwrap_or(!place.place.pinned);
        place.place.pinned = pinned;
        json!(pinned)
    })
}

/// UI commands, part 6.
fn run_ui_part6(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let flag = |b: &mut bool| {
        *b = !*b;
        Ok(json!(*b))
    };
    let r = match id {
        "window.taskBar" => flag(&mut app.ui.task_bar),
        "window.pathfinder.compoundShapes" => flag(&mut app.ui.shape_modes_compound),
        "window.taskBar.pin" => pin_bar(&mut app.ui.task_bar_place, p),
        "window.taskBar.reset" => {
            app.ui.task_bar_place = Default::default();
            Ok(Value::Null)
        }
        // More Options › Bar Actions: one of the bar's actions or chips on or off.
        "window.taskBar.actions" => match (s("action").filter(|a| crate::canvas::is_task_bar_action(a)), opt_bool(p, "visible")) {
            (None, _) => {
                let ids = crate::canvas::TASK_BAR_ACTIONS.iter().map(|a| a.0).chain(crate::canvas::TASK_BAR_CHIPS);
                Err(format!("`action` must be one of {}", ids.collect::<Vec<_>>().join(", ")))
            }
            (_, Err(e)) => Err(e),
            (Some(action), Ok(visible)) => {
                let hidden = &mut app.ui.task_bar_hidden_actions;
                let visible = visible.unwrap_or_else(|| hidden.contains(&action));
                if visible {
                    hidden.remove(&action);
                } else {
                    hidden.insert(action.clone());
                }
                Ok(json!({ "action": action, "visible": visible }))
            }
        },
        // More Options › Reset Bar Actions: every action and chip shows again.
        "window.taskBar.resetActions" => Ok(json!(std::mem::take(&mut app.ui.task_bar_hidden_actions))),
        "window.quickPenToolbar" => flag(&mut app.ui.quick_pen_bar),
        "window.quickPenToolbar.pin" => pin_bar(&mut app.ui.quick_pen_bar_place, p),
        "window.quickPenToolbar.reset" => {
            app.ui.quick_pen_bar_place = Default::default();
            Ok(Value::Null)
        }
        "tool.quickPen.flipArc" => {
            // C's flip, for the toolbar's Flip Arc button: false, with nothing changed beyond the
            // edit finished first, while the last piece of the path being drawn is not a sharp-corner
            // arc. Input held for an edit is finished first, so the key is made now rather than
            // held; when that fails the error is returned and the flip is not made.
            match app.session.finish_tool_edit() {
                Ok(reqs) => crate::canvas::apply_requests(app, Ok(reqs)),
                Err(e) => return Some(Err(e.to_string())),
            }
            let view = app.view_info();
            let key = vectorcraft_tools::ToolKey::Letter('c');
            if app.session.tool_id() != vectorcraft_tools::quickpen::ID || !app.session.tool_claims_key(key, view) {
                return Some(Ok(json!(false)));
            }
            match app.session.tool_key(key, vectorcraft_tools::Mods::default(), view) {
                Ok(reqs) => {
                    crate::canvas::apply_requests(app, Ok(reqs));
                    Ok(json!(true))
                }
                Err(e) => Err(e.to_string()),
            }
        }
        "window.dock" => match app.ui_ctx.clone() {
            // Tab: hide the panels, the toolbars and the Control bar, or show what the last Tab
            // hid; the window's shell applies it in its next pass.
            Some(ctx) => {
                let hidden = craft::ui::shell::chrome_state(&ctx, egui::ViewportId::ROOT).hidden;
                request_chrome_toggle(&ctx, egui::ViewportId::ROOT, if hidden { ChromeToggle::ShowPanels } else { ChromeToggle::Panels });
                Ok(json!(hidden))
            }
            None => Err("no window".into()),
        },
        "window.panels" => match app.ui_ctx.clone() {
            // Shift+Tab: hide or show the panels, leaving the toolbars and the Control bar.
            Some(ctx) => {
                let hidden = craft::ui::shell::chrome_state(&ctx, egui::ViewportId::ROOT).panels_hidden;
                request_chrome_toggle(&ctx, egui::ViewportId::ROOT, if hidden { ChromeToggle::ShowPanels } else { ChromeToggle::PanelsKeepTools });
                if !hidden {
                    app.ui.open_panel = None;
                }
                Ok(json!(hidden))
            }
            None => Err("no window".into()),
        },
        "window.brightness" => match s("brightness").as_deref().and_then(Brightness::parse) {
            Some(b) => {
                app.ui.brightness = b;
                app.session.prefs.ui_brightness = b.id().into();
                app.canvas.key = None;
                Ok(json!(b.id()))
            }
            None => Err("brightness must be dark|mediumDark|mediumLight|light".into()),
        },
        "window.newWindow" => crate::doc_windows::new_window(app),
        "window.arrange.cascade" => crate::doc_windows::cascade(app),
        "window.arrange.tile" => crate::canvas::tile(app, p),
        "window.arrange.floatInWindow" => crate::doc_windows::float_in_window(app, p),
        "window.arrange.floatAllInWindows" => crate::doc_windows::float_all(app),
        "window.arrange.consolidateAllWindows" if p.get("index").is_some_and(|i| !i.is_null()) => crate::doc_windows::dock(app, p),
        "window.arrange.consolidateAllWindows" => crate::canvas::consolidate(app),
        "window.nextDocument" => crate::chrome::step_document(app, true),
        "window.previousDocument" => crate::chrome::step_document(app, false),
        "tool.select" => match s("tool") {
            Some(t) if vectorcraft_tools::tool_info(&t).is_some() => app.try_select_tool(&t).map(|()| json!({"tool": app.session.tool_id()})),
            Some(t) => Err(format!("unknown tool `{t}`")),
            None => Err("missing `tool`".into()),
        },
        "tool.selectLastSelection" => {
            let id = app.session.last_selection_tool();
            let switched = if app.session.tool_id() == id { Ok(()) } else { app.try_select_tool(id) };
            switched.map(|()| json!({"tool": app.session.tool_id()}))
        }
        "tool.switchSelectionTools" => {
            let tool = if app.session.tool_id() == "selection" { "directSelection" } else { "selection" };
            app.try_select_tool(tool).map(|()| json!({"tool": app.session.tool_id()}))
        }
        crate::touch::TOGGLE_MODE => crate::touch::toggle_mode(app),
        crate::addons::pathtools::corner::COMMAND => crate::addons::pathtools::corner::open(app),
        crate::addons::pathtools::shared_edge::COMMAND => crate::addons::pathtools::shared_edge::open(app),
        crate::addons::arrange::ZOOM => crate::addons::arrange::zoom(app),
        crate::addons::select::link_status::COMMAND => crate::addons::select::link_status::run(app, p),
        crate::addons::document::SAVE_COPY => crate::addons::document::save_copy(app, p),
        crate::addons::document::EXPORT_ALL => crate::addons::document::export_all(app, p),
        crate::addons::document::BATCH_TRACE => crate::addons::document::batch_trace(app, p),
        "tool.setOption" => app.session.set_tool_option_cmd(p),
        "effect.dialog" => crate::dialogs::open_effect_dialog(app, p),
        "ui.recolorDialog" => crate::dialogs::recolor::open(app, p),
        "ui.cropImage" if !selected_image(app, |_| true) => Err("select an image".into()),
        "ui.cropImage" => app.try_select_tool(vectorcraft_tools::cropimage::ID).map(|()| json!({"tool": app.session.tool_id()})),
        // Create Object Mosaic… has a dialog of its own, opened on the selected image.
        "ui.paramDialog" if s("command").as_deref() == Some(crate::dialogs::object_mosaic::COMMAND) => {
            crate::dialogs::object_mosaic::open(app, p.get("params").unwrap_or(&Value::Null))
        }
        "ui.paramDialog" => {
            let cmd = s("command").unwrap_or_default();
            let fields = p.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
            // Find and Replace has a dialog of its own, which stays open from match to match.
            if cmd == crate::dialogs::find_replace::COMMAND {
                return Some(crate::dialogs::find_replace::open(app, &fields));
            }
            let label = s("label").unwrap_or(cmd.clone());
            crate::dialogs::command::open(app, &cmd, &label, fields)
        }
        "effect.applyLast" => match app.last_effect.clone() {
            Some((e, params)) => app.run("effect.apply", json!({"effect": e, "params": params})),
            None => Err("no effect applied yet".into()),
        },
        "pathfinder.repeat" => crate::panels::pathfinder::repeat(app),
        "file.export.pdf" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::open_save_pdf(app, &json!({})),
        "file.export.pdf" => io::export_pdf(app, p.clone()),
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 7.
fn run_ui_part7(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "help.about" => {
            crate::credits::open(app);
            Ok(Value::Null)
        }
        "help.checkForUpdates" => crate::updates_ui::check(app),
        "help.safeMode" => crate::dialogs::safe_mode::open(app),
        "help.copyTroubleshootingInfo" => crate::troubleshooting::copy(app),
        "help.commandPalette" => {
            crate::palette::toggle(app);
            Ok(Value::Null)
        }
        // A pick of it in the search is recorded and counted first, and then cleared with the rest.
        "help.clearRecentCommands" => {
            app.ui.recent_commands.clear();
            app.ui.command_uses = Default::default();
            Ok(Value::Null)
        }
        // Closing asks Save / Don't Save / Cancel for modified documents first (`unsaved`).
        // Without an `index`, the current window closes: one of several views of a document closes
        // without closing the document (Window › New Window).
        "file.close" => match opt_index(p, "index") {
            Err(e) => Err(e),
            Ok(Some(i)) => crate::unsaved::close(app, i),
            Ok(None) => {
                app.session.active_index()?;
                crate::doc_windows::close_current(app)
            }
        },
        "file.closeAll" => crate::unsaved::close_all(app, "closeAll"),
        "app.quit" => crate::unsaved::close_all(app, "quit"),
        "ui.swatchOptions" => match s("name") {
            Some(name) => crate::dialogs::swatch_options::open(app, &name),
            None => Err("missing `name`".into()),
        },
        "ui.newSwatch" => {
            let on = |k: &str| opt_bool(p, k).map(|on| on.unwrap_or(false));
            on("spot")
                .and_then(|spot| Ok((spot, on("global")?)))
                .and_then(|(spot, global)| crate::dialogs::new_swatch::open(app, spot, global, s("group").as_deref()))
        }
        "ui.newColorGroup" => {
            let names = p.get("swatches").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::new_color_group::open(app, names.unwrap_or_default())
        }
        "ui.colorGroupOptions" => crate::dialogs::color_group_options::open(app, p),
        "ui.colorPicker" => crate::dialogs::open_color_picker(app, p),
        "ui.sampleScreenColor" => crate::screen_color::start(app, p),
        "ui.graphicStyleOptions" => crate::dialogs::graphic_style_options::open(app, s("name").as_deref()),
        "ui.charStyleOptions" => match s("name") {
            Some(name) => crate::dialogs::char_style_options::open(app, &name),
            None => Err("missing `name`".into()),
        },
        "ui.charStyleOptions.resetPanel" => crate::dialogs::char_style_options::reset_panel(app),
        "ui.paraStyleOptions" => match s("name") {
            Some(name) => crate::dialogs::para_style_options::open(app, &name),
            None => Err("missing `name`".into()),
        },
        "ui.paraStyleOptions.resetPanel" => crate::dialogs::para_style_options::reset_panel(app),
        "ui.newCharStyle" => crate::panels::text_styles::open_new(app, crate::panels::text_styles::Kind::Char),
        "ui.newParaStyle" => crate::panels::text_styles::open_new(app, crate::panels::text_styles::Kind::Para),
        "ui.symbolOptions" => {
            let name = s("name");
            let edit = match opt_bool(p, "edit") {
                Ok(edit) => edit.unwrap_or(false),
                Err(e) => return Some(Err(e)),
            };
            let keep_art = match opt_bool(p, "keepArt") {
                Ok(keep) => keep.unwrap_or(false),
                Err(e) => return Some(Err(e)),
            };
            if edit || name.is_some() {
                if keep_art {
                    return Some(Err("`keepArt` applies only to a new symbol".into()));
                }
                crate::dialogs::symbol_options::open_edit(app, name.as_deref())
            } else {
                crate::dialogs::symbol_options::open(app, keep_art)
            }
        }
        "ui.newBrush" => crate::dialogs::new_brush::open(app, p.get("ids").cloned()),
        "ui.brushOptions" => crate::dialogs::brush_options::open(app, s("name").as_deref()),
        "ui.brushObjectOptions" => crate::dialogs::brush_options::open_object(app),
        "tool.options" => crate::toolbar::open_options(app, &s("tool").unwrap_or_default()),
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 8.
fn run_ui_part8(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "ui.colorGuideOptions" => {
            crate::dialogs::color_guide_options::open(app);
            Ok(Value::Null)
        }
        "ui.colorBalanceDialog" => {
            crate::dialogs::color_balance::open(app);
            Ok(Value::Null)
        }
        "ui.saturateDialog" => {
            crate::dialogs::saturate::open(app);
            Ok(Value::Null)
        }
        "ui.proofSetupDialog" => {
            crate::dialogs::proof_setup::open(app);
            Ok(Value::Null)
        }
        "ui.indexedColorDialog" => {
            crate::dialogs::indexed_color::open(app);
            Ok(Value::Null)
        }
        "window.swatchLibrary" => crate::panels::swatches::open_library(app, p),
        "window.swatchLibrary.other" => crate::panels::swatches::other_library(app, s("path")),
        "ui.saveSwatchLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_swatch_library::open(app, names.unwrap_or_default())
        }
        id if id.starts_with(crate::panels::swatches::USER_SLOT) => match crate::panels::swatches::user_library(app, id) {
            Some(l) => crate::panels::swatches::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        "ui.mergeGraphicStyles" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::graphic_style_options::open_merge(app, names.unwrap_or_default())
        }
        "ui.tileEdgeColor" => crate::dialogs::tile_edge_color::open(app),
        "ui.layerOptions" => crate::dialogs::layer_options::open(app, p),
        "ui.layerDelete" => crate::panels::layers::delete(app, p),
        "ui.rename" => crate::panels::layers::rename(app, p),
        "ui.newLayer" => crate::dialogs::layer_options::open_new(app, p),
        "ui.newArtboard" => crate::dialogs::artboard_options::open_new(app),
        "artboard.selectAll" => crate::panels::artboards::select_all_command(app),
        "ui.layersPanelOptions" => crate::dialogs::layers_panel_options::open(app),
        "ui.navigatorPanelOptions" => crate::panels::navigator::set_options(app, p),
        "ui.attributesPanelOptions" => crate::panels::attributes::set_options(app, p),
        "ui.layersExpand" => crate::panels::layers::expand(app, p),
        "ui.layersFilter" => crate::panels::layers::set_filter(app, p),
        "ui.panelView" => crate::panels::set_view(app, p),
        "ui.flattenTransparencyDialog" => crate::dialogs::flatten::open(app),
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 9.
fn run_ui_part9(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "ui.flattenerPresetsDialog" => {
            crate::dialogs::flattener_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.tracingPresetsDialog" => {
            crate::dialogs::tracing_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.flattenerPreview" => crate::panels::flattener_preview::command(app, p),
        "ui.foldPreview" => crate::addons::packaging::preview::command(app, p),
        "ui.copyScriptConsole" => crate::panels::script_console::copy(app, p),
        "window.graphicStyleLibrary" => crate::panels::graphic_styles::open_library(app, p),
        "ui.libraryPanel" => crate::panels::library_panel::command(app, p),
        "ui.libraryPanel.locate" => crate::panels::library_panel::locate(app, p),
        "window.graphicStyleLibrary.other" => crate::panels::graphic_styles::other_library(app, s("path")),
        "ui.saveGraphicStyleLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_style_library::open(app, names.unwrap_or_default())
        }
        "ui.loadCharStyles" => crate::panels::text_styles::load(app, "charStyle.load", s("path")),
        "ui.loadParaStyles" => crate::panels::text_styles::load(app, "paraStyle.load", s("path")),
        "ui.loadAllStyles" => crate::panels::text_styles::load(app, "textStyle.loadAll", s("path")),
        id if id.starts_with(crate::panels::graphic_styles::USER_SLOT) => match crate::panels::graphic_styles::user_library(app, id) {
            Some(l) => crate::panels::graphic_styles::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        "window.symbolLibrary" => crate::panels::symbols::open_library(app, p),
        "window.symbolLibrary.other" => crate::panels::symbols::other_library(app, s("path")),
        "ui.saveSymbolLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_symbol_library::open(app, names.unwrap_or_default())
        }
        id if id.starts_with(crate::panels::symbols::USER_SLOT) => match crate::panels::symbols::user_library(app, id) {
            Some(l) => crate::panels::symbols::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        "window.brushLibrary" => crate::panels::brushes::open_library(app, p),
        "window.brushLibrary.other" => crate::panels::brushes::other_library(app, s("path")),
        "ui.saveBrushLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_brush_library::open(app, names.unwrap_or_default())
        }
        id if id.starts_with(crate::panels::brushes::USER_SLOT) => match crate::panels::brushes::user_library(app, id) {
            Some(l) => crate::panels::brushes::open_library(app, &json!({ "library": l.id })),
            None => Err("no such user library".into()),
        },
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 10.
fn run_ui_part10(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "ui.expandDialog" => match opt_bool(p, "immediate") {
            Ok(Some(true)) => crate::dialogs::expand::expand_now(app),
            Ok(_) => crate::dialogs::expand::open(app),
            Err(e) => Err(e),
        },
        "ui.simplifyAuto" => crate::dialogs::simplify_slider::auto(app),
        "ui.simplifyMoreOptions" => crate::dialogs::simplify_slider::more_options(app),
        "ui.smoothAuto" => crate::dialogs::smooth_slider::auto(app),
        "ui.recolorUndo" => crate::dialogs::recolor::step(app, true),
        "ui.recolorRedo" => crate::dialogs::recolor::step(app, false),
        "ui.dialogReset" => crate::dialogs::host::reset_top(app),
        "attributes.openUrl" => crate::panels::attributes::open_url(app, p),
        "ui.javascriptFilesDialog" => crate::panels::svg_interactivity::files_dialog(app, p),
        "ui.svgFilterDialog" => crate::dialogs::svg_filter::open(app, p),
        "ui.importSvgFilter" => crate::dialogs::svg_filter::import_file(app),
        "ui.spotColors" => crate::dialogs::spot_colors::open(app),
        "ui.menuDialog" => {
            // As from the menu ([`invoke`]), the edit the tool computes on another thread first.
            let r = app.session.finish_tool_edit();
            crate::canvas::apply_requests(app, r);
            match s("command").as_deref() {
                Some(c) if crate::dialogs::envelope::opens(c) => crate::dialogs::envelope::open(app, c),
                c => c
                    .and_then(|c| open_menu_dialog(app, c))
                    .unwrap_or_else(|| Err("`command` must be a command whose menu item opens a dialog (see ui.menuDialog)".into())),
            }
        }
        "ui.widthPointEdit" => crate::dialogs::width_point::open(app, p),
        "ui.corners" => crate::dialogs::corners::open(app, p),
        "ui.hyphenation" => crate::dialogs::hyphenation::open(app),
        "ui.justification" => crate::dialogs::justification::open(app),
        "ui.tateChuYokoSettings" => crate::dialogs::tate_chu_yoko::open(app),
        "ui.warichuSettings" => crate::dialogs::warichu::open(app),
        "ui.colorGuideLimit" => crate::panels::color_guide::set_limit(app, p),
        "ui.colorPanel" => crate::panels::color::command(app, p),
        "ui.colorRgbPercent" => crate::panels::color::rgb_percent_command(app, p),
        "ui.colorWheel" => crate::panels::color_wheel::command(app, p),
        "ui.savePdfDialog" => crate::dialogs::open_save_pdf(app, p),
        crate::dialogs::save_pdf::SAVE_SUMMARY => crate::dialogs::save_pdf::save_summary(app, p),

        "file.exportAs" if s("path").is_none() => {
            crate::dialogs::open_export_as(app, s("format").as_deref(), p);
            Ok(Value::Null)
        }
        "file.exportAs" => io::export(app, s("format").as_deref(), s("path"), p),
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 11.
fn run_ui_part11(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "ui.fileInfoDialog" => crate::dialogs::file_info::open(app),
        "ui.rasterEffectsSettingsDialog" => crate::dialogs::raster_effects::open(app),
        "ui.effectGalleryDialog" => crate::dialogs::effect_gallery::open(app, p),
        "ui.pdfPresetsDialog" => {
            crate::dialogs::pdf_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.pdfPresetDialog" => crate::dialogs::open_pdf_preset(app, p),
        // Save for Office Documents…: its dialog; with options, pick the file (or write `path`).
        "document.exportForOffice" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::office_export::open(app),
        "document.exportForOffice" => io::save_command_output(app, id, "png", p.clone(), |_, path| Ok(json!({ "path": path }))),
        "ui.dxfOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.dxfOptionsDialog" => {
            crate::dialogs::dxf_options::open(app, p);
            Ok(Value::Null)
        }
        "links.editOriginal" => crate::panels::links::open_file(app, p, false),
        "links.reveal" => crate::panels::links::open_file(app, p, true),
        "links.copyInfo" => crate::panels::links::copy_info(app, p),
        "ui.placementOptionsDialog" => crate::dialogs::placement_options::open(app, p),
        "ui.relinkDialog" => crate::dialogs::relink::open(app, p),
        "ui.findLinksInFolder" => {
            crate::dialogs::find_links::find_in_folder_command(app, s("folder"), p.get("ids").and_then(Value::as_array).cloned())
        }
        "ui.packageDialog" => crate::dialogs::package::open(app),
        crate::panels::actions::RENAME_ACTION => crate::panels::actions::rename_action(app, p),
        crate::panels::actions::INCLUDE_STEPS => crate::panels::actions::include_steps(app, p),
        crate::panels::actions::NEW_ACTION => crate::panels::actions::new_action(app, p),
        crate::panels::actions::ACTION_OPTIONS => crate::panels::actions::action_options(app, p),
        crate::panels::actions::PLAY_ACTION => crate::panels::actions::play_action(app, p),
        crate::panels::actions::PLAY_AT => crate::panels::actions::play_at(app, p),
        crate::panels::actions::BUTTON_MODE => crate::panels::actions::button_mode(app, p),
        crate::panels::actions::PLAYBACK_OPTIONS => crate::panels::actions::playback_options(app, p),
        crate::panels::actions::START_RECORDING => crate::panels::actions::record(app, p),
        crate::panels::actions::STOP_RECORDING => crate::panels::actions::stop(app, p),
        crate::panels::actions::INSERT_STOP => crate::panels::actions::insert_stop(app, p),
        crate::panels::actions::INSERT_MENU_ITEM => crate::panels::actions::insert_menu_item(app, p),
        crate::panels::actions::INSERT_SELECT_PATH => crate::panels::actions::insert_select_path(app, p),
        crate::panels::actions::INSERT_SELECT_OBJECT => crate::panels::actions::insert_select_object(app, p),
        crate::panels::actions::CONTINUE_PLAYBACK => crate::panels::actions::continue_playback(app, p),
        crate::panels::actions::DELETE_STEP => crate::panels::actions::delete_step(app, p),
        crate::panels::actions::MOVE_STEP => crate::panels::actions::move_step(app, p),
        "ui.batchDialog" => {
            let index = |k: &str| opt_index(p, k).map(|i| i.unwrap_or(0));
            index("set").and_then(|set| Ok((set, index("action")?))).and_then(|(set, action)| crate::dialogs::action_batch::open(app, set, action))
        }
        crate::panels::actions::SAVE_SET => crate::panels::actions::save_set(app, p),
        crate::panels::actions::LOAD_SET => crate::panels::actions::load_set(app, p),
        crate::panels::actions::REPLACE_SET => crate::panels::actions::replace_set(app, p),
        crate::panels::actions::DELETE_SETS => crate::panels::actions::delete_sets(app, p),
        crate::panels::actions::DUPLICATE => crate::panels::actions::duplicate(app, p),
        crate::panels::actions::RESET => crate::panels::actions::reset(app, p),
        crate::panels::actions::CLEAR => crate::panels::actions::clear(app, p),
        "ui.shortcutImportDialog" => crate::dialogs::shortcut_import::command(app, p),
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 12.
fn run_ui_part12(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "file.showPackage" => crate::dialogs::package::show_package(app, p),
        "docInfo.save" => crate::panels::doc_info::save_report(app, p),
        // Slice Options… and Divide Slices…: their dialogs, on the selected slices.
        "object.slice.options" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::slices::open_options(app),
        "object.slice.divide" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::slices::open_divide(app),
        "ui.epsOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.epsOptionsDialog" => {
            crate::dialogs::eps_options::open(app, p);
            Ok(Value::Null)
        }
        "ui.hpglOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.hpglOptionsDialog" => {
            crate::dialogs::hpgl_options::open(app, p);
            Ok(Value::Null)
        }
        // Save for Web: its dialog; with settings, write the files.
        "file.saveForWeb" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::save_for_web::open(app),
        "file.saveForWeb" => crate::dialogs::save_for_web::save(app, p.clone()),
        "file.saveForWeb.browser" => crate::dialogs::save_for_web::browser_preview(app, p),
        // File → Export Selection…: the selection becomes one asset (`multiple: true`: one per
        // object), shown checked on Export for Screens' Assets tab.
        "file.exportSelection" => {
            let multiple = match opt_bool(p, "multiple") {
                Ok(multiple) => multiple.unwrap_or(false),
                Err(e) => return Some(Err(e)),
            };
            app.run("assets.add", json!({ "multiple": multiple })).map(|r| {
                let ids: Vec<u64> = r["assets"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
                crate::dialogs::open_export_for_screens_assets(app, Some(&ids));
                json!({ "assets": ids })
            })
        }
        "css.copy" => crate::panels::css_properties::copy(app, p),
        "css.exportFile" => crate::panels::css_properties::export_file(app, p),
        // File → Print…: its dialog; with params, print (or save the job as a PDF) without it.
        "file.print" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::print::open(app),
        "file.print" => crate::print::run(app, p),
        "print.printers" => Ok(crate::print::printers(app)),
        "print.printerSetup" => {
            let printer = s("printer").filter(|n| !n.is_empty());
            match app.services.print.as_mut() {
                Some(service) => service.setup(printer.as_deref()).map(|_| Value::Null),
                None => Err("printing isn't available here".into()),
            }
        }
        _ => return None,
    };
    Some(r)
}

/// UI commands, part 13.
fn run_ui_part13(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let r = match id {
        "ui.printPresetsDialog" => {
            crate::dialogs::print_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.printPresetDialog" => crate::dialogs::print::open_preset(app, p),
        "plugin.dialog" => crate::dialogs::plugin::open(app, p),
        crate::dialogs::plugin::APPLY_LAST => crate::dialogs::plugin::apply_last(app),
        crate::dialogs::plugin::LAST => crate::dialogs::plugin::open_last(app),
        "ui.installPlugin" => io::install_plugin(app, p),
        "plugin.cancelRuns" => Ok(crate::plugin_runs::cancel_all(app)),
        "plugin.hideRuns" => Ok(crate::plugin_runs::hide_all(app)),
        "imageTrace.cancelRuns" => Ok(crate::trace_runs::cancel_all(app)),
        "imageTrace.hideRuns" => Ok(crate::trace_runs::hide_all(app)),
        "ui.checkScript" => crate::dialogs::script_check::command(app, p),
        "ui.runScript" => crate::dialogs::script_run::command(app, p),
        crate::scripts_menu::COMMAND => crate::scripts_menu::run(app, p),
        "ui.perspectiveGridDialog" => crate::dialogs::perspective_grid::open(app),
        "ui.perspectivePresetsDialog" => {
            crate::dialogs::perspective_presets::open(app, s("selected").as_deref());
            Ok(Value::Null)
        }
        "ui.savePerspectivePreset" => crate::dialogs::perspective_grid::open_save(app),
        id if id.starts_with(crate::dialogs::perspective_presets::SLOT) => match crate::dialogs::perspective_presets::slot_preset(app, id) {
            Some(name) => app.run("perspective.grid.preset", json!({ "name": name })),
            None => Err("no such perspective grid preset".into()),
        },
        "ui.blendOptions" => crate::dialogs::blend_options::open(app),
        "ui.livePaintGapOptions" => crate::dialogs::live_paint_gaps::open(app),
        "ui.perspectivePlane" => crate::dialogs::perspective_plane::open(app, p),
        crate::dialogs::check_spelling::COMMAND => crate::dialogs::check_spelling::open(app),
        "ui.findAndReplace" => {
            let empty = serde_json::Map::new();
            crate::dialogs::find_replace::open(app, &empty)
        }
        "ui.retype" => crate::panel_docking::reveal_panel(app, crate::panels::retype::ID),

        _ => return None,
    };
    Some(r)
}

/// `view.goToArtboard`: make artboard `index` (a number, or first / previous / next / last from
/// the current one) active, synchronizing the Artboard tool when in use, and fit it in the window.
fn go_to_artboard(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let n = app.session.active().map(|d| d.doc.artboards.len()).ok_or("no document open")?;
    let last = n.checked_sub(1).ok_or("the document has no artboards")?;
    let current = app.view().map_or(0, |v| v.artboard).min(last);
    let to = match p.get("index") {
        Some(Value::String(s)) => match s.as_str() {
            "first" => 0,
            "previous" => current.saturating_sub(1),
            "next" => current + 1,
            "last" => last,
            other => return Err(format!("index must be a number or first, previous, next or last, not `{other}`")),
        },
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("index must be a number or first, previous, next or last")?,
        None => return Err("missing index".into()),
    }
    .min(last);
    crate::panels::artboards::select(app, to);
    app.request_view(ViewRequest::Command(ViewCommands::FitArtboard), None);
    Ok(json!({ "index": to }))
}
