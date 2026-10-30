# AGENTS.md

This file provides guidance to all AI agents working on this repository, regardless of which agent is operating. The architecture reference (crate graph, backend traits, crate-by-crate notes, FFI exports, font rendering) lives in [`CLAUDE.md`](CLAUDE.md) and is not repeated here; read it before changing code.

## Project Overview

OASIS_OS is an embeddable operating system framework in Rust (edition 2024). It provides a skinnable shell with a scene-graph UI (SDI), window manager, command interpreter, virtual file system, browser engine, plugin system, and remote terminal. It renders anywhere you provide a pixel buffer and an input stream: SDL3 desktop, WASM, PSP and UE5 (via a C ABI). Built from scratch in Rust starting early 2026, inspired by PSP homebrew shells like PSIX.

Default virtual resolution is 480x272 (PSP native). Skins may override this (e.g. desktop=800x600, xp=1024x768); the backend canvas/window scales to match.

All code changes are authored by AI agents under human direction. No external contributions are accepted (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Build and Test Commands

All CI commands run inside Docker containers. Local development works with cargo directly (SDL3 is compiled from source; requires cmake, g++, and X11/audio dev headers).

```bash
# Build (default features need system ffmpeg dev libs; without them, e.g. on Windows:
#   cargo build -p oasis-app --no-default-features --features javascript,video-decode)
cargo build --release -p oasis-app

# Build via Docker (matches CI)
docker compose --profile ci run --rm rust-ci cargo build --workspace --release

# Tests
cargo test --workspace                      # all tests
cargo test --workspace -- test_name         # single test
cargo test -p oasis-core                    # single crate

# Formatting
cargo fmt --all -- --check                  # check only
cargo fmt --all                             # apply

# Linting (CI treats warnings as errors and lints all targets). Run the Docker
# variant before opening a PR: CI pins rust:1.99-slim and lints change between
# releases.
cargo clippy --workspace --all-targets -- -D warnings
docker compose --profile ci run --rm rust-ci cargo clippy --workspace --all-targets -- -D warnings

# License/advisory audit
cargo deny check

# PSP backend / overlay PRX (not workspace members; nightly + cargo-psp)
cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release
cd crates/oasis-plugin-psp && RUST_PSP_BUILD_STD=1 cargo psp --release

# UE5 FFI shared library
cargo build --profile release-ffi -p oasis-ffi

# WASM backend (requires wasm-pack)
./scripts/build-wasm.sh --release

# Screenshots
cargo run -p oasis-app --bin oasis-screenshot
```

## CI Pipeline

All steps run via `docker compose --profile ci run --rm rust-ci` on self-hosted runners.

**`main-ci.yml` (push to `main`):** format check -> clippy -> `oasis-app` feature-combination check -> non-default feature-set clippy (`oasis-browser`, `oasis-core`, `oasis-video`, `oasis-app`, `oasis-ffi`) -> nightly clippy (advisory) -> doc build -> markdown link check -> test (cargo-nextest + doc-tests) -> release build -> screenshot regression -> cargo-deny -> benchmarks -> code coverage (advisory) -> rust-psp pin check (crates, workflows, `docker/psp-build.Dockerfile`) -> PSP EBOOT / overlay PRX / recovery EBOOT builds (rustc warnings are errors) -> PPSSPP headless smoke test; then release binaries and the GitHub Pages deploy (WASM).

**`pr-validation.yml` (pull requests):** the same checks (with a benchmark regression gate), plus:

1. Claude Code security review and Claude Code quality review
2. OpenRouter general review (advisory, `continue-on-error`)
3. **Review Response Agent** reads the review artifacts and auto-fixes issues (via `automation-cli`), max 5 iterations
4. **Failure Handler Agent** auto-fixes CI failures, max 5 iterations
5. Iteration tracking via `.github/actions/agent-iteration-check/` -- admins can comment `[CONTINUE]` to extend limits
6. The `no-auto-fix` label disables the automated fix agents

Fork PRs are blocked from the self-hosted runners (fork guard).

**Additional workflows:** `memory-ci.yml` (ASAN + Valgrind massif, non-blocking), `nightly-streaming.yml` (TV Guide streaming end to end), `fuzz.yml`.

## Code Conventions

- **MSRV:** 1.91.0 (uses `str::floor_char_boundary`; `msrv` in `clippy.toml`)
- **Edition:** 2024 (let-chains, `if let ... &&` syntax is used throughout)
- **Max line width:** 100 characters
- **Formatting:** `cargo fmt` with `rustfmt.toml` -- 4-space indent, Unix newlines, `fn_params_layout = "Tall"`, `match_block_trailing_comma = true`
- **Linting:** Clippy warnings are CI errors (`-D warnings`, all targets). Workspace lints: `clone_on_ref_ptr`, `dbg_macro`, `todo`, `unimplemented` = warn; `unsafe_op_in_unsafe_fn` = warn; `unwrap_used` = **deny** (allowed in `#[test]` / `#[cfg(test)]` code via `clippy.toml`)
- **Unsafe:** All unsafe blocks require `// SAFETY:` comments. Unsafe is limited to FFI boundaries, SDL texture handles, and PSP system calls.
- **Tests:** Unit tests live in-module (`#[cfg(test)] mod tests`); a few crates also have `tests/` integration suites and criterion `benches/`. Desktop-shell user-flow changes get an e2e scenario in `crates/oasis-app/tests/e2e_*.rs` ([docs/testing.md](docs/testing.md)). About 9,000 `#[test]` functions across the repository.
- **Docs:** Update the docs that describe the code you change (see the [docs index](docs/README.md)); CI's markdown link check fails on broken relative links.
- **License:** Dual-licensed Unlicense + MIT

## Multi-Agent System

### Agents

| Agent | Runtime | Role | Status |
|-------|---------|------|--------|
| Claude | Host (Claude Code CLI) | Primary development, PR creation, PR security and quality review, complex tasks | Active |
| OpenCode | Containerized | Code generation, issue implementation | Active |
| Crush | Containerized | Quick code generation, conversion | Active |
| Gemini | Host (Gemini CLI) | Second-opinion consultation via MCP; not used for PR review | Optional |
| Codex | Containerized | -- | **DISABLED** -- see security notice below |

Priorities (from `.agents.yaml.example`): issue creation / PR authoring: Claude > OpenCode; PR reviews: Claude (`pr-validation.yml` runs the Claude security + quality reviews and an OpenRouter general review); code fixes: Claude > Crush > OpenCode.

### MCP Servers

Configured in `.mcp.json`, all run as Docker containers via `docker compose --profile services`:

| Server | Purpose |
|--------|---------|
| code-quality | Linting, formatting, testing, security scanning, type checking |
| content-creation | LaTeX, TikZ, Manim rendering |
| gemini | Gemini AI consultation (second opinion) |
| codex | Codex AI consultation **(DISABLED)** |
| opencode | OpenCode AI consultation |
| crush | Crush AI consultation |
| github-board | GitHub Projects board management (issue claiming, status tracking) |
| agentcore-memory | Persistent agent memory (AWS/ChromaDB) |
| reaction-search | Reaction image search |

This is separate from OASIS_OS's own optional in-app MCP server (`oasis-mcp`, [docs/mcp-server.md](docs/mcp-server.md)), which lets an agent drive a running shell.

### Security Notice: OpenAI / Codex / GPT Phase-Out

**OpenAI Codex has been disabled across all pipelines.** OpenAI has entered partnerships with government agencies that enable mass surveillance and autonomous weapons decision-making. The mass surveillance risk alone -- where code, prompts, and context sent through OpenAI APIs may be accessible to nation-state actors -- is unacceptable for this project and its users.

We strongly recommend **Anthropic (Claude)** models for all AI-assisted workflows. See the [README security notice](README.md#security-notice).

To re-enable at your own risk: set `CODEX_ENABLED=true` in your environment.

### Agent Tooling and Security

- `tools/cli/agents/` -- run scripts for each agent; `tools/cli/containers/` -- containerized agent run scripts
- `.agents.yaml` -- multi-agent configuration (copy `.agents.yaml.example`); `.env` -- environment variables (copy `.env.example`)
- `agent_admins` in `.agents.yaml` controls who can trigger agent actions via `[Approved][Agent]` keywords; `trusted_sources` controls whose comments are marked trusted for AI context
- All agents run in sandboxed environments (`autonomous_mode: true`, `require_sandbox: true`)
- Agent commit authors: "AI Review Agent", "AI Pipeline Agent", "AI Agent Bot"

## Docker Services

`docker-compose.yml` profiles:
- **`ci`** -- rust-ci container (rust:1.99-slim + cmake + X11/audio dev libs + nightly + cargo-deny)
- **`psp`** -- PPSSPP emulator (multi-stage build, NVIDIA GPU passthrough, X11 forwarding)
- **`services`** -- all MCP server containers
- **`memory`** -- AgentCore memory service (also included in `services`)

## Key Files

- [`CLAUDE.md`](CLAUDE.md) -- architecture reference, build commands, conventions
- [`docs/README.md`](docs/README.md) -- index of every document; [`docs/design.md`](docs/design.md) -- technical design (v3.0)
- `skins/` -- the 17 skin directories (every built-in skin; TOML reference in [`docs/skin-authoring.md`](docs/skin-authoring.md))
- `clippy.toml` -- lint thresholds (cognitive complexity 25, too-many-lines 100, too-many-args 7) and MSRV
- `rustfmt.toml` -- formatting rules; `deny.toml` -- license and advisory policy
- `.pre-commit-config.yaml` -- pre-commit hooks (whitespace / YAML / JSON / large-file checks, actionlint, shellcheck, and host-toolchain `cargo fmt`, clippy, `cargo doc -D warnings`, markdown link check; `cargo deny` on pre-push)
- [`CHANGELOG.md`](CHANGELOG.md) -- notable changes; [`CONTRIBUTING.md`](CONTRIBUTING.md) -- contribution policy (AI-authored only)
- [`scripts/psp-scenarios.md`](scripts/psp-scenarios.md) -- PSP test scenarios
