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

**Effort:** 1–3 weeks. **Risk:** high (MIPS codegen quirks, debugging on
real hardware). **Value:** enables a large class of modern sites that use
JS for content hydration.

QuickJS-NG has been ported to ESP32-class microcontrollers (520KB SRAM,
4MB flash). PSP has 24MB user RAM + 333MHz MIPS — strictly more than
ESP32. The RAM argument does not hold up; the real obstacles are build
pipeline and MIPS codegen, not memory budget.

**Phase 1 — Feasibility: does QuickJS compile for `mipsel-sony-psp`?**

- Add `quickjs-ng` as a C dependency via the `cc` crate in a standalone
  test binary under `crates/oasis-backend-psp/tests/`.
- Verify `cc` emits usable MIPS code for QuickJS's large switch tables,
  `setjmp`/`longjmp` usage, and bytecode dispatch loop.
- Expect to hit LLVM MIPS codegen issues similar to the existing
  "manual byte loops needed for memcpy/memset" constraint. Budget a
  week just for this phase — it's the risk-heavy step.

**Phase 2 — Thin FFI wrapper (not `rquickjs`).**

- `rquickjs` uses `std::time::Instant`, which **crashes on PSP
  Allegrex** (documented in memory). Do not try to port `rquickjs`.
- Write a narrow FFI crate exposing just what `oasis-js` needs: engine
  init, eval, function calls, object property get/set, callback
  registration. Route all time sources through `sceKernelGetSystemTimeLow`.
- Configure QuickJS memory limit at ~1MB initial heap, grow to ~4MB
  max. Leaves ~18–20MB for browser/video/SDI/audio.

**Phase 3 — Wire through `oasis-browser` PSP build.**

- Gate on a new `psp-quickjs` feature flag on `oasis-js`.
- Enable the `javascript` feature on `oasis-browser` in the PSP Cargo.toml.
- Audit the DOM binding layer for PSP-hostile code (threading, time, fs).
- Update the `# Feature flags` docstring in `src/lib.rs` and the
  `oasis-backend-psp/Cargo.toml` comment to reflect the new state.

**Non-goals:** V8-level performance. Expect 500–1000× slower than
desktop. Inert pages with small bootstrap scripts will work; React SPAs
will be unusable. That's fine — degraded is better than dead.

**Testing:** PPSSPP headless for smoke tests; real hardware for
performance. Budget extra time for debugging — QuickJS stack traces on
MIPS are not fun.

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
