//! Window & UI control commands: wm, sdi, theme, notify, screenshot.

use oasis_types::error::OasisError;
#[cfg(test)]
use oasis_types::error::Result;

use oasis_types::ipc::{
    NOTIFY_PATH, Notification, NotifyLevel, SCREENSHOT_REQUEST_PATH, THEME_STATUS_PATH,
    WM_REQUEST_PATH, WM_STATUS_PATH, WmRequest,
};

use crate::audio_commands::queue_request_line;
use crate::interpreter::{CommandOutput, CommandSignal};

// ---------------------------------------------------------------------------
// wm
// ---------------------------------------------------------------------------

const WM_USAGE: &str = "wm [list|close [--force] <id>|focus <id>|minimize <id>|maximize <id>]";

define_command!(
    WmCmd,
    "wm",
    "Window manager control",
    WM_USAGE,
    "ui",
    |args, env| {
        let subcmd = args.first().copied().unwrap_or("list");
        match subcmd {
            "list" => {
                // Read window state from VFS if available.
                if env.vfs.exists(WM_STATUS_PATH) {
                    let data = env.vfs.read(WM_STATUS_PATH)?;
                    Ok(CommandOutput::Text(
                        String::from_utf8_lossy(&data).into_owned(),
                    ))
                } else {
                    Ok(CommandOutput::Text(
                        "(no window manager status available)".to_string(),
                    ))
                }
            },
            "close" | "focus" | "minimize" | "maximize" => {
                // `--force` (close only, either side of the id) skips the
                // app's unsaved-changes prompt.
                let force = subcmd == "close" && args[1..].contains(&"--force");
                let id = args[1..]
                    .iter()
                    .copied()
                    .find(|a| !(force && *a == "--force"))
                    .unwrap_or("");
                if id.is_empty() {
                    let flag = if subcmd == "close" { " [--force]" } else { "" };
                    return Err(OasisError::Command(
                        format!("usage: wm {subcmd}{flag} <window-id>").into(),
                    ));
                }
                let request = if force {
                    format!("close --force {id}")
                } else {
                    format!("{subcmd} {id}")
                };
                queue_request_line(env.vfs, WM_REQUEST_PATH, &request)?;
                Ok(CommandOutput::Text(format!("WM request: {request}")))
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}\nusage: {WM_USAGE}").into(),
            )),
        }
    }
);

// ---------------------------------------------------------------------------
// sdi
// ---------------------------------------------------------------------------

define_command!(
    SdiCmd,
    "sdi",
    "Inspect SDI scene objects",
    "sdi [list|get <name>]",
    "ui",
    |args, _env| {
        // The scene registry lives in the host, so both subcommands are
        // signals the host resolves into text (`terminal_sdi::inspect_sdi`).
        let subcmd = args.first().copied().unwrap_or("list");
        match subcmd {
            "list" => Ok(CommandOutput::Signal(CommandSignal::SdiInspect {
                name: None,
            })),
            "get" => {
                let name = args.get(1).copied().unwrap_or("");
                if name.is_empty() {
                    return Err(OasisError::Command("usage: sdi get <name>".into()));
                }
                Ok(CommandOutput::Signal(CommandSignal::SdiInspect {
                    name: Some(name.to_string()),
                }))
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}").into(),
            )),
        }
    }
);

// ---------------------------------------------------------------------------
// theme
// ---------------------------------------------------------------------------

define_command!(
    ThemeCmd,
    "theme",
    "Show or modify current theme",
    "theme [show|colors]",
    "ui",
    |args, env| {
        let subcmd = args.first().copied().unwrap_or("show");
        match subcmd {
            "show" | "colors" => {
                let theme_path = THEME_STATUS_PATH;
                if env.vfs.exists(theme_path) {
                    let data = env.vfs.read(theme_path)?;
                    Ok(CommandOutput::Text(
                        String::from_utf8_lossy(&data).into_owned(),
                    ))
                } else {
                    Ok(CommandOutput::Text(
                        "Current theme: (use 'skin' to switch themes)".to_string(),
                    ))
                }
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}").into(),
            )),
        }
    }
);

// ---------------------------------------------------------------------------
// notify
// ---------------------------------------------------------------------------

define_command!(
    NotifyCmd,
    "notify",
    "Show a notification message",
    "notify [--level info|success|warning|error] <message>",
    "ui",
    |args, env| {
        if args.is_empty() {
            return Err(OasisError::Command(
                "usage: notify [--level info|success|warning|error] <message>".into(),
            ));
        }

        // Parse optional --level flag.
        let (level_str, message_args) = if args.len() >= 3 && args[0] == "--level" {
            (args[1], &args[2..])
        } else {
            ("info", args)
        };

        if message_args.is_empty() {
            return Err(OasisError::Command(
                "usage: notify [--level info|success|warning|error] <message>".into(),
            ));
        }

        let Some(level) = NotifyLevel::parse(level_str) else {
            return Err(OasisError::Command(
                format!("unknown level: {level_str} (use info|success|warning|error)").into(),
            ));
        };
        let notification = Notification {
            level,
            message: message_args.join(" "),
        };

        // Write level and message to VFS for the UI layer to pick up.
        env.vfs
            .write(NOTIFY_PATH, notification.to_payload().as_bytes())?;
        Ok(CommandOutput::Text(format!(
            "Notification queued [{}]: {}",
            level.as_str(),
            notification.message
        )))
    }
);

// ---------------------------------------------------------------------------
// screenshot
// ---------------------------------------------------------------------------

define_command!(
    ScreenshotCmd,
    "screenshot",
    "Take a screenshot",
    "screenshot [path]",
    "ui",
    |args, env| {
        let path = args.first().copied().unwrap_or("/tmp/screenshot.bmp");
        let req_path = SCREENSHOT_REQUEST_PATH;
        env.vfs.write(req_path, path.as_bytes())?;
        Ok(CommandOutput::Text(format!(
            "Screenshot request queued: {path}"
        )))
    }
);

// ---------------------------------------------------------------------------
// open
// ---------------------------------------------------------------------------

const OPEN_USAGE: &str = "open <app> [file]";

/// Whether the trailing `open` argument `arg` names a file rather than the
/// last word of the app title. App titles are plain words ("Text
/// Editor"), so anything path-like (a `/`, a `.` as in `./x` or an
/// extension) is a file, and so is a bare word naming an existing entry
/// in the working directory (`open Text Editor notes`).
fn is_file_arg(arg: &str, cwd: &str, vfs: &dyn oasis_vfs::Vfs) -> bool {
    arg.contains('/')
        || arg.contains('.')
        || vfs.exists(&crate::interpreter::resolve_path(cwd, arg))
}

define_command!(
    OpenCmd,
    "open",
    "Open an app window, optionally with a file",
    OPEN_USAGE,
    "ui",
    |args, env| {
        // App titles contain spaces ("Text Editor"), so every argument
        // belongs to the title except a trailing file, which may be
        // relative to the working directory (the host has no cwd).
        let (title_args, file) = match args.split_last() {
            Some((last, rest)) if !rest.is_empty() && is_file_arg(last, &env.cwd, env.vfs) => {
                (rest, Some(crate::interpreter::resolve_path(&env.cwd, last)))
            },
            _ => (args, None),
        };
        let app = title_args.join(" ");
        if app.is_empty() {
            return Err(OasisError::Command(format!("usage: {OPEN_USAGE}").into()));
        }
        let request = WmRequest::Open {
            app: app.clone(),
            file: file.clone(),
        };
        queue_request_line(env.vfs, WM_REQUEST_PATH, &request.to_line())?;
        Ok(CommandOutput::Text(match file {
            Some(f) => format!("Opening {f} in {app}"),
            None => format!("Opening {app}"),
        }))
    }
);

register_commands!(
    register_ui_commands,
    [WmCmd, SdiCmd, ThemeCmd, NotifyCmd, ScreenshotCmd, OpenCmd]
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
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
        reg.execute(line, &mut env)
    }

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/wm").unwrap();
        vfs.mkdir("/var/notify").unwrap();
        vfs.mkdir("/var/screenshot").unwrap();
        (reg, vfs)
    }

    #[test]
    fn wm_list_no_status() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "wm list").unwrap());
        assert!(s.contains("no window manager"));
    }

    #[test]
    fn wm_close_no_id() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(exec(&reg, &mut vfs, "wm close").is_err());
    }

    #[test]
    fn wm_close_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "wm close browser").unwrap());
        assert!(s.contains("close browser"));
        let data = vfs.read("/var/wm/request").unwrap();
        assert_eq!(data, b"close browser");
    }

    #[test]
    fn wm_close_force_queues_forced_request() {
        let (reg, mut vfs) = setup();
        for line in ["wm close --force editor", "wm close editor --force"] {
            vfs.write(WM_REQUEST_PATH, b"").unwrap();
            exec(&reg, &mut vfs, line).unwrap();
            assert_eq!(vfs.read(WM_REQUEST_PATH).unwrap(), b"close --force editor");
        }
        assert!(exec(&reg, &mut vfs, "wm close --force").is_err());
    }

    /// Requests queue one per line: a second command before the host polls
    /// must not overwrite the first (`open Calculator; open Paint`).
    #[test]
    fn wm_requests_queue_instead_of_overwriting() {
        let (reg, mut vfs) = setup();
        exec(&reg, &mut vfs, "open Calculator").unwrap();
        exec(&reg, &mut vfs, "wm close editor").unwrap();
        let data = String::from_utf8(vfs.read(WM_REQUEST_PATH).unwrap()).unwrap();
        assert_eq!(data, "open Calculator\nclose editor");
    }

    #[test]
    fn open_queues_an_open_request() {
        let (reg, mut vfs) = setup();
        exec(&reg, &mut vfs, "open Text Editor").unwrap();
        assert_eq!(vfs.read(WM_REQUEST_PATH).unwrap(), b"open Text Editor");
        vfs.write(WM_REQUEST_PATH, b"").unwrap();
        exec(&reg, &mut vfs, "open Photo Viewer /home/user/a.png").unwrap();
        let line = String::from_utf8(vfs.read(WM_REQUEST_PATH).unwrap()).unwrap();
        assert_eq!(
            WmRequest::parse(&line),
            Some(WmRequest::Open {
                app: "Photo Viewer".into(),
                file: Some("/home/user/a.png".into()),
            })
        );
        assert!(exec(&reg, &mut vfs, "open").is_err());
    }

    /// `open Text Editor notes.txt` used to treat the relative file as
    /// part of the app title; it now resolves against the cwd.
    #[test]
    fn open_resolves_relative_files_against_cwd() {
        let (reg, mut vfs) = setup();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        vfs.write("/home/user/todo", b"x").unwrap();
        let open_line = |vfs: &mut MemoryVfs, cwd: &str, line: &str| {
            vfs.write(WM_REQUEST_PATH, b"").unwrap();
            let mut env = Environment {
                cwd: cwd.to_string(),
                vfs,
                power: None,
                time: None,
                usb: None,
                network: None,
                tls: None,
                stdin: None,
                stderr: String::new(),
            };
            reg.execute(line, &mut env).unwrap();
            let line = String::from_utf8(vfs.read(WM_REQUEST_PATH).unwrap()).unwrap();
            WmRequest::parse(&line).unwrap()
        };
        let open = |app: &str, file: Option<&str>| WmRequest::Open {
            app: app.into(),
            file: file.map(str::to_string),
        };
        let cwd = "/home/user";
        assert_eq!(
            open_line(&mut vfs, cwd, "open Text Editor notes.txt"),
            open("Text Editor", Some("/home/user/notes.txt"))
        );
        assert_eq!(
            open_line(&mut vfs, cwd, "open Text Editor ../shared/a.md"),
            open("Text Editor", Some("/home/shared/a.md"))
        );
        assert_eq!(
            open_line(&mut vfs, cwd, "open Paint ./pic"),
            open("Paint", Some("/home/user/pic"))
        );
        // An extension-less name counts when it exists in the cwd...
        assert_eq!(
            open_line(&mut vfs, cwd, "open Text Editor todo"),
            open("Text Editor", Some("/home/user/todo"))
        );
        // ...otherwise it is part of the title.
        assert_eq!(
            open_line(&mut vfs, cwd, "open Text Editor"),
            open("Text Editor", None)
        );
        // A lone argument is always the app, even with a dot.
        assert_eq!(open_line(&mut vfs, cwd, "open v1.2"), open("v1.2", None));
    }

    #[test]
    fn sdi_list_signals_host() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let out = exec(&reg, &mut vfs, "sdi").unwrap();
        assert!(matches!(
            out,
            CommandOutput::Signal(CommandSignal::SdiInspect { name: None })
        ));
    }

    #[test]
    fn sdi_get_signals_host_with_name() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let out = exec(&reg, &mut vfs, "sdi get term_prompt").unwrap();
        let CommandOutput::Signal(CommandSignal::SdiInspect { name }) = out else {
            panic!("expected SdiInspect signal, got {out:?}");
        };
        assert_eq!(name.as_deref(), Some("term_prompt"));
        assert!(exec(&reg, &mut vfs, "sdi get").is_err());
    }

    #[test]
    fn theme_show() {
        let mut reg = CommandRegistry::new();
        register_ui_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "theme").unwrap());
        assert!(s.contains("theme"));
    }

    #[test]
    fn notify_queues_message() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "notify Hello World").unwrap());
        assert!(s.contains("Hello World"));
        let data = vfs.read(NOTIFY_PATH).unwrap();
        assert_eq!(data, b"info:Hello World");
    }

    #[test]
    fn notify_with_level() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "notify --level error Something broke").unwrap());
        assert!(s.contains("[error]"));
        let data = vfs.read(NOTIFY_PATH).unwrap();
        assert_eq!(data, b"error:Something broke");
    }

    #[test]
    fn notify_unknown_level() {
        let (reg, mut vfs) = setup();
        let result = exec(&reg, &mut vfs, "notify --level banana oops");
        assert!(result.is_err());
    }

    #[test]
    fn screenshot_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "screenshot /tmp/shot.bmp").unwrap());
        assert!(s.contains("shot.bmp"));
    }
}
