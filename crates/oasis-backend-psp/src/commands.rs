//! Terminal command interpreter and utility commands.
//!
//! Extracted from main.rs to reduce monolithic file size. Contains the
//! `execute_command` dispatcher, save/load helpers, benchmarks, and
//! Media Engine test.

use oasis_backend_psp::{SCREEN_HEIGHT, SCREEN_WIDTH, StatusBarInfo};

use crate::CONFIG_PATH;

// ---------------------------------------------------------------------------
// Command interpreter
// ---------------------------------------------------------------------------

/// Execute a terminal command and return output lines.
///
/// Commands that need access to main-loop state (save, load, usb, sfx)
/// return placeholder text and are handled by the caller.
pub fn execute_command(cmd: &str, config: &mut psp::config::Config) -> Vec<String> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return vec![];
    }
    match trimmed {
        "help" => vec![
            String::from("Available commands:"),
            String::from("  help       - Show this message"),
            String::from("  status     - System status"),
            String::from("  ls [path]  - List directory"),
            String::from("  clock      - Show/set CPU frequency"),
            String::from("  clock 333  - Set max (333/333/166)"),
            String::from("  clock 266  - Set balanced (266/266/133)"),
            String::from("  clock 222  - Set power save (222/222/111)"),
            String::from("  usb mount  - Enable USB storage mode"),
            String::from("  usb unmount- Disable USB storage mode"),
            String::from("  usb status - Show USB state"),
            String::from("  benchmark  - Run performance benchmarks"),
            String::from("  sysinfo    - PSP system parameters"),
            String::from("  me test    - Test Media Engine core"),
            String::from("  cat PATH   - Display file contents"),
            String::from("  mkdir PATH - Create directory"),
            String::from("  rm PATH    - Delete file (confirm)"),
            String::from("  date       - Current date/time (RFC3339)"),
            String::from("  mem        - Memory usage"),
            String::from("  config K=V - Set/get persistent config"),
            String::from("  play PATH  - Play MP3 file"),
            String::from("  pause/resume/stop - Audio control"),
            String::from("  umd        - UMD disc info"),
            String::from("  save/load  - Terminal history"),
            String::from("  plugin install - Install overlay PRX"),
            String::from("  plugin remove  - Remove overlay PRX"),
            String::from("  plugin status  - Plugin load status"),
            String::from("  clear      - Clear terminal"),
            String::new(),
            String::from("[Square] Open keyboard  [X] Execute"),
            String::from("[Up] help  [Down] status"),
        ],
        "status" => {
            let status = StatusBarInfo::poll();
            let bat = if status.battery_percent >= 0 {
                format!(
                    "Battery: {}%{}",
                    status.battery_percent,
                    if status.battery_charging {
                        " (charging)"
                    } else {
                        ""
                    }
                )
            } else {
                String::from("Battery: N/A")
            };
            vec![
                String::from("OASIS_OS v0.1.0 [PSP] (kernel mode)"),
                String::from("Platform: mipsel-sony-psp"),
                String::from("Display: 480x272 RGBA8888"),
                String::from("Backend: sceGu hardware"),
                format!(
                    "CPU: {}MHz  Bus: {}MHz",
                    unsafe { psp::sys::scePowerGetCpuClockFrequency() },
                    unsafe { psp::sys::scePowerGetBusClockFrequency() }
                ),
                bat,
                format!(
                    "WiFi: {}  USB: {}",
                    if status.wifi_on { "ON" } else { "OFF" },
                    if status.usb_connected {
                        "connected"
                    } else {
                        "---"
                    }
                ),
                format!(
                    "Time: {} {:02}:{:02}",
                    status.day_of_week, status.hour, status.minute
                ),
            ]
        },
        "clock" => {
            let clk = psp::power::get_clock();
            vec![format!(
                "Current: CPU {}MHz, Bus {}MHz",
                clk.cpu_mhz, clk.bus_mhz,
            )]
        },
        "clock 333" => set_clock_cmd(config, 333, 166, "max performance"),
        "clock 266" => set_clock_cmd(config, 266, 133, "balanced"),
        "clock 222" => set_clock_cmd(config, 222, 111, "power save"),
        "benchmark" | "bench" => run_benchmark(),
        "sysinfo" => cmd_sysinfo(),
        "me test" => {
            #[cfg(feature = "kernel-me")]
            { cmd_me_test() }
            #[cfg(not(feature = "kernel-me"))]
            { vec!["ME test requires kernel mode.".into()] }
        }
        "screenshot" | "ss" => {
            let bmp = psp::screenshot_bmp();
            let path = "ms0:/PSP/PHOTO/screenshot.bmp";
            match psp::io::write_bytes(path, &bmp) {
                Ok(()) => vec![format!("Screenshot saved: {}", path)],
                Err(e) => vec![format!("Screenshot failed: {:?}", e)],
            }
        },
        _ if trimmed.starts_with("ls") => {
            let path = trimmed.strip_prefix("ls").unwrap().trim();
            let dir = if path.is_empty() { "ms0:/" } else { path };
            let entries = oasis_backend_psp::list_directory(dir);
            if entries.is_empty() {
                vec![format!("(empty or cannot open: {})", dir)]
            } else {
                let mut out = vec![format!("{}  ({} entries)", dir, entries.len())];
                for e in entries.iter().take(30) {
                    if e.is_dir {
                        out.push(format!("  [D] {}/", e.name));
                    } else {
                        out.push(format!(
                            "  [F] {}  {}",
                            e.name,
                            oasis_backend_psp::format_size(e.size)
                        ));
                    }
                }
                if entries.len() > 30 {
                    out.push(format!("  ... and {} more", entries.len() - 30));
                }
                out
            }
        },
        _ if trimmed.starts_with("cat ") => cmd_cat(trimmed),
        _ if trimmed.starts_with("mkdir ") => cmd_mkdir(trimmed),
        _ if trimmed.starts_with("rm ") => cmd_rm(trimmed),
        "date" => cmd_date(),
        "mem" => cmd_mem(),
        _ if trimmed.starts_with("config ") => cmd_config(trimmed, config),
        "umd" | "umdinfo" => cmd_umd(),
        "plugin install" => cmd_plugin_install(),
        "plugin remove" => cmd_plugin_remove(),
        "plugin status" => cmd_plugin_status(),
        "plugin" => vec![
            String::from("Usage:"),
            String::from("  plugin install - Install overlay PRX"),
            String::from("  plugin remove  - Remove overlay PRX"),
            String::from("  plugin status  - Show load status"),
        ],
        "version" => vec![String::from("OASIS_OS v0.1.0")],
        "about" => vec![
            String::from("OASIS_OS -- Embeddable OS Framework"),
            String::from("PSP backend with GU rendering (kernel mode)"),
            String::from("Floating windows + multiprocessing enabled"),
        ],
        "save" | "load" | "play" | "pause" | "resume" | "stop" => {
            vec![String::from("(handled in main loop)")]
        },
        "clear" => vec![],
        _ => vec![format!("Unknown command: {}", trimmed)],
    }
}

fn set_clock_cmd(config: &mut psp::config::Config, cpu: i32, bus: i32, label: &str) -> Vec<String> {
    let ret = oasis_backend_psp::set_clock(cpu, bus);
    if ret >= 0 {
        config.set("clock_mhz", psp::config::ConfigValue::I32(cpu));
        config.set("bus_mhz", psp::config::ConfigValue::I32(bus));
        let _ = config.save(CONFIG_PATH);
        vec![format!("Clock set: {}/{} ({})", cpu, bus, label)]
    } else {
        vec![format!("Failed to set clock: {}", ret)]
    }
}

fn cmd_sysinfo() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(nick) = psp::system_param::nickname() {
        let name = core::str::from_utf8(&nick)
            .unwrap_or("?")
            .trim_end_matches('\0');
        out.push(format!("Nickname: {}", name));
    }
    if let Ok(lang) = psp::system_param::language() {
        out.push(format!("Language: {:?}", lang));
    }
    if let Ok(tz) = psp::system_param::timezone_offset() {
        let sign = if tz >= 0 { '+' } else { '-' };
        out.push(format!(
            "Timezone: UTC{}{:02}:{:02}",
            sign,
            tz.abs() / 60,
            tz.abs() % 60
        ));
    }
    if let Ok(dst) = psp::system_param::daylight_saving() {
        out.push(format!("DST: {}", if dst { "enabled" } else { "disabled" }));
    }
    if let Ok(tf) = psp::system_param::time_format() {
        out.push(format!("Time format: {:?}", tf));
    }
    if let Ok(df) = psp::system_param::date_format() {
        out.push(format!("Date format: {:?}", df));
    }
    if let Ok(tick) = psp::rtc::Tick::now() {
        if let Ok(rfc) = psp::rtc::format_rfc3339_local(&tick) {
            let ts = core::str::from_utf8(&rfc)
                .unwrap_or("?")
                .trim_end_matches('\0');
            out.push(format!("Time: {}", ts));
        }
    }
    if out.is_empty() {
        vec!["sysinfo: failed to query system params".into()]
    } else {
        out
    }
}

fn cmd_cat(trimmed: &str) -> Vec<String> {
    let path = trimmed.strip_prefix("cat ").unwrap().trim();
    if path.is_empty() {
        return vec!["usage: cat <path>".into()];
    }
    match psp::io::read_to_vec(path) {
        Ok(data) => {
            let text = String::from_utf8_lossy(&data);
            let mut lines: Vec<String> = text
                .lines()
                .take(50)
                .map(|l| {
                    if l.len() > 56 {
                        format!("{}...", &l[..l.floor_char_boundary(56)])
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            if text.lines().count() > 50 {
                lines.push(format!("... ({} bytes total)", data.len()));
            }
            lines
        },
        Err(e) => vec![format!("cat: {:?}", e)],
    }
}

fn cmd_mkdir(trimmed: &str) -> Vec<String> {
    let path = trimmed.strip_prefix("mkdir ").unwrap().trim();
    if path.is_empty() {
        return vec!["usage: mkdir <path>".into()];
    }
    match psp::io::create_dir(path) {
        Ok(()) => vec![format!("Created: {}", path)],
        Err(e) => vec![format!("mkdir: {:?}", e)],
    }
}

fn cmd_rm(trimmed: &str) -> Vec<String> {
    let path = trimmed.strip_prefix("rm ").unwrap().trim();
    if path.is_empty() {
        return vec!["usage: rm <path>".into()];
    }
    match psp::dialog::confirm_dialog(&format!("Delete {}?", path)) {
        Ok(psp::dialog::DialogResult::Confirm) => match psp::io::remove_file(path) {
            Ok(()) => vec![format!("Deleted: {}", path)],
            Err(e) => vec![format!("rm: {:?}", e)],
        },
        _ => vec!["Cancelled.".into()],
    }
}

fn cmd_date() -> Vec<String> {
    if let Ok(tick) = psp::rtc::Tick::now() {
        if let Ok(rfc) = psp::rtc::format_rfc3339_local(&tick) {
            let ts = core::str::from_utf8(&rfc)
                .unwrap_or("?")
                .trim_end_matches('\0');
            return vec![ts.to_string()];
        }
    }
    vec!["date: failed to read RTC".into()]
}

fn cmd_mem() -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: sceKernelTotalFreeMemSize returns available heap bytes.
    let free = unsafe { psp::sys::sceKernelTotalFreeMemSize() };
    let max_block = unsafe { psp::sys::sceKernelMaxFreeMemSize() };
    out.push(format!("Free RAM: {} KB", free / 1024));
    out.push(format!("Largest block: {} KB", max_block / 1024));
    out
}

fn cmd_umd() -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: PSP UMD syscalls with no side effects.
    unsafe {
        let present = psp::sys::sceUmdCheckMedium() != 0;
        out.push(format!(
            "Disc: {}",
            if present { "inserted" } else { "not present" }
        ));

        let state = psp::sys::sceUmdGetDriveStat();
        let flags = psp::sys::UmdStateFlags::from_bits_truncate(state);
        let mut state_parts = Vec::new();
        if flags.contains(psp::sys::UmdStateFlags::READY) {
            state_parts.push("READY");
        }
        if flags.contains(psp::sys::UmdStateFlags::INITED) {
            state_parts.push("INITED");
        }
        if flags.contains(psp::sys::UmdStateFlags::INITING) {
            state_parts.push("INITING");
        }
        if flags.contains(psp::sys::UmdStateFlags::NOT_PRESENT) {
            state_parts.push("NOT_PRESENT");
        }
        if state_parts.is_empty() {
            out.push(format!("Drive state: 0x{:02x}", state));
        } else {
            out.push(format!("Drive state: {}", state_parts.join(" | ")));
        }

        if present {
            let mut info = psp::sys::UmdInfo {
                size: core::mem::size_of::<psp::sys::UmdInfo>() as u32,
                type_: psp::sys::UmdType::Game,
            };
            let ret = psp::sys::sceUmdGetDiscInfo(&mut info);
            if ret >= 0 {
                let type_name = match info.type_ {
                    psp::sys::UmdType::Game => "Game",
                    psp::sys::UmdType::Video => "Video",
                    psp::sys::UmdType::Audio => "Audio",
                };
                out.push(format!("Disc type: {}", type_name));
            }
        }
    }
    out
}

fn cmd_config(trimmed: &str, config: &mut psp::config::Config) -> Vec<String> {
    let args = trimmed.strip_prefix("config ").unwrap().trim();
    if args.is_empty() {
        return vec!["usage: config KEY or config KEY=VALUE".into()];
    }
    if let Some((key, val)) = args.split_once('=') {
        let key = key.trim();
        let val = val.trim();
        if let Ok(n) = val.parse::<i32>() {
            config.set(key, psp::config::ConfigValue::I32(n));
        } else {
            config.set(key, psp::config::ConfigValue::Str(val.to_string()));
        }
        let _ = config.save(CONFIG_PATH);
        vec![format!("{} = {}", key, val)]
    } else {
        let key = args.trim();
        if let Some(val) = config.get_i32(key) {
            vec![format!("{} = {}", key, val)]
        } else if let Some(val) = config.get_str(key) {
            vec![format!("{} = {}", key, val)]
        } else {
            vec![format!("{}: not set", key)]
        }
    }
}

#[cfg(feature = "kernel-me")]
fn cmd_me_test() -> Vec<String> {
    match me_compute_test() {
        Ok((result, us)) => vec![
            format!("ME compute result: {}", result),
            format!("Elapsed: {} us", us),
            String::from("Media Engine core is operational."),
        ],
        Err(e) => vec![format!("ME test failed: {}", e)],
    }
}

// ---------------------------------------------------------------------------
// Save data helpers
// ---------------------------------------------------------------------------

const SAVE_GAME_NAME: &[u8; 13] = b"OASIS000000\0\0";
const SAVE_SLOT_NAME: &[u8; 20] = b"STATE000\0\0\0\0\0\0\0\0\0\0\0\0";

fn serialize_history(lines: &[String]) -> Vec<u8> {
    lines.join("\n").into_bytes()
}

fn deserialize_history(data: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(data);
    text.lines().map(|l| l.to_string()).collect()
}

/// Save terminal history using PSP savedata system dialog.
pub fn save_terminal_history(lines: &[String]) -> Result<(), String> {
    let data = serialize_history(lines);
    psp::savedata::Savedata::new(SAVE_GAME_NAME)
        .title("OASIS OS State")
        .detail("Terminal history")
        .save(SAVE_SLOT_NAME, &data)
        .map_err(|e| format!("{e}"))
}

/// Load terminal history using PSP savedata.
pub fn load_terminal_history() -> Result<Vec<String>, String> {
    let data = psp::savedata::Savedata::new(SAVE_GAME_NAME)
        .load(SAVE_SLOT_NAME, 65536)
        .map_err(|e| format!("{e}"))?;
    Ok(deserialize_history(&data))
}

// ---------------------------------------------------------------------------
// Benchmarking
// ---------------------------------------------------------------------------

fn bench_avg<F: FnMut()>(name: &str, iters: u32, mut f: F) -> String {
    let start = psp::time::Instant::now();
    for _ in 0..iters {
        f();
    }
    let total_us = start.elapsed().as_micros();
    let avg = total_us as f64 / iters as f64;
    format!("{}: {:.1} us avg ({} iters)", name, avg, iters)
}

fn run_benchmark() -> Vec<String> {
    let mut out = vec![String::from("Running benchmarks...")];

    out.push(bench_avg("Wallpaper gen (480x272)", 5, || {
        let _ = oasis_backend_psp::generate_gradient(SCREEN_WIDTH, SCREEN_HEIGHT);
    }));

    out.push(bench_avg("Cursor gen (12x18)", 100, || {
        let _ = oasis_backend_psp::generate_cursor_pixels();
    }));

    out.push(bench_avg("StatusBarInfo::poll()", 100, || {
        let _ = StatusBarInfo::poll();
    }));

    out.push(bench_avg("RTC day_of_week()", 1000, || {
        let _ = psp::rtc::day_of_week(2026, 2, 9);
    }));

    out.push(bench_avg("alloc 4KB Vec", 100, || {
        let v: Vec<u8> = vec![0u8; 4096];
        std::hint::black_box(v);
    }));

    let src = vec![0xAAu8; 65536];
    let mut dst = vec![0u8; 65536];
    out.push(bench_avg("DMA 64KB copy", 50, || {
        // SAFETY: src and dst are valid, non-overlapping, 64KB each.
        unsafe {
            let _ = psp::dma::memcpy_dma(dst.as_mut_ptr(), src.as_ptr(), 65536);
        }
    }));

    out.push(bench_avg("CPU 64KB copy", 50, || {
        // SAFETY: src and dst are valid, non-overlapping.
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), 65536);
        }
    }));

    out.push(String::from("Done."));
    out
}

// ---------------------------------------------------------------------------
// Media Engine test
// ---------------------------------------------------------------------------

/// ME task: sum integers 1..=arg on the ME core (pure integer math).
///
/// SAFETY: Runs on ME core. No syscalls, no cached memory, no heap.
#[cfg(feature = "kernel-me")]
unsafe extern "C" fn me_sum_task(arg: i32) -> i32 {
    let mut sum: i32 = 0;
    let mut i: i32 = 1;
    while i <= arg {
        sum = sum.wrapping_add(i);
        i += 1;
    }
    sum
}

// ---------------------------------------------------------------------------
// Plugin management
// ---------------------------------------------------------------------------

/// PRX source path (bundled alongside EBOOT in the GAME directory).
const PLUGIN_SRC: &str = "oasis_plugin.prx";
/// PRX install destination on Memory Stick.
const PLUGIN_DST: &str = "ms0:/seplugins/oasis_plugin.prx";
/// PLUGINS.TXT path.
const PLUGINS_TXT: &str = "ms0:/seplugins/PLUGINS.TXT";
/// Line to add/remove in PLUGINS.TXT.
const PLUGIN_LINE: &str = "game, ms0:/seplugins/oasis_plugin.prx, on";
/// Default oasis.ini content.
const DEFAULT_INI: &str = "\
# OASIS OS Overlay Plugin Configuration\n\
# Trigger button: note or screen\n\
trigger = note\n\
# Music directory\n\
music_dir = ms0:/MUSIC/\n\
# Overlay opacity (0-255)\n\
opacity = 180\n\
# Auto-start music on game launch\n\
autoplay = false\n";

fn cmd_plugin_install() -> Vec<String> {
    let mut out = Vec::new();

    // Ensure seplugins directory exists
    let _ = psp::io::create_dir("ms0:/seplugins");

    // Copy PRX file
    match psp::io::read_to_vec(PLUGIN_SRC) {
        Ok(data) => match psp::io::write_bytes(PLUGIN_DST, &data) {
            Ok(()) => out.push(format!("Copied PRX to {}", PLUGIN_DST)),
            Err(e) => {
                out.push(format!("Failed to write PRX: {:?}", e));
                return out;
            }
        },
        Err(e) => {
            out.push(format!("PRX not found ({}): {:?}", PLUGIN_SRC, e));
            out.push(String::from("Place oasis_plugin.prx next to EBOOT.PBP"));
            return out;
        }
    }

    // Add to PLUGINS.TXT (if not already present)
    let existing = psp::io::read_to_vec(PLUGINS_TXT).unwrap_or_default();
    let text = String::from_utf8_lossy(&existing);
    if !text.contains("oasis_plugin.prx") {
        let mut new_text = text.to_string();
        if !new_text.is_empty() && !new_text.ends_with('\n') {
            new_text.push('\n');
        }
        new_text.push_str(PLUGIN_LINE);
        new_text.push('\n');
        match psp::io::write_bytes(PLUGINS_TXT, new_text.as_bytes()) {
            Ok(()) => out.push(String::from("Added to PLUGINS.TXT")),
            Err(e) => out.push(format!("Failed to update PLUGINS.TXT: {:?}", e)),
        }
    } else {
        out.push(String::from("Already in PLUGINS.TXT"));
    }

    // Write default config if it doesn't exist
    let ini_path = "ms0:/seplugins/oasis.ini";
    if psp::io::read_to_vec(ini_path).is_err() {
        let _ = psp::io::write_bytes(ini_path, DEFAULT_INI.as_bytes());
        out.push(String::from("Created oasis.ini with defaults"));
    }

    out.push(String::from("Plugin installed. Reboot to activate."));
    out
}

fn cmd_plugin_remove() -> Vec<String> {
    let mut out = Vec::new();

    // Remove PRX file
    match psp::io::remove_file(PLUGIN_DST) {
        Ok(()) => out.push(format!("Removed {}", PLUGIN_DST)),
        Err(e) => out.push(format!("PRX not found or remove failed: {:?}", e)),
    }

    // Remove from PLUGINS.TXT
    match psp::io::read_to_vec(PLUGINS_TXT) {
        Ok(data) => {
            let text = String::from_utf8_lossy(&data);
            let filtered: Vec<&str> = text
                .lines()
                .filter(|l| !l.contains("oasis_plugin.prx"))
                .collect();
            let new_text = filtered.join("\n") + "\n";
            match psp::io::write_bytes(PLUGINS_TXT, new_text.as_bytes()) {
                Ok(()) => out.push(String::from("Removed from PLUGINS.TXT")),
                Err(e) => out.push(format!("Failed to update PLUGINS.TXT: {:?}", e)),
            }
        }
        Err(_) => out.push(String::from("PLUGINS.TXT not found")),
    }

    out.push(String::from("Plugin removed. Reboot to deactivate."));
    out
}

fn cmd_plugin_status() -> Vec<String> {
    let mut out = Vec::new();

    // Check if PRX exists on Memory Stick
    let prx_exists = psp::io::read_to_vec(PLUGIN_DST).is_ok();
    out.push(format!(
        "PRX file: {}",
        if prx_exists { "installed" } else { "not found" }
    ));

    // Check PLUGINS.TXT
    match psp::io::read_to_vec(PLUGINS_TXT) {
        Ok(data) => {
            let text = String::from_utf8_lossy(&data);
            let entry = text.lines().find(|l| l.contains("oasis_plugin.prx"));
            match entry {
                Some(line) => {
                    let enabled = line.contains(", on");
                    out.push(format!(
                        "PLUGINS.TXT: {}",
                        if enabled { "enabled" } else { "disabled" }
                    ));
                }
                None => out.push(String::from("PLUGINS.TXT: not registered")),
            }
        }
        Err(_) => out.push(String::from("PLUGINS.TXT: not found")),
    }

    // Check if config exists
    let ini_exists = psp::io::read_to_vec("ms0:/seplugins/oasis.ini").is_ok();
    out.push(format!(
        "Config: {}",
        if ini_exists { "oasis.ini found" } else { "no config" }
    ));

    out
}

#[cfg(feature = "kernel-me")]
fn me_compute_test() -> Result<(i32, u64), String> {
    let mut executor = psp::me::MeExecutor::new(4096).map_err(|e| format!("ME init: {e}"))?;

    let start = psp::time::Instant::now();

    // SAFETY: me_sum_task is a pure integer function with no syscalls
    // or cached memory access.
    let handle = unsafe { executor.submit(me_sum_task, 100_000) };
    let result = executor.wait(&handle);

    let elapsed = start.elapsed().as_micros();
    executor.reset();

    Ok((result, elapsed))
}
