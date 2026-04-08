//! MCP (Model Context Protocol) server — makes Tachy's tools available to
//! any MCP-compatible client (Claude Desktop, Cursor, Windsurf, etc.).
//!
//! Protocol: JSON-RPC 2.0 over stdio.
//!
//! Usage:
//!   tachy mcp-server
//!
//! In Claude Desktop's config:
//! ```json
//! { "mcpServers": { "tachy": { "command": "tachy", "args": ["mcp-server"] } } }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

/// Run the MCP server on stdio. Blocks until stdin closes.
pub fn run_mcp_server() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError { code: -32700, message: format!("parse error: {e}") }),
                };
                let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap_or_default());
                let _ = stdout.flush();
                continue;
            }
        };

        let response = handle_mcp_request(&request);
        let _ = writeln!(stdout, "{}", serde_json::to_string(&response).unwrap_or_default());
        let _ = stdout.flush();
    }
}

fn handle_mcp_request(req: &JsonRpcRequest) -> JsonRpcResponse {
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": false },
                },
                "serverInfo": {
                    "name": "tachy",
                    "version": "0.1.0",
                },
            })),
            error: None,
        },

        "notifications/initialized" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({})),
            error: None,
        },

        "tools/list" => {
            let tools: Vec<Value> = tools::mvp_tool_specs().iter().map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "inputSchema": spec.input_schema,
                })
            }).collect();

            // Add custom tools
            let tachy_dir = std::env::current_dir()
                .unwrap_or_default()
                .join(".tachy");
            let custom = tools::CustomToolRegistry::load(&tachy_dir);
            let custom_tools: Vec<Value> = custom.tool_specs().iter().map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "inputSchema": spec.input_schema,
                })
            }).collect();

            let all_tools: Vec<Value> = tools.into_iter().chain(custom_tools).collect();

            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(json!({ "tools": all_tools })),
                error: None,
            }
        }

        "tools/call" => {
            let tool_name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));

            // Execute the tool
            let tachy_dir = std::env::current_dir().unwrap_or_default().join(".tachy");
            let custom = tools::CustomToolRegistry::load(&tachy_dir);

            let result = if tool_name == "remember" {
                intelligence::execute_remember(&arguments, &tachy_dir)
            } else {
                tools::execute_tool_with_custom(tool_name, &arguments, &custom)
            };

            match result {
                Ok(output) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(json!({
                        "content": [{ "type": "text", "text": output }],
                    })),
                    error: None,
                },
                Err(e) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(json!({
                        "content": [{ "type": "text", "text": e }],
                        "isError": true,
                    })),
                    error: None,
                },
            }
        }

        _ => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code: -32601, message: format!("method not found: {}", req.method) }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_initialize() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: json!({}),
        };
        let resp = handle_mcp_request(&req);
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "tachy");
    }

    #[test]
    fn handles_tools_list() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(2)),
            method: "tools/list".to_string(),
            params: json!({}),
        };
        let resp = handle_mcp_request(&req);
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(tools.len() >= 7);
        assert!(tools.iter().any(|t| t["name"] == "bash"));
        assert!(tools.iter().any(|t| t["name"] == "read_file"));
        assert!(tools.iter().any(|t| t["name"] == "remember"));
    }

    #[test]
    fn handles_unknown_method() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(3)),
            method: "unknown/method".to_string(),
            params: json!({}),
        };
        let resp = handle_mcp_request(&req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }
}
