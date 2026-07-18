# Advanced Theming & Desktop Metaphor Plan

Status: **in progress** — M0 (benchmarks + validation + `skin lint`),
M1 (asset pipeline: skin `assets/`, `texture =` layout objects, image
wallpapers, image decal layers, D1 shader texture reuse), M2
(desktop icons: `icon_layout = "free"` with per-icon positions +
column auto-flow, rect hit-testing + icon drag & drop, per-skin
position persistence, `software_cursor` + `[cursor]` theming, free-mode
selection highlight, D3 vector icon scene cache), M3 (chrome &
motion: `[[chrome_layers]]` overlay vector chrome, tab pill texture
slots, `nine_patch` on layout objects + WM titlebar/frame chrome,
`[transition]` entrance = assemble/fade/none + page_style + easing,
free-mode hover focus, D4 static-layer op caching, D7 bar update_sdi
churn), and M4 (showcase + adoption: B7 `psix-tribute` skin, Track C
wiring — `widget_states` into the UI toolkit, `app_themes` in
terminal/file manager/settings, skin inheritance in practice, exposed
toast/icon-anatomy/desktop-size constants — A6 `skin-dev` hot reload,
skin-authoring docs v2, screenshot fixtures + deterministic z-order),
and M5 (perf tail: D2 color-independent glyph cache, D5 no-alloc window
compositing, D6 glyph cache bookkeeping, D8 handle-based SDI z-lists,
D9 evaluated and declined, A7 typography tokens) have landed. Branch:
`feat/advanced-theming`.

**M6 (second wave — UI/UX theme-engine completeness) has also landed** on
the same branch, closing the gaps a post-M5 audit surfaced:

- **A7 completed — per-skin TTF fonts**: `[typography] font =
  "assets/skin.ttf"` (fontdue rasterization into the colorless SDL glyph
  cache via a new `SdiText::set_font` extension; whole-pixel advances
  shared by measure and draw; per-character bitmap fallback; feature-gated
  so PSP/WASM/UE5 keep the bitmap font).
- **Runtime appearance editor**: skin serialization (`serialize` feature,
  `Skin::to_toml_string`/`save_to_directory`), dark/light/high-contrast
  variant derivation (`variants.rs`, WCAG-ratio enforced), a Settings
  "Appearance" category (base-color editing, Apply preview via in-memory
  swap, save-as-custom-skin), and `skin export` / `skin variant` commands.
- **Accessibility**: WCAG contrast lint (9 derived color pairs, warn-only)
  in `skin lint`; the previously dead `focus_ring_*` fields now render
  through `FocusStyle::from_theme` (exact fallback keeps unset skins
  pixel-identical).
- **Authoring & QA tooling (W4)**: `skin inspect <name>` prints a plain-text
  contact sheet (resolved base colors + WCAG AA contrast report + derived
  bar/icon/start-menu/app-screen tokens + ANSI palette rows); the Settings
  "Appearance" editor gained a per-role inline contrast readout (`AA` / `low`
  against each role's sensible partner color, live while stepping a channel);
  and `skin lint` warning strings were reworded to name the field, the
  offending value, and the fix/threshold. Tooling only — no render change.
- **UI sound themes**: `[sounds]` table (click/open/close/error/toast/nav
  WAV one-shots + volume), an 8-voice SFX mixer in oasis-audio on a
  dedicated SDL stream, shell chokepoint hooks; silent by default.
- **Widget completeness**: menu_bar derives from `ui::Theme` (defaults pin
  the Win95 grays), dedicated slider fields, new `widget_states.menu` /
  `widget_states.slider` slots.
- **Uniform widget interaction states + focus rings**: a single
  `WidgetStateColors` resolver (`oasis-ui/src/states.rs`) maps the
  `WidgetState` (Disabled > Pressed > Hover > Normal) onto the existing
  `ui::Theme` interaction fields, so `[widget_states.*]` overrides recolor
  button / checkbox / radio / toggle / slider / spin box / dropdown /
  tab bar / input field consistently instead of per-widget hardcoding.
  Each of those focusable widgets now also draws a keyboard focus ring via
  `FocusStyle::from_theme`. Pure rename for the default theme (rings only
  show in the focused state) — screenshots unchanged.
- **Terminal + shell holes**: 16-color `[palette]` ANSI table derived per
  skin, SGR foreground-color runs in terminal output (`ls`/`tree`/errors
  emit color), fully themeable boot splash (`[boot]`), themed procedural
  cursor fill/outline, and themable fallback palettes (dashboard,
  start menu, vector LED, background-layer line color).

All M6 fields default to current behavior; existing skins render
pixel-identically until they opt in (screenshot suite: 120 scenarios
green).

## Goal

Close the gap between OASIS_OS's current look — a uniform, centered icon grid
between two flat full-width bars ("mobile phone UI") — and the PSIX-style
desktop GUI: free-standing desktop icons over a rich layered wallpaper, shaped
and notched chrome bars, tab strips, a pointer cursor, and a signature motion
language. At the same time, make the theme engine powerful enough that this
look is *data*, not code, and land the rendering performance work that makes
richer themes affordable on every backend.

Reference: PSIX 1.90 source (analyzed from `psixpsp/1.90uo/psixsrc/`), whose
`oasis-sdi` is the spiritual descendant. Key findings from that analysis are
folded in below.

---

## 1. Research summary

### 1.1 How PSIX does it

- **A PSIX theme is a bundle of bitmaps, not a config file.** `psix.theme` is a
  binary dump of ~115 *named* RGBA sprite objects (`wall`, `bar_top`,
  `bar_lower`, `cursor`, `ms0_tab`…`net_tab`, `audio_tab`…`file_tab`,
  `icon_mypsp`, `icon_iso`, `icon_selected`, `battery_5`…`battery_95`,
  `vdm1`…`vdm4`, font atlases per color, OSK, mini-player, …). The C code
  hard-codes the names; the theme supplies pixels + default x/y/layer/enabled.
- **Shaped chrome is free.** The "notched" bars are just the alpha silhouette
  of the `bar_top` / `bar_lower` PNGs — the compositor alpha-tests every blit,
  so any bar shape is possible without any shape primitives.
- **Two-pass painter's compositor** (base pass then overlay pass, explicit
  `move_top`/`move_below`) — exactly what `oasis-sdi` already implements.
- **Pointer-first interaction**: an analog-stick-driven software `cursor`
  sprite with variable speed, hit-testing on hover, X = click. This — more
  than any layout detail — is what makes PSIX read as a desktop.
- **Fixed 6×3 icon grid per virtual desktop (×4 desktops)** — PSIX icons are
  *not* freely placed either; the desktop feel comes from sparse population
  (file-scan results, not an app launcher), file-type icons, left-aligned
  fill order, the cursor, and the chrome. Worth internalizing: we can get 80%
  of the feel without full free placement, but we should still build free
  placement (§3) because it's what actual desktops do.
- **Motion language**: boot "assemble" animation (top bar slides down, bottom
  bar slides up, black iris shrinks from center) and horizontal page-card
  slides (±480 px at 4 px/tick, direction by page parity).
- **PSIX's weakness we should not copy**: all geometry is `#define`d per
  module; themes can repaint but never re-layout. Our TOML layout system
  already beats this — we keep it and add the asset layer PSIX had.

### 1.2 Where OASIS is today

Skin engine (`crates/oasis-skin`, ~9.4k LoC) is parametric and deep — 9 base
colors, ~10 override tables, derivation engine, geometry table, background
layer system with 13 procedural kinds + 8 built-in shaders, per-skin virtual
resolution, inheritance. But:

- **Skins cannot ship a single image.** No texture/PNG/asset field exists
  anywhere (`SkinObjectDef` in `oasis-skin/src/loader/parsing.rs` has colors
  only, despite ADR-005 claiming otherwise). No bitmap wallpapers, no icon
  art, no chrome textures, no nine-patch assets.
- `oasis-ui`'s **NinePatch renderer is fully implemented and has zero call
  sites** (`crates/oasis-ui/src/nine_patch.rs`).
- **Dashboard is pure grid math**: `index → (row, col) → cell_rect`
  (`oasis-core/src/dashboard/mod.rs:51-86`); `AppEntry` has no position;
  hit-testing divides cursor coords by cell size
  (`oasis-app/src/input.rs:216-259`, `:576-610`); no icon drag; no position
  persistence. Cells stretch to fill the screen evenly — the "phone grid".
- **Bars are hardcoded full-width rectangles** (`statusbar.rs:155`,
  `bottombar.rs:170`); SDI has no polygon primitive and no textured-with-alpha
  chrome convention.
- The software cursor exists but is disabled on SDL (`oasis-app/src/render.rs:170`);
  WM drag/resize/hit-test infrastructure is mature and reusable
  (`oasis-wm/src/drag_resize.rs`, `hit_test.rs`).
- **Silent schema drift**: skin TOMLs already set fields that don't exist
  (vaporwave/win95 phantom `wm_theme` keys) and nothing warns.
- Underused hooks ready to exploit: `app_themes` (2 consumers),
  `widget_states` (parsed, never read), named `gradients`/`animations`,
  skin inheritance (implemented, unused), `from_base_colors()` (programmatic
  themes), the `corrupted.toml` per-skin effect channel.

### 1.3 Performance findings (full detail in §5)

The shell redraws and *re-computes* the entire scene every frame: shader
wallpapers re-shade per-pixel and destroy/recreate a GPU texture per frame;
the glyph cache keys on color so themed text re-rasterizes; vector icons
re-tessellate every frame; background layers rebuild every frame; SDL uses the
allocating window-composite path when a no-alloc variant already exists. None
of this is gated by a draw benchmark today.

---

## 2. Track A — Theme engine v2: the asset pipeline

The single highest-leverage change. Everything PSIX-like downstream (shaped
bars, watermarked wallpapers, themed cursors, bitmap icons) falls out of it.

### A1. Skin asset directory

- Add `assets/` to the skin directory convention:
  `skins/<name>/assets/*.png` (and later `.ttf`).
- `Skin::from_directory` loads/decodes referenced images (via the `png` path
  already used elsewhere in the workspace); WASM/built-in skins embed assets
  through `build.rs` `include_bytes!` the same way TOML is embedded today.
- New `SkinAssets` store: name → decoded RGBA + dims, uploaded lazily as SDI
  textures on skin swap, freed on swap-out.
- PSP constraints respected at load: warn (desktop) / reject (PSP) on
  non-power-of-two textures; document a per-skin asset budget (target ≤ 2 MB
  decoded for PSP skins — PSIX shipped 3.6 MB of *uncompressed* sprites, PNG
  gets us far below that).

### A2. Images in layout.toml — the "named slot" vocabulary, done declaratively

Extend `SkinObjectDef` with:

```toml
[bar_top]
texture = "assets/bar_top.png"     # alpha-cut shaped chrome, PSIX-style
# existing fields still apply: x, y, w, h, z, alpha, visible ...

[window_button_close]
nine_patch = { image = "assets/btn.png", insets = [4, 4, 4, 4] }
```

- `texture` → object renders the bitmap (alpha-blended, so any silhouette).
- `nine_patch` → wires up the dormant `oasis-ui` NinePatch renderer for
  scalable chrome (bars, window frames, buttons, panels).
- Publish the **well-known object name vocabulary** (the OASIS analog of
  PSIX's 115 slots) in `docs/skin-authoring.md`: bars, tabs, cursor, selection
  highlight, page dots, battery states, OSK, toasts. Most names already exist
  in the SDI registry; this makes them a documented, stable theme API.

### A3. Image wallpapers + image background layers

- `[wallpaper] style = "image", source = "assets/wall.png"` with
  `fit = cover|contain|stretch|tile` (CPU-composited into the existing
  wallpaper RGBA buffer in `oasis-core/src/wallpaper.rs`, so it composes with
  procedural layers).
- New `[[background_layers]] kind = "image"` with the existing anchor/offset/
  alpha/animation fields → PSIX-style **watermark logos** and floating decals
  that drift/pulse using the animation system that's already there.

### A4. Themeable cursor, selection, and file-type icons

- `[cursor] texture = "assets/cursor.png", hotspot = [1, 1]` replacing the
  procedural arrow in `oasis-app/src/cursor.rs`; keep procedural as fallback.
- `icon_selected`-style hover/selection highlight object (PSIX's most-loved
  affordance) — themeable texture or derived rounded rect.
- Optional bitmap icon slots: `[icons.overrides] terminal = "assets/term.png"`
  taking precedence over vector presets; plus file-type icon slots consumed by
  the File Manager (`icon_folder`, `icon_doc`, `icon_iso`, `icon_default`).

### A5. Schema validation & author feedback

- Add a validation pass that walks the parsed TOML against known fields and
  reports unknown keys (warning list surfaced in the terminal on `skin` swap
  and in the loader log). Fix the existing phantom keys in vaporwave/win95 —
  either implement the fields they reference or correct the skins.
- `skin lint <name>` terminal command running the same check + asset
  existence/size/POT checks.

### A6. Hot reload (desktop only, dev QoL)

- Watch the active external skin directory (simple mtime poll each second on
  the existing frame loop — no new dependency needed) and re-trigger the
  existing `apply_skin_swap` path on change. Feature-gated `skin-dev`.
  This is what makes iterating on a PSIX-quality skin humane.

### A7. Fonts (stretch, second wave)

- `font = "assets/skin.ttf"` per skin, rasterized with `fontdue` (already a
  workspace dependency via oasis-browser's @font-face support) into the
  backend glyph-cache pipeline. Expose the `ui_theme` font-size ladder and
  spacing tokens (currently hardcoded constants in
  `oasis-skin/src/theme/conversion.rs:86-107`) as skin fields regardless.

---

## 3. Track B — Desktop metaphor: dashboard v2

Keep the shared grid core intact for PSP/d-pad; add a desktop-icon path as an
additive mode.

### B1. Icon layout modes + per-icon positions

- `features.toml`: `icon_layout = "grid" | "free"` (default `grid`).
- Add `position: Option<(i32, i32)>` to `AppEntry`
  (`oasis-core/src/dashboard/discovery.rs:13`) and a `positions` map on
  `DashboardState`. In `free` mode, `update_sdi` and `render_vector_icons`
  use stored positions instead of `GridLayout::cell_rect`; unplaced icons
  auto-flow **top-to-bottom, left-to-right in columns** (classic desktop /
  PSIX fill order — this alone kills the "centered phone grid" look).
- Fixed icon cell size in free mode (e.g. 80×80 at 480×272, scaled by theme
  geometry) instead of stretch-to-fill.

### B2. Real hit-testing + icon drag & drop

- Replace grid-division hit-testing in `oasis-app/src/input.rs:216-259` and
  `:576-610` with per-icon rect tests (pattern: `oasis-wm/src/hit_test.rs`).
- Icon drag: press-hold threshold (≥ 4 px movement or ≥ 250 ms) →
  `IconDrag { index, grab_offset }` state modeled on `oasis-wm`'s
  `DragState::Moving`; raise dragged icon's SDI objects to top; drop commits
  position (with optional `snap_to_grid = true` feature flag quantizing to a
  virtual grid, which is how real desktops behave anyway).
- Single click = select (highlight object from A4), double click (or single,
  per `features.launch_on_single_click`) = launch. D-pad still walks icons in
  reading order so PSP/keyboard users lose nothing.

### B3. Position persistence

- Persist `app path → (x, y, page)` per skin via the existing settings
  mechanism (`oasis-core/src/settings.rs`), key `icon_positions.<skin>`.
  Written on drop, loaded on dashboard build; invalid/off-screen entries are
  clamped, missing entries auto-flow (B1).

### B4. Desktop mode unification

- Let `Mode::Desktop` show dashboard icons *and* windows: vector icons are
  already injected between base SDI and windows in `main.rs:713-753`; extend
  so the full icon pipeline (SDI icon styles, labels, selection) runs there.
  The `desktop` skin then becomes: wallpaper + free icons + taskbar + WM —
  an actual desktop instead of an empty WM playground.
- Re-enable the software cursor on SDL behind
  `features.software_cursor = true` (skin-controlled, themeable via A4);
  default stays host-pointer.

### B5. Shaped, layered chrome

Two complementary mechanisms, both theme-driven:

- **Textured bars (the PSIX way)**: with A2, a skin sets
  `bar_top.texture = "assets/bar_top.png"` — alpha silhouette gives notches,
  slants, and cutouts for free. Bar hit-zones stay rectangular; only visuals
  change. Sub-tab strips (`bar_tab_bg_*` in `statusbar.rs`, media tabs in
  `bottombar.rs`) get texture/nine-patch slots for active/inactive states.
- **Vector chrome (procedural way)**: a `[[chrome_layers]]` table mirroring
  `background_layers` but rendered in the overlay pass via the existing
  `vector_overlay` hook (`main.rs:762` precedent), using `oasis-vector`'s
  polygon/arc ops — for skins that want shaped chrome without shipping art.

### B6. Motion language (PSIX signature moves)

- **Boot assemble**: on skin load / boot-splash handoff, slide `bar_top` down
  and `bar_bottom` up from off-screen while an iris rectangle shrinks from
  center — implement as a general `[transition] entrance = "assemble" |
  "fade" | "none"` with duration + easing from the existing `animations`
  preset system. Honors `reduced_motion`.
- Expand `[transition]` beyond `fade_color`: `page_style = "slide" | "fade"`,
  durations in ms, easing name (resolver already supports 8 curves,
  `theme/overrides.rs:707`).
- Icon hover micro-motion (lift + shadow grow) using existing entrance/press
  animation plumbing in `vector_icons.rs`.

### B7. Showcase skin: `psix-tribute`

A new external skin exercising every capability above, closing the loop on
the reference screenshot: orange→green multi-stop gradient wallpaper with a
drifting watermark decal (A3), alpha-shaped top/bottom bars with tab strips
(B5), document-style file icons in free layout with left-column auto-flow
(B1), themed cursor + selection highlight (A4), assemble entrance + sliding
pages (B6). This skin is the acceptance test for the whole plan and the
screenshot-regression fixture for CI.

---

## 4. Track C — Wire up what already exists (cheap wins)

- **`widget_states`**: make `oasis-ui` Button/InputField/Toggle query
  `ActiveTheme::widget_state_color` for hover/pressed/disabled instead of
  hardcoded derivation. (Parsed today, consumed nowhere.)
- **`app_themes`**: adopt in the remaining apps (only tv-guide and paint read
  it) — at minimum terminal ANSI-ish palette, file manager, settings.
- **Skin inheritance**: use it — rebase variant skins on a shared parent to
  cut duplication, and document it as the recommended authoring pattern.
- **Expose hardcoded constants**: `success_bg`/`warning_bg` toast colors
  (`derive.rs:833/1101`), icon anatomy constants (`derive.rs:171`), wallpaper
  generator magic numbers (`wallpaper.rs`), the forced 1280×720 desktop
  upscale (`oasis-app/src/main.rs:77` → make it a config/skin field).

---

## 5. Track D — Performance

From the pipeline audit (evidence at cited lines). Ordered by
impact-per-invasiveness; items marked ⚑ directly cheapen the new theming
features, so they land in the same milestone as their feature.

| # | Item | Impact | Effort | Scope |
|---|------|--------|--------|-------|
| D1 ⚑ | **Shader wallpaper: stop per-frame texture create/destroy** — reuse one streaming texture in `oasis-backend-sdl/src/shader_bridge.rs:38-62` instead of `load_texture`+`destroy_texture` every frame (also fixes unbounded `next_texture_id`). Throttle software shading to 30 fps and skip entirely when fully occluded (fullscreen app/terminal). | High | Low–Med | realloc: SDL; shade throttle: all backends |
| D2 ⚑ | **Color-independent glyph cache** — render glyphs white, drop color from `GlyphCacheKey` (`oasis-rasterize/src/lib.rs:46`), tint at blit time via `SDL_SetTextureColorMod` (already used in `blitting.rs:52`). Kills re-rasterization on themed/animated text and shrinks the 2048-entry cache. | Med–High | Med | SDL |
| D3 ⚑ | **Cache vector icon tessellation** — `icon_for_app` + scene build re-run per icon per frame (`dashboard/vector_icons.rs:457-498`); cache scaled `VectorScene` per (preset, color, size), rebuild only when the icon's animation actually advances. | Med | Med | all backends |
| D4 ⚑ | **Cache static background/chrome layers** — `render_vector_background` clones the layer list and rebuilds ops every frame (`oasis-core/src/vector_overlay.rs:44-60`); rebuild only animated layers. Same treatment for the new `chrome_layers` (B5) from day one. | Med | Low–Med | all backends |
| D5 | **Switch SDL window compositing to the no-alloc path** — `draw_with_clips_noalloc` exists (`oasis-wm/src/render.rs:93`) but SDL calls the `format!`-heavy variant (`render.rs:50`); also precompute window-prefix sets on window-set change instead of per-frame `starts_with` scans over every SDI object. | Low–Med | Low | all backends |
| D6 | **Glyph cache bookkeeping** — store dims in the entry instead of `texture.query()` per glyph per frame; frame-counter LRU instead of per-hit HashMap insert + O(n) eviction scan (`glyph_cache.rs:160-193`). | Low–Med | Low | SDL |
| D7 | **Bar `update_sdi` churn** — statusbar/bottombar rebuild names via `format!` and reassign `obj.text` every frame (`statusbar.rs:196-356`); cache names like the dashboard already does (`dashboard/mod.rs:178`) and skip unchanged text. | Low | Low | all backends |
| D8 | **SDI handle-based z-lists** — draw path does a `HashMap<String>` lookup per object per frame (`oasis-sdi/src/registry.rs:349`); move to index/handle-based z-lists. Do this *before* themes multiply object counts. | Low–Med | Med | all backends |
| D9 | **Shell batching** — extend `SdiBatch` use beyond browser/file-manager into the SDI registry draw (coalesce same-color rect runs, batched text). Biggest payoff on PSP/WASM. Defer to last; measure first. | Med | High | PSP/WASM mainly |
| D10 | **Draw-path benchmark** — `oasis-sdi/benches/sdi_registry.rs` never benches `draw()`. Add a full-scene draw bench against the test backend + a dashboard-frame bench, so D1–D9 and the new theme features are regression-gated. Land this *first*. | — (enabler) | Low | CI |

### M5 results (measured against the M0 `sdi_draw` benchmarks)

D8 dominated: moving the z-lists from cloned names to slab handles cut the
steady-state draw of a 120-object frame from ~3.6 µs to ~0.53 µs (−85%), a
1000-object frame by −82%, the z-list rebuild by −95%, and the
window-compositing pass by −68% (D5 + D8 together). Per-object registry cost is
now ~4 ns.

**D9 (shell batching) was evaluated and declined.** Two measurements say the
work would not pay for itself:

1. After D8 the registry side of the draw is ~4 ns/object against a no-op
   backend — there is no per-object bookkeeping left worth coalescing. What
   remains is the backend call itself.
2. `SdiBatch` can only coalesce *flat* rects and same-style text runs, and the
   shell's objects do not come in runs. A 12-icon dashboard scene is 37
   objects: 12 flat rects, 8 rounded, 8 text, 4 shadowed, 4 stroked — and they
   interleave per icon (rounded body, flat graphic, text label), so
   order-preserving runs are 1–2 objects long. Coalescing would remove a
   handful of calls per frame and add a staging buffer plus a draw-order
   hazard.

The remaining upside is backend-local — PSP's GU command cost and WASM's
per-call JS FFI overhead — and belongs *inside* those backends'
`fill_rect`/`draw_text` (where `SdiBatch` is already overridable), not in a
restructured shared draw path.

---

## 6. Sequencing

Milestones sized to be independently landable PRs; each ends green
(fmt/clippy/tests/screenshot regression).

1. **M0 — Bench + guardrails**: D10 draw benchmarks; A5 schema validation +
   `skin lint`; fix phantom skin keys. *(Small, unblocks everything.)*
2. **M1 — Asset pipeline**: A1 asset loading, A2 `texture` on layout objects,
   A3 image wallpaper + image background layer. D1 lands here (wallpaper
   perf is about to matter more). First visible payoff: bitmap wallpaper +
   watermark + shaped bar textures.
3. **M2 — Desktop icons**: B1 layout modes + positions, B2 hit-test + drag,
   B3 persistence, B4 desktop-mode unification + optional software cursor,
   A4 cursor/selection theming. D3 lands here.
4. **M3 — Chrome & motion**: B5 shaped chrome (textures + `chrome_layers`),
   B6 transitions/assemble entrance, A2 nine-patch consumers (bars, WM
   chrome via `WmTheme`). D4 + D7 land here.
5. **M4 — Showcase + adoption**: B7 `psix-tribute` skin, Track C wiring
   (`widget_states`, `app_themes`, inheritance), docs
   (`skin-authoring.md` v2 with the named-slot vocabulary), screenshot
   fixtures. A6 hot reload.
6. **M5 — Perf tail**: D2, D5, D6, D8; evaluate D9 with M0 benchmarks in
   hand. A7 fonts if appetite remains.

## 7. Risks & constraints

- **PSP**: dashboard core is shared; `icon_layout = "free"` must be a no-op
  fallback to grid on d-pad-only targets (PSP keeps its 4×3 grid). Asset
  textures must be power-of-two, 16-byte aligned; built-in skin embedding
  already adds ~850 KB on PSP, so PSP skins must opt into assets explicitly.
  All new theme fields default to current behavior — 15 existing skins render
  pixel-identically until they opt in (screenshot regression enforces this).
- **WASM**: assets embed via `build.rs` for built-ins; external skin loading
  stays desktop-only (already the case).
- **Scope discipline**: no custom GLSL from skins, no theme *editor* tool,
  no browser-engine perf work in this plan. Nine-patch WM chrome is the
  boundary — full custom window-shape theming is out.
- **Compat**: schema validation is warn-only for one release before any key
  becomes an error.
