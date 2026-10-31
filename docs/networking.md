# Networking

OASIS_OS networking is split across two crates:

- **`oasis-net`** — TCP transport, PSK challenge-response authentication
  (`auth.rs`), optional rustls TLS for outbound clients, the
  remote terminal protocol, the `StdNetworkBackend` desktop backend, and
  (feature `vfs`) the FTP-like file transfer server (`oasis_net::ftp`),
  layered on `NetworkBackend` and `Vfs`.
- **`oasis-core/src/transfer/`** — the `ftp`, `push` and `pull` terminal
  commands; re-exports `FtpServer` and `DEFAULT_FTP_PORT`.

The `NetworkBackend` trait itself lives in `oasis-types/src/backend/`. Backends
(SDL3, WASM, UE5, PSP) implement it; everything above the trait is shared.

## TCP transport

`StdNetworkBackend` (`oasis-net/src/std_backend.rs`) is the desktop / Pi
implementation. It uses `std::net` with poll-driven, non-blocking sockets —
except during the initial TLS handshake (see Threading model). There is no
async runtime, no thread pool, and no internal task spawning. Listener and
client are designed to be polled from the host's main loop.

Sockets are switched to non-blocking mode on accept and after connect.
`read` and `write` return immediately; when the socket is not ready they
fail with `OasisError::Io` of kind `WouldBlock` (the raw OS error, whose text
is e.g. "Resource temporarily unavailable"). Callers must test
`OasisError::is_would_block()` (`WouldBlock` only), never search
the error's display text for "would block": only the TLS stream happens to
use that wording, so text matching works on `https://` and silently breaks
every plain-TCP stream. `TimedOut` is *not* "try again": on a non-blocking
socket it is the kernel declaring the peer dead (ETIMEDOUT after retransmits
or keepalive gave up), so the listener, FTP and MCP servers drop the
connection, and `RustlsStream` passes it through with its kind intact
(`OasisError::is_timed_out()`). Every server also has an idle timeout, since
a peer that vanishes without a FIN or RST yields `WouldBlock` forever.
`connect` resolves the host with
`ToSocketAddrs` (hostnames, IPv4 literals and IPv6 literals with or without
brackets all work) and tries each resolved address with
`TcpStream::connect_timeout`, so the TCP handshake blocks the caller for at
most `DEFAULT_CONNECT_TIMEOUT` (10 s) per address (adjustable with
`StdNetworkBackend::set_connect_timeout`); the stream is switched to
non-blocking mode once connected. TLS handshakes (outbound HTTPS / Gemini)
also block the calling thread: they spin with a 1 ms sleep between
iterations (`tls_rustls.rs`) until the handshake completes or the 30 s
deadline expires.

`NetworkBackend::listen(port)` binds `0.0.0.0` (all interfaces).
`StdNetworkBackend` additionally offers `listen_loopback(port)` (binds
`127.0.0.1`) and `listen_on(ip, port)`; servers that do not authenticate
their peers use the loopback variant.

The `tls-rustls` Cargo feature provides `RustlsTlsProvider` for **outbound**
client connections that ask for TLS (`https://`, `gemini://`; see TLS
below). The remote terminal and the transfer server do not use TLS: their
TCP sessions are plaintext. The remote terminal never sends its PSK (it uses
a challenge-response, below), but commands and output are readable by
anyone on the path.

## Remote terminal protocol

The remote terminal lets one OASIS instance execute shell commands on another
over TCP. Two halves:

- `RemoteListener` (`oasis-net/src/listener.rs`) — server side.
- `RemoteClient` (`oasis-net/src/client.rs`) — client side.

Wire format is line-based text, `\n`-delimited. The server caps incoming lines
at 1024 bytes (`listener.rs`); the client buffer caps at 16 KiB
(`client.rs`). Excess line length disconnects with an error.

### Server flow

1. `start(backend)` binds according to `ListenerConfig::bind`:
   - `ListenerBind::Loopback` (the default) binds `127.0.0.1` only.
   - `ListenerBind::AllInterfaces` binds `0.0.0.0` and **requires a
     non-empty PSK**; `start` returns a config error otherwise. An empty PSK
     means "no authentication", so it is never exposed to the network.

   The terminal `listen <port>` command cannot supply a PSK and therefore
   always listens on loopback. Up to 4 concurrent connections by default
   (`listener.rs`, configurable via `ListenerConfig`).
2. On accept, send the welcome line:
   - `OASIS_OS remote terminal\n> ` if no PSK is configured.
   - `AUTH_REQUIRED <nonce>\n` if a PSK is configured (see PSK
     authentication). A connection that has not authenticated within 20 s
     gets `AUTH_FAIL timeout\n` and is closed, so idle unauthenticated
     sockets cannot hold every slot.
3. `poll()` reads in 4096-byte chunks (at most 64 KiB per poll), splits on
   `\n`, and returns complete lines as `Vec<(String, usize)>` where the
   second element is the connection id. At most 64 lines are taken from one
   connection per poll; the rest stay buffered (and nothing more is read
   from that peer) until later polls, so a burst cannot hand the host
   thousands of commands in one frame. `quit` and `exit` are intercepted and
   close the connection; nothing after them runs.
4. The host runs each command and replies with
   `send_response(conn_id, text)`. The desktop shell runs it through
   `commands::run_remote_command` (the path MCP `run_command` uses too): the
   shell's own command registry and working directory, with the server
   toggles (`listen`, `remote`, `ftp`, `mcp-server`) answered
   `Not available via remote.`. `send_response` queues the text plus a trailing
   `\n> ` prompt and writes it as the socket accepts it (a peer that lets
   more than 8 MiB back up is dropped).
5. Connections idle beyond 300 s (`listener.rs`) are closed with
   `Idle timeout. Goodbye.\n`.

### Client flow

1. `connect(addr, psk)` opens the TCP connection and, if a PSK was supplied,
   enters the authenticating state (`client.rs`).
2. `send(line)` writes `<line>\n` to the stream, retrying short writes and
   `WouldBlock` (up to 5 s) so a command is never cut.
3. `poll()` returns any complete lines received since the last call. While
   authenticating it answers `AUTH_REQUIRED <nonce>` and consumes
   `AUTH_OK` / `AUTH_FAIL`, updating `ClientState`. The auth handshake has a
   30 s deadline (`client.rs`) -- if neither response arrives, `poll()`
   returns `Authentication timed out.` and closes the connection.

## PSK authentication

PSK is the only built-in identity mechanism. It is configured per-listener via
`ListenerConfig::psk` and per-client via the optional `psk` argument to
`RemoteClient::connect`. The PSK itself never crosses the wire
(`oasis-net/src/auth.rs`):

1. Server sends `AUTH_REQUIRED <nonce>\n` immediately after accept. The
   nonce is 128 fresh random bits as 32 lowercase hex digits, new for every
   connection.
2. Client answers `AUTH <hex(HMAC-SHA256(key = psk, message = nonce))>\n`
   (`oasis_net::psk_response`, on the dependency-free SHA-256 / HMAC in
   `oasis_types::crypto`). By hand:
   `printf %s "$NONCE" | openssl dgst -sha256 -hmac "$PSK"`.
3. Server compares with `oasis_types::util::ct_eq`, a constant-time XOR
   loop.
4. On match, server replies `AUTH_OK\n> ` and the client transitions to
   `Connected`. On mismatch (including sending the bare PSK, which older clients did),
   server replies `AUTH_FAIL\n`, discards anything else the peer sent, and
   closes: one guess per connection.

A passive observer learns neither the PSK nor a reusable answer, so captured
traffic cannot be replayed. The session is **not** encrypted or integrity
protected after authentication: an active man-in-the-middle can read or
inject commands. Use the listener on trusted networks, or tunnel it (SSH).

Brute-force defence is a server-wide throttle (`AuthThrottle` in
`auth.rs`): failures are counted until a quiet 30 s passes without one; the
5th locks authentication out for 30 s from that failure, and every further
failure doubles the lockout (capped at 64x). While locked, new connections
get `RATE_LIMITED\n` and are closed. The throttle is global, not per peer
(`NetworkStream` exposes no peer address), so a flood of bad attempts also
delays legitimate clients -- the trade-off for bounding online guessing.

### PSP dev servers

The two PSP development servers on TCP 9293 speak the same handshake
(steps 1-4, with `AUTH_OK\n` and no prompt), then the one-command-per-
connection protocol they always had. Both are off unless a key is
configured, and neither has an unauthenticated mode:

| Server | Enabled by | Key | Host client |
|--------|-----------|-----|-------------|
| EBOOT command server (`oasis-backend-psp/src/cmd_server/`) | `ms0:/PSP/GAME/OASISOS/cmdserver.key` exists with 8+ bytes | that file, trimmed | `scripts/psp_remote.py` (`PSP_PSK`) |
| Overlay PRX devloop (`oasis-plugin-psp/src/devloop.rs`) | `devloop = true` **and** `devloop_psk` (8+ bytes) in `ms0:/seplugins/oasis.ini` | `devloop_psk` | `psp_remote.PspConnection(psk=...).send_command("launch ms0:/...")` |

The EBOOT verifies with `oasis_net::verify_auth_line`; the PRX, which
cannot link std crates, carries a bounded one-shot SHA-256 / HMAC copy
(`oasis-plugin-psp/src/auth.rs`). Nonces condense the RTC / system timer,
a counter and the previous nonce with SHA-256 (the PSP has no OS entropy
source). A failed answer costs the peer a 2 s stall of the single-threaded
accept loop, and an unanswered challenge times out after 10 s. As with the
remote terminal the session is plaintext: trusted networks only.

## TLS

Enabled by the `tls-rustls` Cargo feature. `RustlsTlsProvider`
(`tls_rustls.rs`) builds a process-wide singleton `ClientConfig` cached
behind `LazyLock`:

- Trust roots: the Mozilla bundle from `webpki-roots`.
- Verifier: standard rustls; SNI is enforced and mismatched server names are
  rejected (`tls_rustls.rs`).
- ALPN advertised: `h2`, `http/1.1`.
- Handshake timeout: 30 s (`tls_rustls.rs`).

Plaintext bytes between reads are buffered in a `VecDeque<u8>`
(`tls_rustls.rs`); `drain_deque` uses `as_slices` to copy out without an
intermediate allocation (`tls_rustls.rs`).

Writes: `write` returns the plaintext bytes rustls accepted even when the
non-blocking socket refuses (part of) the resulting ciphertext; what the
socket cannot take stays queued in rustls and goes out on the next `read`,
`write` or `flush` (the stream implements `NetworkStream::flush`). A write
only fails with `WouldBlock` when rustls's own buffer is full and nothing
could be accepted, so a caller that retries never sends bytes twice.

Pinning custom certificates is not implemented. TLS is used by outbound
clients only (HTTP, Gemini); no server in this crate accepts TLS.

## Hosts file

`hosts.rs` parses a TOML peer-discovery file. Schema:

```toml
[[host]]
name = "briefcase"
address = "192.168.0.50"
port = 9000              # optional, default 9000
protocol = "oasis-terminal"  # optional, default "oasis-terminal"
psk = "secret"           # optional
```

Names are required; missing optional fields use serde defaults. The parser
does **not** resolve DNS — the address is passed to the backend as-is, so
either a literal IP or a name resolvable by the host OS works. The `protocol`
field is informational today; it allows the same hosts file to describe
remote terminal endpoints, FTP servers, and future protocols side by side.

> **Security note:** `psk` values are stored as plaintext in the hosts TOML.
> Do **not** commit this file to a public repository, and treat it like any
> other credential material — keep it out of version control, restrict
> filesystem permissions, and rotate the key if the file is exposed.

## File transfer (`oasis-net/src/ftp.rs`)

The transfer service is FTP-like in spirit but defines its own minimal
line-based protocol over TCP. Default port is 2121
(`oasis-net/src/ftp.rs`).

Requests are `\n`-terminated lines of at most 1024 bytes; paths cannot
contain spaces. Verbs:

| Request | Response |
| --- | --- |
| `LIST [path]` | `200 ` followed by one `<kind> <size> <name>` line per entry (`d` = directory, `f` = file), `200 (empty)`, or `500 <error>`. |
| `GET <path>` | `200 <n> bytes\n` followed by exactly `n` raw bytes (binary-safe), or `500 <error>`. |
| `PUT <path> <text>` | Writes the rest of the line (trimmed) as the file content: `200 written <n> bytes to <path>`, or `500 <error>`. One-line text only. |
| `MKDIR <path>` | `200 created <path>`, or `500 <error>`. |
| `DELETE <path>` | `200 deleted <path>`, or `500 <error>`. |
| `RENAME <from> <to>` | `200 renamed <from> -> <to>`, or `500 <error>`. |
| `STAT <path>` | `200 <directory|file> <bytes> bytes`, or `500 <error>`. |
| `PASS <password>` | `230 Authenticated`, `530 Authentication failed`, or `530 Too many failures` (after 3 attempts the connection closes). Only meaningful when the server has a password. |
| `QUIT` | `200 goodbye` and closes the connection. Always allowed, even before authentication. |

> **Path resolution and exposure:** the server exposes one directory of
> the VFS it polls, its *root* (`FtpServer::with_root`, default `/` = the
> whole VFS). Clients see that directory as `/`: `poll` wraps the VFS in an
> `oasis_vfs::SubtreeVfs`, which normalizes every path
> (`oasis_vfs::normalize_path`: `..` that climbs above `/` and NUL bytes
> are errors) before prefixing the root, so nothing outside it can be
> named, and refuses to remove or rename the root itself. The served `Vfs`
> then applies its own checks (`RealVfs` additionally confines symlinks and
> rejects Windows drive / device segments). The desktop shell serves
> `/home` (`FTP_ROOT` in `oasis-app/src/commands/services.rs`), so FTP clients
> cannot reach the shell's `/system` state or the `/var` IPC request
> files. Error messages name the inner path (`no such file:
> /home/x`).

> **`PUT` is inline text.** The whole request -- `PUT `, the path, a space
> and the content -- must fit on one line, so the payload is at most
> `1024 - 5 - len(path)` bytes, cannot contain a newline and is
> whitespace-trimmed. Longer lines get `500 line too long` and the
> connection closes. There is no multi-line or binary upload mode.

> **`PASS` security posture.** The password travels in plaintext, and with
> a password the server listens on all interfaces. The comparison is
> constant-time (`ct_eq`); lines after the third failure on a connection are
> discarded; failures also feed a server-wide throttle (the same
> `AuthThrottle` as the remote terminal: 5 failures lock logins for 30 s,
> doubling), during which new connections get `421 Too many failed logins`
> and are closed. A connection that has not authenticated within 20 s gets
> `530 Authentication timeout`. An empty password (`--password ""`) is the
> same as none: no `PASS` is required and the server binds loopback only.
> Treat the service as trusted-LAN-only and never reuse a remote-terminal
> PSK as the FTP password.

Each poll processes at most 16 lines per connection; input is only read
while fewer than 1024 bytes are buffered and less than 1 MiB of output is
queued, so a peer that floods (even with empty lines) or stops reading
cannot grow the server's buffers without bound.

The service is integrated into the desktop binary via `FtpServer` in
`oasis-app/src/app_state.rs`. The `ftp` terminal command starts and stops it,
and the main loop polls connections each frame
(`Shell::step` in `oasis-app/src/shell.rs`). `ftp status` reads the network
state the shell publishes every frame at `/system/state/net`
(`oasis_types::ipc::NetState`, the document the Network app shows), so it
reports `listening on port N` while the server runs. (The old
`/var/ftp/status` / `/var/ftp/request` paths, which no host serviced, are
gone.)
`ftp start` rejects port `0` (the stop sentinel) and unparseable ports.

Authentication is optional and password-based: `ftp start <port> --password
<pass>` arms `FtpServer::with_password` (`oasis-net/src/ftp.rs`). Without
`--password` (or with an empty one) every accepted connection is
immediately authenticated, so the server then binds **loopback only**
(`NetworkBackend::listen_loopback`); with a password it listens on all
interfaces. The
protocol does **not** speak TLS, so the password and all subsequent traffic
travel in plaintext.

## Threading model

- All I/O is non-blocking — `poll()` everywhere returns immediately.
- No internal thread spawn, no `tokio`, no `async`/`await`.
- The TCP connect (10 s per resolved address), the outbound TLS handshake
  and `RemoteClient::send` (retrying a full socket buffer for up to 5 s) are
  the only places that block beyond a single syscall, and all of them bound
  the wait with a deadline (TLS: 30 s).

The expected integration is a single-threaded host loop: each frame, call
`listener.poll()`, dispatch returned lines, call `client.poll()` for any open
clients, and call `commands::poll_ftp_server(...)` if the transfer service is
active.

## Failure modes worth knowing about

| Symptom | Likely cause | Source |
| --- | --- | --- |
| `AUTH_FAIL` right after connecting | Wrong PSK, or a client that sends the key itself instead of answering the `AUTH_REQUIRED <nonce>` challenge. | listener.rs |
| `AUTH_FAIL timeout` | No `AUTH` answer within 20 s of connecting. | listener.rs |
| `Authentication timed out.` | Server never responded `AUTH_OK`/`AUTH_FAIL` within 30 s. | client.rs |
| `error: line too long` | Peer sent more than 1024 bytes (server) or 16 KiB (client) in one line. | listener.rs |
| `RATE_LIMITED` on connect | 5+ failed PSK attempts server-wide; lockout active. | auth.rs |
| `TLS handshake timed out` / `peer closed` | Peer is not actually a TLS server, or `tls-rustls` mismatch. | tls_rustls.rs |

## Adding a new protocol

The `protocol` field in the hosts file already anticipates this. The pattern
the existing code uses:

1. Implement client and server halves on top of `NetworkBackend` (mirror the
   `RemoteClient` / `RemoteListener` shape).
2. Use the same non-blocking-poll discipline: no thread spawns, no async.
3. If authenticated, reuse `oasis_net::auth` (challenge-response with
   `psk_response`, `AuthThrottle`, the 20 s unauthenticated deadline) and
   compare secrets with `oasis_types::util::ct_eq`.
4. Register a status / request VFS file pair under `/var/<proto>/` so
   headless tests and the terminal can drive it without a TUI.
