//! Toast notification system for ephemeral feedback messages.
//!
//! Toasts appear at the bottom-right corner, stacking upward.
//! They fade in/out over 10 frames and auto-dismiss after `toast_ttl` frames.

use std::collections::VecDeque;

use oasis_types::color::with_alpha;

use crate::active_theme::ActiveTheme;
use crate::sdi::SdiRegistry;
use crate::transition::ease_out_cubic;

/// Maximum number of visible toasts.
const MAX_VISIBLE: usize = 4;

/// SDI object names for the `MAX_VISIBLE` toast slots, precomputed so the
/// per-frame update never `format!`s a name (it runs even with no toasts).
const BG_NAMES: [&str; MAX_VISIBLE] = ["toast_bg_0", "toast_bg_1", "toast_bg_2", "toast_bg_3"];
const TEXT_NAMES: [&str; MAX_VISIBLE] = [
    "toast_text_0",
    "toast_text_1",
    "toast_text_2",
    "toast_text_3",
];

/// Toast severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// A single toast notification.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    /// Frames remaining before auto-dismiss.
    pub ttl: u32,
    /// Total initial TTL (for computing fade progress).
    pub total_ttl: u32,
    /// Frames since this toast was created (for slide-in animation).
    pub entrance_frame: u32,
}

impl Toast {
    /// Current alpha (0..255) based on fade-in / fade-out progress.
    fn alpha(&self, fade_frames: u32) -> u8 {
        let ff = fade_frames.max(1);
        let elapsed = self.total_ttl.saturating_sub(self.ttl);
        let fade_in = if elapsed < ff {
            elapsed as f32 / ff as f32
        } else {
            1.0
        };
        let fade_out = if self.ttl < ff {
            self.ttl as f32 / ff as f32
        } else {
            1.0
        };
        (fade_in.min(fade_out) * 255.0) as u8
    }
}

/// Manages the toast queue and rendering.
#[derive(Debug)]
pub struct ToastManager {
    toasts: VecDeque<Toast>,
    /// Monotonic count of toasts ever shown (for UI sound derivation).
    shown_total: u64,
    /// Monotonic count of `ToastLevel::Error` toasts ever shown.
    shown_errors: u64,
}

impl Default for ToastManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastManager {
    pub fn new() -> Self {
        Self {
            toasts: VecDeque::new(),
            shown_total: 0,
            shown_errors: 0,
        }
    }

    /// Monotonic `(total, errors)` counters of toasts ever shown. The UI
    /// sound queue diffs these once per frame to fire Toast/Error sounds
    /// without instrumenting every `show()` call site.
    pub fn shown_counts(&self) -> (u64, u64) {
        (self.shown_total, self.shown_errors)
    }

    /// Enqueue a new toast.
    pub fn show(&mut self, msg: impl Into<String>, level: ToastLevel, ttl: u32) {
        self.shown_total += 1;
        if level == ToastLevel::Error {
            self.shown_errors += 1;
        }
        self.toasts.push_back(Toast {
            message: msg.into(),
            level,
            total_ttl: ttl,
            ttl,
            entrance_frame: 0,
        });
        // Keep queue bounded.
        while self.toasts.len() > MAX_VISIBLE * 2 {
            self.toasts.pop_front();
        }
    }

    /// Advance all toasts by one frame, removing expired ones.
    pub fn tick(&mut self) {
        for toast in &mut self.toasts {
            toast.ttl = toast.ttl.saturating_sub(1);
            toast.entrance_frame = toast.entrance_frame.saturating_add(1);
        }
        self.toasts.retain(|t| t.ttl > 0);
    }

    /// Update SDI objects for visible toasts.
    ///
    /// Creates up to `MAX_VISIBLE` bg+text object pairs, hidden when unused.
    /// Objects are positioned at the bottom-right, stacking upward.
    pub fn update_sdi(&self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        let margin = at.toast.margin;
        let toast_w = ((at.screen_w as f32 * at.toast.width_fraction) as u32).max(120);
        let toast_h = at.toast.height;
        let gap = at.toast.gap;

        // The last MAX_VISIBLE toasts, oldest-first: slot i shows the
        // toast at `start + i`. Direct indexing avoids the two Vec
        // collects the old rev/take/rev dance allocated per frame.
        let vis_len = self.toasts.len().min(MAX_VISIBLE);
        let start = self.toasts.len() - vis_len;

        for i in 0..MAX_VISIBLE {
            let bg_name = BG_NAMES[i];
            let text_name = TEXT_NAMES[i];

            if !sdi.contains(bg_name) {
                let obj = sdi.create(bg_name);
                obj.z = 950;
                obj.overlay = true;
            }
            if !sdi.contains(text_name) {
                let obj = sdi.create(text_name);
                obj.z = 951;
                obj.overlay = true;
            }

            if let Some(toast) = (i < vis_len).then(|| &self.toasts[start + i]) {
                let alpha = toast.alpha(at.toast.fade_frames);
                let slot = (vis_len - 1 - i) as i32;
                let final_x = at.screen_w as i32 - toast_w as i32 - margin;
                let y = at.screen_h as i32
                    - at.bottombar_height as i32
                    - margin
                    - (slot + 1) * (toast_h as i32 + gap);

                // Slide-in animation: offset X from the right edge.
                let x = if at.toast.slide_in {
                    let ff = at.toast.fade_frames.max(1);
                    let progress = (toast.entrance_frame as f32 / ff as f32).min(1.0);
                    let offset = ((1.0 - ease_out_cubic(progress)) * toast_w as f32) as i32;
                    final_x + offset
                } else {
                    final_x
                };

                let bg_color = match toast.level {
                    ToastLevel::Info => at.toast.info_bg,
                    ToastLevel::Success => at.toast.success_bg,
                    ToastLevel::Warning => at.toast.warning_bg,
                    ToastLevel::Error => at.toast.error_bg,
                };

                if let Ok(obj) = sdi.get_mut(bg_name) {
                    obj.x = x;
                    obj.y = y;
                    obj.w = toast_w;
                    obj.h = toast_h;
                    obj.color =
                        with_alpha(bg_color, (bg_color.a as u16 * alpha as u16 / 255) as u8);
                    obj.border_radius = Some(at.toast.border_radius);
                    obj.shadow_level = Some(at.toast.shadow_level);
                    obj.visible = true;
                }
                if let Ok(obj) = sdi.get_mut(text_name) {
                    obj.x = x + 8;
                    obj.y = y + 4;
                    obj.w = toast_w.saturating_sub(16);
                    obj.h = toast_h.saturating_sub(8);
                    obj.font_size = at.font_body;
                    obj.set_text(&toast.message);
                    obj.text_color = with_alpha(at.toast.text_color, alpha);
                    obj.visible = true;
                    if at.toast.text_shadow {
                        obj.text_shadow_offset = Some((1, 1));
                        obj.text_shadow_color = Some(at.bar.text_shadow_color);
                    }
                }
            } else {
                if let Ok(obj) = sdi.get_mut(bg_name) {
                    obj.visible = false;
                }
                if let Ok(obj) = sdi.get_mut(text_name) {
                    obj.visible = false;
                }
            }
        }
    }

    /// Hide all toast SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        for name in BG_NAMES.iter().chain(TEXT_NAMES.iter()) {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_and_tick() {
        let mut tm = ToastManager::new();
        tm.show("hello", ToastLevel::Info, 60);
        assert_eq!(tm.toasts.len(), 1);
        for _ in 0..60 {
            tm.tick();
        }
        assert_eq!(tm.toasts.len(), 0);
    }

    #[test]
    fn fade_alpha() {
        let t = Toast {
            message: "test".to_string(),
            level: ToastLevel::Info,
            ttl: 5,
            total_ttl: 60,
            entrance_frame: 55,
        };
        // Near end of life -- should be fading out.
        assert!(t.alpha(10) < 255);

        let t2 = Toast {
            message: "test".to_string(),
            level: ToastLevel::Info,
            ttl: 55,
            total_ttl: 60,
            entrance_frame: 5,
        };
        // Near start -- should be fading in.
        assert!(t2.alpha(10) < 255);
    }

    #[test]
    fn shown_counts_track_levels() {
        let mut tm = ToastManager::new();
        assert_eq!(tm.shown_counts(), (0, 0));
        tm.show("info", ToastLevel::Info, 60);
        tm.show("boom", ToastLevel::Error, 60);
        tm.show("warn", ToastLevel::Warning, 60);
        assert_eq!(tm.shown_counts(), (3, 1));
        // Counters are monotonic: expiry doesn't rewind them.
        for _ in 0..120 {
            tm.tick();
        }
        assert_eq!(tm.shown_counts(), (3, 1));
    }

    #[test]
    fn max_queue_bounded() {
        let mut tm = ToastManager::new();
        for i in 0..20 {
            tm.show(format!("toast {i}"), ToastLevel::Info, 180);
        }
        assert!(tm.toasts.len() <= MAX_VISIBLE * 2);
    }

    #[test]
    fn update_sdi_creates_objects() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let mut tm = ToastManager::new();
        tm.show("test toast", ToastLevel::Success, 60);
        tm.update_sdi(&mut sdi, &at);

        assert!(sdi.contains("toast_bg_0"));
        assert!(sdi.contains("toast_text_0"));
        let bg = sdi.get("toast_bg_0").unwrap();
        assert!(bg.visible);
    }

    #[test]
    fn hide_sdi_hides_all() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let mut tm = ToastManager::new();
        tm.show("test", ToastLevel::Info, 60);
        tm.update_sdi(&mut sdi, &at);

        ToastManager::hide_sdi(&mut sdi);
        for i in 0..MAX_VISIBLE {
            let bg = sdi.get(&format!("toast_bg_{i}")).unwrap();
            assert!(!bg.visible);
        }
    }
}
