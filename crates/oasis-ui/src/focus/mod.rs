//! Keyboard focus navigation for widget groups.
//!
//! `FocusRing` tracks which widget index has focus and handles
//! directional navigation (next/prev/wrapping). Widgets query the
//! ring to determine their visual focus state.
//!
//! `FocusManager` builds on `FocusRing` to provide Tab/Shift-Tab
//! keyboard cycling, skip-disabled item logic, and visual focus
//! indicator drawing through `FocusStyle`.
//!
//! `SpatialFocusManager` adds spatial (arrow-key) navigation and
//! tab-index ordering over a set of `FocusableItem`s with bounds.
//! It tracks whether focus was activated via keyboard (showing a
//! focus ring) or hidden by mouse interaction.

mod manager;
mod ring;
mod spatial;
mod style;

pub use manager::{FocusAction, FocusManager};
pub use ring::{FocusDir, FocusRing, step_wrapping};
pub use spatial::{FocusDirection, FocusEvent, FocusableItem, Rect, SpatialFocusManager};
pub use style::{FocusStyle, draw_focus_ring, draw_focus_ring_themed};
