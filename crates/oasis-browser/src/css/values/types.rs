//! CSS value type definitions (enums and structs).

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

/// CSS `align-content` property (cross-axis line distribution in multi-line flex).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

/// CSS `align-self` property (per-item cross-axis override).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignSelf {
    Auto,
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
    MinContent,
    MaxContent,
    FitContent,
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
    Sticky,
}

/// CSS `overflow` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
    Auto,
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
    Sub,
    Super,
    Length(f32),
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
    /// When `true`, gradient stops repeat (tiled) for
    /// `repeating-linear-gradient()`.
    pub repeating: bool,
}

/// A parsed CSS `radial-gradient(...)` value.
#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    /// `true` = circle, `false` = ellipse (default).
    pub shape_circle: bool,
    pub stops: Vec<GradientStop>,
}

/// CSS `background-image` property.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    None,
    Url(String),
    Gradient(LinearGradient),
    RadialGradient(RadialGradient),
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
    pub inset: bool,
}

/// A single CSS Grid track size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackSize {
    Px(f32),
    Fr(f32),
    Auto,
    /// `minmax(min_px, max_px)` — max is `f32::MAX` for `auto`.
    Minmax(f32, f32),
}

/// A CSS transform function.
#[derive(Debug, Clone, PartialEq)]
pub enum TransformFunction {
    /// `translate(x, y)` in pixels.
    Translate(f32, f32),
    /// `scale(sx, sy)`.
    Scale(f32, f32),
    /// `rotate(angle)` in degrees.
    Rotate(f32),
    /// `skew(ax, ay)` in degrees.
    Skew(f32, f32),
    /// `matrix(a, b, c, d, e, f)` — 2D affine transform.
    Matrix(f32, f32, f32, f32, f32, f32),
}

/// CSS `transform-origin` value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformOrigin {
    pub x: f32,
    pub y: f32,
    pub x_pct: Option<f32>,
    pub y_pct: Option<f32>,
}

/// CSS `filter` function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterFunction {
    Blur(f32),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
    HueRotate(f32),
}

/// A CSS easing function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
}

/// A single CSS transition declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub property: String,
    pub duration_ms: f32,
    pub timing: TimingFunction,
    pub delay_ms: f32,
}

/// CSS animation fill mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

/// CSS animation direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// CSS animation play state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPlayState {
    Running,
    Paused,
}

/// A single CSS animation declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub name: String,
    pub duration_ms: f32,
    pub timing: TimingFunction,
    pub delay_ms: f32,
    /// Iteration count. Use `f32::INFINITY` for infinite.
    pub iteration_count: f32,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

/// Re-export `TextDirection` from `oasis-types` for CSS `direction` property.
pub use oasis_types::text_direction::TextDirection;
