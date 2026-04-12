//! Per-frame render-target pool.
//!
//! Compositing layers allocate an offscreen `RenderTargetId` on each
//! `PushCompositingLayer` and release it on `PopCompositingLayer`.
//! Without recycling, every `mix-blend-mode` element would thrash
//! `create_render_target` / `destroy_render_target` once per frame.
//!
//! [`RenderTargetPool`] keys targets on `(width, height)` and hands out
//! recycled ids within a frame. At the end of each frame, callers call
//! [`RenderTargetPool::end_frame`]; targets that have not been requested
//! for `retain_frames` consecutive frames are destroyed on the backend.
//!
//! This helper lives on the browser side — it is not part of the
//! `SdiRenderTarget` trait. Backends stay dumb: they just allocate,
//! bind, composite, and destroy.

use oasis_types::backend::{RenderTargetId, SdiRenderTarget};
use oasis_types::error::{OasisError, Result};
use std::collections::HashMap;

/// Default number of frames an unused render target stays resident
/// before the pool releases it back to the backend.
pub const DEFAULT_RETAIN_FRAMES: u32 = 4;

/// An entry in the pool: the target id plus the index of the last
/// frame it was handed out in.
#[derive(Debug, Clone, Copy)]
struct PoolEntry {
    id: RenderTargetId,
    last_used_frame: u64,
}

/// A per-frame recycler for offscreen render targets.
///
/// Targets are grouped by their `(width, height)` so a
/// `mix-blend-mode` card that redraws every frame reuses the same
/// underlying surface instead of reallocating. Targets that go
/// unused for more than `retain_frames` consecutive frames are
/// destroyed at [`end_frame`](Self::end_frame) time.
#[derive(Debug)]
pub struct RenderTargetPool {
    /// Free entries available for reuse, grouped by `(w, h)`.
    free: HashMap<(u32, u32), Vec<PoolEntry>>,
    /// Entries handed out during the current frame. Returned to
    /// `free` in [`end_frame`](Self::end_frame).
    in_use: Vec<(RenderTargetId, (u32, u32))>,
    /// Monotonically increasing frame counter used for eviction.
    frame_index: u64,
    /// Maximum number of frames an unused target stays resident
    /// before it is destroyed.
    retain_frames: u32,
}

impl RenderTargetPool {
    /// Create a new pool with the default retention window
    /// ([`DEFAULT_RETAIN_FRAMES`]).
    pub fn new() -> Self {
        Self::with_retain_frames(DEFAULT_RETAIN_FRAMES)
    }

    /// Create a new pool with a custom retention window.
    pub fn with_retain_frames(retain_frames: u32) -> Self {
        Self {
            free: HashMap::new(),
            in_use: Vec::new(),
            frame_index: 0,
            retain_frames,
        }
    }

    /// Acquire a render target of the given size, creating a new one
    /// via the backend if no recycled target is available.
    ///
    /// The returned id stays reserved until [`end_frame`](Self::end_frame)
    /// is called, after which it becomes available for reuse again.
    pub fn acquire<B: SdiRenderTarget + ?Sized>(
        &mut self,
        backend: &mut B,
        width: u32,
        height: u32,
    ) -> Result<RenderTargetId> {
        if width == 0 || height == 0 {
            return Err(OasisError::Backend(
                "zero-dimension render target".into(),
            ));
        }
        let key = (width, height);
        let id = if let Some(bucket) = self.free.get_mut(&key)
            && let Some(entry) = bucket.pop()
        {
            entry.id
        } else {
            backend.create_render_target(width, height)?
        };
        self.in_use.push((id, key));
        Ok(id)
    }

    /// End the current frame: move all in-use entries back to the
    /// free list stamped with the current frame index, then destroy
    /// any free entries that have gone unused for more than
    /// `retain_frames` frames.
    pub fn end_frame<B: SdiRenderTarget + ?Sized>(&mut self, backend: &mut B) -> Result<()> {
        // Return in-use targets to the free list.
        let current = self.frame_index;
        for (id, key) in self.in_use.drain(..) {
            self.free.entry(key).or_default().push(PoolEntry {
                id,
                last_used_frame: current,
            });
        }

        // Evict entries whose age (current frame - last used frame)
        // has reached `retain_frames`. An entry used this frame has
        // age 0 and stays; once its age hits `retain_frames` it is
        // destroyed. So `retain_frames == 2` means "kept for 2 idle
        // end_frame calls, destroyed on the 2nd".
        let retain = self.retain_frames as u64;
        let mut to_destroy: Vec<RenderTargetId> = Vec::new();
        for bucket in self.free.values() {
            for entry in bucket {
                let age = current.saturating_sub(entry.last_used_frame);
                if age >= retain {
                    to_destroy.push(entry.id);
                }
            }
        }
        let mut destroyed: std::collections::HashSet<RenderTargetId> =
            std::collections::HashSet::new();
        let mut first_err: Option<OasisError> = None;
        for id in &to_destroy {
            match backend.destroy_render_target(*id) {
                Ok(()) => {
                    destroyed.insert(*id);
                }
                Err(e) if first_err.is_none() => {
                    first_err = Some(e);
                }
                Err(_) => {}
            }
        }
        self.free.retain(|_, bucket| {
            bucket.retain(|entry| !destroyed.contains(&entry.id));
            !bucket.is_empty()
        });
        if let Some(e) = first_err {
            return Err(e);
        }

        self.frame_index += 1;
        Ok(())
    }

    /// Destroy every target the pool is holding. Call at shutdown or
    /// when the pool is being torn down (e.g. browser navigation).
    pub fn clear<B: SdiRenderTarget + ?Sized>(&mut self, backend: &mut B) -> Result<()> {
        let free_ids: Vec<RenderTargetId> = self
            .free
            .values()
            .flat_map(|bucket| bucket.iter().map(|e| e.id))
            .collect();
        let in_use_ids: Vec<RenderTargetId> =
            self.in_use.iter().map(|(id, _)| *id).collect();
        let mut first_err: Option<OasisError> = None;
        for id in free_ids.iter().chain(in_use_ids.iter()) {
            match backend.destroy_render_target(*id) {
                Ok(()) => {}
                Err(e) if first_err.is_none() => {
                    first_err = Some(e);
                }
                Err(_) => {}
            }
        }
        self.free.clear();
        self.in_use.clear();
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Total number of render targets currently resident in the pool
    /// (in-use plus free). Exposed for tests and telemetry.
    pub fn resident_count(&self) -> usize {
        self.in_use.len() + self.free.values().map(|v| v.len()).sum::<usize>()
    }

    /// Number of targets currently handed out (not available for
    /// reuse this frame).
    pub fn in_use_count(&self) -> usize {
        self.in_use.len()
    }
}

impl Default for RenderTargetPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_test_backend::RecordingBackend;

    #[test]
    fn acquire_creates_when_empty() {
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::new();

        let id = pool.acquire(&mut backend, 64, 48).unwrap();
        assert_eq!(backend.live_render_target_count(), 1);
        assert_eq!(pool.in_use_count(), 1);
        assert_eq!(id.0, 1);
    }

    #[test]
    fn end_frame_returns_to_free_list() {
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::new();

        let _ = pool.acquire(&mut backend, 64, 48).unwrap();
        pool.end_frame(&mut backend).unwrap();

        assert_eq!(pool.in_use_count(), 0);
        assert_eq!(pool.resident_count(), 1);
        assert_eq!(backend.live_render_target_count(), 1);
    }

    #[test]
    fn second_frame_recycles_same_id() {
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::new();

        let id1 = pool.acquire(&mut backend, 64, 48).unwrap();
        pool.end_frame(&mut backend).unwrap();

        let id2 = pool.acquire(&mut backend, 64, 48).unwrap();
        assert_eq!(id1, id2, "same (w,h) should recycle the same target");
        // No extra backend allocation.
        assert_eq!(backend.live_render_target_count(), 1);
    }

    #[test]
    fn different_dimensions_get_different_targets() {
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::new();

        let a = pool.acquire(&mut backend, 64, 48).unwrap();
        let b = pool.acquire(&mut backend, 32, 32).unwrap();
        assert_ne!(a, b);
        assert_eq!(backend.live_render_target_count(), 2);
    }

    #[test]
    fn unused_target_evicted_after_retain_window() {
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::with_retain_frames(2);

        // Frame 0: acquire, release.
        let _ = pool.acquire(&mut backend, 64, 48).unwrap();
        pool.end_frame(&mut backend).unwrap();
        assert_eq!(backend.live_render_target_count(), 1);

        // Frames 1, 2 without use: still resident (retain_frames=2).
        pool.end_frame(&mut backend).unwrap();
        assert_eq!(backend.live_render_target_count(), 1);
        pool.end_frame(&mut backend).unwrap();
        // Now evicted.
        assert_eq!(backend.live_render_target_count(), 0);
    }

    #[test]
    fn clear_destroys_everything() {
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::new();

        let _ = pool.acquire(&mut backend, 64, 48).unwrap();
        let _ = pool.acquire(&mut backend, 32, 32).unwrap();
        // One goes to free list, one stays in use.
        pool.end_frame(&mut backend).unwrap();
        let _ = pool.acquire(&mut backend, 32, 32).unwrap();
        assert_eq!(backend.live_render_target_count(), 2);

        pool.clear(&mut backend).unwrap();
        assert_eq!(backend.live_render_target_count(), 0);
        assert_eq!(pool.resident_count(), 0);
    }

    #[test]
    fn no_thrash_with_recurring_same_size() {
        // Simulate a page with a single mix-blend-mode card that
        // repaints every frame: only one backend allocation total.
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::new();

        for _ in 0..20 {
            let _ = pool.acquire(&mut backend, 100, 100).unwrap();
            pool.end_frame(&mut backend).unwrap();
        }
        assert_eq!(backend.live_render_target_count(), 1);
    }

    #[test]
    fn zero_dimension_rejected() {
        let mut backend = RecordingBackend::new(480, 272);
        let mut pool = RenderTargetPool::new();
        assert!(pool.acquire(&mut backend, 0, 100).is_err());
        assert!(pool.acquire(&mut backend, 100, 0).is_err());
        assert!(pool.acquire(&mut backend, 0, 0).is_err());
        assert_eq!(backend.live_render_target_count(), 0);
    }
}
