//! Internet radio subsystem.
//!
//! Provides a `RadioManager` state machine that orchestrates streaming
//! playback from swappable `RadioSource` implementations (Icecast, VFS).
//! Uses VFS-based IPC for terminal command integration.

pub mod buffer;
pub mod icy;
pub mod source;
pub mod station;

use oasis_types::backend::{AudioBackend, AudioTrackId};
use oasis_types::error::{OasisError, Result};
use oasis_vfs::Vfs;

pub use source::{AudioChunk, IcecastSource, RadioSource, SourceState, VfsSource};
pub use station::{Station, StationRegistry};

/// VFS path where the radio manager publishes its status.
pub const RADIO_STATUS_PATH: &str = "/var/radio/status";
/// VFS path where terminal commands write radio requests.
pub const RADIO_REQUEST_PATH: &str = "/var/radio/request";

/// Buffering threshold: start playback after accumulating this many bytes.
const BUFFER_THRESHOLD: usize = 32 * 1024;

/// Radio playback state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioState {
    /// No station selected.
    Stopped,
    /// Source created, waiting for first data.
    Connecting,
    /// Accumulating audio data before playback starts.
    Buffering,
    /// Actively streaming and playing.
    Playing,
    /// An error occurred.
    Error,
}

impl std::fmt::Display for RadioState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => write!(f, "stopped"),
            Self::Connecting => write!(f, "connecting"),
            Self::Buffering => write!(f, "buffering"),
            Self::Playing => write!(f, "playing"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Manages internet radio streaming playback.
pub struct RadioManager {
    state: RadioState,
    /// Current station name.
    station_name: String,
    /// Current stream metadata (artist/title from ICY).
    now_playing: String,
    /// Streaming audio track handle.
    stream_track: Option<AudioTrackId>,
    /// Audio buffer for accumulating stream data.
    audio_buf: buffer::StreamBuffer,
    /// Volume (0-100).
    volume: u8,
    /// Bitrate of current station (for buffer duration estimates).
    bitrate_kbps: u32,
    /// Error message (when state is Error).
    error_msg: String,
    /// Station registry.
    pub registry: StationRegistry,
    /// Genre filter (empty = show all).
    genre_filter: String,
}

impl RadioManager {
    /// Create a new radio manager with default stations.
    pub fn new() -> Self {
        Self {
            state: RadioState::Stopped,
            station_name: String::new(),
            now_playing: String::new(),
            stream_track: None,
            audio_buf: buffer::StreamBuffer::new(),
            volume: 80,
            bitrate_kbps: 128,
            error_msg: String::new(),
            registry: StationRegistry::defaults(),
            genre_filter: String::new(),
        }
    }

    /// Current state.
    pub fn state(&self) -> RadioState {
        self.state
    }

    /// Current volume.
    pub fn volume(&self) -> u8 {
        self.volume
    }

    /// Current station name.
    pub fn station_name(&self) -> &str {
        &self.station_name
    }

    /// Current "now playing" metadata.
    pub fn now_playing(&self) -> &str {
        &self.now_playing
    }

    /// Current genre filter.
    pub fn genre_filter(&self) -> &str {
        &self.genre_filter
    }

    /// Set genre filter.
    pub fn set_genre_filter(&mut self, genre: &str) {
        self.genre_filter = genre.to_string();
    }

    /// Start tuning to a station via a pre-created source.
    pub fn tune(
        &mut self,
        station_name: &str,
        bitrate: u32,
        backend: &mut dyn AudioBackend,
    ) -> Result<()> {
        // Stop current playback.
        self.stop(backend)?;

        self.station_name = station_name.to_string();
        self.bitrate_kbps = if bitrate > 0 { bitrate } else { 128 };
        self.now_playing.clear();
        self.error_msg.clear();
        self.audio_buf.clear();
        self.state = RadioState::Connecting;

        Ok(())
    }

    /// Transition from Connecting to Buffering (called after source is ready).
    pub fn begin_buffering(&mut self) {
        if self.state == RadioState::Connecting {
            self.state = RadioState::Buffering;
        }
    }

    /// Feed audio data from the source into the buffer.
    ///
    /// Returns `true` if playback should start (buffer threshold reached).
    pub fn feed_audio(
        &mut self,
        chunk: &AudioChunk,
        backend: &mut dyn AudioBackend,
    ) -> Result<bool> {
        if let Some(ref meta) = chunk.metadata
            && !meta.title.is_empty()
        {
            self.now_playing = meta.title.clone();
        }

        self.audio_buf.write(&chunk.data);

        // Feed data to the audio backend if we have a streaming track.
        if let Some(track) = self.stream_track {
            backend.feed_data(track, &chunk.data)?;
        }

        // Check if we should start playback.
        if self.state == RadioState::Buffering && self.audio_buf.available() >= BUFFER_THRESHOLD {
            return Ok(true);
        }

        Ok(false)
    }

    /// Start playback after buffering is complete.
    pub fn start_playback(&mut self, backend: &mut dyn AudioBackend) -> Result<()> {
        let track = backend.load_streaming()?;
        self.stream_track = Some(track);
        backend.play(track)?;
        backend.set_volume(self.volume)?;
        self.state = RadioState::Playing;
        Ok(())
    }

    /// Stop playback.
    pub fn stop(&mut self, backend: &mut dyn AudioBackend) -> Result<()> {
        if let Some(track) = self.stream_track.take() {
            let _ = backend.stop();
            let _ = backend.unload_track(track);
        }
        self.audio_buf.clear();
        self.state = RadioState::Stopped;
        self.station_name.clear();
        self.now_playing.clear();
        Ok(())
    }

    /// Set the volume (0-100).
    pub fn set_volume(&mut self, vol: u8, backend: &mut dyn AudioBackend) -> Result<()> {
        self.volume = vol.min(100);
        backend.set_volume(self.volume)
    }

    /// Set error state with a message.
    pub fn set_error(&mut self, msg: &str) {
        self.state = RadioState::Error;
        self.error_msg = msg.to_string();
    }

    /// Tick the radio manager (called each frame from the main loop).
    ///
    /// Drives the source and state machine. The source is owned by the
    /// caller to avoid lifetime issues with `NetworkBackend`.
    pub fn tick(
        &mut self,
        source: &mut Option<Box<dyn RadioSource>>,
        backend: &mut dyn AudioBackend,
    ) -> Result<()> {
        let src = match source.as_mut() {
            Some(s) => s,
            None => return Ok(()),
        };

        match self.state {
            RadioState::Connecting => {
                // Poll source to drive connection.
                match src.poll() {
                    Ok(Some(chunk)) => {
                        self.state = RadioState::Buffering;
                        let _ = self.feed_audio(&chunk, backend);
                    },
                    Ok(None) => {
                        if src.state() == SourceState::Active {
                            self.state = RadioState::Buffering;
                        }
                    },
                    Err(e) => {
                        self.set_error(&format!("{e}"));
                        *source = None;
                    },
                }
            },
            RadioState::Buffering | RadioState::Playing => match src.poll() {
                Ok(Some(chunk)) => {
                    let should_start = self.feed_audio(&chunk, backend)?;
                    if should_start && self.state == RadioState::Buffering {
                        self.start_playback(backend)?;
                    }
                },
                Ok(None) => {
                    if src.state() == SourceState::Ended {
                        self.stop(backend)?;
                        *source = None;
                    }
                },
                Err(e) => {
                    self.set_error(&format!("{e}"));
                    *source = None;
                },
            },
            RadioState::Stopped | RadioState::Error => {
                // Clean up the source if radio was stopped or errored.
                if let Some(mut src) = source.take() {
                    src.disconnect();
                }
            },
        }

        Ok(())
    }

    /// Process a request string from the terminal (via VFS IPC).
    pub fn process_request(
        &mut self,
        request: &str,
        backend: &mut dyn AudioBackend,
    ) -> Result<String> {
        let parts: Vec<&str> = request.trim().splitn(3, ' ').collect();
        let cmd = parts[0];

        match cmd {
            "stop" => {
                self.stop(backend)?;
                Ok("radio stopped".to_string())
            },
            "vol" => {
                let vol_str = parts.get(1).unwrap_or(&"");
                let vol: u8 = vol_str
                    .parse()
                    .map_err(|_| OasisError::Command(format!("invalid volume: {vol_str}")))?;
                self.set_volume(vol, backend)?;
                Ok(format!("volume: {}%", self.volume))
            },
            "fav" => {
                let idx_str = parts.get(1).unwrap_or(&"");
                let idx: usize = idx_str
                    .parse()
                    .map_err(|_| OasisError::Command(format!("invalid index: {idx_str}")))?;
                if self.registry.toggle_favorite(idx) {
                    let fav = self.registry.stations[idx].favorite;
                    let name = &self.registry.stations[idx].name;
                    let star = if fav { "added" } else { "removed" };
                    Ok(format!("{name}: favorite {star}"))
                } else {
                    Err(OasisError::Command(format!("station {idx} not found")))
                }
            },
            "genre" => {
                let genre = parts.get(1).unwrap_or(&"");
                if genre.is_empty() {
                    let genres = self.registry.genres();
                    Ok(format!("Genres: {}", genres.join(", ")))
                } else {
                    self.set_genre_filter(genre);
                    Ok(format!("genre filter: {genre}"))
                }
            },
            // "tune" is handled by the main loop (needs NetworkBackend).
            _ => Err(OasisError::Command(format!("unknown radio command: {cmd}"))),
        }
    }

    /// Format the current status as a human-readable string.
    pub fn format_status(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("State: {}", self.state));
        lines.push(format!("Volume: {}%", self.volume));

        if !self.station_name.is_empty() {
            lines.push(format!("Station: {}", self.station_name));
        } else {
            lines.push("Station: --".to_string());
        }

        if !self.now_playing.is_empty() {
            lines.push(format!("Now Playing: {}", self.now_playing));
        } else {
            lines.push("Now Playing: --".to_string());
        }

        if self.state == RadioState::Buffering || self.state == RadioState::Playing {
            let buf_ms = self.audio_buf.buffered_ms(self.bitrate_kbps);
            let buf_kb = self.audio_buf.available() / 1024;
            lines.push(format!("Buffer: {buf_kb} KB ({buf_ms} ms)"));
        }

        if self.state == RadioState::Error && !self.error_msg.is_empty() {
            lines.push(format!("Error: {}", self.error_msg));
        }

        if !self.genre_filter.is_empty() {
            lines.push(format!("Genre Filter: {}", self.genre_filter));
        }

        lines.push(format!("Stations: {}", self.registry.stations.len()));

        lines.join("\n")
    }

    /// Publish the current status to the VFS.
    pub fn publish_status(&self, vfs: &mut dyn Vfs) -> Result<()> {
        let status = self.format_status();
        vfs.write(RADIO_STATUS_PATH, status.as_bytes())?;
        Ok(())
    }

    /// Load station registry from a VFS path.
    pub fn load_stations(&mut self, vfs: &dyn Vfs, path: &str) -> Result<()> {
        if !vfs.exists(path) {
            return Ok(());
        }
        let data = vfs.read(path)?;
        let text = String::from_utf8_lossy(&data);
        match StationRegistry::from_toml(&text) {
            Ok(reg) => {
                self.registry = reg;
                Ok(())
            },
            Err(e) => Err(OasisError::Backend(e)),
        }
    }

    /// Return filtered stations (by genre_filter if set, else all).
    pub fn filtered_stations(&self) -> Vec<(usize, &Station)> {
        self.registry
            .stations
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                self.genre_filter.is_empty() || s.genre.eq_ignore_ascii_case(&self.genre_filter)
            })
            .collect()
    }
}

impl Default for RadioManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::AudioTrackId;
    use oasis_types::error::Result;
    use oasis_vfs::MemoryVfs;

    /// Stub audio backend for testing radio.
    struct StubAudioBackend {
        volume: u8,
        playing: bool,
        loaded_count: u64,
    }

    impl StubAudioBackend {
        fn new() -> Self {
            Self {
                volume: 80,
                playing: false,
                loaded_count: 0,
            }
        }
    }

    impl AudioBackend for StubAudioBackend {
        fn init(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_track(&mut self, _data: &[u8]) -> Result<AudioTrackId> {
            let id = self.loaded_count;
            self.loaded_count += 1;
            Ok(AudioTrackId(id))
        }
        fn play(&mut self, _track: AudioTrackId) -> Result<()> {
            self.playing = true;
            Ok(())
        }
        fn pause(&mut self) -> Result<()> {
            self.playing = false;
            Ok(())
        }
        fn resume(&mut self) -> Result<()> {
            self.playing = true;
            Ok(())
        }
        fn stop(&mut self) -> Result<()> {
            self.playing = false;
            Ok(())
        }
        fn set_volume(&mut self, vol: u8) -> Result<()> {
            self.volume = vol;
            Ok(())
        }
        fn get_volume(&self) -> u8 {
            self.volume
        }
        fn is_playing(&self) -> bool {
            self.playing
        }
        fn position_ms(&self) -> u64 {
            0
        }
        fn duration_ms(&self) -> u64 {
            0
        }
        fn unload_track(&mut self, _track: AudioTrackId) -> Result<()> {
            Ok(())
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_streaming(&mut self) -> Result<AudioTrackId> {
            let id = self.loaded_count;
            self.loaded_count += 1;
            Ok(AudioTrackId(id))
        }
        fn feed_data(&mut self, _track: AudioTrackId, _data: &[u8]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn new_manager_is_stopped() {
        let mgr = RadioManager::new();
        assert_eq!(mgr.state(), RadioState::Stopped);
        assert_eq!(mgr.volume(), 80);
        assert!(mgr.station_name().is_empty());
        assert!(mgr.now_playing().is_empty());
    }

    #[test]
    fn tune_transitions_to_connecting() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        mgr.tune("Test FM", 128, &mut backend).unwrap();
        assert_eq!(mgr.state(), RadioState::Connecting);
        assert_eq!(mgr.station_name(), "Test FM");
    }

    #[test]
    fn stop_resets_state() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        mgr.tune("Test FM", 128, &mut backend).unwrap();
        mgr.stop(&mut backend).unwrap();
        assert_eq!(mgr.state(), RadioState::Stopped);
        assert!(mgr.station_name().is_empty());
    }

    #[test]
    fn volume_control() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        mgr.set_volume(50, &mut backend).unwrap();
        assert_eq!(mgr.volume(), 50);
        assert_eq!(backend.volume, 50);

        mgr.set_volume(200, &mut backend).unwrap();
        assert_eq!(mgr.volume(), 100);
    }

    #[test]
    fn process_request_stop() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        mgr.tune("Test FM", 128, &mut backend).unwrap();
        let resp = mgr.process_request("stop", &mut backend).unwrap();
        assert!(resp.contains("stopped"));
        assert_eq!(mgr.state(), RadioState::Stopped);
    }

    #[test]
    fn process_request_vol() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        let resp = mgr.process_request("vol 42", &mut backend).unwrap();
        assert!(resp.contains("42%"));
        assert_eq!(mgr.volume(), 42);
    }

    #[test]
    fn process_request_genre() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        let resp = mgr.process_request("genre ambient", &mut backend).unwrap();
        assert!(resp.contains("ambient"));
        assert_eq!(mgr.genre_filter(), "ambient");
    }

    #[test]
    fn process_request_genre_list() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        let resp = mgr.process_request("genre", &mut backend).unwrap();
        assert!(resp.contains("Genres:"));
        assert!(resp.contains("ambient"));
    }

    #[test]
    fn process_request_fav() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        let was_fav = mgr.registry.stations[0].favorite;
        let resp = mgr.process_request("fav 0", &mut backend).unwrap();
        assert!(resp.contains("favorite"));
        assert_ne!(mgr.registry.stations[0].favorite, was_fav);
    }

    #[test]
    fn process_request_unknown() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        assert!(mgr.process_request("badcmd", &mut backend).is_err());
    }

    #[test]
    fn format_status_output() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        mgr.tune("Test FM", 128, &mut backend).unwrap();
        let status = mgr.format_status();
        assert!(status.contains("connecting"));
        assert!(status.contains("Test FM"));
        assert!(status.contains("Volume: 80%"));
    }

    #[test]
    fn publish_status_to_vfs() {
        let mgr = RadioManager::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/radio").unwrap();

        mgr.publish_status(&mut vfs).unwrap();
        let data = vfs.read(RADIO_STATUS_PATH).unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(text.contains("stopped"));
    }

    #[test]
    fn load_stations_from_vfs() {
        let mut mgr = RadioManager::new();
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/etc/radio").unwrap();
        let toml_data = StationRegistry::defaults().to_toml().unwrap();
        vfs.write("/etc/radio/stations.toml", toml_data.as_bytes())
            .unwrap();

        mgr.load_stations(&vfs, "/etc/radio/stations.toml").unwrap();
        assert!(!mgr.registry.stations.is_empty());
    }

    #[test]
    fn load_stations_missing_file() {
        let mut mgr = RadioManager::new();
        let vfs = MemoryVfs::new();
        // Should not error if file doesn't exist.
        mgr.load_stations(&vfs, "/etc/radio/stations.toml").unwrap();
    }

    #[test]
    fn filtered_stations_no_filter() {
        let mgr = RadioManager::new();
        let filtered = mgr.filtered_stations();
        assert_eq!(filtered.len(), mgr.registry.stations.len());
    }

    #[test]
    fn filtered_stations_with_genre() {
        let mut mgr = RadioManager::new();
        mgr.set_genre_filter("ambient");
        let filtered = mgr.filtered_stations();
        assert!(!filtered.is_empty());
        for (_, s) in &filtered {
            assert_eq!(s.genre, "ambient");
        }
    }

    #[test]
    fn tick_with_vfs_source() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();

        mgr.tune("Test", 128, &mut backend).unwrap();
        assert_eq!(mgr.state(), RadioState::Connecting);

        // Create a VFS source with enough data to trigger playback and survive
        // 20 ticks without exhausting (20 * 4096 = 80KB).
        let data = vec![0xAAu8; 128 * 1024]; // 128 KB >> 32 KB threshold.
        let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

        // Tick should drive through Connecting -> Buffering -> Playing.
        for _ in 0..20 {
            mgr.tick(&mut source, &mut backend).unwrap();
        }

        assert_eq!(mgr.state(), RadioState::Playing);
    }

    #[test]
    fn tick_with_no_source() {
        let mut mgr = RadioManager::new();
        let mut backend = StubAudioBackend::new();
        let mut source: Option<Box<dyn RadioSource>> = None;
        // Should be a no-op.
        mgr.tick(&mut source, &mut backend).unwrap();
        assert_eq!(mgr.state(), RadioState::Stopped);
    }

    #[test]
    fn set_error_updates_state() {
        let mut mgr = RadioManager::new();
        mgr.set_error("connection failed");
        assert_eq!(mgr.state(), RadioState::Error);
        let status = mgr.format_status();
        assert!(status.contains("connection failed"));
    }

    #[test]
    fn state_display() {
        assert_eq!(RadioState::Stopped.to_string(), "stopped");
        assert_eq!(RadioState::Connecting.to_string(), "connecting");
        assert_eq!(RadioState::Buffering.to_string(), "buffering");
        assert_eq!(RadioState::Playing.to_string(), "playing");
        assert_eq!(RadioState::Error.to_string(), "error");
    }
}
