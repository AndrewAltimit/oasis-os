# OASIS_OS Testing Gap Analysis & Plan

## Current State

**1,173 tests** across 96 modules. Coverage is strong in parsing/logic but thin in
integration, visual correctness, and platform-specific behavior.

| Crate | Tests | Assessment |
|-------|------:|------------|
| oasis-core | 1,106 | Strong unit tests, weak integration |
| oasis-backend-ue5 | 36 | Adequate |
| oasis-backend-sdl | 17 | Font only, no rendering verification |
| oasis-ffi | 14 | Basic C-ABI smoke tests |
| oasis-app | 0 | No tests at all |
| oasis-backend-psp | 0 | Excluded from workspace, crash-only CI check |

---

## Gap Categories

### A. Zero-Coverage Modules

These files/modules have literally no tests:

| Module | Lines | Risk | Notes |
|--------|------:|------|-------|
| `input.rs` | ~120 | Medium | Event enums, `Button`/`Trigger` mapping |
| `error.rs` | ~80 | Low | Error type conversions, `Display` impls |
| `backend.rs` | ~400 | Medium | Trait default method impls (rounded_rect, gradients, batch) |
| `oasis-app/main.rs` | 1,348 | High | Entire desktop entry point, mode switching, event loop |
| `oasis-app/screenshot.rs` | 357 | Medium | Screenshot capture tool |

### B. Under-Tested Modules

| Module | Tests | Public API surface | Gap |
|--------|------:|--------------------|-----|
| UI widgets (15+ types) | 12 | Button, Card, TabBar, Panel, TextField, ListView, ScrollView, ProgressBar, Toggle, NinePatch, Avatar, Badge, Divider, Icon, TextBlock | Only animation/color/layout have tests. No widget rendering, state, or interaction tests |
| SDL backend rendering | 0 | fill_rect, blit, draw_text, rounded_rect, gradients, clip stack, transform stack | Zero rendering correctness tests |
| SDL backend input | 0 | Keyboard/mouse/gamepad mapping | Zero input mapping tests |
| Platform services | 12 | PowerService, TimeService, UsbService, NetworkService, OskService | Only stub implementations tested |
| Terminal interpreter | 4 | Pipe parsing, globbing, env vars, quoting | Minimal parser tests |
| Skin TOML loader | 10 | External skin loading from `skins/*.toml` | No tests for malformed TOML, missing fields, partial skins |
| Transfer/FTP | 21 | FTP protocol state machine | No actual I/O tests |
| Browser Gemini | 17 | Gemini protocol, TLS, content types | No real connection tests |

### C. Missing Test Categories

| Category | Current | Gap |
|----------|---------|-----|
| Visual regression | 0 | No screenshot comparison infrastructure |
| Integration (multi-module) | 0 | No end-to-end workflow tests |
| Fuzz testing | 0 | HTML/CSS/Gemini parsers are fuzz-worthy |
| Property-based | 0 | No quickcheck/proptest usage |
| Performance/benchmark | 0 | No `criterion` benchmarks |
| PSP screenshot verification | 0 | PPSSPP runs but only checks crash/no-crash |
| Browser page rendering | 0 | No verification that pages look correct |
| Skin visual correctness | 0 | Screenshots exist but no diff tooling |
| Accessibility | 0 | No keyboard-nav or contrast-ratio tests |
| Concurrency/thread safety | 0 | No multi-threaded stress tests |
| Memory/resource leaks | 0 | No leak detection |

---

## Plan

### Phase 1: Unit Test Gaps (CI-integrated)

Fill zero-coverage and under-tested modules with fast, deterministic unit tests.

#### 1.1 Input Module Tests
**File:** `oasis-core/src/input.rs`
**Effort:** Small
- Test `Button` and `Trigger` enum exhaustiveness
- Test `InputEvent` construction for all variants
- Test `Display`/`Debug` formatting if implemented
- Test any conversion traits (`From`, `TryFrom`)

#### 1.2 Error Module Tests
**File:** `oasis-core/src/error.rs`
**Effort:** Small
- Test `OasisError` variant construction
- Test `Display` output for each variant
- Test `From` conversions (io::Error, string, etc.)
- Test `Result` type alias usage

#### 1.3 Backend Trait Default Method Tests
**File:** `oasis-core/src/backend.rs`
**Effort:** Medium
- Create a `RecordingBackend` test double that records all draw calls
- Test default `fill_rounded_rect` decomposes into correct fill_rect calls
- Test default `stroke_rect` produces correct border rectangles
- Test default `gradient_v`/`gradient_h`/`gradient_4` color interpolation
- Test default `draw_line` rasterization
- Test default `fill_circle`/`fill_triangle` decomposition
- Test `DrawCommand` batch dispatch calls the right methods
- Test clip stack push/pop behavior
- Test transform stack push/pop behavior

#### 1.4 UI Widget Unit Tests
**File:** `oasis-core/src/ui/*.rs`
**Effort:** Large
- **Button:** construction, state transitions (Normal/Hover/Pressed/Disabled), style variants
- **Card:** padding, title, elevation shadow params
- **TabBar:** tab selection, active index, page indicator position
- **Panel:** scroll offset, header layout, content area rect
- **TextField:** cursor movement, text insertion/deletion, selection range
- **ListView:** item count, selection index, scroll-into-view
- **ScrollView:** content size vs viewport, scroll position clamping, scrollbar thumb size
- **ProgressBar:** value clamping (0-100), fill width calculation
- **Toggle:** on/off state, toggle action
- **NinePatch:** border slicing coordinates, center stretch calculation
- **Avatar, Badge, Divider, Icon, TextBlock:** construction and basic properties

#### 1.5 Terminal Interpreter Hardening
**File:** `oasis-core/src/terminal/interpreter.rs`
**Effort:** Medium
- Test command argument splitting with quotes
- Test semicolon-separated multi-command lines
- Test empty input, whitespace-only input
- Test unknown command error message
- Test command name case sensitivity
- Test max argument count handling

#### 1.6 Skin Loader Robustness
**Files:** `oasis-core/src/skin/loader.rs`
**Effort:** Medium
- Test loading each of the 2 external TOML skins (`skins/xp.toml`, etc.)
- Test malformed TOML (syntax errors, wrong types)
- Test partial skins (missing optional fields)
- Test color parsing edge cases (3-char hex, rgba, named colors)
- Test skin hot-swap (load skin A, switch to skin B, verify state)

#### 1.7 FFI Edge Cases
**File:** `oasis-ffi/src/lib.rs`
**Effort:** Small
- Test double-destroy safety
- Test tick with very large delta_ms
- Test send_command with empty string, very long string, non-UTF8
- Test add_vfs_file with empty data, large data
- Test get_buffer after multiple ticks (buffer stability)
- Test concurrent oasis_tick calls (if applicable)

---

### Phase 2: Integration Tests (CI-integrated)

Multi-module workflows that verify components work together.

#### 2.1 Shell Session Integration
**Effort:** Medium
- Create a full `Environment` with MemoryVfs, CommandRegistry, SdiRegistry
- Execute multi-step terminal sessions:
  - `mkdir /home/user/test` → `cd /home/user/test` → `touch file.txt` → `cat file.txt`
  - `cp` + `mv` + `find` workflows
  - `skin list` → `skin set xp` → verify theme changed
  - `plugin load` → execute plugin command → `plugin unload`
- Verify CWD tracking across commands
- Verify SDI state after commands that modify UI

#### 2.2 Window Manager Integration
**Effort:** Medium
- Create WM + SDI registry, open multiple windows
- Verify z-order after focus changes
- Open app → minimize → open another → maximize → restore first
- Drag window to new position, verify SDI object positions updated
- Resize window, verify content rect recalculated
- Close all windows, verify cleanup

#### 2.3 Browser Pipeline Integration
**Effort:** Large
- Load HTML string → parse → cascade CSS → layout → paint → verify draw calls
- Test complete page lifecycle with MockBackend:
  - Simple `<p>Hello</p>` → verify text drawn at correct position
  - `<div style="background:red">` → verify fill_rect with red color
  - `<a href="...">link</a>` → verify link region registered
  - `<table>` → verify cell positions
  - `<img>` → verify blit call with correct dimensions
- Test navigation: load page A → click link → verify page B loaded → back → verify page A
- Test scroll: load tall page → scroll down → verify paint offset

#### 2.4 App Lifecycle Integration
**Effort:** Medium
- Launch app via dashboard selection → verify WM window created
- Run File Manager app → navigate directories → verify VFS reads
- Run Browser app → load local HTML → verify render
- Exit app → verify WM window closed and dashboard restored

#### 2.5 Audio Manager Integration
**Effort:** Small
- Create AudioManager + NullAudioBackend + playlist
- Add tracks → play → next → previous → verify state transitions
- Test shuffle produces different order
- Test repeat modes (off/one/all) at playlist boundary

#### 2.6 Plugin System Integration
**Effort:** Medium
- Load plugin → verify commands registered
- Execute plugin command → verify SDI objects created
- Update plugin → verify state changes
- Unload plugin → verify cleanup (commands removed, SDI objects removed)

#### 2.7 Remote Terminal Integration
**Effort:** Medium
- Start listener on localhost → connect client
- Send commands from client → verify execution on server
- Send multi-line output → verify client receives all lines
- Test PSK authentication (correct key, wrong key, no key)
- Test disconnect and reconnect

---

### Phase 3: Screenshot Tests (Local, Manual Review)

These produce PNG screenshots for human review. Not CI-blocking -- triggered locally
with a `cargo run` command or `make` target.

#### 3.1 Screenshot Test Harness
**Effort:** Medium (infrastructure)

Build a test runner that:
1. Initializes SDL backend in headless/offscreen mode (or use existing screenshot.rs approach)
2. Sets up a specific scenario (skin + state + content)
3. Renders frame(s) and captures via `read_pixels`
4. Saves PNGs to `screenshots/tests/{scenario_name}/`
5. Optionally generates a comparison HTML report (side-by-side with reference)

**Output structure:**
```
screenshots/tests/
├── report.html              # generated comparison page
├── dashboard_classic/
│   ├── actual.png
│   └── reference.png        # committed golden file (optional)
├── browser_simple_html/
│   ├── actual.png
│   └── reference.png
├── ...
```

**CLI:**
```bash
# Generate all screenshot tests
cargo run -p oasis-app --bin screenshot-tests

# Generate for specific scenario
cargo run -p oasis-app --bin screenshot-tests -- --scenario browser_simple_html

# Generate for specific skin
cargo run -p oasis-app --bin screenshot-tests -- --skin xp

# Generate comparison report
cargo run -p oasis-app --bin screenshot-tests -- --report
```

#### 3.2 Skin Screenshot Matrix
**Effort:** Medium

For each of the 8 skins (classic, xp, terminal, ocean, sunset, forest, midnight,
corrupted), capture:
1. Dashboard with icons
2. Terminal with output
3. Start menu open
4. Window manager with 2-3 overlapping windows
5. Settings app open
6. Browser showing a page

**Total: 8 skins x 6 scenarios = 48 screenshots**

Purpose: Verify skin theming applies correctly everywhere. Catch regressions where
a skin change breaks another skin's appearance.

#### 3.3 Browser Rendering Screenshots
**Effort:** Large

Create test HTML/CSS pages in `test-fixtures/html/` and screenshot the browser
rendering them:

| Scenario | Content | Verifies |
|----------|---------|----------|
| `basic_text.html` | Headings h1-h6, paragraphs, bold/italic | Text rendering, font sizes |
| `colors_backgrounds.html` | Colored divs, background colors | Color parsing, fill_rect |
| `box_model.html` | Divs with margin/padding/border | Spacing, border rendering |
| `links.html` | Styled links, visited state | Link colors, underlines |
| `lists.html` | Ordered and unordered lists | List markers, indentation |
| `table.html` | Simple and complex tables | Cell alignment, borders |
| `float_layout.html` | Floated images with text wrap | Float positioning |
| `nested_layout.html` | Deeply nested divs | Layout engine correctness |
| `long_page.html` | Tall content requiring scroll | Scroll clipping |
| `images.html` | Inline images, sized images | Image scaling, blit |
| `css_cascade.html` | Competing selectors, specificity | Cascade resolution |
| `reader_mode.html` | Article with nav/ads/content | Reader mode extraction |
| `gemini_page.gmi` | Gemini text document | Gemini rendering |
| `error_page.html` | Intentionally broken HTML | Graceful degradation |
| `empty_page.html` | Empty body | No-crash baseline |

#### 3.4 Widget Gallery Screenshot
**Effort:** Small

Create a single "widget gallery" scenario that renders every UI widget in one frame:
- Buttons (all states: normal, hover, pressed, disabled)
- Cards with titles
- TabBar with 4 tabs
- ProgressBars at 0%, 50%, 100%
- Toggles (on/off)
- TextFields (empty, with text, with cursor)
- ListView with items
- ScrollView with content

Purpose: One screenshot to visually verify all widget rendering.

#### 3.5 Window Manager Screenshots
**Effort:** Small

Scenarios:
1. Single maximized window
2. Three cascaded windows with different focus
3. Window with dialog overlay
4. Minimized windows (taskbar visible)
5. Window being resized (drag handle visible)

#### 3.6 Transition/Animation Frame Captures
**Effort:** Small

Capture key frames of transitions:
1. Skin transition (frame 0, frame N/2, frame N)
2. Page transition (slide left/right)
3. Window minimize animation

---

### Phase 4: PSP Screenshot Tests (Local, PPSSPP)

Run OASIS_OS on PPSSPP emulator and capture screenshots for manual review.

#### 4.1 PPSSPP Screenshot Capture Infrastructure
**Effort:** Medium (infrastructure)

Extend the existing PPSSPP Docker setup to:
1. Run EBOOT.PBP in PPSSPPHeadless
2. Wait for specific conditions (frame count, or timeout)
3. Capture screenshot via PPSSPP's `--screenshot` flag or framebuffer dump
4. Save to `screenshots/psp/{scenario}/`

**Script:** `scripts/psp-screenshot.sh`
```bash
# Run PSP build in PPSSPP and capture screenshots
docker compose --profile psp run --rm \
  -e PPSSPP_HEADLESS=1 \
  -e SCREENSHOT_DIR=/screenshots \
  ppsspp /roms/release/EBOOT.PBP \
  --timeout=10 --screenshot=/screenshots/psp_dashboard.png
```

#### 4.2 PSP Dashboard Verification
- Boot EBOOT → wait for dashboard render → capture
- Verify: icons visible, status bar present, correct 480x272 resolution
- Verify: no VRAM corruption (garbage pixels, wrong stride)

#### 4.3 PSP Browser HTTPS Page Load
**This is the specific example from the task description.**
- Boot EBOOT → navigate browser to a bundled HTTPS test page
- Wait for page to report "loaded" state (or timeout after N seconds)
- Capture screenshot
- Human review: does the page content look correct? Is text readable?
  Are layout elements in the right positions?

Since PSP can't easily hit real HTTPS servers in an emulator, options:
1. **Bundled test page:** Include HTML in VFS, load via `file://` scheme
2. **Local server:** Run a test HTTPS server in Docker, configure PPSSPP
   networking to reach it
3. **Recorded response:** Replay a captured HTTP response

#### 4.4 PSP Input Verification
- Boot → simulate D-pad input sequence → capture result
- Verify: cursor moved, menu selection changed, terminal command typed

#### 4.5 PSP Audio Verification
- Boot → trigger audio playback → capture status bar (shows track info)
- Verify: no crash, status bar shows correct track name

---

### Phase 5: Fuzz Testing (Periodic, Not CI-blocking)

Long-running tests that find edge cases in parsers. Run nightly or on-demand.

#### 5.1 HTML Tokenizer Fuzzing
**Effort:** Medium
- Use `cargo-fuzz` or `afl` with `html/tokenizer.rs`
- Corpus: real-world HTML samples + generated edge cases
- Target: `HtmlTokenizer::new(input).collect::<Vec<_>>()`
- Goal: no panics, no infinite loops

#### 5.2 CSS Parser Fuzzing
**Effort:** Medium
- Fuzz `css/parser.rs` with malformed CSS
- Target: `parse_stylesheet(input)`
- Include: unclosed braces, invalid selectors, malformed values

#### 5.3 Gemini Parser Fuzzing
**Effort:** Small
- Fuzz `gemini/parser.rs`
- Target: `parse_gemini(input)`
- Simple line-based format, but exercise edge cases

#### 5.4 HTTP Response Parser Fuzzing
**Effort:** Small
- Fuzz `loader/http.rs` response parsing
- Include: malformed headers, huge content-length, chunked encoding

#### 5.5 Skin TOML Fuzzing
**Effort:** Small
- Fuzz `skin/loader.rs` with arbitrary TOML
- Goal: no panics on malformed skin files

#### 5.6 PBP Parser Fuzzing
**Effort:** Small
- Fuzz `pbp.rs` with arbitrary binary data
- PSP executable format parser should handle corruption gracefully

---

### Phase 6: Property-Based Tests (CI-integrated)

Use `proptest` crate for invariant testing.

#### 6.1 VFS Path Normalization Properties
- For any path string, `normalize(normalize(p)) == normalize(p)` (idempotent)
- `normalize` never produces `//` sequences
- `normalize` never ends with `/` (except root)
- Parent traversal `a/b/..` equals `a/`

#### 6.2 Color Arithmetic Properties
- `Color::rgb(r,g,b).r() == r` (roundtrip)
- Alpha blending is associative
- Gradient interpolation at t=0 gives start color, t=1 gives end color

#### 6.3 Layout Box Model Properties
- `content_width + padding_left + padding_right + border_left + border_right == box_width`
- Margin collapsing: `max(margin_a, margin_b)` not `margin_a + margin_b`
- All layout boxes have non-negative dimensions

#### 6.4 Playlist Properties
- `playlist.len()` always equals number of added tracks minus removed
- After `shuffle()`, playlist contains same tracks (just reordered)
- `next()` at end with `RepeatAll` wraps to index 0
- `previous()` at start with `RepeatAll` wraps to last

#### 6.5 Navigation History Properties
- `back()` after `forward()` returns to same URL
- History length never exceeds max
- Current URL always equals last navigated URL

#### 6.6 SDI Z-Order Properties
- No two objects share the same z-order (after normalization)
- `bring_to_front(id)` makes `id` have the highest z-order
- Object count equals number of `create` calls minus `destroy` calls

---

### Phase 7: Performance Benchmarks (Periodic)

Use `criterion` crate. Not CI-blocking but tracked over time.

#### 7.1 HTML Parsing Benchmark
- Parse a 10KB, 50KB, 100KB HTML document
- Measure: tokenization time, tree building time, total parse time

#### 7.2 CSS Cascade Benchmark
- Cascade 100 rules against 1000 elements
- Measure: selector matching time, specificity sorting time

#### 7.3 Layout Engine Benchmark
- Layout a page with 500 block elements
- Layout a page with a 20x20 table
- Measure: layout computation time

#### 7.4 Paint Benchmark
- Paint a page with 1000 draw commands
- Measure: command generation time, batch rendering time

#### 7.5 SDI Registry Benchmark
- Create/destroy 10,000 objects
- Query by name, iterate all objects
- Measure: registry operation throughput

#### 7.6 VFS Benchmark
- Write and read 1000 files
- Directory listing with 1000 entries
- Measure: I/O throughput (MemoryVfs vs RealVfs)

---

### Phase 8: SDL Backend Tests (CI-integrated where possible)

#### 8.1 SDL Rendering Correctness (Local, requires display)
**Effort:** Medium

Create headless SDL tests that:
- Initialize SDL with a hidden window
- Render known patterns (solid colors, gradients, text)
- Read pixels back and verify against expected values
- These may need `SDL_VIDEODRIVER=dummy` for CI

Tests:
- `clear(red)` → all pixels are red
- `fill_rect` at known position → correct pixels colored
- `draw_text("A", 0, 0)` → pixel pattern matches 8x8 glyph
- `load_texture` + `blit` → texture pixels appear at blit position
- Clip rect → drawing outside clip produces no pixels
- Gradient → pixel colors interpolate linearly

#### 8.2 SDL Input Mapping Tests (CI-integrated)
**Effort:** Small
- Test keyboard scancode → `InputEvent` mapping
- Test mouse motion → `CursorMove` mapping
- Test mouse click → `PointerClick` mapping
- Test gamepad button → `Button` mapping
- These can run without a display (mock SDL events)

#### 8.3 SDL Audio Backend Tests
**Effort:** Small
- Test `SdlAudioBackend` initialization (already has 12 tests)
- Add: test volume clamping, test rapid play/pause cycling
- Add: test loading invalid audio file (graceful error)

---

### Phase 9: Robustness & Edge Cases (CI-integrated)

#### 9.1 Browser Error Recovery
- Load HTML with unclosed tags → parser recovers
- Load CSS with syntax errors → cascade skips bad rules
- Load page that references missing images → placeholder shown
- Load page with circular CSS imports → no infinite loop
- Load page with extremely long lines → no OOM
- Navigate to invalid URL → error state displayed

#### 9.2 VFS Edge Cases
- Path with `..` above root → stays at root
- Filename with special characters (spaces, unicode, `.`, `..`)
- Write to read-only VFS → appropriate error
- Read very large file → memory handled
- Concurrent reads (if applicable)

#### 9.3 Network Edge Cases
- Connect to unreachable host → timeout error
- Server closes connection mid-transfer → graceful handling
- Send/receive with zero-length data
- PSK with empty string, very long string, non-ASCII

#### 9.4 Skin Loading Edge Cases
- Skin file with zero-length color values
- Skin referencing nonexistent textures
- Switching skins rapidly in sequence
- Loading corrupted skin data → fallback to default

#### 9.5 Terminal Command Edge Cases
- Commands with 1000+ character arguments
- Commands with null bytes in arguments
- Rapid sequential command execution (100 commands in loop)
- Recursive directory operations (deeply nested paths)

---

## Priority Matrix

| Phase | Effort | Impact | CI? | Priority |
|-------|--------|--------|-----|----------|
| 1. Unit test gaps | Medium | High | Yes | **P0 -- do first** |
| 2. Integration tests | Large | High | Yes | **P0 -- do first** |
| 3. Screenshot tests (desktop) | Large | Very High | No (local) | **P1 -- do next** |
| 9. Robustness/edge cases | Medium | High | Yes | **P1 -- do next** |
| 4. PSP screenshot tests | Medium | High | No (local) | **P2** |
| 6. Property-based tests | Medium | Medium | Yes | **P2** |
| 5. Fuzz testing | Medium | Medium | No (periodic) | **P3** |
| 7. Performance benchmarks | Medium | Low | No (periodic) | **P3** |
| 8. SDL backend tests | Medium | Medium | Partial | **P3** |

---

## Estimated Total Work

| Phase | New Tests (est.) | New Files |
|-------|----------------:|----------:|
| Phase 1: Unit gaps | ~120 | 0 (in-module) |
| Phase 2: Integration | ~60 | 5-8 test modules |
| Phase 3: Screenshots | ~50 scenarios | 1 binary + test fixtures |
| Phase 4: PSP screenshots | ~10 scenarios | 1 script |
| Phase 5: Fuzz targets | ~6 targets | 6 fuzz harnesses |
| Phase 6: Property tests | ~30 properties | 0 (in-module) |
| Phase 7: Benchmarks | ~15 benchmarks | 1 bench file |
| Phase 8: SDL tests | ~25 | 1 test module |
| Phase 9: Robustness | ~40 | 0 (in-module) |
| **Total** | **~356 new tests** | |

This would bring the project from ~1,173 to ~1,529 tests, plus 60+ screenshot
scenarios and 6 fuzz targets.

---

## Implementation Notes

### Test Infrastructure Needed
1. **`RecordingBackend`** -- a test double implementing `SdiBackend` that records all
   draw calls for assertion. Similar to `MockBackend` in browser tests but generalized
   for all modules.
2. **`TestEnvironment`** -- helper that creates a fully wired `Environment` (VFS +
   CommandRegistry + SdiRegistry + platform stubs) for integration tests.
3. **Screenshot test binary** -- extends existing `screenshot.rs` with scenario
   definitions and comparison report generation.
4. **PSP screenshot script** -- wraps PPSSPP Docker invocation with screenshot capture.
5. **`proptest` dependency** -- add to `[dev-dependencies]` in oasis-core.
6. **`criterion` dependency** -- add to `[dev-dependencies]` in oasis-core.
7. **`cargo-fuzz`** -- set up fuzz targets in `fuzz/` directory.

### Test Data / Fixtures
1. **HTML test pages** in `test-fixtures/html/` (15+ pages for browser screenshot tests)
2. **CSS test stylesheets** in `test-fixtures/css/`
3. **Gemini test pages** in `test-fixtures/gemini/`
4. **Malformed input samples** for fuzz corpus seeding
5. **Reference screenshots** (golden files) committed to `screenshots/tests/reference/`

### CI Integration
- Phases 1, 2, 6, 9: Add to existing `cargo test --workspace` (automatic)
- Phase 5: Separate CI job on nightly schedule (or manual trigger)
- Phase 7: Separate CI job with benchmark tracking (optional)
- Phases 3, 4: Local-only, documented in `Makefile` or `justfile`
