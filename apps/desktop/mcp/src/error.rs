//! Typed errors for the MCP client host.
//!
//! The crate used to spell every failure as a bare `String`, which made the
//! transport layer impossible to match on: callers had to compare prose to tell
//! a timeout from a protocol failure, and `?` could not carry a `source` chain.
//! Every variant below renders exactly the message the old `format!` produced,
//! so user-visible text (including `ConnectionStatus::Failed { reason }`) is
//! unchanged — only the type is new.

use thiserror::Error;

/// A failure raised by an MCP session, the host, or the OAuth helpers.
#[derive(Debug, Error)]
pub enum McpError {
    /// An error message the server itself sent, passed through verbatim.
    #[error("{0}")]
    Server(String),

    #[error("MCP request timed out: {0}")]
    Timeout(String),

    #[error("MCP server closed the channel for {0}")]
    ChannelClosed(String),

    #[error("MCP response for {0} has no result")]
    MissingResult(String),

    #[error("stdio session is not running")]
    NotConnected,

    #[error("streamable_http needs a url")]
    MissingUrl,

    #[error("HTTP {0}: {1}")]
    HttpStatus(String, String),

    #[error("SSE stream ended without a JSON-RPC response")]
    SseEnded,

    #[error("stdio needs a command")]
    MissingCommand,

    #[error("failed to spawn {0}: {1}")]
    SpawnFailed(String, String),

    #[error("child has no stdin")]
    NoStdin,

    #[error("child has no stdout")]
    NoStdout,

    #[error("malformed MCP tool name")]
    MalformedToolName,

    #[error("MCP connection not found: {0}")]
    ConnectionNotFound(String),

    #[error("invalid MCP endpoint url {0}: {1}")]
    InvalidUrl(String, #[source] url::ParseError),

    #[error("invalid callback url: {0}")]
    CallbackUrl(#[source] url::ParseError),

    #[error("callback is missing `{0}`")]
    MissingCallbackField(&'static str),

    /// `phase` is `"token"` or `"refresh"`, which is what keeps the two OAuth
    /// round trips distinguishable without doubling the variant count.
    #[error("{0} request failed: {1}")]
    OauthRequest(&'static str, #[source] reqwest::Error),

    #[error("{0} response is not JSON: {1}")]
    OauthBody(&'static str, #[source] reqwest::Error),

    #[error("{0} endpoint returned {1}: {2}")]
    OauthStatus(&'static str, String, String),

    /// Raw `io::Error` text — identical to what `map_err(|e| e.to_string())`
    /// produced before, but now carrying the original error as a source.
    #[error("{0}")]
    Io(#[from] std::io::Error),

    /// Raw `reqwest::Error` text, same reasoning as [`McpError::Io`].
    #[error("{0}")]
    Http(#[from] reqwest::Error),
}
