//! Frame-driven timer queue for `setTimeout` / `setInterval`.
//!
//! Timers are registered from JS closures and fired externally by the
//! host (e.g. the browser widget's tick method) via
//! [`TimerQueue::tick_fired`] (or the legacy string-based
//! [`TimerQueue::tick`]).
//!
//! Watchdog limits:
//! - at most [`MAX_LIVE_TIMERS`] timers may be pending at once; further
//!   registrations are refused and return timer ID `0`;
//! - delays that are NaN, infinite or negative are treated as `0`, and
//!   delays above `i32::MAX` ms are clamped to it (WebIDL `long`);
//! - interval periods are clamped to at least [`MIN_INTERVAL_MS`] (the
//!   HTML spec's nested-timer clamp), so `setInterval(f, 0)` cannot
//!   schedule a zero-period repeat.

/// Maximum number of live (pending) timers per queue.
pub const MAX_LIVE_TIMERS: usize = 1_000;

/// Minimum period for `setInterval`, in milliseconds.
pub const MIN_INTERVAL_MS: f64 = 4.0;

/// Sanitize a JS-supplied delay: non-finite or negative becomes `0`,
/// oversized values clamp to `i32::MAX` milliseconds.
fn sanitize_delay(delay_ms: f64) -> f64 {
    if !delay_ms.is_finite() || delay_ms < 0.0 {
        0.0
    } else {
        delay_ms.min(i32::MAX as f64)
    }
}

/// A timer that fired during [`TimerQueue::tick_fired`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiredTimer {
    /// Timer ID as returned to JS by `setTimeout` / `setInterval`.
    pub id: i32,
    /// `true` for `setInterval` timers (the callback stays registered),
    /// `false` for one-shot timeouts.
    pub repeat: bool,
}

/// A pending timer (`setTimeout` or `setInterval`).
pub(crate) struct Timer {
    id: i32,
    /// JS callback stored as a global name (e.g. `__oasis_timer_cb_1`).
    callback_global: String,
    /// When to fire (in milliseconds from queue creation).
    fire_at_ms: f64,
    /// Repeat interval (`None` for setTimeout, `Some(ms)` for
    /// setInterval).
    interval_ms: Option<f64>,
}

impl Timer {
    /// Replace the callback global name (set after allocation).
    pub(crate) fn set_callback_global(&mut self, name: String) {
        self.callback_global = name;
    }
}

/// Shared timer state between JS closures and the host.
pub struct TimerQueue {
    timers: Vec<Timer>,
    next_id: i32,
    elapsed_ms: f64,
    /// Set once a registration has been refused because the queue is
    /// full; cleared again when a registration succeeds. Lets the
    /// caller warn once per overflow episode instead of once per call.
    cap_warned: bool,
}

impl TimerQueue {
    /// Create an empty timer queue.
    pub fn new() -> Self {
        Self {
            timers: Vec::new(),
            next_id: 1,
            elapsed_ms: 0.0,
            cap_warned: false,
        }
    }

    /// Number of live (pending) timers.
    pub fn len(&self) -> usize {
        self.timers.len()
    }

    /// `true` when no timers are pending.
    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// Record a refused registration. Returns `true` only for the first
    /// refusal since the last successful registration, so the caller
    /// can emit a single warning.
    pub(crate) fn note_cap_hit(&mut self) -> bool {
        !std::mem::replace(&mut self.cap_warned, true)
    }

    /// Push a new timer, or return `0` when [`MAX_LIVE_TIMERS`] are
    /// already pending.
    fn push(&mut self, callback_global: String, delay_ms: f64, interval_ms: Option<f64>) -> i32 {
        if self.timers.len() >= MAX_LIVE_TIMERS {
            return 0;
        }
        self.cap_warned = false;
        let id = self.next_id;
        // Wrap back to 1 instead of overflowing; ID 0 means "refused".
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        self.timers.push(Timer {
            id,
            callback_global,
            fire_at_ms: self.elapsed_ms + delay_ms,
            interval_ms,
        });
        id
    }

    /// Register a one-shot timeout. Returns the timer ID, or `0` if the
    /// queue already holds [`MAX_LIVE_TIMERS`] timers.
    ///
    /// NaN / infinite / negative delays are treated as `0`.
    pub fn add_timeout(&mut self, callback_global: String, delay_ms: f64) -> i32 {
        self.push(callback_global, sanitize_delay(delay_ms), None)
    }

    /// Register a repeating interval. Returns the timer ID, or `0` if
    /// the queue already holds [`MAX_LIVE_TIMERS`] timers.
    ///
    /// The period is sanitized like a timeout delay and then clamped to
    /// at least [`MIN_INTERVAL_MS`].
    pub fn add_interval(&mut self, callback_global: String, delay_ms: f64) -> i32 {
        let period = sanitize_delay(delay_ms).max(MIN_INTERVAL_MS);
        self.push(callback_global, period, Some(period))
    }

    /// Mutable access to the underlying timer list.
    pub(crate) fn timers_mut(&mut self) -> &mut Vec<Timer> {
        &mut self.timers
    }

    /// Remove a timer by ID (works for both timeouts and intervals).
    pub fn clear(&mut self, id: i32) {
        self.timers.retain(|t| t.id != id);
    }

    /// Advance elapsed time by `dt_ms` and return JS eval strings for
    /// every timer that should fire.
    ///
    /// One-shot timeouts are removed after firing. Intervals are
    /// rescheduled.
    ///
    /// Prefer [`tick_fired`](Self::tick_fired): these strings have to be
    /// compiled on every firing. Kept for API compatibility.
    pub fn tick(&mut self, dt_ms: f64) -> Vec<String> {
        self.advance(dt_ms, |timer| {
            if timer.interval_ms.is_some() {
                // Interval: call but don't delete the global.
                format!(
                    "if(typeof {g}==='function'){{{g}();}}",
                    g = timer.callback_global,
                )
            } else {
                // Timeout: delete the global before calling so it
                // is cleaned up even if the callback throws.
                format!(
                    "var __f=globalThis.{g};\
                     delete globalThis.{g};\
                     if(typeof __f==='function'){{__f();}}",
                    g = timer.callback_global,
                )
            }
        })
    }

    /// Advance elapsed time by `dt_ms` and return every timer that
    /// fired, in registration order.
    ///
    /// One-shot timeouts are removed after firing; intervals are
    /// rescheduled one period after the current time (no catch-up burst
    /// after a long frame). A NaN, infinite or negative `dt_ms` is
    /// treated as `0`.
    pub fn tick_fired(&mut self, dt_ms: f64) -> Vec<FiredTimer> {
        self.advance(dt_ms, |timer| FiredTimer {
            id: timer.id,
            repeat: timer.interval_ms.is_some(),
        })
    }

    /// Shared implementation of [`tick`](Self::tick) and
    /// [`tick_fired`](Self::tick_fired).
    fn advance<T>(&mut self, dt_ms: f64, mut on_fire: impl FnMut(&Timer) -> T) -> Vec<T> {
        if dt_ms.is_finite() && dt_ms > 0.0 {
            self.elapsed_ms += dt_ms;
        }
        let now = self.elapsed_ms;
        let mut fired = Vec::new();
        let mut to_remove = Vec::new();

        for timer in &mut self.timers {
            if now >= timer.fire_at_ms {
                fired.push(on_fire(timer));
                match timer.interval_ms {
                    Some(iv) => timer.fire_at_ms = now + iv,
                    None => to_remove.push(timer.id),
                }
            }
        }

        // Remove fired one-shot timers.
        if !to_remove.is_empty() {
            self.timers.retain(|t| !to_remove.contains(&t.id));
        }

        fired
    }
}

impl Default for TimerQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_fires_after_delay() {
        let mut q = TimerQueue::new();
        let id = q.add_timeout("__cb_1".into(), 100.0);
        assert_eq!(id, 1);

        // Not enough time yet.
        let fired = q.tick(50.0);
        assert!(fired.is_empty());

        // Now it should fire.
        let fired = q.tick(60.0);
        assert_eq!(fired.len(), 1);
        assert!(fired[0].contains("__cb_1"));
    }

    #[test]
    fn interval_fires_repeatedly() {
        let mut q = TimerQueue::new();
        q.add_interval("__cb_iv".into(), 100.0);

        // First fire.
        let fired = q.tick(100.0);
        assert_eq!(fired.len(), 1);
        assert!(fired[0].contains("__cb_iv"));
        // Should NOT contain `delete`.
        assert!(!fired[0].contains("delete"));

        // Second fire after another interval.
        let fired = q.tick(100.0);
        assert_eq!(fired.len(), 1);

        // Third.
        let fired = q.tick(100.0);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn clear_removes_timer() {
        let mut q = TimerQueue::new();
        let id = q.add_timeout("__cb_c".into(), 100.0);
        q.clear(id);

        let fired = q.tick(200.0);
        assert!(fired.is_empty());
    }

    #[test]
    fn timeout_removed_after_fire() {
        let mut q = TimerQueue::new();
        q.add_timeout("__cb_once".into(), 50.0);

        // Fire it.
        let fired = q.tick(60.0);
        assert_eq!(fired.len(), 1);

        // Should not fire again.
        let fired = q.tick(100.0);
        assert!(fired.is_empty());
    }

    #[test]
    fn tick_fired_reports_ids_and_kind() {
        let mut q = TimerQueue::new();
        let t = q.add_timeout(String::new(), 10.0);
        let i = q.add_interval(String::new(), 10.0);
        let fired = q.tick_fired(10.0);
        assert_eq!(
            fired,
            vec![
                FiredTimer {
                    id: t,
                    repeat: false
                },
                FiredTimer {
                    id: i,
                    repeat: true
                },
            ]
        );
        // Only the interval survives.
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn nan_and_negative_intervals_clamp_to_min() {
        for bad in [f64::NAN, -5.0, 0.0, f64::INFINITY, f64::NEG_INFINITY] {
            let mut q = TimerQueue::new();
            q.add_interval(String::new(), bad);
            // Below the clamp: nothing fires.
            assert!(
                q.tick_fired(MIN_INTERVAL_MS - 1.0).is_empty(),
                "delay {bad}"
            );
            assert_eq!(q.tick_fired(1.0).len(), 1, "delay {bad}");
            // One firing per tick at most, even for a huge dt.
            assert_eq!(q.tick_fired(1_000.0).len(), 1, "delay {bad}");
        }
    }

    #[test]
    fn nan_timeout_fires_immediately_once() {
        let mut q = TimerQueue::new();
        q.add_timeout(String::new(), f64::NAN);
        assert_eq!(q.tick_fired(0.0).len(), 1);
        assert!(q.is_empty());
    }

    #[test]
    fn nan_dt_does_not_poison_clock() {
        let mut q = TimerQueue::new();
        q.add_timeout(String::new(), 10.0);
        assert!(q.tick_fired(f64::NAN).is_empty());
        assert!(q.tick_fired(-100.0).is_empty());
        assert_eq!(q.tick_fired(10.0).len(), 1);
    }

    #[test]
    fn live_timer_cap_refuses_registration() {
        let mut q = TimerQueue::new();
        for _ in 0..MAX_LIVE_TIMERS {
            assert_ne!(q.add_timeout(String::new(), 1_000.0), 0);
        }
        assert_eq!(q.add_timeout(String::new(), 1_000.0), 0);
        assert_eq!(q.add_interval(String::new(), 1_000.0), 0);
        assert_eq!(q.len(), MAX_LIVE_TIMERS);
        assert!(q.note_cap_hit());
        assert!(!q.note_cap_hit());
        // Firing frees slots again.
        q.tick_fired(1_000.0);
        assert!(q.is_empty());
        assert_ne!(q.add_timeout(String::new(), 1.0), 0);
        assert!(q.note_cap_hit(), "warning re-armed after a successful add");
    }
}
