//! SDI object registry.
//!
//! The registry is a flat collection of named `SdiObject`s. It provides
//! create, lookup, z-order management, and a `draw` method that iterates
//! objects in z-order and dispatches to the rendering backend.

use std::collections::HashMap;

use serde::Deserialize;

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::{OasisError, Result};
use oasis_types::shadow::Shadow;

use crate::object::SdiObject;

/// The SDI scene graph: a flat, named registry of blittable objects.
#[derive(Debug)]
pub struct SdiRegistry {
    objects: HashMap<String, SdiObject>,
    /// Monotonically increasing counter for assigning z-order to new objects.
    next_z: i32,
    /// Pre-sorted names of non-overlay objects in z-order (ascending).
    /// Rebuilt on mutation rather than on every `draw()` call.
    z_sorted_base: Vec<String>,
    /// Pre-sorted names of overlay objects in z-order (ascending).
    z_sorted_overlay: Vec<String>,
    /// Whether the z-sorted lists need rebuilding before next draw.
    z_dirty: bool,
}

impl SdiRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            next_z: 0,
            z_sorted_base: Vec::new(),
            z_sorted_overlay: Vec::new(),
            z_dirty: false,
        }
    }

    /// Create a new object and insert it into the registry.
    /// Returns a mutable reference to the newly created object for chaining.
    ///
    /// If an object with the same name already exists, it is replaced.
    pub fn create(&mut self, name: impl Into<String>) -> &mut SdiObject {
        let name = name.into();
        if self.next_z == i32::MAX {
            self.renormalize_z_orders();
        }
        let mut obj = SdiObject::new(&name);
        obj.z = self.next_z;
        self.next_z += 1;
        self.objects.insert(name.clone(), obj);
        self.z_dirty = true;
        // SAFETY (logical): We just inserted with this key on the line above,
        // so the entry is guaranteed to exist.
        self.objects
            .get_mut(&name)
            .expect("just-inserted key missing")
    }

    /// Get a shared reference to an object by name.
    pub fn get(&self, name: &str) -> Result<&SdiObject> {
        self.objects
            .get(name)
            .ok_or_else(|| OasisError::Sdi(format!("object not found: {name}").into()))
    }

    /// Get a mutable reference to an object by name.
    pub fn get_mut(&mut self, name: &str) -> Result<&mut SdiObject> {
        self.objects
            .get_mut(name)
            .ok_or_else(|| OasisError::Sdi(format!("object not found: {name}").into()))
    }

    /// Remove an object from the registry.
    pub fn destroy(&mut self, name: &str) -> Result<()> {
        self.objects
            .remove(name)
            .map(|_| {
                self.z_dirty = true;
            })
            .ok_or_else(|| OasisError::Sdi(format!("object not found: {name}").into()))
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
        let min_z = self.objects.values().map(|o| o.z).min().unwrap_or(0) - 1;
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
        self.objects.contains_key(name)
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

    /// Apply a theme from a TOML string. Each top-level key is an object name;
    /// nested keys set properties (x, y, w, h, visible, alpha, color, text,
    /// font_size). Objects that don't exist yet are created.
    pub fn load_theme(&mut self, toml_str: &str) -> Result<()> {
        /// Apply optional fields from a theme entry to an SDI object.
        macro_rules! apply_theme {
            // Direct copy: entry.field -> obj.field
            ($entry:expr, $obj:expr, copy: [$($f:ident),*]) => {
                $(if let Some(v) = $entry.$f { $obj.$f = v; })*
            };
            // Clone into Option: entry.field -> obj.field = Some(clone)
            ($entry:expr, $obj:expr, clone: [$($f:ident),*]) => {
                $(if let Some(ref v) = $entry.$f { $obj.$f = Some(v.clone()); })*
            };
            // Parse hex color: entry.field -> obj.field = parsed
            ($entry:expr, $obj:expr, color: [$($f:ident),*]) => {
                $(if let Some(ref c) = $entry.$f
                    && let Some(parsed) = parse_color(c)
                { $obj.$f = parsed; })*
            };
            // Parse hex color into Option: entry.field -> obj.field = Some(parsed)
            ($entry:expr, $obj:expr, opt_color: [$($f:ident),*]) => {
                $(if let Some(ref c) = $entry.$f
                    && let Some(parsed) = parse_color(c)
                { $obj.$f = Some(parsed); })*
            };
            // Wrap in Some: entry.field -> obj.field = Some(v)
            ($entry:expr, $obj:expr, wrap: [$($f:ident),*]) => {
                $(if let Some(v) = $entry.$f { $obj.$f = Some(v); })*
            };
        }

        let theme: HashMap<String, ThemeEntry> =
            toml::from_str(toml_str).map_err(|e| OasisError::Config(format!("{e}").into()))?;

        for (name, entry) in theme {
            if !self.contains(&name) {
                self.create(&name);
            }
            let obj = self.get_mut(&name)?;
            apply_theme!(entry, obj, copy: [x, y, w, h, alpha, visible, font_size, overlay]);
            apply_theme!(entry, obj, clone: [text]);
            apply_theme!(entry, obj, color: [color, text_color]);
            apply_theme!(entry, obj, opt_color: [gradient_top, gradient_bottom, stroke_color, shadow_color]);
            apply_theme!(entry, obj, wrap: [border_radius, shadow_level, stroke_width]);
        }
        Ok(())
    }

    /// Return an iterator over all object names in the registry.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.objects.keys().map(String::as_str)
    }

    /// Renormalize all z-orders by sorting objects by their current z-value
    /// and reassigning sequential values starting from 0. Called when `next_z`
    /// reaches `i32::MAX` to prevent overflow.
    fn renormalize_z_orders(&mut self) {
        let mut sorted: Vec<String> = self.objects.keys().cloned().collect();
        // Tiebreak equal z by name: HashMap iteration order is random per
        // process, so a bare z sort makes draw order (and thus which of two
        // overlapping same-z objects wins) nondeterministic.
        sorted.sort_unstable_by(|a, b| (self.objects[a].z, a).cmp(&(self.objects[b].z, b)));
        for (i, name) in sorted.iter().enumerate() {
            // Cast is safe: object count is bounded by memory well before i32::MAX.
            self.objects.get_mut(name).expect("key from own iterator").z = i as i32;
        }
        self.next_z = sorted.len() as i32;
        self.z_dirty = true;
    }

    /// Rebuild the cached z-order index if it is dirty.
    ///
    /// Partitions objects into base and overlay lists and sorts each by
    /// z-value using `sort_unstable_by_key` (faster than stable sort).
    /// Reuses allocated `Vec` capacity via `clear()` + `push`.
    fn ensure_z_sorted(&mut self) {
        if !self.z_dirty {
            return;
        }
        self.z_sorted_base.clear();
        self.z_sorted_overlay.clear();
        for (name, obj) in &self.objects {
            if obj.overlay {
                self.z_sorted_overlay.push(name.clone());
            } else {
                self.z_sorted_base.push(name.clone());
            }
        }
        // Tiebreak equal z by name so draw order is deterministic (HashMap
        // iteration order is random per process; without this, overlapping
        // same-z objects — e.g. free-layout icons over content_bg — flicker
        // between runs).
        self.z_sorted_base.sort_unstable_by(|a, b| {
            (self.objects[a].z, a.as_str()).cmp(&(self.objects[b].z, b.as_str()))
        });
        self.z_sorted_overlay.sort_unstable_by(|a, b| {
            (self.objects[a].z, a.as_str()).cmp(&(self.objects[b].z, b.as_str()))
        });
        self.z_dirty = false;
    }

    /// Draw all visible objects to the backend, sorted by z-order (ascending).
    /// Uses PSIX-style two-pass rendering: base-layer objects first, then
    /// overlay objects on top. Each pass iterates only its pre-partitioned
    /// list, avoiding per-object overlay checks.
    pub fn draw(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        self.draw_base_layer(backend)?;
        self.draw_overlay_layer(backend)
    }

    /// Draw only the base layer (non-overlay objects), sorted by z-order.
    ///
    /// Use this with [`Self::draw_overlay_layer`] when you need to insert custom
    /// rendering (e.g., vector scenes) between the base and overlay passes.
    pub fn draw_base_layer(&mut self, backend: &mut dyn SdiBackend) -> Result<()> {
        self.ensure_z_sorted();

        for name in &self.z_sorted_base {
            let obj = &self.objects[name];
            if !obj.visible || obj.alpha == 0 {
                continue;
            }
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Draw only the overlay layer, sorted by z-order.
    ///
    /// Call after [`Self::draw_base_layer`] and any custom mid-layer rendering.
    pub fn draw_overlay_layer(&self, backend: &mut dyn SdiBackend) -> Result<()> {
        for name in &self.z_sorted_overlay {
            let obj = &self.objects[name];
            if !obj.visible || obj.alpha == 0 {
                continue;
            }
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Draw a single named object (if it exists and is visible).
    pub fn draw_named(&self, name: &str, backend: &mut dyn SdiBackend) -> Result<()> {
        if let Some(obj) = self.objects.get(name)
            && obj.visible
            && obj.alpha > 0
        {
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Draw all visible objects EXCEPT those whose names start with any of the
    /// given prefixes (e.g., window id prefixes). Useful for drawing
    /// non-window objects separately from window objects.
    pub fn draw_excluding_prefixes(
        &mut self,
        backend: &mut dyn SdiBackend,
        prefixes: &[&str],
    ) -> Result<()> {
        self.ensure_z_sorted();
        let keep = |name: &str| !prefixes.iter().any(|p| name.starts_with(p));
        self.draw_layer_filtered(&self.z_sorted_base, backend, &keep)?;
        self.draw_layer_filtered(&self.z_sorted_overlay, backend, &keep)
    }

    /// Draw only the base layer, excluding objects with given prefixes.
    pub fn draw_base_excluding_prefixes(
        &mut self,
        backend: &mut dyn SdiBackend,
        prefixes: &[&str],
    ) -> Result<()> {
        self.draw_base_filtered(backend, |name| {
            !prefixes.iter().any(|p| name.starts_with(p))
        })
    }

    /// Draw only the overlay layer, excluding objects with given prefixes.
    pub fn draw_overlay_excluding_prefixes(
        &self,
        backend: &mut dyn SdiBackend,
        prefixes: &[&str],
    ) -> Result<()> {
        self.draw_overlay_filtered(backend, |name| {
            !prefixes.iter().any(|p| name.starts_with(p))
        })
    }

    /// Draw only the base layer, keeping objects for which `keep` returns true.
    pub fn draw_base_filtered<F>(&mut self, backend: &mut dyn SdiBackend, keep: F) -> Result<()>
    where
        F: Fn(&str) -> bool,
    {
        self.ensure_z_sorted();
        self.draw_layer_filtered(&self.z_sorted_base, backend, &keep)
    }

    /// Draw only the overlay layer, keeping objects for which `keep` returns true.
    pub fn draw_overlay_filtered<F>(&self, backend: &mut dyn SdiBackend, keep: F) -> Result<()>
    where
        F: Fn(&str) -> bool,
    {
        self.draw_layer_filtered(&self.z_sorted_overlay, backend, &keep)
    }

    /// Internal: draw objects from a z-sorted list, applying a filter.
    fn draw_layer_filtered(
        &self,
        layer: &[String],
        backend: &mut dyn SdiBackend,
        keep: &dyn Fn(&str) -> bool,
    ) -> Result<()> {
        for name in layer {
            let obj = &self.objects[name];
            if !obj.visible || obj.alpha == 0 {
                continue;
            }
            if !keep(name) {
                continue;
            }
            Self::draw_object(obj, backend)?;
        }
        Ok(())
    }

    /// Render a single SDI object to the backend.
    ///
    /// Dispatch order for non-textured objects with nonzero area:
    /// 1. Shadow (if `shadow_level > 0`)
    /// 2. Fill: gradient+radius → gradient → rounded → flat (existing)
    /// 3. Stroke (if `stroke_width` set)
    /// 4. Text (if present)
    fn draw_object(obj: &SdiObject, backend: &mut dyn SdiBackend) -> Result<()> {
        // Textured object -- blit the texture. A non-opaque `alpha` goes
        // through the tinted path so textures can fade/pulse (backends
        // without tint support fall back to a plain blit).
        if let Some(tex) = obj.texture {
            // Nine-patch object: 9-slice blit (corners fixed, edges/center
            // stretched). Alpha fade is not supported on this path.
            if let Some(slices) = obj.nine_patch {
                let patch = oasis_types::nine_patch::NinePatch::from_slices(tex, slices);
                patch.draw(backend, obj.x, obj.y, obj.w, obj.h)?;
                return Ok(());
            }
            if obj.alpha < 255 {
                let tint = Color::rgba(255, 255, 255, obj.alpha);
                backend.blit_tinted(tex, obj.x, obj.y, obj.w, obj.h, tint)?;
            } else {
                backend.blit(tex, obj.x, obj.y, obj.w, obj.h)?;
            }
            return Ok(());
        }

        let has_area = obj.w > 0 && obj.h > 0;
        let radius = obj.border_radius.unwrap_or(0);

        if has_area {
            // 1. Shadow behind the rect.
            if let Some(level) = obj.shadow_level
                && level > 0
            {
                let mut shadow = Shadow::elevation(level);
                if let Some(sc) = obj.shadow_color {
                    shadow = shadow.with_color(sc);
                }
                shadow.draw(backend, obj.x, obj.y, obj.w, obj.h, radius)?;
            }

            // 2. Fill: choose the best primitive.
            let a = ((obj.color.a as u16) * (obj.alpha as u16) / 255) as u8;
            let color = Color::rgba(obj.color.r, obj.color.g, obj.color.b, a);

            if let (Some(gt), Some(gb)) = (obj.gradient_top, obj.gradient_bottom) {
                let top = Color::rgba(
                    gt.r,
                    gt.g,
                    gt.b,
                    ((gt.a as u16) * (obj.alpha as u16) / 255) as u8,
                );
                let bot = Color::rgba(
                    gb.r,
                    gb.g,
                    gb.b,
                    ((gb.a as u16) * (obj.alpha as u16) / 255) as u8,
                );
                let gradient = oasis_types::backend::GradientStyle::Vertical { top, bottom: bot };
                if radius > 0 {
                    backend.fill_rounded_rect_gradient(
                        obj.x, obj.y, obj.w, obj.h, radius, &gradient,
                    )?;
                } else {
                    backend.fill_rect_gradient(obj.x, obj.y, obj.w, obj.h, &gradient)?;
                }
            } else if radius > 0 {
                backend.fill_rounded_rect(obj.x, obj.y, obj.w, obj.h, radius, color)?;
            } else {
                backend.fill_rect(obj.x, obj.y, obj.w, obj.h, color)?;
            }

            // 3. Stroke outline.
            if let Some(sw) = obj.stroke_width
                && sw > 0
            {
                let sc = obj.stroke_color.unwrap_or(Color::rgba(255, 255, 255, 128));
                if radius > 0 {
                    backend.stroke_rounded_rect(obj.x, obj.y, obj.w, obj.h, radius, sw, sc)?;
                } else {
                    backend.stroke_rect(obj.x, obj.y, obj.w, obj.h, sw, sc)?;
                }
            }
        }

        // 4. Text on top (with optional shadow pass).
        if let Some(ref text) = obj.text {
            if let Some((dx, dy)) = obj.text_shadow_offset {
                let shadow_color = obj.text_shadow_color.unwrap_or(Color::rgba(0, 0, 0, 128));
                backend.draw_text(text, obj.x + dx, obj.y + dy, obj.font_size, shadow_color)?;
            }
            backend.draw_text(text, obj.x, obj.y, obj.font_size, obj.text_color)?;
        }

        Ok(())
    }
}

/// Deserialization helper for theme TOML entries.
#[derive(Debug, Deserialize)]
struct ThemeEntry {
    /// X position in virtual screen coordinates.
    x: Option<i32>,
    /// Y position in virtual screen coordinates.
    y: Option<i32>,
    /// Width in pixels.
    w: Option<u32>,
    /// Height in pixels.
    h: Option<u32>,
    /// Alpha (0 = fully transparent, 255 = fully opaque).
    alpha: Option<u8>,
    /// Whether this object is drawn.
    visible: Option<bool>,
    /// Text content.
    text: Option<String>,
    /// Font size in pixels.
    font_size: Option<u16>,
    /// Fill color as hex string ("#RRGGBB" or "#RRGGBBAA").
    color: Option<String>,
    /// Text color as hex string.
    text_color: Option<String>,
    /// Render in overlay pass (on top of base layer).
    overlay: Option<bool>,
    // Extended visual properties.
    /// Corner radius for rounded rectangles (pixels).
    border_radius: Option<u16>,
    /// Gradient top color as hex string.
    gradient_top: Option<String>,
    /// Gradient bottom color as hex string.
    gradient_bottom: Option<String>,
    /// Shadow elevation level (0 = none, 1-3 = increasingly prominent).
    shadow_level: Option<u8>,
    /// Stroke/outline width in pixels.
    stroke_width: Option<u16>,
    /// Stroke/outline color as hex string.
    stroke_color: Option<String>,
    /// Shadow color as hex string (default: black).
    shadow_color: Option<String>,
}

/// Parse a color string like "#RRGGBB" or "#RRGGBBAA".
fn parse_color(s: &str) -> Option<Color> {
    oasis_types::color::parse_hex_color(s)
}

impl Default for SdiRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::{SdiCore, TextureId};

    #[test]
    fn create_and_get() {
        let mut reg = SdiRegistry::new();
        {
            let obj = reg.create("panel");
            obj.x = 10;
            obj.y = 20;
        }
        let obj = reg.get("panel").unwrap();
        assert_eq!(obj.x, 10);
        assert_eq!(obj.y, 20);
    }

    #[test]
    fn get_nonexistent_returns_error() {
        let reg = SdiRegistry::new();
        assert!(reg.get("nope").is_err());
    }

    #[test]
    fn destroy_removes_object() {
        let mut reg = SdiRegistry::new();
        reg.create("temp");
        assert!(reg.contains("temp"));
        reg.destroy("temp").unwrap();
        assert!(!reg.contains("temp"));
    }

    #[test]
    fn equal_z_draw_order_is_deterministic_by_name() {
        // Free-layout dashboards force icon objects to the same z as the
        // content backdrop; the tie must resolve by name (not HashMap
        // iteration order) so "icon_*" always draws over "content_bg".
        let mut reg = SdiRegistry::new();
        // Insert in an order that differs from the alphabetical result.
        for name in ["icon_body_3", "content_bg", "icon_body_1", "icon_body_2"] {
            let obj = reg.create(name);
            obj.z = 0;
        }
        reg.z_dirty = true;
        reg.ensure_z_sorted();
        assert_eq!(
            reg.z_sorted_base,
            vec!["content_bg", "icon_body_1", "icon_body_2", "icon_body_3"]
        );
    }

    #[test]
    fn z_order_auto_increments() {
        let mut reg = SdiRegistry::new();
        reg.create("a");
        reg.create("b");
        reg.create("c");
        let a = reg.get("a").unwrap().z;
        let b = reg.get("b").unwrap().z;
        let c = reg.get("c").unwrap().z;
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn move_to_top() {
        let mut reg = SdiRegistry::new();
        reg.create("bottom");
        reg.create("top");
        let top_z = reg.get("top").unwrap().z;
        reg.move_to_top("bottom").unwrap();
        let bottom_z = reg.get("bottom").unwrap().z;
        assert!(bottom_z > top_z);
    }

    #[test]
    fn move_to_bottom() {
        let mut reg = SdiRegistry::new();
        reg.create("a");
        reg.create("b");
        let a_z = reg.get("a").unwrap().z;
        reg.move_to_bottom("b").unwrap();
        let b_z = reg.get("b").unwrap().z;
        assert!(b_z < a_z);
    }

    #[test]
    fn renormalize_z_orders_on_overflow() {
        let mut reg = SdiRegistry::new();
        // Create three objects with normal z-orders.
        reg.create("a"); // z=0
        reg.create("b"); // z=1
        reg.create("c"); // z=2

        // Force next_z to i32::MAX to trigger renormalization on next create.
        reg.next_z = i32::MAX;

        // Manually set z-orders to large values to simulate long-running usage.
        reg.get_mut("a").unwrap().z = i32::MAX - 100;
        reg.get_mut("b").unwrap().z = i32::MAX - 50;
        reg.get_mut("c").unwrap().z = i32::MAX - 10;

        // This create should trigger renormalization, then assign the new object.
        reg.create("d");

        // After renormalization, z-orders should be sequential starting from 0.
        // The relative order must be preserved: a < b < c < d.
        let za = reg.get("a").unwrap().z;
        let zb = reg.get("b").unwrap().z;
        let zc = reg.get("c").unwrap().z;
        let zd = reg.get("d").unwrap().z;
        assert!(za < zb, "a.z={za} should be < b.z={zb}");
        assert!(zb < zc, "b.z={zb} should be < c.z={zc}");
        assert!(zc < zd, "c.z={zc} should be < d.z={zd}");
        // The values should be small (renormalized from 0).
        assert!(
            zd < 10,
            "z-orders should be renormalized to small values, got d.z={zd}"
        );
    }

    #[test]
    fn renormalize_z_orders_on_move_to_top() {
        let mut reg = SdiRegistry::new();
        reg.create("x"); // z=0
        reg.create("y"); // z=1

        // Force next_z to i32::MAX.
        reg.next_z = i32::MAX;
        reg.get_mut("x").unwrap().z = i32::MAX - 20;
        reg.get_mut("y").unwrap().z = i32::MAX - 10;

        // move_to_top should trigger renormalization first, then assign new top z.
        reg.move_to_top("x").unwrap();

        let zx = reg.get("x").unwrap().z;
        let zy = reg.get("y").unwrap().z;
        assert!(zx > zy, "x.z={zx} should be > y.z={zy} after move_to_top");
        // Values should be small after renormalization.
        assert!(
            zx < 10,
            "z-orders should be renormalized to small values, got x.z={zx}"
        );
    }

    #[test]
    fn len_and_is_empty() {
        let mut reg = SdiRegistry::new();
        assert!(reg.is_empty());
        reg.create("x");
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
    }

    #[test]
    fn load_theme_creates_and_updates() {
        let mut reg = SdiRegistry::new();
        reg.create("existing");
        let theme = r##"
[existing]
x = 42
y = 10

[new_obj]
x = 100
w = 50
h = 30
color = "#FF0000"
text = "hello"
font_size = 16
"##;
        reg.load_theme(theme).unwrap();
        let e = reg.get("existing").unwrap();
        assert_eq!(e.x, 42);
        assert_eq!(e.y, 10);

        let n = reg.get("new_obj").unwrap();
        assert_eq!(n.x, 100);
        assert_eq!(n.w, 50);
        assert_eq!(n.h, 30);
        assert_eq!(n.color.r, 255);
        assert_eq!(n.color.g, 0);
        assert_eq!(n.text.as_deref(), Some("hello"));
        assert_eq!(n.font_size, 16);
    }

    #[test]
    fn parse_color_hex() {
        let c = super::parse_color("#1A2B3C").unwrap();
        assert_eq!(c.r, 0x1A);
        assert_eq!(c.g, 0x2B);
        assert_eq!(c.b, 0x3C);
        assert_eq!(c.a, 255);

        let c2 = super::parse_color("#1A2B3C80").unwrap();
        assert_eq!(c2.a, 0x80);
    }

    #[test]
    fn parse_color_invalid() {
        assert!(super::parse_color("not-a-color").is_none());
        assert!(super::parse_color("#GG0000").is_none());
        assert!(super::parse_color("#12345").is_none());
    }

    #[test]
    fn text_object_defaults() {
        let mut reg = SdiRegistry::new();
        let obj = reg.create("label");
        obj.text = Some("test".into());
        obj.font_size = 14;
        let o = reg.get("label").unwrap();
        assert_eq!(o.text.as_deref(), Some("test"));
        assert_eq!(o.font_size, 14);
    }

    #[test]
    fn names_iterator() {
        let mut reg = SdiRegistry::new();
        reg.create("a");
        reg.create("b");
        let mut names: Vec<&str> = reg.names().collect();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    // -- Draw pipeline tests using a recording backend --

    use std::cell::RefCell;
    use std::rc::Rc;

    /// Recording backend that tracks all draw calls for verification.
    struct RecordingBackend {
        calls: Rc<RefCell<Vec<DrawCall>>>,
    }

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    enum DrawCall {
        Clear(Color),
        FillRect {
            x: i32,
            y: i32,
            w: u32,
            h: u32,
        },
        DrawText {
            text: String,
            x: i32,
            y: i32,
            font_size: u16,
        },
        Blit {
            tex: TextureId,
        },
    }

    impl RecordingBackend {
        fn new() -> (Self, Rc<RefCell<Vec<DrawCall>>>) {
            let calls = Rc::new(RefCell::new(Vec::new()));
            (
                Self {
                    calls: Rc::clone(&calls),
                },
                calls,
            )
        }
    }

    impl SdiCore for RecordingBackend {
        fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn clear(&mut self, color: Color) -> Result<()> {
            self.calls.borrow_mut().push(DrawCall::Clear(color));
            Ok(())
        }
        fn blit(&mut self, tex: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            self.calls.borrow_mut().push(DrawCall::Blit { tex });
            Ok(())
        }
        fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, _color: Color) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(DrawCall::FillRect { x, y, w, h });
            Ok(())
        }
        fn draw_text(
            &mut self,
            text: &str,
            x: i32,
            y: i32,
            font_size: u16,
            _color: Color,
        ) -> Result<()> {
            self.calls.borrow_mut().push(DrawCall::DrawText {
                text: text.to_string(),
                x,
                y,
                font_size,
            });
            Ok(())
        }
        fn swap_buffers(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> Result<TextureId> {
            Ok(TextureId(0))
        }
        fn destroy_texture(&mut self, _tex: TextureId) -> Result<()> {
            Ok(())
        }
        fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> Result<()> {
            Ok(())
        }
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            text.len() as u32 * (font_size as u32 * 6 / 10).max(1)
        }
        fn read_pixels(&self, _x: i32, _y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
            Ok(vec![0u8; (w * h * 4) as usize])
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl oasis_types::backend::SdiShapes for RecordingBackend {}
    impl oasis_types::backend::SdiGradients for RecordingBackend {}
    impl oasis_types::backend::SdiAlpha for RecordingBackend {}
    impl oasis_types::backend::SdiText for RecordingBackend {}
    impl oasis_types::backend::SdiTextures for RecordingBackend {}
    impl oasis_types::backend::SdiClipTransform for RecordingBackend {}
    impl oasis_types::backend::SdiVector for RecordingBackend {}
    impl oasis_types::backend::SdiBatch for RecordingBackend {}
    impl oasis_types::backend::SdiRenderTarget for RecordingBackend {}

    #[test]
    fn draw_dispatches_fill_rect_for_colored_objects() {
        let mut reg = SdiRegistry::new();
        let obj = reg.create("box");
        obj.x = 10;
        obj.y = 20;
        obj.w = 100;
        obj.h = 50;
        obj.color = Color::rgb(255, 0, 0);

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        assert!(
            calls.iter().any(|c| matches!(
                c,
                DrawCall::FillRect {
                    x: 10,
                    y: 20,
                    w: 100,
                    h: 50
                }
            )),
            "expected fill_rect call for colored object"
        );
    }

    #[test]
    fn draw_dispatches_draw_text_for_text_objects() {
        let mut reg = SdiRegistry::new();
        let obj = reg.create("label");
        obj.text = Some("Hello World".to_string());
        obj.font_size = 12;
        obj.text_color = Color::WHITE;

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        let text_calls: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, DrawCall::DrawText { .. }))
            .collect();
        assert_eq!(text_calls.len(), 1, "expected exactly one draw_text call");
        if let DrawCall::DrawText { text, .. } = &text_calls[0] {
            assert_eq!(text, "Hello World");
        }
    }

    #[test]
    fn draw_skips_invisible_objects() {
        let mut reg = SdiRegistry::new();
        let obj = reg.create("hidden");
        obj.text = Some("invisible".to_string());
        obj.visible = false;

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        assert!(
            calls.is_empty(),
            "invisible object should not produce any draw calls"
        );
    }

    #[test]
    fn draw_skips_zero_alpha_objects() {
        let mut reg = SdiRegistry::new();
        let obj = reg.create("transparent");
        obj.text = Some("ghost".to_string());
        obj.alpha = 0;

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        assert!(
            calls.is_empty(),
            "zero-alpha object should not produce any draw calls"
        );
    }

    #[test]
    fn draw_text_only_object_no_fill_rect() {
        let mut reg = SdiRegistry::new();
        let obj = reg.create("text_only");
        obj.w = 0;
        obj.h = 0;
        obj.text = Some("just text".to_string());

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        // Should have draw_text but NOT fill_rect.
        assert!(
            calls.iter().any(|c| matches!(c, DrawCall::DrawText { .. })),
            "text-only object should call draw_text"
        );
        assert!(
            !calls.iter().any(|c| matches!(c, DrawCall::FillRect { .. })),
            "text-only object should NOT call fill_rect"
        );
    }

    #[test]
    fn draw_object_with_rect_and_text() {
        let mut reg = SdiRegistry::new();
        let obj = reg.create("button");
        obj.x = 5;
        obj.y = 10;
        obj.w = 80;
        obj.h = 20;
        obj.color = Color::rgb(50, 50, 50);
        obj.text = Some("Click".to_string());
        obj.text_color = Color::WHITE;
        obj.font_size = 12;

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        // Should have BOTH fill_rect and draw_text.
        assert!(
            calls.iter().any(|c| matches!(c, DrawCall::FillRect { .. })),
            "button should have fill_rect"
        );
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, DrawCall::DrawText { text, .. } if text == "Click")),
            "button should have draw_text with 'Click'"
        );
    }

    #[test]
    fn draw_z_order_respected() {
        let mut reg = SdiRegistry::new();
        // Create two objects. "back" at z=0, "front" at z=10.
        let obj = reg.create("back");
        obj.w = 10;
        obj.h = 10;
        obj.z = 0;
        obj.color = Color::rgb(255, 0, 0);
        let obj = reg.create("front");
        obj.w = 10;
        obj.h = 10;
        obj.z = 10;
        obj.color = Color::rgb(0, 255, 0);

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        // The first fill_rect should be for "back" (lower z), second for "front".
        let rects: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, DrawCall::FillRect { .. }))
            .collect();
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn draw_multiple_text_objects() {
        let mut reg = SdiRegistry::new();
        for i in 0..5 {
            let obj = reg.create(format!("line_{i}"));
            obj.text = Some(format!("Line {i}"));
            obj.y = i * 16;
        }

        let (mut backend, calls) = RecordingBackend::new();
        reg.draw(&mut backend).unwrap();

        let calls = calls.borrow();
        let text_calls: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, DrawCall::DrawText { .. }))
            .collect();
        assert_eq!(text_calls.len(), 5, "should render all 5 text objects");
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;
        use std::collections::HashSet;

        fn arb_names(min: usize, max: usize) -> impl Strategy<Value = Vec<String>> {
            proptest::collection::hash_set("[a-z]{1,8}", min..max)
                .prop_map(|s| s.into_iter().collect())
        }

        proptest! {
            #[test]
            fn len_equals_creates_minus_destroys(names in arb_names(1, 20)) {
                let mut reg = SdiRegistry::new();
                for name in &names {
                    reg.create(name);
                }
                prop_assert_eq!(reg.len(), names.len());

                // Destroy half.
                let destroy_count = names.len() / 2;
                for name in names.iter().take(destroy_count) {
                    reg.destroy(name).unwrap();
                }
                prop_assert_eq!(reg.len(), names.len() - destroy_count);
            }

            #[test]
            fn z_orders_are_unique(names in arb_names(2, 20)) {
                let mut reg = SdiRegistry::new();
                for name in &names {
                    reg.create(name);
                }
                let zs: Vec<i32> = names.iter().map(|n| reg.get(n).unwrap().z).collect();
                let unique: HashSet<i32> = zs.iter().copied().collect();
                prop_assert_eq!(
                    zs.len(), unique.len(),
                    "all z-orders must be unique"
                );
            }

            #[test]
            fn move_to_top_gives_highest_z(names in arb_names(2, 10)) {
                let mut reg = SdiRegistry::new();
                for name in &names {
                    reg.create(name);
                }
                let target = &names[0];
                reg.move_to_top(target).unwrap();
                let target_z = reg.get(target).unwrap().z;
                for name in &names[1..] {
                    let z = reg.get(name).unwrap().z;
                    prop_assert!(
                        target_z > z,
                        "move_to_top({target}): z={target_z} should be > {name}.z={z}"
                    );
                }
            }

            #[test]
            fn move_to_bottom_gives_lowest_z(names in arb_names(2, 10)) {
                let mut reg = SdiRegistry::new();
                for name in &names {
                    reg.create(name);
                }
                let target = &names[0];
                reg.move_to_bottom(target).unwrap();
                let target_z = reg.get(target).unwrap().z;
                for name in &names[1..] {
                    let z = reg.get(name).unwrap().z;
                    prop_assert!(
                        target_z < z,
                        "move_to_bottom({target}): z={target_z} should be < {name}.z={z}"
                    );
                }
            }

            #[test]
            fn create_then_contains(names in arb_names(1, 20)) {
                let mut reg = SdiRegistry::new();
                for name in &names {
                    reg.create(name);
                }
                for name in &names {
                    prop_assert!(reg.contains(name), "registry should contain {name}");
                }
            }

            #[test]
            fn destroy_then_not_contains(names in arb_names(1, 10)) {
                let mut reg = SdiRegistry::new();
                for name in &names {
                    reg.create(name);
                }
                for name in &names {
                    reg.destroy(name).unwrap();
                    prop_assert!(!reg.contains(name), "destroyed object should not be contained");
                }
                prop_assert!(reg.is_empty());
            }

            #[test]
            fn destroy_nonexistent_is_error(name in "[a-z]{1,8}") {
                let reg = SdiRegistry::new();
                prop_assert!(reg.get(&name).is_err());
            }

            #[test]
            fn z_order_auto_increments_monotonically(n in 2usize..20) {
                let mut reg = SdiRegistry::new();
                let mut prev_z = None;
                for i in 0..n {
                    let name = format!("obj_{i}");
                    reg.create(&name);
                    let z = reg.get(&name).unwrap().z;
                    if let Some(pz) = prev_z {
                        prop_assert!(z > pz, "z-order must increase: {z} > {pz}");
                    }
                    prev_z = Some(z);
                }
            }
        }
    }
}
