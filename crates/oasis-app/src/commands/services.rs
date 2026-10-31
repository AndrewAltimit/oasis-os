//! Network services: the remote-terminal listener, the FTP and MCP
//! servers, the outbound remote client, and their published state.
//!
//! The `listen` / `ftp` / `connect` / `mcp-server` commands only emit a
//! signal; the toggles here start and stop the servers for the local
//! terminal (remote callers and background jobs are refused before they
//! get here, see `super::refuse_job_server_toggles`). The `poll_*`
//! functions run once per `Shell::step`.

use oasis_core::net::{
    ListenerBind, ListenerConfig, RemoteClient, RemoteListener, StdNetworkBackend,
};
use oasis_core::sdi::SdiRegistry;
use oasis_core::transfer::FtpServer;
use oasis_core::vfs::{MemoryVfs, Vfs};

use crate::app_state::{AppState, NetworkLayer, TerminalLayer};

/// The VFS directory `ftp start` serves: clients see it as `/`.
pub const FTP_ROOT: &str = "/home";

/// Terminal messages of a toggled server (`listen`, `ftp`).
struct ServiceText {
    stopped: &'static str,
    not_running: &'static str,
    already_running: &'static str,
}

/// The `listen` / `ftp` toggle: `port == 0` stops the server (and frees
/// its listening socket by replacing `backend`), otherwise a running
/// server is refused and a stopped one is started with `start`, which
/// returns the server and its "listening" line (or the error line).
fn toggle_service<S>(
    out: &mut TerminalLayer,
    slot: &mut Option<S>,
    backend: &mut StdNetworkBackend,
    port: u16,
    text: &ServiceText,
    stop: fn(&mut S),
    start: impl FnOnce(&mut StdNetworkBackend) -> Result<(S, String), String>,
) {
    if port == 0 {
        match slot.take() {
            Some(mut server) => {
                stop(&mut server);
                // `stop` only drops the connections; a fresh backend closes
                // the listening socket so the port is free again.
                *backend = StdNetworkBackend::new();
                out.print(text.stopped);
            },
            None => out.print(text.not_running),
        }
    } else if slot.is_some() {
        out.print(text.already_running);
    } else {
        match start(backend) {
            Ok((server, line)) => {
                out.print(line);
                *slot = Some(server);
            },
            Err(line) => out.print(line),
        }
    }
}

/// `listen <port>` / `listen stop` (port 0).
pub(super) fn toggle_listener(term: &mut TerminalLayer, net: &mut NetworkLayer, port: u16) {
    toggle_service(
        term,
        &mut net.listener,
        &mut net.listener_backend,
        port,
        &ServiceText {
            stopped: "Remote listener stopped.",
            not_running: "No listener running.",
            already_running: "Listener already running. Use 'listen stop' first.",
        },
        RemoteListener::stop,
        |backend| {
            // No PSK can be supplied from the terminal, so the listener
            // stays on loopback: an unauthenticated shell must never be
            // reachable from the network.
            let cfg = ListenerConfig {
                port,
                psk: String::new(),
                bind: ListenerBind::Loopback,
                max_connections: 4,
                ..ListenerConfig::default()
            };
            let mut l = RemoteListener::new(cfg);
            match l.start(backend) {
                Ok(()) => Ok((l, format!("Listening on 127.0.0.1:{port} (loopback only)."))),
                Err(e) => Err(format!("Listen error: {e}")),
            }
        },
    );
}

/// `ftp start <port> [password]` / `ftp stop` (port 0).
pub(super) fn toggle_ftp(
    term: &mut TerminalLayer,
    net: &mut NetworkLayer,
    port: u16,
    password: Option<String>,
) {
    toggle_service(
        term,
        &mut net.ftp_server,
        &mut net.ftp_backend,
        port,
        &ServiceText {
            stopped: "FTP server stopped.",
            not_running: "No FTP server running.",
            already_running: "FTP server already running. Use 'ftp stop' first.",
        },
        FtpServer::stop,
        |backend| {
            // Clients get the user's files only, never the shell's /system
            // state or /var IPC request files.
            let mut server = FtpServer::new(port)
                .with_root(FTP_ROOT)
                .map_err(|e| format!("FTP server error: {e}"))?;
            if let Some(pass) = password {
                server = server.with_password(pass);
            }
            match server.start(backend) {
                Ok(()) => Ok((
                    server,
                    format!("FTP server listening on port {port} (serving {FTP_ROOT})."),
                )),
                Err(e) => Err(format!("FTP server error: {e}")),
            }
        },
    );
}

/// `connect <address> <port> [psk]`: open the outbound remote client.
pub(super) fn connect_remote(
    term: &mut TerminalLayer,
    net: &mut NetworkLayer,
    address: &str,
    port: u16,
    psk: Option<&str>,
) {
    if net.remote_client.is_some() {
        term.print("Already connected. Disconnect first.");
        return;
    }
    let mut client = RemoteClient::new();
    match client.connect(&mut net.backend, address, port, psk) {
        Ok(()) => {
            term.print(format!("Connected to {address}:{port}."));
            net.remote_client = Some(client);
        },
        Err(e) => term.print(format!("Connect error: {e}")),
    }
}

/// `mcp-server start [port] [token]` / `mcp-server stop`.
pub(super) fn toggle_mcp(state: &mut AppState, start: bool, port: u16, token: Option<String>) {
    #[cfg(feature = "mcp")]
    {
        if start {
            start_mcp_server(state, port, token);
        } else if let Some(mut server) = state.mcp.take() {
            server.stop();
            state.terminal.print("MCP server stopped.");
        } else {
            state.terminal.print("No MCP server running.");
        }
    }
    #[cfg(not(feature = "mcp"))]
    {
        let _ = (start, port, token);
        state
            .terminal
            .print("MCP support not compiled in (build with --features mcp).");
    }
}

/// Poll the remote listener for incoming commands and execute them.
pub fn poll_remote_listener(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) {
    let remote_cmds = {
        let NetworkLayer {
            ref mut listener,
            listener_backend: ref mut backend,
            ..
        } = state.net;
        let Some(l) = listener else { return };
        l.poll(backend)
    };

    for (cmd_line, conn_idx) in remote_cmds {
        log::info!("Remote command from #{conn_idx}: {cmd_line}");
        let response = super::run_remote_command(&cmd_line, state, sdi, vfs);
        if let Some(l) = state.net.listener.as_mut() {
            let _ = l.send_response(conn_idx, &response);
        }
    }
}

/// Poll the MCP control server, dispatching agent tool calls against the UI.
///
/// The server is taken out of `state` for the poll, so the dispatcher can
/// drive the whole shell state through the same functions the local input
/// pipeline uses (launch, close, skin swap). The rendering `backend` lets
/// the `screenshot` tool read the framebuffer.
#[cfg(feature = "mcp")]
pub fn poll_mcp_server(
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
    capture: &mut crate::mcp_tools::FrameCapture<'_>,
) {
    let Some(mut server) = state.mcp.take() else {
        return;
    };
    let mut disp = crate::mcp_tools::AppDispatcher {
        state,
        sdi,
        vfs,
        capture,
    };
    server.poll(&mut disp);
    // A tool cannot start another server (`mcp-server` is refused over
    // remote channels), so the slot is still empty.
    if state.mcp.is_none() {
        state.mcp = Some(server);
    }
}

/// Start the MCP server from environment variables at boot (`OASIS_MCP=1`,
/// optional `OASIS_MCP_PORT` / `OASIS_MCP_TOKEN`).
#[cfg(feature = "mcp")]
pub fn mcp_start_from_env(state: &mut AppState) {
    if std::env::var("OASIS_MCP").ok().as_deref() != Some("1") {
        return;
    }
    let port = std::env::var("OASIS_MCP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(7345);
    let token = std::env::var("OASIS_MCP_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    start_mcp_server(state, port, token);
}

/// Bind a loopback listener and install the MCP server on `state`.
#[cfg(feature = "mcp")]
fn start_mcp_server(state: &mut AppState, port: u16, token: Option<String>) {
    if state.mcp.is_some() {
        state
            .terminal
            .print("MCP server already running. Use 'mcp-server stop' first.");
        return;
    }
    let mut backend = StdNetworkBackend::new();
    match backend.listen_loopback(port) {
        Ok(()) => {
            let server = oasis_mcp::McpServer::new(Box::new(backend), token);
            log::info!("MCP control server listening on 127.0.0.1:{port}");
            state
                .terminal
                .print(format!("MCP server listening on 127.0.0.1:{port}."));
            state.mcp = Some(server);
        },
        Err(e) => {
            log::warn!("MCP server failed to start: {e}");
            state.terminal.print(format!("MCP start error: {e}"));
        },
    }
}

/// Poll the FTP server for incoming connections and commands.
pub fn poll_ftp_server(state: &mut AppState, vfs: &mut MemoryVfs) {
    let NetworkLayer {
        ref mut ftp_server,
        ftp_backend: ref mut backend,
        ..
    } = state.net;

    let Some(server) = ftp_server else { return };

    if let Err(e) = server.poll(backend, vfs) {
        log::warn!("FTP server poll error: {e}");
    }
}

/// Publish the network service state (remote-terminal listener, FTP
/// server, outbound remote connection, MCP server) at
/// [`NET_STATE_PATH`](oasis_core::ipc::NET_STATE_PATH) for the Network
/// app. Runs every frame; writes only when the state changed.
pub fn publish_net_state(state: &AppState, vfs: &mut MemoryVfs) {
    use oasis_core::ipc::{NET_STATE_PATH, NetState};
    let net = &state.net;
    let st = NetState {
        listener_port: net
            .listener
            .as_ref()
            .filter(|l| l.is_listening())
            .map(RemoteListener::port),
        ftp_port: net
            .ftp_server
            .as_ref()
            .filter(|s| s.is_listening())
            .map(FtpServer::port),
        remote_connected: net
            .remote_client
            .as_ref()
            .is_some_and(RemoteClient::is_connected),
        #[cfg(feature = "mcp")]
        mcp_running: state.mcp.is_some(),
        #[cfg(not(feature = "mcp"))]
        mcp_running: false,
    };
    if !vfs.exists("/system/state") {
        let _ = vfs.mkdir("/system/state");
    }
    super::publish_if_changed(vfs, NET_STATE_PATH, &st.to_text());
}

/// Poll the remote client for received data.
pub fn poll_remote_client(state: &mut AppState) {
    let Some(ref mut client) = state.net.remote_client else {
        return;
    };
    let lines = client.poll();
    for line in lines {
        state.terminal.print(format!("[remote] {line}"));
    }
    if !client.is_connected() {
        state.terminal.print("[remote] Disconnected.");
        state.net.remote_client = None;
    }
    super::trim_output(&mut state.terminal.output_lines);
}
