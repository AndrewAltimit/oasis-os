# OASIS OS PSP Backend Modernization Plan

## Executive Summary

The `rust-psp` SDK has undergone a massive modernization, adding 30+ high-level abstraction
modules on top of the raw `psp::sys::*` syscall bindings. The current OASIS OS PSP backend
(`crates/oasis-backend-psp`) was written against the raw syscalls exclusively. This plan
rewrites the backend to use the new high-level APIs, unlocking capabilities that were
previously impossible or impractical: multi-threading, networking, system fonts, hardware
SIMD math, DMA transfers, on-screen keyboard input, persistent configuration, save data,
system dialogs, HTTP connectivity, USB storage mode, Media Engine offloading, and more.

**Scope:** Every file in `crates/oasis-backend-psp/` (lib.rs, main.rs, font.rs) will be
rewritten. Several new modules and features will be added. The `oasis-core` backend traits
will gain new implementations (NetworkBackend, AudioBackend) that were previously stubbed.

---

## Part 1: Replace Raw Syscalls with High-Level APIs

These changes replace existing manual code with cleaner, safer abstractions from rust-psp.
No new features -- same behavior, better code.

### 1.1 Input: `psp::sys::sceCtrl*` -> `psp::input::Controller`

**Current (lib.rs:734-847):** Manual `sceCtrlPeekBufferPositive` call, manual edge detection
via `prev_buttons` bitfield, manual analog deadzone/scaling, 80+ lines of `check_button`/
`check_trigger` helper methods.

**New:** Replace with `psp::input::Controller`:
```rust
let mut ctrl = psp::input::Controller::new();
// In poll_events_inner:
ctrl.update();
if ctrl.is_pressed(CtrlButtons::CROSS) { events.push(InputEvent::ButtonPress(Button::Confirm)); }
if ctrl.is_released(CtrlButtons::CROSS) { events.push(InputEvent::ButtonRelease(Button::Confirm)); }
let dx = ctrl.analog_x_f32(0.15);  // normalized [-1,1] with deadzone
let dy = ctrl.analog_y_f32(0.15);
```

**Impact:**
- Delete `prev_buttons` field from `PspBackend`
- Delete `check_button()` and `check_trigger()` helper methods (~35 lines)
- Delete manual analog deadzone/scaling code (~15 lines)
- `poll_events_inner()` shrinks from ~115 lines to ~50 lines
- Gain: proper analog normalization (float [-1,1]) vs current integer division

### 1.2 File I/O: `psp::sys::sceIo*` -> `psp::io::*`

**Current (lib.rs:1598-1716):** Manual null-terminated path construction, raw `sceIoDopen/
sceIoDread/sceIoDclose` loops, raw `sceIoGetstat/sceIoOpen/sceIoRead/sceIoClose` with
manual chunked reading, no RAII.

**New:** Replace with `psp::io::*`:
```rust
// list_directory -> psp::io::read_dir()
fn list_directory(path: &str) -> Vec<FileEntry> {
    let Ok(entries) = psp::io::read_dir(path) else { return vec![] };
    let mut result: Vec<FileEntry> = entries
        .filter_map(|e| e.ok())
        .filter(|e| { let n = e.name(); n != b".\0" && n != b"..\0" })
        .map(|e| FileEntry { name: parse_name(e.name()), size: e.stat().st_size, is_dir: e.is_dir() })
        .collect();
    result.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    result
}

// read_file -> psp::io::read_to_vec()
fn read_file(path: &str) -> Option<Vec<u8>> {
    psp::io::read_to_vec(path).ok()
}
```

**Impact:**
- Delete ~120 lines of manual I/O code (`list_directory`, `read_file`)
- Replace with ~30 lines using `psp::io::read_dir()`, `read_to_vec()`
- RAII file handles (auto-close on drop) -- eliminates leak risk
- Proper error propagation via `Result` types

### 1.3 Audio: Raw `sceMp3*`/`sceAudio*` -> `psp::mp3::Mp3Decoder` + `psp::audio::AudioChannel`

**Current (lib.rs:1789-2093):** 300+ lines of manual `AudioPlayer` struct with raw buffer
allocation (`alloc`/`dealloc` with manual `Layout`), raw `sceMp3ReserveMp3Handle`,
`sceMp3Init`, `sceMp3Decode`, `sceAudioChReserve`, `sceAudioOutputBlocking`, manual
stream feeding via `sceMp3GetInfoToAddStreamData/sceMp3NotifyAddStreamData`.

**New:** Replace with `psp::mp3::Mp3Decoder` + `psp::audio::AudioChannel`:
```rust
pub struct AudioPlayer {
    decoder: Option<psp::mp3::Mp3Decoder>,
    channel: Option<psp::audio::AudioChannel>,
    file_data: Vec<u8>,
    playing: bool,
    paused: bool,
}

impl AudioPlayer {
    pub fn load_and_play(&mut self, path: &str) -> bool {
        let data = psp::io::read_to_vec(path).ok()?;
        let decoder = psp::mp3::Mp3Decoder::new(&data).ok()?;
        let channel = psp::audio::AudioChannel::reserve(1024, AudioFormat::Stereo).ok()?;
        self.decoder = Some(decoder);
        self.channel = Some(channel);
        self.file_data = data;
        self.playing = true;
        true
    }

    pub fn update(&mut self) {
        if let (Some(dec), Some(ch)) = (&mut self.decoder, &self.channel) {
            match dec.decode_frame() {
                Ok(samples) => { ch.output_blocking(0x8000, samples).ok(); }
                Err(_) => { self.playing = false; }
            }
        }
    }
}
```

**Impact:**
- Delete ~300 lines of manual audio code
- Replace with ~80 lines using RAII types
- Delete manual `alloc`/`dealloc` for MP3/PCM buffers (Mp3Decoder manages internally)
- RAII: AudioChannel auto-releases on drop, Mp3Decoder auto-cleans up
- Gain: `decoder.set_loop(-1)` for infinite looping (currently impossible)
- Gain: `decoder.sample_rate()`, `decoder.channels()`, `decoder.bitrate()` (cleaner API)

### 1.4 JPEG Decoding: Raw `sceJpeg*` -> `psp::image::decode_jpeg()`

**Current (lib.rs:1728-1783):** 55 lines of manual `sceJpegInitMJpeg/sceJpegCreateMJpeg/
sceJpegDecodeMJpeg` with manual 64-byte-aligned buffer allocation/deallocation and manual
cleanup on error paths.

**New:**
```rust
fn decode_jpeg(data: &[u8], max_w: i32, max_h: i32) -> Option<(u32, u32, Vec<u8>)> {
    let img = psp::image::decode_jpeg(data, max_w, max_h).ok()?;
    Some((img.width, img.height, img.data))
}
```

**Impact:**
- Delete 55 lines, replace with 3 lines
- Automatic buffer management, cleanup, error handling
- Gain: BMP decoding support for free via `psp::image::decode()` auto-detection
- Photo viewer can now display BMP files in addition to JPEG

### 1.5 Power Management: Raw `scePower*` -> `psp::power::*`

**Current (lib.rs:1415-1578, 2096-2148):** Raw `scePowerGetCpuClockFrequency`,
`scePowerSetClockFrequency`, `scePowerIsBatteryExist`, etc. Manual callback registration
via `sceKernelCreateCallback`/`scePowerRegisterCallback`.

**New:**
```rust
let clock = psp::power::get_clock();           // ClockFrequency { cpu_mhz, bus_mhz }
psp::power::set_clock(333, 166).unwrap();       // Returns new ClockFrequency
let bat = psp::power::battery_info();           // BatteryInfo with all fields
psp::power::prevent_sleep();                    // Replaces scePowerTick
let _cb = psp::power::on_power_event(handler);  // RAII callback handle
```

**Impact:**
- Delete `SystemInfo::query()` (20 lines) -- replaced by `psp::power::get_clock()`
- Delete `StatusBarInfo::poll()` battery section -- replaced by `psp::power::battery_info()`
- Delete `register_power_callback()` + `power_callback()` (~45 lines) -- replaced by RAII handle
- Delete `power_tick()` -- replaced by `psp::power::prevent_sleep()`
- Delete `set_clock()` -- replaced by `psp::power::set_clock()`

### 1.6 Exception Handler: Raw -> `psp::callback::setup_exit_callback()`

**Current (lib.rs:1529-1561, main.rs:19):** Manual `sceKernelRegisterDefaultExceptionHandler`
with raw `extern "C"` callback. Uses `psp::module_kernel!` + `psp::enable_home_button()`.

**New:**
```rust
psp::callback::setup_exit_callback().unwrap();  // Replaces enable_home_button
// Exception handler registration stays (kernel-specific, no high-level wrapper)
```

**Impact:**
- Replace `psp::enable_home_button()` with `psp::callback::setup_exit_callback()`
- Cleaner exit handling with proper background thread

### 1.7 Display Timing: Manual -> `psp::time::FrameTimer`

**Current:** No frame timing -- just vsync wait. No FPS tracking.

**New:**
```rust
let mut frame_timer = psp::time::FrameTimer::new();
// In main loop:
let dt = frame_timer.tick();  // delta time in seconds
let fps = frame_timer.fps();  // for status display
```

**Impact:**
- Add `FrameTimer` to main loop
- Can display FPS in status bar or terminal
- Delta time enables smooth animations (currently frame-locked)

### 1.8 Cache Management: Manual Uncached Mirrors -> `psp::cache::*`

**Current (lib.rs:453, 541):** Manual `(ptr as usize | 0x4000_0000) as *const c_void` for
uncached memory access when binding textures and font atlas.

**New:**
```rust
use psp::cache::{CachedPtr, UncachedPtr, dcache_writeback_range};

let cached = unsafe { CachedPtr::new(texture.data) };
let uncached = unsafe { cached.flush_to_uncached(texture_size) };
// Use uncached.as_ptr() for GU texture binding
```

**Impact:**
- Replace ad-hoc `| 0x4000_0000` with type-safe wrappers
- Compile-time distinction between cached/uncached pointers
- Proper dcache flush before GU access (currently relies on uncached mirror)

### 1.9 GU Sprite Batching: Manual Vertices -> `psp::gu_ext::SpriteBatch`

**Current (lib.rs:396-596):** Every `fill_rect_inner`, `blit_inner`, and `draw_text_inner`
call individually allocates vertices via `sceGuGetMemory` and calls `sceGuDrawArray`.
Each glyph in `draw_text_inner` is a separate draw call.

**New:**
```rust
let mut batch = psp::gu_ext::SpriteBatch::new(256);

// In draw_text_inner -- batch all glyphs:
for ch in text.chars() {
    let (u0, v0) = glyph_uv(ch);
    batch.draw_rect(cx as f32, y as f32, 8.0*scale, 8.0*scale, u0, v0, u0+8.0, v0+8.0, abgr);
    cx += glyph_w;
}
unsafe { batch.flush(); }  // Single draw call for all glyphs
```

**Impact:**
- Text rendering: N draw calls -> 1 draw call (major GPU perf improvement)
- SpriteBatch handles dcache writeback automatically
- `psp::gu_ext::setup_2d()` replaces manual orthographic projection setup
- Delete manual vertex struct definitions (`ColorVertex`, `TexturedColorVertex`)
- Delete manual vertex type constants (`COLOR_VTYPE`, `TEXTURED_COLOR_VTYPE`)

### 1.10 VRAM Allocation: Already Uses `psp::vram_alloc` (No Change)

The current code already uses `psp::vram_alloc::get_vram_allocator()`. No change needed.

### 1.11 Wallpaper Generation: Software Math -> VFPU SIMD

**Current (lib.rs:1263-1352):** Software `sin_approx` (Taylor series, ~7 ops) and
`sqrt_approx` (Newton-Raphson, 4 iterations) for procedural wallpaper.

**New:**
```rust
use psp::math::{sinf, sqrtf};  // VFPU hardware, single instruction each

// Replace sin_approx(x) -> psp::math::sinf(x)
// Replace sqrt_approx(x) -> psp::math::sqrtf(x)
// Replace lerp_rgb -> psp::simd::vec4_lerp for 4-component simultaneous lerp
```

**Impact:**
- Delete `sin_approx()` and `sqrt_approx()` functions (~25 lines)
- Hardware VFPU sin/sqrt are exact and ~10x faster than software approximations
- Wallpaper generation becomes significantly faster (runs once at startup)
- `psp::simd::vec4_lerp` processes 4 floats simultaneously for color interpolation

---

## Part 2: New Feature Implementations

These add capabilities that OASIS OS doesn't currently have on PSP.

### 2.1 System Font Rendering: `psp::font::FontLib` + `FontRenderer`

**Current:** Embedded 8x8 bitmap font (font.rs, 132 lines). ASCII only (0x20-0x7E).
Fixed size, no anti-aliasing, no Unicode.

**New:** Use PSP's built-in TrueType system fonts via `psp::font::*`:
```rust
let fontlib = psp::font::FontLib::new(4).unwrap();
let font = fontlib.find_optimum(SansSerif, Regular, Latin).unwrap();
let atlas_vram = vram_allocator.alloc_texture_pixels(512, 512, PsmT8).unwrap();
let mut renderer = FontRenderer::new(&font, atlas_vram.as_mut_ptr_direct(), 16.0);

// In draw_text_inner:
renderer.draw_text(x as f32, y as f32, color_abgr, text);
renderer.flush();  // Single batched draw call
```

**Impact:**
- Anti-aliased TrueType fonts instead of blocky 8x8 bitmaps
- Variable font sizes (8.0, 12.0, 16.0, 20.0, etc.)
- `renderer.measure_text(text)` for precise layout calculations
- `renderer.line_height()` for proper line spacing
- Delete `font.rs` entirely (132 lines) -- no longer needed
- Delete `build_font_atlas()` method (~30 lines)
- Delete `font_atlas_ptr` field from `PspBackend`
- 512x512 VRAM glyph atlas (allocated once) vs 128x64 RAM atlas
- **Keep 8x8 bitmap font as fallback** for PPSSPP emulator compatibility (system fonts
  may not be available in emulator). Feature-gate with runtime detection.

### 2.2 Multi-Threading: `psp::thread::spawn()` (Fix TLS Limitation)

**Current (lib.rs:1072-1075):** Single background thread combining audio + I/O because
"PSP's std::thread has a TLS limitation that prevents spawning multiple threads."

**New:** rust-psp's `psp::thread::spawn()` uses native PSP kernel threads (`sceKernelCreateThread`)
which do NOT have the std TLS limitation. Multiple threads can be spawned:

```rust
use psp::thread;

// Dedicated audio thread
let audio_handle = thread::spawn(b"oasis_audio\0", move || {
    audio_worker(audio_rx, audio_state);
    0
}).unwrap();

// Dedicated I/O thread
let io_handle = thread::spawn(b"oasis_io\0", move || {
    io_worker(io_rx, io_tx);
    0
}).unwrap();

// Dedicated network thread (new!)
let net_handle = thread::spawn(b"oasis_net\0", move || {
    net_worker(net_rx, net_state);
    0
}).unwrap();
```

**Impact:**
- Separate audio and I/O into dedicated threads (currently forced into one)
- Audio thread no longer blocked by I/O operations
- I/O thread no longer blocked by `sceAudioOutputBlocking`
- Can add a dedicated network thread
- Use `psp::sync::SpscQueue` for lock-free inter-thread communication
- Use `psp::sync::Semaphore` or `EventFlag` for thread synchronization
- Delete the unified `WorkerCmd` enum -- split into `AudioCmd` and `IoCmd`
- Thread priority tuning: audio=high (16), I/O=normal (32), network=low (48)

### 2.3 Networking: `psp::net::*` -> Implement `NetworkBackend` Trait

**Current:** `NetworkBackend` trait defined in oasis-core but NOT implemented for PSP.
No networking at all on PSP.

**New:** Full TCP/UDP networking via `psp::net::*`:
```rust
pub struct PspNetworkBackend {
    listener: Option<PspTcpListener>,
}

impl NetworkBackend for PspNetworkBackend {
    fn listen(&mut self, port: u16) -> Result<()> {
        // Use raw sceNetInet for server sockets (psp::net has client-only TcpStream)
    }

    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
        // Accept incoming connections
    }

    fn connect(&mut self, address: &str, port: u16) -> Result<Box<dyn NetworkStream>> {
        let addr = psp::net::resolve_hostname(address.as_bytes())?;
        let stream = psp::net::TcpStream::connect(addr, port)?;
        Ok(Box::new(PspNetStream(stream)))
    }
}
```

**WiFi setup (new terminal commands):**
```
> wifi connect 1     # Connect to AP config slot 1
> wifi status        # Show IP address, signal strength
> wifi disconnect    # Disconnect
```

**Impact:**
- Enable remote terminal access over WiFi
- Enable MCP protocol support on PSP (agent integration)
- Add `wifi` terminal commands
- Network status in status bar (IP address when connected)
- HTTP client for downloading content via `psp::http::HttpClient`
- PSP becomes a networked node in the OASIS ecosystem

### 2.4 On-Screen Keyboard: `psp::osk::*` for Terminal Text Input

**Current:** Terminal has no text input method on PSP. Commands are hardcoded
(Up=help, Down=status). No way to type arbitrary commands.

**New:**
```rust
// When user presses a "type" button in terminal mode:
if ctrl.is_pressed(CtrlButtons::SQUARE) {
    match psp::osk::OskBuilder::new("Enter command")
        .max_chars(256)
        .initial_text(&term_input)
        .show()
    {
        Ok(Some(text)) => { term_input = text; }
        Ok(None) => {}  // Cancelled
        Err(e) => { term_lines.push(format!("OSK error: {:?}", e)); }
    }
}
```

**Impact:**
- Terminal becomes fully interactive -- users can type any command
- Square button opens PSP system keyboard overlay
- Input persists in terminal input field after OSK closes
- Enables command composition for networking, file management, etc.

### 2.5 System Dialogs: `psp::dialog::*`

**New capabilities:**
```rust
// Confirmation before destructive operations:
match psp::dialog::confirm_dialog("Delete this file?") {
    Ok(DialogResult::Confirm) => { psp::io::remove_file(path).ok(); }
    _ => {}
}

// Error display:
psp::dialog::error_dialog(error_code as u32).ok();

// Info messages:
psp::dialog::message_dialog("Settings saved successfully").ok();
```

**Impact:**
- Native PSP system dialogs for user confirmation
- Professional error reporting
- Used in file manager (delete confirmation), settings (save confirmation)

### 2.6 Persistent Configuration: `psp::config::Config`

**Current:** No persistent settings. Clock speed, theme, paths all reset on restart.

**New:**
```rust
const CONFIG_PATH: &str = "ms0:/PSP/GAME/OASISOS/config.rcfg";

fn load_config() -> psp::config::Config {
    psp::config::Config::load(CONFIG_PATH).unwrap_or_else(|_| psp::config::Config::new())
}

fn save_config(config: &psp::config::Config) {
    config.save(CONFIG_PATH).ok();
}

// Usage:
config.set("clock_mhz", ConfigValue::I32(333));
config.set("theme", ConfigValue::Str("classic".into()));
config.set("wifi_ap_index", ConfigValue::I32(1));
config.set("last_path", ConfigValue::Str(fm_path.clone()));
```

**Impact:**
- Persistent settings across restarts
- Remember last-used directories in file manager, photo viewer, music player
- Remember clock speed preference
- Remember WiFi AP configuration
- Settings app becomes functional (currently no persistence)

### 2.7 Save Data: `psp::savedata::Savedata`

**New:**
```rust
let save = psp::savedata::Savedata::new(b"OASIS000000\0\0")
    .title("OASIS OS Settings")
    .detail("Terminal history and window layout");

// Save terminal history + window positions
let data = serialize_state(&term_lines, &wm);
save.save(b"STATE000\0\0\0\0\0\0\0\0\0\0\0\0", &data).ok();

// Load on startup
if let Ok(data) = save.load(b"STATE000\0\0\0\0\0\0\0\0\0\0\0\0", 65536) {
    deserialize_state(&data, &mut term_lines, &mut wm);
}
```

**Impact:**
- System-managed save data with PSP's native save/load UI
- Terminal history preservation
- Window layout restoration (desktop mode)
- Appears in PSP's XMB save data manager

### 2.8 USB Storage Mode: `psp::usb::*`

**Current:** USB state is queried for status bar display only.

**New:**
```rust
// Terminal command: usb mount
psp::usb::start_bus().ok();
let _storage = psp::usb::UsbStorageMode::activate().unwrap();
// Memory Stick is now accessible from PC as USB drive
// Auto-cleanup on drop exits USB mode
```

**Impact:**
- `usb mount` / `usb unmount` terminal commands
- File transfer between PSP and PC without removing Memory Stick
- Status bar shows "USB ACTIVE" when mounted
- RAII: auto-unmount when leaving USB mode

### 2.9 Screenshot Capture: `psp::screenshot::*`

**Current (lib.rs:968-971):** `read_pixels()` returns error "not supported on PSP".

**New:**
```rust
impl SdiBackend for PspBackend {
    fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> OasisResult<Vec<u8>> {
        Ok(psp::screenshot::screenshot_argb_be()
            .iter()
            .flat_map(|&pixel| {
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (pixel & 0xFF) as u8;
                let a = ((pixel >> 24) & 0xFF) as u8;
                [r, g, b, a]
            })
            .collect())
    }
}
```

**Terminal command:**
```
> screenshot
Saved: ms0:/PSP/GAME/OASISOS/screenshots/shot_20260209_1430.bmp
```

**Impact:**
- `read_pixels()` now works -- enables oasis-screenshot binary equivalent
- `screenshot` terminal command saves BMP to Memory Stick
- Can be used by oasis-core for testing/CI via PPSSPP

### 2.10 DMA Transfers: `psp::dma::*`

**New:** Use DMA for large memory copies (texture loading, framebuffer operations):

```rust
// In load_texture_inner -- DMA copy source rows to POT buffer:
unsafe {
    for row in 0..height as usize {
        psp::dma::memcpy_dma(
            data.add(row * dst_stride),
            rgba_data.as_ptr().add(row * src_stride),
            src_stride as u32,
        ).ok();
    }
}
```

**Impact:**
- Faster texture uploads (DMA offloads CPU)
- Particularly beneficial for large textures (wallpaper, photos)
- `psp::dma::vram_blit_dma()` for direct VRAM writes

### 2.11 Audio Mixer: `psp::audio_mixer::Mixer` for Multi-Channel Audio

**Current:** Single-channel MP3 playback only. No sound effects.

**New:**
```rust
let mixer = psp::audio_mixer::Mixer::new(1024).unwrap();

// Music channel (looping)
let music_ch = mixer.alloc_channel(ChannelConfig {
    volume_left: 0x6000, volume_right: 0x6000, looping: true,
}).unwrap();

// SFX channel (one-shot)
let sfx_ch = mixer.alloc_channel(ChannelConfig {
    volume_left: 0x8000, volume_right: 0x8000, looping: false,
}).unwrap();

// Mix and output:
let mut output = vec![0i16; 2048];
mixer.mix_into(&mut output);
mixer.output_blocking(&output).unwrap();
```

**Impact:**
- UI sound effects (click, navigate, error) alongside music
- Up to 8 simultaneous audio channels
- Per-channel volume control
- Fade in/out support for smooth transitions
- Master volume control

### 2.12 Framebuffer Layer Compositor: `psp::framebuffer::LayerCompositor`

**New:** Use the layer system for efficient partial-screen updates:

```rust
let mut compositor = psp::framebuffer::LayerCompositor::new(DisplayPixelFormat::Psm8888);

// Only redraw changed regions:
compositor.mark_dirty(Layer::Content, x, y, w, h);  // App content changed
compositor.mark_dirty(Layer::Overlay, 0, 0, 480, 24); // Status bar updated

// Composite only dirty regions to output buffer
unsafe { compositor.composite_to(framebuf, &layer_buffers); }
compositor.clear_all_dirty();
```

**Impact:**
- Potential for partial-screen rendering (skip unchanged regions)
- Background layer (wallpaper), Content layer (app), Overlay layer (status/cursor)
- `DirtyRect` tracking for efficient redraws
- Particularly valuable for terminal mode (only redraw new lines)

### 2.13 Media Engine Offloading: `psp::me::MeExecutor` (Kernel Mode)

**Current:** The PSP's second 333MHz MIPS core (Media Engine) is completely unused.

**New:** Offload compute-heavy tasks to the ME:
```rust
let mut executor = psp::me::MeExecutor::new(4096).unwrap();

// Offload wallpaper generation to ME core:
unsafe extern "C" fn generate_wallpaper_me(arg: i32) -> i32 {
    // This runs on the ME core while main CPU handles input
    generate_gradient_section(arg);
    0
}

let handle = unsafe { executor.submit(generate_wallpaper_me, section_id) };
// Main CPU continues handling input/rendering while ME generates wallpaper
let result = executor.wait(&handle);
```

**Impact:**
- Wallpaper generation on ME while main CPU handles boot sequence
- JPEG decoding offloaded to ME (parallel with rendering)
- Effectively doubles available CPU power for compute tasks
- Requires kernel mode (already enabled via `module_kernel!`)

### 2.14 RTC and System Info: `psp::rtc::*` + `psp::system_param::*`

**Current:** Raw `sceRtcGetCurrentClockLocalTime` for clock display.

**New:**
```rust
// Rich date/time display:
let tick = psp::rtc::Tick::now().unwrap();
let formatted = psp::rtc::format_rfc3339_local(&tick).unwrap();
let dow = psp::rtc::day_of_week(dt.year(), dt.month(), dt.day()); // 0=Mon..6=Sun

// System info in settings/terminal:
let lang = psp::system_param::language().unwrap();
let nick = psp::system_param::nickname().unwrap();
let tz = psp::system_param::timezone_offset().unwrap();
```

**Impact:**
- Day-of-week display in status bar
- Full RFC 3339 timestamps in terminal
- User's PSP nickname available for display
- Timezone-aware time display
- Settings app can show system parameters

### 2.15 Benchmarking: `psp::benchmark::benchmark()`

**New terminal command:**
```
> benchmark
Wallpaper generation: 12.3ms avg (100 iterations)
Font atlas build: 0.8ms avg (100 iterations)
Full frame render: 4.2ms avg (100 iterations)
```

**Impact:**
- Performance profiling via terminal
- Useful for optimization work
- `psp::time::Instant` for precise timing of individual operations

---

## Part 3: Synchronization and Threading Overhaul

### 3.1 Replace `std::sync::mpsc` with `psp::sync::SpscQueue`

**Current:** `mpsc::channel()` for command passing between main thread and worker.

**New:** Lock-free SPSC queues for each communication path:
```rust
static AUDIO_CMD_QUEUE: psp::sync::SpscQueue<AudioCmd, 16> = SpscQueue::new();
static IO_CMD_QUEUE: psp::sync::SpscQueue<IoCmd, 32> = SpscQueue::new();
static IO_RESP_QUEUE: psp::sync::SpscQueue<IoResponse, 16> = SpscQueue::new();
```

**Impact:**
- Zero allocation, zero-lock inter-thread communication
- Compile-time capacity (no heap allocation for channel buffers)
- Power-of-2 ring buffer for cache-friendly access
- Separate queues for audio, I/O, and network commands

### 3.2 Replace `std::sync::atomic` with `psp::sync::SpinMutex` Where Appropriate

**Current:** `Arc<AudioState>` with individual `AtomicBool`/`AtomicU32` fields.

**New:** Where atomics suffice, keep them. Where richer state is needed:
```rust
static AUDIO_STATE: SpinMutex<AudioStateInner> = SpinMutex::new(AudioStateInner::default());

struct AudioStateInner {
    playing: bool,
    paused: bool,
    sample_rate: u32,
    bitrate: u32,
    channels: u32,
    position_ms: u64,
    duration_ms: u64,  // NEW: track duration
    track_name: [u8; 64], // NEW: current track name
}
```

**Impact:**
- Richer shared state without individual atomics
- `SpinMutex` is appropriate for PSP (single-core, short critical sections)
- Can share more complex data between threads

---

## Part 4: Structural Refactoring

### 4.1 Implement `AudioBackend` Trait

**Current:** Audio is a custom `AudioPlayer` -- doesn't implement the `AudioBackend` trait
from oasis-core.

**New:**
```rust
impl AudioBackend for PspAudioBackend {
    fn init(&mut self) -> Result<()> { ... }
    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> { ... }
    fn play(&mut self, track: AudioTrackId) -> Result<()> { ... }
    fn pause(&mut self) -> Result<()> { ... }
    fn resume(&mut self) -> Result<()> { ... }
    fn stop(&mut self) -> Result<()> { ... }
    fn set_volume(&mut self, volume: u8) -> Result<()> { ... }
    fn is_playing(&self) -> bool { ... }
    fn position_ms(&self) -> u64 { ... }  // NEW: sceMp3 tracking
    fn duration_ms(&self) -> u64 { ... }  // NEW: from MP3 headers
}
```

**Impact:**
- Consistent audio API across all OASIS backends (SDL, PSP, UE5)
- oasis-core apps can use audio through the trait interface
- Volume control exposed (currently hardcoded to max)
- Position/duration tracking enables progress bar display

### 4.2 Split `lib.rs` into Modules

**Current:** Single 2168-line `lib.rs` containing everything.

**New module structure:**
```
crates/oasis-backend-psp/src/
├── lib.rs           # Re-exports, PspBackend struct, SdiBackend impl
├── audio.rs         # AudioPlayer, AudioBackend impl, mixer integration
├── input.rs         # InputBackend impl using psp::input::Controller
├── network.rs       # NetworkBackend impl using psp::net::*
├── render.rs        # GU rendering helpers, SpriteBatch integration
├── font.rs          # FontRenderer wrapper (system fonts + bitmap fallback)
├── textures.rs      # Texture management, volatile allocator
├── filesystem.rs    # list_directory, read_file using psp::io
├── status.rs        # StatusBarInfo, SystemInfo using psp::power/rtc
├── procedural.rs    # Wallpaper generator, cursor generator (VFPU math)
├── threading.rs     # Thread spawning, SPSC queues, shared state
├── config.rs        # Persistent config wrapper
└── main.rs          # Entry point, app modes, rendering functions
```

**Impact:**
- Better code organization and maintainability
- Each module has a clear responsibility
- Easier to test individual components
- Parallel development on different modules

### 4.3 Refactor `main.rs` App Modes

**Current:** 1820-line `main.rs` with all app logic (dashboard, terminal, file manager,
photo viewer, music player, desktop mode) in one function.

**New:** Extract each app into its own struct with `update()` and `draw()` methods:
```rust
trait PspApp {
    fn handle_input(&mut self, event: &InputEvent, backend: &mut PspBackend);
    fn update(&mut self);
    fn draw(&self, backend: &mut PspBackend);
}

struct TerminalApp { lines: Vec<String>, input: String, ... }
struct FileManagerApp { path: String, entries: Vec<FileEntry>, ... }
struct PhotoViewerApp { ... }
struct MusicPlayerApp { ... }
struct SettingsApp { config: psp::config::Config, ... }
struct NetworkApp { ... }
struct SysMonitorApp { frame_timer: FrameTimer, ... }
```

**Impact:**
- Each app is self-contained with its own state
- Apps can be independently developed and tested
- Cleaner main loop: just dispatch to active app
- Settings and Network apps become functional (currently placeholder)
- System Monitor app can show FPS, memory usage, thread status

---

## Part 5: New Terminal Commands

With the new capabilities, the terminal gains many new commands:

```
# Existing (improved)
help            - Show all commands (expanded list)
status          - System status (now with IP, FPS, thread count)
ls [path]       - List directory (using psp::io iterators)
clock [speed]   - Clock management (using psp::power)
clear           - Clear terminal
version         - Version info
about           - About OASIS_OS

# New: Networking
wifi connect N  - Connect to AP slot N
wifi status     - Show IP address, signal info
wifi disconnect - Disconnect from AP
ping HOST       - ICMP-like connectivity test (TCP connect)
http GET URL    - HTTP GET request (psp::http)
curl URL        - Alias for http GET

# New: File Operations
cat PATH        - Display file contents
mkdir PATH      - Create directory
rm PATH         - Delete file (with dialog confirmation)
cp SRC DST      - Copy file
mv SRC DST      - Move/rename file

# New: System
screenshot      - Capture screen to BMP
usb mount       - Enter USB storage mode
usb unmount     - Exit USB storage mode
benchmark       - Run performance benchmarks
threads         - List active threads
mem             - Memory usage report
config KEY=VAL  - Set persistent config
config KEY      - Get config value

# New: Audio
play PATH       - Play MP3 file
pause           - Pause playback
resume          - Resume playback
stop            - Stop playback
volume N        - Set volume (0-100)
```

---

## Part 6: Implementation Order

Prioritized by dependency chain and impact:

### Phase 1: Foundation (No New Features, Code Improvement Only)
1. Split `lib.rs` into modules (4.2)
2. Replace input with `psp::input::Controller` (1.1)
3. Replace file I/O with `psp::io::*` (1.2)
4. Replace power management with `psp::power::*` (1.5)
5. Replace exit callback (1.6)
6. Add FrameTimer (1.7)

### Phase 2: Rendering Improvements
7. Replace JPEG decoding with `psp::image::*` (1.4)
8. Integrate SpriteBatch for batched rendering (1.9)
9. Replace software math with VFPU (1.11)
10. Add cache type-safety wrappers (1.8)
11. Add screenshot support (2.9)

### Phase 3: Audio Overhaul
12. Replace AudioPlayer with `psp::mp3::Mp3Decoder` + `psp::audio::AudioChannel` (1.3)
13. Implement `AudioBackend` trait (4.1)
14. Add audio mixer for multi-channel (2.11)

### Phase 4: Threading Revolution
15. Switch to `psp::thread::spawn()` for multi-threading (2.2)
16. Replace mpsc with SpscQueue (3.1)
17. Split unified worker into audio + I/O + network threads
18. Add SpinMutex-based shared state (3.2)

### Phase 5: System Font Rendering
19. Integrate `psp::font::FontRenderer` (2.1)
20. Runtime detection: system font vs bitmap fallback
21. Variable font size support throughout UI

### Phase 6: Networking
22. Implement NetworkBackend with `psp::net::*` (2.3)
23. WiFi terminal commands
24. HTTP client integration (2.3)
25. Remote terminal access foundation

### Phase 7: User Experience
26. On-screen keyboard for terminal input (2.4)
27. System dialogs for confirmations (2.5)
28. Persistent configuration (2.6)
29. Save data integration (2.7)
30. USB storage mode (2.8)

### Phase 8: Advanced Features
31. DMA transfers for texture loading (2.10)
32. Layer compositor for partial updates (2.12)
33. Media Engine offloading (2.13)
34. Rich RTC and system param display (2.14)
35. Benchmarking framework (2.15)

### Phase 9: App Refactoring
36. Refactor main.rs apps into structs (4.3)
37. Implement Settings app with persistent config
38. Implement Network app with WiFi management
39. Implement System Monitor app with FPS/memory
40. Expand terminal commands (Part 5)

---

## Part 7: Risk Assessment

### Low Risk
- Input, File I/O, Power, RTC replacements (1:1 API mapping)
- VFPU math replacement (drop-in)
- Screenshot, Config, Dialogs (additive features)

### Medium Risk
- Audio overhaul (threading model changes)
- SpriteBatch integration (rendering pipeline change)
- System fonts (PPSSPP compatibility uncertain)
- Threading model change (need careful synchronization)

### High Risk
- Networking (WiFi hardware access, AP configuration complexity)
- Media Engine offloading (shared memory, cache coherency, kernel-only)
- Layer compositor (fundamental rendering architecture change)

### Mitigations
- System fonts: keep bitmap font as runtime fallback
- Threading: incremental migration (one thread at a time)
- Networking: test on real hardware; WiFi requires custom firmware
- ME offloading: optional optimization, not critical path

---

## Part 8: Estimated Line Count Impact

| Change                      | Lines Deleted | Lines Added | Net    |
|-----------------------------|---------------|-------------|--------|
| Input replacement           | ~115          | ~50         | -65    |
| File I/O replacement        | ~120          | ~30         | -90    |
| Audio replacement           | ~300          | ~80         | -220   |
| JPEG replacement            | ~55           | ~5          | -50    |
| Power management            | ~85           | ~30         | -55    |
| Font atlas (delete)         | ~165          | ~0          | -165   |
| Math functions (delete)     | ~25           | ~0          | -25    |
| GU vertex defs (delete)     | ~30           | ~0          | -30    |
| **Subtotal: Removals**      | **~895**      | **~195**    | **-700** |
| System fonts (new)          | 0             | ~60         | +60    |
| NetworkBackend (new)        | 0             | ~150        | +150   |
| AudioBackend trait (new)    | 0             | ~80         | +80    |
| OSK integration (new)       | 0             | ~30         | +30    |
| Config persistence (new)    | 0             | ~40         | +40    |
| Dialog integration (new)    | 0             | ~20         | +20    |
| USB storage mode (new)      | 0             | ~30         | +30    |
| Screenshot (new)            | 0             | ~20         | +20    |
| Terminal commands (new)      | 0             | ~200        | +200   |
| Module split boilerplate    | 0             | ~100        | +100   |
| **Subtotal: Additions**     | **0**         | **~730**    | **+730** |
| **Total**                   | **~895**      | **~925**    | **+30**  |

Net effect: roughly the same total line count, but dramatically more functionality
and safety. The codebase becomes ~700 lines of raw unsafe syscall wrappers lighter
and gains ~730 lines of new features built on safe abstractions.

---

## Dependency Changes

### Cargo.toml Update
```toml
[dependencies]
# Point to latest rust-psp with all new modules
psp = { git = "https://github.com/AndrewAltimit/rust-psp", features = ["kernel", "std"] }
oasis-core = { path = "../oasis-core" }
```

No new external dependencies needed -- everything comes from the `psp` crate.

---

## Testing Strategy

1. **PPSSPP headless tests:** Verify all non-hardware features (input, file I/O, rendering,
   config, screenshots)
2. **Real hardware tests:** WiFi networking, USB storage, system fonts, audio mixer,
   Media Engine (these require actual PSP hardware)
3. **CI integration:** Screenshot comparison tests via `oasis-screenshot` + `read_pixels()`
4. **Benchmark regression:** Track frame timing across changes via `psp::benchmark`
