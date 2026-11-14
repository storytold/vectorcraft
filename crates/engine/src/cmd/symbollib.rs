//! Symbol libraries (Window → Symbol Libraries, the Symbols panel's library button): read-only sets
//! of symbols the library panel shows. A library is a document holding the symbols with the
//! resources they use (nested symbols, patterns, swatches, brushes, graphic styles, images) and
//! their Symbol Options; saved, it is a native `.vectorcraft` file, which File › Open also opens.
//! The built-in libraries are generated here ([`BUILTIN`]); User Defined ones are the `.vectorcraft`
//! files in the user library folder ([`Libraries`]), and Other Library… loads the symbols of any
//! document VectorCraft opens. `symbol.addFromLibrary` copies symbols into the document;
//! `symbol.saveLibrary` writes the document's.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Symbol, SymbolNineSlice};
use vectorcraft_geom::{Affine, Anchor, AnchorKind, PathData, Point, Rect, SubPath, shapes};

use super::brushsym::{CURRENT_SYMBOL, natural_size, normalize, place_xf, set_natural_size};
use super::clipboard::Clipboard;
use super::clipboard::resources::SwatchChoices;
use super::swatch::str_list;
use super::swatchlib::{FileData, LibraryFile, LibraryInfo, concealed_library_id, concealed_library_ids, shown_id, shown_library_id, stem};
use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, has_doc, number_or, str_param};

/// The extension of saved symbol libraries: native documents.
pub const SYMBOLS_EXT: &str = vectorcraft_format::EXTENSION;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "symbol.libraries",
            "Symbol Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, category: \"builtIn\"|\"user\" (User Defined: the .vectorcraft files in the user library folder, rescanned now)|\"loaded\" (symbol.loadLibrary), count, path?}], userFolder} every symbol library the library panel opens, in menu order",
            always,
            list
        ),
        cmd!(
            query "symbol.library",
            "Symbol Library",
            [],
            None,
            "{library: id or name} → {id, name, category, symbols: [{name, size: [w, h], registration: 0..8, exportType}] (as symbol.list)}",
            always,
            get
        ),
        cmd!(
            "symbol.addFromLibrary",
            "Add to Symbols",
            ["Window", "Symbol Libraries"],
            None,
            "{library: id or name, name? | names?: [symbol names in the library] (default: all of it), place?: false (also place an instance of the first one, its registration point at x, y: default the first artboard's centre), x?, y?, swatchConflict? (as edit.paste)} copy library symbols into the document's Symbols as one undo step (with the place), with their Symbol Options and the resources they use (the symbols they nest, patterns, swatches, brushes, graphic styles, images). A symbol the document already has (same name and art) isn't added again; a name another symbol has gets a number (\"Star 2\"). The first one becomes the current symbol → {library, names: [each symbol's name in the document], added: [names added], existing: [names already there], placed?: id}",
            has_doc,
            add
        ),
        cmd!(
            "symbol.saveLibrary",
            "Save Symbol Library…",
            ["Window", "Symbols"],
            None,
            "{path?, names?: [symbol names] (default: all), name?: library name (default: the document's), user?: false (save into the user library folder, listed under User Defined)} save the document's symbols, with their Symbol Options and the resources they use, as a symbol library: a .vectorcraft document holding only them → {path, count, library?: id when saved to the user folder}; without path or user → {dataBase64: the file's bytes, count}",
            has_doc,
            save
        ),
        cmd!(
            "symbol.loadLibrary",
            "Other Library…",
            ["Window", "Symbol Libraries"],
            None,
            "{path? | data?: file text | dataBase64?, name?: file name (default: the path's)} load the symbols of any document VectorCraft opens (see document.formats), a saved symbol library included, for the library panel (Window → Symbol Libraries lists it until the app quits; a file of the user library folder, or a file with the same bytes as one there, opens as that User Defined library) → {library: id, name, count}",
            always,
            load
        ),
    ]
}

/// A symbol library: its name and a document holding its symbols (in panel order) with the
/// resources they use and their Symbol Options; the document has no art of its own.
#[derive(Clone, Debug)]
pub struct SymbolLibrary {
    pub name: String,
    pub doc: Document,
}

impl Default for SymbolLibrary {
    fn default() -> Self {
        Self { name: String::new(), doc: Document::new(100.0, 100.0) }
    }
}

impl SymbolLibrary {
    pub fn len(&self) -> usize {
        self.doc.symbols.len()
    }
    pub fn is_empty(&self) -> bool {
        self.doc.symbols.is_empty()
    }
    pub fn symbol(&self, name: &str) -> Option<&Symbol> {
        self.doc.symbols.iter().find(|s| s.name == name)
    }

    /// Document `src`'s symbols `names` (empty: all of them, in panel order) as a library named
    /// `name`: the symbols, the symbols they nest and every other resource they use, with their
    /// Symbol Options.
    pub fn from_document(src: &Document, names: &[String], name: String) -> std::result::Result<Self, String> {
        // Each name's first place in the panel, looked up rather than scanned for: scanning the
        // symbols for each one made a library of N symbols cost N² comparisons.
        let mut order: HashMap<&str, usize> = HashMap::new();
        for (i, s) in src.symbols.iter().enumerate() {
            order.entry(s.name.as_str()).or_insert(i);
        }
        if let Some(n) = names.iter().find(|n| !order.contains_key(n.as_str())) {
            return Err(format!("no symbol `{n}`"));
        }
        let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
        let picked = src.symbols.iter().filter(|s| names.is_empty() || wanted.contains(s.name.as_str()));
        let clip = Clipboard::gather(src, picked.map(|s| instance(&s.name)).collect());
        let mut doc = clip.to_document();
        for l in &mut doc.layers {
            if let Some(ch) = Arc::make_mut(l).children_mut() {
                ch.clear();
            }
        }
        // The panel order of the document they came from.
        doc.symbols.sort_by_key(|s| order.get(s.name.as_str()).copied());
        for s in &doc.symbols.clone() {
            copy_options(src, &s.name, &mut doc, &s.name);
        }
        doc.title = name.clone();
        Ok(Self { name, doc })
    }
}

impl LibraryFile for SymbolLibrary {
    const EXTS: &[&str] = &[SYMBOLS_EXT];
    fn read(bytes: &[u8], stem: &str) -> std::result::Result<Self, String> {
        let doc = vectorcraft_format::load(bytes).map_err(|e| e.to_string())?;
        SymbolLibrary::from_document(&doc, &[], stem.to_string())
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// User Defined and loaded symbol libraries.
pub type Libraries = super::swatchlib::Libraries<SymbolLibrary>;

/// A library generated in code.
pub struct BuiltinSymbolLibrary {
    pub id: &'static str,
    pub name: &'static str,
    make: fn() -> SymbolLibrary,
}

/// The built-in symbol libraries, in menu order: original art generated in code.
pub const BUILTIN: &[BuiltinSymbolLibrary] = &[
    BuiltinSymbolLibrary { id: "basic-shapes", name: "Basic Shapes", make: basic_shapes },
    BuiltinSymbolLibrary { id: "ui-frames", name: "UI Frames", make: ui_frames },
    BuiltinSymbolLibrary { id: "map-markers", name: "Map Markers", make: map_markers },
    BuiltinSymbolLibrary { id: "inventory-icons", name: "Inventory Icons", make: inventory_icons },
    BuiltinSymbolLibrary { id: "skill-icons", name: "Skill Icons", make: skill_icons },
    BuiltinSymbolLibrary { id: "weather-icons", name: "Weather Icons", make: weather_icons },
];

/// Built-in library `id`, made once.
fn builtin_library(id: &str) -> Option<Arc<SymbolLibrary>> {
    static MADE: LazyLock<Vec<Arc<SymbolLibrary>>> = LazyLock::new(|| BUILTIN.iter().map(|b| Arc::new((b.make)())).collect());
    BUILTIN.iter().position(|b| b.id == id).and_then(|i| MADE.get(i).cloned())
}

/// Basic Shapes: plain filled and outlined shapes, 40 pt (the arrow 48 × 32 pt).
fn basic_shapes() -> SymbolLibrary {
    let square = Rect::new(-20.0, -20.0, 20.0, 20.0);
    let c = Point::ZERO;
    let arrow = [(-24.0, -6.0), (4.0, -6.0), (4.0, -16.0), (24.0, 0.0), (4.0, 16.0), (4.0, 6.0), (-24.0, 6.0)];
    let arrow: Vec<Point> = arrow.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let shapes = [
        ("Circle", shapes::ellipse(square), "#2f80c9"),
        ("Square", shapes::rectangle(square), "#d9534f"),
        ("Rounded Square", shapes::rounded_rectangle(square, 8.0), "#f0ad4e"),
        ("Triangle", shapes::polygon(c, 22.0, 3, 0.0), "#5cb85c"),
        ("Diamond", shapes::polygon(c, 20.0, 4, 0.0), "#8e5bc6"),
        ("Hexagon", shapes::polygon(c, 20.0, 6, 0.0), "#1fa39b"),
        ("Star", shapes::star(c, 21.0, 9.0, 5, 0.0), "#f2c233"),
        ("Burst", shapes::star(c, 21.0, 15.0, 12, 0.0), "#e8743b"),
        ("Arrow", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&arrow, true)), "#4a5a6a"),
    ];
    let mut doc = Document::new(100.0, 100.0);
    for (name, path, hex) in shapes {
        let fill = Color::from_hex(hex).unwrap_or(Color::BLACK);
        // The outline: the fill, darkened.
        let edge = fill.lerp(&Color::BLACK, 0.4);
        let id = doc.alloc_id();
        let art = Node::path(id, path, Appearance::basic(Paint::solid(fill), Paint::solid(edge), 1.0));
        let Ok((art, size, _)) = normalize(art) else { continue };
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
        set_natural_size(&mut doc, name, Some(size));
    }
    doc.title = "Basic Shapes".into();
    SymbolLibrary { name: "Basic Shapes".into(), doc }
}

/// UI Frames: panel, button and bar HUD chrome with default 9-slice guides (original art).
fn ui_frames() -> SymbolLibrary {
    // Heights keep default 9-slice (0.25) past each corner radius so a vertical resize
    // stretches only the flat middle, not the curves.
    let frames = [
        ("Panel", Rect::new(-48.0, -36.0, 48.0, 36.0), 10.0, "#2b3340", "#8aa0b8"),
        ("Button", Rect::new(-36.0, -18.0, 36.0, 18.0), 8.0, "#3a6ea5", "#d7e6f5"),
        ("Bar", Rect::new(-56.0, -24.0, 56.0, 24.0), 10.0, "#1f6b4a", "#b7e0c9"),
    ];
    let mut doc = Document::new(100.0, 100.0);
    for (name, rect, radius, fill_hex, edge_hex) in frames {
        let fill = Color::from_hex(fill_hex).unwrap_or(Color::BLACK);
        let edge = Color::from_hex(edge_hex).unwrap_or(Color::BLACK);
        let id = doc.alloc_id();
        let art = Node::path(id, shapes::rounded_rectangle(rect, radius), Appearance::basic(Paint::solid(fill), Paint::solid(edge), 1.5));
        let Ok((art, size, _)) = normalize(art) else { continue };
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
        set_natural_size(&mut doc, name, Some(size));
        doc.set_symbol_nine_slice(name, Some(SymbolNineSlice::default()));
    }
    doc.title = "UI Frames".into();
    SymbolLibrary { name: "UI Frames".into(), doc }
}

/// Map Markers: pin, flag, crosshair and checkpoint for map/HUD art (original, no 9-slice).
fn map_markers() -> SymbolLibrary {
    let c = Point::ZERO;
    let pin = [(0.0, 22.0), (-9.0, 4.0), (-12.0, -6.0), (-8.0, -16.0), (0.0, -20.0), (8.0, -16.0), (12.0, -6.0), (9.0, 4.0)];
    let flag = [(-16.0, 22.0), (-12.0, 22.0), (-12.0, -8.0), (18.0, -14.0), (-12.0, -20.0), (-12.0, -22.0), (-16.0, -22.0)];
    let pin: Vec<Point> = pin.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let flag: Vec<Point> = flag.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let markers = [
        ("Pin", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pin, true)), "#e74c3c"),
        ("Flag", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&flag, true)), "#3498db"),
        ("Crosshair", shapes::star(c, 20.0, 5.0, 4, 0.0), "#2c3e50"),
        ("Checkpoint", shapes::polygon(c, 18.0, 4, 0.0), "#27ae60"),
    ];
    let mut doc = Document::new(100.0, 100.0);
    for (name, path, hex) in markers {
        let fill = Color::from_hex(hex).unwrap_or(Color::BLACK);
        let edge = fill.lerp(&Color::BLACK, 0.35);
        let id = doc.alloc_id();
        let art = Node::path(id, path, Appearance::basic(Paint::solid(fill), Paint::solid(edge), 1.25));
        let Ok((art, size, _)) = normalize(art) else { continue };
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
        set_natural_size(&mut doc, name, Some(size));
    }
    doc.title = "Map Markers".into();
    SymbolLibrary { name: "Map Markers".into(), doc }
}

/// Inventory Icons: sword, shield, potion, key and gem for loot/RPG art (original, no 9-slice).
fn inventory_icons() -> SymbolLibrary {
    let c = Point::ZERO;
    let sword = [
        (0.0, -22.0),
        (3.0, -10.0),
        (3.0, 6.0),
        (8.0, 8.0),
        (8.0, 12.0),
        (3.0, 12.0),
        (3.0, 20.0),
        (-3.0, 20.0),
        (-3.0, 12.0),
        (-8.0, 12.0),
        (-8.0, 8.0),
        (-3.0, 6.0),
        (-3.0, -10.0),
    ];
    let shield = [(0.0, -20.0), (16.0, -12.0), (14.0, 8.0), (0.0, 20.0), (-14.0, 8.0), (-16.0, -12.0)];
    let potion =
        [(-4.0, -20.0), (4.0, -20.0), (4.0, -12.0), (10.0, -6.0), (12.0, 6.0), (8.0, 18.0), (-8.0, 18.0), (-12.0, 6.0), (-10.0, -6.0), (-4.0, -12.0)];
    let key = [
        (-4.0, -18.0),
        (4.0, -18.0),
        (6.0, -14.0),
        (6.0, -8.0),
        (4.0, -6.0),
        (4.0, 16.0),
        (8.0, 16.0),
        (8.0, 20.0),
        (-2.0, 20.0),
        (-2.0, 12.0),
        (0.0, 12.0),
        (0.0, -6.0),
        (-4.0, -8.0),
        (-6.0, -14.0),
    ];
    let sword: Vec<Point> = sword.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let shield: Vec<Point> = shield.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let potion: Vec<Point> = potion.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let key: Vec<Point> = key.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let icons = [
        ("Sword", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&sword, true)), "#95a5a6"),
        ("Shield", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&shield, true)), "#3498db"),
        ("Potion", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&potion, true)), "#9b59b6"),
        ("Key", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&key, true)), "#f1c40f"),
        ("Gem", shapes::polygon(c, 16.0, 6, 0.0), "#1abc9c"),
    ];
    let mut doc = Document::new(100.0, 100.0);
    for (name, path, hex) in icons {
        let fill = Color::from_hex(hex).unwrap_or(Color::BLACK);
        let edge = fill.lerp(&Color::BLACK, 0.35);
        let id = doc.alloc_id();
        let art = Node::path(id, path, Appearance::basic(Paint::solid(fill), Paint::solid(edge), 1.25));
        let Ok((art, size, _)) = normalize(art) else { continue };
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
        set_natural_size(&mut doc, name, Some(size));
    }
    doc.title = "Inventory Icons".into();
    SymbolLibrary { name: "Inventory Icons".into(), doc }
}

/// Skill Icons: attack, defend, heal, buff and ult marks for RPG/UI art (original, no 9-slice).
fn skill_icons() -> SymbolLibrary {
    let c = Point::ZERO;
    let attack = [(-18.0, 14.0), (-6.0, 2.0), (-2.0, 6.0), (18.0, -14.0), (14.0, -18.0), (-6.0, -2.0), (-10.0, 2.0), (-18.0, 6.0)];
    let defend = [(0.0, -18.0), (14.0, -10.0), (12.0, 6.0), (0.0, 18.0), (-12.0, 6.0), (-14.0, -10.0)];
    let heal = [
        (-4.0, -18.0),
        (4.0, -18.0),
        (4.0, -4.0),
        (18.0, -4.0),
        (18.0, 4.0),
        (4.0, 4.0),
        (4.0, 18.0),
        (-4.0, 18.0),
        (-4.0, 4.0),
        (-18.0, 4.0),
        (-18.0, -4.0),
        (-4.0, -4.0),
    ];
    let buff = [(0.0, -18.0), (10.0, -2.0), (4.0, -2.0), (4.0, 18.0), (-4.0, 18.0), (-4.0, -2.0), (-10.0, -2.0)];
    let attack: Vec<Point> = attack.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let defend: Vec<Point> = defend.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let heal: Vec<Point> = heal.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let buff: Vec<Point> = buff.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let icons = [
        ("Attack", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&attack, true)), "#e74c3c"),
        ("Defend", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&defend, true)), "#3498db"),
        ("Heal", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&heal, true)), "#2ecc71"),
        ("Buff", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&buff, true)), "#f1c40f"),
        ("Ult", shapes::star(c, 18.0, 7.0, 8, 0.0), "#9b59b6"),
    ];
    let mut doc = Document::new(100.0, 100.0);
    for (name, path, hex) in icons {
        let fill = Color::from_hex(hex).unwrap_or(Color::BLACK);
        let edge = fill.lerp(&Color::BLACK, 0.35);
        let id = doc.alloc_id();
        let art = Node::path(id, path, Appearance::basic(Paint::solid(fill), Paint::solid(edge), 1.25));
        let Ok((art, size, _)) = normalize(art) else { continue };
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
        set_natural_size(&mut doc, name, Some(size));
    }
    doc.title = "Skill Icons".into();
    SymbolLibrary { name: "Skill Icons".into(), doc }
}

/// Weather Icons: sun, moon, cloud, rain, thunder and snow for weather/status art (original, no 9-slice).
fn weather_icons() -> SymbolLibrary {
    // 1. Sun: Central circular disk with radial rays.
    let mut sun_subpaths = Vec::new();
    sun_subpaths.extend(shapes::ellipse(Rect::new(-8.5, -8.5, 8.5, 8.5)).subpaths);
    for i in 0..8 {
        let angle = (i as f64) * std::f64::consts::PI / 4.0;
        let (sin, cos) = angle.sin_cos();
        let r_in = 11.5;
        let r_out = 18.5;
        let w = 1.5;
        let p1 = Point::new(r_in * cos - w * sin, r_in * sin + w * cos);
        let tip = Point::new(r_out * cos, r_out * sin);
        let p2 = Point::new(r_in * cos + w * sin, r_in * sin - w * cos);
        sun_subpaths.push(SubPath::polyline(&[p1, tip, p2], true));
    }
    let sun = PathData::new(sun_subpaths);

    // 2. Moon: Crescent moon shape.
    let moon_anchors = vec![
        Anchor { p: Point::new(3.0, -17.0), h_in: Point::new(-2.0, -10.0), h_out: Point::new(-8.0, -17.0), kind: AnchorKind::Corner },
        Anchor { p: Point::new(-16.0, 0.0), h_in: Point::new(-16.0, -9.0), h_out: Point::new(-16.0, 9.0), kind: AnchorKind::Smooth },
        Anchor { p: Point::new(5.0, 16.0), h_in: Point::new(-6.0, 16.0), h_out: Point::new(0.0, 8.0), kind: AnchorKind::Corner },
        Anchor { p: Point::new(-4.0, -1.0), h_in: Point::new(-4.0, 6.0), h_out: Point::new(-4.0, -8.0), kind: AnchorKind::Smooth },
    ];
    let moon = PathData::single(SubPath { anchors: moon_anchors, closed: true });

    // Helper for cloud contour
    let cloud_subpath = |offset_y: f64, scale: f64| -> SubPath {
        let mut pts = Vec::new();
        let base_y = 8.0 * scale + offset_y;
        pts.push(Point::new(9.0 * scale, base_y));
        pts.push(Point::new(-9.0 * scale, base_y));

        let c1 = Point::new(-9.0 * scale, 2.0 * scale + offset_y);
        let r1 = 6.0 * scale;
        let steps_lobe = 8;
        for i in 1..=steps_lobe {
            let a = std::f64::consts::FRAC_PI_2 + (i as f64) * std::f64::consts::PI / (steps_lobe as f64);
            pts.push(Point::new(c1.x + r1 * a.cos(), c1.y + r1 * a.sin()));
        }

        let c2 = Point::new(0.0, -3.0 * scale + offset_y);
        let r2 = 9.0 * scale;
        let a_start = -2.973;
        let a_end = -0.169;
        let steps_top = 10;
        for i in 1..=steps_top {
            let a = a_start + (i as f64) * (a_end - a_start) / (steps_top as f64);
            pts.push(Point::new(c2.x + r2 * a.cos(), c2.y + r2 * a.sin()));
        }

        let c3 = Point::new(9.0 * scale, 2.0 * scale + offset_y);
        let r3 = 6.0 * scale;
        for i in 1..steps_lobe {
            let a = -std::f64::consts::FRAC_PI_2 + (i as f64) * std::f64::consts::PI / (steps_lobe as f64);
            pts.push(Point::new(c3.x + r3 * a.cos(), c3.y + r3 * a.sin()));
        }

        SubPath::polyline(&pts, true)
    };

    // 3. Cloud: Merged circular lobes with a flat base.
    let cloud = PathData::single(cloud_subpath(0.0, 1.0));

    // 4. Rain: Cloud outline with angled falling droplet/dash lines.
    let mut rain_subpaths = vec![cloud_subpath(-4.0, 0.9)];
    let drops = [(-6.0, 6.0), (0.0, 8.0), (6.0, 6.0)];
    for &(x0, y0) in &drops {
        let (dx, dy) = (-2.0, 6.0);
        let w = 1.0;
        let d = [Point::new(x0 + w, y0), Point::new(x0 + dx + w, y0 + dy), Point::new(x0 + dx - w, y0 + dy), Point::new(x0 - w, y0)];
        rain_subpaths.push(SubPath::polyline(&d, true));
    }
    let rain = PathData::new(rain_subpaths);

    // 5. Thunder: Angular zigzag lightning bolt polygon.
    let bolt = [(3.0, -18.0), (-8.0, -1.0), (-1.0, -1.0), (-5.0, 18.0), (8.0, 1.0), (1.0, 1.0)];
    let bolt_pts: Vec<Point> = bolt.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let thunder = PathData::single(SubPath::polyline(&bolt_pts, true));

    // 6. Snow: Symmetrical snowflake with hexagonal cross and branching ticks.
    let mut snow_pts = Vec::new();
    let r_hub = 3.5;
    for k in 0..6 {
        let angle = (k as f64) * std::f64::consts::PI / 3.0;
        let (u_sin, u_cos) = angle.sin_cos();
        let u = Point::new(u_cos, u_sin);
        let v = Point::new(-u_sin, u_cos);
        let to_pt = |du: f64, dv: f64| Point::new(du * u.x + dv * v.x, du * u.y + dv * v.y);

        let valley_angle = angle - std::f64::consts::PI / 6.0;
        snow_pts.push(Point::new(r_hub * valley_angle.cos(), r_hub * valley_angle.sin()));

        snow_pts.push(to_pt(5.0, -1.0));
        snow_pts.push(to_pt(8.5, -1.0));
        snow_pts.push(to_pt(10.0, -3.0));
        snow_pts.push(to_pt(11.5, -1.0));
        snow_pts.push(to_pt(15.5, -1.0));
        snow_pts.push(to_pt(15.5, -2.5));

        snow_pts.push(to_pt(18.0, 0.0));

        snow_pts.push(to_pt(15.5, 2.5));
        snow_pts.push(to_pt(15.5, 1.0));
        snow_pts.push(to_pt(11.5, 1.0));
        snow_pts.push(to_pt(10.0, 3.0));
        snow_pts.push(to_pt(8.5, 1.0));
        snow_pts.push(to_pt(5.0, 1.0));
    }
    let snow = PathData::single(SubPath::polyline(&snow_pts, true));

    let icons = [
        ("Sun", sun, "#f39c12"),
        ("Moon", moon, "#5c6bc0"),
        ("Cloud", cloud, "#78909c"),
        ("Rain", rain, "#3498db"),
        ("Thunder", thunder, "#f1c40f"),
        ("Snow", snow, "#29b6f6"),
    ];
    let mut doc = Document::new(100.0, 100.0);
    for (name, path, hex) in icons {
        let fill = Color::from_hex(hex).unwrap_or(Color::BLACK);
        let edge = fill.lerp(&Color::BLACK, 0.35);
        let id = doc.alloc_id();
        let art = Node::path(id, path, Appearance::basic(Paint::solid(fill), Paint::solid(edge), 1.25));
        let Ok((art, size, _)) = normalize(art) else { continue };
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
        set_natural_size(&mut doc, name, Some(size));
    }
    doc.title = "Weather Icons".into();
    SymbolLibrary { name: "Weather Icons".into(), doc }
}

/// An instance of symbol `name` (what the resource gathering follows).
fn instance(name: &str) -> Node {
    Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::IDENTITY })
}

/// Symbol `from`'s Symbol Options in `src` (natural size, registration, export type, 9-slice
/// guides) given to symbol `to` of `d`.
fn copy_options(src: &Document, from: &str, d: &mut Document, to: &str) {
    set_natural_size(d, to, Some(natural_size(src, from)));
    d.set_symbol_registration(to, src.symbol_registration(from));
    d.set_symbol_export_type(to, src.symbol_export_type(from));
    d.set_symbol_nine_slice(to, src.symbol_nine_slice(from));
}

fn builtin_info(b: &BuiltinSymbolLibrary) -> LibraryInfo {
    LibraryInfo { id: b.id.into(), name: b.name.into(), category: "builtIn" }
}

/// Every symbol library, in menu order: built-in, User Defined, loaded.
pub fn libraries(s: &Session) -> Vec<LibraryInfo> {
    BUILTIN.iter().map(builtin_info).chain(s.symbol_libraries.infos().cloned()).collect()
}

/// Library `key` (an id, or a name in any case) with its info.
pub fn library(s: &Session, key: &str) -> Option<(LibraryInfo, Arc<SymbolLibrary>)> {
    if let Some(found) = s.symbol_libraries.get(key) {
        return Some(found);
    }
    if let Some(found) = concealed_library_id(s, key).and_then(|id| s.symbol_libraries.get(&id)) {
        return Some(found);
    }
    if let Some(b) = BUILTIN.iter().find(|b| b.id == key) {
        return Some((builtin_info(b), builtin_library(key)?));
    }
    let id = libraries(s).into_iter().find(|l| l.name.eq_ignore_ascii_case(key))?.id;
    library(s, &id)
}

fn library_param(s: &Session, p: &Value, cmd: &str) -> Result<(LibraryInfo, Arc<SymbolLibrary>)> {
    let key = str_param(p, "library").ok_or_else(|| bad(cmd, "missing `library` (see symbol.libraries)"))?;
    library(s, key).ok_or_else(|| bad(cmd, format!("no symbol library `{key}` (see symbol.libraries)")))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    s.symbol_libraries.rescan();
    let concealed = concealed_library_ids(s, []);
    let libs: Vec<Value> = libraries(s)
        .into_iter()
        .map(|l| {
            let count = library(s, &l.id).map_or(0, |(_, lib)| lib.len());
            let (id, path) = (shown_id(&concealed, &l.id), s.symbol_libraries.shown_path(&l.id));
            json!({"id": id, "name": l.name, "category": l.category, "count": count, "path": path})
        })
        .collect();
    Ok(json!({ "libraries": libs, "userFolder": s.symbol_libraries.shown_user_dir() }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (info, lib) = library_param(s, p, "symbol.library")?;
    let d = &lib.doc;
    let symbols: Vec<Value> = d
        .symbols
        .iter()
        .map(|sym| {
            let (w, h) = natural_size(d, &sym.name);
            json!({"name": sym.name, "size": [w, h], "registration": d.symbol_registration(&sym.name).0, "exportType": d.symbol_export_type(&sym.name).id()})
        })
        .collect();
    Ok(json!({"id": shown_library_id(s, &info.id), "name": info.name, "category": info.category, "symbols": symbols}))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.addFromLibrary";
    let (info, lib) = library_param(s, p, C)?;
    let mut names = str_list(p, "names");
    names.extend(str_param(p, "name").map(str::to_string));
    if let Some(n) = names.iter().find(|n| lib.symbol(n).is_none()) {
        return Err(bad(C, format!("no symbol `{n}` in `{}`", lib.name)));
    }
    let mut picked: Vec<String> = if names.is_empty() { lib.doc.symbols.iter().map(|s| s.name.clone()).collect() } else { names };
    // Each once, in the order asked for.
    let mut seen = HashSet::new();
    picked.retain(|n| seen.insert(n.clone()));
    let place = bool_or(p, "place", false);
    if place && picked.is_empty() {
        return Err(bad(C, format!("`{}` has no symbols to place", lib.name)));
    }
    let choices = SwatchChoices::parse(C, p)?;
    let clip = Clipboard::gather(&lib.doc, picked.iter().map(|n| instance(n)).collect());
    let st = s.doc()?;
    let before: HashSet<String> = st.doc.symbols.iter().map(|s| s.name.clone()).collect();
    // A library symbol's name in the document: its own, or the one the import gave it.
    let target = |renamed: &[Value], n: &str| -> String {
        let to = renamed.iter().find(|r| r["kind"] == "symbol" && r["from"] == n).and_then(|r| r["to"].as_str());
        to.unwrap_or(n).to_string()
    };
    // Where the symbols land, worked out on a copy: nothing to add and nothing to place is no edit.
    let mut probe = (*st.doc).clone();
    let imported = clip.import_into(&mut probe, &choices, false)?;
    let doc_names: Vec<String> = picked.iter().map(|n| target(&imported.renamed, n)).collect();
    let centre = st.doc.artboards.first().map(|a| a.rect.center()).unwrap_or(Point::ZERO);
    let at = if place { Some(Point::new(number_or(p, "x", centre.x, C)?, number_or(p, "y", centre.y, C)?)) } else { None };
    let parent = if place { st.target_parent()? } else { None };
    let mut out = json!({ "library": shown_library_id(s, &info.id) });
    if imported.added > 0 || place {
        let placed = s.edit("Add to Symbols", |d, sel| {
            let imported = clip.import_into(d, &choices, false)?;
            // Every symbol brought in (nested ones too) takes its Symbol Options along.
            for sym in &clip.symbols {
                let to = target(&imported.renamed, &sym.name);
                if !before.contains(&to) {
                    copy_options(&lib.doc, &sym.name, d, &to);
                }
            }
            let Some(first) = doc_names.first() else { return Ok(None) };
            d.unknown.insert(CURRENT_SYMBOL.into(), json!(first));
            let Some(at) = at else { return Ok(None) };
            let xf = place_xf(at, natural_size(d, first), d.symbol_registration(first));
            let id = d.alloc_id();
            d.insert(parent, usize::MAX, Node::new(id, NodeKind::SymbolInstance { symbol: first.clone(), xf }))?;
            sel.set([id]);
            Ok(Some(id))
        })?;
        if let Some(id) = placed {
            out["placed"] = json!(id.0);
        }
    } else if let Some(first) = doc_names.first() {
        s.execute("symbol.setCurrent", &json!({ "name": first }))?;
    }
    let (added, existing): (Vec<&String>, Vec<&String>) = doc_names.iter().partition(|n| !before.contains(*n));
    out["names"] = json!(doc_names);
    out["added"] = json!(added);
    out["existing"] = json!(existing);
    Ok(out)
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.saveLibrary";
    let st = s.doc()?;
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| stem(&st.title()).to_string(), str::to_string);
    let lib = SymbolLibrary::from_document(&st.doc, &str_list(p, "names"), name).map_err(|e| bad(C, e))?;
    if lib.is_empty() {
        return Err(bad(C, "the document has no symbols to save"));
    }
    let mut out = json!({ "count": lib.len() });
    let bytes = vectorcraft_format::save_file(&lib.doc);
    s.symbol_libraries.write(p, &lib.name, SYMBOLS_EXT, FileData::Binary(bytes), &mut out, C)?;
    Ok(out)
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.loadLibrary";
    let (info, lib) = s.symbol_libraries.load(p, C, |bytes, file| {
        let loaded = super::fileio::load(file, bytes).map_err(|e| bad(C, e.to_string()))?;
        if loaded.preview_only {
            return Err(bad(C, "only this Affinity file's embedded preview could be read; use File › Open to see it with its warning"));
        }
        SymbolLibrary::from_document(&loaded.doc, &[], stem(file).to_string()).map_err(|e| bad(C, e))
    })?;
    Ok(json!({"library": shown_library_id(s, &info.id), "name": info.name, "count": lib.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library made from thousands of symbols holds them all, once each, in the panel order of
    /// the document they came from, a name given twice kept at its first place.
    #[test]
    fn many_symbols_keep_their_panel_order() {
        let mut src = Document::new(100.0, 100.0);
        let n = 4000;
        // Names out of alphabetical order, so the order kept is the panel's.
        src.symbols = (0..n)
            .map(|i| Symbol { name: format!("s{}", (i * 7919) % n), art: Arc::new(Node::group(NodeId(i as u64 + 1), vec![])), dynamic: false })
            .collect();
        let lib = SymbolLibrary::from_document(&src, &[], "Many".into()).unwrap();
        let names = |d: &Document| d.symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&lib.doc), names(&src));
        let picked = ["s3".to_string(), "s1".to_string()];
        let lib = SymbolLibrary::from_document(&src, &picked, "Two".into()).unwrap();
        let order = |name: &str| src.symbols.iter().position(|s| s.name == name);
        let mut want = vec!["s3", "s1"];
        want.sort_by_key(|n| order(n));
        assert_eq!(names(&lib.doc), want);
        assert!(SymbolLibrary::from_document(&src, &["nope".to_string()], "None".into()).is_err());
    }
}
