//! UI sound events and their per-frame dispatch queue.
//!
//! Input handlers and other chokepoints push [`UiSound`] events into a
//! [`UiSoundQueue`] as they run; the shell drains the queue once per frame
//! and triggers the matching skin-defined sample (if the active skin ships
//! one — skins without a `[sounds]` table stay silent).
//!
//! The queue is deliberately dumb: no audio types, no backend handles.
//! That keeps the hooks cheap to sprinkle at the few central seams (icon
//! launch, window close, toast creation, d-pad navigation) without
//! threading an audio reference through every handler.

/// A user-interface sound event. Maps 1:1 onto the `[sounds]` keys in a
/// skin's theme.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiSound {
    /// Button / interactive element click.
    Click,
    /// App launch / window open.
    Open,
    /// App / window close.
    Close,
    /// Error feedback (error toasts).
    Error,
    /// Non-error toast notification.
    Toast,
    /// Cursor / d-pad navigation between icons (rate-limited).
    Nav,
}

impl UiSound {
    /// The `[sounds]` table key (and SFX sample name) for this event.
    pub fn key(self) -> &'static str {
        match self {
            UiSound::Click => "click",
            UiSound::Open => "open",
            UiSound::Close => "close",
            UiSound::Error => "error",
            UiSound::Toast => "toast",
            UiSound::Nav => "nav",
        }
    }

    /// All events, in `[sounds]` key order.
    pub const ALL: [UiSound; 6] = [
        UiSound::Click,
        UiSound::Open,
        UiSound::Close,
        UiSound::Error,
        UiSound::Toast,
        UiSound::Nav,
    ];
}

/// Minimum frames between two Nav sounds, so holding a d-pad direction
/// doesn't machine-gun the cursor sample (~12 Hz max at 60 fps).
pub const NAV_MIN_GAP_FRAMES: u64 = 5;

/// Upper bound on queued events per frame; anything beyond this within a
/// single frame would be an inaudible smear anyway.
const MAX_QUEUED: usize = 16;

/// Collects [`UiSound`] events during a frame for the shell to drain.
#[derive(Debug, Default)]
pub struct UiSoundQueue {
    events: Vec<UiSound>,
    last_nav_frame: Option<u64>,
    toasts_seen: u64,
    error_toasts_seen: u64,
}

impl UiSoundQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an event for this frame.
    pub fn push(&mut self, sound: UiSound) {
        if self.events.len() < MAX_QUEUED {
            self.events.push(sound);
        }
    }

    /// Queue a Nav event, rate-limited by [`NAV_MIN_GAP_FRAMES`] so a held
    /// direction repeats at a musical rate instead of every input event.
    pub fn push_nav(&mut self, frame: u64) {
        if let Some(last) = self.last_nav_frame
            && frame.saturating_sub(last) < NAV_MIN_GAP_FRAMES
        {
            return;
        }
        self.last_nav_frame = Some(frame);
        self.push(UiSound::Nav);
    }

    /// Derive Toast / Error events from the toast manager's monotonic
    /// counters (`(total_shown, errors_shown)`), so every toast call site
    /// is covered without instrumenting each one.
    pub fn observe_toasts(&mut self, shown: u64, errors: u64) {
        let new_total = shown.saturating_sub(self.toasts_seen);
        let new_errors = errors.saturating_sub(self.error_toasts_seen).min(new_total);
        for _ in 0..new_errors {
            self.push(UiSound::Error);
        }
        for _ in 0..new_total.saturating_sub(new_errors) {
            self.push(UiSound::Toast);
        }
        self.toasts_seen = shown;
        self.error_toasts_seen = errors;
    }

    /// Take all queued events (frame drain).
    pub fn drain(&mut self) -> Vec<UiSound> {
        std::mem::take(&mut self.events)
    }

    /// Number of currently queued events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether no events are queued.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_drain() {
        let mut q = UiSoundQueue::new();
        q.push(UiSound::Click);
        q.push(UiSound::Open);
        assert_eq!(q.len(), 2);
        assert_eq!(q.drain(), vec![UiSound::Click, UiSound::Open]);
        assert!(q.is_empty());
        assert!(q.drain().is_empty());
    }

    #[test]
    fn nav_is_rate_limited() {
        let mut q = UiSoundQueue::new();
        q.push_nav(100);
        q.push_nav(101); // Too soon — swallowed.
        q.push_nav(102);
        assert_eq!(q.drain(), vec![UiSound::Nav]);
        q.push_nav(100 + NAV_MIN_GAP_FRAMES);
        assert_eq!(q.drain(), vec![UiSound::Nav]);
    }

    #[test]
    fn nav_fires_after_gap() {
        let mut q = UiSoundQueue::new();
        q.push_nav(10);
        q.push_nav(10 + NAV_MIN_GAP_FRAMES);
        q.push_nav(10 + 2 * NAV_MIN_GAP_FRAMES);
        assert_eq!(q.drain().len(), 3);
    }

    #[test]
    fn observe_toasts_derives_events() {
        let mut q = UiSoundQueue::new();
        q.observe_toasts(0, 0);
        assert!(q.is_empty());
        // Two toasts appear, one of them an error.
        q.observe_toasts(2, 1);
        let events = q.drain();
        assert_eq!(events.len(), 2);
        assert_eq!(events.iter().filter(|s| **s == UiSound::Error).count(), 1);
        assert_eq!(events.iter().filter(|s| **s == UiSound::Toast).count(), 1);
        // No change — no new events.
        q.observe_toasts(2, 1);
        assert!(q.is_empty());
    }

    #[test]
    fn queue_is_bounded() {
        let mut q = UiSoundQueue::new();
        for _ in 0..100 {
            q.push(UiSound::Click);
        }
        assert!(q.len() <= 16);
    }

    #[test]
    fn keys_match_sounds_table() {
        let keys: Vec<&str> = UiSound::ALL.iter().map(|s| s.key()).collect();
        assert_eq!(
            keys,
            vec!["click", "open", "close", "error", "toast", "nav"]
        );
    }
}
