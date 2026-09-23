# Getting Started with OASIS_OS

This guide covers setting up a development environment, building, testing, and
running OASIS_OS on each supported target:

- [Desktop (SDL3)](#desktop-sdl3) -- Linux, macOS, Windows, Raspberry Pi
- [WebAssembly](#webassembly) -- runs in a browser tab
- [UE5 / C FFI](#ue5--c-ffi) -- embed in Unreal Engine 5 or any C host
- [PSP](#psp) -- PlayStation Portable hardware and the PPSSPP emulator

Common to all targets: **Rust 1.91.0 or later** (the MSRV; CI uses 1.93) and a
clone of the repository:

```bash
git clone https://github.com/AndrewAltimit/oasis-os.git
cd oasis-os
```

Other docs: the full index is [docs/README.md](README.md); runnable examples
are in [`examples/`](../examples/README.md).

## Desktop (SDL3)

The desktop shell is the `oasis-app` crate. It opens an SDL3 window and renders
through `oasis-backend-sdl`.

### Build dependencies

SDL3 is compiled from bundled source automatically (the `sdl3` crate's
`build-from-source` feature), so you need **cmake and a C/C++ compiler**, plus
platform headers on Linux.

By default `oasis-app` also links the **system ffmpeg** libraries for video
decode (the `video-decode-ffmpeg` feature, via `ffmpeg-next` + `pkg-config`,
which also needs libclang for bindgen). If you don't want ffmpeg, skip those
packages and use the [ffmpeg-free build](#building-without-ffmpeg).

**Debian / Ubuntu:**

```bash
# SDL3 build + X11/Wayland/audio headers
sudo apt install cmake g++ make pkg-config libxtst-dev libx11-dev libxext-dev \
  libxrandr-dev libxcursor-dev libxi-dev libxss-dev libxkbcommon-dev \
  libwayland-dev libdecor-0-dev libasound2-dev libpulse-dev \
  libdbus-1-dev libudev-dev
# ffmpeg (default video-decode-ffmpeg feature)
sudo apt install libavcodec-dev libavformat-dev libavutil-dev \
  libswscale-dev libswresample-dev libclang-dev
```

**Fedora:**

```bash
sudo dnf install cmake gcc-c++ make pkgconf-pkg-config libXtst-devel libX11-devel \
  libXext-devel libXrandr-devel libXcursor-devel libXi-devel \
  libXScrnSaver-devel libxkbcommon-devel wayland-devel \
  libdecor-devel alsa-lib-devel pulseaudio-libs-devel \
  dbus-devel systemd-devel
# ffmpeg: ffmpeg-free-devel (Fedora) or ffmpeg-devel (RPM Fusion)
sudo dnf install ffmpeg-free-devel clang-devel
```

**Arch Linux:**

```bash
sudo pacman -S cmake base-devel pkgconf libxtst libxrandr libxcursor libxi \
  libxss libxkbcommon wayland libdecor alsa-lib libpulse
# ffmpeg
sudo pacman -S ffmpeg clang
```

**macOS (Homebrew):** Xcode command line tools provide the compiler and libclang.

```bash
brew install cmake pkg-config ffmpeg
```

**Windows:** install Visual Studio (Desktop development with C++) and CMake.
For the default ffmpeg build, install ffmpeg via [vcpkg](https://vcpkg.io)
(`vcpkg install ffmpeg:x64-windows`, with `VCPKG_ROOT` set) or point
`FFMPEG_DIR` at an ffmpeg dev build, and make LLVM's `libclang` available
(`LIBCLANG_PATH`). Most Windows developers use the ffmpeg-free build below.

The authoritative Linux package list is the CI image,
[`docker/rust-ci.Dockerfile`](../docker/rust-ci.Dockerfile).

### Build and run

```bash
cargo build --release -p oasis-app
cargo run --release -p oasis-app
```

The window opens at the skin's resolution (480x272 for the default `classic`
skin) after a short boot splash (`OASIS_SKIP_SPLASH=1` skips the animation).

#### Building without ffmpeg

The software decoder (openh264 + symphonia, Baseline H.264 only) needs just a
C/C++ compiler:

```bash
cargo run -p oasis-app --no-default-features --features javascript,video-decode
```

Use the same flags for any other `cargo build/test/clippy -p oasis-app`
command on a machine without ffmpeg dev libraries.

### Feature flags (`oasis-app`)

| Feature | Default | Description |
|---------|---------|-------------|
| `javascript` | Yes | QuickJS-NG JavaScript engine for the browser |
| `video-decode-ffmpeg` | Yes | Video decode via system ffmpeg (Main/High H.264 profiles); needs ffmpeg dev libs + `pkg-config` + libclang |
| `video-decode` | No | Software decode via openh264 + symphonia (Baseline profile only); no ffmpeg needed. Use instead of `video-decode-ffmpeg`, not together |
| `skin-dev` | No | Polls the active external skin directory every second and hot-reloads it on change: `cargo run -p oasis-app --features skin-dev -- skins/paper` |
| `mcp` | No | Compiles in the MCP control server so a local agent can drive the shell; also needs runtime opt-in (`OASIS_MCP=1` or `mcp-server start`). See [mcp-server.md](mcp-server.md) |

### Choosing a skin

There are 17 built-in skins; 14 of them also ship as editable TOML directories
in [`skins/`](../skins). Select one with a CLI argument or the `OASIS_SKIN`
environment variable:

```bash
cargo run -p oasis-app -- modern
OASIS_SKIN=xp cargo run -p oasis-app
```

Built-in names: `classic`, `corrupted`, `desktop`, `modern`, `xp`, `macos`,
`gnome`, `retro-cga`, `balatro`, `paper`, `win95`, `solarized`, `vaporwave`,
`highcontrast`, `altimit`, `psix-tribute`, `psix-hifi` (`corrupted`, `desktop`,
and `modern` are built-in only).

A directory containing `skin.toml` is also accepted, which is how you develop a
custom skin (see [skin-authoring.md](skin-authoring.md)):

```bash
cargo run -p oasis-app -- ./skins/my-custom-skin
```

Other useful environment variables: `OASIS_APP=<title>` auto-launches an app
(`OASIS_URL=<url>` sets the browser's first page), `OASIS_FRAME_STATS=1` logs
per-phase frame timings, and `RUST_LOG` controls logging (default `info`).

### Screenshots

```bash
cargo run -p oasis-app --bin oasis-screenshot             # classic
cargo run -p oasis-app --bin oasis-screenshot -- xp       # one skin
cargo run -p oasis-app --bin oasis-screenshot -- --all    # every built-in skin
```

Screenshots are written to `screenshots/<skin>/`. CI's visual regression check
is `cargo run -p oasis-app --bin screenshot-tests --release` (`-- --bless` to
update the goldens).

## WebAssembly

```bash
rustup target add wasm32-unknown-unknown
# install wasm-pack: https://rustwasm.github.io/wasm-pack/installer/
./scripts/build-wasm.sh --release       # outputs pkg/
node scripts/serve-wasm.mjs             # or: python3 -m http.server 8080
# open http://localhost:8080/www/?skin=classic
```

No C toolchain is needed for the WASM build. Build details, the JS API, input
mapping, the iframe overlay used for real web pages, and the optional YouTube
feature are covered in [wasm-backend.md](wasm-backend.md).

## UE5 / C FFI

`oasis-ffi` builds a C-ABI shared library around the pure-software
`oasis-backend-ue5` renderer (an RGBA framebuffer; no GPU, window or SDL):

```bash
cargo build --profile release-ffi -p oasis-ffi
# video API (oasis_video_*): add --features video-decode (or video-decode-ffmpeg)
```

Output: `target/release-ffi/liboasis_ffi.so` (Linux), `liboasis_ffi.dylib` (macOS),
or `oasis_ffi.dll` (Windows). The C API reference and a UE5 integration outline
are in [ffi-integration.md](ffi-integration.md); a runnable C host is
[`examples/ffi_demo.c`](../examples/ffi_demo.c).

For a Rust-side headless render, see
[`examples/headless_screenshot.rs`](../examples/headless_screenshot.rs):

```bash
cargo run -p oasis-backend-ue5 --example headless_screenshot -- classic out.png
```

## PSP

The PSP crates are **not** workspace members (they target `mipsel-sony-psp`)
and are built one at a time with `cargo psp`. The build has several
non-standard requirements; see [psp-architecture.md](psp-architecture.md) and
[javascript-engine.md](javascript-engine.md) for the reasons behind each.

### Prerequisites

1. **The rust-psp fork** -- [github.com/AndrewAltimit/rust-psp](https://github.com/AndrewAltimit/rust-psp),
   not upstream rust-psp. It provides the `psp` crate, the std overlay
   (`rust-std-src/`) that `cargo psp` builds `std` from, and `cargo-psp`.
   - `crates/oasis-backend-psp` and `crates/oasis-plugin-psp` depend on the
     fork through an **absolute local `path`** in their `Cargo.toml`
     (`psp = { path = ".../rust-psp/psp", ... }`); edit it to point at your
     checkout (don't commit that change).
   - The EBOOT build expects `crates/oasis-backend-psp/rust-std-src` to be a
     symlink to the fork's `rust-std-src/` (it is gitignored):
     `ln -s /path/to/rust-psp/rust-std-src crates/oasis-backend-psp/rust-std-src`.
   - Some smaller PSP crates (e.g. `oasis-usb-*-psp`) pull the fork by `git`
     URL instead and need no local checkout.
2. **cargo-psp**: `cargo install cargo-psp` (the fork also carries its own
   `cargo-psp`; prefer that one if the published version lags).
3. **Pinned nightly**: `crates/oasis-backend-psp/rust-toolchain.toml` pins a
   specific nightly with `rust-src`; rustup installs it automatically on first
   build. Don't replace it with a floating `nightly` -- the std overlay only
   compiles against a narrow nightly range.
4. **pspdev toolchain at `/opt/pspdev`** (the official
   [pspdev](https://github.com/pspdev/pspdev) release). The EBOOT compiles
   QuickJS-NG's C sources with `psp-gcc` (`-msingle-float`) and links with
   `psp-ld`; the paths are set in `crates/oasis-backend-psp/.cargo/config.toml`,
   which also pins pspdev's GCC include directory (currently 15.2.0).
5. **libclang** for bindgen (`rquickjs-sys` bindings).

Building on Linux (or WSL) is the tested path.

### Build

```bash
# Main shell -> target/mipsel-sony-psp-std/release/EBOOT.PBP
cd crates/oasis-backend-psp
RUST_PSP_BUILD_STD=1 cargo psp --release

# Kernel-mode overlay plugin -> target/mipsel-sony-psp-std/release/oasis_plugin_psp.prx
cd crates/oasis-plugin-psp
RUST_PSP_BUILD_STD=1 cargo psp --release
```

`crates/oasis-backend-psp/build.sh [release]` wraps the EBOOT build. Copy the
EBOOT to `ms0:/PSP/GAME/OASISOS/EBOOT.PBP` on a memory stick. The PRX plugin is
loaded by custom firmware via `PLUGINS.TXT`; the `plugin install` terminal
command sets it up (see [psp-plugin.md](psp-plugin.md)).

### Running in PPSSPP

```bash
# GUI mode
docker compose --profile psp run --rm ppsspp /roms/release/EBOOT.PBP

# Headless mode (CI)
docker compose --profile psp run --rm -e PPSSPP_HEADLESS=1 ppsspp /roms/release/EBOOT.PBP
```

Deterministic scripted runs use the autorun feature; see
[psp-autorun.md](psp-autorun.md).

## Tests, lint and formatting

```bash
cargo test --workspace                 # everything
cargo test -p oasis-browser            # one crate
cargo test --workspace -- test_name    # one test by name
cargo fmt --all                        # format (CI: cargo fmt --all -- --check)
cargo clippy --workspace -- -D warnings  # lint; warnings are CI errors
cargo deny check                       # license / advisory audit
```

Without ffmpeg dev libraries, exclude `oasis-app` from workspace-wide commands
and run it separately with the ffmpeg-free features:

```bash
cargo clippy --workspace --exclude oasis-app -- -D warnings
cargo clippy -p oasis-app --no-default-features --features javascript,video-decode -- -D warnings
```

Examples are built by `cargo test`; to build only them:
`cargo build --examples -p oasis-skin -p oasis-backend-ue5 -p oasis-backend-sdl`.

## Docker-based development

The CI container has every desktop dependency (including ffmpeg) preinstalled:

```bash
docker compose --profile ci run --rm rust-ci cargo build --workspace --release
docker compose --profile ci run --rm rust-ci cargo test --workspace
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings
```

The image is `rust:1.93-slim` plus cmake, X11/audio/ffmpeg dev headers, Rust
nightly, and `cargo-deny`. Run clippy through it before opening a PR: lints
change between Rust releases, so a different local toolchain can pass while CI
fails.

## Memory analysis (ASAN / Valgrind)

A separate, non-blocking workflow (`memory-ci.yml`) runs memory checks.

AddressSanitizer (requires nightly + `rust-src`):

```bash
TARGET=$(rustc -vV | awk '/^host:/ { print $2 }')
RUSTFLAGS="-Zsanitizer=address" cargo +nightly test -p oasis-video \
  --target $TARGET -Zbuild-std --target-dir target/asan
```

Valgrind massif heap profiling of video decode:

```bash
cargo build -p oasis-video --bin video-memprofile --features h264
valgrind --tool=massif --depth=15 \
  ./target/debug/video-memprofile tests/fixtures/test_320x240_2s.mp4 --frames 60
ms_print massif.out.*
```

`video-memprofile` prints machine-readable `PEAK_RSS_KB=` and `FRAME_COUNT=`
lines for CI assertions.

## Project structure

```
oasis-os/
  crates/
    oasis-types/        Foundation types (Color, Button, InputEvent), backend traits
    oasis-vfs/          Virtual file system (MemoryVfs, RealVfs, GameAssetVfs)
    oasis-platform/     Platform service traits (power, time, USB, network, OSK)
    oasis-sdi/          Scene display interface (named objects, z-order)
    oasis-ui/           Reusable widgets and flex layout
    oasis-wm/           Window manager
    oasis-skin/         TOML skin engine and built-in skins
    oasis-i18n/         Internationalization
    oasis-terminal/     Command interpreter and built-in commands
    oasis-browser/      HTML/CSS/Gemini browser engine
    oasis-js/           JavaScript engine (QuickJS-NG)
    oasis-net/          TCP networking, PSK auth, remote terminal, FTP
    oasis-mcp/          Optional MCP control server
    oasis-audio/        Audio manager, playlists, radio
    oasis-video/        MP4 / H.264 + AAC decode
    oasis-vector/       Vector graphics, icons, animations
    oasis-shader/       Shader wallpapers
    oasis-rasterize/    Software rasterizer (text, shapes) for CPU backends
    oasis-usb-host/     USB host for a PSP thin client (sends frames, receives input)
    oasis-test-backend/ Mock backend for tests
    oasis-app-core/     App framework (`App` trait) for app crates
    oasis-app-*/        13 app crates (browser, calculator, file-manager, games,
                        media, network, package-manager, paint, radio, settings,
                        system-monitor, text-editor, tv-guide)
    oasis-core/         Coordination: dashboard, terminal, plugins, scripting, bars
    oasis-backend-sdl/  SDL3 desktop / Raspberry Pi backend
    oasis-backend-wasm/ WebAssembly backend
    oasis-backend-ue5/  Software RGBA framebuffer backend (UE5)
    oasis-ffi/          C-ABI shared library
    oasis-app/          Desktop binaries (oasis-app, oasis-screenshot, tests)
    oasis-backend-psp/  PSP shell EBOOT (not a workspace member)
    oasis-plugin-psp/   PSP kernel-mode overlay PRX (not a workspace member)
    ...-psp / oasis-me-boot  Other standalone PSP tools and research crates
  examples/             Runnable examples (see examples/README.md)
  skins/                External TOML skin definitions
  www/                  WASM page shell
  scripts/              Build, test and analysis scripts
  screenshots/          Per-skin screenshot gallery
  docs/                 Guides, subsystem docs, ADRs (see docs/README.md)
```

## Next steps

- [docs/README.md](README.md) -- index of all documentation
- [design.md](design.md) -- architecture overview
- [skin-authoring.md](skin-authoring.md) -- create a custom skin
- [adding-commands.md](adding-commands.md) -- add a terminal command
- [plugin-development.md](plugin-development.md) -- write a plugin
- [ffi-integration.md](ffi-integration.md) -- embed via C / UE5
- [troubleshooting.md](troubleshooting.md) -- common problems
