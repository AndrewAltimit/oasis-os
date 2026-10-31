//! Boot helpers and the per-frame stages of `psp_main`'s loop, split out
//! of the entry point so it reads as a sequence of named steps.

use oasis_backend_psp::{
    AudioHandle, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH, StatusBarInfo, SystemInfo, TextureId,
};
use oasis_core::active_theme::ActiveTheme;
use oasis_core::bottombar::BottomBar;
use oasis_core::platform::{BatteryState, CpuClock, PowerInfo, SystemTime};
use oasis_core::statusbar::StatusBar;
use oasis_core::vector_overlay::ShaderLayerInfo;
use oasis_shader::software::SoftwareShaderRenderer;

use crate::app_states::*;
use crate::skins::PspSkinPreset;
use crate::types::*;
use crate::{dbg_log, now_us};

/// Wire the browser engine's progress / yield hooks to `eboot.log` and
/// the scheduler, and pre-warm the UA stylesheet.
pub(crate) fn install_browser_hooks() {
    // Wire HTML tokenizer progress hook → on-disk eboot.log so a
    // synchronous `navigate_vfs` that hangs in tokenize is observable
    // from the remote test harness.
    oasis_browser::internals::set_tokenize_progress_hook(|iter, pos, len, tokens, state| {
        let (free_kb, max_blk_kb) = oasis_backend_psp::heap_kb();
        oasis_backend_psp::vlog_force(&format!(
            "[BR/TOK] iter={iter} pos={pos}/{len} tokens={tokens} state={state} \
             free={free_kb}KB blk={max_blk_kb}KB"
        ));
    });
    // Cooperative yield: lets the cmd_server / audio / video threads
    // run between batches of tokenizer state-machine iterations so a
    // long synchronous `navigate_vfs` doesn't starve them.
    // `sleep_ms(1)` is the smallest unit the PSP scheduler exposes.
    oasis_browser::internals::set_tokenize_yield_hook(|| {
        psp::thread::sleep_ms(1);
    });
    oasis_browser::internals::set_tree_builder_progress_hook(|idx, total, nodes| {
        let (free_kb, max_blk_kb) = oasis_backend_psp::heap_kb();
        oasis_backend_psp::vlog_force(&format!(
            "[BR/TREE] idx={idx}/{total} nodes={nodes} free={free_kb}KB blk={max_blk_kb}KB"
        ));
    });
    oasis_browser::internals::set_tree_builder_yield_hook(|| {
        psp::thread::sleep_ms(1);
    });
    oasis_browser::internals::set_tree_builder_raw_log_hook(|msg| {
        oasis_backend_psp::vlog_force(msg);
    });
    oasis_browser::internals::set_cascade_progress_hook(|idx, total| {
        let (free_kb, _) = oasis_backend_psp::heap_kb();
        oasis_backend_psp::vlog_force(&format!("[BR/CASCADE] idx={idx}/{total} free={free_kb}KB"));
    });
    oasis_browser::internals::set_cascade_yield_hook(|| {
        psp::thread::sleep_ms(1);
    });

    // Pre-warm the lazily-initialised UA stylesheet so the first
    // browser navigation doesn't have to parse ~7 KB of CSS through
    // LazyLock's first-init lock while the cascade is also trying
    // to allocate. Verified on real PSP that this turns a hard hang
    // into a clean cascade run.
    let _ = oasis_browser::internals::default_stylesheet();
    dbg_log("[EBOOT] UA stylesheet pre-warmed");
}

/// The terminal's boot banner, plus the previous session's history when
/// it was saved.
pub(crate) fn initial_terminal(backend: &PspBackend, sysinfo: &SystemInfo) -> TerminalState {
    let vol_info = backend.volatile_mem_info();
    let mode_label = if cfg!(feature = "kernel-mode") {
        "kernel"
    } else {
        "user"
    };
    let initial_lines = vec![
        format!(
            "OASIS_OS v{} [PSP] ({mode_label} mode)",
            env!("CARGO_PKG_VERSION")
        ),
        format!(
            "CPU: {}MHz  Bus: {}MHz  ME: {}MHz",
            sysinfo.cpu_mhz, sysinfo.bus_mhz, sysinfo.me_mhz,
        ),
        if let Some((total, _)) = vol_info {
            format!("Texture cache: {} KB volatile RAM claimed", total / 1024)
        } else {
            String::from("Texture cache: main heap only (PSP-1000)")
        },
        String::from("Type 'help' for commands. []=OSK, Up/Down=scroll."),
        String::new(),
    ];
    let mut term = TerminalState::new(initial_lines);

    // Try to restore previous terminal history from save data (silent).
    if let Ok(saved) = crate::commands::load_terminal_history()
        && !saved.is_empty()
    {
        term.lines.push(String::from("(restored previous session)"));
        term.lines.extend(saved);
        term.lines.push(String::new());
    }
    term
}

// ---------------------------------------------------------------------------
// Wallpaper
// ---------------------------------------------------------------------------

/// Shader output size: rendered at 32x32 (internally ~11x11 via
/// RENDER_SCALE=3) to keep expensive shaders (voronoi: 277ms at 64x64)
/// within budget on 333MHz MIPS; GE bilinear scaling to 480x272 hides the
/// lower resolution.
const SHADER_W: u32 = 32;
const SHADER_H: u32 = 32;

/// The desktop background: the preset's static gradient, or its animated
/// software shader.
pub(crate) struct Wallpaper {
    gradient_tex: TextureId,
    shader_tex: TextureId,
    renderer: SoftwareShaderRenderer,
    /// The shader layer of the current theme (cached so the frame loop
    /// doesn't walk `background_layers`).
    shader: Option<ShaderLayerInfo>,
    /// Preset the textures were generated for (to detect skin changes).
    skin_key: &'static str,
}

impl Wallpaper {
    /// Generate the gradient texture and the shader texture for `preset`.
    pub(crate) fn new(
        backend: &mut PspBackend,
        preset: PspSkinPreset,
        theme: &ActiveTheme,
    ) -> Self {
        // Load wallpaper texture at reduced resolution (64x64 = 16KB vs 1MB).
        // The GE scales it up to 480x272 with bilinear filtering during blit.
        use oasis_backend_psp::{WALLPAPER_TEX_H, WALLPAPER_TEX_W};
        let wallpaper_data = oasis_backend_psp::generate_gradient_with(
            WALLPAPER_TEX_W,
            WALLPAPER_TEX_H,
            &preset.gradient_stops(),
        );
        let gradient_tex = backend
            .load_texture_inner(WALLPAPER_TEX_W, WALLPAPER_TEX_H, &wallpaper_data)
            .unwrap_or(TextureId(0));
        let renderer = SoftwareShaderRenderer::new(SHADER_W, SHADER_H);
        // Pre-allocate a 32x32 shader texture for blitting.
        let shader_init = vec![0u8; (SHADER_W * SHADER_H * 4) as usize];
        let shader_tex = backend
            .load_texture_inner(SHADER_W, SHADER_H, &shader_init)
            .unwrap_or(TextureId(0));
        let shader = oasis_core::vector_overlay::get_shader_layer(theme);
        if let Some(ref info) = shader {
            dbg_log(&format!(
                "[SHADER] active: {} ({}x{})",
                info.name, SHADER_W, SHADER_H
            ));
        } else {
            dbg_log("[SHADER] none (using static gradient)");
        }
        Self {
            gradient_tex,
            shader_tex,
            renderer,
            shader,
            skin_key: preset.key(),
        }
    }

    /// Follow a skin change, render the shader (every other frame: 30fps
    /// shader, 60fps UI) and blit the wallpaper fullscreen.
    pub(crate) fn draw(
        &mut self,
        backend: &mut PspBackend,
        preset: PspSkinPreset,
        theme: &ActiveTheme,
        viz_frame: u32,
    ) {
        // Detect skin change: update cached shader info AND regenerate
        // the static gradient wallpaper so non-shader presets visibly
        // change their background, not just the bars/icons.
        let cur_key = preset.key();
        if cur_key != self.skin_key {
            use oasis_backend_psp::{WALLPAPER_TEX_H, WALLPAPER_TEX_W};
            self.skin_key = cur_key;
            self.shader = oasis_core::vector_overlay::get_shader_layer(theme);
            let stops = preset.gradient_stops();
            let pixels =
                oasis_backend_psp::generate_gradient_with(WALLPAPER_TEX_W, WALLPAPER_TEX_H, &stops);
            backend.update_texture_data(self.gradient_tex, &pixels);
        }

        if let Some(ref info) = self.shader {
            if viz_frame.is_multiple_of(2) {
                let log_this_frame = viz_frame.is_multiple_of(600);
                let t0 = if log_this_frame { now_us() } else { 0 };
                let time = viz_frame as f32 / 60.0;
                let pixels = self.renderer.render_shader(&info.name, time, &info.params);
                backend.update_texture_data(self.shader_tex, pixels);
                // Log shader render time periodically (~every 10s).
                if log_this_frame {
                    let elapsed = now_us().wrapping_sub(t0);
                    dbg_log(&format!(
                        "[SHADER] render+upload: {}us (frame {})",
                        elapsed, viz_frame,
                    ));
                }
            }
            // Shader: 32x32 texture scaled to fullscreen by GE (bilinear).
            backend.blit_scaled(self.shader_tex, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);
        } else {
            // Static gradient: 64x64 texture scaled to fullscreen.
            backend.blit_scaled(self.gradient_tex, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);
        }
    }
}

// ---------------------------------------------------------------------------
// Per-frame polling
// ---------------------------------------------------------------------------

/// Mirror the audio thread's radio streaming state into `radio`.
pub(crate) fn poll_radio_status(audio: &AudioHandle, radio: &mut RadioState) {
    if radio.status == RadioStatus::Buffering || radio.status == RadioStatus::Playing {
        if !audio.is_radio_streaming() {
            radio.status = RadioStatus::Stopped;
            radio.now_playing.clear();
        } else if audio.is_radio_buffering() {
            radio.status = RadioStatus::Buffering;
        } else {
            radio.status = RadioStatus::Playing;
        }
        if let Some(meta) = audio.poll_radio_meta() {
            radio.now_playing = meta;
        }
    }
}

/// Upload the newest decoded video frame, notice the end of playback, and
/// run the decode-hang watchdog.
pub(crate) fn poll_video(
    backend: &mut PspBackend,
    tv: &mut TvGuideState,
    kiosk_app: KioskApp,
    viz_frame: u32,
) {
    // Poll regardless of download state — video decode starts as soon
    // as the first keyframe arrives, even during HTTP streaming.
    // Log state every 5 seconds.
    if viz_frame % 300 == 1 {
        let vp = oasis_backend_psp::video::is_video_playing();
        oasis_backend_psp::video::vlog_force(&format!(
            "[MAIN] tuned={} vp={} tex={:?} kiosk={:?} frame={}",
            tv.tuned.is_some(),
            vp,
            tv.preview_tex,
            kiosk_app as u8,
            viz_frame,
        ));
    }
    if tv.tuned.is_some() {
        // Drain all queued frames to keep the queue clear; upload the
        // newest (direct CSC is fast enough, ~15ms).
        let mut latest = oasis_backend_psp::video::poll_video_frame();
        while let Some(newer) = oasis_backend_psp::video::poll_video_frame() {
            latest = Some(newer);
        }
        if let Some(frame) = latest {
            let t0 = now_us();
            let pixels = oasis_backend_psp::video::frame_pixels_raw(&frame);
            let old_tex = tv.preview_tex;
            // Pass stride as width so texture buf_w matches CSC
            // stride (both 512px). This eliminates row-by-row copy.
            tv.preview_tex = backend.update_video_texture(frame.stride, frame.height, pixels);
            // Set actual content dimensions for correct UV mapping.
            backend.set_video_content_size(frame.width, frame.height);
            let dt = now_us().wrapping_sub(t0);
            oasis_backend_psp::video::record_upload_time(dt);
            if old_tex.is_none() && tv.preview_tex.is_some() {
                oasis_backend_psp::video::vlog_force(&format!(
                    "[MAIN] first video tex: {}x{} → {:?}",
                    frame.width, frame.height, tv.preview_tex,
                ));
            }
        }
        // Only clear tuned state when video WAS playing and stopped
        // (not during the startup gap before streaming begins).
        if tv.preview_tex.is_some() && !oasis_backend_psp::video::is_video_playing() {
            backend.free_video_texture();
            tv.preview_tex = None;
            tv.tuned = None;
            tv.now_playing.clear();
        }
    }

    // -- Decode hang watchdog (every ~0.5s) --
    // If DECODE_STEP == 2 (stuck inside sceMpegAvcDecode), signal
    // the internal semaphore to unblock the ME and force an error
    // return, allowing the video thread to fall back to audio-only.
    if tv.tuned.is_some() && viz_frame.is_multiple_of(30) {
        let step = psp::mpeg::DECODE_STEP.load(core::sync::atomic::Ordering::Relaxed);
        if step == 2 {
            oasis_backend_psp::video::vlog_force("[WATCHDOG] ME stuck in AvcDecode — unblocking");
            oasis_backend_psp::video::unblock_stuck_decode();
        }
    }
}

/// Carry out a `browse <url>` from the TCP command server.
///
/// Drives a synchronous `BrowserWidget::navigate_vfs`, switching to the
/// Browser app if necessary. PSP currently uses `navigate_vfs` (not
/// `navigate_to`) because the browser's per-frame `tick()` isn't wired up
/// on PSP — historically because of a misdiagnosed "std::time::Instant
/// crashes on Allegrex" claim (the real cause was the orphaned rust-psp
/// std time overlay, fixed in branch
/// fix/psp-hardware-std-overlay-alignment-and-time). `navigate_vfs` does
/// the fetch + parse + image load synchronously inside this single call.
/// Switching PSP to the async fetch path is a follow-up once we wire
/// `tick()` back in.
pub(crate) fn poll_remote_browse(
    backend: &mut PspBackend,
    kiosk_app: &mut KioskApp,
    br: &mut BrowserState,
) {
    let Some(url) = oasis_backend_psp::cmd_server::take_pending_browse() else {
        return;
    };
    *kiosk_app = KioskApp::Browser;
    // The cmd_server's auto_connect_wifi sets `NET_STACK_INITIALIZED`
    // but not the full `NET_INITIALIZED` flag the browser's TLS
    // provider checks. Call `ensure_net_init_pub` here to take
    // the GotIp fast path and flip the flag without showing the
    // WiFi dialog.
    if let Err(e) = oasis_backend_psp::network::ensure_net_init_pub() {
        dbg_log(&format!("[CMD] browse net init failed: {:?}", e));
    }
    // No-op on the fast path; re-opens the frame if the WiFi dialog
    // had to run after all (and clears its flag either way).
    backend.reinit_gu_frame();
    let _ = br.ensure_widget();
    oasis_backend_psp::vlog_force(&format!("[BR/MAIN] navigate_vfs start: {}", url));
    let BrowserState { widget, vfs, .. } = br;
    if let Some(w) = widget.as_mut() {
        w.navigate_vfs(&url, vfs);
    }
    oasis_backend_psp::vlog_force(&format!("[BR/MAIN] navigate_vfs end: {}", url));
    br.loading = false;
    br.status_msg = format!("Loaded {}", url);
    dbg_log(&format!("[CMD] browse -> {}", url));
}

/// Feed the polled PSP status (clock, battery, CPU) into the shared
/// status and bottom bars.
pub(crate) fn feed_status_bars(
    st: &StatusBarInfo,
    sysinfo: &SystemInfo,
    status_bar: &mut StatusBar,
    bottom_bar: &mut BottomBar,
) {
    let sys_time = SystemTime {
        year: st.year,
        month: st.month as u8,
        day: st.day as u8,
        hour: st.hour as u8,
        minute: st.minute as u8,
        second: 0,
    };
    let bat_state = if st.ac_power && !st.battery_charging {
        BatteryState::Full
    } else if st.battery_charging {
        BatteryState::Charging
    } else if st.battery_percent < 0 {
        BatteryState::NoBattery
    } else {
        BatteryState::Discharging
    };
    let power = PowerInfo {
        battery_percent: if st.battery_percent >= 0 {
            Some(st.battery_percent as u8)
        } else {
            None
        },
        battery_minutes: None,
        state: bat_state,
        cpu: CpuClock {
            current_mhz: sysinfo.cpu_mhz as u32,
            max_mhz: 333,
        },
    };
    status_bar.update_info(Some(&sys_time), Some(&power));
    bottom_bar.update_info(Some(&sys_time));
}
