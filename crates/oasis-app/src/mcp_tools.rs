//! MCP tool implementations that drive the OASIS UI.
//!
//! [`AppDispatcher`] borrows the whole [`AppState`] (the MCP server itself
//! is taken out of it for the poll, see [`crate::commands::poll_mcp_server`])
//! and implements [`oasis_mcp::ToolDispatcher`]. Each `tool_*` method reuses
//! the existing control-surface functions — the same ones the local input
//! pipeline and the remote terminal call ([`launch::launch`],
//! [`close_app_window`], the skin swap) — so agent-driven actions behave
//! identically to a user driving the UI.

use std::time::{Duration, Instant};

use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{Color, SdiCore};
use oasis_core::sdi::SdiRegistry;
use oasis_core::terminal::Environment;
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_mcp::{ToolDispatcher, ToolResult, ToolSpec, base64_encode};
use serde_json::{Value, json};

use crate::app_state::AppState;
use crate::launch::{self, BROWSER_TITLE, LaunchOpts};
use crate::windows::{CloseHow, CloseOutcome, close_app_window};

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

/// What a tool call can reach: the shell state (minus the MCP server,
/// which is polling), the scene, the VFS and the framebuffer.
///
/// Lives only for the duration of one
/// [`McpServer::poll`](oasis_mcp::McpServer::poll).
pub struct AppDispatcher<'a> {
    pub state: &'a mut AppState,
    pub sdi: &'a mut SdiRegistry,
    pub vfs: &'a mut MemoryVfs,
    /// Rendering backend, for `screenshot` (`read_pixels`).
    pub screen: &'a mut dyn SdiCore,
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
    /// Every launchable app: built-in, plugin-registered and the Browser.
    fn tool_list_apps(&self) -> ToolResult {
        ToolResult::text(launch::launchable_titles(self.state).join("\n"))
    }

    fn tool_open_app(&mut self, args: &Value) -> ToolResult {
        let Some(title) = arg_str(args, "title") else {
            return ToolResult::error("missing required argument: title");
        };
        let file = arg_str(args, "file");
        match launch::open_by_title(
            self.state,
            self.sdi,
            self.vfs,
            &title,
            file.as_deref(),
            LaunchOpts::QUIET,
        ) {
            Ok(title) => match file {
                Some(file) => ToolResult::text(format!("Opened {title} with {file}")),
                None => ToolResult::text(format!("Opened {title}")),
            },
            Err(msg) => ToolResult::error(msg),
        }
    }

    fn tool_list_windows(&self) -> ToolResult {
        let wm = &self.state.wm;
        let active = wm.active_window().map(str::to_string);
        let windows: Vec<Value> = wm
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

    /// `close_window`: like the titlebar close button, the app is asked
    /// first; `force: true` closes it regardless (unsaved edits are lost).
    fn tool_close_window(&mut self, id: &str, args: &Value) -> ToolResult {
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        let how = CloseHow::Programmatic { force };
        match close_app_window(self.state, self.sdi, self.vfs, id, how) {
            CloseOutcome::Closed => ToolResult::text(format!("closed {id}")),
            CloseOutcome::Declined => ToolResult::error(format!(
                "{id} declined to close: the app is asking for confirmation (e.g. unsaved \
                 changes; see screenshot). Answer its prompt, or pass force: true to close \
                 it anyway and discard them."
            )),
            CloseOutcome::NotFound => ToolResult::error(format!("no window with id '{id}'")),
        }
    }

    fn tool_window_op(&mut self, op: &str, args: &Value) -> ToolResult {
        let Some(id) = arg_str(args, "id") else {
            return ToolResult::error("missing required argument: id");
        };
        let wm = &mut self.state.wm;
        if wm.get_window(&id).is_none() {
            return ToolResult::error(format!("no window with id '{id}'"));
        }
        match op {
            "focus" => wm_result(wm.focus_window(&id, self.sdi), format!("focused {id}")),
            "close" => self.tool_close_window(&id, args),
            "minimize" => wm_result(wm.minimize_window(&id, self.sdi), format!("minimized {id}")),
            "maximize" => wm_result(wm.maximize_window(&id, self.sdi), format!("maximized {id}")),
            "restore" => wm_result(wm.restore_window(&id, self.sdi), format!("restored {id}")),
            _ => ToolResult::error(format!("unknown window op: {op}")),
        }
    }

    fn tool_move_window(&mut self, args: &Value) -> ToolResult {
        let (Some(id), Some(x), Some(y)) =
            (arg_str(args, "id"), arg_i64(args, "x"), arg_i64(args, "y"))
        else {
            return ToolResult::error("requires: id (string), x (int), y (int)");
        };
        let wm = &mut self.state.wm;
        let Some(win) = wm.get_window(&id) else {
            return ToolResult::error(format!("no window with id '{id}'"));
        };
        // move_window takes a delta; convert absolute target to a delta.
        // Bound the target first: the WM clamps the result on screen, but
        // the delta itself must not overflow (`x: i64::MAX` panicked).
        const LIMIT: i64 = 1 << 20;
        let dx = (x.clamp(-LIMIT, LIMIT) - i64::from(win.x)) as i32;
        let dy = (y.clamp(-LIMIT, LIMIT) - i64::from(win.y)) as i32;
        wm_result(
            wm.move_window(&id, dx, dy, self.sdi),
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
        let wm = &mut self.state.wm;
        if wm.get_window(&id).is_none() {
            return ToolResult::error(format!("no window with id '{id}'"));
        }
        let w = w.clamp(80, 4096) as u32;
        let h = h.clamp(60, 4096) as u32;
        wm_result(
            wm.resize_window(&id, w, h, self.sdi),
            format!("resized {id} to {w}x{h}"),
        )
    }

    fn tool_run_command(&mut self, args: &Value) -> ToolResult {
        let Some(command) = arg_str(args, "command") else {
            return ToolResult::error("missing required argument: command");
        };
        let mut pending_skin = None;
        let text = {
            let AppState {
                ref mut terminal,
                ref mut content,
                ref platform,
                ref net,
                ref skin,
                ..
            } = *self.state;
            let mut env = Environment {
                cwd: terminal.cwd.clone(),
                vfs: &mut *self.vfs,
                power: Some(platform),
                time: Some(platform),
                usb: Some(platform),
                network: None,
                tls: Some(&net.tls_provider),
                stdin: None,
                stderr: String::new(),
            };
            let result = terminal.cmd_reg.execute(&command, &mut env);
            terminal.cwd = env.cwd;
            crate::commands::format_remote_response(
                result,
                &mut content.browser,
                skin,
                self.sdi,
                &mut pending_skin,
            )
        };
        // A `skin` command resolved a new skin; apply it with the whole
        // state at hand (see `format_remote_response`).
        if let Some(new_skin) = pending_skin {
            crate::commands::apply_skin_object(new_skin, self.state, self.sdi, self.vfs);
        }
        ToolResult::text(text)
    }

    fn tool_browser_navigate(&mut self, args: &Value) -> ToolResult {
        let Some(url) = arg_str(args, "url") else {
            return ToolResult::error("missing required argument: url");
        };
        // Ensure a Browser widget exists (open it if needed).
        if self.state.content.browser.is_none() {
            launch::launch_title(
                self.state,
                self.sdi,
                self.vfs,
                BROWSER_TITLE,
                LaunchOpts::QUIET,
            );
        }
        match self.state.content.browser.as_mut() {
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
            .write(oasis_core::ipc::AUDIO_REQUEST_PATH, request.as_bytes())
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
                // The guide owns the channel catalogs; the shell resolves
                // the request against it (tv_controller::tune).
                let guide_open = self
                    .state
                    .content
                    .open_runners
                    .iter()
                    .any(|(_, r)| r.title == "TV Guide");
                if !guide_open {
                    return ToolResult::error(
                        "the TV Guide is not open (open_app 'TV Guide' first)",
                    );
                }
                (oasis_core::ipc::TV_REQUEST_PATH, format!("tune_ch:{n}"))
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
        let st = &*self.state;
        let windows: Vec<Value> = st
            .wm
            .windows()
            .iter()
            .map(|w| json!({ "id": w.id.as_str(), "title": w.title }))
            .collect();
        let state = json!({
            "mode": format!("{:?}", st.mode),
            "skin": st.skin.manifest.name,
            "focused_window": st.wm.active_window(),
            "open_windows": windows,
            "browser_url": st.content.browser.as_ref().and_then(|b| b.current_url().map(str::to_string)),
            "agent_calls": st.agent_activity.call_count,
        });
        ToolResult::text(serde_json::to_string_pretty(&state).unwrap_or_else(|_| "{}".to_string()))
    }

    fn tool_screenshot(&mut self) -> ToolResult {
        // The live framebuffer size: skins are scaled to the configured
        // resolution, so the manifest's native size can be smaller than
        // the screen (the screenshot then only captured the top-left
        // corner).
        let (w, h) = (
            self.state.active_theme.screen_w,
            self.state.active_theme.screen_h,
        );
        let rgba = match self.screen.read_pixels(0, 0, w, h) {
            Ok(px) => px,
            Err(e) => return ToolResult::error(format!("read_pixels: {e}")),
        };
        // Force alpha opaque (the framebuffer has no real transparency).
        let mut opaque = rgba;
        for px in opaque.as_chunks_mut::<4>().0 {
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
            ToolSpec::new(
                "close_window",
                "Close a window. Like the titlebar close button, the app is asked first: an \
                 app with unsaved changes shows its confirmation prompt and the call fails \
                 with 'declined'. Pass force: true to close it anyway (unsaved changes are \
                 lost).",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Window id" },
                        "force": {
                            "type": "boolean",
                            "description": "Close without asking the app (default false)"
                        }
                    },
                    "required": ["id"]
                }),
            ),
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
        self.state.agent_activity.record(name);
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
