# OASIS_OS Documentation

Map of everything under `docs/` (plus the few docs that live next to the
code). Every document here describes the current code unless it is in the
[Archive](#archive-historical-not-current) at the bottom.

New here? Read [getting-started.md](getting-started.md) to build and run it,
then [design.md](design.md) for how the pieces fit. Runnable code samples
live in [`examples/`](../examples/README.md).

## I want to...

| Goal | Read |
|------|------|
| Build and run the desktop shell, WASM, UE5 library or PSP EBOOT | [getting-started.md](getting-started.md) |
| Fix a build or setup problem | [troubleshooting.md](troubleshooting.md) |
| Understand the architecture | [design.md](design.md), then the [ADRs](#architecture-decision-records) |
| Write a dashboard app | [writing-apps.md](writing-apps.md) |
| Add a terminal command | [adding-commands.md](adding-commands.md) |
| Extend the shell at runtime with a plugin | [plugin-development.md](plugin-development.md) |
| Create or change a skin | [skin-authoring.md](skin-authoring.md) |
| Test a change (unit, app e2e, shell e2e, backend parity) | [testing.md](testing.md) |
| Embed OASIS_OS in UE5 or another C host | [ffi-integration.md](ffi-integration.md) |
| Let an agent drive a running shell | [mcp-server.md](mcp-server.md) |
| Work on the PSP build | [psp-architecture.md](psp-architecture.md) |
| Know what is defended and what is not | [security.md](security.md) |

## Start here

| Doc | What it covers |
|-----|----------------|
| [getting-started.md](getting-started.md) | Dependencies per OS, build / run for desktop (incl. the ffmpeg-free Windows build), WASM, UE5 / FFI and PSP (Docker, `prxmin`); feature flags, environment variables, tests and lint |
| [design.md](design.md) | Technical design document: goals, layering, backend drawing-state contract, platform notes; links out to every subsystem doc |
| [troubleshooting.md](troubleshooting.md) | Common build, Docker, PSP-toolchain and runtime problems |

## Extending OASIS_OS

| Doc | What it covers |
|-----|----------------|
| [writing-apps.md](writing-apps.md) | The `App` trait: hooks, lifecycle and frame order, idle-frame rules, a minimal example, registering in `oasis-apps` |
| [adding-commands.md](adding-commands.md) | Adding a terminal command: the `Command` trait, registration layers, tests |
| [plugin-development.md](plugin-development.md) | Writing a compiled-in plugin: `PluginHost`, capabilities, VFS IPC, plugin apps, manifests |
| [skin-authoring.md](skin-authoring.md) | Writing a skin: directory layout and the full TOML reference |
| [testing.md](testing.md) | Test layers, test backends, the `AppHarness` and the headless shell harness (`oasis_app::harness`), backend parity suites |

## Hosts and platforms

| Doc | What it covers |
|-----|----------------|
| [ffi-integration.md](ffi-integration.md) | Embedding via the C ABI (`oasis-ffi`): full API reference, event codes, UE5 outline |
| [wasm-backend.md](wasm-backend.md) | Building, serving and embedding the WebAssembly build; input mapping; iframe overlay; YouTube feature |
| [psp-architecture.md](psp-architecture.md) | PSP constraints and design: two binaries, GU command buffer, QuickJS cross-compile, TLS 1.3, ME video decode |
| [psp-plugin.md](psp-plugin.md) | The kernel-mode PSP overlay PRX: install, configuration, build, memory budget |
| [psp-autorun.md](psp-autorun.md) | Boot-time script runner for deterministic PPSSPP / hardware tests |
| [../scripts/psp-scenarios.md](../scripts/psp-scenarios.md) | PSP test scenarios (lives in `scripts/`) |

## Subsystems

| Doc | What it covers |
|-----|----------------|
| [browser-engine.md](browser-engine.md) | `oasis-browser` feature catalogue: HTTP, HTML, CSS, layout, fonts, chrome, JS bindings |
| [browser-backlog.md](browser-backlog.md) | Browser engine open work |
| [../crates/oasis-browser/docs/README.md](../crates/oasis-browser/docs/README.md) | Browser engine internals (architecture, HTML / DOM, CSS pipeline and coverage, layout, paint, loading, Gemini, JavaScript, testing) across `oasis-browser`, `oasis-html`, `oasis-css` and `oasis-http` |
| [javascript-engine.md](javascript-engine.md) | QuickJS-NG integration and the PSP cross-compile |
| [oasis-js.md](oasis-js.md) | `oasis-js` desktop API and DOM bindings catalogue |
| [networking.md](networking.md) | TCP transport, PSK challenge-response, remote terminal protocol, TLS, FTP-like file transfer |
| [mcp-server.md](mcp-server.md) | Optional MCP control server: enabling, transport, tool catalogue, security |
| [audio-engine.md](audio-engine.md) | `AudioManager`, playlists, repeat / shuffle, internet radio, streaming back-pressure |
| [video-streaming.md](video-streaming.md) | MP4 / H.264 + AAC decode features and desktop progressive streaming |
| [window-manager.md](window-manager.md) | `oasis-wm` API: windows, snapping, keyboard shortcuts, tiling, animations, decorations |
| [vector-graphics.md](vector-graphics.md) | `oasis-vector` scene, icons, animation, `SdiVector` integration |
| [shaders.md](shaders.md) | Shader wallpapers: the built-ins, GPU and software renderers, adding one |
| [boot-splash.md](boot-splash.md) | Functional desktop boot splash: BIOS-phase probes, splash-phase warm-up, `BootSplash` API |
| [i18n.md](i18n.md) | `oasis-i18n`: locales, selecting a locale, adding strings and translations |

## Reference catalogues

| Doc | What it covers |
|-----|----------------|
| [terminal-commands.md](terminal-commands.md) | Every built-in terminal command and shell builtin, by module |
| [ui-widgets.md](ui-widgets.md) | `oasis-ui` widget catalogue with source paths |
| [examples/oasis.mcp.json](examples/oasis.mcp.json) | Example MCP client config (used by mcp-server.md) |

## Policy

| Doc | What it covers |
|-----|----------------|
| [security.md](security.md) | Threat model, VFS / FFI / terminal / browser / network defenses, known gaps |
| [../AGENTS.md](../AGENTS.md) | Multi-agent workflow, CI overview, conventions, Docker services |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Contribution policy (AI-authored only) |

## Architecture decision records

Each ADR keeps its original decision and adds dated "Update" notes where the
code has moved on. Statuses: [adr/README.md](adr/README.md).

| ADR | Decision |
|-----|----------|
| [001](adr/001-arena-based-dom.md) | Arena-based DOM (`NodeId` indices into a node arena) |
| [002](adr/002-vfs-abstraction.md) | Virtual file system abstraction (`Vfs` trait, three implementations, shared invariants) |
| [003](adr/003-backend-trait-design.md) | Backend trait design (`SdiCore` + eight extension traits, drawing-state contract) |
| [004](adr/004-psp-two-binary-architecture.md) | PSP two-binary architecture (EBOOT shell + kernel-mode PRX overlay) |
| [005](adr/005-toml-skin-system.md) | TOML skin system (skin directories as the single source of truth) |

## PSP research notes

Reverse-engineering notes behind the PSP video and USB work. They record
findings about Sony firmware and hardware, not OASIS_OS behavior.

| Doc | What it covers |
|-----|----------------|
| [psp-me-driver-map.md](psp-me-driver-map.md) | Media Engine: driver chain and NID map |
| [psp-me-firmware-analysis.md](psp-me-firmware-analysis.md) | Media Engine: runtime firmware analysis |
| [psp-me-rpc-api.md](psp-me-rpc-api.md) | Media Engine: RPC API reference |
| [psp-usb-hardware-reference.md](psp-usb-hardware-reference.md) | USB controller hardware reference (register level) |
| [psp-usb-vbus-findings.md](psp-usb-vbus-findings.md) | USB VBUS power output findings |
| [../crates/oasis-usb-debug-psp/README.md](../crates/oasis-usb-debug-psp/README.md) | PSP USB host-mode / VBUS debug EBOOTs |

## Elsewhere in the repo

- [`../README.md`](../README.md) -- project overview and crate table
- [`../CLAUDE.md`](../CLAUDE.md) -- developer / agent reference: build commands, crate graph, canonical CI order, conventions
- [`../CHANGELOG.md`](../CHANGELOG.md) -- notable changes by release / branch
- [`../SCREENSHOTS.md`](../SCREENSHOTS.md) -- every built-in skin, a screenshot per app
- [`../examples/README.md`](../examples/README.md) -- runnable examples
- [`../crates/oasis-core/fuzz/README.md`](../crates/oasis-core/fuzz/README.md) -- fuzz targets (run nightly by `fuzz.yml`)
- [`../docker/ppsspp-patches/README.md`](../docker/ppsspp-patches/README.md) -- where PPSSPP source patches for the emulator image go
- [`../tools/cli/containers/README.md`](../tools/cli/containers/README.md) -- running the OpenCode agent in a container

## Archive (historical, not current)

Completed, shipped or superseded plans and investigations, kept because they
explain why the shipped design looks the way it does. **They do not describe
current behavior**: each starts with a banner stating its outcome and pointing
at the current doc. [theming-desktop-plan.md](theming-desktop-plan.md) (all
milestones landed) stays next to the live docs as the theme engine's design
rationale but is likewise a historical plan record.

| Doc | Outcome |
|-----|---------|
| [archive/compositor-overhaul-plan.md](archive/compositor-overhaul-plan.md) | Shipped (browser compositing layers over `SdiRenderTarget`) |
| [archive/psp-video-decode-plan.md](archive/psp-video-decode-plan.md) | Superseded by the `sceMpeg` NAL-direct decode path |
| [archive/psp-me-direct-plan.md](archive/psp-me-direct-plan.md) | Superseded: direct ME programming proved unnecessary |
| [archive/psp-me-decode-next-steps.md](archive/psp-me-decode-next-steps.md) | Completed: hardware H.264 decode working |
| [archive/psp-child-module-investigation.md](archive/psp-child-module-investigation.md) | Resolved: O32/EABI32 argument-passing fix in rust-psp |
| [theming-desktop-plan.md](theming-desktop-plan.md) | Completed (M0–M7): asset pipeline, free desktop icons, chrome and motion, TTF fonts, appearance editor, accessibility tokens |
