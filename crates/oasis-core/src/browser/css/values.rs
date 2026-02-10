//! CSS computed value types.
//!
//! These types represent the *computed* values after cascade resolution -- the
//! final concrete values consumed by the layout engine. Every property has a
//! single canonical representation (e.g. all lengths are resolved to `f32`
//! pixels, all colors to `Color`).

use super::parser::{CssColor, CssValue, LengthUnit};
use crate::backend::Color;

// -----------------------------------------------------------------------
// Enums
// -----------------------------------------------------------------------

/// CSS `display` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    ListItem,
    Table,
    TableRow,
    TableCell,
    None,
}

/// CSS `visibility` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
}

/// A dimension that may be `auto`, a pixel length, or a percentage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dimension {
    Auto,
    Px(f32),
    Percent(f32),
}

/// CSS `border-style` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
}

/// CSS `font-weight` property (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontWeight {
    /// Numeric weight 400.
    Normal,
    /// Numeric weight 700.
    Bold,
}

/// CSS `font-style` property (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyle {
    Normal,
    Italic,
}

/// CSS `font-family` generic families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFamily {
    SansSerif,
    Serif,
    Monospace,
}

/// CSS `text-align` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

/// CSS `text-decoration` property (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecoration {
    None,
    Underline,
    LineThrough,
    Overline,
}

/// CSS `text-transform` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// CSS `white-space` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
}

/// CSS `list-style-type` property (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
}

/// CSS `list-style-position` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStylePosition {
    Outside,
    Inside,
}

/// CSS `border-collapse` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderCollapse {
    Separate,
    Collapse,
}

/// CSS `float` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Float {
    None,
    Left,
    Right,
}

/// CSS `clear` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
}

/// CSS `overflow` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
}

// -----------------------------------------------------------------------
// CssValue helper
// -----------------------------------------------------------------------

/// Extract a keyword string from a `CssValue`, if it is a `Keyword`.
fn as_keyword(value: &CssValue) -> Option<&str> {
    match value {
        CssValue::Keyword(s) => Some(s.as_str()),
        _ => None,
    }
}

// -----------------------------------------------------------------------
// ComputedStyle
// -----------------------------------------------------------------------

/// Computed style for a DOM node after cascade resolution.
///
/// All lengths are resolved to absolute pixels. Relative units (em, %)
/// have been converted during property application. Inherited properties
/// that were not explicitly set carry the parent's computed value.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    // -- Display ----------------------------------------------------
    pub display: Display,
    pub visibility: Visibility,

    // -- Box model --------------------------------------------------
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub border_top_width: f32,
    pub border_right_width: f32,
    pub border_bottom_width: f32,
    pub border_left_width: f32,
    pub border_top_color: Color,
    pub border_right_color: Color,
    pub border_bottom_color: Color,
    pub border_left_color: Color,
    pub border_top_style: BorderStyle,
    pub border_right_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,

    // -- Dimensions -------------------------------------------------
    pub width: Dimension,
    pub height: Dimension,
    pub max_width: Dimension,
    pub min_width: Dimension,

    // -- Text -------------------------------------------------------
    pub color: Color,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub font_family: FontFamily,
    pub text_align: TextAlign,
    pub text_decoration: TextDecoration,
    pub text_indent: f32,
    pub text_transform: TextTransform,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub white_space: WhiteSpace,

    // -- Background -------------------------------------------------
    pub background_color: Color,

    // -- List -------------------------------------------------------
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,

    // -- Table ------------------------------------------------------
    pub border_collapse: BorderCollapse,
    pub border_spacing: f32,

    // -- Float ------------------------------------------------------
    pub float: Float,
    pub clear: Clear,

    // -- Overflow ---------------------------------------------------
    pub overflow: Overflow,
}

/// Standard browser defaults (CSS 2.1 initial values).
impl Default for ComputedStyle {
    fn default() -> Self {
        let base_font_size: f32 = 16.0;
        Self {
            // Display
            display: Display::Inline,
            visibility: Visibility::Visible,

            // Box model -- all zero
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border_top_width: 0.0,
            border_right_width: 0.0,
            border_bottom_width: 0.0,
            border_left_width: 0.0,
            border_top_color: Color::BLACK,
            border_right_color: Color::BLACK,
            border_bottom_color: Color::BLACK,
            border_left_color: Color::BLACK,
            border_top_style: BorderStyle::None,
            border_right_style: BorderStyle::None,
            border_bottom_style: BorderStyle::None,
            border_left_style: BorderStyle::None,

            // Dimensions
            width: Dimension::Auto,
            height: Dimension::Auto,
            max_width: Dimension::Auto,
            min_width: Dimension::Px(0.0),

            // Text
            color: Color::BLACK,
            font_size: base_font_size,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            font_family: FontFamily::SansSerif,
            text_align: TextAlign::Left,
            text_decoration: TextDecoration::None,
            text_indent: 0.0,
            text_transform: TextTransform::None,
            line_height: base_font_size * 1.2,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            white_space: WhiteSpace::Normal,

            // Background -- transparent
            background_color: Color::rgba(0, 0, 0, 0),

            // List
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,

            // Table
            border_collapse: BorderCollapse::Separate,
            border_spacing: 0.0,

            // Float
            float: Float::None,
            clear: Clear::None,

            // Overflow
            overflow: Overflow::Visible,
        }
    }
}

impl ComputedStyle {
    /// Create an initial style that inherits inheritable properties from
    /// the given parent style. Non-inheritable properties keep their
    /// CSS initial values.
    pub fn inherit(parent: &ComputedStyle) -> Self {
        ComputedStyle {
            // Inherited text properties.
            color: parent.color,
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            font_style: parent.font_style,
            font_family: parent.font_family,
            text_align: parent.text_align,
            text_decoration: parent.text_decoration,
            text_indent: parent.text_indent,
            text_transform: parent.text_transform,
            line_height: parent.line_height,
            letter_spacing: parent.letter_spacing,
            word_spacing: parent.word_spacing,
            white_space: parent.white_space,
            // Inherited visibility.
            visibility: parent.visibility,
            // Inherited list properties.
            list_style_type: parent.list_style_type,
            list_style_position: parent.list_style_position,
            // Inherited table properties.
            border_collapse: parent.border_collapse,
            border_spacing: parent.border_spacing,
            // Non-inherited properties keep CSS initial values.
            ..ComputedStyle::default()
        }
    }

    /// Apply a parsed CSS declaration to this style.
    ///
    /// Resolves relative units (`em`, `%`) against the parent font size
    /// so the resulting computed value is in absolute pixels.
    pub fn apply_declaration(&mut self, property: &str, value: &CssValue, parent_font_size: f32) {
        match property {
            // -- Display ------------------------------------------------
            "display" => {
                if let Some(kw) = as_keyword(value) {
                    self.display = match kw {
                        "block" => Display::Block,
                        "inline" => Display::Inline,
                        "inline-block" => Display::InlineBlock,
                        "list-item" => Display::ListItem,
                        "table" => Display::Table,
                        "table-row" => Display::TableRow,
                        "table-cell" => Display::TableCell,
                        "none" => Display::None,
                        _ => return,
                    };
                }
            },
            "visibility" => {
                if let Some(kw) = as_keyword(value) {
                    self.visibility = match kw {
                        "visible" => Visibility::Visible,
                        "hidden" => Visibility::Hidden,
                        _ => return,
                    };
                }
            },

            // -- Margins ------------------------------------------------
            "margin" => {
                let px = resolve_length(value, parent_font_size);
                self.margin_top = px;
                self.margin_right = px;
                self.margin_bottom = px;
                self.margin_left = px;
            },
            "margin-top" => {
                self.margin_top = resolve_length(value, parent_font_size);
            },
            "margin-right" => {
                self.margin_right = resolve_length(value, parent_font_size);
            },
            "margin-bottom" => {
                self.margin_bottom = resolve_length(value, parent_font_size);
            },
            "margin-left" => {
                self.margin_left = resolve_length(value, parent_font_size);
            },

            // -- Padding ------------------------------------------------
            "padding" => {
                let px = resolve_length(value, parent_font_size);
                self.padding_top = px;
                self.padding_right = px;
                self.padding_bottom = px;
                self.padding_left = px;
            },
            "padding-top" => {
                self.padding_top = resolve_length(value, parent_font_size);
            },
            "padding-right" => {
                self.padding_right = resolve_length(value, parent_font_size);
            },
            "padding-bottom" => {
                self.padding_bottom = resolve_length(value, parent_font_size);
            },
            "padding-left" => {
                self.padding_left = resolve_length(value, parent_font_size);
            },

            // -- Border width -------------------------------------------
            "border-width" => {
                let px = resolve_length(value, parent_font_size);
                self.border_top_width = px;
                self.border_right_width = px;
                self.border_bottom_width = px;
                self.border_left_width = px;
            },
            "border-top-width" => {
                self.border_top_width = resolve_length(value, parent_font_size);
            },
            "border-right-width" => {
                self.border_right_width = resolve_length(value, parent_font_size);
            },
            "border-bottom-width" => {
                self.border_bottom_width = resolve_length(value, parent_font_size);
            },
            "border-left-width" => {
                self.border_left_width = resolve_length(value, parent_font_size);
            },

            // -- Border color -------------------------------------------
            "border-color" => {
                if let Some(c) = resolve_color(value) {
                    self.border_top_color = c;
                    self.border_right_color = c;
                    self.border_bottom_color = c;
                    self.border_left_color = c;
                }
            },
            "border-top-color" => {
                if let Some(c) = resolve_color(value) {
                    self.border_top_color = c;
                }
            },
            "border-right-color" => {
                if let Some(c) = resolve_color(value) {
                    self.border_right_color = c;
                }
            },
            "border-bottom-color" => {
                if let Some(c) = resolve_color(value) {
                    self.border_bottom_color = c;
                }
            },
            "border-left-color" => {
                if let Some(c) = resolve_color(value) {
                    self.border_left_color = c;
                }
            },

            // -- Border style -------------------------------------------
            "border-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_top_style = s;
                    self.border_right_style = s;
                    self.border_bottom_style = s;
                    self.border_left_style = s;
                }
            },
            "border-top-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_top_style = s;
                }
            },
            "border-right-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_right_style = s;
                }
            },
            "border-bottom-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_bottom_style = s;
                }
            },
            "border-left-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_left_style = s;
                }
            },

            // -- Dimensions ---------------------------------------------
            "width" => {
                self.width = resolve_dimension(value, parent_font_size);
            },
            "height" => {
                self.height = resolve_dimension(value, parent_font_size);
            },
            "max-width" => {
                self.max_width = resolve_dimension(value, parent_font_size);
            },
            "min-width" => {
                self.min_width = resolve_dimension(value, parent_font_size);
            },

            // -- Color --------------------------------------------------
            "color" => {
                if let Some(c) = resolve_color(value) {
                    self.color = c;
                }
            },

            // -- Font ---------------------------------------------------
            "font-size" => {
                self.font_size = resolve_font_size(value, parent_font_size);
                // Recompute line-height at the default ratio.
                self.line_height = self.font_size * 1.2;
            },
            "font-weight" => {
                self.font_weight = resolve_font_weight(value);
            },
            "font-style" => {
                if let Some(kw) = as_keyword(value) {
                    self.font_style = match kw {
                        "italic" | "oblique" => FontStyle::Italic,
                        "normal" => FontStyle::Normal,
                        _ => return,
                    };
                }
            },
            "font-family" => {
                if let Some(kw) = as_keyword(value) {
                    self.font_family = match kw {
                        "serif" => FontFamily::Serif,
                        "sans-serif" => FontFamily::SansSerif,
                        "monospace" => FontFamily::Monospace,
                        _ => return,
                    };
                }
            },

            // -- Text ---------------------------------------------------
            "text-align" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_align = match kw {
                        "left" => TextAlign::Left,
                        "center" => TextAlign::Center,
                        "right" => TextAlign::Right,
                        "justify" => TextAlign::Justify,
                        _ => return,
                    };
                }
            },
            "text-decoration" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_decoration = match kw {
                        "none" => TextDecoration::None,
                        "underline" => TextDecoration::Underline,
                        "line-through" => TextDecoration::LineThrough,
                        "overline" => TextDecoration::Overline,
                        _ => return,
                    };
                }
            },
            "text-indent" => {
                self.text_indent = resolve_length(value, parent_font_size);
            },
            "text-transform" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_transform = match kw {
                        "none" => TextTransform::None,
                        "uppercase" => TextTransform::Uppercase,
                        "lowercase" => TextTransform::Lowercase,
                        "capitalize" => TextTransform::Capitalize,
                        _ => return,
                    };
                }
            },
            "line-height" => {
                self.line_height = resolve_line_height(value, self.font_size, parent_font_size);
            },
            "letter-spacing" => {
                if let Some("normal") = as_keyword(value) {
                    self.letter_spacing = 0.0;
                    return;
                }
                self.letter_spacing = resolve_length(value, parent_font_size);
            },
            "word-spacing" => {
                if let Some("normal") = as_keyword(value) {
                    self.word_spacing = 0.0;
                    return;
                }
                self.word_spacing = resolve_length(value, parent_font_size);
            },
            "white-space" => {
                if let Some(kw) = as_keyword(value) {
                    self.white_space = match kw {
                        "normal" => WhiteSpace::Normal,
                        "nowrap" => WhiteSpace::NoWrap,
                        "pre" => WhiteSpace::Pre,
                        "pre-wrap" => WhiteSpace::PreWrap,
                        "pre-line" => WhiteSpace::PreLine,
                        _ => return,
                    };
                }
            },

            // -- Background ---------------------------------------------
            "background-color" | "background" => {
                if let Some(c) = resolve_color(value) {
                    self.background_color = c;
                }
            },

            // -- List ---------------------------------------------------
            "list-style-type" => {
                if let Some(kw) = as_keyword(value) {
                    self.list_style_type = match kw {
                        "none" => ListStyleType::None,
                        "disc" => ListStyleType::Disc,
                        "circle" => ListStyleType::Circle,
                        "square" => ListStyleType::Square,
                        "decimal" => ListStyleType::Decimal,
                        _ => return,
                    };
                }
            },
            "list-style-position" => {
                if let Some(kw) = as_keyword(value) {
                    self.list_style_position = match kw {
                        "outside" => ListStylePosition::Outside,
                        "inside" => ListStylePosition::Inside,
                        _ => return,
                    };
                }
            },

            // -- Table --------------------------------------------------
            "border-collapse" => {
                if let Some(kw) = as_keyword(value) {
                    self.border_collapse = match kw {
                        "separate" => BorderCollapse::Separate,
                        "collapse" => BorderCollapse::Collapse,
                        _ => return,
                    };
                }
            },
            "border-spacing" => {
                self.border_spacing = resolve_length(value, parent_font_size);
            },

            // -- Float --------------------------------------------------
            "float" => {
                if let Some(kw) = as_keyword(value) {
                    self.float = match kw {
                        "none" => Float::None,
                        "left" => Float::Left,
                        "right" => Float::Right,
                        _ => return,
                    };
                }
            },
            "clear" => {
                if let Some(kw) = as_keyword(value) {
                    self.clear = match kw {
                        "none" => Clear::None,
                        "left" => Clear::Left,
                        "right" => Clear::Right,
                        "both" => Clear::Both,
                        _ => return,
                    };
                }
            },

            // -- Overflow -----------------------------------------------
            "overflow" => {
                if let Some(kw) = as_keyword(value) {
                    self.overflow = match kw {
                        "visible" => Overflow::Visible,
                        "hidden" => Overflow::Hidden,
                        _ => return,
                    };
                }
            },

            // Unknown properties are silently ignored (per CSS spec).
            _ => {},
        }
    }
}

// -----------------------------------------------------------------------
// Resolution helpers
// -----------------------------------------------------------------------

/// Resolve a `CssValue` to an absolute pixel length.
///
/// - `Px` and `Pt` values pass through (Pt approximated as 1.333 px).
/// - `Em` values are multiplied by `parent_font_size`.
/// - `Rem` values are multiplied by the root font size (16.0).
/// - Percentage and keyword values resolve to 0.
fn resolve_length(value: &CssValue, parent_font_size: f32) -> f32 {
    match value {
        CssValue::Length(n, LengthUnit::Px) => *n,
        CssValue::Length(n, LengthUnit::Em) => *n * parent_font_size,
        CssValue::Length(n, LengthUnit::Rem) => *n * 16.0,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Number(n) => *n,
        _ => 0.0,
    }
}

/// Resolve a `CssValue` to a `Dimension` (auto / px / percent).
fn resolve_dimension(value: &CssValue, parent_font_size: f32) -> Dimension {
    match value {
        CssValue::Keyword(kw) if kw == "auto" => Dimension::Auto,
        CssValue::Percentage(p) => Dimension::Percent(*p),
        CssValue::Length(n, LengthUnit::Px) => Dimension::Px(*n),
        CssValue::Length(n, LengthUnit::Em) => Dimension::Px(*n * parent_font_size),
        CssValue::Length(n, LengthUnit::Rem) => Dimension::Px(*n * 16.0),
        CssValue::Length(n, LengthUnit::Pt) => Dimension::Px(*n * 1.333),
        CssValue::Number(n) => Dimension::Px(*n),
        _ => Dimension::Auto,
    }
}

/// Resolve a color value from the parser's representation.
fn resolve_color(value: &CssValue) -> Option<Color> {
    match value {
        CssValue::Color(css_color) => Some(css_color_to_backend(css_color)),
        CssValue::Keyword(name) => keyword_color(name),
        _ => None,
    }
}

/// Convert a parser `CssColor` to the backend `Color`.
fn css_color_to_backend(c: &CssColor) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

/// Map a named CSS color keyword to an RGBA `Color`.
fn keyword_color(name: &str) -> Option<Color> {
    let c = match name {
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "maroon" => Color::rgb(128, 0, 0),
        "olive" => Color::rgb(128, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "teal" => Color::rgb(0, 128, 128),
        "navy" => Color::rgb(0, 0, 128),
        "purple" => Color::rgb(128, 0, 128),
        "orange" => Color::rgb(255, 165, 0),
        "transparent" => Color::rgba(0, 0, 0, 0),
        _ => return None,
    };
    Some(c)
}

/// Resolve a `border-style` keyword.
fn resolve_border_style(value: &CssValue) -> Option<BorderStyle> {
    let kw = as_keyword(value)?;
    let s = match kw {
        "none" => BorderStyle::None,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        _ => return None,
    };
    Some(s)
}

/// Resolve a `font-weight` value.
///
/// The CSS parser normalises keyword values: `bold` becomes
/// `CssValue::Number(700.0)` and `normal` becomes
/// `CssValue::Number(400.0)`. We also handle keywords directly
/// for inline style strings that may bypass that normalisation.
fn resolve_font_weight(value: &CssValue) -> FontWeight {
    match value {
        CssValue::Number(n) => {
            if *n >= 600.0 {
                FontWeight::Bold
            } else {
                FontWeight::Normal
            }
        },
        CssValue::Keyword(kw) => match kw.as_str() {
            "bold" => FontWeight::Bold,
            "normal" => FontWeight::Normal,
            _ => FontWeight::Normal,
        },
        _ => FontWeight::Normal,
    }
}

/// Resolve a `font-size` value.
///
/// Supports absolute keywords (`small`, `medium`, `large`, etc.),
/// relative keywords (`smaller`, `larger`), lengths, and percentages.
fn resolve_font_size(value: &CssValue, parent_font_size: f32) -> f32 {
    match value {
        CssValue::Length(n, LengthUnit::Px) => *n,
        CssValue::Length(n, LengthUnit::Em) => *n * parent_font_size,
        CssValue::Length(n, LengthUnit::Rem) => *n * 16.0,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Percentage(p) => parent_font_size * (*p / 100.0),
        CssValue::Number(n) => *n,
        CssValue::Keyword(kw) => match kw.as_str() {
            "xx-small" => 9.0,
            "x-small" => 10.0,
            "small" => 13.0,
            "medium" => 16.0,
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "smaller" => parent_font_size * 0.833,
            "larger" => parent_font_size * 1.2,
            _ => parent_font_size,
        },
        _ => parent_font_size,
    }
}

/// Resolve a `line-height` value.
///
/// - A bare number is treated as a multiplier of the element's font size.
/// - A length or percentage is resolved normally.
/// - The keyword `normal` maps to 1.2 * font_size.
fn resolve_line_height(value: &CssValue, font_size: f32, parent_font_size: f32) -> f32 {
    match value {
        CssValue::Number(n) => *n * font_size,
        CssValue::Length(n, LengthUnit::Px) => *n,
        CssValue::Length(n, LengthUnit::Em) => *n * parent_font_size,
        CssValue::Length(n, LengthUnit::Rem) => *n * 16.0,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Percentage(p) => font_size * (*p / 100.0),
        CssValue::Keyword(kw) if kw == "normal" => font_size * 1.2,
        _ => font_size * 1.2,
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_has_browser_defaults() {
        let s = ComputedStyle::default();
        assert_eq!(s.display, Display::Inline);
        assert_eq!(s.visibility, Visibility::Visible);
        assert_eq!(s.color, Color::BLACK);
        assert!((s.font_size - 16.0).abs() < f32::EPSILON);
        assert_eq!(s.font_weight, FontWeight::Normal);
        assert_eq!(s.font_style, FontStyle::Normal);
        assert_eq!(s.font_family, FontFamily::SansSerif);
        assert!((s.line_height - 19.2).abs() < 0.01);
        assert!((s.margin_top).abs() < f32::EPSILON);
        assert!((s.padding_top).abs() < f32::EPSILON);
        assert!((s.border_top_width).abs() < f32::EPSILON);
        assert_eq!(s.background_color, Color::rgba(0, 0, 0, 0));
        assert_eq!(s.float, Float::None);
        assert_eq!(s.overflow, Overflow::Visible);
        assert_eq!(s.text_align, TextAlign::Left);
        assert_eq!(s.text_decoration, TextDecoration::None);
        assert_eq!(s.white_space, WhiteSpace::Normal);
        assert_eq!(s.list_style_type, ListStyleType::Disc);
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
    }

    #[test]
    fn inherit_copies_inheritable_properties() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(255, 0, 0);
        parent.font_size = 20.0;
        parent.font_weight = FontWeight::Bold;
        parent.text_align = TextAlign::Center;
        parent.visibility = Visibility::Hidden;
        parent.list_style_type = ListStyleType::Square;

        let child = ComputedStyle::inherit(&parent);

        // Inherited.
        assert_eq!(child.color, Color::rgb(255, 0, 0));
        assert!((child.font_size - 20.0).abs() < f32::EPSILON);
        assert_eq!(child.font_weight, FontWeight::Bold);
        assert_eq!(child.text_align, TextAlign::Center);
        assert_eq!(child.visibility, Visibility::Hidden);
        assert_eq!(child.list_style_type, ListStyleType::Square);

        // Non-inherited: should be initial values, not parent's.
        assert_eq!(child.display, Display::Inline);
        assert!((child.margin_top).abs() < f32::EPSILON);
        assert_eq!(child.background_color, Color::rgba(0, 0, 0, 0));
        assert_eq!(child.float, Float::None);
    }

    #[test]
    fn apply_keyword_display() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("display", &CssValue::Keyword("block".into()), 16.0);
        assert_eq!(s.display, Display::Block);
    }

    #[test]
    fn apply_px_margin() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("margin", &CssValue::Length(10.0, LengthUnit::Px), 16.0);
        assert!((s.margin_top - 10.0).abs() < f32::EPSILON);
        assert!((s.margin_right - 10.0).abs() < f32::EPSILON);
        assert!((s.margin_bottom - 10.0).abs() < f32::EPSILON);
        assert!((s.margin_left - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_em_padding() {
        let mut s = ComputedStyle::default();
        // 1.5em with parent font-size 20px = 30px.
        s.apply_declaration("padding-top", &CssValue::Length(1.5, LengthUnit::Em), 20.0);
        assert!((s.padding_top - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_color_keyword() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("color", &CssValue::Keyword("red".into()), 16.0);
        assert_eq!(s.color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn apply_color_value() {
        let mut s = ComputedStyle::default();
        let c = CssColor {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        };
        s.apply_declaration("color", &CssValue::Color(c), 16.0);
        assert_eq!(s.color, Color::rgb(10, 20, 30));
    }

    #[test]
    fn apply_font_size_updates_line_height() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-size", &CssValue::Length(20.0, LengthUnit::Px), 16.0);
        assert!((s.font_size - 20.0).abs() < f32::EPSILON);
        // Line height should be recomputed: 20 * 1.2 = 24.
        assert!((s.line_height - 24.0).abs() < 0.01);
    }

    #[test]
    fn apply_font_weight_bold_keyword() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-weight", &CssValue::Keyword("bold".into()), 16.0);
        assert_eq!(s.font_weight, FontWeight::Bold);
    }

    #[test]
    fn apply_font_weight_bold_number() {
        // The CSS parser normalises "bold" to Number(700.0).
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-weight", &CssValue::Number(700.0), 16.0);
        assert_eq!(s.font_weight, FontWeight::Bold);
    }

    #[test]
    fn apply_font_weight_normal_number() {
        let mut s = ComputedStyle::default();
        s.font_weight = FontWeight::Bold;
        s.apply_declaration("font-weight", &CssValue::Number(400.0), 16.0);
        assert_eq!(s.font_weight, FontWeight::Normal);
    }

    #[test]
    fn apply_dimension_percent() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("width", &CssValue::Percentage(50.0), 16.0);
        assert_eq!(s.width, Dimension::Percent(50.0));
    }

    #[test]
    fn apply_dimension_auto() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("width", &CssValue::Keyword("auto".into()), 16.0);
        assert_eq!(s.width, Dimension::Auto);
    }

    #[test]
    fn apply_border_shorthand() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("border-style", &CssValue::Keyword("solid".into()), 16.0);
        assert_eq!(s.border_top_style, BorderStyle::Solid);
        assert_eq!(s.border_right_style, BorderStyle::Solid);
        assert_eq!(s.border_bottom_style, BorderStyle::Solid);
        assert_eq!(s.border_left_style, BorderStyle::Solid);
    }

    #[test]
    fn apply_background_color() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("background-color", &CssValue::Keyword("white".into()), 16.0);
        assert_eq!(s.background_color, Color::WHITE);
    }

    #[test]
    fn apply_unknown_property_is_noop() {
        let mut s = ComputedStyle::default();
        let before = s.clone();
        s.apply_declaration("unknown-prop", &CssValue::Keyword("something".into()), 16.0);
        // Nothing should have changed.
        assert_eq!(s.display, before.display);
        assert_eq!(s.color, before.color);
    }

    #[test]
    fn resolve_font_size_keywords() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-size", &CssValue::Keyword("small".into()), 16.0);
        assert!((s.font_size - 13.0).abs() < f32::EPSILON);

        s.apply_declaration("font-size", &CssValue::Keyword("larger".into()), 16.0);
        assert!((s.font_size - 19.2).abs() < 0.01);
    }

    #[test]
    fn resolve_line_height_number_multiplier() {
        let mut s = ComputedStyle::default();
        s.font_size = 20.0;
        s.apply_declaration("line-height", &CssValue::Number(1.5), 16.0);
        // 1.5 * 20.0 = 30.0
        assert!((s.line_height - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn keyword_color_lookup() {
        assert_eq!(keyword_color("red"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(keyword_color("navy"), Some(Color::rgb(0, 0, 128)),);
        assert_eq!(keyword_color("transparent"), Some(Color::rgba(0, 0, 0, 0)),);
        assert_eq!(keyword_color("nonexistent"), None);
    }
}
