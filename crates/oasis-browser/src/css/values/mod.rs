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
    AlignItems, BackgroundImage, BorderCollapse, BorderStyle, BoxShadow, BoxSizing, Clear,
    Dimension, Display, FlexDirection, FlexWrap, Float, FontFamily, FontStyle, FontWeight,
    GradientDirection, GradientStop, GridTrackSize, JustifyContent, LinearGradient,
    ListStylePosition, ListStyleType, Overflow, OverflowWrap, Position, ROOT_FONT_SIZE, TextAlign,
    TextDecoration, TextDirection, TextOverflow, TextShadow, TextTransform, VerticalAlign,
    Visibility, WhiteSpace, WordBreak,
};
