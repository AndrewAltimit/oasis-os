//! Helper functions for input dispatch: dashboard confirm, terminal confirm,
//! TV Guide confirm.

use oasis_backend_psp::SdiRegistry;
use oasis_backend_psp::threading::IoHandle;
use oasis_backend_psp::{
    AudioCmd, AudioHandle, IoCmd, PspBackend, SfxId, TvCatalogRequest, WindowManager,
};

use oasis_core::active_theme::ActiveTheme;
use oasis_core::dashboard::DashboardState;
use oasis_core::skin::SkinFeatures;

use crate::app_states::{
    BrowserState, FileManagerState, MusicPlayerState, PhotoViewerState, RadioState, SettingsState,
    TvGuideState,
};
use crate::commands;
use crate::desktop;
use crate::skins;
use crate::types::{APPS, KioskApp};

// ---------------------------------------------------------------------------
// Helper: Dashboard Confirm (app launch)
// ---------------------------------------------------------------------------

/// Launch an app from the dashboard.
///
/// Apps with fullscreen views open as kiosk WM windows.  Apps without
/// fullscreen views open as regular floating windows.
#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_dashboard_confirm(
    title: &str,
    kiosk_app: &mut KioskApp,
    _dashboard: &mut DashboardState,
    wm: &mut WindowManager,
    sdi: &mut SdiRegistry,
    _audio: &AudioHandle,
    io: &IoHandle,
    fm: &mut FileManagerState,
    pv: &mut PhotoViewerState,
    mp: &mut MusicPlayerState,
    br: &mut BrowserState,
    radio: &mut RadioState,
    tv: &mut TvGuideState,
    settings: &mut SettingsState,
    current_preset: &skins::PspSkinPreset,
    backend: &mut PspBackend,
    dbg_log: &dyn Fn(&str),
) {
    match title {
        "Terminal" => {
            desktop::open_app_window(wm, sdi, "terminal", "Terminal", true);
            *kiosk_app = KioskApp::Terminal;
        },
        "File Manager" => {
            fm.left.path = String::from("ms0:/");
            fm.left.loaded = false;
            fm.right.path = fm.left.path.clone();
            fm.right.loaded = false;
            fm.active_panel = 0;
            desktop::open_app_window(wm, sdi, "filemgr", "File Manager", true);
            *kiosk_app = KioskApp::FileManager;
        },
        "Settings" => {
            // Position the cursor on the currently active theme so the user
            // can see what's selected before changing it.
            settings.selected = skins::PspSkinPreset::ALL
                .iter()
                .position(|p| p == current_preset)
                .unwrap_or(0);
            settings.scroll = 0;
            desktop::open_app_window(wm, sdi, "settings", "Settings", true);
            *kiosk_app = KioskApp::Settings;
        },
        "Photo Viewer" => {
            pv.viewing = false;
            pv.loaded = false;
            desktop::open_app_window(wm, sdi, "photos", "Photo Viewer", true);
            *kiosk_app = KioskApp::PhotoViewer;
        },
        "Music Player" => {
            mp.loaded = false;
            desktop::open_app_window(wm, sdi, "music", "Music Player", true);
            *kiosk_app = KioskApp::MusicPlayer;
        },
        "Browser" => {
            dbg_log("[Browser] entering browser view");
            br.ensure_widget();
            br.loading = false;
            br.status_msg = String::from("Press [] to enter URL, X to navigate");
            desktop::open_app_window(wm, sdi, "browser", "Browser", true);
            *kiosk_app = KioskApp::Browser;
        },
        "Radio" => {
            radio.selected = 0;
            radio.scroll = 0;
            desktop::open_app_window(wm, sdi, "radio", "Radio", true);
            *kiosk_app = KioskApp::Radio;
        },
        "TV Guide" => {
            dbg_log("[TV] entering TV Guide view");
            if tv.channels.is_empty() {
                if !oasis_backend_psp::network::is_net_initialized() {
                    dbg_log("[TV] init network...");
                    // Check if WiFi is already connected (cmd_server auto-connect).
                    // If so, ensure_net_init won't show a dialog, so we must NOT
                    // call reinit_gu_frame (which is only safe after utility dialogs).
                    // Use sceNetApctlGetState directly — psp::net::is_connected()
                    // uses an internal flag that isn't set by raw sceNetApctlConnect.
                    let was_connected = {
                        let mut state = psp::sys::ApctlState::Disconnected;
                        unsafe { psp::sys::sceNetApctlGetState(&mut state) };
                        matches!(state, psp::sys::ApctlState::GotIp)
                    };
                    if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                        dbg_log(&format!("[TV] net init failed: {e}"));
                        if !was_connected {
                            backend.reinit_gu_frame();
                        }
                    } else {
                        dbg_log("[TV] net init OK");
                        if !was_connected {
                            backend.reinit_gu_frame();
                        }
                    }
                }
                dbg_log("[TV] parsing channel TOML...");
                if let Ok(config) = oasis_core::apps::tv_guide::ChannelConfig::from_toml(
                    oasis_core::apps::tv_guide::channel::DEFAULT_CHANNELS_TOML,
                ) {
                    dbg_log(&format!("[TV] parsed {} channels", config.channel.len()));
                    tv.channels = config.channel;
                    tv.catalogs = vec![None; tv.channels.len()];
                    let mut batch = Vec::new();
                    for (i, ch) in tv.channels.iter().enumerate() {
                        for src in &ch.source {
                            let api_path =
                                oasis_core::apps::tv_guide::ChannelCatalog::files_api_path(
                                    &src.item_id,
                                );
                            batch.push(TvCatalogRequest {
                                url: format!("http://archive.org{}", api_path,),
                                ch_idx: i,
                                item_id: src.item_id.clone(),
                                subfolder: src.subfolder.clone(),
                            });
                        }
                    }
                    io.send(IoCmd::TvCatalogFetchBatch { requests: batch });
                    dbg_log("[TV] catalog batch sent");
                } else {
                    dbg_log("[TV] TOML parse failed");
                }
            }
            tv.selected = 0;
            tv.scroll = 0;
            desktop::open_app_window(wm, sdi, "tvguide", "TV Guide", true);
            *kiosk_app = KioskApp::TvGuide;
        },
        _ => {
            // Apps without a kiosk view: open as regular windowed app.
            if let Some(app) = APPS.iter().find(|a| a.title == title) {
                desktop::open_app_window(wm, sdi, app.id, app.title, false);
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Helper: Terminal Confirm (command execution)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_terminal_confirm(
    backend: &mut PspBackend,
    term: &mut crate::app_states::TerminalState,
    audio: &AudioHandle,
    mp: &mut MusicPlayerState,
    usb_storage: &mut Option<psp::usb::UsbStorageMode>,
    config: &mut psp::config::Config,
    current_preset: &mut skins::PspSkinPreset,
    active_theme: &mut ActiveTheme,
    skin_features: &SkinFeatures,
    dashboard: &mut DashboardState,
) {
    let cmd = term.input.clone();
    term.lines.push(format!("> {}", cmd));
    let (output, used_dialog) = match cmd.trim() {
        "sfx click" => {
            audio.send(AudioCmd::PlaySfx(SfxId::Click));
            (vec!["SFX: click".into()], false)
        },
        "sfx nav" => {
            audio.send(AudioCmd::PlaySfx(SfxId::Navigate));
            (vec!["SFX: navigate".into()], false)
        },
        "sfx error" => {
            audio.send(AudioCmd::PlaySfx(SfxId::Error));
            (vec!["SFX: error".into()], false)
        },
        "save" => match commands::save_terminal_history(&term.lines) {
            Ok(()) => (vec!["State saved.".into()], true),
            Err(e) => (vec![format!("Save failed: {e}")], true),
        },
        "load" => match commands::load_terminal_history() {
            Ok(lines) => {
                term.lines.clear();
                term.lines.extend(lines);
                (vec!["State restored.".into()], true)
            },
            Err(e) => (vec![format!("Load failed: {e}")], true),
        },
        "usb mount" => {
            if usb_storage.is_some() {
                (vec!["USB storage already active.".into()], false)
            } else {
                match psp::usb::start_bus() {
                    Ok(()) => match psp::usb::UsbStorageMode::activate() {
                        Ok(handle) => {
                            *usb_storage = Some(handle);
                            (
                                vec!["USB storage mode active. Connect cable to PC.".into()],
                                false,
                            )
                        },
                        Err(e) => (vec![format!("USB activate failed: {e}")], false),
                    },
                    Err(e) => (vec![format!("USB bus start failed: {e}")], false),
                }
            }
        },
        "usb unmount" | "usb eject" => {
            if usb_storage.take().is_some() {
                (vec!["USB storage mode deactivated.".into()], false)
            } else {
                (vec!["USB storage not active.".into()], false)
            }
        },
        "usb" | "usb status" => {
            let connected = psp::usb::is_connected();
            let established = psp::usb::is_established();
            let active = usb_storage.is_some();
            (
                vec![
                    format!(
                        "USB cable: {}",
                        if connected {
                            "connected"
                        } else {
                            "disconnected"
                        }
                    ),
                    format!(
                        "Storage mode: {}",
                        if active { "ACTIVE" } else { "inactive" }
                    ),
                    format!("Host mounted: {}", if established { "yes" } else { "no" },),
                ],
                false,
            )
        },
        _ if cmd.trim().starts_with("play ") => {
            let path = cmd.trim().strip_prefix("play ").unwrap().trim();
            audio.send(AudioCmd::LoadAndPlay(path.to_string()));
            mp.file_name = path.to_string();
            (vec![format!("Playing: {}", path)], false)
        },
        "pause" => {
            audio.send(AudioCmd::Pause);
            (vec!["Paused.".into()], false)
        },
        "resume" => {
            audio.send(AudioCmd::Resume);
            (vec!["Resumed.".into()], false)
        },
        "stop" => {
            audio.send(AudioCmd::Stop);
            (vec!["Stopped.".into()], false)
        },
        "skin" => {
            let names: Vec<String> = skins::PspSkinPreset::ALL
                .iter()
                .map(|p| {
                    let marker = if *p == *current_preset { ">" } else { " " };
                    format!("{} {}", marker, p.name())
                })
                .collect();
            let mut out = vec!["Skins (use 'skin NAME'):".into()];
            out.extend(names);
            (out, false)
        },
        _ if cmd.trim().starts_with("skin ") => {
            let key = cmd.trim().strip_prefix("skin ").unwrap().trim();
            let preset = skins::PspSkinPreset::from_key(key);
            if skins::apply_skin_preset(
                preset,
                current_preset,
                active_theme,
                skin_features,
                dashboard,
                config,
            ) {
                (vec![format!("Skin changed to '{}'.", preset.name())], false)
            } else {
                (vec![format!("Already using '{}'.", key)], false)
            }
        },
        _ => {
            let r = commands::execute_command(&cmd, config);
            (r.lines, r.used_dialog)
        },
    };
    if used_dialog {
        backend.reinit_gu_frame();
    }
    for line in output {
        term.lines.push(line);
    }
    term.input.clear();
    term.scroll = 0;
    while term.lines.len() > 200 {
        term.lines.remove(0);
    }
}

// ---------------------------------------------------------------------------
// Helper: TV Guide Confirm (tune channel)
// ---------------------------------------------------------------------------

pub(super) fn dispatch_tv_confirm(
    tv: &mut TvGuideState,
    io: &IoHandle,
    backend: &mut PspBackend,
    dbg_log: &dyn Fn(&str),
) {
    if tv.tuned.is_some() || tv.downloading {
        return;
    }
    // Warn if previous download is still shutting down, but don't block.
    // The DOWNLOAD_CANCEL flag will be reset below, and the old I/O thread
    // download will exit at its next cancel check.
    if !oasis_backend_psp::threading::is_download_stopped() {
        dbg_log("[TV] note: previous download still stopping");
    }
    dbg_log(&format!(
        "[TV] X pressed, tuning ch {} (catalogs={})",
        tv.selected,
        tv.catalogs.len()
    ));
    // Pre-allocate the GU video texture now, while ~7.5 MB of partition
    // memory is still free. By the time the video thread reaches
    // `AvcDecoder::new` it will have consumed ~6.5 MB on the persistent
    // sceMpeg DDR workspace, leaving ~1 MB — too little for a 524 KB
    // texture. The texture is reused for every subsequent stream (CSC
    // stride is fixed at 512 for any ≤480p source), so this allocation
    // happens at most once per session.
    if backend.alloc_video_texture(512, 256).is_none() {
        oasis_backend_psp::video::vlog_force(
            "[TV] WARN: video texture pre-alloc failed; video will be audio-only",
        );
    }
    if tv.selected < tv.catalogs.len() {
        if let Some(catalog) = &tv.catalogs[tv.selected] {
            dbg_log(&format!(
                "[TV] catalog has {} episodes",
                catalog.episodes.len()
            ));
            let best = oasis_core::apps::tv_guide::select_smallest_with_max_width(
                &catalog.episodes, 20_000_000, 320, 480,
            );
            if let Some(ep) = best {
                dbg_log(&format!(
                    "[TV] episode: {} ({}x{} {}B)",
                    ep.title, ep.width, ep.height, ep.size_bytes,
                ));
                if !oasis_backend_psp::network::is_net_initialized() {
                    dbg_log("[TV] calling ensure_net_init_pub...");
                    if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
                        dbg_log(&format!("[TV] net init failed: {e}"));
                        tv.error_msg = format!("Net: {e}");
                        backend.reinit_gu_frame();
                        return;
                    }
                    dbg_log("[TV] net init OK");
                    backend.reinit_gu_frame();
                }
                let url = oasis_core::apps::tv_guide::ChannelCatalog::download_url(ep);
                dbg_log(&format!("[TV] starting download: {url}"));
                // Note: DOWNLOAD_CANCEL is reset at the START of
                // handle_video_download (not here) to avoid
                // clearing a cancel that the old download hasn't seen.
                tv.now_playing = ep.title.clone();
                tv.downloading = true;
                tv.download_progress = 0.0;
                tv.error_msg.clear();
                tv.tuned = Some(tv.selected);
                io.send(IoCmd::VideoDownload {
                    url,
                    dest: String::from("ms0:/PSP/GAME/OASISOS/tv_cache.mp4"),
                    tag: 0xBB00,
                });
            } else {
                dbg_log("[TV] no suitable video found");
                tv.error_msg = String::from("No suitable video found");
            }
        } else {
            dbg_log("[TV] catalog still loading");
            tv.error_msg = String::from("Loading channel catalog...");
        }
    } else {
        dbg_log("[TV] tv_selected out of range");
    }
}
