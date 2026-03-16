//! CSS transition engine for smooth property interpolation.
//!
//! Tracks active transitions and advances them each frame using
//! cubic-bezier easing curves for each [`TimingFunction`] variant.

use super::values::types::{TimingFunction, Transition};

/// An active transition being animated.
#[derive(Debug, Clone)]
struct ActiveTransition {
    property: String,
    from: f32,
    to: f32,
    duration_ms: f32,
    delay_ms: f32,
    timing: TimingFunction,
    elapsed_ms: f32,
}

impl ActiveTransition {
    /// Returns the interpolated value at the current elapsed time.
    fn current_value(&self) -> f32 {
        let active = self.elapsed_ms - self.delay_ms;
        if active <= 0.0 {
            return self.from;
        }
        if self.duration_ms <= 0.0 || active >= self.duration_ms {
            return self.to;
        }
        let t = active / self.duration_ms;
        let eased = ease(self.timing, t);
        self.from + (self.to - self.from) * eased
    }

    /// Whether this transition has completed.
    fn is_done(&self) -> bool {
        self.elapsed_ms >= self.delay_ms + self.duration_ms
    }
}

/// Evaluate a cubic bezier at parameter `t` given control points
/// `(x1, y1)` and `(x2, y2)`. The curve maps `[0, 1]` input progress
/// to `[0, 1]` eased output.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    // Newton-Raphson iteration to find the parameter `u` such that
    // `bezier_x(u) == t`, then return `bezier_y(u)`.
    let mut u = t;
    for _ in 0..8 {
        let bx = bezier(x1, x2, u) - t;
        let dx = bezier_derivative(x1, x2, u);
        if dx.abs() < 1e-6 {
            break;
        }
        u -= bx / dx;
        u = u.clamp(0.0, 1.0);
    }
    bezier(y1, y2, u)
}

/// Evaluate one axis of a cubic bezier with control points `p1, p2`
/// (endpoints fixed at 0 and 1).
fn bezier(p1: f32, p2: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    3.0 * mt2 * t * p1 + 3.0 * mt * t2 * p2 + t3
}

/// Derivative of the bezier function for Newton iteration.
fn bezier_derivative(p1: f32, p2: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    3.0 * mt * mt * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// Apply a timing function easing to a linear progress value `t` in `[0, 1]`.
fn ease(timing: TimingFunction, t: f32) -> f32 {
    match timing {
        TimingFunction::Linear => t,
        // CSS standard cubic-bezier values.
        TimingFunction::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
        TimingFunction::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        TimingFunction::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        TimingFunction::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
    }
}

/// Engine that manages active CSS transitions.
#[derive(Debug, Clone, Default)]
pub struct TransitionEngine {
    active: Vec<ActiveTransition>,
}

impl TransitionEngine {
    /// Create a new, empty transition engine.
    pub fn new() -> Self {
        Self { active: Vec::new() }
    }

    /// Begin a transition for the given property.
    ///
    /// If a transition for the same property is already running, it is
    /// replaced by the new one (starting from the current interpolated
    /// value).
    pub fn start_transition(
        &mut self,
        property: &str,
        from_value: f32,
        to_value: f32,
        transition: &Transition,
    ) {
        // Replace any existing transition on the same property.
        self.active.retain(|a| a.property != property);

        self.active.push(ActiveTransition {
            property: property.to_string(),
            from: from_value,
            to: to_value,
            duration_ms: transition.duration_ms,
            delay_ms: transition.delay_ms,
            timing: transition.timing,
            elapsed_ms: 0.0,
        });
    }

    /// Advance all active transitions by `dt_ms` milliseconds.
    ///
    /// Returns `true` if any transitions are still running after the tick.
    pub fn tick(&mut self, dt_ms: f32) -> bool {
        for t in &mut self.active {
            t.elapsed_ms += dt_ms;
        }
        self.active.retain(|t| !t.is_done());
        !self.active.is_empty()
    }

    /// Get the current interpolated value for a property, if a
    /// transition is in flight.
    pub fn get_value(&self, property: &str) -> Option<f32> {
        self.active
            .iter()
            .find(|t| t.property == property)
            .map(|t| t.current_value())
    }

    /// Returns `true` if any transitions are currently in progress.
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_transition(property: &str, dur_ms: f32) -> Transition {
        Transition {
            property: property.to_string(),
            duration_ms: dur_ms,
            timing: TimingFunction::Linear,
            delay_ms: 0.0,
        }
    }

    #[test]
    fn linear_interpolation() {
        let mut engine = TransitionEngine::new();
        let t = test_transition("opacity", 100.0);
        engine.start_transition("opacity", 0.0, 1.0, &t);

        assert!(engine.has_active());
        engine.tick(50.0);
        let val = engine.get_value("opacity").expect("should have value");
        assert!((val - 0.5).abs() < 0.01, "got {val}");

        engine.tick(50.0);
        assert!(
            !engine.has_active(),
            "transition should be done after full duration"
        );
    }

    #[test]
    fn transition_with_delay() {
        let mut engine = TransitionEngine::new();
        let t = Transition {
            property: "opacity".to_string(),
            duration_ms: 100.0,
            timing: TimingFunction::Linear,
            delay_ms: 50.0,
        };
        engine.start_transition("opacity", 0.0, 1.0, &t);

        engine.tick(25.0);
        let val = engine.get_value("opacity").expect("should have value");
        assert!(
            val.abs() < 0.01,
            "should still be at start during delay, got {val}"
        );

        engine.tick(75.0); // 25 ms of delay left + 50ms into transition
        let val = engine.get_value("opacity").expect("should have value");
        assert!((val - 0.5).abs() < 0.01, "should be halfway, got {val}");
    }

    #[test]
    fn ease_timing_differs_from_linear() {
        // The ease curve should not produce 0.5 at t=0.5.
        let linear = ease(TimingFunction::Linear, 0.5);
        let eased = ease(TimingFunction::Ease, 0.5);
        assert!((linear - 0.5).abs() < 0.01, "linear at 0.5 should be ~0.5");
        assert!(
            (eased - linear).abs() > 0.01,
            "ease should differ from linear at t=0.5"
        );
    }

    #[test]
    fn ease_endpoints() {
        for timing in [
            TimingFunction::Linear,
            TimingFunction::Ease,
            TimingFunction::EaseIn,
            TimingFunction::EaseOut,
            TimingFunction::EaseInOut,
        ] {
            let start = ease(timing, 0.0);
            let end = ease(timing, 1.0);
            assert!(
                start.abs() < 0.01,
                "{timing:?}: start should be ~0, got {start}"
            );
            assert!(
                (end - 1.0).abs() < 0.01,
                "{timing:?}: end should be ~1, got {end}"
            );
        }
    }

    #[test]
    fn replace_existing_transition() {
        let mut engine = TransitionEngine::new();
        let t = test_transition("opacity", 100.0);
        engine.start_transition("opacity", 0.0, 1.0, &t);
        engine.tick(50.0);

        // Start a new transition on the same property.
        engine.start_transition("opacity", 0.5, 0.0, &t);
        let val = engine.get_value("opacity").expect("should have value");
        assert!(
            (val - 0.5).abs() < 0.01,
            "new transition should start at 0.5, got {val}"
        );
    }

    #[test]
    fn tick_returns_false_when_empty() {
        let mut engine = TransitionEngine::new();
        assert!(!engine.tick(16.0));
        assert!(!engine.has_active());
    }

    #[test]
    fn get_value_returns_none_for_unknown() {
        let engine = TransitionEngine::new();
        assert!(engine.get_value("color").is_none());
    }
}
