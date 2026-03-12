# Plugin Development Guide

This guide walks through writing a plugin for OASIS_OS. Plugins extend the system with new commands, UI widgets, and behaviors at runtime.

---

## Plugin Architecture

Plugins interact with the OS through services provided by `PluginHost`:

- **`sdi`** -- the SDI scene graph for creating/modifying UI elements
- **`vfs`** -- the virtual file system for reading/writing files
- **`commands`** -- the command registry for adding terminal commands
- **`audio`** -- audio playback (optional, `None` in headless mode)
- **`network`** -- TCP networking (optional, `None` if unavailable)
- **`backend`** -- rendering backend for texture loading (optional)

The plugin lifecycle is:

1. **Register** -- plugin is added to `PluginManager` (state: `Registered`)
2. **Init** -- `init()` called once; register commands, create SDI objects (state: `Active`)
3. **Update** -- `update()` called once per frame; do periodic work
4. **Shutdown** -- `shutdown()` called on unload; clean up resources (state: `Stopped`)

Source: `crates/oasis-core/src/plugin/traits.rs`

---

## Tutorial: Write Your First Plugin

### Step 1: Implement the Plugin Trait

```rust
use oasis_core::plugin::{Plugin, PluginHost, PluginInfo};
use oasis_core::error::Result;

pub struct GreetPlugin;

impl Plugin for GreetPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo::new("greet", "1.0.0")
            .with_author("Your Name")
            .with_description("Greeting plugin example")
    }

    fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
        // Register a command.
        host.commands.register(Box::new(GreetCmd));

        // Create a UI widget in the scene graph.
        let obj = host.sdi.create("greet_banner");
        obj.x = 10;
        obj.y = 250;
        obj.w = 200;
        obj.h = 16;
        obj.text = Some("Greet plugin loaded".to_string());
        obj.visible = true;

        Ok(())
    }

    fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        // Per-frame work. Most plugins leave this as a no-op.
        Ok(())
    }

    fn shutdown(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
        // Clean up SDI objects. Commands remain registered
        // (the registry does not support removal yet).
        let _ = host.sdi.destroy("greet_banner");
        Ok(())
    }
}
```

### Step 2: Implement a Command

Commands implement the `Command` trait from `oasis-terminal`:

```rust
use oasis_core::terminal::{Command, CommandOutput, Environment};
use oasis_core::error::Result;

struct GreetCmd;

impl Command for GreetCmd {
    fn name(&self) -> &str { "greet" }
    fn description(&self) -> &str { "Greet someone (greet plugin)" }
    fn usage(&self) -> &str { "greet [name]" }
    fn category(&self) -> &str { "plugin" }

    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        let name = if args.is_empty() { "World" } else { args[0] };
        Ok(CommandOutput::Text(format!("Greetings, {name}!")))
    }
}
```

### Step 3: Register the Plugin

For built-in (statically linked) plugins, add to the registration function in `crates/oasis-core/src/plugin/examples.rs`:

```rust
pub fn register_builtin_plugins(manager: &mut PluginManager) {
    manager.register_static(Box::new(HelloPlugin));
    manager.register_static(Box::new(ClockWidgetPlugin::new()));
    manager.register_static(Box::new(NotepadPlugin));
    manager.register_static(Box::new(GreetPlugin));  // your plugin
}
```

---

## VFS-Based IPC Patterns

Plugins communicate through the virtual file system. This avoids coupling between plugins and provides a natural persistence mechanism.

### Writing State to VFS

```rust
fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
    // Create a plugin-specific directory.
    if !host.vfs.exists("/var/my-plugin") {
        if !host.vfs.exists("/var") {
            host.vfs.mkdir("/var")?;
        }
        host.vfs.mkdir("/var/my-plugin")?;
    }
    // Write config or state.
    host.vfs.write("/var/my-plugin/config.txt", b"key=value")?;
    Ok(())
}
```

### Reading State from VFS

```rust
fn execute(&self, _args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
    let data = env.vfs.read("/var/my-plugin/config.txt")?;
    let text = String::from_utf8_lossy(&data).into_owned();
    Ok(CommandOutput::Text(text))
}
```

### Inter-Plugin Communication

Plugin A writes to `/var/shared/messages`, Plugin B reads from it. Both plugins share the same VFS instance via `PluginHost`. No global state or synchronization is needed -- the VFS serializes access.

See the `NotepadPlugin` in `crates/oasis-core/src/plugin/examples.rs` for a complete working example of VFS-backed data storage.

---

## The PluginHost and PluginContext

`PluginHost` is the struct passed to every lifecycle method:

See the full `PluginHost` struct definition below.

In addition to the three core fields, `PluginHost` provides optional access to platform services:

```rust
pub struct PluginHost<'a> {
    pub sdi: &'a mut SdiRegistry,
    pub vfs: &'a mut dyn Vfs,
    pub commands: &'a mut CommandRegistry,
    pub audio: Option<&'a mut dyn AudioBackend>,     // audio playback
    pub network: Option<&'a mut dyn NetworkBackend>,  // TCP networking
    pub backend: Option<&'a mut dyn SdiCore>,         // rendering/textures
}
```

The optional fields are `None` in headless or test contexts. Always check for `Some` before using them.

### Texture Loading

Plugins can load textures for use with SDI objects via the rendering backend:

```rust
fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
    // Load a 16x16 RGBA texture.
    let rgba_data = vec![0xFFu8; 16 * 16 * 4];
    let tex = host.load_texture(16, 16, &rgba_data)?;
    self.texture = Some(tex);
    Ok(())
}

fn shutdown(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
    if let Some(tex) = self.texture.take() {
        host.destroy_texture(tex)?;
    }
    Ok(())
}
```

---

## Plugin Discovery via Manifests

Plugins can be discovered from the VFS at `/etc/oasis-os/plugins/<name>/plugin.toml`:

```toml
name = "my-plugin"
version = "2.0"
author = "Your Name"
description = "A discoverable plugin"
library = "libmyplugin.so"
auto_load = true
```

Use `PluginManager::discover_manifests(vfs)` to scan for available plugins.

Source: `crates/oasis-core/src/plugin/manager.rs`

---

## Testing Plugins with Mock VFS

Use `MemoryVfs` for isolated, in-memory testing:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::plugin::PluginManager;
    use oasis_core::sdi::SdiRegistry;
    use oasis_core::terminal::CommandRegistry;
    use oasis_core::vfs::MemoryVfs;

    #[test]
    fn greet_plugin_registers_command() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(GreetPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        let mut env = Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        match cmds.execute("greet OASIS", &mut env).unwrap() {
            CommandOutput::Text(s) => assert_eq!(s, "Greetings, OASIS!"),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn greet_plugin_creates_sdi_widget() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(GreetPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        assert!(sdi.contains("greet_banner"));
    }

    #[test]
    fn greet_plugin_cleanup() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(GreetPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);

        assert!(!sdi.contains("greet_banner"));
    }
}
```

---

## Built-in Plugin Examples

The codebase ships three example plugins in `crates/oasis-core/src/plugin/examples.rs`:

| Plugin | Commands | Description |
|--------|----------|-------------|
| `HelloPlugin` | `hello [name]` | Simplest possible plugin -- single command |
| `ClockWidgetPlugin` | `pclock [show\|hide]` | Creates an SDI clock widget + command |
| `NotepadPlugin` | `note [list\|read\|write]` | VFS-backed notepad with CRUD operations + dashboard app |

Study these for patterns on SDI widget creation, VFS interaction, and command registration.

---

## Plugin App Bridge

Plugins can register as launchable dashboard apps using the **plugin-to-app bridge**. This allows plugin code to appear as a first-class app on the dashboard, with its own icon, title, and full `App` trait implementation.

Source: `crates/oasis-core/src/plugin/app_bridge.rs`

### Registering a Plugin App

During `init()`, call `host.register_app()` with a `PluginAppRegistration`:

```rust
use oasis_core::plugin::app_bridge::{AppCategory, PluginAppRegistration};

fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
    host.register_app(
        PluginAppRegistration::new(
            "My App",
            AppCategory::Utility,
            |path, _vfs| {
                Box::new(MyPluginApp::new(path))
            },
        )
        .with_color(oasis_types::backend::Color {
            r: 100, g: 200, b: 50, a: 255,
        }),
    );
    Ok(())
}
```

The factory closure is called each time the user launches the app from the dashboard.

### App Categories

| Category | Description |
|----------|-------------|
| `AppCategory::Utility` | Tools, editors, utilities |
| `AppCategory::Media` | Music, video, photo apps |
| `AppCategory::Game` | Games and entertainment |
| `AppCategory::System` | System tools and monitors |
| `AppCategory::Network` | Network and communication |
| `AppCategory::Other` | Uncategorized |

### Implementing the App Trait

Your plugin app must implement the `App` trait from `oasis_core::apps::app_trait`:

```rust
use oasis_core::apps::{App, AppAction, ContentState};
use oasis_core::input::Button;
use oasis_core::vfs::Vfs;

struct MyPluginApp {
    content: ContentState,
    counter: i32,
}

impl MyPluginApp {
    fn new(path: &str) -> Self {
        let mut content = ContentState::new("My App", path);
        content.lines = vec![
            "My Plugin App".to_string(),
            String::new(),
            "Counter: 0".to_string(),
            String::new(),
            "UP/DOWN to change, CANCEL to exit.".to_string(),
        ];
        Self { content, counter: 0 }
    }

    fn refresh_lines(&mut self) {
        self.content.lines[2] = format!("Counter: {}", self.counter);
    }
}

impl App for MyPluginApp {
    fn title(&self) -> &str { &self.content.title }
    fn path(&self) -> &str { &self.content.app_path }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => AppAction::Exit,
            Button::Up => { self.counter += 1; self.refresh_lines(); AppAction::None }
            Button::Down => { self.counter -= 1; self.refresh_lines(); AppAction::None }
            _ => AppAction::None,
        }
    }

    fn lines(&self) -> &[String] { &self.content.lines }

    // ... implement remaining required trait methods
    // (see ContentState helpers for default implementations)
}
```

The `NotepadPlugin` in `examples.rs` demonstrates registering a `SimpleApp`-based plugin app on the dashboard.

---

## API Versioning

Plugin API compatibility is tracked via `PLUGIN_API_VERSION` (currently `1`). The version is checked at load time -- plugins compiled against a different API version will be rejected by the manager.

```rust
let info = PluginInfo::new("my-plugin", "2.0.0");
// info.api_version defaults to PLUGIN_API_VERSION
```

**Policy:**
- `PLUGIN_API_VERSION` is incremented only on **breaking changes** to the `Plugin` trait or `PluginHost` struct.
- Additive changes (new `Option` fields on `PluginHost`, new methods with defaults) do **not** bump the version.
- Plugins should always use the default `api_version` from `PluginInfo::new()` unless they need to target a specific older API.

Source: `crates/oasis-core/src/plugin/traits.rs`

---

## EventBus IPC

In addition to VFS-based IPC, plugins can communicate via the **event bus** -- a publish/subscribe system for real-time, topic-based messaging.

Source: `crates/oasis-core/src/plugin/event_bus.rs`

### Publishing Events

```rust
use oasis_core::plugin::event_bus::{EventBus, PluginEvent};

fn update(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
    // Publish an event to a topic.
    let event = PluginEvent {
        topic: "sensor.temperature".to_string(),
        source: "temp-monitor".to_string(),
        data: "72.5".to_string(),
    };
    host.event_bus.publish(event);
    Ok(())
}
```

### Subscribing to Events

```rust
fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
    // Subscribe to a topic. Returns a receiver for incoming events.
    let rx = host.event_bus.subscribe("sensor.temperature");
    self.temp_rx = Some(rx);
    Ok(())
}

fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
    // Check for events (non-blocking).
    if let Some(ref rx) = self.temp_rx {
        while let Ok(event) = rx.try_recv() {
            // Handle event.data
        }
    }
    Ok(())
}
```

Events are string-based for cross-language compatibility. The event bus is useful for real-time notifications (e.g., state changes, sensor data), while VFS-based IPC remains better for persistent state and file-like data exchange.

---

## Plugin Configuration via Manifests

Plugin manifests support a `[config]` section with typed values via `PluginConfigValue`:

```toml
# plugin.toml
name = "my-plugin"
version = "2.0"
author = "Your Name"
description = "A configurable plugin"
library = "libmyplugin.so"
auto_load = true

[config]
refresh_interval = 30
theme = "dark"
auto_start = true
opacity = 0.8
```

### Accessing Config Values

Configuration is available as a `HashMap<String, PluginConfigValue>` on the manifest:

```rust
use oasis_core::plugin::manager::PluginConfigValue;

fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
    // Read config from the manifest (if discovered via plugin.toml).
    if let Some(PluginConfigValue::Int(interval)) = self.config.get("refresh_interval") {
        self.refresh_ms = *interval as u64;
    }
    if let Some(PluginConfigValue::Str(theme)) = self.config.get("theme") {
        self.theme = theme.clone();
    }
    Ok(())
}
```

### PluginConfigValue Variants

| Variant | Rust Type | TOML Example |
|---------|-----------|-------------|
| `Bool(bool)` | `bool` | `auto_start = true` |
| `Int(i64)` | `i64` | `refresh_interval = 30` |
| `Float(f64)` | `f64` | `opacity = 0.8` |
| `Str(String)` | `String` | `theme = "dark"` |
