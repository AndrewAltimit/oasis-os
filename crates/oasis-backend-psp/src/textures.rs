//! Texture storage and volatile memory management.

use std::alloc::{Layout, alloc, dealloc};
use std::ptr;

use oasis_core::backend::TextureId;

use crate::PspBackend;

/// A loaded texture stored in RAM or volatile memory.
pub(crate) struct Texture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Power-of-2 buffer width for GU.
    pub(crate) buf_w: u32,
    /// Power-of-2 buffer height for GU.
    pub(crate) buf_h: u32,
    /// 16-byte aligned pixel data pointer (RAM or volatile mem).
    pub(crate) data: *mut u8,
    /// Layout used for deallocation (only valid if `in_volatile` is false).
    pub(crate) layout: Layout,
    /// True if data lives in volatile memory (not individually freeable).
    pub(crate) in_volatile: bool,
}

/// Simple bump allocator over the volatile memory region.
///
/// On PSP-2000 and later, `sceKernelVolatileMemTryLock` provides access to an
/// extra 4MB of RAM. This allocator hands out 16-byte-aligned chunks from that
/// region for texture storage, freeing main heap for application data.
///
/// The volatile memory is unlocked on drop via `sceKernelVolatileMemUnlock`,
/// ensuring the 4MB region is returned to the system even if the application
/// exits abnormally (panic unwind).
pub(crate) struct VolatileAllocator {
    base: *mut u8,
    pub(crate) size: usize,
    offset: usize,
}

impl Drop for VolatileAllocator {
    fn drop(&mut self) {
        // SAFETY: Unlock the volatile memory region that was acquired via
        // sceKernelVolatileMemTryLock in PspBackend::new(). The lock ID
        // is always 0 (the only volatile memory block on PSP).
        unsafe {
            psp::sys::sceKernelVolatileMemUnlock(0);
        }
    }
}

impl VolatileAllocator {
    /// Create a new allocator over the given memory region.
    #[allow(dead_code)]
    pub(crate) fn new(base: *mut u8, size: usize) -> Self {
        Self {
            base,
            size,
            offset: 0,
        }
    }

    /// Allocate `len` bytes with 16-byte alignment. Returns null on OOM.
    pub(crate) fn alloc(&mut self, len: usize) -> *mut u8 {
        let aligned = (self.offset + 15) & !15;
        if aligned + len > self.size {
            return ptr::null_mut();
        }
        // SAFETY: `aligned` is within `[0, self.size)` (checked above),
        // and `self.base` points to a valid region of `self.size` bytes
        // obtained from `sceKernelVolatileMemTryLock`.
        let ptr = unsafe { self.base.add(aligned) };
        self.offset = aligned + len;
        ptr
    }

    /// Reset the allocator, freeing all allocations.
    #[allow(dead_code)]
    pub(crate) fn reset(&mut self) {
        self.offset = 0;
    }

    /// Bytes remaining.
    pub(crate) fn remaining(&self) -> usize {
        let aligned = (self.offset + 15) & !15;
        self.size.saturating_sub(aligned)
    }
}

impl PspBackend {
    /// Load raw RGBA pixel data as a texture.
    ///
    /// The data is copied into a power-of-2 aligned buffer suitable for the GU.
    /// On PSP-2000+, textures are allocated from the extra 4MB volatile memory
    /// when available, falling back to the main heap.
    pub fn load_texture_inner(
        &mut self,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) -> Option<TextureId> {
        let expected = (width * height * 4) as usize;
        if rgba_data.len() != expected {
            return None;
        }

        let buf_w = width.next_power_of_two();
        let buf_h = height.next_power_of_two();
        let buf_size = (buf_w * buf_h * 4) as usize;

        // Try volatile memory first, fall back to main heap.
        let (data, layout, in_volatile) = if let Some(ref mut va) = self.volatile_alloc {
            let p = va.alloc(buf_size);
            if !p.is_null() {
                (p, Layout::new::<u8>(), true)
            } else {
                let layout = Layout::from_size_align(buf_size, 16).ok()?;
                // SAFETY: Layout is valid (checked by from_size_align). 16-byte aligned
                // for GU texture requirements. Null check follows.
                let p = unsafe { alloc(layout) };
                if p.is_null() {
                    return None;
                }
                (p, layout, false)
            }
        } else {
            let layout = Layout::from_size_align(buf_size, 16).ok()?;
            // SAFETY: Layout is valid (checked by from_size_align). 16-byte aligned
            // for GU texture requirements. Null check follows.
            let p = unsafe { alloc(layout) };
            if p.is_null() {
                return None;
            }
            (p, layout, false)
        };

        // Zero the buffer first (for padding areas).
        // SAFETY: `data` was just allocated with `buf_size` bytes and
        // confirmed non-null above. from_raw_parts_mut is valid.
        unsafe {
            // Manual zero loop to avoid core::ptr::write_bytes (see MEMORY.md).
            let slice = std::slice::from_raw_parts_mut(data, buf_size);
            for byte in slice.iter_mut() {
                *byte = 0;
            }
        }

        // Copy source rows into the power-of-2 buffer.
        // Use DMA for large rows (>= 1 KB) to offload the CPU.
        let src_stride = (width * 4) as usize;
        let dst_stride = (buf_w * 4) as usize;
        let use_dma = src_stride >= 1024;
        if use_dma {
            // SAFETY: Writeback source data from CPU cache to RAM before DMA
            // reads it, and writeback+invalidate destination so stale cache
            // lines don't overwrite DMA results later.
            unsafe {
                psp::cache::dcache_writeback_invalidate_range(
                    rgba_data.as_ptr() as *const std::ffi::c_void,
                    rgba_data.len() as u32,
                );
                psp::cache::dcache_writeback_invalidate_range(
                    data as *const std::ffi::c_void,
                    buf_size as u32,
                );
            }
        }
        for row in 0..height as usize {
            // SAFETY: src and dst are valid pointers within allocated texture
            // and RGBA data buffers. Row indices are within bounds.
            unsafe {
                let src = rgba_data.as_ptr().add(row * src_stride);
                let dst = data.add(row * dst_stride);
                if use_dma {
                    // SAFETY: src and dst are valid, non-overlapping, and
                    // src_stride > 0. Cache coherency handled above.
                    if psp::dma::memcpy_dma(dst, src, src_stride as u32).is_ok() {
                        continue;
                    }
                    // DMA failed — fall through to CPU copy.
                }
                // Fallback: CPU copy for small rows or DMA failure.
                ptr::copy_nonoverlapping(src, dst, src_stride);
            }
        }

        let texture = Texture {
            width,
            height,
            buf_w,
            buf_h,
            data,
            layout,
            in_volatile,
        };

        // Reuse free slot.
        for (i, slot) in self.textures.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(texture);
                return Some(TextureId(i as u64));
            }
        }
        let id = self.textures.len();
        self.textures.push(Some(texture));
        Some(TextureId(id as u64))
    }

    /// Update an existing texture's pixel data in-place.
    ///
    /// Copies new RGBA data into the texture's existing buffer and flushes
    /// the D-cache so the GU sees the updated pixels. Dimensions must match
    /// the original texture — no reallocation is performed.
    pub fn update_texture_data(&mut self, tex: TextureId, rgba_data: &[u8]) {
        let idx = tex.0 as usize;
        let Some(Some(texture)) = self.textures.get(idx) else {
            return;
        };
        let width = texture.width;
        let height = texture.height;
        let buf_w = texture.buf_w;
        let data = texture.data;
        let src_stride = (width * 4) as usize;
        let dst_stride = (buf_w * 4) as usize;

        if rgba_data.len() < (width * height * 4) as usize {
            return;
        }

        // SAFETY: texture.data was allocated in load_texture_inner with
        // buf_w * buf_h * 4 bytes. Source data length is checked above.
        unsafe {
            if src_stride == dst_stride {
                let total = src_stride * height as usize;
                ptr::copy_nonoverlapping(rgba_data.as_ptr(), data, total);
                psp::cache::dcache_writeback_range(data as *const core::ffi::c_void, total as u32);
            } else {
                for row in 0..height as usize {
                    let src = rgba_data.as_ptr().add(row * src_stride);
                    let dst = data.add(row * dst_stride);
                    ptr::copy_nonoverlapping(src, dst, src_stride);
                }
                let total = dst_stride * height as usize;
                psp::cache::dcache_writeback_range(data as *const core::ffi::c_void, total as u32);
            }
        }
    }

    /// Destroy a loaded texture, freeing its memory.
    ///
    /// Textures in volatile memory are not individually freed (the bump
    /// allocator reclaims them all at once on reset). The video texture is
    /// routed through [`Self::free_video_texture`] so it is never freed
    /// while the video thread may be writing into it.
    ///
    /// Returns `false` when `tex` is not a live texture (never loaded, or
    /// already destroyed); `SdiCore::destroy_texture` turns that into
    /// `BackendError::TextureNotFound`, as every backend does.
    pub fn destroy_texture_inner(&mut self, tex: TextureId) -> bool {
        if self.video_tex == Some(tex) {
            self.free_video_texture();
            return true;
        }
        let idx = tex.0 as usize;
        match self.textures.get_mut(idx).and_then(Option::take) {
            Some(texture) => {
                free_texture_storage(texture);
                true
            },
            None => false,
        }
    }

    // -----------------------------------------------------------------------
    // Video frame texture: the video thread's CSC target
    // -----------------------------------------------------------------------
    //
    // One buffer, written by the video thread (`sceMpegBaseCscAvc` straight
    // into it, see `crate::video::VIDEO_TEX_PTR`) and sampled by the GE on
    // the main thread. Never freed or reused while the video thread may be
    // writing it: every release goes through
    // `crate::video::retire_video_texture_buffer` (unpublish, then wait for
    // the decoder's hazard pointer to clear), and a buffer still held after
    // the bounded wait is parked in `video_tex_retired` and freed later.
    //
    // Layout (`Texture` fields for the video texture):
    // * `width` = `buf_w` = 512: the CSC stride, a power of two as the GE
    //   requires for the texture buffer width (`tbw`).
    // * `height` = rows of CSC output it holds, `ceil16(picture height)`
    //   (the ME converts whole macroblocks). The allocation is
    //   `512 × (height + VIDEO_TEX_GUARD_ROWS) × 4` bytes — sized by rows,
    //   not padded to a power of two (480×272 needs 557 KB, not the 1 MB
    //   a 512×512 buffer would cost).
    // * `buf_h` = `height.next_power_of_two()`: only the size declared to
    //   `sceGuTexImage`, whose height must be a power of two. The GE fetches
    //   only the texels the UVs address, and the video blit's V range is
    //   `[0, video_content_h]` with `video_content_h <= height`
    //   (`set_video_content_size` clamps it; until the first frame the blit
    //   uses `height`). Nearest sampling reads rows `0..content_h`; bilinear
    //   (`blit_scaled`) may also touch row `content_h`, which is at most the
    //   zeroed guard row. So the GE never reads past the allocation even
    //   though the declared texture is taller.

    /// Allocate (or reuse) the video frame texture: `width` is the CSC
    /// stride (512) and `height` the tallest picture it must hold. Call when
    /// the user tunes, while partition memory is still free; publishes the
    /// buffer as the video thread's CSC target.
    ///
    /// A current buffer that is already big enough is zeroed and reused
    /// (volatile memory cannot be returned to the bump allocator, so
    /// reallocating it every tune would leak it). A smaller one is retired
    /// safely and replaced. Buffers are 16-byte aligned for the GE and
    /// zeroed at allocation (stride padding and guard rows stay zero).
    pub fn alloc_video_texture(&mut self, width: u32, height: u32) -> Option<TextureId> {
        self.reap_retired_video_textures();

        let buf_w = width.next_power_of_two();
        let rows = crate::video::video_tex_rows(height);

        if let Some(id) = self.video_tex
            && let Some(Some(tex)) = self.textures.get(id.0 as usize)
            && tex.buf_w == buf_w
            && tex.height >= rows
        {
            let (data, size) = (tex.data, video_tex_capacity(tex));
            if crate::video::retire_video_texture_buffer(data) {
                // SAFETY: `data` is this texture's `size`-byte allocation and
                // the video thread no longer writes it (retired above).
                unsafe { zero_bytes(data, size) };
                crate::video::publish_video_texture_buffer(data, size);
                return Some(id);
            }
            // Still held by a stuck decode: replace it below instead.
        }
        self.release_video_texture();

        let alloc_rows = rows + VIDEO_TEX_GUARD_ROWS;
        let buf_size = (buf_w * alloc_rows * 4) as usize;
        let (data, layout, in_volatile) = self.alloc_texture_storage(buf_size)?;
        // SAFETY: `data` is a fresh `buf_size`-byte allocation.
        unsafe { zero_bytes(data, buf_size) };

        let texture = Texture {
            width: buf_w,
            height: rows,
            buf_w,
            buf_h: rows.next_power_of_two(),
            data,
            layout,
            in_volatile,
        };
        let id = match self.textures.iter().position(Option::is_none) {
            Some(i) => {
                self.textures[i] = Some(texture);
                TextureId(i as u64)
            },
            None => {
                self.textures.push(Some(texture));
                TextureId((self.textures.len() - 1) as u64)
            },
        };
        self.video_tex = Some(id);
        self.video_tex_w = buf_w;
        self.video_tex_h = rows;
        // Publish the buffer for the video thread's CSC to write into
        // directly (zero-copy).
        crate::video::publish_video_texture_buffer(data, buf_size);
        Some(id)
    }

    /// Update the persistent video texture with new RGBA frame data.
    ///
    /// `width` is the source row stride in pixels and `height` the picture
    /// height. In the zero-copy case (`rgba_data` is the texture buffer the
    /// video thread's CSC wrote into, from `video::frame_pixels_raw`) this
    /// is a no-op; otherwise it copies the rows + flushes the D-cache.
    /// Alpha fixup is NOT performed; the GU blit uses
    /// `TextureColorComponent::Rgb` to ignore the alpha channel.
    /// Returns the TextureId on success.
    pub fn update_video_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) -> Option<TextureId> {
        self.reap_retired_video_textures();

        // Reuse the existing buffer if it can hold this frame (the tune
        // path sizes it for the selected episode). Reallocate only when
        // there is no texture yet or the frame is wider / taller.
        let current = self
            .video_tex
            .and_then(|id| self.textures.get(id.0 as usize))
            .and_then(|slot| slot.as_ref())
            .map(|tex| (tex.buf_w, tex.height, tex.data));
        let needs_alloc = match current {
            Some((bw, rows, _)) => width > bw || crate::video::video_tex_rows(height) > rows,
            None => true,
        };
        if needs_alloc {
            // In the zero-copy case `rgba_data` borrows the current texture
            // buffer, which reallocating retires: skip this frame's pixels
            // rather than read a retired buffer. The decoder writes the next
            // frame into the new buffer.
            let aliases_current =
                current.is_some_and(|(_, _, data)| core::ptr::eq(rgba_data.as_ptr(), data));
            if aliases_current {
                return self.alloc_video_texture(width, height);
            }
            self.alloc_video_texture(width, height)?;
        }

        let tex_id = self.video_tex?;
        let texture = self.textures.get(tex_id.0 as usize)?.as_ref()?;

        let buf_w = texture.buf_w;
        let data = texture.data;

        // Zero-copy fast path: the video thread's CSC wrote directly into
        // this texture's buffer (uncached, so no cache writeback needed).
        if core::ptr::eq(rgba_data.as_ptr(), data) {
            return Some(tex_id);
        }

        let src_stride = (width * 4) as usize;
        let dst_stride = (buf_w * 4) as usize;
        let total = dst_stride * height as usize;

        // Copy path for sources other than the in-place CSC output. When
        // source stride matches texture stride, do a single bulk copy.
        if src_stride == dst_stride && rgba_data.len() >= total {
            // SAFETY: the buffer holds buf_w * (rows + guard) * 4 >= `total`
            // bytes (height <= rows, checked by `needs_alloc`) and
            // `rgba_data` has at least `total` (checked above); distinct
            // buffers (the aliasing case returned early).
            unsafe {
                ptr::copy_nonoverlapping(rgba_data.as_ptr(), data, total);
                psp::cache::dcache_writeback_range(data as *const core::ffi::c_void, total as u32);
            }
        } else {
            // Fallback: row-by-row copy (no alpha fixup).
            if rgba_data.len() < src_stride * height as usize || src_stride > dst_stride {
                return Some(tex_id);
            }
            // SAFETY: bounds checked just above: each source row is inside
            // `rgba_data` and each destination row (src_stride <= dst_stride)
            // inside the height <= rows rows of `data`.
            unsafe {
                for row in 0..height as usize {
                    let src = rgba_data.as_ptr().add(row * src_stride);
                    let dst = data.add(row * dst_stride);
                    ptr::copy_nonoverlapping(src, dst, src_stride);
                }
                psp::cache::dcache_writeback_range(data as *const core::ffi::c_void, total as u32);
            }
        }

        Some(tex_id)
    }

    /// Set the actual video content dimensions (may differ from texture
    /// stride). This controls UV mapping in blit so only the content
    /// region is sampled, not the stride padding. Clamped to the texture's
    /// width and rows so the GE never samples past the allocation (see the
    /// layout notes above `alloc_video_texture`).
    pub fn set_video_content_size(&mut self, w: u32, h: u32) {
        let (max_w, max_h) = self
            .video_tex
            .and_then(|id| self.textures.get(id.0 as usize))
            .and_then(|slot| slot.as_ref())
            .map_or((0, 0), |tex| (tex.width, tex.height));
        self.video_content_w = w.min(max_w);
        self.video_content_h = h.min(max_h);
    }

    /// Stop using the video texture (call when playback stops): the video
    /// thread can no longer write into it. Heap buffers are freed (once the
    /// decoder is done with them); a volatile-memory buffer is kept for the
    /// next tune, since the bump allocator cannot take it back anyway.
    pub fn free_video_texture(&mut self) {
        self.video_content_w = 0;
        self.video_content_h = 0;
        self.reap_retired_video_textures();

        let volatile_data = self
            .video_tex
            .and_then(|id| self.textures.get(id.0 as usize))
            .and_then(|slot| slot.as_ref())
            .filter(|tex| tex.in_volatile)
            .map(|tex| tex.data);
        if let Some(data) = volatile_data {
            // Kept, not freed: whether or not a stuck decode still holds
            // it, the next `alloc_video_texture` re-checks before reuse.
            let _ = crate::video::retire_video_texture_buffer(data);
            return;
        }
        self.release_video_texture();
    }

    /// Drop the current video texture: unpublish it, then free its storage
    /// once the video thread is done with it (or park it in
    /// `video_tex_retired` if a decode still holds it after the bounded
    /// wait).
    fn release_video_texture(&mut self) {
        self.video_tex_w = 0;
        self.video_tex_h = 0;
        let Some(id) = self.video_tex.take() else {
            crate::video::unpublish_video_texture_buffer();
            return;
        };
        let Some(texture) = self.textures.get_mut(id.0 as usize).and_then(Option::take) else {
            crate::video::unpublish_video_texture_buffer();
            return;
        };
        if crate::video::retire_video_texture_buffer(texture.data) {
            free_texture_storage(texture);
        } else {
            crate::video::vlog_force(
                "[VIDEO] texture still in use by the decoder; deferring its free",
            );
            self.video_tex_retired.push(texture);
        }
    }

    /// Free retired video buffers the video thread has let go of.
    fn reap_retired_video_textures(&mut self) {
        let mut i = 0;
        while i < self.video_tex_retired.len() {
            if crate::video::video_texture_buffer_in_use(self.video_tex_retired[i].data) {
                i += 1;
            } else {
                free_texture_storage(self.video_tex_retired.swap_remove(i));
            }
        }
    }

    /// Allocate `size` bytes of 16-byte aligned texture storage, preferring
    /// volatile memory and falling back to the heap.
    fn alloc_texture_storage(&mut self, size: usize) -> Option<(*mut u8, Layout, bool)> {
        if let Some(ref mut va) = self.volatile_alloc {
            let p = va.alloc(size);
            if !p.is_null() {
                return Some((p, Layout::new::<u8>(), true));
            }
        }
        let layout = Layout::from_size_align(size, 16).ok()?;
        // SAFETY: Layout is valid and non-zero-sized. Null check follows.
        let p = unsafe { alloc(layout) };
        if p.is_null() {
            return None;
        }
        Some((p, layout, false))
    }
}

/// Zeroed rows below the video texture's CSC output, so bilinear sampling
/// at the bottom content edge stays inside the allocation (2 KB per row).
const VIDEO_TEX_GUARD_ROWS: u32 = 1;

/// Byte capacity of a video texture allocation (rows + guard rows).
fn video_tex_capacity(tex: &Texture) -> usize {
    (tex.buf_w * (tex.height + VIDEO_TEX_GUARD_ROWS) * 4) as usize
}

/// Release a texture's storage (heap only; volatile memory is reclaimed
/// solely by resetting the bump allocator).
fn free_texture_storage(texture: Texture) {
    if !texture.in_volatile {
        // SAFETY: heap textures are allocated with `alloc(texture.layout)`
        // (load_texture_inner / alloc_texture_storage) and freed only here,
        // after being removed from the registry.
        unsafe { dealloc(texture.data, texture.layout) };
    }
}

/// Zero `len` bytes at `data`. Manual loop to avoid
/// `core::ptr::write_bytes` on PSP.
///
/// # Safety
///
/// `data` must be valid for writes of `len` bytes and not concurrently
/// accessed.
unsafe fn zero_bytes(data: *mut u8, len: usize) {
    // SAFETY: guaranteed by the caller.
    let slice = unsafe { std::slice::from_raw_parts_mut(data, len) };
    for byte in slice.iter_mut() {
        *byte = 0;
    }
}
