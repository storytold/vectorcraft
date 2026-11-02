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
    /// Attributes in other applications' XML namespaces, in order, kept from SVG import to SVG
    /// export: what another tool notes on an object (a plotter's pen, a cutter's passes) survives
    /// a round trip through VectorCraft. Nothing in VectorCraft reads them. They are the object's,
    /// as `data` is, so copies, duplicates and pastes keep them and a deleted object takes them
    /// with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign: Vec<ForeignAttribute>,
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
