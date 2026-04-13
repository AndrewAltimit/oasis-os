# Browser Engine Backlog

Gap analysis and follow-up work for `oasis-browser`, organized by epic.
Each epic is scoped as a standalone PR or short series. Items marked with
effort estimates assume one focused engineer-week.

This document is the live roadmap for closing the gap between our
from-scratch engine and a launch-ready embedded browser. Items are
grouped by area; ranking guidance is at the bottom.

Last updated: 2026-04-12 (tracked in `feat/browser-improvements`).

---

## Epic: PSP JavaScript integration

**Effort:** 1–3 weeks. **Risk:** medium (revised — see below). **Value:**
enables a large class of modern sites that use JS for content hydration.

PSP has 24MB user RAM + 333MHz MIPS. With the rust-psp std overlay fixed
on 2026-04-13 (`HashMap`, `Instant::now`, and full `std::sync` all
verified working on real hardware), every prerequisite for a pure-Rust
JS engine is in place.

### Revised plan: pure-Rust engine (`boa_engine`) instead of QuickJS-NG

The original three-phase plan called for vendoring QuickJS-NG via the
`cc` crate and writing a hand-rolled FFI wrapper. The two motivations
behind that approach have both evaporated:

1. **"`rquickjs` uses `std::time::Instant`, which crashes on PSP
   Allegrex"** — this turned out to be wrong. `Instant::now` was hitting
   `unsupported::Instant::now` because the rust-psp std overlay had no
   `target_os = "psp"` arm in the new `sys/time/mod.rs`; it never had
   anything to do with `rquickjs`. Fixed in rust-psp branch
   `fix/psp-hardware-std-overlay-alignment-and-time` and verified on
   hardware.
2. **"Need a thin FFI wrapper because `rquickjs` is too heavy"** — the
   real blocker was getting QuickJS's C source to cross-compile for
   `mipsel-sony-psp`, which requires a MIPS libc / cross-compiler
   (pspdev) that isn't installed on the dev box. Sidestepped entirely
   by using `boa_engine`, which is pure Rust with no C dependencies.

**Phase 1 — Add `boa_engine` to `oasis-js` behind a feature flag.**

- New `boa` feature on `oasis-js` swaps the `rquickjs`-backed
  `JsEngine` for a `boa_engine`-backed one with the same public API
  (`new`, `eval`, `JsValue`, `JsError`).
- Desktop and WASM keep using `rquickjs` (no behavior change). PSP
  builds with `--features boa` and gets the pure-Rust engine.
- Console/fetch/storage/timers stay rquickjs-only for this PR — those
  glue layers can be ported to boa in a follow-up if PSP needs them.

**Phase 2 — Wire `oasis-js` into the PSP backend.**

- Add `oasis-js = { workspace = true, features = ["boa"] }` to
  `oasis-backend-psp` so the engine is linked into the EBOOT.
- Add a `js <code>` command to `cmd_server.rs` that evaluates a
  one-shot JavaScript expression and returns the result over TCP.
  Exercises the engine end-to-end on real hardware without needing
  the full DOM glue.

**Phase 3 — Wire through `oasis-browser` PSP build.**

- DOM bindings (`oasis-browser/src/js_dom.rs`) are heavily tied to
  `rquickjs::{Ctx, Function, Object}` — porting to boa requires a
  parallel implementation. Defer this to a follow-up PR; for now the
  PSP build of `oasis-browser` keeps `javascript = false` and scripts
  are still dropped at parse time.
- Update the `# Feature flags` docstring in `src/lib.rs` and the
  `oasis-backend-psp/Cargo.toml` note to describe the new state.

**Non-goals:** V8-level performance. boa is an interpreted reference
implementation; expect ~10× slower than QuickJS, which itself is
~500-1000× slower than V8 on Allegrex. Inert pages with small
bootstrap scripts will work; React SPAs will be unusable. That's fine
— degraded is better than dead.

**Testing:** PPSSPP headless for the unit test, real hardware for the
end-to-end `js` command via TCP cmd_server.

---

## Epic: 3D transforms

**Effort:** 1–2 weeks. Standalone from compositor, similar
cross-cutting nature.

- Add `AffineTransform3D` alongside the existing `AffineTransform2D`.
- Perspective projection: `perspective`, `perspective-origin` (today
  stored opaque, ignored at paint).
- `transform-style: preserve-3d` — parsed, ignored. Needs child z-sort
  under parent's 3D frame and proper matrix composition.
- `translate3d` / `rotate3d` / `rotateX` / `rotateY` / `rotateZ` /
  `scale3d` — parsed today but flattened to 2D affine.
- `backface-visibility: hidden` — parsed; needs paint-time normal
  check to cull back-facing quads.

**Backend impact:** desktop and WASM can rasterize transformed quads
today. UE5 and PSP need careful thought — PSP GU does have a perspective
matrix stack (`sceGumPerspective`) but wiring it into 2D UI paint is
non-trivial.

---

## Epic: Real-world compatibility measurement

**Effort:** ongoing grind. **Highest-leverage item on the list.**

This PR (`feat/browser-improvements`) added 4 fixtures in
`tests/fixtures/`. That's the floor, not the target.

**Corpus expansion (20–50 fixtures):**

- Wikipedia article (real HTML pulled from a stable revision, not
  synthetic).
- Hacker News front page.
- GitHub README rendered output.
- A docs site (Rust `std` docs, MDN reference page).
- A forum (phpBB or Discourse snapshot).
- A news masthead (NYT-style multi-column grid).
- A commerce product page.
- A blog platform post (Medium, Substack).

Strip each to a reasonable size and check in under `tests/fixtures/`.

**Visual regression harness.** Single highest-leverage item:

- Render each corpus fixture to PNG via the SDL backend.
- Check golden PNGs into the repo (one per fixture).
- CI gate on pixel delta > threshold.
- Add as a new CI step after the existing `screenshot regression` job.
- This catches ~90% of paint regressions automatically.

**Layout performance budgets:**

- "Wikipedia frontpage lays out in <500ms on desktop, <2s on PSP."
- Wire into the existing `cargo bench` infrastructure under
  `benches/layout_engine.rs`.
- CI gate on regression > 20%.

**Triage tooling (not in CI):**

- Crawler script: point at a curated list of top-500 sites, record which
  ones panic/error/hang, bucketed by failure mode. Local tool for
  triage, not CI.

---

## Epic: WHATWG HTML conformance

**Effort:** ~1 week. Needs one external test-suite integration.

- **Integrate `html5lib-tests`** (~20k standard tests from the WHATWG
  working group). Add as `tests/html5lib.rs`, allowlist failures we
  can't fix, gate CI on no-regression.
- **Known gaps worth fixing:**
  - Foster parenting is subtly wrong — inserts at the wrong position.
    Should be immediately before the table, not at end of the table's
    parent. See `crates/oasis-browser/src/html/tree_builder/formatting.rs:80`.
  - Adoption agency algorithm is simplified. Handles common formatting
    cases; fails on the adversarial `<b><p></b></p>` reorderings from
    the WHATWG spec examples.
  - No `<template>` element / DocumentFragment support.
  - No SVG/MathML foreign content handling.
  - No parser error reporting — we silently drop malformed input. At
    minimum we should `log::trace!` on parse errors so users can
    diagnose broken pages.
- Full frameset support is **not** a goal — document it as a deliberate
  non-goal.

---

## Epic: Missing CSS features (the long tail)

**Effort:** varies per item. We currently implement ~120 properties;
Blink/WebKit ship ~600. Most of the gap is niche. These are the ones
that cause real breakage on modern sites:

**High-impact, should prioritize:**

- `:has()` selector — shows up in modern sites constantly.
- `@container` queries — most new responsive sites use these.
- `@layer` — cascade layers, used by design systems (Tailwind, etc.).
- CSS nesting (`& .foo { }`) — shipping in Chrome/Firefox.
- `color-mix()`, `oklch()`, `color()`, `light-dark()` functions.
- Logical properties: `margin-inline-start`, `padding-block-end`, etc.
- `text-wrap: balance` / `pretty` — increasingly common for headings.
- `:is()` / `:where()` — check if already supported; audit.
- `aspect-ratio` — audit, may already be supported.

**Medium-impact:**

- `@property` — typed custom property registration.
- `field-sizing: content`.
- `@scope` — shipping in Chrome.
- `counter-style` — rarely breaks rendering but worth parsing.

**Low-impact (skip until someone complains):**

- View Transitions API (`view-transition-*`).
- Anchor Positioning (CSS Anchor Positioning Module Level 1).
- Subgrid.
- `scroll-timeline`, `animation-timeline`.

**Already parsed but not painted — audit needed:**

- `accent-color`, `caret-color` (stored in ComputedStyle; check whether
  form controls actually use them).
- `will-change` — today only sets a boolean hint; should promote to
  layer creation once the compositor lands.

---

## Launch-polish items

These don't show up as CSS properties but bite users first. Not a
single epic — file as individual issues.

- **Font rendering quality across skins** — kerning, hinting, subpixel
  positioning. Especially on PSP where we have system TrueType fonts
  via `psp::font`.
- **Image decoding error recovery** — corrupt JPEG/PNG currently
  crashes the decode path. Should degrade to a placeholder.
- **Network error UX** — timeout, DNS fail, TLS error should produce a
  useful error page, not a blank screen.
- **HTTP/2 support** — we only speak HTTP/1.1. Many modern CDNs require
  h2. Blocks access to some sites entirely.
- **`@font-face` / web fonts** — completely missing. Fallback to system
  fonts works but looks wrong on branded pages.
- **Accessibility** — ARIA roles are parsed but not exposed to anything.
  Low priority for launch but should at least have a plan.

---

## Ranking by ROI for "launch in 1–2 months"

If the constraint is a short runway to public launch, the priority
order is:

1. **Visual regression harness** (biggest leverage per hour of work,
   smallest risk). Catches regressions automatically forever.
2. **`html5lib-tests` integration** (catches tree-builder weirdness in
   one shot, no speculative design needed).
3. **CSS long-tail subset: `:has()` + `@container` + CSS nesting**
   (real-world breakage on modern sites).
4. **Compositor overhaul** (high effort but unlocks mix-blend-mode,
   backdrop-filter, mask, isolation, filter, will-change in one
   architectural change).
5. **3D transforms** (lower user impact — most real sites degrade
   gracefully without them).
6. **PSP JS integration** (high effort, high value on PSP specifically,
   but orthogonal to desktop launch quality).
7. **Launch polish items** (parallel stream, file individually).

---

## Out of scope / non-goals

Document these explicitly so we stop relitigating them:

- **V8-level JS performance on PSP.** Not happening. See PSP JS epic
  for realistic expectations.
- **Service workers, WebRTC, Web Audio API, IndexedDB.** Too much
  surface area for an embedded engine. If something needs these, it's
  not our target use case.
- **Full HTML5 frameset support.** Deliberate non-goal — the web has
  moved on.
- **SVG animation (SMIL).** Parse basic SVG paths only; complex SVG
  rendering is out of scope.
- **CSS Houdini.** Too new, no ecosystem demand.
