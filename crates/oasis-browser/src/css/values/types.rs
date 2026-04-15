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
    InlineFlex,
    Grid,
    InlineGrid,
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

    /// CSS Fonts Level 4 §4.5: relative `bolder` stepping.
    pub fn bolder(self) -> Self {
        if self.0 < 350 {
            Self(400)
        } else if self.0 < 550 {
            Self(700)
        } else {
            Self(900)
        }
    }

    /// CSS Fonts Level 4 §4.5: relative `lighter` stepping.
    pub fn lighter(self) -> Self {
        if self.0 < 550 {
            Self(100)
        } else if self.0 < 750 {
            Self(400)
        } else {
            Self(700)
        }
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

/// CSS `text-decoration-line` as bitflags (supports `underline line-through`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextDecorationLine(pub u8);

impl TextDecorationLine {
    pub const NONE: Self = Self(0);
    pub const UNDERLINE: Self = Self(1);
    pub const LINE_THROUGH: Self = Self(2);
    pub const OVERLINE: Self = Self(4);

    pub fn has_underline(self) -> bool {
        self.0 & Self::UNDERLINE.0 != 0
    }

    pub fn has_line_through(self) -> bool {
        self.0 & Self::LINE_THROUGH.0 != 0
    }

    pub fn has_overline(self) -> bool {
        self.0 & Self::OVERLINE.0 != 0
    }

    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOrAssign for TextDecorationLine {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
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
        line: TextDecorationLine::NONE,
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

/// CSS `text-wrap` property. Controls how text flows to new lines.
/// `balance` and `pretty` are recognised as hints but not yet
/// implemented in layout — they fall through to `wrap` behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextWrap {
    /// Default. Lines wrap on soft line-break opportunities.
    Wrap,
    /// No line wrapping. Equivalent to `white-space: nowrap`.
    Nowrap,
    /// Layout should minimise the difference in line widths.
    /// Currently parsed and stored but not applied.
    Balance,
    /// Layout should avoid orphans / short last lines.
    /// Currently parsed and stored but not applied.
    Pretty,
    /// Like `wrap` but line breaks remain stable as the element
    /// resizes. Currently parsed and stored but not applied.
    Stable,
}

/// CSS `container-type` property — declares whether an element
/// establishes a query container, and along which axes its size can
/// be queried by descendant `@container` rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerType {
    /// Default. Element is not a query container.
    Normal,
    /// Element is a query container for inline-axis size queries
    /// (`inline-size`, `width` in horizontal-LTR writing modes).
    InlineSize,
    /// Element is a query container for both inline- and block-axis
    /// size queries.
    Size,
}

/// CSS `field-sizing` property. Default `Fixed` keeps form controls
/// at their declared / `size`-attribute width. `Content` lets the
/// control track its own value (or placeholder) width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSizing {
    Fixed,
    Content,
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
    /// `translate3d(tx, ty, tz)` in pixels.
    Translate3d(f32, f32, f32),
    /// `translateZ(tz)` in pixels.
    TranslateZ(f32),
    /// `scale3d(sx, sy, sz)`.
    Scale3d(f32, f32, f32),
    /// `scaleZ(sz)`.
    ScaleZ(f32),
    /// `rotateX(angle)` in degrees.
    RotateX(f32),
    /// `rotateY(angle)` in degrees.
    RotateY(f32),
    /// `rotateZ(angle)` in degrees. Equivalent to 2D `rotate()`.
    RotateZ(f32),
    /// `rotate3d(x, y, z, angle_deg)` — rotation around axis `(x, y, z)`.
    Rotate3d(f32, f32, f32, f32),
    /// `matrix3d(...)` — 16 column-major values of a 4×4 matrix.
    Matrix3d([f32; 16]),
    /// `perspective(d)` in pixels. Applies a perspective projection with
    /// the viewer at distance `d` along +Z.
    Perspective(f32),
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

/// CSS `clip-path` basic shape.
///
/// Values are resolved to pixel offsets relative to the element's border
/// box at style-apply time. Percentage inputs are stored as pre-resolved
/// fractions (0.0..=1.0) and multiplied by the border box at paint time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPath {
    /// `inset(top right bottom left)` — insets from the border box edges.
    /// Each component is either a pixel length or a fraction (0..=1).
    Inset {
        top: ClipLength,
        right: ClipLength,
        bottom: ClipLength,
        left: ClipLength,
    },
    /// `rect(top, right, bottom, left)` — legacy rect form, pixel values
    /// measured from the border box top-left.  `None` = `auto` (use the
    /// border-box edge), resolved at paint time.
    Rect {
        top: Option<f32>,
        right: Option<f32>,
        bottom: Option<f32>,
        left: Option<f32>,
    },
    /// `circle(r at cx cy)` — approximated to its bounding box for now.
    Circle {
        cx: ClipLength,
        cy: ClipLength,
        r: ClipLength,
    },
    /// `ellipse(rx ry at cx cy)` — approximated to its bounding box.
    Ellipse {
        cx: ClipLength,
        cy: ClipLength,
        rx: ClipLength,
        ry: ClipLength,
    },
}

/// A length that is either an absolute pixel value or a fraction of the
/// reference box (used for percentage inputs to `clip-path`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipLength {
    Px(f32),
    /// 0.0..=1.0 fraction of the reference dimension.
    Frac(f32),
}

impl ClipLength {
    /// Resolve against a reference length (e.g. border-box width or height).
    pub fn resolve(self, reference: f32) -> f32 {
        match self {
            ClipLength::Px(v) => v,
            ClipLength::Frac(f) => f * reference,
        }
    }
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

/// CSS `appearance` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Auto,
    None,
}

/// CSS `color-scheme` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Normal,
    Light,
    Dark,
    LightDark,
}

/// CSS `isolation` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    Auto,
    Isolate,
}

/// CSS `resize` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    None,
    Both,
    Horizontal,
    Vertical,
}

/// CSS `touch-action` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchAction {
    Auto,
    None,
    Manipulation,
    PanX,
    PanY,
}

/// Re-export `TextDirection` from `oasis-types` for CSS `direction` property.
pub use oasis_types::text_direction::TextDirection;

// -----------------------------------------------------------------------
// Extended property types (CSS coverage expansion)
// -----------------------------------------------------------------------

/// CSS `scroll-behavior` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollBehavior {
    Auto,
    Smooth,
}

/// CSS `mix-blend-mode` / `background-blend-mode` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

/// CSS `background-clip` / `background-origin` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundBox {
    BorderBox,
    PaddingBox,
    ContentBox,
    /// `background-clip: text` only — clips to text glyphs.
    Text,
}

/// CSS `mask-mode` — how the mask image's channel is interpreted.
///
/// Added for compositor overhaul PR6. Parsed today; the actual
/// destination-in composite path wires up in a follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskMode {
    /// Use alpha channel on images with alpha, luminance otherwise.
    #[default]
    MatchSource,
    /// Use luminance channel.
    Luminance,
    /// Use alpha channel.
    Alpha,
}

/// CSS `mask-composite` — how multiple mask layers combine.
///
/// Only single-layer masks are currently wired up; the multi-layer
/// composite operations are parsed but reduce to `Add` at paint time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskComposite {
    /// `source-over` semantics — default single-layer behavior.
    #[default]
    Add,
    /// `xor` of the two mask layers.
    Subtract,
    /// `in` — intersect the two layers.
    Intersect,
    /// `xor` — symmetric difference.
    Exclude,
}

/// CSS `image-rendering` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRendering {
    Auto,
    CrispEdges,
    Pixelated,
}

/// CSS `font-variant` (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontVariant {
    Normal,
    SmallCaps,
}

/// CSS `font-stretch` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStretch {
    UltraCondensed,
    ExtraCondensed,
    Condensed,
    SemiCondensed,
    Normal,
    SemiExpanded,
    Expanded,
    ExtraExpanded,
    UltraExpanded,
}

/// CSS `font-kerning` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontKerning {
    Auto,
    Normal,
    None,
}

/// CSS `hyphens` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hyphens {
    None,
    Manual,
    Auto,
}

/// CSS `backface-visibility` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfaceVisibility {
    Visible,
    Hidden,
}

/// CSS `transform-style` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformStyle {
    Flat,
    Preserve3d,
}

/// CSS `text-align-last` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignLast {
    Auto,
    Left,
    Right,
    Center,
    Justify,
    Start,
    End,
}

/// CSS `text-justify` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextJustify {
    Auto,
    InterWord,
    InterCharacter,
    None,
}

/// CSS `text-underline-position` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextUnderlinePosition {
    Auto,
    Under,
    Left,
    Right,
}

/// CSS `text-rendering` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRendering {
    Auto,
    OptimizeSpeed,
    OptimizeLegibility,
    GeometricPrecision,
}

/// CSS `scroll-snap-align` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollSnapAlign {
    None,
    Start,
    End,
    Center,
}

/// CSS `scroll-snap-stop` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollSnapStop {
    Normal,
    Always,
}

/// CSS `overscroll-behavior` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverscrollBehavior {
    Auto,
    Contain,
    None,
}

/// CSS `content-visibility` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentVisibility {
    Visible,
    Auto,
    Hidden,
}

/// CSS `justify-self` / `justify-items` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifySelf {
    Auto,
    Start,
    End,
    Center,
    Stretch,
    FlexStart,
    FlexEnd,
}
