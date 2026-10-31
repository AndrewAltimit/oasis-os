//! Software RGBA framebuffer renderer.
//!
//! Implements `SdiBackend` by drawing into a `Vec<u8>` RGBA buffer. UE5 reads
//! the buffer via `oasis_get_buffer()` and copies it to a `UTexture2D`.
//!
//! All extended primitives (rounded rects, lines, circles, triangles,
//! gradients, sub-rect blits, clip/transform stacks) are software-rasterized
//! into the pixel buffer via the shared `oasis-rasterize` crate.

use std::collections::HashMap;
use std::rc::Rc;

use oasis_rasterize::SoftwareBuffer;
use oasis_types::backend::stacks::{ClipStack, DrawState, TranslateStack};
use oasis_types::backend::{
    BatchRect, BlendMode, Color, GradientStyle, RenderTargetId, SdiBatch, SdiClipTransform,
    SdiCore, SdiGradients, SdiRenderTarget, SdiShapes, SdiText, SdiTextures, SdiVector, TextureId,
    render_target_not_found, texture_not_found, validate_rgba_data,
};
use oasis_types::error::{OasisError, Result};
use oasis_types::geometry::ClipRect;
use oasis_types::rasterize::PixelSink;

/// A stored texture for later blitting.
struct Texture {
    width: u32,
    height: u32,
    data: Rc<Vec<u8>>,
}

/// One text draw recorded while the text log is enabled
/// ([`Ue5Backend::enable_text_log`]).
///
/// Coordinates are the caller's (before the translate stack is applied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawnText {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub font_size: u16,
    pub color: Color,
}

/// Text drawn in the current and in the last presented frame.
#[derive(Default)]
struct TextLog {
    /// Since the last framebuffer `clear`.
    drawing: Vec<DrawnText>,
    /// The last presented frame.
    presented: Vec<DrawnText>,
}

/// The surface a render-target bind replaced, restored on unbind.
struct BoundFrom {
    /// Render target that was bound before (`None` = framebuffer).
    prev_id: Option<u64>,
    /// The parent surface's buffer (it carries its own applied clip).
    parent: SoftwareBuffer,
    /// The parent surface's clip and translate stacks.
    state: DrawState,
}

/// Software RGBA framebuffer renderer for UE5 integration.
///
/// All rendering operations write directly to an RGBA pixel buffer.
/// The buffer is exposed to UE5 via the FFI layer. A dirty flag tracks
/// whether the buffer has changed since the last read.
pub struct Ue5Backend {
    fb: SoftwareBuffer,
    dirty: bool,
    textures: Vec<Option<Texture>>,
    clip_stack: ClipStack,
    translate_stack: TranslateStack,
    /// Offscreen render targets. Keyed by `RenderTargetId` inner u64.
    /// When a target is *bound*, its buffer is temporarily swapped into
    /// `self.fb` and the parent surface is stored on the bind stack.
    render_targets: HashMap<u64, SoftwareBuffer>,
    /// One entry per active bind, innermost last.
    render_target_bind_stack: Vec<BoundFrom>,
    /// Currently bound render target id (`None` = framebuffer).
    current_render_target: Option<u64>,
    /// Monotonic counter for render-target ids.
    next_render_target_id: u64,
    /// Reused buffer for scaled render-target composites.
    composite_scratch: Vec<u8>,
    /// Text recording for test harnesses; `None` (the default) records
    /// nothing.
    text_log: Option<TextLog>,
    /// Number of `swap_buffers` calls.
    frames_presented: u64,
}

impl Ue5Backend {
    /// Create a new backend with the given resolution.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            fb: SoftwareBuffer::new(width, height),
            dirty: true,
            textures: Vec::new(),
            clip_stack: ClipStack::default(),
            translate_stack: TranslateStack::new(),
            render_targets: HashMap::new(),
            render_target_bind_stack: Vec::new(),
            current_render_target: None,
            next_render_target_id: 1,
            composite_scratch: Vec::new(),
            text_log: None,
            frames_presented: 0,
        }
    }

    /// Get a read-only reference to the RGBA pixel buffer.
    pub fn buffer(&self) -> &[u8] {
        self.fb.data()
    }

    /// Whether the buffer has been modified since the last `clear_dirty()`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag (called after UE5 reads the buffer).
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Buffer dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.fb.width(), self.fb.height())
    }

    /// Blit raw RGBA pixels into the framebuffer at the given position.
    pub fn blit_rgba(&mut self, x: u32, y: u32, w: u32, h: u32, pixels: &[u8]) {
        self.fb.blit_rgba(x, y, w, h, pixels);
        self.dirty = true;
    }

    /// Number of live textures (loaded and not yet destroyed).
    pub fn live_textures(&self) -> usize {
        self.textures.iter().filter(|t| t.is_some()).count()
    }

    /// Number of live offscreen render targets (bound ones included).
    pub fn live_render_targets(&self) -> usize {
        self.render_targets.len()
            + usize::from(self.current_render_target.is_some())
            + self
                .render_target_bind_stack
                .iter()
                .filter(|b| b.prev_id.is_some())
                .count()
    }

    /// Number of `swap_buffers` calls so far.
    pub fn frames_presented(&self) -> u64 {
        self.frames_presented
    }

    /// Start recording every string drawn (`draw_text`, `draw_text_styled`
    /// and everything built on them), for test harnesses that assert on
    /// painted text. The log of a frame runs from the framebuffer `clear`
    /// to `swap_buffers`; clearing a bound render target keeps it.
    pub fn enable_text_log(&mut self) {
        self.text_log.get_or_insert_with(TextLog::default);
    }

    /// Text drawn in the last presented frame (empty unless
    /// [`enable_text_log`](Self::enable_text_log) was called).
    pub fn presented_text(&self) -> &[DrawnText] {
        self.text_log.as_ref().map_or(&[], |l| &l.presented)
    }

    /// Apply cumulative translation to coordinates.
    fn translate(&self, x: i32, y: i32) -> (i32, i32) {
        self.translate_stack.translate(x, y)
    }

    /// Set a single pixel with alpha blending (delegates to
    /// [`SoftwareBuffer`]).
    #[cfg(test)]
    fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        self.fb.set_pixel(x, y, color);
    }

    /// Get texture data via `Rc::clone` (O(1) refcount bump, no data copy).
    fn get_texture_data(&self, tex: TextureId) -> Result<(u32, u32, Rc<Vec<u8>>)> {
        let idx = tex.0 as usize;
        let texture = self
            .textures
            .get(idx)
            .and_then(|t| t.as_ref())
            .ok_or_else(|| texture_not_found(tex))?;
        Ok((texture.width, texture.height, Rc::clone(&texture.data)))
    }

    /// Composite render target `id` into the bound surface, scaled to the
    /// destination rect (nearest-neighbour, the same sampling as `blit`).
    #[allow(clippy::too_many_arguments)]
    fn composite_scaled(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        blend: BlendMode,
        opacity: f32,
    ) -> Result<()> {
        let Self {
            fb,
            render_targets,
            composite_scratch,
            ..
        } = self;
        let src = render_targets
            .get(&id.0)
            .ok_or_else(|| render_target_not_found(id))?;
        let (sw, sh) = (src.width(), src.height());
        if dst_w == 0 || dst_h == 0 {
            return Ok(());
        }
        // `render_targets` and `fb` are disjoint fields, so an unscaled
        // layer is composited straight from its storage.
        if (sw, sh) == (dst_w, dst_h) {
            fb.composite_rgba_blend(dst_x, dst_y, sw, sh, src.data(), opacity, blend);
        } else {
            // Resample only the part of the destination that lands on the
            // framebuffer: the destination rect is caller-controlled and
            // may be far larger than the screen.
            let visible = |pos: i32, len: u32, screen: u32| {
                let start = i64::from(pos).max(0);
                let end = (i64::from(pos) + i64::from(len)).min(i64::from(screen));
                (start < end).then(|| ((start - i64::from(pos)) as u32, (end - start) as u32))
            };
            let (Some((ox, vw)), Some((oy, vh))) = (
                visible(dst_x, dst_w, fb.width()),
                visible(dst_y, dst_h, fb.height()),
            ) else {
                return Ok(());
            };
            let region = ScaleRegion {
                dst_w,
                dst_h,
                x0: ox,
                y0: oy,
                w: vw,
                h: vh,
            };
            scale_nearest(src.data(), sw, sh, &region, composite_scratch);
            // On screen, so in i32 range.
            let at = |pos: i32, off: u32| (i64::from(pos) + i64::from(off)) as i32;
            fb.composite_rgba_blend(
                at(dst_x, ox),
                at(dst_y, oy),
                vw,
                vh,
                composite_scratch,
                opacity,
                blend,
            );
        }
        self.dirty = true;
        Ok(())
    }
}

/// The `w` x `h` window at (`x0`, `y0`) of a `dst_w` x `dst_h` resample.
struct ScaleRegion {
    dst_w: u32,
    dst_h: u32,
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
}

/// Nearest-neighbour resample of an `sw` x `sh` RGBA image to
/// `region.dst_w` x `region.dst_h`, producing only `region`'s pixels into
/// `out` (destination column `k` samples source column `k * sw / dst_w`,
/// like `SoftwareBuffer`'s blits).
fn scale_nearest(src: &[u8], sw: u32, sh: u32, region: &ScaleRegion, out: &mut Vec<u8>) {
    let (sw64, sh64) = (u64::from(sw), u64::from(sh));
    let (dw64, dh64) = (u64::from(region.dst_w), u64::from(region.dst_h));
    out.clear();
    out.reserve(region.w as usize * region.h as usize * 4);
    for y in u64::from(region.y0)..u64::from(region.y0) + u64::from(region.h) {
        let row = (y * sh64 / dh64) as usize * sw as usize * 4;
        for x in u64::from(region.x0)..u64::from(region.x0) + u64::from(region.w) {
            let i = row + (x * sw64 / dw64) as usize * 4;
            out.extend_from_slice(src.get(i..i + 4).unwrap_or(&[0; 4]));
        }
    }
}

impl PixelSink for Ue5Backend {
    fn draw_hline(&mut self, x1: i32, x2: i32, y: i32, color: Color) {
        self.fb.hline(x1, x2, y, color);
    }
}

impl SdiCore for Ue5Backend {
    /// Resize the framebuffer. Textures and render targets survive (their
    /// ids stay valid), the clip and translate stacks are reset. Fails
    /// while a render target is bound.
    fn init(&mut self, width: u32, height: u32) -> Result<()> {
        if self.current_render_target.is_some() {
            return Err(OasisError::Backend(
                "init: cannot resize while a render target is bound".into(),
            ));
        }
        self.fb.resize(width, height);
        self.clip_stack.clear();
        self.translate_stack.clear();
        self.dirty = true;
        Ok(())
    }

    fn clear(&mut self, color: Color) -> Result<()> {
        self.fb.clear(color);
        if self.current_render_target.is_none()
            && let Some(log) = &mut self.text_log
        {
            log.drawing.clear();
        }
        self.dirty = true;
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
        if w == 0 || h == 0 || color.a == 0 {
            return Ok(());
        }
        let (tx, ty) = self.translate(x, y);
        self.fb.fill_rect(tx, ty, w, h, color);
        self.dirty = true;
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
        self.draw_text_styled(text, x, y, font_size, color, false, false)
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        self.fb.blit_texture(&tex_data, tex_w, tex_h, tx, ty, w, h);
        self.dirty = true;
        Ok(())
    }

    fn swap_buffers(&mut self) -> Result<()> {
        self.frames_presented += 1;
        if let Some(log) = &mut self.text_log {
            log.presented = std::mem::take(&mut log.drawing);
        }
        Ok(())
    }

    fn load_texture(&mut self, width: u32, height: u32, rgba_data: &[u8]) -> Result<TextureId> {
        validate_rgba_data(width, height, rgba_data)?;

        let texture = Texture {
            width,
            height,
            data: Rc::new(rgba_data.to_vec()),
        };

        for (i, slot) in self.textures.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(texture);
                return Ok(TextureId(i as u64));
            }
        }
        let id = self.textures.len();
        self.textures.push(Some(texture));
        Ok(TextureId(id as u64))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> Result<()> {
        let slot = usize::try_from(tex.0)
            .ok()
            .and_then(|idx| self.textures.get_mut(idx));
        match slot.and_then(Option::take) {
            Some(_) => Ok(()),
            None => Err(texture_not_found(tex)),
        }
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        // Absolute and replacing (SdiCore contract): no translation, no
        // intersection with the clip stack.
        self.fb.set_clip(Some(ClipRect { x, y, w, h }));
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> Result<()> {
        self.fb.set_clip(None);
        Ok(())
    }

    fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        Ok(self.fb.read_pixels(x, y, w, h))
    }

    fn shutdown(&mut self) -> Result<()> {
        // Drop any bound target's parents first so `fb` ends up empty.
        self.render_target_bind_stack.clear();
        self.current_render_target = None;
        self.render_targets.clear();
        self.fb = SoftwareBuffer::new(0, 0);
        self.textures.clear();
        self.clip_stack.clear();
        self.translate_stack.clear();
        self.composite_scratch = Vec::new();
        if let Some(log) = &mut self.text_log {
            *log = TextLog::default();
        }
        log::info!("UE5 backend shut down");
        Ok(())
    }

    /// The bound surface: the framebuffer, or the bound render target.
    fn viewport_size(&self) -> (u32, u32) {
        (self.fb.width(), self.fb.height())
    }
}

// -------------------------------------------------------------------
// SdiShapes: Shape primitives
// -------------------------------------------------------------------

impl SdiShapes for Ue5Backend {
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> Result<()> {
        if radius == 0 || w == 0 || h == 0 {
            return self.fill_rect(x, y, w, h, color);
        }
        let (tx, ty) = self.translate(x, y);
        self.fb.fill_rounded_rect(tx, ty, w, h, radius, color);
        self.dirty = true;
        Ok(())
    }

    fn stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.fb.stroke_rect(tx, ty, w, h, stroke_width, color);
        self.dirty = true;
        Ok(())
    }

    fn draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx1, ty1) = self.translate(x1, y1);
        let (tx2, ty2) = self.translate(x2, y2);
        self.fb.draw_line(tx1, ty1, tx2, ty2, width, color);
        self.dirty = true;
        Ok(())
    }

    fn stroke_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.fb
            .stroke_rounded_rect(tx, ty, w, h, radius, stroke_width, color);
        self.dirty = true;
        Ok(())
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        self.fb.fill_circle(tcx, tcy, radius, color);
        self.dirty = true;
        Ok(())
    }

    fn stroke_circle(
        &mut self,
        cx: i32,
        cy: i32,
        radius: u16,
        stroke_width: u16,
        color: Color,
    ) -> Result<()> {
        let (tcx, tcy) = self.translate(cx, cy);
        self.fb.stroke_circle(tcx, tcy, radius, stroke_width, color);
        self.dirty = true;
        Ok(())
    }

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
        let v0 = self.translate(x1, y1);
        let v1 = self.translate(x2, y2);
        let v2 = self.translate(x3, y3);
        self.fb.fill_triangle(v0, v1, v2, color);
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiVector: even-odd polygon fill (arcs, dashes and polylines use the
// defaults, which decompose into the shapes above)
// -------------------------------------------------------------------

impl SdiVector for Ue5Backend {
    fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
        let (dx, dy) = self.translate_stack.current();
        if (dx, dy) == (0, 0) {
            self.fb.fill_polygon(points, color);
        } else {
            let moved: Vec<(i32, i32)> = points
                .iter()
                .map(|&(x, y)| (x.saturating_add(dx), y.saturating_add(dy)))
                .collect();
            self.fb.fill_polygon(&moved, color);
        }
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiGradients: Gradient fills
// -------------------------------------------------------------------

impl SdiGradients for Ue5Backend {
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &GradientStyle,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.fb.fill_rect_gradient(tx, ty, w, h, gradient);
        self.dirty = true;
        Ok(())
    }

    fn fill_rounded_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        gradient: &GradientStyle,
    ) -> Result<()> {
        let (tx, ty) = self.translate(x, y);
        self.fb
            .fill_rounded_rect_gradient(tx, ty, w, h, radius, gradient);
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiText: Text system (line metrics are the bitmap-font defaults)
// -------------------------------------------------------------------

impl SdiText for Ue5Backend {
    /// Bitmap text scaled to `font_size` with faux bold / italic -- the
    /// same glyph pixels the SDL backend uploads (`oasis_rasterize::glyph_mask`),
    /// each blended once (bold is baked into the mask, not drawn twice).
    fn draw_text_styled(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        bold: bool,
        italic: bool,
    ) -> Result<()> {
        // Fully transparent text (glyph-cache warm-up) paints nothing.
        if let Some(log) = &mut self.text_log
            && color.a > 0
            && !text.is_empty()
        {
            log.drawing.push(DrawnText {
                text: text.to_string(),
                x,
                y,
                font_size,
                color,
            });
        }
        let (tx, ty) = self.translate(x, y);
        self.fb
            .draw_text(text, tx, ty, font_size, color, bold, italic);
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiTextures: Texture operations
// -------------------------------------------------------------------

impl SdiTextures for Ue5Backend {
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
        let (tex_w, _tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(dst_x, dst_y);
        self.fb.blit_texture_sub(
            &tex_data, tex_w, src_x, src_y, src_w, src_h, tx, ty, dst_w, dst_h,
        );
        self.dirty = true;
        Ok(())
    }

    fn blit_tinted(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    ) -> Result<()> {
        let (tex_w, tex_h, tex_data) = self.get_texture_data(tex)?;
        let (tx, ty) = self.translate(x, y);
        self.fb
            .blit_texture_tinted(&tex_data, tex_w, tex_h, tx, ty, w, h, tint);
        self.dirty = true;
        Ok(())
    }
}

// -------------------------------------------------------------------
// SdiClipTransform: Clip and transform stack
// -------------------------------------------------------------------

impl SdiClipTransform for Ue5Backend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let applied = self
            .clip_stack
            .push_translated(&self.translate_stack, x, y, w, h);
        self.fb.set_clip(Some(applied));
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> Result<()> {
        self.fb.set_clip(self.clip_stack.pop());
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<ClipRect> {
        self.clip_stack.current()
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.translate_stack.push(dx, dy);
        Ok(())
    }

    fn pop_translate(&mut self) -> Result<()> {
        self.translate_stack.pop();
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.translate_stack.current()
    }
}

// -------------------------------------------------------------------
// SdiBatch: tighter inner loop than the default — caches the
// translate offset (the default re-derives it per call), batches the
// `dirty` flag flip into a single store at the end, and skips the
// per-rect SdiCore::fill_rect dispatch. The host UE5 process polls
// `oasis_get_dirty` once per tick so coalescing the dirty flip costs
// nothing visible but saves a few writes per frame.
// -------------------------------------------------------------------

impl SdiBatch for Ue5Backend {
    fn submit_rect_batch(&mut self, rects: &[BatchRect]) -> Result<()> {
        if rects.is_empty() {
            return Ok(());
        }
        let mut wrote_any = false;
        for r in rects {
            if r.w == 0 || r.h == 0 || r.color.a == 0 {
                continue;
            }
            // Saturating, like every other draw path: a far off-screen
            // rect must not wrap onto the screen.
            let (x, y) = self.translate_stack.translate(r.x, r.y);
            self.fb.fill_rect(x, y, r.w, r.h, r.color);
            wrote_any = true;
        }
        if wrote_any {
            self.dirty = true;
        }
        Ok(())
    }

    // `submit_text_batch`: the default, one `draw_text_styled` per item
    // (bold baked into the glyph mask, italic honoured).
}

// -------------------------------------------------------------------
// SdiRenderTarget: Offscreen compositing layers
// -------------------------------------------------------------------

impl SdiRenderTarget for Ue5Backend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        if w == 0 || h == 0 {
            return Err(OasisError::Backend(
                format!("create_render_target: zero dimension ({w}x{h})").into(),
            ));
        }
        let id = self.next_render_target_id;
        self.next_render_target_id += 1;
        self.render_targets.insert(id, SoftwareBuffer::new(w, h));
        Ok(RenderTargetId(id))
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        // Take the target's buffer out of storage and swap it with
        // `self.fb`. The previous fb ends up held by the bind stack
        // entry and is swapped back on `unbind_render_target`.
        let mut target_fb = self
            .render_targets
            .remove(&id.0)
            .ok_or_else(|| render_target_not_found(id))?;
        // A target starts unclipped and untranslated (a clip left from
        // its previous bind must not survive).
        target_fb.set_clip(None);
        std::mem::swap(&mut self.fb, &mut target_fb);
        self.render_target_bind_stack.push(BoundFrom {
            prev_id: self.current_render_target,
            parent: target_fb,
            state: DrawState::take(&mut self.clip_stack, &mut self.translate_stack),
        });
        self.current_render_target = Some(id.0);
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        let BoundFrom {
            prev_id,
            mut parent,
            state,
        } = self.render_target_bind_stack.pop().ok_or_else(|| {
            OasisError::Backend("unbind_render_target: bind stack underflow".into())
        })?;
        // Swap the parent surface back in (it kept its own applied clip);
        // the target's buffer ends up in `parent`, which goes back to the
        // pool.
        std::mem::swap(&mut self.fb, &mut parent);
        state.restore(&mut self.clip_stack, &mut self.translate_stack);
        debug_assert!(
            self.current_render_target.is_some(),
            "unbind_render_target called without active target"
        );
        if let Some(active_id) = self.current_render_target {
            self.render_targets.insert(active_id, parent);
        }
        self.current_render_target = prev_id;
        Ok(())
    }

    fn composite_render_target(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        blend: BlendMode,
        opacity: f32,
    ) -> Result<()> {
        debug_assert!(
            (0.0..=1.0).contains(&opacity),
            "opacity must be in [0.0, 1.0], got {opacity}"
        );
        self.composite_scaled(id, dst_x, dst_y, dst_w, dst_h, blend, opacity)
    }

    /// Render targets here hold *straight* alpha: `SoftwareBuffer` blends
    /// over translucent pixels with the full Porter-Duff operator, so a
    /// layer keeps the color and coverage of what was painted into it.
    /// Straight source-over of that layer is the same operation as
    /// premultiplied source-over of the premultiplied pixels a GPU
    /// leaves, so this is the plain composite at full opacity.
    fn composite_render_target_premultiplied(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        self.composite_scaled(id, dst_x, dst_y, dst_w, dst_h, BlendMode::Normal, 1.0)
    }

    fn read_render_target(&mut self, id: RenderTargetId, dst: &mut [u8]) -> Result<()> {
        let src = self
            .render_targets
            .get(&id.0)
            .ok_or_else(|| render_target_not_found(id))?;
        let expected = src.width() as usize * src.height() as usize * 4;
        if dst.len() != expected {
            return Err(OasisError::Backend(
                format!(
                    "read_render_target: buffer is {} bytes, target needs {expected}",
                    dst.len()
                )
                .into(),
            ));
        }
        dst.copy_from_slice(src.data());
        Ok(())
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        if self.current_render_target == Some(id.0)
            || self
                .render_target_bind_stack
                .iter()
                .any(|b| b.prev_id == Some(id.0))
        {
            return Err(OasisError::Backend(
                format!("destroy_render_target: id {id:?} is still bound").into(),
            ));
        }
        if self.render_targets.remove(&id.0).is_none() {
            return Err(render_target_not_found(id));
        }
        Ok(())
    }

    fn supports_render_targets(&self) -> bool {
        true
    }

    fn supports_render_target_readback(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::BatchText;

    #[test]
    fn new_creates_buffer() {
        let backend = Ue5Backend::new(480, 272);
        assert_eq!(backend.buffer().len(), 480 * 272 * 4);
        assert_eq!(backend.dimensions(), (480, 272));
    }

    #[test]
    fn clear_fills_buffer() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.clear(Color::rgb(255, 0, 0)).unwrap();
        assert_eq!(backend.buffer()[0], 255);
        assert_eq!(backend.buffer()[1], 0);
        assert_eq!(backend.buffer()[2], 0);
        assert_eq!(backend.buffer()[3], 255);
        let last = backend.buffer().len() - 4;
        assert_eq!(backend.buffer()[last], 255);
    }

    #[test]
    fn fill_rect_draws_pixels() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect(2, 2, 3, 3, Color::rgb(0, 255, 0))
            .unwrap();
        let offset = (2 * 10 + 2) * 4;
        assert_eq!(backend.buffer()[offset], 0);
        assert_eq!(backend.buffer()[offset + 1], 255);
        assert_eq!(backend.buffer()[0], 0);
        assert_eq!(backend.buffer()[1], 0);
    }

    #[test]
    fn fill_rect_clips_negative() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect(-2, -2, 5, 5, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 255);
    }

    #[test]
    fn draw_text_renders_characters() {
        let mut backend = Ue5Backend::new(100, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_text("A", 0, 0, 8, Color::rgb(255, 255, 255))
            .unwrap();
        let has_white = backend
            .buffer()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|px| px[0] == 255 && px[1] == 255 && px[2] == 255);
        assert!(has_white);
    }

    #[test]
    fn draw_text_scaled() {
        let mut backend = Ue5Backend::new(100, 40);
        backend.clear(Color::BLACK).unwrap();
        backend.draw_text("X", 0, 0, 16, Color::WHITE).unwrap();
        let white_count = backend
            .buffer()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] == 255)
            .count();
        assert!(white_count > 20);
    }

    #[test]
    fn load_and_blit_texture() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        let tex_data = vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let tex_id = backend.load_texture(2, 2, &tex_data).unwrap();
        backend.blit(tex_id, 1, 1, 2, 2).unwrap();
        let offset = (10 + 1) * 4;
        assert_eq!(backend.buffer()[offset], 255);
        assert_eq!(backend.buffer()[offset + 1], 0);
    }

    #[test]
    fn destroy_texture_invalidates() {
        let mut backend = Ue5Backend::new(10, 10);
        let tex_data = vec![0u8; 2 * 2 * 4];
        let tex_id = backend.load_texture(2, 2, &tex_data).unwrap();
        backend.destroy_texture(tex_id).unwrap();
        assert!(backend.blit(tex_id, 0, 0, 2, 2).is_err());
    }

    #[test]
    fn texture_data_size_mismatch() {
        let mut backend = Ue5Backend::new(10, 10);
        assert!(backend.load_texture(2, 2, &[0; 8]).is_err());
    }

    #[test]
    fn dirty_flag_tracking() {
        let mut backend = Ue5Backend::new(4, 4);
        assert!(backend.is_dirty());
        backend.clear_dirty();
        assert!(!backend.is_dirty());
        backend.clear(Color::BLACK).unwrap();
        assert!(backend.is_dirty());
    }

    #[test]
    fn clip_rect_restricts_drawing() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend.set_clip_rect(2, 2, 3, 3).unwrap();
        backend
            .fill_rect(0, 0, 10, 10, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 0);
        let offset = (3 * 10 + 3) * 4;
        assert_eq!(backend.buffer()[offset], 255);

        backend.reset_clip_rect().unwrap();
        backend.fill_rect(0, 0, 1, 1, Color::WHITE).unwrap();
        assert_eq!(backend.buffer()[0], 255);
    }

    #[test]
    fn shutdown_clears_state() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.shutdown().unwrap();
        assert!(backend.buffer().is_empty());
    }

    #[test]
    fn texture_slot_reuse() {
        let mut backend = Ue5Backend::new(4, 4);
        let data = vec![0u8; 4];
        let id0 = backend.load_texture(1, 1, &data).unwrap();
        let id1 = backend.load_texture(1, 1, &data).unwrap();
        backend.destroy_texture(id0).unwrap();
        let id2 = backend.load_texture(1, 1, &data).unwrap();
        assert_eq!(id2.0, id0.0);
        assert_ne!(id1.0, id2.0);
    }

    // -------------------------------------------------------------------
    // Extended primitive tests
    // -------------------------------------------------------------------

    #[test]
    fn fill_rounded_rect_draws_pixels() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rounded_rect(2, 2, 16, 16, 4, Color::rgb(0, 255, 0))
            .unwrap();
        // Center pixel should be green.
        let offset = (10 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);
        // Corner pixel (2,2) should NOT be filled (inside the radius).
        let corner = (2 * 20 + 2) * 4;
        assert_eq!(backend.buffer()[corner], 0);
    }

    #[test]
    fn draw_line_horizontal() {
        let mut backend = Ue5Backend::new(20, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_line(2, 5, 18, 5, 1, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (10, 5) should be red.
        let offset = (5 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset], 255);
    }

    #[test]
    fn draw_line_diagonal() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_line(0, 0, 19, 19, 1, Color::rgb(0, 0, 255))
            .unwrap();
        // Pixel at (10, 10) should be blue.
        let offset = (10 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 2], 255);
    }

    #[test]
    fn fill_circle_draws() {
        let mut backend = Ue5Backend::new(30, 30);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_circle(15, 15, 10, Color::rgb(255, 0, 0))
            .unwrap();
        // Center should be red.
        let offset = (15 * 30 + 15) * 4;
        assert_eq!(backend.buffer()[offset], 255);
    }

    #[test]
    fn fill_triangle_draws() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_triangle(10, 2, 2, 18, 18, 18, Color::rgb(0, 255, 0))
            .unwrap();
        // A point inside the triangle should be green.
        let offset = (14 * 20 + 10) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);
    }

    #[test]
    fn gradient_v_fills() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect_gradient(
                0,
                0,
                10,
                10,
                &GradientStyle::Vertical {
                    top: Color::WHITE,
                    bottom: Color::BLACK,
                },
            )
            .unwrap();
        // Top pixel should be white.
        assert_eq!(backend.buffer()[0], 255);
        // Bottom pixel should be black.
        let last_row = (9 * 10) * 4;
        assert_eq!(backend.buffer()[last_row], 0);
    }

    #[test]
    fn gradient_h_fills() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend
            .fill_rect_gradient(
                0,
                0,
                10,
                10,
                &GradientStyle::Horizontal {
                    left: Color::WHITE,
                    right: Color::BLACK,
                },
            )
            .unwrap();
        // Left pixel should be white.
        assert_eq!(backend.buffer()[0], 255);
        // Right pixel should be black.
        let right = 9 * 4;
        assert_eq!(backend.buffer()[right], 0);
    }

    #[test]
    fn clip_stack_nesting() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        // Push outer clip.
        backend.push_clip_rect(2, 2, 16, 16).unwrap();
        // Push inner clip.
        backend.push_clip_rect(5, 5, 10, 10).unwrap();
        backend
            .fill_rect(0, 0, 20, 20, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (0,0) should be black (outside both clips).
        assert_eq!(backend.buffer()[0], 0);
        // Pixel at (3,3) should be black (outside inner clip).
        let offset = (3 * 20 + 3) * 4;
        assert_eq!(backend.buffer()[offset], 0);
        // Pixel at (7,7) should be red (inside both clips).
        let offset = (7 * 20 + 7) * 4;
        assert_eq!(backend.buffer()[offset], 255);

        // Pop inner clip.
        backend.pop_clip_rect().unwrap();
        backend
            .fill_rect(0, 0, 20, 20, Color::rgb(0, 255, 0))
            .unwrap();
        // Pixel at (3,3) should now be green (inside outer clip).
        let offset = (3 * 20 + 3) * 4;
        assert_eq!(backend.buffer()[offset + 1], 255);

        // Pop outer clip.
        backend.pop_clip_rect().unwrap();
    }

    #[test]
    fn translate_stack_offsets() {
        let mut backend = Ue5Backend::new(20, 20);
        backend.clear(Color::BLACK).unwrap();
        backend.push_translate(5, 5).unwrap();
        // fill_rect at (0,0) should actually draw at (5,5).
        backend
            .fill_rect(0, 0, 2, 2, Color::rgb(255, 0, 0))
            .unwrap();
        // Pixel at (5,5) should be red.
        let offset = (5 * 20 + 5) * 4;
        assert_eq!(backend.buffer()[offset], 255);
        // Pixel at (0,0) should be black.
        assert_eq!(backend.buffer()[0], 0);

        backend.push_translate(3, 3).unwrap();
        assert_eq!(backend.current_translate(), (8, 8));
        backend.pop_translate().unwrap();
        assert_eq!(backend.current_translate(), (5, 5));
        backend.pop_translate().unwrap();
        assert_eq!(backend.current_translate(), (0, 0));
    }

    #[test]
    fn blit_sub_draws_subregion() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 4x4 texture: top-left 2x2 is red, rest is blue.
        let mut tex_data = vec![0u8; 4 * 4 * 4];
        for y in 0..4u32 {
            for x in 0..4u32 {
                let off = ((y * 4 + x) * 4) as usize;
                if x < 2 && y < 2 {
                    tex_data[off] = 255; // R
                    tex_data[off + 3] = 255; // A
                } else {
                    tex_data[off + 2] = 255; // B
                    tex_data[off + 3] = 255; // A
                }
            }
        }
        let tex_id = backend.load_texture(4, 4, &tex_data).unwrap();
        // Blit only the top-left 2x2 subregion.
        backend.blit_sub(tex_id, 0, 0, 2, 2, 0, 0, 2, 2).unwrap();
        // Pixel (0,0) should be red.
        assert_eq!(backend.buffer()[0], 255);
        assert_eq!(backend.buffer()[2], 0);
    }

    #[test]
    fn blit_tinted_applies_color() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        // 1x1 white texture.
        let tex_data = vec![255u8; 4];
        let tex_id = backend.load_texture(1, 1, &tex_data).unwrap();
        // Tint with red.
        backend
            .blit_tinted(tex_id, 0, 0, 1, 1, Color::rgb(255, 0, 0))
            .unwrap();
        assert_eq!(backend.buffer()[0], 255); // R
        assert_eq!(backend.buffer()[1], 0); // G
        assert_eq!(backend.buffer()[2], 0); // B
    }

    #[test]
    fn rect_batch_far_off_screen_does_not_wrap() {
        let mut backend = Ue5Backend::new(10, 10);
        backend.clear(Color::BLACK).unwrap();
        backend.push_translate(i32::MAX - 2, 0).unwrap();
        let rect = BatchRect {
            x: 5,
            y: 0,
            w: 4,
            h: 4,
            color: Color::WHITE,
        };
        backend.submit_rect_batch(&[rect]).unwrap();
        assert!(
            backend.buffer().chunks(4).all(|p| p[0] == 0),
            "an off-screen rect wrapped onto the screen"
        );
    }

    #[test]
    fn stroke_circle_draws_ring() {
        let mut backend = Ue5Backend::new(30, 30);
        backend.clear(Color::BLACK).unwrap();
        backend
            .stroke_circle(15, 15, 10, 2, Color::rgb(0, 255, 0))
            .unwrap();
        // Center should be black (hollow).
        let center = (15 * 30 + 15) * 4;
        assert_eq!(backend.buffer()[center], 0);
        // Edge pixel should be green.
        let edge = (15 * 30 + 25) * 4;
        assert_eq!(backend.buffer()[edge + 1], 255);
    }

    #[test]
    fn text_measurement() {
        // Same line metrics as the SDL and WASM backends:
        // ceil(max(fs, 8) * 1.2) and ceil(max(fs, 8) * 0.85).
        let backend = Ue5Backend::new(10, 10);
        assert_eq!(backend.measure_text_height(8), 10);
        assert_eq!(backend.measure_text_height(16), 20);
        assert_eq!(backend.measure_text_height(12), 15);
        assert_eq!(backend.font_ascent(8), 7);
        assert_eq!(backend.font_ascent(16), 14);
        let (w, h) = backend.measure_text_extents("AB", 8);
        assert_eq!(w, 14); // proportional: A(7)+B(7) = 14
        assert_eq!(h, 10);
    }

    /// Regression: text at sizes that are not a multiple of 8 was drawn
    /// with the 8x8 glyphs scaled by `font_size / 8` rounded down (12 px
    /// text came out 8 px tall and 8/12 as wide as `measure_text`), so
    /// every label measured with `measure_text` was laid out for text
    /// wider than what got drawn. Glyphs now scale to the exact size.
    #[test]
    fn fractional_font_size_draws_measured_width_and_height() {
        let mut backend = Ue5Backend::new(200, 40);
        backend.clear(Color::BLACK).unwrap();
        backend.draw_text("HHHH", 0, 0, 12, Color::WHITE).unwrap();
        let (mut max_x, mut max_y) = (0, 0);
        for (i, px) in backend.buffer().as_chunks::<4>().0.iter().enumerate() {
            if px[0] == 255 {
                max_x = max_x.max(i % 200);
                max_y = max_y.max(i / 200);
            }
        }
        let measured = backend.measure_text("HHHH", 12) as usize;
        assert!(
            max_x + 1 > measured - 3 && max_x < measured,
            "ink ends at {max_x}, measured {measured}"
        );
        assert!(max_y >= 8, "12 px glyphs only {} px tall", max_y + 1);
    }

    /// Regression: bold text was drawn twice (shifted 1 px), so every
    /// overlapping pixel of translucent bold text was blended twice.
    #[test]
    fn translucent_bold_text_blends_once() {
        let mut backend = Ue5Backend::new(60, 20);
        backend.clear(Color::BLACK).unwrap();
        backend
            .draw_text_styled("Hi", 0, 0, 16, Color::rgba(255, 255, 255, 128), true, false)
            .unwrap();
        let mut values: Vec<u8> = backend
            .buffer()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0])
            .filter(|&v| v != 0)
            .collect();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values, vec![128]);
    }

    // ---------------------------------------------------------------
    // RGBA pixel format / set_pixel color tests
    // ---------------------------------------------------------------

    #[test]
    fn rgba_buffer_layout_red() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(255, 0, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 255); // R
        assert_eq!(buf[1], 0); // G
        assert_eq!(buf[2], 0); // B
        assert_eq!(buf[3], 255); // A
    }

    #[test]
    fn rgba_buffer_layout_green() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(0, 255, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 255);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_blue() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(0, 0, 255)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 255);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_white() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::WHITE).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 255);
        assert_eq!(buf[1], 255);
        assert_eq!(buf[2], 255);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_layout_black() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::BLACK).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 255);
    }

    #[test]
    fn rgba_buffer_transparent_clear() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgba(0, 0, 0, 0)).unwrap();
        let buf = backend.buffer();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn set_pixel_alpha_blend() {
        let mut backend = Ue5Backend::new(1, 1);
        // Start with white background.
        backend.clear(Color::WHITE).unwrap();
        // Draw 50% transparent red over it.
        backend.set_pixel(0, 0, Color::rgba(255, 0, 0, 128));
        let buf = backend.buffer();
        assert!(buf[0] > 200); // R stays high
        assert!(buf[1] > 100 && buf[1] < 140); // G blended ~127
        assert!(buf[2] > 100 && buf[2] < 140); // B blended ~127
        assert_eq!(buf[3], 255); // A always 255 after blend
    }

    #[test]
    fn set_pixel_fully_transparent_no_change() {
        let mut backend = Ue5Backend::new(1, 1);
        backend.clear(Color::rgb(42, 84, 126)).unwrap();
        backend.set_pixel(0, 0, Color::rgba(255, 255, 255, 0));
        let buf = backend.buffer();
        assert_eq!(buf[0], 42);
        assert_eq!(buf[1], 84);
        assert_eq!(buf[2], 126);
    }

    #[test]
    fn set_pixel_out_of_bounds_no_crash() {
        let mut backend = Ue5Backend::new(4, 4);
        backend.clear(Color::BLACK).unwrap();
        let before: Vec<u8> = backend.buffer().to_vec();
        backend.set_pixel(-1, 0, Color::WHITE);
        backend.set_pixel(0, -1, Color::WHITE);
        backend.set_pixel(4, 0, Color::WHITE);
        backend.set_pixel(0, 4, Color::WHITE);
        assert_eq!(backend.buffer(), before.as_slice());
    }

    #[test]
    fn rgba_round_trip_encode_decode() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(123, 45, 67, 255);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        let decoded = Color::rgba(buf[0], buf[1], buf[2], buf[3]);
        assert_eq!(decoded, c);
    }

    #[test]
    fn rgba_round_trip_all_channels_max() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(255, 255, 255, 255);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        assert_eq!(Color::rgba(buf[0], buf[1], buf[2], buf[3]), c);
    }

    #[test]
    fn rgba_round_trip_all_channels_min() {
        let mut backend = Ue5Backend::new(1, 1);
        let c = Color::rgba(0, 0, 0, 0);
        backend.clear(c).unwrap();
        let buf = backend.buffer();
        assert_eq!(Color::rgba(buf[0], buf[1], buf[2], buf[3]), c);
    }

    fn synthetic_rects(n: usize) -> Vec<oasis_types::backend::BatchRect> {
        (0..n)
            .map(|i| oasis_types::backend::BatchRect {
                x: ((i * 13) % 470) as i32,
                y: ((i * 7) % 262) as i32,
                w: 8,
                h: 8,
                color: Color::rgba(
                    (i * 11) as u8,
                    (i * 17) as u8,
                    (i * 23) as u8,
                    if i % 5 == 0 { 200 } else { 255 },
                ),
            })
            .collect()
    }

    /// Visual parity guarantee: the override must produce a pixel buffer
    /// that's bit-identical to the default `fill_rect` loop over the same
    /// inputs. Without this, a screenshot regression somewhere downstream
    /// (browser display list, file-manager grid) could pass on native and
    /// fail on WASM, or vice versa.
    #[test]
    fn submit_rect_batch_matches_default_loop() {
        use oasis_types::backend::SdiBatch;
        let rects = synthetic_rects(200);

        let mut a = Ue5Backend::new(480, 272);
        a.clear(Color::BLACK).unwrap();
        for r in &rects {
            a.fill_rect(r.x, r.y, r.w, r.h, r.color).unwrap();
        }

        let mut b = Ue5Backend::new(480, 272);
        b.clear(Color::BLACK).unwrap();
        b.submit_rect_batch(&rects).unwrap();

        assert_eq!(
            a.buffer(),
            b.buffer(),
            "submit_rect_batch override produced a different framebuffer \
             than the default fill_rect loop"
        );
    }

    /// Translate offsets must be applied per item by both paths or the
    /// override will silently shift everything by the wrong amount.
    #[test]
    fn submit_rect_batch_honors_translate_stack() {
        use oasis_types::backend::{SdiBatch, SdiClipTransform};
        let rects = synthetic_rects(50);

        let mut a = Ue5Backend::new(480, 272);
        a.clear(Color::BLACK).unwrap();
        a.push_translate(7, 11).unwrap();
        for r in &rects {
            a.fill_rect(r.x, r.y, r.w, r.h, r.color).unwrap();
        }
        a.pop_translate().unwrap();

        let mut b = Ue5Backend::new(480, 272);
        b.clear(Color::BLACK).unwrap();
        b.push_translate(7, 11).unwrap();
        b.submit_rect_batch(&rects).unwrap();
        b.pop_translate().unwrap();

        assert_eq!(a.buffer(), b.buffer(), "translate stack diverged");
    }

    fn px(b: &Ue5Backend, x: u32, y: u32) -> [u8; 4] {
        let (w, _) = b.dimensions();
        let i = ((y * w + x) * 4) as usize;
        let p = &b.buffer()[i..i + 4];
        [p[0], p[1], p[2], p[3]]
    }

    /// A bound target starts unclipped and untranslated; the unbind brings
    /// the parent's clip and translate stacks back, so a later pop pops the
    /// parent's clip in parent coordinates.
    #[test]
    fn render_target_bind_saves_and_restores_draw_state() {
        let mut b = Ue5Backend::new(40, 40);
        b.clear(Color::BLACK).unwrap();
        b.push_translate(5, 5).unwrap();
        b.push_clip_rect(0, 0, 10, 10).unwrap();
        b.push_clip_rect(2, 2, 4, 4).unwrap();

        let rt = b.create_render_target(20, 20).unwrap();
        b.bind_render_target(rt).unwrap();
        assert_eq!(b.current_translate(), (0, 0));
        assert_eq!(b.current_clip_rect(), None);
        assert_eq!(b.viewport_size(), (20, 20));
        b.fill_rect(0, 0, 20, 20, Color::WHITE).unwrap();
        b.unbind_render_target().unwrap();

        assert_eq!(b.current_translate(), (5, 5));
        assert_eq!(b.current_clip_rect(), Some(ClipRect::new(7, 7, 4, 4)));
        assert_eq!(b.viewport_size(), (40, 40));
        b.pop_clip_rect().unwrap();
        assert_eq!(b.current_clip_rect(), Some(ClipRect::new(5, 5, 10, 10)));
        b.fill_rect(-5, -5, 40, 40, Color::rgb(255, 0, 0)).unwrap();
        assert_eq!(px(&b, 5, 5), [255, 0, 0, 255]);
        assert_eq!(px(&b, 15, 15), [0, 0, 0, 255], "parent clip restored");
        b.pop_clip_rect().unwrap();
        b.pop_translate().unwrap();
        b.destroy_render_target(rt).unwrap();
    }

    /// The target's buffer keeps no clip from an earlier bind.
    #[test]
    fn render_target_rebind_starts_unclipped() {
        let mut b = Ue5Backend::new(20, 20);
        let rt = b.create_render_target(10, 10).unwrap();
        b.bind_render_target(rt).unwrap();
        b.set_clip_rect(0, 0, 1, 1).unwrap();
        b.unbind_render_target().unwrap();
        b.bind_render_target(rt).unwrap();
        b.clear(Color::TRANSPARENT).unwrap();
        b.fill_rect(0, 0, 10, 10, Color::rgb(0, 255, 0)).unwrap();
        b.unbind_render_target().unwrap();
        let mut out = vec![0u8; 10 * 10 * 4];
        b.read_render_target(rt, &mut out).unwrap();
        assert_eq!(&out[out.len() - 4..], &[0, 255, 0, 255]);
        // Wrong-size buffers are rejected rather than partly filled.
        assert!(b.read_render_target(rt, &mut [0u8; 8]).is_err());
    }

    #[test]
    fn composite_scales_to_destination() {
        let mut b = Ue5Backend::new(40, 20);
        b.clear(Color::BLACK).unwrap();
        let rt = b.create_render_target(4, 2).unwrap();
        b.bind_render_target(rt).unwrap();
        b.fill_rect(0, 0, 2, 2, Color::rgb(255, 0, 0)).unwrap();
        b.fill_rect(2, 0, 2, 2, Color::rgb(0, 0, 255)).unwrap();
        b.unbind_render_target().unwrap();
        b.composite_render_target(rt, 0, 0, 40, 20, BlendMode::Normal, 1.0)
            .unwrap();
        assert_eq!(px(&b, 19, 19), [255, 0, 0, 255]);
        assert_eq!(px(&b, 20, 0), [0, 0, 255, 255]);
        assert_eq!(px(&b, 39, 19), [0, 0, 255, 255]);
    }

    /// A scaled composite far larger than the screen only resamples the
    /// visible part (it used to allocate `dst_w * dst_h * 4` bytes first)
    /// and samples the same source pixels as an on-screen one.
    #[test]
    fn huge_scaled_composite_resamples_only_the_visible_part() {
        let mut b = Ue5Backend::new(40, 20);
        b.clear(Color::BLACK).unwrap();
        let rt = b.create_render_target(4, 2).unwrap();
        b.bind_render_target(rt).unwrap();
        b.fill_rect(0, 0, 2, 2, Color::rgb(255, 0, 0)).unwrap();
        b.fill_rect(2, 0, 2, 2, Color::rgb(0, 0, 255)).unwrap();
        b.unbind_render_target().unwrap();
        // 4x2 scaled to 200_000 x 100_000 (80 GB unclipped), placed so the
        // left/right halves meet at screen column 20.
        b.composite_render_target(
            rt,
            -99_980,
            -50_000,
            200_000,
            100_000,
            BlendMode::Normal,
            1.0,
        )
        .unwrap();
        assert!(b.composite_scratch.len() <= 40 * 20 * 4);
        assert_eq!(px(&b, 19, 0), [255, 0, 0, 255]);
        assert_eq!(px(&b, 20, 19), [0, 0, 255, 255]);
        // Entirely off screen: nothing drawn, nothing resampled.
        b.clear(Color::BLACK).unwrap();
        b.composite_render_target(rt, 50, 0, 1 << 30, 1 << 30, BlendMode::Normal, 1.0)
            .unwrap();
        assert_eq!(px(&b, 39, 0), [0, 0, 0, 255]);
    }

    /// Baking translucent draws into a target and compositing it with the
    /// premultiplied operator looks like drawing them directly.
    #[test]
    fn premultiplied_composite_matches_immediate_drawing() {
        let draw = |b: &mut Ue5Backend| {
            b.fill_rect(2, 2, 20, 10, Color::rgba(230, 40, 30, 128))
                .unwrap();
            b.fill_circle(16, 10, 6, Color::rgba(40, 90, 240, 160))
                .unwrap();
        };
        let mut direct = Ue5Backend::new(32, 24);
        direct.clear(Color::rgb(24, 32, 48)).unwrap();
        draw(&mut direct);

        let mut baked = Ue5Backend::new(32, 24);
        let rt = baked.create_render_target(32, 24).unwrap();
        baked.bind_render_target(rt).unwrap();
        baked.clear(Color::TRANSPARENT).unwrap();
        draw(&mut baked);
        baked.unbind_render_target().unwrap();
        baked.clear(Color::rgb(24, 32, 48)).unwrap();
        baked
            .composite_render_target_premultiplied(rt, 0, 0, 32, 24)
            .unwrap();

        for (a, b) in direct
            .buffer()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(baked.buffer().as_chunks::<4>().0)
        {
            for c in 0..3 {
                assert!(a[c].abs_diff(b[c]) <= 2, "direct {a:?} vs baked {b:?}");
            }
        }
    }

    #[test]
    fn render_target_errors_are_typed() {
        use oasis_types::error::BackendError;
        let mut b = Ue5Backend::new(4, 4);
        let missing = RenderTargetId(99);
        for r in [
            b.bind_render_target(missing),
            b.destroy_render_target(missing),
            b.composite_render_target_premultiplied(missing, 0, 0, 1, 1),
        ] {
            assert!(matches!(
                r,
                Err(OasisError::Backend(BackendError::RenderTargetNotFound {
                    id: 99
                }))
            ));
        }
    }

    /// `init` resizes in place: texture ids stay valid (the e2e harness
    /// resizes the window with textures loaded).
    #[test]
    fn init_keeps_textures_and_resets_stacks() {
        let mut b = Ue5Backend::new(8, 8);
        let tex = b.load_texture(1, 1, &[1, 2, 3, 255]).unwrap();
        b.push_translate(3, 3).unwrap();
        b.push_clip_rect(0, 0, 1, 1).unwrap();
        b.init(16, 12).unwrap();
        assert_eq!(b.dimensions(), (16, 12));
        assert_eq!(b.current_translate(), (0, 0));
        assert_eq!(b.current_clip_rect(), None);
        b.blit(tex, 15, 11, 1, 1).unwrap();
        assert_eq!(px(&b, 15, 11), [1, 2, 3, 255]);
        assert_eq!(b.live_textures(), 1);
    }

    #[test]
    fn shutdown_releases_render_targets() {
        let mut b = Ue5Backend::new(8, 8);
        let a = b.create_render_target(4, 4).unwrap();
        let _c = b.create_render_target(4, 4).unwrap();
        b.bind_render_target(a).unwrap();
        assert_eq!(b.live_render_targets(), 2);
        b.shutdown().unwrap();
        assert_eq!(b.live_render_targets(), 0);
        assert!(b.bind_render_target(a).is_err());
    }

    #[test]
    fn text_log_records_frames() {
        let mut b = Ue5Backend::new(64, 32);
        b.draw_text("before", 0, 0, 8, Color::WHITE).unwrap();
        assert!(b.presented_text().is_empty(), "off by default");
        b.enable_text_log();
        b.clear(Color::BLACK).unwrap();
        b.push_translate(4, 4).unwrap();
        b.draw_text("hello", 1, 2, 8, Color::WHITE).unwrap();
        b.pop_translate().unwrap();
        b.draw_text("invisible", 0, 0, 8, Color::TRANSPARENT)
            .unwrap();
        // Clearing a bound render target does not reset the frame's log.
        let rt = b.create_render_target(4, 4).unwrap();
        b.bind_render_target(rt).unwrap();
        b.clear(Color::TRANSPARENT).unwrap();
        b.unbind_render_target().unwrap();
        b.submit_text_batch(
            &[BatchText {
                text: "batched",
                x: 0,
                y: 10,
                color: Color::WHITE,
            }],
            8,
            true,
            false,
        )
        .unwrap();
        b.swap_buffers().unwrap();
        let texts: Vec<_> = b.presented_text().iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["hello", "batched"]);
        assert_eq!((b.presented_text()[0].x, b.presented_text()[0].y), (1, 2));
        assert_eq!(b.frames_presented(), 1);
    }
}
