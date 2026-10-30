# Getting Started with OASIS_OS

This guide covers setting up a development environment, building, testing, and
running OASIS_OS on each supported target:

- [Desktop (SDL3)](#desktop-sdl3) -- Linux, macOS, Windows, Raspberry Pi
- [WebAssembly](#webassembly) -- runs in a browser tab
- [UE5 / C FFI](#ue5--c-ffi) -- embed in Unreal Engine 5 or any C host
- [PSP](#psp) -- PlayStation Portable hardware and the PPSSPP emulator

Common to all targets: **Rust 1.91.0 or later** (the MSRV; CI uses 1.99) and a
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
Most Windows developers skip ffmpeg and use the
[ffmpeg-free build](#building-without-ffmpeg), which needs nothing else:

```powershell
cargo run -p oasis-app --no-default-features --features javascript,video-decode
```

For the default ffmpeg build instead, install ffmpeg via [vcpkg](https://vcpkg.io)
(`vcpkg install ffmpeg:x64-windows`, with `VCPKG_ROOT` set) or point
`FFMPEG_DIR` at an ffmpeg dev build, and make LLVM's `libclang` available
(`LIBCLANG_PATH`).

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
command on a machine without ffmpeg dev libraries. Drop `javascript` as well
(`--no-default-features --features video-decode`) for a build that links no
QuickJS (no JavaScript in the browser).

### Feature flags (`oasis-app`)

| Feature | Default | Description |
|---------|---------|-------------|
| `javascript` | Yes | QuickJS-NG JavaScript engine for the browser |
| `video-decode-ffmpeg` | Yes | Video decode via system ffmpeg (Main/High H.264 profiles); needs ffmpeg dev libs + `pkg-config` + libclang |
| `video-decode` | No | Software decode via openh264 + symphonia (Baseline profile only); no ffmpeg needed. Use instead of `video-decode-ffmpeg` (with both on, ffmpeg is the decoder) |
| `skin-dev` | No | Polls the active external skin directory every second and hot-reloads it on change: `cargo run -p oasis-app --features skin-dev -- skins/paper` |
| `mcp` | No | Compiles in the MCP control server so a local agent can drive the shell; also needs runtime opt-in (`OASIS_MCP=1` or `mcp-server start`). See [mcp-server.md](mcp-server.md) |

### Choosing a skin

There are 17 built-in skins, each an editable TOML directory in
[`skins/`](../skins). Select one with a CLI argument or the `OASIS_SKIN`
environment variable:

```bash
cargo run -p oasis-app -- modern
OASIS_SKIN=xp cargo run -p oasis-app
```

Built-in names: `classic` (the default), `altimit`, `balatro`, `corrupted`,
`desktop`, `gnome`, `highcontrast`, `macos`, `modern`, `paper`, `psix-hifi`,
`psix-tribute`, `retro-cga`, `solarized`, `vaporwave`, `win95`, `xp`. An unknown
name is reported and the shell boots `classic`.

A directory containing `skin.toml` is also accepted, which is how you develop a
custom skin (see [skin-authoring.md](skin-authoring.md)):

```bash
cargo run -p oasis-app -- ./skins/my-custom-skin
```

### Environment variables

Read by the desktop binaries (`oasis-app`, `oasis-screenshot`) and the platform
layer. `RUST_LOG` controls logging (default `info`).

| Variable | Description | Example |
|----------|-------------|---------|
| `OASIS_SKIN` | Skin to start with (a CLI argument takes precedence); also picks the skin for `oasis-screenshot` | `OASIS_SKIN=modern cargo run -p oasis-app` |
| `OASIS_APP` | Auto-launch an app by title on startup (dashboard tiles and registered apps without a tile) | `OASIS_APP=Browser` |
| `OASIS_URL` | Initial browser URL (with `OASIS_APP=Browser`) | `OASIS_URL="vfs://sites/home/js-test.html"` |
| `OASIS_SKIP_SPLASH` | `1` skips the boot splash animation (init work still runs) | `OASIS_SKIP_SPLASH=1` |
| `OASIS_FRAME_STATS` | `1` logs per-phase frame timings (p50/p99) and drawn vs. skipped frames every ~5 s | `OASIS_FRAME_STATS=1` |
| `OASIS_FIXED_TIME` | Freeze the platform clock (status bar, TV Guide grid) for deterministic screenshots | `OASIS_FIXED_TIME="2026-01-01 12:00:00"` |
| `OASIS_FIXED_FRAME` | Shader wallpaper frame number used by `oasis-screenshot` (default 30) | `OASIS_FIXED_FRAME=0` |
| `OASIS_TV_CHANNEL` | Auto-tune this TV Guide channel number once the catalog loads (automated testing) | `OASIS_TV_CHANNEL=3` |
| `OASIS_TV_SEEK` | Override the seek position (seconds) when tuning a TV channel | `OASIS_TV_SEEK=120` |
| `OASIS_TV_TIMEOUT` | Exit N seconds after video decode starts (automated testing) | `OASIS_TV_TIMEOUT=30` |
| `OASIS_MCP` | `1` starts the MCP control server at boot (requires the `mcp` feature) | `OASIS_MCP=1` |
| `OASIS_MCP_PORT` | MCP server port (default 7345, loopback only) | `OASIS_MCP_PORT=7345` |
| `OASIS_MCP_TOKEN` | Optional bearer token the MCP server requires | `OASIS_MCP_TOKEN=secret` |
| `OASIS_SETTINGS_FILE` | Where saved preferences (skin, resolution, volume, font scale, reduced motion, locale) live; default `<config dir>/oasis-os/settings.toml` (`$XDG_CONFIG_HOME`, `%APPDATA%` or `~/.config`) | `OASIS_SETTINGS_FILE=/tmp/oasis.toml` |

Test-only switches: `OASIS_E2E_NETWORK` (the `e2e-tests` binary),
`OASIS_NETWORK_SCREENSHOTS` and `OASIS_LIVE_CSS` (`screenshot-tests`) enable
network-dependent `oasis-app` runs;
`OASIS_SOAK_SECS` / `OASIS_SOAK_TICK_MS` tune the ignored radio soak test;
`OASIS_LAYOUT_BUDGET_SCALE` scales the browser layout time budgets in
`crates/oasis-browser/tests/layout_budget.rs`; `OASIS_PARITY_RENDER_DRIVER` /
`OASIS_PARITY_DUMP` steer the backend parity suites
([testing.md](testing.md#backend-parity)).

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
cargo install wasm-pack                 # or: https://rustwasm.github.io/wasm-pack/installer/
./scripts/build-wasm.sh --release       # outputs pkg/ (omit --release for a debug build)
node scripts/serve-wasm.mjs             # or: python3 -m http.server 8080
# open http://localhost:8080/www/?skin=classic
```

`build-wasm.sh` is a bash script; on Windows run it from Git Bash or WSL (or
run `wasm-pack build crates/oasis-backend-wasm --target web --release --out-dir ../../pkg`
directly; `--out-dir` is relative to the crate). `serve-wasm.mjs` also proxies cross-origin requests for the
browser app (`/cors-proxy/`), which a plain static server cannot. No C
toolchain is needed for the WASM build. Build details, the JS API, input
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
   - Every PSP crate depends on the fork as a `git` dependency pinned to one
     revision (`psp = { git = "https://github.com/AndrewAltimit/rust-psp",
     rev = "...", ... }`), so no local checkout is needed. CI names that
     revision once (`RUST_PSP_REV` in `.github/workflows/main-ci.yml`) and
     `scripts/check-rust-psp-rev.sh` fails if a crate drifts from it.
   - `cargo psp` finds the std overlay next to the resolved `psp` package
     (the build log prints `[NOTE]: Using PSP std overlay at ...`); no
     symlink is needed.
2. **cargo-psp** from the fork, built from the same revision as the `psp`
   dependency (the crates.io `cargo-psp` is upstream's and lacks the std
   overlay lookup):
   `cargo install --git https://github.com/AndrewAltimit/rust-psp --rev <RUST_PSP_REV> cargo-psp --locked`.
   It installs `cargo-psp`, `prxgen`, `pack-pbp`, `mksfo` and `prxmin`; all
   must be on `PATH`.
3. **Pinned nightly**: each PSP crate's `rust-toolchain.toml` pins
   `nightly-2026-04-13` (the nightly rust-psp's CI validates) with
   `rust-src`; rustup installs it automatically on first build. Don't replace
   it with a floating `nightly` -- the std overlay and the `psp` crate only
   compile against a narrow nightly range.
4. **pspdev toolchain at `/opt/pspdev`** (the official
   [pspdev](https://github.com/pspdev/pspdev) release). The EBOOT compiles
   QuickJS-NG's C sources with `psp-gcc` (`-msingle-float`) and links with
   `psp-ld`; the paths are set in `crates/oasis-backend-psp/.cargo/config.toml`,
   which also pins pspdev's GCC include directory (currently 15.2.0).
5. **libclang** for bindgen (`rquickjs-sys` bindings).

Building on Linux (or WSL) is the tested path.

### Build in Docker (any host)

`docker/psp-build.Dockerfile` (compose profile `psp-build`) bundles all five
prerequisites: the pinned pspdev release, libclang, the pinned nightly with
`rust-src`, and cargo-psp from `RUST_PSP_REV`. This matches the CI runner. It
works from PowerShell on Windows as well as from Linux:

```bash
docker compose --profile psp-build build psp-build      # once
docker compose --profile psp-build run --rm psp-build \
  bash -c 'cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release'
```

The cargo registry and git cache persist in the `psp-cargo-cache` volume. Build
output lands in each crate's own `target/` on the host. If you bump the
nightly, `RUST_PSP_REV` or the pspdev release, change the matching
`ARG`s in the Dockerfile too; `scripts/check-rust-psp-rev.sh` (run in CI) fails
if the Dockerfile, the workflows and the crates disagree.

Boot-test the result in PPSSPPHeadless, as CI does. The script needs a POSIX
shell and Docker; on Windows run it from Git Bash (it sets
`MSYS_NO_PATHCONV=1` itself so Git Bash does not rewrite container paths):

```bash
scripts/psp-smoke-test.sh            # backend + recovery EBOOTs
```

### Build

```bash
# Main shell -> target/mipsel-sony-psp-std/release/EBOOT.PBP
cd crates/oasis-backend-psp
RUST_PSP_BUILD_STD=1 cargo psp --release

# Kernel-mode overlay plugin -> target/mipsel-sony-psp-std/release/oasis-plugin-psp.prx
cd crates/oasis-plugin-psp
RUST_PSP_BUILD_STD=1 cargo psp --release
# ... then strip it (drops ~135 KB of debug sections the loader never reads;
# CI ships the stripped file and fails the build above 192 KiB)
prxmin target/mipsel-sony-psp-std/release/oasis-plugin-psp.prx oasis-overlay.prx

# Network recovery EBOOT -> target/mipsel-sony-psp-std/release/EBOOT.PBP
cd crates/oasis-recovery-psp
RUST_PSP_BUILD_STD=1 cargo psp --release
```

Each block starts from the repository root.
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
[psp-autorun.md](psp-autorun.md). The first run needs
`docker compose --profile psp build ppsspp`.

### Remote development on hardware

The EBOOT runs a WiFi TCP command server for closed-loop build-deploy-test
cycles ([Developer's Journal entry 08](https://andrewaltimit.github.io/oasis-os/journal/08-remote-dev-automation.html)):

```bash
export PSP_IP=192.168.0.249                                  # PSP_PORT defaults to 9293
python3 scripts/psp_remote.py cycle target/.../EBOOT.PBP     # deploy over WiFi and reboot
python3 scripts/psp_remote.py press cross                    # remote input
python3 scripts/psp_remote.py screencap /tmp/psp.png         # live framebuffer capture
python3 scripts/psp_remote.py upload oasis.prx ms0:/seplugins/oasis.prx
python3 scripts/psp_remote.py sequence scripts/sequences/smoke-test.yaml
```

Run `python3 scripts/psp_remote.py` without arguments for the full command
list (status, log, hold, cursor, hard-reboot, skins, build-cycle, ...).

The network recovery EBOOT (`crates/oasis-recovery-psp`, built the same way)
starts a WiFi file server when R is held at boot.

## Tests, lint and formatting

```bash
cargo test --workspace                 # everything
cargo test -p oasis-browser            # one crate
cargo test --workspace -- test_name    # one test by name
cargo fmt --all                        # format (CI: cargo fmt --all -- --check)
cargo clippy --workspace --all-targets -- -D warnings  # lint; warnings are CI errors
cargo deny check                       # license / advisory audit
```

Without ffmpeg dev libraries, exclude `oasis-app` from workspace-wide commands
and run it separately with the ffmpeg-free features:

```bash
cargo clippy --workspace --exclude oasis-app --all-targets -- -D warnings
cargo clippy -p oasis-app --no-default-features --features javascript,video-decode -- -D warnings
```

Examples are built by `cargo test`; to build only them:
`cargo build --examples -p oasis-skin -p oasis-backend-ue5 -p oasis-backend-sdl`.

## Docker-based development

The CI container has every desktop dependency (including ffmpeg) preinstalled:

```bash
docker compose --profile ci run --rm rust-ci cargo build --workspace --release
docker compose --profile ci run --rm rust-ci cargo test --workspace
docker compose --profile ci run --rm rust-ci cargo clippy --workspace --all-targets -- -D warnings
```

The image is `rust:1.99-slim` plus cmake, X11/audio/ffmpeg dev headers, Rust
nightly, and `cargo-deny`. On Windows run these from PowerShell (Git Bash
rewrites the container paths), and give the container its own target
directory so its Linux artifacts don't mix with the host's:
`docker compose --profile ci run --rm -e CARGO_TARGET_DIR=/app/target/linux rust-ci ...`. Run clippy through it before opening a PR: lints
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
  crates/        51 crates: 41 workspace members, oasis-backend-psp (excluded),
                 9 standalone PSP crates
  skins/         the 17 skin directories (every built-in skin)
  examples/      runnable examples (see examples/README.md)
  docs/          guides, subsystem docs, ADRs (see docs/README.md)
  scripts/       build, test and analysis scripts
  screenshots/   per-skin screenshot gallery
  site/, www/    GitHub Pages site and the WASM page shell
```

The crate-by-crate map is the [crate table in the README](../README.md#crates);
the dependency graph is in [CLAUDE.md](../CLAUDE.md#crate-dependency-graph).

## Next steps

- [docs/README.md](README.md) -- index of all documentation
- [design.md](design.md) -- architecture overview
- [skin-authoring.md](skin-authoring.md) -- create a custom skin
- [adding-commands.md](adding-commands.md) -- add a terminal command
- [plugin-development.md](plugin-development.md) -- write a plugin
- [ffi-integration.md](ffi-integration.md) -- embed via C / UE5
- [troubleshooting.md](troubleshooting.md) -- common problems
