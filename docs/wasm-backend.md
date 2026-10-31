# WASM Backend

`crates/oasis-backend-wasm` runs the OASIS_OS shell in a web browser. It
renders with the Canvas 2D API, maps DOM keyboard / mouse / touch events to
`InputEvent`s, plays audio through HTML `<audio>` elements, and delegates real web pages
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

### Tests

There is no headless-browser (`wasm-bindgen-test`) job: the CI image has no
browser or WebDriver. Instead the backend's browser-independent logic is
factored out and tested on the host with `cargo test -p oasis-backend-wasm`
(part of `cargo test --workspace`, so CI runs it):

- **Clip state machine** (`src/clip.rs`): Canvas 2D clips only intersect, so
  each applied clip must be exactly one `save()` level. `SurfaceClip` (set /
  reset / push / pop / clear / render-target bind + unbind) is written against
  a three-method `ClipCanvas` trait and tested against a model of Canvas 2D
  `save` / `restore` / `clip` semantics, including long pseudo-random op
  sequences checked against `ClipStack`.
- **Input and key mapping** (`src/input.rs`), letterbox point scaling, audio
  MIME sniffing, the demo VFS contents.
- **Site skin lists** (`tests/site_skins.rs`, see [Skin list](#skin-list)).

The rendering itself is checked by loading the release build in a browser
(see [Previewing the Pages layout](#previewing-the-pages-layout)).

### Cargo features

| Feature | Default | Effect |
|---------|---------|--------|
| `wasm-youtube` | Yes | Adds a **Video Embed** dashboard app with YouTube search (via Invidious) and playback in the iframe overlay; forwards `oasis-apps/video-embed` |

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
| `www/index.html` | Local page shell: `<canvas id="oasis">` inside `#container` (the positioned parent the iframe overlay attaches to), a skin `<select>` grouped like the landing page that reloads with `?skin=<name>`, a controls card. Styles and chrome script come from `site/css/{site,demo}.css` and `site/js/demo.js`, shared with the deployed `site/demo/index.html` |
| `www/index.js` | Loads `pkg/oasis_backend_wasm.js`, constructs `OasisWasm`, drives `tick()` from `requestAnimationFrame`, letterbox-fits the canvas to `#container`; sets `window.__oasisReady` once booted. The Pages deploy copies it to `demo/index.js` |

**Canvas fit.** The canvas buffer is the skin's resolution; `index.js` sizes
its CSS box to fit `#container` with the buffer's aspect ratio (the container
centers it, so the rest is letterbox). The scale is a whole number when 2x or
more fits (crisp pixels for the 480x272 skins) and fractional below that,
rounded down to whole device pixels so the canvas never overflows the
container (which would clip its right / bottom edge). A `ResizeObserver` on
`#container` re-fits when the demo bar wraps or the web font loads, not only
on window resize.

### Skin list

The skin picker in `www/index.html` and `site/demo/index.html` and the
landing-page gallery (`site/index.html`) are generated from one file,
`site/data/skins.json` (display name, gallery category, blurb; resolutions
come from `skins/<id>/skin.toml`). The pages keep the generated HTML between
`<!-- BEGIN skins:<region> -->` / `<!-- END skins:<region> -->` markers, so
they stay static files with no build step. After editing `skins.json` (or
adding a skin), run:

```bash
./scripts/gen-skin-list.sh           # rewrite the generated regions
./scripts/gen-skin-list.sh --check   # check only
```

The generator is the `site_skins` integration test of this crate, so
`cargo test --workspace` fails in CI when a page is stale, when `skins.json`
misses or invents a skin under `skins/`, or when a gallery screenshot no
longer matches its skin's resolution.

### Previewing the Pages layout

The deployed site differs from `www/`: `site/` is the root, `pkg/` sits
beside it and `www/index.js` becomes `demo/index.js`, all under the
`/oasis-os/` sub-path. To check the real demo page before a deploy:

```bash
./scripts/build-wasm.sh --release
mkdir -p /tmp/pages/oasis-os/demo
cp -r site/. screenshots pkg /tmp/pages/oasis-os/
cp www/index.js /tmp/pages/oasis-os/demo/index.js
(cd /tmp/pages && python3 -m http.server 8080)
# open http://localhost:8080/oasis-os/demo/?skin=classic
```

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
| `launch_app(title)` | Launch an app by title, case-insensitive: any registered app (with or without a dashboard tile), `"Browser"`, or a VFS-discovered tile |

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
| `mouseup` (on `window`, after a press on the canvas, so a drag released outside the canvas still ends) / `touchend` | `PointerRelease { x, y }` |
| `wheel` | `MouseWheel { delta }`: the sign of the vertical motion (a purely sideways scroll emits nothing) |
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

## Windows and apps

The WASM host runs the same `oasis-apps` bundle and window manager as the
desktop shell, with the same close semantics:

- The titlebar close button and Cancel (Esc) on a non-app window **ask the
  app first** (`AppRunner::request_close`): the Text Editor with unsaved
  changes answers with its save prompt and the window stays open and focused.
- In an app window, Cancel goes to the app (backs out of dialogs, leaves
  viewers, raises the unsaved-changes prompt); the window closes when the app
  answers `Exit`. In the browser, Cancel first ends typing in the URL bar or a
  form field.
- Closing the Internet Radio or Music Player window stops its audio.
- Requests an app queued in the frame it closed (or was replaced by a
  File Manager hand-off) are still carried out, ahead of the open apps'
  (`src/app_requests.rs`); TV tunes from a closed guide are dropped.
  `/var/audio/request` and `/var/wm/request` are line queues: the host
  appends to them instead of overwriting earlier lines.
- The `wm` and `open` terminal commands work as on the desktop: the host
  applies every queued `WmRequest` (`close [--force]`, `focus`, `minimize`,
  `maximize`, `open <app>[\t<file>]`) each frame and publishes the window
  list to `/var/wm/status`.

### Skin images

Skin image assets render as on the desktop (`src/skin_assets.rs`): the
`style = "image"` wallpaper is composited from the skin's PNG
(`wallpaper::generate_with_assets`), layout `texture =` objects get their
textures, and `[[background_layers]] kind = "image"` decals (the PSIX HiFi
wordmark) become drifting / pulsing SDI objects. All three are rebuilt on a
skin swap, which also hides the outgoing skin's dashboard icons, taskbar and
start button. Not yet ported: themed software cursors, top-tab pill textures
and WM nine-patch chrome.

### Music

The Music Player and the `music` terminal command talk to the host through
`/var/audio/request` (`play_file <path>`, `play`, `pause`, `resume`, `stop`,
`next`, `prev`, `vol`, `repeat`, `shuffle`). The WASM host serves them with
the same `oasis_audio::MusicController` as the desktop shell (`src/media.rs`),
publishing `/var/audio/position` (progress bar) and `/var/audio/status`
(`music status`). Starting a track stops the radio, and tuning the radio
stops the track. The demo VFS ships one sample track,
`/home/user/music/ambient_dawn.mp3` (shared with the desktop's demo VFS).
Uploaded files (`add_vfs_file`) under `/home/user/music` play too.

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

The Video Embed app (`oasis-apps/src/video_embed.rs`, feature `video-embed`) talks to the
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
- **Audio** (`src/audio.rs`): in-memory tracks (`load_track`, music) are
  wrapped in a typed `Blob` and played by an `<audio>` element, so the
  browser decodes MP3 / Ogg / WAV / FLAC natively and reports real position
  and duration (Web Audio's `decodeAudioData` is async, which the
  synchronous `AudioBackend` API cannot wait for). Streaming (Internet Radio)
  uses an `<audio>` element pointed at the URL, or Media Source Extensions
  where needed. A circuit breaker stops radio auto-advance after repeated
  failures. Playback starts from the frame loop, so browsers allow it only
  after the user has interacted with the page (any click or key press).
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
with `www/index.js` copied next to it; see
[Previewing the Pages layout](#previewing-the-pages-layout) to reproduce it
locally).
