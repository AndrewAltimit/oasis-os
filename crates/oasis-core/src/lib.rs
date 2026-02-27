//! OASIS_OS core framework.
//!
//! Platform-agnostic embeddable OS framework providing a scene graph (SDI),
//! backend abstraction traits, input event pipeline, configuration, and
//! error types. This crate has zero platform dependencies.
//!
//! # Public API
//!
//! The [`prelude`] module re-exports the most commonly used types for
//! convenient single-import access. Individual sub-crate modules are also
//! available for full access.

// -----------------------------------------------------------------------
// Re-exports from oasis-types (foundation types and traits)
// -----------------------------------------------------------------------

pub use oasis_types::backend;
pub use oasis_types::config;
pub use oasis_types::error;
pub use oasis_types::input;
pub use oasis_types::tls;

// Internal-only type re-exports (not part of the primary API surface).
#[doc(hidden)]
pub use oasis_types::color;
#[doc(hidden)]
pub use oasis_types::pbp;
#[doc(hidden)]
pub use oasis_types::shadow;

// -----------------------------------------------------------------------
// Sub-crate re-exports
// -----------------------------------------------------------------------

pub use oasis_audio as audio;
pub use oasis_browser as browser;
pub use oasis_net as net;
pub use oasis_platform as platform;
pub use oasis_sdi as sdi;
pub use oasis_skin as skin;
pub use oasis_skin::active_theme;
#[doc(hidden)]
pub use oasis_skin::legacy_theme as theme;
pub use oasis_ui as ui;
pub use oasis_vfs as vfs;
pub use oasis_wm as wm;

// -----------------------------------------------------------------------
// Core-owned modules
// -----------------------------------------------------------------------

pub mod agent;
pub mod apps;
pub mod bottombar;
pub mod cursor;
pub mod dashboard;
pub mod osk;
pub mod plugin;
pub mod script;
pub mod startmenu;
pub mod statusbar;
pub mod terminal;
pub mod terminal_sdi;
pub mod toast;
pub mod transfer;
pub mod transition;
pub mod update;
pub mod wallpaper;

// -----------------------------------------------------------------------
// Prelude -- curated public API surface
// -----------------------------------------------------------------------

/// Commonly used types and traits for embedding OASIS_OS.
///
/// ```ignore
/// use oasis_core::prelude::*;
/// ```
pub mod prelude {
    // Backend traits
    pub use oasis_types::backend::{
        AudioBackend, AudioTrackId, Color, InputBackend, NetworkBackend, SdiBackend, TextureId,
    };

    // Error handling
    pub use oasis_types::error::{OasisError, Result};

    // Input
    pub use oasis_types::input::{Button, InputEvent, Trigger};

    // Configuration
    pub use oasis_types::config::OasisConfig;

    // Scene graph
    pub use oasis_sdi::SdiRegistry;

    // Skin / theme
    pub use oasis_skin::active_theme::ActiveTheme;
    pub use oasis_skin::{Skin, SkinFeatures, resolve_skin};

    // VFS
    pub use oasis_vfs::{MemoryVfs, Vfs};

    // Platform
    pub use oasis_platform::DesktopPlatform;

    // Terminal
    pub use crate::terminal::{CommandOutput, CommandRegistry, Environment, register_builtins};

    // Apps / Dashboard
    pub use crate::apps::{AppAction, AppRunner};
    pub use crate::dashboard::{DashboardConfig, DashboardState, discover_apps};

    // Window management
    pub use oasis_wm::manager::WindowManager;
    pub use oasis_wm::window::{WindowConfig, WindowType};

    // UI chrome
    pub use crate::bottombar::BottomBar;
    pub use crate::cursor::CursorState;
    pub use crate::startmenu::StartMenuState;
    pub use crate::statusbar::StatusBar;
}
