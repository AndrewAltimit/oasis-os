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
        }
    }

    /// Enqueue a new toast.
    pub fn show(&mut self, msg: impl Into<String>, level: ToastLevel, ttl: u32) {
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
        let margin = at.toast_margin;
        let toast_w = ((at.screen_w as f32 * at.toast_width_fraction) as u32).max(120);
        let toast_h = at.toast_height;
        let gap = at.toast_gap;

        // Take the last MAX_VISIBLE toasts.
        let visible: Vec<_> = self
            .toasts
            .iter()
            .rev()
            .take(MAX_VISIBLE)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        for i in 0..MAX_VISIBLE {
            let bg_name = format!("toast_bg_{i}");
            let text_name = format!("toast_text_{i}");

            if !sdi.contains(&bg_name) {
                let obj = sdi.create(&bg_name);
                obj.z = 950;
                obj.overlay = true;
            }
            if !sdi.contains(&text_name) {
                let obj = sdi.create(&text_name);
                obj.z = 951;
                obj.overlay = true;
            }

            if let Some(toast) = visible.get(i) {
                let alpha = toast.alpha(at.toast_fade_frames);
                let slot = (visible.len() - 1 - i) as i32;
                let final_x = at.screen_w as i32 - toast_w as i32 - margin;
                let y = at.screen_h as i32
                    - at.bottombar_height as i32
                    - margin
                    - (slot + 1) * (toast_h as i32 + gap);

                // Slide-in animation: offset X from the right edge.
                let x = if at.toast_slide_in {
                    let ff = at.toast_fade_frames.max(1);
                    let progress = (toast.entrance_frame as f32 / ff as f32).min(1.0);
                    let offset = ((1.0 - ease_out_cubic(progress)) * toast_w as f32) as i32;
                    final_x + offset
                } else {
                    final_x
                };

                let bg_color = match toast.level {
                    ToastLevel::Info => at.toast_info_bg,
                    ToastLevel::Success => at.toast_success_bg,
                    ToastLevel::Warning => at.toast_warning_bg,
                    ToastLevel::Error => at.toast_error_bg,
                };

                if let Ok(obj) = sdi.get_mut(&bg_name) {
                    obj.x = x;
                    obj.y = y;
                    obj.w = toast_w;
                    obj.h = toast_h;
                    obj.color =
                        with_alpha(bg_color, (bg_color.a as u16 * alpha as u16 / 255) as u8);
                    obj.border_radius = Some(at.toast_border_radius);
                    obj.shadow_level = Some(at.toast_shadow_level);
                    obj.visible = true;
                }
                if let Ok(obj) = sdi.get_mut(&text_name) {
                    obj.x = x + 8;
                    obj.y = y + 4;
                    obj.w = toast_w.saturating_sub(16);
                    obj.h = toast_h.saturating_sub(8);
                    obj.font_size = at.font_body;
                    obj.text = Some(toast.message.clone());
                    obj.text_color = with_alpha(at.toast_text_color, alpha);
                    obj.visible = true;
                    if at.toast_text_shadow {
                        obj.text_shadow_offset = Some((1, 1));
                        obj.text_shadow_color = Some(at.bar_text_shadow_color);
                    }
                }
            } else {
                if let Ok(obj) = sdi.get_mut(&bg_name) {
                    obj.visible = false;
                }
                if let Ok(obj) = sdi.get_mut(&text_name) {
                    obj.visible = false;
                }
            }
        }
    }

    /// Hide all toast SDI objects.
    pub fn hide_sdi(sdi: &mut SdiRegistry) {
        for i in 0..MAX_VISIBLE {
            for prefix in &["toast_bg_", "toast_text_"] {
                let name = format!("{prefix}{i}");
                if let Ok(obj) = sdi.get_mut(&name) {
                    obj.visible = false;
                }
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
