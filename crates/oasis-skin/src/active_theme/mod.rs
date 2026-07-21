//! Runtime theme derived from the active skin.
//!
//! `ActiveTheme` replaces the hardcoded constants in `theme.rs` with a runtime
//! struct whose fields are derived from the skin's 9 base colors. Consumers
//! receive `&ActiveTheme` instead of reading `theme::CONST` directly, allowing
//! skins to actually drive the UI appearance.
//!
//! The theme is decomposed into focused sub-structs:
//! - [`BarTheme`] -- status bar, bottom bar, tab pills, page dots
//! - [`IconTheme`] -- dashboard icon rendering and cursor highlight
//! - [`StartMenuTheme`] -- start button, popup panel, item grid
//! - [`AppScreenTheme`] -- app content area, title bar, terminal, selection
//! - [`OskTheme`] -- on-screen keyboard colors
//! - [`ScrollbarTheme`] -- scrollbar track, thumb, dimensions
//! - [`WallpaperTheme`] -- wallpaper style, gradient stops, effects
//! - [`ToastTheme`] -- toast notification colors and layout

mod defaults;
mod derive;
mod methods;
mod structs;
mod tests;

pub use structs::{
    ActiveTheme, AnsiPalette, AppScreenTheme, BarTheme, IconTheme, ImageLayerTheme, OskTheme,
    ScrollbarTheme, StartMenuTheme, ToastTheme, WallpaperTheme,
};
