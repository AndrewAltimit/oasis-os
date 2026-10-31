//! SVG parsing and rendering for inline SVG elements.
//!
//! Supports: `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<text>`,
//! `<path>`, `<polygon>`, `<polyline>`, and `<g>` groups with nested
//! transform composition. `<defs>` with `<linearGradient>`,
//! `<radialGradient>`, and `<pattern>` definitions resolved via
//! `fill="url(#id)"` / `stroke="url(#id)"` references. Presentation
//! attribute inheritance from `<g>` groups (fill, stroke, stroke-width,
//! etc.). `<text>` with `text-anchor`, `letter-spacing`, `font-weight`,
//! `opacity`, and `<tspan>` children.
//!
//! Layout: this module holds the parsed-tree types; `parse` builds
//! them from the DOM, `path` turns path data into polylines, and
//! `paint` draws them.

use std::collections::HashMap;

use crate::html::dom::{Document, NodeId};
use crate::transform::AffineTransform2D;
use oasis_types::backend::Color;

mod paint;
mod parse;
mod path;
#[cfg(test)]
mod tests;

pub use paint::paint_svg;
pub use parse::{parse_svg, parse_svg_with_styles};
// Canvas 2D shares the path point budget, and (behind `javascript` +
// `canvas`) the colour parser and the Bezier flattening.
pub(crate) use path::MAX_PATH_POINTS;
#[cfg_attr(
    not(all(feature = "javascript", feature = "canvas")),
    allow(unused_imports)
)]
pub(crate) use {
    parse::parse_svg_color,
    path::{flatten_cubic, flatten_quad},
};
// The submodules see each other's `pub(super)` items through these.
#[allow(unused_imports)]
use {paint::*, parse::*, path::*};

type AffineTransform = AffineTransform2D;

/// SVG fill rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// SVG stroke line cap style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// SVG stroke line join style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

// -------------------------------------------------------------------
// SVG paint (fill/stroke value with url() reference support)
// -------------------------------------------------------------------

/// An SVG paint value: solid color, gradient/pattern reference, or none.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SvgPaint {
    /// No paint (SVG `none`).
    #[default]
    None,
    /// Solid color.
    Color(Color),
    /// A `url(#id)` reference into `<defs>`: a `<linearGradient>` /
    /// `<radialGradient>`, or else a `<pattern>`, with that id.
    GradientRef(String),
}

impl SvgPaint {
    /// Extract a solid color, returning `None` for references and `None` paint.
    pub fn as_color(&self) -> Option<Color> {
        match self {
            SvgPaint::Color(c) => Some(*c),
            _ => Option::None,
        }
    }
}

// -------------------------------------------------------------------
// SVG gradient and pattern definitions
// -------------------------------------------------------------------

/// A single gradient color stop.
#[derive(Debug, Clone)]
pub struct SvgGradientStop {
    /// Position along the gradient axis (0.0..=1.0).
    pub offset: f32,
    /// Stop color with stop-opacity baked into the alpha channel.
    pub color: Color,
}

/// A gradient definition from `<defs>`.
#[derive(Debug, Clone)]
pub enum SvgGradientDef {
    Linear {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stops: Vec<SvgGradientStop>,
        /// True = `gradientUnits="userSpaceOnUse"`.
        user_space: bool,
    },
    Radial {
        cx: f32,
        cy: f32,
        r: f32,
        stops: Vec<SvgGradientStop>,
        /// True = `gradientUnits="userSpaceOnUse"`.
        user_space: bool,
    },
}

/// A pattern definition from `<defs>`.
#[derive(Debug, Clone)]
pub struct SvgPatternDef {
    pub width: f32,
    pub height: f32,
    pub shapes: Vec<SvgShape>,
}

/// Collected definitions from `<defs>` elements.
#[derive(Debug, Clone, Default)]
pub struct SvgDefs {
    pub gradients: HashMap<String, SvgGradientDef>,
    pub patterns: HashMap<String, SvgPatternDef>,
}

// -------------------------------------------------------------------
// Attribute inheritance context
// -------------------------------------------------------------------

/// Presentation attributes inherited from ancestor `<g>` elements.
#[derive(Debug, Clone, Default)]
struct InheritedAttrs {
    fill: Option<SvgPaint>,
    stroke: Option<SvgPaint>,
    stroke_width: Option<f32>,
    stroke_linecap: Option<LineCap>,
    stroke_linejoin: Option<LineJoin>,
    font_size: Option<f32>,
    opacity: Option<f32>,
}

// -------------------------------------------------------------------
// Text types
// -------------------------------------------------------------------

/// SVG `text-anchor` alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAnchor {
    #[default]
    Start,
    Middle,
    End,
}

/// A `<tspan>` child within a `<text>` element.
#[derive(Debug, Clone)]
pub struct TextSpan {
    pub text: String,
    pub class: Option<String>,
}

/// A parsed SVG element ready for rendering.
#[derive(Debug, Clone)]
pub struct SvgElement {
    /// Intrinsic width from the `width` attribute (or viewBox).
    pub width: f32,
    /// Intrinsic height from the `height` attribute (or viewBox).
    pub height: f32,
    /// Optional viewBox: (min-x, min-y, width, height).
    pub viewbox: Option<(f32, f32, f32, f32)>,
    /// Shapes parsed from the SVG children.
    pub shapes: Vec<SvgShape>,
    /// Gradient, pattern, and other definitions from `<defs>`.
    pub defs: SvgDefs,
}

/// A single SVG shape primitive.
#[derive(Debug, Clone)]
pub enum SvgShape {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        rx: f32,
        opacity: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        fill: SvgPaint,
        font_size: f32,
        text_anchor: TextAnchor,
        letter_spacing: f32,
        font_weight: u16,
        opacity: f32,
        spans: Vec<TextSpan>,
    },
    /// SVG `<path>` element — flattened to polygon points.
    Path {
        points: Vec<(f32, f32)>,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        fill_rule: FillRule,
        stroke_linecap: LineCap,
        stroke_linejoin: LineJoin,
        opacity: f32,
    },
    /// SVG `<polygon>` element.
    Polygon {
        points: Vec<(f32, f32)>,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        fill_rule: FillRule,
        stroke_linecap: LineCap,
        stroke_linejoin: LineJoin,
        opacity: f32,
    },
    /// SVG `<polyline>` element (no auto-close).
    Polyline {
        points: Vec<(f32, f32)>,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
    },
}

/// SVG path command (parsed from `d` attribute).
#[derive(Debug, Clone, Copy)]
enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    HorizTo(f32),
    VertTo(f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    SmoothCubicTo(f32, f32, f32, f32),
    QuadTo(f32, f32, f32, f32),
    SmoothQuadTo(f32, f32),
    Close,
}
