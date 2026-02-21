# Plugin Development Guide

This guide walks through writing a plugin for OASIS_OS. Plugins extend the system with new commands, UI widgets, and behaviors at runtime.

---

## Plugin Architecture

Plugins interact with the OS through three services provided by `PluginHost`:

- **`sdi`** -- the SDI scene graph for creating/modifying UI elements
- **`vfs`** -- the virtual file system for reading/writing files
- **`commands`** -- the command registry for adding terminal commands

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

```rust
pub struct PluginHost<'a> {
    pub sdi: &'a mut SdiRegistry,      // scene graph
    pub vfs: &'a mut dyn Vfs,          // virtual file system
    pub commands: &'a mut CommandRegistry, // command registry
}
```

All OS interaction goes through these three fields. Plugins do not have direct access to backends, rendering, or input -- they operate at the scene-graph and VFS level.

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
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds).unwrap();

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
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds).unwrap();

        assert!(sdi.contains("greet_banner"));
    }

    #[test]
    fn greet_plugin_cleanup() {
        let mut mgr = PluginManager::new();
        mgr.register_static(Box::new(GreetPlugin));

        let mut sdi = SdiRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut cmds = CommandRegistry::new();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds).unwrap();
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds).unwrap();

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
| `NotepadPlugin` | `note [list\|read\|write]` | VFS-backed notepad with CRUD operations |

Study these for patterns on SDI widget creation, VFS interaction, and command registration.
