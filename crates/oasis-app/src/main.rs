//! OASIS_OS desktop entry point.
//!
//! PSIX-style UI with wallpaper, mouse cursor, status bar, 6x3 icon grid
//! dashboard, and bottom bar with media category tabs.
//! L trigger cycles top tabs, R trigger cycles media categories,
//! D-pad navigates the grid. Click to select/launch icons.
//! Press F1 to toggle terminal, F2 to toggle on-screen keyboard, Escape to quit.

mod app_state;
mod boot_splash;
mod commands;
mod icon_drag;
mod input;
mod launch;
mod media_controller;
mod radio_controller;
mod render;
mod sysinfo;
mod tv_controller;
mod video_player;
use oasis_core::terminal_sdi;
mod vfs_setup;

use anyhow::Result;

use app_state::{AppState, ContentLayer, Mode, NetworkLayer, TerminalLayer, UiLayer};
use oasis_audio::RadioManager;
use oasis_backend_sdl::SdlAudioBackend;
use oasis_backend_sdl::SdlBackend;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{AudioBackend, Color, InputBackend, SdiCore};
use oasis_core::bottombar::BottomBar;
use oasis_core::browser::BrowserConfig;
use oasis_core::config::OasisConfig;
use oasis_core::cursor::CursorState;
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::net::{RustlsTlsProvider, StdNetworkBackend};
use oasis_core::platform::DesktopPlatform;
use oasis_core::platform::{PowerService, TimeService};
use oasis_core::plugin::{PluginManager, register_builtin_plugins};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::resolve_skin;
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{
    CommandRegistry, register_agent_commands, register_builtins, register_plugin_commands,
    register_tv_commands,
};
use oasis_core::toast::{ToastLevel, ToastManager};
use oasis_core::transition;
use oasis_core::vector_overlay::get_shader_layer;
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;
use oasis_core::wm::manager::WindowManager;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut config = OasisConfig::default();

    // Resolve skin from CLI arg, OASIS_SKIN env var, or config.
    let skin_name = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("OASIS_SKIN").ok())
        .unwrap_or_else(|| config.skin_path.to_string_lossy().into_owned());
    let skin = resolve_skin(&skin_name)?;
    log::info!(
        "Loaded skin: {} v{}",
        skin.manifest.name,
        skin.manifest.version
    );

    // Use the skin's screen dimensions (e.g. 1024x768 for xp, 480x272 for classic).
    config.screen_width = skin.manifest.screen_width;
    config.screen_height = skin.manifest.screen_height;

    // Desktop: scale up PSP-native skins (480x272) to a usable desktop resolution.
    if config.screen_width == 480 && config.screen_height == 272 {
        config.screen_width = 1280;
        config.screen_height = 720;
    }
    log::info!(
        "Starting OASIS_OS ({}x{})",
        config.screen_width,
        config.screen_height,
    );

    let mut backend = SdlBackend::new(
        &config.window_title,
        config.screen_width,
        config.screen_height,
    )?;
    backend.init(config.screen_width, config.screen_height)?;

    // Show a black frame immediately so the window isn't frozen during init.
    backend.clear(Color::rgb(0, 0, 0))?;
    backend.swap_buffers()?;

    // Functional boot: the splash animation runs in the foreground while
    // real init work happens between BIOS-line reveals. Each BIOS line
    // reflects a completed probe or registration step. After the last
    // line, the splash phase (3.5–6.5s) warms heavy textures (wallpaper,
    // cursor, shader bridge, SDI layout, audio) so the dashboard's first
    // frame is hitch-free.
    //
    // Skip with OASIS_SKIP_SPLASH=1 for fast development iteration —
    // the same init work still runs, just with no animation.
    let skip_splash = std::env::var("OASIS_SKIP_SPLASH").as_deref() == Ok("1");

    let mut splash: Option<boot_splash::BootSplash> = if skip_splash {
        None
    } else {
        match boot_splash::BootSplash::start(
            &mut backend,
            config.screen_width,
            config.screen_height,
        ) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("Boot splash init failed: {e}");
                None
            },
        }
    };

    // Helper: advance splash to `target_secs` (no-op if splash disabled).
    // Skipping inside the splash consumes it so subsequent calls are cheap.
    macro_rules! splash_wait {
        ($target:expr) => {{
            if let Some(sp) = splash.as_mut() {
                if let Err(e) = sp.run_until(&mut backend, $target) {
                    log::warn!("Boot splash frame failed: {e}");
                }
            }
        }};
    }
    macro_rules! splash_set_line {
        ($idx:expr, $text:expr) => {{
            if let Some(sp) = splash.as_mut() {
                sp.set_bios_line($idx, $text);
            }
        }};
    }
    macro_rules! splash_status {
        ($text:expr) => {{
            if let Some(sp) = splash.as_mut() {
                sp.set_status($text);
            }
        }};
    }

    // -- BIOS phase: lines reveal at 0.4, 0.8, 1.2, 1.6, 2.1, 2.6, 3.0s --

    // Line 0 (0.4s): kernel header — kept short so the {arch} suffix
    // fits inside the 1280px viewport even on aarch64-style long names.
    splash_set_line!(
        0,
        format!(
            "OASIS_KERNEL_V.7.0.4 BOOTING ON {}",
            sysinfo::cpu_arch().to_uppercase(),
        )
    );
    splash_status!("Powering on system bus...");
    splash_wait!(boot_splash::BIOS_REVEAL_TIMES[0]);

    // Line 1 (0.8s): host OS details — replaces the generic copyright
    // so every BIOS line carries real info.
    splash_status!("Verifying boot signature...");
    splash_set_line!(
        1,
        format!(
            "HOST KERNEL: {} | OASIS_OS V{}",
            sysinfo::os_release().to_uppercase(),
            env!("CARGO_PKG_VERSION"),
        )
    );
    splash_wait!(boot_splash::BIOS_REVEAL_TIMES[1]);

    // Line 2 (1.2s): real RAM + core count probe.
    splash_status!("Probing physical memory and CPU topology...");
    let ram_kb = sysinfo::total_ram_kb();
    let cpu_cores = sysinfo::cpu_core_count();
    splash_set_line!(
        2,
        match (ram_kb, cpu_cores) {
            (Some(kb), Some(cores)) => format!(
                "SYSTEM RAM CHECK... {}K OK ({cores} LOGICAL CORES DETECTED)",
                format_thousands(kb)
            ),
            (Some(kb), None) => format!("SYSTEM RAM CHECK... {}K OK", format_thousands(kb)),
            _ => "SYSTEM RAM CHECK... OK".to_string(),
        }
    );
    splash_wait!(boot_splash::BIOS_REVEAL_TIMES[2]);

    // Line 3 (1.6s): VFS population. Run the work now so the line can
    // report the real file count + byte total.
    splash_status!("Mounting virtual file system and seeding /etc, /apps, /home...");
    let mut vfs = MemoryVfs::new();
    vfs_setup::populate_demo_vfs(&mut vfs);
    oasis_core::terminal::populate_man_pages(&mut vfs);
    oasis_core::terminal::populate_motd(&mut vfs);
    oasis_core::terminal::populate_profile(&mut vfs);
    let disk_sample_rx = vfs_setup::spawn_disk_sample_loader();
    let (vfs_files, vfs_dirs, vfs_truncated) = sysinfo::count_vfs_entries(&vfs, "/");
    let (vfs_bytes, bytes_truncated) = sysinfo::total_vfs_bytes(&vfs, "/");
    // Append "+" to counts when the depth guard stopped the walk early —
    // makes it obvious to the user that the numbers are lower bounds
    // rather than showing a confident-but-wrong total.
    let trunc_mark = if vfs_truncated || bytes_truncated {
        "+"
    } else {
        ""
    };
    splash_set_line!(
        3,
        format!(
            "INITIALIZING VIRTUAL FILE SYSTEM... {vfs_files}{trunc_mark} FILES, \
             {vfs_dirs}{trunc_mark} DIRS, {}{trunc_mark} KB OK",
            vfs_bytes / 1024,
        )
    );
    splash_wait!(boot_splash::BIOS_REVEAL_TIMES[3]);

    // Line 4 (2.1s): skin info as the "boot drive" label.
    splash_status!(format!("Verifying skin manifest: {}", skin.manifest.name));
    splash_set_line!(
        4,
        format!(
            "MOUNTING BOOT DRIVE /DEV/HDA1... SKIN \"{}\" V{} OK ({}x{})",
            skin.manifest.name.to_uppercase(),
            skin.manifest.version,
            skin.manifest.screen_width,
            skin.manifest.screen_height,
        )
    );
    splash_wait!(boot_splash::BIOS_REVEAL_TIMES[4]);

    // Line 5 (2.6s): command + plugin registration.
    splash_status!("Registering shell commands...");
    let mut cmd_reg = CommandRegistry::new();
    register_builtins(&mut cmd_reg);
    oasis_core::script::register_script_commands(&mut cmd_reg);
    oasis_core::transfer::register_transfer_commands(&mut cmd_reg);
    oasis_core::update::register_update_commands(&mut cmd_reg);
    register_plugin_commands(&mut cmd_reg);
    register_agent_commands(&mut cmd_reg);
    register_tv_commands(&mut cmd_reg);
    oasis_core::terminal::register_browser_commands(&mut cmd_reg);

    splash_status!("Initializing plugin system...");
    let mut plugin_manager = PluginManager::new();
    register_builtin_plugins(&mut plugin_manager);
    {
        let mut plugin_sdi = SdiRegistry::new();
        plugin_manager.init_all(&mut plugin_sdi, &mut vfs, &mut cmd_reg);
    }
    let plugin_count = plugin_manager.active_count();
    let plugin_app_count = plugin_manager.plugin_apps().len();
    log::info!("Plugin system: {plugin_count} plugins active, {plugin_app_count} plugin apps");
    splash_set_line!(
        5,
        format!("LOADING FRAGMENT... {plugin_count} PLUGINS, {plugin_app_count} APPS OK")
    );
    splash_wait!(boot_splash::BIOS_REVEAL_TIMES[5]);

    // Line 6 (3.0s): display manager handoff with the real resolution.
    splash_status!("Handing off to display manager...");
    splash_set_line!(
        6,
        format!(
            "STARTING DISPLAY MANAGER @ {}x{} | BACKEND: SDL3",
            config.screen_width, config.screen_height
        )
    );
    splash_wait!(boot_splash::BIOS_REVEAL_TIMES[6]);

    // -- Splash phase (3.5–6.5s): warm heavy subsystems. The BIOS lines
    //    stay visible until 3.6s, so short init steps between 3.0 and 3.6
    //    are covered by the BIOS status line; after 3.6s the status line
    //    hides and the splash logo fades in. --

    // Attempt to initialize software shader bridge for background effects.
    splash_status!("Compiling background shader pipeline...");
    let mut shader_bridge = oasis_backend_sdl::shader_bridge::SdlShaderBridge::new(
        config.screen_width,
        config.screen_height,
    );
    if shader_bridge.is_some() {
        log::info!("Shader bridge available");
    }

    // Derive runtime theme from the active skin, applying screen dimensions.
    let active_theme = ActiveTheme::from_skin(&skin.theme)
        .with_screen_size(config.screen_width, config.screen_height)
        .with_features(&skin.features);
    let browser_config = BrowserConfig::from_skin_theme(&skin.theme);

    // Set up platform services.
    let platform = DesktopPlatform::new();

    // Warm the bitmap font glyph cache: rendering never-seen characters
    // allocates + uploads a texture on first use, which used to cause
    // per-glyph hitches on the dashboard's first few frames. Pre-rasterize
    // the common character set now so the cache is primed.
    splash_status!("Rasterizing font atlas...");
    prewarm_glyph_cache(&mut backend, &active_theme);

    splash_wait!(3.8);

    // Discover apps and merge plugin-registered apps.
    splash_status!("Indexing dashboard apps...");
    let mut apps = discover_apps(&vfs, "/apps", Some("OASISOS"))?;
    for reg in plugin_manager.plugin_apps() {
        apps.push(reg.to_app_entry());
    }
    log::info!("Dashboard: {} apps (including plugin apps)", apps.len());

    // Set up dashboard.
    let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
    let dashboard = DashboardState::new(dash_config, apps);

    // Set up PSIX-style bars.
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();

    // Window manager state (Desktop mode).
    let wm = WindowManager::with_theme(
        config.screen_width,
        config.screen_height,
        skin.theme.build_wm_theme(),
    );

    // Boot entrance: skin-selected ("fade" default, "assemble", "none").
    let fade_frames = skin.features.transition_fade_frames.unwrap_or(15);
    let active_transition = launch::make_entrance(
        &active_theme,
        fade_frames,
        config.screen_width,
        config.screen_height,
    );

    let mut mouse_cursor = CursorState::new(config.screen_width, config.screen_height);
    mouse_cursor.scale = active_theme.cursor_scale;

    let start_menu =
        StartMenuState::new_with_theme(StartMenuState::default_items(&active_theme), &active_theme);

    // Assemble application state.
    let clear_color = active_theme.clear_color;
    let mut state = AppState {
        config,
        skin,
        active_theme,
        browser_config,
        platform,
        ui: UiLayer {
            dashboard,
            status_bar: StatusBar::new(),
            bottom_bar,
            taskbar: oasis_core::taskbar::Taskbar::new(),
            start_menu,
            mouse_cursor,
            desktops: oasis_core::wm::DesktopManager::new(1),
        },
        terminal: TerminalLayer {
            cmd_reg,
            cwd: "/".to_string(),
            input_buf: String::new(),
            output_lines: vec![
                "OASIS_OS v0.1.0 -- Type 'help' for commands".to_string(),
                "F1=terminal  F2=on-screen keyboard  Escape=quit".to_string(),
                String::new(),
            ],
            scroll_offset: 0,
            dirty: true,
        },
        net: NetworkLayer {
            backend: {
                let tls = RustlsTlsProvider::new();
                StdNetworkBackend::with_tls(tls)
            },
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
        plugin_manager,
        wm,
        mode: Mode::Dashboard,
        bg_color: clear_color,
        active_transition,
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
        audio_backend: {
            let mut ab = SdlAudioBackend::new();
            ab.init().ok();
            ab
        },
        toasts: ToastManager::new(),
        pending_tv_catalog_fetch: None,
        tv_fetch_start: None,
        video_player: video_player::VideoPlayer::new(),
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
    };

    // Load persisted settings and apply per-skin icon positions (free
    // icon layout). Missing files and grid-layout skins are no-ops.
    state.settings.load(&vfs);
    icon_drag::load_icon_positions(
        &state.settings,
        &state.skin.manifest.name,
        &mut state.ui.dashboard,
    );

    // Prime the status bar with real time + power info so the first frame
    // shows accurate values instead of the "--:--" / "--%" placeholders.
    splash_status!("Polling clock and power services...");
    {
        let time = state.platform.now().ok();
        let power = state.platform.power_info().ok();
        state
            .ui
            .status_bar
            .update_info(time.as_ref(), power.as_ref());
        state.ui.bottom_bar.update_info(time.as_ref());
    }

    // Show a welcome toast.
    state.toasts.show(
        format!("Skin: {}", state.skin.manifest.name),
        ToastLevel::Info,
        state.active_theme.toast.ttl,
    );

    // Load radio stations from VFS.
    splash_status!("Parsing radio station registry...");
    state
        .radio_manager
        .load_stations(&vfs, "/etc/radio/stations.toml")
        .ok();

    // Auto-launch app via OASIS_APP env var (e.g. OASIS_APP=Browser).
    // Optionally OASIS_URL sets the initial URL for the browser.
    // OASIS_TV_CHANNEL=N auto-tunes channel N after catalog loads.
    // OASIS_TV_TIMEOUT=N auto-exits N seconds after video decode starts.
    let auto_launch_app = std::env::var("OASIS_APP").ok();
    let auto_launch_url = std::env::var("OASIS_URL").ok();
    let tv_timeout_secs: Option<u64> = std::env::var("OASIS_TV_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok());
    let mut tv_timeout_start: Option<std::time::Instant> = None;

    // Set up scene graph and apply skin layout (runs during the splash's
    // logo-entrance phase at ~4.0s so the main loop's first frame has a
    // fully-built scene).
    splash_status!("Composing SDI scene graph...");
    let mut sdi = SdiRegistry::new();
    state.skin.apply_layout_scaled(
        &mut sdi,
        state.config.screen_width,
        state.config.screen_height,
    );
    splash_wait!(4.6);

    // Generate + upload the wallpaper texture. This was previously
    // deferred to frame 0 and caused a visible hitch on first paint;
    // doing it here hides the cost under the splash animation.
    splash_status!("Generating wallpaper texture...");
    let wallpaper_tex = {
        let wp_data = wallpaper::generate_with_assets(
            state.config.screen_width,
            state.config.screen_height,
            &state.active_theme,
            &state.skin.assets,
        );
        backend.load_texture(
            state.config.screen_width,
            state.config.screen_height,
            &wp_data,
        )?
    };
    terminal_sdi::setup_wallpaper(
        &mut sdi,
        wallpaper_tex,
        state.config.screen_width,
        state.config.screen_height,
    );
    if get_shader_layer(&state.active_theme).is_some()
        && let Ok(obj) = sdi.get_mut("wallpaper")
    {
        obj.visible = false;
    }
    // Upload layout `texture =` references and image decal layers for the
    // startup skin (skin swaps rebuild these via the pending-refresh path).
    commands::refresh_skin_assets(&mut state, &mut sdi, &mut backend);
    log::info!("Wallpaper loaded");
    splash_wait!(5.2);

    // The SDL build runs as a desktop application, so the host window
    // manager already paints a hardware cursor over our window. Drawing
    // our own software cursor on top duplicates it. We skip cursor
    // texture upload + SDI registration here; `CursorState` is still
    // updated from input events so its position is available for
    // diagnostics or future re-enabling, just never rendered.

    // Apply auto-launch (after scene graph is fully set up).
    if let Some(ref app_name) = auto_launch_app {
        if let Some(app) = state
            .ui
            .dashboard
            .apps
            .iter()
            .find(|a| a.title.eq_ignore_ascii_case(app_name))
        {
            let app = app.clone();
            let result = launch::launch_app_window(
                &app,
                &mut state.wm,
                &mut sdi,
                &mut state.content.open_runners,
                &mut state.content.browser,
                &state.browser_config,
                &vfs,
                &state.net.tls_provider,
                state.skin.features.window_manager,
                &state.plugin_manager,
            );
            launch::apply_launch(result, &mut state.mode);
            log::info!("Auto-launched app: {}", app.title);

            // Navigate browser to OASIS_URL if specified.
            if let Some(ref url) = auto_launch_url
                && let Some(ref mut bw) = state.content.browser
            {
                bw.navigate_vfs(url, &vfs);
                log::info!("Auto-navigated to: {url}");
            }
        } else {
            log::warn!("OASIS_APP={app_name}: app not found in dashboard");
        }
    }

    // Publish the live runtime state (skin / resolution / backend) to VFS
    // so the Settings app and any other consumer can read the real values
    // instead of compile-time defaults. Updated again after every live
    // change in `poll_settings_ipc`.
    commands::publish_runtime_state(&state, "SDL3", &mut vfs);

    // BIOS phase ended at 3.6s; clear the status line so nothing lingers
    // under the splash-phase logo.
    splash_status!("");

    // Finish the splash: run the rest of the animation (or skip to end)
    // then fade out and release GPU textures.
    if let Some(mut sp) = splash.take() {
        if let Err(e) = sp.run_to_end(&mut backend) {
            log::warn!("Boot splash tail failed: {e}");
        }
        if let Err(e) = sp.finish(&mut backend) {
            log::warn!("Boot splash finish failed: {e}");
        }
    }

    'running: loop {
        state.frame_counter += 1;

        // Drain background disk sample loads (non-blocking).
        while let Ok((path, data)) = disk_sample_rx.try_recv() {
            let _ = vfs.write(&path, &data);
        }

        // Update system info every ~60 frames (~1s at 60fps).
        if state.frame_counter.is_multiple_of(60) {
            let time = state.platform.now().ok();
            let power = state.platform.power_info().ok();
            state
                .ui
                .status_bar
                .update_info(time.as_ref(), power.as_ref());
            state.ui.bottom_bar.update_info(time.as_ref());
        }

        let events = backend.poll_events();
        for event in &events {
            state.ui.mouse_cursor.handle_input(event);

            let result = match state.mode {
                Mode::Osk => input::handle_osk_input(event, &mut state, &mut sdi),
                Mode::Desktop => input::handle_desktop_input(event, &mut state, &mut sdi, &mut vfs),
                Mode::App => input::handle_app_input(event, &mut state, &mut sdi, &vfs),
                _ => input::handle_default_input(event, &mut state, &mut sdi, &mut vfs),
            };
            if result == input::InputResult::Quit {
                break 'running;
            }
        }
        if !events.is_empty() {
            state.terminal.dirty = true;
        }

        // Poll remote listener for incoming commands.
        commands::poll_remote_listener(&mut state, &mut sdi, &mut vfs);

        // Poll FTP server for incoming connections.
        commands::poll_ftp_server(&mut state, &mut vfs);

        // Poll remote client for received data.
        commands::poll_remote_client(&mut state);

        // Process pending VFS requests from app runners (e.g. radio tune).
        // Skip TV Guide tune requests — they're handled by the dedicated video
        // player section below.
        {
            let mut pending = None;
            if let Some(ref mut runner) = state.content.app_runner
                && !is_tv_tune_request(runner)
            {
                pending = runner.take_pending_request();
            }
            if pending.is_none() {
                for (_, runner) in &mut state.content.open_runners {
                    if is_tv_tune_request(runner) {
                        continue;
                    }
                    if let Some(req) = runner.take_pending_request() {
                        pending = Some(req);
                        break;
                    }
                }
            }
            if let Some((path, data)) = pending
                && let Err(e) = vfs.write(&path, data.as_bytes())
            {
                log::warn!("pending VFS request write failed ({path}): {e}");
            }
        }

        // Dispatch any Settings-app IPC requests (skin swap, resolution
        // change). Must run after the pending-VFS-request block above,
        // which is what writes the IPC payload into the VFS.
        commands::poll_settings_ipc(
            &mut state,
            &mut sdi,
            &mut backend,
            &mut shader_bridge,
            &mut vfs,
            "SDL3",
        );

        // Any skin swap — whether from the Settings app above or a terminal
        // `skin` command processed in the input loop — sets the pending flag
        // so the wallpaper texture is regenerated against the new theme.
        commands::refresh_wallpaper_if_pending(&mut state, &mut sdi, &mut backend);

        // Animate image decal layers (drift/pulse) by mutating their SDI
        // objects; static layers cost nothing here.
        if !state.image_layers.is_empty() {
            oasis_core::image_layers::tick_image_layers(
                &mut sdi,
                &state.image_layers,
                state.frame_counter as f32 / 60.0,
                state.active_theme.background_reduced_motion,
            );
        }

        // Tick radio, music player, and TV subsystems.
        radio_controller::tick(&mut state, &mut vfs);
        media_controller::tick(&mut state, &mut vfs);
        tv_controller::tick(&mut state, &mut backend, &mut vfs);

        // Auto-exit timer for TV streaming tests.
        if let Some(timeout) = tv_timeout_secs {
            if state.video_player.is_active() && tv_timeout_start.is_none() {
                tv_timeout_start = Some(std::time::Instant::now());
                log::info!("TV test: decode active, auto-exit in {timeout}s");
            }
            if let Some(start) = tv_timeout_start
                && start.elapsed().as_secs() >= timeout
            {
                log::info!("TV test: timeout reached, exiting");
                break 'running;
            }
        }

        // Update SDI scene graph for the active mode.
        render::update_sdi(&mut state, &mut sdi);

        // Assemble entrance: slide the bars in and hide bar content while
        // the transition runs (no-op for fade/none entrances).
        if let Some(ref trans) = state.active_transition {
            transition::apply_assemble(&mut sdi, &state.active_theme, trans);
        }

        // Drive browser image streaming (progressive loading).
        if let Some(ref mut bw) = state.content.browser {
            bw.tick(&vfs);
        }

        // -- Render --
        backend.clear(state.bg_color)?;

        // Render shader wallpaper FIRST (replaces bg_color clear).
        // This runs every frame so the animation stays live in all modes.
        if let Some(ref mut bridge) = shader_bridge
            && let Some(info) = get_shader_layer(&state.active_theme)
        {
            bridge.render_and_blit(
                &mut backend,
                &info.name,
                state.frame_counter as f32 / 60.0,
                &info.params,
            );
        }

        if state.mode == Mode::Desktop && state.wm.window_count() > 0 {
            // Vector-icon dashboards paint glyphs directly to the backend
            // (outside SDI), so we need an extra step between base SDI and
            // per-window rendering to avoid the dashboard icons disappearing
            // whenever a window is open on top of them.
            let wants_vector_icons =
                state.skin.features.dashboard && state.active_theme.icon.style == "vector";
            let dashboard = &state.ui.dashboard;
            let active_theme = &state.active_theme;
            let frame = state.frame_counter as u32;
            let overlay =
                |be: &mut dyn oasis_core::backend::SdiBackend| -> oasis_core::error::Result<()> {
                    if wants_vector_icons {
                        dashboard.render_vector_icons(be, active_theme, frame)?;
                    }
                    Ok(())
                };
            state.wm.draw_with_clips_overlay(
                &mut sdi,
                &mut backend,
                overlay,
                |window_id, cx, cy, cw, ch, be| {
                    if window_id == "browser" {
                        if let Some(ref mut bw) = state.content.browser {
                            bw.set_window(cx, cy, cw, ch);
                            bw.paint(be)
                        } else {
                            Ok(())
                        }
                    } else if let Some((_, runner)) = state
                        .content
                        .open_runners
                        .iter()
                        .find(|(id, _)| id == window_id)
                    {
                        runner.draw_windowed(cx, cy, cw, ch, be, &state.active_theme)
                    } else {
                        Ok(())
                    }
                },
            )?;
        } else if state.mode == Mode::Dashboard
            && (state.active_theme.icon.style == "vector"
                || !state.active_theme.background_layers.is_empty())
        {
            // Split draw: base layer → vector overlays/icons → overlay layer.
            // Shader already rendered above as wallpaper.
            sdi.draw_base_layer(&mut backend)?;

            oasis_core::vector_overlay::render_vector_background_cached(
                &mut backend,
                &state.active_theme,
                state.frame_counter as u32,
                &mut state.background_layer_cache,
            )?;
            state.ui.dashboard.render_vector_icons(
                &mut backend,
                &state.active_theme,
                state.frame_counter as u32,
            )?;
            sdi.draw_overlay_layer(&mut backend)?;
        } else {
            sdi.draw(&mut backend)?;
        }

        // Vector chrome layers paint on top of the SDI scene (bars, tabs,
        // windows) in every mode — procedurally shaped chrome accents.
        if !state.active_theme.chrome_layers.is_empty() {
            oasis_core::vector_overlay::render_vector_chrome(
                &mut backend,
                &state.active_theme,
                state.frame_counter as u32,
                &mut state.chrome_layer_cache,
            )?;
        }

        // Paint terminal scrollbar when in terminal mode.
        if state.mode == Mode::Terminal {
            terminal_sdi::paint_terminal_scrollbar(
                &mut backend,
                state.terminal.output_lines.len(),
                state.terminal.scroll_offset,
                &state.active_theme,
            )?;
        }

        // Draw transition overlay if active.
        if let Some(ref mut trans) = state.active_transition {
            trans.draw_overlay(&mut backend)?;
            trans.tick();
            if trans.is_done() {
                state.active_transition = None;
            }
        }

        backend.swap_buffers()?;
    }

    // Clean up video player before shutting down backend.
    state.video_player.stop(&mut backend);
    if let Some(track) = state.tv_audio_track.take() {
        let _ = state.audio_backend.unload_track(track);
    }

    // Clean up all cached video files.
    #[cfg(feature = "_video")]
    for (_, path) in &state.tv_video_cache {
        if let Err(e) = std::fs::remove_file(path) {
            log::warn!("TV: failed to remove cached file {}: {e}", path.display());
        }
    }

    backend.shutdown()?;
    log::info!("OASIS_OS shut down cleanly");
    Ok(())
}

/// Check if a runner's pending request is a TV Guide tune_url (should not be
/// consumed by the generic VFS handler).
fn is_tv_tune_request(runner: &oasis_core::apps::AppRunner) -> bool {
    runner.peek_pending_request().is_some_and(|req| {
        req.0 == oasis_core::apps::tv_guide::TV_REQUEST_PATH && req.1.starts_with("tune_url ")
    })
}

/// Format a large number with underscore thousand separators — matches the
/// retro BIOS aesthetic when reporting RAM in KB (e.g. `127_539_224`).
fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push('_');
        }
        out.push(*b as char);
    }
    out
}

/// Rasterize a common character set at common font sizes so the first
/// dashboard frame doesn't stall uploading glyph textures one-by-one.
///
/// The backend's `draw_text` lazily renders + caches each glyph; we just
/// need to call it once per (char, size) pair at a fully-transparent
/// color so nothing visibly leaks onto the current frame.
fn prewarm_glyph_cache(
    backend: &mut impl oasis_core::backend::SdiBackend,
    _theme: &oasis_core::active_theme::ActiveTheme,
) {
    // A conservative sample of the character set real UI text uses:
    // ASCII printable range + a handful of box/bullet glyphs that skins
    // and the terminal commonly draw.
    let sample: String = (0x20u8..=0x7Eu8).map(|b| b as char).collect();
    let extras = "•▪▶▼▲◄→←↑↓…";

    // Common font sizes across status/bottom bars, dashboard labels,
    // window titles, terminal text, and toast/start-menu chrome.
    let sizes: [u16; 6] = [8, 10, 12, 14, 16, 20];

    // Draw at (0, 0) with alpha=0 — backends may clip negative coordinates
    // before populating the glyph cache, which would silently no-op the
    // warm-up. Fully-transparent color keeps the pixel invisible while
    // still exercising the rasterize + upload path.
    let col = oasis_core::backend::Color::rgba(255, 255, 255, 0);
    for size in sizes {
        for ch in sample.chars().chain(extras.chars()) {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            // Errors are fine — the cache entry still populates.
            let _ = backend.draw_text(s, 0, 0, size, col);
        }
    }
}

/// Check that a host belongs to the archive.org family.
///
/// Redirect following in `connect_archive_source` honors arbitrary
/// `Location:` hosts from the server — without this guard a malicious
/// or misconfigured response could steer us at an internal address
/// (SSRF). TLS cert validation does not help here: the attacker could
/// own a perfectly valid cert for their host.
fn is_archive_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "archive.org" || h.ends_with(".archive.org")
}

/// Parse an HTTP/HTTPS stream URL into (host, port, path, use_tls).
fn parse_stream_url(url: &str) -> Option<(String, u16, String, bool)> {
    let (remainder, tls) = if let Some(r) = url.strip_prefix("https://") {
        (r, true)
    } else if let Some(r) = url.strip_prefix("http://") {
        (r, false)
    } else {
        return None;
    };
    let (host_port, path) = if let Some(idx) = remainder.find('/') {
        (&remainder[..idx], remainder[idx..].to_string())
    } else {
        (remainder, "/".to_string())
    };
    let default_port = if tls { 443 } else { 80 };
    let (host, port) = if let Some(idx) = host_port.rfind(':') {
        let port: u16 = host_port[idx + 1..].parse().ok()?;
        (host_port[..idx].to_string(), port)
    } else {
        (host_port.to_string(), default_port)
    };
    Some((host, port, path, tls))
}

/// Perform a blocking HTTPS GET and return the response body as a string.
///
/// Used for Internet Archive API calls (small JSON responses).
fn https_get_body(
    net_backend: &mut oasis_core::net::StdNetworkBackend,
    tls_provider: &oasis_core::net::RustlsTlsProvider,
    host: &str,
    path: &str,
) -> std::result::Result<String, String> {
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    log::debug!("HTTPS GET https://{host}{path}");

    let tcp = net_backend
        .connect(host, 443)
        .map_err(|e| format!("connect: {e}"))?;

    log::debug!("HTTPS: TCP connected to {host}:443");

    // This is a minimal HTTP/1.1 blocking client. The shared TLS config
    // advertises both `h2` and `http/1.1` so the browser can negotiate
    // HTTP/2 with CDNs that require it; if we used the default `connect_tls`
    // here, archive.org would select `h2` and our `\r\n\r\n` parser would
    // trip on HTTP/2 frames. Force `http/1.1` only.
    let tls_conn = tls_provider
        .connect_tls_with_alpn(tcp, host, &[b"http/1.1"])
        .map_err(|e| format!("TLS: {e}"))?;
    let mut stream = tls_conn.stream;

    log::debug!("HTTPS: TLS handshake complete (alpn={:?})", tls_conn.alpn);

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: OASIS_OS/0.1\r\n\
         Connection: close\r\nAccept: */*\r\n\r\n"
    );
    let req_bytes = request.as_bytes();
    let mut written = 0;
    while written < req_bytes.len() {
        match stream.write(&req_bytes[written..]) {
            Ok(n) => written += n,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                return Err(format!("write: {e}"));
            },
        }
    }

    // Read full response (runs on background thread; may spin on WouldBlock).
    let mut buf = [0u8; 8192];
    let mut response = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            return Err("timeout reading HTTP response".to_string());
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                if !response.is_empty() {
                    break;
                }
                return Err(format!("read: {e}"));
            },
        }
    }

    log::debug!("HTTPS: received {} bytes from {host}{path}", response.len());

    // Split headers from body on raw bytes to avoid UTF-8 lossy offset issues.
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "no header/body separator in response".to_string())?;
    let header_bytes = &response[..header_end];
    let header_text = String::from_utf8_lossy(header_bytes);

    // Parse status code from first line.
    if let Some(first_line) = header_text.lines().next()
        && let Some(code_str) = first_line.split_whitespace().nth(1)
        && let Ok(code) = code_str.parse::<u16>()
        && code >= 400
    {
        return Err(format!("HTTP {code}"));
    }

    let body_bytes = &response[header_end + 4..];

    // Decode chunked transfer encoding if present.
    let is_chunked = header_text.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("transfer-encoding:")
            && l.to_ascii_lowercase().contains("chunked")
    });
    let final_body = if is_chunked {
        decode_chunked(body_bytes)
    } else {
        body_bytes.to_vec()
    };
    Ok(String::from_utf8_lossy(&final_body).into_owned())
}

/// Decode HTTP chunked transfer encoding on raw bytes.
///
/// Format: `<hex-size>\r\n<data>\r\n` repeated, terminated by `0\r\n\r\n`.
fn decode_chunked(input: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut pos = 0;
    loop {
        // Skip optional leading \r\n.
        while pos < input.len() && (input[pos] == b'\r' || input[pos] == b'\n') {
            pos += 1;
        }
        if pos >= input.len() {
            break;
        }
        // Read chunk size (hex).
        let size_start = pos;
        while pos < input.len() && input[pos] != b'\r' && input[pos] != b'\n' {
            pos += 1;
        }
        let size_str = std::str::from_utf8(&input[size_start..pos]).unwrap_or("");
        // Chunk size may include extensions after `;` — strip them.
        let hex = size_str.split(';').next().unwrap_or("").trim();
        let chunk_size = match usize::from_str_radix(hex, 16) {
            Ok(0) => break, // Final chunk.
            Ok(n) => n,
            Err(_) => break, // Malformed — return what we have.
        };
        // Skip \r\n after size line.
        if pos < input.len() && input[pos] == b'\r' {
            pos += 1;
        }
        if pos < input.len() && input[pos] == b'\n' {
            pos += 1;
        }
        // Extract chunk data.
        let end = (pos + chunk_size).min(input.len());
        result.extend_from_slice(&input[pos..end]);
        pos = end;
    }
    result
}

/// Connect to archive.org over TLS and create an ArchiveSource for the given track.
///
/// Follows up to 3 HTTP redirects per attempt (archive.org returns a 302 to
/// a CDN node like `dn720703.ca.archive.org`). If the CDN responds with a
/// transient 5xx (common — archive.org's CDN intermittently returns 500 on
/// valid files), we retry from archive.org itself: a fresh 302 is typically
/// routed to a different CDN node, which usually succeeds. Polls the source
/// until response headers are parsed before returning.
fn connect_archive_source(
    tls_provider: &oasis_core::net::RustlsTlsProvider,
    track: &oasis_audio::radio::ArchiveTrack,
) -> std::result::Result<Box<dyn oasis_audio::radio::RadioSource + Send>, String> {
    use oasis_audio::radio::RadioSource;
    use oasis_audio::radio::source::SourceState;
    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;

    let orig_host = "archive.org".to_string();
    let orig_path = oasis_audio::radio::ArchiveCatalog::download_path(track);
    let title = track.title.clone();
    let creator = track.creator.clone();

    log::info!("Connecting to archive source: {orig_host}{orig_path}");

    const CDN_RETRIES: usize = 3;
    let mut last_err = String::new();

    'attempt: for attempt in 0..CDN_RETRIES {
        let mut host = orig_host.clone();
        let mut path = orig_path.clone();
        if attempt > 0 {
            log::warn!(
                "Retrying archive source (attempt {}/{CDN_RETRIES}) after: {last_err}",
                attempt + 1,
            );
            // Brief pause before retry so we don't hammer a struggling CDN.
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        'redirect: for _redirect_num in 0..3 {
            // SSRF guard: only follow redirects that stay within the
            // archive.org domain. The initial `orig_host` is hard-coded,
            // but CDN redirects are honoured verbatim, so a compromised
            // or misbehaving response must not be able to steer us at
            // an internal address.
            if !is_archive_host(&host) {
                last_err = format!("redirect to non-archive host rejected: {host}");
                continue 'attempt;
            }
            let mut net_backend = StdNetworkBackend::new();
            let tcp = match net_backend.connect(&host, 443) {
                Ok(t) => t,
                Err(e) => {
                    // TCP failure on a CDN node is transient — fall back
                    // to `'attempt` so archive.org can re-route us.
                    last_err = format!("connect: {e}");
                    continue 'attempt;
                },
            };
            // Force HTTP/1.1 ALPN: ArchiveSource speaks HTTP/1.1 and the shared
            // TLS config also offers `h2` for the browser, so without this the
            // server may hand us an h2 stream that the source can't parse.
            let stream = match tls_provider.connect_tls_with_alpn(tcp, &host, &[b"http/1.1"]) {
                Ok(s) => s.stream,
                Err(e) => {
                    last_err = format!("TLS: {e}");
                    continue 'attempt;
                },
            };

            let mut source =
                oasis_audio::radio::ArchiveSource::new(stream, &host, &path, &title, &creator);

            // Poll until headers are fully parsed (data arrives, or error/redirect).
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if std::time::Instant::now() > deadline {
                    last_err = "timeout waiting for response headers".into();
                    continue 'attempt;
                }
                match source.poll() {
                    Ok(Some(chunk)) => {
                        source.push_back_chunk(chunk);
                        log::info!("Archive source connected, audio data flowing");
                        return Ok(Box::new(source));
                    },
                    Ok(None) => match source.state() {
                        SourceState::Ended => {
                            last_err = "connection closed before headers".into();
                            continue 'attempt;
                        },
                        SourceState::Error => {
                            last_err = "source error during header parsing".into();
                            continue 'attempt;
                        },
                        _ => {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        },
                    },
                    Err(e) => {
                        let msg = format!("{e}");
                        // OasisError::Backend("redirect:...") formats as
                        // "backend error: redirect:..." — strip the prefix.
                        let inner = msg.strip_prefix("backend error: ").unwrap_or(&msg);
                        if let Some(url) = inner.strip_prefix("redirect:") {
                            if let Some((new_host, _, new_path, _)) = parse_stream_url(url) {
                                host = new_host;
                                path = new_path;
                                continue 'redirect;
                            }
                            last_err = format!("bad redirect URL: {url}");
                            continue 'attempt;
                        }
                        // HTTP 4xx is the caller's problem (bad URL, auth,
                        // etc.) — don't retry those. Everything else is
                        // potentially transient: HTTP 5xx from a flaky
                        // CDN node, TLS alerts, TCP RSTs, or connection
                        // resets mid-header-parse all deserve a re-roll
                        // through archive.org to pick up a different CDN
                        // assignment.
                        if inner.starts_with("HTTP 4") {
                            return Err(msg);
                        }
                        last_err = msg;
                        continue 'attempt;
                    },
                }
            }
        }

        last_err = "too many redirects".into();
    }

    Err(format!(
        "archive CDN unreachable after {CDN_RETRIES} attempts: {last_err}"
    ))
}

/// Fetch catalog and connect to first track on a background thread.
///
/// The identifier may be either an Internet Archive collection (in which case
/// we run an `advancedsearch.php` query for audio items under it) or a single
/// item holding many MP3 files (e.g. `OTRR_This_Is_Your_FBI_Singles`). If the
/// collection search returns no items we fall back to treating the identifier
/// as an item id and pull files from `/metadata/<id>/files` directly.
///
/// Creates its own `StdNetworkBackend` (cheap) so no shared state is needed.
fn fetch_catalog_blocking(
    collection: &str,
    seed: u64,
    tls: &oasis_core::net::RustlsTlsProvider,
) -> std::result::Result<app_state::CatalogFetchResult, String> {
    let mut net = StdNetworkBackend::new();

    log::info!("Fetching catalog for '{collection}'");

    let search_path = format!(
        "/advancedsearch.php?\
         q=collection:{collection}+AND+mediatype:audio\
         &fl=identifier,title,creator\
         &sort=random&rows=50&output=json"
    );
    let body = https_get_body(&mut net, tls, "archive.org", &search_path)
        .map_err(|e| format!("search API: {e}"))?;

    let items = oasis_audio::radio::ArchiveCatalog::parse_search_response(&body);
    log::info!("Search returned {} items", items.len());

    let mut catalog = oasis_audio::radio::ArchiveCatalog::new(collection);

    // Collection path: fetch files for up to 5 items returned by search.
    for (item_id, _title, creator) in items.iter().take(5) {
        let fp = oasis_audio::radio::ArchiveCatalog::files_api_path(item_id);
        match https_get_body(&mut net, tls, "archive.org", &fp) {
            Ok(fb) => {
                let tracks =
                    oasis_audio::radio::ArchiveCatalog::parse_files_response(&fb, item_id, creator);
                log::info!("Item '{item_id}': {} MP3 tracks", tracks.len());
                catalog.tracks.extend(tracks);
            },
            Err(e) => {
                log::warn!("Files API for '{item_id}': {e}");
            },
        }
    }

    // Single-item fallback: treat `collection` itself as an IA item id. This
    // is what makes stations like "This Is Your FBI" (an item, not a
    // collection) work.
    if catalog.tracks.is_empty() {
        log::info!("Collection search empty — trying '{collection}' as a single item");
        let fp = oasis_audio::radio::ArchiveCatalog::files_api_path(collection);
        match https_get_body(&mut net, tls, "archive.org", &fp) {
            Ok(fb) => {
                let tracks = oasis_audio::radio::ArchiveCatalog::parse_files_response(
                    &fb, collection, "Unknown",
                );
                log::info!("Item '{collection}': {} MP3 tracks", tracks.len());
                catalog.tracks.extend(tracks);
            },
            Err(e) => {
                log::warn!("Files API for '{collection}': {e}");
            },
        }
    }

    if catalog.tracks.is_empty() {
        return Err("no MP3 files found".to_string());
    }

    catalog.shuffle(seed);

    // Try up to CATALOG_TRACK_RETRIES tracks in the shuffled catalog — if
    // one item's files are on a flaky CDN node, the next item is likely on
    // a different one. Without this, a single bad shuffle (or a file that
    // was de-listed) fails the whole station.
    const CATALOG_TRACK_RETRIES: usize = 5;
    let mut last_err = String::new();
    for _ in 0..CATALOG_TRACK_RETRIES.min(catalog.tracks.len()) {
        let Some(track) = catalog.current_track().cloned() else {
            break;
        };
        match connect_archive_source(tls, &track) {
            Ok(source) => return Ok(app_state::CatalogFetchResult { catalog, source }),
            Err(e) => {
                log::warn!("Track '{}' unreachable: {e}; trying next", track.filename);
                last_err = e;
                catalog.next_track();
            },
        }
    }
    Err(format!("no playable tracks in catalog: {last_err}"))
}

/// Fetch video catalogs for all TV channels on a background thread.
fn fetch_tv_catalogs_blocking(
    channels: &[oasis_core::apps::tv_guide::Channel],
    tls: &oasis_core::net::RustlsTlsProvider,
) -> std::result::Result<Vec<Option<oasis_core::apps::tv_guide::ChannelCatalog>>, String> {
    use oasis_core::apps::tv_guide::catalog::ChannelCatalog;

    log::info!("TV fetch_tv_catalogs_blocking: {} channels", channels.len());

    let mut net = oasis_core::net::StdNetworkBackend::new();
    let mut results = Vec::new();

    for channel in channels {
        log::debug!(
            "TV: fetching CH {} '{}' ({} sources)",
            channel.number,
            channel.call_sign,
            channel.source.len(),
        );
        let mut catalog = ChannelCatalog::new(channel.number);

        for source in &channel.source {
            let files_path = ChannelCatalog::files_api_path(&source.item_id);
            match https_get_body(&mut net, tls, "archive.org", &files_path) {
                Ok(body) => {
                    log::debug!(
                        "TV: source '{}' response: {} bytes",
                        source.item_id,
                        body.len(),
                    );
                    let episodes = ChannelCatalog::parse_files_response(
                        &body,
                        &source.item_id,
                        source.subfolder.as_deref(),
                    );
                    log::info!(
                        "TV item '{}': {} video episodes",
                        source.item_id,
                        episodes.len(),
                    );
                    catalog.add_episodes(episodes);
                },
                Err(e) => {
                    log::warn!("TV files API for '{}': {e}", source.item_id);
                },
            }
        }

        if catalog.episodes.is_empty() {
            log::debug!("TV: CH {} has no episodes", channel.number);
            results.push(None);
        } else {
            log::debug!(
                "TV: CH {} loaded {} episodes ({:.0}s total)",
                channel.number,
                catalog.episodes.len(),
                catalog.total_duration_secs,
            );
            results.push(Some(catalog));
        }
    }

    let loaded = results.iter().filter(|c| c.is_some()).count();
    log::info!(
        "TV fetch_tv_catalogs_blocking done: {loaded}/{} channels loaded",
        results.len(),
    );

    Ok(results)
}

/// Connect to a single archive track on a background thread.
fn connect_archive_track_sync(
    tls: &oasis_core::net::RustlsTlsProvider,
    track: &oasis_audio::radio::ArchiveTrack,
) -> std::result::Result<app_state::TrackFetchResult, String> {
    let source = connect_archive_source(tls, track)?;
    Ok(app_state::TrackFetchResult { source })
}
