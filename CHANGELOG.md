# Changelog

Notable changes to OASIS_OS. GitHub releases (tags `v1.1.0` ... `v1.5.0`)
carry generated notes listing every merged PR; this file summarizes the
larger changes by area. Breaking API changes are marked **(breaking)**.

## Unreleased -- overhaul 3

### Browser (HTML)

- The HTML parser is about 3x faster on typical pages (full parse of a 100 KB page 5.2 ms to
  1.4 ms; table-heavy pages 7.3 ms to 3.5 ms): text runs are one token instead of one per
  character, and the tree builder no longer allocates per tag. The tree builder alone is back
  below its pre-overhaul time.
- `<template>` contents parse with the spec's "in template" mode: `<template><tr><td>`
  (the usual row template for scripts) keeps its rows and cells instead of dropping the tags.
- Quirks mode: the DOCTYPE's public / system identifiers pick quirks, limited-quirks or
  standards mode (new `Document::quirks_mode`); in quirks mode (no DOCTYPE, or a legacy one)
  `<table>` no longer closes an open `<p>`, as in browsers. `DoctypeToken` gains `public_id`
  and `system_id` **(breaking)**.
- `--!>` ends a comment; text in `<textarea>`, `<xmp>`, `<iframe>`, `<noembed>` no longer
  gets bold / italic reopened inside it; misnested formatting around a table
  (`<table><a>1<p>2</a>3`) lands before the table instead of inside it.

### Shell chrome / skins

- Skins with `dashboard = false` (`desktop`, `corrupted`) no longer draw the dashboard icon
  grid (it showed in Dashboard mode and behind windows), and its hidden icons no longer
  take clicks, hover or d-pad launches. Their bottom bar shows no page dots either. A
  terminal-only skin (no dashboard and no window manager: `corrupted`) boots into, and
  returns to, its fullscreen terminal instead of an empty dashboard.
- New windows open below the skin's top chrome: hosts pass the status bar + tab row and
  bottom bar heights (`oasis_core::apps::chrome_insets`) to the new
  `WindowManager::set_placement_insets`, so psix-hifi's windows no longer cover its top tab
  row. A cascaded window taller than the screen below the top chrome is shortened to fit.
- The start button is centred vertically in the bottom bar (it was pinned 3 px below the
  bar's top: macOS's 52 px dock showed it at the top, and on retro-cga / paper it hung
  past the bar's bottom edge).
- `[icon_overrides] label_shadow_color` sets the icon-label drop shadow; retro-cga,
  balatro and solarized use it so labels read on their bright wallpapers / shader.
- Screenshots: the `desktop` skin's Terminal window shows its text (a later window's
  focus buried it under the content background) and the media-tab captures keep the start
  button, as the shell does.

### Browser (CSS)

- The user-agent sheet is its own cascade origin: author rules in any `@layer` now beat it
  (layered pages such as Tailwind v4 lost to UA `margin` / `font-size` / `display`
  defaults), presentational hints beat it (`cellpadding="0"` now removes the UA cell
  padding), and important UA declarations beat important author ones.
- `revert-layer` rolls back to the previous cascade layer (it was ignored), or acts as
  `unset` when there is none.
- Custom properties honour CSS-wide keywords: `--x: initial` is the guaranteed-invalid
  value (the "space toggle": `var(--x, fallback)` uses the fallback), `inherit` / `unset`
  take the parent's value (they used to be stored as the literal text).
- `:is()` / `:where()` / `:not()` accept complex selectors (`:where(.prose > p)`,
  `:not(.a .b)`, Tailwind Typography's `:not(:where([class~=not-prose] *))`); they took only
  compound selectors and misparsed the rest of the selector. `:is()` / `:where()` drop
  invalid entries, an invalid `:not()` entry matches nothing, and `&` inside them works in
  nested rules. `SimpleSelector::{Is, Where, Not}` now hold `Vec<Selector>` **(breaking)**.
- Four-side shorthands (`margin`, `padding`, `border-width` / `-style` / `-color`) with
  more than four values are dropped as invalid instead of using the first four.

### Browser (layout)

- Flex layout is linear in nesting depth: content-sized items take their max-content width
  from the box tree (cached per layout pass) instead of a measuring layout, and each row item
  is laid out once. Sixteen nested `flex-grow` rows went from 3.2 ms to 0.13 ms.
- Flex sizing fixes: an item with block children sizes to its content (it measured as the
  full container width, so `justify-content` and siblings had no room); a definite
  `flex-basis` is the item's size, not its containing width (margins and padding were taken
  out of it, a 40 px avatar with a 12 px margin drew 37 px wide with no margin); column
  containers with a definite height honour `flex-basis`. Flex and grid no longer rewrite the
  item's `width` / `height` style.
- Relaying a page out (after images decode) gives exactly the layout of a fresh load, checked
  over the whole corpus: inline layout keeps its source content, so words of a wrapped line
  no longer glue together and `<pre>` lines no longer merge on a second layout. The partial
  dirty-subtree relayout, which skipped floats and left clean subtrees in place, is gone:
  `layout_block_incremental` and `StyleCache` are replaced by `relayout_tree` **(breaking)**.
- Table cells lay their content out as a block flow (multiple block children stacked on top
  of each other, and the cell padding was counted twice); whitespace between table tags no
  longer draws as spaces when a table sits in inline content.
- Canvas: past the command cap new commands are dropped (`CanvasState::overflowed`), instead
  of the oldest half of the drawing. Web-font glyph textures are evicted least recently used
  instead of all at once when the cache fills. `FontAwareTextMeasurer::set_font_id` is
  removed (layout passes the run's font) **(breaking)**.

### Apps

- Text Editor: undoing (or redoing) back to the text as last opened or saved clears the
  `[Modified]` marker, so closing no longer asks to save an unchanged file. Reopening Find /
  Replace shows the previous query selected (`Find: [query]`): typing replaces it and
  Backspace clears it, instead of appending to the old search.
- `tv tune <ch>` reports "the TV Guide is not open" instead of "Tuning to ..." when no guide
  can act on it: the shell publishes the guide's state to `/var/tv/status`
  (`oasis_types::ipc::TvStatus`).
- System Monitor says "Stale - the host stopped updating" once the host's snapshot stops
  advancing for 5 s, instead of "Live" over frozen values (`SystemMonitorApp::is_stale`).
- Settings shows the shell-published volume / font scale / reduced motion once its request
  was consumed, so a change the shell rejected or clamped no longer stays on screen.
- Labels fit what the font draws at small window sizes and large-font skins: File Manager
  tile names (shrunk to 8 px, then ellipsized) and its address bar (keeps the path's tail),
  System Monitor's info column, Internet Radio's favourite mark, Video Embed result cells.
  `oasis-apps/tests/text_fits_window.rs` now also fails on overlapping text runs.

### Crate APIs (oasis-audio, oasis-video, oasis-net, oasis-mcp)

- Remote terminal client: `RemoteClient` answers exactly one `AUTH_REQUIRED` challenge per
  connection, and only a nonce of the documented form (32 lowercase hex digits; new
  `oasis_net::is_valid_nonce` / `NONCE_HEX_LEN`), so a hostile server can no longer use it
  as an HMAC oracle over chosen text. The nonce is not bound to the server's identity; that
  relay limitation is now documented in `docs/networking.md` and `docs/security.md`.
- `oasis-audio`: only the crate-root re-exports are public. `oasis_audio::{sfx, wav, ogg}::X`
  is `oasis_audio::X` (`SfxPlayer`, `decode_wav`, `WavData`, `decode_ogg`, `OggData`, ...),
  `oasis_audio::radio::{archive, source, station, icy, threaded, connect, buffer}::X` is
  `oasis_audio::radio::X`, and `mixer` / `playlist` / `manager` / `controller` / `types` /
  `null_backend` are no longer public modules **(breaking)**.
- `oasis-video`: `SoftwareVideoDecoder` forwards to a crate-private `DecoderBackend` trait
  that both backends implement. The `aac`, `demux`, `h264` and `ffmpeg_decoder` modules and
  `demux_lite::avcc` are private; removed the `AvccConfig` / `find_avcc_in_mp4` re-export
  (symphonia's, distinct from `demux_lite::AvccConfig`),
  `SoftwareVideoDecoder::open_stream_with_avcc` and `demux_lite::seek_byte_from_moov`
  (use `seek_point_from_moov(..).map(|p| p.min_byte)`) **(breaking)**.
- `oasis-net`: `ftp::process_ftp_request` is crate-private **(breaking)**.
- `oasis-audio`, `oasis-video`, `oasis-net` and `oasis-mcp` warn on undocumented public items.

### UI

- `MenuBar` lays labels and drop-downs out with the backend's `measure_text` (recorded at
  draw) instead of 7 px per character, and fits narrow bars: label padding shrinks, then
  trailing menus fold into a `»` overflow menu (`MenuHit::Label(menus.len())`); drop-downs
  stay inside the bar, dropping shortcut hints and ellipsizing labels.
  `draw_dropdown` and `pointer_move` take the bar rect `(x, y, w, h)` **(breaking)**.
- `TreeView` connector lines run through the rows of a node's children when the node has a
  later sibling (the guide used to break across the subtree).

### Terminal / shell

- Background jobs (`cmd &`) and cron jobs run detached from the session, like a POSIX
  subshell: their exit status and `cd`s no longer overwrite the interactive `$?`, working
  directory, `$OLDPWD` or `$CWD` (`CommandRegistry::run_detached`).
- `cron` keeps each job on its first schedule however coarsely the host ticks (an FFI host
  ticking with > 1 s deltas ran jobs slowly, rescheduling from each tick); after a long
  sleep an overdue job still runs once and resumes at its next slot, with no burst.
- `${...}` finds its closing `}` past quoted or escaped braces (`${X:-"}"}`), and inside
  double quotes the word is double-quoted text (`"${X:-it's}"` used to be a literal `$`).
- `NAME=value` assigns a shell variable (no field splitting: `C=$B` keeps the spaces), and
  before a command (`P=temp myfunc`) holds only while it runs; it used to be "unknown
  command".
- A failing pipeline stage before the last no longer aborts the pipeline: its error is
  shown and the next stage reads empty input (`cat missing | wc -l` prints `0` and
  succeeds), as in a POSIX shell.

### Input & widgets

- App menu bars open from the keyboard: F10 opens the first menu, Left / Right switch
  menus (the `»` overflow included), Up / Down / Home / End pick an item, Enter / Space run
  it, Esc / F10 close (`MenuBar::handle_key` -> `MenuKey`). Text Editor, Paint and File
  Manager route keys through it (Alt belongs to the window manager); WASM keeps the
  browser from taking F10.
- The SDL backend honours the OS "natural scrolling" setting: SDL3 already reports wheel
  motion in the user's direction, and negating `Flipped` events undid it.
- Text Editor clicks land on the glyph drawn under the pointer under a TTF skin: hit-testing
  uses the advances the backend measured while drawing instead of the bitmap font's.
- `RichText` aligns mixed-size spans on a common baseline (they were top-aligned).

### Build, features & dependencies

- Feature cleanup: `oasis-js` drops `rquickjs-engine` (rquickjs is always linked; the
  feature could not be turned off), `oasis-core` drops the unused `test-data`, and
  `oasis-app-tv-guide` drops `test-data` (its fixtures are now test-only) **(breaking)**.
  `oasis-shader/gl-native` no longer also exposes an implicit `glow` feature.
- `oasis-app` now enables `oasis-core/skin-ttf`, so `skin lint` in the desktop shell parses
  skin fonts as documented (the workspace dependency entry had turned it off).
- Every crate with features documents them under `# Feature flags` in its crate docs.
- Releases: a `v*` tag whose version differs from the Cargo workspace version (or the shipped
  PSP crates') now fails `create-release` (`scripts/check-release-version.py`).

### Shell chrome polish

- Chip-style dashboard icons (`icon_container = "chip"`: macOS, GNOME, Modern, Paper,
  Solarized, Vaporwave, Retro CGA, Balatro) fit the glyph inside the chip body below the
  accent band; tall glyphs (Radio's and TV Guide's antennas) used to cover the band and
  poke out of the chip, and labels now start below the chip instead of on its edge.
- The bottom bar's `USB` label keeps a gap from the URL plate's bezel instead of touching it.
- Desktop-mode taskbar buttons no longer cover the bottom bar's URL plate / USB label or the
  media dock's transport pills: the bar reports the span it leaves free
  (`BottomBar::taskbar_span`, `reserve_taskbar`) and `Taskbar::update_sdi_in` lays the
  buttons out in it (`update_sdi` keeps the old placement for the PSP / WASM hosts).
- Start-button labels are centred by their real (proportional) width; new
  `button_shape = "bevel"` (Win9x raised 3D edges, sunken while open), used by `win95`.
- `oasis-screenshot`: window-manager-only skins (`desktop`) show the bottom bar, start
  button, clock and taskbar window buttons like the shell does; the windowed terminal shot
  of desktop skins keeps the bars up and fits the window between them.
- SDL shader wallpaper: `SdlShaderBridge::render_and_blit` / `would_shade` take the clock
  time as unwrapped `f64` seconds and wrap it in `f64` before the shader sees it
  (`shader_bridge::shader_time`), so the 30 Hz throttle and the animation stay smooth after
  days of uptime **(breaking)**.
- Site: the demo page's top bar no longer overflows at tablet widths, and the 404 page has
  a meta description and social card. Regenerated skin screenshots.

### Cross-lane review fixes

- `rm -r` (and the File Manager's delete) removes a symlink to a directory as a link; it
  used to empty the link's target on `RealVfs`. New provided `Vfs::is_symlink`.
- Browser: the anonymous flex / grid items around inline children no longer copy the
  container's `position`, `opacity`, `transform`, background image, ...; an inline child
  is blockified, so its own `margin-left: auto`, padding and width apply to its item.
  Reader mode keeps its stylesheet on pages with a `style-src` CSP.
- `<title>` / `<textarea>` map `&#150;`-style references through Windows-1252 too; merged
  late `<body>` / `<html>` attributes are capped like a tag's.
- TLS `close` drains ciphertext a full socket held back (a write right before close could
  lose its tail).
- Paint: a canvas wider than the window starts at the view's edge (its first columns were
  unreachable). `set X = 5` works again; `tail -n +N` is an error instead of "last N";
  `head -- -5` reads the file `-5`.
- PSP: clip rects are clamped to the screen before `sceGuScissor` (an empty clip drew
  everywhere, a clip left of / above the screen hid everything); the WiFi dialog's
  success path re-opens the GU frame.

### Test coverage (bugs the new tests found)

- Text Editor: opening a stylesheet with a `/` outside a comment (`font: 12px/1.5`,
  `grid-area: 1 / 2`) froze the editor (the CSS highlighter stopped advancing); the HTML
  highlighter panicked on non-ASCII text near a `<!--` check and split characters inside tags.
- Browser SVG: `transform` (on `<g>` or a shape) now scales sizes, not just positions:
  under `scale(0.5)` circles, rects, stroke widths and text drew at full size. A non-uniform
  scale makes a circle an ellipse; a rotated / skewed rect becomes a polygon.
- CSS: a negative `font-size` (`-1`, `-5px`) or an overflowing one (`1e400px`) is dropped
  as invalid instead of computing a negative / infinite size.
- Video Embed (WASM YouTube app): its search hints and footer are ellipsized to the window
  (they ran past a 320 px window, failing `cargo test --workspace`).
- ID3 titles / artists made only of NULs and spaces read as no title (were a NUL).
- New tests: CSS keyword tables through the real parser, SVG from real markup (groups,
  `<defs>` gradients, `url(#id)` paint, polygons, `<tspan>`), TV Guide catalog fetch
  outcomes (error, empty, dead thread, timeout, success) and a rejected-resolution and a
  chained `skin xp; skin @variant:dark` e2e flow; `docs/testing.md` gains coverage, hang-guard
  and sweep guidance.

### Skins, vector graphics & shaders

- Skin inheritance (`inherits = "<parent>"`) now fills *every* field the child omits: the nine
  base colors, `text_scale` and each `strings.toml` entry came from the built-in defaults
  instead of the parent, and `density`, `[elevation]` and `[background_performance]` were
  never inherited. Keys the child sets still win, even when equal to the default.
- Derived Dark / Light / High-contrast variants keep `text_scale`, `density`, typography,
  sounds, animation timing and transition style, and no longer carry palette-tuned colors
  (`titlebar_text_active/inactive`, icon `data_led_color` / `fallback_colors`, the start
  menu's `item_fallback_color`, the transition fade color, cursor fill / outline). Contrast
  repair moves text toward the luminance pole, fixing darkened text on saturated blues.
- `skin lint` additions: `url_text` contrast against its chrome bezel, `text_scale` range,
  lone / zero desktop size, `font_size = 0` text objects, `success` / `warning` hex values.
  The lint no longer overflows on absurd sizes; huge `fade_ms` / `slide_ms` no longer
  overflow the frame conversion; a NaN `text_scale` / sound `volume` falls back to 1.0.
- Skins: readable bottom-bar URL labels in `altimit` and `balatro`, legible `solarized` icon
  labels on the wave wallpaper, a visible white bar edge in `win95`.
- `oasis-skin` build script: numbered loader functions and escaped string literals (odd
  directory / asset names can no longer break the generated code), cargo warnings for
  incomplete skin directories, and no rewrite when the output is unchanged.
- `oasis-vector`: `AnimClock::wrapped(rate, period)`; drifting / wave layers and the
  frame-counter oscillators keep full precision after days of uptime; layer factories and
  `rotate_point` / `rotate_rect` saturate instead of overflowing on extreme parameters (a
  spacing above `i32::MAX` used to loop forever).
- `oasis-shader`: shader time wraps every hour (`TIME_WRAP_S`, `wrap_time`) in both the CPU
  and GL paths, zero-sized outputs are handled, and the nearest-neighbour upscale copies
  whole rows. `SoftwareShaderRenderer::render_balatro` is no longer public; use
  `render_builtin(BuiltinShader::Balatro, ..)` **(breaking)**.

### Built-in apps

- Text Editor: long lines scroll horizontally to keep the caret visible; clicks place the
  caret at the nearest character (proportional font); Up / Down keep a character goal
  column across short lines and multi-byte text; PageDown stops at the last full page.
- Calculator: tiny and huge results show in scientific notation (`0-0.00000000004` read
  `-0`), which the evaluator now accepts; a new operator replaces the whole trailing
  operator run (`5*-` then `+` is `5+`); Ans / MR replace the number being typed (`2` then
  Ans gave `24`); history entries the evaluator rejects are skipped on load.
- File Manager: switching Dual -> Explorer after scrolling no longer underflows the cursor;
  Dual-view Down keeps the selection on a drawn row; an empty folder says `0 object(s)`.
- Music Player / Photo Viewer: files whose names contain `  (` open (the name was parsed
  back out of the row text); a track opened by hand sets the playlist position; leaving
  the viewer re-selects the file instead of jumping to the top.
- Paint: Confirm lifts the Pencil / Eraser again (a gamepad could not stop painting);
  canvases bigger than the window are clipped and pan with the cursor instead of running
  under the palette; a fill with the colour already there is not an undo step.
- Snake: two turns within one move no longer reverse the snake into itself; restarting
  mid-game keeps a new high score. TV Guide: the open guide follows the wall clock.
- Every app's text stays inside its window from 320x200 to 1024x768 under large-font skins
  (footers, status strips, list rows and headers are ellipsized by pixel width).
- `oasis-app-core`: `file_viewer::list_directory_entries` (rows plus the entry behind
  each), `ContentState::select` / `selected`, `render::ellipsize`,
  `render::draw_status_strip`. **(breaking)** `AppAction::SwitchToTerminal` is removed
  (no app returned it).

### Follow-up fixes (security, browser, media, UI)

- **(breaking)** The PSP dev servers on TCP 9293 now require the remote terminal's PSK
  HMAC challenge and are off unless a key is configured: the EBOOT command server needs
  `ms0:/PSP/GAME/OASISOS/cmdserver.key` (8+ bytes; `scripts/psp_remote.py` answers from
  `PSP_PSK`), the overlay PRX devloop needs `devloop_psk` next to `devloop = true` in
  `oasis.ini`. See [networking.md § PSP dev servers](docs/networking.md#psp-dev-servers).
  `oasis_net::verify_auth_line` is the shared check.
- Browser: `fetch()` on PSP now resolves DNS names for the private-network check and
  checks the address it dials (`TlsProvider::resolve_host` / `connect_tcp_checked`); a
  wrapper a script holds for a freed node can no longer reach the node that reuses its
  slot; script-written DOM text / attributes are capped (64 MiB, 4 MiB on PSP); text in
  an `@font-face` font is laid out with that font's widths; `:hover` / `:focus` restyle
  descendants and cascade parents first.
- Media: `demux_lite` sample offsets no longer sum a whole chunk per lookup (hostile
  files made playback quadratic); the oasis-video fuzz targets run nightly; SDL
  `feed_pcm_f32` queues past the high-water mark instead of dropping whole chunks; the UE5
  backend buffers streamed radio bytes for the host (`STREAM_*` audio events,
  `Ue5AudioBackend::drain_stream`); the WASM YouTube grid no longer leaks a callback per
  thumbnail and its glyph cache evicts in O(log n).
- `oasis_types::ipc` gains typed `RadioRequest` / `TvRequest` (wire format unchanged).
- UI / WM: tiling survives huge gaps and margins; `Modal` fits its wrapped body;
  `ContextMenu` draws and hit-tests the open submenu; `RichText` lines grow with their
  largest span; `Spinner::measure` matches what it draws. The overlay PRX keeps its menu
  open when a status message arrives.

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

### Browser parsers (oasis-html, oasis-css)

- HTML tree construction follows WHATWG for tables and forms: new "in caption", "in column
  group", "in select", "in select in table" and "in head noscript" modes (rows after a
  `<caption>` or `<colgroup>` used to nest inside it), formatting-list markers so bold /
  italic no longer leak into or out of table cells, spec foster parenting, and the full
  in-body start / end tag tables (`<button>`, `<li>`, `<option>`, `<wbr>`, `<embed>`, leading
  newline after `<pre>` / `<textarea>`, late `<body>` attributes merged, `<base>` and
  post-`</head>` scripts stay in the head).
- Inline SVG / MathML: `<foreignObject>`, `<desc>`, `<title>` and HTML `annotation-xml`
  content stays inside the SVG / MathML instead of breaking out of it; CDATA sections work
  in SVG. Script-data escape states keep `<!-- document.write("<script>…</script>") -->`
  in one script.
- `<noscript>` content renders as markup in builds without JavaScript (it was raw text).
- Named character references cover all of HTML 4 plus common HTML5 names (`&check;`,
  `&lang;`, `&lpar;`, ...); without a `;` only the legacy Latin-1 names match, as the longest
  prefix (`&copy2024` is `©2024`; `&mdash` without `;` now stays literal, as in browsers).
  An unknown name with a `;` stays literal (`&centerdot;` is not `¢erdot;`).
- New `TreeBuilder::parse` / `parse_with` / `parse_reuse` and `ParseOptions { scripting }`:
  the tree builder drives the tokenizer, as the spec does; `Tokenizer::with_scripting`,
  `TreeBuilder::build_with` for the pre-tokenized path.
- **(breaking)** `TagName::is_formatting` is the spec set (adds `code`, `big`, `font`,
  `nobr`, `strike`, `tt`; drops `mark`, `sub`, `sup`, which no longer reopen in the next
  paragraph); `is_void` / `is_special` cover the obsolete and unnamed void / special tags.
- CSS cascade layers order as a tree (sublayers before their parent, late sublayers in their
  parent's slot) **(breaking: `Stylesheet::layers` is now in cascade order)**; `inherit`
  works for non-inherited properties and on shorthands; `all: unset` (the common button
  reset) is supported, and `revert` rolls a property back to the user-agent value
  (`* { all: unset; display: revert }` keeps blocks blocks); the `flex` shorthand handles every
  form (`flex: auto`, `flex: 2 1 0`, `flex: 100px`); `::before` / `::after` see their own
  custom properties.
- Attribute selectors accept the `i` / `s` flags (the flag used to drop the rule) and match
  HTML's enumerated attributes case-insensitively **(breaking: `SimpleSelector::Attribute`
  gains `case_insensitive`)**; malformed attribute selectors match nothing.

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
  heads are skipped instead of becoming the response; a response slower
  than 16 KiB/s is cut off after about 60 s; every resolved address is tried. A guarded (`fetch()`)
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

- Counts re-verified against the source: `SdiCore` has 12 required methods
  and the eight extension traits carry 45 default methods (`SdiClipTransform`
  has 9 since it gained `draw_bounds`; `SdiTextures` lost two, see the backend
  section) across CLAUDE.md, README, design and ADR-003; 21 shell builtins (was 20 in
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

### UI toolkit and window manager (oasis-ui, oasis-wm)

- Window manager: dragging a window that was snapped and then maximized
  now un-maximizes it (it slid around still marked maximized); titlebar
  buttons under a modal no longer show hover feedback; geometry, drag,
  resize, snapping and animation maths no longer overflow (debug panic)
  or lose precision for windows and pointers far out in the `i32` range.
- oasis-ui widgets tolerate stale state in their pub fields: Accordion,
  ContextMenu, Modal, RadioGroup and Dropdown share one wrapping
  navigation rule (`focus::step_wrapping`), fixing an out-of-bounds panic
  and an infinite loop in Accordion and a highlighted unselectable
  ContextMenu item; TreeView focus no longer gets stuck on a collapsed-away
  node; ListView / Table clamp stale scroll and selection; DatePicker,
  Slider, SpinBox and SplitPane no longer panic on out-of-range months or
  swapped / NaN ranges; TabBar Filled / Pill no longer underflow on short
  rects.
- Layout fixes: `FlexLayout` cross-axis alignment works (children get an
  optional `FlexChild::cross_size`; it was dead code); `MeasureCache`
  evicts entries unused for a generation; `Slider::measure` stays within
  the available width; `TextBlock` draws with its `max_lines` and
  `align`; TreeView connector lines no longer break between siblings;
  focused empty `InputField`s show a caret.
- `ListView::index_at` takes the viewport height; removed the uncalled
  `DrawContext` helpers (`panel`, `heading`, `label_styled`,
  `divider_h` / `divider_v`, `button_bg`, `accent_button_bg`, `blit`,
  `measure_text`, `measure_text_sized`, `padded_rect`),
  `layout::center_text_y`, `InputField::selection` and
  `MenuBar::dropdown_dimensions` **(breaking)**.

### PSP

- EBOOT: a frame whose GU commands outgrow the 1 MB display list now drops the excess draws
  (logged once as `[GU] display list full` in `eboot.log`) instead of writing past the buffer;
  rust-psp's `sceGuGetMemory` never fails, so the old null checks never fired. Tall rounded
  boxes and long text lines no longer emit off-screen geometry.
- EBOOT: textures and the font atlas are written back from the data cache before the GE samples
  them (small textures could show stale texels on hardware); `reinit_gu_frame` only re-opens the
  display list after a utility dialog really ran, so Radio / Browser / TV Confirm with WLAN off
  no longer issues a second `sceGuStart` (hang on hardware).
- EBOOT: JavaScript `Date` / `performance.now()` use pspdev newlib's real layouts (64-bit
  `time_t`, 36-byte `struct tm`); the shim's wider `struct tm` overwrote the caller's stack.
  Audio commands from the main, video and I/O threads no longer race on the single-producer
  queue. Malformed MP4 boxes, non-ASCII HTTP headers and Range-ignoring servers no longer hang
  or panic the I/O thread or corrupt a resumed stream; interlaced H.264 reports its full height.
- Overlay PRX: one bad MP3 frame after the first ~100 no longer ends the track / radio stream;
  a closed radio connection reconnects instead of stalling; "toggle radio" takes effect
  mid-track; `music_dir` / `video_dir` without a trailing `/` work; ME buffers are 64-byte
  aligned; framebuffers with a stride under 480 are skipped. Stripped PRX: 156,546 B.
- CI builds the seven standalone PSP crates (`oasis-me-boot`, `oasis-devloop-psp`,
  `oasis-prx-decrypt-psp`, the four `oasis-usb-*-psp`) with warnings denied; all build at the
  pinned rust-psp revision. Releases ship `oasis-me-boot.prx` (the EBOOT loads it from its GAME
  directory for hardware H.264 decode), and the release notes install the overlay as
  `ms0:/seplugins/oasis_plugin.prx`, the name `plugin status` / `plugin remove` expect.
- **(breaking)** Recovery EBOOT: the WiFi file server on TCP 9293 (upload / readfile / delete /
  reboot) now requires the same PSK challenge as the EBOOT command server
  (`oasis_net::verify_auth_line`; `scripts/psp_remote.py` with `PSP_PSK`). The key comes from
  `ms0:/PSP/SAVEDATA/ARK_01234/oasis-recovery.key` (beside `RECOVERY.PBP`, so a wiped or broken
  shell install does not lock recovery out), else the shell's `cmdserver.key` read as a plain
  file; with neither, every connection gets `AUTH_FAIL no key`.
- EBOOT radio: the body bytes that arrive with the HTTP headers go through the ICY demuxer, so
  stream-title metadata no longer desyncs (decoded as audio, audio swallowed as metadata) on
  stations that send metadata.
- EBOOT and WASM: the taskbar is laid out in the bottom bar's free span
  (`BottomBar::taskbar_span` + `Taskbar::update_sdi_in`), as on the desktop shell, so window
  buttons no longer overlap the URL plate, USB label or media dock.

### FFI (oasis-ffi, oasis-backend-ue5)

All additions are new symbols; existing exports keep their signatures.

- `oasis_audio_drain_stream` / `oasis_audio_stream_buffered`: C exports for streamed (internet
  radio) audio bytes, which were only reachable through the Rust-side `drain_stream`.
- `oasis_register_callback_ex` / `oasis_set_audio_callback_ex`: callbacks that also receive a
  host `void* user_data` (passed back unchanged), so a C++ host can route events to its object.
- `oasis_video_audio_format`: the sample rate and channel count of `oasis_video_get_audio`'s PCM.
- The video decode thread is paced to the wall clock: frames become available at their
  presentation time and audio is decoded ~0.5 s ahead, instead of decoding flat out (which
  overflowed the audio ring and dropped chunks before a per-frame poll could take them).

### Backend traits and desktop / web / UE5 backends

- `SdiTextures::blit_sub_tinted` and `blit_flipped` are removed: nothing
  called them (45 extension defaults remain) **(breaking)**.
- `SdiBatch` now builds on `SdiText`, and `submit_text_batch` defaults to
  one `draw_text_styled` per item; SDL and UE5 drop their identical
  overrides. WASM text batches draw real bold / italic glyphs instead of
  double-striking bold, so batched and unbatched text look the same
  (`conformance::batch_matches_singles` pins this) **(breaking for
  implementors that bound `SdiBatch` without `SdiText`)**.
- `SdiCore::measure_text` is a provided method (the bitmap font); 12
  methods are required now.
- `ClipStack::push` returns the `ClipRect` to apply (`ClipRect::EMPTY`
  when nested clips do not overlap) and `ClipStack::push_translated` does
  the translate-then-push every backend repeated; `ClipPush` is removed
  **(breaking)**.
- `backend::unsupported(op)` builds `BackendError::Unsupported`; the
  `AudioBackend` streaming defaults now return it instead of a free-text
  error. `texture_not_found` takes a `TextureId` **(breaking)**.
- Typed IPC: `ipc::BrowserRequest` (`browse` -> the shell) and
  `ipc::Notification` / `NotifyLevel` (`notify` -> toasts) replace the
  hand-rolled string parsing on both sides; an unknown browse request no
  longer opens the browser.
- SDL: volume is the audio stream's gain, so a change applies at once
  (it was baked into queued PCM: seconds late on radio, never on a fully
  queued track); WAV / Ogg tracks no longer play into a stream opened
  for another format (a mono 44.1 kHz WAV after an MP3 played at double
  speed); a multi-character `TextInput` (IME commit) delivers every
  character, not the first; touchpad wheel fractions accumulate into
  steps and "natural scrolling" devices scroll the right way; a window
  resize while a render target is bound resets the window's clip and
  translation; `fill_rect_gradient` generates only the visible rows and
  columns. `lib.rs` is split (`render_target.rs`, `tests.rs`).
- UE5: rect batches translate with saturation like every other draw, so
  a far off-screen rect no longer wraps onto the screen.
- WASM: releasing a drag outside the canvas ends it; a sideways scroll
  no longer scrolls up; zero / oversized stroke widths match the trait
  defaults (Canvas reused the previous width, or `arcTo` threw); reading
  a render target tainted by a TV frame returns black instead of failing;
  stopping the TV video or a radio stream removes the element's listeners
  first and ends its fetch (late events used to call dropped closures and
  throw); a failed TV start no longer leaves a `<video>` in the page; URL
  log truncation is char-boundary safe.

### Terminal (oasis-terminal)

- One option parser (`oasis_terminal::parse_flags` / `FlagSpec`) for `ls`, `rm`, `mkdir`,
  `grep`, `sort`, `uniq`, `wc`, `head`, `tail`, `tee`, `tr`, `cut`, `base64` and `xxd`:
  combined short options (`-la`, `grep -in`), `-n5` / `-n 5`, `--`, options after operands,
  and an error with the usage line for an unknown option **(breaking: `-x` used to be taken
  as a file name or pattern; use `--` for operands that start with `-`)**. New options:
  `ls -a` / `-l` (plain `ls` now hides dotfiles), `rm -r` / `-f`, `mkdir -p`, `grep -q`,
  `sort -u`, `head -N` / `tail -N`.
- `grep` with no match, `ls` of an empty directory and `find` with no hit print nothing
  (they printed `(no matches)` / `(empty)`, which pipes and `$(...)` then saw). The shell's own
  colour codes (`ls` / `tree` directories) are stripped from text reaching a pipe, redirect
  or `$(...)`; other escape sequences pass through (`cat a.ans > b.ans` copies the bytes).
- `case`: quoted patterns, `$VAR` patterns and quoted `|` work (`case "$X" in "hello")`
  never matched); the case word is not split or globbed; `case foo.in in` finds the right
  `in`. A `case` with no matching arm and a `for` over nothing exit 0.
- Compound commands take a trailing pipe, redirect or `&&` / `||` list
  (`for ...; done | sort > out`, `fi >> log`; the output used to escape to the terminal and
  `> out` created an empty file).
- Redirection: `>/dev/null` and `2>/dev/null` discard (they failed the command), `>&2`,
  `&>` / `&>>` and `1>` are understood, and a target is one word (`echo a > f b` writes
  `a b`).
- Expansion: `$@` / `$*` and `"$@"` (one word per argument), `for VAR; do` over the
  arguments, single-digit positionals (`$1x` is `${1}x`), `${V:-def}`, `${V-def}`,
  `${V:=def}`, `${#V}`, `bad substitution` for other `${...}` forms, and `~` / `~/path`
  tilde expansion. Globs no longer match dotfiles unless the pattern starts with `.`.
- `cd` with no argument goes to `$HOME` (was `/`), `cd -` to `$OLDPWD`; extra arguments
  are an error.
- Aliases whose text has `|`, `;`, `&`, `<` or `>` work (`alias ll='ls | sort'`).
- `set A=1 B=2` sets both and keeps a quoted value's spaces; variable names are validated
  (`set 1=x` is an error); `local X` without a value shadows the caller's `X`.
- `tr` sets take ranges (`tr a-z A-Z`); `find` patterns are globs (`find / '*.txt'`; a
  pattern without glob characters still matches as a substring).
- Tab completion no longer panics on non-ASCII names, escapes spaces / metacharacters in
  completed paths, and completes inside quotes (`cat "my f<Tab>`); an operator inside
  quotes no longer starts a new command.
- Multi-line history entries survive a save / reload (`\` and newlines are escaped in
  `.oasis_history`; an old file's literal `\n` now reads as a newline).
- **(breaking)** `Completer` is stateless: the unused cycling API (`cycle_next`, `reset`,
  `is_cycling`), the always-empty `aliases` / `functions` parameters of `complete` and
  `CompletionContext::Subcommand` are gone. The test-only `control_flow` module and
  `CommandRegistry::completions` are no longer public.

### Scripting: cron jobs actually run

- `cron` used to store "scheduled scripts" that nothing ever ran. Jobs now have an interval
  and run: `cron add <name> <interval> <script>` (`30s`, `5m`, `2h`, `1d`, bare number =
  seconds; 1 s to 30 d; a relative script path is resolved against the current directory),
  `cron list` shows each job's interval. The old two-argument
  `cron add <name> <script>` is rejected, and existing interval-less entries are listed as
  invalid until re-added **(breaking)**.
- `oasis_core::script::CronScheduler` runs due jobs when a host calls `tick(now, ...)`: the
  desktop shell does about once a second (output in the terminal, prefixed
  `[cron <name>]`), `oasis-ffi` on every `oasis_tick`. A job first runs one interval after
  the host first sees it, starts in `/` without moving the shell's cwd, and an overdue job
  runs once. The cron code moved to `oasis_core::script::cron`.

### FFI (oasis-ffi)

- A callback that calls back into the API with the handle that fired it is now refused (the
  call returns its default and logs an error) instead of aliasing the instance; that
  includes `oasis_destroy` from a callback.
- `oasis_tick` sanitizes `delta_seconds` (NaN / infinite / negative count as 0, at most
  1 s per tick) and keeps its clock in `f64`: a single NaN frame time, or about six days
  of uptime, used to freeze the idle render and animations for good.
- `oasis_tick` runs queued `cmd &` jobs (they never ran under FFI) and due `cron` jobs, and
  reports their output through the new `OASIS_CB_SHELL_OUTPUT` (7) callback; `cron` /
  `startup` are now registered in FFI instances.
- `oasis_send_command` appends unredirected stderr to its result; callback details with
  interior NULs fire with U+FFFD instead of being dropped; a rejected or partial skin is
  logged at `error`; the WASM-only "Video Embed" dashboard entry is gone; a failing
  `oasis_video_play` no longer stops the video that was playing.
- CI's release `liboasis_ffi` artifact is built with the `release-ffi` profile
  (`panic = "unwind"`), so the exports' panic guards work in the shipped library.

### VFS, i18n, platform

- `MemoryVfs` / `GameAssetVfs`: only root or an entry's owner may chmod / chown it (anyone
  could before), and reaching an entry needs search permission on every ancestor directory,
  so a `0o700` directory owned by someone else hides its contents **(breaking)**.
- `GameAssetVfs::add_base_*` keeps the tree well formed (no file under a file, no kind
  change of a visible entry; content added below a removed directory brings it back).
- `RealVfs`: chmod works on Windows (it rejected the owner its own `get_permissions`
  reported); `readdir` follows symlinks like `stat`; dangling or escaping symlinks can be
  renamed; Windows names ending in `.` / space and `COM¹`-style device names are rejected;
  writing onto a directory is a typed `IsDirectory` error.
- `oasis-i18n`: interpolation is single-pass (a value containing `{other}` is no longer
  expanded); `Locale::from_code` is case-insensitive and accepts `ja_JP.UTF-8`-style codes.
- `oasis-platform`: `OASIS_FIXED_TIME` rejects impossible dates such as `2025-02-29`.

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
