//! Window open / close / minimize / restore animations.
//!
//! Drives the [`AnimationManager`](crate::animation::AnimationManager)
//! from window lifecycle operations. Motion is **off by default** so
//! headless users (tests, the screenshot tool, PSP) get instant, final
//! geometry; interactive hosts opt in with
//! [`WindowManager::set_motion_enabled`] (typically
//! `!skin.features.reduced_motion`) and call
//! [`WindowManager::tick_animations`] once per frame.
//!
//! Animations are purely visual: a window's logical geometry and state
//! (what hit testing, focus and the taskbar see) change immediately, while
//! its SDI objects are laid out from the interpolated geometry until the
//! animation completes. A closed window leaves the window list at once but
//! keeps its SDI objects (drawn, not interactive) until its close
//! animation finishes; a minimized window is hidden only when its
//! minimize animation finishes.

use oasis_sdi::SdiRegistry;

use super::animation::{AnimationFrame, AnimationKind};
use super::manager::WindowManager;
use super::sdi_objects::set_window_visible;
use super::window::{Geometry, Window, WindowState};

/// Longest frame delta fed to the animations (keeps a stalled frame from
/// skipping an animation entirely, and a long stall from lasting forever).
const MAX_TICK_MS: u32 = 100;

/// `g` scaled to 90% around its center (open start / close end).
fn shrunk(g: Geometry) -> Geometry {
    // In u64 / saturating: `w * 9` overflowed for frames wider than
    // ~477M px.
    let scale = |v: u32| (u64::from(v) * 9 / 10) as u32;
    let (w, h) = (scale(g.w), scale(g.h));
    Geometry {
        x: g.x.saturating_add(((g.w - w) / 2) as i32),
        y: g.y.saturating_add(((g.h - h) / 2) as i32),
        w,
        h,
    }
}

impl WindowManager {
    /// Enable or disable window animations. When disabled (the default)
    /// every operation is instant; disabling while animations run makes
    /// them finish on the next [`Self::tick_animations`].
    pub fn set_motion_enabled(&mut self, enabled: bool) {
        self.motion_enabled = enabled;
        self.anim.set_reduced_motion(!enabled);
    }

    /// Whether window animations are enabled.
    pub fn motion_enabled(&self) -> bool {
        self.motion_enabled
    }

    /// Whether any window animation is in flight (hosts keep redrawing
    /// while this is true).
    pub fn is_animating(&self) -> bool {
        self.anim.active_count() > 0
    }

    /// Advance animations by the wall-clock time since the previous call.
    pub fn tick_animations(&mut self, sdi: &mut SdiRegistry) {
        self.tick_animations_at(web_time::Instant::now(), sdi);
    }

    /// Advance animations to time `now` (the host's frame clock; hosts
    /// with a virtual clock get deterministic animations).
    ///
    /// Only whole milliseconds are consumed; the sub-millisecond remainder
    /// carries over to the next call. (Resetting the clock every call used
    /// to round every sub-millisecond frame down to 0 ms, so a host
    /// looping faster than 1 kHz never finished an animation.)
    pub fn tick_animations_at(&mut self, now: web_time::Instant, sdi: &mut SdiRegistry) {
        let delta = match self.anim_last_tick {
            Some(t) if now > t => {
                let ms = now.duration_since(t).as_millis();
                if ms >= u128::from(MAX_TICK_MS) {
                    // Long stall: clamp the step and drop the backlog.
                    self.anim_last_tick = Some(now);
                    MAX_TICK_MS
                } else {
                    let ms = ms as u32;
                    self.anim_last_tick = Some(t + std::time::Duration::from_millis(u64::from(ms)));
                    ms
                }
            },
            Some(_) => 0,
            None => {
                self.anim_last_tick = Some(now);
                0
            },
        };
        self.tick_animations_by(delta, sdi);
    }

    /// Advance animations by `delta_ms` and apply the resulting frames.
    pub fn tick_animations_by(&mut self, delta_ms: u32, sdi: &mut SdiRegistry) {
        if self.anim.active_count() == 0 {
            return;
        }
        let delta = if self.motion_enabled {
            delta_ms
        } else {
            u32::MAX
        };
        for frame in self.anim.tick(delta) {
            self.apply_animation_frame(&frame, sdi);
        }
    }

    /// Start the open animation for a freshly created window.
    pub(crate) fn animate_open(&mut self, id: &str, sdi: &mut SdiRegistry) {
        let Some(end) = self
            .windows
            .iter()
            .find(|w| w.id == id)
            .map(Window::geometry)
        else {
            return;
        };
        self.start_window_animation(AnimationKind::Open, id, shrunk(end), end, sdi);
    }

    /// Start the minimize animation (shrinking towards the bottom edge,
    /// where taskbars live). Returns `false` when motion is disabled.
    ///
    /// An interrupted animation (minimizing a window that is still opening
    /// or restoring) continues from where the window is drawn instead of
    /// jumping back to its full geometry first.
    pub(crate) fn animate_minimize(&mut self, id: &str, sdi: &mut SdiRegistry) -> bool {
        let Some(logical) = self
            .windows
            .iter()
            .find(|w| w.id == id)
            .map(Window::geometry)
        else {
            return false;
        };
        let start = self.animating_geometry(id).unwrap_or(logical);
        let end = self.minimized_target(logical);
        self.start_window_animation(AnimationKind::Minimize, id, start, end, sdi)
    }

    /// Start the restore-from-minimized animation. `from` is where the
    /// window was drawn when the restore began (a minimize animation still
    /// in flight); `None` starts from the minimized target.
    pub(crate) fn animate_unminimize(
        &mut self,
        id: &str,
        from: Option<Geometry>,
        sdi: &mut SdiRegistry,
    ) {
        let Some(end) = self
            .windows
            .iter()
            .find(|w| w.id == id)
            .map(Window::geometry)
        else {
            return;
        };
        let start = from.unwrap_or_else(|| self.minimized_target(end));
        self.start_window_animation(AnimationKind::Restore, id, start, end, sdi);
    }

    /// Move a window into the closing list and start its close animation.
    /// `from` is the window's drawn geometry if another animation was in
    /// flight (closing a still-opening window shrinks from where it is).
    /// Returns `false` (window untouched) when motion is disabled.
    pub(crate) fn animate_close(&mut self, window: &Window, from: Option<Geometry>) -> bool {
        if !self.motion_enabled || window.state == WindowState::Minimized {
            return false;
        }
        let start = from.unwrap_or_else(|| window.geometry());
        if !self
            .anim
            .start_animation(AnimationKind::Close, &window.id, start, shrunk(start))
        {
            return false;
        }
        self.closing.push(window.clone());
        true
    }

    /// Instantly complete any animation on `id` (window or closing ghost).
    pub(crate) fn finish_animation(&mut self, id: &str, sdi: &mut SdiRegistry) {
        if !self.anim.is_animating(id) {
            return;
        }
        self.anim.cancel(id);
        self.complete_animation(id, sdi);
    }

    /// Destroy every closing ghost immediately (e.g. `close_all`, or a
    /// window with the same id being re-created).
    pub(crate) fn flush_closing(&mut self, id: Option<&str>, sdi: &mut SdiRegistry) {
        let ghosts: Vec<Window> = match id {
            Some(id) => {
                let (gone, keep) = std::mem::take(&mut self.closing)
                    .into_iter()
                    .partition(|w| w.id == id);
                self.closing = keep;
                gone
            },
            None => std::mem::take(&mut self.closing),
        };
        for ghost in ghosts {
            self.anim.cancel(&ghost.id);
            self.destroy_sdi_objects(&ghost, sdi);
        }
    }

    /// Where a minimized window shrinks to: a quarter-size rect centered
    /// horizontally on the window, at the bottom of the screen.
    fn minimized_target(&self, g: Geometry) -> Geometry {
        let w = (g.w / 4).max(1);
        let h = (g.h / 4).max(1);
        Geometry {
            x: g.x + g.w.saturating_sub(w) as i32 / 2,
            y: self.screen_h as i32 - h as i32,
            w,
            h,
        }
    }

    fn start_window_animation(
        &mut self,
        kind: AnimationKind,
        id: &str,
        start: Geometry,
        end: Geometry,
        sdi: &mut SdiRegistry,
    ) -> bool {
        if !self.motion_enabled || !self.anim.start_animation(kind, id, start, end) {
            return false;
        }
        // Show the first frame right away so nothing flashes at the end
        // geometry before the next tick.
        for frame in self.anim.tick(0) {
            if frame.window_id == id {
                self.apply_animation_frame(&frame, sdi);
            }
        }
        true
    }

    /// Lay out a window's SDI objects at the frame's interpolated geometry
    /// and alpha, or apply the final state when the frame completes.
    fn apply_animation_frame(&mut self, frame: &AnimationFrame, sdi: &mut SdiRegistry) {
        if frame.completed {
            self.complete_animation(&frame.window_id, sdi);
            return;
        }
        let source = self
            .windows
            .iter()
            .chain(self.closing.iter())
            .find(|w| w.id == frame.window_id.as_str());
        let Some(mut ghost) = source.cloned() else {
            return;
        };
        ghost.x = frame.x;
        ghost.y = frame.y;
        ghost.outer_w = frame.width.max(1);
        ghost.outer_h = frame.height.max(1);
        self.layout_window_sdi(&ghost, sdi);
        let alpha = (frame.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        set_window_alpha(&ghost, alpha, sdi);
        self.anim_visual.retain(|(id, _)| *id != ghost.id);
        self.anim_visual.push((ghost.id.clone(), ghost.geometry()));
    }

    /// Final state for a window whose animation ended (or was cut short).
    fn complete_animation(&mut self, id: &str, sdi: &mut SdiRegistry) {
        self.anim_visual.retain(|(wid, _)| *wid != id);
        if let Some(pos) = self.closing.iter().position(|w| w.id == id) {
            let ghost = self.closing.remove(pos);
            self.destroy_sdi_objects(&ghost, sdi);
            return;
        }
        let Some(window) = self.windows.iter().find(|w| w.id == id) else {
            return;
        };
        self.layout_window_sdi(window, sdi);
        set_window_alpha(window, 255, sdi);
        if window.state == WindowState::Minimized {
            set_window_visible(window, false, sdi);
        }
    }

    /// The geometry `id` is drawn at mid-animation, or `None` when it has
    /// no animation in flight (then it is drawn at its logical geometry).
    pub(crate) fn animating_geometry(&self, id: &str) -> Option<Geometry> {
        if self.anim.is_animating(id) {
            self.visual_geometry(id)
        } else {
            None
        }
    }

    /// The interpolated outer geometry of `id` while it animates.
    pub(crate) fn visual_geometry(&self, id: &str) -> Option<Geometry> {
        self.anim_visual
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, g)| *g)
    }

    /// Whether `id` has an animation in flight.
    pub(crate) fn window_animating(&self, id: &str) -> bool {
        self.anim.is_animating(id)
    }
}

/// Set `alpha` on every SDI object of `window`.
fn set_window_alpha(window: &Window, alpha: u8, sdi: &mut SdiRegistry) {
    for suffix in window.sdi_suffixes() {
        if let Ok(obj) = sdi.get_mut(&window.sdi_name(suffix))
            && obj.alpha != alpha
        {
            obj.alpha = alpha;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::manager::WindowManager;
    use crate::window::{Geometry, WindowConfig, WindowState, WindowType};
    use oasis_sdi::SdiRegistry;

    #[test]
    fn shrunk_handles_huge_frames() {
        let g = Geometry {
            x: i32::MAX - 5,
            y: 0,
            w: u32::MAX,
            h: 100,
        };
        let s = super::shrunk(g);
        assert_eq!((s.w, s.h), ((u64::from(u32::MAX) * 9 / 10) as u32, 90));
        assert_eq!((s.x, s.y), (i32::MAX, 5));
    }

    fn cfg(id: &str) -> WindowConfig {
        WindowConfig {
            id: id.to_string(),
            title: id.to_string(),
            x: Some(100),
            y: Some(100),
            width: 200,
            height: 100,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        }
    }

    fn animated_wm() -> (WindowManager, SdiRegistry) {
        let mut wm = WindowManager::new(800, 600);
        wm.set_motion_enabled(true);
        (wm, SdiRegistry::new())
    }

    fn frame_rect(sdi: &SdiRegistry, id: &str) -> (i32, i32, u32, u32, u8) {
        let f = sdi.get(&format!("{id}.frame")).expect("frame");
        (f.x, f.y, f.w, f.h, f.alpha)
    }

    #[test]
    fn sub_millisecond_frames_still_finish_animations() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        assert!(wm.is_animating());
        let mut now = web_time::Instant::now();
        // 20 s of 0.4 ms frames: each frame alone rounds down to 0 ms.
        for _ in 0..50_000 {
            now += std::time::Duration::from_micros(400);
            wm.tick_animations_at(now, &mut sdi);
            if !wm.is_animating() {
                break;
            }
        }
        assert!(!wm.is_animating(), "open animation never finished");
        let w = wm.get_window("a").expect("a");
        assert_eq!(frame_rect(&sdi, "a"), (w.x, w.y, w.outer_w, w.outer_h, 255));
    }

    #[test]
    fn motion_disabled_by_default_is_instant() {
        let mut wm = WindowManager::new(800, 600);
        let mut sdi = SdiRegistry::new();
        assert!(!wm.motion_enabled());
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        assert!(!wm.is_animating());
        let w = wm.get_window("a").expect("a");
        assert_eq!(frame_rect(&sdi, "a"), (w.x, w.y, w.outer_w, w.outer_h, 255));
        wm.close_window("a", &mut sdi).expect("close");
        assert!(!sdi.contains("a.frame"), "removed immediately");
    }

    #[test]
    fn open_animation_interpolates_geometry_and_alpha() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        let (fx, fy, fw, fh) = {
            let w = wm.get_window("a").expect("a");
            (w.x, w.y, w.outer_w, w.outer_h)
        };
        assert!(wm.is_animating());
        // First frame: shrunk and transparent.
        let (x0, _, w0, _, a0) = frame_rect(&sdi, "a");
        assert!(w0 < fw && x0 > fx && a0 == 0, "{w0} {x0} {a0}");
        // Half-way: in between.
        wm.tick_animations_by(100, &mut sdi);
        let (_, _, w1, _, a1) = frame_rect(&sdi, "a");
        assert!(w0 < w1 && w1 <= fw, "{w0} {w1} {fw}");
        assert!(a1 > 0 && a1 < 255);
        // Done: exact final geometry, opaque.
        wm.tick_animations_by(200, &mut sdi);
        assert!(!wm.is_animating());
        assert_eq!(frame_rect(&sdi, "a"), (fx, fy, fw, fh, 255));
    }

    #[test]
    fn close_removes_window_only_after_animation() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        wm.tick_animations_by(1000, &mut sdi);
        wm.close_window("a", &mut sdi).expect("close");
        // Logically gone at once...
        assert_eq!(wm.window_count(), 0);
        assert!(wm.get_window("a").is_none());
        // ...but still drawn while fading out.
        assert!(sdi.contains("a.frame"));
        wm.tick_animations_by(100, &mut sdi);
        assert!(sdi.contains("a.frame"));
        assert!(frame_rect(&sdi, "a").4 < 255);
        wm.tick_animations_by(100, &mut sdi);
        assert!(!sdi.contains("a.frame"), "destroyed once the close ends");
        assert!(!wm.is_animating());
    }

    #[test]
    fn reopening_during_close_keeps_new_window_objects() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        wm.close_window("a", &mut sdi).expect("close");
        wm.create_window(&cfg("a"), &mut sdi).expect("re-create");
        wm.tick_animations_by(1000, &mut sdi);
        assert!(sdi.contains("a.frame"));
        assert_eq!(frame_rect(&sdi, "a").4, 255);
    }

    #[test]
    fn minimize_hides_after_animation_and_restore_animates_back() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        wm.tick_animations_by(1000, &mut sdi);
        let full = frame_rect(&sdi, "a");
        wm.minimize_window("a", &mut sdi).expect("minimize");
        assert_eq!(wm.get_window("a").expect("a").state, WindowState::Minimized);
        assert!(sdi.get("a.frame").expect("frame").visible);
        wm.tick_animations_by(1000, &mut sdi);
        assert!(!sdi.get("a.frame").expect("frame").visible);

        wm.restore_window("a", &mut sdi).expect("restore");
        assert!(sdi.get("a.frame").expect("frame").visible);
        assert!(frame_rect(&sdi, "a").2 < full.2, "starts small");
        wm.tick_animations_by(1000, &mut sdi);
        assert_eq!(frame_rect(&sdi, "a"), full);
    }

    #[test]
    fn disabling_motion_finishes_pending_animations() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        wm.close_window("a", &mut sdi).expect("close");
        wm.set_motion_enabled(false);
        wm.tick_animations_by(0, &mut sdi);
        assert!(!sdi.contains("a.frame"));
        assert!(!wm.is_animating());
    }

    #[test]
    fn geometry_change_cuts_open_animation_short() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        wm.maximize_window("a", &mut sdi).expect("maximize");
        assert!(!wm.is_animating());
        assert_eq!(frame_rect(&sdi, "a"), (0, 0, 800, 600, 255));
    }
    // -- Interrupted animations continue from the drawn geometry --------

    #[test]
    fn minimize_during_open_continues_from_drawn_geometry() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        let full_w = wm.get_window("a").expect("a").outer_w;
        wm.tick_animations_by(40, &mut sdi);
        let drawn_w = frame_rect(&sdi, "a").2;
        assert!(drawn_w < full_w);
        wm.minimize_window("a", &mut sdi).expect("minimize");
        // The first minimize frame used to jump to the full width.
        assert_eq!(frame_rect(&sdi, "a").2, drawn_w);
        wm.tick_animations_by(1000, &mut sdi);
        assert!(!sdi.get("a.frame").expect("frame").visible);
    }

    #[test]
    fn restore_during_minimize_continues_from_drawn_geometry() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        wm.tick_animations_by(1000, &mut sdi);
        let full = frame_rect(&sdi, "a");
        wm.minimize_window("a", &mut sdi).expect("minimize");
        wm.tick_animations_by(100, &mut sdi);
        let mid = frame_rect(&sdi, "a");
        assert!(mid.2 < full.2 && mid.2 > full.2 / 4, "{mid:?}");
        wm.restore_window("a", &mut sdi).expect("restore");
        // Used to restart from the quarter-size minimized target.
        let first = frame_rect(&sdi, "a");
        assert_eq!(
            (first.0, first.1, first.2, first.3),
            (mid.0, mid.1, mid.2, mid.3)
        );
        wm.tick_animations_by(1000, &mut sdi);
        assert_eq!(frame_rect(&sdi, "a"), full);
        assert_eq!(wm.get_window("a").expect("a").state, WindowState::Normal);
    }

    #[test]
    fn close_during_open_shrinks_from_drawn_geometry() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("a"), &mut sdi).expect("create");
        wm.tick_animations_by(40, &mut sdi);
        let drawn_w = frame_rect(&sdi, "a").2;
        wm.close_window("a", &mut sdi).expect("close");
        wm.tick_animations_by(0, &mut sdi);
        assert!(
            frame_rect(&sdi, "a").2 <= drawn_w,
            "close grew the window first"
        );
        wm.tick_animations_by(1000, &mut sdi);
        assert!(!sdi.contains("a.frame"));
    }

    #[test]
    fn minimizing_a_zero_width_window_does_not_underflow() {
        let (mut wm, mut sdi) = animated_wm();
        wm.create_window(&cfg("z"), &mut sdi).expect("create");
        wm.tick_animations_by(10_000, &mut sdi);
        wm.resize_window("z", 0, 0, &mut sdi).expect("resize");
        wm.minimize_window("z", &mut sdi).expect("minimize");
        wm.tick_animations_by(10_000, &mut sdi);
        assert_eq!(wm.get_window("z").expect("z").state, WindowState::Minimized);
    }
}
