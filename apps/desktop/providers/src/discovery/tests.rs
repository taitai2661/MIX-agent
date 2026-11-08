use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_api_does_not_double_the_version_segment() {
        assert_eq!(
            join_api("http://127.0.0.1:1234/v1", "/v1/models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            join_api("http://127.0.0.1:1234/v1/", "/v1/models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            join_api("http://127.0.0.1:1234", "/v1/models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            join_api("http://127.0.0.1:11434/v1", "/models"),
            "http://127.0.0.1:11434/v1/models"
        );
    }

    #[test]
    fn origin_of_strips_the_api_path() {
        assert_eq!(
            origin_of("http://127.0.0.1:11434/v1"),
            "http://127.0.0.1:11434"
        );
        assert_eq!(
            origin_of("http://127.0.0.1:1234/v1/"),
            "http://127.0.0.1:1234"
        );
        assert_eq!(origin_of("127.0.0.1:8000"), "http://127.0.0.1:8000");
    }

    #[test]
    fn ollama_show_prefers_the_model_maximum_context() {
        let body = serde_json::json!({
            "details": {
                "family": "llama",
                "parameter_size": "8.0B",
                "quantization_level": "Q4_K_M",
            },
            "model_info": {"llama.context_length": 131072},
            "capabilities": ["completion", "tools", "vision", "thinking"],
        });
        let detail = ollama_details_from_show(&body, Some(42));
        assert_eq!(detail.context_window, Some(131072));
        assert_eq!(detail.parameter_size.as_deref(), Some("8.0B"));
        assert_eq!(detail.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(detail.size_bytes, Some(42));
        assert_eq!(detail.capabilities.tools, Some(true));
        assert_eq!(detail.capabilities.vision, Some(true));
        assert_eq!(detail.capabilities.reasoning, Some(true));
        assert_eq!(detail.capabilities.chat, Some(true));
    }

    #[test]
    fn lmstudio_cli_details_fill_size_and_parameter_metadata() {
        let entry = serde_json::json!({
            "type": "llm",
            "modelKey": "qwen/qwen3-8b",
            "sizeBytes": 4_623_961_222u64,
            "paramsString": "8B",
            "quantization": {"name": "4bit", "bits": 4},
            "architecture": "qwen3",
            "vision": false,
            "trainedForToolUse": true,
            "maxContextLength": 40960,
        });
        let detail = lmstudio_cli_details(&entry);
        assert_eq!(detail.context_window, Some(40960));
        assert_eq!(detail.parameter_size.as_deref(), Some("8B"));
        assert_eq!(detail.quantization.as_deref(), Some("4bit"));
        assert_eq!(detail.family.as_deref(), Some("qwen3"));
        assert_eq!(detail.size_bytes, Some(4_623_961_222));
        assert_eq!(detail.capabilities.tools, Some(true));
        assert_eq!(detail.capabilities.vision, Some(false));
    }

    #[test]
    fn lmstudio_details_treat_embedding_models_as_non_chat() {
        let entry = serde_json::json!({
            "id": "nomic-embed",
            "type": "embeddings",
            "arch": "nomic-bert",
            "state": "not-loaded",
            "max_context_length": 2048,
        });
        let detail = lmstudio_details(&entry);
        assert_eq!(detail.context_window, Some(2048));
        assert_eq!(detail.family.as_deref(), Some("nomic-bert"));
        assert_eq!(detail.capabilities.chat, Some(false));
        assert_eq!(detail.capabilities.embedding, Some(true));
        assert!(!detail.loaded);
    }

    #[test]
    fn host_of_handles_ports_credentials_and_ipv6() {
        assert_eq!(host_of("http://127.0.0.1:1234/v1"), "127.0.0.1");
        assert_eq!(host_of("https://api.openai.com/v1"), "api.openai.com");
        assert_eq!(host_of("http://localhost"), "localhost");
        assert_eq!(host_of("http://[::1]:1234/v1"), "[::1]");
        assert_eq!(host_of("http://user:pw@10.0.0.2:8000/v1"), "10.0.0.2");
    }

    #[test]
    fn loopback_hosts_are_recognised() {
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://192.168.1.20:11434/v1",
            "http://[::1]:1234/v1",
        ] {
            assert!(is_loopback_host(host_of(url)), "{url} should be local");
        }
        for url in [
            "https://api.openai.com/v1",
            "https://api.anthropic.com",
            "https://172.15.0.1/v1",
            "https://172.32.0.1/v1",
        ] {
            assert!(!is_loopback_host(host_of(url)), "{url} should not be local");
        }
    }

    #[test]
    fn loopback_is_a_subset_of_local() {
        for url in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:1234/v1",
            "http://[::1]:8000/v1",
        ] {
            assert!(is_loopback_base_url(url), "{url} should be loopback");
            assert!(is_local_base_url(url), "{url} should also be local");
        }
        for url in [
            "http://192.168.1.20:11434/v1",
            "http://10.0.0.5:11434/v1",
            "http://mac-mini.local:11434/v1",
            "http://100.100.0.1:11434/v1",
            "https://api.openai.com/v1",
        ] {
            assert!(!is_loopback_base_url(url), "{url} must not be managed");
            // The key gate is intentionally broader than process management.
            if !url.contains("openai.com") {
                assert!(is_local_base_url(url), "{url} should still be local");
            }
        }
    }

    #[test]
    fn local_base_urls_cover_aliases_and_private_overlays() {
        for url in [
            "http://127.0.0.1:1234/v1",
            "http://host.docker.internal:11434/v1",
            "http://host.containers.internal:11434/v1",
            "http://studio.local:1234/v1",
            "http://100.101.102.103:11434/v1",
            "http://[fd7a:115c:a1e0::1]:11434/v1",
            "http://[fe80::1]:11434/v1",
        ] {
            assert!(is_local_base_url(url), "{url} should be local");
        }
        for url in [
            "",
            "https://api.openai.com/v1",
            "https://api.groq.com/openai/v1",
            "https://example.local.com/v1",
            "https://100.63.0.1/v1",
        ] {
            assert!(!is_local_base_url(url), "{url} should not be local");
        }
    }

    #[test]
    fn marker_paths_reach_into_nested_bodies() {
        let ollama = serde_json::json!({
            "models": [{
                "name": "llama3.1:8b",
                "details": {"family": "llama", "parameter_size": "8.0B"},
            }],
        });
        assert!(path_get(&ollama, "models.0.details.family").is_some());
        // A path that does not exist must not resolve, or a plain OpenAI list
        // would be mistaken for an Ollama tag list.
        assert!(path_get(&ollama, "details.family").is_none());
        assert!(path_get(&ollama, "models.1.details").is_none());

        let vllm = serde_json::json!({"version": "0.6.3"});
        assert!(path_get(&vllm, "version").is_some());
    }

    #[test]
    fn parses_openai_model_lists() {
        let body = serde_json::json!({
            "object": "list",
            "data": [
                {"id": "qwen2.5-7b-instruct", "object": "model"},
                {"id": "gpt-oss-20b", "name": "GPT OSS 20B", "max_model_len": 131072},
            ],
        });
        let models = parse_openai_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "qwen2.5-7b-instruct");
        assert_eq!(models[0].label, "qwen2.5-7b-instruct");
        assert_eq!(models[0].context_window, 0);
        assert_eq!(models[1].label, "GPT OSS 20B");
        assert_eq!(models[1].context_window, 131072);
    }

    #[test]
    fn parses_ollama_tag_details() {
        let body = serde_json::json!({
            "models": [{
                "name": "llama3.1:8b",
                "details": {
                    "family": "llama",
                    "parameter_size": "8.0B",
                    "quantization_level": "Q4_K_M",
                },
            }],
        });
        let models = parse_ollama_tags(&body);
        assert_eq!(models[0].id, "llama3.1:8b");
        assert_eq!(models[0].parameter_size.as_deref(), Some("8.0B"));
        assert_eq!(models[0].quantization.as_deref(), Some("Q4_K_M"));
    }

    #[test]
    fn every_runtime_maps_to_a_preset_slug() {
        for id in RuntimeId::DETECTABLE {
            assert!(
                crate::preset::by_slug(id.slug()).is_some(),
                "{} has no preset",
                id.slug()
            );
        }
    }

    #[test]
    fn port_hints_have_unique_primary_ports() {
        // Ports are shared on purpose, but the *primary* port of each runtime
        // must be distinct or the "not online" row would claim a port another
        // runtime already answered on.
        let primaries: Vec<u16> = RuntimeId::DETECTABLE
            .iter()
            .map(|id| default_port(*id))
            .collect();
        let mut sorted = primaries.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), primaries.len());
    }

    #[test]
    fn default_base_url_comes_from_the_preset_catalogue() {
        assert_eq!(
            default_base_url(RuntimeId::Ollama),
            "http://127.0.0.1:11434/v1"
        );
        assert_eq!(
            default_base_url(RuntimeId::LmStudio),
            "http://127.0.0.1:1234/v1"
        );
    }

    #[test]
    fn custom_slug_does_not_resolve_to_a_specific_runtime() {
        // Both GPT4All and the generic OpenAI-compatible row use `custom`, so
        // resolving it would make every hand-written provider GPT4All.
        assert_eq!(RuntimeId::from_slug("custom"), None);
        assert_eq!(RuntimeId::from_slug("ollama"), Some(RuntimeId::Ollama));
        assert_eq!(RuntimeId::from_slug("lmstudio"), Some(RuntimeId::LmStudio));
    }

    #[test]
    fn capability_merge_keeps_flags_only_one_source_knows() {
        let mut caps = ModelCapabilities {
            chat: Some(true),
            tools: Some(true),
            reasoning: Some(true),
            vision: None,
            embedding: None,
        };
        caps.merge(ModelCapabilities {
            chat: Some(true),
            tools: Some(false),
            reasoning: None,
            vision: Some(true),
            embedding: None,
        });
        assert_eq!(caps.chat, Some(true));
        assert_eq!(caps.tools, Some(false), "an explicit false must win");
        assert_eq!(caps.reasoning, Some(true), "None must not erase");
        assert_eq!(caps.vision, Some(true));
    }

    #[test]
    fn install_dirs_are_relative_to_an_install_root() {
        for id in RuntimeId::DETECTABLE {
            for dir in id.install_dirs() {
                assert!(
                    !dir.starts_with('/') && !dir.starts_with("Applications/"),
                    "{id:?} install dir must not repeat a root: {dir}"
                );
                assert!(
                    !dir.starts_with("Program Files/"),
                    "{id:?} install dir must not repeat Program Files: {dir}"
                );
            }
        }
    }
}
