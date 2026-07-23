//! JSON-RPC 2.0 + MCP method dispatch.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::ToolDispatcher;

/// MCP protocol version advertised by this server.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Server name reported in `initialize`.
pub const SERVER_NAME: &str = "oasis-mcp";
/// Server version reported in `initialize`.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct CallToolParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// Outcome of handling one JSON-RPC message.
pub enum Handled {
    /// A serialized JSON-RPC response body to send with `200 OK`.
    Response(Vec<u8>),
    /// The message was a notification; send `202 Accepted` with no body.
    Notification,
}

fn result_response(id: Value, result: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
        .unwrap_or_else(|_| b"{\"jsonrpc\":\"2.0\",\"id\":null,\"result\":{}}".to_vec())
}

fn error_response(id: Value, code: i64, message: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    )
    .unwrap_or_else(|_| {
        b"{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"internal\"}}"
            .to_vec()
    })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    })
}

/// Parse and dispatch a single JSON-RPC message body against `disp`.
pub fn handle_message(body: &[u8], disp: &mut dyn ToolDispatcher) -> Handled {
    let req: JsonRpcRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => {
            return Handled::Response(error_response(
                Value::Null,
                -32700,
                &format!("parse error: {e}"),
            ));
        },
    };

    let is_notification = req.id.is_none();
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        "initialize" => Handled::Response(result_response(id, initialize_result())),
        "notifications/initialized" => Handled::Notification,
        "ping" => Handled::Response(result_response(id, json!({}))),
        "tools/list" => {
            let tools = disp.list_tools();
            Handled::Response(result_response(id, json!({ "tools": tools })))
        },
        "tools/call" => {
            let params: CallToolParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Handled::Response(error_response(
                        id,
                        -32602,
                        &format!("invalid params: {e}"),
                    ));
                },
            };
            let result = disp.call_tool(&params.name, params.arguments);
            Handled::Response(result_response(
                id,
                json!({ "content": result.content, "isError": result.is_error }),
            ))
        },
        other => {
            if is_notification {
                // Unknown notification: ignore per JSON-RPC.
                Handled::Notification
            } else {
                Handled::Response(error_response(
                    id,
                    -32601,
                    &format!("method not found: {other}"),
                ))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolResult, ToolSpec};

    struct StubDispatcher;
    impl ToolDispatcher for StubDispatcher {
        fn list_tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec::new("echo", "echoes", json!({ "type": "object" }))]
        }
        fn call_tool(&mut self, name: &str, args: Value) -> ToolResult {
            if name == "echo" {
                ToolResult::text(args.to_string())
            } else {
                ToolResult::error(format!("unknown tool: {name}"))
            }
        }
    }

    fn resp_json(h: Handled) -> Value {
        match h {
            Handled::Response(b) => serde_json::from_slice(&b).expect("valid json"),
            Handled::Notification => panic!("expected response"),
        }
    }

    #[test]
    fn initialize_returns_server_info() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let v = resp_json(handle_message(body, &mut StubDispatcher));
        assert_eq!(v["result"]["serverInfo"]["name"], "oasis-mcp");
        assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn initialized_is_notification() {
        let body = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(matches!(
            handle_message(body, &mut StubDispatcher),
            Handled::Notification
        ));
    }

    #[test]
    fn tools_list_returns_catalog() {
        let body = br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let v = resp_json(handle_message(body, &mut StubDispatcher));
        assert_eq!(v["result"]["tools"][0]["name"], "echo");
    }

    #[test]
    fn tools_call_happy_path() {
        let body = br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"a":1}}}"#;
        let v = resp_json(handle_message(body, &mut StubDispatcher));
        assert_eq!(v["result"]["isError"], false);
        assert_eq!(v["result"]["content"][0]["type"], "text");
    }

    #[test]
    fn tools_call_unknown_is_in_band_error() {
        let body = br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#;
        let v = resp_json(handle_message(body, &mut StubDispatcher));
        // Protocol success, tool-level error.
        assert!(v["error"].is_null());
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn unknown_method_is_protocol_error() {
        let body = br#"{"jsonrpc":"2.0","id":5,"method":"frobnicate"}"#;
        let v = resp_json(handle_message(body, &mut StubDispatcher));
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn parse_error_returns_32700() {
        let v = resp_json(handle_message(b"not json", &mut StubDispatcher));
        assert_eq!(v["error"]["code"], -32700);
    }
}
