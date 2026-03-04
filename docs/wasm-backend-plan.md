# WASM Backend Implementation Plan

## Overview

Add a WebAssembly backend to OASIS_OS so it can run in web browsers. This follows
the established backend pattern (SDL, UE5, PSP) by creating a new
`oasis-backend-wasm` crate that implements `SdiCore`/`SdiBackend`, `InputBackend`,
`AudioBackend`, and `NetworkBackend` using Web APIs via `wasm-bindgen`.

The UE5 backend is the closest architectural reference: it uses a software RGBA
pixel buffer, externally-driven tick model, and push-based input — exactly the
pattern needed for WASM.

**Target**: `wasm32-unknown-unknown` compiled with `wasm-bindgen` + `wasm-pack`

**Native resolution**: 480x272 (PSP native), rendered to a `<canvas>` element and
scaled to fit the browser viewport.

---

## Architecture Decision: Canvas 2D vs WebGL vs Pixel Buffer

Three rendering approaches are possible:

| Approach | Pros | Cons |
|----------|------|------|
| **Software pixel buffer → Canvas `putImageData`** | Closest to UE5 backend, simplest, zero new rendering code | Slow for large displays (CPU-bound pixel copy every frame) |
| **Canvas 2D API** | Native browser acceleration, sub-pixel text, built-in gradients | Requires reimplementing all drawing ops as Canvas calls |
| **WebGL** | GPU-accelerated, best perf for textures | Most complex, overkill for 480x272 |

**Recommendation: Hybrid — Canvas 2D API as primary, with pixel buffer fallback.**

At 480x272 (130,560 pixels), even `putImageData` is fast. But Canvas 2D gives us
free anti-aliased shapes, gradients, and text rendering — a clear upgrade over
the bitmap font rasterizer. The implementation will:

1. Use Canvas 2D for all shape/text/gradient primitives (accelerated path)
2. Use `ImageData` for texture blitting (load RGBA → `createImageBitmap` → `drawImage`)
3. Use `getImageData` for `read_pixels()` (screenshot support)

This gives the best quality with minimal complexity. If we later need more perf
(e.g., running at 1920x1080 native), we can add a WebGL path behind a feature flag.

---

## Phase 1: Foundation — Workspace Setup & Compilation Gates

**Goal**: Get the core crates compiling for `wasm32-unknown-unknown`.

### Step 1.1: Create `oasis-backend-wasm` crate skeleton

```
crates/oasis-backend-wasm/
├── Cargo.toml
└── src/
    ├── lib.rs          # WasmBackend struct + wasm-bindgen exports
    ├── renderer.rs     # SdiBackend impl (Canvas 2D)
    ├── input.rs        # InputBackend impl (DOM events)
    ├── audio.rs        # AudioBackend impl (Web Audio API)
    ├── network.rs      # NetworkBackend stub
    └── font.rs         # Re-export oasis_types::bitmap_font
```

Dependencies:
```toml
[dependencies]
oasis-core = { workspace = true }
oasis-types = { workspace = true }
wasm-bindgen = "0.2"
web-sys = { version = "0.3", features = [
    "Window", "Document", "Element", "HtmlCanvasElement",
    "CanvasRenderingContext2d", "ImageData", "ImageBitmap",
    "KeyboardEvent", "MouseEvent", "WheelEvent", "FocusEvent",
    "EventTarget", "AddEventListenerOptions",
    "AudioContext", "AudioBuffer", "AudioBufferSourceNode",
    "GainNode", "AudioDestinationNode",
    "console",
] }
js-sys = "0.3"
wasm-bindgen-futures = "0.4"
log = { workspace = true }
```

### Step 1.2: Add `#[cfg(not(target_arch = "wasm32"))]` gates to blockers

The following modules use `std::net` or `std::fs` which don't exist on
`wasm32-unknown-unknown`. Gate them:

**oasis-vfs** — `RealVfs` uses `std::fs`:
```rust
// crates/oasis-vfs/src/lib.rs
#[cfg(not(target_arch = "wasm32"))]
pub mod real;
#[cfg(not(target_arch = "wasm32"))]
pub use real::RealVfs;
```
`MemoryVfs` is pure Rust and works on WASM as-is.

**oasis-net** — `StdNetworkBackend` uses `std::net`:
```rust
// crates/oasis-net/src/lib.rs
#[cfg(not(target_arch = "wasm32"))]
pub mod std_backend;
#[cfg(not(target_arch = "wasm32"))]
pub use std_backend::StdNetworkBackend;
```

**oasis-browser** — HTTP/Gemini loaders use `std::net::TcpStream`:
```rust
// crates/oasis-browser/src/loader/mod.rs
#[cfg(not(target_arch = "wasm32"))]
pub mod http;
#[cfg(not(target_arch = "wasm32"))]
pub mod gemini_fetch;

// Provide WASM stubs that return "not supported" errors
#[cfg(target_arch = "wasm32")]
pub mod http { /* stub returning OasisError */ }
#[cfg(target_arch = "wasm32")]
pub mod gemini_fetch { /* stub returning OasisError */ }
```

**oasis-terminal** — Some commands use platform features:
```rust
// Commands that reference std::fs paths need cfg gates
#[cfg(not(target_arch = "wasm32"))]
// ... real filesystem commands
```

### Step 1.3: Add workspace member and wasm-bindgen deps

```toml
# Root Cargo.toml [workspace] members:
"crates/oasis-backend-wasm",

# Root Cargo.toml [workspace.dependencies]:
wasm-bindgen = "0.2"
web-sys = "0.3"
js-sys = "0.3"
wasm-bindgen-futures = "0.4"
```

### Step 1.4: Verify core crates compile for wasm32

```bash
cargo check --target wasm32-unknown-unknown -p oasis-types
cargo check --target wasm32-unknown-unknown -p oasis-sdi
cargo check --target wasm32-unknown-unknown -p oasis-ui
cargo check --target wasm32-unknown-unknown -p oasis-wm
cargo check --target wasm32-unknown-unknown -p oasis-skin
cargo check --target wasm32-unknown-unknown -p oasis-vfs
cargo check --target wasm32-unknown-unknown -p oasis-audio
cargo check --target wasm32-unknown-unknown -p oasis-platform
cargo check --target wasm32-unknown-unknown -p oasis-terminal
cargo check --target wasm32-unknown-unknown -p oasis-net
cargo check --target wasm32-unknown-unknown -p oasis-browser
cargo check --target wasm32-unknown-unknown -p oasis-core
```

Fix any compilation errors discovered (likely in oasis-terminal commands that
assume filesystem access). Each fix uses `#[cfg(not(target_arch = "wasm32"))]`.

---

## Phase 2: SdiCore/SdiBackend — Canvas 2D Rendering

**Goal**: Implement `SdiCore` + `SdiBackend` traits for `WasmBackend` using Canvas 2D API.

### Step 2.1: WasmBackend struct

```rust
pub struct WasmBackend {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    width: u32,           // 480
    height: u32,          // 272
    textures: HashMap<u64, TextureData>,
    next_texture_id: u64,
    clip_stack: Vec<ClipRect>,
    translate_stack: Vec<(i32, i32)>,
    cumulative_translate: (i32, i32),
    dirty: bool,
}

struct TextureData {
    image_data: ImageData,  // Raw RGBA for putImageData path
    width: u32,
    height: u32,
}
```

### Step 2.2: Core rendering methods (13 required)

| Trait method | Canvas 2D implementation |
|---|---|
| `init(w, h)` | Set canvas dimensions, get 2D context |
| `clear(color)` | `ctx.fillStyle = color; ctx.fillRect(0, 0, w, h)` |
| `swap_buffers()` | No-op (canvas is immediately visible) |
| `fill_rect(x, y, w, h, color)` | `ctx.fillStyle = color; ctx.fillRect(...)` |
| `draw_text(text, x, y, size, color)` | Bitmap font rasterizer from `oasis_types::bitmap_font` (same as UE5) via `ctx.fillRect` per pixel, OR use `ctx.fillText` with a monospace web font |
| `blit(tex, x, y, w, h)` | `ctx.putImageData(tex.image_data, x, y)` or `ctx.drawImage(bitmap, ...)` |
| `load_texture(w, h, rgba)` | Create `ImageData` from RGBA bytes, store in HashMap |
| `destroy_texture(tex)` | Remove from HashMap |
| `set_clip_rect(x, y, w, h)` | `ctx.save(); ctx.beginPath(); ctx.rect(...); ctx.clip()` |
| `reset_clip_rect()` | `ctx.restore()` |
| `measure_text(text, size)` | Use `oasis_types::bitmap_font` glyph metrics (consistent with other backends) |
| `read_pixels(x, y, w, h)` | `ctx.getImageData(x, y, w, h).data()` |
| `shutdown()` | Drop references |

### Step 2.3: Extended shape primitives

Canvas 2D natively supports all the extended primitives:

| Primitive | Canvas 2D approach |
|---|---|
| `fill_rounded_rect` | `ctx.roundRect(...)` or manual arc+line path |
| `stroke_rect` | `ctx.strokeRect(...)` with `lineWidth` |
| `draw_line` | `ctx.beginPath(); ctx.moveTo; ctx.lineTo; ctx.stroke()` |
| `fill_circle` | `ctx.arc(...); ctx.fill()` |
| `fill_triangle` | `ctx.beginPath(); moveTo/lineTo ×3; ctx.fill()` |
| `fill_rect_gradient` | `ctx.createLinearGradient(...)` |
| `fill_rect_alpha` | Set `ctx.globalAlpha` temporarily |
| `dim_screen` | Fill with `rgba(0,0,0,alpha)` |

### Step 2.4: Text rendering strategy

Two options — implement both, configurable:

**Option A (default)**: Bitmap font rasterizer — uses `oasis_types::bitmap_font`
glyphs, renders each character as filled rectangles on canvas. Pixel-perfect match
with SDL/UE5/PSP backends. Low quality but consistent.

**Option B (enhanced)**: Canvas `fillText` with a loaded web font that matches the
bitmap metrics. Higher quality text but may have subtle layout differences. Gate
behind a feature flag `web-fonts`.

Start with Option A for correctness, add Option B later.

### Step 2.5: Clip and translate stacks

Canvas 2D has `save()`/`restore()` which manage clip state naturally:

```rust
fn push_clip_rect(&mut self, x, y, w, h) {
    self.ctx.save();
    let (tx, ty) = self.cumulative_translate;
    self.ctx.begin_path();
    self.ctx.rect((x + tx) as f64, (y + ty) as f64, w as f64, h as f64);
    self.ctx.clip();
    self.clip_stack.push(ClipRect { x, y, w, h });
}

fn pop_clip_rect(&mut self) {
    self.ctx.restore();
    self.clip_stack.pop();
}

fn push_translate(&mut self, dx, dy) {
    self.translate_stack.push(self.cumulative_translate);
    self.cumulative_translate.0 += dx;
    self.cumulative_translate.1 += dy;
}

fn pop_translate(&mut self) {
    self.cumulative_translate = self.translate_stack.pop().unwrap_or((0, 0));
}
```

---

## Phase 3: InputBackend — DOM Event Handling

**Goal**: Map browser keyboard/mouse/touch events to `InputEvent`.

### Step 3.1: WasmInputBackend

```rust
pub struct WasmInputBackend {
    events: Vec<InputEvent>,
    // Closures stored to prevent them from being dropped
    _closures: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}
```

### Step 3.2: DOM event listeners

Register on the canvas element during init:

| DOM Event | InputEvent mapping |
|---|---|
| `keydown` Arrow keys | `ButtonPress(Up/Down/Left/Right)` |
| `keydown` Enter | `ButtonPress(Confirm)` |
| `keydown` Escape | `ButtonPress(Cancel)` |
| `keydown` Space | `ButtonPress(Triangle)` |
| `keydown` Tab | `ButtonPress(Square)` |
| `keydown` F1/F2 | `ButtonPress(Start/Select)` |
| `keydown` Q/E | `TriggerPress(Left/Right)` |
| `keydown` Backspace | `Backspace` |
| `keyup` (same keys) | `ButtonRelease(...)` / `TriggerRelease(...)` |
| `input` (text) | `TextInput(char)` |
| `mousemove` | `CursorMove { x, y }` (scaled to 480x272) |
| `mousedown` | `PointerClick { x, y }` |
| `mouseup` | `PointerRelease { x, y }` |
| `wheel` | `MouseWheel { delta }` |
| `focus` / `blur` | `FocusGained` / `FocusLost` |
| `touchstart/move/end` | Mapped to pointer events (mobile support) |

### Step 3.3: Coordinate scaling

The canvas renders at 480x272 but may be displayed at any size. Mouse/touch
coordinates must be scaled:

```rust
fn scale_coords(&self, client_x: f64, client_y: f64) -> (i32, i32) {
    let rect = self.canvas.get_bounding_client_rect();
    let scale_x = 480.0 / rect.width();
    let scale_y = 272.0 / rect.height();
    let x = ((client_x - rect.left()) * scale_x) as i32;
    let y = ((client_y - rect.top()) * scale_y) as i32;
    (x.clamp(0, 479), y.clamp(0, 271))
}
```

### Step 3.4: Event queue pattern

DOM events fire asynchronously. We buffer them and drain on `poll_events()`:

```rust
// In event listener closures:
let events = Rc<RefCell<Vec<InputEvent>>>;
// Push events into shared vec

// In poll_events():
fn poll_events(&mut self) -> Vec<InputEvent> {
    std::mem::take(&mut *self.events.borrow_mut())
}
```

Uses `Rc<RefCell<>>` since WASM is single-threaded (no Arc/Mutex needed).

---

## Phase 4: AudioBackend — Web Audio API

**Goal**: Implement audio playback via the Web Audio API.

### Step 4.1: WasmAudioBackend

```rust
pub struct WasmAudioBackend {
    ctx: Option<AudioContext>,
    tracks: HashMap<u64, AudioBuffer>,
    next_id: u64,
    current_source: Option<AudioBufferSourceNode>,
    current_track: Option<u64>,
    gain_node: Option<GainNode>,
    volume: u8,
    playing: bool,
    paused: bool,
    start_time: f64,
    pause_offset: f64,
}
```

### Step 4.2: Implementation

- `init()` — Create `AudioContext` (lazy, triggered on first user interaction
  to comply with browser autoplay policy)
- `load_track(data)` — Decode audio bytes via `AudioContext.decodeAudioData()`
  (supports MP3, WAV, OGG natively in browsers)
- `play(track)` — Create `AudioBufferSourceNode`, connect to gain node, start
- `pause()` — Suspend `AudioContext`, record offset
- `resume()` — Resume `AudioContext`
- `stop()` — Stop source node, reset offset
- `set_volume(vol)` — Set `GainNode.gain.value` (0.0-1.0, mapped from 0-100)

### Step 4.3: Autoplay policy handling

Browsers block audio before user interaction. Handle by:
1. Creating `AudioContext` in `init()` but leaving it suspended
2. On first `play()` call, resume the context
3. If resume fails, register a one-shot click handler on the document that
   resumes the context, then retry playback

---

## Phase 5: NetworkBackend — Web Fetch Stub

**Goal**: Provide a minimal `NetworkBackend` for WASM. Full TCP is not available
in browsers, but HTTP fetching is.

### Step 5.1: WasmNetworkBackend (stub)

```rust
pub struct WasmNetworkBackend;

impl NetworkBackend for WasmNetworkBackend {
    fn listen(&mut self, _port: u16) -> Result<()> {
        Err(OasisError::Backend("TCP listen not available in browser".into()))
    }

    fn accept(&mut self) -> Result<Option<Box<dyn NetworkStream>>> {
        Err(OasisError::Backend("TCP accept not available in browser".into()))
    }

    fn connect(&mut self, _addr: &str, _port: u16) -> Result<Box<dyn NetworkStream>> {
        Err(OasisError::Backend("TCP connect not available in browser (use fetch)".into()))
    }
}
```

TCP server features (remote terminal, FTP) are not possible in browsers. This is
acceptable — the WASM build focuses on the local UI experience.

### Step 5.2: Future — HTTP via Fetch API

For the browser engine's HTTP loader, a future phase could add:
```rust
#[cfg(target_arch = "wasm32")]
pub async fn fetch_url(url: &str) -> Result<Vec<u8>> {
    // Use web_sys::Request + web_sys::window().fetch()
}
```

This would require the browser module's loader to support async fetching, which
is a significant refactor. Defer to a follow-up PR.

---

## Phase 6: JavaScript Glue & Entry Point

**Goal**: Create the wasm-bindgen exports and JavaScript harness.

### Step 6.1: Rust exports (`lib.rs`)

```rust
#[wasm_bindgen]
pub struct OasisWasm {
    backend: WasmBackend,
    input: WasmInputBackend,
    audio: WasmAudioBackend,
    // ... same fields as OasisInstance in oasis-ffi
    sdi: SdiRegistry,
    cmd_reg: CommandRegistry,
    vfs: MemoryVfs,
    platform: WasmPlatform,
    skin: Option<Skin>,
    active_theme: ActiveTheme,
    dashboard: Option<DashboardState>,
    cwd: String,
    output_lines: Vec<String>,
}

#[wasm_bindgen]
impl OasisWasm {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas_id: &str, skin_toml: Option<String>) -> Result<OasisWasm, JsValue>;

    pub fn tick(&mut self, delta_seconds: f32);

    pub fn send_command(&mut self, cmd: &str) -> String;

    pub fn resize(&mut self, display_width: u32, display_height: u32);
}
```

### Step 6.2: WasmPlatform (platform services)

```rust
pub struct WasmPlatform;

impl TimeService for WasmPlatform {
    fn now(&self) -> SystemTime {
        // js_sys::Date::now() → Duration → SystemTime
    }
}

impl PowerService for WasmPlatform {
    fn battery_percent(&self) -> Option<u8> {
        None // or use Navigator.getBattery() API
    }
    fn is_charging(&self) -> Option<bool> { None }
}

// USB, Network services return not-available
```

### Step 6.3: JavaScript wrapper (`www/index.html` + `www/index.js`)

```
www/
├── index.html       # Minimal HTML with <canvas id="oasis">
├── index.js         # Load WASM, create OasisWasm, run animation loop
├── style.css        # Fullscreen canvas, aspect-ratio preservation
└── webpack.config.js  OR  vite.config.js
```

**index.js** (core loop):
```javascript
import init, { OasisWasm } from './pkg/oasis_backend_wasm.js';

async function main() {
    await init();
    const oasis = new OasisWasm("oasis", null);

    let lastTime = performance.now();
    function frame(now) {
        const delta = (now - lastTime) / 1000.0;
        lastTime = now;
        oasis.tick(delta);
        requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
}

main();
```

### Step 6.4: Canvas scaling CSS

```css
#oasis {
    width: 100%;
    max-width: 960px;     /* 2x native */
    aspect-ratio: 480 / 272;
    image-rendering: pixelated;  /* crisp pixel art scaling */
    image-rendering: crisp-edges;
    display: block;
    margin: 0 auto;
    background: #000;
}
```

---

## Phase 7: Build System & CI

**Goal**: Integrate WASM builds into the project tooling.

### Step 7.1: wasm-pack build script

```bash
# Build WASM package
wasm-pack build crates/oasis-backend-wasm --target web --out-dir ../../www/pkg

# Or for npm integration:
wasm-pack build crates/oasis-backend-wasm --target bundler --out-dir ../../www/pkg
```

### Step 7.2: CI integration

Add to the CI pipeline (after existing steps):

```yaml
# WASM build check
- name: WASM build
  run: |
    rustup target add wasm32-unknown-unknown
    cargo install wasm-pack
    wasm-pack build crates/oasis-backend-wasm --target web
```

### Step 7.3: Workspace configuration

Add to root `Cargo.toml`:
```toml
[workspace.dependencies]
wasm-bindgen = "0.2"
web-sys = "0.3"
js-sys = "0.3"
wasm-bindgen-futures = "0.4"
```

Add `"crates/oasis-backend-wasm"` to workspace members.

### Step 7.4: Ensure native builds are not affected

The WASM backend is a separate crate (like SDL, UE5, PSP). The `#[cfg]` gates
added in Phase 1 use `target_arch = "wasm32"` which never activates during
native compilation. Verify:

```bash
cargo test --workspace   # All existing tests still pass
cargo clippy --workspace -- -D warnings  # No new warnings
```

---

## Phase 8: Demo VFS & Content

**Goal**: Populate the WASM build's MemoryVfs with demo content.

### Step 8.1: Embedded demo assets

The WASM build uses `MemoryVfs` (no filesystem). Populate with:

- Default skin TOML (embedded via `include_str!`)
- App manifests from `skins/` and default apps
- Demo files for the file manager
- Man pages for terminal help
- MOTD and profile

This mirrors what `vfs_setup::populate_demo_vfs()` already does for the desktop
app — reuse the same function.

### Step 8.2: Optional: Load assets from HTTP

Add a JavaScript helper that fetches skin TOML, images, etc. from a web server
and calls `oasis.add_vfs_file(path, data)` before starting the main loop.
This allows customizing the WASM build without recompilation.

---

## Phase 9: Polish & Testing

### Step 9.1: WASM-specific tests

- Unit tests for coordinate scaling
- Test that all 13 core SdiCore methods don't panic
- Test input event mapping for all key codes
- Integration test: create OasisWasm, tick 10 frames, verify no errors

### Step 9.2: Browser testing

- Test in Chrome, Firefox, Safari
- Test touch input on mobile browsers
- Test audio playback (with user interaction gate)
- Verify canvas scaling at various viewport sizes

### Step 9.3: Performance profiling

- Measure frame time at 480x272 (target: <16ms for 60fps)
- Profile texture upload overhead (ImageData creation)
- Profile text rendering (bitmap rasterizer vs canvas fillText)

---

## Implementation Order & Dependencies

```
Phase 1 (Foundation)          ← START HERE
  ├── 1.1 Create crate skeleton
  ├── 1.2 Add cfg gates to blockers
  ├── 1.3 Workspace deps
  └── 1.4 Verify compilation
          │
Phase 2 (SdiBackend)          ← Core rendering
  ├── 2.1 WasmBackend struct
  ├── 2.2 Core methods (13)
  ├── 2.3 Extended primitives
  ├── 2.4 Text rendering
  └── 2.5 Clip/translate stacks
          │
Phase 3 (InputBackend)        ← User interaction
  ├── 3.1 WasmInputBackend
  ├── 3.2 DOM event listeners
  ├── 3.3 Coordinate scaling
  └── 3.4 Event queue
          │
Phase 4 (AudioBackend)        ← Audio playback
  ├── 4.1 WasmAudioBackend
  ├── 4.2 Web Audio implementation
  └── 4.3 Autoplay handling
          │
Phase 5 (NetworkBackend)      ← Stub (no TCP in browser)
  ├── 5.1 Stub implementation
  └── 5.2 Future: Fetch API
          │
Phase 6 (JS Glue)             ← Runnable demo
  ├── 6.1 wasm-bindgen exports
  ├── 6.2 WasmPlatform
  ├── 6.3 HTML/JS harness
  └── 6.4 Canvas scaling
          │
Phase 7 (Build & CI)          ← Automation
  ├── 7.1 wasm-pack script
  ├── 7.2 CI integration
  ├── 7.3 Workspace config
  └── 7.4 Native build verification
          │
Phase 8 (Demo Content)        ← Usable product
  ├── 8.1 Embedded VFS
  └── 8.2 Optional HTTP loading
          │
Phase 9 (Polish)              ← Ship it
  ├── 9.1 WASM tests
  ├── 9.2 Browser testing
  └── 9.3 Performance profiling
```

---

## Files Modified (Existing Crates)

These are the minimal changes needed in existing crates — all gated behind
`#[cfg(not(target_arch = "wasm32"))]` so native builds are unaffected:

| File | Change |
|---|---|
| `Cargo.toml` (root) | Add workspace member + wasm deps |
| `crates/oasis-vfs/src/lib.rs` | Gate `RealVfs` module behind `not(wasm32)` |
| `crates/oasis-net/src/lib.rs` | Gate `StdNetworkBackend` behind `not(wasm32)` |
| `crates/oasis-browser/src/loader/mod.rs` | Gate HTTP/Gemini loaders behind `not(wasm32)` |
| `crates/oasis-terminal/src/commands/*.rs` | Gate filesystem-dependent commands behind `not(wasm32)` |

All other changes are **new files** in the new `oasis-backend-wasm` crate.

---

## New Files Created

| File | Purpose |
|---|---|
| `crates/oasis-backend-wasm/Cargo.toml` | Crate manifest with wasm-bindgen deps |
| `crates/oasis-backend-wasm/src/lib.rs` | `OasisWasm` struct + wasm-bindgen exports |
| `crates/oasis-backend-wasm/src/renderer.rs` | `SdiBackend` impl using Canvas 2D |
| `crates/oasis-backend-wasm/src/input.rs` | `InputBackend` impl using DOM events |
| `crates/oasis-backend-wasm/src/audio.rs` | `AudioBackend` impl using Web Audio |
| `crates/oasis-backend-wasm/src/network.rs` | `NetworkBackend` stub |
| `crates/oasis-backend-wasm/src/font.rs` | Re-export bitmap font |
| `crates/oasis-backend-wasm/src/platform.rs` | `WasmPlatform` services |
| `www/index.html` | Browser host page |
| `www/index.js` | WASM loader + animation loop |
| `www/style.css` | Canvas styling |

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Core crates have hidden `std::net`/`std::fs` usage | Build fails | Phase 1.4 verifies compilation; fix iteratively |
| Canvas 2D text rendering differs from bitmap font | Layout mismatches | Use bitmap rasterizer (Option A) as default |
| `wasm-bindgen` + `web-sys` API churn | Maintenance burden | Pin versions, use stable APIs only |
| Browser autoplay blocks audio | No audio on load | Phase 4.3 handles this with user interaction gate |
| WASM binary size too large | Slow page load | `wasm-opt -Oz`, tree-shake unused features, measure |
| `ring` (rustls dep) doesn't compile to WASM | TLS broken | Not needed for WASM build (no TCP); gate behind feature |

---

## Success Criteria

1. `wasm-pack build` succeeds with zero errors
2. OASIS_OS renders correctly in Chrome/Firefox/Safari at 60fps
3. All 5 UI modes work (Dashboard, Terminal, App, OSK, Desktop)
4. Keyboard and mouse input work correctly
5. Audio playback works after user interaction
6. Existing native `cargo test --workspace` passes unchanged
7. Existing `cargo clippy --workspace -- -D warnings` passes unchanged
8. WASM binary is <2MB gzipped
