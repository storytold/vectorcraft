use super::attributes::ObjectAttributes;
use super::extras::{Extras, NO_EXTRAS, NodeExtras};
use super::ids::NodeId;
use super::kind::NodeKind;
use super::opacity::{Knockout, OpacityMask};
use super::{Appearance, BlendMode, Deserialize, Node, PathData, Serialize, one, yes};

/// Field `name` of a [`Node`] written as `v`, or skipped. A function of its own, so the long field
/// list stays one straight run of calls.
fn write_field<S: serde::ser::SerializeStruct, T: Serialize + ?Sized>(st: &mut S, skip: bool, name: &'static str, v: &T) -> Result<(), S::Error> {
    if skip { st.skip_field(name) } else { st.serialize_field(name, v) }
}

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
            e.flare.is_none(),
            e.intertwine.is_none(),
            e.intertwine_tiers.is_empty(),
            e.live_paint_gaps.is_none(),
            e.objects_on_path.is_none(),
            e.colorize.is_none(),
            is_default(&e.compound_shape),
            is_default(&e.shape_mode),
            e.symbol_overrides.is_empty(),
            e.envelope_edit_margin.is_none(),
            is_default(&e.pixel_aligned),
            e.color_mask.is_none(),
            is_default(&e.warp_piece),
            is_default(&e.clip_pieces),
            e.image_quad.is_none(),
        ];
        let mut st = s.serialize_struct("Node", skips.iter().filter(|s| !**s).count())?;
        let mut skip = skips.into_iter();
        macro_rules! field {
            ($name:literal, $v:expr) => {
                write_field(&mut st, skip.next().unwrap_or(false), $name, $v)?;
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
        field!("flare", &e.flare);
        field!("intertwine", &e.intertwine);
        field!("intertwine_tiers", &e.intertwine_tiers);
        field!("live_paint_gaps", &e.live_paint_gaps);
        field!("objects_on_path", &e.objects_on_path);
        field!("colorize", &e.colorize);
        field!("compound_shape", &e.compound_shape);
        field!("shape_mode", &e.shape_mode);
        field!("symbol_overrides", &e.symbol_overrides);
        field!("envelope_edit_margin", &e.envelope_edit_margin);
        field!("pixel_aligned", &e.pixel_aligned);
        field!("color_mask", &e.color_mask);
        field!("warp_piece", &e.warp_piece);
        field!("clip_pieces", &e.clip_pieces);
        field!("image_quad", &e.image_quad);
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
    count_one()?;
    Ok(id)
}

/// Count one object against [`with_node_budget`].
fn count_one<E: serde::de::Error>() -> Result<(), E> {
    if let Some((left, max)) = NODES_LEFT.get() {
        let left = left.checked_sub(1).ok_or_else(|| E::custom(format!("the document has more than {max} objects")))?;
        NODES_LEFT.set(Some((left, max)));
    }
    Ok(())
}

/// A list read from a file (a member's Intertwine areas of tier 2 and up, an instance's symbol
/// overrides, a document's saved ruler guides), each item counting against [`with_node_budget`]
/// as an object does: a compressed file of repeated empty items stays small while they take
/// memory once read.
pub(crate) fn counted_list<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    struct Items<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Items<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            while let Some(item) = seq.next_element::<T>()? {
                count_one()?;
                out.push(item);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Items(std::marker::PhantomData))
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
    flare: Option<Box<crate::flare::FlareSpec>>,
    #[serde(default)]
    intertwine: Option<Box<PathData>>,
    #[serde(default, deserialize_with = "counted_list")]
    intertwine_tiers: Vec<PathData>,
    #[serde(default)]
    live_paint_gaps: Option<Box<crate::live_paint::GapOptions>>,
    #[serde(default)]
    objects_on_path: Option<Box<crate::objects_on_path::ObjectsOnPath>>,
    #[serde(default)]
    colorize: Option<Box<vectorcraft_color::Paint>>,
    #[serde(default)]
    compound_shape: bool,
    #[serde(default)]
    shape_mode: crate::ShapeMode,
    #[serde(default, deserialize_with = "counted_list")]
    symbol_overrides: Vec<crate::SymbolOverride>,
    #[serde(default)]
    envelope_edit_margin: Option<[f32; 4]>,
    #[serde(default)]
    pixel_aligned: bool,
    #[serde(default)]
    color_mask: Option<Box<crate::color_mask::ColorMask>>,
    #[serde(default)]
    warp_piece: bool,
    #[serde(default)]
    clip_pieces: bool,
    #[serde(default, deserialize_with = "image_corners")]
    image_quad: Option<Box<[vectorcraft_geom::Point; 4]>>,
}

/// A distorted image's corners as a file holds them: a list of exactly four points, else none (the
/// image is then placed by its transform). A point is `[x, y]` or `{"x": x, "y": y}`. A value that
/// is not a list, or a list with an entry that is not a point, is none too. Values that are not
/// taken are skipped as they are read, without being kept in memory, and at most four entries of a
/// longer list are decoded.
fn image_corners<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Box<[vectorcraft_geom::Point; 4]>>, D::Error> {
    use serde::de::{Deserializer, Error, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use std::fmt::{Formatter, Result as FmtResult};
    use vectorcraft_geom::Point;

    /// Skips the rest of a list; true when it held another entry.
    fn rest_of_list<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<bool, A::Error> {
        let mut more = false;
        while seq.next_element::<IgnoredAny>()?.is_some() {
            more = true;
        }
        Ok(more)
    }
    fn rest_of_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(())
    }
    // The visitors below give none for a value of a kind they do not take.
    macro_rules! none_for {
        (scalars) => {
            fn visit_bool<E: Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_str<E: Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
        };
        (numbers) => {
            fn visit_i64<E: Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
        };
        (maps $de:lifetime) => {
            fn visit_map<A: MapAccess<$de>>(self, map: A) -> Result<Self::Value, A::Error> {
                rest_of_map(map)?;
                Ok(None)
            }
        };
    }

    /// A coordinate: a number, else none.
    struct Coord(Option<f64>);
    impl<'de> serde::Deserialize<'de> for Coord {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Option<f64>;
                fn expecting(&self, f: &mut Formatter) -> FmtResult {
                    f.write_str("a number")
                }
                fn visit_i64<E: Error>(self, v: i64) -> Result<Self::Value, E> {
                    Ok(Some(v as f64))
                }
                fn visit_u64<E: Error>(self, v: u64) -> Result<Self::Value, E> {
                    Ok(Some(v as f64))
                }
                fn visit_f64<E: Error>(self, v: f64) -> Result<Self::Value, E> {
                    Ok(Some(v))
                }
                fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                    rest_of_list(seq)?;
                    Ok(None)
                }
                none_for!(scalars);
                none_for!(maps 'de);
            }
            d.deserialize_any(V).map(Coord)
        }
    }

    /// The name of a point's field.
    enum Key {
        X,
        Y,
        Other,
    }
    impl<'de> serde::Deserialize<'de> for Key {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl Visitor<'_> for V {
                type Value = Key;
                fn expecting(&self, f: &mut Formatter) -> FmtResult {
                    f.write_str("a field name")
                }
                fn visit_str<E: Error>(self, v: &str) -> Result<Key, E> {
                    Ok(match v {
                        "x" => Key::X,
                        "y" => Key::Y,
                        _ => Key::Other,
                    })
                }
            }
            d.deserialize_identifier(V)
        }
    }

    /// One entry of the list: a point, else none.
    struct Corner(Option<Point>);
    impl<'de> serde::Deserialize<'de> for Corner {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Option<Point>;
                fn expecting(&self, f: &mut Formatter) -> FmtResult {
                    f.write_str("a point")
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                    let x = seq.next_element::<Coord>()?.and_then(|c| c.0);
                    let y = match x {
                        Some(_) => seq.next_element::<Coord>()?.and_then(|c| c.0),
                        None => None,
                    };
                    let more = rest_of_list(seq)?;
                    Ok(x.zip(y).filter(|_| !more).map(|(x, y)| Point::new(x, y)))
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                    let (mut x, mut y, mut bad) = (None, None, false);
                    while let Some(key) = map.next_key::<Key>()? {
                        let slot = match key {
                            Key::X => &mut x,
                            Key::Y => &mut y,
                            Key::Other => {
                                map.next_value::<IgnoredAny>()?;
                                continue;
                            }
                        };
                        let v = map.next_value::<Coord>()?.0;
                        // A repeated or non-numeric coordinate makes the entry no point.
                        bad |= slot.is_some() || v.is_none();
                        *slot = slot.or(v);
                    }
                    Ok(x.zip(y).filter(|_| !bad).map(|(x, y)| Point::new(x, y)))
                }
                none_for!(scalars);
                none_for!(numbers);
            }
            d.deserialize_any(V).map(Corner)
        }
    }

    struct Corners;
    impl<'de> Visitor<'de> for Corners {
        type Value = Option<Box<[Point; 4]>>;
        fn expecting(&self, f: &mut Formatter) -> FmtResult {
            f.write_str("a list of points")
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(Corners)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut points = Vec::with_capacity(4);
            let mut malformed = false;
            while points.len() < 4 && !malformed {
                match seq.next_element::<Corner>()? {
                    Some(Corner(Some(p))) => points.push(p),
                    Some(Corner(None)) => malformed = true,
                    None => break,
                }
            }
            // A malformed or fifth entry drops the field, so the rest is skipped without decoding it.
            let extra = rest_of_list(seq)?;
            Ok(if extra || malformed { None } else { <[Point; 4]>::try_from(points).ok().map(Box::new) })
        }
        none_for!(scalars);
        none_for!(numbers);
        none_for!(maps 'de);
    }
    d.deserialize_option(Corners)
}

impl From<NodeWire> for Node {
    fn from(w: NodeWire) -> Self {
        // Corners an image cannot take are dropped, so a later transform cannot make them its own.
        let image_quad = match &w.kind {
            NodeKind::Image(im) => w.image_quad.filter(|q| crate::image_quad::check_for(im, q).is_ok()),
            _ => None,
        };
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
            flare: w.flare,
            intertwine: w.intertwine,
            intertwine_tiers: w.intertwine_tiers,
            live_paint_gaps: w.live_paint_gaps,
            objects_on_path: w.objects_on_path,
            colorize: w.colorize,
            compound_shape: w.compound_shape,
            shape_mode: w.shape_mode,
            symbol_overrides: w.symbol_overrides,
            envelope_edit_margin: w.envelope_edit_margin,
            pixel_aligned: w.pixel_aligned,
            color_mask: w.color_mask,
            warp_piece: w.warp_piece,
            clip_pieces: w.clip_pieces,
            image_quad,
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
