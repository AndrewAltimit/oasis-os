use super::*;
use oasis_types::backend::AudioTrackId;
use oasis_types::error::Result;
use oasis_vfs::MemoryVfs;

/// Stub audio backend for testing radio.
///
/// Simulates the back-pressure the real SDL3 backend applies: `feed_data`
/// tracks buffered bytes and `streaming_can_accept` reports false once a
/// threshold is crossed, draining a small amount per query to model
/// playback progress. Without this the drain loop in
/// `RadioManager::tick` would empty any finite `VfsSource` in a single
/// tick and skip past the state transitions the tests are checking.
struct StubAudioBackend {
    volume: u8,
    playing: bool,
    loaded_count: u64,
    buffered: std::cell::Cell<usize>,
}

impl StubAudioBackend {
    /// Bytes "buffered" before `streaming_can_accept` returns false.
    /// Sized so a normal tick pulls ~7 × 4 KB = 28 KB before blocking.
    const BACK_PRESSURE_LIMIT: usize = 16 * 1024;
    /// Simulated playback drain per `streaming_can_accept` call. Half the
    /// 4 KB poll chunk, so back-pressured ticks still make steady
    /// forward progress — otherwise tests like
    /// `error_during_playing_cleans_up` spend dozens of ticks stuck just
    /// below the cap.
    const DRAIN_PER_QUERY: usize = 2048;

    fn new() -> Self {
        Self {
            volume: 80,
            playing: false,
            loaded_count: 0,
            buffered: std::cell::Cell::new(0),
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
        self.buffered.set(0);
        Ok(AudioTrackId(id))
    }
    fn feed_data(&mut self, _track: AudioTrackId, data: &[u8]) -> Result<()> {
        self.buffered.set(self.buffered.get() + data.len());
        Ok(())
    }
    fn streaming_can_accept(&self, _track: AudioTrackId) -> bool {
        let before = self.buffered.get();
        self.buffered
            .set(before.saturating_sub(Self::DRAIN_PER_QUERY));
        before < Self::BACK_PRESSURE_LIMIT
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
    let resp = mgr.process_request("genre drama", &mut backend).unwrap();
    assert!(resp.contains("drama"));
    assert_eq!(mgr.genre_filter(), "drama");
}

#[test]
fn process_request_genre_list() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();
    let resp = mgr.process_request("genre", &mut backend).unwrap();
    assert!(resp.contains("Genres:"));
    assert!(resp.contains("drama"));
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
    mgr.set_genre_filter("drama");
    let filtered = mgr.filtered_stations();
    assert!(!filtered.is_empty());
    for (_, s) in &filtered {
        assert_eq!(s.genre, "drama");
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
    assert_eq!(RadioState::TrackEnded.to_string(), "loading");
    assert_eq!(RadioState::Error.to_string(), "error");
}

#[test]
fn track_ended_transition() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test", 128, &mut backend).unwrap();

    // Create a VFS source that will end after all data consumed.
    let data = vec![0xAAu8; 128 * 1024];
    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

    // Tick until playing.
    for _ in 0..20 {
        mgr.tick(&mut source, &mut backend).unwrap();
    }
    assert_eq!(mgr.state(), RadioState::Playing);

    // Tick until source exhausted -> TrackEnded.
    for _ in 0..100 {
        mgr.tick(&mut source, &mut backend).unwrap();
        if mgr.state() == RadioState::TrackEnded {
            break;
        }
    }
    assert_eq!(mgr.state(), RadioState::TrackEnded);
    assert!(mgr.needs_next_track());
    assert!(source.is_none());
}

#[test]
fn continue_playing_resets_state() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test", 128, &mut backend).unwrap();
    let data = vec![0xAAu8; 128 * 1024];
    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

    // Drive to TrackEnded.
    for _ in 0..200 {
        mgr.tick(&mut source, &mut backend).unwrap();
        if mgr.needs_next_track() {
            break;
        }
    }
    assert!(mgr.needs_next_track());

    mgr.continue_playing();
    assert_eq!(mgr.state(), RadioState::Connecting);
    assert!(!mgr.needs_next_track());
}

#[test]
fn needs_next_track_false_when_not_track_ended() {
    let mgr = RadioManager::new();
    assert!(!mgr.needs_next_track());
}

// ---------------------------------------------------------------
// Integration: full pipeline end-to-end tests
// ---------------------------------------------------------------

/// Helper: drive RadioManager through Connecting → Buffering → Playing.
fn drive_to_playing(
    mgr: &mut RadioManager,
    source: &mut Option<Box<dyn RadioSource>>,
    backend: &mut StubAudioBackend,
) -> bool {
    for _ in 0..200 {
        mgr.tick(source, backend).unwrap();
        if mgr.state() == RadioState::Playing {
            return true;
        }
    }
    false
}

/// Helper: drive RadioManager until TrackEnded or max iterations.
fn drive_to_track_ended(
    mgr: &mut RadioManager,
    source: &mut Option<Box<dyn RadioSource>>,
    backend: &mut StubAudioBackend,
) -> bool {
    for _ in 0..500 {
        mgr.tick(source, backend).unwrap();
        if mgr.needs_next_track() {
            return true;
        }
    }
    false
}

#[test]
fn full_pipeline_connecting_to_playing() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();
    assert_eq!(mgr.state(), RadioState::Connecting);

    // 128KB data → well above 32KB threshold.
    let data = vec![0xAAu8; 128 * 1024];
    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

    assert!(drive_to_playing(&mut mgr, &mut source, &mut backend));
    assert!(backend.playing);
}

#[test]
fn now_playing_cannot_forge_status_lines() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();
    let chunk = AudioChunk {
        data: vec![0; 16],
        metadata: Some(icy::StreamMetadata {
            title: "Song\nState: Error\nError: pwned".into(),
        }),
    };
    mgr.feed_audio(&chunk, &mut backend).unwrap();
    assert_eq!(mgr.now_playing(), "SongState: ErrorError: pwned");
    let status = mgr.format_status();
    assert_eq!(
        status.lines().filter(|l| l.starts_with("State:")).count(),
        1
    );
    assert!(!status.lines().any(|l| l.starts_with("Error:")), "{status}");
}

#[test]
fn track_shorter_than_the_buffer_threshold_still_plays() {
    // Regression: a source that ended before BUFFER_THRESHOLD bytes had
    // arrived went straight to TrackEnded and its audio was discarded.
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();
    mgr.tune("Short", 128, &mut backend).unwrap();
    let data = vec![0x55u8; BUFFER_THRESHOLD / 4];
    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));
    assert!(drive_to_track_ended(&mut mgr, &mut source, &mut backend));
    assert!(backend.playing, "the short track must be played");
    assert_eq!(backend.loaded_count, 1, "one streaming track was loaded");
}

#[test]
fn full_pipeline_playing_to_track_ended_to_next() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();

    let data = vec![0xAAu8; 128 * 1024];
    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

    // Drive to playing.
    assert!(drive_to_playing(&mut mgr, &mut source, &mut backend));

    // Drive to track ended.
    assert!(drive_to_track_ended(&mut mgr, &mut source, &mut backend));
    assert!(mgr.needs_next_track());
    assert!(source.is_none(), "source should be cleared on TrackEnded");

    // Simulate auto-advance: supply new source and continue.
    mgr.continue_playing();
    assert_eq!(mgr.state(), RadioState::Connecting);

    let data2 = vec![0xBBu8; 128 * 1024];
    source = Some(Box::new(VfsSource::new(data2)));

    // Should play the second track.
    assert!(drive_to_playing(&mut mgr, &mut source, &mut backend));
}

#[test]
fn full_pipeline_multiple_track_advances() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();
    mgr.tune("Test FM", 128, &mut backend).unwrap();

    // Simulate 5 consecutive track plays + auto-advances.
    for i in 0..5 {
        let data = vec![(i as u8).wrapping_mul(0x11); 64 * 1024];
        let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

        assert!(
            drive_to_playing(&mut mgr, &mut source, &mut backend),
            "track {i}: should reach Playing"
        );
        assert!(
            drive_to_track_ended(&mut mgr, &mut source, &mut backend),
            "track {i}: should reach TrackEnded"
        );

        if i < 4 {
            mgr.continue_playing();
        }
    }
}

#[test]
fn error_during_connecting_sets_error_state() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Broken Station", 128, &mut backend).unwrap();

    // Create a source that immediately errors.
    struct ErrorSource;
    impl RadioSource for ErrorSource {
        fn poll(&mut self) -> oasis_types::error::Result<Option<AudioChunk>> {
            Err(oasis_types::error::OasisError::Backend(
                "connection refused".into(),
            ))
        }
        fn disconnect(&mut self) {}
        fn state(&self) -> SourceState {
            SourceState::Error
        }
        fn source_type(&self) -> &str {
            "error-test"
        }
    }

    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(ErrorSource));
    mgr.tick(&mut source, &mut backend).unwrap();

    assert_eq!(mgr.state(), RadioState::Error);
    assert!(
        mgr.error_msg().contains("connection refused"),
        "error_msg: {}",
        mgr.error_msg()
    );
    assert!(source.is_none(), "source should be cleared on error");
}

#[test]
fn error_during_playing_cleans_up() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();

    // Source that delivers data then errors after 40KB.
    struct ErrorAfterSource {
        delivered: usize,
    }
    impl RadioSource for ErrorAfterSource {
        fn poll(&mut self) -> oasis_types::error::Result<Option<AudioChunk>> {
            if self.delivered >= 40 * 1024 {
                return Err(oasis_types::error::OasisError::Backend(
                    "stream interrupted".into(),
                ));
            }
            let chunk = vec![0xAA; 4096];
            self.delivered += chunk.len();
            Ok(Some(AudioChunk {
                data: chunk,
                metadata: None,
            }))
        }
        fn disconnect(&mut self) {}
        fn state(&self) -> SourceState {
            SourceState::Active
        }
        fn source_type(&self) -> &str {
            "error-after-test"
        }
    }

    let mut source: Option<Box<dyn RadioSource>> =
        Some(Box::new(ErrorAfterSource { delivered: 0 }));

    // Drive to playing (32KB threshold).
    for _ in 0..20 {
        let _ = mgr.tick(&mut source, &mut backend);
        if mgr.state() == RadioState::Playing {
            break;
        }
    }
    assert_eq!(mgr.state(), RadioState::Playing);

    // Continue ticking — error occurs after 40KB.
    for _ in 0..20 {
        let _ = mgr.tick(&mut source, &mut backend);
        if mgr.state() == RadioState::Error {
            break;
        }
    }
    assert_eq!(mgr.state(), RadioState::Error);
    assert!(mgr.error_msg().contains("stream interrupted"));
    assert!(source.is_none());
}

#[test]
fn tune_stop_tune_cycle() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    for i in 0..3 {
        mgr.tune(&format!("Station {i}"), 128, &mut backend)
            .unwrap();
        assert_eq!(mgr.state(), RadioState::Connecting);
        assert_eq!(mgr.station_name(), format!("Station {i}"));

        let data = vec![0xAA; 64 * 1024];
        let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));
        assert!(drive_to_playing(&mut mgr, &mut source, &mut backend));

        mgr.stop(&mut backend).unwrap();
        assert_eq!(mgr.state(), RadioState::Stopped);
        assert!(mgr.station_name().is_empty());
    }
}

#[test]
fn rapid_tune_without_waiting() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    // Tune 5 times rapidly without letting sources complete.
    for i in 0..5 {
        mgr.tune(&format!("Station {i}"), 128, &mut backend)
            .unwrap();
    }
    // Should be in Connecting for the last station.
    assert_eq!(mgr.state(), RadioState::Connecting);
    assert_eq!(mgr.station_name(), "Station 4");
}

#[test]
fn feed_audio_updates_now_playing() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();
    mgr.begin_buffering();

    let chunk = AudioChunk {
        data: vec![0; 100],
        metadata: Some(super::icy::StreamMetadata {
            title: "Artist - Song Title".into(),
        }),
    };
    mgr.feed_audio(&chunk, &mut backend).unwrap();
    assert_eq!(mgr.now_playing(), "Artist - Song Title");
}

#[test]
fn feed_audio_ignores_empty_metadata() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();
    mgr.begin_buffering();

    // Set initial metadata.
    let chunk1 = AudioChunk {
        data: vec![0; 100],
        metadata: Some(super::icy::StreamMetadata {
            title: "First Song".into(),
        }),
    };
    mgr.feed_audio(&chunk1, &mut backend).unwrap();
    assert_eq!(mgr.now_playing(), "First Song");

    // Empty title should not overwrite.
    let chunk2 = AudioChunk {
        data: vec![0; 100],
        metadata: Some(super::icy::StreamMetadata {
            title: String::new(),
        }),
    };
    mgr.feed_audio(&chunk2, &mut backend).unwrap();
    assert_eq!(mgr.now_playing(), "First Song");
}

#[test]
fn feed_audio_below_threshold_does_not_start_playback() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();
    mgr.begin_buffering();

    // Feed less than 32KB.
    let chunk = AudioChunk {
        data: vec![0; 1024],
        metadata: None,
    };
    let should_start = mgr.feed_audio(&chunk, &mut backend).unwrap();
    assert!(!should_start, "should not start with only 1KB buffered");
    assert_eq!(mgr.state(), RadioState::Buffering);
}

#[test]
fn feed_audio_above_threshold_signals_start() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();
    mgr.begin_buffering();

    // Feed 32KB+ to cross threshold.
    let chunk = AudioChunk {
        data: vec![0; 33 * 1024],
        metadata: None,
    };
    let should_start = mgr.feed_audio(&chunk, &mut backend).unwrap();
    assert!(should_start, "should signal start after 33KB buffered");
}

#[test]
fn source_info_set_and_cleared() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test Station", 128, &mut backend).unwrap();
    mgr.set_source_info("archive", "oldtimeradio");

    let status = mgr.format_status();
    assert!(status.contains("Source: archive"));
    assert!(status.contains("Collection: oldtimeradio"));

    mgr.stop(&mut backend).unwrap();
    let status = mgr.format_status();
    assert!(!status.contains("Source:"));
    assert!(!status.contains("Collection:"));
}

#[test]
fn resume_from_redirect_resets_state() {
    let mut mgr = RadioManager::new();
    mgr.set_error("redirect:https://cdn.example.com/file.mp3");
    assert_eq!(mgr.state(), RadioState::Error);

    mgr.resume_from_redirect();
    assert_eq!(mgr.state(), RadioState::Connecting);
    assert!(mgr.error_msg().is_empty());
}

#[test]
fn resume_from_redirect_noop_when_not_error() {
    let mut mgr = RadioManager::new();
    assert_eq!(mgr.state(), RadioState::Stopped);

    mgr.resume_from_redirect();
    assert_eq!(mgr.state(), RadioState::Stopped);
}

#[test]
fn format_status_shows_error() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();
    mgr.set_error("HTTP 404");

    let status = mgr.format_status();
    assert!(status.contains("error"), "state should show error");
    assert!(status.contains("HTTP 404"), "should include error message");
}

#[test]
fn format_status_shows_buffer_during_buffering() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.tune("Test FM", 128, &mut backend).unwrap();
    mgr.begin_buffering();

    let chunk = AudioChunk {
        data: vec![0; 4096],
        metadata: None,
    };
    mgr.feed_audio(&chunk, &mut backend).unwrap();

    let status = mgr.format_status();
    assert!(status.contains("Buffer:"), "should show buffer info");
    assert!(status.contains("KB"), "should show buffer size in KB");
}

#[test]
fn tick_cleans_up_source_when_stopped() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    let data = vec![0xAAu8; 100];
    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

    // Manager is stopped — tick should clean up the source.
    mgr.tick(&mut source, &mut backend).unwrap();
    assert!(source.is_none(), "source should be cleaned up when stopped");
}

#[test]
fn tick_cleans_up_source_when_errored() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();

    mgr.set_error("some error");
    let data = vec![0xAAu8; 100];
    let mut source: Option<Box<dyn RadioSource>> = Some(Box::new(VfsSource::new(data)));

    mgr.tick(&mut source, &mut backend).unwrap();
    assert!(source.is_none(), "source should be cleaned up on error");
}

// ---------------------------------------------------------------
// VFS IPC integration tests
// ---------------------------------------------------------------

#[test]
fn publish_status_reflects_playing_state() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/var").unwrap();
    vfs.mkdir("/var/radio").unwrap();

    mgr.tune("My Station", 128, &mut backend).unwrap();
    mgr.set_source_info("archive", "oldtimeradio");

    // Feed enough data to trigger playback.
    mgr.begin_buffering();
    let chunk = AudioChunk {
        data: vec![0; 33 * 1024],
        metadata: Some(super::icy::StreamMetadata {
            title: "Now Playing Track".into(),
        }),
    };
    mgr.feed_audio(&chunk, &mut backend).unwrap();
    mgr.start_playback(&mut backend).unwrap();

    mgr.publish_status(&mut vfs).unwrap();
    let data = vfs.read(RADIO_STATUS_PATH).unwrap();
    let status = String::from_utf8_lossy(&data);

    assert!(status.contains("playing"), "status: {status}");
    assert!(status.contains("My Station"), "status: {status}");
    assert!(status.contains("Now Playing Track"), "status: {status}");
    assert!(status.contains("Source: archive"), "status: {status}");
    assert!(
        status.contains("Collection: oldtimeradio"),
        "status: {status}"
    );
}

#[test]
fn process_request_vol_invalid() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();
    assert!(mgr.process_request("vol abc", &mut backend).is_err());
}

#[test]
fn process_request_fav_out_of_bounds() {
    let mut mgr = RadioManager::new();
    let mut backend = StubAudioBackend::new();
    assert!(mgr.process_request("fav 999", &mut backend).is_err());
}

#[test]
fn default_stations_are_present() {
    let mgr = RadioManager::new();
    assert!(mgr.registry.stations.len() >= 4);

    let names: Vec<&str> = mgr
        .registry
        .stations
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(names.contains(&"Old Time Radio"));
    assert!(names.contains(&"LibriVox Audiobooks"));
    assert!(names.contains(&"Netlabel Music"));
    assert!(names.contains(&"78rpm Records"));
}
