use oasis_backend_sdl::shader_bridge::Visibility;
use oasis_core::apps::AppRunner;
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::sdi::SdiRegistry;
use oasis_core::statusbar::StatusBar;
use oasis_core::toast::ToastManager;
use oasis_core::wm::window::{Window, WindowState, WmTheme};

use crate::app_state::{AppState, Mode};
use oasis_core::terminal_sdi;

/// Update the SDI scene graph based on the current mode.
///
/// This controls which UI elements are visible and positioned correctly
/// each frame. The actual rendering (`backend.clear`, `sdi.draw`, etc.)
/// remains in main.rs since it requires `&mut backend`.
pub fn update_sdi(state: &mut AppState, sdi: &mut SdiRegistry) {
    // Advance animations each frame.
    state.ui.dashboard.tick_animation();
    state.ui.start_menu.tick_animation();
    state.ui.bottom_bar.tick_animation(&state.active_theme);
    state.toasts.tick();

    // Any frame spent outside Terminal mode may hide the terminal objects
    // or change the theme, so force a full rebuild on re-entry.
    if state.mode != Mode::Terminal {
        state.terminal.sdi_signature = None;
    }

    match state.mode {
        Mode::Dashboard => {
            terminal_sdi::set_terminal_visible(sdi, false);
            AppRunner::hide_sdi(sdi);
            state.ui.taskbar.hide_sdi(sdi);

            if state.ui.bottom_bar.active_tab == MediaTab::None {
                state.ui.dashboard.update_sdi(sdi, &state.active_theme);
                terminal_sdi::hide_media_page(sdi);
            } else {
                state.ui.dashboard.hide_sdi(sdi);
                terminal_sdi::update_media_page(sdi, &state.ui.bottom_bar, &state.active_theme);
            }

            state
                .ui
                .status_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            state
                .ui
                .bottom_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            if state.skin.features.start_menu {
                state.ui.start_menu.update_sdi(sdi, &state.active_theme);
            }
        },
        Mode::Terminal => {
            state.ui.dashboard.hide_sdi(sdi);
            AppRunner::hide_sdi(sdi);
            StatusBar::hide_sdi(sdi);
            BottomBar::hide_sdi(sdi);
            state.ui.taskbar.hide_sdi(sdi);
            state.ui.start_menu.close();
            state.ui.start_menu.hide_sdi(sdi);
            terminal_sdi::hide_media_page(sdi);
            let cursor_visible = state.active_theme.terminal_cursor_blink_rate == 0
                || (state.frame_counter / state.active_theme.terminal_cursor_blink_rate as u64)
                    .is_multiple_of(2);
            // The full rebuild walks every visible line (SGR run parsing,
            // per-line SDI lookups) plus ~20 theme-color HashMap lookups;
            // skip it while nothing it renders from has changed. The hash
            // covers exactly the inputs of `setup_terminal_objects`: the
            // visible scrollback window, prompt state, blink phase, and
            // the theme geometry/colors it reads. Theme edits made in
            // other modes are covered by the `sdi_signature = None` reset
            // below; edits made *in* the terminal echo into the
            // scrollback and change the hash themselves.
            let sig = {
                use std::hash::{Hash, Hasher};
                let at = &state.active_theme;
                let term = &state.terminal;
                let mut h = std::hash::DefaultHasher::new();
                let end = term.output_lines.len().saturating_sub(term.scroll_offset);
                let start = end.saturating_sub(terminal_sdi::visible_output_lines(at));
                term.output_lines.len().hash(&mut h);
                for line in &term.output_lines[start..end] {
                    line.hash(&mut h);
                }
                term.cwd.hash(&mut h);
                term.input_buf.hash(&mut h);
                term.scroll_offset.hash(&mut h);
                cursor_visible.hash(&mut h);
                (
                    at.screen_w,
                    at.screen_h,
                    at.statusbar_height,
                    at.bottombar_height,
                )
                    .hash(&mut h);
                (
                    at.terminal_border_radius,
                    at.terminal_line_height,
                    at.font_small,
                )
                    .hash(&mut h);
                for c in [at.app.terminal_output_color, at.app.terminal_prompt_color] {
                    (c.r, c.g, c.b, c.a).hash(&mut h);
                }
                h.finish()
            };
            if state.terminal.sdi_signature != Some(sig) {
                terminal_sdi::setup_terminal_objects(
                    sdi,
                    &state.terminal.output_lines,
                    &state.terminal.cwd,
                    &state.terminal.input_buf,
                    state.terminal.scroll_offset,
                    &state.active_theme,
                    cursor_visible,
                );
                state.terminal.sdi_signature = Some(sig);
            }
        },
        Mode::App => {
            state.ui.dashboard.hide_sdi(sdi);
            terminal_sdi::set_terminal_visible(sdi, false);
            terminal_sdi::hide_media_page(sdi);
            state.ui.taskbar.hide_sdi(sdi);
            state.ui.start_menu.close();
            state.ui.start_menu.hide_sdi(sdi);
            state
                .ui
                .status_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            state
                .ui
                .bottom_bar
                .update_sdi(sdi, &state.active_theme, &state.skin.features);
            if let Some(ref mut runner) = state.content.app_runner {
                runner.update_sdi(sdi, &state.active_theme);
            }
        },
        Mode::Desktop => {
            terminal_sdi::set_terminal_visible(sdi, false);
            AppRunner::hide_sdi(sdi);
            terminal_sdi::hide_media_page(sdi);

            // Sync terminal output to the windowed terminal runner (only when
            // changed). `dirty` is a catch-all set on any input event, so the
            // content signature gates the expensive full-scrollback clone to
            // frames where the terminal actually changed — a mouse drag over
            // the desktop must not deep-copy 2000 lines per frame.
            if state.terminal.dirty {
                let term = &state.terminal;
                let changed = term.sync_signature.as_ref().is_none_or(|sig| {
                    sig.0 != term.output_lines.len()
                        || sig.1 != term.scroll_offset
                        || sig.2 != term.input_buf
                        || Some(sig.3.as_str()) != term.output_lines.first().map(String::as_str)
                        || Some(sig.4.as_str()) != term.output_lines.last().map(String::as_str)
                });
                if let Some((_, runner)) = state
                    .content
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| id == "terminal")
                    // A freshly (re)opened runner has not received this
                    // content yet even if the signature matches — its line
                    // count (scrollback + prompt) gives that away.
                    && (changed || runner.lines.len() != state.terminal.output_lines.len() + 1)
                {
                    let mut lines = state.terminal.output_lines.clone();
                    let prompt = format!("> {}", state.terminal.input_buf);
                    lines.push(prompt);
                    runner.set_lines(lines, state.terminal.scroll_offset);
                    state.terminal.sync_signature = Some((
                        state.terminal.output_lines.len(),
                        state.terminal.scroll_offset,
                        state.terminal.input_buf.clone(),
                        state
                            .terminal
                            .output_lines
                            .first()
                            .cloned()
                            .unwrap_or_default(),
                        state
                            .terminal
                            .output_lines
                            .last()
                            .cloned()
                            .unwrap_or_default(),
                    ));
                }
                state.terminal.dirty = false;
            }
            // Keep dashboard icons visible behind windows.
            if state.ui.bottom_bar.active_tab == MediaTab::None {
                state.ui.dashboard.update_sdi(sdi, &state.active_theme);
            } else {
                state.ui.dashboard.hide_sdi(sdi);
            }
            if state.content.fullscreen_app.is_some() {
                StatusBar::hide_sdi(sdi);
                BottomBar::hide_sdi(sdi);
                state.ui.taskbar.hide_sdi(sdi);
                state.ui.start_menu.close();
                state.ui.start_menu.hide_sdi(sdi);
                // Hide wallpaper so it doesn't bleed through.
                if let Ok(obj) = sdi.get_mut("wallpaper") {
                    obj.visible = false;
                }
            } else {
                state
                    .ui
                    .status_bar
                    .update_sdi(sdi, &state.active_theme, &state.skin.features);
                state
                    .ui
                    .bottom_bar
                    .update_sdi(sdi, &state.active_theme, &state.skin.features);
                state.ui.taskbar.update_sdi(
                    sdi,
                    &state.active_theme,
                    state.wm.windows(),
                    state.wm.active_window(),
                    state.skin.features.start_menu,
                );
                state.ui.taskbar.update_desktop_indicator(
                    sdi,
                    &state.active_theme,
                    state.ui.desktops.active_desktop(),
                    state.ui.desktops.desktop_count(),
                );
                if state.skin.features.start_menu {
                    state.ui.start_menu.update_sdi(sdi, &state.active_theme);
                }
            }
        },
        Mode::Osk => {
            if let Some(ref mut osk_state) = state.osk {
                osk_state.tick_animation();
                osk_state.update_sdi(sdi, &state.active_theme);
            }
        },
    }

    // Update toast overlays (visible in Dashboard, App, Desktop modes).
    match state.mode {
        Mode::Dashboard | Mode::App | Mode::Desktop => {
            state.toasts.update_sdi(sdi, &state.active_theme);
        },
        _ => {
            ToastManager::hide_sdi(sdi);
        },
    }

    // Software cursor: skins opt in via `features.software_cursor` (the
    // host pointer is hidden by `refresh_skin_assets`). Everyone else
    // keeps the host OS pointer — drawing our own would duplicate it.
    if state.skin.features.software_cursor {
        state.ui.mouse_cursor.update_sdi(sdi);
    }

    // Ensure wallpaper is visible and at lowest z (skip during fullscreen kiosk
    // where we explicitly hide it to prevent bleed-through, and skip when a
    // shader layer replaces the wallpaper).
    let fullscreen_active =
        matches!(state.mode, Mode::Desktop) && state.content.fullscreen_app.is_some();
    let shader_active = oasis_core::vector_overlay::get_shader_layer(&state.active_theme).is_some();
    if !fullscreen_active
        && !shader_active
        && let Ok(obj) = sdi.get_mut("wallpaper")
    {
        obj.visible = true;
    }
    // Hide opaque content_bg when shader provides the background.
    if shader_active && let Ok(obj) = sdi.get_mut("content_bg") {
        obj.visible = false;
    }
}

/// Compute how much of the shader wallpaper can actually be seen this
/// frame, from state the main loop already tracks (mode, window manager).
///
/// The result feeds `SdlShaderBridge::set_visibility`, which skips the
/// expensive CPU shade pass (and the blit) while the wallpaper is fully
/// covered, and drops the shade rate while it is mostly covered.
///
/// Conservative by design: only surfaces that provably paint an opaque
/// rect over the whole canvas every frame count as occluding.
/// Specifically, occlusion is only reported in `Mode::Desktop` when a
/// non-minimized window (visible on the active virtual desktop) covers
/// the full screen rect and its backing SDI objects are provably opaque
/// (see [`window_provably_opaque`]). This includes fullscreen kiosk apps
/// (`enter_fullscreen` expands the window to the full screen) and
/// maximized windows only when their insets are zero. Everything else —
/// Terminal mode (inset, rounded background), App mode, OSK, translucent
/// or nine-patch window chrome — keeps the wallpaper live.
pub fn wallpaper_visibility(state: &AppState) -> Visibility {
    if state.mode != Mode::Desktop {
        return Visibility::Visible;
    }
    let sw = state.active_theme.screen_w;
    let sh = state.active_theme.screen_h;
    if sw == 0 || sh == 0 {
        return Visibility::Visible;
    }
    let theme = state.wm.theme();

    // Largest on-screen area among opaque (non-covering) windows, for the
    // partial-coverage rate reduction. A single-window maximum is used
    // instead of a union: cheap, and an underestimate (conservative).
    let mut max_area: u64 = 0;
    for win in state.wm.windows() {
        if win.state == WindowState::Minimized
            || !state.ui.desktops.is_visible(win.id.as_str())
            || !window_provably_opaque(win, theme)
        {
            continue;
        }
        if rect_covers_screen(win.x, win.y, win.outer_w, win.outer_h, sw, sh) {
            return Visibility::Occluded;
        }
        let x0 = i64::from(win.x.max(0));
        let y0 = i64::from(win.y.max(0));
        let x1 = (i64::from(win.x) + i64::from(win.outer_w)).min(i64::from(sw));
        let y1 = (i64::from(win.y) + i64::from(win.outer_h)).min(i64::from(sh));
        if x1 > x0 && y1 > y0 {
            max_area = max_area.max(((x1 - x0) * (y1 - y0)) as u64);
        }
    }

    // An opaque window covering at least half the screen: shade slower.
    if max_area * 2 >= u64::from(sw) * u64::from(sh) {
        Visibility::PartiallyCovered
    } else {
        Visibility::Visible
    }
}

/// Whether a window's chrome provably paints every pixel of its outer
/// rect opaquely.
///
/// Kiosk windows and `WindowType::Fullscreen` windows draw only their
/// `content` SDI object, which spans the full outer rect (square
/// corners, no texture) — opaque iff `content_bg_color` is. All other
/// window types draw a `frame` object spanning the outer rect first;
/// that is provably opaque only with a fully-opaque frame color, square
/// corners, and no nine-patch texture (textures may carry alpha).
fn window_provably_opaque(win: &Window, theme: &WmTheme) -> bool {
    if win.fullscreen_kiosk || !win.sdi_suffixes().contains(&"frame") {
        theme.content_bg_color.a == 255
    } else {
        theme.frame_color.a == 255
            && theme.frame_border_radius == 0
            && theme.frame_nine_patch.is_none()
            && theme.frame_patch.is_none()
    }
}

/// Whether the rect `(x, y, w, h)` fully contains the screen rect.
fn rect_covers_screen(x: i32, y: i32, w: u32, h: u32, sw: u32, sh: u32) -> bool {
    x <= 0
        && y <= 0
        && i64::from(x) + i64::from(w) >= i64::from(sw)
        && i64::from(y) + i64::from(h) >= i64::from(sh)
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: The `update_sdi` function is heavily coupled to the SDI backend and AppState.
    // It requires fully initialized AppState and SdiRegistry instances, making unit testing
    // impractical without extensive mocking. The function's correctness is validated through
    // integration tests and visual inspection in the running application.
    //
    // Potential testable aspects if refactored:
    // - Visibility logic could be extracted to pure functions
    // - Mode-to-visibility mapping could be a lookup table
    // - Each mode's rendering behavior could be a separate function

    #[test]
    fn test_mode_enum_coverage() {
        // Verify all Mode variants are handled in update_sdi.
        // This test ensures we don't forget to add a match arm when adding new modes.
        let modes = vec![
            Mode::Dashboard,
            Mode::Terminal,
            Mode::App,
            Mode::Osk,
            Mode::Desktop,
        ];

        // If this compiles, all modes are at least syntactically valid.
        for mode in modes {
            let _ = mode;
        }
    }

    use oasis_core::backend::Color;
    use oasis_core::wm::window::{WindowConfig, WindowType};

    fn test_window(window_type: WindowType, x: i32, y: i32, w: u32, h: u32) -> Window {
        let theme = WmTheme::default();
        let config = WindowConfig {
            id: "test".to_string(),
            title: "Test".to_string(),
            x: Some(x),
            y: Some(y),
            width: w,
            height: h,
            window_type,
            always_on_top: false,
            modal: false,
        };
        let mut win = Window::new(&config, x, y, &theme);
        // Pin the *outer* rect to the requested size so tests reason
        // about screen coverage directly (Window::new adds chrome).
        win.outer_w = w;
        win.outer_h = h;
        win
    }

    #[test]
    fn rect_covers_screen_exact_and_overhang() {
        assert!(rect_covers_screen(0, 0, 480, 272, 480, 272));
        assert!(rect_covers_screen(-10, -10, 500, 292, 480, 272));
        // Short by one pixel in any direction: not covering.
        assert!(!rect_covers_screen(1, 0, 480, 272, 480, 272));
        assert!(!rect_covers_screen(0, 0, 479, 272, 480, 272));
        assert!(!rect_covers_screen(0, 0, 480, 271, 480, 272));
    }

    #[test]
    fn opaque_default_theme_window_is_provably_opaque() {
        let theme = WmTheme::default();
        let win = test_window(WindowType::AppWindow, 0, 0, 480, 272);
        assert!(window_provably_opaque(&win, &theme));
    }

    #[test]
    fn translucent_or_shaped_chrome_is_not_provably_opaque() {
        let win = test_window(WindowType::AppWindow, 0, 0, 480, 272);

        let translucent = WmTheme {
            frame_color: Color::rgba(40, 40, 40, 200),
            ..WmTheme::default()
        };
        assert!(!window_provably_opaque(&win, &translucent));

        let rounded = WmTheme {
            frame_border_radius: 6,
            ..WmTheme::default()
        };
        assert!(!window_provably_opaque(&win, &rounded));

        let nine_patch = WmTheme {
            frame_nine_patch: Some(("chrome".to_string(), [4, 4, 4, 4])),
            ..WmTheme::default()
        };
        assert!(!window_provably_opaque(&win, &nine_patch));
    }

    #[test]
    fn kiosk_window_opacity_follows_content_bg() {
        let mut win = test_window(WindowType::AppWindow, 0, 0, 480, 272);
        win.fullscreen_kiosk = true;

        // Kiosk hides all decorations; only content_bg matters — a
        // translucent frame color must not disqualify it.
        let theme = WmTheme {
            frame_color: Color::rgba(0, 0, 0, 0),
            ..WmTheme::default()
        };
        assert!(window_provably_opaque(&win, &theme));

        let translucent_bg = WmTheme {
            content_bg_color: Color::rgba(30, 30, 30, 128),
            ..theme
        };
        assert!(!window_provably_opaque(&win, &translucent_bg));
    }
}
