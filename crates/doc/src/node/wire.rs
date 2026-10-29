use super::attributes::ObjectAttributes;
use super::extras::{Extras, NO_EXTRAS, NodeExtras};
use super::ids::NodeId;
use super::kind::NodeKind;
use super::opacity::{Knockout, OpacityMask};
use super::{Appearance, BlendMode, Deserialize, Node, PathData, Serialize, one, yes};

/// How a [`Node`] is written: the flat field list of the file format, in its order, borrowed from
/// the node and its extras (nothing is copied). The node reads back through `NodeWire`.
impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use crate::skip::{is_default, is_one, is_true};
        use serde::ser::SerializeStruct;
        let e = self.extras();
        // As the derive did: the fields written, then each field written or skipped in order.
        let skips = [
            false,
            self.name.is_none(),
            is_true(&self.visible),
            is_default(&self.locked),
            is_one(&self.opacity),
            is_default(&self.blend),
            is_default(&self.isolate),
            is_default(&self.knockout),
            is_default(&self.knockout_shape),
            is_default(&self.appearance),
            e.mask.is_none(),
            e.trace.is_none(),
            e.wrap.is_none(),
            e.graph.is_none(),
            e.series_index.is_none(),
            is_default(&e.type_outlines),
            is_default(&e.pixelated),
            e.shaper.is_none(),
            false,
            e.graphic_style.is_none(),
            e.attrs.is_none(),
            e.slice.is_none(),
            is_default(&self.bbox_angle),
            e.perspective.is_none(),
            e.dimension.is_none(),
            e.intertwine.is_none(),
            e.live_paint_gaps.is_none(),
            e.objects_on_path.is_none(),
        ];
        let mut st = s.serialize_struct("Node", skips.iter().filter(|s| !**s).count())?;
        let mut skip = skips.into_iter();
        macro_rules! field {
            ($name:literal, $v:expr) => {
                if skip.next().unwrap_or(false) {
                    st.skip_field($name)?;
                } else {
                    st.serialize_field($name, $v)?;
                }
            };
        }
        field!("id", &self.id);
        field!("name", &self.name);
        field!("visible", &self.visible);
        field!("locked", &self.locked);
        field!("opacity", &self.opacity);
        field!("blend", &self.blend);
        field!("isolate", &self.isolate);
        field!("knockout", &self.knockout);
        field!("knockout_shape", &self.knockout_shape);
        field!("appearance", &self.appearance);
        field!("mask", &e.mask);
        field!("trace", &e.trace);
        field!("wrap", &e.wrap);
        field!("graph", &e.graph);
        field!("series_index", &e.series_index);
        field!("type_outlines", &e.type_outlines);
        field!("pixelated", &e.pixelated);
        field!("shaper", &e.shaper);
        field!("kind", &self.kind);
        field!("graphic_style", &e.graphic_style);
        field!("attrs", &e.attrs);
        field!("slice", &e.slice);
        field!("bbox_angle", &self.bbox_angle);
        field!("perspective", &e.perspective);
        field!("dimension", &e.dimension);
        field!("intertwine", &e.intertwine);
        field!("live_paint_gaps", &e.live_paint_gaps);
        field!("objects_on_path", &e.objects_on_path);
        st.end()
    }
}

/// Most objects [`with_node_budget`] lets a document read: 4M (about 1 GB of the smallest
/// objects), 512K in the web build. Paths have their own anchor budget
/// ([`vectorcraft_geom::path::MAX_READ_ANCHORS`]); this one bounds the objects themselves.
pub const MAX_READ_NODES: usize = if cfg!(target_arch = "wasm32") { 512 << 10 } else { 4 << 20 };

std::thread_local! {
    /// Objects the document being read on this thread may still have, and the budget they come
    /// from ([`with_node_budget`]).
    static NODES_LEFT: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
}

/// Run `f` (which reads nodes with serde) letting it read at most `max` objects together; reading
/// more fails. A compressed file of repeated empty objects stays small while they take gigabytes
/// once read, about 270 bytes each against some 50 in the file (#3587).
pub fn with_node_budget<T>(max: usize, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<(usize, usize)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            NODES_LEFT.set(self.0);
        }
    }
    let _restore = Restore(NODES_LEFT.replace(Some((max, max))));
    f()
}

/// A node's id, counting the node against [`with_node_budget`]. Every node has exactly one id, so
/// each counts once, and the id usually comes first, before the node's children are read.
fn counted_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<NodeId, D::Error> {
    let id = NodeId::deserialize(d)?;
    if let Some((left, max)) = NODES_LEFT.get() {
        let left = left.checked_sub(1).ok_or_else(|| serde::de::Error::custom(format!("the document has more than {max} objects")))?;
        NODES_LEFT.set(Some((left, max)));
    }
    Ok(id)
}

/// How a [`Node`] is read: the flat field list of the file format. Fields move into the node
/// (and its extras, allocated only when one is set) without a copy of what they own.
#[derive(Deserialize)]
#[serde(rename = "Node")]
pub(super) struct NodeWire {
    #[serde(deserialize_with = "counted_id")]
    id: NodeId,
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    blend: BlendMode,
    #[serde(default)]
    isolate: bool,
    #[serde(default)]
    knockout: Knockout,
    #[serde(default)]
    knockout_shape: bool,
    #[serde(default)]
    appearance: crate::Shared<Appearance>,
    #[serde(default)]
    mask: Option<Box<OpacityMask>>,
    #[serde(default)]
    trace: Option<Box<serde_json::Value>>,
    #[serde(default)]
    wrap: Option<crate::text::TextWrap>,
    #[serde(default)]
    graph: Option<Box<crate::graph::GraphSpec>>,
    #[serde(default)]
    series_index: Option<u32>,
    #[serde(default)]
    type_outlines: bool,
    #[serde(default)]
    pixelated: bool,
    #[serde(default)]
    shaper: Option<Box<crate::shaper::ShaperSpec>>,
    kind: NodeKind,
    #[serde(default)]
    graphic_style: Option<u32>,
    #[serde(default)]
    attrs: Option<Box<ObjectAttributes>>,
    #[serde(default)]
    slice: Option<Box<crate::SliceOptions>>,
    #[serde(default)]
    bbox_angle: f64,
    #[serde(default)]
    perspective: Option<Box<crate::PerspectiveAttachment>>,
    #[serde(default)]
    dimension: Option<Box<crate::dimension::Dimension>>,
    #[serde(default)]
    intertwine: Option<Box<PathData>>,
    #[serde(default)]
    live_paint_gaps: Option<Box<crate::live_paint::GapOptions>>,
    #[serde(default)]
    objects_on_path: Option<Box<crate::objects_on_path::ObjectsOnPath>>,
}

impl From<NodeWire> for Node {
    fn from(w: NodeWire) -> Self {
        let extras = NodeExtras {
            mask: w.mask,
            trace: w.trace,
            wrap: w.wrap,
            graph: w.graph,
            series_index: w.series_index,
            type_outlines: w.type_outlines,
            pixelated: w.pixelated,
            shaper: w.shaper,
            graphic_style: w.graphic_style,
            attrs: w.attrs,
            slice: w.slice,
            perspective: w.perspective,
            dimension: w.dimension,
            intertwine: w.intertwine,
            live_paint_gaps: w.live_paint_gaps,
            objects_on_path: w.objects_on_path,
        };
        Node {
            id: w.id,
            name: w.name,
            visible: w.visible,
            locked: w.locked,
            opacity: w.opacity,
            blend: w.blend,
            isolate: w.isolate,
            knockout: w.knockout,
            knockout_shape: w.knockout_shape,
            appearance: w.appearance,
            kind: w.kind,
            bbox_angle: w.bbox_angle,
            extras: Extras((extras != NO_EXTRAS).then(|| Box::new(extras))),
        }
    }
}
