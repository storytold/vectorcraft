use super::attributes::ObjectAttributes;
use super::kind::NodeKind;
use super::live_shape::LiveShape;
use super::opacity::OpacityMask;
use super::*;

/// The parts of a [`Node`] that few objects have: kept out of line, so that the nodes without
/// them stay small ([`Node::extras`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeExtras {
    /// Opacity mask (Transparency panel). Its art lives here, outside the layer tree.
    pub mask: Option<Box<OpacityMask>>,
    /// Image Trace object: `{preset, params, view?}` it was traced with (the Image Trace panel shows
    /// them; `view` is its [`crate::TraceView`] id, absent for the tracing result).
    pub trace: Option<Box<serde_json::Value>>,
    /// Object → Text Wrap: area type below this object (in the same layer) flows around it.
    pub wrap: Option<crate::text::TextWrap>,
    /// Graph object: the group's children are generated from this spec (Object → Graph).
    pub graph: Option<Box<crate::graph::GraphSpec>>,
    /// Index of the graph series this generated group belongs to. Axes, the legend and other
    /// children leave it unset, so a series named "Legend" or "Axes" is not mistaken for them.
    pub series_index: Option<u32>,
    /// The group holds the glyph outlines of type. Geometry effects that reshape it move and scale
    /// the placed gradients in it with the outlines.
    pub type_outlines: bool,
    /// Editable Shaper composition; the original art is retained in its source child.
    pub shaper: Option<Box<crate::shaper::ShaperSpec>>,
    /// The [`crate::GraphicStyle::id`] last applied to this object. It stays linked while it keeps
    /// that style's look: editing its appearance or transparency breaks the link.
    pub graphic_style: Option<u32>,
    /// Attributes panel: centre point display, image map, URL and note (`None`: all defaults).
    pub attrs: Option<Box<ObjectAttributes>>,
    /// Object → Slice → Make: the object is an object slice (its slice follows its bounds) with
    /// these options.
    pub slice: Option<Box<crate::SliceOptions>>,
    /// Object › Perspective: the perspective grid plane the object is attached to.
    pub perspective: Option<Box<crate::PerspectiveAttachment>>,
    /// Dimension object: the live measurement it draws; its ordinary group children hold its
    /// exportable appearance (Object › Dimensions, the Dimension tool).
    pub dimension: Option<Box<crate::dimension::Dimension>>,
    /// Object › Intertwine: as a member of an Intertwine group, the area (document coordinates,
    /// filled non-zero) where this object is drawn over the group's other members.
    pub intertwine: Option<Box<PathData>>,
    /// Live Paint group: its Gap Options (Object › Live Paint › Gap Options; `None`: the defaults).
    pub live_paint_gaps: Option<Box<crate::live_paint::GapOptions>>,
    /// Object › Objects on Path: the group's path and the objects attached to it (see
    /// [`crate::objects_on_path`]).
    pub objects_on_path: Option<Box<crate::objects_on_path::ObjectsOnPath>>,
}

/// No extras: what [`Node::extras`] gives for a node without any.
pub(super) static NO_EXTRAS: NodeExtras = NodeExtras {
    mask: None,
    trace: None,
    wrap: None,
    graph: None,
    series_index: None,
    type_outlines: false,
    shaper: None,
    graphic_style: None,
    attrs: None,
    slice: None,
    perspective: None,
    dimension: None,
    intertwine: None,
    live_paint_gaps: None,
    objects_on_path: None,
};

/// A node's [`NodeExtras`], if it has any ([`Node::extras`]). Extras that are all unset equal
/// none.
#[derive(Clone, Debug, Default)]
pub struct Extras(pub(crate) Option<Box<NodeExtras>>);

impl Extras {
    fn get(&self) -> &NodeExtras {
        self.0.as_deref().unwrap_or(&NO_EXTRAS)
    }
}

impl PartialEq for Extras {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => a == b,
            _ => self.get() == other.get(),
        }
    }
}

impl Node {
    /// The rarely set parts of this node (all unset when it has none).
    #[inline]
    pub fn extras(&self) -> &NodeExtras {
        self.extras.get()
    }

    /// The rarely set parts of this node, to change: when the change is done (the guard dropped),
    /// extras that were set are allocated and extras left all unset are let go. Setting one to
    /// `None` on a node without extras allocates nothing.
    #[inline]
    pub fn extras_mut(&mut self) -> ExtrasMut<'_> {
        ExtrasMut { slot: &mut self.extras.0, local: NodeExtras::default() }
    }

    /// The rarely set parts of this node, if it has any: to change what is there in place.
    #[inline]
    pub fn extras_if_any(&mut self) -> Option<&mut NodeExtras> {
        self.extras.0.as_deref_mut()
    }

    /// The live shape of a path object, if it is one.
    pub fn live_shape(&self) -> Option<&LiveShape> {
        match &self.kind {
            NodeKind::Path { live, .. } => live.as_deref(),
            _ => None,
        }
    }

    /// The live shape of a path object, if it is one, to change.
    pub fn live_shape_mut(&mut self) -> Option<&mut LiveShape> {
        match &mut self.kind {
            NodeKind::Path { live, .. } => live.as_deref_mut(),
            _ => None,
        }
    }

    /// This node with opacity mask `mask` (or none).
    pub fn with_mask(mut self, mask: Option<Box<OpacityMask>>) -> Self {
        self.extras_mut().mask = mask;
        self
    }

    /// The extras, if this node has them allocated (for [`crate::mem`]'s estimate).
    pub(crate) fn extras_box(&self) -> Option<&NodeExtras> {
        self.extras.0.as_deref()
    }
}

/// [`Node::extras_mut`]: the node's extras while they change.
pub struct ExtrasMut<'a> {
    slot: &'a mut Option<Box<NodeExtras>>,
    /// Stands for the extras of a node without any, boxed into it when dropped if one was set.
    local: NodeExtras,
}

impl std::ops::Deref for ExtrasMut<'_> {
    type Target = NodeExtras;
    fn deref(&self) -> &NodeExtras {
        self.slot.as_deref().unwrap_or(&self.local)
    }
}

impl std::ops::DerefMut for ExtrasMut<'_> {
    fn deref_mut(&mut self) -> &mut NodeExtras {
        match self.slot.as_deref_mut() {
            Some(e) => e,
            None => &mut self.local,
        }
    }
}

impl Drop for ExtrasMut<'_> {
    fn drop(&mut self) {
        match self.slot.as_deref() {
            Some(e) if *e == NO_EXTRAS => *self.slot = None,
            Some(_) => {}
            None if self.local != NO_EXTRAS => *self.slot = Some(Box::new(std::mem::take(&mut self.local))),
            None => {}
        }
    }
}
