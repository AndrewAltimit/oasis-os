# OASIS_OS Documentation

Index of everything under `docs/`. New here? Start with
[getting-started.md](getting-started.md), then skim [design.md](design.md).
Runnable code samples live in [`examples/`](../examples/README.md).

## Guides

| Doc | What it covers |
|-----|----------------|
| [getting-started.md](getting-started.md) | Dependencies and build/run instructions for desktop, WASM, UE5/FFI and PSP; feature flags; tests and lint |
| [writing-apps.md](writing-apps.md) | Writing an app: the `App` trait lifecycle and hooks, idle-frame rules, a minimal example, registration |
| [skin-authoring.md](skin-authoring.md) | Writing a skin: directory layout and the full TOML reference |
| [adding-commands.md](adding-commands.md) | Adding a terminal command |
| [plugin-development.md](plugin-development.md) | Writing a runtime plugin |
| [ffi-integration.md](ffi-integration.md) | Embedding via the C ABI (`oasis-ffi`), full API reference, UE5 outline |
| [wasm-backend.md](wasm-backend.md) | Building, serving and embedding the WebAssembly build; input mapping; iframe overlay |
| [psp-plugin.md](psp-plugin.md) | The kernel-mode PSP overlay PRX: install, configuration, build |
| [troubleshooting.md](troubleshooting.md) | Common build and runtime problems |
| [security.md](security.md) | Security model, threat model and mitigations |

## Architecture and subsystems

| Doc | What it covers |
|-----|----------------|
| [design.md](design.md) | Technical design document: overall architecture |
| [browser-engine.md](browser-engine.md) | `oasis-browser` feature catalogue (HTML, CSS, layout, fonts, JS bindings) |
| [browser-backlog.md](browser-backlog.md) | Browser engine backlog and recently shipped epics |
| [javascript-engine.md](javascript-engine.md) | QuickJS-NG integration and the PSP cross-compile |
| [oasis-js.md](oasis-js.md) | `oasis-js` desktop API and DOM bindings catalogue |
| [terminal-commands.md](terminal-commands.md) | Catalogue of built-in terminal commands by module |
| [ui-widgets.md](ui-widgets.md) | `oasis-ui` widget catalogue with source paths |
| [window-manager.md](window-manager.md) | `oasis-wm` API: windows, snapping, keyboard shortcuts, tiling, animations, decorations |
| [networking.md](networking.md) | TCP transport, PSK auth, remote terminal, TLS, file transfer |
| [mcp-server.md](mcp-server.md) | Optional MCP control server for agent-driven UI |
| [audio-engine.md](audio-engine.md) | Audio manager, playlists, radio, streaming back-pressure |
| [video-streaming.md](video-streaming.md) | MP4/H.264 + AAC progressive streaming (desktop focus) |
| [vector-graphics.md](vector-graphics.md) | `oasis-vector` scene, icons, animation, `SdiVector` integration |
| [shaders.md](shaders.md) | Shader wallpapers, GPU and software renderers |
| [boot-splash.md](boot-splash.md) | Functional desktop boot splash and its phase probes |
| [theming-desktop-plan.md](theming-desktop-plan.md) | Advanced theming and desktop-metaphor plan (in progress) |
| [examples/oasis.mcp.json](examples/oasis.mcp.json) | Example MCP client config (used by mcp-server.md) |

## PSP

| Doc | What it covers |
|-----|----------------|
| [psp-architecture.md](psp-architecture.md) | PSP target constraints: two binaries, GU, QuickJS, TLS 1.3, ME video decode |
| [psp-autorun.md](psp-autorun.md) | Boot-time script runner for deterministic PPSSPP / hardware tests |
| [psp-me-driver-map.md](psp-me-driver-map.md) | Media Engine research: driver chain and NID map |
| [psp-me-firmware-analysis.md](psp-me-firmware-analysis.md) | Media Engine research: runtime firmware analysis |
| [psp-me-rpc-api.md](psp-me-rpc-api.md) | Media Engine research: RPC API reference |
| [psp-usb-hardware-reference.md](psp-usb-hardware-reference.md) | USB controller hardware reference (register-level) |
| [psp-usb-vbus-findings.md](psp-usb-vbus-findings.md) | USB VBUS power output research findings |
| [../scripts/psp-scenarios.md](../scripts/psp-scenarios.md) | PSP test scenarios (lives in `scripts/`) |

## Architecture decision records

See [adr/README.md](adr/README.md) for the index with statuses.

| ADR | Decision |
|-----|----------|
| [001](adr/001-arena-based-dom.md) | Arena-based DOM |
| [002](adr/002-vfs-abstraction.md) | Virtual file system abstraction |
| [003](adr/003-backend-trait-design.md) | Backend trait design |
| [004](adr/004-psp-two-binary-architecture.md) | PSP two-binary architecture (EBOOT + PRX) |
| [005](adr/005-toml-skin-system.md) | TOML skin system |

## Archive

Completed, shipped or superseded plans and investigations, kept for history.
Each starts with a banner stating its outcome.

| Doc | Outcome |
|-----|---------|
| [archive/compositor-overhaul-plan.md](archive/compositor-overhaul-plan.md) | Shipped (browser compositing layers over `SdiRenderTarget`) |
| [archive/psp-video-decode-plan.md](archive/psp-video-decode-plan.md) | Superseded by the `sceMpeg` NAL-direct decode path |
| [archive/psp-me-direct-plan.md](archive/psp-me-direct-plan.md) | Superseded: direct ME programming proved unnecessary |
| [archive/psp-me-decode-next-steps.md](archive/psp-me-decode-next-steps.md) | Completed: hardware H.264 decode working |
| [archive/psp-child-module-investigation.md](archive/psp-child-module-investigation.md) | Resolved: O32/EABI32 argument-passing fix in rust-psp |

## Elsewhere in the repo

- [`../README.md`](../README.md) -- project overview
- [`../CLAUDE.md`](../CLAUDE.md) -- developer / agent reference (build commands, architecture, conventions)
- [`../AGENTS.md`](../AGENTS.md) -- multi-agent workflow and CI
- [`../CONTRIBUTING.md`](../CONTRIBUTING.md) -- contribution policy
- [`../examples/README.md`](../examples/README.md) -- runnable examples
