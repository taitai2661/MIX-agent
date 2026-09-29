//! Parse-level coverage for stored MCP connections.
//!
//! Everything here runs without a server: `config_from_json` /
//! `parse_connection` turn the stored JSON shape into a
//! [`mixd_mcp::ConnectionConfig`], and the one request-time failure that can be
//! provoked without a network is a `streamable_http` connection that was never
//! given an endpoint.

use mixd_mcp::host::{config_from_json, parse_connection, ConnectionConfig, Session, Transport};
use mixd_mcp::ConnectionStatus;
use mixd_mcp::McpError;
use serde_json::json;

#[test]
fn a_stdio_connection_keeps_its_command_and_args() {
    let value = json!({
        "name": "files",
        "transport": "stdio",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
    });
    let cfg = parse_connection(&value).expect("parse");
    assert_eq!(cfg.transport, Transport::Stdio);
    assert_eq!(cfg.name, "files");
    assert_eq!(cfg.command.as_deref(), Some("npx"));
    assert_eq!(
        cfg.args,
        vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-filesystem".to_string(),
            "/tmp".to_string()
        ]
    );
    assert!(cfg.url.is_none());
}

/// The stored shape predates the `transport` field on some rows; anything that
/// is not explicitly `streamable_http` has to keep working as stdio.
#[test]
fn an_unknown_transport_falls_back_to_stdio() {
    let value = json!({ "name": "legacy", "transport": "weird", "command": "mcp-server" });
    let cfg = config_from_json("row-1", &value).expect("parse");
    assert_eq!(cfg.transport, Transport::Stdio);
    assert_eq!(cfg.id, "row-1", "the row id becomes the connection id");
    assert_eq!(cfg.name, "legacy");
}

#[test]
fn name_defaults_to_the_row_id_when_absent() {
    let cfg = config_from_json("row-2", &json!({ "command": "mcp" })).expect("parse");
    assert_eq!(cfg.name, "row-2");
}

#[test]
fn a_streamable_http_connection_carries_its_headers() {
    let value = json!({
        "name": "remote",
        "transport": "streamable_http",
        "url": "https://mcp.example.com/mcp",
        "headers": { "Authorization": "Bearer x", "X-Tenant": "acme" },
    });
    let cfg = parse_connection(&value).expect("parse");
    assert_eq!(cfg.transport, Transport::StreamableHttp);
    assert_eq!(cfg.url.as_deref(), Some("https://mcp.example.com/mcp"));
    assert_eq!(
        cfg.headers.get("Authorization").map(String::as_str),
        Some("Bearer x")
    );
    assert_eq!(
        cfg.headers.get("X-Tenant").map(String::as_str),
        Some("acme")
    );
}

/// A row written by hand (or by an older build) can contain numbers or
/// booleans where strings are expected; those entries have to be dropped
/// instead of poisoning the whole connection.
#[test]
fn non_string_headers_and_args_are_dropped() {
    let value = json!({
        "transport": "stdio",
        "command": "mcp",
        "args": ["keep", 42, true, null],
        "headers": { "X-Count": 3, "X-Ok": true, "X-Keep": "yes" },
    });
    let cfg = parse_connection(&value).expect("parse");
    assert_eq!(cfg.args, vec!["keep".to_string()]);
    assert_eq!(cfg.headers.len(), 1);
    assert_eq!(cfg.headers.get("X-Keep").map(String::as_str), Some("yes"));
}

#[test]
fn a_remote_endpoint_must_be_an_absolute_url() {
    let err = parse_connection(&json!({
        "transport": "streamable_http",
        "url": "not a url",
    }))
    .expect_err("a relative endpoint cannot be posted to");
    // `anyhow`'s `Display` only shows the outermost context, so the chain has
    // to be rendered explicitly to reach the parser's own message.
    let chain = format!("{err:#}");
    assert!(
        chain.contains("invalid MCP endpoint url"),
        "unexpected message: {chain}"
    );
    // The context attached by `parse_connection` has to survive, so the caller
    // can tell which parser rejected the row.
    assert!(
        chain.contains("parsing MCP connection"),
        "context chain lost: {chain}"
    );
}

/// The endpoint only matters for the HTTP transport: a stdio row that happens
/// to carry a stale url must still load.
#[test]
fn a_stdio_connection_ignores_its_url() {
    let cfg = parse_connection(&json!({
        "transport": "stdio",
        "command": "mcp",
        "url": "definitely-not-a-url",
    }))
    .expect("stdio does not post to the url");
    assert_eq!(cfg.transport, Transport::Stdio);
}

/// A `streamable_http` row with no endpoint parses (the UI shows it as
/// configured) but cannot complete a handshake — and it fails before any
/// socket is opened, which is what makes it testable offline.
#[tokio::test]
async fn a_remote_connection_without_an_endpoint_fails_before_any_request() {
    let cfg = ConnectionConfig {
        id: "no-endpoint".into(),
        name: "no endpoint".into(),
        transport: Transport::StreamableHttp,
        command: None,
        args: Vec::new(),
        url: None,
        headers: Default::default(),
    };
    let session = Session::new(cfg);
    let err = session
        .connect()
        .await
        .expect_err("there is nowhere to POST to");
    assert!(matches!(err, McpError::MissingUrl), "unexpected: {err:?}");
    assert!(
        matches!(
            *session.status.read().await,
            ConnectionStatus::Failed { .. }
        ),
        "the failure has to be visible to the connection list"
    );
}
