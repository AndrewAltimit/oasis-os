//! The `wm` and `plugin` terminal commands' VFS IPC: request files the
//! commands queue lines on, and status files the shell publishes.

use oasis_core::ipc::{WM_REQUEST_PATH, WM_STATUS_PATH, WmRequest};
use oasis_core::sdi::SdiRegistry;
use oasis_core::vfs::{MemoryVfs, Vfs};

use super::skin::rebuild_dashboard;
use super::{publish_if_changed, take_request};
use crate::app_state::AppState;
use crate::windows::{CloseHow, CloseOutcome, close_app_window, exit_kiosk};

/// Service the `plugin` terminal command's VFS IPC: apply every
/// [`PluginRequest`](oasis_core::ipc::PluginRequest) queued on
/// [`PLUGIN_REQUEST_PATH`](oasis_core::ipc::PLUGIN_REQUEST_PATH) (`load`
/// initializes a registered or unloaded plugin again, `unload` shuts it
/// down and removes its commands, apps and SDI objects), report each
/// outcome in the terminal, and publish the plugin list at
/// [`PLUGIN_STATUS_PATH`](oasis_core::ipc::PLUGIN_STATUS_PATH). Without
/// this bridge `plugin list` had no data and `load` / `unload` were
/// silently dropped.
pub fn poll_plugin_ipc(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    use oasis_core::ipc::{PLUGIN_REQUEST_PATH, PLUGIN_STATUS_PATH, PluginRequest};
    if !vfs.exists("/var/plugin") && vfs.mkdir("/var/plugin").is_err() {
        return;
    }
    let mut changed = false;
    if let Some(requests) = take_request(vfs, PLUGIN_REQUEST_PATH) {
        for line in requests.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let Some(req) = PluginRequest::parse(line) else {
                state
                    .terminal
                    .print(format!("plugin: malformed request {line:?}"));
                continue;
            };
            let pm = &mut state.plugin_manager;
            let cmds = &mut state.terminal.cmd_reg;
            let (name, verb, result) = match &req {
                PluginRequest::Load(name) => (name, "loaded", pm.init_plugin(name, sdi, vfs, cmds)),
                PluginRequest::Unload(name) => {
                    (name, "unloaded", pm.stop_plugin(name, sdi, vfs, cmds))
                },
            };
            let msg = match result {
                Ok(()) => {
                    changed = true;
                    format!("Plugin {name} {verb}.")
                },
                Err(_) if !pm.is_loaded(name) => format!(
                    "plugin: no plugin named '{name}' (plugins are statically linked; \
                     there is no dynamic loader)"
                ),
                Err(e) => format!("plugin: {e}"),
            };
            state.terminal.print(msg);
        }
        state.terminal.dirty = true;
    }
    if changed {
        // Plugin apps come and go with their plugin: re-sync the registry
        // and the dashboard tiles.
        state
            .plugin_manager
            .extend_registry(&mut state.content.apps);
        rebuild_dashboard(state, vfs);
    }
    publish_if_changed(vfs, PLUGIN_STATUS_PATH, &state.plugin_manager.status_text());
}

/// Run every active plugin's per-frame `update`, with the current screen
/// size and wall-clock time as its [`PluginContext`](oasis_core::plugin::PluginContext).
pub fn update_plugins(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    use oasis_core::platform::TimeService as _;
    state
        .plugin_manager
        .set_context(oasis_core::plugin::PluginContext {
            screen_w: state.config.screen_width,
            screen_h: state.config.screen_height,
            time: state.platform.now().ok(),
        });
    state
        .plugin_manager
        .update_all(sdi, vfs, &mut state.terminal.cmd_reg);
}

/// Service the `wm` terminal command's VFS IPC (local, remote terminal and
/// MCP `run_command` alike): publish the window list to
/// [`WM_STATUS_PATH`] and apply the [`WmRequest`]s (`close [--force]|focus|
/// minimize|maximize <id>`, `open <app>[\t<file>]`) from
/// [`WM_REQUEST_PATH`]. Without this bridge `wm` was a no-op in the
/// desktop shell (and failed outright: `/var/wm` did not exist).
pub fn poll_wm_ipc(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    if !vfs.exists("/var/wm") && vfs.mkdir("/var/wm").is_err() {
        return;
    }
    // One request per line: `open Calculator; open Paint` queues both
    // before this runs.
    if let Some(requests) = take_request(vfs, WM_REQUEST_PATH) {
        for request in requests.lines().map(str::trim).filter(|l| !l.is_empty()) {
            apply_wm_request(request, state, sdi, vfs);
        }
    }
    publish_if_changed(vfs, WM_STATUS_PATH, &wm_status_text(&state.wm));
}

fn apply_wm_request(request: &str, state: &mut AppState, sdi: &mut SdiRegistry, vfs: &MemoryVfs) {
    let Some(req) = WmRequest::parse(request) else {
        log::warn!("malformed wm request {request:?}");
        return;
    };
    // `open <app> [file]`: launch (or raise) an app window by title.
    if let WmRequest::Open { app, file } = &req {
        let opened = crate::launch::open_by_title(
            state,
            sdi,
            vfs,
            app,
            file.as_deref(),
            crate::launch::LaunchOpts::USER_NO_FADE,
        );
        if let Err(msg) = opened {
            state.terminal.print(format!("open: {msg}"));
            state.terminal.dirty = true;
        }
        return;
    }
    let id = match &req {
        WmRequest::Close { id, .. }
        | WmRequest::Focus(id)
        | WmRequest::Minimize(id)
        | WmRequest::Maximize(id) => id.as_str(),
        WmRequest::Open { .. } => return,
    };
    if state.wm.get_window(id).is_none() {
        log::warn!("wm request for unknown window {id:?}");
        return;
    }
    let result = match &req {
        // Without `--force`, same as the titlebar close button: an app with
        // unsaved changes answers with its prompt instead of losing them.
        WmRequest::Close { force, .. } => {
            let how = CloseHow::Programmatic { force: *force };
            if close_app_window(state, sdi, vfs, id, how) == CloseOutcome::Declined {
                state.terminal.print(format!(
                    "wm: {id} did not close: the app is asking for confirmation"
                ));
                state.terminal.dirty = true;
            }
            Ok(())
        },
        // Restores a minimized window first (focusing alone made the
        // invisible window the keyboard target).
        WmRequest::Focus(_) => {
            crate::launch::raise_window(&mut state.wm, id, sdi);
            Ok(())
        },
        // Geometry changes leave kiosk fullscreen first, so the shell
        // chrome comes back with the window's frame.
        WmRequest::Minimize(_) => {
            exit_kiosk(state, sdi, id);
            state.wm.minimize_window(id, sdi)
        },
        WmRequest::Maximize(_) => {
            exit_kiosk(state, sdi, id);
            state.wm.maximize_window(id, sdi)
        },
        WmRequest::Open { .. } => Ok(()),
    };
    if let Err(e) = result {
        log::warn!("wm {request:?}: {e}");
    }
}

/// One line per window: `id | title | state | x,y wxh`, the focused
/// window marked with `*`.
fn wm_status_text(wm: &oasis_core::wm::manager::WindowManager) -> String {
    if wm.window_count() == 0 {
        return "(no windows)".to_string();
    }
    let active = wm.active_window();
    wm.windows()
        .iter()
        .map(|w| {
            let mark = if active == Some(w.id.as_str()) {
                "*"
            } else {
                " "
            };
            format!(
                "{mark}{} | {} | {:?} | {},{} {}x{}",
                w.id.as_str(),
                w.title,
                w.state,
                w.x,
                w.y,
                w.outer_w,
                w.outer_h
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
