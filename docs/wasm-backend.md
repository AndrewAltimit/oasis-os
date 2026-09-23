# WASM Backend

`crates/oasis-backend-wasm` runs the OASIS_OS shell in a web browser. It
renders with the Canvas 2D API, maps DOM keyboard / mouse / touch events to
`InputEvent`s, plays audio through Web Audio, and delegates real web pages
and YouTube playback to a native `<iframe>` overlaid on the canvas. The live
demo on GitHub Pages is built from this crate on every push to `main`.

## Building

Prerequisites:

- Rust with the `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/)
- No C toolchain is required: the crate builds `oasis-core` with
  `default-features = false`, so the QuickJS `javascript` feature (whose C
  sources would need clang to cross-compile to wasm) is left out.

```bash
./scripts/build-wasm.sh            # debug build  (wasm-pack --dev)
./scripts/build-wasm.sh --release  # release build (smaller + faster)
```

The script runs `wasm-pack build crates/oasis-backend-wasm --target web` and
writes the JS glue + `.wasm` to `pkg/` at the repo root. `wasm-opt` is
disabled in the crate's `[package.metadata.wasm-pack]` profiles (Rust LTO does
the optimization, and it avoids wasm-opt version mismatches with bulk-memory
instructions).

To type-check without wasm-pack:

```bash
cargo check -p oasis-backend-wasm --target wasm32-unknown-unknown
```

The input/key-mapping logic is pure Rust and its unit tests run on the host:
`cargo test -p oasis-backend-wasm`.

### Cargo features

| Feature | Default | Effect |
|---------|---------|--------|
| `wasm-youtube` | Yes | Adds a **Video Embed** dashboard app with YouTube search (via Invidious) and playback in the iframe overlay; forwards `oasis-core/wasm-youtube` |

Build without it via
`wasm-pack build crates/oasis-backend-wasm --target web --no-default-features`.

## Serving

`www/` holds the page shell and imports the module from `../pkg/`, so serve the
**repo root** (not `www/`):

```bash
# Plain static server
python3 -m http.server 8080          # then open http://localhost:8080/www/

# Dev server (recommended)
node scripts/serve-wasm.mjs          # http://localhost:8080/www/
node scripts/serve-wasm.mjs --port 3000
```

`scripts/serve-wasm.mjs` serves `www/` and `pkg/` with correct MIME types
(including `application/wasm`), redirects `/` to `/www/`, and exposes a
`/cors-proxy?url=<remote>` route that fetches a remote URL (following up to
5 redirects) and re-serves it with `Access-Control-Allow-Origin: *`. The
current Rust code does not route requests through the proxy automatically; it
is there for manual testing of remote media that lacks CORS headers.

### Page files

| File | Role |
|------|------|
| `www/index.html` | Page shell: `<canvas id="oasis">`, a skin `<select>` that reloads with `?skin=<name>`, a controls hint |
| `www/index.js` | Loads `pkg/oasis_backend_wasm.js`, constructs `OasisWasm`, drives `tick()` from `requestAnimationFrame`, letterbox-fits the canvas to the window |
| `www/style.css` | Layout / demo bar styling |

URL parameters read by `index.js`:

- `?skin=<name>` -- any **built-in** skin name (the browser has no filesystem,
  so directory skins are unavailable; unknown names fall back to `classic`).
  Without the parameter the constructor defaults to `xp`.
- `?app=<title>` -- launch a dashboard app on boot, e.g. `?app=TV+Guide`.

## JavaScript API

`OasisWasm` is exported through `wasm-bindgen`:

| Method | Description |
|--------|-------------|
| `new OasisWasm(canvasId, skinName?)` | Attach to a `<canvas>`; resizes its buffer to the skin's declared resolution (e.g. classic 480x272, xp 1024x768) |
| `tick(dt)` | Poll input, update, and render one frame |
| `send_command(cmd)` | Run a terminal command; returns its text output (also echoed to the in-canvas terminal) |
| `read_pixels()` | RGBA framebuffer (`width * height * 4` bytes), e.g. for Playwright screenshots |
| `add_vfs_file(path, bytes)` | Write a file into the in-memory VFS |
| `screen_width()` / `screen_height()` | Current virtual resolution |
| `launch_app(title)` | Launch a dashboard app by title (`"Browser"`, `"TV Guide"`, ...) |

`index.js` also exposes the instance as `window.__oasis` and sets
`window.__oasisReady = true` for automated tests.

## Input mapping

`WasmInputBackend` (`src/input.rs`) listens for keyboard events on `window`
and pointer events on the canvas. Pointer coordinates are converted from CSS
pixels to virtual-screen pixels accounting for the letterboxed canvas and
device scale.

| DOM event | `InputEvent` |
|-----------|--------------|
| `mousemove` / `touchmove` | `CursorMove { x, y }` |
| `mousedown` / `touchstart` | `PointerClick { x, y }` |
| `mouseup` / `touchend` | `PointerRelease { x, y }` |
| `wheel` | `MouseWheel { delta }` |
| window `focus` / `blur` | `FocusGained` / `FocusLost` |
| `keydown` | `Key { key, mods }`, then its gamepad-style twin (if any), then `TextInput(ch)` |
| `keyup` | the gamepad-style release twin (if any) |

### Keyboard

Every `keydown` that maps to a [`Key`](../crates/oasis-types/src/input.rs)
first produces a raw `InputEvent::Key { key, mods }` (the same contract as the
SDL backend and the FFI's `OASIS_EVENT_KEY`):

- `KeyboardEvent.key` is mapped to `Key::Up/Down/Left/Right`, `Enter`,
  `Escape`, `Tab`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`,
  `PageDown`, `Space`, and `F(1..=12)`.
- Printable keys become `Key::Char(c)`, lowercased. Digit keys use
  `KeyboardEvent.code` (`Digit1`), so Shift+1 still reports `Key::Char('1')`.
- Modifier keys alone (Shift, Control, ...) and unknown named keys produce no
  `Key` event. `mods` carries Shift / Ctrl / Alt / Meta (as `SUPER`).

The gamepad-style twin comes from `Key::legacy_press`:

| Key | Twin |
|-----|------|
| Arrows | `ButtonPress(Up/Down/Left/Right)` |
| Enter / Escape | `ButtonPress(Confirm)` / `ButtonPress(Cancel)` |
| Space | `ButtonPress(Triangle)` |
| F1 / F2 | `ButtonPress(Start)` (terminal toggle) / `ButtonPress(Select)` (on-screen keyboard) |
| F11 | `ToggleFullscreen` |
| Backspace | `Backspace` |
| Tab / Shift+Tab | `Tab` / `ShiftTab` |
| Q / E | `TriggerPress(Left)` / `TriggerPress(Right)` |

Finally, a single-character `key` typed without Ctrl/Alt/Meta also produces
`TextInput(ch)` (with its original case).

Dispatch (`src/input_dispatch.rs`) sends each `Key` event to the focused app's
`handle_key` first. If the app consumes it -- or the key merely typed text into
a text-entry target (terminal, browser URL bar / form field, or an app that
accepts text) -- a `KeyTwinFilter` drops the gamepad-style twin, so typing `q`
in the terminal does not also fire the left trigger.

`preventDefault()` is called for keys with a gamepad twin, for
Delete/Home/End/PageUp/PageDown (page scrolling), and for Ctrl/Cmd+letter
shortcuts **except Ctrl+R**, so page reload keeps working. F5 and other
browser keys are left alone.

## Iframe overlay

The built-in browser engine cannot make synchronous network requests from a
browser tab, so in the WASM build the Browser app runs in *iframe HTTP mode*
(`BrowserConfig.features.iframe_http_mode`, home page
`https://www.google.com/webhp?igu=1`):

- VFS pages (`vfs://`, local HTML) are rendered by the OASIS engine on the
  canvas as usual.
- `http(s)://` pages are loaded into a single `<iframe>` (`src/iframe.rs`)
  positioned over the browser window's content area (below the URL bar, above
  the status bar). OASIS still paints the window chrome.
- The iframe's CSS rect is recomputed every frame from the window's content
  rect, the canvas letterbox offset and scale, so dragging and resizing the
  window keeps it glued in place.

Limits and behavior:

- **Many sites refuse to be framed** (`X-Frame-Options` / CSP
  `frame-ancestors`); they show the browser's own "refused to connect" page.
  Google's `igu=1` home page is used because it allows framing.
- The iframe is sandboxed: `allow-scripts allow-same-origin allow-forms
  allow-popups`.
- It sits at a fixed `z-index` above the canvas, so it cannot be composited
  beneath other OASIS windows. It is only shown while its window is the
  topmost (active) window; otherwise it is *soft-hidden* (`display: none`
  with the document kept alive, so scroll position, form state, and playback
  survive). Closing the window or navigating to a VFS page fully hides it and
  resets `src` to `about:blank`.
- Only one iframe exists, shared between the Browser and Video Embed windows.
- While the pointer is over the iframe the canvas receives no mouse events,
  so the OASIS software cursor is hidden and the native cursor takes over.
- Page content inside the iframe is invisible to `read_pixels()` and to the
  OASIS input pipeline.

## YouTube (`wasm-youtube`)

The Video Embed app (`oasis-core/src/apps/video_embed.rs`) talks to the
backend over the VFS: it writes `search:<query>`, `play:<video id>`, or `stop`
to `/tmp/video_embed_request`, and the backend publishes a JSON result blob to
`/tmp/video_embed_results`.

- **Search** (`src/youtube.rs`) queries a short list of public
  [Invidious](https://invidious.io/) instances in order (6 s timeout each)
  via `fetch`; the first 2xx response wins. Up to 18 hits are returned, and
  each 320x180 `mqdefault` thumbnail is drawn onto an offscreen `<canvas>`
  registered as a texture for the thumbnail grid. Textures from the previous
  search are freed when a new one starts. Public Invidious instances come and
  go; if all of them fail the app shows an error.
- **Playback** switches the iframe to YouTube mode (adds
  `allow-presentation allow-popups-to-escape-sandbox` and
  `allow="autoplay; encrypted-media"`) and loads the embed URL inside the
  Video Embed window's content area, tracking drag/resize like the browser.
  Minimizing the window soft-hides the player; closing it or `stop` hides it.

## Other subsystems

- **Rendering** (`src/renderer.rs`, `shapes.rs`, `gradients.rs`,
  `textures.rs`, `batch.rs`): Canvas 2D implementation of all `SdiBackend`
  traits. Text uses the shared `oasis-rasterize` glyph cache. Shader
  wallpapers are rendered by the software renderer in `oasis-shader` and
  blitted to the canvas (`src/shader_bridge.rs`).
- **Audio** (`src/audio.rs`): Web Audio `AudioBuffer` playback for static
  tracks; streaming (Internet Radio) via an `<audio>` element, using Media
  Source Extensions where needed. A circuit breaker stops radio auto-advance
  after repeated failures.
- **TV Guide video** (`src/video.rs`): plays direct MP4 URLs in a hidden
  `<video>` element and copies frames to an offscreen canvas texture each
  tick. Requires browser H.264 support (Chrome/Chromium have it built in; the
  Firefox snap on Linux may need system ffmpeg).
- **Networking** (`src/network.rs`): browsers have no raw TCP, so the
  `NetworkBackend` returns errors; remote terminal / FTP / MCP are unavailable.
  HTTP goes through `fetch` (catalogs, search) or the iframe.
- **VFS**: an in-memory `MemoryVfs` seeded with demo content
  (`src/vfs_content.rs`); nothing persists across reloads.
- **JavaScript in pages**: the `javascript` feature is off, so the OASIS
  browser engine does not execute `<script>` on VFS pages (framed pages run
  their scripts natively in the iframe).

## Deployment

The `deploy-pages` job in `.github/workflows/main-ci.yml` runs
`./scripts/build-wasm.sh --release` on pushes to `main` and publishes `pkg/`
together with the `site/` directory (the demo page lives under `site/demo/`,
with `www/index.js` copied next to it).
