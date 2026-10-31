//! The dark runs of a linear symbol, accumulated left to right.
//!
//! Encoders work in module coordinates: they walk a cursor along the symbol, writing a dark run
//! where the standard has a bar and leaving the cursor to advance where it has a space. Touching
//! runs at the same height merge as they are written, so a symbol that is one long bar at the top
//! and a broken one at the bottom does not turn into a hundred objects.

use crate::Bar;

/// One dark run, in module coordinates horizontally and millimetres vertically.
#[derive(Clone, Copy, Debug)]
struct Run {
    x: f64,
    width: f64,
    top: f64,
    height: f64,
}

/// The dark runs of a symbol being built.
#[derive(Clone, Debug, Default)]
pub struct Linear {
    runs: Vec<Run>,
    cursor: f64,
}

impl Linear {
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the next element starts, in modules.
    pub fn cursor(&self) -> f64 {
        self.cursor
    }

    /// The width of the bar pattern so far, in modules.
    pub fn width_modules(&self) -> f64 {
        self.cursor
    }

    /// A dark run of `width` modules at the cursor, from `top` millimetres down by `height`.
    pub fn bar(&mut self, width: f64, top: f64, height: f64) {
        if !(width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0) {
            return;
        }
        let run = Run { x: self.cursor, width, top, height };
        match self.runs.last_mut() {
            // Merge with the previous run when they touch and share their vertical extent.
            Some(last) if last.top == run.top && last.height == run.height && (last.x + last.width - run.x).abs() < 1e-9 => {
                last.width += run.width;
            }
            _ => self.runs.push(run),
        }
        self.cursor += width;
    }

    /// Advance the cursor by `width` modules of clear space.
    pub fn gap(&mut self, width: f64) {
        if width.is_finite() && width > 0.0 {
            self.cursor += width;
        }
    }

    /// Write one module per bit, dark where the bit is set, all of them at the same height.
    pub fn modules(&mut self, bits: &[bool], top: f64, height: f64) {
        for &dark in bits {
            if dark {
                self.bar(1.0, top, height);
            } else {
                self.gap(1.0);
            }
        }
    }

    /// Move every run by `dx_mm` millimetres right and `dy_mm` down, scaling the horizontal
    /// positions from modules to millimetres on the way.
    pub fn shift(&mut self, dx_mm: f64, dy_mm: f64, module_mm: f64) {
        for run in &mut self.runs {
            run.x = dx_mm + run.x * module_mm;
            run.width *= module_mm;
            run.top += dy_mm;
        }
    }

    /// The runs, in the order they were written.
    pub fn into_bars(self) -> Vec<Bar> {
        self.runs.into_iter().map(|r| Bar { x: r.x, y: r.top, width: r.width, height: r.height }).collect()
    }
}
