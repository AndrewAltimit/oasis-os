# Getting Started with OASIS_OS

This guide covers setting up a development environment, building, testing, and running OASIS_OS.

## System Requirements

- **Rust:** 1.91.0 or later (uses `str::floor_char_boundary`)
- **SDL2 development libraries** (desktop builds only)
- **Docker + Docker Compose** (optional, for CI-matching builds)
- **cargo-psp + Rust nightly** (PSP cross-compilation only)

### Installing SDL2 Dev Libraries

**Debian/Ubuntu:**
```bash
sudo apt install libsdl2-dev libsdl2-mixer-dev
```

**Fedora:**
```bash
sudo dnf install SDL2-devel SDL2_mixer-devel
```

**macOS (Homebrew):**
```bash
brew install sdl2 sdl2_mixer
```

**Arch Linux:**
```bash
sudo pacman -S sdl2 sdl2_mixer
```

## Clone and Build

```bash
git clone https://github.com/AndrewAltimit/oasis-os.git
cd oasis-os

# Build the desktop app
cargo build --release -p oasis-app

# Run it
cargo run -p oasis-app
```

The desktop app opens an SDL2 window at 480x272 native resolution with the default "classic" skin.

### Choosing a Skin

OASIS_OS ships with 8 skins. Select one via environment variable or CLI argument:

```bash
# Via environment variable
OASIS_SKIN=modern cargo run -p oasis-app

# Via CLI argument
cargo run -p oasis-app -- --skin terminal
```

Available skins: `classic`, `modern`, `terminal`, `retro`, `midnight`, `xp`, `aqua`, `monochrome`.

Custom skins can be loaded from a directory containing `skin.toml`:

```bash
OASIS_SKIN=./skins/my-custom-skin cargo run -p oasis-app
```

See `docs/skin-authoring.md` for the TOML skin format.

## Running Tests

```bash
# Full workspace test suite (~2,400 tests)
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

## Taking Screenshots

Generate screenshots for all 8 skins:

```bash
cargo run -p oasis-app --bin oasis-screenshot
```

Screenshots are saved to `screenshots/<skin_name>/`.

## Docker-Based Development

If you prefer not to install SDL2 locally, the Docker CI container has everything pre-installed:

```bash
# Build (matches CI exactly)
docker compose --profile ci run --rm rust-ci cargo build --workspace --release

# Run tests
docker compose --profile ci run --rm rust-ci cargo test --workspace

# Lint
docker compose --profile ci run --rm rust-ci cargo clippy --workspace -- -D warnings
```

The CI container is `rust:1.93-slim` with SDL2 dev libs, Rust nightly, and `cargo-deny`.

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

## Project Structure

```
oasis-os/
  crates/
    oasis-types/        Foundation types, backend traits
    oasis-vfs/          Virtual file system
    oasis-sdi/          Scene display interface
    oasis-ui/           20+ reusable widgets
    oasis-wm/           Window manager
    oasis-skin/         TOML skin engine (8 skins)
    oasis-terminal/     90+ commands, shell interpreter
    oasis-browser/      HTML/CSS/Gemini browser engine
    oasis-core/         Coordination, apps, dashboard
    oasis-backend-sdl/  Desktop/Raspberry Pi backend
    oasis-backend-ue5/  Unreal Engine 5 backend
    oasis-ffi/          C-ABI shared library
    oasis-backend-psp/  PSP hardware backend
    oasis-plugin-psp/   PSP kernel-mode overlay
    oasis-net/          TCP networking, FTP
    oasis-audio/        Audio manager, MP3
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
