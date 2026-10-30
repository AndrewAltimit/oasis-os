# Changelog

Notable changes to OASIS_OS. GitHub releases (tags `v1.1.0` ... `v1.5.0`)
carry generated notes listing every merged PR; this file summarizes the
larger changes by area. Breaking API changes are marked **(breaking)**.

## Unreleased -- overhaul (`overhaul/site-code-docs-2026-10`)

A repository-wide cleanup: bug fixes found by a full code review, tighter
traits and crate layering, dead-code removal, and docs that match the code.

> **Still in flight (wave 4).** The following lanes had not landed when this
> entry was written; the orchestrator will fill them in before merge:
> app registry inversion (`oasis-apps::builtin_registry`), the browser crate
> split, video / UI / test-support consolidation, and the second site pass.

### Foundation (`oasis-types`, `oasis-sdi`, `oasis-rasterize`)

- **Span-exact SDI defaults.** The `SdiShapes` / `SdiVector` defaults
  (circles, rings, rounded rects, thick lines, polygons) now use the same
  span generators as the SDL and UE5 rasterizers, moved into
  `oasis_types::rasterize`. Previously `stroke_circle` drew a filled disc,
  rounded rects ignored the radius and concave polygons filled wrong; PSP and
  every mock run these defaults.
- `cos_approx_f32` / `sin_approx_f32` used a Taylor series that overshot
  default arcs by up to 21 %; they now call `f32::cos` / `f32::sin`.
- `impl_sdi_defaults!(T)` / `impl_sdi_defaults!(T; except ..)` write the nine
  empty extension impls for mocks and null backends.
- **(breaking)** Removed traits and types with no production users:
  `SdiBlendMode`, `SdiGeometry`, `GeometryVertex`, `sync.rs` (`SpscQueue`, which
  was unsound), `sync_clock.rs`, texture dedup, `DashStyle` and others.
  `DrawCommand` moved to `oasis-test-backend`.
- Typed errors: `BackendError::{Unsupported, RenderTargetNotFound}`,
  `SdiError::NotFound`.
- SDI: overlay draws re-sort before drawing (no stale handles after a
  create/destroy); object alpha also fades text, shadows and strokes.
- Shared helpers replace per-crate copies: `oasis_types::easing`,
  `util::ct_eq`, `util::base64`, `OasisError::is_would_block()`.
- `oasis_types::ipc`: the `/var/audio`, `/var/radio` and `/var/wm` paths and a
  typed `MediaRequest`.

### Backends

- **(breaking)** `oasis-backend-sdl` and `oasis-backend-ue5` no longer depend on
  `oasis-core`; a desktop build without the `javascript` feature links no
  QuickJS.
- **One drawing-state contract** for every backend: absolute `set_clip_rect`,
  translated and intersecting `push_clip_rect`, clip-independent unblended
  `clear`, render targets that start clean and restore the parent's state,
  scaled composites. Pinned on SDL and UE5 by
  `oasis_test_backend::conformance` (`backend_parity`). Fixes WASM clip
  narrowing and save-stack growth, UE5 stale target clips and 1:1-only
  composites, and the vector-layer bake cache that disabled itself on UE5,
  WASM and in the e2e harness.
- UE5 / FFI: cached vector backgrounds, animated layers on a 12 FPS cadence,
  render errors logged instead of discarded; `init` resizes in place.
- PSP: shared `oasis-types` glyph table and `Color` ABGR conversion (the
  duplicate table and `ColorExt` are gone); `set_clip_rect` untranslated per
  the contract. (PSP build not verified on the development host.)
- **(breaking)** `ClipboardBackend::clear` is now `clear_clipboard`.

### Apps and the desktop shell

- **(breaking)** Slimmer `App` trait (`oasis-app-core`): `AsAny` blanket impl
  replaces 19 hand-written `as_any`s; `refresh` is gone (follow-ups happen in
  the handler, polling in `tick`); `drain_requests` replaces the single
  pending-request slot; new `layout` and `handle_scroll` hooks; `handle_click`
  gets the VFS. `AppRunner` drops its mirrored state.
- Desktop shell: one launch path (`launch::launch`) and one close path
  (`windows::close_app_window`). `wm close` and MCP `close_window` now ask
  the app first instead of dropping unsaved edits (`--force` / `force` skip
  the prompt). Skin swaps keep plugin-registered apps, the status bar and the
  taskbar. The unused desktop `Mode::App` is removed. Every app request is
  drained each frame (was one per frame across all apps); the mouse wheel
  reaches apps.
- e2e coverage for the close paths, plugin tiles, terminal scroll, skin-swap
  clocks and music control.

### Terminal, skins, audio

- Terminal: one `BUILTINS` table for the 20 shell builtins (dispatch, `help`,
  `which`, completion); `CommandOutput::Status` exit codes for `&&` / `||` /
  `$?`; `kill N` works without `%`; `wm close --force`. The `skin` command moved
  to `oasis-core`, so `oasis-terminal` depends only on types / vfs / platform
  / net. Dead `HelpCmd` and `RunCmd` removed.
- **Skins have one source of truth.** All 17 built-ins are generated from
  `skins/` (the Rust-only `desktop`, `modern`, `corrupted` and the diverged
  `xp` constants became TOML directories; a snapshot test proves the move was
  lossless). Skin resolution is strict (`skin typo` is an error, not Classic);
  `skin lint` flags unknown shader names. `from_base_colors` delegates to
  `from_skin` instead of carrying a 400-line copy.
- **Music IPC.** `AudioManager` is a backend-free state machine; the desktop
  media controller now serves every `music` request (pause, resume, next,
  prev, vol, repeat, shuffle were silently dropped) and publishes
  `/var/audio/status`.
- Radio: OS-level `WouldBlock` is "no data" (http:// stations used to fail on
  the first empty read); non-2xx Icecast responses are rejected.
- Shaders: `BuiltinShader` enum is the one catalogue of the 8 built-ins; the
  no-op `software` feature is removed.

### VFS

- `Vfs::validate_rename` shared by all implementations: no renaming `/`, no
  moving a directory into itself, no clobbering directories.
- `RealVfs`: fixed a symlink escape on `mkdir` of a missing path under a link
  pointing outside the root.
- `tests/vfs_conformance.rs` runs one suite against all three implementations.

### Browser and JavaScript

- **Cookies:** RFC 6265 jar -- host-only cookies, `Max-Age` / `Expires`,
  public-suffix guard, `Secure` rules, `__Secure-` / `__Host-` prefixes, size
  caps. Cookies are sent and stored per redirect hop (they used to follow
  cross-site redirects, even https to http).
- **Redirects / URLs:** per-hop headers (`Cookie` / `Authorization` dropped
  off-origin, strict-origin-when-cross-origin `Referer`), URL normalization,
  credentials never sent.
- **CSP:** `script-src` / `style-src` enforced with nonces and sha256 hashes
  (previously only `connect-src`); web-origin pages cannot load `vfs://`
  subresources.
- **HTTP/2:** header-block and HPACK output caps (CONTINUATION-flood class),
  `GOAWAY` on oversized frames. Shared HTTP/1.1 framing between desktop and
  PSP.
- Script-built DOM trees are depth-capped (a deep `appendChild` loop used to
  overflow the stack); the selector matcher backtracks correctly with a
  traversal budget; `text-align: start/end` resolve against `direction`.
- JS: one per-tick timer budget that defers the rest; configurable
  `js_max_exec_ms` / `js_memory_limit_bytes` (2 s / 64 MiB desktop, 500 ms /
  8 MiB PSP); the heap limit is verified to be enforced.

### Design-vs-code gaps (wave 5)

- The **corrupted skin's glitch effect** now runs on the desktop shell:
  each drawn frame is distorted and then restored, so nothing drifts.
  **(breaking)** `CorruptedModifiers::apply` returns a `CorruptionUndo`;
  the unused `SkinEffect` / `CorruptedEffect` / `ScanlineEffect` are gone.
- **Boot scripts run at boot:** `oasis_core::script::run_boot_scripts` runs
  `/home/.profile` and then `/etc/startup.sh`, and the desktop shell calls it.
  **(breaking)** Replaces `run_startup`.
- **(breaking)** Removed things that never did anything: the skin
  `features.toml` `command_categories` key (now an unknown-key lint warning),
  and FFI callback codes `FILE_ACCESS` / `LOGIN` / `NETWORK_SEND` /
  `PLUGIN_LOAD`, which never fired. Registering an unknown code is now
  ignored with a warning.
- Removed `oasis_core::apps::BUILTIN_APP_TITLES`; the icon and label checks
  for every built-in title now run in `oasis-apps` against the real registry.
- Review fixes:
  - Start-menu clicks in the right padding launched an item from the next
    row; they now hit the same item the hover highlight shows.
  - Cron names can no longer escape `/etc/cron`.
  - `ftp start 0` no longer stops the server.
  - Duplicate plugin app titles stay owned by the first plugin.
  - WM: underflow, focus and fullscreen fixes.
  - Vector: fixes for a zero blink interval and for gradients under group
    alpha.

### Docs and site

- `docs/design.md` v3.0; ADRs re-reviewed (dates, DOM free list, PSP crate
  layout, backend counts); every doc indexed in `docs/README.md`; command,
  skin, trait and FFI counts re-verified against the source; drifted
  `file.rs:NN` references replaced; CHANGELOG added.
- Site landing page revamped with current facts.

## Earlier releases

See the [GitHub releases](https://github.com/AndrewAltimit/oasis-os/releases)
for `v1.5.0` and before.
