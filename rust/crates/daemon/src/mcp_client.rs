//! MCP client — allows agents to call tools on external MCP servers.
//!
//! Configured in .tachy/config.json:
//! ```json
//! {
//!   "mcp_servers": [
//!     {"name": "database", "command": "uvx", "args": ["mcp-server-sqlite", "--db", "app.db"]},
//!     {"name": "github", "command": "uvx", "args": ["mcp-server-github"]}
//!   ]
//! }
//! ```
//!
//! The agent can then call tools like "mcp__database__query" or "mcp__github__list_issues".

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Configuration for an external MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// A tool discovered from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub input_schema: Value,
}

impl McpTool {
    /// The qualified name used in the agent's tool list: mcp__<server>__<tool>
    pub fn qualified_name(&self) -> String {
        format!("mcp__{}_{}", self.server_name, self.tool_name)
    }
}

/// An active connection to an MCP server process.
pub struct McpConnection {
    pub config: McpServerConfig,
    pub tools: Vec<McpTool>,
    child: Child,
    request_id: u64,
}

impl McpConnection {
    /// Start an MCP server and discover its tools.
    pub fn connect(config: &McpServerConfig) -> Result<Self, String> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let child = cmd.spawn()
            .map_err(|e| format!("failed to start MCP server '{}': {e}", config.name))?;

        let mut conn = Self {
            config: config.clone(),
            tools: Vec::new(),
            child,
            request_id: 0,
        };

        // Initialize
        conn.send_request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "tachy-agent", "version": "0.2.0"},
        }))?;

        // Notify initialized
        conn.send_notification("notifications/initialized", json!({}))?;

        // Discover tools
        let tools_result = conn.send_request("tools/list", json!({}))?;
        if let Some(tools_array) = tools_result.get("tools").and_then(|v| v.as_array()) {
            for tool in tools_array {
                let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let desc = tool.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let schema = tool.get("inputSchema").cloned().unwrap_or(json!({}));
                conn.tools.push(McpTool {
                    server_name: config.name.clone(),
                    tool_name: name,
                    description: desc,
                    input_schema: schema,
                });
            }
        }

        Ok(conn)
    }

    /// Call a tool on this MCP server.
    pub fn call_tool(&mut self, tool_name: &str, arguments: &Value) -> Result<String, String> {
        let result = self.send_request("tools/call", json!({
            "name": tool_name,
            "arguments": arguments,
        }))?;

        // Extract text content from the response
        if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
            let texts: Vec<&str> = content.iter()
                .filter_map(|c| c.get("text").and_then(|v| v.as_str()))
                .collect();
            Ok(texts.join("\n"))
        } else {
            Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.request_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
            "params": params,
        });

        let stdin = self.child.stdin.as_mut()
            .ok_or("MCP server stdin not available")?;
        writeln!(stdin, "{}", serde_json::to_string(&request).unwrap_or_default())
            .map_err(|e| format!("failed to write to MCP server: {e}"))?;
        stdin.flush().map_err(|e| format!("flush failed: {e}"))?;

        // Read response
        let stdout = self.child.stdout.as_mut()
            .ok_or("MCP server stdout not available")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line)
            .map_err(|e| format!("failed to read from MCP server: {e}"))?;

        let response: Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("invalid JSON from MCP server: {e}"))?;

        if let Some(error) = response.get("error") {
            return Err(format!("MCP error: {}", error));
        }

        Ok(response.get("result").cloned().unwrap_or(json!({})))
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let stdin = self.child.stdin.as_mut()
            .ok_or("MCP server stdin not available")?;
        writeln!(stdin, "{}", serde_json::to_string(&notification).unwrap_or_default())
            .map_err(|e| format!("failed to write notification: {e}"))?;
        stdin.flush().map_err(|e| format!("flush failed: {e}"))?;
        Ok(())
    }
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Manager for multiple MCP server connections.
pub struct McpClientManager {
    connections: BTreeMap<String, McpConnection>,
}

impl McpClientManager {
    pub fn new() -> Self {
        Self { connections: BTreeMap::new() }
    }

    /// Connect to all configured MCP servers.
    pub fn connect_all(configs: &[McpServerConfig]) -> Self {
        let mut mgr = Self::new();
        for config in configs {
            if config.enabled == Some(false) {
                continue;
            }
            match McpConnection::connect(config) {
                Ok(conn) => {
                    eprintln!("MCP: connected to '{}' ({} tools)", config.name, conn.tools.len());
                    mgr.connections.insert(config.name.clone(), conn);
                }
                Err(e) => {
                    eprintln!("MCP: failed to connect to '{}': {e}", config.name);
                }
            }
        }
        mgr
    }

    /// Get all discovered tools across all servers.
    pub fn all_tools(&self) -> Vec<&McpTool> {
        self.connections.values()
            .flat_map(|c| c.tools.iter())
            .collect()
    }

    /// Call a tool by its qualified name (mcp__<server>__<tool>).
    pub fn call_tool(&mut self, qualified_name: &str, arguments: &Value) -> Result<String, String> {
        // Parse mcp__<server>__<tool> → (server, tool)
        let parts: Vec<&str> = qualified_name.splitn(3, "__").collect();
        if parts.len() < 3 || parts[0] != "mcp" {
            return Err(format!("invalid MCP tool name: {qualified_name} (expected mcp__<server>__<tool>)"));
        }
        let server_name = parts[1];
        let tool_name = parts[2];

        let conn = self.connections.get_mut(server_name)
            .ok_or_else(|| format!("MCP server '{}' not connected", server_name))?;

        conn.call_tool(tool_name, arguments)
    }

    /// Check if a qualified name is an MCP tool.
    pub fn is_mcp_tool(&self, name: &str) -> bool {
        name.starts_with("mcp__")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tool_qualified_name() {
        let tool = McpTool {
            server_name: "database".to_string(),
            tool_name: "query".to_string(),
            description: "Run SQL".to_string(),
            input_schema: json!({}),
        };
        assert_eq!(tool.qualified_name(), "mcp__database_query");
    }

    #[test]
    fn mcp_client_manager_rejects_bad_names() {
        let mut mgr = McpClientManager::new();
        let err = mgr.call_tool("not_mcp_tool", &json!({})).unwrap_err();
        assert!(err.contains("invalid MCP tool name"));
    }

    #[test]
    fn mcp_client_manager_rejects_unknown_server() {
        let mut mgr = McpClientManager::new();
        let err = mgr.call_tool("mcp__unknown__tool", &json!({})).unwrap_err();
        assert!(err.contains("not connected"));
    }
}
