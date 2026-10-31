//! Core types: Color, TextureId, RenderTargetId, BlendMode, GradientStyle and the helpers the trait defaults share.

use crate::error::Result;

/// Measure text width using proportional bitmap font metrics.
///
/// Sums per-character advance widths derived from the actual glyph ink bounds,
/// then scales by the font-size multiplier. This produces tighter text than
/// the old fixed `8 * len` calculation.
///
/// Uses a fast path for ASCII-only text that avoids per-char function call
/// overhead by summing raw advance values from the metrics table directly.
pub fn bitmap_measure_text(text: &str, font_size: u16) -> u32 {
    let fs = font_size.max(1) as u32;

    // Fast path: ASCII-only text (common for English, URLs, code).
    // Sum scaled advance widths directly from bytes, avoiding char
    // decoding and per-char function call overhead. Each advance is
    // scaled individually to match the rounding behavior of glyph_advance_scaled.
    if text.is_ascii() {
        let mut total: u32 = 0;
        for &b in text.as_bytes() {
            let advance = crate::bitmap_font::glyph_metrics_ascii(b).1 as u32;
            total += advance * fs / 8;
        }
        return total;
    }

    // Slow path: mixed Unicode text.
    text.chars()
        .map(|ch| crate::bitmap_font::glyph_advance_scaled(ch, font_size))
        .sum()
}

/// A color in RGBA format (0-255 per channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Return the same color with a different alpha value.
    pub const fn with_alpha(self, a: u8) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a,
        }
    }

    /// Darken the color by a factor (0.0 = black, 1.0 = unchanged).
    pub fn darken(self, factor: f32) -> Self {
        let f = factor.clamp(0.0, 1.0);
        Self::rgba(
            (self.r as f32 * f) as u8,
            (self.g as f32 * f) as u8,
            (self.b as f32 * f) as u8,
            self.a,
        )
    }

    /// Lighten the color by blending toward white (0.0 = unchanged, 1.0 = white).
    pub fn lighten(self, factor: f32) -> Self {
        self.lerp(Self::WHITE, factor)
    }

    /// Linearly interpolate between `self` and `other`.
    ///
    /// `t` is clamped to `[0.0, 1.0]`. Returns `self` when `t == 0.0` and
    /// `other` when `t == 1.0`.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self::rgba(
            (self.r as f32 + (other.r as f32 - self.r as f32) * t) as u8,
            (self.g as f32 + (other.g as f32 - self.g as f32) * t) as u8,
            (self.b as f32 + (other.b as f32 - self.b as f32) * t) as u8,
            (self.a as f32 + (other.a as f32 - self.a as f32) * t) as u8,
        )
    }

    /// Scale the alpha channel by an opacity factor (0.0..=1.0).
    ///
    /// Returns the color unchanged when `opacity >= 1.0`.
    pub fn apply_opacity(self, opacity: f32) -> Self {
        if opacity >= 1.0 {
            return self;
        }
        Self::rgba(self.r, self.g, self.b, (self.a as f32 * opacity) as u8)
    }

    /// Alpha-blend `self` (foreground) over `dst` (background).
    ///
    /// Uses the standard Porter-Duff "source over" compositing formula.
    pub fn alpha_over(self, dst: Self) -> Self {
        let sa = self.a as u32;
        if sa == 0 {
            return dst;
        }
        if sa == 255 {
            return self;
        }
        let da = dst.a as u32;
        let out_a = sa + da * (255 - sa) / 255;
        if out_a == 0 {
            return Self::TRANSPARENT;
        }
        let r = (self.r as u32 * sa + dst.r as u32 * da * (255 - sa) / 255) / out_a;
        let g = (self.g as u32 * sa + dst.g as u32 * da * (255 - sa) / 255) / out_a;
        let b = (self.b as u32 * sa + dst.b as u32 * da * (255 - sa) / 255) / out_a;
        Self::rgba(
            r.min(255) as u8,
            g.min(255) as u8,
            b.min(255) as u8,
            out_a.min(255) as u8,
        )
    }

    /// Pack into ABGR `u32` (alpha in high byte, red in low byte).
    ///
    /// This is the native format for the PSP Graphics Engine (GU).
    pub const fn to_abgr(self) -> u32 {
        (self.a as u32) << 24 | (self.b as u32) << 16 | (self.g as u32) << 8 | (self.r as u32)
    }

    /// Decode an ABGR `u32` back to `Color` (inverse of [`Self::to_abgr`]).
    pub const fn from_abgr(abgr: u32) -> Self {
        Self {
            r: abgr as u8,
            g: (abgr >> 8) as u8,
            b: (abgr >> 16) as u8,
            a: (abgr >> 24) as u8,
        }
    }

    /// Pack into ARGB `u32` (alpha in high byte, blue in low byte).
    pub const fn to_argb(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | (self.b as u32)
    }

    /// Decode an ARGB `u32` back to `Color` (inverse of [`Self::to_argb`]).
    pub const fn from_argb(argb: u32) -> Self {
        Self {
            a: (argb >> 24) as u8,
            r: (argb >> 16) as u8,
            g: (argb >> 8) as u8,
            b: argb as u8,
        }
    }

    /// Pack into RGBA `u32` (red in high byte, alpha in low byte).
    pub const fn to_rgba_u32(self) -> u32 {
        (self.r as u32) << 24 | (self.g as u32) << 16 | (self.b as u32) << 8 | (self.a as u32)
    }

    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
}

/// Opaque handle to a loaded texture in the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureId(pub u64);

/// Opaque handle to an offscreen render target.
///
/// Render targets are surfaces that can be drawn into via
/// [`SdiRenderTarget::bind_render_target`](super::SdiRenderTarget::bind_render_target),
/// then composited back into the parent surface with a blend mode.
/// They are the primitive the browser compositor uses to implement
/// `mix-blend-mode`, `backdrop-filter`, `mask-*`, `isolation: isolate`,
/// and box-level `filter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderTargetId(pub u64);

/// CSS-aligned blend modes used when compositing a render target back
/// into its parent surface.
///
/// The 16 variants match the CSS `mix-blend-mode` / `background-blend-mode`
/// vocabulary from [Compositing and Blending Level 1](https://drafts.fxtf.org/compositing/#blendingseparable).
/// `Normal` is standard alpha blending and is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    /// Standard alpha blending (`src * alpha + dst * (1 - alpha)`).
    /// Matches CSS `mix-blend-mode: normal`.
    #[default]
    Normal,
    /// `dst * src`. Darkens.
    Multiply,
    /// `1 - (1 - dst) * (1 - src)`. Lightens.
    Screen,
    /// Multiply or screen depending on destination luminance.
    Overlay,
    /// `min(src, dst)`.
    Darken,
    /// `max(src, dst)`.
    Lighten,
    /// Brightens the destination toward the source.
    ColorDodge,
    /// Darkens the destination toward the source.
    ColorBurn,
    /// Like `Overlay` but conditioned on source luminance.
    HardLight,
    /// Softer variant of `HardLight`.
    SoftLight,
    /// `|dst - src|`.
    Difference,
    /// Like `Difference` but lower contrast.
    Exclusion,
    /// Replaces destination hue with source hue.
    Hue,
    /// Replaces destination saturation with source saturation.
    Saturation,
    /// Replaces destination hue + saturation with source hue + saturation.
    Color,
    /// Replaces destination luminosity with source luminosity.
    Luminosity,
}

impl BlendMode {
    /// Whether this blend mode is the no-op default. Useful for the
    /// recorder to skip emitting a compositing layer when the only
    /// reason to push one would have been a `Normal` blend.
    pub const fn is_normal(self) -> bool {
        matches!(self, BlendMode::Normal)
    }
}

/// Gradient direction and associated colors for gradient fill operations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientStyle {
    /// Vertical gradient from top to bottom.
    Vertical { top: Color, bottom: Color },
    /// Horizontal gradient from left to right.
    Horizontal { left: Color, right: Color },
    /// Four-corner bilinear gradient.
    FourCorner {
        top_left: Color,
        top_right: Color,
        bottom_left: Color,
        bottom_right: Color,
    },
}

impl GradientStyle {
    /// Return the dominant / start color for fallback rendering.
    pub const fn primary_color(&self) -> Color {
        match *self {
            Self::Vertical { top, .. } => top,
            Self::Horizontal { left, .. } => left,
            Self::FourCorner { top_left, .. } => top_left,
        }
    }
}

// ---------------------------------------------------------------------------
// Shape parameter structs (reduce argument counts in rendering functions)
// ---------------------------------------------------------------------------

/// Parameters for stroking (drawing outlines of) shapes.
#[derive(Debug, Clone, Copy)]
pub struct StrokeStyle {
    /// Stroke width in pixels.
    pub width: u16,
    /// Stroke color.
    pub color: Color,
}

// ---------------------------------------------------------------------------
// Vector graphics helpers (used by default trait implementations)
// ---------------------------------------------------------------------------

/// Cosine used by the default arc rasterizers.
///
/// This used to be a 4-term Taylor polynomial evaluated over `[-pi, pi]`,
/// which is off by 0.21 at the ends of that range (`cos(pi)` came out as
/// -1.21), so every default arc overshot its left, top and bottom by up to
/// 21 %. Every target, PSP included, has `std` float math, so this is now
/// plain [`f32::cos`]; the name is kept for the backends that call it.
#[inline]
pub fn cos_approx_f32(x: f32) -> f32 {
    x.cos()
}

/// Sine counterpart of [`cos_approx_f32`]: plain [`f32::sin`].
#[inline]
pub fn sin_approx_f32(x: f32) -> f32 {
    x.sin()
}

/// Compute the number of line segments for an arc at the given radius
/// and angular span. Uses ~1 segment per 8 pixels of arc length,
/// clamped to 4..64.
pub fn arc_segments(radius: u16, start_angle: f32, end_angle: f32) -> usize {
    let arc_len = radius as f32 * (end_angle - start_angle).abs();
    (arc_len / 8.0).ceil().clamp(4.0, 64.0) as usize
}

/// The end angle of an arc from `start` to `end` with the sweep clamped to
/// one full turn either way: more than a turn draws nothing new, and an
/// unbounded sweep would only grow the segment count. Every arc (filled or
/// stroked, default or backend override) clamps through this.
#[must_use]
pub fn clamp_sweep(start: f32, end: f32) -> f32 {
    let tau = core::f32::consts::TAU;
    start + (end - start).clamp(-tau, tau)
}

// ---------------------------------------------------------------------------
// Backend error helpers
// ---------------------------------------------------------------------------

/// Extension trait for converting any error into `OasisError::Backend`.
///
/// Eliminates repeated `.map_err(|e| OasisError::Backend(e.to_string()))`
/// across backend implementations.
pub trait BackendErrExt<T> {
    /// Convert the error to `OasisError::Backend` with the error's display string.
    fn backend_err(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> BackendErrExt<T> for std::result::Result<T, E> {
    fn backend_err(self) -> Result<T> {
        self.map_err(|e| crate::error::OasisError::Backend(e.to_string().into()))
    }
}

/// Return a [`BackendError::TextureNotFound`](crate::error::BackendError)
/// error for the given id.
pub fn texture_not_found(id: u64) -> crate::error::OasisError {
    crate::error::OasisError::Backend(crate::error::BackendError::TextureNotFound { id })
}

/// Return a [`BackendError::RenderTargetNotFound`](crate::error::BackendError)
/// error for the given render-target id.
pub fn render_target_not_found(id: RenderTargetId) -> crate::error::OasisError {
    crate::error::OasisError::Backend(crate::error::BackendError::RenderTargetNotFound { id: id.0 })
}

// ---------------------------------------------------------------------------
// Texture validation helpers
// ---------------------------------------------------------------------------

/// Validate that `rgba_data` has exactly `width * height * 4` bytes.
///
/// Backends should call this at the top of `load_texture()` to replace the
/// duplicated size-check boilerplate.
pub fn validate_rgba_data(width: u32, height: u32, rgba_data: &[u8]) -> Result<()> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| {
            crate::error::OasisError::Backend(
                format!("texture dimensions overflow: {width}x{height}").into(),
            )
        })?;
    if rgba_data.len() != expected {
        return Err(crate::error::OasisError::Backend(
            format!(
                "texture data size mismatch: expected {expected}, got {}",
                rgba_data.len()
            )
            .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn to_abgr_pure_red() {
        let c = Color::rgba(255, 0, 0, 255);
        assert_eq!(c.to_abgr(), 0xFF00_00FF);
    }

    #[test]
    fn to_abgr_pure_green() {
        let c = Color::rgba(0, 255, 0, 255);
        assert_eq!(c.to_abgr(), 0xFF00_FF00);
    }

    #[test]
    fn to_abgr_pure_blue() {
        let c = Color::rgba(0, 0, 255, 255);
        assert_eq!(c.to_abgr(), 0xFFFF_0000);
    }

    #[test]
    fn to_abgr_transparent() {
        let c = Color::rgba(0, 0, 0, 0);
        assert_eq!(c.to_abgr(), 0x0000_0000);
    }

    #[test]
    fn abgr_roundtrip() {
        let original = Color::rgba(0x12, 0x34, 0x56, 0x78);
        assert_eq!(Color::from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn to_argb_pure_red() {
        let c = Color::rgba(255, 0, 0, 255);
        assert_eq!(c.to_argb(), 0xFFFF_0000);
    }

    #[test]
    fn argb_roundtrip() {
        let original = Color::rgba(0x12, 0x34, 0x56, 0x78);
        assert_eq!(Color::from_argb(original.to_argb()), original);
    }

    #[test]
    fn to_rgba_u32_pure_red() {
        let c = Color::rgba(255, 0, 0, 255);
        assert_eq!(c.to_rgba_u32(), 0xFF00_00FF);
    }

    #[test]
    fn all_formats_roundtrip_black() {
        let c = Color::BLACK;
        assert_eq!(Color::from_abgr(c.to_abgr()), c);
        assert_eq!(Color::from_argb(c.to_argb()), c);
    }

    #[test]
    fn all_formats_roundtrip_white() {
        let c = Color::WHITE;
        assert_eq!(Color::from_abgr(c.to_abgr()), c);
        assert_eq!(Color::from_argb(c.to_argb()), c);
    }

    // -----------------------------------------------------------------------
    // Item 70: Color/pixel format conversion tests (RGBA<->ABGR, ARGB, etc.)
    // -----------------------------------------------------------------------

    #[test]
    fn abgr_from_raw_known_value() {
        // ABGR 0xAABBGGRR
        let c = Color::from_abgr(0x80_40_20_10);
        assert_eq!(c.r, 0x10);
        assert_eq!(c.g, 0x20);
        assert_eq!(c.b, 0x40);
        assert_eq!(c.a, 0x80);
    }

    #[test]
    fn argb_from_raw_known_value() {
        // ARGB 0xAARRGGBB
        let c = Color::from_argb(0x80_10_20_40);
        assert_eq!(c.a, 0x80);
        assert_eq!(c.r, 0x10);
        assert_eq!(c.g, 0x20);
        assert_eq!(c.b, 0x40);
    }

    #[test]
    fn abgr_argb_differ_for_nonsymmetric_color() {
        let c = Color::rgba(10, 20, 30, 255);
        // ABGR: A=255, B=30, G=20, R=10 => 0xFF_1E_14_0A
        // ARGB: A=255, R=10, G=20, B=30 => 0xFF_0A_14_1E
        assert_ne!(c.to_abgr(), c.to_argb());
        assert_eq!(c.to_abgr(), 0xFF_1E_14_0A);
        assert_eq!(c.to_argb(), 0xFF_0A_14_1E);
    }

    #[test]
    fn abgr_argb_equal_for_gray() {
        // For gray (r == g == b), ABGR and ARGB differ in byte layout
        // but only if r != b. Gray: r == g == b, so R and B are the same.
        let c = Color::rgba(128, 128, 128, 255);
        // ABGR: 0xFF_80_80_80, ARGB: 0xFF_80_80_80
        assert_eq!(c.to_abgr(), c.to_argb());
    }

    #[test]
    fn rgba_u32_layout() {
        // RGBA u32: R in high byte, A in low byte
        let c = Color::rgba(0xAA, 0xBB, 0xCC, 0xDD);
        assert_eq!(c.to_rgba_u32(), 0xAA_BB_CC_DD);
    }

    #[test]
    fn abgr_roundtrip_all_channels_distinct() {
        let c = Color::rgba(11, 22, 33, 44);
        assert_eq!(Color::from_abgr(c.to_abgr()), c);
    }

    #[test]
    fn argb_roundtrip_all_channels_distinct() {
        let c = Color::rgba(55, 66, 77, 88);
        assert_eq!(Color::from_argb(c.to_argb()), c);
    }

    #[test]
    fn abgr_roundtrip_boundary_values() {
        for &c in &[
            Color::rgba(0, 0, 0, 0),
            Color::rgba(255, 255, 255, 255),
            Color::rgba(255, 0, 0, 0),
            Color::rgba(0, 255, 0, 0),
            Color::rgba(0, 0, 255, 0),
            Color::rgba(0, 0, 0, 255),
        ] {
            assert_eq!(Color::from_abgr(c.to_abgr()), c);
        }
    }

    #[test]
    fn argb_roundtrip_boundary_values() {
        for &c in &[
            Color::rgba(0, 0, 0, 0),
            Color::rgba(255, 255, 255, 255),
            Color::rgba(255, 0, 0, 0),
            Color::rgba(0, 255, 0, 0),
            Color::rgba(0, 0, 255, 0),
            Color::rgba(0, 0, 0, 255),
        ] {
            assert_eq!(Color::from_argb(c.to_argb()), c);
        }
    }

    #[test]
    fn alpha_over_opaque_foreground() {
        let fg = Color::rgba(100, 150, 200, 255);
        let bg = Color::rgba(50, 50, 50, 255);
        let result = fg.alpha_over(bg);
        assert_eq!(result, fg);
    }

    #[test]
    fn alpha_over_transparent_foreground() {
        let fg = Color::rgba(100, 150, 200, 0);
        let bg = Color::rgba(50, 60, 70, 255);
        let result = fg.alpha_over(bg);
        assert_eq!(result, bg);
    }

    #[test]
    fn alpha_over_half_alpha() {
        let fg = Color::rgba(255, 0, 0, 128);
        let bg = Color::rgba(0, 0, 255, 255);
        let result = fg.alpha_over(bg);
        // Foreground red at 50% alpha over blue bg: should produce purple-ish.
        assert!(result.r > 100);
        assert!(result.b > 50);
        assert_eq!(result.a, 255);
    }

    #[test]
    fn alpha_over_both_transparent() {
        let fg = Color::rgba(100, 100, 100, 0);
        let bg = Color::rgba(200, 200, 200, 0);
        let result = fg.alpha_over(bg);
        // fg alpha is 0, returns bg which is fully transparent.
        assert_eq!(result, bg);
    }

    #[test]
    fn with_alpha_preserves_rgb() {
        let c = Color::rgb(10, 20, 30);
        let c2 = c.with_alpha(128);
        assert_eq!(c2.r, 10);
        assert_eq!(c2.g, 20);
        assert_eq!(c2.b, 30);
        assert_eq!(c2.a, 128);
    }

    #[test]
    fn with_alpha_zero_makes_transparent() {
        let c = Color::rgb(255, 255, 255);
        let c2 = c.with_alpha(0);
        assert_eq!(c2.a, 0);
        assert_eq!(c2.r, 255);
    }

    #[test]
    fn lerp_halfway_between_red_and_blue() {
        let r = Color::rgb(255, 0, 0);
        let b = Color::rgb(0, 0, 255);
        let mid = r.lerp(b, 0.5);
        assert_eq!(mid.r, 127);
        assert_eq!(mid.g, 0);
        assert_eq!(mid.b, 127);
    }

    #[test]
    fn apply_opacity_full() {
        let c = Color::rgba(100, 100, 100, 200);
        let result = c.apply_opacity(1.0);
        assert_eq!(result, c);
    }

    #[test]
    fn apply_opacity_half() {
        let c = Color::rgba(100, 100, 100, 200);
        let result = c.apply_opacity(0.5);
        assert_eq!(result.r, 100);
        assert_eq!(result.a, 100);
    }

    #[test]
    fn apply_opacity_zero() {
        let c = Color::rgba(100, 100, 100, 200);
        let result = c.apply_opacity(0.0);
        assert_eq!(result.a, 0);
        assert_eq!(result.r, 100);
    }

    #[test]
    fn trig_helpers_match_std_within_1e3() {
        use core::f32::consts::PI;
        let mut x = -4.0 * PI;
        while x <= 4.0 * PI {
            assert!((cos_approx_f32(x) - x.cos()).abs() < 1e-3, "cos({x})");
            assert!((sin_approx_f32(x) - x.sin()).abs() < 1e-3, "sin({x})");
            x += 0.01;
        }
        // The old Taylor polynomial returned -1.2114 at both of these.
        assert!((cos_approx_f32(PI) + 1.0).abs() < 1e-6);
        assert!((sin_approx_f32(-PI / 2.0) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn color_constants() {
        assert_eq!(Color::BLACK, Color::rgb(0, 0, 0));
        assert_eq!(Color::WHITE, Color::rgb(255, 255, 255));
        assert_eq!(Color::TRANSPARENT, Color::rgba(0, 0, 0, 0));
    }
}
