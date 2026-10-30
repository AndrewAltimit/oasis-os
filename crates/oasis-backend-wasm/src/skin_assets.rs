//! Skin image assets on the WASM host: the image wallpaper, layout
//! `texture =` uploads and `[[background_layers]] kind = "image"` decals.
//!
//! Mirrors the desktop shell's `install_wallpaper` /
//! `refresh_wallpaper_if_pending` / `refresh_skin_assets`. The WASM host
//! used to build its wallpaper with `wallpaper::generate_from_config`,
//! which for `style = "image"` is only the solid base color, and never
//! created image layers, so skins like `psix-hifi` showed a plain orange
//! background with no wordmark.

use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{SdiCore, TextureId};
use oasis_core::image_layers::{self, ImageLayerObject};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::terminal_sdi;
use oasis_core::wallpaper;

/// The wallpaper pixels for `theme` at `w`x`h`, compositing the skin's
/// image asset for `style = "image"`.
pub(crate) fn wallpaper_pixels(w: u32, h: u32, theme: &ActiveTheme, skin: &Skin) -> Vec<u8> {
    wallpaper::generate_with_assets(w, h, theme, &skin.assets)
}

/// Uniform scale from the skin's native resolution to the screen, so
/// decals keep their aspect ratio (the desktop shell's rule).
pub(crate) fn decal_scale(skin: &Skin, screen_w: u32, screen_h: u32) -> f32 {
    let base_w = skin.manifest.screen_width.max(1) as f32;
    let base_h = skin.manifest.screen_height.max(1) as f32;
    (screen_w as f32 / base_w).min(screen_h as f32 / base_h)
}

/// Upload the skin's layout textures and create its image decal layers.
pub(crate) fn create_skin_assets(
    sdi: &mut SdiRegistry,
    backend: &mut crate::WasmBackend,
    skin: &Skin,
    theme: &ActiveTheme,
) -> (Vec<TextureId>, Vec<ImageLayerObject>) {
    let layout = skin.upload_layout_textures(sdi, backend);
    let (w, h) = (theme.screen_w, theme.screen_h);
    let layers = image_layers::create_image_layers(
        sdi,
        backend,
        &theme.image_layers,
        &skin.assets,
        w,
        h,
        decal_scale(skin, w, h),
    );
    (layout, layers)
}

impl crate::OasisWasm {
    /// Rebuild the wallpaper texture, layout textures and image layers for
    /// the current skin (after a skin swap), freeing the previous ones.
    pub(crate) fn refresh_skin_assets(&mut self) {
        let (w, h) = (self.width, self.height);
        let old_tex = self.sdi.get("wallpaper").ok().and_then(|o| o.texture);
        let pixels = wallpaper_pixels(w, h, &self.active_theme, &self.skin);
        match self.backend.load_texture(w, h, &pixels) {
            Ok(tex) => {
                terminal_sdi::setup_wallpaper(&mut self.sdi, tex, w, h);
                if let Ok(obj) = self.sdi.get_mut("wallpaper") {
                    obj.visible =
                        oasis_core::vector_overlay::get_shader_layer(&self.active_theme).is_none();
                }
                if let Some(old) = old_tex.filter(|&old| old != tex) {
                    let _ = self.backend.destroy_texture(old);
                }
            },
            Err(e) => log::warn!("wallpaper refresh failed: {e}"),
        }

        for tex in self.skin_layout_textures.drain(..) {
            let _ = self.backend.destroy_texture(tex);
        }
        image_layers::destroy_image_layers(&mut self.sdi, &mut self.backend, &self.image_layers);
        let (layout, layers) = create_skin_assets(
            &mut self.sdi,
            &mut self.backend,
            &self.skin,
            &self.active_theme,
        );
        self.skin_layout_textures = layout;
        self.image_layers = layers;
    }

    /// Advance drift / pulse animation of the image decal layers.
    pub(crate) fn tick_image_layers(&mut self) {
        if !self.image_layers.is_empty() {
            image_layers::tick_image_layers(
                &mut self.sdi,
                &self.image_layers,
                self.anim_secs,
                self.active_theme.background_reduced_motion,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn psix_hifi() -> (Skin, ActiveTheme) {
        let skin = oasis_skin::resolve_skin("psix-hifi").unwrap();
        let theme = ActiveTheme::from_skin(&skin.theme)
            .with_screen_size(skin.manifest.screen_width, skin.manifest.screen_height)
            .with_features(&skin.features);
        (skin, theme)
    }

    #[test]
    fn psix_hifi_wallpaper_is_the_image_not_the_base_color() {
        let (skin, theme) = psix_hifi();
        assert_eq!(theme.wallpaper.style, "image");
        let (w, h) = (theme.screen_w, theme.screen_h);
        let with_assets = wallpaper_pixels(w, h, &theme, &skin);
        let base_only = wallpaper::generate_from_config(w, h, &theme);
        assert_eq!(with_assets.len(), (w * h * 4) as usize);
        // The ribbon image covers the screen: most pixels differ from the
        // solid base color the WASM host used to show.
        let differing = with_assets
            .as_chunks::<4>()
            .0
            .iter()
            .zip(base_only.as_chunks::<4>().0.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            differing > (w * h / 2) as usize,
            "only {differing} of {} pixels come from the image",
            w * h
        );
    }

    #[test]
    fn psix_hifi_declares_an_image_layer_with_its_asset() {
        let (skin, theme) = psix_hifi();
        assert!(!theme.image_layers.is_empty(), "wordmark layer missing");
        for layer in &theme.image_layers {
            assert!(
                skin.assets.contains_key(&layer.source),
                "asset {} not embedded",
                layer.source
            );
        }
        assert!((decal_scale(&skin, 960, 544) - 2.0).abs() < 1e-6);
    }
}
