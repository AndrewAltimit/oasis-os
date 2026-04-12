//! CSS computed value types.
//!
//! These types represent the *computed* values after cascade resolution -- the
//! final concrete values consumed by the layout engine. Every property has a
//! single canonical representation (e.g. all lengths are resolved to `f32`
//! pixels, all colors to `Color`).

mod apply;
mod computed;
pub(crate) mod resolve;
pub mod types;

// Re-export everything so the public API is unchanged.
pub use computed::ComputedStyle;
#[allow(unused_imports)]
pub use types::{
    AlignContent, AlignItems, AlignSelf, Animation, AnimationDirection, AnimationFillMode,
    AnimationPlayState, Appearance, BackgroundImage, BackgroundPosition, BackgroundRepeat,
    BackgroundSize, BorderCollapse, BorderRadius, BorderStyle, BoxShadow, BoxSizing, Clear,
    ClipLength, ClipPath, ColorScheme, Cursor, Dimension, Display, FilterFunction, FlexDirection,
    FlexWrap, Float, FontFamily, FontStyle, FontWeight, GradientDirection, GradientStop,
    GridTrackSize, Isolation, JustifyContent, LinearGradient, ListStylePosition, ListStyleType,
    ObjectFit, ObjectPosition, Overflow, OverflowWrap, PointerEvents, Position, ROOT_FONT_SIZE,
    RadialGradient, Resize, TextAlign, TextDecoration, TextDecorationLine, TextDecorationStyle,
    TextDirection, TextOverflow, TextShadow, TextTransform, TimingFunction, TouchAction,
    TransformFunction, TransformOrigin, Transition, UserSelect, VerticalAlign, Visibility,
    WhiteSpace, WordBreak,
};
