use super::kind::NodeKind;
use super::live_shape::LiveShape;
use super::{Deserialize, Node, Serialize};

/// Window → Attributes: an object's settings that don't change how it prints.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectAttributes {
    /// Show Center / Don't Show Center (`None`: the object kind's default, [`Node::shows_center`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_center: Option<bool>,
    /// Image Map: the clickable area the URL covers in web output.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub image_map: ImageMap,
    /// URL the object links to (SVG `<a href>`; empty: none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// The note shown in the Attributes panel.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The object's own data, in order: SVG's `data-*` attributes (`data-pivot="100,180"` is
    /// `("pivot", "100,180")`), kept from import to export (`object.setProps {data}`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<(String, String)>,
    /// Window › SVG Interactivity: the JavaScript each event runs, in the panel's order
    /// (`("onclick", "alert(1)")`, one entry per event of [`SVG_EVENTS`]). SVG export writes them as
    /// the element's event attributes; nothing in VectorCraft runs them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<(String, String)>,
    /// The CSS classes the object had in an imported SVG, written back to `class` on export
    /// (beside the generated `cls-N` classes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    /// Attributes in other applications' XML namespaces, in order, kept from SVG import to SVG
    /// export: what another tool notes on an object (a plotter's pen, a cutter's passes) survives
    /// a round trip through VectorCraft. Nothing in VectorCraft reads them. They are the object's,
    /// as `data` is, so copies, duplicates and pastes keep them and a deleted object takes them
    /// with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign: Vec<ForeignAttribute>,
    /// Elements in other applications' XML namespaces that were children of the object's SVG
    /// group, in order, kept whole from SVG import to SVG export as `foreign` is (#9241): a tool's
    /// own configuration (`<app:config>`) survives a round trip through VectorCraft. Export writes
    /// them first among the group's children. Nothing in VectorCraft reads them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign_elements: Vec<ForeignElement>,
    /// The object's tags, in order: name and value pairs that scripts read and write
    /// (`object.tag.add`). Names may be empty or repeated. At most [`MAX_TAGS`], each name and
    /// value at most [`MAX_TAG_TEXT`] bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<ObjectTag>,
}

/// The most tags one object has.
pub const MAX_TAGS: usize = 64;

/// The longest name or value a tag has, in bytes.
pub const MAX_TAG_TEXT: usize = 64 * 1024;

/// A tag on an object: a name and a value, with an id that tells it apart from the object's other
/// tags while its name and value change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectTag {
    /// Distinct among the object's tags: a new tag takes an id from
    /// [`crate::Document::alloc_tag_id`], and deleting a tag raises the next node id above its id,
    /// so a deleted tag's id is not given out again.
    #[serde(default, deserialize_with = "lenient_tag_id")]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

/// Reading a native file never fails over a tag id that is not a whole number from 0 to
/// `u64::MAX` (a negative one, a fraction, text): it reads as 0, and
/// [`crate::Document::tidy_tags`] gives the tag a new id.
fn lenient_tag_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(serde_json::Value::deserialize(d)?.as_u64().unwrap_or(0))
}

impl ObjectAttributes {
    /// Keeps the tags within the limits: the first [`MAX_TAGS`] whose name and value are at most
    /// [`MAX_TAG_TEXT`] bytes. → whether any was dropped.
    pub fn tidy_tags(&mut self) -> bool {
        let before = self.tags.len();
        self.tags.retain(|t| t.name.len() <= MAX_TAG_TEXT && t.value.len() <= MAX_TAG_TEXT);
        self.tags.truncate(MAX_TAGS);
        self.tags.len() != before
    }
}

impl crate::Document {
    /// Keeps every object's tags within the limits ([`ObjectAttributes::tidy_tags`]) in the art of
    /// the layers, symbols, graph designs and pattern swatches (opacity-mask art included). A tag
    /// keeps its id when the id is from 1 to below [`crate::limits::MAX_NEXT_ID`] and no earlier
    /// tag of the object has it. Every other tag gets a new id: from the next node id on, above
    /// every kept id, when those ids stay below that limit; otherwise the smallest id the object's
    /// other tags do not have. The next node id is then raised past every tag id below the limit.
    /// It recurses through the art: call it on a document no deeper than
    /// [`crate::depth::MAX_DEPTH`].
    pub fn tidy_tags(&mut self) {
        fn keeps(t: &ObjectTag, earlier: &[ObjectTag]) -> bool {
            (1..crate::limits::MAX_NEXT_ID).contains(&t.id) && !earlier.iter().any(|o| o.id == t.id)
        }
        // The limits, the largest kept id and the number of new ids first, so that a new id never
        // meets a kept one.
        let mut next = self.peek_next_id();
        let mut fresh = 0u64;
        self.each_tagged(&mut |a| {
            a.tidy_tags();
            for (i, t) in a.tags.iter().enumerate() {
                if keeps(t, &a.tags[..i]) {
                    next = next.max(t.id.saturating_add(1));
                } else {
                    fresh = fresh.saturating_add(1);
                }
            }
        });
        if fresh == 0 {
            self.raise_next_id(next);
            return;
        }
        let in_range = next.saturating_add(fresh) <= crate::limits::MAX_NEXT_ID;
        self.each_tagged(&mut |a| {
            let kept: Vec<bool> = (0..a.tags.len()).map(|i| keeps(&a.tags[i], &a.tags[..i])).collect();
            let mut used: Vec<u64> = a.tags.iter().zip(&kept).filter(|(_, k)| **k).map(|(t, _)| t.id).collect();
            for (i, k) in kept.iter().enumerate() {
                if *k {
                    continue;
                }
                let id = if in_range {
                    next = next.saturating_add(1);
                    next - 1
                } else {
                    (1..).find(|c| !used.contains(c)).unwrap_or(1)
                };
                a.tags[i].id = id;
                used.push(id);
            }
        });
        self.raise_next_id(next);
    }

    /// A new tag id for object `id`, which none of its tags has: the next node id. `None` once the
    /// next node id has reached [`crate::limits::MAX_NEXT_ID`] (a file with ids at the top of the
    /// range), since an id from there would be outside the range [`Self::tidy_tags`] keeps and an
    /// id below it could be a deleted tag's.
    pub fn alloc_tag_id(&mut self, id: crate::NodeId) -> Option<u64> {
        let used: Vec<u64> =
            self.node(id).map(|n| n.extras().attrs.as_deref().map_or(Vec::new(), |a| a.tags.iter().map(|t| t.id).collect())).unwrap_or_default();
        for _ in 0..=used.len() {
            if self.peek_next_id() >= crate::limits::MAX_NEXT_ID {
                return None;
            }
            let tag = self.alloc_id().0;
            if !used.contains(&tag) {
                return Some(tag);
            }
        }
        None
    }

    /// Calls `f` with the attributes of every object with tags in the art [`Self::tidy_tags`] reads.
    fn each_tagged(&mut self, f: &mut impl FnMut(&mut ObjectAttributes)) {
        let mut fix = |n: &mut Node| {
            if n.extras().attrs.as_deref().is_some_and(|a| !a.tags.is_empty()) {
                n.edit_attrs(&mut *f);
            }
        };
        let roots = self
            .layers
            .iter_mut()
            .chain(self.symbols.iter_mut().map(|s| &mut s.art))
            .chain(self.graph_designs.iter_mut().map(|g| &mut g.art))
            .chain(self.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
            .chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()).flat_map(|p| p.art.iter_mut()));
        for root in roots {
            crate::placed_document::each_mut(std::sync::Arc::make_mut(root), &mut fix);
        }
    }
}

/// An attribute in another application's XML namespace: `plotter:pen="2"` with `xmlns:plotter=
/// "https://example.org/plotter"` is `{ns: "https://example.org/plotter", prefix: "plotter", name:
/// "pen", value: "2"}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignAttribute {
    /// The namespace's URI, which names it.
    pub ns: String,
    /// The prefix the file gave the namespace, which export reuses where it can.
    pub prefix: String,
    /// The attribute's local name.
    pub name: String,
    /// The attribute's value, as the file had it.
    pub value: String,
}

/// An element in another application's XML namespace, with its attributes and what it holds:
/// `<app:config mode="on">text</app:config>` with `xmlns:app="https://example.com/app"` is
/// `{ns: "https://example.com/app", prefix: "app", name: "config", attrs: [{ns: "", …, name:
/// "mode", value: "on"}], content: [Text("text")]}`. Elements inside it may be in any namespace
/// (`ns` empty: in none).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignElement {
    /// The namespace's URI (empty: no namespace).
    pub ns: String,
    /// The prefix the file gave the namespace, which export reuses where it can.
    pub prefix: String,
    /// The element's local name.
    pub name: String,
    /// Its attributes, in order; one without a namespace has `ns` and `prefix` empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attrs: Vec<ForeignAttribute>,
    /// Its text and elements, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ForeignContent>,
}

/// What a [`ForeignElement`] holds: text (CDATA sections included), or an element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ForeignContent {
    Text(String),
    Element(ForeignElement),
}

impl ForeignElement {
    /// The heap the element owns, with what it holds.
    pub fn heap_bytes(&self) -> usize {
        let attrs = self.attrs.capacity() * std::mem::size_of::<ForeignAttribute>()
            + self.attrs.iter().map(|a| a.ns.capacity() + a.prefix.capacity() + a.name.capacity() + a.value.capacity()).sum::<usize>();
        let content = self.content.capacity() * std::mem::size_of::<ForeignContent>()
            + self
                .content
                .iter()
                .map(|c| match c {
                    ForeignContent::Text(t) => t.capacity(),
                    ForeignContent::Element(e) => e.heap_bytes(),
                })
                .sum::<usize>();
        self.ns.capacity() + self.prefix.capacity() + self.name.capacity() + attrs + content
    }
}

/// The events the SVG Interactivity panel attaches JavaScript to, by the names of the SVG event
/// attributes that run it.
pub const SVG_EVENTS: [&str; 19] = [
    "onfocusin",
    "onfocusout",
    "onactivate",
    "onmousedown",
    "onmouseup",
    "onclick",
    "onmouseover",
    "onmousemove",
    "onmouseout",
    "onkeydown",
    "onkeypress",
    "onkeyup",
    "onload",
    "onerror",
    "onabort",
    "onunload",
    "onzoom",
    "onresize",
    "onscroll",
];

/// The most JavaScript files a document links (`Document::svg_scripts`): SVG import keeps the
/// first ones and warns about the rest, and `svgInteractivity.addFile` refuses another.
pub const MAX_SVG_SCRIPTS: usize = 64;

/// The Attributes panel's Image Map shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageMap {
    #[default]
    None,
    Rectangle,
    Polygon,
}

impl ImageMap {
    pub const ALL: [ImageMap; 3] = [ImageMap::None, ImageMap::Rectangle, ImageMap::Polygon];

    pub fn label(self) -> &'static str {
        match self {
            ImageMap::None => "None",
            ImageMap::Rectangle => "Rectangle",
            ImageMap::Polygon => "Polygon",
        }
    }

    /// Parse a label or serialized name, ignoring case.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(s))
    }
}

impl Node {
    /// The URL this object links to, if any.
    pub fn url(&self) -> Option<&str> {
        self.extras().attrs.as_deref().map(|a| a.url.as_str()).filter(|u| !u.is_empty())
    }

    /// Whether the canvas shows this object's centre point when it is selected: as set in the
    /// Attributes panel, else [`Self::shows_center_by_default`].
    pub fn shows_center(&self) -> bool {
        self.extras().attrs.as_deref().and_then(|a| a.show_center).unwrap_or_else(|| self.shows_center_by_default())
    }

    /// Shapes drawn with the shape tools (live rectangles, ellipses and polygons) show their centre.
    pub fn shows_center_by_default(&self) -> bool {
        matches!(&self.kind, NodeKind::Path { live: Some(l), .. } if !matches!(**l, LiveShape::Line { .. } | LiveShape::Path { .. }))
    }

    /// Change this object's attributes with `f`; all-default attributes are dropped.
    pub fn edit_attrs<R>(&mut self, f: impl FnOnce(&mut ObjectAttributes) -> R) -> R {
        let mut a = self.extras_mut().attrs.take().unwrap_or_default();
        let r = f(&mut a);
        self.extras_mut().attrs = (*a != ObjectAttributes::default()).then_some(a);
        r
    }
}
