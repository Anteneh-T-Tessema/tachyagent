pub mod channels;
mod engine;
mod http;
pub mod mcp;
pub mod mcp_client;
pub mod parallel;
mod state;
mod web;

pub use channels::{load_channels, ChannelConfig, ChannelType};
pub use engine::{AgentEngine, AgentRunResult};
pub use http::serve;
pub use mcp::run_mcp_server;
pub use mcp_client::{McpClientManager, McpServerConfig, McpTool};
pub use parallel::{Orchestrator, ParallelRun, AgentTask, TaskStatus, RunStatus, execute_parallel_run};
pub use state::{DaemonState, PendingPatch};
