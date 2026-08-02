// CDP WebSocket backend — connects to any Chrome-compatible CDP endpoint.
// Works with: obscura serve, lightpanda serve, Chrome --remote-debugging-port.
//
// ponytail: single connection, one page, synchronous CDP command/response per call.

use crate::browser::{detect_antibot, BrowserBackend, InteractiveElement, PageState};
use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

/// Anti-bot JS injected before page scripts run (Page.addScriptToEvaluateOnNewDocument).
/// Self-destructing by design: everything lives inside the IIFE, so no `window.setXxx`
/// helper survives for page scripts to detect (camoufox addInitScript pattern).
const STEALTH_JS: &str = r#"
(() => {
  const apply = (obj, prop, val) => {
    try { Object.defineProperty(obj, prop, { get: () => val, configurable: true }); } catch (e) {}
  };
  apply(Navigator.prototype, 'webdriver', false);
  apply(Navigator.prototype, 'languages', ['en-US', 'en']);
  apply(Navigator.prototype, 'plugins', [1, 2, 3, 4, 5]);
  apply(Navigator.prototype, 'maxTouchPoints', 1);
  apply(Navigator.prototype, 'hardwareConcurrency', 8);
  apply(Navigator.prototype, 'deviceMemory', 8);
  apply(Navigator.prototype, 'platform', 'Win32');
  apply(Navigator.prototype, 'vendor', 'Google Inc.');
  apply(Navigator.prototype, 'oscpu', 'Windows NT 10.0; Win64; x64');
  // WebGL vendor/renderer — hook getParameter once, self-destruct inside the IIFE.
  try {
    const gl = document.createElement('canvas').getContext('webgl');
    if (gl) {
      const ext = gl.getExtension('WEBGL_debug_renderer_info');
      const orig = gl.getParameter.bind(gl);
      gl.getParameter = (p) => {
        if (ext && p === ext.UNMASKED_VENDOR_WEBGL) return 'Google Inc. (Intel)';
        if (ext && p === ext.UNMASKED_RENDERER_WEBGL) return 'ANGLE (Intel, Intel(R) UHD Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)';
        return orig(p);
      };
    }
  } catch (e) {}
  // Canvas + audio fingerprint noise: deterministic per-context seed so the
  // canvas/audio hash differs across contexts but is stable within one
  // (camoufox seed pattern, JS-level approximation). Kills hash fingerprinting.
  try {
    const _seed = (Date.now() & 0x7fffffff) || 1;
    const _det = (i, m) => ((Math.imul(i * 2654435761, _seed) ^ 0x9e3779b9) >>> 8) % m;
    const _hook = (proto, name, mut) => {
      const orig = proto[name];
      proto[name] = function () {
        const out = orig.apply(this, arguments);
        try { mut(out); } catch (e) {}
        return out;
      };
    };
    _hook(CanvasRenderingContext2D.prototype, 'getImageData', (img) => {
      const d = img.data;
      for (let i = 0; i < d.length; i += 4) {
        const n = _det(i, 7) - 3;
        d[i] = (d[i] + n) & 255;
        d[i + 1] = (d[i + 1] + n) & 255;
        d[i + 2] = (d[i + 2] + n) & 255;
      }
    });
    _hook(AudioBuffer.prototype, 'getChannelData', (buf) => {
      for (let i = 0; i < buf.length; i++) buf[i] *= 1 + (_det(i, 25) - 12) / 1000;
    });
  } catch (e) {}
  window.chrome = window.chrome || { runtime: {} };
})();
"#;

/// Trackers/analytics/fingerprinting hosts blocked at the network layer before
/// they load (obscura/camofox-browser pattern). CDP `Network.setBlockedURLs`
/// wildcard patterns; blocking these never breaks page function, only tracking.
const BLOCKED_URLS: &[&str] = &[
    "*google-analytics.com*", "*googletagmanager.com*", "*googlesyndication.com*",
    "*doubleclick.net*", "*facebook.net*", "*facebook.com/tr*", "*ads-twitter.com*",
    "*hotjar.com*", "*newrelic.com*", "*mixpanel.com*", "*segment.io*",
    "*amplitude.com*", "*intercomcdn.com*", "*scorecardresearch.com",
    "*criteo.com*", "*taboola.com*", "*outbrain.com*", "*quantserve.com",
    "*chartbeat.com*", "*fullstory.com*", "*mouseflow.com*", "*crazyegg.com*",
    "*snap.licdn.com*", "*linkedin.com/analytics*", "*gtag/js*", "*/gtm.js*",
    "*/analytics.js*", "*/ga.js*",
];

/// Resource-type patterns added to the block list when `disable_resources` is on
/// (Scrapling's font/image/media/stylesheet drop — speeds loads, saves tokens).
const RESOURCE_PATTERNS: &[&str] = &[
    "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.avif", "*.svg", "*.ico",
    "*.css", "*.woff", "*.woff2", "*.ttf", "*.otf", "*.eot", "*.mp4", "*.webm",
    "*.mp3", "*.ogg", "*.wav", "*.m4a", "*.pdf",
];

/// 3500 tracker/ad domains ported from anudeepND/blacklist (see scripts/port_blocklist.ps1).
/// Loaded at compile time via include_str! — zero runtime IO. Applied to CDP only when
/// `block_trackers` is on (the big list is ~35KB over CDP per navigate; keep default fast path
/// at the 28 wildcards above). Lazy OnceLock parse: 3500 lines split once, microsecond cost.
fn tracker_domains() -> &'static [&'static str] {
    static DOMAINS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    DOMAINS.get_or_init(|| {
        include_str!("../../data/tracker_domains.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    })
}
/// Optional navigate/batch request-quality params (Scrapling-style). All default off.
/// ponytail: one shared struct, threaded through navigate + navigate_session so the
/// single-tool path and the batch crawl path hit the SAME wait logic — a fix here
/// covers every caller, not just the one the ticket names.
#[derive(Default, Clone)]
pub struct NavOpts {
    /// Block font/image/media/stylesheet requests for speed + token savings.
    pub disable_resources: bool,
    /// Also block the 3500-domain tracker list (default off — ~35KB over CDP per navigate).
    pub block_trackers: bool,
    /// Wait until network is idle (no new resource entries ~400ms) before returning.
    pub network_idle: bool,
    /// Wait for a CSS selector to reach `wait_selector_state` before returning.
    pub wait_selector: Option<String>,
    /// attached | visible | hidden | detached (default "visible" when selector set).
    pub wait_selector_state: String,
    /// If set, narrow returned text to this element's innerText (token saver).
    pub css_selector: Option<String>,
}

/// JS that indexes interactive elements for click/type tools (shared by
/// navigate and snapshot so both produce identical element lists).
/// Interaction is index-based only (querySelectorAll order); precise
/// selectors for extraction come from webrain_a11y (css_path/xpath).
pub const ELEMENTS_JS: &str = r#"
        (() => {
            const elems = document.querySelectorAll('a, button, input, select, textarea, [role="button"]');
            return Array.from(elems).slice(0, 60).map((el, i) => ({
                index: i,
                tag: el.tagName.toLowerCase(),
                text: (el.textContent || el.value || '').trim().substring(0, 80),
                selector: el.id ? '#' + el.id : el.className ? '.' + el.className.split(' ')[0] : el.tagName.toLowerCase(),
                visible: el.offsetParent !== null
            }));
        })()
        "#;

/// Compact page-state caps: keep MCP responses small (a11y-style), not full-page dumps.
/// ponytail: 60 elements + 3k text is enough for an agent step; a11y gives full structure.
const PAGE_TEXT_CAP: usize = 3000;

/// JS that collects deduped same-origin links (capped) for the PageState `links`
/// field. Scrapling LinkExtractor-level quality: canonicalize, filter non-http
/// schemes, strip fragments, drop content-obvious extensions (images, fonts,
/// pdf, archives, ico, css/js), dedupe in insertion order. Turns "crawl +
/// internal links" into a single navigate call.
/// ponytail: returns the array directly (like ELEMENTS_JS) so returnByValue gives
/// a JSON array — JSON.stringify would make from_value::<Vec<_>>() fail.
pub const LINKS_JS: &str = r#"
    (() => {
        try {
            const origin = location.origin;
            const skipExt = new Set(['pdf','zip','rar','7z','tar','gz','xz','bz2',
                'jpg','jpeg','png','gif','webp','avif','svg','ico','bmp','tif','tiff',
                'woff','woff2','ttf','otf','eot',
                'mp4','webm','mp3','ogg','wav','mov','avi','m4a',
                'css','js','json','xml','rss',
                'exe','dmg','iso','apk','msi']);
            const seen = new Set();
            const out = [];
            for (const el of document.querySelectorAll('a[href], area[href]')) {
                const raw = el.href || '';
                if (!raw) continue;
                // strip fragment + trailing slash; drop non-http
                let u = raw;
                const hash = u.indexOf('#');
                if (hash > -1) u = u.slice(0, hash);
                // drop non-http(s) schemes (mailto, javascript, tel, file, etc.)
                if (!u.startsWith('http://') && !u.startsWith('https://')) continue;
                // same-origin only
                if (!u.startsWith(origin)) continue;
                // trailing-slash normalise
                const qp = u.indexOf('?');
                let path = qp > -1 ? u.slice(0, qp) : u;
                if (path.endsWith('/')) u = u.slice(0, path.length - 1) + (qp > -1 ? u.slice(qp) : '');
                // drop known non-content extensions
                const seg = u.split('/').pop() || '';
                const dot = seg.lastIndexOf('.');
                if (dot > -1 && skipExt.has(seg.slice(dot + 1).toLowerCase())) continue;
                if (seen.has(u)) continue;
                seen.add(u);
                if (out.push(u) >= 200) break;
            }
            return out;
        } catch (e) { return []; }
    })()
    "#;

/// Resolve a CDP HTTP endpoint (or pass through a ws:// URL) to its browser WebSocket URL.
fn resolve_ws(http_url: &str) -> anyhow::Result<String> {
    if http_url.starts_with("ws://") || http_url.starts_with("wss://") {
        return Ok(http_url.to_string());
    }
    // ponytail: 5s timeout — a half-open/filtered port must fail fast, not hang
    // every browser-dependent MCP call (seen: batch "0 results" after browser died).
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build(),
    );
    let resp = agent
        .get(&format!("{http_url}/json/version"))
        .call()
        .context("CDP endpoint not reachable. Is a browser running?")?;
    let s = resp.into_body().read_to_string()?;
    let body: Value = serde_json::from_str(&s)?;
    body["webSocketDebuggerUrl"]
        .as_str()
        .map(|s| s.to_string())
        .context("No webSocketDebuggerUrl in response")
}

/// Clone shares the same CDP connection + tab registry (all Arc internals), so
/// concurrent tasks can each own a handle to drive their OWN tab concurrently.
#[derive(Clone)]
pub struct CdpBackend {
    /// Browser-level WebSocket; commands are routed per-tab via sessionId.
    inner: Arc<Mutex<CdpConnection>>,
    /// Attached page tabs: id → target/session. The active tab receives commands.
    tabs: Arc<Mutex<HashMap<String, Tab>>>,
    active: Arc<Mutex<String>>,
    next_id: Arc<Mutex<u64>>,
    /// D1 fingerprint cache: (element count, text hash) → last PageState.
    fp: Arc<Mutex<Option<(usize, usize)>>>,
    snap: Arc<Mutex<Option<PageState>>>,
    /// Default execution-context id of the active document (from
    /// Runtime.executionContextCreated). Runtime.evaluate without a contextId can
    /// land in a stale pre-navigation context (empty body, stub performance).
    exec_ctx: Arc<Mutex<Option<i64>>>,
    /// Network capture: while net_capture is set, requestWillBeSent URLs are
    /// buffered in net_urls (browsemind NetworkCapture pattern).
    net_capture: Arc<Mutex<bool>>,
    net_urls: Arc<Mutex<Vec<String>>>,
}

/// A single attached page target (one browser tab).
struct Tab {
    target_id: String,
    session_id: Option<String>,
    url: String,
}

struct CdpConnection {
    write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    cmd_id: u64,
}

impl CdpBackend {
    /// Connect to a CDP browser at the given WebSocket URL.
    /// Example: `ws://127.0.0.1:9222/devtools/browser/...`
    pub async fn connect(ws_url: &str) -> anyhow::Result<Self> {
        // ponytail: 5s timeout on the WS handshake — same half-open-port guard as
        // resolve_ws. connect_async can hang indefinitely without it.
        let (ws, _) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .map_err(|_| anyhow::anyhow!("CDP connect timed out after 5s"))?
        .context("CDP WebSocket connection failed. Is the browser running?")?;

        let (write, read) = ws.split();

        let inner = CdpConnection { write, read, cmd_id: 1 };

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            tabs: Arc::new(Mutex::new(HashMap::new())),
            active: Arc::new(Mutex::new(String::new())),
            next_id: Arc::new(Mutex::new(0)),
            fp: Arc::new(Mutex::new(None)),
            snap: Arc::new(Mutex::new(None)),
            exec_ctx: Arc::new(Mutex::new(None)),
            net_capture: Arc::new(Mutex::new(false)),
            net_urls: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Connect from CDP_URL env (default http://127.0.0.1:9222).
    /// ponytail: one shared resolution for CLI and MCP.
    pub async fn connect_default() -> anyhow::Result<Self> {
        let cdp_url =
            std::env::var("CDP_URL").unwrap_or_else(|_| "http://127.0.0.1:9222".to_string());
        Self::connect_with_url(&cdp_url).await
    }

    /// Connect with an explicit CDP URL (e.g. per-session browser routing).
    /// ponytail: same as connect_default but takes url instead of env var.
    pub async fn connect_with_url(cdp_url: &str) -> anyhow::Result<Self> {
        Self::connect(&resolve_ws(cdp_url)?).await
    }

    /// Send a command scoped to the ACTIVE tab's session.
    async fn send_cmd(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let session = self.active_session().await;
        self.send_cmd_with(session.as_deref(), method, params).await
    }

    /// Send a command, optionally scoped to a session id (None = browser-level).
    async fn send_cmd_with(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        let mut conn = self.inner.lock().await;
        let id = conn.cmd_id;
        conn.cmd_id += 1;

        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(sid) = session {
            msg["sessionId"] = json!(sid);
        }

        let text = serde_json::to_string(&msg)?;
        conn.write.send(Message::Text(text.into())).await?;

        loop {
            let resp = conn.read.next().await;
            match resp {
                Some(Ok(Message::Text(text))) => {
                    let v: Value = serde_json::from_str(&text)?;
                    if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                        if let Some(err) = v.get("error") {
                            anyhow::bail!("CDP error [{}]: {}", method, err);
                        }
                        return Ok(v.get("result").cloned().unwrap_or(json!({})));
                    }
                    // Event (no id). Track the default execution context for the
                    // session we're driving so later Runtime.evaluate calls land in
                    // the CURRENT document, not a stale pre-navigation one.
                    let ev_session = v.get("sessionId").and_then(|s| s.as_str());
                    if ev_session == session {
                        if let Some(m) = v.get("method").and_then(|m| m.as_str()) {
                            if m == "Runtime.executionContextCreated" {
                                let c = &v["params"]["context"];
                                if c.get("auxData").and_then(|a| a.get("isDefault")).and_then(|d| d.as_bool()) == Some(true) {
                                    if let Some(id) = c.get("id").and_then(|i| i.as_i64()) {
                                        *self.exec_ctx.lock().await = Some(id);
                                    }
                                }
                            } else if m == "Runtime.executionContextsCleared"
                                || m == "Runtime.executionContextDestroyed"
                            {
                                *self.exec_ctx.lock().await = None;
                            } else if m == "Network.requestWillBeSent" {
                                if *self.net_capture.lock().await {
                                    if let Some(u) = v.get("params").and_then(|p| p.get("request")).and_then(|r| r.get("url")).and_then(|u| u.as_str()) {
                                        self.net_urls.lock().await.push(u.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => anyhow::bail!("CDP read error: {e}"),
                None => anyhow::bail!("CDP connection closed"),
            }
        }
    }

    async fn active_session(&self) -> Option<String> {
        let active = self.active.lock().await;
        let tabs = self.tabs.lock().await;
        tabs.get(&*active).and_then(|t| t.session_id.clone())
    }

    /// Allocate the next tab id (monotonic; shared by ensure_page_attached and open_tab).
    async fn next_tab_id(&self) -> String {
        let mut n = self.next_id.lock().await;
        let id = n.to_string();
        *n += 1;
        id
    }

    /// Register an attached tab as the active tab and reset the D1 caches.
    async fn register_tab(&self, id: String, tab: Tab) {
        self.tabs.lock().await.insert(id.clone(), tab);
        *self.active.lock().await = id;
        *self.fp.lock().await = None;
        *self.snap.lock().await = None;
        *self.exec_ctx.lock().await = None;
    }

    async fn eval_js(&self, expression: &str) -> anyhow::Result<Value> {
        let mut params = json!({"expression": expression, "returnByValue": true, "awaitPromise": true});
        if let Some(ctx) = *self.exec_ctx.lock().await {
            params["contextId"] = json!(ctx);
        }
        let result = self
            .send_cmd("Runtime.evaluate", params)
            .await?;
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn ensure_page_attached(&self) -> anyhow::Result<()> {
        if !self.tabs.lock().await.is_empty() {
            return Ok(());
        }
        // Browser-level: pick the first page target (or create one) and attach it as tab "0".
        let targets = self
            .send_cmd_with(None, "Target.getTargets", json!({}))
            .await?;
        let pages: Vec<&Value> = targets
            .get("targetInfos")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|t| t.get("type").and_then(|s| s.as_str()) == Some("page"))
                    .collect()
            })
            .unwrap_or_default();

        let (target_id, url) = if let Some(page) = pages.first() {
            (
                page["targetId"].as_str().unwrap_or("").to_string(),
                page["url"].as_str().unwrap_or("").to_string(),
            )
        } else {
            let r = self
                .send_cmd_with(None, "Target.createTarget", json!({"url": "about:blank"}))
                .await?;
            (
                r["targetId"].as_str().unwrap_or("").to_string(),
                "about:blank".to_string(),
            )
        };

        if target_id.is_empty() {
            anyhow::bail!("No page targets available");
        }

        let session_id = self.attach_and_init(&target_id).await?;
        let id = self.next_tab_id().await;
        self.register_tab(id, Tab { target_id, session_id: Some(session_id), url })
            .await;
        Ok(())
    }

    /// Attach to a page target (browser-level) and enable Runtime/Page + inject stealth.
    async fn attach_and_init(&self, target_id: &str) -> anyhow::Result<String> {
        let result = self
            .send_cmd_with(
                None,
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
            )
            .await?;
        let sid = result["sessionId"]
            .as_str()
            .context("No sessionId from attachToTarget")?
            .to_string();
        self.send_cmd_with(Some(&sid), "Runtime.enable", json!({})).await?;
        self.send_cmd_with(Some(&sid), "Page.enable", json!({})).await?;
        // Block trackers/analytics/fingerprinting hosts before they load.
        self.send_cmd_with(Some(&sid), "Network.setBlockedURLs", json!({ "urls": BLOCKED_URLS }))
            .await?;
        // Stealth: mask automation markers before any page script runs.
        self.send_cmd_with(
            Some(&sid),
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": STEALTH_JS}),
        )
        .await?;
        Ok(sid)
    }

    /// Open a new blank tab, attach, and make it active. Returns the tab id.
    /// ponytail: blank tab so the caller's `navigate(url)` is the ONE load — the
    /// old pre-load here made every batch op fetch each URL twice.
    pub async fn open_tab(&self, url: &str) -> anyhow::Result<String> {
        let r = self
            .send_cmd_with(None, "Target.createTarget", json!({"url": "about:blank"}))
            .await?;
        let target_id = r["targetId"]
            .as_str()
            .context("No targetId from createTarget")?
            .to_string();
        let session_id = self.attach_and_init(&target_id).await?;
        let id = self.next_tab_id().await;
        self.register_tab(
            id.clone(),
            Tab { target_id, session_id: Some(session_id), url: url.to_string() },
        )
        .await;
        Ok(id)
    }

    /// Make an existing tab active; its page receives subsequent commands.
    pub async fn activate_tab(&self, id: &str) -> anyhow::Result<()> {
        if !self.tabs.lock().await.contains_key(id) {
            anyhow::bail!("tab '{id}' not found");
        }
        *self.active.lock().await = id.to_string();
        *self.fp.lock().await = None;
        *self.snap.lock().await = None;
        Ok(())
    }

    /// CDP session id for a tab — lets a concurrent task drive its OWN tab via
    /// `send_cmd_with(Some(sid), ...)` instead of racing on the global active tab.
    pub async fn tab_session(&self, id: &str) -> anyhow::Result<String> {
        self.tabs
            .lock()
            .await
            .get(id)
            .and_then(|t| t.session_id.clone())
            .context("tab not found / no session")
    }

    /// Navigate a specific tab (session-scoped, not the global active) and wait
    /// for interactive/complete. Pages load in parallel in the browser across tabs.
    pub async fn navigate_session(&self, sid: &str, url: &str) -> anyhow::Result<()> {
        self.navigate_session_opts(sid, url, &NavOpts::default()).await
    }

    /// Same as `navigate_session` but honors `NavOpts` (resource blocking, network
    /// idle, wait_selector). Single shared root — batch and session tools both route
    /// through here so a wait fix covers every caller.
    pub async fn navigate_session_opts(&self, sid: &str, url: &str, opts: &NavOpts) -> anyhow::Result<()> {
        self.apply_blocking(Some(sid), opts).await?;
        self.send_cmd_with(Some(sid), "Page.navigate", json!({"url": url}))
            .await?;
        // ponytail: eval_session omits contextId (default ctx), so no stale ctx here.
        let start = std::time::Instant::now();
        loop {
            let rs: String = self
                .eval_session(sid, "document.readyState")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            if rs == "interactive" || rs == "complete" || start.elapsed().as_secs() > 15 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        self.wait_for_conditions(Some(sid), opts).await?;
        Ok(())
    }

    /// (Re)apply the network block list; adds resource-type patterns when
    /// `disable_resources` is on and the 3500-domain tracker list when `block_trackers`
    /// is on. Blocks before navigate so requests never start.
    async fn apply_blocking(&self, sid: Option<&str>, opts: &NavOpts) -> anyhow::Result<()> {
        let mut urls: Vec<&str> = BLOCKED_URLS.to_vec();
        if opts.disable_resources {
            urls.extend_from_slice(RESOURCE_PATTERNS);
        }
        if opts.block_trackers {
            urls.extend_from_slice(tracker_domains());
        }
        self.send_cmd_with(sid, "Network.setBlockedURLs", json!({ "urls": urls }))
            .await?;
        Ok(())
    }

    /// Post-navigate quality waits: network idle + wait_selector(state). Zero-LLM,
    /// pure JS polls via the scoped eval. ponytail: one helper, both paths use it.
    async fn wait_for_conditions(&self, sid: Option<&str>, opts: &NavOpts) -> anyhow::Result<()> {
        // network_idle: resource-entry count stops growing for ~4 consecutive polls.
        if opts.network_idle {
            let mut last: i64 = -1;
            let mut stable = 0u32;
            let start = std::time::Instant::now();
            loop {
                let n: i64 = self
                    .eval_scoped(sid, "performance.getEntriesByType('resource').length")
                    .await?
                    .as_i64()
                    .unwrap_or(last);
                if n == last {
                    stable += 1;
                } else {
                    stable = 0;
                    last = n;
                }
                if stable >= 4 || start.elapsed().as_secs() > 15 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        // wait_selector: poll until the element reaches the requested state.
        if let Some(sel) = &opts.wait_selector {
            let state = if opts.wait_selector_state.is_empty() {
                "visible"
            } else {
                &opts.wait_selector_state
            };
            let js = format!(
                r#"(function() {{
                    const el = document.querySelector({sel:?});
                    if (!el) return 'detached';
                    if (el.offsetParent !== null) return 'visible';
                    return 'hidden';
                }})()"#,
                sel = sel
            );
            let start = std::time::Instant::now();
            loop {
                let got: String = self
                    .eval_scoped(sid, &js)
                    .await?
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let done = match state {
                    "attached" => got != "detached",
                    "visible" => got == "visible",
                    "hidden" => got == "hidden",
                    "detached" => got == "detached",
                    _ => got == state,
                };
                if done || start.elapsed().as_secs() > 20 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        Ok(())
    }

    /// Evaluate in a session (if any) else the active page — shared by wait helpers.
    async fn eval_scoped(&self, sid: Option<&str>, expr: &str) -> anyhow::Result<Value> {
        match sid {
            Some(s) => self.eval_session(s, expr).await,
            None => self.eval_js(expr).await,
        }
    }

    /// Evaluate JS in a specific tab, scoped to its session's default context
    /// (no global exec_ctx — safe for concurrent tabs).
    pub async fn eval_session(&self, sid: &str, expr: &str) -> anyhow::Result<Value> {
        let result = self
            .send_cmd_with(
                Some(sid),
                "Runtime.evaluate",
                json!({"expression": expr, "returnByValue": true, "awaitPromise": true}),
            )
            .await?;
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Full-page screenshot of a specific tab (session-scoped).
    pub async fn screenshot_session(&self, sid: &str, full_page: bool) -> anyhow::Result<Vec<u8>> {
        let result = self
            .send_cmd_with(
                Some(sid),
                "Page.captureScreenshot",
                json!({"format": "png", "fullPage": full_page}),
            )
            .await?;
        let b64 = result["data"].as_str().context("No screenshot data")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("Failed to decode screenshot base64")
    }

    /// Close a tab by id.
    pub async fn close_tab(&self, id: &str) -> anyhow::Result<()> {
        let target_id = self.tabs.lock().await.get(id).map(|t| t.target_id.clone());
        if let Some(tid) = target_id {
            let _ = self
                .send_cmd_with(None, "Target.closeTarget", json!({"targetId": tid}))
                .await;
        }
        self.tabs.lock().await.remove(id);
        let active = self.active.lock().await.clone();
        if active == id {
            let next = self
                .tabs
                .lock()
                .await
                .keys()
                .next()
                .cloned()
                .unwrap_or_default();
            *self.active.lock().await = next;
        }
        *self.fp.lock().await = None;
        *self.snap.lock().await = None;
        Ok(())
    }

    /// List open tabs: [{id, url, active}].
    pub async fn list_tabs(&self) -> anyhow::Result<Value> {
        let active = self.active.lock().await.clone();
        let tabs = self.tabs.lock().await;
        let out: Vec<Value> = tabs
            .iter()
            .map(|(id, t)| json!({"id": id, "url": t.url, "active": *id == active}))
            .collect();
        Ok(json!(out))
    }

    /// Capture media-ish URLs seen on the network. With `url`, navigates first and
    /// captures the whole load (the antenna player's JS-loaded .m3u8/.mp4 appear
    /// here even though they're invisible to the performance buffer). Without it,
    /// just captures the tail of the current page's traffic for `wait_ms`.
    /// ponytail: Network.requestWillBeSent buffered by the WS reader; pump with
    /// trivial evals since events are only read while a command is in flight.
    pub async fn capture_media(
        &self,
        url: Option<&str>,
        wait_ms: u64,
    ) -> anyhow::Result<Value> {
        self.ensure_page_attached().await?;
        let sid = self.active_session().await;
        self.send_cmd_with(sid.as_deref(), "Network.enable", json!({}))
            .await?;
        *self.net_capture.lock().await = true;
        self.net_urls.lock().await.clear();

        let start = std::time::Instant::now();
        if let Some(u) = url.filter(|u| !u.is_empty()) {
            let _ = self.navigate(u).await; // its evals pump events into net_urls
        }
        // Pump: keep commands flowing so late XHR/fetch (player) URLs get read.
        let mut waited = start.elapsed().as_millis() as u64;
        while waited < wait_ms {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = self.eval_js("1").await;
            waited = start.elapsed().as_millis() as u64;
        }

        let urls = self.net_urls.lock().await.clone();
        *self.net_capture.lock().await = false;
        let _ = self.send_cmd_with(sid.as_deref(), "Network.disable", json!({})).await;

        let ext = |u: &str| {
            let low = u.to_lowercase();
            // media + browsemind _DOWNLOAD_EXTENSIONS (docs/archives) so
            // webrain_media can feed download_many for "download any file".
            [".m3u8", ".mpd", ".mp4", ".webm", ".mov", ".m4a", ".mp3", ".wav",
             ".pdf", ".zip", ".gz", ".tar", ".7z", ".rar", ".csv",
             ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx"]
                .iter()
                .any(|e| low.contains(e))
        };
        let hint = |u: &str| {
            let low = u.to_lowercase();
            ["player", "video", "media", "manifest", "stream", "videos/", "/api/", ".json"]
                .iter()
                .any(|k| low.contains(k))
        };
        let mut seen = std::collections::HashSet::new();
        let mut media: Vec<String> = Vec::new();
        for u in urls {
            if (ext(&u) || hint(&u)) && seen.insert(u.clone()) {
                media.push(u);
                if media.len() >= 100 {
                    break;
                }
            }
        }
        Ok(json!({"total": seen.len(), "media": media}))
    }
}

/// Inherent navigate honoring `NavOpts` (single shared root with the session
/// path for wait logic). Defaults = trait `navigate` behavior. Kept OUT of the
/// trait impl so callers can pass options without changing the trait contract.
impl CdpBackend {
    /// Navigate the active page with request-quality options.
    pub async fn navigate_opts(&self, url: &str, opts: &NavOpts) -> anyhow::Result<PageState> {
        self.ensure_page_attached().await?;
        // Real navigation invalidates any cached snapshot/D1 fingerprint.
        *self.snap.lock().await = None;
        *self.fp.lock().await = None;

        self.apply_blocking(self.active_session().await.as_deref(), opts)
            .await?;
        self.send_cmd("Page.navigate", json!({"url": url}))
            .await?;

        // ponytail: clear stale execution context so eval_js sends browser-level
        // evaluate until the reader picks up the new Runtime.executionContextCreated.
        *self.exec_ctx.lock().await = None;

        // Queen Reader wait: poll until DOMContentLoaded (interactive), fall back to
        // full load when the page is still sparse (<500 chars). Faster than a fixed
        // sleep; absorbed the old read_fast/webrain_read (one wait strategy, one tool).
        let start = std::time::Instant::now();
        loop {
            let rs: String = self
                .eval_js("document.readyState")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            if rs == "interactive" || rs == "complete" || start.elapsed().as_secs() > 4 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let mut text: String = self
            .eval_js("document.body ? document.body.innerText || '' : ''")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        if text.chars().count() < 500 {
            let start2 = std::time::Instant::now();
            loop {
                let rs: String = self
                    .eval_js("document.readyState")
                    .await?
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if rs == "complete" || start2.elapsed().as_secs() > 6 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            text = self
                .eval_js("document.body ? document.body.innerText || '' : ''")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
        }

        // network_idle + wait_selector (shared with the session path).
        self.wait_for_conditions(None, opts).await?;

        // css_selector narrowing: return only that element's innerText (token saver).
        if let Some(sel) = &opts.css_selector {
            text = self
                .eval_js(&format!(
                    r#"(() => {{ const el = document.querySelector({sel:?}); return el ? el.innerText || '' : ''; }})()"#,
                    sel = sel
                ))
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
        }

        // Extract page state via JS
        let title: String = self
            .eval_js("document.title")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();

        // Get interactive elements
        let elements_val = self.eval_js(ELEMENTS_JS).await?;
        let elements: Vec<InteractiveElement> =
            serde_json::from_value(elements_val).unwrap_or_default();

        let challenge = detect_antibot(&title, &text);
        let links: Vec<String> = serde_json::from_value(self.eval_js(LINKS_JS).await?).unwrap_or_default();
        Ok(PageState {
            url: url.to_string(),
            title,
            text: text.chars().take(PAGE_TEXT_CAP).collect(),
            elements,
            links,
            challenge,
        })
    }
}

#[async_trait::async_trait]
impl BrowserBackend for CdpBackend {
    async fn navigate(&self, url: &str) -> anyhow::Result<PageState> {
        self.navigate_opts(url, &NavOpts::default()).await
    }

    async fn screenshot(&self, full_page: bool) -> anyhow::Result<Vec<u8>> {
        self.ensure_page_attached().await?;
        let result = self
            .send_cmd(
                "Page.captureScreenshot",
                json!({"format": "png", "fullPage": full_page}),
            )
            .await?;
        let b64 = result["data"]
            .as_str()
            .context("No screenshot data")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("Failed to decode screenshot base64")
    }

    async fn evaluate(&self, js: &str) -> anyhow::Result<Value> {
        self.ensure_page_attached().await?;
        self.eval_js(js).await
    }

    async fn click(&self, index: usize) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // Click by index — uses the interactive elements array
        let js = format!(
            r#"(function() {{
                const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
                if (el) {{ el.click(); return 'clicked'; }}
                return 'not found';
            }})()"#
        );
        self.eval_js(&js).await?;
        Ok(())
    }

    async fn type_text(&self, index: usize, text: &str) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
        // Same selector as ELEMENTS_JS/click so indices from navigate/snapshot
        // map 1:1 to type_text (was: input-only list — index mismatch bug).
        let js = format!(
            r#"(function() {{
                const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
                if (!el) return 'not found';
                if (!/input|textarea|select/i.test(el.tagName) && !el.isContentEditable) return 'not an input';
                el.focus();
                el.value = '{escaped}';
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                return 'typed';
            }})()"#
        );
        self.eval_js(&js).await?;
        Ok(())
    }

    async fn scroll(&self, direction: &str) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        let amount = if direction == "down" { 500 } else { -500 };
        self.eval_js(&format!("window.scrollBy(0, {amount})"))
            .await?;
        Ok(())
    }

    async fn get_html(&self) -> anyhow::Result<String> {
        self.ensure_page_attached().await?;
        let html: String = self
            .eval_js("document.documentElement.outerHTML")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(html)
    }

    async fn screenshot_clip(
        &self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> anyhow::Result<Vec<u8>> {
        self.ensure_page_attached().await?;
        let result = self
            .send_cmd(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "clip": {"x": x, "y": y, "width": width, "height": height, "scale": 1},
                    "captureBeyondViewport": true
                }),
            )
            .await?;
        let b64 = result["data"].as_str().context("No screenshot data")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("Failed to decode screenshot base64")
    }

    async fn pdf(&self) -> anyhow::Result<Vec<u8>> {
        self.ensure_page_attached().await?;
        let result = self.send_cmd("Page.printToPDF", json!({})).await?;
        let b64 = result["data"].as_str().context("No PDF data")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("Failed to decode PDF base64")
    }

    async fn snapshot(&self) -> anyhow::Result<PageState> {
        self.ensure_page_attached().await?;
        // D1 fingerprint: element count + djb2 hash of first 4000 text chars.
        let fp_js = r#"
        (() => {
            const t = (document.body ? document.body.innerText || '' : '');
            let h = 5381;
            const s = t.slice(0, 4000);
            for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;
            return [document.querySelectorAll('*').length, h];
        })()
        "#;
        let fp_v = self.eval_js(fp_js).await?;
        let fp = (
            fp_v[0].as_u64().unwrap_or(0) as usize,
            fp_v[1].as_u64().unwrap_or(0) as usize,
        );
        {
            let mut f = self.fp.lock().await;
            if *f == Some(fp) {
                if let Some(state) = self.snap.lock().await.clone() {
                    return Ok(state);
                }
            }
            *f = Some(fp);
        }
        // Full capture
        let url = self
            .eval_js("location.href")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        let title: String = self
            .eval_js("document.title")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        let text: String = self
            .eval_js("document.body ? document.body.innerText || '' : ''")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        let elements: Vec<InteractiveElement> =
            serde_json::from_value(self.eval_js(ELEMENTS_JS).await?).unwrap_or_default();
        let challenge = detect_antibot(&title, &text);
        let links: Vec<String> = serde_json::from_value(self.eval_js(LINKS_JS).await?).unwrap_or_default();
        let state = PageState {
            url,
            title,
            text: text.chars().take(PAGE_TEXT_CAP).collect(),
            elements,
            links,
            challenge,
        };
        *self.snap.lock().await = Some(state.clone());
        Ok(state)
    }

    fn backend_name(&self) -> &'static str {
        "cdp"
    }

    async fn open_tab(&self, url: &str) -> anyhow::Result<String> {
        CdpBackend::open_tab(self, url).await
    }
    async fn activate_tab(&self, id: &str) -> anyhow::Result<()> {
        CdpBackend::activate_tab(self, id).await
    }
    async fn close_tab(&self, id: &str) -> anyhow::Result<()> {
        CdpBackend::close_tab(self, id).await
    }
    async fn list_tabs(&self) -> anyhow::Result<Value> {
        CdpBackend::list_tabs(self).await
    }

    /// Prefetch / discovery-only navigation — fast path that skips innerText
    /// extraction and the full-load fallback (crawl4ai prefetch=True).
    /// ponytail: wait for interactive only, then extract links; no content.
    async fn discover_links(&self, url: &str) -> anyhow::Result<Vec<String>> {
        self.ensure_page_attached().await?;
        *self.snap.lock().await = None;
        *self.fp.lock().await = None;
        self.send_cmd("Page.navigate", json!({"url": url})).await?;
        *self.exec_ctx.lock().await = None;
        let start = std::time::Instant::now();
        loop {
            let rs: String = self
                .eval_js("document.readyState")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            if rs == "interactive" || rs == "complete" || start.elapsed().as_secs() > 4 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let v = self
            .eval_js("Array.from(document.querySelectorAll('a[href]')).map(a => a.href)")
            .await?;
        let mut out = vec![];
        if let Some(arr) = v.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push(s.to_string());
                }
            }
        }
        Ok(out)
    }

    /// Accessibility-tree snapshot with CSS paths / XPaths for precise extraction.
    /// role/name/value from the AX tree; css_path/xpath resolved from the DOM tree
    /// so extraction (get_html by selector / CSS-schema / regex) can target exactly.
    async fn a11y(&self) -> anyhow::Result<Value> {
        self.ensure_page_attached().await?;
        let tree = self.send_cmd("Accessibility.getFullAXTree", json!({})).await?;
        let nodes = tree["nodes"].as_array().cloned().unwrap_or_default();

        // One DOM.getDocument walk maps backendNodeId -> (css_path, xpath).
        let mut paths: std::collections::HashMap<i64, (String, String)> = Default::default();
        if let Ok(doc) = self
            .send_cmd("DOM.getDocument", json!({"depth": -1, "pierce": true}))
            .await
        {
            if let Some(children) = doc.get("root").and_then(|r| r.get("children")).and_then(|c| c.as_array()) {
                build_dom_paths(children, "", "", &mut paths);
            }
        }

        let flat: Vec<Value> = nodes
            .iter()
            .filter_map(|n| {
                let role = n
                    .get("role")
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let name = n
                    .get("name")
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let value = n
                    .get("value")
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if role.is_empty() && name.is_empty() {
                    return None;
                }
                let mut m = json!({"role": role, "name": name, "value": value});
                if let Some(bid) = n.get("backendDOMNodeId").and_then(|v| v.as_i64()) {
                    if let Some((css, xp)) = paths.get(&bid) {
                        m["css_path"] = json!(css);
                        m["xpath"] = json!(xp);
                    }
                }
                Some(m)
            })
            .collect();
        Ok(json!(flat))
    }
}

/// Walk a DOM children slice, emitting a css_path (nth-of-type chain, id-shortcut)
/// and an xpath (tag[n]) for every element, keyed by backendNodeId.
fn build_dom_paths(
    children: &[Value],
    css_prefix: &str,
    xpath_prefix: &str,
    out: &mut std::collections::HashMap<i64, (String, String)>,
) {
    let mut by_tag: std::collections::HashMap<String, usize> = Default::default();
    for child in children {
        let Some(name) = child.get("nodeName").and_then(|v| v.as_str()) else {
            continue;
        };
        if child.get("nodeType").and_then(|v| v.as_u64()) != Some(1) {
            continue;
        }
        let tag = name.to_ascii_lowercase();
        let n = by_tag.entry(tag.clone()).or_insert(0);
        *n += 1;

        let mut id = "";
        if let Some(attrs) = child.get("attributes").and_then(|a| a.as_array()) {
            for pair in attrs.chunks(2) {
                if pair.len() == 2 && pair[0].as_str() == Some("id") {
                    if let Some(v) = pair[1].as_str() {
                        if !v.is_empty() {
                            id = v;
                        }
                    }
                }
            }
        }
        let css_seg = if id.is_empty() {
            format!("{tag}:nth-of-type({n})")
        } else {
            format!("#{}", css_escape(id))
        };
        let css = if css_prefix.is_empty() {
            css_seg.clone()
        } else {
            format!("{css_prefix} > {css_seg}")
        };
        let xp = format!("{xpath_prefix}/{tag}[{n}]");

        if let Some(bid) = child.get("backendNodeId").and_then(|v| v.as_i64()) {
            out.insert(bid, (css.clone(), xp.clone()));
        }
        if let Some(grand) = child.get("children").and_then(|c| c.as_array()) {
            build_dom_paths(grand, &css, &xp, out);
        }
    }
}

/// CSS-safe id: plain [A-Za-z0-9_-] as-is, otherwise an attribute selector.
fn css_escape(id: &str) -> String {
    if id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        id.to_string()
    } else {
        format!("[id={id:?}]")
    }
}
