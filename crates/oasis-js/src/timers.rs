//! Frame-driven timer queue for `setTimeout` / `setInterval`.
//!
//! Timers are registered from JS closures and fired externally by the
//! host (e.g. the browser widget's tick method) via [`TimerQueue::tick`].

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
}

impl TimerQueue {
    /// Create an empty timer queue.
    pub fn new() -> Self {
        Self {
            timers: Vec::new(),
            next_id: 1,
            elapsed_ms: 0.0,
        }
    }

    /// Register a one-shot timeout. Returns the timer ID.
    pub fn add_timeout(&mut self, callback_global: String, delay_ms: f64) -> i32 {
        let id = self.next_id;
        self.next_id += 1;
        self.timers.push(Timer {
            id,
            callback_global,
            fire_at_ms: self.elapsed_ms + delay_ms,
            interval_ms: None,
        });
        id
    }

    /// Register a repeating interval. Returns the timer ID.
    pub fn add_interval(&mut self, callback_global: String, delay_ms: f64) -> i32 {
        let id = self.next_id;
        self.next_id += 1;
        self.timers.push(Timer {
            id,
            callback_global,
            fire_at_ms: self.elapsed_ms + delay_ms,
            interval_ms: Some(delay_ms),
        });
        id
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
    pub fn tick(&mut self, dt_ms: f64) -> Vec<String> {
        self.elapsed_ms += dt_ms;
        let mut callbacks = Vec::new();
        let mut to_remove = Vec::new();
        let mut to_reschedule: Vec<(usize, f64)> = Vec::new();

        for (idx, timer) in self.timers.iter().enumerate() {
            if self.elapsed_ms >= timer.fire_at_ms {
                if let Some(iv) = timer.interval_ms {
                    // Interval: call but don't delete the global.
                    callbacks.push(format!(
                        "if(typeof {g}==='function'){{{g}();}}",
                        g = timer.callback_global,
                    ));
                    to_reschedule.push((idx, iv));
                } else {
                    // Timeout: delete the global before calling so it
                    // is cleaned up even if the callback throws.
                    callbacks.push(format!(
                        "var __f=globalThis.{g};\
                         delete globalThis.{g};\
                         if(typeof __f==='function'){{__f();}}",
                        g = timer.callback_global,
                    ));
                    to_remove.push(timer.id);
                }
            }
        }

        // Reschedule intervals.
        for (idx, iv) in &to_reschedule {
            if let Some(timer) = self.timers.get_mut(*idx) {
                timer.fire_at_ms = self.elapsed_ms + iv;
            }
        }

        // Remove fired one-shot timers.
        if !to_remove.is_empty() {
            self.timers.retain(|t| !to_remove.contains(&t.id));
        }

        callbacks
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
}
