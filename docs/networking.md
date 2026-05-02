# Networking

OASIS_OS networking is split across two crates:

- **`oasis-net`** — TCP transport, PSK authentication, optional rustls TLS, the
  remote terminal protocol, and the `StdNetworkBackend` desktop backend.
- **`oasis-core/src/transfer/`** — the FTP-like file transfer service, layered
  on top of `NetworkBackend` from `oasis-types`.

The `NetworkBackend` trait itself lives in `oasis-types/src/backend/`. Backends
(SDL3, WASM, UE5, PSP) implement it; everything above the trait is shared.

## TCP transport

`StdNetworkBackend` (`oasis-net/src/std_backend.rs`) is the desktop / Pi
implementation. It uses `std::net` with poll-driven, non-blocking sockets —
except during the initial TLS handshake (see Threading model). There is no
async runtime, no thread pool, and no internal task spawning. Listener and
client are designed to be polled from the host's main loop.

Sockets are switched to non-blocking mode on accept (`std_backend.rs:52`,
`67`, `82`). `connect`, `send`, `recv` all return immediately with
`WouldBlock` translated into "no data yet". TLS handshakes are the one
exception: they spin with a 1 ms sleep between iterations
(`tls_rustls.rs:186`), so `RemoteClient::connect` blocks the calling thread
until the handshake completes or the 30 s deadline expires.

When the `tls-rustls` Cargo feature is enabled the backend wraps outbound
streams with rustls and listeners accept TLS connections. Without the feature
all traffic is plaintext TCP and PSK authentication is rejected (see below).

## Remote terminal protocol

The remote terminal lets one OASIS instance execute shell commands on another
over TCP. Two halves:

- `RemoteListener` (`oasis-net/src/listener.rs`) — server side.
- `RemoteClient` (`oasis-net/src/client.rs`) — client side.

Wire format is line-based text, `\n`-delimited. The server caps incoming lines
at 1024 bytes (`listener.rs:16`); the client buffer caps at 16 KiB
(`client.rs:15`). Excess line length disconnects with an error.

### Server flow

1. `start(backend)` calls `backend.listen(port)`. Up to 4 concurrent
   connections by default (`listener.rs:13`, configurable via
   `ListenerConfig`).
2. On accept, send the welcome line:
   - `OASIS_OS remote terminal\n> ` if no PSK is configured.
   - `AUTH_REQUIRED\n` if a PSK is configured.
3. `poll()` reads in 512-byte chunks (`listener.rs:258`), splits on `\n`, and
   returns complete lines as `Vec<(String, usize)>` where the second element
   is the connection index. `quit` and `exit` are intercepted and close the
   connection immediately.
4. The host runs each command and replies with
   `send_response(conn_idx, text)`, which writes the text plus a trailing
   `\n> ` prompt.
5. Connections idle beyond 300 s (`listener.rs:27`) are closed with
   `Idle timeout. Goodbye.\n`.

### Client flow

1. `connect(addr, psk)` opens the TCP connection and, if a PSK was supplied,
   enters the authenticating state (`client.rs:57`).
2. `send(line)` writes `<line>\n` to the stream.
3. `poll()` returns any complete lines received since the last call. It also
   transparently handles `AUTH_OK` / `AUTH_FAIL` and updates `ClientState`.
   The auth handshake has a 30 s deadline (`client.rs:12`) — if neither
   response arrives, `poll()` returns `Authentication timed out.` and closes
   the connection.

## PSK authentication

PSK is the only built-in identity mechanism. It is configured per-listener via
`ListenerConfig::psk` and per-client via the optional `psk` argument to
`RemoteClient::connect`.

Handshake:

1. Server sends `AUTH_REQUIRED\n` immediately after accept.
2. Client sends `<key>\n` (`client.rs:93`).
3. Server compares with a constant-time XOR loop (`listener.rs:32`) so the
   timing of the comparison reveals neither the length nor the content of the
   stored key.
4. On match, server replies `AUTH_OK\n> ` and the client transitions to
   `Connected`. On mismatch, server replies `AUTH_FAIL\n` and closes.

Brute-force defence is rate limiting: 5 consecutive failures from the same
peer trigger an exponentially increasing back-off starting at 30 s, with the
listener responding `RATE_LIMITED\n` to new connections from that peer
(`listener.rs:19`, `216`).

**TLS is mandatory for PSK.** Without the `tls-rustls` feature, both sides
refuse to use a PSK over plaintext (`client.rs:68`, `listener.rs:226`). This
is enforced at the API: passing a PSK to `connect` without TLS support fails
with the error `"PSK authentication requires TLS — enable the `tls-rustls`
feature"`.

## TLS

Enabled by the `tls-rustls` Cargo feature. `RustlsTlsProvider`
(`tls_rustls.rs:25`) builds a process-wide singleton `ClientConfig` cached
behind `LazyLock`:

- Trust roots: the Mozilla bundle from `webpki-roots`.
- Verifier: standard rustls; SNI is enforced and mismatched server names are
  rejected (`tls_rustls.rs:69`).
- ALPN advertised: `h2`, `http/1.1`.
- Handshake timeout: 30 s (`tls_rustls.rs:159`).

Plaintext bytes between reads are buffered in a `VecDeque<u8>`
(`tls_rustls.rs:146`); `drain_deque` uses `as_slices` to copy out without an
intermediate allocation (`tls_rustls.rs:339`).

Pinning custom certificates is not yet implemented; tracked at the TODO
comment in `client.rs:91`.

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

## File transfer (`oasis-core/src/transfer/`)

The transfer service is FTP-like in spirit but defines its own minimal
line-based protocol over TCP. Default port is 2121
(`transfer/mod.rs:23`).

Verbs:

| Request | Response |
| --- | --- |
| `LIST <path>` | `200 <kind> <size> <name>` lines, or `500 <error>`. `<kind>` is `d` for directory, `f` for file. |
| `GET <path>` | `200 <bytes> bytes\n<file body>`, or `500 <error>`. Text-only — see binary/newline note below. |
| `PUT <path> <content>` | `200 written <n> bytes to <path>`, or `500 <error>`. Inline-only — see size limit below. |
| `MKDIR <path>` | `200 created <path>`, or `500 <error>`. |
| `DELETE <path>` | `200 deleted <path>`, or `500 <error>`. |
| `RENAME <from> <to>` | `200 renamed <from> -> <to>`, or `500 <error>`. |
| `STAT <path>` | `200 <directory|file> <bytes> bytes`, or `500 <error>`. |
| `PASS <password>` | `230 Authenticated`, `530 Authentication failed`, or `530 Too many failures` (after 3 attempts, the connection closes). Only meaningful when the server was started with `--password`. |
| `QUIT` | `200 goodbye` and closes the connection. Always allowed even before authentication. |

> **Path resolution and exposure:** path arguments are passed straight
> through to the active `Vfs` (`process_ftp_request` in
> `transfer/mod.rs:32`) — there is no extra chroot, allow-list, or
> traversal check at the transfer layer. The exposed filesystem is
> exactly whatever the host's VFS implementation exposes (typically a
> `MemoryVfs` for headless drivers or a `RealVfs`/`GameAssetVfs` rooted
> at a host directory). Integrators are responsible for choosing a VFS
> root that does not escape the data they want to share.

> **`GET` is text-only — binary files and embedded newlines are
> unsafe.** The server formats responses as
> `format!("200 {} bytes\n{text}", data.len())` where `text` is
> `String::from_utf8_lossy(&data)` (`transfer/mod.rs:65`). Two
> consequences:
>
> 1. **Binary data is silently corrupted.** Any byte that is not
>    valid UTF-8 gets replaced with the U+FFFD replacement character
>    (`EF BF BD`, three bytes). The `<bytes>` header still reports the
>    original `data.len()`, so the header byte count and the body
>    length will diverge for any non-UTF-8 file.
> 2. **Embedded newlines split the line-based parse on the client
>    side.** A file body containing a literal `\n` looks
>    indistinguishable from the start of a fresh response line to a
>    naive line reader. Clients that read response-by-line will only
>    see the first body line. Use the `<bytes>` header to read a fixed
>    payload after the header `\n` *only* if you also know the file is
>    valid UTF-8 — otherwise the corruption above means the read count
>    will not match anyway.
>
> For binary files or files containing newlines, tunnel through the
> TLS-protected remote terminal session and use shell redirection or
> base64 there, mirroring the `PUT` workaround below.

> **`PUT` payload size and newline handling:** the protocol is strictly
> line-based with a 1024-byte per-line cap (`MAX_FTP_LINE_LEN` in
> `transfer/mod.rs:143`). The full request — `PUT `, the path, a single
> space, and the entire file body — must fit on one `\n`-terminated
> line. Effective inline payload is therefore roughly
> `1024 - 5 - len(path)` bytes, and the body cannot contain a literal
> `\n` (it would be parsed as the end of the request and split the
> payload across the next command). Exceeding the cap clears the read
> buffer and replies `500 line too long`. There is no chunked /
> multi-line upload mode today; for files larger than ~1 KiB or files
> containing newlines, tunnel through the TLS-protected remote terminal
> session instead and use shell redirection on the remote side.

> **`PASS` security posture:** three caveats that auditors and
> integrators should be aware of, in addition to the plaintext-on-wire
> warning above.
>
> 1. **Comparison is not constant-time.** The password check uses a
>    plain `==` byte-string comparison (`transfer/mod.rs:306`), unlike
>    the remote-terminal PSK path which uses an explicit constant-time
>    XOR loop (`listener.rs:32`).
> 2. **No cross-connection brute-force protection.** The 3-attempt
>    limit (`MAX_AUTH_FAILURES` at `transfer/mod.rs:152`) is tracked
>    per-connection only (`FtpConnection::failed_attempts` at
>    `transfer/mod.rs:162`). The listener does **not** track failures
>    by remote IP across reconnections, so the effective brute-force
>    budget is `3 × unbounded reconnects`. This contrasts with the PSK
>    path, which does apply per-peer exponential back-off
>    (`listener.rs:216`). On a hostile network an attacker can drain
>    the password space at TCP-accept rate.
> 3. **Empty passwords are accepted.** `ftp start <port> --password ""`
>    is parsed verbatim (`transfer/mod.rs:399`) and arms
>    `FtpServer::with_password("")`. A client can then authenticate by
>    sending `PASS ` with no value. `--password` without any argument
>    errors with `--password requires a value`, but `--password ""`
>    silently produces an empty-credential server — pass a real
>    password, or omit the flag entirely if you intend the server to
>    accept all peers.
>
> Combined, these mean the FTP service is designed for trusted-LAN use
> only. Treat any deployment on an untrusted network as authenticated
> by IP allow-list at most, and tunnel through the TLS-protected remote
> terminal session if you need confidentiality, integrity, or proper
> brute-force resistance.

The service is integrated into the desktop binary via `FtpServer` in
`oasis-app/src/app_state.rs`. The `ftp` terminal command starts and stops it,
and the main loop polls connections each frame
(`oasis-app/src/main.rs:622`). Status and request files live at
`/var/ftp/status` and `/var/ftp/request` so headless drivers can inspect or
trigger transfers via the VFS.

Authentication is optional and password-based: `ftp start <port> --password
<pass>` arms `FtpServer::with_password` (`transfer/mod.rs:203`). Without
`--password` every accepted connection is immediately authenticated. The
protocol does **not** speak TLS, so the password and all subsequent traffic
travel in plaintext — treat the service as trusted-LAN-only and never reuse
a remote-terminal PSK as the FTP password. For authenticated, encrypted
file transfer over an untrusted network, tunnel through the TLS-protected
remote terminal session instead.

## Threading model

- All I/O is blocking on non-blocking sockets — `poll()` everywhere returns
  immediately.
- No internal thread spawn, no `tokio`, no `async`/`await`.
- The TLS handshake and the `RemoteClient::connect` path are the only places
  that block beyond a single syscall, and even those bound the wait with a
  30 s deadline.

The expected integration is a single-threaded host loop: each frame, call
`listener.poll()`, dispatch returned lines, call `client.poll()` for any open
clients, and call `commands::poll_ftp_server(...)` if the transfer service is
active.

## Failure modes worth knowing about

| Symptom | Likely cause | Source |
| --- | --- | --- |
| `PSK authentication requires TLS` | Built without `tls-rustls`. | client.rs:77 |
| `AUTH_FAIL TLS required` from server | Server compiled without TLS but a client tried PSK. | listener.rs:139 |
| `Authentication timed out.` | Server never responded `AUTH_OK`/`AUTH_FAIL` within 30 s. | client.rs:132 |
| `error: line too long` | Peer sent more than 1024 bytes (server) or 16 KiB (client) in one line. | listener.rs:305 |
| `RATE_LIMITED` on connect | 5+ failed PSK attempts; back-off active. | listener.rs:216 |
| `TLS handshake timed out` / `peer closed` | Peer is not actually a TLS server, or `tls-rustls` mismatch. | tls_rustls.rs:171 |

## Adding a new protocol

The `protocol` field in the hosts file already anticipates this. The pattern
the existing code uses:

1. Implement client and server halves on top of `NetworkBackend` (mirror the
   `RemoteClient` / `RemoteListener` shape).
2. Use the same non-blocking-poll discipline: no thread spawns, no async.
3. If authenticated, reuse the constant-time PSK comparison helper
   (`listener.rs:32`) and require TLS for the PSK path.
4. Register a status / request VFS file pair under `/var/<proto>/` so
   headless tests and the terminal can drive it without a TUI.
