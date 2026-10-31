//! Internet radio subsystem.
//!
//! Provides a `RadioManager` state machine that orchestrates streaming
//! playback from swappable `RadioSource` implementations (Icecast, VFS).
//! Uses VFS-based IPC for terminal command integration.

pub mod archive;
pub mod buffer;
pub mod connect;
pub mod icy;
pub mod source;
pub mod station;
pub mod threaded;

use oasis_types::backend::{AudioBackend, AudioTrackId};
use oasis_types::error::{OasisError, Result};
use oasis_vfs::Vfs;

pub use archive::{ArchiveCatalog, ArchiveTrack};
pub use connect::{StreamUrl, connect_icecast};
pub use source::{ArchiveSource, AudioChunk, IcecastSource, RadioSource, SourceState, VfsSource};
pub use station::{Station, StationRegistry};
pub use threaded::ThreadedSource;

/// VFS paths of the radio IPC (defined in [`oasis_types::ipc`]).
pub use oasis_types::ipc::{RADIO_REQUEST_PATH, RADIO_STATUS_PATH};
/// Display title of the Internet Radio app, shared between the app
/// itself and every place that needs to identify a radio runner (e.g.
/// the close-handler that stops playback when the window closes). Keep
/// this as the single source of truth so renaming the app in one place
/// doesn't silently break behaviour in another.
pub const RADIO_APP_TITLE: &str = "Internet Radio";

/// Buffering threshold: start playback after accumulating this many bytes.
///
/// Public so platform-specific [`RadioSource`] implementations (e.g. the
/// WASM archive primer, which seeds enough bytes to walk
/// `Connecting → Buffering → Playing` without actually feeding decoded
/// audio) can size their primer chunks against the same threshold and
/// stay in sync if it ever changes.
pub const BUFFER_THRESHOLD: usize = 32 * 1024;

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
    /// Current track finished; ready for next track (archive auto-advance).
    TrackEnded,
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
            Self::TrackEnded => write!(f, "loading"),
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
    /// Source type (e.g. "icecast", "archive").
    source_type_info: String,
    /// Collection identifier (for archive stations).
    collection_info: String,
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
            source_type_info: String::new(),
            collection_info: String::new(),
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

    /// Current error message (when state is Error).
    pub fn error_msg(&self) -> &str {
        &self.error_msg
    }

    /// Clear error and reset to Connecting (e.g. after handling a redirect).
    pub fn resume_from_redirect(&mut self) {
        if self.state == RadioState::Error {
            self.error_msg.clear();
            self.state = RadioState::Connecting;
        }
    }

    /// Set the source type and collection for status display.
    pub fn set_source_info(&mut self, source_type: &str, collection: &str) {
        self.source_type_info = source_type.to_string();
        self.collection_info = collection.to_string();
    }

    /// Current genre filter.
    pub fn genre_filter(&self) -> &str {
        &self.genre_filter
    }

    /// Set genre filter.
    pub fn set_genre_filter(&mut self, genre: &str) {
        self.genre_filter = genre.to_string();
    }

    /// Returns true if the current track ended and the manager is waiting
    /// for the caller to supply the next track (archive auto-advance).
    pub fn needs_next_track(&self) -> bool {
        self.state == RadioState::TrackEnded
    }

    /// Reset state to Connecting after TrackEnded so the next source can play.
    pub fn continue_playing(&mut self) {
        if self.state == RadioState::TrackEnded {
            self.audio_buf.clear();
            self.state = RadioState::Connecting;
        }
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

        // Drain buffered audio into the backend before starting playback.
        let mut tmp = [0u8; 4096];
        loop {
            let n = self.audio_buf.read(&mut tmp);
            if n == 0 {
                break;
            }
            backend.feed_data(track, &tmp[..n])?;
        }

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
        self.source_type_info.clear();
        self.collection_info.clear();
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
            RadioState::Buffering | RadioState::Playing => {
                // Graduated pull. Two competing constraints:
                //
                // 1. Ingest must NOT be coupled to the tick rate: ticks
                //    come from the render loop, and a collapsed frame rate
                //    (occluded window, heavy scene) would otherwise drop
                //    delivery below the stream bitrate and starve playback
                //    ~25 s later (SDL queue + readahead runway) — heard as
                //    stutters that begin about a minute into a session.
                // 2. Delivery should stay gentle while the queue is
                //    healthy: decoding a whole hysteresis-band refill
                //    (~900 KB of PCM) in one tick costs ~15 ms in release
                //    (measured) — a periodic render hitch.
                //
                // So: while `streaming_queued_ms` reports a healthy queue,
                // cap each tick at a small budget (a refill then spreads
                // over ~10 ticks); once the queue erodes below the danger
                // line (collapsed tick rate, post-hitch recovery) pull up
                // to the boost budget per tick to out-run playout.
                // Backends that can't measure their queue get the gentle
                // budget, which is still ~2× the old one-chunk-per-tick.
                const NORMAL_FEED_PER_TICK: usize = 8 * 1024;
                const BOOST_FEED_PER_TICK: usize = 64 * 1024;
                // Below the SDL backend's ~3.6 s refill floor, so normal
                // hysteresis cycling never triggers the boost.
                const DANGER_QUEUE_MS: u32 = 3000;
                let budget = match self
                    .stream_track
                    .and_then(|t| backend.streaming_queued_ms(t))
                {
                    Some(ms) if ms < DANGER_QUEUE_MS => BOOST_FEED_PER_TICK,
                    Some(_) => NORMAL_FEED_PER_TICK,
                    // Unknown queue depth (still buffering, or a backend
                    // without measurement): gentle.
                    None => NORMAL_FEED_PER_TICK,
                };
                let mut ended = false;
                let mut err: Option<OasisError> = None;
                let mut fed = 0usize;
                while fed < budget {
                    let can_pull = self
                        .stream_track
                        .map(|t| backend.streaming_can_accept(t))
                        .unwrap_or(true);
                    if !can_pull {
                        break;
                    }
                    match src.poll() {
                        Ok(Some(chunk)) => {
                            fed += chunk.data.len();
                            let should_start = self.feed_audio(&chunk, backend)?;
                            if should_start && self.state == RadioState::Buffering {
                                self.start_playback(backend)?;
                            }
                        },
                        Ok(None) => {
                            if src.state() == SourceState::Ended {
                                ended = true;
                            }
                            break;
                        },
                        Err(e) => {
                            err = Some(e);
                            break;
                        },
                    }
                }
                if let Some(e) = err {
                    // Clean up the streaming track before entering error state.
                    if let Some(track) = self.stream_track.take() {
                        let _ = backend.stop();
                        let _ = backend.unload_track(track);
                    }
                    self.audio_buf.clear();
                    self.set_error(&format!("{e}"));
                    *source = None;
                } else if ended {
                    // Tell the backend no more data is coming so it can
                    // drain its decode-lookahead side buffer with a
                    // smaller threshold — otherwise the mid-stream
                    // threshold leaves up to ~1 s of audio orphaned at
                    // the end of each track (especially audible with
                    // archive auto-advance between short OTR / LibriVox
                    // segments).
                    if let Some(track) = self.stream_track.take()
                        && let Err(e) = backend.finalize_streaming(track)
                    {
                        log::warn!("finalize_streaming failed draining tail: {e}");
                    }
                    // Release the stream track handle but do NOT stop the
                    // backend — the SDL audio queue may still have seconds
                    // of decoded PCM waiting to play.  Calling stop() would
                    // discard that audio, clipping the end of the track.
                    // The next track's tune() or start_playback() will
                    // reset the backend when it begins.
                    self.audio_buf.clear();
                    // Signal TrackEnded so caller can auto-advance.
                    self.state = RadioState::TrackEnded;
                    *source = None;
                }
            },
            RadioState::Stopped | RadioState::TrackEnded | RadioState::Error => {
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
                let vol: u8 = vol_str.parse().map_err(|_| {
                    OasisError::Command(format!("invalid volume: {vol_str}").into())
                })?;
                self.set_volume(vol, backend)?;
                Ok(format!("volume: {}%", self.volume))
            },
            "fav" => {
                let idx_str = parts.get(1).unwrap_or(&"");
                let idx: usize = idx_str
                    .parse()
                    .map_err(|_| OasisError::Command(format!("invalid index: {idx_str}").into()))?;
                if self.registry.toggle_favorite(idx) {
                    let fav = self.registry.stations[idx].favorite;
                    let name = &self.registry.stations[idx].name;
                    let star = if fav { "added" } else { "removed" };
                    Ok(format!("{name}: favorite {star}"))
                } else {
                    Err(OasisError::Command(
                        format!("station {idx} not found").into(),
                    ))
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
            _ => Err(OasisError::Command(
                format!("unknown radio command: {cmd}").into(),
            )),
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

        if !self.source_type_info.is_empty() {
            lines.push(format!("Source: {}", self.source_type_info));
        }
        if !self.collection_info.is_empty() {
            lines.push(format!("Collection: {}", self.collection_info));
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
            Err(e) => Err(OasisError::Backend(e.into())),
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
mod tests;
