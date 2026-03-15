//! Thread-safe PTS-based A/V synchronization clock.
//!
//! Uses audio PTS as the master reference clock. Video frames are displayed
//! only when their presentation timestamp is at or before the current audio
//! position. This is the standard approach used by most media players (audio
//! is the master, video syncs to it).
//!
//! The clock is `Send + Sync` and designed to be shared via `Arc` between
//! the audio decode thread (which calls [`SyncClock::update_audio_pts`]) and
//! the video render thread (which calls [`SyncClock::should_display_frame`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A thread-safe presentation-timestamp clock for A/V synchronization.
///
/// Audio PTS is the reference (master) clock. The video renderer queries
/// [`Self::should_display_frame`](Self::should_display_frame) to decide whether a
/// decoded video frame should be presented.
///
/// Uses `Mutex<i64>` for PTS values instead of `AtomicI64` to support
/// 32-bit targets (e.g. PSP MIPS Allegrex) where 64-bit atomics are
/// unavailable.
pub struct SyncClock {
    /// Audio PTS in microseconds (the reference clock).
    audio_pts_us: Mutex<i64>,
    /// Wall-clock instant when playback started.
    start: Mutex<Option<Instant>>,
    /// Whether playback is paused.
    paused: AtomicBool,
    /// Accumulated pause duration in microseconds.
    pause_offset_us: Mutex<i64>,
    /// Wall-clock instant when the most recent pause began.
    pause_start: Mutex<Option<Instant>>,
}

impl SyncClock {
    /// Create a new sync clock wrapped in an `Arc`.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            audio_pts_us: Mutex::new(0),
            start: Mutex::new(None),
            paused: AtomicBool::new(false),
            pause_offset_us: Mutex::new(0),
            pause_start: Mutex::new(None),
        })
    }

    /// Record the playback start time.
    ///
    /// This must be called once before any PTS queries. Resets the clock
    /// state (audio PTS, pause offset) so it can be reused across seeks.
    pub fn start(&self) {
        *self.audio_pts_us.lock().unwrap_or_else(|e| e.into_inner()) = 0;
        *self
            .pause_offset_us
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = 0;
        self.paused.store(false, Ordering::Release);
        {
            let mut ps = self.pause_start.lock().unwrap_or_else(|e| e.into_inner());
            *ps = None;
        }
        let mut s = self.start.lock().unwrap_or_else(|e| e.into_inner());
        *s = Some(Instant::now());
    }

    /// Update the audio reference PTS (in microseconds).
    ///
    /// Called by the audio decode/output thread each time a chunk of audio
    /// is sent to the sound card. The provided value should be the PTS of
    /// the most recently played audio sample.
    pub fn update_audio_pts(&self, pts_us: i64) {
        *self.audio_pts_us.lock().unwrap_or_else(|e| e.into_inner()) = pts_us;
    }

    /// Return the current playback position in microseconds.
    ///
    /// When audio PTS has been updated at least once (non-zero), it is
    /// returned directly as the authoritative time source.  Before the
    /// first audio update, the wall-clock elapsed time since [`Self::start`]
    /// is used as a fallback so that video-only streams still advance.
    ///
    /// While paused, the value is frozen at the last known position.
    pub fn current_pts_us(&self) -> i64 {
        let audio = *self.audio_pts_us.lock().unwrap_or_else(|e| e.into_inner());
        if audio != 0 {
            return audio;
        }

        // Fallback: wall-clock elapsed time minus accumulated pause time.
        let guard = self.start.lock().unwrap_or_else(|e| e.into_inner());
        let Some(start) = *guard else {
            return 0;
        };
        drop(guard);

        if self.paused.load(Ordering::Acquire) {
            // While paused, return the elapsed time up to the pause point.
            let pause_guard = self.pause_start.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pause_instant) = *pause_guard {
                let elapsed = pause_instant.duration_since(start).as_micros() as i64;
                let offset = *self
                    .pause_offset_us
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                return elapsed - offset;
            }
        }

        let elapsed = start.elapsed().as_micros() as i64;
        let offset = *self
            .pause_offset_us
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        elapsed - offset
    }

    /// Return `true` if the given frame PTS (in microseconds) is at or
    /// before the current audio position, meaning it should be displayed.
    pub fn should_display_frame(&self, frame_pts_us: i64) -> bool {
        if self.paused.load(Ordering::Acquire) {
            return false;
        }
        frame_pts_us <= self.current_pts_us()
    }

    /// Pause the clock. While paused, [`Self::should_display_frame`] always
    /// returns `false` and [`Self::current_pts_us`] is frozen.
    pub fn pause(&self) {
        if self.paused.swap(true, Ordering::AcqRel) {
            return; // Already paused.
        }
        let mut ps = self.pause_start.lock().unwrap_or_else(|e| e.into_inner());
        *ps = Some(Instant::now());
    }

    /// Resume the clock after a pause. Accumulates the paused duration so
    /// that wall-clock fallback stays accurate.
    pub fn resume(&self) {
        if !self.paused.swap(false, Ordering::AcqRel) {
            return; // Not paused.
        }
        let mut ps = self.pause_start.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pause_instant) = ps.take() {
            let paused_dur = pause_instant.elapsed().as_micros() as i64;
            let mut offset = self
                .pause_offset_us
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *offset += paused_dur;
        }
    }

    /// Reset the clock to its initial state.
    ///
    /// Clears the start time, audio PTS, and pause state. Call [`Self::start`]
    /// again before reusing.
    pub fn reset(&self) {
        *self.audio_pts_us.lock().unwrap_or_else(|e| e.into_inner()) = 0;
        *self
            .pause_offset_us
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = 0;
        self.paused.store(false, Ordering::Release);
        {
            let mut ps = self.pause_start.lock().unwrap_or_else(|e| e.into_inner());
            *ps = None;
        }
        let mut s = self.start.lock().unwrap_or_else(|e| e.into_inner());
        *s = None;
    }

    /// Return `true` if the clock is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for SyncClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let audio = self.audio_pts_us.lock().map(|g| *g).unwrap_or_default();
        let offset = self.pause_offset_us.lock().map(|g| *g).unwrap_or_default();
        f.debug_struct("SyncClock")
            .field("audio_pts_us", &audio)
            .field("paused", &self.paused.load(Ordering::Relaxed))
            .field("pause_offset_us", &offset)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn new_clock_returns_zero() {
        let clock = SyncClock::new();
        assert_eq!(clock.current_pts_us(), 0);
        assert!(!clock.is_paused());
    }

    #[test]
    fn update_and_read_audio_pts() {
        let clock = SyncClock::new();
        clock.start();
        clock.update_audio_pts(500_000); // 0.5s
        assert_eq!(clock.current_pts_us(), 500_000);
    }

    #[test]
    fn should_display_frame_before_pts() {
        let clock = SyncClock::new();
        clock.start();
        clock.update_audio_pts(1_000_000); // 1.0s

        // Frame at 0.5s should display (it's in the past).
        assert!(clock.should_display_frame(500_000));
        // Frame at exactly 1.0s should display.
        assert!(clock.should_display_frame(1_000_000));
        // Frame at 1.5s should NOT display yet.
        assert!(!clock.should_display_frame(1_500_000));
    }

    #[test]
    fn pause_freezes_display() {
        let clock = SyncClock::new();
        clock.start();
        clock.update_audio_pts(1_000_000);

        clock.pause();
        assert!(clock.is_paused());
        // Even a frame in the past should not display while paused.
        assert!(!clock.should_display_frame(500_000));
    }

    #[test]
    fn resume_after_pause() {
        let clock = SyncClock::new();
        clock.start();
        clock.update_audio_pts(1_000_000);

        clock.pause();
        assert!(!clock.should_display_frame(500_000));

        clock.resume();
        assert!(!clock.is_paused());
        // After resume, frame in the past should display again.
        assert!(clock.should_display_frame(500_000));
    }

    #[test]
    fn double_pause_is_idempotent() {
        let clock = SyncClock::new();
        clock.start();
        clock.pause();
        clock.pause(); // Should not panic or double-count.
        assert!(clock.is_paused());
        clock.resume();
        assert!(!clock.is_paused());
    }

    #[test]
    fn double_resume_is_idempotent() {
        let clock = SyncClock::new();
        clock.start();
        clock.pause();
        clock.resume();
        clock.resume(); // Should not panic or double-count.
        assert!(!clock.is_paused());
    }

    #[test]
    fn reset_clears_state() {
        let clock = SyncClock::new();
        clock.start();
        clock.update_audio_pts(5_000_000);
        clock.pause();

        clock.reset();
        assert_eq!(clock.current_pts_us(), 0);
        assert!(!clock.is_paused());
    }

    #[test]
    fn start_resets_previous_state() {
        let clock = SyncClock::new();
        clock.start();
        clock.update_audio_pts(3_000_000);
        clock.pause();

        // Calling start again should reset everything.
        clock.start();
        assert!(!clock.is_paused());
        // Audio PTS was reset to 0, so current_pts_us uses wall-clock
        // fallback which should be very small (just started).
        let pts = clock.current_pts_us();
        assert!(
            pts < 100_000,
            "expected near-zero PTS after restart, got {pts}"
        );
    }

    #[test]
    fn wall_clock_fallback_advances() {
        let clock = SyncClock::new();
        clock.start();
        // Don't update audio PTS -- should fall back to wall clock.
        thread::sleep(Duration::from_millis(50));
        let pts = clock.current_pts_us();
        // Should have advanced by at least ~40ms (allowing scheduler slack).
        assert!(
            pts >= 40_000,
            "wall-clock fallback should advance, got {pts}us"
        );
    }

    #[test]
    fn wall_clock_pauses_correctly() {
        let clock = SyncClock::new();
        clock.start();
        // Let some time pass.
        thread::sleep(Duration::from_millis(50));

        clock.pause();
        let pts_at_pause = clock.current_pts_us();

        // Sleep while paused.
        thread::sleep(Duration::from_millis(50));
        let pts_still_paused = clock.current_pts_us();

        // PTS should not have advanced significantly while paused.
        let drift = (pts_still_paused - pts_at_pause).unsigned_abs();
        assert!(
            drift < 5_000,
            "PTS should be frozen while paused, drifted {drift}us"
        );
    }

    #[test]
    fn cross_thread_update() {
        let clock = SyncClock::new();
        clock.start();

        let clock2 = Arc::clone(&clock);
        let handle = thread::spawn(move || {
            clock2.update_audio_pts(2_000_000);
        });
        handle.join().expect("thread panicked");

        assert_eq!(clock.current_pts_us(), 2_000_000);
        assert!(clock.should_display_frame(1_500_000));
        assert!(!clock.should_display_frame(2_500_000));
    }

    #[test]
    fn debug_impl() {
        let clock = SyncClock::new();
        clock.update_audio_pts(123_456);
        let dbg = format!("{clock:?}");
        assert!(dbg.contains("SyncClock"));
        assert!(dbg.contains("123456"));
    }

    #[test]
    fn audio_pts_takes_precedence_over_wall_clock() {
        let clock = SyncClock::new();
        clock.start();
        // Let wall clock advance.
        thread::sleep(Duration::from_millis(100));
        // Now set audio PTS to a value less than wall clock elapsed.
        clock.update_audio_pts(10_000); // 10ms
        // Audio PTS should be the authoritative value.
        assert_eq!(clock.current_pts_us(), 10_000);
    }
}
