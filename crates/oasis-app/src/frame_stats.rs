//! Frame-phase instrumentation for the desktop shell (opt-in).
//!
//! Enabled by `OASIS_FRAME_STATS=1`. Accumulates per-phase render
//! timings (shader, SDI draw, vector overlays, present) plus drawn vs
//! skipped frame counts, and logs p50/p99 every ~5 seconds. When the env
//! var is unset, `FrameStats::from_env()` returns `None` and the main
//! loop takes no timestamps at all — zero steady-state overhead.

use std::time::{Duration, Instant};

/// How often accumulated stats are flushed to the log.
const REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// Per-frame phase durations captured by [`PhaseClock`].
pub struct PhaseTimes {
    /// Clear + shader wallpaper render/blit.
    pub shader: Duration,
    /// SDI scene draw (including windowed-app painting).
    pub sdi: Duration,
    /// Vector chrome overlays, scrollbar, transition overlay.
    pub vector: Duration,
    /// `swap_buffers` (vsync wait lands here).
    pub present: Duration,
    /// Whole render section, clear through present.
    pub total: Duration,
}

/// Lap timer for the render section's phases.
///
/// Construct at the top of the render section, call [`Self::lap`] at
/// each phase boundary, and [`Self::finish`] after present.
pub struct PhaseClock {
    start: Instant,
    last: Instant,
    shader: Duration,
    sdi: Duration,
    vector: Duration,
}

impl PhaseClock {
    /// Start timing a frame's render section.
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
            shader: Duration::ZERO,
            sdi: Duration::ZERO,
            vector: Duration::ZERO,
        }
    }

    /// Time elapsed since the previous lap (or since start).
    fn lap(&mut self) -> Duration {
        let now = Instant::now();
        let d = now - self.last;
        self.last = now;
        d
    }

    /// Mark the end of the clear + shader wallpaper phase.
    pub fn lap_shader(&mut self) {
        self.shader = self.lap();
    }

    /// Mark the end of the SDI scene draw phase.
    pub fn lap_sdi(&mut self) {
        self.sdi = self.lap();
    }

    /// Mark the end of the vector overlay / transition phase.
    pub fn lap_vector(&mut self) {
        self.vector = self.lap();
    }

    /// Finish after present; the remaining lap is the present time.
    pub fn finish(mut self) -> PhaseTimes {
        let present = self.lap();
        PhaseTimes {
            shader: self.shader,
            sdi: self.sdi,
            vector: self.vector,
            present,
            total: self.last - self.start,
        }
    }
}

/// Accumulates frame timings and periodically logs a summary.
pub struct FrameStats {
    shader_us: Vec<u32>,
    sdi_us: Vec<u32>,
    vector_us: Vec<u32>,
    present_us: Vec<u32>,
    total_us: Vec<u32>,
    drawn: u64,
    skipped: u64,
    last_report: Instant,
}

impl FrameStats {
    /// Build stats collection iff `OASIS_FRAME_STATS=1` is set.
    pub fn from_env() -> Option<Self> {
        (std::env::var("OASIS_FRAME_STATS").as_deref() == Ok("1")).then(|| Self {
            shader_us: Vec::new(),
            sdi_us: Vec::new(),
            vector_us: Vec::new(),
            present_us: Vec::new(),
            total_us: Vec::new(),
            drawn: 0,
            skipped: 0,
            last_report: Instant::now(),
        })
    }

    /// Record a fully rendered frame's phase timings.
    pub fn record_drawn(&mut self, t: PhaseTimes) {
        // Cap sample growth defensively; at 60 fps a 5 s report window
        // holds ~300 samples, so this never triggers in practice.
        if self.total_us.len() < 100_000 {
            self.shader_us.push(t.shader.as_micros() as u32);
            self.sdi_us.push(t.sdi.as_micros() as u32);
            self.vector_us.push(t.vector.as_micros() as u32);
            self.present_us.push(t.present.as_micros() as u32);
            self.total_us.push(t.total.as_micros() as u32);
        }
        self.drawn += 1;
        self.maybe_report();
    }

    /// Record an elided (skipped) frame.
    pub fn record_skipped(&mut self) {
        self.skipped += 1;
        self.maybe_report();
    }

    /// Flush a summary to the log if the report interval elapsed.
    fn maybe_report(&mut self) {
        if self.last_report.elapsed() < REPORT_INTERVAL {
            return;
        }
        let frames = self.drawn + self.skipped;
        let elided_pct = if frames > 0 {
            self.skipped as f64 * 100.0 / frames as f64
        } else {
            0.0
        };
        log::info!(
            "frame stats: drawn {} / skipped {} ({:.1}% elided) | p50/p99 us: \
             shader {}/{} sdi {}/{} vector {}/{} present {}/{} total {}/{}",
            self.drawn,
            self.skipped,
            elided_pct,
            pctl(&mut self.shader_us, 50),
            pctl(&mut self.shader_us, 99),
            pctl(&mut self.sdi_us, 50),
            pctl(&mut self.sdi_us, 99),
            pctl(&mut self.vector_us, 50),
            pctl(&mut self.vector_us, 99),
            pctl(&mut self.present_us, 50),
            pctl(&mut self.present_us, 99),
            pctl(&mut self.total_us, 50),
            pctl(&mut self.total_us, 99),
        );
        self.shader_us.clear();
        self.sdi_us.clear();
        self.vector_us.clear();
        self.present_us.clear();
        self.total_us.clear();
        self.drawn = 0;
        self.skipped = 0;
        self.last_report = Instant::now();
    }
}

/// Percentile via nearest-rank on a sorted copy of `samples`.
///
/// Sorts in place (samples are discarded after each report, so order
/// does not matter to the caller). Returns 0 for an empty slice.
fn pctl(samples: &mut [u32], p: u32) -> u32 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let rank = (samples.len() * p as usize).div_ceil(100);
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pctl_empty_is_zero() {
        assert_eq!(pctl(&mut [], 50), 0);
        assert_eq!(pctl(&mut [], 99), 0);
    }

    #[test]
    fn pctl_single_sample() {
        assert_eq!(pctl(&mut [42], 50), 42);
        assert_eq!(pctl(&mut [42], 99), 42);
    }

    #[test]
    fn pctl_nearest_rank() {
        let mut v: Vec<u32> = (1..=100).collect();
        assert_eq!(pctl(&mut v, 50), 50);
        assert_eq!(pctl(&mut v, 99), 99);
        assert_eq!(pctl(&mut v, 100), 100);
    }

    #[test]
    fn pctl_unsorted_input() {
        let mut v = vec![90, 10, 50, 30, 70];
        assert_eq!(pctl(&mut v, 50), 50);
    }

    #[test]
    fn phase_clock_accumulates_total() {
        let clock = PhaseClock::start();
        let t = clock.finish();
        // No laps: everything lands in present; total >= present.
        assert!(t.total >= t.present);
        assert_eq!(t.shader, Duration::ZERO);
        assert_eq!(t.sdi, Duration::ZERO);
        assert_eq!(t.vector, Duration::ZERO);
    }
}
