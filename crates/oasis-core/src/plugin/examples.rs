//! Built-in example plugins demonstrating the plugin API.
//!
//! These serve as both functional examples and integration tests for the
//! plugin system. They are statically linked and can be registered via
//! `register_builtin_plugins()`.

use crate::error::{OasisError, Result};
use crate::terminal::{Command, CommandOutput, Environment};

use super::traits::{Plugin, PluginHost, PluginInfo};

// ---------------------------------------------------------------------------
// HelloPlugin -- simplest possible plugin
// ---------------------------------------------------------------------------

/// A minimal plugin that registers a single "hello" command.
pub struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo::new("hello", "1.0.0")
            .with_author("OASIS_OS")
            .with_description("Hello world example plugin")
    }

    fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
        host.commands.register(Box::new(HelloCmd));
        Ok(())
    }

    fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        // Command remains registered (CommandRegistry doesn't support removal).
        // In a real implementation, the registry would track plugin ownership.
        Ok(())
    }
}

struct HelloCmd;
impl Command for HelloCmd {
    fn name(&self) -> &str {
        "hello"
    }
    fn description(&self) -> &str {
        "Say hello (example plugin)"
    }
    fn usage(&self) -> &str {
        "hello [name]"
    }
    fn category(&self) -> &str {
        "plugin"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        let name = if args.is_empty() { "World" } else { args[0] };
        Ok(CommandOutput::Text(format!("Hello, {name}!")))
    }
}

// ---------------------------------------------------------------------------
// ClockWidgetPlugin -- creates an SDI clock widget + command
// ---------------------------------------------------------------------------

/// Plugin that draws a live clock in the top-right corner of the screen
/// (two SDI overlay objects, positioned from [`PluginContext`]) and
/// registers a `pclock [show|hide]` command.
///
/// The command reaches the plugin through the VFS ([`PCLOCK_VISIBLE_PATH`]):
/// commands only get an `Environment`, never the plugin itself.
///
/// [`PluginContext`]: super::PluginContext
pub struct ClockWidgetPlugin {
    /// Whether the widget has been created.
    widget_created: bool,
}

impl ClockWidgetPlugin {
    pub fn new() -> Self {
        Self {
            widget_created: false,
        }
    }
}

impl Default for ClockWidgetPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// SDI object holding the clock text.
pub const CLOCK_WIDGET_NAME: &str = "plugin_clock";
/// SDI object behind the text (a translucent rounded pill).
pub const CLOCK_WIDGET_BG_NAME: &str = "plugin_clock_bg";
/// `1` / `0`: whether the clock is shown (written by `pclock show|hide`).
/// The widget is opt-in: anything but `1` (including no file) hides it, so
/// the example plugin never covers the dashboard unless asked to.
pub const PCLOCK_VISIBLE_PATH: &str = "/var/pclock/visible";

const CLOCK_W: u32 = 60;
const CLOCK_H: u32 = 14;
/// Below a typical status bar + tab row, clear of the bar's own clock.
const CLOCK_TOP: i32 = 46;
/// Above everything else in the overlay pass (toasts, menus).
const CLOCK_Z: i32 = 100_000;

impl ClockWidgetPlugin {
    /// Lay the widget out for `host.context` and refresh its text and
    /// visibility.
    fn refresh(host: &mut PluginHost<'_>) {
        let ctx = host.context;
        // An unknown screen size (0) falls back to the PSP-native 480.
        let screen_w = if ctx.screen_w == 0 { 480 } else { ctx.screen_w };
        let x = screen_w.saturating_sub(CLOCK_W + 6) as i32;
        let text = ctx.time.map_or_else(
            || "--:--:--".to_string(),
            |t| format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second),
        );
        let visible = host.vfs.read(PCLOCK_VISIBLE_PATH).is_ok_and(|d| d == b"1");
        if let Ok(bg) = host.sdi.get_mut(CLOCK_WIDGET_BG_NAME) {
            bg.x = x;
            bg.y = CLOCK_TOP;
            bg.visible = visible;
        }
        if let Ok(obj) = host.sdi.get_mut(CLOCK_WIDGET_NAME) {
            obj.x = x + 6;
            obj.y = CLOCK_TOP + 3;
            obj.visible = visible;
            if obj.text.as_deref() != Some(text.as_str()) {
                obj.text = Some(text);
            }
        }
    }
}

impl Plugin for ClockWidgetPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo::new("clock-widget", "1.0.0")
            .with_author("OASIS_OS")
            .with_description("Clock display widget and pclock command")
    }

    fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
        let bg = host.sdi.create(CLOCK_WIDGET_BG_NAME);
        bg.w = CLOCK_W;
        bg.h = CLOCK_H;
        bg.color = crate::backend::Color::rgba(0, 0, 0, 150);
        bg.border_radius = Some(4);
        bg.overlay = true;
        bg.z = CLOCK_Z;
        let obj = host.sdi.create(CLOCK_WIDGET_NAME);
        obj.font_size = 8;
        obj.text_color = crate::backend::Color::WHITE;
        obj.overlay = true;
        obj.z = CLOCK_Z + 1;
        self.widget_created = true;
        Self::refresh(host);

        // Register the pclock command.
        host.commands.register(Box::new(PclockCmd));
        Ok(())
    }

    fn update(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
        if self.widget_created {
            // Only a changed second rewrites the text, so an idle shell
            // redraws once a second for the clock, not every frame.
            Self::refresh(host);
        }
        Ok(())
    }

    fn shutdown(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
        if self.widget_created {
            for name in [CLOCK_WIDGET_NAME, CLOCK_WIDGET_BG_NAME] {
                if let Err(e) = host.sdi.destroy(name) {
                    log::debug!("destroy({name}) during plugin shutdown: {e}");
                }
            }
            self.widget_created = false;
        }
        Ok(())
    }
}

struct PclockCmd;
impl Command for PclockCmd {
    fn name(&self) -> &str {
        "pclock"
    }
    fn description(&self) -> &str {
        "Show or hide the clock widget (clock-widget plugin)"
    }
    fn usage(&self) -> &str {
        "pclock [show|hide]"
    }
    fn category(&self) -> &str {
        "plugin"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("status");
        let visible = match subcmd {
            "show" => true,
            "hide" => false,
            "status" => {
                let shown = !env.vfs.read(PCLOCK_VISIBLE_PATH).is_ok_and(|d| d == b"0");
                return Ok(CommandOutput::Text(format!(
                    "Clock widget plugin active ({}). Use 'pclock show' or 'pclock hide'.",
                    if shown { "visible" } else { "hidden" }
                )));
            },
            _ => {
                return Err(OasisError::Command(
                    format!("unknown subcommand: {subcmd}\nusage: {}", self.usage()).into(),
                ));
            },
        };
        for dir in ["/var", "/var/pclock"] {
            if !env.vfs.exists(dir) {
                env.vfs.mkdir(dir)?;
            }
        }
        env.vfs
            .write(PCLOCK_VISIBLE_PATH, if visible { b"1" } else { b"0" })?;
        Ok(CommandOutput::Text(format!(
            "Clock widget: {}",
            if visible { "visible" } else { "hidden" }
        )))
    }
}

// ---------------------------------------------------------------------------
// NotepadPlugin -- VFS-backed notepad with read/write commands
// ---------------------------------------------------------------------------

/// Plugin that provides a simple notepad backed by VFS files.
/// Demonstrates VFS interaction from a plugin.
pub struct NotepadPlugin;

const NOTEPAD_DIR: &str = "/var/notepad";

impl Plugin for NotepadPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo::new("notepad", "1.0.0")
            .with_author("OASIS_OS")
            .with_description("Simple notepad with VFS-backed notes")
    }

    fn init(&mut self, host: &mut PluginHost<'_>) -> Result<()> {
        // Ensure the notepad directory exists.
        if !host.vfs.exists(NOTEPAD_DIR) {
            // Create parent directories if needed.
            if !host.vfs.exists("/var") {
                host.vfs.mkdir("/var")?;
            }
            host.vfs.mkdir(NOTEPAD_DIR)?;
        }
        host.commands.register(Box::new(NoteCmd));

        // The `note` terminal command is the user-facing surface for
        // this plugin. We deliberately do not register a dashboard
        // app — the GUI Text Editor is the primary note-taking UI
        // and a second CLI-placeholder "Notepad" tile just duplicates
        // the slot.
        Ok(())
    }

    fn update(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self, _host: &mut PluginHost<'_>) -> Result<()> {
        // Notes persist in VFS -- don't delete them on shutdown.
        Ok(())
    }
}

/// Validate and sanitize a note name to prevent path traversal.
fn sanitize_note_name(name: &str) -> Result<&str> {
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(OasisError::Command(
            "note name must not contain path separators".into(),
        ));
    }
    Ok(name)
}

struct NoteCmd;
impl Command for NoteCmd {
    fn name(&self) -> &str {
        "note"
    }
    fn description(&self) -> &str {
        "Read/write notes (notepad plugin)"
    }
    fn usage(&self) -> &str {
        "note [list|read <name>|write <name> <text>]"
    }
    fn category(&self) -> &str {
        "plugin"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("list");

        match subcmd {
            "list" => {
                if !env.vfs.exists(NOTEPAD_DIR) {
                    return Ok(CommandOutput::Text("(no notes)".to_string()));
                }
                let entries = env.vfs.readdir(NOTEPAD_DIR)?;
                if entries.is_empty() {
                    return Ok(CommandOutput::Text("(no notes)".to_string()));
                }
                let mut lines = Vec::new();
                lines.push("Notes:".to_string());
                for e in &entries {
                    lines.push(format!("  {}", e.name));
                }
                Ok(CommandOutput::Text(lines.join("\n")))
            },
            "read" => {
                let name = args.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return Err(OasisError::Command("usage: note read <name>".into()));
                }
                let name = sanitize_note_name(name)?;
                let path = format!("{NOTEPAD_DIR}/{name}");
                let data = env.vfs.read(&path)?;
                let text = String::from_utf8_lossy(&data).into_owned();
                Ok(CommandOutput::Text(text))
            },
            "write" => {
                let name = args.get(1).copied().unwrap_or("");
                if name.is_empty() || args.len() < 3 {
                    return Err(OasisError::Command(
                        "usage: note write <name> <text...>".into(),
                    ));
                }
                let name = sanitize_note_name(name)?;
                let text = args[2..].join(" ");
                let path = format!("{NOTEPAD_DIR}/{name}");
                env.vfs.write(&path, text.as_bytes())?;
                Ok(CommandOutput::Text(format!("Note '{name}' saved.")))
            },
            _ => Err(OasisError::Command(format!(
                "unknown subcommand: {subcmd}\nusage: note [list|read <name>|write <name> <text>]"
            ).into())),
        }
    }
}

/// Register all built-in example plugins.
pub fn register_builtin_plugins(manager: &mut super::PluginManager) {
    manager.register_static(Box::new(HelloPlugin));
    manager.register_static(Box::new(ClockWidgetPlugin::new()));
    manager.register_static(Box::new(NotepadPlugin));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manager::PluginManager;
    use crate::sdi::SdiRegistry;
    use crate::terminal::CommandRegistry;
    use crate::vfs::{MemoryVfs, Vfs};

    fn setup() -> (PluginManager, SdiRegistry, MemoryVfs, CommandRegistry) {
        let mut mgr = PluginManager::new();
        register_builtin_plugins(&mut mgr);
        (
            mgr,
            SdiRegistry::new(),
            MemoryVfs::new(),
            CommandRegistry::new(),
        )
    }

    #[test]
    fn register_builtin_count() {
        let (mgr, _, _, _) = setup();
        assert_eq!(mgr.count(), 3);
    }

    #[test]
    fn hello_plugin_registers_command() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
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
        let __out = cmds.execute("hello", &mut env).unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected text, got {__out:?}");
        };
        assert_eq!(s, "Hello, World!");
    }

    #[test]
    fn hello_plugin_with_name() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
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
        let __out = cmds.execute("hello OASIS", &mut env).unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected text, got {__out:?}");
        };
        assert_eq!(s, "Hello, OASIS!");
    }

    fn context(w: u32, second: u8) -> crate::plugin::PluginContext {
        crate::plugin::PluginContext {
            screen_w: w,
            screen_h: 272,
            time: Some(crate::platform::SystemTime {
                year: 2026,
                month: 10,
                day: 30,
                hour: 9,
                minute: 5,
                second,
            }),
        }
    }

    #[test]
    fn clock_widget_creates_sdi() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
        mgr.set_context(context(480, 7));
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        let obj = sdi.get(CLOCK_WIDGET_NAME).unwrap();
        assert_eq!(obj.text.as_deref(), Some("09:05:07"));
        assert!(obj.overlay);
        assert!(!obj.visible, "the example clock is opt-in (`pclock show`)");
        let bg = sdi.get(CLOCK_WIDGET_BG_NAME).unwrap();
        assert_eq!(bg.x + bg.w as i32, 480 - 6, "right-aligned on screen");
    }

    /// `update` used to be a no-op: the clock showed "00:00:00" forever.
    #[test]
    fn clock_widget_update_follows_time_and_screen() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
        mgr.set_context(context(480, 7));
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        mgr.set_context(context(1024, 8));
        mgr.update_all(&mut sdi, &mut vfs, &mut cmds);
        let obj = sdi.get(CLOCK_WIDGET_NAME).unwrap();
        assert_eq!(obj.text.as_deref(), Some("09:05:08"));
        let bg = sdi.get(CLOCK_WIDGET_BG_NAME).unwrap();
        assert_eq!(bg.x + bg.w as i32, 1024 - 6);
    }

    #[test]
    fn clock_widget_cleanup_on_shutdown() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert!(sdi.contains(CLOCK_WIDGET_NAME));

        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);
        assert!(!sdi.contains(CLOCK_WIDGET_NAME));
        assert!(!sdi.contains(CLOCK_WIDGET_BG_NAME));
    }

    #[test]
    fn pclock_hide_and_show_toggle_the_widget() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        let run = |vfs: &mut MemoryVfs, line: &str| {
            let mut env = Environment {
                cwd: "/".to_string(),
                vfs,
                power: None,
                time: None,
                usb: None,
                network: None,
                tls: None,
                stdin: None,
                stderr: String::new(),
            };
            match cmds.execute(line, &mut env).unwrap() {
                CommandOutput::Text(s) => s,
                other => panic!("expected text, got {other:?}"),
            }
        };
        assert!(run(&mut vfs, "pclock hide").contains("hidden"));
        assert!(run(&mut vfs, "pclock").contains("(hidden)"));
        let mut cmds2 = CommandRegistry::new();
        mgr.update_all(&mut sdi, &mut vfs, &mut cmds2);
        assert!(!sdi.get(CLOCK_WIDGET_NAME).unwrap().visible);
        assert!(!sdi.get(CLOCK_WIDGET_BG_NAME).unwrap().visible);
        run(&mut vfs, "pclock show");
        mgr.update_all(&mut sdi, &mut vfs, &mut cmds2);
        assert!(sdi.get(CLOCK_WIDGET_NAME).unwrap().visible);
    }

    #[test]
    fn pclock_command() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
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
        let __out = cmds.execute("pclock", &mut env).unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected text, got {__out:?}");
        };
        assert!(s.contains("plugin active"));
    }

    #[test]
    fn notepad_creates_directory() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        assert!(vfs.exists("/var/notepad"));
    }

    #[test]
    fn notepad_write_and_read() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
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
        // Write a note.
        let __out = cmds
            .execute("note write todo Fix the bug", &mut env)
            .unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected text, got {__out:?}");
        };
        assert!(s.contains("saved"));

        // Read it back.
        let __out = cmds.execute("note read todo", &mut env).unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected text, got {__out:?}");
        };
        assert_eq!(s, "Fix the bug");
    }

    #[test]
    fn notepad_list_notes() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
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
        // Initially empty.
        let __out = cmds.execute("note list", &mut env).unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected text, got {__out:?}");
        };
        assert!(s.contains("no notes"));

        // Write a note and list again.
        cmds.execute("note write memo Remember to test", &mut env)
            .unwrap();
        let __out = cmds.execute("note list", &mut env).unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected text, got {__out:?}");
        };
        assert!(s.contains("memo"));
    }

    #[test]
    fn notepad_read_missing() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
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
        assert!(cmds.execute("note read nonexistent", &mut env).is_err());
    }

    #[test]
    fn notepad_notes_persist_after_shutdown() {
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);

        // Write a note.
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
        cmds.execute("note write test Hello", &mut env).unwrap();

        // Shutdown plugins.
        mgr.shutdown_all(&mut sdi, &mut vfs, &mut cmds);

        // Note file should still exist in VFS.
        assert!(vfs.exists("/var/notepad/test"));
    }

    #[test]
    fn notepad_does_not_register_dashboard_app() {
        // The Notepad plugin intentionally does NOT register a
        // dashboard app — its `note` terminal command is the CLI
        // surface, while the GUI Text Editor covers the dashboard
        // slot. Keep this test to prevent accidental re-introduction
        // of a second CLI-placeholder "Notepad" tile.
        let (mut mgr, mut sdi, mut vfs, mut cmds) = setup();
        mgr.init_all(&mut sdi, &mut vfs, &mut cmds);
        let apps = mgr.plugin_apps();
        assert!(
            apps.iter().all(|a| a.title != "Notepad"),
            "Notepad plugin must not register a dashboard app (duplicates Text Editor)",
        );
    }
}
