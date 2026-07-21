//! Shared skin-asset setup for the screenshot capture binaries.
//!
//! Mirrors the runtime `refresh_skin_assets` path in the main binary:
//! layout `texture =` uploads, top-tab pill textures, image decal layers,
//! WM nine-patch chrome, and the themed software cursor — so captures
//! exercise the same skin features the live shell renders. Every helper
//! is a no-op for skins without the corresponding assets, keeping
//! pre-existing goldens pixel-identical.

use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{SdiCore, TextureId};
use oasis_core::nine_patch::NinePatchSlices;
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::statusbar::StatusBar;

/// Upload layout `texture =` / `nine_patch` assets, top-tab pill
/// textures, and image decal layers. Call after `skin.apply_layout`.
pub fn setup(
    skin: &Skin,
    at: &ActiveTheme,
    sdi: &mut SdiRegistry,
    backend: &mut SdlBackend,
    status_bar: &mut StatusBar,
) {
    let _ = skin.upload_layout_textures(sdi, backend);
    status_bar.tab_texture_active =
        upload_asset(skin, at.bar.tab_texture_active.as_deref(), backend);
    status_bar.tab_texture_inactive =
        upload_asset(skin, at.bar.tab_texture_inactive.as_deref(), backend);
    let _ = oasis_core::image_layers::create_image_layers(
        sdi,
        backend,
        &at.image_layers,
        &skin.assets,
        at.screen_w,
        at.screen_h,
        1.0, // captures render at the skin's native resolution
    );
}

/// Upload a named skin asset as a texture (screenshot runs never destroy
/// textures — the process exits after the capture).
pub fn upload_asset(
    skin: &Skin,
    asset_key: Option<&str>,
    backend: &mut SdlBackend,
) -> Option<TextureId> {
    let asset = skin.assets.get(asset_key?)?;
    backend
        .load_texture(asset.width, asset.height, &asset.rgba)
        .ok()
}

/// Resolve the theme's `titlebar_nine_patch` / `frame_nine_patch` configs
/// into uploaded textures + slice metrics on a `WmTheme`.
pub fn resolve_wm_patches(
    skin: &Skin,
    theme: &mut oasis_core::wm::WmTheme,
    backend: &mut SdlBackend,
) {
    theme.titlebar_patch = upload_patch(skin, theme.titlebar_nine_patch.as_ref(), backend);
    theme.frame_patch = upload_patch(skin, theme.frame_nine_patch.as_ref(), backend);
}

fn upload_patch(
    skin: &Skin,
    config: Option<&(String, [u16; 4])>,
    backend: &mut SdlBackend,
) -> Option<(TextureId, NinePatchSlices)> {
    let (key, insets) = config?;
    let asset = skin.assets.get(key)?;
    let tex = backend
        .load_texture(asset.width, asset.height, &asset.rgba)
        .ok()?;
    let [left, top, right, bottom] = *insets;
    Some((
        tex,
        NinePatchSlices {
            tex_width: asset.width,
            tex_height: asset.height,
            left,
            top,
            right,
            bottom,
        },
    ))
}

/// Themed `[cursor]` pixels when the skin opts into a software cursor
/// with a texture; `None` means keep the procedural arrow.
pub fn themed_cursor(skin: &Skin, at: &ActiveTheme) -> Option<(Vec<u8>, u32, u32)> {
    if !skin.features.software_cursor {
        return None;
    }
    let asset = skin.assets.get(at.cursor_texture.as_ref()?)?;
    Some((asset.rgba.clone(), asset.width, asset.height))
}
