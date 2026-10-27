use super::kind::NodeKind;
use super::*;

/// Opacity mask: the luminance of the mask art sets the object's opacity (white = opaque).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpacityMask {
    /// Mask art in document coordinates.
    pub art: Arc<Node>,
    /// Clip: outside the mask art is hidden. Off: outside the mask art stays visible.
    #[serde(default = "yes")]
    pub clip: bool,
    /// Invert the mask's luminance.
    #[serde(default)]
    pub invert: bool,
    /// Disabled masks are kept but not applied.
    #[serde(default)]
    pub disabled: bool,
    /// Linked masks move with the object.
    #[serde(default = "yes")]
    pub linked: bool,
}

impl OpacityMask {
    pub fn new(art: Node, clip: bool) -> Self {
        Self { art: Arc::new(art), clip, invert: false, disabled: false, linked: true }
    }
}

/// Every paint of `n` (and its children) white, opacities kept: an alpha mask's art as an opacity
/// mask's, whose luminance over the black outside the art is then the art's alpha.
pub fn whiten(n: &mut Node) {
    let white = |p: &mut Paint| match p {
        Paint::Solid { color, swatch, tint } => {
            (*color, *swatch, *tint) = (Color::WHITE, None, 1.0);
        }
        Paint::Gradient(g) => {
            for s in &mut g.gradient.stops {
                (s.color, s.swatch, s.tint) = (Color::WHITE, None, 1.0);
            }
        }
        Paint::None | Paint::Pattern { .. } => {}
    };
    for item in &mut n.appearance.items {
        white(item.paint_mut());
    }
    match &mut n.kind {
        NodeKind::Mesh(m) => m.points.iter_mut().for_each(|p| p.color = Color::WHITE),
        NodeKind::Text(t) => t.runs.iter_mut().for_each(|r| {
            white(&mut r.style.fill);
            white(&mut r.style.stroke);
        }),
        _ => {}
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            whiten(Arc::make_mut(c));
        }
    }
}

/// Knockout Group state of a container (the Transparency panel's three-state checkbox). In a
/// knockout group each child composites against the group's backdrop, so it hides the children
/// below it instead of showing them through its transparency. Neutral passes the enclosing
/// group's setting through to the children. Files from before the three states stored a bool:
/// `true` loads as On, `false` as Neutral.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Knockout {
    #[default]
    Neutral,
    Off,
    On,
}

impl Knockout {
    pub const ALL: [Knockout; 3] = [Knockout::Neutral, Knockout::Off, Knockout::On];

    /// Name in files and command parameters.
    pub fn label(self) -> &'static str {
        match self {
            Knockout::Neutral => "neutral",
            Knockout::Off => "off",
            Knockout::On => "on",
        }
    }
    /// A name ([`Self::label`], any case) or a bool as older files and commands wrote it.
    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        match v {
            serde_json::Value::Bool(b) => Some(Self::from(*b)),
            serde_json::Value::String(s) => Self::ALL.into_iter().find(|k| k.label().eq_ignore_ascii_case(s.trim())),
            _ => None,
        }
    }
    /// The state a click on the checkbox moves to: on → neutral → off → on.
    pub fn cycle(self) -> Self {
        match self {
            Knockout::On => Knockout::Neutral,
            Knockout::Neutral => Knockout::Off,
            Knockout::Off => Knockout::On,
        }
    }
    /// Whether children of a group with this state knock each other out, inside a group (or page)
    /// where that is `enclosing`.
    pub fn resolve(self, enclosing: bool) -> bool {
        match self {
            Knockout::Neutral => enclosing,
            Knockout::Off => false,
            Knockout::On => true,
        }
    }
}

impl From<bool> for Knockout {
    fn from(b: bool) -> Self {
        if b { Knockout::On } else { Knockout::Neutral }
    }
}

impl Serialize for Knockout {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.label())
    }
}

impl<'de> Deserialize<'de> for Knockout {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Self::from_value(&v).ok_or_else(|| serde::de::Error::custom(format!("invalid knockout {v}")))
    }
}
