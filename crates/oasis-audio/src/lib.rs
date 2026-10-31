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
//! - [`SfxPlayer`] -- UI sound effects, mixed from [`decode_wav`] clips.
//! - [`decode_wav`] / [`decode_ogg`] -- WAV and Ogg Vorbis decoding.

#![warn(missing_docs)]

mod controller;
mod manager;
mod mixer;
mod null_backend;
mod ogg;
mod playlist;
pub mod radio;
mod sfx;
mod types;
mod wav;

pub use controller::{MusicController, queue_request};
pub use manager::{AudioManager, MediaAction};
pub use null_backend::NullAudioBackend;
pub use oasis_types::ipc::{
    AUDIO_POSITION_PATH, AUDIO_REQUEST_PATH, AUDIO_STATUS_PATH, MediaRequest,
};
pub use ogg::{OggData, decode_ogg, is_ogg};
pub use playlist::{Playlist, format_duration, format_playlist};
pub use radio::{RADIO_APP_TITLE, RADIO_REQUEST_PATH, RADIO_STATUS_PATH, RadioManager};
pub use sfx::SfxPlayer;
pub use types::{PlaybackState, RepeatMode, TrackInfo};
pub use wav::{WavData, decode_wav, is_wav};
