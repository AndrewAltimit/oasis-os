//! Dashboard animations: page slide, press flash, hover lift, entrance.

use super::DashboardState;
use crate::active_theme::ActiveTheme;

/// How far (px) a hovered icon lifts at the top of its eased animation.
const HOVER_LIFT_PX: f32 = 3.0;
/// Hover lift progress per frame (~5 frames / 80ms to fully lift).
const HOVER_LIFT_STEP: f32 = 0.2;

/// Active page-slide animation state.
#[derive(Debug)]
pub(super) struct PageSlideAnim {
    /// Current animation frame.
    pub(super) frame: u32,
    /// Total animation duration in frames (~200ms at 60fps).
    pub(super) duration: u32,
    /// Slide direction: -1 = left, +1 = right.
    pub(super) direction: i32,
}

impl PageSlideAnim {
    /// Eased progress in `[0, 1]`; a zero-frame slide (skin
    /// `page_slide_duration = 0`) is already complete instead of `0/0 = NaN`.
    pub(super) fn eased_progress(&self) -> f32 {
        if self.duration == 0 {
            return 1.0;
        }
        oasis_types::easing::ease_in_out_cubic(self.frame as f32 / self.duration as f32)
    }
}

impl DashboardState {
    /// Lift target for slot `i`: 1.0 while hovered (and not dragged).
    fn hover_target(&self, i: usize) -> f32 {
        if self.hover_index == Some(i) && self.drag_index != Some(i) {
            1.0
        } else {
            0.0
        }
    }

    /// Current hover lift (px) for slot `i`. Always 0 under
    /// `reduced_motion`.
    pub(super) fn hover_lift_px(&self, i: usize, at: &ActiveTheme) -> i32 {
        if at.ui_theme.reduced_motion {
            return 0;
        }
        let p = self.hover_lift.get(i).copied().unwrap_or(0.0);
        (oasis_types::easing::ease_out_cubic(p) * HOVER_LIFT_PX).round() as i32
    }

    /// Whether any hover lift is still easing toward its target.
    fn hover_lift_animating(&self) -> bool {
        self.hover_lift
            .iter()
            .enumerate()
            .any(|(i, &p)| p != self.hover_target(i))
    }

    /// Drop hover state (page change: slot indices now mean other apps).
    pub(super) fn reset_hover(&mut self) {
        self.hover_index = None;
        self.hover_lift.iter_mut().for_each(|p| *p = 0.0);
    }

    /// Trigger an icon press flash on the currently selected icon.
    pub fn trigger_press_flash(&mut self) {
        self.press_flash_frame = self.config.press_flash_duration;
        self.press_flash_index = self.selected;
    }

    /// Advance page-slide and cursor-lerp animations by one frame.
    pub fn tick_animation(&mut self) {
        // Page slide.
        if let Some(ref mut anim) = self.page_anim {
            anim.frame += 1;
            if anim.frame >= anim.duration {
                self.page_anim = None;
            }
        }
        // Press flash countdown.
        self.press_flash_frame = self.press_flash_frame.saturating_sub(1);
        // Entrance animation.
        if let Some(ref mut elapsed) = self.entrance_elapsed_ms {
            *elapsed = elapsed.saturating_add(16); // ~60fps
        }
        // Track selected index for focus glow.
        self.selected_index = self.selected;
        // Ease hover lifts toward their targets.
        for i in 0..self.hover_lift.len() {
            let target = self.hover_target(i);
            let p = &mut self.hover_lift[i];
            *p = if *p < target {
                (*p + HOVER_LIFT_STEP).min(target)
            } else {
                (*p - HOVER_LIFT_STEP).max(target)
            };
        }
    }

    /// Whether any dashboard animation is (or may be) live this frame.
    ///
    /// Used by the shell's idle frame elision: the vector-icon render
    /// path paints directly to the backend (outside the SDI scene
    /// graph), so its animations need an explicit "keep redrawing"
    /// signal. Deliberately over-approximate — e.g. per-icon
    /// spin/pulse/blink presets are reported page-wide — because a
    /// wasted redraw is cheap and a stale frame is a bug.
    pub fn has_active_animation(&self, at: &ActiveTheme) -> bool {
        // Transient animations: page slide, press flash, icon drag.
        if self.page_anim.is_some() || self.press_flash_frame > 0 || self.drag_index.is_some() {
            return true;
        }
        // Hover lift easing (skipped under reduced motion: nothing moves).
        if !at.ui_theme.reduced_motion && self.hover_lift_animating() {
            return true;
        }
        // Entrance animation. `entrance_elapsed_ms` saturates upward and
        // never returns to `None`, so compare against the full staggered
        // window (last icon's stagger + its duration).
        if at.entrance_style != "none"
            && let Some(elapsed) = self.entrance_elapsed_ms
        {
            let window = at.entrance_duration_ms.saturating_add(
                self.config
                    .icons_per_page
                    .saturating_mul(at.entrance_stagger_ms),
            );
            if elapsed < window {
                return true;
            }
        }
        // Continuous vector-icon idle animations (float bob and the
        // spin/pulse/blink glyph presets).
        if at.icon.style == "vector" {
            let preset_static = matches!(
                at.icon.vector_preset.as_str(),
                "geometric" | "hud" | "outline" | "solid" | "pixel"
            );
            let glyphs_animated = !preset_static
                && (at.icon.spin_enabled || at.icon.pulse_enabled || at.icon.blink_enabled);
            if at.icon.idle_float || glyphs_animated {
                return true;
            }
        }
        false
    }
}
