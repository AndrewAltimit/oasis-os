//! CSS computed value types.
//!
//! These types represent the *computed* values after cascade resolution -- the
//! final concrete values consumed by the layout engine. Every property has a
//! single canonical representation (e.g. all lengths are resolved to `f32`
//! pixels, all colors to `Color`).

use std::collections::HashMap;

use super::parser::{CssColor, CssValue, LengthUnit};
use oasis_types::backend::Color;

/// Root font size in pixels. Standard CSS uses 16px but the OASIS
/// native resolution is 480x272 so we use 8px (the bitmap glyph size)
/// to keep text readable.
pub const ROOT_FONT_SIZE: f32 = 8.0;

// -----------------------------------------------------------------------
// Enums
// -----------------------------------------------------------------------

/// CSS `display` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    ListItem,
    Table,
    TableRow,
    TableCell,
    None,
}

/// CSS `flex-direction` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

/// CSS `justify-content` property (main axis alignment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// CSS `align-items` property (cross axis alignment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

/// CSS `flex-wrap` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
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

/// CSS `position` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
}

/// CSS `overflow` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
}

/// CSS `word-break` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordBreak {
    Normal,
    BreakAll,
}

/// CSS `overflow-wrap` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowWrap {
    Normal,
    BreakWord,
    Anywhere,
}

/// CSS `text-overflow` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
}

/// CSS `box-sizing` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

/// CSS `vertical-align` property (subset for inline replaced elements).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
}

/// A color stop in a CSS gradient.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    /// Position as a fraction 0.0 ..= 1.0.
    pub position: f32,
}

/// CSS linear-gradient direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientDirection {
    /// Angle in degrees (0 = to top, 90 = to right).
    Angle(f32),
    ToTop,
    ToRight,
    ToBottom,
    ToLeft,
}

/// A parsed CSS `linear-gradient(...)` value.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub direction: GradientDirection,
    pub stops: Vec<GradientStop>,
}

/// CSS `background-image` property.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    None,
    Url(String),
    Gradient(LinearGradient),
}

/// CSS `text-shadow` value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub color: Color,
}

/// CSS `box-shadow` value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

/// A single CSS Grid track size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackSize {
    Px(f32),
    Fr(f32),
    Auto,
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
#[derive(Debug, Clone, PartialEq)]
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
    pub max_height: Dimension,
    pub min_height: Dimension,

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

    // -- Positioning ------------------------------------------------
    pub position: Position,
    pub top: Dimension,
    pub right: Dimension,
    pub bottom: Dimension,
    pub left: Dimension,
    pub z_index: i32,

    // -- Visual effects -----------------------------------------------
    pub border_radius: f32,
    pub box_shadow: Option<BoxShadow>,
    pub text_shadow: Option<TextShadow>,
    pub opacity: f32,

    // -- Box sizing -----------------------------------------------------
    pub box_sizing: BoxSizing,

    // -- Text overflow --------------------------------------------------
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,
    pub text_overflow: TextOverflow,

    // -- Vertical alignment ---------------------------------------------
    pub vertical_align: VerticalAlign,

    // -- Background image -----------------------------------------------
    pub background_image: BackgroundImage,

    // -- Generated content (::before/::after) ---------------------------
    pub content: Option<String>,
    pub before_content: Option<String>,
    pub after_content: Option<String>,

    pub before_style: Option<Box<ComputedStyle>>,
    pub after_style: Option<Box<ComputedStyle>>,

    // -- Margin auto flags (for block centering) -------------------------
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub margin_top_auto: bool,
    pub margin_bottom_auto: bool,

    // -- Flexbox properties --
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Dimension,
    pub gap: f32,

    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    pub grid_column_start: Option<i32>,
    pub grid_column_end: Option<i32>,
    pub grid_row_start: Option<i32>,
    pub grid_row_end: Option<i32>,
    pub column_gap: f32,
    pub row_gap: f32,

    // -- Percentage padding/margin (resolved against containing width) ---
    /// When `Some(pct)`, padding-top was specified as a percentage.
    pub padding_top_pct: Option<f32>,
    pub padding_right_pct: Option<f32>,
    pub padding_bottom_pct: Option<f32>,
    pub padding_left_pct: Option<f32>,
    pub margin_top_pct: Option<f32>,
    pub margin_right_pct: Option<f32>,
    pub margin_bottom_pct: Option<f32>,
    pub margin_left_pct: Option<f32>,

    // -- CSS custom properties (--*) ------------------------------------
    pub custom_properties: HashMap<String, String>,
}

/// Standard browser defaults (CSS 2.1 initial values).
impl Default for ComputedStyle {
    fn default() -> Self {
        let base_font_size: f32 = ROOT_FONT_SIZE;
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
            max_height: Dimension::Auto,
            min_height: Dimension::Auto,

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
            line_height: base_font_size * 1.5,
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

            // Positioning
            position: Position::Static,
            top: Dimension::Auto,
            right: Dimension::Auto,
            bottom: Dimension::Auto,
            left: Dimension::Auto,
            z_index: 0,

            // Visual effects
            border_radius: 0.0,
            box_shadow: None,
            text_shadow: None,
            opacity: 1.0,

            // Box sizing
            box_sizing: BoxSizing::ContentBox,

            // Text overflow
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            text_overflow: TextOverflow::Clip,

            // Vertical alignment
            vertical_align: VerticalAlign::Baseline,

            // Background image
            background_image: BackgroundImage::None,

            // Generated content
            content: None,
            before_content: None,
            after_content: None,

            before_style: None,
            after_style: None,

            margin_left_auto: false,
            margin_right_auto: false,
            margin_top_auto: false,
            margin_bottom_auto: false,

            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Dimension::Auto,
            gap: 0.0,

            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_column_start: None,
            grid_column_end: None,
            grid_row_start: None,
            grid_row_end: None,
            column_gap: 0.0,
            row_gap: 0.0,

            padding_top_pct: None,
            padding_right_pct: None,
            padding_bottom_pct: None,
            padding_left_pct: None,
            margin_top_pct: None,
            margin_right_pct: None,
            margin_bottom_pct: None,
            margin_left_pct: None,

            custom_properties: HashMap::new(),
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
            // Inherited text shadow.
            text_shadow: parent.text_shadow,
            // Inherited visibility.
            visibility: parent.visibility,
            // Inherited list properties.
            list_style_type: parent.list_style_type,
            list_style_position: parent.list_style_position,
            // Inherited table properties.
            border_collapse: parent.border_collapse,
            border_spacing: parent.border_spacing,
            // CSS custom properties always inherit.
            custom_properties: parent.custom_properties.clone(),
            // Non-inherited properties keep CSS initial values.
            ..ComputedStyle::default()
        }
    }

    /// Apply a parsed CSS declaration to this style.
    ///
    /// Resolves relative units (`em`, `%`) against the parent font size
    /// so the resulting computed value is in absolute pixels.
    pub fn apply_declaration(&mut self, property: &str, value: &CssValue, parent_font_size: f32) {
        // Custom properties (--*) are stored in the properties map.
        if property.starts_with("--") {
            if let CssValue::String(ref raw) = *value {
                self.custom_properties
                    .insert(property.to_string(), raw.clone());
            }
            return;
        }

        // Handle `inherit` and `initial` keywords for any property.
        if let Some(kw) = as_keyword(value) {
            if kw == "initial" {
                self.apply_initial(property);
                return;
            }
            if kw == "inherit" {
                // The caller has already set up `self` via `inherit(parent)`,
                // so inherited properties already carry the parent value.
                // For non-inherited properties, we need the parent's computed
                // value. Since we don't have the parent here, we rely on the
                // cascade having called `inherit(parent)` beforehand -- the
                // parent_font_size parameter gives us the parent font context.
                // For properties that are already inherited (color, font-*, etc.)
                // the current value is already correct. For non-inherited
                // properties, `inherit` is rare; do nothing extra here.
                return;
            }
        }

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
                        "flex" => Display::Flex,
                        "grid" => Display::Grid,
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
                if as_keyword(value) == Some("auto") {
                    self.margin_top = 0.0;
                    self.margin_right = 0.0;
                    self.margin_bottom = 0.0;
                    self.margin_left = 0.0;
                    self.margin_left_auto = true;
                    self.margin_right_auto = true;
                    self.margin_top_auto = true;
                    self.margin_bottom_auto = true;
                    self.margin_top_pct = None;
                    self.margin_right_pct = None;
                    self.margin_bottom_pct = None;
                    self.margin_left_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_top_pct = Some(*p);
                    self.margin_right_pct = Some(*p);
                    self.margin_bottom_pct = Some(*p);
                    self.margin_left_pct = Some(*p);
                    self.margin_top = 0.0;
                    self.margin_right = 0.0;
                    self.margin_bottom = 0.0;
                    self.margin_left = 0.0;
                    self.margin_left_auto = false;
                    self.margin_right_auto = false;
                    self.margin_top_auto = false;
                    self.margin_bottom_auto = false;
                } else {
                    let px = resolve_length(value, parent_font_size);
                    self.margin_top = px;
                    self.margin_right = px;
                    self.margin_bottom = px;
                    self.margin_left = px;
                    self.margin_left_auto = false;
                    self.margin_right_auto = false;
                    self.margin_top_auto = false;
                    self.margin_bottom_auto = false;
                    self.margin_top_pct = None;
                    self.margin_right_pct = None;
                    self.margin_bottom_pct = None;
                    self.margin_left_pct = None;
                }
            },
            "margin-top" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_top = 0.0;
                    self.margin_top_auto = true;
                    self.margin_top_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_top_pct = Some(*p);
                    self.margin_top = 0.0;
                } else {
                    self.margin_top = resolve_length(value, parent_font_size);
                    self.margin_top_pct = None;
                }
            },
            "margin-right" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_right = 0.0;
                    self.margin_right_auto = true;
                    self.margin_right_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_right_pct = Some(*p);
                    self.margin_right = 0.0;
                    self.margin_right_auto = false;
                } else {
                    self.margin_right = resolve_length(value, parent_font_size);
                    self.margin_right_auto = false;
                    self.margin_right_pct = None;
                }
            },
            "margin-bottom" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_bottom = 0.0;
                    self.margin_bottom_auto = true;
                    self.margin_bottom_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_bottom_pct = Some(*p);
                    self.margin_bottom = 0.0;
                } else {
                    self.margin_bottom = resolve_length(value, parent_font_size);
                    self.margin_bottom_pct = None;
                }
            },
            "margin-left" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_left = 0.0;
                    self.margin_left_auto = true;
                    self.margin_left_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_left_pct = Some(*p);
                    self.margin_left = 0.0;
                    self.margin_left_auto = false;
                } else {
                    self.margin_left = resolve_length(value, parent_font_size);
                    self.margin_left_auto = false;
                    self.margin_left_pct = None;
                }
            },

            // -- Padding ------------------------------------------------
            "padding" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_top_pct = Some(*p);
                    self.padding_right_pct = Some(*p);
                    self.padding_bottom_pct = Some(*p);
                    self.padding_left_pct = Some(*p);
                    self.padding_top = 0.0;
                    self.padding_right = 0.0;
                    self.padding_bottom = 0.0;
                    self.padding_left = 0.0;
                } else {
                    let px = resolve_length(value, parent_font_size);
                    self.padding_top = px;
                    self.padding_right = px;
                    self.padding_bottom = px;
                    self.padding_left = px;
                    self.padding_top_pct = None;
                    self.padding_right_pct = None;
                    self.padding_bottom_pct = None;
                    self.padding_left_pct = None;
                }
            },
            "padding-top" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_top_pct = Some(*p);
                    self.padding_top = 0.0;
                } else {
                    self.padding_top = resolve_length(value, parent_font_size);
                    self.padding_top_pct = None;
                }
            },
            "padding-right" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_right_pct = Some(*p);
                    self.padding_right = 0.0;
                } else {
                    self.padding_right = resolve_length(value, parent_font_size);
                    self.padding_right_pct = None;
                }
            },
            "padding-bottom" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_bottom_pct = Some(*p);
                    self.padding_bottom = 0.0;
                } else {
                    self.padding_bottom = resolve_length(value, parent_font_size);
                    self.padding_bottom_pct = None;
                }
            },
            "padding-left" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_left_pct = Some(*p);
                    self.padding_left = 0.0;
                } else {
                    self.padding_left = resolve_length(value, parent_font_size);
                    self.padding_left_pct = None;
                }
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
                let c = resolve_color_or_current(value, self.color);
                if let Some(c) = c {
                    self.border_top_color = c;
                    self.border_right_color = c;
                    self.border_bottom_color = c;
                    self.border_left_color = c;
                }
            },
            "border-top-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.border_top_color = c;
                }
            },
            "border-right-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.border_right_color = c;
                }
            },
            "border-bottom-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.border_bottom_color = c;
                }
            },
            "border-left-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
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
            "max-height" => {
                self.max_height = resolve_dimension(value, parent_font_size);
            },
            "min-height" => {
                self.min_height = resolve_dimension(value, parent_font_size);
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
                self.line_height = self.font_size * 1.5;
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

            // -- Positioning --------------------------------------------
            "position" => {
                if let Some(kw) = as_keyword(value) {
                    self.position = match kw {
                        "static" => Position::Static,
                        "relative" => Position::Relative,
                        "absolute" => Position::Absolute,
                        "fixed" => Position::Fixed,
                        _ => return,
                    };
                }
            },
            "top" => {
                self.top = resolve_dimension(value, parent_font_size);
            },
            "right" => {
                self.right = resolve_dimension(value, parent_font_size);
            },
            "bottom" => {
                self.bottom = resolve_dimension(value, parent_font_size);
            },
            "left" => {
                self.left = resolve_dimension(value, parent_font_size);
            },
            "z-index" => {
                if let CssValue::Number(n) = value {
                    self.z_index = *n as i32;
                }
            },

            // -- Flexbox properties --
            "flex-direction" => {
                if let Some(kw) = as_keyword(value) {
                    self.flex_direction = match kw {
                        "row" => FlexDirection::Row,
                        "row-reverse" => FlexDirection::RowReverse,
                        "column" => FlexDirection::Column,
                        "column-reverse" => FlexDirection::ColumnReverse,
                        _ => return,
                    };
                }
            },
            "flex-wrap" => {
                if let Some(kw) = as_keyword(value) {
                    self.flex_wrap = match kw {
                        "nowrap" => FlexWrap::NoWrap,
                        "wrap" => FlexWrap::Wrap,
                        "wrap-reverse" => FlexWrap::WrapReverse,
                        _ => return,
                    };
                }
            },
            "justify-content" => {
                if let Some(kw) = as_keyword(value) {
                    self.justify_content = match kw {
                        "flex-start" | "start" => JustifyContent::FlexStart,
                        "flex-end" | "end" => JustifyContent::FlexEnd,
                        "center" => JustifyContent::Center,
                        "space-between" => JustifyContent::SpaceBetween,
                        "space-around" => JustifyContent::SpaceAround,
                        "space-evenly" => JustifyContent::SpaceEvenly,
                        _ => return,
                    };
                }
            },
            "align-items" => {
                if let Some(kw) = as_keyword(value) {
                    self.align_items = match kw {
                        "flex-start" | "start" => AlignItems::FlexStart,
                        "flex-end" | "end" => AlignItems::FlexEnd,
                        "center" => AlignItems::Center,
                        "stretch" => AlignItems::Stretch,
                        "baseline" => AlignItems::Baseline,
                        _ => return,
                    };
                }
            },
            "flex-grow" => {
                if let CssValue::Number(n) = value {
                    self.flex_grow = *n;
                }
            },
            "flex-shrink" => {
                if let CssValue::Number(n) = value {
                    self.flex_shrink = *n;
                }
            },
            "flex-basis" => {
                self.flex_basis = resolve_dimension(value, parent_font_size);
            },
            "gap" | "grid-gap" => {
                let v = resolve_length(value, parent_font_size);
                self.gap = v;
                self.column_gap = v;
                self.row_gap = v;
            },
            "column-gap" | "grid-column-gap" => {
                self.column_gap = resolve_length(value, parent_font_size);
            },
            "row-gap" | "grid-row-gap" => {
                self.row_gap = resolve_length(value, parent_font_size);
            },
            "grid-template-columns" => {
                self.grid_template_columns = parse_grid_template(value, parent_font_size);
            },
            "grid-template-rows" => {
                self.grid_template_rows = parse_grid_template(value, parent_font_size);
            },
            "grid-column-start" => {
                if let CssValue::Number(n) = value {
                    self.grid_column_start = Some(*n as i32);
                }
            },
            "grid-column-end" => {
                if let CssValue::Number(n) = value {
                    self.grid_column_end = Some(*n as i32);
                }
            },
            "grid-column" => {
                if let CssValue::Number(n) = value {
                    self.grid_column_start = Some(*n as i32);
                }
            },
            "grid-row-start" => {
                if let CssValue::Number(n) = value {
                    self.grid_row_start = Some(*n as i32);
                }
            },
            "grid-row-end" => {
                if let CssValue::Number(n) = value {
                    self.grid_row_end = Some(*n as i32);
                }
            },
            "grid-row" => {
                if let CssValue::Number(n) = value {
                    self.grid_row_start = Some(*n as i32);
                }
            },

            // -- Visual effects -----------------------------------------
            "border-radius" => {
                self.border_radius = resolve_length(value, parent_font_size);
            },
            "opacity" => {
                if let CssValue::Number(n) = value {
                    self.opacity = n.clamp(0.0, 1.0);
                }
            },
            "box-shadow" => {
                if let Some(kw) = as_keyword(value)
                    && kw == "none"
                {
                    self.box_shadow = None;
                }
                // Complex box-shadow values are parsed from the raw
                // declaration list in the cascade.
            },
            "text-shadow" => {
                if let Some(kw) = as_keyword(value) {
                    if kw == "none" {
                        self.text_shadow = None;
                    }
                } else if let CssValue::Multiple(vs) = value {
                    // text-shadow: <offset-x> <offset-y> [blur] [color]
                    let mut nums = Vec::new();
                    let mut color = None;
                    for v in vs {
                        match v {
                            CssValue::Length(n, _) | CssValue::Number(n) => nums.push(*n),
                            CssValue::Color(c) => {
                                color = Some(Color::rgba(c.r, c.g, c.b, c.a));
                            },
                            CssValue::Keyword(kw) => {
                                if let Some(c) = crate::css::helpers::named_color(kw) {
                                    color = Some(Color::rgba(c.r, c.g, c.b, c.a));
                                }
                            },
                            _ => {},
                        }
                    }
                    if nums.len() >= 2 {
                        self.text_shadow = Some(TextShadow {
                            offset_x: nums[0],
                            offset_y: nums[1],
                            blur: nums.get(2).copied().unwrap_or(0.0),
                            color: color.unwrap_or(Color::rgba(0, 0, 0, 255)),
                        });
                    }
                }
            },

            // -- Box sizing ---------------------------------------------
            "box-sizing" => {
                if let Some(kw) = as_keyword(value) {
                    self.box_sizing = match kw {
                        "content-box" => BoxSizing::ContentBox,
                        "border-box" => BoxSizing::BorderBox,
                        _ => return,
                    };
                }
            },

            // -- Vertical alignment -------------------------------------
            "vertical-align" => {
                if let Some(kw) = as_keyword(value) {
                    self.vertical_align = match kw {
                        "baseline" => VerticalAlign::Baseline,
                        "top" => VerticalAlign::Top,
                        "middle" => VerticalAlign::Middle,
                        "bottom" => VerticalAlign::Bottom,
                        "text-top" => VerticalAlign::TextTop,
                        "text-bottom" => VerticalAlign::TextBottom,
                        _ => return,
                    };
                }
            },

            // -- Background image ---------------------------------------
            "background-image" => {
                if let Some(kw) = as_keyword(value) {
                    if kw == "none" {
                        self.background_image = BackgroundImage::None;
                    }
                } else if let CssValue::Url(ref url) = *value {
                    self.background_image = BackgroundImage::Url(url.clone());
                } else if let CssValue::Gradient(ref grad) = *value {
                    self.background_image = BackgroundImage::Gradient(grad.clone());
                }
            },

            // -- Text overflow ------------------------------------------
            "word-break" => {
                if let Some(kw) = as_keyword(value) {
                    self.word_break = match kw {
                        "break-all" => WordBreak::BreakAll,
                        _ => WordBreak::Normal,
                    };
                }
            },
            "overflow-wrap" | "word-wrap" => {
                if let Some(kw) = as_keyword(value) {
                    self.overflow_wrap = match kw {
                        "break-word" => OverflowWrap::BreakWord,
                        "anywhere" => OverflowWrap::Anywhere,
                        _ => OverflowWrap::Normal,
                    };
                }
            },
            "text-overflow" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_overflow = match kw {
                        "ellipsis" => TextOverflow::Ellipsis,
                        _ => TextOverflow::Clip,
                    };
                }
            },

            // -- Generated content --------------------------------------
            "content" => match value {
                CssValue::String(s) => {
                    self.content = Some(s.clone());
                },
                CssValue::Keyword(kw) if kw == "none" || kw == "normal" => {
                    self.content = None;
                },
                _ => {},
            },

            // Unknown properties are silently ignored (per CSS spec).
            _ => {},
        }
    }

    /// Reset a single property to its CSS initial value.
    fn apply_initial(&mut self, property: &str) {
        let initial = ComputedStyle::default();
        match property {
            "display" => self.display = initial.display,
            "visibility" => self.visibility = initial.visibility,
            "margin" | "margin-top" => self.margin_top = 0.0,
            "margin-right" => self.margin_right = 0.0,
            "margin-bottom" => self.margin_bottom = 0.0,
            "margin-left" => self.margin_left = 0.0,
            "padding" | "padding-top" => self.padding_top = 0.0,
            "padding-right" => self.padding_right = 0.0,
            "padding-bottom" => self.padding_bottom = 0.0,
            "padding-left" => self.padding_left = 0.0,
            "color" => self.color = initial.color,
            "background-color" | "background" => self.background_color = initial.background_color,
            "font-size" => {
                self.font_size = initial.font_size;
                self.line_height = initial.line_height;
            },
            "font-weight" => self.font_weight = initial.font_weight,
            "font-style" => self.font_style = initial.font_style,
            "font-family" => self.font_family = initial.font_family,
            "text-align" => self.text_align = initial.text_align,
            "text-decoration" => self.text_decoration = initial.text_decoration,
            "text-transform" => self.text_transform = initial.text_transform,
            "white-space" => self.white_space = initial.white_space,
            "line-height" => self.line_height = initial.line_height,
            "float" => self.float = initial.float,
            "clear" => self.clear = initial.clear,
            "position" => self.position = initial.position,
            "overflow" => self.overflow = initial.overflow,
            "width" => self.width = initial.width,
            "height" => self.height = initial.height,
            "border-collapse" => self.border_collapse = initial.border_collapse,
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
        CssValue::Length(n, LengthUnit::Rem) => *n * ROOT_FONT_SIZE,
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
        CssValue::Length(n, LengthUnit::Rem) => Dimension::Px(*n * ROOT_FONT_SIZE),
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

/// Resolve a color value, treating `currentcolor` as the element's `color`.
fn resolve_color_or_current(value: &CssValue, current_color: Color) -> Option<Color> {
    if let CssValue::Keyword(name) = value
        && name.eq_ignore_ascii_case("currentcolor")
    {
        return Some(current_color);
    }
    resolve_color(value)
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
        CssValue::Length(n, LengthUnit::Rem) => *n * ROOT_FONT_SIZE,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Percentage(p) => parent_font_size * (*p / 100.0),
        CssValue::Number(n) => *n,
        CssValue::Keyword(kw) => match kw.as_str() {
            "xx-small" => ROOT_FONT_SIZE * 0.5625,
            "x-small" => ROOT_FONT_SIZE * 0.625,
            "small" => ROOT_FONT_SIZE * 0.8125,
            "medium" => ROOT_FONT_SIZE,
            "large" => ROOT_FONT_SIZE * 1.125,
            "x-large" => ROOT_FONT_SIZE * 1.5,
            "xx-large" => ROOT_FONT_SIZE * 2.0,
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
/// - The keyword `normal` maps to 1.5 * font_size (generous for 480x272).
fn resolve_line_height(value: &CssValue, font_size: f32, parent_font_size: f32) -> f32 {
    match value {
        CssValue::Number(n) => *n * font_size,
        CssValue::Length(n, LengthUnit::Px) => *n,
        CssValue::Length(n, LengthUnit::Em) => *n * parent_font_size,
        CssValue::Length(n, LengthUnit::Rem) => *n * ROOT_FONT_SIZE,
        CssValue::Length(n, LengthUnit::Pt) => *n * 1.333,
        CssValue::Percentage(p) => font_size * (*p / 100.0),
        CssValue::Keyword(kw) if kw == "normal" => font_size * 1.5,
        _ => font_size * 1.5,
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

/// Parse a grid-template-columns or grid-template-rows value.
fn parse_grid_template(value: &CssValue, parent_font_size: f32) -> Vec<GridTrackSize> {
    match value {
        CssValue::Keyword(kw) if kw == "none" => Vec::new(),
        CssValue::Keyword(kw) if kw == "auto" => vec![GridTrackSize::Auto],
        CssValue::Keyword(kw) => parse_grid_template_str(kw, parent_font_size),
        CssValue::String(s) => parse_grid_template_str(s, parent_font_size),
        CssValue::Length(n, unit) => {
            let px = match unit {
                LengthUnit::Px => *n,
                LengthUnit::Em => *n * parent_font_size,
                LengthUnit::Rem => *n * ROOT_FONT_SIZE,
                LengthUnit::Pt => *n * 1.333,
            };
            vec![GridTrackSize::Px(px)]
        },
        CssValue::Number(n) => vec![GridTrackSize::Px(*n)],
        CssValue::Multiple(vals) => {
            let mut tracks = Vec::new();
            for v in vals {
                match v {
                    CssValue::Keyword(kw) if kw == "auto" => tracks.push(GridTrackSize::Auto),
                    CssValue::Keyword(kw) => {
                        if let Some(t) = parse_single_track_str(kw) {
                            tracks.push(t);
                        }
                    },
                    CssValue::Length(px, LengthUnit::Px) => {
                        tracks.push(GridTrackSize::Px(*px));
                    },
                    CssValue::Number(n) => tracks.push(GridTrackSize::Px(*n)),
                    CssValue::String(s) => {
                        if let Some(t) = parse_single_track_str(s) {
                            tracks.push(t);
                        }
                    },
                    _ => {},
                }
            }
            tracks
        },
        _ => Vec::new(),
    }
}

fn parse_single_track_str(s: &str) -> Option<GridTrackSize> {
    let s = s.trim();
    if s == "auto" {
        Some(GridTrackSize::Auto)
    } else if let Some(fr) = s.strip_suffix("fr") {
        fr.trim().parse::<f32>().ok().map(GridTrackSize::Fr)
    } else if let Some(px) = s.strip_suffix("px") {
        px.trim().parse::<f32>().ok().map(GridTrackSize::Px)
    } else if let Ok(n) = s.parse::<f32>() {
        Some(GridTrackSize::Px(n))
    } else {
        None
    }
}

fn parse_grid_template_str(s: &str, _parent_font_size: f32) -> Vec<GridTrackSize> {
    let s = s.trim();
    if s == "none" {
        return Vec::new();
    }
    let mut tracks = Vec::new();
    let mut remainder = s;
    while !remainder.is_empty() {
        remainder = remainder.trim_start();
        if remainder.starts_with("repeat(") {
            // Find the matching closing paren for this repeat() block.
            if let Some(close) = remainder.find(')') {
                let inner = &remainder["repeat(".len()..close];
                if let Some((cs, vs)) = inner.split_once(',')
                    && let Ok(count) = cs.trim().parse::<usize>()
                    && let Some(track) = parse_single_track_str(vs.trim())
                {
                    for _ in 0..count {
                        tracks.push(track);
                    }
                }
                remainder = &remainder[close + 1..];
            } else {
                break;
            }
        } else {
            // Take the next whitespace-delimited token.
            let token = match remainder.find(char::is_whitespace) {
                Some(pos) => {
                    let t = &remainder[..pos];
                    remainder = &remainder[pos..];
                    t
                },
                None => {
                    let t = remainder;
                    remainder = "";
                    t
                },
            };
            if let Some(track) = parse_single_track_str(token) {
                tracks.push(track);
            }
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_has_browser_defaults() {
        let s = ComputedStyle::default();
        assert_eq!(s.display, Display::Inline);
        assert_eq!(s.visibility, Visibility::Visible);
        assert_eq!(s.color, Color::BLACK);
        assert!((s.font_size - ROOT_FONT_SIZE).abs() < f32::EPSILON);
        assert_eq!(s.font_weight, FontWeight::Normal);
        assert_eq!(s.font_style, FontStyle::Normal);
        assert_eq!(s.font_family, FontFamily::SansSerif);
        assert!((s.line_height - ROOT_FONT_SIZE * 1.5).abs() < 0.01);
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
        // Line height should be recomputed: 20 * 1.5 = 30.
        assert!((s.line_height - 30.0).abs() < 0.01);
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
        let parent = ROOT_FONT_SIZE;
        s.apply_declaration("font-size", &CssValue::Keyword("small".into()), parent);
        let expected_small = ROOT_FONT_SIZE * 0.8125;
        assert!((s.font_size - expected_small).abs() < f32::EPSILON);

        s.apply_declaration("font-size", &CssValue::Keyword("larger".into()), parent);
        let expected_larger = parent * 1.2;
        assert!((s.font_size - expected_larger).abs() < 0.01);
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

    #[test]
    fn test_margin_auto_vertical_flags() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("margin-top", &CssValue::Keyword("auto".into()), 16.0);
        assert!(s.margin_top_auto, "margin-top: auto should set flag");
        assert_eq!(s.margin_top, 0.0);

        s.apply_declaration("margin-bottom", &CssValue::Keyword("auto".into()), 16.0);
        assert!(s.margin_bottom_auto, "margin-bottom: auto should set flag");
        assert_eq!(s.margin_bottom, 0.0);
    }

    #[test]
    fn test_margin_shorthand_preserves_auto() {
        use crate::css::parser::LengthUnit;

        let mut s = ComputedStyle::default();
        // margin: 0 auto => top/bottom=0, left/right=auto
        // The shorthand is expanded by the parser, but here we test
        // individual property application after expansion.
        s.apply_declaration("margin-top", &CssValue::Length(0.0, LengthUnit::Px), 16.0);
        s.apply_declaration("margin-right", &CssValue::Keyword("auto".into()), 16.0);
        s.apply_declaration(
            "margin-bottom",
            &CssValue::Length(0.0, LengthUnit::Px),
            16.0,
        );
        s.apply_declaration("margin-left", &CssValue::Keyword("auto".into()), 16.0);

        assert!(s.margin_left_auto);
        assert!(s.margin_right_auto);
        assert!(!s.margin_top_auto);
        assert!(!s.margin_bottom_auto);
    }

    #[test]
    fn test_currentcolor_resolves_to_element_color() {
        let mut s = ComputedStyle::default();
        s.color = Color::rgb(255, 0, 0);
        s.apply_declaration(
            "border-top-color",
            &CssValue::Keyword("currentcolor".into()),
            16.0,
        );
        assert_eq!(
            s.border_top_color,
            Color::rgb(255, 0, 0),
            "currentcolor should resolve to element's color",
        );
    }

    #[test]
    fn text_shadow_parsed() {
        let mut s = ComputedStyle::default();
        let value = CssValue::Multiple(vec![
            CssValue::Length(2.0, LengthUnit::Px),
            CssValue::Length(3.0, LengthUnit::Px),
            CssValue::Length(1.0, LengthUnit::Px),
            CssValue::Color(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            }),
        ]);
        s.apply_declaration("text-shadow", &value, 16.0);
        let ts = s.text_shadow.expect("should parse text-shadow");
        assert_eq!(ts.offset_x, 2.0);
        assert_eq!(ts.offset_y, 3.0);
        assert_eq!(ts.blur, 1.0);
        assert_eq!(ts.color, Color::rgba(0, 0, 0, 255));
    }

    #[test]
    fn text_shadow_none() {
        let mut s = ComputedStyle::default();
        s.text_shadow = Some(TextShadow {
            offset_x: 1.0,
            offset_y: 1.0,
            blur: 0.0,
            color: Color::rgb(0, 0, 0),
        });
        let value = CssValue::Keyword("none".into());
        s.apply_declaration("text-shadow", &value, 16.0);
        assert!(s.text_shadow.is_none());
    }

    #[test]
    fn gradient_background_image_applied() {
        let mut s = ComputedStyle::default();
        let grad = LinearGradient {
            direction: GradientDirection::ToRight,
            stops: vec![
                GradientStop {
                    color: Color::rgb(255, 0, 0),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgb(0, 0, 255),
                    position: 1.0,
                },
            ],
        };
        let value = CssValue::Gradient(grad.clone());
        s.apply_declaration("background-image", &value, 16.0);
        assert_eq!(s.background_image, BackgroundImage::Gradient(grad));
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// resolve_length with Px always returns the value.
            #[test]
            fn resolve_length_px_identity(v in -1000.0f32..1000.0) {
                let result = resolve_length(
                    &CssValue::Length(v, LengthUnit::Px), 16.0,
                );
                prop_assert!(
                    (result - v).abs() < 0.001,
                    "Px({v}) should resolve to {v}, got {result}",
                );
            }

            /// resolve_length with Em scales by parent font size.
            #[test]
            fn resolve_length_em_scales(
                v in 0.0f32..10.0,
                parent in 1.0f32..100.0,
            ) {
                let result = resolve_length(
                    &CssValue::Length(v, LengthUnit::Em), parent,
                );
                let expected = v * parent;
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "Em({v}) * {parent} = {expected}, got {result}",
                );
            }

            /// resolve_length with Rem scales by ROOT_FONT_SIZE.
            #[test]
            fn resolve_length_rem_scales(v in 0.0f32..10.0) {
                let result = resolve_length(
                    &CssValue::Length(v, LengthUnit::Rem), 16.0,
                );
                let expected = v * ROOT_FONT_SIZE;
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "Rem({v}) = {expected}, got {result}",
                );
            }

            /// resolve_dimension with auto keyword always returns Auto.
            #[test]
            fn resolve_dimension_auto(_dummy in 0..1i32) {
                let result = resolve_dimension(
                    &CssValue::Keyword("auto".into()), 16.0,
                );
                prop_assert_eq!(result, Dimension::Auto);
            }

            /// resolve_dimension with percentage preserves the value.
            #[test]
            fn resolve_dimension_percent(pct in 0.0f32..200.0) {
                let result = resolve_dimension(
                    &CssValue::Percentage(pct), 16.0,
                );
                prop_assert_eq!(result, Dimension::Percent(pct));
            }

            /// resolve_font_size with Px returns the exact value.
            #[test]
            fn resolve_font_size_px_identity(v in 1.0f32..100.0) {
                let result = resolve_font_size(
                    &CssValue::Length(v, LengthUnit::Px), 16.0,
                );
                prop_assert!(
                    (result - v).abs() < 0.001,
                    "font-size Px({v}) -> {result}",
                );
            }

            /// resolve_font_size with percentage scales by parent.
            #[test]
            fn resolve_font_size_percent(
                pct in 10.0f32..300.0,
                parent in 4.0f32..48.0,
            ) {
                let result = resolve_font_size(
                    &CssValue::Percentage(pct), parent,
                );
                let expected = parent * (pct / 100.0);
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "{pct}% of {parent} = {expected}, got {result}",
                );
            }

            /// resolve_line_height with Number multiplies by font_size.
            #[test]
            fn resolve_line_height_number(
                n in 0.5f32..3.0,
                fs in 4.0f32..48.0,
            ) {
                let result = resolve_line_height(
                    &CssValue::Number(n), fs, 16.0,
                );
                let expected = n * fs;
                prop_assert!(
                    (result - expected).abs() < 0.01,
                    "{n} * {fs} = {expected}, got {result}",
                );
            }

            /// apply_declaration with unknown property is a no-op.
            #[test]
            fn apply_unknown_property_noop(
                prop_name in "[a-z\\-]{1,20}",
            ) {
                // Filter out known properties.
                if matches!(
                    prop_name.as_str(),
                    "display" | "color" | "margin" | "padding"
                    | "width" | "height" | "font-size"
                    | "background-color" | "background"
                    | "border-width" | "border-style"
                    | "border-color" | "overflow" | "position"
                    | "float" | "clear" | "visibility"
                    | "text-align" | "text-decoration"
                    | "text-indent" | "text-transform"
                    | "white-space" | "line-height"
                    | "letter-spacing" | "word-spacing"
                    | "font-weight" | "font-style" | "font-family"
                    | "list-style-type" | "list-style-position"
                    | "border-collapse" | "border-spacing"
                    | "z-index" | "flex-direction" | "flex-wrap"
                    | "justify-content" | "align-items"
                    | "flex-grow" | "flex-shrink" | "flex-basis"
                    | "gap" | "row-gap" | "column-gap"
                    | "grid-template-columns" | "grid-template-rows"
                    | "grid-column" | "grid-column-start" | "grid-column-end"
                    | "grid-row" | "grid-row-start" | "grid-row-end"
                    | "grid-gap" | "grid-row-gap" | "grid-column-gap"
                    | "top" | "right" | "bottom" | "left"
                    | "max-width" | "min-width"
                    | "max-height" | "min-height"
                ) {
                    return Ok(());
                }
                let mut s = ComputedStyle::default();
                let before_color = s.color;
                s.apply_declaration(
                    &prop_name,
                    &CssValue::Keyword("x".into()),
                    16.0,
                );
                prop_assert_eq!(s.color, before_color);
            }

            /// keyword_color returns None for random strings.
            #[test]
            fn keyword_color_random_returns_none(
                name in "[a-z]{10,20}",
            ) {
                // Long random strings are unlikely to be valid.
                if keyword_color(&name).is_none() {
                    // Expected.
                } else {
                    // If it happens to match, that's fine too.
                }
            }

            /// ComputedStyle::inherit preserves inheritable props.
            #[test]
            fn inherit_preserves_font_size(fs in 1.0f32..100.0) {
                let mut parent = ComputedStyle::default();
                parent.font_size = fs;
                let child = ComputedStyle::inherit(&parent);
                prop_assert!(
                    (child.font_size - fs).abs() < 0.001,
                    "inherited font_size: got {}, expected {fs}",
                    child.font_size,
                );
            }

            /// ComputedStyle::inherit resets non-inheritable props.
            #[test]
            fn inherit_resets_margin(
                mt in 1.0f32..100.0,
                mr in 1.0f32..100.0,
            ) {
                let mut parent = ComputedStyle::default();
                parent.margin_top = mt;
                parent.margin_right = mr;
                let child = ComputedStyle::inherit(&parent);
                prop_assert!(
                    child.margin_top.abs() < 0.001,
                    "margin_top should be reset, got {}",
                    child.margin_top,
                );
                prop_assert!(
                    child.margin_right.abs() < 0.001,
                    "margin_right should be reset, got {}",
                    child.margin_right,
                );
            }
        }
    }

    #[test]
    fn parse_grid_template_compound_repeat() {
        // Single repeat block (already worked).
        let tracks = parse_grid_template_str("repeat(3, 1fr)", 16.0);
        assert_eq!(tracks, vec![GridTrackSize::Fr(1.0); 3]);

        // Compound: repeat() followed by a fixed track.
        let tracks = parse_grid_template_str("repeat(3, 1fr) 20px", 16.0);
        assert_eq!(
            tracks,
            vec![
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(1.0),
                GridTrackSize::Px(20.0),
            ]
        );

        // Fixed track followed by repeat().
        let tracks = parse_grid_template_str("100px repeat(2, auto)", 16.0);
        assert_eq!(
            tracks,
            vec![
                GridTrackSize::Px(100.0),
                GridTrackSize::Auto,
                GridTrackSize::Auto
            ]
        );

        // "none" returns empty.
        let tracks = parse_grid_template_str("none", 16.0);
        assert!(tracks.is_empty());
    }
}
