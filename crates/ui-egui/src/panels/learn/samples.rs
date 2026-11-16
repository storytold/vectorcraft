//! The practice samples that a tutorial's View more opens: one small original SVG drawing for each
//! tutorial, with only the art its steps work on. Each one opens as a new untitled document.

/// The largest sample, in bytes: the catalog test keeps every sample under it.
pub const MAX_SAMPLE_BYTES: usize = 16 * 1024;

/// Every tutorial's sample, by tutorial id, in the catalog's order.
pub const SAMPLES: &[(&str, &str)] = &[
    (
        "draw-and-color",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <rect x="100" y="75" width="200" height="150" fill="none" stroke="#9aa0a6" stroke-width="2" stroke-dasharray="8 6"/>
</svg>
"##,
    ),
    (
        "combine-shapes",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <circle cx="160" cy="150" r="80" fill="#4c8bf5"/>
  <circle cx="250" cy="150" r="80" fill="#f4b400"/>
</svg>
"##,
    ),
    (
        "add-headline",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <rect x="40" y="90" width="320" height="120" rx="12" fill="#e8f0fe"/>
</svg>
"##,
    ),
    (
        "pen-paths",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <path d="M 200 60 L 320 240 L 80 240 Z" fill="none" stroke="#9aa0a6" stroke-width="2" stroke-dasharray="8 6"/>
  <circle cx="200" cy="60" r="5" fill="#1a73e8"/>
  <circle cx="320" cy="240" r="5" fill="#1a73e8"/>
  <circle cx="80" cy="240" r="5" fill="#1a73e8"/>
</svg>
"##,
    ),
    (
        "smooth-curves",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <path d="M 40 150 C 100 50 160 50 200 150 C 240 250 300 250 360 150" fill="none" stroke="#9aa0a6" stroke-width="2" stroke-dasharray="8 6"/>
</svg>
"##,
    ),
    (
        "pencil-sketch",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <path d="M 60 200 C 100 80 180 80 200 150 C 220 220 300 220 340 100" fill="none" stroke="#9aa0a6" stroke-width="2" stroke-dasharray="8 6"/>
</svg>
"##,
    ),
    (
        "fill-and-stroke",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <rect x="110" y="80" width="180" height="140" rx="16" fill="#f4b400" stroke="#1a73e8" stroke-width="6"/>
</svg>
"##,
    ),
    (
        "gradient",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <circle cx="200" cy="150" r="100" fill="#34a853"/>
</svg>
"##,
    ),
    (
        "copy-colors",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <rect x="50" y="90" width="120" height="120" fill="#db4437" stroke="#202124" stroke-width="4"/>
  <circle cx="290" cy="150" r="60" fill="#dadce0"/>
</svg>
"##,
    ),
    (
        "area-type",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <rect x="80" y="60" width="240" height="180" fill="none" stroke="#1a73e8" stroke-width="2"/>
</svg>
"##,
    ),
    (
        "type-on-path",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <path d="M 50 200 C 120 80 280 80 350 200" fill="none" stroke="#1a73e8" stroke-width="2"/>
</svg>
"##,
    ),
    (
        "outline-type",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <text x="60" y="170" font-size="64" fill="#202124">Outline</text>
</svg>
"##,
    ),
];

/// The sample of the tutorial whose id is `id`.
pub fn sample(id: &str) -> Option<&'static str> {
    SAMPLES.iter().find(|(tutorial, _)| *tutorial == id).map(|(_, svg)| *svg)
}
