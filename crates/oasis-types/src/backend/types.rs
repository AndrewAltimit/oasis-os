//! Core types: Color, TextureId, DrawCommand, TextMetrics, GradientStyle.

use crate::error::Result;

/// Width of a single glyph in the bitmap font system.
pub const BITMAP_GLYPH_WIDTH: u32 = 8;

/// Height of a single glyph in the bitmap font system.
pub const BITMAP_GLYPH_HEIGHT: u32 = 8;

/// Measure text width using proportional bitmap font metrics.
///
/// Sums per-character advance widths derived from the actual glyph ink bounds,
/// then scales by the font-size multiplier. This produces tighter text than
/// the old fixed `8 * len` calculation.
pub fn bitmap_measure_text(text: &str, font_size: u16) -> u32 {
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

    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
}

/// Opaque handle to a loaded texture in the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureId(pub u64);

/// A recorded draw command for batch submission.
///
/// Draw commands capture all parameters needed to replay a draw call. The
/// batch renderer sorts commands to minimize GPU state changes before
/// executing them.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    FillRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    },
    FillRoundedRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    },
    StrokeRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    },
    DrawLine {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    },
    FillCircle {
        cx: i32,
        cy: i32,
        radius: u16,
        color: Color,
    },
    FillTriangle {
        points: [(i32, i32); 3],
        color: Color,
    },
    Gradient {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: GradientStyle,
    },
    DrawText {
        text: String,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    },
    Blit {
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    BlitSub {
        tex: TextureId,
        src: (u32, u32, u32, u32),
        dst: (i32, i32, u32, u32),
    },
    BlitTinted {
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    },
    PushClip {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    PopClip,
    PushTranslate {
        dx: i32,
        dy: i32,
    },
    PopTranslate,
    FillPolygon {
        points: Vec<(i32, i32)>,
        color: Color,
    },
    FillArc {
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    },
    StrokeArc {
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        width: u16,
        color: Color,
    },
    StrokeLineDashed {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
        dash: u16,
        gap: u16,
    },
}

/// Measured dimensions and baseline metrics for a text string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMetrics {
    /// Width of the text string in pixels.
    pub width: u32,
    /// Total line height (ascent + descent + leading) in pixels.
    pub height: u32,
    /// Distance from the baseline to the top of the tallest glyph, in pixels.
    pub ascent: u32,
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
// Vector graphics helpers (used by default trait implementations)
// ---------------------------------------------------------------------------

/// Fast cosine approximation using a 4-term Taylor series.
///
/// Accurate to ~0.001 for the range used in arc rendering.
/// Avoids pulling in libm on `no_std` targets (PSP).
pub fn cos_approx_f32(x: f32) -> f32 {
    use core::f32::consts::PI;
    // Reduce to [-PI, PI].
    let mut x = x % (2.0 * PI);
    if x > PI {
        x -= 2.0 * PI;
    } else if x < -PI {
        x += 2.0 * PI;
    }
    let x2 = x * x;
    // Taylor: 1 - x^2/2 + x^4/24 - x^6/720
    1.0 - x2 * (0.5 - x2 * (1.0 / 24.0 - x2 * (1.0 / 720.0)))
}

/// Fast sine approximation: `sin(x) = cos(x - PI/2)`.
pub fn sin_approx_f32(x: f32) -> f32 {
    cos_approx_f32(x - core::f32::consts::FRAC_PI_2)
}

/// Compute the number of line segments for an arc at the given radius
/// and angular span. Uses ~1 segment per 8 pixels of arc length,
/// clamped to 4..64.
pub fn arc_segments(radius: u16, start_angle: f32, end_angle: f32) -> usize {
    let arc_len = radius as f32 * (end_angle - start_angle).abs();
    (arc_len / 8.0).ceil().clamp(4.0, 64.0) as usize
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

/// Look up a value in an `Option`, returning `OasisError::Backend` if `None`.
///
/// Eliminates repeated `.ok_or_else(|| OasisError::Backend(format!(...).into()))`.
pub fn backend_require<T>(opt: Option<T>, msg: &str) -> Result<T> {
    opt.ok_or_else(|| crate::error::OasisError::Backend(msg.into()))
}

/// Return a "texture not found" backend error for the given id.
pub fn texture_not_found(id: u64) -> crate::error::OasisError {
    crate::error::OasisError::Backend(format!("texture not found: {id}").into())
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
