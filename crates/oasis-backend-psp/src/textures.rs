//! Texture storage and volatile memory management.

use std::alloc::{alloc, dealloc, Layout};
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
pub(crate) struct VolatileAllocator {
    base: *mut u8,
    pub(crate) size: usize,
    offset: usize,
}

impl VolatileAllocator {
    /// Create a new allocator over the given memory region.
    pub(crate) fn new(base: *mut u8, size: usize) -> Self {
        Self { base, size, offset: 0 }
    }

    /// Allocate `len` bytes with 16-byte alignment. Returns null on OOM.
    pub(crate) fn alloc(&mut self, len: usize) -> *mut u8 {
        let aligned = (self.offset + 15) & !15;
        if aligned + len > self.size {
            return ptr::null_mut();
        }
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
        let (data, layout, in_volatile) =
            if let Some(ref mut va) = self.volatile_alloc {
                let p = va.alloc(buf_size);
                if !p.is_null() {
                    (p, Layout::new::<u8>(), true)
                } else {
                    let layout = Layout::from_size_align(buf_size, 16).ok()?;
                    let p = unsafe { alloc(layout) };
                    if p.is_null() {
                        return None;
                    }
                    (p, layout, false)
                }
            } else {
                let layout = Layout::from_size_align(buf_size, 16).ok()?;
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
            unsafe {
                let src = rgba_data.as_ptr().add(row * src_stride);
                let dst = data.add(row * dst_stride);
                if use_dma {
                    // SAFETY: src and dst are valid, non-overlapping, and
                    // src_stride > 0. Cache coherency handled above.
                    if psp::dma::memcpy_dma(dst, src, src_stride as u32).is_ok() {
                        continue;
                    }
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

    /// Destroy a loaded texture, freeing its memory.
    ///
    /// Textures in volatile memory are not individually freed (the bump
    /// allocator reclaims them all at once on reset).
    pub fn destroy_texture_inner(&mut self, tex: TextureId) {
        let idx = tex.0 as usize;
        if idx < self.textures.len() {
            if let Some(texture) = self.textures[idx].take() {
                if !texture.in_volatile {
                    // SAFETY: texture.data was allocated with texture.layout
                    // via alloc() in load_texture_inner. Not in volatile mem.
                    unsafe {
                        dealloc(texture.data, texture.layout);
                    }
                }
            }
        }
    }
}
