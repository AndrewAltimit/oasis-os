//! Window animation system for the WM.
//!
//! Provides smooth transitions for window open, close, minimize, maximize,
//! restore, and snap operations. Each animation interpolates geometry and
//! alpha over a configurable duration using pluggable easing functions.
//!
//! The [`AnimationManager`] drives all active animations forward each frame
//! and returns [`AnimationFrame`] snapshots that the renderer uses to
//! position and composite windows during transitions.

use crate::window::Geometry;

// ── Easing functions ────────────────────────────────────────────────

/// Standalone easing functions for window animations.
///
/// Input `t` is clamped to `[0.0, 1.0]`. Output is the eased value.
/// These are self-contained so that `oasis-wm` does not depend on
/// `oasis-ui`.
pub mod easing {
    /// Linear easing (no acceleration).
    pub fn linear(t: f32) -> f32 {
        t.clamp(0.0, 1.0)
    }

    /// Quadratic ease-out (decelerating).
    pub fn ease_out_quad(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * (2.0 - t)
    }

    /// Quadratic ease-in-out (slow start and end).
    pub fn ease_in_out_quad(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t < 0.5 {
            2.0 * t * t
        } else {
            -1.0 + (4.0 - 2.0 * t) * t
        }
    }

    /// Cubic ease-out (decelerating, sharper than quad).
    pub fn ease_out_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let t1 = t - 1.0;
        t1 * t1 * t1 + 1.0
    }

    /// Elastic ease-out (overshoots then settles).
    pub fn ease_out_elastic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 {
            return t;
        }
        let p = 0.3_f32;
        let two_pi_over_p = 2.0 * core::f32::consts::PI / p;
        2.0_f32.powf(-10.0 * t) * ((t - p / 4.0) * two_pi_over_p).sin() + 1.0
    }

    /// Bounce ease-out (bounces multiple times before settling).
    pub fn ease_out_bounce(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t < 1.0 / 2.75 {
            7.5625 * t * t
        } else if t < 2.0 / 2.75 {
            let t = t - 1.5 / 2.75;
            7.5625 * t * t + 0.75
        } else if t < 2.5 / 2.75 {
            let t = t - 2.25 / 2.75;
            7.5625 * t * t + 0.9375
        } else {
            let t = t - 2.625 / 2.75;
            7.5625 * t * t + 0.984375
        }
    }
}

// ── Animation types ─────────────────────────────────────────────────

/// The kind of window transition being animated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationKind {
    /// Window opening (scale-up + fade-in).
    Open,
    /// Window closing (scale-down + fade-out).
    Close,
    /// Shrink to taskbar position.
    Minimize,
    /// Expand from current geometry to full screen.
    Maximize,
    /// Return from maximized to normal geometry.
    Restore,
    /// Snap to a zone geometry (e.g. half-screen tiling).
    SnapTransition,
}

/// Per-animation-kind default durations in milliseconds.
#[derive(Debug, Clone)]
pub struct AnimationDurations {
    /// Duration for window open animation.
    pub open_ms: u32,
    /// Duration for window close animation.
    pub close_ms: u32,
    /// Duration for minimize animation.
    pub minimize_ms: u32,
    /// Duration for maximize animation.
    pub maximize_ms: u32,
    /// Duration for restore animation.
    pub restore_ms: u32,
    /// Duration for snap transition animation.
    pub snap_ms: u32,
}

impl Default for AnimationDurations {
    fn default() -> Self {
        Self {
            open_ms: 200,
            close_ms: 150,
            minimize_ms: 250,
            maximize_ms: 200,
            restore_ms: 200,
            snap_ms: 150,
        }
    }
}

impl AnimationDurations {
    /// Look up the default duration for a given animation kind.
    fn duration_for(&self, kind: AnimationKind) -> u32 {
        match kind {
            AnimationKind::Open => self.open_ms,
            AnimationKind::Close => self.close_ms,
            AnimationKind::Minimize => self.minimize_ms,
            AnimationKind::Maximize => self.maximize_ms,
            AnimationKind::Restore => self.restore_ms,
            AnimationKind::SnapTransition => self.snap_ms,
        }
    }
}

/// The state of a single in-flight animation.
#[derive(Debug)]
pub struct AnimationState {
    /// What kind of transition this is.
    pub kind: AnimationKind,
    /// The window being animated.
    pub window_id: String,
    /// Geometry at animation start.
    pub start_geometry: Geometry,
    /// Target geometry at animation end.
    pub end_geometry: Geometry,
    /// Total animation duration in milliseconds.
    pub duration_ms: u32,
    /// Time elapsed so far in milliseconds.
    pub elapsed_ms: u32,
    /// Easing function applied to the progress value.
    pub easing: fn(f32) -> f32,
    /// Starting opacity (0.0 = transparent, 1.0 = opaque).
    pub start_alpha: f32,
    /// Target opacity.
    pub end_alpha: f32,
}

impl AnimationState {
    /// Compute raw linear progress in `[0.0, 1.0]`.
    fn raw_progress(&self) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        (self.elapsed_ms as f32 / self.duration_ms as f32).min(1.0)
    }

    /// Whether this animation has finished.
    fn is_complete(&self) -> bool {
        self.elapsed_ms >= self.duration_ms
    }

    /// Produce the current interpolated frame.
    fn frame(&self) -> AnimationFrame {
        let t = (self.easing)(self.raw_progress());
        AnimationFrame {
            window_id: self.window_id.clone(),
            x: lerp_i32(self.start_geometry.x, self.end_geometry.x, t),
            y: lerp_i32(self.start_geometry.y, self.end_geometry.y, t),
            width: lerp_u32(self.start_geometry.w, self.end_geometry.w, t),
            height: lerp_u32(self.start_geometry.h, self.end_geometry.h, t),
            alpha: lerp_f32(self.start_alpha, self.end_alpha, t),
            completed: self.is_complete(),
        }
    }
}

/// A snapshot of a window's animated position, size, and opacity
/// for a single frame.
#[derive(Debug, Clone)]
pub struct AnimationFrame {
    /// The window this frame belongs to.
    pub window_id: String,
    /// Current interpolated X position.
    pub x: i32,
    /// Current interpolated Y position.
    pub y: i32,
    /// Current interpolated width.
    pub width: u32,
    /// Current interpolated height.
    pub height: u32,
    /// Current interpolated opacity (0.0 to 1.0).
    pub alpha: f32,
    /// Whether this frame represents the final state
    /// (the animation just completed).
    pub completed: bool,
}

// ── AnimationManager ────────────────────────────────────────────────

/// Drives window animations and produces per-frame snapshots.
///
/// The renderer calls [`tick`](Self::tick) each frame with the elapsed
/// delta and uses the returned [`AnimationFrame`]s to position windows
/// during transitions.
pub struct AnimationManager {
    animations: Vec<AnimationState>,
    reduced_motion: bool,
    default_durations: AnimationDurations,
}

impl AnimationManager {
    /// Create a new animation manager with default settings.
    pub fn new() -> Self {
        Self {
            animations: Vec::new(),
            reduced_motion: false,
            default_durations: AnimationDurations::default(),
        }
    }

    /// Enable or disable reduced-motion mode.
    ///
    /// When enabled, all animations complete instantly (0ms duration)
    /// so that the start animation call returns `false` and no
    /// intermediate frames are produced.
    pub fn set_reduced_motion(&mut self, enabled: bool) {
        self.reduced_motion = enabled;
    }

    /// Whether reduced-motion mode is active.
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// Override the default durations for each animation kind.
    pub fn set_durations(&mut self, durations: AnimationDurations) {
        self.default_durations = durations;
    }

    /// Return a reference to the current duration settings.
    pub fn durations(&self) -> &AnimationDurations {
        &self.default_durations
    }

    /// Start a new animation for the given window.
    ///
    /// If `reduced_motion` is enabled the animation is not queued and
    /// this method returns `false`, signaling the caller to apply the
    /// end state immediately.
    ///
    /// Any existing animation for the same `window_id` is replaced.
    ///
    /// Returns `true` if the animation was started.
    pub fn start_animation(
        &mut self,
        kind: AnimationKind,
        window_id: &str,
        start_geom: Geometry,
        end_geom: Geometry,
    ) -> bool {
        if self.reduced_motion {
            return false;
        }

        // Remove any existing animation for this window.
        self.animations.retain(|a| a.window_id != window_id);

        let duration_ms = self.default_durations.duration_for(kind);
        let (start_alpha, end_alpha) = default_alpha(kind);
        let easing_fn = default_easing(kind);

        self.animations.push(AnimationState {
            kind,
            window_id: window_id.to_string(),
            start_geometry: start_geom,
            end_geometry: end_geom,
            duration_ms,
            elapsed_ms: 0,
            easing: easing_fn,
            start_alpha,
            end_alpha,
        });

        true
    }

    /// Advance all active animations by `delta_ms` and return the
    /// current frame for each.
    ///
    /// Completed animations are removed after producing their final
    /// frame (with `completed == true`).
    pub fn tick(&mut self, delta_ms: u32) -> Vec<AnimationFrame> {
        for anim in &mut self.animations {
            anim.elapsed_ms = anim
                .elapsed_ms
                .saturating_add(delta_ms)
                .min(anim.duration_ms);
        }

        let frames: Vec<AnimationFrame> = self.animations.iter().map(|a| a.frame()).collect();

        // Remove completed animations.
        self.animations.retain(|a| !a.is_complete());

        frames
    }

    /// Check whether a window currently has an active animation.
    pub fn is_animating(&self, window_id: &str) -> bool {
        self.animations.iter().any(|a| a.window_id == window_id)
    }

    /// Cancel any active animation for the given window.
    pub fn cancel(&mut self, window_id: &str) {
        self.animations.retain(|a| a.window_id != window_id);
    }

    /// The number of currently active animations.
    pub fn active_count(&self) -> usize {
        self.animations.len()
    }
}

impl Default for AnimationManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Pick the default easing function for an animation kind.
fn default_easing(kind: AnimationKind) -> fn(f32) -> f32 {
    match kind {
        AnimationKind::Open => easing::ease_out_cubic,
        AnimationKind::Close => easing::ease_out_quad,
        AnimationKind::Minimize => easing::ease_in_out_quad,
        AnimationKind::Maximize => easing::ease_out_cubic,
        AnimationKind::Restore => easing::ease_out_cubic,
        AnimationKind::SnapTransition => easing::ease_out_quad,
    }
}

/// Pick the default start/end alpha for an animation kind.
fn default_alpha(kind: AnimationKind) -> (f32, f32) {
    match kind {
        AnimationKind::Open => (0.0, 1.0),
        AnimationKind::Close => (1.0, 0.0),
        // All others keep full opacity.
        _ => (1.0, 1.0),
    }
}

/// Linearly interpolate between two `i32` values.
fn lerp_i32(a: i32, b: i32, t: f32) -> i32 {
    (a as f32 + (b as f32 - a as f32) * t).round() as i32
}

/// Linearly interpolate between two `u32` values.
fn lerp_u32(a: u32, b: u32, t: f32) -> u32 {
    let v = a as f32 + (b as f32 - a as f32) * t;
    v.round().max(0.0) as u32
}

/// Linearly interpolate between two `f32` values.
fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn geom(x: i32, y: i32, w: u32, h: u32) -> Geometry {
        Geometry { x, y, w, h }
    }

    // ── Easing function tests ───────────────────────────────────

    #[test]
    fn easing_linear_boundaries() {
        assert!((easing::linear(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((easing::linear(0.5) - 0.5).abs() < f32::EPSILON);
        assert!((easing::linear(1.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn easing_linear_clamps() {
        assert!((easing::linear(-0.5) - 0.0).abs() < f32::EPSILON);
        assert!((easing::linear(1.5) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn easing_ease_out_quad_boundaries() {
        assert!((easing::ease_out_quad(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((easing::ease_out_quad(1.0) - 1.0).abs() < f32::EPSILON);
        // At t=0.5: 0.5 * (2.0 - 0.5) = 0.75
        assert!((easing::ease_out_quad(0.5) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn easing_ease_in_out_quad_boundaries() {
        assert!((easing::ease_in_out_quad(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((easing::ease_in_out_quad(1.0) - 1.0).abs() < f32::EPSILON);
        // At t=0.5: boundary between two branches, both give 0.5
        assert!((easing::ease_in_out_quad(0.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn easing_ease_out_cubic_boundaries() {
        assert!((easing::ease_out_cubic(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((easing::ease_out_cubic(1.0) - 1.0).abs() < f32::EPSILON);
        // At t=0.5: (0.5-1)^3 + 1 = -0.125 + 1 = 0.875
        assert!((easing::ease_out_cubic(0.5) - 0.875).abs() < 1e-5);
    }

    #[test]
    fn easing_ease_out_elastic_boundaries() {
        assert!((easing::ease_out_elastic(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((easing::ease_out_elastic(1.0) - 1.0).abs() < f32::EPSILON);
        // Mid-value should be near 1.0 (overshoots slightly)
        let mid = easing::ease_out_elastic(0.5);
        assert!(mid > 0.9 && mid < 1.1);
    }

    #[test]
    fn easing_ease_out_bounce_boundaries() {
        assert!((easing::ease_out_bounce(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((easing::ease_out_bounce(1.0) - 1.0).abs() < 1e-5);
        // Mid-value should be in valid range
        let mid = easing::ease_out_bounce(0.5);
        assert!(mid > 0.0 && mid <= 1.0);
    }

    #[test]
    fn easing_monotonic_out_quad() {
        // ease_out_quad should be monotonically increasing
        let mut prev = 0.0_f32;
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let v = easing::ease_out_quad(t);
            assert!(v >= prev, "not monotonic at t={t}");
            prev = v;
        }
    }

    // ── Animation lifecycle tests ───────────────────────────────

    #[test]
    fn animation_start_and_complete() {
        let mut mgr = AnimationManager::new();
        let started = mgr.start_animation(
            AnimationKind::Open,
            "win1",
            geom(100, 100, 200, 150),
            geom(100, 100, 200, 150),
        );
        assert!(started);
        assert!(mgr.is_animating("win1"));
        assert_eq!(mgr.active_count(), 1);

        // Tick past the full duration (open default = 200ms).
        let frames = mgr.tick(300);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].completed);

        // Animation should now be removed.
        assert!(!mgr.is_animating("win1"));
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn animation_partial_progress() {
        let mut mgr = AnimationManager::new();
        // Use linear easing for predictable values.
        mgr.start_animation(
            AnimationKind::SnapTransition,
            "win1",
            geom(0, 0, 100, 100),
            geom(100, 50, 200, 200),
        );

        // Override the easing to linear for this test.
        if let Some(anim) = mgr.animations.first_mut() {
            anim.easing = easing::linear;
            anim.duration_ms = 100;
        }

        let frames = mgr.tick(50);
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert!(!f.completed);
        // At 50% linear: x=50, y=25, w=150, h=150
        assert_eq!(f.x, 50);
        assert_eq!(f.y, 25);
        assert_eq!(f.width, 150);
        assert_eq!(f.height, 150);
    }

    #[test]
    fn multiple_simultaneous_animations() {
        let mut mgr = AnimationManager::new();
        mgr.start_animation(
            AnimationKind::Open,
            "win_a",
            geom(0, 0, 100, 100),
            geom(0, 0, 200, 200),
        );
        mgr.start_animation(
            AnimationKind::Close,
            "win_b",
            geom(50, 50, 300, 200),
            geom(50, 50, 0, 0),
        );
        assert_eq!(mgr.active_count(), 2);
        assert!(mgr.is_animating("win_a"));
        assert!(mgr.is_animating("win_b"));

        let frames = mgr.tick(10);
        assert_eq!(frames.len(), 2);

        let ids: Vec<&str> = frames.iter().map(|f| f.window_id.as_str()).collect();
        assert!(ids.contains(&"win_a"));
        assert!(ids.contains(&"win_b"));
    }

    #[test]
    fn reduced_motion_prevents_animation() {
        let mut mgr = AnimationManager::new();
        mgr.set_reduced_motion(true);
        assert!(mgr.reduced_motion());

        let started = mgr.start_animation(
            AnimationKind::Open,
            "win1",
            geom(0, 0, 100, 100),
            geom(0, 0, 200, 200),
        );
        assert!(!started);
        assert!(!mgr.is_animating("win1"));
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn cancel_animation() {
        let mut mgr = AnimationManager::new();
        mgr.start_animation(
            AnimationKind::Maximize,
            "win1",
            geom(50, 50, 200, 150),
            geom(0, 0, 800, 600),
        );
        assert!(mgr.is_animating("win1"));

        mgr.cancel("win1");
        assert!(!mgr.is_animating("win1"));
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn cancel_nonexistent_is_noop() {
        let mut mgr = AnimationManager::new();
        mgr.cancel("ghost_window");
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn alpha_interpolation_open() {
        let mut mgr = AnimationManager::new();
        mgr.start_animation(
            AnimationKind::Open,
            "win1",
            geom(0, 0, 100, 100),
            geom(0, 0, 100, 100),
        );

        // Override to linear for predictable alpha.
        if let Some(anim) = mgr.animations.first_mut() {
            anim.easing = easing::linear;
            anim.duration_ms = 100;
        }

        let frames = mgr.tick(50);
        let f = &frames[0];
        // Open: start_alpha=0.0, end_alpha=1.0, at t=0.5 -> 0.5
        assert!((f.alpha - 0.5).abs() < 1e-5);
    }

    #[test]
    fn alpha_interpolation_close() {
        let mut mgr = AnimationManager::new();
        mgr.start_animation(
            AnimationKind::Close,
            "win1",
            geom(0, 0, 100, 100),
            geom(0, 0, 100, 100),
        );

        if let Some(anim) = mgr.animations.first_mut() {
            anim.easing = easing::linear;
            anim.duration_ms = 100;
        }

        let frames = mgr.tick(50);
        let f = &frames[0];
        // Close: start_alpha=1.0, end_alpha=0.0, at t=0.5 -> 0.5
        assert!((f.alpha - 0.5).abs() < 1e-5);
    }

    #[test]
    fn alpha_stays_opaque_for_maximize() {
        let mut mgr = AnimationManager::new();
        mgr.start_animation(
            AnimationKind::Maximize,
            "win1",
            geom(50, 50, 200, 150),
            geom(0, 0, 800, 600),
        );

        if let Some(anim) = mgr.animations.first_mut() {
            anim.easing = easing::linear;
        }

        let frames = mgr.tick(100);
        let f = &frames[0];
        assert!((f.alpha - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn geometry_interpolation_accuracy() {
        let mut mgr = AnimationManager::new();
        mgr.start_animation(
            AnimationKind::SnapTransition,
            "win1",
            geom(0, 0, 400, 300),
            geom(400, 0, 400, 600),
        );

        if let Some(anim) = mgr.animations.first_mut() {
            anim.easing = easing::linear;
            anim.duration_ms = 100;
        }

        // At 25%.
        let frames = mgr.tick(25);
        let f = &frames[0];
        assert_eq!(f.x, 100);
        assert_eq!(f.y, 0);
        assert_eq!(f.width, 400);
        assert_eq!(f.height, 375);
    }

    #[test]
    fn duration_override() {
        let mut mgr = AnimationManager::new();
        mgr.set_durations(AnimationDurations {
            open_ms: 500,
            close_ms: 300,
            minimize_ms: 400,
            maximize_ms: 350,
            restore_ms: 350,
            snap_ms: 200,
        });

        mgr.start_animation(
            AnimationKind::Open,
            "win1",
            geom(0, 0, 100, 100),
            geom(0, 0, 100, 100),
        );

        // The open animation should now have 500ms duration.
        assert_eq!(
            mgr.animations[0].duration_ms, 500,
            "expected overridden duration"
        );

        // After 499ms it should still be running.
        let frames = mgr.tick(499);
        assert_eq!(frames.len(), 1);
        assert!(!frames[0].completed);
        assert!(mgr.is_animating("win1"));
    }

    #[test]
    fn animation_durations_default_values() {
        let d = AnimationDurations::default();
        assert_eq!(d.open_ms, 200);
        assert_eq!(d.close_ms, 150);
        assert_eq!(d.minimize_ms, 250);
        assert_eq!(d.maximize_ms, 200);
        assert_eq!(d.restore_ms, 200);
        assert_eq!(d.snap_ms, 150);
    }

    #[test]
    fn replacing_animation_for_same_window() {
        let mut mgr = AnimationManager::new();
        mgr.start_animation(
            AnimationKind::Maximize,
            "win1",
            geom(0, 0, 200, 150),
            geom(0, 0, 800, 600),
        );
        assert_eq!(mgr.active_count(), 1);

        // Start a new animation for the same window.
        mgr.start_animation(
            AnimationKind::Restore,
            "win1",
            geom(0, 0, 800, 600),
            geom(50, 50, 200, 150),
        );
        // Should still be 1, not 2.
        assert_eq!(mgr.active_count(), 1);
        assert_eq!(mgr.animations[0].kind, AnimationKind::Restore);
    }

    #[test]
    fn zero_duration_completes_instantly() {
        let mut mgr = AnimationManager::new();
        mgr.set_durations(AnimationDurations {
            open_ms: 0,
            close_ms: 0,
            minimize_ms: 0,
            maximize_ms: 0,
            restore_ms: 0,
            snap_ms: 0,
        });

        mgr.start_animation(
            AnimationKind::Open,
            "win1",
            geom(0, 0, 100, 100),
            geom(0, 0, 200, 200),
        );

        let frames = mgr.tick(0);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].completed);
        // Should reach end geometry.
        assert_eq!(frames[0].width, 200);
        assert_eq!(frames[0].height, 200);
        assert!(!mgr.is_animating("win1"));
    }

    #[test]
    fn tick_with_no_animations_returns_empty() {
        let mut mgr = AnimationManager::new();
        let frames = mgr.tick(16);
        assert!(frames.is_empty());
    }

    #[test]
    fn default_easing_assignments() {
        // Verify each kind maps to the expected easing function by
        // comparing output at a known input against the expected
        // function's output. This avoids function pointer comparison
        // warnings.
        let t = 0.37;
        assert!(
            (default_easing(AnimationKind::Open)(t) - easing::ease_out_cubic(t)).abs()
                < f32::EPSILON
        );
        assert!(
            (default_easing(AnimationKind::Close)(t) - easing::ease_out_quad(t)).abs()
                < f32::EPSILON
        );
        assert!(
            (default_easing(AnimationKind::Minimize)(t) - easing::ease_in_out_quad(t)).abs()
                < f32::EPSILON
        );
        assert!(
            (default_easing(AnimationKind::SnapTransition)(t) - easing::ease_out_quad(t)).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn lerp_helpers_correctness() {
        assert_eq!(lerp_i32(0, 100, 0.0), 0);
        assert_eq!(lerp_i32(0, 100, 0.5), 50);
        assert_eq!(lerp_i32(0, 100, 1.0), 100);
        assert_eq!(lerp_i32(-50, 50, 0.5), 0);

        assert_eq!(lerp_u32(0, 200, 0.0), 0);
        assert_eq!(lerp_u32(0, 200, 0.5), 100);
        assert_eq!(lerp_u32(0, 200, 1.0), 200);

        assert!((lerp_f32(0.0, 1.0, 0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn animation_manager_default_trait() {
        let mgr = AnimationManager::default();
        assert_eq!(mgr.active_count(), 0);
        assert!(!mgr.reduced_motion());
    }
}
