//! SDI object registry.
//!
//! The registry is a flat collection of named `SdiObject`s. It provides
//! create, lookup, z-order management, and a `draw` method that iterates
//! objects in z-order and dispatches to the rendering backend.
//!
//! # Names and z-order
//!
//! - **Names are unique keys.** [`SdiRegistry::create`] with a name that
//!   already exists *replaces* that object: the slot is reused but every
//!   property is reset to defaults and it gets a fresh (topmost) auto-z.
//!   Use [`SdiRegistry::contains`] + [`SdiRegistry::get_mut`] to update in
//!   place instead.
//! - **Auto z.** Each `create` (and `move_to_top`) takes the next value of
//!   a monotonically increasing counter, so later-created objects draw on
//!   top. Destroying an object never re-numbers the others.
//! - **Ties.** Objects with equal `z` (only possible when callers assign
//!   `z` explicitly) draw in ascending *name* order, independent of
//!   creation / destruction history, so the result is deterministic.
//! - **Explicit `z` / `overlay` edits** made through [`SdiRegistry::get_mut`]
//!   are picked up by the next draw (the cached sort is re-validated
//!   whenever an object was handed out mutably).

mod draw;
mod hash;
mod theme;

use std::hash::Hasher;

use oasis_types::backend::SdiBackend;
use oasis_types::error::{OasisError, Result, SdiError};

use crate::object::SdiObject;
use hash::{NameIndex, hash_object_into, object_hash};

/// The SDI scene graph: a flat, named registry of blittable objects.
///
/// Objects live in a dense `Vec` (the slab); `index` maps a name to its slot.
/// The z-sorted draw lists hold slot handles rather than names, so the draw
/// path never hashes a string — it indexes straight into the slab.
#[derive(Debug)]
pub struct SdiRegistry {
    /// Dense object storage. A slot's handle is stable until an object is
    /// destroyed (`destroy` swap-removes and repairs the moved entry's index).
    objects: Vec<SdiObject>,
    /// Object name -> slab handle (FNV-hashed; see [`FnvHasher`]).
    index: NameIndex,
    /// Monotonically increasing counter for assigning z-order to new objects.
    next_z: i32,
    /// Pre-sorted handles of non-overlay objects in z-order (ascending).
    /// Rebuilt on mutation rather than on every `draw()` call.
    z_sorted_base: Vec<usize>,
    /// Pre-sorted handles of overlay objects in z-order (ascending).
    z_sorted_overlay: Vec<usize>,
    /// Whether the z-sorted lists need rebuilding before next draw.
    z_dirty: bool,
    /// An object was handed out through [`Self::get_mut`] since the last
    /// sort, so its `z` / `overlay` may have changed: the cached lists are
    /// re-validated (one linear pass) before the next draw.
    z_check: bool,
    /// Structural change (create/destroy/z renormalize) since the last
    /// settle: the scene is definitely dirty and every per-object hash
    /// must be recomputed.
    structural_dirty: bool,
    /// Explicit invalidation that no object property reflects (e.g. a
    /// texture's pixels updated in place). Dirty without a rehash.
    forced_dirty: bool,
    /// Per-slot render-state hash as of the last settle
    /// ([`Self::take_scene_dirty`] / [`Self::clear_scene_dirty`]).
    settled_hash: Vec<u64>,
    /// Per-slot "handed out through `get_mut` since the last settle".
    touched_flag: Vec<bool>,
    /// Slots with `touched_flag` set, so a settle rehashes only those.
    touched: Vec<usize>,
}

impl SdiRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
            index: NameIndex::default(),
            next_z: 0,
            z_sorted_base: Vec::new(),
            z_sorted_overlay: Vec::new(),
            z_dirty: false,
            z_check: false,
            // Start dirty so a freshly built scene always draws once.
            structural_dirty: true,
            forced_dirty: false,
            settled_hash: Vec::new(),
            touched_flag: Vec::new(),
            touched: Vec::new(),
        }
    }

    /// Explicitly mark the scene as changed.
    ///
    /// Useful when something outside the registry invalidates rendered
    /// output (e.g. a texture's pixels were updated in place).
    pub fn mark_scene_dirty(&mut self) {
        self.forced_dirty = true;
    }

    /// Record that the object in `handle` may have been written.
    fn touch(&mut self, handle: usize) {
        // Slots past `touched_flag` were created since the last settle,
        // which set `structural_dirty` (full rehash) — nothing to track.
        if let Some(flag) = self.touched_flag.get_mut(handle)
            && !*flag
        {
            *flag = true;
            self.touched.push(handle);
        }
    }

    /// Whether the scene changed since the last settle
    /// ([`Self::take_scene_dirty`] / [`Self::clear_scene_dirty`]).
    ///
    /// Exact up to 64-bit hash collisions: objects handed out through
    /// [`Self::get_mut`] are re-hashed and compared against their state
    /// at the last settle, so per-frame update paths that rewrite
    /// identical values (or edit hidden objects) do not count as a
    /// change. Structural edits (create/destroy/z renormalization) and
    /// [`Self::mark_scene_dirty`] always count.
    pub fn is_scene_dirty(&self) -> bool {
        self.structural_dirty
            || self.forced_dirty
            || self
                .touched
                .iter()
                .any(|&h| object_hash(&self.objects[h]) != self.settled_hash[h])
    }

    /// Return whether the scene changed (see [`Self::is_scene_dirty`]) and
    /// settle it (call once per frame, after all scene mutation for the
    /// frame is done).
    pub fn take_scene_dirty(&mut self) -> bool {
        self.settle()
    }

    /// Settle the scene without reading the dirty state.
    pub fn clear_scene_dirty(&mut self) {
        self.settle();
    }

    /// Re-baseline per-object hashes; returns whether anything changed.
    ///
    /// Cost is proportional to the number of objects touched since the
    /// last settle (or the whole scene after a structural change).
    fn settle(&mut self) -> bool {
        let mut dirty = std::mem::take(&mut self.forced_dirty);
        if std::mem::take(&mut self.structural_dirty) {
            self.settled_hash.clear();
            self.settled_hash
                .extend(self.objects.iter().map(object_hash));
            self.touched_flag.clear();
            self.touched_flag.resize(self.objects.len(), false);
            self.touched.clear();
            return true;
        }
        for h in self.touched.drain(..) {
            self.touched_flag[h] = false;
            let new = object_hash(&self.objects[h]);
            if new != self.settled_hash[h] {
                self.settled_hash[h] = new;
                dirty = true;
            }
        }
        dirty
    }

    /// Hash of every render-relevant property of every object.
    ///
    /// Two calls return the same value iff the scene would rasterize
    /// identically (modulo the astronomically unlikely 64-bit hash
    /// collision). Hidden objects contribute only name + visibility.
    /// [`Self::is_scene_dirty`] is already exact for property rewrites;
    /// the signature additionally catches frames whose changes cancel
    /// out against an older frame (e.g. a popup shown and hidden again).
    pub fn scene_signature(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        // Slab order is deterministic between mutations; it only changes
        // through create/destroy, which legitimately change the hash
        // anyway (object count / names differ).
        for obj in &self.objects {
            hash_object_into(obj, &mut h);
        }
        h.finish()
    }

    /// Create a new object and insert it into the registry.
    /// Returns a mutable reference to the newly created object for chaining.
    ///
    /// If an object with the same name already exists, it is replaced (its
    /// slab slot is reused, but every property — including z — is reset).
    pub fn create(&mut self, name: impl Into<String>) -> &mut SdiObject {
        let name = name.into();
        if self.next_z == i32::MAX {
            self.renormalize_z_orders();
        }
        let mut obj = SdiObject::new(&name);
        obj.z = self.next_z;
        self.next_z += 1;
        self.z_dirty = true;
        self.structural_dirty = true;

        let handle = match self.index.get(&name) {
            Some(&h) => {
                self.objects[h] = obj;
                h
            },
            None => {
                let h = self.objects.len();
                self.objects.push(obj);
                self.index.insert(name, h);
                h
            },
        };
        &mut self.objects[handle]
    }

    /// Get a shared reference to an object by name.
    pub fn get(&self, name: &str) -> Result<&SdiObject> {
        self.index
            .get(name)
            .map(|&h| &self.objects[h])
            .ok_or_else(|| not_found(name))
    }

    /// Get a mutable reference to an object by name.
    ///
    /// Records the object as *touched*: the next dirty check re-hashes it
    /// and compares against its last settled state, so writing identical
    /// values costs one hash, never a redraw. For single-field updates
    /// prefer the change-detecting setters ([`Self::set_visible`],
    /// [`Self::set_text`], [`Self::set_position`]), which skip even that.
    pub fn get_mut(&mut self, name: &str) -> Result<&mut SdiObject> {
        match self.index.get(name) {
            Some(&h) => {
                self.touch(h);
                // The caller may rewrite `z` or `overlay` directly.
                self.z_check = true;
                Ok(&mut self.objects[h])
            },
            None => Err(not_found(name)),
        }
    }

    /// Set an object's visibility, touching it only on a real change.
    ///
    /// Returns `true` if the object exists and its visibility changed.
    /// Missing objects are ignored (`false`), matching the
    /// `if let Ok(obj) = sdi.get_mut(..)` idiom this replaces.
    pub fn set_visible(&mut self, name: &str, visible: bool) -> bool {
        let Some(&h) = self.index.get(name) else {
            return false;
        };
        if self.objects[h].visible == visible {
            return false;
        }
        self.touch(h);
        self.objects[h].visible = visible;
        true
    }

    /// Set an object's text, touching it (and allocating) only on a real
    /// change. Returns `true` if the object exists and its text changed.
    pub fn set_text(&mut self, name: &str, text: Option<&str>) -> bool {
        let Some(&h) = self.index.get(name) else {
            return false;
        };
        if self.objects[h].text.as_deref() == text {
            return false;
        }
        self.touch(h);
        let obj = &mut self.objects[h];
        match text {
            Some(t) => obj.set_text(t),
            None => obj.text = None,
        }
        true
    }

    /// Set an object's position, touching it only on a real change.
    /// Returns `true` if the object exists and moved.
    pub fn set_position(&mut self, name: &str, x: i32, y: i32) -> bool {
        let Some(&h) = self.index.get(name) else {
            return false;
        };
        let obj = &self.objects[h];
        if obj.x == x && obj.y == y {
            return false;
        }
        self.touch(h);
        let obj = &mut self.objects[h];
        obj.x = x;
        obj.y = y;
        true
    }

    /// Remove an object from the registry.
    pub fn destroy(&mut self, name: &str) -> Result<()> {
        let handle = self.index.remove(name).ok_or_else(|| not_found(name))?;
        self.objects.swap_remove(handle);
        // `swap_remove` moved the former last object into `handle` (unless the
        // removed object *was* last) — repoint its index entry.
        if let Some(moved) = self.objects.get(handle) {
            self.index.insert(moved.name.clone(), handle);
        }
        self.z_dirty = true;
        self.structural_dirty = true;
        Ok(())
    }

    /// Move an object to the top of the z-order (drawn last = on top).
    pub fn move_to_top(&mut self, name: &str) -> Result<()> {
        if self.next_z == i32::MAX {
            self.renormalize_z_orders();
        }
        let new_z = self.next_z;
        self.next_z += 1;
        let obj = self.get_mut(name)?;
        obj.z = new_z;
        self.z_dirty = true;
        Ok(())
    }

    /// Move an object to the bottom of the z-order (drawn first = behind).
    pub fn move_to_bottom(&mut self, name: &str) -> Result<()> {
        if !self.contains(name) {
            return Err(not_found(name));
        }
        let mut min_z = self.objects.iter().map(|o| o.z).min().unwrap_or(0);
        if min_z == i32::MIN {
            // No room below: compact the z range to 0.. first.
            self.renormalize_z_orders();
            min_z = 0;
        }
        let min_z = min_z - 1;
        let obj = self.get_mut(name)?;
        obj.z = min_z;
        self.z_dirty = true;
        Ok(())
    }

    /// Returns the number of objects in the registry.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Returns true if the registry contains no objects.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Returns true if an object with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// Load raw RGBA pixel data as a texture through the backend and assign it
    /// to the named object. The object's dimensions are updated to match.
    pub fn load_image(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        rgba_data: &[u8],
        backend: &mut dyn SdiBackend,
    ) -> Result<()> {
        let tex = backend.load_texture(width, height, rgba_data)?;
        let obj = self.get_mut(name)?;
        obj.texture = Some(tex);
        obj.w = width;
        obj.h = height;
        Ok(())
    }

    /// Return an iterator over all object names in the registry.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.objects.iter().map(|o| o.name.as_str())
    }

    /// Renormalize all z-orders by sorting objects by their current z-value
    /// and reassigning sequential values starting from 0. Called when `next_z`
    /// reaches `i32::MAX` to prevent overflow.
    fn renormalize_z_orders(&mut self) {
        let mut handles: Vec<usize> = (0..self.objects.len()).collect();
        // Tiebreak equal z by name: slab order depends on insertion/removal
        // history, so a bare z sort would leave draw order (and thus which of
        // two overlapping same-z objects wins) dependent on it.
        handles.sort_unstable_by(|&a, &b| self.z_key(a).cmp(&self.z_key(b)));
        for (i, &h) in handles.iter().enumerate() {
            // Cast is safe: object count is bounded by memory well before i32::MAX.
            self.objects[h].z = i as i32;
        }
        self.next_z = handles.len() as i32;
        self.z_dirty = true;
        self.structural_dirty = true;
    }

    /// Sort key for an object handle: z first, then name for a stable tiebreak.
    fn z_key(&self, handle: usize) -> (i32, &str) {
        let obj = &self.objects[handle];
        (obj.z, obj.name.as_str())
    }

    /// Rebuild the cached z-order index if it is dirty.
    ///
    /// Partitions object handles into base and overlay lists and sorts each by
    /// z-value. Reuses allocated `Vec` capacity via `clear()` + `push`.
    fn ensure_z_sorted(&mut self) {
        if !self.z_dirty && std::mem::take(&mut self.z_check) && !self.z_lists_valid() {
            self.z_dirty = true;
        }
        if !self.z_dirty {
            return;
        }
        // Move the lists out so the sort comparator can borrow `self.objects`.
        let mut base = std::mem::take(&mut self.z_sorted_base);
        let mut overlay = std::mem::take(&mut self.z_sorted_overlay);
        base.clear();
        overlay.clear();
        for (handle, obj) in self.objects.iter().enumerate() {
            if obj.overlay {
                overlay.push(handle);
            } else {
                base.push(handle);
            }
        }
        // Tiebreak equal z by name so draw order is deterministic: without it,
        // overlapping same-z objects — e.g. free-layout icons over content_bg —
        // would swap depending on slab order.
        base.sort_unstable_by(|&a, &b| self.z_key(a).cmp(&self.z_key(b)));
        overlay.sort_unstable_by(|&a, &b| self.z_key(a).cmp(&self.z_key(b)));
        self.z_sorted_base = base;
        self.z_sorted_overlay = overlay;
        self.z_dirty = false;
        self.z_check = false;
    }

    /// Whether the cached base / overlay lists still partition the objects
    /// by their `overlay` flag and are sorted by [`Self::z_key`]. Catches
    /// `z` / `overlay` rewritten through [`Self::get_mut`] after the sort.
    fn z_lists_valid(&self) -> bool {
        let layer_ok = |list: &[usize], overlay: bool| {
            list.iter().all(|&h| self.objects[h].overlay == overlay)
                && list
                    .windows(2)
                    .all(|w| self.z_key(w[0]) <= self.z_key(w[1]))
        };
        layer_ok(&self.z_sorted_base, false) && layer_ok(&self.z_sorted_overlay, true)
    }
}

/// The error for a missing object name.
fn not_found(name: &str) -> OasisError {
    OasisError::Sdi(SdiError::NotFound {
        name: name.to_owned(),
    })
}

impl Default for SdiRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
