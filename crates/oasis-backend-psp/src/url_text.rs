//! URL text computation for the status bar.
//!
//! Derives the current navigation context string shown in the bar's
//! URL/path display area based on app mode and active view.

use crate::app_states::*;
use crate::types::{AppMode, ClassicView};

/// Compute the URL/path text displayed in the bottom bar.
pub(crate) fn compute_url_text(
    app_mode: AppMode,
    classic_view: ClassicView,
    fm: &FileManagerState,
    umd_activated: bool,
    audio: &oasis_backend_psp::AudioHandle,
    tv: &TvGuideState,
) -> String {
    match (app_mode, classic_view) {
        (AppMode::Desktop, _) => "SYS://DESKTOP".to_string(),
        (_, ClassicView::Dashboard) => "SYS://DASHBOARD".to_string(),
        (_, ClassicView::Terminal) => "SYS://TERMINAL".to_string(),
        (_, ClassicView::FileManager) => {
            let active_path = if fm.active_panel == 0 {
                &fm.left.path
            } else {
                &fm.right.path
            };
            let path_part = if active_path.len() > 14 {
                let start = active_path.ceil_char_boundary(active_path.len() - 14);
                &active_path[start..]
            } else {
                active_path.as_str()
            };
            if umd_activated {
                format!("UMD:{}", path_part)
            } else {
                format!("MSO:/{}", path_part)
            }
        },
        (_, ClassicView::PhotoViewer) => "SYS://PHOTOS".to_string(),
        (_, ClassicView::MusicPlayer) => {
            if audio.is_playing() {
                "SYS://NOW_PLAY".to_string()
            } else {
                "SYS://MUSIC".to_string()
            }
        },
        (_, ClassicView::Browser) => "SYS://BROWSER".to_string(),
        (_, ClassicView::Radio) => {
            if audio.is_radio_streaming() {
                "SYS://RADIO_ON".to_string()
            } else {
                "SYS://RADIO".to_string()
            }
        },
        (_, ClassicView::TvGuide) => {
            if tv.tuned.is_some() {
                "SYS://TV_LIVE".to_string()
            } else {
                "SYS://TV_GUIDE".to_string()
            }
        },
    }
}
