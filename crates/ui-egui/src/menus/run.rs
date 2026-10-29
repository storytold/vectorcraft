use super::bar::home_key;
use super::invoke::menu_dialog;
use super::slots::{recent_slot, saved_selection_name, saved_selection_slot, saved_view, saved_view_slot};
use super::state::{checked, selected_image};
use super::ui_commands::UI_COMMANDS;
use super::{Brightness, Face, SaveMode, Value, VectorcraftApp, io, json, next_zoom};

/// Canonical panel id for `window.panel`: a dock tab's or an icon panel's id or display label,
/// matched case-insensitively (`"Layers"`, `"swatches"`, `"Color Guide"`), or a library panel's id
/// as it is (`library:swatches:pastels`).
pub(crate) fn normalize_panel(input: &str) -> Option<&'static str> {
    let name = input.trim();
    crate::state::all_panels()
        .find(|(id, label)| name.eq_ignore_ascii_case(id) || name.eq_ignore_ascii_case(label))
        .map(|(id, _)| id)
        .or_else(|| crate::panels::library_panel::intern(input))
}

/// An optional `{key?: bool}` param: none when it is omitted or null (the commands then toggle).
pub(crate) fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// Handle a UI command. `None` = not a UI command (the engine handles it).
/// `params` has only keys the params description of engine command `id` names, or the parameter
/// error naming the one it doesn't (#1442). The UI's own commands aren't checked yet: their
/// descriptions haven't been audited against what they read.
pub fn check_params(session: &vectorcraft_engine::Session, id: &str, params: &Value) -> Result<(), String> {
    if UI_COMMANDS.iter().any(|c| c.0 == id) {
        return Ok(());
    }
    session.check_params(id, params).map_err(|e| e.to_string())
}

/// Runs `id` when the UI runs it itself: one of [`UI_COMMANDS`], or an engine command the UI
/// handles its own way (a Window menu panel item that toggles, …) → `None` to leave it to the
/// engine.
pub fn run_ui_command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    // The Window menu's panel items toggle like its other checked items (#2361); the panel's
    // other ways in (shortcuts, Show Properties Panel, a tool's double-click) only show it.
    if id == "window.panel" && p.get("toggle").and_then(Value::as_bool) == Some(true) && checked(app, id, p) == Some(true) {
        let panel = p.get("panel").and_then(Value::as_str).and_then(normalize_panel).unwrap_or_default();
        if app.ui.open_panel.as_deref() == Some(panel) {
            app.ui.open_panel = None;
            return Some(Ok(json!({"open": null})));
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
            Some(path) => io::open_path(app, &path),
            None => io::open_dialog(app).map(|_| Value::Null),
        },
        // Saves write through the app (save panel, download); with no path known the panel and the
        // format's options dialog ask first.
        "file.save" => io::save(app, SaveMode::Save, p, true),
        "file.saveAs" => io::save(app, SaveMode::SaveAs, p, true),
        "file.saveCopy" => io::save(app, SaveMode::Copy, p, true),
        "file.saveAsTemplate" => io::save(app, SaveMode::Template, p, true),
        "document.exportSelection" => {
            io::save_command_output(app, id, "png", if p.is_object() { p.clone() } else { json!({}) }).map(|p| json!({"path": p}))
        }
        // Bytes sent by an agent go straight to the engine; a path or nothing opens it here.
        "file.newFromTemplate" if p.get("dataBase64").is_none() => io::new_from_template(app, s("path")),
        "file.revert" if p.get("confirmed").and_then(Value::as_bool) != Some(true) => io::ask_revert(app),
        "file.reveal" => io::reveal(app),
        "file.cancelExport" => Ok(io::cancel_exports(app)),
        "file.exportInBackground" => Ok(io::hide_exports(app)),
        id if id.starts_with("file.openRecent") => match recent_slot(app, id).cloned() {
            Some(path) => io::open_recent_reporting(app, &path),
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
        "file.recentFiles" => Ok(json!(io::recent_files(app))),
        "file.place" => crate::place::run(app, p),
        "file.place.queue" => crate::place::queue(app, p),
        "file.export.svg" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::svg_options::open(app, crate::dialogs::svg_options::Mode::Export, None);
            Ok(Value::Null)
        }
        "file.export.svg" => io::export(app, Some("svg"), s("path"), p),
        "file.exportForScreens" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::dialogs::open_export_for_screens(app);
            Ok(Value::Null)
        }
        "file.exportForScreens" => io::export_for_screens(app, p.clone()),
        "file.export.png" if s("path").is_none() => io::target_path(app, None, "png").and_then(|path| {
            let f = vectorcraft_engine::cmd::fileio::format("png").ok_or("no PNG encoder")?;
            let mut params = if p.is_object() { p.clone() } else { json!({}) };
            params["format"] = json!("png");
            params["path"] = json!(path);
            crate::dialogs::open_raster_options(app, f, params);
            Ok(Value::Null)
        }),
        "file.export.png" => io::export(app, Some("png"), s("path"), p),
        "file.documentSetup" => crate::dialogs::open_document_setup(app),
        "edit.preferences" => {
            crate::prefs_dialog::open(app, s("category").as_deref());
            Ok(Value::Null)
        }
        // Edit → Color Settings… / Assign Profile… (no params): show the colour-management panel
        // wherever it is, as Window › Separations Preview does (it isn't in an icon column, so a
        // flyout for it never opened: #4542).
        "edit.colorSettings" | "edit.assignProfile" if p.as_object().is_none_or(|o| o.is_empty()) => {
            crate::panel_docking::reveal_panel(app, "separations").and_then(|_| app.session.execute(id, p).map_err(|e| e.to_string()))
        }
        "view.outline" => flag(&mut app.ui.view.outline),
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
        "view.trimView" => flag(&mut app.ui.view.trim_view),
        "view.cornerWidget" => flag(&mut app.ui.view.corner_widgets),
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
        "type.bold" => crate::panels::character::toggle_face(app, Face::Bold),
        "type.italic" => crate::panels::character::toggle_face(app, Face::Italic),
        "type.highlightFontName" => crate::panels::character::highlight(app, crate::panels::character::Field::FontName),
        "type.highlightKerning" => crate::panels::character::highlight(app, crate::panels::character::Field::Kerning),
        "type.highlightTracking" => crate::panels::character::highlight(app, crate::panels::character::Field::Tracking),
        "type.underline" => crate::panels::character::toggle_underline(app),
        "type.lineComposer" => crate::panels::paragraph::toggle_composer(app),
        "type.hyphenate" => crate::panels::paragraph::toggle_hyphenate(app),
        id if let Some((_, justify)) = crate::panels::paragraph::ALIGN_COMMANDS.iter().find(|c| c.0 == id) => {
            crate::panels::paragraph::align(app, justify)
        }
        "type.applyBullets" => crate::panels::paragraph::apply_bullets(app, p),
        "type.applyNumbers" => crate::panels::paragraph::apply_numbers(app, p),
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
        "view.gradientAnnotator" => flag(&mut app.ui.view.gradient_annotator),
        "view.livePaintGaps" => flag(&mut app.ui.view.live_paint_gaps),
        "effect.last" => match app.last_effect.clone() {
            Some((e, params)) => {
                let label = vectorcraft_effects::effect_info(&e).map(|i| i.label.trim_end_matches('…').to_string()).unwrap_or_else(|| e.clone());
                let mut fields = params.as_object().cloned().unwrap_or_default();
                fields.insert("__effect".into(), json!(e));
                fields.insert("__label".into(), json!(label));
                fields.insert("preview".into(), json!(true));
                app.ui.dialog = Some(crate::state::Dialog { kind: "effect".into(), fields });
                Ok(Value::Null)
            }
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
        "view.guides" => {
            let r = flag(&mut app.ui.view.guides);
            // Hidden ruler guides and guide paths cannot stay selected: their selection overlay
            // would otherwise reveal geometry that Hide Guides just hid.
            let hidden_selection = !app.ui.view.guides
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
            let print = app.session.prefs.display_print_size;
            // Selection & Anchor Display › Zoom to Selection: the selection comes to the middle.
            let focus = if app.session.prefs.zoom_to_selection { app.selection_box().map(|b| b.center()) } else { None };
            match app.view_mut() {
                Some(v) => {
                    v.zoom = next_zoom(v.zoom, up);
                    if let Some(c) = focus {
                        v.center = c;
                    }
                    Ok(json!({"zoom": crate::state::zoom_display_percent(v.zoom, print)}))
                }
                None => Err("no document".into()),
            }
        }
        "view.setZoom" => {
            let print = app.session.prefs.display_print_size;
            let z = crate::state::zoom_from_display_percent(p.get("zoom").and_then(Value::as_f64).unwrap_or(100.0), print);
            let center =
                p.get("center").and_then(Value::as_array).and_then(|a| Some(vectorcraft_geom::Point::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
            match app.view_mut() {
                Some(v) => {
                    v.zoom = z.clamp(0.0313, 640.0);
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
            crate::canvas::fit(app, id);
            Ok(Value::Null)
        }
        "view.goToArtboard" => go_to_artboard(app, p),
        "view.presentation" => {
            app.ui.screen_mode = if app.ui.screen_mode == 3 { 0 } else { 3 };
            Ok(json!(app.ui.screen_mode))
        }
        "view.screenMode" => {
            app.ui.screen_mode = match p.get("mode").and_then(Value::as_u64) {
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
        "window.toolbar" => flag(&mut app.ui.toolbar),
        "window.toolbarColumns" => opt_bool(p, "double").map(|on| {
            let on = on.unwrap_or(!app.ui.toolbar_double);
            app.ui.toolbar_double = on;
            json!(on)
        }),
        "window.toolbarAdvanced" => flag(&mut app.ui.toolbar_advanced),
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
            crate::toolbar::float_group(app, &tool, floating).map(Value::Bool)
        }
        _ => return None,
    };
    Some(r)
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
        "window.taskBar.pin" => {
            let place = &mut app.ui.task_bar_place;
            opt_bool(p, "pinned").map(|pinned| {
                let pinned = pinned.unwrap_or(!place.pinned);
                // Pinning holds the bar where it shows; unpinning lets it follow the selection from
                // there (the bar turns its pinned spot into an offset when it is next drawn).
                if pinned && !place.pinned {
                    place.pin_at = place.shown_at;
                }
                place.pinned = pinned;
                json!(pinned)
            })
        }
        "window.taskBar.reset" => {
            app.ui.task_bar_place = Default::default();
            Ok(Value::Null)
        }
        "window.dock" => {
            let ui = &mut app.ui;
            // The bars the last Tab hid show again, unless the toolbar or the Control bar was shown
            // another way since then (Window › Tools, Window › Control). Panels shown with Shift+Tab
            // in between stay shown. Switching workspaces clears the record (`workspaces::apply`).
            let hid = ui.tab_hid.take().filter(|h| (h.dock || h.toolbar || h.control_bar) && !ui.toolbar && !ui.control_bar);
            let on = if let Some(h) = hid {
                ui.dock |= h.dock;
                ui.toolbar = h.toolbar;
                ui.control_bar = h.control_bar;
                true
            } else if ui.dock || ui.toolbar || ui.control_bar {
                ui.tab_hid = Some(crate::state::TabHid { dock: ui.dock, toolbar: ui.toolbar, control_bar: ui.control_bar });
                ui.dock = false;
                ui.toolbar = false;
                ui.control_bar = false;
                ui.open_panel = None;
                false
            } else {
                ui.dock = true;
                ui.toolbar = true;
                ui.control_bar = true;
                true
            };
            Ok(json!(on))
        }
        "window.panels" => {
            let on = !app.ui.dock;
            app.ui.dock = on;
            if !on {
                app.ui.open_panel = None;
            }
            Ok(json!(on))
        }
        "window.brightness" => match s("brightness").as_deref().and_then(Brightness::parse) {
            Some(b) => {
                app.ui.brightness = b;
                app.session.prefs.ui_brightness = b.id().into();
                app.canvas.key = None;
                Ok(json!(b.id()))
            }
            None => Err("brightness must be dark|mediumDark|mediumLight|light".into()),
        },
        "window.newWindow" => Err("New Window isn't available yet: a document opens in one window".into()),
        "window.nextDocument" => crate::chrome::step_document(app, true),
        "window.previousDocument" => crate::chrome::step_document(app, false),
        "tool.select" => match s("tool") {
            Some(t) if vectorcraft_tools::tool_info(&t).is_some() => {
                app.select_tool(&t);
                Ok(json!({"tool": app.session.tool_id()}))
            }
            Some(t) => Err(format!("unknown tool `{t}`")),
            None => Err("missing `tool`".into()),
        },
        "tool.setOption" => app.session.set_tool_option_cmd(p),
        "effect.dialog" => crate::dialogs::open_effect_dialog(app, p),
        "ui.recolorDialog" => crate::dialogs::recolor::open(app, p),
        "ui.cropImage" if !selected_image(app, |_| true) => Err("select an image".into()),
        "ui.cropImage" => {
            app.select_tool(vectorcraft_tools::cropimage::ID);
            Ok(json!({"tool": app.session.tool_id()}))
        }
        "ui.paramDialog" => {
            let cmd = s("command").unwrap_or_default();
            let mut fields = p.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
            fields.insert("__command".into(), json!(cmd));
            fields.insert("__label".into(), json!(s("label").unwrap_or(cmd.clone())));
            app.ui.dialog = Some(crate::state::Dialog { kind: "command".into(), fields });
            Ok(Value::Null)
        }
        "effect.applyLast" => match app.last_effect.clone() {
            Some((e, params)) => app.run("effect.apply", json!({"effect": e, "params": params})),
            None => Err("no effect applied yet".into()),
        },
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
            app.ui.about = true;
            Ok(Value::Null)
        }
        "help.checkForUpdates" => crate::updates_ui::check(app),
        "help.commandPalette" => {
            app.ui.palette_open = !app.ui.palette_open;
            app.ui.palette_query.clear();
            Ok(Value::Null)
        }
        // Closing asks Save / Don't Save / Cancel for modified documents first (`unsaved`).
        "file.close" => {
            let i = p.get("index").and_then(Value::as_u64).map(|i| i as usize).or(app.session.active_index())?;
            crate::unsaved::close(app, i)
        }
        "file.closeAll" => crate::unsaved::close_all(app, "closeAll"),
        "app.quit" => crate::unsaved::close_all(app, "quit"),
        "ui.swatchOptions" => match s("name") {
            Some(name) => crate::dialogs::swatch_options::open(app, &name),
            None => Err("missing `name`".into()),
        },
        "ui.newSwatch" => {
            let on = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
            crate::dialogs::new_swatch::open(app, on("spot"), on("global"), s("group").as_deref())
        }
        "ui.newColorGroup" => {
            let names = p.get("swatches").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::new_color_group::open(app, names.unwrap_or_default())
        }
        "ui.colorPicker" => crate::dialogs::open_color_picker(app, p),
        "ui.sampleScreenColor" => crate::screen_color::start(app, p),
        "ui.graphicStyleOptions" => crate::dialogs::graphic_style_options::open(app, s("name").as_deref()),
        "ui.newCharStyle" => crate::panels::text_styles::open_new(app, crate::panels::text_styles::Kind::Char),
        "ui.newParaStyle" => crate::panels::text_styles::open_new(app, crate::panels::text_styles::Kind::Para),
        "ui.symbolOptions" => {
            let name = s("name");
            let edit = p.get("edit").and_then(Value::as_bool).unwrap_or(false);
            if edit || name.is_some() {
                crate::dialogs::symbol_options::open_edit(app, name.as_deref())
            } else {
                crate::dialogs::symbol_options::open(app)
            }
        }
        "ui.newBrush" => crate::dialogs::new_brush::open(app, p.get("ids").cloned()),
        "ui.brushOptions" => crate::dialogs::brush_options::open(app, s("name").as_deref()),
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
        "ui.newLayer" => crate::dialogs::layer_options::open_new(app, p),
        "ui.newArtboard" => crate::dialogs::artboard_options::open_new(app),
        "ui.layersPanelOptions" => crate::dialogs::layers_panel_options::open(app),
        "ui.layersExpand" => crate::panels::layers::expand(app, p),
        "ui.layersFilter" => crate::panels::layers::set_filter(app, p),
        "ui.flattenTransparencyDialog" => {
            crate::dialogs::flatten::open(app);
            Ok(Value::Null)
        }
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
        "ui.imageTracePresetDialog" => crate::dialogs::trace_presets::open(app, p),
        "ui.flattenerPreview" => crate::panels::flattener_preview::command(app, p),
        "window.graphicStyleLibrary" => crate::panels::graphic_styles::open_library(app, p),
        "ui.libraryPanel" => crate::panels::library_panel::command(app, p),
        "ui.libraryPanel.locate" => crate::panels::library_panel::locate(app, p),
        "window.graphicStyleLibrary.other" => crate::panels::graphic_styles::other_library(app, s("path")),
        "ui.saveGraphicStyleLibrary" => {
            let names = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            crate::dialogs::save_style_library::open(app, names.unwrap_or_default())
        }
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
        "ui.expandDialog" => crate::dialogs::expand::open(app),
        "ui.simplifyAuto" => crate::dialogs::simplify_slider::auto(app),
        "ui.simplifyMoreOptions" => crate::dialogs::simplify_slider::more_options(app),
        "ui.smoothAuto" => crate::dialogs::smooth_slider::auto(app),
        "ui.dialogReset" => crate::dialogs::reset(app),
        "attributes.openUrl" => crate::panels::attributes::open_url(app, p),
        "ui.javascriptFilesDialog" => crate::panels::svg_interactivity::files_dialog(app, p),
        "ui.spotColors" => crate::dialogs::spot_colors::open(app),
        "ui.menuDialog" => match s("command").as_deref() {
            Some(c) if crate::dialogs::envelope::opens(c) => crate::dialogs::envelope::open(app, c),
            c => match c.and_then(|c| menu_dialog(app, c)) {
                Some((kind, fields)) => {
                    app.ui.dialog = Some(crate::state::Dialog::new(kind, fields));
                    Ok(Value::Null)
                }
                None => Err("`command` must be a command whose menu item opens a dialog (see ui.menuDialog)".into()),
            },
        },
        "ui.widthPointEdit" => crate::dialogs::width_point::open(app, p),
        "ui.corners" => crate::dialogs::corners::open(app, p),
        "ui.hyphenation" => crate::dialogs::hyphenation::open(app),
        "ui.justification" => crate::dialogs::justification::open(app),
        "ui.colorGuideLimit" => crate::panels::color_guide::set_limit(app, p),
        "ui.colorPanel" => crate::panels::color::command(app, p),
        "ui.savePdfDialog" => crate::dialogs::open_save_pdf(app, p),
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
        "document.exportForOffice" => io::save_command_output(app, id, "png", p.clone()).map(|path| json!({ "path": path })),
        "ui.dxfOptionsDialog" if app.session.active().is_none() => Err("no document".into()),
        "ui.dxfOptionsDialog" => {
            crate::dialogs::dxf_options::open(app, p);
            Ok(Value::Null)
        }
        "links.editOriginal" => crate::panels::links::open_file(app, p, false),
        "links.reveal" => crate::panels::links::open_file(app, p, true),
        "ui.placementOptionsDialog" => crate::dialogs::placement_options::open(app, p),
        "ui.findLinksInFolder" => {
            crate::dialogs::find_links::find_in_folder_command(app, s("folder"), p.get("ids").and_then(Value::as_array).cloned())
        }
        "ui.packageDialog" => crate::dialogs::package::open(app),
        "ui.batchDialog" => {
            let set = p.get("set").and_then(Value::as_u64).unwrap_or(0) as usize;
            let action = p.get("action").and_then(Value::as_u64).unwrap_or(0) as usize;
            crate::dialogs::action_batch::open(app, set, action)
        }
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
        // Save for Web: its dialog; with settings, write the files.
        "file.saveForWeb" if p.as_object().is_none_or(|o| o.is_empty()) => crate::dialogs::save_for_web::open(app),
        "file.saveForWeb" => crate::dialogs::save_for_web::save(app, p.clone()),
        "file.saveForWeb.browser" => crate::dialogs::save_for_web::browser_preview(app, p),
        // File → Export Selection…: the selection becomes one asset (`multiple: true`: one per
        // object), shown checked on Export for Screens' Assets tab.
        "file.exportSelection" => {
            app.run("assets.add", json!({ "multiple": p.get("multiple").and_then(Value::as_bool).unwrap_or(false) })).map(|r| {
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
        "ui.installPlugin" => io::install_plugin(app, p),
        "plugin.cancelRuns" => Ok(crate::plugin_runs::cancel_all(app)),
        "plugin.hideRuns" => Ok(crate::plugin_runs::hide_all(app)),
        "imageTrace.cancelRuns" => Ok(crate::trace_runs::cancel_all(app)),
        "imageTrace.hideRuns" => Ok(crate::trace_runs::hide_all(app)),
        "ui.checkScript" => crate::dialogs::script_check::command(app, p),
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
    crate::canvas::fit(app, "view.fitArtboard");
    Ok(json!({ "index": to }))
}
