//! The `--features chat` half of `webrain chat`: a real Copilot session whose
//! model traffic goes to the user's key (`ProviderConfig`, BYOK) and whose tools
//! are *this binary* spawned as an MCP server (`webrain mcp`, stdio).
//!
//! Why self-attach instead of shipping tools: the 18 `webrain_*` tools already
//! exist and are maintained; the agent gets them as `webrain-webrain_*` with no
//! second implementation to drift. The child inherits this process's env, so
//! `CDP_URL`, the vault and the engine cache apply unchanged.
//!
//! ponytail: no REPL, no session resume — one prompt, one turn, an exit code.
//! Everything below is the smallest path through the SDK.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use github_copilot_sdk::types::{
    InfiniteSessionConfig, McpServerConfig, McpStdioServerConfig, PermissionRequestData,
    PermissionRequestKind, ProviderConfig, SessionConfig, SystemMessageConfig,
};
use github_copilot_sdk::{Client, ClientOptions, IndexMap, MessageOptions};

use super::chat::Args;
use webrain_core::byok::ChatProvider;

/// The session's system prompt, replacing the runtime's coding-agent default.
///
/// Two reasons to replace rather than append: the default is ~8k tokens about
/// writing code and editing repositories, which is both wrong for a scraping
/// agent and, measured on a real turn, half the per-turn prompt budget
/// (`systemTokens: 8048` against `toolDefinitionsTokens: 8080`) — enough to
/// exceed a small provider's per-minute quota before the question is asked.
fn system_prompt() -> String {
    let mut names: Vec<String> = webrain_mcp::tool_names();
    names.sort_unstable();
    format!(
        "You are webrain, a browser-automation and web-scraping agent. You answer \
         questions by driving a real browser through one tool server, `webrain`, \
         which exposes: {}.\n\n\
         How to work:\n\
         - Static HTML: `webrain_http` (no browser). Rendered/JS pages: `webrain_navigate`.\n\
         - After every navigate, read the `challenge` field. A non-null challenge means \
         the page is gated, not loaded: say so instead of extracting it as content.\n\
         - Discover selectors with `webrain_observe`/`webrain_autoschema`; never guess them.\n\
         - Report only what a tool actually returned, and name the URL you read it from. \
         If a tool failed, say what failed. Never invent page content or prices.\n\
         - Do not write files or edit the user's code unless they ask for it.\n\
         - Answer in prose, briefly, with the numbers and titles you found.",
        names.join(", ")
    )
}

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

/// Without `--yes`: reads, navigation and MCP calls run; a shell command, a
/// file write, or a permission category we do not recognise is the agent
/// reaching outside the web, and a human should see it. `Unknown` is denied on
/// purpose — the SDK only says "the CLI sent a `kind` we do not model", and
/// blanket-allowing a category nobody has defined is how a gate stops gating.
/// An absent `kind` stays allowed: the CLI omits it for ordinary tool calls,
/// so denying it would block every `webrain_*` call.
fn not_destructive(d: &PermissionRequestData) -> bool {
    if d.managed_approval_required == Some(true) {
        return false;
    }
    !matches!(
        d.kind,
        Some(PermissionRequestKind::Shell)
            | Some(PermissionRequestKind::Write)
            | Some(PermissionRequestKind::Unknown)
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

    // SDK builders (ProviderConfig::new(..).with_*), so a renamed field is a
    // compile error rather than a silently-dropped setting.
    let mut provider_cfg =
        ProviderConfig::new(provider.base_url.clone()).with_provider_type(provider.provider_type);
    if let Some(k) = provider.bearer_token() {
        provider_cfg = provider_cfg.with_api_key(k);
    }
    provider_cfg.model_id = Some(provider.model.clone());
    // Only when the user asked: the runtime already knows the model's real
    // limit, and overriding it upward makes a small-context model reject the
    // prompt as too long.
    provider_cfg.max_prompt_tokens = args.max_prompt_tokens;
    // The runtime's default asks the provider for a 64k-token completion. That
    // is a *reservation* in some billing models: measured on a real turn,
    // OpenRouter refused a request outright ("you requested up to 64000 tokens,
    // but can only afford 171"). A chat answer does not need 64k.
    provider_cfg.max_output_tokens = Some(i64::from(args.max_output_tokens.unwrap_or(4096)));

    let mut config = SessionConfig::default()
        .with_model(provider.model.clone())
        .with_provider(provider_cfg)
        .with_mcp_servers(servers)
        .with_system_message(
            SystemMessageConfig::new()
                .with_mode("replace")
                .with_content(system_prompt()),
        )
        // One-shot: no persistent workspace, no cross-session store, no
        // `~/.copilot/session-state/<id>` left behind for a command that exits.
        .with_infinite_sessions(InfiniteSessionConfig::new().with_enabled(false));
    if args.yes {
        config = config.approve_all_permissions();
    } else {
        config = config.approve_permissions_if(not_destructive);
    }
    config.streaming = Some(args.stream);
    config.enable_session_store = Some(false);
    // Keep the agent's reachable surface to our own tools. Note what this does
    // NOT do: measured (`session.usage_info`) the prompt is still
    // `toolDefinitionsTokens: 8080` / `systemTokens: 272` ~= 8.4k either way, so
    // the allowlist is a behavioural guard (the model cannot call the runtime's
    // built-in GitHub tools), not a way to shrink the prompt. Trimming the
    // budget further means fewer `webrain_*` tools, which is a product call.
    config.enable_config_discovery = Some(false);
    config.available_tools = Some(
        webrain_mcp::tool_names()
            .iter()
            .map(|n| format!("mcp:webrain-{n}"))
            .collect(),
    );

    // A process that exits must not leave the runtime holding a session: reap
    // it server-side instead of relying on the disconnect being enough.
    //
    // `base_directory` becomes `COPILOT_HOME` on the spawned CLI, which is the
    // only thing that actually stops this machine's Copilot config from being
    // read: the MCP servers, skills, and instructions under `~/.copilot` are
    // discovered by the CLI itself, before any session option applies. Without
    // it every turn loaded ~20 unrelated servers (~480 tools) and blew the
    // context window. A BYOK chat owns its configuration, so give it one.
    let home = std::env::var_os("WEBRAIN_CHAT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("webrain-copilot-home"));
    let mut options = ClientOptions::default()
        .with_session_idle_timeout_seconds(60)
        .with_base_directory(home);
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
