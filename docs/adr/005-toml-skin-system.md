# ADR-005: TOML Skin System

**Status:** Accepted
**Date:** 2025-02-12
**Last reviewed:** 2026-05-03 — decision still current; skin counts in Context and Consequences updated below to match current `crates/oasis-skin/src/builtin/` and `skins/`.

## Context

OASIS_OS needs a theming system to support multiple visual styles. Early
prototypes had hardcoded color schemes. Users expect customizable themes.

Requirements:
- Hot-swappable at runtime (no recompilation)
- Human-editable by non-programmers
- Embeddable in the binary for built-in skins
- Expressive enough for 15 distinct visual styles (the current built-in count)

## Decision

Use a **TOML-based skin system** with four files per skin:

- **`skin.toml`** -- manifest (name, version, metadata)
- **`layout.toml`** -- SDI scene object definitions (positions, sizes, textures)
- **`features.toml`** -- feature flags, transitions, dashboard config
- **`theme` section** -- colors, gradients, fonts, window manager overrides

Skins are resolved by name or path. Built-in skins use `include_str!()` to embed
TOML at compile time.

## Rationale

- **TOML is human-readable.** Non-programmers can tweak colors without Rust
  knowledge. JSON lacks comments; YAML has footgun indentation.
- **`include_str!()` embedding.** Built-in skins compile into the binary with
  zero runtime cost. No file I/O needed for defaults.
- **Separation of concerns.** Layout (where things are), theme (how things look),
  and features (what's enabled) are independent axes. Changing colors doesn't
  require understanding layout definitions.
- **Theme derivation.** The `ActiveTheme::from_skin()` function derives 50+
  specific colors from 9 base colors. Skin authors set a handful of primary
  colors; the system fills in button highlights, shadow colors, text contrasts.
- **Hot-swap.** Skins are loaded and applied at runtime via the `skin` terminal
  command. The SDI scene is rebuilt from the new layout; the theme updates all
  color references.

## Alternatives Considered

- **Compiled Rust themes** (const structs). Rejected: requires recompilation for
  any change. Users can't create custom skins without a Rust toolchain.
- **CSS-based theming.** Rejected: CSS is designed for document styling, not
  for configuring a desktop shell. TOML's key-value structure maps better to
  theme properties.
- **JSON.** Rejected: no comments, no multi-line strings, harder to hand-edit.

## Consequences

- `oasis-skin` crate owns parsing, resolution, and theme derivation.
- 15 built-in skins ship today: `classic`, `corrupted`, `desktop`, `modern`,
  `xp`, `macos`, `gnome`, `retro-cga`, `balatro`, `paper`, `win95`,
  `solarized`, `vaporwave`, `highcontrast`, `altimit`. Twelve of them
  (`altimit`, `balatro`, `classic`, `gnome`, `highcontrast`, `macos`,
  `paper`, `retro-cga`, `solarized`, `vaporwave`, `win95`, `xp`) also ship
  as external TOML directories under `skins/` and demonstrate the
  filesystem-loading path; the other three (`corrupted`, `desktop`,
  `modern`) are built-in only via Rust constructors.
- The `ActiveTheme` struct provides runtime-mutable access to all derived colors.
- Adding a new skin requires only TOML files -- no Rust code changes.
