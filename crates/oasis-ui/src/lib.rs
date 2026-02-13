//! oasis-ui: Higher-level UI abstractions built on `SdiBackend`.
//!
//! This module provides layout helpers, themed drawing context, animation
//! primitives, and a widget toolkit. All rendering goes through `SdiBackend`
//! trait methods -- no platform-specific code.

pub mod animation;
pub mod avatar;
pub mod badge;
pub mod button;
pub mod card;
pub use oasis_types::color;
pub mod context;
pub mod divider;
pub mod flex;
pub mod icon;
pub mod input_field;
pub mod layout;
pub mod list_view;
pub mod nine_patch;
pub mod panel;
pub mod progress_bar;
pub mod scroll_view;
pub use oasis_types::shadow;
pub mod tab_bar;
pub mod text_block;
pub mod theme;
pub mod toggle;
pub mod widget;

#[cfg(test)]
pub(crate) mod test_utils;

pub use context::DrawContext;
pub use layout::Padding;
pub use theme::Theme;
pub use widget::Widget;
