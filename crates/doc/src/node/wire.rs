use super::attributes::ObjectAttributes;
use super::extras::{Extras, NO_EXTRAS, NodeExtras};
use super::ids::NodeId;
use super::kind::NodeKind;
use super::opacity::{Knockout, OpacityMask};
use super::*;

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

/// How a [`Node`] is read: the flat field list of the file format. Fields move into the node
/// (and its extras, allocated only when one is set) without a copy of what they own.
#[derive(Deserialize)]
#[serde(rename = "Node")]
pub(super) struct NodeWire {
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
