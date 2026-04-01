//! Internet Archive TV Guide — retro EPG with deterministic scheduling.
//!
//! Provides a cable-TV-style channel grid with video content from Internet
//! Archive collections. Each channel has a deterministic schedule seeded by
//! channel number so every instance computes the same "what's on" result
//! for any given Unix timestamp.

pub mod app;
pub mod catalog;
pub mod channel;
pub mod grid_layout;
pub mod grid_render;
pub mod grid_state;
pub mod guide;
pub mod schedule;
#[cfg(any(test, feature = "test-data"))]
pub mod test_data;

pub use app::TvGuideApp;
pub use catalog::{
    ChannelCatalog, VideoEpisode, VideoFormat, select_smallest_for, select_smallest_with_max_width,
};
pub use channel::{Channel, ChannelConfig, ChannelSource};
pub use guide::TvGuideState;
pub use schedule::{CachedSchedule, ScheduleSlot, schedule_at, schedule_range};

/// VFS path for TV channel configuration.
pub const TV_CHANNELS_PATH: &str = "/etc/tv/channels.toml";

/// VFS path for TV playback requests (IPC).
pub const TV_REQUEST_PATH: &str = "/var/tv/request";

/// VFS path for TV playback status (IPC).
pub const TV_STATUS_PATH: &str = "/var/tv/status";
