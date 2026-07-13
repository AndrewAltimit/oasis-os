# Troubleshooting Guide

Common issues and solutions for building, testing, and running OASIS_OS.

---

## SDL3 Build Dependencies

The desktop backend (`oasis-backend-sdl`) compiles SDL3 from bundled source via cmake. You need cmake, a C++ compiler, and platform development headers.

**Debian / Ubuntu:**
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
```

If cmake fails with `Couldn't find dependency package for XTEST` (or similar), install the missing X11 dev package (e.g. `libxtst-dev` on Debian/Ubuntu).

---

## PSP Build Setup

The PSP backend is excluded from the workspace and requires:

1. **Rust nightly toolchain** -- the `mipsel-sony-psp` target uses `-Z build-std`.
2. **cargo-psp** -- `cargo install cargo-psp`.

Build the EBOOT:

```bash
cd crates/oasis-backend-psp
RUST_PSP_BUILD_STD=1 cargo psp --release
```

Build the kernel PRX plugin:

```bash
cd crates/oasis-plugin-psp
RUST_PSP_BUILD_STD=1 cargo psp --release
```

### MIPS memcpy/memset Recursion

LLVM on `mipsel-sony-psp` can emit calls to `memcpy`/`memset` inside their own implementations, causing infinite recursion. The workaround is manual byte-copy loops instead of slice copies in hot paths. If you see a stack overflow or hang on PSP startup, check for slice operations in `unsafe` blocks.

---

## PPSSPP Setup for PSP Testing

PPSSPP is used for headless screenshot testing in CI. The Docker Compose service handles setup automatically:

```bash
# Build the PPSSPP container (one-time)
docker compose --profile psp build ppsspp

# Run headless test
docker compose --profile psp run --rm -e PPSSPP_HEADLESS=1 ppsspp /roms/release/EBOOT.PBP
```

### GPU Passthrough

The PPSSPP container uses `runtime: nvidia` for GPU-accelerated rendering. Requirements:

- NVIDIA GPU with drivers installed on the host
- `nvidia-container-toolkit` package installed
- Docker configured to use the nvidia runtime

If you do not have an NVIDIA GPU, edit `docker-compose.yml` to remove `runtime: nvidia` and the `NVIDIA_*` environment variables. PPSSPP will fall back to software rendering (slower but functional for testing).

### X11 Display

For GUI mode (non-headless), the container needs X11 access:

```bash
xhost +local:docker
docker compose --profile psp run --rm ppsspp /roms/release/EBOOT.PBP
```

---

## Docker Environment Issues

### Volume Mount Permissions

The `rust-ci` container runs as `${USER_ID}:${GROUP_ID}` (defaulting to 1000:1000). If your host UID differs, set the variables:

```bash
USER_ID=$(id -u) GROUP_ID=$(id -g) docker compose --profile ci run --rm rust-ci cargo build --workspace
```

### Cargo Cache

The CI container stores cargo artifacts in a Docker volume (`cargo-registry-cache`) mounted at `/tmp/cargo`. If you get stale dependency errors, prune the volume:

```bash
docker volume rm oasis-os_cargo-registry-cache
```

### Rebuilding the CI Image

After changing dependencies in `Cargo.toml` or updating the Dockerfile:

```bash
docker compose --profile ci build rust-ci
```

---

## Common Build Failures

### SDL3 CMake Build Failure

```
CMake Error: Couldn't find dependency package for XTEST
```

Install the missing X11/system development headers for your platform (see "SDL3 Build Dependencies" above). Common missing packages on Debian/Ubuntu: `libxtst-dev`, `libasound2-dev`, `libpulse-dev`.

### Clippy Warnings as Errors

CI runs `cargo clippy --workspace -- -D warnings`. Any warning fails the build. Fix all warnings before pushing:

```bash
cargo clippy --workspace -- -D warnings
```

### `str::floor_char_boundary` Not Found

This method requires Rust 1.91.0+. Update your toolchain:

```bash
rustup update stable
```

### PSP Nightly Toolchain Missing

```
error: toolchain 'nightly' is not installed
```

Install it:

```bash
rustup toolchain install nightly
```

---

## Debugging Tips

### Enable Tracing

Set `RUST_LOG` for debug output:

```bash
RUST_LOG=debug cargo run -p oasis-app
```

Levels: `error`, `warn`, `info`, `debug`, `trace`. You can filter by crate:

```bash
RUST_LOG=oasis_core=debug,oasis_browser=trace cargo run -p oasis-app
```

### Single-Crate Testing

Run tests for one crate at a time to isolate failures:

```bash
cargo test -p oasis-core
cargo test -p oasis-terminal
cargo test -p oasis-browser
```

Run a single test by name:

```bash
cargo test --workspace -- test_name
```

### Screenshot Testing

Take screenshots of all skins for visual regression:

```bash
cargo run -p oasis-app --bin oasis-screenshot
```

Screenshots are saved to the `screenshots/` directory.

### Format Check

If CI fails on formatting:

```bash
# Check what's wrong
cargo fmt --all -- --check

# Auto-fix
cargo fmt --all
```
