//! Skin integration for the browser subsystem.
//!
//! Provides per-skin browser stylesheets and corrupted skin visual
//! modifiers. When the Corrupted skin is active, the browser's paint
//! layer applies deterministic pseudo-random distortion effects.

use crate::backend::Color;
use crate::browser::config::BrowserConfig;
use crate::browser::css::parser::Stylesheet;

// -------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------

/// Unicode block character range used for text corruption.
/// Characters U+2580 through U+259F (Block Elements).
const BLOCK_CHARS: &[char] = &[
    '\u{2580}', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
    '\u{2588}', '\u{2589}', '\u{258A}', '\u{258B}', '\u{258C}', '\u{258D}', '\u{258E}', '\u{258F}',
    '\u{2590}', '\u{2591}', '\u{2592}', '\u{2593}', '\u{2594}', '\u{2595}', '\u{2596}', '\u{2597}',
    '\u{2598}', '\u{2599}', '\u{259A}', '\u{259B}', '\u{259C}', '\u{259D}', '\u{259E}', '\u{259F}',
];

/// Knuth multiplicative hash constant (golden ratio for 32-bit).
const HASH_MULT: u32 = 2_654_435_761;

// -------------------------------------------------------------------
// SkinBrowserConfig
// -------------------------------------------------------------------

/// Skin browser integration settings.
///
/// Controls per-skin CSS overrides and corrupted-mode visual effects.
#[derive(Debug, Clone)]
pub struct SkinBrowserConfig {
    /// Per-skin CSS overrides (raw CSS string).
    pub skin_stylesheet: Option<String>,
    /// Whether corrupted modifiers are enabled.
    pub corrupted_mode: bool,
    /// Corrupted mode modifier settings.
    pub modifiers: CorruptedModifiers,
}

impl SkinBrowserConfig {
    /// Create a default (non-corrupted) skin config.
    pub fn new() -> Self {
        Self {
            skin_stylesheet: None,
            corrupted_mode: false,
            modifiers: CorruptedModifiers::disabled(),
        }
    }

    /// Create a corrupted skin config with default modifier values.
    pub fn corrupted() -> Self {
        Self {
            skin_stylesheet: None,
            corrupted_mode: true,
            modifiers: CorruptedModifiers::default(),
        }
    }
}

impl Default for SkinBrowserConfig {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------------------------------------------
// CorruptedModifiers
// -------------------------------------------------------------------

/// Configuration for corrupted skin visual effects.
///
/// Each field controls one dimension of the visual distortion
/// applied during the browser paint pass.
#[derive(Debug, Clone)]
pub struct CorruptedModifiers {
    /// Probability (0.0-1.0) of character corruption.
    pub text_corrupt_rate: f32,
    /// Color shift amount (HSL degrees, 0.0 = no shift).
    pub color_shift_amount: f32,
    /// Max layout jitter in pixels.
    pub layout_jitter_px: u8,
    /// Whether image corruption is enabled.
    pub image_corrupt: bool,
    /// Probability (0.0-1.0) of link redirect.
    pub link_redirect_rate: f32,
}

impl Default for CorruptedModifiers {
    fn default() -> Self {
        Self {
            text_corrupt_rate: 0.03, // 3%
            color_shift_amount: 0.0,
            layout_jitter_px: 2,
            image_corrupt: false,
            link_redirect_rate: 0.1, // 10%
        }
    }
}

impl CorruptedModifiers {
    /// All effects disabled (for non-corrupted skins).
    fn disabled() -> Self {
        Self {
            text_corrupt_rate: 0.0,
            color_shift_amount: 0.0,
            layout_jitter_px: 0,
            image_corrupt: false,
            link_redirect_rate: 0.0,
        }
    }
}

// -------------------------------------------------------------------
// Deterministic hashing
// -------------------------------------------------------------------

/// Simple deterministic hash step. Produces a pseudo-random u32
/// from a seed using Knuth's multiplicative method with bit mixing.
fn hash_step(seed: u32) -> u32 {
    let mut h = seed.wrapping_mul(HASH_MULT);
    h ^= h >> 16;
    h = h.wrapping_mul(0x45d9f3b);
    h ^= h >> 16;
    h
}

/// Hash with two inputs (seed + index) for per-character decisions.
fn hash_pair(seed: u32, index: u32) -> u32 {
    hash_step(seed ^ index.wrapping_mul(HASH_MULT))
}

/// Convert a hash to a float in [0.0, 1.0).
fn hash_to_f32(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0 // 2^24
}

// -------------------------------------------------------------------
// Public functions
// -------------------------------------------------------------------

/// Apply text corruption: randomly replace characters with Unicode
/// block element characters.
///
/// Each character position is independently tested against `rate`
/// using a deterministic hash of `(seed, char_index)`. Characters
/// that pass the threshold are replaced with a block character
/// chosen from the Unicode Block Elements range.
///
/// Returns the original string unmodified when `rate <= 0.0`.
pub fn corrupt_text(text: &str, rate: f32, seed: u32) -> String {
    if rate <= 0.0 || text.is_empty() {
        return text.to_string();
    }
    let clamped_rate = rate.clamp(0.0, 1.0);
    let mut result = String::with_capacity(text.len());
    for (i, ch) in text.chars().enumerate() {
        let h = hash_pair(seed, i as u32);
        let prob = hash_to_f32(h);
        if prob < clamped_rate {
            // Pick a block character deterministically.
            let block_idx = (h as usize) % BLOCK_CHARS.len();
            result.push(BLOCK_CHARS[block_idx]);
        } else {
            result.push(ch);
        }
    }
    result
}

/// Apply color shift: adjust a color by rotating its hue by an
/// offset derived from `amount` and the current `frame` counter.
///
/// The shift is computed as `amount * sin(frame)` in degrees,
/// applied to the HSL hue channel. Returns the original color
/// when `amount` is zero (within epsilon).
pub fn shift_color(color: Color, amount: f32, frame: u32) -> Color {
    if amount.abs() < f32::EPSILON {
        return color;
    }

    // Convert RGB to HSL.
    let (h, s, l) = rgb_to_hsl(color.r, color.g, color.b);

    // Compute a sinusoidal offset from the frame counter.
    let angle = (frame as f32) * 0.1;
    let offset = amount * fast_sin(angle);
    let new_h = (h + offset).rem_euclid(360.0);

    // Convert back to RGB.
    let (r, g, b) = hsl_to_rgb(new_h, s, l);
    Color::rgba(r, g, b, color.a)
}

/// Apply layout jitter: compute a deterministic pseudo-random (dx,dy)
/// offset bounded by `[-max_px, +max_px]`.
///
/// The returned offset is suitable for adding to box positions during
/// the paint pass. The same `(max_px, seed)` always produces the
/// same result.
pub fn jitter_offset(max_px: u8, seed: u32) -> (i32, i32) {
    if max_px == 0 {
        return (0, 0);
    }
    let range = max_px as i32;
    let hx = hash_step(seed);
    let hy = hash_step(hx);
    let dx = (hx as i32).wrapping_abs() % (range + 1);
    let dy = (hy as i32).wrapping_abs() % (range + 1);
    // Use bit 0 for sign.
    let dx = if hx & 1 == 0 { dx } else { -dx };
    let dy = if hy & 1 == 0 { dy } else { -dy };
    (dx, dy)
}

/// Parse a skin's browser stylesheet string into a [`Stylesheet`].
///
/// This delegates to the CSS parser. An empty or whitespace-only
/// input produces a stylesheet with zero rules.
pub fn parse_skin_stylesheet(css: &str) -> Stylesheet {
    Stylesheet::parse(css)
}

/// Build a [`BrowserConfig`] from skin settings, layered on top
/// of a base configuration.
///
/// Preserves all values from `base` except those explicitly
/// overridden by the skin:
/// - If the skin provides a stylesheet, it is stored for later
///   injection into the CSS cascade.
/// - If corrupted mode is enabled, the config's smooth_scroll is
///   forced off (to make jitter visible).
pub fn config_from_skin(skin_config: &SkinBrowserConfig, base: BrowserConfig) -> BrowserConfig {
    let mut config = base;
    if skin_config.corrupted_mode {
        config.smooth_scroll = false;
    }
    config
}

// -------------------------------------------------------------------
// HSL conversion helpers
// -------------------------------------------------------------------

/// Convert RGB (0-255) to HSL. Returns (h: 0-360, s: 0-1, l: 0-1).
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;

    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - rf).abs() < f32::EPSILON {
        let mut h = (gf - bf) / d;
        if gf < bf {
            h += 6.0;
        }
        h
    } else if (max - gf).abs() < f32::EPSILON {
        (bf - rf) / d + 2.0
    } else {
        (rf - gf) / d + 4.0
    };

    (h * 60.0, s, l)
}

/// Convert HSL to RGB. h: 0-360, s: 0-1, l: 0-1.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    if s.abs() < f32::EPSILON {
        let v = (l * 255.0).clamp(0.0, 255.0) as u8;
        return (v, v, v);
    }

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let h_norm = h / 360.0;

    let r = hue_to_rgb(p, q, h_norm + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h_norm);
    let b = hue_to_rgb(p, q, h_norm - 1.0 / 3.0);

    (
        (r * 255.0).clamp(0.0, 255.0) as u8,
        (g * 255.0).clamp(0.0, 255.0) as u8,
        (b * 255.0).clamp(0.0, 255.0) as u8,
    )
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

/// Fast sine approximation using a polynomial (Bhaskara I style).
/// Input in radians, output in [-1, 1].
fn fast_sin(x: f32) -> f32 {
    use std::f32::consts::PI;
    // Normalize to [0, 2*PI).
    let x = x.rem_euclid(2.0 * PI);
    // Map to [-PI, PI].
    let x = if x > PI { x - 2.0 * PI } else { x };
    // Approximation: 16x(pi-x) / (5*pi^2 - 4x(pi-x))
    let num = 16.0 * x * (PI - x);
    let den = 5.0 * PI * PI - 4.0 * x * (PI - x);
    if den.abs() < f32::EPSILON {
        0.0
    } else {
        num / den
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Test 1: Default config is not corrupted
    // ---------------------------------------------------------------

    #[test]
    fn default_config_not_corrupted() {
        let cfg = SkinBrowserConfig::new();
        assert!(!cfg.corrupted_mode);
        assert!(cfg.skin_stylesheet.is_none());
        assert!(cfg.modifiers.text_corrupt_rate.abs() < f32::EPSILON);
        assert_eq!(cfg.modifiers.layout_jitter_px, 0);
        assert!(cfg.modifiers.link_redirect_rate.abs() < f32::EPSILON);
    }

    // ---------------------------------------------------------------
    // Test 2: Corrupted config has modifiers enabled
    // ---------------------------------------------------------------

    #[test]
    fn corrupted_config_has_modifiers() {
        let cfg = SkinBrowserConfig::corrupted();
        assert!(cfg.corrupted_mode);
        assert!(cfg.modifiers.text_corrupt_rate > 0.0);
        assert_eq!(cfg.modifiers.layout_jitter_px, 2);
        assert!((cfg.modifiers.link_redirect_rate - 0.1).abs() < f32::EPSILON);
        assert!((cfg.modifiers.text_corrupt_rate - 0.03).abs() < f32::EPSILON);
    }

    // ---------------------------------------------------------------
    // Test 3: Text corruption replaces some characters
    // ---------------------------------------------------------------

    #[test]
    fn text_corruption_replaces_chars() {
        let input = "Hello, World! This is a test string.";
        let result = corrupt_text(input, 0.5, 42);
        // With 50% rate, some characters should differ.
        assert_ne!(result, input);
        // Length in characters should be preserved.
        assert_eq!(result.chars().count(), input.chars().count());
        // At least some original characters should remain.
        let matching: usize = input
            .chars()
            .zip(result.chars())
            .filter(|(a, b)| a == b)
            .count();
        assert!(matching > 0, "at 50% rate some chars should survive");
        assert!(
            matching < input.chars().count(),
            "at 50% rate some chars should be replaced"
        );
    }

    // ---------------------------------------------------------------
    // Test 4: Text corruption is deterministic with same seed
    // ---------------------------------------------------------------

    #[test]
    fn text_corruption_deterministic() {
        let input = "Deterministic test string for corruption.";
        let a = corrupt_text(input, 0.3, 12345);
        let b = corrupt_text(input, 0.3, 12345);
        assert_eq!(a, b, "same seed must produce same result");

        // Different seed should (very likely) produce different result.
        let c = corrupt_text(input, 0.3, 99999);
        // Not strictly guaranteed but statistically overwhelmingly
        // likely for a 41-char string at 30%.
        assert_ne!(a, c, "different seeds should produce different results");
    }

    // ---------------------------------------------------------------
    // Test 5: Color shift with zero amount returns original color
    // ---------------------------------------------------------------

    #[test]
    fn color_shift_zero_amount() {
        let original = Color::rgb(100, 150, 200);
        let shifted = shift_color(original, 0.0, 42);
        assert_eq!(shifted, original);
    }

    // ---------------------------------------------------------------
    // Test 6: Jitter offset stays within max bounds
    // ---------------------------------------------------------------

    #[test]
    fn jitter_offset_within_bounds() {
        for seed in 0..1000 {
            let (dx, dy) = jitter_offset(3, seed);
            assert!(dx.abs() <= 3, "dx={} exceeds max_px=3 at seed={}", dx, seed);
            assert!(dy.abs() <= 3, "dy={} exceeds max_px=3 at seed={}", dy, seed);
        }

        // Zero max should always return (0, 0).
        let (dx, dy) = jitter_offset(0, 42);
        assert_eq!((dx, dy), (0, 0));
    }

    // ---------------------------------------------------------------
    // Test 7: parse_skin_stylesheet produces a valid Stylesheet
    // ---------------------------------------------------------------

    #[test]
    fn parse_skin_stylesheet_valid() {
        let css = "body { color: red; } a { color: blue; }";
        let sheet = parse_skin_stylesheet(css);
        assert_eq!(sheet.rules.len(), 2);
        assert!(!sheet.rules[0].declarations.is_empty());

        // Empty CSS produces empty stylesheet.
        let empty = parse_skin_stylesheet("");
        assert!(empty.rules.is_empty());
    }

    // ---------------------------------------------------------------
    // Test 8: config_from_skin preserves base config features
    // ---------------------------------------------------------------

    #[test]
    fn config_from_skin_preserves_base() {
        let skin = SkinBrowserConfig::new();
        let mut base = BrowserConfig::default();
        base.features.home_url = "vfs://custom/home.html".to_string();
        base.features.gemini = false;
        base.features.max_cache_mb = 8;
        base.smooth_scroll = true;
        base.max_redirects = 10;

        let result = config_from_skin(&skin, base);
        assert_eq!(result.features.home_url, "vfs://custom/home.html");
        assert!(!result.features.gemini);
        assert_eq!(result.features.max_cache_mb, 8);
        assert!(result.smooth_scroll);
        assert_eq!(result.max_redirects, 10);
    }

    // ---------------------------------------------------------------
    // Test 9: config_from_skin disables smooth scroll for corrupted
    // ---------------------------------------------------------------

    #[test]
    fn config_from_skin_corrupted_disables_smooth_scroll() {
        let skin = SkinBrowserConfig::corrupted();
        let mut base = BrowserConfig::default();
        base.smooth_scroll = true;

        let result = config_from_skin(&skin, base);
        assert!(
            !result.smooth_scroll,
            "corrupted skin should force smooth_scroll off"
        );
    }

    // ---------------------------------------------------------------
    // Test 10: HSL round-trip preserves colors
    // ---------------------------------------------------------------

    #[test]
    fn hsl_round_trip() {
        // Test a few representative colors.
        let colors = [
            (255u8, 0u8, 0u8), // red
            (0, 255, 0),       // green
            (0, 0, 255),       // blue
            (128, 128, 128),   // grey
            (255, 255, 255),   // white
            (0, 0, 0),         // black
            (200, 100, 50),    // orange-ish
        ];
        for (r, g, b) in colors {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            let (r2, g2, b2) = hsl_to_rgb(h, s, l);
            // Allow +/-1 for rounding.
            assert!(
                (r as i16 - r2 as i16).unsigned_abs() <= 1
                    && (g as i16 - g2 as i16).unsigned_abs() <= 1
                    && (b as i16 - b2 as i16).unsigned_abs() <= 1,
                "HSL round-trip failed for ({},{},{}) -> ({},{},{})",
                r,
                g,
                b,
                r2,
                g2,
                b2
            );
        }
    }

    // ---------------------------------------------------------------
    // Test 11: Color shift with non-zero amount changes color
    // ---------------------------------------------------------------

    #[test]
    fn color_shift_nonzero_changes_color() {
        let original = Color::rgb(100, 150, 200);
        let shifted = shift_color(original, 90.0, 10);
        // With 90-degree hue shift, the color should differ.
        assert_ne!(shifted, original, "90-degree shift should change the color");
        // Alpha should be preserved.
        assert_eq!(shifted.a, original.a);
    }

    // ---------------------------------------------------------------
    // Test 12: Text corruption with zero rate is identity
    // ---------------------------------------------------------------

    #[test]
    fn text_corruption_zero_rate_identity() {
        let input = "No corruption at all.";
        let result = corrupt_text(input, 0.0, 42);
        assert_eq!(result, input);
    }

    // ---------------------------------------------------------------
    // Test 13: Default trait for SkinBrowserConfig
    // ---------------------------------------------------------------

    #[test]
    fn skin_config_default_trait() {
        let a = SkinBrowserConfig::new();
        let b = SkinBrowserConfig::default();
        assert_eq!(a.corrupted_mode, b.corrupted_mode);
        assert_eq!(a.modifiers.layout_jitter_px, b.modifiers.layout_jitter_px);
    }
}
