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
        self.push_clip_rect(x, y, w, h)
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
