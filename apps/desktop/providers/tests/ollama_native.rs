//! End-to-end coverage for the native `/api/chat` transport.
//!
//! A hand-rolled HTTP server is enough: the adapter only needs a socket that
//! speaks NDJSON, and that keeps the test independent of an Ollama install.

use std::sync::Arc;

use futures_util::StreamExt;
use mixd_providers::ollama::OllamaProvider;
use mixd_providers::{Message, Provider, ProviderRequest, StreamEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve(body: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buffer = [0u8; 8192];
        let _ = socket.read(&mut buffer).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/x-ndjson\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.flush().await;
    });
    (port, handle)
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "qwen3:8b".into(),
        messages: vec![Message::text("user", "hello")],
        system: Some("be useful".into()),
        tools: Arc::new(Vec::new()),
        temperature: Some(0.0),
        top_p: None,
        max_tokens: Some(64),
        reasoning: Some(mixd_providers::ReasoningControl::High),
        stream: true,
        context_window: Some(8192),
        keep_alive: Some("5m".into()),
    }
}

#[tokio::test]
async fn streams_text_thinking_tool_calls_and_usage() {
    let body = concat!(
        r#"{"message":{"role":"assistant","content":"Hel"}}"#,
        "\n",
        r#"{"message":{"role":"assistant","content":"lo","thinking":"hmm"}}"#,
        "\n",
        r#"{"message":{"role":"assistant","tool_calls":[{"function":{"name":"mix_tool_probe","arguments":{"x":1}}}]}}"#,
        "\n",
        r#"{"done":true,"done_reason":"tool_calls","prompt_eval_count":3,"eval_count":2}"#,
    );
    let (port, _server) = serve(body).await;
    let provider = OllamaProvider::new(format!("http://127.0.0.1:{port}/v1"));
    let mut stream = provider.stream(request()).await.expect("stream");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    assert!(events
        .iter()
        .any(|event| matches!(event, StreamEvent::TextDelta(text) if text == "Hel")));
    assert!(events
        .iter()
        .any(|event| matches!(event, StreamEvent::TextDelta(text) if text == "lo")));
    assert!(events
        .iter()
        .any(|event| matches!(event, StreamEvent::ReasoningDelta(text) if text == "hmm")));
    assert!(events.iter().any(|event| matches!(
        event,
        StreamEvent::ToolCallStart { name, .. } if name == "mix_tool_probe"
    )));
    let finished = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::Finished {
                finish_reason,
                usage,
            } => Some((finish_reason, usage)),
            _ => None,
        })
        .expect("finished");
    assert_eq!(finished.0, "tool_calls");
    let usage = finished.1.as_ref().expect("usage");
    assert_eq!(usage.input_tokens, 3);
    assert_eq!(usage.output_tokens, 2);
}

/// Serve a fixed sequence of responses, one per connection.
async fn serve_sequence(responses: Vec<(u16, String)>) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = tokio::spawn(async move {
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buffer = [0u8; 8192];
            let _ = socket.read(&mut buffer).await;
            let reason = if status == 200 { "OK" } else { "Bad Request" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/x-ndjson\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });
    (port, handle)
}

#[tokio::test]
async fn retries_without_think_when_the_model_does_not_support_it() {
    let (port, _server) = serve_sequence(vec![
        (
            400,
            r#"{"error":"registry.ollama.ai/library/llama3.2 does not support thinking"}"#
                .to_string(),
        ),
        (
            200,
            r#"{"message":{"content":"hi"},"done":true,"prompt_eval_count":1,"eval_count":1}"#
                .to_string(),
        ),
    ])
    .await;
    let provider = OllamaProvider::new(format!("http://127.0.0.1:{port}/v1"));
    // `request()` asks for High reasoning, which sends `think: true` first.
    let mut stream = provider
        .stream(request())
        .await
        .expect("fallback must not fail the turn");
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if let StreamEvent::TextDelta(delta) = event {
            text.push_str(&delta);
        }
    }
    assert_eq!(text, "hi");
}
