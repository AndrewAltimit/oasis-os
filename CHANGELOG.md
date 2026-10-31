# Changelog

Notable changes to OASIS_OS. GitHub releases (tags `v1.1.0` ... `v1.5.0`)
carry generated notes listing every merged PR; this file summarizes the
larger changes by area. Breaking API changes are marked **(breaking)**.

## Unreleased -- overhaul 3

### Crate map

- New interactive site page, `site/crates.html`: all 51 crates in their layers; hover / focus
  lights what a crate uses and what uses it, click pins a detail card (description, Rust lines,
  dependencies by kind, reach, source link); filters for optional / dev edges, the PSP crates and
  a search box; three guided walks (a frame, a web page, a video stream) that trace onto the map.
  Linked from the nav, the homepage architecture section and the sitemap.
- `scripts/gen-crate-map.py` writes its data, `site/data/crates.json`, from every
  `crates/*/Cargo.toml` (the excluded and standalone PSP crates included). Its unit test
  (`scripts/test_gen_crate_map.py`, run by CI's python step) fails when the JSON is stale:
  after adding a crate or changing an internal dependency, run `python3 scripts/gen-crate-map.py`.

### Website

- Landing page polish (hero video kept): the contract section puts the three steps beside
  the `SdiCore` code on wide screens; feature stats gain one-line notes; the features
  section gets a lede; platform tabs fit one row on tablets; pillar / stat / trait grids
  no longer leave orphan cells; journal feature cards align.
- Landing page facts re-checked against the repo: 21 shell builtins (was 20); the
  architecture diagram now accounts for all 51 crates (new Hosts row with `oasis-app`,
  `oasis-ffi`, `oasis-usb-host`; `oasis-test-backend` under Foundation; the eight
  undrawn standalone PSP tools named in the caption); code-comment color meets WCAG AA.

### CI and tooling

- PR validation now runs the feature-set checks main runs (the `oasis-app`
  feature combinations and clippy over the non-default feature sets of
  `oasis-browser`, `oasis-core`, `oasis-video`, `oasis-app`, `oasis-ffi`),
  from one shared `scripts/ci-feature-clippy.sh`, so a PR can no longer
  pass and then break main. The script runs every line and reports all
  failures at once; `--list` prints the commands.

### Desktop shell and oasis-core

- Start menu: "Games" opens Games (it opened the File Manager) and
  "Video" opens the TV Guide (it opened the Photo Viewer); items launch
  any registered app, with or without a dashboard tile.
- `skin xp; skin @variant:dark` in the terminal derives the dark variant
  of xp, not of the skin active before the line ran.
- The Internet Radio app's Stop closes the stream connection (the shell
  kept it open and redrew every frame until the station changed).
- Settings: a rejected resolution no longer pins the current one for later
  skins, and high-contrast "off" on a non-high-contrast skin is a no-op.
- A plugin whose `init` fails no longer leaves the commands it registered
  in the shell's command registry.
- **(breaking)** `oasis_core::settings` (`SettingsStore`, `UserPrefs`,
  `pref_keys`) and `oasis_core::ui_sound` moved into the desktop shell
  (`oasis_app::settings`); no other host used them.

### Browser parsers and loader (oasis-html, oasis-css, oasis-http)

- HTTP/1.1: `304` / `204` / `1xx` responses and `HEAD` answers no longer
  wait 15 s for a body their `Content-Length` promises; `103 Early Hints`
  heads are skipped instead of becoming the response; one response gets
  at most 60 s; every resolved address is tried. A guarded (`fetch()`)
  request now refuses to connect if *any* resolved address is disallowed.
- HTTP/2: POST bodies over 16 KiB are split into frames (they failed on
  servers with default settings); padded DATA no longer drains the
  flow-control window; control-frame floods end the connection; faster
  HPACK Huffman decoding; non-UTF-8 header values decode lossily.
- `data:` URLs percent-decode to bytes (`%C3%A9` is `é`, `+` stays `+`).
  Gemini status lines with non-ASCII bytes no longer panic.
- HTML: CR LF normalised to LF and NUL to U+FFFD; duplicate attributes
  dropped (first wins) and at most 256 per tag; `&copy=` style query
  strings in attributes stay literal; `&#150;` maps through
  Windows-1252. Stray end tags no longer close enclosing table cells or
  push content outside `<html>`; `<script>` text at the nesting cap stays
  in the script. Quadratic / multiplicative cases (huge attribute lists,
  adoption agency, foster parenting, DOM depth) are bounded.
- CSS: unquoted `url(a/b.png)` and `url(data:...)` parse correctly;
  string escapes (`"\f101"` icon fonts) and escaped / non-ASCII idents
  (`.md\:flex`) work; nested rules using `:not()` etc. and declarations
  inside nested `@media` apply; at-rules nest inside `@media` /
  `@supports`; `em` media-query breakpoints work; nested `@layer` names
  are qualified. Deep nesting, `calc()` / `not` chains, nesting selector
  explosion, `var()` expansion, `:has()` with structural pseudo-classes,
  and `:nth-child(2n-2147483648)` no longer crash or hang.
- `oasis_http::http::response_find_header` and
  `http_psp::response_find_header` removed (use `http1::find_header`),
  `oasis_html::entities` and `oasis_css::selectors` are crate-private,
  `cascade::build_global_layer_map` is crate-private **(breaking)**.

### Documentation and site content

- Counts re-verified against the source: the eight extension traits carry
  47 default methods (`SdiClipTransform` has 9 since it gained
  `draw_bounds`), was 46 / 8 across CLAUDE.md, README, design, ADR-003, the
  landing page and journal entries 01 and 06; 21 shell builtins (was 20 in
  the README and two journal entries).
- CI descriptions follow the workflows: PRs run the same feature-set checks
  as main (`scripts/ci-feature-clippy.sh`, also in getting-started), criterion
  benchmarks run only on a manual `run_benchmarks` dispatch, and the
  memory workflow's peak-RSS gates run natively, outside valgrind.
- `tv` is registered by `oasis-apps`, not `oasis-core` (design, adding
  commands); remote-terminal and MCP `run_command` refuse the server toggles
  (networking, MCP server docs); a plugin whose `init` fails drops what it
  registered (plugin guide).

### Browser engine

- Flexbox layout fixes: container padding is no longer counted twice;
  `width: auto` row items size to their content instead of 0 px; column
  containers are as tall as their items, explicit flex-container heights
  apply (they were 0 px) and `align-items` centres against them; inline
  children (a nav bar of links, bare text) become flex / grid items
  instead of rendering at 0 x 0; nested flex containers no longer cost
  2^depth layouts.
- Content scrolled into a nested `overflow: auto` box is painted (it was
  culled against the unscrolled clip), `overflow: hidden` children of
  translucent / filtered boxes are clipped in the right place, and the
  display-list optimizer no longer deletes a rect because an opaque rect
  in a *sibling* clip box covers its coordinates.
- Web-font glyphs no longer carry over to the next page (font ids are
  reused per page); the glyph texture cache, image atlas (smaller pages
  on PSP) and back history (100 entries) are bounded.
- Security: Gemini page titles and directly-opened stylesheets are
  escaped (both could inject live script); `fetch()` judges the page's
  own network class by its least private address, so a page whose name
  also resolves to a LAN / loopback address cannot reach those.
- Hostile-page bounds: BMP / GIF / WebP decodes check sizes before
  allocating; self-referencing SVG `<pattern>`s, huge path coordinates
  and far off-screen strokes, `blur()` radii, sub-pixel repeating
  gradients, tiny `background-size` tiles, million-column tables, dense
  grid auto-placement, thousands of floats, long unbreakable words and
  animated `<canvas>` command lists are all bounded.
- Forms: Enter submits only after validation (as a click does), Enter in
  a `<textarea>` inserts a newline, and GET submission replaces the
  action URL's query instead of appending to it.
- Reader mode rebuilds the page through the normal pipeline (toggling it
  could panic on the next hover) and keeps no extra history entry;
  single-column multicol boxes render; `hue-rotate()` on filtered
  layers uses degrees; 16-bit PNGs decode correctly; `innerHTML` keeps
  `<style>` / `<script>` text verbatim.
- Removed the unused `gemini::renderer` module (`render_gemini`,
  `GeminiTheme`), `reader::reader_stylesheet` (now internal),
  `scroll::SCROLL_ACCELERATION` and `SvgPaint::PatternRef` (pattern
  fills are `GradientRef`s that resolve to a `<pattern>`) **(breaking)**.

### Website: journal, demo, blueprints, shared chrome

- Journal index is a card grid: newest first with the latest entry featured, generated
  per-topic covers, topic filters plus a newest / oldest sort, and "Read the latest" /
  "Start at entry 01" calls to action.
- Journal articles: running text held to ~70 characters at 18px while code, tables and
  figures break out on wide screens; an "Updated" field on entries with Oct 2026 notes;
  BlogPosting JSON-LD; an end-of-article row (tags, back to top, all entries); wide
  tables scroll with edge shadows and keyboard focus; the table of contents tracks
  jumps as well as scrolling.
- Live demo: a boot panel naming the skin being loaded, a two-column controls card
  (keyboard / mouse and touch) that no longer covers the screen on portrait phones, a
  full-screen button, and an immediate error card when the WebAssembly boot fails
  (`www/index.js` now sets `window.__oasisBootError` and fires `oasis:boot-error`).
- Blueprints: the gallery no longer downloads three.js (~700 KB, now fetched only for a
  `?diagram=` view); viewer hints and "Open the 3D model" cues; on phones the legend
  folds behind a dock button and the dock fits.
- Every page shares a new footer: Explore / Read / Project link columns (adding the crate
  map, changelog, releases and contributing guide) and a back-to-top row.

### Media and networking (oasis-audio, oasis-video, oasis-net, oasis-mcp)

- Internet radio refuses stream URLs, redirect `Location`s, hosts and paths
  containing whitespace or control bytes (a CR / LF could inject request
  headers); archive.org item identifiers outside `[A-Za-z0-9._-]` are
  dropped; remote stream titles lose control characters before reaching
  the status file. An archive 3xx without `Location` is an error, not audio.
- A radio track shorter than the 32 KiB start buffer now plays instead of
  being skipped silently.
- WAV files with a sample rate outside 1-768 kHz are rejected, and the SFX
  resampler caps its output, so a 1 Hz header can no longer exhaust memory.
- MCP server: a `401` closes the connection (one token guess per connection;
  pipelined guessing on a keep-alive connection is gone).
- FTP server: `with_password("")` / `ftp start --password ""` means no
  authentication on loopback (it used to lock every client out); `LIST`
  shows control characters in file names as `?`.
- TLS (rustls) streams no longer fail a `write` when the non-blocking
  socket is momentarily full: the plaintext was already taken, so a caller
  retrying used to send it twice. The queued ciphertext goes out on the next
  read, write or `flush` (which TLS streams now implement).
- oasis-video: one shared `avcC` parser for both demuxers, and the `moov`
  read no longer allocates the header-declared size before reading.

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
- Minimizing a maximized window and restoring it brings it back
  maximized (the next restore returns the original size); maximizing a
  minimized window shows it.
- The browser enforces CSP `script-src` / `style-src` and `SameSite`
  cookies; large images are downscaled instead of shown broken (on PSP
  they are capped at 256 px per side).
- Desktop JavaScript is optional: `oasis-app` without the `javascript`
  feature links no QuickJS (the WASM and UE5 FFI builds don't link it
  either).
- **Remote terminal PSK protocol (breaking):** the key no longer crosses
  the wire (it was sent in clear; the docs' "TLS-protected" claim was
  wrong). The listener sends `AUTH_REQUIRED <nonce>` and expects
  `AUTH <hex HMAC-SHA256(psk, nonce)>` (`oasis_net::psk_response`); a bare
  PSK is a failed attempt. PSK auth now works without the `tls-rustls`
  feature. Unauthenticated connections close after 20 s; failed logins
  (remote terminal and FTP `PASS`) feed a server-wide lockout.
- FTP `GET` returns the raw bytes (binary-safe; the length header now
  matches the body).
- MCP server: stricter HTTP parsing (`Transfer-Encoding` of any kind, bad
  header lines and non-HTTP/1.x are refused), the Origin / token checks run
  before a body is buffered or `100 Continue` is sent, a 30 s whole-request
  deadline, a 60 s keep-alive idle timeout, `DELETE` returns `405`, `null`
  JSON-RPC ids are rejected and client-posted responses get `202`.
- VFS: every backend resolves `.` / `..` the same way (`..` above `/` and
  NUL bytes are errors); `write` never replaces a directory and nothing is
  created beneath a file; `RealVfs` rejects Windows drive / stream /
  device-name segments and removes or renames a symlink itself, not its
  target.
- Shell: `'$HOME'` is literal, `"*"` / `'{a,b}'` are not expanded, `""`
  is an argument, operators inside `$( ... )` no longer split the outer
  line, piped stdin does not leak into the next command, `run` passes
  `$1..$n`, and runaway lines stop at resource limits (100k commands,
  256 KiB expansions, nesting depth) instead of hanging or overflowing
  the stack.
- **Shell exit status (breaking):** `if` / `elif` / `while` / `until`
  decide on the exit status of their condition, not on its printed text
  (`if echo false` is now true), and condition output is shown. `test` /
  the new `[ ... ]` print nothing and only set the status (malformed
  expressions are errors); `grep` exits 1 on no match, `diff` when files
  differ; `! cmd` negates any pipeline.
- Shell expansion: `$VAR` and `$(...)` results are inserted as words
  (split on whitespace when unquoted) and never re-parsed, so quotes, `$`
  or `\` in command output stay literal; globbing follows quoting per
  character; alias text is expanded with the line. `#` comments work on
  typed lines.
- Shell jobs and `xargs`: a lone `&` ends a command list anywhere
  (`a & b`, loops, functions, scripts), with parameters frozen when the
  job is queued. `xargs` is a builtin that actually runs its command
  (`-n N`, `-I REPL`; exit 123 on failure) instead of printing
  "would execute". Nesting is capped at 32 levels (`$(...)`, functions,
  scripts and blocks together; was 96 `execute` levels plus 64 block
  levels), measured to fit a PSP EBOOT's 256 KiB main-thread stack.
- **FTP serves a subtree:** `FtpServer::with_root` exposes one VFS
  directory as `/` (via the new `oasis_vfs::SubtreeVfs`), and the desktop
  shell's `ftp start` serves `/home` -- clients can no longer read
  `/system` or write `/var` IPC request files. Paths clients use change
  accordingly (`/home/x` is now `/x`).

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
  `is_timed_out()`, and `oasis_types::crypto` (incremental `Sha256`,
  `hmac_sha256`, `hex`; used by the remote-terminal PSK handshake).
- `oasis-core`'s status bar, bottom bar and dashboard, `oasis-video`'s
  ffmpeg decoder and `oasis-audio`'s radio sources are split into
  submodules. `DashboardConfig::max_pages` / `cursor_lerp_speed` (never
  read) and `oasis-shader`'s `gl-web` feature (pulled in `web-sys` for
  no code) are removed **(breaking)**.
- Unused dependencies dropped (`oasis-sdi` from seven app crates,
  `serde_json` / `thiserror` / `js-sys` from `oasis-core`, `serde` /
  `brotli` from `oasis-browser`, `log` from four crates, `oasis-shader`
  from `oasis-app`, `rustls-pki-types`); `symphonia`, `criterion`, `glow`
  and `log` use workspace entries. `impl_content_app_methods!` expands to
  `oasis_app_core` paths, so app crates using it need no `oasis-skin`
  dependency.

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
- **(breaking)** `destroy_texture` of an unknown or already-destroyed id
  fails with `BackendError::TextureNotFound` on every backend (SDL, UE5,
  WASM, PSP and the test doubles), like `destroy_render_target`; it used
  to succeed silently. Checked by `conformance::texture_misuse`.
- **(breaking)** `thick_line_rows`, `polygon_rows`, `triangle_rows` and
  `rasterize_triangle` take a `ClipRect` of bounds and generate only the
  visible rows, so a line spanning +-1e9 no longer effectively hangs
  (`ClipRect::UNBOUNDED` keeps the old behaviour). `SdiShapes` now builds
  on `SdiClipTransform`, which gains `draw_bounds()`.
- `stroke_arc` clamps its sweep to one turn like `fill_arc`
  (`backend::clamp_sweep`; WASM arcs too, and negative sweeps run
  anticlockwise there). WASM `blit_tinted` no longer paints the tint into
  the texture's transparent pixels, and `blit_sub_tinted` tints at all.
  SDL gradients and glyph blits report SDL errors instead of dropping
  them. `RecordingBackend` / `ClipAuditBackend` log `set_clip_rect` /
  `reset_clip_rect` as the new `DrawCommand::SetClip` / `ResetClip`
  instead of `PushClip` / `PopClip`.
- `oasis_ui::Theme` gains `button_text`, the label colour on neutral
  button fills (secondary buttons, unfocused modal buttons): derived from
  the skin's `text` with 4.5:1 contrast enforced against the button fill,
  or set with `[widget_states.button] text`. The retro-cga, solarized and
  corrupted buttons are readable now and left the accepted low-contrast
  list.
- `VectorOp::scale` snaps positions as absolute pixels (rect edges rather
  than position + size, through group translations), so vector icons no
  longer show 1 px seams between abutting parts at fractional scales;
  dashboard icon pixels change slightly and the screenshots were
  regenerated.
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
- FFI soundness: a NULL callback passed to `oasis_register_callback` /
  `oasis_set_audio_callback` unregisters it (it used to be stored as a
  non-null Rust function pointer, which is undefined behaviour);
  `oasis_destroy` and a new `oasis_video_play` stop and join the decode
  thread (it used to run on detached); `oasis_video_next_frame` keeps a
  frame that didn't fit the host buffer and still reports its size;
  `oasis_video_is_playing` turns 0 once the stream ended and was drained;
  `oasis_set_vfs_root` re-seeds the dashboard's `/apps`;
  `oasis_add_vfs_file` with NULL data and length 0 creates an empty file.
  The crate docs spell out the full safety contract (NULL handles,
  callback re-entrancy, borrowed / owned pointers, panics).

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
- App fixes from a review pass: File Manager acted on the wrong entry
  when a name contained `"  ("` (it parsed display text; real names are
  kept now), kept the selection in range after deletes / rescans, and
  only falls back to copy + delete when rename is unsupported (deleting
  the source after a full copy); Calculator rejects inf / NaN results,
  bounds unary / power recursion, negates the last number with +/- and
  chains from the exact previous result; Text Editor opens non-UTF-8
  files read-only, keeps CRLF, and commits Save As only after a
  successful write (relative names go to `/home/user`); Paint's
  cancelled shapes leave undo history alone; Games shuffles use the
  PRNG's high bits and Snake may move into its vacating tail; Photo
  Viewer refuses > 40 MP sources before allocating and actually rotates;
  ID3 parsing is shared, v2.4-aware and overflow-safe; TV Guide accepts
  `MM:SS` lengths and rejects inf / NaN; Radio clamps a published volume
  and never overwrites an unparsable `stations.toml`; Settings clamps a
  published font scale and System / About scrolling.
- e2e coverage for the close paths, plugin tiles, terminal scroll,
  skin-swap clocks, music control and window placement across skins.
- Games deals new boards each launch on desktop (seeded from the platform
  clock via `HostInfo::clock`; a frozen `OASIS_FIXED_TIME` clock keeps
  them reproducible), and Memory / Puzzle resets no longer repeat the same
  sequence every session. `GamesApp::new` keeps the fixed seed;
  `GamesApp::with_clock` / `with_seed` choose.
- Bottom bar labels (USB indicator, media tabs) follow skin swaps and the
  clock gaining its date instead of keeping their first position; vector
  dashboard icons left-align in the `"column"` layout like their labels.

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
- **Hostile-input hardening (overhaul 2):** a property-based pipeline
  fuzz test (`oasis-browser/tests/pipeline_fuzz.rs`) found, and the
  engine now survives, CSS that allocated tens of gigabytes
  (`column-count`, huge `colspan` x `rowspan`, `tab-size`, line-box and
  offscreen-layer sizing) or overflowed paint arithmetic (geometry now
  saturates at ±2^25 px); a non-ASCII `bgcolor` / SVG `fill` no longer
  panics; the console buffer is capped (1000 entries, 4 KiB each).
- **Correctness (overhaul 2):** table spans no longer ride on
  `min-width` / `max-width`, so `max-width` of 999 px and up applies
  again (`.container { max-width: 1200px }`) and `min-width` on a cell
  is not read as a `colspan`; `colspan` / `rowspan` follow HTML's limits.
  All 148 CSS named colours are recognised, and SVG / canvas colours
  share the CSS parser (`rgba()`, `hsl()`, 8-digit hex). Relative URLs
  keep a directory's trailing `/` and absolute paths drop dot segments.
  `oasis-browser`'s largest files (`svg`, `paint/record`,
  `widget/pipeline`, `widget/input`) are now module directories.

### Skins

- **One source of truth:** all 17 built-ins are generated from `skins/`
  at build time (`desktop`, `modern`, `corrupted` and the diverged `xp`
  became TOML directories; a snapshot test pins every parsed skin).
  Settings-saved `custom-*` skins are never compiled in, and saving over a
  built-in's name is refused.
- Strict `resolve_skin` (plus `resolve_skin_or_classic` for boot);
  `skin lint` flags unknown shader names and the removed `features.toml`
  `command_categories` key. `from_base_colors` delegates to `from_skin`.
- Derived colors stay readable: primary-button labels on the accent,
  selection text, app body text and bar text are lifted to 4.5:1 when the
  derivation fell short (several skins drew labels at 1.0-1.6:1); colors a
  skin sets explicitly are kept. win95's terminal text is black / navy on
  the gray app surface.
- `skin lint` catches malformed colors anywhere in `theme.toml`, unknown
  layer / anchor / wallpaper / entrance / page-style / easing / density
  names, short `glass_shard` layers, bad shader-param colors and
  non-built-in `inherits` parents; TOML errors name the file and line.
  Every built-in lints clean (`all_builtin_skins_lint_clean`, with an
  explicit list of accepted palette-constrained contrast misses).

### UI toolkit and window manager

- **(breaking)** `oasis-ui` drops `ToastStack` (the shell's
  `oasis_core::toast` is the one toast system), `Avatar`, `IconAtlas` /
  `Icon` with the never-drawn `Button::icon`, and `accessibility` (use
  `oasis_types::color::{contrast_ratio, meets_wcag_aa}`): 27 widgets.
  `focus.rs` became `focus/{ring,manager,spatial,style}`.
- `DrawContext::text_in` ellipsizes text wider than its rect instead of
  drawing outside the widget. A sweep draws every widget at degenerate
  sizes; it found `SpinBox` underflowing when narrower than a step button.
- Widget fixes: `SpatialFocusManager` ignored the `InputEvent::Tab` /
  `ShiftTab` events keyboards send; `Table` sorting moved the selection
  to another row; `Slider` could never reach `max` when the range was not
  a step multiple; circular `ProgressBar` never drew progress; focused
  `InputField`s put the cursor outside the field for long text (and
  password masks counted bytes); flex children left up to n-1 pixels
  unused; tooltips at a screen edge covered their target (now they flip;
  `ContextMenu::show_within` does the same for menus); `MenuBar` panicked
  on a stale `open` index; disabled dropdowns still changed selection.
  New `ListView::scroll_by`, `InputField` cursor movement / `delete`.
- Window manager fixes: right-edge resize on narrow windows resolved to
  the left edge; narrow windows could not be dragged to x = 0; a drag in
  progress undid a tile / maximize / snap / fullscreen on the next mouse
  move; interrupted open / minimize / restore animations jumped; grid
  tiling left the partial last row's cells empty.
- Scene graph / vector / shaders: SDI `z` / `overlay` edits made through
  `get_mut` now reorder the next draw; the vector `AnimClock` keeps f64
  time (animations ran ~2x fast after ~3 days up); matrix-rain wallpaper
  falls instead of rising; the GLSL shaders match the CPU renderer
  (orientation, per-shader defaults, hashes).

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
- Shuffle is a real random order (it was the same fixed interleave every
  time) that starts at the playing track and plays every other track once
  per round; picking, adding or removing a track keeps the rest of the
  round. **(breaking)** `Playlist::shuffle` is private: use `shuffle()` /
  `set_shuffle()`.
- WAV: `WAVE_FORMAT_EXTENSIBLE` PCM plays; short `fmt ` chunks and 0 Hz
  rates are rejected. Ogg decode stops at 2^27 samples. Radio responses
  whose headers never end fail after 16 KiB.
- `demux_lite` (PSP playback, desktop seek points) is hardened against
  hostile MP4s: overflow-checked box ends, a nesting limit, table sizes
  bounded by the bytes in the file, size caps on config boxes and
  samples; per-sample lookups are indexed binary searches (playback was
  quadratic in track length). AAC channel count, `esds` parsing and
  multi-SPS `avcC` are fixed.
- The symphonia video backend reads only `moov` when opening a stream (it
  read the whole file into memory), finds `avcC` only inside `moov`, and
  bounds its per-stream packet queues; the ffmpeg backend no longer leaks
  its AVIO context and buffer on every close.
- ffmpeg backend: frames drained on an `EAGAIN` send are returned in order
  (the frame decoded after the retry used to jump ahead of them), and a
  caller that only pulls audio buffers at most 8 decoded video frames
  instead of every frame of the stream.
- TV streaming: a channel change no longer leaves the decoder thread
  spinning (cancellation was an `Interrupted` read error, which
  `read_exact` retries forever); the download thread no longer panics
  when the window moves past a stalled atom scan; the retained file
  header stops at the incomplete `mdat` instead of re-copying the whole
  buffer on every chunk.
- Radio: an Internet Archive response with a `Location` header is only a
  redirect on 3xx.

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
- The standalone PSP crates (overlay PRX, PRX decrypter, recovery,
  devloop, ME boot, USB client) now have a `SAFETY` comment on every
  `unsafe` block too. Fixed on the way: the ME watchdog's inline hook ran
  a trampoline in a dead stack frame; hook trampolines are cache-flushed
  after moving to their statics; unchecked null function pointers;
  out-of-bounds and non-UTF-8 handling in the PRX decrypter; module-ID
  buffer sizes; a wrapping size parse in recovery.
- TV Guide video texture: the main thread no longer frees (or reuses) the
  CSC target while the video thread may be writing it. The decoder holds
  a hazard pointer around each decode + CSC and re-acquires the buffer
  every frame; the main thread unpublishes, waits up to 150 ms and
  otherwise defers the free. The texture is now 512-stride x
  `ceil16(height)` rows (+1 guard row) sized at tune time for
  `max(272, episode height)`, so 480x272 streams decode into it (the old
  fixed 512x256 buffer was too short: the CSC either errored or, with a
  reused persistent decoder, overran it by 16 rows) and 557 KB is
  allocated instead of padding to 512x512. Frames report the stream's
  own SPS size, and volatile-memory buffers are reused instead of leaking
  one per tune.
- Overlay PRX: the shipped `oasis-overlay.prx` is stripped with
  `prxmin` (293 KB -> 153 KB; the loaded image is identical), and the
  docs give its measured size instead of the never-met "<64 KB".
- The EBOOT no longer links `webpki` 0.22 (unused; it pulled `ring`'s C
  and assembly into the "pure Rust" TLS build) or the getrandom 0.2 / 0.3
  custom backends nothing called.

### CI and tooling

- New `main-ci.yml` steps: an `oasis-app` feature-combination check,
  clippy over the supported non-default feature sets (`oasis-browser`,
  `oasis-core`, `oasis-video`, `oasis-app`, `oasis-ffi`), and
  `scripts/check-rust-psp-rev.sh`, which fails when the rust-psp pin,
  nightly date, toolchain channel or pspdev GCC version drift between
  crates, workflows and `docker/psp-build.Dockerfile`.
- PSP EBOOT, overlay PRX and recovery EBOOT builds deny rustc warnings
  (`PSP_DENY_WARNINGS`).
- `main-ci.yml` holds the stripped overlay PRX to a 192 KiB budget, and
  both workflows run the `scripts/test_*.py` unit tests
  (`psp_remote.py` screencap decoding).

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
