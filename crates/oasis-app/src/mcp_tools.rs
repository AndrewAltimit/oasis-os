//! MCP tool implementations that drive the OASIS UI.
//!
//! [`AppDispatcher`] holds field-level borrows of [`AppState`](crate::app_state::AppState)
//! (assembled in [`crate::commands::poll_mcp_server`]) and implements
//! [`oasis_mcp::ToolDispatcher`]. Each `tool_*` method reuses the existing
//! control-surface functions — the same ones the local input pipeline and the
//! remote terminal call — so agent-driven actions behave identically to a user
//! driving the UI.

use std::time::{Duration, Instant};

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::{AppRunner, registered_app_titles};
use oasis_core::backend::{Color, SdiCore};
use oasis_core::browser::{BrowserConfig, BrowserWidget};
use oasis_core::dashboard::AppEntry;
use oasis_core::net::RustlsTlsProvider;
use oasis_core::platform::DesktopPlatform;
use oasis_core::plugin::PluginManager;
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::terminal::{CommandRegistry, Environment};
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wm::manager::WindowManager;
use oasis_mcp::{ToolDispatcher, ToolResult, ToolSpec, base64_encode};
use serde_json::{Value, json};

use crate::app_state::Mode;
use crate::launch;

/// Rolling record of agent activity, surfaced by the in-UI assistant overlay.
#[derive(Debug, Default)]
pub struct AgentActivity {
    /// Total tool calls handled since boot.
    pub call_count: u64,
    /// Name of the most recently invoked tool.
    pub last_tool: Option<String>,
    /// When the last tool call was handled.
    pub last_at: Option<Instant>,
}

impl AgentActivity {
    fn record(&mut self, tool: &str) {
        self.call_count += 1;
        self.last_tool = Some(tool.to_string());
        self.last_at = Some(Instant::now());
    }

    /// Whether a tool ran within the last `within` (drives the overlay pill).
    pub fn is_active(&self, within: Duration) -> bool {
        self.last_at.is_some_and(|t| t.elapsed() < within)
    }
}

/// Draw the assistant-activity overlay pill (bottom-right) when an agent has
/// acted recently. Called once per frame from the draw pass.
pub fn draw_agent_overlay(
    backend: &mut dyn SdiCore,
    activity: &AgentActivity,
    at: &ActiveTheme,
) -> oasis_core::error::Result<()> {
    // Fade the pill out a few seconds after the last tool call.
    if !activity.is_active(Duration::from_secs(6)) {
        return Ok(());
    }
    let label = match &activity.last_tool {
        Some(tool) => format!("\u{25CF} agent: {tool}"),
        None => "\u{25CF} agent".to_string(),
    };
    let font: u16 = 8;
    let text_w = backend.measure_text(&label, font);
    let pill_w = text_w + 12;
    let pill_h: u32 = 13;
    let x = (at.screen_w as i32 - pill_w as i32 - 6).max(0);
    let y = (at.screen_h as i32 - pill_h as i32 - 6).max(0);
    backend.fill_rect(x, y, pill_w, pill_h, Color::rgba(24, 26, 34, 230))?;
    backend.draw_text(&label, x + 6, y + 3, font, Color::rgb(120, 220, 150))?;
    Ok(())
}

/// Borrows of application state needed to service tool calls.
///
/// Constructed per-frame from an [`AppState`](crate::app_state::AppState)
/// borrow split; lives only for the duration of one
/// [`McpServer::poll`](oasis_mcp::McpServer::poll).
pub struct AppDispatcher<'a> {
    pub wm: &'a mut WindowManager,
    pub sdi: &'a mut SdiRegistry,
    pub vfs: &'a mut MemoryVfs,
    pub browser: &'a mut Option<BrowserWidget>,
    pub open_runners: &'a mut Vec<(String, AppRunner)>,
    pub cmd_reg: &'a mut CommandRegistry,
    pub cwd: &'a mut String,
    pub skin: &'a mut Skin,
    pub active_theme: &'a mut ActiveTheme,
    pub browser_config: &'a mut BrowserConfig,
    pub platform: &'a DesktopPlatform,
    pub tls_provider: &'a RustlsTlsProvider,
    pub plugin_manager: &'a PluginManager,
    pub mode: &'a mut Mode,
    /// Rendering backend, for `screenshot` (`read_pixels`).
    pub screen: &'a mut dyn SdiCore,
    pub screen_w: u32,
    pub screen_h: u32,
    pub activity: &'a mut AgentActivity,
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

fn wm_result(res: oasis_core::error::Result<()>, ok_msg: String) -> ToolResult {
    match res {
        Ok(()) => ToolResult::text(ok_msg),
        Err(e) => ToolResult::error(format!("{e}")),
    }
}

impl AppDispatcher<'_> {
    fn tool_list_apps(&self) -> ToolResult {
        let titles = registered_app_titles();
        ToolResult::text(titles.join("\n"))
    }

    fn tool_open_app(&mut self, args: &Value) -> ToolResult {
        let Some(title) = arg_str(args, "title") else {
            return ToolResult::error("missing required argument: title");
        };
        let known = registered_app_titles();
        if !known.iter().any(|t| t.eq_ignore_ascii_case(&title)) {
            return ToolResult::error(format!(
                "unknown app '{title}'. Available: {}",
                known.join(", ")
            ));
        }
        // Normalize to the registry's exact casing.
        let title = known
            .iter()
            .find(|t| t.eq_ignore_ascii_case(&title))
            .map(|t| (*t).to_string())
            .unwrap_or(title);

        if let Some(file) = arg_str(args, "file") {
            launch::launch_app_window_for_file(
                &title,
                &file,
                self.wm,
                self.sdi,
                self.open_runners,
                self.vfs,
            );
            *self.mode = Mode::Desktop;
            return ToolResult::text(format!("Opened {title} with {file}"));
        }

        let entry = AppEntry {
            title: title.clone(),
            path: format!("/apps/{title}"),
            icon_png: Vec::new(),
            color: Color::rgb(100, 100, 100),
        };
        let result = launch::launch_app_window(
            &entry,
            self.wm,
            self.sdi,
            self.open_runners,
            self.browser,
            self.browser_config,
            self.vfs,
            self.tls_provider,
            self.skin.features.window_manager,
            self.plugin_manager,
        );
        launch::apply_launch(result, self.mode);
        ToolResult::text(format!("Opened {title}"))
    }

    fn tool_list_windows(&self) -> ToolResult {
        let active = self.wm.active_window().map(str::to_string);
        let windows: Vec<Value> = self
            .wm
            .windows()
            .iter()
            .map(|w| {
                json!({
                    "id": w.id.as_str(),
                    "title": w.title,
                    "x": w.x,
                    "y": w.y,
                    "width": w.outer_w,
                    "height": w.outer_h,
                    "state": format!("{:?}", w.state),
                    "focused": active.as_deref() == Some(w.id.as_str()),
                })
            })
            .collect();
        ToolResult::text(
            serde_json::to_string_pretty(&json!({ "windows": windows }))
                .unwrap_or_else(|_| "{}".to_string()),
        )
    }

    fn tool_window_op(&mut self, op: &str, args: &Value) -> ToolResult {
        let Some(id) = arg_str(args, "id") else {
            return ToolResult::error("missing required argument: id");
        };
        if self.wm.get_window(&id).is_none() {
            return ToolResult::error(format!("no window with id '{id}'"));
        }
        match op {
            "focus" => wm_result(self.wm.focus_window(&id, self.sdi), format!("focused {id}")),
            "close" => wm_result(self.wm.close_window(&id, self.sdi), format!("closed {id}")),
            "minimize" => wm_result(
                self.wm.minimize_window(&id, self.sdi),
                format!("minimized {id}"),
            ),
            "maximize" => wm_result(
                self.wm.maximize_window(&id, self.sdi),
                format!("maximized {id}"),
            ),
            "restore" => wm_result(
                self.wm.restore_window(&id, self.sdi),
                format!("restored {id}"),
            ),
            _ => ToolResult::error(format!("unknown window op: {op}")),
        }
    }

    fn tool_move_window(&mut self, args: &Value) -> ToolResult {
        let (Some(id), Some(x), Some(y)) =
            (arg_str(args, "id"), arg_i64(args, "x"), arg_i64(args, "y"))
        else {
            return ToolResult::error("requires: id (string), x (int), y (int)");
        };
        let Some(win) = self.wm.get_window(&id) else {
            return ToolResult::error(format!("no window with id '{id}'"));
        };
        // move_window takes a delta; convert absolute target to a delta.
        let dx = (x - win.x as i64) as i32;
        let dy = (y - win.y as i64) as i32;
        wm_result(
            self.wm.move_window(&id, dx, dy, self.sdi),
            format!("moved {id} toward ({x}, {y})"),
        )
    }

    fn tool_resize_window(&mut self, args: &Value) -> ToolResult {
        let (Some(id), Some(w), Some(h)) = (
            arg_str(args, "id"),
            arg_i64(args, "width"),
            arg_i64(args, "height"),
        ) else {
            return ToolResult::error("requires: id (string), width (int), height (int)");
        };
        if self.wm.get_window(&id).is_none() {
            return ToolResult::error(format!("no window with id '{id}'"));
        }
        let w = w.clamp(80, 4096) as u32;
        let h = h.clamp(60, 4096) as u32;
        wm_result(
            self.wm.resize_window(&id, w, h, self.sdi),
            format!("resized {id} to {w}x{h}"),
        )
    }

    fn tool_run_command(&mut self, args: &Value) -> ToolResult {
        let Some(command) = arg_str(args, "command") else {
            return ToolResult::error("missing required argument: command");
        };
        let mut env = Environment {
            cwd: self.cwd.clone(),
            vfs: self.vfs,
            power: Some(self.platform),
            time: Some(self.platform),
            usb: Some(self.platform),
            network: None,
            tls: Some(self.tls_provider),
            stdin: None,
            stderr: String::new(),
        };
        let result = self.cmd_reg.execute(&command, &mut env);
        *self.cwd = env.cwd;
        let text = crate::commands::format_remote_response(
            result,
            self.browser,
            self.skin,
            self.active_theme,
            self.browser_config,
            self.wm,
            self.sdi,
        );
        ToolResult::text(text)
    }

    fn tool_browser_navigate(&mut self, args: &Value) -> ToolResult {
        let Some(url) = arg_str(args, "url") else {
            return ToolResult::error("missing required argument: url");
        };
        // Ensure a Browser widget exists (open it if needed).
        if self.browser.is_none() {
            let entry = AppEntry {
                title: "Browser".to_string(),
                path: "/apps/Browser".to_string(),
                icon_png: Vec::new(),
                color: Color::rgb(100, 100, 100),
            };
            let result = launch::launch_app_window(
                &entry,
                self.wm,
                self.sdi,
                self.open_runners,
                self.browser,
                self.browser_config,
                self.vfs,
                self.tls_provider,
                self.skin.features.window_manager,
                self.plugin_manager,
            );
            launch::apply_launch(result, self.mode);
        }
        match self.browser {
            Some(bw) => {
                bw.navigate_vfs(&url, self.vfs);
                ToolResult::text(format!("Navigated to {url}"))
            },
            None => ToolResult::error("browser unavailable"),
        }
    }

    fn tool_play_media(&mut self, args: &Value) -> ToolResult {
        let Some(path) = arg_str(args, "path") else {
            return ToolResult::error("missing required argument: path");
        };
        let request = format!("play_file {path}");
        match self
            .vfs
            .write(oasis_app_media::MEDIA_REQUEST_PATH, request.as_bytes())
        {
            Ok(()) => ToolResult::text(format!("Queued playback: {path}")),
            Err(e) => ToolResult::error(format!("{e}")),
        }
    }

    fn tool_tune(&mut self, args: &Value) -> ToolResult {
        let source = arg_str(args, "source").unwrap_or_default();
        let Some(channel) = arg_str(args, "channel") else {
            return ToolResult::error("missing required argument: channel");
        };
        let (path, request) = match source.as_str() {
            "radio" => (oasis_audio::RADIO_REQUEST_PATH, format!("tune {channel}")),
            "tv" => {
                let Ok(n) = channel.parse::<u32>() else {
                    return ToolResult::error("tv channel must be a number");
                };
                (
                    oasis_core::apps::tv_guide::TV_REQUEST_PATH,
                    format!("tune_ch:{n}"),
                )
            },
            other => {
                return ToolResult::error(format!("source must be 'radio' or 'tv', got '{other}'"));
            },
        };
        match self.vfs.write(path, request.as_bytes()) {
            Ok(()) => ToolResult::text(format!("Tuning {source} -> {channel}")),
            Err(e) => ToolResult::error(format!("{e}")),
        }
    }

    fn tool_get_state(&self) -> ToolResult {
        let windows: Vec<Value> = self
            .wm
            .windows()
            .iter()
            .map(|w| json!({ "id": w.id.as_str(), "title": w.title }))
            .collect();
        let state = json!({
            "mode": format!("{:?}", self.mode),
            "skin": self.skin.manifest.name,
            "focused_window": self.wm.active_window(),
            "open_windows": windows,
            "browser_url": self.browser.as_ref().and_then(|b| b.current_url().map(str::to_string)),
            "agent_calls": self.activity.call_count,
        });
        ToolResult::text(serde_json::to_string_pretty(&state).unwrap_or_else(|_| "{}".to_string()))
    }

    fn tool_screenshot(&mut self) -> ToolResult {
        let (w, h) = (self.screen_w, self.screen_h);
        let rgba = match self.screen.read_pixels(0, 0, w, h) {
            Ok(px) => px,
            Err(e) => return ToolResult::error(format!("read_pixels: {e}")),
        };
        // Force alpha opaque (the framebuffer has no real transparency).
        let mut opaque = rgba;
        for px in opaque.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let mut png_bytes: Vec<u8> = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = match encoder.write_header() {
                Ok(wr) => wr,
                Err(e) => return ToolResult::error(format!("png header: {e}")),
            };
            if let Err(e) = writer.write_image_data(&opaque) {
                return ToolResult::error(format!("png encode: {e}"));
            }
        }
        ToolResult::image(base64_encode(&png_bytes), "image/png")
    }
}

impl ToolDispatcher for AppDispatcher<'_> {
    fn list_tools(&self) -> Vec<ToolSpec> {
        let no_args = || json!({ "type": "object", "properties": {} });
        let id_only = || {
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Window id" } },
                "required": ["id"]
            })
        };
        vec![
            ToolSpec::new(
                "list_apps",
                "List the titles of all launchable apps.",
                no_args(),
            ),
            ToolSpec::new(
                "open_app",
                "Open an app by title, optionally pre-loading a VFS file path.",
                json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "App title, e.g. 'Terminal', 'Browser'" },
                        "file": { "type": "string", "description": "Optional VFS file path to open in the app" }
                    },
                    "required": ["title"]
                }),
            ),
            ToolSpec::new(
                "list_windows",
                "List all open windows with id, title, position, size, and focus.",
                no_args(),
            ),
            ToolSpec::new("focus_window", "Bring a window to the front.", id_only()),
            ToolSpec::new("close_window", "Close a window.", id_only()),
            ToolSpec::new("minimize_window", "Minimize a window.", id_only()),
            ToolSpec::new("maximize_window", "Maximize a window.", id_only()),
            ToolSpec::new(
                "restore_window",
                "Restore a minimized/maximized window.",
                id_only(),
            ),
            ToolSpec::new(
                "move_window",
                "Move a window to an absolute (x, y) position.",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "x": { "type": "integer" },
                        "y": { "type": "integer" }
                    },
                    "required": ["id", "x", "y"]
                }),
            ),
            ToolSpec::new(
                "resize_window",
                "Resize a window to width x height (outer pixels).",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "width": { "type": "integer" },
                        "height": { "type": "integer" }
                    },
                    "required": ["id", "width", "height"]
                }),
            ),
            ToolSpec::new(
                "run_command",
                "Run a terminal command and return its output. Grants shell-level authority.",
                json!({
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"]
                }),
            ),
            ToolSpec::new(
                "browser_navigate",
                "Open the Browser (if needed) and navigate to a URL or vfs:// path.",
                json!({
                    "type": "object",
                    "properties": { "url": { "type": "string" } },
                    "required": ["url"]
                }),
            ),
            ToolSpec::new(
                "play_media",
                "Play an audio/media file by VFS path in the Music Player.",
                json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
            ),
            ToolSpec::new(
                "tune",
                "Tune the radio or TV to a channel/station.",
                json!({
                    "type": "object",
                    "properties": {
                        "source": { "type": "string", "enum": ["radio", "tv"] },
                        "channel": { "type": "string", "description": "Station index/name (radio) or channel number (tv)" }
                    },
                    "required": ["source", "channel"]
                }),
            ),
            ToolSpec::new(
                "get_state",
                "Return the current shell state: mode, skin, focused window, open windows, browser URL.",
                no_args(),
            ),
            ToolSpec::new(
                "screenshot",
                "Capture the current screen as a PNG image so you can see the UI.",
                no_args(),
            ),
        ]
    }

    fn call_tool(&mut self, name: &str, args: Value) -> ToolResult {
        self.activity.record(name);
        log::info!("MCP tool call: {name}");
        match name {
            "list_apps" => self.tool_list_apps(),
            "open_app" => self.tool_open_app(&args),
            "list_windows" => self.tool_list_windows(),
            "focus_window" => self.tool_window_op("focus", &args),
            "close_window" => self.tool_window_op("close", &args),
            "minimize_window" => self.tool_window_op("minimize", &args),
            "maximize_window" => self.tool_window_op("maximize", &args),
            "restore_window" => self.tool_window_op("restore", &args),
            "move_window" => self.tool_move_window(&args),
            "resize_window" => self.tool_resize_window(&args),
            "run_command" => self.tool_run_command(&args),
            "browser_navigate" => self.tool_browser_navigate(&args),
            "play_media" => self.tool_play_media(&args),
            "tune" => self.tool_tune(&args),
            "get_state" => self.tool_get_state(),
            "screenshot" => self.tool_screenshot(),
            other => ToolResult::error(format!("unknown tool: {other}")),
        }
    }
}
