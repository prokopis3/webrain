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

use github_copilot_sdk::handler::{
    PermissionHandler, PermissionResult, UserInputHandler, UserInputResponse,
};
use github_copilot_sdk::types::{
    DeliveryMode, PermissionRequestData, RequestId, SessionId, SetModelOptions,
};
use github_copilot_sdk::{Client, session::Session};
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

    let live = spawn_live(
        args,
        provider,
        permission.clone(),
        input.clone(),
        bus.clone(),
        store.clone(),
        convo.clone(),
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
        timeout: Duration::from_secs(args.timeout_secs),
        busy: AtomicBool::new(false),
        store,
        convo,
        ui,
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(stream, state).await {
                eprintln!("· http · {e}");
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
async fn spawn_live(
    args: &Args,
    provider: &ChatProvider,
    permission: Arc<WebPermission>,
    input: Arc<WebInput>,
    bus: Bus,
    store: Option<Arc<chat_store::Store>>,
    convo: String,
) -> anyhow::Result<Live> {
    // The runtime's own idea of a window is Copilot's 128k, so a 1M-token model
    // was measured at 7% of 128k and compacted at a quarter of its real size.
    // The vendor's catalogue is the authority here — it is the same number the
    // page prints — and `--max-prompt-tokens` still wins over it.
    let mut args = args.clone();
    args.max_prompt_tokens = args
        .max_prompt_tokens
        .or_else(|| model_context(&provider.base_url, &provider.model));
    let (client, session) = chat_session::connect(
        &args,
        provider,
        Some(permission as Arc<dyn PermissionHandler>),
        Some(input as Arc<dyn UserInputHandler>),
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
                "type": event.event_type,
                "data": event.data,
            });
            // A closed bus just means nobody is watching.
            let _ = out.send(frame.to_string());
        }
    });
    Ok(Live {
        _client: client,
        session,
        reader,
        model: std::sync::Mutex::new(provider.model.clone()),
        effort: std::sync::Mutex::new(String::new()),
        base_url: provider.base_url.clone(),
    })
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
pub(crate) fn turn_cost(base: &str, model: &str, input: i64, output: i64, cached: i64) -> Option<f64> {
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
    _client: Client,
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

struct State {
    bus: Bus,
    permission: Arc<WebPermission>,
    input: Arc<WebInput>,
    /// Swappable: a provider change makes a new session, and the turn handler
    /// reads whichever one is current.
    live: RwLock<Arc<Live>>,
    /// The first session's flags, so a replacement is configured identically.
    args: Args,
    timeout: Duration,
    busy: AtomicBool,
    store: Option<Arc<chat_store::Store>>,
    convo: String,
    /// The page, read from disk and verified once at startup — not carried in
    /// the binary (`super::chat_ui`).
    ui: super::chat_ui::Ui,
}

/// Answers a permission prompt from the page instead of the CLI's policy.
struct WebPermission {
    bus: Bus,
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
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
        self.pending.lock().await.insert(id.clone(), tx);
        // The runtime packs a structured request inside `extra` — server, tool,
        // arguments, whether it only reads. `format!("{data:?}")` was what turned
        // the dialog into a wall of text; these are the fields a person decides on.
        let req = &data.extra["permissionRequest"];
        let tool = req["toolName"].as_str().unwrap_or("tool");
        let ask = json!({
            "type": "webrain.permission",
            "data": {
                "id": id,
                "kind": req["kind"].as_str().unwrap_or("tool"),
                "server": req["serverName"].as_str().unwrap_or_default(),
                // `webrain-webrain_pdf` → `pdf`: the doubling is the MCP server
                // key plus the tool name, and it means nothing to a reader.
                "tool": tool.strip_prefix("webrain-webrain_").unwrap_or(tool),
                "title": req["toolTitle"].as_str().unwrap_or(tool),
                "args": req["args"].clone(),
                "read_only": req["readOnly"].as_bool().unwrap_or(false),
                "managed": data.managed_approval_required.unwrap_or(false),
            },
        });
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

struct WebInput {
    bus: Bus,
    pending: Mutex<HashMap<String, oneshot::Sender<(String, bool)>>>,
    seq: AtomicU64,
}

/// Answers `ask_user`, which is not a permission: it is a question, and a turn
/// that gets none blocks. `None` means "no answer available" and lets the
/// runtime fall back to its own prompt.
#[async_trait::async_trait]
impl UserInputHandler for WebInput {
    async fn handle(
        &self,
        _session: SessionId,
        question: String,
        choices: Option<Vec<String>>,
        allow_freeform: Option<bool>,
    ) -> Option<UserInputResponse> {
        let id = format!("u{}", self.seq.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let ask = json!({
            "type": "webrain.ask",
            "data": {
                "id": id,
                "question": question,
                "choices": choices.unwrap_or_default(),
                "allow_freeform": allow_freeform.unwrap_or(true),
            },
        });
        let _ = self.bus.send(ask.to_string());

        // Longer than a permission: a person has to read and type an answer.
        let answer = tokio::time::timeout(INPUT_WAIT, rx).await;
        self.pending.lock().await.remove(&id);
        match answer {
            Ok(Ok((answer, was_freeform))) => Some(UserInputResponse {
                answer,
                was_freeform,
            }),
            _ => None,
        }
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
        ("GET", "/") => {
            respond(&mut stream, 200, "text/html; charset=utf-8", &state.ui.html).await
        }
        ("GET", "/chat.js") => {
            respond(&mut stream, 200, "text/javascript; charset=utf-8", &state.ui.js).await
        }
        ("GET", "/chat.css") => {
            respond(&mut stream, 200, "text/css; charset=utf-8", &state.ui.css).await
        }
        // The markdown renderer's chunk. `chat.js` imports it dynamically, so the
        // first paint never parses `marked` or DOMPurify; the page warms it when
        // the browser is idle.
        ("GET", "/chunk-markdown.js") => {
            respond(&mut stream, 200, "text/javascript; charset=utf-8", &state.ui.markdown).await
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
            let prompt = with_controls(
                &with_mode(&expand_mentions(&raw), mode),
                effort,
                &tools,
                &tools_off,
                &user_notes(
                    payload["instructions"].as_str().unwrap_or_default(),
                    payload["language"].as_str().unwrap_or_default(),
                ),
            );
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
                    store.append(&state.convo, "user", &raw);
                }
                let live = state.live.read().await.clone();
                let opts = github_copilot_sdk::MessageOptions::new(prompt.as_str()).with_mode(
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
                store.append(&state.convo, "user", &raw);
            }
            let live = state.live.read().await.clone();
            let st = state.clone();
            tokio::spawn(async move {
                let wait = github_copilot_sdk::MessageOptions::new(prompt.as_str())
                    .with_wait_timeout(st.timeout);
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
        ("POST", "/permission") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let id = v["id"].as_str().unwrap_or_default().to_string();
            let allow = v["allow"].as_bool().unwrap_or(false);
            let sent = match state.permission.pending.lock().await.remove(&id) {
                Some(tx) => tx.send(allow).is_ok(),
                None => false,
            };
            let body = json!({ "answered": sent }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
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
        // The session menu: what was said before, and the current id so the page
        // can mark the live conversation.
        ("GET", "/sessions") => {
            let list = state.store.as_ref().map(|s| s.convos()).unwrap_or_default();
            let body = json!({ "current": state.convo, "sessions": list }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        ("GET", p) if p.starts_with("/sessions/") => {
            let id = p.trim_start_matches("/sessions/");
            let msgs = state
                .store
                .as_ref()
                .map(|s| s.messages(id))
                .unwrap_or_default();
            // The numbers the right-hand panel shows. They are computed on the
            // page (provider prices, cache discounts), so they are stored as the
            // page computed them and replayed here.
            let turns = state
                .store
                .as_ref()
                .map(|s| s.turns(id))
                .unwrap_or_default();
            let body = json!({ "id": id, "messages": msgs, "turns": turns }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        // The instruments of a finished turn, handed over by the page that
        // measured them. A conversation reopened from history used to show a
        // column of `—` and "no turns yet" because nothing kept these.
        ("POST", "/ledger") => {
            let v = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            let n = v["n"].as_u64().unwrap_or(0) as u32;
            let saved = n > 0
                && state
                    .store
                    .as_ref()
                    .is_some_and(|s| s.record_turn(&state.convo, n, &v["turn"].to_string()));
            let out = json!({ "saved": saved }).to_string();
            respond(&mut stream, 200, "application/json", &out).await
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
            let from = v["convo"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(state.convo.as_str())
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
            match spawn_live(
                &state.args,
                &provider,
                state.permission.clone(),
                state.input.clone(),
                state.bus.clone(),
                state.store.clone(),
                state.convo.clone(),
            )
            .await
            {
                Ok(fresh) => {
                    let old = std::mem::replace(&mut *state.live.write().await, Arc::new(fresh));
                    // Stop the fan-out before the runtime it reads from.
                    old.reader.abort();
                    drop(old);
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
            // Installed on the session by this binary, not reachable through the
            // MCP server, so they come last and read as the always-on tail.
            for name in chat_tools::NAMES
                .iter()
                .map(|n| n.to_string())
                .chain(["ask_user".to_string(), "skill".to_string()])
            {
                items.push(json!({"name": name, "kind": "builtin", "hint": "installed by the runtime"}));
            }
            let body = json!({ "items": items }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
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
            let dir = attach_dir().join(&state.convo);
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
                    respond_bytes(&mut stream, 200, mime_of(&path), &[(&"Content-Disposition", &cd)], &bytes)
                        .await
                }
                _ => {
                    respond(&mut stream, 404, "application/json", r#"{"error":"not found"}"#).await
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
                .join(&state.convo)
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
/// the session is created.
fn with_mode(prompt: &str, mode: &str) -> String {
    match mode {
        "plan" => format!(
            "[mode: plan] Investigate and answer with a numbered plan. Do not carry it out yet.\n\n{prompt}"
        ),
        "ask" => format!(
            "[mode: ask] Answer from the conversation and what you already know. Do not call tools.\n\n{prompt}"
        ),
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
        let name = name.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
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
    fn finds_the_header_break_and_tolerates_absence() {
        assert_eq!(find(b"GET / HTTP/1.1\r\n\r\nbody", b"\r\n\r\n"), Some(14));
        assert_eq!(find(b"GET / HTTP/1.1\r\n", b"\r\n\r\n"), None);
    }

    #[test]
    fn mentions_are_names_not_prose() {
        assert_eq!(mentions("@pdf-extraction and @page-tiles."), ["pdf-extraction", "page-tiles"]);
        assert_eq!(mentions("@pdf-extraction @pdf-extraction"), ["pdf-extraction"]);
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
    fn instructions_and_language_reach_the_prompt() {
        let notes = user_notes("answer in bullets", "Greek");
        let out = with_controls("hi", "low", &[], &[], &notes);
        assert!(out.starts_with("[effort: low]"), "{out}");
        assert!(out.contains("[instructions]") && out.contains("answer in bullets"), "{out}");
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
        assert_eq!(
            mime_of(std::path::Path::new("1700-shot.PNG")),
            "image/png"
        );
        assert_eq!(mime_of(std::path::Path::new("1700-a.webp")), "image/webp");
        assert_eq!(mime_of(std::path::Path::new("1700-a.md")), "text/plain; charset=utf-8");
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
        assert_eq!(with_mode("do it", "agent"), "do it");
        assert_eq!(with_mode("do it", ""), "do it");
        let plan = with_mode("do it", "plan");
        assert!(plan.starts_with("[mode: plan]"));
        assert!(plan.ends_with("do it"));
        assert!(with_mode("x", "ask").contains("Do not call tools"));
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
