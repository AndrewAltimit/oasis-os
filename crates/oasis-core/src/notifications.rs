//! System-level notification manager.
//!
//! Provides a [`NotificationManager`] that queues [`Notification`] items with
//! automatic expiry based on severity level. This is the data-model layer;
//! rendering is handled separately by the toast / SDI system.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Severity level of a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    /// Informational message.
    Info,
    /// Positive outcome / success feedback.
    Success,
    /// Caution / warning.
    Warning,
    /// Error / failure.
    Error,
}

/// Default TTL for Info and Success notifications.
const DEFAULT_TTL_SHORT: Duration = Duration::from_secs(5);

/// Default TTL for Warning and Error notifications.
const DEFAULT_TTL_LONG: Duration = Duration::from_secs(10);

/// Return the default TTL for a given notification level.
fn default_ttl(level: NotificationLevel) -> Duration {
    match level {
        NotificationLevel::Info | NotificationLevel::Success => DEFAULT_TTL_SHORT,
        NotificationLevel::Warning | NotificationLevel::Error => DEFAULT_TTL_LONG,
    }
}

/// A single notification with message, severity, creation time, and TTL.
#[derive(Debug, Clone)]
pub struct Notification {
    /// Human-readable message.
    pub message: String,
    /// Severity level.
    pub level: NotificationLevel,
    /// When this notification was created.
    pub created_at: Instant,
    /// How long this notification should remain visible.
    pub ttl: Duration,
}

impl Notification {
    /// Whether this notification has expired.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.ttl
    }

    /// Fraction of lifetime elapsed, from 0.0 (just created) to 1.0 (expired).
    pub fn progress(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.ttl.as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        (elapsed / total).min(1.0)
    }
}

/// Manages a queue of system notifications with automatic expiry.
///
/// Call [`tick`](Self::tick) periodically to remove expired notifications.
/// Use [`visible`](Self::visible) to retrieve the currently active slice
/// for rendering.
#[derive(Debug)]
pub struct NotificationManager {
    queue: VecDeque<Notification>,
    max_visible: usize,
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationManager {
    /// Create a new empty notification manager.
    ///
    /// Default max visible: 5.
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            max_visible: 5,
        }
    }

    /// Create a notification manager with a custom max-visible limit.
    pub fn with_max_visible(mut self, max: usize) -> Self {
        self.max_visible = max;
        self
    }

    /// Push a notification with the default TTL for its level.
    pub fn push(&mut self, message: impl Into<String>, level: NotificationLevel) {
        self.push_with_ttl(message, level, default_ttl(level));
    }

    /// Push a notification with a custom TTL.
    pub fn push_with_ttl(
        &mut self,
        message: impl Into<String>,
        level: NotificationLevel,
        ttl: Duration,
    ) {
        self.queue.push_back(Notification {
            message: message.into(),
            level,
            created_at: Instant::now(),
            ttl,
        });
        // Keep the queue bounded (2x max_visible as buffer).
        let cap = self.max_visible * 2;
        while self.queue.len() > cap {
            self.queue.pop_front();
        }
    }

    /// Remove expired notifications.
    pub fn tick(&mut self) {
        self.queue.retain(|n| !n.is_expired());
    }

    /// Return the currently visible notifications (most recent, up to
    /// `max_visible`).
    pub fn visible(&self) -> Vec<&Notification> {
        self.queue.iter().rev().take(self.max_visible).collect()
    }

    /// Dismiss the notification at the given index in the queue.
    ///
    /// Returns `true` if a notification was removed.
    pub fn dismiss(&mut self, index: usize) -> bool {
        if index < self.queue.len() {
            self.queue.remove(index);
            true
        } else {
            false
        }
    }

    /// Remove all notifications.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Number of notifications currently in the queue.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_adds_notification() {
        let mut mgr = NotificationManager::new();
        mgr.push("hello", NotificationLevel::Info);
        assert_eq!(mgr.len(), 1);
        assert!(!mgr.is_empty());
    }

    #[test]
    fn push_multiple_levels() {
        let mut mgr = NotificationManager::new();
        mgr.push("info", NotificationLevel::Info);
        mgr.push("success", NotificationLevel::Success);
        mgr.push("warning", NotificationLevel::Warning);
        mgr.push("error", NotificationLevel::Error);
        assert_eq!(mgr.len(), 4);
    }

    #[test]
    fn push_with_custom_ttl() {
        let mut mgr = NotificationManager::new();
        mgr.push_with_ttl("custom", NotificationLevel::Info, Duration::from_secs(30));
        assert_eq!(mgr.len(), 1);
        let vis = mgr.visible();
        assert_eq!(vis[0].ttl, Duration::from_secs(30));
    }

    #[test]
    fn default_ttl_info_success() {
        assert_eq!(default_ttl(NotificationLevel::Info), Duration::from_secs(5));
        assert_eq!(
            default_ttl(NotificationLevel::Success),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn default_ttl_warning_error() {
        assert_eq!(
            default_ttl(NotificationLevel::Warning),
            Duration::from_secs(10)
        );
        assert_eq!(
            default_ttl(NotificationLevel::Error),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn tick_removes_expired() {
        let mut mgr = NotificationManager::new();
        mgr.push_with_ttl(
            "expire me",
            NotificationLevel::Info,
            Duration::from_millis(0),
        );
        // The notification has zero TTL so it is already expired.
        mgr.tick();
        assert!(mgr.is_empty());
    }

    #[test]
    fn tick_retains_active() {
        let mut mgr = NotificationManager::new();
        mgr.push("still alive", NotificationLevel::Info);
        mgr.tick();
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn visible_returns_up_to_max() {
        let mut mgr = NotificationManager::new().with_max_visible(2);
        for i in 0..5 {
            mgr.push(format!("msg {i}"), NotificationLevel::Info);
        }
        let vis = mgr.visible();
        assert_eq!(vis.len(), 2);
    }

    #[test]
    fn visible_returns_most_recent() {
        let mut mgr = NotificationManager::new().with_max_visible(2);
        mgr.push("old", NotificationLevel::Info);
        mgr.push("mid", NotificationLevel::Info);
        mgr.push("new", NotificationLevel::Info);
        let vis = mgr.visible();
        // visible() returns most recent first (reversed).
        assert_eq!(vis[0].message, "new");
        assert_eq!(vis[1].message, "mid");
    }

    #[test]
    fn dismiss_removes_by_index() {
        let mut mgr = NotificationManager::new();
        mgr.push("a", NotificationLevel::Info);
        mgr.push("b", NotificationLevel::Warning);
        mgr.push("c", NotificationLevel::Error);
        assert!(mgr.dismiss(1));
        assert_eq!(mgr.len(), 2);
        // "b" was removed; remaining are "a" and "c".
        let vis = mgr.visible();
        let messages: Vec<&str> = vis.iter().map(|n| n.message.as_str()).collect();
        assert!(messages.contains(&"a"));
        assert!(messages.contains(&"c"));
        assert!(!messages.contains(&"b"));
    }

    #[test]
    fn dismiss_out_of_bounds() {
        let mut mgr = NotificationManager::new();
        mgr.push("only", NotificationLevel::Info);
        assert!(!mgr.dismiss(5));
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn clear_removes_all() {
        let mut mgr = NotificationManager::new();
        mgr.push("a", NotificationLevel::Info);
        mgr.push("b", NotificationLevel::Error);
        mgr.clear();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn queue_bounded_by_capacity() {
        let mut mgr = NotificationManager::new().with_max_visible(2);
        // Capacity = 2 * max_visible = 4.
        for i in 0..10 {
            mgr.push(format!("msg {i}"), NotificationLevel::Info);
        }
        assert!(mgr.len() <= 4);
    }

    #[test]
    fn notification_is_expired_zero_ttl() {
        let n = Notification {
            message: "x".to_string(),
            level: NotificationLevel::Info,
            created_at: Instant::now(),
            ttl: Duration::from_millis(0),
        };
        assert!(n.is_expired());
    }

    #[test]
    fn notification_not_expired_long_ttl() {
        let n = Notification {
            message: "x".to_string(),
            level: NotificationLevel::Info,
            created_at: Instant::now(),
            ttl: Duration::from_secs(3600),
        };
        assert!(!n.is_expired());
    }

    #[test]
    fn notification_progress_fresh() {
        let n = Notification {
            message: "x".to_string(),
            level: NotificationLevel::Info,
            created_at: Instant::now(),
            ttl: Duration::from_secs(10),
        };
        // Just created -- progress should be near zero.
        assert!(n.progress() < 0.1);
    }

    #[test]
    fn notification_progress_zero_ttl() {
        let n = Notification {
            message: "x".to_string(),
            level: NotificationLevel::Info,
            created_at: Instant::now(),
            ttl: Duration::from_millis(0),
        };
        assert!((n.progress() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn default_manager() {
        let mgr = NotificationManager::default();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn level_debug_format() {
        assert_eq!(format!("{:?}", NotificationLevel::Info), "Info");
        assert_eq!(format!("{:?}", NotificationLevel::Success), "Success");
        assert_eq!(format!("{:?}", NotificationLevel::Warning), "Warning");
        assert_eq!(format!("{:?}", NotificationLevel::Error), "Error");
    }

    #[test]
    fn level_equality() {
        assert_eq!(NotificationLevel::Info, NotificationLevel::Info);
        assert_ne!(NotificationLevel::Info, NotificationLevel::Error);
    }

    #[test]
    fn with_max_visible_builder() {
        let mgr = NotificationManager::new().with_max_visible(10);
        assert_eq!(mgr.max_visible, 10);
    }
}
