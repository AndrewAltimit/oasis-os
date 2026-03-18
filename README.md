# OASIS_OS

An embeddable operating system framework in Rust. Renders a skinnable shell interface -- scene-graph UI, command interpreter, virtual file system, browser engine, plugin system, remote terminal -- anywhere you can provide a pixel buffer and an input stream.

**[Try it in your browser](https://andrewaltimit.github.io/oasis-os/demo/)** -- no install required.

https://github.com/user-attachments/assets/8e12988e-fb1a-4a4e-a0e5-e1f04d8cd433

**[All 18 skin screenshots](SCREENSHOTS.md)** | **[Developer's Journal](https://andrewaltimit.github.io/oasis-os/journal/)**

| Altimit | Paper | XP | Win95 | Retro CGA |
|:---:|:---:|:---:|:---:|:---:|
| ![Altimit](screenshots/altimit/01_dashboard.png) | ![Paper](screenshots/paper/01_dashboard.png) | ![XP](screenshots/xp/01_dashboard.png) | ![Win95](screenshots/win95/01_dashboard.png) | ![Retro CGA](screenshots/retro-cga/01_dashboard.png) |

| macOS | GNOME | Cyberpunk | Solarized | Vaporwave |
|:---:|:---:|:---:|:---:|:---:|
| ![macOS](screenshots/macos/01_dashboard.png) | ![GNOME](screenshots/gnome/01_dashboard.png) | ![Cyberpunk](screenshots/cyberpunk/01_dashboard.png) | ![Solarized](screenshots/solarized/01_dashboard.png) | ![Vaporwave](screenshots/vaporwave/01_dashboard.png) |

## Origin Story

OASIS_OS started as a PSP homebrew shell written in C (2006-2008), ported to Rust, and evolved into a cross-platform framework. The trait-based backend system means a single codebase renders to everything from a 333MHz MIPS handheld to Unreal Engine 5 -- including TLS 1.3 on the PSP (pure Rust, circumventing the firmware's 2008 SSL 3.0 stack) and in-memory video streaming with hardware AAC decode.

## Architecture: Write Once, Render Anywhere

Core code never calls platform APIs directly. All rendering, input, networking, and audio flow through a small set of backend traits defined in `oasis-types`. Implement 13 methods and you have a working backend; opt into up to 39 more for accelerated rendering.

```
                        ┌──────────────────────┐
                        │     Your App Code    │
                        │  oasis-core + 11 app │
                        │  crates, 32 widgets, │
                        │  browser, terminal,  │
                        │  18 skins, 90+ cmds  │
                        └──────────┬───────────┘
                                   │
                        ┌──────────┴───────────┐
                        │    Backend Traits     │
                        │                      │
                        │  SdiCore (13 req'd)   │
                        │  SdiBackend (39 opt)  │
                        │  InputBackend         │
                        │  NetworkBackend       │
                        │  AudioBackend         │
                        └──────────┬───────────┘
            ┌──────────┬───────────┼───────────┬──────────┐
            ▼          ▼           ▼           ▼          ▼
       ┌─────────┐┌─────────┐┌─────────┐┌─────────┐┌─────────┐
       │  SDL3   ││  WASM   ││   PSP   ││   UE5   ││   fb    │
       │ Desktop ││ Browser ││ sceGu   ││ RGBA    ││ /dev/fb0│
       │ + Pi    ││Canvas2D ││ HW GPU  ││ FFI buf ││ evdev   │
       └─────────┘└─────────┘└─────────┘└─────────┘└─────────┘
        keyboard    DOM       PSP ctrl   FFI input   planned
        mouse       events    + analog   queue
        gamepad     touch
```

## Key Features

**Rendering & UI**
- Scene graph (SDI) with z-order, gradients, rounded corners, shadows, alpha blending
- 32 widgets: Button, Card, TabBar, ListView, ScrollView, Slider, TreeView, Modal, and more
- Window manager with drag/resize, snap zones, tiling, virtual desktops
- 18 data-driven TOML skins (no code changes to reskin the entire UI)
- Vector graphics with path operations and frame-driven animations
- GPU shader wallpapers (Shadertoy-style: Voronoi, City Lights, Ocean Waves, Balatro)

**Browser & JavaScript**
- HTML/CSS/Gemini renderer with DOM, CSS cascade, flex/grid/table layout, `calc()`, transforms, animations, `@media`/`@supports` queries, cookies, gzip
- JavaScript engine (QuickJS-NG) with `console` API, DOM manipulation, event dispatch

**Shell & Apps**
- 90+ terminal commands across 17 modules with piping, globs, aliases, variable expansion
- 16 built-in apps: File Manager, Browser, TV Guide, Radio, Paint, Games, Calculator, Clock, Text Editor, Settings, and more
- TV Guide streams video from Internet Archive with 1980s Prevue Channel aesthetic
- Internet Radio streams MP3 from curated Internet Archive collections

**Networking & Platform**
- TLS 1.3 in pure Rust via `embedded-tls` (enables HTTPS on PSP's 2005 hardware)
- Software MP4/H.264+AAC video decode; PSP uses hardware AAC + in-memory streaming
- Remote terminal with PSK authentication for headless management
- Plugin system with VFS-based IPC and manifest discovery
- Virtual file system: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5)

## Skins

All 18 skins are defined in TOML configuration with theme derivation from 9 base colors. No code changes required. See the [Skin Authoring Guide](docs/skin-authoring.md).

| Category | Skins |
|----------|-------|
| **Desktop** | xp, macos, gnome, win95, desktop, modern |
| **Dashboard** | classic, altimit |
| **Aesthetic** | cyberpunk, vaporwave, solarized, paper |
| **Terminal** | terminal, tactical, corrupted, agent-terminal |
| **Accessibility** | highcontrast, retro-cga |

Default virtual resolution is 480x272 (PSP native). Skins may override this (e.g. modern=800x600, xp=1024x768); the backend canvas/window scales to match.

## Crates

37 crates (35 workspace + 2 excluded PSP crates):

| Crate | Description |
|-------|-------------|
| **Foundation** | |
| `oasis-types` | `Color`, `Button`, `InputEvent`, backend traits (`SdiCore`, `SdiBackend`, `InputBackend`, `NetworkBackend`, `AudioBackend`), error types, TLS |
| `oasis-vfs` | Virtual file system: `MemoryVfs`, `RealVfs`, `GameAssetVfs` |
| `oasis-platform` | Platform service traits: Power, Time, USB, Network, OSK |
| **Rendering & UI** | |
| `oasis-sdi` | Scene Display Interface: named object registry, z-order, gradients, shadows |
| `oasis-ui` | 32 widgets with theming, flex layout, accessibility labels |
| `oasis-wm` | Window manager: drag/resize, snap, tiling, virtual desktops |
| `oasis-skin` | TOML skin engine with 18 skins, theme derivation, skin inheritance |
| `oasis-vector` | Resolution-independent vector graphics, path ops, icons, animations |
| `oasis-shader` | Shadertoy-style animated fragment shaders |
| **Content** | |
| `oasis-browser` | HTML/CSS/Gemini engine: DOM, CSS cascade, flex/grid layout, `calc()`, full 2D transforms, Canvas 2D path API, SVG paths/groups, animations, transitions, `@media`/`@supports`, cookies, CSP, reader mode, JS bindings |
| `oasis-js` | QuickJS-NG runtime: `console` API, DOM manipulation, event dispatch (click/key/mouse with detail properties) |
| `oasis-terminal` | 90+ commands, 17 modules, shell features (variables, globs, aliases, piping) |
| `oasis-video` | MP4/H.264+AAC decode, streaming `VideoSource` API, symphonia + openh264 |
| `oasis-rasterize` | Software rasterizer for CPU-side rendering |
| `oasis-i18n` | Internationalization support |
| `oasis-test-backend` | Mock backend for testing |
| **Infrastructure** | |
| `oasis-net` | TCP networking, PSK auth, remote terminal, FTP |
| `oasis-audio` | Playlist, MP3/WAV decode, ID3 parsing, shuffle/repeat |
| `oasis-core` | Coordination: dashboard, agent/MCP, plugin, scripting |
| **Apps** (11 crates) | |
| `oasis-app-core` | Shared `App` trait and utilities |
| `oasis-app-*` | games, paint, clock, text-editor, calculator, media, tv-guide, radio, settings, file-manager |
| **Backends** | |
| `oasis-backend-sdl` | SDL3 desktop + Raspberry Pi |
| `oasis-backend-wasm` | Canvas 2D + DOM input + Web Audio |
| `oasis-backend-ue5` | Software RGBA framebuffer + FFI input |
| `oasis-backend-psp` | [excluded] sceGu HW rendering, TLS 1.3, in-memory streaming, AAC HW decode |
| `oasis-plugin-psp` | [excluded] Kernel-mode PRX: in-game overlay + background MP3 |
| `oasis-ffi` | C-ABI boundary for UE5 and external embeddings |
| `oasis-app` | Desktop entry point + screenshot tool |

## Building

### Desktop (SDL3)

```bash
cargo build --release -p oasis-app

# Or via Docker (matches CI)
docker compose --profile ci run --rm rust-ci cargo build --release -p oasis-app
```

### WebAssembly (Browser)

```bash
./scripts/build-wasm.sh --release    # requires wasm-pack
python3 -m http.server 8080          # serve at http://localhost:8080/www/
```

### PSP (EBOOT.PBP)

```bash
cd crates/oasis-backend-psp
RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
```

### PSP Overlay Plugin (PRX)

```bash
cd crates/oasis-plugin-psp
RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
```

See [PSP Plugin Guide](docs/psp-plugin.md) for installation and usage.

### UE5 (FFI Library)

```bash
cargo build --release -p oasis-ffi
```

### Screenshots

```bash
cargo run -p oasis-app --bin oasis-screenshot          # classic (default)
cargo run -p oasis-app --bin oasis-screenshot xp       # any skin by name
```

## Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `OASIS_SKIN` | Override the default skin at startup | `OASIS_SKIN=modern cargo run -p oasis-app` |
| `OASIS_APP` | Auto-launch a named app on startup | `OASIS_APP=Browser` |
| `OASIS_URL` | Navigate the browser to a URL on startup (requires `OASIS_APP=Browser`) | `OASIS_URL="vfs://sites/home/js-test.html"` |

## PSP Testing (PPSSPP)

The repo includes a containerized PPSSPP emulator with NVIDIA GPU passthrough:

```bash
docker compose --profile psp build ppsspp                                                     # first time only
docker compose --profile psp run --rm ppsspp /roms/release/EBOOT.PBP                          # GUI (X11)
docker compose --profile psp run --rm -e PPSSPP_HEADLESS=1 ppsspp /roms/release/EBOOT.PBP --timeout=5  # headless
```

Headless mode exits with `TIMEOUT` on success (OASIS_OS runs an infinite render loop). Any crash produces a non-zero exit code.

## CI

All CI runs in Docker on a self-hosted runner. GitHub Actions runs the full pipeline on push/PR:

format check -> clippy -> nightly clippy -> docs -> markdown links -> tests -> release build -> screenshot regression -> cargo-deny -> benchmarks -> PSP EBOOT + PPSSPP headless -> coverage -> WASM deploy

```bash
docker compose --profile ci run --rm rust-ci cargo fmt --all -- --check
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings
docker compose --profile ci run --rm rust-ci cargo test --workspace
docker compose --profile ci run --rm rust-ci cargo deny check
```

## Documentation

- [Technical Design](docs/design.md) -- architecture, backends, skins, PSP, VFS, plugins, security (v2.4, 1300+ lines)
- [Getting Started](docs/getting-started.md) -- setup, building, testing
- [Skin Authoring Guide](docs/skin-authoring.md) -- TOML reference, theme derivation, effects
- [Adding Commands](docs/adding-commands.md) -- terminal command development
- [Plugin Development](docs/plugin-development.md) -- plugin API, VFS-based IPC
- [FFI Integration](docs/ffi-integration.md) -- C API for UE5 and external hosts
- [PSP Plugin Guide](docs/psp-plugin.md) -- overlay PRX installation and controls
- [Troubleshooting](docs/troubleshooting.md) -- common issues and solutions
- [ADR Index](docs/adr/) -- architectural decision records

## Security Notice

> **OpenAI/Codex integrations are disabled.** OpenAI permits government partners unrestricted use of its models within their own definition of legality. Anthropic maintains explicit prohibitions on mass surveillance and autonomous weapons. We default to the stricter policy. To re-enable at your own risk: `CODEX_ENABLED=true`.

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Unlicense](LICENSE).
