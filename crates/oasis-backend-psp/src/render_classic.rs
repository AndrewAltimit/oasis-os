//! Classic mode rendering (full-screen PSIX dashboard views).
//!
//! Handles lazy-loading of directory entries and dispatches rendering
//! to view-specific SDI update functions and direct backend drawing.

use oasis_backend_psp::{Color, PspBackend, SdiRegistry};

use crate::app_states::*;
use crate::chrome;
use crate::dashboard;
use crate::desktop;
use crate::types::{ClassicView, RadioStatus};
use crate::views;
use crate::views_sdi;

use oasis_core::terminal_sdi;

/// Render the current classic-mode view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_classic(
    backend: &mut PspBackend,
    sdi: &mut SdiRegistry,
    dashboard_state: &mut oasis_core::dashboard::DashboardState,
    active_theme: &oasis_core::active_theme::ActiveTheme,
    classic_view: ClassicView,
    prev_classic_view: &mut ClassicView,
    icons_hidden: bool,
    fm: &mut FileManagerState,
    pv: &mut PhotoViewerState,
    mp: &mut MusicPlayerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    term: &mut TerminalState,
    audio: &oasis_backend_psp::AudioHandle,
    viz_frame: u32,
    dbg_log: &dyn Fn(&str),
) {
    // Lazy-load directory entries for browser modes.
    if classic_view == ClassicView::FileManager && !fm.left.loaded {
        fm.left.entries = oasis_backend_psp::list_directory(&fm.left.path);
        fm.left.selected = 0;
        fm.left.scroll = 0;
        fm.left.loaded = true;
    }
    if classic_view == ClassicView::FileManager && !fm.right.loaded {
        fm.right.entries = oasis_backend_psp::list_directory(&fm.right.path);
        fm.right.selected = 0;
        fm.right.scroll = 0;
        fm.right.loaded = true;
    }
    if classic_view == ClassicView::PhotoViewer && !pv.loaded && !pv.viewing {
        let all = oasis_backend_psp::list_directory(&pv.path);
        pv.entries = all
            .into_iter()
            .filter(|e| {
                e.is_dir || {
                    let lower: String = e.name.chars().map(|c| c.to_ascii_lowercase()).collect();
                    lower.ends_with(".jpg") || lower.ends_with(".jpeg")
                }
            })
            .collect();
        pv.selected = 0;
        pv.scroll = 0;
        pv.loaded = true;
    }
    if classic_view == ClassicView::MusicPlayer && !mp.loaded && !audio.is_playing() {
        let all = oasis_backend_psp::list_directory(&mp.path);
        mp.entries = all
            .into_iter()
            .filter(|e| {
                e.is_dir || {
                    let lower: String = e.name.chars().map(|c| c.to_ascii_lowercase()).collect();
                    lower.ends_with(".mp3")
                }
            })
            .collect();
        mp.selected = 0;
        mp.scroll = 0;
        mp.loaded = true;
    }

    // Show or hide dashboard icons based on current view.
    let show_dashboard = classic_view == ClassicView::Dashboard && !icons_hidden;
    if show_dashboard {
        dashboard::show_dashboard_sdi(dashboard_state, sdi, active_theme);
    } else {
        dashboard::hide_dashboard_sdi(dashboard_state, sdi);
    }

    // Show or hide terminal SDI objects.
    if classic_view != ClassicView::Terminal {
        terminal_sdi::set_terminal_visible(sdi, false);
    }

    // View transition: hide old SDI objects, set up new ones.
    if classic_view != *prev_classic_view {
        views_sdi::hide_all(sdi);
        views_sdi::setup_view(sdi, classic_view);
        *prev_classic_view = classic_view;
    }

    match classic_view {
        ClassicView::Dashboard => {
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[
                    ("X", "Open"),
                    ("L/R", "Window"),
                    ("Start", "Term"),
                    ("Sel", "Desktop"),
                ],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::Terminal => {
            terminal_sdi::setup_terminal_objects(
                sdi,
                &term.lines,
                "/",
                &term.input,
                term.scroll,
                active_theme,
                viz_frame % 30 < 15,
            );
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[
                    ("X", "Run"),
                    ("[]", "OSK"),
                    ("Up/Dn", "Scroll"),
                    ("Start", "Back"),
                ],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::FileManager => {
            views_sdi::update_file_manager(
                sdi,
                &fm.left.path,
                &fm.left.entries,
                fm.left.selected,
                fm.left.scroll,
                &fm.right.path,
                &fm.right.entries,
                fm.right.selected,
                fm.right.scroll,
                fm.active_panel,
                active_theme,
            );
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[("X", "Open"), ("O", "Back"), ("<>", "Panel"), ("^v", "Nav")],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::PhotoViewer => {
            if pv.viewing {
                views_sdi::update_photo_view(sdi, pv.tex, pv.img_w, pv.img_h);
                backend.force_bitmap_font = true;
                chrome::draw_button_hints(backend, &[("O", "Back")]);
                backend.force_bitmap_font = false;
            } else if pv.loading {
                desktop::draw_loading_indicator(backend, "Decoding image...");
            } else {
                views_sdi::update_photo_browser(
                    sdi,
                    &pv.path,
                    &pv.entries,
                    pv.selected,
                    pv.scroll,
                    active_theme,
                );
                backend.force_bitmap_font = true;
                chrome::draw_button_hints(backend, &[("X", "View"), ("O", "Back"), ("^v", "Nav")]);
                backend.force_bitmap_font = false;
            }
        },
        ClassicView::MusicPlayer => {
            if audio.is_playing() {
                backend.force_bitmap_font = true;
                views::draw_music_player_threaded(backend, &mp.file_name, audio, viz_frame);
                chrome::draw_button_hints(
                    backend,
                    &[("X", "Pause"), ("[]", "Stop"), ("^v", "Back")],
                );
                backend.force_bitmap_font = false;
            } else {
                views_sdi::update_music_browser(
                    sdi,
                    &mp.path,
                    &mp.entries,
                    mp.selected,
                    mp.scroll,
                    active_theme,
                );
                backend.force_bitmap_font = true;
                chrome::draw_button_hints(backend, &[("X", "Play"), ("O", "Back"), ("^v", "Nav")]);
                backend.force_bitmap_font = false;
            }
        },
        ClassicView::Browser => {
            if let Some(ref mut w) = br.widget {
                if br.loading
                    || w.loading_state() == oasis_browser::LoadingState::Loading
                {
                    desktop::draw_loading_indicator(backend, "Loading page...");
                } else if w.loading_state() == oasis_browser::LoadingState::Error {
                    // Show error as simple text (avoid paint crash on PSP).
                    let msg = w.error_message().unwrap_or("Unknown error").to_string();
                    backend.force_bitmap_font = true;
                    let lines = views::wrap_text(&msg, 58);
                    for (i, line) in lines.iter().enumerate().take(25) {
                        backend.draw_text_inner(line, 4, 20 + (i as i32 * 9), 8, Color::WHITE);
                    }
                    backend.force_bitmap_font = false;
                } else {
                    let _ = w.paint(backend);
                }
            } else {
                // Widget not yet initialized -- show status message.
                backend.force_bitmap_font = true;
                backend.draw_text_inner(&br.status_msg, 4, 30, 8, Color::WHITE);
                backend.force_bitmap_font = false;
            }
            backend.force_bitmap_font = true;
            chrome::draw_button_hints(
                backend,
                &[("[]", "URL"), ("X", "Go"), ("LR", "Link"), ("O", "Back")],
            );
            backend.force_bitmap_font = false;
        },
        ClassicView::Radio => {
            backend.force_bitmap_font = true;
            match radio.status {
                RadioStatus::Stopped => {
                    views_sdi::update_radio(sdi, radio.selected, radio.scroll, active_theme);
                    chrome::draw_button_hints(
                        backend,
                        &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")],
                    );
                },
                RadioStatus::Connecting => {
                    desktop::draw_loading_indicator(backend, "Connecting...");
                },
                RadioStatus::Buffering | RadioStatus::Playing => {
                    views::draw_radio_playing(
                        backend,
                        &radio.station_name,
                        &radio.now_playing,
                        radio.status == RadioStatus::Buffering,
                        audio,
                        viz_frame,
                    );
                    chrome::draw_button_hints(
                        backend,
                        &[("[]", "Stop"), ("^", "Back"), ("O", "Stop+Back")],
                    );
                },
                RadioStatus::Error => {
                    views::draw_radio_error(backend, &radio.error_msg);
                    chrome::draw_button_hints(backend, &[("X", "Retry"), ("O", "Back")]);
                },
            }
            backend.force_bitmap_font = false;
        },
        ClassicView::TvGuide => {
            if viz_frame < 3 || viz_frame % 60 == 0 {
                dbg_log(&format!("[TV] render frame {}", viz_frame));
            }
            backend.force_bitmap_font = true;
            if tv.tuned.is_some() {
                views::draw_tv_playing(
                    backend,
                    &tv.now_playing,
                    tv.downloading,
                    tv.download_progress,
                    tv.preview_tex,
                    &tv.error_msg,
                );
                chrome::draw_button_hints(backend, &[("O", "Untune"), ("^", "Back")]);
            } else if !tv.error_msg.is_empty() {
                views::draw_tv_error(backend, &tv.error_msg);
                chrome::draw_button_hints(backend, &[("X", "Retry"), ("O", "Back")]);
            } else {
                views_sdi::update_tv_channels(
                    sdi,
                    &tv.channels,
                    &tv.catalogs,
                    tv.selected,
                    tv.scroll,
                    active_theme,
                );
                chrome::draw_button_hints(backend, &[("X", "Tune"), ("^v", "Nav"), ("O", "Back")]);
            }
            backend.force_bitmap_font = false;
        },
    }
}
