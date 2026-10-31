use super::*;
use oasis_types::ipc::RepeatMode;
use oasis_vfs::MemoryVfs;

fn play(path: &str) -> MediaRequest {
    MediaRequest::PlayFile(path.to_string())
}

/// A manager with three tracks queued (played once each, then stopped).
fn setup() -> AudioManager {
    let mut mgr = AudioManager::new();
    for p in ["/music/a.mp3", "/music/b.mp3", "/music/c.mp3"] {
        mgr.apply(&play(p));
    }
    mgr.apply(&MediaRequest::Stop);
    mgr.playlist.set_current(0);
    mgr
}

#[test]
fn new_is_stopped_and_empty() {
    let mgr = AudioManager::new();
    assert_eq!(mgr.state(), PlaybackState::Stopped);
    assert!(mgr.playlist.is_empty());
    assert_eq!(mgr.current_path(), None);
}

#[test]
fn play_file_adds_once_and_plays() {
    let mut mgr = AudioManager::new();
    assert_eq!(
        mgr.apply(&play("/music/a.mp3")),
        MediaAction::PlayPath("/music/a.mp3".into())
    );
    assert_eq!(mgr.state(), PlaybackState::Playing);
    mgr.apply(&play("/music/b.mp3"));
    mgr.apply(&play("/music/a.mp3"));
    assert_eq!(
        mgr.playlist.len(),
        2,
        "re-playing a file does not duplicate it"
    );
    assert_eq!(mgr.current_path(), Some("/music/a.mp3"));
}

#[test]
fn play_on_empty_playlist_does_nothing() {
    let mut mgr = AudioManager::new();
    assert_eq!(mgr.apply(&MediaRequest::Play), MediaAction::None);
    assert_eq!(mgr.state(), PlaybackState::Stopped);
}

#[test]
fn play_when_stopped_plays_current() {
    let mut mgr = setup();
    assert_eq!(
        mgr.apply(&MediaRequest::Play),
        MediaAction::PlayPath("/music/a.mp3".into())
    );
    assert_eq!(mgr.state(), PlaybackState::Playing);
    assert_eq!(mgr.apply(&MediaRequest::Play), MediaAction::None);
}

#[test]
fn pause_resume_cycle() {
    let mut mgr = setup();
    assert_eq!(
        mgr.apply(&MediaRequest::Pause),
        MediaAction::None,
        "not playing"
    );
    mgr.apply(&MediaRequest::Play);
    assert_eq!(mgr.apply(&MediaRequest::Pause), MediaAction::Pause);
    assert_eq!(mgr.state(), PlaybackState::Paused);
    assert_eq!(mgr.apply(&MediaRequest::Pause), MediaAction::None);
    assert_eq!(mgr.apply(&MediaRequest::Resume), MediaAction::Resume);
    assert_eq!(mgr.state(), PlaybackState::Playing);
    assert_eq!(mgr.apply(&MediaRequest::Resume), MediaAction::None);
    // `play` while paused resumes.
    mgr.apply(&MediaRequest::Pause);
    assert_eq!(mgr.apply(&MediaRequest::Play), MediaAction::Resume);
}

#[test]
fn stop_always_stops() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Play);
    assert_eq!(mgr.apply(&MediaRequest::Stop), MediaAction::Stop);
    assert_eq!(mgr.state(), PlaybackState::Stopped);
}

#[test]
fn next_and_prev_while_playing() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Play);
    assert_eq!(
        mgr.apply(&MediaRequest::Next),
        MediaAction::PlayPath("/music/b.mp3".into())
    );
    assert_eq!(
        mgr.apply(&MediaRequest::Prev),
        MediaAction::PlayPath("/music/a.mp3".into())
    );
}

/// `next` / `prev` while paused used to start the new track. They now
/// cue it and stay paused; `resume` / `play` then start it.
#[test]
fn next_and_prev_while_paused_cue_and_stay_paused() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Play);
    mgr.apply(&MediaRequest::Pause);
    assert_eq!(
        mgr.apply(&MediaRequest::Next),
        MediaAction::Cue("/music/b.mp3".into())
    );
    assert_eq!(mgr.state(), PlaybackState::Paused);
    assert_eq!(
        mgr.apply(&MediaRequest::Next),
        MediaAction::Cue("/music/c.mp3".into())
    );
    assert_eq!(
        mgr.apply(&MediaRequest::Prev),
        MediaAction::Cue("/music/b.mp3".into())
    );
    assert_eq!(mgr.state(), PlaybackState::Paused);
    // `pause` again is still a no-op; `resume` starts the cued track.
    assert_eq!(mgr.apply(&MediaRequest::Pause), MediaAction::None);
    assert_eq!(
        mgr.apply(&MediaRequest::Resume),
        MediaAction::PlayPath("/music/b.mp3".into())
    );
    assert_eq!(mgr.state(), PlaybackState::Playing);
    // A plain pause / resume afterwards resumes the backend again.
    mgr.apply(&MediaRequest::Pause);
    assert_eq!(mgr.apply(&MediaRequest::Resume), MediaAction::Resume);
    // `play` starts a cued track too.
    mgr.apply(&MediaRequest::Pause);
    mgr.apply(&MediaRequest::Next);
    assert_eq!(
        mgr.apply(&MediaRequest::Play),
        MediaAction::PlayPath("/music/c.mp3".into())
    );
}

#[test]
fn stop_clears_a_cued_track() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Play);
    mgr.apply(&MediaRequest::Pause);
    mgr.apply(&MediaRequest::Next);
    mgr.apply(&MediaRequest::Stop);
    assert_eq!(
        mgr.apply(&MediaRequest::Play),
        MediaAction::PlayPath("/music/b.mp3".into())
    );
    mgr.apply(&MediaRequest::Pause);
    assert_eq!(mgr.apply(&MediaRequest::Resume), MediaAction::Resume);
}

/// A track that ends on its own advances like `next`, honouring the
/// repeat mode.
#[test]
fn track_finished_advances_per_repeat_mode() {
    let mut mgr = setup();
    // Not playing: nothing to advance.
    assert_eq!(mgr.track_finished(), MediaAction::None);
    mgr.apply(&MediaRequest::Play);
    assert_eq!(
        mgr.track_finished(),
        MediaAction::PlayPath("/music/b.mp3".into())
    );
    assert_eq!(
        mgr.track_finished(),
        MediaAction::PlayPath("/music/c.mp3".into())
    );
    // Repeat off: the end of the playlist stops.
    assert_eq!(mgr.track_finished(), MediaAction::Stop);
    assert_eq!(mgr.state(), PlaybackState::Stopped);

    // Repeat all wraps around.
    mgr.apply(&MediaRequest::Repeat(RepeatMode::All));
    mgr.apply(&MediaRequest::Play);
    assert_eq!(
        mgr.track_finished(),
        MediaAction::PlayPath("/music/a.mp3".into())
    );
    // Repeat one replays the same track.
    mgr.apply(&MediaRequest::Repeat(RepeatMode::One));
    assert_eq!(
        mgr.track_finished(),
        MediaAction::PlayPath("/music/a.mp3".into())
    );
    // Paused: the host never reports an end, but be safe.
    mgr.apply(&MediaRequest::Pause);
    assert_eq!(mgr.track_finished(), MediaAction::None);
}

#[test]
fn track_finished_with_shuffle_visits_every_track() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Shuffle);
    mgr.apply(&MediaRequest::Repeat(RepeatMode::All));
    mgr.apply(&MediaRequest::Play);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..3 {
        if let MediaAction::PlayPath(p) = mgr.track_finished() {
            seen.insert(p);
        }
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
}

#[test]
fn next_while_stopped_only_moves_selection() {
    let mut mgr = setup();
    assert_eq!(mgr.apply(&MediaRequest::Next), MediaAction::None);
    assert_eq!(mgr.current_path(), Some("/music/b.mp3"));
    assert_eq!(mgr.state(), PlaybackState::Stopped);
}

#[test]
fn next_past_the_end_stops_unless_repeating() {
    let mut mgr = setup();
    mgr.playlist.set_current(2);
    mgr.apply(&MediaRequest::Play);
    assert_eq!(mgr.apply(&MediaRequest::Next), MediaAction::Stop);
    assert_eq!(mgr.state(), PlaybackState::Stopped);

    mgr.apply(&MediaRequest::Repeat(RepeatMode::All));
    mgr.apply(&MediaRequest::Play);
    assert_eq!(
        mgr.apply(&MediaRequest::Next),
        MediaAction::PlayPath("/music/a.mp3".into())
    );
}

#[test]
fn repeat_one_replays_the_same_track() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Play);
    mgr.apply(&MediaRequest::Repeat(RepeatMode::One));
    assert_eq!(
        mgr.apply(&MediaRequest::Next),
        MediaAction::PlayPath("/music/a.mp3".into())
    );
}

#[test]
fn volume_is_clamped_and_forwarded() {
    let mut mgr = AudioManager::new();
    assert_eq!(
        mgr.apply(&MediaRequest::Volume(42)),
        MediaAction::SetVolume(42)
    );
    assert_eq!(
        mgr.apply(&MediaRequest::Volume(250)),
        MediaAction::SetVolume(100)
    );
}

#[test]
fn shuffle_toggles() {
    let mut mgr = setup();
    assert_eq!(mgr.apply(&MediaRequest::Shuffle), MediaAction::None);
    assert!(mgr.playlist.shuffle());
    mgr.apply(&MediaRequest::Shuffle);
    assert!(!mgr.playlist.shuffle());
}

#[test]
fn shuffle_visits_every_track() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Shuffle);
    mgr.apply(&MediaRequest::Repeat(RepeatMode::All));
    mgr.apply(&MediaRequest::Play);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..3 {
        if let MediaAction::PlayPath(p) = mgr.apply(&MediaRequest::Next) {
            seen.insert(p);
        }
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
}

/// `music play <file>` with shuffle on used to leave the shuffle cursor
/// where it was, so `next` could replay the track just picked (or
/// skip unrelated ones). Shuffle now continues from the picked track.
#[test]
fn shuffle_next_after_play_file_moves_on() {
    let mut mgr = AudioManager::new();
    mgr.apply(&MediaRequest::Shuffle);
    mgr.apply(&MediaRequest::Repeat(RepeatMode::All));
    let paths: Vec<String> = (0..4).map(|i| format!("/music/{i}.mp3")).collect();
    for p in &paths {
        mgr.apply(&MediaRequest::PlayFile(p.clone()));
    }
    for p in &paths {
        mgr.apply(&MediaRequest::PlayFile(p.clone()));
        let MediaAction::PlayPath(next) = mgr.apply(&MediaRequest::Next) else {
            panic!("next plays a track");
        };
        assert_ne!(&next, p, "next after play_file {p} replayed it");
        let MediaAction::PlayPath(back) = mgr.apply(&MediaRequest::Prev) else {
            panic!("prev plays a track");
        };
        assert_eq!(&back, p, "prev returns to the picked track");
    }
}

#[test]
fn format_status_reports_everything() {
    let mut mgr = setup();
    mgr.apply(&MediaRequest::Play);
    let status = mgr.format_status(65, 61_000, 180_000);
    assert!(status.contains("State: playing"), "{status}");
    assert!(status.contains("Volume: 65%"), "{status}");
    assert!(status.contains("Track: a.mp3"), "{status}");
    assert!(status.contains("Path: /music/a.mp3"), "{status}");
    assert!(status.contains("Position: 1:01 / 3:00"), "{status}");
    assert!(status.contains("Repeat: off"), "{status}");
    assert!(status.contains("Shuffle: off"), "{status}");
    assert!(status.contains("Playlist: 3 tracks"), "{status}");

    mgr.apply(&MediaRequest::Stop);
    let status = mgr.format_status(65, 0, 0);
    assert!(status.contains("State: stopped"), "{status}");
    assert!(!status.contains("Position:"), "{status}");
}

#[test]
fn publish_status_writes_only_on_change() {
    let mut mgr = setup();
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/var").unwrap();
    vfs.mkdir("/var/audio").unwrap();

    assert!(mgr.publish_status(&mut vfs, 80, 0, 0).unwrap());
    assert!(!mgr.publish_status(&mut vfs, 80, 0, 0).unwrap());
    mgr.apply(&MediaRequest::Play);
    assert!(mgr.publish_status(&mut vfs, 80, 0, 0).unwrap());
    let text = String::from_utf8(vfs.read(AUDIO_STATUS_PATH).unwrap()).unwrap();
    assert!(text.contains("State: playing"));
}
