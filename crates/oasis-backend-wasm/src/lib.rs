//! WebAssembly backend for OASIS_OS.
//!
//! Renders to an HTML `<canvas>` element using the Canvas 2D API,
//! maps DOM events to `InputEvent`, and provides Web Audio playback.

pub mod archive;
pub mod audio;
pub mod font;
mod gradients;
pub mod iframe;
pub mod input;
pub mod network;
pub mod platform;
pub mod renderer;
pub mod shader_bridge;
mod shapes;
mod textures;
pub mod tv_catalog;
pub mod video;
#[cfg(feature = "wasm-youtube")]
pub mod youtube;

mod input_dispatch;
mod vfs_content;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::AppRunner;
use oasis_core::backend::{AudioBackend, Color, InputBackend, SdiCore, TextureId};
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{AppEntry, DashboardConfig, DashboardState, discover_apps};
use oasis_core::osk::OskState;
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::startmenu::{StartMenuAction, StartMenuState};
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{CommandOutput, CommandRegistry, Environment, register_builtins};
use oasis_core::terminal_sdi;
use oasis_core::toast::ToastManager;
use oasis_core::transition::{self, TransitionState};
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;
use oasis_core::wm::manager::WindowManager;
use oasis_core::wm::window::{WindowConfig, WindowType};

use audio::WasmAudioBackend;
use iframe::IframeOverlay;
use input::WasmInputBackend;
use network::WasmNetworkBackend;
use platform::WasmPlatform;
use renderer::WasmBackend;

// ---------------------------------------------------------------------------
// Console logging for WASM
// ---------------------------------------------------------------------------

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format!($($t)*)))
}

// ---------------------------------------------------------------------------
// Mode enum (mirrors oasis-app's AppState::Mode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Dashboard,
    Terminal,
    #[allow(dead_code)]
    App,
    Osk,
    Desktop,
}

// ---------------------------------------------------------------------------
// OasisWasm -- the wasm-bindgen entry point
// ---------------------------------------------------------------------------

/// OASIS_OS instance for the browser.
///
/// Create with `OasisWasm::new("canvas_id")`, then call `tick()` from
/// `requestAnimationFrame`.
#[wasm_bindgen]
pub struct OasisWasm {
    backend: WasmBackend,
    input: WasmInputBackend,
    audio: WasmAudioBackend,
    #[allow(dead_code)]
    network: WasmNetworkBackend,
    sdi: SdiRegistry,
    cmd_reg: CommandRegistry,
    vfs: MemoryVfs,
    platform: WasmPlatform,
    skin: Skin,
    active_theme: ActiveTheme,
    browser_config: BrowserConfig,
    dashboard: DashboardState,
    status_bar: StatusBar,
    bottom_bar: BottomBar,
    taskbar: oasis_core::taskbar::Taskbar,
    desktops: oasis_wm::DesktopManager,
    start_menu: StartMenuState,
    mouse_cursor: CursorState,
    cursor_texture: Option<TextureId>,
    wm: WindowManager,
    app_runner: Option<AppRunner>,
    open_runners: Vec<(String, AppRunner)>,
    browser: Option<BrowserWidget>,
    iframe: IframeOverlay,
    toasts: ToastManager,
    osk: Option<OskState>,
    active_transition: Option<TransitionState>,
    mode: Mode,
    cwd: String,
    input_buf: String,
    output_lines: Vec<String>,
    terminal_scroll_offset: usize,
    frame_counter: u64,
    bg_color: Color,
    width: u32,
    height: u32,
    radio_manager: oasis_audio::RadioManager,
    radio_source: Option<Box<dyn oasis_audio::radio::RadioSource>>,
    archive_catalog: Option<oasis_audio::radio::ArchiveCatalog>,
    pending_catalog: Option<archive::WasmArchiveCatalogFetcher>,
    video_player: video::VideoPlayer,
    pending_tv_catalog: Option<tv_catalog::WasmTvCatalogFetcher>,
    shader_bridge: Option<shader_bridge::WasmShaderBridge>,
    /// Window id of the currently fullscreen-kiosk app (if any).
    fullscreen_app: Option<String>,
    /// In-flight YouTube search; backend polls each tick and publishes
    /// results to `/tmp/video_embed_results` so the embed app can
    /// re-render.
    #[cfg(feature = "wasm-youtube")]
    pending_youtube_search: Option<youtube::WasmYoutubeSearchFetcher>,
    /// Textures allocated for the most recent search's thumbnails. Held
    /// here so the next search can free them up front instead of
    /// leaking offscreen `<canvas>` elements.
    #[cfg(feature = "wasm-youtube")]
    youtube_thumb_textures: Vec<TextureId>,
    /// Video id currently being shown in the iframe overlay (if any).
    /// `None` while the embed app is in search/results mode; set when
    /// `play:<id>` is received and cleared on `stop` or window close.
    #[cfg(feature = "wasm-youtube")]
    youtube_active_id: Option<String>,
    /// Pre-built embed URL for `youtube_active_id`, computed once on
    /// `play:<id>` and reused every frame so the per-frame iframe glue
    /// in `tick()` doesn't re-allocate `format!("…/embed/{id}…")`.
    #[cfg(feature = "wasm-youtube")]
    youtube_active_url: Option<String>,
}

#[wasm_bindgen]
impl OasisWasm {
    /// Create a new OASIS_OS instance attached to a canvas element.
    ///
    /// `canvas_id` is the DOM `id` of the target `<canvas>`.
    /// `skin_name` is an optional built-in skin name (e.g. "classic", "modern").
    #[wasm_bindgen(constructor)]
    pub fn new(canvas_id: &str, skin_name: Option<String>) -> Result<OasisWasm, JsValue> {
        // Route Rust panics through console.error with a readable stack
        // trace. Without this, WASM unwinds surface as "RuntimeError:
        // unreachable" with only the JS frame, making panics inside
        // tick() nearly impossible to diagnose from the browser.
        console_error_panic_hook::set_once();

        // Get canvas element.
        let document = web_sys::window()
            .ok_or_else(|| JsValue::from_str("no window"))?
            .document()
            .ok_or_else(|| JsValue::from_str("no document"))?;

        let canvas = document
            .get_element_by_id(canvas_id)
            .ok_or_else(|| JsValue::from_str(&format!("canvas '{canvas_id}' not found")))?
            .dyn_into::<HtmlCanvasElement>()
            .map_err(|_| JsValue::from_str("element is not a canvas"))?;

        // Resolve skin first so we can use its declared resolution.
        let skin_ref = skin_name.as_deref().unwrap_or("xp");
        let skin = oasis_skin::resolve_skin(skin_ref)
            .map_err(|e| JsValue::from_str(&format!("skin: {e}")))?;

        // Use the skin's declared resolution (e.g. classic=480x272,
        // modern=800x600, xp=1024x768) so the WASM build matches the
        // desktop SDL version for each skin.
        let width = skin.manifest.screen_width;
        let height = skin.manifest.screen_height;

        // Resize the canvas to the skin's resolution. CSS handles the
        // visual scaling to fill the viewport.
        canvas.set_width(width);
        canvas.set_height(height);

        // Create backends.
        let mut backend = WasmBackend::new(canvas.clone())
            .map_err(|e| JsValue::from_str(&format!("renderer init: {e}")))?;
        backend
            .init(width, height)
            .map_err(|e| JsValue::from_str(&format!("backend init: {e}")))?;

        let input_backend = WasmInputBackend::new(&canvas, width, height);

        let mut audio = WasmAudioBackend::new();
        let _ = audio.init();

        let network = WasmNetworkBackend::new();
        let platform = WasmPlatform::new();

        // Iframe overlay for real web page rendering.
        let iframe = IframeOverlay::new(&canvas)
            .map_err(|e| JsValue::from_str(&format!("iframe overlay: {e:?}")))?;

        // In-canvas video player for TV Guide playback.
        let video_player = video::VideoPlayer::new();

        // Scene graph and commands.
        let mut sdi = SdiRegistry::new();
        let mut cmd_reg = CommandRegistry::new();
        register_builtins(&mut cmd_reg);
        oasis_core::terminal::register_tv_commands(&mut cmd_reg);

        // VFS with demo content.
        let mut vfs = MemoryVfs::new();
        vfs_content::populate_wasm_vfs(&mut vfs);

        let active_theme = ActiveTheme::from_skin(&skin.theme)
            .with_screen_size(width, height)
            .with_features(&skin.features);
        let mut browser_config = BrowserConfig::from_skin_theme(&skin.theme);
        // In WASM mode, use Google's iframe-compatible search page as home
        // and delegate http(s) rendering to the <iframe> overlay. The
        // built-in engine can't sync-fetch over the network in a browser
        // and initialising QuickJS on the error page each navigation used
        // to crash the tab; iframe mode skips both.
        browser_config.features.home_url = "https://www.google.com/webhp?igu=1".to_string();
        browser_config.features.iframe_http_mode = true;

        // Apply skin layout and discover apps.
        skin.apply_layout(&mut sdi);
        let mut apps = discover_apps(&vfs, "/apps", None).unwrap_or_default();
        #[cfg(feature = "wasm-youtube")]
        apps.push(AppEntry {
            title: "Video Embed".to_string(),
            path: "/apps/video-embed".to_string(),
            icon_png: Vec::new(),
            color: Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
        });
        let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
        let dashboard = DashboardState::new(dash_config, apps);

        // Bars, start menu.
        let mut bottom_bar = BottomBar::new();
        bottom_bar.total_pages = dashboard.page_count();
        let start_menu = StartMenuState::new_with_theme(
            StartMenuState::default_items(&active_theme),
            &active_theme,
        );

        // Window manager.
        let wm = WindowManager::with_theme(width, height, skin.theme.build_wm_theme());

        // Wallpaper texture.
        let wp_data = wallpaper::generate_from_config(width, height, &active_theme);
        let wallpaper_tex = backend
            .load_texture(width, height, &wp_data)
            .map_err(|e| JsValue::from_str(&format!("wallpaper: {e}")))?;
        terminal_sdi::setup_wallpaper(&mut sdi, wallpaper_tex, width, height);
        // Hide wallpaper SDI object when a shader layer replaces it.
        if oasis_core::vector_overlay::get_shader_layer(&active_theme).is_some()
            && let Ok(obj) = sdi.get_mut("wallpaper")
        {
            obj.visible = false;
        }

        // Mouse cursor texture.
        let mut mouse_cursor = CursorState::new(width, height);
        mouse_cursor.scale = active_theme.cursor_scale;
        let cursor_texture;
        {
            let (cursor_pixels, cw, ch) = cursor::generate_cursor_pixels(active_theme.cursor_scale);
            let cursor_tex = backend
                .load_texture(cw, ch, &cursor_pixels)
                .map_err(|e| JsValue::from_str(&format!("cursor: {e}")))?;
            cursor_texture = Some(cursor_tex);
            mouse_cursor.update_sdi(&mut sdi);
            if let Ok(obj) = sdi.get_mut("mouse_cursor") {
                obj.texture = Some(cursor_tex);
            }
        }

        // Boot transition: fade in from black.
        let fade_frames = skin.features.transition_fade_frames.unwrap_or(15);
        let active_transition = Some(transition::fade_in_custom(width, height, fade_frames));

        console_log!(
            "OASIS_OS WASM initialized ({}x{}, skin: {})",
            width,
            height,
            skin_ref
        );

        Ok(OasisWasm {
            backend,
            input: input_backend,
            audio,
            network,
            sdi,
            cmd_reg,
            vfs,
            platform,
            skin,
            active_theme,
            browser_config,
            dashboard,
            status_bar: StatusBar::new(),
            bottom_bar,
            taskbar: oasis_core::taskbar::Taskbar::new(),
            desktops: oasis_wm::DesktopManager::new(4),
            start_menu,
            mouse_cursor,
            cursor_texture,
            wm,
            app_runner: None,
            open_runners: Vec::new(),
            browser: None,
            iframe,
            toasts: ToastManager::new(),
            osk: None,
            active_transition,
            mode: Mode::Dashboard,
            cwd: "/".to_string(),
            input_buf: String::new(),
            output_lines: vec![
                "OASIS_OS v1.0.0 -- Type 'help' for commands".to_string(),
                "F1=terminal  F2=on-screen keyboard".to_string(),
                String::new(),
            ],
            terminal_scroll_offset: 0,
            frame_counter: 0,
            bg_color: Color::rgb(0, 0, 0),
            width,
            height,
            radio_manager: oasis_audio::RadioManager::new(),
            radio_source: None,
            archive_catalog: None,
            pending_catalog: None,
            video_player,
            pending_tv_catalog: None,
            shader_bridge: shader_bridge::WasmShaderBridge::new(width, height),
            fullscreen_app: None,
            #[cfg(feature = "wasm-youtube")]
            pending_youtube_search: None,
            #[cfg(feature = "wasm-youtube")]
            youtube_thumb_textures: Vec::new(),
            #[cfg(feature = "wasm-youtube")]
            youtube_active_id: None,
            #[cfg(feature = "wasm-youtube")]
            youtube_active_url: None,
        })
    }

    // -----------------------------------------------------------------------
    // Main loop
    // -----------------------------------------------------------------------

    /// Advance the OS state by one frame.
    ///
    /// Call this from `requestAnimationFrame`. Processes input events,
    /// updates the scene graph, and renders to the canvas.
    pub fn tick(&mut self, _delta_seconds: f32) {
        self.frame_counter += 1;

        // Update system info every ~60 frames (~1s at 60fps).
        if self.frame_counter.is_multiple_of(60) {
            use oasis_core::platform::TimeService;
            let time = self.platform.now().ok();
            self.status_bar.update_info(time.as_ref(), None);
        }

        // Process queued input events.
        let events = self.input.poll_events();
        for event in &events {
            self.mouse_cursor.handle_input(event);
            match self.mode {
                Mode::Osk => self.handle_osk_input(event),
                Mode::Desktop => self.handle_desktop_input(event),
                Mode::App => self.handle_app_input(event),
                _ => self.handle_default_input(event),
            }
        }

        // Process pending VFS requests from app runners (e.g. radio tune).
        // Skip TV Guide tune requests — they're handled by the dedicated video
        // overlay section below.
        {
            let mut pending = None;
            if let Some(ref mut runner) = self.app_runner
                && !vfs_content::is_tv_tune_request_wasm(runner)
            {
                pending = runner.take_pending_request();
            }
            if pending.is_none() {
                for (_, runner) in &mut self.open_runners {
                    if vfs_content::is_tv_tune_request_wasm(runner) {
                        continue;
                    }
                    if let Some(req) = runner.take_pending_request() {
                        pending = Some(req);
                        break;
                    }
                }
            }
            if let Some((path, data)) = pending {
                #[cfg(feature = "wasm-youtube")]
                if path == oasis_core::apps::video_embed::VIDEO_EMBED_REQUEST_PATH {
                    self.handle_video_embed_request(&data);
                } else {
                    let _ = self.vfs.write(&path, data.as_bytes());
                }
                #[cfg(not(feature = "wasm-youtube"))]
                {
                    let _ = self.vfs.write(&path, data.as_bytes());
                }
            }

            // Drive the YouTube search fetcher and let the embed app
            // pick up freshly-published results from VFS.
            #[cfg(feature = "wasm-youtube")]
            {
                self.poll_youtube_search();
                if let Some(ref mut runner) = self.app_runner {
                    runner.refresh_video_embed(&self.vfs);
                }
                for (_, runner) in &mut self.open_runners {
                    runner.refresh_video_embed(&self.vfs);
                }
            }
        }

        // -- Radio processing --
        {
            use oasis_audio::RADIO_REQUEST_PATH;

            // Read radio requests from VFS.
            if self.vfs.exists(RADIO_REQUEST_PATH)
                && let Ok(data) = self.vfs.read(RADIO_REQUEST_PATH)
            {
                let request = String::from_utf8_lossy(&data).to_string();
                if !request.is_empty() {
                    let _ = self.vfs.write(RADIO_REQUEST_PATH, b"");

                    if let Some(target) = request.strip_prefix("tune ") {
                        let station = if let Ok(idx) = target.parse::<usize>() {
                            self.radio_manager.registry.stations.get(idx).cloned()
                        } else {
                            self.radio_manager
                                .registry
                                .stations
                                .iter()
                                .find(|s| s.name.eq_ignore_ascii_case(target.trim()))
                                .cloned()
                        };
                        if let Some(station) = station {
                            let _ = self.radio_manager.tune(
                                &station.name,
                                station.bitrate,
                                &mut self.audio,
                            );

                            // Clear stale catalog/pending fetches on station change.
                            self.archive_catalog = None;
                            self.pending_catalog = None;
                            // Disconnect old source so tick() doesn't keep
                            // polling the previous station while the new
                            // catalog is being fetched.
                            if let Some(mut old) = self.radio_source.take() {
                                old.disconnect();
                            }

                            self.radio_manager
                                .set_source_info(&station.source_type, &station.collection);

                            if station.source_type == "archive" && !station.collection.is_empty() {
                                self.pending_catalog =
                                    Some(archive::WasmArchiveCatalogFetcher::new(
                                        &station.collection,
                                        self.frame_counter,
                                    ));
                            }
                        } else {
                            self.radio_manager
                                .set_error(&format!("station not found: {target}"));
                        }
                    } else {
                        let _ = self
                            .radio_manager
                            .process_request(&request, &mut self.audio);
                        // Clear catalog on stop.
                        if self.radio_manager.state() == oasis_audio::radio::RadioState::Stopped {
                            self.archive_catalog = None;
                            self.pending_catalog = None;
                        }
                    }
                }
            }

            // Poll pending catalog fetch.
            if self.pending_catalog.as_ref().is_some_and(|f| f.is_ready())
                && let Some(fetcher) = self.pending_catalog.take()
            {
                match fetcher.take_results() {
                    Ok((catalog, source)) => {
                        if let Some(mut old) = self.radio_source.take() {
                            old.disconnect();
                        }
                        self.archive_catalog = Some(catalog);
                        self.radio_source = Some(source);
                    },
                    Err(e) => {
                        self.radio_manager.set_error(&e);
                    },
                }
            }

            // Drive the radio state machine.
            let _ = self
                .radio_manager
                .tick(&mut self.radio_source, &mut self.audio);

            // Auto-advance to next track for archive stations.
            if self.radio_manager.needs_next_track()
                && let Some(ref mut catalog) = self.archive_catalog
                && let Some(track) = catalog.next_track().cloned()
            {
                if let Some(mut old) = self.radio_source.take() {
                    old.disconnect();
                }
                let url = oasis_audio::radio::ArchiveCatalog::download_url(&track);
                let source = archive::WasmArchiveSource::new(&url, &track.title, &track.creator);
                self.radio_source = Some(Box::new(source));
                self.radio_manager.continue_playing();
            }

            // Publish radio status periodically.
            if self.frame_counter.is_multiple_of(15) {
                let _ = self.radio_manager.publish_status(&mut self.vfs);
            }

            // Refresh radio app display if visible.
            if let Some(ref mut runner) = self.app_runner {
                runner.refresh_radio(&self.vfs);
            }
            for (_, runner) in &mut self.open_runners {
                runner.refresh_radio(&self.vfs);
            }

            // Poll pending TV catalog fetch.
            if self
                .pending_tv_catalog
                .as_ref()
                .is_some_and(|f| f.is_ready())
                && let Some(fetcher) = self.pending_tv_catalog.take()
            {
                match fetcher.take_results() {
                    Ok(catalogs) => {
                        let runner = vfs_content::find_tv_guide_runner_wasm(
                            &mut self.app_runner,
                            &mut self.open_runners,
                        );
                        if let Some(runner) = runner {
                            if let Some(guide) = runner.tv_guide_state() {
                                guide.fetch_in_progress = false;
                                let all_none = catalogs.iter().all(|c| c.is_none());
                                for (i, cat) in catalogs.into_iter().enumerate() {
                                    if let Some(c) = cat
                                        && i < guide.catalogs.len()
                                    {
                                        guide.catalogs[i] = Some(c);
                                        guide.rebuild_cached_schedule(i);
                                    }
                                }
                                if all_none {
                                    guide.fetch_error =
                                        Some("No episodes found for any channel".into());
                                }
                            }
                            runner.refresh_tv_text();
                        }
                    },
                    Err(e) => {
                        console_log!("TV catalog fetch failed: {e}");
                        let runner = vfs_content::find_tv_guide_runner_wasm(
                            &mut self.app_runner,
                            &mut self.open_runners,
                        );
                        if let Some(runner) = runner {
                            if let Some(guide) = runner.tv_guide_state() {
                                guide.fetch_in_progress = false;
                                guide.fetch_error = Some(e);
                            }
                            runner.refresh_tv_text();
                        }
                    },
                }
            }

            // Start TV catalog fetch if a TV Guide app needs it.
            if self.pending_tv_catalog.is_none() {
                let runner = vfs_content::find_tv_guide_runner_wasm(
                    &mut self.app_runner,
                    &mut self.open_runners,
                );
                if let Some(runner) = runner
                    && let Some(guide) = runner.tv_guide_state()
                    && !guide.fetch_attempted
                    && guide.catalogs.iter().all(|c| c.is_none())
                {
                    guide.fetch_attempted = true;
                    guide.fetch_in_progress = true;
                    self.pending_tv_catalog =
                        Some(tv_catalog::WasmTvCatalogFetcher::new(&guide.channels));
                }
            }

            // Handle TV Guide tune requests via VFS IPC.
            {
                let runner = vfs_content::find_tv_guide_runner_wasm(
                    &mut self.app_runner,
                    &mut self.open_runners,
                );
                if let Some(runner) = runner
                    && let Some((path, data)) = runner.take_pending_request()
                {
                    if path == oasis_core::apps::tv_guide::TV_REQUEST_PATH
                        && data.starts_with("tune_url ")
                    {
                        // Format: "tune_url {url} {seek_secs}"
                        let rest = &data["tune_url ".len()..];
                        let (url, seek_secs) = rest
                            .rsplit_once(' ')
                            .map(|(u, s)| (u, s.parse::<u64>().unwrap_or(0)))
                            .unwrap_or((rest, 0));
                        if !url.is_empty() {
                            console_log!(
                                "TV tune: {} seek={}s",
                                &url[..url.len().min(80)],
                                seek_secs,
                            );
                            let (_, _, pw, ph) = vfs_content::tv_preview_rect(&self.active_theme);
                            let tex_id =
                                self.video_player
                                    .start(url, seek_secs, pw, ph, &mut self.backend);
                            // Assign the texture to the guide so its
                            // existing rendering code displays it.
                            if let Some(guide) = runner.tv_guide_state() {
                                guide.preview_texture = tex_id;
                                console_log!("TV tune: texture={:?} preview={}x{}", tex_id, pw, ph,);
                            }
                        }
                    } else {
                        let _ = self.vfs.write(&path, data.as_bytes());
                    }
                }
            }
        }

        // Detect untune: video is playing but guide has no tuned channel.
        if self.video_player.is_active() {
            let should_stop = {
                let runner = vfs_content::find_tv_guide_runner_wasm(
                    &mut self.app_runner,
                    &mut self.open_runners,
                );
                match runner {
                    Some(runner) => runner
                        .tv_guide_state()
                        .is_none_or(|g| g.tuned_channel.is_none()),
                    None => true, // TV Guide closed.
                }
            };
            if should_stop {
                self.video_player.stop(&mut self.backend);
            }
        }

        // Sync volume from guide state to the video element.
        if self.video_player.is_active() {
            let runner = vfs_content::find_tv_guide_runner_wasm(
                &mut self.app_runner,
                &mut self.open_runners,
            );
            if let Some(runner) = runner
                && let Some(guide) = runner.tv_guide_state()
                && guide.volume_changed
            {
                self.video_player.set_volume(guide.volume as f64 / 100.0);
                guide.volume_changed = false;
            }
        }

        // Auto-advance to next episode when the current video ends.
        if self.video_player.is_ended() {
            self.auto_advance_episode();
        }

        // Capture the latest video frame onto the texture canvas.
        self.video_player.tick();

        // Update SDI scene graph for the active mode.
        self.update_sdi();

        // Drive browser image streaming.
        if let Some(ref mut bw) = self.browser {
            bw.tick(&self.vfs);
        }

        // -- Render --
        // Hide cursor from SDI so window content doesn't draw under it;
        // we blit the cursor manually after all rendering.
        CursorState::hide_sdi(&mut self.sdi);

        if let Err(e) = self.backend.clear(self.bg_color) {
            console_log!("clear error: {e}");
        }

        // Render shader wallpaper FIRST (replaces bg_color clear).
        // Runs every frame so the animation stays live in all modes.
        if let Some(ref mut bridge) = self.shader_bridge
            && let Some(info) = oasis_core::vector_overlay::get_shader_layer(&self.active_theme)
        {
            bridge.render_frame(
                &info.name,
                self.frame_counter as f32 / 60.0,
                &info.params,
                self.backend.ctx(),
            );
        }

        // Pre-draw: hide the YouTube iframe if its window is minimized or
        // gone. Windows in `Minimized` state are not visited by
        // `draw_with_clips_overlay`, so without this the iframe would
        // stay glued to its last position even though the canvas has
        // already cleared the area. Use `soft_hide` for the minimize
        // case so playback state survives until the user restores the
        // window; full `hide` is reserved for stop/close.
        #[cfg(feature = "wasm-youtube")]
        if self.youtube_active_id.is_some() {
            match self.wm.get_window("video_embed") {
                Some(w) if w.state == oasis_core::wm::window::WindowState::Minimized => {
                    self.iframe.soft_hide();
                },
                None => {
                    self.iframe.hide();
                },
                _ => {},
            }
        }

        if self.mode == Mode::Desktop && self.wm.window_count() > 0 {
            let browser = &mut self.browser;
            let iframe_ref = &mut self.iframe;
            let open_runners = &self.open_runners;
            let active_theme = &self.active_theme;
            let dashboard = &self.dashboard;
            let frame = self.frame_counter as u32;
            #[cfg(feature = "wasm-youtube")]
            let youtube_active_url = self.youtube_active_url.clone();
            let wants_vector_icons =
                self.skin.features.dashboard && active_theme.icon.style == "vector";
            let overlay =
                |be: &mut dyn oasis_core::backend::SdiBackend| -> oasis_core::error::Result<()> {
                    if wants_vector_icons {
                        dashboard.render_vector_icons(be, active_theme, frame)?;
                    }
                    Ok(())
                };
            if let Err(e) = self.wm.draw_with_clips_overlay(
                &mut self.sdi,
                &mut self.backend,
                overlay,
                |window_id, cx, cy, cw, ch, be| {
                    let result = if window_id == "browser" {
                        if let Some(ref mut bw) = *browser {
                            bw.set_window(cx, cy, cw, ch);
                            let url = bw.current_url().map(|s| s.to_string());
                            let is_http = url.as_ref().is_some_and(|u| {
                                u.starts_with("http://") || u.starts_with("https://")
                            });
                            if is_http && let Some(ref u) = url {
                                let url_bar_h = bw.config.url_bar_height;
                                let status_bar_h = bw.config.status_bar_height;
                                let content_y = cy + url_bar_h as i32;
                                let content_h = ch.saturating_sub(url_bar_h + status_bar_h);
                                iframe_ref.show(u, cx, content_y, cw, content_h);
                                bw.paint_chrome_only(be)
                            } else {
                                iframe_ref.hide();
                                bw.paint(be)
                            }
                        } else {
                            iframe_ref.hide();
                            Ok(())
                        }
                    } else if let Some((_, runner)) =
                        open_runners.iter().find(|(id, _)| id == window_id)
                    {
                        let r = runner.draw_windowed(cx, cy, cw, ch, be, active_theme);
                        // YouTube iframe: glue to the Video Embed window's
                        // content rect every frame so drag/resize tracks.
                        #[cfg(feature = "wasm-youtube")]
                        if window_id == "video_embed"
                            && let Some(ref url) = youtube_active_url
                        {
                            let title_h = active_theme.app.title_bar_height as i32;
                            let inner_y = cy + title_h;
                            let inner_h = ch.saturating_sub(active_theme.app.title_bar_height);
                            iframe_ref.show(url, cx, inner_y, cw, inner_h);
                        }
                        r
                    } else {
                        Ok(())
                    };
                    if let Err(ref e) = result {
                        web_sys::console::error_1(
                            &format!("window '{window_id}' content error: {e}").into(),
                        );
                    }
                    result
                },
            ) {
                console_log!("draw_with_clips error: {e}");
            }
        } else {
            // Not in desktop mode or no windows — hide iframe.
            self.iframe.hide();
            if self.mode == Mode::Dashboard
                && (self.active_theme.icon.style == "vector"
                    || !self.active_theme.background_layers.is_empty())
            {
                // Shader already rendered above as wallpaper.
                if let Err(e) = self.sdi.draw_base_layer(&mut self.backend) {
                    console_log!("sdi draw_base error: {e}");
                }

                let _ = oasis_core::vector_overlay::render_vector_background(
                    &mut self.backend,
                    &self.active_theme,
                    self.frame_counter as u32,
                );
                let _ = self.dashboard.render_vector_icons(
                    &mut self.backend,
                    &self.active_theme,
                    self.frame_counter as u32,
                );
                if let Err(e) = self.sdi.draw_overlay_layer(&mut self.backend) {
                    console_log!("sdi draw_overlay error: {e}");
                }
            } else if let Err(e) = self.sdi.draw(&mut self.backend) {
                console_log!("sdi draw error: {e}");
            }
        }

        // Paint terminal scrollbar when in terminal mode.
        if self.mode == Mode::Terminal
            && let Err(e) = terminal_sdi::paint_terminal_scrollbar(
                &mut self.backend,
                self.output_lines.len(),
                self.terminal_scroll_offset,
                &self.active_theme,
            )
        {
            console_log!("terminal scrollbar error: {e}");
        }

        // Draw transition overlay if active.
        if let Some(ref mut trans) = self.active_transition {
            if let Err(e) = trans.draw_overlay(&mut self.backend) {
                console_log!("transition overlay error: {e}");
            }
            trans.tick();
            if trans.is_done() {
                self.active_transition = None;
            }
        }

        // Draw cursor on top of everything (after windows, scrollbar,
        // transition overlay). Hide it while the pointer is over the
        // iframe overlay or off the page — the canvas stops receiving
        // mousemove events there so the cursor would otherwise freeze
        // at its last tracked position, looking trapped.
        if self.mouse_cursor.visible
            && self.input.pointer_on_canvas()
            && let Some(tex) = self.cursor_texture
            && let Err(e) = self.backend.blit(
                tex,
                self.mouse_cursor.x,
                self.mouse_cursor.y,
                12 * self.mouse_cursor.scale,
                18 * self.mouse_cursor.scale,
            )
        {
            console_log!("cursor blit error: {e}");
        }

        if let Err(e) = self.backend.swap_buffers() {
            console_log!("swap_buffers error: {e}");
        }
    }

    // -----------------------------------------------------------------------
    // TV Guide auto-advance
    // -----------------------------------------------------------------------

    /// When the current episode's `<video>` element fires `ended`, stop the
    /// player and tune to whatever the deterministic schedule says should be
    /// Handle a request from the Video Embed app: `search:<q>`,
    /// `play:<id>`, `stop`, or empty (alias for `stop`). Search kicks off
    /// the async fetcher; play+stop record the desired iframe state — the
    /// per-window draw callback applies it so dragging/resizing the
    /// containing window keeps the iframe glued in place.
    #[cfg(feature = "wasm-youtube")]
    fn handle_video_embed_request(&mut self, data: &str) {
        if data.is_empty() || data == "stop" {
            self.youtube_active_id = None;
            self.youtube_active_url = None;
            self.iframe.hide();
            return;
        }

        if let Some(query) = data.strip_prefix("search:") {
            self.kick_youtube_search(query);
            return;
        }

        if let Some(id) = data.strip_prefix("play:") {
            self.youtube_active_url = Some(oasis_core::apps::video_embed::embed_url(id));
            self.youtube_active_id = Some(id.to_string());
            self.iframe.set_youtube_mode();
        }
    }

    /// Free thumbnail textures from the previous search and start a new
    /// fetcher. Idempotent — calling repeatedly with the same query
    /// just discards the in-flight fetch.
    #[cfg(feature = "wasm-youtube")]
    fn kick_youtube_search(&mut self, query: &str) {
        for tex in self.youtube_thumb_textures.drain(..) {
            let _ = self.backend.destroy_texture(tex);
        }
        // Publish a "loading" placeholder immediately so the app's next
        // refresh() reflects in-flight state instead of stale results.
        let pending = oasis_core::apps::video_embed::SearchResults {
            query: query.to_string(),
            status: oasis_core::apps::video_embed::SearchStatus::Loading,
            error: None,
            results: Vec::new(),
        };
        if let Ok(json) = serde_json::to_string(&pending) {
            let _ = self.vfs.write(
                oasis_core::apps::video_embed::VIDEO_EMBED_RESULTS_PATH,
                json.as_bytes(),
            );
        }

        self.pending_youtube_search = Some(youtube::WasmYoutubeSearchFetcher::new(query));
    }

    /// Poll the YouTube search fetcher; when ready, allocate textures
    /// for thumbnails, kick off image loads, and publish the result blob
    /// over VFS IPC.
    #[cfg(feature = "wasm-youtube")]
    fn poll_youtube_search(&mut self) {
        let Some(fetcher) = self.pending_youtube_search.as_ref() else {
            return;
        };
        if !fetcher.is_ready() {
            return;
        }

        let query = fetcher.query.clone();
        let result = fetcher.take_results();
        self.pending_youtube_search = None;

        let blob = match result {
            Ok(hits) => {
                let (tw, th) = youtube::thumb_dimensions();
                let mut out = Vec::with_capacity(hits.len());
                for hit in hits {
                    let (tex, canvas) = match self.backend.allocate_paintable_texture(tw, th) {
                        Ok(p) => p,
                        Err(e) => {
                            console_log!("youtube: alloc texture: {e}");
                            continue;
                        },
                    };
                    self.youtube_thumb_textures.push(tex);
                    if let Err(e) = youtube::paint_canvas_from_url(canvas, &hit.thumbnail_url) {
                        console_log!("youtube: img load: {e:?}");
                    }
                    out.push(oasis_core::apps::video_embed::SearchResult {
                        id: hit.video_id,
                        title: hit.title,
                        author: hit.author,
                        duration: hit.duration,
                        thumb_tex: tex.0,
                    });
                }
                oasis_core::apps::video_embed::SearchResults {
                    query,
                    status: oasis_core::apps::video_embed::SearchStatus::Ready,
                    error: None,
                    results: out,
                }
            },
            Err(e) => oasis_core::apps::video_embed::SearchResults {
                query,
                status: oasis_core::apps::video_embed::SearchStatus::Error,
                error: Some(e),
                results: Vec::new(),
            },
        };

        if let Ok(json) = serde_json::to_string(&blob) {
            let _ = self.vfs.write(
                oasis_core::apps::video_embed::VIDEO_EMBED_RESULTS_PATH,
                json.as_bytes(),
            );
        }
    }

    /// playing now (which will be the next episode).
    fn auto_advance_episode(&mut self) {
        // Gather schedule info from the guide state, then release the borrow.
        let tune_data = {
            let runner = vfs_content::find_tv_guide_runner_wasm(
                &mut self.app_runner,
                &mut self.open_runners,
            );
            let Some(runner) = runner else { return };
            let Some(guide) = runner.tv_guide_state() else {
                return;
            };
            let Some(channel_idx) = guide.tuned_channel else {
                return;
            };

            // Clean up the guide's texture reference.
            guide.preview_texture = None;

            // Update the guide's clock so schedule_at returns the current slot.
            let now = (js_sys::Date::now() / 1000.0) as u64;
            guide.current_time = now;

            let catalog = guide.catalogs.get(channel_idx).and_then(|c| c.as_ref());
            let Some(catalog) = catalog else { return };

            // If the current slot has <5s remaining, skip ahead to avoid
            // re-tuning to the same nearly-finished episode.
            let query_time = {
                let Some(slot) = oasis_core::apps::tv_guide::schedule_at(catalog, now) else {
                    return;
                };
                if slot.remaining_secs < 5 {
                    now + slot.remaining_secs + 1
                } else {
                    now
                }
            };
            let Some(slot) = oasis_core::apps::tv_guide::schedule_at(catalog, query_time) else {
                return;
            };

            let url =
                oasis_core::apps::tv_guide::catalog::ChannelCatalog::download_url(&slot.episode);
            let seek_secs = slot.elapsed_secs;
            console_log!(
                "TV auto-advance -> {} (seek={}s, remaining={}s)",
                slot.episode.title,
                seek_secs,
                slot.remaining_secs,
            );
            format!("tune_url {url} {seek_secs}")
        };

        // Stop the finished player (needs &mut self.video_player + backend).
        self.video_player.stop(&mut self.backend);

        // Write tune request via VFS IPC (same path as manual tune).
        let runner =
            vfs_content::find_tv_guide_runner_wasm(&mut self.app_runner, &mut self.open_runners);
        if let Some(runner) = runner {
            runner.set_pending_request(
                oasis_core::apps::tv_guide::TV_REQUEST_PATH.to_string(),
                tune_data,
            );
        }
    }

    // -----------------------------------------------------------------------
    // SDI scene graph update (mirrors oasis-app render.rs)
    // -----------------------------------------------------------------------

    fn update_sdi(&mut self) {
        // Advance animations each frame.
        self.dashboard.tick_animation();
        self.start_menu.tick_animation();
        self.bottom_bar.tick_animation(&self.active_theme);
        self.toasts.tick();

        match self.mode {
            Mode::Dashboard => {
                terminal_sdi::set_terminal_visible(&mut self.sdi, false);
                AppRunner::hide_sdi(&mut self.sdi);
                self.taskbar.hide_sdi(&mut self.sdi);

                if self.bottom_bar.active_tab == MediaTab::None {
                    self.dashboard.update_sdi(&mut self.sdi, &self.active_theme);
                    terminal_sdi::hide_media_page(&mut self.sdi);
                } else {
                    self.dashboard.hide_sdi(&mut self.sdi);
                    terminal_sdi::update_media_page(
                        &mut self.sdi,
                        &self.bottom_bar,
                        &self.active_theme,
                    );
                }

                self.status_bar
                    .update_sdi(&mut self.sdi, &self.active_theme, &self.skin.features);
                self.bottom_bar
                    .update_sdi(&mut self.sdi, &self.active_theme, &self.skin.features);
                if self.skin.features.start_menu {
                    self.start_menu
                        .update_sdi(&mut self.sdi, &self.active_theme);
                }
            },
            Mode::Terminal => {
                self.dashboard.hide_sdi(&mut self.sdi);
                AppRunner::hide_sdi(&mut self.sdi);
                StatusBar::hide_sdi(&mut self.sdi);
                BottomBar::hide_sdi(&mut self.sdi);
                self.taskbar.hide_sdi(&mut self.sdi);
                self.start_menu.close();
                self.start_menu.hide_sdi(&mut self.sdi);
                terminal_sdi::hide_media_page(&mut self.sdi);
                let cursor_visible = self.active_theme.terminal_cursor_blink_rate == 0
                    || (self.frame_counter / self.active_theme.terminal_cursor_blink_rate as u64)
                        .is_multiple_of(2);
                terminal_sdi::setup_terminal_objects(
                    &mut self.sdi,
                    &self.output_lines,
                    &self.cwd,
                    &self.input_buf,
                    self.terminal_scroll_offset,
                    &self.active_theme,
                    cursor_visible,
                );
            },
            Mode::App => {
                self.dashboard.hide_sdi(&mut self.sdi);
                terminal_sdi::set_terminal_visible(&mut self.sdi, false);
                terminal_sdi::hide_media_page(&mut self.sdi);
                self.taskbar.hide_sdi(&mut self.sdi);
                self.start_menu.close();
                self.start_menu.hide_sdi(&mut self.sdi);
                self.status_bar
                    .update_sdi(&mut self.sdi, &self.active_theme, &self.skin.features);
                self.bottom_bar
                    .update_sdi(&mut self.sdi, &self.active_theme, &self.skin.features);
                if let Some(ref mut runner) = self.app_runner {
                    runner.update_sdi(&mut self.sdi, &self.active_theme);
                }
            },
            Mode::Desktop => {
                terminal_sdi::set_terminal_visible(&mut self.sdi, false);
                AppRunner::hide_sdi(&mut self.sdi);
                terminal_sdi::hide_media_page(&mut self.sdi);

                // Sync terminal output to the windowed terminal runner.
                if let Some((_, runner)) = self
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| id == "terminal")
                {
                    let mut lines = self.output_lines.clone();
                    let prompt = format!("> {}", self.input_buf);
                    lines.push(prompt);
                    runner.set_lines(lines, self.terminal_scroll_offset);
                }

                // Keep dashboard icons visible behind windows.
                if self.bottom_bar.active_tab == MediaTab::None {
                    self.dashboard.update_sdi(&mut self.sdi, &self.active_theme);
                } else {
                    self.dashboard.hide_sdi(&mut self.sdi);
                }
                if self.fullscreen_app.is_some() {
                    StatusBar::hide_sdi(&mut self.sdi);
                    BottomBar::hide_sdi(&mut self.sdi);
                    self.taskbar.hide_sdi(&mut self.sdi);
                    self.start_menu.close();
                    self.start_menu.hide_sdi(&mut self.sdi);
                    if let Ok(obj) = self.sdi.get_mut("wallpaper") {
                        obj.visible = false;
                    }
                } else {
                    self.status_bar.update_sdi(
                        &mut self.sdi,
                        &self.active_theme,
                        &self.skin.features,
                    );
                    self.bottom_bar.update_sdi(
                        &mut self.sdi,
                        &self.active_theme,
                        &self.skin.features,
                    );
                    self.taskbar.update_sdi(
                        &mut self.sdi,
                        &self.active_theme,
                        self.wm.windows(),
                        self.wm.active_window(),
                        self.skin.features.start_menu,
                    );
                    self.taskbar.update_desktop_indicator(
                        &mut self.sdi,
                        &self.active_theme,
                        self.desktops.active_desktop(),
                        self.desktops.desktop_count(),
                    );
                    if self.skin.features.start_menu {
                        self.start_menu
                            .update_sdi(&mut self.sdi, &self.active_theme);
                    }
                }
            },
            Mode::Osk => {
                if let Some(ref mut osk_state) = self.osk {
                    osk_state.tick_animation();
                    osk_state.update_sdi(&mut self.sdi, &self.active_theme);
                }
            },
        }

        // Update toast overlays (visible in Dashboard, App, Desktop modes).
        match self.mode {
            Mode::Dashboard | Mode::App | Mode::Desktop => {
                self.toasts.update_sdi(&mut self.sdi, &self.active_theme);
            },
            _ => {
                ToastManager::hide_sdi(&mut self.sdi);
            },
        }

        // Update cursor SDI position (always on top).
        self.mouse_cursor.update_sdi(&mut self.sdi);

        // Ensure wallpaper is visible and at lowest z (skip during fullscreen kiosk
        // where we explicitly hide it to prevent bleed-through, and skip when a
        // shader layer replaces the wallpaper).
        let fullscreen_active = self.mode == Mode::Desktop && self.fullscreen_app.is_some();
        let shader_active =
            oasis_core::vector_overlay::get_shader_layer(&self.active_theme).is_some();
        if !fullscreen_active
            && !shader_active
            && let Ok(obj) = self.sdi.get_mut("wallpaper")
        {
            obj.visible = true;
        }
        // Hide opaque content_bg when shader provides the background.
        if shader_active && let Ok(obj) = self.sdi.get_mut("content_bg") {
            obj.visible = false;
        }
    }

    // -----------------------------------------------------------------------
    // App launching (mirrors oasis-app launch.rs)
    // -----------------------------------------------------------------------

    fn launch_app_window(&mut self, app: &AppEntry) {
        // Terminal: fullscreen mode for non-WM skins; windowed for WM skins.
        if app.title == "Terminal" && !self.skin.features.window_manager {
            self.mode = Mode::Terminal;
            self.active_transition = Some(self.make_transition());
            return;
        }

        // Scale window dimensions proportionally to screen resolution.
        let win_w = (self.width * 380 + 240) / 480;
        let win_h = (self.height * 220 + 136) / 272;

        if app.title == "Browser" {
            let win_id = "browser";
            if self.wm.get_window(win_id).is_some() {
                let _ = self.wm.focus_window(win_id, &mut self.sdi);
            } else {
                let wc = WindowConfig {
                    id: win_id.to_string(),
                    title: "Browser".to_string(),
                    x: None,
                    y: None,
                    width: win_w,
                    height: win_h,
                    window_type: WindowType::AppWindow,
                    always_on_top: false,
                    modal: false,
                };
                let _ = self.wm.create_window(&wc, &mut self.sdi);
                let mut bw = BrowserWidget::new(self.browser_config.clone());
                bw.set_window(0, 0, win_w, win_h);
                let home = bw.config.features.home_url.clone();
                bw.navigate_vfs(&home, &self.vfs);
                self.browser = Some(bw);
            }
            self.mode = Mode::Desktop;
            self.active_transition = Some(self.make_transition());
            return;
        }

        let win_id = app.title.to_lowercase().replace(' ', "_");
        if self.wm.get_window(&win_id).is_some() {
            let _ = self.wm.focus_window(&win_id, &mut self.sdi);
        } else {
            let wc = WindowConfig {
                id: win_id.clone(),
                title: app.title.clone(),
                x: None,
                y: None,
                width: win_w,
                height: win_h,
                window_type: WindowType::AppWindow,
                always_on_top: false,
                modal: false,
            };
            let _ = self.wm.create_window(&wc, &mut self.sdi);
            self.open_runners
                .push((win_id, AppRunner::launch(app, &self.vfs)));
        }
        self.mode = Mode::Desktop;
        self.active_transition = Some(self.make_transition());
    }

    /// Launch an app window pre-loaded with a file. Mirrors
    /// [`launch_app_window`] but calls `AppRunner::launch_with_file` so
    /// File Manager's Confirm-on-typed-file flow can hand off to
    /// Photo Viewer / Music Player / Text Editor under the WASM backend.
    fn launch_app_window_for_file(&mut self, app_title: &str, file_path: &str) {
        let win_w = (self.width * 380 + 240) / 480;
        let win_h = (self.height * 220 + 136) / 272;
        let win_id = app_title.to_lowercase().replace(' ', "_");
        let entry = AppEntry {
            title: app_title.to_string(),
            path: format!("/apps/{app_title}"),
            icon_png: Vec::new(),
            color: oasis_core::backend::Color::rgb(100, 100, 100),
        };

        if self.wm.get_window(&win_id).is_some() {
            let _ = self.wm.focus_window(&win_id, &mut self.sdi);
            if let Some(slot) = self.open_runners.iter_mut().find(|(id, _)| *id == win_id) {
                let new_runner = AppRunner::launch_with_file(&entry, file_path, &self.vfs);
                // Transfer pending Photo Viewer GPU textures so they
                // don't leak when the outgoing runner is dropped.
                if let (Some(old_app), Some(new_app)) = (
                    slot.1.delegate_as::<oasis_app_media::BrowsingApp>(),
                    new_runner.delegate_as::<oasis_app_media::BrowsingApp>(),
                ) {
                    new_app.inherit_textures_from(old_app);
                }
                slot.1 = new_runner;
            } else {
                self.open_runners.push((
                    win_id,
                    AppRunner::launch_with_file(&entry, file_path, &self.vfs),
                ));
            }
            return;
        }

        let wc = WindowConfig {
            id: win_id.clone(),
            title: app_title.to_string(),
            x: None,
            y: None,
            width: win_w,
            height: win_h,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        };
        let _ = self.wm.create_window(&wc, &mut self.sdi);
        self.open_runners.push((
            win_id,
            AppRunner::launch_with_file(&entry, file_path, &self.vfs),
        ));
    }

    fn make_transition(&self) -> TransitionState {
        transition::fade_in_custom(
            self.width,
            self.height,
            self.skin.features.transition_fade_frames.unwrap_or(15),
        )
    }

    // -----------------------------------------------------------------------
    // Start menu actions
    // -----------------------------------------------------------------------

    fn handle_start_menu_action(&mut self, action: &StartMenuAction) {
        match action {
            StartMenuAction::LaunchApp(title) => {
                let app = self
                    .dashboard
                    .apps
                    .iter()
                    .find(|a| a.title == *title)
                    .cloned();
                if let Some(app) = app {
                    self.launch_app_window(&app);
                }
            },
            StartMenuAction::OpenTerminal => {
                self.mode = Mode::Terminal;
            },
            StartMenuAction::Exit | StartMenuAction::RunCommand(_) | StartMenuAction::None => {},
        }
    }

    // -----------------------------------------------------------------------
    // Terminal command execution
    // -----------------------------------------------------------------------

    fn execute_terminal_command(&mut self, line: &str) {
        let pending_skin_swap;
        {
            let mut env = Environment {
                cwd: self.cwd.clone(),
                vfs: &mut self.vfs,
                power: Some(&self.platform),
                time: Some(&self.platform),
                usb: Some(&self.platform),
                network: Some(&self.platform),
                tls: None,
                stdin: None,
                stderr: String::new(),
            };
            let result = self.cmd_reg.execute(line, &mut env);
            self.cwd = env.cwd;
            pending_skin_swap = self.process_command_output(result);
        }
        if let Some(name) = pending_skin_swap {
            self.apply_skin_swap(&name);
        }
    }

    /// Process a command result. Returns a pending skin swap name if applicable.
    fn process_command_output(
        &mut self,
        result: oasis_core::error::Result<CommandOutput>,
    ) -> Option<String> {
        match result {
            Ok(CommandOutput::Text(text)) => {
                for l in text.lines() {
                    self.output_lines.push(l.to_string());
                }
            },
            Ok(CommandOutput::Table { headers, rows }) => {
                self.output_lines.push(headers.join(" | "));
                for row in &rows {
                    self.output_lines.push(row.join(" | "));
                }
            },
            Ok(CommandOutput::Clear) => self.output_lines.clear(),
            Ok(CommandOutput::None) => {},
            Ok(CommandOutput::Signal(ref sig)) => {
                use oasis_core::terminal::CommandSignal;
                match sig {
                    CommandSignal::BrowserSandbox { enable } => {
                        let enable = *enable;
                        if let Some(ref mut bw) = self.browser {
                            bw.config.features.sandbox_only = enable;
                        }
                        let st = if enable {
                            "on (VFS only)"
                        } else {
                            "off (HTTP enabled)"
                        };
                        self.output_lines.push(format!("Browser sandbox: {st}"));
                    },
                    CommandSignal::SkinSwap { name } => {
                        return Some(name.clone());
                    },
                    CommandSignal::ListenToggle { .. }
                    | CommandSignal::RemoteConnect { .. }
                    | CommandSignal::FtpToggle { .. } => {
                        self.output_lines
                            .push("Not available in browser.".to_string());
                    },
                }
            },
            Ok(CommandOutput::Multi(outputs)) => {
                let mut skin_swap = None;
                for output in outputs {
                    let result = self.process_command_output(Ok(output));
                    if result.is_some() {
                        skin_swap = result;
                    }
                }
                return skin_swap;
            },
            Err(e) => {
                self.output_lines.push(format!("error: {e}"));
            },
        }
        None
    }

    /// Apply a skin swap.
    fn apply_skin_swap(&mut self, name: &str) {
        match oasis_skin::resolve_skin(name) {
            Ok(new_skin) => {
                let swapped =
                    Skin::swap_scaled(&self.skin, new_skin, &mut self.sdi, self.width, self.height);
                self.active_theme = ActiveTheme::from_skin(&swapped.theme)
                    .with_screen_size(self.width, self.height)
                    .with_features(&swapped.features);
                self.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                self.wm.set_theme(swapped.theme.build_wm_theme());
                let dash_config =
                    DashboardConfig::from_features(&swapped.features, &self.active_theme);
                let mut apps = discover_apps(&self.vfs, "/apps", None).unwrap_or_default();
                #[cfg(feature = "wasm-youtube")]
                apps.push(AppEntry {
                    title: "Video Embed".to_string(),
                    path: "/apps/video-embed".to_string(),
                    icon_png: Vec::new(),
                    color: Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                });
                self.dashboard = DashboardState::new(dash_config, apps);
                self.bottom_bar.total_pages = self.dashboard.page_count();
                self.bottom_bar.current_page = 0;
                self.start_menu = StartMenuState::new_with_theme(
                    StartMenuState::default_items(&self.active_theme),
                    &self.active_theme,
                );
                self.output_lines
                    .push(format!("Switched to skin: {}", swapped.manifest.name));
                self.skin = swapped;
            },
            Err(e) => {
                self.output_lines.push(format!("Skin error: {e}"));
            },
        }
    }

    // -----------------------------------------------------------------------
    // Public API (JS-facing)
    // -----------------------------------------------------------------------

    /// Execute a terminal command and return the output.
    ///
    /// Also pushes output to the in-canvas terminal display.
    pub fn send_command(&mut self, cmd: &str) -> String {
        self.output_lines.push(format!("> {cmd}"));
        let mut env = Environment {
            cwd: self.cwd.clone(),
            vfs: &mut self.vfs,
            power: Some(&self.platform),
            time: Some(&self.platform),
            usb: Some(&self.platform),
            network: Some(&self.platform),
            tls: None,
            stdin: None,
            stderr: String::new(),
        };

        let result = self.cmd_reg.execute(cmd, &mut env);
        self.cwd = env.cwd;

        let output = match &result {
            Ok(CommandOutput::Text(t)) => t.clone(),
            Ok(CommandOutput::Clear) => String::new(),
            Ok(CommandOutput::Table { headers, rows }) => {
                let mut out = headers.join(" | ");
                for row in rows {
                    out.push('\n');
                    out.push_str(&row.join(" | "));
                }
                out
            },
            Err(e) => format!("error: {e}"),
            _ => String::new(),
        };

        let pending_skin_swap = self.process_command_output(result);
        if let Some(name) = pending_skin_swap {
            self.apply_skin_swap(&name);
        }
        vfs_content::trim_output(&mut self.output_lines);
        output
    }

    /// Read the current framebuffer as RGBA pixel data.
    ///
    /// Returns a flat `Vec<u8>` of length `width * height * 4` in RGBA order.
    /// Useful for screenshot capture from JavaScript (e.g. Playwright tests).
    pub fn read_pixels(&self) -> Vec<u8> {
        self.backend
            .read_pixels(0, 0, self.width, self.height)
            .unwrap_or_default()
    }

    /// Add a file to the in-memory VFS.
    pub fn add_vfs_file(&mut self, path: &str, data: &[u8]) {
        let _ = self.vfs.write(path, data);
    }

    /// Get the current screen width.
    pub fn screen_width(&self) -> u32 {
        self.width
    }

    /// Get the current screen height.
    pub fn screen_height(&self) -> u32 {
        self.height
    }

    /// Launch an app by title (e.g. "TV Guide", "Browser", "File Manager").
    pub fn launch_app(&mut self, title: &str) {
        let app = self
            .dashboard
            .apps
            .iter()
            .find(|a| a.title == title)
            .cloned();
        if let Some(app) = app {
            self.launch_app_window(&app);
        } else {
            log::warn!("App not found: {title}");
        }
    }
}
