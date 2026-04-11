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
    Groove,
    Ridge,
    Inset,
    Outset,
}

/// CSS `font-weight` property.
///
/// Stores the numeric weight (100–900). The `Normal` and `Bold` constants
/// map to 400 and 700 respectively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontWeight(pub u16);

impl FontWeight {
    /// Normal weight (400).
    pub const NORMAL: Self = Self(400);
    /// Bold weight (700).
    pub const BOLD: Self = Self(700);

    /// Returns `true` when the weight is bold (≥ 600).
    pub fn is_bold(self) -> bool {
        self.0 >= 600
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
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

/// CSS `text-decoration-line` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecorationLine {
    None,
    Underline,
    LineThrough,
    Overline,
}

/// CSS `text-decoration-style` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecorationStyle {
    Solid,
    Dashed,
    Dotted,
    Double,
    Wavy,
}

/// Combined CSS text-decoration properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextDecoration {
    pub line: TextDecorationLine,
    pub style: TextDecorationStyle,
    pub color: Option<Color>,
}

impl TextDecoration {
    pub const NONE: Self = Self {
        line: TextDecorationLine::None,
        style: TextDecorationStyle::Solid,
        color: None,
    };
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

/// CSS `object-fit` property for replaced elements (images, video).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFit {
    /// Scale the content to fill the box, distorting aspect ratio.
    Fill,
    /// Scale to fit within the box, preserving aspect ratio (letterbox).
    Contain,
    /// Scale to cover the box, preserving aspect ratio (crop).
    Cover,
    /// No scaling, use intrinsic size.
    None,
    /// Like `none` or `contain`, whichever is smaller.
    ScaleDown,
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

/// CSS `background-size` property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundSize {
    /// `auto` — intrinsic size.
    Auto,
    /// `cover` — scale to cover the entire area.
    Cover,
    /// `contain` — scale to fit within the area.
    Contain,
    /// Explicit width and height (pixels). `None` means `auto` for that axis.
    Explicit(Option<f32>, Option<f32>),
}

/// CSS `background-position` property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPosition {
    /// Horizontal offset as fraction (0.0 = left, 0.5 = center, 1.0 = right)
    /// or pixel value stored via `x_is_px`.
    pub x: f32,
    /// Vertical offset as fraction (0.0 = top, 0.5 = center, 1.0 = bottom)
    /// or pixel value stored via `y_is_px`.
    pub y: f32,
    /// When `true`, `x` is in pixels rather than fraction of (container - image).
    pub x_is_px: bool,
    /// When `true`, `y` is in pixels rather than fraction of (container - image).
    pub y_is_px: bool,
}

impl Default for BackgroundPosition {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            x_is_px: false,
            y_is_px: false,
        }
    }
}

/// CSS `background-repeat` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeat {
    Repeat,
    NoRepeat,
    RepeatX,
    RepeatY,
}

/// CSS `background-image` property.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    None,
    Url(String),
    Gradient(LinearGradient),
    RadialGradient(RadialGradient),
}

/// CSS `border-radius` with per-corner values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderRadius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl BorderRadius {
    pub const ZERO: Self = Self {
        top_left: 0.0,
        top_right: 0.0,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };

    /// Create a uniform border-radius.
    pub fn uniform(r: f32) -> Self {
        Self {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }

    /// Returns `true` if all corners are zero.
    pub fn is_zero(&self) -> bool {
        self.top_left == 0.0
            && self.top_right == 0.0
            && self.bottom_right == 0.0
            && self.bottom_left == 0.0
    }

    /// Returns `true` if all corners are the same value.
    pub fn is_uniform(&self) -> bool {
        self.top_left == self.top_right
            && self.top_right == self.bottom_right
            && self.bottom_right == self.bottom_left
    }

    /// Returns the maximum corner radius (used for single-value fallback).
    pub fn max_radius(&self) -> f32 {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }
}

impl Default for BorderRadius {
    fn default() -> Self {
        Self::ZERO
    }
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

/// CSS `cursor` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Auto,
    Default,
    Pointer,
    Text,
    Move,
    NotAllowed,
    Crosshair,
    Wait,
    Help,
    Grab,
    Grabbing,
    ColResize,
    RowResize,
    NResize,
    EResize,
    SResize,
    WResize,
    NeResize,
    NwResize,
    SeResize,
    SwResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ZoomIn,
    ZoomOut,
    None,
}

/// CSS `pointer-events` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEvents {
    Auto,
    None,
}

/// CSS `user-select` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserSelect {
    Auto,
    None,
    Text,
    All,
}

/// CSS `object-position` property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectPosition {
    /// Horizontal position as fraction (0.0 = left, 0.5 = center, 1.0 = right)
    /// or pixel value when `x_is_px` is true.
    pub x: f32,
    /// Vertical position as fraction (0.0 = top, 0.5 = center, 1.0 = bottom)
    /// or pixel value when `y_is_px` is true.
    pub y: f32,
    pub x_is_px: bool,
    pub y_is_px: bool,
}

impl Default for ObjectPosition {
    fn default() -> Self {
        // CSS default: 50% 50% (centered)
        Self {
            x: 0.5,
            y: 0.5,
            x_is_px: false,
            y_is_px: false,
        }
    }
}

/// Re-export `TextDirection` from `oasis-types` for CSS `direction` property.
pub use oasis_types::text_direction::TextDirection;
