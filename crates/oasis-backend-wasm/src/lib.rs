//! WebAssembly backend for OASIS_OS.
//!
//! Renders to an HTML `<canvas>` element using the Canvas 2D API,
//! maps DOM events to `InputEvent`, and provides Web Audio playback.

pub mod audio;
pub mod font;
pub mod input;
pub mod network;
pub mod platform;
pub mod renderer;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use oasis_core::active_theme::ActiveTheme;
use oasis_core::backend::{AudioBackend, Color, InputBackend, SdiBackend};
use oasis_core::dashboard::{DashboardConfig, DashboardState, discover_apps};
use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::sdi::SdiRegistry;
use oasis_core::skin::Skin;
use oasis_core::terminal::{
    CommandOutput, CommandRegistry, Environment, populate_man_pages, populate_motd,
    populate_profile, register_builtins,
};
use oasis_core::vfs::{MemoryVfs, Vfs};

use audio::WasmAudioBackend;
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
    #[allow(dead_code)]
    audio: WasmAudioBackend,
    #[allow(dead_code)]
    network: WasmNetworkBackend,
    sdi: SdiRegistry,
    cmd_reg: CommandRegistry,
    vfs: MemoryVfs,
    #[allow(dead_code)]
    platform: WasmPlatform,
    #[allow(dead_code)]
    skin: Option<Skin>,
    active_theme: ActiveTheme,
    dashboard: Option<DashboardState>,
    cwd: String,
    output_lines: Vec<String>,
    width: u32,
    height: u32,
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

        let width = 480;
        let height = 272;
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

        // Scene graph and commands.
        let mut sdi = SdiRegistry::new();
        let mut cmd_reg = CommandRegistry::new();
        register_builtins(&mut cmd_reg);

        // VFS with demo content.
        let mut vfs = MemoryVfs::new();
        populate_wasm_vfs(&mut vfs);

        // Resolve skin.
        let skin_ref = skin_name.as_deref().unwrap_or("classic");
        let skin = oasis_skin::resolve_skin(skin_ref).ok();

        let active_theme = skin
            .as_ref()
            .map(|s| ActiveTheme::from_skin(&s.theme))
            .unwrap_or_default();

        // Apply skin layout and discover apps.
        let dashboard = if let Some(ref skin) = skin {
            skin.apply_layout(&mut sdi);
            let apps = discover_apps(&vfs, "/apps", None).unwrap_or_default();
            let dash_config = DashboardConfig::from_features(&skin.features, &active_theme);
            Some(DashboardState::new(dash_config, apps))
        } else {
            None
        };

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
            dashboard,
            cwd: "/".to_string(),
            output_lines: Vec::new(),
            width,
            height,
        })
    }

    /// Advance the OS state by one frame.
    ///
    /// Call this from `requestAnimationFrame`. Processes input events,
    /// updates the scene graph, and renders to the canvas.
    pub fn tick(&mut self, _delta_seconds: f32) {
        // Process queued input events.
        let events = self.input.poll_events();

        for event in &events {
            match event {
                InputEvent::ButtonPress(btn) => {
                    if let Some(ref mut dashboard) = self.dashboard {
                        match btn {
                            Button::Up | Button::Down | Button::Left | Button::Right => {
                                dashboard.handle_input(btn);
                            },
                            Button::Confirm => {
                                // App launch would happen here.
                            },
                            _ => {},
                        }
                    }
                },
                InputEvent::TriggerPress(Trigger::Right) => {
                    if let Some(ref mut dashboard) = self.dashboard {
                        dashboard.next_page();
                    }
                },
                InputEvent::TriggerPress(Trigger::Left) => {
                    if let Some(ref mut dashboard) = self.dashboard {
                        dashboard.prev_page();
                    }
                },
                _ => {},
            }
        }

        // Update SDI.
        if let Some(ref dashboard) = self.dashboard {
            dashboard.update_sdi(&mut self.sdi, &self.active_theme);
        }

        // Render.
        let _ = self.backend.clear(Color::rgb(10, 10, 18));
        let _ = self.sdi.draw(&mut self.backend);
        let _ = self.backend.swap_buffers();
    }

    /// Execute a terminal command and return the output.
    pub fn send_command(&mut self, cmd: &str) -> String {
        let mut env = Environment {
            cwd: self.cwd.clone(),
            vfs: &mut self.vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };

        let result = match self.cmd_reg.execute(cmd, &mut env) {
            Ok(output) => match output {
                CommandOutput::Text(t) => t,
                CommandOutput::Clear => {
                    self.output_lines.clear();
                    String::new()
                },
                _ => String::new(),
            },
            Err(e) => format!("error: {e}"),
        };

        // Persist cwd changes (e.g. from `cd`).
        self.cwd = env.cwd;
        result
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
    let _ = vfs.mkdir("/apps");
    let _ = vfs.mkdir("/apps/file_manager");
    let _ = vfs.mkdir("/apps/settings");
    let _ = vfs.mkdir("/apps/browser");
    let _ = vfs.mkdir("/apps/music_player");
    let _ = vfs.mkdir("/apps/terminal");

    // Demo startup script.
    let _ = vfs.write(
        "/home/user/startup.sh",
        b"# OASIS_OS startup script\necho Welcome back!\nls /apps\n",
    );
}
