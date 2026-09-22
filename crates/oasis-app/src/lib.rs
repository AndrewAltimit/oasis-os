//! OASIS_OS desktop shell as a library.
//!
//! The `oasis-app` binary (`main.rs`) is a thin host around [`shell::Shell`]:
//! it opens the SDL window and audio device, plays the boot splash and runs
//! the frame loop. Keeping the shell itself in this library lets the
//! headless end-to-end harness ([`harness`]) drive the exact same boot
//! sequence, input dispatch, per-frame ticking and rendering against a
//! software framebuffer — see `docs/testing.md`.

pub mod app_state;
pub mod audio_out;
pub mod boot_splash;
mod commands;
mod frame_stats;
pub mod harness;
pub mod headless;
#[cfg(feature = "skin-dev")]
mod hot_reload;
mod icon_drag;
mod input;
mod launch;
#[cfg(feature = "mcp")]
mod mcp_command;
#[cfg(feature = "mcp")]
mod mcp_tools;
mod media_controller;
mod net_fetch;
mod radio_controller;
#[cfg(test)]
mod radio_soak;
mod render;
pub mod shell;
pub mod shell_backend;
mod sysinfo;
mod terminal_input;
mod tv_controller;
mod ui_sfx;
mod user_prefs;
mod vfs_setup;
mod video_player;

pub use app_state::Mode;
pub use shell::{BootObserver, BootOptions, NoSplash, Shell, StepOutcome};
pub use shell_backend::ShellBackend;

// Background fetch helpers, reached as `crate::...` / `super::...` by the
// radio and TV controllers.
#[cfg(test)]
pub(crate) use net_fetch::connect_archive_source;
pub(crate) use net_fetch::{
    connect_archive_track_sync, fetch_catalog_blocking, fetch_tv_catalogs_blocking,
    parse_stream_url,
};
