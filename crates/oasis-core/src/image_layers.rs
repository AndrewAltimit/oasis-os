//! Image background layers (watermark decals).
//!
//! Skins declare `[[background_layers]] kind = "image"` entries that render
//! a bitmap between the wallpaper and the icon layer — the PSIX-style
//! watermark logo. Each layer becomes a z-ordered SDI object
//! (`bg_image_{i}`) whose texture is uploaded once at skin swap;
//! [`tick_image_layers`] animates position (drift) and opacity (pulse)
//! per frame by mutating the object, never re-uploading pixels.

use std::collections::HashMap;

use oasis_sdi::SdiRegistry;
use oasis_skin::{ImageLayerTheme, SkinAsset};
use oasis_types::backend::{SdiBackend, TextureId};

/// Z-order for image decals: above the wallpaper (-1000), below every
/// other scene object.
const IMAGE_LAYER_Z: i32 = -990;

/// Runtime state for one image layer: the SDI object it drives plus the
/// animation parameters and base placement needed to tick it.
pub struct ImageLayerObject {
    /// Name of the SDI object (`bg_image_{i}`).
    pub object_name: String,
    /// Backend texture (owned by this layer; freed by [`destroy_image_layers`]).
    pub texture: TextureId,
    /// Static top-left position before drift.
    pub base_x: i32,
    /// Static top-left position before drift.
    pub base_y: i32,
    /// Base opacity before pulse.
    pub base_alpha: u8,
    /// Drift / pulse parameters from the skin.
    pub animation: oasis_vector::background::LayerAnimation,
}

impl ImageLayerObject {
    /// Whether this layer needs per-frame updates.
    fn is_animated(&self) -> bool {
        let a = &self.animation;
        a.drift_x != 0.0 || a.drift_y != 0.0 || a.pulse_speed > 0.0
    }
}

/// Upload textures and create SDI objects for the active theme's image
/// layers. `scale` converts skin-native pixels to screen pixels (use
/// `min(sx, sy)` so decals keep their aspect ratio).
pub fn create_image_layers(
    sdi: &mut SdiRegistry,
    backend: &mut dyn SdiBackend,
    layers: &[ImageLayerTheme],
    assets: &HashMap<String, SkinAsset>,
    screen_w: u32,
    screen_h: u32,
    scale: f32,
) -> Vec<ImageLayerObject> {
    let mut out = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        if !layer.enabled {
            continue;
        }
        let Some(asset) = assets.get(&layer.source) else {
            log::warn!("image layer {i}: missing asset \"{}\"", layer.source);
            continue;
        };
        let tex = match backend.load_texture(asset.width, asset.height, &asset.rgba) {
            Ok(tex) => tex,
            Err(e) => {
                log::warn!("image layer {i}: texture upload failed: {e}");
                continue;
            },
        };

        let w = (asset.width as f32 * scale).round().max(1.0) as u32;
        let h = (asset.height as f32 * scale).round().max(1.0) as u32;

        // Anchor resolves to a point on the viewport; center the decal on
        // it, then pull edge/corner anchors back inside the screen so e.g.
        // BottomRight hugs the corner instead of hanging half off-screen.
        let (ax, ay) = layer.position.anchor.resolve(screen_w, screen_h);
        let mut base_x = ax - (w as i32) / 2 + (layer.position.offset_x * screen_w as f32) as i32;
        let mut base_y = ay - (h as i32) / 2 + (layer.position.offset_y * screen_h as f32) as i32;
        if ax == 0 {
            base_x = base_x.max(0);
        } else if ax == screen_w as i32 {
            base_x = base_x.min(screen_w as i32 - w as i32);
        }
        if ay == 0 {
            base_y = base_y.max(0);
        } else if ay == screen_h as i32 {
            base_y = base_y.min(screen_h as i32 - h as i32);
        }

        let name = format!("bg_image_{i}");
        let obj = sdi.create(&name);
        obj.x = base_x;
        obj.y = base_y;
        obj.w = w;
        obj.h = h;
        obj.z = IMAGE_LAYER_Z;
        obj.alpha = layer.alpha;
        obj.texture = Some(tex);

        out.push(ImageLayerObject {
            object_name: name,
            texture: tex,
            base_x,
            base_y,
            base_alpha: layer.alpha,
            animation: layer.animation.clone(),
        });
    }
    out
}

/// Advance drift/pulse animation for image layers. `time_s` is seconds since
/// boot; call once per frame. No-op for static layers or when the theme asks
/// for reduced motion.
pub fn tick_image_layers(
    sdi: &mut SdiRegistry,
    layers: &[ImageLayerObject],
    time_s: f32,
    reduced_motion: bool,
) {
    if reduced_motion {
        return;
    }
    for layer in layers {
        if !layer.is_animated() {
            continue;
        }
        let Ok(obj) = sdi.get_mut(&layer.object_name) else {
            continue;
        };
        let a = &layer.animation;
        let t = time_s + a.phase_offset;
        // Drift oscillates around the base position so decals never wander
        // off-screen; drift_x/y set the amplitude in pixels (one cycle per
        // ~6.28s at speed 1 px/s equivalent).
        if a.drift_x != 0.0 {
            obj.x = layer.base_x + (t.sin() * a.drift_x) as i32;
        }
        if a.drift_y != 0.0 {
            obj.y = layer.base_y + (t.cos() * a.drift_y) as i32;
        }
        if a.pulse_speed > 0.0 {
            let phase = (t * a.pulse_speed * std::f32::consts::TAU).sin() * 0.5 + 0.5;
            let min = a.pulse_min_alpha.clamp(0.0, 1.0);
            let level = min + (1.0 - min) * phase;
            obj.alpha = (layer.base_alpha as f32 * level).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Destroy the SDI objects and backend textures for a set of image layers.
/// Call on skin swap before creating the next skin's layers.
pub fn destroy_image_layers(
    sdi: &mut SdiRegistry,
    backend: &mut dyn SdiBackend,
    layers: &[ImageLayerObject],
) {
    for layer in layers {
        let _ = sdi.destroy(&layer.object_name);
        let _ = backend.destroy_texture(layer.texture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vector::background::{Anchor, LayerAnimation, LayerPosition};

    fn theme_layer(anchor: Anchor, animation: LayerAnimation) -> ImageLayerTheme {
        ImageLayerTheme {
            source: "assets/logo.png".to_string(),
            position: LayerPosition {
                anchor,
                offset_x: 0.0,
                offset_y: 0.0,
            },
            animation,
            alpha: 200,
            enabled: true,
        }
    }

    fn assets_with_logo(w: u32, h: u32) -> HashMap<String, SkinAsset> {
        let mut assets = HashMap::new();
        assets.insert(
            "assets/logo.png".to_string(),
            SkinAsset {
                width: w,
                height: h,
                rgba: vec![255; (w * h * 4) as usize],
            },
        );
        assets
    }

    #[test]
    fn create_positions_bottom_right_inside_screen() {
        let mut sdi = SdiRegistry::new();
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let layers = [theme_layer(Anchor::BottomRight, LayerAnimation::default())];
        let assets = assets_with_logo(64, 32);
        let created = create_image_layers(&mut sdi, &mut backend, &layers, &assets, 480, 272, 1.0);
        assert_eq!(created.len(), 1);
        let obj = sdi.get("bg_image_0").unwrap();
        assert_eq!(obj.x, 480 - 64);
        assert_eq!(obj.y, 272 - 32);
        assert_eq!(obj.w, 64);
        assert_eq!(obj.alpha, 200);
        assert!(obj.texture.is_some());
        assert_eq!(obj.z, IMAGE_LAYER_Z);
    }

    #[test]
    fn create_skips_missing_asset_and_disabled() {
        let mut sdi = SdiRegistry::new();
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let mut missing = theme_layer(Anchor::Center, LayerAnimation::default());
        missing.source = "assets/nope.png".to_string();
        let mut disabled = theme_layer(Anchor::Center, LayerAnimation::default());
        disabled.enabled = false;
        let assets = assets_with_logo(8, 8);
        let created = create_image_layers(
            &mut sdi,
            &mut backend,
            &[missing, disabled],
            &assets,
            480,
            272,
            1.0,
        );
        assert!(created.is_empty());
        assert!(!sdi.contains("bg_image_0"));
        assert!(!sdi.contains("bg_image_1"));
    }

    #[test]
    fn tick_pulses_alpha_and_respects_reduced_motion() {
        let mut sdi = SdiRegistry::new();
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let anim = LayerAnimation {
            pulse_speed: 1.0,
            pulse_min_alpha: 0.0,
            ..LayerAnimation::default()
        };
        let layers = [theme_layer(Anchor::Center, anim)];
        let assets = assets_with_logo(8, 8);
        let created = create_image_layers(&mut sdi, &mut backend, &layers, &assets, 480, 272, 1.0);

        // Quarter period of a 1 Hz pulse peaks the sine -> full base alpha.
        tick_image_layers(&mut sdi, &created, 0.25, false);
        assert_eq!(sdi.get("bg_image_0").unwrap().alpha, 200);
        // Three-quarter period bottoms out -> zero alpha.
        tick_image_layers(&mut sdi, &created, 0.75, false);
        assert_eq!(sdi.get("bg_image_0").unwrap().alpha, 0);

        // Reduced motion leaves the object untouched.
        tick_image_layers(&mut sdi, &created, 0.25, true);
        assert_eq!(sdi.get("bg_image_0").unwrap().alpha, 0);
    }

    #[test]
    fn destroy_removes_objects() {
        let mut sdi = SdiRegistry::new();
        let mut backend = oasis_test_backend::MockSdiCore::new(480, 272);
        let layers = [theme_layer(Anchor::Center, LayerAnimation::default())];
        let assets = assets_with_logo(8, 8);
        let created = create_image_layers(&mut sdi, &mut backend, &layers, &assets, 480, 272, 1.0);
        destroy_image_layers(&mut sdi, &mut backend, &created);
        assert!(!sdi.contains("bg_image_0"));
    }
}
