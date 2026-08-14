// CDP WebSocket backend — connects to any Chrome-compatible CDP endpoint.
// Works with: obscura serve, lightpanda serve, Chrome --remote-debugging-port.
//
// ponytail: single connection, one page, synchronous CDP command/response per call.

use crate::browser::{
    BrowserBackend, InteractiveElement, PageState, detect_antibot, detect_chrome_error,
    detect_crippled,
};
use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

/// Anti-bot JS injected before page scripts run (Page.addScriptToEvaluateOnNewDocument).
/// Self-destructing by design: everything lives inside the IIFE, so no `window.setXxx`
/// helper survives for page scripts to detect (camoufox addInitScript pattern).
///
/// CORE is always on (webdriver→false, real plugins, window.chrome, Function.toString
/// spoof — the bits Cloudflare/Google don't punish). NOISE is ON by default:
/// ponytail: canvas/audio/WebGL spoofs + hardwareConcurrency/deviceMemory/connection
/// lies — the managed Cloudflare challenge (scrapingcourse cf-antibot) measures
/// exactly these, and real values leave it stuck on "Just a moment…" (verified:
/// the stealth-noise profile applies them always and passes). Real Chrome's
/// real fingerprint alone is NOT CF-managed-challenge-trustworthy. Set
/// WEBRAIN_STEALTH_NOISE=0 to opt out for sites that distrust the spoofed profile.
fn stealth_js() -> String {
    const CORE: &str = r#"(() => {
  const apply = (obj, prop, val) => {
    try { Object.defineProperty(obj, prop, { get: () => val, configurable: true }); } catch (e) {}
  };
  apply(Navigator.prototype, 'webdriver', false);
  apply(Navigator.prototype, 'languages', ['en-US', 'en']);
  // Real fake PluginArray built on the REAL prototype — Array.isArray stays false
  // (a plain array here is the classic detectable leak).
  try {
    const proto = Object.getPrototypeOf(navigator.plugins);
    const names = [['PDF Viewer', 'Portable Document Format'], ['Chrome PDF Viewer', ''], ['Chromium PDF Viewer', ''], ['Internal PDF Viewer', ''], ['Microsoft Edge PDF Viewer', '']];
    const arr = Object.create(proto);
    names.forEach(([n, d], i) => {
      const p = Object.create(proto);
      p.name = n; p.description = d; p.filename = n + '.dll'; p.length = 0;
      arr[i] = p;
    });
    arr.length = names.length;
    arr.item = (i) => arr[i] || null;
    arr.namedItem = (n) => Array.from(arr).find((p) => p.name === n) || null;
    apply(Navigator.prototype, 'plugins', arr);
    apply(Navigator.prototype, 'mimeTypes', arr);
  } catch (e) {}
  apply(Navigator.prototype, 'maxTouchPoints', 1);
  apply(Navigator.prototype, 'platform', 'Win32');
  apply(Navigator.prototype, 'vendor', 'Google Inc.');
  apply(Navigator.prototype, 'oscpu', 'Windows NT 10.0; Win64; x64');
  /*STEALTH_NOISE*/
  // Full window.chrome stub (app/runtime/csi/loadTimes) — the `{runtime:{}}`
  // one is detectable (chrome.app/csi/loadTimes missing).
  try {
    window.chrome = {
      app: { isInstalled: false, InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' }, RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' } },
      csi: () => {}, loadTimes: () => {},
      runtime: { OnInstalledReason: {}, OnRestartRequiredReason: {}, PlatformArch: {}, PlatformNaclArch: {}, PlatformOs: {}, RequestUpdateCheckStatus: {} },
    };
  } catch (e) {}
  // permissions.query: 'notifications' reflects Notification.permission.
  try {
    const q = navigator.permissions.query.bind(navigator.permissions);
    navigator.permissions.query = (p) => (p && p.name === 'notifications')
      ? Promise.resolve({ state: Notification.permission, onchange: null })
      : q(p);
  } catch (e) {}
  // Function.prototype.toString native spoof (uc): permissions.query and
  // friends must read as `[native code]`, not a wrapped source.
  try {
    const oldCall = Function.prototype.call;
    const oldToString = Function.prototype.toString;
    const nativeStr = Error.toString().replace(/Error/g, 'toString');
    Function.prototype.toString = function () {
      if (this === Function.prototype.toString) return nativeStr;
      if (this === navigator.permissions.query) return 'function query() { [native code] }';
      return oldCall.call(oldToString, this);
    };
  } catch (e) {}
})();"#;
    // Opt-in fingerprint noise — only for foiling third-party hash fingerprinters.
    // Turnstile + reCAPTCHA measure these exact signals; keep them REAL by default.
    const NOISE: &str = r#"
  apply(Navigator.prototype, 'hardwareConcurrency', 8);
  apply(Navigator.prototype, 'deviceMemory', 8);
  apply(Navigator.prototype, 'connection', { effectiveType: '4g', rtt: 100, downlink: 10, saveData: false });
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
  // window.outerdimensions (puppeteer-extra evasion): report a consistent
  // outer/inner size so window.outerWidth - innerWidth isn't a headless tell.
  try {
    apply(window, 'outerWidth', window.innerWidth);
    apply(window, 'outerHeight', window.innerHeight + 72);
    apply(window, 'outerHeight', Math.max(window.outerHeight, 700));
  } catch (e) {}
  // iframe.contentWindow (puppeteer-extra evasion): the HEADCHR_IFRAME check —
  // cross-origin iframe.contentWindow.chrome must not be undefined. Patchright/
  // stealth_sync include this; a missing chrome in the widget iframe is a
  // bot signal Cloudflare's Turnstile widget checks.
  try {
    const _cwProxy = new Proxy(window, {
      get(t, k) { if (k === 'self') return window; if (k === 'frameElement') return null; return Reflect.get(t, k); }
    });
    const _oDefine = Object.defineProperty;
    Object.defineProperty = function (o, p, d) {
      if (o && o.tagName === 'IFRAME' && p === 'contentWindow' && !o.contentWindow) {
        d.get = () => _cwProxy;
      }
      return _oDefine.call(this, o, p, d);
    };
  } catch (e) {}
  // media.codecs (puppeteer-extra evasion): Chromium reports 'maybe' for
  // proprietary codecs a real Chrome reports 'probably' — the exact check
  // fingerprinters use (avc1.42E01E / mp4).
  try {
    const _cpt = HTMLMediaElement.prototype.canPlayType;
    HTMLMediaElement.prototype.canPlayType = function (arg) {
      const s = String(arg || '').trim();
      if (s.startsWith('video/mp4') && s.includes('avc1.42E01E')) return 'probably';
      if (s.startsWith('audio/x-m4a') && !s.includes('codecs')) return 'maybe';
      return _cpt.call(this, arg);
    };
  } catch (e) {}"#;
    static NOISE_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on = *NOISE_ON.get_or_init(|| {
        // Default ON: the managed Cloudflare challenge (scrapingcourse cf-antibot)
        // checks hardwareConcurrency + WebGL vendor — real values get it stuck on
        // "Just a moment…". The noise evasions are applied always. Opt out with
        // WEBRAIN_STEALTH_NOISE=0 for the rare site that distrusts the spoofed profile.
        std::env::var("WEBRAIN_STEALTH_NOISE")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true)
    });
    if on {
        CORE.replace("/*STEALTH_NOISE*/", NOISE)
    } else {
        CORE.replace("/*STEALTH_NOISE*/", "")
    }
}

/// Trackers/analytics/fingerprinting hosts blocked at the network layer before
/// they load (obscura/camofox-browser pattern). CDP `Network.setBlockedURLs`
/// wildcard patterns; blocking these never breaks page function, only tracking.
const BLOCKED_URLS: &[&str] = &[
    "*google-analytics.com*",
    "*googletagmanager.com*",
    "*googlesyndication.com*",
    "*doubleclick.net*",
    "*facebook.net*",
    "*facebook.com/tr*",
    "*ads-twitter.com*",
    "*hotjar.com*",
    "*newrelic.com*",
    "*mixpanel.com*",
    "*segment.io*",
    "*amplitude.com*",
    "*intercomcdn.com*",
    "*scorecardresearch.com",
    "*criteo.com*",
    "*taboola.com*",
    "*outbrain.com*",
    "*quantserve.com",
    "*chartbeat.com*",
    "*fullstory.com*",
    "*mouseflow.com*",
    "*crazyegg.com*",
    "*snap.licdn.com*",
    "*linkedin.com/analytics*",
    "*gtag/js*",
    "*/gtm.js*",
    "*/analytics.js*",
    "*/ga.js*",
];

/// Resource-type patterns added to the block list when `disable_resources` is on
/// (Scrapling's font/image/media/stylesheet drop — speeds loads, saves tokens).
const RESOURCE_PATTERNS: &[&str] = &[
    "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.avif", "*.svg", "*.ico", "*.css", "*.woff",
    "*.woff2", "*.ttf", "*.otf", "*.eot", "*.mp4", "*.webm", "*.mp3", "*.ogg", "*.wav", "*.m4a",
    "*.pdf",
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
#[derive(Default, Clone, Debug)]
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
    /// Max seconds to wait for readyState + conditions before returning (default 20).
    pub wait_timeout_secs: Option<u64>,
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
                text: (el.type === 'password' ? '' : (el.textContent || el.value || '')).trim().substring(0, 80),
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
    let ws = body["webSocketDebuggerUrl"]
        .as_str()
        .map(|s| s.to_string())
        .context("No webSocketDebuggerUrl in response")?;
    // Docker/port-forward: the advertised ws URL can carry the container-INTERNAL
    // host:port (obscura maps host 9224 -> container 9222, so /json/version says
    // ws://127.0.0.1:9222/...). ws is served on the same endpoint we reached, so
    // rewrite to http_url's host:port and keep the path.
    let mut out = ws.clone();
    if let (Ok(u), Ok(mut w)) = (url::Url::parse(http_url), url::Url::parse(&ws)) {
        if let Some(host) = u.host_str() {
            if w.set_host(Some(host)).is_ok() {
                let _ = w.set_port(u.port());
                let scheme = if u.scheme() == "https" { "wss" } else { "ws" };
                let _ = w.set_scheme(scheme);
                out = w.to_string();
            }
        }
    }
    Ok(out)
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
    /// Network capture: while net_capture is set, requestWillBeSent URLs are
    /// buffered in net_urls (browsemind NetworkCapture pattern).
    net_capture: Arc<Mutex<bool>>,
    net_urls: Arc<Mutex<Vec<String>>>,
    /// User-registered init scripts (agent-browser `--init-script` borrow):
    /// replayed via Page.addScriptToEvaluateOnNewDocument in attach_and_init so
    /// they run before every new document on every tab.
    init_scripts: Arc<Mutex<Vec<String>>>,
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
        // ponytail: 5s fail-fast on the WS handshake (half-open-port guard), but
        // obscura's CDP server can finish its handshake slowly on a cold start —
        // retry once with a longer budget before giving up.
        let (ws, _) = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        {
            Ok(r) => r.context("CDP WebSocket connection failed. Is the browser running?")?,
            Err(_) => tokio::time::timeout(
                std::time::Duration::from_secs(20),
                tokio_tungstenite::connect_async(ws_url),
            )
            .await
            .map_err(|_| anyhow::anyhow!("CDP connect timed out after 20s (retry)"))?
            .context("CDP WebSocket connection failed. Is the browser running?")?,
        };

        let (write, read) = ws.split();

        let inner = CdpConnection {
            write,
            read,
            cmd_id: 1,
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            tabs: Arc::new(Mutex::new(HashMap::new())),
            active: Arc::new(Mutex::new(String::new())),
            next_id: Arc::new(Mutex::new(0)),
            fp: Arc::new(Mutex::new(None)),
            snap: Arc::new(Mutex::new(None)),
            net_capture: Arc::new(Mutex::new(false)),
            net_urls: Arc::new(Mutex::new(Vec::new())),
            init_scripts: Arc::new(Mutex::new(Vec::new())),
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

    /// All cookies for the browser context (includes HttpOnly, which
    /// document.cookie can't see). Used for logged_in detection + export.
    ///
    /// `Network.getCookies` is deprecated and returns empty on recent Chrome
    /// page sessions, so prefer browser-level `Storage.getCookies` first and
    /// fall back to Network on the active page session.
    pub async fn cookies(&self) -> anyhow::Result<Vec<Value>> {
        if let Ok(r) = self
            .send_cmd_with(None, "Storage.getCookies", json!({}))
            .await
        {
            let c = r["cookies"].as_array().cloned().unwrap_or_default();
            if !c.is_empty() {
                return Ok(c);
            }
        }
        let r = self.send_cmd("Network.getCookies", json!({})).await?;
        Ok(r["cookies"].as_array().cloned().unwrap_or_default())
    }

    /// Set cookies (Network.setCookies) — cross-browser session migration:
    /// log in in Chrome, export via `cookies()`, import into obscura/lightpanda
    /// with this. Only the fields setCookies accepts are forwarded.
    pub async fn set_cookies(&self, cookies: &[Value]) -> anyhow::Result<()> {
        let clean: Vec<Value> = cookies
            .iter()
            .map(|c| {
                let mut o = serde_json::Map::new();
                for k in [
                    "name", "value", "domain", "path", "expires", "httpOnly", "secure", "sameSite",
                    "priority",
                ] {
                    if let Some(v) = c.get(k) {
                        o.insert(k.to_string(), v.clone());
                    }
                }
                Value::Object(o)
            })
            .collect();
        self.send_cmd("Network.setCookies", json!({ "cookies": clean }))
            .await?;
        Ok(())
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
                    // Event (no id). Network capture: while net_capture is set,
                    // requestWillBeSent URLs are buffered (browsemind pattern).
                    let ev_session = v.get("sessionId").and_then(|s| s.as_str());
                    if ev_session == session {
                        if let Some(m) = v.get("method").and_then(|m| m.as_str()) {
                            if m == "Network.requestWillBeSent" {
                                if *self.net_capture.lock().await {
                                    if let Some(u) = v
                                        .get("params")
                                        .and_then(|p| p.get("request"))
                                        .and_then(|r| r.get("url"))
                                        .and_then(|u| u.as_str())
                                    {
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
    }

    async fn eval_js(&self, expression: &str) -> anyhow::Result<Value> {
        // ponytail: NO contextId — same as eval_session. The exec_ctx tracking
        // (for Chrome's stale pre-navigation context) misfires on lightpanda:
        // a Turbo-style client re-render fires a SECOND isDefault context that
        // is empty, poisoning every later eval. Callers wait for interactive/
        // complete before extracting, so the browser default context is live.
        let result = self
            .send_cmd(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
            )
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
        self.register_tab(
            id,
            Tab {
                target_id,
                session_id: Some(session_id),
                url,
            },
        )
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
        // ponytail: NO Runtime.enable — patchright's "biggest leak": Runtime.enable
        // marks the debugger as attached (detectable; a real browser has none).
        // Runtime.evaluate works without it (enable only subscribes to
        // consoleAPICalled / executionContextCreated events, which we don't
        // consume — eval already omits contextId). Keep Page.enable for page events.
        self.send_cmd_with(Some(&sid), "Page.enable", json!({}))
            .await?;
        // Stealth at the CDP level: clear the automation flag only. NO
        // Network.setUserAgentOverride — patchright's documented best practice is
        // "do NOT add custom browser headers or user_agent": a forged UA
        // (Chrome/151.0.0.0) mismatches the real browser build's TLS fingerprint +
        // Sec-CH-UA, which Cloudflare flags ("verification failed"). Real Chrome
        // speaks its own, internally-consistent UA. Network.enable stays for
        // blocked-urls / cookie plumbing (best-effort; lightpanda may skip it).
        let _ = self
            .send_cmd_with(Some(&sid), "Network.enable", json!({}))
            .await;
        // ponytail: NO Emulation.setAutomationOverride — patchright's driver
        // patch list has zero calls to it, and it's a detectable CDP automation
        // signal (real Chrome never sets it). navigator.webdriver is already
        // masked by stealth_js(); the launch flag --disable-blink-features=
        // AutomationControlled clears the command-flag leak. Leave the emulation
        // domain untouched — that's what patchright does.
        // Block trackers/analytics/fingerprinting hosts before they load.
        // ponytail: adaptive shape — lightpanda wants urlPatterns, Chrome wants urls.
        self.set_blocked_urls(Some(&sid), &BLOCKED_URLS).await?;
        // Stealth: mask automation markers before any page script runs.
        self.send_cmd_with(
            Some(&sid),
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": stealth_js()}),
        )
        .await?;
        // User init scripts (agent-browser --init-script borrow): replay on every
        // newly attached tab so they survive per-tab session creation. Best-effort
        // (a bad user script must not kill the attach).
        for js in self.init_scripts.lock().await.iter() {
            let _ = self
                .send_cmd_with(
                    Some(&sid),
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({"source": js}),
                )
                .await;
        }
        Ok(sid)
    }

    /// Register a page init script (agent-browser `--init-script` borrow): runs
    /// via Page.addScriptToEvaluateOnNewDocument before every FUTURE navigation
    /// (new documents only — already-loaded pages aren't rewritten). Push to the
    /// shared list (replayed in attach_and_init for every new tab) + register on
    /// the active session immediately.
    pub async fn add_init_script(&self, js: &str) -> anyhow::Result<()> {
        self.init_scripts.lock().await.push(js.to_string());
        if let Some(sid) = self.active_session().await {
            let _ = self
                .send_cmd_with(
                    Some(&sid),
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({"source": js}),
                )
                .await;
        }
        Ok(())
    }

    /// Id of the first registered tab, if any (single-target reuse).
    pub async fn existing_tab(&self) -> Option<String> {
        self.tabs.lock().await.keys().next().cloned()
    }

    /// Detect single-target mode (lightpanda serve) by probing
    /// `Target.createTarget` directly — bypasses open_tab so it sees the raw
    /// CDP behavior. lightpanda holds ONE browser context; its 2nd createTarget
    /// errors `TargetAlreadyLoaded` (src/cdp/domains/target.zig). Scratch
    /// targets are closed before returning so the caller starts clean.
    pub async fn single_target_probe(&self) -> anyhow::Result<bool> {
        let first = self
            .send_cmd_with(None, "Target.createTarget", json!({"url": "about:blank"}))
            .await;
        let (tid, first_err) = match first {
            Ok(r) => (r["targetId"].as_str().unwrap_or("").to_string(), None),
            Err(e) => (String::new(), Some(e)),
        };
        if let Some(e) = first_err {
            // A target already exists (e.g. from a prior navigate) → single-target.
            return Ok(e.to_string().contains("TargetAlreadyLoaded"));
        }
        match self
            .send_cmd_with(None, "Target.createTarget", json!({"url": "about:blank"}))
            .await
        {
            Ok(r2) => {
                if let Some(t2) = r2["targetId"].as_str() {
                    let _ = self
                        .send_cmd_with(None, "Target.closeTarget", json!({"targetId": t2}))
                        .await;
                }
                if !tid.is_empty() {
                    let _ = self
                        .send_cmd_with(None, "Target.closeTarget", json!({"targetId": tid}))
                        .await;
                }
                Ok(false)
            }
            Err(e) => {
                if !tid.is_empty() {
                    let _ = self
                        .send_cmd_with(None, "Target.closeTarget", json!({"targetId": tid}))
                        .await;
                }
                if e.to_string().contains("TargetAlreadyLoaded") {
                    Ok(true)
                } else {
                    Err(e)
                }
            }
        }
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
            Tab {
                target_id,
                session_id: Some(session_id),
                url: url.to_string(),
            },
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
        self.navigate_session_opts(sid, url, &NavOpts::default())
            .await
    }

    /// Same as `navigate_session` but honors `NavOpts` (resource blocking, network
    /// idle, wait_selector). Single shared root — batch and session tools both route
    /// through here so a wait fix covers every caller.
    pub async fn navigate_session_opts(
        &self,
        sid: &str,
        url: &str,
        opts: &NavOpts,
    ) -> anyhow::Result<()> {
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
            let cap = opts.wait_timeout_secs.unwrap_or(20);
            if rs == "interactive" || rs == "complete" || start.elapsed().as_secs() > cap {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        self.wait_for_conditions(Some(sid), opts).await?;
        Ok(())
    }

    /// Send Network.setBlockedURLs, adapting to backend shape: Chrome/obscura
    /// take `urls: [string]`, lightpanda takes `urlPatterns: [{urlPattern, block}]`.
    /// ponytail: try standard first, retry with lightpanda's shape on MissingField.
    async fn set_blocked_urls(&self, sid: Option<&str>, urls: &[&str]) -> anyhow::Result<()> {
        match self
            .send_cmd_with(sid, "Network.setBlockedURLs", json!({ "urls": urls }))
            .await
        {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("MissingField") => {
                let patterns: Vec<Value> = urls
                    .iter()
                    .map(|u| json!({ "urlPattern": u, "block": true }))
                    .collect();
                self.send_cmd_with(
                    sid,
                    "Network.setBlockedURLs",
                    json!({ "urlPatterns": patterns }),
                )
                .await
                .map(|_| ())
            }
            Err(e) => Err(e),
        }
    }

    /// (Re)apply the network block list; adds resource-type patterns when
    /// `disable_resources` is on and the 3500-domain tracker list when `block_trackers`
    /// is on. Blocks before navigate so requests never start.
    /// ponytail: the base BLOCKED_URLS is already set once at attach_and_init, so
    /// with default opts this is a no-op — saves a redundant CDP round-trip per page.
    async fn apply_blocking(&self, sid: Option<&str>, opts: &NavOpts) -> anyhow::Result<()> {
        if opts.disable_resources || opts.block_trackers {
            let mut urls: Vec<&str> = BLOCKED_URLS.to_vec();
            if opts.disable_resources {
                urls.extend_from_slice(RESOURCE_PATTERNS);
            }
            if opts.block_trackers {
                urls.extend_from_slice(tracker_domains());
            }
            self.set_blocked_urls(sid, &urls).await?;
        }
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
                let cap = opts.wait_timeout_secs.unwrap_or(20);
                if stable >= 4 || start.elapsed().as_secs() > cap {
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
                let cap = opts.wait_timeout_secs.unwrap_or(20);
                if done || start.elapsed().as_secs() > cap {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        Ok(())
    }

    /// Wait out a Cloudflare/interactive anti-bot interstitial (the Python
    /// sidecar's poll+reload loop, native): poll title+text for challenge
    /// markers, reload every ~15s to re-kick the proof, cap at `secs`. Returns
    /// true when the challenge cleared. On a normal page returns immediately.
    /// ponytail: marker set is detect_antibot's; challenge layouts that need
    /// HTML markers go there, not here.
    pub async fn wait_out_challenge(&self, secs: u64) -> bool {
        let start = std::time::Instant::now();
        let mut last_reload = std::time::Instant::now();
        loop {
            let title: String = self
                .eval_js("document.title")
                .await
                .map(|v| v.as_str().unwrap_or("").to_string())
                .unwrap_or_default();
            let text: String = self
                .eval_js("document.body ? document.body.innerText || '' : ''")
                .await
                .map(|v| v.as_str().unwrap_or("").to_string())
                .unwrap_or_default();
            if detect_antibot(&title, &text).is_none() {
                return true;
            }
            if start.elapsed().as_secs() >= secs {
                return false;
            }
            // reload every 15s to re-kick the challenge (__cf_chl_rt_tk rotates
            // per reload).
            if last_reload.elapsed().as_secs() >= 15 {
                let _ = self
                    .send_cmd("Page.reload", json!({"ignoreCache": false}))
                    .await;
                last_reload = std::time::Instant::now();
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    /// Generic captcha widget claiming — Turnstile / reCAPTCHA / hCaptcha
    /// (cf-turnstile login form, Google /sorry block page, hCaptcha gates: a
    /// plain page + a checkbox widget, NO "Just a moment" interstitial). Port
    /// of browsemind's `claim_captcha_widget` + `_WIDGET_CLAIM_JS`:
    ///   1. score ALL visible iframes by overlay traits (fixed/sticky, z-index,
    ///      captcha src pattern, size), JS-click the best one
    ///   2. CDP-click its center via Input.dispatchMouseEvent — crosses the
    ///      cross-origin iframe barrier real reCAPTCHA/Turnstile checkboxes
    ///      live behind (a JS el.click() alone can't reach them)
    /// Then poll until a captcha token populates (`*-response`), or the block
    /// page redirects away (Google /sorry auto-continues on pass). Returns
    /// true when cleared.
    pub async fn wait_turnstile_token(&self, secs: u64) -> bool {
        // browsemind _WIDGET_CLAIM_JS (trimmed): score + click the best captcha
        // iframe, return its center for the cross-origin CDP click.
        const CLAIM_JS: &str = r#"(function() {
  var iframes = document.querySelectorAll('iframe');
  var best = null, bestScore = 0;
  var vw = window.innerWidth, vh = window.innerHeight;
  for (var i = 0; i < iframes.length; i++) {
    var f = iframes[i], r = f.getBoundingClientRect();
    if (r.width < 10 || r.height < 10) continue;
    if (r.top > vh || r.bottom < 0) continue;
    var cs = window.getComputedStyle(f);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    var score = 0;
    var pos = cs.position;
    if (pos === 'fixed') score += 30; else if (pos === 'sticky') score += 20;
    var zi = parseInt(cs.zIndex) || 0;
    if (zi >= 100) score += 20; else if (zi >= 10) score += 10;
    if (r.width >= 60 && r.width <= 500 && r.height >= 60 && r.height <= 600) score += 25;
    if (r.width > vw * 0.8 && r.height > vh * 0.8) score -= 20;
    var src = (f.src || '').toLowerCase();
    if (src.indexOf('google.com/recaptcha') >= 0) score += 10;
    if (src.indexOf('challenges.cloudflare.com') >= 0) score += 10;
    if (src.indexOf('hcaptcha.com') >= 0) score += 10;
    if (f.getAttribute('role') === 'presentation') score += 5;
    if (score > bestScore) { bestScore = score; best = f; }
  }
  if (best) {
    try { best.click(); } catch (e) {}
    var br = best.getBoundingClientRect();
    // reCAPTCHA v2/enterprise anchor (304x78): the checkbox is a 28px box at
    // the TOP-LEFT of the iframe (~27,37 from its origin), NOT the center —
    // center-clicking the anchor hits the "I'm not a robot" text and does
    // nothing. Verified live: checkbox center sits at iframe-left+27,
    // iframe-top+37. Turnstile/hCaptcha widgets keep the center click.
    var src = (best.src || '').toLowerCase();
    var isRecaptchaAnchor = src.indexOf('google.com/recaptcha') >= 0 && br.height >= 70 && br.height <= 90;
    var ox = isRecaptchaAnchor ? 27 : br.width / 2;
    var oy = isRecaptchaAnchor ? 37 : br.height / 2;
    return JSON.stringify({claimed: true, cx: Math.round(br.left + ox), cy: Math.round(br.top + oy)});
  }
  return JSON.stringify({claimed: false});
})()"#;
        let start = std::time::Instant::now();
        let mut claimed = false;
        loop {
            // Any captcha token set => cleared (Turnstile / reCAPTCHA / hCaptcha
            // all write a hidden `*-response` textarea on pass).
            let val: String = self
                .eval_js(
                    r#"(function(){ var t = document.querySelector('textarea[name="cf-turnstile-response"], textarea[name="g-recaptcha-response"], textarea[name="h-captcha-response"]'); return (t && t.value) || ''; })()"#,
                )
                .await
                .map(|v| v.as_str().unwrap_or("").to_string())
                .unwrap_or_default();
            if !val.is_empty() {
                return true;
            }
            // Google /sorry style: the block page auto-redirects on pass — the
            // recaptcha iframe + "unusual traffic" copy vanish from the DOM.
            let left_block: bool = self
                .eval_js(
                    r#"(function(){ var b = document.body ? document.body.innerText : ''; if (location.href.indexOf('/sorry') >= 0 && !document.querySelector('iframe[src*="recaptcha"], .g-recaptcha')) return true; return /unusual traffic/.test(b) === false && location.href.indexOf('/sorry') < 0 && b.indexOf('Just a moment') < 0; })()"#,
                )
                .await
                .map(|v| v.as_bool().unwrap_or(false))
                .unwrap_or(false);
            if left_block && start.elapsed().as_secs() > 2 {
                return true;
            }
            // Not claimed yet: run the widget-claim (JS click + CDP center click).
            if !claimed && start.elapsed().as_secs() < 4 {
                let res: Value = self.eval_js(CLAIM_JS).await.unwrap_or(Value::Null);
                if let Some(obj) = res.as_object() {
                    if obj.get("claimed").and_then(|v| v.as_bool()) == Some(true) {
                        let cx = obj.get("cx").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cy = obj.get("cy").and_then(|v| v.as_i64()).unwrap_or(0);
                        // CDP Input.dispatchMouseEvent — trusted, crosses
                        // cross-origin iframe (real checkbox behind it).
                        let _ = self.click_coords(cx, cy).await;
                        claimed = true;
                    }
                }
                // Fallback nudge: click the widget container div too (some
                // Turnstile builds render the checkbox in-page, not iframe).
                let _ = self
                    .eval_js(
                        r#"(function(){ const w = document.querySelector('#cf-turnstile, .cf-turnstile'); if (w) { w.click(); return true; } return false; })()"#,
                    )
                    .await;
            }
            if start.elapsed().as_secs() >= secs {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    /// Evaluate in a session (if any) else the active page — shared by wait helpers.
    async fn eval_scoped(&self, sid: Option<&str>, expr: &str) -> anyhow::Result<Value> {
        match sid {
            Some(s) => self.eval_session(s, expr).await,
            None => self.eval_js(expr).await,
        }
    }

    /// Evaluate JS in a specific tab, scoped to its session (safe for concurrent tabs).
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
                // ponytail: captureBeyondViewport is the real CDP param; the old
                // "fullPage" was a Playwright abstraction CDP silently ignored.
                json!({"format": "png", "captureBeyondViewport": full_page}),
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
    pub async fn capture_media(&self, url: Option<&str>, wait_ms: u64) -> anyhow::Result<Value> {
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
        let _ = self
            .send_cmd_with(sid.as_deref(), "Network.disable", json!({}))
            .await;

        let ext = |u: &str| {
            let low = u.to_lowercase();
            // media + browsemind _DOWNLOAD_EXTENSIONS (docs/archives) so
            // webrain_media can feed download_many for "download any file".
            [
                ".m3u8", ".mpd", ".mp4", ".webm", ".mov", ".m4a", ".mp3", ".wav", ".pdf", ".zip",
                ".gz", ".tar", ".7z", ".rar", ".csv", ".doc", ".docx", ".xls", ".xlsx", ".ppt",
                ".pptx",
            ]
            .iter()
            .any(|e| low.contains(e))
        };
        let hint = |u: &str| {
            let low = u.to_lowercase();
            [
                "player", "video", "media", "manifest", "stream", "videos/", "/api/", ".json",
            ]
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
        self.send_cmd("Page.navigate", json!({"url": url})).await?;

        // Queen Reader wait: poll until DOMContentLoaded (interactive), fall back to
        // full load when the page is still sparse (<500 chars). Faster than a fixed
        // sleep; absorbed the old read_fast/webrain_read (one wait strategy, one tool).
        let cap = opts.wait_timeout_secs.unwrap_or(4);
        let start = std::time::Instant::now();
        loop {
            let rs: String = self
                .eval_js("document.readyState")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            if rs == "interactive" || rs == "complete" || start.elapsed().as_secs() > cap {
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
            let cap2 = opts.wait_timeout_secs.unwrap_or(6);
            let start2 = std::time::Instant::now();
            loop {
                let rs: String = self
                    .eval_js("document.readyState")
                    .await?
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if rs == "complete" || start2.elapsed().as_secs() > cap2 {
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
        let links: Vec<String> =
            serde_json::from_value(self.eval_js(LINKS_JS).await?).unwrap_or_default();
        let crippled = detect_crippled(&challenge, elements.len());
        let chrome_error = detect_chrome_error(&title, &text);
        Ok(PageState {
            url: url.to_string(),
            title,
            text: text.chars().take(PAGE_TEXT_CAP).collect(),
            elements,
            links,
            challenge,
            crippled,
            chrome_error,
        })
    }
}

// ── Trusted-input helpers (browsemind cdp_session_* borrow) ──────────────
// CDP Input.* produces isTrusted=true events React/Polymer/shadow-DOM respect,
// unlike JS el.click()/.value= (synthetic, often swallowed). JS fallback keeps
// engines without Input.* support (lightpanda) working.

/// Scroll into view + viewport-center of element `index`. CDP Input uses
/// VIEWPORT coords — do NOT add scrollX/scrollY. None when missing/invisible.
async fn element_center(b: &CdpBackend, index: usize) -> anyhow::Result<Option<(i64, i64)>> {
    let js = format!(
        r#"(function() {{
            const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
            if (!el) return null;
            el.scrollIntoView({{ block: 'center', behavior: 'instant' }});
            const r = el.getBoundingClientRect();
            if (r.width === 0 || r.height === 0) return null;
            return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];
        }})()"#
    );
    let v = b.eval_js(&js).await?;
    Ok(v.as_array()
        .and_then(|a| Some((a.get(0)?.as_i64()?, a.get(1)?.as_i64()?))))
}

/// True when `elementFromPoint(x,y)` resolves to the element at `index` (or a
/// descendant). Coordinate-less engines (obscura: no layout → elementFromPoint
/// is null) return false so callers fall back to element-based JS activation.
async fn click_landed(b: &CdpBackend, x: i64, y: i64, index: usize) -> anyhow::Result<bool> {
    let js = format!(
        r#"(function() {{
            const els = document.querySelectorAll('a, button, input, select, textarea, [role="button"]');
            const el = els[{index}];
            if (!el) return false;
            const hit = document.elementFromPoint({x}, {y});
            if (!hit) return false;
            return el === hit || el.contains(hit);
        }})()"#
    );
    let v = b.eval_js(&js).await?;
    Ok(v.as_bool().unwrap_or(false))
}

/// Focus + clear an input/textarea/contenteditable by index.
async fn focus_clear(b: &CdpBackend, index: usize) -> anyhow::Result<()> {
    let js = format!(
        r#"(function() {{
            const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
            if (!el) return false;
            if (!/input|textarea|select/i.test(el.tagName) && !el.isContentEditable) return false;
            el.focus();
            if ('value' in el) el.value = '';
            return true;
        }})()"#
    );
    let v = b.eval_js(&js).await?;
    if v.as_bool().unwrap_or(false) {
        Ok(())
    } else {
        anyhow::bail!("element {index} is not an input")
    }
}

/// Trusted click at viewport coords via CDP Input.dispatchMouseEvent
/// (mousePressed + mouseReleased). Goes through the real browser input
/// pipeline → isTrusted=true, crosses shadow-DOM + cross-origin iframe
/// boundaries (reCAPTCHA). NO JS click here — CDP already fires the click, so
/// adding el.click() would double-fire. Engines without Input.* (lightpanda)
/// return Err; callers fall back to element-based el.click().
async fn dispatch_click(b: &CdpBackend, x: i64, y: i64) -> anyhow::Result<()> {
    // Move the pointer to (x,y) FIRST. reCAPTCHA v2/enterprise (and most
    // modern widgets) track hover/move state: a mousePressed+mouseReleased
    // with no prior pointer position is treated as synthetic and silently
    // ignored even though isTrusted=true. Playwright/browsemind both send a
    // mouseMoved before the click for exactly this reason — verified live:
    // without it the recaptcha-demo checkbox never registers, with it the
    // click lands and advances to the image puzzle.
    let moved = b.send_cmd(
        "Input.dispatchMouseEvent",
        json!({"type": "mouseMoved", "x": x, "y": y}),
    );
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), moved).await;
    // Bound the ack wait (agent-browser `dispatch_mouse_or_dialog` borrow): a
    // synchronous alert()/confirm() in the click handler pauses the renderer,
    // so the CDP input ack never arrives — without a timeout the click call
    // AND every later eval hang forever. On timeout the click usually landed
    // and a dialog is pending: return Ok and let webrain_dialog resolve it.
    let press = b.send_cmd(
        "Input.dispatchMouseEvent",
        json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1, "buttons": 1}),
    );
    match tokio::time::timeout(std::time::Duration::from_secs(2), press).await {
        Ok(inner) => inner.map(|_| ())?, // engine without Input.* → Err → JS fallback
        Err(_) => return Ok(()),         // renderer paused on a dialog — treat as dispatched
    }
    let release = b.send_cmd(
        "Input.dispatchMouseEvent",
        json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1, "buttons": 0}),
    );
    match tokio::time::timeout(std::time::Duration::from_secs(2), release).await {
        Ok(inner) => inner.map(|_| ()), // engine error → caller JS fallback
        Err(_) => Ok(()),               // dialog pending — treated as dispatched
    }
}

/// Stable backend-node click center (browsemind cdp_session_click_backend).
/// Element at `index` → backend node → `DOM.getContentQuads` → first-quad
/// center. `backend_node_id` survives incremental DOM mutations that stale
/// getBoundingClientRect coords don't, so the click lands even after the page
/// shifted. Chrome/Chromium (incl. Playwright chromium on Linux) implement it;
/// obscura/lightpanda lack layout/quads → None and the caller falls back to
/// the viewport-coord path.
async fn backend_click_center(b: &CdpBackend, index: usize) -> Option<(i64, i64)> {
    // Reuse webrain's own per-element CSS selector (ELEMENTS_JS) — the same one
    // navigate/snapshot indices map to.
    let elements: Value = b.eval_js(ELEMENTS_JS).await.ok()?;
    let selector = elements.as_array()?.get(index)?.get("selector")?.as_str()?;
    if selector.is_empty() {
        return None;
    }
    // DOM.enable is idempotent; ignore failure — getDocument/querySelector
    // below will also fail → None → caller falls back.
    let _ = b.send_cmd("DOM.enable", json!({})).await;
    let doc = b
        .send_cmd("DOM.getDocument", json!({"depth": 0}))
        .await
        .ok()?;
    let root = doc["root"]["nodeId"].as_i64()?;
    // Resolve the selector to a backend node (stable across DOM mutations).
    let q = b
        .send_cmd(
            "DOM.querySelector",
            json!({"nodeId": root, "selector": selector}),
        )
        .await
        .ok()?;
    let node_id = q["nodeId"].as_i64()?;
    if node_id == 0 {
        return None;
    }
    // nodeId → layout quads.
    let quads = b
        .send_cmd("DOM.getContentQuads", json!({"nodeId": node_id}))
        .await
        .ok()?;
    let first = quads["quads"].as_array()?.first()?;
    // Quad = 8 numbers (x0,y0,x1,y1,x2,y2,x3,y3); center of the first quad.
    let xs: Vec<f64> = (0..4)
        .filter_map(|i| first.get(i * 2).and_then(|v| v.as_f64()))
        .collect();
    let ys: Vec<f64> = (0..4)
        .filter_map(|i| first.get(i * 2 + 1).and_then(|v| v.as_f64()))
        .collect();
    if xs.len() != 4 || ys.len() != 4 {
        return None;
    }
    let cx = ((xs[0] + xs[1] + xs[2] + xs[3]) / 4.0).round() as i64;
    let cy = ((ys[0] + ys[1] + ys[2] + ys[3]) / 4.0).round() as i64;
    if cx <= 0 || cy <= 0 {
        return None;
    }
    Some((cx, cy))
}

/// Dispatch a trusted click at (x,y) IF it lands on the element at `index`.
/// True when the trusted CDP click was actually dispatched (landed + engine
/// accepted Input.dispatchMouseEvent).
async fn try_click_at(b: &CdpBackend, x: i64, y: i64, index: usize) -> bool {
    click_landed(b, x, y, index).await.unwrap_or(false) && dispatch_click(b, x, y).await.is_ok()
}

/// Resolve element `index` (ELEMENTS_JS selector) to a CDP DOM nodeId. Same
/// getDocument→querySelector path backend_click_center uses — needed by upload
/// (DOM.setFileInputFiles wants a node). None when missing/unresolvable.
async fn resolve_node_id(b: &CdpBackend, index: usize) -> Option<i64> {
    let elements: Value = b.eval_js(ELEMENTS_JS).await.ok()?;
    let selector = elements.as_array()?.get(index)?.get("selector")?.as_str()?;
    if selector.is_empty() {
        return None;
    }
    let _ = b.send_cmd("DOM.enable", json!({})).await;
    let doc = b
        .send_cmd("DOM.getDocument", json!({"depth": 0}))
        .await
        .ok()?;
    let root = doc["root"]["nodeId"].as_i64()?;
    let q = b
        .send_cmd(
            "DOM.querySelector",
            json!({"nodeId": root, "selector": selector}),
        )
        .await
        .ok()?;
    let node_id = q["nodeId"].as_i64()?;
    if node_id == 0 {
        return None;
    }
    Some(node_id)
}

// ── agent-browser borrows: select / hover / check / dialog / wait / upload ──
impl CdpBackend {
    /// Select a `<select>` option by value OR visible text (agent-browser
    /// `select_option` borrow). No-match is an ERROR listing available options
    /// so the LLM self-corrects instead of silently staying wrong.
    pub async fn select_option(&self, index: usize, value: &str) -> anyhow::Result<Value> {
        self.ensure_page_attached().await?;
        let js = format!(
            r#"(function() {{
                const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
                if (!el) return {{ error: 'no element at index {index}' }};
                if (el.tagName.toLowerCase() !== 'select') return {{ error: 'element {index} is not a <select>' }};
                const options = Array.from(el.options);
                const want = {value:?};
                let matched = 0;
                for (const opt of options) {{
                    opt.selected = opt.value === want || opt.textContent.trim() === want;
                    if (opt.selected) matched += 1;
                }}
                if (matched === 0) {{
                    const avail = options.map(o => o.value + ' ("' + o.textContent.trim() + '")').join(', ');
                    return {{ error: 'No option matched ' + JSON.stringify(want) + '. Available options: ' + avail }};
                }}
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return {{ matched: matched }};
            }})()"#,
            value = value
        );
        let v = self.eval_js(&js).await?;
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            anyhow::bail!("{e}");
        }
        Ok(v)
    }

    /// Trusted hover (agent-browser `hover` borrow): Input.dispatchMouseEvent
    /// mouseMoved to the element's center — triggers CSS :hover / JS
    /// mouseenter / hover-reveal content. JS fallback for engines w/o Input.*.
    pub async fn hover(&self, index: usize) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        if let Some((x, y)) = element_center(self, index).await? {
            if self
                .send_cmd(
                    "Input.dispatchMouseEvent",
                    json!({"type": "mouseMoved", "x": x, "y": y}),
                )
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        let js = format!(
            r#"(function() {{
                const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
                if (!el) return 'not found';
                for (const t of ['mouseover', 'mouseenter', 'mousemove']) el.dispatchEvent(new MouseEvent(t, {{ bubbles: true }}));
                return 'hovered';
            }})()"#
        );
        self.eval_js(&js).await?;
        Ok(())
    }

    /// Checked state by index (agent-browser `is_element_checked` borrow):
    /// native .checked → aria-checked → label retarget → nested input.
    pub async fn is_checked(&self, index: usize) -> anyhow::Result<bool> {
        self.ensure_page_attached().await?;
        let js = format!(
            r#"(function() {{
                const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
                if (!el) return false;
                const tag = el.tagName.toUpperCase();
                if (tag === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio')) return !!el.checked;
                const role = el.getAttribute && el.getAttribute('role');
                if (role && ['checkbox','radio','switch','menuitemcheckbox','menuitemradio','option','treeitem'].indexOf(role) !== -1) return el.getAttribute('aria-checked') === 'true';
                const label = tag === 'LABEL' ? el : (el.closest ? el.closest('label') : null);
                if (label && label.control && (label.control.type === 'checkbox' || label.control.type === 'radio')) return !!label.control.checked;
                const nested = el.querySelector && el.querySelector('input[type="checkbox"], input[type="radio"]');
                return nested ? !!nested.checked : false;
            }})()"#
        );
        Ok(self.eval_js(&js).await?.as_bool().unwrap_or(false))
    }

    /// Drive a checkbox/radio to `want` by index (agent-browser `check`/
    /// `uncheck` borrow). CDP click first; on state mismatch, JS retargets
    /// (native input → label.control → nested input) and clicks that. Returns
    /// the ACTUAL post-click state so the LLM can verify.
    pub async fn set_checked(&self, index: usize, want: bool) -> anyhow::Result<bool> {
        self.click(index).await?;
        let mut state = self.is_checked(index).await.unwrap_or(false);
        if state != want {
            let js = format!(
                r#"(function() {{
                    const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
                    if (!el) return false;
                    const tag = el.tagName.toUpperCase();
                    let target = null;
                    if (tag === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio')) target = el;
                    else {{
                        const label = tag === 'LABEL' ? el : (el.closest ? el.closest('label') : null);
                        if (label && label.control && (label.control.type === 'checkbox' || label.control.type === 'radio')) target = label.control;
                        else target = el.querySelector && el.querySelector('input[type="checkbox"], input[type="radio"]');
                    }}
                    if (target) target.click();
                    return !!target;
                }})()"#
            );
            let _ = self.eval_js(&js).await;
            state = self.is_checked(index).await.unwrap_or(state);
        }
        Ok(state)
    }

    /// Resolve a pending JS dialog (agent-browser `handle_dialog` borrow).
    /// Browser-served — works even when a sync alert() has paused the
    /// renderer, unblocking a stuck session.
    pub async fn dialog(&self, accept: bool, prompt_text: Option<&str>) -> anyhow::Result<()> {
        let sid = self
            .active_session()
            .await
            .ok_or_else(|| anyhow::anyhow!("no active page"))?;
        let mut params = json!({"accept": accept});
        if let Some(t) = prompt_text {
            params["promptText"] = json!(t);
        }
        self.send_cmd_with(Some(&sid), "Page.handleJavaScriptDialog", params)
            .await?;
        Ok(())
    }

    /// Upload files to a file input by index (agent-browser `upload_files`
    /// borrow): DOM.getDocument → DOM.querySelector → DOM.setFileInputFiles.
    pub async fn set_file_inputs(&self, index: usize, files: &[String]) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        let is_file: bool = self
            .eval_js(&format!(
                r#"(function() {{
                    const el = document.querySelectorAll('a, button, input, select, textarea, [role="button"]')[{index}];
                    return !!(el && el.tagName === 'INPUT' && el.type === 'file');
                }})()"#
            ))
            .await?
            .as_bool()
            .unwrap_or(false);
        if !is_file {
            anyhow::bail!("element {index} is not an <input type=file>");
        }
        let node_id = resolve_node_id(self, index)
            .await
            .ok_or_else(|| anyhow::anyhow!("could not resolve file input node at index {index}"))?;
        self.send_cmd(
            "DOM.setFileInputFiles",
            json!({"nodeId": node_id, "files": files}),
        )
        .await?;
        Ok(())
    }

    /// Standalone wait (agent-browser `wait` borrow): poll JS `expr` until
    /// truthy or `timeout_ms`. The LLM's post-action wait primitive
    /// (click→AJAX→render) — navigate already has an internal wait.
    pub async fn wait_for(&self, expr: &str, timeout_ms: u64) -> anyhow::Result<bool> {
        self.ensure_page_attached().await?;
        let start = std::time::Instant::now();
        loop {
            let v = self.eval_js(expr).await?;
            let truthy = match &v {
                Value::Bool(b) => *b,
                Value::String(s) => !s.is_empty(),
                Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
                Value::Array(a) => !a.is_empty(),
                Value::Object(o) => !o.is_empty(),
                _ => false,
            };
            if truthy {
                return Ok(true);
            }
            if start.elapsed().as_millis() as u64 >= timeout_ms {
                return Ok(false);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
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
                json!({"format": "png", "captureBeyondViewport": full_page}),
            )
            .await?;
        let b64 = result["data"].as_str().context("No screenshot data")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("Failed to decode screenshot base64")
    }

    async fn evaluate(&self, js: &str) -> anyhow::Result<Value> {
        self.ensure_page_attached().await?;
        self.eval_js(js).await
    }

    async fn eval_in_frame(
        &self,
        url_contains: &str,
        expression: &str,
    ) -> anyhow::Result<Option<Value>> {
        self.ensure_page_attached().await?;
        // Page.getFrameTree → find the frame whose URL matches (needs
        // Page.enable, already on at attach). Returns CDP result obj, so the
        // tree lives under `frameTree`.
        let tree = self.send_cmd("Page.getFrameTree", json!({})).await?;
        fn walk(node: &Value, needle: &str, out: &mut Option<String>) {
            if out.is_some() {
                return;
            }
            let f = node.get("frame");
            if let Some(url) = f.and_then(|x| x.get("url")).and_then(|x| x.as_str()) {
                if url.contains(needle) {
                    if let Some(id) = f.and_then(|x| x.get("id")).and_then(|x| x.as_str()) {
                        *out = Some(id.to_string());
                        return;
                    }
                }
            }
            if let Some(children) = node.get("childFrames").and_then(|c| c.as_array()) {
                for child in children {
                    walk(child, needle, out);
                    if out.is_some() {
                        return;
                    }
                }
            }
        }
        let mut frame_id = None;
        walk(
            &tree.get("frameTree").cloned().unwrap_or(Value::Null),
            url_contains,
            &mut frame_id,
        );
        let Some(frame_id) = frame_id else {
            return Ok(None);
        };
        // Create an isolated world in that frame and evaluate there.
        let world = self
            .send_cmd("Page.createIsolatedWorld", json!({"frameId": frame_id}))
            .await?;
        let Some(ctx) = world.get("executionContextId").and_then(|v| v.as_i64()) else {
            return Ok(None);
        };
        let result = self
            .send_cmd(
                "Runtime.evaluate",
                json!({"contextId": ctx, "expression": expression, "returnByValue": true, "awaitPromise": true}),
            )
            .await?;
        Ok(result.get("result").and_then(|r| r.get("value")).cloned())
    }

    async fn click(&self, index: usize) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // 1) Stable backend-node click (browsemind cdp_session_click_backend):
        //    DOM.getContentQuads via the element's backend node — survives
        //    incremental DOM mutations that stale viewport coords don't.
        //    Chrome/Chromium (incl. Playwright chromium on Linux) implement it;
        //    obscura/lightpanda return None → fall through.
        if let Some((x, y)) = backend_click_center(self, index).await {
            if try_click_at(self, x, y, index).await {
                return Ok(());
            }
        }
        // 2) Trusted CDP at the element's viewport center (browsemind
        //    cdp_session_click). GATE on the click landing: coordinate-less
        //    engines (obscura) report elementFromPoint()==null → fall through
        //    to the element-based JS click, which fires reliably on every
        //    engine.
        if let Ok(Some((x, y))) = element_center(self, index).await {
            if try_click_at(self, x, y, index).await {
                return Ok(());
            }
        }
        // 3) JS fallback: el.click() on the element directly (obscura/lightpanda).
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
        // Trusted CDP path (browsemind cdp_session_type): focus+clear then
        // Input.insertText — React/Vue detect isTrusted=true (JS .value= +
        // synthetic Event is ignored by controlled inputs).
        if focus_clear(self, index).await.is_ok() {
            if self
                .send_cmd("Input.insertText", json!({"text": text}))
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        // JS fallback
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

    async fn type_text_delayed(
        &self,
        index: usize,
        text: &str,
        delay_ms: u64,
    ) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // Focus+clear once, then Input.insertText per character with a delay —
        // human-like keystroke pacing (browsemind press_sequentially 40-120ms).
        // Google flags a whole-string insertText as scripted.
        if focus_clear(self, index).await.is_ok() {
            for c in text.chars() {
                if self
                    .send_cmd("Input.insertText", json!({ "text": c.to_string() }))
                    .await
                    .is_err()
                {
                    return self.type_text(index, text).await; // fallback
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            return Ok(());
        }
        self.type_text(index, text).await
    }

    async fn press(&self, key: &str) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // Trusted CDP path (browsemind cdp_session_press). NOTE: CDP
        // dispatchKeyEvent does NOT trigger native form submission for Enter —
        // Enter keeps the JS path which dispatches form.submit().
        if key != "Enter" {
            let (k, code) = match key {
                "Tab" => ("Tab", "Tab"),
                "Escape" => ("Escape", "Escape"),
                "Backspace" => ("Backspace", "Backspace"),
                "ArrowDown" => ("ArrowDown", "ArrowDown"),
                "ArrowUp" => ("ArrowUp", "ArrowUp"),
                "Space" => (" ", "Space"),
                _ => (key, key),
            };
            let p = json!({"type": "keyDown", "key": k, "code": code});
            if self.send_cmd("Input.dispatchKeyEvent", p).await.is_ok()
                && self
                    .send_cmd(
                        "Input.dispatchKeyEvent",
                        json!({"type": "keyUp", "key": k, "code": code}),
                    )
                    .await
                    .is_ok()
            {
                return Ok(());
            }
        }
        // JS fallback (Enter here — dispatches form.submit; lightpanda too)
        let js = format!(
            r#"(function() {{
                const k = '{key}';
                const map = {{'Enter':'Enter','Tab':'Tab','Escape':'Escape','Backspace':'Backspace','ArrowDown':'ArrowDown','ArrowUp':'ArrowUp','Space':' '}};
                const key = map[k] || k;
                const el = document.activeElement || document.body;
                const opts = {{ key, code: key, bubbles: true, cancelable: true }};
                el.dispatchEvent(new KeyboardEvent('keydown', opts));
                el.dispatchEvent(new KeyboardEvent('keyup', opts));
                if (el.form && k === 'Enter') {{ el.form.dispatchEvent(new Event('submit', {{ bubbles: true, cancelable: true }})); return 'form-submitted'; }}
                return 'pressed ' + k;
            }})()"#
        );
        self.eval_js(&js).await?;
        Ok(())
    }

    async fn click_coords(&self, x: i64, y: i64) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // Trusted coordinate click (browsemind cdp_session_click_coords):
        // crosses cross-origin iframe boundaries (reCAPTCHA) where JS clicks
        // only focus. dispatch_click already has a JS pointer fallback.
        if dispatch_click(self, x, y).await.is_err() {
            // JS fallback (lightpanda lacks Input.*)
            self.eval_js(&format!(
                r#"(function() {{ const el = document.elementFromPoint({x}, {y}); if (el) el.click(); return !!el; }})()"#
            ))
            .await?;
        }
        Ok(())
    }

    async fn mouse_move_human(&self, x: i64, y: i64) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // Trace an eased, slightly-jittered path from a plausible start toward
        // (x,y) — a real pointer travels, it doesn't teleport. Engines without
        // Input.* return Err and the caller's click_coords falls back on its own.
        let (sx, sy) = ((x - 180).max(0), (y - 70).max(0));
        let steps = 16;
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let ease = 1.0 - (1.0 - t) * (1.0 - t); // ease-out
            let jx = ((i * 7) % 5) as i64 - 2; // tiny pseudo-jitter
            let jy = ((i * 13) % 7) as i64 - 3;
            let px = sx + ((x - sx) as f64 * ease) as i64 + jx;
            let py = sy + ((y - sy) as f64 * ease) as i64 + jy;
            self.send_cmd(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": px, "y": py }),
            )
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(10 + (i % 4))).await;
        }
        Ok(())
    }

    async fn reload_hard(&self) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // Ctrl+Shift+R equivalent: Page.reload with ignoreCache — the anti-bot
        // wall sets state that a plain navigation re-uses; the hard reload
        // drops it (the manual recipe that works).
        self.send_cmd("Page.reload", json!({ "ignoreCache": true }))
            .await?;
        Ok(())
    }

    async fn drag(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> anyhow::Result<()> {
        self.ensure_page_attached().await?;
        // Trusted drag (drag-and-drop / slider CAPTCHAs): mouseMoved to the
        // handle, press (button held), a few move steps so the widget tracks
        // the pointer, release. Crosses cross-origin iframes like clicks.
        self.send_cmd(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": x1, "y": y1, "button": "left", "buttons": 0}),
        )
        .await?;
        self.send_cmd(
            "Input.dispatchMouseEvent",
            json!({"type": "mousePressed", "x": x1, "y": y1, "button": "left", "buttons": 1, "clickCount": 1}),
        )
        .await?;
        let steps = 10;
        for i in 1..=steps {
            let x = x1 + (x2 - x1) * i / steps;
            let y = y1 + (y2 - y1) * i / steps;
            self.send_cmd(
                "Input.dispatchMouseEvent",
                json!({"type": "mouseMoved", "x": x, "y": y, "button": "left", "buttons": 1}),
            )
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        self.send_cmd(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseReleased", "x": x2, "y": y2, "button": "left", "buttons": 0}),
        )
        .await?;
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
        scale: f64,
    ) -> anyhow::Result<Vec<u8>> {
        self.ensure_page_attached().await?;
        let result = self
            .send_cmd(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "clip": {"x": x, "y": y, "width": width, "height": height, "scale": scale.max(0.5)},
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
        let links: Vec<String> =
            serde_json::from_value(self.eval_js(LINKS_JS).await?).unwrap_or_default();
        let crippled = detect_crippled(&challenge, elements.len());
        let chrome_error = detect_chrome_error(&title, &text);
        let state = PageState {
            url,
            title,
            text: text.chars().take(PAGE_TEXT_CAP).collect(),
            elements,
            links,
            challenge,
            crippled,
            chrome_error,
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

    async fn add_init_script(&self, js: &str) -> anyhow::Result<()> {
        CdpBackend::add_init_script(self, js).await
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
        let tree = self
            .send_cmd("Accessibility.getFullAXTree", json!({}))
            .await?;
        let nodes = tree["nodes"].as_array().cloned().unwrap_or_default();

        // One DOM.getDocument walk maps backendNodeId -> (css_path, xpath).
        let mut paths: std::collections::HashMap<i64, (String, String)> = Default::default();
        if let Ok(doc) = self
            .send_cmd("DOM.getDocument", json!({"depth": -1, "pierce": true}))
            .await
        {
            if let Some(children) = doc
                .get("root")
                .and_then(|r| r.get("children"))
                .and_then(|c| c.as_array())
            {
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
    if id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        id.to_string()
    } else {
        format!("[id={id:?}]")
    }
}
