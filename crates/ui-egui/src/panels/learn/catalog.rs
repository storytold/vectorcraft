//! The Learn panel's tutorials, VectorCraft's own short lessons, grouped by skill. The text is
//! English data: the panel shows it as written in every interface language.

/// One step of a tutorial: what to do, and the tool it chooses or the command it runs, if any.
pub struct Step {
    pub text: &'static str,
    /// A tool id ([`vectorcraft_tools::catalog::tool_info`]) the step names.
    pub tool: Option<&'static str>,
    /// A command id the step names.
    pub command: Option<&'static str>,
}

/// A tutorial: an id for `learn.show`, its title and its numbered steps.
pub struct Tutorial {
    pub id: &'static str,
    pub title: &'static str,
    pub steps: &'static [Step],
}

/// A skill and its tutorials, in the order the panel lists them.
pub struct Skill {
    pub id: &'static str,
    pub title: &'static str,
    pub tutorials: &'static [Tutorial],
}

/// A step that names no tool or command.
const fn say(text: &'static str) -> Step {
    Step { text, tool: None, command: None }
}

/// A step that uses tool `id`.
const fn tool(text: &'static str, id: &'static str) -> Step {
    Step { text, tool: Some(id), command: None }
}

/// A step that runs command `id`.
const fn command(text: &'static str, id: &'static str) -> Step {
    Step { text, tool: None, command: Some(id) }
}

/// Every skill, in the order the Skills tab lists them.
pub const SKILLS: &[Skill] = &[
    Skill {
        id: "essentials",
        title: "Best of VectorCraft",
        tutorials: &[
            Tutorial {
                id: "draw-and-color",
                title: "Draw and color a shape",
                steps: &[
                    tool("Choose the Rectangle tool in the toolbar.", "rectangle"),
                    say("Drag on the artboard to draw a rectangle."),
                    say("With the rectangle still selected, click a color in the Swatches panel to fill it."),
                    command("Click the Stroke box under the tools, then click another swatch to color the outline.", "paint.toggleActive"),
                ],
            },
            Tutorial {
                id: "combine-shapes",
                title: "Combine shapes into one",
                steps: &[
                    tool("Choose the Ellipse tool and draw two circles that overlap.", "ellipse"),
                    tool("Choose the Selection tool and drag across both circles to select them.", "selection"),
                    tool("Choose the Shape Builder tool and drag across the parts you want to merge.", "shapeBuilder"),
                ],
            },
            Tutorial {
                id: "add-headline",
                title: "Add a headline",
                steps: &[
                    tool("Choose the Type tool and click on the artboard.", "type"),
                    say("Type a few words, then press Escape to finish."),
                    say("Pick a font and a size in the Character panel."),
                ],
            },
        ],
    },
    Skill {
        id: "draw",
        title: "Draw and build",
        tutorials: &[
            Tutorial {
                id: "pen-paths",
                title: "Draw paths with the Pen",
                steps: &[
                    tool("Choose the Pen tool.", "pen"),
                    say("Click to place corner points, or drag to pull out a curve."),
                    say("Click the first point again to close the shape, or press Enter to leave it open."),
                ],
            },
            Tutorial {
                id: "smooth-curves",
                title: "Draw smooth curves",
                steps: &[
                    tool("Choose the Curvature tool.", "curvature"),
                    say("Click to place points: the path bends smoothly through each one."),
                    tool("Choose the Direct Selection tool and drag a point to reshape the curve.", "directSelection"),
                ],
            },
            Tutorial {
                id: "pencil-sketch",
                title: "Sketch with the Pencil",
                steps: &[
                    tool("Choose the Pencil tool.", "pencil"),
                    say("Drag to draw a freehand line."),
                    tool("Choose the Smooth tool and drag along the line to even it out.", "smooth"),
                ],
            },
        ],
    },
    Skill {
        id: "color",
        title: "Apply color",
        tutorials: &[
            Tutorial {
                id: "fill-and-stroke",
                title: "Fill and stroke",
                steps: &[
                    tool("Select an object with the Selection tool.", "selection"),
                    say("Click the Fill box under the tools, then pick a color in the Color panel."),
                    command("Click the double arrow beside the boxes to swap the fill and the stroke.", "paint.swap"),
                ],
            },
            Tutorial {
                id: "gradient",
                title: "Add a gradient",
                steps: &[
                    tool("Select an object with the Selection tool.", "selection"),
                    tool("Choose the Gradient tool and click the object to give it a gradient.", "gradient"),
                    say("Drag across the object to set the gradient's direction and length."),
                ],
            },
            Tutorial {
                id: "copy-colors",
                title: "Copy colors from other art",
                steps: &[
                    tool("Select the object to color with the Selection tool.", "selection"),
                    tool("Choose the Eyedropper tool and click the art whose colors you want.", "eyedropper"),
                ],
            },
        ],
    },
    Skill {
        id: "type",
        title: "Set type",
        tutorials: &[
            Tutorial {
                id: "area-type",
                title: "Flow text into a shape",
                steps: &[
                    tool("Draw a rectangle for the text to fill.", "rectangle"),
                    tool("Choose the Area Type tool and click the rectangle's edge.", "areaType"),
                    say("Type or paste your text: it wraps inside the shape."),
                ],
            },
            Tutorial {
                id: "type-on-path",
                title: "Type along a path",
                steps: &[
                    tool("Draw a curved path with the Pen tool.", "pen"),
                    tool("Choose the Type on a Path tool and click the path.", "typeOnPath"),
                    say("Type your text: it follows the path."),
                ],
            },
            Tutorial {
                id: "outline-type",
                title: "Turn type into shapes",
                steps: &[
                    tool("Select a text object with the Selection tool.", "selection"),
                    command("Choose Type › Create Outlines to turn its letters into paths.", "type.createOutlines"),
                ],
            },
        ],
    },
];

/// The skill whose id is `id`.
pub fn skill(id: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|s| s.id == id)
}

/// The tutorial whose id is `id`, with its skill.
pub fn tutorial(id: &str) -> Option<(&'static Skill, &'static Tutorial)> {
    SKILLS.iter().find_map(|s| s.tutorials.iter().find(|t| t.id == id).map(|t| (s, t)))
}
