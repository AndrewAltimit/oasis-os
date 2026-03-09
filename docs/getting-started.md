# Getting Started with OASIS_OS

This guide covers setting up a development environment, building, testing, and running OASIS_OS.

## System Requirements

- **Rust:** 1.91.0 or later (uses `str::floor_char_boundary`)
- **cmake + C++ compiler** (desktop builds only -- SDL3 is compiled from bundled source)
- **X11 and audio development headers** (Linux desktop builds)
- **Docker + Docker Compose** (optional, for CI-matching builds)
- **cargo-psp + Rust nightly** (PSP cross-compilation only)

### Installing Build Dependencies (Desktop)

SDL3 is compiled from source automatically via the `build-from-source` cargo feature. You need cmake, a C++ compiler, and platform headers.

**Debian/Ubuntu:**
```bash
sudo apt install cmake g++ make libxtst-dev libx11-dev libxext-dev \
  libxrandr-dev libxcursor-dev libxi-dev libxss-dev libxkbcommon-dev \
  libwayland-dev libdecor-0-dev libasound2-dev libpulse-dev \
  libdbus-1-dev libudev-dev
```

**Fedora:**
```bash
sudo dnf install cmake gcc-c++ make libXtst-devel libX11-devel \
  libXext-devel libXrandr-devel libXcursor-devel libXi-devel \
  libXScrnSaver-devel libxkbcommon-devel wayland-devel \
  libdecor-devel alsa-lib-devel pulseaudio-libs-devel \
  dbus-devel systemd-devel
```

**macOS (Homebrew):**
```bash
brew install cmake
```

**Arch Linux:**
```bash
sudo pacman -S cmake base-devel libxtst libxrandr libxcursor libxi \
  libxss libxkbcommon wayland libdecor alsa-lib libpulse

## Clone and Build

```bash
git clone https://github.com/AndrewAltimit/oasis-os.git
cd oasis-os

# Build the desktop app
cargo build --release -p oasis-app

# Run it
cargo run -p oasis-app
```

The desktop app opens an SDL3 window at 480x272 native resolution with the default "classic" skin.

### Choosing a Skin

OASIS_OS ships with 18 skins. Select one via environment variable:

```bash
OASIS_SKIN=modern cargo run -p oasis-app
```

Available skins: `classic`, `xp`, `macos`, `gnome`, `cyberpunk`, `retro-cga`, `paper`, `win95`, `solarized`, `vaporwave`, `highcontrast`, `altimit`, `terminal`, `tactical`, `corrupted`, `desktop`, `agent-terminal`, `modern`.

Custom skins can be loaded from a directory containing `skin.toml`:

```bash
OASIS_SKIN=./skins/my-custom-skin cargo run -p oasis-app
```

See `docs/skin-authoring.md` for the TOML skin format.

## Running Tests

```bash
# Full workspace test suite (~4,600 tests)
cargo test --workspace

# Single crate
cargo test -p oasis-browser

# Single test by name
cargo test --workspace -- test_name

# With output
cargo test --workspace -- --nocapture
```

## Linting and Formatting

The CI pipeline enforces zero warnings:

```bash
# Check formatting
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all

# Lint (CI treats warnings as errors)
cargo clippy --workspace -- -D warnings

# License/advisory audit
cargo deny check
```

## Feature Flags

`oasis-app` supports optional features that can be toggled at build time:

| Feature | Default | Description |
|---------|---------|-------------|
| `javascript` | Yes | JavaScript engine (QuickJS-NG) for the browser |
| `video-decode` | Yes | Software MP4/H.264+AAC decode via oasis-video (no ffmpeg needed) |

To build without software video decode (falls back to ffmpeg subprocesses for TV Guide):

```bash
cargo build -p oasis-app --no-default-features --features javascript
```

## Taking Screenshots

Generate screenshots for all 17 skins:

```bash
cargo run -p oasis-app --bin oasis-screenshot
```

Screenshots are saved to `screenshots/<skin_name>/`.

## Docker-Based Development

If you prefer not to install build dependencies locally, the Docker CI container has everything pre-installed:

```bash
# Build (matches CI exactly)
docker compose --profile ci run --rm rust-ci cargo build --workspace --release

# Run tests
docker compose --profile ci run --rm rust-ci cargo test --workspace

# Lint
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings
```

The CI container is `rust:1.93-slim` with cmake, X11/audio dev headers, Rust nightly, and `cargo-deny`. SDL3 is compiled from source during the first build.

## PSP Cross-Compilation

Building for the PlayStation Portable requires `cargo-psp` and Rust nightly:

### Setup

```bash
# Install cargo-psp
cargo install cargo-psp

# Ensure nightly toolchain is available
rustup toolchain install nightly
```

### Build EBOOT (Main Application)

```bash
cd crates/oasis-backend-psp
RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
```

Output: `target/mipsel-sony-psp-std/release/EBOOT.PBP`

### Build PRX (Overlay Plugin)

```bash
cd crates/oasis-plugin-psp
RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
```

Output: `target/mipsel-sony-psp-std/release/oasis_plugin_psp.prx`

### Running in PPSSPP Emulator

```bash
# GUI mode
docker compose --profile psp run --rm ppsspp \
  /roms/release/EBOOT.PBP

# Headless mode (CI)
docker compose --profile psp run --rm \
  -e PPSSPP_HEADLESS=1 ppsspp /roms/release/EBOOT.PBP
```

## Building the UE5 FFI Library

For embedding OASIS_OS in Unreal Engine 5 or any C-compatible host:

```bash
cargo build --release -p oasis-ffi
```

Output: `target/release/liboasis_ffi.so` (Linux), `.dylib` (macOS), or `.dll` (Windows).

See `docs/ffi-integration.md` for the C API reference and integration guide.

## Memory Analysis (ASAN / Valgrind)

The project includes a separate CI workflow (`memory-ci.yml`) for memory safety analysis:

### AddressSanitizer (ASAN)

Catches use-after-free, buffer overflows, and memory leaks at ~2x runtime cost:

```bash
# Single-crate ASAN test (requires nightly + rust-src)
TARGET=$(rustc -vV | awk '/^host:/ { print $2 }')
RUSTFLAGS="-Zsanitizer=address" cargo +nightly test -p oasis-video \
  --target $TARGET -Zbuild-std --target-dir target/asan
```

### Valgrind Massif (Heap Profiling)

Profile peak heap usage during video decode:

```bash
# Build the profiling binary
cargo build -p oasis-video --bin video-memprofile --features h264

# Run under valgrind massif
valgrind --tool=massif --depth=15 \
  ./target/debug/video-memprofile tests/fixtures/test_320x240_2s.mp4 --frames 60

# View the heap timeline
ms_print massif.out.*
```

The `video-memprofile` binary reports machine-readable `PEAK_RSS_KB=` and `FRAME_COUNT=` on stdout for CI assertion. The `[profile.asan]` Cargo profile (`opt-level=1`, inherits `dev`) is available for ASAN builds.

## Project Structure

```
oasis-os/
  crates/
    oasis-types/        Foundation types, backend traits
    oasis-vfs/          Virtual file system
    oasis-sdi/          Scene display interface
    oasis-ui/           27 reusable widgets
    oasis-wm/           Window manager
    oasis-skin/         TOML skin engine (17 skins)
    oasis-terminal/     90+ commands, shell interpreter
    oasis-browser/      HTML/CSS/Gemini browser engine
    oasis-js/           JavaScript engine (QuickJS-NG)
    oasis-video/        Software video decode (H.264+AAC)
    oasis-core/         Coordination, 16 apps, dashboard
    oasis-backend-sdl/  Desktop/Raspberry Pi backend
    oasis-backend-wasm/ WebAssembly browser backend
    oasis-backend-ue5/  Unreal Engine 5 backend
    oasis-ffi/          C-ABI shared library
    oasis-backend-psp/  PSP hardware backend
    oasis-plugin-psp/   PSP kernel-mode overlay
    oasis-net/          TCP networking, FTP
    oasis-audio/        Audio manager, MP3/WAV
    oasis-platform/     Platform service traits
    oasis-app/          Desktop binary entry points
  skins/                External TOML skin definitions
  screenshots/          Per-skin screenshot gallery
  docs/                 Design docs, guides, ADRs
```

## Next Steps

- Read `docs/design.md` for architectural overview
- Read `docs/ffi-integration.md` for C/C++ embedding
- Read `docs/skin-authoring.md` for custom skin creation
- Browse `CLAUDE.md` for the complete developer reference
