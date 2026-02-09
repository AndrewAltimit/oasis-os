# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

OASIS_OS is an embeddable operating system framework in Rust (edition 2024). It provides a skinnable shell with a scene-graph UI, command interpreter, virtual file system, plugin system, and remote terminal. It renders to any pixel buffer + input stream. Originally ported from a PSP homebrew shell (2006-2008).

Native virtual resolution is 480x272 (PSP native) across all backends.

## Build Commands

All CI commands run inside Docker containers. For local development you can run cargo directly if SDL2 dev libs are installed, or use the Docker wrapper.

```bash
# Build (desktop)
cargo build --release -p oasis-app

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
cargo clippy --workspace -- -D warnings

# License/advisory audit
cargo deny check

# Build PSP backend (excluded from workspace, requires nightly + cargo-psp)
cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release

# Build UE5 FFI shared library
cargo build --release -p oasis-ffi

# Take screenshots
cargo run -p oasis-app --bin oasis-screenshot
```

## CI Pipeline Order

format check -> clippy -> test -> release build -> cargo-deny -> PSP EBOOT build -> PPSSPP headless test

All steps run via `docker compose --profile ci run --rm rust-ci`.

## Architecture

### Crate Dependency Graph

```
oasis-core  (platform-agnostic core, zero internal deps)
├── oasis-backend-sdl  (SDL2 desktop/Pi rendering + input + audio)
│   └── oasis-app      (binary entry points: oasis-app, oasis-screenshot)
├── oasis-backend-ue5  (software RGBA framebuffer for Unreal Engine 5)
│   └── oasis-ffi      (cdylib C-ABI for UE5 integration)
└── oasis-backend-psp  (EXCLUDED from workspace, PSP hardware via sceGu)
```

### Key Abstraction: Backend Traits

`oasis-core/src/backend.rs` defines the only abstraction boundary between core and platform:
- `SdiBackend` -- rendering (clear, blit, fill_rect, draw_text, load_texture, swap_buffers, read_pixels)
- `InputBackend` -- input polling (returns `Vec<InputEvent>`)
- `NetworkBackend` -- TCP networking
- `AudioBackend` -- audio playback

Core code never calls platform APIs directly. All platform interaction goes through these traits.

### Core Modules (oasis-core)

- **sdi** -- Scene Display Interface: the scene graph. Named objects with position, size, color, texture, text, z-order. String ID registry.
- **skin** -- Data-driven TOML skin system (skin.toml + layout.toml + features.toml). See `skins/classic/`.
- **vfs** -- Virtual file system with three implementations: `MemoryVfs` (in-RAM), `RealVfs` (disk), `GameAssetVfs` (UE5 with overlay writes)
- **terminal** -- Command interpreter with registry of builtin commands
- **wm** -- Window manager (window configs, hit testing)
- **apps** -- App runner (file manager, settings, music player, photo viewer, etc.)
- **dashboard** -- Icon grid, discovers apps from VFS
- **input** -- Platform-agnostic `InputEvent`, `Button`, `Trigger` enums
- **plugin** -- Plugin traits and manager
- **agent** -- Agent status, MCP integration, tamper detection, health monitoring
- **net** -- TCP networking with PSK authentication

### Font Rendering

Each backend implements its own 8x8 bitmap font via glyph tables in `font.rs` files. No external font dependencies.

### FFI Boundary (oasis-ffi)

Exports C-ABI functions: `oasis_create`, `oasis_destroy`, `oasis_tick`, `oasis_send_input`, `oasis_get_buffer`, `oasis_get_dirty`, `oasis_send_command`, `oasis_free_string`, `oasis_set_vfs_root`, `oasis_register_callback`, `oasis_add_vfs_file`. This is how UE5 (or any C-compatible host) embeds OASIS_OS.

## Code Conventions

- MSRV: 1.91.0 (uses `str::floor_char_boundary`)
- Max line width: 100 characters
- Clippy warnings are CI errors (`-D warnings`)
- Workspace lints: `clone_on_ref_ptr`, `dbg_macro`, `todo`, `unimplemented` = warn; `unsafe_op_in_unsafe_fn` = warn
- All unsafe blocks require `// SAFETY:` comments
- Tests are in-module (`#[cfg(test)] mod tests`), not in a separate `tests/` directory
- Dual-licensed: Unlicense + MIT

## Docker Services

`docker-compose.yml` profiles:
- `ci` -- rust-ci container (rust:1.93-slim + SDL2 dev libs + nightly + cargo-deny)
- `psp` -- PPSSPP emulator (multi-stage build, NVIDIA GPU passthrough)
- `services` -- MCP server containers (code-quality, content-creation, gemini, etc.)
