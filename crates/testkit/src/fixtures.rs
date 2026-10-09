//! Sessions and documents in known states.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::{PathData, Rect, shapes};

/// Execute a command and panic with context on error.
#[track_caller]
pub fn exec(s: &mut Session, id: &str, params: Value) -> Value {
    match s.execute(id, &params) {
        Ok(v) => v,
        Err(e) => panic!("`{id}` {params} failed: {e}"),
    }
}

/// The `id` field of a creation command's result.
#[track_caller]
pub fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap_or_else(|| panic!("no id in {v}")))
}

/// The created date (Unix seconds) of every fixture document: File Info dates come from the clock
/// otherwise, and documents made a second apart would not compare equal.
pub const CREATED: i64 = 1_767_225_600;

/// A session with one new `w`×`h` document, dated [`CREATED`].
pub fn session_with(w: f64, h: f64) -> Session {
    let mut s = Session::new();
    exec(&mut s, "file.new", json!({"width": w, "height": h, "created": CREATED}));
    s
}

/// A session with one new 400×300 document.
pub fn session() -> Session {
    session_with(400.0, 300.0)
}

pub fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&exec(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})))
}
pub fn ellipse(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&exec(s, "shape.ellipse", json!({"x": x, "y": y, "width": w, "height": h})))
}
pub fn star(s: &mut Session, cx: f64, cy: f64, r1: f64, r2: f64) -> NodeId {
    id_of(&exec(s, "shape.star", json!({"cx": cx, "cy": cy, "radius1": r1, "radius2": r2, "points": 5})))
}
pub fn select(s: &mut Session, ids: &[NodeId]) {
    exec(s, "select.set", json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}));
}

/// The three standard fixture states for sweeping commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
    /// A new document with nothing in it.
    Empty,
    /// One rectangle, selected.
    Single,
    /// Several overlapping objects of different kinds (paths, live shapes, text, a group), three selected.
    Multi,
}

impl Fixture {
    pub const ALL: [Fixture; 3] = [Fixture::Empty, Fixture::Single, Fixture::Multi];

    pub fn session(self) -> Session {
        let mut s = session();
        match self {
            Fixture::Empty => {}
            Fixture::Single => {
                rect(&mut s, 50.0, 50.0, 100.0, 60.0);
            }
            Fixture::Multi => {
                let a = rect(&mut s, 40.0, 40.0, 120.0, 80.0);
                let b = ellipse(&mut s, 100.0, 60.0, 90.0, 90.0);
                let c = star(&mut s, 200.0, 150.0, 50.0, 25.0);
                let t = id_of(&exec(&mut s, "text.create", json!({"x": 20, "y": 250, "text": "Hi", "size": 24})));
                let p = id_of(&exec(
                    &mut s,
                    "path.create",
                    json!({"anchors": [{"x": 250, "y": 30}, {"x": 330, "y": 60, "in": [300, 20], "out": [360, 100]}, {"x": 300, "y": 140}], "closed": false}),
                ));
                let g1 = rect(&mut s, 300.0, 200.0, 40.0, 40.0);
                let g2 = ellipse(&mut s, 320.0, 220.0, 40.0, 40.0);
                select(&mut s, &[g1, g2]);
                exec(&mut s, "object.group", json!({}));
                let _ = (t, p);
                select(&mut s, &[a, b, c]);
            }
        }
        s
    }
}

/// A document touching most node kinds and appearance features, built through commands (so its
/// history is non-trivial). Selection is left empty.
pub fn rich_session() -> Session {
    let mut s = session_with(500.0, 400.0);
    let a = rect(&mut s, 20.0, 20.0, 150.0, 100.0);
    exec(
        &mut s,
        "paint.setFill",
        json!({"gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}], "angle": 30}}),
    );
    let b = ellipse(&mut s, 100.0, 60.0, 120.0, 120.0);
    exec(&mut s, "transparency.set", json!({"opacity": 60, "blend": "Multiply"}));
    exec(&mut s, "stroke.set", json!({"weight": 4, "dash": [6, 3], "cap": "round"}));
    let c = star(&mut s, 300.0, 100.0, 60.0, 30.0);
    exec(&mut s, "effect.apply", json!({"effect": "distort.roughen", "params": {"size": 3, "detail": 5, "seed": 7}}));
    let l = id_of(&exec(&mut s, "shape.line", json!({"x1": 20, "y1": 300, "x2": 200, "y2": 350})));
    exec(&mut s, "stroke.set", json!({"weight": 3, "endArrow": "Arrow"}));
    let tx = id_of(&exec(&mut s, "text.create", json!({"x": 250, "y": 300, "text": "VectorCraft", "size": 30})));
    // A non-default paragraph composer, so format round trips cover it.
    exec(&mut s, "text.setFormat", json!({"ids": [tx.0], "composer": "singleLine"}));
    // Area type with non-default Area Type Options (vertical alignment, inset).
    let area = id_of(&exec(
        &mut s,
        "text.create",
        json!({"x": 250, "y": 340, "text": "Centred area type", "size": 12, "area": {"width": 120, "height": 50}}),
    ));
    select(&mut s, &[area]);
    exec(&mut s, "text.areaOptions", json!({"verticalAlign": "center", "inset": 2}));
    let _ = id_of(&exec(
        &mut s,
        "text.create",
        json!({"x": 300, "y": 330, "size": 14, "text": "Area type shrinks its text to fit the frame.",
               "area": {"width": 150, "height": 40, "fit": "shrinkText", "fitMinPercent": 40}}),
    ));
    // Area type whose paragraphs differ (alignment, spacing, tabs).
    let para = id_of(&exec(
        &mut s,
        "text.create",
        json!({"x": 20, "y": 360, "text": "Heading\nBody one\tx\nBody two", "size": 9, "area": {"width": 160, "height": 36}}),
    ));
    exec(&mut s, "text.setStyle", json!({"id": para, "justify": "center", "start": 0, "end": 0}));
    exec(&mut s, "text.setFormat", json!({"ids": [para], "spaceBefore": 3, "firstLineIndent": 6, "start": 9, "end": 9}));
    exec(&mut s, "text.tabs.set", json!({"ids": [para], "stops": [{"position": 80, "align": "right"}], "start": 9, "end": 9}));
    exec(&mut s, "layer.new", json!({"name": "Top"}));
    let d = rect(&mut s, 350.0, 200.0, 80.0, 80.0);
    let e = ellipse(&mut s, 380.0, 230.0, 80.0, 80.0);
    select(&mut s, &[d, e]);
    exec(&mut s, "object.compoundPath.make", json!({}));
    // A live compound shape: a ring (a square minus a circle) with a member of each mode.
    let ring = rect(&mut s, 470.0, 200.0, 80.0, 80.0);
    let hole = ellipse(&mut s, 490.0, 220.0, 40.0, 40.0);
    select(&mut s, &[ring, hole]);
    let cs = id_of(&exec(&mut s, "object.compoundShape.make", json!({"mode": "subtract"})));
    let x = rect(&mut s, 520.0, 250.0, 50.0, 50.0);
    exec(&mut s, "layer.move", json!({"ids": [x.0], "target": cs.0, "place": "inside"}));
    exec(&mut s, "object.compoundShape.setMode", json!({"ids": [x.0], "mode": "exclude"}));
    let clip = ellipse(&mut s, 30.0, 150.0, 80.0, 80.0);
    let inner = rect(&mut s, 10.0, 170.0, 140.0, 30.0);
    exec(&mut s, "paint.setFill", json!({"color": "#22aa44"}));
    select(&mut s, &[inner, clip]);
    exec(&mut s, "object.clippingMask.make", json!({}));
    // Type with an inline graphic: a symbol (a small circle, drawn here) set in a line of text.
    let dot = ellipse(&mut s, 440.0, 40.0, 12.0, 12.0);
    select(&mut s, &[dot]);
    exec(&mut s, "symbol.new", json!({"name": "Dot"}));
    let t = id_of(&exec(&mut s, "text.create", json!({"x": 250, "y": 370, "text": "Tap  to add", "size": 18})));
    exec(&mut s, "text.insertInline", json!({"id": t.0, "at": 4, "symbol": "Dot"}));
    let _ = (a, b, c, l);
    exec(&mut s, "select.none", json!({}));
    s
}

/// Build documents directly (without the engine), e.g. for render tests.
pub struct DocBuilder {
    pub doc: Document,
    pub layer: NodeId,
}

impl DocBuilder {
    pub fn new(w: f64, h: f64) -> Self {
        let doc = Document::new(w, h);
        let layer = doc.layers[0].id;
        Self { doc, layer }
    }
    pub fn alloc(&mut self) -> NodeId {
        self.doc.alloc_id()
    }
    /// A path node (not inserted).
    pub fn path_node(&mut self, path: PathData, appearance: Appearance) -> Node {
        let id = self.alloc();
        Node::path(id, path, appearance)
    }
    /// A filled, unstroked rectangle node (not inserted).
    pub fn rect_node(&mut self, r: Rect, fill: Color) -> Node {
        self.path_node(shapes::rectangle(r), Appearance::basic(Paint::solid(fill), Paint::None, 0.0))
    }
    /// Append `node` at the top of the current layer.
    pub fn add(&mut self, node: Node) -> NodeId {
        let l = self.layer;
        self.doc.insert(Some(l), usize::MAX, node).expect("insert")
    }
    /// Add a filled rectangle; `f` may tweak the node first.
    pub fn rect(&mut self, r: Rect, fill: Color, f: impl FnOnce(&mut Node)) -> NodeId {
        let mut n = self.rect_node(r, fill);
        f(&mut n);
        self.add(n)
    }
    /// Add any path with an appearance; `f` may tweak the node first.
    pub fn path(&mut self, path: PathData, appearance: Appearance, f: impl FnOnce(&mut Node)) -> NodeId {
        let mut n = self.path_node(path, appearance);
        f(&mut n);
        self.add(n)
    }
    /// Add a group of `children` (bottom first). With `clip`, the first child is the clipping path.
    pub fn group(&mut self, mut children: Vec<Node>, clip: bool) -> NodeId {
        if clip && let Some(NodeKind::Path { clipping, .. }) = children.first_mut().map(|n| &mut n.kind) {
            *clipping = true;
        }
        let id = self.alloc();
        let mut g = Node::group(id, children.into_iter().map(Arc::new).collect());
        if let NodeKind::Group { clip: c, .. } = &mut g.kind {
            *c = clip;
        }
        self.add(g)
    }
    /// Add a new layer and make it current.
    pub fn layer(&mut self, name: &str) -> NodeId {
        self.layer = self.doc.add_layer(Some(name));
        self.layer
    }
    pub fn build(self) -> Document {
        self.doc
    }
}

/// All non-layer nodes of a document in paint order.
pub fn art_nodes(doc: &Document) -> Vec<&Node> {
    let mut v = Vec::new();
    doc.walk(|n| {
        if !n.is_layer() {
            v.push(n)
        }
    });
    v
}

/// Ids of the top-level art (direct children of layers), bottom first.
pub fn top_level_ids(doc: &Document) -> Vec<NodeId> {
    doc.layers.iter().flat_map(|l| l.children().into_iter().flatten().map(|n| n.id)).collect()
}

/// Every node id (layers included), paint order.
pub fn all_ids(doc: &Document) -> Vec<NodeId> {
    let mut v = Vec::new();
    doc.walk(|n| v.push(n.id));
    v
}
