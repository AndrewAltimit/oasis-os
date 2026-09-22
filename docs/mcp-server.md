# MCP Control Server

OASIS_OS can optionally host a [Model Context Protocol](https://modelcontextprotocol.io)
server so a **local agent** (e.g. a Claude Code instance running on the same
device) can drive the shell as a virtual assistant: open apps, move and resize
windows, run terminal commands, load pages and media, and *see* the screen.

This is the software side of the cyberdeck vision — OASIS_OS is the interface,
and the on-device agent reaches into it through these tools instead of being a
detached CLI.

The feature is **off by default** in two independent ways: it is compiled out
unless you build with `--features mcp`, and even then it does nothing until you
opt in at runtime. A default build carries no MCP code, no extra dependencies,
and no open ports.

## Transport

OASIS is the long-running process that owns the UI and the framebuffer, so it
*hosts* the server and the agent connects to it. The transport is
**Streamable HTTP** on loopback (`127.0.0.1`), the standard MCP HTTP transport.

The server implements the minimal conformant subset for a single local client:
`POST /mcp` with a JSON-RPC request returns a single `application/json`
response; notifications return `202`; `GET /mcp` returns `405` (there is no
server-initiated SSE stream); no session IDs are issued. It is driven by
per-frame polling on the main thread — no async runtime, no background threads —
mirroring the existing remote-terminal and FTP servers.

## Building

```bash
cargo build --release -p oasis-app --features mcp
# combine with the usual defaults if you want them:
cargo build --release -p oasis-app --features mcp,javascript,video-decode-ffmpeg
```

## Enabling at runtime

Either start it from the environment at boot:

```bash
OASIS_MCP=1 ./target/release/oasis-app
# optional overrides:
OASIS_MCP=1 OASIS_MCP_PORT=7345 OASIS_MCP_TOKEN=my-secret ./target/release/oasis-app
```

…or toggle it from the OASIS terminal at any time:

```
mcp-server start            # loopback :7345
mcp-server start 8000       # custom port
mcp-server start 7345 --token my-secret
mcp-server stop
```

| Variable          | Default | Meaning                                          |
| ----------------- | ------- | ------------------------------------------------ |
| `OASIS_MCP`       | unset   | `1` starts the server at boot                    |
| `OASIS_MCP_PORT`  | `7345`  | loopback port                                    |
| `OASIS_MCP_TOKEN` | unset   | if set, every request needs `Authorization: Bearer <token>` |

## Connecting an agent (Claude Code)

Add an HTTP MCP server to `.mcp.json` (see `docs/examples/oasis.mcp.json`):

```json
{
  "mcpServers": {
    "oasis": {
      "type": "http",
      "url": "http://127.0.0.1:7345/mcp"
    }
  }
}
```

If you set `OASIS_MCP_TOKEN`, add the header:

```json
{
  "mcpServers": {
    "oasis": {
      "type": "http",
      "url": "http://127.0.0.1:7345/mcp",
      "headers": { "Authorization": "Bearer my-secret" }
    }
  }
}
```

## Tools

| Tool | Arguments | Effect |
| ---- | --------- | ------ |
| `list_apps` | — | List launchable app titles. |
| `open_app` | `title`, `file?` | Open an app (optionally pre-loading a VFS file). |
| `list_windows` | — | List open windows (id, title, pos, size, focus). |
| `focus_window` / `close_window` / `minimize_window` / `maximize_window` / `restore_window` | `id` | Window lifecycle. |
| `move_window` | `id`, `x`, `y` | Move a window to an absolute position. |
| `resize_window` | `id`, `width`, `height` | Resize a window. |
| `run_command` | `command` | Run a terminal command and return its output. |
| `browser_navigate` | `url` | Open the Browser (if needed) and navigate. |
| `play_media` | `path` | Play a media file by VFS path. |
| `tune` | `source` (`radio`/`tv`), `channel` | Tune the radio or TV. |
| `get_state` | — | Current mode, skin, focused window, open windows, browser URL. |
| `screenshot` | — | Return the current screen as a PNG image. |

When an agent acts, a small activity pill appears in the bottom-right of the UI
showing the most recent tool — the beginning of the in-OS assistant surface.

## Security

- The server binds **loopback only** (`127.0.0.1`); it is never reachable off
  the device.
- It is inert unless both compiled in (`--features mcp`) and turned on at
  runtime.
- `OASIS_MCP_TOKEN` adds a constant-time-compared bearer-token gate.
- **`run_command` runs arbitrary terminal commands.** Enabling the MCP server
  grants the connected agent the same authority as the local OASIS shell. Only
  enable it for agents you trust on that device.

### Threat model

Binding to loopback keeps other machines out, but it does **not** keep out
code running *on* the device — most importantly, any web page open in a local
browser. Without further checks a page could reach the server in two ways:

1. **Cross-site "simple" requests.** A page can `fetch("http://127.0.0.1:7345/mcp",
   {method: "POST", mode: "no-cors", body: ...})` with `Content-Type: text/plain`.
   Such requests skip the CORS preflight, so the page cannot read the response
   but the server would still execute the JSON-RPC call (e.g. `run_command`).
2. **DNS rebinding.** An attacker-controlled hostname is re-pointed at
   `127.0.0.1` after the page loads, making the requests same-origin from the
   browser's point of view (so the response becomes readable too).

Because the token is optional, the server enforces these request-level rules
on every request, before the token check and before any dispatch:

| Rule | Response |
|------|----------|
| `Origin` present and not `http(s)://` + `127.0.0.1` / `localhost` / `[::1]` (optional `:port`); `Origin: null` is rejected | `403` |
| `Host` present and not `127.0.0.1` / `localhost` / `[::1]` (optional `:port`) | `403` |
| `POST` whose `Content-Type` media type is not `application/json` (parameters such as `; charset=utf-8` are fine; missing is rejected) | `415` |
| Malformed, non-decimal or duplicate `Content-Length` | `400` |

Non-browser MCP clients (Claude Code, `curl`) send a loopback `Host`, no
`Origin` and `application/json`, so they are unaffected. Requiring
`application/json` means a cross-origin browser request always needs a CORS
preflight, which the server never approves (the `OPTIONS` response carries no
`Access-Control-Allow-*` headers). The `Host` check defeats rebinding, since
the rebound request still carries the attacker's hostname.

Resource limits (the server has only 4 connection slots, so these bound what a
misbehaving local client can hold):

- A request's header section must complete within **10 s** of the connection
  opening (or of the first byte of a follow-up request); otherwise the server
  replies `408` and closes. Idle keep-alive connections between requests fall
  under the separate 300 s idle timeout.
- Each poll reads until the socket would block, capped at **64 KiB** per
  connection, so large bodies do not trickle in at 4 KiB per frame.
- Headers are capped at 16 KiB (`431`) and bodies at 1 MiB (`413`).
- A connection closed after its final response (an error, `Connection: close`,
  HTTP/1.0) *lingers*: further input is read and discarded (never buffered)
  until the peer has been quiet for 100 ms, reaches EOF, or 2 s pass. Closing
  on top of unread input would make the OS reset the connection and could
  destroy the `413` a client streaming an oversized body is about to read.
- Once **4 MiB** of responses are queued for a peer that is not reading them,
  the server stops reading and dispatching further requests on that
  connection until the backlog drains (backpressure, not data loss).

Setting `OASIS_MCP_TOKEN` is still recommended when other local users or
processes on the device are not trusted: the checks above stop browsers, not a
local process that can open a raw socket.

## Architecture

The protocol/transport layer lives in the standalone [`oasis-mcp`](../crates/oasis-mcp)
crate (HTTP framing + JSON-RPC/MCP dispatch), which depends only on
`oasis-types` and is fully unit-tested with mock sockets. The app-specific tool
implementations live in `crates/oasis-app/src/mcp_tools.rs` (`AppDispatcher`),
driven once per frame by `commands::poll_mcp_server`. See the plan and code
comments for the borrow-split details.
