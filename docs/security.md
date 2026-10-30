# OASIS_OS Security Model

This document describes the security boundaries, threat model, and
mitigation strategies used throughout OASIS_OS.

## Threat Model

OASIS_OS is an embeddable OS framework that renders to a host-provided
pixel buffer. The primary deployment targets are:

- **PSP homebrew** -- single-user, no network isolation
- **UE5 game integration** -- sandboxed inside a game process via FFI
- **WASM browser** -- sandboxed by the browser runtime
- **Desktop SDL3** -- developer/demo use

The framework does NOT provide OS-level process isolation or privilege
separation. Security focuses on:

1. Preventing the VFS from escaping its sandbox
2. Ensuring FFI callers cannot trigger undefined behavior
3. Sanitizing untrusted input (HTML/CSS, terminal commands)

## VFS Path Traversal Defense

The virtual file system (`oasis-vfs`) uses a three-layer defense
against path traversal attacks:

### Layer 1: Path Canonicalization

All paths are normalized before use. `..` components are resolved
against the current working directory, and the result is always an
absolute path starting with `/`.

### Layer 2: Permission Enforcement

`FilePermissions` tracks an owner and Unix-style mode bits, checked against
the caller's `VfsContext` by `Vfs::check_permission`. `MemoryVfs` and
`GameAssetVfs` enforce them on reads, writes, `readdir()`, `mkdir()`,
`remove()` and both parents of a `rename()`. Files default to `0o644`
(owner rw, others r) and directories to `0o755`.

### Layer 3: Jail Roots

`GameAssetVfs` (UE5) restricts all operations to a set of declared
base directories. Writes create overlay entries that never touch the
underlying asset files.

`RealVfs` maps a host directory. No shipped host mounts it (the desktop
shell, WASM build and FFI library use `MemoryVfs` / `GameAssetVfs`), but
it is jailed anyway: `resolve` canonicalizes the deepest existing ancestor
of a path (lstat-based, so dangling symlinks fail), checks it is under the
root and only then appends the missing tail, so `mkdir("/link/new/x")`
with `link` pointing outside the root is refused. Renames check the
resolved paths too.

### Shared rename invariants

`Vfs::validate_rename` (a provided trait method every implementation
calls) refuses renaming `/`, a missing source, a destination inside the
source, a missing or non-directory destination parent, an existing
directory destination, and a directory onto a file.
`tests/vfs_conformance.rs` runs the same suite against all three
implementations. See [ADR-002](adr/002-vfs-abstraction.md).

## FFI Boundary Safety

The C-ABI boundary in `oasis-ffi` follows these rules:

- Every `extern "C"` function documents its safety contract in a
  `# Safety` doc comment
- Every `unsafe` block has a `// SAFETY:` comment explaining why the
  invariants hold
- Null handles are checked at the top of every function (early return)
- Null string pointers are checked before `CStr::from_ptr`
- Returned strings are allocated via `CString::into_raw` and freed
  by `oasis_free_string` via `CString::from_raw`
- No panics cross the FFI boundary (all operations use `Result` or
  return sentinel values)

### Opaque Handle Pattern

C callers never see Rust types. They receive an opaque
`*mut OasisInstance` from `oasis_create` and pass it to every
subsequent call. The Rust side reconstructs the reference via
`handle.as_mut()`, which returns `None` for null pointers.

## Terminal Input Sanitization

The command interpreter in `oasis-terminal` sanitizes input at
multiple levels:

- **Variable expansion** -- `$VAR` references are expanded before
  execution; unknown variables expand to empty strings
- **Glob expansion** -- `*` and `?` patterns are expanded against
  VFS entries (not the real filesystem)
- **Pipe chains** -- commands connected by `|` run sequentially
  with stdout piped to stdin; each command is isolated
- **Command substitution** -- `$(...)` executes the inner command
  and substitutes its output

Commands that modify system state (e.g. `chmod`, `write`) operate
only on the VFS, never on the host filesystem.

## Browser Engine Sandboxing

The HTML/CSS engine (`oasis-browser`) processes untrusted content:

- **Scripts** -- JavaScript (feature `javascript`, on in every shipped
  build) runs in QuickJS-NG with no host filesystem access. The only
  network path is `fetch()`, which is origin-checked: CORS, `connect-src`,
  no credentials in URLs, and no requests to a more private address class
  than the page's own (public < private / link-local < loopback).
  Each tick has a time budget and the heap is capped
  (`BrowserConfig::js_max_exec_ms` / `js_memory_limit_bytes`: 2 s and
  64 MiB on desktop, 500 ms and 8 MiB on PSP); timers still due when the
  budget runs out are deferred to the next tick
- **DOM depth cap** -- the parser (SVG / MathML content included) and
  script DOM insertion both stop at `MAX_DOM_DEPTH`, so script-built
  chains cannot overflow the stack in the recursive cascade / layout /
  paint passes; text and comment nodes never take children. Scripts may
  grow the document to at most 200k live nodes (`QuotaExceededError`
  past it). Freed DOM slots are tracked, so a stale node reference can't
  be re-inserted. The CSS selector matcher has a per-match traversal
  budget that also pays for sibling-list scans; running out never counts
  as a match (not even under `:not()`)
- **No nested browsing contexts** -- `<iframe>` content is never loaded
- **CSS property limits** -- unknown properties are ignored; no
  `expression()` or `-moz-binding` support
- **Image loading** -- images are decoded in-process with bounds
  checking on dimensions
- **Content Security Policy** -- `script-src` / `style-src` (with
  nonces and `'sha256-…'` hashes) gate inline scripts, event handlers
  (each checked with its own text, so `'unsafe-hashes'` works),
  `<style>`, `style=""` and linked stylesheets; `connect-src` gates
  `fetch()` (every redirect hop, and `data:` URLs). The policy a page was
  served with stays in force when it is re-rendered from cache (back /
  forward, `304`). `<template>` contents are inert. `img-src` is not
  enforced. See `oasis-http/src/csp.rs` for the exact directive and
  source coverage
- **Cookies** -- RFC 6265 host-only / `Domain` / `Secure` / expiry rules
  with a public-suffix guard and size caps; cookies are attached and
  stored per redirect hop so they never follow a cross-site redirect;
  plain http can neither overwrite nor delete a `Secure` cookie.
  `SameSite=Strict` / `Lax` are withheld from cross-site subresources
  and form posts (missing `SameSite` behaves as `None`). Session-only
  (never persisted). `document.cookie` is a capped per-page map
- **Local files** -- pages with a web origin cannot read `vfs://`
  subresources, and a network response can't redirect to `vfs://`,
  `file:` or any other non-http(s) scheme
- **Request hygiene** -- the request target is percent-encoded and
  methods / header names / values are validated before anything is
  sent, so CR / LF from a URL or a `fetch()` header can't split a
  request
- **Redirects** -- validators are sent on the first hop only;
  `Cookie` / `Authorization` are dropped when a hop leaves the origin and
  `Referer` follows `strict-origin-when-cross-origin`. URLs are
  normalized (lower-cased host, default port elided, userinfo split off
  and never sent)
- **HTTP/2** -- frame size, header-block and HPACK decoded-size caps
  bound memory against CONTINUATION floods and HPACK amplification;
  HPACK lengths and indices are overflow-checked
- **Body size** -- decompressed HTTP bodies are capped at 8 MB on
  desktop (2 MB on PSP and for Gemini)

The browser feature and limit catalogue is
[browser-engine.md](browser-engine.md).

## Network Security

`oasis-net` provides TCP networking with pre-shared key (PSK)
authentication for remote terminal connections:

- PSK authentication **requires TLS** (`tls-rustls` feature). Without
  TLS the client refuses to transmit the key and the listener rejects
  connections that attempt PSK auth, preventing plaintext key leakage
- The listener binds loopback by default and refuses to bind all
  interfaces without a non-empty PSK, so the terminal `listen` command
  (which cannot supply one) is loopback-only
- The key is compared in constant time (`oasis_types::util::ct_eq`);
  five consecutive failures from one peer start an exponential back-off
- Authentication has a 30-second timeout to prevent indefinite hangs
  from malicious or unresponsive servers
- **Line length cap** -- the listener drops a connection whose line
  exceeds 1024 bytes and the client one that exceeds 16 KiB, so a peer
  cannot exhaust memory by sending data without newlines
- The FTP-like transfer server (`oasis-core/src/transfer/`) is opt-in
  (`ftp start [port] [--password <pass>]`); after three failed logins the
  connection is dropped. The limit is per connection, not per peer; see
  [networking.md](networking.md#file-transfer-oasis-coresrctransfer) for
  its caveats
- The optional MCP control server (`oasis-mcp`, off by default) listens
  on loopback only, checks `Origin` / `Host` and can require a bearer
  token; see [mcp-server.md](mcp-server.md)
- No outbound connections are made without user initiation

## MP4 Demux Safety

The lightweight MP4 parser (`oasis-video/src/demux_lite/`, feature `demux-lite`) processes
untrusted media files from the Internet Archive. Hardening measures:

- **Table size cap** -- all 7 sample table parsers (`stts`, `ctts`,
  `stsc`, `stsz`, `stco`, `co64`, `stss`) enforce a
  `MAX_TABLE_ENTRIES` (10M) limit. Malformed files with inflated
  counts are rejected before allocation, preventing OOM
- **Checked arithmetic** -- the `sample_to_chunk()` accumulator uses
  `checked_add` to prevent integer overflow from corrupt chunk tables
- **StreamingBuffer gap handling** -- reads from evicted buffer
  regions return `io::Error` instead of zero-filled data, preventing
  silent decode corruption

## Thread Safety

- The VFS implementations are not internally synchronized: writes take
  `&mut self` and each host owns its VFS on the UI thread
- The FFI video decode thread communicates via `Arc<Mutex<...>>` and
  `AtomicBool` stop flags
- `unsafe impl Send/Sync` appears in the PSP crates (`PspTlsProvider`,
  `PspTlsStream`, `PspNetworkStream`, the USB client's static
  descriptors), where the hardware is single-core with cooperative
  scheduling, and once in `oasis-browser` (`ParallelStyles`, behind the
  non-default `parallel-style` feature: each thread writes a disjoint
  per-node cell). All other thread safety derives from standard library
  primitives

## PSP TLS Host Pinning

The PSP backend uses `embedded-tls` with `UnsecureProvider` (no
certificate store available on PSP hardware -- skips cert validation
entirely). To mitigate the lack of certificate validation,
`PspTlsProvider` supports a host pinning allowlist. When pinned hosts
are configured, `connect_tls` rejects any server name not in the list,
preventing accidental connections to untrusted servers.

The `embedded-tls` crate requires the `alloc` feature to advertise RSA
signature schemes during the TLS 1.3 handshake. Without it, servers
using RSA certificates (e.g. archive.org) reject the connection with
HandshakeFailure.

RNG entropy for TLS key exchange uses `sceKernelGetSystemTimeLow()`
(a microsecond timer). The COP0 Count register (`mfc0 $9`) is
privileged on PSP Allegrex and crashes in user mode, so it cannot be
used as an entropy source.

## Audit Logging

The terminal provides an `audit` command that reads/clears
`/var/log/audit.log` in the VFS. Applications can append entries
to track security-relevant operations.
