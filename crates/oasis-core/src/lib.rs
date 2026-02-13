//! OASIS_OS core framework.
//!
//! Platform-agnostic embeddable OS framework providing a scene graph (SDI),
//! backend abstraction traits, input event pipeline, configuration, and
//! error types. This crate has zero platform dependencies.

// Re-exports from oasis-types (foundation types and traits).
pub use oasis_types::backend;
pub use oasis_types::color;
pub use oasis_types::config;
pub use oasis_types::error;
pub use oasis_types::input;
pub use oasis_types::pbp;
pub use oasis_types::shadow;
pub use oasis_types::tls;

pub mod active_theme;
pub mod agent;
pub mod apps;
pub mod audio;
pub mod bottombar;
pub mod browser;
pub mod cursor;
pub mod dashboard;
pub use oasis_net as net;
pub mod osk;
pub use oasis_platform as platform;
pub mod plugin;
pub mod script;
pub use oasis_sdi as sdi;
pub mod skin;
pub mod startmenu;
pub mod statusbar;
pub mod terminal;
pub mod theme;
pub mod transfer;
pub mod transition;
pub mod ui;
pub mod update;
pub use oasis_vfs as vfs;
pub mod wallpaper;
pub mod wm;
