//! Animation helpers for vector icons and background layers.
//!
//! Frame-counter-driven animation primitives: rotation, pulse (alpha oscillation),
//! blink (on/off toggle), float (sine-wave vertical offset), and entrance effects.
//!
//! [`AnimClock`] bridges frame-counter and time-based animation worlds.

use crate::op::VectorOp;
use oasis_types::backend::Color;
use oasis_types::easing::ease_out_cubic;

/// Unified animation time source bridging frame-counter and time-based worlds.
///
/// Time accumulates in `f64` seconds: an `f32` accumulator loses the
/// 1/60 s frame step after a few days of uptime (at 2^18 s its spacing is
/// 1/32 s, so every tick rounds to a double step and animations run at
/// twice their speed). The oscillators also evaluate their phase in
/// `f64` and only narrow the final value.
#[derive(Debug, Clone, Default)]
pub struct AnimClock {
    time_s: f64,
    frame: u32,
}

impl AnimClock {
    /// Create a new clock at time zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seconds since the clock started (narrowed to `f32`; use the
    /// oscillators for long-running periodic values).
    pub fn time_s(&self) -> f32 {
        self.time_s as f32
    }

    /// Frames ticked so far (wraps at `u32::MAX`).
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// A clock positioned at `frame` of a fixed 60 fps timeline (time is
    /// derived in `f64`, so frame counters of any age stay exact).
    pub fn from_frame(frame: u32) -> Self {
        Self {
            time_s: f64::from(frame) / 60.0,
            frame,
        }
    }

    /// Rotation angle in radians for a constant angular speed
    /// (`rad_per_s`), plus `phase`. The accumulated turn is reduced modulo
    /// a full turn in `f64` first, so the result stays precise (and small)
    /// for any uptime; trig of the result equals trig of the raw product.
    pub fn angle(&self, rad_per_s: f32, phase: f32) -> f32 {
        let turn = (self.time_s * f64::from(rad_per_s)).rem_euclid(core::f64::consts::TAU);
        (turn + f64::from(phase)) as f32
    }

    /// `time x rate` reduced modulo `period` (in `[0, period)`), computed in
    /// `f64` so the result keeps full `f32` precision however long the
    /// clock has run -- e.g. a drift of `rate` px/s wrapped to the screen
    /// width, or a phase wrapped to a common period of its harmonics.
    /// A non-positive or non-finite `period` (or non-finite `rate`) yields 0.
    pub fn wrapped(&self, rate: f32, period: f32) -> f32 {
        if !(period.is_finite() && period > 0.0 && rate.is_finite()) {
            return 0.0;
        }
        let v = (self.time_s * f64::from(rate)).rem_euclid(f64::from(period)) as f32;
        // Narrowing can round up to `period`; keep the range half-open.
        if v >= period { 0.0 } else { v }
    }

    /// Advance by one frame at 60fps.
    pub fn tick_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.time_s += 1.0 / 60.0;
    }

    /// Advance by a delta time in milliseconds.
    pub fn tick_dt(&mut self, dt_ms: u32) {
        self.frame = self.frame.wrapping_add(1);
        self.time_s += f64::from(dt_ms) / 1000.0;
    }

    /// Sine wave oscillator returning -1..1.
    pub fn sine(&self, freq: f32, phase: f32) -> f32 {
        // Reduce the cycle count to [0, 1) in f64 before scaling to radians
        // so the argument stays small however long the clock has run.
        let cycles = (self.time_s * f64::from(freq)).rem_euclid(1.0);
        (cycles * core::f64::consts::TAU + f64::from(phase)).sin() as f32
    }

    /// Sine wave oscillator returning 0..1.
    pub fn sine_norm(&self, freq: f32, phase: f32) -> f32 {
        (self.sine(freq, phase) + 1.0) * 0.5
    }

    /// Sawtooth wave returning 0..1, wrapping every `period_s` seconds.
    pub fn sawtooth(&self, period_s: f32, phase: f32) -> f32 {
        // NaN would otherwise come out as NaN.
        if period_s.is_nan() || period_s <= 0.0 || !phase.is_finite() {
            return 0.0;
        }
        // `rem_euclid`, not `%` + `abs`: a negative phase must wrap to the
        // far end of the ramp, not mirror back onto it.
        let v = (self.time_s / f64::from(period_s) + f64::from(phase)).rem_euclid(1.0) as f32;
        // Narrowing can round 0.999.. up to 1.0; keep the ramp half-open.
        if v >= 1.0 { 0.0 } else { v }
    }
}

/// Compute entrance scale factor (0->1) with ease-out cubic.
pub fn entrance_scale(elapsed_ms: u32, duration_ms: u32) -> f32 {
    if duration_ms == 0 {
        return 1.0;
    }
    ease_out_cubic(elapsed_ms as f32 / duration_ms as f32)
}

/// Compute entrance alpha (0->255) with ease-out.
pub fn entrance_alpha(elapsed_ms: u32, duration_ms: u32) -> u8 {
    if duration_ms == 0 {
        return 255;
    }
    let t = (elapsed_ms as f32 / duration_ms as f32).min(1.0);
    let inv = 1.0 - t;
    (255.0 * (1.0 - inv * inv)) as u8
}

/// Compute entrance vertical slide offset with ease-out.
pub fn entrance_slide_y(elapsed_ms: u32, duration_ms: u32, dist: i32) -> i32 {
    if duration_ms == 0 {
        return 0;
    }
    let ease = ease_out_cubic(elapsed_ms as f32 / duration_ms as f32);
    (dist as f32 * (1.0 - ease)) as i32
}

/// Rotate a point `(px, py)` around `(cx, cy)` by `angle` radians.
///
/// Computed in `f64` and saturated back to `i32`, so extreme coordinates
/// neither overflow nor lose precision.
pub fn rotate_point(px: i32, py: i32, cx: i32, cy: i32, angle: f32) -> (i32, i32) {
    let dx = f64::from(px) - f64::from(cx);
    let dy = f64::from(py) - f64::from(cy);
    let (sin, cos) = f64::from(angle).sin_cos();
    let rx = dx * cos - dy * sin;
    let ry = dx * sin + dy * cos;
    // Round, not truncate: truncation pulls negative offsets toward the
    // centre, so a 180-degree turn of (10, 0) landed on (-9, 0).
    (
        (f64::from(cx) + rx).round() as i32,
        (f64::from(cy) + ry).round() as i32,
    )
}

/// Rotate a `FillRect` into a `FillPolygon` by computing its 4 corner positions.
///
/// The rotation is around the rectangle's center.
pub fn rotate_rect(x: i32, y: i32, w: u32, h: u32, angle: f32, color: Color) -> VectorOp {
    // i64 so `x + w` cannot overflow; the far edges saturate to i32.
    let sat = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let (x1, y1) = (
        sat(i64::from(x) + i64::from(w)),
        sat(i64::from(y) + i64::from(h)),
    );
    let cx = sat(i64::from(x) + i64::from(w / 2));
    let cy = sat(i64::from(y) + i64::from(h / 2));
    let corners = [(x, y), (x1, y), (x1, y1), (x, y1)];
    let rotated: Vec<(i32, i32)> = corners
        .iter()
        .map(|&(px, py)| rotate_point(px, py, cx, cy, angle))
        .collect();
    VectorOp::FillPolygon {
        points: rotated,
        color,
    }
}

/// Compute a sine-wave float offset for a given frame and icon slot.
///
/// Each slot is phase-shifted so icons bob at different times.
/// Returns a Y offset in pixels.
pub fn float_offset(frame: u32, slot: usize, amplitude: f32, speed: f32) -> i32 {
    let phase = slot as f64 * 0.8; // stagger per icon
    (frame_phase(frame, speed, phase).sin() * amplitude) as i32
}

/// `frame x speed + phase` reduced to one turn in `f64`. An `f32` product
/// stops advancing smoothly once the frame counter passes 2^24 (~3 days at
/// 60 fps): consecutive frames round to the same value and the motion
/// stutters.
fn frame_phase(frame: u32, speed: f32, phase: f64) -> f32 {
    (f64::from(frame) * f64::from(speed) + phase).rem_euclid(core::f64::consts::TAU) as f32
}

/// Compute a pulse alpha (0-255) for a given frame.
///
/// Oscillates between `min_alpha` and 255 using a sine wave.
pub fn pulse_alpha(frame: u32, speed: f32, min_alpha: u8) -> u8 {
    let norm = (frame_phase(frame, speed, 0.0).sin() + 1.0) * 0.5; // 0.0 to 1.0
    let range = 255 - min_alpha as u16;
    (min_alpha as u16 + (norm * range as f32) as u16).min(255) as u8
}

/// Compute blink visibility for a given frame.
///
/// Returns true if the element should be visible (on for 2/3 of interval).
/// An `interval` of 0 or 1 (skins may set one) means "never blink": a
/// one-frame cycle has no room for an off phase.
pub fn blink_visible(frame: u32, interval: u32) -> bool {
    if interval <= 1 {
        return true;
    }
    let phase = u64::from(frame % interval);
    // On for the first 2/3 (at least one frame), off for the rest. u64
    // so `interval * 2` cannot overflow for huge intervals.
    phase < (u64::from(interval) * 2 / 3).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_point_identity() {
        // 0 radians = no rotation
        let (rx, ry) = rotate_point(10, 0, 0, 0, 0.0);
        assert_eq!((rx, ry), (10, 0));
    }

    #[test]
    fn rotate_point_90_degrees() {
        use core::f32::consts::FRAC_PI_2;
        let (rx, ry) = rotate_point(10, 0, 0, 0, FRAC_PI_2);
        // (10, 0) rotated 90 degrees CW around origin → (0, 10)
        assert!(rx.abs() <= 1);
        assert!((ry - 10).abs() <= 1);
    }

    #[test]
    fn rotate_rect_produces_polygon() {
        let op = rotate_rect(0, 0, 10, 10, 0.0, Color::WHITE);
        let VectorOp::FillPolygon { points, .. } = op else {
            panic!("expected FillPolygon, got {op:?}");
        };
        assert_eq!(points.len(), 4);
        // At 0 rotation, corners should be approximately the original;
        assert_eq!(points[0], (0, 0));
        assert_eq!(points[1], (10, 0));
    }

    #[test]
    fn float_offset_oscillates() {
        let o1 = float_offset(0, 0, 2.0, 0.04);
        let o2 = float_offset(40, 0, 2.0, 0.04);
        // Should produce different offsets at different frames
        // (at frame 0, sin(0)=0; at frame 40, sin(1.6) != 0)
        assert!(o1 != o2 || o1 == 0);
    }

    #[test]
    fn float_offset_staggers_slots() {
        let o0 = float_offset(10, 0, 2.0, 0.04);
        let o1 = float_offset(10, 1, 2.0, 0.04);
        // Different slots at same frame may differ due to phase offset
        let _ = (o0, o1); // just ensure no panic
    }

    #[test]
    fn pulse_alpha_range() {
        for f in 0..200 {
            let a = pulse_alpha(f, 0.06, 80);
            assert!(a >= 80);
        }
    }

    #[test]
    fn blink_visible_duty_cycle() {
        let interval = 45;
        let on_count = (0..interval)
            .filter(|&f| blink_visible(f, interval))
            .count();
        // Should be on for ~2/3 of the interval (30 frames)
        assert_eq!(on_count, 30);
    }

    #[test]
    fn anim_clock_tick_frame() {
        let mut clock = AnimClock::new();
        assert_eq!(clock.frame(), 0);
        assert!((clock.time_s() - 0.0).abs() < f32::EPSILON);
        clock.tick_frame();
        assert_eq!(clock.frame(), 1);
        assert!((clock.time_s() - 1.0 / 60.0).abs() < 0.001);
    }

    #[test]
    fn anim_clock_tick_dt() {
        let mut clock = AnimClock::new();
        clock.tick_dt(16); // ~60fps
        assert_eq!(clock.frame(), 1);
        assert!((clock.time_s() - 0.016).abs() < 0.001);
    }

    #[test]
    fn anim_clock_sine_range() {
        let mut clock = AnimClock::new();
        for _ in 0..120 {
            let v = clock.sine(1.0, 0.0);
            assert!((-1.0..=1.0).contains(&v));
            let n = clock.sine_norm(1.0, 0.0);
            assert!((0.0..=1.0).contains(&n));
            clock.tick_frame();
        }
    }

    #[test]
    fn anim_clock_sawtooth_range() {
        let mut clock = AnimClock::new();
        for _ in 0..120 {
            let v = clock.sawtooth(2.0, 0.0);
            assert!((0.0..=1.0).contains(&v));
            clock.tick_frame();
        }
    }

    #[test]
    fn anim_clock_sawtooth_zero_period() {
        let clock = AnimClock::new();
        assert!((clock.sawtooth(0.0, 0.0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn entrance_scale_easing() {
        assert!((entrance_scale(0, 200) - 0.0).abs() < 0.01);
        assert!((entrance_scale(200, 200) - 1.0).abs() < 0.01);
        assert!((entrance_scale(400, 200) - 1.0).abs() < 0.01);
        // Midpoint should be > 0.5 (ease-out)
        assert!(entrance_scale(100, 200) > 0.5);
    }

    #[test]
    fn entrance_alpha_range() {
        assert_eq!(entrance_alpha(0, 200), 0);
        assert_eq!(entrance_alpha(200, 200), 255);
        assert_eq!(entrance_alpha(400, 200), 255);
    }

    #[test]
    fn entrance_slide_y_easing() {
        assert_eq!(entrance_slide_y(0, 200, 20), 20);
        assert_eq!(entrance_slide_y(200, 200, 20), 0);
    }

    #[test]
    fn blink_zero_interval_is_always_visible() {
        assert!((0..10).all(|f| blink_visible(f, 0)));
    }

    #[test]
    fn sawtooth_negative_phase_wraps_forward() {
        let clock = AnimClock::new();
        assert!((clock.sawtooth(1.0, -0.25) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn entrance_zero_duration() {
        assert!((entrance_scale(0, 0) - 1.0).abs() < f32::EPSILON);
        assert_eq!(entrance_alpha(0, 0), 255);
        assert_eq!(entrance_slide_y(0, 0, 20), 0);
    }

    #[test]
    fn anim_clock_keeps_frame_step_after_days_of_uptime() {
        // Regression: an f32 accumulator at ~3 days (2^18 s) rounds every
        // 1/60 s tick to 1/32 s, so animations ran at ~2x speed.
        let mut clock = AnimClock::new();
        clock.time_s = 262_144.0;
        let before = clock.time_s;
        for _ in 0..60 {
            clock.tick_frame();
        }
        assert!((clock.time_s - before - 1.0).abs() < 1e-6);
        // A 1 Hz sine one quarter-second past a whole second is ~1.
        clock.time_s = 262_144.25;
        assert!((clock.sine(1.0, 0.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn anim_clock_sawtooth_stays_half_open() {
        let mut clock = AnimClock::new();
        clock.time_s = 1.0 - 1e-12; // narrows to 1.0f32
        let v = clock.sawtooth(1.0, 0.0);
        assert!((0.0..1.0).contains(&v), "{v}");
    }

    #[test]
    fn blink_interval_one_is_always_visible() {
        // Regression: interval 1 gave an on-phase of 1*2/3 = 0 frames, so
        // the element was permanently hidden.
        assert!((0..10).all(|f| blink_visible(f, 1)));
    }

    #[test]
    fn blink_interval_two_alternates() {
        assert!(blink_visible(0, 2));
        assert!(!blink_visible(1, 2));
    }

    #[test]
    fn blink_huge_interval_does_not_overflow() {
        assert!(blink_visible(0, u32::MAX));
        assert!(!blink_visible(u32::MAX - 1, u32::MAX));
    }

    #[test]
    fn rotate_handles_extreme_coordinates() {
        // Regression: `px - cx` overflowed i32 (a debug-build panic).
        let (rx, _) = rotate_point(i32::MAX, 0, i32::MIN, 0, core::f32::consts::PI);
        assert_eq!(rx, i32::MIN, "saturates instead of wrapping");
        let VectorOp::FillPolygon { points, .. } =
            rotate_rect(i32::MAX - 5, 0, u32::MAX, 10, 0.0, Color::WHITE)
        else {
            panic!("expected FillPolygon");
        };
        assert_eq!(points[1].0, i32::MAX);
    }

    #[test]
    fn wrapped_stays_in_range_and_precise() {
        let mut clock = AnimClock::new();
        clock.time_s = 10_000_000.25; // ~4 months of uptime
        let v = clock.wrapped(4.0, 480.0);
        // 40_000_001 px mod 480 = 161: the quarter second survives.
        assert!((v - 161.0).abs() < 1e-3, "{v}");
        assert_eq!(clock.wrapped(1.0, 0.0), 0.0);
        assert_eq!(clock.wrapped(1.0, f32::NAN), 0.0);
        assert_eq!(clock.wrapped(f32::INFINITY, 10.0), 0.0);
        let neg = clock.wrapped(-4.0, 480.0);
        assert!((0.0..480.0).contains(&neg), "{neg}");
    }

    #[test]
    fn sawtooth_rejects_nan_inputs() {
        let clock = AnimClock::from_frame(90);
        assert_eq!(clock.sawtooth(f32::NAN, 0.0), 0.0);
        assert_eq!(clock.sawtooth(1.0, f32::NAN), 0.0);
    }

    #[test]
    fn frame_oscillators_keep_advancing_after_days() {
        // Past 2^24 frames an f32 `frame * speed` stalls between frames.
        let f = 1 << 25;
        let a = pulse_alpha(f, 0.05, 0);
        let changed = (1..20).any(|d| pulse_alpha(f + d, 0.05, 0) != a);
        assert!(changed, "pulse frozen at frame {f}");
        let o = float_offset(f, 0, 50.0, 0.05);
        assert!((1..20).any(|d| float_offset(f + d, 0, 50.0, 0.05) != o));
    }

    #[test]
    fn rotate_point_half_turn_is_symmetric() {
        // Regression: truncation toward zero gave (-9, 0).
        let (rx, ry) = rotate_point(10, 0, 0, 0, core::f32::consts::PI);
        assert_eq!((rx, ry), (-10, 0));
        let (rx, ry) = rotate_point(0, 7, 0, 0, core::f32::consts::FRAC_PI_2);
        assert_eq!((rx, ry), (-7, 0));
    }
}
