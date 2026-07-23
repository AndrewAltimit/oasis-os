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

## Architecture

The protocol/transport layer lives in the standalone [`oasis-mcp`](../crates/oasis-mcp)
crate (HTTP framing + JSON-RPC/MCP dispatch), which depends only on
`oasis-types` and is fully unit-tested with mock sockets. The app-specific tool
implementations live in `crates/oasis-app/src/mcp_tools.rs` (`AppDispatcher`),
driven once per frame by `commands::poll_mcp_server`. See the plan and code
comments for the borrow-split details.
