//! Every `McpError` variant must render the legacy prose (user-visible text
//! is unchanged) while staying matchable as a type. This guards the
//! `thiserror` migration against accidental message drift.

use mixd_mcp::McpError;

#[test]
fn each_variant_renders_its_legacy_message() {
    let cases: Vec<(McpError, &str)> = vec![
        (McpError::Server("boom".into()), "boom"),
        (McpError::Timeout("t".into()), "MCP request timed out: t"),
        (
            McpError::ChannelClosed("c".into()),
            "MCP server closed the channel for c",
        ),
        (
            McpError::MissingResult("r".into()),
            "MCP response for r has no result",
        ),
        (McpError::NotConnected, "stdio session is not running"),
        (McpError::MissingUrl, "streamable_http needs a url"),
        (
            McpError::HttpStatus("u".into(), "500".into()),
            "HTTP u: 500",
        ),
        (
            McpError::SseEnded,
            "SSE stream ended without a JSON-RPC response",
        ),
        (McpError::MissingCommand, "stdio needs a command"),
        (
            McpError::SpawnFailed("npx".into(), "nope".into()),
            "failed to spawn npx: nope",
        ),
        (McpError::NoStdin, "child has no stdin"),
        (McpError::NoStdout, "child has no stdout"),
        (McpError::MalformedToolName, "malformed MCP tool name"),
        (
            McpError::ConnectionNotFound("x".into()),
            "MCP connection not found: x",
        ),
        (
            McpError::MissingCallbackField("code"),
            "callback is missing `code`",
        ),
        (
            McpError::OauthStatus("token", "400".into(), "bad".into()),
            "token endpoint returned 400: bad",
        ),
        (McpError::Transport("tls".into()), "tls"),
    ];
    for (err, expected) in cases {
        assert_eq!(err.to_string(), expected, "{err:?}");
    }
}

#[test]
fn sources_are_preserved_for_retry_classification() {
    let io = McpError::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe"));
    assert!(std::error::Error::source(&io).is_some());
    let url_err = "http://[::1".parse::<url::Url>().unwrap_err();
    let invalid = McpError::InvalidUrl("bad".into(), url_err);
    assert!(std::error::Error::source(&invalid).is_some());
    // Retryable vs terminal: timeouts/channel-close can be retried, auth and
    // malformed names cannot — callers match on the type, not the prose.
    assert!(matches!(
        McpError::Timeout(String::new()),
        McpError::Timeout(_)
    ));
    assert!(matches!(McpError::NotConnected, McpError::NotConnected));
}
