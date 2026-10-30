//! Audio subsystem -- music playlist/playback state, internet radio, SFX,
//! and WAV / Ogg Vorbis decoding.
//!
//! The crate is backend-agnostic. Actual audio output goes through the
//! `AudioBackend` trait (defined in `oasis-types/src/backend/audio.rs`),
//! implemented per platform: SDL3 on desktop/Pi (`oasis-backend-sdl`),
//! Web Audio on WASM, a software ring for UE5, and the Media Engine path
//! on PSP.
//!
//! - [`AudioManager`] -- playlist + playback state machine behind the
//!   `/var/audio/*` VFS IPC ([`oasis_types::ipc::MediaRequest`]); the host
//!   carries out the [`MediaAction`]s it returns.
//! - [`MusicController`] -- carries those actions out on an
//!   `AudioBackend` and publishes position/status; shared by every host
//!   that runs the Music Player (desktop shell, WASM).
//! - [`RadioManager`] -- internet-radio streaming state machine.
//! - [`sfx`] -- UI sound effects.

pub mod controller;
pub mod manager;
pub mod mixer;
pub mod null_backend;
pub mod ogg;
pub mod playlist;
pub mod radio;
pub mod sfx;
pub mod types;
pub mod wav;

pub use controller::MusicController;
pub use manager::{AudioManager, MediaAction};
pub use null_backend::NullAudioBackend;
pub use oasis_types::ipc::{
    AUDIO_POSITION_PATH, AUDIO_REQUEST_PATH, AUDIO_STATUS_PATH, MediaRequest,
};
pub use playlist::{Playlist, format_duration, format_playlist};
pub use radio::{RADIO_APP_TITLE, RADIO_REQUEST_PATH, RADIO_STATUS_PATH, RadioManager};
pub use types::{PlaybackState, RepeatMode, TrackInfo};
