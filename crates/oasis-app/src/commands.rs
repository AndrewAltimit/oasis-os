use oasis_backend_sdl::SdlBackend;
use oasis_backend_sdl::shader_bridge::SdlShaderBridge;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{SdiCore, SdiText};
use oasis_core::browser::BrowserConfig;
use oasis_core::cursor::CursorState;
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps_themed};
use oasis_core::net::{ListenerConfig, RemoteClient, RemoteListener};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, SkinTheme, resolve_skin, resolve_skin_request};
use oasis_core::startmenu::StartMenuState;
use oasis_core::terminal::{CommandOutput, CommandSignal, Environment};
use oasis_core::terminal_sdi;
use oasis_core::transfer::FtpServer;
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;

#[cfg(test)]
use crate::app_state::UiLayer;
use crate::app_state::{AppState, ContentLayer, NetworkLayer, TerminalLayer};

/// Process a local terminal command result. Returns a pending skin swap name
/// if the command was `SkinSwap`.
pub fn process_command_output(
    result: oasis_core::error::Result<CommandOutput>,
    state: &mut AppState,
) -> Option<String> {
    match result {
        Ok(CommandOutput::Text(text)) => {
            for l in text.lines() {
                state.terminal.output_lines.push(l.to_string());
            }
        },
        Ok(CommandOutput::Table { headers, rows }) => {
            state.terminal.output_lines.push(headers.join(" | "));
            for row in &rows {
                state.terminal.output_lines.push(row.join(" | "));
            }
        },
        Ok(CommandOutput::Clear) => state.terminal.output_lines.clear(),
        Ok(CommandOutput::None) => {},
        Ok(CommandOutput::Signal(CommandSignal::ListenToggle { port })) => {
            if port == 0 {
                if let Some(ref mut l) = state.net.listener {
                    l.stop();
                    state.net.listener = None;
                    state
                        .terminal
                        .output_lines
                        .push("Remote listener stopped.".to_string());
                } else {
                    state
                        .terminal
                        .output_lines
                        .push("No listener running.".to_string());
                }
            } else if state.net.listener.is_some() {
                state
                    .terminal
                    .output_lines
                    .push("Listener already running. Use 'listen stop' first.".to_string());
            } else {
                let cfg = ListenerConfig {
                    port,
                    psk: String::new(),
                    max_connections: 4,
                    ..ListenerConfig::default()
                };
                let mut l = RemoteListener::new(cfg);
                match l.start(&mut state.net.backend) {
                    Ok(()) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Listening on port {port}."));
                        state.net.listener = Some(l);
                    },
                    Err(e) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Listen error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::Signal(CommandSignal::RemoteConnect { address, port, psk })) => {
            if state.net.remote_client.is_some() {
                state
                    .terminal
                    .output_lines
                    .push("Already connected. Disconnect first.".to_string());
            } else {
                let mut client = RemoteClient::new();
                match client.connect(&mut state.net.backend, &address, port, psk.as_deref()) {
                    Ok(()) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Connected to {address}:{port}."));
                        state.net.remote_client = Some(client);
                    },
                    Err(e) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("Connect error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox { enable })) => {
            if let Some(ref mut bw) = state.content.browser {
                bw.config.features.sandbox_only = enable;
            }
            let st = if enable {
                "on (VFS only)"
            } else {
                "off (HTTP enabled)"
            };
            state
                .terminal
                .output_lines
                .push(format!("Browser sandbox: {st}"));
        },
        Ok(CommandOutput::Signal(CommandSignal::FtpToggle { port, password })) => {
            if port == 0 {
                if let Some(ref mut f) = state.net.ftp_server {
                    f.stop();
                    state.net.ftp_server = None;
                    state
                        .terminal
                        .output_lines
                        .push("FTP server stopped.".to_string());
                } else {
                    state
                        .terminal
                        .output_lines
                        .push("No FTP server running.".to_string());
                }
            } else if state.net.ftp_server.is_some() {
                state
                    .terminal
                    .output_lines
                    .push("FTP server already running. Use 'ftp stop' first.".to_string());
            } else {
                let mut server = FtpServer::new(port);
                if let Some(pass) = password {
                    server = server.with_password(pass);
                }
                match server.start(&mut state.net.backend) {
                    Ok(()) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("FTP server listening on port {port}."));
                        state.net.ftp_server = Some(server);
                    },
                    Err(e) => {
                        state
                            .terminal
                            .output_lines
                            .push(format!("FTP server error: {e}"));
                    },
                }
            }
        },
        Ok(CommandOutput::Signal(CommandSignal::McpToggle { start, port, token })) => {
            #[cfg(feature = "mcp")]
            {
                if start {
                    start_mcp_server(state, port, token);
                } else if let Some(mut server) = state.mcp.take() {
                    server.stop();
                    state
                        .terminal
                        .output_lines
                        .push("MCP server stopped.".to_string());
                } else {
                    state
                        .terminal
                        .output_lines
                        .push("No MCP server running.".to_string());
                }
            }
            #[cfg(not(feature = "mcp"))]
            {
                let _ = (start, port, token);
                state
                    .terminal
                    .output_lines
                    .push("MCP support not compiled in (build with --features mcp).".to_string());
            }
        },
        Ok(CommandOutput::Signal(CommandSignal::SkinSwap { name })) => {
            return Some(name);
        },
        Ok(CommandOutput::Multi(outputs)) => {
            let mut skin_swap = None;
            for output in outputs {
                let result = process_command_output(Ok(output), state);
                if result.is_some() {
                    skin_swap = result;
                }
            }
            return skin_swap;
        },
        Err(e) => {
            // Red via the skin's ANSI palette (SGR 31); the terminal
            // renderer resolves the escape into a themed colored run.
            state
                .terminal
                .output_lines
                .push(oasis_core::ansi::colorize(&format!("error: {e}"), 31));
        },
    }
    None
}

/// Apply a skin swap after the Environment borrow has been dropped.
///
/// `name` may also be a variant request (`"@variant:dark"`), which derives
/// a Dark / Light / High-contrast variant of the currently active skin.
pub fn apply_skin_swap(name: &str, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs) {
    match resolve_skin_request(name, &state.skin) {
        Ok(new_skin) => apply_skin_object(new_skin, state, sdi, vfs),
        Err(e) => {
            state.terminal.output_lines.push(format!("Skin error: {e}"));
        },
    }
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
    let swapped = Skin::swap_scaled(&state.skin, new_skin, sdi, sw, sh);
    state.active_theme = ActiveTheme::from_skin(&swapped.theme)
        .with_screen_size(sw, sh)
        .with_features(&swapped.features);
    state.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
    state.wm.set_theme(swapped.theme.build_wm_theme());

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

    let dash_config = DashboardConfig::from_features(&swapped.features, &state.active_theme);
    let apps = discover_apps_themed(
        vfs,
        "/apps",
        Some("OASISOS"),
        &state.active_theme.icon.fallback_colors,
    )
    .unwrap_or_default();
    state.ui.dashboard = DashboardState::new(dash_config, apps);
    crate::icon_drag::load_icon_positions(
        &state.settings,
        &swapped.manifest.name,
        &mut state.ui.dashboard,
    );
    state.ui.bottom_bar.total_pages = state.ui.dashboard.page_count();
    state.ui.bottom_bar.current_page = 0;
    state.ui.start_menu = StartMenuState::new_with_theme(
        StartMenuState::default_items(&state.active_theme),
        &state.active_theme,
    );
    state.ui.status_bar = oasis_core::statusbar::StatusBar::new();
    state.ui.taskbar = oasis_core::taskbar::Taskbar::new();

    // Mirror the parts of startup (`main()`) that depend on the
    // theme rather than on the window surface: clear color and
    // cursor scale are derived from the active theme, so they
    // have to be re-read whenever the theme changes.
    state.bg_color = state.active_theme.clear_color;
    state.ui.mouse_cursor.scale = state.active_theme.cursor_scale;

    state
        .terminal
        .output_lines
        .push(format!("Switched to skin: {}", swapped.manifest.name));
    state.skin = swapped;
    // Swap-out frees the old skin's decoded SFX samples and loads
    // the new skin's [sounds] WAVs (mirrors image asset lifecycle).
    crate::ui_sfx::reload_for_skin(state);
    // The wallpaper texture was generated against the previous theme
    // (grid color, gradient stops, shader-layer visibility). The main
    // loop holds the backend needed to upload a fresh texture, so
    // flag it here and let `refresh_wallpaper_if_pending` do the work.
    state.pending_wallpaper_refresh = true;
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
        "cursor_highlight", // dashboard selector (now invisible, but still rebuilt)
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

/// If `state.pending_wallpaper_refresh` is set, regenerate the wallpaper
/// texture against the current theme and toggle its SDI visibility based on
/// whether the new skin uses a shader layer. The flag is cleared after
/// processing. No-op when not set.
pub fn refresh_wallpaper_if_pending(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut SdlBackend,
) {
    if !state.pending_wallpaper_refresh {
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
            state.pending_wallpaper_refresh = false;
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
                .output_lines
                .push(format!("Warning: wallpaper refresh failed: {e}"));
        },
    }
}

/// Rebuild backend-side skin assets: layout `texture =` uploads and image
/// background layers. Destroys the previous skin's textures first.
pub fn refresh_skin_assets(state: &mut AppState, sdi: &mut SdiRegistry, backend: &mut SdlBackend) {
    // Install the skin's `[typography] font` (or restore the bitmap font).
    // This also flushes the backend glyph cache, whose textures belong to
    // the outgoing font.
    backend.set_font(state.skin.active_font_bytes());

    for tex in state.skin_layout_textures.drain(..) {
        let _ = backend.destroy_texture(tex);
    }
    oasis_core::image_layers::destroy_image_layers(sdi, backend, &state.image_layers);

    // Cached vector layer ops belong to the outgoing theme/resolution (D4).
    state.background_layer_cache.invalidate();
    state.chrome_layer_cache.invalidate();

    state.skin_layout_textures = state.skin.upload_layout_textures(sdi, backend);

    // Top-tab pill textures (B5): upload the skin's `tab_texture_*` bar
    // slots and hand the ids to the status bar. They share the layout
    // texture lifecycle (destroyed above on the next refresh).
    state.ui.status_bar.tab_texture_active = upload_bar_texture(
        &state.skin,
        state.active_theme.bar.tab_texture_active.as_deref(),
        backend,
        &mut state.skin_layout_textures,
    );
    state.ui.status_bar.tab_texture_inactive = upload_bar_texture(
        &state.skin,
        state.active_theme.bar.tab_texture_inactive.as_deref(),
        backend,
        &mut state.skin_layout_textures,
    );

    // WM nine-patch chrome (A2): resolve the theme's titlebar/frame patch
    // configs into uploaded textures and re-stamp any open windows.
    {
        let mut wm_theme = state.wm.theme().clone();
        wm_theme.titlebar_patch = upload_wm_patch(
            &state.skin,
            wm_theme.titlebar_nine_patch.as_ref(),
            backend,
            &mut state.skin_layout_textures,
        );
        wm_theme.frame_patch = upload_wm_patch(
            &state.skin,
            wm_theme.frame_nine_patch.as_ref(),
            backend,
            &mut state.skin_layout_textures,
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
    state.image_layers = oasis_core::image_layers::create_image_layers(
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
    if let Some(tex) = state.cursor_texture.take() {
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
                state.cursor_texture = Some(tex);
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
    backend: &mut SdlBackend,
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
    backend: &mut SdlBackend,
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
    backend: &mut SdlBackend,
    shader_bridge: &mut Option<SdlShaderBridge>,
    vfs: &MemoryVfs,
) {
    if new_w < MIN_RESOLUTION_W
        || new_h < MIN_RESOLUTION_H
        || new_w > MAX_RESOLUTION_W
        || new_h > MAX_RESOLUTION_H
    {
        state.terminal.output_lines.push(format!(
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
            .output_lines
            .push(format!("Resolution change failed: {e}"));
        return;
    }

    // Resize the shader compositor if present.
    if let Some(bridge) = shader_bridge.as_mut() {
        bridge.resize(new_w, new_h);
    }

    state.config.screen_width = new_w;
    state.config.screen_height = new_h;

    // Re-apply the current skin's layout at the new target size. This rebuilds
    // every skin-owned SDI object (taskbar, dashboard tiles, etc.) for the new
    // canvas. We clone the current skin name because `Skin::swap_scaled`
    // consumes the new skin, and we want to reuse the already-resolved skin
    // rather than re-reading it from disk.
    let current_skin_name = state.skin.manifest.name.clone();
    match resolve_skin(&current_skin_name) {
        Ok(fresh_skin) => {
            let swapped = Skin::swap_scaled(&state.skin, fresh_skin, sdi, new_w, new_h);
            state.active_theme = ActiveTheme::from_skin(&swapped.theme)
                .with_screen_size(new_w, new_h)
                .with_features(&swapped.features);
            state.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
            state.wm.set_theme(swapped.theme.build_wm_theme());
            state.skin = swapped;
        },
        Err(e) => {
            // Reloading the skin failed (shouldn't happen for a built-in),
            // but we still need to keep the theme dimensions consistent with
            // the resized window so rendering doesn't draw to stale coords.
            // We also refresh the WM theme from the existing `state.skin` so
            // `state.wm` stays consistent with the skin that's actually
            // still active (rather than holding whatever theme it was
            // constructed with before this fallback path ran).
            state
                .terminal
                .output_lines
                .push(format!("Warning: skin reload failed: {e}"));
            state.active_theme = state
                .active_theme
                .clone()
                .with_screen_size(new_w, new_h)
                .with_features(&state.skin.features);
            state.wm.set_theme(state.skin.theme.build_wm_theme());
        },
    }

    // Rebuild dashboard + bars for the new layout grid.
    let dash_config = DashboardConfig::from_features(&state.skin.features, &state.active_theme);
    let apps = discover_apps_themed(
        vfs,
        "/apps",
        Some("OASISOS"),
        &state.active_theme.icon.fallback_colors,
    )
    .unwrap_or_default();
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

    state.wm.set_screen_size(new_w, new_h);
    // `set_screen_size` updates the viewport bounds but leaves open windows
    // at their original coordinates. On a downward resize a window near the
    // old right/bottom edge can end up fully off-screen and unreachable.
    // `move_window(id, 0, 0, sdi)` is a no-op delta but runs the positions
    // through `clamp_position`, which pulls each titlebar back on-screen.
    let window_ids: Vec<String> = state
        .wm
        .windows()
        .iter()
        .map(|w| w.id.as_str().to_string())
        .collect();
    for id in window_ids {
        let _ = state.wm.move_window(&id, 0, 0, sdi);
    }
    state.ui.mouse_cursor = CursorState::new(new_w, new_h);
    state.ui.mouse_cursor.scale = state.active_theme.cursor_scale;

    // Regenerate the wallpaper texture at the new size. The old wallpaper
    // object survived `swap_scaled` (it isn't a skin-layout object).
    state.pending_wallpaper_refresh = true;
    refresh_wallpaper_if_pending(state, sdi, backend);

    state
        .terminal
        .output_lines
        .push(format!("Resolution: {new_w}x{new_h}"));
}

/// Publish the current runtime state (skin, resolution, backend) to VFS so
/// the Settings app and any other UI can read it on demand. Called on
/// startup and after every apply.
pub fn publish_runtime_state(state: &AppState, backend_name: &str, vfs: &mut MemoryVfs) {
    // `MemoryVfs::write` requires the parent directory to exist, so we
    // proactively create every directory both the state publisher and the
    // IPC request poller will touch. Without `/system/ipc`, the shell's
    // pending-VFS-request block silently fails to write skin / resolution
    // change requests, and `poll_settings_ipc` never sees them.
    let _ = vfs.mkdir("/system");
    let _ = vfs.mkdir("/system/state");
    let _ = vfs.mkdir("/system/ipc");
    let _ = vfs.write(
        oasis_app_settings::SKIN_STATE_PATH,
        state.skin.manifest.name.as_bytes(),
    );
    let res = format!(
        "{}x{}",
        state.active_theme.screen_w, state.active_theme.screen_h
    );
    let _ = vfs.write(oasis_app_settings::RESOLUTION_STATE_PATH, res.as_bytes());
    let _ = vfs.write(
        oasis_app_settings::BACKEND_STATE_PATH,
        backend_name.as_bytes(),
    );
}

/// Poll the Settings IPC paths once per frame and dispatch any pending
/// change. Clears each request immediately after reading so the shell
/// doesn't reapply on every subsequent frame.
pub fn poll_settings_ipc(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut SdlBackend,
    shader_bridge: &mut Option<SdlShaderBridge>,
    vfs: &mut MemoryVfs,
    backend_name: &str,
) {
    let mut skin_request: Option<String> = None;
    if let Ok(data) = vfs.read(oasis_app_settings::SKIN_CHANGE_REQUEST_PATH) {
        let req = String::from_utf8_lossy(&data).trim().to_string();
        // Always clear the request so we don't loop on malformed input.
        let _ = vfs.write(oasis_app_settings::SKIN_CHANGE_REQUEST_PATH, b"");
        if !req.is_empty() && req != state.skin.manifest.name {
            skin_request = Some(req);
        }
    }

    let mut resolution_request: Option<(u32, u32)> = None;
    if let Ok(data) = vfs.read(oasis_app_settings::RESOLUTION_CHANGE_REQUEST_PATH) {
        let req = String::from_utf8_lossy(&data).trim().to_string();
        let _ = vfs.write(oasis_app_settings::RESOLUTION_CHANGE_REQUEST_PATH, b"");
        if let Some((w, h)) = oasis_app_settings::parse_resolution(&req) {
            resolution_request = Some((w, h));
        } else if !req.is_empty() {
            state
                .terminal
                .output_lines
                .push(format!("Ignoring malformed resolution request: {req}"));
        }
    }

    // "Apply" from the Settings Appearance editor: an in-memory theme
    // preview. The payload is a serialized `SkinTheme` TOML document; the
    // current skin's layout/features/strings are kept and only the theme is
    // replaced. Nothing is written to disk.
    let mut theme_preview: Option<SkinTheme> = None;
    if let Ok(data) = vfs.read(oasis_app_settings::SKIN_APPLY_THEME_REQUEST_PATH) {
        let req = String::from_utf8_lossy(&data).to_string();
        let _ = vfs.write(oasis_app_settings::SKIN_APPLY_THEME_REQUEST_PATH, b"");
        if !req.trim().is_empty() {
            match SkinTheme::from_toml_str(&req) {
                Ok(theme) => theme_preview = Some(theme),
                Err(e) => {
                    state
                        .terminal
                        .output_lines
                        .push(format!("Ignoring malformed theme preview: {e}"));
                },
            }
        }
    }

    // "Save as custom skin" from the Settings Appearance editor. Payload is
    // `<name>\n<theme toml>`; the skin is written to `skins/<name>/` in the
    // standard directory format and then swapped in by name through the
    // normal resolution path (which validates the save round-trips).
    let mut save_custom: Option<(String, SkinTheme)> = None;
    if let Ok(data) = vfs.read(oasis_app_settings::SKIN_SAVE_CUSTOM_REQUEST_PATH) {
        let req = String::from_utf8_lossy(&data).to_string();
        let _ = vfs.write(oasis_app_settings::SKIN_SAVE_CUSTOM_REQUEST_PATH, b"");
        if !req.trim().is_empty() {
            match parse_save_custom_request(&req) {
                Ok(parsed) => save_custom = Some(parsed),
                Err(e) => {
                    state
                        .terminal
                        .output_lines
                        .push(format!("Ignoring malformed save-custom request: {e}"));
                },
            }
        }
    }

    let mut changed = false;
    if let Some(name) = skin_request {
        apply_skin_swap(&name, state, sdi, vfs);
        changed = true;
    }
    if let Some(theme) = theme_preview {
        let mut preview = state.skin.clone();
        preview.theme = theme;
        apply_skin_object(preview, state, sdi, vfs);
        changed = true;
    }
    if let Some((name, theme)) = save_custom {
        let mut custom = state.skin.clone();
        custom.theme = theme;
        custom.manifest.name.clone_from(&name);
        let dir = std::path::Path::new("skins").join(&name);
        match custom.save_to_directory(&dir) {
            Ok(()) => {
                state
                    .terminal
                    .output_lines
                    .push(format!("Saved custom skin to {}", dir.display()));
                // Swap by name through the normal resolution path so the
                // running session uses exactly what was written to disk.
                apply_skin_swap(&name, state, sdi, vfs);
                changed = true;
            },
            Err(e) => {
                state
                    .terminal
                    .output_lines
                    .push(format!("Failed to save custom skin: {e}"));
            },
        }
    }
    if let Some((w, h)) = resolution_request {
        apply_resolution_change(w, h, state, sdi, backend, shader_bridge, vfs);
        changed = true;
    }

    if changed {
        publish_runtime_state(state, backend_name, vfs);
    }
}

/// Parse a save-custom-skin IPC payload (`<name>\n<theme toml>`).
///
/// The name is restricted to `[A-Za-z0-9_-]` so it stays a safe directory
/// name under `skins/`.
fn parse_save_custom_request(req: &str) -> Result<(String, SkinTheme), String> {
    let (name, theme_toml) = req
        .split_once('\n')
        .ok_or_else(|| "missing name line".to_string())?;
    let name = name.trim();
    if name.is_empty() {
        return Err("empty skin name".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("invalid skin name '{name}'"));
    }
    let theme = SkinTheme::from_toml_str(theme_toml).map_err(|e| e.to_string())?;
    Ok((name.to_string(), theme))
}

/// Format a remote command result as a response string, applying side effects
/// (browser sandbox, skin swap) as needed.
pub(crate) fn format_remote_response(
    result: oasis_core::error::Result<CommandOutput>,
    browser: &mut Option<oasis_core::browser::BrowserWidget>,
    skin: &mut Skin,
    active_theme: &mut ActiveTheme,
    browser_config: &mut BrowserConfig,
    wm: &mut oasis_core::wm::manager::WindowManager,
    sdi: &mut SdiRegistry,
) -> String {
    match result {
        Ok(CommandOutput::Text(text)) => text,
        Ok(CommandOutput::Table { headers, rows }) => {
            let mut out = headers.join(" | ");
            for row in &rows {
                out.push('\n');
                out.push_str(&row.join(" | "));
            }
            out
        },
        Ok(CommandOutput::Clear) => "OK".to_string(),
        Ok(CommandOutput::None) => "OK".to_string(),
        Ok(CommandOutput::Signal(
            CommandSignal::ListenToggle { .. }
            | CommandSignal::RemoteConnect { .. }
            | CommandSignal::FtpToggle { .. }
            | CommandSignal::McpToggle { .. },
        )) => "Not available via remote.".to_string(),
        Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox { enable })) => {
            if let Some(bw) = browser {
                bw.config.features.sandbox_only = enable;
            }
            let st = if enable {
                "on (VFS only)"
            } else {
                "off (HTTP enabled)"
            };
            format!("Browser sandbox: {st}")
        },
        Ok(CommandOutput::Signal(CommandSignal::SkinSwap { name })) => {
            match resolve_skin_request(&name, skin) {
                Ok(new_skin) => {
                    let sw = active_theme.screen_w;
                    let sh = active_theme.screen_h;
                    let swapped = Skin::swap_scaled(skin, new_skin, sdi, sw, sh);
                    *active_theme = ActiveTheme::from_skin(&swapped.theme)
                        .with_screen_size(sw, sh)
                        .with_features(&swapped.features);
                    *browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                    wm.set_theme(swapped.theme.build_wm_theme());
                    let msg = format!("Switched to skin: {}", swapped.manifest.name);
                    *skin = swapped;
                    msg
                },
                Err(e) => format!("Skin error: {e}"),
            }
        },
        Ok(CommandOutput::Multi(outputs)) => {
            let mut parts = Vec::new();
            for output in outputs {
                let resp = format_remote_response(
                    Ok(output),
                    browser,
                    skin,
                    active_theme,
                    browser_config,
                    wm,
                    sdi,
                );
                if !resp.is_empty() {
                    parts.push(resp);
                }
            }
            parts.join("\n")
        },
        Err(e) => format!("error: {e}"),
    }
}

/// Poll the remote listener for incoming commands and execute them.
pub fn poll_remote_listener(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    // Destructure to allow field-level borrow splitting.
    let AppState {
        ref mut net,
        ref mut terminal,
        ref mut content,
        ref platform,
        ref mut skin,
        ref mut active_theme,
        ref mut browser_config,
        ref mut wm,
        ..
    } = *state;

    let NetworkLayer {
        ref mut listener,
        ref mut backend,
        ref tls_provider,
        ..
    } = *net;

    let TerminalLayer {
        ref mut cmd_reg,
        ref mut cwd,
        ..
    } = *terminal;

    let ContentLayer {
        ref mut browser, ..
    } = *content;

    let Some(l) = listener else { return };

    let remote_cmds = l.poll(backend);
    for (cmd_line, conn_idx) in remote_cmds {
        log::info!("Remote command from #{conn_idx}: {cmd_line}");
        let mut env = Environment {
            cwd: cwd.clone(),
            vfs,
            power: Some(platform),
            time: Some(platform),
            usb: Some(platform),
            network: None,
            tls: Some(tls_provider),
            stdin: None,
            stderr: String::new(),
        };
        let result = cmd_reg.execute(&cmd_line, &mut env);
        *cwd = env.cwd;
        let response =
            format_remote_response(result, browser, skin, active_theme, browser_config, wm, sdi);
        let _ = l.send_response(conn_idx, &response);
    }
}

/// Poll the MCP control server, dispatching agent tool calls against the UI.
///
/// Mirrors [`poll_remote_listener`]'s borrow-split idiom, additionally taking
/// the rendering `backend` so the `screenshot` tool can read the framebuffer.
#[cfg(feature = "mcp")]
pub fn poll_mcp_server(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
    backend: &mut dyn oasis_core::backend::SdiCore,
) {
    let AppState {
        ref mut mcp,
        ref mut wm,
        ref mut content,
        ref mut terminal,
        ref mut skin,
        ref mut active_theme,
        ref mut browser_config,
        ref platform,
        ref net,
        ref mut mode,
        ref plugin_manager,
        ref mut agent_activity,
        ..
    } = *state;

    let Some(server) = mcp else { return };

    let ContentLayer {
        ref mut browser,
        ref mut open_runners,
        ..
    } = *content;
    let TerminalLayer {
        ref mut cmd_reg,
        ref mut cwd,
        ..
    } = *terminal;

    let screen_w = skin.manifest.screen_width;
    let screen_h = skin.manifest.screen_height;

    let mut disp = crate::mcp_tools::AppDispatcher {
        wm,
        sdi,
        vfs,
        browser,
        open_runners,
        cmd_reg,
        cwd,
        skin,
        active_theme,
        browser_config,
        platform,
        tls_provider: &net.tls_provider,
        plugin_manager,
        mode,
        screen: backend,
        screen_w,
        screen_h,
        activity: agent_activity,
    };
    server.poll(&mut disp);
}

/// Start the MCP server from environment variables at boot (`OASIS_MCP=1`,
/// optional `OASIS_MCP_PORT` / `OASIS_MCP_TOKEN`).
#[cfg(feature = "mcp")]
pub fn mcp_start_from_env(state: &mut AppState) {
    if std::env::var("OASIS_MCP").ok().as_deref() != Some("1") {
        return;
    }
    let port = std::env::var("OASIS_MCP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(7345);
    let token = std::env::var("OASIS_MCP_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    start_mcp_server(state, port, token);
}

/// Bind a loopback listener and install the MCP server on `state`.
#[cfg(feature = "mcp")]
fn start_mcp_server(state: &mut AppState, port: u16, token: Option<String>) {
    if state.mcp.is_some() {
        state
            .terminal
            .output_lines
            .push("MCP server already running. Use 'mcp-server stop' first.".to_string());
        return;
    }
    let mut backend = oasis_core::net::StdNetworkBackend::new();
    match backend.listen_loopback(port) {
        Ok(()) => {
            let server = oasis_mcp::McpServer::new(Box::new(backend), token);
            log::info!("MCP control server listening on 127.0.0.1:{port}");
            state
                .terminal
                .output_lines
                .push(format!("MCP server listening on 127.0.0.1:{port}."));
            state.mcp = Some(server);
        },
        Err(e) => {
            log::warn!("MCP server failed to start: {e}");
            state
                .terminal
                .output_lines
                .push(format!("MCP start error: {e}"));
        },
    }
}

/// Poll the FTP server for incoming connections and commands.
pub fn poll_ftp_server(state: &mut AppState, vfs: &mut MemoryVfs) {
    let NetworkLayer {
        ref mut ftp_server,
        ref mut backend,
        ..
    } = state.net;

    let Some(server) = ftp_server else { return };

    if let Err(e) = server.poll(backend, vfs) {
        log::warn!("FTP server poll error: {e}");
    }
}

/// Poll the remote client for received data.
pub fn poll_remote_client(state: &mut AppState) {
    let Some(ref mut client) = state.net.remote_client else {
        return;
    };
    let lines = client.poll();
    for line in lines {
        state.terminal.output_lines.push(format!("[remote] {line}"));
    }
    if !client.is_connected() {
        state
            .terminal
            .output_lines
            .push("[remote] Disconnected.".to_string());
        state.net.remote_client = None;
    }
    trim_output(&mut state.terminal.output_lines);
}

/// Truncate output lines to `MAX_OUTPUT_LINES`.
pub fn trim_output(output_lines: &mut Vec<String>) {
    while output_lines.len() > terminal_sdi::MAX_OUTPUT_LINES {
        output_lines.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::terminal::{CommandOutput, CommandSignal};

    // -- trim_output --

    #[test]
    fn trim_output_noop_under_limit() {
        let mut lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "line0");
    }

    #[test]
    fn trim_output_noop_at_limit() {
        let mut lines: Vec<String> = (0..terminal_sdi::MAX_OUTPUT_LINES)
            .map(|i| format!("line{i}"))
            .collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
    }

    #[test]
    fn trim_output_trims_excess() {
        let count = terminal_sdi::MAX_OUTPUT_LINES + 50;
        let mut lines: Vec<String> = (0..count).map(|i| format!("line{i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
        // Oldest lines should have been removed.
        assert_eq!(lines[0], "line50");
        assert_eq!(lines.last().unwrap(), &format!("line{}", count - 1));
    }

    #[test]
    fn trim_output_empty() {
        let mut lines: Vec<String> = vec![];
        trim_output(&mut lines);
        assert!(lines.is_empty());
    }

    // -- process_command_output (using a real AppState) --

    fn make_test_state() -> AppState {
        use oasis_audio::RadioManager;
        use oasis_backend_sdl::SdlAudioBackend;
        use oasis_core::active_theme::ActiveTheme;
        use oasis_core::backend::Color;
        use oasis_core::bottombar::BottomBar;
        use oasis_core::browser::BrowserConfig;
        use oasis_core::config::OasisConfig;
        use oasis_core::cursor::CursorState;
        use oasis_core::dashboard::{DashboardConfig, DashboardState};
        use oasis_core::net::{RustlsTlsProvider, StdNetworkBackend};
        use oasis_core::platform::DesktopPlatform;
        use oasis_core::skin::SkinFeatures;
        use oasis_core::skin::builtin::load_builtin;
        use oasis_core::startmenu::StartMenuState;
        use oasis_core::statusbar::StatusBar;
        use oasis_core::terminal::CommandRegistry;
        use oasis_core::wm::manager::WindowManager;

        let skin = load_builtin("classic").unwrap();
        let active_theme = ActiveTheme::from_skin(&skin.theme);
        let dash_cfg = DashboardConfig::from_features(&SkinFeatures::default(), &active_theme);

        AppState {
            config: OasisConfig::default(),
            skin,
            active_theme: active_theme.clone(),
            browser_config: BrowserConfig::default(),
            platform: DesktopPlatform::new(),
            ui: UiLayer {
                dashboard: DashboardState::new(dash_cfg, vec![]),
                status_bar: StatusBar::new(),
                bottom_bar: BottomBar::new(),
                taskbar: oasis_core::taskbar::Taskbar::new(),
                start_menu: StartMenuState::new(StartMenuState::default_items(&active_theme)),
                mouse_cursor: CursorState::default(),
                desktops: oasis_core::wm::DesktopManager::new(1),
            },
            terminal: TerminalLayer {
                cmd_reg: CommandRegistry::new(),
                cwd: "/".to_string(),
                input_buf: String::new(),
                output_lines: Vec::new(),
                scroll_offset: 0,
                dirty: true,
                sync_signature: None,
                sdi_signature: None,
            },
            net: NetworkLayer {
                backend: StdNetworkBackend::new(),
                listener: None,
                ftp_server: None,
                remote_client: None,
                tls_provider: RustlsTlsProvider::new(),
            },
            content: ContentLayer {
                app_runner: None,
                open_runners: Vec::new(),
                browser: None,
                fullscreen_app: None,
            },
            osk: None,
            plugin_manager: oasis_core::plugin::PluginManager::new(),
            wm: WindowManager::new(480, 272),
            mode: crate::app_state::Mode::Dashboard,
            bg_color: Color::rgb(0, 0, 0),
            active_transition: None,
            frame_counter: 0,
            pending_wallpaper_refresh: false,
            skin_layout_textures: Vec::new(),
            image_layers: Vec::new(),
            background_layer_cache: oasis_core::vector_overlay::LayerOpsCache::new(),
            chrome_layer_cache: oasis_core::vector_overlay::LayerOpsCache::new(),
            icon_drag: None,
            cursor_texture: None,
            settings: oasis_core::settings::SettingsStore::new(),
            radio_manager: RadioManager::new(),
            radio_source: None,
            archive_catalog: None,
            pending_catalog_fetch: None,
            pending_source_fetch: None,
            audio_backend: SdlAudioBackend::new(),
            toasts: oasis_core::toast::ToastManager::new(),
            ui_sounds: oasis_core::ui_sound::UiSoundQueue::new(),
            sfx: oasis_audio::sfx::SfxPlayer::new(),
            pending_tv_catalog_fetch: None,
            tv_fetch_start: None,
            video_player: crate::video_player::VideoPlayer::new(),
            tv_audio_track: None,
            media_track: None,
            tv_audio_chunks_fed: 0,
            tv_audio_samples_fed: 0,
            #[cfg(feature = "_video")]
            pending_video_download: None,
            #[cfg(feature = "_video")]
            tv_video_cache_path: None,
            #[cfg(feature = "_video")]
            pending_video_params: None,
            #[cfg(feature = "_video")]
            tv_download_progress: None,
            #[cfg(feature = "_video")]
            tv_video_cache: Vec::new(),
            #[cfg(feature = "_video")]
            tv_stream_session: None,
            #[cfg(feature = "_video")]
            tv_current_url: None,
            #[cfg(feature = "mcp")]
            mcp: None,
            #[cfg(feature = "mcp")]
            agent_activity: crate::mcp_tools::AgentActivity::default(),
        }
    }

    #[test]
    fn process_text_output() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Text("hello\nworld".to_string())),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines, vec!["hello", "world"]);
    }

    #[test]
    fn process_table_output() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Table {
                headers: vec!["Name".into(), "Size".into()],
                rows: vec![vec!["foo.txt".into(), "42".into()]],
            }),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 2);
        assert_eq!(state.terminal.output_lines[0], "Name | Size");
        assert_eq!(state.terminal.output_lines[1], "foo.txt | 42");
    }

    #[test]
    fn process_clear_output() {
        let mut state = make_test_state();
        state.terminal.output_lines.push("existing".to_string());
        let result = process_command_output(Ok(CommandOutput::Clear), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn process_none_output() {
        let mut state = make_test_state();
        let result = process_command_output(Ok(CommandOutput::None), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn process_skin_swap_returns_name() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::SkinSwap {
                name: "modern".to_string(),
            })),
            &mut state,
        );
        assert_eq!(result, Some("modern".to_string()));
    }

    #[test]
    fn process_error_output() {
        let mut state = make_test_state();
        let err = oasis_core::error::OasisError::Command("test error".into());
        let result = process_command_output(Err(err), &mut state);
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 1);
        // Errors are wrapped in a red SGR sequence for the terminal UI.
        let line = &state.terminal.output_lines[0];
        assert!(line.starts_with("\u{1b}[31m"));
        assert!(oasis_core::ansi::strip_sgr(line).starts_with("error:"));
    }

    #[test]
    fn process_multi_output() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Text("first".to_string()),
                CommandOutput::Text("second".to_string()),
            ])),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines, vec!["first", "second"]);
    }

    #[test]
    fn process_multi_with_skin_swap() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Text("before".to_string()),
                CommandOutput::Signal(CommandSignal::SkinSwap {
                    name: "corrupted".to_string(),
                }),
            ])),
            &mut state,
        );
        assert_eq!(result, Some("corrupted".to_string()));
        assert_eq!(state.terminal.output_lines, vec!["before"]);
    }

    #[test]
    fn process_browser_sandbox_on() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox {
                enable: true,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 1);
        assert!(state.terminal.output_lines[0].contains("sandbox"));
        assert!(state.terminal.output_lines[0].contains("on"));
    }

    #[test]
    fn process_browser_sandbox_off() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::BrowserSandbox {
                enable: false,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert!(state.terminal.output_lines[0].contains("off"));
    }

    #[test]
    fn process_listen_stop_no_listener() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::ListenToggle {
                port: 0,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines[0], "No listener running.");
    }

    #[test]
    fn process_ftp_stop_no_server() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::FtpToggle {
                port: 0,
                password: None,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines[0], "No FTP server running.");
    }

    #[test]
    fn process_remote_connect_already_connected() {
        let mut state = make_test_state();
        // Simulate an existing client.
        state.net.remote_client = Some(oasis_core::net::RemoteClient::new());
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::RemoteConnect {
                address: "127.0.0.1".into(),
                port: 9999,
                psk: None,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(
            state.terminal.output_lines[0],
            "Already connected. Disconnect first."
        );
    }

    // -- Additional command handler tests --

    #[test]
    fn process_listen_already_running() {
        let mut state = make_test_state();
        // Start a listener first.
        let cfg = oasis_core::net::ListenerConfig {
            port: 19999,
            psk: String::new(),
            max_connections: 1,
            ..oasis_core::net::ListenerConfig::default()
        };
        state.net.listener = Some(oasis_core::net::RemoteListener::new(cfg));
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::ListenToggle {
                port: 8080,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert!(state.terminal.output_lines[0].contains("already running"));
    }

    #[test]
    fn process_ftp_already_running() {
        let mut state = make_test_state();
        state.net.ftp_server = Some(oasis_core::transfer::FtpServer::new(19000));
        let result = process_command_output(
            Ok(CommandOutput::Signal(CommandSignal::FtpToggle {
                port: 21,
                password: None,
            })),
            &mut state,
        );
        assert!(result.is_none());
        assert!(state.terminal.output_lines[0].contains("already running"));
    }

    #[test]
    fn process_multi_empty_list() {
        let mut state = make_test_state();
        let result = process_command_output(Ok(CommandOutput::Multi(vec![])), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn process_multi_preserves_order() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Text("alpha".to_string()),
                CommandOutput::Text("beta".to_string()),
                CommandOutput::Text("gamma".to_string()),
            ])),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 3);
        assert_eq!(state.terminal.output_lines[0], "alpha");
        assert_eq!(state.terminal.output_lines[1], "beta");
        assert_eq!(state.terminal.output_lines[2], "gamma");
    }

    #[test]
    fn process_multi_last_skin_swap_wins() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Multi(vec![
                CommandOutput::Signal(CommandSignal::SkinSwap {
                    name: "first".to_string(),
                }),
                CommandOutput::Signal(CommandSignal::SkinSwap {
                    name: "second".to_string(),
                }),
            ])),
            &mut state,
        );
        assert_eq!(result, Some("second".to_string()));
    }

    #[test]
    fn process_table_empty_rows() {
        let mut state = make_test_state();
        let result = process_command_output(
            Ok(CommandOutput::Table {
                headers: vec!["Col1".into(), "Col2".into()],
                rows: vec![],
            }),
            &mut state,
        );
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 1);
        assert_eq!(state.terminal.output_lines[0], "Col1 | Col2");
    }

    #[test]
    fn process_text_multiline() {
        let mut state = make_test_state();
        let text = "line1\nline2\nline3\nline4";
        let result = process_command_output(Ok(CommandOutput::Text(text.to_string())), &mut state);
        assert!(result.is_none());
        assert_eq!(state.terminal.output_lines.len(), 4);
    }

    #[test]
    fn process_clear_empties_all() {
        let mut state = make_test_state();
        state.terminal.output_lines = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = process_command_output(Ok(CommandOutput::Clear), &mut state);
        assert!(result.is_none());
        assert!(state.terminal.output_lines.is_empty());
    }

    #[test]
    fn trim_output_single_excess() {
        let count = terminal_sdi::MAX_OUTPUT_LINES + 1;
        let mut lines: Vec<String> = (0..count).map(|i| format!("line{i}")).collect();
        trim_output(&mut lines);
        assert_eq!(lines.len(), terminal_sdi::MAX_OUTPUT_LINES);
        assert_eq!(lines[0], "line1");
    }

    #[test]
    fn process_error_format() {
        let mut state = make_test_state();
        let err = oasis_core::error::OasisError::Vfs("file not found".into());
        process_command_output(Err(err), &mut state);
        assert!(state.terminal.output_lines[0].contains("error:"));
        assert!(state.terminal.output_lines[0].contains("file not found"));
    }
}
