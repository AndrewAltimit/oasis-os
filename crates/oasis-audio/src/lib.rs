//! Audio subsystem -- playlist management, playback state, and manager.
//!
//! The core audio module is backend-agnostic. It manages track metadata,
//! playlists, and playback state. Actual audio decoding and output is
//! handled by the `AudioBackend` trait (defined in `backend.rs`) which
//! is implemented per-platform: rodio/SDL2_mixer on desktop/Pi, Media
//! Engine offloading on PSP.

pub mod audio_queue;
pub mod manager;
pub mod mixer;
pub mod null_backend;
pub mod ogg;
pub mod playlist;
pub mod radio;
pub mod types;
pub mod wav;

pub use manager::{AUDIO_REQUEST_PATH, AUDIO_STATUS_PATH, AudioManager};
pub use null_backend::NullAudioBackend;
pub use playlist::{Playlist, format_duration, format_playlist};
pub use radio::{RADIO_APP_TITLE, RADIO_REQUEST_PATH, RADIO_STATUS_PATH, RadioManager};
pub use types::{PlaybackState, RepeatMode, TrackInfo};
