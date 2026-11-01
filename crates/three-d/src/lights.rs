//! Bounded directional lights, shared by all four effects and independent of the UI.
use crate::{DEFAULT_LIGHT, V3, finite};

pub const MAX_LIGHTS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub rotation: f64,
    pub height: f64,
    pub behind: bool,
    pub color: [f64; 3],
    pub intensity: f64,
    pub softness: f64,
}

impl Default for Light {
    fn default() -> Self {
        Self { rotation: DEFAULT_LIGHT.0, height: DEFAULT_LIGHT.1, behind: false, color: [1.0; 3], intensity: 0.8, softness: 0.0 }
    }
}

impl Light {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            rotation: (finite(self.rotation, DEFAULT_LIGHT.0, -1e6, 1e6) + 180.0).rem_euclid(360.0) - 180.0,
            height: finite(self.height, DEFAULT_LIGHT.1, 0.0, 90.0),
            color: self.color.map(|c| finite(c, 1.0, 0.0, 1.0)),
            intensity: finite(self.intensity, 0.8, 0.0, 1.0),
            softness: finite(self.softness, 0.0, 0.0, 1.0),
            ..self
        }
    }

    pub fn direction(self) -> [f64; 3] {
        let light = self.sanitized();
        let (sr, cr) = light.rotation.to_radians().sin_cos();
        let (sh, ch) = light.height.to_radians().sin_cos();
        [ch * cr, -ch * sr, if light.behind { -sh } else { sh }]
    }

    pub(crate) fn direct(self, normal: V3) -> f64 {
        let [x, y, z] = self.direction();
        self.intensity * ((normal.dot(V3(x, y, z)) + self.softness) / (1.0 + self.softness)).max(0.0)
    }
}

/// A fixed capacity avoids unbounded work from files or command input. Construction validates
/// the count and sanitizes every light, including callers of the portable geometry API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lights {
    entries: [Option<Light>; MAX_LIGHTS],
}

impl Lights {
    pub fn new(lights: &[Light]) -> Result<Self, String> {
        if lights.is_empty() || lights.len() > MAX_LIGHTS {
            return Err("3D effects require one to ten lights".into());
        }
        let mut entries = [None; MAX_LIGHTS];
        for (slot, light) in entries.iter_mut().zip(lights) {
            *slot = Some(light.sanitized());
        }
        Ok(Self { entries })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Light> {
        self.entries.iter().flatten()
    }
}
