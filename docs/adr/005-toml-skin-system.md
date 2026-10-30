# ADR-005: TOML Skin System

**Status:** Accepted
**Date:** 2026-02-09 (initial release; recorded 2026-02-19)
**Last reviewed:** 2026-10-30 — decision still current. Consequences updated: every built-in skin is generated from its `skins/` directory (the hand-written Rust loaders and constants were removed), resolution is strict, derived text colors are kept readable, and `skin lint` must pass on every built-in.

## Context

OASIS_OS needs a theming system to support multiple visual styles. Early
prototypes had hardcoded color schemes. Users expect customizable themes.

Requirements:
- Hot-swappable at runtime (no recompilation)
- Human-editable by non-programmers
- Embeddable in the binary for built-in skins
- Expressive enough for 17 distinct visual styles (the current built-in count)

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
- 17 built-in skins ship today, and all 17 are directories under `skins/`
  (`altimit`, `balatro`, `classic`, `corrupted`, `desktop`, `gnome`,
  `highcontrast`, `macos`, `modern`, `paper`, `psix-hifi`, `psix-tribute`,
  `retro-cga`, `solarized`, `vaporwave`, `win95`, `xp`). Those directories are
  the single source of truth: `oasis-skin`'s build script embeds each one with
  `include_str!` / `include_bytes!` (plus `corrupted.toml` when present) and
  generates the loaders and `builtin_names()`. No skin is defined in Rust, so a
  built-in and its directory form cannot drift apart. (Until 2026-10 three
  skins existed only as Rust constants and `xp`'s constants had diverged from
  `skins/xp/`; a snapshot test now pins what every built-in parses to.)
  `psix-tribute` and `psix-hifi` use manifest inheritance
  (`inherits = "classic"`).
- `resolve_skin` is strict (unknown or broken skin = error);
  `resolve_skin_or_classic` is the boot-time variant that falls back to
  `classic`.
- Derived (not skin-specified) text colors are contrast-checked against the
  surface they are drawn on and nudged toward black / white only when they
  fall short (`theme::readable_on`); explicit skin colors are never changed.
- `skin lint` (`Skin::validate`, split by concern under
  `oasis-skin/src/loader/validation/`) covers schema, colors anywhere in
  `theme.toml`, enumerated names that would otherwise fall back silently,
  assets, and advisory WCAG contrast. Every built-in must lint clean apart
  from the documented `ACCEPTED_CONTRAST` entries (CI test
  `all_builtin_skins_lint_clean`).
- The `ActiveTheme` struct provides runtime-mutable access to all derived colors.
- Adding a new skin requires only TOML files -- no Rust code changes. Adding a
  complete directory under `skins/` makes it a built-in.

## Update (2026-10-30)

The Decision section lists four parts; a skin directory today holds
`skin.toml` (manifest, including the optional `inherits = "<parent>"`),
`layout.toml`, `features.toml`, `theme.toml` (its own file, not a section)
and `strings.toml`, plus an optional `corrupted.toml` and an `assets/`
directory for images and fonts. The full reference is
[`skin-authoring.md`](../skin-authoring.md).
