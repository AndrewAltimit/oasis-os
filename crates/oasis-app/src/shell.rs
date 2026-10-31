//! The desktop shell: boot sequence, per-frame step and render.
//!
//! This is the body of the desktop binary's main loop, factored out of
//! `main()` so it can run against any [`ShellBackend`]:
//!
//! - `main.rs` drives it with the SDL window, the SDL audio device and the
//!   animated boot splash ([`BootObserver`]).
//! - [`crate::harness`] drives it headlessly (software framebuffer,
//!   recording audio fake, virtual clock) for end-to-end scenarios.
//!
//! One iteration of the desktop loop is:
//!
//! ```text
//! let events = backend.poll_events();
//! if shell.step(&events, Instant::now()).quit { break }
//! if redraw { shell.render(Instant::now())? } else { idle sleep }
//! ```
//!
//! Everything time-dependent inside a frame takes the `now` passed in,
//! so a caller with a virtual clock gets fully deterministic frames.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::settings::{SettingsStore, UserPrefs};
use oasis_backend_sdl::shader_bridge::SdlShaderBridge;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::SdiBackend;
use oasis_core::bottombar::BottomBar;
use oasis_core::browser::BrowserConfig;
use oasis_core::config::OasisConfig;
use oasis_core::cursor::CursorState;
use oasis_core::dashboard::{AppEntry, DashboardConfig, DashboardState};
use oasis_core::input::{InputEvent, KeyTwinFilter};
use oasis_core::platform::{DesktopPlatform, NetworkService, PowerService, TimeService};
use oasis_core::plugin::{Plugin, PluginContext, PluginManager, register_builtin_plugins};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::{Skin, resolve_skin, resolve_skin_or_classic};
use oasis_core::startmenu::StartMenuState;
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{
    CommandRegistry, register_agent_commands, register_builtins, register_plugin_commands,
};
use oasis_core::terminal_sdi;
use oasis_core::toast::{ToastLevel, ToastManager};
use oasis_core::transition;
use oasis_core::vector_overlay::get_shader_layer;
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;
use oasis_core::wm::manager::WindowManager;

use crate::app_state::{
    AppState, ContentLayer, Mode, NetworkLayer, RadioLayer, SkinAssets, TerminalLayer, TvLayer,
    UiLayer,
};
use crate::audio_out::ShellAudio;
use crate::launch::{BROWSER_WIN, LaunchOpts};
use crate::shell_backend::ShellBackend;
use crate::{
    commands, frame_stats, icon_drag, input, launch, media_controller, radio_controller, render,
    sysinfo, terminal_input, tv_controller, ui_ipc, ui_sfx, user_prefs, vfs_setup,
};

/// How often the status bar clock / battery / Wi-Fi, the System Monitor
/// probe and the settings disk mirror refresh.
const SYSINFO_INTERVAL: Duration = Duration::from_secs(1);

/// After an input event, keep redrawing unconditionally for this long —
/// covers hover states, drags, key repeat, and any UI that reacts a few
/// frames later than the event itself.
pub const INPUT_REDRAW_GRACE: Duration = Duration::from_millis(250);

/// Upper bound on how long the shell may go without a real present.
/// Safety net for idle frame elision: any redraw condition missed above
/// (e.g. a window expose the event loop didn't surface) self-heals
/// within this window.
pub const REDRAW_HEARTBEAT: Duration = Duration::from_secs(1);

/// Everything [`Shell::boot`] needs that is decided before the backend
/// exists: resolved skin + screen size, persisted preferences, and the
/// host-integration switches the headless harness turns off.
pub struct BootOptions {
    /// Shell config with `screen_width` / `screen_height` already
    /// resolved (see [`resolve_screen_size`]).
    pub config: OasisConfig,
    /// The startup skin (features already patched with `prefs`).
    pub skin: Skin,
    /// Persisted user preferences.
    pub prefs: UserPrefs,
    /// The persisted settings store (seeds `/system/settings.toml`).
    pub boot_settings: SettingsStore,
    /// Real-storage settings file mirrored from the VFS. `None` disables
    /// persistence.
    pub settings_disk_path: Option<PathBuf>,
    /// Never start network fetches (see [`AppState::offline`]).
    pub offline: bool,
    /// Where Settings saves custom skins (see [`AppState::custom_skin_root`]).
    pub custom_skin_root: PathBuf,
    /// Load the sample media from the host disk in the background.
    pub load_disk_samples: bool,
    /// Create the software shader-wallpaper bridge.
    pub shader_wallpaper: bool,
    /// Freeze the platform clock (status bar, clock widgets).
    pub fixed_time: Option<oasis_core::platform::SystemTime>,
    /// Launch this dashboard app after boot (`OASIS_APP`).
    pub auto_launch_app: Option<String>,
    /// Navigate the auto-launched browser here (`OASIS_URL`).
    pub auto_launch_url: Option<String>,
    /// Quit this many seconds after TV video decode starts
    /// (`OASIS_TV_TIMEOUT`).
    pub tv_timeout_secs: Option<u64>,
    /// Plugins registered next to the built-in ones (embedders, tests);
    /// initialized with them at boot, so the apps they register get
    /// dashboard tiles.
    pub plugins: Vec<Box<dyn Plugin>>,
}

impl BootOptions {
    /// Resolve the options the way the desktop binary always has: skin
    /// from the first CLI argument, `OASIS_SKIN`, the persisted preference
    /// or the config default; preferences from real storage; auto-launch
    /// from `OASIS_APP` / `OASIS_URL` / `OASIS_TV_TIMEOUT`.
    ///
    /// Also applies the persisted UI locale (process-global).
    pub fn from_env() -> Result<Self> {
        let mut config = OasisConfig::default();

        // Persisted user preferences (skin, resolution, volume, locale,
        // accessibility) from real storage; see `user_prefs`.
        let settings_disk_path = user_prefs::disk_path();
        let boot_settings = user_prefs::load_from_disk(settings_disk_path.as_ref());
        let prefs = UserPrefs::from_store(&boot_settings);
        oasis_core::i18n::set_ui_locale(prefs.locale());

        // Resolve skin from CLI arg, OASIS_SKIN env var, the persisted
        // preference, or config.
        let default_skin = config.skin_path.to_string_lossy().into_owned();
        let explicit_skin = std::env::args()
            .nth(1)
            .or_else(|| std::env::var("OASIS_SKIN").ok());
        // An unknown or broken skin never blocks boot: fall back to classic
        // (logged), so a typo or a stale preference still shows a shell.
        let mut skin = match (explicit_skin, prefs.skin.as_deref()) {
            (Some(name), _) => resolve_skin_or_classic(&name)?,
            (None, Some(saved)) => resolve_skin(saved).or_else(|e| {
                log::warn!("Persisted skin '{saved}' failed to load ({e}); using default");
                resolve_skin_or_classic(&default_skin)
            })?,
            (None, None) => resolve_skin_or_classic(&default_skin)?,
        };
        prefs.patch_features(&mut skin.features);
        log::info!(
            "Loaded skin: {} v{}",
            skin.manifest.name,
            skin.manifest.version
        );
        resolve_screen_size(&mut config, &skin, &prefs);

        Ok(Self {
            config,
            skin,
            prefs,
            boot_settings,
            settings_disk_path,
            offline: false,
            custom_skin_root: PathBuf::from("skins"),
            load_disk_samples: true,
            shader_wallpaper: true,
            fixed_time: None,
            auto_launch_app: std::env::var("OASIS_APP").ok(),
            auto_launch_url: std::env::var("OASIS_URL").ok(),
            tv_timeout_secs: std::env::var("OASIS_TV_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok()),
            plugins: Vec::new(),
        })
    }

    /// Hermetic options for skin `name`: default preferences, no disk
    /// persistence, no disk samples, offline, no auto-launch, no shader
    /// wallpaper. Environment variables are ignored.
    pub fn hermetic(name: &str) -> Result<Self> {
        let mut config = OasisConfig::default();
        let prefs = UserPrefs::default();
        let mut skin = resolve_skin(name)?;
        prefs.patch_features(&mut skin.features);
        resolve_screen_size(&mut config, &skin, &prefs);
        Ok(Self {
            config,
            skin,
            prefs,
            boot_settings: SettingsStore::new(),
            settings_disk_path: None,
            offline: true,
            custom_skin_root: hermetic_skin_root(),
            load_disk_samples: false,
            shader_wallpaper: false,
            fixed_time: None,
            auto_launch_app: None,
            auto_launch_url: None,
            tv_timeout_secs: None,
            plugins: Vec::new(),
        })
    }
}

/// A fresh per-boot temp directory for custom skins saved during a
/// hermetic (harness) session, so tests never write into the repository.
fn hermetic_skin_root() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "oasis-hermetic-skins-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Whether the active skin's corrupted modifiers should distort this frame
/// (the skin enables `corrupted`, intensity > 0, and the user has not asked
/// for reduced motion).
fn corruption_active(state: &AppState) -> bool {
    !state.active_theme.ui_theme.reduced_motion
        && state
            .skin
            .corrupted_modifiers
            .as_ref()
            .is_some_and(|m| m.is_active())
}

/// Apply the skin's screen size to `config`: the skin's native size,
/// PSP-native (480x272) skins scaled to 1280x720 unless they pin
/// `desktop_width` / `desktop_height`, and a resolution picked in
/// Settings wins over both.
pub fn resolve_screen_size(config: &mut OasisConfig, skin: &Skin, prefs: &UserPrefs) {
    config.screen_width = skin.manifest.screen_width;
    config.screen_height = skin.manifest.screen_height;
    if let (Some(dw), Some(dh)) = (skin.manifest.desktop_width, skin.manifest.desktop_height) {
        config.screen_width = dw.max(1);
        config.screen_height = dh.max(1);
    } else if config.screen_width == 480 && config.screen_height == 272 {
        config.screen_width = 1280;
        config.screen_height = 720;
    }
    if let Some((w, h)) = prefs.resolution {
        config.screen_width = w;
        config.screen_height = h;
    }
}

/// Receives boot progress so a host can animate a splash screen while the
/// real init work runs. All methods default to no-ops.
pub trait BootObserver<B> {
    /// A new "currently loading" status line.
    fn status(&mut self, _text: &str) {}
    /// BIOS line `idx` (0..7) now reports `text`.
    fn bios_line(&mut self, _idx: usize, _text: String) {}
    /// Keep animating until the splash clock reaches `secs`.
    fn wait_until(&mut self, _backend: &mut B, _secs: f32) {}
}

/// A [`BootObserver`] that ignores all progress (no splash).
pub struct NoSplash;

impl<B> BootObserver<B> for NoSplash {}

/// Outcome of one [`Shell::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    /// The user asked to quit (window close, Escape on the dashboard,
    /// start-menu Exit) or the TV test timeout fired.
    pub quit: bool,
    /// Something on screen may have changed: call [`Shell::render`].
    /// `false` means the frame can be elided entirely.
    pub redraw: bool,
    /// The SDI scene content differs from the last frame's (a subset of
    /// `redraw`; lets a driver detect that the UI has settled).
    pub scene_changed: bool,
}

/// The running desktop shell (see module docs).
pub struct Shell<B: ShellBackend> {
    pub state: AppState,
    pub sdi: SdiRegistry,
    pub vfs: MemoryVfs,
    pub backend: B,
    pub shader_bridge: Option<SdlShaderBridge>,
    key_filter: KeyTwinFilter,
    sysmon_probe: oasis_apps::system_monitor::probe::HostProbe,
    settings_mirror: user_prefs::DiskMirror,
    disk_sample_rx: Option<mpsc::Receiver<(String, Vec<u8>)>>,
    frame_stats: Option<frame_stats::FrameStats>,
    /// Last confirmed SDI scene content signature (None = never hashed).
    last_scene_sig: Option<u64>,
    last_input_at: Instant,
    last_present_at: Instant,
    /// Clock for `App::tick` deltas (independent of drawn frames).
    last_app_tick_at: Instant,
    /// When the status bar / System Monitor info last refreshed
    /// ([`SYSINFO_INTERVAL`]).
    last_sysinfo_at: Instant,
    /// Runs the `cron` jobs, checked with the sysinfo refresh.
    cron: oasis_core::script::CronScheduler,
    /// Zero point of the animation clock (shader wallpapers, image decal
    /// layers), so they run at wall-clock speed at any frame rate.
    clock_origin: Instant,
    tv_timeout_secs: Option<u64>,
    tv_timeout_start: Option<Instant>,
    /// VFS path the next presented frame is saved to (`screenshot`).
    pending_screenshot: Option<String>,
    /// Mode the pointer drags (icon drag, WM drag) were started in: any
    /// mode change ends them (see [`input::end_drags_on_mode_change`]).
    drag_mode: Mode,
    #[cfg(feature = "skin-dev")]
    skin_watcher: crate::hot_reload::SkinWatcher,
}

macro_rules! bios {
    ($obs:expr, $idx:expr, $text:expr) => {
        $obs.bios_line($idx, $text)
    };
}

impl<B: ShellBackend> Shell<B> {
    /// Run the boot sequence on `backend` and return the ready shell.
    ///
    /// Progress is reported to `observer` in the same order and at the
    /// same splash times the desktop binary always used; `make_audio`
    /// opens the audio output at the point in the sequence the SDL device
    /// used to be opened.
    pub fn boot(
        opts: BootOptions,
        mut backend: B,
        make_audio: impl FnOnce() -> Box<dyn ShellAudio>,
        observer: &mut dyn BootObserver<B>,
    ) -> Result<Self> {
        let BootOptions {
            config,
            skin,
            prefs,
            boot_settings,
            settings_disk_path,
            offline,
            custom_skin_root,
            load_disk_samples,
            shader_wallpaper,
            fixed_time,
            auto_launch_app,
            auto_launch_url,
            tv_timeout_secs,
            plugins,
        } = opts;
        log::info!(
            "Starting OASIS_OS ({}x{})",
            config.screen_width,
            config.screen_height,
        );

        // -- BIOS phase: lines reveal at 0.4, 0.8, 1.2, 1.6, 2.1, 2.6, 3.0s --
        use crate::boot_splash::BIOS_REVEAL_TIMES;

        // Line 0 (0.4s): kernel header — kept short so the {arch} suffix
        // fits inside the 1280px viewport even on aarch64-style long names.
        bios!(
            observer,
            0,
            format!(
                "OASIS_KERNEL_V.7.0.4 BOOTING ON {}",
                sysinfo::cpu_arch().to_uppercase(),
            )
        );
        observer.status("Powering on system bus...");
        observer.wait_until(&mut backend, BIOS_REVEAL_TIMES[0]);

        // Line 1 (0.8s): host OS details — replaces the generic copyright
        // so every BIOS line carries real info.
        observer.status("Verifying boot signature...");
        bios!(
            observer,
            1,
            format!(
                "HOST KERNEL: {} | OASIS_OS V{}",
                sysinfo::os_release().to_uppercase(),
                env!("CARGO_PKG_VERSION"),
            )
        );
        observer.wait_until(&mut backend, BIOS_REVEAL_TIMES[1]);

        // Line 2 (1.2s): real RAM + core count probe.
        observer.status("Probing physical memory and CPU topology...");
        bios!(observer, 2, ram_bios_line());
        observer.wait_until(&mut backend, BIOS_REVEAL_TIMES[2]);

        // Line 3 (1.6s): VFS population. Run the work now so the line can
        // report the real file count + byte total.
        observer.status("Mounting virtual file system and seeding /etc, /apps, /home...");
        let mut vfs = seed_vfs(&boot_settings);
        let disk_sample_rx = load_disk_samples.then(vfs_setup::spawn_disk_sample_loader);
        bios!(observer, 3, vfs_bios_line(&vfs));
        observer.wait_until(&mut backend, BIOS_REVEAL_TIMES[3]);

        // Line 4 (2.1s): skin info as the "boot drive" label.
        observer.status(&format!("Verifying skin manifest: {}", skin.manifest.name));
        bios!(
            observer,
            4,
            format!(
                "MOUNTING BOOT DRIVE /DEV/HDA1... SKIN \"{}\" V{} OK ({}x{})",
                skin.manifest.name.to_uppercase(),
                skin.manifest.version,
                skin.manifest.screen_width,
                skin.manifest.screen_height,
            )
        );
        observer.wait_until(&mut backend, BIOS_REVEAL_TIMES[4]);

        // Line 5 (2.6s): command + plugin registration.
        observer.status("Registering shell commands...");
        let mut cmd_reg = shell_commands();

        observer.status("Initializing plugin system...");
        // The real scene graph, created now so plugins draw into it (their
        // `init` runs before the skin layout is applied below).
        let mut sdi = SdiRegistry::new();
        let plugin_manager = init_plugins(
            plugins,
            PluginContext {
                screen_w: config.screen_width,
                screen_h: config.screen_height,
                time: None,
            },
            &mut sdi,
            &mut vfs,
            &mut cmd_reg,
        );
        let mut apps = oasis_apps::builtin_registry(&oasis_apps::HostInfo::desktop());
        plugin_manager.extend_registry(&mut apps);
        let plugin_count = plugin_manager.active_count();
        let plugin_app_count = plugin_manager.plugin_apps().len();
        log::info!("Plugin system: {plugin_count} plugins active, {plugin_app_count} plugin apps");
        bios!(
            observer,
            5,
            format!("LOADING FRAGMENT... {plugin_count} PLUGINS, {plugin_app_count} APPS OK")
        );
        observer.wait_until(&mut backend, BIOS_REVEAL_TIMES[5]);

        // Line 6 (3.0s): display manager handoff with the real resolution.
        observer.status("Handing off to display manager...");
        bios!(
            observer,
            6,
            format!(
                "STARTING DISPLAY MANAGER @ {}x{} | BACKEND: {}",
                config.screen_width,
                config.screen_height,
                B::NAME
            )
        );
        observer.wait_until(&mut backend, BIOS_REVEAL_TIMES[6]);

        // -- Splash phase (3.5–6.5s): warm heavy subsystems. The BIOS lines
        //    stay visible until 3.6s, so short init steps between 3.0 and 3.6
        //    are covered by the BIOS status line; after 3.6s the status line
        //    hides and the splash logo fades in. --

        // Software shader bridge for animated shader wallpapers.
        observer.status("Compiling background shader pipeline...");
        let shader_bridge = if shader_wallpaper {
            SdlShaderBridge::new(config.screen_width, config.screen_height)
        } else {
            None
        };
        if shader_bridge.is_some() {
            log::info!("Shader bridge available");
        }

        // Derive runtime theme from the active skin, applying screen dimensions.
        let mut active_theme = ActiveTheme::from_skin(&skin.theme)
            .with_screen_size(config.screen_width, config.screen_height)
            .with_features(&skin.features);
        prefs.apply_font_scale(&mut active_theme);
        let browser_config = BrowserConfig::from_skin_theme(&skin.theme);

        // Set up platform services.
        let mut platform = DesktopPlatform::new();
        if fixed_time.is_some() {
            platform.set_fixed_time(fixed_time);
        }

        // Warm the bitmap font glyph cache: rendering never-seen characters
        // allocates + uploads a texture on first use, which used to cause
        // per-glyph hitches on the dashboard's first few frames.
        observer.status("Rasterizing font atlas...");
        prewarm_glyph_cache(&mut backend);

        observer.wait_until(&mut backend, 3.8);

        // Discover apps and merge plugin-registered apps.
        observer.status("Indexing dashboard apps...");
        let tiles = commands::dashboard_apps(&vfs, &apps, &active_theme)?;
        log::info!("Dashboard: {} apps (including plugin apps)", tiles.len());
        let ui = build_ui_layer(&config, &skin, &active_theme, tiles);

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

        // Assemble application state.
        let clear_color = active_theme.clear_color;
        let mut state = AppState {
            config,
            skin,
            active_theme,
            browser_config,
            platform,
            ui,
            terminal: TerminalLayer::new(
                cmd_reg,
                vec![
                    format!(
                        "OASIS_OS v{} -- Type 'help' for commands",
                        env!("CARGO_PKG_VERSION")
                    ),
                    "F1=terminal  F2=on-screen keyboard  Escape=quit".to_string(),
                    String::new(),
                ],
            ),
            net: NetworkLayer::new(),
            content: ContentLayer::new(apps),
            osk: None,
            plugin_manager,
            wm,
            mode: Mode::Dashboard,
            bg_color: clear_color,
            active_transition,
            frame_counter: 0,
            skin_assets: SkinAssets::default(),
            icon_drag: None,
            settings: SettingsStore::new(),
            radio: RadioLayer::new(),
            tv: TvLayer::new(),
            audio_backend: make_audio(),
            offline,
            custom_skin_root,
            toasts: ToastManager::new(),
            ui_sounds: crate::ui_sound::UiSoundQueue::new(),
            sfx: oasis_audio::sfx::SfxPlayer::new(),
            media_track: None,
            media: oasis_audio::AudioManager::new(),
            #[cfg(feature = "mcp")]
            mcp: None,
            #[cfg(feature = "mcp")]
            agent_activity: crate::mcp_tools::AgentActivity::default(),
        };

        // Optionally start the MCP control server from the environment.
        #[cfg(feature = "mcp")]
        if !offline {
            commands::mcp_start_from_env(&mut state);
        }

        // Load persisted settings and apply per-skin icon positions (free
        // icon layout). Missing files and grid-layout skins are no-ops.
        state.settings.load(&vfs);
        if let Err(e) = state.audio_backend.set_volume(prefs.volume) {
            log::warn!("Restoring volume failed: {e}");
        }
        icon_drag::load_icon_positions(
            &state.settings,
            &state.skin.manifest.name,
            &mut state.ui.dashboard,
        );

        // Prime the status bar with real time + power info so the first
        // frame shows accurate values instead of the "--:--" / "--%"
        // placeholders.
        observer.status("Polling clock and power services...");
        refresh_status_info(&mut state);

        // Load the startup skin's UI sounds (no-op for silent skins).
        ui_sfx::reload_for_skin(&mut state);

        // Show a welcome toast.
        state.toasts.show(
            format!("Skin: {}", state.skin.manifest.name),
            ToastLevel::Info,
            state.active_theme.toast.ttl,
        );

        // Load radio stations from VFS.
        observer.status("Parsing radio station registry...");
        state
            .radio
            .manager
            .load_stations(&vfs, "/etc/radio/stations.toml")
            .ok();

        // Set up scene graph and apply skin layout (runs during the
        // splash's logo-entrance phase at ~4.0s so the main loop's first
        // frame has a fully-built scene).
        observer.status("Composing SDI scene graph...");
        state.skin.apply_layout_scaled(
            &mut sdi,
            state.config.screen_width,
            state.config.screen_height,
        );
        observer.wait_until(&mut backend, 4.6);

        // Generate + upload the wallpaper texture. Doing it here hides the
        // cost under the splash animation instead of hitching frame 0.
        observer.status("Generating wallpaper texture...");
        install_wallpaper(&mut state, &mut sdi, &mut backend)?;
        log::info!("Wallpaper loaded");
        observer.wait_until(&mut backend, 5.2);

        // The SDL build runs as a desktop application, so the host window
        // manager already paints a hardware cursor over our window; no
        // software cursor is registered here. `CursorState` is still
        // updated from input events.

        // Apply auto-launch (after scene graph is fully set up).
        if let Some(ref app_name) = auto_launch_app {
            auto_launch(
                &mut state,
                &mut sdi,
                &vfs,
                app_name,
                auto_launch_url.as_deref(),
            );
        }

        // Publish the live runtime state (skin / resolution / backend) to
        // VFS so the Settings app and any other consumer can read the real
        // values instead of compile-time defaults.
        commands::publish_runtime_state(&state, B::NAME, &mut vfs);

        // Shell profile + `startup` script, now that the registry, plugins
        // and VFS are final. Output lands in the terminal scrollback.
        crate::terminal_input::run_boot_scripts(&mut state, &mut vfs);

        // BIOS phase ended at 3.6s; clear the status line so nothing
        // lingers under the splash-phase logo.
        observer.status("");

        // Restore the terminal history persisted in the VFS (Up / Ctrl+R).
        state
            .terminal
            .session
            .load_history(&state.terminal.cmd_reg, &vfs);
        // Writes /system/settings.toml back to real storage when it changes.
        let settings_mirror = user_prefs::DiskMirror::new(settings_disk_path, &vfs);

        let now = Instant::now();
        let drag_mode = state.mode;
        Ok(Self {
            drag_mode,
            state,
            sdi,
            vfs,
            backend,
            shader_bridge,
            key_filter: KeyTwinFilter::default(),
            // Publishes CPU / memory / battery / uptime to /var/sysmon/status
            // for the System Monitor app.
            sysmon_probe: oasis_apps::system_monitor::probe::HostProbe::new(
                &format!("Desktop ({})", B::NAME),
                B::NAME,
            ),
            settings_mirror,
            disk_sample_rx,
            // Frame-phase stats are opt-in via OASIS_FRAME_STATS=1.
            frame_stats: frame_stats::FrameStats::from_env(),
            last_scene_sig: None,
            last_input_at: now,
            last_present_at: now,
            last_app_tick_at: now,
            last_sysinfo_at: now,
            cron: oasis_core::script::CronScheduler::new(),
            clock_origin: now,
            tv_timeout_secs,
            tv_timeout_start: None,
            pending_screenshot: None,
            #[cfg(feature = "skin-dev")]
            skin_watcher: crate::hot_reload::SkinWatcher::new(),
        })
    }

    /// Restart the frame clocks at `now` (call once after the boot splash
    /// finishes, so its duration doesn't count as idle/input time).
    pub fn reset_clocks(&mut self, now: Instant) {
        self.last_input_at = now;
        self.last_present_at = now;
        self.last_app_tick_at = now;
        self.last_sysinfo_at = now;
        self.clock_origin = now;
    }

    /// Seconds on the animation clock at `now`, unwrapped and in `f64` so
    /// they keep sub-frame precision over any uptime (the shader bridge
    /// wraps them before the shader sees them).
    fn anim_secs(&self, now: Instant) -> f64 {
        now.saturating_duration_since(self.clock_origin)
            .as_secs_f64()
    }

    /// Run one frame of shell logic for `events` at time `now`: input
    /// dispatch, network/IPC polling, app ticks, media controllers, SDI
    /// scene update and window animations. Rendering is separate
    /// ([`Self::render`]) so the caller can elide idle frames.
    pub fn step(&mut self, events: &[InputEvent], now: Instant) -> StepOutcome {
        let anim_secs = self.anim_secs(now);
        let state = &mut self.state;
        let sdi = &mut self.sdi;
        let vfs = &mut self.vfs;
        state.frame_counter += 1;

        // Drain background disk sample loads (non-blocking).
        if let Some(rx) = self.disk_sample_rx.as_ref() {
            while let Ok((path, data)) = rx.try_recv() {
                let _ = vfs.write(&path, &data);
            }
        }

        // Refresh system info once per second of wall time (a frame count
        // ran 2.4x fast on a 144 Hz display).
        if now.saturating_duration_since(self.last_sysinfo_at) >= SYSINFO_INTERVAL {
            self.last_sysinfo_at = now;
            refresh_status_info(state);
            let _ = self
                .sysmon_probe
                .publish(vfs, Some(&state.platform), Some(&state.platform));
            self.settings_mirror.sync(vfs);
            let since_start = now.saturating_duration_since(self.clock_origin);
            terminal_input::poll_cron(state, vfs, &mut self.cron, since_start);
        }

        for event in events {
            state.ui.mouse_cursor.handle_input(event);
            let result = input::handle_event(event, &mut self.key_filter, state, sdi, vfs);
            input::end_drags_on_mode_change(state, sdi, vfs, &mut self.drag_mode);
            if result == input::InputResult::Quit {
                return StepOutcome {
                    quit: true,
                    redraw: false,
                    scene_changed: false,
                };
            }
        }
        if !events.is_empty() {
            state.terminal.dirty = true;
            self.last_input_at = now;
        }

        // Poll remote listener for incoming commands.
        commands::poll_remote_listener(state, sdi, vfs);

        // Poll the MCP control server (agent-driven UI actions).
        #[cfg(feature = "mcp")]
        {
            let backend = &mut self.backend;
            let shader_bridge = &mut self.shader_bridge;
            let mut capture = |state: &mut AppState, sdi: &mut SdiRegistry, w: u32, h: u32| {
                draw_frame(
                    state,
                    sdi,
                    backend,
                    shader_bridge,
                    anim_secs,
                    &mut None,
                    false,
                )
                .map_err(|e| e.to_string())?;
                backend.read_pixels(0, 0, w, h).map_err(|e| e.to_string())
            };
            commands::poll_mcp_server(state, sdi, vfs, &mut capture);
        }

        // Poll FTP server for incoming connections.
        commands::poll_ftp_server(state, vfs);

        // Poll remote client for received data.
        commands::poll_remote_client(state);

        // Network service state for the Network app.
        commands::publish_net_state(state, vfs);

        // `wm` terminal command IPC (window list + close/focus/... requests).
        commands::poll_wm_ipc(state, sdi, vfs);

        // `plugin load|unload` requests + the published plugin list, then
        // every active plugin's per-frame `update`.
        commands::poll_plugin_ipc(state, sdi, vfs);
        commands::update_plugins(state, sdi, vfs);

        // `notify` / `screenshot` / `theme` / `browse` request files.
        if let Some(path) = ui_ipc::poll(state, sdi, vfs) {
            self.pending_screenshot = Some(path);
        }

        // Run the next queued background terminal job (`cmd &`), if any.
        terminal_input::poll_jobs(state, sdi, vfs);

        // Carry out every VFS request the open apps queued (radio tune,
        // settings changes, ...). TV Guide tunes go to the video player
        // (`tv_controller::tick` below) instead of the VFS.
        {
            // Apps closed (or replaced) by this frame's input still get
            // their last requests carried out (a skin picked right before
            // Escape), first since they were made first, except TV tunes:
            // there is no guide left to play them in.
            let mut requests = drain_retired_requests(state);
            for (_, runner) in &mut state.content.open_runners {
                runner.drain_requests(&mut requests);
            }
            carry_out_app_requests(state, vfs, requests);
        }

        // Let open apps apply queued VFS mutations (file manager deletes /
        // renames / copies, paint's binary BMP saves).
        for (_, runner) in &mut state.content.open_runners {
            runner.apply_vfs_ops(vfs);
        }

        // Advance time-driven app state (game loops, slideshows) by wall
        // time, every iteration — elided frames included — so it runs at
        // a fixed rate independent of FPS and input.
        {
            let dt_ms = u32::try_from(
                now.saturating_duration_since(self.last_app_tick_at)
                    .as_millis(),
            )
            .unwrap_or(u32::MAX);
            self.last_app_tick_at = now;
            for (_, runner) in &mut state.content.open_runners {
                runner.tick(dt_ms, vfs);
            }
        }
        // Apps that decided to close outside an input handler (Text
        // Editor "Save & close" once the save above is written).
        input::apply_app_close_requests(state, sdi, vfs);
        // Runners closed just now may have queued requests in this frame's
        // `apply_vfs_ops` / `tick` (after the pass above): carry them out
        // before the runners drop, or they are lost.
        let late = drain_retired_requests(state);
        carry_out_app_requests(state, vfs, late);
        // Closed apps release their backend resources (textures) now that
        // the backend is at hand, then drop.
        for mut runner in state.content.retired_runners.drain(..) {
            runner.release_resources(&mut self.backend);
        }

        // Dispatch any Settings-app IPC requests (skin swap, resolution
        // change). Must run after the pending-VFS-request block above,
        // which is what writes the IPC payload into the VFS.
        commands::poll_settings_ipc(
            state,
            sdi,
            &mut self.backend,
            &mut self.shader_bridge,
            vfs,
            B::NAME,
        );

        // Dev-only: reload the active external skin when its files change
        // on disk. Passing the directory path forces `resolve_skin` to
        // re-read the TOML instead of hitting the compiled-in copy.
        #[cfg(feature = "skin-dev")]
        if state
            .frame_counter
            .is_multiple_of(crate::hot_reload::POLL_INTERVAL_FRAMES)
            && let Some(dir) = self.skin_watcher.poll(&state.skin.manifest.name)
        {
            log::info!("skin-dev: reloading skin from {}", dir.display());
            commands::apply_skin_swap(&dir.to_string_lossy(), state, sdi, vfs);
        }

        // Any skin swap — whether from the Settings app above or a terminal
        // `skin` command processed in the input loop — sets the pending flag
        // so the wallpaper texture is regenerated against the new theme.
        commands::refresh_wallpaper_if_pending(state, sdi, &mut self.backend);

        // Animate image decal layers (drift/pulse) by mutating their SDI
        // objects; static layers cost nothing here.
        if !state.skin_assets.image_layers.is_empty() {
            oasis_core::image_layers::tick_image_layers(
                sdi,
                &state.skin_assets.image_layers,
                anim_secs as f32,
                state.active_theme.background_reduced_motion,
            );
        }

        // Tick radio, music player, and TV subsystems.
        radio_controller::tick(state, vfs, now);
        media_controller::tick(state, vfs);
        tv_controller::tick(state, &mut self.backend, vfs);

        // UI sound effects: fire queued events and pump mixed PCM into the
        // dedicated SFX stream (no-op for skins without a [sounds] table).
        ui_sfx::tick(state);

        // Auto-exit timer for TV streaming tests.
        if let Some(timeout) = self.tv_timeout_secs {
            if state.tv.video_player.is_active() && self.tv_timeout_start.is_none() {
                self.tv_timeout_start = Some(now);
                log::info!("TV test: decode active, auto-exit in {timeout}s");
            }
            if let Some(start) = self.tv_timeout_start
                && now.saturating_duration_since(start).as_secs() >= timeout
            {
                log::info!("TV test: timeout reached, exiting");
                return StepOutcome {
                    quit: true,
                    redraw: false,
                    scene_changed: false,
                };
            }
        }

        // Remote / MCP / IPC / app actions above may have switched modes.
        input::end_drags_on_mode_change(state, sdi, vfs, &mut self.drag_mode);

        // Update SDI scene graph for the active mode.
        render::update_sdi(state, sdi, vfs);

        // Window open/close/minimize/restore animations; skins that ask
        // for reduced motion get instant transitions.
        state
            .wm
            .set_motion_enabled(!state.skin.features.reduced_motion);
        state.wm.tick_animations_at(now, sdi);

        // Assemble entrance: slide the bars in and hide bar content while
        // the transition runs (no-op for fade/none entrances).
        if let Some(ref trans) = state.active_transition {
            transition::apply_assemble(sdi, &state.active_theme, trans);
        }

        // Drive browser image streaming (progressive loading), and keep
        // the window title in step with the page title.
        if let Some(ref mut bw) = state.content.browser {
            bw.tick(vfs);
            let title = launch::browser_window_title(bw);
            state.wm.set_window_title(BROWSER_WIN, &title, sdi);
        }

        let (redraw, scene_changed) = self.needs_redraw(now);
        if !redraw && let Some(ref mut stats) = self.frame_stats {
            stats.record_skipped();
        }
        StepOutcome {
            quit: false,
            redraw,
            scene_changed,
        }
    }

    /// Idle frame elision: whether anything on screen can have changed.
    ///
    /// Skip clear/draw/present entirely when nothing can have changed.
    /// The SDI dirty check re-hashes only objects touched this frame
    /// (identical rewrites don't count); a dirty frame is then confirmed
    /// against a full scene content signature, which also catches changes
    /// that cancel out against the last drawn frame. Everything that
    /// paints OUTSIDE the SDI scene graph gets an explicit condition.
    /// Bias: when in doubt, redraw — a wasted frame is cheap, a stale
    /// frame is a bug.
    ///
    /// Returns `(redraw, scene_changed)`.
    fn needs_redraw(&mut self, now: Instant) -> (bool, bool) {
        let anim_secs = self.anim_secs(now);
        let state = &self.state;
        let scene_changed = if self.sdi.take_scene_dirty() {
            let sig = self.sdi.scene_signature();
            let changed = self.last_scene_sig != Some(sig);
            self.last_scene_sig = Some(sig);
            changed
        } else {
            false
        };

        // Shader wallpapers advance at the bridge's 30 Hz shade cadence;
        // between shades a redraw would reproduce identical pixels.
        let shader_wants_frame = if let Some(ref bridge) = self.shader_bridge {
            get_shader_layer(&state.active_theme).is_some() && bridge.would_shade(anim_secs)
        } else {
            false
        };

        // Animations that paint directly to the backend (outside SDI):
        // transitions, dashboard vector icons, background/chrome layers.
        let anim_active = state.active_transition.is_some()
            || state.ui.dashboard.has_active_animation(&state.active_theme)
            || oasis_core::vector_overlay::layers_animated(
                &state.active_theme.background_layers,
                state.active_theme.background_reduced_motion,
            )
            || oasis_core::vector_overlay::layers_animated(
                &state.active_theme.chrome_layers,
                state.active_theme.background_reduced_motion,
            )
            || state.wm.is_animating()
            // The corrupted skin's per-frame glitch never settles.
            || corruption_active(state);

        // Windowed app content (browser, app runners) paints via draw
        // callbacks the SDI registry can't see: redraw while a visible
        // window's content changed or animates, or a window is being
        // dragged/animated — not merely because a window is open.
        // Fullscreen kiosk apps and media playback repaint continuously.
        let windows_active = state.mode == Mode::Desktop
            && render::windows_want_frame(
                &state.wm,
                &state.ui.desktops,
                &state.content.open_runners,
                state.content.browser.as_ref(),
            );
        let content_active = windows_active
            || state.content.fullscreen_app.is_some()
            || state.tv.video_player.is_active()
            || state.radio.source.is_some()
            || state.media_track.is_some()
            || state.tv.audio_track.is_some();

        // Keep redrawing while the agent-activity overlay is visible (and
        // so the frame right after a tool call always paints).
        #[cfg(feature = "mcp")]
        let agent_active = state.agent_activity.is_active(Duration::from_secs(6));
        #[cfg(not(feature = "mcp"))]
        let agent_active = false;

        let redraw = scene_changed
            || anim_active
            || shader_wants_frame
            || content_active
            || agent_active
            || self.pending_screenshot.is_some()
            // Input drives derived UI (hover, drag, key repeat) for a few
            // frames past the event; window resize/expose arrive as events
            // too, so they land in the same grace window.
            || now.saturating_duration_since(self.last_input_at) <= INPUT_REDRAW_GRACE
            // Present at least once per heartbeat as a self-healing
            // backstop for any condition missed above.
            || now.saturating_duration_since(self.last_present_at) >= REDRAW_HEARTBEAT;
        (redraw, scene_changed)
    }

    /// Draw and present one frame at time `now`.
    pub fn render(&mut self, now: Instant) -> Result<()> {
        let anim_secs = self.anim_secs(now);
        let state = &mut self.state;
        let sdi = &mut self.sdi;
        let backend = &mut self.backend;
        let mut phase_clock = self
            .frame_stats
            .as_ref()
            .map(|_| frame_stats::PhaseClock::start());
        draw_frame(
            state,
            sdi,
            backend,
            &mut self.shader_bridge,
            anim_secs,
            &mut phase_clock,
            true,
        )?;

        // `screenshot`: capture the finished frame before presenting it.
        if let Some(path) = self.pending_screenshot.take() {
            let (w, h) = (state.active_theme.screen_w, state.active_theme.screen_h);
            let pixels = backend.read_pixels(0, 0, w, h);
            ui_ipc::save_screenshot(state, &mut self.vfs, &path, pixels, w, h);
        }

        backend.swap_buffers()?;
        self.last_present_at = now;
        // Every drawn frame repaints all visible window content, so each
        // runner's pending change is now on screen.
        for (_, runner) in &mut state.content.open_runners {
            runner.mark_drawn();
        }
        if let (Some(stats), Some(pc)) = (self.frame_stats.as_mut(), phase_clock.take()) {
            stats.record_drawn(pc.finish());
        }
        Ok(())
    }

    /// Drop every app, open or already closed, after it released its
    /// backend resources (the `App::release_resources` contract; open
    /// windows used to be dropped at exit without it).
    pub(crate) fn release_all_apps(&mut self) {
        let content = &mut self.state.content;
        let open = content.open_runners.drain(..).map(|(_, runner)| runner);
        for mut runner in content.retired_runners.drain(..).chain(open) {
            runner.release_resources(&mut self.backend);
        }
    }

    /// Shut the plugins down, persist settings, stop media and release
    /// the backend.
    pub fn shutdown(mut self) -> Result<()> {
        self.state.plugin_manager.shutdown_all(
            &mut self.sdi,
            &mut self.vfs,
            &mut self.state.terminal.cmd_reg,
        );
        // Persist any settings changed since the last periodic sync.
        self.settings_mirror.sync(&self.vfs);

        self.release_all_apps();
        // Clean up video player before shutting down backend.
        self.state.tv.video_player.stop(&mut self.backend);
        if let Some(track) = self.state.tv.audio_track.take() {
            let _ = self.state.audio_backend.unload_track(track);
        }

        // Clean up all cached video files.
        #[cfg(feature = "_video")]
        for (_, path) in &self.state.tv.video_cache {
            if let Err(e) = std::fs::remove_file(path) {
                log::warn!("TV: failed to remove cached file {}: {e}", path.display());
            }
        }

        self.backend.shutdown()?;
        log::info!("OASIS_OS shut down cleanly");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Boot phases (called from `Shell::boot` between splash progress reports)
// ---------------------------------------------------------------------------

/// Draw one complete frame into the backend's back buffer (no present):
/// the shader wallpaper, the SDI scene with its windows, the vector
/// chrome, the transition overlay and the agent overlay.
///
/// [`Shell::render`] draws with it and then presents; MCP `screenshot`
/// draws with it and reads the pixels back before anything is presented,
/// like the `screenshot` command (reading the back buffer after a present
/// is undefined on SDL). `advance_transition` is false for such extra
/// draws, so they do not speed up a running transition.
fn draw_frame<B: ShellBackend>(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut B,
    shader_bridge: &mut Option<SdlShaderBridge>,
    anim_secs: f64,
    phase_clock: &mut Option<frame_stats::PhaseClock>,
    advance_transition: bool,
) -> Result<()> {
    backend.clear(state.bg_color)?;

    // Render shader wallpaper FIRST (replaces bg_color clear).
    // This runs every drawn frame so the animation stays live in all
    // modes (elided frames redraw whenever the bridge would shade),
    // except while an opaque surface provably covers the whole canvas
    // (fullscreen kiosk app, full-screen opaque window): then the
    // bridge skips both the CPU shade pass and the blit. Time keeps
    // advancing during occlusion, so on reveal the animation resumes
    // at the current time (immediately — no 30 Hz wait). Occluding
    // states always come with `content_active` redraws, so the
    // visibility pushed here is never stale on a frame that needs it.
    if let Some(bridge) = shader_bridge.as_mut()
        && let Some(info) = get_shader_layer(&state.active_theme)
    {
        bridge.set_visibility(render::wallpaper_visibility(state));
        bridge.render_and_blit(backend, &info.name, anim_secs, &info.params);
    }
    if let Some(pc) = phase_clock.as_mut() {
        pc.lap_shader();
    }

    // Corrupted-skin glitch: distort the final layout for this one
    // frame only; restored right after the SDI pass below.
    let corruption = corruption_active(state)
        .then_some(state.skin.corrupted_modifiers.as_ref())
        .flatten()
        .map(|m| {
            let seed = state.frame_counter as u32;
            m.apply(sdi, &mut oasis_core::skin::SimpleRng::new(seed))
        });

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
        let overlay = |be: &mut dyn SdiBackend| -> oasis_core::error::Result<()> {
            if wants_vector_icons {
                dashboard.render_vector_icons(be, active_theme, frame)?;
            }
            Ok(())
        };
        state.wm.draw_with_clips_overlay(
            sdi,
            backend,
            overlay,
            |window_id, cx, cy, cw, ch, be| {
                if window_id == BROWSER_WIN {
                    if let Some(ref mut bw) = state.content.browser {
                        bw.set_window(cx, cy, cw, ch);
                        bw.paint(be)
                    } else {
                        Ok(())
                    }
                } else if let Some((_, runner)) = state
                    .content
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| id == window_id)
                {
                    runner.draw_windowed(cx, cy, cw, ch, be, &state.active_theme)
                } else {
                    Ok(())
                }
            },
        )?;
        // Drag-to-edge snap preview, on top of the windows.
        render::draw_snap_preview(state, backend)?;
    } else if state.mode == Mode::Dashboard
        && (state.active_theme.icon.style == "vector"
            || !state.active_theme.background_layers.is_empty())
    {
        // Split draw: base layer → vector overlays/icons → overlay
        // layer. Shader already rendered above as wallpaper.
        sdi.draw_base_layer(backend)?;

        oasis_core::vector_overlay::render_vector_background_cached(
            backend,
            &state.active_theme,
            state.frame_counter as u32,
            &mut state.skin_assets.background_cache,
        )?;
        state.ui.dashboard.render_vector_icons(
            backend,
            &state.active_theme,
            state.frame_counter as u32,
        )?;
        sdi.draw_overlay_layer(backend)?;
    } else {
        sdi.draw(backend)?;
    }
    if let Some(undo) = corruption {
        undo.restore(sdi);
    }
    if let Some(pc) = phase_clock.as_mut() {
        pc.lap_sdi();
    }

    // Vector chrome layers paint on top of the SDI scene (bars, tabs,
    // windows) in every mode — procedurally shaped chrome accents.
    if !state.active_theme.chrome_layers.is_empty() {
        oasis_core::vector_overlay::render_vector_chrome(
            backend,
            &state.active_theme,
            state.frame_counter as u32,
            &mut state.skin_assets.chrome_cache,
        )?;
    }

    // Status bar indicator glyphs (battery / AC / Wi-Fi) are vector
    // art, painted over the bar after the SDI pass. No-op while the
    // bar is hidden.
    oasis_core::statusbar::render_status_glyphs(backend, sdi)?;

    // Paint terminal scrollbar when in terminal mode.
    if state.mode == Mode::Terminal {
        terminal_sdi::paint_terminal_scrollbar(
            backend,
            state.terminal.output_lines.len(),
            state.terminal.scroll_offset,
            &state.active_theme,
        )?;
    }

    // Draw transition overlay if active.
    if let Some(ref mut trans) = state.active_transition {
        trans.draw_overlay(backend)?;
        if advance_transition {
            trans.tick();
            if trans.is_done() {
                state.active_transition = None;
            }
        }
    }
    if let Some(pc) = phase_clock.as_mut() {
        pc.lap_vector();
    }

    // Assistant-activity overlay (agent connected/acting).
    #[cfg(feature = "mcp")]
    crate::mcp_tools::draw_agent_overlay(backend, &state.agent_activity, &state.active_theme)?;
    Ok(())
}

/// Drain the VFS requests of the retired (closed or replaced) runners,
/// minus TV tunes: there is no guide left to play them in.
fn drain_retired_requests(state: &mut AppState) -> Vec<oasis_core::apps::AppRequest> {
    let mut requests = Vec::new();
    for runner in &mut state.content.retired_runners {
        runner.drain_requests(&mut requests);
    }
    requests.retain(|req| oasis_apps::tv_guide::tune_url_request(req).is_none());
    requests
}

/// Carry out the apps' VFS requests in order: TV Guide tunes go to the
/// video player (`tv_controller::tick`), the rest are written to their
/// request files.
fn carry_out_app_requests(
    state: &mut AppState,
    vfs: &mut MemoryVfs,
    requests: Vec<oasis_core::apps::AppRequest>,
) {
    for req in requests {
        if req.path == oasis_core::ipc::TV_REQUEST_PATH
            && let Some(oasis_core::ipc::TvRequest::TuneUrl(tune)) =
                oasis_core::ipc::TvRequest::parse(&req.data)
        {
            state.tv.pending_tune = Some(tune);
            continue;
        }
        // The music request file is a line queue (several requests per
        // frame, the `music` command's included); the other request files
        // hold the latest request.
        let written = if req.path == oasis_core::ipc::AUDIO_REQUEST_PATH {
            media_controller::queue_request(vfs, &req.data)
        } else {
            vfs.write(&req.path, req.data.as_bytes())
        };
        if let Err(e) = written {
            log::warn!("app VFS request write failed ({}): {e}", req.path);
        }
    }
}

/// BIOS line 2: real RAM + logical core count.
fn ram_bios_line() -> String {
    match (sysinfo::total_ram_kb(), sysinfo::cpu_core_count()) {
        (Some(kb), Some(cores)) => format!(
            "SYSTEM RAM CHECK... {}K OK ({cores} LOGICAL CORES DETECTED)",
            format_thousands(kb)
        ),
        (Some(kb), None) => format!("SYSTEM RAM CHECK... {}K OK", format_thousands(kb)),
        _ => "SYSTEM RAM CHECK... OK".to_string(),
    }
}

/// The boot VFS: demo content, man pages, motd, profile, and
/// `/system/settings.toml` seeded with the persisted settings.
fn seed_vfs(boot_settings: &SettingsStore) -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs_setup::populate_demo_vfs(&mut vfs);
    oasis_core::terminal::populate_man_pages(&mut vfs);
    oasis_core::terminal::populate_motd(&mut vfs);
    oasis_core::terminal::populate_profile(&mut vfs);
    if boot_settings.keys().next().is_some() {
        let mut seeded = boot_settings.clone();
        seeded.save(&mut vfs);
    }
    vfs
}

/// BIOS line 3: the VFS file / dir / byte totals.
fn vfs_bios_line(vfs: &MemoryVfs) -> String {
    let (vfs_files, vfs_dirs, vfs_truncated) = sysinfo::count_vfs_entries(vfs, "/");
    let (vfs_bytes, bytes_truncated) = sysinfo::total_vfs_bytes(vfs, "/");
    // Append "+" to counts when the depth guard stopped the walk early —
    // makes it obvious to the user that the numbers are lower bounds
    // rather than showing a confident-but-wrong total.
    let trunc_mark = if vfs_truncated || bytes_truncated {
        "+"
    } else {
        ""
    };
    format!(
        "INITIALIZING VIRTUAL FILE SYSTEM... {vfs_files}{trunc_mark} FILES, \
         {vfs_dirs}{trunc_mark} DIRS, {}{trunc_mark} KB OK",
        vfs_bytes / 1024,
    )
}

/// Every terminal command the desktop shell offers.
fn shell_commands() -> CommandRegistry {
    let mut cmd_reg = CommandRegistry::new();
    register_builtins(&mut cmd_reg);
    oasis_core::script::register_script_commands(&mut cmd_reg);
    oasis_core::transfer::register_transfer_commands(&mut cmd_reg);
    oasis_core::update::register_update_commands(&mut cmd_reg);
    register_plugin_commands(&mut cmd_reg);
    register_agent_commands(&mut cmd_reg);
    oasis_apps::register_commands(&mut cmd_reg);
    oasis_core::terminal::register_browser_commands(&mut cmd_reg);
    #[cfg(feature = "mcp")]
    crate::mcp_command::register(&mut cmd_reg);
    cmd_reg
}

/// Register the built-in plugins plus `extra`, and initialize them all
/// into the shell's scene graph.
fn init_plugins(
    extra: Vec<Box<dyn Plugin>>,
    context: PluginContext,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
    cmd_reg: &mut CommandRegistry,
) -> PluginManager {
    let mut plugin_manager = PluginManager::new();
    register_builtin_plugins(&mut plugin_manager);
    for plugin in extra {
        plugin_manager.register_static(plugin);
    }
    plugin_manager.set_context(context);
    plugin_manager.init_all(sdi, vfs, cmd_reg);
    plugin_manager
}

/// Dashboard (with `apps`), PSIX-style bars, taskbar, start menu and
/// mouse cursor for the startup skin.
fn build_ui_layer(
    config: &OasisConfig,
    skin: &Skin,
    at: &ActiveTheme,
    apps: Vec<AppEntry>,
) -> UiLayer {
    let dashboard = DashboardState::new(DashboardConfig::from_features(&skin.features, at), apps);
    let mut bottom_bar = BottomBar::new();
    bottom_bar.total_pages = dashboard.page_count();
    let mut mouse_cursor = CursorState::new(config.screen_width, config.screen_height);
    mouse_cursor.scale = at.cursor_scale;
    UiLayer {
        dashboard,
        status_bar: StatusBar::new(),
        bottom_bar,
        taskbar: oasis_core::taskbar::Taskbar::new(),
        start_menu: StartMenuState::new_with_theme(StartMenuState::default_items(at), at),
        mouse_cursor,
        desktops: oasis_core::wm::DesktopManager::new(1),
    }
}

/// Generate and upload the wallpaper texture, then the skin's layout
/// textures and image decal layers (skin swaps rebuild these via the
/// pending-refresh path).
fn install_wallpaper(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    backend: &mut impl ShellBackend,
) -> Result<()> {
    let (w, h) = (state.config.screen_width, state.config.screen_height);
    let wp_data = wallpaper::generate_with_assets(w, h, &state.active_theme, &state.skin.assets);
    let wallpaper_tex = backend.load_texture(w, h, &wp_data)?;
    terminal_sdi::setup_wallpaper(sdi, wallpaper_tex, w, h);
    if get_shader_layer(&state.active_theme).is_some()
        && let Ok(obj) = sdi.get_mut("wallpaper")
    {
        obj.visible = false;
    }
    commands::refresh_skin_assets(state, sdi, backend);
    Ok(())
}

/// `OASIS_APP` / `OASIS_URL`: open `app_name`, and navigate the browser to
/// `url` when it is the browser.
fn auto_launch(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    app_name: &str,
    url: Option<&str>,
) {
    if !launch_by_title(state, sdi, vfs, app_name) {
        log::warn!("OASIS_APP={app_name}: no such app");
        return;
    }
    if let Some(url) = url
        && let Some(ref mut bw) = state.content.browser
    {
        bw.navigate_vfs(url, vfs);
        log::info!("Auto-navigated to: {url}");
    }
}

/// Refresh the status / bottom bar's clock, battery and Wi-Fi info from
/// the platform services.
fn refresh_status_info(state: &mut AppState) {
    let time = state.platform.now().ok();
    let power = state.platform.power_info().ok();
    state
        .ui
        .status_bar
        .update_info(time.as_ref(), power.as_ref());
    let wifi = state.platform.wifi_info().ok();
    state.ui.status_bar.update_wifi(wifi.as_ref());
    state.ui.bottom_bar.update_info(time.as_ref());
}

/// Launch the app titled `title` (case-insensitive) the way `OASIS_APP`
/// auto-launch does: any app in the registry (built-in or plugin, with or
/// without a dashboard tile) or the Browser. Returns `false` if no such
/// app exists.
pub fn launch_by_title(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &MemoryVfs,
    title: &str,
) -> bool {
    match launch::open_by_title(state, sdi, vfs, title, None, LaunchOpts::QUIET) {
        Ok(title) => {
            log::info!("Auto-launched app: {title}");
            true
        },
        Err(msg) => {
            log::warn!("OASIS_APP: {msg}");
            false
        },
    }
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
fn prewarm_glyph_cache(backend: &mut impl SdiBackend) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_thousands_groups_digits() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1_000");
        assert_eq!(format_thousands(127_539_224), "127_539_224");
    }
}
