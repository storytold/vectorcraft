//! Raster (painted) effects: descriptions for the renderer.

use serde_json::Value;
use vectorcraft_doc::Effect;
use vectorcraft_doc::color::{BlendMode, Color};

use vectorcraft_geom::Rect;

use crate::merged_params;
use crate::pixel::{self, PixelFx};
use crate::util::*;

/// A painted effect, in document units. `blur` is Illustrator's blur distance; the renderer uses
/// a Gaussian with σ = blur / 2 (so the visible spread is about 1.5 × blur).
#[derive(Clone, Debug, PartialEq)]
pub enum RasterFx {
    DropShadow {
        mode: BlendMode,
        opacity: f32,
        dx: f64,
        dy: f64,
        blur: f64,
        color: Color,
    },
    OuterGlow {
        mode: BlendMode,
        opacity: f32,
        blur: f64,
        color: Color,
    },
    InnerGlow {
        mode: BlendMode,
        opacity: f32,
        blur: f64,
        color: Color,
        center: bool,
    },
    /// Soften the object's edges inward over `radius`.
    Feather {
        radius: f64,
    },
    /// Blur the whole object (σ = radius / 2).
    GaussianBlur {
        radius: f64,
    },
    /// A Photoshop-style filter over the object's pixels (Radial Blur, Unsharp Mask…).
    Pixel(PixelFx),
}

impl RasterFx {
    /// Painted below the object (shadows, outer glows)?
    pub fn is_below(&self) -> bool {
        matches!(self, RasterFx::DropShadow { .. } | RasterFx::OuterGlow { .. })
    }
    /// The blend mode it paints with (shadows and glows).
    pub fn mode(&self) -> Option<BlendMode> {
        match self {
            RasterFx::DropShadow { mode, .. } | RasterFx::OuterGlow { mode, .. } | RasterFx::InnerGlow { mode, .. } => Some(*mode),
            _ => None,
        }
    }
    /// How far this effect reaches beyond `bounds`, what it paints around (Radial Blur reaches
    /// further around larger objects).
    pub fn outset(&self, bounds: Rect) -> f64 {
        match self {
            RasterFx::DropShadow { dx, dy, blur, .. } => dx.abs().max(dy.abs()) + 1.5 * blur,
            RasterFx::OuterGlow { blur, .. } => 1.5 * blur,
            RasterFx::InnerGlow { .. } | RasterFx::Feather { .. } => 0.0,
            RasterFx::GaussianBlur { radius } => 1.5 * radius,
            RasterFx::Pixel(p) => p.outset(bounds),
        }
    }
}

/// Parse a blend mode name (`"multiply"`, `"Screen"`, `"color-burn"`…).
pub fn blend_mode(s: &str) -> BlendMode {
    let key: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    BlendMode::ALL.iter().copied().find(|m| format!("{m:?}").to_ascii_lowercase() == key).unwrap_or(BlendMode::Normal)
}

/// An effect's `color` parameter (`"#rrggbb"`, `[r, g, b]` 0..1 or a colour object).
pub(crate) fn color_param(p: &Value, default: Color) -> Color {
    match p.get("color") {
        Some(Value::String(s)) => Color::from_hex(s).unwrap_or(default),
        Some(Value::Array(a)) if a.len() >= 3 => {
            let f = |i: usize| a[i].as_f64().unwrap_or(0.0) as f32;
            Color::rgb(f(0), f(1), f(2))
        }
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).unwrap_or(default),
        _ => default,
    }
}

fn opacity(p: &Value) -> f32 {
    (num(p, "opacity", 75.0) / 100.0).clamp(0.0, 1.0) as f32
}

/// The visible raster effects of `effects`, in stack order.
pub fn raster_effects(effects: &[Effect]) -> Vec<RasterFx> {
    effects
        .iter()
        .filter(|e| e.visible)
        .filter_map(|e| {
            let p = merged_params(&e.id, &e.params);
            let blur = num(&p, "blur", 5.0).clamp(0.0, 1000.0);
            Some(match e.id.as_str() {
                "stylize.dropShadow" => RasterFx::DropShadow {
                    mode: blend_mode(text(&p, "mode", "multiply")),
                    opacity: opacity(&p),
                    dx: num(&p, "x", 7.0).clamp(-1e4, 1e4),
                    dy: num(&p, "y", 7.0).clamp(-1e4, 1e4),
                    blur,
                    color: color_param(&p, Color::BLACK),
                },
                "stylize.outerGlow" => RasterFx::OuterGlow {
                    mode: blend_mode(text(&p, "mode", "screen")),
                    opacity: opacity(&p),
                    blur,
                    color: color_param(&p, Color::rgb(1.0, 1.0, 0.0)),
                },
                "stylize.innerGlow" => RasterFx::InnerGlow {
                    mode: blend_mode(text(&p, "mode", "screen")),
                    opacity: opacity(&p),
                    blur,
                    color: color_param(&p, Color::WHITE),
                    center: text(&p, "source", "edge").eq_ignore_ascii_case("center"),
                },
                "stylize.feather" => RasterFx::Feather { radius: num(&p, "radius", 5.0).clamp(0.0, 1000.0) },
                "blur.gaussian" => RasterFx::GaussianBlur { radius: num(&p, "radius", 5.0).clamp(0.0, 1000.0) },
                id => RasterFx::Pixel(pixel::parse(id, &p)?),
            })
        })
        .collect()
}

/// How far the visible raster effects of `effects` paint beyond `bounds`, the (effected)
/// geometry's, in document units. Geometry effects are accounted for by evaluating them.
pub fn outset(effects: &[Effect], bounds: Rect) -> f64 {
    raster_effects(effects).iter().map(|fx| fx.outset(bounds)).fold(0.0, f64::max)
}
