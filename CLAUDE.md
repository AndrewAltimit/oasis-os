# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

OASIS_OS is an embeddable operating system framework in Rust (edition 2024). It provides a skinnable shell with a scene-graph UI, command interpreter, virtual file system, browser engine (HTML/CSS/Gemini), plugin system, and remote terminal. It renders to any pixel buffer + input stream. Built from scratch in Rust starting early 2026, inspired by PSP homebrew shells like PSIX. Seventeen built-in skins are implemented, all of them TOML directories in `skins/` (the single source of truth; `oasis-skin/build.rs` embeds them).

Default virtual resolution is 480x272 (PSP native). Skins may override this (e.g. desktop=800x600, xp=1024x768); the backend canvas/window scales to match.

## Build Commands

All CI commands run inside Docker containers. For local development you can run cargo directly (SDL3 is compiled from source automatically via the `build-from-source` feature), or use the Docker wrapper.

```bash
# Build (desktop). Default features need system ffmpeg dev libs (video-decode-ffmpeg);
# without them (e.g. local Windows) use the openh264 fallback (drop `javascript`
# to build without QuickJS):
cargo build --release -p oasis-app
cargo build -p oasis-app --no-default-features --features javascript,video-decode

# Build via Docker (matches CI exactly)
docker compose --profile ci run --rm rust-ci cargo build --workspace --release

# Run tests
cargo test --workspace

# Run a single test
cargo test --workspace -- test_name

# Run tests in a specific crate
cargo test -p oasis-core

# Format check
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all

# Lint (CI treats warnings as errors)
cargo clippy --workspace --all-targets -- -D warnings

# Lint exactly as CI does — ALWAYS run this before opening a PR. The CI image
# pins a specific Rust version (rust:1.99-slim), so a locally newer/older
# toolchain can pass clippy while CI fails (lints change between releases).
docker compose --profile ci run --rm rust-ci cargo clippy --workspace --all-targets -- -D warnings

# License/advisory audit
cargo deny check

# Build PSP backend (excluded from workspace, requires nightly + cargo-psp)
cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release

# ... or in Docker with pspdev + nightly + cargo-psp preinstalled (any host)
docker compose --profile psp-build run --rm psp-build \
  bash -c 'cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release'
# Boot-test the PSP EBOOTs in PPSSPPHeadless (as CI does)
scripts/psp-smoke-test.sh

# Build PSP overlay plugin PRX (excluded from workspace, kernel mode)
cd crates/oasis-plugin-psp && RUST_PSP_BUILD_STD=1 cargo psp --release
# ... then strip it (CI ships this file, size-gated at 192 KiB)
prxmin target/mipsel-sony-psp-std/release/oasis-plugin-psp.prx oasis-overlay.prx

# Build UE5 FFI shared library
cargo build --profile release-ffi -p oasis-ffi

# Build WASM backend (requires wasm-pack)
./scripts/build-wasm.sh          # debug build
./scripts/build-wasm.sh --release # release (smaller + faster)
# Serve: python3 -m http.server 8080 → http://localhost:8080/www/

# Take screenshots
cargo run -p oasis-app --bin oasis-screenshot
```

## CI Pipeline Order

This section is the canonical description of CI; `AGENTS.md` and `docs/design.md` link here.

`main-ci.yml` (push to `main`): format check -> clippy (`--workspace --all-targets -D warnings`) -> feature-set checks (`scripts/ci-feature-clippy.sh`, shared with PR validation: the `oasis-app` feature-combination check `--no-default-features --features javascript[,mcp]`, then clippy over the non-default feature sets of `oasis-browser`, `oasis-core`, `oasis-video`, `oasis-app`, `oasis-ffi`, `oasis-shader`, `oasis-backend-wasm`; `--list` prints them) -> nightly clippy (advisory) -> doc build (`RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`) -> markdown link check (`md-link-checker --internal-only` over `*.md`, `docs/`, `scripts/psp-scenarios.md`) -> test (cargo-nextest + doc-tests) -> release build -> screenshot regression -> cargo-deny -> benchmarks (manual `run_benchmarks` dispatch only) -> code coverage (advisory) -> rust-psp pin check (`scripts/check-rust-psp-rev.sh`: crates, workflows, `docker/psp-build.Dockerfile`) -> PSP tooling unit tests (`python3 -m unittest` over `scripts/test_*.py`) -> PSP EBOOT / overlay PRX / recovery EBOOT builds, then the seven other standalone PSP crates (ME boot PRX, devloop, PRX decryptor, USB tools) in one loop (rustc warnings are errors via `PSP_DENY_WARNINGS`) -> PPSSPP headless smoke test (`scripts/psp-smoke-test.sh`); then release binaries and GitHub Pages deploy (WASM). On a `v*` tag, `create-release` first runs `scripts/check-release-version.py`: the tag must equal the `[workspace.package]` version and the shipped standalone PSP crates' versions, so bump them in a commit before tagging (v1.5.0 was once tagged on 1.4.0).

`pr-validation.yml` (pull requests) runs a subset -- format, clippy, the same feature-set checks script, nightly clippy (advisory), doc build, link check, test, release build, screenshot generation, cargo-deny, pin check, PSP tooling unit tests, PSP EBOOT build and smoke test, and on a manual `run_benchmarks` dispatch a benchmark regression gate -- without coverage or the PRX / recovery builds; plus Claude security/quality reviews and the auto-fix agents (see [`AGENTS.md`](AGENTS.md)).

The Rust steps run via `docker compose --profile ci run --rm rust-ci`; the PSP builds use cargo-psp installed on the runner by `.github/actions/install-cargo-psp` (locally, use the `psp-build` Docker profile below).

**Memory analysis** (`memory-ci.yml`, separate workflow):
- **ASAN** (AddressSanitizer) -- catches use-after-free, buffer overflow, leaks (~2x slowdown)
- **Valgrind massif** -- heap profiling of video decode (`video-memprofile`), plus a native peak-RSS gate (`--max-rss-kb`, run outside valgrind, whose own footprint would count) and a multi-loop leak test

Both jobs are non-blocking on pull requests (`continue-on-error` there) and run on pushes to main, PRs touching `oasis-video`, `oasis-core`, `oasis-app`, `oasis-backend-sdl`, `oasis-backend-ue5`, `oasis-types`, `Cargo.toml`/`Cargo.lock` or the workflow itself, and `workflow_dispatch`.

**Scheduled:** `nightly-streaming.yml` (end-to-end TV Guide video streaming) and `fuzz.yml` (fuzz targets in `crates/oasis-core/fuzz/`).

## Architecture

### Crate Dependency Graph

```
oasis-types     (foundation: Color, Button, InputEvent, backend traits, error types, geometry)
├── oasis-vfs        (virtual file system: MemoryVfs, RealVfs, GameAssetVfs)
├── oasis-platform   (platform service traits: Power, Time, USB, Network, OSK)
├── oasis-sdi        (scene display interface: named object registry, z-order)
├── oasis-net        (TCP networking, PSK auth, remote terminal; FTP server behind feature `vfs`)
├── oasis-mcp        (optional MCP control server: Streamable HTTP + JSON-RPC, agent tool dispatch)
├── oasis-audio      (music playlist + playback state behind /var/audio IPC, internet radio, SFX, WAV/Ogg)
├── oasis-ui         (27 Widget impls + ListView/MenuBar/NinePatch/focus/flex+grid layout)
├── oasis-wm         (window manager: drag/resize, hit testing, decorations, snapping, tiling, animations)
├── oasis-skin       (TOML skin engine, 17 built-in skins, theme derivation)
├── oasis-terminal   (75 commands + 21 shell builtins, line editing, completion, jobs, scripting)
├── oasis-html       (HTML tokenizer, tree builder, arena DOM; leaf of the browser engine)
├── oasis-css        (CSS parser, cascade, computed values; depends on oasis-html)
├── oasis-http       (browser resource loader: Url, HTTP/1.1 + HTTP/2, Gemini framing, cookies, cache, CSP, IO thread; feature psp)
├── oasis-browser    (HTML/CSS/Gemini on oasis-html + oasis-css + oasis-http (re-exported as `loader`): layout engine, @font-face web fonts (fontdue TTF/OTF rasterizer, font registry, glyph texture cache), full 2D CSS transforms + 3D transform functions (rotateX/Y/Z, translate3d, scale3d, rotate3d, matrix3d, perspective()) with screen-space perspective projection from ancestor `perspective:`, transform-style: preserve-3d propagation, transform-origin Z, perspective-origin, and backface-visibility culling, Canvas 2D path API, SVG paths/groups, light compositor, z-index stacking contexts, nested scroll containers, form elements with select dropdown + label association, hover-triggered CSS transitions, soft hyphens, bidi text, JS DOM bindings)
├── oasis-js         (JavaScript engine: QuickJS-NG via rquickjs; desktop with `javascript` feature + PSP)
├── oasis-video      (MP4/H.264+AAC decode; additive features: demux-lite, symphonia, h264, ffmpeg)
├── oasis-vector     (vector graphics: scene graph, path ops, icons, frame-driven animations)
├── oasis-shader     (animated shader wallpapers: Shadertoy-style fragment shaders)
├── oasis-rasterize  (software rasterizer for CPU-side rendering)
├── oasis-i18n       (internationalization support)
├── oasis-test-backend (mock/recording backends for testing)
├── oasis-app-core   (shared app framework: `App` trait, `AppAction`, render helpers)
├── oasis-app-games  (Games app)
├── oasis-app-paint  (Paint app)
├── oasis-app-text-editor (Text Editor app)
├── oasis-app-calculator  (Calculator app)
├── oasis-app-media       (Music Player + Photo Viewer apps)
├── oasis-app-tv-guide    (TV Guide app)
├── oasis-app-radio       (Internet Radio app)
├── oasis-app-settings    (Settings app)
├── oasis-app-file-manager (File Manager app)
├── oasis-app-system-monitor  (System Monitor + Network + Package Manager apps)
├── oasis-backend-sdl  (SDL3 desktop/Pi rendering + input + audio; needs only oasis-types/-net/-audio/-rasterize/-shader, not oasis-core, so it never pulls in QuickJS)
├── oasis-backend-ue5  (software RGBA framebuffer for Unreal Engine 5; needs only oasis-types/-rasterize/-audio)
└── oasis-core       (coordination only: dashboard, agent, plugin, script, terminal, AppRunner + AppRegistry; depends on oasis-app-core but on no app crate)
    ├── oasis-apps     (+ every oasis-app-* crate: `builtin_registry(&HostInfo)`, `tv` command, `TvGuideRunnerExt`; feature video-embed)
    ├── oasis-app      (+ oasis-apps, oasis-backend-sdl, oasis-backend-ue5; lib `oasis_app::Shell` + headless e2e harness; binaries oasis-app, oasis-screenshot, screenshot-tests (CI regression), e2e-tests (TV Guide pipeline); features javascript, video-decode-ffmpeg (default) / video-decode (openh264), skin-dev, mcp)
    ├── oasis-backend-wasm (+ oasis-apps; Canvas 2D + DOM input + `<audio>` playback; iframe overlay drives a Browser pane and a YouTube embed; YouTube search + thumbnail grid via Invidious; feature: wasm-youtube)
    ├── oasis-ffi      (+ oasis-backend-ue5; cdylib C-ABI for UE5 integration; features video-decode / video-decode-ffmpeg)
    ├── oasis-backend-psp  (in the workspace `exclude` list, PSP hardware; + oasis-app-tv-guide, oasis-browser[psp,javascript], oasis-js, oasis-shader, oasis-video[demux-lite])
    └── oasis-plugin-psp   (standalone, not in the workspace; kernel-mode PRX overlay)
oasis-usb-host  (standalone binary, rusb only: host side of the PSP USB thin client)
```

### PSP (target-specific architecture)

PSP has meaningful enough constraints that the detail lives in
[`docs/psp-architecture.md`](docs/psp-architecture.md). Summary:

- **Two binaries** — `oasis-backend-psp` (EBOOT.PBP, the shell) and
  `oasis-plugin-psp` (PRX overlay loaded by CFW, ~153 KB stripped with
  `prxmin`, ~212 KB resident; no `oasis-core` dependency). See [ADR 004](docs/adr/004-psp-two-binary-architecture.md).
- **GU command buffer (1 MB)** overflows with dense pages. Don't
  double-start GU frames after `swap_buffers_inner()`.
- **QuickJS-NG** built through pspdev's `psp-gcc` with `-msingle-float`,
  linked via `psp-ld`, libc provided by
  `oasis-backend-psp/src/quickjs_shim.rs` — see
  [`docs/javascript-engine.md`](docs/javascript-engine.md).
- **TLS 1.3** via `embedded-tls` + `UnsecureProvider` because PSP
  firmware ships 2008-vintage CAs.
- **Video streaming** via ME hardware decode (`sceMpeg` NAL direct
  path) + `sceAudiocodec`, ≤480p only. See
  [`docs/video-streaming.md`](docs/video-streaming.md).

### Desktop Video Streaming

Progressive streaming via `StreamingBuffer` (in oasis-app,
`src/tv_controller/streaming_buffer.rs`; a sliding window over an
`Arc<StreamingInner>` while oasis-video's decoder reads the same
buffer via `Read + Seek`). Probe-mode reads return zeros, deferred tail
probe for moov-at-end files, CDN failover through the archive.org
URL for fresh 302 redirects. Full details in
[`docs/video-streaming.md`](docs/video-streaming.md).

### Key Abstraction: Backend Traits

`oasis-types/src/backend/` defines the only abstraction boundary between core and platform (re-exported by `oasis-core`):
- `SdiCore` -- required rendering (12 required methods: init, clear, blit, fill_rect, draw_text, swap_buffers, load_texture, destroy_texture, set_clip_rect, reset_clip_rect, read_pixels, shutdown; plus the provided `measure_text` (bitmap font; SDL overrides it for skin TTFs) and `viewport_size`, the bound surface's size)
- `SdiBackend` -- a marker super-trait with no methods of its own. A blanket impl satisfies it for any type that implements `SdiCore` plus all eight extension traits: `SdiShapes`, `SdiGradients`, `SdiText`, `SdiTextures`, `SdiClipTransform`, `SdiVector`, `SdiBatch`, `SdiRenderTarget`. The extensions live under `oasis-types/src/backend/extensions/` (one file per trait, re-exported via `extensions/mod.rs`) and total 45 default-impl methods (shapes 8, gradients 2, text 7, textures 2, clip/transform 9, vector 6, batch 2, render targets 9). Mocks and null backends get all eight with one `oasis_types::impl_sdi_defaults!(T)` line (`impl_sdi_defaults!(T; except SdiShapes, ..)` skips the ones a type writes itself). The shape defaults emit `fill_rect` spans from the shared generators in `oasis_types::rasterize` (incl. `triangle_rows`, `stroke_rect_parts`), so they cover exactly the pixels SDL/UE5 draw (PSP and every mock rely on them). `SdiBatch` (on `SdiText`) is `submit_rect_batch`/`submit_text_batch` for batched rect and text geometry submission, drawing exactly what the items one by one (`fill_rect`, `draw_text_styled`) would (backends can override with GPU geometry calls; `conformance::batch_matches_singles` pins it). `ClipStack::push` returns the clip to apply (`ClipRect::EMPTY` when nested clips miss), and unimplemented optional operations report `BackendError::Unsupported` (`backend::unsupported`). `current_clip_rect` returns `Option<ClipRect>`.
- `InputBackend` -- input polling (returns `Vec<InputEvent>`)
- `NetworkBackend` -- TCP networking
- `AudioBackend` -- audio playback
- `ClipboardBackend` -- copy/paste (`InMemoryClipboard` fallback)

Core code never calls platform APIs directly. All platform interaction goes through these traits.

Every backend follows one drawing-state contract (rustdoc on `SdiCore`, `SdiClipTransform`, `SdiRenderTarget`; rationale in [ADR-003](docs/adr/003-backend-trait-design.md)): `set_clip_rect` is absolute (untranslated) and replaces the clip, `push_clip_rect` is translated and intersects; `clear` ignores the clip and does not blend; a bound render target starts with no clip/translate and the unbind restores the parent's (`stacks::DrawState`); composites scale the target to the destination rect; binding, compositing, reading or destroying a target that is on the bind chain is an error. `oasis_test_backend::conformance` pins it on SDL and UE5 (`cargo test -p oasis-backend-sdl --test backend_parity`).

### Core Modules

The framework lives in 51 crate directories under `crates/`:
- **41 workspace members** (the desktop / WASM / UE5 build surface — what `cargo build --workspace` compiles)
- **1 explicitly excluded crate** (`oasis-backend-psp`, on the `mipsel-sony-psp` target)
- **9 standalone PSP-target crates** that are not mentioned in the workspace `Cargo.toml` at all and are built individually via `cargo psp`: `oasis-plugin-psp`, `oasis-devloop-psp`, `oasis-me-boot`, `oasis-prx-decrypt-psp`, `oasis-recovery-psp`, `oasis-usb-client-psp`, `oasis-usb-debug-psp`, `oasis-usb-trace-psp`, `oasis-usb-vbus-psp`

Each module below is its own crate (previously all in oasis-core):

- **oasis-types** -- Foundation types: `Color`, `Button`, `InputEvent` (incl. raw `Key` + `Modifiers` events, `Key::legacy_press` gamepad-twin table, `KeyTwinFilter`), backend traits (`SdiCore`, `SdiBackend`, `InputBackend`, `NetworkBackend`, `AudioBackend`), VFS IPC paths + typed requests (`ipc.rs`: `/var/audio`, `/var/radio`, `/var/wm`, `/var/tv`, the Settings `/system/ipc` + `/system/state` paths, the terminal UI request files, `/system/state/net`; `MediaRequest`, `WmRequest`, `PluginRequest`, `BrowserRequest`, `Notification`, `NetState`, `parse_resolution`), error types (`BackendError::{TextureNotFound, Unsupported, RenderTargetNotFound}`, `SdiError::NotFound`, `OasisError::is_would_block()` for non-blocking I/O, ...), TLS, bitmap font glyphs + metrics (`bitmap_font`, incl. `bitmap_line_height`/`bitmap_ascent`), `geometry.rs` (`ClipRect`, `isqrt_i32`, `rounded_rect_inset`), `rasterize` (scanline span generators for circles, rings, rounded rects, thick lines and even-odd polygons, plus `PixelSink`/`rasterize_triangle`), `easing.rs` (easing curves; re-exported as `oasis_ui`/`oasis_wm` `animation::easing`), `util.rs` (`ct_eq` constant-time compare, `base64`)
- **oasis-sdi** -- Scene Display Interface: named objects with position, size, color, texture, text, z-order, gradients, rounded corners, shadows
- **oasis-skin** -- Data-driven TOML skin system with 17 built-in skins, each generated at build time from its `skins/<name>/` directory (no skin is defined in Rust). Strict `resolve_skin` (unknown = error) plus `resolve_skin_or_classic` for boot paths; `skin lint` validation (malformed colors, unknown shader / layer / easing / enum names, assets, contrast) that every built-in passes (`all_builtin_skins_lint_clean`; contrast exceptions are listed explicitly). Theme derivation from 9 base colors, with derived text lifted to readable contrast.
- **oasis-html** / **oasis-css** / **oasis-http** -- The browser engine's parser and loader crates (no layout / paint / QuickJS dependency, so each builds and tests standalone): HTML tokenizer + tree builder + arena DOM; CSS parser + cascade + computed values (features `parallel-style`, `web-fonts`, `scripting`, forwarded from oasis-browser); resource loader (HTTP/1.1 + HTTP/2, Gemini framing, cookies, cache, CSP, IO thread, `native_net` build-script cfg, feature `psp`). oasis-core's `browse` / `fetch` / `gemini` / `curl` commands use oasis-http directly
- **oasis-browser** -- Embeddable HTML/CSS/Gemini rendering engine (layout, paint, widget, forms, JS DOM; aliases oasis-html / oasis-css internally and re-exports `oasis_http as loader`): WHATWG HTML, full CSS cascade with `@media` / `@container` / `@layer` / `@supports` / `@scope` / CSS Nesting / `:has()`, viewport-aware stylesheet parsing, CSS Grid, `var()` substituted at computed-value time, 2D + 3D transforms, `@font-face` web fonts, canvas + SVG, HTTP/1.1 + HTTP/2 over rustls (8 MB decompressed-body cap desktop, 2 MB on PSP), RFC 6265 session cookies handled per redirect hop (desktop only), CSP `script-src` / `style-src` (nonces, sha256 hashes) + `connect-src`, origin-checked `fetch` + per-origin quota'd Web Storage (JS glue in `src/js_dom/`), reader mode, forms, bookmarks. **Feature catalogue:** [`docs/browser-engine.md`](docs/browser-engine.md). **Backlog:** [`docs/browser-backlog.md`](docs/browser-backlog.md).
- **oasis-js** -- QuickJS-NG via `rquickjs`, linked where `oasis-browser/javascript` is on: the desktop shell (`oasis-app` feature `javascript`, default) and PSP. WASM and the UE5 FFI build without it (`oasis-core` default features carry no JS). DOM bindings (`getElementById`, `querySelector`, `fetch`, `setTimeout`, `localStorage`, event bubbling, …) feature-gated as `javascript`. An execution watchdog (`JsEngine::with_context_guarded`, bounded microtask drains, timer limits) keeps runaway scripts from freezing the shell. PSP cross-compile is non-trivial (pspdev toolchain, `-msingle-float`, `psp-ld`, hand-rolled libc shim) — see [`docs/javascript-engine.md`](docs/javascript-engine.md).
- **oasis-ui** -- 27 types implementing `Widget` (Button, Card, TabBar, Panel, InputField, ScrollView, ProgressBar, Toggle, Accordion, Badge, Checkbox, ColorPicker, ContextMenu, DatePicker, Divider, Dropdown, Modal, RadioGroup, RichText, Slider, SpinBox, Spinner, SplitPane, Table, TextBlock, Tooltip, TreeView) plus ListView, MenuBar, NinePatch, flex/grid layout, focus (ring, Tab manager, spatial) and animation helpers, and the `DrawContext::text_in` (ellipsizes overflow) / `frame` drawing helpers; a robustness sweep draws every widget at degenerate sizes. Most widgets are an unadopted library catalogue (only Button, ProgressBar, Slider, Toggle, MenuBar and flex have in-tree callers). Catalogue: [`docs/ui-widgets.md`](docs/ui-widgets.md)
- **oasis-vfs** -- Virtual file system: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes). One path normal form (`normalize_path`) and shared `Vfs::validate_rename` invariants; `tests/vfs_conformance.rs` runs one suite against all three. See [`docs/adr/002-vfs-abstraction.md`](docs/adr/002-vfs-abstraction.md)
- **oasis-terminal** -- Command interpreter: 75 registered commands (core, platform, remote, network, audio, radio, text, file, system, dev, fun, UI, security, doc; `open <app> [file]` queues a `/var/wm/request` open); oasis-core adds 19 (skin, agent ×6, browser ×5, plugin, transfer ×3, script ×2, update), oasis-apps adds `tv` and oasis-app adds `mcp-server` (feature `mcp`) -- 95 on the default desktop build. Plus 21 shell builtins (one `BUILTINS` table in `builtins.rs`). To recount: `CommandRegistry::list_commands().len()` after `oasis_terminal::register_builtins` (75), and `grep -A2 'fn name(&self)'` / `define_command!` in `oasis-core/src`, `oasis-apps/src`, `oasis-app/src` for the rest (`docs/terminal-commands.md` has the catalogue). Depends only on oasis-types/vfs/platform/net. Shell features: variables, globs, aliases, piping, one script engine (`script/`: if/while/until/for/case) for inline and `run` scripts, `cmd &` jobs, and a `ShellSession` (`session.rs`) with readline editing, tab completion and persistent history
- **oasis-wm** -- Window manager (window configs, hit testing, drag/resize, minimize/maximize/close, edge snapping, tiling, open/close/minimize animations). See [`docs/window-manager.md`](docs/window-manager.md)
- **oasis-net** -- TCP networking with PSK authentication, remote terminal, TLS, hosts; with feature `vfs` (enabled by oasis-core) the FTP-like transfer server (`ftp::FtpServer`). The `ftp` / `push` / `pull` commands live in `oasis-core::transfer`
- **oasis-mcp** -- Optional MCP control server (off by default). Minimal Streamable-HTTP transport + JSON-RPC 2.0 / MCP dispatch over a non-blocking `NetworkStream`; app-agnostic (depends only on `oasis-types`) with a `ToolDispatcher` callback the app implements in `crates/oasis-app/src/mcp_tools.rs`. Lets an on-device agent drive the shell. See [`docs/mcp-server.md`](docs/mcp-server.md)
- **oasis-audio** -- `AudioManager` (music playlist + playback state machine; `MusicController` carries out its actions for the desktop shell and the WASM backend, serving the `music` command / Music Player over `/var/audio/*`, paths and `MediaRequest` in `oasis_types::ipc`), shuffle/repeat modes, internet radio (`RadioManager`), SFX, WAV/Ogg Vorbis decode
- **oasis-platform** -- Platform service traits: PowerService, TimeService, UsbService, NetworkService, OskService
- **oasis-video** -- MP4 / H.264 + AAC decode pipeline. Additive feature flags, none on by default: `demux-lite` (dependency-free `demux_lite::Mp4Lite`, used by PSP and oasis-app's seek lookup), `symphonia` (symphonia demux/AAC; implies `demux-lite`), `h264` (openh264 on top of `symphonia`), `ffmpeg` (statically linked ffmpeg). With both `h264` and `ffmpeg` on, ffmpeg is the `SoftwareVideoDecoder` backend. `video-decode` / `video-decode-ffmpeg` are features of `oasis-app` / `oasis-ffi` that select `h264` / `ffmpeg`. Desktop progressive streaming + PSP in-memory ME-hardware pipeline are documented in [`docs/video-streaming.md`](docs/video-streaming.md) and [`docs/psp-architecture.md`](docs/psp-architecture.md) §Video Streaming.
- **oasis-vector** -- Resolution-independent vector graphics: scene graph with path-based drawing operations (fill, stroke, arcs, beziers), Altimit-style dashboard icons, and frame-driven animations. Integrates via `SdiBackend` vector graphics trait extensions
- **oasis-shader** -- Animated shader wallpapers: Shadertoy-style fragment shaders, 8 built-ins (`BuiltinShader`: balatro, voronoi, city_lights, ocean_waves, calm_waves, starfield, plasma, matrix_rain), each with a GLSL and a CPU implementation
- **oasis-app-core** -- Shared app framework: the `App` trait (input, `handle_key`, `accepts_text`, `handle_scroll`, `layout`, `tick`, `wants_frame`, `apply_vfs_ops`, `drain_requests`, `draw_windowed` -- the one render path, kiosk-fullscreen windows included; `as_any` via the `AsAny` blanket impl), `AppAction`, `AppRequest`, `ContentState` + `impl_content_app_methods!`, the windowed line-list renderer, `testing::AppHarness` (feature `testing`). Guide: [`docs/writing-apps.md`](docs/writing-apps.md)
- **oasis-app-*** -- 10 app crates: `oasis-app-games`, `oasis-app-paint`, `oasis-app-text-editor`, `oasis-app-calculator`, `oasis-app-media` (Music Player + Photo Viewer), `oasis-app-tv-guide`, `oasis-app-radio`, `oasis-app-settings`, `oasis-app-file-manager`, `oasis-app-system-monitor` (System Monitor + Network status from `/system/state/net` + Package Manager). The Browser is not an app crate: every host opens its own `BrowserWidget` for that title
- **oasis-apps** -- The built-in app bundle hosts depend on: `builtin_registry(&HostInfo)` (one `AppRegistration` per app; `HostInfo` carries the platform / backend names), the `tv` command (`register_commands`), `TvGuideRunnerExt` (TV Guide state on an `AppRunner`), `titles` constants, re-exports of the app crates hosts use; feature `video-embed` (WASM YouTube app). Editing an app crate rebuilds this and the hosts, not `oasis-core`
- **oasis-rasterize** -- Software rasterizer: `SoftwareBuffer` (the UE5 framebuffer), bitmap glyph masks (`glyph_mask`, shared by SDL, UE5 and WASM), optional TTF (`ttf` feature, fontdue; SDL only) and `GlyphCacheKey`; re-exports the span generators from `oasis_types::rasterize` (which SDL uses for its shapes). WASM uses only the glyph masks/metrics; its shapes are native Canvas2D paths
- **oasis-i18n** -- Internationalization: 7 embedded TOML catalogues, `tr!`, `translate_app_title`, bitmap-font coverage check with English fallback (`set_ui_locale`). Used by the desktop shell (Settings → Language, persisted) and by oasis-core chrome (dashboard labels, start menu, taskbar, bottom bar); see [`docs/i18n.md`](docs/i18n.md)
- **oasis-test-backend** -- `SdiBackend` test doubles: `NullBackend` (no-op, measure-only), `RecordingBackend` (`DrawCommand` log + query helpers, real clip/translate/render-target state), `MockSdiCore`, `ClipAuditBackend`, and the cross-backend `conformance` scenarios. Custom recorders use `oasis_types::impl_sdi_defaults!`; see [`docs/testing.md`](docs/testing.md)
- **oasis-usb-host** -- Standalone host binary (rusb) that streams frames to / reads input from the PSP USB thin client (`oasis-usb-client-psp`)
- **oasis-core** -- Coordination layer: dashboard, agent/MCP, plugin, scripting, status/bottom bars, desktop taskbar, `AppRunner` + `AppRegistry` / `AppRegistration` (host-owned; `AppRunner::launch(&apps, &entry, &vfs)`). It depends on `oasis-app-core` but on no app crate, and ships no app beyond the Terminal delegate (`SimpleApp`, which the desktop input pipeline syncs scrollback into via `AppRunner::sync_terminal_lines`; it is also the placeholder for unknown titles). Plugin apps join the host's registry through `PluginManager::extend_registry`. Desktop-shell-only state (the `settings` store / `UserPrefs`, the UI sound queue) lives in `oasis-app`.

### Font Rendering

Proportional bitmap font rendering from glyph ink bounds. `oasis-types` provides `glyph_advance()` / `glyph_advance_scaled()` with variable per-character widths (3-8px) and the one bitmap glyph table (`oasis-types/src/bitmap_font.rs`); `oasis-rasterize::glyph_mask` is the single bitmap rasterizer shared by SDL, UE5 and WASM, and `bitmap_font::bitmap_line_height` / `bitmap_ascent` are the line metrics every backend (and the `SdiText` defaults) report. The PSP EBOOT builds its glyph atlas from the same table; only the standalone `oasis-plugin-psp/src/font.rs` carries a copy (the PRX must stay self-contained). The SDL backend can also draw skin TTF/OTF fonts via fontdue (`oasis-rasterize` feature `ttf`, `SdiText::set_font`); UE5 and WASM always use the bitmap font. The PSP backend additionally uses system TrueType fonts via `psp::font` with a VRAM glyph atlas.

### FFI Boundary (oasis-ffi)

Exports C-ABI functions: `oasis_create`, `oasis_create_full`, `oasis_destroy`, `oasis_tick`, `oasis_send_input`, `oasis_get_buffer`, `oasis_get_dirty`, `oasis_send_command`, `oasis_free_string`, `oasis_set_vfs_root`, `oasis_register_callback`, `oasis_add_vfs_file`; audio: `oasis_audio_load`, `oasis_audio_play`, `oasis_audio_pause`, `oasis_audio_resume`, `oasis_audio_stop`, `oasis_audio_set_volume`, `oasis_audio_get_volume`, `oasis_audio_is_playing`, `oasis_set_audio_callback`; video (feature `video-decode`/`video-decode-ffmpeg`): `oasis_video_play`, `oasis_video_stop`, `oasis_video_next_frame`, `oasis_video_get_audio`, `oasis_video_is_playing`. This is how UE5 (or any C-compatible host) embeds OASIS_OS. Reference: [`docs/ffi-integration.md`](docs/ffi-integration.md).

## Code Conventions

- MSRV: 1.91.0 (uses `str::floor_char_boundary`)
- Max line width: 100 characters
- Clippy warnings are CI errors (`-D warnings`)
- Workspace lints: `clone_on_ref_ptr`, `dbg_macro`, `todo`, `unimplemented` = warn; `unsafe_op_in_unsafe_fn` = warn; `unwrap_used` = deny (`clippy.toml` allows it in `#[test]` / `#[cfg(test)]` code; CI lints all targets, so tests and benches must be clippy-clean too)
- All unsafe blocks require `// SAFETY:` comments
- Unit tests are in-module (`#[cfg(test)] mod tests`); add new tests there. Many crates also have `tests/` integration suites (every app crate, oasis-app e2e, oasis-browser, oasis-html, oasis-terminal, oasis-vfs, oasis-wm, ...) and some have criterion `benches/`
- User-flow / wiring changes in the desktop shell get an end-to-end scenario in `crates/oasis-app/tests/e2e_*.rs` using `oasis_app::harness::Harness` (see [`docs/testing.md`](docs/testing.md))
- Dual-licensed: Unlicense + MIT

## Docker Services

`docker-compose.yml` profiles:
- `ci` -- rust-ci container (rust:1.99-slim + cmake + X11/audio dev libs + nightly + cargo-deny)
- `psp-build` -- PSP build environment (`docker/psp-build.Dockerfile`: pspdev toolchain + the pinned nightly + cargo-psp, as the CI runner has them)
- `psp` -- PPSSPP emulator (multi-stage build, NVIDIA GPU passthrough)
- `services` -- MCP server containers (code-quality, content-creation, gemini, etc.); `memory` -- the AgentCore memory service alone

## Document Index

Key documentation files for agents and contributors. Read these for deeper context on specific topics rather than loading everything into every conversation.

### Architecture & Design
- [`docs/README.md`](docs/README.md) -- Index of every document under `docs/`
- [`docs/design.md`](docs/design.md) -- Technical design document v3.1 (~830 lines: goals, layering, backend contract, platform notes; links to the subsystem docs)
- [`docs/browser-engine.md`](docs/browser-engine.md) -- Browser feature catalogue (HTTP, HTML, CSS, layout, fonts, chrome, JS bindings)
- [`docs/javascript-engine.md`](docs/javascript-engine.md) -- QuickJS-NG integration and PSP cross-compile
- [`docs/psp-architecture.md`](docs/psp-architecture.md) -- PSP two-binary split, GU, TLS 1.3, ME video decode
- [`docs/video-streaming.md`](docs/video-streaming.md) -- Desktop `StreamingBuffer` progressive playback
- [`docs/boot-splash.md`](docs/boot-splash.md) -- Functional boot splash: BIOS phase probes, splash-phase warm-up, `BootSplash` API
- [`docs/adr/001-arena-based-dom.md`](docs/adr/001-arena-based-dom.md) -- ADR: Arena-based DOM allocation
- [`docs/adr/002-vfs-abstraction.md`](docs/adr/002-vfs-abstraction.md) -- ADR: Virtual file system design
- [`docs/adr/003-backend-trait-design.md`](docs/adr/003-backend-trait-design.md) -- ADR: Backend trait hierarchy
- [`docs/adr/004-psp-two-binary-architecture.md`](docs/adr/004-psp-two-binary-architecture.md) -- ADR: PSP EBOOT + PRX split
- [`docs/adr/005-toml-skin-system.md`](docs/adr/005-toml-skin-system.md) -- ADR: TOML skin engine

### Guides
- [`docs/getting-started.md`](docs/getting-started.md) -- Getting started guide
- [`docs/writing-apps.md`](docs/writing-apps.md) -- `App` trait lifecycle, hooks, idle-frame rules, minimal example app
- [`docs/testing.md`](docs/testing.md) -- Test layers and the headless shell e2e harness (`oasis_app::harness`): drive the real shell (click, key, type, open apps, pixels, drawn text, captured audio)
- [`docs/ui-widgets.md`](docs/ui-widgets.md) -- `oasis-ui` widget catalogue
- [`docs/window-manager.md`](docs/window-manager.md) -- `WindowManager` API, snapping, shortcuts, animations
- [`docs/wasm-backend.md`](docs/wasm-backend.md) -- WASM build, serving, input mapping, iframe overlay
- [`docs/adding-commands.md`](docs/adding-commands.md) -- How to add terminal commands
- [`docs/terminal-commands.md`](docs/terminal-commands.md) -- Catalog of built-in terminal commands grouped by module
- [`docs/skin-authoring.md`](docs/skin-authoring.md) -- Skin creation with full TOML reference
- [`docs/plugin-development.md`](docs/plugin-development.md) -- Plugin development guide
- [`docs/ffi-integration.md`](docs/ffi-integration.md) -- UE5 / C-ABI integration guide
- [`docs/psp-plugin.md`](docs/psp-plugin.md) -- PSP kernel plugin (PRX) documentation
- [`docs/browser-backlog.md`](docs/browser-backlog.md) -- Browser engine backlog and roadmap
- [`docs/theming-desktop-plan.md`](docs/theming-desktop-plan.md) -- Advanced theming + PSIX-style desktop metaphor plan (asset pipeline, free icons, shaped chrome, perf)

### Subsystems
- [`docs/oasis-js.md`](docs/oasis-js.md) -- JavaScript engine desktop API + DOM bindings catalog (PSP cross-compile lives in `javascript-engine.md`)
- [`docs/networking.md`](docs/networking.md) -- TCP transport, PSK auth, remote terminal protocol, TLS, FTP-like file transfer
- [`docs/mcp-server.md`](docs/mcp-server.md) -- Optional MCP control server: enabling, transport, tool catalog, security, agent (Claude Code) integration
- [`docs/audio-engine.md`](docs/audio-engine.md) -- AudioManager, playlist/repeat/shuffle, radio sources, streaming back-pressure
- [`docs/vector-graphics.md`](docs/vector-graphics.md) -- VectorOp/VectorScene, icon catalog, AnimClock, SdiVector integration
- [`docs/shaders.md`](docs/shaders.md) -- Built-in shader wallpapers, GPU + software backends, extension guide
- [`docs/i18n.md`](docs/i18n.md) -- `oasis-i18n` locales, locale selection, adding strings and translations

### Operations
- [`docs/psp-autorun.md`](docs/psp-autorun.md) -- Boot-time script runner for deterministic PSP/PPSSPP integration tests
- [`docs/troubleshooting.md`](docs/troubleshooting.md) -- Troubleshooting common issues
- [`docs/security.md`](docs/security.md) -- Security policy and advisories
- [`AGENTS.md`](AGENTS.md) -- Multi-agent system configuration and CI workflow
- [`CHANGELOG.md`](CHANGELOG.md) -- Notable changes by area (update it for user-visible or breaking changes)
- [`CONTRIBUTING.md`](CONTRIBUTING.md) -- Contribution policy (AI-authored only)
- [`scripts/psp-scenarios.md`](scripts/psp-scenarios.md) -- PSP test scenario documentation
