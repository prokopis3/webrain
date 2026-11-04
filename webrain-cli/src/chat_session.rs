//! The `--features chat` half of `webrain chat`: a real Copilot session whose
//! model traffic goes to the user's key (`ProviderConfig`, BYOK) and whose tools
//! are *this binary* spawned as an MCP server (`webrain mcp`, stdio).
//!
//! Why self-attach instead of shipping tools: the 18 `webrain_*` tools already
//! exist and are maintained; the agent gets them as `webrain-webrain_*` with no
//! second implementation to drift. The child inherits this process's env, so
//! `CDP_URL`, the vault and the engine cache apply unchanged.
//!
//! ponytail: no REPL, no session resume, no tool allowlist — one prompt, one
//! turn, an exit code. Everything below is the smallest path through the SDK.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use github_copilot_sdk::types::{
    McpServerConfig, McpStdioServerConfig, PermissionRequestData, PermissionRequestKind,
    ProviderConfig, SessionConfig,
};
use github_copilot_sdk::{Client, ClientOptions, IndexMap, MessageOptions};

use super::chat::Args;
use webrain_core::byok::ChatProvider;

/// Text and tool names gathered off the event stream, because `send_and_wait`
/// only returns the terminal event.
#[derive(Default)]
struct Collected {
    deltas: String,
    final_text: Option<String>,
    tools: Vec<String>,
}

/// Reads the first string present among several key spellings: event payloads
/// are untyped JSON and the exact key differs between event kinds (and has
/// moved between SDK releases before).
fn first_str<'a>(data: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| data.get(*k).and_then(|v| v.as_str()))
}

/// Without `--yes`: reads, navigation and MCP calls run; a shell command or a
/// file write is the agent reaching outside the web, which a human should see.
fn not_destructive(d: &PermissionRequestData) -> bool {
    if d.managed_approval_required == Some(true) {
        return false;
    }
    !matches!(
        d.kind,
        Some(PermissionRequestKind::Shell) | Some(PermissionRequestKind::Write)
    )
}

pub async fn run(args: &Args, provider: &ChatProvider, prompt: &str) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;

    // Self-attach: server key `webrain`, so tools surface as
    // `webrain-webrain_navigate` etc. Env is inherited (no `env` entries), which
    // is what makes CDP_URL / the vault / the engine cache agree with the CLI.
    let mut servers: IndexMap<String, McpServerConfig> = IndexMap::new();
    servers.insert(
        "webrain".to_string(),
        McpServerConfig::Stdio(McpStdioServerConfig {
            command: exe.to_string_lossy().into_owned(),
            args: vec!["mcp".to_string()],
            ..Default::default()
        }),
    );

    // `ProviderConfig` is #[non_exhaustive], so fields are set on a default.
    let mut provider_cfg = ProviderConfig::default();
    provider_cfg.provider_type = Some(provider.provider_type.to_string());
    provider_cfg.base_url = provider.base_url.clone();
    provider_cfg.api_key = provider.bearer_token().map(str::to_string);
    provider_cfg.model_id = Some(provider.model.clone());
    provider_cfg.max_output_tokens = args.max_output_tokens.map(i64::from);

    let mut config = SessionConfig::default()
        .with_model(provider.model.clone())
        .with_mcp_servers(servers);
    if args.yes {
        config = config.approve_all_permissions();
    } else {
        config = config.approve_permissions_if(not_destructive);
    }
    config.provider = Some(provider_cfg);
    config.streaming = Some(args.stream);

    let mut options = ClientOptions::default();
    options.working_directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let client = Client::start(options)
        .await
        .map_err(|e| anyhow::anyhow!("could not start the Copilot runtime: {e}"))?;
    let session = client
        .create_session(config)
        .await
        .map_err(|e| anyhow::anyhow!("could not create the session: {e}"))?;

    // Subscribe before sending: a create-time turn can emit deltas before a
    // late subscriber exists.
    let mut events = session.subscribe();
    let sink = Arc::new(Mutex::new(Collected::default()));
    let reader_sink = sink.clone();
    let streaming = args.stream;
    // WEBRAIN_CHAT_DEBUG=1 mirrors every raw event to stderr. It is the only
    // way to see what the runtime actually emitted when a turn fails with no
    // assistant message (the SDK's own error text does not say which provider
    // rejected what).
    let debug = std::env::var_os("WEBRAIN_CHAT_DEBUG").is_some();
    let reader = tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            if debug {
                eprintln!("[event] {} {}", ev.event_type, ev.data);
            }
            match ev.event_type.as_str() {
                "assistant.message_delta" => {
                    if let Some(d) = first_str(&ev.data, &["delta", "text"]) {
                        if streaming {
                            print!("{d}");
                            use std::io::Write;
                            let _ = std::io::stdout().flush();
                        }
                        if let Ok(mut c) = reader_sink.lock() {
                            c.deltas.push_str(d);
                        }
                    }
                }
                "assistant.message" => {
                    if let Some(t) = first_str(&ev.data, &["content", "message", "text"]) {
                        if let Ok(mut c) = reader_sink.lock() {
                            c.final_text = Some(t.to_string());
                        }
                    }
                }
                "tool.execution_start" => {
                    if let Some(n) = first_str(&ev.data, &["toolName", "tool_name", "name"]) {
                        // stderr: the answer owns stdout, so `| jq` and `> file`
                        // stay clean even while tools are running.
                        eprintln!("· {n}");
                        if let Ok(mut c) = reader_sink.lock() {
                            c.tools.push(n.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    });

    let wait =
        MessageOptions::new(prompt).with_wait_timeout(Duration::from_secs(args.timeout_secs));
    let result = session.send_and_wait(wait).await;

    // disconnect() closes the subscription so the reader task ends on its own;
    // stop() flushes runtime telemetry.
    let _ = session.disconnect().await;
    let _ = client.stop().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), reader).await;

    let collected = sink.lock().map(|mut c| std::mem::take(&mut *c)).ok();
    let answer = collected
        .as_ref()
        .and_then(|c| c.final_text.clone())
        .or_else(|| {
            collected
                .as_ref()
                .map(|c| c.deltas.clone())
                .filter(|d| !d.is_empty())
        })
        .unwrap_or_default();
    let tools = collected.map(|c| c.tools).unwrap_or_default();

    if streaming && !answer.is_empty() {
        println!();
    }
    if !args.stream && !answer.is_empty() {
        println!("{answer}");
    }

    if let Err(e) = result {
        if args.json {
            println!(
                "{}",
                serde_json::json!({
                    "provider": provider.provider_type,
                    "model": provider.model,
                    "base_url": provider.base_url,
                    "answer": answer,
                    "tools": tools,
                    "error": e.to_string(),
                    "exit_code": 1,
                })
            );
        }
        anyhow::bail!("{e}");
    }

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "provider": provider.provider_type,
                "model": provider.model,
                "base_url": provider.base_url,
                "answer": answer,
                "tools": tools,
                "exit_code": 0,
            })
        );
    }
    Ok(())
}
