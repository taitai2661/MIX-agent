//! Cancellation behaviour for the native model operations.
//!
//! A real socket is used because the bug being guarded against is in the
//! transport: a cancellation flag was only checked when a chunk arrived, so a
//! server that went silent kept the UI in "cancelling" until the 600 s read
//! timeout.

use std::time::Duration;

use mixd_providers::local_ops::{ollama_pull, CancelToken};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn cancelling_a_stalled_pull_returns_immediately() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buffer = [0u8; 4096];
        let _ = socket.read(&mut buffer).await;
        // Headers only. The body never arrives, so `bytes_stream()` stays
        // pending — exactly the state that used to ignore cancellation.
        let _ = socket
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/x-ndjson\r\n\
                  transfer-encoding: chunked\r\n\r\n",
            )
            .await;
        let _ = socket.flush().await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(socket);
    });

    let cancel = CancelToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
    });

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        ollama_pull(
            &format!("http://127.0.0.1:{port}"),
            "qwen3:8b",
            cancel,
            |_| {},
        ),
    )
    .await
    .expect("cancellation must not wait for the transport timeout");

    let err = result.expect_err("a cancelled pull is an error");
    assert!(
        format!("{err:#}").contains("pull_cancelled"),
        "unexpected error: {err:#}"
    );

    server.abort();
}
