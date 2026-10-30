//! Per-frame VFS IPC plumbing for the WASM host: app requests and the
//! `wm` / `open` terminal commands' window-manager queue.
//!
//! Mirrors the desktop shell (`oasis-app` `shell.rs` / `commands.rs`):
//!
//! - Requests of apps that closed (or were replaced) during the frame are
//!   carried out first, before those of the apps still open, because they
//!   were made first (a skin picked right before Escape). TV tunes from a
//!   closed guide are dropped: there is no guide left to play them in.
//! - `/var/audio/request` and `/var/wm/request` are line queues with
//!   several producers per frame (apps, the `music` / `wm` / `open`
//!   commands): requests are appended, never written over one another.

use oasis_core::apps::{AppRequest, AppRunner};
use oasis_core::error::Result;
use oasis_core::ipc::{AUDIO_REQUEST_PATH, WM_REQUEST_PATH, WM_STATUS_PATH, WmRequest};
use oasis_core::vfs::{MemoryVfs, Vfs};
use oasis_core::wm::manager::WindowManager;

/// Every request to carry out this frame: `closed` (drained from apps that
/// went away this frame, emptied) first, minus their TV tunes, then each
/// open runner's in window order.
pub(crate) fn gather(
    closed: &mut Vec<AppRequest>,
    open: &mut [(String, AppRunner)],
) -> Vec<AppRequest> {
    let mut requests = std::mem::take(closed);
    requests.retain(|req| oasis_apps::tv_guide::tune_url_request(req).is_none());
    for (_, runner) in open {
        runner.drain_requests(&mut requests);
    }
    requests
}

/// Carry out a generic app request: append to the line-queued request
/// files, write any other path.
pub(crate) fn write(vfs: &mut MemoryVfs, req: &AppRequest) -> Result<()> {
    if req.path == AUDIO_REQUEST_PATH || req.path == WM_REQUEST_PATH {
        append_line(vfs, &req.path, &req.data)
    } else {
        vfs.write(&req.path, req.data.as_bytes())
    }
}

/// Append one line to the line-queued request file `path` (the
/// `oasis_audio::queue_request` convention, for any queue path).
fn append_line(vfs: &mut MemoryVfs, path: &str, line: &str) -> Result<()> {
    if path == AUDIO_REQUEST_PATH {
        return oasis_core::audio::queue_request(vfs, line);
    }
    let mut data = vfs.read(path).unwrap_or_default();
    if !data.is_empty() && !data.ends_with(b"\n") {
        data.push(b'\n');
    }
    data.extend_from_slice(line.trim_end_matches('\n').as_bytes());
    vfs.write(path, &data)
}

/// Take every queued `wm` request (one per line) out of
/// [`WM_REQUEST_PATH`], creating `/var/wm` on first use. Malformed lines
/// are logged and dropped.
pub(crate) fn take_wm_requests(vfs: &mut MemoryVfs) -> Vec<WmRequest> {
    if !vfs.exists("/var/wm") && vfs.mkdir("/var/wm").is_err() {
        return Vec::new();
    }
    let Ok(data) = vfs.read(WM_REQUEST_PATH) else {
        return Vec::new();
    };
    if data.is_empty() {
        return Vec::new();
    }
    let _ = vfs.write(WM_REQUEST_PATH, b"");
    String::from_utf8_lossy(&data)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let req = WmRequest::parse(line);
            if req.is_none() {
                log::warn!("malformed wm request {line:?}");
            }
            req
        })
        .collect()
}

/// Publish the window list to [`WM_STATUS_PATH`] (only when it changed):
/// one line per window, `id | title | state | x,y wxh`, the focused window
/// marked with `*` -- the format `wm list` prints on the desktop shell.
pub(crate) fn publish_wm_status(vfs: &mut MemoryVfs, wm: &WindowManager) {
    let status = if wm.window_count() == 0 {
        "(no windows)".to_string()
    } else {
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
    };
    if vfs.read(WM_STATUS_PATH).ok().as_deref() != Some(status.as_bytes()) {
        let _ = vfs.write(WM_STATUS_PATH, status.as_bytes());
    }
}

impl crate::OasisWasm {
    /// Service the `wm` / `open` commands' IPC: apply every queued
    /// [`WmRequest`] and publish the window list (the desktop shell's
    /// `commands::poll_wm_ipc`). Without this `wm` and `open` were no-ops
    /// in the browser build.
    pub(crate) fn poll_wm_ipc(&mut self) {
        for req in take_wm_requests(&mut self.vfs) {
            self.apply_wm_request(req);
        }
        publish_wm_status(&mut self.vfs, &self.wm);
    }

    fn apply_wm_request(&mut self, req: WmRequest) {
        let id = match req {
            WmRequest::Open { app, file } => {
                match file {
                    Some(file) => match self.apps.find(&app).map(|r| r.title.clone()) {
                        Some(title) => self.launch_app_window_for_file(&title, &file),
                        None => self.output_lines.push(format!("open: no app named {app}")),
                    },
                    None => self.launch_app(&app),
                }
                return;
            },
            WmRequest::Close { ref id, .. }
            | WmRequest::Focus(ref id)
            | WmRequest::Minimize(ref id)
            | WmRequest::Maximize(ref id) => id.clone(),
        };
        if self.wm.get_window(&id).is_none() {
            log::warn!("wm request for unknown window {id:?}");
            return;
        }
        let result = match req {
            WmRequest::Close { force: true, .. } => {
                self.close_app_window(&id);
                Ok(())
            },
            // Without `--force`, same as the titlebar close button: an app
            // with unsaved changes answers with its prompt instead.
            WmRequest::Close { force: false, .. } => {
                if !self.request_close_window(&id) {
                    self.output_lines.push(format!(
                        "wm: {id} did not close: the app is asking for confirmation"
                    ));
                }
                Ok(())
            },
            WmRequest::Focus(_) => self.wm.focus_window(&id, &mut self.sdi),
            WmRequest::Minimize(_) => self.wm.minimize_window(&id, &mut self.sdi),
            WmRequest::Maximize(_) => self.wm.maximize_window(&id, &mut self.sdi),
            WmRequest::Open { .. } => Ok(()),
        };
        if let Err(e) = result {
            log::warn!("wm request on {id:?}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::vfs::MemoryVfs;

    fn vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/audio").unwrap();
        vfs.mkdir("/var/wm").unwrap();
        vfs
    }

    #[test]
    fn audio_requests_append_instead_of_overwriting() {
        let mut vfs = vfs();
        // A `music` command queued earlier in the frame...
        oasis_core::audio::queue_request(&mut vfs, "vol 40").unwrap();
        // ...then two app requests in the same frame.
        write(
            &mut vfs,
            &AppRequest::new(AUDIO_REQUEST_PATH, "play /music/a.mp3"),
        )
        .unwrap();
        write(&mut vfs, &AppRequest::new(AUDIO_REQUEST_PATH, "pause")).unwrap();
        let data = String::from_utf8(vfs.read(AUDIO_REQUEST_PATH).unwrap()).unwrap();
        assert_eq!(data, "vol 40\nplay /music/a.mp3\npause");
    }

    #[test]
    fn wm_requests_append_and_drain_in_order() {
        let mut vfs = vfs();
        vfs.write(WM_REQUEST_PATH, b"open Calculator").unwrap();
        write(
            &mut vfs,
            &AppRequest::new(WM_REQUEST_PATH, "close --force paint"),
        )
        .unwrap();
        write(&mut vfs, &AppRequest::new(WM_REQUEST_PATH, "bogus")).unwrap();
        write(
            &mut vfs,
            &AppRequest::new(WM_REQUEST_PATH, "minimize editor"),
        )
        .unwrap();
        let reqs = take_wm_requests(&mut vfs);
        assert_eq!(
            reqs,
            vec![
                WmRequest::Open {
                    app: "Calculator".into(),
                    file: None
                },
                WmRequest::Close {
                    id: "paint".into(),
                    force: true
                },
                WmRequest::Minimize("editor".into()),
            ]
        );
        // Drained: the next frame sees nothing.
        assert!(take_wm_requests(&mut vfs).is_empty());
    }

    #[test]
    fn other_request_paths_hold_the_latest_request() {
        let mut vfs = vfs();
        vfs.mkdir("/var/radio").unwrap();
        write(&mut vfs, &AppRequest::new("/var/radio/request", "tune 1")).unwrap();
        write(&mut vfs, &AppRequest::new("/var/radio/request", "tune 2")).unwrap();
        assert_eq!(vfs.read("/var/radio/request").unwrap(), b"tune 2");
    }

    #[test]
    fn closed_apps_requests_come_first_without_tv_tunes() {
        let mut closed = vec![
            AppRequest::new(AUDIO_REQUEST_PATH, "play /music/a.mp3"),
            AppRequest::new(
                oasis_apps::tv_guide::TV_REQUEST_PATH,
                "tune_url http://x/v.mp4",
            ),
            AppRequest::new("/var/skin/request", "xp"),
        ];
        let reqs = gather(&mut closed, &mut []);
        assert!(closed.is_empty(), "closed requests are consumed");
        assert_eq!(
            reqs,
            vec![
                AppRequest::new(AUDIO_REQUEST_PATH, "play /music/a.mp3"),
                AppRequest::new("/var/skin/request", "xp"),
            ]
        );
    }

    #[test]
    fn wm_status_lists_windows() {
        let mut vfs = vfs();
        let wm = WindowManager::new(480, 272);
        publish_wm_status(&mut vfs, &wm);
        assert_eq!(vfs.read(WM_STATUS_PATH).unwrap(), b"(no windows)");
    }
}
