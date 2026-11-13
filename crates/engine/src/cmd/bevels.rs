//! Custom bevels for Extrude & Bevel and Extrude & Bevel (Classic). The bevels file is a native
//! document kept next to the preferences ([`Session::attach_bevels_file`]). Each of its symbols
//! whose art is one open path is one bevel, listed by the symbol's name after the built-in shapes
//! in the Bevel Shape menus. The path runs from its first anchor, at the cap's edge, to its last,
//! at the side wall: across the page is across the bevel's width and down the page is along its
//! height, whichever way the path is drawn, and points past its ends are held to them. Curves are
//! cut into straight lines, at most [`BevelProfile::MAX_POINTS`] points in all. Saving the file
//! reloads the bevels. An effect that uses a custom bevel carries a copy of its points
//! (`bevelProfile`), so its document draws it anywhere.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Symbol};
use vectorcraft_geom::{Affine, PathData, PathEl, Point, SubPath};
use vectorcraft_render::effects::{BEVEL_NAME, BEVEL_PROFILE, BevelProfile};

use super::arrowheads::{read, same_file};
use super::{CommandSpec, Result, Session, always, bad, cmd};

/// The bevels file's name, in the folder of the preferences.
pub const FILE_NAME: &str = "Bevels.vectorcraft";
/// The most bevels read from the file; symbols past it are skipped.
pub const MAX_BEVELS: usize = 256;
/// The most anchors a bevel's path may have; a symbol past it is skipped.
pub const MAX_ANCHORS: usize = 4096;
/// The most points a bevel's path is cut into before it is simplified; a symbol past it is skipped.
pub const MAX_LINE_POINTS: usize = 8192;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "threeD.bevels.list",
            "Bevels",
            [],
            None,
            "{} → {bevels: [{name (effect.apply and effect.setParams take Extrude & Bevel's and Extrude & Bevel (Classic)'s `bevel` as {custom: name}), points: [[x, y]…] (shares of the bevel's width and height from the cap's edge [0, 0] to the side wall [1, 1])}], file: the bevels file's path or null (no file here, as on the web), skipped: [{name, reason}] symbols of the file that are not bevels, error: why the file could not be read, or null} the custom bevels, listed after the built-in shapes in the Bevel Shape menus",
            always,
            list
        ),
        cmd!(
            "threeD.bevels.edit",
            "Edit Bevels",
            [],
            None,
            "{} open the bevels file (a native document next to the preferences; created first with one example bevel when it is missing). Each symbol in it whose art is one open path is one custom bevel named after the symbol: the path runs from its first anchor at the cap's edge to its last at the side wall, across the page across the bevel's width and down the page along its height. Saving the file reloads the bevels → {path, created: whether the file was made now}",
            has_file,
            edit
        ),
        cmd!(
            "threeD.bevels.reload",
            "Reload Bevels",
            [],
            None,
            "{} read the bevels file again (a save of it from VectorCraft does this already) → {count, skipped: [{name, reason}]}",
            has_file,
            reload
        ),
    ]
}

/// A custom bevel: its name and its points, (share of the width, share of the height).
#[derive(Clone, Debug, PartialEq)]
pub struct CustomBevel {
    pub name: String,
    pub points: Vec<(f64, f64)>,
}

impl CustomBevel {
    /// The points as `bevelProfile` holds them.
    pub fn profile_json(&self) -> Value {
        json!(self.points.iter().map(|&(x, y)| [x, y]).collect::<Vec<_>>())
    }
}

/// The custom bevels read from the bevels file.
#[derive(Default)]
pub struct Bevels {
    path: Option<String>,
    bevels: Vec<Arc<CustomBevel>>,
    skipped: Vec<(String, String)>,
    error: Option<String>,
}

impl Bevels {
    /// The bevels file's path; `None` without one (the web, runs without preferences).
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
    /// The bevels, in the order of the file's symbols.
    pub fn bevels(&self) -> &[Arc<CustomBevel>] {
        &self.bevels
    }
    /// Bevel `name`.
    pub fn get(&self, name: &str) -> Option<Arc<CustomBevel>> {
        self.bevels.iter().find(|b| b.name == name).cloned()
    }
    fn set(&mut self, read: Read) {
        (self.bevels, self.skipped) = (read.bevels.into_iter().map(Arc::new).collect(), read.skipped);
        self.error = None;
    }
}

/// The bevels a document holds, and the symbols that are not bevels with the reason.
#[derive(Debug, Default)]
pub struct Read {
    pub bevels: Vec<CustomBevel>,
    pub skipped: Vec<(String, String)>,
}

/// The bevels of document `doc`: one per symbol, up to [`MAX_BEVELS`] (see the module docs).
pub fn bevels_of(doc: &Document) -> Read {
    let mut out = Read::default();
    for (i, sym) in doc.symbols.iter().enumerate() {
        if i >= MAX_BEVELS {
            out.skipped.push((sym.name.clone(), format!("only the first {MAX_BEVELS} symbols are read")));
            continue;
        }
        if out.bevels.iter().any(|b| b.name == sym.name) {
            out.skipped.push((sym.name.clone(), "another symbol has this name".into()));
            continue;
        }
        match points_of(&sym.art) {
            Ok(points) => out.bevels.push(CustomBevel { name: sym.name.clone(), points }),
            Err(why) => out.skipped.push((sym.name.clone(), why.into())),
        }
    }
    out
}

/// The one open path of art `n`: `n` itself, or the one member of groups that hold one member each.
fn open_path(n: &Node) -> Option<&PathData> {
    match &n.kind {
        NodeKind::Path { path, guide: false, .. } if path.subpaths.len() == 1 && !path.subpaths[0].closed => Some(path),
        NodeKind::Group { children, clip: false } if children.len() == 1 => open_path(&children[0]),
        _ => None,
    }
}

/// The profile points of a bevel symbol's art `n` (see the module docs).
fn points_of(n: &Node) -> std::result::Result<Vec<(f64, f64)>, &'static str> {
    let path = open_path(n).ok_or("its art is not one open path")?;
    if path.anchor_count() > MAX_ANCHORS {
        return Err("its path has too many anchors for a bevel");
    }
    let mut line: Vec<Point> = vec![];
    let mut too_many = false;
    let tolerance = path.bounds().map_or(1.0, |b| b.width().hypot(b.height()) / 2000.0).max(1e-9);
    vectorcraft_geom::flatten(path.to_bezpath().elements().iter().copied(), tolerance, |el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) if line.len() < MAX_LINE_POINTS => line.push(p),
        PathEl::MoveTo(_) | PathEl::LineTo(_) => too_many = true,
        _ => {}
    });
    if too_many {
        return Err("its path is too detailed for a bevel");
    }
    let (Some(&first), Some(&last)) = (line.first(), line.last()) else { return Err("its path has no length") };
    let (dx, dy) = (last.x - first.x, last.y - first.y);
    let size = path.bounds().map_or(0.0, |b| b.width().hypot(b.height()));
    if !(dx.abs() > size * 1e-6 && dy.abs() > size * 1e-6 && size.is_finite()) {
        return Err("its ends are not apart both across and down");
    }
    let mut pts: Vec<(f64, f64)> = line.iter().map(|p| (((p.x - first.x) / dx).clamp(0.0, 1.0), ((p.y - first.y) / dy).clamp(0.0, 1.0))).collect();
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
    if pts.len() < 2 {
        return Err("its ends are not apart both across and down");
    }
    // The ends map to the corners exactly.
    let last = pts.len() - 1;
    (pts[0], pts[last]) = ((0.0, 0.0), (1.0, 1.0));
    let pts = simplified(&pts);
    BevelProfile::new(&pts).ok_or("its path is too detailed for a bevel")?;
    Ok(pts)
}

/// `pts` with the fewest points the Ramer-Douglas-Peucker rule keeps at a tolerance (from a
/// thousandth of the bevel, doubled until they fit) that leaves at most
/// [`BevelProfile::MAX_POINTS`]. The first and last points are always kept.
fn simplified(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut eps = 1e-3;
    loop {
        let out = rdp(pts, eps);
        if out.len() <= BevelProfile::MAX_POINTS || eps > 2.0 {
            return out;
        }
        eps *= 2.0;
    }
}

/// The points of `pts` Ramer-Douglas-Peucker keeps at tolerance `eps`.
fn rdp(pts: &[(f64, f64)], eps: f64) -> Vec<(f64, f64)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (p, q) = (pts[a], pts[b]);
        let (vx, vy) = (q.0 - p.0, q.1 - p.1);
        let len = vx.hypot(vy);
        let far = (a + 1..b)
            .map(|i| {
                let (wx, wy) = (pts[i].0 - p.0, pts[i].1 - p.1);
                let d = if len > 0.0 { (vx * wy - vy * wx).abs() / len } else { wx.hypot(wy) };
                (i, d)
            })
            .fold(None, |best: Option<(usize, f64)>, (i, d)| if best.is_none_or(|(_, bd)| d > bd) { Some((i, d)) } else { best });
        if let Some((i, d)) = far
            && d > eps
        {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    pts.iter().zip(keep).filter_map(|(p, k)| k.then_some(*p)).collect()
}

/// The document a missing bevels file starts as: one example bevel, a chamfer with a notch.
pub fn starter() -> Document {
    let mut doc = Document::new(612.0, 792.0);
    doc.title = "Bevels".into();
    let name = "Example Bevel";
    let pts = [Point::new(0.0, 0.0), Point::new(10.0, 40.0), Point::new(55.0, 30.0), Point::new(100.0, 100.0)];
    let art =
        Node::path(NodeId(0), PathData::single(SubPath::polyline(&pts, false)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0));
    let art = doc.reid(&art);
    doc.symbols.push(Symbol { name: name.into(), art: Arc::new(art), dynamic: false });
    let id = doc.alloc_id();
    let instance = Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::translate((256.0, 346.0)) });
    let layer = doc.layers.first().map(|l| l.id);
    let _ = doc.insert(layer, 0, instance);
    doc
}

impl Session {
    /// Keep the custom bevels in the file at `path` and read the bevels it holds (none while it is
    /// missing; an unreadable file is reported by `threeD.bevels.list`).
    pub fn attach_bevels_file(&mut self, path: String) {
        self.bevels.path = Some(path);
        self.reload_bevels();
    }

    /// Read the bevels file again.
    fn reload_bevels(&mut self) {
        let Some(path) = self.bevels.path.clone() else { return };
        match read(&path) {
            Ok(Some(doc)) => self.bevels.set(bevels_of(&doc)),
            Ok(None) => self.bevels.set(Read::default()),
            Err(e) => {
                self.bevels.set(Read::default());
                self.bevels.error = Some(e);
            }
        }
    }

    /// A document was saved to `path`: when that is the bevels file, its bevels are read back from
    /// what was written.
    pub(crate) fn bevels_saved(&mut self, path: &str) {
        if self.bevels.path.as_deref().is_some_and(|b| same_file(b, path)) {
            self.reload_bevels();
        }
    }
}

/// `params` of an effect with `bevel` given as `{custom: name}`: the bevel of the bevels file named
/// so, stored as `bevel` `"custom"` with its `bevelName` and `bevelProfile`. Other values are left
/// for the effect to check.
pub(crate) fn resolve_custom(s: &Session, params: &mut Value, cmd: &str) -> Result<()> {
    let Some(Value::Object(o)) = params.get("bevel") else { return Ok(()) };
    let name = match (o.len(), o.get("custom")) {
        (1, Some(Value::String(name))) => name.clone(),
        _ => return Err(bad(cmd, "bevel: give a shape id, or {custom: name} for a bevel of the bevels file (threeD.bevels.list)")),
    };
    let bevel = s.bevels.get(&name).ok_or_else(|| bad(cmd, format!("no custom bevel {name} (see threeD.bevels.list)")))?;
    params["bevel"] = json!("custom");
    params[BEVEL_NAME] = json!(bevel.name);
    params[BEVEL_PROFILE] = bevel.profile_json();
    Ok(())
}

fn has_file(s: &Session) -> std::result::Result<(), String> {
    if s.bevels.path.is_some() { Ok(()) } else { Err("no bevels file here".into()) }
}

fn skipped(s: &Session) -> Vec<Value> {
    s.bevels.skipped.iter().map(|(name, reason)| json!({"name": name, "reason": reason})).collect()
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let bevels: Vec<Value> = s.bevels.bevels.iter().map(|b| json!({"name": b.name, "points": b.profile_json()})).collect();
    Ok(json!({"bevels": bevels, "file": s.bevels.path, "skipped": skipped(s), "error": s.bevels.error}))
}

fn reload(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "threeD.bevels.reload";
    s.reload_bevels();
    if let Some(e) = &s.bevels.error {
        return Err(bad(C, e.clone()));
    }
    Ok(json!({"count": s.bevels.bevels.len(), "skipped": skipped(s)}))
}

fn edit(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "threeD.bevels.edit";
    let path = s.bevels.path.clone().ok_or_else(|| bad(C, "no bevels file here"))?;
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| bad(C, format!("{}: {e}", dir.display())))?;
    }
    // Made only when no file is there, also when another run makes one at the same moment.
    let bytes = vectorcraft_format::save_file(&starter());
    let created = match crate::file_access::unconfined(|| super::fileio::write_new_file(&path, &bytes)) {
        Ok(()) => true,
        Err(_) if std::path::Path::new(&path).exists() => false,
        Err(e) => return Err(bad(C, e.to_string())),
    };
    if created {
        s.reload_bevels();
    }
    // Already open: that document comes to the front.
    let open = s.documents().iter().position(|st| st.path.as_deref().is_some_and(|p| same_file(p, &path)));
    match open {
        Some(i) => {
            s.set_active(i);
        }
        None => {
            crate::file_access::unconfined(|| s.execute("document.open", &json!({ "path": path })))?;
        }
    }
    Ok(json!({"path": path, "created": created}))
}
