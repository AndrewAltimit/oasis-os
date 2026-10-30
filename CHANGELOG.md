# Changelog

Notable changes to OASIS_OS. GitHub releases (tags `v1.1.0` ... `v1.5.0`)
carry generated notes listing every merged PR; this file summarizes the
larger changes by area. Breaking API changes are marked **(breaking)**.

## Unreleased -- overhaul (`overhaul/site-code-docs-2026-10`)

A repository-wide cleanup: bug fixes from a full code review and an
adversarial review pass, tighter traits and crate layering, dead-code
removal, a restyled site, and docs that match the code.

### Behaviour changes to know about

- Skin names resolve strictly: `skin typo` is an error instead of silently
  acting on Classic (boot paths still fall back to Classic).
- Closing a window asks the app first everywhere (titlebar, `wm close`,
  MCP `close_window`, WASM), so unsaved Text Editor edits raise a
  Save / Discard / Cancel prompt; `wm close --force <id>` and MCP
  `close_window { force: true }` skip it.
- Requests written to `/var/wm/request` and `/var/audio/request` queue:
  `open a; open b` or `wm close a; wm close b` apply both.
- Music: finished tracks auto-advance (repeat / shuffle rules apply), and
  `next` / `prev` while paused stay paused.
- Maximized windows have no resize handles.
- New windows open at a size that follows the screen (380x220 on
  PSP-native skins, growing to 40 % x 45 % of larger screens), and
  cascaded windows stay on screen.
- The example clock plugin is hidden until `pclock show`.
- The browser enforces CSP `script-src` / `style-src` and `SameSite`
  cookies; large images are downscaled instead of shown broken (on PSP
  they are capped at 256 px per side).
- Desktop JavaScript is optional: `oasis-app` without the `javascript`
  feature links no QuickJS (the WASM and UE5 FFI builds don't link it
  either).

### Architecture and crate layout

- **(breaking)** The app registry is inverted: hosts own an
  `AppRegistry`, and the new `oasis-apps` crate provides
  `builtin_registry(&HostInfo)`, the `tv` command and `TvGuideRunnerExt`.
  `oasis-core` depends on no app crate, so editing an app no longer
  rebuilds it. Plugin apps join the same registry
  (`PluginManager::extend_registry`).
- **(breaking)** `oasis-app-browser` is deleted (every host opens its own
  browser view); `oasis-app-network` and `oasis-app-package-manager` are
  merged into `oasis-app-system-monitor`. Network now shows the
  shell-published `/system/state/net` instead of constants. 10 app crates
  remain.
- The browser's parser and loader are their own crates: `oasis-html`
  (tokenizer, tree builder, arena DOM), `oasis-css` (parser, cascade,
  computed values) and `oasis-http` (HTTP/1.1 + HTTP/2, Gemini, cookies,
  cache, CSP, IO thread; re-exported as `oasis_browser::loader`). Each
  builds and tests without layout, paint or QuickJS.
- **(breaking)** The FTP-like transfer server moved from `oasis-core` to
  `oasis-net::ftp` (feature `vfs`); `oasis-terminal` depends only on
  types / vfs / platform / net (the `skin` command moved to `oasis-core`).
- **(breaking)** `oasis-video` features are additive: `demux-lite`
  (replaces the no-op `no-std-demux`), `symphonia`, `h264`, `ffmpeg`;
  with both decoders on, ffmpeg wins, so `--all-features` builds.
- `oasis_types::ipc` is the one home for VFS IPC paths and typed
  requests (`/var/audio`, `/var/radio`, `/var/wm`, `/var/tv`, Settings,
  terminal UI, plugins, `/system/state/net`; `MediaRequest`, `WmRequest`,
  `PluginRequest`, `NetState`).
- **(breaking)** The `App` trait has one render path, `draw_windowed`:
  `update_sdi` / `hide_sdi` (a fullscreen SDI path no host called),
  `AppRunner::update_sdi` / `hide_sdi`, `AppRegistration::hide_sdi`,
  `ContentState::update_layout` / `animate_selection`, `AppLayout` and
  the SDI line-list renderers in `oasis_app_core::render` are gone.
  `App::handle_click` / `AppRunner::handle_click` lose their
  `fullscreen` flag: hosts passed the kiosk-window flag, which apps read
  as "use the SDI layout", so clicks in a kiosk-fullscreen Music Player
  were ignored and Settings / File Manager / TV Guide hit-tested rows
  other than the ones drawn. `App::handle_click(lx, ly, vfs)` also
  drops its `cw` / `ch`: apps hit-test against the size `App::layout`
  recorded just before (`AppRunner::handle_click` still takes the size
  and lays the app out first). `ContentState::line_at` maps a click to a
  line; `oasis_app_core::testing::click_at` lays out and clicks in unit
  tests. `TvGuideState`'s SDI renderer (`update_sdi` / `hide_sdi`,
  `colors`) is gone too, and `TvGuideState::new` / `TvGuideApp::new` no
  longer take an `ActiveTheme`.
- Shared helpers replace per-crate copies: `oasis_types::easing`,
  `util::ct_eq`, `util::base64`, `OasisError::is_would_block()` /
  `is_timed_out()`.

### Backend contract

- **(breaking)** `oasis-backend-sdl` and `oasis-backend-ue5` no longer
  depend on `oasis-core`.
- **Span-exact SDI defaults:** the `SdiShapes` / `SdiVector` defaults use
  the same span generators as the SDL and UE5 rasterizers
  (`oasis_types::rasterize`). Previously `stroke_circle` drew a filled
  disc, rounded rects ignored the radius and concave polygons filled
  wrong; PSP and every mock run these defaults. Default arcs no longer
  overshoot (exact `f32::cos` / `sin`).
- **One drawing-state contract** (ADR-003): absolute `set_clip_rect`,
  translated and intersecting `push_clip_rect`, clip-independent
  unblended `clear`, render targets that start clean and restore the
  parent's state, scaled composites. Pinned on SDL and UE5 by
  `oasis_test_backend::conformance`. Fixes WASM clip narrowing, UE5 stale
  target clips and unbounded scaled composites, and a vector-layer cache
  that disabled itself on UE5, WASM and the e2e harness.
- **(breaking)** Slimmer `SdiBackend`: `SdiAlpha` is gone
  (`viewport_size` is a provided `SdiCore` method; use
  `fill_rect(.., color.with_alpha(a))` for `fill_rect_alpha` /
  `dim_screen`), as are `SdiBatch::begin_batch` / `flush_batch`,
  `SdiVector::fill_polygon_gradient`, `SdiText::text_metrics` /
  `TextMetrics` and `ArcParams`. `current_clip_rect` returns
  `Option<ClipRect>`. Eight extension traits, 46 default methods.
- Rendering fixes: 1 px circle strokes were 2 px wide through the
  centre; translucent `fill_arc` wedges double-blended every spoke;
  `stroke_rect` drew outside thin rects and double-blended narrow ones on
  SDL / UE5; flat triangles lost their right end in the default;
  `stroke_line_dashed` hung with `dash = gap = 0`; vector
  `PolygonGradient` drew only its start colour. Huge sizes and
  coordinates clip or saturate instead of wrapping or panicking
  (`ClipRect::intersect`, translate stacks, triangle edges, circle radii,
  `SoftwareBuffer` sizes / `fill_rect` / `read_pixels`), and sub-blits
  past a texture's right edge no longer sample the next row.
- Binding, compositing or destroying a render target that is already
  bound is an error on every backend (SDL used to accept it and lose the
  target's clip); `RecordingBackend` enforces the same rules. SDL window
  resizes drop the SDL-side clip and read back bound targets at their own
  size; WASM keeps its context save stack balanced when a flipped blit
  fails.
- `impl_sdi_defaults!(T)` writes the eight empty extension impls for mocks;
  `oasis-test-backend` gains `NullBackend` and a stateful
  `RecordingBackend` (with `DrawCommand`), replacing per-crate mocks.
- **(breaking)** Removed traits and types with no production users:
  `SdiBlendMode`, `SdiGeometry`, `GeometryVertex`, the unsound
  `SpscQueue`, texture dedup, `DashStyle` and others.
  `ClipboardBackend::clear` is now `clear_clipboard`. Typed errors:
  `BackendError::{Unsupported, RenderTargetNotFound}`, `SdiError::NotFound`.
- SDI: overlay draws re-sort before drawing; object alpha also fades text,
  shadows and strokes.
- UE5 / FFI: cached vector backgrounds, animated layers at 12 FPS, render
  errors logged; `init` resizes in place. **(breaking)** FFI callback codes
  that never fired (`FILE_ACCESS`, `LOGIN`, `NETWORK_SEND`,
  `PLUGIN_LOAD`) are removed; unknown codes are ignored with a warning.

### Shell and apps

- **(breaking)** Slimmer `App` trait (`oasis-app-core`): an `AsAny`
  blanket impl replaces 19 hand-written `as_any`s; `refresh` is gone;
  `drain_requests` replaces the single pending-request slot; new `layout`
  and `handle_scroll` hooks; `handle_click` gets the VFS.
  `testing::AppHarness` drives an app in tests.
- Desktop shell: one launch path (`launch::launch`) and one close path
  (`windows::close_app_window`); every app request is drained each frame
  (was one per frame across all apps), including from apps closed that
  frame; apps still open at exit release their resources; the mouse
  wheel reaches apps. Skin swaps keep plugin apps, the status bar and the
  taskbar.
- Plugins run for real: the shell initializes them into its scene, calls
  `update_all` with a `PluginContext` (screen size, wall-clock time) and
  `shutdown_all`, and services `plugin list / load / unload` over
  `oasis_types::ipc`. Duplicate plugin app titles stay with the first
  plugin; removing a plugin app that shadowed a built-in restores it.
- The corrupted skin's glitch effect runs on the desktop (each frame is
  distorted, then restored). **(breaking)** `CorruptedModifiers::apply`
  returns a `CorruptionUndo`.
- Boot scripts run at boot: `oasis_core::script::run_boot_scripts` runs
  `/home/.profile` then `/etc/startup.sh`. **(breaking)** Replaces
  `run_startup`.
- Fixes: start-menu clicks in the right padding launched the next row's
  item; oversized skin bars overflowed bar / dashboard / terminal
  geometry; the status-bar clock drifted off its right edge; long Text
  Editor prompts overlapped the Ln/Col readout; WM underflow, focus after
  minimize and fullscreen re-entry.
- e2e coverage for the close paths, plugin tiles, terminal scroll,
  skin-swap clocks, music control and window placement across skins.

### Browser engine and security

- **Cookies:** RFC 6265 jar (host-only, `Max-Age` / `Expires`,
  public-suffix guard, `Secure`, `__Secure-` / `__Host-`, size caps),
  sent and stored per redirect hop; `SameSite=Strict/Lax` cookies are
  withheld from cross-site subresources and form posts
  (`ResourceRequest::initiator`, **breaking** for struct literals); http
  pages can't overwrite or delete `Secure` cookies; `document.cookie` is
  capped.
- **Redirects and requests:** per-hop headers (`Cookie` / `Authorization`
  dropped off-origin, strict-origin-when-cross-origin `Referer`), URL
  normalization, no credentials in URLs; network responses can't redirect
  to `vfs://` / `file:`; CR / LF in URLs or `fetch()` headers can't split
  requests.
- **CSP:** `script-src` / `style-src` enforced with nonces, sha256 hashes
  and `'unsafe-hashes'` per handler (previously only `connect-src`);
  `fetch()` redirect hops and `data:` URLs obey `connect-src`; the page
  policy survives back / forward and `304`; web-origin pages can't load
  `vfs://` subresources.
- **HTTP/2:** header-block and HPACK output caps (CONTINUATION-flood
  class), no panic on huge HPACK lengths, `GOAWAY` on oversized frames,
  trailers handled; 307 / 308 keep the body and only framed responses are
  pooled.
- **Robustness:** script-built DOM trees are depth-capped (incl. SVG /
  MathML and `innerHTML`) and capped at 200k nodes; stale JS node ids
  can't corrupt the tree; `<template>` scripts stay inert; the selector
  matcher backtracks within a traversal budget that never matches through
  `:not()` when exhausted; one per-tick JS timer budget; configurable
  `js_max_exec_ms` / `js_memory_limit_bytes` (2 s / 64 MiB desktop,
  500 ms / 8 MiB PSP) with the heap limit verified.
- **Rendering:** one record + replay renderer everywhere, with SVG and
  canvas in paint order; compositing layers placed correctly under scroll;
  one sticky-positioning formula; box `filter` applied once; gradients
  honour layer opacity; large images downscaled during decode
  (`max_image_dimension` / `max_image_pixels`); previous pages' image
  textures are freed on navigation; `text-align: start/end` follow
  `direction`.

### Skins

- **One source of truth:** all 17 built-ins are generated from `skins/`
  at build time (`desktop`, `modern`, `corrupted` and the diverged `xp`
  became TOML directories; a snapshot test pins every parsed skin).
  Settings-saved `custom-*` skins are never compiled in, and saving over a
  built-in's name is refused.
- Strict `resolve_skin` (plus `resolve_skin_or_classic` for boot);
  `skin lint` flags unknown shader names and the removed `features.toml`
  `command_categories` key. `from_base_colors` delegates to `from_skin`.

### Terminal

- One `BUILTINS` table for the 20 shell builtins (dispatch, `help`,
  `which`, completion); `CommandOutput::Status` exit codes for `&&` /
  `||` / `$?`; a pipeline's status is its last segment's; `kill N` works
  without `%`.
- `open <app> [file]` resolves relative files against the cwd;
  `wm close --force`; `ftp start 0` and bad ports are errors (0 used to
  stop the server); `ftp status` reads the published network state; cron
  names can't escape `/etc/cron`. Dead `HelpCmd` / `RunCmd` removed.

### Audio and video

- **Music IPC:** `AudioManager` is a backend-free state machine and
  `oasis_audio::MusicController` carries it out on the desktop and WASM:
  every `music` request is served (pause, resume, next, prev, vol, repeat
  and shuffle used to be dropped) and `/var/audio/status` is published.
  Shuffle continues from a track picked with `play_file`.
- **(breaking)** `OasisError::is_would_block()` no longer accepts
  `TimedOut` (new `is_timed_out()`): on a non-blocking socket it is a dead
  peer, and retrying it hung radio, FTP and MCP connections. Radio streams
  silent for 20 s are dropped; request writes are bounded.
- Radio: OS `WouldBlock` is "no data" (http:// stations failed on the
  first empty read); non-2xx Icecast responses are rejected;
  `radio::connect_icecast` follows up to 5 redirects (never https to
  http); archive redirects resolve relative `Location`s.
- Shaders: `BuiltinShader` is the one catalogue of the 8 built-ins; the
  no-op `software` feature is removed.

### VFS and networking

- `Vfs::validate_rename` shared by all implementations (no renaming `/`,
  no moving a directory into itself, no clobbering directories);
  `tests/vfs_conformance.rs` runs one suite against all three.
- `RealVfs`: fixed a symlink escape on `mkdir` of a missing path under a
  link pointing outside the root.
- `RustlsStream` keeps the I/O error kind instead of folding it into
  `WouldBlock`; MCP idle connections are reaped.

### WASM

- App requests to `/var/audio/request` and `/var/wm/request` append;
  closing / replaced apps' last requests are kept; `wm` and `open` work.
- The Music Player plays real audio through an `<audio>` element; music
  and radio stop each other.
- Closing a window asks the app first and Cancel reaches apps, as on the
  desktop.
- Skin image wallpapers, layout textures and image layers render (PSIX
  HiFi showed a plain orange screen); host-side tests cover the Canvas 2D
  clip state machine.

### PSP

- The PSP crates build warning-free, in CI and in a new `psp-build`
  Docker profile (pspdev + pinned nightly + cargo-psp) usable on any
  host; `scripts/psp-smoke-test.sh` boot-tests the EBOOTs in
  PPSSPPHeadless.
- One rust-psp revision pinned across every PSP crate; the backend
  lockfile was regenerated.
- Shared `oasis-types` glyph table and `Color` ABGR conversion (the
  duplicate table and `ColorExt` are gone); `set_clip_rect` untranslated
  per the contract; browser images fit within 256 px.
- Screen captures have the right colors. `PSM_8888` pixels are the bytes
  R, G, B, A (the "ABGR" name describes the `u32`); `psp_remote.py
  screencap` read them as A, B, G, R and wrote blue/cyan, see-through
  PNGs. It now writes opaque RGB (unit-tested in
  `scripts/test_psp_remote.py`), the journal's ffmpeg one-liner uses
  `rgb0`, and `oasis-usb-host` packs `PSM_5650` with red in the low bits
  (its "red" test frame used to show blue).
- `screencap` / `screenshot` capture the framebuffer that is on screen
  (via `sceDisplayGetFrameBuf`) instead of a fixed VRAM address, which
  returned the half-drawn back buffer every other frame; replies are sent
  in full instead of ignoring short writes.
- The `video-limit` TCP command and the `frame_limit` field of
  `video-status` are gone: nothing enforced the limit (>480p streams are
  always audio-only).
- `cmd_server` and `video` are split into modules; the never-wired PSMF
  decoder (`psmf.rs`, `psmf_decode.rs`) and the unused ME-call patching
  code (which still scanned 4,096 kernel UIDs on every first tune) are
  deleted. Fixed a null write when the font atlas allocation fails and
  a read of freed memory when a video frame outgrows its texture. Every
  `unsafe` block in the backend has a `SAFETY` comment.

### CI and tooling

- New `main-ci.yml` steps: an `oasis-app` feature-combination check,
  clippy over the supported non-default feature sets (`oasis-browser`,
  `oasis-core`, `oasis-video`, `oasis-app`, `oasis-ffi`), and
  `scripts/check-rust-psp-rev.sh`, which fails when the rust-psp pin,
  nightly date, toolchain channel or pspdev GCC version drift between
  crates, workflows and `docker/psp-build.Dockerfile`.
- PSP EBOOT, overlay PRX and recovery EBOOT builds deny rustc warnings
  (`PSP_DENY_WARNINGS`).

### Site

- Landing page, journal, blueprint viewer and WASM demo share one design
  system (tokens, top bar, footer), with current facts, social card,
  favicon, 404 page and sitemap.
- Journal articles keep their narrative with dated "Update (Oct 2026)"
  notes where claims changed.
- The demo lists all 17 skins (generated from `site/data/skins.json`;
  a test fails on stale lists) and its canvas refits without overflowing.
- Landing page revamp (hero video kept): `site/css/site.css` documents a
  token set (color, fluid type scale, spacing, radii, shadows, motion,
  focus ring) and shared components; new sections show the backend
  contract with real code (`SdiCore`, the C host loop, an `App`), a
  platform switcher with device frames, a skin stage that frames each
  capture by resolution and tints itself from colors sampled from it,
  an inline-SVG crate diagram with a text fallback, and a redesigned PSP
  section around the hardware captures. Counts re-verified (9k+ tests).

### Docs

- `docs/design.md` v3.0 links to the subsystem docs instead of repeating
  them; ADRs re-reviewed; `docs/README.md` indexes every doc;
  `CLAUDE.md` is the one architecture and CI reference (`AGENTS.md`
  links to it).
- Command, builtin, skin, widget, trait-method and FFI counts re-verified
  against the source; drifted `file.rs:NN` references and paths replaced;
  the browser crate docs describe the oasis-html / oasis-css / oasis-http
  split; this CHANGELOG added.

## Earlier releases

See the [GitHub releases](https://github.com/AndrewAltimit/oasis-os/releases)
for `v1.5.0` and before.
