//! The WASM host's music controller: serves the Music Player app and the
//! `music` command (`/var/audio/*` VFS IPC) through the shared
//! [`oasis_audio::MusicController`], on the `<audio>`-element backend in
//! [`crate::audio`].
//!
//! TV Guide video plays through its own `<video>` element here, not the
//! audio backend, so music never shares the backend's stream
//! (`stream_shared` is always `false`); internet radio does, and is torn
//! down before a track starts (and the other way round in the radio tune
//! path, see `OasisWasm::tick`).

use oasis_core::backend::AudioBackend;

use crate::OasisWasm;

impl OasisWasm {
    /// Carry out pending music requests and publish position + status.
    pub(crate) fn tick_music(&mut self) {
        let Self {
            media,
            media_track,
            audio,
            vfs,
            radio_manager,
            radio_source,
            archive_catalog,
            pending_catalog,
            ..
        } = self;
        let mut stop_radio = |backend: &mut dyn AudioBackend| {
            if radio_manager.state() == oasis_audio::radio::RadioState::Stopped {
                return;
            }
            let _ = radio_manager.process_request("stop", backend);
            *archive_catalog = None;
            *pending_catalog = None;
            if let Some(mut src) = radio_source.take() {
                src.disconnect();
            }
        };
        oasis_audio::MusicController {
            manager: media,
            track: media_track,
            audio,
            stream_shared: false,
        }
        .tick(vfs, &mut stop_radio);
    }

    /// Stop and unload the music track (the Music Player window closed,
    /// or the radio is taking over the audio backend).
    pub(crate) fn stop_music(&mut self) {
        oasis_audio::MusicController {
            manager: &mut self.media,
            track: &mut self.media_track,
            audio: &mut self.audio,
            stream_shared: false,
        }
        .shutdown();
    }
}
