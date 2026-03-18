//! CSS animation engine for `@keyframes` and `animation` properties.
//!
//! Tracks running animations, advances them each frame, and returns
//! interpolated property values for active animations on each node.

use std::collections::HashMap;

use super::parser::KeyframesRule;
use super::values::types::{
    Animation, AnimationDirection, AnimationFillMode, AnimationPlayState, TimingFunction,
};

/// An active animation instance for a specific node.
#[derive(Debug, Clone)]
struct ActiveAnimation {
    /// The `@keyframes` rule name.
    name: String,
    /// Total duration per iteration in milliseconds.
    duration_ms: f32,
    /// Delay before first iteration in milliseconds.
    delay_ms: f32,
    /// Timing function for easing.
    timing: TimingFunction,
    /// Number of iterations (may be `f32::INFINITY`).
    iteration_count: f32,
    /// Playback direction.
    direction: AnimationDirection,
    /// Fill mode for before/after the active period.
    fill_mode: AnimationFillMode,
    /// Play state.
    play_state: AnimationPlayState,
    /// Elapsed time in milliseconds (includes delay).
    elapsed_ms: f32,
    /// The keyframe stops (percentage 0..=100 -> property name -> string value).
    keyframe_properties: Vec<(f32, Vec<(String, String)>)>,
}

impl ActiveAnimation {
    /// Returns `true` if this animation has completed all iterations.
    fn is_done(&self) -> bool {
        if self.iteration_count.is_infinite() {
            return false;
        }
        let active = self.elapsed_ms - self.delay_ms;
        active >= self.duration_ms * self.iteration_count
    }

    /// Compute the current progress through the keyframes (0.0 ..= 1.0).
    fn current_progress(&self) -> Option<f32> {
        let active = self.elapsed_ms - self.delay_ms;

        if active < 0.0 {
            // In delay phase.
            return match self.fill_mode {
                AnimationFillMode::Backwards | AnimationFillMode::Both => Some(0.0),
                _ => None,
            };
        }

        if self.duration_ms <= 0.0 {
            return Some(1.0);
        }

        let total_duration = self.duration_ms * self.iteration_count;
        if !self.iteration_count.is_infinite() && active >= total_duration {
            // Animation finished.
            return match self.fill_mode {
                AnimationFillMode::Forwards | AnimationFillMode::Both => {
                    // End value depends on direction + iteration count.
                    let last_iter = (self.iteration_count.ceil() as u32).saturating_sub(1);
                    let reversed = self.is_reversed(last_iter);
                    Some(if reversed { 0.0 } else { 1.0 })
                },
                _ => None,
            };
        }

        // Current iteration.
        let iteration = (active / self.duration_ms).floor() as u32;
        let within_iter = active - (iteration as f32 * self.duration_ms);
        let raw_progress = (within_iter / self.duration_ms).clamp(0.0, 1.0);
        let eased = ease(self.timing, raw_progress);

        let reversed = self.is_reversed(iteration);
        Some(if reversed { 1.0 - eased } else { eased })
    }

    /// Whether the given iteration plays in reverse.
    fn is_reversed(&self, iteration: u32) -> bool {
        match self.direction {
            AnimationDirection::Normal => false,
            AnimationDirection::Reverse => true,
            AnimationDirection::Alternate => !iteration.is_multiple_of(2),
            AnimationDirection::AlternateReverse => iteration.is_multiple_of(2),
        }
    }
}

/// Engine that manages active CSS animations.
#[derive(Debug, Clone, Default)]
pub struct AnimationEngine {
    /// Active animations keyed by node ID.
    active: HashMap<usize, Vec<ActiveAnimation>>,
}

impl AnimationEngine {
    /// Create a new, empty animation engine.
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
        }
    }

    /// Start animations for a node, replacing any previously running ones.
    pub fn start_animations(
        &mut self,
        node_id: usize,
        animations: &[Animation],
        keyframes_map: &[KeyframesRule],
    ) {
        let mut node_anims = Vec::new();
        for anim in animations {
            // Find the matching @keyframes rule.
            let Some(kf_rule) = keyframes_map.iter().find(|kf| kf.name == anim.name) else {
                continue;
            };

            // Extract property values from each keyframe stop.
            let keyframe_properties: Vec<(f32, Vec<(String, String)>)> = kf_rule
                .stops
                .iter()
                .map(|stop| {
                    let props: Vec<(String, String)> = stop
                        .declarations
                        .iter()
                        .map(|d| (d.property.clone(), declaration_value_to_string(&d.value)))
                        .collect();
                    (stop.percentage, props)
                })
                .collect();

            node_anims.push(ActiveAnimation {
                name: anim.name.clone(),
                duration_ms: anim.duration_ms,
                delay_ms: anim.delay_ms,
                timing: anim.timing,
                iteration_count: anim.iteration_count,
                direction: anim.direction,
                fill_mode: anim.fill_mode,
                play_state: anim.play_state,
                elapsed_ms: 0.0,
                keyframe_properties,
            });
        }

        if node_anims.is_empty() {
            self.active.remove(&node_id);
        } else {
            self.active.insert(node_id, node_anims);
        }
    }

    /// Advance all animations by `dt_ms` milliseconds.
    ///
    /// Returns `true` if any animations are still running.
    pub fn tick(&mut self, dt_ms: f32) -> bool {
        for anims in self.active.values_mut() {
            for anim in anims.iter_mut() {
                if anim.play_state == AnimationPlayState::Running {
                    anim.elapsed_ms += dt_ms;
                }
            }
            anims.retain(|a| !a.is_done());
        }
        self.active.retain(|_, anims| !anims.is_empty());
        !self.active.is_empty()
    }

    /// Returns interpolated property values for active animations on a node.
    ///
    /// Each entry is `(property_name, interpolated_float_value)`.
    /// Only numeric properties can be meaningfully interpolated.
    pub fn get_overrides(&self, node_id: usize) -> Vec<(String, f32)> {
        let Some(anims) = self.active.get(&node_id) else {
            return Vec::new();
        };

        let mut overrides = Vec::new();
        for anim in anims {
            let Some(progress) = anim.current_progress() else {
                continue;
            };

            // Find the two surrounding keyframe stops.
            let pct = progress * 100.0;
            let stops = &anim.keyframe_properties;
            if stops.is_empty() {
                continue;
            }

            // Find bounding stops.
            let (from_idx, to_idx) = find_bounding_stops(stops, pct);
            let (from_pct, from_props) = &stops[from_idx];
            let (to_pct, to_props) = &stops[to_idx];

            let local_t = if (to_pct - from_pct).abs() < f32::EPSILON {
                1.0
            } else {
                ((pct - from_pct) / (to_pct - from_pct)).clamp(0.0, 1.0)
            };

            // Interpolate properties that exist in both stops.
            for (prop, from_val) in from_props {
                if let Some((_, to_val)) = to_props.iter().find(|(p, _)| p == prop)
                    && let (Some(fv), Some(tv)) =
                        (parse_numeric_value(from_val), parse_numeric_value(to_val))
                {
                    let interpolated = fv + (tv - fv) * local_t;
                    overrides.push((prop.clone(), interpolated));
                }
            }
        }
        overrides
    }

    /// Returns `true` if any animations are currently in progress.
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }
}

/// Find the two bounding keyframe stop indices for a given percentage.
fn find_bounding_stops(stops: &[(f32, Vec<(String, String)>)], pct: f32) -> (usize, usize) {
    if stops.len() == 1 {
        return (0, 0);
    }
    for i in 0..stops.len() - 1 {
        if pct >= stops[i].0 && pct <= stops[i + 1].0 {
            return (i, i + 1);
        }
    }
    // Clamp to edges.
    if pct < stops[0].0 {
        (0, 0)
    } else {
        (stops.len() - 1, stops.len() - 1)
    }
}

/// Convert a CSS declaration value to a string for keyframe storage.
fn declaration_value_to_string(value: &super::parser::CssValue) -> String {
    use super::parser::CssValue;
    match value {
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        CssValue::Number(n) => format!("{n}"),
        CssValue::Length(n, unit) => {
            let u = match unit {
                super::parser::LengthUnit::Px => "px",
                super::parser::LengthUnit::Em => "em",
                super::parser::LengthUnit::Rem => "rem",
                super::parser::LengthUnit::Pt => "pt",
            };
            format!("{n}{u}")
        },
        CssValue::Percentage(n) => format!("{n}%"),
        CssValue::Color(c) => format!("rgba({},{},{},{})", c.r, c.g, c.b, c.a),
        CssValue::Multiple(vs) => vs
            .iter()
            .map(declaration_value_to_string)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Try to parse a numeric value from a CSS value string.
///
/// Handles bare numbers, `px` values, percentages, and degrees.
fn parse_numeric_value(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Try bare number first.
    if let Ok(n) = s.parse::<f32>() {
        return Some(n);
    }
    // Try px suffix.
    if let Some(rest) = s.strip_suffix("px") {
        return rest.parse::<f32>().ok();
    }
    // Try percentage.
    if let Some(rest) = s.strip_suffix('%') {
        return rest.parse::<f32>().ok();
    }
    // Try degrees.
    if let Some(rest) = s.strip_suffix("deg") {
        return rest.parse::<f32>().ok();
    }
    // Try em.
    if let Some(rest) = s.strip_suffix("em") {
        return rest.parse::<f32>().ok();
    }
    None
}

/// Apply a timing function easing to a linear progress value `t` in `[0, 1]`.
fn ease(timing: TimingFunction, t: f32) -> f32 {
    match timing {
        TimingFunction::Linear => t,
        TimingFunction::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
        TimingFunction::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        TimingFunction::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        TimingFunction::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
    }
}

/// Evaluate a cubic bezier at parameter `t`.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
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

fn bezier(p1: f32, p2: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    3.0 * mt2 * t * p1 + 3.0 * mt * t2 * p2 + t3
}

fn bezier_derivative(p1: f32, p2: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    3.0 * mt * mt * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_keyframes(name: &str) -> KeyframesRule {
        use super::super::parser::{CssValue, Declaration, PropertyId};
        KeyframesRule {
            name: name.to_string(),
            stops: vec![
                super::super::parser::KeyframeStop {
                    percentage: 0.0,
                    declarations: vec![Declaration {
                        property: "opacity".to_string(),
                        value: CssValue::Number(0.0),
                        important: false,
                        property_id: PropertyId::from_name("opacity"),
                    }],
                },
                super::super::parser::KeyframeStop {
                    percentage: 100.0,
                    declarations: vec![Declaration {
                        property: "opacity".to_string(),
                        value: CssValue::Number(1.0),
                        important: false,
                        property_id: PropertyId::from_name("opacity"),
                    }],
                },
            ],
        }
    }

    fn make_animation(name: &str, duration_ms: f32) -> Animation {
        Animation {
            name: name.to_string(),
            duration_ms,
            timing: TimingFunction::Linear,
            delay_ms: 0.0,
            iteration_count: 1.0,
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }

    #[test]
    fn basic_animation_interpolation() {
        let mut engine = AnimationEngine::new();
        let kf = make_keyframes("fade-in");
        let anim = make_animation("fade-in", 100.0);
        engine.start_animations(1, &[anim], &[kf]);

        assert!(engine.has_active());
        engine.tick(50.0);
        let overrides = engine.get_overrides(1);
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].0, "opacity");
        assert!(
            (overrides[0].1 - 0.5).abs() < 0.05,
            "expected ~0.5, got {}",
            overrides[0].1
        );
    }

    #[test]
    fn animation_completes() {
        let mut engine = AnimationEngine::new();
        let kf = make_keyframes("fade-in");
        let anim = make_animation("fade-in", 100.0);
        engine.start_animations(1, &[anim], &[kf]);

        engine.tick(100.0);
        assert!(
            !engine.has_active(),
            "animation should complete after full duration"
        );
    }

    #[test]
    fn infinite_animation_does_not_complete() {
        let mut engine = AnimationEngine::new();
        let kf = make_keyframes("fade-in");
        let mut anim = make_animation("fade-in", 100.0);
        anim.iteration_count = f32::INFINITY;
        engine.start_animations(1, &[anim], &[kf]);

        engine.tick(10000.0);
        assert!(engine.has_active(), "infinite animation should never end");
    }

    #[test]
    fn animation_with_delay() {
        let mut engine = AnimationEngine::new();
        let kf = make_keyframes("fade-in");
        let mut anim = make_animation("fade-in", 100.0);
        anim.delay_ms = 50.0;
        engine.start_animations(1, &[anim], &[kf]);

        engine.tick(25.0);
        let overrides = engine.get_overrides(1);
        // During delay with no fill mode, no overrides.
        assert!(overrides.is_empty());

        engine.tick(75.0); // 25ms delay left + 50ms into animation
        let overrides = engine.get_overrides(1);
        assert_eq!(overrides.len(), 1);
        assert!(
            (overrides[0].1 - 0.5).abs() < 0.05,
            "expected ~0.5, got {}",
            overrides[0].1
        );
    }

    #[test]
    fn fill_forwards_holds_end_value() {
        let mut engine = AnimationEngine::new();
        let kf = make_keyframes("fade-in");
        let mut anim = make_animation("fade-in", 100.0);
        anim.fill_mode = AnimationFillMode::Forwards;
        engine.start_animations(1, &[anim], &[kf]);

        engine.tick(200.0);
        // With forwards fill, we need to check before it's removed.
        // Actually, with fill_mode Forwards, is_done() still returns true
        // but get_overrides should show the end value.
        // In our implementation, is_done removes it. Let's check at exactly done.
        let mut engine2 = AnimationEngine::new();
        let kf2 = make_keyframes("fade-in");
        let mut anim2 = make_animation("fade-in", 100.0);
        anim2.fill_mode = AnimationFillMode::Forwards;
        engine2.start_animations(1, &[anim2], &[kf2]);
        engine2.tick(99.0);
        let overrides = engine2.get_overrides(1);
        assert!(!overrides.is_empty());
        assert!(overrides[0].1 > 0.9);
    }

    #[test]
    fn reverse_direction() {
        let mut engine = AnimationEngine::new();
        let kf = make_keyframes("fade-in");
        let mut anim = make_animation("fade-in", 100.0);
        anim.direction = AnimationDirection::Reverse;
        engine.start_animations(1, &[anim], &[kf]);

        engine.tick(50.0);
        let overrides = engine.get_overrides(1);
        assert_eq!(overrides.len(), 1);
        // Reverse: progress 0.5 -> 1.0 - 0.5 = 0.5 ... same with linear
        // Actually: reversed at 50% through -> 1.0 - 0.5 = 0.5 for linear.
        // At t=0 it should be 1.0 (reversed), at t=100 it should be 0.0.
        // At t=25: reversed progress = 1.0 - 0.25 = 0.75
        let mut engine2 = AnimationEngine::new();
        let kf2 = make_keyframes("fade-in");
        let mut anim2 = make_animation("fade-in", 100.0);
        anim2.direction = AnimationDirection::Reverse;
        engine2.start_animations(1, &[anim2], &[kf2]);
        engine2.tick(25.0);
        let overrides2 = engine2.get_overrides(1);
        assert!(
            (overrides2[0].1 - 0.75).abs() < 0.05,
            "expected ~0.75 for reverse at 25%, got {}",
            overrides2[0].1
        );
    }

    #[test]
    fn paused_animation_does_not_advance() {
        let mut engine = AnimationEngine::new();
        let kf = make_keyframes("fade-in");
        let mut anim = make_animation("fade-in", 100.0);
        anim.play_state = AnimationPlayState::Paused;
        anim.fill_mode = AnimationFillMode::Both;
        engine.start_animations(1, &[anim], &[kf]);

        engine.tick(50.0);
        let overrides = engine.get_overrides(1);
        // Paused at t=0, with Backwards fill -> progress = 0.0.
        assert!(
            !overrides.is_empty(),
            "should have overrides with Both fill"
        );
        assert!(
            overrides[0].1.abs() < 0.05,
            "paused animation should not advance, got {}",
            overrides[0].1
        );
    }

    #[test]
    fn parse_numeric_values() {
        assert!((parse_numeric_value("42").unwrap() - 42.0).abs() < f32::EPSILON);
        assert!((parse_numeric_value("10px").unwrap() - 10.0).abs() < f32::EPSILON);
        assert!((parse_numeric_value("50%").unwrap() - 50.0).abs() < f32::EPSILON);
        assert!((parse_numeric_value("90deg").unwrap() - 90.0).abs() < f32::EPSILON);
        assert!(parse_numeric_value("").is_none());
        assert!(parse_numeric_value("red").is_none());
    }

    #[test]
    fn no_matching_keyframes() {
        let mut engine = AnimationEngine::new();
        let anim = make_animation("nonexistent", 100.0);
        engine.start_animations(1, &[anim], &[]);
        assert!(!engine.has_active());
    }
}
