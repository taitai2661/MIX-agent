use super::*;

impl RunEngine {
    pub(super) async fn run_turn(
        &self,
        run_id: RunId,
        provider: &dyn Provider,
        request: &RunRequest,
        messages: Vec<Message>,
        system: &str,
    ) -> anyhow::Result<TurnResult> {
        let req = ProviderRequest {
            model: request.wire_model().to_string(),
            messages,
            system: if system.trim().is_empty() {
                None
            } else {
                Some(system.to_string())
            },
            tools: Arc::clone(&request.tools),
            temperature: request.temperature,
            top_p: None,
            max_tokens: request.max_tokens,
            reasoning: request.reasoning,
            stream: true,
            context_window: Some(request.context_window).filter(|value| *value > 0),
            keep_alive: request.keep_alive.clone(),
        };

        self.emit(
            run_id,
            "model_started",
            serde_json::json!({ "model_id": request.model_id }),
        )
        .await;

        // A connection-level failure (bad key, DNS, refused socket) used to
        // escape through `?` before any terminal event was emitted, so the row
        // stayed `running` and the UI spun forever. It is reported exactly like
        // an in-stream error.
        let stream = match provider.stream(req).await {
            Ok(stream) => stream,
            Err(err) => {
                tracing::error!(%run_id, provider = provider.id(), %err, "provider stream failed to open");
                return Ok(TurnResult {
                    text: String::new(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    usage: None,
                    finish_reason: None,
                    error: Some(err.to_string()),
                });
            }
        };
        let mut s = stream;
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut usage: Option<Usage> = None;
        let mut finish_reason: Option<String> = None;
        let mut error: Option<String> = None;
        let mut pending: HashMap<String, (String, String)> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

        // `cancelled_signal` is polled alongside the stream so a Run cancelled
        // while the provider is generating stops promptly instead of after the
        // whole response has arrived.
        let cancellation = self.cancellation_of(run_id).await;
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled_signal() => {
                    error = Some("run cancelled".to_string());
                    break;
                }
                ev = futures_util::StreamExt::next(&mut s) => ev,
            };
            let Some(ev) = next else {
                break;
            };
            match ev {
                StreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                    self.emit(run_id, "text", serde_json::json!({ "delta": delta }))
                        .await;
                }
                StreamEvent::ReasoningDelta(delta) => {
                    reasoning.push_str(&delta);
                    self.emit(run_id, "reasoning", serde_json::json!({ "delta": delta }))
                        .await;
                }
                StreamEvent::ToolCallStart { id, name } => {
                    pending.entry(id.clone()).or_insert((name, String::new()));
                    if !order.contains(&id) {
                        order.push(id.clone());
                    }
                }
                StreamEvent::ToolCallArgumentDelta { id, delta } => {
                    if let Some(entry) = pending.get_mut(&id) {
                        entry.1.push_str(&delta);
                    }
                }
                StreamEvent::ToolCallFinish {
                    id,
                    name,
                    arguments,
                } => {
                    let entry = pending.entry(id.clone()).or_insert((name, String::new()));
                    if !arguments.is_empty() {
                        entry.1 = arguments;
                    }
                    if !order.contains(&id) {
                        order.push(id);
                    }
                }
                StreamEvent::Finished {
                    finish_reason: reason,
                    usage: u,
                } => {
                    finish_reason = Some(reason);
                    usage = u;
                    break;
                }
                StreamEvent::Error(err) => {
                    error = Some(err);
                    break;
                }
            }
        }

        if error.is_none() && finish_reason.is_none() {
            // The connection dropped or the server closed the stream early.
            // Without this the fragment collected so far was stored as a
            // complete answer, and the run's next turn looked like progress.
            tracing::warn!(
                %run_id,
                provider = provider.id(),
                text_chars = text.len(),
                "provider stream ended without a completion signal"
            );
            error = Some("the provider stream ended without a completion signal".to_string());
        }

        // Fallback heal for leaked DeepSeek DSML markup. The OpenAI-compatible
        // adapter already filters the live stream, but any other transport —
        // or a turn assembled from cached history — can still carry raw
        // `<｜DSML｜…>` tags in `text`. Parse them back into tool calls so the
        // turn executes instead of stalling, and never persist raw tags.
        let mut tool_calls: Vec<AssistantToolCall> = order
            .iter()
            .filter_map(|id| {
                pending.get(id).map(|(name, args)| AssistantToolCall {
                    id: id.clone(),
                    kind: "function".to_string(),
                    function: AssistantToolCallFn {
                        name: name.clone(),
                        arguments: args.clone(),
                    },
                })
            })
            .collect();
        if error.is_none() && mixd_providers::dsml::contains_dsml(&text) {
            let (cleaned, healed) = mixd_providers::dsml::sanitize_assistant_text(&text);
            if !healed.is_empty() {
                tracing::warn!(
                    %run_id,
                    healed = healed.len(),
                    "healed leaked DSML tool call(s) from turn text"
                );
                for call in healed {
                    tool_calls.push(AssistantToolCall {
                        id: mixd_providers::dsml::synthesize_call_id(),
                        kind: "function".to_string(),
                        function: AssistantToolCallFn {
                            name: call.name,
                            arguments: call.arguments,
                        },
                    });
                }
            }
            text = cleaned;
        }

        Ok(TurnResult {
            text,
            reasoning,
            tool_calls,
            usage,
            finish_reason,
            error,
        })
    }
}

pub(super) struct TurnResult {
    pub(super) text: String,
    pub(super) reasoning: String,
    pub(super) tool_calls: Vec<AssistantToolCall>,
    pub(super) usage: Option<Usage>,
    pub(super) finish_reason: Option<String>,
    pub(super) error: Option<String>,
}
