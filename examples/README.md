# Examples

Small, self-contained programs showing how to embed and drive OASIS_OS. The
Rust examples live here at the repo root but are registered as `[[example]]`
targets of the crate whose API they demonstrate (the workspace root is a
virtual manifest, so it cannot own examples). None of them need ffmpeg.

| Example | Owning crate | What it shows | Run |
|---------|--------------|---------------|-----|
| [`custom_skin.rs`](custom_skin.rs) | `oasis-skin` | Resolve a skin by built-in name or directory, validate it, derive the `ActiveTheme`, parse a skin from inline TOML | `cargo run -p oasis-skin --example custom_skin -- skins/paper` |
| [`headless_screenshot.rs`](headless_screenshot.rs) | `oasis-backend-ue5` | Render a skinned dashboard with the pure-software UE5 backend and save a PNG (no window, no GPU) | `cargo run -p oasis-backend-ue5 --example headless_screenshot -- xp out.png` |
| [`minimal_sdl.rs`](minimal_sdl.rs) | `oasis-backend-sdl` | The smallest interactive shell: SDL3 window, skin layout, dashboard navigation, render loop | `cargo run -p oasis-backend-sdl --example minimal_sdl -- classic` |
| [`ffi_demo.c`](ffi_demo.c) | `oasis-ffi` (C) | Embedding through the C ABI: `oasis_create_full`, VFS, ticking, framebuffer, commands, input, callbacks, audio | see below |

All Rust examples take an optional skin name (any built-in, e.g. `classic`,
`xp`, `modern`) or a skin directory path as their first argument.

Build every example without running it:

```bash
cargo build --examples -p oasis-skin -p oasis-backend-ue5 -p oasis-backend-sdl
```

`minimal_sdl` compiles SDL3 from source on first build (needs cmake and a C/C++
compiler; see [docs/getting-started.md](../docs/getting-started.md)).

## C FFI demo

Build the shared library, then compile the demo against it from the repo root
(the demo reads `skins/classic/*.toml` relative to the working directory):

```bash
cargo build --release -p oasis-ffi

# Linux
gcc -o ffi_demo examples/ffi_demo.c -L target/release -loasis_ffi -Wl,-rpath,target/release
# macOS
clang -o ffi_demo examples/ffi_demo.c -L target/release -loasis_ffi \
  -Wl,-rpath,@loader_path/target/release
# Windows (MinGW gcc can link the DLL directly; keep oasis_ffi.dll next to the exe)
gcc -o ffi_demo.exe examples/ffi_demo.c target/release/oasis_ffi.dll

./ffi_demo
```

Build `oasis-ffi` with `--features video-decode` and compile the demo with
`-DOASIS_WITH_VIDEO` to include the video API. The full C API reference is in
[docs/ffi-integration.md](../docs/ffi-integration.md).

## Further examples in the tree

- `crates/oasis-app/src/main.rs` -- the full desktop shell (the best reference
  for wiring apps, terminal, window manager, and browser together).
- `crates/oasis-app/src/screenshot.rs` -- the screenshot tool used by CI.
- [`docs/examples/oasis.mcp.json`](../docs/examples/oasis.mcp.json) -- an MCP
  client config for driving the shell from an agent ([docs/mcp-server.md](../docs/mcp-server.md)).
