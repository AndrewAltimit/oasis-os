//! Window manager.
//!
//! Enables skins that present multiple movable, resizable, overlapping
//! windows. The WM is a consumer of the SDI API -- it creates, positions,
//! and manipulates groups of SDI objects to simulate windowed interfaces.
//! SDI remains a flat, dumb scene graph; the WM is the smart layer on top.

pub mod animation;
pub mod desktops;
mod drag_resize;
pub mod hit_test;
pub mod manager;
mod sdi_objects;
pub mod snap;
pub mod tiling;
pub mod window;

pub use animation::{
    AnimationDurations, AnimationFrame, AnimationKind, AnimationManager, AnimationState,
};
pub use desktops::{DesktopId, DesktopManager, WindowPlacement};
pub use hit_test::{ButtonKind, HitRegion, ResizeEdge};
pub use manager::{WindowManager, WmEvent};
pub use snap::{KeyboardSnapDirection, SnapManager, SnapPreview, SnapZone, keyboard_snap};
pub use tiling::{
    TileGeometry, TilingConfig, TilingLayout, TilingManager, adjust_master_ratio, cycle_layout,
};
pub use window::{Geometry, Window, WindowConfig, WindowId, WindowState, WindowType, WmTheme};
