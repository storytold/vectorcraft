//! View → Perspective Grid → Define Grid: `perspective.grid.get` reads the grid (the model, the
//! Define Grid fields and the station point) and `perspective.grid.define` sets it from those
//! fields. The model and the camera maths live in `vectorcraft_tools::distort::perspective`.
//!
//! Presets: the built-in views (generated in code, fitted to the first artboard, protected) and
//! the user's, saved with the preferences ([`crate::Prefs::perspective_presets`]) as Define Grid
//! fields; `perspective.grid.preset` applies one, `perspective.presets.*` manage them.
//!
//! View options (Lock Grid, Lock Station Point, Snap to Grid, Show Rulers) are view state on
//! the grid: they toggle without an undo step, as Show Grid does.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::{Point, Rect, Vec2};
use vectorcraft_tools::distort::perspective::Plane;
use vectorcraft_tools::distort::perspective::define::{BUILTINS, LETTER, first_artboard, is_builtin};
use vectorcraft_tools::distort::perspective::widget::{WidgetCorner, WidgetPlace};
use vectorcraft_tools::distort::perspective::{GridDefinition, PerspectiveGrid};
use vectorcraft_tools::{PointerEvent, PointerKind, ToolKey};

use super::distortcmds::{grid_of, silent, store_grid};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "perspective.grid.get",
            "Perspective Grid",
            [],
            None,
            "{} → {grid: the model (as perspective.grid.set takes it), define: the Define Grid fields (as perspective.grid.define takes them), station: {x, y, distance} (the viewer: centre of vision on the horizon, distance to the picture plane, pt), defined: false while the document uses the default grid}",
            has_doc,
            grid_get
        ),
        cmd!(
            "perspective.grid.define",
            "Define Perspective Grid",
            [],
            None,
            "{name?: preset name (\"\" = custom), kind?: 1|2|3, units?: unit name (points, pixels, inches, centimeters, millimeters…), scale?: [artboard, real world], gridline?: gridline every, angle?: viewing angle (0–90°), distance?: viewing distance, horizonHeight?: above the ground level, thirdVp?: [x right, y up] from the centre of vision (3-point), leftColor?, rightColor?, groundColor?: \"#rrggbb\", opacity?: 0–100} lengths are real-world lengths in `units` at `scale`; missing fields keep the grid's (see perspective.grid.get define). Only what changes changes: a new type, angle or distance moves the vanishing points around the station point. One undo step → the new define fields",
            has_doc,
            grid_define
        ),
        cmd!(
            query "perspective.presets.list",
            "Perspective Grid Presets",
            [],
            None,
            "{} → {presets: [{builtIn, name, kind, …the perspective.grid.define fields}]} the built-in views (fitted to the first artboard, protected) first, then the saved presets",
            always,
            presets_list
        ),
        cmd!(
            query "perspective.presets.save",
            "Save Perspective Grid Preset",
            [],
            None,
            "{name?: (default: a new \"Perspective Preset N\"), newName?: rename it, preset?: the preset to start from (default: the saved preset `name`, else the document's grid), …the fields of perspective.grid.define (over it)} create or change a saved preset (Save Grid as Preset: just `name`); built-in ones are protected → {name, created, definition}",
            always,
            presets_save
        ),
        cmd!(
            query "perspective.presets.delete",
            "Delete Perspective Grid Preset",
            [],
            None,
            "{name} delete a saved preset (built-in ones stay) → {deleted: name}",
            always,
            presets_delete
        ),
        cmd!(
            query "perspective.presets.export",
            "Export Perspective Grid Presets",
            [],
            None,
            "{names?: [preset names, built-in ones too] (default: every saved preset), path?} write the presets as a .vcperspective file (JSON) → {path, count}; without path → {data: the file's text, count}",
            always,
            presets_export
        ),
        cmd!(
            query "perspective.presets.import",
            "Import Perspective Grid Presets",
            [],
            None,
            "{path? | data?: file text | dataBase64?, replace?: false (replace saved presets of the same names; else imported ones whose name is taken get a number)} add the presets of a .vcperspective file (as perspective.presets.export writes) to the saved ones → {imported: [names]}",
            always,
            presets_import
        ),
        cmd!(
            query "perspective.widget.options",
            "Perspective Grid Options",
            [],
            None,
            "{show?: bool, position?: topLeft|topRight|bottomLeft|bottomRight} the Plane Switching Widget: shown or not, and the corner of the document window it stays in (kept with the preferences; no params reads them) → {show, position}",
            always,
            widget_options
        ),
        cmd!(
            "perspective.grid.lock",
            "Lock Grid",
            [],
            None,
            "{on?: bool} (no param toggles) View › Perspective Grid › Lock Grid: the grid's widgets can't be dragged (the Plane Switching Widget still works). View state: not an undo step → {on}",
            has_doc,
            lock
        ),
        cmd!(
            "perspective.grid.lockStation",
            "Lock Station Point",
            [],
            None,
            "{on?: bool} (no param toggles) dragging one vanishing point moves the other around the station point (the viewing angle turns, the viewer stays). View state: not an undo step → {on}",
            has_doc,
            lock_station
        ),
        cmd!(
            "perspective.grid.snap",
            "Snap to Grid",
            [],
            None,
            "{on?: bool} (no param toggles) art drawn or moved in perspective lands on gridlines within a quarter cell (on by default). View state: not an undo step → {on}",
            has_doc,
            snap
        ),
        cmd!(
            "perspective.grid.rulers",
            "Show Rulers",
            [],
            None,
            "{on?: bool} (no param toggles) a ruler up the line where the planes meet, in the grid's units at its scale. View state: not an undo step → {on}",
            has_doc,
            rulers
        ),
    ]
}

/// The `format` of a perspective grid presets file, also its extension.
pub const PRESET_FORMAT: &str = "vcperspective";

/// The extensions of perspective grid presets files (opening one imports it).
pub const PRESET_EXTS: &[&str] = &[PRESET_FORMAT];

/// The most presets one file imports.
const MAX_IMPORT: usize = 1000;

/// What `perspective.presets.export` writes and `perspective.presets.import` reads.
#[derive(Serialize, Deserialize)]
struct PresetFile {
    format: String,
    #[serde(default)]
    presets: Vec<Value>,
}

impl Session {
    /// The artboard presets are fitted to: the active document's first (a Letter page without).
    fn preset_artboard(&self) -> Rect {
        self.active().map_or(LETTER, |d| first_artboard(&d.doc))
    }

    /// Every perspective grid preset: the built-in views, then the saved ones.
    pub fn perspective_presets(&self) -> Vec<GridDefinition> {
        let ab = self.preset_artboard();
        let builtins = BUILTINS.iter().filter_map(|b| PerspectiveGrid::builtin(b.0, ab)).map(|g| g.definition());
        builtins.chain(self.prefs.perspective_presets.iter().cloned()).collect()
    }

    /// The preset `name` (any case): a built-in view or a saved one.
    pub fn perspective_preset(&self, name: &str) -> Option<GridDefinition> {
        match PerspectiveGrid::builtin(name, self.preset_artboard()) {
            Some(g) => Some(g.definition()),
            None => self.persp_saved_index(name).and_then(|i| self.prefs.perspective_presets.get(i)).cloned(),
        }
    }

    /// The grid preset `name` makes on `ab`: a built-in view, or a saved preset's fields over the
    /// normal view of its type.
    pub fn perspective_preset_grid(&self, name: &str, ab: Rect) -> Option<PerspectiveGrid> {
        if let Some(g) = PerspectiveGrid::builtin(name, ab) {
            return Some(g);
        }
        let def = self.perspective_preset(name)?;
        PerspectiveGrid::normal(def.kind, ab).with_definition(&def).ok()
    }

    /// The first free "Perspective Preset N": the name a new preset gets.
    pub fn new_perspective_preset_name(&self) -> String {
        (1..).map(|i| format!("Perspective Preset {i}")).find(|n| !self.persp_preset_taken(n, None)).unwrap_or_default()
    }

    /// The index of the saved preset `name` (any case).
    fn persp_saved_index(&self, name: &str) -> Option<usize> {
        self.prefs.perspective_presets.iter().position(|q| q.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Whether `name` is a built-in view's or a saved preset's other than the one at `except`.
    fn persp_preset_taken(&self, name: &str, except: Option<usize>) -> bool {
        is_builtin(name) || self.persp_saved_index(name).is_some_and(|i| Some(i) != except)
    }

    /// The name of the first preset among `candidates` whose fields `def` has ("" for none).
    fn preset_named(&self, def: &GridDefinition, candidates: &[&str]) -> String {
        candidates
            .iter()
            .filter(|n| !n.is_empty())
            .find_map(|n| self.perspective_preset(n).filter(|p| p.same(def)))
            .map(|p| p.name)
            .unwrap_or_default()
    }
}

/// What `perspective.grid.get` reports for `g`.
pub(crate) fn grid_info(doc: &vectorcraft_doc::Document, g: &PerspectiveGrid) -> Value {
    let st = g.station();
    json!({
        "grid": g.definition_json(),
        "define": g.definition(),
        "station": {"x": st.x, "y": g.horizon, "distance": st.distance},
        "defined": PerspectiveGrid::from_doc(doc).is_some(),
    })
}

fn grid_get(s: &mut Session, _: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    Ok(grid_info(doc, &grid_of(doc)))
}

fn grid_define(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.grid.define";
    let old = grid_of(&s.doc()?.doc);
    let def: GridDefinition = old.definition().merged(p).map_err(|e| bad(C, e))?;
    let mut g = old.with_definition(&def).map_err(|e| bad(C, e))?;
    // The grid is the preset it was given or came from while it has that preset's fields.
    g.name = s.preset_named(&g.definition(), &[&def.name, &old.name]);
    if g != old {
        s.edit("Define Perspective Grid", |d, _| {
            store_grid(d, &g);
            Ok(())
        })?;
    }
    Ok(json!(g.definition()))
}

/// `perspective.grid.preset`: reset the grid to a preset fitted to the first artboard (shown;
/// the attached objects stay attached).
pub(crate) fn grid_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.grid.preset";
    let name = match (str_param(p, "name"), p.get("kind").and_then(Value::as_u64)) {
        (Some(n), _) => n.to_string(),
        (None, Some(k @ 1..=3)) => PerspectiveGrid::normal(k as u8, LETTER).name,
        _ => return Err(bad(C, "kind must be 1, 2 or 3, or give a preset `name` (see perspective.presets.list)")),
    };
    let old = grid_of(&s.doc()?.doc);
    let ab = s.doc()?.doc.artboards.first().map(|a| a.rect).ok_or_else(|| EngineError::Other("no artboard".into()))?;
    let g = s.perspective_preset_grid(&name, ab).ok_or_else(|| bad(C, format!("no perspective grid preset named `{name}`")))?;
    // The objects stay attached and the view options stay as they were.
    let g = PerspectiveGrid { attached: old.attached, locked: old.locked, lock_station: old.lock_station, snap: old.snap, rulers: old.rulers, ..g };
    s.edit("Perspective Grid Preset", |d, _| {
        store_grid(d, &g);
        Ok(())
    })?;
    Ok(g.definition_json())
}

fn presets_list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows: Vec<Value> = s
        .perspective_presets()
        .into_iter()
        .map(|d| {
            let mut v = json!(d);
            v["builtIn"] = json!(is_builtin(&d.name));
            v
        })
        .collect();
    Ok(json!({ "presets": rows }))
}

fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.presets.save";
    let name = match str_param(p, "name").map(str::trim) {
        Some("") => return Err(bad(C, "`name` is empty")),
        Some(n) => n.to_string(),
        None => s.new_perspective_preset_name(),
    };
    if is_builtin(&name) {
        return Err(bad(C, format!("`{name}` is a built-in preset and can't change: save it under another name")));
    }
    let at = s.persp_saved_index(&name);
    let saved = at.and_then(|i| s.prefs.perspective_presets.get(i)).cloned();
    // A saved preset changes from its own fields unless `preset` says where to start.
    let base = match (str_param(p, "preset"), &saved) {
        (Some(b), _) => {
            s.perspective_preset(b).ok_or_else(|| bad(C, format!("no perspective grid preset named `{b}` (see perspective.presets.list)")))?
        }
        (None, Some(old)) => old.clone(),
        (None, None) => s.active().map(|d| grid_of(&d.doc).definition()).unwrap_or_default(),
    };
    let mut def = base.merged(p).map_err(|e| bad(C, e))?;
    def.name = match str_param(p, "newName").map(str::trim) {
        Some("") => return Err(bad(C, "`newName` is empty")),
        Some(n) if s.persp_preset_taken(n, at) => return Err(bad(C, format!("a preset named `{n}` exists"))),
        Some(n) => n.to_string(),
        None => saved.map_or(name, |old| old.name),
    };
    let r = json!({ "name": def.name, "created": at.is_none(), "definition": def });
    match at.and_then(|i| s.prefs.perspective_presets.get_mut(i)) {
        Some(slot) => *slot = def,
        None => s.prefs.perspective_presets.push(def),
    }
    Ok(r)
}

fn presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.presets.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    if is_builtin(name) {
        return Err(bad(C, format!("`{name}` is a built-in preset and stays")));
    }
    let i = s.persp_saved_index(name).ok_or_else(|| bad(C, format!("no saved preset named `{name}`")))?;
    Ok(json!({ "deleted": s.prefs.perspective_presets.remove(i).name }))
}

fn presets_export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.presets.export";
    let presets = match p.get("names").and_then(Value::as_array) {
        Some(names) => {
            let find = |n: &Value| {
                let n = n.as_str().unwrap_or_default();
                s.perspective_preset(n).ok_or_else(|| bad(C, format!("no perspective grid preset named `{n}`")))
            };
            names.iter().map(find).collect::<Result<Vec<_>>>()?
        }
        None => s.prefs.perspective_presets.clone(),
    };
    if presets.is_empty() {
        return Err(bad(C, "no saved presets to export (name built-in ones in `names`)"));
    }
    let count = presets.len();
    let presets = presets.iter().map(|d| json!(d)).collect();
    let text = serde_json::to_string_pretty(&PresetFile { format: PRESET_FORMAT.into(), presets }).map_err(|e| bad(C, e.to_string()))?;
    match str_param(p, "path") {
        Some(path) => {
            super::fileio::write_file(path, text.as_bytes())?;
            Ok(json!({"path": path, "count": count}))
        }
        None => Ok(json!({"data": text, "count": count})),
    }
}

fn presets_import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.presets.import";
    let bytes = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => super::fileio::read_file(path)?,
        (None, Some(text), _) => text.as_bytes().to_vec(),
        (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "bad dataBase64"))?,
        _ => return Err(bad(C, "give `path`, `data` or `dataBase64`")),
    };
    let file: PresetFile = serde_json::from_slice(&bytes).map_err(|e| bad(C, format!("not a perspective grid presets file: {e}")))?;
    if file.format != PRESET_FORMAT {
        return Err(bad(C, format!("not a perspective grid presets file (format `{}`)", file.format)));
    }
    if file.presets.len() > MAX_IMPORT {
        return Err(bad(C, format!("more than {MAX_IMPORT} presets")));
    }
    // Every preset is checked as Define Grid checks its fields before any is added.
    let base = GridDefinition::default();
    let check = |v: &Value| base.merged(v).map_err(|e| bad(C, format!("`{}`: {e}", v["name"].as_str().unwrap_or("?"))));
    let presets = file.presets.iter().map(check).collect::<Result<Vec<_>>>()?;
    let replace = switch_or(p, "replace", false, C)?;
    let mut imported = vec![];
    for mut preset in presets {
        // A built-in view comes in as a saved copy ("2P-Normal View"…): the built-in one stays.
        let name = preset.name.trim().trim_start_matches('[').trim_end_matches(']').trim();
        let base = Some(name).filter(|n| !n.is_empty()).unwrap_or("Perspective Preset").to_string();
        match s.persp_saved_index(&base).filter(|_| replace) {
            Some(i) => {
                if let Some(slot) = s.prefs.perspective_presets.get_mut(i) {
                    preset.name = slot.name.clone();
                    imported.push(preset.name.clone());
                    *slot = preset;
                }
            }
            None => {
                preset.name = unique_name(&base, |n| s.persp_preset_taken(n, None));
                imported.push(preset.name.clone());
                s.prefs.perspective_presets.push(preset);
            }
        }
    }
    Ok(json!({ "imported": imported }))
}

/// Toggle (or set, with `on`) a view option of the grid for the command `cmd`.
fn view_option(s: &mut Session, p: &Value, cmd: &str, field: fn(&mut PerspectiveGrid) -> &mut bool) -> Result<Value> {
    // A missing or null `on` toggles the option.
    let want = match p.get("on") {
        None | Some(Value::Null) => None,
        Some(_) => Some(switch_or(p, "on", false, cmd)?),
    };
    let mut g = silent(s, |g| {
        let f = field(g);
        *f = want.unwrap_or(!*f);
    })?;
    Ok(json!({ "on": *field(&mut g) }))
}

fn lock(s: &mut Session, p: &Value) -> Result<Value> {
    view_option(s, p, "perspective.grid.lock", |g| &mut g.locked)
}

fn lock_station(s: &mut Session, p: &Value) -> Result<Value> {
    view_option(s, p, "perspective.grid.lockStation", |g| &mut g.lock_station)
}

fn snap(s: &mut Session, p: &Value) -> Result<Value> {
    view_option(s, p, "perspective.grid.snap", |g| &mut g.snap)
}

fn rulers(s: &mut Session, p: &Value) -> Result<Value> {
    view_option(s, p, "perspective.grid.rulers", |g| &mut g.rulers)
}

/// Snap to Grid for a move of `ids` from page point `from` to `to` (unless `p` says `snap: false`):
/// the plane and depth of the first one and the plane-space correction that lands the edge of the
/// joint bounds of the objects there nearer a gridline on it.
pub(crate) fn snap_fix(doc: &Document, ids: &[NodeId], p: &Value, (from, to): (Point, Point)) -> Option<(Plane, f64, Vec2)> {
    let g = grid_of(doc);
    if !bool_or(p, "snap", g.snap) {
        return None;
    }
    let (plane, depth, b) = g.plane_bounds(doc, ids)?;
    let dv = g.plane_delta(plane, depth, from, to)?;
    Some((plane, depth, g.snap_offset(b, dv) - dv))
}

fn widget_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "perspective.widget.options";
    let mut o = s.prefs.perspective_widget;
    if let Some(v) = p.get("show") {
        o.show = v.as_bool().ok_or_else(|| bad(C, "show must be true or false"))?;
    }
    if let Some(v) = p.get("position") {
        o.position =
            v.as_str().and_then(WidgetCorner::parse).ok_or_else(|| bad(C, "position must be topLeft, topRight, bottomLeft or bottomRight"))?;
    }
    s.prefs.perspective_widget = o;
    Ok(json!(o))
}

impl Session {
    /// The Plane Switching Widget's part of a pointer event: a press on it picks the plane (its
    /// drag and release go nowhere); None when the event isn't the widget's.
    pub(crate) fn plane_widget_pointer(&mut self, ev: &PointerEvent, view: crate::ViewInfo) -> Option<Result<Vec<crate::UiRequest>>> {
        match ev.kind {
            PointerKind::Down => {
                let w = self.prefs.perspective_widget;
                let st = self.active().filter(|_| w.show)?;
                let g = PerspectiveGrid::current(&st.doc);
                if !g.visible {
                    return None;
                }
                let place = WidgetPlace { screen: view.screen.as_ref(), corner: w.position };
                let plane = g.widget_hit_at(&st.doc, 1.0 / view.zoom.max(1e-9), place, ev.pos)?;
                self.plane_widget_press = true;
                Some(self.execute("perspective.plane.set", &json!({ "plane": plane.id() })).map(|_| vec![]))
            }
            PointerKind::Drag if self.plane_widget_press => Some(Ok(vec![])),
            PointerKind::Up if std::mem::take(&mut self.plane_widget_press) => Some(Ok(vec![])),
            _ => None,
        }
    }

    /// The plane digit key `key` picks while the grid is shown: 1 left, 2 horizontal (ground),
    /// 3 right, 4 none.
    pub(crate) fn plane_key(&self, key: ToolKey) -> Option<Plane> {
        let plane = match key {
            ToolKey::Digit(1) => Plane::Left,
            ToolKey::Digit(2) => Plane::Ground,
            ToolKey::Digit(3) => Plane::Right,
            ToolKey::Digit(4) => Plane::None,
            _ => return None,
        };
        PerspectiveGrid::current(&self.active()?.doc).visible.then_some(plane)
    }
}
