use super::{Appearance, Deserialize, Knockout, Node, Rect, Serialize, skip};

/// A named graphic style (Graphic Styles panel): an appearance plus the object's transparency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphicStyle {
    pub name: String,
    pub appearance: Appearance,
    /// Stable id linked objects refer to ([`Node::graphic_style`]); 0 until first needed (styles
    /// from older files get one when an object is first linked to them).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub id: u32,
    #[serde(default = "one", skip_serializing_if = "skip::is_one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub blend: vectorcraft_color::BlendMode,
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub isolate: bool,
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub knockout: Knockout,
    /// Placed gradients are stored relative to the unit box (0, 0)–(1, 1), so applying the style
    /// places them on each object's own bounds. Styles saved before this kept document
    /// coordinates (false).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unit_box: bool,
}

fn one() -> f32 {
    1.0
}

/// The name of the style new documents list first (Sort by Name keeps it first).
pub const DEFAULT_GRAPHIC_STYLE: &str = "Default Graphic Style";

impl GraphicStyle {
    /// The box a style's placed gradients are stored relative to.
    pub const UNIT_BOX: Rect = Rect::new(0.0, 0.0, 1.0, 1.0);

    /// A style with `appearance` (its placed gradients, if any, in unit-box space) and default
    /// transparency (no id yet).
    pub fn new(name: impl Into<String>, appearance: Appearance) -> Self {
        Self {
            name: name.into(),
            appearance,
            id: 0,
            opacity: 1.0,
            blend: Default::default(),
            isolate: false,
            knockout: Knockout::Neutral,
            unit_box: true,
        }
    }
    /// A style capturing `n`'s transparency and `appearance` (in unit-box space).
    pub fn of(name: impl Into<String>, appearance: Appearance, n: &Node) -> Self {
        Self { opacity: n.opacity, blend: n.blend, isolate: n.isolate, knockout: n.knockout, ..Self::new(name, appearance) }
    }
    /// The style's appearance as object `n` takes it: placed gradients stored in unit-box space
    /// land at the same place relative to `n`'s geometric bounds.
    pub fn appearance_on(&self, n: &Node) -> std::borrow::Cow<'_, Appearance> {
        let placed = self.unit_box && self.appearance.has_placed_gradient();
        match n.geometric_bounds().filter(|_| placed) {
            Some(b) => {
                let mut ap = self.appearance.clone();
                ap.rebase_gradients(Self::UNIT_BOX, b);
                std::borrow::Cow::Owned(ap)
            }
            None => std::borrow::Cow::Borrowed(&self.appearance),
        }
    }
    /// Give `n` this style's transparency.
    pub fn apply_transparency(&self, n: &mut Node) {
        (n.opacity, n.blend, n.isolate, n.knockout) = (self.opacity, self.blend, self.isolate, self.knockout);
    }
    /// Does `n` have this style's transparency?
    pub fn transparency_matches(&self, n: &Node) -> bool {
        (n.opacity, n.blend, n.isolate, n.knockout) == (self.opacity, self.blend, self.isolate, self.knockout)
    }
}

pub(crate) fn default_graphic_styles() -> Vec<GraphicStyle> {
    use vectorcraft_color::{Color, Paint};
    let solid = |hex| Paint::solid(Color::from_hex(hex).unwrap_or(Color::BLACK));
    [
        GraphicStyle::new(DEFAULT_GRAPHIC_STYLE, Appearance::default_art()),
        GraphicStyle::new("Black Outline", Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0)),
        GraphicStyle::new("Heavy Ink", Appearance::basic(solid("#1b1464"), Paint::solid(Color::BLACK), 4.0)),
        GraphicStyle::new("Sunshine", Appearance::basic(solid("#fbb03b"), solid("#f15a24"), 2.0)),
    ]
    .into_iter()
    .zip(1..)
    .map(|(g, id)| GraphicStyle { id, ..g })
    .collect()
}
