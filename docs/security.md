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

`VfsPermissions` tracks Unix-style mode bits (owner read/write/execute).
`MemoryVfs` enforces these on every `read()`, `write()`, and
`readdir()` call. Files default to `0o644` (owner rw, others r) and
directories to `0o755`.

### Layer 3: Jail Roots

`GameAssetVfs` (UE5) restricts all operations to a set of declared
base directories. Writes create overlay entries that never touch the
underlying asset files.

`RealVfs` (desktop) operates on the real filesystem but is only used
in development builds.

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

- **No script execution by default** -- JavaScript support is
  feature-gated (`javascript`) and uses QuickJS-NG in a sandboxed
  runtime with no host filesystem or network access
- **No `<iframe>` nesting** -- prevents frame injection attacks
- **CSS property limits** -- unknown properties are ignored; no
  `expression()` or `-moz-binding` support
- **Image loading** -- images are decoded in-process with bounds
  checking on dimensions

## Network Security

`oasis-net` provides TCP networking with pre-shared key (PSK)
authentication for remote terminal connections:

- PSK authentication **requires TLS** (`tls-rustls` feature). Without
  TLS the client refuses to transmit the key and the listener rejects
  connections that attempt PSK auth, preventing plaintext key leakage
- Connections require a shared secret before command execution
- Authentication has a 30-second timeout to prevent indefinite hangs
  from malicious or unresponsive servers
- FTP transfers are opt-in and require explicit user action
- FTP server supports optional password authentication (`ftp on [port] [password]`);
  after three failed login attempts the connection is dropped
- No outbound connections are made without user initiation

## Thread Safety

- `MemoryVfs` uses interior mutability via `RefCell` (single-threaded)
- `GameAssetVfs` uses `RwLock` for thread-safe overlay writes
- The FFI video decode thread communicates via `Arc<Mutex<...>>` and
  `AtomicBool` stop flags
- `unsafe impl Send/Sync` is used only for the PSP backend
  (`PspTlsProvider`, `PspTlsStream`) where the hardware is
  single-core with cooperative scheduling; all other thread safety
  derives from standard library primitives

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
