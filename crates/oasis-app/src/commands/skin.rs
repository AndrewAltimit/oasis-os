//! Skin swaps, live resolution changes and the backend-side skin assets
//! they invalidate (wallpaper texture, layout / bar / WM chrome textures,
//! image decal layers, the software cursor).

use oasis_backend_sdl::shader_bridge::SdlShaderBridge;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::AppRegistry;
use oasis_core::browser::BrowserConfig;
use oasis_core::cursor::CursorState;
use oasis_core::dashboard::{AppEntry, DashboardConfig, DashboardState, discover_apps_themed};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, resolve_skin_request};
use oasis_core::startmenu::StartMenuState;
use oasis_core::terminal_sdi;
use oasis_core::vfs::MemoryVfs;
use oasis_core::wallpaper;

use crate::app_state::AppState;
use crate::shell_backend::ShellBackend;
/// The dashboard's apps: everything installed under `/apps` plus the
/// plugin apps in the registry. Boot and every dashboard rebuild (skin
/// swap, resolution change) use this, so plugin apps never drop off.
pub(crate) fn dashboard_apps(
    vfs: &MemoryVfs,
    apps: &AppRegistry,
    at: &ActiveTheme,
) -> oasis_core::error::Result<Vec<AppEntry>> {
    let mut tiles = discover_apps_themed(vfs, "/apps", Some("OASISOS"), &at.icon.fallback_colors)?;
    tiles.extend(
        apps.iter()
            .filter(|reg| reg.from_plugin && reg.on_dashboard)
            .map(|reg| reg.to_app_entry()),
    );
    Ok(tiles)
}

/// Rebuild the dashboard and start menu for the current skin / theme /
/// resolution (per-skin free-layout icon positions restored), back on
/// page 0.
pub(super) fn rebuild_dashboard(state: &mut AppState, vfs: &MemoryVfs) {
    let dash_config = DashboardConfig::from_features(&state.skin.features, &state.active_theme);
    let apps = dashboard_apps(vfs, &state.content.apps, &state.active_theme).unwrap_or_else(|e| {
        log::warn!("app discovery failed: {e}");
        Vec::new()
    });
    state.ui.dashboard = DashboardState::new(dash_config, apps);
    crate::icon_drag::load_icon_positions(
        &state.settings,
        &state.skin.manifest.name,
        &mut state.ui.dashboard,
    );
    state.ui.bottom_bar.total_pages = state.ui.dashboard.page_count();
    state.ui.bottom_bar.current_page = 0;
    state.ui.start_menu = StartMenuState::new_with_theme(
        StartMenuState::default_items(&state.active_theme),
        &state.active_theme,
    );
}

/// Apply a skin swap after the Environment borrow has been dropped.
///
/// `name` may also be a variant request (`"@variant:dark"`), which derives
/// a Dark / Light / High-contrast variant of the currently active skin.
pub fn apply_skin_swap(name: &str, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs) {
    match resolve_skin_request(name, &state.skin) {
        Ok(new_skin) => apply_skin_object(new_skin, state, sdi, vfs),
        Err(e) => {
            state.terminal.print(format!("Skin error: {e}"));
        },
    }
}

/// Derive everything theme-dependent from `skin` (about to become the
/// active skin, laid out at `sw` x `sh`): the user's accessibility
/// preferences (reduced motion, font scale) patched on top of its
/// features, the active theme, the browser chrome and the WM theme.
fn derive_theme(state: &mut AppState, skin: &mut Skin, sw: u32, sh: u32) {
    let prefs = crate::user_prefs::current(state);
    prefs.patch_features(&mut skin.features);
    state.active_theme = ActiveTheme::from_skin(&skin.theme)
        .with_screen_size(sw, sh)
        .with_features(&skin.features);
    prefs.apply_font_scale(&mut state.active_theme);
    state.browser_config = BrowserConfig::from_skin_theme(&skin.theme);
    if let Some(ref mut bw) = state.content.browser {
        bw.apply_chrome_theme(&state.browser_config);
    }
    state.wm.set_theme(skin.theme.build_wm_theme());
}

/// Apply an already-resolved skin to the running session.
///
/// This is the in-memory swap entry point: it never touches the skin
/// registry on disk, so it also serves as the "Apply" (preview without
/// saving) path for the Settings Appearance editor.
pub fn apply_skin_object(
    new_skin: Skin,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
) {
    let sw = state.active_theme.screen_w;
    let sh = state.active_theme.screen_h;
    let mut swapped = Skin::swap_scaled(&state.skin, new_skin, sdi, sw, sh);
    derive_theme(state, &mut swapped, sw, sh);
    // Open windows take the new skin's chrome and work area.
    state.wm.restyle_windows(sdi);
    state.wm.fit_to_screen(sdi);

    // Component SDI objects (dashboard icons, status/bottom bar,
    // taskbar, start menu, toasts) are NOT part of `skin.layout`, so
    // `Skin::swap_scaled` didn't destroy them. Their decorative
    // attributes (gradient_top/bottom, text_shadow_*, stroke_*,
    // shadow_level, border_radius, …) persist from the previous
    // skin because each component's `update_sdi` only writes the
    // attributes the *current* skin needs. That bleed-through is
    // what caused icon labels to render invisibly (e.g. stale
    // gradient fill on the label object from a prior skin). Drop
    // those objects here so every component rebuilds them cleanly
    // on the next frame.
    clear_component_sdi_objects(sdi);

    state
        .terminal
        .print(format!("Switched to skin: {}", swapped.manifest.name));
    state.skin = swapped;
    rebuild_dashboard(state, vfs);
    // The status bar and taskbar keep their instances: their state (clock,
    // battery and Wi-Fi text, the selected top tab, taskbar order) is not
    // skin-specific, and their SDI objects rebuild from the new theme on
    // the next frame. (Fresh instances showed "00:00" until the next
    // periodic info update.) Their tab textures are re-uploaded by
    // `refresh_skin_assets`.

    // Mirror the parts of startup (`main()`) that depend on the
    // theme rather than on the window surface: clear color and
    // cursor scale are derived from the active theme, so they
    // have to be re-read whenever the theme changes.
    state.bg_color = state.active_theme.clear_color;
    state.ui.mouse_cursor.scale = state.active_theme.cursor_scale;
    // Swap-out frees the old skin's decoded SFX samples and loads
    // the new skin's [sounds] WAVs (mirrors image asset lifecycle).
    crate::ui_sfx::reload_for_skin(state);
    // The wallpaper texture was generated against the previous theme
    // (grid color, gradient stops, shader-layer visibility). The main
    // loop holds the backend needed to upload a fresh texture, so
    // flag it here and let `refresh_wallpaper_if_pending` do the work.
    state.skin_assets.pending_refresh = true;
    // Play the new skin's entrance so swaps feel like PSIX theme
    // loads (also masks the wallpaper regeneration pop).
    state.active_transition = crate::launch::make_entrance(
        &state.active_theme,
        state.skin.features.transition_fade_frames.unwrap_or(15),
        sw,
        sh,
    );
}

/// Destroy every SDI object owned by a UI component (dashboard icons,
/// status/bottom bar, taskbar, start menu, toasts) so the next frame
/// recreates them with fresh default attributes. This avoids decorative
/// attribute bleed-through across skin swaps — the classic symptom is
/// an icon label object keeping a gradient/shadow from the previous
/// skin, which can render the label invisible under the new skin.
///
/// Layout objects owned by `skin.layout` are left alone; `Skin::swap_scaled`
/// already destroyed them.
fn clear_component_sdi_objects(sdi: &mut SdiRegistry) {
    const COMPONENT_PREFIXES: &[&str] = &[
        "icon_",            // dashboard icons (icon_label_*, icon_shadow_*, …)
        "cursor_highlight", // dashboard selection highlight
        "hover_highlight",  // dashboard pointer-hover highlight
        "bar_",             // status bar + bottom bar
        "taskbar_",         // taskbar buttons + desktop indicator
        "start_btn_",       // start menu button on the taskbar
        "sm_",              // start menu panel, items, footer
        "toast_",           // toast notifications
    ];
    let to_destroy: Vec<String> = sdi
        .names()
        .filter(|n| COMPONENT_PREFIXES.iter().any(|p| n.starts_with(p)))
        .map(|n| n.to_string())
        .collect();
    for name in to_destroy {
        let _ = sdi.destroy(&name);
    }
}

/// If `state.skin_assets.pending_refresh` is set, regenerate the wallpaper
/// texture against the current theme and toggle its SDI visibility based on
/// whether the new skin uses a shader layer. The flag is cleared after
/// processing. No-op when not set.
pub fn refresh_wallpaper_if_pending(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut impl ShellBackend,
) {
    if !state.skin_assets.pending_refresh {
        return;
    }

    let w = state.active_theme.screen_w;
    let h = state.active_theme.screen_h;
    let old_tex = sdi.get("wallpaper").ok().and_then(|o| o.texture);
    let wp_data = wallpaper::generate_with_assets(w, h, &state.active_theme, &state.skin.assets);
    match backend.load_texture(w, h, &wp_data) {
        Ok(new_tex) => {
            // Only clear the flag on a successful upload so transient backend
            // failures (GPU OOM, driver hiccups) retry on the next frame
            // instead of leaving the wallpaper stale until the next skin swap.
            state.skin_assets.pending_refresh = false;
            terminal_sdi::setup_wallpaper(sdi, new_tex, w, h);
            // Hide the raster wallpaper under shader-driven skins so the
            // shader's output isn't overdrawn by the SDI wallpaper object.
            if let Ok(obj) = sdi.get_mut("wallpaper") {
                obj.visible =
                    oasis_core::vector_overlay::get_shader_layer(&state.active_theme).is_none();
            }
            if let Some(tex) = old_tex {
                let _ = backend.destroy_texture(tex);
            }
            // A pending refresh also means the skin (or resolution) changed,
            // which invalidates layout textures and image decal layers.
            refresh_skin_assets(state, sdi, backend);
        },
        Err(e) => {
            state
                .terminal
                .print(format!("Warning: wallpaper refresh failed: {e}"));
        },
    }
}

/// Rebuild backend-side skin assets: layout `texture =` uploads and image
/// background layers. Destroys the previous skin's textures first.
pub fn refresh_skin_assets(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut impl ShellBackend,
) {
    // Install the skin's `[typography] font` (or restore the bitmap font).
    // This also flushes the backend glyph cache, whose textures belong to
    // the outgoing font.
    backend.set_font(state.skin.active_font_bytes());

    for tex in state.skin_assets.layout_textures.drain(..) {
        let _ = backend.destroy_texture(tex);
    }
    oasis_core::image_layers::destroy_image_layers(sdi, backend, &state.skin_assets.image_layers);

    // Cached vector layer ops belong to the outgoing theme/resolution (D4).
    state.skin_assets.background_cache.invalidate();
    state.skin_assets.chrome_cache.invalidate();

    state.skin_assets.layout_textures = state.skin.upload_layout_textures(sdi, backend);

    // Top-tab pill textures (B5): upload the skin's `tab_texture_*` bar
    // slots and hand the ids to the status bar. They share the layout
    // texture lifecycle (destroyed above on the next refresh).
    state.ui.status_bar.tab_texture_active = upload_bar_texture(
        &state.skin,
        state.active_theme.bar.tab_texture_active.as_deref(),
        backend,
        &mut state.skin_assets.layout_textures,
    );
    state.ui.status_bar.tab_texture_inactive = upload_bar_texture(
        &state.skin,
        state.active_theme.bar.tab_texture_inactive.as_deref(),
        backend,
        &mut state.skin_assets.layout_textures,
    );

    // WM nine-patch chrome (A2): resolve the theme's titlebar/frame patch
    // configs into uploaded textures and re-stamp any open windows.
    {
        let mut wm_theme = state.wm.theme().clone();
        wm_theme.titlebar_patch = upload_wm_patch(
            &state.skin,
            wm_theme.titlebar_nine_patch.as_ref(),
            backend,
            &mut state.skin_assets.layout_textures,
        );
        wm_theme.frame_patch = upload_wm_patch(
            &state.skin,
            wm_theme.frame_nine_patch.as_ref(),
            backend,
            &mut state.skin_assets.layout_textures,
        );
        let dirty = wm_theme.titlebar_patch.is_some() || wm_theme.frame_patch.is_some();
        state.wm.set_theme(wm_theme);
        if dirty || state.wm.window_count() > 0 {
            state.wm.apply_chrome_patches(sdi);
        }
    }

    let sw = state.active_theme.screen_w;
    let sh = state.active_theme.screen_h;
    // Decals scale uniformly with the skin's native resolution so logos
    // keep their aspect ratio on scaled-up screens.
    let base_w = state.skin.manifest.screen_width.max(1) as f32;
    let base_h = state.skin.manifest.screen_height.max(1) as f32;
    let scale = (sw as f32 / base_w).min(sh as f32 / base_h);
    state.skin_assets.image_layers = oasis_core::image_layers::create_image_layers(
        sdi,
        backend,
        &state.active_theme.image_layers,
        &state.skin.assets,
        sw,
        sh,
        scale,
    );

    // Software cursor: themed `[cursor]` texture when the skin ships one,
    // procedural arrow otherwise. Skins that don't opt in keep the host
    // OS pointer (and no SDI cursor object is shown).
    if let Some(tex) = state.skin_assets.cursor_texture.take() {
        let _ = backend.destroy_texture(tex);
    }
    if state.skin.features.software_cursor {
        let themed = state
            .active_theme
            .cursor_texture
            .as_ref()
            .and_then(|name| state.skin.assets.get(name))
            .map(|a| (a.rgba.clone(), a.width, a.height));
        let is_themed = themed.is_some();
        let (pixels, cw, ch) = themed.unwrap_or_else(|| {
            oasis_core::cursor::generate_cursor_pixels(state.active_theme.cursor_scale)
        });
        match backend.load_texture(cw, ch, &pixels) {
            Ok(tex) => {
                state.skin_assets.cursor_texture = Some(tex);
                let cursor = &mut state.ui.mouse_cursor;
                cursor.size = is_themed.then_some((cw, ch));
                cursor.hotspot = if is_themed {
                    state.active_theme.cursor_hotspot
                } else {
                    (0, 0)
                };
                cursor.update_sdi(sdi);
                if let Ok(obj) = sdi.get_mut("mouse_cursor") {
                    obj.texture = Some(tex);
                }
                backend.set_host_cursor_visible(false);
            },
            Err(e) => log::warn!("software cursor texture upload failed: {e}"),
        }
    } else {
        if let Ok(obj) = sdi.get_mut("mouse_cursor") {
            obj.visible = false;
        }
        backend.set_host_cursor_visible(true);
    }
}

/// Resolve a WM nine-patch config (asset key + insets) into an uploaded
/// texture + slicing metadata. Returns None when unset, the asset is
/// missing (already flagged by `skin lint`), or the upload fails.
fn upload_wm_patch(
    skin: &Skin,
    config: Option<&(String, [u16; 4])>,
    backend: &mut impl ShellBackend,
    owned: &mut Vec<oasis_core::backend::TextureId>,
) -> Option<(
    oasis_core::backend::TextureId,
    oasis_core::nine_patch::NinePatchSlices,
)> {
    let (key, insets) = config?;
    let asset = skin.assets.get(key)?;
    match backend.load_texture(asset.width, asset.height, &asset.rgba) {
        Ok(tex) => {
            owned.push(tex);
            let [left, top, right, bottom] = *insets;
            Some((
                tex,
                oasis_core::nine_patch::NinePatchSlices {
                    tex_width: asset.width,
                    tex_height: asset.height,
                    left,
                    top,
                    right,
                    bottom,
                },
            ))
        },
        Err(e) => {
            log::warn!("WM chrome texture upload failed for '{key}': {e}");
            None
        },
    }
}

/// Upload a bar chrome asset (if the skin sets and ships it) and track the
/// texture id in `owned` for destruction on the next skin swap. Missing
/// assets were already flagged by `skin lint` / load-time validation.
fn upload_bar_texture(
    skin: &Skin,
    asset_key: Option<&str>,
    backend: &mut impl ShellBackend,
    owned: &mut Vec<oasis_core::backend::TextureId>,
) -> Option<oasis_core::backend::TextureId> {
    let key = asset_key?;
    let asset = skin.assets.get(key)?;
    match backend.load_texture(asset.width, asset.height, &asset.rgba) {
        Ok(tex) => {
            owned.push(tex);
            Some(tex)
        },
        Err(e) => {
            log::warn!("bar texture upload failed for '{key}': {e}");
            None
        },
    }
}

/// Minimum virtual resolution accepted from a live resize request. Anything
/// smaller makes the dashboard/window-manager layout unusable.
const MIN_RESOLUTION_W: u32 = 320;
const MIN_RESOLUTION_H: u32 = 240;
/// Maximum virtual resolution — primarily a sanity bound. Requests beyond
/// this are rejected rather than clamped so the caller notices.
const MAX_RESOLUTION_W: u32 = 3840;
const MAX_RESOLUTION_H: u32 = 2160;

/// Apply a live resolution change. Rebuilds skin layout at the new size,
/// resizes the SDL window + shader bridge, and re-derives the dashboard,
/// window manager, and cursor state. No-op if `(new_w, new_h)` already
/// matches the active resolution.
pub fn apply_resolution_change(
    new_w: u32,
    new_h: u32,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut impl ShellBackend,
    shader_bridge: &mut Option<SdlShaderBridge>,
    vfs: &MemoryVfs,
) {
    if new_w < MIN_RESOLUTION_W
        || new_h < MIN_RESOLUTION_H
        || new_w > MAX_RESOLUTION_W
        || new_h > MAX_RESOLUTION_H
    {
        state.terminal.print(format!(
            "Resolution {new_w}x{new_h} out of range ({MIN_RESOLUTION_W}x{MIN_RESOLUTION_H} \
             to {MAX_RESOLUTION_W}x{MAX_RESOLUTION_H})"
        ));
        return;
    }

    if state.active_theme.screen_w == new_w && state.active_theme.screen_h == new_h {
        return;
    }

    // Resize the host window first so any subsequent render call sees the
    // new viewport.
    if let Err(e) = backend.set_window_size(new_w, new_h) {
        state
            .terminal
            .print(format!("Resolution change failed: {e}"));
        return;
    }

    // Resize the shader compositor if present.
    if let Some(bridge) = shader_bridge.as_mut() {
        bridge.resize(new_w, new_h);
    }

    state.config.screen_width = new_w;
    state.config.screen_height = new_h;

    // Re-apply the running skin's layout at the new target size. This
    // rebuilds every skin-owned SDI object (taskbar, dashboard tiles, etc.)
    // for the new canvas. The skin is cloned rather than re-resolved by
    // name: `Skin::swap_scaled` consumes its new skin, and a lookup by name
    // would fail for external skins (strict resolve) and drop an in-memory
    // theme preview. Layout coordinates stay at the skin's base resolution
    // (`apply_layout_scaled` scales on the way into the SDI), so re-applying
    // a clone is exact.
    let current = state.skin.clone();
    let mut swapped = Skin::swap_scaled(&state.skin, current, sdi, new_w, new_h);
    derive_theme(state, &mut swapped, new_w, new_h);
    state.skin = swapped;

    // Rebuild dashboard + bars for the new layout grid.
    rebuild_dashboard(state, vfs);

    state.wm.set_screen_size(new_w, new_h);
    state.wm.restyle_windows(sdi);
    // Refit open windows: maximized / snapped / tiled / fullscreen ones
    // take the new screen's geometry and floating ones are pulled back on
    // screen, so no titlebar or close button ends up out of reach.
    state.wm.fit_to_screen(sdi);
    state.ui.mouse_cursor = CursorState::new(new_w, new_h);
    state.ui.mouse_cursor.scale = state.active_theme.cursor_scale;

    // Regenerate the wallpaper texture at the new size. The old wallpaper
    // object survived `swap_scaled` (it isn't a skin-layout object).
    state.skin_assets.pending_refresh = true;
    refresh_wallpaper_if_pending(state, sdi, backend);

    state.terminal.print(format!("Resolution: {new_w}x{new_h}"));
}
