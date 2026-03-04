//! Plugin event bus for inter-plugin communication.
//!
//! Provides a simple publish/subscribe message passing system. Plugins can
//! subscribe to named topics and publish events that other plugins receive.
//!
//! Events are string-based for simplicity and cross-language compatibility.

use std::collections::HashMap;

/// A message on the event bus.
#[derive(Debug, Clone)]
pub struct PluginEvent {
    /// Topic name (e.g. "audio.track_changed", "settings.updated").
    pub topic: String,
    /// Source plugin name.
    pub source: String,
    /// Event payload (JSON, plain text, or empty).
    pub data: String,
}

impl PluginEvent {
    /// Create a new event.
    pub fn new(
        topic: impl Into<String>,
        source: impl Into<String>,
        data: impl Into<String>,
    ) -> Self {
        Self {
            topic: topic.into(),
            source: source.into(),
            data: data.into(),
        }
    }
}

/// Simple publish/subscribe event bus for plugin-to-plugin communication.
///
/// Events are buffered until `drain()` is called. Subscribers receive all
/// events matching their topic filter.
pub struct EventBus {
    /// Pending events not yet drained.
    pending: Vec<PluginEvent>,
    /// Topic subscriptions: topic -> list of subscriber plugin names.
    subscriptions: HashMap<String, Vec<String>>,
}

impl EventBus {
    /// Create a new empty event bus.
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            subscriptions: HashMap::new(),
        }
    }

    /// Subscribe a plugin to a topic.
    pub fn subscribe(&mut self, plugin_name: &str, topic: &str) {
        self.subscriptions
            .entry(topic.to_string())
            .or_default()
            .push(plugin_name.to_string());
    }

    /// Unsubscribe a plugin from a topic.
    pub fn unsubscribe(&mut self, plugin_name: &str, topic: &str) {
        if let Some(subs) = self.subscriptions.get_mut(topic) {
            subs.retain(|s| s != plugin_name);
        }
    }

    /// Unsubscribe a plugin from all topics (used during shutdown).
    pub fn unsubscribe_all(&mut self, plugin_name: &str) {
        for subs in self.subscriptions.values_mut() {
            subs.retain(|s| s != plugin_name);
        }
    }

    /// Publish an event to the bus.
    ///
    /// The event is buffered and delivered to subscribers when `drain()` is
    /// called by the host.
    pub fn publish(&mut self, event: PluginEvent) {
        self.pending.push(event);
    }

    /// Drain all pending events, returning only those for the given subscriber.
    pub fn drain_for(&mut self, plugin_name: &str) -> Vec<PluginEvent> {
        // Collect events whose topic this plugin subscribes to.
        let mut result = Vec::new();
        for event in &self.pending {
            // Don't deliver events back to the source plugin.
            if event.source == plugin_name {
                continue;
            }
            if let Some(subs) = self.subscriptions.get(&event.topic)
                && subs.iter().any(|s| s == plugin_name)
            {
                result.push(event.clone());
            }
        }
        result
    }

    /// Clear all pending events. Called after all plugins have drained.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// Number of pending events.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Number of topics with at least one subscriber.
    pub fn topic_count(&self) -> usize {
        self.subscriptions
            .values()
            .filter(|v| !v.is_empty())
            .count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_and_drain() {
        let mut bus = EventBus::new();
        bus.subscribe("listener", "audio.changed");
        bus.publish(PluginEvent::new("audio.changed", "player", "track1"));

        let events = bus.drain_for("listener");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "audio.changed");
        assert_eq!(events[0].source, "player");
        assert_eq!(events[0].data, "track1");
    }

    #[test]
    fn source_does_not_receive_own_events() {
        let mut bus = EventBus::new();
        bus.subscribe("player", "audio.changed");
        bus.publish(PluginEvent::new("audio.changed", "player", "data"));

        let events = bus.drain_for("player");
        assert!(events.is_empty());
    }

    #[test]
    fn unsubscribed_plugins_miss_events() {
        let mut bus = EventBus::new();
        bus.publish(PluginEvent::new("test.topic", "source", "data"));

        let events = bus.drain_for("unsubscribed");
        assert!(events.is_empty());
    }

    #[test]
    fn unsubscribe_stops_delivery() {
        let mut bus = EventBus::new();
        bus.subscribe("listener", "topic");
        bus.unsubscribe("listener", "topic");
        bus.publish(PluginEvent::new("topic", "source", "data"));

        let events = bus.drain_for("listener");
        assert!(events.is_empty());
    }

    #[test]
    fn unsubscribe_all_cleans_up() {
        let mut bus = EventBus::new();
        bus.subscribe("listener", "topic1");
        bus.subscribe("listener", "topic2");
        bus.unsubscribe_all("listener");
        bus.publish(PluginEvent::new("topic1", "source", "data"));
        bus.publish(PluginEvent::new("topic2", "source", "data"));

        let events = bus.drain_for("listener");
        assert!(events.is_empty());
    }

    #[test]
    fn multiple_subscribers() {
        let mut bus = EventBus::new();
        bus.subscribe("a", "topic");
        bus.subscribe("b", "topic");
        bus.publish(PluginEvent::new("topic", "source", "data"));

        assert_eq!(bus.drain_for("a").len(), 1);
        assert_eq!(bus.drain_for("b").len(), 1);
    }

    #[test]
    fn clear_pending_removes_events() {
        let mut bus = EventBus::new();
        bus.subscribe("listener", "topic");
        bus.publish(PluginEvent::new("topic", "source", "data"));
        assert_eq!(bus.pending_count(), 1);

        bus.clear_pending();
        assert_eq!(bus.pending_count(), 0);
        assert!(bus.drain_for("listener").is_empty());
    }

    #[test]
    fn topic_count() {
        let mut bus = EventBus::new();
        assert_eq!(bus.topic_count(), 0);
        bus.subscribe("a", "topic1");
        bus.subscribe("b", "topic2");
        assert_eq!(bus.topic_count(), 2);
    }

    #[test]
    fn event_new_builder() {
        let event = PluginEvent::new("topic", "source", "payload");
        assert_eq!(event.topic, "topic");
        assert_eq!(event.source, "source");
        assert_eq!(event.data, "payload");
    }
}
