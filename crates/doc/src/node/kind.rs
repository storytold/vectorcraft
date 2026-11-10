use super::ids::LayerColor;
use super::live_shape::LiveShape;
use super::{Affine, Arc, BlendSpec, Deserialize, EnvelopeKind, FillRule, GradientMesh, Node, PathData, RepeatSpec, Serialize, TextObject, yes};

/// Embedded or linked raster image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageObject {
    /// Key into [`crate::Document::images`].
    pub key: String,
    pub width: u32,
    pub height: u32,
    /// Maps pixel space (0..w, 0..h) into the document.
    pub xf: Affine,
    /// The file a linked image shows (File → Place with Link); `None`: embedded. Files saved
    /// before links had details hold just the path.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::links::de_link")]
    pub link: Option<crate::LinkInfo>,
    /// Links panel → Placement Options: how a file read again takes this image's place.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub placement: crate::links::PlacementOptions,
    /// The pixels of an image in art made outside a document (type's outlines, a glyph's art),
    /// which has no [`crate::Document::images`] to keep them in. A document takes them in
    /// ([`crate::Document::adopt_pixels`]): no image in one holds its own.
    #[serde(skip)]
    pub pixels: Option<Arc<crate::ImageBlob>>,
}

/// Written with its `type` first, then its fields (an internally tagged enum); read by
/// [`kind_de`], which takes the fields straight from the file when `type` comes first.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum NodeKind {
    /// A top-level layer or sublayer.
    Layer {
        color: LayerColor,
        #[serde(default)]
        template: bool,
        #[serde(default = "yes")]
        printable: bool,
        children: Vec<Arc<Node>>,
        /// Layer clipping mask: the first (bottom-most) child clips the others, as in a clip group.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        clip: bool,
        /// Layer Options → Preview: off, the layer's art is drawn (and clicked) in outline on screen
        /// (Ctrl-click its eye in the Layers panel).
        #[serde(default = "yes", skip_serializing_if = "crate::skip::is_true")]
        preview: bool,
        /// Layer Options → Dim Images to: on screen, images on the layer show faded to this
        /// percentage (0–100); `None`: not dimmed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dim_images: Option<u8>,
    },
    /// A group. With `clip`, the first (bottom-most) child is the clipping path.
    Group {
        children: Vec<Arc<Node>>,
        #[serde(default)]
        clip: bool,
    },
    Path {
        path: PathData,
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        rule: FillRule,
        /// Boxed: few paths are live shapes, and the shape is larger than the rest of the variant.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live: Option<Box<LiveShape>>,
        /// This path acts as a clipping path inside a clip group.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        clipping: bool,
        /// Guide (non-printing) path.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        guide: bool,
    },
    /// Compound path: children are paths painted as one with the compound's appearance.
    Compound {
        children: Vec<Arc<Node>>,
        #[serde(default)]
        rule: FillRule,
    },
    Text(Box<TextObject>),
    /// Boxed, as the other large kinds: a node is as large as its largest kind.
    Image(Box<ImageObject>),
    SymbolInstance {
        symbol: String,
        xf: Affine,
    },
    /// Live blend: the key objects (paint order) plus spacing/orientation/spine; the intermediate
    /// steps are evaluated on demand (`live::blend_expand`).
    Blend {
        children: Vec<Arc<Node>>,
        #[serde(default)]
        spec: Box<BlendSpec>,
    },
    /// Live envelope distortion of `content`.
    Envelope {
        content: Vec<Arc<Node>>,
        kind: Box<EnvelopeKind>,
        /// Envelope Options → Fidelity (0–100).
        #[serde(default = "crate::live::default_fidelity")]
        fidelity: f64,
        /// Edit Contents mode (the content, not the envelope, is edited).
        #[serde(default)]
        editing: bool,
        /// Envelope Options besides Fidelity.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        options: crate::live::EnvelopeOptions,
        /// The envelope's own axes (→ the document): the transforms it took since it was made that
        /// the page axes can't stand for (rotation, shear, reflection). The content maps from its
        /// bounds in this frame, and a warp bends along it.
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        frame: Box<Affine>,
    },
    /// Gradient mesh.
    Mesh(GradientMesh),
    /// Live Repeat (radial / grid / mirror) of source art.
    Repeat(RepeatSpec),
    /// A placed document: an artboard of another VectorCraft file, or a page of a PDF or `.ai`
    /// file, linked and locked (see [`crate::placed_document`]).
    PlacedDocument(Box<crate::placed_document::PlacedDocument>),
    /// Object › Intertwine: the objects keep their own stacking order, except where one of them
    /// is drawn over the others: the area [`NodeExtras::intertwine`] of that member (see
    /// [`crate::intertwine`]).
    Intertwine {
        children: Vec<Arc<Node>>,
    },
}

/// Reading a [`NodeKind`]. Derived, an internally tagged enum buffers each kind (with every
/// object inside a group, again at each level of nesting) before reading it, since `type` may come
/// anywhere among its fields. Files have it first (it is written first): then the kind is read
/// straight from the file. Otherwise (hand-written files, values whose keys are sorted) the
/// fields before `type` are buffered, and only those.
pub use kind_de::{MAX_BUFFERED_VALUES, with_buffered_value_budget};

mod kind_de {
    use std::fmt;
    use std::sync::Arc;

    use serde::de::value::{MapAccessDeserializer, StringDeserializer};
    use serde::de::{self, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, VariantAccess, Visitor};
    use serde::{Deserialize, forward_to_deserialize_any};
    use vectorcraft_geom::{Affine, FillRule, PathData};

    use super::{LayerColor, LiveShape, Node, NodeKind, yes};
    use crate::live::{BlendSpec, EnvelopeKind, GradientMesh};
    use crate::pattern::RepeatSpec;
    use crate::text::TextObject;

    /// The kinds and their fields as [`NodeKind`] has them, with the defaults its field attributes
    /// note (the compiler checks that every field is here), read externally tagged: the kind from
    /// `type`, its fields from the rest of the object ([`Tagged`]).
    #[derive(Deserialize)]
    #[serde(remote = "NodeKind", rename_all = "lowercase")]
    enum KindDef {
        Layer {
            color: LayerColor,
            #[serde(default)]
            template: bool,
            #[serde(default = "yes")]
            printable: bool,
            children: Vec<Arc<Node>>,
            #[serde(default)]
            clip: bool,
            #[serde(default = "yes")]
            preview: bool,
            #[serde(default)]
            dim_images: Option<u8>,
        },
        Group {
            children: Vec<Arc<Node>>,
            #[serde(default)]
            clip: bool,
        },
        Path {
            path: PathData,
            #[serde(default)]
            rule: FillRule,
            #[serde(default)]
            live: Option<Box<LiveShape>>,
            #[serde(default)]
            clipping: bool,
            #[serde(default)]
            guide: bool,
        },
        Compound {
            children: Vec<Arc<Node>>,
            #[serde(default)]
            rule: FillRule,
        },
        Text(Box<TextObject>),
        Image(Box<super::ImageObject>),
        SymbolInstance {
            symbol: String,
            xf: Affine,
        },
        Blend {
            children: Vec<Arc<Node>>,
            #[serde(default)]
            spec: Box<BlendSpec>,
        },
        Envelope {
            content: Vec<Arc<Node>>,
            kind: Box<EnvelopeKind>,
            #[serde(default = "crate::live::default_fidelity")]
            fidelity: f64,
            #[serde(default)]
            editing: bool,
            #[serde(default)]
            options: crate::live::EnvelopeOptions,
            #[serde(default)]
            frame: Box<Affine>,
        },
        Mesh(GradientMesh),
        Repeat(RepeatSpec),
        PlacedDocument(Box<crate::placed_document::PlacedDocument>),
        Intertwine {
            children: Vec<Arc<Node>>,
        },
    }

    impl<'de> Deserialize<'de> for NodeKind {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_map(KindVisitor)
        }
    }

    struct KindVisitor;

    impl<'de> Visitor<'de> for KindVisitor {
        type Value = NodeKind;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an object kind (a map with `type`)")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<NodeKind, A::Error> {
            let mut before = vec![];
            while let Some(key) = map.next_key::<String>()? {
                if key == "type" {
                    let tag: String = map.next_value()?;
                    return KindDef::deserialize(Tagged { tag, map: Buffered { before: before.into_iter(), value: None, map } });
                }
                before.push((key, map.next_value::<Counted>()?.0));
            }
            Err(de::Error::missing_field("type"))
        }
    }

    /// Most JSON values (each number, string, array, object…) the fields read before an object's
    /// `type` may hold together while [`with_buffered_value_budget`] runs: 32M (about 1 GB as
    /// `serde_json::Value`s, some 32 bytes each), 4M in the web build. VectorCraft writes `type`
    /// first, so its files buffer nothing; a reordered file buffers what comes before it, and
    /// a crafted one could fill memory with values two bytes each in the file before any object in
    /// them is counted (#3587).
    pub const MAX_BUFFERED_VALUES: usize = if cfg!(target_arch = "wasm32") { 4 << 20 } else { 32 << 20 };

    std::thread_local! {
        /// Values the fields buffered on this thread may still hold, and the budget they come from.
        static VALUES_LEFT: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
        /// Buffered fields being read back: what they hold was counted when it was buffered.
        static REPLAYING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// Run `f` (which reads objects with serde) letting the fields it buffers before an object's
    /// `type` hold at most `max` values together; buffering more fails.
    pub fn with_buffered_value_budget<T>(max: usize, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<(usize, usize)>);
        impl Drop for Restore {
            fn drop(&mut self) {
                VALUES_LEFT.set(self.0);
            }
        }
        let _restore = Restore(VALUES_LEFT.replace(Some((max, max))));
        f()
    }

    /// While buffered fields are read back ([`REPLAYING`]).
    struct Replaying;

    impl Replaying {
        fn enter() -> Self {
            REPLAYING.set(REPLAYING.get().saturating_add(1));
            Replaying
        }
    }

    impl Drop for Replaying {
        fn drop(&mut self) {
            REPLAYING.set(REPLAYING.get().saturating_sub(1));
        }
    }

    /// Count one buffered value against [`with_buffered_value_budget`].
    fn count<E: de::Error>() -> Result<(), E> {
        if REPLAYING.get() > 0 {
            return Ok(());
        }
        if let Some((left, max)) = VALUES_LEFT.get() {
            let left = left.checked_sub(1).ok_or_else(|| E::custom(format!("an object has more than {max} values before its type")))?;
            VALUES_LEFT.set(Some((left, max)));
        }
        Ok(())
    }

    /// A field read before `type`, as a `serde_json::Value` counted value by value as it is read,
    /// so the budget stops a large one before it is all in memory.
    struct Counted(serde_json::Value);

    impl<'de> Deserialize<'de> for Counted {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_any(CountedVisitor)
        }
    }

    struct CountedVisitor;

    impl<'de> Visitor<'de> for CountedVisitor {
        type Value = Counted;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a JSON value")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Counted, E> {
            count()?;
            Ok(Counted(serde_json::Value::Bool(v)))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Counted, E> {
            count()?;
            Ok(Counted(v.into()))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Counted, E> {
            count()?;
            Ok(Counted(v.into()))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Counted, E> {
            count()?;
            Ok(Counted(serde_json::Number::from_f64(v).map_or(serde_json::Value::Null, serde_json::Value::Number)))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Counted, E> {
            count()?;
            Ok(Counted(serde_json::Value::String(v.to_owned())))
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Counted, E> {
            count()?;
            Ok(Counted(serde_json::Value::String(v)))
        }

        fn visit_none<E: de::Error>(self) -> Result<Counted, E> {
            self.visit_unit()
        }

        fn visit_unit<E: de::Error>(self) -> Result<Counted, E> {
            count()?;
            Ok(Counted(serde_json::Value::Null))
        }

        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Counted, D::Error> {
            Counted::deserialize(d)
        }

        fn visit_seq<S: de::SeqAccess<'de>>(self, mut seq: S) -> Result<Counted, S::Error> {
            count()?;
            let mut out = Vec::new();
            while let Some(Counted(v)) = seq.next_element()? {
                out.push(v);
            }
            Ok(Counted(serde_json::Value::Array(out)))
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Counted, M::Error> {
            count()?;
            let mut out = serde_json::Map::new();
            while let Some(key) = map.next_key::<String>()? {
                let Counted(v) = map.next_value()?;
                out.insert(key, v);
            }
            Ok(Counted(serde_json::Value::Object(out)))
        }
    }

    /// The fields read before `type` (buffered), then the rest of the object.
    struct Buffered<A> {
        before: std::vec::IntoIter<(String, serde_json::Value)>,
        value: Option<serde_json::Value>,
        map: A,
    }

    impl<'de, A: MapAccess<'de>> MapAccess<'de> for Buffered<A> {
        type Error = A::Error;

        fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error> {
            match self.before.next() {
                Some((key, value)) => {
                    self.value = Some(value);
                    seed.deserialize(StringDeserializer::<A::Error>::new(key)).map(Some)
                }
                None => self.map.next_key_seed(seed),
            }
        }

        fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
            match self.value.take() {
                Some(value) => {
                    // Counted when it was read: the objects it holds buffer it again as they read it.
                    let _replaying = Replaying::enter();
                    seed.deserialize(value).map_err(de::Error::custom)
                }
                None => self.map.next_value_seed(seed),
            }
        }
    }

    /// An object whose `type` was read: the kind is `tag`, its fields are the rest of `map`.
    struct Tagged<A> {
        tag: String,
        map: A,
    }

    impl<'de, A: MapAccess<'de>> Deserializer<'de> for Tagged<A> {
        type Error = A::Error;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, A::Error> {
            visitor.visit_enum(self)
        }

        forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
            option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
            identifier ignored_any
        }
    }

    impl<'de, A: MapAccess<'de>> EnumAccess<'de> for Tagged<A> {
        type Error = A::Error;
        type Variant = Fields<A>;

        fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Fields<A>), A::Error> {
            let kind = seed.deserialize(self.tag.into_deserializer())?;
            Ok((kind, Fields(self.map)))
        }
    }

    /// A kind's fields: the rest of the object.
    struct Fields<A>(A);

    impl<'de, A: MapAccess<'de>> VariantAccess<'de> for Fields<A> {
        type Error = A::Error;

        fn unit_variant(self) -> Result<(), A::Error> {
            Err(de::Error::invalid_type(de::Unexpected::UnitVariant, &"an object kind with fields"))
        }

        fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, A::Error> {
            seed.deserialize(MapAccessDeserializer::new(self.0))
        }

        fn tuple_variant<V: Visitor<'de>>(self, _: usize, _: V) -> Result<V::Value, A::Error> {
            Err(de::Error::invalid_type(de::Unexpected::TupleVariant, &"an object kind with fields"))
        }

        fn struct_variant<V: Visitor<'de>>(self, _: &'static [&'static str], visitor: V) -> Result<V::Value, A::Error> {
            visitor.visit_map(self.0)
        }
    }
}
