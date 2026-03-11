//! TV Guide grid state and SDI rendering.
//!
//! Manages the retro cable-TV EPG layout: header bar with current channel
//! info, time-slot column headers, channel rows with variable-width
//! program cells, selection highlight, and footer navigation hints.

pub use crate::grid_layout::TvGuideColors;
pub use crate::grid_render;
pub use crate::grid_state::{TuneRequest, TvGuideState};

#[cfg(test)]
mod tests {
    use crate::grid_layout::{VISIBLE_ROWS, truncate_title};
    use crate::grid_state::TvGuideState;

    use oasis_skin::active_theme::ActiveTheme;

    use crate::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};

    #[test]
    fn new_guide_state() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.channels.len(), 5);
        assert_eq!(state.catalogs.len(), 5);
        assert_eq!(state.selected_channel, 0);
        assert_eq!(state.scroll_offset, 0);
        assert!(state.tuned_channel.is_none());
    }

    #[test]
    fn navigation() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        state.select_down();
        assert_eq!(state.selected_channel, 1);
        state.select_down();
        assert_eq!(state.selected_channel, 2);
        state.select_up();
        assert_eq!(state.selected_channel, 1);
        state.select_up();
        assert_eq!(state.selected_channel, 0);
        state.select_up(); // Should not go below 0.
        assert_eq!(state.selected_channel, 0);
    }

    #[test]
    fn navigation_bounds() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        for _ in 0..100 {
            state.select_down();
        }
        assert_eq!(state.selected_channel, 4); // 5 channels, max index 4.
    }

    #[test]
    fn navigation_auto_scroll() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        // 5 channels, VISIBLE_ROWS=5, so no scroll needed.
        for _ in 0..4 {
            state.select_down();
        }
        assert_eq!(state.selected_channel, 4);
        assert_eq!(state.scroll_offset, 0);

        // Go back up.
        state.select_up();
        assert_eq!(state.selected_channel, 3);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn paging_methods() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.current_page(), 1);
        assert_eq!(state.total_pages(), 1);
    }

    #[test]
    fn paging_empty_channels() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.current_page(), 1);
        assert_eq!(state.total_pages(), 1);
    }

    #[test]
    fn time_scroll() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        let start = state.grid_start_time();
        state.scroll_right();
        let after_right = state.grid_start_time();
        assert_eq!(after_right, start + 1800);
        state.scroll_left();
        let after_left = state.grid_start_time();
        assert_eq!(after_left, start);
    }

    #[test]
    fn text_content_no_catalogs() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("TV Guide")));
        assert!(lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn tune_without_catalog() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        assert!(state.tune().is_none());
    }

    #[test]
    fn truncate_title_short() {
        assert_eq!(truncate_title("Hello", 10), "Hello");
    }

    #[test]
    fn truncate_title_exact() {
        assert_eq!(truncate_title("Hello", 5), "Hello");
    }

    #[test]
    fn truncate_title_overflow() {
        assert_eq!(truncate_title("Hello World", 7), "Hello..");
    }

    #[test]
    fn truncate_title_tiny() {
        assert_eq!(truncate_title("Hello", 2), "He");
    }

    #[test]
    fn truncate_title_zero() {
        assert_eq!(truncate_title("Hello", 0), "");
    }

    #[test]
    fn text_content_with_fetch_error() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        state.fetch_attempted = true;
        state.fetch_error = Some("network timeout".to_string());
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("Error: network timeout")));
        assert!(!lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn text_content_after_partial_load() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        state.fetch_attempted = true;

        // Inject a catalog for channel 0.
        let mut catalog = crate::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![crate::catalog::VideoEpisode {
            item_id: "test-item".to_string(),
            filename: "ep1.mp4".to_string(),
            title: "Test Episode".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
            format: "MPEG4".into(),
            original: None,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        let lines = state.text_content();
        // Should not show "Loading" — at least one catalog loaded.
        assert!(!lines.iter().any(|l| l.contains("Loading")));
        // Channel 0 should show the episode title.
        assert!(lines.iter().any(|l| l.contains("Test Episode")));
    }

    #[test]
    fn fetch_attempted_prevents_refetch_text() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        // Before fetch_attempted: shows loading.
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("Loading")));

        // After fetch_attempted with all None: shows no content.
        state.fetch_attempted = true;
        let lines = state.text_content();
        assert!(lines.iter().any(|l| l.contains("No content")));
        assert!(!lines.iter().any(|l| l.contains("Loading")));
    }

    #[test]
    fn rebuild_cached_schedule_after_catalog() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        let mut catalog = crate::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![crate::catalog::VideoEpisode {
            item_id: "test-item".to_string(),
            filename: "ep1.mp4".to_string(),
            title: "Test Episode".to_string(),
            duration_secs: 3600.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
            format: "MPEG4".into(),
            original: None,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        // Tune should now work for channel 0.
        let req = state.tune();
        assert!(req.is_some());
        let req = req.unwrap();
        assert_eq!(req.episode.title, "Test Episode");
    }

    #[test]
    fn now_playing_detail_with_content() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        let mut catalog = crate::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![crate::catalog::VideoEpisode {
            item_id: "test".to_string(),
            filename: "ep.mp4".to_string(),
            title: "Test".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
            format: "MPEG4".into(),
            original: None,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        let detail = state.build_now_playing_detail();
        assert!(detail.contains("640x480"));
        assert!(detail.contains("remaining"));
    }

    #[test]
    fn now_playing_detail_empty() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        let detail = state.build_now_playing_detail();
        assert!(detail.is_empty());
    }

    #[test]
    fn scroll_offset_with_many_channels() {
        // Build a config with 12 channels.
        let mut toml_str = String::new();
        for i in 0..12 {
            toml_str.push_str(&format!(
                "[[channel]]\nnumber = {}\ncall_sign = \"C{i}\"\n\
                 name = \"Channel {i}\"\ngenre = \"test\"\n\
                 [[channel.source]]\nitem_id = \"test-{i}\"\n\n",
                i + 1,
            ));
        }
        let config: ChannelConfig = toml::from_str(&toml_str).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.channels.len(), 12);

        // Navigate down past VISIBLE_ROWS.
        for _ in 0..7 {
            state.select_down();
        }
        assert_eq!(state.selected_channel, 7);
        // scroll_offset should have adjusted so selected is visible.
        assert!(state.selected_channel < state.scroll_offset + VISIBLE_ROWS);
        assert!(state.selected_channel >= state.scroll_offset);

        // Navigate back up.
        for _ in 0..7 {
            state.select_up();
        }
        assert_eq!(state.selected_channel, 0);
        assert_eq!(state.scroll_offset, 0);
    }

    // -- handle_click tests --

    /// Compute grid_y for a given content height (mirrors handle_click layout).
    fn grid_y_for(ch: u32) -> u32 {
        let usable_h = ch;
        let header_h = (usable_h * 20 / 100).max(60);
        let time_header_h = (usable_h * 4 / 100).max(20);
        header_h + time_header_h
    }

    /// Compute row height for a given content height and channel count.
    fn row_h_for(ch: u32, num_channels: usize) -> u32 {
        let usable_h = ch;
        let header_h = (usable_h * 20 / 100).max(60);
        let time_header_h = (usable_h * 4 / 100).max(20);
        let footer_h = (usable_h * 5 / 100).max(18);
        let grid_h = usable_h.saturating_sub(header_h + time_header_h + footer_h);
        let row_count = num_channels.clamp(1, VISIBLE_ROWS) as u32;
        (grid_h / row_count).max(20)
    }

    #[test]
    fn click_selects_channel() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.selected_channel, 0);

        let ch = 600u32;
        let cw = 800u32;
        let gy = grid_y_for(ch);
        let rh = row_h_for(ch, state.channels.len());

        // Click on row 2 (third channel).
        let ly = (gy + rh * 2 + rh / 2) as i32;
        let result = state.handle_click(100, ly, cw, ch, true);
        assert!(result.is_none(), "first click should select, not tune");
        assert_eq!(state.selected_channel, 2);
    }

    #[test]
    fn click_already_selected_tunes() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        // Inject a catalog so tune() can succeed.
        let mut catalog = crate::catalog::ChannelCatalog::new(state.channels[0].number);
        catalog.add_episodes(vec![crate::catalog::VideoEpisode {
            item_id: "click-test".to_string(),
            filename: "ep.mp4".to_string(),
            title: "Click Test".to_string(),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 1000,
            format: "MPEG4".into(),
            original: None,
        }]);
        state.catalogs[0] = Some(catalog);
        state.rebuild_cached_schedule(0);

        let ch = 600u32;
        let cw = 800u32;
        let gy = grid_y_for(ch);
        let rh = row_h_for(ch, state.channels.len());

        // Channel 0 is already selected. Click on row 0.
        let ly = (gy + rh / 2) as i32;
        let result = state.handle_click(100, ly, cw, ch, true);
        assert!(result.is_some(), "second click on selected should tune");
        assert_eq!(result.unwrap().episode.title, "Click Test");
    }

    #[test]
    fn click_outside_grid_ignored() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());

        let ch = 600u32;
        let cw = 800u32;

        // Click in header area (y=10, well above grid).
        let result = state.handle_click(100, 10, cw, ch, true);
        assert!(result.is_none());
        assert_eq!(state.selected_channel, 0);

        // Click below grid area (y=ch-1, in footer).
        let result = state.handle_click(100, ch as i32 - 1, cw, ch, true);
        assert!(result.is_none());
        assert_eq!(state.selected_channel, 0);
    }

    #[test]
    fn click_negative_coords_ignored() {
        let config = ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        let result = state.handle_click(-10, -5, 800, 600, true);
        assert!(result.is_none());
        assert_eq!(state.selected_channel, 0);
    }

    #[test]
    fn click_empty_channels_ignored() {
        let config: ChannelConfig = toml::from_str("channel = []").unwrap();
        let mut state = TvGuideState::new(&config, &ActiveTheme::default());
        let result = state.handle_click(100, 200, 800, 600, true);
        assert!(result.is_none());
    }

    #[test]
    fn paging_with_many_channels() {
        let mut toml_str = String::new();
        for i in 0..12 {
            toml_str.push_str(&format!(
                "[[channel]]\nnumber = {}\ncall_sign = \"C{i}\"\n\
                 name = \"Ch {i}\"\ngenre = \"t\"\n\
                 [[channel.source]]\nitem_id = \"t-{i}\"\n\n",
                i + 1,
            ));
        }
        let config: ChannelConfig = toml::from_str(&toml_str).unwrap();
        let state = TvGuideState::new(&config, &ActiveTheme::default());
        assert_eq!(state.total_pages(), 3); // ceil(12/5) = 3
        assert_eq!(state.current_page(), 1);
    }
}
