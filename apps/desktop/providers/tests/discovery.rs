//! End-to-end checks for local runtime detection.
//!
//! A hand-rolled HTTP server stands in for each runtime so the fingerprint
//! paths, the model-list parsing, and the timeout behaviour are exercised
//! against real sockets rather than mocked responses.

use std::collections::HashMap;
use std::time::Duration;

use mixd_providers::discovery::{self, RuntimeId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve `routes` on a loopback port until the returned future is dropped.
///
/// Keys are the request path; values are the JSON body. Anything unlisted
/// answers 404, which is what a real server does for a path it does not
/// implement.
async fn serve(routes: HashMap<&'static str, String>) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let read = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let response = match routes.get(path.as_str()) {
                    Some(body) => format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    ),
                    None => "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_string(),
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    (port, handle)
}

fn routes(entries: &[(&'static str, String)]) -> HashMap<&'static str, String> {
    entries
        .iter()
        .map(|(path, body)| (*path, body.clone()))
        .collect()
}

const OPENAI_MODELS: &str = r#"{"object":"list","data":[
  {"id":"qwen2.5-7b-instruct","object":"model"},
  {"id":"gpt-oss-20b","name":"GPT OSS 20B","max_model_len":131072}
]}"#;

#[tokio::test]
async fn reads_an_openai_compatible_model_list() {
    let (port, _server) = serve(routes(&[("/v1/models", OPENAI_MODELS.into())])).await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    eprintln!("FOUND: {:?}", found);
    assert!(found.online);
    assert_eq!(found.models.len(), 2);
    assert_eq!(found.models[0].id, "qwen2.5-7b-instruct");
    // The context length is read from whichever key the server used.
    assert_eq!(found.models[1].context_window, 131072);
    assert!(found.error.is_none());
    assert_eq!(found.kind, "compatible");
}

#[tokio::test]
async fn a_base_url_without_the_version_segment_still_resolves() {
    // LM Studio's own docs show the bare origin; the preset catalogue stores
    // the `/v1` form. Both must reach the same list.
    let (port, _server) = serve(routes(&[("/v1/models", OPENAI_MODELS.into())])).await;
    for base in [
        format!("http://127.0.0.1:{port}"),
        format!("http://127.0.0.1:{port}/v1"),
    ] {
        let found = discovery::probe_base_url(&base).await.expect("probe");
        assert_eq!(found.models.len(), 2, "base {base} did not resolve");
    }
}

#[tokio::test]
async fn recognises_ollama_and_prefers_its_richer_tag_list() {
    let (port, _server) = serve(routes(&[
        (
            "/api/tags",
            r#"{"models":[{"name":"llama3.1:8b","details":{"family":"llama","parameter_size":"8.0B","quantization_level":"Q4_K_M"}}]}"#.into(),
        ),
        ("/v1/models", r#"{"data":[{"id":"llama3.1:8b"}]}"#.into()),
    ]))
    .await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    assert_eq!(found.label, "Ollama");
    assert_eq!(found.kind, "ollama");
    assert_eq!(found.models[0].parameter_size.as_deref(), Some("8.0B"));
    assert_eq!(found.models[0].quantization.as_deref(), Some("Q4_K_M"));
}

#[tokio::test]
async fn enriches_ollama_with_native_context_and_capabilities() {
    let (port, _server) = serve(routes(&[
        (
            "/api/tags",
            r#"{"models":[{"name":"llama3.1:8b","size":4930000000,"details":{"family":"llama","parameter_size":"8.0B","quantization_level":"Q4_K_M"}}]}"#.into(),
        ),
        ("/v1/models", r#"{"data":[{"id":"llama3.1:8b"}]}"#.into()),
        (
            "/api/ps",
            r#"{"models":[{"name":"llama3.1:8b","context_length":8192}]}"#.into(),
        ),
        (
            "/api/show",
            r#"{"details":{"family":"llama","parameter_size":"8.0B","quantization_level":"Q4_K_M"},"model_info":{"llama.context_length":131072},"capabilities":["completion","tools","vision","thinking"]}"#.into(),
        ),
    ]))
    .await;
    let found = discovery::probe_base_url_detailed(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    assert_eq!(found.label, "Ollama");
    let model = &found.models[0];
    assert_eq!(model.context_window, 131072);
    assert_eq!(model.size_bytes, Some(4_930_000_000));
    assert_eq!(model.family.as_deref(), Some("llama"));
    assert!(model.loaded);
    let capabilities = model.capabilities.as_ref().expect("capabilities");
    assert_eq!(capabilities.tools, Some(true));
    assert_eq!(capabilities.vision, Some(true));
    assert_eq!(capabilities.reasoning, Some(true));
    assert_eq!(capabilities.chat, Some(true));
}

#[tokio::test]
async fn enriches_lmstudio_from_its_native_model_list() {
    let (port, _server) = serve(routes(&[
        (
            "/api/v0/models",
            r#"{"data":[{"id":"qwen2.5-7b-instruct","object":"model","type":"vlm","arch":"qwen2","quantization":"Q4_K_M","state":"loaded","max_context_length":32768,"capabilities":["tool_use"]}]}"#.into(),
        ),
        (
            "/v1/models",
            r#"{"data":[{"id":"qwen2.5-7b-instruct"}]}"#.into(),
        ),
    ]))
    .await;
    let found = discovery::probe_base_url_detailed(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    assert_eq!(found.label, "LM Studio");
    let model = &found.models[0];
    assert_eq!(model.context_window, 32768);
    assert_eq!(model.family.as_deref(), Some("qwen2"));
    assert_eq!(model.quantization.as_deref(), Some("Q4_K_M"));
    assert_eq!(model.model_type.as_deref(), Some("vlm"));
    assert!(model.loaded);
    let capabilities = model.capabilities.as_ref().expect("capabilities");
    assert_eq!(capabilities.chat, Some(true));
    assert_eq!(capabilities.vision, Some(true));
    assert_eq!(capabilities.tools, Some(true));
}

#[tokio::test]
async fn recognises_vllm_by_its_version_endpoint() {
    let (port, _server) = serve(routes(&[
        ("/version", r#"{"version":"0.6.3.post1"}"#.into()),
        (
            "/v1/models",
            r#"{"data":[{"id":"meta-llama/Llama-3.1-8B"}]}"#.into(),
        ),
    ]))
    .await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    assert_eq!(found.label, "vLLM");
    assert_eq!(found.version.as_deref(), Some("0.6.3.post1"));
    assert_eq!(found.models[0].id, "meta-llama/Llama-3.1-8B");
}

#[tokio::test]
async fn recognises_llama_cpp_by_its_props_endpoint() {
    let (port, _server) = serve(routes(&[
        (
            "/props",
            r#"{"default_generation_settings":{"n_ctx":4096},"model_path":"/models/q4.gguf"}"#
                .into(),
        ),
        ("/v1/models", r#"{"data":[{"id":"q4.gguf"}]}"#.into()),
    ]))
    .await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}"))
        .await
        .expect("probe");

    assert_eq!(found.label, "llama.cpp server");
    assert_eq!(found.models[0].id, "q4.gguf");
}

#[tokio::test]
async fn recognises_koboldcpp_by_its_model_endpoint() {
    let (port, _server) = serve(routes(&[
        ("/api/v1/model", r#"{"result":"koboldcpp-l2"}"#.into()),
        ("/v1/models", r#"{"data":[{"id":"koboldcpp-l2"}]}"#.into()),
    ]))
    .await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");
    assert_eq!(found.label, "KoboldCpp");
}

#[tokio::test]
async fn an_unidentified_server_still_produces_a_usable_provider() {
    // Something the app has never heard of, serving a valid OpenAI list. It
    // must not be rejected: the conversation endpoint works the same.
    let (port, _server) = serve(routes(&[("/v1/models", OPENAI_MODELS.into())])).await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    eprintln!("FOUND: {:?}", found);
    assert!(found.online);
    assert_eq!(found.kind, "compatible");
    assert_eq!(found.models.len(), 2);
}

#[tokio::test]
async fn a_server_with_no_models_loaded_reports_why_rather_than_failing() {
    let (port, _server) = serve(routes(&[("/v1/models", r#"{"data":[]}"#.into())])).await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    assert!(found.listening);
    assert!(
        !found.online,
        "an empty model list is not an importable runtime"
    );
    assert!(found.models.is_empty());
    assert!(
        found.error.is_some(),
        "an empty model list needs an explanation"
    );
}

#[tokio::test]
async fn an_identified_ollama_with_no_models_is_online_so_the_first_pull_can_start() {
    // A fresh Ollama install has no tags yet. It is a healthy server, and the
    // app must be able to start it, show it online, and pull the first model.
    let (port, _server) = serve(routes(&[
        ("/api/tags", r#"{"models":[]}"#.into()),
        ("/api/version", r#"{"version":"0.12.0"}"#.into()),
        ("/v1/models", r#"{"data":[]}"#.into()),
    ]))
    .await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    assert!(found.listening);
    assert!(found.online, "an identified empty library is still usable");
    assert!(found.models.is_empty());
    assert!(found.error.is_none(), "no error for an empty library");
    assert_eq!(found.slug, "ollama");
}

#[tokio::test]
async fn a_non_llm_service_on_a_known_port_is_not_reported_as_a_runtime() {
    // The macOS AirPlay Receiver shape: something that answers on a port this
    // module probes, but serves no model list. It must not look importable.
    let (port, _server) = serve(routes(&[("/status", "<html>ok</html>".into())])).await;
    let found = discovery::probe_base_url(&format!("http://127.0.0.1:{port}/v1"))
        .await
        .expect("probe");

    assert!(found.listening);
    assert!(!found.online);
    assert!(found.models.is_empty());
    assert_eq!(
        found.unavailable_reason(),
        Some("このポートは他のアプリケーションが使用中です")
    );
}

#[tokio::test]
async fn a_dead_endpoint_fails_rather_than_hanging() {
    // Port 1 on loopback refuses immediately; the probe must surface that as an
    // error instead of blocking on the OS connect timeout.
    let started = std::time::Instant::now();
    let result = discovery::probe_base_url("http://127.0.0.1:1/v1").await;
    assert!(result.is_ok(), "a refused connection still yields a result");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(result.expect("result").models.len(), 0);
}

#[tokio::test]
async fn an_empty_base_url_is_rejected_without_touching_the_network() {
    assert!(discovery::probe_base_url("   ").await.is_err());
    assert!(discovery::probe_base_url("http://").await.is_err());
}

#[tokio::test]
async fn the_sweep_survives_a_machine_with_nothing_running() {
    // The contract the UI depends on: a sweep on a quiet machine completes, and
    // whatever comes back is a consistent report rather than an error.
    let report = discovery::scan_local_runtimes().await.expect("scan");
    assert!(report.hosts.contains(&"127.0.0.1".to_string()));
    for runtime in &report.runtimes {
        // Every row is either a live server or an installed-but-stopped one.
        // A runtime that is neither is omitted rather than shown as noise.
        assert!(runtime.online || runtime.installed);
        // An offline row can never carry a model list.
        if !runtime.online {
            assert!(runtime.models.is_empty());
            assert!(!runtime.listening);
        }
    }
    // A port that answered but served no model list is reported separately, so
    // it can never be mistaken for a runtime the user can import.
    for occupied in &report.occupied_ports {
        assert!(
            !report
                .runtimes
                .iter()
                .any(|r| r.online && r.port == occupied.port),
            "port {} is both occupied and importable",
            occupied.port
        );
    }
}

#[test]
fn runtime_labels_are_distinct_where_it_matters() {
    // `OpenAiCompat` is the catch-all label, so every real runtime needs its
    // own or the UI cannot tell two rows apart.
    let mut labels: Vec<&str> = RuntimeId::DETECTABLE.iter().map(|id| id.label()).collect();
    labels.sort_unstable();
    let before = labels.len();
    labels.dedup();
    assert_eq!(labels.len(), before);
}
