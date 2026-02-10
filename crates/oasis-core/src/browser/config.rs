//! Browser configuration and skin feature gates.

use crate::backend::Color;

/// Browser feature configuration (from skin features.toml).
#[derive(Debug, Clone)]
pub struct BrowserFeatures {
    /// Show browser icon on dashboard.
    pub enabled: bool,
    /// Use built-in HTML engine.
    pub native_engine: bool,
    /// Delegate to WebKitGTK (desktop only).
    pub webkit_delegation: bool,
    /// Enable Gemini protocol.
    pub gemini: bool,
    /// Enable reader mode toggle.
    pub reader_mode: bool,
    /// Block all network requests (force VFS).
    pub sandbox_only: bool,
    /// Home page URL.
    pub home_url: String,
    /// Resource cache limit in MB.
    pub max_cache_mb: usize,
}

impl Default for BrowserFeatures {
    fn default() -> Self {
        Self {
            enabled: true,
            native_engine: true,
            webkit_delegation: false,
            gemini: true,
            reader_mode: true,
            sandbox_only: false,
            home_url: "vfs://sites/home/index.html".to_string(),
            max_cache_mb: 2,
        }
    }
}

/// Visual configuration for browser chrome.
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    pub features: BrowserFeatures,

    // Chrome dimensions
    pub url_bar_height: u32,
    pub status_bar_height: u32,
    pub button_width: u32,

    // Chrome colors
    pub chrome_bg: Color,
    pub chrome_text: Color,
    pub chrome_button_bg: Color,
    pub chrome_button_hover: Color,
    pub url_bar_bg: Color,
    pub url_bar_text: Color,
    pub status_bar_bg: Color,
    pub status_bar_text: Color,

    // Page defaults
    pub default_font_size: f32,
    pub default_text_color: Color,
    pub default_bg_color: Color,
    pub default_link_color: Color,
    pub default_visited_color: Color,

    // Scroll
    pub smooth_scroll: bool,
    pub scroll_line_px: i32,

    // Limits
    pub max_redirects: u8,
    pub max_image_dimension: u32,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            features: BrowserFeatures::default(),
            url_bar_height: 20,
            status_bar_height: 14,
            button_width: 20,
            chrome_bg: Color::rgb(48, 48, 48),
            chrome_text: Color::rgb(200, 200, 200),
            chrome_button_bg: Color::rgb(64, 64, 64),
            chrome_button_hover: Color::rgb(80, 80, 80),
            url_bar_bg: Color::rgb(32, 32, 32),
            url_bar_text: Color::rgb(220, 220, 220),
            status_bar_bg: Color::rgb(40, 40, 40),
            status_bar_text: Color::rgb(160, 160, 160),
            default_font_size: 12.0,
            default_text_color: Color::rgb(0, 0, 0),
            default_bg_color: Color::rgb(255, 255, 255),
            default_link_color: Color::rgb(0, 102, 204),
            default_visited_color: Color::rgb(85, 26, 139),
            smooth_scroll: false,
            scroll_line_px: 24,
            max_redirects: 5,
            max_image_dimension: 480,
        }
    }
}

impl BrowserConfig {
    /// Cache size in bytes.
    pub fn cache_size_bytes(&self) -> usize {
        self.features.max_cache_mb * 1024 * 1024
    }

    /// Content area height (viewport minus chrome).
    pub fn content_height(&self, window_height: u32) -> u32 {
        window_height
            .saturating_sub(self.url_bar_height)
            .saturating_sub(self.status_bar_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_are_sensible() {
        let cfg = BrowserConfig::default();
        assert!(cfg.features.enabled);
        assert!(cfg.features.native_engine);
        assert!(!cfg.features.webkit_delegation);
        assert!(cfg.features.gemini);
        assert!(cfg.features.reader_mode);
        assert!(!cfg.features.sandbox_only);
        assert_eq!(cfg.features.home_url, "vfs://sites/home/index.html");
        assert_eq!(cfg.features.max_cache_mb, 2);
        assert_eq!(cfg.url_bar_height, 20);
        assert_eq!(cfg.status_bar_height, 14);
        assert_eq!(cfg.button_width, 20);
        assert!((cfg.default_font_size - 12.0).abs() < f32::EPSILON);
        assert_eq!(cfg.max_redirects, 5);
        assert_eq!(cfg.max_image_dimension, 480);
        assert!(!cfg.smooth_scroll);
        assert_eq!(cfg.scroll_line_px, 24);
    }

    #[test]
    fn cache_size_bytes_calculation() {
        let cfg = BrowserConfig::default();
        // 2 MB = 2 * 1024 * 1024 = 2_097_152
        assert_eq!(cfg.cache_size_bytes(), 2 * 1024 * 1024);
    }

    #[test]
    fn content_height_calculation() {
        let cfg = BrowserConfig::default();
        // url_bar_height=20, status_bar_height=14 => 34 total chrome
        assert_eq!(cfg.content_height(272), 272 - 20 - 14);
        // Window smaller than chrome: saturating_sub prevents underflow.
        assert_eq!(cfg.content_height(10), 0);
    }
}
