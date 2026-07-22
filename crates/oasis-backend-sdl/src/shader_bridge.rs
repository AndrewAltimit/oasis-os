//! CPU software shader background rendering bridge for the SDL3 backend.
//!
//! Uses the software (CPU) Balatro renderer to generate pixels, then uploads
//! them via the backend's own texture path — the same pipeline that renders
//! wallpaper, icons, and every other texture. This avoids any subtle
//! differences between a separate streaming texture and the backend's
//! internal texture management.
//!
//! The bridge keeps one long-lived streaming texture and refreshes its
//! pixels in place via [`super::SdlBackend::update_texture`] instead of a
//! destroy + create round-trip per frame. The CPU shade pass itself is
//! throttled to 30 Hz; throttled frames re-blit the cached texture (the
//! backend clears the canvas every frame, so the blit must always happen).

use oasis_shader::ShaderParams;
use oasis_shader::software::SoftwareShaderRenderer;
use oasis_types::backend::{SdiCore, TextureId};

/// Minimum shader-time advance (seconds) between CPU shade passes.
const SHADE_INTERVAL: f32 = 1.0 / 30.0;

/// Reduced shade rate used while the wallpaper is partially covered by
/// opaque surfaces (e.g. a large non-maximized window): ~12 Hz.
const SHADE_INTERVAL_PARTIAL: f32 = 1.0 / 12.0;

/// How much of the wallpaper the caller has determined to be visible.
///
/// Computed by the main loop from what it already knows (active mode,
/// window-manager state) and pushed into the bridge each frame via
/// [`SdlShaderBridge::set_visibility`]. Callers must be conservative:
/// only report [`Visibility::Occluded`] when an opaque surface provably
/// covers the whole canvas every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Wallpaper is (at least mostly) visible: shade at the full 30 Hz.
    #[default]
    Visible,
    /// A large opaque surface covers much of the wallpaper: shade at a
    /// reduced ~12 Hz. The blit still happens every frame.
    PartiallyCovered,
    /// An opaque surface provably covers the entire canvas: skip both
    /// the shade pass and the blit.
    Occluded,
}

/// Throttle for the expensive per-pixel CPU shade pass.
///
/// The first call always shades. Later calls shade only once `time` has
/// advanced by at least `interval` since the last shade; time moving
/// backwards (e.g. a frame-counter reset) also forces a shade.
struct ShadeThrottle {
    /// Shader time of the last granted shade, `None` before the first.
    last_shade_time: Option<f32>,
}

impl ShadeThrottle {
    fn new() -> Self {
        Self {
            last_shade_time: None,
        }
    }

    /// Whether a shade pass would run at `time`, without recording it.
    fn would_shade(&self, time: f32, interval: f32) -> bool {
        match self.last_shade_time {
            None => true,
            Some(last) => time < last || time - last >= interval,
        }
    }

    /// Whether a shade pass should run at `time`. Records `time` as the
    /// last shade when returning `true`.
    fn should_shade(&mut self, time: f32, interval: f32) -> bool {
        let shade = self.would_shade(time, interval);
        if shade {
            self.last_shade_time = Some(time);
        }
        shade
    }

    /// Forget the last shade so the next `should_shade` returns `true`.
    fn reset(&mut self) {
        self.last_shade_time = None;
    }
}

/// Shader rendering state for the SDL3 backend.
pub struct SdlShaderBridge {
    renderer: SoftwareShaderRenderer,
    /// Cached texture ID (managed by the backend's own texture system)
    /// plus the dimensions it was created with. A dimension mismatch
    /// after `resize` triggers lazy destroy + re-create on the next
    /// `render_and_blit`, which is the first point a backend is at hand.
    cached_tex: Option<(TextureId, u32, u32)>,
    /// Shader name last rendered into the cached texture; a change
    /// forces an immediate re-shade regardless of the throttle.
    last_shader: String,
    throttle: ShadeThrottle,
    /// Caller-reported wallpaper visibility (see [`Visibility`]).
    visibility: Visibility,
    width: u32,
    height: u32,
}

impl SdlShaderBridge {
    /// Create a software shader bridge.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        log::info!("shader bridge: software renderer initialized {width}x{height}");
        Some(Self {
            renderer: SoftwareShaderRenderer::new(width, height),
            cached_tex: None,
            last_shader: String::new(),
            throttle: ShadeThrottle::new(),
            visibility: Visibility::default(),
            width,
            height,
        })
    }

    /// The shade interval for the current visibility state.
    fn shade_interval(&self) -> f32 {
        match self.visibility {
            Visibility::PartiallyCovered => SHADE_INTERVAL_PARTIAL,
            Visibility::Visible | Visibility::Occluded => SHADE_INTERVAL,
        }
    }

    /// Whether calling [`Self::render_and_blit`] at `time` would run a
    /// CPU shade pass (as opposed to only re-blitting the cached
    /// texture).
    ///
    /// Non-mutating peek at the shade throttle. The shell's idle
    /// frame elision uses this: with a shader wallpaper active, a frame
    /// only needs redrawing when the shader would actually advance —
    /// otherwise re-blitting the cached texture reproduces the previous
    /// frame exactly. A missing cached texture always wants a frame.
    /// While [`Visibility::Occluded`] this returns `false`:
    /// `render_and_blit` is a no-op, so the shader never needs a frame
    /// (the visibility *transition* comes from a mode or window change,
    /// which dirties the scene and forces a redraw on its own).
    /// (A shader *switch* also forces a shade, but that comes from a
    /// skin change, which dirties the scene and forces a redraw anyway.)
    pub fn would_shade(&self, time: f32) -> bool {
        if self.visibility == Visibility::Occluded {
            return false;
        }
        self.cached_tex.is_none() || self.throttle.would_shade(time, self.shade_interval())
    }

    /// Report the wallpaper's current visibility (computed by the caller
    /// from mode / window-manager state).
    ///
    /// While [`Visibility::Occluded`], [`Self::render_and_blit`] is a
    /// no-op: no shade pass, no blit. Shader `time` is caller-supplied
    /// and keeps advancing during occlusion, so the animation keeps
    /// running "behind" the cover instead of freezing — when occlusion
    /// ends the wallpaper resumes at the current time rather than
    /// appearing frozen in the past. The throttle is reset on the
    /// occluded → visible transition so the first visible frame shades
    /// immediately instead of waiting out the 30 Hz window (which would
    /// flash the stale pre-occlusion frame).
    pub fn set_visibility(&mut self, visibility: Visibility) {
        if self.visibility == Visibility::Occluded && visibility != Visibility::Occluded {
            self.throttle.reset();
        }
        self.visibility = visibility;
    }

    /// Render a shader and blit the result to the SDL canvas.
    ///
    /// Allocates the streaming texture once, then refreshes its pixels in
    /// place with `update_texture` on later shades. The CPU shade pass is
    /// throttled to 30 Hz; throttled frames only re-blit the cached
    /// texture, which must happen every frame because the backend clears
    /// the canvas at the start of each frame.
    ///
    /// When the caller reported [`Visibility::Occluded`] via
    /// [`Self::set_visibility`], this is a no-op (the wallpaper cannot be
    /// seen, so neither the shade nor the blit is spent). Under
    /// [`Visibility::PartiallyCovered`] the shade rate drops to ~12 Hz
    /// while the per-frame blit continues.
    pub fn render_and_blit(
        &mut self,
        backend: &mut super::SdlBackend,
        shader_name: &str,
        time: f32,
        params: &ShaderParams,
    ) {
        if self.visibility == Visibility::Occluded {
            return;
        }

        // Drop a stale-size cached texture (e.g. after `resize`) so it is
        // re-created at the current dimensions below.
        if let Some((tex, w, h)) = self.cached_tex
            && (w != self.width || h != self.height)
        {
            let _ = backend.destroy_texture(tex);
            self.cached_tex = None;
        }

        // A missing texture or a shader switch must render immediately —
        // resetting the throttle makes the next check always pass.
        if self.cached_tex.is_none() || self.last_shader != shader_name {
            self.throttle.reset();
        }

        if self.throttle.should_shade(time, self.shade_interval()) {
            let pixels = self.renderer.render_shader(shader_name, time, params);
            self.last_shader = shader_name.to_string();

            // Refresh the cached texture in place; on failure (e.g. a
            // dimension mismatch) fall back to destroy + re-create.
            if let Some((tex, _, _)) = self.cached_tex
                && let Err(e) = backend.update_texture(tex, self.width, self.height, pixels)
            {
                log::warn!("shader texture update failed ({e}); re-creating");
                let _ = backend.destroy_texture(tex);
                self.cached_tex = None;
            }

            if self.cached_tex.is_none() {
                match backend.load_texture(self.width, self.height, pixels) {
                    Ok(tex) => {
                        self.cached_tex = Some((tex, self.width, self.height));
                    },
                    Err(e) => {
                        log::warn!("shader texture upload failed: {e}");
                    },
                }
            }
        }

        if let Some((tex, _, _)) = self.cached_tex {
            let _ = backend.blit(tex, 0, 0, self.width, self.height);
        }
    }

    /// Resize the renderer.
    ///
    /// The cached texture cannot be destroyed here (no backend at hand);
    /// `render_and_blit` notices the dimension mismatch and re-creates it
    /// at the new size on the next frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.renderer.resize(width, height);
        self.throttle.reset();
    }

    /// Clean up the cached texture.
    pub fn destroy(&mut self, backend: &mut super::SdlBackend) {
        if let Some((tex, _, _)) = self.cached_tex.take() {
            let _ = backend.destroy_texture(tex);
        }
        self.throttle.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_first_call_always_shades() {
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(0.0, SHADE_INTERVAL));
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(123.456, SHADE_INTERVAL));
    }

    #[test]
    fn throttle_blocks_within_interval() {
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(1.0, SHADE_INTERVAL));
        // 1/60s later: below the 1/30s interval.
        assert!(!t.should_shade(1.0 + 1.0 / 60.0, SHADE_INTERVAL));
        // Same time again: still blocked.
        assert!(!t.should_shade(1.0 + 1.0 / 60.0, SHADE_INTERVAL));
    }

    #[test]
    fn throttle_allows_after_interval() {
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(1.0, SHADE_INTERVAL));
        // Comfortably past the 1/30s interval (avoids f32 boundary
        // rounding at exactly `last + SHADE_INTERVAL`).
        assert!(t.should_shade(1.04, SHADE_INTERVAL));
    }

    #[test]
    fn throttle_measures_from_last_shade_not_last_call() {
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(0.0, SHADE_INTERVAL));
        // Denied calls must not push the reference time forward.
        assert!(!t.should_shade(0.02, SHADE_INTERVAL));
        assert!(t.should_shade(0.035, SHADE_INTERVAL));
        // Reference is now 0.035, not 0.02.
        assert!(!t.should_shade(0.05, SHADE_INTERVAL));
        assert!(t.should_shade(0.07, SHADE_INTERVAL));
    }

    #[test]
    fn throttle_time_going_backwards_forces_shade() {
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(100.0, SHADE_INTERVAL));
        // e.g. frame counter reset: never freeze on the old image.
        assert!(t.should_shade(0.0, SHADE_INTERVAL));
    }

    #[test]
    fn would_shade_is_non_mutating() {
        let mut t = ShadeThrottle::new();
        assert!(t.would_shade(1.0, SHADE_INTERVAL));
        // Peeking must not record the time — should_shade still shades.
        assert!(t.would_shade(1.0, SHADE_INTERVAL));
        assert!(t.should_shade(1.0, SHADE_INTERVAL));
        // Within the interval: peek and commit agree (both deny).
        assert!(!t.would_shade(1.01, SHADE_INTERVAL));
        assert!(!t.should_shade(1.01, SHADE_INTERVAL));
        // Past the interval: peek predicts the shade.
        assert!(t.would_shade(1.05, SHADE_INTERVAL));
        assert!(t.should_shade(1.05, SHADE_INTERVAL));
    }

    #[test]
    fn throttle_reset_forces_next_shade() {
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(2.0, SHADE_INTERVAL));
        assert!(!t.should_shade(2.01, SHADE_INTERVAL));
        t.reset();
        assert!(t.should_shade(2.01, SHADE_INTERVAL));
    }

    #[test]
    fn throttle_partial_interval_reduces_rate() {
        let mut t = ShadeThrottle::new();
        assert!(t.should_shade(1.0, SHADE_INTERVAL_PARTIAL));
        // Past the 30 Hz window but inside the 12 Hz window: blocked.
        assert!(!t.should_shade(1.04, SHADE_INTERVAL_PARTIAL));
        // Past the 12 Hz window: allowed.
        assert!(t.should_shade(1.09, SHADE_INTERVAL_PARTIAL));
    }

    fn test_bridge() -> SdlShaderBridge {
        match SdlShaderBridge::new(8, 8) {
            Some(b) => b,
            None => panic!("bridge creation failed"),
        }
    }

    #[test]
    fn occlusion_end_forces_immediate_shade() {
        let mut b = test_bridge();
        // Simulate a granted shade at t=5.0.
        assert!(b.throttle.should_shade(5.0, SHADE_INTERVAL));
        // Covered, then revealed one frame later: the reveal must not
        // wait out the 30 Hz window (5.016 - 5.0 < SHADE_INTERVAL).
        b.set_visibility(Visibility::Occluded);
        b.set_visibility(Visibility::Visible);
        assert!(b.throttle.should_shade(5.016, SHADE_INTERVAL));
    }

    #[test]
    fn would_shade_false_while_occluded() {
        let mut b = test_bridge();
        // No cached texture: a visible bridge always wants a frame, but
        // an occluded one never does (render_and_blit is a no-op).
        assert!(b.would_shade(1.0));
        b.set_visibility(Visibility::Occluded);
        assert!(!b.would_shade(1.0));
        // Reveal: wants a frame again (throttle was reset on reveal).
        b.set_visibility(Visibility::Visible);
        assert!(b.would_shade(1.0));
    }

    #[test]
    fn visibility_change_without_occlusion_keeps_throttle() {
        let mut b = test_bridge();
        assert!(b.throttle.should_shade(5.0, SHADE_INTERVAL));
        // Visible → partial → visible never passed through Occluded, so
        // the throttle reference is preserved.
        b.set_visibility(Visibility::PartiallyCovered);
        b.set_visibility(Visibility::Visible);
        assert!(!b.throttle.should_shade(5.016, SHADE_INTERVAL));
    }
}
