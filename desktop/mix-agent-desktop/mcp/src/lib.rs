//! MCP client host.
//!
//! Architecture:
//!
//! ```text
//! Agent (mixd-core::runs::engine)
//!   │ call_tool("mcp__<connection>__<tool>", args)
//!   ▼
//! mixd-tools  (permission gate applies here)
//!   ▼
//! mixd-mcp::McpHost
//!   ├── Transport::Stdio          (newline-delimited JSON-RPC 2.0)
//!   └── Transport::StreamableHttp (POST + SSE)
//! ```
//!
//! Remote servers authenticate with OAuth 2.0 + PKCE; the authorization code
//! returns through the `mix-agent-desktop://` deep link so no local HTTP
//! listener is required.

pub mod host;
pub mod oauth;
pub mod registry;

pub use host::{ConnectionConfig, ConnectionStatus, McpHost, McpTool, Transport};
pub use oauth::{AuthRequest, PkcePair};
pub use registry::{collect_tool_specs, hydrate_and_connect, load_configs};

use std::sync::Arc;

/// Convenience handle held by the app state.
pub type SharedMcpHost = Arc<McpHost>;
