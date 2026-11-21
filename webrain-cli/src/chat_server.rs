//! `webrain chat --serve`: the same turn, in a browser.
//!
//! Everything the runtime knows is already on the wire — `session.subscribe()`
//! hands back typed events, and `chat_session::run` flattens them into a pipe.
//! This module keeps the events and gives each one a place on a page:
//!
//! ```text
//! GET  /            the shell — read from disk and digest-verified, not
//!                   embedded (`webrain-cli/src/chat_ui.rs`)
//! GET  /chat.js     the Svelte bundle, built from `webrain-cli/ui` and committed
//! GET  /chat.css    its stylesheet
//! GET  /events      SSE: {"type":"<runtime event>","data":{…}} + permission asks
//! GET  /sessions    the history menu (SQLite in the vault dir)
//! GET  /sessions/id that conversation's messages and its per-turn instruments
//! POST /ledger      {"n":3,"turn":{…}} → the instruments row, kept with the convo
//! GET  /catalog     every tool this build exposes and every skill it ships —
//!                   what the composer's `/`, `@` and `#` menus list
//! GET  /search      substring hits over every stored message
//! POST /attach      raw bytes + `x-name` → a file under the temp upload dir
//! POST /transcribe  raw audio + `x-name` / `x-lang` → `{"text":…}`, locally
//! POST /turn        {"prompt":"…","delivery":"turn|queue|steer"} -> 202
//! POST /stop        abort the turn that is running
//! POST /permission  {"id":"…","allow":true}
//! POST /keys        {"provider":"deepseek","key":"…"} → the key goes to the vault
//! POST /switch      {"provider":"…","model":"…","effort":"high"} → live model/effort
//!                   change, or a new session when the endpoint itself changes
//! GET  /settings    the runtime switches only this server can honour
//! POST /settings    {"memory":true,…} → saved beside the chat database
//! GET  /todos       the agent's todo rows, out of the session SQL database
//! GET  /context     why the context window is full: token breakdown + attribution
//! GET  /mcp         the MCP servers this session has, and the `webrain` one's tools
//! GET  /skills      the skills on offer, and the ones this session invoked
//! POST /fleet       {"prompt":"…"} → let the runtime dispatch sub-agents
//! GET  /tasks       the agents this session is running, as the runtime sent them
//! POST /agents      {"id":"…","text":"…"} → a word to one of them
//! ```
//!
//! Loopback only, like `webrain mcp --http` without a token: a UI that can
//! approve a shell command must not be reachable from the LAN by default.
//!
//! The event *names* are the runtime's own (`assistant.message_delta`,
//! `tool.execution_start`, …) rather than a translated dialect: the page maps
//! them to elements, and a name we do not recognise still prints instead of
//! vanishing. `webrain.permission` is the one event this file adds.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use copilot_sdk::handler::{
    PermissionHandler, PermissionResult, UserInputHandler, UserInputResponse,
};
use copilot_sdk::rpc::{
    CommandsInvokeRequest, FleetStartRequest, HandlePendingToolCallRequest,
    HistoryPreviewRewindRequest, HistoryRewindMode, HistoryRewindRequest, McpDisableRequest,
    McpEnableRequest, McpListToolsRequest, MetadataContextHeaviestMessagesRequest,
    MetadataContextInfoRequest, ModeSetRequest, NameSetRequest, PermissionsSetModeRequest,
    PlanUpdateRequest, QueueRemoveAtRequest, QueueSendNowRequest, QueueUpdateTextRequest,
    SkillsDisableRequest, SkillsEnableRequest, SlashCommandInvocationResult,
    TasksSendMessageRequest, ToolsExecuteRequest, UIEphemeralQueryRequest,
};
use copilot_sdk::session_events::{PermissionMode, SessionMode};
use copilot_sdk::types::{
    DeliveryMode, PermissionRequestData, RequestId, SessionId, SetModelOptions,
};
use copilot_sdk::{Client, session::Session};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot};

use super::chat::Args;
use super::chat_session;
use super::chat_store;
use super::chat_tools;
use webrain_core::byok::ChatProvider;

/// One frame on the event bus. SSE needs a string, so the JSON is built once
/// and cloned to every client.
type Bus = broadcast::Sender<String>;

/// How long a permission prompt waits for a human before it is denied. A turn
/// that hangs on a modal nobody saw is worse than a turn that says no.
const PERMISSION_WAIT: Duration = Duration::from_secs(120);

/// `ask_user` gets longer than a permission: it needs a typed answer.
const INPUT_WAIT: Duration = Duration::from_secs(300);

pub async fn run(args: &Args, provider: &ChatProvider) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", args.port);
    // Before the port is taken and before a runtime is started: a page that
    // cannot be served is the whole feature missing, and finding that out after
    // two seconds of session setup reads like a hang.
    let ui = super::chat_ui::load().map_err(|e| anyhow::anyhow!(e))?;
    let (bus, _) = broadcast::channel::<String>(256);
    // A job that ends while the page is idle has to say so: nothing polls the
    // terminal registry on its own, and "did my build finish" is the question a
    // background job exists to answer.
    chat_tools::watch_exit({
        let bus = bus.clone();
        move |frame| {
            let _ = bus.send(frame);
        }
    });
    let permission = Arc::new(WebPermission {
        bus: bus.clone(),
        pending: Mutex::new(HashMap::new()),
        seq: AtomicU64::new(1),
    });
    let input = Arc::new(WebInput {
        bus: bus.clone(),
        pending: Mutex::new(HashMap::new()),
        seq: AtomicU64::new(1),
    });

    // History is a convenience, never a dependency: no store means no menu, and
    // the chat still runs.
    let store = chat_store::Store::open().map(Arc::new);
    // ponytail: one conversation per server boot — choosing a past one only
    // replays it. Resuming into a live runtime session needs the SDK's own
    // store, not this table.
    let convo = format!(
        "c{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    // Read once, before the first session exists, so a page that switched
    // something on last time boots with it.
    let settings = RuntimeSettings::load();

    let live = spawn_live(
        args,
        provider,
        permission.clone(),
        input.clone(),
        bus.clone(),
        store.clone(),
        convo.clone(),
        &settings,
        None,
    )
    .await?;

    // `oserror 10048` / `EADDRINUSE` is the second-most-likely thing to happen
    // here (a previous `--serve` still running), and the OS wording says nothing
    // about it.
    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            anyhow::anyhow!(
                "{addr} is already in use — is another `webrain chat --serve` running? \
                 Stop it, or pick another port with `--port N`"
            )
        } else {
            anyhow::anyhow!("could not listen on {addr}: {e}")
        }
    })?;
    eprintln!("webrain chat · serving on http://{addr}/ · Ctrl-C to stop");
    let state = Arc::new(State {
        bus,
        permission,
        input,
        live: RwLock::new(Arc::new(live)),
        args: args.clone(),
        timeout: AtomicU64::new(settings.timeout_secs),
        settings: Mutex::new(settings),
        busy: AtomicBool::new(false),
        store,
        convo: Mutex::new(convo),
        provider: Mutex::new(provider.clone()),
        seed: AtomicBool::new(false),
        ui,
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(stream, state).await {
                // A browser closes sockets all the time: a reload drops the SSE
                // stream, a tab switch aborts an in-flight read, a prefetch is
                // cancelled. Printing those as server errors buried the real ones
                // under a wall of `os error 10053` on every page load.
                let gone = e.downcast_ref::<std::io::Error>().is_some_and(|io| {
                    matches!(
                        io.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::UnexpectedEof
                    )
                });
                if !gone {
                    eprintln!("· http · {e}");
                }
            }
        });
    }
}

/// Connect, subscribe, and start the fan-out task — one session's whole
/// lifetime in one place, because `--serve` builds a second one at runtime.
///
/// One task owns the subscription and every SSE client taps the bus;
/// subscribing here (not per request) is what makes a delta emitted during
/// session creation reach the page.
// One assembly point for a whole session: the eight arguments are the eight
// things a session is made of, and bundling them into a struct would only move
// the list somewhere else.
#[allow(clippy::too_many_arguments)]
async fn spawn_live(
    args: &Args,
    provider: &ChatProvider,
    permission: Arc<WebPermission>,
    input: Arc<WebInput>,
    bus: Bus,
    store: Option<Arc<chat_store::Store>>,
    convo: String,
    settings: &RuntimeSettings,
    // `Some(id)` restores the runtime's session for this conversation instead of
    // starting a blank one; every other caller starts fresh.
    resume: Option<SessionId>,
) -> anyhow::Result<Live> {
    // The runtime's own idea of a window is Copilot's 128k, so a 1M-token model
    // was measured at 7% of 128k and compacted at a quarter of its real size.
    // The vendor's catalogue is the authority here — it is the same number the
    // page prints — and `--max-prompt-tokens` still wins over it.
    let mut args = args.clone();
    settings.stamp(&mut args);
    args.max_prompt_tokens = args
        .max_prompt_tokens
        .or_else(|| model_context(&provider.base_url, &provider.model));
    let (client, session) = chat_session::connect(
        &args,
        provider,
        Some(permission as Arc<dyn PermissionHandler>),
        // The same card answers `ask_user`, exit-plan-mode and auto-mode.
        Some(input),
        // The conversation id names the runtime's copy of this session, so the
        // runtime can list it from creation and a later resume has a handle.
        Some(format!("webrain-{convo}")),
        // `resume` is filled by the caller that has a runtime session id to
        // restore; a plain spawn starts a fresh one.
        resume,
    )
    .await?;
    let session = Arc::new(session);
    let mut events = session.subscribe();
    let out = bus;
    let reader = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            // The final text per reply, written once — the deltas before it are
            // the same characters.
            if event.event_type == "assistant.message"
                && let Some(text) = event.data.get("content").and_then(Value::as_str)
                && let Some(store) = &store
            {
                store.append(&convo, "assistant", text);
            }
            let frame = json!({
                // Which conversation this belongs to. There is exactly one live
                // session and every open tab is a viewer of it, so without this
                // a turn still running in the previous conversation painted its
                // rows into a tab that had just started a new one.
                "convo": &convo,
                "type": event.event_type,
                "data": event.data,
            });
            // A closed bus just means nobody is watching.
            let _ = out.send(frame.to_string());
        }
    });
    Ok(Live {
        client,
        session,
        reader,
        model: std::sync::Mutex::new(provider.model.clone()),
        effort: std::sync::Mutex::new(String::new()),
        base_url: provider.base_url.clone(),
    })
}

/// How much of a reopened conversation travels with the next turn. A long chat
/// is many times this; the model needs the shape of it, not every word.
const HISTORY_CAP: usize = 6000;

/// Restart the runtime on the conversation the page just opened, and return its
/// stored transcript for the turn to carry.
///
/// A failure keeps the session already running and returns no history: a chat
/// that cannot reseed must still answer, and a transcript from another
/// conversation on top of the wrong context helps nobody.
async fn reseed(state: &Arc<State>) -> String {
    let convo = state.convo().await;
    let history = state
        .store
        .as_ref()
        .map(|s| transcript(&s.messages(&convo), HISTORY_CAP))
        .unwrap_or_default();
    // The provider the live session runs, so the restarted one is the same model
    // on the same key.
    let provider = state.provider.lock().await.clone();
    let settings = state.settings.lock().await.clone();
    match spawn_live(
        &state.args,
        &provider,
        state.permission.clone(),
        state.input.clone(),
        state.bus.clone(),
        state.store.clone(),
        convo,
        &settings,
        None,
    )
    .await
    {
        Ok(fresh) => {
            let old = std::mem::replace(&mut *state.live.write().await, Arc::new(fresh));
            // Stop the fan-out before the runtime it reads from.
            old.reader.abort();
            drop(old);
            history
        }
        Err(e) => {
            let _ = state.bus.send(
                json!({"type": "webrain.error", "data": {
                    "message": format!("could not restart the session for this conversation: {e} \u{2014} continuing the current one")
                }})
                .to_string(),
            );
            String::new()
        }
    }
}

/// The tail of a stored conversation as one block — oldest first, whole
/// messages only. The store is the only place that remembers what was said
/// before the runtime session that said it went away.
fn transcript(msgs: &[Value], cap: usize) -> String {
    let mut rows: Vec<String> = Vec::new();
    let mut used = 0usize;
    // Newest first while filling, so the cap drops the OLDEST messages.
    for m in msgs.iter().rev() {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
        let body = m.get("body").and_then(Value::as_str).unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        let row = format!("{role}: {body}");
        if used + row.len() > cap {
            break;
        }
        used += row.len() + 1;
        rows.push(row);
    }
    rows.reverse();
    rows.join("\n")
}

/// The row name a base url belongs to, which is what the page calls a provider.
/// A base url no row claims has no name of its own.
fn row_name_for_base(base: &str) -> String {
    webrain_core::byok::CHAT_PROVIDERS
        .iter()
        .find(|r| r.1 == base)
        .map(|r| r.0[0].trim_end_matches("_API_KEY").to_ascii_lowercase())
        .unwrap_or_default()
}

/// Vault service name for provider keys: one profile per provider id, the key in
/// the password field. `webrain vault list` therefore shows which keys exist
/// without ever printing one.
const KEY_SERVICE: &str = "webrain-chat-provider";

fn vault_key(id: &str) -> Option<String> {
    // `qwen` was the `dashscope` row until it was renamed: a key saved under the
    // old id has to keep working, or upgrading would silently drop it.
    let legacy = (id == "qwen").then_some("dashscope");
    std::iter::once(id).chain(legacy).find_map(|id| {
        webrain_core::vault::get(KEY_SERVICE, id)
            .ok()
            .map(|c| c.password)
            .filter(|k| !k.trim().is_empty())
    })
}

/// What the page shows about a provider it did not get from the vendor: a line of
/// prose, and — for a vendor whose docs page is the only place the figure lives —
/// the context size of the row's default model. `None` means "ask the catalogue",
/// which is not the same as zero and must not be printed as one.
const PROVIDER_INFO: [(&str, &str, Option<u64>, &str); 16] = [
    (
        "openrouter",
        "one key for hundreds of models; routes each request to whichever vendor hosts the model you pick",
        None,
        "https://openrouter.ai/docs",
    ),
    (
        "openai",
        "OpenAI's own API — GPT models, and a hosted web_search tool on the Responses endpoint",
        None,
        "https://platform.openai.com/docs/models",
    ),
    (
        "anthropic",
        "Anthropic's own API — Claude models; the only row that speaks the Messages wire format",
        None,
        "https://docs.anthropic.com/en/docs/about-claude/models",
    ),
    (
        "gemini",
        "Google's Gemini API through its OpenAI-compatible endpoint — long context, Search grounding",
        None,
        "https://ai.google.dev/gemini-api/docs/models",
    ),
    (
        "fireworks",
        "Fireworks AI — open-weight models served on their own hardware, OpenAI-compatible",
        None,
        "https://fireworks.ai/models",
    ),
    (
        "qwen",
        "Alibaba Model Studio (DashScope, international) — the Qwen family, OpenAI-compatible",
        None,
        "https://www.alibabacloud.com/help/en/model-studio/models",
    ),
    (
        "groq",
        "Groq — open-weight models on their own inference chip, very high tokens per second",
        None,
        "https://console.groq.com/docs/models",
    ),
    (
        "deepseek",
        "DeepSeek's own API — thinking mode on by default, and half price outside peak hours",
        Some(1_000_000),
        "https://api-docs.deepseek.com/quick_start/pricing",
    ),
    (
        "grok",
        "xAI — Grok models; a long context and live search on the same key",
        None,
        "https://docs.x.ai/docs/models",
    ),
    (
        "kimi",
        "Moonshot AI — Kimi, long-context Chinese and English models",
        None,
        "https://platform.moonshot.ai/docs/pricing/chat",
    ),
    (
        "zhipu",
        "Z.ai (Zhipu) — the GLM family, OpenAI-compatible",
        None,
        "https://docs.z.ai/guides/llm/glm-4.6",
    ),
    (
        "minimax",
        "MiniMax — the M-series, OpenAI-compatible",
        None,
        "https://platform.minimax.io/docs/api-reference/text-openai-api",
    ),
    (
        "hunyuan",
        "Tencent Hunyuan — OpenAI-compatible endpoint",
        None,
        "https://cloud.tencent.com/document/product/1729",
    ),
    (
        "doubao",
        "Volcengine Ark — ByteDance's Doubao models; the model id is a dated release",
        None,
        "https://www.volcengine.com/docs/82379/1330310",
    ),
    (
        "qianfan",
        "Baidu Qianfan v2 — the ERNIE family on an OpenAI-compatible endpoint",
        None,
        "https://cloud.baidu.com/doc/qianfan-api/index.html",
    ),
    (
        "siliconflow",
        "SiliconFlow — one Chinese key for Qwen, GLM, Kimi and DeepSeek weights",
        None,
        "https://docs.siliconflow.com/en/api-reference/chat-completions",
    ),
];

fn provider_info(id: &str) -> (&'static str, Option<u64>, &'static str) {
    PROVIDER_INFO
        .iter()
        .find(|r| r.0 == id)
        .map(|r| (r.1, r.2, r.3))
        .unwrap_or(("", None, ""))
}

/// How many tokens this exact model accepts, in the order the sources can be
/// trusted: the vendor's own catalogue row, then the figure the row's docs
/// publish. `None` means "do not override", which is what keeps an unknown model
/// on the runtime's default instead of a number invented here.
///
/// shortcut: only the vendor catalogues we have already fetched are consulted
/// (the page loads one per tab), so a `/switch` to a provider nobody opened this
/// second falls back to the doc figure. Fetch here instead if that gap shows.
fn model_context(base: &str, model: &str) -> Option<i64> {
    cached_models(base)
        .and_then(|ms| ms.into_iter().find(|m| m["id"].as_str() == Some(model)))
        .and_then(|m| m["context"].as_u64())
        .or_else(|| provider_info(&row_name_for_base(base)).1)
        .map(|n| n as i64)
}

/// DeepSeek's off-peak window, from its own pricing page: peak is 01:00–04:00 and
/// 06:00–10:00 UTC, Monday to Friday; every other hour — weekends and Chinese
/// public holidays in full — bills at half price. The page computes "now" from
/// this, so the rule lives in one place and not in a comment on a button.
fn off_peak(id: &str) -> Value {
    if id != "deepseek" {
        return Value::Null;
    }
    json!({
        "peak": [[1, 4], [6, 10]],
        "days": [1, 2, 3, 4, 5],
        "discount": 0.5,
        "note": "other hours, weekends and Chinese public holidays are off-peak",
        "docs": "https://api-docs.deepseek.com/quick_start/pricing",
    })
}

/// DeepSeek lists its rates in the docs and not in `/models`, so the catalogue
/// answers nothing and a DeepSeek turn would show `n/a` while its price is
/// public. These are the figures from the same page `off_peak` cites, in USD per
/// million tokens at the **peak** rate — the page halves them when the clock is
/// outside the window — with the date they were read, because a price nobody
/// re-reads is a price nobody should trust.
pub(crate) fn doc_price(base: &str, model: &str) -> Value {
    if !base.contains("deepseek.com") {
        return Value::Null;
    }
    let (prompt, completion, cached) = match model {
        "deepseek-flash" => (0.30, 1.20, 0.006),
        "deepseek-v4-pro" => (1.32, 3.96, 0.044),
        _ => return Value::Null,
    };
    json!({
        "prompt": prompt / 1e6,
        "completion": completion / 1e6,
        "input_cache_read": cached / 1e6,
        "as_of": "2026-11-05",
        "source": "https://api-docs.deepseek.com/quick_start/pricing",
    })
}

/// UTC, without a date crate: epoch seconds → hour and weekday (1970-01-01 was
/// a Thursday). Only the off-peak window needs a clock, and it is UTC by
/// definition in `off_peak`.
fn peak_now() -> bool {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let hour = (secs % 86_400) / 3_600;
    let weekday = (secs / 86_400 + 4) % 7;
    (1..=5).contains(&weekday) && ((1..4).contains(&hour) || (6..10).contains(&hour))
}

/// USD for one turn from the table `/price` serves, so the terminal and the page
/// report the same number instead of the page having money and the CLI saying
/// `n/a`. Cached input bills at its own rate, and DeepSeek's off-peak discount
/// (the 0.5 in `off_peak`) is applied here too. `None` = no published rate.
pub(crate) fn turn_cost(
    base: &str,
    model: &str,
    input: i64,
    output: i64,
    cached: i64,
) -> Option<f64> {
    let p = doc_price(base, model);
    let num = |k: &str| p.get(k).and_then(Value::as_f64);
    let (pin, pout) = (num("prompt")?, num("completion")?);
    let hit = num("input_cache_read").unwrap_or(pin);
    let cached = cached.clamp(0, input) as f64;
    let raw = (input as f64 - cached) * pin + cached * hit + output as f64 * pout;
    Some(if base.contains("deepseek.com") && !peak_now() {
        raw * 0.5
    } else {
        raw
    })
}

/// Fill in the price of every row that has none, from the one catalogue that
/// lists most vendors' models with prices. A table per vendor would be stale
/// before it was finished and would still miss half the model ids; a join on the
/// id after the last `/` — the model name every vendor uses — stays true as long
/// as the catalogue is right, and the row says where the figure came from.
/// Price every row that has none. Three sources, in the order they are worth
/// trusting: the vendor's own catalogue (already on the row), the rate its docs
/// publish (`doc_price`), and finally the one catalogue that lists everyone —
/// OpenRouter's, joined on the id after the last `/`, the model name every vendor
/// uses. A table per vendor would be stale before it was finished and would still
/// miss half the model ids; the row keeps the source it came from.
async fn price_join(base: &str, models: &mut [Value]) {
    const OR: &str = "https://openrouter.ai/api/v1";
    for m in models.iter_mut() {
        if !m["pricing"].is_null() {
            continue;
        }
        let Some(id) = m["id"].as_str() else { continue };
        let doc = doc_price(base, id);
        if !doc.is_null() {
            m["pricing"] = doc;
        }
    }
    if !models.iter().any(|m| m["pricing"].is_null()) {
        return;
    }
    let catalogue = match cached_models(OR) {
        Some(v) => v,
        // The same fetch `/models` runs, boxed: this fn is reached from
        // `list_models`, and an async fn cannot hold its own future unboxed.
        None => match Box::pin(list_models(OR, None, "openai")).await {
            Ok(v) => {
                remember_models(OR, &v);
                v
            }
            // Offline, rate-limited, or the catalogue moved: no price, no guess.
            Err(_) => return,
        },
    };
    for m in models.iter_mut() {
        if !m["pricing"].is_null() {
            continue;
        }
        let Some(id) = m["id"].as_str() else { continue };
        let want = id.rsplit('/').next().unwrap_or(id);
        let hit = catalogue.iter().find(|c| {
            !c["pricing"].is_null()
                && c["id"]
                    .as_str()
                    .and_then(|i| i.rsplit('/').next())
                    .is_some_and(|t| t.eq_ignore_ascii_case(want))
        });
        if let Some(hit) = hit {
            let mut pricing = hit["pricing"].clone();
            // Carried into the price object itself, so `/price` — which returns
            // the object, not the row — can still say where it came from.
            pricing["source"] = json!("openrouter");
            m["pricing"] = pricing;
            m["price_from"] = json!("openrouter");
        }
    }
}

/// Where a provider id points and which key reaches it. `env` is the running
/// session's own provider, a catalog row resolves its aliases, and a saved row
/// resolves the variable it named. A base url from the client is never trusted.
///
/// Order is environment first, `WEBRAIN_CHAT_API_KEY` second, vault last: an
/// exported variable keeps working, and a key pasted into the page fills the gap
/// an export would have filled.
fn provider_target(state: &State, id: &str) -> Option<(String, Option<String>, &'static str)> {
    let key = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| std::env::var(n).ok())
            .or_else(|| std::env::var("WEBRAIN_CHAT_API_KEY").ok())
            .or_else(|| vault_key(id))
    };
    if id.is_empty() || id == "env" {
        return Some((
            state
                .live
                .try_read()
                .map(|l| l.base_url.clone())
                .unwrap_or_default(),
            key(&["WEBRAIN_CHAT_API_KEY"]),
            "openai",
        ));
    }
    // Any alias of the row resolves, not just the first: the page calls the row
    // `grok` while someone reading xAI's docs types `xai`, and both are the row.
    if let Some(row) = webrain_core::byok::CHAT_PROVIDERS.iter().find(|r| {
        r.0.iter()
            .any(|n| n.trim_end_matches("_API_KEY").eq_ignore_ascii_case(id))
    }) {
        let typ = if row
            .0
            .iter()
            .any(|n| n.starts_with("ANTHROPIC") || n.starts_with("CLAUDE"))
        {
            "anthropic"
        } else {
            "openai"
        };
        return Some((row.1.to_string(), key(row.0), typ));
    }
    let row = state
        .store
        .as_ref()?
        .providers()
        .into_iter()
        .find(|p| p["id"].as_str() == Some(id))?;
    let env = row["key_env"].as_str().unwrap_or_default();
    Some((row["base_url"].as_str()?.to_string(), key(&[env]), "openai"))
}

/// A session plus everything that must die with it.
struct Live {
    /// Dropping the client stops its runtime, so it is held here, not in a local.
    /// Also the handle for client-level reads: `sessions.list` is the runtime's
    /// answer, where our own store can only answer for this machine.
    client: Client,
    session: Arc<Session>,
    reader: tokio::task::JoinHandle<()>,
    /// The model this session runs. `set_model` changes it on a live session,
    /// which is why it is behind a lock and not just a `String` beside the rest.
    model: std::sync::Mutex<String>,
    /// The `reasoning_effort` this session is actually running with. Empty until
    /// the page asks for one: a session is created before the page can switch.
    effort: std::sync::Mutex<String>,
    base_url: String,
}

impl Live {
    fn model(&self) -> String {
        self.model
            .lock()
            .map(|m| m.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    fn set_model(&self, model: &str) {
        match self.model.lock() {
            Ok(mut m) => *m = model.to_string(),
            Err(e) => *e.into_inner() = model.to_string(),
        }
    }

    /// `reasoning_effort` only travels on a model set, so the session's own
    /// effort was never applied until the page happened to open the model
    /// dialog. Apply it the first time a turn names one, and only when it
    /// changed — every turn would otherwise be a round trip for nothing.
    async fn apply_effort(&self, effort: &str) {
        let have = self
            .effort
            .lock()
            .map(|e| e.clone())
            .unwrap_or_else(|e| e.into_inner().clone());
        if have == effort {
            return;
        }
        let mut opts = SetModelOptions::default();
        if !effort.is_empty() && effort != "off" {
            opts = opts.with_reasoning_effort(effort);
        }
        if self
            .session
            .set_model(&self.model(), Some(opts))
            .await
            .is_ok()
            && let Ok(mut e) = self.effort.lock()
        {
            *e = effort.to_string();
        }
    }
}

/// The runtime switches only the server can honour — the ones a page cannot
/// express by shaping a prompt.
///
/// Kept in `settings.json` beside the chat database: a knob that resets with the
/// process is not a setting. A missing or partial file means the defaults, which
/// are exactly the constants `chat_session::connect` hard-coded before this
/// existed, so nothing changes for anyone who never opens the dialog.
/// Ranges a control may hold. Clamped here and not only in the page: `min`/`max`
/// on an `<input>` is advisory, and a one-token budget or a one-second timeout is
/// a chat that cannot answer — a legal-looking value that cannot work is worse
/// than a rejected one.
const OUT_TOKENS: (u32, u32) = (64, 200_000);
const TIMEOUT_S: (u64, u64) = (30, 3600);
const COMPACT_PCT: (u8, u8) = (30, 80);

#[derive(Clone)]
struct RuntimeSettings {
    memory: bool,
    session_store: bool,
    allow_agents: bool,
    max_output_tokens: u32,
    timeout_secs: u64,
    /// Percentage of the context window where history starts being rewritten.
    compact_at: u8,
    /// The page's reasoning effort. Empty means "let the runtime decide", which
    /// is what the first turn used before this existed.
    effort: String,
    /// Standing instructions for every turn, applied by the runtime. Empty means
    /// none.
    instructions: String,
    /// One turn may ask for several tool calls at once. Creation-time only, so
    /// flipping it costs a restart — the trade nobody minds for a faster turn.
    parallel_tools: bool,
    /// Comma-separated models the runtime may use; empty means unrestricted.
    allowed_models: String,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            memory: false,
            session_store: false,
            allow_agents: true,
            max_output_tokens: 4096,
            timeout_secs: 300,
            compact_at: 80,
            effort: String::new(),
            instructions: String::new(),
            parallel_tools: true,
            allowed_models: String::new(),
        }
    }
}

impl RuntimeSettings {
    fn load() -> Self {
        let d = Self::default();
        let Ok(text) = std::fs::read_to_string(chat_store::settings_path()) else {
            return d;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return d;
        };
        Self {
            memory: v["memory"].as_bool().unwrap_or(d.memory),
            session_store: v["session_store"].as_bool().unwrap_or(d.session_store),
            allow_agents: v["allow_agents"].as_bool().unwrap_or(d.allow_agents),
            max_output_tokens: v["max_output_tokens"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(d.max_output_tokens)
                .clamp(OUT_TOKENS.0, OUT_TOKENS.1),
            timeout_secs: v["timeout_secs"]
                .as_u64()
                .unwrap_or(d.timeout_secs)
                .clamp(TIMEOUT_S.0, TIMEOUT_S.1),
            compact_at: v["compact_at"]
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .unwrap_or(d.compact_at)
                .clamp(COMPACT_PCT.0, COMPACT_PCT.1),
            effort: v["effort"].as_str().unwrap_or_default().to_string(),
            instructions: v["instructions"].as_str().unwrap_or_default().to_string(),
            parallel_tools: v["parallel_tools"].as_bool().unwrap_or(d.parallel_tools),
            allowed_models: v["allowed_models"].as_str().unwrap_or_default().to_string(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "memory": self.memory,
            "session_store": self.session_store,
            "allow_agents": self.allow_agents,
            "max_output_tokens": self.max_output_tokens,
            "timeout_secs": self.timeout_secs,
            "compact_at": self.compact_at,
            "effort": self.effort,
            "instructions": self.instructions,
            "parallel_tools": self.parallel_tools,
            "allowed_models": self.allowed_models,
        })
    }

    /// Merge a partial body and report whether a *session* field moved. Timeout
    /// is read per turn, so changing it alone never costs a restart.
    fn merge(&mut self, v: &Value) -> bool {
        // Only the fields a live session cannot absorb. `session_store`, `effort`
        // and `instructions` are all reachable through `session.options.update`,
        // and rebuilding the session for them threw away the prompt cache for a
        // save that is usually just a wording change.
        let session = |s: &Self| {
            (
                s.memory,
                s.allow_agents,
                s.max_output_tokens,
                s.compact_at,
                s.parallel_tools,
                s.allowed_models.clone(),
            )
        };
        let before = session(self);
        if let Some(b) = v["memory"].as_bool() {
            self.memory = b;
        }
        if let Some(b) = v["session_store"].as_bool() {
            self.session_store = b;
        }
        if let Some(b) = v["allow_agents"].as_bool() {
            self.allow_agents = b;
        }
        if let Some(n) = v["max_output_tokens"].as_u64() {
            self.max_output_tokens = u32::try_from(n)
                .unwrap_or(OUT_TOKENS.1)
                .clamp(OUT_TOKENS.0, OUT_TOKENS.1);
        }
        if let Some(n) = v["timeout_secs"].as_u64() {
            self.timeout_secs = n.clamp(TIMEOUT_S.0, TIMEOUT_S.1);
        }
        if let Some(n) = v["compact_at"].as_u64() {
            self.compact_at = u8::try_from(n)
                .unwrap_or(COMPACT_PCT.1)
                .clamp(COMPACT_PCT.0, COMPACT_PCT.1);
        }
        if let Some(e) = v["effort"].as_str() {
            self.effort = e.to_string();
        }
        if let Some(t) = v["instructions"].as_str() {
            self.instructions = t.to_string();
        }
        if let Some(b) = v["parallel_tools"].as_bool() {
            self.parallel_tools = b;
        }
        if let Some(m) = v["allowed_models"].as_str() {
            self.allowed_models = m.to_string();
        }
        before != session(self)
    }

    fn save(&self) {
        let path = chat_store::settings_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.to_json().to_string());
    }

    /// Stamp onto the flags `connect` reads, so the three places that rebuild a
    /// session cannot disagree about what the page asked for.
    fn stamp(&self, args: &mut Args) {
        args.memory = Some(self.memory);
        args.session_store = Some(self.session_store);
        args.allow_agents = Some(self.allow_agents);
        args.max_output_tokens = Some(self.max_output_tokens);
        args.timeout_secs = self.timeout_secs;
        args.compact_at = Some(self.compact_at);
        // empty is "unset", which is what `connect` already treats as the default.
        args.effort = Some(self.effort.clone());
        args.instructions = Some(self.instructions.clone());
        args.parallel_tools = Some(self.parallel_tools);
        args.allowed_models = Some(self.allowed_models.clone());
    }
}

struct State {
    bus: Bus,
    permission: Arc<WebPermission>,
    input: Arc<WebInput>,
    /// Swappable: a provider change makes a new session, and the turn handler
    /// reads whichever one is current.
    live: RwLock<Arc<Live>>,
    /// The first session's flags, so a replacement is configured identically.
    args: Args,
    /// Seconds per turn. Atomic because the page can change it mid-conversation
    /// and no session has to be rebuilt for it.
    timeout: AtomicU64,
    /// What the page switched on. Read when a session is built, written by
    /// `POST /settings`.
    settings: Mutex<RuntimeSettings>,
    busy: AtomicBool,
    store: Option<Arc<chat_store::Store>>,
    /// The conversation new turns are stored in. Mutable: opening a past
    /// conversation moves it, so a continued chat lands where the page shows it.
    convo: Mutex<String>,
    /// The provider the live session runs, kept here because restarting the
    /// session (a conversation switch) has to rebuild one and `/switch` owns the
    /// key, which `Live` deliberately does not hold.
    provider: Mutex<ChatProvider>,
    /// Set when the page opens a stored conversation: the next turn restarts the
    /// runtime on it instead of continuing the previous one's context.
    seed: AtomicBool,
    /// The page, read from disk and verified once at startup — not carried in
    /// the binary (`super::chat_ui`).
    ui: super::chat_ui::Ui,
}

impl State {
    async fn convo(&self) -> String {
        self.convo.lock().await.clone()
    }
}

/// Answers a permission prompt from the page instead of the CLI's policy.
struct WebPermission {
    bus: Bus,
    // The card's payload is kept beside its sender so a page reload can ask for
    // the open requests back. Without it a refresh stranded the turn: the
    // runtime waited for an answer the page could no longer give, and nothing
    // was on screen to answer.
    pending: Mutex<HashMap<String, (oneshot::Sender<bool>, Value)>>,
    seq: AtomicU64,
}

#[async_trait::async_trait]
impl PermissionHandler for WebPermission {
    async fn handle(
        &self,
        _session: SessionId,
        _request: RequestId,
        data: PermissionRequestData,
    ) -> PermissionResult {
        let id = format!("p{}", self.seq.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        // The runtime packs a structured request inside `extra` — server, tool,
        // arguments, whether it only reads. `format!("{data:?}")` was what turned
        // the dialog into a wall of text; these are the fields a person decides on.
        let req = &data.extra["permissionRequest"];
        let card = permission_card(&id, req, data.managed_approval_required.unwrap_or(false));
        self.pending
            .lock()
            .await
            .insert(id.clone(), (tx, card.clone()));
        let ask = json!({ "type": "webrain.permission", "data": card });
        let _ = self.bus.send(ask.to_string());

        let answer = tokio::time::timeout(PERMISSION_WAIT, rx).await;
        self.pending.lock().await.remove(&id);
        match answer {
            Ok(Ok(true)) => PermissionResult::approve_once(),
            Ok(Ok(false)) => PermissionResult::reject(Some("denied in the web UI".into())),
            _ => PermissionResult::reject(Some("no answer from the web UI".into())),
        }
    }
}

/// One approval, as the page draws it. A memory write is not a tool call: the
/// runtime asks before it stores a fact that will be recalled in *other*
/// sessions, so the card has to carry the fact. Without that branch the quote
/// being remembered arrived as a tool named "tool" with no arguments — the one
/// card a reader must be able to judge was the one card that said nothing.
fn permission_card(id: &str, req: &Value, managed: bool) -> Value {
    let kind = req["kind"].as_str().unwrap_or("tool");
    if kind == "memory" {
        let action = req["action"].as_str().unwrap_or("store");
        let fact = req["fact"].as_str().unwrap_or_default();
        return json!({
            "id": id,
            "kind": "memory",
            "action": action,
            "title": if action == "store" { "remember this" } else { "vote on a memory" },
            "tool": if action == "store" { "remember" } else { "memory vote" },
            "server": "memory",
            "read_only": false,
            "managed": managed,
            // The whole point of the card: the fact, where it came from, and the
            // subject it will be filed under.
            "args": json!({
                "fact": fact,
                "subject": req["subject"].as_str().unwrap_or_default(),
                "citations": req["citations"].as_str().unwrap_or_default(),
            }),
            "fact": fact,
        });
    }
    let tool = req["toolName"].as_str().unwrap_or("tool");
    json!({
        "id": id,
        "kind": kind,
        "server": req["serverName"].as_str().unwrap_or_default(),
        // `webrain-webrain_pdf` → `pdf`: the doubling is the MCP server key plus
        // the tool name, and it means nothing to a reader.
        "tool": tool.strip_prefix("webrain-webrain_").unwrap_or(tool),
        "title": req["toolTitle"].as_str().unwrap_or(tool),
        "args": req["args"].clone(),
        "read_only": req["readOnly"].as_bool().unwrap_or(false),
        "managed": managed,
    })
}

pub(crate) struct WebInput {
    bus: Bus,
    pending: Mutex<HashMap<String, oneshot::Sender<(String, bool)>>>,
    seq: AtomicU64,
}

impl WebInput {
    /// Ask the page a question and wait for the answer. `ask_user`, exit-plan-mode
    /// and auto-mode all land here: from the page's side it is always one card.
    pub(crate) async fn ask(
        &self,
        question: String,
        choices: Option<Vec<String>>,
    ) -> Option<(String, bool)> {
        let id = format!("u{}", self.seq.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let ask = json!({
            "type": "webrain.ask",
            "data": {
                "id": id,
                "question": question,
                "choices": choices.unwrap_or_default(),
                "allow_freeform": true,
            },
        });
        let _ = self.bus.send(ask.to_string());

        // Longer than a permission: a person has to read and type an answer.
        let answer = tokio::time::timeout(INPUT_WAIT, rx).await;
        self.pending.lock().await.remove(&id);
        match answer {
            Ok(Ok(pair)) => Some(pair),
            _ => None,
        }
    }
}

/// Answers `ask_user`, which is not a permission: it is a question, and a turn
/// that gets none blocks. `None` means "no answer available" and lets the
/// runtime fall back to its own prompt.
#[async_trait::async_trait]
impl UserInputHandler for WebInput {
    async fn handle(
        &self,
        _session: SessionId,
        _request_id: Option<copilot_sdk::types::RequestId>,
        question: String,
        choices: Option<Vec<String>>,
        _allow_freeform: Option<bool>,
    ) -> Option<UserInputResponse> {
        let (answer, was_freeform) = self.ask(question, choices).await?;
        Some(UserInputResponse {
            answer,
            was_freeform,
        })
    }
}

async fn serve_conn(mut stream: TcpStream, state: Arc<State>) -> anyhow::Result<()> {
    let (head, body) = read_request(&mut stream).await?;
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    match (method, path) {
        ("GET", "/") => respond(&mut stream, 200, "text/html; charset=utf-8", &state.ui.html).await,
        ("GET", "/chat.js") => {
            respond(
                &mut stream,
                200,
                "text/javascript; charset=utf-8",
                &state.ui.js,
            )
            .await
        }
        ("GET", "/chat.css") => {
            respond(&mut stream, 200, "text/css; charset=utf-8", &state.ui.css).await
        }
        // The markdown renderer's chunk. `chat.js` imports it dynamically, so the
        // first paint never parses `marked` or DOMPurify; the page warms it when
        // the browser is idle.
        ("GET", "/chunk-markdown.js") => {
            respond(
                &mut stream,
                200,
                "text/javascript; charset=utf-8",
                &state.ui.markdown,
            )
            .await
        }
        ("GET", "/events") => stream_events(&mut stream, state.bus.subscribe()).await,
        ("POST", "/turn") => {
            let payload = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let raw = payload["prompt"].as_str().unwrap_or_default().to_string();
            let mode = payload["mode"].as_str().unwrap_or("agent");
            if raw.trim().is_empty() {
                return respond(
                    &mut stream,
                    400,
                    "application/json",
                    r#"{"error":"prompt"}"#,
                )
                .await;
            }
            // Modes and the composer's own picks are instructions, not a hard
            // tool switch: the runtime's tool set is fixed at session creation.
            let effort = payload["effort"].as_str().unwrap_or("medium");
            // A conversation opened from history gets its own runtime session:
            // the live one still holds whatever was said before the switch, and
            // two histories in one context is worse than a fresh session with
            // the stored transcript on the front of the turn.
            let history = if state.seed.swap(false, Ordering::SeqCst) {
                reseed(&state).await
            } else {
                String::new()
            };
            // Before the busy check: a queued or steering message must not
            // change the model mid-turn, so one that is already running keeps
            // the effort it started with and the next turn picks the new one.
            if !state.busy.load(Ordering::SeqCst) {
                state.live.read().await.clone().apply_effort(effort).await;
            }
            let tools: Vec<String> = payload["tools"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .take(24)
                        .collect()
                })
                .unwrap_or_default();
            // Built-in tools the page unticked. They are installed on the session
            // either way — the allowlist is fixed at creation — so this is the
            // only lever there is, and it is the same one the preferred list uses.
            let tools_off: Vec<String> = payload["tools_off"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .take(24)
                        .collect()
                })
                .unwrap_or_default();
            // Plan mode WRITES the plan; every other mode reads it, so "do it"
            // after a plan still knows what the plan was. A mode is otherwise
            // only a sentence on the front of the prompt.
            let plan = if mode == "plan" {
                String::new()
            } else {
                state
                    .live
                    .read()
                    .await
                    .session
                    .rpc()
                    .plan()
                    .read()
                    .await
                    .ok()
                    .and_then(|r| r.content)
                    .unwrap_or_default()
            };
            let prompt = with_controls(
                &with_mode(&expand_mentions(&raw), mode, &plan),
                effort,
                &tools,
                &tools_off,
                &user_notes(
                    payload["instructions"].as_str().unwrap_or_default(),
                    payload["language"].as_str().unwrap_or_default(),
                ),
            );
            // Empty for every turn except the first one after opening a stored
            // conversation, which is the only turn that has history to carry.
            let prompt = if history.is_empty() {
                prompt
            } else {
                format!("[earlier in this conversation]\n{history}\n\n{prompt}")
            };
            // `delivery` is the runtime's own knob, not a queue this page
            // invents: `queue` waits behind the turn that is running, `steer`
            // interrupts it and runs now. Only a fresh turn is refused while one
            // runs, because two answers interleaving in one transcript is what a
            // single composer cannot show.
            let delivery = payload["delivery"].as_str().unwrap_or("turn");
            if state.busy.swap(true, Ordering::SeqCst) {
                if delivery != "queue" && delivery != "steer" {
                    return respond(&mut stream, 409, "application/json", r#"{"error":"busy"}"#)
                        .await;
                }
                if let Some(store) = &state.store {
                    store.append(&state.convo().await, "user", &raw);
                }
                let live = state.live.read().await.clone();
                let opts = copilot_sdk::MessageOptions::new(prompt.as_str()).with_mode(
                    if delivery == "steer" {
                        DeliveryMode::Immediate
                    } else {
                        DeliveryMode::Enqueue
                    },
                );
                let reply = match live.session.send(opts).await {
                    Ok(id) => json!({ "delivery": delivery, "message": id }),
                    Err(e) => json!({ "error": e.to_string() }),
                };
                let code = if reply.get("error").is_some() {
                    500
                } else {
                    200
                };
                return respond(&mut stream, code, "application/json", &reply.to_string()).await;
            }
            if let Some(store) = &state.store {
                store.append(&state.convo().await, "user", &raw);
            }
            let live = state.live.read().await.clone();
            let st = state.clone();
            let plan_mode = mode == "plan";
            tokio::spawn(async move {
                let wait = copilot_sdk::MessageOptions::new(prompt.as_str())
                    .with_wait_timeout(Duration::from_secs(st.timeout.load(Ordering::SeqCst)));
                let turn = live.session.send_and_wait(wait);
                tokio::pin!(turn);

                // Same sampler as the CLI: the runtime only reports tokens per
                // *model call*, through `usage.getMetrics`. Without this the page
                // shows a stream of text and no idea what it cost.
                let mut ticker = tokio::time::interval(Duration::from_millis(900));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut seen = (0i64, 0i64);
                let outcome = loop {
                    tokio::select! {
                        r = &mut turn => break r,
                        _ = ticker.tick() => {
                            if let Ok(m) = live.session.rpc().usage().get_metrics().await {
                                let call = (m.last_call_input_tokens, m.last_call_output_tokens);
                                if call != seen && call.1 > 0 {
                                    seen = call;
                                    let _ = st.bus.send(
                                        json!({"type": "webrain.model", "data": {
                                            "model": live.model(),
                                            "input": call.0, "output": call.1,
                                        }})
                                        .to_string(),
                                    );
                                }
                            }
                        }
                    }
                };
                if let Err(e) = outcome {
                    let _ = st.bus.send(
                        json!({"type": "webrain.error", "data": {"message": e.to_string()}})
                            .to_string(),
                    );
                } else if plan_mode
                    && let Some(text) = outcome
                        .ok()
                        .flatten()
                        .and_then(|ev| ev.data.get("content")?.as_str().map(str::to_string))
                        .filter(|t| !t.trim().is_empty())
                {
                    // The answer to a plan-mode turn IS the plan. Stored through
                    // the runtime so the next turn can be told to execute it
                    // without the whole plan being pasted in again.
                    if let Err(e) = live
                        .session
                        .rpc()
                        .plan()
                        .update(PlanUpdateRequest { content: text })
                        .await
                    {
                        let _ = st.bus.send(
                            json!({"type": "webrain.error", "data": {
                                "message": format!("planned, but could not save the plan: {e}")
                            }})
                            .to_string(),
                        );
                    }
                }
                // The closing block. `nano_aiu` is 0 on BYOK turns — the key
                // pays the provider — so the page shows cost as n/a rather than
                // inventing one.
                if let Ok(m) = live.session.rpc().usage().get_metrics().await {
                    let t = chat_session::totals_of(&m);
                    let _ = st.bus.send(
                        json!({"type": "webrain.usage", "data": {
                            "model": live.model(),
                            "input": t.input,
                            "output": t.output,
                            "cached": t.cached,
                            "requests": t.requests,
                            "api_ms": t.api_ms,
                            "tokens_per_sec": t.tokens_per_sec(),
                            "nano_aiu": t.nano_aiu,
                        }})
                        .to_string(),
                    );
                }
                st.busy.store(false, Ordering::SeqCst);
            });
            respond(&mut stream, 202, "application/json", r#"{"started":true}"#).await
        }
        // Stop: the runtime's own abort. The turn's `send_and_wait` then returns,
        // which is what clears `busy` — this route only asks.
        ("POST", "/stop") => {
            let live = state.live.read().await.clone();
            let reply = match live.session.abort().await {
                Ok(()) => json!({ "stopped": true }),
                Err(e) => json!({ "stopped": false, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // The plan on file: written by a plan-mode turn, read into every later
        // prompt. Exposed so the page (or a script) can see what the next turn
        // will be told, instead of trusting that the agent remembers.
        ("GET", "/plan") => {
            let live = state.live.read().await.clone();
            let body = match live.session.rpc().plan().read().await {
                Ok(r) => json!({
                    "exists": r.exists,
                    "content": r.content.unwrap_or_default(),
                    "path": r.path,
                }),
                Err(e) => json!({ "exists": false, "content": "", "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // Put the last file change back. The tool row already shows the diff;
        // this is the half a diff cannot do.
        ("POST", "/undo") => {
            let body = match crate::chat_tools::undo_last() {
                Ok(what) => json!({ "restored": what }),
                Err(e) => json!({ "error": e }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // The switches a page cannot express as prompt text. Saved next to the
        // chat database so they outlive the process.
        ("GET", "/settings") => {
            let s = state.settings.lock().await.clone().to_json();
            let body = json!({ "settings": s }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        ("POST", "/settings") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let mut s = state.settings.lock().await;
            let restart = s.merge(&v);
            s.save();
            state.timeout.store(s.timeout_secs, Ordering::SeqCst);
            // The three that do not need a rebuild go into the live session now;
            // `None` means "leave it", so an empty effort sends nothing.
            // 2.0.1 groups the flat patch: instructions / state / inference.
            let patch = copilot_sdk::rpc::SessionUpdateOptionsParams {
                instructions: Some(copilot_sdk::rpc::SessionUpdateInstructionOptions {
                    organization_text: Some(s.instructions.clone()),
                    ..Default::default()
                }),
                state: Some(copilot_sdk::rpc::SessionUpdateStateOptions {
                    enable_cross_session_store: Some(s.session_store),
                    ..Default::default()
                }),
                inference: if s.effort.is_empty() {
                    None
                } else {
                    Some(copilot_sdk::rpc::SessionUpdateInferenceOptions {
                        reasoning_effort: Some(s.effort.clone()),
                        ..Default::default()
                    })
                },
                ..Default::default()
            };
            let out = json!({ "saved": true, "restart": restart });
            drop(s);
            let live = state.live.read().await.clone();
            let patched = match live.session.rpc().options().update(patch).await {
                Ok(_) => json!({ "ok": true }),
                Err(e) => json!({ "error": e.to_string() }),
            };
            let mut out = out;
            out["patched"] = patched;
            // A session field cannot reach a session that already exists, so the
            // next turn rebuilds one — the same lever opening a past conversation
            // uses, and the page's transcript is what carries over.
            if restart && !state.busy.load(Ordering::SeqCst) {
                state.seed.store(true, Ordering::SeqCst);
            }
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        // The agent's own todo rows, straight out of the session SQL database.
        // The plan file says what the work is; these say how far it got.
        ("GET", "/todos") => {
            let live = state.live.read().await.clone();
            let body = match live
                .session
                .rpc()
                .plan()
                .read_sql_todos_with_dependencies()
                .await
            {
                Ok(r) => json!({
                    "rows": r.rows.iter().map(|t| json!({
                        "id": t.id, "title": t.title,
                        "status": t.status, "description": t.description,
                    })).collect::<Vec<_>>(),
                    "dependencies": r.dependencies.iter().map(|d| json!({
                        "todo": d.todo_id, "depends_on": d.depends_on,
                    })).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "rows": [], "dependencies": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // The runtime's own token breakdown answers "why is the context full"
        // instead of leaving it to a guess. Attribution is passed through
        // untouched: its shape is the CLI's, not ours.
        ("GET", "/context") => {
            let live = state.live.read().await.clone();
            // Both limits at 0 means "resolve from the session and its model".
            let info = live
                .session
                .rpc()
                .metadata()
                .context_info(MetadataContextInfoRequest {
                    output_token_limit: 0,
                    prompt_token_limit: 0,
                    ..Default::default()
                })
                .await;
            let attribution = live
                .session
                .rpc()
                .metadata()
                .get_context_attribution()
                .await;
            // Best effort, like the attribution above: the attribution says which
            // *category* is expensive, this says which message is. A silent empty
            // list cannot be told from a failed call, so the failure is named.
            let heaviest = live
                .session
                .rpc()
                .metadata()
                .get_context_heaviest_messages(MetadataContextHeaviestMessagesRequest {
                    limit: Some(5),
                })
                .await;
            let heaviest_error = heaviest.as_ref().err().map(|e| e.to_string());
            let heaviest: Vec<Value> = heaviest
                .map(|r| {
                    r.messages
                        .into_iter()
                        .map(|m| json!({"label": m.label, "role": m.role, "tokens": m.tokens}))
                        .collect()
                })
                .unwrap_or_default();
            let mut body = match (info, attribution) {
                (Ok(r), Ok(a)) => match r.context_info {
                    Some(c) => json!({
                        "totalTokens": c.total_tokens,
                        "promptTokenLimit": c.prompt_token_limit,
                        "systemTokens": c.system_tokens,
                        "conversationTokens": c.conversation_tokens,
                        "attribution": a,
                        "heaviest": heaviest,
                    }),
                    None => json!({
                        "totalTokens": 0,
                        "promptTokenLimit": 0,
                        "systemTokens": 0,
                        "conversationTokens": 0,
                        "attribution": a,
                        "heaviest": heaviest,
                        "error": "no context snapshot yet — the session has not initialised",
                    }),
                },
                (Ok(_), Err(e)) => json!({
                    "totalTokens": 0,
                    "promptTokenLimit": 0,
                    "systemTokens": 0,
                    "conversationTokens": 0,
                    "attribution": null,
                    "heaviest": heaviest,
                    "error": e.to_string(),
                }),
                (Err(e), _) => json!({
                    "totalTokens": 0,
                    "promptTokenLimit": 0,
                    "systemTokens": 0,
                    "conversationTokens": 0,
                    "attribution": null,
                    "heaviest": heaviest,
                    "error": e.to_string(),
                }),
            };
            if let Some(e) = heaviest_error {
                body["heaviestError"] = json!(e);
            }
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // Whether the server this session attached to itself is actually up:
        // `list` reports connection status, `list_tools` proves it answers.
        ("GET", "/mcp") => {
            let live = state.live.read().await.clone();
            let body = match live.session.rpc().mcp().list().await {
                Ok(r) => {
                    let tools = live
                        .session
                        .rpc()
                        .mcp()
                        .list_tools(McpListToolsRequest {
                            server_name: "webrain".to_string(),
                        })
                        .await;
                    match tools {
                        Ok(t) => json!({ "servers": r.servers, "tools": t.tools }),
                        Err(e) => {
                            json!({ "servers": r.servers, "tools": [], "error": e.to_string() })
                        }
                    }
                }
                Err(e) => json!({ "servers": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // Which skills exist is one question, which ones the agent used is
        // another — an unused skill is a prompt problem, not a missing file.
        ("GET", "/skills") => {
            let live = state.live.read().await.clone();
            let list = live.session.rpc().skills().list().await;
            let invoked = live.session.rpc().skills().get_invoked().await;
            let body = match (list, invoked) {
                (Ok(l), Ok(i)) => json!({ "skills": l.skills, "invoked": i.skills }),
                (Ok(l), Err(e)) => {
                    json!({ "skills": l.skills, "invoked": [], "error": e.to_string() })
                }
                (Err(e), _) => json!({ "skills": [], "invoked": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // Turn one skill on or off in the live session. The tool set is fixed at
        // creation, so before this the only way to drop a skill was to restart —
        // and a skill's tools went with it.
        ("POST", "/skills") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let name = v["name"].as_str().unwrap_or_default().trim().to_string();
            if name.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"name"}"#).await;
            }
            let on = v["enabled"].as_bool().unwrap_or(true);
            let live = state.live.read().await.clone();
            let skills = live.session.rpc().skills();
            let r = if on {
                skills
                    .enable(SkillsEnableRequest { name: name.clone() })
                    .await
            } else {
                skills
                    .disable(SkillsDisableRequest { name: name.clone() })
                    .await
            };
            let reply = match r {
                Ok(()) => json!({ "name": name, "enabled": on }),
                Err(e) => json!({ "name": name, "enabled": !on, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // Start or stop the MCP server this session attached to itself. A stopped
        // server is why a turn suddenly has no webrain tools; the page can now say
        // so and fix it without a restart.
        ("POST", "/mcp") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let name = v["server"].as_str().unwrap_or("webrain").trim().to_string();
            let on = v["enabled"].as_bool().unwrap_or(true);
            let live = state.live.read().await.clone();
            let mcp = live.session.rpc().mcp();
            let r = if on {
                mcp.enable(McpEnableRequest {
                    server_name: name.clone(),
                    // We attach at session creation, so there is no receipt to match.
                    expected_installation_id: None,
                })
                .await
            } else {
                mcp.disable(McpDisableRequest {
                    server_name: name.clone(),
                    expected_installation_id: None,
                })
                .await
            };
            let reply = match r {
                Ok(()) => json!({ "server": name, "enabled": on }),
                Err(e) => json!({ "server": name, "enabled": !on, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // Run one tool directly, with arguments the page can edit. This is the
        // difference between "the model called the wrong selector" and "let me fix
        // the selector": no model turn, no tokens, the same invocation pipeline.
        ("POST", "/tool") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let name = v["name"].as_str().unwrap_or_default().trim().to_string();
            if name.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"name"}"#).await;
            }
            let arguments = if v["args"].is_null() {
                json!({})
            } else {
                v["args"].clone()
            };
            let live = state.live.read().await.clone();
            let reply = match live
                .session
                .rpc()
                .tools()
                .execute(ToolsExecuteRequest {
                    name: name.clone(),
                    arguments,
                    tool_call_id: None,
                })
                .await
            {
                Ok(r) => {
                    json!({ "name": name, "result": serde_json::to_value(&r).unwrap_or(Value::Null) })
                }
                Err(e) => json!({ "name": name, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // The permission policy the runtime itself applies. `manual` asks the page
        // (what it already did), `allow-all` approves tool, path and URL requests
        // without asking, and `assisted` attaches a safety recommendation.
        ("GET", "/permission-mode") => {
            let live = state.live.read().await.clone();
            let r = live.session.rpc().permissions();
            let body = match r.get_mode().await {
                Ok(m) => json!({ "mode": mode_name(&m.mode) }),
                Err(e) => json!({ "mode": "unknown", "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        ("POST", "/permission-mode") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let want = v["mode"].as_str().unwrap_or("manual");
            let live = state.live.read().await.clone();
            let r = live
                .session
                .rpc()
                .permissions()
                .set_mode(PermissionsSetModeRequest {
                    mode: mode_of(want),
                    assisted_approval_model: None,
                    source: None,
                })
                .await;
            let reply = match r {
                Ok(m) => json!({ "mode": mode_name(&m.mode) }),
                Err(e) => json!({ "mode": "unknown", "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // Hand the runtime an objective and let it dispatch sub-agents. Refused
        // when the page switched delegating off, because that switch hides the
        // very tools the workers would be given.
        ("POST", "/fleet") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let mut prompt = v["prompt"].as_str().unwrap_or_default().trim().to_string();
            // No objective means "the thing I just asked": the composer cleared
            // itself the moment the command was picked, and the store is the only
            // side that still has the words.
            if prompt.is_empty() {
                let convo = state.convo().await;
                if let Some(store) = &state.store {
                    let msgs = store.messages(&convo);
                    prompt = msgs
                        .iter()
                        .rev()
                        .find(|m| m["role"] == "user")
                        .and_then(|m| m["body"].as_str())
                        .unwrap_or_default()
                        .to_string();
                }
            }
            if prompt.is_empty() {
                let out =
                    json!({ "started": false, "error": "nothing to run — ask something first" });
                return respond(&mut stream, 400, "application/json", &out.to_string()).await;
            }
            if !state.settings.lock().await.allow_agents {
                let out =
                    json!({ "started": false, "error": "delegating is off in runtime settings" });
                return respond(&mut stream, 409, "application/json", &out.to_string()).await;
            }
            let live = state.live.read().await.clone();
            // `FleetStartRequest` carries a crate-private field, so it is built
            // from `Default` rather than a struct literal.
            let mut req = FleetStartRequest::default();
            req.prompt = Some(prompt);
            req.wait = Some(false);
            let reply = match live.session.rpc().fleet().start(req).await {
                Ok(r) => json!({ "started": r.started }),
                Err(e) => json!({ "started": false, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // The agents this session is running. Experimental on the wire and
        // untyped (`Vec<Value>`), so it is passed through as the runtime sent it
        // rather than reshaped into a schema this build would have to invent.
        ("GET", "/tasks") => {
            let live = state.live.read().await.clone();
            let body = match live.session.rpc().tasks().list().await {
                Ok(r) => json!({ "tasks": r.tasks }),
                Err(e) => json!({ "tasks": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // The other kind of long-running work: OS processes this session started
        // through its own `terminal` tool and left running. `/tasks` is the
        // runtime's sub-agents; these are ours, and they are what a dev server or
        // a build looks like from here. Reading the list consumes nothing, so the
        // model's own `terminal read` still gets its unread output after a poll.
        ("GET", "/jobs") => {
            let body = json!({ "jobs": chat_tools::jobs() }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        // Killing a process waits on the OS, so it goes off the async task — the
        // same reason the `terminal` tool wraps every op in `spawn_blocking`.
        ("POST", "/jobs") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let Some(id) = v["id"].as_u64() else {
                return respond(&mut stream, 400, "application/json", r#"{"error":"id"}"#).await;
            };
            let reply = match tokio::task::spawn_blocking(move || chat_tools::job_stop(id)).await {
                Ok(Ok(message)) => json!({ "stopped": true, "id": id, "message": message }),
                Ok(Err(e)) => json!({ "stopped": false, "id": id, "error": e }),
                Err(e) => json!({ "stopped": false, "id": id, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // Agent-to-agent: a word to one of them mid-flight. Same gate as fleet,
        // because a message to a worker is delegation by another name.
        ("POST", "/agents") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let id = v["id"].as_str().unwrap_or_default().to_string();
            let message = v["text"].as_str().unwrap_or_default().to_string();
            if id.is_empty() || message.is_empty() {
                return respond(
                    &mut stream,
                    400,
                    "application/json",
                    r#"{"error":"id and text"}"#,
                )
                .await;
            }
            if !state.settings.lock().await.allow_agents {
                let out =
                    json!({ "sent": false, "error": "delegating is off in runtime settings" });
                return respond(&mut stream, 409, "application/json", &out.to_string()).await;
            }
            let live = state.live.read().await.clone();
            let reply = match live
                .session
                .rpc()
                .tasks()
                .send_message(TasksSendMessageRequest {
                    from_agent_id: None,
                    id,
                    message,
                })
                .await
            {
                Ok(_) => json!({ "sent": true }),
                Err(e) => json!({ "sent": false, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        ("POST", "/plan") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let content = v["content"].as_str().unwrap_or_default().to_string();
            let live = state.live.read().await.clone();
            let reply = match live
                .session
                .rpc()
                .plan()
                .update(PlanUpdateRequest { content })
                .await
            {
                Ok(()) => json!({ "saved": true }),
                Err(e) => json!({ "saved": false, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        ("POST", "/permission") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let id = v["id"].as_str().unwrap_or_default().to_string();
            let allow = v["allow"].as_bool().unwrap_or(false);
            let sent = match state.permission.pending.lock().await.remove(&id) {
                Some((tx, _)) => tx.send(allow).is_ok(),
                None => false,
            };
            let body = json!({ "answered": sent }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        // The approvals still waiting for an answer. A reload used to lose them:
        // the runtime kept waiting, the page had no card, and the turn sat there
        // until the five-minute timeout rejected it.
        //
        // `asks` is OUR store, so it only knows prompts this process asked. Two
        // more sets can exist in the runtime and were never asked about:
        // external tool calls it holds for a host to answer, and permission
        // prompts reconstructed from event history (the ones a suspended or
        // resumed session carries). They are reported under `runtime`, apart
        // from `asks`, so a card renderer that does not know them yet cannot
        // break on them.
        ("GET", "/pending") => {
            let asks: Vec<Value> = state
                .permission
                .pending
                .lock()
                .await
                .values()
                .map(|(_, card)| card.clone())
                .collect();
            let live = state.live.read().await.clone();
            let mut runtime = json!({ "external": [], "permissions": [] });
            let mut runtime_errors: Vec<String> = Vec::new();
            match live.session.rpc().tools().list_pending_requests().await {
                Ok(l) => {
                    runtime["external"] = json!(
                        l.items
                            .iter()
                            .map(|i| json!({
                                "request_id": i.request_id,
                                "tool_call_id": i.tool_call_id,
                                "tool": i.tool_name,
                                "arguments": i.arguments,
                                "agent_id": i.agent_id,
                            }))
                            .collect::<Vec<_>>()
                    );
                }
                Err(e) => runtime_errors.push(format!("tools.listPendingRequests: {e}")),
            }
            match live.session.rpc().permissions().pending_requests().await {
                Ok(l) => {
                    runtime["permissions"] = json!(
                        l.items
                            .iter()
                            .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                            .collect::<Vec<_>>()
                    );
                }
                Err(e) => runtime_errors.push(format!("permissions.pendingRequests: {e}")),
            }
            let mut out = json!({ "asks": asks, "runtime": runtime });
            if !runtime_errors.is_empty() {
                out["runtime"]["errors"] = json!(runtime_errors);
            }
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        // Answer an external tool call the runtime is holding. Our own prompts go
        // through `/permission` (a channel into `WebPermission`); this is the
        // other path, where the runtime wants a result rather than a yes/no.
        ("POST", "/pending") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let request_id = v["request_id"].as_str().unwrap_or_default();
            if request_id.is_empty() {
                return respond(
                    &mut stream,
                    400,
                    "application/json",
                    r#"{"error":"request_id"}"#,
                )
                .await;
            }
            let reply = HandlePendingToolCallRequest {
                request_id: RequestId::new(request_id),
                result: v.get("result").cloned(),
                error: v.get("error").and_then(|e| e.as_str()).map(String::from),
            };
            let live = state.live.read().await.clone();
            let out = match live
                .session
                .rpc()
                .tools()
                .handle_pending_tool_call(reply)
                .await
            {
                Ok(r) => json!({ "handled": r.success }),
                Err(e) => json!({ "handled": false, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        // Host-driven composer completions: ask the runtime which characters open
        // them. An empty answer is the whole answer — it is how the SDK says the
        // host (a local session, in our case) does not advertise completions, and
        // building ghost text on top of that would be UI over nothing.
        ("GET", "/completions") => {
            let live = state.live.read().await.clone();
            let out = match live
                .session
                .rpc()
                .completions()
                .get_trigger_characters()
                .await
            {
                Ok(r) => json!({
                    "trigger_characters": r.trigger_characters,
                    "supported": !r.trigger_characters.is_empty(),
                }),
                Err(e) => json!({
                    "trigger_characters": [],
                    "supported": false,
                    "error": e.to_string(),
                }),
            };
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        // The runtime's name for this session, and a way to set it. The page's
        // title comes from `session.title_changed`, which the model writes; the
        // runtime's name is what `sessions.list` keys on and what a resume will
        // look up, so the two are worth being able to compare.
        ("GET", "/name") => {
            let live = state.live.read().await.clone();
            let out = match live.session.rpc().name().get().await {
                Ok(r) => json!({ "name": r.name }),
                Err(e) => json!({ "name": null, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        ("POST", "/name") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let name = v["name"].as_str().unwrap_or_default().trim().to_string();
            if name.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"name"}"#).await;
            }
            let live = state.live.read().await.clone();
            let out = match live
                .session
                .rpc()
                .name()
                .set(NameSetRequest { name: name.clone() })
                .await
            {
                Ok(()) => json!({ "saved": true, "name": name }),
                Err(e) => json!({ "saved": false, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        // What the runtime itself holds. `GET /sessions` is our database, so a
        // thread the runtime still has — one this machine's store lost, or one
        // another client made — is invisible there. This is also the list a
        // resume works from, which is why it comes before resume, not after.
        ("GET", "/runtime/sessions") => {
            let live = state.live.read().await.clone();
            let out = match live.client.rpc().sessions().list().await {
                Ok(l) => json!({
                    "sessions": serde_json::to_value(&l).unwrap_or(Value::Null),
                }),
                Err(e) => json!({ "sessions": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        // Restore the runtime's own session for this conversation, instead of
        // replaying our transcript onto a blank one. The transcript stays (the
        // page still draws it); what changes is that the model gets its context
        // back — plan file, rewind points, tool history included.
        ("POST", "/resume") => {
            // The conversation is whatever `POST /sessions/{id}` last claimed —
            // that route already owns switching, so this one only restores.
            let convo = state.convo().await;
            let want = format!("webrain-{convo}");
            let live = state.live.read().await.clone();
            let id = live
                .client
                .rpc()
                .sessions()
                .list()
                .await
                .ok()
                .and_then(|l| serde_json::to_value(&l).ok())
                .and_then(|v| v.get("sessions").and_then(Value::as_array).cloned())
                .and_then(|rows| {
                    rows.into_iter().find_map(|s| {
                        (s["name"].as_str() == Some(want.as_str()))
                            .then(|| s["sessionId"].as_str().map(str::to_string))
                            .flatten()
                    })
                });
            let Some(id) = id else {
                let out = json!({
                    "resumed": false,
                    "error": "the runtime has no session for this conversation",
                });
                return respond(&mut stream, 200, "application/json", &out.to_string()).await;
            };
            let provider = state.provider.lock().await.clone();
            let settings = state.settings.lock().await.clone();
            let out = match spawn_live(
                &state.args,
                &provider,
                state.permission.clone(),
                state.input.clone(),
                state.bus.clone(),
                state.store.clone(),
                convo,
                &settings,
                Some(SessionId::new(id)),
            )
            .await
            {
                Ok(fresh) => {
                    let old = std::mem::replace(&mut *state.live.write().await, Arc::new(fresh));
                    // Stop the fan-out before the runtime it reads from.
                    old.reader.abort();
                    drop(old);
                    json!({ "resumed": true, "name": want })
                }
                Err(e) => json!({ "resumed": false, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        // The answer to an `ask_user` question. Same shape as `/permission`, but
        // the payload is free text plus whether it was typed or picked.
        ("POST", "/answer") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let id = v["id"].as_str().unwrap_or_default().to_string();
            let answer = v["answer"].as_str().unwrap_or_default().to_string();
            let freeform = v["freeform"].as_bool().unwrap_or(true);
            let sent = match state.input.pending.lock().await.remove(&id) {
                Some(tx) => tx.send((answer, freeform)).is_ok(),
                None => false,
            };
            let out = json!({ "answered": sent }).to_string();
            respond(&mut stream, 200, "application/json", &out).await
        }
        // What is waiting behind the running turn, and what is steering it. The
        // page can queue a prompt mid-turn, so it has to be able to show the
        // queue — otherwise "I sent it" and "it ran" look the same.
        ("GET", "/queue") => {
            let live = state.live.read().await.clone();
            let body = match live.session.rpc().queue().pending_items().await {
                Ok(q) => json!({
                    "items": q.items.iter().map(|i| json!({
                        "id": i.id,
                        "text": i.display_text,
                        "kind": i.kind,
                    })).collect::<Vec<_>>(),
                    "steering": q.steering_messages,
                    "withdrawable": q.withdrawable_turn_message_id,
                }),
                Err(e) => json!({ "items": [], "steering": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // Promote, edit or drop one queued prompt. `sendNow` runs it now instead
        // of after the turn in flight; the other two exist because a prompt you
        // can send but not correct is a prompt you retype in the composer.
        ("POST", "/queue") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let id = v["id"].as_str().unwrap_or_default().to_string();
            if id.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"id"}"#).await;
            }
            let live = state.live.read().await.clone();
            let q = live.session.rpc().queue();
            let reply = if v["remove"].as_bool().unwrap_or(false) {
                match q.remove_at(QueueRemoveAtRequest { id: id.clone() }).await {
                    Ok(r) => json!({ "id": id, "removed": r.removed }),
                    Err(e) => json!({ "id": id, "error": e.to_string() }),
                }
            } else if let Some(text) = v["text"].as_str() {
                match q
                    .update_text(QueueUpdateTextRequest {
                        display_prompt: None,
                        id: id.clone(),
                        prompt: text.to_string(),
                    })
                    .await
                {
                    Ok(r) => json!({ "id": id, "updated": r.updated }),
                    Err(e) => json!({ "id": id, "error": e.to_string() }),
                }
            } else {
                match q.send_now(QueueSendNowRequest { id: id.clone() }).await {
                    Ok(r) => json!({ "id": id, "steered": r.steered }),
                    Err(e) => json!({ "id": id, "error": e.to_string() }),
                }
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // Every user turn the session could go back to. `listRewindPoints`
        // answers with a reason instead of points while a turn is in flight
        // (`session-busy`), so that reason is passed on rather than shown as an
        // empty list. `files` says whether file restore is available at all —
        // without it "undo" would promise the workspace back and only get the
        // conversation.
        ("GET", "/rewind") => {
            let live = state.live.read().await.clone();
            let body = match live.session.rpc().history().list_rewind_points().await {
                Ok(r) => json!({
                    "files": r.file_change_tracking_enabled,
                    "unavailable": r.unavailable_reason,
                    "points": r.points.iter().map(|p| json!({
                        "event": p.event_id,
                        "text": p.user_message,
                        "ts": p.timestamp,
                        "files": p.file_count,
                        "added": p.lines_added,
                        "removed": p.lines_removed,
                    })).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "points": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // Undo a conversation, and optionally the files it touched. `preview`
        // runs first for a reason: a rewind that restores files writes to the
        // workspace, and the list of what it would overwrite has to be readable
        // before that happens. `files:false` leaves the workspace alone.
        ("POST", "/rewind") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let event_id = v["event"].as_str().unwrap_or_default().to_string();
            if event_id.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"event"}"#).await;
            }
            let live = state.live.read().await.clone();
            let reply = if v["preview"].as_bool().unwrap_or(false) {
                match live
                    .session
                    .rpc()
                    .history()
                    .preview_rewind(HistoryPreviewRewindRequest { event_id })
                    .await
                {
                    Ok(r) => json!({
                        "preview": true,
                        "available": r.available,
                        "reason": r.reason,
                        "files": r.files.iter().map(|f| json!({
                            "path": f.path,
                            "change": f.change_type,
                            "added": f.lines_added,
                            "removed": f.lines_removed,
                        })).collect::<Vec<_>>(),
                    }),
                    Err(e) => json!({ "preview": true, "error": e.to_string() }),
                }
            } else {
                let mode = if v["files"].as_bool().unwrap_or(false) {
                    HistoryRewindMode::ConversationAndFiles
                } else {
                    HistoryRewindMode::Conversation
                };
                match live
                    .session
                    .rpc()
                    .history()
                    .rewind(HistoryRewindRequest { event_id, mode })
                    .await
                {
                    Ok(r) => json!({
                        "outcome": r.outcome,
                        "removed": r.events_removed,
                        "restored": r.restored_files,
                        "skipped": r.skipped_files.iter().map(|s| json!({
                            "path": s.path,
                            "reason": s.reason,
                        })).collect::<Vec<_>>(),
                        "error": r.error,
                    }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // The runtime's session mode. Until now a mode was only a sentence
        // pasted onto the front of the prompt (`with_mode`); this is the switch
        // the agent actually reads, and `set` reports when honouring it needs a
        // model switch or a compaction.
        ("GET", "/mode") => {
            let live = state.live.read().await.clone();
            let body = match live.session.rpc().mode().get().await {
                Ok(m) => json!({ "mode": mode_word(&m) }),
                Err(e) => json!({ "mode": "unknown", "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        ("POST", "/mode") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let want = v["mode"].as_str().unwrap_or("interactive");
            // A crate-private field, so `Default` then assign.
            let mut req = ModeSetRequest::default();
            req.mode = session_mode_of(want);
            let live = state.live.read().await.clone();
            let reply = match live.session.rpc().mode().set(req).await {
                Ok(r) => {
                    json!({ "mode": want, "message": r.message, "deferred": r.defer_implementation })
                }
                Err(e) => json!({ "mode": "unknown", "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // What the model was told before the user's words, in merge order. The
        // content is left out on purpose: it is raw instruction files, and the
        // page only needs to name them.
        ("GET", "/instructions") => {
            let live = state.live.read().await.clone();
            let body = match live.session.rpc().instructions().get_sources().await {
                Ok(r) => json!({
                    "sources": r.sources.iter().map(|s| json!({
                        "label": s.label,
                        "path": s.source_path,
                        "kind": s.r#type,
                        "apply": s.apply_to,
                        "disabled": s.default_disabled,
                        "about": s.description,
                    })).collect::<Vec<_>>(),
                }),
                Err(e) => json!({ "sources": [], "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // Account credits. Both are Copilot-plan reads and answer null on BYOK,
        // which is the honest answer for a key you pay for yourself.
        ("GET", "/quota") => {
            let live = state.live.read().await.clone();
            let quota = live.session.rpc().quota().get().await.ok();
            let prediction = live.session.rpc().limit_prediction().predict().await.ok();
            let body = json!({
                "quota": quota.map(|q| serde_json::to_value(&q).unwrap_or(Value::Null)),
                "prediction": prediction.map(|p| serde_json::to_value(&p).unwrap_or(Value::Null)),
            });
            respond(&mut stream, 200, "application/json", &body.to_string()).await
        }
        // One question answered against the conversation context, with no tools
        // and no history: "summarise this result" must not become a turn the
        // agent can act in.
        ("POST", "/ephemeral") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let question = v["question"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string();
            if question.is_empty() {
                return respond(
                    &mut stream,
                    400,
                    "application/json",
                    r#"{"error":"question"}"#,
                )
                .await;
            }
            // A crate-private field, so `Default` then assign.
            let mut req = UIEphemeralQueryRequest::default();
            req.question = question;
            let live = state.live.read().await.clone();
            let reply = match live.session.rpc().ui().ephemeral_query(req).await {
                Ok(r) => json!({ "answer": r.answer }),
                Err(e) => json!({ "answer": "", "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        // The session menu: what was said before, and the current id so the page
        // can mark the live conversation. `busy` is here because a reloaded page
        // has no other way to learn that a turn is still running on the server —
        // without it the page looks idle while the agent keeps working.
        ("GET", "/sessions") => {
            let mut list = state.store.as_ref().map(|s| s.convos()).unwrap_or_default();
            let current = state.convo().await;
            // A conversation with no messages yet is not in the store, so it had
            // no row — and the rail marks rows by id, which meant nothing was
            // marked while the page sat in exactly that conversation. Pressing
            // "new chat" and finding no active tab is this, not a click that
            // failed to register.
            if !list
                .iter()
                .any(|c| c["id"].as_str() == Some(current.as_str()))
            {
                list.insert(
                    0,
                    json!({ "id": current, "title": "(new conversation)", "ts": "", "n": 0 }),
                );
            }
            // The runtime's own answer beats our `AtomicBool`: a background
            // continuation runs with nothing of ours flagging it, and a turn
            // killed by a page reload leaves ours stuck on.
            let processing = state
                .live
                .read()
                .await
                .session
                .rpc()
                .metadata()
                .is_processing()
                .await
                .map(|r| r.processing)
                .unwrap_or_else(|_| state.busy.load(Ordering::SeqCst));
            let body = json!({
                "current": state.convo().await,
                "sessions": list,
                "busy": state.busy.load(Ordering::SeqCst),
                "processing": processing,
            })
            .to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        // A new conversation, minted here so the id exists before the first word
        // is said and the page can put it in the address bar. The next turn
        // reseeds, which is what drops the previous conversation's context.
        //
        // A conversation nobody has written to is not worth an id: pressing "new
        // chat" twice used to mint a second one, leave the first as an empty row
        // in the menu and make the browser store a per-conversation setup key for
        // it. Reuse the empty one and mint only when there is something to leave.
        ("POST", "/sessions") => {
            let current = state.convo().await;
            let unused = state
                .store
                .as_ref()
                .is_some_and(|s| s.messages(&current).is_empty());
            if unused && !state.busy.load(Ordering::SeqCst) {
                let body = json!({ "current": current, "reused": true }).to_string();
                return respond(&mut stream, 200, "application/json", &body).await;
            }
            let id = format!(
                "c{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            );
            *state.convo.lock().await = id.clone();
            state.seed.store(true, Ordering::SeqCst);
            let body = json!({ "current": id }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        // Reading a conversation is a read. It used to also make that
        // conversation the live one, which the page calls from `seedLive` on
        // every load — so a second tab, or a reload racing a slow list, stole the
        // live conversation from the tab that owned it and two pages ping-ponged
        // `current` between them. The menu then looked like it had stopped
        // taking clicks until a reload. Claiming is its own route, below.
        ("GET", p) if p.starts_with("/sessions/") => {
            let id = p.trim_start_matches("/sessions/");
            let out = convo_payload(&state, id).to_string();
            respond_gz(&mut stream, &head, 200, "application/json", &out).await
        }
        // What a link is worth showing before it is clicked. `POST` so the url
        // needs no query-string decoding.
        ("POST", "/preview") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let url = v["url"].as_str().unwrap_or_default().trim().to_string();
            if url.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"url"}"#).await;
            }
            let out = link_preview(&url).await.to_string();
            respond(&mut stream, 200, "application/json", &out).await
        }
        // The page saves what it drew. Whole-array replace rather than a row at a
        // time: the page owns the row shape, and a second copy of it here would
        // drift the day a row gains a field.
        ("POST", "/transcript") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let rows = v["rows"].clone();
            // The tab says which conversation it drew. Not `state.convo`: two
            // tabs can be on two conversations, and the server's own id follows
            // whichever one was opened last — a debounced save from the other tab
            // would land on the wrong transcript.
            let convo = match v["convo"].as_str().filter(|c| !c.is_empty()) {
                Some(c) => c.to_string(),
                None => state.convo().await,
            };
            let saved = state
                .store
                .as_ref()
                .is_some_and(|s| s.save_rows(&convo, &rows.to_string()));
            let out = json!({ "saved": saved, "convo": convo }).to_string();
            respond(&mut stream, 200, "application/json", &out).await
        }
        // The instruments of a finished turn, handed over by the page that
        // measured them. A conversation reopened from history used to show a
        // column of `—` and "no turns yet" because nothing kept these.
        ("POST", "/ledger") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let n = v["n"].as_u64().unwrap_or(0) as u32;
            let convo = state.convo().await;
            let saved = n > 0
                && state
                    .store
                    .as_ref()
                    .is_some_and(|s| s.record_turn(&convo, n, &v["turn"].to_string()));
            let out = json!({ "saved": saved }).to_string();
            respond(&mut stream, 200, "application/json", &out).await
        }
        // Claim a conversation: from here on, a new turn belongs to it and the
        // next turn restarts the runtime on it (see `reseed`). The rail sends
        // this when you pick a thread, and it is deliberately not a GET.
        ("POST", p) if p.starts_with("/sessions/") => {
            let id = p.trim_start_matches("/sessions/").to_string();
            *state.convo.lock().await = id.clone();
            state.seed.store(true, Ordering::SeqCst);
            // The claim answers with the transcript as well. Picking a thread used
            // to be two round trips — GET for the rows, POST to claim — and the GET
            // is the expensive one (measured 354–663 KB of JSON for a scrape), so
            // folding it in halves both the requests and the wait. `?light=1` is a
            // caller that already holds those rows (a revisit) and wants only the
            // claim, which is a `state` write either way.
            if param(query, "light").is_some() {
                let out = json!({ "current": id }).to_string();
                return respond(&mut stream, 200, "application/json", &out).await;
            }
            let out = convo_payload(&state, &id).to_string();
            respond_gz(&mut stream, &head, 200, "application/json", &out).await
        }
        ("PATCH", p) if p.starts_with("/sessions/") => {
            let id = p.trim_start_matches("/sessions/");
            let title = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null)["title"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let renamed = state.store.as_ref().is_some_and(|s| s.rename(id, &title));
            let out = json!({ "renamed": renamed }).to_string();
            let code = if renamed { 200 } else { 400 };
            respond(&mut stream, code, "application/json", &out).await
        }
        ("DELETE", p) if p.starts_with("/sessions/") => {
            let id = p.trim_start_matches("/sessions/");
            let deleted = state.store.as_ref().is_some_and(|s| s.forget(id));
            let out = json!({ "deleted": deleted }).to_string();
            respond(&mut stream, 200, "application/json", &out).await
        }
        // Branching, at the level the runtime can actually offer: a conversation
        // is a transcript, so a fork copies a prefix into a new one. The model's
        // own context stays linear, and the page says so rather than implying a
        // rewind.
        ("POST", "/fork") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let current = state.convo().await;
            let from = v["convo"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(current.as_str())
                .to_string();
            let upto = v["upto"].as_u64().unwrap_or(0) as usize;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let into = format!("fork-{stamp}");
            let forked = state
                .store
                .as_ref()
                .is_some_and(|s| s.fork(&from, upto, &into));
            let out = json!({ "convo": into, "upto": upto, "forked": forked }).to_string();
            let code = if forked { 200 } else { 400 };
            respond(&mut stream, code, "application/json", &out).await
        }
        // Every provider this build can talk to, configured or not: the modal
        // shows the capability and then which rows actually hold a key, instead
        // of pretending the only provider that exists is the one with a key set.
        //
        // A key can come from the environment or from the vault; `key_present`
        // and `key_vault` say which, and nothing here ever echoes one.
        ("GET", "/providers") => {
            let live = state.live.read().await.clone();
            let env_key = std::env::var("WEBRAIN_CHAT_API_KEY").is_ok();
            let mut list: Vec<Value> = Vec::new();
            // The session's own endpoint is a row of its own only when the table
            // does not claim it — a custom base url. Otherwise the table row is
            // shown once, carrying the model the live session actually runs.
            if row_name_for_base(&live.base_url).is_empty() {
                list.push(json!({
                    "id": "env",
                    "base_url": live.base_url,
                    "model": live.model(),
                    "key_env": "WEBRAIN_CHAT_API_KEY",
                    "type": "openai",
                    "key_present": env_key,
                    "key_vault": false,
                    "active": true,
                    "saved": false,
                    "description": "the endpoint this session was started against",
                    "context": Value::Null,
                    "docs": Value::Null,
                    "off_peak": Value::Null,
                }));
            }
            for row in webrain_core::byok::CHAT_PROVIDERS.iter() {
                // One entry per row: `row.0` is the alias list, so the first
                // alias is the row's name and the rest are alternatives.
                let id = row.0[0].trim_end_matches("_API_KEY").to_ascii_lowercase();
                let active = row.1 == live.base_url;
                let (description, context, docs) = provider_info(&id);
                list.push(json!({
                    "id": id,
                    "base_url": row.1,
                    "model": if active { live.model() } else { row.2.to_string() },
                    "key_env": row.0[0],
                    "type": if id == "anthropic" { "anthropic" } else { "openai" },
                    "key_present": env_key || row.0.iter().any(|n| std::env::var(n).is_ok()),
                    // The vault is the second source, and the one the page can
                    // fill in: a key pasted there needs no restart of anything.
                    "key_vault": vault_key(&id).is_some(),
                    "active": active,
                    "saved": false,
                    "description": description,
                    "context": context,
                    "docs": docs,
                    "off_peak": off_peak(&id),
                }));
            }
            let known: Vec<String> = list
                .iter()
                .filter_map(|v| v["id"].as_str().map(str::to_string))
                .collect();
            for p in state
                .store
                .as_ref()
                .map(|s| s.providers())
                .unwrap_or_default()
            {
                let id = p["id"].as_str().unwrap_or_default();
                let env = p["key_env"].as_str().unwrap_or_default();
                if id.is_empty() || known.iter().any(|k| k == id) {
                    continue;
                }
                let base = p["base_url"].as_str().unwrap_or_default();
                list.push(json!({
                    "id": id,
                    "base_url": base,
                    "model": p["model"],
                    "key_env": env,
                    "type": "openai",
                    "key_present": env_key || (!env.is_empty() && std::env::var(env).is_ok()),
                    "key_vault": vault_key(id).is_some(),
                    "active": base == live.base_url,
                    "saved": true,
                    "description": "saved on this machine — any OpenAI-compatible endpoint",
                    "context": Value::Null,
                    "docs": Value::Null,
                    "off_peak": Value::Null,
                }));
            }
            respond(
                &mut stream,
                200,
                "application/json",
                &json!({ "providers": list }).to_string(),
            )
            .await
        }
        // A provider's live catalogue, for the modal's tab body. Cached for five
        // minutes so switching tabs costs nothing and the vendor is not asked
        // twice per click.
        ("GET", "/models") => {
            let want = param(query, "provider").unwrap_or_default();
            let Some((base, key, typ)) = provider_target(&state, &want) else {
                let out = json!({ "error": "unknown provider", "models": [] }).to_string();
                return respond(&mut stream, 400, "application/json", &out).await;
            };
            let mut err = Value::Null;
            let models = match cached_models(&base) {
                Some(v) => v,
                None => match list_models(&base, key.as_deref(), typ).await {
                    Ok(v) => {
                        remember_models(&base, &v);
                        v
                    }
                    Err(e) => {
                        err = json!(e);
                        Vec::new()
                    }
                },
            };
            let out = json!({ "provider": want, "models": models, "error": err }).to_string();
            respond(&mut stream, 200, "application/json", &out).await
        }
        // The one honest answer to "what does this model cost": the provider's own
        // published price for it, or nothing. It reuses the catalogue cache, so a
        // page that asks on every model change costs at most one vendor call.
        ("GET", "/price") => {
            let provider = param(query, "provider").unwrap_or_default();
            let model = param(query, "model").unwrap_or_default();
            let Some((base, key, typ)) = provider_target(&state, &provider) else {
                return respond(
                    &mut stream,
                    400,
                    "application/json",
                    r#"{"error":"unknown provider"}"#,
                )
                .await;
            };
            let models = match cached_models(&base) {
                Some(v) => v,
                None => match list_models(&base, key.as_deref(), typ).await {
                    Ok(v) => {
                        remember_models(&base, &v);
                        v
                    }
                    Err(e) => {
                        let out = json!({ "error": e }).to_string();
                        return respond(&mut stream, 200, "application/json", &out).await;
                    }
                },
            };
            let priced = models
                .iter()
                .find(|m| m["id"].as_str() == Some(model.as_str()))
                .map(|m| m["pricing"].clone())
                .unwrap_or(Value::Null);
            // A vendor that publishes nothing in its catalogue may still publish
            // its rates in its docs (`doc_price`); every other vendor's rows were
            // already filled from the joined catalogue by `list_models`.
            let pricing = if priced.is_null() {
                doc_price(&base, &model)
            } else {
                priced
            };
            let out = json!({ "provider": provider, "model": model, "pricing": pricing });
            respond(&mut stream, 200, "application/json", &out.to_string()).await
        }
        ("POST", "/providers") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let id = v["id"].as_str().unwrap_or_default().trim().to_string();
            let base = v["base_url"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string();
            let model = v["model"].as_str().unwrap_or_default().trim().to_string();
            let key_env = v["key_env"].as_str().unwrap_or_default().trim().to_string();
            let ok = valid_provider(&id, &base, &model, &key_env);
            if !ok {
                let out = json!({ "error": "id/base_url/model/key_env" }).to_string();
                return respond(&mut stream, 400, "application/json", &out).await;
            }
            let added = state
                .store
                .as_ref()
                .is_some_and(|s| s.add_provider(&id, &base, &model, &key_env));
            let out = json!({ "added": added }).to_string();
            let code = if added { 200 } else { 500 };
            respond(&mut stream, code, "application/json", &out).await
        }
        // A provider key, typed into the page instead of exported somewhere
        // first. It goes to the encrypted vault, not to the chat database and not
        // to the environment, and it is never sent back out.
        ("POST", "/keys") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let id = v["provider"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let key = v["key"].as_str().unwrap_or_default().trim();
            // The vault is shared with every site login webrain holds, so a write
            // has to name a provider this server knows and a plausible secret —
            // otherwise this route is a way to stuff arbitrary entries into it.
            let known = webrain_core::byok::CHAT_PROVIDERS.iter().any(|r| {
                r.0[0]
                    .trim_end_matches("_API_KEY")
                    .eq_ignore_ascii_case(&id)
            }) || state
                .store
                .as_ref()
                .is_some_and(|s| s.providers().iter().any(|p| p["id"].as_str() == Some(&id)));
            if !known || key.is_empty() || key.len() > 400 {
                let out = json!({ "error": "unknown provider or empty key" }).to_string();
                return respond(&mut stream, 400, "application/json", &out).await;
            }
            match webrain_core::vault::set(KEY_SERVICE, &id, &id, key, None) {
                Ok(()) => {
                    // The catalogue was fetched with whatever key existed before,
                    // so the next tab switch must ask again.
                    forget_models();
                    let out = json!({ "saved": true, "provider": id }).to_string();
                    respond(&mut stream, 200, "application/json", &out).await
                }
                Err(e) => {
                    let out = json!({ "saved": false, "error": e.to_string() }).to_string();
                    respond(&mut stream, 500, "application/json", &out).await
                }
            }
        }
        // Switch model and thinking effort, or the provider itself.
        //
        // Two different operations, because the runtime's are: a model on the
        // endpoint this session already talks to is `set_model`, which keeps the
        // context; another endpoint means a new session, which does not.
        ("POST", "/switch") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let want = v["provider"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string();
            let model = v["model"].as_str().unwrap_or_default().trim().to_string();
            let effort = v["effort"].as_str().unwrap_or_default().trim().to_string();
            if state.busy.load(Ordering::SeqCst) {
                let out = json!({ "error": "busy" }).to_string();
                return respond(&mut stream, 409, "application/json", &out).await;
            }
            let Some((base, key, typ)) = provider_target(&state, &want) else {
                let out = json!({ "error": "unknown provider" }).to_string();
                return respond(&mut stream, 400, "application/json", &out).await;
            };
            let live = state.live.read().await.clone();

            if base == live.base_url {
                let model = if model.is_empty() {
                    live.model()
                } else {
                    model
                };
                let mut opts = SetModelOptions::default();
                // `off` is the runtime's own name for "send nothing" — leaving
                // the field out is the only honest way to say it, since the
                // runtime, not this page, decides whether the model thinks.
                if !effort.is_empty() && effort != "off" {
                    opts = opts.with_reasoning_effort(effort.clone());
                }
                let reply = match live.session.set_model(&model, Some(opts)).await {
                    Ok(()) => {
                        live.set_model(&model);
                        if let Ok(mut e) = live.effort.lock() {
                            *e = effort.clone();
                        }
                        json!({
                            "switched": true, "restarted": false,
                            "provider": want, "model": model, "effort": effort,
                        })
                    }
                    Err(e) => json!({ "switched": false, "error": e.to_string() }),
                };
                return respond(&mut stream, 200, "application/json", &reply.to_string()).await;
            }

            // Another endpoint: the provider is fixed when a session is created,
            // so this replaces the session. The transcript stays on screen and in
            // history; the model's own context starts empty, and the page says so.
            let model = if model.is_empty() {
                webrain_core::byok::CHAT_PROVIDERS
                    .iter()
                    .find(|r| r.1 == base)
                    .map(|r| r.2.to_string())
                    .unwrap_or_default()
            } else {
                model
            };
            if model.is_empty() {
                let out = json!({ "error": "model" }).to_string();
                return respond(&mut stream, 400, "application/json", &out).await;
            }
            let provider = ChatProvider::new(typ, base, model.clone(), None, key);
            let settings = state.settings.lock().await.clone();
            match spawn_live(
                &state.args,
                &provider,
                state.permission.clone(),
                state.input.clone(),
                state.bus.clone(),
                state.store.clone(),
                state.convo().await,
                &settings,
                None,
            )
            .await
            {
                Ok(fresh) => {
                    let old = std::mem::replace(&mut *state.live.write().await, Arc::new(fresh));
                    // Stop the fan-out before the runtime it reads from.
                    old.reader.abort();
                    drop(old);
                    // A later conversation switch restarts from the provider in
                    // use now, not the one this process booted with.
                    *state.provider.lock().await = provider.clone();
                    let reply = json!({
                        "switched": true, "restarted": true,
                        "provider": row_name_for_base(&provider.base_url),
                        "model": model, "effort": effort,
                    });
                    respond(&mut stream, 200, "application/json", &reply.to_string()).await
                }
                Err(e) => {
                    let out = json!({ "switched": false, "error": e.to_string() }).to_string();
                    respond(&mut stream, 500, "application/json", &out).await
                }
            }
        }
        // The composer's menus. Every row carries its own kind and hint, so one
        // menu can hold commands, skills and tools without the page knowing how
        // many of each there are. Everything here is read from the live registry:
        // a tool this build does not expose must not appear in the list.
        ("GET", "/catalog") => {
            let mut items: Vec<Value> = vec![
                json!({"name": "clear", "kind": "cmd", "hint": "empty the transcript"}),
                json!({"name": "tools", "kind": "cmd", "hint": "list every tool this build has"}),
                json!({"name": "undo", "kind": "cmd", "hint": "put back the last file the agent changed"}),
                json!({"name": "rewind", "kind": "cmd", "hint": "walk the conversation back to an earlier turn, files too"}),
                json!({"name": "instructions", "kind": "cmd", "hint": "the instruction files the model was given, in merge order"}),
                json!({"name": "quota", "kind": "cmd", "hint": "account credits, when the provider has a plan"}),
                json!({"name": "todos", "kind": "cmd", "hint": "read the agent's todo list"}),
                json!({"name": "jobs", "kind": "cmd", "hint": "processes it started and left running — output and exit codes"}),
                json!({"name": "agents", "kind": "cmd", "hint": "list the agents this session is running"}),
                json!({"name": "fleet", "kind": "cmd", "hint": "run the last thing you asked as a fleet"}),
                json!({"name": "context", "kind": "cmd", "hint": "how full the context window is, and where it goes"}),
                json!({"name": "mcp", "kind": "cmd", "hint": "the MCP server this session attached to, and its tools"}),
                json!({"name": "skills", "kind": "cmd", "hint": "installed skills and which ones the agent used"}),
            ];
            for name in skill_names() {
                // A reference chapter is a `.md` with no frontmatter `name:`, and
                // `skills::list` names it after the folder holding it — so without
                // this check the menu offers `references` and `workflows`, which
                // are folders, not skills, and which resolve to one arbitrary file.
                let Some(hint) = skill_hint(&name) else {
                    continue;
                };
                items.push(json!({"name": name, "kind": "skill", "hint": hint}));
            }
            for (name, hint) in webrain_mcp::tool_briefs() {
                items.push(json!({"name": name, "kind": "tool", "hint": hint}));
            }
            // This binary's own tools. They are not runtime built-ins —
            // `tool_filter` sends them as `custom:` — and they belong in the
            // same checklist as the MCP ones, or the dialog would show a group
            // it cannot tick.
            for name in chat_tools::NAMES {
                items.push(json!({
                    "name": name, "kind": "tool",
                    "hint": "installed by this binary, path-fenced",
                }));
            }
            // The four the session actually allows. `task` and `task_complete`
            // were missing here, so delegation had no checkbox at all and the
            // only place to switch it was the Runtime tab.
            let mut builtins = vec!["ask_user", "skill", "task", "task_complete"];
            // `tool_filter` serves these with delegation on, so the checklist
            // has to agree or the dialog shows a group it cannot tick.
            let agents_on = state.settings.lock().await.allow_agents;
            if agents_on {
                builtins.extend(["read_agent", "write_agent", "list_agents"]);
            }
            for name in builtins {
                items.push(
                    json!({"name": name, "kind": "builtin", "hint": "installed by the runtime"}),
                );
            }
            // The runtime's own slash commands. `session.commands.list` is the
            // namespace that lists them — the page's menu was ours alone, so
            // `/compact` and friends existed and were never offered. Marked
            // `runtime` so `pick` can tell one from a command this page handles
            // itself without keeping a second copy of the names.
            let ours: std::collections::HashSet<String> = items
                .iter()
                .filter_map(|i| i["name"].as_str().map(str::to_owned))
                .collect();
            let live = state.live.read().await.clone();
            if let Ok(list) = live.session.rpc().commands().list().await {
                for c in list.commands {
                    if ours.contains(&c.name) {
                        continue;
                    }
                    let exp = if c.experimental.unwrap_or(false) {
                        "experimental · "
                    } else {
                        ""
                    };
                    items.push(json!({
                        "name": c.name,
                        "kind": "cmd",
                        "runtime": true,
                        "cmdKind": serde_json::to_value(&c.kind).unwrap_or(Value::Null),
                        "hint": format!("{exp}{}", c.description),
                    }));
                }
            }
            let body = json!({ "items": items }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        // Run one of the runtime's slash commands. `commands.execute` refused all
        // 29 of them ("No client found for command") — that path is for a host
        // that registered handlers — so this is `invoke`, which returns what to
        // show: text, a message, or a prompt the host should submit.
        ("POST", "/command") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let name = v["name"].as_str().unwrap_or_default().trim().to_string();
            if name.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"name"}"#).await;
            }
            let input = v["input"].as_str().unwrap_or_default().to_string();
            let mut req = CommandsInvokeRequest::default();
            req.name = name.clone();
            req.input = (!input.is_empty()).then_some(input);
            let live = state.live.read().await.clone();
            let reply = match live.session.rpc().commands().invoke(req).await {
                Ok(SlashCommandInvocationResult::Text(t)) => {
                    json!({ "name": name, "text": t.text, "markdown": t.markdown.unwrap_or(false) })
                }
                Ok(SlashCommandInvocationResult::Completed(c)) => {
                    json!({ "name": name, "text": c.message.unwrap_or_default() })
                }
                // The command wants a turn run with a prompt it wrote. Handing the
                // prompt back lets the page put it in the composer rather than
                // starting a turn the reader never chose.
                Ok(SlashCommandInvocationResult::AgentPrompt(p)) => json!({
                    "name": name, "prompt": p.display_prompt, "notice": p.notice,
                }),
                // Model switches, subcommand pickers and dialogs each want a
                // surface this page does not have. Say so instead of pretending.
                Ok(_) => json!({
                    "name": name,
                    "text": "ran — it wants a surface this page does not have yet",
                }),
                Err(e) => json!({ "name": name, "error": e.to_string() }),
            };
            respond(&mut stream, 200, "application/json", &reply.to_string()).await
        }
        ("GET", "/search") => {
            let q = param(query, "q").unwrap_or_default();
            let hits = state
                .store
                .as_ref()
                .map(|s| s.search(&q))
                .unwrap_or_default();
            let body = json!({ "q": q, "hits": hits }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        // Uploads go through here: raw bytes in the body, the (percent-encoded)
        // filename in `x-name`. Nothing about the client's name decides where the
        // bytes land, and only a small set of types is accepted.
        ("POST", "/attach") => {
            let raw = header(&head, "x-name").map(decode).unwrap_or_default();
            let Some(name) = safe_name(&raw) else {
                return respond(&mut stream, 415, "application/json", r#"{"error":"type"}"#).await;
            };
            let dir = attach_dir().join(state.convo().await);
            if std::fs::create_dir_all(&dir).is_err() {
                return respond(&mut stream, 500, "application/json", r#"{"error":"dir"}"#).await;
            }
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let path = dir.join(format!("{stamp}-{name}"));
            if std::fs::write(&path, &body).is_err() {
                return respond(&mut stream, 500, "application/json", r#"{"error":"write"}"#).await;
            }
            let out = json!({
                "path": path.display().to_string(),
                "name": name,
                "bytes": body.len(),
            })
            .to_string();
            respond(&mut stream, 200, "application/json", &out).await
        }
        // The same bytes, back out: what you attached has to stay visible in the
        // transcript after a reload, and be downloadable without hunting for a
        // temp path. Two segments only — the conversation and the stored
        // basename — and neither may hold a dot or a slash, so the join can only
        // ever name a file the upload route itself wrote.
        ("GET", p) if p.starts_with("/attach/") => {
            let rest = p.trim_start_matches("/attach/");
            let (convo, file) = rest.split_once('/').unwrap_or((rest, ""));
            match (stored_segment(convo, false), stored_segment(file, true)) {
                (Some(convo), Some(file)) => {
                    let path = attach_dir().join(&convo).join(&file);
                    let Ok(bytes) = tokio::fs::read(&path).await else {
                        return respond(
                            &mut stream,
                            404,
                            "application/json",
                            r#"{"error":"not found"}"#,
                        )
                        .await;
                    };
                    // Inline is what makes a thumbnail possible; `?dl=1` is the
                    // download the page offers beside it.
                    let disp = if param(query, "dl").as_deref() == Some("1") {
                        "attachment"
                    } else {
                        "inline"
                    };
                    let cd = format!("{disp}; filename=\"{file}\"");
                    respond_bytes(
                        &mut stream,
                        200,
                        mime_of(&path),
                        &[(&"Content-Disposition", &cd)],
                        &bytes,
                    )
                    .await
                }
                _ => {
                    respond(
                        &mut stream,
                        404,
                        "application/json",
                        r#"{"error":"not found"}"#,
                    )
                    .await
                }
            }
        }
        // Dictation. The take arrives as raw bytes, ffmpeg turns it into the
        // 16 kHz mono WAV the recognizer needs, the recognizer runs locally, and
        // both temp files are deleted: what someone says into a microphone is not
        // chat history, so it is not stored, not queued and not logged.
        ("POST", "/transcribe") => {
            let name = header(&head, "x-name").map(decode).unwrap_or_default();
            let lang = header(&head, "x-lang").map(decode).unwrap_or_default();
            let Some(name) = safe_name(&name) else {
                return respond(&mut stream, 415, "application/json", r#"{"error":"type"}"#).await;
            };
            if body.is_empty() {
                return respond(&mut stream, 400, "application/json", r#"{"error":"empty"}"#).await;
            }
            if body.len() > 24 * 1024 * 1024 {
                return respond(
                    &mut stream,
                    413,
                    "application/json",
                    r#"{"error":"too large"}"#,
                )
                .await;
            }
            if let Some(why) = webrain_core::asr::unavailable() {
                let out = json!({ "error": why }).to_string();
                return respond(&mut stream, 503, "application/json", &out).await;
            }
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let dir = attach_dir()
                .join(state.convo().await)
                .join(format!("dict-{stamp}"));
            if std::fs::create_dir_all(&dir).is_err() {
                return respond(&mut stream, 500, "application/json", r#"{"error":"dir"}"#).await;
            }
            let src = dir.join(&name);
            if std::fs::write(&src, &body).is_err() {
                return respond(&mut stream, 500, "application/json", r#"{"error":"write"}"#).await;
            }
            // Blocking work on the blocking pool: ffmpeg + an RNN-T pass are
            // seconds of CPU, and a hung recognizer must not park the runtime.
            let task = tokio::task::spawn_blocking(move || {
                let text = webrain_core::asr::transcribe_audio(&src, &lang);
                let _ = std::fs::remove_dir_all(&dir);
                text
            });
            let reply = match task.await {
                Ok(Ok(text)) => json!({ "text": text }),
                Ok(Err(e)) => json!({ "error": e.to_string() }),
                Err(e) => json!({ "error": e.to_string() }),
            };
            let code = if reply.get("text").is_some() {
                200
            } else {
                500
            };
            respond(&mut stream, code, "application/json", &reply.to_string()).await
        }
        // A conversation is addressable. `/<id>` serves the same shell, so a
        // reload — or a link pasted into another tab — opens that thread instead
        // of a 404, and the page reads the id back out of the path.
        ("GET", p) if is_convo_path(p) => {
            respond(&mut stream, 200, "text/html; charset=utf-8", &state.ui.html).await
        }
        _ => respond(&mut stream, 404, "text/plain", "not found").await,
    }
}

async fn stream_events(
    stream: &mut TcpStream,
    mut rx: broadcast::Receiver<String>,
) -> anyhow::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
        )
        .await?;
    // A page that reloads reconnects; lagging behind means the tab was asleep,
    // which is worth a `: skip` comment rather than a dropped connection.
    loop {
        match rx.recv().await {
            Ok(frame) => {
                if stream
                    .write_all(format!("data: {frame}\n\n").as_bytes())
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                if stream.write_all(b": skip\n\n").await.is_err() {
                    return Ok(());
                }
            }
            Err(broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

/// Read headers, then exactly `Content-Length` body bytes. Anything longer is
/// refused by the cap: this listener takes one small JSON object per request.
async fn read_request(stream: &mut TcpStream) -> anyhow::Result<(String, Vec<u8>)> {
    // Raised from 256 KiB for `/attach`: an image has to fit, and 8 MiB is the
    // ceiling the upload route enforces anyway.
    const MAX_BODY: usize = 8 * 1024 * 1024;
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_string();
            let want = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0)
                .min(MAX_BODY);
            let start = end + 4;
            while buf.len() < start + want {
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            // Bytes, not a String: an upload is binary.
            let body = buf[start..(start + want).min(buf.len())].to_vec();
            return Ok((head, body));
        }
        anyhow::ensure!(buf.len() <= MAX_BODY, "request head too large");
    }
    anyhow::bail!("empty request")
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> anyhow::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n",
        reason(status),
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    Ok(())
}

/// Same envelope as `respond`, for bytes that are not text — an image comes back
/// out exactly as it went in, with the headers a browser needs to show or save it.
async fn respond_bytes(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> anyhow::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n",
        reason(status),
        body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

/// The type a stored upload is served as. Kept to the list `safe_name` already
/// accepts, so a file that could be stored can also be shown.
fn mime_of(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "webm" => "audio/webm",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "json" => "application/json",
        "csv" => "text/csv",
        "xml" => "application/xml",
        _ => "text/plain; charset=utf-8",
    }
}

/// One path segment of a stored upload. No slash, no `..`, and a dot only where
/// a file extension belongs — the upload route's names are `{stamp}-{stem}.{ext}`,
/// so anything else is not a file this server wrote.
fn stored_segment(raw: &str, dotted: bool) -> Option<String> {
    let ok = !raw.is_empty()
        && raw.len() <= 96
        && !(dotted && raw.starts_with('.'))
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || (dotted && c == '.'));
    ok.then(|| raw.to_string())
}

/// The runtime's permission policy, as the page names it. `Unknown` is not
/// printed as a mode: the runtime sends it for a variant this build does not
/// model, and a page that claimed "manual" would be inventing the answer.
fn mode_name(m: &PermissionMode) -> &'static str {
    match m {
        PermissionMode::Manual => "manual",
        PermissionMode::Assisted => "assisted",
        PermissionMode::AllowAll => "allow-all",
        _ => "unknown",
    }
}

/// `manual` for anything unrecognised, because that is the mode that asks.
fn mode_of(name: &str) -> PermissionMode {
    match name {
        "allow-all" => PermissionMode::AllowAll,
        "assisted" => PermissionMode::Assisted,
        _ => PermissionMode::Manual,
    }
}

/// The runtime's *session* mode — what the agent is allowed to do — which is a
/// different switch from the permission policy above.
fn mode_word(m: &SessionMode) -> &'static str {
    match m {
        SessionMode::Interactive => "interactive",
        SessionMode::Plan => "plan",
        SessionMode::Autopilot => "autopilot",
        _ => "unknown",
    }
}

/// `interactive` for anything unrecognised: it is the mode that asks before it
/// acts, and the alternative is putting the agent in autopilot by typo.
fn session_mode_of(name: &str) -> SessionMode {
    match name {
        "plan" => SessionMode::Plan,
        "autopilot" => SessionMode::Autopilot,
        _ => SessionMode::Interactive,
    }
}

/// Whether a GET path is a conversation id rather than a route. Our ids are/// `c<millis>` and forks are `fork-<millis>`; a prefix test alone would swallow
/// The one-line summary of a page: what a link is worth showing before it is
/// clicked. Fetched here because a browser cannot read another origin's HTML.
///
/// Only `http`/`https` is followed — the URL arrives from the page, and a
/// `file:` or `data:` preview would turn a link into a local read.
async fn link_preview(url: &str) -> Value {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return json!({ "url": url, "error": "not an http(s) url" });
    }
    let owned = url.to_string();
    let got = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let body = ureq::get(&owned)
            .header("accept", "text/html,application/xhtml+xml")
            .header("user-agent", "Mozilla/5.0 (webrain chat link preview)")
            .call()
            .map_err(|e| e.to_string())?
            .into_body()
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        // 512 KB is more than any `<head>`; a 50 MB page is not a preview.
        let cut = body.len().min(512 * 1024);
        Ok(String::from_utf8_lossy(&body[..cut]).to_string())
    })
    .await;
    let html = match got {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return json!({ "url": url, "error": e }),
        Err(e) => return json!({ "url": url, "error": e.to_string() }),
    };
    let pick = |keys: &[&str]| -> String {
        keys.iter()
            .find_map(|k| meta_content(&html, k))
            .unwrap_or_default()
    };
    let og = pick(&["og:title"]);
    let title = if og.is_empty() {
        text_between(&html, "title").unwrap_or_default()
    } else {
        og
    };
    json!({
        "url": url,
        "title": clean(&title),
        "description": clean(&pick(&["og:description", "description", "twitter:description"])),
        "image": pick(&["og:image", "og:image:url", "twitter:image"]),
        "site": clean(&pick(&["og:site_name"])),
    })
}

/// `content="…"` of the `<meta>` that names `key`, in either attribute order —
/// the order in the file is the page's business, not ours.
fn meta_content(html: &str, key: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = html[from..].find(key) {
        let start = from + at;
        // The tag that carries it: back to the nearest `<meta>`.
        let open = html[..start].rfind("<meta").unwrap_or(start);
        let end = html[start..].find('>').map(|e| start + e)?;
        if let Some(v) = attr(&html[open..end], "content") {
            return Some(v);
        }
        from = end;
    }
    None
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}="))? + name.len() + 1;
    let quote = tag[at..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &tag[at + 1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// The text inside `<tag>…</tag>`, for the one case worth reading: the fallback
/// `<title>` when a page publishes no `og:title`.
fn text_between(html: &str, tag: &str) -> Option<String> {
    let at = html.find(&format!("<{tag}"))?;
    let start = html[at..].find('>')? + at + 1;
    let end = html[start..].find(&format!("</{tag}>"))? + start;
    Some(html[start..end].to_string())
}

/// Entities a `<title>` or a description realistically carries, decoded so the
/// card does not read `Tom &amp; Jerry`.
fn clean(s: &str) -> String {
    let t = s
        .trim()
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    t.chars().take(300).collect()
}

/// `/catalog`, `/chat.js`, `/context` and half the API, so the tail must be
/// digits.
fn is_convo_path(path: &str) -> bool {
    let s = path.trim_start_matches('/');
    let tail = s
        .strip_prefix('c')
        .or_else(|| s.strip_prefix("fork-"))
        .unwrap_or_default();
    !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit())
}

/// One stored conversation as the page wants it. Shared by the read and the
/// claim so opening a thread is one request instead of two.
fn convo_payload(state: &State, id: &str) -> Value {
    let store = state.store.as_ref();
    // The words, for the model's side of a reseed.
    let messages = store.map(|s| s.messages(id)).unwrap_or_default();
    // The numbers the right-hand panel shows. They are computed on the page
    // (provider prices, cache discounts), so they are stored as the page
    // computed them and replayed here.
    let turns = store.map(|s| s.turns(id)).unwrap_or_default();
    // The rows the page drew, when it kept them: a tool call with its arguments
    // and result is the evidence, and `messages` has only the words. Older
    // conversations fall back to the words.
    let rows = store.and_then(|s| s.rows(id)).unwrap_or(Value::Null);
    json!({ "id": id, "current": id, "messages": messages, "turns": turns, "rows": rows })
}

/// `respond`, gzipped when the client asked for it. A saved conversation is the
/// largest thing this server sends — 354–663 KB of JSON for a scrape, measured
/// off the wire — and it is repetitive JSON, so it deflates by roughly an order
/// of magnitude for the cost of one `miniz_oxide` pass that is already compiled
/// in via `flate2`.
async fn respond_gz(
    stream: &mut TcpStream,
    head: &str,
    status: u16,
    content_type: &str,
    body: &str,
) -> anyhow::Result<()> {
    // Below this the framing outweighs the saving, and `Vary` would need to be
    // honoured for a body nobody notices.
    const FLOOR: usize = 2048;
    let wants = header(head, "accept-encoding").is_some_and(|v| {
        v.to_ascii_lowercase()
            .split(',')
            .any(|enc| enc.trim().starts_with("gzip"))
    });
    if !wants || body.len() < FLOOR {
        return respond(stream, status, content_type, body).await;
    }
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(body.as_bytes())?;
    let gz = enc.finish()?;
    respond_bytes(
        stream,
        status,
        content_type,
        &[("Content-Encoding", "gzip"), ("Vary", "Accept-Encoding")],
        &gz,
    )
    .await
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

/// One value out of a `a=1&b=2` query string, percent- and `+`-decoded the way
/// `encodeURIComponent` / `URLSearchParams` produce it on the other end.
fn param(query: &str, key: &str) -> Option<String> {
    let raw = query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })?;
    Some(decode(raw))
}

fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            // Byte-wise: slicing the str here would split a multibyte char and
            // panic on a stray `%` before one.
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// A mode shapes the prompt; it does not change the tool set, which is fixed when
/// the session is created. `plan` is what a plan-mode answer is stored as, and
/// every other mode carries it so "do it" still has the plan in hand.
fn with_mode(prompt: &str, mode: &str, plan: &str) -> String {
    match mode {
        "plan" => format!(
            "[mode: plan] Investigate and answer with a numbered plan. Do not carry it out yet.\n\n{prompt}"
        ),
        "ask" => format!(
            "[mode: ask] Answer from the conversation and what you already know. Do not call tools.\n\n{prompt}"
        ),
        _ if !plan.trim().is_empty() => {
            format!("[the plan on file]\n{}\n\n{prompt}", plan.trim())
        }
        _ => prompt.to_string(),
    }
}

/// One response per base url for five minutes. Only the live catalogue uses it.
type ModelCache = std::sync::Mutex<HashMap<String, (std::time::Instant, Vec<Value>)>>;

static MODEL_CACHE: std::sync::OnceLock<ModelCache> = std::sync::OnceLock::new();

fn cached_models(base: &str) -> Option<Vec<Value>> {
    let cache = MODEL_CACHE.get_or_init(Default::default).lock().ok()?;
    let (at, models) = cache.get(base)?;
    (at.elapsed() < Duration::from_secs(300)).then(|| models.clone())
}

fn remember_models(base: &str, models: &[Value]) {
    if let Ok(mut cache) = MODEL_CACHE.get_or_init(Default::default).lock() {
        cache.insert(
            base.to_string(),
            (std::time::Instant::now(), models.to_vec()),
        );
    }
}

/// A new key invalidates every catalogue: the last fetch may have been the one
/// that failed for want of it.
fn forget_models() {
    if let Ok(mut cache) = MODEL_CACHE.get_or_init(Default::default).lock() {
        cache.clear();
    }
}

/// `GET {base}/models` — the one call every chat provider answers. The shape is
/// OpenAI's (`{"data":[{"id":…}]}`); anything else is reported, never guessed.
///
/// A timeout cancels the *wait*, not the blocking thread, which is why this runs
/// on the blocking pool: a vendor that hangs costs one parked thread, not the
/// whole page.
async fn list_models(
    base: &str,
    key: Option<&str>,
    typ: &'static str,
) -> Result<Vec<Value>, String> {
    let url = format!("{}/models", base.trim_end_matches('/'));
    let key = key.map(str::to_string);
    let call = tokio::task::spawn_blocking(move || {
        let mut req = ureq::get(&url).header("Accept", "application/json");
        if let Some(k) = &key {
            req = if typ == "anthropic" {
                req.header("x-api-key", k)
                    .header("anthropic-version", "2023-06-01")
            } else {
                req.header("Authorization", &format!("Bearer {k}"))
            };
        }
        let body = req
            .call()
            .map_err(|e| e.to_string())?
            .into_body()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        let v: Value =
            serde_json::from_str(&body).map_err(|_| "response was not JSON".to_string())?;
        let arr = v["data"]
            .as_array()
            .or_else(|| v["models"].as_array())
            .cloned()
            .unwrap_or_default();
        let mut models: Vec<Value> = arr
            .iter()
            .filter_map(|m| {
                let id = m["id"].as_str().or_else(|| m["name"].as_str())?;
                let mut out = json!({ "id": id });
                // OpenRouter is the one catalogue that says whether a model
                // takes a reasoning parameter; where it does not say, the page
                // leaves effort as a prompt-level hint rather than guessing.
                if let Some(params) = m["supported_parameters"].as_array() {
                    out["reasoning"] =
                        json!(params.iter().any(|p| p.as_str() == Some("reasoning")));
                }
                // OpenRouter publishes per-token prices and nobody else in this
                // table does. Passing them through is what lets the page show real
                // money instead of n/a; leaving the field absent is what keeps it
                // from inventing one.
                if let Some(p) = m["pricing"].as_object() {
                    out["pricing"] = json!({
                        "prompt": p.get("prompt").and_then(Value::as_str),
                        "completion": p.get("completion").and_then(Value::as_str),
                        // The cache-hit rate when the vendor lists one: a cached
                        // read is billed at a fraction of a fresh token, and
                        // pricing both the same overstates a warm conversation.
                        "input_cache_read": p.get("input_cache_read").and_then(Value::as_str),
                    });
                }
                // Catalogues spell the window several ways; the page needs one
                // number, and only when the vendor actually printed one.
                for k in [
                    "context_length",
                    "context_window",
                    "max_input_tokens",
                    "max_context_tokens",
                ] {
                    if let Some(n) = m[k].as_u64() {
                        out["context"] = json!(n);
                        break;
                    }
                }
                Some(out)
            })
            .collect();
        models.sort_by(|a, b| {
            a["id"]
                .as_str()
                .unwrap_or("")
                .cmp(b["id"].as_str().unwrap_or(""))
        });
        models.truncate(1000);
        Ok::<_, String>(models)
    });
    match tokio::time::timeout(Duration::from_secs(12), call).await {
        Ok(Ok(Ok(mut models))) => {
            // This endpoint lists prices itself, so a joined figure would only be
            // a second opinion; everyone else gets one.
            if !base.contains("openrouter.ai") {
                price_join(base, &mut models).await;
            }
            Ok(models)
        }
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(joined)) => Err(format!("models task failed: {joined}")),
        Err(_) => Err("provider did not answer within 12s".to_string()),
    }
}

/// Every skill this build can serve: the bundled set plus whatever is installed.
/// The name is the folder that holds `SKILL.md`.
fn skill_names() -> Vec<String> {
    let mut skills: Vec<String> = webrain_core::skills::BUNDLED
        .iter()
        .filter_map(|(p, _)| p.split('/').rev().nth(1).map(str::to_string))
        .collect();
    skills.extend(
        webrain_core::skills::list(&webrain_core::skills::dir())
            .into_iter()
            .map(|(name, _)| name),
    );
    skills.sort();
    skills.dedup();
    skills
}

/// A skill's menu hint. `None` when `name` is not a skill at all — a reference
/// chapter has no frontmatter `name:`, and that is the rule `webrain skills`
/// already uses to tell the two apart. The installed file wins over the copy
/// compiled into the binary, so an install older than the description it now
/// carries does not leave the menu with a bare name.
fn skill_hint(name: &str) -> Option<String> {
    let text = webrain_mcp::skill_text(name).or_else(|| {
        webrain_core::skills::BUNDLED
            .iter()
            .find(|(p, _)| p.split('/').rev().nth(1) == Some(name))
            .map(|(_, t)| t.to_string())
    })?;
    webrain_core::skills::frontmatter_name(&text)?;
    Some(
        webrain_core::skills::frontmatter(&text, "description")
            .unwrap_or_default()
            // A description is a paragraph; a menu row is one line of it.
            .chars()
            .take(96)
            .collect(),
    )
}

/// `@name` in a prompt is a mention, not prose. When `name` is an installed
/// skill, that skill's text rides along with the turn.
///
/// Without this the model reads the literal `@pdf-extraction` and has to go
/// fetch the skill itself — which it cannot do in ask mode, where the tools are
/// off, so it answers from the tool descriptions and admits it never read the
/// skill. The mention is the one place the page can know what was meant.
fn expand_mentions(prompt: &str) -> String {
    let named: Vec<(&str, String)> = mentions(prompt)
        .into_iter()
        .filter_map(|n| webrain_mcp::skill_text(n).map(|body| (n, body)))
        .collect();
    attach_skills(prompt, named)
}

/// Append each named skill to the prompt. Named first wins the budget: a skill
/// that does not fit is dropped rather than cut mid-sentence.
fn attach_skills(prompt: &str, skills: Vec<(&str, String)>) -> String {
    let mut out = String::new();
    let mut budget = 32 * 1024; // shortcut: 32 KiB of skills per turn, raise it if a real prompt names more
    for (name, body) in skills {
        let body = body.trim();
        if body.len() > budget {
            continue;
        }
        budget -= body.len();
        out.push_str(&format!("\n\n<skill name=\"{name}\">\n{body}\n</skill>"));
    }
    if out.is_empty() {
        return prompt.to_string();
    }
    format!("{prompt}{out}")
}

/// The distinct `@name` tokens of a prompt, in the order they appear. Trailing
/// punctuation is not part of a name, so `@pdf-extraction,` still resolves, and
/// an address like `foo@bar.com` is not a mention at all.
fn mentions(prompt: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for tok in prompt.split_whitespace() {
        let Some(name) = tok.strip_prefix('@') else {
            continue;
        };
        let name =
            name.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// The two preferences the page can hold that the model has to be *told*:
/// standing instructions and a reply language. Same honest lever as a mode —
/// a preamble on the prompt, not a switch we do not have.
///
/// Empty for both returns an empty string, so a turn with no settings is
/// byte-identical to what this sent before the settings existed.
fn user_notes(instructions: &str, language: &str) -> String {
    let mut out = String::new();
    // A page can post anything: 8 KiB is far more than a sane instruction block
    // and keeps one turn from becoming an upload.
    let text: String = instructions.trim().chars().take(8 * 1024).collect();
    if !text.is_empty() {
        out.push_str(
            "[instructions] The operator's standing instructions follow. Follow them unless \
             this message contradicts them.\n",
        );
        out.push_str(&text);
        out.push('\n');
    }
    let lang: String = language.trim().chars().take(40).collect();
    if !lang.is_empty() {
        out.push_str(&format!("[language] Reply in {lang}.\n"));
    }
    out
}

/// The composer's effort and tool picks are instructions too — the same honest
/// lever as a mode, not a runtime setting we do not have.
fn with_controls(
    prompt: &str,
    effort: &str,
    tools: &[String],
    tools_off: &[String],
    notes: &str,
) -> String {
    let mut out = String::new();
    match effort {
        "off" => out.push_str("[effort: off] Answer immediately; do not reason first.\n"),
        "low" => out.push_str("[effort: low] Answer directly and briefly.\n"),
        "high" => out.push_str(
            "[effort: high] Reason carefully before answering; check your work against what tools returned.\n",
        ),
        "max" => out.push_str(
            "[effort: max] Take as much reasoning as the task needs: plan, verify each step against tool output, and only then answer.\n",
        ),
        _ => {}
    }
    out.push_str(notes);
    if !tools.is_empty() {
        out.push_str(&format!(
            "[tools: prefer] Restrict yourself to these tools where they apply: {}.\n",
            tools.join(", ")
        ));
    }
    if !tools_off.is_empty() {
        out.push_str(&format!(
            "[tools: avoid] Do not call these unless the user asks for them by name: {}.\n",
            tools_off.join(", ")
        ));
    }
    if out.is_empty() {
        prompt.to_string()
    } else {
        format!("{out}\n{prompt}")
    }
}

/// A provider row is a target, not a secret: id and env-var name are the only
/// free-form fields that reach SQL, so both are held to one small alphabet.
/// `key_env` may be empty: a key typed into the page lives in the vault, and a
/// local endpoint needs no key at all.
fn valid_provider(id: &str, base_url: &str, model: &str, key_env: &str) -> bool {
    let slug = |s: &str| {
        s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    slug(id)
        && !id.is_empty()
        && slug(key_env)
        && !model.is_empty()
        && model.len() <= 128
        && (base_url.starts_with("http://") || base_url.starts_with("https://"))
}

fn attach_dir() -> PathBuf {
    std::env::var_os("WEBRAIN_CHAT_FILES")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("webrain-chat"))
}

/// The client's filename is data, never a path. One basename, a small alphabet,
/// a length cap and an extension from a list the page can actually use — so
/// `../../etc/passwd` and `x.html` both come back empty.
fn safe_name(raw: &str) -> Option<String> {
    const OK: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "txt", "md", "json", "csv", "log", "yml", "yaml",
        "toml", "rs", "js", "ts", "py", "css", "xml", "webm", "wav", "mp3", "ogg", "m4a",
    ];
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    // Split the extension before truncating, or a long name loses the very part
    // that decides whether it is allowed.
    let (stem, ext) = base.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    if !OK.contains(&ext.as_str()) {
        return None;
    }
    // `..png` and `.png` are names that start with a dot, not files with an
    // extension.
    let stem = stem.trim_start_matches('.');
    let clean: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    (!clean.is_empty()).then(|| format!("{clean}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_conversation_id_is_a_page() {
        for ok in ["/c1700000000000", "/fork-1700000000000", "/c1"] {
            assert!(is_convo_path(ok), "{ok}");
        }
        // Every one of these starts with `c` and is an API route.
        for bad in [
            "/catalog",
            "/chat.js",
            "/context",
            "/chat.css",
            "/c",
            "/c12x",
            "/",
        ] {
            assert!(!is_convo_path(bad), "{bad}");
        }
    }

    #[test]
    fn a_memory_write_asks_about_the_fact_not_the_tool() {
        let store = json!({
            "kind": "memory",
            "action": "store",
            "fact": "ab.gr prices need the desktop profile",
            "subject": "ab.gr",
            "citations": "webrain_navigate",
        });
        let card = permission_card("p1", &store, false);
        assert_eq!(card["kind"], "memory");
        assert_eq!(card["fact"], "ab.gr prices need the desktop profile");
        assert_eq!(card["title"], "remember this");
        assert_eq!(card["args"]["subject"], "ab.gr");

        // A vote is the same card with different words.
        let vote = json!({ "kind": "memory", "action": "vote", "fact": "f" });
        assert_eq!(
            permission_card("p2", &vote, false)["title"],
            "vote on a memory"
        );

        // Everything else is still a tool, with the server prefix stripped.
        let tool = json!({
            "kind": "tool",
            "serverName": "webrain",
            "toolName": "webrain-webrain_navigate",
            "args": { "url": "https://example.com" },
        });
        let card = permission_card("p3", &tool, false);
        assert_eq!(card["kind"], "tool");
        assert_eq!(card["tool"], "navigate");
        assert_eq!(card["args"]["url"], "https://example.com");
        // No name at all must not produce a card claiming to be something.
        assert_eq!(permission_card("p4", &json!({}), false)["tool"], "tool");
    }

    #[test]
    fn a_page_summary_is_read_from_either_attribute_order() {
        let page = concat!(
            r#"<html><head><title>Tom &amp; Jerry</title>"#,
            r#"<meta property="og:title" content="T1">"#,
            r#"<meta content="D1" name="description">"#,
            r#"<meta property='og:image' content='https://x/i.png'></head></html>"#
        );
        assert_eq!(meta_content(page, "og:title").as_deref(), Some("T1"));
        // `content` before `name` is just as valid and just as common.
        assert_eq!(meta_content(page, "description").as_deref(), Some("D1"));
        assert_eq!(
            meta_content(page, "og:image").as_deref(),
            Some("https://x/i.png")
        );
        assert_eq!(meta_content(page, "og:site_name"), None);
        assert_eq!(
            text_between(page, "title").as_deref(),
            Some("Tom &amp; Jerry")
        );
        assert_eq!(clean(" Tom &amp;  Jerry "), "Tom & Jerry");
        // A page with no `<head>` is not an error, it is an empty card.
        assert_eq!(meta_content("<p>hi</p>", "og:title"), None);
    }

    #[test]
    fn the_permission_mode_round_trips_through_its_wire_name() {
        // The page posts these exact strings, so a rename on either side would
        // silently fall back to `manual` and start asking again.
        for want in ["manual", "assisted", "allow-all"] {
            assert_eq!(mode_name(&mode_of(want)), want);
        }
        // Anything unrecognised must resolve to the mode that asks, never to
        // `allow-all`.
        assert_eq!(mode_name(&mode_of("")), "manual");
        assert_eq!(mode_name(&mode_of("Allow-All")), "manual");
    }

    #[test]
    fn the_session_mode_round_trips_and_never_defaults_to_autopilot() {
        // The composer's own three modes: `agent` and `ask` are both the
        // runtime's `interactive`, and only `plan` is a separate wire value.
        assert_eq!(mode_word(&session_mode_of("plan")), "plan");
        assert_eq!(mode_word(&session_mode_of("agent")), "interactive");
        assert_eq!(mode_word(&session_mode_of("ask")), "interactive");
        // A typo must not start a turn in the mode that stops asking.
        assert_eq!(mode_word(&session_mode_of("")), "interactive");
        assert_eq!(mode_word(&session_mode_of("Autopilot")), "interactive");
        // The one that *is* asked for still arrives.
        assert_eq!(mode_word(&session_mode_of("autopilot")), "autopilot");
    }

    #[test]
    fn finds_the_header_break_and_tolerates_absence() {
        assert_eq!(find(b"GET / HTTP/1.1\r\n\r\nbody", b"\r\n\r\n"), Some(14));
        assert_eq!(find(b"GET / HTTP/1.1\r\n", b"\r\n\r\n"), None);
    }

    #[test]
    fn mentions_are_names_not_prose() {
        assert_eq!(
            mentions("@pdf-extraction and @page-tiles."),
            ["pdf-extraction", "page-tiles"]
        );
        assert_eq!(
            mentions("@pdf-extraction @pdf-extraction"),
            ["pdf-extraction"]
        );
        // An address is not a mention, and neither is a bare `@`.
        assert!(mentions("mail me at foo@bar.com").is_empty());
        assert!(mentions("@ and @@ not either").is_empty());
        // A mention that is not a skill leaves the prompt alone.
        assert_eq!(expand_mentions("hello @nobody-here"), "hello @nobody-here");
    }

    #[test]
    fn a_named_skill_rides_along_with_the_prompt() {
        let out = attach_skills("explain @a", vec![("a", "  body a  ".to_string())]);
        assert_eq!(out, "explain @a\n\n<skill name=\"a\">\nbody a\n</skill>");
    }

    #[test]
    fn no_settings_means_no_preamble() {
        // Byte-identical to what a turn looked like before settings existed.
        assert_eq!(with_controls("hi", "", &[], &[], &user_notes("", "")), "hi");
        assert_eq!(user_notes("   ", "  "), "");
    }

    #[test]
    fn only_a_session_setting_asks_for_a_restart() {
        let mut s = RuntimeSettings::default();
        // The defaults are what `connect` hard-coded: nothing switched on, every
        // built-in reachable.
        assert!(!s.memory && !s.session_store && s.allow_agents);
        assert_eq!(s.max_output_tokens, 4096);
        // Timeout is read per turn, so it never costs a rebuilt session.
        assert!(!s.merge(&json!({"timeout_secs": 900})));
        assert_eq!(s.timeout_secs, 900);
        // A session field does.
        assert!(s.merge(&json!({"memory": true})));
        // Repeating the same value is not a change.
        assert!(!s.merge(&json!({"memory": true})));
        // A body that says nothing leaves everything alone.
        assert!(!s.merge(&Value::Null));
        // Values that cannot work are clamped, not stored: a one-second timeout
        // or a one-token budget is a chat that never answers.
        s.merge(&json!({"timeout_secs": 0, "max_output_tokens": 1}));
        assert_eq!(s.timeout_secs, TIMEOUT_S.0);
        assert_eq!(s.max_output_tokens, OUT_TOKENS.0);
        // The compaction point is a session field: the runtime reads it when the
        // session is created, so a change has to rebuild one.
        assert!(s.merge(&json!({"compact_at": 60})));
        assert_eq!(s.compact_at, 60);
        s.merge(&json!({"compact_at": 5}));
        assert_eq!(s.compact_at, COMPACT_PCT.0);
        // Parallel tool calls are creation-time in 2.0.1, so the switch is a
        // session field too — on by default, and a flip rebuilds.
        assert!(s.parallel_tools);
        assert!(s.merge(&json!({"parallel_tools": false})));
        assert!(!s.merge(&json!({"parallel_tools": false})));
        // And the stamp is what `connect` actually reads.
        let mut args = Args::default();
        s.stamp(&mut args);
        assert_eq!(args.memory, Some(true));
        assert_eq!(args.compact_at, Some(COMPACT_PCT.0));
    }

    #[test]
    fn instructions_and_language_reach_the_prompt() {
        let notes = user_notes("answer in bullets", "Greek");
        let out = with_controls("hi", "low", &[], &[], &notes);
        assert!(out.starts_with("[effort: low]"), "{out}");
        assert!(
            out.contains("[instructions]") && out.contains("answer in bullets"),
            "{out}"
        );
        assert!(out.contains("[language] Reply in Greek."), "{out}");
        assert!(out.ends_with("hi"), "{out}");
        // Over-long input is capped rather than refused: a page cannot upload.
        let huge = "x".repeat(20_000);
        assert_eq!(user_notes(&huge, "").matches('x').count(), 8 * 1024);
    }

    #[test]
    fn reasons_cover_every_status_the_server_sends() {
        for s in [200u16, 202, 400, 404, 409, 413, 415, 500] {
            assert_ne!(reason(s), "Error");
        }
        assert_eq!(reason(599), "Error");
    }

    #[test]
    fn a_stored_upload_can_only_be_named_by_its_own_segments() {
        // What the upload route writes: `{stamp}-{stem}.{ext}` inside a `c…` dir.
        assert_eq!(
            stored_segment("c1700000000000", false).as_deref(),
            Some("c1700000000000")
        );
        assert_eq!(
            stored_segment("1700000000000-shot.png", true).as_deref(),
            Some("1700000000000-shot.png")
        );
        // Traversal, absolute paths and dot-dirs are simply not segments.
        for bad in ["..", ".", "", "a/b", "a\\b", "%2e%2e", "x y"] {
            assert_eq!(stored_segment(bad, false), None, "{bad}");
        }
        assert_eq!(stored_segment("..", true), None);
        assert_eq!(stored_segment(".hidden", true), None);
        assert_eq!(stored_segment(&"a".repeat(120), true), None);
        // A type the browser cannot show is served as text, never guessed.
        assert_eq!(mime_of(std::path::Path::new("1700-shot.PNG")), "image/png");
        assert_eq!(mime_of(std::path::Path::new("1700-a.webp")), "image/webp");
        assert_eq!(
            mime_of(std::path::Path::new("1700-a.md")),
            "text/plain; charset=utf-8"
        );
    }

    #[test]
    fn decodes_query_values_like_urlsearchparams() {
        assert_eq!(param("q=hello+world", "q").as_deref(), Some("hello world"));
        assert_eq!(param("q=a%20b%2Fc", "q").as_deref(), Some("a b/c"));
        assert_eq!(param("x=1&q=two", "q").as_deref(), Some("two"));
        // A key that merely starts with the one asked for is not a match.
        assert_eq!(param("query=x", "q"), None);
        // Bad escapes survive as text instead of vanishing or panicking.
        assert_eq!(param("q=100%", "q").as_deref(), Some("100%"));
        assert_eq!(param("q=%zz", "q").as_deref(), Some("%zz"));
        assert_eq!(param("q=%e2%82%ac", "q").as_deref(), Some("€"));
        assert_eq!(param("", "q"), None);
    }

    #[test]
    fn upload_names_cannot_escape_the_upload_directory() {
        assert_eq!(safe_name("shot.png").as_deref(), Some("shot.png"));
        assert_eq!(
            safe_name("C:\\Users\\x\\notes.md").as_deref(),
            Some("notes.md")
        );
        assert_eq!(safe_name("../../etc/passwd"), None);
        assert_eq!(
            safe_name("..\\..\\Windows\\evil.png").as_deref(),
            Some("evil.png")
        );
        // Types the page never renders are refused rather than stored.
        assert_eq!(safe_name("payload.html"), None);
        assert_eq!(safe_name("x.svg"), None);
        assert_eq!(safe_name(".png"), None);
        assert_eq!(safe_name(""), None);
        // Odd characters collapse instead of travelling.
        assert_eq!(
            safe_name("my photo (1).png").as_deref(),
            Some("my_photo__1_.png")
        );
        assert_eq!(safe_name(&"a".repeat(200)), None);
        assert!(
            safe_name(&format!("{}.png", "a".repeat(200)))
                .expect("long but valid")
                .len()
                <= 64
        );
    }

    #[test]
    fn modes_shape_the_prompt_only() {
        assert_eq!(with_mode("do it", "agent", ""), "do it");
        assert_eq!(with_mode("do it", "", ""), "do it");
        let plan = with_mode("do it", "plan", "");
        assert!(plan.starts_with("[mode: plan]"));
        assert!(plan.ends_with("do it"));
        assert!(with_mode("x", "ask", "").contains("Do not call tools"));
    }

    #[test]
    fn a_saved_plan_rides_along_with_the_next_turn() {
        let p = with_mode("do it", "agent", "1. read the page\n2. extract the price");
        assert!(p.starts_with("[the plan on file]"), "{p}");
        assert!(p.contains("2. extract the price"), "{p}");
        assert!(p.ends_with("do it"), "{p}");
        // Plan mode writes the plan, so it must not be handed its own draft back.
        assert!(!with_mode("x", "plan", "stale").contains("stale"));
        // Whitespace is not a plan.
        assert_eq!(with_mode("do it", "agent", "  \n "), "do it");
    }

    #[test]
    fn a_reopened_conversation_carries_its_tail() {
        let msgs = vec![
            json!({"role": "user", "body": "first"}),
            json!({"role": "assistant", "body": "second"}),
            json!({"role": "user", "body": "third"}),
        ];
        // Oldest first, and each row names who said it.
        assert_eq!(
            transcript(&msgs, 1000),
            "user: first\nassistant: second\nuser: third"
        );
        // The cap keeps the NEWEST messages: a reopened chat continues from the
        // end, not the beginning.
        assert_eq!(transcript(&msgs, 15), "user: third");
        assert_eq!(transcript(&[], 100), "");
        // A stored body of whitespace is not something the model should read.
        assert_eq!(
            transcript(&[json!({"role": "user", "body": "   "})], 100),
            ""
        );
    }

    #[test]
    fn provider_rows_are_validated_before_they_reach_sql() {
        assert!(valid_provider(
            "or",
            "https://openrouter.ai/api/v1",
            "m",
            "KEY"
        ));
        assert!(valid_provider(
            "a-b_1",
            "http://127.0.0.1:11434/v1",
            "m",
            "K1"
        ));
        // The id and the env var name are the free-form fields: one alphabet only.
        assert!(!valid_provider("a b", "https://x/v1", "m", "KEY"));
        assert!(!valid_provider("a;drop", "https://x/v1", "m", "KEY"));
        assert!(!valid_provider("ok", "https://x/v1", "m", "with space"));
        assert!(!valid_provider("", "https://x/v1", "m", "KEY"));
        // A scheme-less or file:// base url is not a provider target.
        assert!(!valid_provider("ok", "openrouter.ai/api", "m", "KEY"));
        assert!(!valid_provider("ok", "file:///etc/passwd", "m", "KEY"));
        assert!(!valid_provider("ok", "https://x/v1", "", "KEY"));
        // An empty env-var name is the vault-keyed (or keyless) case, and still
        // needs a usable id.
        assert!(valid_provider("local", "http://127.0.0.1:8080/v1", "m", ""));
        assert!(!valid_provider("", "http://127.0.0.1:8080/v1", "m", ""));
    }

    /// Every row the page can show must have something to say about itself, and
    /// a provider whose catalogue publishes no context must not borrow another's.
    #[test]
    fn every_provider_row_is_described() {
        for row in webrain_core::byok::CHAT_PROVIDERS.iter() {
            let id = row.0[0].trim_end_matches("_API_KEY").to_ascii_lowercase();
            let (description, _, docs) = provider_info(&id);
            assert!(!description.is_empty(), "{id} has no description");
            assert!(docs.starts_with("https://"), "{id} has no docs link");
            assert_eq!(row_name_for_base(row.1), id, "row name round-trips");
        }
        // Unknown ids degrade to empty rather than to someone else's copy.
        assert_eq!(provider_info("nope"), ("", None, ""));
    }

    /// DeepSeek is half price outside its peak windows; those windows are the
    /// only reason this modal exists, so they are asserted, not buried in JS.
    #[test]
    fn deepseek_reports_peak_windows_and_nobody_else_does() {
        let op = off_peak("deepseek");
        assert_eq!(op["peak"], json!([[1, 4], [6, 10]]));
        assert_eq!(op["days"], json!([1, 2, 3, 4, 5]));
        assert!(off_peak("openrouter").is_null());
    }
}
