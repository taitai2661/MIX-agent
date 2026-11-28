#![cfg(unix)] // the fake server is a POSIX-sh script; Windows has a different stdio adapter.

//! End-to-end check of the stdio transport.
//!
//! A tiny POSIX-sh JSON-RPC server is spawned as the MCP child process. It
//! answers `initialize`, `tools/list` and `tools/call` over stdin/stdout, so
//! this test fails if the child's pipes are inherited instead of piped (the
//! old behaviour where `stdin.take()` returned `None` and `connect()` died with
//! `NoStdin`).

use std::sync::Arc;

use mixd_mcp::host::{ConnectionConfig, Session, Transport};
use mixd_mcp::ConnectionStatus;
use serde_json::json;

const FAKE_SERVER: &str = r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fake","version":"0.1"}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"echo text","inputSchema":{"type":"object"}}]}}'
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"pong"}],"isError":false}}'
      ;;
  esac
done
"#;

fn fake_config() -> ConnectionConfig {
    ConnectionConfig {
        id: "fake-stdio".into(),
        name: "fake stdio".into(),
        transport: Transport::Stdio,
        command: Some("sh".into()),
        args: vec!["-c".into(), FAKE_SERVER.into()],
        url: None,
        headers: Default::default(),
        http_timeout_secs: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_handshake_lists_tools_and_dispatches_a_call() {
    let session = Arc::new(Session::new(fake_config()));
    session.connect().await.expect("stdio connect must work");

    assert!(
        matches!(*session.status.read().await, ConnectionStatus::Ready),
        "a successful handshake must mark the session ready"
    );
    let tools = session.tools.read().await;
    assert_eq!(tools.len(), 1, "tools/list result must be parsed");
    assert_eq!(tools[0].name, "echo");
    assert_eq!(tools[0].description, "echo text");
    drop(tools);

    let result = session
        .call_tool("echo", json!({ "text": "ping" }))
        .await
        .expect("tools/call must succeed");
    assert!(!result.is_error);
    assert_eq!(result.content[0]["text"], "pong");

    session.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_stdio_command_reports_a_typed_error() {
    let cfg = ConnectionConfig {
        id: "no-command".into(),
        name: "no command".into(),
        transport: Transport::Stdio,
        command: None,
        ..fake_config()
    };
    let session = Session::new(cfg);
    let err = session.connect().await.expect_err("no command configured");
    assert!(
        matches!(err, mixd_mcp::McpError::MissingCommand),
        "unexpected: {err:?}"
    );
}
