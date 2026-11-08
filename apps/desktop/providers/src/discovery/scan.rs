use super::*;

/// `true` when the port accepted a TCP connection.
pub async fn tcp_open(host: &str, port: u16) -> bool {
    // `LOOPBACK_HOSTS` keeps the IPv6 literal in URL form (`[::1]`) so
    // `format!("http://{host}:{port}")` stays correct, but `TcpStream::connect`
    // resolves `(&str, u16)` via `ToSocketAddrs` where the brackets are not a
    // valid IP — a literal `[::1]` always failed DNS and the IPv6 half of the
    // sweep never detected anything. Strip them for the connect only.
    let host = host.trim_matches(|c| c == '[' || c == ']');
    tokio::time::timeout(
        TCP_CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Result of running every fingerprint against one open port.
pub struct Identified {
    runtime: Option<RuntimeId>,
    version: Option<String>,
    /// Present when the Ollama tag list was readable, which is the richer model
    /// source.
    ollama_tags: Option<serde_json::Value>,
}

pub async fn fingerprint(client: &reqwest::Client, origin: &str) -> Identified {
    // `join_all` preserves input order, so the position of a match in
    // `FINGERPRINTS` decides the winner. The unambiguous probes come first, and
    // the weaker `/api/v0/models` probe comes last.
    let probes: Vec<Option<(&Fingerprint, serde_json::Value)>> = join_all(
        FINGERPRINTS
            .iter()
            .map(|fp| async move {
                let url = format!("{origin}{}", fp.path);
                let body: serde_json::Value = match client.get(&url).send().await {
                    Ok(response) if response.status().is_success() => {
                        response.json().await.unwrap_or_default()
                    }
                    _ => return None,
                };
                // A 200 is not enough: most OpenAI-compatible servers answer 200
                // on paths they do not implement, with an error object. Require
                // the marker path so only a genuine match counts.
                path_get(&body, fp.marker)?;
                Some((fp, body))
            })
            .collect::<Vec<_>>(),
    )
    .await;

    match probes.into_iter().flatten().next() {
        Some((fp, body)) => {
            let ollama_tags = (fp.runtime == RuntimeId::Ollama).then(|| body.clone());
            Identified {
                runtime: Some(fp.runtime),
                version: version_of(&body),
                ollama_tags,
            }
        }
        None => Identified {
            runtime: None,
            version: None,
            ollama_tags: None,
        },
    }
}

/// Probe one `host:port` and build the runtime it turned out to be.
pub async fn probe_port(
    client: &reqwest::Client,
    host: &str,
    port: u16,
    fallback: RuntimeId,
    detailed: bool,
) -> DetectedRuntime {
    let origin = format!("http://{host}:{port}");
    let base_url = format!("{origin}/v1");
    let mut runtime = DetectedRuntime {
        slug: fallback.slug().to_string(),
        label: fallback.label().to_string(),
        kind: fallback.kind().as_str().to_string(),
        base_url: base_url.clone(),
        host: host.to_string(),
        port,
        // The caller only gets here after a successful TCP connect.
        listening: true,
        online: false,
        latency_ms: None,
        version: None,
        installed: false,
        install_hint: None,
        launcher: None,
        models: Vec::new(),
        error: None,
    };

    // The two probes that matter — the model list and the fingerprint set — are
    // issued together, so identifying a server costs one round trip.
    let started = Instant::now();
    let models_fut = async {
        let url = join_api(&base_url, "/v1/models");
        let response = client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: serde_json::Value = response.json().await.ok()?;
        Some(parse_openai_models(&body))
    };
    let (models, identified) = tokio::join!(models_fut, fingerprint(client, &origin));
    runtime.latency_ms = Some(started.elapsed().as_millis() as u64);

    // Whichever way the identity was settled — a fingerprint match, or the
    // runtime the port most commonly belongs to — the install evidence and the
    // label have to come from the same one, so a row never claims to be
    // LM Studio while reporting some other program's install path.
    //
    // Keep whether the fingerprint actually matched: an identified runtime with
    // an empty model library is a healthy server waiting for its first `pull`,
    // not a port held by an unrelated process.
    let identified_id = identified.runtime;
    let id = identified_id.unwrap_or(fallback);
    runtime.slug = id.slug().to_string();
    runtime.label = id.label().to_string();
    runtime.kind = id.kind().as_str().to_string();
    let (installed, install_hint, executable) = locate_install(id);
    runtime.installed = installed;
    runtime.install_hint = install_hint;
    runtime.launcher = executable.as_deref().and_then(|exe| id.launch_command(exe));
    runtime.version = identified.version;

    if let Some(tags) = identified.ollama_tags {
        let parsed = parse_ollama_tags(&tags);
        if !parsed.is_empty() {
            runtime.models = parsed;
        }
    }
    if runtime.models.is_empty() {
        match models {
            Some(list) if !list.is_empty() => runtime.models = list,
            // An identified runtime with an empty library is healthy: the user
            // has not pulled anything yet. Reporting an error here made a fresh
            // Ollama look like a broken port and blocked the first pull.
            Some(_) if identified_id.is_some() => {}
            Some(_) => runtime.error = Some("モデルが 1 つもロードされていません".into()),
            None if identified_id.is_some() => {}
            None => runtime.error = Some("OpenAI 互換のモデル一覧を取得できませんでした".into()),
        }
    }
    // `online` means "a runtime this app can talk to", not "has at least one
    // model". A port that answered something else — AirPlay Receiver on macOS's
    // `5000`, a stray dev server — has no fingerprint and stays out of the
    // report; an identified runtime with zero models must stay in it so the
    // start/pull flow can bootstrap the library.
    runtime.online = runtime.listening && (identified_id.is_some() || !runtime.models.is_empty());
    // The detailed pass is opt-in: the plain sweep is what the "detect" button
    // calls, and it must stay a single round trip per open port. Imports and
    // refreshes ask for `detailed`, which adds the per-model native metadata.
    if detailed && runtime.online {
        let _ = tokio::time::timeout(
            Duration::from_secs(4),
            enrich_models(id, &runtime.base_url, &mut runtime.models),
        )
        .await;
    }
    runtime
}

/// Sweep the loopback interface for local inference runtimes.
///
/// Online runtimes come first, then installed-but-stopped ones, so the UI can
/// lead with something actionable. Runtimes that are neither online nor
/// installed are omitted: a list of nine "not found" rows is noise. Ports that
/// answer but serve no model list are reported as [`OccupiedPort`] instead,
/// since silently claiming "port 5000 is text-generation-webui" when it is
/// macOS's AirPlay Receiver would be worse than saying nothing.
pub async fn scan_local_runtimes() -> anyhow::Result<ScanReport> {
    scan_local_runtimes_impl(false).await
}

/// The sweep plus per-model native metadata (context length, capabilities,
/// loaded state). Slower than [`scan_local_runtimes`] on a large Ollama
/// library, so it is used by the import flow and by an explicit refresh.
pub async fn scan_local_runtimes_detailed() -> anyhow::Result<ScanReport> {
    scan_local_runtimes_impl(true).await
}

pub async fn scan_local_runtimes_impl(detailed: bool) -> anyhow::Result<ScanReport> {
    let started = Instant::now();
    let client = http_client()?;

    let mut targets: Vec<(String, u16, RuntimeId)> = Vec::new();
    for host in LOOPBACK_HOSTS {
        for hint in PORT_HINTS {
            targets.push(((*host).to_string(), hint.port, hint.fallback));
        }
    }

    let results = join_all(targets.iter().map(|(host, port, fallback)| {
        let client = client.clone();
        async move {
            if tcp_open(host, *port).await {
                Some(probe_port(&client, host, *port, *fallback, detailed).await)
            } else {
                None
            }
        }
    }))
    .await;

    let probed: Vec<DetectedRuntime> = results.into_iter().flatten().collect();
    let mut runtimes: Vec<DetectedRuntime> = probed.iter().filter(|r| r.online).cloned().collect();
    let mut occupied: Vec<OccupiedPort> = probed
        .iter()
        .filter(|r| !r.online)
        .map(|r| OccupiedPort {
            port: r.port,
            note: occupied_port_note(r.port),
        })
        .collect();
    // A port that answers on both loopback hosts is one occupied port, not two.
    occupied.sort_by_key(|o| o.port);
    occupied.dedup_by_key(|o| o.port);

    // Online first, then lowest port, so the row a user is most likely to want
    // leads. A dual-stack server answers on both `127.0.0.1` and `[::1]` for
    // the same port, so dedupe by port and keep the IPv4 row: string ordering
    // would put `"[::1]"` first (`'[' > '1'` is false — `'['` is 91, `'1'` is
    // 49), leaving the report on the IPv6 literal.
    runtimes.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| (a.host != "127.0.0.1").cmp(&(b.host != "127.0.0.1")))
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.port.cmp(&b.port))
    });
    {
        let mut seen_ports = std::collections::HashSet::new();
        runtimes.retain(|r| seen_ports.insert(r.port));
    }

    for id in RuntimeId::DETECTABLE {
        // A runtime already answered on its own port, or on the port this one
        // would be probed at, so there is nothing to add.
        if runtimes
            .iter()
            .any(|r| r.online && (r.slug == id.slug() || r.port == default_port(*id)))
        {
            continue;
        }
        let entry = installed_only(*id);
        if entry.installed {
            runtimes.push(entry);
        }
    }

    Ok(ScanReport {
        runtimes,
        occupied_ports: occupied,
        hosts: LOOPBACK_HOSTS.iter().map(|h| (*h).to_string()).collect(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Probe one arbitrary base URL, for a runtime on another machine or on a
/// non-default port. Local-network inference boxes are common, and they are
/// indistinguishable from a loopback server by URL alone.
pub async fn probe_base_url(base_url: &str) -> anyhow::Result<DetectedRuntime> {
    probe_base_url_impl(base_url, false).await
}

/// Probe one URL and fill the model list with native metadata.
pub async fn probe_base_url_detailed(base_url: &str) -> anyhow::Result<DetectedRuntime> {
    probe_base_url_impl(base_url, true).await
}

pub async fn probe_base_url_impl(
    base_url: &str,
    detailed: bool,
) -> anyhow::Result<DetectedRuntime> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(anyhow::anyhow!("Base URL が空です"));
    }
    let with_scheme = if base_url.contains("://") {
        base_url.to_string()
    } else {
        format!("http://{base_url}")
    };
    let host = host_of(&with_scheme).to_string();
    if host.is_empty() {
        return Err(anyhow::anyhow!("ホストを解析できません: {base_url}"));
    }
    // The port has to survive being split off for the TCP check; the base URL
    // keeps it for the HTTP request. A bracketed IPv6 literal contains colons
    // itself, so `rsplit_once(':')` misparses `[::1]` (no port) as port `1]`.
    // Split the port off after the closing bracket instead.
    let authority = with_scheme
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .unwrap_or("");
    let port = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']')
            .nth(1)
            .and_then(|after| after.strip_prefix(':'))
            .and_then(|port| port.parse::<u16>().ok())
    } else {
        authority
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
    }
    .unwrap_or(if with_scheme.starts_with("https") {
        443
    } else {
        80
    });

    let client = http_client()?;
    let started = Instant::now();
    let models_fut = async {
        let url = join_api(&with_scheme, "/v1/models");
        let response = client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: serde_json::Value = response.json().await.ok()?;
        Some(parse_openai_models(&body))
    };
    let origin = format!(
        "{}://{host}:{port}",
        with_scheme.split("://").next().unwrap_or("http")
    );
    let (models, identified) = tokio::join!(models_fut, fingerprint(&client, &origin));
    // An unrecognised server is still a usable provider — the OpenAI-compatible
    // client talks to it fine — it just gets a generic label.
    let identified_id = identified.runtime;
    let runtime_id = identified_id.unwrap_or(RuntimeId::OpenAiCompat);
    let mut runtime = installed_only(runtime_id);
    runtime.base_url = with_scheme.trim_end_matches('/').to_string();
    let tcp_host = host.trim_matches(|c| c == '[' || c == ']').to_string();
    runtime.host = host;
    runtime.port = port;
    runtime.latency_ms = Some(started.elapsed().as_millis() as u64);
    runtime.version = identified.version;
    if let Some(tags) = identified.ollama_tags {
        let parsed = parse_ollama_tags(&tags);
        if !parsed.is_empty() {
            runtime.models = parsed;
        }
    }
    if runtime.models.is_empty() {
        runtime.models = models.unwrap_or_default();
    }
    // Whether the port is reachable is measured, not assumed: the caller may
    // have typed a stopped server or a dead address, and reporting
    // `listening: true` there makes `unavailable_reason()` blame another app
    // for holding the port instead of reporting the connection failure.
    // A service that answers TCP but serves no model list is still reachable
    // (and still not a provider); a refused connection is not listening.
    runtime.listening = tcp_open(&tcp_host, port).await;
    // An identified runtime with an empty library is reachable and usable for
    // `pull`; only an unidentified empty answer is a port we cannot import.
    runtime.online = runtime.listening && (identified_id.is_some() || !runtime.models.is_empty());
    if runtime.models.is_empty() && identified_id.is_none() && runtime.error.is_none() {
        runtime.error = Some("OpenAI 互換のモデル一覧を取得できませんでした".into());
    }
    // The install fields describe *this* machine, which is irrelevant to a URL
    // that may well point at another one. Default to false since we cannot
    // confirm local installation from a URL probe alone.
    runtime.installed = false;
    runtime.install_hint = None;
    runtime.launcher = None;
    if detailed && runtime.online {
        let _ = tokio::time::timeout(
            Duration::from_secs(4),
            enrich_models(runtime_id, &runtime.base_url, &mut runtime.models),
        )
        .await;
    }
    Ok(runtime)
}
