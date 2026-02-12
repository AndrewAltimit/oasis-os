//! Backend trait definitions.
//!
//! Every platform implements these traits. The core framework dispatches all
//! I/O through trait boundaries -- it never calls platform-specific APIs.
//!
//! The `SdiBackend` trait provides both core rendering methods (required) and
//! extended drawing primitives (optional, with default implementations). See
//! the "Extended Primitives" section for shape, gradient, text, texture, clip,
//! and batch methods that backends can progressively override.

use crate::error::Result;
use crate::input::InputEvent;

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
    GradientV {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        top: Color,
        bottom: Color,
    },
    GradientH {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        left: Color,
        right: Color,
    },
    Gradient4 {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        corners: [Color; 4],
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
}

/// Rendering backend trait.
///
/// Four implementations cover all deployment targets: GU (PSP), SDL2
/// (desktop/Pi), framebuffer (headless Pi), and UE5 render target.
///
/// # Core Methods (required)
///
/// All backends must implement the 13 core methods: `init`, `clear`, `blit`,
/// `fill_rect`, `draw_text`, `swap_buffers`, `load_texture`,
/// `destroy_texture`, `set_clip_rect`, `reset_clip_rect`, `measure_text`,
/// `read_pixels`, and `shutdown`.
///
/// # Extended Primitives (optional, with defaults)
///
/// Backends may override the extended methods for native-accelerated
/// rendering. Default implementations approximate using `fill_rect` and
/// other core methods, so existing backends continue to work without changes.
#[allow(clippy::too_many_arguments)]
pub trait SdiBackend {
    // -----------------------------------------------------------------------
    // Core methods (required -- no default implementations)
    // -----------------------------------------------------------------------

    /// Initialize the rendering subsystem.
    fn init(&mut self, width: u32, height: u32) -> Result<()>;

    /// Clear the screen to a solid color.
    fn clear(&mut self, color: Color) -> Result<()>;

    /// Blit a texture at the given position and size.
    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()>;

    /// Draw a filled rectangle (used when no texture is assigned).
    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()>;

    /// Draw text at the given position. The backend chooses its available font.
    /// `font_size` is a hint in pixels; backends may approximate.
    fn draw_text(&mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color)
    -> Result<()>;

    /// Present the current frame to the display.
    fn swap_buffers(&mut self) -> Result<()>;

    /// Load raw RGBA pixel data as a texture. Returns a handle for later blit.
    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId>;

    /// Destroy a previously loaded texture.
    fn destroy_texture(&mut self, tex: TextureId) -> Result<()>;

    /// Set the clipping rectangle (for window manager content clipping).
    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()>;

    /// Reset clipping to the full screen.
    fn reset_clip_rect(&mut self) -> Result<()>;

    /// Measure the width of a text string at the given font size.
    /// Returns width in pixels. Used by inline layout for line breaking.
    fn measure_text(&self, text: &str, font_size: u16) -> u32;

    /// Read the current framebuffer as RGBA pixel data.
    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>>;

    /// Shut down the rendering subsystem and release resources.
    fn shutdown(&mut self) -> Result<()>;

    // -----------------------------------------------------------------------
    // Extended: Shape Primitives (Phase 1)
    // -----------------------------------------------------------------------

    /// Draw a filled rectangle with rounded corners.
    ///
    /// `radius` specifies the corner radius in pixels. If `radius` exceeds
    /// half the smaller dimension, it is clamped. A radius of 0 is equivalent
    /// to `fill_rect`.
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        _radius: u16,
        color: Color,
    ) -> Result<()> {
        // Default: fall back to sharp-cornered fill_rect.
        self.fill_rect(x, y, w, h, color)
    }

    /// Draw the outline of a rectangle.
    ///
    /// `stroke_width` is drawn inward from the given bounds.
    fn stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let sw = stroke_width as u32;
        self.fill_rect(x, y, w, sw, color)?;
        self.fill_rect(x, y + h as i32 - sw as i32, w, sw, color)?;
        self.fill_rect(x, y + sw as i32, sw, h.saturating_sub(sw * 2), color)?;
        self.fill_rect(
            x + w as i32 - sw as i32,
            y + sw as i32,
            sw,
            h.saturating_sub(sw * 2),
            color,
        )?;
        Ok(())
    }

    /// Draw the outline of a rounded rectangle.
    fn stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        _radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        // Default: fall back to sharp stroke_rect.
        self.stroke_rect(x, y, w, h, stroke_width, color)
    }

    /// Draw a line between two points.
    ///
    /// `width` is the line thickness in pixels. Diagonal lines have no
    /// default rendering; backends must override for diagonal support.
    fn draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        if y1 == y2 {
            let lx = x1.min(x2);
            let w = (x1 - x2).unsigned_abs();
            self.fill_rect(lx, y1, w.max(1), width as u32, color)?;
        } else if x1 == x2 {
            let ly = y1.min(y2);
            let h = (y1 - y2).unsigned_abs();
            self.fill_rect(x1, ly, width as u32, h.max(1), color)?;
        }
        Ok(())
    }

    /// Draw a filled circle.
    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        let r = radius as i32;
        self.fill_rect(cx - r, cy - r, radius as u32 * 2, radius as u32 * 2, color)
    }

    /// Draw the outline of a circle.
    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let _ = stroke_width;
        self.fill_circle(cx, cy, radius, color)
    }

    /// Draw a filled triangle defined by three vertices.
    fn fill_triangle(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        x3: i32,
        y3: i32,
        color: Color,
    ) -> Result<()> {
        let _ = (x1, y1, x2, y2, x3, y3, color);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Extended: Gradient Fills (Phase 2)
    // -----------------------------------------------------------------------

    /// Draw a filled rectangle with a vertical gradient (top to bottom).
    fn fill_rect_gradient_v(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        top_color: Color,
        bottom_color: Color,
    ) -> Result<()> {
        let _ = bottom_color;
        self.fill_rect(x, y, w, h, top_color)
    }

    /// Draw a filled rectangle with a horizontal gradient (left to right).
    fn fill_rect_gradient_h(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        left_color: Color,
        right_color: Color,
    ) -> Result<()> {
        let _ = right_color;
        self.fill_rect(x, y, w, h, left_color)
    }

    /// Draw a filled rectangle with a four-corner gradient.
    fn fill_rect_gradient_4(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        top_left: Color,
        top_right: Color,
        bottom_left: Color,
        bottom_right: Color,
    ) -> Result<()> {
        let _ = (top_right, bottom_left, bottom_right);
        self.fill_rect(x, y, w, h, top_left)
    }

    /// Draw a rounded rectangle with a vertical gradient.
    fn fill_rounded_rect_gradient_v(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        top_color: Color,
        bottom_color: Color,
    ) -> Result<()> {
        let _ = bottom_color;
        self.fill_rounded_rect(x, y, w, h, radius, top_color)
    }

    // -----------------------------------------------------------------------
    // Extended: Alpha Utilities (Phase 2)
    // -----------------------------------------------------------------------

    /// Draw a filled rectangle with explicit alpha override.
    fn fill_rect_alpha(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
        alpha: u8,
    ) -> Result<()> {
        self.fill_rect(x, y, w, h, color.with_alpha(alpha))
    }

    /// Dim the entire viewport with a semi-transparent overlay.
    fn dim_screen(&mut self, alpha: u8) -> Result<()> {
        self.fill_rect(0, 0, 480, 272, Color::rgba(0, 0, 0, alpha))
    }

    // -----------------------------------------------------------------------
    // Extended: Text System (Phase 3)
    // -----------------------------------------------------------------------

    /// Measure the height of text at the given font size.
    fn measure_text_height(&self, font_size: u16) -> u32 {
        (font_size as f32 * 1.2) as u32
    }

    /// Measure both width and height of a text string.
    fn measure_text_extents(&self, text: &str, font_size: u16) -> (u32, u32) {
        (
            self.measure_text(text, font_size),
            self.measure_text_height(font_size),
        )
    }

    /// Measure the font's ascent (baseline to top of tallest glyph).
    fn font_ascent(&self, font_size: u16) -> u32 {
        (font_size as f32 * 0.8) as u32
    }

    /// Draw text truncated with "..." if it exceeds `max_width`.
    ///
    /// Returns the actual drawn width in pixels.
    fn draw_text_ellipsis(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
    ) -> Result<u32> {
        let text_w = self.measure_text(text, font_size);
        if text_w <= max_width {
            self.draw_text(text, x, y, font_size, color)?;
            return Ok(text_w);
        }
        let ellipsis_w = self.measure_text("...", font_size);
        let target = max_width.saturating_sub(ellipsis_w);
        let mut drawn_w = 0u32;
        let mut end_byte = 0;
        for (i, ch) in text.char_indices() {
            let ch_w = self.measure_text(&text[i..i + ch.len_utf8()], font_size);
            if drawn_w + ch_w > target {
                break;
            }
            drawn_w += ch_w;
            end_byte = i + ch.len_utf8();
        }
        let truncated = format!("{}...", &text[..end_byte]);
        self.draw_text(&truncated, x, y, font_size, color)?;
        Ok(drawn_w + ellipsis_w)
    }

    /// Draw text with a font weight hint.
    ///
    /// `weight`: 100 (thin) to 900 (black), 400 = normal, 700 = bold.
    /// Backends with only a single bitmap font ignore the weight.
    fn draw_text_weighted(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        weight: u16,
        color: Color,
    ) -> Result<()> {
        let _ = weight;
        self.draw_text(text, x, y, font_size, color)
    }

    /// Draw multiline word-wrapped text within a bounding box.
    ///
    /// Returns the total height used in pixels.
    fn draw_text_wrapped(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
        line_height: u32,
    ) -> Result<u32> {
        let lh = if line_height > 0 {
            line_height
        } else {
            self.measure_text_height(font_size)
        };
        let mut cy = y;
        for line in text.split('\n') {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.is_empty() {
                cy += lh as i32;
                continue;
            }
            let mut current_line = String::new();
            for word in words {
                let test = if current_line.is_empty() {
                    word.to_string()
                } else {
                    format!("{current_line} {word}")
                };
                if self.measure_text(&test, font_size) > max_width && !current_line.is_empty() {
                    self.draw_text(&current_line, x, cy, font_size, color)?;
                    cy += lh as i32;
                    current_line = word.to_string();
                } else {
                    current_line = test;
                }
            }
            if !current_line.is_empty() {
                self.draw_text(&current_line, x, cy, font_size, color)?;
                cy += lh as i32;
            }
        }
        Ok((cy - y) as u32)
    }

    // -----------------------------------------------------------------------
    // Extended: Texture Operations (Phase 4)
    // -----------------------------------------------------------------------

    /// Blit a sub-rectangle from a texture (sprite sheet / atlas support).
    fn blit_sub(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        let _ = (src_x, src_y, src_w, src_h);
        self.blit(tex, dst_x, dst_y, dst_w, dst_h)
    }

    /// Blit a texture with a multiplicative color tint.
    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        let _ = tint;
        self.blit(tex, x, y, w, h)
    }

    /// Blit a texture sub-rectangle with a color tint.
    fn blit_sub_tinted(
        &mut self,
        tex: TextureId,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        tint: Color,
    ) -> Result<()> {
        let _ = tint;
        self.blit_sub(tex, src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h)
    }

    /// Blit a texture with horizontal and/or vertical flip.
    fn blit_flipped(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        flip_h: bool,
        flip_v: bool,
    ) -> Result<()> {
        let _ = (flip_h, flip_v);
        self.blit(tex, x, y, w, h)
    }

    // -----------------------------------------------------------------------
    // Extended: Clip and Transform Stack (Phase 5)
    // -----------------------------------------------------------------------

    /// Push a clip rectangle onto the clip stack.
    ///
    /// The effective clip is the intersection of all pushed rects.
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.set_clip_rect(x, y, w, h)
    }

    /// Pop the most recently pushed clip rectangle.
    fn pop_clip_rect(&mut self) -> Result<()> {
        self.reset_clip_rect()
    }

    /// Query the current effective clip rectangle.
    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        None
    }

    /// Push a coordinate origin translation onto the transform stack.
    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        let _ = (dx, dy);
        Ok(())
    }

    /// Pop the most recently pushed translation.
    fn pop_translate(&mut self) -> Result<()> {
        Ok(())
    }

    /// Query the current cumulative translation offset.
    fn current_translate(&self) -> (i32, i32) {
        (0, 0)
    }

    /// Push a rendering region (translate + clip).
    fn push_region(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        self.push_translate(x, y)?;
        self.push_clip_rect(0, 0, w, h)
    }

    /// Pop a previously pushed region.
    fn pop_region(&mut self) -> Result<()> {
        self.pop_clip_rect()?;
        self.pop_translate()
    }

    // -----------------------------------------------------------------------
    // Extended: Batch Rendering (Phase 6)
    // -----------------------------------------------------------------------

    /// Begin recording draw commands into a batch.
    fn begin_batch(&mut self) -> Result<()> {
        Ok(())
    }

    /// Flush and execute all batched draw commands.
    fn flush_batch(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Input backend trait.
///
/// Maps platform-specific input to the platform-agnostic `InputEvent` enum.
pub trait InputBackend {
    /// Poll for pending input events.
    fn poll_events(&mut self) -> Vec<InputEvent>;
}

/// Network backend trait.
///
/// Abstracts TCP operations across sceNetInet (PSP) and std::net (Linux).
pub trait NetworkBackend {
    /// Start listening for incoming connections on the given port.
    fn listen(&mut self, port: u16) -> Result<()>;

    /// Accept a pending connection. Returns `None` if no connection waiting.
    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>>;

    /// Open an outbound TCP connection.
    fn connect(&mut self, address: &str, port: u16) -> Result<Box<dyn NetworkStream>>;

    /// Return the TLS provider for this backend, if available.
    ///
    /// When `Some`, the browser can negotiate HTTPS and Gemini connections.
    /// Backends without TLS support return `None` (the default).
    fn tls_provider(&self) -> Option<&dyn crate::net::tls::TlsProvider> {
        None
    }
}

/// A bidirectional byte stream (TCP connection).
pub trait NetworkStream: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<usize>;
    fn close(&mut self) -> Result<()>;
}

/// Opaque handle to a loaded audio track in the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioTrackId(pub u64);

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A test backend that records all draw calls for assertion.
    struct RecordingBackend {
        calls: RefCell<Vec<String>>,
    }

    impl RecordingBackend {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        #[allow(dead_code)]
        fn clear_calls(&self) {
            self.calls.borrow_mut().clear();
        }
    }

    impl SdiBackend for RecordingBackend {
        fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn clear(&mut self, color: Color) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("clear({},{},{},{})", color.r, color.g, color.b, color.a));
            Ok(())
        }
        fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("blit({},{x},{y},{w},{h})", tex.0));
            Ok(())
        }
        fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
            self.calls.borrow_mut().push(format!(
                "fill_rect({x},{y},{w},{h},{},{},{},{})",
                color.r, color.g, color.b, color.a
            ));
            Ok(())
        }
        fn draw_text(
            &mut self,
            text: &str,
            x: i32,
            y: i32,
            font_size: u16,
            color: Color,
        ) -> Result<()> {
            self.calls.borrow_mut().push(format!(
                "draw_text({text},{x},{y},{font_size},{},{},{},{})",
                color.r, color.g, color.b, color.a
            ));
            Ok(())
        }
        fn swap_buffers(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> Result<TextureId> {
            Ok(TextureId(1))
        }
        fn destroy_texture(&mut self, _tex: TextureId) -> Result<()> {
            Ok(())
        }
        fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("set_clip({x},{y},{w},{h})"));
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> Result<()> {
            self.calls.borrow_mut().push("reset_clip".into());
            Ok(())
        }
        fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
            text.len() as u32 * 8
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // -- Color tests --

    #[test]
    fn color_rgb_alpha_255() {
        let c = Color::rgb(10, 20, 30);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn color_rgba_explicit() {
        let c = Color::rgba(1, 2, 3, 128);
        assert_eq!((c.r, c.g, c.b, c.a), (1, 2, 3, 128));
    }

    #[test]
    fn color_with_alpha() {
        let c = Color::rgb(100, 200, 50).with_alpha(64);
        assert_eq!(c.r, 100);
        assert_eq!(c.a, 64);
    }

    #[test]
    fn color_constants() {
        assert_eq!(Color::BLACK, Color::rgb(0, 0, 0));
        assert_eq!(Color::WHITE, Color::rgb(255, 255, 255));
        assert_eq!(Color::TRANSPARENT, Color::rgba(0, 0, 0, 0));
    }

    // -- TextureId tests --

    #[test]
    fn texture_id_equality() {
        assert_eq!(TextureId(42), TextureId(42));
        assert_ne!(TextureId(1), TextureId(2));
    }

    #[test]
    fn texture_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TextureId(1));
        set.insert(TextureId(2));
        set.insert(TextureId(1));
        assert_eq!(set.len(), 2);
    }

    // -- Default: fill_rounded_rect falls back to fill_rect --

    #[test]
    fn fill_rounded_rect_defaults_to_fill_rect() {
        let mut b = RecordingBackend::new();
        b.fill_rounded_rect(10, 20, 100, 50, 8, Color::rgb(255, 0, 0))
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("fill_rect(10,20,100,50,"));
    }

    // -- Default: stroke_rect emits 4 fill_rect calls --

    #[test]
    fn stroke_rect_emits_four_rects() {
        let mut b = RecordingBackend::new();
        b.stroke_rect(0, 0, 100, 80, 2, Color::WHITE).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 4, "stroke_rect should emit 4 fill_rect calls");
        for call in &calls {
            assert!(call.starts_with("fill_rect("));
        }
    }

    #[test]
    fn stroke_rect_top_edge() {
        let mut b = RecordingBackend::new();
        b.stroke_rect(5, 10, 100, 80, 3, Color::WHITE).unwrap();
        let calls = b.calls();
        // First call is the top edge: fill_rect(5,10,100,3,...)
        assert!(calls[0].starts_with("fill_rect(5,10,100,3,"));
    }

    // -- Default: stroke_rounded_rect falls back to stroke_rect --

    #[test]
    fn stroke_rounded_rect_defaults_to_stroke_rect() {
        let mut b = RecordingBackend::new();
        b.stroke_rounded_rect(0, 0, 50, 50, 5, 1, Color::WHITE)
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 4); // Same as stroke_rect
    }

    // -- Default: draw_line horizontal --

    #[test]
    fn draw_line_horizontal() {
        let mut b = RecordingBackend::new();
        b.draw_line(10, 50, 100, 50, 2, Color::WHITE).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("fill_rect(10,50,90,2,"));
    }

    #[test]
    fn draw_line_vertical() {
        let mut b = RecordingBackend::new();
        b.draw_line(50, 10, 50, 80, 3, Color::WHITE).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("fill_rect(50,10,3,70,"));
    }

    #[test]
    fn draw_line_diagonal_is_noop() {
        let mut b = RecordingBackend::new();
        b.draw_line(0, 0, 100, 100, 1, Color::WHITE).unwrap();
        assert!(b.calls().is_empty(), "diagonal lines should be no-op in default impl");
    }

    // -- Default: fill_circle falls back to bounding box fill_rect --

    #[test]
    fn fill_circle_default() {
        let mut b = RecordingBackend::new();
        b.fill_circle(50, 50, 10, Color::rgb(0, 255, 0)).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("fill_rect(40,40,20,20,"));
    }

    // -- Default: stroke_circle falls back to fill_circle --

    #[test]
    fn stroke_circle_default() {
        let mut b = RecordingBackend::new();
        b.stroke_circle(50, 50, 10, 1, Color::WHITE).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("fill_rect(")); // fill_circle → fill_rect
    }

    // -- Default: fill_triangle is no-op --

    #[test]
    fn fill_triangle_default_noop() {
        let mut b = RecordingBackend::new();
        b.fill_triangle(0, 0, 10, 0, 5, 10, Color::WHITE).unwrap();
        assert!(b.calls().is_empty());
    }

    // -- Gradient defaults --

    #[test]
    fn gradient_v_defaults_to_fill_rect() {
        let mut b = RecordingBackend::new();
        let top = Color::rgb(255, 0, 0);
        b.fill_rect_gradient_v(0, 0, 100, 50, top, Color::rgb(0, 0, 255))
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("255,0,0")); // Uses top_color
    }

    #[test]
    fn gradient_h_defaults_to_fill_rect() {
        let mut b = RecordingBackend::new();
        let left = Color::rgb(0, 255, 0);
        b.fill_rect_gradient_h(0, 0, 100, 50, left, Color::rgb(0, 0, 255))
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("0,255,0")); // Uses left_color
    }

    #[test]
    fn gradient_4_defaults_to_fill_rect() {
        let mut b = RecordingBackend::new();
        let tl = Color::rgb(10, 20, 30);
        b.fill_rect_gradient_4(0, 0, 100, 50, tl, Color::WHITE, Color::WHITE, Color::WHITE)
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("10,20,30")); // Uses top_left
    }

    #[test]
    fn rounded_rect_gradient_v_default() {
        let mut b = RecordingBackend::new();
        let top = Color::rgb(255, 0, 0);
        b.fill_rounded_rect_gradient_v(0, 0, 100, 50, 5, top, Color::BLACK)
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("fill_rect(")); // Falls to fill_rounded_rect → fill_rect
    }

    // -- Alpha utilities --

    #[test]
    fn fill_rect_alpha_overrides() {
        let mut b = RecordingBackend::new();
        b.fill_rect_alpha(0, 0, 100, 50, Color::rgb(255, 255, 255), 128)
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(",128)")); // Alpha applied
    }

    #[test]
    fn dim_screen_uses_black_overlay() {
        let mut b = RecordingBackend::new();
        b.dim_screen(100).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("fill_rect(0,0,480,272,0,0,0,100)"));
    }

    // -- Text system defaults --

    #[test]
    fn measure_text_height_default() {
        let b = RecordingBackend::new();
        // font_size 10 → 10 * 1.2 = 12
        assert_eq!(b.measure_text_height(10), 12);
    }

    #[test]
    fn measure_text_extents_default() {
        let b = RecordingBackend::new();
        let (w, h) = b.measure_text_extents("ABCD", 10);
        assert_eq!(w, 32); // 4 chars * 8px
        assert_eq!(h, 12); // 10 * 1.2
    }

    #[test]
    fn font_ascent_default() {
        let b = RecordingBackend::new();
        assert_eq!(b.font_ascent(10), 8); // 10 * 0.8
    }

    #[test]
    fn draw_text_ellipsis_short_text() {
        let mut b = RecordingBackend::new();
        let drawn = b
            .draw_text_ellipsis("Hi", 0, 0, 8, Color::WHITE, 200)
            .unwrap();
        assert_eq!(drawn, 16); // 2 chars * 8px
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("Hi"));
    }

    #[test]
    fn draw_text_ellipsis_truncates() {
        let mut b = RecordingBackend::new();
        let long_text = "Hello World This Is Long";
        let drawn = b
            .draw_text_ellipsis(long_text, 0, 0, 8, Color::WHITE, 80)
            .unwrap();
        assert!(drawn <= 80);
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("..."));
    }

    #[test]
    fn draw_text_weighted_ignores_weight() {
        let mut b = RecordingBackend::new();
        b.draw_text_weighted("Bold", 0, 0, 8, 700, Color::WHITE)
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("Bold"));
    }

    #[test]
    fn draw_text_wrapped_single_line() {
        let mut b = RecordingBackend::new();
        let h = b
            .draw_text_wrapped("Short", 0, 0, 8, Color::WHITE, 200, 10)
            .unwrap();
        assert_eq!(h, 10); // One line of height 10
    }

    #[test]
    fn draw_text_wrapped_wraps_long_line() {
        let mut b = RecordingBackend::new();
        // max_width=40 → 5 chars fit per line. "Hello World" should wrap.
        let h = b
            .draw_text_wrapped("Hello World", 0, 0, 8, Color::WHITE, 40, 10)
            .unwrap();
        assert_eq!(h, 20); // Two lines
    }

    #[test]
    fn draw_text_wrapped_newlines() {
        let mut b = RecordingBackend::new();
        let h = b
            .draw_text_wrapped("A\nB\nC", 0, 0, 8, Color::WHITE, 200, 10)
            .unwrap();
        assert_eq!(h, 30); // Three lines
    }

    // -- Texture operation defaults --

    #[test]
    fn blit_sub_defaults_to_blit() {
        let mut b = RecordingBackend::new();
        let tex = TextureId(5);
        b.blit_sub(tex, 0, 0, 32, 32, 10, 20, 64, 64).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("blit(5,10,20,64,64)"));
    }

    #[test]
    fn blit_tinted_defaults_to_blit() {
        let mut b = RecordingBackend::new();
        let tex = TextureId(3);
        b.blit_tinted(tex, 5, 10, 32, 32, Color::rgb(255, 0, 0))
            .unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("blit(3,5,10,32,32)"));
    }

    #[test]
    fn blit_flipped_defaults_to_blit() {
        let mut b = RecordingBackend::new();
        let tex = TextureId(7);
        b.blit_flipped(tex, 0, 0, 16, 16, true, false).unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("blit(7,"));
    }

    // -- Clip/transform stack defaults --

    #[test]
    fn push_pop_clip_rect() {
        let mut b = RecordingBackend::new();
        b.push_clip_rect(10, 20, 100, 50).unwrap();
        b.pop_clip_rect().unwrap();
        let calls = b.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].starts_with("set_clip("));
        assert_eq!(calls[1], "reset_clip");
    }

    #[test]
    fn current_clip_rect_default_none() {
        let b = RecordingBackend::new();
        assert!(b.current_clip_rect().is_none());
    }

    #[test]
    fn push_pop_translate_noop() {
        let mut b = RecordingBackend::new();
        b.push_translate(10, 20).unwrap();
        b.pop_translate().unwrap();
        assert!(b.calls().is_empty()); // Default is no-op
    }

    #[test]
    fn current_translate_default_zero() {
        let b = RecordingBackend::new();
        assert_eq!(b.current_translate(), (0, 0));
    }

    #[test]
    fn push_pop_region() {
        let mut b = RecordingBackend::new();
        b.push_region(10, 20, 100, 50).unwrap();
        b.pop_region().unwrap();
        let calls = b.calls();
        // push_region: push_translate (no-op) + push_clip_rect (set_clip)
        // pop_region: pop_clip_rect (reset_clip) + pop_translate (no-op)
        assert!(calls.contains(&"set_clip(0,0,100,50)".to_string()));
        assert!(calls.contains(&"reset_clip".to_string()));
    }

    // -- Batch rendering defaults --

    #[test]
    fn begin_flush_batch_noop() {
        let mut b = RecordingBackend::new();
        b.begin_batch().unwrap();
        b.flush_batch().unwrap();
        assert!(b.calls().is_empty());
    }

    // -- AudioTrackId --

    #[test]
    fn audio_track_id_equality() {
        assert_eq!(AudioTrackId(1), AudioTrackId(1));
        assert_ne!(AudioTrackId(1), AudioTrackId(2));
    }

    // -- DrawCommand variants --

    #[test]
    fn draw_command_fill_rect() {
        let cmd = DrawCommand::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::WHITE,
        };
        // Just verify it can be constructed and debug-printed.
        let dbg = format!("{cmd:?}");
        assert!(dbg.contains("FillRect"));
    }

    #[test]
    fn draw_command_clone() {
        let cmd = DrawCommand::DrawText {
            text: "hello".into(),
            x: 5,
            y: 10,
            font_size: 8,
            color: Color::BLACK,
        };
        let cmd2 = cmd.clone();
        let dbg1 = format!("{cmd:?}");
        let dbg2 = format!("{cmd2:?}");
        assert_eq!(dbg1, dbg2);
    }

    #[test]
    fn draw_command_all_variants_constructible() {
        // Verify all DrawCommand variants can be constructed without panic.
        let _commands = vec![
            DrawCommand::FillRect { x: 0, y: 0, w: 1, h: 1, color: Color::BLACK },
            DrawCommand::FillRoundedRect { x: 0, y: 0, w: 1, h: 1, radius: 2, color: Color::BLACK },
            DrawCommand::StrokeRect { x: 0, y: 0, w: 1, h: 1, stroke_width: 1, color: Color::BLACK },
            DrawCommand::DrawLine { x1: 0, y1: 0, x2: 1, y2: 1, width: 1, color: Color::BLACK },
            DrawCommand::FillCircle { cx: 0, cy: 0, radius: 5, color: Color::BLACK },
            DrawCommand::FillTriangle { points: [(0, 0), (1, 0), (0, 1)], color: Color::BLACK },
            DrawCommand::GradientV { x: 0, y: 0, w: 1, h: 1, top: Color::BLACK, bottom: Color::WHITE },
            DrawCommand::GradientH { x: 0, y: 0, w: 1, h: 1, left: Color::BLACK, right: Color::WHITE },
            DrawCommand::Gradient4 { x: 0, y: 0, w: 1, h: 1, corners: [Color::BLACK; 4] },
            DrawCommand::DrawText { text: "x".into(), x: 0, y: 0, font_size: 8, color: Color::BLACK },
            DrawCommand::Blit { tex: TextureId(1), x: 0, y: 0, w: 1, h: 1 },
            DrawCommand::BlitSub { tex: TextureId(1), src: (0, 0, 1, 1), dst: (0, 0, 1, 1) },
            DrawCommand::BlitTinted { tex: TextureId(1), x: 0, y: 0, w: 1, h: 1, tint: Color::WHITE },
            DrawCommand::PushClip { x: 0, y: 0, w: 1, h: 1 },
            DrawCommand::PopClip,
            DrawCommand::PushTranslate { dx: 1, dy: 2 },
            DrawCommand::PopTranslate,
        ];
    }
}

/// Audio playback backend trait.
///
/// Two implementations cover all deployment targets: rodio/SDL2_mixer (desktop/Pi)
/// and Media Engine offloading (PSP via PRX stubs).
pub trait AudioBackend {
    /// Initialize the audio subsystem (open device, set sample rate).
    fn init(&mut self) -> Result<()>;

    /// Load an audio file from raw bytes (MP3, WAV, OGG).
    /// Returns a handle for playback control.
    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId>;

    /// Start playing a loaded track from the beginning.
    fn play(&mut self, track: AudioTrackId) -> Result<()>;

    /// Pause the currently playing track.
    fn pause(&mut self) -> Result<()>;

    /// Resume a paused track.
    fn resume(&mut self) -> Result<()>;

    /// Stop playback and reset position to the beginning.
    fn stop(&mut self) -> Result<()>;

    /// Set volume (0 = silent, 100 = full).
    fn set_volume(&mut self, volume: u8) -> Result<()>;

    /// Get the current volume (0-100).
    fn get_volume(&self) -> u8;

    /// Return `true` if audio is currently playing.
    fn is_playing(&self) -> bool;

    /// Get the current playback position in milliseconds.
    fn position_ms(&self) -> u64;

    /// Get the total duration of the current track in milliseconds.
    /// Returns 0 if no track is loaded.
    fn duration_ms(&self) -> u64;

    /// Unload a previously loaded track and free its resources.
    fn unload_track(&mut self, track: AudioTrackId) -> Result<()>;

    /// Shut down the audio subsystem and release all resources.
    fn shutdown(&mut self) -> Result<()>;
}
