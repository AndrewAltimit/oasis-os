//! WebAssembly backend for OASIS_OS.
//!
//! Renders to an HTML `<canvas>` element using the Canvas 2D API,
//! maps DOM events to `InputEvent`, and provides Web Audio playback.

pub mod archive;
pub mod audio;
pub mod font;
pub mod iframe;
pub mod input;
pub mod network;
pub mod platform;
pub mod renderer;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::{AppAction, AppRunner};
use oasis_core::backend::{AudioBackend, Color, InputBackend, SdiBackend, TextureId};
use oasis_core::bottombar::{BottomBar, MediaTab};
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::cursor::{self, CursorState};
use oasis_core::dashboard::{AppEntry, DashboardConfig, DashboardState, discover_apps};
use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::osk::{OskConfig, OskState};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::startmenu::{StartMenuAction, StartMenuState};
use oasis_core::statusbar::StatusBar;
use oasis_core::terminal::{
    CommandOutput, CommandRegistry, Environment, populate_man_pages, populate_motd,
    populate_profile, register_builtins,
};
use oasis_core::terminal_sdi;
use oasis_core::toast::ToastManager;
use oasis_core::transition::{self, TransitionState};
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wallpaper;
use oasis_core::wm::manager::{WindowManager, WmEvent};
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
}

#[wasm_bindgen]
impl OasisWasm {
    /// Create a new OASIS_OS instance attached to a canvas element.
    ///
    /// `canvas_id` is the DOM `id` of the target `<canvas>`.
    /// `skin_name` is an optional built-in skin name (e.g. "classic", "modern").
    #[wasm_bindgen(constructor)]
    pub fn new(canvas_id: &str, skin_name: Option<String>) -> Result<OasisWasm, JsValue> {
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

        // Scene graph and commands.
        let mut sdi = SdiRegistry::new();
        let mut cmd_reg = CommandRegistry::new();
        register_builtins(&mut cmd_reg);

        // VFS with demo content.
        let mut vfs = MemoryVfs::new();
        populate_wasm_vfs(&mut vfs);

        let active_theme = ActiveTheme::from_skin(&skin.theme).with_screen_size(width, height);
        let mut browser_config = BrowserConfig::from_skin_theme(&skin.theme);
        // In WASM mode, use Google's iframe-compatible search page as home.
        browser_config.features.home_url = "https://www.google.com/webhp?igu=1".to_string();

        // Apply skin layout and discover apps.
        skin.apply_layout(&mut sdi);
        let apps = discover_apps(&vfs, "/apps", None).unwrap_or_default();
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

        // Process pending VFS requests from app runners.
        {
            let mut pending = None;
            if let Some(ref mut runner) = self.app_runner {
                pending = runner.take_pending_request();
            }
            if pending.is_none() {
                for (_, runner) in &mut self.open_runners {
                    if let Some(req) = runner.take_pending_request() {
                        pending = Some(req);
                        break;
                    }
                }
            }
            if let Some((path, data)) = pending {
                let _ = self.vfs.write(&path, data.as_bytes());
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
            if let Some(ref fetcher) = self.pending_catalog
                && fetcher.is_ready()
            {
                let fetcher = self.pending_catalog.take().unwrap();
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
        }

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
        if self.mode == Mode::Desktop && self.wm.window_count() > 0 {
            let browser = &mut self.browser;
            let iframe_ref = &mut self.iframe;
            let open_runners = &self.open_runners;
            let active_theme = &self.active_theme;
            if let Err(e) = self.wm.draw_with_clips(
                &mut self.sdi,
                &mut self.backend,
                |window_id, cx, cy, cw, ch, be| {
                    let result = if window_id == "browser" {
                        if let Some(ref mut bw) = *browser {
                            bw.set_window(cx, cy, cw, ch);
                            let url = bw.current_url().map(|s| s.to_string());
                            let is_http = url.as_ref().is_some_and(|u| {
                                u.starts_with("http://") || u.starts_with("https://")
                            });
                            if is_http {
                                let url_bar_h = bw.config.url_bar_height;
                                let status_bar_h = bw.config.status_bar_height;
                                let content_y = cy + url_bar_h as i32;
                                let content_h = ch.saturating_sub(url_bar_h + status_bar_h);
                                iframe_ref.show(
                                    url.as_ref().unwrap(),
                                    cx,
                                    content_y,
                                    cw,
                                    content_h,
                                );
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
                        runner.draw_windowed(cx, cy, cw, ch, be, active_theme)
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
            if let Err(e) = self.sdi.draw(&mut self.backend) {
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
        // transition overlay).
        if self.mouse_cursor.visible
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
                self.dashboard.hide_sdi(&mut self.sdi);
                terminal_sdi::hide_media_page(&mut self.sdi);
                self.status_bar
                    .update_sdi(&mut self.sdi, &self.active_theme, &self.skin.features);
                self.bottom_bar
                    .update_sdi(&mut self.sdi, &self.active_theme, &self.skin.features);
                if self.skin.features.start_menu {
                    self.start_menu
                        .update_sdi(&mut self.sdi, &self.active_theme);
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

        // Ensure wallpaper is visible and at lowest z.
        if let Ok(obj) = self.sdi.get_mut("wallpaper") {
            obj.visible = true;
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: default (Dashboard / Terminal)
    // -----------------------------------------------------------------------

    fn handle_default_input(&mut self, event: &InputEvent) {
        match event {
            // Launch app from dashboard.
            InputEvent::ButtonPress(Button::Confirm) if self.mode == Mode::Dashboard => {
                self.dashboard.trigger_press_flash();
                if self.bottom_bar.active_tab == MediaTab::None
                    && let Some(app) = self.dashboard.selected_app()
                {
                    let app = app.clone();
                    self.launch_app_window(&app);
                }
            },

            // Pointer click on dashboard: start menu takes priority.
            InputEvent::PointerClick { x, y } if self.mode == Mode::Dashboard => {
                if self.start_menu.hit_test_button(*x, *y) {
                    self.start_menu.toggle();
                    return;
                }
                if self.start_menu.open {
                    if let Some(action) = self.start_menu.hit_test_item(*x, *y) {
                        self.start_menu.close();
                        self.handle_start_menu_action(&action);
                    } else {
                        self.start_menu.close();
                    }
                    return;
                }
                if self.bottom_bar.active_tab == MediaTab::None {
                    let cfg = &self.dashboard.config;
                    let gx = *x - cfg.grid_x;
                    let gy = *y - cfg.grid_y;
                    if gx >= 0 && gy >= 0 {
                        let col = gx as usize / cfg.cell_w as usize;
                        let row = gy as usize / cfg.cell_h as usize;
                        if col < cfg.grid_cols as usize && row < cfg.grid_rows as usize {
                            let idx = row * cfg.grid_cols as usize + col;
                            let page_apps = self.dashboard.current_page_apps().len();
                            if idx < page_apps {
                                if self.dashboard.selected == idx {
                                    if let Some(app) = self.dashboard.selected_app() {
                                        let app = app.clone();
                                        self.launch_app_window(&app);
                                    }
                                } else {
                                    self.dashboard.selected = idx;
                                }
                            }
                        }
                    }
                }
            },

            InputEvent::ButtonPress(Button::Start) => {
                self.mode = match self.mode {
                    Mode::Dashboard => Mode::Terminal,
                    Mode::Terminal => Mode::Dashboard,
                    other => other,
                };
            },
            InputEvent::ButtonPress(Button::Select) => {
                if self.mode != Mode::Osk {
                    let osk_cfg = OskConfig {
                        title: "On-Screen Keyboard".to_string(),
                        ..OskConfig::for_screen(
                            self.active_theme.screen_w,
                            self.active_theme.screen_h,
                        )
                    };
                    self.osk = Some(OskState::new(osk_cfg, ""));
                    self.mode = Mode::Osk;
                }
            },

            InputEvent::ButtonPress(Button::Cancel) if self.mode == Mode::Dashboard => {},

            // L trigger: cycle top tabs.
            InputEvent::TriggerPress(Trigger::Left) if self.mode == Mode::Dashboard => {
                self.status_bar.next_tab();
                self.bottom_bar.l_pressed = true;
            },
            InputEvent::TriggerRelease(Trigger::Left) => {
                self.bottom_bar.l_pressed = false;
            },

            // R trigger: cycle media category tabs.
            InputEvent::TriggerPress(Trigger::Right) if self.mode == Mode::Dashboard => {
                self.bottom_bar.next_tab();
                self.bottom_bar.r_pressed = true;
                self.active_transition = Some(transition::fade_in_custom(
                    self.width,
                    self.height,
                    self.skin.features.transition_fade_frames.unwrap_or(15),
                ));
            },
            InputEvent::TriggerRelease(Trigger::Right) => {
                self.bottom_bar.r_pressed = false;
            },

            // Start menu intercepts input when open.
            InputEvent::ButtonPress(btn)
                if self.mode == Mode::Dashboard && self.start_menu.open =>
            {
                let action = self.start_menu.handle_input(btn);
                if action != StartMenuAction::None {
                    self.handle_start_menu_action(&action);
                }
            },

            // Dashboard D-pad navigation.
            InputEvent::ButtonPress(btn) if self.mode == Mode::Dashboard => match btn {
                Button::Up | Button::Down | Button::Left | Button::Right => {
                    if self.bottom_bar.active_tab == MediaTab::None {
                        self.dashboard.handle_input(btn);
                    }
                },
                Button::Triangle => {
                    if self.bottom_bar.active_tab == MediaTab::None {
                        self.dashboard.next_page();
                        self.bottom_bar.current_page = self.dashboard.page;
                    }
                },
                Button::Square => {
                    if self.bottom_bar.active_tab == MediaTab::None {
                        self.dashboard.prev_page();
                        self.bottom_bar.current_page = self.dashboard.page;
                    }
                },
                _ => {},
            },

            // Terminal input.
            InputEvent::TextInput(ch) if self.mode == Mode::Terminal => {
                self.input_buf.push(*ch);
            },
            InputEvent::Backspace if self.mode == Mode::Terminal => {
                self.input_buf.pop();
            },
            InputEvent::ButtonPress(Button::Confirm) if self.mode == Mode::Terminal => {
                let line = self.input_buf.clone();
                self.input_buf.clear();
                self.terminal_scroll_offset = 0;
                if !line.is_empty() {
                    self.output_lines.push(format!("> {line}"));
                    self.execute_terminal_command(&line);
                    trim_output(&mut self.output_lines);
                }
            },
            InputEvent::ButtonPress(Button::Square) if self.mode == Mode::Terminal => {
                self.input_buf.pop();
            },
            InputEvent::ButtonPress(Button::Cancel) if self.mode == Mode::Terminal => {
                terminal_sdi::set_terminal_visible(&mut self.sdi, false);
                self.mode = Mode::Dashboard;
            },
            InputEvent::MouseWheel { delta } if self.mode == Mode::Terminal => {
                let len = self.output_lines.len();
                let max_visible = terminal_sdi::visible_output_lines(&self.active_theme);
                if len > max_visible {
                    let max_offset = len - max_visible;
                    if *delta < 0 {
                        self.terminal_scroll_offset =
                            (self.terminal_scroll_offset + (-*delta as usize) * 3).min(max_offset);
                    } else {
                        self.terminal_scroll_offset = self
                            .terminal_scroll_offset
                            .saturating_sub(*delta as usize * 3);
                    }
                }
            },

            _ => {},
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: Desktop (windowed WM) mode
    // -----------------------------------------------------------------------

    fn handle_desktop_input(&mut self, event: &InputEvent) {
        match event {
            InputEvent::PointerClick { x, y } => {
                // Start menu takes priority over window manager.
                if self.start_menu.hit_test_button(*x, *y) {
                    self.start_menu.toggle();
                    return;
                }
                if self.start_menu.open {
                    if let Some(action) = self.start_menu.hit_test_item(*x, *y) {
                        self.start_menu.close();
                        self.handle_start_menu_action(&action);
                    } else {
                        self.start_menu.close();
                    }
                    return;
                }

                let wm_event = self
                    .wm
                    .handle_input(&InputEvent::PointerClick { x: *x, y: *y }, &mut self.sdi);
                match wm_event {
                    WmEvent::WindowClosed(id) => {
                        self.open_runners.retain(|(rid, _)| *rid != id);
                        if id == "browser" {
                            self.browser = None;
                            self.iframe.hide();
                        }
                        if self.wm.window_count() == 0 {
                            self.mode = Mode::Dashboard;
                        }
                    },
                    WmEvent::ContentClick(id, lx, ly) => {
                        if id == "browser"
                            && let Some(ref mut bw) = self.browser
                        {
                            // When the iframe is showing a real web page,
                            // only forward clicks in the URL bar area to
                            // the browser widget — the iframe handles its
                            // own content input natively.
                            let in_url_bar = ly < bw.config.url_bar_height as i32;
                            if in_url_bar || !self.iframe.is_visible() {
                                let abs_x = bw.window_x() + lx;
                                let abs_y = bw.window_y() + ly;
                                bw.handle_input(
                                    &InputEvent::PointerClick { x: abs_x, y: abs_y },
                                    &self.vfs,
                                );
                            }
                        }
                    },
                    WmEvent::DesktopClick(_, _) => {
                        if self.wm.window_count() == 0 {
                            self.mode = Mode::Dashboard;
                        }
                    },
                    _ => {},
                }
            },
            InputEvent::CursorMove { x, y } => {
                self.wm
                    .handle_input(&InputEvent::CursorMove { x: *x, y: *y }, &mut self.sdi);
            },
            InputEvent::PointerRelease { x, y } => {
                self.wm
                    .handle_input(&InputEvent::PointerRelease { x: *x, y: *y }, &mut self.sdi);
            },
            InputEvent::ButtonPress(Button::Cancel) => {
                if let Some(active_id) = self.wm.active_window().map(|s| s.to_string()) {
                    let _ = self.wm.close_window(&active_id, &mut self.sdi);
                    self.open_runners.retain(|(rid, _)| *rid != active_id);
                    if active_id == "browser" {
                        self.browser = None;
                        self.iframe.hide();
                    }
                    if self.wm.window_count() == 0 {
                        self.mode = Mode::Dashboard;
                    }
                } else {
                    self.mode = Mode::Dashboard;
                }
            },
            InputEvent::ButtonPress(Button::Start) => {
                self.mode = Mode::Terminal;
            },
            InputEvent::TextInput(ch) => {
                if self.wm.active_window() == Some("browser")
                    && let Some(ref mut bw) = self.browser
                {
                    bw.handle_input(&InputEvent::TextInput(*ch), &self.vfs);
                }
            },
            InputEvent::Backspace => {
                if self.wm.active_window() == Some("browser")
                    && let Some(ref mut bw) = self.browser
                {
                    bw.handle_input(&InputEvent::Backspace, &self.vfs);
                }
            },
            InputEvent::MouseWheel { delta } => {
                if self.wm.active_window() == Some("browser")
                    && let Some(ref mut bw) = self.browser
                {
                    bw.handle_input(&InputEvent::MouseWheel { delta: *delta }, &self.vfs);
                }
            },
            InputEvent::ButtonPress(btn) => {
                if let Some(active_id) = self.wm.active_window().map(|s| s.to_string()) {
                    if active_id == "browser" {
                        if let Some(ref mut bw) = self.browser {
                            bw.handle_input(&InputEvent::ButtonPress(*btn), &self.vfs);
                        }
                    } else if let Some((_, runner)) = self
                        .open_runners
                        .iter_mut()
                        .find(|(id, _)| *id == active_id)
                    {
                        match runner.handle_input(btn, &self.vfs) {
                            AppAction::Exit => {
                                let _ = self.wm.close_window(&active_id, &mut self.sdi);
                                self.open_runners.retain(|(rid, _)| *rid != active_id);
                                if self.wm.window_count() == 0 {
                                    self.mode = Mode::Dashboard;
                                }
                            },
                            AppAction::SwitchToTerminal => {
                                self.mode = Mode::Terminal;
                            },
                            AppAction::None => {},
                        }
                    }
                }
            },
            _ => {},
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: App (fullscreen) mode
    // -----------------------------------------------------------------------

    fn handle_app_input(&mut self, event: &InputEvent) {
        if let Some(ref mut runner) = self.app_runner
            && let InputEvent::ButtonPress(btn) = event
        {
            match runner.handle_input(btn, &self.vfs) {
                AppAction::Exit => {
                    AppRunner::hide_sdi(&mut self.sdi);
                    self.app_runner = None;
                    self.mode = Mode::Dashboard;
                },
                AppAction::SwitchToTerminal => {
                    AppRunner::hide_sdi(&mut self.sdi);
                    self.app_runner = None;
                    self.mode = Mode::Terminal;
                },
                AppAction::None => {},
            }
        }
    }

    // -----------------------------------------------------------------------
    // Input dispatch: OSK mode
    // -----------------------------------------------------------------------

    fn handle_osk_input(&mut self, event: &InputEvent) {
        if let Some(ref mut osk_state) = self.osk {
            match event {
                InputEvent::Backspace => {
                    osk_state.buffer.pop();
                },
                InputEvent::ButtonPress(btn) => {
                    osk_state.handle_input(btn);
                    if let Some(text) = osk_state.confirmed_text() {
                        self.output_lines.push(format!("[OSK] Input: {text}"));
                        trim_output(&mut self.output_lines);
                        osk_state.hide_sdi(&mut self.sdi);
                        self.osk = None;
                        self.mode = Mode::Dashboard;
                    } else if osk_state.is_cancelled() {
                        self.output_lines.push("[OSK] Cancelled".to_string());
                        trim_output(&mut self.output_lines);
                        osk_state.hide_sdi(&mut self.sdi);
                        self.osk = None;
                        self.mode = Mode::Dashboard;
                    }
                },
                _ => {},
            }
        }
    }

    // -----------------------------------------------------------------------
    // App launching (mirrors oasis-app launch.rs)
    // -----------------------------------------------------------------------

    fn launch_app_window(&mut self, app: &AppEntry) {
        if app.title == "Terminal" {
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
            Ok(CommandOutput::ListenToggle { .. }) => {
                self.output_lines
                    .push("Not available in browser.".to_string());
            },
            Ok(CommandOutput::RemoteConnect { .. }) => {
                self.output_lines
                    .push("Not available in browser.".to_string());
            },
            Ok(CommandOutput::FtpToggle { .. }) => {
                self.output_lines
                    .push("Not available in browser.".to_string());
            },
            Ok(CommandOutput::BrowserSandbox { enable }) => {
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
            Ok(CommandOutput::SkinSwap { name }) => {
                return Some(name);
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
                let swapped = Skin::swap(&self.skin, new_skin, &mut self.sdi);
                self.active_theme = ActiveTheme::from_skin(&swapped.theme)
                    .with_screen_size(self.width, self.height);
                self.browser_config = BrowserConfig::from_skin_theme(&swapped.theme);
                self.wm.set_theme(swapped.theme.build_wm_theme());
                let dash_config =
                    DashboardConfig::from_features(&swapped.features, &self.active_theme);
                let apps = discover_apps(&self.vfs, "/apps", None).unwrap_or_default();
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
        trim_output(&mut self.output_lines);
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate output lines to `MAX_OUTPUT_LINES`.
fn trim_output(output_lines: &mut Vec<String>) {
    while output_lines.len() > terminal_sdi::MAX_OUTPUT_LINES {
        output_lines.remove(0);
    }
}

// ---------------------------------------------------------------------------
// VFS population
// ---------------------------------------------------------------------------

/// Populate the WASM VFS with demo content.
fn populate_wasm_vfs(vfs: &mut MemoryVfs) {
    // Core directory structure.
    let _ = vfs.mkdir("/home");
    let _ = vfs.mkdir("/home/user");
    let _ = vfs.mkdir("/etc");
    let _ = vfs.mkdir("/tmp");
    let _ = vfs.mkdir("/var");
    let _ = vfs.mkdir("/var/oasis");
    let _ = vfs.mkdir("/var/log");

    // Use the terminal's built-in content populators.
    populate_motd(vfs);
    populate_profile(vfs);
    populate_man_pages(vfs);

    // System metadata.
    let _ = vfs.write("/etc/hostname", b"oasis-wasm");
    let _ = vfs.write("/etc/version", b"1.0.0-wasm");

    // Demo user files.
    let _ = vfs.write(
        "/home/user/readme.txt",
        b"OASIS_OS is running in your browser!\n\
          \n\
          This is a retro operating system shell originally built for the PSP.\n\
          It now runs on desktop (SDL2), Unreal Engine 5, and WebAssembly.\n\
          \n\
          Try these commands:\n\
            help        Show available commands\n\
            ls          List files\n\
            cat <file>  Read a file\n\
            skin list   Show available skins\n\
            fortune     Random fortune\n\
            tutorial    Interactive terminal tutorial\n\
            man ls      Manual page for a command\n",
    );

    let _ = vfs.write(
        "/home/user/notes.txt",
        b"Shopping list:\n- Milk\n- Bread\n- Memory Stick PRO Duo\n",
    );

    // Demo app directories (discovered by the dashboard).
    // Names must match the title strings in AppRunner::init_content().
    let _ = vfs.mkdir("/apps");
    let _ = vfs.mkdir("/apps/File Manager");
    let _ = vfs.mkdir("/apps/Settings");
    let _ = vfs.mkdir("/apps/Browser");
    let _ = vfs.mkdir("/apps/Music Player");
    let _ = vfs.mkdir("/apps/Terminal");

    // Browser home page content.
    let _ = vfs.mkdir("/sites");
    let _ = vfs.mkdir("/sites/home");
    let _ = vfs.write(
        "/sites/home/index.html",
        br#"<html><head><title>OASIS Home</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
h2 { color: #80d0a0; }
a { color: #64c8ff; }
code { background-color: rgba(100,200,255,30); }
pre { background-color: rgba(100,200,255,15); border: 1px solid rgba(100,200,255,30); }
blockquote { border-left-color: #64c8ff; color: #a0a0c0; }
table { border-collapse: collapse; }
th { background-color: rgba(100,200,255,20); border: 1px solid rgba(255,255,255,30); }
td { border: 1px solid rgba(255,255,255,20); }
</style>
</head><body>
<h1>Welcome to OASIS Browser</h1>
<p>A lightweight <strong>HTML/CSS</strong> rendering engine for
<em>OASIS_OS</em>. Supports block, inline, flex, and table layout.</p>

<h2>Features</h2>
<ul>
<li>CSS cascade with <code>specificity</code></li>
<li>Block, inline, flex, and table layout</li>
<li>Text wrapping and decoration</li>
<li>Smooth scrolling with mouse wheel</li>
</ul>

<h2>Shortcuts</h2>
<table>
<tr><th>Key</th><th>Action</th></tr>
<tr><td>Tab</td><td>Focus URL bar</td></tr>
<tr><td>Left/Right</td><td>Navigate links</td></tr>
<tr><td>Up/Down</td><td>Scroll page</td></tr>
</table>

<blockquote>Originally ported from a PSP homebrew shell (2006-2008).</blockquote>

<h2>Links</h2>
<ol>
<li><a href="/sites/home/about.html">About OASIS Browser</a></li>
<li><a href="/sites/home/features.html">CSS Feature Test</a></li>
</ol>
</body></html>"#,
    );
    let _ = vfs.write(
        "/sites/home/about.html",
        br#"<html><head><title>About OASIS Browser</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
a { color: #64c8ff; }
</style>
</head><body>
<h1>About OASIS Browser</h1>
<p>A lightweight HTML/CSS engine for embedded systems:</p>
<ul>
<li><strong>HTML</strong> -- WHATWG tokenizer, 70+ tags</li>
<li><strong>CSS</strong> -- cascade, specificity, media queries</li>
<li><strong>Layout</strong> -- block, inline, flex, table, float</li>
<li><strong>Gemini</strong> -- lightweight text protocol</li>
</ul>
<p><a href="/sites/home/index.html">Back to home</a></p>
</body></html>"#,
    );
    let _ = vfs.write(
        "/sites/home/features.html",
        br#"<html><head><title>CSS Features</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
h2 { color: #80d0a0; font-size: 1.2em; }
a { color: #64c8ff; }
</style>
</head><body>
<h1>CSS Feature Test</h1>
<h2>Text Formatting</h2>
<p><strong>Bold</strong>, <em>italic</em>, <u>underline</u>,
<s>strikethrough</s>, <code>inline code</code>,
<mark>highlighted</mark>, <small>small</small>.</p>
<h2>Blockquote</h2>
<blockquote>Blockquote with left border.</blockquote>
<h2>Ordered List</h2>
<ol><li>First</li><li>Second</li><li>Third</li></ol>
<h2>Preformatted</h2>
<pre>fn main() {
    println!("Hello!");
}</pre>
<p><a href="/sites/home/index.html">Back to home</a></p>
</body></html>"#,
    );

    // Demo startup script.
    let _ = vfs.write(
        "/home/user/startup.sh",
        b"# OASIS_OS startup script\necho Welcome back!\nls /apps\n",
    );
}
