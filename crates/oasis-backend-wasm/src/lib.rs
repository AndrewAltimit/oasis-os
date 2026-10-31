//! WebAssembly backend for OASIS_OS.
//!
//! Renders to an HTML `<canvas>` element using the Canvas 2D API,
//! maps DOM events to `InputEvent`, and plays audio through HTML `<audio>` elements.
//!
//! # Feature flags
//!
//! - `wasm-youtube` (default) — the YouTube search / embed app
//!   (`oasis-apps/video-embed`).

mod app_requests;
pub mod archive;
pub mod audio;
mod batch;
mod clip;
pub mod font;
mod gradients;
pub mod iframe;
pub mod input;
mod lru;
pub mod network;
pub mod platform;
pub mod renderer;
pub mod shader_bridge;
mod shapes;
mod skin_assets;
mod textures;
pub mod tv_catalog;
pub mod video;
#[cfg(feature = "wasm-youtube")]
pub mod youtube;

mod input_dispatch;
mod media;
mod vfs_content;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use oasis_apps::TvGuideRunnerExt as _;
use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::{AppRegistry, AppRunner, default_window_size, window_id};
use oasis_core::backend::{AudioBackend, Color, InputBackend, SdiCore, TextureId};
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{AppEntry, DashboardConfig, DashboardState, discover_apps};
use oasis_core::media_page;
use oasis_core::osk::OskState;
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::startmenu::{StartMenuAction, StartMenuState};
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{
    CommandOutput, CommandRegistry, Environment, ShellSession, register_builtins,
};
use oasis_core::terminal_sdi;
use oasis_core::toast::ToastManager;
use oasis_core::transition::{self, TransitionState};
use oasis_core::vfs::{MemoryVfs, Vfs};
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

// After console_log!, which the tick phases use.
mod tick;

/// Maximum consecutive radio failures before we stop the auto-advance
/// loop until the user manually picks a station again. Three is enough
/// to ride out a transient network blip but small enough that a
/// genuinely broken station (decoder mismatch, dead collection) doesn't
/// flood archive.org with catalog/files API hits.
const RADIO_FAILURE_LIMIT: u32 = 3;

/// Dashboard tiles: the apps installed under `/apps`, plus the YouTube
/// Video Embed app (feature `wasm-youtube`), which has no directory there.
fn dashboard_apps(vfs: &MemoryVfs, apps: &AppRegistry) -> Vec<AppEntry> {
    #[allow(unused_mut)]
    let mut tiles = discover_apps(vfs, "/apps", None).unwrap_or_default();
    #[cfg(feature = "wasm-youtube")]
    if let Some(reg) = apps.get(oasis_apps::titles::VIDEO_EMBED) {
        tiles.push(reg.to_app_entry());
    }
    #[cfg(not(feature = "wasm-youtube"))]
    let _ = apps;
    tiles
}

// ---------------------------------------------------------------------------
// Mode enum (mirrors oasis-app's AppState::Mode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Dashboard,
    Terminal,
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
    /// Launchable apps (the `oasis-apps` bundle).
    apps: AppRegistry,
    open_runners: Vec<(String, AppRunner)>,
    /// Requests drained from apps that closed or were replaced since the
    /// last tick; carried out ahead of the open apps' (see `app_requests`).
    closed_app_requests: Vec<oasis_core::apps::AppRequest>,
    /// Textures uploaded for the skin's layout `texture =` objects.
    skin_layout_textures: Vec<TextureId>,
    /// The skin's `kind = "image"` background decals (wordmarks, logos).
    image_layers: Vec<oasis_core::image_layers::ImageLayerObject>,
    /// Seconds of animation time, for image-layer drift / pulse.
    anim_secs: f32,
    browser: Option<BrowserWidget>,
    iframe: IframeOverlay,
    toasts: ToastManager,
    osk: Option<OskState>,
    active_transition: Option<TransitionState>,
    mode: Mode,
    cwd: String,
    /// Terminal line editor, tab completion and persistent history.
    shell: ShellSession,
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
    /// Circuit breaker: number of consecutive radio failures since the
    /// last user-initiated tune. Reset on every `tune <idx>` request,
    /// incremented on every error transition. When it reaches
    /// [`RADIO_FAILURE_LIMIT`] we stop the auto-advance loop until the
    /// user picks a station again — otherwise a station whose audio
    /// can't be decoded (e.g. Firefox MSE quirks) cascades into
    /// hundreds of catalog/track API requests in a few seconds.
    radio_consecutive_errors: u32,
    /// Music playlist + playback state behind `/var/audio/*` (the Music
    /// Player app and the `music` command), carried out by
    /// [`oasis_audio::MusicController`] in [`OasisWasm::tick_music`].
    media: oasis_audio::AudioManager,
    /// The loaded music track (an `<audio>` element in the audio backend).
    media_track: Option<oasis_core::backend::AudioTrackId>,
    video_player: video::VideoPlayer,
    pending_tv_catalog: Option<tv_catalog::WasmTvCatalogFetcher>,
    /// Next TV tune to start: a TV Guide
    /// `tune_url` request from the per-frame request drain, or an
    /// auto-advance. Consumed by the TV section of the same frame.
    pending_tv_tune: Option<oasis_core::ipc::TvTune>,
    shader_bridge: Option<shader_bridge::WasmShaderBridge>,
    /// Window id of the currently fullscreen-kiosk app (if any).
    fullscreen_app: Option<String>,
    /// Drops the gamepad-style twin of a key press already consumed as a
    /// shortcut or as typing (see `oasis_types::input::KeyTwinFilter`).
    key_filter: oasis_core::input::KeyTwinFilter,
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
        // Startup path: an unknown skin name falls back to classic (logged).
        let skin = oasis_skin::resolve_skin_or_classic(skin_ref)
            .map_err(|e| JsValue::from_str(&format!("skin: {e}")))?;

        // Use the skin's declared resolution (e.g. classic=480x272,
        // desktop=800x600, xp=1024x768) so the WASM build matches the
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
        oasis_apps::register_commands(&mut cmd_reg);
        let apps = oasis_apps::builtin_registry(&oasis_apps::HostInfo::new(
            "Browser (WASM)",
            "WASM Canvas 2D",
        ));

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
        let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
        let dashboard = DashboardState::new(dash_config, dashboard_apps(&vfs, &apps));

        // Bars, start menu.
        let mut bottom_bar = BottomBar::new();
        bottom_bar.total_pages = dashboard.page_count();
        let start_menu = StartMenuState::new_with_theme(
            StartMenuState::default_items(&active_theme),
            &active_theme,
        );

        // Window manager.
        let wm = WindowManager::with_theme(width, height, skin.theme.build_wm_theme());

        // Wallpaper texture (composites the skin's image for
        // `style = "image"`), then layout textures and image decals.
        let wp_data = skin_assets::wallpaper_pixels(width, height, &active_theme, &skin);
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
        let (skin_layout_textures, image_layers) =
            skin_assets::create_skin_assets(&mut sdi, &mut backend, &skin, &active_theme);

        // Mouse cursor texture.
        let mut mouse_cursor = CursorState::new(width, height);
        mouse_cursor.scale = active_theme.cursor_scale;
        let cursor_texture;
        {
            let (cursor_pixels, cw, ch) = cursor::generate_cursor_pixels_themed(
                active_theme.cursor_scale,
                active_theme.cursor_fill,
                active_theme.cursor_outline,
            );
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

        // Terminal line editor; restores history persisted in the VFS.
        let shell = ShellSession::new();
        shell.load_history(&cmd_reg, &vfs);

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
            desktops: oasis_wm::DesktopManager::new(1),
            start_menu,
            mouse_cursor,
            cursor_texture,
            wm,
            apps,
            open_runners: Vec::new(),
            closed_app_requests: Vec::new(),
            skin_layout_textures,
            image_layers,
            anim_secs: 0.0,
            browser: None,
            iframe,
            toasts: ToastManager::new(),
            osk: None,
            active_transition,
            mode: Mode::Dashboard,
            cwd: "/".to_string(),
            shell,
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
            radio_consecutive_errors: 0,
            media: oasis_audio::AudioManager::new(),
            media_track: None,
            video_player,
            pending_tv_catalog: None,
            pending_tv_tune: None,
            shader_bridge: shader_bridge::WasmShaderBridge::new(width, height),
            fullscreen_app: None,
            key_filter: oasis_core::input::KeyTwinFilter::default(),
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
    pub fn tick(&mut self, delta_seconds: f32) {
        self.frame_counter += 1;
        self.anim_secs += delta_seconds.clamp(0.0, 1.0);

        // Update system info on the first frame (so the clock never shows
        // a placeholder) and then every ~60 frames (~1s at 60fps).
        if self.frame_counter == 1 || self.frame_counter.is_multiple_of(60) {
            use oasis_core::platform::TimeService;
            let time = self.platform.now().ok();
            self.status_bar.update_info(time.as_ref(), None);
            self.bottom_bar.update_info(time.as_ref());
        }

        // Process queued input events.
        let events = self.input.poll_events();
        for event in &events {
            self.mouse_cursor.handle_input(event);
            self.handle_event(event);
        }

        // Run the next queued background terminal job (`cmd &`), if any.
        self.poll_terminal_jobs();

        // Carry out every request the apps queued (radio tune, settings,
        // ...), then advance the open apps.
        self.tick_app_requests(delta_seconds);

        // Internet radio, the Music Player, then the TV Guide.
        self.tick_radio();
        self.tick_music();
        self.tick_tv_catalog();
        self.start_pending_tv_tune();
        self.tick_video();

        // Update SDI scene graph for the active mode.
        self.update_sdi();

        // Drive browser image streaming.
        if let Some(ref mut bw) = self.browser {
            bw.tick(&self.vfs);
        }

        self.render_frame();
    }

    // -----------------------------------------------------------------------
    // Video Embed + TV Guide auto-advance
    // -----------------------------------------------------------------------

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
            self.youtube_active_url = Some(oasis_apps::video_embed::embed_url(id));
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
        let pending = oasis_apps::video_embed::SearchResults {
            query: query.to_string(),
            status: oasis_apps::video_embed::SearchStatus::Loading,
            error: None,
            results: Vec::new(),
        };
        if let Ok(json) = serde_json::to_string(&pending) {
            let _ = self.vfs.write(
                oasis_apps::video_embed::VIDEO_EMBED_RESULTS_PATH,
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
                    out.push(oasis_apps::video_embed::SearchResult {
                        id: hit.video_id,
                        title: hit.title,
                        author: hit.author,
                        duration: hit.duration,
                        thumb_tex: tex.0,
                    });
                }
                oasis_apps::video_embed::SearchResults {
                    query,
                    status: oasis_apps::video_embed::SearchStatus::Ready,
                    error: None,
                    results: out,
                }
            },
            Err(e) => oasis_apps::video_embed::SearchResults {
                query,
                status: oasis_apps::video_embed::SearchStatus::Error,
                error: Some(e),
                results: Vec::new(),
            },
        };

        if let Ok(json) = serde_json::to_string(&blob) {
            let _ = self.vfs.write(
                oasis_apps::video_embed::VIDEO_EMBED_RESULTS_PATH,
                json.as_bytes(),
            );
        }
    }

    /// When the current episode's `<video>` element fires `ended`, stop the
    /// player and tune to whatever the deterministic schedule says should be
    /// playing now (which will be the next episode).
    fn auto_advance_episode(&mut self) {
        // Gather schedule info from the guide state, then release the borrow.
        let tune_data = {
            let runner = vfs_content::find_tv_guide_runner_wasm(&mut self.open_runners);
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
                let Some(slot) = oasis_apps::tv_guide::schedule_at(catalog, now) else {
                    return;
                };
                if slot.remaining_secs < 5 {
                    now + slot.remaining_secs + 1
                } else {
                    now
                }
            };
            let Some(slot) = oasis_apps::tv_guide::schedule_at(catalog, query_time) else {
                return;
            };

            let url = oasis_apps::tv_guide::catalog::ChannelCatalog::download_url(&slot.episode);
            let seek_secs = slot.elapsed_secs;
            console_log!(
                "TV auto-advance -> {} (seek={}s, remaining={}s)",
                slot.episode.title,
                seek_secs,
                slot.remaining_secs,
            );
            oasis_core::ipc::TvTune { url, seek_secs }
        };

        // Stop the finished player (needs &mut self.video_player + backend).
        self.video_player.stop(&mut self.backend);

        // Queue the tune (same path as a manual tune from the guide).
        self.pending_tv_tune = Some(tune_data);
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
        self.tick_image_layers();

        match self.mode {
            Mode::Dashboard => {
                terminal_sdi::set_terminal_visible(&mut self.sdi, false);
                self.taskbar.hide_sdi(&mut self.sdi);

                if self.bottom_bar.active_tab == MediaTab::None {
                    self.dashboard.update_sdi(&mut self.sdi, &self.active_theme);
                    media_page::hide_media_page(&mut self.sdi);
                } else {
                    self.dashboard.hide_sdi(&mut self.sdi);
                    media_page::update_media_page(
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
                StatusBar::hide_sdi(&mut self.sdi);
                BottomBar::hide_sdi(&mut self.sdi);
                self.taskbar.hide_sdi(&mut self.sdi);
                self.start_menu.close();
                self.start_menu.hide_sdi(&mut self.sdi);
                media_page::hide_media_page(&mut self.sdi);
                let cursor_visible = self.active_theme.terminal_cursor_blink_rate == 0
                    || (self.frame_counter / self.active_theme.terminal_cursor_blink_rate as u64)
                        .is_multiple_of(2);
                let (input_text, cursor_col) = self.shell.display();
                terminal_sdi::setup_terminal_objects_with_cursor(
                    &mut self.sdi,
                    &self.output_lines,
                    &self.cwd,
                    &input_text,
                    cursor_col,
                    self.terminal_scroll_offset,
                    &self.active_theme,
                    cursor_visible,
                );
            },
            Mode::Desktop => {
                terminal_sdi::set_terminal_visible(&mut self.sdi, false);
                media_page::hide_media_page(&mut self.sdi);

                // Sync terminal output to the windowed terminal runner
                // (incremental: only new scrollback lines are cloned).
                let focused = self.wm.active_window() == Some("terminal");
                if let Some((_, runner)) = self
                    .open_runners
                    .iter_mut()
                    .find(|(id, _)| id == "terminal")
                {
                    let (input_text, cursor_col) = self.shell.display();
                    let prompt = format!("> {input_text}");
                    runner.sync_terminal_lines(
                        &self.output_lines,
                        &prompt,
                        self.terminal_scroll_offset,
                    );
                    runner.set_terminal_cursor(focused.then_some(2 + cursor_col));
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
                    // The bar lays out first and leaves the taskbar its
                    // free span, as on the desktop shell.
                    self.bottom_bar.reserve_taskbar = !self.wm.windows().is_empty();
                    self.bottom_bar.update_sdi(
                        &mut self.sdi,
                        &self.active_theme,
                        &self.skin.features,
                    );
                    self.taskbar.update_sdi_in(
                        &mut self.sdi,
                        &self.active_theme,
                        self.wm.windows(),
                        self.wm.active_window(),
                        self.bottom_bar.taskbar_span(),
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
            Mode::Dashboard | Mode::Desktop => {
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

    /// Raise window `id` (restoring it if minimized) or create it at the
    /// default size. Returns whether it already existed.
    fn open_or_raise_window(&mut self, id: &str, title: &str) -> bool {
        if let Some(win) = self.wm.get_window(id) {
            if win.state == oasis_core::wm::window::WindowState::Minimized {
                let _ = self.wm.restore_window(id, &mut self.sdi);
            }
            let _ = self.wm.focus_window(id, &mut self.sdi);
            return true;
        }
        let (width, height) = default_window_size(self.width, self.height);
        let wc = WindowConfig {
            id: id.to_string(),
            title: title.to_string(),
            x: None,
            y: None,
            width,
            height,
            window_type: WindowType::AppWindow,
            always_on_top: false,
            modal: false,
        };
        let _ = self.wm.create_window(&wc, &mut self.sdi);
        false
    }

    fn launch_app_window(&mut self, app: &AppEntry) {
        // Terminal: fullscreen mode for non-WM skins; windowed for WM skins.
        if app.title == oasis_apps::titles::TERMINAL && !self.skin.features.window_manager {
            self.mode = Mode::Terminal;
            self.active_transition = Some(self.make_transition());
            return;
        }

        if app.title == oasis_apps::titles::BROWSER {
            if !self.open_or_raise_window("browser", oasis_apps::titles::BROWSER) {
                let (win_w, win_h) = default_window_size(self.width, self.height);
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

        let win_id = window_id(&app.title);
        if !self.open_or_raise_window(&win_id, &app.title) {
            let runner = AppRunner::launch(&self.apps, app, &self.vfs);
            self.open_runners.push((win_id, runner));
        }
        self.mode = Mode::Desktop;
        self.active_transition = Some(self.make_transition());
    }

    /// Launch entry for `title`: the registered app's own (title and path),
    /// or a bare tile-less entry for the Browser and unknown titles.
    fn entry_for_title(&self, title: &str) -> AppEntry {
        self.apps
            .get(title)
            .map_or_else(|| AppEntry::for_title(title), |reg| reg.to_app_entry())
    }

    /// Launch an app window pre-loaded with a file (File Manager's
    /// Confirm-on-typed-file hand-off to Photo Viewer / Music Player /
    /// Text Editor). An already-open window of that app is reused: its
    /// runner is replaced, and the old runner's backend resources
    /// (textures) are released before it is dropped.
    fn launch_app_window_for_file(&mut self, app_title: &str, file_path: &str) {
        let win_id = window_id(app_title);
        let entry = self.entry_for_title(app_title);
        let runner = AppRunner::launch_with_file(&self.apps, &entry, file_path, &self.vfs);
        self.open_or_raise_window(&win_id, app_title);
        match self.open_runners.iter_mut().find(|(id, _)| *id == win_id) {
            Some((_, slot)) => {
                let mut old = std::mem::replace(slot, runner);
                // The replaced app's last requests still get carried out.
                old.drain_requests(&mut self.closed_app_requests);
                old.release_resources(&mut self.backend);
            },
            None => self.open_runners.push((win_id, runner)),
        }
        self.mode = Mode::Desktop;
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
            StartMenuAction::LaunchApp(title) => self.launch_app(title),
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
        if let Err(e) = self.shell.save_history(&self.cmd_reg, &mut self.vfs) {
            log::warn!("terminal history not saved: {e}");
        }
    }

    /// Run the next queued background terminal job (`cmd &`), if any.
    fn poll_terminal_jobs(&mut self) {
        if self.cmd_reg.pending_jobs() == 0 {
            return;
        }
        let output = {
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
            let output = self.cmd_reg.poll_jobs(&mut env);
            self.cwd = env.cwd;
            output
        };
        if let Some(output) = output {
            if let Some(name) = self.process_command_output(Ok(output)) {
                self.apply_skin_swap(&name);
            }
            vfs_content::trim_output(&mut self.output_lines);
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
                    CommandSignal::SdiInspect { name } => {
                        let text = terminal_sdi::inspect_sdi(&self.sdi, name.as_deref());
                        self.output_lines.extend(text.lines().map(str::to_string));
                    },
                    CommandSignal::ListenToggle { .. }
                    | CommandSignal::RemoteConnect { .. }
                    | CommandSignal::FtpToggle { .. }
                    | CommandSignal::McpToggle { .. } => {
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
            // Unwrapped by the executor; handled for completeness.
            Ok(CommandOutput::Status { output, .. }) => {
                return self.process_command_output(Ok(*output));
            },
            Err(e) => {
                self.output_lines.push(format!("error: {e}"));
            },
        }
        None
    }

    /// Apply a skin swap.
    ///
    /// `name` may also be a variant request (`"@variant:dark"`) that derives
    /// a Dark / Light / High-contrast variant of the currently active skin.
    fn apply_skin_swap(&mut self, name: &str) {
        match oasis_skin::resolve_skin_request(name, &self.skin) {
            Ok(new_skin) => {
                // Hide the outgoing skin's dashboard icons, taskbar and
                // start button: the new skin re-shows only what it uses
                // (a skin with fewer icons or no start menu otherwise
                // keeps stale labels / a START pill from the old one).
                self.dashboard.hide_sdi(&mut self.sdi);
                self.taskbar.hide_sdi(&mut self.sdi);
                self.start_menu.hide_sdi(&mut self.sdi);
                let swapped =
                    Skin::swap_scaled(&self.skin, new_skin, &mut self.sdi, self.width, self.height);
                self.active_theme = ActiveTheme::from_skin(&swapped.theme)
                    .with_screen_size(self.width, self.height)
                    .with_features(&swapped.features);
                self.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                self.wm.set_theme(swapped.theme.build_wm_theme());
                let dash_config =
                    DashboardConfig::from_features(&swapped.features, &self.active_theme);
                self.dashboard =
                    DashboardState::new(dash_config, dashboard_apps(&self.vfs, &self.apps));
                self.bottom_bar.total_pages = self.dashboard.page_count();
                self.bottom_bar.current_page = 0;
                self.start_menu = StartMenuState::new_with_theme(
                    StartMenuState::default_items(&self.active_theme),
                    &self.active_theme,
                );
                self.output_lines
                    .push(format!("Switched to skin: {}", swapped.manifest.name));
                self.skin = swapped;
                // New wallpaper (image or procedural), layout textures and
                // image decals; the old skin's are freed.
                self.refresh_skin_assets();
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

    /// Launch an app by title (e.g. "TV Guide", "Browser", "File Manager";
    /// ASCII case-insensitive): any registered app, with or without a
    /// dashboard tile, or the Browser.
    pub fn launch_app(&mut self, title: &str) {
        let entry = if title.eq_ignore_ascii_case(oasis_apps::titles::BROWSER) {
            AppEntry::for_title(oasis_apps::titles::BROWSER)
        } else if let Some(reg) = self.apps.find(title) {
            reg.to_app_entry()
        } else if let Some(tile) = self
            .dashboard
            .apps
            .iter()
            .find(|a| a.title.eq_ignore_ascii_case(title))
        {
            // A VFS-discovered app the registry doesn't know (placeholder).
            tile.clone()
        } else {
            log::warn!("App not found: {title}");
            return;
        };
        self.launch_app_window(&entry);
    }
}
